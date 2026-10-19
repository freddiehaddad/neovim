//! Buffer line storage and swapfile recovery (`src/nvim/memline.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_mut,
    unused_variables,
    clippy::collapsible_if,
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::useless_conversion
)]

use crate::globals::*;
use crate::message::{semsg, siemsg, smsg};
use crate::option_vars::*;
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use std::ptr;

use crate::api::command::cmdmod_T;
use crate::api::private::validate::{Object, ObjectData, ObjectType};
use crate::api::tabpage::win_T;
use crate::autocmd::{
    EVENT_BUFREADPOST, EVENT_BUFWINENTER, EVENT_SWAPEXISTS, apply_autocmds, has_autocmd,
};
use crate::change::{changed_internal, unchanged};
use crate::clipboard::{list_T, listitem_T, typval_T};
use crate::cursor::{check_cursor, coladvance, colnr_T, linenr_T};
use crate::drawscreen::redraw_curbuf_later;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{buf_T, pos_T};
use crate::eval::fs::FileInfo;
use crate::input::flush_buffers;
use crate::map::{Map, mh_get_int64_t, ptr_t};
use crate::map_glyph_cache::{MH_TOMBSTONE, NvimString};
use crate::mark::setpcmark;
use crate::mbyte::{mb_utflen, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::memfile::{
    bhdr_T, blocknr_T, memfile_T, mf_close, mf_close_file, mf_free, mf_free_fnames, mf_get,
    mf_need_trans, mf_new, mf_new_page_size, mf_open, mf_open_file, mf_put, mf_set_dirty,
    mf_set_fnames, mf_sync, mf_trans_del, off_T,
};
use crate::memory::{xfree, xmalloc, xmemdupz, xrealloc, xstpcpy, xstrdup, xstrlcpy};
use crate::os::env::{expand_env, home_replace, home_replace_save, os_get_hostname, os_get_pid};
use crate::os::fs::{
    os_fileinfo, os_fileinfo_inode, os_fileinfo_link, os_fileinfo_size, os_fileinfo2, os_isdir,
    os_mkdir_recurse, os_open, os_path_exists, os_remove, os_set_cloexec,
};
use crate::os::input::{line_breakcheck, os_char_avail};
use crate::os::proc::os_proc_running;
use crate::os::time::{Timestamp, os_ctime_r, os_time, time_t};
#[cfg(unix)]
use crate::os::users::os_get_uname;
use crate::os::users::os_get_username;
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{
    FreeWild, after_pathsep, concat_fnames, expand_wildcards, fix_fname, path_equal,
    path_is_absolute, path_tail, same_directory, shorten_dir, vim_FullName, vim_ispathsep,
};
use crate::spell::spell_delete_wordlist;
use crate::statusline::get_trans_bufname;

const NUL: c_char = 0;
const NL: c_char = b'\n' as c_char;
const OK: c_int = 1;
const FAIL: c_int = 0;
const NOTDONE: c_int = 2;

const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const MAXLNUM: linenr_T = 0x7fffffff;
const MAXCOL: colnr_T = 0x7fffffff;

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

const ML_EMPTY: c_int = 0x01;
const ML_LINE_DIRTY: c_int = 0x02;
const ML_LOCKED_DIRTY: c_int = 0x04;
const ML_LOCKED_POS: c_int = 0x08;
const ML_ALLOCATED: c_int = 0x10;

const ML_DEL_MESSAGE: c_int = 1;

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

const MIN_SWAP_PAGE_SIZE: c_uint = 1048;
const BH_DIRTY: c_uint = 1;
const MF_DIRTY_YES: c_int = 1;
const MF_DIRTY_YES_NOSYNC: c_int = 2;
const MFS_ALL: c_int = 1;
const MFS_STOP: c_int = 2;
const MFS_FLUSH: c_int = 4;
const MFS_ZERO: c_int = 8;

const SEEK_END: c_int = 2;
const O_RDONLY: c_int = 0;
const O_RDWR: c_int = 2;

const CMOD_NOSWAPFILE: c_int = 0x2000;
const BF_RECOVERED: c_int = 0x01;
const BF_DUMMY: c_int = 0x80;

const HLF_E: c_int = 6;
const UPD_NOT_VALID: c_int = 40;
const READ_NEW: c_int = 0x01;
const OPT_LOCAL: c_int = 0x02;
const EOL_DOS: c_int = 1;
const kOptFileencoding: c_int = 92;
const kShmAttention: c_int = b'A' as c_int;
const kUIMessages: c_int = 4;
const FLUSH_TYPEAHEAD: c_int = 1;
const VIM_WARNING: c_int = 2;

const SEA_NONE: c_int = 0;
const SEA_QUIT: c_int = 2;
const SEA_RECOVER: c_int = 3;
const SEA_READONLY: c_int = 4;

const VV_SWAPNAME: c_int = 47;
const VV_SWAPCHOICE: c_int = 48;

const EW_FILE: c_int = 0x02;
const EW_KEEPALL: c_int = 0x10;
const EW_SILENT: c_int = 0x20;

const kPathCmpLiteral: c_int = 1 << 0;
const kPathCmpFull: c_int = 1 << 2;
const kPathDeviceUNC: c_int = 5;

const ENOENT: c_int = 2;
const EINVAL: c_int = 22;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct infoptr_T {
    pub ip_bnum: blocknr_T,
    pub ip_low: linenr_T,
    pub ip_high: linenr_T,
    pub ip_index: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct chunksize_T {
    pub mlcs_numlines: c_int,
    pub mlcs_totalsize: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointerEntry {
    pe_bnum: blocknr_T,
    pe_line_count: linenr_T,
    pe_old_lnum: linenr_T,
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
enum upd_block0_T {
    UB_FNAME = 0,
    UB_SAME_DIR = 1,
}

pub type sea_choice_T = c_int;
const SEA_CHOICE_NONE: sea_choice_T = 0;
const SEA_CHOICE_READONLY: sea_choice_T = 1;
const SEA_CHOICE_EDIT: sea_choice_T = 2;
const SEA_CHOICE_RECOVER: sea_choice_T = 3;
const SEA_CHOICE_DELETE: sea_choice_T = 4;
const SEA_CHOICE_QUIT: sea_choice_T = 5;
const SEA_CHOICE_ABORT: sea_choice_T = 6;

static mut lowest_marked: linenr_T = 0;
static mut proc_running: c_int = 0;
static mut ml_get_recursive: c_int = 0;
static mut ml_get_questions: [c_char; 4] = [0; 4];
static mut ml_flush_entered: bool = false;
static mut ml_upd_lastbuf: *mut buf_T = ptr::null_mut();
static mut ml_upd_lastcurline: linenr_T = 0;
static mut ml_upd_lastcurix: c_int = 0;

static e_ml_get_invalid_lnum_nr: &CStr = c"E315: ml_get: Invalid lnum: %lld";
static e_ml_get_cannot_find_line_nr_in_buffer_nr_str: &CStr =
    c"E316: ml_get: Cannot find line %lldin buffer %d %s";
static e_pointer_block_id_wrong: &CStr = c"E317: Pointer block id wrong";
static e_pointer_block_id_wrong_two: &CStr = c"E317: Pointer block id wrong 2";
static e_pointer_block_id_wrong_three: &CStr = c"E317: Pointer block id wrong 3";
static e_pointer_block_id_wrong_four: &CStr = c"E317: Pointer block id wrong 4";
static e_line_number_out_of_range_nr_past_the_end: &CStr =
    c"E322: Line number out of range: %lld past the end";
static e_line_count_wrong_in_block_nr: &CStr = c"E323: Line count wrong in block %lld";
static e_warning_pointer_block_corrupted: &CStr = c"E1364: Warning: Pointer block corrupted";

unsafe extern "C" {
    static mut Versions: [*mut c_char; 1];

    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn emsg(s: *const c_char) -> bool;
    fn iemsg(s: *const c_char);
    fn msg(s: *const c_char, hl_id: c_int) -> bool;
    fn verb_msg(s: *const c_char) -> c_int;
    fn msg_start();
    fn msg_end() -> bool;
    fn msg_putchar(c: c_int);
    fn msg_puts(s: *const c_char);
    fn msg_puts_hl(s: *const c_char, hl_id: c_int, hist: bool);
    fn msg_outtrans(str_: *const c_char, hl_id: c_int, hist: bool) -> c_int;
    fn msg_outnum(n: c_int);
    fn msg_ext_set_kind(msg_kind: *const c_char);
    fn msg_reset_scroll();
    fn msg_multiline(
        str_: NvimString,
        hl_id: c_int,
        check_int: bool,
        hist: bool,
        need_clear: *mut bool,
    );
    fn set_keep_msg(s: *const c_char, hl_id: c_int);

    fn get_fileformat(buf: *const buf_T) -> c_int;
    fn set_fileformat(eol_style: c_int, opt_flags: c_int);
    fn set_option_value_give_err(opt_idx: c_int, value: Object, opt_flags: c_int);
    fn copy_option_part(
        option: *mut *mut c_char,
        buf: *mut c_char,
        maxlen: usize,
        sep_chars: *const c_char,
    ) -> usize;

    fn close(fd: c_int) -> c_int;
    fn uv_uptime(uptime: *mut f64) -> c_int;
    #[link_name = "uv_strerror"]
    fn os_strerror(err: c_int) -> *const c_char;
    fn read_eintr(fd: c_int, buf: *mut c_void, bufsize: usize) -> isize;
    #[cfg_attr(windows, link_name = "_lseeki64")]
    #[cfg_attr(not(windows), link_name = "lseek")]
    fn vim_lseek(fd: c_int, offset: off_T, whence: c_int) -> off_T;
    #[cfg(unix)]
    fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: usize) -> isize;

    fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char;
    fn mb_adjust_cursor();
    fn ui_flush();
    fn ui_has(ext: c_int) -> bool;

    fn vim_rename(from: *const c_char, to: *const c_char) -> c_int;
    fn vim_tempname() -> *mut c_char;
    fn vim_deltempdir();
    fn modname(fname: *const c_char, ext: *const c_char, prepend_dot: bool) -> *mut c_char;
    fn buf_spname(buf: *mut buf_T) -> *mut c_char;
    fn bufIsChanged(buf: *mut buf_T) -> bool;
    fn buf_store_file_info(buf: *mut buf_T, file_info: *mut FileInfo);
    fn buf_inc_changedtick(buf: *mut buf_T);
    fn open_buffer(read_stdin: bool, eap: *mut c_void, flags_arg: c_int) -> c_int;
    fn setfname(
        buf: *mut buf_T,
        ffname_arg: *mut c_char,
        sfname_arg: *mut c_char,
        message: bool,
    ) -> c_int;
    fn readfile(
        fname: *mut c_char,
        sfname: *mut c_char,
        from: linenr_T,
        lines_to_skip: linenr_T,
        lines_to_read: linenr_T,
        eap: *mut c_void,
        flags: c_int,
        silent: bool,
    ) -> c_int;
    fn getout(r: c_int) -> !;
    fn nlua_call_typval(
        module: *const c_char,
        func: *const c_char,
        argvars: *mut typval_T,
        rettv: *mut typval_T,
    );
    fn do_dialog(
        type_: c_int,
        title: *const c_char,
        message: *const c_char,
        buttons: *const c_char,
        dfltbutton: c_int,
        textfield: *const c_char,
        ex_cmd: c_int,
    ) -> c_int;

    fn set_vim_var_string(idx: c_int, val: *const c_char, len: isize);
    fn get_vim_var_str(idx: c_int) -> *mut c_char;

    fn tv_list_alloc_ret(ret_tv: *mut typval_T, len: isize) -> *mut list_T;
    fn tv_list_append_allocated_string(l: *mut list_T, str_: *mut c_char);
    fn tv_clear(varp: *mut typval_T);
    fn tv_dict_add_str_len(
        d: *mut dict_T,
        key: *const c_char,
        key_len: usize,
        val: *const c_char,
        val_len: c_int,
    ) -> c_int;
    fn tv_dict_add_nr(d: *mut dict_T, key: *const c_char, key_len: usize, nr: i64) -> c_int;
}

#[inline]
const fn ascii_isalpha(c: c_int) -> bool {
    (c >= b'a' as c_int && c <= b'z' as c_int) || (c >= b'A' as c_int && c <= b'Z' as c_int)
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    unsafe {
        if s.is_null() {
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            }
        } else {
            NvimString {
                data: s.cast_mut(),
                size: c_strlen(s),
            }
        }
    }
}

#[inline]
unsafe fn xstrnsave(string: *const c_char, len: usize) -> *mut c_char {
    unsafe { xmemdupz(string.cast(), len).cast() }
}

#[inline]
unsafe fn tr(s: &'static CStr) -> *mut c_char {
    unsafe { gettext(s.as_ptr()) }
}

#[inline]
const fn ml_simple(x: c_int) -> bool {
    (x & 0x10) != 0
}

#[inline]
unsafe fn pb_count_max(mfp: *const memfile_T) -> u16 {
    unsafe {
        (((*mfp).mf_page_size as usize - std::mem::offset_of!(PointerBlock, pb_pointer))
            / size_of::<PointerEntry>()) as u16
    }
}

#[inline]
unsafe fn pb_ptr(pp: *mut PointerBlock, idx: usize) -> *mut PointerEntry {
    unsafe { (&raw mut (*pp).pb_pointer).cast::<PointerEntry>().add(idx) }
}

#[inline]
unsafe fn db_idx_ptr(dp: *mut DataBlock, idx: usize) -> *mut c_uint {
    unsafe { (&raw mut (*dp).db_index).cast::<c_uint>().add(idx) }
}

#[inline]
unsafe fn c_strlen(s: *const c_char) -> usize {
    if s.is_null() {
        return 0;
    }
    unsafe { CStr::from_ptr(s).to_bytes().len() }
}

#[inline]
unsafe fn c_stricmp(mut a: *const c_char, mut b: *const c_char) -> c_int {
    unsafe {
        loop {
            let ca = (*a as u8).to_ascii_lowercase();
            let cb = (*b as u8).to_ascii_lowercase();
            if ca != cb || ca == 0 {
                return c_int::from(ca) - c_int::from(cb);
            }
            a = a.add(1);
            b = b.add(1);
        }
    }
}

#[inline]
unsafe fn c_strnicmp(mut a: *const c_char, mut b: *const c_char, mut len: usize) -> c_int {
    unsafe {
        while len > 0 {
            let ca = (*a as u8).to_ascii_lowercase();
            let cb = (*b as u8).to_ascii_lowercase();
            if ca != cb || ca == 0 {
                return c_int::from(ca) - c_int::from(cb);
            }
            a = a.add(1);
            b = b.add(1);
            len -= 1;
        }
        0
    }
}

#[inline]
unsafe fn c_strcmp(a: *const c_char, b: *const c_char) -> c_int {
    unsafe {
        let sa = CStr::from_ptr(a).to_bytes();
        let sb = CStr::from_ptr(b).to_bytes();
        match sa.cmp(sb) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

#[inline]
unsafe fn tv_list_len(l: *const list_T) -> c_int {
    if l.is_null() {
        0
    } else {
        unsafe { (*l).lv_len }
    }
}

#[inline]
unsafe fn tv_list_first(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*l).lv_first }
    }
}

#[inline]
unsafe fn pmap_get_bhdr(map: *mut Map<i64, ptr_t>, key: i64) -> *mut bhdr_T {
    unsafe {
        let k = mh_get_int64_t(&raw mut (*map).set, key);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*(*map).values.add(k as usize)).cast::<bhdr_T>()
        }
    }
}

#[inline]
unsafe fn sb_push_cstr(sb: &mut Vec<u8>, s: *const c_char) {
    if !s.is_null() {
        unsafe {
            sb.extend_from_slice(CStr::from_ptr(s).to_bytes());
        }
    }
}

