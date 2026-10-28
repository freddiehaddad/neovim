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

const ALLOC_ALIGN: usize = 16;
const HEADER_SIZE: usize = 16;

pub(crate) struct MemoryModuleState {
    pub(crate) entered_free_all_mem: bool,
    TRYING_TO_FREE: bool,
    ARENA_REUSE_HEAD: *mut ConsumedBlk,
    ARENA_REUSE_COUNT: usize,
}

impl MemoryModuleState {
    const fn new() -> Self {
        Self {
            entered_free_all_mem: false,
            TRYING_TO_FREE: false,
            ARENA_REUSE_HEAD: std::ptr::null_mut(),
            ARENA_REUSE_COUNT: 0,
        }
    }
}

static MEM_STATE: crate::globals::GlobalState<MemoryModuleState> = crate::globals::GlobalState::new(MemoryModuleState::new());

#[inline]
pub(crate) fn mem_st() -> &'static mut MemoryModuleState {
    unsafe { &mut *MEM_STATE.as_ptr() }
}

#[inline]
fn layout_with_header(size: usize) -> Option<std::alloc::Layout> {
    let total = HEADER_SIZE.checked_add(size.max(1))?;
    std::alloc::Layout::from_size_align(total, ALLOC_ALIGN).ok()
}

#[inline]
fn sys_malloc(size: usize) -> *mut u8 {
    let Some(layout) = layout_with_header(size) else {
        return std::ptr::null_mut();
    };
    let base = unsafe { std::alloc::alloc(layout) };
    if base.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        std::ptr::write(base.cast::<usize>(), layout.size());
        base.add(HEADER_SIZE)
    }
}

#[inline]
pub fn raw_dealloc(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let base = ptr.cast::<u8>().sub(HEADER_SIZE);
        let total_size = std::ptr::read(base.cast::<usize>());
        let layout = std::alloc::Layout::from_size_align_unchecked(total_size, ALLOC_ALIGN);
        std::alloc::dealloc(base, layout);
    }
}

#[inline]
fn sys_calloc(count: usize, size: usize) -> *mut u8 {
    let Some(byte_size) = count.checked_mul(size) else {
        return std::ptr::null_mut();
    };
    let Some(layout) = layout_with_header(byte_size) else {
        return std::ptr::null_mut();
    };
    let base = unsafe { std::alloc::alloc_zeroed(layout) };
    if base.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        std::ptr::write(base.cast::<usize>(), layout.size());
        base.add(HEADER_SIZE)
    }
}

#[inline]
fn sys_realloc(ptr: *mut c_void, size: usize) -> *mut u8 {
    if ptr.is_null() {
        return sys_malloc(size);
    }
    let Some(new_layout) = layout_with_header(size) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let old_base = ptr.cast::<u8>().sub(HEADER_SIZE);
        let old_total = std::ptr::read(old_base.cast::<usize>());
        let old_layout = std::alloc::Layout::from_size_align_unchecked(old_total, ALLOC_ALIGN);
        let new_base = std::alloc::realloc(old_base, old_layout, new_layout.size());
        if new_base.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::write(new_base.cast::<usize>(), new_layout.size());
        new_base.add(HEADER_SIZE)
    }
}

fn try_to_free_memory() {
    if mem_st().TRYING_TO_FREE {
            return;
        }
        mem_st().TRYING_TO_FREE = true;

        clear_sb_text(true);
        mf_release_all();
        arena_free_reuse_blks();

        mem_st().TRYING_TO_FREE = false;
}

fn out_of_memory_exit() -> ! {
    preserve_exit(e_outofmem)
}

/// Free all cached reusable arena blocks (`arena_free_reuse_blks`).
pub fn arena_free_reuse_blks() {
    unsafe {
        while mem_st().ARENA_REUSE_COUNT > 0 {
            let blk = mem_st().ARENA_REUSE_HEAD;
            mem_st().ARENA_REUSE_HEAD = (*blk).prev;
            raw_dealloc(blk.cast::<c_void>());
            mem_st().ARENA_REUSE_COUNT -= 1;
        }
    }
}

