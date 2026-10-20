//! Socket watcher and client connection helpers (`src/nvim/event/socket.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::charset::try_getdigits;
use crate::event::r#loop::{
    loop_poll_events, main_loop_ptr, uv_close, uv_freeaddrinfo, uv_handle_get_loop,
    uv_handle_get_type, uv_handle_size, uv_pipe_connect, uv_pipe_init, uv_req_size, uv_strerror,
};
use crate::event::multiqueue::{EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_put_event};
use crate::event::stream::{
    Stream, UV_NAMED_PIPE, UV_TCP, stream_fields, stream_init, stream_may_close, stream_uv_ptr,
};
use crate::log::logmsg;
use crate::memory::{xfree, xstrdup, xstrlcpy};
use crate::os::fs::{os_path_exists, os_remove};
use crate::os::time::os_hrtime;
use crate::path::path_tail;
use crate::strings::{gettext, strlen as c_strlen, strrchr, vim_snprintf};

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

const NUL: c_char = 0;
const ADDRESS_MAX_SIZE: usize = 256;

const UV_REQ: c_int = 1;
const UV_GETADDRINFO: c_int = 8;
const UV_PIPE_NO_TRUNCATE: c_uint = 1;

const AF_UNSPEC: c_int = 0;
const SOCK_STREAM: c_int = 1;

#[cfg(windows)]
const AI_NUMERICSERV: c_int = 0x0008;
#[cfg(target_os = "linux")]
const AI_NUMERICSERV: c_int = 0x0400;
#[cfg(not(any(windows, target_os = "linux")))]
const AI_NUMERICSERV: c_int = 0x1000;

#[cfg(windows)]
const UV_EACCES: c_int = -4092;
#[cfg(not(windows))]
const UV_EACCES: c_int = -13;

#[cfg(windows)]
const UV_EADDRINUSE: c_int = -4091;
#[cfg(target_os = "linux")]
const UV_EADDRINUSE: c_int = -98;
#[cfg(not(any(windows, target_os = "linux")))]
const UV_EADDRINUSE: c_int = -48;

#[cfg(windows)]
const UV_EINVAL: c_int = -4071;
#[cfg(not(windows))]
const UV_EINVAL: c_int = -22;

#[cfg(windows)]
const UV_ENOENT: c_int = -4058;
#[cfg(not(windows))]
const UV_ENOENT: c_int = -2;

const LOGLVL_WRN: c_int = 3;
const LOGLVL_ERR: c_int = 4;

pub use crate::event::r#loop::Loop;
pub type SocketWatcher = c_void;
pub use crate::event::r#loop::uv_loop_t;
pub use crate::event::r#loop::{uv_close_cb, uv_connect_cb, uv_connect_t, uv_handle_t};
pub use crate::event::rstream::RStream;
pub use crate::event::stream::uv_pipe_t;
pub use crate::event::stream::uv_stream_t;
pub type uv_tcp_t = c_void;
pub type uv_getaddrinfo_t = c_void;

pub type socket_cb =
    Option<unsafe extern "C" fn(watcher: *mut SocketWatcher, result: c_int, data: *mut c_void)>;
pub type socket_close_cb =
    Option<unsafe extern "C" fn(watcher: *mut SocketWatcher, data: *mut c_void)>;
type uv_connection_cb = Option<unsafe extern "C" fn(server: *mut uv_stream_t, status: c_int)>;

#[repr(C)]
pub struct addrinfo {
    pub ai_flags: c_int,
    pub ai_family: c_int,
    pub ai_socktype: c_int,
    pub ai_protocol: c_int,
    pub ai_addrlen: usize,
    #[cfg(target_os = "linux")]
    pub ai_addr: *mut c_void,
    pub ai_canonname: *mut c_char,
    #[cfg(not(target_os = "linux"))]
    pub ai_addr: *mut c_void,
    pub ai_next: *mut addrinfo,
}

#[repr(C)]
struct SocketWatcherTail {
    stream: *mut uv_stream_t,
    data: *mut c_void,
    cb: socket_cb,
    close_cb: socket_close_cb,
    events: *mut MultiQueue,
}