/// Open a new memline for `buf`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_open(buf: *mut buf_T) -> c_int {
    unsafe {
        (*buf).b_ml.ml_stack_size = 0;
        (*buf).b_ml.ml_stack = ptr::null_mut();
        (*buf).b_ml.ml_stack_top = 0;
        (*buf).b_ml.ml_locked = ptr::null_mut();
        (*buf).b_ml.ml_line_lnum = 0;
        (*buf).b_ml.ml_line_offset = 0;
        (*buf).b_ml.ml_chunksize = ptr::null_mut();
        (*buf).b_ml.ml_usedchunks = 0;

        if (cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0 {
            (*buf).b_p_swf = 0;
        }

        (*buf).b_may_swap = (*buf).terminal.is_null() && p_uc != 0 && (*buf).b_p_swf != 0;

        let mfp = mf_open(ptr::null_mut(), 0);
        if mfp.is_null() {
            (*buf).b_ml.ml_mfp = ptr::null_mut();
            return FAIL;
        }

        (*buf).b_ml.ml_mfp = mfp.cast();
        (*buf).b_ml.ml_flags = ML_EMPTY;
        (*buf).b_ml.ml_line_count = 1;

        let mut hp = mf_new(mfp, false, 1);
        if (*hp).bh_bnum != 0 {
            iemsg(tr(c"E298: Didn't get block nr 0?"));
            return ml_open_error(buf, mfp, hp);
        }
        let b0p = (*hp).bh_data.cast::<ZeroBlock>();

        (*b0p).b0_id[0] = BLOCK0_ID0;
        (*b0p).b0_id[1] = BLOCK0_ID1;
        (*b0p).b0_magic_long = B0_MAGIC_LONG;
        (*b0p).b0_magic_int = B0_MAGIC_INT;
        (*b0p).b0_magic_short = B0_MAGIC_SHORT;
        (*b0p).b0_magic_char = B0_MAGIC_CHAR;
        let ver_dst = xstpcpy((&raw mut (*b0p).b0_version).cast(), c"VIM ".as_ptr());
        xstrlcpy(ver_dst, *(&raw const Versions).cast::<*mut c_char>(), 6);
        long_to_char(
            (*mfp).mf_page_size as c_long,
            (&raw mut (*b0p).b0_page_size).cast(),
        );

        if !(*buf).b_spell {
            (*b0p).set_dirty(if (*buf).b_changed != 0 { B0_DIRTY } else { 0 });
            (*b0p).set_flags((get_fileformat(buf) + 1) as u8);
            set_b0_fname(b0p, buf);
            os_get_username((&raw mut (*b0p).b0_uname).cast(), B0_UNAME_SIZE);
            (*b0p).b0_uname[B0_UNAME_SIZE - 1] = NUL;
            os_get_hostname((&raw mut (*b0p).b0_hname).cast(), B0_HNAME_SIZE);
            (*b0p).b0_hname[B0_HNAME_SIZE - 1] = NUL;
            long_to_char(os_get_pid() as c_long, (&raw mut (*b0p).b0_pid).cast());
        }

        mf_put(mfp, hp, true, false);
        if !(*buf).b_help && !(*buf).b_spell {
            mf_sync(mfp, 0);
        }

        hp = ml_new_ptr(mfp);
        assert!(!hp.is_null());
        if (*hp).bh_bnum != 1 {
            iemsg(tr(c"E298: Didn't get block nr 1?"));
            return ml_open_error(buf, mfp, hp);
        }
        let pp = (*hp).bh_data.cast::<PointerBlock>();
        (*pp).pb_count = 1;
        let pe0 = pb_ptr(pp, 0);
        (*pe0).pe_bnum = 2;
        (*pe0).pe_page_count = 1;
        (*pe0).pe_old_lnum = 1;
        (*pe0).pe_line_count = 1;
        mf_put(mfp, hp, true, false);

        hp = ml_new_data(mfp, false, 1);
        if (*hp).bh_bnum != 2 {
            iemsg(tr(c"E298: Didn't get block nr 2?"));
            return ml_open_error(buf, mfp, hp);
        }

        let dp = (*hp).bh_data.cast::<DataBlock>();
        (*dp).db_txt_start -= 1;
        *db_idx_ptr(dp, 0) = (*dp).db_txt_start;
        (*dp).db_free -= 1 + INDEX_SIZE as c_uint;
        (*dp).db_line_count = 1;
        *dp.cast::<c_char>().add((*dp).db_txt_start as usize) = NUL;

        OK
    }
}

unsafe fn ml_open_error(buf: *mut buf_T, mfp: *mut memfile_T, hp: *mut bhdr_T) -> c_int {
    unsafe {
        if !mfp.is_null() {
            if !hp.is_null() {
                mf_put(mfp, hp, false, false);
            }
            mf_close(mfp, true);
        }
        (*buf).b_ml.ml_mfp = ptr::null_mut();
        FAIL
    }
}

/// Called when the file name of `buf` has been changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_setname(buf: *mut buf_T) {
    unsafe {
        let mut success = false;
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if (*mfp).mf_fd < 0 {
            if p_uc != 0 && (cmdmod.cmod_flags & CMOD_NOSWAPFILE) == 0 {
                ml_open_file(buf);
            }
            return;
        }

        let mut dirp = p_dir;
        let mut found_existing_dir = false;
        loop {
            if *dirp == NUL {
                break;
            }
            let fname = findswapname(
                buf,
                &raw mut dirp,
                (*mfp).mf_fname,
                &raw mut found_existing_dir,
            );
            if (*buf).b_ml.ml_mfp.cast::<memfile_T>() != mfp {
                xfree(fname.cast());
                return;
            }
            if dirp.is_null() {
                break;
            }
            if fname.is_null() {
                continue;
            }

            if path_equal(fname, (*mfp).mf_fname, kPathCmpLiteral) {
                xfree(fname.cast());
                success = true;
                break;
            }
            if (*mfp).mf_fd >= 0 {
                close((*mfp).mf_fd);
                (*mfp).mf_fd = -1;
            }

            if vim_rename((*mfp).mf_fname, fname) == 0 {
                success = true;
                mf_free_fnames(mfp);
                mf_set_fnames(mfp, fname);
                ml_upd_block0(buf, upd_block0_T::UB_SAME_DIR);
                break;
            }
            xfree(fname.cast());
        }

        if (*mfp).mf_fd == -1 {
            (*mfp).mf_fd = os_open((*mfp).mf_fname, O_RDWR, 0);
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_open_files() {
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            if (*buf).b_p_ro == 0 || (*buf).b_changed != 0 {
                ml_open_file(buf);
            }
            buf = (*buf).b_next;
        }
    }
}

/// Open a swapfile for an existing memfile, if there is no swapfile yet.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_open_file(buf: *mut buf_T) {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if mfp.is_null()
            || (*mfp).mf_fd >= 0
            || (*buf).b_p_swf == 0
            || (cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0
            || !(*buf).terminal.is_null()
        {
            return;
        }

        if (*buf).b_spell {
            let fname = vim_tempname();
            if !fname.is_null() {
                mf_open_file(mfp, fname);
            }
            (*buf).b_may_swap = false;
            return;
        }

        let mut dirp = p_dir;
        let mut found_existing_dir = false;
        loop {
            if *dirp == NUL {
                break;
            }
            let fname = findswapname(
                buf,
                &raw mut dirp,
                ptr::null_mut(),
                &raw mut found_existing_dir,
            );
            if (*buf).b_ml.ml_mfp.cast::<memfile_T>() != mfp {
                xfree(fname.cast());
                return;
            }
            if dirp.is_null() {
                break;
            }
            if fname.is_null() {
                continue;
            }
            if mf_open_file(mfp, fname) == OK {
                (*mfp).mf_dirty = MF_DIRTY_YES_NOSYNC;
                ml_upd_block0(buf, upd_block0_T::UB_SAME_DIR);

                if mf_sync(mfp, MFS_ZERO) == OK {
                    mf_set_dirty(mfp);
                    break;
                }
                mf_close_file(buf, false);
            }
        }

        if *p_dir != NUL && (*mfp).mf_fname.is_null() {
            need_wait_return = true;
            no_wait_return += 1;
            let sp = buf_spname(buf);
            semsg!(
                tr(c"E303: Unable to open swap file for \"%s\", recovery impossible"),
                if !sp.is_null() { sp } else { (*buf).b_fname },
            );
            no_wait_return -= 1;
        }

        (*buf).b_may_swap = false;
    }
}

/// If still need to create a swapfile, create a swapfile now.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_need_swap(newfile: bool) {
    unsafe {
        let old_msg_silent = msg_silent;
        msg_silent = 0;

        if (*curbuf).b_may_swap && ((*curbuf).b_p_ro == 0 || !newfile) {
            ml_open_file(curbuf);
        }

        msg_silent = old_msg_silent;
    }
}

/// Close memline for buffer `buf`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_close(buf: *mut buf_T, del_file: c_int) {
    unsafe {
        if (*buf).b_ml.ml_mfp.is_null() {
            return;
        }
        mf_close((*buf).b_ml.ml_mfp.cast(), del_file != 0);
        if (*buf).b_ml.ml_line_lnum != 0
            && ((*buf).b_ml.ml_flags & (ML_LINE_DIRTY | ML_ALLOCATED)) != 0
        {
            xfree((*buf).b_ml.ml_line_ptr.cast());
        }
        xfree((*buf).b_ml.ml_stack);
        xfree((*buf).b_ml.ml_chunksize);
        (*buf).b_ml.ml_chunksize = ptr::null_mut();
        (*buf).b_ml.ml_mfp = ptr::null_mut();

        (*buf).b_flags &= !BF_RECOVERED;
    }
}

/// Close all existing memlines and memfiles.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_close_all(del_file: bool) {
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            ml_close(buf, c_int::from(del_file));
            buf = (*buf).b_next;
        }
        spell_delete_wordlist();
        vim_deltempdir();
    }
}

/// Close all memfiles for not modified buffers.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_close_notmod() {
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            if !bufIsChanged(buf) {
                ml_close(buf, 1);
            }
            buf = (*buf).b_next;
        }
    }
}

/// Update the timestamp in the `.swp` file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_timestamp(buf: *mut buf_T) {
    unsafe {
        ml_upd_block0(buf, upd_block0_T::UB_FNAME);
    }
}

#[inline]
unsafe fn ml_check_b0_id(b0p: *const ZeroBlock) -> bool {
    unsafe { (*b0p).b0_id[0] == BLOCK0_ID0 && (*b0p).b0_id[1] == BLOCK0_ID1 }
}

#[inline]
unsafe fn ml_check_b0_strings(b0p: *const ZeroBlock) -> bool {
    unsafe {
        let b0 = &*b0p;
        b0.b0_version.contains(&NUL)
            && b0.b0_uname.contains(&NUL)
            && b0.b0_hname.contains(&NUL)
            && b0.b0_fname[..B0_FNAME_SIZE_CRYPT].contains(&NUL)
    }
}

unsafe fn ml_upd_block0(buf: *mut buf_T, what: upd_block0_T) {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if mfp.is_null() {
            return;
        }
        let hp = mf_get(mfp, 0, 1);
        if hp.is_null() {
            return;
        }
        let b0p = (*hp).bh_data.cast::<ZeroBlock>();
        if !ml_check_b0_id(b0p) {
            iemsg(tr(c"E304: ml_upd_block0(): Didn't get block 0??"));
        } else if what == upd_block0_T::UB_FNAME {
            set_b0_fname(b0p, buf);
        } else {
            set_b0_dir_flag(b0p, buf);
        }
        mf_put(mfp, hp, true, false);
    }
}

unsafe fn set_b0_fname(b0p: *mut ZeroBlock, buf: *mut buf_T) {
    unsafe {
        if (*buf).b_ffname.is_null() {
            (*b0p).b0_fname[0] = NUL;
        } else {
            let mut uname = [0 as c_char; B0_UNAME_SIZE];
            let fname_ptr = (&raw mut (*b0p).b0_fname).cast::<c_char>();
            let flen = home_replace(
                ptr::null_mut(),
                (*buf).b_ffname,
                fname_ptr,
                B0_FNAME_SIZE_CRYPT,
                true,
            );
            if (*b0p).b0_fname[0] == b'~' as c_char {
                let retval = os_get_username((&raw mut uname).cast(), B0_UNAME_SIZE);
                let ulen = c_strlen((&raw const uname).cast());
                if retval == FAIL || ulen + flen > B0_FNAME_SIZE_CRYPT - 1 {
                    xstrlcpy(fname_ptr, (*buf).b_ffname, B0_FNAME_SIZE_CRYPT);
                } else {
                    ptr::copy(fname_ptr.add(1), fname_ptr.add(ulen + 1), flen);
                    ptr::copy_nonoverlapping((&raw const uname).cast(), fname_ptr.add(1), ulen);
                }
            }
            let mut file_info = FileInfo::default();
            if os_fileinfo((*buf).b_ffname, &raw mut file_info) {
                long_to_char(
                    file_info.stat.st_mtim.tv_sec as c_long,
                    (&raw mut (*b0p).b0_mtime).cast(),
                );
                long_to_char(
                    os_fileinfo_inode(&raw const file_info) as c_long,
                    (&raw mut (*b0p).b0_ino).cast(),
                );
                buf_store_file_info(buf, &raw mut file_info);
                (*buf).b_mtime_read = (*buf).b_mtime;
                (*buf).b_mtime_read_ns = (*buf).b_mtime_ns;
            } else {
                long_to_char(0, (&raw mut (*b0p).b0_mtime).cast());
                long_to_char(0, (&raw mut (*b0p).b0_ino).cast());
                (*buf).b_mtime = 0;
                (*buf).b_mtime_ns = 0;
                (*buf).b_mtime_read = 0;
                (*buf).b_mtime_read_ns = 0;
                (*buf).b_orig_size = 0;
                (*buf).b_orig_mode = 0;
            }
        }

        add_b0_fenc(b0p, curbuf);
    }
}

unsafe fn set_b0_dir_flag(b0p: *mut ZeroBlock, buf: *mut buf_T) {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if same_directory((*mfp).mf_fname, (*buf).b_ffname) {
            (*b0p).set_flags((*b0p).flags() | B0_SAME_DIR);
        } else {
            (*b0p).set_flags((*b0p).flags() & !B0_SAME_DIR);
        }
    }
}

unsafe fn add_b0_fenc(b0p: *mut ZeroBlock, buf: *mut buf_T) {
    unsafe {
        let size = B0_FNAME_SIZE_NOCRYPT;
        let n = c_strlen((*buf).b_p_fenc);
        let fname_ptr = (&raw mut (*b0p).b0_fname).cast::<c_char>();
        if c_strlen(fname_ptr) + n + 1 > size {
            (*b0p).set_flags((*b0p).flags() & !B0_HAS_FENC);
        } else {
            ptr::copy((*buf).b_p_fenc, fname_ptr.add(size - n), n);
            *fname_ptr.add(size - n - 1) = NUL;
            (*b0p).set_flags((*b0p).flags() | B0_HAS_FENC);
        }
    }
}

unsafe fn swapfile_proc_running(b0p: *const ZeroBlock, swap_fname: *const c_char) -> c_int {
    unsafe {
        let mut st = FileInfo::default();
        let mut uptime: f64 = 0.0;
        if os_fileinfo(swap_fname, &raw mut st)
            && uv_uptime(&raw mut uptime) == 0
            && (st.stat.st_mtim.tv_sec as Timestamp) < os_time().wrapping_sub(uptime as Timestamp)
        {
            return 0;
        }
        let pid = char_to_long((&raw const (*b0p).b0_pid).cast()) as c_int;
        if os_proc_running(pid) { pid } else { 0 }
    }
}

