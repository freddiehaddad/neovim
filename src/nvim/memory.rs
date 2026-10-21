//! Memory allocation, arena allocator, string utilities, and linked-list mergesort
//! (`src/nvim/memory.c`).

use crate::api::ui::remote_ui_free_all_mem;
use crate::buffer_updates::buf_free_callbacks;
use crate::errors::*;
use crate::ex_cmds::free_prev_shellcmd;
use crate::ex_docmd::{do_cmdline_cmd, free_cd_dir};
use crate::globals::*;
use crate::input::close_all_scripts;
use crate::lua::executor::nlua_free_all_mem;
use crate::main_init::preserve_exit;
use crate::memfile::mf_release_all;
use crate::message::{clear_sb_text, msg_hist_clear, reset_last_sourcing, semsg, set_keep_msg};
use crate::option::free_all_options;
use crate::option_vars::*;
use crate::register::set_expr_line;
use crate::ui::ui_free_all_mem;
use crate::ui_client::ui_client_free_all_mem;
use std::cell::UnsafeCell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

pub const ARENA_BLOCK_SIZE: usize = 4096;
const REUSE_MAX: usize = 4;

pub type MemMalloc = unsafe extern "C" fn(size: usize) -> *mut c_void;
pub type MemFree = unsafe extern "C" fn(ptr: *mut c_void);
pub type MemCalloc = unsafe extern "C" fn(count: usize, size: usize) -> *mut c_void;
pub type MemRealloc = unsafe extern "C" fn(ptr: *mut c_void, size: usize) -> *mut c_void;

pub type MergeSortGetFunc = Option<unsafe extern "C" fn(node: *mut c_void) -> *mut c_void>;
pub type MergeSortSetFunc = Option<unsafe extern "C" fn(node: *mut c_void, target: *mut c_void)>;
pub type MergeSortCompareFunc =
    Option<unsafe extern "C" fn(a: *const c_void, b: *const c_void) -> c_int>;

#[repr(C)]
pub struct ConsumedBlk {
    pub prev: *mut ConsumedBlk,
}

pub type ArenaMem = *mut ConsumedBlk;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Arena {
    pub cur_blk: *mut c_char,
    pub pos: usize,
    pub size: usize,
}

unsafe extern "C" {
    #[link_name = "malloc"]
    pub(crate) fn crt_malloc(size: usize) -> *mut c_void;
    #[link_name = "free"]
    pub(crate) fn crt_free(ptr: *mut c_void);
    #[link_name = "calloc"]
    pub(crate) fn crt_calloc(count: usize, size: usize) -> *mut c_void;
    #[link_name = "realloc"]
    pub(crate) fn crt_realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
}

/// Unit-testable `malloc` hook (`mem_malloc`).
#[unsafe(no_mangle)]
pub static mut mem_malloc: MemMalloc = crt_malloc;
/// Unit-testable `free` hook (`mem_free`).
#[unsafe(no_mangle)]
pub static mut mem_free: MemFree = crt_free;
/// Unit-testable `calloc` hook (`mem_calloc`).
#[unsafe(no_mangle)]
pub static mut mem_calloc: MemCalloc = crt_calloc;
/// Unit-testable `realloc` hook (`mem_realloc`).
#[unsafe(no_mangle)]
pub static mut mem_realloc: MemRealloc = crt_realloc;

/// Indicates that `free_all_mem` function was or is running.
#[unsafe(no_mangle)]
pub static mut entered_free_all_mem: bool = false;

static TRYING_TO_FREE: AtomicBool = AtomicBool::new(false);

struct ArenaReuseState {
    head: UnsafeCell<*mut ConsumedBlk>,
    count: UnsafeCell<usize>,
}

unsafe impl Sync for ArenaReuseState {}

static ARENA_REUSE: ArenaReuseState = ArenaReuseState {
    head: UnsafeCell::new(std::ptr::null_mut()),
    count: UnsafeCell::new(0),
};

#[inline]
unsafe fn raw_malloc(size: usize) -> *mut c_void {
    unsafe { (mem_malloc)(size) }
}

#[inline]
unsafe fn raw_free(ptr: *mut c_void) {
    unsafe { (mem_free)(ptr) }
}

