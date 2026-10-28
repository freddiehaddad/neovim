//! Multi-level undo facility (`src/nvim/undo.c`).

use crate::api::private::validate::NvimString;
use crate::autocmd::{HLF_T, block_autocmds, unblock_autocmds};
use crate::buffer::{
    MAXLNUM, NMARKS, NO_LOCAL_UNDOLEVEL, bt_dontwrite, bt_prompt, buf_T, buf_get_changedtick,
    buf_is_empty, buf_set_changedtick,
};
use crate::buffer_updates::{buf_updates_changedtick, buf_updates_unload};
use crate::change::{
    change_warning, changed, changed_bytes, changed_lines, file_ff_differs, unchanged,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{
    check_cursor, check_cursor_col, check_cursor_lnum, check_pos, clearpos, coladvance, colnr_T,
    equalpos, getviscol, linenr_T, pos_T,
};
use crate::drawscreen::{UPD_NOT_VALID, redraw_later, redrawWinline};
use crate::errors::*;
use crate::eval::funcs::get_buf_arg;
use crate::eval::typval::{
    EvalFuncData, VarType, list_T, tv_dict_add_list, tv_dict_add_nr, tv_dict_alloc,
    tv_dict_alloc_ret, tv_get_string, tv_list_alloc, tv_list_append_dict, typval_T, varnumber_T,
};
use crate::event::r#loop::uv_strerror;
use crate::ex_docmd::{UNDO_HASH_SIZE, exarg_T, expr_map_locked};
use crate::ex_getln::{text_locked, text_locked_msg};
use crate::extmark::{
    ExtmarkMove, ExtmarkSplice, ExtmarkUndoObject, UndoObjectType, extmark_apply_undo,
    extmark_splice_cols, extmark_undo_vec_t, kExtmarkMove, kExtmarkNOOP, kExtmarkSplice,
    kExtmarkUndo,
};
#[cfg(unix)]
use crate::fileio::getuid;
use crate::fileio::{
    EOF, O_RDONLY, fdopen, feof, fflush, fgetc, fileno, fread, fwrite, get2c, get4c, get8ctime,
    off_T, read_eintr, vim_ftell,
};
use crate::fold::foldOpenCursor;
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::input::beep_flush;
use crate::input_cmdatom::{atom_did_global_op, atom_origin_pos};
use crate::insert::beginline;
use crate::mark::{fmark_T, mark_adjust, setpcmark};
use crate::mbyte::utfc_ptr2len;
use crate::mcursor::mc_undo_time;
#[cfg(unix)]
use crate::memline::resolve_symlink;
use crate::memline::{ml_append_flags, ml_delete, ml_get, ml_get_buf, ml_replace};
use crate::message::{
    emsg, give_warning, iemsg, internal_error, messaging, msg, msg_end, msg_ext_set_kind,
    msg_putchar, msg_puts, msg_puts_hl, msg_start, semsg, smsg, smsg_keep, verb_msg, verbose_enter,
    verbose_leave,
};
use crate::normal::visualinfo_T;
use crate::option::{OptInt, copy_option_part, shortmess};
use crate::option_vars::*;
use crate::os::fs::{
    FileInfo, MAXPATHL, O_CREAT, O_EXCL, O_NOFOLLOW, O_WRONLY, os_close, os_fileinfo_fd,
    os_fileinfo_size, os_fopen, os_free_acl, os_fsync, os_get_acl, os_getperm, os_isdir,
    os_mkdir_recurse, os_open, os_path_exists, os_remove, os_set_acl, os_setperm, vim_acl_T,
};
use crate::os::input::fast_breakcheck;
use crate::os::time::{Timestamp, Tm, os_localtime_r, os_time, strftime, time_t};
use crate::path::{FullName_save, concat_fnames, path_tail, vim_ispathsep, vim_ispathsep_nocolon};
use crate::sha256::context_sha256_T;
use crate::spell::spell_check_window;
use crate::state::virtual_active;
use crate::strings::{
    gettext, ngettext, vim_snprintf, vim_snprintf_add, vim_strchr,
};
use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong};
use std::ptr;

#[cfg(unix)]
use crate::event::r#loop::{uv_gid_t, uv_uid_t};
#[cfg(unix)]
use crate::os::fs::{os_fchown, os_fileinfo};

use crate::arglist::ML_EMPTY;
use crate::charset::NUL;
use crate::eval::kListLenMayKnow;
use crate::ex_cmds::{BL_FIX, BL_SOL, kCpoUndo};
use crate::math::{FAIL, OK};

const UH_CHANGED: c_int = 0x01;
const UH_EMPTYBUF: c_int = 0x02;
const UH_RELOAD: c_int = 0x04;

const kShmUndo: c_int = b'u' as c_int;
const kOptFdoFlagUndo: c_uint = 0x200;

const UF_START_MAGIC: &[u8; 9] = b"Vim\x9fUnDo\xe5";
const UF_START_MAGIC_LEN: usize = 9;
const UF_HEADER_MAGIC: c_int = 0x5fd0;
const UF_HEADER_END_MAGIC: c_int = 0xe7aa;
const UF_ENTRY_MAGIC: c_int = 0xf518;
const UF_ENTRY_END_MAGIC: c_int = 0x3581;
const UF_VERSION: c_int = 3;
const UF_LAST_SAVE_NR: c_int = 1;
const UHP_SAVE_NR: c_int = 1;

const E_UNDO_LIST_CORRUPT: &CStr = c"E439: Undo list corrupt";
const E_UNDO_LINE_MISSING: &CStr = c"E440: Undo line missing";
const E_WRITE_ERROR_IN_UNDO_FILE_STR: &CStr = c"E829: Write error in undo file: %s";
const E_NOT_OPEN: &CStr = c"E828: Cannot open undo file for writing: %s";

#[repr(C)]
pub struct u_entry_T {
    pub ue_top: linenr_T,
    pub ue_bot: linenr_T,
    pub ue_lcount: linenr_T,
    pub ue_array: Vec<std::ffi::CString>,
}

impl u_entry_T {
    #[inline]
    pub const fn ue_size(&self) -> linenr_T {
        self.ue_array.len() as linenr_T
    }
}

#[repr(C)]
pub struct u_header_T {
    pub uh_next: Option<usize>,
    pub uh_prev: Option<usize>,
    pub uh_alt_next: Option<usize>,
    pub uh_alt_prev: Option<usize>,
    pub uh_seq: c_int,
    pub uh_walk: c_int,
    pub uh_entries: Vec<u_entry_T>,
    pub uh_getbot_entry: bool,
    pub uh_cursor: pos_T,
    pub uh_cursor_after: pos_T,
    pub uh_cursor_vcol: colnr_T,
    pub uh_flags: c_int,
    pub uh_namedm: [fmark_T; NMARKS],
    pub uh_extmark: extmark_undo_vec_t,
    pub uh_visual: visualinfo_T,
    pub uh_time: time_t,
    pub uh_save_nr: c_int,
}

impl u_header_T {
    fn new() -> Self {
        Self {
            uh_next: None,
            uh_prev: None,
            uh_alt_next: None,
            uh_alt_prev: None,
            uh_seq: 0,
            uh_walk: 0,
            uh_entries: Vec::new(),
            uh_getbot_entry: false,
            uh_cursor: pos_T::new(),
            uh_cursor_after: pos_T::new(),
            uh_cursor_vcol: 0,
            uh_flags: 0,
            uh_namedm: [const { fmark_T::new() }; NMARKS],
            uh_extmark: Vec::new(),
            uh_visual: visualinfo_T::new(),
            uh_time: 0,
            uh_save_nr: 0,
        }
    }

    fn clone_from_slice_or_reset(&mut self) {
        *self = Self::new();
    }
}

#[inline]
pub fn uhp_ref<'a>(buf: &buf_T, uh: usize) -> &'a u_header_T {
    unsafe { &*buf.b_u_headers.as_ptr().add(uh) }
}

#[inline]
pub fn uhp_mut<'a>(buf: &mut buf_T, uh: usize) -> &'a mut u_header_T {
    unsafe { &mut *buf.b_u_headers.as_mut_ptr().add(uh) }
}

fn u_alloc_header(buf: *mut buf_T) -> usize {
    unsafe {
        if let Some(idx) = (*buf).b_u_free_headers.pop() {
            uhp_mut(&mut *buf, idx).clone_from_slice_or_reset();
            idx
        } else {
            let idx = (*buf).b_u_headers.len();
            (*buf).b_u_headers.push(u_header_T::new());
            idx
        }
    }
}

#[derive(Clone)]
pub struct UndoCheckpoint {
    pub uc_oldhead: Option<usize>,
    pub uc_newhead: Option<usize>,
    pub uc_curhead: Option<usize>,
    pub uc_numhead: c_int,
    pub uc_synced: bool,
    pub uc_seq_last: c_int,
    pub uc_save_nr_last: c_int,
    pub uc_seq_cur: c_int,
    pub uc_time_cur: time_t,
    pub uc_save_nr_cur: c_int,
    pub uc_line_ptr: Option<std::ffi::CString>,
    pub uc_line_lnum: linenr_T,
    pub uc_line_colnr: colnr_T,
    pub uc_undolevels: OptInt,
    pub uc_changedtick: varnumber_T,
}

impl UndoCheckpoint {
    pub const fn new() -> Self {
        Self {
            uc_oldhead: None,
            uc_newhead: None,
            uc_curhead: None,
            uc_numhead: 0,
            uc_synced: false,
            uc_seq_last: 0,
            uc_save_nr_last: 0,
            uc_seq_cur: 0,
            uc_time_cur: 0,
            uc_save_nr_cur: 0,
            uc_line_ptr: None,
            uc_line_lnum: 0,
            uc_line_colnr: 0,
            uc_undolevels: 0,
            uc_changedtick: 0,
        }
    }
}

impl Default for UndoCheckpoint {
    fn default() -> Self {
        Self::new()
    }
}

struct bufinfo_T {
    bi_buf: *mut buf_T,
    bi_fp: crate::os::fs::CFile,
    bi_fsize: off_T,
}

struct UndoModuleState {
    u_newcount: c_int,
    u_oldcount: c_int,
    undo_undoes: bool,
    lastmark: c_int,
}

impl UndoModuleState {
    const fn new() -> Self {
        Self {
            u_newcount: 0,
            u_oldcount: 0,
            undo_undoes: false,
            lastmark: 0,
        }
    }
}

static UNDO_STATE: GlobalState<UndoModuleState> = GlobalState::new(UndoModuleState::new());

#[inline]
fn undo_st() -> &'static mut UndoModuleState {
    unsafe { &mut *UNDO_STATE.as_ptr() }
}

/// Save the current line for both the `"u"` and `"U"` command.
pub fn u_save_cursor() -> c_int {
    unsafe {
        let cur = (**curwin.as_ptr()).w_cursor.lnum;
        let top = if cur > 0 { cur - 1 } else { 0 };
        let bot = cur + 1;
        u_save(top, bot)
    }
}

/// Save the lines between `top` and `bot` for both the `"u"` and `"U"` command.
pub fn u_save(top: linenr_T, bot: linenr_T) -> c_int {
    unsafe { u_save_buf(&mut *g_st().curbuf, top, bot) }
}

pub fn u_save_buf(buf: &mut buf_T, top: linenr_T, bot: linenr_T) -> c_int {
    if top >= bot || bot > buf.b_ml.ml_line_count + 1 {
            return FAIL;
        }

        if top + 2 == bot {
            u_saveline(buf, top + 1);
        }

        u_savecommon(&raw mut *buf, top, bot, 0, false)
}

/// Save the line `lnum` (used by `":s"` and `"~"` command).
pub fn u_savesub(lnum: linenr_T) -> c_int {
    u_savecommon(g_st().curbuf, lnum - 1, lnum + 1, lnum + 1, false)
}

/// A new line is inserted before line `lnum` (used by `:s` command).
pub fn u_inssub(lnum: linenr_T) -> c_int {
    u_savecommon(g_st().curbuf, lnum - 1, lnum, lnum + 1, false)
}

/// Save the lines `lnum` - `lnum + nlines` (used by delete command).
pub fn u_savedel(lnum: linenr_T, nlines: linenr_T) -> c_int {
    unsafe {
        u_savecommon(
            g_st().curbuf,
            lnum - 1,
            lnum + nlines,
            if nlines == (*g_st().curbuf).b_ml.ml_line_count {
                2
            } else {
                lnum
            },
            false,
        )
    }
}

/// Return true when undo is allowed. Otherwise print an error message and return false.
pub fn undo_allowed(buf: *mut buf_T) -> bool {
    unsafe {
        if (*buf).b_p_ma == 0 {
            emsg(gettext((&raw const e_modifiable).cast()));
            return false;
        }

        if g_st().sandbox != 0 {
            emsg(gettext((&raw const e_sandbox).cast()));
            return false;
        }

        if g_st().textlock != 0 || expr_map_locked() {
            emsg(gettext((&raw const e_textlock).cast()));
            return false;
        }

        true
    }
}

fn get_undolevel(buf: *mut buf_T) -> OptInt {
    unsafe {
        if (*buf).b_p_ul == NO_LOCAL_UNDOLEVEL {
            opt_vars().p_ul
        } else {
            (*buf).b_p_ul
        }
    }
}

#[inline]
fn zero_fmark_additional_data(fmarks: *mut fmark_T) {
    unsafe {
        for i in 0..NMARKS {
            (*fmarks.add(i)).additional_data = None;
        }
    }
}

