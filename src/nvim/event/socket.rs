//! Socket watcher and client connection helpers (`src/nvim/event/socket.c`).

use crate::charset::{NUL, try_getdigits};
use crate::event::r#loop::{
    Loop, loop_poll_events, main_loop_ptr, uv_close, uv_connect_t, uv_freeaddrinfo,
    uv_handle_get_loop, uv_handle_get_type, uv_handle_t, uv_pipe_connect, uv_pipe_init,
    uv_req_size, uv_strerror,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_put_event};
use crate::event::stream::{
    Stream, StreamInternalData, UV_TCP, stream_init, stream_may_close, uv_pipe_t, uv_stream_t,
};
use crate::log::{LOGLVL_ERR, LOGLVL_WRN, logmsg};
use crate::memory::xstrlcpy;
use crate::os::fs::{UV_EINVAL, UV_ENOENT, os_path_exists, os_remove};
use crate::os::time::os_hrtime;
use crate::path::path_tail;
use crate::strings::{gettext, vim_snprintf};

use std::ffi::{CStr, c_void};
use std::ptr;

use crate::event::r#loop::UV_REQ;

pub(crate) const ADDRESS_MAX_SIZE: usize = 256;

const UV_GETADDRINFO: i32 = 8;
const UV_PIPE_NO_TRUNCATE: u32 = 1;

const AF_UNSPEC: i32 = 0;
const SOCK_STREAM: i32 = 1;

#[cfg(windows)]
const AI_NUMERICSERV: i32 = 0x0008;
#[cfg(target_os = "linux")]
const AI_NUMERICSERV: i32 = 0x0400;
#[cfg(not(any(windows, target_os = "linux")))]
const AI_NUMERICSERV: i32 = 0x1000;

#[cfg(windows)]
const UV_EACCES: i32 = -4092;
#[cfg(not(windows))]
const UV_EACCES: i32 = -13;

#[cfg(windows)]
const UV_EADDRINUSE: i32 = -4091;
#[cfg(target_os = "linux")]
const UV_EADDRINUSE: i32 = -98;
#[cfg(not(any(windows, target_os = "linux")))]
const UV_EADDRINUSE: i32 = -48;

use crate::event::rstream::RStream;
#[allow(non_camel_case_types)]
pub type uv_tcp_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_getaddrinfo_t = c_void;

#[repr(C)]
pub struct addrinfo {
    pub ai_flags: i32,
    pub ai_family: i32,
    pub ai_socktype: i32,
    pub ai_protocol: i32,
    pub ai_addrlen: usize,
    #[cfg(target_os = "linux")]
    pub ai_addr: *mut c_void,
    pub ai_canonname: *mut u8,
    #[cfg(not(target_os = "linux"))]
    pub ai_addr: *mut c_void,
    pub ai_next: *mut addrinfo,
}

#[repr(C, align(8))]
pub struct UvSocketHandle(pub [u8; 640]);

#[repr(C)]
pub struct SocketWatcher {
    pub addr: [u8; ADDRESS_MAX_SIZE],
    pub uv: UvSocketHandle,
    pub tcp_addrinfo: *mut addrinfo,
    pub events: *mut MultiQueue,
}

impl SocketWatcher {
    pub const fn new() -> Self {
        Self {
            addr: [0; ADDRESS_MAX_SIZE],
            uv: UvSocketHandle([0; 640]),
            tcp_addrinfo: ptr::null_mut(),
            events: ptr::null_mut(),
        }
    }

    #[inline]
    pub fn stream_ptr(&mut self) -> *mut uv_stream_t {
        (&raw mut self.uv).cast()
    }

    #[inline]
    pub fn addr_cstr(&self) -> &CStr {
        CStr::from_bytes_until_nul(&self.addr).unwrap_or(c"")
    }

    #[inline]
    pub fn addr_bytes(&self) -> &[u8] {
        self.addr_cstr().to_bytes()
    }
}