#[inline]
unsafe fn raw_calloc(count: usize, size: usize) -> *mut c_void {
    unsafe { (mem_calloc)(count, size) }
}

#[inline]
unsafe fn raw_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { (mem_realloc)(ptr, size) }
}

unsafe fn try_to_free_memory() {
    if TRYING_TO_FREE.swap(true, Ordering::Relaxed) {
        return;
    }

    unsafe {
        clear_sb_text(true);
        mf_release_all();
    }
    unsafe {
        arena_free_reuse_blks();
    }

    TRYING_TO_FREE.store(false, Ordering::Relaxed);
}

unsafe fn do_outofmem_msg(size: usize) {
    unsafe {
        if did_outofmem_msg {
            return;
        }
        emsg_silent = 0;
        did_outofmem_msg = true;
        semsg!(
            c"E342: Out of memory!  (allocating %llu bytes)".as_ptr(),
            size as u64,
        );
    }
}

unsafe fn out_of_memory_exit() -> ! {
    unsafe { preserve_exit(e_outofmem.as_ptr()) }
}

/// Free all cached reusable arena blocks (`arena_free_reuse_blks`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_free_reuse_blks() {
    unsafe {
        let count_ptr = ARENA_REUSE.count.get();
        let head_ptr = ARENA_REUSE.head.get();
        while *count_ptr > 0 {
            let blk = *head_ptr;
            *head_ptr = (*blk).prev;
            xfree(blk.cast::<c_void>());
            *count_ptr -= 1;
        }
    }
}

/// `malloc()` wrapper that tries to free caches before failing (`try_malloc`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn try_malloc(size: usize) -> *mut c_void {
    let allocated_size = size.max(1);
    let mut ret = unsafe { raw_malloc(allocated_size) };
    if ret.is_null() {
        unsafe {
            try_to_free_memory();
            ret = raw_malloc(allocated_size);
        }
    }
    ret
}

/// `try_malloc()` wrapper that reports E342 on failure (`verbose_try_malloc`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn verbose_try_malloc(size: usize) -> *mut c_void {
    let ret = unsafe { try_malloc(size) };
    if ret.is_null() {
        unsafe {
            do_outofmem_msg(size);
        }
    }
    ret
}

/// `malloc()` wrapper that never returns null (`xmalloc`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmalloc(size: usize) -> *mut c_void {
    let ret = unsafe { try_malloc(size) };
    if ret.is_null() {
        unsafe { out_of_memory_exit() }
    }
    ret
}

/// `free()` wrapper (`xfree`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xfree(ptr: *mut c_void) {
    unsafe {
        raw_free(ptr);
    }
}

/// `calloc()` wrapper that never returns null (`xcalloc`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xcalloc(count: usize, size: usize) -> *mut c_void {
    let (allocated_count, allocated_size) = if count != 0 && size != 0 {
        (count, size)
    } else {
        (1, 1)
    };
    let mut ret = unsafe { raw_calloc(allocated_count, allocated_size) };
    if ret.is_null() {
        unsafe {
            try_to_free_memory();
            ret = raw_calloc(allocated_count, allocated_size);
            if ret.is_null() {
                out_of_memory_exit();
            }
        }
    }
    ret
}

/// `realloc()` wrapper that never returns null (`xrealloc`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xrealloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    let allocated_size = size.max(1);
    let mut ret = unsafe { raw_realloc(ptr, allocated_size) };
    if ret.is_null() {
        unsafe {
            try_to_free_memory();
            ret = raw_realloc(ptr, allocated_size);
            if ret.is_null() {
                out_of_memory_exit();
            }
        }
    }
    ret
}

/// Allocate `size + 1` bytes and NUL-terminate at `size` (`xmallocz`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmallocz(size: usize) -> *mut c_void {
    let Some(total_size) = size.checked_add(1) else {
        unsafe {
            preserve_exit(c"Nvim: Data too large to fit into virtual memory space\n".as_ptr());
        }
    };
    let ret = unsafe { xmalloc(total_size).cast::<u8>() };
    unsafe {
        *ret.add(size) = 0;
    }
    ret.cast::<c_void>()
}

