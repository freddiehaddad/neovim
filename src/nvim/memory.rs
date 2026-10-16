//! FFI bindings to Neovim's memory allocation functions (`src/nvim/memory.c`).

use std::ffi::c_void;

#[cfg(not(test))]
unsafe extern "C" {
    #[link_name = "xmalloc"]
    fn c_xmalloc(size: usize) -> *mut c_void;

    #[link_name = "xcalloc"]
    fn c_xcalloc(count: usize, size: usize) -> *mut c_void;

    #[link_name = "xfree"]
    fn c_xfree(ptr: *mut c_void);
}

#[cfg(test)]
unsafe extern "C" {
    #[link_name = "malloc"]
    fn crt_malloc(size: usize) -> *mut c_void;

    #[link_name = "calloc"]
    fn crt_calloc(count: usize, size: usize) -> *mut c_void;

    #[link_name = "free"]
    fn crt_free(ptr: *mut c_void);
}

/// Allocate `size` bytes using Neovim's `xmalloc` (never returns null; `size == 0` allocates 1 byte).
///
/// # Safety
/// Returned pointer must be freed with [`xfree`].
#[inline]
pub unsafe fn xmalloc(size: usize) -> *mut c_void {
    #[cfg(not(test))]
    unsafe {
        c_xmalloc(size)
    }
    #[cfg(test)]
    unsafe {
        let ptr = crt_malloc(size.max(1));
        assert!(!ptr.is_null(), "out of memory in test xmalloc");
        ptr
    }
}

/// Allocate zero-initialized `count * size` bytes using Neovim's `xcalloc`.
///
/// # Safety
/// Returned pointer must be freed with [`xfree`].
#[inline]
pub unsafe fn xcalloc(count: usize, size: usize) -> *mut c_void {
    #[cfg(not(test))]
    unsafe {
        c_xcalloc(count, size)
    }
    #[cfg(test)]
    unsafe {
        let ptr = crt_calloc(count.max(1), size.max(1));
        assert!(!ptr.is_null(), "out of memory in test xcalloc");
        ptr
    }
}

/// Free a pointer allocated by [`xmalloc`].
///
/// # Safety
/// `ptr` must be null or a pointer previously allocated by Neovim's allocator (`xmalloc`, etc.).
#[inline]
pub unsafe fn xfree(ptr: *mut c_void) {
    #[cfg(not(test))]
    unsafe {
        c_xfree(ptr);
    }
    #[cfg(test)]
    unsafe {
        crt_free(ptr);
    }
}
