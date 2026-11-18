#![forbid(unsafe_code)]

//! Buffer line storage and swapfile recovery (`src/nvim/memline.c`).

use crate::api::private::helpers::{HLF_E, cstr_as_string};
use crate::api::private::validate::{NvimString, Object};
use crate::autocmd::{
    EVENT_BUFREADPOST, EVENT_BUFWINENTER, EVENT_SWAPEXISTS, apply_autocmds, has_autocmd,
};
use crate::buffer::{
    FileBuf, K_UIMESSAGES, MAXLNUM, MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC, SEA_NONE, SEA_QUIT,
    SEA_RECOVER, buf_inc_changedtick, buf_spname, open_buffer, setfname,
};
use crate::change::{ML_DEL_MESSAGE, changed_internal, unchanged};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor, coladvance};
use crate::dialog::do_dialog;
use crate::drawscreen::{UPD_NOT_VALID, redraw_curbuf_later};
use crate::eval::typval::{
    TvDict, TvList, TypVal, tv_clear, tv_dict_add_nr, tv_dict_add_str_len, tv_list_alloc_ret,
    tv_list_first, tv_list_len,
};
use crate::eval::vars::{VV_SWAPCHOICE, VV_SWAPNAME, get_vim_var_str, set_vim_var_string};
use crate::fileio::{
    EOL_DOS, K_OPT_FILEENCODING, O_RDONLY, Off, READ_NEW, SEEK_END, buf_store_file_info, modname,
    read_eintr, readfile, vim_deltempdir, vim_rename, vim_tempname,
};
use crate::globals::*;
use crate::input::flush_buffers;
use crate::lua::executor::nlua_call_typval;
use crate::main_init::getout;
use crate::mark::setpcmark;
use crate::mbyte::{mb_adjust_cursor, mb_utflen};
use crate::memfile::{
    BH_DIRTY, BH_LOCKED, Bhdr, BhdrId, Blocknr, MFS_ALL, MFS_STOP, MFS_ZERO, Memfile, mf_close,
    mf_close_file, mf_free, mf_free_fnames, mf_get, mf_new, mf_new_page_size, mf_open,
    mf_open_file, mf_put, mf_set_dirty, mf_set_fnames, mf_sync,
};
use crate::message::{
    emsg, iemsg, msg, msg_end, msg_ext_set_kind, msg_multiline, msg_outnum, msg_outtrans,
    msg_putchar, msg_puts, msg_puts_hl, msg_reset_scroll, msg_start, semsg, set_keep_msg, siemsg,
    smsg, verb_msg,
};
use crate::option::{copy_option_part, get_fileformat, set_fileformat, set_option_value_give_err};
use crate::option_vars::*;
use crate::os::env::{expand_env, home_replace, home_replace_save, os_get_hostname, os_get_pid};
use crate::os::fs::{
    FileInfo, O_RDWR, os_close, os_fileinfo, os_fileinfo_inode, os_fileinfo_link, os_fileinfo_size,
    os_fileinfo2, os_isdir, os_mkdir_recurse, os_open, os_path_exists, os_remove, os_set_cloexec,
};
use crate::os::input::{line_breakcheck, os_char_avail};
use crate::os::proc::os_proc_running;
use crate::os::time::{Timestamp, os_ctime_r, os_time, time_t};
use crate::os::users::os_get_username;
#[cfg(unix)]
use crate::path::path_is_absolute;
use crate::path::{
    EW_FILE, EW_KEEPALL, EW_SILENT, K_PATH_CMP_FULL, K_PATH_CMP_LITERAL, after_pathsep,
    concat_fnames, expand_wildcards_vec, fix_fname, path_equal, path_tail, same_directory,
    shorten_dir, vim_full_name, vim_ispathsep,
};
use crate::spell::spell_delete_wordlist;
use crate::statusline::get_trans_bufname;
use crate::strings::{gettext, tr};
use crate::ui::{ui_flush, ui_has};
use crate::undo::buf_is_changed_fn;
use std::ffi::CStr;
use std::ptr;

#[derive(Clone)]
struct LineEntry {
    text: crate::option_vars::RcCStr,
    marked: bool,
}

impl LineEntry {
    const EMPTY: Self = Self {
        text: crate::option_vars::RcCStr::EMPTY,
        marked: false,
    };
}

#[derive(Clone)]
struct LineChunk {
    lines: Vec<LineEntry>,
    start_lnum: Linenr,
    totalsize: i32,
    swap_blocks: Vec<(PointerEntry, BhdrId)>,
}

pub struct Memline {
    pub ml_line_count: std::cell::Cell<i32>,
    pub ml_mfp: std::cell::RefCell<Option<Box<crate::memfile::Memfile>>>,
    ml_chunks: std::cell::RefCell<Vec<LineChunk>>,
    ml_cur_chunk: std::cell::Cell<usize>,
    ml_swap_dirty: std::cell::Cell<bool>,
    pub ml_flags: std::cell::Cell<i32>,
    ml_line_textlen: std::cell::Cell<i32>,
    pub ml_line_lnum: std::cell::Cell<i32>,
    ml_line_buf: std::cell::RefCell<Vec<u8>>,
    ml_line_cstr: crate::globals::CloneCell<Option<crate::option_vars::RcCStr>>,
    ml_line_offset: std::cell::Cell<usize>,
    ml_usedchunks: std::cell::Cell<i32>,
}

impl Memline {
    pub const fn new() -> Self {
        Self {
            ml_line_count: std::cell::Cell::new(0),
            ml_mfp: std::cell::RefCell::new(None),
            ml_chunks: std::cell::RefCell::new(Vec::new()),
            ml_cur_chunk: std::cell::Cell::new(0),
            ml_swap_dirty: std::cell::Cell::new(false),
            ml_flags: std::cell::Cell::new(0),
            ml_line_textlen: std::cell::Cell::new(0),
            ml_line_lnum: std::cell::Cell::new(0),
            ml_line_buf: std::cell::RefCell::new(Vec::new()),
            ml_line_cstr: crate::globals::CloneCell::new(None),
            ml_line_offset: std::cell::Cell::new(0),
            ml_usedchunks: std::cell::Cell::new(0),
        }
    }

    #[inline(always)]
    fn find_chunk_idx(&self, chunks: &[LineChunk], lnum: Linenr) -> Option<usize> {
        let cur = self.ml_cur_chunk.get();
        if let Some(ch) = chunks.get(cur) {
            if lnum >= ch.start_lnum && lnum < ch.start_lnum + (ch.lines.len() as Linenr) {
                return Some(cur);
            }
            if let Some(next_ch) = chunks.get(cur + 1)
                && lnum >= next_ch.start_lnum
                && lnum < next_ch.start_lnum + (next_ch.lines.len() as Linenr)
            {
                self.ml_cur_chunk.set(cur + 1);
                return Some(cur + 1);
            }
            if cur > 0
                && let Some(prev_ch) = chunks.get(cur - 1)
                && lnum >= prev_ch.start_lnum
                && lnum < prev_ch.start_lnum + (prev_ch.lines.len() as Linenr)
            {
                self.ml_cur_chunk.set(cur - 1);
                return Some(cur - 1);
            }
        }
        let idx = chunks
            .partition_point(|ch| ch.start_lnum <= lnum)
            .saturating_sub(1);
        let ch = chunks.get(idx)?;
        if lnum >= ch.start_lnum && lnum < ch.start_lnum + (ch.lines.len() as Linenr) {
            self.ml_cur_chunk.set(idx);
            Some(idx)
        } else {
            None
        }
    }

    #[inline]
    pub fn mark_swap_dirty(&self) {
        self.ml_swap_dirty.set(true);
        if let Some(mut mfp) = self.mfp_opt()
            && mfp.mf_dirty != MF_DIRTY_YES_NOSYNC
        {
            mfp.mf_dirty = MF_DIRTY_YES;
        }
    }

    #[inline(always)]
    pub fn with_current_line_slice<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let len = self.ml_line_textlen.get().max(0) as usize;
        if (self.ml_flags.get() & ML_LINE_DIRTY) != 0 {
            let b = self.ml_line_buf.borrow();
            let s = b.as_slice();
            f(&s[..len.min(s.len())])
        } else if let Some(cstr) = self.ml_line_cstr.get() {
            let s = cstr.to_bytes_with_nul();
            f(&s[..len.min(s.len())])
        } else {
            f(&b"???\0"[..len.min(4)])
        }
    }

    #[inline(always)]
    pub fn current_line_slice(&self) -> crate::option_vars::RcSlice {
        if (self.ml_flags.get() & ML_LINE_DIRTY) == 0
            && let Some(cstr) = self.ml_line_cstr.get()
        {
            return cstr.into_bytes_with_nul_slice();
        }
        let cstr = self.with_current_line_slice(|s| {
            crate::option_vars::RcCStr::from_cstr(crate::memory::bytes_as_cstr(s))
        });
        self.ml_line_cstr.set(Some(cstr.clone()));
        cstr.into_bytes_with_nul_slice()
    }

    #[inline(always)]
    pub fn has_chunks(&self) -> bool {
        self.ml_usedchunks.get() > 0
    }

    pub fn delete_dirty_line_bytes(
        &self,
        buf: &FileBuf,
        col: usize,
        count: usize,
        movelen: usize,
        oldlen: usize,
        newlen: Colnr,
    ) {
        {
            let lb = self.ml_line_buf.borrow();
            ml_add_deleted_len_buf(buf, &lb[..oldlen.min(lb.len())]);
        }
        let mut lb = self.ml_line_buf.borrow_mut();
        lb.copy_within((col + count)..(col + count + movelen), col);
        lb.truncate((newlen + 1) as usize);
        drop(lb);
        self.ml_line_textlen.set(newlen + 1);
        self.ml_line_cstr.set(None);
    }

    #[inline(always)]
    pub fn bhdr(&self, id: BhdrId) -> std::cell::Ref<'_, Bhdr> {
        std::cell::Ref::map(self.ml_mfp.borrow(), |opt| opt.as_ref().unwrap().bhdr(id))
    }

    #[inline(always)]
    pub fn bhdr_mut(&self, id: BhdrId) -> std::cell::RefMut<'_, Bhdr> {
        std::cell::RefMut::map(self.ml_mfp.borrow_mut(), |opt| {
            opt.as_mut().unwrap().bhdr_mut(id)
        })
    }

    #[inline(always)]
    pub fn mf_fname_cstr(&self) -> Option<std::ffi::CString> {
        self.ml_mfp
            .borrow()
            .as_ref()
            .and_then(|m| m.mf_fname.clone())
    }

    #[inline(always)]
    pub fn mfp_ref(&self) -> Option<std::cell::Ref<'_, crate::memfile::Memfile>> {
        std::cell::Ref::filter_map(self.ml_mfp.borrow(), Option::as_deref).ok()
    }

    #[inline(always)]
    pub fn mfp_opt(&self) -> Option<std::cell::RefMut<'_, crate::memfile::Memfile>> {
        std::cell::RefMut::filter_map(self.ml_mfp.borrow_mut(), Option::as_deref_mut).ok()
    }

    #[inline(always)]
    pub fn mfp_mut(&self) -> std::cell::RefMut<'_, crate::memfile::Memfile> {
        self.mfp_opt().expect("ml_mfp is None")
    }
}

pub(crate) const ML_LINE_DIRTY: i32 = 0x02;

#[cfg(unix)]
use crate::os::users::os_get_uname;
#[cfg(windows)]
use crate::path::path_to_slash;

const DATA_ID: u16 = ((b'd' as u16) << 8) + (b'a' as u16);
const PTR_ID: u16 = ((b'p' as u16) << 8) + (b't' as u16);
const BLOCK0_ID0: u8 = b'b';
const BLOCK0_ID1: u8 = b'0';

const DB_MARKED: u32 = 1u32 << (u32::BITS - 1);
const DB_INDEX_MASK: u32 = !DB_MARKED;

const INDEX_SIZE: usize = size_of::<u32>();
const HEADER_SIZE: usize = std::mem::offset_of!(DataBlock, db_index);

const B0_FNAME_SIZE_ORG: usize = 900;
const B0_FNAME_SIZE_NOCRYPT: usize = 898;
const B0_FNAME_SIZE_CRYPT: usize = 890;
const B0_UNAME_SIZE: usize = 40;
const B0_HNAME_SIZE: usize = 40;

const B0_MAGIC_LONG: i64 = 0x30313233;
const B0_MAGIC_INT: i32 = 0x20212223;
const B0_MAGIC_SHORT: i16 = 0x1213;
const B0_MAGIC_CHAR: u8 = 0x55;

const B0_DIRTY: u8 = 0x55;
const B0_FF_MASK: u8 = 3;
const B0_SAME_DIR: u8 = 4;
const B0_HAS_FENC: u8 = 8;

const ML_LINE_INPLACE: i32 = 0x10;

const ML_APPEND_NEW: i32 = 1;
const ML_APPEND_MARK: i32 = 2;

const MLCS_MAXL: i32 = 800;
const MLCS_MINL: i32 = 400;

use crate::memfile::MIN_SWAP_PAGE_SIZE;
pub(crate) const MFS_FLUSH: i32 = 4;

const BF_RECOVERED: i32 = 0x01;

const K_SHM_ATTENTION: i32 = b'A' as i32;

pub(crate) const K_PATH_DEVICE_UNC: i32 = 5;

#[cfg(unix)]
const ENOENT: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct PointerEntry {
    pe_bnum: Blocknr,
    pe_line_count: Linenr,
    pe_old_lnum: Linenr,
    pe_page_count: i32,
}
#[repr(C)]
struct PointerBlock {
    pb_id: u16,
    pb_count: u16,
    pb_count_max: u16,
    pb_pointer: [PointerEntry; 0],
}
#[repr(C)]
struct DataBlock {
    db_id: u16,
    db_free: u32,
    db_txt_start: u32,
    db_txt_end: u32,
    db_line_count: i64,
    db_index: [u32; 0],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct ZeroBlock {
    b0_id: [u8; 2],
    b0_version: [u8; 10],
    b0_page_size: [u8; 4],
    b0_mtime: [u8; 4],
    b0_ino: [u8; 4],
    b0_pid: [u8; 4],
    b0_uname: [u8; B0_UNAME_SIZE],
    b0_hname: [u8; B0_HNAME_SIZE],
    b0_fname: [u8; B0_FNAME_SIZE_ORG],
    b0_magic_long: i64,
    b0_magic_int: i32,
    b0_magic_short: i16,
    b0_magic_char: u8,
}

impl ZeroBlock {
    const fn new() -> Self {
        Self {
            b0_id: [0; 2],
            b0_version: [0; 10],
            b0_page_size: [0; 4],
            b0_mtime: [0; 4],
            b0_ino: [0; 4],
            b0_pid: [0; 4],
            b0_uname: [0; B0_UNAME_SIZE],
            b0_hname: [0; B0_HNAME_SIZE],
            b0_fname: [0; B0_FNAME_SIZE_ORG],
            b0_magic_long: 0,
            b0_magic_int: 0,
            b0_magic_short: 0,
            b0_magic_char: 0,
        }
    }
    #[inline(always)]
    fn dirty(&self) -> u8 {
        self.b0_fname[B0_FNAME_SIZE_ORG - 1]
    }

    #[inline(always)]
    fn set_dirty(&mut self, val: u8) {
        self.b0_fname[B0_FNAME_SIZE_ORG - 1] = val;
    }

    #[inline(always)]
    fn flags(&self) -> u8 {
        self.b0_fname[B0_FNAME_SIZE_ORG - 2]
    }

    #[inline(always)]
    fn set_flags(&mut self, val: u8) {
        self.b0_fname[B0_FNAME_SIZE_ORG - 2] = val;
    }

    fn from_bytes(raw: &[u8]) -> Self {
        let mut b0 = Self::new();
        b0.b0_id.copy_from_slice(&raw[0..2]);
        b0.b0_version.copy_from_slice(&raw[2..12]);
        b0.b0_page_size.copy_from_slice(&raw[12..16]);
        b0.b0_mtime.copy_from_slice(&raw[16..20]);
        b0.b0_ino.copy_from_slice(&raw[20..24]);
        b0.b0_pid.copy_from_slice(&raw[24..28]);
        b0.b0_uname.copy_from_slice(&raw[28..68]);
        b0.b0_hname.copy_from_slice(&raw[68..108]);
        b0.b0_fname.copy_from_slice(&raw[108..1008]);
        b0.b0_magic_long = i64::from_ne_bytes(raw[1008..1016].try_into().unwrap());
        b0.b0_magic_int = i32::from_ne_bytes(raw[1016..1020].try_into().unwrap());
        b0.b0_magic_short = i16::from_ne_bytes(raw[1020..1022].try_into().unwrap());
        b0.b0_magic_char = raw[1022];
        b0
    }

    fn write_to_bytes(&self, raw: &mut [u8]) {
        raw[0..2].copy_from_slice(&self.b0_id);
        raw[2..12].copy_from_slice(&self.b0_version);
        raw[12..16].copy_from_slice(&self.b0_page_size);
        raw[16..20].copy_from_slice(&self.b0_mtime);
        raw[20..24].copy_from_slice(&self.b0_ino);
        raw[24..28].copy_from_slice(&self.b0_pid);
        raw[28..68].copy_from_slice(&self.b0_uname);
        raw[68..108].copy_from_slice(&self.b0_hname);
        raw[108..1008].copy_from_slice(&self.b0_fname);
        raw[1008..1016].copy_from_slice(&self.b0_magic_long.to_ne_bytes());
        raw[1016..1020].copy_from_slice(&self.b0_magic_int.to_ne_bytes());
        raw[1020..1022].copy_from_slice(&self.b0_magic_short.to_ne_bytes());
        raw[1022] = self.b0_magic_char;
    }

