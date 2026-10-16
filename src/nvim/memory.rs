//! FFI bindings to Neovim's memory allocation functions (`src/nvim/memory.c`).

use std::ffi::c_void;

#[cfg(not(test))]
unsafe extern "C" {
    #[link_name = "xmalloc"]
    fn c_xmalloc(size: usize) -> *mut c_void;

    #[link_name = "xcalloc"]
    fn c_xcalloc(count: usize, size: usize) -> *mut c_void;

    #[link_name = "xrealloc"]
    fn c_xrealloc(ptr: *mut c_void, size: usize) -> *mut c_void;

    #[link_name = "xmallocz"]
    fn c_xmallocz(size: usize) -> *mut c_void;

    #[link_name = "xstrdup"]
    fn c_xstrdup(str_ptr: *const std::ffi::c_char) -> *mut std::ffi::c_char;

    #[link_name = "xfree"]
    fn c_xfree(ptr: *mut c_void);
}

#[cfg(test)]
unsafe extern "C" {
    #[link_name = "malloc"]
    fn crt_malloc(size: usize) -> *mut c_void;

    #[link_name = "calloc"]
    fn crt_calloc(count: usize, size: usize) -> *mut c_void;

    #[link_name = "realloc"]
    fn crt_realloc(ptr: *mut c_void, size: usize) -> *mut c_void;

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

/// Reallocate `ptr` to `size` bytes using Neovim's `xrealloc`.
///
/// # Safety
/// `ptr` must be null or previously allocated by Neovim's allocator.
#[inline]
pub unsafe fn xrealloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    #[cfg(not(test))]
    unsafe {
        c_xrealloc(ptr, size)
    }
    #[cfg(test)]
    unsafe {
        let out = crt_realloc(ptr, size.max(1));
        assert!(!out.is_null(), "out of memory in test xrealloc");
        out
    }
}

/// Allocate `size + 1` bytes using Neovim's `xmallocz` and zero the byte at `size`.
///
/// # Safety
/// Returned pointer must be freed with [`xfree`].
#[inline]
pub unsafe fn xmallocz(size: usize) -> *mut c_void {
    #[cfg(not(test))]
    unsafe {
        c_xmallocz(size)
    }
    #[cfg(test)]
    unsafe {
        let total = size.checked_add(1).expect("overflow in test xmallocz");
        let ptr = xmalloc(total).cast::<u8>();
        *ptr.add(size) = 0;
        ptr.cast::<c_void>()
    }
}

/// Duplicate a NUL-terminated C string using Neovim's `xstrdup`.
///
/// # Safety
/// `str_ptr` must be a valid NUL-terminated C string; returned pointer must be freed with [`xfree`].
#[inline]
pub unsafe fn xstrdup(str_ptr: *const std::ffi::c_char) -> *mut std::ffi::c_char {
    #[cfg(not(test))]
    unsafe {
        c_xstrdup(str_ptr)
    }
    #[cfg(test)]
    unsafe {
        let bytes = std::ffi::CStr::from_ptr(str_ptr).to_bytes_with_nul();
        let out = xmalloc(bytes.len()).cast::<u8>();
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
        out.cast::<std::ffi::c_char>()
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