#[repr(C, align(8))]
struct SockaddrStorage {
    bytes: [u8; 128],
}

#[repr(C, align(8))]
struct UvGetaddrinfoReq([u8; 512]);

#[repr(C, align(8))]
struct UvConnectReq([u8; 512]);

#[repr(C, align(8))]
struct RStreamBuf([u8; 1024]);

unsafe extern "C" {
    fn uv_getaddrinfo(
        loop_: *mut uv_loop_t,
        req: *mut uv_getaddrinfo_t,
        getaddrinfo_cb: *mut c_void,
        node: *const c_char,
        service: *const c_char,
        hints: *const addrinfo,
    ) -> c_int;

    fn uv_tcp_init(loop_: *mut uv_loop_t, handle: *mut uv_tcp_t) -> c_int;
    fn uv_tcp_nodelay(handle: *mut uv_tcp_t, enable: c_int) -> c_int;
    fn uv_tcp_bind(handle: *mut uv_tcp_t, addr: *const c_void, flags: c_uint) -> c_int;
    fn uv_tcp_getsockname(handle: *const uv_tcp_t, name: *mut c_void, namelen: *mut c_int)
    -> c_int;
    fn uv_tcp_connect(
        req: *mut uv_connect_t,
        handle: *mut uv_tcp_t,
        addr: *const c_void,
        cb: uv_connect_cb,
    ) -> c_int;

    fn uv_pipe_bind2(
        handle: *mut uv_pipe_t,
        name: *const c_char,
        namelen: usize,
        flags: c_uint,
    ) -> c_int;

    fn uv_listen(stream: *mut uv_stream_t, backlog: c_int, cb: uv_connection_cb) -> c_int;
    fn uv_accept(server: *mut uv_stream_t, client: *mut uv_stream_t) -> c_int;

}

#[inline]
unsafe fn socket_watcher_uv_size() -> usize {
    unsafe {
        let tcp_size = uv_handle_size(UV_TCP) + size_of::<*mut addrinfo>();
        let pipe_size = uv_handle_size(UV_NAMED_PIPE);
        tcp_size.max(pipe_size)
    }
}

#[inline]
unsafe fn watcher_addr(watcher: *mut SocketWatcher) -> *mut c_char {
    watcher.cast::<c_char>()
}

#[inline]
unsafe fn watcher_uv_ptr(watcher: *mut SocketWatcher) -> *mut c_void {
    unsafe { watcher.cast::<u8>().add(ADDRESS_MAX_SIZE).cast() }
}

#[inline]
unsafe fn watcher_tcp_addrinfo_ptr(watcher: *mut SocketWatcher) -> *mut *mut addrinfo {
    unsafe {
        watcher
            .cast::<u8>()
            .add(ADDRESS_MAX_SIZE + uv_handle_size(UV_TCP))
            .cast::<*mut addrinfo>()
    }
}

#[inline]
unsafe fn watcher_tail(watcher: *mut SocketWatcher) -> *mut SocketWatcherTail {
    unsafe {
        watcher
            .cast::<u8>()
            .add(ADDRESS_MAX_SIZE + socket_watcher_uv_size())
            .cast::<SocketWatcherTail>()
    }
}

#[inline]
unsafe fn getaddrinfo_req_addrinfo_ptr(req: *mut uv_getaddrinfo_t) -> *mut *mut addrinfo {
    unsafe {
        req.cast::<u8>()
            .add(uv_req_size(UV_GETADDRINFO) - 2 * size_of::<*mut c_void>())
            .cast::<*mut addrinfo>()
    }
}

#[inline]
unsafe fn connect_req_handle(req: *mut uv_connect_t) -> *mut uv_stream_t {
    unsafe {
        *req.cast::<u8>()
            .add(uv_req_size(UV_REQ) + size_of::<*mut c_void>())
            .cast::<*mut uv_stream_t>()
    }
}

