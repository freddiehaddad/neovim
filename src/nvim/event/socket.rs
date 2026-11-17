//! Socket watcher and client connection helpers (`src/nvim/event/socket.c`).

use crate::charset::try_getdigits;
use crate::event::r#loop::{
    Loop, loop_poll_events, main_loop, uv_close, uv_connect_t, uv_freeaddrinfo, uv_handle_size,
    uv_handle_t, uv_pipe_connect, uv_pipe_init, uv_req_size, uv_strerror,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_put_event};
use crate::event::owned::SharedCallbackHandle;
use crate::event::stream::{
    Stream, StreamInternalData, UV_NAMED_PIPE, UV_TCP, stream_init, stream_may_close, uv_pipe_t,
    uv_stream_t,
};
use crate::log::{LOGLVL_ERR, LOGLVL_WRN, logmsg};
use crate::os::fs::{UV_EINVAL, UV_ENOENT, os_path_exists, os_remove};
use crate::os::time::os_hrtime;
use crate::path::path_tail;
use crate::strings::{format_to_buf, gettext};

use std::cell::Cell;
use std::ffi::{CStr, c_void};
use std::ptr::{self, NonNull};

use crate::event::r#loop::UV_REQ;

pub(crate) const ADDRESS_MAX_SIZE: usize = 256;
pub(crate) const SOCKET_HANDLE_WORDS: usize = 80;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SocketKind {
    Tcp,
    Pipe,
}

struct SocketState {
    loop_: &'static Loop,
    kind: SocketKind,
    events: MultiQueue,
    listening: Cell<bool>,
}

#[derive(Clone)]
pub struct SocketHandle {
    native: SharedCallbackHandle<SocketState, SOCKET_HANDLE_WORDS>,
}

impl SocketHandle {
    fn new(loop_: &'static Loop, kind: SocketKind, events: MultiQueue) -> Result<Self, i32> {
        let handle = Self {
            native: SharedCallbackHandle::new(SocketState {
                loop_,
                kind,
                events,
                listening: Cell::new(false),
            }),
        };
        let result = unsafe {
            let native_type = match kind {
                SocketKind::Tcp => UV_TCP,
                SocketKind::Pipe => UV_NAMED_PIPE,
            };
            assert!(uv_handle_size(native_type) <= SOCKET_HANDLE_WORDS * size_of::<u64>());
            match kind {
                SocketKind::Tcp => uv_tcp_init(loop_.uv_ptr(), handle.as_ptr()),
                SocketKind::Pipe => uv_pipe_init(loop_.uv_ptr(), handle.as_ptr(), 0),
            }
        };
        if result != 0 {
            return Err(result);
        }
        unsafe {
            if kind == SocketKind::Tcp {
                uv_tcp_nodelay(handle.as_ptr(), 1);
            }
            handle.native.register();
        }
        loop_.register_handle(handle.as_ptr(), close_registered);
        Ok(handle)
    }

    fn as_ptr(&self) -> *mut uv_stream_t {
        self.native.as_ptr()
    }

    fn is_listening(&self) -> bool {
        self.native.is_active() && self.native.data().listening.get()
    }

    fn close(&self) {
        if self.native.begin_close() {
            self.native.data().listening.set(false);
            unsafe { uv_close(self.as_ptr(), Some(close_cb)) };
        }
    }

    /// `handle` must carry this listener type's live registration on its owning thread.
    unsafe fn from_callback(handle: *mut c_void) -> Self {
        Self {
            native: unsafe { SharedCallbackHandle::from_callback(handle) },
        }
    }
}

#[must_use]
pub struct SocketWatcher {
    addr: [u8; ADDRESS_MAX_SIZE],
    handle: SocketHandle,
    tcp_addrinfo: Option<AddrInfo>,
}

impl SocketWatcher {
    #[inline]
    pub fn addr_cstr(&self) -> &CStr {
        CStr::from_bytes_until_nul(&self.addr).unwrap_or(c"")
    }

