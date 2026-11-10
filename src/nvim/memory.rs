//! Memory allocation, arena allocator, string utilities, and linked-list mergesort
//! (`src/nvim/memory.c`).
#![deny(unsafe_code)]

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
use std::ffi::{CStr, CString, c_void};

pub const ARENA_BLOCK_SIZE: usize = 4096;
const REUSE_MAX: usize = 4;

#[repr(C)]
pub struct ConsumedBlk {
    pub prev: crate::globals::GraphPtr<ConsumedBlk>,
}

pub type ArenaMem = crate::globals::GraphPtr<ConsumedBlk>;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Arena {
    pub cur_blk: crate::globals::GraphPtr<u8>,
    pub pos: usize,
    pub size: usize,
}

impl Arena {
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            cur_blk: crate::globals::GraphPtr::NULL,
            pos: 0,
            size: 0,
        }
    }
}

impl Default for Arena {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

const ALLOC_ALIGN: usize = 16;
const HEADER_SIZE: usize = 16;

pub(crate) struct MemoryModuleState {
    pub(crate) entered_free_all_mem: std::cell::Cell<bool>,
    trying_to_free: std::cell::Cell<bool>,
    arena_reuse_head: std::cell::Cell<crate::globals::GraphPtr<ConsumedBlk>>,
    arena_reuse_count: std::cell::Cell<usize>,
}

impl MemoryModuleState {
    const fn new() -> Self {
        Self {
            entered_free_all_mem: std::cell::Cell::new(false),
            trying_to_free: std::cell::Cell::new(false),
            arena_reuse_head: std::cell::Cell::new(crate::globals::GraphPtr::NULL),
            arena_reuse_count: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static MEM_STATE: MemoryModuleState = const { MemoryModuleState::new() };
}

#[inline(always)]
pub(crate) fn entered_free_all_mem() -> bool {
    MEM_STATE.with(|s| s.entered_free_all_mem.get())
}

#[allow(unsafe_code)]
mod alloc_ffi {
    use super::{ALLOC_ALIGN, HEADER_SIZE};
    use std::ffi::c_void;

    #[inline(always)]
    fn layout_with_header(size: usize) -> Option<std::alloc::Layout> {
        let total = HEADER_SIZE.checked_add(size.max(1))?;
        std::alloc::Layout::from_size_align(total, ALLOC_ALIGN).ok()
    }

    #[inline(always)]
    pub fn sys_malloc(size: usize) -> *mut u8 {
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

    #[inline(always)]
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

    #[inline(always)]
    pub fn sys_calloc(count: usize, size: usize) -> *mut u8 {
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

    #[inline(always)]
    pub fn sys_realloc(ptr: *mut c_void, size: usize) -> *mut u8 {
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
}

pub use self::alloc_ffi::raw_dealloc;
use self::alloc_ffi::{sys_calloc, sys_malloc, sys_realloc};

fn try_to_free_memory() {
    if MEM_STATE.with(|s| s.trying_to_free.get()) {
        return;
    }
    MEM_STATE.with(|s| s.trying_to_free.set(true));

    clear_sb_text(true);
    mf_release_all();
    arena_free_reuse_blks();

    MEM_STATE.with(|s| s.trying_to_free.set(false));
}

fn out_of_memory_exit() -> ! {
    preserve_exit(Some(E_OUTOFMEM))
}

/// Free all cached reusable arena blocks (`arena_free_reuse_blks`).
pub fn arena_free_reuse_blks() {
    while MEM_STATE.with(|s| s.arena_reuse_count.get()) > 0 {
        let blk = MEM_STATE.with(|s| s.arena_reuse_head.get());
        MEM_STATE.with(|s| s.arena_reuse_head.set(blk.prev));
        raw_dealloc(blk.as_ptr().cast::<c_void>());
        MEM_STATE.with(|s| s.arena_reuse_count.set(s.arena_reuse_count.get() - (1)));
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

#[inline(always)]
pub fn realloc_items<T>(
    items: crate::globals::GraphPtr<T>,
    new_cap: usize,
) -> crate::globals::GraphPtr<T> {
    crate::globals::GraphPtr::new(
        raw_realloc(
            items.as_ptr().cast::<c_void>(),
            std::mem::size_of::<T>() * new_cap,
        )
        .cast::<T>(),
    )
}

#[inline(always)]
pub fn free_items<T>(items: crate::globals::GraphPtr<T>) {
    raw_dealloc(items.as_ptr().cast::<c_void>());
}

/// Safe O(1) sub-slicing of a `&CStr`.
pub trait CStrSubslice<'a> {
    fn advance(self, n: usize) -> &'a CStr;
}

impl<'a> CStrSubslice<'a> for &'a CStr {
    #[inline(always)]
    fn advance(self, n: usize) -> &'a CStr {
        let bytes = self.to_bytes_with_nul();
        let off = n.min(bytes.len() - 1);
        CStr::from_bytes_with_nul(&bytes[off..]).unwrap_or(c"")
    }
}

impl<'a> CStrSubslice<'a> for &&'a CStr {
    #[inline(always)]
    fn advance(self, n: usize) -> &'a CStr {
        (*self).advance(n)
    }
}

impl<'a> CStrSubslice<'a> for &mut &'a CStr {
    #[inline(always)]
    fn advance(self, n: usize) -> &'a CStr {
        (*self).advance(n)
    }
}

/// Construct a `&CStr` from a single-line byte slice whose trailing byte is NUL.
#[inline(always)]
pub fn cstr_from_bytes_with_nul(bytes: &[u8]) -> &CStr {
    CStr::from_bytes_with_nul(bytes).unwrap_or_else(|_| bytes_as_cstr(bytes))
}

/// View a byte slice up to the first NUL byte (or the whole slice if no NUL is present).
#[inline(always)]
pub fn bytes_until_nul(bytes: &[u8]) -> &[u8] {
    CStr::from_bytes_until_nul(bytes).map_or(bytes, CStr::to_bytes)
}

/// View a byte slice up to the first NUL byte as a `&CStr` (or `c""` if no NUL is present).
#[inline(always)]
pub fn bytes_as_cstr(bytes: &[u8]) -> &CStr {
    CStr::from_bytes_until_nul(bytes).unwrap_or(c"")
}

/// Clone a byte slice up to the first NUL byte into an owned `CString`.
#[inline(always)]
pub fn cstring_until_nul(bytes: &[u8]) -> CString {
    bytes_as_cstr(bytes).to_owned()
}

/// Safely mutate the bytes of a `CString` in place via its owned `Vec<u8>`.
#[inline(always)]
pub fn cstring_modify<R>(s: &mut CString, f: impl FnOnce(&mut [u8]) -> R) -> R {
    let mut v = std::mem::take(s).into_bytes_with_nul();
    let r = f(&mut v);
    *s = cstring_until_nul(&v);
    r
}

/// Copy `src` (up to any embedded NUL) into sized C buffer `dst`, NUL-terminating `dst`.
/// Returns the byte length of `src` (up to any embedded NUL), matching `strlcpy` semantics.
#[inline(always)]
pub fn copy_bytes_to_c_buf(dst: &mut [u8], src: &[u8]) -> usize {
    let src = bytes_until_nul(src);
    let slen = src.len();
    if !dst.is_empty() {
        let len = slen.min(dst.len() - 1);
        dst[..len].copy_from_slice(&src[..len]);
        dst[len] = 0;
    }
    slen
}

/// Append `src` (up to any embedded NUL) onto NUL-terminated C buffer `dst`, NUL-terminating `dst`.
/// Returns the byte length of `src` (up to any embedded NUL).
#[inline(always)]
pub fn append_bytes_to_c_buf(dst: &mut [u8], src: &[u8]) -> usize {
    let dlen = bytes_until_nul(dst).len();
    if dlen < dst.len() {
        copy_bytes_to_c_buf(&mut dst[dlen..], src)
    } else {
        bytes_until_nul(src).len()
    }
}

/// Copy a byte slice (truncating at any embedded NUL) into an owned `CString`.
pub fn bytes_to_cstring(bytes: &[u8]) -> CString {
    match CStr::from_bytes_until_nul(bytes) {
        Ok(c) => c.to_owned(),
        Err(_) => CString::new(bytes).unwrap_or_default(),
    }
}

/// Finish allocations in `arena` and return an `ArenaMem` handle (`arena_finish`).
pub fn arena_finish(arena: &mut Arena) -> ArenaMem {
    let res = crate::globals::GraphPtr::new(arena.cur_blk.cast::<ConsumedBlk>());
    *arena = Arena::new();
    res
}

/// Allocate a `ARENA_BLOCK_SIZE` block (`alloc_block`).
fn alloc_block() -> crate::globals::GraphPtr<u8> {
    if MEM_STATE.with(|s| s.arena_reuse_count.get()) > 0 {
        let retval = crate::globals::GraphPtr::new(
            MEM_STATE.with(|s| s.arena_reuse_head.get()).as_ptr().cast(),
        );
        MEM_STATE.with(|s| {
            s.arena_reuse_head
                .set(MEM_STATE.with(|s| s.arena_reuse_head.get()).prev)
        });
        MEM_STATE.with(|s| s.arena_reuse_count.set(s.arena_reuse_count.get() - (1)));
        retval
    } else {
        g_st().arena_alloc_count += 1;
        crate::globals::GraphPtr::new(raw_alloc(ARENA_BLOCK_SIZE).cast())
    }
}

/// Allocate a fresh block on `arena` (`arena_alloc_block`).
pub fn arena_alloc_block(arena: &mut Arena) {
    let prev_blk = arena.cur_blk.cast::<ConsumedBlk>();
    arena.cur_blk = alloc_block();
    arena.pos = 0;
    arena.size = ARENA_BLOCK_SIZE;
    let blk = arena_alloc(arena, std::mem::size_of::<ConsumedBlk>(), true).cast::<ConsumedBlk>();
    crate::globals::GraphPtr::of(blk).borrow_mut().prev = crate::globals::GraphPtr::new(prev_blk);
}

#[inline(always)]
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
            let cur_blk =
                crate::globals::GraphPtr::of(arena.cur_blk.cast::<ConsumedBlk>()).borrow_mut();
            let fix_blk = alloc.cast::<ConsumedBlk>();
            crate::globals::GraphPtr::of(fix_blk).borrow_mut().prev = cur_blk.prev;
            cur_blk.prev = crate::globals::GraphPtr::new(fix_blk);
            return alloc.wrapping_add(aligned_hdr_size).cast::<c_void>();
        } else {
            arena_alloc_block(arena);
            alloc_pos = if align {
                arena_align_offset(arena.pos)
            } else {
                arena.pos
            };
        }
    }

    let mem = arena.cur_blk.wrapping_add(alloc_pos).cast::<c_void>();
    arena.pos = alloc_pos + size;
    mem
}