/// Try to recover `curbuf` from the `.swp` file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_recover(checkext: bool) {
    unsafe {
        let mut buf: *mut buf_T = ptr::null_mut();
        let mut mfp: *mut memfile_T = ptr::null_mut();
        let mut fname_used: *mut c_char = ptr::null_mut();
        let mut hp: *mut bhdr_T = ptr::null_mut();
        let mut b0_fenc: *mut c_char = ptr::null_mut();
        let directly: bool;
        let mut serious_error = true;
        let mut orig_file_status = NOTDONE;

        recoverymode = true;
        let called_from_main = (*curbuf).b_ml.ml_mfp.is_null();

        let mut fname = (*curbuf).b_fname;
        if fname.is_null() {
            fname = c"".as_ptr().cast_mut();
        }
        let len = c_strlen(fname);
        if checkext
            && len >= 4
            && c_strnicmp(fname.add(len - 4), c".s".as_ptr(), 2) == 0
            && !vim_strchr(
                c"abcdefghijklmnopqrstuvw".as_ptr(),
                ((*fname.add(len - 2) as u8).to_ascii_lowercase()) as c_int,
            )
            .is_null()
            && ascii_isalpha(*fname.add(len - 1) as c_int)
        {
            directly = true;
            fname_used = xstrdup(fname);
        } else {
            directly = false;
            let mut items_tv = std::mem::zeroed::<typval_T>();
            tv_list_alloc_ret(&raw mut items_tv, 0);
            recover_names(fname, true, items_tv.vval.v_list);
            let n_swaps = tv_list_len(items_tv.vval.v_list);

            if n_swaps == 0 {
                tv_clear(&raw mut items_tv);
                semsg!(tr(c"E305: No swap file found for %s"), fname);
                ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
                return;
            }
            if n_swaps > 1 {
                let mut lua_args = [items_tv, std::mem::zeroed::<typval_T>()];
                nlua_call_typval(
                    c"vim._core.swapfile".as_ptr(),
                    c"select_swap".as_ptr(),
                    (&raw mut lua_args).cast(),
                    ptr::null_mut(),
                );
                tv_clear(&raw mut items_tv);
                ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
                return;
            }
            fname_used = xstrdup((*tv_list_first(items_tv.vval.v_list)).li_tv.vval.v_string);
            tv_clear(&raw mut items_tv);
        }
        if fname_used.is_null() {
            ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
            return;
        }
        if called_from_main && ml_open(curbuf) == FAIL {
            getout(1);
        }

        buf = xmalloc(size_of::<buf_T>()).cast::<buf_T>();
        (*buf).b_ml.ml_stack_size = 0;
        (*buf).b_ml.ml_stack = ptr::null_mut();
        (*buf).b_ml.ml_stack_top = 0;
        (*buf).b_ml.ml_line_lnum = 0;
        (*buf).b_ml.ml_line_offset = 0;
        (*buf).b_ml.ml_locked = ptr::null_mut();
        (*buf).b_ml.ml_flags = 0;

        let p = xstrdup(fname_used);
        mfp = mf_open(fname_used, O_RDONLY);
        fname_used = p;
        if mfp.is_null() || (*mfp).mf_fd < 0 {
            semsg!(tr(c"E306: Cannot open %s"), fname_used);
            ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
            return;
        }
        (*buf).b_ml.ml_mfp = mfp.cast();
        (*mfp).mf_page_size = MIN_SWAP_PAGE_SIZE;

        let hl_id = HLF_E;
        msg_ext_set_kind(c"emsg".as_ptr());
        hp = mf_get(mfp, 0, 1);
        if hp.is_null() {
            msg_start();
            msg_puts_hl(tr(c"Unable to read block 0 from "), hl_id, true);
            msg_outtrans((*mfp).mf_fname, hl_id, true);
            msg_puts_hl(
                tr(c"\nMaybe no changes were made or Nvim did not update the swap file."),
                hl_id,
                true,
            );
            msg_end();
            ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
            return;
        }
        let mut b0p = (*hp).bh_data.cast::<ZeroBlock>();
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
            msg_outtrans((*mfp).mf_fname, 0, true);
            msg_puts_hl(tr(c" cannot be used with this version of Nvim.\n"), 0, true);
            msg_puts_hl(tr(c"Use Vim version 3.0.\n"), 0, true);
            msg_end();
            ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
            return;
        }
        if !ml_check_b0_id(b0p) {
            semsg!(
                tr(c"E307: %s does not look like a Nvim swap file"),
                (*mfp).mf_fname,
            );
            ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
            return;
        }
        if b0_magic_wrong(b0p) {
            msg_start();
            msg_outtrans((*mfp).mf_fname, hl_id, true);
            msg_puts_hl(tr(c" cannot be used on this computer.\n"), hl_id, true);
            msg_puts_hl(tr(c"The file was created on "), hl_id, true);
            (*b0p).b0_fname[0] = NUL;
            msg_puts_hl((&raw const (*b0p).b0_hname).cast(), hl_id, true);
            msg_puts_hl(tr(c",\nor the file has been damaged."), hl_id, true);
            msg_end();
            ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
            return;
        }

        let b0_page_size = char_to_long((&raw const (*b0p).b0_page_size).cast()) as c_uint;
        if (*mfp).mf_page_size != b0_page_size {
            let previous_page_size = (*mfp).mf_page_size;
            mf_new_page_size(mfp, b0_page_size);
            if (*mfp).mf_page_size < previous_page_size {
                msg_start();
                msg_outtrans((*mfp).mf_fname, hl_id, true);
                msg_puts_hl(
                    tr(c" has been damaged (page size is smaller than minimum value).\n"),
                    hl_id,
                    true,
                );
                msg_end();
                ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
                return;
            }
            let size: off_T = vim_lseek((*mfp).mf_fd, 0, SEEK_END);
            (*mfp).mf_blocknr_max = if size <= 0 {
                0
            } else {
                size / i64::from((*mfp).mf_page_size)
            };
            (*mfp).mf_infile_count = (*mfp).mf_blocknr_max;

            let new_data = xmalloc((*mfp).mf_page_size as usize);
            ptr::copy(
                (*hp).bh_data.cast::<u8>(),
                new_data.cast::<u8>(),
                previous_page_size as usize,
            );
            xfree((*hp).bh_data);
            (*hp).bh_data = new_data;
            b0p = (*hp).bh_data.cast::<ZeroBlock>();
        }

        let namebuff_ptr = (&raw mut NameBuff).cast::<c_char>();
        if directly {
            #[cfg(windows)]
            path_to_slash((&raw mut (*b0p).b0_fname).cast());
            expand_env(
                (&raw mut (*b0p).b0_fname).cast(),
                namebuff_ptr,
                MAXPATHL as c_int,
            );
            if setfname(curbuf, namebuff_ptr, ptr::null_mut(), true) == FAIL {
                ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
                return;
            }
        }

        msg_ext_set_kind(c"wmsg".as_ptr());
        msg_ext_skip_flush = true;
        home_replace(
            ptr::null_mut(),
            (*mfp).mf_fname,
            namebuff_ptr,
            MAXPATHL,
            true,
        );
        smsg!(0, tr(c"Using swap file \"%s\""), namebuff_ptr);

        let sp = buf_spname(curbuf);
        if !sp.is_null() {
            xstrlcpy(namebuff_ptr, sp, MAXPATHL);
        } else {
            home_replace(
                ptr::null_mut(),
                (*curbuf).b_ffname,
                namebuff_ptr,
                MAXPATHL,
                true,
            );
        }
        msg_putchar(b'\n' as c_int);
        smsg!(0, tr(c"Original file \"%s\""), namebuff_ptr);
        msg_putchar(b'\n' as c_int);
        msg_ext_skip_flush = false;

        let mut org_file_info = FileInfo::default();
        let mut swp_file_info = FileInfo::default();
        let mtime = char_to_long((&raw const (*b0p).b0_mtime).cast()) as i64;
        if !(*curbuf).b_ffname.is_null()
            && os_fileinfo((*curbuf).b_ffname, &raw mut org_file_info)
            && ((os_fileinfo((*mfp).mf_fname, &raw mut swp_file_info)
                && org_file_info.stat.st_mtim.tv_sec > swp_file_info.stat.st_mtim.tv_sec)
                || i64::from(org_file_info.stat.st_mtim.tv_sec) != mtime)
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
            b0_fenc = xstrnsave(p, fname_base.add(fnsize).offset_from(p) as usize);
        }

        mf_put(mfp, hp, false, false);
        hp = ptr::null_mut();

        while ((*curbuf).b_ml.ml_flags & ML_EMPTY) == 0 {
            ml_delete(1);
        }

        if !(*curbuf).b_ffname.is_null() {
            orig_file_status = readfile(
                (*curbuf).b_ffname,
                ptr::null_mut(),
                0,
                0,
                MAXLNUM,
                ptr::null_mut(),
                READ_NEW,
                false,
            );
        }

        if b0_ff != 0 {
            set_fileformat(b0_ff - 1, OPT_LOCAL);
        }
        if !b0_fenc.is_null() {
            let fenc_obj = Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData {
                    string: cstr_as_string(b0_fenc),
                },
            };
            set_option_value_give_err(kOptFileencoding, fenc_obj, OPT_LOCAL);
            xfree(b0_fenc.cast());
        }
        unchanged(curbuf, true, true);

        let mut bnum: blocknr_T = 1;
        let mut page_count: c_uint = 1;
        let mut lnum: linenr_T = 0;
        let mut line_count: linenr_T = 0;
        let mut idx: c_int = 0;
        let mut error: c_int = 0;
        (*buf).b_ml.ml_stack_top = 0;
        (*buf).b_ml.ml_stack = ptr::null_mut();
        (*buf).b_ml.ml_stack_size = 0;

        let mut cannot_open = (*curbuf).b_ffname.is_null();

        serious_error = false;
        while !got_int {
            if !hp.is_null() {
                mf_put(mfp, hp, false, false);
            }
            hp = mf_get(mfp, bnum, page_count);
            if hp.is_null() {
                if bnum == 1 {
                    semsg!(tr(c"E309: Unable to read block 1 from %s"), (*mfp).mf_fname);
                    ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
                    return;
                }
                error += 1;
                ml_append(lnum, tr(c"???MANY LINES MISSING"), 0, true);
                lnum += 1;
            } else {
                let pp = (*hp).bh_data.cast::<PointerBlock>();
                if (*pp).pb_id == PTR_ID {
                    let mut ptr_block_error = false;
                    let max_cnt = pb_count_max(mfp);
                    if (*pp).pb_count_max != max_cnt {
                        ptr_block_error = true;
                        (*pp).pb_count_max = max_cnt;
                    }
                    if (*pp).pb_count > (*pp).pb_count_max {
                        ptr_block_error = true;
                        (*pp).pb_count = (*pp).pb_count_max;
                    }
                    if ptr_block_error {
                        emsg(tr(e_warning_pointer_block_corrupted));
                    }

                    if idx == 0 && line_count != 0 {
                        for i in 0..((*pp).pb_count as usize) {
                            line_count -= (*pb_ptr(pp, i)).pe_line_count;
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
                        let pe = pb_ptr(pp, idx as usize);
                        if (*pe).pe_bnum < 0 {
                            if !cannot_open {
                                line_count = (*pe).pe_line_count;
                                let pe_old_lnum = (*pe).pe_old_lnum;
                                if line_count <= 0
                                    || pe_old_lnum < 1
                                    || readfile(
                                        (*curbuf).b_ffname,
                                        ptr::null_mut(),
                                        lnum,
                                        pe_old_lnum - 1,
                                        line_count,
                                        ptr::null_mut(),
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

                        let top = ml_add_stack(buf);
                        let ip = (*buf).b_ml.ml_stack.cast::<infoptr_T>().add(top as usize);
                        (*ip).ip_bnum = bnum;
                        (*ip).ip_index = idx;

                        bnum = (*pe).pe_bnum;
                        line_count = (*pe).pe_line_count;
                        page_count = (*pe).pe_page_count as c_uint;
                        if page_count < 1
                            || bnum.saturating_add(i64::from(page_count))
                                > (*mfp).mf_blocknr_max.saturating_add(1)
                        {
                            error += 1;
                            ml_append(lnum, tr(c"???ILLEGAL BLOCK NUMBER"), 0, true);
                            lnum += 1;
                            idx = (*ip).ip_index + 1;
                            bnum = (*ip).ip_bnum;
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
                    let dp = (*hp).bh_data.cast::<DataBlock>();
                    if (*dp).db_id != DATA_ID {
                        if bnum == 1 {
                            semsg!(
                                tr(c"E310: Block 1 ID wrong (%s not a .swp file?)"),
                                (*mfp).mf_fname,
                            );
                            ml_recover_theend(
                                fname_used,
                                mfp,
                                hp,
                                buf,
                                serious_error,
                                called_from_main,
                            );
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
                            let idx_ptr = db_idx_ptr(dp, i as usize);
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
                .cast::<infoptr_T>()
                .add((*buf).b_ml.ml_stack_top as usize);
            bnum = (*ip).ip_bnum;
            idx = (*ip).ip_index + 1;
            page_count = 1;
            line_breakcheck();
        }

        if orig_file_status != OK || (*curbuf).b_ml.ml_line_count != lnum * 2 + 1 {
            if !((*curbuf).b_ml.ml_line_count == 2 && *ml_get(1) == NUL) {
                changed_internal(curbuf);
                buf_inc_changedtick(curbuf);
            }
        } else {
            for i_lnum in 1..=lnum {
                let p_line = xstrnsave(ml_get(i_lnum), ml_get_len(i_lnum) as usize);
                let cmp = c_strcmp(p_line, ml_get(i_lnum + lnum));
                xfree(p_line.cast());
                if cmp != 0 {
                    changed_internal(curbuf);
                    buf_inc_changedtick(curbuf);
                    break;
                }
            }
        }

        while (*curbuf).b_ml.ml_line_count > lnum && ((*curbuf).b_ml.ml_flags & ML_EMPTY) == 0 {
            ml_delete((*curbuf).b_ml.ml_line_count);
        }
        (*curbuf).b_flags |= BF_RECOVERED;
        check_cursor(curwin);

        msg_ext_skip_flush = !got_int;
        recoverymode = false;
        if got_int {
            emsg(tr(c"E311: Recovery Interrupted"));
        } else if error != 0 {
            no_wait_return += 1;
            msg_ext_set_kind(c"emsg".as_ptr());
            msg(c">>>>>>>>>>>>>\n".as_ptr(), 0);
            emsg(tr(
                c"E312: Errors detected while recovering; look for lines starting with ???",
            ));
            no_wait_return -= 1;
            msg_putchar(b'\n' as c_int);
            msg(tr(c"See \":help E312\" for more information."), 0);
            msg(c"\n>>>>>>>>>>>>>".as_ptr(), 0);
        } else {
            msg_ext_set_kind(c"wmsg".as_ptr());
            if (*curbuf).b_changed != 0 {
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
            if swapfile_proc_running(b0p, fname_used) != 0 {
                msg_puts(tr(c"\nNote: process STILL RUNNING: "));
                msg_outnum(char_to_long((&raw const (*b0p).b0_pid).cast()) as c_int);
            }
            if !ui_has(kUIMessages) {
                msg_puts(c"\n\n".as_ptr());
            }
            cmdline_row = msg_row;
        }
        redraw_curbuf_later(UPD_NOT_VALID);

        ml_recover_theend(fname_used, mfp, hp, buf, serious_error, called_from_main);
    }
}

unsafe fn ml_recover_theend(
    fname_used: *mut c_char,
    mfp: *mut memfile_T,
    hp: *mut bhdr_T,
    buf: *mut buf_T,
    serious_error: bool,
    called_from_main: bool,
) {
    unsafe {
        msg_ext_skip_flush = false;
        xfree(fname_used.cast());
        recoverymode = false;
        if !mfp.is_null() {
            if !hp.is_null() {
                mf_put(mfp, hp, false, false);
            }
            mf_close(mfp, false);
        }
        if !buf.is_null() {
            xfree((*buf).b_ml.ml_stack);
            xfree(buf.cast());
        }
        if serious_error && called_from_main {
            ml_close(curbuf, 1);
        } else {
            apply_autocmds(
                EVENT_BUFREADPOST,
                ptr::null_mut(),
                (*curbuf).b_fname,
                false,
                curbuf,
            );
            apply_autocmds(
                EVENT_BUFWINENTER,
                ptr::null_mut(),
                (*curbuf).b_fname,
                false,
                curbuf,
            );
        }
    }
}

/// Enumerate swapfiles for `fname`, appending each found path to `ret_list`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn recover_names(
    fname: *mut c_char,
    skip_curbuf: bool,
    ret_list: *mut list_T,
) {
    unsafe {
        let mut names = [ptr::null_mut::<c_char>(); 6];
        let mut files: *mut *mut c_char = ptr::null_mut();
        #[cfg(unix)]
        let mut fname_buf = [0 as c_char; MAXPATHL];
        let mut fname_res: *mut c_char = ptr::null_mut();

        if !fname.is_null() {
            #[cfg(unix)]
            {
                let buf_ptr = (&raw mut fname_buf).cast::<c_char>();
                fname_res = if resolve_symlink(fname, buf_ptr) == OK {
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

        let mut dir_name = NvimString {
            data: xmalloc(c_strlen(p_dir) + 1).cast(),
            size: 0,
        };
        let mut dirp = p_dir;
        while *dirp != NUL {
            dir_name.size = copy_option_part(&raw mut dirp, dir_name.data, 31000, c",".as_ptr());

            let num_names: c_int;
            if *dir_name.data == b'.' as c_char && *dir_name.data.add(1) == NUL {
                if fname.is_null() {
                    names[0] = xmemdupz(c"*.sw?".as_ptr().cast(), 5).cast();
                    names[1] = xmemdupz(c".*.sw?".as_ptr().cast(), 6).cast();
                    names[2] = xmemdupz(c".sw?".as_ptr().cast(), 4).cast();
                    num_names = 3;
                } else {
                    num_names = recov_file_names((&raw mut names).cast(), fname_res, true);
                }
            } else if fname.is_null() {
                names[0] = concat_fnames(
                    dir_name,
                    NvimString {
                        data: c"*.sw?".as_ptr().cast_mut(),
                        size: 5,
                    },
                    true,
                )
                .data;
                names[1] = concat_fnames(
                    dir_name,
                    NvimString {
                        data: c".*.sw?".as_ptr().cast_mut(),
                        size: 6,
                    },
                    true,
                )
                .data;
                names[2] = concat_fnames(
                    dir_name,
                    NvimString {
                        data: c".sw?".as_ptr().cast_mut(),
                        size: 4,
                    },
                    true,
                )
                .data;
                num_names = 3;
            } else {
                let p = dir_name.data.add(dir_name.size);
                let tail = if after_pathsep(dir_name.data, p) != 0
                    && dir_name.size > 1
                    && *p.offset(-1) == *p.offset(-2)
                {
                    make_percent_swname(dir_name.data, p, fname_res)
                } else {
                    let t = path_tail(fname_res);
                    concat_fnames(dir_name, cstr_as_string(t), true).data
                };
                num_names = recov_file_names((&raw mut names).cast(), tail, false);
                xfree(tail.cast());
            }

            let mut num_files: c_int = 0;
            if num_names == 0
                || expand_wildcards(
                    num_names,
                    (&raw mut names).cast(),
                    &raw mut num_files,
                    &raw mut files,
                    EW_KEEPALL | EW_FILE | EW_SILENT,
                ) == FAIL
            {
                num_files = 0;
            }

            if *dirp == NUL && tv_list_len(ret_list) + num_files == 0 && !fname.is_null() {
                let mut swapname = modname(fname_res, c".swp".as_ptr(), true);
                if !swapname.is_null() {
                    if os_path_exists(swapname) {
                        files = xmalloc(size_of::<*mut c_char>()).cast();
                        *files = swapname;
                        swapname = ptr::null_mut();
                        num_files = 1;
                    }
                    xfree(swapname.cast());
                }
            }

            if skip_curbuf && !(*curbuf).b_ml.ml_mfp.is_null() {
                let cur_mfp = (*curbuf).b_ml.ml_mfp.cast::<memfile_T>();
                let p = (*cur_mfp).mf_fname;
                if !p.is_null() {
                    let mut i = 0;
                    while i < num_files {
                        if path_equal(p, *files.add(i as usize), kPathCmpFull) {
                            xfree((*files.add(i as usize)).cast());
                            num_files -= 1;
                            if num_files == 0 {
                                xfree(files.cast());
                                files = ptr::null_mut();
                            } else {
                                for j in i..num_files {
                                    *files.add(j as usize) = *files.add((j + 1) as usize);
                                }
                            }
                        }
                        i += 1;
                    }
                }
            }

            for i in 0..num_files {
                tv_list_append_allocated_string(ret_list, xstrdup(*files.add(i as usize)));
            }

            for name in names.iter().take(num_names as usize) {
                xfree((*name).cast());
            }
            if num_files > 0 {
                FreeWild(num_files, files);
            }
        }
        xfree(dir_name.data.cast());
    }
}

/// Append the full path to `name` with path separators made into percent signs to `dir`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn make_percent_swname(
    dir: *mut c_char,
    dir_end: *mut c_char,
    name: *const c_char,
) -> *mut c_char {
    unsafe {
        let fname = fix_fname(if !name.is_null() { name } else { c"".as_ptr() });
        if fname.is_null() {
            return ptr::null_mut();
        }

        let mut file_info = FileInfo::default();
        if !os_fileinfo2(fname, &raw mut file_info) {
            xfree(fname.cast());
            return ptr::null_mut();
        }
        let mut fixed_data = fname.add(file_info.root_off);
        if file_info.r#type == kPathDeviceUNC {
            assert!(file_info.root_off >= 2);
            fixed_data = fixed_data.sub(2);
            *fixed_data = b'/' as c_char;
        }
        let mut p = fixed_data;
        while *p != NUL {
            if vim_ispathsep(*p as u8 as c_int) {
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
        let d = concat_fnames(
            NvimString {
                data: dir,
                size: last_sep.offset_from(dir) as usize,
            },
            fixed_fname,
            true,
        );
        xfree(fname.cast());
        d.data
    }
}

/// For Vimscript `swapinfo()`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn swapfile_dict(fname: *const c_char, d: *mut dict_T) {
    unsafe {
        let mut b0 = std::mem::zeroed::<ZeroBlock>();
        let fd = os_open(fname, O_RDONLY, 0);
        if fd >= 0 {
            if read_eintr(fd, (&raw mut b0).cast(), size_of::<ZeroBlock>())
                == size_of::<ZeroBlock>() as isize
            {
                if !ml_check_b0_id(&raw const b0) {
                    tv_dict_add_str_len(d, c"error".as_ptr(), 5, c"Not a swap file".as_ptr(), 15);
                } else if b0_magic_wrong(&raw const b0) {
                    tv_dict_add_str_len(
                        d,
                        c"error".as_ptr(),
                        5,
                        c"Magic number mismatch".as_ptr(),
                        21,
                    );
                } else {
                    tv_dict_add_str_len(
                        d,
                        c"version".as_ptr(),
                        7,
                        (&raw const b0.b0_version).cast(),
                        10,
                    );
                    tv_dict_add_str_len(
                        d,
                        c"user".as_ptr(),
                        4,
                        (&raw const b0.b0_uname).cast(),
                        B0_UNAME_SIZE as c_int,
                    );
                    tv_dict_add_str_len(
                        d,
                        c"host".as_ptr(),
                        4,
                        (&raw const b0.b0_hname).cast(),
                        B0_HNAME_SIZE as c_int,
                    );
                    tv_dict_add_str_len(
                        d,
                        c"fname".as_ptr(),
                        5,
                        (&raw const b0.b0_fname).cast(),
                        B0_FNAME_SIZE_ORG as c_int,
                    );

                    tv_dict_add_nr(
                        d,
                        c"pid".as_ptr(),
                        3,
                        i64::from(swapfile_proc_running(&raw const b0, fname)),
                    );
                    tv_dict_add_nr(
                        d,
                        c"mtime".as_ptr(),
                        5,
                        char_to_long((&raw const b0.b0_mtime).cast()) as i64,
                    );
                    tv_dict_add_nr(d, c"dirty".as_ptr(), 5, if b0.dirty() != 0 { 1 } else { 0 });
                    tv_dict_add_nr(
                        d,
                        c"inode".as_ptr(),
                        5,
                        char_to_long((&raw const b0.b0_ino).cast()) as i64,
                    );
                }
            } else {
                tv_dict_add_str_len(d, c"error".as_ptr(), 5, c"Cannot read file".as_ptr(), 16);
            }
            close(fd);
        } else {
            tv_dict_add_str_len(d, c"error".as_ptr(), 5, c"Cannot open file".as_ptr(), 16);
        }
    }
}

unsafe fn swapfile_info(fname: *mut c_char, msg: &mut Vec<u8>) -> time_t {
    unsafe {
        assert!(!fname.is_null());
        let mut b0 = std::mem::zeroed::<ZeroBlock>();
        let mut x: time_t = 0;

        let mut file_info = FileInfo::default();
        if os_fileinfo(fname, &raw mut file_info) {
            #[cfg(unix)]
            {
                let mut uname = [0 as c_char; B0_UNAME_SIZE];
                if os_get_uname(
                    file_info.stat.st_uid as u32,
                    (&raw mut uname).cast(),
                    B0_UNAME_SIZE,
                ) == OK
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
            x = i64::from(file_info.stat.st_mtim.tv_sec);
            let mut ctime_buf = [0 as c_char; 100];
            let ct = os_ctime_r(
                &raw const x,
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
                } else if !ml_check_b0_id(&raw const b0) {
                    sb_push_cstr(msg, tr(c"         [does not look like a Nvim swap file]"));
                } else if !ml_check_b0_strings(&raw const b0) {
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
                        proc_running = swapfile_proc_running(&raw const b0, fname);
                        if proc_running != 0 {
                            sb_push_cstr(msg, tr(c" (STILL RUNNING)"));
                        }
                    }

                    if b0_magic_wrong(&raw const b0) {
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

unsafe fn swapfile_unchanged(fname: *mut c_char) -> bool {
    unsafe {
        if !os_path_exists(fname) {
            return false;
        }
        let fd = os_open(fname, O_RDONLY, 0);
        if fd < 0 {
            return false;
        }
        let mut b0 = std::mem::zeroed::<ZeroBlock>();
        if read_eintr(fd, (&raw mut b0).cast(), size_of::<ZeroBlock>())
            != size_of::<ZeroBlock>() as isize
        {
            close(fd);
            return false;
        }

        let mut ret = true;
        if !ml_check_b0_id(&raw const b0) || b0_magic_wrong(&raw const b0) {
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
            if c_stricmp(
                (&raw const b0.b0_hname).cast(),
                (&raw const hostname).cast(),
            ) != 0
            {
                ret = false;
            }
        }
        if char_to_long((&raw const b0.b0_pid).cast()) == 0
            || swapfile_proc_running(&raw const b0, fname) != 0
        {
            ret = false;
        }
        close(fd);
        ret
    }
}

unsafe fn recov_file_names(names: *mut *mut c_char, path: *mut c_char, prepend_dot: bool) -> c_int {
    unsafe {
        let mut num_names: c_int = 0;
        if prepend_dot {
            *names.add(num_names as usize) = modname(path, c".sw?".as_ptr(), true);
            if (*names.add(num_names as usize)).is_null() {
                return num_names;
            }
            num_names += 1;
        }

        *names.add(num_names as usize) = concat_fnames(
            cstr_as_string(path),
            NvimString {
                data: c".sw?".as_ptr().cast_mut(),
                size: 4,
            },
            false,
        )
        .data;
        if num_names >= 1 {
            let prev = *names.add((num_names - 1) as usize);
            let cur = *names.add(num_names as usize);
            let mut p = prev;
            let i = c_strlen(prev) as isize - c_strlen(cur) as isize;
            if i > 0 {
                p = p.offset(i);
            }
            if c_strcmp(p, cur) != 0 {
                num_names += 1;
            } else {
                xfree(cur.cast());
            }
        } else {
            num_names += 1;
        }

        num_names
    }
}

/// Sync all memlines.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_sync_all(check_file: c_int, check_char: c_int, do_fsync: bool) {
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
            if mfp.is_null() || (*mfp).mf_fname.is_null() {
                buf = (*buf).b_next;
                continue;
            }
            ml_flush_line(buf, false);
            ml_find_line(buf, 0, ML_FLUSH);
            if bufIsChanged(buf)
                && check_file != 0
                && mf_need_trans(mfp)
                && !(*buf).b_ffname.is_null()
            {
                let mut file_info = FileInfo::default();
                if !os_fileinfo((*buf).b_ffname, &raw mut file_info)
                    || i64::from(file_info.stat.st_mtim.tv_sec) != (*buf).b_mtime_read
                    || i64::from(file_info.stat.st_mtim.tv_nsec) != (*buf).b_mtime_read_ns
                    || os_fileinfo_size(&raw const file_info) != (*buf).b_orig_size
                {
                    ml_preserve(buf, false, do_fsync);
                    did_check_timestamps = false;
                    need_check_timestamps = true;
                }
            }
            if (*mfp).mf_dirty == MF_DIRTY_YES {
                mf_sync(
                    mfp,
                    (if check_char != 0 { MFS_STOP } else { 0 })
                        | (if do_fsync && bufIsChanged(buf) {
                            MFS_FLUSH
                        } else {
                            0
                        }),
                );
                if check_char != 0 && os_char_avail() {
                    break;
                }
            }
            buf = (*buf).b_next;
        }
    }
}

/// Sync one buffer, including negative blocks.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_preserve(buf: *mut buf_T, message: bool, do_fsync: bool) {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        let got_int_save = got_int;

        if mfp.is_null() || (*mfp).mf_fname.is_null() {
            if message {
                emsg(tr(c"E313: Cannot preserve, there is no swap file"));
            }
            return;
        }

        got_int = false;

        ml_flush_line(buf, false);
        ml_find_line(buf, 0, ML_FLUSH);
        let mut status = mf_sync(mfp, MFS_ALL | if do_fsync { MFS_FLUSH } else { 0 });

        (*buf).b_ml.ml_stack_top = 0;

        if mf_need_trans(mfp) && !got_int {
            let mut lnum: linenr_T = 1;
            while mf_need_trans(mfp) && lnum <= (*buf).b_ml.ml_line_count {
                let hp = ml_find_line(buf, lnum, ML_FIND);
                if hp.is_null() {
                    status = FAIL;
                    break;
                }
                lnum = (*buf).b_ml.ml_locked_high + 1;
            }
            if status != FAIL {
                ml_find_line(buf, 0, ML_FLUSH);
                if mf_sync(mfp, MFS_ALL | if do_fsync { MFS_FLUSH } else { 0 }) == FAIL {
                    status = FAIL;
                }
                (*buf).b_ml.ml_stack_top = 0;
            }
        }

        got_int |= got_int_save;

        if message {
            if status == OK {
                msg(tr(c"File preserved"), 0);
            } else {
                emsg(tr(c"E314: Preserve failed"));
            }
        }
    }
}

/// Return a pointer to a (read-only copy of a) line in `curbuf`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_get(lnum: linenr_T) -> *mut c_char {
    unsafe { ml_get_buf_impl(curbuf, lnum, false) }
}

/// Return a pointer to a (read-only copy of a) line in `buf`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_get_buf(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char {
    unsafe { ml_get_buf_impl(buf, lnum, false) }
}

/// Like `ml_get_buf`, but allow the line to be mutated in place.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_get_buf_mut(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char {
    unsafe { ml_get_buf_impl(buf, lnum, true) }
}

/// Return pointer to position `pos`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_get_pos(pos: *const pos_T) -> *mut c_char {
    unsafe { ml_get_buf(curbuf, (*pos).lnum).offset((*pos).col as isize) }
}

/// Return length (excluding NUL) of the given line.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_get_len(lnum: linenr_T) -> colnr_T {
    unsafe { ml_get_buf_len(curbuf, lnum) }
}

/// Return length (excluding NUL) of the text after position `pos`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_get_pos_len(pos: *mut pos_T) -> colnr_T {
    unsafe { ml_get_buf_len(curbuf, (*pos).lnum) - (*pos).col }
}

/// Return length (excluding NUL) of the given line in `buf`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_get_buf_len(buf: *mut buf_T, lnum: linenr_T) -> colnr_T {
    unsafe {
        let line = ml_get_buf(buf, lnum);
        if *line == NUL {
            return 0;
        }
        assert!((*buf).b_ml.ml_line_textlen > 0);
        (*buf).b_ml.ml_line_textlen - 1
    }
}

/// Gets the charwise NL-joined text of range `[start, end)` as an allocated `String`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_region_text(buf: *mut buf_T, start: pos_T, end: pos_T) -> NvimString {
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
        let data = xmemdupz(sb.as_ptr().cast(), sb.len()).cast();
        NvimString {
            data,
            size: sb.len(),
        }
    }
}

/// Return codepoint at `pos`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn gchar_pos(pos: *mut pos_T) -> c_int {
    unsafe {
        if (*pos).col == MAXCOL || (*pos).col > ml_get_len((*pos).lnum) {
            return c_int::from(NUL);
        }
        utf_ptr2char(ml_get_pos(pos))
    }
}