    fn read_from_fd(fd: i32) -> Option<Self> {
        const RAW_SIZE: usize = size_of::<ZeroBlock>();
        let mut raw = [0u8; RAW_SIZE];
        if read_eintr(fd, &mut raw) != RAW_SIZE as isize {
            return None;
        }
        Some(Self::from_bytes(&raw))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdBlock0 {
    UbFname = 0,
    UbSameDir = 1,
}

pub type SeaChoice = i32;
const SEA_CHOICE_NONE: SeaChoice = 0;
const SEA_CHOICE_READONLY: SeaChoice = 1;
const SEA_CHOICE_EDIT: SeaChoice = 2;
const SEA_CHOICE_RECOVER: SeaChoice = 3;
const SEA_CHOICE_DELETE: SeaChoice = 4;
const SEA_CHOICE_QUIT: SeaChoice = 5;
const SEA_CHOICE_ABORT: SeaChoice = 6;

struct MemlineModuleState {
    lowest_marked: std::cell::Cell<Linenr>,
    proc_running: std::cell::Cell<i32>,
    ml_get_recursive: std::cell::Cell<i32>,
    ml_flush_entered: std::cell::Cell<bool>,
}

impl MemlineModuleState {
    const fn new() -> Self {
        Self {
            lowest_marked: std::cell::Cell::new(0),
            proc_running: std::cell::Cell::new(0),
            ml_get_recursive: std::cell::Cell::new(0),
            ml_flush_entered: std::cell::Cell::new(false),
        }
    }
}

std::thread_local! {
    static ML_STATE: MemlineModuleState = const { MemlineModuleState::new() };
}

#[inline(always)]
fn ml_st<R>(f: impl FnOnce(&MemlineModuleState) -> R) -> R {
    ML_STATE.with(f)
}

static E_ML_GET_INVALID_LNUM_NR: &CStr = c"E315: ml_get: Invalid lnum: %lld";
static E_ML_GET_CANNOT_FIND_LINE_NR_IN_BUFFER_NR_STR: &CStr =
    c"E316: ml_get: Cannot find line %lldin buffer %d %s";
static E_WARNING_POINTER_BLOCK_CORRUPTED: &CStr = c"E1364: Warning: Pointer block corrupted";

use crate::arglist::ML_EMPTY;
use crate::buffer::BF_DUMMY;
use crate::charset::{NL, NUL};
use crate::eval::buffer::SEA_READONLY;
use crate::eval::funcs::VIM_WARNING;
use crate::event::r#loop::uv_strerror;
use crate::ex_docmd::{CMOD_NOSWAPFILE, FLUSH_TYPEAHEAD};
#[cfg(unix)]
use crate::fileio::os_readlink;
use crate::fileio::vim_lseek;
use crate::globals::IOSIZE;
use crate::option::OPT_LOCAL;
use crate::os::fs::MAXPATHL;
#[cfg(unix)]
const EINVAL: i32 = 22;
use crate::os::time::os_uptime;
use crate::regexp::MAXCOL;
use crate::version::VERSIONS;

#[inline(always)]
fn pb_count_max(mfp: &Memfile) -> u16 {
    ((mfp.mf_page_size as usize - std::mem::offset_of!(PointerBlock, pb_pointer))
        / size_of::<PointerEntry>()) as u16
}

#[inline(always)]
fn bh_write_bytes(hp: &mut Bhdr, dst_off: usize, src: &[u8], total_len: usize) {
    if total_len == 0 {
        return;
    }
    let copy_len = src.len().min(total_len.saturating_sub(1));
    let dst = &mut hp.bh_data[dst_off..dst_off + total_len];
    dst[..copy_len].copy_from_slice(&src[..copy_len]);
    dst[copy_len..].fill(0);
}

const PB_HEADER_SIZE: usize = std::mem::offset_of!(PointerBlock, pb_pointer);
const PE_SIZE: usize = size_of::<PointerEntry>();

impl Bhdr {
    #[inline(always)]
    fn pb_id(&self) -> u16 {
        u16::from_ne_bytes(self.bh_data[0..2].try_into().unwrap())
    }

    #[inline(always)]
    fn set_pb_id(&mut self, v: u16) {
        self.bh_data[0..2].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn pb_count(&self) -> u16 {
        u16::from_ne_bytes(self.bh_data[2..4].try_into().unwrap())
    }

    #[inline(always)]
    fn set_pb_count(&mut self, v: u16) {
        self.bh_data[2..4].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn pb_count_max(&self) -> u16 {
        u16::from_ne_bytes(self.bh_data[4..6].try_into().unwrap())
    }

    #[inline(always)]
    fn set_pb_count_max(&mut self, v: u16) {
        self.bh_data[4..6].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn pb_entry(&self, idx: usize) -> PointerEntry {
        let off = PB_HEADER_SIZE + idx * PE_SIZE;
        PointerEntry {
            pe_bnum: i64::from_ne_bytes(self.bh_data[off..off + 8].try_into().unwrap()),
            pe_line_count: i32::from_ne_bytes(self.bh_data[off + 8..off + 12].try_into().unwrap()),
            pe_old_lnum: i32::from_ne_bytes(self.bh_data[off + 12..off + 16].try_into().unwrap()),
            pe_page_count: i32::from_ne_bytes(self.bh_data[off + 16..off + 20].try_into().unwrap()),
        }
    }

    #[inline(always)]
    fn set_pb_entry(&mut self, idx: usize, pe: PointerEntry) {
        let off = PB_HEADER_SIZE + idx * PE_SIZE;
        self.bh_data[off..off + 8].copy_from_slice(&pe.pe_bnum.to_ne_bytes());
        self.bh_data[off + 8..off + 12].copy_from_slice(&pe.pe_line_count.to_ne_bytes());
        self.bh_data[off + 12..off + 16].copy_from_slice(&pe.pe_old_lnum.to_ne_bytes());
        self.bh_data[off + 16..off + 20].copy_from_slice(&pe.pe_page_count.to_ne_bytes());
    }

    #[inline(always)]
    fn db_id(&self) -> u16 {
        u16::from_ne_bytes(self.bh_data[0..2].try_into().unwrap())
    }

    #[inline(always)]
    fn set_db_id(&mut self, v: u16) {
        self.bh_data[0..2].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn db_free(&self) -> u32 {
        u32::from_ne_bytes(self.bh_data[4..8].try_into().unwrap())
    }

    #[inline(always)]
    fn set_db_free(&mut self, v: u32) {
        self.bh_data[4..8].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn db_txt_start(&self) -> u32 {
        u32::from_ne_bytes(self.bh_data[8..12].try_into().unwrap())
    }

    #[inline(always)]
    fn set_db_txt_start(&mut self, v: u32) {
        self.bh_data[8..12].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn db_txt_end(&self) -> u32 {
        u32::from_ne_bytes(self.bh_data[12..16].try_into().unwrap())
    }

    #[inline(always)]
    fn set_db_txt_end(&mut self, v: u32) {
        self.bh_data[12..16].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn db_line_count(&self) -> i64 {
        i64::from_ne_bytes(self.bh_data[16..24].try_into().unwrap())
    }

    #[inline(always)]
    fn set_db_line_count(&mut self, v: i64) {
        self.bh_data[16..24].copy_from_slice(&v.to_ne_bytes());
    }

    #[inline(always)]
    fn db_idx(&self, idx: usize) -> u32 {
        let off = HEADER_SIZE + idx * INDEX_SIZE;
        u32::from_ne_bytes(self.bh_data[off..off + 4].try_into().unwrap())
    }

    #[inline(always)]
    fn set_db_idx(&mut self, idx: usize, v: u32) {
        let off = HEADER_SIZE + idx * INDEX_SIZE;
        self.bh_data[off..off + 4].copy_from_slice(&v.to_ne_bytes());
    }
}

#[inline(always)]
fn sb_push_cstr(sb: &mut Vec<u8>, s: &CStr) {
    sb.extend_from_slice(s.to_bytes());
}

/// Open a new memline for `buf`.
pub fn ml_open(buf: &FileBuf) -> bool {
    let mut first_lines = Vec::with_capacity(MLCS_MINL as usize);
    first_lines.push(LineEntry::EMPTY);
    *buf.b_ml.ml_chunks.borrow_mut() = vec![LineChunk {
        lines: first_lines,
        start_lnum: 1,
        totalsize: 1,
        swap_blocks: Vec::new(),
    }];
    buf.b_ml.ml_cur_chunk.set(0);
    buf.b_ml.ml_swap_dirty.set(true);
    buf.b_ml.ml_line_lnum.set(0);
    buf.b_ml.ml_line_cstr.set(None);
    buf.b_ml.ml_line_offset.set(0);
    buf.b_ml.ml_usedchunks.set(0);

    if (cmdline_state().cmod_flags() & CMOD_NOSWAPFILE) != 0 {
        buf.b_p_swf.set(false);
    }

    buf.b_may_swap
        .set(buf.terminal.get().is_null() && opt_vars().p_uc.get() != 0 && buf.b_p_swf.get());

    *buf.b_ml.ml_mfp.borrow_mut() = mf_open(None, 0);
    if buf.b_ml.mfp_ref().is_none() {
        return false;
    }
    buf.b_ml.ml_flags.set(ML_EMPTY);
    buf.b_ml.ml_line_count.set(1);

    let hp = mf_new(&mut buf.b_ml.mfp_mut(), false, 1);
    if buf.b_ml.bhdr(hp).bh_bnum != 0 {
        iemsg(tr(c"E298: Didn't get block nr 0?"));
        return ml_open_error(buf, Some(hp));
    }
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.bhdr(hp).bh_data);

    b0p.b0_id[0] = BLOCK0_ID0;
    b0p.b0_id[1] = BLOCK0_ID1;
    b0p.b0_magic_long = B0_MAGIC_LONG;
    b0p.b0_magic_int = B0_MAGIC_INT;
    b0p.b0_magic_short = B0_MAGIC_SHORT;
    b0p.b0_magic_char = B0_MAGIC_CHAR;
    b0p.b0_version[..4].copy_from_slice(b"VIM ");
    crate::memory::copy_bytes_to_c_buf(&mut b0p.b0_version[4..], VERSIONS[0].to_bytes());
    let page_size = buf.b_ml.mfp_ref().unwrap().mf_page_size;
    long_to_char(page_size as i64, &mut b0p.b0_page_size);

    if !buf.b_spell.get() {
        b0p.set_dirty(if buf.b_changed.get() != 0 {
            B0_DIRTY
        } else {
            0
        });
        b0p.set_flags((get_fileformat(buf) + 1) as u8);
        set_b0_fname(&mut b0p, buf);
        os_get_username(&mut b0p.b0_uname);
        b0p.b0_uname[B0_UNAME_SIZE - 1] = 0;
        os_get_hostname(&mut b0p.b0_hname);
        b0p.b0_hname[B0_HNAME_SIZE - 1] = 0;
        long_to_char(os_get_pid(), &mut b0p.b0_pid);
    }

    b0p.write_to_bytes(&mut buf.b_ml.bhdr_mut(hp).bh_data);
    mf_put(&mut buf.b_ml.mfp_mut(), hp, true, false);
    if !buf.b_help.get() && !buf.b_spell.get() {
        mf_sync(&mut buf.b_ml.mfp_mut(), 0);
    }

    true
}

fn ml_open_error(buf: &FileBuf, hp: Option<BhdrId>) -> bool {
    if let Some(mut mfp) = buf.b_ml.mfp_opt() {
        if let Some(hp) = hp {
            mf_put(&mut mfp, hp, false, false);
        }
        drop(mfp);
        mf_close(buf.b_ml.ml_mfp.borrow_mut().take(), true);
    }
    false
}

/// Called when the file name of `buf` has been changed.
pub fn ml_setname(buf: &FileBuf) {
    let mut success = false;
    let mfp_addr = buf
        .b_ml
        .mfp_ref()
        .map_or(0, |m| ptr::from_ref(&*m) as usize);
    if buf.b_ml.mfp_mut().mf_fd < 0 {
        if opt_vars().p_uc.get() != 0 && (cmdline_state().cmod_flags() & CMOD_NOSWAPFILE) == 0 {
            ml_open_file(buf);
        }
        return;
    }

    let p_dir = opt_vars().p_dir.to_owned();
    let mut dirp = p_dir.as_bytes();
    let mut found_existing_dir = false;
    while !dirp.is_empty() {
        let mf_fname = buf.b_ml.mfp_ref().and_then(|m| m.mf_fname.clone());
        let fname = findswapname(buf, &mut dirp, mf_fname.as_deref(), &mut found_existing_dir);
        if buf
            .b_ml
            .mfp_ref()
            .map_or(0, |m| ptr::from_ref(&*m) as usize)
            != mfp_addr
        {
            return;
        }
        let Some(fname) = fname else {
            continue;
        };

        let renamed = {
            let mfp = &mut *buf.b_ml.mfp_mut();
            let mf_fname_cstr = mfp.mf_fname.as_deref().unwrap_or(c"");
            if path_equal(
                fname.to_bytes(),
                mf_fname_cstr.to_bytes(),
                K_PATH_CMP_LITERAL,
            ) {
                success = true;
                break;
            }
            if mfp.mf_fd >= 0 {
                os_close(mfp.mf_fd);
                mfp.mf_fd = -1;
            }

            if vim_rename(mf_fname_cstr, &fname) {
                success = true;
                mf_free_fnames(mfp);
                mf_set_fnames(mfp, fname);
                true
            } else {
                false
            }
        };
        if renamed {
            ml_upd_block0(buf, UpdBlock0::UbSameDir);
            break;
        }
    }

    let mfp = &mut *buf.b_ml.mfp_mut();
    if mfp.mf_fd == -1 {
        mfp.mf_fd = os_open(mfp.mf_fname.as_ref().unwrap(), O_RDWR, 0);
        if mfp.mf_fd < 0 {
            emsg(tr(c"E301: Oops, lost the swap file!!!"));
            return;
        }
        os_set_cloexec(mfp.mf_fd);
    }
    if !success {
        emsg(tr(c"E302: Could not rename swap file"));
    }
}

/// Open a file for the memfile for all buffers that are not readonly or have been modified.
pub fn ml_open_files() {
    for buf in for_all_buffers_ptr() {
        if !buf.b_p_ro.get() || buf.b_changed.get() != 0 {
            ml_open_file(&buf);
        }
    }
}

/// Open a swapfile for an existing memfile, if there is no swapfile yet.
pub fn ml_open_file(buf: &FileBuf) {
    let mfp_addr = buf
        .b_ml
        .mfp_ref()
        .map_or(0, |m| ptr::from_ref(&*m) as usize);
    let Some(mfp_fd) = buf.b_ml.mfp_ref().map(|m| m.mf_fd) else {
        return;
    };
    if mfp_fd >= 0
        || !buf.b_p_swf.get()
        || (cmdline_state().cmod_flags() & CMOD_NOSWAPFILE) != 0
        || !buf.terminal.get().is_null()
    {
        return;
    }

    if buf.b_spell.get() {
        if let Some(fname) = vim_tempname() {
            mf_open_file(&mut buf.b_ml.mfp_mut(), fname);
        }
        buf.b_may_swap.set(false);
        return;
    }

    let p_dir = opt_vars().p_dir.to_owned();
    let mut dirp = p_dir.as_bytes();
    let mut found_existing_dir = false;
    while !dirp.is_empty() {
        let fname = findswapname(buf, &mut dirp, None, &mut found_existing_dir);
        if buf
            .b_ml
            .mfp_ref()
            .map_or(0, |m| ptr::from_ref(&*m) as usize)
            != mfp_addr
        {
            return;
        }
        let Some(fname) = fname else {
            continue;
        };
        let opened = {
            let mfp = &mut *buf.b_ml.mfp_mut();
            if mf_open_file(mfp, fname) {
                mfp.mf_dirty = MF_DIRTY_YES_NOSYNC;
                true
            } else {
                false
            }
        };
        if opened {
            ml_upd_block0(buf, UpdBlock0::UbSameDir);

            let synced = mf_sync(&mut buf.b_ml.mfp_mut(), MFS_ZERO);
            if synced {
                buf.b_ml.ml_swap_dirty.set(true);
                mf_set_dirty(&mut buf.b_ml.mfp_mut());
                break;
            }
            mf_close_file(buf, false);
        }
    }

    if !opt_vars().p_dir.is_empty() && buf.b_ml.mfp_mut().mf_fname.is_none() {
        msg_state().need_wait_return.set(true);
        let _no_wait = msg_state().suppress_wait_return();
        let sp = buf_spname(buf);
        semsg!(
            tr(c"E303: Unable to open swap file for \"%s\", recovery impossible"),
            sp.unwrap_or_else(|| buf.fname_cstr()),
        );
    }

    buf.b_may_swap.set(false);
}

/// If still need to create a swapfile, create a swapfile now.
pub fn check_need_swap(newfile: bool) {
    let old_msg_silent = msg_state().msg_silent.get();
    msg_state().msg_silent.set(0);

    if curbuf().b_may_swap.get() && (!curbuf().b_p_ro.get() || !newfile) {
        ml_open_file(&curbuf());
    }

    msg_state().msg_silent.set(old_msg_silent);
}

/// Close memline for buffer `buf`.
pub fn ml_close(buf: &FileBuf, del_file: i32) {
    if buf.b_ml.ml_mfp.borrow().is_none() {
        return;
    }
    mf_close(buf.b_ml.ml_mfp.borrow_mut().take(), del_file != 0);
    buf.b_ml.ml_line_cstr.set(None);
    buf.b_ml.ml_swap_dirty.set(false);
    buf.b_ml.ml_cur_chunk.set(0);
    *buf.b_ml.ml_chunks.borrow_mut() = Vec::new();
    *buf.b_ml.ml_line_buf.borrow_mut() = Vec::new();

    buf.b_flags.set(buf.b_flags.get() & (!BF_RECOVERED));
}

/// Close all existing memlines and memfiles.
pub fn ml_close_all(del_file: bool) {
    for buf in for_all_buffers_ptr() {
        ml_close(&buf, i32::from(del_file));
    }
    spell_delete_wordlist();
    vim_deltempdir();
}

/// Close all memfiles for not modified buffers.
pub fn ml_close_notmod() {
    for buf in for_all_buffers_ptr() {
        if !buf_is_changed_fn(&buf) {
            ml_close(&buf, 1);
        }
    }
}

/// Update the timestamp in the `.swp` file.
pub fn ml_timestamp(buf: &FileBuf) {
    ml_upd_block0(buf, UpdBlock0::UbFname);
}

#[inline(always)]
fn ml_check_b0_id(b0p: &ZeroBlock) -> bool {
    b0p.b0_id[0] == BLOCK0_ID0 && b0p.b0_id[1] == BLOCK0_ID1
}

#[inline(always)]
fn ml_check_b0_strings(b0p: &ZeroBlock) -> bool {
    let b0 = b0p;
    b0.b0_version.contains(&0)
        && b0.b0_uname.contains(&0)
        && b0.b0_hname.contains(&0)
        && b0.b0_fname[..B0_FNAME_SIZE_CRYPT].contains(&0)
}

fn ml_upd_block0(buf: &FileBuf, what: UpdBlock0) {
    let Some(hp) = buf
        .b_ml
        .mfp_opt()
        .and_then(|mut mfp| mf_get(&mut mfp, 0, 1))
    else {
        return;
    };
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.bhdr(hp).bh_data);
    if !ml_check_b0_id(&b0p) {
        iemsg(tr(c"E304: ml_upd_block0(): Didn't get block 0??"));
    } else {
        if what == UpdBlock0::UbFname {
            set_b0_fname(&mut b0p, buf);
        } else {
            set_b0_dir_flag(&mut b0p, buf);
        }
        b0p.write_to_bytes(&mut buf.b_ml.bhdr_mut(hp).bh_data);
    }
    mf_put(&mut buf.b_ml.mfp_mut(), hp, true, false);
}

fn set_b0_fname(b0p: &mut ZeroBlock, buf: &FileBuf) {
    if buf.b_ffname.get().is_none() {
        b0p.b0_fname[0] = 0;
    } else {
        let mut uname = [0u8; B0_UNAME_SIZE];
        let flen = home_replace(
            None,
            buf.ffname_opt().as_deref(),
            &mut b0p.b0_fname[..B0_FNAME_SIZE_CRYPT],
            true,
        );
        if b0p.b0_fname[0] == b'~' {
            let retval = os_get_username(&mut uname);
            let ulen = crate::memory::bytes_until_nul(&uname).len();
            if !retval || ulen + flen > B0_FNAME_SIZE_CRYPT - 1 {
                crate::memory::copy_bytes_to_c_buf(
                    &mut b0p.b0_fname[..B0_FNAME_SIZE_CRYPT],
                    &buf.ffname_bytes(),
                );
            } else {
                b0p.b0_fname.copy_within(1..(1 + flen), ulen + 1);
                b0p.b0_fname[1..(1 + ulen)].copy_from_slice(&uname[..ulen]);
            }
        }
        let mut file_info = FileInfo::new();
        if os_fileinfo(&buf.ffname_cstr(), &mut file_info) {
            buf_store_file_info(buf, &mut file_info);
            long_to_char(buf.b_mtime.get(), &mut b0p.b0_mtime);
            long_to_char(os_fileinfo_inode(&file_info) as i64, &mut b0p.b0_ino);
            buf.b_mtime_read.set(buf.b_mtime.get());
            buf.b_mtime_read_ns.set(buf.b_mtime_ns.get());
        } else {
            long_to_char(0, &mut b0p.b0_mtime);
            long_to_char(0, &mut b0p.b0_ino);
            buf.b_mtime.set(0);
            buf.b_mtime_ns.set(0);
            buf.b_mtime_read.set(0);
            buf.b_mtime_read_ns.set(0);
            buf.b_orig_size.set(0);
            buf.b_orig_mode.set(0);
        }
    }

    add_b0_fenc(b0p, &curbuf());
}

fn set_b0_dir_flag(b0p: &mut ZeroBlock, buf: &FileBuf) {
    if same_directory(
        buf.b_ml.mf_fname_cstr().as_deref().map(CStr::to_bytes),
        buf.ffname_opt().as_deref().map(CStr::to_bytes),
    ) {
        b0p.set_flags(b0p.flags() | B0_SAME_DIR);
    } else {
        b0p.set_flags(b0p.flags() & !B0_SAME_DIR);
    }
}

fn add_b0_fenc(b0p: &mut ZeroBlock, buf: &FileBuf) {
    let size = B0_FNAME_SIZE_NOCRYPT;
    let fenc_bytes = buf.b_p_fenc.as_bytes();
    let n = fenc_bytes.len();
    if crate::memory::bytes_as_cstr(&b0p.b0_fname).to_bytes().len() + n + 1 > size {
        b0p.set_flags(b0p.flags() & !B0_HAS_FENC);
    } else {
        b0p.b0_fname[(size - n)..size].copy_from_slice(&fenc_bytes);
        b0p.b0_fname[size - n - 1] = 0;
        b0p.set_flags(b0p.flags() | B0_HAS_FENC);
    }
}

fn swapfile_proc_running(b0p: &ZeroBlock, swap_fname: &CStr) -> i32 {
    let mut st = FileInfo::new();
    if os_fileinfo(swap_fname, &mut st)
        && let Some(uptime) = os_uptime()
        && (st.stat.st_mtim.tv_sec as Timestamp) < os_time().wrapping_sub(uptime as Timestamp)
    {
        return 0;
    }
    let pid = char_to_long(&b0p.b0_pid) as i32;
    if os_proc_running(pid) { pid } else { 0 }
}

/// Try to recover `curbuf` from the `.swp` file.
pub fn ml_recover(checkext: bool) {
    let fname_used: Option<std::ffi::CString>;
    let mut hp: Option<BhdrId>;
    let mut b0_fenc: Option<std::ffi::CString> = None;
    let directly: bool;
    let mut serious_error = true;
    let mut orig_file_status = None;

    exc_state().recoverymode.set(true);
    let called_from_main = curbuf().b_ml.ml_mfp.borrow().is_none();

    let fname_opt = curbuf().fname_opt();
    let fname_cstr = fname_opt.as_deref().unwrap_or(c"");
    let fname_slice = fname_cstr.to_bytes();
    let len = fname_slice.len();
    if checkext
        && len >= 4
        && fname_slice[len - 4..len - 2].eq_ignore_ascii_case(b".s")
        && crate::strings::vim_haschr(
            c"abcdefghijklmnopqrstuvw",
            fname_slice[len - 2].to_ascii_lowercase() as i32,
        )
        && fname_slice[len - 1].is_ascii_alphabetic()
    {
        directly = true;
        fname_used = Some(fname_cstr.to_owned());
    } else {
        directly = false;
        let mut items_tv = TypVal::new();
        tv_list_alloc_ret(&mut items_tv, 0);

        recover_names(Some(fname_cstr), true, &items_tv.vval.v_list());

        let n_swaps = tv_list_len(items_tv.vval.v_list().opt());

        if n_swaps == 0 {
            tv_clear(&mut items_tv);
            semsg!(tr(c"E305: No swap file found for %s"), fname_cstr);

            ml_recover_theend(None, None, serious_error, called_from_main);

            return;
        }
        if n_swaps > 1 {
            let mut lua_args = [items_tv, TypVal::new()];
            nlua_call_typval(c"vim._core.swapfile", c"select_swap", &mut lua_args, None);
            tv_clear(&mut lua_args[0]);

            ml_recover_theend(None, None, serious_error, called_from_main);

            return;
        }
        fname_used = tv_list_first(items_tv.vval.v_list().opt())
            .unwrap()
            .tv()
            .vval
            .v_string_cstr()
            .map(CStr::to_owned);
        tv_clear(&mut items_tv);
    }
    let Some(fname_used) = fname_used else {
        ml_recover_theend(None, None, serious_error, called_from_main);

        return;
    };
    if called_from_main && !ml_open(&curbuf()) {
        getout(1);
    }

    let buf = FileBuf::new_rc();

    *buf.b_ml.ml_mfp.borrow_mut() = mf_open(Some(fname_used.clone()), O_RDONLY);
    if buf.b_ml.mfp_ref().is_none_or(|m| m.mf_fd < 0) {
        semsg!(tr(c"E306: Cannot open %s"), &fname_used);

        ml_recover_theend(None, Some(buf), serious_error, called_from_main);

        return;
    }
    buf.b_ml.mfp_mut().mf_page_size = MIN_SWAP_PAGE_SIZE;

    let hl_id = HLF_E;
    msg_ext_set_kind(c"emsg");
    hp = mf_get(&mut buf.b_ml.mfp_mut(), 0, 1);
    let Some(hp_b0) = hp else {
        msg_start();
        msg_puts_hl(tr(c"Unable to read block 0 from "), hl_id, true);
        msg_outtrans(
            buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""),
            hl_id,
            true,
        );
        msg_puts_hl(
            tr(c"\nMaybe no changes were made or Nvim did not update the swap file."),
            hl_id,
            true,
        );
        msg_end();
        ml_recover_theend(None, Some(buf), serious_error, called_from_main);
        return;
    };
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.bhdr(hp_b0).bh_data);
    if b0p.b0_version[..7] == *b"VIM 3.0" {
        msg_start();
        msg_outtrans(buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""), 0, true);
        msg_puts_hl(tr(c" cannot be used with this version of Nvim.\n"), 0, true);
        msg_puts_hl(tr(c"Use Vim version 3.0.\n"), 0, true);
        msg_end();
        ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
        return;
    }
    if !ml_check_b0_id(&b0p) {
        semsg!(
            tr(c"E307: %s does not look like a Nvim swap file"),
            buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""),
        );
        ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
        return;
    }
    if b0_magic_wrong(&b0p) {
        msg_start();
        msg_outtrans(
            buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""),
            hl_id,
            true,
        );
        msg_puts_hl(tr(c" cannot be used on this computer.\n"), hl_id, true);
        msg_puts_hl(tr(c"The file was created on "), hl_id, true);

        b0p.b0_fname[0] = 0;

        msg_puts_hl(crate::memory::bytes_as_cstr(&b0p.b0_hname), hl_id, true);

        msg_puts_hl(tr(c",\nor the file has been damaged."), hl_id, true);
        msg_end();
        ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
        return;
    }

    let b0_page_size = char_to_long(&b0p.b0_page_size) as u32;
    let cur_page_size = buf.b_ml.mfp_ref().unwrap().mf_page_size;
    if cur_page_size != b0_page_size {
        let previous_page_size = cur_page_size;
        mf_new_page_size(&mut buf.b_ml.mfp_mut(), b0_page_size);
        let new_page_size = buf.b_ml.mfp_ref().unwrap().mf_page_size;
        if new_page_size < previous_page_size {
            msg_start();
            msg_outtrans(
                buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""),
                hl_id,
                true,
            );
            msg_puts_hl(
                tr(c" has been damaged (page size is smaller than minimum value).\n"),
                hl_id,
                true,
            );
            msg_end();
            ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
            return;
        }
        {
            let mfp = &mut *buf.b_ml.mfp_mut();
            let size: Off = vim_lseek(mfp.mf_fd, 0, SEEK_END);
            mfp.mf_blocknr_max = if size <= 0 {
                0
            } else {
                size / i64::from(mfp.mf_page_size)
            };
            mfp.mf_infile_count = mfp.mf_blocknr_max;
        }

        buf.b_ml
            .mfp_mut()
            .bhdr_mut(hp_b0)
            .bh_data
            .resize(new_page_size as usize, 0);
    }

    if directly {
        #[cfg(windows)]
        path_to_slash(&mut b0p.b0_fname);

        let mut name_buff = [0u8; crate::os::fs::MAXPATHL];
        expand_env(crate::memory::bytes_as_cstr(&b0p.b0_fname), &mut name_buff);

        let namebuff_cstr = crate::memory::cstring_until_nul(&name_buff);
        if !setfname(&curbuf(), Some(&namebuff_cstr), None, true) {
            ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
            return;
        }
    }

    msg_ext_set_kind(c"wmsg");
    msg_state().msg_ext_skip_flush.set(true);
    let mf_fname = buf.b_ml.mf_fname_cstr();
    let mf_fname_cstr = mf_fname.as_deref().unwrap_or(c"");
    let mut name_buff = [0u8; crate::os::fs::MAXPATHL];
    home_replace(None, Some(mf_fname_cstr), &mut name_buff, true);
    smsg!(0, tr(c"Using swap file \"%s\""), &name_buff);

    if let Some(sp) = buf_spname(&curbuf()) {
        crate::memory::copy_bytes_to_c_buf(&mut name_buff, sp.to_bytes());
    } else {
        home_replace(None, curbuf().ffname_opt().as_deref(), &mut name_buff, true);
    }
    msg_putchar(b'\n' as i32);
    smsg!(0, tr(c"Original file \"%s\""), &name_buff);
    msg_putchar(b'\n' as i32);
    msg_state().msg_ext_skip_flush.set(false);

    let mut org_file_info = FileInfo::new();
    let mut swp_file_info = FileInfo::new();
    let mtime = char_to_long(&b0p.b0_mtime);
    if curbuf().b_ffname.get().is_some()
        && os_fileinfo(&curbuf().ffname_cstr(), &mut org_file_info)
        && ((os_fileinfo(
            buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""),
            &mut swp_file_info,
        ) && org_file_info.stat.st_mtim.tv_sec > swp_file_info.stat.st_mtim.tv_sec)
            || mtime != org_file_info.stat.st_mtim.tv_sec as _)
    {
        emsg(tr(c"E308: Warning: Original file may have been changed"));
    }
    ui_flush();