/// Common code for various ways to save text before a change.
pub fn u_savecommon(
    buf: *mut buf_T,
    top: linenr_T,
    bot: linenr_T,
    newbot: linenr_T,
    reload: bool,
) -> c_int {
    unsafe {
        if !reload {
            if !undo_allowed(buf) {
                return FAIL;
            }

            if buf == g_st().curbuf {
                change_warning(buf, 0);
            }

            if bot > (*buf).b_ml.ml_line_count + 1 {
                emsg(gettext(c"E881: Line count changed unexpectedly".as_ptr()));
                return FAIL;
            }
        }

        let size = bot - top - 1;

        if (*buf).b_u_synced {
            (*buf).b_new_change = true;

            let uhp = (get_undolevel(buf) >= 0).then(|| u_alloc_header(buf));

            let mut old_curhead = (*buf).b_u_curhead;
            if let Some(oc) = old_curhead {
                (*buf).b_u_newhead = uhp_ref(&*buf, oc).uh_next;
                (*buf).b_u_curhead = None;
            }

            while ((*buf).b_u_numhead as OptInt) > get_undolevel(buf)
                && let Some(mut uhfree) = (*buf).b_u_oldhead
            {
                if Some(uhfree) == old_curhead {
                    u_freebranch(buf, uhfree, &mut old_curhead);
                } else if uhp_ref(&*buf, uhfree).uh_alt_next.is_none() {
                    u_freeheader(buf, uhfree, &mut old_curhead);
                } else {
                    while let Some(alt_next) = uhp_ref(&*buf, uhfree).uh_alt_next {
                        uhfree = alt_next;
                    }
                    u_freebranch(buf, uhfree, &mut old_curhead);
                }
            }

            let Some(uhp) = uhp else {
                if let Some(oc) = old_curhead {
                    let mut dummy = None;
                    u_freebranch(buf, oc, &mut dummy);
                }
                (*buf).b_u_synced = false;
                return OK;
            };

            uhp_mut(&mut *buf, uhp).uh_prev = None;
            uhp_mut(&mut *buf, uhp).uh_next = (*buf).b_u_newhead;
            uhp_mut(&mut *buf, uhp).uh_alt_next = old_curhead;
            if let Some(oc) = old_curhead {
                let oc_alt_prev = uhp_ref(&*buf, oc).uh_alt_prev;
                uhp_mut(&mut *buf, uhp).uh_alt_prev = oc_alt_prev;
                if let Some(ap) = oc_alt_prev {
                    uhp_mut(&mut *buf, ap).uh_alt_next = Some(uhp);
                }
                uhp_mut(&mut *buf, oc).uh_alt_prev = Some(uhp);
                if (*buf).b_u_oldhead == Some(oc) {
                    (*buf).b_u_oldhead = Some(uhp);
                }
            } else {
                uhp_mut(&mut *buf, uhp).uh_alt_prev = None;
            }

            if let Some(nh) = (*buf).b_u_newhead {
                uhp_mut(&mut *buf, nh).uh_prev = Some(uhp);
            }

            (*buf).b_u_seq_last += 1;
            let seq_last = (*buf).b_u_seq_last;
            uhp_mut(&mut *buf, uhp).uh_seq = seq_last;
            (*buf).b_u_seq_cur = seq_last;
            let now_time = os_time() as time_t;
            uhp_mut(&mut *buf, uhp).uh_time = now_time;
            uhp_mut(&mut *buf, uhp).uh_save_nr = 0;
            (*buf).b_u_time_cur = now_time + 1;

            uhp_mut(&mut *buf, uhp).uh_walk = 0;
            uhp_mut(&mut *buf, uhp).uh_entries.clear();
            uhp_mut(&mut *buf, uhp).uh_getbot_entry = false;
            let origin_cursor = atom_origin_pos(buf);
            uhp_mut(&mut *buf, uhp).uh_cursor = origin_cursor;
            if virtual_active(*curwin.as_ptr())
                && (**curwin.as_ptr()).w_cursor.coladd > 0
                && equalpos(origin_cursor, (**curwin.as_ptr()).w_cursor)
            {
                uhp_mut(&mut *buf, uhp).uh_cursor_vcol = getviscol();
            } else {
                uhp_mut(&mut *buf, uhp).uh_cursor_vcol = -1;
            }
            clearpos(&mut uhp_mut(&mut *buf, uhp).uh_cursor_after);

            uhp_mut(&mut *buf, uhp).uh_flags = (if (*buf).b_changed != 0 { UH_CHANGED } else { 0 })
                + (if ((*buf).b_ml.ml_flags & ML_EMPTY) != 0 {
                    UH_EMPTYBUF
                } else {
                    0
                });

            zero_fmark_additional_data(&mut *(&raw mut (*buf).b_namedm).cast::<fmark_T>());
            uhp_mut(&mut *buf, uhp).uh_namedm = (*buf).b_namedm.clone();
            uhp_mut(&mut *buf, uhp).uh_visual = (*buf).b_visual;

            (*buf).b_u_newhead = Some(uhp);
            if (*buf).b_u_oldhead.is_none() {
                (*buf).b_u_oldhead = Some(uhp);
            }
            (*buf).b_u_numhead += 1;
        } else {
            if get_undolevel(buf) < 0 {
                return OK;
            }

            if size == 1 {
                let head_uep = u_get_headentry(&mut *buf);
                if !head_uep.is_null() {
                    let newhead = (*buf).b_u_newhead.unwrap();
                    let limit = uhp_ref(&*buf, newhead).uh_entries.len().min(10);
                    for i in 0..limit {
                        let uep: *mut u_entry_T =
                            &raw mut uhp_mut(&mut *buf, newhead).uh_entries.as_mut_slice()[i];

                        if u_entry_resized(&mut *buf, &*uep)
                            || ((*uep).ue_size() > 1
                                && top >= (*uep).ue_top
                                && top + 2 <= (*uep).ue_top + (*uep).ue_size() + 1)
                        {
                            break;
                        }

                        if (*uep).ue_size() == 1 && (*uep).ue_top == top {
                            if i > 0 {
                                u_getbot(buf);
                                (*buf).b_u_synced = false;

                                let entry = uhp_mut(&mut *buf, newhead).uh_entries.remove(i);
                                uhp_mut(&mut *buf, newhead).uh_entries.insert(0, entry);
                            }

                            let first_uep =
                                &mut uhp_mut(&mut *buf, newhead).uh_entries.as_mut_slice()[0];
                            if newbot != 0 {
                                first_uep.ue_bot = newbot;
                            } else if bot > (*buf).b_ml.ml_line_count {
                                first_uep.ue_bot = 0;
                            } else {
                                first_uep.ue_lcount = (*buf).b_ml.ml_line_count;
                                uhp_mut(&mut *buf, newhead).uh_getbot_entry = true;
                            }
                            return OK;
                        }
                    }
                }
            }

            u_getbot(buf);
        }

        let mut uep = u_entry_T {
            ue_top: top,
            ue_bot: 0,
            ue_lcount: 0,
            ue_array: Vec::new(),
        };

        let newhead = (*buf).b_u_newhead.unwrap();
        if newbot != 0 {
            uep.ue_bot = newbot;
        } else if bot > (*buf).b_ml.ml_line_count {
            uep.ue_bot = 0;
        } else {
            uep.ue_lcount = (*buf).b_ml.ml_line_count;
            uhp_mut(&mut *buf, newhead).uh_getbot_entry = true;
        }

        if size > 0 {
            uep.ue_array.reserve(size as usize);
            for i in 0..size {
                fast_breakcheck();
                if g_st().got_int {
                    uhp_mut(&mut *buf, newhead).uh_getbot_entry = false;
                    return FAIL;
                }
                uep.ue_array.push(u_save_line_buf(buf, top + 1 + i));
            }
        }

        uhp_mut(&mut *buf, newhead).uh_entries.insert(0, uep);
        if reload {
            uhp_mut(&mut *buf, newhead).uh_flags |= UH_RELOAD;
        }
        (*buf).b_u_synced = false;
        undo_st().undo_undoes = false;

        OK
    }
}

fn u_entry_resized(buf: &mut buf_T, uep: &u_entry_T) -> bool {
    let newhead = uhp_ref(buf, buf.b_u_newhead.unwrap());
        if !newhead.uh_getbot_entry || !ptr::eq(uep, newhead.uh_entries.as_ptr()) {
            uep.ue_top + uep.ue_size() + 1
                != if uep.ue_bot == 0 {
                    buf.b_ml.ml_line_count + 1
                } else {
                    uep.ue_bot
                }
        } else {
            uep.ue_lcount != buf.b_ml.ml_line_count
        }
}

/// Compute the hash for a buffer text into `hash[UNDO_HASH_SIZE]`.
pub fn u_compute_hash(buf: &mut buf_T, hash: *mut u8) {
    unsafe {
        let mut ctx = context_sha256_T::new();
        for lnum in 1..=buf.b_ml.ml_line_count {
            let p = ml_get_buf(&raw mut *buf, lnum);
            ctx.update(std::slice::from_raw_parts(p.cast::<u8>(), std::ffi::CStr::from_ptr(p).to_bytes().len() + 1));
        }
        ctx.finish_into(&mut *(hash as *mut [u8; UNDO_HASH_SIZE]));
    }
}

/// Return an allocated string of the full path of the target undofile.
pub fn u_get_undo_file_name(
    buf_ffname: *const c_char,
    reading: bool,
) -> Option<std::ffi::CString> {
    unsafe {
        #[cfg(unix)]
        let mut ffname = buf_ffname;
        #[cfg(not(unix))]
        let ffname = buf_ffname;
        if ffname.is_null() {
            return None;
        }

        #[cfg(unix)]
        let mut fname_buf = [0 as c_char; MAXPATHL];
        #[cfg(unix)]
        {
            if resolve_symlink(ffname, fname_buf.as_mut_ptr()) == OK {
                ffname = fname_buf.as_ptr();
            }
        }

        let mut dir_name = [0 as c_char; MAXPATHL + 1];
        let mut munged_vec: Vec<c_char> = Vec::new();
        let mut undo_file_name: Option<std::ffi::CString> = None;

        let ffname_len = std::ffi::CStr::from_ptr(ffname).to_bytes().len();
        let mut dirp = opt_vars().p_udir;
        while *dirp != NUL {
            let mut dir_len = copy_option_part(
                &raw mut dirp,
                dir_name.as_mut_ptr(),
                MAXPATHL,
                c",".as_ptr(),
            );
            if dir_len == 1 && dir_name[0] == b'.' as c_char {
                let mut ufn_vec: Vec<c_char> = vec![0; ffname_len + 6];
                let ufn = ufn_vec.as_mut_ptr();
                ptr::copy(ffname, ufn, ffname_len + 1);
                let tail = path_tail(ufn);
                let tail_len = ffname_len - (tail.offset_from(ufn) as usize);
                ptr::copy(tail, tail.add(1), tail_len + 1);
                *tail = b'.' as c_char;
                ptr::copy(c".un~".as_ptr(), tail.add(tail_len + 1), 5);
                undo_file_name = Some(std::ffi::CStr::from_ptr(ufn).to_owned());
            } else {
                dir_name[dir_len] = NUL;

                while dir_len > 1 && vim_ispathsep_nocolon(dir_name[dir_len - 1] as c_int) {
                    dir_len -= 1;
                    dir_name[dir_len] = NUL;
                }

                let mut has_directory = os_isdir(dir_name.as_ptr());
                if !has_directory && *dirp == NUL && !reading {
                    let mut failed_dir: Option<std::ffi::CString> = None;
                    let ret = os_mkdir_recurse(dir_name.as_ptr(), 0o755, &mut failed_dir, None);
                    if ret != 0 {
                        semsg!(
                            gettext(
                                c"E5003: Unable to create directory \"%s\" for undo file: %s"
                                    .as_ptr(),
                            ),
                            failed_dir.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                            uv_strerror(ret),
                        );
                    } else {
                        has_directory = true;
                    }
                }
                if has_directory {
                    if munged_vec.is_empty() {
                        munged_vec = vec![0; ffname_len + 1];
                        ptr::copy_nonoverlapping(ffname, munged_vec.as_mut_ptr(), ffname_len);
                        let mut p = munged_vec.as_mut_ptr();
                        while *p != NUL {
                            if vim_ispathsep(*p as c_int) {
                                *p = b'%' as c_char;
                            }
                            p = p.add(utfc_ptr2len(p) as usize);
                        }
                    }
                    let dir_str = NvimString {
                        data: dir_name.as_mut_ptr(),
                        size: dir_len,
                    };
                    let munged_name = NvimString {
                        data: munged_vec.as_mut_ptr(),
                        size: ffname_len,
                    };
                    undo_file_name = Some(concat_fnames(dir_str, munged_name, true));
                }
            }

            if let Some(ref ufn) = undo_file_name
                && (!reading || os_path_exists(ufn.as_ptr()))
            {
                break;
            }
            undo_file_name = None;
        }

        undo_file_name
    }
}

fn corruption_error(mesg: *const c_char, file_name: *const c_char) {
    semsg!(
            gettext(c"E825: Corrupted undo file (%s): %s".as_ptr()),
            mesg,
            file_name,
        );
}

fn serialize_header(bi: &mut bufinfo_T, hash: *mut u8) -> bool {
    unsafe {
        let buf = bi.bi_buf;
        let fp = bi.bi_fp.as_ptr();

        if fwrite(UF_START_MAGIC.as_ptr().cast(), UF_START_MAGIC_LEN, 1, fp) != 1 {
            return false;
        }

        undo_write_bytes(bi, UF_VERSION as u64, 2);

        if !undo_write(bi, hash, UNDO_HASH_SIZE) {
            return false;
        }

        undo_write_bytes(bi, (*buf).b_ml.ml_line_count as u64, 4);
        let len = (*buf)
            .b_u_line_ptr
            .as_ref()
            .map_or(0, |s| s.as_bytes().len());
        undo_write_bytes(bi, len as u64, 4);
        if len > 0
            && !undo_write(
                bi,
                (*buf)
                    .b_u_line_ptr
                    .as_ref()
                    .unwrap()
                    .as_ptr()
                    .cast_mut()
                    .cast(),
                len,
            )
        {
            return false;
        }
        undo_write_bytes(bi, (*buf).b_u_line_lnum as u64, 4);
        undo_write_bytes(bi, (*buf).b_u_line_colnr as u64, 4);

        put_header_ptr(bi, (*buf).b_u_oldhead);
        put_header_ptr(bi, (*buf).b_u_newhead);
        put_header_ptr(bi, (*buf).b_u_curhead);

        undo_write_bytes(bi, (*buf).b_u_numhead as u64, 4);
        undo_write_bytes(bi, (*buf).b_u_seq_last as u64, 4);
        undo_write_bytes(bi, (*buf).b_u_seq_cur as u64, 4);
        let mut time_buf = ((*buf).b_u_time_cur as u64).to_be_bytes();
        undo_write(bi, time_buf.as_mut_ptr(), time_buf.len());

        undo_write_bytes(bi, 4, 1);
        undo_write_bytes(bi, UF_LAST_SAVE_NR as u64, 1);
        undo_write_bytes(bi, (*buf).b_u_save_nr_last as u64, 4);

        undo_write_bytes(bi, 0, 1);

        true
    }
}

