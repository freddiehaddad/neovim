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
use crate::message::{clear_sb_text, msg_hist_clear, reset_last_sourcing, set_keep_msg};
use crate::option::free_all_options;
use crate::option_vars::*;
use crate::register::set_expr_line;
use crate::ui::ui_free_all_mem;
use crate::ui_client::ui_client_free_all_mem;
use std::ffi::{CStr, CString, c_char, c_void};

pub const ARENA_BLOCK_SIZE: usize = 4096;
const REUSE_MAX: usize = 4;

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

impl Arena {
    #[inline]
    pub const fn new() -> Self {
        Self {
            cur_blk: std::ptr::null_mut(),
            pos: 0,
            size: 0,
        }
    }
}

impl Default for Arena {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

unsafe extern "C" {
    #[link_name = "malloc"]
    fn crt_malloc(size: usize) -> *mut c_void;
    #[link_name = "free"]
    fn crt_free(ptr: *mut c_void);
    #[link_name = "calloc"]
    fn crt_calloc(count: usize, size: usize) -> *mut c_void;
    #[link_name = "realloc"]
    fn crt_realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
}

/// Indicates that `free_all_mem` function was or is running.
pub static mut entered_free_all_mem: bool = false;

static mut TRYING_TO_FREE: bool = false;
static mut ARENA_REUSE_HEAD: *mut ConsumedBlk = std::ptr::null_mut();
static mut ARENA_REUSE_COUNT: usize = 0;

#[inline]
unsafe fn raw_malloc(size: usize) -> *mut c_void {
    unsafe { crt_malloc(size) }
}

#[inline]
unsafe fn raw_free(ptr: *mut c_void) {
    unsafe { crt_free(ptr) }
}

#[inline]
unsafe fn raw_calloc(count: usize, size: usize) -> *mut c_void {
    unsafe { crt_calloc(count, size) }
}

#[inline]
unsafe fn raw_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { crt_realloc(ptr, size) }
}

unsafe fn try_to_free_memory() {
    unsafe {
        if TRYING_TO_FREE {
            return;
        }
        TRYING_TO_FREE = true;

        clear_sb_text(true);
        mf_release_all();
        arena_free_reuse_blks();

        TRYING_TO_FREE = false;
    }
}

unsafe fn out_of_memory_exit() -> ! {
    unsafe { preserve_exit(e_outofmem.as_ptr()) }
}

/// Free all cached reusable arena blocks (`arena_free_reuse_blks`).
pub unsafe fn arena_free_reuse_blks() {
    unsafe {
        while ARENA_REUSE_COUNT > 0 {
            let blk = ARENA_REUSE_HEAD;
            ARENA_REUSE_HEAD = (*blk).prev;
            xfree(blk.cast::<c_void>());
            ARENA_REUSE_COUNT -= 1;
        }
    }
}

/// `malloc()` wrapper that tries to free caches before failing (`try_malloc`).
pub unsafe fn try_malloc(size: usize) -> *mut c_void {
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

/// `malloc()` wrapper that never returns null (`xmalloc`).
pub unsafe fn xmalloc(size: usize) -> *mut c_void {
    let ret = unsafe { try_malloc(size) };
    if ret.is_null() {
        unsafe { out_of_memory_exit() }
    }
    ret
}

/// `free()` wrapper (`xfree`).
pub unsafe fn xfree(ptr: *mut c_void) {
    unsafe {
        raw_free(ptr);
    }
}

/// `calloc()` wrapper that never returns null (`xcalloc`).
pub unsafe fn xcalloc(count: usize, size: usize) -> *mut c_void {
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
pub unsafe fn xrealloc(ptr: *mut c_void, size: usize) -> *mut c_void {
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
pub unsafe fn xmallocz(size: usize) -> *mut c_void {
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
pub unsafe fn xmemdupz(data: *const c_void, len: usize) -> *mut c_void {
    let ret = unsafe { xmallocz(len).cast::<u8>() };
    if len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(data.cast::<u8>(), ret, len);
        }
    }
    ret.cast::<c_void>()
}

/// Copy `len` bytes from `src` to `dst` and NUL-terminate at `dst[len]` (`xmemcpyz`).
pub unsafe fn xmemcpyz(dst: *mut c_void, src: *const c_void, len: usize) -> *mut c_void {
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
pub unsafe fn xstrnlen(s: *const c_char, n: usize) -> usize {
    let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), n) };
    slice.iter().position(|&b| b == 0).unwrap_or(n)
}