/// Allocate `len + 1` bytes, copy `data[..len]`, and NUL-terminate (`xmemdupz`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmemdupz(data: *const c_void, len: usize) -> *mut c_void {
    let ret = unsafe { xmallocz(len).cast::<u8>() };
    if len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(data.cast::<u8>(), ret, len);
        }
    }
    ret.cast::<c_void>()
}

/// Copy `len` bytes from `src` to `dst` and NUL-terminate at `dst[len]` (`xmemcpyz`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmemcpyz(dst: *mut c_void, src: *const c_void, len: usize) -> *mut c_void {
    if len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), len);
        }
    }
    unsafe {
        *dst.cast::<u8>().add(len) = 0;
    }
    dst
}

/// Portable `strnlen` (`xstrnlen`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstrnlen(s: *const c_char, n: usize) -> usize {
    let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), n) };
    slice.iter().position(|&b| b == 0).unwrap_or(n)
}

/// Find `c` in `str_ptr` or return a pointer to its terminating NUL (`xstrchrnul`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstrchrnul(str_ptr: *const c_char, c: c_char) -> *mut c_char {
    let target = c as u8;
    let mut p = str_ptr.cast::<u8>();
    loop {
        let b = unsafe { *p };
        if b == target || b == 0 {
            return p.cast::<c_char>().cast_mut();
        }
        p = unsafe { p.add(1) };
    }
}

/// Find `c` in `addr[..size]` or return `addr + size` (`xmemscan`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmemscan(addr: *const c_void, c: c_char, size: usize) -> *mut c_void {
    if size == 0 {
        return addr.cast_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(addr.cast::<u8>(), size) };
    let offset = slice.iter().position(|&b| b == (c as u8)).unwrap_or(size);
    unsafe { addr.cast::<u8>().add(offset).cast::<c_void>().cast_mut() }
}

/// Replace every occurrence of non-NUL `c` in C string `str_ptr` with `x` (`strchrsub`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchrsub(str_ptr: *mut c_char, c: c_char, x: c_char) {
    assert_ne!(c, 0);
    let target = c as u8;
    let replacement = x as u8;
    let mut p = str_ptr.cast::<u8>();
    loop {
        let b = unsafe { *p };
        if b == 0 {
            break;
        }
        if b == target {
            unsafe {
                *p = replacement;
            }
        }
        p = unsafe { p.add(1) };
    }
}

/// Replace every occurrence of `c` in `data[..len]` with `x` (`memchrsub`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memchrsub(data: *mut c_void, c: c_char, x: c_char, len: usize) {
    if len == 0 {
        return;
    }
    let target = c as u8;
    let replacement = x as u8;
    let slice = unsafe { std::slice::from_raw_parts_mut(data.cast::<u8>(), len) };
    for b in slice {
        if *b == target {
            *b = replacement;
        }
    }
}

/// Count occurrences of non-NUL `c` in C string `str_ptr` (`strcnt`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcnt(str_ptr: *const c_char, c: c_char) -> usize {
    assert_ne!(c, 0);
    let bytes = unsafe { CStr::from_ptr(str_ptr).to_bytes() };
    bytes.iter().filter(|&&b| b == (c as u8)).count()
}

/// Count occurrences of `c` in `data[..len]` (`memcnt`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcnt(data: *const c_void, c: c_char, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) };
    slice.iter().filter(|&&b| b == (c as u8)).count()
}

/// Copy `src` to `dst` (including NUL) and return a pointer to the trailing NUL (`xstpcpy`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstpcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    let len = unsafe { CStr::from_ptr(src).to_bytes().len() };
    unsafe {
        std::ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), len + 1);
        dst.add(len)
    }
}

/// Copy at most `maxlen` bytes from `src` to `dst`, zero-padding any remainder (`xstpncpy`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstpncpy(
    dst: *mut c_char,
    src: *const c_char,
    maxlen: usize,
) -> *mut c_char {
    if maxlen == 0 {
        return dst;
    }
    let src_bytes = unsafe { std::slice::from_raw_parts(src.cast::<u8>(), maxlen) };
    if let Some(srclen) = src_bytes.iter().position(|&b| b == 0) {
        unsafe {
            if srclen > 0 {
                std::ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), srclen);
            }
            std::ptr::write_bytes(dst.cast::<u8>().add(srclen), 0, maxlen - srclen);
            dst.add(srclen)
        }
    } else {
        unsafe {
            std::ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), maxlen);
            dst.add(maxlen)
        }
    }
}