    let b0_ff = (b0p.flags() & B0_FF_MASK) as i32;
    if (b0p.flags() & B0_HAS_FENC) != 0 {
        let fnsize = B0_FNAME_SIZE_NOCRYPT;
        let fname_slice = &b0p.b0_fname[..fnsize];
        let mut p_idx = fnsize;
        while p_idx > 0 && fname_slice[p_idx - 1] != 0 {
            p_idx -= 1;
        }
        let fenc_bytes = &fname_slice[p_idx..fnsize];
        let end = fenc_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(fenc_bytes.len());
        b0_fenc = Some(std::ffi::CString::new(&fenc_bytes[..end]).unwrap_or_default());
    }

    mf_put(&mut buf.b_ml.mfp_mut(), hp_b0, false, false);
    hp = None;

    while (curbuf().b_ml.ml_flags.get() & ML_EMPTY) == 0 {
        ml_delete(1);
    }

    if curbuf().b_ffname.get().is_some() {
        orig_file_status = readfile(
            curbuf().ffname_opt().as_deref(),
            None,
            0,
            0,
            MAXLNUM,
            None,
            READ_NEW,
            false,
        );
    }

    if b0_ff != 0 {
        set_fileformat(b0_ff - 1, OPT_LOCAL);
    }
    if let Some(ref fenc) = b0_fenc {
        let fenc_obj = Object::String(cstr_as_string(Some(fenc)));
        set_option_value_give_err(K_OPT_FILEENCODING, fenc_obj, OPT_LOCAL);
    }
    unchanged(&curbuf(), true, true);

    let mut bnum: Blocknr = 1;
    let mut page_count: u32 = 1;
    let mut lnum: Linenr = 0;
    let mut line_count: Linenr = 0;
    let mut idx: i32 = 0;
    let mut error: i32 = 0;
    let mut recover_stack: Vec<(Blocknr, i32)> = Vec::new();

    let mut cannot_open = curbuf().b_ffname.get().is_none();

    serious_error = false;
    while !input_state().got_int.get() {
        if let Some(hp_id) = hp {
            mf_put(&mut buf.b_ml.mfp_mut(), hp_id, false, false);
        }
        hp = mf_get(&mut buf.b_ml.mfp_mut(), bnum, page_count);
        let Some(hp_id) = hp else {
            if bnum == 1 {
                semsg!(
                    tr(c"E309: Unable to read block 1 from %s"),
                    buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""),
                );
                ml_recover_theend(None, Some(buf), serious_error, called_from_main);
                return;
            }
            error += 1;
            ml_append(lnum, tr(c"???MANY LINES MISSING").to_bytes(), 0, true);
            lnum += 1;
            let Some((prev_bnum, prev_idx)) = recover_stack.pop() else {
                break;
            };
            bnum = prev_bnum;
            idx = prev_idx + 1;
            page_count = 1;
            line_breakcheck();
            continue;
        };
        if buf.b_ml.bhdr(hp_id).pb_id() == PTR_ID {
            let mut ptr_block_error = false;
            let max_cnt = pb_count_max(&buf.b_ml.mfp_ref().unwrap());
            {
                let mut hp_ref = buf.b_ml.bhdr_mut(hp_id);
                if hp_ref.pb_count_max() != max_cnt {
                    ptr_block_error = true;
                    hp_ref.set_pb_count_max(max_cnt);
                }
                let pb_max = hp_ref.pb_count_max();
                if hp_ref.pb_count() > pb_max {
                    ptr_block_error = true;
                    hp_ref.set_pb_count(pb_max);
                }
            }
            if ptr_block_error {
                emsg(tr(E_WARNING_POINTER_BLOCK_CORRUPTED));
            }

            let pb_count = buf.b_ml.bhdr(hp_id).pb_count();
            if idx == 0 && line_count != 0 {
                for i in 0..(pb_count as usize) {
                    line_count -= buf
                        .b_ml
                        .mfp_ref()
                        .unwrap()
                        .bhdr(hp_id)
                        .pb_entry(i)
                        .pe_line_count;
                }
                if line_count != 0 {
                    error += 1;
                    ml_append(lnum, tr(c"???LINE COUNT WRONG").to_bytes(), 0, true);
                    lnum += 1;
                }
            }

            if pb_count == 0 {
                ml_append(lnum, tr(c"???EMPTY BLOCK").to_bytes(), 0, true);
                lnum += 1;
                error += 1;
            } else if idx < i32::from(pb_count) {
                let pe = buf
                    .b_ml
                    .mfp_ref()
                    .unwrap()
                    .bhdr(hp_id)
                    .pb_entry(idx as usize);
                if pe.pe_bnum < 0 {
                    if !cannot_open {
                        line_count = pe.pe_line_count;
                        let pe_old_lnum = pe.pe_old_lnum;
                        if line_count <= 0
                            || pe_old_lnum < 1
                            || readfile(
                                curbuf().ffname_opt().as_deref(),
                                None,
                                lnum,
                                pe_old_lnum - 1,
                                line_count,
                                None,
                                0,
                                false,
                            ) != Some(true)
                        {
                            cannot_open = true;
                        } else {
                            lnum += line_count;
                        }
                    }
                    if cannot_open {
                        error += 1;
                        ml_append(lnum, tr(c"???LINES MISSING").to_bytes(), 0, true);
                        lnum += 1;
                    }
                    idx += 1;
                    line_breakcheck();
                    continue;
                }

                let max_blocknr = buf.b_ml.mfp_ref().unwrap().mf_blocknr_max;
                let prev_bnum = bnum;
                let prev_idx = idx;
                recover_stack.push((prev_bnum, prev_idx));

                bnum = pe.pe_bnum;
                line_count = pe.pe_line_count;
                page_count = pe.pe_page_count as u32;
                if page_count < 1
                    || bnum.saturating_add(i64::from(page_count)) > max_blocknr.saturating_add(1)
                {
                    error += 1;
                    ml_append(lnum, tr(c"???ILLEGAL BLOCK NUMBER").to_bytes(), 0, true);
                    lnum += 1;
                    idx = prev_idx + 1;
                    bnum = prev_bnum;
                    page_count = 1;
                    recover_stack.pop();
                    line_breakcheck();
                    continue;
                }
                idx = 0;
                line_breakcheck();
                continue;
            }
        } else if buf.b_ml.bhdr(hp_id).db_id() != DATA_ID {
            if bnum == 1 {
                semsg!(
                    tr(c"E310: Block 1 ID wrong (%s not a .swp file?)"),
                    buf.b_ml.mf_fname_cstr().as_deref().unwrap_or(c""),
                );
                ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
                return;
            }
            error += 1;
            ml_append(lnum, tr(c"???BLOCK MISSING").to_bytes(), 0, true);
            lnum += 1;
        } else {
            let mut has_error = false;
            let hp_page_count = buf.b_ml.bhdr(hp_id).bh_page_count;
            if hp_page_count != page_count {
                error += 1;
                ml_append(
                    lnum,
                    tr(c"??? BLOCK PAGE COUNT MISMATCH").to_bytes(),
                    0,
                    true,
                );
                lnum += 1;

                page_count = hp_page_count;
            }

            let mf_page_size = buf.b_ml.mfp_ref().unwrap().mf_page_size;
            let mut db_txt_end = buf.b_ml.bhdr(hp_id).db_txt_end();
            if page_count.wrapping_mul(mf_page_size) != db_txt_end {
                ml_append(
                    lnum,
                    tr(c"??? from here until ???END lines may be messed up").to_bytes(),
                    0,
                    true,
                );
                lnum += 1;
                error += 1;
                has_error = true;
                db_txt_end = page_count.wrapping_mul(mf_page_size);
                buf.b_ml
                    .mfp_mut()
                    .bhdr_mut(hp_id)
                    .set_db_txt_end(db_txt_end);
            }

            let mut db_txt_start = buf.b_ml.bhdr(hp_id).db_txt_start();
            if (db_txt_start as usize) < HEADER_SIZE || db_txt_start > db_txt_end {
                ml_append(lnum, tr(c"??? block header corrupted").to_bytes(), 0, true);
                lnum += 1;
                error += 1;
                has_error = true;
                db_txt_start = db_txt_end;
                buf.b_ml
                    .mfp_mut()
                    .bhdr_mut(hp_id)
                    .set_db_txt_start(db_txt_start);
            }

            buf.b_ml.bhdr_mut(hp_id).bh_data[db_txt_end as usize - 1] = 0;

            let db_line_count = buf.b_ml.bhdr(hp_id).db_line_count();
            if i64::from(line_count) != db_line_count {
                ml_append(
                    lnum,
                    tr(c"??? from here until ???END lines may have been inserted/deleted")
                        .to_bytes(),
                    0,
                    true,
                );
                lnum += 1;
                error += 1;
                has_error = true;
            }

            let mut did_questions = false;
            for i in 0..(db_line_count as isize) {
                if HEADER_SIZE + (i as usize) * INDEX_SIZE >= db_txt_start as usize {
                    error += 1;
                    ml_append(lnum, tr(c"??? lines may be missing").to_bytes(), 0, true);
                    lnum += 1;
                    break;
                }

                let idx_val = buf.b_ml.bhdr(hp_id).db_idx(i as usize);
                let txt_start = (idx_val & DB_INDEX_MASK) as i32;
                if txt_start <= HEADER_SIZE as i32 || txt_start >= db_txt_end as i32 {
                    error += 1;
                    if did_questions {
                        continue;
                    }
                    did_questions = true;
                    ml_append(lnum, b"???", 0, true);
                } else {
                    did_questions = false;
                    let line_vec = crate::memory::bytes_as_cstr(
                        &buf.b_ml.bhdr(hp_id).bh_data[txt_start as usize..db_txt_end as usize],
                    )
                    .to_bytes()
                    .to_vec();
                    ml_append(lnum, &line_vec, 0, true);
                }
                lnum += 1;
            }
            if has_error {
                ml_append(lnum, tr(c"???END").to_bytes(), 0, true);
                lnum += 1;
            }
        }

        let Some((prev_bnum, prev_idx)) = recover_stack.pop() else {
            break;
        };

        bnum = prev_bnum;
        idx = prev_idx + 1;

        page_count = 1;
        line_breakcheck();
    }

    if orig_file_status != Some(true) || curbuf().b_ml.ml_line_count.get() != lnum * 2 + 1 {
        if !(curbuf().b_ml.ml_line_count.get() == 2 && ml_get_slice(1).is_empty()) {
            changed_internal(&curbuf());
            buf_inc_changedtick(&curbuf());
        }
    } else {
        for i_lnum in 1..=lnum {
            let p_line = ml_get_slice(i_lnum).to_vec();
            if p_line.as_slice() != ml_get_slice(i_lnum + lnum) {
                changed_internal(&curbuf());
                buf_inc_changedtick(&curbuf());
                break;
            }
        }
    }

    while curbuf().b_ml.ml_line_count.get() > lnum && (curbuf().b_ml.ml_flags.get() & ML_EMPTY) == 0
    {
        ml_delete(curbuf().b_ml.ml_line_count.get());
    }
    curbuf()
        .b_flags
        .set(curbuf().b_flags.get() | (BF_RECOVERED));
    check_cursor(&curwin());

    msg_state()
        .msg_ext_skip_flush
        .set(!input_state().got_int.get());
    exc_state().recoverymode.set(false);
    if input_state().got_int.get() {
        emsg(tr(c"E311: Recovery Interrupted"));
    } else if error != 0 {
        msg_state()
            .no_wait_return
            .set(msg_state().no_wait_return.get() + (1));
        msg_ext_set_kind(c"emsg");
        msg(c">>>>>>>>>>>>>\n", 0);
        emsg(tr(
            c"E312: Errors detected while recovering; look for lines starting with ???",
        ));
        msg_state()
            .no_wait_return
            .set(msg_state().no_wait_return.get() - (1));
        msg_putchar(b'\n' as i32);
        msg(tr(c"See \":help E312\" for more information."), 0);
        msg(c"\n>>>>>>>>>>>>>", 0);
    } else {
        msg_ext_set_kind(c"wmsg");
        if curbuf().b_changed.get() != 0 {
            msg(
                tr(c"Recovery completed. You should check if everything is OK."),
                0,
            );
            msg_puts(tr(
                c"\n(You might want to write out this file under another name\n",
            ));
            msg_puts(tr(
                c"and run diff with the original file to check for changes)",
            ));
        } else {
            msg(
                tr(c"Recovery completed. Buffer contents equals file contents."),
                0,
            );
        }
        msg_puts(tr(c"\nYou may want to delete the .swp file now."));
        if swapfile_proc_running(&b0p, &fname_used) != 0 {
            msg_puts(tr(c"\nNote: process STILL RUNNING: "));

            msg_outnum(char_to_long(&b0p.b0_pid) as i32);
        }
        if !ui_has(K_UIMESSAGES) {
            msg_puts(c"\n\n");
        }
        cmdline_state().cmdline_row.set(msg_state().msg_row.get());
    }
    redraw_curbuf_later(UPD_NOT_VALID);

    ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
}

fn ml_recover_theend(
    hp: Option<BhdrId>,
    buf: Option<std::rc::Rc<FileBuf>>,
    serious_error: bool,
    called_from_main: bool,
) {
    msg_state().msg_ext_skip_flush.set(false);
    exc_state().recoverymode.set(false);
    if let Some(ref b) = buf
        && let Some(mut mfp) = b.b_ml.mfp_opt()
    {
        if let Some(hp) = hp {
            mf_put(&mut mfp, hp, false, false);
        }
        drop(mfp);
        mf_close(b.b_ml.ml_mfp.borrow_mut().take(), false);
    }
    drop(buf);
    if serious_error && called_from_main {
        ml_close(&curbuf(), 1);
    } else {
        apply_autocmds(
            EVENT_BUFREADPOST,
            None,
            curbuf().fname_opt().as_deref(),
            false,
            curbuf_opt().as_deref(),
        );
        apply_autocmds(
            EVENT_BUFWINENTER,
            None,
            curbuf().fname_opt().as_deref(),
            false,
            curbuf_opt().as_deref(),
        );
    }
}

/// Enumerate swapfiles for `fname`, appending each found path to `ret_list`.
pub fn recover_names(fname: Option<&CStr>, skip_curbuf: bool, ret_list: &TvList) {
    #[cfg(unix)]
    let mut fname_buf = [0u8; MAXPATHL];
    #[cfg(unix)]
    let fname_res: Option<&CStr> = fname.map(|f| {
        if resolve_symlink(Some(f), &mut fname_buf) {
            crate::memory::bytes_as_cstr(&fname_buf)
        } else {
            f
        }
    });
    #[cfg(not(unix))]
    let fname_res: Option<&CStr> = fname;

    let dir_cap = (opt_vars().p_dir.as_bytes().len() + 1).min(31000);
    let mut dir_name_vec: Vec<u8> = vec![0; dir_cap];
    let mut dir_name: NvimString;
    let p_dir = opt_vars().p_dir.to_owned();
    let mut dirp = p_dir.as_bytes();
    while !dirp.is_empty() {
        let size = copy_option_part(&mut dirp, &mut dir_name_vec, c",");
        dir_name = NvimString::from_bytes_len(&dir_name_vec, size);

        let dir_bytes = dir_name.as_bytes();
        let mut names_vec: Vec<std::ffi::CString> = Vec::with_capacity(6);
        if dir_bytes == b"." {
            if let Some(f_res) = fname_res {
                recov_file_names(&mut names_vec, f_res, true);
            } else {
                names_vec.push(c"*.sw?".to_owned());
                names_vec.push(c".*.sw?".to_owned());
                names_vec.push(c".sw?".to_owned());
            }
        } else if let Some(f_res) = fname_res {
            let tail = if after_pathsep(dir_bytes)
                && dir_bytes.len() > 1
                && dir_bytes[dir_bytes.len() - 1] == dir_bytes[dir_bytes.len() - 2]
            {
                make_percent_swname(&dir_bytes[..dir_bytes.len() - 1], Some(f_res))
            } else {
                let t = path_tail(f_res.to_bytes());
                Some(concat_fnames(
                    dir_name,
                    crate::api::private::helpers::bytes_to_string(t),
                    true,
                ))
            };
            if let Some(ref tail) = tail {
                recov_file_names(&mut names_vec, tail, false);
            }
        } else {
            for pat in [c"*.sw?", c".*.sw?", c".sw?"] {
                names_vec.push(concat_fnames(
                    dir_name.clone(),
                    cstr_as_string(Some(pat)),
                    true,
                ));
            }
        }

        let mut files: Vec<std::ffi::CString> = Vec::new();
        if !names_vec.is_empty() {
            let name_cstrs: Vec<&[u8]> =
                names_vec.iter().map(std::ffi::CString::as_bytes).collect();
            let _ = expand_wildcards_vec(&name_cstrs, &mut files, EW_KEEPALL | EW_FILE | EW_SILENT);
        }

        let cur_mfp_fname = if skip_curbuf {
            curbuf().b_ml.mf_fname_cstr()
        } else {
            None
        };

        if let Some(ref cur_p) = cur_mfp_fname {
            files.retain(|f| !path_equal(cur_p.to_bytes(), f.as_bytes(), K_PATH_CMP_FULL));
        }

        if dirp.is_empty()
            && tv_list_len(Some(ret_list)) == 0
            && files.is_empty()
            && let Some(f_res) = fname_res
            && let Some(swapname) = modname(Some(f_res), c".swp", true)
            && os_path_exists(&swapname)
            && cur_mfp_fname.as_deref().is_none_or(|cur_p| {
                !path_equal(cur_p.to_bytes(), swapname.as_bytes(), K_PATH_CMP_FULL)
            })
        {
            crate::eval::typval::tv_list_append_string(ret_list, Some(swapname.as_c_str()), -1);
        }

        for f in files {
            crate::eval::typval::tv_list_append_string(
                ret_list,
                Some(f.as_c_str()),
                f.as_bytes().len() as isize,
            );
        }
    }
}

/// Append the full path to `name` with path separators made into percent signs to `dir_slice`.
pub fn make_percent_swname(dir_slice: &[u8], name: Option<&CStr>) -> Option<std::ffi::CString> {
    let fname = fix_fname(Some(name.map_or(b"", CStr::to_bytes)))?;
    let mut fname_vec: Vec<u8> = fname.into_bytes_with_nul();

    let mut file_info = FileInfo::new();
    if !os_fileinfo2(crate::memory::bytes_as_cstr(&fname_vec), &mut file_info) {
        return None;
    }
    let mut fixed_off = file_info.root_off;
    if file_info.r#type == K_PATH_DEVICE_UNC {
        assert!(file_info.root_off >= 2);
        fixed_off -= 2;
        fname_vec[fixed_off] = b'/';
    }
    let mut idx = fixed_off;
    while idx < fname_vec.len() && fname_vec[idx] != 0 {
        if vim_ispathsep(fname_vec[idx] as i32) {
            fname_vec[idx] = b'%';
        }
        let step = crate::mbyte::utfc_slice2len(&fname_vec[idx..]);
        idx += step.max(1);
    }
    let fixed_fname = NvimString::from_bytes_len(&fname_vec[fixed_off..], idx - fixed_off);

    Some(concat_fnames(
        NvimString::from_bytes(dir_slice),
        fixed_fname,
        true,
    ))
}

