#![forbid(unsafe_code)]

//! Multi-level undo facility (`src/nvim/undo.c`).

use crate::api::private::validate::NvimString;
use crate::autocmd::{HLF_T, block_autocmds, unblock_autocmds};
use crate::buffer::{
    FileBuf, MAXLNUM, NMARKS, NO_LOCAL_UNDOLEVEL, bt_dontwrite, bt_prompt, buf_get_changedtick,
    buf_is_empty, buf_set_changedtick,
};
use crate::buffer_updates::{buf_updates_changedtick, buf_updates_unload};
use crate::change::{
    change_warning, changed, changed_bytes, changed_lines, file_ff_differs, unchanged,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{
    Colnr, CursorPos, Linenr, check_cursor, check_cursor_col, check_cursor_lnum, check_pos,
    clearpos, coladvance, equalpos, getviscol,
};
use crate::drawscreen::{UPD_NOT_VALID, redraw_later, redraw_winline};
use crate::errors::*;
use crate::eval::funcs::get_buf_arg;
use crate::eval::typval::{
    EvalFuncData, TvList, TypVal, VarType, Varnumber, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_alloc, tv_dict_alloc_ret, tv_get_string, tv_list_alloc, tv_list_append_dict,
};
use crate::event::r#loop::uv_strerror;
use crate::ex_docmd::{Exarg, UNDO_HASH_SIZE, expr_map_locked};
use crate::ex_getln::{text_locked, text_locked_msg};
use crate::extmark::{
    ExtmarkMove, ExtmarkSplice, ExtmarkUndoObject, ExtmarkUndoVec, K_EXTMARK_MOVE, K_EXTMARK_NOOP,
    K_EXTMARK_SPLICE, K_EXTMARK_UNDO, UndoObjectType, extmark_apply_undo, extmark_splice_cols,
};
#[cfg(unix)]
use crate::fileio::getuid;
use crate::fileio::{EOF, O_RDONLY, Off, get2c, get4c, get8ctime, read_eintr};
use crate::fold::fold_open_cursor;
use crate::globals::*;
use crate::input::beep_flush;
use crate::input_cmdatom::{atom_did_global_op, atom_origin_pos};
use crate::insert::beginline;
use crate::mark::{Fmark, mark_adjust, setpcmark};
use crate::mcursor::mc_undo_time;
#[cfg(unix)]
use crate::memline::resolve_symlink;
use crate::memline::{ml_append_flags, ml_delete, ml_replace};
use crate::message::{
    emsg, give_warning, iemsg, internal_error, messaging, msg, msg_end, msg_ext_set_kind,
    msg_putchar, msg_puts, msg_puts_hl, msg_start, semsg, smsg, smsg_keep, verb_msg, verbose_enter,
    verbose_leave,
};
use crate::normal::Visualinfo;
use crate::option::{OptInt, copy_option_part, shortmess};
use crate::option_vars::*;
use crate::os::fs::{
    CFile, FileInfo, MAXPATHL, O_CREAT, O_EXCL, O_NOFOLLOW, O_WRONLY, VimAcl, os_close,
    os_fileinfo_fd, os_fileinfo_size, os_free_acl, os_fsync, os_get_acl, os_getperm, os_isdir,
    os_mkdir_recurse, os_open, os_path_exists, os_remove, os_set_acl, os_setperm,
};
use crate::os::input::fast_breakcheck;
use crate::os::time::{Timestamp, Tm, os_localtime_r, os_strftime, os_time, time_t};
use crate::path::{FullName_save, concat_fnames, path_tail, vim_ispathsep, vim_ispathsep_nocolon};
use crate::sha256::ContextSha256;
use crate::spell::spell_check_window;
use crate::state::virtual_active;
use crate::strings::{format_to_buf, format_to_buf_add, gettext, ngettext};
use std::ffi::CStr;
use std::ptr;

#[cfg(unix)]
use crate::event::r#loop::{uv_gid_t, uv_uid_t};
#[cfg(unix)]
use crate::os::fs::{os_fchown, os_fileinfo};

use crate::arglist::ML_EMPTY;
use crate::eval::K_LIST_LEN_MAY_KNOW_CONST;
use crate::ex_cmds::{BL_FIX, BL_SOL, K_CPO_UNDO};

const UH_CHANGED: i32 = 0x01;
const UH_EMPTYBUF: i32 = 0x02;
const UH_RELOAD: i32 = 0x04;

const K_SHM_UNDO: i32 = b'u' as i32;
const K_OPT_FDO_FLAG_UNDO: u32 = 0x200;

const UF_START_MAGIC: &[u8; 9] = b"Vim\x9fUnDo\xe5";
const UF_START_MAGIC_LEN: usize = 9;
const UF_HEADER_MAGIC: i32 = 0x5fd0;
const UF_HEADER_END_MAGIC: i32 = 0xe7aa;
const UF_ENTRY_MAGIC: i32 = 0xf518;
const UF_ENTRY_END_MAGIC: i32 = 0x3581;
const UF_VERSION: i32 = 3;
const UF_LAST_SAVE_NR: i32 = 1;
const UHP_SAVE_NR: i32 = 1;

const E_UNDO_LIST_CORRUPT: &CStr = c"E439: Undo list corrupt";
const E_UNDO_LINE_MISSING: &CStr = c"E440: Undo line missing";
const E_WRITE_ERROR_IN_UNDO_FILE_STR: &CStr = c"E829: Write error in undo file: %s";
const E_NOT_OPEN: &CStr = c"E828: Cannot open undo file for writing: %s";

#[repr(C)]
pub struct UEntry {
    pub ue_top: Linenr,
    pub ue_bot: Linenr,
    pub ue_lcount: Linenr,
    pub ue_array: Vec<std::ffi::CString>,
}

impl UEntry {
    #[inline]
    pub const fn ue_size(&self) -> Linenr {
        self.ue_array.len() as Linenr
    }
}

#[repr(C)]
pub struct UHeader {
    pub uh_next: Option<usize>,
    pub uh_prev: Option<usize>,
    pub uh_alt_next: Option<usize>,
    pub uh_alt_prev: Option<usize>,
    pub uh_seq: i32,
    pub uh_walk: i32,
    pub uh_entries: Vec<UEntry>,
    pub uh_getbot_entry: bool,
    pub uh_cursor: CursorPos,
    pub uh_cursor_after: CursorPos,
    pub uh_cursor_vcol: Colnr,
    pub uh_flags: i32,
    pub uh_namedm: [Fmark; NMARKS],
    pub uh_extmark: ExtmarkUndoVec,
    pub uh_visual: Visualinfo,
    pub uh_time: time_t,
    pub uh_save_nr: i32,
}

impl UHeader {
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
            uh_cursor: CursorPos::new(),
            uh_cursor_after: CursorPos::new(),
            uh_cursor_vcol: 0,
            uh_flags: 0,
            uh_namedm: [const { Fmark::new() }; NMARKS],
            uh_extmark: Vec::new(),
            uh_visual: Visualinfo::new(),
            uh_time: 0,
            uh_save_nr: 0,
        }
    }

    fn clone_from_slice_or_reset(&mut self) {
        *self = Self::new();
    }
}

#[inline]
pub fn uhp_ref(buf: &FileBuf, uh: usize) -> &UHeader {
    &buf.b_u_headers[uh]
}

#[inline]
pub fn uhp_mut(buf: &mut FileBuf, uh: usize) -> &mut UHeader {
    &mut buf.b_u_headers[uh]
}

fn u_alloc_header(buf: &mut FileBuf) -> usize {
    if let Some(idx) = buf.b_u_free_headers.pop() {
        uhp_mut(buf, idx).clone_from_slice_or_reset();
        idx
    } else {
        let idx = buf.b_u_headers.len();
        buf.b_u_headers.push(UHeader::new());
        idx
    }
}