/// Copy `src` into sized buffer `dst[..dsize]` (`xstrlcpy`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstrlcpy(dst: *mut c_char, src: *const c_char, dsize: usize) -> usize {
    let slen = unsafe { CStr::from_ptr(src).to_bytes().len() };
    if dsize != 0 {
        let len = slen.min(dsize - 1);
        unsafe {
            if len > 0 {
                std::ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), len);
            }
            *dst.add(len) = 0;
        }
    }
    slen
}

/// Append `src` to `dst[..dsize]`, supporting overlapping buffers (`xstrlcat`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstrlcat(dst: *mut c_char, src: *const c_char, dsize: usize) -> usize {
    assert!(dsize > 0);
    let dlen = unsafe { CStr::from_ptr(dst).to_bytes().len() };
    assert!(dlen < dsize);
    let slen = unsafe { CStr::from_ptr(src).to_bytes().len() };

    unsafe {
        if slen > dsize - dlen - 1 {
            std::ptr::copy(
                src.cast::<u8>(),
                dst.cast::<u8>().add(dlen),
                dsize - dlen - 1,
            );
            *dst.add(dsize - 1) = 0;
        } else {
            std::ptr::copy(src.cast::<u8>(), dst.cast::<u8>().add(dlen), slen + 1);
        }
    }

    slen + dlen
}

/// Duplicate a NUL-terminated C string using `xmalloc` (`xstrdup`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstrdup(str_ptr: *const c_char) -> *mut c_char {
    let len = unsafe { CStr::from_ptr(str_ptr).to_bytes().len() };
    unsafe { xmemdupz(str_ptr.cast::<c_void>(), len).cast::<c_char>() }
}

/// Duplicate `str_ptr` or allocate an empty string when `str_ptr` is null (`xstrdupnul`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstrdupnul(str_ptr: *const c_char) -> *mut c_char {
    if str_ptr.is_null() {
        unsafe { xmallocz(0).cast::<c_char>() }
    } else {
        unsafe { xstrdup(str_ptr) }
    }
}

/// Reverse search for byte `c` in `src[..len]` (`xmemrchr`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmemrchr(src: *const c_void, c: u8, len: usize) -> *mut c_void {
    if len == 0 {
        return std::ptr::null_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(src.cast::<u8>(), len) };
    if let Some(pos) = slice.iter().rposition(|&b| b == c) {
        unsafe { src.cast::<u8>().add(pos).cast::<c_void>().cast_mut() }
    } else {
        std::ptr::null_mut()
    }
}

/// Duplicate up to `len` bytes of `str_ptr` (`xstrndup`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xstrndup(str_ptr: *const c_char, len: usize) -> *mut c_char {
    let actual_len = unsafe { xstrnlen(str_ptr, len) };
    unsafe { xmemdupz(str_ptr.cast::<c_void>(), actual_len).cast::<c_char>() }
}

/// Duplicate `len` bytes from `data` (`xmemdup`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmemdup(data: *const c_void, len: usize) -> *mut c_void {
    let ret = unsafe { xmalloc(len).cast::<u8>() };
    if len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(data.cast::<u8>(), ret, len);
        }
    }
    ret.cast::<c_void>()
}

/// Return true if C string `a` equals Rust string `b` (`strequal_str`).
#[inline]
pub unsafe fn strequal_str(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == b.as_bytes() }
}

/// Return true if C strings `a` and `b` are equal (both may be null) (`strequal`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strequal(a: *const c_char, b: *const c_char) -> bool {
    if a.is_null() && b.is_null() {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == CStr::from_ptr(b).to_bytes() }
}

/// Return true if the first `n` characters of `a` and `b` are equal (`strnequal`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strnequal(a: *const c_char, b: *const c_char, n: usize) -> bool {
    if a.is_null() && b.is_null() {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }
    let la = unsafe { xstrnlen(a, n) };
    let lb = unsafe { xstrnlen(b, n) };
    if la != lb {
        return false;
    }
    let sa = unsafe { std::slice::from_raw_parts(a.cast::<u8>(), la) };
    let sb = unsafe { std::slice::from_raw_parts(b.cast::<u8>(), lb) };
    sa == sb
}

