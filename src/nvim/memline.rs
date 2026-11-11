#![forbid(unsafe_code)]

//! Buffer line storage and swapfile recovery (`src/nvim/memline.c`).

use crate::api::private::helpers::{HLF_E, cstr_as_string};
use crate::api::private::validate::{NvimString, Object, ObjectData, ObjectType};
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
    BH_DIRTY, Bhdr, BhdrId, Blocknr, MFS_ALL, MFS_STOP, MFS_ZERO, Memfile, mf_close, mf_close_file,
    mf_free, mf_free_fnames, mf_get, mf_need_trans, mf_new, mf_new_page_size, mf_open,
    mf_open_file, mf_put, mf_set_dirty, mf_set_fnames, mf_sync, mf_trans_del,
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

#[repr(C)]
pub struct Memline {
    pub ml_line_count: i32,
    pub ml_mfp: Option<Box<crate::memfile::Memfile>>,
    pub ml_stack: Vec<Infoptr>,
    pub ml_stack_top: i32,
    pub ml_flags: i32,
    pub ml_line_textlen: i32,
    pub ml_line_lnum: i32,
    pub ml_line_start: usize,
    pub ml_line_buf: Vec<u8>,
    pub ml_line_offset: usize,
    pub ml_line_offset_ff: i32,
    pub ml_locked: Option<BhdrId>,
    pub ml_locked_low: i32,
    pub ml_locked_high: i32,
    pub ml_locked_lineadd: i32,
    pub ml_chunksize: Vec<Chunksize>,
    pub ml_usedchunks: i32,
}

impl Memline {
    pub const fn new() -> Self {
        Self {
            ml_line_count: 0,
            ml_mfp: None,
            ml_stack: Vec::new(),
            ml_stack_top: 0,
            ml_flags: 0,
            ml_line_textlen: 0,
            ml_line_lnum: 0,
            ml_line_start: 0,
            ml_line_buf: Vec::new(),
            ml_line_offset: 0,
            ml_line_offset_ff: 0,
            ml_locked: None,
            ml_locked_low: 0,
            ml_locked_high: 0,
            ml_locked_lineadd: 0,
            ml_chunksize: Vec::new(),
            ml_usedchunks: 0,
        }
    }

    #[inline(always)]
    pub fn locked_bhdr(&self) -> Option<&Bhdr> {
        let id = self.ml_locked?;
        Some(self.ml_mfp.as_ref()?.bhdr(id))
    }

    #[inline(always)]
    pub fn locked_bhdr_mut(&mut self) -> Option<&mut Bhdr> {
        let id = self.ml_locked?;
        Some(self.ml_mfp.as_mut()?.bhdr_mut(id))
    }

    #[inline(always)]
    pub fn current_line_slice(&self) -> &[u8] {
        let len = self.ml_line_textlen.max(0) as usize;
        if (self.ml_flags & ML_LINE_DIRTY) != 0 {
            let s = self.ml_line_buf.as_slice();
            &s[..len.min(s.len())]
        } else if self.ml_line_start == usize::MAX {
            &b"???\0"[..len.min(4)]
        } else if let Some(hp) = self.locked_bhdr() {
            let s = hp.bh_data.as_slice();
            let start = self.ml_line_start.min(s.len());
            &s[start..(start + len).min(s.len())]
        } else {
            &b"???\0"[..len.min(4)]
        }
    }

    #[inline(always)]
    pub fn mfp_ref(&self) -> Option<&crate::memfile::Memfile> {
        self.ml_mfp.as_deref()
    }

    #[inline(always)]
    pub fn mfp_opt(&mut self) -> Option<&mut crate::memfile::Memfile> {
        self.ml_mfp.as_deref_mut()
    }