/// Allocate `size` bytes from `arena` (or via `raw_alloc` if `arena` is `None`).
#[inline(always)]
pub fn arena_alloc_opt(arena: Option<&mut Arena>, size: usize, align: bool) -> *mut c_void {
    match arena {
        Some(a) => arena_alloc(a, size, align),
        None => raw_alloc(size),
    }
}

/// Release a `ARENA_BLOCK_SIZE` block back to the reuse pool or free it (`free_block`).
fn free_block(mut block: ArenaMem) {
    if block.is_null() {
        return;
    }
    if MEM_STATE.with(|s| s.arena_reuse_count.get()) < REUSE_MAX {
        block.prev = MEM_STATE.with(|s| s.arena_reuse_head.get());
        MEM_STATE.with(|s| s.arena_reuse_head.set(block));
        MEM_STATE.with(|s| s.arena_reuse_count.set(s.arena_reuse_count.get() + (1)));
    } else {
        raw_dealloc(block.cast::<c_void>());
    }
}

/// Free all blocks belonging to `mem` (`arena_mem_free`).
pub fn arena_mem_free(mem: ArenaMem) {
    let mut b = mem;
    if !b.is_null() {
        let reuse_blk = b;
        b = b.prev;
        free_block(reuse_blk);
    }
    while !b.is_null() {
        let prev = b.prev;
        raw_dealloc(b.cast::<c_void>());
        b = prev;
    }
}