/// Write `time_` as 8 big-endian bytes into `buf[0..8]` (`time_to_bytes`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time_to_bytes(time_val: i64, buf: *mut u8) {
    let bytes = (time_val as u64).to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, 8);
    }
}

/// Iterative $O(N \log N)$ stable merge sort for doubly-linked lists (`mergesort_list`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mergesort_list(
    mut head: *mut c_void,
    get_next: MergeSortGetFunc,
    set_next: MergeSortSetFunc,
    get_prev: MergeSortGetFunc,
    set_prev: MergeSortSetFunc,
    compare: MergeSortCompareFunc,
) -> *mut c_void {
    let (Some(get_next), Some(set_next), Some(get_prev), Some(set_prev), Some(compare)) =
        (get_next, set_next, get_prev, set_prev, compare)
    else {
        return head;
    };

    if head.is_null() || unsafe { get_next(head).is_null() } {
        return head;
    }

    let mut n: c_int = 0;
    let mut curr = head;
    while !curr.is_null() {
        n += 1;
        curr = unsafe { get_next(curr) };
    }

    let mut size: c_int = 1;
    while size < n {
        let mut new_head: *mut c_void = std::ptr::null_mut();
        let mut tail: *mut c_void = std::ptr::null_mut();
        curr = head;

        while !curr.is_null() {
            let mut left = curr;
            let mut right = left;
            for _ in 0..size {
                if right.is_null() {
                    break;
                }
                right = unsafe { get_next(right) };
            }

            let mut next = right;
            for _ in 0..size {
                if next.is_null() {
                    break;
                }
                next = unsafe { get_next(next) };
            }

            let l_end = if !right.is_null() {
                unsafe { get_prev(right) }
            } else {
                std::ptr::null_mut()
            };
            if !l_end.is_null() {
                unsafe {
                    set_next(l_end, std::ptr::null_mut());
                }
            }
            if !right.is_null() {
                unsafe {
                    set_prev(right, std::ptr::null_mut());
                }
            }

            let r_end = if !next.is_null() {
                unsafe { get_prev(next) }
            } else {
                std::ptr::null_mut()
            };
            if !r_end.is_null() {
                unsafe {
                    set_next(r_end, std::ptr::null_mut());
                }
            }
            if !next.is_null() {
                unsafe {
                    set_prev(next, std::ptr::null_mut());
                }
            }

            let mut merged: *mut c_void = std::ptr::null_mut();
            let mut merged_tail: *mut c_void = std::ptr::null_mut();

            while !left.is_null() || !right.is_null() {
                let chosen = if left.is_null() {
                    let c = right;
                    right = unsafe { get_next(right) };
                    c
                } else if right.is_null() || unsafe { compare(left, right) <= 0 } {
                    let c = left;
                    left = unsafe { get_next(left) };
                    c
                } else {
                    let c = right;
                    right = unsafe { get_next(right) };
                    c
                };

                if !merged_tail.is_null() {
                    unsafe {
                        set_next(merged_tail, chosen);
                        set_prev(chosen, merged_tail);
                    }
                    merged_tail = chosen;
                } else {
                    merged = chosen;
                    merged_tail = chosen;
                    unsafe {
                        set_prev(chosen, std::ptr::null_mut());
                    }
                }
            }

            if new_head.is_null() {
                new_head = merged;
            } else {
                unsafe {
                    set_next(tail, merged);
                    set_prev(merged, tail);
                }
            }

            while unsafe { !get_next(merged_tail).is_null() } {
                merged_tail = unsafe { get_next(merged_tail) };
            }
            tail = merged_tail;
            curr = next;
        }

        head = new_head;
        size *= 2;
    }

    head
}

/// Finish allocations in `arena` and return an `ArenaMem` handle (`arena_finish`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_finish(arena: *mut Arena) -> ArenaMem {
    let a = unsafe { &mut *arena };
    let res = a.cur_blk.cast::<ConsumedBlk>();
    *a = Arena {
        cur_blk: std::ptr::null_mut(),
        pos: 0,
        size: 0,
    };
    res
}