/// For Vimscript `swapinfo()`.
pub fn swapfile_dict(fname: &CStr, d: &TvDict) {
    let fd = os_open(fname, O_RDONLY, 0);
    if fd >= 0 {
        if let Some(b0) = ZeroBlock::read_from_fd(fd) {
            if !ml_check_b0_id(&b0) {
                tv_dict_add_str_len(d, c"error", Some(b"Not a swap file"));
            } else if b0_magic_wrong(&b0) {
                tv_dict_add_str_len(d, c"error", Some(b"Magic number mismatch"));
            } else {
                tv_dict_add_str_len(
                    d,
                    c"version",
                    Some(crate::memory::bytes_until_nul(&b0.b0_version[..10])),
                );
                tv_dict_add_str_len(
                    d,
                    c"user",
                    Some(crate::memory::bytes_until_nul(
                        &b0.b0_uname[..B0_UNAME_SIZE],
                    )),
                );
                tv_dict_add_str_len(
                    d,
                    c"host",
                    Some(crate::memory::bytes_until_nul(
                        &b0.b0_hname[..B0_HNAME_SIZE],
                    )),
                );
                tv_dict_add_str_len(
                    d,
                    c"fname",
                    Some(crate::memory::bytes_until_nul(
                        &b0.b0_fname[..B0_FNAME_SIZE_ORG],
                    )),
                );

                tv_dict_add_nr(d, c"pid", i64::from(swapfile_proc_running(&b0, fname)));
                tv_dict_add_nr(d, c"mtime", char_to_long(&b0.b0_mtime));
                tv_dict_add_nr(d, c"dirty", if b0.dirty() != 0 { 1 } else { 0 });
                tv_dict_add_nr(d, c"inode", char_to_long(&b0.b0_ino));
            }
        } else {
            tv_dict_add_str_len(d, c"error", Some(b"Cannot read file"));
        }
        os_close(fd);
    } else {
        tv_dict_add_str_len(d, c"error", Some(b"Cannot open file"));
    }
}

fn swapfile_info(fname: &CStr, msg: &mut Vec<u8>) -> time_t {
    let mut x: time_t = 0;

    let mut file_info = FileInfo::new();
    if os_fileinfo(fname, &mut file_info) {
        #[cfg(unix)]
        {
            let mut uname = [0u8; B0_UNAME_SIZE];
            if os_get_uname(file_info.stat.st_uid as u32, &mut uname) {
                sb_push_cstr(msg, tr(c"          owned by: "));
                sb_push_cstr(msg, crate::memory::bytes_as_cstr(&uname));
                sb_push_cstr(msg, tr(c"   dated: "));
            } else {
                sb_push_cstr(msg, tr(c"             dated: "));
            }
        }
        #[cfg(not(unix))]
        {
            sb_push_cstr(msg, tr(c"             dated: "));
        }
        x = file_info.stat.st_mtim.tv_sec as _;
        let mut ctime_buf = [0u8; 100];
        let ct = os_ctime_r(&x, &mut ctime_buf, true);
        sb_push_cstr(msg, ct);
    }

    let fd = os_open(fname, O_RDONLY, 0);
    if fd >= 0 {
        if let Some(b0) = ZeroBlock::read_from_fd(fd) {
            if b0.b0_version[..7] == *b"VIM 3.0" {
                sb_push_cstr(msg, tr(c"         [from Vim version 3.0]"));
            } else if !ml_check_b0_id(&b0) {
                sb_push_cstr(msg, tr(c"         [does not look like a Nvim swap file]"));
            } else if !ml_check_b0_strings(&b0) {
                sb_push_cstr(msg, tr(c"         [garbled strings (not nul terminated)]"));
            } else {
                sb_push_cstr(msg, tr(c"         file name: "));
                if b0.b0_fname[0] == 0 {
                    sb_push_cstr(msg, tr(c"[No Name]"));
                } else {
                    sb_push_cstr(msg, crate::memory::bytes_as_cstr(&b0.b0_fname));
                }

                sb_push_cstr(msg, tr(c"\n          modified: "));
                sb_push_cstr(
                    msg,
                    if b0.dirty() != 0 {
                        tr(c"YES")
                    } else {
                        tr(c"no")
                    },
                );

                if b0.b0_uname[0] != 0 {
                    sb_push_cstr(msg, tr(c"\n         user name: "));
                    sb_push_cstr(msg, crate::memory::bytes_as_cstr(&b0.b0_uname));
                }

                if b0.b0_hname[0] != 0 {
                    if b0.b0_uname[0] != 0 {
                        sb_push_cstr(msg, tr(c"   host name: "));
                    } else {
                        sb_push_cstr(msg, tr(c"\n         host name: "));
                    }
                    sb_push_cstr(msg, crate::memory::bytes_as_cstr(&b0.b0_hname));
                }

                let pid = char_to_long(&b0.b0_pid) as i32;
                if pid != 0 {
                    sb_push_cstr(msg, tr(c"\n        process ID: "));
                    msg.extend_from_slice(pid.to_string().as_bytes());
                    let running = swapfile_proc_running(&b0, fname);
                    ml_st(|st| st.proc_running.set(running));
                    if running != 0 {
                        sb_push_cstr(msg, tr(c" (STILL RUNNING)"));
                    }
                }

                if b0_magic_wrong(&b0) {
                    sb_push_cstr(msg, tr(c"\n         [not usable on this computer]"));
                }
            }
        } else {
            sb_push_cstr(msg, tr(c"         [cannot be read]"));
        }
        os_close(fd);
    } else {
        sb_push_cstr(msg, tr(c"         [cannot be opened]"));
    }
    msg.push(b'\n');

    x
}

fn swapfile_unchanged(fname: &CStr) -> bool {
    if !os_path_exists(fname) {
        return false;
    }
    let fd = os_open(fname, O_RDONLY, 0);
    if fd < 0 {
        return false;
    }
    let Some(mut b0) = ZeroBlock::read_from_fd(fd) else {
        os_close(fd);
        return false;
    };

    let mut ret = true;
    if !ml_check_b0_id(&b0) || b0_magic_wrong(&b0) {
        ret = false;
    }
    if b0.dirty() != 0 {
        ret = false;
    }
    if b0.b0_hname[0] == 0 {
        ret = false;
    } else {
        let mut hostname = [0u8; B0_HNAME_SIZE];
        os_get_hostname(&mut hostname);
        hostname[B0_HNAME_SIZE - 1] = 0;
        b0.b0_hname[B0_HNAME_SIZE - 1] = 0;
        if !crate::memory::bytes_as_cstr(&b0.b0_hname)
            .to_bytes()
            .eq_ignore_ascii_case(crate::memory::bytes_until_nul(&hostname))
        {
            ret = false;
        }
    }
    if char_to_long(&b0.b0_pid) == 0 || swapfile_proc_running(&b0, fname) != 0 {
        ret = false;
    }
    os_close(fd);
    ret
}

fn recov_file_names(names: &mut Vec<std::ffi::CString>, path: &CStr, prepend_dot: bool) {
    if prepend_dot {
        let Some(m) = modname(Some(path), c".sw?", true) else {
            return;
        };
        names.push(m);
    }

    let cur = concat_fnames(
        cstr_as_string(Some(path)),
        NvimString::from_cstr(c".sw?"),
        false,
    );
    if let Some(prev) = names.last() {
        let prev_bytes = prev.as_bytes();
        let cur_bytes = cur.as_bytes();
        let tail_prev = if prev_bytes.len() > cur_bytes.len() {
            &prev_bytes[prev_bytes.len() - cur_bytes.len()..]
        } else {
            prev_bytes
        };
        if tail_prev != cur_bytes {
            names.push(cur);
        }
    } else {
        names.push(cur);
    }
}

fn ml_sync_swap_blocks(buf: &FileBuf) {
    ml_flush_line(buf, false);
    if !buf.b_ml.ml_swap_dirty.get() {
        return;
    }

    let Some(mut mfp) = buf.b_ml.mfp_opt() else {
        return;
    };
    let mut chunks = buf.b_ml.ml_chunks.borrow_mut();

    let has_any_clean_chunk = chunks.iter().any(|ch| !ch.swap_blocks.is_empty());
    let root_pb_id = if !has_any_clean_chunk {
        let b0_id_opt = mfp.find_bhdr(0);
        let b0_bhdr = b0_id_opt.and_then(|id| mfp.remove_bhdr(id));
        mfp.mf_blocks.clear();
        mfp.mf_free_slots.clear();
        mfp.mf_hash.clear();
        mfp.mf_free_list.clear();
        mfp.mf_trans.clear();
        mfp.mf_blocknr_max = 0;
        mfp.mf_blocknr_min = -1;
        mfp.mf_neg_count = 0;
        mfp.mf_infile_count = mfp.mf_infile_count.min(1);

        if let Some(b0) = b0_bhdr {
            mfp.mf_blocknr_max = 1;
            let _ = mfp.insert_bhdr(b0);
        } else {
            let hp0 = mf_new(&mut mfp, false, 1);
            mf_put(&mut mfp, hp0, true, false);
        }

        let pb_id = ml_new_ptr(&mut mfp);
        debug_assert_eq!(mfp.bhdr(pb_id).bh_bnum, 1);
        pb_id
    } else {
        if mfp.find_bhdr(0).is_none() {
            let hp0 = mf_new(&mut mfp, false, 1);
            mf_put(&mut mfp, hp0, true, false);
        }
        let pb_id = if let Some(id1) = mfp.find_bhdr(1) {
            mfp.bhdr_mut(id1).bh_flags |= BH_LOCKED | BH_DIRTY;
            id1
        } else {
            ml_new_ptr(&mut mfp)
        };
        let mut keep_slot = vec![false; mfp.mf_blocks.len()];
        if let Some(b0_id) = mfp.find_bhdr(0) {
            keep_slot[b0_id.0] = true;
        }
        keep_slot[pb_id.0] = true;
        for ch in chunks.iter() {
            for &(_, db_id) in &ch.swap_blocks {
                if db_id.0 < keep_slot.len() {
                    keep_slot[db_id.0] = true;
                }
            }
        }
        for (slot_idx, &keep) in keep_slot.iter().enumerate() {
            if !keep && mfp.mf_blocks[slot_idx].is_some() {
                mf_free(&mut mfp, BhdrId(slot_idx));
            }
        }
        pb_id
    };

    let page_size = mfp.mf_page_size;
    let mut data_entries: Vec<PointerEntry> = Vec::with_capacity(chunks.len() * 8);

    if chunks.is_empty() {
        let db_id = ml_new_data(&mut mfp, false, 1);
        let db_bnum = mfp.bhdr(db_id).bh_bnum;
        {
            let hp_ref = mfp.bhdr_mut(db_id);
            let new_txt_start = hp_ref.db_txt_start() - 1;
            hp_ref.set_db_txt_start(new_txt_start);
            hp_ref.set_db_idx(0, new_txt_start);
            let free = hp_ref.db_free() - (1 + INDEX_SIZE as u32);
            hp_ref.set_db_free(free);
            hp_ref.set_db_line_count(1);
            bh_write_bytes(hp_ref, new_txt_start as usize, b"", 1);
        }
        mf_put(&mut mfp, db_id, true, false);
        data_entries.push(PointerEntry {
            pe_bnum: db_bnum,
            pe_page_count: 1,
            pe_old_lnum: 1,
            pe_line_count: 1,
        });
    } else {
        fn serialize_chunk_blocks(
            ch: &LineChunk,
            page_size: u32,
        ) -> Vec<(crate::memfile::Bhdr, PointerEntry)> {
            let mut blocks = Vec::with_capacity(8);
            let mut line_iter = ch.lines.iter().peekable();
            let mut cur_lnum = ch.start_lnum;
            loop {
                let first_entry = line_iter.peek().copied().unwrap_or(&LineEntry::EMPTY);
                let first_len = first_entry.text.to_bytes().len() + 1;
                let first_needed = HEADER_SIZE + INDEX_SIZE + first_len;
                let page_count = first_needed.div_ceil(page_size as usize).max(1) as u32;
                let txt_end = page_count.wrapping_mul(page_size);
                let mut hp = crate::memfile::Bhdr {
                    bh_bnum: 0,
                    bh_data: vec![0u8; txt_end as usize],
                    bh_page_count: page_count,
                    bh_flags: BH_DIRTY,
                };
                hp.set_db_id(DATA_ID);
                hp.set_db_txt_end(txt_end);
                hp.set_db_txt_start(txt_end);
                let mut free = txt_end - (HEADER_SIZE as u32);
                hp.set_db_free(free);
                hp.set_db_line_count(0);

                let start_lnum = cur_lnum;
                let mut db_line_count: i32 = 0;

                while let Some(&entry) = line_iter.peek() {
                    let text_bytes = entry.text.to_bytes();
                    let len = text_bytes.len() + 1;
                    let needed = (len + INDEX_SIZE) as u32;

                    if db_line_count > 0 && free < needed {
                        break;
                    }
                    let _ = line_iter.next();

                    let new_txt_start = hp.db_txt_start() - (len as u32);
                    hp.set_db_txt_start(new_txt_start);
                    let idx_val = new_txt_start | if entry.marked { DB_MARKED } else { 0 };
                    hp.set_db_idx(db_line_count as usize, idx_val);
                    free -= needed;
                    hp.set_db_free(free);
                    hp.set_db_line_count(i64::from(db_line_count + 1));
                    bh_write_bytes(&mut hp, new_txt_start as usize, text_bytes, len);

                    db_line_count += 1;
                    cur_lnum += 1;
                }

                if db_line_count == 0 {
                    let new_txt_start = hp.db_txt_start() - 1;
                    hp.set_db_txt_start(new_txt_start);
                    hp.set_db_idx(0, new_txt_start);
                    let free2 = hp.db_free() - (1 + INDEX_SIZE as u32);
                    hp.set_db_free(free2);
                    hp.set_db_line_count(1);
                    bh_write_bytes(&mut hp, new_txt_start as usize, b"", 1);
                    db_line_count = 1;
                }

                let pe = PointerEntry {
                    pe_bnum: 0,
                    pe_page_count: page_count as i32,
                    pe_old_lnum: start_lnum,
                    pe_line_count: db_line_count,
                };
                blocks.push((hp, pe));

                if line_iter.peek().is_none() {
                    break;
                }
            }
            blocks
        }

        let dirty_count = chunks.iter().filter(|ch| ch.swap_blocks.is_empty()).count();
        let mut prebuilt: Vec<Option<Vec<(crate::memfile::Bhdr, PointerEntry)>>> =
            if dirty_count >= 8 {
                let chunk_refs: &[LineChunk] = &chunks;
                let nthreads = std::thread::available_parallelism()
                    .map_or(4, |n| n.get().min(8))
                    .min(chunk_refs.len());
                let group_len = chunk_refs.len().div_ceil(nthreads).max(1);
                std::thread::scope(|s| {
                    let mut handles = Vec::with_capacity(nthreads);
                    for slice in chunk_refs.chunks(group_len) {
                        handles.push(s.spawn(move || {
                            slice
                                .iter()
                                .map(|ch| {
                                    ch.swap_blocks
                                        .is_empty()
                                        .then(|| serialize_chunk_blocks(ch, page_size))
                                })
                                .collect::<Vec<_>>()
                        }));
                    }
                    let mut all = Vec::with_capacity(chunk_refs.len());
                    for h in handles {
                        if let Ok(mut part) = h.join() {
                            all.append(&mut part);
                        }
                    }
                    all
                })
            } else {
                Vec::new()
            };

        for (c_idx, ch) in chunks.iter_mut().enumerate() {
            if !ch.swap_blocks.is_empty() {
                let mut cur_lnum = ch.start_lnum;
                for &mut (ref mut pe, _) in &mut ch.swap_blocks {
                    pe.pe_old_lnum = cur_lnum;
                    cur_lnum += pe.pe_line_count;
                    data_entries.push(*pe);
                }
                continue;
            }

            let blocks = prebuilt
                .get_mut(c_idx)
                .and_then(Option::take)
                .unwrap_or_else(|| serialize_chunk_blocks(ch, page_size));

            for (bhdr, mut pe) in blocks {
                let (db_id, db_bnum) = crate::memfile::mf_insert_new_bhdr(&mut mfp, bhdr);
                pe.pe_bnum = db_bnum;
                ch.swap_blocks.push((pe, db_id));
                data_entries.push(pe);
            }
        }
    }

    let max_cnt = pb_count_max(&mfp) as usize;
    let mut cur_entries = data_entries;
    while cur_entries.len() > max_cnt {
        let mut parent_entries = Vec::with_capacity(cur_entries.len().div_ceil(max_cnt));
        for chunk in cur_entries.chunks(max_cnt) {
            let child_pb_id = ml_new_ptr(&mut mfp);
            let child_bnum = mfp.bhdr(child_pb_id).bh_bnum;
            let mut chunk_lines: i32 = 0;
            {
                let hp_ref = mfp.bhdr_mut(child_pb_id);
                hp_ref.set_pb_count(chunk.len() as u16);
                for (i, &pe) in chunk.iter().enumerate() {
                    hp_ref.set_pb_entry(i, pe);
                    chunk_lines += pe.pe_line_count;
                }
            }
            mf_put(&mut mfp, child_pb_id, true, false);
            parent_entries.push(PointerEntry {
                pe_bnum: child_bnum,
                pe_page_count: 1,
                pe_old_lnum: chunk[0].pe_old_lnum,
                pe_line_count: chunk_lines,
            });
        }
        cur_entries = parent_entries;
    }

    {
        let hp_ref = mfp.bhdr_mut(root_pb_id);
        hp_ref.set_pb_count(cur_entries.len() as u16);
        for (i, &pe) in cur_entries.iter().enumerate() {
            hp_ref.set_pb_entry(i, pe);
        }
    }
    mf_put(&mut mfp, root_pb_id, true, false);
    drop(chunks);
    drop(mfp);
    buf.b_ml.ml_swap_dirty.set(false);
}

/// Sync all memlines.
pub fn ml_sync_all(check_file: i32, check_char: i32, do_fsync: bool) {
    for buf in for_all_buffers_ptr() {
        if buf.b_ml.mfp_ref().is_none_or(|m| m.mf_fname.is_none()) {
            continue;
        }
        ml_flush_line(&buf, false);
        if buf_is_changed_fn(&buf) && check_file != 0 && buf.b_ffname.get().is_some() {
            let mut file_info = FileInfo::new();
            if !os_fileinfo(&buf.ffname_cstr(), &mut file_info)
                || buf.b_mtime_read.get() != file_info.stat.st_mtim.tv_sec as _
                || buf.b_mtime_read_ns.get() != file_info.stat.st_mtim.tv_nsec as _
                || os_fileinfo_size(&file_info) != buf.b_orig_size.get()
            {
                ml_preserve(&buf, false, do_fsync);
                workspace_state().did_check_timestamps.set(false);
                workspace_state().need_check_timestamps.set(true);
            }
        }
        if buf.b_ml.ml_swap_dirty.get()
            || buf
                .b_ml
                .mfp_ref()
                .is_some_and(|m| m.mf_dirty == MF_DIRTY_YES)
        {
            ml_sync_swap_blocks(&buf);
            let sync_flags = (if check_char != 0 { MFS_STOP } else { 0 })
                | (if do_fsync && buf_is_changed_fn(&buf) {
                    MFS_FLUSH
                } else {
                    0
                });
            mf_sync(&mut buf.b_ml.mfp_mut(), sync_flags);
            if check_char != 0 && os_char_avail() {
                break;
            }
        }
    }
}

/// Sync one buffer, including negative blocks.
pub fn ml_preserve(buf: &FileBuf, message: bool, do_fsync: bool) {
    let got_int_save = input_state().got_int.get();

    if buf.b_ml.mfp_ref().is_none_or(|m| m.mf_fname.is_none()) {
        if message {
            emsg(tr(c"E313: Cannot preserve, there is no swap file"));
        }
        return;
    }

    input_state().got_int.set(false);

    ml_sync_swap_blocks(buf);
    let status = mf_sync(
        &mut buf.b_ml.mfp_mut(),
        MFS_ALL | if do_fsync { MFS_FLUSH } else { 0 },
    );

    input_state()
        .got_int
        .set(input_state().got_int.get() | (got_int_save));

    if message {
        if status {
            msg(tr(c"File preserved"), 0);
        } else {
            emsg(tr(c"E314: Preserve failed"));
        }
    }
}

/// Borrow the bytes (excluding NUL) of line `lnum` in `buf` directly from `Memline`.
#[inline(always)]
pub fn with_ml_buf_line<R>(buf: &FileBuf, lnum: Linenr, f: impl FnOnce(&[u8]) -> R) -> R {
    if lnum >= 1
        && lnum <= buf.b_ml.ml_line_count.get()
        && (buf.b_ml.ml_line_lnum.get() != lnum
            || (buf.b_ml.ml_flags.get() & (ML_LINE_DIRTY | ML_LINE_INPLACE)) == 0)
        && buf.b_ml.ml_mfp.borrow().is_some()
    {
        let cstr_opt = {
            let chunks = buf.b_ml.ml_chunks.borrow();
            buf.b_ml.find_chunk_idx(&chunks, lnum).and_then(|c| {
                let ch = &chunks[c];
                ch.lines
                    .get((lnum - ch.start_lnum) as usize)
                    .map(|e| e.text.clone())
            })
        };
        if let Some(cstr) = cstr_opt {
            return f(cstr.to_bytes());
        }
    }
    if !ml_load_line(buf, lnum, false) {
        return f(&[]);
    }
    buf.b_ml.with_current_line_slice(|s| {
        let line = if (buf.b_ml.ml_flags.get() & ML_LINE_INPLACE) != 0 {
            crate::memory::bytes_until_nul(s)
        } else {
            &s[..s.len().saturating_sub(1)]
        };
        f(line)
    })
}

/// Borrow the NUL-terminated bytes (including trailing NUL) of line `lnum` in `buf`.
#[inline(always)]
pub fn with_ml_buf_bytes_with_nul<R>(buf: &FileBuf, lnum: Linenr, f: impl FnOnce(&[u8]) -> R) -> R {
    if lnum >= 1
        && lnum <= buf.b_ml.ml_line_count.get()
        && (buf.b_ml.ml_line_lnum.get() != lnum
            || (buf.b_ml.ml_flags.get() & (ML_LINE_DIRTY | ML_LINE_INPLACE)) == 0)
        && buf.b_ml.ml_mfp.borrow().is_some()
    {
        let cstr_opt = {
            let chunks = buf.b_ml.ml_chunks.borrow();
            buf.b_ml.find_chunk_idx(&chunks, lnum).and_then(|c| {
                let ch = &chunks[c];
                ch.lines
                    .get((lnum - ch.start_lnum) as usize)
                    .map(|e| e.text.clone())
            })
        };
        if let Some(cstr) = cstr_opt {
            return f(cstr.to_bytes_with_nul());
        }
    }
    if !ml_load_line(buf, lnum, false) {
        return f(b"\0");
    }
    buf.b_ml.with_current_line_slice(|s| {
        let slice = if (buf.b_ml.ml_flags.get() & ML_LINE_INPLACE) != 0 {
            let len = crate::memory::bytes_until_nul(s).len();
            &s[..(len + 1).min(s.len())]
        } else {
            s
        };
        f(slice)
    })
}