fn serialize_uhp(bi: &mut bufinfo_T, uhp: usize) -> bool {
    unsafe {
        if !undo_write_bytes(bi, UF_HEADER_MAGIC as u64, 2) {
            return false;
        }

        let buf = bi.bi_buf;
        put_header_ptr(bi, uhp_ref(&*buf, uhp).uh_next);
        put_header_ptr(bi, uhp_ref(&*buf, uhp).uh_prev);
        put_header_ptr(bi, uhp_ref(&*buf, uhp).uh_alt_next);
        put_header_ptr(bi, uhp_ref(&*buf, uhp).uh_alt_prev);
        undo_write_bytes(bi, uhp_ref(&*buf, uhp).uh_seq as u64, 4);
        serialize_pos(bi, uhp_ref(&*buf, uhp).uh_cursor);
        undo_write_bytes(bi, uhp_ref(&*buf, uhp).uh_cursor_vcol as u64, 4);
        undo_write_bytes(bi, uhp_ref(&*buf, uhp).uh_flags as u64, 2);
        for i in 0..NMARKS {
            serialize_pos(bi, uhp_ref(&*buf, uhp).uh_namedm[i].mark);
        }
        serialize_visualinfo(bi, &mut uhp_mut(&mut *buf, uhp).uh_visual);
        let mut time_buf = (uhp_ref(&*buf, uhp).uh_time as u64).to_be_bytes();
        undo_write(bi, time_buf.as_mut_ptr(), time_buf.len());

        undo_write_bytes(bi, 4, 1);
        undo_write_bytes(bi, UHP_SAVE_NR as u64, 1);
        undo_write_bytes(bi, uhp_ref(&*buf, uhp).uh_save_nr as u64, 4);

        undo_write_bytes(bi, 0, 1);

        let entries_len = uhp_ref(&*buf, uhp).uh_entries.len();
        for i in 0..entries_len {
            undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
            let uep = &uhp_ref(&*buf, uhp).uh_entries[i];
            if !serialize_uep(bi, uep) {
                return false;
            }
        }
        undo_write_bytes(bi, UF_ENTRY_END_MAGIC as u64, 2);

        let extmark_len = uhp_ref(&*buf, uhp).uh_extmark.len();
        for i in 0..extmark_len {
            let item = uhp_ref(&*buf, uhp).uh_extmark[i];
            if !serialize_extmark(bi, item) {
                return false;
            }
        }
        undo_write_bytes(bi, UF_ENTRY_END_MAGIC as u64, 2);

        true
    }
}

fn unserialize_uhp(
    bi: &mut bufinfo_T,
    file_name: *const c_char,
) -> Option<(u_header_T, [c_int; 4])> {
    let mut uhp = u_header_T::new();
        let seqs = [
            undo_read_4c(bi),
            undo_read_4c(bi),
            undo_read_4c(bi),
            undo_read_4c(bi),
        ];
        uhp.uh_seq = undo_read_4c(bi);
        if uhp.uh_seq <= 0 {
            corruption_error(c"uh_seq".as_ptr(), file_name);
            return None;
        }
        unserialize_pos(bi, &mut uhp.uh_cursor);
        uhp.uh_cursor_vcol = undo_read_4c(bi);
        uhp.uh_flags = undo_read_2c(bi);
        let cur_timestamp: Timestamp = os_time();
        for i in 0..NMARKS {
            unserialize_pos(bi, &mut uhp.uh_namedm[i].mark);
            uhp.uh_namedm[i].timestamp = cur_timestamp;
            uhp.uh_namedm[i].fnum = 0;
        }
        unserialize_visualinfo(bi, &mut uhp.uh_visual);
        uhp.uh_time = undo_read_time(bi);

        loop {
            let mut len = undo_read_byte(bi);
            if len == EOF {
                corruption_error(c"truncated".as_ptr(), file_name);
                return None;
            }
            if len == 0 {
                break;
            }
            let what = undo_read_byte(bi);
            match what {
                UHP_SAVE_NR => {
                    uhp.uh_save_nr = undo_read_4c(bi);
                }
                _ => {
                    while {
                        len -= 1;
                        len >= 0
                    } {
                        undo_read_byte(bi);
                    }
                }
            }
        }

        let mut c = undo_read_2c(bi);
        while c == UF_ENTRY_MAGIC {
            let mut error = false;
            let uep = unserialize_uep(bi, &mut error, file_name);
            uhp.uh_entries.push(uep);
            if error {
                return None;
            }
            c = undo_read_2c(bi);
        }
        if c != UF_ENTRY_END_MAGIC {
            corruption_error(c"entry end".as_ptr(), file_name);
            return None;
        }

        c = undo_read_2c(bi);
        while c == UF_ENTRY_MAGIC {
            let extup = unserialize_extmark(bi, file_name)?;
            uhp.uh_extmark.push(extup);
            c = undo_read_2c(bi);
        }
        if c != UF_ENTRY_END_MAGIC {
            corruption_error(c"entry end".as_ptr(), file_name);
            return None;
        }

        Some((uhp, seqs))
}

fn serialize_extmark(bi: &mut bufinfo_T, extup: ExtmarkUndoObject) -> bool {
    match extup {
            ExtmarkUndoObject::Splice(splice) => {
                undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
                undo_write_bytes(bi, kExtmarkSplice as u64, 4);
                if !undo_write(
                    bi,
                    (&raw const splice).cast::<u8>().cast_mut(),
                    size_of::<ExtmarkSplice>(),
                ) {
                    return false;
                }
            }
            ExtmarkUndoObject::Move(move_) => {
                undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
                undo_write_bytes(bi, kExtmarkMove as u64, 4);
                if !undo_write(
                    bi,
                    (&raw const move_).cast::<u8>().cast_mut(),
                    size_of::<ExtmarkMove>(),
                ) {
                    return false;
                }
            }
            ExtmarkUndoObject::SavePos(_) => {}
        }
        true
}

fn unserialize_extmark(
    bi: &mut bufinfo_T,
    _filename: *const c_char,
) -> Option<ExtmarkUndoObject> {
    let type_ = undo_read_4c(bi) as UndoObjectType;
        if type_ == kExtmarkSplice {
            let mut splice = ExtmarkSplice::new();
            if !undo_read(
                bi,
                (&raw mut splice).cast::<u8>(),
                size_of::<ExtmarkSplice>(),
            ) {
                return None;
            }
            Some(ExtmarkUndoObject::Splice(splice))
        } else if type_ == kExtmarkMove {
            let mut move_ = ExtmarkMove::new();
            if !undo_read(bi, (&raw mut move_).cast::<u8>(), size_of::<ExtmarkMove>()) {
                return None;
            }
            Some(ExtmarkUndoObject::Move(move_))
        } else {
            None
        }
}

fn serialize_uep(bi: &mut bufinfo_T, uep: &u_entry_T) -> bool {
    undo_write_bytes(bi, uep.ue_top as u64, 4);
        undo_write_bytes(bi, uep.ue_bot as u64, 4);
        undo_write_bytes(bi, uep.ue_lcount as u64, 4);
        undo_write_bytes(bi, uep.ue_size() as u64, 4);

        for line in &uep.ue_array {
            let len = line.as_bytes().len();
            if !undo_write_bytes(bi, len as u64, 4) {
                return false;
            }
            if len > 0 && !undo_write(bi, line.as_ptr().cast_mut().cast::<u8>(), len) {
                return false;
            }
        }
        true
}

fn unserialize_uep(
    bi: &mut bufinfo_T,
    error: &mut bool,
    file_name: *const c_char,
) -> u_entry_T {
    let mut uep = u_entry_T {
            ue_top: undo_read_4c(bi),
            ue_bot: undo_read_4c(bi),
            ue_lcount: undo_read_4c(bi),
            ue_array: Vec::new(),
        };
        if uep.ue_top < 0 || uep.ue_bot < 0 || uep.ue_lcount < 0 {
            corruption_error(c"entry lnum".as_ptr(), file_name);
            *error = true;
            return uep;
        }

        let size = undo_read_len(bi, c"entry size".as_ptr(), file_name);
        if size < 0 {
            *error = true;
            return uep;
        }

        if size > 0 {
            uep.ue_array.reserve(size as usize);
        }

        for _ in 0..size as usize {
            let line_len = undo_read_len(bi, c"line length".as_ptr(), file_name);
            let Some(line) = (line_len >= 0)
                .then(|| undo_read_string(bi, line_len as usize))
                .flatten()
            else {
                *error = true;
                return uep;
            };
            uep.ue_array.push(line);
        }
        uep
}

fn serialize_pos(bi: &mut bufinfo_T, pos: pos_T) {
    undo_write_bytes(bi, pos.lnum as u64, 4);
        undo_write_bytes(bi, pos.col as u64, 4);
        undo_write_bytes(bi, pos.coladd as u64, 4);
}

fn unserialize_pos(bi: &mut bufinfo_T, pos: &mut pos_T) {
    pos.lnum = undo_read_4c(bi).max(0);
        pos.col = undo_read_4c(bi).max(0);
        pos.coladd = undo_read_4c(bi).max(0);
}

fn serialize_visualinfo(bi: &mut bufinfo_T, info: &mut visualinfo_T) {
    serialize_pos(bi, info.vi_start);
        serialize_pos(bi, info.vi_end);
        undo_write_bytes(bi, info.vi_mode as u64, 4);
        undo_write_bytes(bi, info.vi_curswant as u64, 4);
}

fn unserialize_visualinfo(bi: &mut bufinfo_T, info: &mut visualinfo_T) {
    unserialize_pos(bi, &mut info.vi_start);
        unserialize_pos(bi, &mut info.vi_end);
        info.vi_mode = undo_read_4c(bi);
        info.vi_curswant = undo_read_4c(bi);
}

/// Write the undo tree in an undo file.
pub fn u_write_undo(name: *const c_char, forceit: bool, buf: *mut buf_T, hash: *mut u8) {
    unsafe {
        let f_owned: Option<std::ffi::CString>;
        let file_name = if name.is_null() {
            f_owned = u_get_undo_file_name((*buf).b_ffname, false);
            let Some(ref f) = f_owned else {
                if opt_vars().p_verbose > 0 {
                    verbose_enter();
                    smsg!(
                        0,
                        c"%s".as_ptr(),
                        gettext(c"Cannot write undo file in any directory in 'undodir'".as_ptr()),
                    );
                    verbose_leave();
                }
                return;
            };
            f.as_ptr().cast_mut()
        } else {
            name.cast_mut()
        };

        let mut perm: c_int = 0o600;
        if !(*buf).b_ffname.is_null() {
            perm = os_getperm((*buf).b_ffname);
            if perm < 0 {
                perm = 0o600;
            }
        }
        perm &= 0o666;

        'theend: {
            if os_path_exists(file_name) {
                if name.is_null() || !forceit {
                    let fd = os_open(file_name, O_RDONLY, 0);
                    if fd < 0 {
                        if !name.is_null() || opt_vars().p_verbose > 0 {
                            if name.is_null() {
                                verbose_enter();
                            }
                            smsg!(
                                0,
                                gettext(
                                    c"Will not overwrite with undo file, cannot read: %s".as_ptr(),
                                ),
                                file_name,
                            );
                            if name.is_null() {
                                verbose_leave();
                            }
                        }
                        break 'theend;
                    } else {
                        let mut mbuf = [0u8; UF_START_MAGIC_LEN];
                        let len = read_eintr(fd, mbuf.as_mut_ptr().cast(), UF_START_MAGIC_LEN);
                        os_close(fd);
                        if len < UF_START_MAGIC_LEN as isize
                            || mbuf != UF_START_MAGIC[..UF_START_MAGIC_LEN]
                        {
                            if !name.is_null() || opt_vars().p_verbose > 0 {
                                if name.is_null() {
                                    verbose_enter();
                                }
                                smsg!(
                                    0,
                                    gettext(
                                        c"Will not overwrite, this is not an undo file: %s"
                                            .as_ptr(),
                                    ),
                                    file_name,
                                );
                                if name.is_null() {
                                    verbose_leave();
                                }
                            }
                            break 'theend;
                        }
                    }
                }
                os_remove(file_name);
            }

            if (*buf).b_u_numhead == 0 && (*buf).b_u_line_ptr.is_none() {
                if opt_vars().p_verbose > 0 {
                    verb_msg(gettext(
                        c"Skipping undo file write, nothing to undo".as_ptr(),
                    ));
                }
                break 'theend;
            }

            let fd = os_open(file_name, O_CREAT | O_WRONLY | O_EXCL | O_NOFOLLOW, perm);
            if fd < 0 {
                semsg!(gettext(E_NOT_OPEN.as_ptr()), file_name);
                break 'theend;
            }
            os_setperm(file_name, perm);
            if opt_vars().p_verbose > 0 {
                verbose_enter();
                smsg!(0, gettext(c"Writing undo file: %s".as_ptr()), file_name);
                verbose_leave();
            }

            #[cfg(unix)]
            {
                let mut file_info_old = FileInfo::new();
                let mut file_info_new = FileInfo::new();
                if !(*buf).b_ffname.is_null()
                    && os_fileinfo((*buf).b_ffname, &mut file_info_old)
                    && os_fileinfo(file_name, &mut file_info_new)
                    && file_info_old.stat.st_gid != file_info_new.stat.st_gid
                    && os_fchown(
                        fd,
                        u32::MAX as uv_uid_t,
                        file_info_old.stat.st_gid as uv_gid_t,
                    ) != 0
                {
                    os_setperm(file_name, (perm & 0o707) | ((perm & 0o7) << 3));
                }
            }

            let Some(bi_fp) = crate::os::fs::CFile::from_raw(fdopen(fd, c"w".as_ptr())) else {
                semsg!(gettext(E_NOT_OPEN.as_ptr()), file_name);
                os_close(fd);
                os_remove(file_name);
                break 'theend;
            };

            u_sync(true);

            let mut write_ok = false;
            let mut bi = bufinfo_T {
                bi_buf: buf,
                bi_fp,
                bi_fsize: 0,
            };

            'write_error: {
                if !serialize_header(&mut bi, hash) {
                    break 'write_error;
                }

                undo_st().lastmark += 1;
                let mark = undo_st().lastmark;
                let mut uhp = (*buf).b_u_oldhead;
                while let Some(uh) = uhp {
                    if uhp_ref(&*buf, uh).uh_walk != mark {
                        uhp_mut(&mut *buf, uh).uh_walk = mark;
                        if !serialize_uhp(&mut bi, uh) {
                            break 'write_error;
                        }
                    }

                    let prev = uhp_ref(&*buf, uh).uh_prev;
                    let alt_next = uhp_ref(&*buf, uh).uh_alt_next;
                    let next = uhp_ref(&*buf, uh).uh_next;
                    let alt_prev = uhp_ref(&*buf, uh).uh_alt_prev;
                    if let Some(p) = prev
                        && uhp_ref(&*buf, p).uh_walk != mark
                    {
                        uhp = Some(p);
                    } else if let Some(an) = alt_next
                        && uhp_ref(&*buf, an).uh_walk != mark
                    {
                        uhp = Some(an);
                    } else if let Some(n) = next
                        && alt_prev.is_none()
                        && uhp_ref(&*buf, n).uh_walk != mark
                    {
                        uhp = Some(n);
                    } else if let Some(ap) = alt_prev {
                        uhp = Some(ap);
                    } else {
                        uhp = next;
                    }
                }

                if undo_write_bytes(&mut bi, UF_HEADER_END_MAGIC as u64, 2) {
                    write_ok = true;
                }

                let do_fsync = if (*buf).b_p_fs >= 0 {
                    (*buf).b_p_fs != 0
                } else {
                    opt_vars().p_fs != 0
                };
                if do_fsync && fflush(bi.bi_fp.as_ptr()) == 0 && os_fsync(fd) != 0 {
                    write_ok = false;
                }
            }

            drop(bi);
            if !write_ok {
                semsg!(gettext(E_WRITE_ERROR_IN_UNDO_FILE_STR.as_ptr()), file_name);
            }

            if !(*buf).b_ffname.is_null() {
                let acl: vim_acl_T = os_get_acl((*buf).b_ffname);
                os_set_acl(file_name, acl);
                os_free_acl(acl);
            }
        }
    }
}