/// Allocate a `ARENA_BLOCK_SIZE` block (`alloc_block`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_block() -> *mut c_char {
    unsafe {
        let count_ptr = ARENA_REUSE.count.get();
        let head_ptr = ARENA_REUSE.head.get();
        if *count_ptr > 0 {
            let retval = (*head_ptr).cast::<c_char>();
            *head_ptr = (*(*head_ptr)).prev;
            *count_ptr -= 1;
            retval
        } else {
            {
                arena_alloc_count += 1;
            }
            xmalloc(ARENA_BLOCK_SIZE).cast::<c_char>()
        }
    }
}

/// Allocate a fresh block on `arena` (`arena_alloc_block`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_alloc_block(arena: *mut Arena) {
    let a = unsafe { &mut *arena };
    let prev_blk = a.cur_blk.cast::<ConsumedBlk>();
    a.cur_blk = unsafe { alloc_block() };
    a.pos = 0;
    a.size = ARENA_BLOCK_SIZE;
    let blk = unsafe {
        arena_alloc(arena, std::mem::size_of::<ConsumedBlk>(), true).cast::<ConsumedBlk>()
    };
    unsafe {
        (*blk).prev = prev_blk;
    }
}

#[inline]
const fn arena_align_offset(off: usize) -> usize {
    let align = if std::mem::size_of::<*mut c_void>() > std::mem::size_of::<f64>() {
        std::mem::size_of::<*mut c_void>()
    } else {
        std::mem::size_of::<f64>()
    };
    (off + (align - 1)) & !(align - 1)
}

/// Allocate `size` bytes from `arena` (or via `xmalloc` if `arena` is null) (`arena_alloc`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_alloc(arena: *mut Arena, size: usize, align: bool) -> *mut c_void {
    if arena.is_null() {
        return unsafe { xmalloc(size) };
    }
    let a = unsafe { &mut *arena };
    if a.cur_blk.is_null() {
        unsafe {
            arena_alloc_block(arena);
        }
    }
    let mut alloc_pos = if align {
        arena_align_offset(a.pos)
    } else {
        a.pos
    };
    if alloc_pos + size > a.size {
        if size > (ARENA_BLOCK_SIZE - std::mem::size_of::<ConsumedBlk>()) >> 1 {
            unsafe {
                arena_alloc_count += 1;
            }
            let hdr_size = std::mem::size_of::<ConsumedBlk>();
            let aligned_hdr_size = if align {
                arena_align_offset(hdr_size)
            } else {
                hdr_size
            };
            let alloc = unsafe { xmalloc(size + aligned_hdr_size).cast::<u8>() };
            unsafe {
                let cur_blk = a.cur_blk.cast::<ConsumedBlk>();
                let fix_blk = alloc.cast::<ConsumedBlk>();
                (*fix_blk).prev = (*cur_blk).prev;
                (*cur_blk).prev = fix_blk;
                return alloc.add(aligned_hdr_size).cast::<c_void>();
            }
        } else {
            unsafe {
                arena_alloc_block(arena);
            }
            alloc_pos = if align {
                arena_align_offset(a.pos)
            } else {
                a.pos
            };
        }
    }

    let mem = unsafe { a.cur_blk.add(alloc_pos).cast::<c_void>() };
    a.pos = alloc_pos + size;
    mem
}

/// Release a `ARENA_BLOCK_SIZE` block back to the reuse pool or free it (`free_block`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_block(block: *mut c_char) {
    if block.is_null() {
        return;
    }
    unsafe {
        let count_ptr = ARENA_REUSE.count.get();
        let head_ptr = ARENA_REUSE.head.get();
        if *count_ptr < REUSE_MAX {
            let reuse_blk = block.cast::<ConsumedBlk>();
            (*reuse_blk).prev = *head_ptr;
            *head_ptr = reuse_blk;
            *count_ptr += 1;
        } else {
            xfree(block.cast::<c_void>());
        }
    }
}

/// Free all blocks belonging to `mem` (`arena_mem_free`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_mem_free(mem: ArenaMem) {
    let mut b = mem;
    if !b.is_null() {
        let reuse_blk = b;
        b = unsafe { (*b).prev };
        unsafe {
            free_block(reuse_blk.cast::<c_char>());
        }
    }
    while !b.is_null() {
        let prev = unsafe { (*b).prev };
        unsafe {
            xfree(b.cast::<c_void>());
        }
        b = prev;
    }
}