    #[inline]
    pub fn addr_bytes(&self) -> &[u8] {
        self.addr_cstr().to_bytes()
    }
}

impl Drop for SocketWatcher {
    fn drop(&mut self) {
        self.handle.close();
    }
}

struct AddrInfo(NonNull<addrinfo>);

impl AddrInfo {
    /// The request must contain a completed synchronous getaddrinfo result.
    unsafe fn take(request: &mut UvGetaddrinfoReq) -> Option<Self> {
        let address = unsafe {
            (&raw mut *request)
                .cast::<u8>()
                .add(uv_req_size(UV_GETADDRINFO) - 2 * size_of::<*mut c_void>())
                .cast::<*mut addrinfo>()
                .replace(ptr::null_mut())
        };
        NonNull::new(address).map(Self)
    }

    fn as_ptr(&self) -> *mut addrinfo {
        self.0.as_ptr()
    }
}

impl Drop for AddrInfo {
    fn drop(&mut self) {
        unsafe { uv_freeaddrinfo(self.as_ptr().cast()) };
    }
}

#[repr(C, align(8))]
struct SockaddrStorage {
    bytes: [u8; 128],
}

#[repr(C, align(8))]
struct UvGetaddrinfoReq([u8; 512]);

impl UvGetaddrinfoReq {
    fn new() -> Self {
        assert!(unsafe { uv_req_size(UV_GETADDRINFO) } <= size_of::<Self>());
        Self([0; 512])
    }
}

#[repr(C, align(8))]
struct UvConnectReq([u8; 512]);

use crate::event::r#loop::{
    uv_accept, uv_getaddrinfo, uv_listen, uv_pipe_bind2, uv_tcp_bind, uv_tcp_connect,
    uv_tcp_getsockname, uv_tcp_init, uv_tcp_nodelay,
};

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
        loop_poll_events(main_loop(), remaining);
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

pub fn socket_watcher_init(
    loop_: &'static Loop,
    endpoint: &CStr,
    events: MultiQueue,
) -> Result<SocketWatcher, i32> {
    let mut address = [0; ADDRESS_MAX_SIZE];
    crate::memory::copy_bytes_to_c_buf(&mut address, endpoint.to_bytes());
    let host_end_idx = socket_address_tcp_host_end(
        CStr::from_bytes_until_nul(&address)
            .unwrap_or(c"")
            .to_bytes(),
    );
    let mut tcp_addrinfo = None;
    let kind = if let Some(colon_idx) = host_end_idx {
        address[colon_idx] = 0;
        let addr = address.as_ptr().cast();
        let port_cstr = CStr::from_bytes_until_nul(&address[colon_idx + 1..]).unwrap_or(c"");
        let mut port_scan = port_cstr.to_bytes();
        let mut iport: i64 = 0;

        let ok = try_getdigits(&mut port_scan, &mut iport);
        if !ok || iport < 0 || iport > i64::from(u16::MAX) {
            logmsg!(
                LOGLVL_ERR,
                None,
                Some(c"socket_watcher_init"),
                line!() as i32,
                true,
                c"Invalid port: %s",
                port_cstr,
            );
            return Err(UV_EINVAL);
        }

        let port = if port_cstr.to_bytes().is_empty() {
            ptr::null_mut()
        } else {
            port_cstr.as_ptr().cast_mut()
        };

        let mut request = UvGetaddrinfoReq::new();
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
                loop_.uv_ptr(),
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
                None,
                Some(c"socket_watcher_init"),
                line!() as i32,
                true,
                c"Host lookup failed: %s",
                endpoint,
            );
            return Err(retval);
        }
        tcp_addrinfo = unsafe { AddrInfo::take(&mut request) };
        SocketKind::Tcp
    } else {
        SocketKind::Pipe
    };

    Ok(SocketWatcher {
        addr: address,
        handle: SocketHandle::new(loop_, kind, events)?,
        tcp_addrinfo,
    })
}

