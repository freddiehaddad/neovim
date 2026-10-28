//! Buffer line storage and swapfile recovery (`src/nvim/memline.c`).

use crate::api::private::helpers::{HLF_E, cstr_as_string};
use crate::api::private::validate::{NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{
    EVENT_BUFREADPOST, EVENT_BUFWINENTER, EVENT_SWAPEXISTS, apply_autocmds, has_autocmd,
};
use crate::buffer::{
    MAXLNUM, MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC, NOTDONE, SEA_NONE, SEA_QUIT, SEA_RECOVER, FileBuf,
    buf_inc_changedtick, buf_spname, K_UIMESSAGES, open_buffer, setfname,
};
use crate::change::{ML_DEL_MESSAGE, changed_internal, unchanged};
use crate::charset::ascii_isalpha;
use crate::cursor::{check_cursor, coladvance, Colnr, Linenr, CursorPos};
use crate::dialog::do_dialog;
use crate::drawscreen::{UPD_NOT_VALID, redraw_curbuf_later};
use crate::eval::typval::{
    TvDict, TvList, tv_clear, tv_dict_add_nr, tv_dict_add_str_len, tv_list_alloc_ret,
    tv_list_first, tv_list_len, TypVal,
};
use crate::eval::vars::{VV_SWAPCHOICE, VV_SWAPNAME, get_vim_var_str, set_vim_var_string};
use crate::fileio::{
    EOL_DOS, O_RDONLY, READ_NEW, SEEK_END, buf_store_file_info, K_OPT_FILEENCODING, modname, Off,
    read_eintr, readfile, vim_deltempdir, vim_rename, vim_tempname,
};
use crate::globals::*;
use crate::input::flush_buffers;
use crate::lua::executor::nlua_call_typval;
use crate::main_init::getout;
use crate::mark::setpcmark;
use crate::mbyte::{mb_adjust_cursor, mb_utflen, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::memfile::{
    BH_DIRTY, MFS_ALL, MFS_STOP, MFS_ZERO, Bhdr, Blocknr, Memfile, mf_close, mf_close_file,
    mf_free, mf_free_fnames, mf_get, mf_need_trans, mf_new, mf_new_page_size, mf_open,
    mf_open_file, mf_put, mf_set_dirty, mf_set_fnames, mf_sync, mf_trans_del, pmap_get_bhdr,
};
use crate::memory::{xstrlcpy};
use crate::message::{
    emsg, iemsg, msg, msg_end, msg_ext_set_kind, msg_multiline, msg_outnum, msg_outtrans,
    msg_putchar, msg_puts, msg_puts_hl, msg_reset_scroll, msg_start, semsg, set_keep_msg, siemsg,
    smsg, verb_msg,
};
use crate::option::{copy_option_part, get_fileformat, set_fileformat, set_option_value_give_err};
use crate::option_vars::*;
use crate::os::env::{expand_env, home_replace, home_replace_save, os_get_hostname, os_get_pid};
use crate::os::fs::{
    FileInfo, O_RDWR, os_fileinfo, os_fileinfo_inode, os_fileinfo_link, os_fileinfo_size,
    os_fileinfo2, os_isdir, os_mkdir_recurse, os_open, os_path_exists, os_remove, os_set_cloexec,
};
use crate::os::input::{line_breakcheck, os_char_avail};
use crate::os::proc::os_proc_running;
use crate::os::time::{Timestamp, os_ctime_r, os_time, time_t};
use crate::os::users::os_get_username;
#[cfg(unix)]
use crate::path::path_is_absolute;
use crate::path::{
    EW_FILE, EW_KEEPALL, EW_SILENT, after_pathsep, concat_fnames, expand_wildcards_vec, fix_fname,
    K_PATH_CMP_FULL, K_PATH_CMP_LITERAL, path_equal, path_tail, same_directory, shorten_dir,
    vim_full_name, vim_ispathsep,
};
use crate::spell::spell_delete_wordlist;
use crate::statusline::get_trans_bufname;
use crate::strings::{gettext, tr, vim_strchr};
use crate::ui::{ui_flush, ui_has};
use crate::undo::buf_is_changed_fn;
use std::ffi::{CStr, c_char, c_int, c_long, c_uint};
use std::ptr;

#[repr(C)]
pub struct Memline {
    pub ml_line_count: i32,
    pub ml_mfp: Option<Box<crate::memfile::Memfile>>,
    pub ml_stack: Vec<Infoptr>,
    pub ml_stack_top: c_int,
    pub ml_flags: c_int,
    pub ml_line_textlen: c_int,
    pub ml_line_lnum: i32,
    pub ml_line_ptr: *mut c_char,
    pub ml_line_buf: Vec<c_char>,
    pub ml_line_offset: usize,
    pub ml_line_offset_ff: c_int,
    pub ml_locked: *mut crate::memfile::Bhdr,
    pub ml_locked_low: i32,
    pub ml_locked_high: i32,
    pub ml_locked_lineadd: c_int,
    pub ml_chunksize: Vec<Chunksize>,
    pub ml_usedchunks: c_int,
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
            ml_line_ptr: ptr::null_mut(),
            ml_line_buf: Vec::new(),
            ml_line_offset: 0,
            ml_line_offset_ff: 0,
            ml_locked: ptr::null_mut(),
            ml_locked_low: 0,
            ml_locked_high: 0,
            ml_locked_lineadd: 0,
            ml_chunksize: Vec::new(),
            ml_usedchunks: 0,
        }
    }

    #[inline]
    pub fn mfp_ptr(&mut self) -> *mut crate::memfile::Memfile {
        self.ml_mfp
            .as_deref_mut()
            .map_or(ptr::null_mut(), |m| &raw mut *m)
    }
}

pub(crate) const ML_LINE_DIRTY: c_int = 0x02;

#[cfg(unix)]
use crate::os::users::os_get_uname;
#[cfg(windows)]
use crate::path::path_to_slash;

const DATA_ID: u16 = ((b'd' as u16) << 8) + (b'a' as u16);
const PTR_ID: u16 = ((b'p' as u16) << 8) + (b't' as u16);
const BLOCK0_ID0: c_char = b'b' as c_char;
const BLOCK0_ID1: c_char = b'0' as c_char;

const DB_MARKED: c_uint = 1u32 << (c_uint::BITS - 1);
const DB_INDEX_MASK: c_uint = !DB_MARKED;

const INDEX_SIZE: usize = size_of::<c_uint>();
const HEADER_SIZE: usize = std::mem::offset_of!(DataBlock, db_index);

const B0_FNAME_SIZE_ORG: usize = 900;
const B0_FNAME_SIZE_NOCRYPT: usize = 898;
const B0_FNAME_SIZE_CRYPT: usize = 890;
const B0_UNAME_SIZE: usize = 40;
const B0_HNAME_SIZE: usize = 40;

const B0_MAGIC_LONG: c_long = 0x30313233;
const B0_MAGIC_INT: c_int = 0x20212223;
const B0_MAGIC_SHORT: i16 = 0x1213;
const B0_MAGIC_CHAR: c_char = 0x55;

const B0_DIRTY: c_char = 0x55;
const B0_FF_MASK: u8 = 3;
const B0_SAME_DIR: u8 = 4;
const B0_HAS_FENC: u8 = 8;

const STACK_INCR: c_int = 5;

const ML_LOCKED_DIRTY: c_int = 0x04;
const ML_LOCKED_POS: c_int = 0x08;

const ML_APPEND_NEW: c_int = 1;
const ML_APPEND_MARK: c_int = 2;

const ML_CHNK_ADDLINE: c_int = 1;
const ML_CHNK_DELLINE: c_int = 2;
const ML_CHNK_UPDLINE: c_int = 3;

const ML_DELETE: c_int = 0x11;
const ML_INSERT: c_int = 0x12;
const ML_FIND: c_int = 0x13;
const ML_FLUSH: c_int = 0x02;

const MLCS_MAXL: c_int = 800;
const MLCS_MINL: c_int = 400;

use crate::memfile::MIN_SWAP_PAGE_SIZE;
pub(crate) const MFS_FLUSH: c_int = 4;

const BF_RECOVERED: c_int = 0x01;

const K_SHM_ATTENTION: c_int = b'A' as c_int;

pub(crate) const K_PATH_DEVICE_UNC: c_int = 5;