/// Allocate `size + 1` bytes on `arena` and NUL-terminate (`arena_allocz`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_allocz(arena: *mut Arena, size: usize) -> *mut c_char {
    let mem = unsafe { arena_alloc(arena, size + 1, false).cast::<u8>() };
    unsafe {
        *mem.add(size) = 0;
    }
    mem.cast::<c_char>()
}

/// Copy `buf[..size]` into a NUL-terminated buffer on `arena` (`arena_memdupz`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_memdupz(
    arena: *mut Arena,
    buf: *const c_char,
    size: usize,
) -> *mut c_char {
    let mem = unsafe { arena_allocz(arena, size).cast::<u8>() };
    if size > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(buf.cast::<u8>(), mem, size);
        }
    }
    mem.cast::<c_char>()
}

/// Duplicate C string `str_ptr` onto `arena` (`arena_strdup`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn arena_strdup(arena: *mut Arena, str_ptr: *const c_char) -> *mut c_char {
    let len = unsafe { CStr::from_ptr(str_ptr).to_bytes().len() };
    unsafe { arena_memdupz(arena, str_ptr, len) }
}

/// Free everything that we allocated (`free_all_mem`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_all_mem() {
    use crate::api::extmark::api_extmark_free_all_mem;
    use crate::arglist::alist_clear;
    use crate::autocmd::{autocmd_free_all_mem, block_autocmds, free_all_autocmds};
    use crate::buffer::{
        DOBUF_WIPE, bufref_T, bufref_valid, close_buffer, free_titles, set_bufref,
    };
    use crate::channel::channel_free_all_mem;
    use crate::cmdhist::init_history;
    use crate::decoration_provider::decor_free_all_mem;
    use crate::diff::diff_clear;
    use crate::drawline::drawline_free_all_mem;
    use crate::eval::eval_clear;
    use crate::ex_cmds::free_old_sub;
    use crate::ex_docmd::exarg_T;
    use crate::file_search::free_findfile;
    use crate::grid::grid_free_all_mem;
    use crate::highlight::clear_hl_tables;
    use crate::highlight_group::free_highlight;
    use crate::input::redo_free_all;
    use crate::input_cmdatom::atom_free_all;
    use crate::insert::free_last_insert;
    use crate::insexpand::free_insexpand_stuff;
    use crate::map::{Map, MapHash, Set};
    use crate::mapping::map_clear_mode;
    use crate::mark::free_all_marks;
    use crate::mcursor::mc_free_all;

    use crate::msgpack_rpc::channel::rpc_free_all_mem;
    use crate::os::env::free_homedir;
    use crate::os::users::free_users;
    use crate::quickfix::{check_quickfix_busy, free_quickfix};
    use crate::regexp::free_regexp_stuff;
    use crate::register::clear_registers;
    use crate::search::free_search_patterns;
    use crate::sign::free_signs;
    use crate::spell::spell_free_all;
    use crate::state::MAP_ALL_MODES;
    use crate::statusline::stl_clear_click_defs;
    use crate::tag::free_tag_stuff;
    use crate::ui_compositor::ui_comp_free_all_mem;
    use crate::usercmd::ex_comclear;
    use crate::window::{free_tabpage, win_free_all};

    unsafe fn destroy_int_pmap(map: *mut Map<c_int, *mut c_void>) {
        unsafe {
            xfree((*map).set.keys.cast());
            xfree((*map).set.h.hash.cast());
            (*map).set = Set {
                h: MapHash {
                    n_buckets: 0,
                    size: 0,
                    n_occupied: 0,
                    upper_bound: 0,
                    n_keys: 0,
                    keys_capacity: 0,
                    hash: std::ptr::null_mut(),
                },
                keys: std::ptr::null_mut(),
            };
            xfree((*map).values.cast());
            (*map).values = std::ptr::null_mut();
        }
    }

    unsafe {
        if entered_free_all_mem {
            nlua_free_all_mem();
            return;
        }
        entered_free_all_mem = true;
        block_autocmds();

        p_ea = 0;
        if !first_tabpage.is_null() && !(*first_tabpage).tp_next.is_null() {
            do_cmdline_cmd(c"tabonly!".as_ptr());
        }

        spell_free_all();
        ex_comclear(std::ptr::null_mut::<exarg_T>());

        if !curbuf.is_null() {
            do_cmdline_cmd(c"aunmenu *".as_ptr());
            do_cmdline_cmd(c"tlunmenu *".as_ptr());
            do_cmdline_cmd(c"menutranslate clear".as_ptr());

            map_clear_mode(curbuf, MAP_ALL_MODES, false, false);
            map_clear_mode(curbuf, MAP_ALL_MODES, false, true);
            do_cmdline_cmd(c"breakdel *".as_ptr());
            do_cmdline_cmd(c"profdel *".as_ptr());
            do_cmdline_cmd(c"set keymap=".as_ptr());
        }

        free_titles();
        free_findfile();

        free_all_autocmds();
        free_all_marks();
        alist_clear(&raw mut global_alist);
        free_homedir();
        free_users();
        free_search_patterns();
        free_old_sub();
        free_last_insert();
        free_insexpand_stuff();
        free_prev_shellcmd();
        free_regexp_stuff();
        free_tag_stuff();
        free_cd_dir();
        free_signs();
        set_expr_line(std::ptr::null_mut());
        if !curtab.is_null() {
            diff_clear(curtab);
        }
        clear_sb_text(true);

        xfree(last_cmdline.cast());
        xfree(new_last_cmdline.cast());
        set_keep_msg(std::ptr::null(), 0);

        p_hi = 0;
        init_history();

        free_quickfix();
        close_all_scripts();

        if !curwin.is_null() {
            win_free_all();
        }

        free_all_options();

        p_acd = 0;
        let mut buf = firstbuf;
        while !buf.is_null() {
            let mut bufref = bufref_T {
                br_buf: std::ptr::null_mut(),
                br_fnum: 0,
                br_buf_free_count: 0,
            };
            set_bufref(&raw mut bufref, buf);
            let nextbuf = (*buf).b_next;
            (*buf).b_nwindows = 0;

            buf_free_callbacks(buf);
            close_buffer(std::ptr::null_mut(), buf, DOBUF_WIPE, false, false, false);
            buf = if bufref_valid(&raw mut bufref) {
                nextbuf
            } else {
                firstbuf
            };
        }

        clear_registers();
        redo_free_all();
        free_highlight();
        reset_last_sourcing();

        if !first_tabpage.is_null() {
            free_tabpage(first_tabpage);
            first_tabpage = std::ptr::null_mut();
        }

        msg_hist_clear(0);

        channel_free_all_mem();
        eval_clear();
        api_extmark_free_all_mem();
        atom_free_all();
        mc_free_all();

        destroy_int_pmap(&raw mut buffer_handles);
        destroy_int_pmap(&raw mut window_handles);
        destroy_int_pmap(&raw mut tabpage_handles);

        grid_free_all_mem();
        stl_clear_click_defs(tab_page_click_defs, tab_page_click_defs_size);
        xfree(tab_page_click_defs.cast());

        clear_hl_tables(false);
        check_quickfix_busy();
        decor_free_all_mem();
        drawline_free_all_mem();

        if ui_client_channel_id != 0 {
            ui_client_free_all_mem();
        }

        remote_ui_free_all_mem();
        ui_free_all_mem();
        ui_comp_free_all_mem();
        nlua_free_all_mem();
        rpc_free_all_mem();
        autocmd_free_all_mem();

        arena_free_reuse_blks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xstrlcat_and_arena_lifecycle() {
        let mut buf = [0 as c_char; 8];
        unsafe {
            assert_eq!(xstrlcpy(buf.as_mut_ptr(), c"abc".as_ptr(), buf.len()), 3);
            assert_eq!(xstrlcat(buf.as_mut_ptr(), c"defgh".as_ptr(), buf.len()), 8);
            assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), "abcdefg");

            let mut arena = Arena {
                cur_blk: std::ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let s1 = arena_strdup(&mut arena, c"hello arena".as_ptr());
            let big = arena_alloc(&mut arena, 8192, true);
            assert!(!big.is_null());
            assert_eq!(CStr::from_ptr(s1).to_str().unwrap(), "hello arena");
            let mem = arena_finish(&mut arena);
            arena_mem_free(mem);
            arena_free_reuse_blks();
        }
    }
}