#[inline]
unsafe fn poll_main_loop_until<F: FnMut() -> bool>(timeout: i64, mut cond: F) {
    unsafe {
        let mut remaining = timeout;
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !cond() {
            loop_poll_events(main_loop_ptr(), remaining);
            if remaining == 0 {
                break;
            } else if remaining > 0 {
                let now = os_hrtime();
                remaining -= ((now - before) / 1_000_000) as i64;
                before = now;
                if remaining <= 0 {
                    break;
                }
            }
        }
    }
}

/// Checks if an address string looks like a TCP endpoint, and returns the end of the host part.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn socket_address_tcp_host_end(address: *const c_char) -> *mut c_char {
    if address.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        // Windows drive letter path: "X:/..." is a local path, not TCP.
        let b0 = *address as u8;
        if b0.is_ascii_alphabetic()
            && *address.add(1) == b':' as c_char
            && *address.add(2) == b'/' as c_char
        {
            return ptr::null_mut();
        }

        let colon = strrchr(address, b':' as c_int);
        if !colon.is_null() && colon != address.cast_mut() {
            colon
        } else {
            ptr::null_mut()
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn socket_watcher_init(
    loop_: *mut Loop,
    watcher: *mut SocketWatcher,
    endpoint: *const c_char,
) -> c_int {
    unsafe {
        let addr = watcher_addr(watcher);
        xstrlcpy(addr, endpoint, ADDRESS_MAX_SIZE);
        let host_end = socket_address_tcp_host_end(addr);
        let tail = watcher_tail(watcher);
        let uv_ptr = watcher_uv_ptr(watcher);

        if !host_end.is_null() {
            *host_end = NUL;
            let mut port = host_end.add(1);
            let mut port_scan = port;
            let mut iport: i64 = 0;

            let ok = try_getdigits(&raw mut port_scan, &raw mut iport);
            if !ok || iport < 0 || iport > i64::from(u16::MAX) {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"socket_watcher_init".as_ptr(),
                    line!() as c_int,
                    true,
                    c"Invalid port: %s".as_ptr(),
                    port,
                );
                return UV_EINVAL;
            }

            if *port == NUL {
                port = ptr::null_mut();
            }

            let mut request = UvGetaddrinfoReq([0; 512]);
            let hints = addrinfo {
                ai_flags: 0,
                ai_family: AF_UNSPEC,
                ai_socktype: SOCK_STREAM,
                ai_protocol: 0,
                ai_addrlen: 0,
                ai_addr: ptr::null_mut(),
                ai_canonname: ptr::null_mut(),
                ai_next: ptr::null_mut(),
            };

            let retval = uv_getaddrinfo(
                loop_.cast(),
                (&raw mut request).cast(),
                ptr::null_mut(),
                addr,
                port,
                &raw const hints,
            );
            if retval != 0 {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"socket_watcher_init".as_ptr(),
                    line!() as c_int,
                    true,
                    c"Host lookup failed: %s".as_ptr(),
                    endpoint,
                );
                return retval;
            }
            *watcher_tcp_addrinfo_ptr(watcher) =
                *getaddrinfo_req_addrinfo_ptr((&raw mut request).cast());

            uv_tcp_init(loop_.cast(), uv_ptr.cast());
            uv_tcp_nodelay(uv_ptr.cast(), 1);
            (*tail).stream = uv_ptr.cast();
        } else {
            uv_pipe_init(loop_.cast(), uv_ptr.cast(), 0);
            (*tail).stream = uv_ptr.cast();
        }

        *(*tail).stream.cast::<*mut SocketWatcher>() = watcher;
        (*tail).cb = None;
        (*tail).close_cb = None;
        (*tail).events = ptr::null_mut();
        (*tail).data = ptr::null_mut();

        0
    }
}

/// Callback after closing a Stream initialized by socket_connect().
unsafe extern "C" fn connect_close_cb(_stream: *mut Stream, data: *mut c_void) {
    unsafe {
        *data.cast::<bool>() = true;
    }
}