    #[inline(always)]
    pub fn mfp_mut(&mut self) -> &mut crate::memfile::Memfile {
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

const STACK_INCR: i32 = 5;

const ML_LOCKED_DIRTY: i32 = 0x04;
const ML_LOCKED_POS: i32 = 0x08;
const ML_LINE_INPLACE: i32 = 0x10;

const ML_APPEND_NEW: i32 = 1;
const ML_APPEND_MARK: i32 = 2;

const ML_CHNK_ADDLINE: i32 = 1;
const ML_CHNK_DELLINE: i32 = 2;
const ML_CHNK_UPDLINE: i32 = 3;

const ML_DELETE: i32 = 0x11;
const ML_INSERT: i32 = 0x12;
const ML_FIND: i32 = 0x13;
const ML_FLUSH: i32 = 0x02;

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
#[derive(Clone, Copy, Default)]
pub struct Infoptr {
    pub ip_bnum: Blocknr,
    pub ip_low: Linenr,
    pub ip_high: Linenr,
    pub ip_index: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Chunksize {
    pub mlcs_numlines: i32,
    pub mlcs_totalsize: i32,
}

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

#[repr(C)]
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
    ml_upd_lastbuf: std::cell::Cell<crate::globals::BufPtr>,
    ml_upd_lastcurline: std::cell::Cell<Linenr>,
    ml_upd_lastcurix: std::cell::Cell<i32>,
}

impl MemlineModuleState {
    const fn new() -> Self {
        Self {
            lowest_marked: std::cell::Cell::new(0),
            proc_running: std::cell::Cell::new(0),
            ml_get_recursive: std::cell::Cell::new(0),
            ml_flush_entered: std::cell::Cell::new(false),
            ml_upd_lastbuf: std::cell::Cell::new(crate::globals::BufPtr::NULL),
            ml_upd_lastcurline: std::cell::Cell::new(0),
            ml_upd_lastcurix: std::cell::Cell::new(0),
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
static E_POINTER_BLOCK_ID_WRONG: &CStr = c"E317: Pointer block id wrong";
static E_POINTER_BLOCK_ID_WRONG_TWO: &CStr = c"E317: Pointer block id wrong 2";
static E_POINTER_BLOCK_ID_WRONG_THREE: &CStr = c"E317: Pointer block id wrong 3";
static E_POINTER_BLOCK_ID_WRONG_FOUR: &CStr = c"E317: Pointer block id wrong 4";
static E_LINE_NUMBER_OUT_OF_RANGE_NR_PAST_THE_END: &CStr =
    c"E322: Line number out of range: %lld past the end";
static E_LINE_COUNT_WRONG_IN_BLOCK_NR: &CStr = c"E323: Line count wrong in block %lld";
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
const fn ml_simple(x: i32) -> bool {
    (x & 0x10) != 0
}

#[inline(always)]
fn pb_count_max(mfp: &Memfile) -> u16 {
    ((mfp.mf_page_size as usize - std::mem::offset_of!(PointerBlock, pb_pointer))
        / size_of::<PointerEntry>()) as u16
}

#[inline(always)]
fn bh_copy_within(hp: &mut Bhdr, src_off: usize, dst_off: usize, len: usize) {
    hp.bh_data.copy_within(src_off..src_off + len, dst_off);
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
    fn pb_copy_entries(&mut self, src_idx: usize, dst_idx: usize, count: usize) {
        let src_off = PB_HEADER_SIZE + src_idx * PE_SIZE;
        let dst_off = PB_HEADER_SIZE + dst_idx * PE_SIZE;
        let len = count * PE_SIZE;
        self.bh_data.copy_within(src_off..src_off + len, dst_off);
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
pub fn ml_open(buf: &mut FileBuf) -> bool {
    buf.b_ml.ml_stack.clear();
    buf.b_ml.ml_stack_top = 0;
    buf.b_ml.ml_locked = None;
    buf.b_ml.ml_line_lnum = 0;
    buf.b_ml.ml_line_offset = 0;
    buf.b_ml.ml_chunksize.clear();
    buf.b_ml.ml_usedchunks = 0;

    if (g_st().cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0 {
        buf.b_p_swf = false;
    }

    buf.b_may_swap = buf.terminal.is_null() && opt_vars().p_uc != 0 && buf.b_p_swf;

    buf.b_ml.ml_mfp = mf_open(None, 0);
    if buf.b_ml.mfp_ref().is_none() {
        return false;
    }
    buf.b_ml.ml_flags = ML_EMPTY;
    buf.b_ml.ml_line_count = 1;

    let mut hp = mf_new(buf.b_ml.mfp_mut(), false, 1);
    if buf.b_ml.mfp_ref().unwrap().bhdr(hp).bh_bnum != 0 {
        iemsg(tr(c"E298: Didn't get block nr 0?"));
        return ml_open_error(buf, Some(hp));
    }
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.mfp_ref().unwrap().bhdr(hp).bh_data);

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

    if !buf.b_spell {
        b0p.set_dirty(if buf.b_changed != 0 { B0_DIRTY } else { 0 });
        b0p.set_flags((get_fileformat(&*buf) + 1) as u8);
        set_b0_fname(&mut b0p, buf);
        os_get_username(&mut b0p.b0_uname);
        b0p.b0_uname[B0_UNAME_SIZE - 1] = 0;
        os_get_hostname(&mut b0p.b0_hname);
        b0p.b0_hname[B0_HNAME_SIZE - 1] = 0;
        long_to_char(os_get_pid(), &mut b0p.b0_pid);
    }

    b0p.write_to_bytes(&mut buf.b_ml.mfp_mut().bhdr_mut(hp).bh_data);
    mf_put(buf.b_ml.mfp_mut(), hp, true, false);
    if !buf.b_help && !buf.b_spell {
        mf_sync(buf.b_ml.mfp_mut(), 0);
    }

    hp = ml_new_ptr(buf.b_ml.mfp_mut());
    if buf.b_ml.mfp_ref().unwrap().bhdr(hp).bh_bnum != 1 {
        iemsg(tr(c"E298: Didn't get block nr 1?"));
        return ml_open_error(buf, Some(hp));
    }
    {
        let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
        hp_ref.set_pb_count(1);
        hp_ref.set_pb_entry(
            0,
            PointerEntry {
                pe_bnum: 2,
                pe_page_count: 1,
                pe_old_lnum: 1,
                pe_line_count: 1,
            },
        );
    }
    mf_put(buf.b_ml.mfp_mut(), hp, true, false);

    hp = ml_new_data(buf.b_ml.mfp_mut(), false, 1);
    if buf.b_ml.mfp_ref().unwrap().bhdr(hp).bh_bnum != 2 {
        iemsg(tr(c"E298: Didn't get block nr 2?"));
        return ml_open_error(buf, Some(hp));
    }

    let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
    let txt_start = hp_ref.db_txt_start() - 1;
    hp_ref.set_db_txt_start(txt_start);
    hp_ref.set_db_idx(0, txt_start);
    hp_ref.set_db_free(hp_ref.db_free() - (1 + INDEX_SIZE as u32));
    hp_ref.set_db_line_count(1);
    bh_write_bytes(hp_ref, txt_start as usize, b"", 1);

    true
}

fn ml_open_error(buf: &mut FileBuf, hp: Option<BhdrId>) -> bool {
    if let Some(mfp) = buf.b_ml.mfp_opt() {
        if let Some(hp) = hp {
            mf_put(mfp, hp, false, false);
        }
        mf_close(buf.b_ml.ml_mfp.take(), true);
    }
    false
}

/// Called when the file name of `buf` has been changed.
pub fn ml_setname(buf: &mut FileBuf) {
    let mut success = false;
    let mfp_addr = buf.b_ml.mfp_ref().map_or(0, |m| ptr::from_ref(m) as usize);
    if buf.b_ml.mfp_mut().mf_fd < 0 {
        if opt_vars().p_uc != 0 && (g_st().cmdmod.cmod_flags & CMOD_NOSWAPFILE) == 0 {
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
        if buf.b_ml.mfp_ref().map_or(0, |m| ptr::from_ref(m) as usize) != mfp_addr {
            return;
        }
        let Some(fname) = fname else {
            continue;
        };

        let mfp = buf.b_ml.mfp_mut();
        let mf_fname_cstr = mfp.mf_fname.as_deref().unwrap_or(c"");
        if path_equal(&fname, mf_fname_cstr, K_PATH_CMP_LITERAL) {
            success = true;
            break;
        }
        if mfp.mf_fd >= 0 {
            os_close(mfp.mf_fd);
            mfp.mf_fd = -1;
        }

        if vim_rename(mf_fname_cstr, &fname) == 0 {
            success = true;
            mf_free_fnames(mfp);
            mf_set_fnames(mfp, fname);
            ml_upd_block0(buf, UpdBlock0::UbSameDir);
            break;
        }
    }

    let mfp = buf.b_ml.mfp_mut();
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
    for buf in for_all_buffers_mut() {
        if !buf.b_p_ro || buf.b_changed != 0 {
            ml_open_file(buf);
        }
    }
}

/// Open a swapfile for an existing memfile, if there is no swapfile yet.
pub fn ml_open_file(buf: &mut FileBuf) {
    let mfp_addr = buf.b_ml.mfp_ref().map_or(0, |m| ptr::from_ref(m) as usize);
    let Some(mfp_fd) = buf.b_ml.mfp_ref().map(|m| m.mf_fd) else {
        return;
    };
    if mfp_fd >= 0
        || !buf.b_p_swf
        || (g_st().cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0
        || !buf.terminal.is_null()
    {
        return;
    }

    if buf.b_spell {
        if let Some(fname) = vim_tempname() {
            mf_open_file(buf.b_ml.mfp_mut(), fname);
        }
        buf.b_may_swap = false;
        return;
    }

    let p_dir = opt_vars().p_dir.to_owned();
    let mut dirp = p_dir.as_bytes();
    let mut found_existing_dir = false;
    while !dirp.is_empty() {
        let fname = findswapname(buf, &mut dirp, None, &mut found_existing_dir);
        if buf.b_ml.mfp_ref().map_or(0, |m| ptr::from_ref(m) as usize) != mfp_addr {
            return;
        }
        let Some(fname) = fname else {
            continue;
        };
        let opened = {
            let mfp = buf.b_ml.mfp_mut();
            if mf_open_file(mfp, fname) {
                mfp.mf_dirty = MF_DIRTY_YES_NOSYNC;
                true
            } else {
                false
            }
        };
        if opened {
            ml_upd_block0(buf, UpdBlock0::UbSameDir);

            let synced = mf_sync(buf.b_ml.mfp_mut(), MFS_ZERO);
            if synced {
                mf_set_dirty(buf.b_ml.mfp_mut());
                break;
            }
            mf_close_file(buf, false);
        }
    }

    if !opt_vars().p_dir.is_empty() && buf.b_ml.mfp_mut().mf_fname.is_none() {
        g_st().need_wait_return = true;
        g_st().no_wait_return += 1;
        let sp = buf_spname(buf);
        semsg!(
            tr(c"E303: Unable to open swap file for \"%s\", recovery impossible"),
            sp.unwrap_or_else(|| buf.fname_cstr()),
        );
        g_st().no_wait_return -= 1;
    }

    buf.b_may_swap = false;
}

/// If still need to create a swapfile, create a swapfile now.
pub fn check_need_swap(newfile: bool) {
    let old_msg_silent = g_st().msg_silent;
    g_st().msg_silent = 0;

    if curbuf_mut().b_may_swap && (!curbuf_mut().b_p_ro || !newfile) {
        ml_open_file(curbuf_mut());
    }

    g_st().msg_silent = old_msg_silent;
}

/// Close memline for buffer `buf`.
pub fn ml_close(buf: &mut FileBuf, del_file: i32) {
    if buf.b_ml.ml_mfp.is_none() {
        return;
    }
    mf_close(buf.b_ml.ml_mfp.take(), del_file != 0);
    buf.b_ml.ml_line_buf = Vec::new();
    buf.b_ml.ml_stack = Vec::new();
    buf.b_ml.ml_chunksize = Vec::new();

    buf.b_flags &= !BF_RECOVERED;
}

/// Close all existing memlines and memfiles.
pub fn ml_close_all(del_file: bool) {
    for buf in for_all_buffers_ptr() {
        ml_close(&mut *buf, i32::from(del_file));
    }
    spell_delete_wordlist();
    vim_deltempdir();
}

/// Close all memfiles for not modified buffers.
pub fn ml_close_notmod() {
    for buf in for_all_buffers_ptr() {
        if !buf_is_changed_fn(&mut *buf) {
            ml_close(&mut *buf, 1);
        }
    }
}

/// Update the timestamp in the `.swp` file.
pub fn ml_timestamp(buf: &mut FileBuf) {
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

fn ml_upd_block0(buf: &mut FileBuf, what: UpdBlock0) {
    let Some(hp) = buf.b_ml.mfp_opt().and_then(|mfp| mf_get(mfp, 0, 1)) else {
        return;
    };
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.mfp_ref().unwrap().bhdr(hp).bh_data);
    if !ml_check_b0_id(&b0p) {
        iemsg(tr(c"E304: ml_upd_block0(): Didn't get block 0??"));
    } else {
        if what == UpdBlock0::UbFname {
            set_b0_fname(&mut b0p, buf);
        } else {
            set_b0_dir_flag(&mut b0p, buf);
        }
        b0p.write_to_bytes(&mut buf.b_ml.mfp_mut().bhdr_mut(hp).bh_data);
    }
    mf_put(buf.b_ml.mfp_mut(), hp, true, false);
}

fn set_b0_fname(b0p: &mut ZeroBlock, buf: &mut FileBuf) {
    if buf.b_ffname.is_none() {
        b0p.b0_fname[0] = 0;
    } else {
        let mut uname = [0u8; B0_UNAME_SIZE];
        let flen = home_replace(
            None,
            buf.ffname_opt(),
            &mut b0p.b0_fname[..B0_FNAME_SIZE_CRYPT],
            true,
        );
        if b0p.b0_fname[0] == b'~' {
            let retval = os_get_username(&mut uname);
            let ulen = crate::memory::bytes_until_nul(&uname).len();
            if !retval || ulen + flen > B0_FNAME_SIZE_CRYPT - 1 {
                crate::memory::copy_bytes_to_c_buf(
                    &mut b0p.b0_fname[..B0_FNAME_SIZE_CRYPT],
                    buf.ffname_bytes(),
                );
            } else {
                b0p.b0_fname.copy_within(1..(1 + flen), ulen + 1);
                b0p.b0_fname[1..(1 + ulen)].copy_from_slice(&uname[..ulen]);
            }
        }
        let mut file_info = FileInfo::new();
        if os_fileinfo(buf.ffname_cstr(), &mut file_info) {
            buf_store_file_info(buf, &mut file_info);
            long_to_char(buf.b_mtime, &mut b0p.b0_mtime);
            long_to_char(os_fileinfo_inode(&file_info) as i64, &mut b0p.b0_ino);
            buf.b_mtime_read = buf.b_mtime;
            buf.b_mtime_read_ns = buf.b_mtime_ns;
        } else {
            long_to_char(0, &mut b0p.b0_mtime);
            long_to_char(0, &mut b0p.b0_ino);
            buf.b_mtime = 0;
            buf.b_mtime_ns = 0;
            buf.b_mtime_read = 0;
            buf.b_mtime_read_ns = 0;
            buf.b_orig_size = 0;
            buf.b_orig_mode = 0;
        }
    }

    add_b0_fenc(b0p, curbuf_mut());
}

fn set_b0_dir_flag(b0p: &mut ZeroBlock, buf: &mut FileBuf) {
    if same_directory(
        buf.b_ml.mfp_ref().and_then(|m| m.mf_fname.as_deref()),
        buf.ffname_opt(),
    ) {
        b0p.set_flags(b0p.flags() | B0_SAME_DIR);
    } else {
        b0p.set_flags(b0p.flags() & !B0_SAME_DIR);
    }
}

fn add_b0_fenc(b0p: &mut ZeroBlock, buf: &mut FileBuf) {
    let size = B0_FNAME_SIZE_NOCRYPT;
    let fenc_bytes = buf.b_p_fenc.as_bytes();
    let n = fenc_bytes.len();
    if crate::memory::bytes_as_cstr(&b0p.b0_fname).to_bytes().len() + n + 1 > size {
        b0p.set_flags(b0p.flags() & !B0_HAS_FENC);
    } else {
        b0p.b0_fname[(size - n)..size].copy_from_slice(fenc_bytes);
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

    g_st().recoverymode = true;
    let called_from_main = curbuf_mut().b_ml.ml_mfp.is_none();

    let fname_opt = curbuf_mut().fname_opt();
    let fname_cstr = fname_opt.unwrap_or(c"");
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
    if called_from_main && !ml_open(curbuf_mut()) {
        getout(1);
    }

    let mut buf = Box::new(FileBuf::new());

    buf.b_ml.ml_mfp = mf_open(Some(fname_used.clone()), O_RDONLY);
    if buf.b_ml.mfp_ref().is_none_or(|m| m.mf_fd < 0) {
        semsg!(tr(c"E306: Cannot open %s"), &fname_used);

        ml_recover_theend(None, Some(buf), serious_error, called_from_main);

        return;
    }
    buf.b_ml.mfp_mut().mf_page_size = MIN_SWAP_PAGE_SIZE;

    let hl_id = HLF_E;
    msg_ext_set_kind(c"emsg");
    hp = mf_get(buf.b_ml.mfp_mut(), 0, 1);
    let Some(hp_b0) = hp else {
        msg_start();
        msg_puts_hl(tr(c"Unable to read block 0 from "), hl_id, true);
        msg_outtrans(
            buf.b_ml
                .mfp_ref()
                .and_then(|m| m.mf_fname.as_deref())
                .unwrap_or(c""),
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
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.mfp_ref().unwrap().bhdr(hp_b0).bh_data);
    if b0p.b0_version[..7] == *b"VIM 3.0" {
        msg_start();
        msg_outtrans(
            buf.b_ml
                .mfp_ref()
                .and_then(|m| m.mf_fname.as_deref())
                .unwrap_or(c""),
            0,
            true,
        );
        msg_puts_hl(tr(c" cannot be used with this version of Nvim.\n"), 0, true);
        msg_puts_hl(tr(c"Use Vim version 3.0.\n"), 0, true);
        msg_end();
        ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
        return;
    }
    if !ml_check_b0_id(&b0p) {
        semsg!(
            tr(c"E307: %s does not look like a Nvim swap file"),
            buf.b_ml
                .mfp_ref()
                .and_then(|m| m.mf_fname.as_deref())
                .unwrap_or(c""),
        );
        ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
        return;
    }
    if b0_magic_wrong(&b0p) {
        msg_start();
        msg_outtrans(
            buf.b_ml
                .mfp_ref()
                .and_then(|m| m.mf_fname.as_deref())
                .unwrap_or(c""),
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
        mf_new_page_size(buf.b_ml.mfp_mut(), b0_page_size);
        let new_page_size = buf.b_ml.mfp_ref().unwrap().mf_page_size;
        if new_page_size < previous_page_size {
            msg_start();
            msg_outtrans(
                buf.b_ml
                    .mfp_ref()
                    .and_then(|m| m.mf_fname.as_deref())
                    .unwrap_or(c""),
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
            let mfp = buf.b_ml.mfp_mut();
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

        expand_env(
            crate::memory::bytes_as_cstr(&b0p.b0_fname),
            g_st().name_buff_bytes_mut(),
        );

        let namebuff_cstr = g_st().name_buff_cstr().to_owned();
        if !setfname(curbuf_mut(), Some(&namebuff_cstr), None, true) {
            ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
            return;
        }
    }

    msg_ext_set_kind(c"wmsg");
    g_st().msg_ext_skip_flush = true;
    let mf_fname_cstr = buf
        .b_ml
        .mfp_ref()
        .and_then(|m| m.mf_fname.as_deref())
        .unwrap_or(c"");
    home_replace(
        None,
        Some(mf_fname_cstr),
        g_st().name_buff_bytes_mut(),
        true,
    );
    smsg!(0, tr(c"Using swap file \"%s\""), g_st().name_buff_cstr());

    if let Some(sp) = buf_spname(curbuf_mut()) {
        crate::memory::copy_bytes_to_c_buf(g_st().name_buff_bytes_mut(), sp.to_bytes());
    } else {
        home_replace(
            None,
            curbuf_mut().ffname_opt(),
            g_st().name_buff_bytes_mut(),
            true,
        );
    }
    msg_putchar(b'\n' as i32);
    smsg!(0, tr(c"Original file \"%s\""), g_st().name_buff_cstr());
    msg_putchar(b'\n' as i32);
    g_st().msg_ext_skip_flush = false;

    let mut org_file_info = FileInfo::new();
    let mut swp_file_info = FileInfo::new();
    let mtime = char_to_long(&b0p.b0_mtime);
    if curbuf_mut().b_ffname.is_some()
        && os_fileinfo(curbuf_mut().ffname_cstr(), &mut org_file_info)
        && ((os_fileinfo(
            buf.b_ml
                .mfp_ref()
                .and_then(|m| m.mf_fname.as_deref())
                .unwrap_or(c""),
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

    mf_put(buf.b_ml.mfp_mut(), hp_b0, false, false);
    hp = None;

    while (curbuf_mut().b_ml.ml_flags & ML_EMPTY) == 0 {
        ml_delete(1);
    }

    if curbuf_mut().b_ffname.is_some() {
        orig_file_status = readfile(
            curbuf_mut().ffname_opt(),
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
        let fenc_obj = Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(cstr_as_string(Some(fenc))),
        };
        set_option_value_give_err(K_OPT_FILEENCODING, fenc_obj, OPT_LOCAL);
    }
    unchanged(curbuf_mut(), true, true);

    let mut bnum: Blocknr = 1;
    let mut page_count: u32 = 1;
    let mut lnum: Linenr = 0;
    let mut line_count: Linenr = 0;
    let mut idx: i32 = 0;
    let mut error: i32 = 0;
    buf.b_ml.ml_stack_top = 0;
    buf.b_ml.ml_stack.clear();

    let mut cannot_open = curbuf_mut().b_ffname.is_none();

    serious_error = false;
    while !g_st().got_int {
        if let Some(hp_id) = hp {
            mf_put(buf.b_ml.mfp_mut(), hp_id, false, false);
        }
        hp = mf_get(buf.b_ml.mfp_mut(), bnum, page_count);
        let Some(hp_id) = hp else {
            if bnum == 1 {
                semsg!(
                    tr(c"E309: Unable to read block 1 from %s"),
                    buf.b_ml
                        .mfp_ref()
                        .and_then(|m| m.mf_fname.as_deref())
                        .unwrap_or(c""),
                );
                ml_recover_theend(None, Some(buf), serious_error, called_from_main);
                return;
            }
            error += 1;
            ml_append(lnum, tr(c"???MANY LINES MISSING").to_bytes(), 0, true);
            lnum += 1;
            if buf.b_ml.ml_stack_top == 0 {
                break;
            }
            buf.b_ml.ml_stack_top -= 1;
            let ip = &(&buf.b_ml.ml_stack)[buf.b_ml.ml_stack_top as usize];
            bnum = ip.ip_bnum;
            idx = ip.ip_index + 1;
            page_count = 1;
            line_breakcheck();
            continue;
        };
        if buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).pb_id() == PTR_ID {
            let mut ptr_block_error = false;
            let max_cnt = pb_count_max(buf.b_ml.mfp_ref().unwrap());
            {
                let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_id);
                if hp_ref.pb_count_max() != max_cnt {
                    ptr_block_error = true;
                    hp_ref.set_pb_count_max(max_cnt);
                }
                if hp_ref.pb_count() > hp_ref.pb_count_max() {
                    ptr_block_error = true;
                    hp_ref.set_pb_count(hp_ref.pb_count_max());
                }
            }
            if ptr_block_error {
                emsg(tr(E_WARNING_POINTER_BLOCK_CORRUPTED));
            }

            let pb_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).pb_count();
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
                                curbuf_mut().ffname_opt(),
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

                let top = ml_add_stack(&mut buf);
                let max_blocknr = buf.b_ml.mfp_ref().unwrap().mf_blocknr_max;
                let ip = &mut (&mut buf.b_ml.ml_stack)[top as usize];
                ip.ip_bnum = bnum;
                ip.ip_index = idx;

                bnum = pe.pe_bnum;
                line_count = pe.pe_line_count;
                page_count = pe.pe_page_count as u32;
                if page_count < 1
                    || bnum.saturating_add(i64::from(page_count)) > max_blocknr.saturating_add(1)
                {
                    error += 1;
                    ml_append(lnum, tr(c"???ILLEGAL BLOCK NUMBER").to_bytes(), 0, true);
                    lnum += 1;
                    idx = ip.ip_index + 1;
                    bnum = ip.ip_bnum;
                    page_count = 1;
                    buf.b_ml.ml_stack_top -= 1;
                    line_breakcheck();
                    continue;
                }
                idx = 0;
                line_breakcheck();
                continue;
            }
        } else if buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).db_id() != DATA_ID {
            if bnum == 1 {
                semsg!(
                    tr(c"E310: Block 1 ID wrong (%s not a .swp file?)"),
                    buf.b_ml
                        .mfp_ref()
                        .and_then(|m| m.mf_fname.as_deref())
                        .unwrap_or(c""),
                );
                ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
                return;
            }
            error += 1;
            ml_append(lnum, tr(c"???BLOCK MISSING").to_bytes(), 0, true);
            lnum += 1;
        } else {
            let mut has_error = false;
            let hp_page_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).bh_page_count;
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
            let mut db_txt_end = buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).db_txt_end();
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

            let mut db_txt_start = buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).db_txt_start();
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

            buf.b_ml.mfp_mut().bhdr_mut(hp_id).bh_data[db_txt_end as usize - 1] = 0;

            let db_line_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).db_line_count();
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

                let idx_val = buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).db_idx(i as usize);
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
                        &buf.b_ml.mfp_ref().unwrap().bhdr(hp_id).bh_data
                            [txt_start as usize..db_txt_end as usize],
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

        if buf.b_ml.ml_stack_top == 0 {
            break;
        }

        buf.b_ml.ml_stack_top -= 1;
        let ip = &buf.b_ml.ml_stack[buf.b_ml.ml_stack_top as usize];

        bnum = ip.ip_bnum;
        idx = ip.ip_index + 1;

        page_count = 1;
        line_breakcheck();
    }

    if orig_file_status != Some(true) || curbuf_mut().b_ml.ml_line_count != lnum * 2 + 1 {
        if !(curbuf_mut().b_ml.ml_line_count == 2 && ml_get_slice(1).is_empty()) {
            changed_internal(curbuf_mut());
            buf_inc_changedtick(curbuf_mut());
        }
    } else {
        for i_lnum in 1..=lnum {
            let p_line = ml_get_slice(i_lnum).to_vec();
            if p_line.as_slice() != ml_get_slice(i_lnum + lnum) {
                changed_internal(curbuf_mut());
                buf_inc_changedtick(curbuf_mut());
                break;
            }
        }
    }

    while curbuf_mut().b_ml.ml_line_count > lnum && (curbuf_mut().b_ml.ml_flags & ML_EMPTY) == 0 {
        ml_delete(curbuf_mut().b_ml.ml_line_count);
    }
    curbuf_mut().b_flags |= BF_RECOVERED;
    check_cursor(curwin_mut());

    g_st().msg_ext_skip_flush = !g_st().got_int;
    g_st().recoverymode = false;
    if g_st().got_int {
        emsg(tr(c"E311: Recovery Interrupted"));
    } else if error != 0 {
        g_st().no_wait_return += 1;
        msg_ext_set_kind(c"emsg");
        msg(c">>>>>>>>>>>>>\n", 0);
        emsg(tr(
            c"E312: Errors detected while recovering; look for lines starting with ???",
        ));
        g_st().no_wait_return -= 1;
        msg_putchar(b'\n' as i32);
        msg(tr(c"See \":help E312\" for more information."), 0);
        msg(c"\n>>>>>>>>>>>>>", 0);
    } else {
        msg_ext_set_kind(c"wmsg");
        if curbuf_mut().b_changed != 0 {
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
        g_st().cmdline_row = g_st().msg_row;
    }
    redraw_curbuf_later(UPD_NOT_VALID);

    ml_recover_theend(hp, Some(buf), serious_error, called_from_main);
}

fn ml_recover_theend(
    hp: Option<BhdrId>,
    mut buf: Option<Box<FileBuf>>,
    serious_error: bool,
    called_from_main: bool,
) {
    g_st().msg_ext_skip_flush = false;
    g_st().recoverymode = false;
    if let Some(ref mut b) = buf
        && let Some(mfp) = b.b_ml.mfp_opt()
    {
        if let Some(hp) = hp {
            mf_put(mfp, hp, false, false);
        }
        mf_close(b.b_ml.ml_mfp.take(), false);
    }
    drop(buf);
    if serious_error && called_from_main {
        ml_close(curbuf_mut(), 1);
    } else {
        apply_autocmds(
            EVENT_BUFREADPOST,
            None,
            curbuf_mut().fname_opt(),
            false,
            curbuf_opt_mut(),
        );
        apply_autocmds(
            EVENT_BUFWINENTER,
            None,
            curbuf_mut().fname_opt(),
            false,
            curbuf_opt_mut(),
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
            let tail = if after_pathsep(dir_bytes) != 0
                && dir_bytes.len() > 1
                && dir_bytes[dir_bytes.len() - 1] == dir_bytes[dir_bytes.len() - 2]
            {
                make_percent_swname(&dir_bytes[..dir_bytes.len() - 1], Some(f_res))
            } else {
                let t = path_tail(f_res);
                Some(concat_fnames(dir_name, cstr_as_string(Some(t)), true))
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
            let name_cstrs: Vec<&std::ffi::CStr> =
                names_vec.iter().map(std::ffi::CString::as_c_str).collect();
            let _ = expand_wildcards_vec(&name_cstrs, &mut files, EW_KEEPALL | EW_FILE | EW_SILENT);
        }

        let cur_mfp_fname = if skip_curbuf {
            curbuf_mut()
                .b_ml
                .mfp_opt()
                .and_then(|mfp| mfp.mf_fname.as_deref())
        } else {
            None
        };

        if let Some(cur_p) = cur_mfp_fname {
            files.retain(|f| !path_equal(cur_p, f, K_PATH_CMP_FULL));
        }

        if dirp.is_empty()
            && tv_list_len(Some(ret_list)) == 0
            && files.is_empty()
            && let Some(f_res) = fname_res
            && let Some(swapname) = modname(Some(f_res), c".swp", true)
            && os_path_exists(&swapname)
            && cur_mfp_fname.is_none_or(|cur_p| !path_equal(cur_p, &swapname, K_PATH_CMP_FULL))
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
    let fname = fix_fname(Some(name.unwrap_or(c"")))?;
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

/// Sync all memlines.
pub fn ml_sync_all(check_file: i32, check_char: i32, do_fsync: bool) {
    for buf in for_all_buffers_ptr() {
        if buf.b_ml.mfp_ref().is_none_or(|m| m.mf_fname.is_none()) {
            continue;
        }
        ml_flush_line(&mut *buf, false);
        ml_find_line(&mut *buf, 0, ML_FLUSH);
        if buf_is_changed_fn(&mut *buf)
            && check_file != 0
            && buf.b_ml.mfp_ref().is_some_and(mf_need_trans)
            && buf.b_ffname.is_some()
        {
            let mut file_info = FileInfo::new();
            if !os_fileinfo(buf.ffname_cstr(), &mut file_info)
                || buf.b_mtime_read != file_info.stat.st_mtim.tv_sec as _
                || buf.b_mtime_read_ns != file_info.stat.st_mtim.tv_nsec as _
                || os_fileinfo_size(&file_info) != buf.b_orig_size
            {
                ml_preserve(&mut *buf, false, do_fsync);
                g_st().did_check_timestamps = false;
                g_st().need_check_timestamps = true;
            }
        }
        if buf
            .b_ml
            .mfp_ref()
            .is_some_and(|m| m.mf_dirty == MF_DIRTY_YES)
        {
            let sync_flags = (if check_char != 0 { MFS_STOP } else { 0 })
                | (if do_fsync && buf_is_changed_fn(&mut *buf) {
                    MFS_FLUSH
                } else {
                    0
                });
            mf_sync(buf.b_ml.mfp_mut(), sync_flags);
            if check_char != 0 && os_char_avail() {
                break;
            }
        }
    }
}

/// Sync one buffer, including negative blocks.
pub fn ml_preserve(buf: &mut FileBuf, message: bool, do_fsync: bool) {
    let got_int_save = g_st().got_int;

    if buf.b_ml.mfp_ref().is_none_or(|m| m.mf_fname.is_none()) {
        if message {
            emsg(tr(c"E313: Cannot preserve, there is no swap file"));
        }
        return;
    };

    g_st().got_int = false;

    ml_flush_line(&mut *buf, false);
    ml_find_line(buf, 0, ML_FLUSH);
    let mut status = mf_sync(
        buf.b_ml.mfp_mut(),
        MFS_ALL | if do_fsync { MFS_FLUSH } else { 0 },
    );

    buf.b_ml.ml_stack_top = 0;

    if buf.b_ml.mfp_ref().is_some_and(mf_need_trans) && !g_st().got_int {
        let mut lnum: Linenr = 1;
        while buf.b_ml.mfp_ref().is_some_and(mf_need_trans) && lnum <= buf.b_ml.ml_line_count {
            if ml_find_line(buf, lnum, ML_FIND).is_none() {
                status = false;
                break;
            }
            lnum = buf.b_ml.ml_locked_high + 1;
        }
        if status {
            ml_find_line(buf, 0, ML_FLUSH);
            if !mf_sync(
                buf.b_ml.mfp_mut(),
                MFS_ALL | if do_fsync { MFS_FLUSH } else { 0 },
            ) {
                status = false;
            }
            buf.b_ml.ml_stack_top = 0;
        }
    }

    g_st().got_int |= got_int_save;

    if message {
        if status {
            msg(tr(c"File preserved"), 0);
        } else {
            emsg(tr(c"E314: Preserve failed"));
        }
    }
}

/// Return a reference to a (read-only copy of a) line in `curbuf`.
pub fn ml_get(lnum: Linenr) -> &'static CStr {
    ml_get_buf_impl(curbuf_mut(), lnum, false)
}

/// Return a reference to a (read-only copy of a) line in `buf`.
pub fn ml_get_buf(buf: &mut FileBuf, lnum: Linenr) -> &CStr {
    ml_get_buf_impl(buf, lnum, false)
}

/// Return the bytes (excluding NUL) of a line in `curbuf`.
#[inline(always)]
pub fn ml_get_slice(lnum: Linenr) -> &'static [u8] {
    ml_get_buf_slice(curbuf_mut(), lnum)
}

/// Return the bytes (excluding NUL) of a line in `buf`.
#[inline(always)]
pub fn ml_get_buf_slice(buf: &mut FileBuf, lnum: Linenr) -> &[u8] {
    ml_get_buf(buf, lnum).to_bytes()
}

/// Return the bytes (excluding NUL) starting at position `pos` in `curbuf`.
#[inline(always)]
pub fn ml_get_pos_slice(pos: &CursorPos) -> &'static [u8] {
    let line = ml_get_buf_slice(curbuf_mut(), pos.lnum);
    let col = (pos.col.max(0) as usize).min(line.len());
    &line[col..]
}

/// Return a mutable byte slice (excluding NUL) of a line in `buf`, marking its data block dirty.
#[inline(always)]
pub fn ml_get_buf_mut_slice(buf: &mut FileBuf, lnum: Linenr) -> &mut [u8] {
    if ml_get_buf_impl(buf, lnum, true).is_empty() {
        return &mut [];
    }
    assert!(buf.b_ml.ml_line_textlen > 0);
    let len = (buf.b_ml.ml_line_textlen - 1) as usize;
    if (buf.b_ml.ml_flags & ML_LINE_DIRTY) != 0 {
        let s = buf.b_ml.ml_line_buf.as_mut_slice();
        let slen = s.len();
        &mut s[..len.min(slen)]
    } else if buf.b_ml.ml_line_start == usize::MAX || buf.b_ml.ml_locked.is_none() {
        buf.b_ml.ml_line_buf = b"???\0".to_vec();
        let s = buf.b_ml.ml_line_buf.as_mut_slice();
        &mut s[..len.min(3)]
    } else {
        let start = buf.b_ml.ml_line_start;
        let s = buf.b_ml.locked_bhdr_mut().unwrap().bh_data.as_mut_slice();
        let slen = s.len();
        let start = start.min(slen);
        &mut s[start..(start + len).min(slen)]
    }
}

/// Return reference to CStr starting at position `pos`.
pub fn ml_get_pos(pos: &CursorPos) -> &'static CStr {
    let line = ml_get_buf(crate::globals::curbuf_mut(), pos.lnum);
    let col = (pos.col.max(0) as usize).min(line.to_bytes().len());
    &line[col..]
}

/// Return length (excluding NUL) of the given line.
pub fn ml_get_len(lnum: Linenr) -> Colnr {
    ml_get_buf_len(crate::globals::curbuf_mut(), lnum)
}

/// Return length (excluding NUL) of the text after position `pos`.
pub fn ml_get_pos_len(pos: &mut CursorPos) -> Colnr {
    ml_get_buf_len(crate::globals::curbuf_mut(), pos.lnum) - pos.col
}

/// Return length (excluding NUL) of the given line in `buf`.
pub fn ml_get_buf_len(buf: &mut FileBuf, lnum: Linenr) -> Colnr {
    ml_get_buf_slice(buf, lnum).len() as Colnr
}

/// Gets the charwise NL-joined text of range `[start, end)` as a `Vec<u8>`.
pub fn ml_region_text(buf: &mut FileBuf, start: CursorPos, end: CursorPos) -> Vec<u8> {
    let mut sb = Vec::<u8>::new();
    for lnum in start.lnum..=end.lnum {
        let line = ml_get_buf_slice(buf, lnum);
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
        if lnum < end.lnum {
            sb.push(NL as u8);
        }
    }
    sb
}

/// Return codepoint at `pos`.
pub fn gchar_pos(pos: &mut CursorPos) -> i32 {
    if pos.col == MAXCOL || pos.col > ml_get_len(pos.lnum) {
        return i32::from(NUL);
    }
    crate::mbyte::utf_slice2char(ml_get_pos_slice(pos))
}

fn ml_get_buf_impl(buf: &mut FileBuf, mut lnum: Linenr, will_change: bool) -> &CStr {
    if buf.b_ml.ml_mfp.is_none() {
        buf.b_ml.ml_line_textlen = 1;
        return c"";
    }

    if lnum > buf.b_ml.ml_line_count {
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
        return ml_get_errorret(buf, lnum);
    }
    lnum = lnum.max(1);

    if buf.b_ml.ml_line_lnum != lnum {
        ml_flush_line(buf, false);

        let Some(hp) = ml_find_line(buf, lnum, ML_FIND) else {
            if ml_st(|st| {
                let r = st.ml_get_recursive.get();
                if r == 0 {
                    st.ml_get_recursive.set(1);
                }
                r == 0
            }) {
                get_trans_bufname(buf);
                shorten_dir(g_st().name_buff_bytes_mut());
                siemsg!(
                    tr(E_ML_GET_CANNOT_FIND_LINE_NR_IN_BUFFER_NR_STR),
                    i64::from(lnum),
                    buf.b_fnum,
                    &g_st().name_buff,
                );
                ml_st(|st| st.ml_get_recursive.set(st.ml_get_recursive.get() - 1));
            }
            return ml_get_errorret(buf, lnum);
        };
        let idx = (lnum - buf.b_ml.ml_locked_low) as usize;
        let hp_ref = buf.b_ml.mfp_ref().unwrap().bhdr(hp);
        let start = hp_ref.db_idx(idx) & DB_INDEX_MASK;
        let end = if idx == 0 {
            hp_ref.db_txt_end()
        } else {
            hp_ref.db_idx(idx - 1) & DB_INDEX_MASK
        };

        let len = (end - start) as usize;
        buf.b_ml.ml_line_start = start as usize;
        buf.b_ml.ml_line_textlen = len as Colnr;
        buf.b_ml.ml_line_lnum = lnum;
        buf.b_ml.ml_flags &= !(ML_LINE_DIRTY | ML_LINE_INPLACE);
    }
    if will_change {
        buf.b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS | ML_LINE_INPLACE;
        let deleted_len = crate::memory::bytes_until_nul(buf.b_ml.current_line_slice()).len();
        if g_st().inhibit_delete_count == 0 {
            buf.deleted_bytes += deleted_len + 1;
            buf.deleted_bytes2 += deleted_len + 1;
            if buf.update_need_codepoints {
                let mut cp = buf.deleted_codepoints;
                let mut cu = buf.deleted_codeunits;
                mb_utflen(
                    &buf.b_ml.current_line_slice()[..deleted_len],
                    deleted_len,
                    &mut cp,
                    &mut cu,
                );
                buf.deleted_codepoints = cp + 1;
                buf.deleted_codeunits = cu + 1;
            }
        }
    } else if (buf.b_ml.ml_flags & ML_LINE_INPLACE) == 0
        && buf.b_ml.current_line_slice().last() == Some(&0)
    {
        return crate::memory::cstr_from_bytes_with_nul(buf.b_ml.current_line_slice());
    }

    crate::memory::bytes_as_cstr(buf.b_ml.current_line_slice())
}

fn ml_get_errorret(buf: &mut FileBuf, lnum: Linenr) -> &'static CStr {
    buf.b_ml.ml_line_start = usize::MAX;
    buf.b_ml.ml_line_textlen = 4;
    buf.b_ml.ml_line_lnum = lnum;
    c"???"
}

/// Check if a line that was just obtained by `ml_get` is in allocated memory.
pub fn ml_line_alloced() -> i32 {
    curbuf_mut().b_ml.ml_flags & ML_LINE_DIRTY
}

fn ml_append_int(buf: &mut FileBuf, lnum: Linenr, line: &[u8], len_arg: Colnr, flags: i32) -> bool {
    let mut len = len_arg;

    if lnum > buf.b_ml.ml_line_count || buf.b_ml.ml_mfp.is_none() {
        return false;
    }

    ml_st(|st| {
        if st.lowest_marked.get() != 0 && st.lowest_marked.get() > lnum {
            st.lowest_marked.set(lnum + 1);
        }
    });

    if len == 0 {
        len = crate::memory::bytes_until_nul(line).len() as Colnr + 1;
    }
    let mut space_needed = i64::from(len) + INDEX_SIZE as i64;

    let page_size = i64::from(buf.b_ml.mfp_ref().expect("ml_mfp is None").mf_page_size);

    let Some(mut hp) = ml_find_line(buf, if lnum == 0 { 1 } else { lnum }, ML_INSERT) else {
        return false;
    };

    buf.b_ml.ml_flags &= !ML_EMPTY;

    let mut db_idx: i32 = if lnum == 0 {
        -1
    } else {
        lnum - buf.b_ml.ml_locked_low
    };
    let mut line_count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low;

    if i64::from(buf.b_ml.mfp_ref().unwrap().bhdr(hp).db_free()) < space_needed
        && db_idx == line_count - 1
        && lnum < buf.b_ml.ml_line_count
    {
        buf.b_ml.ml_locked_lineadd -= 1;
        buf.b_ml.ml_locked_high -= 1;
        let Some(hp_next) = ml_find_line(buf, lnum + 1, ML_INSERT) else {
            return false;
        };
        hp = hp_next;

        db_idx = -1;
        line_count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low;
    }

    if buf.b_prev_line_count == 0 {
        buf.b_prev_line_count = buf.b_ml.ml_line_count;
    }
    buf.b_ml.ml_line_count += 1;

    if i64::from(buf.b_ml.mfp_ref().unwrap().bhdr(hp).db_free()) >= space_needed {
        let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
        let new_txt_start = hp_ref.db_txt_start() - len as u32;
        hp_ref.set_db_txt_start(new_txt_start);
        hp_ref.set_db_free(hp_ref.db_free() - space_needed as u32);
        hp_ref.set_db_line_count(hp_ref.db_line_count() + 1);

        if line_count > db_idx + 1 {
            let offset = if db_idx < 0 {
                hp_ref.db_txt_end() as i32
            } else {
                (hp_ref.db_idx(db_idx as usize) & DB_INDEX_MASK) as i32
            };
            let txt_start = new_txt_start as usize;
            bh_copy_within(
                hp_ref,
                txt_start + len as usize,
                txt_start,
                offset as usize - (txt_start + len as usize),
            );
            let mut i = line_count - 1;
            while i > db_idx {
                let prev = hp_ref.db_idx(i as usize);
                hp_ref.set_db_idx((i + 1) as usize, prev - len as u32);
                i -= 1;
            }
            hp_ref.set_db_idx((db_idx + 1) as usize, (offset - len) as u32);
        } else {
            hp_ref.set_db_idx((db_idx + 1) as usize, new_txt_start);
        }

        let dst_off = hp_ref.db_idx((db_idx + 1) as usize) as usize;
        bh_write_bytes(hp_ref, dst_off, line, len as usize);
        if (flags & ML_APPEND_MARK) != 0 {
            let cur = hp_ref.db_idx((db_idx + 1) as usize);
            hp_ref.set_db_idx((db_idx + 1) as usize, cur | DB_MARKED);
        }

        buf.b_ml.ml_flags |= ML_LOCKED_DIRTY;
        if (flags & ML_APPEND_NEW) == 0 {
            buf.b_ml.ml_flags |= ML_LOCKED_POS;
        }
    } else {
        let mut data_moved: i32 = 0;
        let mut total_moved: i32 = 0;
        let lines_moved: i32;
        let in_left: bool;

        if db_idx < 0 {
            lines_moved = 0;
            in_left = true;
        } else {
            lines_moved = line_count - db_idx - 1;
            if lines_moved == 0 {
                in_left = false;
            } else {
                let hp_ref = buf.b_ml.mfp_ref().unwrap().bhdr(hp);
                data_moved = ((hp_ref.db_idx(db_idx as usize) & DB_INDEX_MASK)
                    - hp_ref.db_txt_start()) as i32;
                total_moved = data_moved + lines_moved * (INDEX_SIZE as i32);
                if i64::from(hp_ref.db_free()) + i64::from(total_moved) >= space_needed {
                    in_left = true;
                    space_needed = i64::from(total_moved);
                } else {
                    in_left = false;
                    space_needed += i64::from(total_moved);
                }
            }
        }

        let new_page_count = ((space_needed + HEADER_SIZE as i64) + page_size - 1) / page_size;
        let hp_new = ml_new_data(
            buf.b_ml.mfp_mut(),
            (flags & ML_APPEND_NEW) != 0,
            new_page_count,
        );
        let hp_left;
        let hp_right;
        let mut line_count_left: i32;
        let mut line_count_right: i32;
        if db_idx < 0 {
            hp_left = hp_new;
            hp_right = hp;
            line_count_left = 0;
            line_count_right = line_count;
        } else {
            hp_left = hp;
            hp_right = hp_new;
            line_count_left = line_count;
            line_count_right = 0;
        }
        let mut bnum_left = buf.b_ml.mfp_ref().unwrap().bhdr(hp_left).bh_bnum;
        let mut bnum_right = buf.b_ml.mfp_ref().unwrap().bhdr(hp_right).bh_bnum;
        let mut page_count_left = buf.b_ml.mfp_ref().unwrap().bhdr(hp_left).bh_page_count as i32;
        let mut page_count_right = buf.b_ml.mfp_ref().unwrap().bhdr(hp_right).bh_page_count as i32;

        if !in_left {
            let hp_right_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_right);
            let txt_start = hp_right_ref.db_txt_start() - len as u32;
            hp_right_ref.set_db_txt_start(txt_start);
            hp_right_ref.set_db_free(hp_right_ref.db_free() - (len as u32 + INDEX_SIZE as u32));
            let idx0 = if (flags & ML_APPEND_MARK) != 0 {
                txt_start | DB_MARKED
            } else {
                txt_start
            };
            hp_right_ref.set_db_idx(0, idx0);
            bh_write_bytes(hp_right_ref, txt_start as usize, line, len as usize);
            line_count_right += 1;
        }

        if lines_moved != 0 {
            let (src_start, dst_start, offset) = {
                let mfp = buf.b_ml.mfp_mut();
                let right_txt_start = mfp.bhdr(hp_right).db_txt_start() - data_moved as u32;
                let right_free = mfp.bhdr(hp_right).db_free() - total_moved as u32;
                mfp.bhdr_mut(hp_right).set_db_txt_start(right_txt_start);
                mfp.bhdr_mut(hp_right).set_db_free(right_free);

                let left_txt_start = mfp.bhdr(hp_left).db_txt_start();
                let left_free = mfp.bhdr(hp_left).db_free();
                let offset = (right_txt_start as i32).wrapping_sub(left_txt_start as i32);
                mfp.bhdr_mut(hp_left)
                    .set_db_txt_start(left_txt_start + data_moved as u32);
                mfp.bhdr_mut(hp_left)
                    .set_db_free(left_free + total_moved as u32);
                (
                    left_txt_start as usize,
                    right_txt_start as usize,
                    offset as u32,
                )
            };
            buf.b_ml.mfp_mut().copy_bhdr_data(
                hp_left,
                src_start,
                hp_right,
                dst_start,
                data_moved as usize,
            );

            let mut to = line_count_right;
            let mut from = db_idx + 1;
            while from < line_count_left {
                let from_val = buf.b_ml.mfp_ref().unwrap().bhdr(hp).db_idx(from as usize);
                buf.b_ml
                    .mfp_mut()
                    .bhdr_mut(hp_right)
                    .set_db_idx(to as usize, from_val.wrapping_add(offset));
                from += 1;
                to += 1;
            }
            line_count_right += lines_moved;
            line_count_left -= lines_moved;
        }

        if in_left {
            let hp_left_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_left);
            let txt_start = hp_left_ref.db_txt_start() - len as u32;
            hp_left_ref.set_db_txt_start(txt_start);
            hp_left_ref.set_db_free(hp_left_ref.db_free() - (len as u32 + INDEX_SIZE as u32));
            let idx_val = if (flags & ML_APPEND_MARK) != 0 {
                txt_start | DB_MARKED
            } else {
                txt_start
            };
            hp_left_ref.set_db_idx(line_count_left as usize, idx_val);
            bh_write_bytes(hp_left_ref, txt_start as usize, line, len as usize);
            line_count_left += 1;
        }

        let mut lnum_left: Linenr;
        let mut lnum_right: Linenr;
        if db_idx < 0 {
            lnum_left = lnum + 1;
            lnum_right = 0;
        } else {
            lnum_left = 0;
            if in_left {
                lnum_right = lnum + 2;
            } else {
                lnum_right = lnum + 1;
            }
        }
        buf.b_ml
            .mfp_mut()
            .bhdr_mut(hp_left)
            .set_db_line_count(i64::from(line_count_left));
        buf.b_ml
            .mfp_mut()
            .bhdr_mut(hp_right)
            .set_db_line_count(i64::from(line_count_right));

        if lines_moved != 0 || in_left {
            buf.b_ml.ml_flags |= ML_LOCKED_DIRTY;
        }
        if (flags & ML_APPEND_NEW) == 0 && db_idx >= 0 && in_left {
            buf.b_ml.ml_flags |= ML_LOCKED_POS;
        }
        mf_put(buf.b_ml.mfp_mut(), hp_new, true, false);

        let lineadd = buf.b_ml.ml_locked_lineadd;
        buf.b_ml.ml_locked_lineadd = 0;
        ml_find_line(buf, 0, ML_FLUSH);

        let mut stack_idx = buf.b_ml.ml_stack_top - 1;
        while stack_idx >= 0 {
            let ip = buf.b_ml.ml_stack[stack_idx as usize];
            let pb_idx = ip.ip_index as usize;
            let Some(mut hp_ptr) = mf_get(buf.b_ml.mfp_mut(), ip.ip_bnum, 1) else {
                return false;
            };
            if buf.b_ml.mfp_ref().unwrap().bhdr(hp_ptr).pb_id() != PTR_ID {
                iemsg(tr(E_POINTER_BLOCK_ID_WRONG_THREE));
                mf_put(buf.b_ml.mfp_mut(), hp_ptr, false, false);
                return false;
            }
            let pb_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_ptr).pb_count();
            let pb_max = buf.b_ml.mfp_ref().unwrap().bhdr(hp_ptr).pb_count_max();
            if pb_count < pb_max {
                {
                    let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_ptr);
                    if pb_idx + 1 < pb_count as usize {
                        hp_ref.pb_copy_entries(
                            pb_idx + 1,
                            pb_idx + 2,
                            pb_count as usize - pb_idx - 1,
                        );
                    }
                    hp_ref.set_pb_count(pb_count + 1);
                    let mut pe_cur = hp_ref.pb_entry(pb_idx);
                    pe_cur.pe_line_count = line_count_left;
                    pe_cur.pe_bnum = bnum_left;
                    pe_cur.pe_page_count = page_count_left;
                    if lnum_left != 0 {
                        pe_cur.pe_old_lnum = lnum_left;
                    }
                    hp_ref.set_pb_entry(pb_idx, pe_cur);

                    let mut pe_next = hp_ref.pb_entry(pb_idx + 1);
                    pe_next.pe_line_count = line_count_right;
                    pe_next.pe_bnum = bnum_right;
                    pe_next.pe_page_count = page_count_right;
                    if lnum_right != 0 {
                        pe_next.pe_old_lnum = lnum_right;
                    }
                    hp_ref.set_pb_entry(pb_idx + 1, pe_next);
                }

                mf_put(buf.b_ml.mfp_mut(), hp_ptr, true, false);
                buf.b_ml.ml_stack_top = stack_idx + 1;

                if lineadd != 0 {
                    buf.b_ml.ml_stack_top -= 1;
                    ml_lineadd(buf, lineadd);
                    buf.b_ml.ml_stack[buf.b_ml.ml_stack_top as usize].ip_high += lineadd;
                    buf.b_ml.ml_stack_top += 1;
                }

                break;
            }

            let mut hp_new_ptr: BhdrId;
            loop {
                hp_new_ptr = ml_new_ptr(buf.b_ml.mfp_mut());

                if buf.b_ml.mfp_ref().unwrap().bhdr(hp_ptr).bh_bnum != 1 {
                    break;
                }

                buf.b_ml
                    .mfp_mut()
                    .copy_bhdr_data(hp_ptr, 0, hp_new_ptr, 0, page_size as usize);
                let new_bnum = buf.b_ml.mfp_ref().unwrap().bhdr(hp_new_ptr).bh_bnum;
                let total_lines = buf.b_ml.ml_line_count;
                {
                    let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_ptr);
                    hp_ref.set_pb_count(1);
                    hp_ref.set_pb_entry(
                        0,
                        PointerEntry {
                            pe_bnum: new_bnum,
                            pe_line_count: total_lines,
                            pe_old_lnum: 1,
                            pe_page_count: 1,
                        },
                    );
                }
                mf_put(buf.b_ml.mfp_mut(), hp_ptr, true, false);
                hp_ptr = hp_new_ptr;
                buf.b_ml.ml_stack[stack_idx as usize].ip_index = 0;
                stack_idx += 1;
            }

            let cur_pb_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_ptr).pb_count();
            total_moved = i32::from(cur_pb_count) - (pb_idx as i32) - 1;
            if total_moved != 0 {
                for k in 0..(total_moved as usize) {
                    let entry = buf
                        .b_ml
                        .mfp_ref()
                        .unwrap()
                        .bhdr(hp_ptr)
                        .pb_entry(pb_idx + 1 + k);
                    buf.b_ml
                        .mfp_mut()
                        .bhdr_mut(hp_new_ptr)
                        .set_pb_entry(k, entry);
                }
                buf.b_ml
                    .mfp_mut()
                    .bhdr_mut(hp_new_ptr)
                    .set_pb_count(total_moved as u16);
                let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_ptr);
                hp_ref.set_pb_count((i32::from(cur_pb_count) - (total_moved - 1)) as u16);
                let mut pe_next = hp_ref.pb_entry(pb_idx + 1);
                pe_next.pe_bnum = bnum_right;
                pe_next.pe_line_count = line_count_right;
                pe_next.pe_page_count = page_count_right;
                if lnum_right != 0 {
                    pe_next.pe_old_lnum = lnum_right;
                }
                hp_ref.set_pb_entry(pb_idx + 1, pe_next);
            } else {
                let hp_new_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_new_ptr);
                hp_new_ref.set_pb_count(1);
                hp_new_ref.set_pb_entry(
                    0,
                    PointerEntry {
                        pe_bnum: bnum_right,
                        pe_line_count: line_count_right,
                        pe_page_count: page_count_right,
                        pe_old_lnum: lnum_right,
                    },
                );
            }
            {
                let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp_ptr);
                let mut pe_cur = hp_ref.pb_entry(pb_idx);
                pe_cur.pe_bnum = bnum_left;
                pe_cur.pe_line_count = line_count_left;
                pe_cur.pe_page_count = page_count_left;
                if lnum_left != 0 {
                    pe_cur.pe_old_lnum = lnum_left;
                }
                hp_ref.set_pb_entry(pb_idx, pe_cur);
            }
            lnum_left = 0;
            lnum_right = 0;