fn uhp_table_find(uhp_table: &[(u_header_T, [c_int; 4])], seq: c_int) -> c_int {
    let mut lo: c_int = 0;
    let mut hi: c_int = uhp_table.len() as c_int - 1;

    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let mid_seq = uhp_table[mid as usize].0.uh_seq;
        if mid_seq < seq {
            lo = mid + 1;
        } else if mid_seq > seq {
            hi = mid - 1;
        } else {
            return mid;
        }
    }
    -1
}

/// Loads the undo tree from an undo file.
pub fn u_read_undo(name: *mut c_char, hash: *const u8, orig_name: *const c_char) {
    unsafe {
        #[cfg(not(unix))]
        let _ = orig_name;
        let mut uhp_table: Vec<(u_header_T, [c_int; 4])> = Vec::new();
        let mut line_ptr: Option<std::ffi::CString> = None;
        let mut num_read_uhps: c_int = 0;

        let f_owned: Option<std::ffi::CString>;
        let file_name = if name.is_null() {
            f_owned = u_get_undo_file_name((*g_st().curbuf).b_ffname, true);
            let Some(ref f_cstr) = f_owned else {
                return;
            };
            let f = f_cstr.as_ptr().cast_mut();

            #[cfg(unix)]
            {
                let mut file_info_orig = FileInfo::new();
                let mut file_info_undo = FileInfo::new();
                if os_fileinfo(orig_name, &mut file_info_orig)
                    && os_fileinfo(f, &mut file_info_undo)
                    && file_info_orig.stat.st_uid != file_info_undo.stat.st_uid
                    && file_info_undo.stat.st_uid != u64::from(getuid())
                {
                    if opt_vars().p_verbose > 0 {
                        verbose_enter();
                        smsg!(
                            0,
                            gettext(c"Not reading undo file, owner differs: %s".as_ptr()),
                            f,
                        );
                        verbose_leave();
                    }
                    return;
                }
            }
            f
        } else {
            name
        };

        if opt_vars().p_verbose > 0 {
            verbose_enter();
            smsg!(0, gettext(c"Reading undo file: %s".as_ptr()), file_name);
            verbose_leave();
        }

        let fp_opt = crate::os::fs::CFile::from_raw(os_fopen(file_name, c"r".as_ptr()));
        'theend: {
            'error: {
                let Some(bi_fp) = fp_opt else {
                    if !name.is_null() || opt_vars().p_verbose > 0 {
                        semsg!(
                            gettext(c"E822: Cannot open undo file for reading: %s".as_ptr()),
                            file_name,
                        );
                    }
                    break 'error;
                };
                let fp = bi_fp.as_ptr();

                let mut file_info = FileInfo::new();
                let fsize = if os_fileinfo_fd(fileno(fp), &mut file_info) {
                    os_fileinfo_size(&file_info) as off_T
                } else {
                    0
                };
                let mut bi = bufinfo_T {
                    bi_buf: g_st().curbuf,
                    bi_fp,
                    bi_fsize: fsize,
                };

                let mut magic_buf = [0u8; UF_START_MAGIC_LEN];
                if fread(magic_buf.as_mut_ptr().cast(), UF_START_MAGIC_LEN, 1, fp) != 1
                    || magic_buf != UF_START_MAGIC[..UF_START_MAGIC_LEN]
                {
                    semsg!(gettext(c"E823: Not an undo file: %s".as_ptr()), file_name);
                    break 'error;
                }
                let version = get2c(fp);
                if version != UF_VERSION {
                    semsg!(
                        gettext(c"E824: Incompatible undo file: %s".as_ptr()),
                        file_name,
                    );
                    break 'error;
                }

                let mut read_hash = [0u8; UNDO_HASH_SIZE];
                if !undo_read(&mut bi, read_hash.as_mut_ptr(), UNDO_HASH_SIZE) {
                    corruption_error(c"hash".as_ptr(), file_name);
                    break 'error;
                }
                let line_count = undo_read_4c(&mut bi) as linenr_T;
                if std::slice::from_raw_parts(hash, UNDO_HASH_SIZE) != read_hash
                    || line_count != (*g_st().curbuf).b_ml.ml_line_count
                {
                    if opt_vars().p_verbose > 0 || !name.is_null() {
                        if name.is_null() {
                            verbose_enter();
                        }
                        give_warning(
                            gettext(c"File contents changed, cannot use undo info".as_ptr()),
                            true,
                            true,
                        );
                        if name.is_null() {
                            verbose_leave();
                        }
                    }
                    break 'error;
                }

                let str_len = undo_read_len(&mut bi, c"line length".as_ptr(), file_name);
                if str_len < 0 {
                    break 'error;
                }

                if str_len > 0 {
                    let Some(s) = undo_read_string(&mut bi, str_len as usize) else {
                        corruption_error(c"truncated".as_ptr(), file_name);
                        break 'error;
                    };
                    line_ptr = Some(s);
                }
                let line_lnum = undo_read_4c(&mut bi) as linenr_T;
                let line_colnr = undo_read_4c(&mut bi) as colnr_T;
                if line_lnum < 0 || line_colnr < 0 {
                    corruption_error(c"line lnum/col".as_ptr(), file_name);
                    break 'error;
                }

                let old_header_seq = undo_read_4c(&mut bi);
                let new_header_seq = undo_read_4c(&mut bi);
                let cur_header_seq = undo_read_4c(&mut bi);
                let num_head = undo_read_len(&mut bi, c"num_head".as_ptr(), file_name);
                if num_head < 0 {
                    break 'error;
                }
                let seq_last = undo_read_4c(&mut bi);
                let seq_cur = undo_read_4c(&mut bi);
                let seq_time = undo_read_time(&mut bi);

                let mut last_save_nr: c_int = 0;
                loop {
                    let mut len = undo_read_byte(&mut bi);
                    if len == 0 || len == EOF {
                        break;
                    }
                    let what = undo_read_byte(&mut bi);
                    match what {
                        UF_LAST_SAVE_NR => {
                            last_save_nr = undo_read_4c(&mut bi);
                        }
                        _ => {
                            while {
                                len -= 1;
                                len >= 0
                            } {
                                undo_read_byte(&mut bi);
                            }
                        }
                    }
                }

                if num_head > 0 {
                    uhp_table.reserve(num_head as usize);
                }

                let mut c = undo_read_2c(&mut bi);
                while c == UF_HEADER_MAGIC {
                    if num_read_uhps >= num_head {
                        corruption_error(c"num_head too small".as_ptr(), file_name);
                        break 'error;
                    }

                    let Some(entry) = unserialize_uhp(&mut bi, file_name) else {
                        break 'error;
                    };
                    uhp_table.push(entry);
                    num_read_uhps += 1;
                    c = undo_read_2c(&mut bi);
                }

                if num_read_uhps != num_head {
                    corruption_error(c"num_head".as_ptr(), file_name);
                    break 'error;
                }
                if c != UF_HEADER_END_MAGIC {
                    corruption_error(c"end marker".as_ptr(), file_name);
                    break 'error;
                }

                if num_head > 0 {
                    uhp_table.sort_unstable_by_key(|(u, _)| u.uh_seq);
                }

                for i in 0..(num_head - 1) {
                    if uhp_table[i as usize].0.uh_seq == uhp_table[(i + 1) as usize].0.uh_seq {
                        corruption_error(c"duplicate uh_seq".as_ptr(), file_name);
                        break 'error;
                    }
                }

                let mut old_idx: Option<usize> = None;
                let mut new_idx: Option<usize> = None;
                let mut cur_idx: Option<usize> = None;
                for i in 0..num_head {
                    let [next_seq, prev_seq, alt_next_seq, alt_prev_seq] = uhp_table[i as usize].1;

                    let fidx_next = uhp_table_find(&uhp_table, next_seq);
                    let uh_next = (fidx_next >= 0 && fidx_next != i).then_some(fidx_next as usize);

                    let fidx_prev = uhp_table_find(&uhp_table, prev_seq);
                    let uh_prev = (fidx_prev >= 0 && fidx_prev != i).then_some(fidx_prev as usize);

                    let fidx_anext = uhp_table_find(&uhp_table, alt_next_seq);
                    let uh_alt_next =
                        (fidx_anext >= 0 && fidx_anext != i).then_some(fidx_anext as usize);

                    let fidx_aprev = uhp_table_find(&uhp_table, alt_prev_seq);
                    let uh_alt_prev =
                        (fidx_aprev >= 0 && fidx_aprev != i).then_some(fidx_aprev as usize);

                    let uhp = &mut uhp_table[i as usize].0;
                    uhp.uh_next = uh_next;
                    uhp.uh_prev = uh_prev;
                    uhp.uh_alt_next = uh_alt_next;
                    uhp.uh_alt_prev = uh_alt_prev;

                    if old_header_seq > 0 && old_idx.is_none() && uhp.uh_seq == old_header_seq {
                        old_idx = Some(i as usize);
                    }
                    if new_header_seq > 0 && new_idx.is_none() && uhp.uh_seq == new_header_seq {
                        new_idx = Some(i as usize);
                    }
                    if cur_header_seq > 0 && cur_idx.is_none() && uhp.uh_seq == cur_header_seq {
                        cur_idx = Some(i as usize);
                    }
                }

                u_blockfree(g_st().curbuf);
                (*g_st().curbuf).b_u_headers = uhp_table.into_iter().map(|(u, _)| u).collect();
                (*g_st().curbuf).b_u_free_headers.clear();
                (*g_st().curbuf).b_u_oldhead = old_idx;
                (*g_st().curbuf).b_u_newhead = new_idx;
                (*g_st().curbuf).b_u_curhead = cur_idx;
                (*g_st().curbuf).b_u_line_ptr = line_ptr;
                (*g_st().curbuf).b_u_line_lnum = line_lnum;
                (*g_st().curbuf).b_u_line_colnr = line_colnr;
                (*g_st().curbuf).b_u_numhead = num_head;
                (*g_st().curbuf).b_u_seq_last = seq_last;
                (*g_st().curbuf).b_u_seq_cur = seq_cur;
                (*g_st().curbuf).b_u_time_cur = seq_time;
                (*g_st().curbuf).b_u_save_nr_last = last_save_nr;
                (*g_st().curbuf).b_u_save_nr_cur = last_save_nr;

                (*g_st().curbuf).b_u_synced = true;

                if !name.is_null() {
                    smsg!(
                        0,
                        gettext(c"Finished reading undo file %s".as_ptr()),
                        file_name,
                    );
                }
                break 'theend;
            }

            drop(line_ptr);
            drop(uhp_table);
        }
    }
}

fn undo_write(bi: &mut bufinfo_T, ptr: *mut u8, len: usize) -> bool {
    unsafe { fwrite(ptr.cast(), len, 1, bi.bi_fp.as_ptr()) == 1 }
}

fn undo_write_bytes(bi: &mut bufinfo_T, nr: u64, len: usize) -> bool {
    assert!(len > 0);
        let mut buf = [0u8; 8];
        let mut i = len - 1;
        for slot in buf.iter_mut().take(len) {
            *slot = (nr >> (i * 8)) as u8;
            i = i.wrapping_sub(1);
        }
        undo_write(bi, buf.as_mut_ptr(), len)
}

fn put_header_ptr(bi: &mut bufinfo_T, uhp: Option<usize>) {
    unsafe {
        let seq = if let Some(uh) = uhp {
            let s = uhp_ref(&*bi.bi_buf, uh).uh_seq;
            assert!(s >= 0);
            s as u64
        } else {
            0
        };
        undo_write_bytes(bi, seq, 4);
    }
}

fn undo_read_4c(bi: &mut bufinfo_T) -> c_int {
    get4c(bi.bi_fp.as_ptr())
}

fn undo_read_len(
    bi: &mut bufinfo_T,
    what: *const c_char,
    file_name: *const c_char,
) -> c_int {
    unsafe {
        let len = undo_read_4c(bi);
        let fp = bi.bi_fp.as_ptr();
        let pos = vim_ftell(fp);
        if len < 0 || (bi.bi_fsize > 0 && (pos < 0 || (len as off_T) > bi.bi_fsize - pos)) {
            corruption_error(
                if feof(fp) != 0 {
                    c"truncated".as_ptr()
                } else {
                    what
                },
                file_name,
            );
            return -1;
        }
        len
    }
}

fn undo_read_2c(bi: &mut bufinfo_T) -> c_int {
    get2c(bi.bi_fp.as_ptr())
}

fn undo_read_byte(bi: &mut bufinfo_T) -> c_int {
    unsafe { fgetc(bi.bi_fp.as_ptr()) }
}

fn undo_read_time(bi: &mut bufinfo_T) -> time_t {
    get8ctime(bi.bi_fp.as_ptr())
}

fn undo_read(bi: &mut bufinfo_T, buffer: *mut u8, size: usize) -> bool {
    unsafe {
        let retval = fread(buffer.cast(), size, 1, bi.bi_fp.as_ptr()) == 1;
        if !retval {
            ptr::write_bytes(buffer, 0, size);
        }
        retval
    }
}