#[repr(C, align(8))]
struct SockaddrStorage {
    bytes: [u8; 128],
}

#[repr(C, align(8))]
struct UvGetaddrinfoReq([u8; 512]);

#[repr(C, align(8))]
struct UvConnectReq([u8; 512]);

use crate::event::r#loop::{
    uv_accept, uv_getaddrinfo, uv_listen, uv_pipe_bind2, uv_tcp_bind, uv_tcp_connect,
    uv_tcp_getsockname, uv_tcp_init, uv_tcp_nodelay,
};

#[inline]
fn getaddrinfo_req_addrinfo_ptr(req: *mut uv_getaddrinfo_t) -> *mut *mut addrinfo {
    req.cast::<u8>()
        .wrapping_add(unsafe { uv_req_size(UV_GETADDRINFO) } - 2 * size_of::<*mut c_void>())
        .cast::<*mut addrinfo>()
}

#[inline]
fn connect_req_handle(req: *mut uv_connect_t) -> *mut uv_stream_t {
    unsafe {
        *req.cast::<u8>()
            .wrapping_add(uv_req_size(UV_REQ) + size_of::<*mut c_void>())
            .cast::<*mut uv_stream_t>()
    }
}

#[inline]
fn poll_main_loop_until<F: FnMut() -> bool>(timeout: i64, mut cond: F) {
    let timeout_ms = timeout;
    let start = if timeout_ms >= 0 { os_hrtime() } else { 0 };
    let mut remaining = timeout_ms;
    let mut expired = false;
    while !cond() && !expired {
        loop_poll_events(&mut *main_loop_ptr(), remaining);
        if timeout_ms >= 0 {
            remaining = timeout_ms - ((os_hrtime() - start) / 1_000_000) as i64;
            expired = remaining <= 0;
        }
    }
}

/// Checks if an address string looks like a TCP endpoint, and returns the byte offset of the `:` separator.
pub fn socket_address_tcp_host_end(address: &[u8]) -> Option<usize> {
    // Windows drive letter path: "X:/..." is a local path, not TCP.
    if address.first().is_some_and(u8::is_ascii_alphabetic)
        && address.get(1) == Some(&b':')
        && address.get(2) == Some(&b'/')
    {
        return None;
    }

    address.iter().rposition(|&b| b == b':').filter(|&i| i != 0)
}