/// Return a reference to a (read-only copy of a) line in `curbuf`.
pub fn ml_get(lnum: Linenr) -> crate::option_vars::RcCStr {
    ml_get_buf_impl(&curbuf(), lnum, false)
}

/// Return a reference to a (read-only copy of a) line in `buf`.
pub fn ml_get_buf(buf: &FileBuf, lnum: Linenr) -> crate::option_vars::RcCStr {
    ml_get_buf_impl(buf, lnum, false)
}

/// Return the bytes (excluding NUL) of a line in `curbuf`.
#[inline(always)]
pub fn ml_get_slice(lnum: Linenr) -> crate::option_vars::RcSlice {
    ml_get_buf_slice(&curbuf(), lnum)
}

/// Return the bytes (excluding NUL) of a line in `buf`.
#[inline(always)]
pub fn ml_get_buf_slice(buf: &FileBuf, lnum: Linenr) -> crate::option_vars::RcSlice {
    ml_get_buf(buf, lnum).into_bytes_slice()
}

/// Return the bytes (excluding NUL) starting at position `pos` in `curbuf`.
#[inline(always)]
pub fn ml_get_pos_slice(pos: &CursorPos) -> crate::option_vars::RcSlice {
    ml_get_buf_slice(&curbuf(), pos.lnum).wrapping_add(pos.col.max(0) as usize)
}

/// Mutate the byte slice (excluding NUL) of a line in `buf`, marking its data block dirty.
#[inline(always)]
pub fn with_ml_get_buf_mut_slice<R>(
    buf: &FileBuf,
    lnum: Linenr,
    f: impl FnOnce(&mut [u8]) -> R,
) -> R {
    if !ml_load_line(buf, lnum, true) || buf.b_ml.ml_line_textlen.get() <= 1 {
        return f(&mut []);
    }
    assert!(buf.b_ml.ml_line_textlen.get() > 0);
    let len = (buf.b_ml.ml_line_textlen.get() - 1) as usize;
    let (rv, new_textlen) = {
        let mut b = buf.b_ml.ml_line_buf.borrow_mut();
        let s = b.as_mut_slice();
        let slen = len.min(s.len());
        let rv = f(&mut s[..slen]);
        let actual_len = crate::memory::bytes_until_nul(&s[..slen]).len();
        (rv, (actual_len + 1) as Colnr)
    };
    buf.b_ml.ml_line_textlen.set(new_textlen);
    buf.b_ml.ml_line_cstr.set(None);
    rv
}

/// Return reference to CStr starting at position `pos`.
pub fn ml_get_pos(pos: &CursorPos) -> crate::option_vars::RcCStr {
    let line = ml_get_buf(&crate::globals::curbuf(), pos.lnum);
    line.slice(pos.col.max(0) as usize)
}

/// Return length (excluding NUL) of the given line.
pub fn ml_get_len(lnum: Linenr) -> Colnr {
    ml_get_buf_len(&crate::globals::curbuf(), lnum)
}

/// Return length (excluding NUL) of the text after position `pos`.
pub fn ml_get_pos_len(pos: &mut CursorPos) -> Colnr {
    ml_get_buf_len(&crate::globals::curbuf(), pos.lnum) - pos.col
}

/// Return length (excluding NUL) of the given line in `buf`.
pub fn ml_get_buf_len(buf: &FileBuf, lnum: Linenr) -> Colnr {
    if buf.b_ml.ml_line_lnum.get() == lnum {
        return (buf.b_ml.ml_line_textlen.get() - 1).max(0);
    }
    if lnum >= 1 && lnum <= buf.b_ml.ml_line_count.get() && buf.b_ml.ml_mfp.borrow().is_some() {
        let chunks = buf.b_ml.ml_chunks.borrow();
        if let Some(c) = buf.b_ml.find_chunk_idx(&chunks, lnum) {
            let ch = &chunks[c];
            if let Some(entry) = ch.lines.get((lnum - ch.start_lnum) as usize) {
                return entry.text.to_bytes().len() as Colnr;
            }
        }
    }
    if !ml_load_line(buf, lnum, false) {
        return 0;
    }
    (buf.b_ml.ml_line_textlen.get() - 1).max(0)
}

/// Gets the charwise NL-joined text of range `[start, end)` as a `Vec<u8>`.
pub fn ml_region_text(buf: &FileBuf, start: CursorPos, end: CursorPos) -> Vec<u8> {
    let mut sb = Vec::<u8>::new();
    for lnum in start.lnum..=end.lnum {
        with_ml_buf_line(buf, lnum, |line| {
            let len = line.len() as Colnr;
            let from = if lnum == start.lnum {
                start.col.min(len).max(0)
            } else {
                0
            };
            let to = if lnum == end.lnum {
                end.col.min(len).max(from)
            } else {
                len
            };
            sb.extend_from_slice(&line[from as usize..to as usize]);
        });
        if lnum < end.lnum {
            sb.push(NL as u8);
        }
    }
    sb
}

/// Return codepoint at `pos`.
pub fn gchar_pos(pos: &mut CursorPos) -> i32 {
    if pos.col == MAXCOL {
        return i32::from(NUL);
    }
    let buf = curbuf();
    let col = pos.col.max(0) as usize;
    with_ml_buf_line(&buf, pos.lnum, |line| {
        if pos.col > line.len() as Colnr {
            i32::from(NUL)
        } else {
            crate::mbyte::utf_slice2char(line.get(col..).unwrap_or(&[]))
        }
    })
}

fn ml_load_line(buf: &FileBuf, mut lnum: Linenr, will_change: bool) -> bool {
    if buf.b_ml.ml_mfp.borrow().is_none() {
        buf.b_ml.ml_line_textlen.set(1);
        buf.b_ml.ml_line_lnum.set(0);
        return false;
    }

    if lnum > buf.b_ml.ml_line_count.get() {
        if ml_st(|st| {
            let r = st.ml_get_recursive.get();
            if r == 0 {
                st.ml_get_recursive.set(1);
            }
            r == 0
        }) {
            siemsg!(tr(E_ML_GET_INVALID_LNUM_NR), i64::from(lnum));
            ml_st(|st| st.ml_get_recursive.set(st.ml_get_recursive.get() - 1));
        }
        ml_flush_line(buf, false);
        ml_set_errorret(buf, lnum);
        return true;
    }
    lnum = lnum.max(1);

    if buf.b_ml.ml_line_lnum.get() != lnum {
        ml_flush_line(buf, false);

        let cstr_opt = {
            let chunks = buf.b_ml.ml_chunks.borrow();
            buf.b_ml.find_chunk_idx(&chunks, lnum).and_then(|c| {
                let ch = &chunks[c];
                ch.lines
                    .get((lnum - ch.start_lnum) as usize)
                    .map(|e| e.text.clone())
            })
        };
        let Some(cstr) = cstr_opt else {
            if ml_st(|st| {
                let r = st.ml_get_recursive.get();
                if r == 0 {
                    st.ml_get_recursive.set(1);
                }
                r == 0
            }) {
                let mut name_buff = [0u8; crate::os::fs::MAXPATHL];
                get_trans_bufname(buf, &mut name_buff);
                shorten_dir(&mut name_buff);
                siemsg!(
                    tr(E_ML_GET_CANNOT_FIND_LINE_NR_IN_BUFFER_NR_STR),
                    i64::from(lnum),
                    buf.b_fnum.get(),
                    &name_buff,
                );
                ml_st(|st| st.ml_get_recursive.set(st.ml_get_recursive.get() - 1));
            }
            ml_set_errorret(buf, lnum);
            return true;
        };

        let len = cstr.to_bytes().len() + 1;
        buf.b_ml.ml_line_textlen.set(len as Colnr);
        buf.b_ml.ml_line_lnum.set(lnum);
        buf.b_ml.ml_line_cstr.set(Some(cstr));
        buf.b_ml
            .ml_flags
            .set(buf.b_ml.ml_flags.get() & (!(ML_LINE_DIRTY | ML_LINE_INPLACE)));
    }
    if will_change {
        let deleted_len = buf
            .b_ml
            .with_current_line_slice(|s| crate::memory::bytes_until_nul(s).len());
        if (buf.b_ml.ml_flags.get() & ML_LINE_DIRTY) == 0 {
            buf.b_ml.with_current_line_slice(|s| {
                let mut lb = buf.b_ml.ml_line_buf.borrow_mut();
                lb.clear();
                lb.extend_from_slice(s);
            });
        }
        buf.b_ml.ml_line_cstr.set(None);
        buf.b_ml
            .ml_flags
            .set(buf.b_ml.ml_flags.get() | (ML_LINE_DIRTY | ML_LINE_INPLACE));
        buf.b_ml.mark_swap_dirty();
        if edit_state().inhibit_delete_count.get() == 0 {
            buf.deleted_bytes
                .set(buf.deleted_bytes.get() + (deleted_len + 1));
            buf.deleted_bytes2
                .set(buf.deleted_bytes2.get() + (deleted_len + 1));
            if buf.update_need_codepoints.get() {
                let mut cp = buf.deleted_codepoints.get();
                let mut cu = buf.deleted_codeunits.get();
                buf.b_ml.with_current_line_slice(|s| {
                    mb_utflen(&s[..deleted_len], deleted_len, &mut cp, &mut cu);
                });
                buf.deleted_codepoints.set(cp + 1);
                buf.deleted_codeunits.set(cu + 1);
            }
        }
    }
    true
}

fn ml_get_buf_impl(buf: &FileBuf, lnum: Linenr, will_change: bool) -> crate::option_vars::RcCStr {
    if !ml_load_line(buf, lnum, will_change) {
        return crate::option_vars::RcCStr::EMPTY;
    }
    if !will_change && let Some(cstr) = buf.b_ml.ml_line_cstr.get() {
        return cstr;
    }

    let cstr = buf.b_ml.with_current_line_slice(|s| {
        let cs = if (buf.b_ml.ml_flags.get() & ML_LINE_INPLACE) != 0 {
            crate::memory::bytes_as_cstr(s)
        } else {
            std::ffi::CStr::from_bytes_with_nul(s)
                .unwrap_or_else(|_| crate::memory::bytes_as_cstr(s))
        };
        crate::option_vars::RcCStr::from_cstr(cs)
    });
    if !will_change {
        buf.b_ml.ml_line_cstr.set(Some(cstr.clone()));
    }
    cstr
}

fn ml_set_errorret(buf: &FileBuf, lnum: Linenr) {
    buf.b_ml.ml_line_textlen.set(4);
    buf.b_ml.ml_line_lnum.set(lnum);
    buf.b_ml
        .ml_line_cstr
        .set(Some(crate::option_vars::RcCStr::from_static(c"???")));
}

/// Check if a line that was just obtained by `ml_get` is in allocated memory.
pub fn ml_line_alloced() -> bool {
    (curbuf().b_ml.ml_flags.get() & ML_LINE_DIRTY) != 0
}

fn ml_append_int(buf: &FileBuf, lnum: Linenr, line: &[u8], len_arg: Colnr, flags: i32) -> bool {
    let text = if len_arg > 0
        && let Some(with_nul) = line.get(..len_arg as usize)
        && let Ok(cstr) = std::ffi::CStr::from_bytes_with_nul(with_nul)
    {
        crate::option_vars::RcCStr::from_cstr(cstr)
    } else {
        let slice = if len_arg > 0 {
            crate::memory::bytes_until_nul(&line[..((len_arg - 1) as usize).min(line.len())])
        } else {
            crate::memory::bytes_until_nul(line)
        };
        crate::option_vars::RcCStr::from_bytes(slice)
    };
    ml_append_cstr_int(buf, lnum, text, flags)
}

fn ml_append_cstr_int(
    buf: &FileBuf,
    lnum: Linenr,
    text: crate::option_vars::RcCStr,
    flags: i32,
) -> bool {
    let old_line_count = buf.b_ml.ml_line_count.get();
    if lnum < 0 || lnum > old_line_count || buf.b_ml.ml_mfp.borrow().is_none() {
        return false;
    }

    ml_st(|st| {
        if st.lowest_marked.get() != 0 && st.lowest_marked.get() > lnum {
            st.lowest_marked.set(lnum + 1);
        }
    });

    let len = (text.to_bytes().len() as i32) + 1;
    let entry = LineEntry {
        text,
        marked: (flags & ML_APPEND_MARK) != 0,
    };

    buf.b_ml.ml_flags.set(buf.b_ml.ml_flags.get() & (!ML_EMPTY));

    if buf.b_prev_line_count.get() == 0 {
        buf.b_prev_line_count.set(old_line_count);
    }
    buf.b_ml.ml_line_count.set(old_line_count + 1);

    {
        let mut chunks = buf.b_ml.ml_chunks.borrow_mut();
        if chunks.is_empty() {
            let mut first_lines = Vec::with_capacity(MLCS_MINL as usize);
            first_lines.push(entry);
            chunks.push(LineChunk {
                lines: first_lines,
                start_lnum: 1,
                totalsize: len,
                swap_blocks: Vec::new(),
            });
            buf.b_ml.ml_cur_chunk.set(0);
        } else if lnum == old_line_count {
            let last_idx = chunks.len() - 1;
            if chunks[last_idx].lines.len() >= MLCS_MINL as usize {
                let mut new_lines = Vec::with_capacity(MLCS_MINL as usize);
                new_lines.push(entry);
                chunks.push(LineChunk {
                    lines: new_lines,
                    start_lnum: old_line_count + 1,
                    totalsize: len,
                    swap_blocks: Vec::new(),
                });
                buf.b_ml.ml_cur_chunk.set(last_idx + 1);
            } else {
                chunks[last_idx].lines.push(entry);
                chunks[last_idx].totalsize += len;
                chunks[last_idx].swap_blocks.clear();
                buf.b_ml.ml_cur_chunk.set(last_idx);
            }
        } else {
            let (c, in_chunk_idx) = if lnum == 0 {
                (0usize, 0usize)
            } else {
                let c = buf.b_ml.find_chunk_idx(&chunks, lnum).unwrap_or(0);
                let in_idx = (lnum - chunks[c].start_lnum + 1) as usize;
                (c, in_idx)
            };
            chunks[c].lines.insert(in_chunk_idx, entry);
            chunks[c].totalsize += len;
            chunks[c].swap_blocks.clear();
            for ch in &mut chunks[(c + 1)..] {
                ch.start_lnum += 1;
            }
            if chunks[c].lines.len() >= MLCS_MAXL as usize {
                let right_lines = chunks[c].lines.split_off(MLCS_MINL as usize);
                let right_size: i32 = right_lines
                    .iter()
                    .map(|e| e.text.to_bytes().len() as i32 + 1)
                    .sum();
                chunks[c].totalsize -= right_size;
                let right_start = chunks[c].start_lnum + (MLCS_MINL as Linenr);
                chunks.insert(
                    c + 1,
                    LineChunk {
                        lines: right_lines,
                        start_lnum: right_start,
                        totalsize: right_size,
                        swap_blocks: Vec::new(),
                    },
                );
            }
            buf.b_ml.ml_cur_chunk.set(c);
        }
        buf.b_ml.ml_usedchunks.set(chunks.len() as i32);
    }

    buf.b_ml.mark_swap_dirty();
    true
}

fn ml_append_flush(buf: &FileBuf, lnum: Linenr, line: &[u8], len: Colnr, flags: i32) -> bool {
    if lnum > buf.b_ml.ml_line_count.get() {
        return false;
    }
    if buf.b_ml.ml_line_lnum.get() != 0 {
        ml_flush_line(buf, false);
    }
    ml_append_int(buf, lnum, line, len, flags)
}

/// Append a line after `lnum` (may be 0 to insert in front of the file).
pub fn ml_append(lnum: Linenr, line: &[u8], len: Colnr, newfile: bool) -> bool {
    ml_append_flags(lnum, line, len, if newfile { ML_APPEND_NEW } else { 0 })
}

/// Append a line after `lnum` using `ML_APPEND_` flags.
pub fn ml_append_flags(lnum: Linenr, line: &[u8], len: Colnr, flags: i32) -> bool {
    if curbuf().b_ml.ml_mfp.borrow().is_none() && !open_buffer(false, None, 0) {
        return false;
    }
    ml_append_flush(&curbuf(), lnum, line, len, flags)
}

/// Like `ml_append()` but for an arbitrary buffer.
pub fn ml_append_buf(buf: &FileBuf, lnum: Linenr, line: &[u8], len: Colnr, newfile: bool) -> bool {
    if buf.b_ml.ml_mfp.borrow().is_none() {
        return false;
    }
    ml_append_flush(
        buf,
        lnum,
        line,
        len,
        if newfile { ML_APPEND_NEW } else { 0 },
    )
}

/// Fast path to append an already-validated `RcCStr` line to `buf`.
pub fn ml_append_buf_cstr(
    buf: &FileBuf,
    lnum: Linenr,
    text: crate::option_vars::RcCStr,
    newfile: bool,
) -> bool {
    if buf.b_ml.ml_mfp.borrow().is_none() || lnum > buf.b_ml.ml_line_count.get() {
        return false;
    }
    if buf.b_ml.ml_line_lnum.get() != 0 {
        ml_flush_line(buf, false);
    }
    ml_append_cstr_int(buf, lnum, text, if newfile { ML_APPEND_NEW } else { 0 })
}

pub fn ml_add_deleted_len_buf(buf: &FileBuf, bytes: &[u8]) {
    if edit_state().inhibit_delete_count.get() != 0 {
        return;
    }
    let len = bytes.len();
    buf.deleted_bytes.set(buf.deleted_bytes.get() + (len + 1));
    buf.deleted_bytes2.set(buf.deleted_bytes2.get() + (len + 1));
    if buf.update_need_codepoints.get() {
        let mut del_cp = buf.deleted_codepoints.get();
        let mut del_cu = buf.deleted_codeunits.get();
        mb_utflen(bytes, len, &mut del_cp, &mut del_cu);
        buf.deleted_codepoints.set(del_cp + 1);
        buf.deleted_codeunits.set(del_cu + 1);
    }
}

/// Replace line `lnum`, with buffering, in current buffer.
pub fn ml_replace(lnum: Linenr, line: &[u8], copy: bool) -> bool {
    ml_replace_buf(&curbuf(), lnum, line, copy, false)
}

/// Replace a line for the current buffer with explicit `len` (excluding NUL).
pub fn ml_replace_len(lnum: Linenr, line: &[u8], len: usize, copy: bool) -> bool {
    ml_replace_buf_len(&curbuf(), lnum, line, len, copy, false)
}

pub fn ml_replace_buf(buf: &FileBuf, lnum: Linenr, line: &[u8], copy: bool, noalloc: bool) -> bool {
    let len = crate::memory::bytes_until_nul(line).len();
    ml_replace_buf_len(buf, lnum, line, len, copy, noalloc)
}

/// Replace line `lnum`, with buffering.
pub fn ml_replace_buf_len(
    buf: &FileBuf,
    lnum: Linenr,
    line_arg: &[u8],
    len_arg: usize,
    copy: bool,
    noalloc: bool,
) -> bool {
    if buf.b_ml.ml_mfp.borrow().is_none() && !open_buffer(false, None, 0) {
        return false;
    }

    if !copy {
        assert!(noalloc);
    }

    if buf.b_ml.ml_line_lnum.get() != lnum {
        ml_flush_line(buf, false);
    }

    if buf.has_update_callbacks() {
        with_ml_buf_line(buf, lnum, |old_line| {
            ml_add_deleted_len_buf(buf, old_line);
        });
    }

    if copy {
        let mut lb = buf.b_ml.ml_line_buf.borrow_mut();
        lb.clear();
        lb.reserve(len_arg + 1);
        let copy_len = len_arg.min(line_arg.len());
        if copy_len > 0 {
            lb.extend_from_slice(&line_arg[..copy_len]);
        }
        if len_arg > copy_len {
            lb.resize(len_arg, 0);
        }
        lb.push(0);
    } else {
        buf.b_ml.ml_line_buf.borrow_mut().clear();
    }

    buf.b_ml.ml_line_textlen.set((len_arg as Colnr) + 1);
    buf.b_ml.ml_line_lnum.set(lnum);
    buf.b_ml.ml_line_cstr.set(None);
    buf.b_ml
        .ml_flags
        .set((buf.b_ml.ml_flags.get() | ML_LINE_DIRTY) & !ML_EMPTY);
    if noalloc {
        ml_flush_line_impl(buf, Some(&line_arg[..len_arg.min(line_arg.len())]));
    }

    true
}