fn undo_read_string(bi: &mut bufinfo_T, len: usize) -> Option<std::ffi::CString> {
    if len == 0 {
            return Some(std::ffi::CString::default());
        }
        let mut buf = vec![0u8; len];
        if !undo_read(bi, buf.as_mut_ptr(), len) {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(len);
        buf.truncate(end);
        std::ffi::CString::new(buf).ok()
}

/// Undo `count` times.
pub fn u_undo(mut count: c_int) {
    unsafe {
        if !(*g_st().curbuf).b_u_synced {
            u_sync(true);
            count = 1;
        }

        if vim_strchr(opt_vars().p_cpo, kCpoUndo).is_null() {
            undo_st().undo_undoes = true;
        } else {
            undo_st().undo_undoes = !undo_st().undo_undoes;
        }
        u_doit(count, false, true);
    }
}

/// Redo `count` times.
pub fn u_redo(count: c_int) {
    if vim_strchr(opt_vars().p_cpo, kCpoUndo).is_null() {
            undo_st().undo_undoes = false;
        }

        u_doit(count, false, true);
}

/// Undo and remove the branch from the undo tree.
pub fn u_undo_and_forget(mut count: c_int, do_buf_event: bool) -> bool {
    unsafe {
        if !(*g_st().curbuf).b_u_synced {
            u_sync(true);
            count = 1;
        }
        undo_st().undo_undoes = true;
        u_doit(count, true, do_buf_event);

        let Some(curhead) = (*g_st().curbuf).b_u_curhead else {
            return false;
        };

        u_forget_header(&mut *g_st().curbuf, curhead);
        true
    }
}

fn u_doit(startcount: c_int, quiet: bool, do_buf_event: bool) {
    unsafe {
        if !undo_allowed(g_st().curbuf) {
            return;
        }
        atom_did_global_op();

        undo_st().u_newcount = 0;
        undo_st().u_oldcount = 0;
        if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            undo_st().u_oldcount = -1;
        }

        msg_ext_set_kind(c"undo".as_ptr());
        let mut count = startcount;
        while {
            let prev = count;
            count -= 1;
            prev != 0
        } {
            change_warning(g_st().curbuf, 0);

            if undo_st().undo_undoes {
                if (*g_st().curbuf).b_u_curhead.is_none() {
                    (*g_st().curbuf).b_u_curhead = (*g_st().curbuf).b_u_newhead;
                } else if get_undolevel(g_st().curbuf) > 0 {
                    let ch = (*g_st().curbuf).b_u_curhead.unwrap();
                    (*g_st().curbuf).b_u_curhead = uhp_ref(&*g_st().curbuf, ch).uh_next;
                }
                if (*g_st().curbuf).b_u_numhead == 0 || (*g_st().curbuf).b_u_curhead.is_none() {
                    (*g_st().curbuf).b_u_curhead = (*g_st().curbuf).b_u_oldhead;
                    beep_flush();
                    if count == startcount - 1 {
                        if !shortmess(kShmUndo) {
                            msg(gettext(c"Already at oldest change".as_ptr()), 0);
                        }
                        return;
                    }
                    break;
                }

                u_undoredo(true, do_buf_event);
            } else {
                if (*g_st().curbuf).b_u_curhead.is_none() || get_undolevel(g_st().curbuf) <= 0 {
                    beep_flush();
                    if count == startcount - 1 {
                        if !shortmess(kShmUndo) {
                            msg(gettext(c"Already at newest change".as_ptr()), 0);
                        }
                        return;
                    }
                    break;
                }

                u_undoredo(false, do_buf_event);

                let curhead = (*g_st().curbuf).b_u_curhead.unwrap();
                let prev = uhp_ref(&*g_st().curbuf, curhead).uh_prev;
                if prev.is_none() {
                    (*g_st().curbuf).b_u_newhead = Some(curhead);
                }
                (*g_st().curbuf).b_u_curhead = prev;
            }
        }
        u_undo_end(undo_st().undo_undoes, false, quiet);
    }
}

/// Undo or redo over the timeline.
pub fn undo_time(step: c_int, sec: bool, file: bool, absolute: bool) {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return;
        }
        mc_undo_time();

        if !(*g_st().curbuf).b_u_synced {
            u_sync(true);
        }

        undo_st().u_newcount = 0;
        undo_st().u_oldcount = 0;
        if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            undo_st().u_oldcount = -1;
        }

        let mut target: c_int;
        let mut closest: c_int;
        let mut uhp: Option<usize> = None;
        let mut dosec = sec;
        let mut dofile = file;
        let mut above = false;
        let mut did_undo = true;

        if absolute {
            target = step;
            closest = -1;
        } else {
            if dosec {
                target = ((*g_st().curbuf).b_u_time_cur as c_int) + step;
            } else if dofile {
                if step < 0 {
                    uhp = if let Some(ch) = (*g_st().curbuf).b_u_curhead {
                        uhp_ref(&*g_st().curbuf, ch).uh_next
                    } else {
                        (*g_st().curbuf).b_u_newhead
                    };
                    if uhp.is_some_and(|uh| uhp_ref(&*g_st().curbuf, uh).uh_save_nr != 0) {
                        target = (*g_st().curbuf).b_u_save_nr_cur + step;
                    } else {
                        target = (*g_st().curbuf).b_u_save_nr_cur + step + 1;
                    }
                    if target <= 0 {
                        dofile = false;
                    }
                } else {
                    target = (*g_st().curbuf).b_u_save_nr_cur + step;
                    if target > (*g_st().curbuf).b_u_save_nr_last {
                        target = (*g_st().curbuf).b_u_seq_last + 1;
                        dofile = false;
                    }
                }
            } else {
                target = (*g_st().curbuf).b_u_seq_cur + step;
            }
            if step < 0 {
                target = target.max(0);
                closest = -1;
            } else {
                if dosec {
                    closest = (os_time() + 1) as c_int;
                } else if dofile {
                    closest = (*g_st().curbuf).b_u_save_nr_last + 2;
                } else {
                    closest = (*g_st().curbuf).b_u_seq_last + 2;
                }
                if target >= closest {
                    target = closest - 1;
                }
            }
        }
        let closest_start = closest;
        let mut closest_seq = (*g_st().curbuf).b_u_seq_cur;
        let mut mark = undo_st().lastmark;
        let mut nomark: c_int = 0;

        if target != 0 {
            for round in 1..=2 {
                undo_st().lastmark += 1;
                mark = undo_st().lastmark;
                undo_st().lastmark += 1;
                nomark = undo_st().lastmark;

                uhp = (*g_st().curbuf).b_u_curhead.or((*g_st().curbuf).b_u_newhead);

                while let Some(uh) = uhp {
                    uhp_mut(&mut *g_st().curbuf, uh).uh_walk = mark;
                    let val = if dosec {
                        uhp_ref(&*g_st().curbuf, uh).uh_time as c_int
                    } else if dofile {
                        uhp_ref(&*g_st().curbuf, uh).uh_save_nr
                    } else {
                        uhp_ref(&*g_st().curbuf, uh).uh_seq
                    };

                    let uh_seq = uhp_ref(&*g_st().curbuf, uh).uh_seq;
                    if round == 1 && !(dofile && val == 0) {
                        let dir_ok = if step < 0 {
                            uh_seq <= (*g_st().curbuf).b_u_seq_cur
                        } else {
                            uh_seq > (*g_st().curbuf).b_u_seq_cur
                        };
                        let closer = if dosec && val == closest {
                            if step < 0 {
                                uh_seq < closest_seq
                            } else {
                                uh_seq > closest_seq
                            }
                        } else {
                            closest == closest_start
                                || if val > target {
                                    if closest > target {
                                        val - target <= closest - target
                                    } else {
                                        val - target <= target - closest
                                    }
                                } else if closest > target {
                                    target - val <= closest - target
                                } else {
                                    target - val <= target - closest
                                }
                        };
                        if dir_ok && closer {
                            closest = val;
                            closest_seq = uh_seq;
                        }
                    }

                    if target == val && !dosec {
                        target = uh_seq;
                        break;
                    }

                    let prev = uhp_ref(&*g_st().curbuf, uh).uh_prev;
                    let alt_next = uhp_ref(&*g_st().curbuf, uh).uh_alt_next;
                    let next = uhp_ref(&*g_st().curbuf, uh).uh_next;
                    let alt_prev = uhp_ref(&*g_st().curbuf, uh).uh_alt_prev;
                    if let Some(p) = prev
                        && uhp_ref(&*g_st().curbuf, p).uh_walk != nomark
                        && uhp_ref(&*g_st().curbuf, p).uh_walk != mark
                    {
                        uhp = Some(p);
                    } else if let Some(an) = alt_next
                        && uhp_ref(&*g_st().curbuf, an).uh_walk != nomark
                        && uhp_ref(&*g_st().curbuf, an).uh_walk != mark
                    {
                        uhp = Some(an);
                    } else if let Some(n) = next
                        && alt_prev.is_none()
                        && uhp_ref(&*g_st().curbuf, n).uh_walk != nomark
                        && uhp_ref(&*g_st().curbuf, n).uh_walk != mark
                    {
                        if Some(uh) == (*g_st().curbuf).b_u_curhead {
                            uhp_mut(&mut *g_st().curbuf, uh).uh_walk = nomark;
                        }
                        uhp = Some(n);
                    } else {
                        uhp_mut(&mut *g_st().curbuf, uh).uh_walk = nomark;
                        uhp = alt_prev.or(next);
                    }
                }

                if uhp.is_some() {
                    break;
                }

                if absolute {
                    semsg!(
                        gettext(c"E830: Undo number %lld not found".as_ptr()),
                        step as i64,
                    );
                    return;
                }

                if closest == closest_start {
                    if !shortmess(kShmUndo) {
                        if step < 0 {
                            msg(gettext(c"Already at oldest change".as_ptr()), 0);
                        } else {
                            msg(gettext(c"Already at newest change".as_ptr()), 0);
                        }
                    }
                    return;
                }

                target = closest_seq;
                dosec = false;
                dofile = false;
                if step < 0 {
                    above = true;
                }
            }
        }

        if uhp.is_some() || target == 0 {
            while !g_st().got_int {
                change_warning(g_st().curbuf, 0);

                uhp = if let Some(ch) = (*g_st().curbuf).b_u_curhead {
                    uhp_ref(&*g_st().curbuf, ch).uh_next
                } else {
                    (*g_st().curbuf).b_u_newhead
                };
                let Some(uh) = uhp else {
                    break;
                };
                if (target > 0 && uhp_ref(&*g_st().curbuf, uh).uh_walk != mark)
                    || (uhp_ref(&*g_st().curbuf, uh).uh_seq == target && !above)
                {
                    break;
                }
                (*g_st().curbuf).b_u_curhead = Some(uh);
                u_undoredo(true, true);
                if target > 0 {
                    uhp_mut(&mut *g_st().curbuf, uh).uh_walk = nomark;
                }
            }

            if target > 0 {
                while !g_st().got_int {
                    change_warning(g_st().curbuf, 0);

                    let Some(mut uh) = (*g_st().curbuf).b_u_curhead else {
                        break;
                    };

                    while let Some(ap) = uhp_ref(&*g_st().curbuf, uh).uh_alt_prev
                        && uhp_ref(&*g_st().curbuf, ap).uh_walk == mark
                    {
                        uh = ap;
                    }

                    let mut last = uh;
                    while let Some(an) = uhp_ref(&*g_st().curbuf, last).uh_alt_next
                        && uhp_ref(&*g_st().curbuf, an).uh_walk == mark
                    {
                        last = an;
                    }
                    if last != uh {
                        while let Some(ap) = uhp_ref(&*g_st().curbuf, uh).uh_alt_prev {
                            uh = ap;
                        }
                        let last_alt_next = uhp_ref(&*g_st().curbuf, last).uh_alt_next;
                        let last_alt_prev = uhp_ref(&*g_st().curbuf, last).uh_alt_prev;
                        if let Some(an) = last_alt_next {
                            uhp_mut(&mut *g_st().curbuf, an).uh_alt_prev = last_alt_prev;
                        }
                        if let Some(ap) = last_alt_prev {
                            uhp_mut(&mut *g_st().curbuf, ap).uh_alt_next = last_alt_next;
                        }
                        uhp_mut(&mut *g_st().curbuf, last).uh_alt_prev = None;
                        uhp_mut(&mut *g_st().curbuf, last).uh_alt_next = Some(uh);
                        uhp_mut(&mut *g_st().curbuf, uh).uh_alt_prev = Some(last);

                        if (*g_st().curbuf).b_u_oldhead == Some(uh) {
                            (*g_st().curbuf).b_u_oldhead = Some(last);
                        }
                        uh = last;
                        if let Some(n) = uhp_ref(&*g_st().curbuf, uh).uh_next {
                            uhp_mut(&mut *g_st().curbuf, n).uh_prev = Some(uh);
                        }
                    }
                    (*g_st().curbuf).b_u_curhead = Some(uh);

                    if uhp_ref(&*g_st().curbuf, uh).uh_walk != mark {
                        break;
                    }

                    if uhp_ref(&*g_st().curbuf, uh).uh_seq == target && above {
                        (*g_st().curbuf).b_u_seq_cur = target - 1;
                        break;
                    }

                    u_undoredo(false, true);

                    let prev = uhp_ref(&*g_st().curbuf, uh).uh_prev;
                    if prev.is_none() {
                        (*g_st().curbuf).b_u_newhead = Some(uh);
                    }
                    (*g_st().curbuf).b_u_curhead = prev;
                    did_undo = false;

                    if uhp_ref(&*g_st().curbuf, uh).uh_seq == target {
                        break;
                    }

                    uhp = prev;
                    if uhp.is_none_or(|p| uhp_ref(&*g_st().curbuf, p).uh_walk != mark) {
                        internal_error(c"undo_time()".as_ptr());
                        break;
                    }
                }
            }
        }
        u_undo_end(did_undo, absolute, false);
    }
}