/// Checks if a socket is alive by attempting to connect to it.
pub fn socket_alive(
    loop_: &Loop,
    is_tcp: bool,
    addr: &CStr,
    error: &mut Option<&'static CStr>,
) -> bool {
    let stream = RStream::new();

    let stream_ptr = &stream;
    let connected = socket_connect(loop_, stream_ptr, is_tcp, addr, 500, error);
    if !connected {
        return false;
    }

    let closed = std::rc::Rc::new(std::cell::Cell::new(false));
    stream
        .s
        .internal_data
        .set(StreamInternalData::Closed(closed.clone()));
    stream_may_close(&stream.s);
    poll_main_loop_until(-1, || closed.get());

    true
}

pub fn socket_watcher_start(watcher: &mut SocketWatcher, backlog: i32) -> i32 {
    if !watcher.handle.native.is_active() || watcher.handle.is_listening() {
        return UV_EINVAL;
    }
    unsafe {
        let addr_len = watcher.addr_bytes().len();
        let addr = watcher.addr.as_ptr().cast();
        let mut uv_ptr = watcher.handle.as_ptr();
        let mut result: i32 = UV_EINVAL;

        if watcher.handle.native.data().kind == SocketKind::Tcp {
            let addresses = watcher.tcp_addrinfo.take();
            let mut ai = addresses.as_ref().map_or(ptr::null_mut(), AddrInfo::as_ptr);

            while !ai.is_null() {
                result = uv_tcp_bind(uv_ptr.cast(), (*ai).ai_addr, 0);
                if result != 0 {
                    ai = (*ai).ai_next;
                    continue;
                }
                result = uv_listen(uv_ptr, backlog, Some(connection_cb));
                if result == 0 {
                    let mut sas = SockaddrStorage { bytes: [0; 128] };
                    let mut sas_len = size_of::<SockaddrStorage>() as i32;
                    uv_tcp_getsockname(uv_ptr.cast(), (&raw mut sas).cast(), &mut sas_len);
                    let port = u16::from_be_bytes([sas.bytes[2], sas.bytes[3]]);
                    format_to_buf!(
                        &mut watcher.addr[addr_len..ADDRESS_MAX_SIZE],
                        c":%u",
                        u32::from(port),
                    );
                    break;
                }
                ai = (*ai).ai_next;
            }
        } else {
            result = uv_pipe_bind2(uv_ptr.cast(), addr, addr_len, UV_PIPE_NO_TRUNCATE);

            // If bind failed with EACCES/EADDRINUSE, check if socket is stale
            if result == UV_EACCES || result == UV_EADDRINUSE {
                let loop_ = watcher.handle.native.data().loop_;
                let mut error = None;

                if !socket_alive(loop_, false, watcher.addr_cstr(), &mut error) {
                    let rm_result = os_remove(watcher.addr_cstr());

                    if rm_result != 0 {
                        logmsg!(
                            LOGLVL_WRN,
                            None,
                            Some(c"socket_watcher_start"),
                            line!() as i32,
                            true,
                            c"Failed to remove stale socket %s: %s",
                            watcher.addr_cstr(),
                            uv_strerror(rm_result),
                        );
                    } else {
                        let previous = watcher.handle.clone();
                        let events = previous.native.data().events;
                        previous.close();
                        poll_main_loop_until(-1, || previous.native.is_closed());
                        watcher.handle = match SocketHandle::new(loop_, SocketKind::Pipe, events) {
                            Ok(handle) => handle,
                            Err(error) => return error,
                        };
                        uv_ptr = watcher.handle.as_ptr();
                        result = uv_pipe_bind2(uv_ptr.cast(), addr, addr_len, UV_PIPE_NO_TRUNCATE);
                    }
                } else {
                    logmsg!(
                        LOGLVL_ERR,
                        None,
                        Some(c"socket_watcher_start"),
                        line!() as i32,
                        true,
                        c"Socket already in use by another Nvim instance: %s",
                        watcher.addr_cstr(),
                    );
                }
            }

            if result == 0 {
                result = uv_listen(uv_ptr, backlog, Some(connection_cb));
            }
        }

        debug_assert!(result <= 0);
        if result < 0 {
            if result == UV_EACCES {
                let addr_bytes = watcher.addr_cstr().to_bytes();
                let tail_off = addr_bytes.len() - path_tail(addr_bytes).len();
                watcher.addr[tail_off] = 0;
                if !os_path_exists(watcher.addr_cstr()) {
                    result = UV_ENOENT;
                }
            }
            return result;
        }

        watcher.handle.native.data().listening.set(true);
        0
    }
}