/// Allocate `size` bytes via `std::alloc`, retrying after freeing caches (`try_malloc`).
pub fn try_malloc(size: usize) -> *mut c_void {
    let allocated_size = size.max(1);
    let mut ret = sys_malloc(allocated_size);
    if ret.is_null() {
        try_to_free_memory();
            ret = sys_malloc(allocated_size);
    }
    ret.cast::<c_void>()
}

/// Allocate `size` bytes via `std::alloc` (never returns null).
pub fn raw_alloc(size: usize) -> *mut c_void {
    let ret = try_malloc(size);
    if ret.is_null() {
        out_of_memory_exit()
    }
    ret
}

/// Zero-initialized allocation via `std::alloc` (never returns null).
pub fn raw_calloc(count: usize, size: usize) -> *mut c_void {
    let (allocated_count, allocated_size) = if count != 0 && size != 0 {
        (count, size)
    } else {
        (1, 1)
    };
    let mut ret = sys_calloc(allocated_count, allocated_size);
    if ret.is_null() {
        try_to_free_memory();
            ret = sys_calloc(allocated_count, allocated_size);
            if ret.is_null() {
                out_of_memory_exit();
            }
    }
    ret.cast::<c_void>()
}

/// Resize an allocation created by `raw_alloc`/`raw_calloc` (never returns null).
pub fn raw_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    let allocated_size = size.max(1);
    let mut ret = sys_realloc(ptr, allocated_size);
    if ret.is_null() {
        try_to_free_memory();
            ret = sys_realloc(ptr, allocated_size);
            if ret.is_null() {
                out_of_memory_exit();
            }
    }
    ret.cast::<c_void>()
}

#[inline]
pub fn realloc_items<T>(items: *mut T, new_cap: usize) -> *mut T {
    raw_realloc(items.cast::<c_void>(), std::mem::size_of::<T>() * new_cap).cast::<T>()
}

#[inline]
pub fn free_items<T>(items: *mut T) {
    raw_dealloc(items.cast::<c_void>());
}

/// Allocate `len + 1` bytes, copy `data[..len]`, and NUL-terminate.
pub fn alloc_cstring_len(data: *const c_char, len: usize) -> *mut c_char {
    let Some(total_size) = len.checked_add(1) else {
        preserve_exit(c"Nvim: Data too large to fit into virtual memory space\n");
    };
    let ret = raw_alloc(total_size).cast::<u8>();
    if len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(data.cast::<u8>(), ret, len);
        }
    }
    unsafe {
        *ret.add(len) = 0;
    }
    ret.cast::<c_char>()
}

/// Duplicate a NUL-terminated C string.
#[inline]
pub fn alloc_cstring(str_ptr: &std::ffi::CStr) -> *mut c_char {
    let len =  { str_ptr.to_bytes().len() };
    alloc_cstring_len(str_ptr.as_ptr(), len)
}

/// Free a C string allocated by `alloc_cstring` / `alloc_cstring_len`.
#[inline]
pub fn free_cstring(p: *mut c_char) {
    raw_dealloc(p.cast::<c_void>());
}

/// Portable `strnlen` (`xstrnlen`).
pub fn xstrnlen(s: *const c_char, n: usize) -> usize {
    let mut len = 0;
    while len < n && unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

/// View up to `n` bytes of a NUL-terminated C string as a byte slice.
#[inline]
pub fn cstr_prefix_bytes<'a, S: crate::strings::IntoCCharPtr>(s: S, n: usize) -> &'a [u8] {
    let s = s.into_c_char_ptr();
    let len = xstrnlen(s, n);
    unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) }
}