fn u_undoredo(undo: bool, do_buf_event: bool) {
    unsafe {
        let mut newlnum: linenr_T = MAXLNUM;
        let mut new_curpos = (**curwin.as_ptr()).w_cursor;
        let curhead_idx = (*g_st().curbuf).b_u_curhead.unwrap();
        let curhead: *mut u_header_T = uhp_mut(&mut *g_st().curbuf, curhead_idx);

        block_autocmds();

        let old_flags = (*curhead).uh_flags;
        let new_flags = (if (*g_st().curbuf).b_changed != 0 {
            UH_CHANGED
        } else {
            0
        }) | (if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            UH_EMPTYBUF
        } else {
            0
        }) | (old_flags & UH_RELOAD);
        setpcmark();

        zero_fmark_additional_data(&mut *(&raw mut (*g_st().curbuf).b_namedm).cast::<fmark_T>());
        let namedm = (*g_st().curbuf).b_namedm.clone();
        let visualinfo = (*g_st().curbuf).b_visual;
        (*g_st().curbuf).b_op_start.lnum = (*g_st().curbuf).b_ml.ml_line_count;
        (*g_st().curbuf).b_op_start.col = 0;
        (*g_st().curbuf).b_op_end.lnum = 0;
        (*g_st().curbuf).b_op_end.col = 0;

        let entries_len = (*curhead).uh_entries.len();
        for entry_idx in 0..entries_len {
            let is_last = entry_idx + 1 == entries_len;
            let uep: *mut u_entry_T = &raw mut (*curhead).uh_entries.as_mut_slice()[entry_idx];
            let top = (*uep).ue_top;
            let mut bot = (*uep).ue_bot;
            if bot == 0 {
                bot = (*g_st().curbuf).b_ml.ml_line_count + 1;
            }
            if top > (*g_st().curbuf).b_ml.ml_line_count
                || top >= bot
                || bot > (*g_st().curbuf).b_ml.ml_line_count + 1
            {
                unblock_autocmds();
                iemsg(gettext(c"E438: u_undo: line numbers wrong".as_ptr()));
                changed(g_st().curbuf);
                return;
            }

            let oldsize = bot - top - 1;
            let newsize = (*uep).ue_size();

            {
                let lnum = (*curhead).uh_cursor.lnum;
                if lnum >= top && lnum <= top + newsize + 1 {
                    new_curpos = (*curhead).uh_cursor;
                    newlnum = -1;
                } else if top < newlnum {
                    let mut i: linenr_T = 0;
                    while i < newsize && i < oldsize {
                        if std::ffi::CStr::from_ptr((*uep).ue_array.as_slice()[i as usize].as_ptr()).to_bytes() != std::ffi::CStr::from_ptr(ml_get(top + 1 + i)).to_bytes()
                        {
                            break;
                        }
                        i += 1;
                    }
                    if i == newsize && newlnum == MAXLNUM && is_last {
                        newlnum = top;
                        new_curpos.lnum = newlnum + 1;
                    } else if i < newsize {
                        newlnum = top + i;
                        new_curpos.lnum = newlnum + 1;
                    }
                }
            }

            let mut empty_buffer = false;
            let mut newarray: Vec<std::ffi::CString> = Vec::new();

            if oldsize > 0 {
                newarray.resize_with(oldsize as usize, std::ffi::CString::default);
                let mut lnum = bot - 1;
                let mut i = oldsize;
                while {
                    i -= 1;
                    i >= 0
                } {
                    newarray[i as usize] = u_save_line(lnum);
                    if (*g_st().curbuf).b_ml.ml_line_count == 1 {
                        empty_buffer = true;
                    }
                    ml_delete(lnum);
                    lnum -= 1;
                }
            }

            check_cursor_lnum(&mut **curwin.as_ptr());

            if newsize != 0 {
                for (i, line) in (*uep).ue_array.drain(..).enumerate() {
                    let lnum = top + i as linenr_T;
                    let line_ptr = line.as_ptr().cast_mut();
                    if empty_buffer && lnum == 0 {
                        ml_replace(1, line_ptr, true);
                    } else {
                        ml_append_flags(lnum, line_ptr, 0, 0);
                    }
                }
            }

            if oldsize != newsize {
                mark_adjust(
                    top + 1,
                    top + oldsize,
                    MAXLNUM,
                    newsize - oldsize,
                    kExtmarkNOOP,
                );
                if (*g_st().curbuf).b_op_start.lnum > top + oldsize {
                    (*g_st().curbuf).b_op_start.lnum += newsize - oldsize;
                }
                if (*g_st().curbuf).b_op_end.lnum > top + oldsize {
                    (*g_st().curbuf).b_op_end.lnum += newsize - oldsize;
                }
            }

            if oldsize > 0 || newsize > 0 {
                changed_lines(g_st().curbuf, top + 1, 0, bot, newsize - oldsize, do_buf_event);
                if spell_check_window(&mut **curwin.as_ptr()) && bot <= (*g_st().curbuf).b_ml.ml_line_count {
                    redrawWinline(*curwin.as_ptr(), bot);
                }
            }

            (*g_st().curbuf).b_op_start.lnum = (*g_st().curbuf).b_op_start.lnum.min(top + 1);
            if newsize == 0 && top + 1 > (*g_st().curbuf).b_op_end.lnum {
                (*g_st().curbuf).b_op_end.lnum = top + 1;
            } else if top + newsize > (*g_st().curbuf).b_op_end.lnum {
                (*g_st().curbuf).b_op_end.lnum = top + newsize;
            }

            undo_st().u_newcount += newsize;
            undo_st().u_oldcount += oldsize;
            (*uep).ue_array = newarray;
            (*uep).ue_bot = top + newsize + 1;
        }

        (*g_st().curbuf).b_op_start.lnum = (*g_st().curbuf).b_op_start.lnum.min((*g_st().curbuf).b_ml.ml_line_count);
        (*g_st().curbuf).b_op_end.lnum = (*g_st().curbuf).b_op_end.lnum.min((*g_st().curbuf).b_ml.ml_line_count);

        if undo {
            for &item in (*curhead).uh_extmark.iter().rev() {
                extmark_apply_undo(item, undo);
            }
        } else {
            for &item in &(*curhead).uh_extmark {
                extmark_apply_undo(item, undo);
            }
        }
        if ((*curhead).uh_flags & UH_RELOAD) != 0 {
            buf_updates_unload(&mut *g_st().curbuf, true);
        }

        (**curwin.as_ptr()).w_cursor = new_curpos;
        check_cursor_lnum(&mut **curwin.as_ptr());

        (*curhead).uh_entries.reverse();
        (*curhead).uh_flags = new_flags;
        if (old_flags & UH_EMPTYBUF) != 0 && buf_is_empty(&mut *g_st().curbuf) {
            (*g_st().curbuf).b_ml.ml_flags |= ML_EMPTY;
        }
        if (old_flags & UH_CHANGED) != 0 {
            changed(g_st().curbuf);
        } else {
            unchanged(g_st().curbuf, false, true);
        }

        if do_buf_event {
            buf_updates_changedtick(&mut *g_st().curbuf);
        }

        for (i, nm) in namedm.into_iter().enumerate().take(NMARKS) {
            if (*curhead).uh_namedm[i].mark.lnum != 0 {
                (*g_st().curbuf).b_namedm[i] = (*curhead).uh_namedm[i].clone();
            }
            if nm.mark.lnum != 0 {
                (*curhead).uh_namedm[i] = nm;
            } else {
                (*curhead).uh_namedm[i].mark.lnum = 0;
            }
        }
        if (*curhead).uh_visual.vi_start.lnum != 0 {
            (*g_st().curbuf).b_visual = (*curhead).uh_visual;
            (*curhead).uh_visual = visualinfo;
        }

        if undo
            && (*curhead).uh_cursor.lnum >= 1
            && (*curhead).uh_cursor.lnum <= (*g_st().curbuf).b_ml.ml_line_count
        {
            (**curwin.as_ptr()).w_cursor = (*curhead).uh_cursor;
            if virtual_active(*curwin.as_ptr()) && (*curhead).uh_cursor_vcol >= 0 {
                coladvance(&mut **curwin.as_ptr(), (*curhead).uh_cursor_vcol);
            }
        } else {
            if (*curhead).uh_cursor.lnum + 1 == (**curwin.as_ptr()).w_cursor.lnum
                && (**curwin.as_ptr()).w_cursor.lnum > 1
            {
                (**curwin.as_ptr()).w_cursor.lnum -= 1;
            }
            if (**curwin.as_ptr()).w_cursor.lnum <= (*g_st().curbuf).b_ml.ml_line_count {
                if (*curhead).uh_cursor.lnum == (**curwin.as_ptr()).w_cursor.lnum {
                    (**curwin.as_ptr()).w_cursor.col = (*curhead).uh_cursor.col;
                    if virtual_active(*curwin.as_ptr()) && (*curhead).uh_cursor_vcol >= 0 {
                        coladvance(&mut **curwin.as_ptr(), (*curhead).uh_cursor_vcol);
                    } else {
                        (**curwin.as_ptr()).w_cursor.coladd = 0;
                    }
                } else {
                    beginline(BL_SOL | BL_FIX);
                }
            } else {
                (**curwin.as_ptr()).w_cursor.col = 0;
                (**curwin.as_ptr()).w_cursor.coladd = 0;
            }
        }

        check_cursor(*curwin.as_ptr());

        if !undo && (*curhead).uh_cursor_after.lnum > 0 {
            (**curwin.as_ptr()).w_cursor = (*curhead).uh_cursor_after;
            check_cursor(*curwin.as_ptr());
        }

        (*g_st().curbuf).b_u_seq_cur = (*curhead).uh_seq;
        if undo {
            (*g_st().curbuf).b_u_seq_cur = if let Some(n) = (*curhead).uh_next {
                uhp_ref(&*g_st().curbuf, n).uh_seq
            } else {
                0
            };
        }

        if (*curhead).uh_save_nr != 0 {
            if undo {
                (*g_st().curbuf).b_u_save_nr_cur = (*curhead).uh_save_nr - 1;
            } else {
                (*g_st().curbuf).b_u_save_nr_cur = (*curhead).uh_save_nr;
            }
        }

        (*g_st().curbuf).b_u_time_cur = (*curhead).uh_time;

        unblock_autocmds();
    }
}

fn u_undo_end(mut did_undo: bool, absolute: bool, quiet: bool) {
    unsafe {
        if (opt_vars().fdo_flags & kOptFdoFlagUndo) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }

        if quiet || g_st().global_busy != 0 || !messaging() || shortmess(kShmUndo) {
            return;
        }

        if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            undo_st().u_newcount -= 1;
        }

        undo_st().u_oldcount -= undo_st().u_newcount;
        let msgstr = if undo_st().u_oldcount == -1 {
            c"more line".as_ptr()
        } else if undo_st().u_oldcount < 0 {
            c"more lines".as_ptr()
        } else if undo_st().u_oldcount == 1 {
            c"line less".as_ptr()
        } else if undo_st().u_oldcount > 1 {
            c"fewer lines".as_ptr()
        } else {
            undo_st().u_oldcount = undo_st().u_newcount;
            if undo_st().u_newcount == 1 {
                c"change".as_ptr()
            } else {
                c"changes".as_ptr()
            }
        };

        let uhp = if let Some(ch) = (*g_st().curbuf).b_u_curhead {
            let next = uhp_ref(&*g_st().curbuf, ch).uh_next;
            if absolute && next.is_some() {
                did_undo = false;
                next
            } else if did_undo {
                Some(ch)
            } else {
                next
            }
        } else {
            (*g_st().curbuf).b_u_newhead
        };

        let mut msgbuf = [0 as c_char; 80];
        if let Some(uh) = uhp {
            undo_fmt_time(
                msgbuf.as_mut_ptr(),
                msgbuf.len(),
                uhp_ref(&*g_st().curbuf, uh).uh_time,
            );
        }

        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            if (*wp).w_buffer == g_st().curbuf && (*wp).w_onebuf_opt.wo_cole > 0 {
                redraw_later(wp, UPD_NOT_VALID);
            }
            wp = (*wp).w_next;
        }

        if g_st().Visual.active {
            check_pos(&mut *g_st().curbuf, &mut g_st().Visual.start);
        }

        smsg_keep!(
            0,
            gettext(c"%lld %s; %s #%lld  %s".as_ptr()),
            if undo_st().u_oldcount < 0 {
                -(undo_st().u_oldcount as i64)
            } else {
                undo_st().u_oldcount as i64
            },
            gettext(msgstr),
            if did_undo {
                gettext(c"before".as_ptr())
            } else {
                gettext(c"after".as_ptr())
            },
            uhp.map_or(0i64, |uh| uhp_ref(&*g_st().curbuf, uh).uh_seq as i64),
            msgbuf.as_ptr(),
        );
    }
}

/// Put the timestamp of an undo header in `buf[buflen]` in a nice format.
pub fn undo_fmt_time(buf: *mut c_char, buflen: usize, tt: time_t) {
    unsafe {
        let now = os_time() as time_t;
        if now - tt >= 100 {
            let mut curtime = Tm { _fields: [0; 16] };
            os_localtime_r(&tt, &raw mut curtime);
            let n = if (os_time() as time_t) - tt < 60 * 60 * 12 {
                strftime(buf, buflen, c"%H:%M:%S".as_ptr(), &raw const curtime)
            } else {
                strftime(
                    buf,
                    buflen,
                    c"%Y/%m/%d %H:%M:%S".as_ptr(),
                    &raw const curtime,
                )
            };
            if n == 0 {
                *buf = NUL;
            }
        } else {
            let seconds = now - tt;
            vim_snprintf!(
                buf,
                buflen,
                ngettext(
                    c"%lld second ago".as_ptr(),
                    c"%lld seconds ago".as_ptr(),
                    seconds as u32 as c_ulong,
                ),
                seconds,
            );
        }
    }
}

/// `u_sync`: stop adding to the current entry list.
pub fn u_sync(force: bool) {
    unsafe {
        if (*g_st().curbuf).b_u_synced || (!force && g_st().no_u_sync > 0) {
            return;
        }

        if get_undolevel(g_st().curbuf) < 0 {
            (*g_st().curbuf).b_u_synced = true;
        } else {
            u_getbot(g_st().curbuf);
            (*g_st().curbuf).b_u_curhead = None;
        }
    }
}