unsafe fn ml_get_buf_impl(buf: *mut buf_T, mut lnum: linenr_T, will_change: bool) -> *mut c_char {
    unsafe {
        if (*buf).b_ml.ml_mfp.is_null() {
            (*buf).b_ml.ml_line_textlen = 1;
            return c"".as_ptr().cast_mut();
        }

        if lnum > (*buf).b_ml.ml_line_count {
            if ml_get_recursive == 0 {
                ml_get_recursive += 1;
                siemsg!(tr(e_ml_get_invalid_lnum_nr), i64::from(lnum));
                ml_get_recursive -= 1;
            }
            ml_flush_line(buf, false);
            return ml_get_errorret(buf, lnum);
        }
        lnum = lnum.max(1);

        if (*buf).b_ml.ml_line_lnum != lnum {
            ml_flush_line(buf, false);

            let hp = ml_find_line(buf, lnum, ML_FIND);
            if hp.is_null() {
                if ml_get_recursive == 0 {
                    ml_get_recursive += 1;
                    get_trans_bufname(buf);
                    let namebuff_ptr = (&raw mut NameBuff).cast::<c_char>();
                    shorten_dir(namebuff_ptr);
                    siemsg!(
                        tr(e_ml_get_cannot_find_line_nr_in_buffer_nr_str),
                        i64::from(lnum),
                        (*buf).b_fnum,
                        namebuff_ptr,
                    );
                    ml_get_recursive -= 1;
                }
                return ml_get_errorret(buf, lnum);
            }

            let dp = (*hp).bh_data.cast::<DataBlock>();
            let idx = (lnum - (*buf).b_ml.ml_locked_low) as usize;
            let start = *db_idx_ptr(dp, idx) & DB_INDEX_MASK;
            let end = if idx == 0 {
                (*dp).db_txt_end
            } else {
                *db_idx_ptr(dp, idx - 1) & DB_INDEX_MASK
            };

            (*buf).b_ml.ml_line_ptr = dp.cast::<c_char>().add(start as usize);
            (*buf).b_ml.ml_line_textlen = (end - start) as colnr_T;
            (*buf).b_ml.ml_line_lnum = lnum;
            (*buf).b_ml.ml_flags &= !(ML_LINE_DIRTY | ML_ALLOCATED);
        }
        if will_change {
            (*buf).b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
            ml_add_deleted_len_buf(buf, (*buf).b_ml.ml_line_ptr, -1);
        }

        (*buf).b_ml.ml_line_ptr
    }
}