/// Find `c` in `str_ptr` or return a pointer to its terminating NUL (`xstrchrnul`).
pub unsafe fn xstrchrnul(str_ptr: *const c_char, c: c_char) -> *mut c_char {
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
pub unsafe fn xmemscan(addr: *const c_void, c: c_char, size: usize) -> *mut c_void {
    if size == 0 {
        return addr.cast_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(addr.cast::<u8>(), size) };
    let offset = slice.iter().position(|&b| b == (c as u8)).unwrap_or(size);
    unsafe { addr.cast::<u8>().add(offset).cast::<c_void>().cast_mut() }
}

/// Replace every occurrence of non-NUL `c` in C string `str_ptr` with `x` (`strchrsub`).
pub unsafe fn strchrsub(str_ptr: *mut c_char, c: c_char, x: c_char) {
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
pub unsafe fn memchrsub(data: *mut c_void, c: c_char, x: c_char, len: usize) {
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

/// Count occurrences of `c` in `data[..len]` (`memcnt`).
pub unsafe fn memcnt(data: *const c_void, c: c_char, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) };
    slice.iter().filter(|&&b| b == (c as u8)).count()
}

/// Copy `src` to `dst` (including NUL) and return a pointer to the trailing NUL (`xstpcpy`).
pub unsafe fn xstpcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    let len = unsafe { CStr::from_ptr(src).to_bytes().len() };
    unsafe {
        std::ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), len + 1);
        dst.add(len)
    }
}

/// Copy `src` into sized buffer `dst[..dsize]` (`xstrlcpy`).
pub unsafe fn xstrlcpy(dst: *mut c_char, src: *const c_char, dsize: usize) -> usize {
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
pub unsafe fn xstrlcat(dst: *mut c_char, src: *const c_char, dsize: usize) -> usize {
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

/// Copy an `xmalloc`-allocated C string into an owned `CString` and free `ptr`.
pub unsafe fn cstr_take(ptr: *mut c_char) -> Option<CString> {
    if ptr.is_null() {
        None
    } else {
        let s = unsafe { CStr::from_ptr(ptr).to_owned() };
        unsafe { xfree(ptr.cast()) };
        Some(s)
    }
}

/// Copy `len` bytes from an `xmalloc`-allocated buffer into a `Vec<u8>` and free `ptr`.
pub unsafe fn bytes_take(ptr: *mut c_char, len: usize) -> Option<Vec<u8>> {
    if ptr.is_null() {
        None
    } else {
        let v = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len).to_vec() };
        unsafe { xfree(ptr.cast()) };
        Some(v)
    }
}

/// Duplicate a NUL-terminated C string using `xmalloc` (`xstrdup`).
pub unsafe fn xstrdup(str_ptr: *const c_char) -> *mut c_char {
    let len = unsafe { CStr::from_ptr(str_ptr).to_bytes().len() };
    unsafe { xmemdupz(str_ptr.cast::<c_void>(), len).cast::<c_char>() }
}

/// Reverse search for byte `c` in `src[..len]` (`xmemrchr`).
pub unsafe fn xmemrchr(src: *const c_void, c: u8, len: usize) -> *mut c_void {
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
pub unsafe fn xstrndup(str_ptr: *const c_char, len: usize) -> *mut c_char {
    let actual_len = unsafe { xstrnlen(str_ptr, len) };
    unsafe { xmemdupz(str_ptr.cast::<c_void>(), actual_len).cast::<c_char>() }
}

/// Return true if C string `a` equals Rust string `b` (`strequal_str`).
#[inline]
pub unsafe fn strequal_str(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == b.as_bytes() }
}

/// Return true if C string `a` equals Rust string `b` ignoring ASCII case (`striequal_str`).
#[inline]
pub unsafe fn striequal_str(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe {
        CStr::from_ptr(a)
            .to_bytes()
            .eq_ignore_ascii_case(b.as_bytes())
    }
}

/// Return true if C strings `a` and `b` are equal (both may be null) (`strequal`).
pub unsafe fn strequal(a: *const c_char, b: *const c_char) -> bool {
    if a.is_null() && b.is_null() {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == CStr::from_ptr(b).to_bytes() }
}

/// Return true if the first `n` characters of `a` and `b` are equal (`strnequal`).
pub unsafe fn strnequal(a: *const c_char, b: *const c_char, n: usize) -> bool {
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
pub unsafe fn time_to_bytes(time_val: i64, buf: *mut u8) {
    let bytes = (time_val as u64).to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, 8);
    }
}