pub fn socket_watcher_accept(watcher: &SocketHandle, stream: &RStream) -> i32 {
    assert!(stream.uv.can_initialize());
    assert_eq!(stream.pending_reqs.get(), 0);
    if !watcher.is_listening() {
        return UV_EINVAL;
    }
    unsafe {
        let uv_ptr = watcher.as_ptr();
        let client: *mut uv_stream_t = stream.s.uv.as_ptr();
        let loop_ = watcher.native.data().loop_;

        let result = match watcher.native.data().kind {
            SocketKind::Tcp => uv_tcp_init(loop_.uv_ptr(), client.cast()),
            SocketKind::Pipe => uv_pipe_init(loop_.uv_ptr(), client.cast(), 0),
        };
        if result != 0 {
            return result;
        }
        if watcher.native.data().kind == SocketKind::Tcp {
            uv_tcp_nodelay(client.cast(), 1);
        }

        stream_init(None, &stream.s, -1, true);
        let result = uv_accept(uv_ptr, client);

        if result != 0 {
            stream_may_close(&stream.s);
            return result;
        }

        0
    }
}

pub fn connection_event(watcher: SocketHandle, status: i32) {
    if watcher.is_listening() {
        crate::msgpack_rpc::server::connection_cb(&watcher, status);
    }
}

unsafe extern "C" fn connection_cb(handle: *mut uv_stream_t, status: i32) {
    let watcher = unsafe { SocketHandle::from_callback(handle) };
    if let Some(events) = watcher.native.data().events.opt() {
        multiqueue_put_event(events, Event::SocketConnection { watcher, status });
    } else {
        connection_event(watcher, status);
    }
}

unsafe fn close_registered(handle: *mut c_void) {
    unsafe { SocketHandle::from_callback(handle) }.close();
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    let watcher = unsafe { SocketHandle::from_callback(handle) };
    watcher.native.data().loop_.unregister_handle(handle);
    unsafe { watcher.native.finish_close() };
}

unsafe extern "C" fn connect_cb(req: *mut uv_connect_t, status: i32) {
    unsafe {
        let ret_status = *req.cast::<*mut i32>();
        *ret_status = status;
        if status != 0 {
            let handle = connect_req_handle(req);
            stream_may_close(&Stream::from_callback(handle));
        }
    }
}