fn ml_update_chunk_line(buf: &FileBuf, lnum: Linenr, new_cstr: crate::option_vars::RcCStr) -> bool {
    let actual_new_len = (new_cstr.to_bytes().len() as i32) + 1;
    let updated = {
        let mut chunks = buf.b_ml.ml_chunks.borrow_mut();
        if let Some(c) = buf.b_ml.find_chunk_idx(&chunks, lnum) {
            let ch = &mut chunks[c];
            let in_idx = (lnum - ch.start_lnum) as usize;
            if let Some(entry) = ch.lines.get_mut(in_idx) {
                let old_len = (entry.text.to_bytes().len() as i32) + 1;
                entry.text = new_cstr;
                let extra = actual_new_len - old_len;
                ch.totalsize += extra;
                ch.swap_blocks.clear();
                if buf.b_ml.ml_usedchunks.get() == 0 {
                    buf.b_ml.ml_usedchunks.set(chunks.len() as i32);
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if !updated {
        siemsg!(tr(c"E320: Cannot find line %lld"), i64::from(lnum));
    }
    updated
}

/// Fast path to replace line `lnum` in `buf` with an already-validated `RcCStr`.
pub fn ml_replace_buf_cstr(
    buf: &FileBuf,
    lnum: Linenr,
    new_cstr: crate::option_vars::RcCStr,
) -> bool {
    if buf.b_ml.ml_mfp.borrow().is_none() && !open_buffer(false, None, 0) {
        return false;
    }

    if buf.b_ml.ml_line_lnum.get() != lnum {
        ml_flush_line(buf, false);
    }

    if buf.has_update_callbacks() {
        with_ml_buf_line(buf, lnum, |old_line| {
            ml_add_deleted_len_buf(buf, old_line);
        });
    }

    if buf.b_ml.ml_line_lnum.get() == lnum {
        buf.b_ml
            .ml_flags
            .set(buf.b_ml.ml_flags.get() & (!ML_LINE_DIRTY));
        buf.b_ml.ml_line_lnum.set(0);
        buf.b_ml.ml_line_cstr.set(None);
        buf.b_ml.ml_line_offset.set(0);
    }

    buf.flush_count.set(buf.flush_count.get() + 1);
    if !ml_update_chunk_line(buf, lnum, new_cstr) {
        return false;
    }
    buf.b_ml.ml_flags.set(buf.b_ml.ml_flags.get() & (!ML_EMPTY));
    buf.b_ml.mark_swap_dirty();
    true
}

/// Replace the in-memory dirty line buffer in `buf` with `bytes` (without trailing NUL).
pub fn ml_replace_dirty_line(buf: &FileBuf, bytes: &[u8], clear_empty: bool) {
    {
        let mut lb = buf.b_ml.ml_line_buf.borrow_mut();
        lb.clear();
        lb.reserve(bytes.len() + 1);
        lb.extend_from_slice(bytes);
        lb.push(0);
    }
    buf.b_ml.ml_line_textlen.set((bytes.len() as Colnr) + 1);
    buf.b_ml.ml_line_cstr.set(None);
    buf.b_ml
        .ml_flags
        .set(buf.b_ml.ml_flags.get() | (ML_LINE_DIRTY));
    if clear_empty {
        buf.b_ml.ml_flags.set(buf.b_ml.ml_flags.get() & (!ML_EMPTY));
    }
}

/// Delete line `lnum` in `buf`.
pub fn ml_delete_buf(buf: &FileBuf, lnum: Linenr, message: bool) -> bool {
    ml_flush_line(buf, false);
    ml_delete_int(buf, lnum, if message { ML_DEL_MESSAGE } else { 0 })
}

fn ml_delete_int(buf: &FileBuf, lnum: Linenr, flags: i32) -> bool {
    ml_st(|st| {
        let low = st.lowest_marked.get();
        if low != 0 && low > lnum {
            st.lowest_marked.set(low - 1);
        }
    });

    if buf.b_ml.ml_line_count.get() == 1 {
        if (flags & ML_DEL_MESSAGE) != 0 {
            set_keep_msg(Some(gettext(NO_LINES_MSG)), 0);
        }
        let i = ml_replace_buf(buf, 1, b"", true, false);
        buf.b_ml.ml_flags.set(buf.b_ml.ml_flags.get() | (ML_EMPTY));
        return i;
    }

    if buf.b_ml.mfp_ref().is_none() || lnum < 1 || lnum > buf.b_ml.ml_line_count.get() {
        return false;
    }

    if buf.b_prev_line_count.get() == 0 {
        buf.b_prev_line_count.set(buf.b_ml.ml_line_count.get());
    }
    buf.b_ml
        .ml_line_count
        .set(buf.b_ml.ml_line_count.get() - (1));

    let removed = {
        let mut chunks = buf.b_ml.ml_chunks.borrow_mut();
        let Some(c) = buf.b_ml.find_chunk_idx(&chunks, lnum) else {
            return false;
        };
        let in_chunk_idx = (lnum - chunks[c].start_lnum) as usize;
        let rem = chunks[c].lines.remove(in_chunk_idx);
        let line_size = (rem.text.to_bytes().len() + 1) as i32;
        chunks[c].totalsize -= line_size;
        chunks[c].swap_blocks.clear();
        for ch in &mut chunks[(c + 1)..] {
            ch.start_lnum -= 1;
        }

        if chunks[c].lines.is_empty() && chunks.len() > 1 {
            chunks.remove(c);
            if buf.b_ml.ml_cur_chunk.get() >= chunks.len() {
                buf.b_ml.ml_cur_chunk.set(chunks.len() - 1);
            }
        } else if c + 1 < chunks.len()
            && chunks[c].lines.len() + chunks[c + 1].lines.len() <= MLCS_MINL as usize
        {
            let right = chunks.remove(c + 1);
            chunks[c].totalsize += right.totalsize;
            chunks[c].lines.extend(right.lines);
            chunks[c].swap_blocks.clear();
            buf.b_ml.ml_cur_chunk.set(c);
        } else if c > 0 && chunks[c - 1].lines.len() + chunks[c].lines.len() <= MLCS_MINL as usize {
            let cur_ch = chunks.remove(c);
            chunks[c - 1].totalsize += cur_ch.totalsize;
            chunks[c - 1].lines.extend(cur_ch.lines);
            chunks[c - 1].swap_blocks.clear();
            buf.b_ml.ml_cur_chunk.set(c - 1);
        }
        if buf.b_ml.ml_usedchunks.get() > 0 {
            buf.b_ml.ml_usedchunks.set(chunks.len() as i32);
        }
        rem
    };

    let text_bytes = removed.text.to_bytes();
    let len = text_bytes.len();

    if edit_state().inhibit_delete_count.get() == 0 {
        buf.deleted_bytes.set(buf.deleted_bytes.get() + (len + 1));
        buf.deleted_bytes2.set(buf.deleted_bytes2.get() + (len + 1));
        if buf.update_need_codepoints.get() {
            let mut cp = buf.deleted_codepoints.get();
            let mut cu = buf.deleted_codeunits.get();
            mb_utflen(text_bytes, len, &mut cp, &mut cu);
            buf.deleted_codepoints.set(cp + 1);
            buf.deleted_codeunits.set(cu + 1);
        }
    }

    buf.b_ml.mark_swap_dirty();
    true
}

/// Delete line `lnum` in the current buffer.
pub fn ml_delete(lnum: Linenr) -> bool {
    ml_delete_flags(lnum, 0)
}

/// Like `ml_delete()` but using flags.
pub fn ml_delete_flags(lnum: Linenr, flags: i32) -> bool {
    ml_flush_line(&curbuf(), false);
    if lnum < 1 || lnum > curbuf().b_ml.ml_line_count.get() {
        return false;
    }
    ml_delete_int(&curbuf(), lnum, flags)
}

/// Set the `DB_MARKED` flag for line `lnum`.
pub fn ml_setmarked(lnum: Linenr) {
    let cb = curbuf();
    if lnum < 1 || lnum > cb.b_ml.ml_line_count.get() || cb.b_ml.ml_mfp.borrow().is_none() {
        return;
    }
    ml_st(|st| {
        let low = st.lowest_marked.get();
        if low == 0 || low > lnum {
            st.lowest_marked.set(lnum);
        }
    });

    {
        let mut chunks = cb.b_ml.ml_chunks.borrow_mut();
        if let Some(c) = cb.b_ml.find_chunk_idx(&chunks, lnum) {
            let ch = &mut chunks[c];
            if let Some(entry) = ch.lines.get_mut((lnum - ch.start_lnum) as usize) {
                entry.marked = true;
            }
        }
    }
}

/// Find the first line with its `DB_MARKED` flag set.
pub fn ml_firstmarked() -> Linenr {
    let lnum = ml_st(|st| st.lowest_marked.get());
    let cb = curbuf();
    if cb.b_ml.ml_mfp.borrow().is_none() || lnum == 0 {
        return 0;
    }

    let found_lnum = {
        let mut chunks = cb.b_ml.ml_chunks.borrow_mut();
        let start_c = cb.b_ml.find_chunk_idx(&chunks, lnum).unwrap_or(0);
        let mut found = 0;
        'outer: for (c_off, ch) in chunks.get_mut(start_c..).into_iter().flatten().enumerate() {
            let line_start_idx = if c_off == 0 && lnum > ch.start_lnum {
                (lnum - ch.start_lnum) as usize
            } else {
                0
            };
            for (idx, entry) in ch
                .lines
                .get_mut(line_start_idx..)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if entry.marked {
                    entry.marked = false;
                    found = ch.start_lnum + ((line_start_idx + idx) as Linenr);
                    cb.b_ml.ml_cur_chunk.set(start_c + c_off);
                    break 'outer;
                }
            }
        }
        found
    };

    if found_lnum != 0 {
        ml_st(|st| st.lowest_marked.set(found_lnum + 1));
        found_lnum
    } else {
        ml_st(|st| st.lowest_marked.set(0));
        0
    }
}

/// Clear all `DB_MARKED` flags.
pub fn ml_clearmarked() {
    let lnum = ml_st(|st| st.lowest_marked.get());
    let cb = curbuf();
    if cb.b_ml.ml_mfp.borrow().is_none() || lnum == 0 {
        return;
    }

    {
        let mut chunks = cb.b_ml.ml_chunks.borrow_mut();
        let start_c = cb.b_ml.find_chunk_idx(&chunks, lnum).unwrap_or(0);
        for ch in chunks.get_mut(start_c..).into_iter().flatten() {
            for entry in &mut ch.lines {
                entry.marked = false;
            }
        }
    }
    ml_st(|st| st.lowest_marked.set(0));
}

pub fn ml_try_mark_global_lines(
    buf: &FileBuf,
    line1: Linenr,
    line2: Linenr,
    regmatch: &crate::regexp::Regmmatch,
    invert: bool,
) -> Option<i32> {
    if line1 > line2 || line1 < 1 || input_state().got_int.get() {
        return Some(0);
    }
    if regmatch.regprog.re_in_use() || crate::regexp::rex_in_use() {
        return None;
    }
    if (buf.b_ml.ml_flags.get() & ML_LINE_DIRTY) != 0 {
        ml_flush_line(buf, false);
    }
    if buf.b_ml.ml_mfp.borrow().is_none() {
        return Some(0);
    }
    let rmm_ic = regmatch.rmm_ic;
    let rmm_maxcol = regmatch.rmm_maxcol;

    crate::regexp::with_pure_nfa_prog(&regmatch.regprog, |nfa| {
        let mut chunks = buf.b_ml.ml_chunks.borrow_mut();
        let Some(start_c) = buf.b_ml.find_chunk_idx(&chunks, line1) else {
            return Some(0);
        };
        let Some(end_c) = buf.b_ml.find_chunk_idx(&chunks, line2) else {
            return Some(0);
        };
        let active_chunks = &mut chunks[start_c..=end_c];
        let total_lines = (line2 - line1 + 1) as usize;

        fn scan_slice(
            slice: &mut [LineChunk],
            nfa: &crate::regexp::NfaRegprog,
            line1: Linenr,
            line2: Linenr,
            rmm_ic: bool,
            rmm_maxcol: Colnr,
            invert: bool,
        ) -> Result<(i32, Linenr), ()> {
            let mut ndone: i32 = 0;
            let mut lowest: Linenr = 0;
            if rmm_maxcol == 0
                && let Some((regstart, mt, reg_ic)) = nfa.fast_literal_params(rmm_ic)
            {
                for ch in slice {
                    let ch_start = ch.start_lnum;
                    for (idx, entry) in ch.lines.iter_mut().enumerate() {
                        let lnum = ch_start + idx as Linenr;
                        if lnum < line1 || lnum > line2 {
                            continue;
                        }
                        let line_bytes = entry.text.to_bytes();
                        let matched = if let Some(off) =
                            crate::regexp::cstrchr_ic(line_bytes, regstart, reg_ic)
                        {
                            let mut mcol = off as Colnr;
                            crate::regexp::find_match_text_in_bytes(
                                line_bytes, &mut mcol, regstart, mt, reg_ic,
                            )
                            .is_some()
                        } else {
                            false
                        };
                        if matched != invert {
                            entry.marked = true;
                            ndone += 1;
                            if lowest == 0 {
                                lowest = lnum;
                            }
                        }
                    }
                }
                return Ok((ndone, lowest));
            }
            for ch in slice {
                let ch_start = ch.start_lnum;
                for (idx, entry) in ch.lines.iter_mut().enumerate() {
                    let lnum = ch_start + idx as Linenr;
                    if lnum < line1 || lnum > line2 {
                        continue;
                    }
                    let matched = crate::regexp::nfa_match_line_slice(
                        nfa,
                        entry.text.to_bytes_with_nul(),
                        0,
                        rmm_ic,
                        rmm_maxcol,
                    )?
                    .is_some();
                    if matched != invert {
                        entry.marked = true;
                        ndone += 1;
                        if lowest == 0 {
                            lowest = lnum;
                        }
                    }
                }
            }
            Ok((ndone, lowest))
        }

        let use_parallel = active_chunks.len() >= 4
            && if rmm_maxcol == 0 && nfa.fast_literal_params(rmm_ic).is_some() {
                total_lines >= 65536
            } else {
                total_lines >= 8192
            };

        let res = if use_parallel {
            let b_chartab = buf.b_chartab.get();
            let (isf_chartab, isi_chartab) = crate::charset::get_global_chartabs();
            let p_mmp = opt_vars().p_mmp.get();
            let nthreads = std::thread::available_parallelism()
                .map_or(4, |n| n.get().min(8))
                .min(active_chunks.len());
            let group_len = active_chunks.len().div_ceil(nthreads).max(1);
            std::thread::scope(|s| {
                let mut handles = Vec::with_capacity(nthreads);
                for worker_slice in active_chunks.chunks_mut(group_len) {
                    handles.push(s.spawn(move || {
                        crate::regexp::init_worker_regex_env(
                            b_chartab,
                            isf_chartab,
                            isi_chartab,
                            p_mmp,
                        );
                        scan_slice(worker_slice, nfa, line1, line2, rmm_ic, rmm_maxcol, invert)
                    }));
                }
                let mut total_ndone = 0i32;
                let mut min_lowest = 0;
                let mut any_err = false;
                for h in handles {
                    match h.join() {
                        Ok(Ok((n, l))) => {
                            total_ndone += n;
                            if l != 0 && (min_lowest == 0 || l < min_lowest) {
                                min_lowest = l;
                            }
                        }
                        _ => any_err = true,
                    }
                }
                if any_err {
                    Err(())
                } else {
                    Ok((total_ndone, min_lowest))
                }
            })
        } else {
            crate::regexp::init_main_regex_buf(buf);
            scan_slice(active_chunks, nfa, line1, line2, rmm_ic, rmm_maxcol, invert)
        };

        match res {
            Ok((ndone, lowest)) => {
                if lowest != 0 {
                    ml_st(|st| {
                        let low = st.lowest_marked.get();
                        if low == 0 || low > lowest {
                            st.lowest_marked.set(lowest);
                        }
                    });
                }
                Some(ndone)
            }
            Err(()) => {
                for ch in active_chunks {
                    for entry in &mut ch.lines {
                        entry.marked = false;
                    }
                }
                None
            }
        }
    })
    .flatten()
}

pub struct VgrFastMatch {
    pub lnum: Linenr,
    pub end_lnum: Linenr,
    pub col: Colnr,
    pub end_col: Colnr,
    pub text: RcCStr,
}

pub fn ml_try_vimgrep_lines(
    buf: &FileBuf,
    regmatch: &crate::regexp::Regmmatch,
    spat_bytes: &[u8],
    is_fuzzy: bool,
    is_global: bool,
    tomatch: usize,
) -> Option<Vec<VgrFastMatch>> {
    if tomatch == 0 || input_state().got_int.get() {
        return Some(Vec::new());
    }
    if !is_fuzzy && (regmatch.regprog.re_in_use() || crate::regexp::rex_in_use()) {
        return None;
    }
    if (buf.b_ml.ml_flags.get() & ML_LINE_DIRTY) != 0 {
        ml_flush_line(buf, false);
    }
    let total_lines = buf.b_ml.ml_line_count.get() as usize;
    if total_lines == 0 {
        return Some(Vec::new());
    }
    let b_chartab = buf.b_chartab.get();

    if is_fuzzy {
        let chunks = buf.b_ml.ml_chunks.borrow();
        let pat_len = spat_bytes.len().min(crate::fuzzy::FUZZY_MATCH_MAX_LEN);
        if pat_len == 0 {
            return None;
        }
        let scan_fuzzy = |slice: &[LineChunk], limit: usize| -> Vec<VgrFastMatch> {
            let mut out = Vec::new();
            let mut score: i32 = 0;
            let mut matches = [0u32; crate::fuzzy::FUZZY_MATCH_MAX_LEN];
            'outer: for ch in slice {
                let ch_start = ch.start_lnum;
                for (idx, entry) in ch.lines.iter().enumerate() {
                    let lnum = ch_start + idx as Linenr;
                    let str_bytes = entry.text.to_bytes();
                    let linelen = str_bytes.len() as Colnr;
                    let mut col: Colnr = 0;
                    while crate::fuzzy::fuzzy_match(
                        &str_bytes[(col as usize).min(str_bytes.len())..],
                        spat_bytes,
                        false,
                        &mut score,
                        &mut matches,
                    ) {
                        out.push(VgrFastMatch {
                            lnum,
                            end_lnum: 0,
                            col: matches[0] as Colnr + col + 1,
                            end_col: 0,
                            text: entry.text.clone(),
                        });
                        if out.len() >= limit {
                            break 'outer;
                        }
                        if !is_global {
                            break;
                        }
                        col += matches[pat_len - 1] as Colnr + 1;
                        if col >= linelen {
                            break;
                        }
                    }
                }
            }
            out
        };

        if chunks.len() >= 4 && total_lines >= 4096 && tomatch >= 512 {
            let nthreads = std::thread::available_parallelism()
                .map_or(4, |n| n.get().min(8))
                .min(chunks.len());
            let group_len = chunks.len().div_ceil(nthreads).max(1);
            let mut all = std::thread::scope(|s| {
                let mut handles = Vec::with_capacity(nthreads);
                for worker_slice in chunks.chunks(group_len) {
                    handles.push(s.spawn(move || {
                        crate::fuzzy::init_worker_fuzzy_chartab(b_chartab);
                        scan_fuzzy(worker_slice, tomatch)
                    }));
                }
                let mut combined = Vec::new();
                for h in handles {
                    if let Ok(mut part) = h.join() {
                        let rem = tomatch.saturating_sub(combined.len());
                        if part.len() > rem {
                            part.truncate(rem);
                        }
                        combined.append(&mut part);
                        if combined.len() >= tomatch {
                            break;
                        }
                    }
                }
                combined
            });
            all.truncate(tomatch);
            return Some(all);
        }
        crate::fuzzy::init_worker_fuzzy_chartab(b_chartab);
        let res = scan_fuzzy(&chunks, tomatch);
        crate::fuzzy::clear_worker_fuzzy_chartab();
        return Some(res);
    }

    let rmm_ic = regmatch.rmm_ic;
    let rmm_maxcol = regmatch.rmm_maxcol;

    crate::regexp::with_pure_nfa_prog(&regmatch.regprog, |nfa| {
        let chunks = buf.b_ml.ml_chunks.borrow();
        let scan_nfa = |slice: &[LineChunk], limit: usize| -> Result<Vec<VgrFastMatch>, ()> {
            let mut out = Vec::new();
            if rmm_maxcol == 0
                && let Some((regstart, mt, reg_ic)) = nfa.fast_literal_params(rmm_ic)
            {
                'outer_lit: for ch in slice {
                    let ch_start = ch.start_lnum;
                    for (idx, entry) in ch.lines.iter().enumerate() {
                        let lnum = ch_start + idx as Linenr;
                        let line_bytes = entry.text.to_bytes();
                        let linelen = line_bytes.len() as Colnr;
                        let mut col: Colnr = 0;
                        loop {
                            let start_off = (col as usize).min(line_bytes.len());
                            let Some(off) = crate::regexp::cstrchr_ic(
                                &line_bytes[start_off..],
                                regstart,
                                reg_ic,
                            ) else {
                                break;
                            };
                            let mut mcol = col + off as Colnr;
                            let Some(e_col) = crate::regexp::find_match_text_in_bytes(
                                line_bytes, &mut mcol, regstart, mt, reg_ic,
                            ) else {
                                break;
                            };
                            out.push(VgrFastMatch {
                                lnum,
                                end_lnum: lnum,
                                col: mcol + 1,
                                end_col: e_col + 1,
                                text: entry.text.clone(),
                            });
                            if out.len() >= limit {
                                break 'outer_lit;
                            }
                            if !is_global {
                                break;
                            }
                            col = e_col + Colnr::from(col == e_col);
                            if col > linelen {
                                break;
                            }
                        }
                    }
                }
                return Ok(out);
            }
            'outer: for ch in slice {
                let ch_start = ch.start_lnum;
                for (idx, entry) in ch.lines.iter().enumerate() {
                    let lnum = ch_start + idx as Linenr;
                    let str_bytes_with_nul = entry.text.to_bytes_with_nul();
                    let linelen = str_bytes_with_nul.len().saturating_sub(1) as Colnr;
                    let mut col: Colnr = 0;
                    while let Some((s_col, e_col)) = crate::regexp::nfa_match_line_slice(
                        nfa,
                        str_bytes_with_nul,
                        col,
                        rmm_ic,
                        rmm_maxcol,
                    )? {
                        out.push(VgrFastMatch {
                            lnum,
                            end_lnum: lnum,
                            col: s_col + 1,
                            end_col: e_col + 1,
                            text: entry.text.clone(),
                        });
                        if out.len() >= limit {
                            break 'outer;
                        }
                        if !is_global {
                            break;
                        }
                        col = e_col + Colnr::from(col == e_col);
                        if col > linelen {
                            break;
                        }
                    }
                }
            }
            Ok(out)
        };

        let use_parallel = chunks.len() >= 4
            && tomatch >= 512
            && if rmm_maxcol == 0 && nfa.fast_literal_params(rmm_ic).is_some() {
                total_lines >= 65536
            } else {
                total_lines >= 8192
            };

        if use_parallel {
            let (isf_chartab, isi_chartab) = crate::charset::get_global_chartabs();
            let p_mmp = opt_vars().p_mmp.get();
            let nthreads = std::thread::available_parallelism()
                .map_or(4, |n| n.get().min(8))
                .min(chunks.len());
            let group_len = chunks.len().div_ceil(nthreads).max(1);
            std::thread::scope(|s| {
                let mut handles = Vec::with_capacity(nthreads);
                for worker_slice in chunks.chunks(group_len) {
                    handles.push(s.spawn(move || {
                        crate::regexp::init_worker_regex_env(
                            b_chartab,
                            isf_chartab,
                            isi_chartab,
                            p_mmp,
                        );
                        scan_nfa(worker_slice, tomatch)
                    }));
                }
                let mut combined = Vec::new();
                let mut any_err = false;
                for h in handles {
                    match h.join() {
                        Ok(Ok(mut part)) => {
                            let rem = tomatch.saturating_sub(combined.len());
                            if part.len() > rem {
                                part.truncate(rem);
                            }
                            combined.append(&mut part);
                        }
                        _ => any_err = true,
                    }
                }
                if any_err {
                    None
                } else {
                    combined.truncate(tomatch);
                    Some(combined)
                }
            })
        } else {
            crate::regexp::init_main_regex_buf(buf);
            scan_nfa(&chunks, tomatch).ok()
        }
    })
    .flatten()
}

pub fn ml_flush_deleted_bytes(
    buf: &FileBuf,
    codepoints: &mut usize,
    codeunits: &mut usize,
) -> usize {
    let ret = buf.deleted_bytes.get();
    *codepoints = buf.deleted_codepoints.get();
    *codeunits = buf.deleted_codeunits.get();
    buf.deleted_bytes.set(0);
    buf.deleted_codepoints.set(0);
    buf.deleted_codeunits.set(0);
    ret
}