/// Length in bytes of a NUL-terminated C string (0 if null).
#[inline]
pub fn cstr_len(s: *const c_char) -> usize {
    if s.is_null() {
        0
    } else {
        unsafe { CStr::from_ptr(s).to_bytes().len() }
    }
}

/// Copy NUL-terminated C string `src` (including trailing NUL) to `dst`.
#[inline]
pub fn cstr_cpy<S: crate::strings::IntoCCharPtr>(dst: *mut c_char, src: S) {
    let src = src.into_c_char_ptr();
    unsafe {
        let len = CStr::from_ptr(src).to_bytes_with_nul().len();
        std::ptr::copy_nonoverlapping(src, dst, len);
    }
}

/// Move potentially overlapping NUL-terminated C string `src` (including trailing NUL) to `dst`.
#[inline]
pub fn cstr_move<S: crate::strings::IntoCCharPtr>(dst: *mut c_char, src: S) {
    let src = src.into_c_char_ptr();
    unsafe {
        let len = CStr::from_ptr(src).to_bytes_with_nul().len();
        std::ptr::copy(src, dst, len);
    }
}

/// Convert a NUL-terminated C string (including trailing NUL) into a `Vec<c_char>`.
#[inline]
pub fn cstr_to_cchar_vec<S: crate::strings::IntoCCharPtr>(s: S) -> Vec<c_char> {
    let s = s.into_c_char_ptr();
    unsafe {
        let slice = CStr::from_ptr(s).to_bytes_with_nul();
        std::slice::from_raw_parts(slice.as_ptr().cast::<c_char>(), slice.len()).to_vec()
    }
}

/// View a NUL-terminated C string as a lossy UTF-8 string (`Cow<str>`).
#[inline]
pub fn cstr_lossy<'a, S: crate::strings::IntoCCharPtr>(s: S) -> std::borrow::Cow<'a, str> {
    let s = s.into_c_char_ptr();
    if s.is_null() {
        std::borrow::Cow::Borrowed("")
    } else {
        unsafe { CStr::from_ptr(s).to_string_lossy() }
    }
}

/// Copy `len` bytes from `src` to `dst` and append a trailing NUL byte at `dst[len]`.
#[inline]
pub fn cstr_ncpy<S: crate::strings::IntoCCharPtr>(dst: *mut c_char, src: S, len: usize) {
    let src = src.into_c_char_ptr();
    unsafe {
        if len > 0 {
            std::ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), len);
        }
        *dst.cast::<u8>().add(len) = 0;
    }
}

/// Append NUL-terminated C string `src` (including trailing NUL) to the end of `dst`.
#[inline]
pub fn cstr_cat<S: crate::strings::IntoCCharPtr>(dst: *mut c_char, src: S) {
    let src = src.into_c_char_ptr();
    unsafe {
        let dlen = cstr_len(dst);
        let slen = CStr::from_ptr(src).to_bytes_with_nul().len();
        std::ptr::copy_nonoverlapping(src, dst.add(dlen), slen);
    }
}

/// View a NUL-terminated C string as a byte slice (including trailing NUL), returning `b"\0"` if null.
#[inline]
pub fn cstr_as_bytes_with_nul<'a, S: crate::strings::IntoCCharPtr>(s: S) -> &'a [u8] {
    let s = s.into_c_char_ptr();
    if s.is_null() {
        b"\0"
    } else {
        unsafe { CStr::from_ptr(s).to_bytes_with_nul() }
    }
}

/// View a NUL-terminated C string as a byte slice, returning `b""` if null.
#[inline]
pub fn cstr_as_bytes<'a, S: crate::strings::IntoCCharPtr>(s: S) -> &'a [u8] {
    let s = s.into_c_char_ptr();
    if s.is_null() {
        b""
    } else {
        unsafe { CStr::from_ptr(s).to_bytes() }
    }
}