/// `":undolist"`: List the leafs of the undo tree.
pub fn ex_undolist(_eap: &mut exarg_T) {
    unsafe {
        let mut changes: c_int = 1;

        undo_st().lastmark += 1;
        let mark = undo_st().lastmark;
        undo_st().lastmark += 1;
        let nomark = undo_st().lastmark;
        let mut lines: Vec<std::ffi::CString> = Vec::new();

        let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        let mut uhp = (*g_st().curbuf).b_u_oldhead;
        while let Some(uh) = uhp {
            let uh_prev = uhp_ref(&*g_st().curbuf, uh).uh_prev;
            let uh_walk = uhp_ref(&*g_st().curbuf, uh).uh_walk;
            let uh_seq = uhp_ref(&*g_st().curbuf, uh).uh_seq;
            let uh_time = uhp_ref(&*g_st().curbuf, uh).uh_time;
            let uh_save_nr = uhp_ref(&*g_st().curbuf, uh).uh_save_nr;
            if uh_prev.is_none() && uh_walk != nomark && uh_walk != mark {
                vim_snprintf!(
                    iobuf_ptr,
                    IOSIZE,
                    c"%6d %7d  ".as_ptr(),
                    uh_seq,
                    changes,
                );
                let cur_len = std::ffi::CStr::from_ptr(iobuf_ptr).to_bytes().len();
                undo_fmt_time(iobuf_ptr.add(cur_len), IOSIZE - cur_len, uh_time);
                if uh_save_nr > 0 {
                    while std::ffi::CStr::from_ptr(iobuf_ptr).to_bytes().len() < 33 {
                        { let _dst: *mut c_char = iobuf_ptr; let _dsize: usize = IOSIZE; let _dlen = std::ffi::CStr::from_ptr(_dst).to_bytes().len(); crate::memory::xstrlcpy(_dst.add(_dlen), c" ".as_ptr(), _dsize - _dlen); };
                    }
                    vim_snprintf_add!(iobuf_ptr, IOSIZE, c"  %3d".as_ptr(), uh_save_nr);
                }
                lines.push(CStr::from_ptr(iobuf_ptr).to_owned());
            }

            uhp_mut(&mut *g_st().curbuf, uh).uh_walk = mark;

            let alt_next = uhp_ref(&*g_st().curbuf, uh).uh_alt_next;
            let next = uhp_ref(&*g_st().curbuf, uh).uh_next;
            let alt_prev = uhp_ref(&*g_st().curbuf, uh).uh_alt_prev;
            if let Some(p) = uh_prev
                && uhp_ref(&*g_st().curbuf, p).uh_walk != nomark
                && uhp_ref(&*g_st().curbuf, p).uh_walk != mark
            {
                uhp = Some(p);
                changes += 1;
            } else if let Some(an) = alt_next
                && uhp_ref(&*g_st().curbuf, an).uh_walk != nomark
                && uhp_ref(&*g_st().curbuf, an).uh_walk != mark
            {
                uhp = Some(an);
            } else if let Some(n) = next
                && alt_prev.is_none()
                && uhp_ref(&*g_st().curbuf, n).uh_walk != nomark
                && uhp_ref(&*g_st().curbuf, n).uh_walk != mark
            {
                uhp = Some(n);
                changes -= 1;
            } else {
                uhp_mut(&mut *g_st().curbuf, uh).uh_walk = nomark;
                if let Some(ap) = alt_prev {
                    uhp = Some(ap);
                } else {
                    uhp = next;
                    changes -= 1;
                }
            }
        }

        msg_ext_set_kind(c"list_cmd".as_ptr());
        if lines.is_empty() {
            msg(gettext(c"Nothing to undo".as_ptr()), 0);
        } else {
            lines.sort_unstable();

            msg_start();
            msg_puts_hl(
                gettext(c"number changes  when               saved".as_ptr()),
                HLF_T,
                false,
            );
            for line in &lines {
                if g_st().got_int {
                    break;
                }
                msg_putchar(b'\n' as c_int);
                if g_st().got_int {
                    break;
                }
                msg_puts(line.as_ptr());
            }
            msg_end();
        }
    }
}

/// `":undojoin"`: continue adding to the last entry list.
pub fn ex_undojoin(_eap: &mut exarg_T) {
    unsafe {
        if (*g_st().curbuf).b_u_newhead.is_none() {
            return;
        }
        if (*g_st().curbuf).b_u_curhead.is_some() {
            emsg(gettext(
                c"E790: undojoin is not allowed after undo".as_ptr(),
            ));
            return;
        }
        if !(*g_st().curbuf).b_u_synced {
            return;
        }
        if get_undolevel(g_st().curbuf) < 0 {
            return;
        }
        (*g_st().curbuf).b_u_synced = false;
    }
}

/// Called after writing or reloading the file and setting `b_changed` to false.
pub fn u_unchanged(buf: &mut buf_T) {
    u_unch_branch(buf, buf.b_u_oldhead);
        buf.b_did_warn = false;
}

/// After reloading a buffer which was saved for `'undoreload'`: Find the first
/// line that was changed and set the cursor there.
pub fn u_find_first_changed() {
    unsafe {
        if (*g_st().curbuf).b_u_curhead.is_some() {
            return;
        }
        let Some(nh) = (*g_st().curbuf).b_u_newhead else {
            return;
        };
        if uhp_ref(&*g_st().curbuf, nh).uh_entries.is_empty() {
            return;
        }
        let uep: *const u_entry_T = &raw const uhp_ref(&*g_st().curbuf, nh).uh_entries[0];
        if (*uep).ue_top != 0 || (*uep).ue_bot != 0 {
            return;
        }

        let mut lnum: linenr_T = 1;
        while lnum < (*g_st().curbuf).b_ml.ml_line_count && lnum <= (*uep).ue_size() {
            if std::ffi::CStr::from_ptr(ml_get_buf(g_st().curbuf, lnum)).to_bytes() != std::ffi::CStr::from_ptr((*uep).ue_array.as_slice()[(lnum - 1) as usize].as_ptr()).to_bytes()
            {
                clearpos(&mut uhp_mut(&mut *g_st().curbuf, nh).uh_cursor);
                uhp_mut(&mut *g_st().curbuf, nh).uh_cursor.lnum = lnum;
                return;
            }
            lnum += 1;
        }
        if (*g_st().curbuf).b_ml.ml_line_count != (*uep).ue_size() {
            clearpos(&mut uhp_mut(&mut *g_st().curbuf, nh).uh_cursor);
            uhp_mut(&mut *g_st().curbuf, nh).uh_cursor.lnum = lnum;
        }
    }
}

/// Increase the write count, store it in the last undo header, what would be used for `"u"`.
pub fn u_update_save_nr(buf: &mut buf_T) {
    buf.b_u_save_nr_last += 1;
        buf.b_u_save_nr_cur = buf.b_u_save_nr_last;
        let uhp = if let Some(ch) = buf.b_u_curhead {
            uhp_ref(buf, ch).uh_next
        } else {
            buf.b_u_newhead
        };
        if let Some(uh) = uhp {
            uhp_mut(buf, uh).uh_save_nr = buf.b_u_save_nr_last;
        }
}

fn u_unch_branch(buf: *mut buf_T, uhp: Option<usize>) {
    unsafe {
        let mut uh = uhp;
        while let Some(u) = uh {
            uhp_mut(&mut *buf, u).uh_flags |= UH_CHANGED;
            let alt_next = uhp_ref(&*buf, u).uh_alt_next;
            let prev = uhp_ref(&*buf, u).uh_prev;
            if alt_next.is_some() {
                u_unch_branch(buf, alt_next);
            }
            uh = prev;
        }
    }
}

fn u_get_headentry(buf: &mut buf_T) -> *mut u_entry_T {
    let Some(newhead) = buf.b_u_newhead else {
            iemsg(gettext(E_UNDO_LIST_CORRUPT.as_ptr()));
            return ptr::null_mut();
        };
        if uhp_ref(buf, newhead).uh_entries.is_empty() {
            iemsg(gettext(E_UNDO_LIST_CORRUPT.as_ptr()));
            return ptr::null_mut();
        }
        &raw mut uhp_mut(buf, newhead).uh_entries.as_mut_slice()[0]
}

fn u_getbot(buf: *mut buf_T) {
    unsafe {
        let uep = u_get_headentry(&mut *buf);
        if uep.is_null() {
            return;
        }

        let newhead = (*buf).b_u_newhead.unwrap();
        if uhp_ref(&*buf, newhead).uh_getbot_entry {
            let extra = (*buf).b_ml.ml_line_count - (*uep).ue_lcount;
            (*uep).ue_bot = (*uep).ue_top + (*uep).ue_size() + 1 + extra;
            if (*uep).ue_bot < 1 || (*uep).ue_bot > (*buf).b_ml.ml_line_count {
                iemsg(gettext(E_UNDO_LINE_MISSING.as_ptr()));
                (*uep).ue_bot = (*uep).ue_top + 1;
            }

            uhp_mut(&mut *buf, newhead).uh_getbot_entry = false;
        }

        (*buf).b_u_synced = true;
    }
}

fn u_freeheader(buf: *mut buf_T, uhp: usize, uhpp: &mut Option<usize>) {
    unsafe {
        if let Some(alt_next) = uhp_ref(&*buf, uhp).uh_alt_next {
            u_freebranch(buf, alt_next, uhpp);
        }

        if let Some(alt_prev) = uhp_ref(&*buf, uhp).uh_alt_prev {
            uhp_mut(&mut *buf, alt_prev).uh_alt_next = None;
        }

        let next = uhp_ref(&*buf, uhp).uh_next;
        let prev = uhp_ref(&*buf, uhp).uh_prev;
        if let Some(n) = next {
            uhp_mut(&mut *buf, n).uh_prev = prev;
        } else {
            (*buf).b_u_oldhead = prev;
        }

        if let Some(p) = prev {
            let mut uhap = Some(p);
            while let Some(ua) = uhap {
                uhp_mut(&mut *buf, ua).uh_next = next;
                uhap = uhp_ref(&*buf, ua).uh_alt_next;
            }
        } else {
            (*buf).b_u_newhead = next;
        }

        u_freeentries(buf, uhp, uhpp);
    }
}

fn u_freebranch(buf: *mut buf_T, uhp: usize, uhpp: &mut Option<usize>) {
    unsafe {
        if Some(uhp) == (*buf).b_u_oldhead {
            while let Some(oh) = (*buf).b_u_oldhead {
                u_freeheader(buf, oh, uhpp);
            }
            return;
        }

        if let Some(alt_prev) = uhp_ref(&*buf, uhp).uh_alt_prev {
            uhp_mut(&mut *buf, alt_prev).uh_alt_next = None;
        }

        let mut next = Some(uhp);
        while let Some(tofree) = next {
            if let Some(alt_next) = uhp_ref(&*buf, tofree).uh_alt_next {
                u_freebranch(buf, alt_next, uhpp);
            }
            next = uhp_ref(&*buf, tofree).uh_prev;
            u_freeentries(buf, tofree, uhpp);
        }
    }
}

fn u_forget_header(buf: &mut buf_T, uhp: usize) {
    let parent = uhp_ref(&*buf, uhp).uh_next;
        let alt = uhp_ref(&*buf, uhp).uh_alt_next;
        let alt_prev = uhp_ref(&*buf, uhp).uh_alt_prev;
        if let Some(a) = alt {
            uhp_mut(&mut *buf, a).uh_alt_prev = alt_prev;
        }
        if let Some(ap) = alt_prev {
            uhp_mut(&mut *buf, ap).uh_alt_next = alt;
        }
        if let Some(p) = parent {
            uhp_mut(&mut *buf, p).uh_prev = alt;
        }
        if buf.b_u_oldhead == Some(uhp) {
            buf.b_u_oldhead = alt;
        }
        uhp_mut(&mut *buf, uhp).uh_alt_next = None;
        uhp_mut(&mut *buf, uhp).uh_alt_prev = None;
        buf.b_u_newhead = parent;
        buf.b_u_curhead = alt;
        buf.b_u_seq_cur = parent.map_or(0, |p| uhp_ref(&*buf, p).uh_seq);
        if buf.b_u_seq_last == uhp_ref(&*buf, uhp).uh_seq {
            buf.b_u_seq_last -= 1;
        }
        buf.b_u_synced = true;
        let mut dummy = None;
        u_freebranch(&raw mut *buf, uhp, &mut dummy);
}

fn u_freeentries(buf: *mut buf_T, uhp: usize, uhpp: &mut Option<usize>) {
    unsafe {
        if (*buf).b_u_curhead == Some(uhp) {
            (*buf).b_u_curhead = None;
        }
        if (*buf).b_u_newhead == Some(uhp) {
            (*buf).b_u_newhead = None;
        }
        if *uhpp == Some(uhp) {
            *uhpp = None;
        }

        uhp_mut(&mut *buf, uhp).clone_from_slice_or_reset();
        (*buf).b_u_free_headers.push(uhp);
        (*buf).b_u_numhead -= 1;
    }
}

/// Prunes the newest undo entry if it has no changes.
pub fn u_forget_unchanged(buf: &mut buf_T) {
    unsafe {
        let Some(uhp) = buf.b_u_newhead else {
            return;
        };
        if uhp_ref(&*buf, uhp).uh_entries.is_empty() || buf.b_u_curhead.is_some() {
            return;
        };
        let uep: *mut u_entry_T = &raw mut uhp_mut(&mut *buf, uhp).uh_entries.as_mut_slice()[0];
        if u_entry_resized(&mut *buf, &*uep) {
            return;
        }
        for i in 0..(*uep).ue_size() {
            if std::ffi::CStr::from_ptr((*uep).ue_array.as_slice()[i as usize].as_ptr()).to_bytes() != std::ffi::CStr::from_ptr(ml_get_buf(&raw mut *buf, (*uep).ue_top + 1 + i)).to_bytes()
            {
                return;
            }
        }

        uhp_mut(&mut *buf, uhp).uh_entries.remove(0);
        uhp_mut(&mut *buf, uhp).uh_getbot_entry = false;
        if uhp_ref(&*buf, uhp).uh_entries.is_empty() {
            u_forget_header(buf, uhp);
        }
    }
}

/// Invalidate the undo buffer; called when storage has already been released.
pub fn u_clearall(buf: &mut buf_T) {
    buf.b_u_newhead = None;
        buf.b_u_oldhead = None;
        buf.b_u_curhead = None;
        buf.b_u_synced = true;
        buf.b_u_numhead = 0;
        buf.b_u_line_ptr = None;
        buf.b_u_line_lnum = 0;
}

/// Free all allocated memory blocks for the buffer `buf`.
pub fn u_blockfree(buf: *mut buf_T) {
    unsafe {
        while let Some(previous_oldhead) = (*buf).b_u_oldhead {
            let mut dummy = None;
            u_freeheader(buf, previous_oldhead, &mut dummy);
            assert!((*buf).b_u_oldhead != Some(previous_oldhead));
        }
        (*buf).b_u_line_ptr = None;
    }
}

/// Free all allocated memory blocks for the buffer `buf` and invalidate the undo buffer.
pub fn u_clearallandblockfree(buf: &mut buf_T) {
    u_blockfree(buf);
        u_clearall(buf);
        buf.b_u_headers.clear();
        buf.b_u_free_headers.clear();
}

fn u_saveline(buf: &mut buf_T, lnum: linenr_T) {
    unsafe {
        if lnum == buf.b_u_line_lnum {
            return;
        }
        if lnum < 1 || lnum > buf.b_ml.ml_line_count {
            return;
        }
        u_clearline(buf);
        buf.b_u_line_lnum = lnum;
        if (**curwin.as_ptr()).w_buffer == buf && (**curwin.as_ptr()).w_cursor.lnum == lnum {
            buf.b_u_line_colnr = (**curwin.as_ptr()).w_cursor.col;
        } else {
            buf.b_u_line_colnr = 0;
        }
        buf.b_u_line_ptr = Some(u_save_line_buf(&raw mut *buf, lnum));
    }
}