pub fn socket_connect(
    loop_: &Loop,
    stream: &RStream,
    is_tcp: bool,
    address: &std::ffi::CStr,
    timeout: i32,
    error: &mut Option<&'static std::ffi::CStr>,
) -> bool {
    unsafe {
        let mut success = false;
        let closed = std::rc::Rc::new(std::cell::Cell::new(false));
        let mut status: i32 = 1;
        let mut req = UvConnectReq([0; 512]);
        let req_ptr = (&raw mut req).cast::<uv_connect_t>();
        let status_ptr = &raw mut status;
        *req_ptr.cast::<*mut i32>() = status_ptr;

        let s = &stream.s;
        let uv_ptr = s.uv.as_ptr();

        let mut addresses = None;
        let mut cur_addrinfo: *const addrinfo = std::ptr::null();
        let mut addr_buf: Vec<u8> = Vec::new();

        'cleanup: {
            if is_tcp {
                addr_buf.extend_from_slice(address.to_bytes_with_nul());
                let Some(colon_idx) = addr_buf[..addr_buf.len() - 1]
                    .iter()
                    .rposition(|&b| b == b':')
                else {
                    *error = Some(gettext(c"tcp address must be host:port"));
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
                let mut addr_req = UvGetaddrinfoReq::new();
                let retval = uv_getaddrinfo(
                    loop_.uv_ptr(),
                    (&raw mut addr_req).cast(),
                    ptr::null_mut(),
                    addr,
                    port,
                    &raw const hints,
                );
                if retval != 0 {
                    *error = Some(gettext(c"failed to lookup host or port"));
                    break 'cleanup;
                }
                addresses = AddrInfo::take(&mut addr_req);
                cur_addrinfo = addresses
                    .as_ref()
                    .map_or(ptr::null(), |addresses| addresses.as_ptr().cast_const());
            }

            loop {
                assert!(s.uv.can_initialize());
                assert_eq!(s.pending_reqs.get(), 0);
                if is_tcp {
                    let tcp: *mut uv_tcp_t = uv_ptr.cast();
                    uv_tcp_init(loop_.uv_ptr(), tcp);
                    uv_tcp_nodelay(tcp, 1);
                    uv_tcp_connect(req_ptr, tcp, (*cur_addrinfo).ai_addr, Some(connect_cb));
                } else {
                    let pipe: *mut uv_pipe_t = uv_ptr.cast();
                    uv_pipe_init(loop_.uv_ptr(), pipe, 0);
                    uv_pipe_connect(req_ptr, pipe, address.as_ptr(), Some(connect_cb));
                }
                stream_init(None, s, -1, true);
                s.internal_data
                    .set(StreamInternalData::Closed(closed.clone()));
                closed.set(false);
                ptr::write_volatile(status_ptr, 1);
                poll_main_loop_until(i64::from(timeout), || ptr::read_volatile(status_ptr) != 1);
                if ptr::read_volatile(status_ptr) == 0 {
                    success = true;
                    break;
                } else {
                    stream_may_close(s);
                    poll_main_loop_until(-1, || closed.get());

                    if is_tcp && !(*cur_addrinfo).ai_next.is_null() {
                        cur_addrinfo = (*cur_addrinfo).ai_next;
                        continue;
                    } else {
                        *error = Some(gettext(c"connection refused"));
                        break;
                    }
                }
            }
        }

        s.internal_data.set(StreamInternalData::NONE);
        drop(addr_buf);
        drop(addresses);
        success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::r#loop::uv_is_closing;
    use crate::event::multiqueue::{
        multiqueue_cancel_events, multiqueue_empty, multiqueue_free, multiqueue_new,
        multiqueue_process_events,
    };

    fn editor() -> (
        std::sync::MutexGuard<'static, ()>,
        crate::main_init::TestEditorGuard,
    ) {
        let lock = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (lock, crate::main_init::test_editor())
    }

    #[test]
    fn socket_address_tcp_host_end_detects_tcp_and_rejects_drive_letters() {
        assert_eq!(socket_address_tcp_host_end(b""), None);
        assert_eq!(socket_address_tcp_host_end(b"C:/tmp/nvim.sock"), None);
        assert_eq!(socket_address_tcp_host_end(b"/tmp/nvim.sock"), None);
        assert_eq!(socket_address_tcp_host_end(b"127.0.0.1:6666"), Some(9));
    }

    #[test]
    fn initialized_listener_survives_owner_moves_and_closes_on_drop() {
        let (_lock, _editor) = editor();
        let watcher = socket_watcher_init(main_loop(), c"127.0.0.1:0", MultiQueue::NULL).unwrap();
        let handle = watcher.handle.clone();
        let pointer = handle.as_ptr();
        let mut registry = vec![watcher];
        assert_eq!(socket_watcher_start(&mut registry[0], 8), 0);
        assert_eq!(handle.as_ptr(), pointer);
        drop(registry);
        assert_ne!(unsafe { uv_is_closing(pointer) }, 0);
        loop_poll_events(main_loop(), 0);
        assert!(handle.native.is_closed());
    }

    #[test]
    fn unstarted_listener_drop_completes_native_close() {
        let (_lock, _editor) = editor();
        let watcher = socket_watcher_init(main_loop(), c"127.0.0.1:0", MultiQueue::NULL).unwrap();
        let handle = watcher.handle.clone();
        assert!(watcher.tcp_addrinfo.is_some());
        drop(watcher);
        loop_poll_events(main_loop(), 0);
        assert!(handle.native.is_closed());
    }

    #[test]
    fn queued_connections_survive_close_and_can_be_dispatched_or_cancelled() {
        let (_lock, _editor) = editor();
        for cancel in [false, true] {
            let events = multiqueue_new(None);
            let mut watcher = socket_watcher_init(main_loop(), c"127.0.0.1:0", events).unwrap();
            assert_eq!(socket_watcher_start(&mut watcher, 8), 0);
            let handle = watcher.handle.clone();
            let client = RStream::new();
            let mut error = None;
            assert!(socket_connect(
                main_loop(),
                &client,
                true,
                watcher.addr_cstr(),
                1000,
                &mut error,
            ));
            poll_main_loop_until(1000, || !multiqueue_empty(events));
            assert!(!multiqueue_empty(events));
            drop(watcher);
            loop_poll_events(main_loop(), 0);
            assert!(handle.native.is_closed());
            drop(handle);

            let channels = main_loop().channels().len();
            if cancel {
                multiqueue_cancel_events(events);
            } else {
                multiqueue_process_events(events);
            }
            assert_eq!(main_loop().channels().len(), channels);
            multiqueue_free(events);
            stream_may_close(&client.s);
            loop_poll_events(main_loop(), 0);
        }
    }

    #[test]
    fn failed_bind_closes_only_the_failed_listener() {
        let (_lock, _editor) = editor();
        let mut listener =
            socket_watcher_init(main_loop(), c"127.0.0.1:0", MultiQueue::NULL).unwrap();
        assert_eq!(socket_watcher_start(&mut listener, 8), 0);
        let mut duplicate =
            socket_watcher_init(main_loop(), listener.addr_cstr(), MultiQueue::NULL).unwrap();
        let failed = duplicate.handle.clone();
        assert!(socket_watcher_start(&mut duplicate, 8) < 0);
        assert!(duplicate.tcp_addrinfo.is_none());
        drop(duplicate);
        loop_poll_events(main_loop(), 0);
        assert!(failed.native.is_closed());
        assert!(listener.handle.is_listening());
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_retry_uses_a_new_native_lifetime() {
        struct SocketPath(std::path::PathBuf);
        impl Drop for SocketPath {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        let (_lock, _editor) = editor();
        let path = SocketPath(std::env::temp_dir().join(format!(
            "nvim-listener-{}-{}",
            std::process::id(),
            os_hrtime(),
        )));
        std::fs::write(&path.0, []).unwrap();
        let endpoint = std::ffi::CString::new(path.0.as_os_str().as_encoded_bytes()).unwrap();
        let mut watcher = socket_watcher_init(main_loop(), &endpoint, MultiQueue::NULL).unwrap();
        let previous = watcher.handle.clone();
        assert_eq!(socket_watcher_start(&mut watcher, 8), 0);
        assert!(previous.native.is_closed());
        assert_ne!(previous.as_ptr(), watcher.handle.as_ptr());
        assert!(watcher.handle.is_listening());
        drop(watcher);
        loop_poll_events(main_loop(), 0);
        assert!(!path.0.exists());
    }

    #[test]
    fn loop_shutdown_closes_a_listener_before_its_owner_is_dropped() {
        let (_lock, editor) = editor();
        let mut watcher =
            socket_watcher_init(main_loop(), c"127.0.0.1:0", MultiQueue::NULL).unwrap();
        assert_eq!(socket_watcher_start(&mut watcher, 8), 0);
        let handle = watcher.handle.clone();
        drop(editor);
        assert!(handle.native.is_closed());
        drop(watcher);
    }
}