/// Check if a socket is alive by attempting to connect to it.
unsafe fn socket_alive(loop_: *mut Loop, addr: *const c_char) -> bool {
    unsafe {
        debug_assert!(crate::event::rstream::rstream_size() <= size_of::<RStreamBuf>());
        let mut stream = RStreamBuf([0; 1024]);
        let mut error: *const c_char = ptr::null();

        let stream_ptr = (&raw mut stream).cast::<RStream>();
        let connected = socket_connect(loop_, stream_ptr, false, addr, 500, &raw mut error);
        if !connected {
            return false;
        }

        let mut closed = false;
        let closed_ptr: *mut bool = &raw mut closed;
        let fields = stream_fields(stream_ptr.cast());
        (*fields).internal_close_cb = Some(connect_close_cb);
        (*fields).internal_data = closed_ptr.cast();
        stream_may_close(stream_ptr.cast());
        poll_main_loop_until(-1, || ptr::read_volatile(closed_ptr));

        true
    }
}

unsafe extern "C" fn early_server_close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let closed = *handle.cast::<*mut bool>();
        *closed = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn socket_watcher_start(
    watcher: *mut SocketWatcher,
    backlog: c_int,
    cb: socket_cb,
) -> c_int {
    unsafe {
        let tail = watcher_tail(watcher);
        let addr = watcher_addr(watcher);
        let uv_ptr = watcher_uv_ptr(watcher);
        (*tail).cb = cb;
        let mut result: c_int = UV_EINVAL;

        if uv_handle_get_type((*tail).stream.cast()) == UV_TCP {
            let mut ai = *watcher_tcp_addrinfo_ptr(watcher);

            while !ai.is_null() {
                result = uv_tcp_bind(uv_ptr.cast(), (*ai).ai_addr, 0);
                if result != 0 {
                    ai = (*ai).ai_next;
                    continue;
                }
                result = uv_listen((*tail).stream, backlog, Some(connection_cb));
                if result == 0 {
                    let mut sas = SockaddrStorage { bytes: [0; 128] };
                    let mut sas_len = size_of::<SockaddrStorage>() as c_int;
                    uv_tcp_getsockname(uv_ptr.cast(), (&raw mut sas).cast(), &raw mut sas_len);
                    let port = u16::from_be_bytes([sas.bytes[2], sas.bytes[3]]);
                    let len = c_strlen(addr);
                    vim_snprintf!(
                        addr.add(len),
                        ADDRESS_MAX_SIZE - len,
                        c":%u".as_ptr(),
                        c_uint::from(port),
                    );
                    break;
                }
                ai = (*ai).ai_next;
            }
            uv_freeaddrinfo((*watcher_tcp_addrinfo_ptr(watcher)).cast());
        } else {
            result = uv_pipe_bind2(uv_ptr.cast(), addr, c_strlen(addr), UV_PIPE_NO_TRUNCATE);

            // If bind failed with EACCES/EADDRINUSE, check if socket is stale
            if result == UV_EACCES || result == UV_EADDRINUSE {
                let uv_loop = uv_handle_get_loop((*tail).stream.cast());
                let loop_ = *uv_loop.cast::<*mut Loop>();

                if !socket_alive(loop_, addr) {
                    let rm_result = os_remove(addr);

                    if rm_result != 0 {
                        logmsg!(
                            LOGLVL_WRN,
                            ptr::null(),
                            c"socket_watcher_start".as_ptr(),
                            line!() as c_int,
                            true,
                            c"Failed to remove stale socket %s: %s".as_ptr(),
                            addr,
                            uv_strerror(rm_result),
                        );
                    } else {
                        // Close and reinit the pipe handle before retrying bind
                        let uv_loop = uv_handle_get_loop(uv_ptr.cast());
                        let mut closed = false;
                        let closed_ptr: *mut bool = &raw mut closed;
                        *uv_ptr.cast::<*mut bool>() = closed_ptr;
                        uv_close(uv_ptr.cast(), Some(early_server_close_cb));
                        poll_main_loop_until(-1, || ptr::read_volatile(closed_ptr));

                        uv_pipe_init(uv_loop, uv_ptr.cast(), 0);
                        (*tail).stream = uv_ptr.cast();
                        *(*tail).stream.cast::<*mut SocketWatcher>() = watcher;

                        // Retry bind with fresh handle
                        result =
                            uv_pipe_bind2(uv_ptr.cast(), addr, c_strlen(addr), UV_PIPE_NO_TRUNCATE);
                    }
                } else {
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"socket_watcher_start".as_ptr(),
                        line!() as c_int,
                        true,
                        c"Socket already in use by another Nvim instance: %s".as_ptr(),
                        addr,
                    );
                }
            }

            if result == 0 {
                result = uv_listen((*tail).stream, backlog, Some(connection_cb));
            }
        }

        debug_assert!(result <= 0);
        if result < 0 {
            if result == UV_EACCES {
                *path_tail(addr) = NUL;
                if !os_path_exists(addr) {
                    result = UV_ENOENT;
                }
            }
            return result;
        }

        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn socket_watcher_accept(
    watcher: *mut SocketWatcher,
    stream: *mut RStream,
) -> c_int {
    unsafe {
        let tail = watcher_tail(watcher);
        let uv_ptr = watcher_uv_ptr(watcher);
        let client: *mut uv_stream_t = stream_uv_ptr(stream.cast()).cast();

        if uv_handle_get_type((*tail).stream.cast()) == UV_TCP {
            let uv_loop = uv_handle_get_loop(uv_ptr.cast());
            uv_tcp_init(uv_loop, client.cast());
            uv_tcp_nodelay(client.cast(), 1);
        } else {
            let uv_loop = uv_handle_get_loop(uv_ptr.cast());
            uv_pipe_init(uv_loop, client.cast(), 0);
        }

        let result = uv_accept((*tail).stream, client);

        if result != 0 {
            uv_close(client.cast(), None);
            return result;
        }

        stream_init(ptr::null_mut(), stream.cast(), -1, client);
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn socket_watcher_close(watcher: *mut SocketWatcher, cb: socket_close_cb) {
    unsafe {
        let tail = watcher_tail(watcher);
        (*tail).close_cb = cb;
        uv_close((*tail).stream.cast(), Some(close_cb));
    }
}

unsafe extern "C" fn connection_event(argv: *mut *mut c_void) {
    unsafe {
        let watcher = *argv as *mut SocketWatcher;
        let status = (*argv.add(1) as usize) as c_int;
        let tail = watcher_tail(watcher);
        if let Some(cb) = (*tail).cb {
            cb(watcher, status, (*tail).data);
        }
    }
}

unsafe extern "C" fn connection_cb(handle: *mut uv_stream_t, status: c_int) {
    unsafe {
        let watcher = *handle.cast::<*mut SocketWatcher>();
        let tail = watcher_tail(watcher);
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = watcher;
        argv[1] = (status as usize) as *mut c_void;
        if !(*tail).events.is_null() {
            multiqueue_put_event(
                (*tail).events,
                Event {
                    handler: Some(connection_event),
                    argv,
                },
            );
        } else {
            connection_event(argv.as_mut_ptr());
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let watcher = *handle.cast::<*mut SocketWatcher>();
        let tail = watcher_tail(watcher);
        if let Some(cb) = (*tail).close_cb {
            cb(watcher, (*tail).data);
        }
    }
}

unsafe extern "C" fn connect_cb(req: *mut uv_connect_t, status: c_int) {
    unsafe {
        let ret_status = *req.cast::<*mut c_int>();
        *ret_status = status;
        let handle = connect_req_handle(req);
        if status != 0 {
            let stream = *handle.cast::<*mut Stream>();
            stream_may_close(stream);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn socket_connect(
    loop_: *mut Loop,
    stream: *mut RStream,
    is_tcp: bool,
    address: *const c_char,
    timeout: c_int,
    error: *mut *const c_char,
) -> bool {
    unsafe {
        let mut success = false;
        let mut closed = false;
        let mut status: c_int = 1;
        let mut req = UvConnectReq([0; 512]);
        let req_ptr = (&raw mut req).cast::<uv_connect_t>();
        let status_ptr: *mut c_int = &raw mut status;
        let closed_ptr: *mut bool = &raw mut closed;
        *req_ptr.cast::<*mut c_int>() = status_ptr;

        let s_ptr: *mut Stream = stream.cast();
        let uv_ptr = stream_uv_ptr(s_ptr);
        let fields = stream_fields(s_ptr);

        let mut addr_req = UvGetaddrinfoReq([0; 512]);
        let addr_req_ptr = (&raw mut addr_req).cast::<uv_getaddrinfo_t>();
        *getaddrinfo_req_addrinfo_ptr(addr_req_ptr) = ptr::null_mut();
        let mut cur_addrinfo: *const addrinfo = ptr::null();
        let mut addr: *mut c_char = ptr::null_mut();

        'cleanup: {
            if is_tcp {
                addr = xstrdup(address);
                let host_end = strrchr(addr, b':' as c_int);
                if host_end.is_null() {
                    *error = gettext(c"tcp address must be host:port".as_ptr());
                    break 'cleanup;
                }
                *host_end = NUL;

                let hints = addrinfo {
                    ai_flags: AI_NUMERICSERV,
                    ai_family: AF_UNSPEC,
                    ai_socktype: SOCK_STREAM,
                    ai_protocol: 0,
                    ai_addrlen: 0,
                    ai_addr: ptr::null_mut(),
                    ai_canonname: ptr::null_mut(),
                    ai_next: ptr::null_mut(),
                };
                let retval = uv_getaddrinfo(
                    loop_.cast(),
                    addr_req_ptr,
                    ptr::null_mut(),
                    addr,
                    host_end.add(1),
                    &raw const hints,
                );
                if retval != 0 {
                    *error = gettext(c"failed to lookup host or port".as_ptr());
                    break 'cleanup;
                }
                cur_addrinfo = *getaddrinfo_req_addrinfo_ptr(addr_req_ptr);
            }

            loop {
                let uv_stream: *mut uv_stream_t = if is_tcp {
                    let tcp: *mut uv_tcp_t = uv_ptr.cast();
                    uv_tcp_init(loop_.cast(), tcp);
                    uv_tcp_nodelay(tcp, 1);
                    uv_tcp_connect(req_ptr, tcp, (*cur_addrinfo).ai_addr, Some(connect_cb));
                    tcp.cast()
                } else {
                    let pipe: *mut uv_pipe_t = uv_ptr.cast();
                    uv_pipe_init(loop_.cast(), pipe, 0);
                    uv_pipe_connect(req_ptr, pipe, address, Some(connect_cb));
                    pipe.cast()
                };
                stream_init(ptr::null_mut(), s_ptr, -1, uv_stream);
                (*fields).internal_close_cb = Some(connect_close_cb);
                (*fields).internal_data = closed_ptr.cast();
                ptr::write_volatile(closed_ptr, false);
                ptr::write_volatile(status_ptr, 1);
                poll_main_loop_until(i64::from(timeout), || ptr::read_volatile(status_ptr) != 1);
                if ptr::read_volatile(status_ptr) == 0 {
                    success = true;
                    break;
                } else {
                    stream_may_close(s_ptr);
                    poll_main_loop_until(-1, || ptr::read_volatile(closed_ptr));

                    if is_tcp && !(*cur_addrinfo).ai_next.is_null() {
                        cur_addrinfo = (*cur_addrinfo).ai_next;
                        continue;
                    } else {
                        *error = gettext(c"connection refused".as_ptr());
                        break;
                    }
                }
            }
        }

        (*fields).internal_close_cb = None;
        (*fields).internal_data = ptr::null_mut();
        xfree(addr.cast());
        uv_freeaddrinfo((*getaddrinfo_req_addrinfo_ptr(addr_req_ptr)).cast());
        success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_address_tcp_host_end_detects_tcp_and_rejects_drive_letters() {
        unsafe {
            assert!(socket_address_tcp_host_end(ptr::null()).is_null());
            assert!(socket_address_tcp_host_end(c"C:/tmp/nvim.sock".as_ptr()).is_null());
            assert!(socket_address_tcp_host_end(c"/tmp/nvim.sock".as_ptr()).is_null());
            let s = c"127.0.0.1:6666".as_ptr();
            let host_end = socket_address_tcp_host_end(s);
            assert_eq!(host_end, s.add(9).cast_mut());
        }
    }
}