pub fn socket_watcher_init(loop_: &mut Loop, watcher: &mut SocketWatcher, endpoint: &CStr) -> i32 {
    xstrlcpy(
        watcher.addr.as_mut_ptr().cast(),
        endpoint.as_ptr(),
        ADDRESS_MAX_SIZE,
    );
    let host_end_idx = socket_address_tcp_host_end(watcher.addr_bytes());
    let uv_ptr: *mut c_void = (&raw mut watcher.uv).cast();

    if let Some(colon_idx) = host_end_idx {
        watcher.addr[colon_idx] = 0;
        let addr = watcher.addr.as_ptr().cast();
        let mut port = watcher.addr.as_mut_ptr().wrapping_add(colon_idx + 1).cast();
        let mut port_scan = port;
        let mut iport: i64 = 0;

        let ok = try_getdigits(&raw mut port_scan, &mut iport);
        if !ok || iport < 0 || iport > i64::from(u16::MAX) {
            logmsg!(
                LOGLVL_ERR,
                std::ptr::null(),
                c"socket_watcher_init",
                line!() as i32,
                true,
                c"Invalid port: %s",
                port,
            );
            return UV_EINVAL;
        }

        if crate::memory::cstr_is_empty(port) {
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

        let retval = unsafe {
            uv_getaddrinfo(
                (&raw mut loop_.uv).cast(),
                (&raw mut request).cast(),
                ptr::null_mut(),
                addr,
                port,
                &raw const hints,
            )
        };
        if retval != 0 {
            logmsg!(
                LOGLVL_ERR,
                std::ptr::null(),
                c"socket_watcher_init",
                line!() as i32,
                true,
                c"Host lookup failed: %s",
                endpoint.as_ptr(),
            );
            return retval;
        }
        watcher.tcp_addrinfo = unsafe { *getaddrinfo_req_addrinfo_ptr((&raw mut request).cast()) };

        unsafe { uv_tcp_init((&raw mut loop_.uv).cast(), uv_ptr.cast()) };
        unsafe { uv_tcp_nodelay(uv_ptr.cast(), 1) };
    } else {
        unsafe { uv_pipe_init((&raw mut loop_.uv).cast(), uv_ptr.cast(), 0) };
    }

    unsafe {
        *watcher.stream_ptr().cast::<*mut SocketWatcher>() = (&raw mut *watcher);
    }
    watcher.events = ptr::null_mut();

    0
}

/// Check if a socket is alive by attempting to connect to it.
fn socket_alive(loop_: &mut Loop, addr: &CStr) -> bool {
    let mut stream = RStream::new();
    let mut error = None;

    let stream_ptr = &mut stream;
    let connected = socket_connect(loop_, stream_ptr, false, addr, 500, &mut error);
    if !connected {
        return false;
    }

    let mut closed = false;
    let closed_ptr: *mut bool = &raw mut closed;
    stream.s.internal_data = StreamInternalData::Closed(closed_ptr);
    stream_may_close(&mut stream.s);
    poll_main_loop_until(-1, || unsafe { ptr::read_volatile(closed_ptr) });

    true
}

unsafe extern "C" fn early_server_close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let closed = *handle.cast::<*mut bool>();
        *closed = true;
    }
}