/// Finish allocations in `arena` and return an `ArenaMem` handle (`arena_finish`).
pub unsafe fn arena_finish(arena: *mut Arena) -> ArenaMem {
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
pub unsafe fn alloc_block() -> *mut c_char {
    unsafe {
        if ARENA_REUSE_COUNT > 0 {
            let retval = ARENA_REUSE_HEAD.cast::<c_char>();
            ARENA_REUSE_HEAD = (*ARENA_REUSE_HEAD).prev;
            ARENA_REUSE_COUNT -= 1;
            retval
        } else {
            arena_alloc_count += 1;
            xmalloc(ARENA_BLOCK_SIZE).cast::<c_char>()
        }
    }
}

/// Allocate a fresh block on `arena` (`arena_alloc_block`).
pub unsafe fn arena_alloc_block(arena: *mut Arena) {
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
pub unsafe fn arena_alloc(arena: *mut Arena, size: usize, align: bool) -> *mut c_void {
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
pub unsafe fn free_block(block: *mut c_char) {
    if block.is_null() {
        return;
    }
    unsafe {
        if ARENA_REUSE_COUNT < REUSE_MAX {
            let reuse_blk = block.cast::<ConsumedBlk>();
            (*reuse_blk).prev = ARENA_REUSE_HEAD;
            ARENA_REUSE_HEAD = reuse_blk;
            ARENA_REUSE_COUNT += 1;
        } else {
            xfree(block.cast::<c_void>());
        }
    }
}

/// Free all blocks belonging to `mem` (`arena_mem_free`).
pub unsafe fn arena_mem_free(mem: ArenaMem) {
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
pub unsafe fn arena_allocz(arena: *mut Arena, size: usize) -> *mut c_char {
    let mem = unsafe { arena_alloc(arena, size + 1, false).cast::<u8>() };
    unsafe {
        *mem.add(size) = 0;
    }
    mem.cast::<c_char>()
}

/// Copy `buf[..size]` into a NUL-terminated buffer on `arena` (`arena_memdupz`).
pub unsafe fn arena_memdupz(arena: *mut Arena, buf: *const c_char, size: usize) -> *mut c_char {
    let mem = unsafe { arena_allocz(arena, size).cast::<u8>() };
    if size > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(buf.cast::<u8>(), mem, size);
        }
    }
    mem.cast::<c_char>()
}

/// Duplicate C string `str_ptr` onto `arena` (`arena_strdup`).
pub unsafe fn arena_strdup(arena: *mut Arena, str_ptr: *const c_char) -> *mut c_char {
    let len = unsafe { CStr::from_ptr(str_ptr).to_bytes().len() };
    unsafe { arena_memdupz(arena, str_ptr, len) }
}

/// Free everything that we allocated (`free_all_mem`).
pub unsafe fn free_all_mem() {
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
        set_expr_line(None);
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

        let bh_ptr = &raw mut buffer_handles;
        *bh_ptr = std::collections::BTreeMap::new();
        let wh_ptr = &raw mut window_handles;
        *wh_ptr = std::collections::BTreeMap::new();
        let th_ptr = &raw mut tabpage_handles;
        *th_ptr = std::collections::BTreeMap::new();

        grid_free_all_mem();
        let tp_defs = &raw mut tab_page_click_defs;
        stl_clear_click_defs(&mut *tp_defs);
        *tp_defs = Vec::new();

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
        unsafe {
            let check_cat = |dst_init: &str, src: &str, dsize: usize, expected: &str| {
                let mut buf = vec![0 as c_char; dsize];
                std::ptr::copy_nonoverlapping(
                    dst_init.as_ptr().cast::<c_char>(),
                    buf.as_mut_ptr(),
                    dst_init.len(),
                );
                let src_c = std::ffi::CString::new(src).unwrap();
                assert_eq!(
                    xstrlcat(buf.as_mut_ptr(), src_c.as_ptr(), dsize),
                    dst_init.len() + src.len()
                );
                assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), expected);
            };
            check_cat("a", "b", 3, "ab");
            check_cat("a", "b", 4096, "ab");
            check_cat("ABCיהZ", "defgiיהZ", 4096, "ABCיהZdefgiיהZ");
            check_cat("", "b", 4096, "b");
            check_cat("a", "", 4096, "a");
            check_cat("a", "b", 2, "a");
            check_cat("", "b", 1, "");
            check_cat("ABCיהZ", "defgiיהZ", 10, "ABCיהZd");

            let check_overlap = |dst_init: &str, src_idx: usize, dsize: usize, expected: &str| {
                let mut buf = vec![0 as c_char; dsize];
                std::ptr::copy_nonoverlapping(
                    dst_init.as_ptr().cast::<c_char>(),
                    buf.as_mut_ptr(),
                    dst_init.len(),
                );
                let src_ptr = buf.as_ptr().add(src_idx);
                assert_eq!(
                    xstrlcat(buf.as_mut_ptr(), src_ptr, dsize),
                    dst_init.len() * 2 - src_idx
                );
                assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), expected);
            };
            check_overlap("abc", 0, 7, "abcabc");
            check_overlap("abc", 0, 5, "abca");
            check_overlap("abc", 1, 5, "abcb");
            check_overlap("abc", 2, 10, "abcc");
            check_overlap("abc", 0, 2343, "abcabc");

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