/// View a nullable NUL-terminated C string as `Option<&[u8]>`.
#[inline]
pub fn cstr_as_opt_bytes<'a, S: crate::strings::IntoCCharPtr>(s: S) -> Option<&'a [u8]> {
    let s = s.into_c_char_ptr();
    (!s.is_null()).then(|| unsafe { CStr::from_ptr(s).to_bytes() })
}

/// Copy `src` into sized buffer `dst[..dsize]` (`xstrlcpy`).
pub fn xstrlcpy<S: crate::strings::IntoCCharPtr>(dst: *mut c_char, src: S, dsize: usize) -> usize {
    let src = src.into_c_char_ptr();
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

/// Copy a NUL-terminated C string (or NULL) into an owned `CString`.
pub fn cstr_to_cstring(ptr: *const c_char) -> CString {
    if ptr.is_null() {
        CString::default()
    } else {
        unsafe { CStr::from_ptr(ptr).to_owned() }
    }
}

/// Copy a byte slice (truncating at any embedded NUL) into an owned `CString`.
pub fn cstr_from_bytes(bytes: &[u8]) -> CString {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut v = Vec::with_capacity(end + 1);
    v.extend_from_slice(&bytes[..end]);
    v.push(0);
    unsafe { CString::from_vec_with_nul_unchecked(v) }
}

/// Finish allocations in `arena` and return an `ArenaMem` handle (`arena_finish`).
pub fn arena_finish(arena: &mut Arena) -> ArenaMem {
    let res = arena.cur_blk.cast::<ConsumedBlk>();
    *arena = Arena {
        cur_blk: std::ptr::null_mut(),
        pos: 0,
        size: 0,
    };
    res
}

/// Allocate a `ARENA_BLOCK_SIZE` block (`alloc_block`).
pub fn alloc_block() -> *mut c_char {
    unsafe {
        if mem_st().ARENA_REUSE_COUNT > 0 {
            let retval = mem_st().ARENA_REUSE_HEAD.cast::<c_char>();
            mem_st().ARENA_REUSE_HEAD = (*mem_st().ARENA_REUSE_HEAD).prev;
            mem_st().ARENA_REUSE_COUNT -= 1;
            retval
        } else {
            g_st().arena_alloc_count += 1;
            raw_alloc(ARENA_BLOCK_SIZE).cast::<c_char>()
        }
    }
}

/// Allocate a fresh block on `arena` (`arena_alloc_block`).
pub fn arena_alloc_block(arena: &mut Arena) {
    let prev_blk = arena.cur_blk.cast::<ConsumedBlk>();
    arena.cur_blk = alloc_block();
    arena.pos = 0;
    arena.size = ARENA_BLOCK_SIZE;
    let blk = arena_alloc(arena, std::mem::size_of::<ConsumedBlk>(), true).cast::<ConsumedBlk>();
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

/// Allocate `size` bytes from `arena` (`arena_alloc`).
pub fn arena_alloc(arena: &mut Arena, size: usize, align: bool) -> *mut c_void {
    if arena.cur_blk.is_null() {
        arena_alloc_block(arena);
    }
    let mut alloc_pos = if align {
        arena_align_offset(arena.pos)
    } else {
        arena.pos
    };
    if alloc_pos + size > arena.size {
        if size > (ARENA_BLOCK_SIZE - std::mem::size_of::<ConsumedBlk>()) >> 1 {
            g_st().arena_alloc_count += 1;
            let hdr_size = std::mem::size_of::<ConsumedBlk>();
            let aligned_hdr_size = if align {
                arena_align_offset(hdr_size)
            } else {
                hdr_size
            };
            let alloc = raw_alloc(size + aligned_hdr_size).cast::<u8>();
            unsafe {
                let cur_blk = arena.cur_blk.cast::<ConsumedBlk>();
                let fix_blk = alloc.cast::<ConsumedBlk>();
                (*fix_blk).prev = (*cur_blk).prev;
                (*cur_blk).prev = fix_blk;
                return alloc.add(aligned_hdr_size).cast::<c_void>();
            }
        } else {
            arena_alloc_block(arena);
            alloc_pos = if align {
                arena_align_offset(arena.pos)
            } else {
                arena.pos
            };
        }
    }

    let mem = unsafe { arena.cur_blk.add(alloc_pos).cast::<c_void>() };
    arena.pos = alloc_pos + size;
    mem
}

/// Allocate `size` bytes from `arena` (or via `raw_alloc` if `arena` is `None`).
#[inline]
pub fn arena_alloc_opt(arena: Option<&mut Arena>, size: usize, align: bool) -> *mut c_void {
    match arena {
        Some(a) => arena_alloc(a, size, align),
        None => raw_alloc(size),
    }
}

/// Release a `ARENA_BLOCK_SIZE` block back to the reuse pool or free it (`free_block`).
pub fn free_block(block: *mut c_char) {
    if block.is_null() {
        return;
    }
    unsafe {
        if mem_st().ARENA_REUSE_COUNT < REUSE_MAX {
            let reuse_blk = block.cast::<ConsumedBlk>();
            (*reuse_blk).prev = mem_st().ARENA_REUSE_HEAD;
            mem_st().ARENA_REUSE_HEAD = reuse_blk;
            mem_st().ARENA_REUSE_COUNT += 1;
        } else {
            raw_dealloc(block.cast::<c_void>());
        }
    }
}

/// Free all blocks belonging to `mem` (`arena_mem_free`).
pub fn arena_mem_free(mem: ArenaMem) {
    let mut b = mem;
    if !b.is_null() {
        let reuse_blk = b;
        b = unsafe { (*b).prev };
        free_block(reuse_blk.cast::<c_char>());
    }
    while !b.is_null() {
        let prev = unsafe { (*b).prev };
        raw_dealloc(b.cast::<c_void>());
        b = prev;
    }
}

/// Allocate `size + 1` bytes on `arena` and NUL-terminate (`arena_allocz`).
pub fn arena_allocz(arena: &mut Arena, size: usize) -> *mut c_char {
    let mem = arena_alloc(arena, size + 1, false).cast::<u8>();
    unsafe {
        *mem.add(size) = 0;
    }
    mem.cast::<c_char>()
}

/// Allocate `size + 1` bytes on `arena` (or heap if `None`) and NUL-terminate.
#[inline]
pub fn arena_allocz_opt(arena: Option<&mut Arena>, size: usize) -> *mut c_char {
    let mem = arena_alloc_opt(arena, size + 1, false).cast::<u8>();
    unsafe {
        *mem.add(size) = 0;
    }
    mem.cast::<c_char>()
}

/// Copy `buf[..size]` into a NUL-terminated buffer on `arena` (`arena_memdupz`).
pub fn arena_memdupz(arena: &mut Arena, buf: *const c_char, size: usize) -> *mut c_char {
    let mem = arena_allocz(arena, size).cast::<u8>();
    if size > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(buf.cast::<u8>(), mem, size);
        }
    }
    mem.cast::<c_char>()
}