pub fn socket_watcher_start(watcher: &mut SocketWatcher, backlog: i32) -> i32 {
    unsafe {
        let addr_len = watcher.addr_bytes().len();
        let addr = watcher.addr.as_mut_ptr().cast();
        let uv_ptr: *mut c_void = (&raw mut watcher.uv).cast();
        let mut result: i32 = UV_EINVAL;

        if uv_handle_get_type(watcher.stream_ptr().cast()) == UV_TCP {
            let mut ai = watcher.tcp_addrinfo;

            while !ai.is_null() {
                result = uv_tcp_bind(uv_ptr.cast(), (*ai).ai_addr, 0);
                if result != 0 {
                    ai = (*ai).ai_next;
                    continue;
                }
                result = uv_listen(watcher.stream_ptr(), backlog, Some(connection_cb));
                if result == 0 {
                    let mut sas = SockaddrStorage { bytes: [0; 128] };
                    let mut sas_len = size_of::<SockaddrStorage>() as i32;
                    uv_tcp_getsockname(uv_ptr.cast(), (&raw mut sas).cast(), &mut sas_len);
                    let port = u16::from_be_bytes([sas.bytes[2], sas.bytes[3]]);
                    vim_snprintf!(
                        watcher.addr.as_mut_ptr().wrapping_add(addr_len).cast(),
                        ADDRESS_MAX_SIZE - addr_len,
                        c":%u",
                        u32::from(port),
                    );
                    break;
                }
                ai = (*ai).ai_next;
            }
            uv_freeaddrinfo(watcher.tcp_addrinfo.cast());
        } else {
            result = uv_pipe_bind2(uv_ptr.cast(), addr, addr_len, UV_PIPE_NO_TRUNCATE);

            // If bind failed with EACCES/EADDRINUSE, check if socket is stale
            if result == UV_EACCES || result == UV_EADDRINUSE {
                let uv_loop = uv_handle_get_loop(watcher.stream_ptr().cast());
                let loop_ = *uv_loop.cast::<*mut Loop>();

                if !socket_alive(&mut *loop_, watcher.addr_cstr()) {
                    let rm_result = os_remove(addr);

                    if rm_result != 0 {
                        logmsg!(
                            LOGLVL_WRN,
                            std::ptr::null(),
                            c"socket_watcher_start",
                            line!() as i32,
                            true,
                            c"Failed to remove stale socket %s: %s",
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
                        *watcher.stream_ptr().cast::<*mut SocketWatcher>() = (&raw mut *watcher);

                        // Retry bind with fresh handle
                        result = uv_pipe_bind2(uv_ptr.cast(), addr, addr_len, UV_PIPE_NO_TRUNCATE);
                    }
                } else {
                    logmsg!(
                        LOGLVL_ERR,
                        std::ptr::null(),
                        c"socket_watcher_start",
                        line!() as i32,
                        true,
                        c"Socket already in use by another Nvim instance: %s",
                        addr,
                    );
                }
            }

            if result == 0 {
                result = uv_listen(watcher.stream_ptr(), backlog, Some(connection_cb));
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

pub fn socket_watcher_accept(watcher: &mut SocketWatcher, stream: &mut RStream) -> i32 {
    unsafe {
        let uv_ptr: *mut c_void = (&raw mut watcher.uv).cast();
        let client: *mut uv_stream_t = (&raw mut stream.s.uv).cast();

        if uv_handle_get_type(watcher.stream_ptr().cast()) == UV_TCP {
            let uv_loop = uv_handle_get_loop(uv_ptr.cast());
            uv_tcp_init(uv_loop, client.cast());
            uv_tcp_nodelay(client.cast(), 1);
        } else {
            let uv_loop = uv_handle_get_loop(uv_ptr.cast());
            uv_pipe_init(uv_loop, client.cast(), 0);
        }

        let result = uv_accept(watcher.stream_ptr(), client);

        if result != 0 {
            uv_close(client.cast(), None);
            return result;
        }

        stream_init(ptr::null_mut(), &mut stream.s, -1, true);
        0
    }
}

pub fn socket_watcher_close(watcher: &mut SocketWatcher) {
    unsafe {
        uv_close(watcher.stream_ptr().cast(), Some(close_cb));
    }
}

pub fn connection_event(watcher: *mut SocketWatcher, status: i32) {
    crate::msgpack_rpc::server::connection_cb(watcher, status);
}

unsafe extern "C" fn connection_cb(handle: *mut uv_stream_t, status: i32) {
    unsafe {
        let watcher = *handle.cast::<*mut SocketWatcher>();
        if !(*watcher).events.is_null() {
            multiqueue_put_event(
                &mut *(*watcher).events,
                Event::SocketConnection { watcher, status },
            );
        } else {
            connection_event(watcher, status);
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let watcher = *handle.cast::<*mut SocketWatcher>();
        drop(Box::from_raw(watcher));
    }
}

unsafe extern "C" fn connect_cb(req: *mut uv_connect_t, status: i32) {
    unsafe {
        let ret_status = *req.cast::<*mut i32>();
        *ret_status = status;
        let handle = connect_req_handle(req);
        if status != 0 {
            let stream = *handle.cast::<*mut Stream>();
            stream_may_close(&mut *stream);
        }
    }
}

pub fn socket_connect(
    loop_: &mut Loop,
    stream: &mut RStream,
    is_tcp: bool,
    address: &std::ffi::CStr,
    timeout: i32,
    error: &mut Option<&'static std::ffi::CStr>,
) -> bool {
    unsafe {
        let mut success = false;
        let mut closed = false;
        let mut status: i32 = 1;
        let mut req = UvConnectReq([0; 512]);
        let req_ptr = (&raw mut req).cast::<uv_connect_t>();
        let status_ptr: *mut i32 = &raw mut status;
        let closed_ptr: *mut bool = &raw mut closed;
        *req_ptr.cast::<*mut i32>() = status_ptr;

        let s_ptr: *mut Stream = &raw mut stream.s;
        let uv_ptr: *mut c_void = (&raw mut (*s_ptr).uv).cast();
        let fields = s_ptr;

        let mut addr_req = UvGetaddrinfoReq([0; 512]);
        let addr_req_ptr = (&raw mut addr_req).cast::<uv_getaddrinfo_t>();
        *getaddrinfo_req_addrinfo_ptr(addr_req_ptr) = ptr::null_mut();
        let mut cur_addrinfo: *const addrinfo = std::ptr::null();
        let mut addr_buf: Vec<u8> = Vec::new();

        'cleanup: {
            if is_tcp {
                addr_buf.extend_from_slice(address.to_bytes_with_nul());
                let Some(colon_idx) = addr_buf[..addr_buf.len() - 1]
                    .iter()
                    .rposition(|&b| b == b':')
                else {
                    *error = Some(crate::memory::cstr_from_ptr(gettext(
                        c"tcp address must be host:port",
                    )));
                    break 'cleanup;
                };
                addr_buf[colon_idx] = 0;
                let addr = addr_buf.as_ptr().cast();
                let port = addr_buf.as_ptr().wrapping_add(colon_idx + 1).cast();

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
                    (&raw mut loop_.uv).cast(),
                    addr_req_ptr,
                    ptr::null_mut(),
                    addr,
                    port,
                    &raw const hints,
                );
                if retval != 0 {
                    *error = Some(crate::memory::cstr_from_ptr(gettext(
                        c"failed to lookup host or port",
                    )));
                    break 'cleanup;
                }
                cur_addrinfo = *getaddrinfo_req_addrinfo_ptr(addr_req_ptr);
            }

            loop {
                if is_tcp {
                    let tcp: *mut uv_tcp_t = uv_ptr.cast();
                    uv_tcp_init((&raw mut loop_.uv).cast(), tcp);
                    uv_tcp_nodelay(tcp, 1);
                    uv_tcp_connect(req_ptr, tcp, (*cur_addrinfo).ai_addr, Some(connect_cb));
                } else {
                    let pipe: *mut uv_pipe_t = uv_ptr.cast();
                    uv_pipe_init((&raw mut loop_.uv).cast(), pipe, 0);
                    uv_pipe_connect(req_ptr, pipe, address.as_ptr(), Some(connect_cb));
                }
                stream_init(ptr::null_mut(), &mut *s_ptr, -1, true);
                (*fields).internal_data = StreamInternalData::Closed(closed_ptr);
                ptr::write_volatile(closed_ptr, false);
                ptr::write_volatile(status_ptr, 1);
                poll_main_loop_until(i64::from(timeout), || ptr::read_volatile(status_ptr) != 1);
                if ptr::read_volatile(status_ptr) == 0 {
                    success = true;
                    break;
                } else {
                    stream_may_close(&mut *s_ptr);
                    poll_main_loop_until(-1, || ptr::read_volatile(closed_ptr));

                    if is_tcp && !(*cur_addrinfo).ai_next.is_null() {
                        cur_addrinfo = (*cur_addrinfo).ai_next;
                        continue;
                    } else {
                        *error = Some(crate::memory::cstr_from_ptr(gettext(c"connection refused")));
                        break;
                    }
                }
            }
        }

        (*fields).internal_data = StreamInternalData::NONE;
        drop(addr_buf);
        uv_freeaddrinfo((*getaddrinfo_req_addrinfo_ptr(addr_req_ptr)).cast());
        success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_address_tcp_host_end_detects_tcp_and_rejects_drive_letters() {
        assert_eq!(socket_address_tcp_host_end(b""), None);
        assert_eq!(socket_address_tcp_host_end(b"C:/tmp/nvim.sock"), None);
        assert_eq!(socket_address_tcp_host_end(b"/tmp/nvim.sock"), None);
        assert_eq!(socket_address_tcp_host_end(b"127.0.0.1:6666"), Some(9));
    }
}