/// Clear the line saved for the `"U"` command.
pub fn u_clearline(buf: &mut buf_T) {
    if buf.b_u_line_ptr.is_none() {
            return;
        }

        buf.b_u_line_ptr = None;
        buf.b_u_line_lnum = 0;
}

/// Implementation of the `"U"` command.
pub fn u_undoline() {
    unsafe {
        if (*g_st().curbuf).b_u_line_ptr.is_none()
            || (*g_st().curbuf).b_u_line_lnum > (*g_st().curbuf).b_ml.ml_line_count
        {
            beep_flush();
            return;
        }
        atom_did_global_op();

        if u_savecommon(
            g_st().curbuf,
            (*g_st().curbuf).b_u_line_lnum - 1,
            (*g_st().curbuf).b_u_line_lnum + 1,
            0,
            false,
        ) == FAIL
        {
            return;
        }

        let oldp = u_save_line((*g_st().curbuf).b_u_line_lnum);
        let new_line = (*g_st().curbuf).b_u_line_ptr.as_ref().unwrap();
        let new_line_len = new_line.as_bytes().len() as colnr_T;
        ml_replace((*g_st().curbuf).b_u_line_lnum, new_line.as_ptr().cast_mut(), true);
        extmark_splice_cols(
            g_st().curbuf,
            (*g_st().curbuf).b_u_line_lnum - 1,
            0,
            oldp.as_bytes().len() as colnr_T,
            new_line_len,
            kExtmarkUndo,
        );
        changed_bytes((*g_st().curbuf).b_u_line_lnum, 0);
        (*g_st().curbuf).b_u_line_ptr = Some(oldp);

        let t = (*g_st().curbuf).b_u_line_colnr;
        if (**curwin.as_ptr()).w_cursor.lnum == (*g_st().curbuf).b_u_line_lnum {
            (*g_st().curbuf).b_u_line_colnr = (**curwin.as_ptr()).w_cursor.col;
        }
        (**curwin.as_ptr()).w_cursor.col = t;
        (**curwin.as_ptr()).w_cursor.lnum = (*g_st().curbuf).b_u_line_lnum;
        check_cursor_col(&mut **curwin.as_ptr());
    }
}

fn u_save_line(lnum: linenr_T) -> std::ffi::CString {
    u_save_line_buf(g_st().curbuf, lnum)
}

fn u_save_line_buf(buf: *mut buf_T, lnum: linenr_T) -> std::ffi::CString {
    unsafe { CStr::from_ptr(ml_get_buf(buf, lnum)).to_owned() }
}

/// Check if the `'modified'` flag is set, or `'ff'` has changed.
pub fn bufIsChanged(buf: &mut buf_T) -> bool {
    if bt_prompt(&raw mut *buf) {
            buf.b_modified_was_set
        } else {
            !bt_dontwrite(&raw mut *buf) && (buf.b_changed != 0 || file_ff_differs(&raw mut *buf, true))
        }
}

/// Return true if any buffer has changes. Also buffers that are not written.
pub fn anyBufIsChanged() -> bool {
    unsafe {
        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if bufIsChanged(&mut *buf) {
                return true;
            }
            buf = (*buf).b_next;
        }
        false
    }
}

/// Return true if the current buffer has changed.
pub fn curbufIsChanged() -> bool {
    unsafe { bufIsChanged(&mut *g_st().curbuf) }
}

fn u_eval_tree(buf: &mut buf_T, first_uhp: Option<usize>) -> *mut list_T {
    unsafe {
        let list = tv_list_alloc(kListLenMayKnow);

        let mut uhp = first_uhp;
        while let Some(uh) = uhp {
            let dict = tv_dict_alloc();
            tv_dict_add_nr(
                dict,
                c"seq".as_ptr(),
                3,
                uhp_ref(&*buf, uh).uh_seq as varnumber_T,
            );
            tv_dict_add_nr(
                dict,
                c"time".as_ptr(),
                4,
                uhp_ref(&*buf, uh).uh_time as varnumber_T,
            );
            if Some(uh) == buf.b_u_newhead {
                tv_dict_add_nr(dict, c"newhead".as_ptr(), 7, 1);
            }
            if Some(uh) == buf.b_u_curhead {
                tv_dict_add_nr(dict, c"curhead".as_ptr(), 7, 1);
            }
            if uhp_ref(&*buf, uh).uh_save_nr > 0 {
                tv_dict_add_nr(
                    dict,
                    c"save".as_ptr(),
                    4,
                    uhp_ref(&*buf, uh).uh_save_nr as varnumber_T,
                );
            }

            let alt_next = uhp_ref(&*buf, uh).uh_alt_next;
            let prev = uhp_ref(&*buf, uh).uh_prev;
            if alt_next.is_some() {
                tv_dict_add_list(dict, c"alt".as_ptr(), 3, u_eval_tree(buf, alt_next));
            }

            tv_list_append_dict(list, dict);
            uhp = prev;
        }

        list
    }
}

/// `"undofile(name)"` function.
pub fn f_undofile(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    unsafe {
        crate::eval::typval::tv_set_string(rettv, ptr::null());
        let fname = tv_get_string(argvars);

        if *fname != NUL
            && let Some(ffname) = FullName_save(fname, true)
            && let Some(ufile) = u_get_undo_file_name(ffname.as_ptr(), false)
        {
            crate::eval::typval::tv_set_string(rettv, ufile.as_ptr());
        }
    }
}

/// `"undotree(expr)"` function.
pub fn f_undotree(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    unsafe {
        tv_dict_alloc_ret(rettv);

        let tv = argvars;
        let buf = if (*tv).v_type == VarType::VAR_UNKNOWN {
            g_st().curbuf
        } else {
            get_buf_arg(tv)
        };
        if buf.is_null() {
            return;
        }

        let dict = rettv.vval.v_dict();

        tv_dict_add_nr(
            dict,
            c"synced".as_ptr(),
            6,
            varnumber_T::from((*buf).b_u_synced),
        );
        tv_dict_add_nr(
            dict,
            c"seq_last".as_ptr(),
            8,
            (*buf).b_u_seq_last as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"save_last".as_ptr(),
            9,
            (*buf).b_u_save_nr_last as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"seq_cur".as_ptr(),
            7,
            (*buf).b_u_seq_cur as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"time_cur".as_ptr(),
            8,
            (*buf).b_u_time_cur as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"save_cur".as_ptr(),
            8,
            (*buf).b_u_save_nr_cur as varnumber_T,
        );

        tv_dict_add_list(
            dict,
            c"entries".as_ptr(),
            7,
            u_eval_tree(&mut *buf, (*buf).b_u_oldhead),
        );
    }
}

/// Drops named mark `idx` from the pending undo snapshot.
pub fn u_update_named_mark(buf: &mut buf_T, idx: c_int) {
    let uhp = buf.b_u_curhead.or(buf.b_u_newhead);
        if let Some(uh) = uhp {
            uhp_mut(&mut *buf, uh).uh_namedm[idx as usize].mark.lnum = 0;
        }
}

/// Given the buffer, return the undo header index. If none is set, set one first.
pub fn u_force_get_undo_header(buf: &mut buf_T) -> Option<usize> {
    let mut uhp = buf.b_u_curhead.or(buf.b_u_newhead);

        if uhp.is_none() {
            u_savecommon(buf, 0, 1, 1, true);
            uhp = buf.b_u_curhead;
            if uhp.is_none() {
                uhp = buf.b_u_newhead;
                if get_undolevel(buf) > 0 && uhp.is_none() {
                    std::process::abort();
                }
            }
        }
        uhp
}

/// Checkpoints the undo state of `buf` and detaches its undotree.
pub fn u_checkpoint(uc: &mut UndoCheckpoint, buf: &mut buf_T) {
    uc.uc_synced = buf.b_u_synced;
        uc.uc_oldhead = buf.b_u_oldhead;
        uc.uc_newhead = buf.b_u_newhead;
        uc.uc_curhead = buf.b_u_curhead;
        uc.uc_numhead = buf.b_u_numhead;
        uc.uc_seq_last = buf.b_u_seq_last;
        uc.uc_save_nr_last = buf.b_u_save_nr_last;
        uc.uc_seq_cur = buf.b_u_seq_cur;
        uc.uc_time_cur = buf.b_u_time_cur;
        uc.uc_save_nr_cur = buf.b_u_save_nr_cur;
        uc.uc_line_ptr = buf.b_u_line_ptr.take();
        uc.uc_line_lnum = buf.b_u_line_lnum;
        uc.uc_line_colnr = buf.b_u_line_colnr;
        uc.uc_undolevels = buf.b_p_ul;
        uc.uc_changedtick = buf_get_changedtick(&*buf);

        u_clearall(&mut *buf);
        buf.b_p_ul = OptInt::from(c_int::MAX);
}

/// Reverts `buf` from a checkpoint.
pub fn u_rollback(uc: &mut UndoCheckpoint, buf: &mut buf_T) {
    unsafe {
        if buf.b_u_seq_cur != uc.uc_seq_cur {
            let mut count: c_int = 0;
            let mut uhp = buf.b_u_curhead.or(buf.b_u_newhead);
            while let Some(uh) = uhp {
                uhp = uhp_ref(&*buf, uh).uh_next;
                count += 1;
            }

            let mut cs: CtxSwitch = CtxSwitch::new();
            ctx_switch(&raw mut cs, ptr::null_mut(), ptr::null_mut(), &raw mut *buf, 0);
            if !(*g_st().curbuf).b_u_synced {
                u_sync(true);
            }
            if !u_undo_and_forget(count, false) {
                std::process::abort();
            }
            ctx_restore(&mut cs);
        }

        u_blockfree(&raw mut *buf);
        buf.b_u_oldhead = uc.uc_oldhead;
        buf.b_u_newhead = uc.uc_newhead;
        buf.b_u_curhead = uc.uc_curhead;
        buf.b_u_numhead = uc.uc_numhead;
        buf.b_u_seq_last = uc.uc_seq_last;
        buf.b_u_save_nr_last = uc.uc_save_nr_last;
        buf.b_u_seq_cur = uc.uc_seq_cur;
        buf.b_u_time_cur = uc.uc_time_cur;
        buf.b_u_save_nr_cur = uc.uc_save_nr_cur;
        buf.b_u_line_ptr = uc.uc_line_ptr.take();
        buf.b_u_line_lnum = uc.uc_line_lnum;
        buf.b_u_line_colnr = uc.uc_line_colnr;
        if buf.b_u_curhead.is_none() {
            buf.b_u_synced = uc.uc_synced;
        }

        if uc.uc_changedtick != buf_get_changedtick(&*buf) {
            buf_set_changedtick(&mut *buf, uc.uc_changedtick);
        }
        buf.b_p_ul = uc.uc_undolevels;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalpos_and_clearpos_helpers_work() {
        let mut p = pos_T {
            lnum: 5,
            col: 3,
            coladd: 1,
        };
        assert!(equalpos(
            p,
            pos_T {
                lnum: 5,
                col: 3,
                coladd: 1,
            }
        ));
        clearpos(&mut p);
        assert_eq!(p.lnum, 0);
        assert_eq!(p.col, 0);
        assert_eq!(p.coladd, 0);
    }

    #[test]
    fn u_write_undo_and_hash_work() {
        use crate::buffer::{BLN_LISTED, DOBUF_WIPE, buflist_new, close_buffer};
        use crate::os::fs::{MAXPATHL, os_dirname};
        use std::ffi::{CStr, CString};

        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            crate::main_init::ensure_test_editor_init();
            g_st().emsg_skip += 1;

            let mut cwd_buf = [0 as c_char; MAXPATHL];
            assert_eq!(os_dirname(cwd_buf.as_mut_ptr(), MAXPATHL), OK);
            let cwd = CStr::from_ptr(cwd_buf.as_ptr()).to_str().unwrap();
            let base = format!("{cwd}/target/ut_undo_{}", std::process::id());
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();

            let save_udir = opt_vars().p_udir;
            let base_c = CString::new(base.clone()).unwrap();
            opt_vars().p_udir = base_c.as_ptr().cast_mut();

            let fpath = format!("{base}/Xtest-unit-undo");
            std::fs::write(&fpath, b"hello\n").unwrap();
            let mut fpath_c = CString::new(fpath.clone()).unwrap().into_bytes_with_nul();
            let buf = buflist_new(
                fpath_c.as_mut_ptr().cast(),
                fpath_c.as_mut_ptr().cast(),
                1,
                BLN_LISTED,
            );
            (*buf).b_u_numhead = 1;

            let mut hash = [0u8; UNDO_HASH_SIZE];
            u_compute_hash(&mut *buf, hash.as_mut_ptr());

            // Write undo file to undodir
            u_write_undo(ptr::null(), false, buf, hash.as_mut_ptr());
            let ufile_cstr = u_get_undo_file_name((*buf).b_ffname, false).unwrap();
            let ufile = ufile_cstr.to_str().unwrap().to_owned();
            assert!(std::path::Path::new(&ufile).exists());
            let _ = std::fs::remove_file(&ufile);

            // Write undo file with explicit name
            let explicit = format!("{base}/undofile.test");
            let explicit_c = CString::new(explicit.clone()).unwrap();
            u_write_undo(explicit_c.as_ptr(), false, buf, hash.as_mut_ptr());
            assert!(std::path::Path::new(&explicit).exists());
            let _ = std::fs::remove_file(&explicit);

            // Force write (:wundo!) overwrites non-undo file
            std::fs::write(&ufile, b"not an undo file").unwrap();
            let ufile_c = CString::new(ufile.clone()).unwrap();
            u_write_undo(ufile_c.as_ptr(), true, buf, hash.as_mut_ptr());
            assert_ne!(std::fs::read(&ufile).unwrap(), b"not an undo file");
            let _ = std::fs::remove_file(&ufile);

            // Does not write undo file when b_u_numhead == 0
            (*buf).b_u_numhead = 0;
            u_write_undo(ptr::null(), false, buf, hash.as_mut_ptr());
            assert!(!std::path::Path::new(&ufile).exists());

            close_buffer(ptr::null_mut(), buf, DOBUF_WIPE, false, false, false);
            opt_vars().p_udir = save_udir;
            let _ = std::fs::remove_dir_all(&base);
            g_st().emsg_skip -= 1;
        }
    }
}