#[derive(Clone)]
pub struct UndoCheckpoint {
    pub uc_oldhead: Option<usize>,
    pub uc_newhead: Option<usize>,
    pub uc_curhead: Option<usize>,
    pub uc_numhead: i32,
    pub uc_synced: bool,
    pub uc_seq_last: i32,
    pub uc_save_nr_last: i32,
    pub uc_seq_cur: i32,
    pub uc_time_cur: time_t,
    pub uc_save_nr_cur: i32,
    pub uc_line_ptr: Option<std::ffi::CString>,
    pub uc_line_lnum: Linenr,
    pub uc_line_colnr: Colnr,
    pub uc_undolevels: OptInt,
    pub uc_changedtick: Varnumber,
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

struct Bufinfo {
    bi_buf: crate::globals::BufPtr,
    bi_fp: crate::os::fs::CFile,
    bi_fsize: Off,
}

struct UndoModuleState {
    u_newcount: std::cell::Cell<i32>,
    u_oldcount: std::cell::Cell<i32>,
    undo_undoes: std::cell::Cell<bool>,
    lastmark: std::cell::Cell<i32>,
}

impl UndoModuleState {
    const fn new() -> Self {
        Self {
            u_newcount: std::cell::Cell::new(0),
            u_oldcount: std::cell::Cell::new(0),
            undo_undoes: std::cell::Cell::new(false),
            lastmark: std::cell::Cell::new(0),
        }
    }
}

std::thread_local! {
    static UNDO_STATE: UndoModuleState = const { UndoModuleState::new() };
}

#[inline]
fn undo_st<R>(f: impl FnOnce(&UndoModuleState) -> R) -> R {
    UNDO_STATE.with(f)
}

/// Save the current line for both the `"u"` and `"U"` command.
pub fn u_save_cursor() -> bool {
    let cur = curwin_mut().w_cursor.lnum;
    let top = if cur > 0 { cur - 1 } else { 0 };
    let bot = cur + 1;
    u_save(top, bot)
}

/// Save the lines between `top` and `bot` for both the `"u"` and `"U"` command.
pub fn u_save(top: Linenr, bot: Linenr) -> bool {
    u_save_buf(curbuf_mut(), top, bot)
}

pub fn u_save_buf(buf: &mut FileBuf, top: Linenr, bot: Linenr) -> bool {
    if top >= bot || bot > buf.b_ml.ml_line_count + 1 {
        return false;
    }

    if top + 2 == bot {
        u_saveline(buf, top + 1);
    }

    u_savecommon(buf, top, bot, 0, false)
}

/// Save the line `lnum` (used by `":s"` and `"~"` command).
pub fn u_savesub(lnum: Linenr) -> bool {
    u_savecommon(curbuf_mut(), lnum - 1, lnum + 1, lnum + 1, false)
}

/// A new line is inserted before line `lnum` (used by `:s` command).
pub fn u_inssub(lnum: Linenr) -> bool {
    u_savecommon(curbuf_mut(), lnum - 1, lnum, lnum + 1, false)
}

/// Save the lines `lnum` - `lnum + nlines` (used by delete command).
pub fn u_savedel(lnum: Linenr, nlines: Linenr) -> bool {
    u_savecommon(
        curbuf_mut(),
        lnum - 1,
        lnum + nlines,
        if nlines == curbuf_mut().b_ml.ml_line_count {
            2
        } else {
            lnum
        },
        false,
    )
}

/// Return true when undo is allowed. Otherwise print an error message and return false.
pub fn undo_allowed(buf: &mut FileBuf) -> bool {
    if !buf.b_p_ma {
        emsg(gettext(E_MODIFIABLE));
        return false;
    }

    if g_st().sandbox != 0 {
        emsg(gettext(E_SANDBOX));
        return false;
    }

    if g_st().textlock != 0 || expr_map_locked() {
        emsg(gettext(E_TEXTLOCK));
        return false;
    }

    true
}

fn get_undolevel(buf: &mut FileBuf) -> OptInt {
    if buf.b_p_ul == NO_LOCAL_UNDOLEVEL {
        opt_vars().p_ul
    } else {
        buf.b_p_ul
    }
}

#[inline]
fn zero_fmark_additional_data(fmarks: &mut [Fmark; NMARKS]) {
    for fm in fmarks {
        fm.additional_data = None;
    }
}

/// Common code for various ways to save text before a change.
pub fn u_savecommon(
    buf: &mut FileBuf,
    top: Linenr,
    bot: Linenr,
    newbot: Linenr,
    reload: bool,
) -> bool {
    if !reload {
        if !undo_allowed(buf) {
            return false;
        }

        if (&raw mut *buf) == g_st().curbuf.as_ptr() {
            change_warning(buf, 0);
        }

        if bot > buf.b_ml.ml_line_count + 1 {
            emsg(gettext(c"E881: Line count changed unexpectedly"));
            return false;
        }
    }

    let size = bot - top - 1;

    if buf.b_u_synced {
        buf.b_new_change = true;

        let uhp = (get_undolevel(buf) >= 0).then(|| u_alloc_header(buf));

        let mut old_curhead = buf.b_u_curhead;
        if let Some(oc) = old_curhead {
            buf.b_u_newhead = uhp_ref(buf, oc).uh_next;
            buf.b_u_curhead = None;
        }

        while (buf.b_u_numhead as OptInt) > get_undolevel(buf)
            && let Some(mut uhfree) = buf.b_u_oldhead
        {
            if Some(uhfree) == old_curhead {
                u_freebranch(buf, uhfree, &mut old_curhead);
            } else if uhp_ref(buf, uhfree).uh_alt_next.is_none() {
                u_freeheader(buf, uhfree, &mut old_curhead);
            } else {
                while let Some(alt_next) = uhp_ref(buf, uhfree).uh_alt_next {
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
            buf.b_u_synced = false;
            return true;
        };

        uhp_mut(buf, uhp).uh_prev = None;
        uhp_mut(buf, uhp).uh_next = buf.b_u_newhead;
        uhp_mut(buf, uhp).uh_alt_next = old_curhead;
        if let Some(oc) = old_curhead {
            let oc_alt_prev = uhp_ref(buf, oc).uh_alt_prev;
            uhp_mut(buf, uhp).uh_alt_prev = oc_alt_prev;
            if let Some(ap) = oc_alt_prev {
                uhp_mut(buf, ap).uh_alt_next = Some(uhp);
            }
            uhp_mut(buf, oc).uh_alt_prev = Some(uhp);
            if buf.b_u_oldhead == Some(oc) {
                buf.b_u_oldhead = Some(uhp);
            }
        } else {
            uhp_mut(buf, uhp).uh_alt_prev = None;
        }

        if let Some(nh) = buf.b_u_newhead {
            uhp_mut(buf, nh).uh_prev = Some(uhp);
        }

        buf.b_u_seq_last += 1;
        let seq_last = buf.b_u_seq_last;
        uhp_mut(buf, uhp).uh_seq = seq_last;
        buf.b_u_seq_cur = seq_last;
        let now_time = os_time() as time_t;
        uhp_mut(buf, uhp).uh_time = now_time;
        uhp_mut(buf, uhp).uh_save_nr = 0;
        buf.b_u_time_cur = now_time + 1;

        uhp_mut(buf, uhp).uh_walk = 0;
        uhp_mut(buf, uhp).uh_entries.clear();
        uhp_mut(buf, uhp).uh_getbot_entry = false;
        let origin_cursor = atom_origin_pos(buf);
        uhp_mut(buf, uhp).uh_cursor = origin_cursor;
        if virtual_active(curwin_mut())
            && curwin_mut().w_cursor.coladd > 0
            && equalpos(origin_cursor, curwin_mut().w_cursor)
        {
            uhp_mut(buf, uhp).uh_cursor_vcol = getviscol();
        } else {
            uhp_mut(buf, uhp).uh_cursor_vcol = -1;
        }
        clearpos(&mut uhp_mut(buf, uhp).uh_cursor_after);

        uhp_mut(buf, uhp).uh_flags = (if buf.b_changed != 0 { UH_CHANGED } else { 0 })
            + (if (buf.b_ml.ml_flags & ML_EMPTY) != 0 {
                UH_EMPTYBUF
            } else {
                0
            });

        zero_fmark_additional_data(&mut buf.b_namedm);
        uhp_mut(buf, uhp).uh_namedm = buf.b_namedm.clone();
        uhp_mut(buf, uhp).uh_visual = buf.b_visual;

        buf.b_u_newhead = Some(uhp);
        if buf.b_u_oldhead.is_none() {
            buf.b_u_oldhead = Some(uhp);
        }
        buf.b_u_numhead += 1;
    } else {
        if get_undolevel(buf) < 0 {
            return true;
        }

        if size == 1 && u_get_headentry(buf).is_some() {
            let newhead = buf.b_u_newhead.unwrap();
            let limit = uhp_ref(buf, newhead).uh_entries.len().min(10);
            for i in 0..limit {
                let uep = &uhp_ref(buf, newhead).uh_entries[i];
                let ue_size = uep.ue_size();
                let ue_top = uep.ue_top;

                if u_entry_resized(buf, uep)
                    || (ue_size > 1 && top >= ue_top && top + 2 <= ue_top + ue_size + 1)
                {
                    break;
                }

                if ue_size == 1 && ue_top == top {
                    if i > 0 {
                        u_getbot(buf);
                        buf.b_u_synced = false;

                        let entry = uhp_mut(buf, newhead).uh_entries.remove(i);
                        uhp_mut(buf, newhead).uh_entries.insert(0, entry);
                    }

                    let line_count = buf.b_ml.ml_line_count;
                    let first_uep = &mut uhp_mut(buf, newhead).uh_entries.as_mut_slice()[0];
                    if newbot != 0 {
                        first_uep.ue_bot = newbot;
                    } else if bot > line_count {
                        first_uep.ue_bot = 0;
                    } else {
                        first_uep.ue_lcount = line_count;
                        uhp_mut(buf, newhead).uh_getbot_entry = true;
                    }
                    return true;
                }
            }
        }

        u_getbot(buf);
    }

    let mut uep = UEntry {
        ue_top: top,
        ue_bot: 0,
        ue_lcount: 0,
        ue_array: Vec::new(),
    };

    let newhead = buf.b_u_newhead.unwrap();
    if newbot != 0 {
        uep.ue_bot = newbot;
    } else if bot > buf.b_ml.ml_line_count {
        uep.ue_bot = 0;
    } else {
        uep.ue_lcount = buf.b_ml.ml_line_count;
        uhp_mut(buf, newhead).uh_getbot_entry = true;
    }

    if size > 0 {
        uep.ue_array.reserve(size as usize);
        for i in 0..size {
            fast_breakcheck();
            if g_st().got_int {
                uhp_mut(buf, newhead).uh_getbot_entry = false;
                return false;
            }
            uep.ue_array.push(u_save_line_buf(buf, top + 1 + i));
        }
    }

    uhp_mut(buf, newhead).uh_entries.insert(0, uep);
    if reload {
        uhp_mut(buf, newhead).uh_flags |= UH_RELOAD;
    }
    buf.b_u_synced = false;
    undo_st(|st| st.undo_undoes.set(false));

    true
}

fn u_entry_resized(buf: &FileBuf, uep: &UEntry) -> bool {
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
pub fn u_compute_hash(buf: &mut FileBuf, hash: &mut [u8; UNDO_HASH_SIZE]) {
    let mut ctx = ContextSha256::new();
    for lnum in 1..=buf.b_ml.ml_line_count {
        let slice = crate::memline::ml_get_buf_slice(buf, lnum);
        ctx.update(slice);
        ctx.update(&[0]);
    }
    ctx.finish_into(hash);
}

/// Return an allocated string of the full path of the target undofile.
pub fn u_get_undo_file_name(buf_ffname: Option<&CStr>, reading: bool) -> Option<std::ffi::CString> {
    let ffname_cstr = buf_ffname?;
    #[cfg(unix)]
    let mut ffname = ffname_cstr;
    #[cfg(not(unix))]
    let ffname = ffname_cstr;

    #[cfg(unix)]
    let mut fname_buf = [0u8; MAXPATHL];
    #[cfg(unix)]
    {
        if resolve_symlink(Some(ffname_cstr), &mut fname_buf) {
            ffname = crate::memory::bytes_as_cstr(&fname_buf);
        }
    }

    let mut dir_name = [0u8; MAXPATHL + 1];
    let mut munged_vec: Vec<u8> = Vec::new();
    let mut undo_file_name: Option<std::ffi::CString> = None;

    let ffname_slice = ffname.to_bytes();
    let ffname_len = ffname_slice.len();
    let p_udir = opt_vars().p_udir.to_owned();
    let mut dirp = p_udir.as_bytes();
    while !dirp.is_empty() {
        let mut dir_len = copy_option_part(&mut dirp, &mut dir_name[..MAXPATHL], c",");
        if dir_len == 1 && dir_name[0] == b'.' {
            let tail_off = path_tail(ffname).as_ptr() as usize - ffname.as_ptr() as usize;
            let mut ufn_vec: Vec<u8> = Vec::with_capacity(ffname_len + 5);
            ufn_vec.extend_from_slice(&ffname_slice[..tail_off]);
            ufn_vec.push(b'.');
            ufn_vec.extend_from_slice(&ffname_slice[tail_off..]);
            ufn_vec.extend_from_slice(b".un~");
            undo_file_name = Some(std::ffi::CString::new(ufn_vec).unwrap_or_default());
        } else {
            dir_name[dir_len] = 0;

            while dir_len > 1 && vim_ispathsep_nocolon(dir_name[dir_len - 1] as i32) {
                dir_len -= 1;
                dir_name[dir_len] = 0;
            }

            let mut has_directory = os_isdir(crate::memory::bytes_as_cstr(&dir_name));
            if !has_directory && dirp.is_empty() && !reading {
                let mut failed_dir: Option<std::ffi::CString> = None;
                let ret = os_mkdir_recurse(
                    crate::memory::bytes_as_cstr(&dir_name),
                    0o755,
                    &mut failed_dir,
                    None,
                );
                if ret != 0 {
                    semsg!(
                        gettext(c"E5003: Unable to create directory \"%s\" for undo file: %s",),
                        failed_dir.as_deref().unwrap_or(c""),
                        uv_strerror(ret),
                    );
                } else {
                    has_directory = true;
                }
            }
            if has_directory {
                if munged_vec.is_empty() {
                    munged_vec = ffname.to_bytes_with_nul().to_vec();
                    let mut idx = 0usize;
                    while idx < ffname_len {
                        if vim_ispathsep(i32::from(munged_vec[idx])) {
                            munged_vec[idx] = b'%';
                        }
                        idx += crate::mbyte::utfc_slice2len(&munged_vec[idx..ffname_len]);
                    }
                }
                let dir_str = NvimString::from_bytes(&dir_name[..dir_len]);
                let munged_name = NvimString::from_bytes(&munged_vec[..ffname_len]);
                undo_file_name = Some(concat_fnames(dir_str, munged_name, true));
            }
        }

        if let Some(ref ufn) = undo_file_name
            && (!reading || os_path_exists(ufn))
        {
            break;
        }
        undo_file_name = None;
    }

    undo_file_name
}

fn corruption_error(mesg: &std::ffi::CStr, file_name: &CStr) {
    semsg!(
        gettext(c"E825: Corrupted undo file (%s): %s"),
        mesg,
        file_name,
    );
}

fn serialize_header(bi: &mut Bufinfo, hash: &[u8; UNDO_HASH_SIZE]) -> bool {
    let buf = bi.bi_buf.borrow_mut();

    if !bi.bi_fp.write_all(&UF_START_MAGIC[..UF_START_MAGIC_LEN]) {
        return false;
    }

    undo_write_bytes(bi, UF_VERSION as u64, 2);

    if !undo_write(bi, hash) {
        return false;
    }

    undo_write_bytes(bi, buf.b_ml.ml_line_count as u64, 4);
    let len = buf.b_u_line_ptr.as_ref().map_or(0, |s| s.as_bytes().len());
    undo_write_bytes(bi, len as u64, 4);
    if len > 0 && !undo_write(bi, &buf.b_u_line_ptr.as_ref().unwrap().as_bytes()[..len]) {
        return false;
    }
    undo_write_bytes(bi, buf.b_u_line_lnum as u64, 4);
    undo_write_bytes(bi, buf.b_u_line_colnr as u64, 4);

    put_header_ptr(bi, buf.b_u_oldhead);
    put_header_ptr(bi, buf.b_u_newhead);
    put_header_ptr(bi, buf.b_u_curhead);

    undo_write_bytes(bi, buf.b_u_numhead as u64, 4);
    undo_write_bytes(bi, buf.b_u_seq_last as u64, 4);
    undo_write_bytes(bi, buf.b_u_seq_cur as u64, 4);
    let time_buf = (buf.b_u_time_cur as u64).to_be_bytes();
    undo_write(bi, &time_buf);

    undo_write_bytes(bi, 4, 1);
    undo_write_bytes(bi, UF_LAST_SAVE_NR as u64, 1);
    undo_write_bytes(bi, buf.b_u_save_nr_last as u64, 4);

    undo_write_bytes(bi, 0, 1);

    true
}

fn serialize_uhp(bi: &mut Bufinfo, uhp: usize) -> bool {
    if !undo_write_bytes(bi, UF_HEADER_MAGIC as u64, 2) {
        return false;
    }

    let buf = bi.bi_buf.borrow_mut();
    put_header_ptr(bi, uhp_ref(buf, uhp).uh_next);
    put_header_ptr(bi, uhp_ref(buf, uhp).uh_prev);
    put_header_ptr(bi, uhp_ref(buf, uhp).uh_alt_next);
    put_header_ptr(bi, uhp_ref(buf, uhp).uh_alt_prev);
    undo_write_bytes(bi, uhp_ref(buf, uhp).uh_seq as u64, 4);
    serialize_pos(bi, uhp_ref(buf, uhp).uh_cursor);
    undo_write_bytes(bi, uhp_ref(buf, uhp).uh_cursor_vcol as u64, 4);
    undo_write_bytes(bi, uhp_ref(buf, uhp).uh_flags as u64, 2);
    for i in 0..NMARKS {
        serialize_pos(bi, uhp_ref(buf, uhp).uh_namedm[i].mark);
    }
    serialize_visualinfo(bi, &mut uhp_mut(buf, uhp).uh_visual);
    let time_buf = (uhp_ref(buf, uhp).uh_time as u64).to_be_bytes();
    undo_write(bi, &time_buf);

    undo_write_bytes(bi, 4, 1);
    undo_write_bytes(bi, UHP_SAVE_NR as u64, 1);
    undo_write_bytes(bi, uhp_ref(buf, uhp).uh_save_nr as u64, 4);

    undo_write_bytes(bi, 0, 1);

    let entries_len = uhp_ref(buf, uhp).uh_entries.len();
    for i in 0..entries_len {
        undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
        let uep = &uhp_ref(buf, uhp).uh_entries[i];
        if !serialize_uep(bi, uep) {
            return false;
        }
    }
    undo_write_bytes(bi, UF_ENTRY_END_MAGIC as u64, 2);

    let extmark_len = uhp_ref(buf, uhp).uh_extmark.len();
    for i in 0..extmark_len {
        let item = uhp_ref(buf, uhp).uh_extmark[i];
        if !serialize_extmark(bi, item) {
            return false;
        }
    }
    undo_write_bytes(bi, UF_ENTRY_END_MAGIC as u64, 2);

    true
}

fn unserialize_uhp(bi: &mut Bufinfo, file_name: &CStr) -> Option<(UHeader, [i32; 4])> {
    let mut uhp = UHeader::new();
    let seqs = [
        undo_read_4c(bi),
        undo_read_4c(bi),
        undo_read_4c(bi),
        undo_read_4c(bi),
    ];
    uhp.uh_seq = undo_read_4c(bi);
    if uhp.uh_seq <= 0 {
        corruption_error(c"uh_seq", file_name);
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
            corruption_error(c"truncated", file_name);
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
        corruption_error(c"entry end", file_name);
        return None;
    }

    c = undo_read_2c(bi);
    while c == UF_ENTRY_MAGIC {
        let extup = unserialize_extmark(bi)?;
        uhp.uh_extmark.push(extup);
        c = undo_read_2c(bi);
    }
    if c != UF_ENTRY_END_MAGIC {
        corruption_error(c"entry end", file_name);
        return None;
    }

    Some((uhp, seqs))
}

fn serialize_extmark(bi: &mut Bufinfo, extup: ExtmarkUndoObject) -> bool {
    match extup {
        ExtmarkUndoObject::Splice(splice) => {
            undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
            undo_write_bytes(bi, K_EXTMARK_SPLICE as u64, 4);
            let bytes = splice.to_ne_bytes();
            if !undo_write(bi, &bytes) {
                return false;
            }
        }
        ExtmarkUndoObject::Move(move_) => {
            undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
            undo_write_bytes(bi, K_EXTMARK_MOVE as u64, 4);
            let bytes = move_.to_ne_bytes();
            if !undo_write(bi, &bytes) {
                return false;
            }
        }
        ExtmarkUndoObject::SavePos(_) => {}
    }
    true
}

fn unserialize_extmark(bi: &mut Bufinfo) -> Option<ExtmarkUndoObject> {
    let type_ = undo_read_4c(bi) as UndoObjectType;
    if type_ == K_EXTMARK_SPLICE {
        let mut bytes = [0u8; size_of::<ExtmarkSplice>()];
        if !undo_read(bi, &mut bytes) {
            return None;
        }
        Some(ExtmarkUndoObject::Splice(ExtmarkSplice::from_ne_bytes(
            &bytes,
        )))
    } else if type_ == K_EXTMARK_MOVE {
        let mut bytes = [0u8; size_of::<ExtmarkMove>()];
        if !undo_read(bi, &mut bytes) {
            return None;
        }
        Some(ExtmarkUndoObject::Move(ExtmarkMove::from_ne_bytes(&bytes)))
    } else {
        None
    }
}

fn serialize_uep(bi: &mut Bufinfo, uep: &UEntry) -> bool {
    undo_write_bytes(bi, uep.ue_top as u64, 4);
    undo_write_bytes(bi, uep.ue_bot as u64, 4);
    undo_write_bytes(bi, uep.ue_lcount as u64, 4);
    undo_write_bytes(bi, uep.ue_size() as u64, 4);

    for line in &uep.ue_array {
        let bytes = line.as_bytes();
        let len = bytes.len();
        if !undo_write_bytes(bi, len as u64, 4) {
            return false;
        }
        if len > 0 && !undo_write(bi, bytes) {
            return false;
        }
    }
    true
}

fn unserialize_uep(bi: &mut Bufinfo, error: &mut bool, file_name: &CStr) -> UEntry {
    let mut uep = UEntry {
        ue_top: undo_read_4c(bi),
        ue_bot: undo_read_4c(bi),
        ue_lcount: undo_read_4c(bi),
        ue_array: Vec::new(),
    };
    if uep.ue_top < 0 || uep.ue_bot < 0 || uep.ue_lcount < 0 {
        corruption_error(c"entry lnum", file_name);
        *error = true;
        return uep;
    }

    let size = undo_read_len(bi, c"entry size", file_name);
    if size < 0 {
        *error = true;
        return uep;
    }

    if size > 0 {
        uep.ue_array.reserve(size as usize);
    }

    for _ in 0..size as usize {
        let line_len = undo_read_len(bi, c"line length", file_name);
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

fn serialize_pos(bi: &mut Bufinfo, pos: CursorPos) {
    undo_write_bytes(bi, pos.lnum as u64, 4);
    undo_write_bytes(bi, pos.col as u64, 4);
    undo_write_bytes(bi, pos.coladd as u64, 4);
}

fn unserialize_pos(bi: &mut Bufinfo, pos: &mut CursorPos) {
    pos.lnum = undo_read_4c(bi).max(0);
    pos.col = undo_read_4c(bi).max(0);
    pos.coladd = undo_read_4c(bi).max(0);
}

fn serialize_visualinfo(bi: &mut Bufinfo, info: &mut Visualinfo) {
    serialize_pos(bi, info.vi_start);
    serialize_pos(bi, info.vi_end);
    undo_write_bytes(bi, info.vi_mode as u64, 4);
    undo_write_bytes(bi, info.vi_curswant as u64, 4);
}

fn unserialize_visualinfo(bi: &mut Bufinfo, info: &mut Visualinfo) {
    unserialize_pos(bi, &mut info.vi_start);
    unserialize_pos(bi, &mut info.vi_end);
    info.vi_mode = undo_read_4c(bi);
    info.vi_curswant = undo_read_4c(bi);
}

/// Write the undo tree in an undo file.
pub fn u_write_undo(
    name: Option<&CStr>,
    forceit: bool,
    buf: &mut FileBuf,
    hash: &[u8; UNDO_HASH_SIZE],
) {
    let f_owned: Option<std::ffi::CString>;
    let file_name_cstr: &CStr = if let Some(n) = name {
        n
    } else {
        f_owned = u_get_undo_file_name(buf.ffname_opt(), false);
        let Some(ref f) = f_owned else {
            if opt_vars().p_verbose > 0 {
                verbose_enter();
                smsg!(
                    0,
                    c"%s",
                    gettext(c"Cannot write undo file in any directory in 'undodir'"),
                );
                verbose_leave();
            }
            return;
        };
        f.as_c_str()
    };

    let mut perm: i32 = 0o600;
    if buf.b_ffname.is_some() {
        perm = os_getperm(buf.ffname_cstr());
        if perm < 0 {
            perm = 0o600;
        }
    }
    perm &= 0o666;

    'theend: {
        if os_path_exists(file_name_cstr) {
            if name.is_none() || !forceit {
                let fd = os_open(file_name_cstr, O_RDONLY, 0);
                if fd < 0 {
                    if name.is_some() || opt_vars().p_verbose > 0 {
                        if name.is_none() {
                            verbose_enter();
                        }
                        smsg!(
                            0,
                            gettext(c"Will not overwrite with undo file, cannot read: %s",),
                            file_name_cstr,
                        );
                        if name.is_none() {
                            verbose_leave();
                        }
                    }
                    break 'theend;
                } else {
                    let mut mbuf = [0u8; UF_START_MAGIC_LEN];
                    let len = read_eintr(fd, &mut mbuf);
                    os_close(fd);
                    if len < UF_START_MAGIC_LEN as isize
                        || mbuf != UF_START_MAGIC[..UF_START_MAGIC_LEN]
                    {
                        if name.is_some() || opt_vars().p_verbose > 0 {
                            if name.is_none() {
                                verbose_enter();
                            }
                            smsg!(
                                0,
                                gettext(c"Will not overwrite, this is not an undo file: %s",),
                                file_name_cstr,
                            );
                            if name.is_none() {
                                verbose_leave();
                            }
                        }
                        break 'theend;
                    }
                }
            }
            os_remove(file_name_cstr);
        }

        if buf.b_u_numhead == 0 && buf.b_u_line_ptr.is_none() {
            if opt_vars().p_verbose > 0 {
                verb_msg(gettext(c"Skipping undo file write, nothing to undo"));
            }
            break 'theend;
        }

        let fd = os_open(
            file_name_cstr,
            O_CREAT | O_WRONLY | O_EXCL | O_NOFOLLOW,
            perm,
        );
        if fd < 0 {
            semsg!(gettext(E_NOT_OPEN), file_name_cstr);
            break 'theend;
        }
        os_setperm(file_name_cstr, perm);
        if opt_vars().p_verbose > 0 {
            verbose_enter();
            smsg!(0, gettext(c"Writing undo file: %s"), file_name_cstr);
            verbose_leave();
        }

        #[cfg(unix)]
        {
            let mut file_info_old = FileInfo::new();
            let mut file_info_new = FileInfo::new();
            if buf.b_ffname.is_some()
                && os_fileinfo(buf.ffname_cstr(), &mut file_info_old)
                && os_fileinfo(file_name_cstr, &mut file_info_new)
                && file_info_old.stat.st_gid != file_info_new.stat.st_gid
                && os_fchown(
                    fd,
                    u32::MAX as uv_uid_t,
                    file_info_old.stat.st_gid as uv_gid_t,
                ) != 0
            {
                os_setperm(file_name_cstr, (perm & 0o707) | ((perm & 0o7) << 3));
            }
        }

        let Some(bi_fp) = CFile::from_fd(fd, c"w") else {
            semsg!(gettext(E_NOT_OPEN), file_name_cstr);
            os_close(fd);
            os_remove(file_name_cstr);
            break 'theend;
        };

        u_sync(true);

        let mut write_ok = false;
        let mut bi = Bufinfo {
            bi_buf: crate::globals::BufPtr::from(&mut *buf),
            bi_fp,
            bi_fsize: 0,
        };

        'write_error: {
            if !serialize_header(&mut bi, hash) {
                break 'write_error;
            }

            let mark = undo_st(|st| {
                let m = st.lastmark.get() + 1;
                st.lastmark.set(m);
                m
            });
            let mut uhp = buf.b_u_oldhead;
            while let Some(uh) = uhp {
                if uhp_ref(buf, uh).uh_walk != mark {
                    uhp_mut(buf, uh).uh_walk = mark;
                    if !serialize_uhp(&mut bi, uh) {
                        break 'write_error;
                    }
                }

                let prev = uhp_ref(buf, uh).uh_prev;
                let alt_next = uhp_ref(buf, uh).uh_alt_next;
                let next = uhp_ref(buf, uh).uh_next;
                let alt_prev = uhp_ref(buf, uh).uh_alt_prev;
                if let Some(p) = prev
                    && uhp_ref(buf, p).uh_walk != mark
                {
                    uhp = Some(p);
                } else if let Some(an) = alt_next
                    && uhp_ref(buf, an).uh_walk != mark
                {
                    uhp = Some(an);
                } else if let Some(n) = next
                    && alt_prev.is_none()
                    && uhp_ref(buf, n).uh_walk != mark
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

            let do_fsync = buf.b_p_fs.unwrap_or(opt_vars().p_fs);
            if do_fsync && bi.bi_fp.flush() == 0 && os_fsync(fd) != 0 {
                write_ok = false;
            }
        }

        drop(bi);
        if !write_ok {
            semsg!(gettext(E_WRITE_ERROR_IN_UNDO_FILE_STR), file_name_cstr);
        }

        if buf.b_ffname.is_some() {
            let acl: VimAcl = os_get_acl(buf.ffname_cstr());
            os_set_acl(file_name_cstr, acl);
            os_free_acl(acl);
        }
    }
}

fn uhp_table_find(uhp_table: &[(UHeader, [i32; 4])], seq: i32) -> i32 {
    let mut lo: i32 = 0;
    let mut hi: i32 = uhp_table.len() as i32 - 1;

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
pub fn u_read_undo(name: Option<&CStr>, hash: &[u8; UNDO_HASH_SIZE], orig_name: Option<&CStr>) {
    #[cfg(not(unix))]
    let _ = orig_name;
    let mut uhp_table: Vec<(UHeader, [i32; 4])> = Vec::new();
    let mut line_ptr: Option<std::ffi::CString> = None;
    let mut num_read_uhps: i32 = 0;

    let f_owned: Option<std::ffi::CString>;
    let file_name_cstr: &CStr = if let Some(n) = name {
        n
    } else {
        f_owned = u_get_undo_file_name(curbuf_mut().ffname_opt(), true);
        let Some(ref f_cstr) = f_owned else {
            return;
        };

        #[cfg(unix)]
        {
            let mut file_info_orig = FileInfo::new();
            let mut file_info_undo = FileInfo::new();
            if os_fileinfo(orig_name.unwrap_or(c""), &mut file_info_orig)
                && os_fileinfo(f_cstr, &mut file_info_undo)
                && file_info_orig.stat.st_uid != file_info_undo.stat.st_uid
                && file_info_undo.stat.st_uid != u64::from(getuid())
            {
                if opt_vars().p_verbose > 0 {
                    verbose_enter();
                    smsg!(
                        0,
                        gettext(c"Not reading undo file, owner differs: %s"),
                        f_cstr,
                    );
                    verbose_leave();
                }
                return;
            }
        }
        f_cstr.as_c_str()
    };

    if opt_vars().p_verbose > 0 {
        verbose_enter();
        smsg!(0, gettext(c"Reading undo file: %s"), file_name_cstr);
        verbose_leave();
    }

    let fp_opt = CFile::open(file_name_cstr, c"r");
    'theend: {
        'error: {
            let Some(bi_fp) = fp_opt else {
                if name.is_some() || opt_vars().p_verbose > 0 {
                    semsg!(
                        gettext(c"E822: Cannot open undo file for reading: %s"),
                        file_name_cstr,
                    );
                }
                break 'error;
            };

            let mut file_info = FileInfo::new();
            let fsize = if os_fileinfo_fd(bi_fp.fileno(), &mut file_info) {
                os_fileinfo_size(&file_info) as Off
            } else {
                0
            };
            let mut bi = Bufinfo {
                bi_buf: g_st().curbuf,
                bi_fp,
                bi_fsize: fsize,
            };

            let mut magic_buf = [0u8; UF_START_MAGIC_LEN];
            if !undo_read(&mut bi, &mut magic_buf)
                || magic_buf != UF_START_MAGIC[..UF_START_MAGIC_LEN]
            {
                semsg!(gettext(c"E823: Not an undo file: %s"), file_name_cstr);
                break 'error;
            }
            let version = get2c(&bi.bi_fp);
            if version != UF_VERSION {
                semsg!(gettext(c"E824: Incompatible undo file: %s"), file_name_cstr,);
                break 'error;
            }

            let mut read_hash = [0u8; UNDO_HASH_SIZE];
            if !undo_read(&mut bi, &mut read_hash) {
                corruption_error(c"hash", file_name_cstr);
                break 'error;
            }
            let line_count = undo_read_4c(&mut bi) as Linenr;
            if *hash != read_hash || line_count != curbuf_mut().b_ml.ml_line_count {
                if opt_vars().p_verbose > 0 || name.is_some() {
                    if name.is_none() {
                        verbose_enter();
                    }
                    give_warning(
                        gettext(c"File contents changed, cannot use undo info"),
                        true,
                        true,
                    );
                    if name.is_none() {
                        verbose_leave();
                    }
                }
                break 'error;
            }

            let str_len = undo_read_len(&mut bi, c"line length", file_name_cstr);
            if str_len < 0 {
                break 'error;
            }

            if str_len > 0 {
                let Some(s) = undo_read_string(&mut bi, str_len as usize) else {
                    corruption_error(c"truncated", file_name_cstr);
                    break 'error;
                };
                line_ptr = Some(s);
            }
            let line_lnum = undo_read_4c(&mut bi) as Linenr;
            let line_colnr = undo_read_4c(&mut bi) as Colnr;
            if line_lnum < 0 || line_colnr < 0 {
                corruption_error(c"line lnum/col", file_name_cstr);
                break 'error;
            }

            let old_header_seq = undo_read_4c(&mut bi);
            let new_header_seq = undo_read_4c(&mut bi);
            let cur_header_seq = undo_read_4c(&mut bi);
            let num_head = undo_read_len(&mut bi, c"num_head", file_name_cstr);
            if num_head < 0 {
                break 'error;
            }
            let seq_last = undo_read_4c(&mut bi);
            let seq_cur = undo_read_4c(&mut bi);
            let seq_time = undo_read_time(&mut bi);

            let mut last_save_nr: i32 = 0;
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
                    corruption_error(c"num_head too small", file_name_cstr);
                    break 'error;
                }

                let Some(entry) = unserialize_uhp(&mut bi, file_name_cstr) else {
                    break 'error;
                };
                uhp_table.push(entry);
                num_read_uhps += 1;
                c = undo_read_2c(&mut bi);
            }

            if num_read_uhps != num_head {
                corruption_error(c"num_head", file_name_cstr);
                break 'error;
            }
            if c != UF_HEADER_END_MAGIC {
                corruption_error(c"end marker", file_name_cstr);
                break 'error;
            }

            if num_head > 0 {
                uhp_table.sort_unstable_by_key(|(u, _)| u.uh_seq);
            }

            for i in 0..(num_head - 1) {
                if uhp_table[i as usize].0.uh_seq == uhp_table[(i + 1) as usize].0.uh_seq {
                    corruption_error(c"duplicate uh_seq", file_name_cstr);
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

            u_blockfree(curbuf_mut());
            curbuf_mut().b_u_headers = uhp_table.into_iter().map(|(u, _)| u).collect();
            curbuf_mut().b_u_free_headers.clear();
            curbuf_mut().b_u_oldhead = old_idx;
            curbuf_mut().b_u_newhead = new_idx;
            curbuf_mut().b_u_curhead = cur_idx;
            curbuf_mut().b_u_line_ptr = line_ptr;
            curbuf_mut().b_u_line_lnum = line_lnum;
            curbuf_mut().b_u_line_colnr = line_colnr;
            curbuf_mut().b_u_numhead = num_head;
            curbuf_mut().b_u_seq_last = seq_last;
            curbuf_mut().b_u_seq_cur = seq_cur;
            curbuf_mut().b_u_time_cur = seq_time;
            curbuf_mut().b_u_save_nr_last = last_save_nr;
            curbuf_mut().b_u_save_nr_cur = last_save_nr;

            curbuf_mut().b_u_synced = true;

            if name.is_some() {
                smsg!(0, gettext(c"Finished reading undo file %s"), file_name_cstr,);
            }
            break 'theend;
        }

        drop(line_ptr);
        drop(uhp_table);
    }
}

fn undo_write(bi: &mut Bufinfo, bytes: &[u8]) -> bool {
    bi.bi_fp.write_all(bytes)
}

fn undo_write_bytes(bi: &mut Bufinfo, nr: u64, len: usize) -> bool {
    assert!(len > 0);
    let mut buf = [0u8; 8];
    let mut i = len - 1;
    for slot in buf.iter_mut().take(len) {
        *slot = (nr >> (i * 8)) as u8;
        i = i.wrapping_sub(1);
    }
    undo_write(bi, &buf[..len])
}

fn put_header_ptr(bi: &mut Bufinfo, uhp: Option<usize>) {
    let seq = if let Some(uh) = uhp {
        let s = uhp_ref(bi.bi_buf.borrow(), uh).uh_seq;
        assert!(s >= 0);
        s as u64
    } else {
        0
    };
    undo_write_bytes(bi, seq, 4);
}

fn undo_read_4c(bi: &mut Bufinfo) -> i32 {
    get4c(&bi.bi_fp)
}

fn undo_read_len(bi: &mut Bufinfo, what: &std::ffi::CStr, file_name: &CStr) -> i32 {
    let len = undo_read_4c(bi);
    let pos = bi.bi_fp.tell();
    if len < 0 || (bi.bi_fsize > 0 && (pos < 0 || (len as Off) > bi.bi_fsize - pos)) {
        corruption_error(
            if bi.bi_fp.is_eof() {
                c"truncated"
            } else {
                what
            },
            file_name,
        );
        return -1;
    }
    len
}

fn undo_read_2c(bi: &mut Bufinfo) -> i32 {
    get2c(&bi.bi_fp)
}

fn undo_read_byte(bi: &mut Bufinfo) -> i32 {
    bi.bi_fp.getc()
}

fn undo_read_time(bi: &mut Bufinfo) -> time_t {
    get8ctime(&bi.bi_fp)
}

fn undo_read(bi: &mut Bufinfo, slice: &mut [u8]) -> bool {
    let retval = bi.bi_fp.read_exact(slice);
    if !retval {
        slice.fill(0);
    }
    retval
}

fn undo_read_string(bi: &mut Bufinfo, len: usize) -> Option<std::ffi::CString> {
    if len == 0 {
        return Some(std::ffi::CString::default());
    }
    let mut buf = vec![0u8; len];
    if !undo_read(bi, &mut buf) {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(len);
    buf.truncate(end);
    std::ffi::CString::new(buf).ok()
}

/// Undo `count` times.
pub fn u_undo(mut count: i32) {
    if !curbuf_mut().b_u_synced {
        u_sync(true);
        count = 1;
    }

    if !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_UNDO) {
        undo_st(|st| st.undo_undoes.set(true));
    } else {
        undo_st(|st| st.undo_undoes.set(!st.undo_undoes.get()));
    }
    u_doit(count, false, true);
}

/// Redo `count` times.
pub fn u_redo(count: i32) {
    if !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_UNDO) {
        undo_st(|st| st.undo_undoes.set(false));
    }

    u_doit(count, false, true);
}

/// Undo and remove the branch from the undo tree.
pub fn u_undo_and_forget(mut count: i32, do_buf_event: bool) -> bool {
    if !curbuf_mut().b_u_synced {
        u_sync(true);
        count = 1;
    }
    undo_st(|st| st.undo_undoes.set(true));
    u_doit(count, true, do_buf_event);

    let Some(curhead) = curbuf_mut().b_u_curhead else {
        return false;
    };

    u_forget_header(curbuf_mut(), curhead);
    true
}

fn u_doit(startcount: i32, quiet: bool, do_buf_event: bool) {
    if !undo_allowed(curbuf_mut()) {
        return;
    }
    atom_did_global_op();

    let empty = (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0;
    undo_st(|st| {
        st.u_newcount.set(0);
        st.u_oldcount.set(if empty { -1 } else { 0 });
    });

    msg_ext_set_kind(c"undo");
    let mut count = startcount;
    while {
        let prev = count;
        count -= 1;
        prev != 0
    } {
        change_warning(curbuf_mut(), 0);

        if undo_st(|st| st.undo_undoes.get()) {
            if curbuf_mut().b_u_curhead.is_none() {
                curbuf_mut().b_u_curhead = curbuf_mut().b_u_newhead;
            } else if get_undolevel(curbuf_mut()) > 0 {
                let ch = curbuf_mut().b_u_curhead.unwrap();
                curbuf_mut().b_u_curhead = uhp_ref(curbuf_mut(), ch).uh_next;
            }
            if curbuf_mut().b_u_numhead == 0 || curbuf_mut().b_u_curhead.is_none() {
                curbuf_mut().b_u_curhead = curbuf_mut().b_u_oldhead;
                beep_flush();
                if count == startcount - 1 {
                    if !shortmess(K_SHM_UNDO) {
                        msg(gettext(c"Already at oldest change"), 0);
                    }
                    return;
                }
                break;
            }

            u_undoredo(true, do_buf_event);
        } else {
            if curbuf_mut().b_u_curhead.is_none() || get_undolevel(curbuf_mut()) <= 0 {
                beep_flush();
                if count == startcount - 1 {
                    if !shortmess(K_SHM_UNDO) {
                        msg(gettext(c"Already at newest change"), 0);
                    }
                    return;
                }
                break;
            }

            u_undoredo(false, do_buf_event);

            let curhead = curbuf_mut().b_u_curhead.unwrap();
            let prev = uhp_ref(curbuf_mut(), curhead).uh_prev;
            if prev.is_none() {
                curbuf_mut().b_u_newhead = Some(curhead);
            }
            curbuf_mut().b_u_curhead = prev;
        }
    }
    u_undo_end(undo_st(|st| st.undo_undoes.get()), false, quiet);
}

/// Undo or redo over the timeline.
pub fn undo_time(step: i32, sec: bool, file: bool, absolute: bool) {
    if text_locked() {
        text_locked_msg();
        return;
    }
    mc_undo_time();

    if !curbuf_mut().b_u_synced {
        u_sync(true);
    }

    let empty = (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0;
    undo_st(|st| {
        st.u_newcount.set(0);
        st.u_oldcount.set(if empty { -1 } else { 0 });
    });

    let mut target: i32;
    let mut closest: i32;
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
            target = (curbuf_mut().b_u_time_cur as i32) + step;
        } else if dofile {
            if step < 0 {
                uhp = if let Some(ch) = curbuf_mut().b_u_curhead {
                    uhp_ref(curbuf_mut(), ch).uh_next
                } else {
                    curbuf_mut().b_u_newhead
                };
                if uhp.is_some_and(|uh| uhp_ref(curbuf_mut(), uh).uh_save_nr != 0) {
                    target = curbuf_mut().b_u_save_nr_cur + step;
                } else {
                    target = curbuf_mut().b_u_save_nr_cur + step + 1;
                }
                if target <= 0 {
                    dofile = false;
                }
            } else {
                target = curbuf_mut().b_u_save_nr_cur + step;
                if target > curbuf_mut().b_u_save_nr_last {
                    target = curbuf_mut().b_u_seq_last + 1;
                    dofile = false;
                }
            }
        } else {
            target = curbuf_mut().b_u_seq_cur + step;
        }
        if step < 0 {
            target = target.max(0);
            closest = -1;
        } else {
            if dosec {
                closest = (os_time() + 1) as i32;
            } else if dofile {
                closest = curbuf_mut().b_u_save_nr_last + 2;
            } else {
                closest = curbuf_mut().b_u_seq_last + 2;
            }
            if target >= closest {
                target = closest - 1;
            }
        }
    }
    let closest_start = closest;
    let mut closest_seq = curbuf_mut().b_u_seq_cur;
    let mut mark = undo_st(|st| st.lastmark.get());
    let mut nomark: i32 = 0;

    if target != 0 {
        for round in 1..=2 {
            undo_st(|st| {
                let m = st.lastmark.get() + 1;
                mark = m;
                nomark = m + 1;
                st.lastmark.set(nomark);
            });

            uhp = curbuf_mut().b_u_curhead.or(curbuf_mut().b_u_newhead);

            while let Some(uh) = uhp {
                uhp_mut(curbuf_mut(), uh).uh_walk = mark;
                let val = if dosec {
                    uhp_ref(curbuf_mut(), uh).uh_time as i32
                } else if dofile {
                    uhp_ref(curbuf_mut(), uh).uh_save_nr
                } else {
                    uhp_ref(curbuf_mut(), uh).uh_seq
                };

                let uh_seq = uhp_ref(curbuf_mut(), uh).uh_seq;
                if round == 1 && !(dofile && val == 0) {
                    let dir_ok = if step < 0 {
                        uh_seq <= curbuf_mut().b_u_seq_cur
                    } else {
                        uh_seq > curbuf_mut().b_u_seq_cur
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

                let prev = uhp_ref(curbuf_mut(), uh).uh_prev;
                let alt_next = uhp_ref(curbuf_mut(), uh).uh_alt_next;
                let next = uhp_ref(curbuf_mut(), uh).uh_next;
                let alt_prev = uhp_ref(curbuf_mut(), uh).uh_alt_prev;
                if let Some(p) = prev
                    && uhp_ref(curbuf_mut(), p).uh_walk != nomark
                    && uhp_ref(curbuf_mut(), p).uh_walk != mark
                {
                    uhp = Some(p);
                } else if let Some(an) = alt_next
                    && uhp_ref(curbuf_mut(), an).uh_walk != nomark
                    && uhp_ref(curbuf_mut(), an).uh_walk != mark
                {
                    uhp = Some(an);
                } else if let Some(n) = next
                    && alt_prev.is_none()
                    && uhp_ref(curbuf_mut(), n).uh_walk != nomark
                    && uhp_ref(curbuf_mut(), n).uh_walk != mark
                {
                    if Some(uh) == curbuf_mut().b_u_curhead {
                        uhp_mut(curbuf_mut(), uh).uh_walk = nomark;
                    }
                    uhp = Some(n);
                } else {
                    uhp_mut(curbuf_mut(), uh).uh_walk = nomark;
                    uhp = alt_prev.or(next);
                }
            }

            if uhp.is_some() {
                break;
            }

            if absolute {
                semsg!(gettext(c"E830: Undo number %lld not found"), step as i64,);
                return;
            }

            if closest == closest_start {
                if !shortmess(K_SHM_UNDO) {
                    if step < 0 {
                        msg(gettext(c"Already at oldest change"), 0);
                    } else {
                        msg(gettext(c"Already at newest change"), 0);
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
            change_warning(curbuf_mut(), 0);

            uhp = if let Some(ch) = curbuf_mut().b_u_curhead {
                uhp_ref(curbuf_mut(), ch).uh_next
            } else {
                curbuf_mut().b_u_newhead
            };
            let Some(uh) = uhp else {
                break;
            };
            if (target > 0 && uhp_ref(curbuf_mut(), uh).uh_walk != mark)
                || (uhp_ref(curbuf_mut(), uh).uh_seq == target && !above)
            {
                break;
            }
            curbuf_mut().b_u_curhead = Some(uh);
            u_undoredo(true, true);
            if target > 0 {
                uhp_mut(curbuf_mut(), uh).uh_walk = nomark;
            }
        }

        if target > 0 {
            while !g_st().got_int {
                change_warning(curbuf_mut(), 0);

                let Some(mut uh) = curbuf_mut().b_u_curhead else {
                    break;
                };

                while let Some(ap) = uhp_ref(curbuf_mut(), uh).uh_alt_prev
                    && uhp_ref(curbuf_mut(), ap).uh_walk == mark
                {
                    uh = ap;
                }

                let mut last = uh;
                while let Some(an) = uhp_ref(curbuf_mut(), last).uh_alt_next
                    && uhp_ref(curbuf_mut(), an).uh_walk == mark
                {
                    last = an;
                }
                if last != uh {
                    while let Some(ap) = uhp_ref(curbuf_mut(), uh).uh_alt_prev {
                        uh = ap;
                    }
                    let last_alt_next = uhp_ref(curbuf_mut(), last).uh_alt_next;
                    let last_alt_prev = uhp_ref(curbuf_mut(), last).uh_alt_prev;
                    if let Some(an) = last_alt_next {
                        uhp_mut(curbuf_mut(), an).uh_alt_prev = last_alt_prev;
                    }
                    if let Some(ap) = last_alt_prev {
                        uhp_mut(curbuf_mut(), ap).uh_alt_next = last_alt_next;
                    }
                    uhp_mut(curbuf_mut(), last).uh_alt_prev = None;
                    uhp_mut(curbuf_mut(), last).uh_alt_next = Some(uh);
                    uhp_mut(curbuf_mut(), uh).uh_alt_prev = Some(last);

                    if curbuf_mut().b_u_oldhead == Some(uh) {
                        curbuf_mut().b_u_oldhead = Some(last);
                    }
                    uh = last;
                    if let Some(n) = uhp_ref(curbuf_mut(), uh).uh_next {
                        uhp_mut(curbuf_mut(), n).uh_prev = Some(uh);
                    }
                }
                curbuf_mut().b_u_curhead = Some(uh);

                if uhp_ref(curbuf_mut(), uh).uh_walk != mark {
                    break;
                }

                if uhp_ref(curbuf_mut(), uh).uh_seq == target && above {
                    curbuf_mut().b_u_seq_cur = target - 1;
                    break;
                }

                u_undoredo(false, true);

                let prev = uhp_ref(curbuf_mut(), uh).uh_prev;
                if prev.is_none() {
                    curbuf_mut().b_u_newhead = Some(uh);
                }
                curbuf_mut().b_u_curhead = prev;
                did_undo = false;

                if uhp_ref(curbuf_mut(), uh).uh_seq == target {
                    break;
                }

                uhp = prev;
                if uhp.is_none_or(|p| uhp_ref(curbuf_mut(), p).uh_walk != mark) {
                    internal_error(c"undo_time()");
                    break;
                }
            }
        }
    }
    u_undo_end(did_undo, absolute, false);
}

fn u_undoredo(undo: bool, do_buf_event: bool) {
    let mut newlnum: Linenr = MAXLNUM;
    let mut new_curpos = curwin_mut().w_cursor;
    let curhead_idx = curbuf_mut().b_u_curhead.unwrap();
    let curhead = uhp_mut(curbuf_mut(), curhead_idx);

    block_autocmds();

    let old_flags = curhead.uh_flags;
    let new_flags = (if curbuf_mut().b_changed != 0 {
        UH_CHANGED
    } else {
        0
    }) | (if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
        UH_EMPTYBUF
    } else {
        0
    }) | (old_flags & UH_RELOAD);
    setpcmark();

    zero_fmark_additional_data(&mut curbuf_mut().b_namedm);
    let namedm = curbuf_mut().b_namedm.clone();
    let visualinfo = curbuf_mut().b_visual;
    curbuf_mut().b_op_start.lnum = curbuf_mut().b_ml.ml_line_count;
    curbuf_mut().b_op_start.col = 0;
    curbuf_mut().b_op_end.lnum = 0;
    curbuf_mut().b_op_end.col = 0;

    let entries_len = curhead.uh_entries.len();
    for entry_idx in 0..entries_len {
        let is_last = entry_idx + 1 == entries_len;
        let uep = &mut curhead.uh_entries.as_mut_slice()[entry_idx];
        let top = uep.ue_top;
        let mut bot = uep.ue_bot;
        if bot == 0 {
            bot = curbuf_mut().b_ml.ml_line_count + 1;
        }
        if top > curbuf_mut().b_ml.ml_line_count
            || top >= bot
            || bot > curbuf_mut().b_ml.ml_line_count + 1
        {
            unblock_autocmds();
            iemsg(gettext(c"E438: u_undo: line numbers wrong"));
            changed(crate::globals::curbuf_mut());
            return;
        }

        let oldsize = bot - top - 1;
        let newsize = uep.ue_size();

        {
            let lnum = curhead.uh_cursor.lnum;
            if lnum >= top && lnum <= top + newsize + 1 {
                new_curpos = curhead.uh_cursor;
                newlnum = -1;
            } else if top < newlnum {
                let mut i: Linenr = 0;
                while i < newsize && i < oldsize {
                    if uep.ue_array.as_slice()[i as usize].as_bytes()
                        != crate::memline::ml_get_slice(top + 1 + i)
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
                if curbuf_mut().b_ml.ml_line_count == 1 {
                    empty_buffer = true;
                }
                ml_delete(lnum);
                lnum -= 1;
            }
        }

        check_cursor_lnum(curwin_mut());

        if newsize != 0 {
            for (i, line) in uep.ue_array.drain(..).enumerate() {
                let lnum = top + i as Linenr;
                if empty_buffer && lnum == 0 {
                    ml_replace(1, line.to_bytes(), true);
                } else {
                    ml_append_flags(lnum, line.to_bytes(), 0, 0);
                }
            }
        }

        if oldsize != newsize {
            mark_adjust(
                top + 1,
                top + oldsize,
                MAXLNUM,
                newsize - oldsize,
                K_EXTMARK_NOOP,
            );
            if curbuf_mut().b_op_start.lnum > top + oldsize {
                curbuf_mut().b_op_start.lnum += newsize - oldsize;
            }
            if curbuf_mut().b_op_end.lnum > top + oldsize {
                curbuf_mut().b_op_end.lnum += newsize - oldsize;
            }
        }

        if oldsize > 0 || newsize > 0 {
            changed_lines(
                curbuf_mut(),
                top + 1,
                0,
                bot,
                newsize - oldsize,
                do_buf_event,
            );
            if spell_check_window(curwin_mut()) && bot <= curbuf_mut().b_ml.ml_line_count {
                redraw_winline(crate::globals::curwin_mut(), bot);
            }
        }

        curbuf_mut().b_op_start.lnum = curbuf_mut().b_op_start.lnum.min(top + 1);
        if newsize == 0 && top + 1 > curbuf_mut().b_op_end.lnum {
            curbuf_mut().b_op_end.lnum = top + 1;
        } else if top + newsize > curbuf_mut().b_op_end.lnum {
            curbuf_mut().b_op_end.lnum = top + newsize;
        }

        undo_st(|st| {
            st.u_newcount.set(st.u_newcount.get() + newsize);
            st.u_oldcount.set(st.u_oldcount.get() + oldsize);
        });
        uep.ue_array = newarray;
        uep.ue_bot = top + newsize + 1;
    }

    curbuf_mut().b_op_start.lnum = curbuf_mut()
        .b_op_start
        .lnum
        .min(curbuf_mut().b_ml.ml_line_count);
    curbuf_mut().b_op_end.lnum = curbuf_mut()
        .b_op_end
        .lnum
        .min(curbuf_mut().b_ml.ml_line_count);

    if undo {
        for &item in curhead.uh_extmark.iter().rev() {
            extmark_apply_undo(item, undo);
        }
    } else {
        for &item in &curhead.uh_extmark {
            extmark_apply_undo(item, undo);
        }
    }
    if (curhead.uh_flags & UH_RELOAD) != 0 {
        buf_updates_unload(curbuf_mut(), true);
    }

    curwin_mut().w_cursor = new_curpos;
    check_cursor_lnum(curwin_mut());

    curhead.uh_entries.reverse();
    curhead.uh_flags = new_flags;
    if (old_flags & UH_EMPTYBUF) != 0 && buf_is_empty(curbuf_mut()) {
        curbuf_mut().b_ml.ml_flags |= ML_EMPTY;
    }
    if (old_flags & UH_CHANGED) != 0 {
        changed(crate::globals::curbuf_mut());
    } else {
        unchanged(curbuf_mut(), false, true);
    }

    if do_buf_event {
        buf_updates_changedtick(curbuf_mut());
    }

    for (i, nm) in namedm.into_iter().enumerate().take(NMARKS) {
        if curhead.uh_namedm[i].mark.lnum != 0 {
            curbuf_mut().b_namedm[i] = curhead.uh_namedm[i].clone();
        }
        if nm.mark.lnum != 0 {
            curhead.uh_namedm[i] = nm;
        } else {
            curhead.uh_namedm[i].mark.lnum = 0;
        }
    }
    if curhead.uh_visual.vi_start.lnum != 0 {
        curbuf_mut().b_visual = curhead.uh_visual;
        curhead.uh_visual = visualinfo;
    }

    if undo
        && curhead.uh_cursor.lnum >= 1
        && curhead.uh_cursor.lnum <= curbuf_mut().b_ml.ml_line_count
    {
        curwin_mut().w_cursor = curhead.uh_cursor;
        if virtual_active(curwin_mut()) && curhead.uh_cursor_vcol >= 0 {
            coladvance(curwin_mut(), curhead.uh_cursor_vcol);
        }
    } else {
        if curhead.uh_cursor.lnum + 1 == curwin_mut().w_cursor.lnum
            && curwin_mut().w_cursor.lnum > 1
        {
            curwin_mut().w_cursor.lnum -= 1;
        }
        if curwin_mut().w_cursor.lnum <= curbuf_mut().b_ml.ml_line_count {
            if curhead.uh_cursor.lnum == curwin_mut().w_cursor.lnum {
                curwin_mut().w_cursor.col = curhead.uh_cursor.col;
                if virtual_active(curwin_mut()) && curhead.uh_cursor_vcol >= 0 {
                    coladvance(curwin_mut(), curhead.uh_cursor_vcol);
                } else {
                    curwin_mut().w_cursor.coladd = 0;
                }
            } else {
                beginline(BL_SOL | BL_FIX);
            }
        } else {
            curwin_mut().w_cursor.col = 0;
            curwin_mut().w_cursor.coladd = 0;
        }
    }

    check_cursor(curwin_mut());

    if !undo && curhead.uh_cursor_after.lnum > 0 {
        curwin_mut().w_cursor = curhead.uh_cursor_after;
        check_cursor(curwin_mut());
    }

    curbuf_mut().b_u_seq_cur = curhead.uh_seq;
    if undo {
        curbuf_mut().b_u_seq_cur = if let Some(n) = curhead.uh_next {
            uhp_ref(curbuf_mut(), n).uh_seq
        } else {
            0
        };
    }

    if curhead.uh_save_nr != 0 {
        if undo {
            curbuf_mut().b_u_save_nr_cur = curhead.uh_save_nr - 1;
        } else {
            curbuf_mut().b_u_save_nr_cur = curhead.uh_save_nr;
        }
    }

    curbuf_mut().b_u_time_cur = curhead.uh_time;

    unblock_autocmds();
}

fn u_undo_end(mut did_undo: bool, absolute: bool, quiet: bool) {
    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_UNDO) != 0 && g_st().key_typed {
        fold_open_cursor();
    }

    if quiet || g_st().global_busy != 0 || !messaging() || shortmess(K_SHM_UNDO) {
        return;
    }

    if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
        undo_st(|st| st.u_newcount.set(st.u_newcount.get() - 1));
    }

    let (oldcount_abs, msgstr) = undo_st(|st| {
        let newcount = st.u_newcount.get();
        let mut oldcount = st.u_oldcount.get() - newcount;
        let msgstr = if oldcount == -1 {
            c"more line"
        } else if oldcount < 0 {
            c"more lines"
        } else if oldcount == 1 {
            c"line less"
        } else if oldcount > 1 {
            c"fewer lines"
        } else {
            oldcount = newcount;
            if newcount == 1 { c"change" } else { c"changes" }
        };
        st.u_oldcount.set(oldcount);
        (
            if oldcount < 0 {
                -(oldcount as i64)
            } else {
                oldcount as i64
            },
            msgstr,
        )
    });

    let uhp = if let Some(ch) = curbuf_mut().b_u_curhead {
        let next = uhp_ref(curbuf_mut(), ch).uh_next;
        if absolute && next.is_some() {
            did_undo = false;
            next
        } else if did_undo {
            Some(ch)
        } else {
            next
        }
    } else {
        curbuf_mut().b_u_newhead
    };

    let mut msgbuf = [0u8; 80];
    if let Some(uh) = uhp {
        undo_fmt_time(&mut msgbuf, uhp_ref(curbuf_mut(), uh).uh_time);
    }

    for wp in for_all_windows_ptr() {
        if wp.w_buffer == g_st().curbuf && wp.w_onebuf_opt.wo_cole > 0 {
            redraw_later(Some(&mut *wp), UPD_NOT_VALID);
        }
    }

    if g_st().visual_global.active {
        check_pos(curbuf_mut(), &mut g_st().visual_global.start);
    }

    smsg_keep!(
        0,
        gettext(c"%lld %s; %s #%lld  %s"),
        oldcount_abs,
        gettext(msgstr),
        if did_undo {
            gettext(c"before")
        } else {
            gettext(c"after")
        },
        uhp.map_or(0i64, |uh| uhp_ref(curbuf_mut(), uh).uh_seq as i64),
        CStr::from_bytes_until_nul(&msgbuf).unwrap_or(c""),
    );
}

/// Put the timestamp of an undo header in `buf` in a nice format.
pub fn undo_fmt_time(buf: &mut [u8], tt: time_t) {
    let now = os_time() as time_t;
    if now - tt >= 100 {
        let mut curtime = Tm { _fields: [0; 16] };
        let _ = os_localtime_r(&tt, &mut curtime);
        let fmt = if (os_time() as time_t) - tt < 60 * 60 * 12 {
            c"%H:%M:%S"
        } else {
            c"%Y/%m/%d %H:%M:%S"
        };
        let n = os_strftime(buf, fmt, &curtime);
        if n == 0 && !buf.is_empty() {
            buf[0] = 0;
        }
    } else {
        let seconds = now - tt;
        format_to_buf!(
            buf,
            ngettext(
                c"%lld second ago",
                c"%lld seconds ago",
                seconds as u32 as u64,
            ),
            seconds,
        );
    }
}

/// `u_sync`: stop adding to the current entry list.
pub fn u_sync(force: bool) {
    if curbuf_mut().b_u_synced || (!force && g_st().no_u_sync > 0) {
        return;
    }

    if get_undolevel(curbuf_mut()) < 0 {
        curbuf_mut().b_u_synced = true;
    } else {
        u_getbot(curbuf_mut());
        curbuf_mut().b_u_curhead = None;
    }
}

/// `":undolist"`: List the leafs of the undo tree.
pub fn ex_undolist(_eap: &mut Exarg) {
    let mut changes: i32 = 1;

    let (mark, nomark) = undo_st(|st| {
        let m = st.lastmark.get() + 1;
        let nm = m + 1;
        st.lastmark.set(nm);
        (m, nm)
    });
    let mut lines: Vec<std::ffi::CString> = Vec::new();

    let mut uhp = curbuf_mut().b_u_oldhead;
    while let Some(uh) = uhp {
        let uh_prev = uhp_ref(curbuf_mut(), uh).uh_prev;
        let uh_walk = uhp_ref(curbuf_mut(), uh).uh_walk;
        let uh_seq = uhp_ref(curbuf_mut(), uh).uh_seq;
        let uh_time = uhp_ref(curbuf_mut(), uh).uh_time;
        let uh_save_nr = uhp_ref(curbuf_mut(), uh).uh_save_nr;
        if uh_prev.is_none() && uh_walk != nomark && uh_walk != mark {
            format_to_buf!(g_st().iobuff_bytes_mut(), c"%6d %7d  ", uh_seq, changes);
            let cur_len = g_st().iobuff_cstr().to_bytes().len();
            undo_fmt_time(&mut g_st().iobuff_bytes_mut()[cur_len..], uh_time);
            if uh_save_nr > 0 {
                while g_st().iobuff_cstr().to_bytes().len() < 33 {
                    crate::memory::append_bytes_to_c_buf(g_st().iobuff_bytes_mut(), b" ");
                }
                format_to_buf_add!(g_st().iobuff_bytes_mut(), c"  %3d", uh_save_nr);
            }
            lines.push(g_st().iobuff_cstr().to_owned());
        }

        uhp_mut(curbuf_mut(), uh).uh_walk = mark;

        let alt_next = uhp_ref(curbuf_mut(), uh).uh_alt_next;
        let next = uhp_ref(curbuf_mut(), uh).uh_next;
        let alt_prev = uhp_ref(curbuf_mut(), uh).uh_alt_prev;
        if let Some(p) = uh_prev
            && uhp_ref(curbuf_mut(), p).uh_walk != nomark
            && uhp_ref(curbuf_mut(), p).uh_walk != mark
        {
            uhp = Some(p);
            changes += 1;
        } else if let Some(an) = alt_next
            && uhp_ref(curbuf_mut(), an).uh_walk != nomark
            && uhp_ref(curbuf_mut(), an).uh_walk != mark
        {
            uhp = Some(an);
        } else if let Some(n) = next
            && alt_prev.is_none()
            && uhp_ref(curbuf_mut(), n).uh_walk != nomark
            && uhp_ref(curbuf_mut(), n).uh_walk != mark
        {
            uhp = Some(n);
            changes -= 1;
        } else {
            uhp_mut(curbuf_mut(), uh).uh_walk = nomark;
            if let Some(ap) = alt_prev {
                uhp = Some(ap);
            } else {
                uhp = next;
                changes -= 1;
            }
        }
    }

    msg_ext_set_kind(c"list_cmd");
    if lines.is_empty() {
        msg(gettext(c"Nothing to undo"), 0);
    } else {
        lines.sort_unstable();

        msg_start();
        msg_puts_hl(
            gettext(c"number changes  when               saved"),
            HLF_T,
            false,
        );
        for line in &lines {
            if g_st().got_int {
                break;
            }
            msg_putchar(b'\n' as i32);
            if g_st().got_int {
                break;
            }
            msg_puts(line);
        }
        msg_end();
    }
}

/// `":undojoin"`: continue adding to the last entry list.
pub fn ex_undojoin(_eap: &mut Exarg) {
    if curbuf_mut().b_u_newhead.is_none() {
        return;
    }
    if curbuf_mut().b_u_curhead.is_some() {
        emsg(gettext(c"E790: undojoin is not allowed after undo"));
        return;
    }
    if !curbuf_mut().b_u_synced {
        return;
    }
    if get_undolevel(curbuf_mut()) < 0 {
        return;
    }
    curbuf_mut().b_u_synced = false;
}

/// Called after writing or reloading the file and setting `b_changed` to false.
pub fn u_unchanged(buf: &mut FileBuf) {
    let oldhead = buf.b_u_oldhead;
    u_unch_branch(buf, oldhead);
    buf.b_did_warn = false;
}

/// After reloading a buffer which was saved for `'undoreload'`: Find the first
/// line that was changed and set the cursor there.
pub fn u_find_first_changed() {
    if curbuf_mut().b_u_curhead.is_some() {
        return;
    }
    let Some(nh) = curbuf_mut().b_u_newhead else {
        return;
    };
    if uhp_ref(curbuf_mut(), nh).uh_entries.is_empty() {
        return;
    }
    let uep = &uhp_ref(curbuf_mut(), nh).uh_entries[0];
    if uep.ue_top != 0 || uep.ue_bot != 0 {
        return;
    }

    let mut lnum: Linenr = 1;
    while lnum < curbuf_mut().b_ml.ml_line_count && lnum <= uep.ue_size() {
        if crate::memline::ml_get_buf_slice(crate::globals::curbuf_mut(), lnum)
            != uep.ue_array.as_slice()[(lnum - 1) as usize].as_bytes()
        {
            clearpos(&mut uhp_mut(curbuf_mut(), nh).uh_cursor);
            uhp_mut(curbuf_mut(), nh).uh_cursor.lnum = lnum;
            return;
        }
        lnum += 1;
    }
    if curbuf_mut().b_ml.ml_line_count != uep.ue_size() {
        clearpos(&mut uhp_mut(curbuf_mut(), nh).uh_cursor);
        uhp_mut(curbuf_mut(), nh).uh_cursor.lnum = lnum;
    }
}

/// Increase the write count, store it in the last undo header, what would be used for `"u"`.
pub fn u_update_save_nr(buf: &mut FileBuf) {
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

fn u_unch_branch(buf: &mut FileBuf, uhp: Option<usize>) {
    let mut uh = uhp;
    while let Some(u) = uh {
        uhp_mut(buf, u).uh_flags |= UH_CHANGED;
        let alt_next = uhp_ref(&*buf, u).uh_alt_next;
        let prev = uhp_ref(&*buf, u).uh_prev;
        if alt_next.is_some() {
            u_unch_branch(buf, alt_next);
        }
        uh = prev;
    }
}

fn u_get_headentry(buf: &mut FileBuf) -> Option<&mut UEntry> {
    let Some(newhead) = buf.b_u_newhead else {
        iemsg(gettext(E_UNDO_LIST_CORRUPT));
        return None;
    };
    if uhp_ref(buf, newhead).uh_entries.is_empty() {
        iemsg(gettext(E_UNDO_LIST_CORRUPT));
        return None;
    }
    Some(&mut uhp_mut(buf, newhead).uh_entries[0])
}

fn u_getbot(buf: &mut FileBuf) {
    if u_get_headentry(buf).is_none() {
        return;
    }

    let newhead = buf.b_u_newhead.unwrap();

    if uhp_ref(buf, newhead).uh_getbot_entry {
        let line_count = buf.b_ml.ml_line_count;
        let uep = &mut uhp_mut(buf, newhead).uh_entries[0];
        let extra = line_count - uep.ue_lcount;
        uep.ue_bot = uep.ue_top + uep.ue_size() + 1 + extra;
        if uep.ue_bot < 1 || uep.ue_bot > line_count {
            let ue_top = uep.ue_top;
            iemsg(gettext(E_UNDO_LINE_MISSING));
            uhp_mut(buf, newhead).uh_entries[0].ue_bot = ue_top + 1;
        }

        uhp_mut(buf, newhead).uh_getbot_entry = false;
    }

    buf.b_u_synced = true;
}

fn u_freeheader(buf: &mut FileBuf, uhp: usize, uhpp: &mut Option<usize>) {
    if let Some(alt_next) = uhp_ref(buf, uhp).uh_alt_next {
        u_freebranch(buf, alt_next, uhpp);
    }

    if let Some(alt_prev) = uhp_ref(buf, uhp).uh_alt_prev {
        uhp_mut(buf, alt_prev).uh_alt_next = None;
    }

    let next = uhp_ref(buf, uhp).uh_next;
    let prev = uhp_ref(buf, uhp).uh_prev;
    if let Some(n) = next {
        uhp_mut(buf, n).uh_prev = prev;
    } else {
        buf.b_u_oldhead = prev;
    }

    if let Some(p) = prev {
        let mut uhap = Some(p);
        while let Some(ua) = uhap {
            uhp_mut(buf, ua).uh_next = next;
            uhap = uhp_ref(buf, ua).uh_alt_next;
        }
    } else {
        buf.b_u_newhead = next;
    }

    u_freeentries(buf, uhp, uhpp);
}

fn u_freebranch(buf: &mut FileBuf, uhp: usize, uhpp: &mut Option<usize>) {
    if Some(uhp) == buf.b_u_oldhead {
        while let Some(oh) = buf.b_u_oldhead {
            u_freeheader(buf, oh, uhpp);
        }
        return;
    }

    if let Some(alt_prev) = uhp_ref(buf, uhp).uh_alt_prev {
        uhp_mut(buf, alt_prev).uh_alt_next = None;
    }

    let mut next = Some(uhp);
    while let Some(tofree) = next {
        if let Some(alt_next) = uhp_ref(buf, tofree).uh_alt_next {
            u_freebranch(buf, alt_next, uhpp);
        }
        next = uhp_ref(buf, tofree).uh_prev;
        u_freeentries(buf, tofree, uhpp);
    }
}

fn u_forget_header(buf: &mut FileBuf, uhp: usize) {
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
    u_freebranch(buf, uhp, &mut dummy);
}

fn u_freeentries(buf: &mut FileBuf, uhp: usize, uhpp: &mut Option<usize>) {
    if buf.b_u_curhead == Some(uhp) {
        buf.b_u_curhead = None;
    }
    if buf.b_u_newhead == Some(uhp) {
        buf.b_u_newhead = None;
    }
    if *uhpp == Some(uhp) {
        *uhpp = None;
    }

    uhp_mut(buf, uhp).clone_from_slice_or_reset();
    buf.b_u_free_headers.push(uhp);
    buf.b_u_numhead -= 1;
}

/// Prunes the newest undo entry if it has no changes.
pub fn u_forget_unchanged(buf: &mut FileBuf) {
    let Some(uhp) = buf.b_u_newhead else {
        return;
    };
    if uhp_ref(&*buf, uhp).uh_entries.is_empty() || buf.b_u_curhead.is_some() {
        return;
    };
    let (ue_top, ue_size, resized) = {
        let uep = &uhp_ref(buf, uhp).uh_entries[0];
        (uep.ue_top, uep.ue_size(), u_entry_resized(buf, uep))
    };
    if resized {
        return;
    }
    for i in 0..ue_size {
        let line_bytes = crate::memline::ml_get_buf_slice(buf, ue_top + 1 + i);
        let saved_bytes = uhp_ref(buf, uhp).uh_entries[0].ue_array[i as usize].as_bytes();
        if saved_bytes != line_bytes {
            return;
        }
    }

    uhp_mut(&mut *buf, uhp).uh_entries.remove(0);
    uhp_mut(&mut *buf, uhp).uh_getbot_entry = false;
    if uhp_ref(&*buf, uhp).uh_entries.is_empty() {
        u_forget_header(buf, uhp);
    }
}

/// Invalidate the undo buffer; called when storage has already been released.
pub fn u_clearall(buf: &mut FileBuf) {
    buf.b_u_newhead = None;
    buf.b_u_oldhead = None;
    buf.b_u_curhead = None;
    buf.b_u_synced = true;
    buf.b_u_numhead = 0;
    buf.b_u_line_ptr = None;
    buf.b_u_line_lnum = 0;
}

/// Free all allocated memory blocks for the buffer `buf`.
pub fn u_blockfree(buf: &mut FileBuf) {
    while let Some(previous_oldhead) = buf.b_u_oldhead {
        let mut dummy = None;
        u_freeheader(buf, previous_oldhead, &mut dummy);
        assert!(buf.b_u_oldhead != Some(previous_oldhead));
    }
    buf.b_u_line_ptr = None;
}

/// Free all allocated memory blocks for the buffer `buf` and invalidate the undo buffer.
pub fn u_clearallandblockfree(buf: &mut FileBuf) {
    u_blockfree(&mut *buf);
    u_clearall(buf);
    buf.b_u_headers.clear();
    buf.b_u_free_headers.clear();
}

fn u_saveline(buf: &mut FileBuf, lnum: Linenr) {
    if lnum == buf.b_u_line_lnum {
        return;
    }
    if lnum < 1 || lnum > buf.b_ml.ml_line_count {
        return;
    }
    u_clearline(buf);
    buf.b_u_line_lnum = lnum;
    if curwin_mut().w_buffer == buf && curwin_mut().w_cursor.lnum == lnum {
        buf.b_u_line_colnr = curwin_mut().w_cursor.col;
    } else {
        buf.b_u_line_colnr = 0;
    }
    buf.b_u_line_ptr = Some(u_save_line_buf(buf, lnum));
}

/// Clear the line saved for the `"U"` command.
pub fn u_clearline(buf: &mut FileBuf) {
    if buf.b_u_line_ptr.is_none() {
        return;
    }

    buf.b_u_line_ptr = None;
    buf.b_u_line_lnum = 0;
}

/// Implementation of the `"U"` command.
pub fn u_undoline() {
    if curbuf_mut().b_u_line_ptr.is_none()
        || curbuf_mut().b_u_line_lnum > curbuf_mut().b_ml.ml_line_count
    {
        beep_flush();
        return;
    }
    atom_did_global_op();

    if !u_savecommon(
        curbuf_mut(),
        curbuf_mut().b_u_line_lnum - 1,
        curbuf_mut().b_u_line_lnum + 1,
        0,
        false,
    ) {
        return;
    }

    let oldp = u_save_line(curbuf_mut().b_u_line_lnum);
    let new_line = curbuf_mut().b_u_line_ptr.as_ref().unwrap();
    let new_line_len = new_line.as_bytes().len() as Colnr;
    ml_replace(curbuf_mut().b_u_line_lnum, new_line.to_bytes(), true);
    extmark_splice_cols(
        curbuf_mut(),
        curbuf_mut().b_u_line_lnum - 1,
        0,
        oldp.as_bytes().len() as Colnr,
        new_line_len,
        K_EXTMARK_UNDO,
    );
    changed_bytes(curbuf_mut().b_u_line_lnum, 0);
    curbuf_mut().b_u_line_ptr = Some(oldp);

    let t = curbuf_mut().b_u_line_colnr;
    if curwin_mut().w_cursor.lnum == curbuf_mut().b_u_line_lnum {
        curbuf_mut().b_u_line_colnr = curwin_mut().w_cursor.col;
    }
    curwin_mut().w_cursor.col = t;
    curwin_mut().w_cursor.lnum = curbuf_mut().b_u_line_lnum;
    check_cursor_col(curwin_mut());
}

fn u_save_line(lnum: Linenr) -> std::ffi::CString {
    u_save_line_buf(crate::globals::curbuf_mut(), lnum)
}

fn u_save_line_buf(buf: &mut FileBuf, lnum: Linenr) -> std::ffi::CString {
    crate::memory::bytes_to_cstring(crate::memline::ml_get_buf_slice(buf, lnum))
}

/// Check if the `'modified'` flag is set, or `'ff'` has changed.
pub fn buf_is_changed_fn(buf: &mut FileBuf) -> bool {
    if bt_prompt(Some(&*buf)) {
        buf.b_modified_was_set
    } else {
        !bt_dontwrite(Some(&*buf)) && (buf.b_changed != 0 || file_ff_differs(buf, true))
    }
}

/// Return true if any buffer has changes. Also buffers that are not written.
pub fn any_buf_is_changed() -> bool {
    for buf in for_all_buffers_mut() {
        if buf_is_changed_fn(buf) {
            return true;
        }
    }
    false
}

/// Return true if the current buffer has changed.
pub fn curbuf_is_changed_fn() -> bool {
    buf_is_changed_fn(curbuf_mut())
}

fn u_eval_tree(buf: &mut FileBuf, first_uhp: Option<usize>) -> &'static mut TvList {
    let list = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);

    let mut uhp = first_uhp;
    while let Some(uh) = uhp {
        let dict = tv_dict_alloc();
        tv_dict_add_nr(&mut *dict, c"seq", uhp_ref(&*buf, uh).uh_seq as Varnumber);
        tv_dict_add_nr(&mut *dict, c"time", uhp_ref(&*buf, uh).uh_time as Varnumber);
        if Some(uh) == buf.b_u_newhead {
            tv_dict_add_nr(&mut *dict, c"newhead", 1);
        }
        if Some(uh) == buf.b_u_curhead {
            tv_dict_add_nr(&mut *dict, c"curhead", 1);
        }
        if uhp_ref(&*buf, uh).uh_save_nr > 0 {
            tv_dict_add_nr(
                &mut *dict,
                c"save",
                uhp_ref(&*buf, uh).uh_save_nr as Varnumber,
            );
        }

        let alt_next = uhp_ref(&*buf, uh).uh_alt_next;
        let prev = uhp_ref(&*buf, uh).uh_prev;
        if alt_next.is_some() {
            tv_dict_add_list(&mut *dict, c"alt", Some(u_eval_tree(buf, alt_next)));
        }

        tv_list_append_dict(&mut *list, Some(dict));
        uhp = prev;
    }

    list
}

/// `"undofile(name)"` function.
pub fn f_undofile(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(rettv, None);
    let fname = tv_get_string(&argvars[0]);

    if !fname.to_bytes().is_empty()
        && let Some(ffname) = FullName_save(Some(&fname), true)
        && let Some(ufile) = u_get_undo_file_name(Some(ffname.as_c_str()), false)
    {
        crate::eval::typval::tv_set_string(rettv, Some(ufile.as_c_str()));
    }
}

/// `"undotree(expr)"` function.
pub fn f_undotree(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let dict = tv_dict_alloc_ret(rettv);

    let tv = &mut argvars[0];
    let buf = if tv.v_type == VarType::VarUnknown {
        g_st().curbuf.opt_mut()
    } else {
        get_buf_arg(tv)
    };
    let Some(buf) = buf else {
        return;
    };

    tv_dict_add_nr(dict, c"synced", Varnumber::from(buf.b_u_synced));
    tv_dict_add_nr(dict, c"seq_last", buf.b_u_seq_last as Varnumber);
    tv_dict_add_nr(dict, c"save_last", buf.b_u_save_nr_last as Varnumber);
    tv_dict_add_nr(dict, c"seq_cur", buf.b_u_seq_cur as Varnumber);
    tv_dict_add_nr(dict, c"time_cur", buf.b_u_time_cur as Varnumber);
    tv_dict_add_nr(dict, c"save_cur", buf.b_u_save_nr_cur as Varnumber);

    tv_dict_add_list(dict, c"entries", Some(u_eval_tree(buf, buf.b_u_oldhead)));
}

/// Drops named mark `idx` from the pending undo snapshot.
pub fn u_update_named_mark(buf: &mut FileBuf, idx: i32) {
    let uhp = buf.b_u_curhead.or(buf.b_u_newhead);
    if let Some(uh) = uhp {
        uhp_mut(&mut *buf, uh).uh_namedm[idx as usize].mark.lnum = 0;
    }
}

/// Given the buffer, return the undo header index. If none is set, set one first.
pub fn u_force_get_undo_header(buf: &mut FileBuf) -> Option<usize> {
    let mut uhp = buf.b_u_curhead.or(buf.b_u_newhead);

    if uhp.is_none() {
        u_savecommon(&mut *buf, 0, 1, 1, true);
        uhp = buf.b_u_curhead;
        if uhp.is_none() {
            uhp = buf.b_u_newhead;
            if get_undolevel(&mut *buf) > 0 && uhp.is_none() {
                std::process::abort();
            }
        }
    }
    uhp
}

/// Checkpoints the undo state of `buf` and detaches its undotree.
pub fn u_checkpoint(uc: &mut UndoCheckpoint, buf: &mut FileBuf) {
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
    buf.b_p_ul = OptInt::from(i32::MAX);
}

/// Reverts `buf` from a checkpoint.
pub fn u_rollback(uc: &mut UndoCheckpoint, buf: &mut FileBuf) {
    if buf.b_u_seq_cur != uc.uc_seq_cur {
        let mut count: i32 = 0;
        let mut uhp = buf.b_u_curhead.or(buf.b_u_newhead);
        while let Some(uh) = uhp {
            uhp = uhp_ref(&*buf, uh).uh_next;
            count += 1;
        }

        let mut cs: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut cs, None, None, Some(&mut *buf), 0);
        if !curbuf_mut().b_u_synced {
            u_sync(true);
        }
        if !u_undo_and_forget(count, false) {
            std::process::abort();
        }
        ctx_restore(&mut cs);
    }

    u_blockfree(buf);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalpos_and_clearpos_helpers_work() {
        let mut p = CursorPos {
            lnum: 5,
            col: 3,
            coladd: 1,
        };
        assert!(equalpos(
            p,
            CursorPos {
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
        use std::ffi::CString;

        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::main_init::ensure_test_editor_init();
            g_st().emsg_skip += 1;

            let mut cwd_buf = [0u8; MAXPATHL];
            assert!(os_dirname(&mut cwd_buf));
            let cwd = CStr::from_bytes_until_nul(&cwd_buf)
                .unwrap()
                .to_str()
                .unwrap();
            let base = format!("{cwd}/target/ut_undo_{}", std::process::id());
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();

            let base_c = CString::new(base.clone()).unwrap();
            let save_udir =
                std::mem::replace(&mut opt_vars().p_udir, OptString::from_cstr(&base_c));

            let fpath = format!("{base}/Xtest-unit-undo");
            std::fs::write(&fpath, b"hello\n").unwrap();
            let fpath_c = CString::new(fpath.clone()).unwrap();
            let buf = buflist_new(
                Some(fpath_c.as_c_str()),
                Some(fpath_c.as_c_str()),
                1,
                BLN_LISTED,
            )
            .unwrap();
            buf.b_u_numhead = 1;

            let mut hash = [0u8; UNDO_HASH_SIZE];
            u_compute_hash(&mut *buf, &mut hash);

            // Write undo file to undodir
            u_write_undo(None, false, &mut *buf, &hash);
            let ufile_cstr = u_get_undo_file_name(buf.ffname_opt(), false).unwrap();
            let ufile = ufile_cstr.to_str().unwrap().to_owned();
            assert!(std::path::Path::new(&ufile).exists());
            let _ = std::fs::remove_file(&ufile);

            // Write undo file with explicit name
            let explicit = format!("{base}/undofile.test");
            let explicit_c = CString::new(explicit.clone()).unwrap();
            u_write_undo(Some(explicit_c.as_c_str()), false, &mut *buf, &hash);
            assert!(std::path::Path::new(&explicit).exists());
            let _ = std::fs::remove_file(&explicit);

            // Force write (:wundo!) overwrites non-undo file
            std::fs::write(&ufile, b"not an undo file").unwrap();
            let ufile_c = CString::new(ufile.clone()).unwrap();
            u_write_undo(Some(ufile_c.as_c_str()), true, &mut *buf, &hash);
            assert_ne!(std::fs::read(&ufile).unwrap(), b"not an undo file");
            let _ = std::fs::remove_file(&ufile);

            // Does not write undo file when b_u_numhead == 0
            buf.b_u_numhead = 0;
            u_write_undo(None, false, &mut *buf, &hash);
            assert!(!std::path::Path::new(&ufile).exists());

            close_buffer(None, Some(buf), DOBUF_WIPE, false, false, false);
            opt_vars().p_udir = save_udir;
            let _ = std::fs::remove_dir_all(&base);
            g_st().emsg_skip -= 1;
        }
    }
}