            line_count_right = 0;
            let new_pb_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_new_ptr).pb_count();
            for i in 0..(new_pb_count as usize) {
                line_count_right += buf
                    .b_ml
                    .mfp_ref()
                    .unwrap()
                    .bhdr(hp_new_ptr)
                    .pb_entry(i)
                    .pe_line_count;
            }
            line_count_left = 0;
            let left_pb_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_ptr).pb_count();
            for i in 0..(left_pb_count as usize) {
                line_count_left += buf
                    .b_ml
                    .mfp_ref()
                    .unwrap()
                    .bhdr(hp_ptr)
                    .pb_entry(i)
                    .pe_line_count;
            }

            bnum_left = buf.b_ml.mfp_ref().unwrap().bhdr(hp_ptr).bh_bnum;
            bnum_right = buf.b_ml.mfp_ref().unwrap().bhdr(hp_new_ptr).bh_bnum;
            page_count_left = 1;
            page_count_right = 1;
            mf_put(buf.b_ml.mfp_mut(), hp_ptr, true, false);
            mf_put(buf.b_ml.mfp_mut(), hp_new_ptr, true, false);

            stack_idx -= 1;
        }

        if stack_idx < 0 {
            iemsg(tr(c"E318: Updated too many blocks?"));
            buf.b_ml.ml_stack_top = 0;
        }
    }

    ml_updatechunk(buf, lnum + 1, len, ML_CHNK_ADDLINE);
    true
}