#[cfg(unix)]
const ENOENT: c_int = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Infoptr {
    pub ip_bnum: Blocknr,
    pub ip_low: Linenr,
    pub ip_high: Linenr,
    pub ip_index: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Chunksize {
    pub mlcs_numlines: c_int,
    pub mlcs_totalsize: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointerEntry {
    pe_bnum: Blocknr,
    pe_line_count: Linenr,
    pe_old_lnum: Linenr,
    pe_page_count: c_int,
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
    db_free: c_uint,
    db_txt_start: c_uint,
    db_txt_end: c_uint,
    db_line_count: c_long,
    db_index: [c_uint; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ZeroBlock {
    b0_id: [c_char; 2],
    b0_version: [c_char; 10],
    b0_page_size: [c_char; 4],
    b0_mtime: [c_char; 4],
    b0_ino: [c_char; 4],
    b0_pid: [c_char; 4],
    b0_uname: [c_char; B0_UNAME_SIZE],
    b0_hname: [c_char; B0_HNAME_SIZE],
    b0_fname: [c_char; B0_FNAME_SIZE_ORG],
    b0_magic_long: c_long,
    b0_magic_int: c_int,
    b0_magic_short: i16,
    b0_magic_char: c_char,
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
    #[inline]
    fn dirty(&self) -> c_char {
        self.b0_fname[B0_FNAME_SIZE_ORG - 1]
    }

    #[inline]
    fn set_dirty(&mut self, val: c_char) {
        self.b0_fname[B0_FNAME_SIZE_ORG - 1] = val;
    }

    #[inline]
    fn flags(&self) -> u8 {
        self.b0_fname[B0_FNAME_SIZE_ORG - 2] as u8
    }

    #[inline]
    fn set_flags(&mut self, val: u8) {
        self.b0_fname[B0_FNAME_SIZE_ORG - 2] = val as c_char;
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdBlock0 {
    UbFname = 0,
    UbSameDir = 1,
}

pub type SeaChoice = c_int;
const SEA_CHOICE_NONE: SeaChoice = 0;
const SEA_CHOICE_READONLY: SeaChoice = 1;
const SEA_CHOICE_EDIT: SeaChoice = 2;
const SEA_CHOICE_RECOVER: SeaChoice = 3;
const SEA_CHOICE_DELETE: SeaChoice = 4;
const SEA_CHOICE_QUIT: SeaChoice = 5;
const SEA_CHOICE_ABORT: SeaChoice = 6;

struct MemlineModuleState {
    lowest_marked: Linenr,
    proc_running: c_int,
    ml_get_recursive: c_int,
    ml_get_questions: [c_char; 4],
    ml_flush_entered: bool,
    ml_upd_lastbuf: *mut FileBuf,
    ml_upd_lastcurline: Linenr,
    ml_upd_lastcurix: c_int,
}

impl MemlineModuleState {
    const fn new() -> Self {
        Self {
            lowest_marked: 0,
            proc_running: 0,
            ml_get_recursive: 0,
            ml_get_questions: [0; 4],
            ml_flush_entered: false,
            ml_upd_lastbuf: ptr::null_mut(),
            ml_upd_lastcurline: 0,
            ml_upd_lastcurix: 0,
        }
    }
}

static ML_STATE: GlobalState<MemlineModuleState> = GlobalState::new(MemlineModuleState::new());

#[inline]
fn ml_st() -> &'static mut MemlineModuleState {
    unsafe { &mut *ML_STATE.as_ptr() }
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
use crate::fileio::readlink;
use crate::fileio::{close, vim_lseek};
use crate::globals::IOSIZE;
use crate::math::{FAIL, OK};
use crate::option::OPT_LOCAL;
use crate::os::fs::MAXPATHL;
#[cfg(unix)]
const EINVAL: c_int = 22;
use crate::regexp::MAXCOL;
use crate::version::VERSIONS;

use crate::event::r#loop::uv_uptime;

#[inline]
const fn ml_simple(x: c_int) -> bool {
    (x & 0x10) != 0
}

#[inline]
fn pb_count_max(mfp: &Memfile) -> u16 {
    ((mfp.mf_page_size as usize - std::mem::offset_of!(PointerBlock, pb_pointer))
            / size_of::<PointerEntry>()) as u16
}

#[inline]
fn pb_ptr(pp: &mut PointerBlock, idx: usize) -> &'static mut PointerEntry {
    unsafe { &mut *(&raw mut pp.pb_pointer).cast::<PointerEntry>().add(idx) }
}

#[inline]
fn db_idx_ptr(dp: &mut DataBlock, idx: usize) -> *mut c_uint {
    unsafe { (&raw mut dp.db_index).cast::<c_uint>().add(idx) }
}


#[inline]
fn sb_push_cstr(sb: &mut Vec<u8>, s: impl crate::strings::IntoCCharPtr) { let s = s.into_c_char_ptr();
    if !s.is_null() {
         {
            sb.extend_from_slice(crate::memory::cstr_as_bytes(s));
        }
    }
}

/// Open a new memline for `buf`.
pub fn ml_open(buf: &mut FileBuf) -> bool {
    unsafe {
        buf.b_ml.ml_stack.clear();
        buf.b_ml.ml_stack_top = 0;
        buf.b_ml.ml_locked = ptr::null_mut();
        buf.b_ml.ml_line_lnum = 0;
        buf.b_ml.ml_line_offset = 0;
        buf.b_ml.ml_chunksize.clear();
        buf.b_ml.ml_usedchunks = 0;

        if (g_st().cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0 {
            buf.b_p_swf = 0;
        }

        buf.b_may_swap = buf.terminal.is_null() && opt_vars().p_uc != 0 && buf.b_p_swf != 0;

        buf.b_ml.ml_mfp = mf_open(None, 0);
        let Some(mfp_box) = buf.b_ml.ml_mfp.as_deref_mut() else {
            return false;
        };
        let mfp: *mut Memfile = &raw mut *mfp_box;
        buf.b_ml.ml_flags = ML_EMPTY;
        buf.b_ml.ml_line_count = 1;

        let mut hp = mf_new(&mut *mfp, false, 1);
        if (*hp).bh_bnum != 0 {
            iemsg(tr(c"E298: Didn't get block nr 0?"));
            return ml_open_error(buf, mfp.as_mut(), hp);
        }
        let b0p = (*hp).data_ptr().cast::<ZeroBlock>();

        (*b0p).b0_id[0] = BLOCK0_ID0;
        (*b0p).b0_id[1] = BLOCK0_ID1;
        (*b0p).b0_magic_long = B0_MAGIC_LONG;
        (*b0p).b0_magic_int = B0_MAGIC_INT;
        (*b0p).b0_magic_short = B0_MAGIC_SHORT;
        (*b0p).b0_magic_char = B0_MAGIC_CHAR;
        let ver_dst = (&raw mut (*b0p).b0_version).cast::<c_char>();
        ptr::copy_nonoverlapping(c"VIM ".as_ptr(), ver_dst, 5);
        let ver_dst = ver_dst.add(4);
        xstrlcpy(ver_dst, VERSIONS[0].as_ptr(), 6);
        long_to_char(
            (*mfp).mf_page_size as c_long,
            (&raw mut (*b0p).b0_page_size).cast(),
        );

        if !buf.b_spell {
            (*b0p).set_dirty(if buf.b_changed != 0 { B0_DIRTY } else { 0 });
            (*b0p).set_flags((get_fileformat(&*buf) + 1) as u8);
            set_b0_fname(&mut *b0p, &mut *buf);
            os_get_username((&raw mut (*b0p).b0_uname).cast(), B0_UNAME_SIZE);
            (*b0p).b0_uname[B0_UNAME_SIZE - 1] = NUL;
            os_get_hostname((&raw mut (*b0p).b0_hname).cast(), B0_HNAME_SIZE);
            (*b0p).b0_hname[B0_HNAME_SIZE - 1] = NUL;
            long_to_char(os_get_pid() as c_long, (&raw mut (*b0p).b0_pid).cast());
        }

        mf_put( &mut *mfp, hp, true, false);
        if !buf.b_help && !buf.b_spell {
            mf_sync(&mut *mfp, 0);
        }

        hp = ml_new_ptr(&mut *mfp);
        assert!(!hp.is_null());
        if (*hp).bh_bnum != 1 {
            iemsg(tr(c"E298: Didn't get block nr 1?"));
            return ml_open_error(buf, mfp.as_mut(), hp);
        }
        let pp = (*hp).data_ptr().cast::<PointerBlock>();
        (*pp).pb_count = 1;
        let pe0 = pb_ptr(&mut *pp, 0);
        pe0.pe_bnum = 2;
        pe0.pe_page_count = 1;
        pe0.pe_old_lnum = 1;
        pe0.pe_line_count = 1;
        mf_put( &mut *mfp, hp, true, false);

        hp = ml_new_data(&mut *mfp, false, 1);
        if (*hp).bh_bnum != 2 {
            iemsg(tr(c"E298: Didn't get block nr 2?"));
            return ml_open_error(buf, mfp.as_mut(), hp);
        }

        let dp = (*hp).data_ptr().cast::<DataBlock>();
        (*dp).db_txt_start -= 1;
        *db_idx_ptr(&mut *dp, 0) = (*dp).db_txt_start;
        (*dp).db_free -= 1 + INDEX_SIZE as c_uint;
        (*dp).db_line_count = 1;
        *dp.cast::<c_char>().add((*dp).db_txt_start as usize) = NUL;

        true
    }
}

fn ml_open_error(buf: &mut FileBuf, mfp: Option<&mut Memfile>, hp: *mut Bhdr) -> bool {
    if let Some(mfp) = mfp {
            if !hp.is_null() {
                mf_put( mfp, hp, false, false);
            }
            mf_close(buf.b_ml.ml_mfp.take(), true);
        }
        false
}

/// Called when the file name of `buf` has been changed.
pub fn ml_setname(buf: &mut FileBuf) {
    unsafe {
        let mut success = false;
        let mfp = buf.b_ml.mfp_ptr();
        if (*mfp).mf_fd < 0 {
            if opt_vars().p_uc != 0 && (g_st().cmdmod.cmod_flags & CMOD_NOSWAPFILE) == 0 {
                ml_open_file(buf);
            }
            return;
        }

        let mut dirp = opt_vars().p_dir;
        let mut found_existing_dir = false;
        loop {
            if crate::memory::cstr_is_empty(dirp) {
                break;
            }
            let mf_fname_ptr = (*mfp)
                .mf_fname
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            let fname = findswapname(
                buf,
                &raw mut dirp,
                mf_fname_ptr,
                &mut found_existing_dir,
            );
            if buf.b_ml.mfp_ptr() != mfp {
                return;
            }
            if dirp.is_null() {
                break;
            }
            let Some(fname) = fname else {
                continue;
            };

            if path_equal(fname.as_ptr(), mf_fname_ptr, K_PATH_CMP_LITERAL) {
                success = true;
                break;
            }
            if (*mfp).mf_fd >= 0 {
                close((*mfp).mf_fd);
                (*mfp).mf_fd = -1;
            }

            if vim_rename(mf_fname_ptr, fname.as_ptr()) == 0 {
                success = true;
                mf_free_fnames(&mut *mfp);
                mf_set_fnames(&mut *mfp, fname);
                ml_upd_block0(buf, UpdBlock0::UbSameDir);
                break;
            }
        }

        if (*mfp).mf_fd == -1 {
            (*mfp).mf_fd = os_open((*mfp).mf_fname.as_ref().unwrap().as_ptr(), O_RDWR, 0);
            if (*mfp).mf_fd < 0 {
                emsg(tr(c"E301: Oops, lost the swap file!!!"));
                return;
            }
            os_set_cloexec((*mfp).mf_fd);
        }
        if !success {
            emsg(tr(c"E302: Could not rename swap file"));
        }
    }
}

/// Open a file for the memfile for all buffers that are not readonly or have been modified.
pub fn ml_open_files() {
     {
        for buf in for_all_buffers_mut() {
            if buf.b_p_ro == 0 || buf.b_changed != 0 {
                ml_open_file(buf);
            }
        }
    }
}

/// Open a swapfile for an existing memfile, if there is no swapfile yet.
pub fn ml_open_file(buf: &mut FileBuf) {
    unsafe {
        let mfp = buf.b_ml.mfp_ptr();
        if mfp.is_null()
            || (*mfp).mf_fd >= 0
            || buf.b_p_swf == 0
            || (g_st().cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0
            || !buf.terminal.is_null()
        {
            return;
        }

        if buf.b_spell {
            if let Some(fname) = vim_tempname() {
                mf_open_file(&mut *mfp, fname);
            }
            buf.b_may_swap = false;
            return;
        }

        let mut dirp = opt_vars().p_dir;
        let mut found_existing_dir = false;
        loop {
            if crate::memory::cstr_is_empty(dirp) {
                break;
            }
            let fname = findswapname(
                buf,
                &raw mut dirp,
                ptr::null_mut(),
                &mut found_existing_dir,
            );
            if buf.b_ml.mfp_ptr() != mfp {
                return;
            }
            if dirp.is_null() {
                break;
            }
            let Some(fname) = fname else {
                continue;
            };
            if mf_open_file(&mut *mfp, fname) {
                (*mfp).mf_dirty = MF_DIRTY_YES_NOSYNC;
                ml_upd_block0(buf, UpdBlock0::UbSameDir);

                if mf_sync(&mut *mfp, MFS_ZERO) {
                    mf_set_dirty(&mut *mfp);
                    break;
                }
                mf_close_file(buf, false);
            }
        }

        if !crate::memory::cstr_is_empty(opt_vars().p_dir) && (*mfp).mf_fname.is_none() {
            g_st().need_wait_return = true;
            g_st().no_wait_return += 1;
            let sp = buf_spname(buf);
            semsg!(
                tr(c"E303: Unable to open swap file for \"%s\", recovery impossible"),
                if !sp.is_null() { sp } else { buf.b_fname },
            );
            g_st().no_wait_return -= 1;
        }

        buf.b_may_swap = false;
    }
}

/// If still need to create a swapfile, create a swapfile now.
pub fn check_need_swap(newfile: bool) {
     {
        let old_msg_silent = g_st().msg_silent;
        g_st().msg_silent = 0;

        if curbuf_mut().b_may_swap && (curbuf_mut().b_p_ro == 0 || !newfile) {
            ml_open_file(curbuf_mut());
        }

        g_st().msg_silent = old_msg_silent;
    }
}

/// Close memline for buffer `buf`.
pub fn ml_close(buf: &mut FileBuf, del_file: c_int) {
     {
        if buf.b_ml.ml_mfp.is_none() {
            return;
        }
        mf_close(buf.b_ml.ml_mfp.take(), del_file != 0);
        buf.b_ml.ml_line_buf = Vec::new();
        buf.b_ml.ml_stack = Vec::new();
        buf.b_ml.ml_chunksize = Vec::new();

        buf.b_flags &= !BF_RECOVERED;
    }
}

/// Close all existing memlines and memfiles.
pub fn ml_close_all(del_file: bool) {
     {
        for buf in for_all_buffers_ptr() {
            ml_close(unsafe { &mut *buf }, c_int::from(del_file));
        }
        spell_delete_wordlist();
        vim_deltempdir();
    }
}

/// Close all memfiles for not modified buffers.
pub fn ml_close_notmod() {
    unsafe {
        for buf in for_all_buffers_ptr() {
            if !buf_is_changed_fn(&mut *buf) {
                ml_close( &mut *buf, 1);
            }
        }
    }
}

/// Update the timestamp in the `.swp` file.
pub fn ml_timestamp(buf: &mut FileBuf) {
     {
        ml_upd_block0(buf, UpdBlock0::UbFname);
    }
}

#[inline]
fn ml_check_b0_id(b0p: &ZeroBlock) -> bool {
    b0p.b0_id[0] == BLOCK0_ID0 && b0p.b0_id[1] == BLOCK0_ID1
}

#[inline]
fn ml_check_b0_strings(b0p: &ZeroBlock) -> bool {
    {
        let b0 = b0p;
        b0.b0_version.contains(&NUL)
            && b0.b0_uname.contains(&NUL)
            && b0.b0_hname.contains(&NUL)
            && b0.b0_fname[..B0_FNAME_SIZE_CRYPT].contains(&NUL)
    }
}

fn ml_upd_block0(buf: &mut FileBuf, what: UpdBlock0) {
    unsafe {
        let mfp = buf.b_ml.mfp_ptr();
        if mfp.is_null() {
            return;
        }
        let hp = mf_get(&mut *mfp, 0, 1);
        if hp.is_null() {
            return;
        }
        let b0p = (*hp).data_ptr().cast::<ZeroBlock>();
        if !ml_check_b0_id(&*b0p) {
            iemsg(tr(c"E304: ml_upd_block0(): Didn't get block 0??"));
        } else if what == UpdBlock0::UbFname {
            set_b0_fname(&mut *b0p, &mut *buf);
        } else {
            set_b0_dir_flag(&mut *b0p, buf);
        }
        mf_put( &mut *mfp, hp, true, false);
    }
}

fn set_b0_fname(b0p: &mut ZeroBlock,buf: &mut FileBuf) {
    unsafe {
        if buf.b_ffname.is_null() {
            b0p.b0_fname[0] = NUL;
        } else {
            let mut uname = [0 as c_char; B0_UNAME_SIZE];
            let fname_ptr = (&raw mut b0p.b0_fname).cast::<c_char>();
            let flen = home_replace(
                None,
                buf.b_ffname,
                fname_ptr,
                B0_FNAME_SIZE_CRYPT,
                true,
            );
            if b0p.b0_fname[0] == b'~' as c_char {
                let retval = os_get_username((&raw mut uname).cast(), B0_UNAME_SIZE);
                let ulen = crate::memory::cstr_len((&raw const uname).cast());
                if !retval || ulen + flen > B0_FNAME_SIZE_CRYPT - 1 {
                    xstrlcpy(fname_ptr, buf.b_ffname, B0_FNAME_SIZE_CRYPT);
                } else {
                    ptr::copy(fname_ptr.add(1), fname_ptr.add(ulen + 1), flen);
                    ptr::copy_nonoverlapping((&raw const uname).cast(), fname_ptr.add(1), ulen);
                }
            }
            let mut file_info = FileInfo::new();
            if os_fileinfo(buf.b_ffname, &mut file_info) {
                long_to_char(
                    file_info.stat.st_mtim.tv_sec as c_long,
                    (&raw mut b0p.b0_mtime).cast(),
                );
                long_to_char(
                    os_fileinfo_inode(&file_info) as c_long,
                    (&raw mut b0p.b0_ino).cast(),
                );
                buf_store_file_info(buf, &mut file_info);
                buf.b_mtime_read = buf.b_mtime;
                buf.b_mtime_read_ns = buf.b_mtime_ns;
            } else {
                long_to_char(0, (&raw mut b0p.b0_mtime).cast());
                long_to_char(0, (&raw mut b0p.b0_ino).cast());
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
}

fn set_b0_dir_flag(b0p: &mut ZeroBlock, buf: &mut FileBuf) {
    unsafe {
        let mfp = buf.b_ml.mfp_ptr();
        let mf_fname_ptr = (*mfp)
            .mf_fname
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        if same_directory(mf_fname_ptr, buf.b_ffname) {
            b0p.set_flags(b0p.flags() | B0_SAME_DIR);
        } else {
            b0p.set_flags(b0p.flags() & !B0_SAME_DIR);
        }
    }
}

fn add_b0_fenc(b0p: &mut ZeroBlock, buf: &mut FileBuf) {
    unsafe {
        let size = B0_FNAME_SIZE_NOCRYPT;
        let n = crate::memory::cstr_len(buf.b_p_fenc);
        let fname_ptr = (&raw mut b0p.b0_fname).cast::<c_char>();
        if crate::memory::cstr_len(fname_ptr) + n + 1 > size {
            b0p.set_flags(b0p.flags() & !B0_HAS_FENC);
        } else {
            ptr::copy(buf.b_p_fenc, fname_ptr.add(size - n), n);
            *fname_ptr.add(size - n - 1) = NUL;
            b0p.set_flags(b0p.flags() | B0_HAS_FENC);
        }
    }
}

fn swapfile_proc_running(b0p: &ZeroBlock, swap_fname: impl crate::strings::IntoCCharPtr) -> c_int { let swap_fname = swap_fname.into_c_char_ptr();
    unsafe {
        let mut st = FileInfo::new();
        let mut uptime: f64 = 0.0;
        if os_fileinfo(swap_fname, &mut st)
            && uv_uptime(&mut uptime) == 0
            && (st.stat.st_mtim.tv_sec as Timestamp) < os_time().wrapping_sub(uptime as Timestamp)
        {
            return 0;
        }
        let pid = char_to_long((&raw const b0p.b0_pid).cast()) as c_int;
        if os_proc_running(pid) { pid } else { 0 }
    }
}

/// Try to recover `curbuf` from the `.swp` file.
pub fn ml_recover(checkext: bool) {
    unsafe {
        let mut buf_box: Option<Box<FileBuf>> = None;
        let mut mfp: *mut Memfile = ptr::null_mut();
        let fname_used: Option<std::ffi::CString>;
        let mut hp: *mut Bhdr = ptr::null_mut();
        let mut b0_fenc: Option<std::ffi::CString> = None;
        let directly: bool;
        let mut serious_error = true;
        let mut orig_file_status = NOTDONE;

        g_st().recoverymode = true;
        let called_from_main = curbuf_mut().b_ml.ml_mfp.is_none();

        let mut fname = curbuf_mut().b_fname;
        if fname.is_null() {
            fname = c"".as_ptr().cast_mut();
        }
        let len = crate::memory::cstr_len(fname);
        if checkext
            && len >= 4
            && crate::memory::cstr_prefix_bytes(fname.add(len - 4), 2).eq_ignore_ascii_case(b".s")
            && !vim_strchr(
                c"abcdefghijklmnopqrstuvw",
                ((crate::memory::cstr_byte_at(fname, len - 2)).to_ascii_lowercase()) as c_int,
            )
            .is_null()
            && ascii_isalpha(crate::memory::cstr_char_at(fname, len - 1))
        {
            directly = true;
            fname_used = Some(crate::memory::cstr_to_cstring(fname));
        } else {
            directly = false;
            let mut items_tv = TypVal::new();
            tv_list_alloc_ret(&mut items_tv, 0);
            recover_names(fname, true,  &mut *items_tv.vval.v_list());
            let n_swaps = tv_list_len(items_tv.vval.v_list().as_ref());

            if n_swaps == 0 {
                tv_clear(&mut items_tv);
                semsg!(tr(c"E305: No swap file found for %s"), fname);
                ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
                return;
            }
            if n_swaps > 1 {
                let mut lua_args = [items_tv, TypVal::new()];
                nlua_call_typval(
                    c"vim._core.swapfile",
                    c"select_swap",
                    (&raw mut lua_args).cast(),
                    None,
                );
                tv_clear(&mut items_tv);
                ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
                return;
            }
            let first_str = (*tv_list_first(items_tv.vval.v_list().as_ref())).li_tv.vval.v_string();
            fname_used = (!first_str.is_null()).then(|| crate::memory::cstr_to_cstring(first_str));
            tv_clear(&mut items_tv);
        }
        let Some(fname_used) = fname_used else {
            ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
            return;
        };
        if called_from_main && !ml_open(curbuf_mut()) {
            getout(1);
        }

        let mut allocated_buf = Box::new(FileBuf::new());
        let buf: *mut FileBuf = &raw mut *allocated_buf;
        buf_box = Some(allocated_buf);

        (*buf).b_ml.ml_mfp = mf_open(Some(fname_used.clone()), O_RDONLY);
        mfp = (*buf).b_ml.mfp_ptr();
        if mfp.is_null() || (*mfp).mf_fd < 0 {
            semsg!(tr(c"E306: Cannot open %s"), fname_used.as_ptr());
            ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
            return;
        }
        (*mfp).mf_page_size = MIN_SWAP_PAGE_SIZE;

        let hl_id = HLF_E;
        msg_ext_set_kind(c"emsg");
        hp = mf_get(&mut *mfp, 0, 1);
        if hp.is_null() {
            msg_start();
            msg_puts_hl(tr(c"Unable to read block 0 from "), hl_id, true);
            msg_outtrans(
                (*mfp)
                    .mf_fname
                    .as_ref()
                    .map_or(c"".as_ptr(), |s| s.as_ptr()),
                hl_id,
                true,
            );
            msg_puts_hl(
                tr(c"\nMaybe no changes were made or Nvim did not update the swap file."),
                hl_id,
                true,
            );
            msg_end();
            ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
            return;
        }
        let mut b0p = (*hp).data_ptr().cast::<ZeroBlock>();
        if (&(*b0p).b0_version)[..7]
            == [
                b'V' as c_char,
                b'I' as c_char,
                b'M' as c_char,
                b' ' as c_char,
                b'3' as c_char,
                b'.' as c_char,
                b'0' as c_char,
            ]
        {
            msg_start();
            msg_outtrans(
                (*mfp)
                    .mf_fname
                    .as_ref()
                    .map_or(c"".as_ptr(), |s| s.as_ptr()),
                0,
                true,
            );
            msg_puts_hl(tr(c" cannot be used with this version of Nvim.\n"), 0, true);
            msg_puts_hl(tr(c"Use Vim version 3.0.\n"), 0, true);
            msg_end();
            ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
            return;
        }
        if !ml_check_b0_id(&*b0p) {
            semsg!(
                tr(c"E307: %s does not look like a Nvim swap file"),
                (*mfp)
                    .mf_fname
                    .as_ref()
                    .map_or(c"".as_ptr(), |s| s.as_ptr()),
            );
            ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
            return;
        }
        if b0_magic_wrong(&*b0p) {
            msg_start();
            msg_outtrans(
                (*mfp)
                    .mf_fname
                    .as_ref()
                    .map_or(c"".as_ptr(), |s| s.as_ptr()),
                hl_id,
                true,
            );
            msg_puts_hl(tr(c" cannot be used on this computer.\n"), hl_id, true);
            msg_puts_hl(tr(c"The file was created on "), hl_id, true);
            (*b0p).b0_fname[0] = NUL;
            msg_puts_hl((&raw const (*b0p).b0_hname).cast(), hl_id, true);
            msg_puts_hl(tr(c",\nor the file has been damaged."), hl_id, true);
            msg_end();
            ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
            return;
        }

        let b0_page_size = char_to_long((&raw const (*b0p).b0_page_size).cast()) as c_uint;
        if (*mfp).mf_page_size != b0_page_size {
            let previous_page_size = (*mfp).mf_page_size;
            mf_new_page_size(&mut *mfp, b0_page_size);
            if (*mfp).mf_page_size < previous_page_size {
                msg_start();
                msg_outtrans(
                    (*mfp)
                        .mf_fname
                        .as_ref()
                        .map_or(c"".as_ptr(), |s| s.as_ptr()),
                    hl_id,
                    true,
                );
                msg_puts_hl(
                    tr(c" has been damaged (page size is smaller than minimum value).\n"),
                    hl_id,
                    true,
                );
                msg_end();
                ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
                return;
            }
            let size: Off = vim_lseek((*mfp).mf_fd, 0, SEEK_END);
            (*mfp).mf_blocknr_max = if size <= 0 {
                0
            } else {
                size / i64::from((*mfp).mf_page_size)
            };
            (*mfp).mf_infile_count = (*mfp).mf_blocknr_max;

            (*hp).bh_data.resize((*mfp).mf_page_size as usize, 0);
            b0p = (*hp).data_ptr().cast::<ZeroBlock>();
        }

        let namebuff_ptr = g_st().name_buff.as_mut_ptr();
        if directly {
            #[cfg(windows)]
            path_to_slash((&raw mut (*b0p).b0_fname).cast());
            expand_env(
                (&raw mut (*b0p).b0_fname).cast(),
                namebuff_ptr,
                MAXPATHL as c_int,
            );
            if !setfname(curbuf_mut(), namebuff_ptr, ptr::null_mut(), true) {
                ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
                return;
            }
        }

        msg_ext_set_kind(c"wmsg");
        g_st().msg_ext_skip_flush = true;
        let mf_fname_ptr = (*mfp)
            .mf_fname
            .as_ref()
            .map_or(c"".as_ptr().cast_mut(), |s| s.as_ptr().cast_mut());
        home_replace(None, mf_fname_ptr, namebuff_ptr, MAXPATHL, true);
        smsg!(0, tr(c"Using swap file \"%s\""), namebuff_ptr);

        let sp = buf_spname(g_st().curbuf);
        if !sp.is_null() {
            xstrlcpy(namebuff_ptr, sp, MAXPATHL);
        } else {
            home_replace(
                None,
                curbuf_mut().b_ffname,
                namebuff_ptr,
                MAXPATHL,
                true,
            );
        }
        msg_putchar(b'\n' as c_int);
        smsg!(0, tr(c"Original file \"%s\""), namebuff_ptr);
        msg_putchar(b'\n' as c_int);
        g_st().msg_ext_skip_flush = false;

        let mut org_file_info = FileInfo::new();
        let mut swp_file_info = FileInfo::new();
        let mtime = char_to_long((&raw const (*b0p).b0_mtime).cast()) as i64;
        if !curbuf_mut().b_ffname.is_null()
            && os_fileinfo(curbuf_mut().b_ffname, &mut org_file_info)
            && ((os_fileinfo(mf_fname_ptr, &mut swp_file_info)
                && org_file_info.stat.st_mtim.tv_sec > swp_file_info.stat.st_mtim.tv_sec)
                || mtime != org_file_info.stat.st_mtim.tv_sec as _)
        {
            emsg(tr(c"E308: Warning: Original file may have been changed"));
        }
        ui_flush();

        let b0_ff = ((*b0p).flags() & B0_FF_MASK) as c_int;
        if ((*b0p).flags() & B0_HAS_FENC) != 0 {
            let fnsize = B0_FNAME_SIZE_NOCRYPT;
            let fname_base = (&raw const (*b0p).b0_fname).cast::<c_char>();
            let mut p = fname_base.add(fnsize);
            while p > fname_base && *p.offset(-1) != NUL {
                p = p.offset(-1);
            }
            let fenc_len = fname_base.add(fnsize).offset_from(p) as usize;
            let fenc_bytes = crate::memory::cstr_slice(p, fenc_len);
            let end = fenc_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(fenc_bytes.len());
            b0_fenc = Some(std::ffi::CString::new(&fenc_bytes[..end]).unwrap_or_default());
        }

        mf_put( &mut *mfp, hp, false, false);
        hp = ptr::null_mut();

        while (curbuf_mut().b_ml.ml_flags & ML_EMPTY) == 0 {
            ml_delete(1);
        }

        if !curbuf_mut().b_ffname.is_null() {
            orig_file_status = readfile(
                curbuf_mut().b_ffname,
                ptr::null_mut(),
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
                data: ObjectData::String(cstr_as_string(fenc.as_ptr())),
            };
            set_option_value_give_err(K_OPT_FILEENCODING, fenc_obj, OPT_LOCAL);
        }
        unchanged(curbuf_mut(), true, true);

        let mut bnum: Blocknr = 1;
        let mut page_count: c_uint = 1;
        let mut lnum: Linenr = 0;
        let mut line_count: Linenr = 0;
        let mut idx: c_int = 0;
        let mut error: c_int = 0;
        (*buf).b_ml.ml_stack_top = 0;
        (*buf).b_ml.ml_stack.clear();

        let mut cannot_open = curbuf_mut().b_ffname.is_null();

        serious_error = false;
        while !g_st().got_int {
            if !hp.is_null() {
                mf_put( &mut *mfp, hp, false, false);
            }
            hp = mf_get(&mut *mfp, bnum, page_count);
            if hp.is_null() {
                if bnum == 1 {
                    semsg!(
                        tr(c"E309: Unable to read block 1 from %s"),
                        (*mfp)
                            .mf_fname
                            .as_ref()
                            .map_or(c"".as_ptr(), |s| s.as_ptr()),
                    );
                    ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
                    return;
                }
                error += 1;
                ml_append(lnum, tr(c"???MANY LINES MISSING"), 0, true);
                lnum += 1;
            } else {
                let pp = (*hp).data_ptr().cast::<PointerBlock>();
                if (*pp).pb_id == PTR_ID {
                    let mut ptr_block_error = false;
                    let max_cnt = pb_count_max(&*mfp);
                    if (*pp).pb_count_max != max_cnt {
                        ptr_block_error = true;
                        (*pp).pb_count_max = max_cnt;
                    }
                    if (*pp).pb_count > (*pp).pb_count_max {
                        ptr_block_error = true;
                        (*pp).pb_count = (*pp).pb_count_max;
                    }
                    if ptr_block_error {
                        emsg(tr(E_WARNING_POINTER_BLOCK_CORRUPTED));
                    }

                    if idx == 0 && line_count != 0 {
                        for i in 0..((*pp).pb_count as usize) {
                            line_count -= pb_ptr(&mut *pp, i).pe_line_count;
                        }
                        if line_count != 0 {
                            error += 1;
                            ml_append(lnum, tr(c"???LINE COUNT WRONG"), 0, true);
                            lnum += 1;
                        }
                    }

                    if (*pp).pb_count == 0 {
                        ml_append(lnum, tr(c"???EMPTY BLOCK"), 0, true);
                        lnum += 1;
                        error += 1;
                    } else if idx < c_int::from((*pp).pb_count) {
                        let pe = pb_ptr(&mut *pp, idx as usize);
                        if pe.pe_bnum < 0 {
                            if !cannot_open {
                                line_count = pe.pe_line_count;
                                let pe_old_lnum = pe.pe_old_lnum;
                                if line_count <= 0
                                    || pe_old_lnum < 1
                                    || readfile(
                                        curbuf_mut().b_ffname,
                                        ptr::null_mut(),
                                        lnum,
                                        pe_old_lnum - 1,
                                        line_count,
                                        None,
                                        0,
                                        false,
                                    ) != OK
                                {
                                    cannot_open = true;
                                } else {
                                    lnum += line_count;
                                }
                            }
                            if cannot_open {
                                error += 1;
                                ml_append(lnum, tr(c"???LINES MISSING"), 0, true);
                                lnum += 1;
                            }
                            idx += 1;
                            line_breakcheck();
                            continue;
                        }

                        let top = ml_add_stack(&mut *buf);
                        let ip = &mut (&mut (*buf).b_ml.ml_stack)[top as usize];
                        ip.ip_bnum = bnum;
                        ip.ip_index = idx;

                        bnum = pe.pe_bnum;
                        line_count = pe.pe_line_count;
                        page_count = pe.pe_page_count as c_uint;
                        if page_count < 1
                            || bnum.saturating_add(i64::from(page_count))
                                > (*mfp).mf_blocknr_max.saturating_add(1)
                        {
                            error += 1;
                            ml_append(lnum, tr(c"???ILLEGAL BLOCK NUMBER"), 0, true);
                            lnum += 1;
                            idx = ip.ip_index + 1;
                            bnum = ip.ip_bnum;
                            page_count = 1;
                            (*buf).b_ml.ml_stack_top -= 1;
                            line_breakcheck();
                            continue;
                        }
                        idx = 0;
                        line_breakcheck();
                        continue;
                    }
                } else {
                    let dp = (*hp).data_ptr().cast::<DataBlock>();
                    if (*dp).db_id != DATA_ID {
                        if bnum == 1 {
                            semsg!(
                                tr(c"E310: Block 1 ID wrong (%s not a .swp file?)"),
                                (*mfp)
                                    .mf_fname
                                    .as_ref()
                                    .map_or(c"".as_ptr(), |s| s.as_ptr()),
                            );
                            ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
                            return;
                        }
                        error += 1;
                        ml_append(lnum, tr(c"???BLOCK MISSING"), 0, true);
                        lnum += 1;
                    } else {
                        let mut has_error = false;
                        if (*hp).bh_page_count != page_count {
                            error += 1;
                            ml_append(lnum, tr(c"??? BLOCK PAGE COUNT MISMATCH"), 0, true);
                            lnum += 1;
                            page_count = (*hp).bh_page_count;
                        }

                        if page_count.wrapping_mul((*mfp).mf_page_size) != (*dp).db_txt_end {
                            ml_append(
                                lnum,
                                tr(c"??? from here until ???END lines may be messed up"),
                                0,
                                true,
                            );
                            lnum += 1;
                            error += 1;
                            has_error = true;
                            (*dp).db_txt_end = page_count.wrapping_mul((*mfp).mf_page_size);
                        }

                        if ((*dp).db_txt_start as usize) < HEADER_SIZE
                            || (*dp).db_txt_start > (*dp).db_txt_end
                        {
                            ml_append(lnum, tr(c"??? block header corrupted"), 0, true);
                            lnum += 1;
                            error += 1;
                            has_error = true;
                            (*dp).db_txt_start = (*dp).db_txt_end;
                        }
                        *dp.cast::<c_char>().add((*dp).db_txt_end as usize - 1) = NUL;

                        if c_long::from(line_count) != (*dp).db_line_count {
                            ml_append(
                                lnum,
                                tr(c"??? from here until ???END lines may have been inserted/deleted"),
                                0,
                                true,
                            );
                            lnum += 1;
                            error += 1;
                            has_error = true;
                        }

                        let mut did_questions = false;
                        for i in 0..((*dp).db_line_count as isize) {
                            let idx_ptr = db_idx_ptr(&mut *dp, i as usize);
                            if idx_ptr.cast::<c_char>()
                                >= dp.cast::<c_char>().add((*dp).db_txt_start as usize)
                            {
                                error += 1;
                                ml_append(lnum, tr(c"??? lines may be missing"), 0, true);
                                lnum += 1;
                                break;
                            }

                            let txt_start = (*idx_ptr & DB_INDEX_MASK) as c_int;
                            let line_p: *mut c_char;
                            if txt_start <= HEADER_SIZE as c_int
                                || txt_start >= (*dp).db_txt_end as c_int
                            {
                                error += 1;
                                if did_questions {
                                    continue;
                                }
                                did_questions = true;
                                line_p = c"???".as_ptr().cast_mut();
                            } else {
                                did_questions = false;
                                line_p = dp.cast::<c_char>().add(txt_start as usize);
                            }
                            ml_append(lnum, line_p, 0, true);
                            lnum += 1;
                        }
                        if has_error {
                            ml_append(lnum, tr(c"???END"), 0, true);
                            lnum += 1;
                        }
                    }
                }
            }

            if (*buf).b_ml.ml_stack_top == 0 {
                break;
            }

            (*buf).b_ml.ml_stack_top -= 1;
            let ip = (*buf)
                .b_ml
                .ml_stack
                .as_mut_ptr()
                .add((*buf).b_ml.ml_stack_top as usize);
            bnum = (*ip).ip_bnum;
            idx = (*ip).ip_index + 1;
            page_count = 1;
            line_breakcheck();
        }

        if orig_file_status != OK || curbuf_mut().b_ml.ml_line_count != lnum * 2 + 1 {
            if !(curbuf_mut().b_ml.ml_line_count == 2 && *ml_get(1) == NUL) {
                changed_internal(curbuf_mut());
                buf_inc_changedtick(curbuf_mut());
            }
        } else {
            for i_lnum in 1..=lnum {
                let p_line = crate::memory::cstr_to_cstring(ml_get(i_lnum));
                let cmp = crate::memory::cstr_as_bytes(p_line.as_ptr()).cmp(crate::memory::cstr_as_bytes(ml_get(i_lnum + lnum))) as c_int;
                if cmp != 0 {
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
            msg_putchar(b'\n' as c_int);
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
            if swapfile_proc_running( &*b0p, fname_used.as_ptr().cast_mut()) != 0 {
                msg_puts(tr(c"\nNote: process STILL RUNNING: "));
                msg_outnum(char_to_long((&raw const (*b0p).b0_pid).cast()) as c_int);
            }
            if !ui_has(K_UIMESSAGES) {
                msg_puts(c"\n\n");
            }
            g_st().cmdline_row = g_st().msg_row;
        }
        redraw_curbuf_later(UPD_NOT_VALID);

        ml_recover_theend(mfp.as_mut(), hp, buf_box, serious_error, called_from_main);
    }
}

fn ml_recover_theend(
    mfp: Option<&mut Memfile>,
    hp: *mut Bhdr,
    mut buf: Option<Box<FileBuf>>,
    serious_error: bool,
    called_from_main: bool,
) {
     {
        g_st().msg_ext_skip_flush = false;
        g_st().recoverymode = false;
        if let Some(mfp) = mfp {
            if !hp.is_null() {
                mf_put(mfp, hp, false, false);
            }
            mf_close(buf.as_mut().and_then(|b| b.b_ml.ml_mfp.take()), false);
        }
        drop(buf);
        if serious_error && called_from_main {
            ml_close(curbuf_mut(), 1);
        } else {
            apply_autocmds(
                EVENT_BUFREADPOST,
                ptr::null_mut(),
                curbuf_mut().b_fname,
                false,
                g_st().curbuf,
            );
            apply_autocmds(
                EVENT_BUFWINENTER,
                ptr::null_mut(),
                curbuf_mut().b_fname,
                false,
                g_st().curbuf,
            );
        }
    }
}

/// Enumerate swapfiles for `fname`, appending each found path to `ret_list`.
pub fn recover_names(fname: *mut c_char, skip_curbuf: bool, ret_list: &mut TvList) {
    unsafe {
        #[cfg(unix)]
        let mut fname_buf = [0 as c_char; MAXPATHL];
        let mut fname_res: *mut c_char = ptr::null_mut();

        if !fname.is_null() {
            #[cfg(unix)]
            {
                let buf_ptr = (&raw mut fname_buf).cast::<c_char>();
                fname_res = if resolve_symlink(fname, buf_ptr) {
                    buf_ptr
                } else {
                    fname
                };
            }
            #[cfg(not(unix))]
            {
                fname_res = fname;
            }
        }

        let mut dir_name_vec: Vec<c_char> = vec![0; crate::memory::cstr_len(opt_vars().p_dir) + 1];
        let mut dir_name = NvimString {
            data: dir_name_vec.as_mut_ptr(),
            size: 0,
        };
        let mut dirp = opt_vars().p_dir;
        while !crate::memory::cstr_is_empty(dirp) {
            dir_name.size = copy_option_part(&raw mut dirp, dir_name.data, 31000, c",");

            let mut names_vec: Vec<std::ffi::CString> = Vec::with_capacity(6);
            if crate::memory::cstr_first_byte(dir_name.data) == b'.' && crate::memory::cstr_byte_at(dir_name.data, 1) == 0 {
                if fname.is_null() {
                    names_vec.push(c"*.sw?".to_owned());
                    names_vec.push(c".*.sw?".to_owned());
                    names_vec.push(c".sw?".to_owned());
                } else {
                    recov_file_names(&mut names_vec, fname_res, true);
                }
            } else if fname.is_null() {
                for pat in [c"*.sw?", c".*.sw?", c".sw?"] {
                    names_vec.push(concat_fnames(dir_name, cstr_as_string(pat.as_ptr()), true));
                }
            } else {
                let p = dir_name.data.add(dir_name.size);
                let tail = if after_pathsep(dir_name.data, p) != 0
                    && dir_name.size > 1
                    && *p.offset(-1) == *p.offset(-2)
                {
                    make_percent_swname(dir_name.data, p, fname_res)
                } else {
                    let t = path_tail(fname_res);
                    Some(concat_fnames(dir_name, cstr_as_string(t), true))
                };
                if let Some(ref tail) = tail {
                    recov_file_names(&mut names_vec, tail.as_ptr().cast_mut(), false);
                }
            }

            let mut files: Vec<std::ffi::CString> = Vec::new();
            if !names_vec.is_empty() {
                let mut name_ptrs: Vec<*mut c_char> =
                    names_vec.iter().map(|s| s.as_ptr().cast_mut()).collect();
                let _ = expand_wildcards_vec(
                    name_ptrs.len() as c_int,
                    name_ptrs.as_mut_ptr(),
                    &mut files,
                    EW_KEEPALL | EW_FILE | EW_SILENT,
                );
            }

            let cur_mfp_fname = if skip_curbuf && curbuf_mut().b_ml.ml_mfp.is_some() {
                let cur_mfp = curbuf_mut().b_ml.mfp_ptr();
                (*cur_mfp).mf_fname.as_ref().map(|p| p.as_ptr())
            } else {
                None
            };

            if let Some(cur_p) = cur_mfp_fname {
                files.retain(|f| !path_equal(cur_p, f.as_ptr(), K_PATH_CMP_FULL));
            }

            if crate::memory::cstr_is_empty(dirp)
                && tv_list_len(Some(ret_list)) == 0
                && files.is_empty()
                && !fname.is_null()
                && let Some(swapname) = modname(fname_res, c".swp", true)
                && os_path_exists(swapname.as_ptr())
                && cur_mfp_fname.is_none_or(|cur_p| !path_equal(cur_p, swapname.as_ptr(), K_PATH_CMP_FULL))
            {
                crate::eval::typval::tv_list_append_string(ret_list, swapname.as_ptr(), -1);
            }

            for f in files {
                crate::eval::typval::tv_list_append_string(
                    ret_list,
                    f.as_ptr(),
                    f.as_bytes().len() as isize,
                );
            }
        }
    }
}

/// Append the full path to `name` with path separators made into percent signs to `dir`.
pub fn make_percent_swname(
    dir: *mut c_char,
    dir_end: *mut c_char,
    name: impl crate::strings::IntoCCharPtr,
) -> Option<std::ffi::CString> { let name = name.into_c_char_ptr();
    unsafe {
        let fname = fix_fname(if !name.is_null() { name } else { c"".as_ptr() })?;
        let mut fname_vec: Vec<c_char> = fname
            .into_bytes_with_nul()
            .into_iter()
            .map(|b| b as c_char)
            .collect();
        let fname_ptr = fname_vec.as_mut_ptr();

        let mut file_info = FileInfo::new();
        if !os_fileinfo2(fname_ptr, &mut file_info) {
            return None;
        }
        let mut fixed_data = fname_ptr.add(file_info.root_off);
        if file_info.r#type == K_PATH_DEVICE_UNC {
            assert!(file_info.root_off >= 2);
            fixed_data = fixed_data.sub(2);
            *fixed_data = b'/' as c_char;
        }
        let mut p = fixed_data;
        while !crate::memory::cstr_is_empty(p) {
            if vim_ispathsep(crate::memory::cstr_first_byte(p) as c_int) {
                *p = b'%' as c_char;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        let fixed_fname = NvimString {
            data: fixed_data,
            size: p.offset_from(fixed_data) as usize,
        };

        let last_sep = dir_end.offset(-1);
        *last_sep = NUL;
        Some(concat_fnames(
            NvimString {
                data: dir,
                size: last_sep.offset_from(dir) as usize,
            },
            fixed_fname,
            true,
        ))
    }
}

/// For Vimscript `swapinfo()`.
pub fn swapfile_dict(fname: impl crate::strings::IntoCCharPtr, d: &mut TvDict) { let fname = fname.into_c_char_ptr();
    unsafe {
        let mut b0 = ZeroBlock::new();
        let fd = os_open(fname, O_RDONLY, 0);
        if fd >= 0 {
            if read_eintr(fd, (&raw mut b0).cast(), size_of::<ZeroBlock>())
                == size_of::<ZeroBlock>() as isize
            {
                if !ml_check_b0_id(&b0) {
                    tv_dict_add_str_len(d, c"error", c"Not a swap file", 15);
                } else if b0_magic_wrong(&b0) {
                    tv_dict_add_str_len(
                        d,
                        c"error",
                        c"Magic number mismatch",
                        21,
                    );
                } else {
                    tv_dict_add_str_len(
                        d,
                        c"version",
                        (&raw const b0.b0_version).cast(),
                        10,
                    );
                    tv_dict_add_str_len(
                        d,
                        c"user",
                        (&raw const b0.b0_uname).cast(),
                        B0_UNAME_SIZE as c_int,
                    );
                    tv_dict_add_str_len(
                        d,
                        c"host",
                        (&raw const b0.b0_hname).cast(),
                        B0_HNAME_SIZE as c_int,
                    );
                    tv_dict_add_str_len(
                        d,
                        c"fname",
                        (&raw const b0.b0_fname).cast(),
                        B0_FNAME_SIZE_ORG as c_int,
                    );

                    tv_dict_add_nr(
                        d,
                        c"pid",
                        i64::from(swapfile_proc_running(&b0, fname)),
                    );
                    tv_dict_add_nr(
                        d,
                        c"mtime",
                        char_to_long((&raw const b0.b0_mtime).cast()) as i64,
                    );
                    tv_dict_add_nr(d, c"dirty", if b0.dirty() != 0 { 1 } else { 0 });
                    tv_dict_add_nr(
                        d,
                        c"inode",
                        char_to_long((&raw const b0.b0_ino).cast()) as i64,
                    );
                }
            } else {
                tv_dict_add_str_len(d, c"error", c"Cannot read file", 16);
            }
            close(fd);
        } else {
            tv_dict_add_str_len(d, c"error", c"Cannot open file", 16);
        }
    }
}

fn swapfile_info(fname: impl crate::strings::IntoCCharPtr, msg: &mut Vec<u8>) -> time_t { let fname = fname.into_c_char_ptr();
    unsafe {
        assert!(!fname.is_null());
        let mut b0 = ZeroBlock::new();
        let mut x: time_t = 0;

        let mut file_info = FileInfo::new();
        if os_fileinfo(fname, &mut file_info) {
            #[cfg(unix)]
            {
                let mut uname = [0 as c_char; B0_UNAME_SIZE];
                if os_get_uname(
                    file_info.stat.st_uid as u32,
                    (&raw mut uname).cast(),
                    B0_UNAME_SIZE,
                )
                {
                    sb_push_cstr(msg, tr(c"          owned by: "));
                    sb_push_cstr(msg, (&raw const uname).cast());
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
            let mut ctime_buf = [0 as c_char; 100];
            let ct = os_ctime_r(
                &x,
                (&raw mut ctime_buf).cast(),
                size_of_val(&ctime_buf),
                true,
            );
            sb_push_cstr(msg, ct);
        }

        let fd = os_open(fname, O_RDONLY, 0);
        if fd >= 0 {
            if read_eintr(fd, (&raw mut b0).cast(), size_of::<ZeroBlock>())
                == size_of::<ZeroBlock>() as isize
            {
                if b0.b0_version[..7]
                    == [
                        b'V' as c_char,
                        b'I' as c_char,
                        b'M' as c_char,
                        b' ' as c_char,
                        b'3' as c_char,
                        b'.' as c_char,
                        b'0' as c_char,
                    ]
                {
                    sb_push_cstr(msg, tr(c"         [from Vim version 3.0]"));
                } else if !ml_check_b0_id(&b0) {
                    sb_push_cstr(msg, tr(c"         [does not look like a Nvim swap file]"));
                } else if !ml_check_b0_strings(&b0) {
                    sb_push_cstr(msg, tr(c"         [garbled strings (not nul terminated)]"));
                } else {
                    sb_push_cstr(msg, tr(c"         file name: "));
                    if b0.b0_fname[0] == NUL {
                        sb_push_cstr(msg, tr(c"[No Name]"));
                    } else {
                        sb_push_cstr(msg, (&raw const b0.b0_fname).cast());
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

                    if b0.b0_uname[0] != NUL {
                        sb_push_cstr(msg, tr(c"\n         user name: "));
                        sb_push_cstr(msg, (&raw const b0.b0_uname).cast());
                    }

                    if b0.b0_hname[0] != NUL {
                        if b0.b0_uname[0] != NUL {
                            sb_push_cstr(msg, tr(c"   host name: "));
                        } else {
                            sb_push_cstr(msg, tr(c"\n         host name: "));
                        }
                        sb_push_cstr(msg, (&raw const b0.b0_hname).cast());
                    }

                    let pid = char_to_long((&raw const b0.b0_pid).cast()) as c_int;
                    if pid != 0 {
                        sb_push_cstr(msg, tr(c"\n        process ID: "));
                        msg.extend_from_slice(pid.to_string().as_bytes());
                        ml_st().proc_running = swapfile_proc_running(&b0, fname);
                        if ml_st().proc_running != 0 {
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
            close(fd);
        } else {
            sb_push_cstr(msg, tr(c"         [cannot be opened]"));
        }
        msg.push(b'\n');

        x
    }
}

fn swapfile_unchanged(fname: impl crate::strings::IntoCCharPtr) -> bool { let fname = fname.into_c_char_ptr();
    unsafe {
        if !os_path_exists(fname) {
            return false;
        }
        let fd = os_open(fname, O_RDONLY, 0);
        if fd < 0 {
            return false;
        }
        let mut b0 = ZeroBlock::new();
        if read_eintr(fd, (&raw mut b0).cast(), size_of::<ZeroBlock>())
            != size_of::<ZeroBlock>() as isize
        {
            close(fd);
            return false;
        }

        let mut ret = true;
        if !ml_check_b0_id(&b0) || b0_magic_wrong(&b0) {
            ret = false;
        }
        if b0.dirty() != 0 {
            ret = false;
        }
        if b0.b0_hname[0] == NUL {
            ret = false;
        } else {
            let mut hostname = [0 as c_char; B0_HNAME_SIZE];
            os_get_hostname((&raw mut hostname).cast(), B0_HNAME_SIZE);
            hostname[B0_HNAME_SIZE - 1] = NUL;
            b0.b0_hname[B0_HNAME_SIZE - 1] = NUL;
            if !crate::memory::cstr_as_bytes((&raw const b0.b0_hname).cast()).eq_ignore_ascii_case(crate::memory::cstr_as_bytes((&raw const hostname).cast()))
            {
                ret = false;
            }
        }
        if char_to_long((&raw const b0.b0_pid).cast()) == 0
            || swapfile_proc_running(&b0, fname) != 0
        {
            ret = false;
        }
        close(fd);
        ret
    }
}

fn recov_file_names(
    names: &mut Vec<std::ffi::CString>,
    path: impl crate::strings::IntoCCharPtr,
    prepend_dot: bool,
) { let path = path.into_c_char_ptr();
    if prepend_dot {
            let Some(m) = modname(path, c".sw?", true) else {
                return;
            };
            names.push(m);
        }

        let cur = concat_fnames(
            cstr_as_string(path),
            NvimString {
                data: c".sw?".as_ptr().cast_mut(),
                size: 4,
            },
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
pub fn ml_sync_all(check_file: c_int, check_char: c_int, do_fsync: bool) {
    unsafe {
        for buf in for_all_buffers_ptr() {
            let mfp = (*buf).b_ml.mfp_ptr();
            if mfp.is_null() || (*mfp).mf_fname.is_none() {
                continue;
            }
            ml_flush_line( &mut *buf, false);
            ml_find_line(&mut *buf, 0, ML_FLUSH);
            if buf_is_changed_fn(&mut *buf)
                && check_file != 0
                && mf_need_trans(&mut *mfp)
                && !(*buf).b_ffname.is_null()
            {
                let mut file_info = FileInfo::new();
                if !os_fileinfo((*buf).b_ffname, &mut file_info)
                    || (*buf).b_mtime_read != file_info.stat.st_mtim.tv_sec as _
                    || (*buf).b_mtime_read_ns != file_info.stat.st_mtim.tv_nsec as _
                    || os_fileinfo_size(&file_info) != (*buf).b_orig_size
                {
                    ml_preserve(&mut *buf, false, do_fsync);
                    g_st().did_check_timestamps = false;
                    g_st().need_check_timestamps = true;
                }
            }
            if (*mfp).mf_dirty == MF_DIRTY_YES {
                mf_sync(
                    &mut *mfp,
                    (if check_char != 0 { MFS_STOP } else { 0 })
                        | (if do_fsync && buf_is_changed_fn(&mut *buf) {
                            MFS_FLUSH
                        } else {
                            0
                        }),
                );
                if check_char != 0 && os_char_avail() {
                    break;
                }
            }
        }
    }
}

/// Sync one buffer, including negative blocks.
pub fn ml_preserve(buf: &mut FileBuf, message: bool, do_fsync: bool) {
    unsafe {
        let mfp = buf.b_ml.mfp_ptr();
        let got_int_save = g_st().got_int;

        if mfp.is_null() || (*mfp).mf_fname.is_none() {
            if message {
                emsg(tr(c"E313: Cannot preserve, there is no swap file"));
            }
            return;
        }

        g_st().got_int = false;

        ml_flush_line( &mut *buf, false);
        ml_find_line(buf, 0, ML_FLUSH);
        let mut status = mf_sync(&mut *mfp, MFS_ALL | if do_fsync { MFS_FLUSH } else { 0 });

        buf.b_ml.ml_stack_top = 0;

        if mf_need_trans(&mut *mfp) && !g_st().got_int {
            let mut lnum: Linenr = 1;
            while mf_need_trans(&mut *mfp) && lnum <= buf.b_ml.ml_line_count {
                let hp = ml_find_line(buf, lnum, ML_FIND);
                if hp.is_null() {
                    status = false;
                    break;
                }
                lnum = buf.b_ml.ml_locked_high + 1;
            }
            if status {
                ml_find_line(buf, 0, ML_FLUSH);
                if !mf_sync(&mut *mfp, MFS_ALL | if do_fsync { MFS_FLUSH } else { 0 }) {
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
}

/// Return a pointer to a (read-only copy of a) line in `curbuf`.
pub fn ml_get(lnum: Linenr) -> *mut c_char {
    ml_get_buf_impl(curbuf_mut(), lnum, false)
}

/// Return a pointer to a (read-only copy of a) line in `buf`.
pub fn ml_get_buf(buf: &mut FileBuf, lnum: Linenr) -> *mut c_char {
    ml_get_buf_impl( buf, lnum, false)
}

/// Like `ml_get_buf`, but allow the line to be mutated in place.
pub fn ml_get_buf_mut(buf: &mut FileBuf, lnum: Linenr) -> *mut c_char {
    ml_get_buf_impl( buf, lnum, true)
}

/// Return pointer to position `pos`.
pub fn ml_get_pos(pos: &CursorPos) -> *mut c_char {
    unsafe { ml_get_buf(crate::globals::curbuf_mut(), pos.lnum).offset(pos.col as isize) }
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
     {
        let line = ml_get_buf(buf, lnum);
        if crate::memory::cstr_is_empty(line) {
            return 0;
        }
        assert!(buf.b_ml.ml_line_textlen > 0);
        buf.b_ml.ml_line_textlen - 1
    }
}

/// Gets the charwise NL-joined text of range `[start, end)` as a `Vec<u8>`.
pub fn ml_region_text(buf: &mut FileBuf, start: CursorPos, end: CursorPos) -> Vec<u8> {
    unsafe {
        let mut sb = Vec::<u8>::new();
        for lnum in start.lnum..=end.lnum {
            let line = ml_get_buf(buf, lnum);
            let len = ml_get_buf_len(buf, lnum);
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
            let slice = std::slice::from_raw_parts(
                line.add(from as usize).cast::<u8>(),
                (to - from) as usize,
            );
            sb.extend_from_slice(slice);
            if lnum < end.lnum {
                sb.push(NL as u8);
            }
        }
        sb
    }
}

/// Return codepoint at `pos`.
pub fn gchar_pos(pos: &mut CursorPos) -> c_int {
    if pos.col == MAXCOL || pos.col > ml_get_len(pos.lnum) {
            return c_int::from(NUL);
        }
        utf_ptr2char(ml_get_pos(pos))
}

fn ml_get_buf_impl(buf: &mut FileBuf, mut lnum: Linenr, will_change: bool) -> *mut c_char {
    unsafe {
        if buf.b_ml.ml_mfp.is_none() {
            buf.b_ml.ml_line_textlen = 1;
            return c"".as_ptr().cast_mut();
        }

        if lnum > buf.b_ml.ml_line_count {
            if ml_st().ml_get_recursive == 0 {
                ml_st().ml_get_recursive += 1;
                siemsg!(tr(E_ML_GET_INVALID_LNUM_NR), i64::from(lnum));
                ml_st().ml_get_recursive -= 1;
            }
            ml_flush_line(buf, false);
            return ml_get_errorret(buf, lnum);
        }
        lnum = lnum.max(1);

        if buf.b_ml.ml_line_lnum != lnum {
            ml_flush_line(buf, false);

            let hp = ml_find_line(buf, lnum, ML_FIND);
            if hp.is_null() {
                if ml_st().ml_get_recursive == 0 {
                    ml_st().ml_get_recursive += 1;
                    get_trans_bufname(buf);
                    let namebuff_ptr = g_st().name_buff.as_mut_ptr();
                    shorten_dir(namebuff_ptr);
                    siemsg!(
                        tr(E_ML_GET_CANNOT_FIND_LINE_NR_IN_BUFFER_NR_STR),
                        i64::from(lnum),
                        buf.b_fnum,
                        namebuff_ptr,
                    );
                    ml_st().ml_get_recursive -= 1;
                }
                return ml_get_errorret(buf, lnum);
            }

            let dp = (*hp).data_ptr().cast::<DataBlock>();
            let idx = (lnum - buf.b_ml.ml_locked_low) as usize;
            let start = *db_idx_ptr(&mut *dp, idx) & DB_INDEX_MASK;
            let end = if idx == 0 {
                (*dp).db_txt_end
            } else {
                *db_idx_ptr(&mut *dp, idx - 1) & DB_INDEX_MASK
            };

            buf.b_ml.ml_line_ptr = dp.cast::<c_char>().add(start as usize);
            buf.b_ml.ml_line_textlen = (end - start) as Colnr;
            buf.b_ml.ml_line_lnum = lnum;
            buf.b_ml.ml_flags &= !ML_LINE_DIRTY;
        }
        if will_change {
            buf.b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
            ml_add_deleted_len_buf( buf, buf.b_ml.ml_line_ptr, -1);
        }

        buf.b_ml.ml_line_ptr
    }
}

fn ml_get_errorret(buf: &mut FileBuf, lnum: Linenr) -> *mut c_char {
    unsafe {
        let q = (&raw mut ml_st().ml_get_questions).cast::<c_char>();
        *q.add(0) = b'?' as c_char;
        *q.add(1) = b'?' as c_char;
        *q.add(2) = b'?' as c_char;
        *q.add(3) = NUL;
        buf.b_ml.ml_line_textlen = 4;
        buf.b_ml.ml_line_lnum = lnum;
        q
    }
}

/// Check if a line that was just obtained by `ml_get` is in allocated memory.
pub fn ml_line_alloced() -> c_int {
     curbuf_mut().b_ml.ml_flags & ML_LINE_DIRTY
}

fn ml_append_int(
    buf: &mut FileBuf,
    lnum: Linenr,
    line_arg: impl crate::strings::IntoCCharPtr,
    len_arg: Colnr,
    flags: c_int,
) -> bool { let line_arg = line_arg.into_c_char_ptr();
    unsafe {
        let line = line_arg;
        let mut len = len_arg;

        if lnum > buf.b_ml.ml_line_count || buf.b_ml.ml_mfp.is_none() {
            return false;
        }

        if ml_st().lowest_marked != 0 && ml_st().lowest_marked > lnum {
            ml_st().lowest_marked = lnum + 1;
        }

        if len == 0 {
            len = crate::memory::cstr_len(line) as Colnr + 1;
        }
        let mut space_needed = i64::from(len) + INDEX_SIZE as i64;

        let mfp = buf.b_ml.mfp_ptr();
        let page_size = i64::from((*mfp).mf_page_size);

        let mut hp = ml_find_line(buf, if lnum == 0 { 1 } else { lnum }, ML_INSERT);
        if hp.is_null() {
            return false;
        }

        buf.b_ml.ml_flags &= !ML_EMPTY;

        let mut db_idx: c_int = if lnum == 0 {
            -1
        } else {
            lnum - buf.b_ml.ml_locked_low
        };
        let mut line_count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low;

        let mut dp = (*hp).data_ptr().cast::<DataBlock>();

        if i64::from((*dp).db_free) < space_needed
            && db_idx == line_count - 1
            && lnum < buf.b_ml.ml_line_count
        {
            buf.b_ml.ml_locked_lineadd -= 1;
            buf.b_ml.ml_locked_high -= 1;
            hp = ml_find_line(buf, lnum + 1, ML_INSERT);
            if hp.is_null() {
                return false;
            }

            db_idx = -1;
            line_count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low;
            dp = (*hp).data_ptr().cast::<DataBlock>();
        }

        if buf.b_prev_line_count == 0 {
            buf.b_prev_line_count = buf.b_ml.ml_line_count;
        }
        buf.b_ml.ml_line_count += 1;

        if i64::from((*dp).db_free) >= space_needed {
            (*dp).db_txt_start -= len as c_uint;
            (*dp).db_free -= space_needed as c_uint;
            (*dp).db_line_count += 1;

            if line_count > db_idx + 1 {
                let offset = if db_idx < 0 {
                    (*dp).db_txt_end as c_int
                } else {
                    (*db_idx_ptr(&mut *dp, db_idx as usize) & DB_INDEX_MASK) as c_int
                };
                ptr::copy(
                    dp.cast::<c_char>()
                        .add((*dp).db_txt_start as usize + len as usize),
                    dp.cast::<c_char>().add((*dp).db_txt_start as usize),
                    offset as usize - ((*dp).db_txt_start as usize + len as usize),
                );
                let mut i = line_count - 1;
                while i > db_idx {
                    *db_idx_ptr(&mut *dp, (i + 1) as usize) = *db_idx_ptr(&mut *dp, i as usize) - len as c_uint;
                    i -= 1;
                }
                *db_idx_ptr(&mut *dp, (db_idx + 1) as usize) = (offset - len) as c_uint;
            } else {
                *db_idx_ptr(&mut *dp, (db_idx + 1) as usize) = (*dp).db_txt_start;
            }

            let dst_off = *db_idx_ptr(&mut *dp, (db_idx + 1) as usize) as usize;
            ptr::copy(line, dp.cast::<c_char>().add(dst_off), len as usize);
            if (flags & ML_APPEND_MARK) != 0 {
                *db_idx_ptr(&mut *dp, (db_idx + 1) as usize) |= DB_MARKED;
            }

            buf.b_ml.ml_flags |= ML_LOCKED_DIRTY;
            if (flags & ML_APPEND_NEW) == 0 {
                buf.b_ml.ml_flags |= ML_LOCKED_POS;
            }
        } else {
            let mut data_moved: c_int = 0;
            let mut total_moved: c_int = 0;
            let lines_moved: c_int;
            let in_left: bool;

            if db_idx < 0 {
                lines_moved = 0;
                in_left = true;
            } else {
                lines_moved = line_count - db_idx - 1;
                if lines_moved == 0 {
                    in_left = false;
                } else {
                    data_moved = ((*db_idx_ptr(&mut *dp, db_idx as usize) & DB_INDEX_MASK)
                        - (*dp).db_txt_start) as c_int;
                    total_moved = data_moved + lines_moved * (INDEX_SIZE as c_int);
                    if i64::from((*dp).db_free) + i64::from(total_moved) >= space_needed {
                        in_left = true;
                        space_needed = i64::from(total_moved);
                    } else {
                        in_left = false;
                        space_needed += i64::from(total_moved);
                    }
                }
            }

            let new_page_count = ((space_needed + HEADER_SIZE as i64) + page_size - 1) / page_size;
            let mut hp_new = ml_new_data(&mut *mfp, (flags & ML_APPEND_NEW) != 0, new_page_count);
            let hp_left: *mut Bhdr;
            let hp_right: *mut Bhdr;
            let mut line_count_left: c_int;
            let mut line_count_right: c_int;
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
            let dp_right = (*hp_right).data_ptr().cast::<DataBlock>();
            let dp_left = (*hp_left).data_ptr().cast::<DataBlock>();
            let mut bnum_left = (*hp_left).bh_bnum;
            let mut bnum_right = (*hp_right).bh_bnum;
            let mut page_count_left = (*hp_left).bh_page_count as c_int;
            let mut page_count_right = (*hp_right).bh_page_count as c_int;

            if !in_left {
                (*dp_right).db_txt_start -= len as c_uint;
                (*dp_right).db_free -= len as c_uint + INDEX_SIZE as c_uint;
                *db_idx_ptr(&mut *dp_right, 0) = (*dp_right).db_txt_start;
                if (flags & ML_APPEND_MARK) != 0 {
                    *db_idx_ptr(&mut *dp_right, 0) |= DB_MARKED;
                }
                ptr::copy(
                    line,
                    dp_right
                        .cast::<c_char>()
                        .add((*dp_right).db_txt_start as usize),
                    len as usize,
                );
                line_count_right += 1;
            }

            if lines_moved != 0 {
                (*dp_right).db_txt_start -= data_moved as c_uint;
                (*dp_right).db_free -= total_moved as c_uint;
                ptr::copy(
                    dp_left
                        .cast::<c_char>()
                        .add((*dp_left).db_txt_start as usize),
                    dp_right
                        .cast::<c_char>()
                        .add((*dp_right).db_txt_start as usize),
                    data_moved as usize,
                );
                let offset = ((*dp_right).db_txt_start as c_int)
                    .wrapping_sub((*dp_left).db_txt_start as c_int);
                (*dp_left).db_txt_start += data_moved as c_uint;
                (*dp_left).db_free += total_moved as c_uint;

                let mut to = line_count_right;
                let mut from = db_idx + 1;
                while from < line_count_left {
                    *db_idx_ptr(&mut *dp_right, to as usize) =
                        (*db_idx_ptr(&mut *dp, from as usize)).wrapping_add(offset as c_uint);
                    from += 1;
                    to += 1;
                }
                line_count_right += lines_moved;
                line_count_left -= lines_moved;
            }

            if in_left {
                (*dp_left).db_txt_start -= len as c_uint;
                (*dp_left).db_free -= len as c_uint + INDEX_SIZE as c_uint;
                *db_idx_ptr(&mut *dp_left, line_count_left as usize) = (*dp_left).db_txt_start;
                if (flags & ML_APPEND_MARK) != 0 {
                    *db_idx_ptr(&mut *dp_left, line_count_left as usize) |= DB_MARKED;
                }
                ptr::copy(
                    line,
                    dp_left
                        .cast::<c_char>()
                        .add((*dp_left).db_txt_start as usize),
                    len as usize,
                );
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
            (*dp_left).db_line_count = line_count_left as c_long;
            (*dp_right).db_line_count = line_count_right as c_long;

            if lines_moved != 0 || in_left {
                buf.b_ml.ml_flags |= ML_LOCKED_DIRTY;
            }
            if (flags & ML_APPEND_NEW) == 0 && db_idx >= 0 && in_left {
                buf.b_ml.ml_flags |= ML_LOCKED_POS;
            }
            mf_put( &mut *mfp, hp_new, true, false);

            let lineadd = buf.b_ml.ml_locked_lineadd;
            buf.b_ml.ml_locked_lineadd = 0;
            ml_find_line(buf, 0, ML_FLUSH);

            let mut stack_idx = buf.b_ml.ml_stack_top - 1;
            while stack_idx >= 0 {
                let ip = buf.b_ml.ml_stack.as_mut_ptr().add(stack_idx as usize);
                let pb_idx = (*ip).ip_index as usize;
                hp = mf_get(&mut *mfp, (*ip).ip_bnum, 1);
                if hp.is_null() {
                    return false;
                }
                let mut pp = (*hp).data_ptr().cast::<PointerBlock>();
                if (*pp).pb_id != PTR_ID {
                    iemsg(tr(E_POINTER_BLOCK_ID_WRONG_THREE));
                    mf_put( &mut *mfp, hp, false, false);
                    return false;
                }
                if (*pp).pb_count < (*pp).pb_count_max {
                    if pb_idx + 1 < (*pp).pb_count as usize {
                        ptr::copy(
                            pb_ptr(&mut *pp, pb_idx + 1),
                            pb_ptr(&mut *pp, pb_idx + 2),
                            (*pp).pb_count as usize - pb_idx - 1,
                        );
                    }
                    (*pp).pb_count += 1;
                    let pe_cur = pb_ptr(&mut *pp, pb_idx);
                    pe_cur.pe_line_count = line_count_left;
                    pe_cur.pe_bnum = bnum_left;
                    pe_cur.pe_page_count = page_count_left;
                    let pe_next = pb_ptr(&mut *pp, pb_idx + 1);
                    pe_next.pe_line_count = line_count_right;
                    pe_next.pe_bnum = bnum_right;
                    pe_next.pe_page_count = page_count_right;

                    if lnum_left != 0 {
                        pe_cur.pe_old_lnum = lnum_left;
                    }
                    if lnum_right != 0 {
                        pe_next.pe_old_lnum = lnum_right;
                    }

                    mf_put( &mut *mfp, hp, true, false);
                    buf.b_ml.ml_stack_top = stack_idx + 1;

                    if lineadd != 0 {
                        buf.b_ml.ml_stack_top -= 1;
                        ml_lineadd(buf, lineadd);
                        let top_ip = buf
                            .b_ml
                            .ml_stack
                            .as_mut_ptr()
                            .add(buf.b_ml.ml_stack_top as usize);
                        (*top_ip).ip_high += lineadd;
                        buf.b_ml.ml_stack_top += 1;
                    }

                    break;
                }

                let mut pp_new: *mut PointerBlock;
                loop {
                    hp_new = ml_new_ptr(&mut *mfp);
                    if hp_new.is_null() {
                        return false;
                    }
                    pp_new = (*hp_new).data_ptr().cast::<PointerBlock>();

                    if (*hp).bh_bnum != 1 {
                        break;
                    }

                    ptr::copy(pp.cast::<u8>(), pp_new.cast::<u8>(), page_size as usize);
                    (*pp).pb_count = 1;
                    let pe0 = pb_ptr(&mut *pp, 0);
                    pe0.pe_bnum = (*hp_new).bh_bnum;
                    pe0.pe_line_count = buf.b_ml.ml_line_count;
                    pe0.pe_old_lnum = 1;
                    pe0.pe_page_count = 1;
                    mf_put( &mut *mfp, hp, true, false);
                    hp = hp_new;
                    pp = pp_new;
                    (*ip).ip_index = 0;
                    stack_idx += 1;
                }

                total_moved = c_int::from((*pp).pb_count) - (pb_idx as c_int) - 1;
                if total_moved != 0 {
                    ptr::copy(
                        pb_ptr(&mut *pp, pb_idx + 1),
                        pb_ptr(&mut *pp_new, 0),
                        total_moved as usize,
                    );
                    (*pp_new).pb_count = total_moved as u16;
                    (*pp).pb_count = (c_int::from((*pp).pb_count) - (total_moved - 1)) as u16;
                    let pe_next = pb_ptr(&mut *pp, pb_idx + 1);
                    pe_next.pe_bnum = bnum_right;
                    pe_next.pe_line_count = line_count_right;
                    pe_next.pe_page_count = page_count_right;
                    if lnum_right != 0 {
                        pe_next.pe_old_lnum = lnum_right;
                    }
                } else {
                    (*pp_new).pb_count = 1;
                    let pe_new0 = pb_ptr(&mut *pp_new, 0);
                    pe_new0.pe_bnum = bnum_right;
                    pe_new0.pe_line_count = line_count_right;
                    pe_new0.pe_page_count = page_count_right;
                    pe_new0.pe_old_lnum = lnum_right;
                }
                let pe_cur = pb_ptr(&mut *pp, pb_idx);
                pe_cur.pe_bnum = bnum_left;
                pe_cur.pe_line_count = line_count_left;
                pe_cur.pe_page_count = page_count_left;
                if lnum_left != 0 {
                    pe_cur.pe_old_lnum = lnum_left;
                }
                lnum_left = 0;
                lnum_right = 0;

                line_count_right = 0;
                for i in 0..((*pp_new).pb_count as usize) {
                    line_count_right += pb_ptr(&mut *pp_new, i).pe_line_count;
                }
                line_count_left = 0;
                for i in 0..((*pp).pb_count as usize) {
                    line_count_left += pb_ptr(&mut *pp, i).pe_line_count;
                }

                bnum_left = (*hp).bh_bnum;
                bnum_right = (*hp_new).bh_bnum;
                page_count_left = 1;
                page_count_right = 1;
                mf_put( &mut *mfp, hp, true, false);
                mf_put( &mut *mfp, hp_new, true, false);

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
}

fn ml_append_flush(
    buf: &mut FileBuf,
    lnum: Linenr,
    line: impl crate::strings::IntoCCharPtr,
    len: Colnr,
    flags: c_int,
) -> bool { let line = line.into_c_char_ptr();
     {
        if lnum > buf.b_ml.ml_line_count {
            return false;
        }
        if buf.b_ml.ml_line_lnum != 0 {
            ml_flush_line(buf, false);
        }
        ml_append_int(buf, lnum, line, len, flags)
    }
}

/// Append a line after `lnum` (may be 0 to insert in front of the file).
pub fn ml_append(lnum: Linenr, line: impl crate::strings::IntoCCharPtr, len: Colnr, newfile: bool) -> bool {
    let line = line.into_c_char_ptr().cast_mut();
    ml_append_flags(lnum, line, len, if newfile { ML_APPEND_NEW } else { 0 })
}

/// Append a line after `lnum` using `ML_APPEND_` flags.
pub fn ml_append_flags(
    lnum: Linenr,
    line: impl crate::strings::IntoCCharPtr,
    len: Colnr,
    flags: c_int,
) -> bool { let line = line.into_c_char_ptr();
     {
        if curbuf_mut().b_ml.ml_mfp.is_none() && open_buffer(false, None, 0) == FAIL {
            return false;
        }
        ml_append_flush(curbuf_mut(), lnum, line, len, flags)
    }
}

/// Like `ml_append()` but for an arbitrary buffer.
pub fn ml_append_buf(
    buf: &mut FileBuf,
    lnum: Linenr,
    line: impl crate::strings::IntoCCharPtr,
    len: Colnr,
    newfile: bool,
) -> bool { let line = line.into_c_char_ptr();
     {
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
}

pub fn ml_add_deleted_len(ptr: impl crate::strings::IntoCCharPtr, len: isize) { let ptr = ptr.into_c_char_ptr();
    ml_add_deleted_len_buf(curbuf_mut(), ptr, len);
}

pub fn ml_add_deleted_len_buf(buf: &mut FileBuf, ptr: impl crate::strings::IntoCCharPtr, mut len: isize) { let ptr = ptr.into_c_char_ptr();
     {
        if g_st().inhibit_delete_count != 0 {
            return;
        }
        let maxlen = crate::memory::cstr_len(ptr) as isize;
        if len == -1 || len > maxlen {
            len = maxlen;
        }
        buf.deleted_bytes += (len as usize) + 1;
        buf.deleted_bytes2 += (len as usize) + 1;
        if buf.update_need_codepoints {
            mb_utflen(
                ptr,
                len as usize,
                &mut buf.deleted_codepoints,
                &mut buf.deleted_codeunits,
            );
            buf.deleted_codepoints += 1;
            buf.deleted_codeunits += 1;
        }
    }
}

/// Replace line `lnum`, with buffering, in current buffer.
pub fn ml_replace(lnum: Linenr, line: impl crate::strings::IntoCCharPtr, copy: bool) -> bool { let line = line.into_c_char_ptr();
    ml_replace_buf(curbuf_mut(), lnum, line, copy, false)
}

/// Replace a line for the current buffer with explicit `len` (excluding NUL).
pub fn ml_replace_len(lnum: Linenr, line: *mut c_char, len: usize, copy: bool) -> bool {
    ml_replace_buf_len(curbuf_mut(), lnum, line, len, copy, false)
}

pub fn ml_replace_buf(
    buf: &mut FileBuf,
    lnum: Linenr,
    line: impl crate::strings::IntoCCharPtr,
    copy: bool,
    noalloc: bool,
) -> bool {
    let line = line.into_c_char_ptr().cast_mut();
     {
        let len = if !line.is_null() {
            crate::memory::cstr_len(line)
        } else {
            usize::MAX
        };
        ml_replace_buf_len( buf, lnum, line, len, copy, noalloc)
    }
}

/// Replace line `lnum`, with buffering.
pub fn ml_replace_buf_len(
    buf: &mut FileBuf,
    lnum: Linenr,
    line_arg: *mut c_char,
    len_arg: usize,
    copy: bool,
    noalloc: bool,
) -> bool {
    unsafe {
        let mut line = line_arg;
        if line.is_null() {
            return false;
        }

        if buf.b_ml.ml_mfp.is_none() && open_buffer(false, None, 0) == FAIL {
            return false;
        }

        let new_buf = if copy {
            assert!(!noalloc);
            let mut v = Vec::with_capacity(len_arg + 1);
            if len_arg > 0 {
                v.extend_from_slice(std::slice::from_raw_parts(line, len_arg));
            }
            v.push(NUL);
            Some(v)
        } else {
            assert!(noalloc);
            None
        };

        if buf.b_ml.ml_line_lnum != lnum {
            ml_flush_line(buf, false);
        }

        if !buf.update_callbacks.is_empty() {
            let old_line = ml_get_buf(buf, lnum);
            ml_add_deleted_len_buf( buf, old_line, -1);
        }

        if let Some(mut v) = new_buf {
            line = v.as_mut_ptr();
            buf.b_ml.ml_line_buf = v;
        } else {
            buf.b_ml.ml_line_buf.clear();
        }

        buf.b_ml.ml_line_ptr = line;
        buf.b_ml.ml_line_textlen = (len_arg as Colnr) + 1;
        buf.b_ml.ml_line_lnum = lnum;
        buf.b_ml.ml_flags = (buf.b_ml.ml_flags | ML_LINE_DIRTY) & !ML_EMPTY;
        if noalloc {
            ml_flush_line(buf, true);
        }

        true
    }
}

/// Replace the in-memory dirty line buffer in `buf` with `bytes` (without trailing NUL).
pub fn ml_replace_dirty_line(buf: &mut FileBuf, bytes: &[c_char], clear_empty: bool) {
    let mut new_buf = Vec::with_capacity(bytes.len() + 1);
        new_buf.extend_from_slice(bytes);
        new_buf.push(NUL);
        buf.b_ml.ml_line_ptr = new_buf.as_mut_ptr();
        buf.b_ml.ml_line_buf = new_buf;
        buf.b_ml.ml_line_textlen = (bytes.len() as Colnr) + 1;
        buf.b_ml.ml_flags |= ML_LINE_DIRTY;
        if clear_empty {
            buf.b_ml.ml_flags &= !ML_EMPTY;
        }
}

/// Delete line `lnum` in `buf`.
pub fn ml_delete_buf(buf: &mut FileBuf, lnum: Linenr, message: bool) -> bool {
    ml_flush_line( buf, false);
        ml_delete_int( buf, lnum, if message { ML_DEL_MESSAGE } else { 0 })
}

fn ml_delete_int(buf: &mut FileBuf, lnum: Linenr, flags: c_int) -> bool {
    unsafe {
        if ml_st().lowest_marked != 0 && ml_st().lowest_marked > lnum {
            ml_st().lowest_marked -= 1;
        }

        if buf.b_ml.ml_line_count == 1 {
            if (flags & ML_DEL_MESSAGE) != 0 {
                set_keep_msg(gettext(NO_LINES_MSG), 0);
            }
            let i = ml_replace_buf(buf, 1, c"", true, false);
            buf.b_ml.ml_flags |= ML_EMPTY;
            return i;
        }

        let mfp = buf.b_ml.mfp_ptr();
        if mfp.is_null() {
            return false;
        }

        let mut hp = ml_find_line(buf, lnum, ML_DELETE);
        if hp.is_null() {
            return false;
        }

        let dp = (*hp).data_ptr().cast::<DataBlock>();
        let mut count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low + 2;
        let mut idx = lnum - buf.b_ml.ml_locked_low;

        if buf.b_prev_line_count == 0 {
            buf.b_prev_line_count = buf.b_ml.ml_line_count;
        }
        buf.b_ml.ml_line_count -= 1;

        let line_start = (*db_idx_ptr(&mut *dp, idx as usize) & DB_INDEX_MASK) as c_int;
        let line_size: c_int = if idx == 0 {
            ((*dp).db_txt_end as c_int) - line_start
        } else {
            ((*db_idx_ptr(&mut *dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int) - line_start
        };

        assert!(line_size >= 1);
        ml_add_deleted_len_buf(
             buf,
            dp.cast::<c_char>().add(line_start as usize),
            (line_size - 1) as isize,
        );

        if count == 1 {
            mf_free(&mut *mfp, &mut *hp);
            buf.b_ml.ml_locked = ptr::null_mut();

            let mut stack_idx = buf.b_ml.ml_stack_top - 1;
            while stack_idx >= 0 {
                buf.b_ml.ml_stack_top = 0;
                let ip = buf.b_ml.ml_stack.as_mut_ptr().add(stack_idx as usize);
                idx = (*ip).ip_index;
                hp = mf_get(&mut *mfp, (*ip).ip_bnum, 1);
                if hp.is_null() {
                    return false;
                }
                let pp = (*hp).data_ptr().cast::<PointerBlock>();
                if (*pp).pb_id != PTR_ID {
                    iemsg(tr(E_POINTER_BLOCK_ID_WRONG_FOUR));
                    mf_put( &mut *mfp, hp, false, false);
                    return false;
                }
                (*pp).pb_count -= 1;
                count = c_int::from((*pp).pb_count);
                if count == 0 {
                    mf_free(&mut *mfp, &mut *hp);
                } else {
                    if count != idx {
                        ptr::copy(
                            pb_ptr(&mut *pp, (idx + 1) as usize),
                            pb_ptr(&mut *pp, idx as usize),
                            (count - idx) as usize,
                        );
                    }
                    mf_put( &mut *mfp, hp, true, false);

                    buf.b_ml.ml_stack_top = stack_idx;
                    if buf.b_ml.ml_locked_lineadd != 0 {
                        ml_lineadd(buf, buf.b_ml.ml_locked_lineadd);
                        let top_ip = buf
                            .b_ml
                            .ml_stack
                            .as_mut_ptr()
                            .add(buf.b_ml.ml_stack_top as usize);
                        (*top_ip).ip_high += buf.b_ml.ml_locked_lineadd;
                    }
                    buf.b_ml.ml_stack_top += 1;
                    break;
                }
                stack_idx -= 1;
            }
        } else {
            let text_start = (*dp).db_txt_start as c_int;
            ptr::copy(
                dp.cast::<c_char>().add(text_start as usize),
                dp.cast::<c_char>().add((text_start + line_size) as usize),
                (line_start - text_start) as usize,
            );

            for i in idx..(count - 1) {
                *db_idx_ptr(&mut *dp, i as usize) =
                    *db_idx_ptr(&mut *dp, (i + 1) as usize) + (line_size as c_uint);
            }

            (*dp).db_free += (line_size as c_uint) + (INDEX_SIZE as c_uint);
            (*dp).db_txt_start += line_size as c_uint;
            (*dp).db_line_count -= 1;

            buf.b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
        }

        ml_updatechunk(buf, lnum, line_size, ML_CHNK_DELLINE);
        true
    }
}

/// Delete line `lnum` in the current buffer.
pub fn ml_delete(lnum: Linenr) -> bool {
    ml_delete_flags(lnum, 0)
}

/// Like `ml_delete()` but using flags.
pub fn ml_delete_flags(lnum: Linenr, flags: c_int) -> bool {
     {
        ml_flush_line(curbuf_mut(), false);
        if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count {
            return false;
        }
        ml_delete_int(curbuf_mut(), lnum, flags)
    }
}

/// Set the `DB_MARKED` flag for line `lnum`.
pub fn ml_setmarked(lnum: Linenr) {
    unsafe {
        if lnum < 1 || lnum > curbuf_mut().b_ml.ml_line_count || curbuf_mut().b_ml.ml_mfp.is_none() {
            return;
        }
        if ml_st().lowest_marked == 0 || ml_st().lowest_marked > lnum {
            ml_st().lowest_marked = lnum;
        }

        let hp = ml_find_line(curbuf_mut(), lnum, ML_FIND);
        if hp.is_null() {
            return;
        }
        let dp = (*hp).data_ptr().cast::<DataBlock>();
        *db_idx_ptr(&mut *dp, (lnum - curbuf_mut().b_ml.ml_locked_low) as usize) |= DB_MARKED;
        curbuf_mut().b_ml.ml_flags |= ML_LOCKED_DIRTY;
    }
}

/// Find the first line with its `DB_MARKED` flag set.
pub fn ml_firstmarked() -> Linenr {
    unsafe {
        if curbuf_mut().b_ml.ml_mfp.is_none() {
            return 0;
        }

        let mut lnum = ml_st().lowest_marked;
        while lnum <= curbuf_mut().b_ml.ml_line_count {
            let hp = ml_find_line(curbuf_mut(), lnum, ML_FIND);
            if hp.is_null() {
                return 0;
            }
            let dp = (*hp).data_ptr().cast::<DataBlock>();
            let mut i = (lnum - curbuf_mut().b_ml.ml_locked_low) as isize;
            while lnum <= curbuf_mut().b_ml.ml_locked_high {
                let idx_p = (&raw mut (*dp).db_index).cast::<c_uint>().offset(i);
                if (*idx_p & DB_MARKED) != 0 {
                    *idx_p &= DB_INDEX_MASK;
                    curbuf_mut().b_ml.ml_flags |= ML_LOCKED_DIRTY;
                    ml_st().lowest_marked = lnum + 1;
                    return lnum;
                }
                i += 1;
                lnum += 1;
            }
        }

        0
    }
}

/// Clear all `DB_MARKED` flags.
pub fn ml_clearmarked() {
    unsafe {
        if curbuf_mut().b_ml.ml_mfp.is_none() {
            return;
        }

        let mut lnum = ml_st().lowest_marked;
        while lnum <= curbuf_mut().b_ml.ml_line_count {
            let hp = ml_find_line(curbuf_mut(), lnum, ML_FIND);
            if hp.is_null() {
                return;
            }
            let dp = (*hp).data_ptr().cast::<DataBlock>();
            let mut i = (lnum - curbuf_mut().b_ml.ml_locked_low) as isize;
            while lnum <= curbuf_mut().b_ml.ml_locked_high {
                let idx_p = (&raw mut (*dp).db_index).cast::<c_uint>().offset(i);
                if (*idx_p & DB_MARKED) != 0 {
                    *idx_p &= DB_INDEX_MASK;
                    curbuf_mut().b_ml.ml_flags |= ML_LOCKED_DIRTY;
                }
                i += 1;
                lnum += 1;
            }
        }

        ml_st().lowest_marked = 0;
    }
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

fn ml_flush_line(buf: &mut FileBuf, noalloc: bool) {
    unsafe {
        if buf.b_ml.ml_line_lnum == 0 || buf.b_ml.ml_mfp.is_none() {
            return;
        }
        if (buf.b_ml.ml_flags & ML_LINE_DIRTY) != 0 {
            if ml_st().ml_flush_entered {
                return;
            }
            ml_st().ml_flush_entered = true;

            buf.flush_count += 1;

            let lnum = buf.b_ml.ml_line_lnum;
            let owned_line = if noalloc {
                Vec::new()
            } else {
                std::mem::take(&mut buf.b_ml.ml_line_buf)
            };
            let new_line = if noalloc {
                buf.b_ml.ml_line_ptr
            } else {
                owned_line.as_ptr().cast_mut()
            };

            let hp = ml_find_line(buf, lnum, ML_FIND);
            if hp.is_null() {
                siemsg!(tr(c"E320: Cannot find line %lld"), i64::from(lnum));
            } else {
                let dp = (*hp).data_ptr().cast::<DataBlock>();
                let idx = (lnum - buf.b_ml.ml_locked_low) as usize;
                let start = (*db_idx_ptr(&mut *dp, idx) & DB_INDEX_MASK) as c_int;
                let old_line = dp.cast::<c_char>().add(start as usize);
                let old_len: c_int = if idx == 0 {
                    ((*dp).db_txt_end as c_int) - start
                } else {
                    ((*db_idx_ptr(&mut *dp, idx - 1) & DB_INDEX_MASK) as c_int) - start
                };
                let new_len = buf.b_ml.ml_line_textlen;
                let extra = new_len - old_len;

                if ((*dp).db_free as c_int) >= extra {
                    let count =
                        (buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low + 1) as usize;
                    if extra != 0 && idx < count - 1 {
                        let txt_start = (*dp).db_txt_start as isize;
                        ptr::copy(
                            dp.cast::<c_char>().offset(txt_start),
                            dp.cast::<c_char>().offset(txt_start - extra as isize),
                            (start - (*dp).db_txt_start as c_int) as usize,
                        );
                        for i in (idx + 1)..count {
                            *db_idx_ptr(&mut *dp, i) = (*db_idx_ptr(&mut *dp, i)).wrapping_sub(extra as c_uint);
                        }
                    }
                    *db_idx_ptr(&mut *dp, idx) = (*db_idx_ptr(&mut *dp, idx)).wrapping_sub(extra as c_uint);

                    (*dp).db_free = (*dp).db_free.wrapping_sub(extra as c_uint);
                    (*dp).db_txt_start = (*dp).db_txt_start.wrapping_sub(extra as c_uint);

                    ptr::copy(
                        new_line,
                        old_line.offset(-(extra as isize)),
                        new_len as usize,
                    );
                    buf.b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
                    if extra != 0 {
                        ml_updatechunk(buf, lnum, extra, ML_CHNK_UPDLINE);
                    }
                } else {
                    let mark_flag = if (*db_idx_ptr(&mut *dp, idx) & DB_MARKED) != 0 {
                        ML_APPEND_MARK
                    } else {
                        0
                    };
                    let _ = ml_append_int(buf, lnum, new_line, new_len, mark_flag);
                    let _ = ml_delete_int(buf, lnum, 0);
                }
            }
            drop(owned_line);

            ml_st().ml_flush_entered = false;
        }

        buf.b_ml.ml_flags &= !ML_LINE_DIRTY;
        buf.b_ml.ml_line_lnum = 0;
        buf.b_ml.ml_line_offset = 0;
    }
}

fn ml_new_data(mfp: &mut Memfile, negative: bool, page_count: i64) -> *mut Bhdr {
    unsafe {
        assert!(page_count >= 0);
        let hp = mf_new(mfp, negative, page_count as c_uint);
        let dp = (*hp).data_ptr().cast::<DataBlock>();
        (*dp).db_id = DATA_ID;
        (*dp).db_txt_end = (page_count as c_uint).wrapping_mul(mfp.mf_page_size);
        (*dp).db_txt_start = (*dp).db_txt_end;
        (*dp).db_free = (*dp).db_txt_start - (HEADER_SIZE as c_uint);
        (*dp).db_line_count = 0;
        hp
    }
}

fn ml_new_ptr(mfp: &mut Memfile) -> *mut Bhdr {
    unsafe {
        let hp = mf_new(mfp, false, 1);
        let pp = (*hp).data_ptr().cast::<PointerBlock>();
        (*pp).pb_id = PTR_ID;
        (*pp).pb_count = 0;
        (*pp).pb_count_max = pb_count_max(mfp);
        hp
    }
}

fn ml_find_line(buf: &mut FileBuf, lnum: Linenr, action: c_int) -> *mut Bhdr {
    unsafe {
        let mfp = buf.b_ml.mfp_ptr();

        if !buf.b_ml.ml_locked.is_null() {
            if ml_simple(action)
                && buf.b_ml.ml_locked_low <= lnum
                && buf.b_ml.ml_locked_high >= lnum
            {
                if action == ML_INSERT {
                    buf.b_ml.ml_locked_lineadd += 1;
                    buf.b_ml.ml_locked_high += 1;
                } else if action == ML_DELETE {
                    buf.b_ml.ml_locked_lineadd -= 1;
                    buf.b_ml.ml_locked_high -= 1;
                }
                return buf.b_ml.ml_locked.cast();
            }

            mf_put(
                 &mut *mfp,
                buf.b_ml.ml_locked.cast(),
                (buf.b_ml.ml_flags & ML_LOCKED_DIRTY) != 0,
                (buf.b_ml.ml_flags & ML_LOCKED_POS) != 0,
            );
            buf.b_ml.ml_locked = ptr::null_mut();

            if buf.b_ml.ml_locked_lineadd != 0 {
                ml_lineadd(buf, buf.b_ml.ml_locked_lineadd);
            }
        }

        if action == ML_FLUSH {
            return ptr::null_mut();
        }

        let mut bnum: Blocknr = 1;
        let mut page_count: c_int = 1;
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
            let hp = mf_get(&mut *mfp, bnum, page_count as c_uint);
            if hp.is_null() {
                return ml_find_line_error_noblock(buf, action);
            }

            if action == ML_INSERT {
                high += 1;
            } else if action == ML_DELETE {
                high -= 1;
            }

            let dp = (*hp).data_ptr().cast::<DataBlock>();
            if (*dp).db_id == DATA_ID {
                buf.b_ml.ml_locked = hp.cast();
                buf.b_ml.ml_locked_low = low;
                buf.b_ml.ml_locked_high = high;
                buf.b_ml.ml_locked_lineadd = 0;
                buf.b_ml.ml_flags &= !(ML_LOCKED_DIRTY | ML_LOCKED_POS);
                return hp;
            }

            let pp = dp.cast::<PointerBlock>();
            if (*pp).pb_id != PTR_ID {
                iemsg(tr(E_POINTER_BLOCK_ID_WRONG));
                mf_put( &mut *mfp, hp, false, false);
                return ml_find_line_error_noblock(buf, action);
            }

            let top = ml_add_stack(buf);
            let ip = &mut (&mut buf.b_ml.ml_stack)[top as usize];
            ip.ip_bnum = bnum;
            ip.ip_low = low;
            ip.ip_high = high;
            ip.ip_index = -1;

            let mut dirty = false;
            let mut idx: c_int = 0;
            while idx < c_int::from((*pp).pb_count) {
                let pe = pb_ptr(&mut *pp, idx as usize);
                let t = pe.pe_line_count;
                low += t;
                if low > lnum {
                    ip.ip_index = idx;
                    bnum = pe.pe_bnum;
                    page_count = pe.pe_page_count;
                    high = low - 1;
                    low -= t;

                    if bnum < 0 {
                        let bnum2 = mf_trans_del(&mut *mfp, bnum);
                        if bnum != bnum2 {
                            bnum = bnum2;
                            pe.pe_bnum = bnum;
                            dirty = true;
                        }
                    }
                    break;
                }
                idx += 1;
            }
            if idx >= c_int::from((*pp).pb_count) {
                if lnum > buf.b_ml.ml_line_count {
                    siemsg!(
                        tr(E_LINE_NUMBER_OUT_OF_RANGE_NR_PAST_THE_END),
                        i64::from(lnum - buf.b_ml.ml_line_count),
                    );
                } else {
                    siemsg!(tr(E_LINE_COUNT_WRONG_IN_BLOCK_NR), bnum);
                }
                mf_put( &mut *mfp, hp, false, false);
                return ml_find_line_error_noblock(buf, action);
            }
            if action == ML_DELETE {
                pb_ptr(&mut *pp, idx as usize).pe_line_count -= 1;
                dirty = true;
            } else if action == ML_INSERT {
                pb_ptr(&mut *pp, idx as usize).pe_line_count += 1;
                dirty = true;
            }
            mf_put( &mut *mfp, hp, dirty, false);
        }
    }
}

fn ml_find_line_error_noblock(buf: &mut FileBuf, action: c_int) -> *mut Bhdr {
    {
        if action == ML_DELETE {
            ml_lineadd(&mut *buf, 1);
        } else if action == ML_INSERT {
            ml_lineadd(&mut *buf, -1);
        }
        buf.b_ml.ml_stack_top = 0;
        ptr::null_mut()
    }
}

fn ml_add_stack(buf: &mut FileBuf) -> c_int {
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

fn ml_lineadd(buf: &mut FileBuf, count: c_int) {
    unsafe {
        let mfp = buf.b_ml.mfp_ptr();
        let mut idx = buf.b_ml.ml_stack_top - 1;
        while idx >= 0 {
            let ip = &mut (&mut buf.b_ml.ml_stack)[idx as usize];
            let hp = mf_get(&mut *mfp, ip.ip_bnum, 1);
            if hp.is_null() {
                break;
            }
            let pp = (*hp).data_ptr().cast::<PointerBlock>();
            if (*pp).pb_id != PTR_ID {
                mf_put( &mut *mfp, hp, false, false);
                iemsg(tr(E_POINTER_BLOCK_ID_WRONG_TWO));
                break;
            }
            pb_ptr(&mut *pp, ip.ip_index as usize).pe_line_count += count;
            ip.ip_high += count;
            mf_put( &mut *mfp, hp, true, false);
            idx -= 1;
        }
    }
}

/// Resolve a symlink in the last component of a file name.
#[cfg(unix)]
pub fn resolve_symlink(fname: impl crate::strings::IntoCCharPtr, buf: *mut c_char) -> bool { let fname = fname.into_c_char_ptr();
    unsafe {
        let mut tmp = [0 as c_char; MAXPATHL];
        let mut depth: c_int = 0;

        if fname.is_null() {
            return false;
        }

        let tmp_ptr = (&raw mut tmp).cast::<c_char>();
        xstrlcpy(tmp_ptr, fname, MAXPATHL);

        loop {
            depth += 1;
            if depth == 100 {
                semsg!(tr(c"E773: Symlink loop for \"%s\""), fname);
                return false;
            }

            let ret = readlink(tmp_ptr, buf, MAXPATHL - 1);
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
            *buf.add(ret as usize) = NUL;

            if path_is_absolute(buf) {
                xstrlcpy(tmp_ptr, buf, MAXPATHL);
            } else {
                let tail = path_tail(tmp_ptr);
                if crate::memory::cstr_len(tail) + crate::memory::cstr_len(buf) >= MAXPATHL {
                    return false;
                }
                let rem = MAXPATHL - tail.offset_from(tmp_ptr) as usize;
                xstrlcpy(tail, buf, rem);
            }
        }

        vim_full_name(tmp_ptr, buf, MAXPATHL, true)
    }
}

/// Make swapfile name out of the file name and a directory name.
pub fn makeswapname(
    fname: impl crate::strings::IntoCCharPtr,
    dir_name: *mut c_char,
) -> Option<std::ffi::CString> { let fname = fname.into_c_char_ptr();
    unsafe {
        #[cfg(unix)]
        let mut fname_res = fname;
        #[cfg(not(unix))]
        let fname_res = fname;
        #[cfg(unix)]
        let mut fname_buf = [0 as c_char; MAXPATHL];
        #[cfg(unix)]
        {
            let buf_ptr = (&raw mut fname_buf).cast::<c_char>();
            if resolve_symlink(fname, buf_ptr) {
                fname_res = buf_ptr;
            }
        }

        let len = crate::memory::cstr_len(dir_name);
        let s = dir_name.add(len);
        if after_pathsep(dir_name, s) != 0 && len > 1 && *s.offset(-1) == *s.offset(-2) {
            let sw = make_percent_swname(dir_name, s, fname_res)?;
            return modname(sw.as_ptr(), c".swp", false);
        }

        let r = modname(
            fname_res,
            c".swp",
            crate::memory::cstr_first_byte(dir_name) == b'.' && crate::memory::cstr_byte_at(dir_name, 1) == 0,
        )?;

        get_file_in_dir(r.as_ptr().cast_mut(), dir_name)
    }
}

/// Get file name to use for swapfile or backup file.
pub fn get_file_in_dir(fname: *mut c_char, dname: *mut c_char) -> Option<std::ffi::CString> {
    unsafe {
        let tail = cstr_as_string(path_tail(fname));

        if crate::memory::cstr_first_byte(dname) == b'.' && crate::memory::cstr_byte_at(dname, 1) == 0 {
            let n = tail.data.offset_from(fname) as usize + tail.size;
            return Some(
                std::ffi::CString::new(crate::memory::cstr_slice(fname, n))
                    .unwrap_or_default(),
            );
        }

        let dname_len = crate::memory::cstr_len(dname);
        let retval = if crate::memory::cstr_first_byte(dname) == b'.' && vim_ispathsep(crate::memory::cstr_byte_at(dname, 1) as c_int) {
            if tail.data == fname {
                concat_fnames(
                    NvimString {
                        data: dname.add(2),
                        size: dname_len - 2,
                    },
                    tail,
                    true,
                )
            } else {
                let save_char = *tail.data;
                *tail.data = NUL;
                let tmp = concat_fnames(
                    NvimString {
                        data: fname,
                        size: tail.data.offset_from(fname) as usize,
                    },
                    NvimString {
                        data: dname.add(2),
                        size: dname_len - 2,
                    },
                    true,
                );
                *tail.data = save_char;
                concat_fnames(cstr_as_string(tmp.as_ptr()), tail, true)
            }
        } else {
            concat_fnames(
                NvimString {
                    data: dname,
                    size: dname_len,
                },
                tail,
                true,
            )
        };

        Some(retval)
    }
}

fn attention_message(
    buf: &mut FileBuf,
    fname: impl crate::strings::IntoCCharPtr,
    fhname: impl crate::strings::IntoCCharPtr,
    msg: &mut Vec<u8>,
) { let fname = fname.into_c_char_ptr(); let fhname = fhname.into_c_char_ptr();
    assert!(!buf.b_fname.is_null());

        emsg(tr(c"E325: ATTENTION"));
        sb_push_cstr(msg, tr(c"Found a swap file by the name \""));
        sb_push_cstr(msg, fhname);
        msg.extend_from_slice(b"\"\n");
        let swap_mtime = swapfile_info(fname, msg);
        sb_push_cstr(msg, tr(c"While opening file \""));
        sb_push_cstr(msg, buf.b_fname);
        msg.extend_from_slice(b"\"\n");
        let mut file_info = FileInfo::new();
        if !os_fileinfo(buf.b_fname, &mut file_info) {
            sb_push_cstr(msg, tr(c"      CANNOT BE FOUND"));
        } else {
            sb_push_cstr(msg, tr(c"             dated: "));
            let x: time_t = file_info.stat.st_mtim.tv_sec as _;
            let mut ctime_buf = [0 as c_char; 50];
            let ct = os_ctime_r(
                &x,
                (&raw mut ctime_buf).cast(),
                size_of_val(&ctime_buf),
                true,
            );
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
        sb_push_cstr(msg, buf.b_fname);
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

fn do_swapexists(buf: &mut FileBuf, fname: impl crate::strings::IntoCCharPtr) -> SeaChoice { let fname = fname.into_c_char_ptr();
    unsafe {
        set_vim_var_string(VV_SWAPNAME, fname, -1);
        set_vim_var_string(VV_SWAPCHOICE, ptr::null(), -1);

        g_st().allbuf_lock += 1;
        apply_autocmds(
            EVENT_SWAPEXISTS,
            buf.b_fname,
            ptr::null_mut(),
            false,
            ptr::null_mut(),
        );
        g_st().allbuf_lock -= 1;

        set_vim_var_string(VV_SWAPNAME, ptr::null(), -1);

        match *get_vim_var_str(VV_SWAPCHOICE) as u8 {
            b'o' => SEA_CHOICE_READONLY,
            b'e' => SEA_CHOICE_EDIT,
            b'r' => SEA_CHOICE_RECOVER,
            b'd' => SEA_CHOICE_DELETE,
            b'q' => SEA_CHOICE_QUIT,
            b'a' => SEA_CHOICE_ABORT,
            _ => SEA_CHOICE_NONE,
        }
    }
}

fn findswapname(
    buf: &mut FileBuf,
    dirp: *mut *mut c_char,
    old_fname: impl crate::strings::IntoCCharPtr,
    found_existing_dir: &mut bool,
) -> Option<std::ffi::CString> { let old_fname = old_fname.into_c_char_ptr();
    unsafe {
        let buf_fname = buf.b_fname;
        let dir_len = crate::memory::cstr_len(*dirp) + 1;
        let mut dir_name_vec: Vec<c_char> = vec![0; dir_len];
        let dir_name = dir_name_vec.as_mut_ptr();
        copy_option_part(dirp, dir_name, dir_len, c",");

        let mut fname_vec: Option<Vec<c_char>> = makeswapname(buf_fname, dir_name)
            .map(|cstr| cstr.into_bytes_with_nul().into_iter().map(|b| b as c_char).collect());

        while let Some(ref mut fvec) = fname_vec {
            let fname = fvec.as_mut_ptr();
            let n = crate::memory::cstr_len(fname);
            if n == 0 {
                fname_vec = None;
                break;
            }
            let mut file_info = FileInfo::new();
            let file_or_link_found = os_fileinfo_link(fname, &mut file_info);
            if !file_or_link_found {
                break;
            }

            if !old_fname.is_null() && path_equal(fname, old_fname, K_PATH_CMP_LITERAL) {
                break;
            }

            if crate::memory::cstr_byte_at(fname, n - 2) == b'w'
                && crate::memory::cstr_byte_at(fname, n - 1) == b'p'
                && !g_st().recoverymode
                && !buf_fname.is_null()
                && !buf.b_help
                && (buf.b_flags & BF_DUMMY) == 0
            {
                let mut b0 = ZeroBlock::new();
                let mut differ = false;
                let namebuff_ptr = g_st().name_buff.as_mut_ptr();

                let fd = os_open(fname, O_RDONLY, 0);
                if fd >= 0 {
                    if read_eintr(fd, (&raw mut b0).cast(), size_of::<ZeroBlock>())
                        == size_of::<ZeroBlock>() as isize
                    {
                        #[cfg(windows)]
                        path_to_slash((&raw mut b0.b0_fname).cast());
                        ml_st().proc_running = swapfile_proc_running(&b0, fname);

                        if (b0.flags() & B0_SAME_DIR) != 0 {
                            if !path_equal(
                                path_tail(buf.b_ffname),
                                path_tail((&raw const b0.b0_fname).cast()),
                                K_PATH_CMP_LITERAL,
                            ) || !same_directory(fname, buf.b_ffname)
                            {
                                expand_env(
                                    (&raw mut b0.b0_fname).cast(),
                                    namebuff_ptr,
                                    MAXPATHL as c_int,
                                );
                                if fnamecmp_ino(
                                    buf.b_ffname,
                                    namebuff_ptr,
                                    char_to_long((&raw const b0.b0_ino).cast()),
                                ) {
                                    differ = true;
                                }
                            }
                        } else {
                            expand_env(
                                (&raw mut b0.b0_fname).cast(),
                                namebuff_ptr,
                                MAXPATHL as c_int,
                            );
                            if fnamecmp_ino(
                                buf.b_ffname,
                                namebuff_ptr,
                                char_to_long((&raw const b0.b0_ino).cast()),
                            ) {
                                differ = true;
                            }
                        }
                    }
                    close(fd);
                }

                if !differ
                    && (curbuf_mut().b_flags & BF_RECOVERED) == 0
                    && vim_strchr(opt_vars().p_shm, K_SHM_ATTENTION).is_null()
                {
                    let mut choice: SeaChoice = SEA_CHOICE_NONE;

                    if os_path_exists(buf.b_fname) && swapfile_unchanged(fname) {
                        choice = SEA_CHOICE_DELETE;
                        if opt_vars().p_verbose > 0 {
                            verb_msg(tr(c"Found a swap file that is not useful, deleting it"));
                        }
                    }

                    if choice == SEA_CHOICE_NONE
                        && g_st().swap_exists_action != SEA_NONE
                        && has_autocmd(EVENT_SWAPEXISTS, buf_fname, Some(&mut *buf))
                    {
                        choice = do_swapexists(buf, fname);
                    }

                    if choice == SEA_CHOICE_NONE && g_st().swap_exists_action == SEA_READONLY {
                        choice = SEA_CHOICE_READONLY;
                    }

                    ml_st().proc_running = 0;
                    if choice == SEA_CHOICE_NONE {
                        g_st().no_wait_return += 1;
                        let mut msg_sb = Vec::<u8>::with_capacity(IOSIZE);
                        let fhname = home_replace_save(None, fname);
                        let fhname_ptr = fhname.as_ptr().cast_mut();
                        attention_message(buf, fname, fhname_ptr, &mut msg_sb);

                        g_st().got_int = false;
                        flush_buffers(FLUSH_TYPEAHEAD);

                        if g_st().swap_exists_action != SEA_NONE {
                            sb_push_cstr(&mut msg_sb, tr(c"Swap file \""));
                            sb_push_cstr(&mut msg_sb, fhname_ptr);
                            sb_push_cstr(&mut msg_sb, tr(c"\" already exists!"));
                            msg_sb.push(0);
                            let run_but =
                                tr(c"&Open Read-Only\n&Edit anyway\n&Recover\n&Quit\n&Abort");
                            let but = tr(
                                    c"&Open Read-Only\n&Edit anyway\n&Recover\n&Delete it\n&Quit\n&Abort",
                                );
                            choice = do_dialog(
                                VIM_WARNING,
                                tr(c"VIM - ATTENTION"),
                                msg_sb.as_ptr().cast(),
                                if ml_st().proc_running != 0 { run_but } else { but },
                                1,
                                ptr::null(),
                                0,
                            );

                            if ml_st().proc_running != 0 && choice >= 4 {
                                choice += 1;
                            }
                            msg_reset_scroll();
                        } else {
                            let mut need_clear = false;
                            msg_ext_set_kind(c"wmsg");
                            let msg_len = msg_sb.len();
                            msg_sb.push(0);
                            msg_multiline(
                                NvimString {
                                    data: msg_sb.as_mut_ptr().cast(),
                                    size: msg_len,
                                },
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
                            buf.b_p_ro = 1;
                        }
                        SEA_CHOICE_EDIT => {}
                        SEA_CHOICE_RECOVER => {
                            g_st().swap_exists_action = SEA_RECOVER;
                        }
                        SEA_CHOICE_DELETE => {
                            os_remove(fname);
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

                    if choice != SEA_CHOICE_NONE && !os_path_exists(fname) {
                        break;
                    }
                }
            }

            if crate::memory::cstr_byte_at(fname, n - 1) == b'a' {
                if crate::memory::cstr_byte_at(fname, n - 2) == b'a' {
                    emsg(tr(c"E326: Too many swap files found"));
                    fname_vec = None;
                    break;
                }
                *fname.add(n - 2) -= 1;
                *fname.add(n - 1) = (b'z' + 1) as c_char;
            }
            *fname.add(n - 1) -= 1;
        }

        if os_isdir(dir_name) {
            *found_existing_dir = true;
        } else if !*found_existing_dir && **dirp == NUL {
            let mut failed_dir: Option<std::ffi::CString> = None;
            let ret = os_mkdir_recurse(dir_name, 0o755, &mut failed_dir, None);
            if ret != 0 {
                semsg!(
                    tr(c"E303: Unable to create directory \"%s\" for swap file, recovery impossible: %s"),
                    failed_dir.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr()),
                    uv_strerror(ret),
                );
            }
        }

        fname_vec.map(|v| crate::memory::cstr_to_cstring(v.as_ptr()))
    }
}

#[inline]
fn b0_magic_wrong(b0p: &ZeroBlock) -> bool {
    b0p.b0_magic_long != B0_MAGIC_LONG
            || b0p.b0_magic_int != B0_MAGIC_INT
            || b0p.b0_magic_short != B0_MAGIC_SHORT
            || b0p.b0_magic_char != B0_MAGIC_CHAR
}

fn fnamecmp_ino(fname_c: impl crate::strings::IntoCCharPtr, fname_s: impl crate::strings::IntoCCharPtr, ino_block0: c_long) -> bool { let fname_c = fname_c.into_c_char_ptr(); let fname_s = fname_s.into_c_char_ptr();
     {
        let mut ino_c: u64 = 0;
        let mut buf_c = [0 as c_char; MAXPATHL];
        let mut buf_s = [0 as c_char; MAXPATHL];

        let mut file_info = FileInfo::new();
        if os_fileinfo(fname_c, &mut file_info) {
            ino_c = os_fileinfo_inode(&file_info);
        }

        let ino_s: u64 = if os_fileinfo(fname_s, &mut file_info) {
            os_fileinfo_inode(&file_info)
        } else {
            ino_block0 as u64
        };

        if ino_c != 0 && ino_s != 0 {
            return ino_c != ino_s;
        }

        let buf_c_ptr = (&raw mut buf_c).cast::<c_char>();
        let buf_s_ptr = (&raw mut buf_s).cast::<c_char>();
        let retval_c = vim_full_name(fname_c, buf_c_ptr, MAXPATHL, true);
        let retval_s = vim_full_name(fname_s, buf_s_ptr, MAXPATHL, true);
        if retval_c && retval_s {
            return crate::memory::cstr_as_bytes(buf_c_ptr) != crate::memory::cstr_as_bytes(buf_s_ptr);
        }

        if ino_s == 0 && ino_c == 0 && !retval_c && !retval_s {
            return crate::memory::cstr_as_bytes(fname_c) != crate::memory::cstr_as_bytes(fname_s);
        }
        true
    }
}

#[inline]
fn long_to_char(n: c_long, s_in: *mut c_char) {
    unsafe {
        let mut u = n as c_uint;
        let s = s_in.cast::<u8>();
        *s.add(0) = (u & 0xff) as u8;
        u >>= 8;
        *s.add(1) = (u & 0xff) as u8;
        u >>= 8;
        *s.add(2) = (u & 0xff) as u8;
        u >>= 8;
        *s.add(3) = (u & 0xff) as u8;
    }
}

#[inline]
fn char_to_long(s_in: impl crate::strings::IntoCCharPtr) -> c_long { let s_in = s_in.into_c_char_ptr();
    unsafe {
        let s = s_in.cast::<u8>();
        let mut retval = c_long::from(*s.add(3));
        retval <<= 8;
        retval |= c_long::from(*s.add(2));
        retval <<= 8;
        retval |= c_long::from(*s.add(1));
        retval <<= 8;
        retval |= c_long::from(*s.add(0));
        retval
    }
}

/// Set the flags in the first block of the swapfile.
pub fn ml_setflags(buf: &mut FileBuf) {
    unsafe {
        if buf.b_ml.ml_mfp.is_none() {
            return;
        }
        let mfp = buf.b_ml.mfp_ptr();
        let hp = pmap_get_bhdr(&(*mfp).mf_hash, 0);
        if !hp.is_null() {
            let b0p = (*hp).data_ptr().cast::<ZeroBlock>();
            (*b0p).set_dirty(if buf.b_changed != 0 { B0_DIRTY } else { 0 });
            let ff_bits = ((get_fileformat(buf) + 1) as u8) & B0_FF_MASK;
            (*b0p).set_flags(((*b0p).flags() & !B0_FF_MASK) | ff_bits);
            add_b0_fenc(&mut *b0p, buf);
            (*hp).bh_flags |= BH_DIRTY;
            mf_sync(&mut *mfp, MFS_ZERO);
        }
    }
}

fn ml_updatechunk(buf: &mut FileBuf, line: Linenr, mut len: c_int, updtype: c_int) {
    unsafe {
        let mut curline = ml_st().ml_upd_lastcurline;
        let mut curix = ml_st().ml_upd_lastcurix;

        if buf.b_ml.ml_usedchunks == -1 || len == 0 {
            return;
        }
        if buf.b_ml.ml_chunksize.is_empty() {
            buf.b_ml.ml_chunksize.resize(100, Chunksize::default());
            buf.b_ml.ml_usedchunks = 1;
            let cs0 = buf.b_ml.ml_chunksize.as_mut_ptr();
            (*cs0).mlcs_numlines = 1;
            (*cs0).mlcs_totalsize = 1;
            ml_st().ml_upd_lastbuf = ptr::null_mut();
        }

        if updtype == ML_CHNK_UPDLINE && buf.b_ml.ml_line_count == 1 {
            buf.b_ml.ml_usedchunks = 1;
            let cs0 = buf.b_ml.ml_chunksize.as_mut_ptr();
            (*cs0).mlcs_numlines = 1;
            (*cs0).mlcs_totalsize = buf.b_ml.ml_line_textlen;
            ml_st().ml_upd_lastbuf = ptr::null_mut();
            return;
        }

        if (&raw mut *buf) != ml_st().ml_upd_lastbuf || line < curline {
            curline = 1;
            curix = 0;
        }
        let chunks = buf.b_ml.ml_chunksize.as_mut_ptr();
        while curix < buf.b_ml.ml_usedchunks - 1
            && line >= curline + (*chunks.add(curix as usize)).mlcs_numlines
        {
            curline += (*chunks.add(curix as usize)).mlcs_numlines;
            curix += 1;
        }
        let mut curchnk = chunks.add(curix as usize);

        if updtype == ML_CHNK_DELLINE {
            len = -len;
        }
        (*curchnk).mlcs_totalsize += len;
        if updtype == ML_CHNK_ADDLINE {
            (*curchnk).mlcs_numlines += 1;

            if (buf.b_ml.ml_usedchunks + 1) as usize >= buf.b_ml.ml_chunksize.len() {
                let new_len = buf.b_ml.ml_chunksize.len() * 3 / 2;
                buf
                    .b_ml
                    .ml_chunksize
                    .resize(new_len, Chunksize::default());
            }
            let chunks = buf.b_ml.ml_chunksize.as_mut_ptr();

            if (*chunks.add(curix as usize)).mlcs_numlines >= MLCS_MAXL {
                ptr::copy(
                    chunks.add(curix as usize),
                    chunks.add((curix + 1) as usize),
                    (buf.b_ml.ml_usedchunks - curix) as usize,
                );
                let mut size: c_int = 0;
                let mut linecnt: c_int = 0;
                while curline < buf.b_ml.ml_line_count && linecnt < MLCS_MINL {
                    let hp = ml_find_line(buf, curline, ML_FIND);
                    if hp.is_null() {
                        buf.b_ml.ml_usedchunks = -1;
                        return;
                    }
                    let dp = (*hp).data_ptr().cast::<DataBlock>();
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
                    let text_end = if idx == 0 {
                        (*dp).db_txt_end as c_int
                    } else {
                        (*db_idx_ptr(&mut *dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int
                    };
                    size +=
                        text_end - ((*db_idx_ptr(&mut *dp, end_idx as usize) & DB_INDEX_MASK) as c_int);
                }
                (*chunks.add(curix as usize)).mlcs_numlines = linecnt;
                (*chunks.add((curix + 1) as usize)).mlcs_numlines -= linecnt;
                (*chunks.add(curix as usize)).mlcs_totalsize = size;
                (*chunks.add((curix + 1) as usize)).mlcs_totalsize -= size;
                buf.b_ml.ml_usedchunks += 1;
                ml_st().ml_upd_lastbuf = ptr::null_mut();
                return;
            } else if (*chunks.add(curix as usize)).mlcs_numlines >= MLCS_MINL
                && curix == buf.b_ml.ml_usedchunks - 1
                && buf.b_ml.ml_line_count - line <= 1
            {
                curchnk = chunks.add((curix + 1) as usize);
                buf.b_ml.ml_usedchunks += 1;
                if line == buf.b_ml.ml_line_count {
                    (*curchnk).mlcs_numlines = 0;
                    (*curchnk).mlcs_totalsize = 0;
                } else {
                    let hp = ml_find_line(buf, buf.b_ml.ml_line_count, ML_FIND);
                    if hp.is_null() {
                        buf.b_ml.ml_usedchunks = -1;
                        return;
                    }
                    let dp = (*hp).data_ptr().cast::<DataBlock>();
                    let rest = if (*dp).db_line_count == 1 {
                        ((*dp).db_txt_end - (*dp).db_txt_start) as c_int
                    } else {
                        ((*db_idx_ptr(&mut *dp, ((*dp).db_line_count - 2) as usize) & DB_INDEX_MASK)
                            as c_int)
                            - ((*dp).db_txt_start as c_int)
                    };
                    (*curchnk).mlcs_totalsize = rest;
                    (*curchnk).mlcs_numlines = 1;
                    (*curchnk.offset(-1)).mlcs_totalsize -= rest;
                    (*curchnk.offset(-1)).mlcs_numlines -= 1;
                }
            }
        } else if updtype == ML_CHNK_DELLINE {
            (*curchnk).mlcs_numlines -= 1;
            let chunks = buf.b_ml.ml_chunksize.as_mut_ptr();
            if curix < buf.b_ml.ml_usedchunks - 1
                && ((*curchnk).mlcs_numlines + (*curchnk.add(1)).mlcs_numlines) <= MLCS_MINL
            {
                curix += 1;
                curchnk = chunks.add(curix as usize);
            } else if curix == 0 && (*curchnk).mlcs_numlines <= 0 {
                ml_st().ml_upd_lastbuf = ptr::null_mut();
                buf.b_ml.ml_usedchunks -= 1;
                ptr::copy(chunks.add(1), chunks, buf.b_ml.ml_usedchunks as usize);
                return;
            } else if curix == 0
                || ((*curchnk).mlcs_numlines > 10
                    && ((*curchnk).mlcs_numlines + (*curchnk.offset(-1)).mlcs_numlines) > MLCS_MINL)
            {
                ml_st().ml_upd_lastbuf = (&raw mut *buf);
                ml_st().ml_upd_lastcurline = curline;
                ml_st().ml_upd_lastcurix = curix;
                return;
            }

            ml_st().ml_upd_lastbuf = ptr::null_mut();
            (*curchnk.offset(-1)).mlcs_numlines += (*curchnk).mlcs_numlines;
            (*curchnk.offset(-1)).mlcs_totalsize += (*curchnk).mlcs_totalsize;
            buf.b_ml.ml_usedchunks -= 1;
            if curix < buf.b_ml.ml_usedchunks {
                ptr::copy(
                    chunks.add((curix + 1) as usize),
                    chunks.add(curix as usize),
                    (buf.b_ml.ml_usedchunks - curix) as usize,
                );
            }
            return;
        }
        ml_st().ml_upd_lastbuf = (&raw mut *buf);
        ml_st().ml_upd_lastcurline = curline;
        ml_st().ml_upd_lastcurix = curix;
    }
}

/// Find offset for line or line with offset.
pub fn ml_find_line_or_offset(
    buf: &mut FileBuf,
    lnum: Linenr,mut offp: Option<&mut c_int>,
    no_ff: bool,
) -> c_int {
    unsafe {
        let ffdos = c_int::from(!no_ff && get_fileformat(buf) == EOL_DOS);
        let mut extra: c_int = 0;

        let can_cache = lnum != 0 && ffdos == 0 && buf.b_ml.ml_line_lnum == lnum;
        if lnum == 0 || buf.b_ml.ml_line_lnum < lnum || !no_ff {
            ml_flush_line(curbuf_mut(), false);
        } else if can_cache && buf.b_ml.ml_line_offset > 0 {
            return buf.b_ml.ml_line_offset as c_int;
        }

        if buf.b_ml.ml_usedchunks == -1 || buf.b_ml.ml_chunksize.is_empty() || lnum < 0 {
            if no_ff && buf.b_ml.ml_mfp.is_some() && (lnum == 1 || lnum == 2) {
                return lnum - 1;
            }
            return -1;
        }

        let offset = if offp.is_none() { 0 } else { *offp.as_deref_mut().unwrap() };
        if lnum == 0 && offset <= 0 {
            return 1;
        }

        let mut curline: Linenr = 1;
        let mut curix: c_int = 0;
        let mut size: c_int = 0;
        let chunks = buf.b_ml.ml_chunksize.as_mut_ptr();
        while curix < buf.b_ml.ml_usedchunks - 1
            && ((lnum != 0 && lnum >= curline + (*chunks.add(curix as usize)).mlcs_numlines)
                || (offset != 0
                    && offset
                        > size
                            + (*chunks.add(curix as usize)).mlcs_totalsize
                            + ffdos * (*chunks.add(curix as usize)).mlcs_numlines))
        {
            let ch = chunks.add(curix as usize);
            curline += (*ch).mlcs_numlines;
            size += (*ch).mlcs_totalsize;
            if offset != 0 && ffdos != 0 {
                size += (*ch).mlcs_numlines;
            }
            curix += 1;
        }

        while (lnum != 0 && curline < lnum) || (offset != 0 && size < offset) {
            if curline > buf.b_ml.ml_line_count {
                return -1;
            }
            let hp = ml_find_line(buf, curline, ML_FIND);
            if hp.is_null() {
                return -1;
            }
            let dp = (*hp).data_ptr().cast::<DataBlock>();
            let count = buf.b_ml.ml_locked_high - buf.b_ml.ml_locked_low + 1;
            let start_idx = curline - buf.b_ml.ml_locked_low;
            let mut idx = start_idx;
            let text_end = if idx == 0 {
                (*dp).db_txt_end as c_int
            } else {
                (*db_idx_ptr(&mut *dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int
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
                        < size + text_end
                            - ((*db_idx_ptr(&mut *dp, idx as usize) & DB_INDEX_MASK) as c_int)
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
            let len = text_end - ((*db_idx_ptr(&mut *dp, idx as usize) & DB_INDEX_MASK) as c_int);
            size += len;
            if offset != 0 && size >= offset {
                if size + ffdos == offset {
                    *offp.as_deref_mut().unwrap() = 0;
                } else if idx == start_idx {
                    *offp.as_deref_mut().unwrap() = offset - size + len;
                } else {
                    *offp.as_deref_mut().unwrap() = offset - size + len
                        - (text_end
                            - ((*db_idx_ptr(&mut *dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int));
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
            if (buf.b_p_fixeol == 0 || buf.b_p_bin != 0)
                && buf.b_p_eol == 0
                && lnum > buf.b_ml.ml_line_count
            {
                size -= ffdos + 1;
            }
        }

        if can_cache && size > 0 {
            buf.b_ml.ml_line_offset = size as usize;
        }

        size
    }
}

/// Goto byte in buffer with offset `cnt`.
pub fn goto_byte(cnt: c_int) {
     {
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
}

/// Increment the line pointer `lp` crossing line boundaries as necessary.
pub fn inc(lp: &mut CursorPos) -> c_int {
     {
        if lp.col != MAXCOL {
            let p = ml_get_pos(lp);
            if !crate::memory::cstr_is_empty(p) {
                let l = utfc_ptr2len(p);
                lp.col += l;
                return if crate::memory::cstr_byte_at(p, l as usize) != 0 { 0 } else { 2 };
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
}

/// Same as `inc()`, but skip NUL at the end of non-empty lines.
pub fn incl(lp: &mut CursorPos) -> c_int {
    let mut r = inc(lp);
        if r >= 1 && lp.col != 0 {
            r = inc(lp);
        }
        r
}

/// Decrements position `lp` by one character (multibyte-aware), crossing line boundaries.
pub fn dec(lp: &mut CursorPos) -> c_int {
    unsafe {
        lp.coladd = 0;
        if lp.col == MAXCOL {
            let p = ml_get(lp.lnum);
            lp.col = ml_get_len(lp.lnum);
            lp.col -= utf_head_off(p, p.offset(lp.col as isize));
            return 0;
        }

        if lp.col > 0 {
            lp.col -= 1;
            let p = ml_get(lp.lnum);
            lp.col -= utf_head_off(p, p.offset(lp.col as isize));
            return 0;
        }
        if lp.lnum > 1 {
            lp.lnum -= 1;
            let p = ml_get(lp.lnum);
            lp.col = ml_get_len(lp.lnum);
            lp.col -= utf_head_off(p, p.offset(lp.col as isize));
            return 1;
        }

        -1
    }
}

/// Same as `dec()`, but skip NUL at the end of non-empty lines.
pub fn decl(lp: &mut CursorPos) -> c_int {
    let mut r = dec(lp);
        if r == 1 && lp.col != 0 {
            r = dec(lp);
        }
        r
}