fn ml_flush_line(buf: &FileBuf, _noalloc: bool) {
    ml_flush_line_impl(buf, None);
}

fn ml_flush_line_impl(buf: &FileBuf, override_line: Option<&[u8]>) {
    if buf.b_ml.ml_line_lnum.get() == 0 || buf.b_ml.ml_mfp.borrow().is_none() {
        return;
    }
    if (buf.b_ml.ml_flags.get() & ML_LINE_DIRTY) != 0 {
        if ml_st(|st| st.ml_flush_entered.replace(true)) {
            return;
        }

        buf.flush_count.set(buf.flush_count.get() + (1));

        let lnum = buf.b_ml.ml_line_lnum.get();
        let new_len = buf.b_ml.ml_line_textlen.get();
        let text_len = (new_len - 1).max(0) as usize;
        let new_cstr = if let Some(over) = override_line {
            let slice = crate::memory::bytes_until_nul(&over[..text_len.min(over.len())]);
            crate::option_vars::RcCStr::from_bytes(slice)
        } else {
            let lb = buf.b_ml.ml_line_buf.borrow();
            if let Some(with_nul) = lb.get(..=text_len)
                && let Ok(cstr) = std::ffi::CStr::from_bytes_with_nul(with_nul)
            {
                crate::option_vars::RcCStr::from_cstr(cstr)
            } else {
                let slice = crate::memory::bytes_until_nul(&lb[..text_len.min(lb.len())]);
                crate::option_vars::RcCStr::from_bytes(slice)
            }
        };

        if !ml_update_chunk_line(buf, lnum, new_cstr) {
            ml_st(|st| st.ml_flush_entered.set(false));
            buf.b_ml
                .ml_flags
                .set(buf.b_ml.ml_flags.get() & (!ML_LINE_DIRTY));
            buf.b_ml.ml_line_lnum.set(0);
            buf.b_ml.ml_line_cstr.set(None);
            buf.b_ml.ml_line_offset.set(0);
            return;
        }

        buf.b_ml.mark_swap_dirty();

        ml_st(|st| st.ml_flush_entered.set(false));
    }

    buf.b_ml
        .ml_flags
        .set(buf.b_ml.ml_flags.get() & (!ML_LINE_DIRTY));
    buf.b_ml.ml_line_lnum.set(0);
    buf.b_ml.ml_line_cstr.set(None);
    buf.b_ml.ml_line_offset.set(0);
}

fn ml_new_data(mfp: &mut Memfile, negative: bool, page_count: i64) -> BhdrId {
    assert!(page_count >= 0);
    let page_size = mfp.mf_page_size;
    let hp = mf_new(mfp, negative, page_count as u32);
    let hp_ref = mfp.bhdr_mut(hp);
    hp_ref.set_db_id(DATA_ID);
    let txt_end = (page_count as u32).wrapping_mul(page_size);
    hp_ref.set_db_txt_end(txt_end);
    hp_ref.set_db_txt_start(txt_end);
    hp_ref.set_db_free(txt_end - (HEADER_SIZE as u32));
    hp_ref.set_db_line_count(0);
    hp
}

fn ml_new_ptr(mfp: &mut Memfile) -> BhdrId {
    let max_cnt = pb_count_max(mfp);
    let hp = mf_new(mfp, false, 1);
    let hp_ref = mfp.bhdr_mut(hp);
    hp_ref.set_pb_id(PTR_ID);
    hp_ref.set_pb_count(0);
    hp_ref.set_pb_count_max(max_cnt);
    hp
}

/// Resolve a symlink in the last component of a file name.
#[cfg(unix)]
pub fn resolve_symlink(fname: Option<&CStr>, buf: &mut [u8]) -> bool {
    let mut tmp = [0u8; MAXPATHL];
    let mut depth: i32 = 0;

    let Some(fname) = fname else {
        return false;
    };

    crate::memory::copy_bytes_to_c_buf(&mut tmp, fname.to_bytes());

    loop {
        depth += 1;
        if depth == 100 {
            semsg!(tr(c"E773: Symlink loop for \"%s\""), fname);
            return false;
        }

        let buf_cap = buf.len().saturating_sub(1);
        let ret = os_readlink(crate::memory::bytes_as_cstr(&tmp), &mut buf[..buf_cap]);
        if ret <= 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == EINVAL || err == ENOENT {
                if depth == 1 {
                    return false;
                }
                break;
            }
            return false;
        }
        buf[ret as usize] = 0;

        let buf_bytes = &buf[..ret as usize];
        if path_is_absolute(buf_bytes) {
            crate::memory::copy_bytes_to_c_buf(&mut tmp, buf_bytes);
        } else {
            let tmp_bytes = crate::memory::bytes_until_nul(&tmp);
            let tail_len = path_tail(tmp_bytes).len();
            let off = tmp_bytes.len() - tail_len;
            if tail_len + buf_bytes.len() >= MAXPATHL {
                return false;
            }
            crate::memory::copy_bytes_to_c_buf(&mut tmp[off..], buf_bytes);
        }
    }

    vim_full_name(Some(crate::memory::bytes_until_nul(&tmp)), buf, true)
}

/// Make swapfile name out of the file name and a directory name.
pub fn makeswapname(fname: Option<&CStr>, dir_name: &CStr) -> Option<std::ffi::CString> {
    #[cfg(unix)]
    let mut fname_buf = [0u8; MAXPATHL];
    #[cfg(unix)]
    let fname_res: Option<&CStr> = if resolve_symlink(fname, &mut fname_buf) {
        Some(crate::memory::bytes_as_cstr(&fname_buf))
    } else {
        fname
    };
    #[cfg(not(unix))]
    let fname_res: Option<&CStr> = fname;

    let dir_bytes = dir_name.to_bytes();
    let len = dir_bytes.len();
    if after_pathsep(dir_bytes) && len > 1 && dir_bytes[len - 1] == dir_bytes[len - 2] {
        let sw = make_percent_swname(&dir_bytes[..len - 1], fname_res)?;
        return modname(Some(&sw), c".swp", false);
    }

    let r = modname(fname_res, c".swp", dir_bytes == b".")?;

    get_file_in_dir(Some(&r), dir_name)
}

/// Get file name to use for swapfile or backup file.
pub fn get_file_in_dir(fname: Option<&CStr>, dname: &CStr) -> Option<std::ffi::CString> {
    let fname = fname.unwrap_or(c"");
    let fname_bytes = fname.to_bytes();
    let tail_bytes = path_tail(fname_bytes);
    let tail = crate::api::private::helpers::bytes_to_string(tail_bytes);
    let tail_off = fname_bytes.len() - tail_bytes.len();
    let dname_bytes = dname.to_bytes();

    if dname_bytes == b"." {
        return Some(fname.to_owned());
    }

    let dname_len = dname_bytes.len();
    let retval = if dname_bytes.first() == Some(&b'.')
        && dname_bytes.get(1).is_some_and(|&b| vim_ispathsep(b))
    {
        if tail_off == 0 {
            concat_fnames(
                NvimString::from_bytes_len(&dname.to_bytes_with_nul()[2..], dname_len - 2),
                tail,
                true,
            )
        } else {
            let tmp = concat_fnames(
                NvimString::from_bytes_len(fname.to_bytes_with_nul(), tail_off),
                NvimString::from_bytes_len(&dname.to_bytes_with_nul()[2..], dname_len - 2),
                true,
            );
            concat_fnames(cstr_as_string(Some(&tmp)), tail, true)
        }
    } else {
        concat_fnames(NvimString::from_cstr(dname), tail, true)
    };

    Some(retval)
}

fn attention_message(buf: &FileBuf, fname: &CStr, fhname: &CStr, msg: &mut Vec<u8>) {
    assert!(buf.fname_opt().is_some());
    let buf_fname_cstr = buf.fname_cstr();

    emsg(tr(c"E325: ATTENTION"));
    sb_push_cstr(msg, tr(c"Found a swap file by the name \""));
    sb_push_cstr(msg, fhname);
    msg.extend_from_slice(b"\"\n");
    let swap_mtime = swapfile_info(fname, msg);
    sb_push_cstr(msg, tr(c"While opening file \""));
    sb_push_cstr(msg, &buf_fname_cstr);
    msg.extend_from_slice(b"\"\n");
    let mut file_info = FileInfo::new();
    if !os_fileinfo(&buf_fname_cstr, &mut file_info) {
        sb_push_cstr(msg, tr(c"      CANNOT BE FOUND"));
    } else {
        sb_push_cstr(msg, tr(c"             dated: "));
        let x: time_t = file_info.stat.st_mtim.tv_sec as _;
        let mut ctime_buf = [0u8; 50];
        let ct = os_ctime_r(&x, &mut ctime_buf, true);
        sb_push_cstr(msg, ct);
        if swap_mtime != 0 && x > swap_mtime {
            sb_push_cstr(msg, tr(c"      NEWER than swap file!\n"));
        }
    }
    sb_push_cstr(
        msg,
        tr(
            c"\n(1) Another program may be editing the same file.  If this is the case,\n    be careful not to end up with two different instances of the same\n    file when making changes.  Quit, or continue with caution.\n",
        ),
    );
    sb_push_cstr(msg, tr(c"(2) An edit session for this file crashed.\n"));
    sb_push_cstr(
        msg,
        tr(c"    If this is the case, use \":recover\" or \"nvim -r "),
    );
    sb_push_cstr(msg, &buf_fname_cstr);
    sb_push_cstr(
        msg,
        tr(c"\"\n    to recover the changes (see \":help recovery\").\n"),
    );
    sb_push_cstr(
        msg,
        tr(c"    If you did this already, delete the swap file \""),
    );
    sb_push_cstr(msg, fname);
    sb_push_cstr(msg, tr(c"\"\n    to avoid this message.\n"));
}

fn do_swapexists(buf: &FileBuf, fname: &CStr) -> SeaChoice {
    set_vim_var_string(VV_SWAPNAME, Some(fname), -1);
    set_vim_var_string(VV_SWAPCHOICE, None, -1);

    {
        let _allbuf_lock = input_state().lock_allbuf();
        apply_autocmds(
            EVENT_SWAPEXISTS,
            buf.fname_opt().as_deref(),
            None,
            false,
            None,
        );
    }

    set_vim_var_string(VV_SWAPNAME, None, -1);

    match get_vim_var_str(VV_SWAPCHOICE)
        .to_bytes()
        .first()
        .copied()
        .unwrap_or(0)
    {
        b'o' => SEA_CHOICE_READONLY,
        b'e' => SEA_CHOICE_EDIT,
        b'r' => SEA_CHOICE_RECOVER,
        b'd' => SEA_CHOICE_DELETE,
        b'q' => SEA_CHOICE_QUIT,
        b'a' => SEA_CHOICE_ABORT,
        _ => SEA_CHOICE_NONE,
    }
}

fn findswapname(
    buf: &FileBuf,
    dirp: &mut &[u8],
    old_fname: Option<&CStr>,
    found_existing_dir: &mut bool,
) -> Option<std::ffi::CString> {
    let buf_fname = buf.fname_opt().as_deref().map(CStr::to_owned);
    let dir_len = dirp.len() + 1;
    let mut dir_name_vec: Vec<u8> = vec![0; dir_len];
    copy_option_part(dirp, &mut dir_name_vec, c",");
    let dir_name = crate::memory::bytes_as_cstr(&dir_name_vec);

    let mut fname_vec: Option<Vec<u8>> =
        makeswapname(buf_fname.as_deref(), dir_name).map(std::ffi::CString::into_bytes_with_nul);

    while let Some(ref mut fvec) = fname_vec {
        let fname_cstr = crate::memory::bytes_as_cstr(fvec);
        let n = fname_cstr.to_bytes().len();
        if n == 0 {
            fname_vec = None;
            break;
        }
        let mut file_info = FileInfo::new();
        let file_or_link_found = os_fileinfo_link(fname_cstr, &mut file_info);
        if !file_or_link_found {
            break;
        }

        if let Some(old_f) = old_fname
            && path_equal(fname_cstr.to_bytes(), old_f.to_bytes(), K_PATH_CMP_LITERAL)
        {
            break;
        }

        let fname_slice = &fvec[..n];
        if fname_slice.ends_with(b"wp")
            && !exc_state().recoverymode.get()
            && buf_fname.is_some()
            && !buf.b_help.get()
            && (buf.b_flags.get() & BF_DUMMY) == 0
        {
            let mut differ = false;

            let fd = os_open(fname_cstr, O_RDONLY, 0);
            if fd >= 0 {
                #[allow(unused_mut)]
                if let Some(mut b0) = ZeroBlock::read_from_fd(fd) {
                    #[cfg(windows)]
                    path_to_slash(&mut b0.b0_fname);
                    let running = swapfile_proc_running(&b0, fname_cstr);
                    ml_st(|st| st.proc_running.set(running));

                    let b0_fname_cstr = crate::memory::bytes_as_cstr(&b0.b0_fname);
                    let buf_ffname_opt = buf.ffname_opt();
                    if (b0.flags() & B0_SAME_DIR) != 0 {
                        if !path_equal(
                            path_tail(buf_ffname_opt.as_deref().map_or(b"", CStr::to_bytes)),
                            path_tail(b0_fname_cstr.to_bytes()),
                            K_PATH_CMP_LITERAL,
                        ) || !same_directory(
                            Some(fname_cstr.to_bytes()),
                            buf_ffname_opt.as_deref().map(CStr::to_bytes),
                        ) {
                            let mut name_buff = [0u8; crate::os::fs::MAXPATHL];
                            expand_env(b0_fname_cstr, &mut name_buff);
                            if fnamecmp_ino(
                                buf_ffname_opt.as_deref(),
                                Some(crate::memory::bytes_as_cstr(&name_buff)),
                                char_to_long(&b0.b0_ino),
                            ) {
                                differ = true;
                            }
                        }
                    } else {
                        let mut name_buff = [0u8; crate::os::fs::MAXPATHL];
                        expand_env(b0_fname_cstr, &mut name_buff);
                        if fnamecmp_ino(
                            buf_ffname_opt.as_deref(),
                            Some(crate::memory::bytes_as_cstr(&name_buff)),
                            char_to_long(&b0.b0_ino),
                        ) {
                            differ = true;
                        }
                    }
                }
                os_close(fd);
            }

            if !differ
                && (curbuf().b_flags.get() & BF_RECOVERED) == 0
                && !crate::strings::vim_haschr(&opt_vars().p_shm.as_cstr(), K_SHM_ATTENTION)
            {
                let mut choice: SeaChoice = SEA_CHOICE_NONE;

                if os_path_exists(&buf.fname_cstr()) && swapfile_unchanged(fname_cstr) {
                    choice = SEA_CHOICE_DELETE;
                    if opt_vars().p_verbose.get() > 0 {
                        verb_msg(tr(c"Found a swap file that is not useful, deleting it"));
                    }
                }

                if choice == SEA_CHOICE_NONE
                    && workspace_state().swap_exists_action.get() != SEA_NONE
                    && has_autocmd(EVENT_SWAPEXISTS, buf_fname.as_deref(), Some(buf))
                {
                    choice = do_swapexists(buf, fname_cstr);
                }

                if choice == SEA_CHOICE_NONE
                    && workspace_state().swap_exists_action.get() == SEA_READONLY
                {
                    choice = SEA_CHOICE_READONLY;
                }

                ml_st(|st| st.proc_running.set(0));
                if choice == SEA_CHOICE_NONE {
                    msg_state()
                        .no_wait_return
                        .set(msg_state().no_wait_return.get() + (1));
                    let mut msg_sb = Vec::<u8>::with_capacity(IOSIZE);
                    let fhname = home_replace_save(None, Some(fname_cstr));
                    attention_message(buf, fname_cstr, &fhname, &mut msg_sb);

                    input_state().got_int.set(false);
                    flush_buffers(FLUSH_TYPEAHEAD);

                    if workspace_state().swap_exists_action.get() != SEA_NONE {
                        sb_push_cstr(&mut msg_sb, tr(c"Swap file \""));
                        sb_push_cstr(&mut msg_sb, &fhname);
                        sb_push_cstr(&mut msg_sb, tr(c"\" already exists!"));
                        msg_sb.push(0);
                        let run_but = tr(c"&Open Read-Only\n&Edit anyway\n&Recover\n&Quit\n&Abort");
                        let but = tr(
                            c"&Open Read-Only\n&Edit anyway\n&Recover\n&Delete it\n&Quit\n&Abort",
                        );
                        let running = ml_st(|st| st.proc_running.get()) != 0;
                        choice = do_dialog(
                            VIM_WARNING,
                            Some(tr(c"VIM - ATTENTION")),
                            crate::memory::bytes_as_cstr(&msg_sb),
                            if running { run_but } else { but },
                            1,
                            None,
                            0,
                        );

                        if running && choice >= 4 {
                            choice += 1;
                        }
                        msg_reset_scroll();
                    } else {
                        let mut need_clear = false;
                        msg_ext_set_kind(c"wmsg");
                        let msg_len = msg_sb.len();
                        msg_sb.push(0);
                        msg_multiline(
                            NvimString::from_bytes_len(&msg_sb, msg_len),
                            0,
                            false,
                            false,
                            &mut need_clear,
                        );
                    }
                    msg_state()
                        .no_wait_return
                        .set(msg_state().no_wait_return.get() - (1));
                }

                match choice {
                    SEA_CHOICE_READONLY => {
                        buf.b_p_ro.set(true);
                    }
                    SEA_CHOICE_EDIT => {}
                    SEA_CHOICE_RECOVER => {
                        workspace_state().swap_exists_action.set(SEA_RECOVER);
                    }
                    SEA_CHOICE_DELETE => {
                        os_remove(fname_cstr);
                    }
                    SEA_CHOICE_QUIT => {
                        workspace_state().swap_exists_action.set(SEA_QUIT);
                    }
                    SEA_CHOICE_ABORT => {
                        workspace_state().swap_exists_action.set(SEA_QUIT);
                        input_state().got_int.set(true);
                    }
                    SEA_CHOICE_NONE => {
                        msg_puts(c"\n");
                        if msg_state().msg_silent.get() == 0 {
                            msg_state().need_wait_return.set(true);
                        }
                    }
                    _ => {}
                }

                if choice != SEA_CHOICE_NONE && !os_path_exists(crate::memory::bytes_as_cstr(fvec))
                {
                    break;
                }
            }
        }

        if fvec[n - 1] == b'a' {
            if fvec[n - 2] == b'a' {
                emsg(tr(c"E326: Too many swap files found"));
                fname_vec = None;
                break;
            }
            fvec[n - 2] -= 1;
            fvec[n - 1] = b'z' + 1;
        }
        fvec[n - 1] -= 1;
    }

    if os_isdir(dir_name) {
        *found_existing_dir = true;
    } else if !*found_existing_dir && dirp.is_empty() {
        let mut failed_dir: Option<std::ffi::CString> = None;
        let ret = os_mkdir_recurse(dir_name, 0o755, &mut failed_dir, None);
        if ret != 0 {
            semsg!(
                tr(c"E303: Unable to create directory \"%s\" for swap file, recovery impossible: %s"),
                failed_dir.as_deref().unwrap_or(c""),
                uv_strerror(ret),
            );
        }
    }

    fname_vec.map(|v| crate::memory::cstring_until_nul(&v))
}

#[inline(always)]
fn b0_magic_wrong(b0p: &ZeroBlock) -> bool {
    b0p.b0_magic_long != B0_MAGIC_LONG
        || b0p.b0_magic_int != B0_MAGIC_INT
        || b0p.b0_magic_short != B0_MAGIC_SHORT
        || b0p.b0_magic_char != B0_MAGIC_CHAR
}

fn fnamecmp_ino(fname_c: Option<&CStr>, fname_s: Option<&CStr>, ino_block0: i64) -> bool {
    let mut ino_c: u64 = 0;
    let mut buf_c = [0u8; MAXPATHL];
    let mut buf_s = [0u8; MAXPATHL];

    let mut file_info = FileInfo::new();
    if let Some(fc) = fname_c
        && os_fileinfo(fc, &mut file_info)
    {
        ino_c = os_fileinfo_inode(&file_info);
    }

    let ino_s: u64 = if let Some(fs) = fname_s
        && os_fileinfo(fs, &mut file_info)
    {
        os_fileinfo_inode(&file_info)
    } else {
        ino_block0 as u64
    };

    if ino_c != 0 && ino_s != 0 {
        return ino_c != ino_s;
    }

    let retval_c = vim_full_name(fname_c.map(CStr::to_bytes), &mut buf_c, true);
    let retval_s = vim_full_name(fname_s.map(CStr::to_bytes), &mut buf_s, true);
    if retval_c && retval_s {
        return crate::memory::bytes_as_cstr(&buf_c) != crate::memory::bytes_as_cstr(&buf_s);
    }

    if ino_s == 0 && ino_c == 0 && !retval_c && !retval_s {
        return fname_c != fname_s;
    }
    true
}

#[inline(always)]
fn long_to_char(n: i64, s: &mut [u8; 4]) {
    *s = (n as u32).to_le_bytes();
}

#[inline(always)]
fn char_to_long(s: &[u8; 4]) -> i64 {
    i64::from(u32::from_le_bytes(*s))
}

/// Set the flags in the first block of the swapfile.
pub fn ml_setflags(buf: &FileBuf) {
    let Some(hp) = buf.b_ml.mfp_ref().and_then(|mfp| mfp.find_bhdr(0)) else {
        return;
    };
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.bhdr(hp).bh_data);
    b0p.set_dirty(if buf.b_changed.get() != 0 {
        B0_DIRTY
    } else {
        0
    });
    let ff_bits = ((get_fileformat(buf) + 1) as u8) & B0_FF_MASK;
    b0p.set_flags((b0p.flags() & !B0_FF_MASK) | ff_bits);
    add_b0_fenc(&mut b0p, buf);
    let mut hp_ref = buf.b_ml.bhdr_mut(hp);
    b0p.write_to_bytes(&mut hp_ref.bh_data);
    hp_ref.bh_flags |= BH_DIRTY;
    drop(hp_ref);
    mf_sync(&mut buf.b_ml.mfp_mut(), MFS_ZERO);
}