fn ml_append_flush(buf: &mut FileBuf, lnum: Linenr, line: &[u8], len: Colnr, flags: i32) -> bool {
    if lnum > buf.b_ml.ml_line_count {
        return false;
    }
    if buf.b_ml.ml_line_lnum != 0 {
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
    if curbuf_mut().b_ml.ml_mfp.is_none() && !open_buffer(false, None, 0) {
        return false;
    }
    ml_append_flush(curbuf_mut(), lnum, line, len, flags)
}

/// Like `ml_append()` but for an arbitrary buffer.
pub fn ml_append_buf(
    buf: &mut FileBuf,
    lnum: Linenr,
    line: &[u8],
    len: Colnr,
    newfile: bool,
) -> bool {
    if buf.b_ml.ml_mfp.is_none() {
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

pub fn ml_add_deleted_len(bytes: &[u8]) {
    ml_add_deleted_len_buf(curbuf_mut(), bytes);
}

pub fn ml_add_deleted_len_buf(buf: &mut FileBuf, bytes: &[u8]) {
    if g_st().inhibit_delete_count != 0 {
        return;
    }
    let len = bytes.len();
    buf.deleted_bytes += len + 1;
    buf.deleted_bytes2 += len + 1;
    if buf.update_need_codepoints {
        mb_utflen(
            bytes,
            len,
            &mut buf.deleted_codepoints,
            &mut buf.deleted_codeunits,
        );
        buf.deleted_codepoints += 1;
        buf.deleted_codeunits += 1;
    }
}

/// Replace line `lnum`, with buffering, in current buffer.
pub fn ml_replace(lnum: Linenr, line: &[u8], copy: bool) -> bool {
    ml_replace_buf(curbuf_mut(), lnum, line, copy, false)
}

/// Replace a line for the current buffer with explicit `len` (excluding NUL).
pub fn ml_replace_len(lnum: Linenr, line: &[u8], len: usize, copy: bool) -> bool {
    ml_replace_buf_len(curbuf_mut(), lnum, line, len, copy, false)
}

pub fn ml_replace_buf(
    buf: &mut FileBuf,
    lnum: Linenr,
    line: &[u8],
    copy: bool,
    noalloc: bool,
) -> bool {
    let len = crate::memory::bytes_until_nul(line).len();
    ml_replace_buf_len(buf, lnum, line, len, copy, noalloc)
}

/// Replace line `lnum`, with buffering.
pub fn ml_replace_buf_len(
    buf: &mut FileBuf,
    lnum: Linenr,
    line_arg: &[u8],
    len_arg: usize,
    copy: bool,
    noalloc: bool,
) -> bool {
    if buf.b_ml.ml_mfp.is_none() && !open_buffer(false, None, 0) {
        return false;
    }

    let new_buf = if copy {
        assert!(!noalloc);
        let mut v = Vec::with_capacity(len_arg + 1);
        let copy_len = len_arg.min(line_arg.len());
        if copy_len > 0 {
            v.extend_from_slice(&line_arg[..copy_len]);
        }
        if len_arg > copy_len {
            v.resize(len_arg, 0);
        }
        v.push(0);
        Some(v)
    } else {
        assert!(noalloc);
        None
    };

    if buf.b_ml.ml_line_lnum != lnum {
        ml_flush_line(buf, false);
    }

    if !buf.update_callbacks.is_empty() {
        let old_line = ml_get_buf(buf, lnum).to_bytes().to_vec();
        ml_add_deleted_len_buf(buf, &old_line);
    }

    if let Some(v) = new_buf {
        buf.b_ml.ml_line_buf = v;
    } else {
        buf.b_ml.ml_line_buf.clear();
    }

    buf.b_ml.ml_line_textlen = (len_arg as Colnr) + 1;
    buf.b_ml.ml_line_lnum = lnum;
    buf.b_ml.ml_flags = (buf.b_ml.ml_flags | ML_LINE_DIRTY) & !ML_EMPTY;
    if noalloc {
        ml_flush_line_impl(buf, Some(&line_arg[..len_arg.min(line_arg.len())]));
    }

    true
}

/// Replace the in-memory dirty line buffer in `buf` with `bytes` (without trailing NUL).
pub fn ml_replace_dirty_line(buf: &mut FileBuf, bytes: &[u8], clear_empty: bool) {
    let mut new_buf = Vec::with_capacity(bytes.len() + 1);
    new_buf.extend_from_slice(bytes);
    new_buf.push(0);
    buf.b_ml.ml_line_buf = new_buf;
    buf.b_ml.ml_line_textlen = (bytes.len() as Colnr) + 1;
    buf.b_ml.ml_flags |= ML_LINE_DIRTY;
    if clear_empty {
        buf.b_ml.ml_flags &= !ML_EMPTY;
    }
}

/// Delete line `lnum` in `buf`.
pub fn ml_delete_buf(buf: &mut FileBuf, lnum: Linenr, message: bool) -> bool {
    ml_flush_line(buf, false);
    ml_delete_int(buf, lnum, if message { ML_DEL_MESSAGE } else { 0 })
}

fn ml_delete_int(buf: &mut FileBuf, lnum: Linenr, flags: i32) -> bool {
    ml_st(|st| {
        let low = st.lowest_marked.get();
        if low != 0 && low > lnum {
            st.lowest_marked.set(low - 1);
        }
    });

    if buf.b_ml.ml_line_count == 1 {
        if (flags & ML_DEL_MESSAGE) != 0 {
            set_keep_msg(Some(gettext(NO_LINES_MSG)), 0);
        }
        let i = ml_replace_buf(buf, 1, b"", true, false);
        buf.b_ml.ml_flags |= ML_EMPTY;
        return i;
    }

    if buf.b_ml.mfp_ref().is_none() {
        return false;
    }

    let Some(hp) = ml_find_line(buf, lnum, ML_DELETE) else {
        return false;
    };

    let mut count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low + 2;
    let mut idx = lnum - buf.b_ml.ml_locked_low;

    if buf.b_prev_line_count == 0 {
        buf.b_prev_line_count = buf.b_ml.ml_line_count;
    }
    buf.b_ml.ml_line_count -= 1;

    let (line_start, line_size) = {
        let hp_ref = buf.b_ml.mfp_ref().unwrap().bhdr(hp);
        let line_start = (hp_ref.db_idx(idx as usize) & DB_INDEX_MASK) as i32;
        let line_size: i32 = if idx == 0 {
            (hp_ref.db_txt_end() as i32) - line_start
        } else {
            ((hp_ref.db_idx((idx - 1) as usize) & DB_INDEX_MASK) as i32) - line_start
        };
        (line_start, line_size)
    };

    assert!(line_size >= 1);
    let start = line_start as usize;
    let end = start + (line_size - 1) as usize;
    if g_st().inhibit_delete_count == 0 {
        let len = end - start;
        buf.deleted_bytes += len + 1;
        buf.deleted_bytes2 += len + 1;
        if buf.update_need_codepoints {
            let mut cp = buf.deleted_codepoints;
            let mut cu = buf.deleted_codeunits;
            mb_utflen(
                &buf.b_ml.mfp_ref().unwrap().bhdr(hp).bh_data[start..end],
                len,
                &mut cp,
                &mut cu,
            );
            buf.deleted_codepoints = cp + 1;
            buf.deleted_codeunits = cu + 1;
        }
    }

    if count == 1 {
        mf_free(buf.b_ml.mfp_mut(), hp);
        buf.b_ml.ml_locked = None;

        let mut stack_idx = buf.b_ml.ml_stack_top - 1;
        while stack_idx >= 0 {
            buf.b_ml.ml_stack_top = 0;
            let ip = buf.b_ml.ml_stack[stack_idx as usize];
            idx = ip.ip_index;
            let Some(hp_cur) = mf_get(buf.b_ml.mfp_mut(), ip.ip_bnum, 1) else {
                return false;
            };
            if buf.b_ml.mfp_ref().unwrap().bhdr(hp_cur).pb_id() != PTR_ID {
                iemsg(tr(E_POINTER_BLOCK_ID_WRONG_FOUR));
                mf_put(buf.b_ml.mfp_mut(), hp_cur, false, false);
                return false;
            }
            let pb_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp_cur).pb_count() - 1;
            buf.b_ml.mfp_mut().bhdr_mut(hp_cur).set_pb_count(pb_count);
            count = i32::from(pb_count);
            if count == 0 {
                mf_free(buf.b_ml.mfp_mut(), hp_cur);
            } else {
                if count != idx {
                    buf.b_ml.mfp_mut().bhdr_mut(hp_cur).pb_copy_entries(
                        (idx + 1) as usize,
                        idx as usize,
                        (count - idx) as usize,
                    );
                }
                mf_put(buf.b_ml.mfp_mut(), hp_cur, true, false);

                buf.b_ml.ml_stack_top = stack_idx;
                if buf.b_ml.ml_locked_lineadd != 0 {
                    ml_lineadd(buf, buf.b_ml.ml_locked_lineadd);
                    buf.b_ml.ml_stack[buf.b_ml.ml_stack_top as usize].ip_high +=
                        buf.b_ml.ml_locked_lineadd;
                }
                buf.b_ml.ml_stack_top += 1;
                break;
            }
            stack_idx -= 1;
        }
    } else {
        let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
        let text_start = hp_ref.db_txt_start() as i32;
        bh_copy_within(
            hp_ref,
            text_start as usize,
            (text_start + line_size) as usize,
            (line_start - text_start) as usize,
        );

        for i in idx..(count - 1) {
            let next_val = hp_ref.db_idx((i + 1) as usize);
            hp_ref.set_db_idx(i as usize, next_val + (line_size as u32));
        }

        hp_ref.set_db_free(hp_ref.db_free() + (line_size as u32) + (INDEX_SIZE as u32));
        hp_ref.set_db_txt_start(hp_ref.db_txt_start() + line_size as u32);
        hp_ref.set_db_line_count(hp_ref.db_line_count() - 1);

        buf.b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
    }

    ml_updatechunk(buf, lnum, line_size, ML_CHNK_DELLINE);
    true
}

/// Delete line `lnum` in the current buffer.
pub fn ml_delete(lnum: Linenr) -> bool {
    ml_delete_flags(lnum, 0)
}

/// Like `ml_delete()` but using flags.
pub fn ml_delete_flags(lnum: Linenr, flags: i32) -> bool {
    ml_flush_line(curbuf_mut(), false);
    if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count {
        return false;
    }
    ml_delete_int(curbuf_mut(), lnum, flags)
}

/// Set the `DB_MARKED` flag for line `lnum`.
pub fn ml_setmarked(lnum: Linenr) {
    if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count || curbuf_mut().b_ml.ml_mfp.is_none() {
        return;
    }
    ml_st(|st| {
        let low = st.lowest_marked.get();
        if low == 0 || low > lnum {
            st.lowest_marked.set(lnum);
        }
    });

    let Some(hp) = ml_find_line(curbuf_mut(), lnum, ML_FIND) else {
        return;
    };
    let idx = (lnum - curbuf_mut().b_ml.ml_locked_low) as usize;
    let hp_ref = curbuf_mut().b_ml.mfp_mut().bhdr_mut(hp);
    hp_ref.set_db_idx(idx, hp_ref.db_idx(idx) | DB_MARKED);
    curbuf_mut().b_ml.ml_flags |= ML_LOCKED_DIRTY;
}

/// Find the first line with its `DB_MARKED` flag set.
pub fn ml_firstmarked() -> Linenr {
    let mut lnum = ml_st(|st| st.lowest_marked.get());
    if curbuf_mut().b_ml.ml_mfp.is_none() || lnum == 0 {
        return 0;
    }

    while lnum <= curbuf_mut().b_ml.ml_line_count {
        let Some(hp) = ml_find_line(curbuf_mut(), lnum, ML_FIND) else {
            return 0;
        };
        let mut i = (lnum - curbuf_mut().b_ml.ml_locked_low) as usize;
        while lnum <= curbuf_mut().b_ml.ml_locked_high {
            let idx_val = curbuf_mut().b_ml.mfp_ref().unwrap().bhdr(hp).db_idx(i);
            if (idx_val & DB_MARKED) != 0 {
                curbuf_mut()
                    .b_ml
                    .mfp_mut()
                    .bhdr_mut(hp)
                    .set_db_idx(i, idx_val & DB_INDEX_MASK);
                curbuf_mut().b_ml.ml_flags |= ML_LOCKED_DIRTY;
                ml_st(|st| st.lowest_marked.set(lnum + 1));
                return lnum;
            }
            i += 1;
            lnum += 1;
        }
    }

    0
}

/// Clear all `DB_MARKED` flags.
pub fn ml_clearmarked() {
    let mut lnum = ml_st(|st| st.lowest_marked.get());
    if curbuf_mut().b_ml.ml_mfp.is_none() || lnum == 0 {
        return;
    }

    while lnum <= curbuf_mut().b_ml.ml_line_count {
        let Some(hp) = ml_find_line(curbuf_mut(), lnum, ML_FIND) else {
            return;
        };
        let mut i = (lnum - curbuf_mut().b_ml.ml_locked_low) as usize;
        while lnum <= curbuf_mut().b_ml.ml_locked_high {
            let idx_val = curbuf_mut().b_ml.mfp_ref().unwrap().bhdr(hp).db_idx(i);
            if (idx_val & DB_MARKED) != 0 {
                curbuf_mut()
                    .b_ml
                    .mfp_mut()
                    .bhdr_mut(hp)
                    .set_db_idx(i, idx_val & DB_INDEX_MASK);
                curbuf_mut().b_ml.ml_flags |= ML_LOCKED_DIRTY;
            }
            i += 1;
            lnum += 1;
        }
    }

    ml_st(|st| st.lowest_marked.set(0));
}

pub fn ml_flush_deleted_bytes(
    buf: &mut FileBuf,
    codepoints: &mut usize,
    codeunits: &mut usize,
) -> usize {
    let ret = buf.deleted_bytes;
    *codepoints = buf.deleted_codepoints;
    *codeunits = buf.deleted_codeunits;
    buf.deleted_bytes = 0;
    buf.deleted_codepoints = 0;
    buf.deleted_codeunits = 0;
    ret
}

fn ml_flush_line(buf: &mut FileBuf, _noalloc: bool) {
    ml_flush_line_impl(buf, None);
}

fn ml_flush_line_impl(buf: &mut FileBuf, override_line: Option<&[u8]>) {
    if buf.b_ml.ml_line_lnum == 0 || buf.b_ml.ml_mfp.is_none() {
        return;
    }
    if (buf.b_ml.ml_flags & ML_LINE_DIRTY) != 0 {
        if ml_st(|st| st.ml_flush_entered.replace(true)) {
            return;
        }

        buf.flush_count += 1;

        let lnum = buf.b_ml.ml_line_lnum;
        let new_len = buf.b_ml.ml_line_textlen;
        let owned_line = if override_line.is_some() {
            Vec::new()
        } else {
            std::mem::take(&mut buf.b_ml.ml_line_buf)
        };
        let new_line_slice: &[u8] = override_line.unwrap_or(&owned_line);

        let Some(hp) = ml_find_line(buf, lnum, ML_FIND) else {
            siemsg!(tr(c"E320: Cannot find line %lld"), i64::from(lnum));
            drop(owned_line);
            ml_st(|st| st.ml_flush_entered.set(false));
            buf.b_ml.ml_flags &= !ML_LINE_DIRTY;
            buf.b_ml.ml_line_lnum = 0;
            buf.b_ml.ml_line_offset = 0;
            return;
        };
        {
            let idx = (lnum - buf.b_ml.ml_locked_low) as usize;
            let (start, old_len, db_free, txt_start, is_marked) = {
                let hp_ref = buf.b_ml.mfp_ref().unwrap().bhdr(hp);
                let raw_idx = hp_ref.db_idx(idx);
                let start = (raw_idx & DB_INDEX_MASK) as i32;
                let old_len: i32 = if idx == 0 {
                    (hp_ref.db_txt_end() as i32) - start
                } else {
                    ((hp_ref.db_idx(idx - 1) & DB_INDEX_MASK) as i32) - start
                };
                (
                    start,
                    old_len,
                    hp_ref.db_free() as i32,
                    hp_ref.db_txt_start() as isize,
                    (raw_idx & DB_MARKED) != 0,
                )
            };
            let extra = new_len - old_len;

            if db_free >= extra {
                let count = (buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low + 1) as usize;
                {
                    let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
                    if extra != 0 && idx < count - 1 {
                        bh_copy_within(
                            hp_ref,
                            txt_start as usize,
                            (txt_start - extra as isize) as usize,
                            (start - txt_start as i32) as usize,
                        );
                        for i in (idx + 1)..count {
                            let v = hp_ref.db_idx(i).wrapping_sub(extra as u32);
                            hp_ref.set_db_idx(i, v);
                        }
                    }
                    let cur_v = hp_ref.db_idx(idx).wrapping_sub(extra as u32);
                    hp_ref.set_db_idx(idx, cur_v);

                    hp_ref.set_db_free(hp_ref.db_free().wrapping_sub(extra as u32));
                    hp_ref.set_db_txt_start(hp_ref.db_txt_start().wrapping_sub(extra as u32));

                    bh_write_bytes(
                        hp_ref,
                        (start - extra) as usize,
                        new_line_slice,
                        new_len as usize,
                    );
                }
                buf.b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
                if extra != 0 {
                    ml_updatechunk(buf, lnum, extra, ML_CHNK_UPDLINE);
                }
            } else {
                let mark_flag = if is_marked { ML_APPEND_MARK } else { 0 };
                let _ = ml_append_int(buf, lnum, new_line_slice, new_len, mark_flag);
                let _ = ml_delete_int(buf, lnum, 0);
            }
        }
        drop(owned_line);

        ml_st(|st| st.ml_flush_entered.set(false));
    }

    buf.b_ml.ml_flags &= !ML_LINE_DIRTY;
    buf.b_ml.ml_line_lnum = 0;
    buf.b_ml.ml_line_offset = 0;
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

fn ml_find_line(buf: &mut FileBuf, lnum: Linenr, action: i32) -> Option<BhdrId> {
    assert!(buf.b_ml.mfp_ref().is_some(), "ml_mfp is None");

    if let Some(locked_id) = buf.b_ml.ml_locked {
        if ml_simple(action) && buf.b_ml.ml_locked_low <= lnum && buf.b_ml.ml_locked_high >= lnum {
            if action == ML_INSERT {
                buf.b_ml.ml_locked_lineadd += 1;
                buf.b_ml.ml_locked_high += 1;
            } else if action == ML_DELETE {
                buf.b_ml.ml_locked_lineadd -= 1;
                buf.b_ml.ml_locked_high -= 1;
            }
            return Some(locked_id);
        }

        let dirty = (buf.b_ml.ml_flags & ML_LOCKED_DIRTY) != 0;
        let pos = (buf.b_ml.ml_flags & ML_LOCKED_POS) != 0;
        mf_put(buf.b_ml.mfp_mut(), locked_id, dirty, pos);
        buf.b_ml.ml_locked = None;

        if buf.b_ml.ml_locked_lineadd != 0 {
            ml_lineadd(buf, buf.b_ml.ml_locked_lineadd);
        }
    }

    if action == ML_FLUSH {
        return None;
    }

    let mut bnum: Blocknr = 1;
    let mut page_count: i32 = 1;
    let mut low: Linenr = 1;
    let mut high: Linenr = buf.b_ml.ml_line_count;

    if action == ML_FIND {
        let mut top = buf.b_ml.ml_stack_top - 1;
        while top >= 0 {
            let ip = &(&buf.b_ml.ml_stack)[top as usize];
            if ip.ip_low <= lnum && ip.ip_high >= lnum {
                bnum = ip.ip_bnum;
                low = ip.ip_low;
                high = ip.ip_high;
                buf.b_ml.ml_stack_top = top;
                break;
            }
            top -= 1;
        }
        if top < 0 {
            buf.b_ml.ml_stack_top = 0;
        }
    } else {
        buf.b_ml.ml_stack_top = 0;
    }

    loop {
        let Some(hp) = mf_get(buf.b_ml.mfp_mut(), bnum, page_count as u32) else {
            return ml_find_line_error_noblock(buf, action);
        };

        if action == ML_INSERT {
            high += 1;
        } else if action == ML_DELETE {
            high -= 1;
        }

        if buf.b_ml.mfp_ref().unwrap().bhdr(hp).db_id() == DATA_ID {
            buf.b_ml.ml_locked = Some(hp);
            buf.b_ml.ml_locked_low = low;
            buf.b_ml.ml_locked_high = high;
            buf.b_ml.ml_locked_lineadd = 0;
            buf.b_ml.ml_flags &= !(ML_LOCKED_DIRTY | ML_LOCKED_POS);
            return Some(hp);
        }

        if buf.b_ml.mfp_ref().unwrap().bhdr(hp).pb_id() != PTR_ID {
            iemsg(tr(E_POINTER_BLOCK_ID_WRONG));
            mf_put(buf.b_ml.mfp_mut(), hp, false, false);
            return ml_find_line_error_noblock(buf, action);
        }

        let top = ml_add_stack(buf);
        let ip = &mut (&mut buf.b_ml.ml_stack)[top as usize];
        ip.ip_bnum = bnum;
        ip.ip_low = low;
        ip.ip_high = high;
        ip.ip_index = -1;

        let mut dirty = false;
        let mut idx: i32 = 0;
        let pb_count = buf.b_ml.mfp_ref().unwrap().bhdr(hp).pb_count();
        while idx < i32::from(pb_count) {
            let mut pe = buf.b_ml.mfp_ref().unwrap().bhdr(hp).pb_entry(idx as usize);
            let t = pe.pe_line_count;
            low += t;
            if low > lnum {
                buf.b_ml.ml_stack[top as usize].ip_index = idx;
                bnum = pe.pe_bnum;
                page_count = pe.pe_page_count;
                high = low - 1;
                low -= t;

                if bnum < 0 {
                    let bnum2 = mf_trans_del(buf.b_ml.mfp_mut(), bnum);
                    if bnum != bnum2 {
                        bnum = bnum2;
                        pe.pe_bnum = bnum;
                        buf.b_ml
                            .mfp_mut()
                            .bhdr_mut(hp)
                            .set_pb_entry(idx as usize, pe);
                        dirty = true;
                    }
                }
                break;
            }
            idx += 1;
        }
        if idx >= i32::from(pb_count) {
            if lnum > buf.b_ml.ml_line_count {
                siemsg!(
                    tr(E_LINE_NUMBER_OUT_OF_RANGE_NR_PAST_THE_END),
                    i64::from(lnum - buf.b_ml.ml_line_count),
                );
            } else {
                siemsg!(tr(E_LINE_COUNT_WRONG_IN_BLOCK_NR), bnum);
            }
            mf_put(buf.b_ml.mfp_mut(), hp, false, false);
            return ml_find_line_error_noblock(buf, action);
        }
        if action == ML_DELETE {
            let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
            let mut pe = hp_ref.pb_entry(idx as usize);
            pe.pe_line_count -= 1;
            hp_ref.set_pb_entry(idx as usize, pe);
            dirty = true;
        } else if action == ML_INSERT {
            let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
            let mut pe = hp_ref.pb_entry(idx as usize);
            pe.pe_line_count += 1;
            hp_ref.set_pb_entry(idx as usize, pe);
            dirty = true;
        }
        mf_put(buf.b_ml.mfp_mut(), hp, dirty, false);
    }
}

fn ml_find_line_error_noblock(buf: &mut FileBuf, action: i32) -> Option<BhdrId> {
    if action == ML_DELETE {
        ml_lineadd(&mut *buf, 1);
    } else if action == ML_INSERT {
        ml_lineadd(&mut *buf, -1);
    }
    buf.b_ml.ml_stack_top = 0;
    None
}

fn ml_add_stack(buf: &mut FileBuf) -> i32 {
    let top = buf.b_ml.ml_stack_top;
    if (top as usize) == buf.b_ml.ml_stack.len() {
        buf.b_ml.ml_stack.resize(
            buf.b_ml.ml_stack.len() + (STACK_INCR as usize),
            Infoptr::default(),
        );
    }
    buf.b_ml.ml_stack_top += 1;
    top
}

fn ml_lineadd(buf: &mut FileBuf, count: i32) {
    let Memline {
        ml_mfp,
        ml_stack,
        ml_stack_top,
        ..
    } = &mut buf.b_ml;
    let mfp = ml_mfp.as_deref_mut().expect("ml_mfp is None");
    let mut idx = *ml_stack_top - 1;
    while idx >= 0 {
        let ip = &mut ml_stack[idx as usize];
        let Some(hp) = mf_get(mfp, ip.ip_bnum, 1) else {
            break;
        };
        if mfp.bhdr(hp).pb_id() != PTR_ID {
            mf_put(mfp, hp, false, false);
            iemsg(tr(E_POINTER_BLOCK_ID_WRONG_TWO));
            break;
        }
        {
            let hp_ref = mfp.bhdr_mut(hp);
            let mut pe = hp_ref.pb_entry(ip.ip_index as usize);
            pe.pe_line_count += count;
            hp_ref.set_pb_entry(ip.ip_index as usize, pe);
        }
        ip.ip_high += count;
        mf_put(mfp, hp, true, false);
        idx -= 1;
    }
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
            let tmp_cstr = crate::memory::bytes_as_cstr(&tmp);
            let tail = path_tail(tmp_cstr);
            let off = tmp_cstr.to_bytes().len() - tail.to_bytes().len();
            if tail.to_bytes().len() + buf_bytes.len() >= MAXPATHL {
                return false;
            }
            crate::memory::copy_bytes_to_c_buf(&mut tmp[off..], buf_bytes);
        }
    }

    vim_full_name(Some(crate::memory::bytes_as_cstr(&tmp)), buf, true)
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
    if after_pathsep(dir_bytes) != 0 && len > 1 && dir_bytes[len - 1] == dir_bytes[len - 2] {
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
    let tail_cstr = path_tail(fname);
    let tail = cstr_as_string(Some(tail_cstr));
    let tail_off = fname_bytes.len() - tail_cstr.to_bytes().len();
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

fn attention_message(buf: &mut FileBuf, fname: &CStr, fhname: &CStr, msg: &mut Vec<u8>) {
    assert!(buf.fname_opt().is_some());
    let buf_fname_cstr = buf.fname_cstr();

    emsg(tr(c"E325: ATTENTION"));
    sb_push_cstr(msg, tr(c"Found a swap file by the name \""));
    sb_push_cstr(msg, fhname);
    msg.extend_from_slice(b"\"\n");
    let swap_mtime = swapfile_info(fname, msg);
    sb_push_cstr(msg, tr(c"While opening file \""));
    sb_push_cstr(msg, buf_fname_cstr);
    msg.extend_from_slice(b"\"\n");
    let mut file_info = FileInfo::new();
    if !os_fileinfo(buf_fname_cstr, &mut file_info) {
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
    sb_push_cstr(msg, buf_fname_cstr);
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

fn do_swapexists(buf: &mut FileBuf, fname: &CStr) -> SeaChoice {
    set_vim_var_string(VV_SWAPNAME, Some(fname), -1);
    set_vim_var_string(VV_SWAPCHOICE, None, -1);

    g_st().allbuf_lock += 1;
    apply_autocmds(EVENT_SWAPEXISTS, buf.fname_opt(), None, false, None);
    g_st().allbuf_lock -= 1;

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
    buf: &mut FileBuf,
    dirp: &mut &[u8],
    old_fname: Option<&CStr>,
    found_existing_dir: &mut bool,
) -> Option<std::ffi::CString> {
    let buf_fname = buf.fname_opt().map(CStr::to_owned);
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
            && path_equal(fname_cstr, old_f, K_PATH_CMP_LITERAL)
        {
            break;
        }

        let fname_slice = &fvec[..n];
        if fname_slice.ends_with(b"wp")
            && !g_st().recoverymode
            && buf_fname.is_some()
            && !buf.b_help
            && (buf.b_flags & BF_DUMMY) == 0
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
                            path_tail(buf_ffname_opt.unwrap_or(c"")),
                            path_tail(b0_fname_cstr),
                            K_PATH_CMP_LITERAL,
                        ) || !same_directory(Some(fname_cstr), buf_ffname_opt)
                        {
                            expand_env(b0_fname_cstr, g_st().name_buff_bytes_mut());
                            if fnamecmp_ino(
                                buf_ffname_opt,
                                Some(g_st().name_buff_cstr()),
                                char_to_long(&b0.b0_ino),
                            ) {
                                differ = true;
                            }
                        }
                    } else {
                        expand_env(b0_fname_cstr, g_st().name_buff_bytes_mut());
                        if fnamecmp_ino(
                            buf_ffname_opt,
                            Some(g_st().name_buff_cstr()),
                            char_to_long(&b0.b0_ino),
                        ) {
                            differ = true;
                        }
                    }
                }
                os_close(fd);
            }

            if !differ
                && (curbuf_mut().b_flags & BF_RECOVERED) == 0
                && !crate::strings::vim_haschr(opt_vars().p_shm.as_cstr(), K_SHM_ATTENTION)
            {
                let mut choice: SeaChoice = SEA_CHOICE_NONE;

                if os_path_exists(buf.fname_cstr()) && swapfile_unchanged(fname_cstr) {
                    choice = SEA_CHOICE_DELETE;
                    if opt_vars().p_verbose > 0 {
                        verb_msg(tr(c"Found a swap file that is not useful, deleting it"));
                    }
                }

                if choice == SEA_CHOICE_NONE
                    && g_st().swap_exists_action != SEA_NONE
                    && has_autocmd(EVENT_SWAPEXISTS, buf_fname.as_deref(), Some(&mut *buf))
                {
                    choice = do_swapexists(buf, fname_cstr);
                }

                if choice == SEA_CHOICE_NONE && g_st().swap_exists_action == SEA_READONLY {
                    choice = SEA_CHOICE_READONLY;
                }

                ml_st(|st| st.proc_running.set(0));
                if choice == SEA_CHOICE_NONE {
                    g_st().no_wait_return += 1;
                    let mut msg_sb = Vec::<u8>::with_capacity(IOSIZE);
                    let fhname = home_replace_save(None, Some(fname_cstr));
                    attention_message(buf, fname_cstr, &fhname, &mut msg_sb);

                    g_st().got_int = false;
                    flush_buffers(FLUSH_TYPEAHEAD);

                    if g_st().swap_exists_action != SEA_NONE {
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
                    g_st().no_wait_return -= 1;
                }

                match choice {
                    SEA_CHOICE_READONLY => {
                        buf.b_p_ro = true;
                    }
                    SEA_CHOICE_EDIT => {}
                    SEA_CHOICE_RECOVER => {
                        g_st().swap_exists_action = SEA_RECOVER;
                    }
                    SEA_CHOICE_DELETE => {
                        os_remove(fname_cstr);
                    }
                    SEA_CHOICE_QUIT => {
                        g_st().swap_exists_action = SEA_QUIT;
                    }
                    SEA_CHOICE_ABORT => {
                        g_st().swap_exists_action = SEA_QUIT;
                        g_st().got_int = true;
                    }
                    SEA_CHOICE_NONE => {
                        msg_puts(c"\n");
                        if g_st().msg_silent == 0 {
                            g_st().need_wait_return = true;
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

    let retval_c = vim_full_name(fname_c, &mut buf_c, true);
    let retval_s = vim_full_name(fname_s, &mut buf_s, true);
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
pub fn ml_setflags(buf: &mut FileBuf) {
    let Some(hp) = buf.b_ml.mfp_ref().and_then(|mfp| mfp.find_bhdr(0)) else {
        return;
    };
    let mut b0p = ZeroBlock::from_bytes(&buf.b_ml.mfp_ref().unwrap().bhdr(hp).bh_data);
    b0p.set_dirty(if buf.b_changed != 0 { B0_DIRTY } else { 0 });
    let ff_bits = ((get_fileformat(buf) + 1) as u8) & B0_FF_MASK;
    b0p.set_flags((b0p.flags() & !B0_FF_MASK) | ff_bits);
    add_b0_fenc(&mut b0p, buf);
    let hp_ref = buf.b_ml.mfp_mut().bhdr_mut(hp);
    b0p.write_to_bytes(&mut hp_ref.bh_data);
    hp_ref.bh_flags |= BH_DIRTY;
    mf_sync(buf.b_ml.mfp_mut(), MFS_ZERO);
}

fn ml_updatechunk(buf: &mut FileBuf, line: Linenr, mut len: i32, updtype: i32) {
    let (mut curline, mut curix, lastbuf) = ml_st(|st| {
        (
            st.ml_upd_lastcurline.get(),
            st.ml_upd_lastcurix.get(),
            st.ml_upd_lastbuf.get(),
        )
    });

    if buf.b_ml.ml_usedchunks == -1 || len == 0 {
        return;
    }
    if buf.b_ml.ml_chunksize.is_empty() {
        buf.b_ml.ml_chunksize.resize(100, Chunksize::default());
        buf.b_ml.ml_usedchunks = 1;
        buf.b_ml.ml_chunksize[0].mlcs_numlines = 1;
        buf.b_ml.ml_chunksize[0].mlcs_totalsize = 1;
        ml_st(|st| st.ml_upd_lastbuf.set(crate::globals::BufPtr::NULL));
    }

    if updtype == ML_CHNK_UPDLINE && buf.b_ml.ml_line_count == 1 {
        buf.b_ml.ml_usedchunks = 1;
        buf.b_ml.ml_chunksize[0].mlcs_numlines = 1;
        buf.b_ml.ml_chunksize[0].mlcs_totalsize = buf.b_ml.ml_line_textlen;
        ml_st(|st| st.ml_upd_lastbuf.set(crate::globals::BufPtr::NULL));
        return;
    }

    if lastbuf != buf || line < curline {
        curline = 1;
        curix = 0;
    }
    while curix < buf.b_ml.ml_usedchunks - 1
        && line >= curline + buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines
    {
        curline += buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines;
        curix += 1;
    }

    if updtype == ML_CHNK_DELLINE {
        len = -len;
    }
    buf.b_ml.ml_chunksize[curix as usize].mlcs_totalsize += len;
    if updtype == ML_CHNK_ADDLINE {
        buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines += 1;

        if (buf.b_ml.ml_usedchunks + 1) as usize >= buf.b_ml.ml_chunksize.len() {
            let new_len = buf.b_ml.ml_chunksize.len() * 3 / 2;
            buf.b_ml.ml_chunksize.resize(new_len, Chunksize::default());
        }

        if buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines >= MLCS_MAXL {
            let used = buf.b_ml.ml_usedchunks as usize;
            let cix = curix as usize;
            buf.b_ml.ml_chunksize.copy_within(cix..used, cix + 1);
            let mut size: i32 = 0;
            let mut linecnt: i32 = 0;
            while curline < buf.b_ml.ml_line_count && linecnt < MLCS_MINL {
                let Some(hp) = ml_find_line(buf, curline, ML_FIND) else {
                    buf.b_ml.ml_usedchunks = -1;
                    return;
                };
                let count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low + 1;
                let idx = curline - buf.b_ml.ml_locked_low;
                curline = buf.b_ml.ml_locked_high + 1;
                let rest = count - idx;
                let end_idx = if linecnt + rest > MLCS_MINL {
                    let e = idx + MLCS_MINL - linecnt - 1;
                    linecnt = MLCS_MINL;
                    e
                } else {
                    linecnt += rest;
                    count - 1
                };
                let hp_ref = buf.b_ml.mfp_ref().unwrap().bhdr(hp);
                let text_end = if idx == 0 {
                    hp_ref.db_txt_end() as i32
                } else {
                    (hp_ref.db_idx((idx - 1) as usize) & DB_INDEX_MASK) as i32
                };
                size += text_end - ((hp_ref.db_idx(end_idx as usize) & DB_INDEX_MASK) as i32);
            }
            buf.b_ml.ml_chunksize[cix].mlcs_numlines = linecnt;
            buf.b_ml.ml_chunksize[cix + 1].mlcs_numlines -= linecnt;
            buf.b_ml.ml_chunksize[cix].mlcs_totalsize = size;
            buf.b_ml.ml_chunksize[cix + 1].mlcs_totalsize -= size;
            buf.b_ml.ml_usedchunks += 1;
            ml_st(|st| st.ml_upd_lastbuf.set(crate::globals::BufPtr::NULL));
            return;
        } else if buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines >= MLCS_MINL
            && curix == buf.b_ml.ml_usedchunks - 1
            && buf.b_ml.ml_line_count - line <= 1
        {
            let next_ix = (curix + 1) as usize;
            buf.b_ml.ml_usedchunks += 1;
            if line == buf.b_ml.ml_line_count {
                buf.b_ml.ml_chunksize[next_ix].mlcs_numlines = 0;
                buf.b_ml.ml_chunksize[next_ix].mlcs_totalsize = 0;
            } else {
                let Some(hp) = ml_find_line(buf, buf.b_ml.ml_line_count, ML_FIND) else {
                    buf.b_ml.ml_usedchunks = -1;
                    return;
                };
                let hp_ref = buf.b_ml.mfp_ref().unwrap().bhdr(hp);
                let rest = if hp_ref.db_line_count() == 1 {
                    (hp_ref.db_txt_end() - hp_ref.db_txt_start()) as i32
                } else {
                    ((hp_ref.db_idx((hp_ref.db_line_count() - 2) as usize) & DB_INDEX_MASK) as i32)
                        - (hp_ref.db_txt_start() as i32)
                };
                buf.b_ml.ml_chunksize[next_ix].mlcs_totalsize = rest;
                buf.b_ml.ml_chunksize[next_ix].mlcs_numlines = 1;
                buf.b_ml.ml_chunksize[next_ix - 1].mlcs_totalsize -= rest;
                buf.b_ml.ml_chunksize[next_ix - 1].mlcs_numlines -= 1;
            }
        }
    } else if updtype == ML_CHNK_DELLINE {
        buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines -= 1;
        if curix < buf.b_ml.ml_usedchunks - 1
            && (buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines
                + buf.b_ml.ml_chunksize[(curix + 1) as usize].mlcs_numlines)
                <= MLCS_MINL
        {
            curix += 1;
        } else if curix == 0 && buf.b_ml.ml_chunksize[0].mlcs_numlines <= 0 {
            ml_st(|st| st.ml_upd_lastbuf.set(crate::globals::BufPtr::NULL));
            buf.b_ml.ml_usedchunks -= 1;
            let used = buf.b_ml.ml_usedchunks as usize;
            buf.b_ml.ml_chunksize.copy_within(1..(1 + used), 0);
            return;
        } else if curix == 0
            || (buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines > 10
                && (buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines
                    + buf.b_ml.ml_chunksize[(curix - 1) as usize].mlcs_numlines)
                    > MLCS_MINL)
        {
            let buf_ptr = crate::globals::BufPtr::from(&mut *buf);
            ml_st(|st| {
                st.ml_upd_lastbuf.set(buf_ptr);
                st.ml_upd_lastcurline.set(curline);
                st.ml_upd_lastcurix.set(curix);
            });
            return;
        }

        let cix = curix as usize;
        ml_st(|st| st.ml_upd_lastbuf.set(crate::globals::BufPtr::NULL));
        buf.b_ml.ml_chunksize[cix - 1].mlcs_numlines += buf.b_ml.ml_chunksize[cix].mlcs_numlines;
        buf.b_ml.ml_chunksize[cix - 1].mlcs_totalsize += buf.b_ml.ml_chunksize[cix].mlcs_totalsize;
        buf.b_ml.ml_usedchunks -= 1;
        if curix < buf.b_ml.ml_usedchunks {
            let used = buf.b_ml.ml_usedchunks as usize;
            buf.b_ml.ml_chunksize.copy_within((cix + 1)..=used, cix);
        }
        return;
    }
    let buf_ptr = crate::globals::BufPtr::from(&mut *buf);
    ml_st(|st| {
        st.ml_upd_lastbuf.set(buf_ptr);
        st.ml_upd_lastcurline.set(curline);
        st.ml_upd_lastcurix.set(curix);
    });
}

/// Find offset for line or line with offset.
pub fn ml_find_line_or_offset(
    buf: &mut FileBuf,
    lnum: Linenr,
    mut offp: Option<&mut i32>,
    no_ff: bool,
) -> i32 {
    let ffdos = i32::from(!no_ff && get_fileformat(buf) == EOL_DOS);
    let mut extra: i32 = 0;

    let can_cache = lnum != 0 && ffdos == 0 && buf.b_ml.ml_line_lnum == lnum;
    if lnum == 0 || buf.b_ml.ml_line_lnum < lnum || !no_ff {
        ml_flush_line(curbuf_mut(), false);
    } else if can_cache && buf.b_ml.ml_line_offset > 0 {
        return buf.b_ml.ml_line_offset as i32;
    }

    if buf.b_ml.ml_usedchunks == -1 || buf.b_ml.ml_chunksize.is_empty() || lnum < 0 {
        if no_ff && buf.b_ml.ml_mfp.is_some() && (lnum == 1 || lnum == 2) {
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

    let mut curline: Linenr = 1;
    let mut curix: i32 = 0;
    let mut size: i32 = 0;
    while curix < buf.b_ml.ml_usedchunks - 1
        && ((lnum != 0 && lnum >= curline + buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines)
            || (offset != 0
                && offset
                    > size
                        + buf.b_ml.ml_chunksize[curix as usize].mlcs_totalsize
                        + ffdos * buf.b_ml.ml_chunksize[curix as usize].mlcs_numlines))
    {
        let ch = buf.b_ml.ml_chunksize[curix as usize];
        curline += ch.mlcs_numlines;
        size += ch.mlcs_totalsize;
        if offset != 0 && ffdos != 0 {
            size += ch.mlcs_numlines;
        }
        curix += 1;
    }

    while (lnum != 0 && curline < lnum) || (offset != 0 && size < offset) {
        if curline > buf.b_ml.ml_line_count {
            return -1;
        }
        let Some(hp) = ml_find_line(buf, curline, ML_FIND) else {
            return -1;
        };
        let count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low + 1;
        let start_idx = curline - buf.b_ml.ml_locked_low;
        let mut idx = start_idx;
        let hp_ref = buf.b_ml.mfp_ref().unwrap().bhdr(hp);
        let text_end = if idx == 0 {
            hp_ref.db_txt_end() as i32
        } else {
            (hp_ref.db_idx((idx - 1) as usize) & DB_INDEX_MASK) as i32
        };

        if lnum != 0 {
            if curline + (count - idx) >= lnum {
                idx += lnum - curline - 1;
            } else {
                idx = count - 1;
            }
        } else {
            extra = 0;
            loop {
                if offset
                    < size + text_end - ((hp_ref.db_idx(idx as usize) & DB_INDEX_MASK) as i32)
                        + ffdos
                {
                    break;
                }
                if ffdos != 0 {
                    size += 1;
                }
                if idx == count - 1 {
                    extra = 1;
                    break;
                }
                idx += 1;
            }
        }
        let len = text_end - ((hp_ref.db_idx(idx as usize) & DB_INDEX_MASK) as i32);
        size += len;
        if offset != 0 && size >= offset {
            if size + ffdos == offset {
                *offp.as_deref_mut().unwrap() = 0;
            } else if idx == start_idx {
                *offp.as_deref_mut().unwrap() = offset - size + len;
            } else {
                *offp.as_deref_mut().unwrap() = offset - size + len
                    - (text_end - ((hp_ref.db_idx((idx - 1) as usize) & DB_INDEX_MASK) as i32));
            }
            curline += idx - start_idx + extra;
            if curline > buf.b_ml.ml_line_count {
                return -1;
            }
            return curline;
        }
        curline = buf.b_ml.ml_locked_high + 1;
    }

    if lnum != 0 {
        if ffdos != 0 {
            size += lnum - 1;
        }
        if (!buf.b_p_fixeol || buf.b_p_bin) && !buf.b_p_eol && lnum > buf.b_ml.ml_line_count {
            size -= ffdos + 1;
        }
    }

    if can_cache && size > 0 {
        buf.b_ml.ml_line_offset = size as usize;
    }

    size
}

/// Goto byte in buffer with offset `cnt`.
pub fn goto_byte(cnt: i32) {
    let mut boff = cnt;
    ml_flush_line(curbuf_mut(), false);
    setpcmark();
    if boff != 0 {
        boff -= 1;
    }
    let lnum = ml_find_line_or_offset(curbuf_mut(), 0, Some(&mut boff), false) as Linenr;
    if lnum < 1 {
        curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
        curwin_mut().w_curswant = MAXCOL;
        coladvance(curwin_mut(), MAXCOL);
    } else {
        curwin_mut().w_cursor.lnum = lnum;
        curwin_mut().w_cursor.col = boff as Colnr;
        curwin_mut().w_cursor.coladd = 0;
        curwin_mut().w_set_curswant = true;
    }
    check_cursor(curwin_mut());
    mb_adjust_cursor();
}

/// Increment the line pointer `lp` crossing line boundaries as necessary.
pub fn inc(lp: &mut CursorPos) -> i32 {
    if lp.col != MAXCOL {
        let p = ml_get_pos_slice(lp);
        if !p.is_empty() {
            let l = crate::mbyte::utfc_slice2len(p) as i32;
            lp.col += l;
            return if p.len() > l as usize { 0 } else { 2 };
        }
    }
    if lp.lnum != curbuf_mut().b_ml.ml_line_count {
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
        lp.col -= crate::mbyte::utf_head_off_slice(line, lp.col as usize);
        return 0;
    }

    if lp.col > 0 {
        lp.col -= 1;
        let line = ml_get_slice(lp.lnum);
        lp.col -= crate::mbyte::utf_head_off_slice(line, lp.col as usize);
        return 0;
    }
    if lp.lnum > 1 {
        lp.lnum -= 1;
        let line = ml_get_slice(lp.lnum);
        lp.col = line.len() as Colnr;
        lp.col -= crate::mbyte::utf_head_off_slice(line, lp.col as usize);
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
        let mut buf = FileBuf::new();
        assert_eq!(ml_get_buf_slice(&mut buf, 1), b"");
        assert_eq!(ml_get_buf_len(&mut buf, 1), 0);
    }
}