unsafe fn ml_get_errorret(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char {
    unsafe {
        let q = (&raw mut ml_get_questions).cast::<c_char>();
        *q.add(0) = b'?' as c_char;
        *q.add(1) = b'?' as c_char;
        *q.add(2) = b'?' as c_char;
        *q.add(3) = NUL;
        (*buf).b_ml.ml_line_textlen = 4;
        (*buf).b_ml.ml_line_lnum = lnum;
        q
    }
}

/// Check if a line that was just obtained by `ml_get` is in allocated memory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_line_alloced() -> c_int {
    unsafe { (*curbuf).b_ml.ml_flags & ML_LINE_DIRTY }
}

unsafe fn ml_append_int(
    buf: *mut buf_T,
    lnum: linenr_T,
    line_arg: *mut c_char,
    len_arg: colnr_T,
    flags: c_int,
) -> c_int {
    unsafe {
        let line = line_arg;
        let mut len = len_arg;

        if lnum > (*buf).b_ml.ml_line_count || (*buf).b_ml.ml_mfp.is_null() {
            return FAIL;
        }

        if lowest_marked != 0 && lowest_marked > lnum {
            lowest_marked = lnum + 1;
        }

        if len == 0 {
            len = c_strlen(line) as colnr_T + 1;
        }
        let mut space_needed = i64::from(len) + INDEX_SIZE as i64;

        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        let page_size = i64::from((*mfp).mf_page_size);

        let mut hp = ml_find_line(buf, if lnum == 0 { 1 } else { lnum }, ML_INSERT);
        if hp.is_null() {
            return FAIL;
        }

        (*buf).b_ml.ml_flags &= !ML_EMPTY;

        let mut db_idx: c_int = if lnum == 0 {
            -1
        } else {
            lnum - (*buf).b_ml.ml_locked_low
        };
        let mut line_count = (*buf).b_ml.ml_locked_high - (*buf).b_ml.ml_locked_low;

        let mut dp = (*hp).bh_data.cast::<DataBlock>();

        if i64::from((*dp).db_free) < space_needed
            && db_idx == line_count - 1
            && lnum < (*buf).b_ml.ml_line_count
        {
            (*buf).b_ml.ml_locked_lineadd -= 1;
            (*buf).b_ml.ml_locked_high -= 1;
            hp = ml_find_line(buf, lnum + 1, ML_INSERT);
            if hp.is_null() {
                return FAIL;
            }

            db_idx = -1;
            line_count = (*buf).b_ml.ml_locked_high - (*buf).b_ml.ml_locked_low;
            dp = (*hp).bh_data.cast::<DataBlock>();
        }

        if (*buf).b_prev_line_count == 0 {
            (*buf).b_prev_line_count = (*buf).b_ml.ml_line_count;
        }
        (*buf).b_ml.ml_line_count += 1;

        if i64::from((*dp).db_free) >= space_needed {
            (*dp).db_txt_start -= len as c_uint;
            (*dp).db_free -= space_needed as c_uint;
            (*dp).db_line_count += 1;

            if line_count > db_idx + 1 {
                let offset = if db_idx < 0 {
                    (*dp).db_txt_end as c_int
                } else {
                    (*db_idx_ptr(dp, db_idx as usize) & DB_INDEX_MASK) as c_int
                };
                ptr::copy(
                    dp.cast::<c_char>()
                        .add((*dp).db_txt_start as usize + len as usize),
                    dp.cast::<c_char>().add((*dp).db_txt_start as usize),
                    offset as usize - ((*dp).db_txt_start as usize + len as usize),
                );
                let mut i = line_count - 1;
                while i > db_idx {
                    *db_idx_ptr(dp, (i + 1) as usize) = *db_idx_ptr(dp, i as usize) - len as c_uint;
                    i -= 1;
                }
                *db_idx_ptr(dp, (db_idx + 1) as usize) = (offset - len) as c_uint;
            } else {
                *db_idx_ptr(dp, (db_idx + 1) as usize) = (*dp).db_txt_start;
            }

            let dst_off = *db_idx_ptr(dp, (db_idx + 1) as usize) as usize;
            ptr::copy(line, dp.cast::<c_char>().add(dst_off), len as usize);
            if (flags & ML_APPEND_MARK) != 0 {
                *db_idx_ptr(dp, (db_idx + 1) as usize) |= DB_MARKED;
            }

            (*buf).b_ml.ml_flags |= ML_LOCKED_DIRTY;
            if (flags & ML_APPEND_NEW) == 0 {
                (*buf).b_ml.ml_flags |= ML_LOCKED_POS;
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
                    data_moved = ((*db_idx_ptr(dp, db_idx as usize) & DB_INDEX_MASK)
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
            let mut hp_new = ml_new_data(mfp, (flags & ML_APPEND_NEW) != 0, new_page_count);
            let hp_left: *mut bhdr_T;
            let hp_right: *mut bhdr_T;
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
            let dp_right = (*hp_right).bh_data.cast::<DataBlock>();
            let dp_left = (*hp_left).bh_data.cast::<DataBlock>();
            let mut bnum_left = (*hp_left).bh_bnum;
            let mut bnum_right = (*hp_right).bh_bnum;
            let mut page_count_left = (*hp_left).bh_page_count as c_int;
            let mut page_count_right = (*hp_right).bh_page_count as c_int;

            if !in_left {
                (*dp_right).db_txt_start -= len as c_uint;
                (*dp_right).db_free -= len as c_uint + INDEX_SIZE as c_uint;
                *db_idx_ptr(dp_right, 0) = (*dp_right).db_txt_start;
                if (flags & ML_APPEND_MARK) != 0 {
                    *db_idx_ptr(dp_right, 0) |= DB_MARKED;
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
                    *db_idx_ptr(dp_right, to as usize) =
                        (*db_idx_ptr(dp, from as usize)).wrapping_add(offset as c_uint);
                    from += 1;
                    to += 1;
                }
                line_count_right += lines_moved;
                line_count_left -= lines_moved;
            }

            if in_left {
                (*dp_left).db_txt_start -= len as c_uint;
                (*dp_left).db_free -= len as c_uint + INDEX_SIZE as c_uint;
                *db_idx_ptr(dp_left, line_count_left as usize) = (*dp_left).db_txt_start;
                if (flags & ML_APPEND_MARK) != 0 {
                    *db_idx_ptr(dp_left, line_count_left as usize) |= DB_MARKED;
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

            let mut lnum_left: linenr_T;
            let mut lnum_right: linenr_T;
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
                (*buf).b_ml.ml_flags |= ML_LOCKED_DIRTY;
            }
            if (flags & ML_APPEND_NEW) == 0 && db_idx >= 0 && in_left {
                (*buf).b_ml.ml_flags |= ML_LOCKED_POS;
            }
            mf_put(mfp, hp_new, true, false);

            let lineadd = (*buf).b_ml.ml_locked_lineadd;
            (*buf).b_ml.ml_locked_lineadd = 0;
            ml_find_line(buf, 0, ML_FLUSH);

            let mut stack_idx = (*buf).b_ml.ml_stack_top - 1;
            while stack_idx >= 0 {
                let ip = (*buf)
                    .b_ml
                    .ml_stack
                    .cast::<infoptr_T>()
                    .add(stack_idx as usize);
                let pb_idx = (*ip).ip_index as usize;
                hp = mf_get(mfp, (*ip).ip_bnum, 1);
                if hp.is_null() {
                    return FAIL;
                }
                let mut pp = (*hp).bh_data.cast::<PointerBlock>();
                if (*pp).pb_id != PTR_ID {
                    iemsg(tr(e_pointer_block_id_wrong_three));
                    mf_put(mfp, hp, false, false);
                    return FAIL;
                }
                if (*pp).pb_count < (*pp).pb_count_max {
                    if pb_idx + 1 < (*pp).pb_count as usize {
                        ptr::copy(
                            pb_ptr(pp, pb_idx + 1),
                            pb_ptr(pp, pb_idx + 2),
                            (*pp).pb_count as usize - pb_idx - 1,
                        );
                    }
                    (*pp).pb_count += 1;
                    let pe_cur = pb_ptr(pp, pb_idx);
                    (*pe_cur).pe_line_count = line_count_left;
                    (*pe_cur).pe_bnum = bnum_left;
                    (*pe_cur).pe_page_count = page_count_left;
                    let pe_next = pb_ptr(pp, pb_idx + 1);
                    (*pe_next).pe_line_count = line_count_right;
                    (*pe_next).pe_bnum = bnum_right;
                    (*pe_next).pe_page_count = page_count_right;

                    if lnum_left != 0 {
                        (*pe_cur).pe_old_lnum = lnum_left;
                    }
                    if lnum_right != 0 {
                        (*pe_next).pe_old_lnum = lnum_right;
                    }

                    mf_put(mfp, hp, true, false);
                    (*buf).b_ml.ml_stack_top = stack_idx + 1;

                    if lineadd != 0 {
                        (*buf).b_ml.ml_stack_top -= 1;
                        ml_lineadd(buf, lineadd);
                        let top_ip = (*buf)
                            .b_ml
                            .ml_stack
                            .cast::<infoptr_T>()
                            .add((*buf).b_ml.ml_stack_top as usize);
                        (*top_ip).ip_high += lineadd;
                        (*buf).b_ml.ml_stack_top += 1;
                    }

                    break;
                }

                let mut pp_new: *mut PointerBlock;
                loop {
                    hp_new = ml_new_ptr(mfp);
                    if hp_new.is_null() {
                        return FAIL;
                    }
                    pp_new = (*hp_new).bh_data.cast::<PointerBlock>();

                    if (*hp).bh_bnum != 1 {
                        break;
                    }

                    ptr::copy(pp.cast::<u8>(), pp_new.cast::<u8>(), page_size as usize);
                    (*pp).pb_count = 1;
                    let pe0 = pb_ptr(pp, 0);
                    (*pe0).pe_bnum = (*hp_new).bh_bnum;
                    (*pe0).pe_line_count = (*buf).b_ml.ml_line_count;
                    (*pe0).pe_old_lnum = 1;
                    (*pe0).pe_page_count = 1;
                    mf_put(mfp, hp, true, false);
                    hp = hp_new;
                    pp = pp_new;
                    (*ip).ip_index = 0;
                    stack_idx += 1;
                }

                total_moved = c_int::from((*pp).pb_count) - (pb_idx as c_int) - 1;
                if total_moved != 0 {
                    ptr::copy(
                        pb_ptr(pp, pb_idx + 1),
                        pb_ptr(pp_new, 0),
                        total_moved as usize,
                    );
                    (*pp_new).pb_count = total_moved as u16;
                    (*pp).pb_count = (c_int::from((*pp).pb_count) - (total_moved - 1)) as u16;
                    let pe_next = pb_ptr(pp, pb_idx + 1);
                    (*pe_next).pe_bnum = bnum_right;
                    (*pe_next).pe_line_count = line_count_right;
                    (*pe_next).pe_page_count = page_count_right;
                    if lnum_right != 0 {
                        (*pe_next).pe_old_lnum = lnum_right;
                    }
                } else {
                    (*pp_new).pb_count = 1;
                    let pe_new0 = pb_ptr(pp_new, 0);
                    (*pe_new0).pe_bnum = bnum_right;
                    (*pe_new0).pe_line_count = line_count_right;
                    (*pe_new0).pe_page_count = page_count_right;
                    (*pe_new0).pe_old_lnum = lnum_right;
                }
                let pe_cur = pb_ptr(pp, pb_idx);
                (*pe_cur).pe_bnum = bnum_left;
                (*pe_cur).pe_line_count = line_count_left;
                (*pe_cur).pe_page_count = page_count_left;
                if lnum_left != 0 {
                    (*pe_cur).pe_old_lnum = lnum_left;
                }
                lnum_left = 0;
                lnum_right = 0;

                line_count_right = 0;
                for i in 0..((*pp_new).pb_count as usize) {
                    line_count_right += (*pb_ptr(pp_new, i)).pe_line_count;
                }
                line_count_left = 0;
                for i in 0..((*pp).pb_count as usize) {
                    line_count_left += (*pb_ptr(pp, i)).pe_line_count;
                }

                bnum_left = (*hp).bh_bnum;
                bnum_right = (*hp_new).bh_bnum;
                page_count_left = 1;
                page_count_right = 1;
                mf_put(mfp, hp, true, false);
                mf_put(mfp, hp_new, true, false);

                stack_idx -= 1;
            }

            if stack_idx < 0 {
                iemsg(tr(c"E318: Updated too many blocks?"));
                (*buf).b_ml.ml_stack_top = 0;
            }
        }

        ml_updatechunk(buf, lnum + 1, len, ML_CHNK_ADDLINE);
        OK
    }
}

unsafe fn ml_append_flush(
    buf: *mut buf_T,
    lnum: linenr_T,
    line: *mut c_char,
    len: colnr_T,
    flags: c_int,
) -> c_int {
    unsafe {
        if lnum > (*buf).b_ml.ml_line_count {
            return FAIL;
        }
        if (*buf).b_ml.ml_line_lnum != 0 {
            ml_flush_line(buf, false);
        }
        ml_append_int(buf, lnum, line, len, flags)
    }
}

/// Append a line after `lnum` (may be 0 to insert in front of the file).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_append(
    lnum: linenr_T,
    line: *mut c_char,
    len: colnr_T,
    newfile: bool,
) -> c_int {
    unsafe { ml_append_flags(lnum, line, len, if newfile { ML_APPEND_NEW } else { 0 }) }
}

/// Append a line after `lnum` using `ML_APPEND_` flags.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_append_flags(
    lnum: linenr_T,
    line: *mut c_char,
    len: colnr_T,
    flags: c_int,
) -> c_int {
    unsafe {
        if (*curbuf).b_ml.ml_mfp.is_null() && open_buffer(false, ptr::null_mut(), 0) == FAIL {
            return FAIL;
        }
        ml_append_flush(curbuf, lnum, line, len, flags)
    }
}

/// Like `ml_append()` but for an arbitrary buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_append_buf(
    buf: *mut buf_T,
    lnum: linenr_T,
    line: *mut c_char,
    len: colnr_T,
    newfile: bool,
) -> c_int {
    unsafe {
        if (*buf).b_ml.ml_mfp.is_null() {
            return FAIL;
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_add_deleted_len(ptr: *mut c_char, len: isize) {
    unsafe {
        ml_add_deleted_len_buf(curbuf, ptr, len);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_add_deleted_len_buf(buf: *mut buf_T, ptr: *mut c_char, mut len: isize) {
    unsafe {
        if inhibit_delete_count != 0 {
            return;
        }
        let maxlen = c_strlen(ptr) as isize;
        if len == -1 || len > maxlen {
            len = maxlen;
        }
        (*buf).deleted_bytes += (len as usize) + 1;
        (*buf).deleted_bytes2 += (len as usize) + 1;
        if (*buf).update_need_codepoints {
            mb_utflen(
                ptr,
                len as usize,
                &raw mut (*buf).deleted_codepoints,
                &raw mut (*buf).deleted_codeunits,
            );
            (*buf).deleted_codepoints += 1;
            (*buf).deleted_codeunits += 1;
        }
    }
}

/// Replace line `lnum`, with buffering, in current buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_replace(lnum: linenr_T, line: *mut c_char, copy: bool) -> c_int {
    unsafe { ml_replace_buf(curbuf, lnum, line, copy, false) }
}

/// Replace a line for the current buffer with explicit `len` (excluding NUL).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_replace_len(
    lnum: linenr_T,
    line: *mut c_char,
    len: usize,
    copy: bool,
) -> c_int {
    unsafe { ml_replace_buf_len(curbuf, lnum, line, len, copy, false) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_replace_buf(
    buf: *mut buf_T,
    lnum: linenr_T,
    line: *mut c_char,
    copy: bool,
    noalloc: bool,
) -> c_int {
    unsafe {
        let len = if !line.is_null() {
            c_strlen(line)
        } else {
            usize::MAX
        };
        ml_replace_buf_len(buf, lnum, line, len, copy, noalloc)
    }
}

/// Replace line `lnum`, with buffering.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_replace_buf_len(
    buf: *mut buf_T,
    lnum: linenr_T,
    line_arg: *mut c_char,
    len_arg: usize,
    copy: bool,
    noalloc: bool,
) -> c_int {
    unsafe {
        let mut line = line_arg;
        if line.is_null() {
            return FAIL;
        }

        if (*buf).b_ml.ml_mfp.is_null() && open_buffer(false, ptr::null_mut(), 0) == FAIL {
            return FAIL;
        }

        if copy {
            assert!(!noalloc);
            line = xmemdupz(line.cast(), len_arg).cast();
        }

        if (*buf).b_ml.ml_line_lnum != lnum {
            ml_flush_line(buf, false);
        }

        if (*buf).update_callbacks.size != 0 {
            let old_line = ml_get_buf(buf, lnum);
            ml_add_deleted_len_buf(buf, old_line, -1);
        }

        if ((*buf).b_ml.ml_flags & (ML_LINE_DIRTY | ML_ALLOCATED)) != 0 {
            xfree((*buf).b_ml.ml_line_ptr.cast());
        }

        (*buf).b_ml.ml_line_ptr = line;
        (*buf).b_ml.ml_line_textlen = (len_arg as colnr_T) + 1;
        (*buf).b_ml.ml_line_lnum = lnum;
        (*buf).b_ml.ml_flags = ((*buf).b_ml.ml_flags | ML_LINE_DIRTY) & !ML_EMPTY;
        if noalloc {
            ml_flush_line(buf, true);
        }

        OK
    }
}

/// Delete line `lnum` in `buf`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_delete_buf(buf: *mut buf_T, lnum: linenr_T, message: bool) -> c_int {
    unsafe {
        ml_flush_line(buf, false);
        ml_delete_int(buf, lnum, if message { ML_DEL_MESSAGE } else { 0 })
    }
}

unsafe fn ml_delete_int(buf: *mut buf_T, lnum: linenr_T, flags: c_int) -> c_int {
    unsafe {
        if lowest_marked != 0 && lowest_marked > lnum {
            lowest_marked -= 1;
        }

        if (*buf).b_ml.ml_line_count == 1 {
            if (flags & ML_DEL_MESSAGE) != 0 {
                set_keep_msg(gettext(no_lines_msg.as_ptr()), 0);
            }
            let i = ml_replace_buf(buf, 1, c"".as_ptr().cast_mut(), true, false);
            (*buf).b_ml.ml_flags |= ML_EMPTY;
            return i;
        }

        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if mfp.is_null() {
            return FAIL;
        }

        let mut hp = ml_find_line(buf, lnum, ML_DELETE);
        if hp.is_null() {
            return FAIL;
        }

        let dp = (*hp).bh_data.cast::<DataBlock>();
        let mut count = (*buf).b_ml.ml_locked_high - (*buf).b_ml.ml_locked_low + 2;
        let mut idx = lnum - (*buf).b_ml.ml_locked_low;

        if (*buf).b_prev_line_count == 0 {
            (*buf).b_prev_line_count = (*buf).b_ml.ml_line_count;
        }
        (*buf).b_ml.ml_line_count -= 1;

        let line_start = (*db_idx_ptr(dp, idx as usize) & DB_INDEX_MASK) as c_int;
        let line_size: c_int = if idx == 0 {
            ((*dp).db_txt_end as c_int) - line_start
        } else {
            ((*db_idx_ptr(dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int) - line_start
        };

        assert!(line_size >= 1);
        ml_add_deleted_len_buf(
            buf,
            dp.cast::<c_char>().add(line_start as usize),
            (line_size - 1) as isize,
        );

        if count == 1 {
            mf_free(mfp, hp);
            (*buf).b_ml.ml_locked = ptr::null_mut();

            let mut stack_idx = (*buf).b_ml.ml_stack_top - 1;
            while stack_idx >= 0 {
                (*buf).b_ml.ml_stack_top = 0;
                let ip = (*buf)
                    .b_ml
                    .ml_stack
                    .cast::<infoptr_T>()
                    .add(stack_idx as usize);
                idx = (*ip).ip_index;
                hp = mf_get(mfp, (*ip).ip_bnum, 1);
                if hp.is_null() {
                    return FAIL;
                }
                let pp = (*hp).bh_data.cast::<PointerBlock>();
                if (*pp).pb_id != PTR_ID {
                    iemsg(tr(e_pointer_block_id_wrong_four));
                    mf_put(mfp, hp, false, false);
                    return FAIL;
                }
                (*pp).pb_count -= 1;
                count = c_int::from((*pp).pb_count);
                if count == 0 {
                    mf_free(mfp, hp);
                } else {
                    if count != idx {
                        ptr::copy(
                            pb_ptr(pp, (idx + 1) as usize),
                            pb_ptr(pp, idx as usize),
                            (count - idx) as usize,
                        );
                    }
                    mf_put(mfp, hp, true, false);

                    (*buf).b_ml.ml_stack_top = stack_idx;
                    if (*buf).b_ml.ml_locked_lineadd != 0 {
                        ml_lineadd(buf, (*buf).b_ml.ml_locked_lineadd);
                        let top_ip = (*buf)
                            .b_ml
                            .ml_stack
                            .cast::<infoptr_T>()
                            .add((*buf).b_ml.ml_stack_top as usize);
                        (*top_ip).ip_high += (*buf).b_ml.ml_locked_lineadd;
                    }
                    (*buf).b_ml.ml_stack_top += 1;
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
                *db_idx_ptr(dp, i as usize) =
                    *db_idx_ptr(dp, (i + 1) as usize) + (line_size as c_uint);
            }

            (*dp).db_free += (line_size as c_uint) + (INDEX_SIZE as c_uint);
            (*dp).db_txt_start += line_size as c_uint;
            (*dp).db_line_count -= 1;

            (*buf).b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
        }

        ml_updatechunk(buf, lnum, line_size, ML_CHNK_DELLINE);
        OK
    }
}

/// Delete line `lnum` in the current buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_delete(lnum: linenr_T) -> c_int {
    unsafe { ml_delete_flags(lnum, 0) }
}

/// Like `ml_delete()` but using flags.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_delete_flags(lnum: linenr_T, flags: c_int) -> c_int {
    unsafe {
        ml_flush_line(curbuf, false);
        if lnum < 1 || lnum > (*curbuf).b_ml.ml_line_count {
            return FAIL;
        }
        ml_delete_int(curbuf, lnum, flags)
    }
}

/// Set the `DB_MARKED` flag for line `lnum`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_setmarked(lnum: linenr_T) {
    unsafe {
        if lnum < 1 || lnum > (*curbuf).b_ml.ml_line_count || (*curbuf).b_ml.ml_mfp.is_null() {
            return;
        }
        if lowest_marked == 0 || lowest_marked > lnum {
            lowest_marked = lnum;
        }

        let hp = ml_find_line(curbuf, lnum, ML_FIND);
        if hp.is_null() {
            return;
        }
        let dp = (*hp).bh_data.cast::<DataBlock>();
        *db_idx_ptr(dp, (lnum - (*curbuf).b_ml.ml_locked_low) as usize) |= DB_MARKED;
        (*curbuf).b_ml.ml_flags |= ML_LOCKED_DIRTY;
    }
}

/// Find the first line with its `DB_MARKED` flag set.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_firstmarked() -> linenr_T {
    unsafe {
        if (*curbuf).b_ml.ml_mfp.is_null() {
            return 0;
        }

        let mut lnum = lowest_marked;
        while lnum <= (*curbuf).b_ml.ml_line_count {
            let hp = ml_find_line(curbuf, lnum, ML_FIND);
            if hp.is_null() {
                return 0;
            }
            let dp = (*hp).bh_data.cast::<DataBlock>();
            let mut i = (lnum - (*curbuf).b_ml.ml_locked_low) as usize;
            while lnum <= (*curbuf).b_ml.ml_locked_high {
                let idx_p = db_idx_ptr(dp, i);
                if (*idx_p & DB_MARKED) != 0 {
                    *idx_p &= DB_INDEX_MASK;
                    (*curbuf).b_ml.ml_flags |= ML_LOCKED_DIRTY;
                    lowest_marked = lnum + 1;
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_clearmarked() {
    unsafe {
        if (*curbuf).b_ml.ml_mfp.is_null() {
            return;
        }

        let mut lnum = lowest_marked;
        while lnum <= (*curbuf).b_ml.ml_line_count {
            let hp = ml_find_line(curbuf, lnum, ML_FIND);
            if hp.is_null() {
                return;
            }
            let dp = (*hp).bh_data.cast::<DataBlock>();
            let mut i = (lnum - (*curbuf).b_ml.ml_locked_low) as usize;
            while lnum <= (*curbuf).b_ml.ml_locked_high {
                let idx_p = db_idx_ptr(dp, i);
                if (*idx_p & DB_MARKED) != 0 {
                    *idx_p &= DB_INDEX_MASK;
                    (*curbuf).b_ml.ml_flags |= ML_LOCKED_DIRTY;
                }
                i += 1;
                lnum += 1;
            }
        }

        lowest_marked = 0;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_flush_deleted_bytes(
    buf: *mut buf_T,
    codepoints: *mut usize,
    codeunits: *mut usize,
) -> usize {
    unsafe {
        let ret = (*buf).deleted_bytes;
        *codepoints = (*buf).deleted_codepoints;
        *codeunits = (*buf).deleted_codeunits;
        (*buf).deleted_bytes = 0;
        (*buf).deleted_codepoints = 0;
        (*buf).deleted_codeunits = 0;
        ret
    }
}

unsafe fn ml_flush_line(buf: *mut buf_T, noalloc: bool) {
    unsafe {
        if (*buf).b_ml.ml_line_lnum == 0 || (*buf).b_ml.ml_mfp.is_null() {
            return;
        }
        if ((*buf).b_ml.ml_flags & ML_LINE_DIRTY) != 0 {
            if ml_flush_entered {
                return;
            }
            ml_flush_entered = true;

            (*buf).flush_count += 1;

            let lnum = (*buf).b_ml.ml_line_lnum;
            let new_line = (*buf).b_ml.ml_line_ptr;

            let hp = ml_find_line(buf, lnum, ML_FIND);
            if hp.is_null() {
                siemsg!(tr(c"E320: Cannot find line %lld"), i64::from(lnum));
            } else {
                let dp = (*hp).bh_data.cast::<DataBlock>();
                let idx = (lnum - (*buf).b_ml.ml_locked_low) as usize;
                let start = (*db_idx_ptr(dp, idx) & DB_INDEX_MASK) as c_int;
                let old_line = dp.cast::<c_char>().add(start as usize);
                let old_len: c_int = if idx == 0 {
                    ((*dp).db_txt_end as c_int) - start
                } else {
                    ((*db_idx_ptr(dp, idx - 1) & DB_INDEX_MASK) as c_int) - start
                };
                let new_len = (*buf).b_ml.ml_line_textlen;
                let extra = new_len - old_len;

                if ((*dp).db_free as c_int) >= extra {
                    let count =
                        ((*buf).b_ml.ml_locked_high - (*buf).b_ml.ml_locked_low + 1) as usize;
                    if extra != 0 && idx < count - 1 {
                        let txt_start = (*dp).db_txt_start as isize;
                        ptr::copy(
                            dp.cast::<c_char>().offset(txt_start),
                            dp.cast::<c_char>().offset(txt_start - extra as isize),
                            (start - (*dp).db_txt_start as c_int) as usize,
                        );
                        for i in (idx + 1)..count {
                            *db_idx_ptr(dp, i) = (*db_idx_ptr(dp, i)).wrapping_sub(extra as c_uint);
                        }
                    }
                    *db_idx_ptr(dp, idx) = (*db_idx_ptr(dp, idx)).wrapping_sub(extra as c_uint);

                    (*dp).db_free = (*dp).db_free.wrapping_sub(extra as c_uint);
                    (*dp).db_txt_start = (*dp).db_txt_start.wrapping_sub(extra as c_uint);

                    ptr::copy(
                        new_line,
                        old_line.offset(-(extra as isize)),
                        new_len as usize,
                    );
                    (*buf).b_ml.ml_flags |= ML_LOCKED_DIRTY | ML_LOCKED_POS;
                    if extra != 0 {
                        ml_updatechunk(buf, lnum, extra, ML_CHNK_UPDLINE);
                    }
                } else {
                    let mark_flag = if (*db_idx_ptr(dp, idx) & DB_MARKED) != 0 {
                        ML_APPEND_MARK
                    } else {
                        0
                    };
                    let _ = ml_append_int(buf, lnum, new_line, new_len, mark_flag);
                    let _ = ml_delete_int(buf, lnum, 0);
                }
            }
            if !noalloc {
                xfree(new_line.cast());
            }

            ml_flush_entered = false;
        } else if ((*buf).b_ml.ml_flags & ML_ALLOCATED) != 0 {
            assert!(!noalloc);
            xfree((*buf).b_ml.ml_line_ptr.cast());
        }

        (*buf).b_ml.ml_flags &= !(ML_LINE_DIRTY | ML_ALLOCATED);
        (*buf).b_ml.ml_line_lnum = 0;
        (*buf).b_ml.ml_line_offset = 0;
    }
}

unsafe fn ml_new_data(mfp: *mut memfile_T, negative: bool, page_count: i64) -> *mut bhdr_T {
    unsafe {
        assert!(page_count >= 0);
        let hp = mf_new(mfp, negative, page_count as c_uint);
        let dp = (*hp).bh_data.cast::<DataBlock>();
        (*dp).db_id = DATA_ID;
        (*dp).db_txt_end = (page_count as c_uint).wrapping_mul((*mfp).mf_page_size);
        (*dp).db_txt_start = (*dp).db_txt_end;
        (*dp).db_free = (*dp).db_txt_start - (HEADER_SIZE as c_uint);
        (*dp).db_line_count = 0;
        hp
    }
}

unsafe fn ml_new_ptr(mfp: *mut memfile_T) -> *mut bhdr_T {
    unsafe {
        let hp = mf_new(mfp, false, 1);
        let pp = (*hp).bh_data.cast::<PointerBlock>();
        (*pp).pb_id = PTR_ID;
        (*pp).pb_count = 0;
        (*pp).pb_count_max = pb_count_max(mfp);
        hp
    }
}

unsafe fn ml_find_line(buf: *mut buf_T, lnum: linenr_T, action: c_int) -> *mut bhdr_T {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();

        if !(*buf).b_ml.ml_locked.is_null() {
            if ml_simple(action)
                && (*buf).b_ml.ml_locked_low <= lnum
                && (*buf).b_ml.ml_locked_high >= lnum
            {
                if action == ML_INSERT {
                    (*buf).b_ml.ml_locked_lineadd += 1;
                    (*buf).b_ml.ml_locked_high += 1;
                } else if action == ML_DELETE {
                    (*buf).b_ml.ml_locked_lineadd -= 1;
                    (*buf).b_ml.ml_locked_high -= 1;
                }
                return (*buf).b_ml.ml_locked.cast();
            }

            mf_put(
                mfp,
                (*buf).b_ml.ml_locked.cast(),
                ((*buf).b_ml.ml_flags & ML_LOCKED_DIRTY) != 0,
                ((*buf).b_ml.ml_flags & ML_LOCKED_POS) != 0,
            );
            (*buf).b_ml.ml_locked = ptr::null_mut();

            if (*buf).b_ml.ml_locked_lineadd != 0 {
                ml_lineadd(buf, (*buf).b_ml.ml_locked_lineadd);
            }
        }

        if action == ML_FLUSH {
            return ptr::null_mut();
        }

        let mut bnum: blocknr_T = 1;
        let mut page_count: c_int = 1;
        let mut low: linenr_T = 1;
        let mut high: linenr_T = (*buf).b_ml.ml_line_count;

        if action == ML_FIND {
            let mut top = (*buf).b_ml.ml_stack_top - 1;
            while top >= 0 {
                let ip = (*buf).b_ml.ml_stack.cast::<infoptr_T>().add(top as usize);
                if (*ip).ip_low <= lnum && (*ip).ip_high >= lnum {
                    bnum = (*ip).ip_bnum;
                    low = (*ip).ip_low;
                    high = (*ip).ip_high;
                    (*buf).b_ml.ml_stack_top = top;
                    break;
                }
                top -= 1;
            }
            if top < 0 {
                (*buf).b_ml.ml_stack_top = 0;
            }
        } else {
            (*buf).b_ml.ml_stack_top = 0;
        }

        loop {
            let hp = mf_get(mfp, bnum, page_count as c_uint);
            if hp.is_null() {
                return ml_find_line_error_noblock(buf, action);
            }

            if action == ML_INSERT {
                high += 1;
            } else if action == ML_DELETE {
                high -= 1;
            }

            let dp = (*hp).bh_data.cast::<DataBlock>();
            if (*dp).db_id == DATA_ID {
                (*buf).b_ml.ml_locked = hp.cast();
                (*buf).b_ml.ml_locked_low = low;
                (*buf).b_ml.ml_locked_high = high;
                (*buf).b_ml.ml_locked_lineadd = 0;
                (*buf).b_ml.ml_flags &= !(ML_LOCKED_DIRTY | ML_LOCKED_POS);
                return hp;
            }

            let pp = dp.cast::<PointerBlock>();
            if (*pp).pb_id != PTR_ID {
                iemsg(tr(e_pointer_block_id_wrong));
                mf_put(mfp, hp, false, false);
                return ml_find_line_error_noblock(buf, action);
            }

            let top = ml_add_stack(buf);
            let ip = (*buf).b_ml.ml_stack.cast::<infoptr_T>().add(top as usize);
            (*ip).ip_bnum = bnum;
            (*ip).ip_low = low;
            (*ip).ip_high = high;
            (*ip).ip_index = -1;

            let mut dirty = false;
            let mut idx: c_int = 0;
            while idx < c_int::from((*pp).pb_count) {
                let pe = pb_ptr(pp, idx as usize);
                let t = (*pe).pe_line_count;
                low += t;
                if low > lnum {
                    (*ip).ip_index = idx;
                    bnum = (*pe).pe_bnum;
                    page_count = (*pe).pe_page_count;
                    high = low - 1;
                    low -= t;

                    if bnum < 0 {
                        let bnum2 = mf_trans_del(mfp, bnum);
                        if bnum != bnum2 {
                            bnum = bnum2;
                            (*pe).pe_bnum = bnum;
                            dirty = true;
                        }
                    }
                    break;
                }
                idx += 1;
            }
            if idx >= c_int::from((*pp).pb_count) {
                if lnum > (*buf).b_ml.ml_line_count {
                    siemsg!(
                        tr(e_line_number_out_of_range_nr_past_the_end),
                        i64::from(lnum - (*buf).b_ml.ml_line_count),
                    );
                } else {
                    siemsg!(tr(e_line_count_wrong_in_block_nr), bnum);
                }
                mf_put(mfp, hp, false, false);
                return ml_find_line_error_noblock(buf, action);
            }
            if action == ML_DELETE {
                (*pb_ptr(pp, idx as usize)).pe_line_count -= 1;
                dirty = true;
            } else if action == ML_INSERT {
                (*pb_ptr(pp, idx as usize)).pe_line_count += 1;
                dirty = true;
            }
            mf_put(mfp, hp, dirty, false);
        }
    }
}

unsafe fn ml_find_line_error_noblock(buf: *mut buf_T, action: c_int) -> *mut bhdr_T {
    unsafe {
        if action == ML_DELETE {
            ml_lineadd(buf, 1);
        } else if action == ML_INSERT {
            ml_lineadd(buf, -1);
        }
        (*buf).b_ml.ml_stack_top = 0;
        ptr::null_mut()
    }
}

unsafe fn ml_add_stack(buf: *mut buf_T) -> c_int {
    unsafe {
        let top = (*buf).b_ml.ml_stack_top;
        if top == (*buf).b_ml.ml_stack_size {
            (*buf).b_ml.ml_stack_size += STACK_INCR;
            let new_size = size_of::<infoptr_T>() * ((*buf).b_ml.ml_stack_size as usize);
            (*buf).b_ml.ml_stack = xrealloc((*buf).b_ml.ml_stack, new_size);
        }
        (*buf).b_ml.ml_stack_top += 1;
        top
    }
}

unsafe fn ml_lineadd(buf: *mut buf_T, count: c_int) {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        let mut idx = (*buf).b_ml.ml_stack_top - 1;
        while idx >= 0 {
            let ip = (*buf).b_ml.ml_stack.cast::<infoptr_T>().add(idx as usize);
            let hp = mf_get(mfp, (*ip).ip_bnum, 1);
            if hp.is_null() {
                break;
            }
            let pp = (*hp).bh_data.cast::<PointerBlock>();
            if (*pp).pb_id != PTR_ID {
                mf_put(mfp, hp, false, false);
                iemsg(tr(e_pointer_block_id_wrong_two));
                break;
            }
            (*pb_ptr(pp, (*ip).ip_index as usize)).pe_line_count += count;
            (*ip).ip_high += count;
            mf_put(mfp, hp, true, false);
            idx -= 1;
        }
    }
}

/// Resolve a symlink in the last component of a file name.
#[cfg(unix)]
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn resolve_symlink(fname: *const c_char, buf: *mut c_char) -> c_int {
    unsafe {
        let mut tmp = [0 as c_char; MAXPATHL];
        let mut depth: c_int = 0;

        if fname.is_null() {
            return FAIL;
        }

        let tmp_ptr = (&raw mut tmp).cast::<c_char>();
        xstrlcpy(tmp_ptr, fname, MAXPATHL);

        loop {
            depth += 1;
            if depth == 100 {
                semsg!(tr(c"E773: Symlink loop for \"%s\""), fname);
                return FAIL;
            }

            let ret = readlink(tmp_ptr, buf, MAXPATHL - 1);
            if ret <= 0 {
                let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if err == EINVAL || err == ENOENT {
                    if depth == 1 {
                        return FAIL;
                    }
                    break;
                }
                return FAIL;
            }
            *buf.add(ret as usize) = NUL;

            if path_is_absolute(buf) {
                xstrlcpy(tmp_ptr, buf, MAXPATHL);
            } else {
                let tail = path_tail(tmp_ptr);
                if c_strlen(tail) + c_strlen(buf) >= MAXPATHL {
                    return FAIL;
                }
                let rem = MAXPATHL - tail.offset_from(tmp_ptr) as usize;
                xstrlcpy(tail, buf, rem);
            }
        }

        vim_FullName(tmp_ptr, buf, MAXPATHL, true)
    }
}

/// Make swapfile name out of the file name and a directory name.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn makeswapname(
    fname: *mut c_char,
    _ffname: *mut c_char,
    _buf: *mut buf_T,
    dir_name: *mut c_char,
) -> *mut c_char {
    unsafe {
        let mut fname_res = fname;
        #[cfg(unix)]
        let mut fname_buf = [0 as c_char; MAXPATHL];
        #[cfg(unix)]
        {
            let buf_ptr = (&raw mut fname_buf).cast::<c_char>();
            if resolve_symlink(fname, buf_ptr) == OK {
                fname_res = buf_ptr;
            }
        }

        let len = c_strlen(dir_name);
        let s = dir_name.add(len);
        if after_pathsep(dir_name, s) != 0 && len > 1 && *s.offset(-1) == *s.offset(-2) {
            let mut r: *mut c_char = ptr::null_mut();
            let sw = make_percent_swname(dir_name, s, fname_res);
            if !sw.is_null() {
                r = modname(sw, c".swp".as_ptr(), false);
                xfree(sw.cast());
            }
            return r;
        }

        let r = modname(
            fname_res,
            c".swp".as_ptr(),
            *dir_name == b'.' as c_char && *dir_name.add(1) == NUL,
        );
        if r.is_null() {
            return ptr::null_mut();
        }

        let res = get_file_in_dir(r, dir_name);
        xfree(r.cast());
        res
    }
}

/// Get file name to use for swapfile or backup file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_file_in_dir(fname: *mut c_char, dname: *mut c_char) -> *mut c_char {
    unsafe {
        let tail = cstr_as_string(path_tail(fname));
        let retval: NvimString;

        if *dname == b'.' as c_char && *dname.add(1) == NUL {
            let n = tail.data.offset_from(fname) as usize + tail.size;
            return xmemdupz(fname.cast(), n).cast();
        }

        let dname_len = c_strlen(dname);
        if *dname == b'.' as c_char && vim_ispathsep(*dname.add(1) as u8 as c_int) {
            if tail.data == fname {
                retval = concat_fnames(
                    NvimString {
                        data: dname.add(2),
                        size: dname_len - 2,
                    },
                    tail,
                    true,
                );
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
                retval = concat_fnames(tmp, tail, true);
                xfree(tmp.data.cast());
            }
        } else {
            retval = concat_fnames(
                NvimString {
                    data: dname,
                    size: dname_len,
                },
                tail,
                true,
            );
        }

        retval.data
    }
}

unsafe fn attention_message(
    buf: *mut buf_T,
    fname: *mut c_char,
    fhname: *mut c_char,
    msg: &mut Vec<u8>,
) {
    unsafe {
        assert!(!(*buf).b_fname.is_null());

        emsg(tr(c"E325: ATTENTION"));
        sb_push_cstr(msg, tr(c"Found a swap file by the name \""));
        sb_push_cstr(msg, fhname);
        msg.extend_from_slice(b"\"\n");
        let swap_mtime = swapfile_info(fname, msg);
        sb_push_cstr(msg, tr(c"While opening file \""));
        sb_push_cstr(msg, (*buf).b_fname);
        msg.extend_from_slice(b"\"\n");
        let mut file_info = FileInfo::default();
        if !os_fileinfo((*buf).b_fname, &raw mut file_info) {
            sb_push_cstr(msg, tr(c"      CANNOT BE FOUND"));
        } else {
            sb_push_cstr(msg, tr(c"             dated: "));
            let x: time_t = i64::from(file_info.stat.st_mtim.tv_sec);
            let mut ctime_buf = [0 as c_char; 50];
            let ct = os_ctime_r(
                &raw const x,
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
        sb_push_cstr(msg, (*buf).b_fname);
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
}

unsafe fn do_swapexists(buf: *mut buf_T, fname: *mut c_char) -> sea_choice_T {
    unsafe {
        set_vim_var_string(VV_SWAPNAME, fname, -1);
        set_vim_var_string(VV_SWAPCHOICE, ptr::null(), -1);

        allbuf_lock += 1;
        apply_autocmds(
            EVENT_SWAPEXISTS,
            (*buf).b_fname,
            ptr::null_mut(),
            false,
            ptr::null_mut(),
        );
        allbuf_lock -= 1;

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

unsafe fn findswapname(
    buf: *mut buf_T,
    dirp: *mut *mut c_char,
    old_fname: *mut c_char,
    found_existing_dir: *mut bool,
) -> *mut c_char {
    unsafe {
        let buf_fname = (*buf).b_fname;
        let dir_len = c_strlen(*dirp) + 1;
        let dir_name = xmalloc(dir_len).cast::<c_char>();
        copy_option_part(dirp, dir_name, dir_len, c",".as_ptr());

        let mut fname = makeswapname(buf_fname, (*buf).b_ffname, buf, dir_name);

        loop {
            if fname.is_null() {
                break;
            }
            let n = c_strlen(fname);
            if n == 0 {
                xfree(fname.cast());
                fname = ptr::null_mut();
                break;
            }
            let mut file_info = FileInfo::default();
            let file_or_link_found = os_fileinfo_link(fname, &raw mut file_info);
            if !file_or_link_found {
                break;
            }

            if !old_fname.is_null() && path_equal(fname, old_fname, kPathCmpLiteral) {
                break;
            }

            if *fname.add(n - 2) == b'w' as c_char && *fname.add(n - 1) == b'p' as c_char {
                if !recoverymode
                    && !buf_fname.is_null()
                    && !(*buf).b_help
                    && ((*buf).b_flags & BF_DUMMY) == 0
                {
                    let mut b0 = std::mem::zeroed::<ZeroBlock>();
                    let mut differ = false;
                    let namebuff_ptr = (&raw mut NameBuff).cast::<c_char>();

                    let fd = os_open(fname, O_RDONLY, 0);
                    if fd >= 0 {
                        if read_eintr(fd, (&raw mut b0).cast(), size_of::<ZeroBlock>())
                            == size_of::<ZeroBlock>() as isize
                        {
                            #[cfg(windows)]
                            path_to_slash((&raw mut b0.b0_fname).cast());
                            proc_running = swapfile_proc_running(&raw const b0, fname);

                            if (b0.flags() & B0_SAME_DIR) != 0 {
                                if !path_equal(
                                    path_tail((*buf).b_ffname),
                                    path_tail((&raw const b0.b0_fname).cast()),
                                    kPathCmpLiteral,
                                ) || !same_directory(fname, (*buf).b_ffname)
                                {
                                    expand_env(
                                        (&raw mut b0.b0_fname).cast(),
                                        namebuff_ptr,
                                        MAXPATHL as c_int,
                                    );
                                    if fnamecmp_ino(
                                        (*buf).b_ffname,
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
                                    (*buf).b_ffname,
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
                        && ((*curbuf).b_flags & BF_RECOVERED) == 0
                        && vim_strchr(p_shm, kShmAttention).is_null()
                    {
                        let mut choice: sea_choice_T = SEA_CHOICE_NONE;

                        if os_path_exists((*buf).b_fname) && swapfile_unchanged(fname) {
                            choice = SEA_CHOICE_DELETE;
                            if p_verbose > 0 {
                                verb_msg(tr(c"Found a swap file that is not useful, deleting it"));
                            }
                        }

                        if choice == SEA_CHOICE_NONE
                            && swap_exists_action != SEA_NONE
                            && has_autocmd(EVENT_SWAPEXISTS, buf_fname, buf)
                        {
                            choice = do_swapexists(buf, fname);
                        }

                        if choice == SEA_CHOICE_NONE && swap_exists_action == SEA_READONLY {
                            choice = SEA_CHOICE_READONLY;
                        }

                        proc_running = 0;
                        if choice == SEA_CHOICE_NONE {
                            no_wait_return += 1;
                            let mut msg_sb = Vec::<u8>::with_capacity(IOSIZE);
                            let fhname = home_replace_save(ptr::null_mut(), fname);
                            attention_message(buf, fname, fhname, &mut msg_sb);

                            got_int = false;
                            flush_buffers(FLUSH_TYPEAHEAD);

                            if swap_exists_action != SEA_NONE {
                                sb_push_cstr(&mut msg_sb, tr(c"Swap file \""));
                                sb_push_cstr(&mut msg_sb, fhname);
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
                                    if proc_running != 0 { run_but } else { but },
                                    1,
                                    ptr::null(),
                                    0,
                                );

                                if proc_running != 0 && choice >= 4 {
                                    choice += 1;
                                }
                                msg_reset_scroll();
                            } else {
                                let mut need_clear = false;
                                msg_ext_set_kind(c"wmsg".as_ptr());
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
                                    &raw mut need_clear,
                                );
                            }
                            no_wait_return -= 1;
                            xfree(fhname.cast());
                        }

                        match choice {
                            SEA_CHOICE_READONLY => {
                                (*buf).b_p_ro = 1;
                            }
                            SEA_CHOICE_EDIT => {}
                            SEA_CHOICE_RECOVER => {
                                swap_exists_action = SEA_RECOVER;
                            }
                            SEA_CHOICE_DELETE => {
                                os_remove(fname);
                            }
                            SEA_CHOICE_QUIT => {
                                swap_exists_action = SEA_QUIT;
                            }
                            SEA_CHOICE_ABORT => {
                                swap_exists_action = SEA_QUIT;
                                got_int = true;
                            }
                            SEA_CHOICE_NONE => {
                                msg_puts(c"\n".as_ptr());
                                if msg_silent == 0 {
                                    need_wait_return = true;
                                }
                            }
                            _ => {}
                        }

                        if choice != SEA_CHOICE_NONE && !os_path_exists(fname) {
                            break;
                        }
                    }
                }
            }

            if *fname.add(n - 1) == b'a' as c_char {
                if *fname.add(n - 2) == b'a' as c_char {
                    emsg(tr(c"E326: Too many swap files found"));
                    xfree(fname.cast());
                    fname = ptr::null_mut();
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
            let mut failed_dir: *mut c_char = ptr::null_mut();
            let ret = os_mkdir_recurse(dir_name, 0o755, &raw mut failed_dir, ptr::null_mut());
            if ret != 0 {
                semsg!(
                    tr(c"E303: Unable to create directory \"%s\" for swap file, recovery impossible: %s"),
                    failed_dir,
                    os_strerror(ret),
                );
                xfree(failed_dir.cast());
            }
        }

        xfree(dir_name.cast());
        fname
    }
}

#[inline]
unsafe fn b0_magic_wrong(b0p: *const ZeroBlock) -> bool {
    unsafe {
        (*b0p).b0_magic_long != B0_MAGIC_LONG
            || (*b0p).b0_magic_int != B0_MAGIC_INT
            || (*b0p).b0_magic_short != B0_MAGIC_SHORT
            || (*b0p).b0_magic_char != B0_MAGIC_CHAR
    }
}

unsafe fn fnamecmp_ino(fname_c: *mut c_char, fname_s: *mut c_char, ino_block0: c_long) -> bool {
    unsafe {
        let mut ino_c: u64 = 0;
        let mut buf_c = [0 as c_char; MAXPATHL];
        let mut buf_s = [0 as c_char; MAXPATHL];

        let mut file_info = FileInfo::default();
        if os_fileinfo(fname_c, &raw mut file_info) {
            ino_c = os_fileinfo_inode(&raw const file_info);
        }

        let ino_s: u64 = if os_fileinfo(fname_s, &raw mut file_info) {
            os_fileinfo_inode(&raw const file_info)
        } else {
            ino_block0 as u64
        };

        if ino_c != 0 && ino_s != 0 {
            return ino_c != ino_s;
        }

        let buf_c_ptr = (&raw mut buf_c).cast::<c_char>();
        let buf_s_ptr = (&raw mut buf_s).cast::<c_char>();
        let retval_c = vim_FullName(fname_c, buf_c_ptr, MAXPATHL, true);
        let retval_s = vim_FullName(fname_s, buf_s_ptr, MAXPATHL, true);
        if retval_c == OK && retval_s == OK {
            return c_strcmp(buf_c_ptr, buf_s_ptr) != 0;
        }

        if ino_s == 0 && ino_c == 0 && retval_c == FAIL && retval_s == FAIL {
            return c_strcmp(fname_c, fname_s) != 0;
        }
        true
    }
}

#[inline]
unsafe fn long_to_char(n: c_long, s_in: *mut c_char) {
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
unsafe fn char_to_long(s_in: *const c_char) -> c_long {
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_setflags(buf: *mut buf_T) {
    unsafe {
        if (*buf).b_ml.ml_mfp.is_null() {
            return;
        }
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        let hp = pmap_get_bhdr(&raw mut (*mfp).mf_hash, 0);
        if !hp.is_null() {
            let b0p = (*hp).bh_data.cast::<ZeroBlock>();
            (*b0p).set_dirty(if (*buf).b_changed != 0 { B0_DIRTY } else { 0 });
            let ff_bits = ((get_fileformat(buf) + 1) as u8) & B0_FF_MASK;
            (*b0p).set_flags(((*b0p).flags() & !B0_FF_MASK) | ff_bits);
            add_b0_fenc(b0p, buf);
            (*hp).bh_flags |= BH_DIRTY;
            mf_sync(mfp, MFS_ZERO);
        }
    }
}

unsafe fn ml_updatechunk(buf: *mut buf_T, line: linenr_T, mut len: c_int, updtype: c_int) {
    unsafe {
        let mut curline = ml_upd_lastcurline;
        let mut curix = ml_upd_lastcurix;

        if (*buf).b_ml.ml_usedchunks == -1 || len == 0 {
            return;
        }
        if (*buf).b_ml.ml_chunksize.is_null() {
            (*buf).b_ml.ml_chunksize = xmalloc(size_of::<chunksize_T>() * 100);
            (*buf).b_ml.ml_numchunks = 100;
            (*buf).b_ml.ml_usedchunks = 1;
            let cs0 = (*buf).b_ml.ml_chunksize.cast::<chunksize_T>();
            (*cs0).mlcs_numlines = 1;
            (*cs0).mlcs_totalsize = 1;
            ml_upd_lastbuf = ptr::null_mut();
        }

        if updtype == ML_CHNK_UPDLINE && (*buf).b_ml.ml_line_count == 1 {
            (*buf).b_ml.ml_usedchunks = 1;
            let cs0 = (*buf).b_ml.ml_chunksize.cast::<chunksize_T>();
            (*cs0).mlcs_numlines = 1;
            (*cs0).mlcs_totalsize = (*buf).b_ml.ml_line_textlen;
            ml_upd_lastbuf = ptr::null_mut();
            return;
        }

        if buf != ml_upd_lastbuf || line < curline {
            curline = 1;
            curix = 0;
        }
        let chunks = (*buf).b_ml.ml_chunksize.cast::<chunksize_T>();
        while curix < (*buf).b_ml.ml_usedchunks - 1
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

            if (*buf).b_ml.ml_usedchunks + 1 >= (*buf).b_ml.ml_numchunks {
                (*buf).b_ml.ml_numchunks = (*buf).b_ml.ml_numchunks * 3 / 2;
                (*buf).b_ml.ml_chunksize = xrealloc(
                    (*buf).b_ml.ml_chunksize,
                    size_of::<chunksize_T>() * ((*buf).b_ml.ml_numchunks as usize),
                );
            }
            let chunks = (*buf).b_ml.ml_chunksize.cast::<chunksize_T>();

            if (*chunks.add(curix as usize)).mlcs_numlines >= MLCS_MAXL {
                ptr::copy(
                    chunks.add(curix as usize),
                    chunks.add((curix + 1) as usize),
                    ((*buf).b_ml.ml_usedchunks - curix) as usize,
                );
                let mut size: c_int = 0;
                let mut linecnt: c_int = 0;
                while curline < (*buf).b_ml.ml_line_count && linecnt < MLCS_MINL {
                    let hp = ml_find_line(buf, curline, ML_FIND);
                    if hp.is_null() {
                        (*buf).b_ml.ml_usedchunks = -1;
                        return;
                    }
                    let dp = (*hp).bh_data.cast::<DataBlock>();
                    let count = (*buf).b_ml.ml_locked_high - (*buf).b_ml.ml_locked_low + 1;
                    let idx = curline - (*buf).b_ml.ml_locked_low;
                    curline = (*buf).b_ml.ml_locked_high + 1;
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
                        (*db_idx_ptr(dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int
                    };
                    size +=
                        text_end - ((*db_idx_ptr(dp, end_idx as usize) & DB_INDEX_MASK) as c_int);
                }
                (*chunks.add(curix as usize)).mlcs_numlines = linecnt;
                (*chunks.add((curix + 1) as usize)).mlcs_numlines -= linecnt;
                (*chunks.add(curix as usize)).mlcs_totalsize = size;
                (*chunks.add((curix + 1) as usize)).mlcs_totalsize -= size;
                (*buf).b_ml.ml_usedchunks += 1;
                ml_upd_lastbuf = ptr::null_mut();
                return;
            } else if (*chunks.add(curix as usize)).mlcs_numlines >= MLCS_MINL
                && curix == (*buf).b_ml.ml_usedchunks - 1
                && (*buf).b_ml.ml_line_count - line <= 1
            {
                curchnk = chunks.add((curix + 1) as usize);
                (*buf).b_ml.ml_usedchunks += 1;
                if line == (*buf).b_ml.ml_line_count {
                    (*curchnk).mlcs_numlines = 0;
                    (*curchnk).mlcs_totalsize = 0;
                } else {
                    let hp = ml_find_line(buf, (*buf).b_ml.ml_line_count, ML_FIND);
                    if hp.is_null() {
                        (*buf).b_ml.ml_usedchunks = -1;
                        return;
                    }
                    let dp = (*hp).bh_data.cast::<DataBlock>();
                    let rest = if (*dp).db_line_count == 1 {
                        ((*dp).db_txt_end - (*dp).db_txt_start) as c_int
                    } else {
                        ((*db_idx_ptr(dp, ((*dp).db_line_count - 2) as usize) & DB_INDEX_MASK)
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
            let chunks = (*buf).b_ml.ml_chunksize.cast::<chunksize_T>();
            if curix < (*buf).b_ml.ml_usedchunks - 1
                && ((*curchnk).mlcs_numlines + (*curchnk.add(1)).mlcs_numlines) <= MLCS_MINL
            {
                curix += 1;
                curchnk = chunks.add(curix as usize);
            } else if curix == 0 && (*curchnk).mlcs_numlines <= 0 {
                ml_upd_lastbuf = ptr::null_mut();
                (*buf).b_ml.ml_usedchunks -= 1;
                ptr::copy(chunks.add(1), chunks, (*buf).b_ml.ml_usedchunks as usize);
                return;
            } else if curix == 0
                || ((*curchnk).mlcs_numlines > 10
                    && ((*curchnk).mlcs_numlines + (*curchnk.offset(-1)).mlcs_numlines) > MLCS_MINL)
            {
                ml_upd_lastbuf = buf;
                ml_upd_lastcurline = curline;
                ml_upd_lastcurix = curix;
                return;
            }

            ml_upd_lastbuf = ptr::null_mut();
            (*curchnk.offset(-1)).mlcs_numlines += (*curchnk).mlcs_numlines;
            (*curchnk.offset(-1)).mlcs_totalsize += (*curchnk).mlcs_totalsize;
            (*buf).b_ml.ml_usedchunks -= 1;
            if curix < (*buf).b_ml.ml_usedchunks {
                ptr::copy(
                    chunks.add((curix + 1) as usize),
                    chunks.add(curix as usize),
                    ((*buf).b_ml.ml_usedchunks - curix) as usize,
                );
            }
            return;
        }
        ml_upd_lastbuf = buf;
        ml_upd_lastcurline = curline;
        ml_upd_lastcurix = curix;
    }
}

/// Find offset for line or line with offset.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ml_find_line_or_offset(
    buf: *mut buf_T,
    lnum: linenr_T,
    offp: *mut c_int,
    no_ff: bool,
) -> c_int {
    unsafe {
        let ffdos = c_int::from(!no_ff && get_fileformat(buf) == EOL_DOS);
        let mut extra: c_int = 0;

        let can_cache = lnum != 0 && ffdos == 0 && (*buf).b_ml.ml_line_lnum == lnum;
        if lnum == 0 || (*buf).b_ml.ml_line_lnum < lnum || !no_ff {
            ml_flush_line(curbuf, false);
        } else if can_cache && (*buf).b_ml.ml_line_offset > 0 {
            return (*buf).b_ml.ml_line_offset as c_int;
        }

        if (*buf).b_ml.ml_usedchunks == -1 || (*buf).b_ml.ml_chunksize.is_null() || lnum < 0 {
            if no_ff && !(*buf).b_ml.ml_mfp.is_null() && (lnum == 1 || lnum == 2) {
                return lnum - 1;
            }
            return -1;
        }

        let offset = if offp.is_null() { 0 } else { *offp };
        if lnum == 0 && offset <= 0 {
            return 1;
        }

        let mut curline: linenr_T = 1;
        let mut curix: c_int = 0;
        let mut size: c_int = 0;
        let chunks = (*buf).b_ml.ml_chunksize.cast::<chunksize_T>();
        while curix < (*buf).b_ml.ml_usedchunks - 1
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
            if curline > (*buf).b_ml.ml_line_count {
                return -1;
            }
            let hp = ml_find_line(buf, curline, ML_FIND);
            if hp.is_null() {
                return -1;
            }
            let dp = (*hp).bh_data.cast::<DataBlock>();
            let count = (*buf).b_ml.ml_locked_high - (*buf).b_ml.ml_locked_low + 1;
            let start_idx = curline - (*buf).b_ml.ml_locked_low;
            let mut idx = start_idx;
            let text_end = if idx == 0 {
                (*dp).db_txt_end as c_int
            } else {
                (*db_idx_ptr(dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int
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
                            - ((*db_idx_ptr(dp, idx as usize) & DB_INDEX_MASK) as c_int)
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
            let len = text_end - ((*db_idx_ptr(dp, idx as usize) & DB_INDEX_MASK) as c_int);
            size += len;
            if offset != 0 && size >= offset {
                if size + ffdos == offset {
                    *offp = 0;
                } else if idx == start_idx {
                    *offp = offset - size + len;
                } else {
                    *offp = offset - size + len
                        - (text_end
                            - ((*db_idx_ptr(dp, (idx - 1) as usize) & DB_INDEX_MASK) as c_int));
                }
                curline += idx - start_idx + extra;
                if curline > (*buf).b_ml.ml_line_count {
                    return -1;
                }
                return curline;
            }
            curline = (*buf).b_ml.ml_locked_high + 1;
        }

        if lnum != 0 {
            if ffdos != 0 {
                size += lnum - 1;
            }
            if ((*buf).b_p_fixeol == 0 || (*buf).b_p_bin != 0)
                && (*buf).b_p_eol == 0
                && lnum > (*buf).b_ml.ml_line_count
            {
                size -= ffdos + 1;
            }
        }

        if can_cache && size > 0 {
            (*buf).b_ml.ml_line_offset = size as usize;
        }

        size
    }
}

/// Goto byte in buffer with offset `cnt`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn goto_byte(cnt: c_int) {
    unsafe {
        let mut boff = cnt;
        ml_flush_line(curbuf, false);
        setpcmark();
        if boff != 0 {
            boff -= 1;
        }
        let lnum = ml_find_line_or_offset(curbuf, 0, &raw mut boff, false) as linenr_T;
        if lnum < 1 {
            (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
            (*curwin).w_curswant = MAXCOL;
            coladvance(curwin, MAXCOL);
        } else {
            (*curwin).w_cursor.lnum = lnum;
            (*curwin).w_cursor.col = boff as colnr_T;
            (*curwin).w_cursor.coladd = 0;
            (*curwin).w_set_curswant = true;
        }
        check_cursor(curwin);
        mb_adjust_cursor();
    }
}

/// Increment the line pointer `lp` crossing line boundaries as necessary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn inc(lp: *mut pos_T) -> c_int {
    unsafe {
        if (*lp).col != MAXCOL {
            let p = ml_get_pos(lp);
            if *p != NUL {
                let l = utfc_ptr2len(p);
                (*lp).col += l;
                return if *p.add(l as usize) != NUL { 0 } else { 2 };
            }
        }
        if (*lp).lnum != (*curbuf).b_ml.ml_line_count {
            (*lp).col = 0;
            (*lp).lnum += 1;
            (*lp).coladd = 0;
            return 1;
        }
        -1
    }
}

/// Same as `inc()`, but skip NUL at the end of non-empty lines.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn incl(lp: *mut pos_T) -> c_int {
    unsafe {
        let mut r = inc(lp);
        if r >= 1 && (*lp).col != 0 {
            r = inc(lp);
        }
        r
    }
}

/// Decrements position `lp` by one character (multibyte-aware), crossing line boundaries.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn dec(lp: *mut pos_T) -> c_int {
    unsafe {
        (*lp).coladd = 0;
        if (*lp).col == MAXCOL {
            let p = ml_get((*lp).lnum);
            (*lp).col = ml_get_len((*lp).lnum);
            (*lp).col -= utf_head_off(p, p.offset((*lp).col as isize));
            return 0;
        }

        if (*lp).col > 0 {
            (*lp).col -= 1;
            let p = ml_get((*lp).lnum);
            (*lp).col -= utf_head_off(p, p.offset((*lp).col as isize));
            return 0;
        }
        if (*lp).lnum > 1 {
            (*lp).lnum -= 1;
            let p = ml_get((*lp).lnum);
            (*lp).col = ml_get_len((*lp).lnum);
            (*lp).col -= utf_head_off(p, p.offset((*lp).col as isize));
            return 1;
        }

        -1
    }
}

/// Same as `dec()`, but skip NUL at the end of non-empty lines.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decl(lp: *mut pos_T) -> c_int {
    unsafe {
        let mut r = dec(lp);
        if r == 1 && (*lp).col != 0 {
            r = dec(lp);
        }
        r
    }
}