/// Find offset for line or line with offset.
pub fn ml_find_line_or_offset(
    buf: &FileBuf,
    lnum: Linenr,
    mut offp: Option<&mut i32>,
    no_ff: bool,
) -> i32 {
    let ffdos = i32::from(!no_ff && get_fileformat(buf) == EOL_DOS);

    let can_cache = lnum != 0 && ffdos == 0 && buf.b_ml.ml_line_lnum.get() == lnum;
    if lnum == 0 || buf.b_ml.ml_line_lnum.get() < lnum || !no_ff {
        ml_flush_line(buf, false);
        let cb = curbuf();
        if !cb.is_null() && cb != buf {
            ml_flush_line(&cb, false);
        }
    } else if can_cache && buf.b_ml.ml_line_offset.get() > 0 {
        return buf.b_ml.ml_line_offset.get() as i32;
    }

    if buf.b_ml.ml_usedchunks.get() <= 0 || lnum < 0 {
        if no_ff && buf.b_ml.ml_mfp.borrow().is_some() && (lnum == 1 || lnum == 2) {
            return lnum - 1;
        }
        return -1;
    }

    let offset = if offp.is_none() {
        0
    } else {
        *offp.as_deref_mut().unwrap()
    };
    if lnum == 0 && offset <= 0 {
        return 1;
    }

    let chunks = buf.b_ml.ml_chunks.borrow();
    if chunks.is_empty() {
        return -1;
    }

    let line_count = buf.b_ml.ml_line_count.get();
    if lnum != 0 {
        if lnum > line_count + 1 {
            return -1;
        }
        let mut size: i32 = 0;
        for ch in chunks.iter() {
            let ch_end_lnum = ch.start_lnum + (ch.lines.len() as Linenr);
            if lnum >= ch_end_lnum {
                size += ch.totalsize;
            } else {
                let in_cnt = (lnum - ch.start_lnum).max(0) as usize;
                for entry in &ch.lines[..in_cnt.min(ch.lines.len())] {
                    size += entry.text.to_bytes().len() as i32 + 1;
                }
                break;
            }
        }
        if ffdos != 0 {
            size += lnum - 1;
        }
        if (!buf.b_p_fixeol.get() || buf.b_p_bin.get()) && !buf.b_p_eol.get() && lnum > line_count {
            size -= ffdos + 1;
        }
        if can_cache && size > 0 {
            buf.b_ml.ml_line_offset.set(size as usize);
        }
        return size;
    }

    if offset != 0 {
        let mut size: i32 = 0;
        for ch in chunks.iter() {
            let ch_lines = ch.lines.len() as i32;
            let ch_bytes = ch.totalsize + ffdos * ch_lines;
            if offset > size + ch_bytes {
                size += ch_bytes;
                continue;
            }
            for (curline, entry) in (ch.start_lnum..).zip(ch.lines.iter()) {
                let line_len = entry.text.to_bytes().len() as i32 + 1;
                let total_line_bytes = line_len + ffdos;
                if offset <= size + total_line_bytes {
                    if offset == size + total_line_bytes {
                        if curline + 1 > line_count {
                            return -1;
                        }
                        *offp.as_deref_mut().unwrap() = 0;
                        return curline + 1;
                    }
                    *offp.as_deref_mut().unwrap() = offset - size;
                    return curline;
                }
                size += total_line_bytes;
            }
            return -1;
        }
        return -1;
    }

    0
}

/// Goto byte in buffer with offset `cnt`.
pub fn goto_byte(cnt: i32) {
    let mut boff = cnt;
    ml_flush_line(&curbuf(), false);
    setpcmark();
    if boff != 0 {
        boff -= 1;
    }
    let lnum = ml_find_line_or_offset(&curbuf(), 0, Some(&mut boff), false) as Linenr;
    if lnum < 1 {
        curwin()
            .w_cursor
            .set_lnum(curbuf().b_ml.ml_line_count.get());
        curwin().w_curswant.set(MAXCOL);
        coladvance(&curwin(), MAXCOL);
    } else {
        curwin().w_cursor.set_lnum(lnum);
        curwin().w_cursor.set_col(boff as Colnr);
        curwin().w_cursor.set_coladd(0);
        curwin().w_set_curswant.set(true);
    }
    check_cursor(&curwin());
    mb_adjust_cursor();
}

/// Increment the line pointer `lp` crossing line boundaries as necessary.
pub fn inc(lp: &mut CursorPos) -> i32 {
    if lp.col != MAXCOL {
        let p = ml_get_pos_slice(lp);
        if !p.is_empty() {
            let l = crate::mbyte::utfc_slice2len(&p) as i32;
            lp.col += l;
            return if p.len() > l as usize { 0 } else { 2 };
        }
    }
    if lp.lnum != curbuf().b_ml.ml_line_count.get() {
        lp.col = 0;
        lp.lnum += 1;
        lp.coladd = 0;
        return 1;
    }
    -1
}

/// Same as `inc()`, but skip NUL at the end of non-empty lines.
pub fn incl(lp: &mut CursorPos) -> i32 {
    let mut r = inc(lp);
    if r >= 1 && lp.col != 0 {
        r = inc(lp);
    }
    r
}

/// Decrements position `lp` by one character (multibyte-aware), crossing line boundaries.
pub fn dec(lp: &mut CursorPos) -> i32 {
    lp.coladd = 0;
    if lp.col == MAXCOL {
        let line = ml_get_slice(lp.lnum);
        lp.col = line.len() as Colnr;
        lp.col -= crate::mbyte::utf_head_off_slice(&line, lp.col as usize);
        return 0;
    }

    if lp.col > 0 {
        lp.col -= 1;
        let line = ml_get_slice(lp.lnum);
        lp.col -= crate::mbyte::utf_head_off_slice(&line, lp.col as usize);
        return 0;
    }
    if lp.lnum > 1 {
        lp.lnum -= 1;
        let line = ml_get_slice(lp.lnum);
        lp.col = line.len() as Colnr;
        lp.col -= crate::mbyte::utf_head_off_slice(&line, lp.col as usize);
        return 1;
    }

    -1
}

/// Same as `dec()`, but skip NUL at the end of non-empty lines.
pub fn decl(lp: &mut CursorPos) -> i32 {
    let mut r = dec(lp);
    if r == 1 && lp.col != 0 {
        r = dec(lp);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ml_get_buf_slice_returns_empty_when_unallocated() {
        let buf = FileBuf::new();
        assert_eq!(ml_get_buf_slice(&buf, 1), b"");
        assert_eq!(ml_get_buf_len(&buf, 1), 0);
    }

    #[test]
    fn native_memline_operations_chunksize_and_swap_serialization_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let saved_curbuf = workspace_state().curbuf.get();
        let buf = FileBuf::new_rc();
        buf.b_p_swf.set(false);
        workspace_state()
            .curbuf
            .set(crate::globals::BufPtr::from(buf.clone()));

        assert!(ml_open(&buf));
        assert_eq!(buf.b_ml.ml_line_count.get(), 1);
        assert_ne!(buf.b_ml.ml_flags.get() & ML_EMPTY, 0);

        assert!(ml_replace_buf(&buf, 1, b"line-1", true, false));
        for i in 2..=900 {
            let s = format!("line-{i}");
            assert!(ml_append_buf(&buf, i - 1, s.as_bytes(), 0, false));
        }
        assert_eq!(buf.b_ml.ml_line_count.get(), 900);
        assert_eq!(ml_get_buf_slice(&buf, 1), b"line-1");
        assert_eq!(ml_get_buf_slice(&buf, 450), b"line-450");
        assert_eq!(ml_get_buf_slice(&buf, 900), b"line-900");

        // Chunk splitting (>800 lines) and byte-offset lookup
        assert!(buf.b_ml.ml_usedchunks.get() >= 2);
        let off_line_2 = ml_find_line_or_offset(&buf, 2, None, true);
        assert_eq!(off_line_2, 7); // "line-1\n"
        let mut boff = 8;
        let found_lnum = ml_find_line_or_offset(&buf, 0, Some(&mut boff), true);
        assert_eq!(found_lnum, 2);
        assert_eq!(boff, 1);

        // :global mark bits
        ml_setmarked(10);
        ml_setmarked(500);
        assert_eq!(ml_firstmarked(), 10);
        assert_eq!(ml_firstmarked(), 500);
        assert_eq!(ml_firstmarked(), 0);
        ml_setmarked(25);
        ml_clearmarked();
        assert_eq!(ml_firstmarked(), 0);

        // Swap block serialization builds Block 1 (PointerBlock) and Block 2+ (DataBlocks)
        ml_sync_swap_blocks(&buf);
        assert!(!buf.b_ml.ml_swap_dirty.get());
        {
            let mfp = buf.b_ml.mfp_ref().unwrap();
            let pb_id = mfp.find_bhdr(1).expect("block 1 must exist");
            assert_eq!(mfp.bhdr(pb_id).pb_id(), PTR_ID);
            assert!(mfp.bhdr(pb_id).pb_count() >= 1);
            let db_id = mfp.find_bhdr(2).expect("block 2 must exist");
            assert_eq!(mfp.bhdr(db_id).db_id(), DATA_ID);
        }

        assert!(ml_delete_buf(&buf, 450, false));
        assert_eq!(buf.b_ml.ml_line_count.get(), 899);
        assert_eq!(ml_get_buf_slice(&buf, 450), b"line-451");

        ml_close(&buf, 1);
        workspace_state().curbuf.set(saved_curbuf);
    }

    fn assert_chunk_invariants(buf: &FileBuf) {
        let chunks = buf.b_ml.ml_chunks.borrow();
        let mut expected_start: Linenr = 1;
        for ch in chunks.iter() {
            assert_eq!(ch.start_lnum, expected_start);
            assert!(!ch.lines.is_empty());
            let actual_total: i32 = ch
                .lines
                .iter()
                .map(|e| e.text.to_bytes().len() as i32 + 1)
                .sum();
            assert_eq!(ch.totalsize, actual_total);
            expected_start += ch.lines.len() as Linenr;
        }
        assert_eq!(expected_start - 1, buf.b_ml.ml_line_count.get());
    }

    #[test]
    fn chunk_split_merge_top_mid_edits_and_byte_offsets_preserve_invariants() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let saved_curbuf = workspace_state().curbuf.get();
        let buf = FileBuf::new_rc();
        buf.b_p_swf.set(false);
        workspace_state()
            .curbuf
            .set(crate::globals::BufPtr::from(buf.clone()));

        assert!(ml_open(&buf));
        // Freshly opened empty buffer: line2byte(1) is -1 (unless no_ff=true -> 0)
        assert_eq!(ml_find_line_or_offset(&buf, 1, None, false), -1);
        assert_eq!(ml_find_line_or_offset(&buf, 1, None, true), 0);
        assert_eq!(ml_find_line_or_offset(&buf, 2, None, true), 1);

        // Replacing line 1 with "" (extra == 0) must still activate ml_usedchunks
        assert!(ml_replace_buf_cstr(
            &buf,
            1,
            crate::option_vars::RcCStr::EMPTY,
        ));
        assert_eq!(buf.b_ml.ml_flags.get() & ML_EMPTY, 0);
        assert_eq!(buf.b_ml.ml_usedchunks.get(), 1);
        assert_eq!(ml_find_line_or_offset(&buf, 1, None, false), 0);

        // Insert 1,700 lines at the top of the buffer (lnum = 0) to force multiple
        // chunk splits at chunk 0 (MLCS_MAXL = 800 -> 400 + 400).
        assert!(ml_replace_buf_cstr(
            &buf,
            1,
            crate::option_vars::RcCStr::from_bytes(b"tail"),
        ));
        for i in (1..=1700).rev() {
            let s = format!("top-{i:04}");
            assert!(ml_append_buf_cstr(
                &buf,
                0,
                crate::option_vars::RcCStr::from_bytes(s.as_bytes()),
                false,
            ));
        }
        assert_eq!(buf.b_ml.ml_line_count.get(), 1701);
        assert!(buf.b_ml.ml_chunks.borrow().len() >= 3);
        assert_chunk_invariants(&buf);

        for i in 1..=1700 {
            let expected = format!("top-{i:04}");
            with_ml_buf_line(&buf, i, |bytes| {
                assert_eq!(bytes, expected.as_bytes());
            });
            assert_eq!(ml_get_buf_len(&buf, i), expected.len() as Colnr);
        }
        assert_eq!(ml_get_buf_slice(&buf, 1701), b"tail");

        // Test dirty in-place line interaction with with_ml_buf_line and ml_get_buf_len
        with_ml_get_buf_mut_slice(&buf, 500, |s| {
            s[0..3].copy_from_slice(b"MOD");
        });
        with_ml_buf_line(&buf, 500, |bytes| {
            assert_eq!(bytes, b"MOD-0500");
        });
        with_ml_buf_line(&buf, 501, |bytes| {
            assert_eq!(bytes, b"top-0501");
        });
        with_ml_get_buf_mut_slice(&buf, 500, |s| {
            s[5] = 0;
        });
        assert_eq!(ml_get_buf_len(&buf, 500), 5);
        with_ml_buf_line(&buf, 500, |bytes| {
            assert_eq!(bytes, b"MOD-0");
        });
        buf.b_ml.delete_dirty_line_bytes(&buf, 3, 2, 1, 5, 3);
        assert_eq!(ml_get_buf_len(&buf, 500), 3);
        with_ml_buf_line(&buf, 500, |bytes| {
            assert_eq!(bytes, b"MOD");
        });

        // Verify byte-offset round-trip (line -> offset -> line) across all chunks
        // in both unix and dos fileformats.
        for &ff in &[crate::fileio::EOL_UNIX, EOL_DOS] {
            buf.b_p_ff
                .set_option_cstr(if ff == EOL_DOS { c"dos" } else { c"unix" });
            let eol_len = if ff == EOL_DOS { 2 } else { 1 };
            let mut expected_off: i32 = 0;
            for lnum in 1..=buf.b_ml.ml_line_count.get() {
                let off = ml_find_line_or_offset(&buf, lnum, None, false);
                assert_eq!(off, expected_off);
                let mut query_off = off;
                let resolved_lnum = ml_find_line_or_offset(&buf, 0, Some(&mut query_off), false);
                assert_eq!(resolved_lnum, lnum);
                assert_eq!(query_off, 0);

                let line_len = ml_get_buf_len(&buf, lnum);
                if line_len > 0 {
                    let mut mid_off = off + line_len - 1;
                    let mid_lnum = ml_find_line_or_offset(&buf, 0, Some(&mut mid_off), false);
                    assert_eq!(mid_lnum, lnum);
                    assert_eq!(mid_off, line_len - 1);
                }
                expected_off += line_len + eol_len;
            }
        }

        // Delete 1,600 lines from middle and top to exercise chunk removal and left/right merges.
        for _ in 0..800 {
            assert!(ml_delete_buf(&buf, 200, false));
        }
        assert_chunk_invariants(&buf);
        for _ in 0..800 {
            assert!(ml_delete_buf(&buf, 1, false));
        }
        assert_chunk_invariants(&buf);
        assert_eq!(buf.b_ml.ml_line_count.get(), 101);
        assert_eq!(buf.b_ml.ml_chunks.borrow().len(), 1);
        assert_eq!(ml_get_buf_slice(&buf, 101), b"tail");

        // Nested with_ml_buf_line and ml_get_buf on the same buffer must not panic on ml_line_cstr borrow
        with_ml_buf_line(&buf, 1, |line1| {
            assert_eq!(line1, b"top-1601");
            with_ml_buf_line(&buf, 2, |line2| {
                assert_eq!(line2, b"top-1602");
                assert_eq!(ml_get_buf(&buf, 3).to_bytes(), b"top-1603");
            });
        });

        ml_close(&buf, 1);
        workspace_state().curbuf.set(saved_curbuf);
    }

    #[test]
    fn swap_serialization_handles_oversized_lines_and_multi_level_pointer_blocks() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let saved_curbuf = workspace_state().curbuf.get();
        let buf = FileBuf::new_rc();
        buf.b_p_swf.set(false);
        workspace_state()
            .curbuf
            .set(crate::globals::BufPtr::from(buf.clone()));

        assert!(ml_open(&buf));
        // Line 1 is 9,000 bytes (> 2 4KB swap pages)
        let huge_line = vec![b'x'; 9000];
        assert!(ml_replace_buf(&buf, 1, &huge_line, true, false));
        // Append 220 2,500-byte lines so each requires its own DataBlock (> 204 DataBlocks total,
        // forcing a 2-level PointerBlock tree in ml_sync_swap_blocks).
        let medium_line = vec![b'y'; 2500];
        for i in 1..=220 {
            assert!(ml_append_buf(&buf, i, &medium_line, 0, false));
        }
        ml_sync_swap_blocks(&buf);
        assert!(!buf.b_ml.ml_swap_dirty.get());
        {
            let mfp = buf.b_ml.mfp_ref().unwrap();
            let root_pb = mfp.find_bhdr(1).expect("root pointer block 1 must exist");
            assert_eq!(mfp.bhdr(root_pb).pb_id(), PTR_ID);
            // 221 DataBlocks > pb_count_max (204), so root PointerBlock points to child PointerBlocks
            assert_eq!(mfp.bhdr(root_pb).pb_count(), 2);
            let child_bnum = mfp.bhdr(root_pb).pb_entry(0).pe_bnum;
            let child_pb = mfp
                .find_bhdr(child_bnum)
                .expect("child pointer block must exist");
            assert_eq!(mfp.bhdr(child_pb).pb_id(), PTR_ID);
            // First DataBlock in child_pb is the 9,000-byte huge line spanning 3 pages
            let first_pe = mfp.bhdr(child_pb).pb_entry(0);
            assert!(first_pe.pe_page_count >= 3);
        }

        // Append more lines so we have 2 chunks (>400 lines total), sync, clear BH_DIRTY
        // (simulating mf_sync writing all blocks to disk), then modify 1 line in chunk 0
        // and verify incremental ml_sync_swap_blocks leaves chunk 1's DataBlocks clean (BH_DIRTY == 0).
        for i in 221..=500 {
            let s = format!("tail-chunk-line-{i}");
            assert!(ml_append_buf(&buf, i, s.as_bytes(), 0, false));
        }
        assert_eq!(buf.b_ml.ml_chunks.borrow().len(), 2);
        ml_sync_swap_blocks(&buf);
        let chunk1_db_ids: Vec<BhdrId> = buf.b_ml.ml_chunks.borrow()[1]
            .swap_blocks
            .iter()
            .map(|&(_, id)| id)
            .collect();
        assert!(!chunk1_db_ids.is_empty());
        {
            let mut mfp = buf.b_ml.mfp_mut();
            for hp in mfp.mf_blocks.iter_mut().flatten() {
                hp.bh_flags &= !BH_DIRTY;
            }
        }
        assert!(ml_replace_buf_cstr(
            &buf,
            10,
            crate::option_vars::RcCStr::from_bytes(b"modified-line-10"),
        ));
        assert!(buf.b_ml.ml_chunks.borrow()[0].swap_blocks.is_empty());
        assert!(!buf.b_ml.ml_chunks.borrow()[1].swap_blocks.is_empty());
        ml_sync_swap_blocks(&buf);
        {
            let mfp = buf.b_ml.mfp_ref().unwrap();
            // Untouched chunk 1 keeps its exact DataBlocks with BH_DIRTY == 0
            for &id in &chunk1_db_ids {
                assert_eq!(mfp.bhdr(id).bh_flags & BH_DIRTY, 0);
            }
            // Modified chunk 0 has newly packed DataBlocks with BH_DIRTY != 0
            for &(_, id) in &buf.b_ml.ml_chunks.borrow()[0].swap_blocks {
                assert_ne!(mfp.bhdr(id).bh_flags & BH_DIRTY, 0);
            }
        }

        ml_close(&buf, 1);
        workspace_state().curbuf.set(saved_curbuf);
    }

    #[test]
    fn ml_try_mark_global_and_vimgrep_parallel_deterministic() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _editor = crate::main_init::test_editor();
        let saved_curbuf = workspace_state().curbuf.get();
        let buf = FileBuf::new_rc();
        buf.b_p_swf.set(false);
        workspace_state()
            .curbuf
            .set(crate::globals::BufPtr::from(buf.clone()));

        assert!(ml_open(&buf));
        let total = 9000;
        for i in 1..=total {
            let line = if i % 150 == 0 {
                format!("line {i:05}: [TAG] special entry {i}")
            } else {
                format!("line {i:05}: ordinary entry {i}")
            };
            let rc = crate::option_vars::RcCStr::from_bytes(line.as_bytes());
            if i == 1 {
                assert!(ml_replace_buf_cstr(&buf, 1, rc));
            } else {
                assert!(ml_append_buf_cstr(&buf, i - 1, rc, false));
            }
        }

        // 1. Test general NFA regex across >= 8,192 lines (parallel worker path)
        let mut rmm = crate::regexp::Regmmatch::new();
        rmm.set_prog(crate::regexp::vim_regcomp(
            b"special entry [0-9]\\+$",
            crate::regexp::RE_MAGIC,
        ));
        let ndone = ml_try_mark_global_lines(&buf, 1, total, &rmm, false)
            .expect("general single-line NFA should use fast path");
        assert_eq!(ndone, 60);
        for k in 1..=60 {
            assert_eq!(ml_firstmarked(), k * 150);
        }
        assert_eq!(ml_firstmarked(), 0);

        let vgr = ml_try_vimgrep_lines(&buf, &rmm, b"", false, true, 1000)
            .expect("vimgrep NFA should use parallel fast path");
        assert_eq!(vgr.len(), 60);
        assert_eq!(vgr[0].lnum, 150);
        assert_eq!(vgr[59].lnum, 9000);
        rmm.free_prog();

        // 2. Test fuzzy vimgrep across >= 4,096 lines (parallel worker path)
        let vgr_fuzzy =
            ml_try_vimgrep_lines(&buf, &rmm, b"TAG special", true, false, 1000).unwrap();
        assert_eq!(vgr_fuzzy.len(), 60);
        assert_eq!(vgr_fuzzy[0].lnum, 150);
        assert_eq!(vgr_fuzzy[59].lnum, 9000);

        // 3. Test fast_literal_params with sub-range, invert (:vglobal), and multi-match /g
        rmm.set_prog(crate::regexp::vim_regcomp(
            b"entry",
            crate::regexp::RE_MAGIC,
        ));
        let inv_ndone = ml_try_mark_global_lines(&buf, 100, 500, &rmm, true).unwrap();
        assert_eq!(inv_ndone, 0);
        let vgr_lit = ml_try_vimgrep_lines(&buf, &rmm, b"", false, true, 10).unwrap();
        assert_eq!(vgr_lit.len(), 10);
        assert_eq!(vgr_lit[0].lnum, 1);
        rmm.free_prog();

        // 4. Test parallel ml_sync_swap_blocks (dirty_count = 23 chunks >= 8)
        assert!(buf.b_ml.ml_chunks.borrow().len() >= 8);
        ml_sync_swap_blocks(&buf);
        assert!(!buf.b_ml.ml_swap_dirty.get());
        {
            let chunks = buf.b_ml.ml_chunks.borrow();
            let mut expected_lnum = 1;
            for ch in chunks.iter() {
                assert!(!ch.swap_blocks.is_empty());
                for &(pe, _) in &ch.swap_blocks {
                    assert_eq!(pe.pe_old_lnum, expected_lnum);
                    expected_lnum += pe.pe_line_count;
                }
            }
            assert_eq!(expected_lnum - 1, total);
        }

        ml_close(&buf, 1);
        workspace_state().curbuf.set(saved_curbuf);
    }
}