fn arena_allocz_mut(arena: &mut Arena, size: usize) -> &'static mut [u8] {
    let mem = crate::globals::GraphPtr::new(arena_alloc(arena, size + 1, false).cast::<u8>());
    let slice = mem.ptr_slice_mut(size + 1);
    slice[size] = 0;
    slice
}

/// Allocate `size + 1` bytes on `arena` and NUL-terminate (`arena_allocz`).
pub fn arena_allocz(arena: &mut Arena, size: usize) -> crate::globals::GraphPtr<u8> {
    crate::globals::GraphPtr::new(arena_allocz_mut(arena, size).as_mut_ptr())
}

/// Allocate `size + 1` mutable bytes on `arena` (or heap if `None`) and NUL-terminate.
#[inline(always)]
pub fn arena_allocz_opt_mut(arena: Option<&mut Arena>, size: usize) -> &'static mut [u8] {
    let mem = crate::globals::GraphPtr::new(arena_alloc_opt(arena, size + 1, false).cast::<u8>());
    let slice = mem.ptr_slice_mut(size + 1);
    slice[size] = 0;
    slice
}

/// Copy `buf` into a NUL-terminated buffer on `arena` (`arena_memdupz`).
pub fn arena_memdupz(arena: &mut Arena, buf: &[u8]) -> crate::globals::GraphPtr<u8> {
    let slice = arena_allocz_mut(arena, buf.len());
    slice[..buf.len()].copy_from_slice(buf);
    crate::globals::GraphPtr::new(slice.as_mut_ptr())
}

