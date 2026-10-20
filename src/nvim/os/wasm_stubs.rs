//! WebAssembly / Emscripten libuv and pthread shims (`src/wasm/wasm_stubs.c`).

#![allow(non_camel_case_types, non_snake_case, clippy::missing_safety_doc)]

use std::ffi::{c_char, c_double, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

pub const UV_BROWSER_MAX_FD: usize = 256;

const UV_EINVAL: c_int = -22;
const EINVAL: c_int = 22;

#[repr(C)]
pub struct UvBrowserSharedState {
    pub generation: AtomicU32,
    pub readable: [AtomicI32; UV_BROWSER_MAX_FD],
    pub writable: [AtomicI32; UV_BROWSER_MAX_FD],
}

impl UvBrowserSharedState {
    pub const fn new() -> Self {
        #[allow(clippy::declare_interior_mutable_const)]
        const ZERO_I32: AtomicI32 = AtomicI32::new(0);
        Self {
            generation: AtomicU32::new(0),
            readable: [ZERO_I32; UV_BROWSER_MAX_FD],
            writable: [ZERO_I32; UV_BROWSER_MAX_FD],
        }
    }
}

impl Default for UvBrowserSharedState {
    fn default() -> Self {
        Self::new()
    }
}

static G_SHARED_STATE: UvBrowserSharedState = UvBrowserSharedState::new();

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv_get_free_memory() -> u64 {
    8 * 1024 * 1024
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv_get_total_memory() -> u64 {
    256 * 1024 * 1024
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv_get_available_memory() -> u64 {
    uv_get_free_memory()
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv_get_constrained_memory() -> u64 {
    uv_get_total_memory()
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_loadavg(avg: *mut c_double) {
    if !avg.is_null() {
        unsafe {
            *avg.add(0) = 0.0;
            *avg.add(1) = 0.0;
            *avg.add(2) = 0.0;
        }
    }
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub unsafe extern "C" fn uv_resident_set_memory(rss: *mut usize) -> c_int {
    if rss.is_null() {
        return UV_EINVAL;
    }
    unsafe {
        *rss = (uv_get_total_memory() / 2) as usize;
    }
    0
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv_browser_get_shared_state_ptr() -> *mut c_void {
    (&raw const G_SHARED_STATE).cast_mut().cast()
}

#[cfg(target_os = "emscripten")]
unsafe extern "C" {
    fn emscripten_get_now() -> c_double;
    fn emscripten_futex_wake(addr: *const c_void, count: c_int) -> c_int;
    fn emscripten_futex_wait(addr: *const c_void, val: u32, max_wait_ms: c_double) -> c_int;
    fn uv_ip4_addr(ip: *const c_char, port: c_int, addr: *mut c_void) -> c_int;
    fn uv_update_time(loop_: *mut c_void);
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv_browser_set_readable(fd: c_int, nbytes: i32) {
    if fd < 0 || (fd as usize) >= UV_BROWSER_MAX_FD {
        return;
    }
    G_SHARED_STATE.readable[fd as usize].store(nbytes, Ordering::SeqCst);
    G_SHARED_STATE.generation.fetch_add(1, Ordering::SeqCst);
    #[cfg(target_os = "emscripten")]
    unsafe {
        let _ = emscripten_futex_wake((&raw const G_SHARED_STATE.generation).cast(), 1);
    }
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv_browser_set_writable(fd: c_int, nbytes: i32) {
    if fd < 0 || (fd as usize) >= UV_BROWSER_MAX_FD {
        return;
    }
    G_SHARED_STATE.writable[fd as usize].store(nbytes, Ordering::SeqCst);
    G_SHARED_STATE.generation.fetch_add(1, Ordering::SeqCst);
    #[cfg(target_os = "emscripten")]
    unsafe {
        let _ = emscripten_futex_wake((&raw const G_SHARED_STATE.generation).cast(), 1);
    }
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub unsafe extern "C" fn uv__platform_invalidate_fd(_loop: *mut c_void, fd: c_int) {
    if fd < 0 || (fd as usize) >= UV_BROWSER_MAX_FD {
        return;
    }
    G_SHARED_STATE.readable[fd as usize].store(0, Ordering::SeqCst);
    G_SHARED_STATE.writable[fd as usize].store(0, Ordering::SeqCst);
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv__platform_loop_delete(_loop: *mut c_void) {}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv__io_check_fd(_loop: *mut c_void, _fd: c_int) -> c_int {
    0
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn uv__io_fork(_loop: *mut c_void) -> c_int {
    0
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub unsafe extern "C" fn wasm_pthread_getname_np(
    _thread: usize,
    name: *mut c_char,
    len: usize,
) -> c_int {
    if name.is_null() || len < 1 {
        return EINVAL;
    }
    let src = b"nvim-main";
    let copy_len = src.len().min(len - 1);
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr().cast::<c_char>(), name, copy_len);
        *name.add(copy_len) = 0;
    }
    0
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn wasm_pthread_setname_np(_thread: usize, _name: *const c_char) -> c_int {
    0
}

#[cfg_attr(target_os = "emscripten", unsafe(no_mangle))]
pub extern "C" fn wasm_pthread_setschedparam(
    _thread: usize,
    _policy: c_int,
    _param: *const c_void,
) -> c_int {
    0
}

#[cfg(target_os = "emscripten")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uv_uptime(uptime: *mut c_double) -> c_int {
    if uptime.is_null() {
        return UV_EINVAL;
    }
    unsafe {
        *uptime = emscripten_get_now() / 1000.0;
    }
    0
}

#[cfg(target_os = "emscripten")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uv_exepath(buffer: *mut c_char, size: *mut usize) -> c_int {
    if buffer.is_null() || size.is_null() {
        return UV_EINVAL;
    }
    let exepath = b"/nvim.wasm";
    let len = exepath.len();
    unsafe {
        if *size <= len {
            *size = len + 1;
            return UV_ENOBUFS;
        }
        ptr::copy_nonoverlapping(exepath.as_ptr().cast::<c_char>(), buffer, len);
        *buffer.add(len) = 0;
        *size = len;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn memory_and_shared_state_shims_work() {
        unsafe {
            assert_eq!(uv_get_free_memory(), 8 * 1024 * 1024);
            assert_eq!(uv_get_total_memory(), 256 * 1024 * 1024);
            assert_eq!(uv_get_available_memory(), uv_get_free_memory());
            assert_eq!(uv_get_constrained_memory(), uv_get_total_memory());

            let mut rss = 0usize;
            assert_eq!(uv_resident_set_memory(ptr::null_mut()), UV_EINVAL);
            assert_eq!(uv_resident_set_memory(&raw mut rss), 0);
            assert_eq!(rss, 128 * 1024 * 1024);

            uv_browser_set_readable(3, 64);
            uv_browser_set_writable(3, 128);
            assert_eq!(G_SHARED_STATE.readable[3].load(Ordering::SeqCst), 64);
            assert_eq!(G_SHARED_STATE.writable[3].load(Ordering::SeqCst), 128);
            uv__platform_invalidate_fd(ptr::null_mut(), 3);
            assert_eq!(G_SHARED_STATE.readable[3].load(Ordering::SeqCst), 0);
            assert_eq!(G_SHARED_STATE.writable[3].load(Ordering::SeqCst), 0);

            let mut buf = [0 as c_char; 16];
            assert_eq!(wasm_pthread_getname_np(0, buf.as_mut_ptr(), buf.len()), 0);
            assert_eq!(CStr::from_ptr(buf.as_ptr()).to_bytes(), b"nvim-main");
        }
    }
}