/// Copy `buf[..size]` into a NUL-terminated buffer on `arena` (or heap if `None`).
#[inline]
pub fn arena_memdupz_opt(arena: Option<&mut Arena>, buf: *const c_char, size: usize) -> *mut c_char {
    let mem = arena_allocz_opt(arena, size).cast::<u8>();
    if size > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(buf.cast::<u8>(), mem, size);
        }
    }
    mem.cast::<c_char>()
}

/// Duplicate C string `str_ptr` onto `arena` (`arena_strdup`).
pub fn arena_strdup(arena: &mut Arena, str_ptr: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let str_ptr = str_ptr.into_c_char_ptr();
    let len = unsafe { CStr::from_ptr(str_ptr).to_bytes().len() };
    arena_memdupz(arena, str_ptr, len)
}

/// Free everything that we allocated (`free_all_mem`).
pub fn free_all_mem() {
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
        if mem_st().entered_free_all_mem {
            nlua_free_all_mem();
            return;
        }
        mem_st().entered_free_all_mem = true;
        block_autocmds();

        opt_vars().p_ea = 0;
        if !g_st().first_tabpage.is_null() && !(*g_st().first_tabpage).tp_next.is_null() {
            do_cmdline_cmd(c"tabonly!");
        }

        spell_free_all();
        ex_comclear(&mut exarg_T::new());

        if !g_st().curbuf.is_null() {
            do_cmdline_cmd(c"aunmenu *");
            do_cmdline_cmd(c"tlunmenu *");
            do_cmdline_cmd(c"menutranslate clear");

            map_clear_mode(g_st().curbuf, MAP_ALL_MODES, false, false);
            map_clear_mode(g_st().curbuf, MAP_ALL_MODES, false, true);
            do_cmdline_cmd(c"breakdel *");
            do_cmdline_cmd(c"profdel *");
            do_cmdline_cmd(c"set keymap=");
        }

        free_titles();
        free_findfile();

        free_all_autocmds();
        free_all_marks();
        alist_clear(&raw mut g_st().global_alist);
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
        if !g_st().curtab.is_null() {
            diff_clear(&mut *g_st().curtab);
        }
        clear_sb_text(true);

        g_st().last_cmdline = None;
        g_st().new_last_cmdline = None;
        set_keep_msg(std::ptr::null(), 0);

        opt_vars().p_hi = 0;
        init_history();

        free_quickfix();
        close_all_scripts();

        if !(*curwin.as_ptr()).is_null() {
            win_free_all();
        }

        free_all_options();

        opt_vars().p_acd = 0;
        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            let mut bufref = bufref_T {
                br_buf: std::ptr::null_mut(),
                br_fnum: 0,
                br_buf_free_count: 0,
            };
            set_bufref(&mut bufref, buf);
            let nextbuf = (*buf).b_next;
            (*buf).b_nwindows = 0;

            buf_free_callbacks(&mut *buf);
            close_buffer(std::ptr::null_mut(), buf, DOBUF_WIPE, false, false, false);
            buf = if bufref_valid(Some(&bufref)) {
                nextbuf
            } else {
                g_st().firstbuf
            };
        }

        clear_registers();
        redo_free_all();
        free_highlight();
        reset_last_sourcing();

        if !g_st().first_tabpage.is_null() {
            free_tabpage(g_st().first_tabpage);
            g_st().first_tabpage = std::ptr::null_mut();
        }

        msg_hist_clear(0);

        channel_free_all_mem();
        eval_clear();
        api_extmark_free_all_mem();
        atom_free_all();
        mc_free_all();

        let bh_ptr = &raw mut g_st().buffer_handles;
        *bh_ptr = std::collections::BTreeMap::new();
        let wh_ptr = &raw mut g_st().window_handles;
        *wh_ptr = std::collections::BTreeMap::new();
        let th_ptr = &raw mut g_st().tabpage_handles;
        *th_ptr = std::collections::BTreeMap::new();

        grid_free_all_mem();
        let tp_defs = &raw mut g_st().tab_page_click_defs;
        stl_clear_click_defs(&mut *tp_defs);
        *tp_defs = Vec::new();

        clear_hl_tables(false);
        check_quickfix_busy();
        decor_free_all_mem();
        drawline_free_all_mem();

        if g_st().ui_client_channel_id != 0 {
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
    fn arena_lifecycle() {
        unsafe {
            let mut arena = Arena {
                cur_blk: std::ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let s1 = arena_strdup(&mut arena, c"hello arena");
            let big = arena_alloc(&mut arena, 8192, true);
            assert!(!big.is_null());
            assert_eq!(CStr::from_ptr(s1).to_str().unwrap(), "hello arena");
            let mem = arena_finish(&mut arena);
            arena_mem_free(mem);
            arena_free_reuse_blks();
        }
    }
}