/// Copy `buf` into a NUL-terminated buffer on `arena` (or heap if `None`).
#[inline(always)]
pub fn arena_memdupz_opt(arena: Option<&mut Arena>, buf: &[u8]) -> crate::globals::GraphPtr<u8> {
    let slice = arena_allocz_opt_mut(arena, buf.len());
    slice[..buf.len()].copy_from_slice(buf);
    crate::globals::GraphPtr::new(slice.as_mut_ptr())
}

/// Free everything that we allocated (`free_all_mem`).
pub fn free_all_mem() {
    use crate::api::extmark::api_extmark_free_all_mem;
    use crate::arglist::alist_clear;
    use crate::autocmd::{autocmd_free_all_mem, block_autocmds, free_all_autocmds};
    use crate::buffer::{Bufref, DOBUF_WIPE, bufref_valid, close_buffer, free_titles, set_bufref};
    use crate::channel::channel_free_all_mem;
    use crate::cmdhist::init_history;
    use crate::decoration_provider::decor_free_all_mem;
    use crate::diff::diff_clear;
    use crate::drawline::drawline_free_all_mem;
    use crate::eval::eval_clear;
    use crate::ex_cmds::free_old_sub;
    use crate::ex_docmd::Exarg;
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

    {
        if MEM_STATE.with(|s| s.entered_free_all_mem.get()) {
            nlua_free_all_mem();
            return;
        }
        MEM_STATE.with(|s| s.entered_free_all_mem.set(true));
        block_autocmds();

        opt_vars().p_ea = false;
        if !g_st().first_tabpage.is_null() && !crate::globals::first_tabpage_mut().tp_next.is_null()
        {
            do_cmdline_cmd(c"tabonly!");
        }

        spell_free_all();
        ex_comclear(&mut Exarg::new());

        if !g_st().curbuf.is_null() {
            do_cmdline_cmd(c"aunmenu *");
            do_cmdline_cmd(c"tlunmenu *");
            do_cmdline_cmd(c"menutranslate clear");

            map_clear_mode(curbuf_mut(), MAP_ALL_MODES, false, false);
            map_clear_mode(curbuf_mut(), MAP_ALL_MODES, false, true);
            do_cmdline_cmd(c"breakdel *");
            do_cmdline_cmd(c"profdel *");
            do_cmdline_cmd(c"set keymap=");
        }

        free_titles();
        free_findfile();

        free_all_autocmds();
        free_all_marks();
        alist_clear(&mut g_st().global_alist);
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
            diff_clear(curtab_mut());
        }
        clear_sb_text(true);

        g_st().last_cmdline = None;
        g_st().new_last_cmdline = None;
        set_keep_msg(None, 0);

        opt_vars().p_hi = 0;
        init_history();

        free_quickfix();
        close_all_scripts();

        if !(curwin_ptr()).is_null() {
            win_free_all();
        }

        free_all_options();

        opt_vars().p_acd = false;
        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            let mut bufref = Bufref {
                br_buf: crate::globals::BufPtr::NULL,
                br_fnum: 0,
                br_buf_free_count: 0,
            };
            set_bufref(&mut bufref, buf.opt_mut());
            let nextbuf = buf.b_next;
            buf.b_nwindows = 0;

            buf_free_callbacks(buf.borrow_mut());
            close_buffer(None, buf.opt_mut(), DOBUF_WIPE, false, false, false);
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
            free_tabpage(g_st().first_tabpage.opt_mut());
            g_st().first_tabpage = crate::globals::TabpagePtr::NULL;
        }

        msg_hist_clear(0);

        channel_free_all_mem();
        eval_clear();
        api_extmark_free_all_mem();
        atom_free_all();
        mc_free_all();

        g_st().buffer_handles = std::collections::BTreeMap::new();
        g_st().window_handles = std::collections::BTreeMap::new();
        g_st().tabpage_handles = std::collections::BTreeMap::new();

        grid_free_all_mem();
        stl_clear_click_defs(&mut g_st().tab_page_click_defs);
        g_st().tab_page_click_defs = Vec::new();

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
        let mut arena = Arena {
            cur_blk: crate::globals::GraphPtr::NULL,
            pos: 0,
            size: 0,
        };
        let s1 = arena_memdupz(&mut arena, b"hello arena");
        let big = arena_alloc(&mut arena, 8192, true);
        assert!(!big.is_null());
        assert_eq!(
            bytes_as_cstr(s1.ptr_slice(12)).to_str().unwrap(),
            "hello arena"
        );
        let mem = arena_finish(&mut arena);
        arena_mem_free(mem);
        arena_free_reuse_blks();
    }
}
