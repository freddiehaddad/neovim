//! Memory allocation, arena allocator, string utilities, and linked-list mergesort
//! (`src/nvim/memory.c`).
#![forbid(unsafe_code)]

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
use std::ffi::{CStr, CString};

pub const ARENA_BLOCK_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArenaMem;

impl ArenaMem {
    pub const NULL: Self = Self;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Arena {
    pub pos: usize,
    pub size: usize,
}

impl Arena {
    #[inline(always)]
    pub const fn new() -> Self {
        Self { pos: 0, size: 0 }
    }
}

pub(crate) struct MemoryModuleState {
    pub(crate) entered_free_all_mem: std::cell::Cell<bool>,
    trying_to_free: std::cell::Cell<bool>,
}

impl MemoryModuleState {
    const fn new() -> Self {
        Self {
            entered_free_all_mem: std::cell::Cell::new(false),
            trying_to_free: std::cell::Cell::new(false),
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

pub(crate) fn try_to_free_memory() {
    if MEM_STATE.with(|s| s.trying_to_free.get()) {
        return;
    }
    MEM_STATE.with(|s| s.trying_to_free.set(true));

    clear_sb_text(true);
    mf_release_all();
    arena_free_reuse_blks();

    MEM_STATE.with(|s| s.trying_to_free.set(false));
}

pub(crate) fn out_of_memory_exit() -> ! {
    preserve_exit(Some(E_OUTOFMEM))
}

/// Free all cached reusable arena blocks (`arena_free_reuse_blks`).
pub fn arena_free_reuse_blks() {}

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

impl<'a> CStrSubslice<'a> for &'a crate::option_vars::RcCStr {
    #[inline(always)]
    fn advance(self, n: usize) -> &'a CStr {
        self.as_cstr().advance(n)
    }
}

impl<'a> CStrSubslice<'a> for &&'a crate::option_vars::RcCStr {
    #[inline(always)]
    fn advance(self, n: usize) -> &'a CStr {
        self.as_cstr().advance(n)
    }
}

impl<'a> CStrSubslice<'a> for &mut &'a CStr {
    #[inline(always)]
    fn advance(self, n: usize) -> &'a CStr {
        (*self).advance(n)
    }
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

/// Convert an owned `Vec<u8>` (truncating at any embedded NUL) into a `CString` without copying.
#[inline]
pub fn vec_to_cstring(mut v: Vec<u8>) -> CString {
    if let Some(pos) = v.iter().position(|&b| b == 0) {
        v.truncate(pos + 1);
    } else {
        v.push(0);
    }
    CString::from_vec_with_nul(v).unwrap_or_default()
}

/// Finish allocations in `arena` and return an `ArenaMem` handle (`arena_finish`).
pub fn arena_finish(arena: &mut Arena) -> ArenaMem {
    *arena = Arena::new();
    ArenaMem
}

/// Free all blocks belonging to `mem` (`arena_mem_free`).
pub fn arena_mem_free(_mem: ArenaMem) {}

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

        opt_vars().p_ea.set(false);
        if !g_st().first_tabpage.get().is_null()
            && !crate::globals::first_tabpage_mut().tp_next.get().is_null()
        {
            do_cmdline_cmd(c"tabonly!");
        }

        spell_free_all();
        ex_comclear(&mut Exarg::new());

        if !g_st().curbuf.get().is_null() {
            do_cmdline_cmd(c"aunmenu *");
            do_cmdline_cmd(c"tlunmenu *");
            do_cmdline_cmd(c"menutranslate clear");

            map_clear_mode(&curbuf_mut(), MAP_ALL_MODES, false, false);
            map_clear_mode(&curbuf_mut(), MAP_ALL_MODES, false, true);
            do_cmdline_cmd(c"breakdel *");
            do_cmdline_cmd(c"profdel *");
            do_cmdline_cmd(c"set keymap=");
        }

        free_titles();
        free_findfile();

        free_all_autocmds();
        free_all_marks();
        alist_clear(&crate::arglist::global_alist_rc());
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
        if !g_st().curtab.get().is_null() {
            diff_clear(&curtab_mut());
        }
        clear_sb_text(true);

        g_st().last_cmdline.set(None);
        g_st().new_last_cmdline.set(None);
        set_keep_msg(None, 0);

        opt_vars().p_hi.set(0);
        init_history();

        free_quickfix();
        close_all_scripts();

        if !(curwin_ptr()).is_null() {
            win_free_all();
        }

        free_all_options();

        opt_vars().p_acd.set(false);
        let mut buf = g_st().firstbuf.get();
        while !buf.is_null() {
            let mut bufref = Bufref {
                br_buf: crate::globals::BufPtr::NULL,
                br_fnum: 0,
                br_buf_free_count: 0,
            };
            set_bufref(&mut bufref, buf.opt_mut());
            let nextbuf = buf.b_next.get();
            buf.b_nwindows.set(0);

            buf_free_callbacks(buf.borrow_mut());
            close_buffer(None, buf.opt_mut(), DOBUF_WIPE, false, false, false);
            buf = if bufref_valid(Some(&bufref)) {
                nextbuf
            } else {
                g_st().firstbuf.get()
            };
        }

        clear_registers();
        redo_free_all();
        free_highlight();
        reset_last_sourcing();

        if !g_st().first_tabpage.get().is_null() {
            let first_tp = g_st().first_tabpage.get().clone();
            free_tabpage(first_tp.opt());
            g_st().first_tabpage.set(crate::globals::TabpagePtr::NULL);
            g_st().curtab.set(crate::globals::TabpagePtr::NULL);
        }

        msg_hist_clear(0);

        channel_free_all_mem();
        eval_clear();
        api_extmark_free_all_mem();
        atom_free_all();
        mc_free_all();

        *g_st().buffer_handles.borrow_mut() = std::collections::BTreeMap::new();
        *g_st().window_handles.borrow_mut() = std::collections::BTreeMap::new();
        *g_st().tabpage_handles.borrow_mut() = std::collections::BTreeMap::new();

        grid_free_all_mem();
        stl_clear_click_defs(&mut g_st().tab_page_click_defs.borrow_mut());
        *g_st().tab_page_click_defs.borrow_mut() = Vec::new();

        clear_hl_tables(false);
        check_quickfix_busy();
        decor_free_all_mem();
        drawline_free_all_mem();

        if g_st().ui_client_channel_id.get() != 0 {
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
        let mut arena = Arena::new();
        let mem = arena_finish(&mut arena);
        arena_mem_free(mem);
        arena_free_reuse_blks();
    }
}
