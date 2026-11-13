//! Functions for setting marks and jumping to them (`src/nvim/mark.c`).
#![forbid(unsafe_code)]

use crate::api::private::validate::{
    Dict, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::autocmd::{AUGROUP_ALL, EVENT_MARKSET, aucmd_defer, has_event};
use crate::buffer::{
    FileBuf, GETF_SETMARK, K_MARK_ADJUST_NORMAL, MAXLNUM, NMARKS, bt_prompt, buflist_findnr,
    buflist_findnr_opt, buflist_getfile, buflist_new, buflist_nr2name, first_win_in_tab,
};
use crate::charset::{ascii_isdigit, ascii_islower, ascii_isupper, vim_isprintc};
use crate::context::{INIT_FMARK, INIT_FMARKV};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor, equalpos, lt};
use crate::diff::diff_mark_adjust;
use crate::errors::*;
use crate::eval::typval::{
    TvList, tv_dict_add_list, tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_list_alloc,
    tv_list_append_dict, tv_list_append_number,
};
use crate::ex_docmd::{CMOD_KEEPJUMPS, CMOD_LOCKMARKS, Exarg};
use crate::extmark::{ExtmarkOp, K_EXTMARK_NOOP, extmark_adjust};
use crate::fold::{FoldEntry, fold_mark_adjust, has_folding};
use crate::globals::*;
use crate::insert::beginline;
use crate::lua::executor::nlua_call_excmd;
use crate::mbyte::slice2cells;
use crate::mcursor::mc_replaying;
use crate::memline::ml_get;
use crate::message::{
    emsg, message_filtered, msg_ext_set_kind, msg_outtrans, msg_putchar, msg_puts, msg_puts_title,
    semsg,
};
use crate::r#move::set_topline;
use crate::option_vars::*;
use crate::os::env::expand_env;
use crate::os::fs::os_dirname;
use crate::os::input::os_breakcheck;
use crate::os::time::{Timestamp, os_time};
use crate::path::{K_PATH_CMP_LITERAL, path_equal, path_shorten_fname, vim_ispathsep_nocolon};
use crate::plines::linetabsize_disp_eol;
use crate::quickfix::qf_mark_adjust;
use crate::strings::gettext;
use crate::textformat::Oparg;
use crate::textobject::{findpar, findsent};
use crate::undo::u_update_named_mark;
use crate::window::{WinInfoInfo, Winopt};
use std::ffi::CStr;

pub type MarkMoveRes = i32;
pub type MarkMove = i32;
pub type MarkGet = i32;
pub type MarkAdjustMode = i32;

pub(crate) const K_MARK_ADJUST_API: MarkAdjustMode = 1;
pub(crate) const K_MARK_ALL_NO_RESOLVE: MarkGet = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Fmarkv {
    pub topline_offset: i32,
    pub skipcol: i32,
}

#[derive(Clone)]
pub struct Fmark {
    pub mark: CursorPos,
    pub fnum: i32,
    pub timestamp: u64,
    pub view: Fmarkv,
    pub additional_data: Option<crate::search::AdditionalData>,
}

impl Fmark {
    pub const ZERO: Self = Self {
        mark: CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        },
        fnum: 0,
        timestamp: 0,
        view: Fmarkv {
            topline_offset: 0,
            skipcol: 0,
        },
        additional_data: None,
    };

    pub const fn new() -> Self {
        Self::ZERO
    }
}

impl Default for Fmark {
    fn default() -> Self {
        Self::ZERO
    }
}

#[derive(Clone)]
pub struct Xfmark {
    pub fmark: Fmark,
    pub fname: Option<std::ffi::CString>,
}

impl Xfmark {
    pub const ZERO: Self = Self {
        fmark: Fmark::ZERO,
        fname: None,
    };

    pub const fn new() -> Self {
        Self::ZERO
    }

    #[inline]
    pub fn fname_cstr(&self) -> Option<&CStr> {
        self.fname.as_deref()
    }
}

impl Default for Xfmark {
    fn default() -> Self {
        Self::ZERO
    }
}
use crate::change::TAB;
use crate::cmdexpand::HLF_D;
use crate::eval::K_LIST_LEN_MAY_KNOW_CONST;
use crate::ex_cmds::{BL_FIX, BL_WHITE};
use crate::regexp::{K_MARK_BUF_LOCAL, MAXCOL};
use crate::search::{BACKWARD, FORWARD};

#[repr(C)]
pub struct WinInfo {
    pub wi_win: crate::globals::WinPtr,
    pub wi_mark: Fmark,
    pub wi_optset: bool,
    pub wi_opt: Winopt,
    pub wi_fold_manual: bool,
    pub wi_folds: Vec<FoldEntry>,
    pub wi_changelistidx: i32,
}

impl WinInfo {
    pub const fn new() -> Self {
        Self {
            wi_win: crate::globals::WinPtr::NULL,
            wi_mark: Fmark::ZERO,
            wi_optset: false,
            wi_opt: Winopt::ZERO,
            wi_fold_manual: false,
            wi_folds: Vec::new(),
            wi_changelistidx: 0,
        }
    }

    #[inline]
    pub fn win_opt(&self) -> Option<crate::globals::WinPtr> {
        (!self.wi_win.is_null()).then(|| self.wi_win.clone())
    }
}

impl Drop for WinInfo {
    fn drop(&mut self) {
        if self.wi_optset {
            crate::option::clear_winopt(&self.wi_opt);
        }
    }
}

pub(crate) const EXTRA_MARKS: usize = 10;
const NGLOBALMARKS: usize = NMARKS + EXTRA_MARKS;
const NMARK_LOCAL_MAX: i32 = 126;
pub(crate) const JUMPLISTSIZE: i32 = 100;

pub(crate) const K_MARK_MOVE_SUCCESS: MarkMoveRes = 1;
pub(crate) const K_MARK_MOVE_FAILED: MarkMoveRes = 2;
pub(crate) const K_MARK_SWITCHED_BUF: MarkMoveRes = 4;
const K_MARK_CHANGED_COL: MarkMoveRes = 8;
pub(crate) const K_MARK_CHANGED_LINE: MarkMoveRes = 16;
pub(crate) const K_MARK_CHANGED_CURSOR: MarkMoveRes = 32;

pub(crate) const K_MARK_BEGIN_LINE: MarkMove = 1;
pub(crate) const K_MARK_CONTEXT: MarkMove = 2;
pub(crate) const K_MARK_SET_VIEW: MarkMove = 8;
pub(crate) const K_MARK_JUMP_LIST: MarkMove = 16;

pub(crate) const K_MARK_ADJUST_TERM: MarkAdjustMode = 2;

const K_OPT_JOP_FLAG_STACK: u32 = 0x01;

pub(crate) const BUF_HAS_QF_ENTRY: i32 = 1;
pub(crate) const BUF_HAS_LL_ENTRY: i32 = 2;

const INIT_XFMARK: Xfmark = Xfmark {
    fmark: INIT_FMARK,
    fname: None,
};

#[inline]
pub const fn mark_global_index(name: u8) -> i32 {
    if name.is_ascii_uppercase() {
        (name - b'A') as i32
    } else if name.is_ascii_digit() {
        (NMARKS as i32) + ((name - b'0') as i32)
    } else {
        -1
    }
}

#[inline]
fn set_fmark(fmarkp: &mut Fmark, mark: CursorPos, fnum: i32, view: Fmarkv) {
    fmarkp.mark = mark;
    fmarkp.fnum = fnum;
    fmarkp.timestamp = os_time();
    fmarkp.view = view;
    fmarkp.additional_data = None;
}

#[inline]
pub fn reset_fmark(fmarkp: &mut Fmark, mark: CursorPos, fnum: i32, view: Fmarkv) {
    set_fmark(fmarkp, mark, fnum, view);
}

#[inline]
fn set_xfmark(
    xfmarkp: &mut Xfmark,
    mark: CursorPos,
    fnum: i32,
    view: Fmarkv,
    fname: Option<std::ffi::CString>,
) {
    xfmarkp.fname = fname;
    set_fmark(&mut xfmarkp.fmark, mark, fnum, view);
}

#[inline]
fn reset_xfmark(
    xfmarkp: &mut Xfmark,
    mark: CursorPos,
    fnum: i32,
    view: Fmarkv,
    fname: Option<std::ffi::CString>,
) {
    free_xfmark(&mut *xfmarkp);
    set_xfmark(&mut *xfmarkp, mark, fnum, view, fname);
}

/// Set named mark `c` at current cursor position (`setmark`).
pub fn setmark(c: i32) -> bool {
    let mut pos = curwin().w_cursor.get();
    let view = mark_view_make(&curwin(), pos);
    setmark_pos(c, &mut pos, curbuf().b_fnum.get(), Some(&view))
}

/// Free `Xfmark` item (`free_xfmark`).
pub fn free_xfmark(fm: &mut Xfmark) {
    fm.fname = None;
    fm.fmark.additional_data = None;
}

/// Free and clear `Fmark` item (`clear_fmark`).
pub fn clear_fmark(fm: &mut Fmark, timestamp: Timestamp) {
    *fm = INIT_FMARK;
    fm.timestamp = timestamp;
}

/// Schedules `"MarkSet"` event (`do_markset_autocmd`).
fn do_markset_autocmd(c: u8, pos: &CursorPos, buf: &FileBuf) {
    if !has_event(EVENT_MARKSET) {
        return;
    }

    let mark_str: [u8; 2] = [c, 0];
    let mut items: [KeyValuePair; 3] = [
        KeyValuePair {
            key: NvimString::from_cstr(c"name"),
            value: Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(NvimString::from_bytes_len(&mark_str, 1)),
            },
        },
        KeyValuePair {
            key: NvimString::from_cstr(c"line"),
            value: Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer(i64::from(pos.lnum)),
            },
        },
        KeyValuePair {
            key: NvimString::from_cstr(c"col"),
            value: Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer(i64::from(pos.col)),
            },
        },
    ];
    let data = Dict::from_slice(&mut items[..3]);
    let mut dict_obj = Object {
        obj_type: ObjectType::ObjectTypeDict,
        data: ObjectData::Dict(data),
    };
    aucmd_defer(
        EVENT_MARKSET,
        Some(crate::memory::bytes_as_cstr(&mark_str)),
        None,
        AUGROUP_ALL,
        buf,
        Some(&mut dict_obj),
    );
}

/// Set named mark `c` to position `pos` (`setmark_pos`).
pub fn setmark_pos(c: i32, pos: &mut CursorPos, fnum: i32, view_pt: Option<&Fmarkv>) -> bool {
    let view = if let Some(view_pt) = view_pt {
        *view_pt
    } else {
        INIT_FMARKV
    };

    if c < 0 {
        return false;
    }

    if c == b'\'' as i32 || c == b'`' as i32 {
        if view_pt.is_some() {
            setpcmark();
            curwin().w_prev_pcmark.set(curwin().w_pcmark.get());
        } else {
            curwin().w_pcmark.set(*pos);
        }
        return true;
    }

    let Some(buf) = buflist_findnr_opt(fnum) else {
        return false;
    };

    if c == b'"' as i32 {
        reset_fmark(
            &mut buf.b_last_cursor.borrow_mut(),
            *pos,
            buf.b_fnum.get(),
            view,
        );
        do_markset_autocmd(c as u8, pos, &buf);
        return true;
    }

    if c == b'[' as i32 {
        buf.b_op_start.set(*pos);
        do_markset_autocmd(c as u8, pos, &buf);
        return true;
    }
    if c == b']' as i32 {
        buf.b_op_end.set(*pos);
        do_markset_autocmd(c as u8, pos, &buf);
        return true;
    }

    if c == b'<' as i32 || c == b'>' as i32 {
        if c == b'<' as i32 {
            buf.b_visual.borrow_mut().vi_start = *pos;
        } else {
            buf.b_visual.borrow_mut().vi_end = *pos;
        }
        if buf.b_visual.borrow().vi_mode == 0 {
            buf.b_visual.borrow_mut().vi_mode = b'v' as i32;
        }
        do_markset_autocmd(c as u8, pos, &buf);
        return true;
    }

    if c == b':' as i32 && bt_prompt(Some(&buf)) {
        reset_fmark(
            &mut buf.b_prompt_start.borrow_mut(),
            *pos,
            buf.b_fnum.get(),
            view,
        );
        return true;
    }

    if ascii_islower(c) {
        let i = (c - b'a' as i32) as usize;
        reset_fmark(&mut buf.b_namedm.borrow_mut()[i], *pos, fnum, view);
        u_update_named_mark(&buf, i as i32);
        do_markset_autocmd(c as u8, pos, &buf);
        return true;
    }
    if ascii_isupper(c) || ascii_isdigit(c) {
        let i = if ascii_isdigit(c) {
            (c - b'0' as i32) as usize + NMARKS
        } else {
            (c - b'A' as i32) as usize
        };
        reset_xfmark(&mut g_st().namedfm.borrow_mut()[i], *pos, fnum, view, None);
        do_markset_autocmd(c as u8, pos, &buf);
        return true;
    }
    false
}

/// Remove every jump list entry referring to a given buffer (`mark_jumplist_forget_file`).
pub fn mark_jumplist_forget_file(wp: &WinInfoInfo, fnum: i32) {
    let mut i = wp.w_jumplistlen.get() - 1;
    while i >= 0 {
        let idx = i as usize;
        if wp.w_jumplist.borrow()[idx].fmark.fnum == fnum {
            free_xfmark(&mut wp.w_jumplist.borrow_mut()[idx]);
            if wp.w_jumplistidx.get() > i {
                wp.w_jumplistidx.set(wp.w_jumplistidx.get() - (1));
            }
            let old_len = wp.w_jumplistlen.get() as usize;
            (&mut wp.w_jumplist.borrow_mut())[idx..old_len].rotate_left(1);
            wp.w_jumplist.borrow_mut()[old_len - 1] = Xfmark::ZERO;
            wp.w_jumplistlen.set(wp.w_jumplistlen.get() - (1));
        }
        i -= 1;
    }
}

/// Delete every entry referring to file `fnum` from both the jumplist and the tag stack (`mark_forget_file`).
pub fn mark_forget_file(wp: &WinInfoInfo, fnum: i32) {
    mark_jumplist_forget_file(wp, fnum);

    let mut i = wp.w_tagstacklen.get() - 1;
    while i >= 0 {
        let idx = i as usize;
        if wp.w_tagstack.borrow()[idx].fmark.fnum == fnum {
            let old_len = wp.w_tagstacklen.get() as usize;
            (&mut wp.w_tagstack.borrow_mut())[idx..old_len].rotate_left(1);
            wp.w_tagstack.borrow_mut()[old_len - 1] = crate::tag::Taggy::ZERO;
            if wp.w_tagstackidx.get() > i {
                wp.w_tagstackidx.set(wp.w_tagstackidx.get() - (1));
            }
            wp.w_tagstacklen.set(wp.w_tagstacklen.get() - (1));
        }
        i -= 1;
    }
}

/// Set the previous context mark to the current position and add it to the jump list (`setpcmark`).
pub fn setpcmark() {
    if g_st().global_busy.get() != 0
        || g_st().listcmd_busy.get()
        || (g_st().cmdmod.borrow().cmod_flags & CMOD_KEEPJUMPS) != 0
        || mc_replaying()
    {
        return;
    }

    curwin().w_prev_pcmark.set(curwin().w_pcmark.get());
    curwin().w_pcmark.set(curwin().w_cursor.get());

    if curwin().w_pcmark.get().lnum == 0 {
        curwin().w_pcmark.set_lnum(1);
    }

    if (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_STACK) != 0
        && curwin().w_jumplistidx.get() < curwin().w_jumplistlen.get() - 1
    {
        curwin().w_jumplistlen.set(curwin().w_jumplistidx.get() + 1);
    }

    curwin()
        .w_jumplistlen
        .set(curwin().w_jumplistlen.get() + (1));
    if curwin().w_jumplistlen.get() > JUMPLISTSIZE {
        curwin().w_jumplistlen.set(JUMPLISTSIZE);
        free_xfmark(&mut curwin().w_jumplist.borrow_mut()[0]);
        (&mut curwin().w_jumplist.borrow_mut())[..JUMPLISTSIZE as usize].rotate_left(1);
    }
    curwin().w_jumplistidx.set(curwin().w_jumplistlen.get());
    let idx = (curwin().w_jumplistlen.get() - 1) as usize;
    let pcmark = curwin().w_pcmark.get();
    let fnum = curbuf().b_fnum.get();
    let view = mark_view_make(&curwin(), pcmark);
    set_xfmark(
        &mut curwin().w_jumplist.borrow_mut()[idx],
        pcmark,
        fnum,
        view,
        None,
    );
}

/// Check previous context mark (`checkpcmark`).
pub fn checkpcmark() {
    if curwin().w_prev_pcmark.get().lnum != 0
        && (equalpos(curwin().w_pcmark.get(), curwin().w_cursor.get())
            || curwin().w_pcmark.get().lnum == 0)
    {
        curwin().w_pcmark.set(curwin().w_prev_pcmark.get());
    }
    curwin().w_prev_pcmark.set_lnum(0);
}

/// Get mark in `count` position in the jumplist relative to the current index (`get_jumplist`).
pub fn get_jumplist(win: crate::globals::WinPtr, mut count: i32) -> Option<Fmark> {
    cleanup_jumplist(win.clone(), true);

    if win.w_jumplistlen.get() == 0 {
        return None;
    }

    loop {
        let jl_idx = win.w_jumplistidx.get();
        let jl_len = win.w_jumplistlen.get();
        if jl_idx + count < 0 || jl_idx + count >= jl_len {
            return None;
        }

        if jl_idx == jl_len {
            setpcmark();

            win.w_jumplistidx.set(win.w_jumplistidx.get() - (1));

            if win.w_jumplistidx.get() + count < 0 {
                return None;
            }
        }

        win.w_jumplistidx.set(win.w_jumplistidx.get() + (count));

        let idx = win.w_jumplistidx.get() as usize;
        if win.w_jumplist.borrow()[idx].fmark.fnum == 0 {
            let xfm = win.w_jumplist.borrow()[idx].clone();
            fname2fnum(&xfm);
        }
        let fnum = win.w_jumplist.borrow()[idx].fmark.fnum;
        if fnum != curbuf().b_fnum.get() && buflist_findnr(fnum).is_none() {
            count += if count < 0 { -1 } else { 1 };
            continue;
        }
        return Some(win.w_jumplist.borrow()[idx].fmark.clone());
    }
}

/// Get mark in `count` position in the changelist relative to the current index (`get_changelist`).
pub fn get_changelist(buf: &FileBuf, win: &WinInfoInfo, count: i32) -> Option<Fmark> {
    if buf.b_changelistlen.get() == 0 {
        return None;
    }

    let mut n = win.w_changelistidx.get();
    if n + count < 0 {
        if n == 0 {
            return None;
        }
        n = 0;
    } else if n + count >= buf.b_changelistlen.get() {
        if n == buf.b_changelistlen.get() - 1 {
            return None;
        }
        n = buf.b_changelistlen.get() - 1;
    } else {
        n += count;
    }
    win.w_changelistidx.set(n);
    let cur_fnum = curbuf().b_fnum.get();
    let fm = &mut buf.b_changelist.borrow_mut()[n as usize];
    fm.fnum = cur_fnum;
    Some(fm.clone())
}

/// Get a named mark (`mark_get`).
pub fn mark_get(
    buf: &FileBuf,
    win: &WinInfoInfo,
    fmp: Option<&mut Fmark>,
    flag: MarkGet,
    name: i32,
) -> Option<Fmark> {
    let mut fm: Option<Fmark> = None;
    if ascii_isupper(name) || ascii_isdigit(name) {
        let xfm = mark_get_global(flag != K_MARK_ALL_NO_RESOLVE, name);

        if flag == K_MARK_BUF_LOCAL && xfm.fmark.fnum != buf.b_fnum.get() {
            return Some(pos_to_mark(
                buf,
                None,
                CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                },
            ));
        }

        fm = Some(xfm.fmark.clone());
    } else if name > 0 && name < NMARK_LOCAL_MAX {
        fm = mark_get_local(buf, win, name);
    }
    if let Some(fmp) = fmp
        && let Some(ref fm_ref) = fm
    {
        *fmp = fm_ref.clone();
    }
    fm
}

/// Get a global mark `{A-Z0-9}` (`mark_get_global`).
pub fn mark_get_global(resolve: bool, mut name: i32) -> Xfmark {
    if ascii_isdigit(name) {
        name = name - b'0' as i32 + NMARKS as i32;
    } else if ascii_isupper(name) {
        name -= b'A' as i32;
    } else {
        debug_assert!(false);
    }
    let idx = name as usize;
    if resolve && g_st().namedfm.borrow()[idx].fmark.fnum == 0 {
        let xfm = g_st().namedfm.borrow()[idx].clone();
        fname2fnum(&xfm);
    }
    g_st().namedfm.borrow()[idx].clone()
}

/// Get a local mark (`mark_get_local`).
pub fn mark_get_local(buf: &FileBuf, win: &WinInfoInfo, name: i32) -> Option<Fmark> {
    let fnum = buf.b_fnum.get();
    let mut mark = if ascii_islower(name) {
        let idx = (name - b'a' as i32) as usize;
        buf.b_namedm.borrow_mut()[idx].fnum = fnum;
        Some(buf.b_namedm.borrow()[idx].clone())
    } else if name == b'[' as i32 {
        Some(pos_to_mark(buf, None, buf.b_op_start.get()))
    } else if name == b']' as i32 {
        Some(pos_to_mark(buf, None, buf.b_op_end.get()))
    } else if name == b'<' as i32 || name == b'>' as i32 {
        mark_get_visual(buf, name)
    } else if name == b'\'' as i32 || name == b'`' as i32 {
        Some(pos_to_mark(buf, None, win.w_pcmark.get()))
    } else if name == b'"' as i32 {
        buf.b_last_cursor.borrow_mut().fnum = fnum;
        Some(buf.b_last_cursor.borrow().clone())
    } else if name == b'^' as i32 {
        buf.b_last_insert.borrow_mut().fnum = fnum;
        Some(buf.b_last_insert.borrow().clone())
    } else if name == b'.' as i32 {
        buf.b_last_change.borrow_mut().fnum = fnum;
        Some(buf.b_last_change.borrow().clone())
    } else if name == b':' as i32 && bt_prompt(Some(buf)) {
        buf.b_prompt_start.borrow_mut().fnum = fnum;
        Some(buf.b_prompt_start.borrow().clone())
    } else {
        mark_get_motion(buf, win, name)
    };

    if let Some(ref mut mark_ref) = mark {
        mark_ref.fnum = fnum;
    }
    mark
}

/// Get marks that are actually motions (`mark_get_motion`).
pub fn mark_get_motion(buf: &FileBuf, win: &WinInfoInfo, name: i32) -> Option<Fmark> {
    let mut mark: Option<Fmark> = None;
    let pos = curwin().w_cursor.get();
    let slcb = g_st().listcmd_busy.get();
    g_st().listcmd_busy.set(true);
    if name == b'{' as i32 || name == b'}' as i32 {
        let mut oa = Oparg::new();
        if findpar(
            &mut oa.inclusive,
            if name == b'}' as i32 {
                FORWARD
            } else {
                BACKWARD
            },
            1,
            0,
            false,
        ) {
            mark = Some(pos_to_mark(buf, None, win.w_cursor.get()));
        }
    } else if (name == b'(' as i32 || name == b')' as i32)
        && findsent(
            if name == b')' as i32 {
                FORWARD
            } else {
                BACKWARD
            },
            1,
        )
    {
        mark = Some(pos_to_mark(buf, None, win.w_cursor.get()));
    }
    curwin().w_cursor.set(pos);
    g_st().listcmd_busy.set(slcb);
    mark
}

/// Get visual marks `'<'`, `'>'` (`mark_get_visual`).
pub fn mark_get_visual(buf: &FileBuf, name: i32) -> Option<Fmark> {
    if name == b'<' as i32 || name == b'>' as i32 {
        let startp = buf.b_visual.borrow().vi_start;
        let endp = buf.b_visual.borrow().vi_end;
        let mut mark =
            if ((name == b'<' as i32) == lt(startp, endp) || endp.lnum == 0) && startp.lnum != 0 {
                pos_to_mark(buf, None, startp)
            } else {
                pos_to_mark(buf, None, endp)
            };

        if buf.b_visual.borrow().vi_mode == b'V' as i32 {
            mark.mark.col = if name == b'<' as i32 { 0 } else { MAXCOL };
            mark.mark.coladd = 0;
        }
        return Some(mark);
    }
    None
}

/// Wrap a `CursorPos` into an `Fmark` (`pos_to_mark`).
pub fn pos_to_mark(buf: &FileBuf, fmp: Option<&mut Fmark>, pos: CursorPos) -> Fmark {
    let fm = Fmark {
        mark: pos,
        fnum: buf.b_fnum.get(),
        ..INIT_FMARK
    };
    if let Some(fmp) = fmp {
        *fmp = fm.clone();
    }
    fm
}

fn switch_to_mark_buf(fm: &mut Fmark, pcmark_on_switch: bool) -> MarkMoveRes {
    if fm.fnum != curbuf().b_fnum.get() {
        let getfile_flag = if pcmark_on_switch { GETF_SETMARK } else { 0 };
        let res = buflist_getfile(fm.fnum, fm.mark.lnum, getfile_flag, 0);
        return if res {
            K_MARK_SWITCHED_BUF
        } else {
            K_MARK_MOVE_FAILED
        };
    }
    0
}

/// Move to the given file mark, changing the buffer and cursor position (`mark_move_to`).
pub fn mark_move_to(fm: Option<&Fmark>, flags: MarkMove) -> MarkMoveRes {
    let mut res: MarkMoveRes = K_MARK_MOVE_SUCCESS;
    let mut errormsg: Option<std::borrow::Cow<'static, CStr>> = None;
    if !mark_check(fm, &mut errormsg) {
        if let Some(err) = errormsg {
            emsg(&err);
        }
        return K_MARK_MOVE_FAILED;
    }

    let mut fm_copy = fm.unwrap().clone();
    let fm_ref = &mut fm_copy;
    if fm_ref.fnum != curbuf().b_fnum.get() {
        res |= switch_to_mark_buf(fm_ref, (flags & K_MARK_JUMP_LIST) == 0);

        if (res & K_MARK_MOVE_FAILED) != 0 {
            return res;
        }
        if !mark_check_line_bounds(Some(&crate::globals::curbuf()), fm_ref, &mut errormsg) {
            if let Some(err) = errormsg {
                emsg(&err);
            }
            res |= K_MARK_MOVE_FAILED;
            return res;
        }
    } else if (flags & K_MARK_CONTEXT) != 0 {
        setpcmark();
    }

    let prev_pos = curwin().w_cursor.get();
    let pos = fm_ref.mark;

    curwin().w_cursor.set(fm_ref.mark);

    if (flags & K_MARK_BEGIN_LINE) != 0 {
        beginline(BL_WHITE | BL_FIX);
    }
    if prev_pos.lnum != pos.lnum {
        res |= K_MARK_CHANGED_LINE | K_MARK_CHANGED_CURSOR;
    }
    if prev_pos.col != pos.col {
        res |= K_MARK_CHANGED_COL | K_MARK_CHANGED_CURSOR;
    }
    if (flags & K_MARK_SET_VIEW) != 0 {
        mark_view_restore(Some(&*fm_ref));
    }

    if (res & K_MARK_SWITCHED_BUF) != 0 || (res & K_MARK_CHANGED_CURSOR) != 0 {
        check_cursor(&curwin());
    }
    res
}

/// Restore the mark view (`mark_view_restore`).
pub fn mark_view_restore(fm: Option<&Fmark>) {
    if let Some(fm) = fm
        && fm.view.topline_offset >= 0
    {
        let topline = fm.mark.lnum - fm.view.topline_offset;
        if topline >= 1 {
            set_topline(&curwin(), topline);
            let sc = if fm.view.skipcol > 0
                && !has_folding(&curwin(), topline, None, None)
                && fm.view.skipcol < linetabsize_disp_eol(&curwin(), topline)
            {
                fm.view.skipcol
            } else {
                0
            };
            curwin().w_skipcol.set(sc);
        }
    }
}

pub fn mark_view_make(wp: &WinInfoInfo, pos: CursorPos) -> Fmarkv {
    Fmarkv {
        topline_offset: pos.lnum - wp.w_topline.get(),
        skipcol: wp.w_skipcol.get(),
    }
}

/// Search for the next named mark in the current file from a start position (`getnextmark`).
pub fn getnextmark(startpos: &mut CursorPos, dir: i32, begin_line: i32) -> Option<Fmark> {
    let mut result_idx: Option<usize> = None;
    let mut pos = *startpos;

    if dir == BACKWARD && begin_line != 0 {
        pos.col = 0;
    } else if dir == FORWARD && begin_line != 0 {
        pos.col = MAXCOL;
    }

    for i in 0..NMARKS {
        let cur_mark = curbuf().b_namedm.borrow()[i].mark;
        if cur_mark.lnum > 0 {
            if dir == FORWARD {
                if result_idx.is_none_or(|r| lt(cur_mark, curbuf().b_namedm.borrow()[r].mark))
                    && lt(pos, cur_mark)
                {
                    result_idx = Some(i);
                }
            } else if result_idx.is_none_or(|r| lt(curbuf().b_namedm.borrow()[r].mark, cur_mark))
                && lt(cur_mark, pos)
            {
                result_idx = Some(i);
            }
        }
    }

    result_idx.map(|i| curbuf().b_namedm.borrow()[i].clone())
}

/// For an extended filemark: set the `fnum` from the `fname` (`fname2fnum`).
fn fname2fnum(fm: &Xfmark) {
    let Some(ref fname_cstr) = fm.fname else {
        return;
    };
    let fname_bytes = fname_cstr.to_bytes();
    let mut name_buff = [0u8; crate::os::fs::MAXPATHL];
    if fname_bytes.first() == Some(&b'~')
        && fname_bytes
            .get(1)
            .is_some_and(|&b| vim_ispathsep_nocolon(i32::from(b)))
    {
        let len = expand_env(c"~/", &mut name_buff);
        crate::memory::copy_bytes_to_c_buf(&mut name_buff[len..], &fname_bytes[2..]);
    } else {
        crate::memory::copy_bytes_to_c_buf(&mut name_buff, fname_bytes);
    }

    let mut iobuff = [0u8; IOSIZE];
    os_dirname(&mut iobuff);
    let nb = crate::memory::bytes_as_cstr(&name_buff);
    let p = path_shorten_fname(Some(nb), crate::memory::bytes_as_cstr(&iobuff));

    let _ = buflist_new(Some(nb), p, 1, 0);
}

/// Check all file marks for a name that matches the file name in `buf` (`fmarks_check_names`).
pub fn fmarks_check_names(buf: &FileBuf) {
    let Some(name) = buf.ffname_opt().as_deref().map(CStr::to_owned) else {
        return;
    };

    for i in 0..NGLOBALMARKS {
        fmarks_check_one(&mut g_st().namedfm.borrow_mut()[i], &name, buf);
    }

    for wp in win_iter_ptr(first_win_in_tab(&curtab())) {
        for i in 0..(wp.w_jumplistlen.get() as usize) {
            fmarks_check_one(&mut wp.w_jumplist.borrow_mut()[i], &name, buf);
        }
    }
}

fn fmarks_check_one(fm: &mut Xfmark, name: &CStr, buf: &FileBuf) {
    if fm.fmark.fnum == 0
        && fm
            .fname
            .as_ref()
            .is_some_and(|f| path_equal(name, f, K_PATH_CMP_LITERAL))
    {
        fm.fmark.fnum = buf.b_fnum.get();
        fm.fname = None;
    }
}

/// Check the position in `fm` is valid (`mark_check`).
pub fn mark_check(
    fm: Option<&Fmark>,
    errormsg: &mut Option<std::borrow::Cow<'static, CStr>>,
) -> bool {
    let Some(fm) = fm else {
        *errormsg = Some(std::borrow::Cow::Borrowed(gettext(E_UMARK)));
        return false;
    };
    if fm.mark.lnum <= 0 {
        if fm.mark.lnum == 0 {
            *errormsg = Some(std::borrow::Cow::Borrowed(gettext(E_MARKNOTSET)));
        }
        return false;
    }
    if fm.fnum == curbuf().b_fnum.get()
        && !mark_check_line_bounds(Some(&crate::globals::curbuf()), fm, errormsg)
    {
        return false;
    }
    true
}

/// Check if a mark line number is greater than the buffer line count (`mark_check_line_bounds`).
pub fn mark_check_line_bounds(
    buf: Option<&FileBuf>,
    fm: &Fmark,
    errormsg: &mut Option<std::borrow::Cow<'static, CStr>>,
) -> bool {
    if let Some(buf) = buf
        && fm.mark.lnum > buf.b_ml.ml_line_count.get()
    {
        *errormsg = Some(std::borrow::Cow::Borrowed(gettext(E_MARKINVAL)));
        return false;
    }
    true
}

/// Clear all marks and change list in the given buffer (`clrallmarks`).
pub fn clrallmarks(buf: &FileBuf, timestamp: Timestamp) {
    for i in 0..NMARKS {
        clear_fmark(&mut buf.b_namedm.borrow_mut()[i], timestamp);
    }
    clear_fmark(&mut buf.b_last_cursor.borrow_mut(), timestamp);
    buf.b_last_cursor.borrow_mut().mark.lnum = 1;
    clear_fmark(&mut buf.b_last_insert.borrow_mut(), timestamp);
    clear_fmark(&mut buf.b_last_change.borrow_mut(), timestamp);
    buf.b_op_start.set_lnum(0);
    buf.b_op_end.set_lnum(0);
    for i in 0..(buf.b_changelistlen.get() as usize) {
        clear_fmark(&mut buf.b_changelist.borrow_mut()[i], timestamp);
    }
    buf.b_changelistlen.set(0);
}

/// Get name of file from a filemark (`fm_getname`).
pub fn fm_getname(fmark: &Fmark, lead_len: i32) -> Option<std::ffi::CString> {
    if fmark.fnum == curbuf().b_fnum.get() {
        return Some(mark_line(&fmark.mark, lead_len));
    }
    buflist_nr2name(fmark.fnum, 0, 1)
}

/// Return the line at mark `mp`, truncated to fit in window (`mark_line`).
fn mark_line(mp: &CursorPos, lead_len: i32) -> std::ffi::CString {
    if mp.lnum == 0 || mp.lnum > curbuf().b_ml.ml_line_count.get() {
        return c"-invalid-".to_owned();
    }
    debug_assert!(g_st().columns_global.get() >= 0);
    let ml_line = ml_get(mp.lnum);
    let line = crate::charset::skipwhite_slice(ml_line.to_bytes());
    let max_bytes = line.len().min((g_st().columns_global.get() as usize) * 5);

    let mut len: i32 = 0;
    let mut off = 0usize;
    while off < max_bytes {
        len += slice2cells(&line[off..]);
        if len >= g_st().columns_global.get() - lead_len {
            break;
        }
        off += crate::mbyte::utfc_slice2len(&line[off..]);
    }
    let keep = off.min(max_bytes);
    std::ffi::CString::new(&line[..keep]).unwrap_or_default()
}

/// `:marks {arg}` (`ex_marks`).
pub fn ex_marks(eap: &mut Exarg) {
    let cmod = g_st().cmdmod.borrow().clone();
    nlua_call_excmd(c"vim._core.marks", c"ex_marks", eap, &cmod, None);
}

/// `:delmarks[!] [marks]` (`ex_delmarks`).
pub fn ex_delmarks(eap: &mut Exarg) {
    let pos = CursorPos {
        lnum: 0,
        col: 0,
        coladd: 0,
    };

    if eap.arg.is_empty() && eap.forceit {
        for i in 0..NMARKS {
            if curbuf().b_namedm.borrow()[i].mark.lnum != 0 {
                do_markset_autocmd(i as u8 + b'a', &pos, &curbuf());
            }
        }
        if curbuf().b_last_cursor.borrow().mark.lnum != 0 {
            do_markset_autocmd(b'"', &pos, &curbuf());
        }
        if curbuf().b_last_insert.borrow().mark.lnum != 0 {
            do_markset_autocmd(b'^', &pos, &curbuf());
        }
        if curbuf().b_last_change.borrow().mark.lnum != 0 {
            do_markset_autocmd(b'.', &pos, &curbuf());
        }
        if curbuf().b_op_start.get().lnum != 0 {
            do_markset_autocmd(b'[', &pos, &curbuf());
        }
        if curbuf().b_op_end.get().lnum != 0 {
            do_markset_autocmd(b']', &pos, &curbuf());
        }
        clrallmarks(&curbuf(), os_time());
    } else if eap.forceit {
        emsg(gettext(E_INVARG));
    } else if eap.arg.is_empty() {
        emsg(gettext(E_ARGREQ));
    } else {
        let timestamp = os_time();
        let mut p = eap.arg.clone();
        while !(p).is_empty() {
            let ch = i32::from((p).first_byte());
            let lower = ascii_islower(ch);
            let digit = ascii_isdigit(ch);
            if lower || digit || ascii_isupper(ch) {
                let from: i32;
                let to: i32;
                let p_bytes = (p).as_bytes();
                if p_bytes.get(1) == Some(&b'-') {
                    from = ch;
                    to = i32::from(p_bytes.get(2).copied().unwrap_or(0));
                    let valid_to = if lower {
                        ascii_islower(to)
                    } else if digit {
                        ascii_isdigit(to)
                    } else {
                        ascii_isupper(to)
                    };
                    if !valid_to || to < from {
                        semsg!(gettext(E_INVARG2), p);
                        return;
                    }
                    p = p.wrapping_add(2);
                } else {
                    from = ch;
                    to = ch;
                }

                for i in from..=to {
                    if lower {
                        let idx = (i - b'a' as i32) as usize;
                        if curbuf().b_namedm.borrow()[idx].mark.lnum != 0 {
                            do_markset_autocmd(i as u8, &pos, &curbuf());
                        }
                        curbuf().b_namedm.borrow_mut()[idx].mark.lnum = 0;
                        curbuf().b_namedm.borrow_mut()[idx].timestamp = timestamp;
                    } else {
                        let n = if digit {
                            (i - b'0' as i32) as usize + NMARKS
                        } else {
                            (i - b'A' as i32) as usize
                        };
                        if g_st().namedfm.borrow()[n].fmark.mark.lnum != 0 {
                            let buf = buflist_findnr_opt(g_st().namedfm.borrow()[n].fmark.fnum)
                                .unwrap_or_else(curbuf);
                            do_markset_autocmd(i as u8, &pos, &buf);
                        }
                        g_st().namedfm.borrow_mut()[n].fmark.mark.lnum = 0;
                        g_st().namedfm.borrow_mut()[n].fmark.fnum = 0;
                        g_st().namedfm.borrow_mut()[n].fmark.timestamp = timestamp;
                        g_st().namedfm.borrow_mut()[n].fname = None;
                    }
                }
            } else {
                match (p).first_byte() {
                    b'"' => {
                        if curbuf().b_last_cursor.borrow().mark.lnum != 0 {
                            do_markset_autocmd(b'"', &pos, &curbuf());
                        }
                        clear_fmark(&mut curbuf().b_last_cursor.borrow_mut(), timestamp);
                    }
                    b'^' => {
                        if curbuf().b_last_insert.borrow().mark.lnum != 0 {
                            do_markset_autocmd(b'^', &pos, &curbuf());
                        }
                        clear_fmark(&mut curbuf().b_last_insert.borrow_mut(), timestamp);
                    }
                    b':' => {}
                    b'.' => {
                        if curbuf().b_last_change.borrow().mark.lnum != 0 {
                            do_markset_autocmd(b'.', &pos, &curbuf());
                        }
                        clear_fmark(&mut curbuf().b_last_change.borrow_mut(), timestamp);
                    }
                    b'[' => {
                        if curbuf().b_op_start.get().lnum != 0 {
                            do_markset_autocmd(b'[', &pos, &curbuf());
                        }
                        curbuf().b_op_start.set_lnum(0);
                    }
                    b']' => {
                        if curbuf().b_op_end.get().lnum != 0 {
                            do_markset_autocmd(b']', &pos, &curbuf());
                        }
                        curbuf().b_op_end.set_lnum(0);
                    }
                    b'<' => {
                        if curbuf().b_visual.borrow().vi_start.lnum != 0 {
                            do_markset_autocmd(b'<', &pos, &curbuf());
                        }
                        curbuf().b_visual.borrow_mut().vi_start.lnum = 0;
                    }
                    b'>' => {
                        if curbuf().b_visual.borrow().vi_end.lnum != 0 {
                            do_markset_autocmd(b'>', &pos, &curbuf());
                        }
                        curbuf().b_visual.borrow_mut().vi_end.lnum = 0;
                    }
                    b' ' => {}
                    _ => {
                        semsg!(gettext(E_INVARG2), p);
                        return;
                    }
                }
            }
            p = p.wrapping_add(1);
        }
    }
}

/// Print the jumplist (`ex_jumps`).
pub fn ex_jumps(_eap: &mut Exarg) {
    cleanup_jumplist(crate::globals::curwin_handle(), true);
    msg_ext_set_kind(c"list_cmd");
    msg_puts_title(gettext(c"\n jump line  col file/text"));
    let mut i: i32 = 0;
    while i < curwin().w_jumplistlen.get() && !g_st().got_int.get() {
        let idx = i as usize;
        if curwin().w_jumplist.borrow()[idx].fmark.mark.lnum != 0 {
            let mut name = fm_getname(&curwin().w_jumplist.borrow()[idx].fmark, 16);

            if name.is_none() && i == curwin().w_jumplistidx.get() {
                name = Some(c"-invalid-".to_owned());
            }
            let Some(name_cstr) = name else {
                i += 1;
                continue;
            };
            if message_filtered(&name_cstr) {
                i += 1;
                continue;
            }

            msg_putchar(b'\n' as i32);
            if g_st().got_int.get() {
                break;
            }
            let indicator = if i == curwin().w_jumplistidx.get() {
                '>'
            } else {
                ' '
            };
            let dist = (i - curwin().w_jumplistidx.get()).abs();
            let lnum = curwin().w_jumplist.borrow()[idx].fmark.mark.lnum;
            let col = curwin().w_jumplist.borrow()[idx].fmark.mark.col;
            let s = format!("{indicator} {dist:2} {lnum:5} {col:4} ");
            let mut iobuff = [0u8; IOSIZE];
            crate::memory::copy_bytes_to_c_buf(&mut iobuff, s.as_bytes());

            msg_outtrans(crate::memory::bytes_as_cstr(&iobuff), 0, false);
            msg_outtrans(
                &name_cstr,
                if curwin().w_jumplist.borrow()[idx].fmark.fnum == curbuf().b_fnum.get() {
                    HLF_D
                } else {
                    0
                },
                false,
            );
            os_breakcheck();
        }
        i += 1;
    }
    if curwin().w_jumplistidx.get() == curwin().w_jumplistlen.get() {
        msg_puts(c"\n>");
    }
}

pub fn ex_clearjumps(_eap: &mut Exarg) {
    free_jumplist(&curwin());
    curwin().w_jumplistlen.set(0);
    curwin().w_jumplistidx.set(0);
}

/// Print the changelist (`ex_changes`).
pub fn ex_changes(_eap: &mut Exarg) {
    msg_ext_set_kind(c"list_cmd");
    msg_puts_title(gettext(c"\nchange line  col text"));

    let mut i: i32 = 0;
    while i < curbuf().b_changelistlen.get() && !g_st().got_int.get() {
        let idx = i as usize;
        if curbuf().b_changelist.borrow()[idx].mark.lnum != 0 {
            msg_putchar(b'\n' as i32);
            if g_st().got_int.get() {
                break;
            }
            let indicator = if i == curwin().w_changelistidx.get() {
                '>'
            } else {
                ' '
            };
            let dist = (i - curwin().w_changelistidx.get()).abs();
            let lnum = curbuf().b_changelist.borrow()[idx].mark.lnum;
            let col = curbuf().b_changelist.borrow()[idx].mark.col;
            let s = format!("{indicator} {dist:3} {lnum:5} {col:4} ");
            let mut iobuff = [0u8; IOSIZE];
            crate::memory::copy_bytes_to_c_buf(&mut iobuff, s.as_bytes());

            msg_outtrans(crate::memory::bytes_as_cstr(&iobuff), 0, false);
            let mark = curbuf().b_changelist.borrow()[idx].mark;
            let name = mark_line(&mark, 17);
            msg_outtrans(&name, HLF_D, false);
            os_breakcheck();
        }
        i += 1;
    }
    if curwin().w_changelistidx.get() == curbuf().b_changelistlen.get() {
        msg_puts(c"\n>");
    }
}

#[inline]
fn one_adjust(lp: &mut Linenr, line1: Linenr, line2: Linenr, amount: Linenr, amount_after: Linenr) {
    if *lp >= line1 && *lp <= line2 {
        if amount == MAXLNUM {
            *lp = 0;
        } else {
            *lp += amount;
        }
    } else if amount_after != 0 && *lp > line2 {
        *lp += amount_after;
    }
}

#[inline]
fn one_adjust_nodel(
    lp: &mut Linenr,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    if *lp >= line1 && *lp <= line2 {
        if amount == MAXLNUM {
            *lp = line1;
        } else {
            *lp += amount;
        }
    } else if amount_after != 0 && *lp > line2 {
        *lp += amount_after;
    }
}

#[inline]
fn one_adjust_cursor(
    posp: &mut CursorPos,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    if posp.lnum >= line1 && posp.lnum <= line2 {
        if amount == MAXLNUM {
            posp.lnum = (line1 - 1).max(1);
            posp.col = 0;
        } else {
            posp.lnum += amount;
        }
    } else if amount_after != 0 && posp.lnum > line2 {
        posp.lnum += amount_after;
    }
}

/// Adjust marks between `line1` and `line2` (inclusive) to move `amount` lines (`mark_adjust`).
pub fn mark_adjust(
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
    op: ExtmarkOp,
) {
    mark_adjust_buf(
        &curbuf(),
        line1,
        line2,
        amount,
        amount_after,
        true,
        K_MARK_ADJUST_NORMAL,
        op,
    );
}

/// Adjust marks without adjusting folds (`mark_adjust_nofold`).
pub fn mark_adjust_nofold(
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
    op: ExtmarkOp,
) {
    mark_adjust_buf(
        &curbuf(),
        line1,
        line2,
        amount,
        amount_after,
        false,
        K_MARK_ADJUST_NORMAL,
        op,
    );
}

pub fn mark_adjust_buf(
    buf: &FileBuf,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
    adjust_folds: bool,
    mode: MarkAdjustMode,
    op: ExtmarkOp,
) {
    let fnum = buf.b_fnum.get();
    const INITPOS: CursorPos = CursorPos {
        lnum: 1,
        col: 0,
        coladd: 0,
    };

    if line2 < line1 && amount_after == 0 {
        return;
    }

    let by_api = mode == K_MARK_ADJUST_API;
    let by_term = mode == K_MARK_ADJUST_TERM;

    if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
        for i in 0..NMARKS {
            one_adjust(
                &mut buf.b_namedm.borrow_mut()[i].mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
            let fm = &mut g_st().namedfm.borrow_mut()[i];
            if fm.fmark.fnum == fnum {
                one_adjust_nodel(&mut fm.fmark.mark.lnum, line1, line2, amount, amount_after);
            }
        }
        for i in NMARKS..NGLOBALMARKS {
            let fm = &mut g_st().namedfm.borrow_mut()[i];
            if fm.fmark.fnum == fnum {
                one_adjust_nodel(&mut fm.fmark.mark.lnum, line1, line2, amount, amount_after);
            }
        }

        one_adjust(
            &mut buf.b_last_insert.borrow_mut().mark.lnum,
            line1,
            line2,
            amount,
            amount_after,
        );
        one_adjust(
            &mut buf.b_last_change.borrow_mut().mark.lnum,
            line1,
            line2,
            amount,
            amount_after,
        );

        if !equalpos(buf.b_last_cursor.borrow().mark, INITPOS)
            && (!by_term || buf.b_last_cursor.borrow().mark.lnum < buf.b_ml.ml_line_count.get())
        {
            one_adjust(
                &mut buf.b_last_cursor.borrow_mut().mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
        }

        if bt_prompt(Some(buf)) {
            one_adjust_nodel(
                &mut buf.b_prompt_start.borrow_mut().mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
        }

        for i in 0..(buf.b_changelistlen.get() as usize) {
            one_adjust_nodel(
                &mut buf.b_changelist.borrow_mut()[i].mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
        }

        one_adjust_nodel(
            &mut buf.b_visual.borrow_mut().vi_start.lnum,
            line1,
            line2,
            amount,
            amount_after,
        );
        one_adjust_nodel(
            &mut buf.b_visual.borrow_mut().vi_end.lnum,
            line1,
            line2,
            amount,
            amount_after,
        );

        if !qf_mark_adjust(buf, None, line1, line2, amount, amount_after) {
            buf.b_has_qf_entry
                .set(buf.b_has_qf_entry.get() & (!BUF_HAS_QF_ENTRY));
        }
        let mut found_one = false;
        for tab in for_all_tabpages_ptr() {
            for win in win_iter_ptr(first_win_in_tab(&tab)) {
                found_one |= qf_mark_adjust(buf, Some(&win), line1, line2, amount, amount_after);
            }
        }
        if !found_one {
            buf.b_has_qf_entry
                .set(buf.b_has_qf_entry.get() & (!BUF_HAS_LL_ENTRY));
        }
    }

    if op != K_EXTMARK_NOOP {
        extmark_adjust(buf, line1, line2, amount, amount_after, op);
    }

    if curwin().w_buffer.get() == buf {
        curwin().w_pcmark.update_mut(|p| {
            one_adjust(&mut p.lnum, line1, line2, amount, amount_after);
        });
        curwin().w_prev_pcmark.update_mut(|p| {
            one_adjust(&mut p.lnum, line1, line2, amount, amount_after);
        });

        if g_st().saved_cursor.get().lnum != 0 {
            let mut sc = g_st().saved_cursor.get();
            one_adjust_nodel(&mut sc.lnum, line1, line2, amount, amount_after);
            g_st().saved_cursor.set(sc);
        }
    }

    for tab in for_all_tabpages_ptr() {
        for win in win_iter_ptr(first_win_in_tab(&tab)) {
            if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
                for i in 0..(win.w_jumplistlen.get() as usize) {
                    if win.w_jumplist.borrow()[i].fmark.fnum == fnum {
                        one_adjust_nodel(
                            &mut win.w_jumplist.borrow_mut()[i].fmark.mark.lnum,
                            line1,
                            line2,
                            amount,
                            amount_after,
                        );
                    }
                }
            }

            if win.w_buffer.get() == buf {
                if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
                    for i in 0..(win.w_tagstacklen.get() as usize) {
                        if win.w_tagstack.borrow()[i].fmark.fnum == fnum {
                            one_adjust_nodel(
                                &mut win.w_tagstack.borrow_mut()[i].fmark.mark.lnum,
                                line1,
                                line2,
                                amount,
                                amount_after,
                            );
                        }
                    }
                }

                if win.w_old_cursor_lnum.get() != 0 {
                    let mut old_cur = win.w_old_cursor_lnum.get();
                    one_adjust_nodel(&mut old_cur, line1, line2, amount, amount_after);
                    win.w_old_cursor_lnum.set(old_cur);
                    let mut old_vis = win.w_old_visual_lnum.get();
                    one_adjust_nodel(&mut old_vis, line1, line2, amount, amount_after);
                    win.w_old_visual_lnum.set(old_vis);
                }

                if by_api
                    || (if by_term {
                        win.w_cursor.get().lnum < buf.b_ml.ml_line_count.get()
                    } else {
                        win != curwin_ptr()
                    })
                {
                    if win.w_topline.get() >= line1 && win.w_topline.get() <= line2 {
                        if amount == MAXLNUM {
                            if !(by_api && amount_after > line1 - line2 - 1) {
                                win.w_topline.set((line1 - 1).max(1));
                            }
                        } else if win.w_topline.get() > line1 {
                            win.w_topline.set(win.w_topline.get() + (amount));
                        }
                        win.w_topfill.set(0);
                    } else if amount_after != 0
                        && win.w_topline.get() > line2 + if by_api && line2 < line1 { 1 } else { 0 }
                    {
                        win.w_topline.set(win.w_topline.get() + (amount_after));
                        win.w_topfill.set(0);
                    }
                }
                if !by_api
                    && (if by_term {
                        win.w_cursor.get().lnum < buf.b_ml.ml_line_count.get()
                    } else {
                        win != curwin_ptr()
                    })
                {
                    win.w_cursor.update_mut(|c| {
                        one_adjust_cursor(c, line1, line2, amount, amount_after);
                    });
                }

                if adjust_folds {
                    fold_mark_adjust(&win, line1, line2, amount, amount_after);
                }
            }
        }
    }

    diff_mark_adjust(buf, line1, line2, amount, amount_after);

    for wip in &mut *buf.b_wininfo.borrow_mut() {
        if !by_term || wip.wi_mark.mark.lnum < buf.b_ml.ml_line_count.get() {
            one_adjust_cursor(&mut wip.wi_mark.mark, line1, line2, amount, amount_after);
        }
    }
}

#[inline]
fn col_adjust(
    posp: &mut CursorPos,
    lnum: Linenr,
    mincol: Colnr,
    lnum_amount: Linenr,
    col_amount: Colnr,
    spaces_removed: i32,
) {
    if posp.lnum == lnum && posp.col >= mincol {
        posp.lnum += lnum_amount;
        if col_amount < 0 && posp.col <= -col_amount {
            posp.col = 0;
        } else if posp.col < spaces_removed {
            posp.col = col_amount + spaces_removed;
        } else {
            posp.col += col_amount;
        }
    }
}

/// Adjust marks in line `lnum` at column `mincol` and further (`mark_col_adjust`).
pub fn mark_col_adjust(
    lnum: Linenr,
    mincol: Colnr,
    lnum_amount: Linenr,
    col_amount: Colnr,
    spaces_removed: i32,
) {
    let fnum = curbuf().b_fnum.get();

    if (col_amount == 0 && lnum_amount == 0)
        || (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) != 0
    {
        return;
    }

    for i in 0..NMARKS {
        col_adjust(
            &mut curbuf().b_namedm.borrow_mut()[i].mark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        let fm = &mut g_st().namedfm.borrow_mut()[i];
        if fm.fmark.fnum == fnum {
            col_adjust(
                &mut fm.fmark.mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
        }
    }
    for i in NMARKS..NGLOBALMARKS {
        let fm = &mut g_st().namedfm.borrow_mut()[i];
        if fm.fmark.fnum == fnum {
            col_adjust(
                &mut fm.fmark.mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
        }
    }

    col_adjust(
        &mut curbuf().b_last_insert.borrow_mut().mark,
        lnum,
        mincol,
        lnum_amount,
        col_amount,
        spaces_removed,
    );
    col_adjust(
        &mut curbuf().b_last_change.borrow_mut().mark,
        lnum,
        mincol,
        lnum_amount,
        col_amount,
        spaces_removed,
    );

    if bt_prompt(Some(&crate::globals::curbuf())) {
        col_adjust(
            &mut curbuf().b_prompt_start.borrow_mut().mark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
    }

    for i in 0..(curbuf().b_changelistlen.get() as usize) {
        col_adjust(
            &mut curbuf().b_changelist.borrow_mut()[i].mark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
    }

    col_adjust(
        &mut curbuf().b_visual.borrow_mut().vi_start,
        lnum,
        mincol,
        lnum_amount,
        col_amount,
        spaces_removed,
    );
    col_adjust(
        &mut curbuf().b_visual.borrow_mut().vi_end,
        lnum,
        mincol,
        lnum_amount,
        col_amount,
        spaces_removed,
    );

    curwin().w_pcmark.update_mut(|p| {
        col_adjust(p, lnum, mincol, lnum_amount, col_amount, spaces_removed);
    });
    curwin().w_prev_pcmark.update_mut(|p| {
        col_adjust(p, lnum, mincol, lnum_amount, col_amount, spaces_removed);
    });
    {
        let mut sc = g_st().saved_cursor.get();
        col_adjust(
            &mut sc,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        g_st().saved_cursor.set(sc);
    }

    for win in win_iter_ptr(first_win_in_tab(&curtab())) {
        for i in 0..(win.w_jumplistlen.get() as usize) {
            if win.w_jumplist.borrow()[i].fmark.fnum == fnum {
                col_adjust(
                    &mut win.w_jumplist.borrow_mut()[i].fmark.mark,
                    lnum,
                    mincol,
                    lnum_amount,
                    col_amount,
                    spaces_removed,
                );
            }
        }

        if win.w_buffer.get() == g_st().curbuf.get() {
            for i in 0..(win.w_tagstacklen.get() as usize) {
                if win.w_tagstack.borrow()[i].fmark.fnum == fnum {
                    col_adjust(
                        &mut win.w_tagstack.borrow_mut()[i].fmark.mark,
                        lnum,
                        mincol,
                        lnum_amount,
                        col_amount,
                        spaces_removed,
                    );
                }
            }

            if win != curwin_ptr() {
                win.w_cursor.update_mut(|c| {
                    col_adjust(c, lnum, mincol, lnum_amount, col_amount, spaces_removed);
                });
            }
        }
    }
}

/// Remove duplicate marks in the jumplist (`cleanup_jumplist`).
pub fn cleanup_jumplist(wp: crate::globals::WinPtr, loadfiles: bool) {
    if loadfiles {
        let len = wp.w_jumplistlen.get() as usize;
        for i in 0..len {
            let xfm_opt = {
                let jl = wp.w_jumplist.borrow();
                (jl[i].fmark.fnum == 0 && jl[i].fmark.mark.lnum != 0).then(|| jl[i].clone())
            };
            if let Some(xfm) = xfm_opt {
                fname2fnum(&xfm);
            }
        }
    }

    let wp = wp.borrow();
    let mut to: i32 = 0;
    for from in 0..wp.w_jumplistlen.get() {
        let from_u = from as usize;
        if wp.w_jumplistidx.get() == from {
            wp.w_jumplistidx.set(to);
        }
        let mut i = from + 1;
        while i < wp.w_jumplistlen.get() {
            let i_u = i as usize;
            if wp.w_jumplist.borrow()[i_u].fmark.fnum == wp.w_jumplist.borrow()[from_u].fmark.fnum
                && wp.w_jumplist.borrow()[from_u].fmark.fnum != 0
                && wp.w_jumplist.borrow()[i_u].fmark.mark.lnum
                    == wp.w_jumplist.borrow()[from_u].fmark.mark.lnum
            {
                break;
            }
            i += 1;
        }

        let mustfree = if i >= wp.w_jumplistlen.get() {
            false
        } else if i > from + 1 {
            (opt_vars().jop_flags.get() & K_OPT_JOP_FLAG_STACK) == 0
        } else {
            true
        };

        if mustfree {
            wp.w_jumplist.borrow_mut()[from_u].fname = None;
        } else {
            if to != from {
                let entry = wp.w_jumplist.borrow()[from_u].clone();
                wp.w_jumplist.borrow_mut()[to as usize] = entry;
            }
            to += 1;
        }
    }
    if wp.w_jumplistidx.get() == wp.w_jumplistlen.get() {
        wp.w_jumplistidx.set(to);
    }
    wp.w_jumplistlen.set(to);

    if loadfiles && wp.w_jumplistlen.get() != 0 && wp.w_jumplistidx.get() == wp.w_jumplistlen.get()
    {
        let last_idx = (wp.w_jumplistlen.get() - 1) as usize;
        if wp.w_jumplist.borrow()[last_idx].fmark.fnum == curbuf().b_fnum.get()
            && wp.w_jumplist.borrow()[last_idx].fmark.mark.lnum == wp.w_cursor.get().lnum
        {
            wp.w_jumplist.borrow_mut()[last_idx].fname = None;
            wp.w_jumplistlen.set(wp.w_jumplistlen.get() - (1));
            wp.w_jumplistidx.set(wp.w_jumplistidx.get() - (1));
        }
    }
}

/// Copy the jumplist from window `from` to window `to` (`copy_jumplist`).
pub fn copy_jumplist(from: &WinInfoInfo, to: &WinInfoInfo) {
    for i in 0..(from.w_jumplistlen.get() as usize) {
        to.w_jumplist.borrow_mut()[i] = from.w_jumplist.borrow()[i].clone();
    }
    to.w_jumplistlen.set(from.w_jumplistlen.get());
    to.w_jumplistidx.set(from.w_jumplistidx.get());
}

/// Iterate over jumplist items (`mark_jumplist_iter`).
pub fn mark_jumplist_iter(
    iter: Option<usize>,
    win: &WinInfoInfo,
    fm: &mut Xfmark,
) -> Option<usize> {
    if iter.is_none() && win.w_jumplistlen.get() == 0 {
        *fm = INIT_XFMARK;
        return None;
    }
    let idx = iter.unwrap_or(0);
    *fm = win.w_jumplist.borrow()[idx].clone();
    if idx + 1 >= win.w_jumplistlen.get() as usize {
        None
    } else {
        Some(idx + 1)
    }
}

/// Iterate over global marks (`mark_global_iter`).
pub fn mark_global_iter(iter: Option<usize>, name: &mut u8, fm: &mut Xfmark) -> Option<usize> {
    *name = 0;
    let marks = &*g_st().namedfm.borrow();
    let mut idx = iter.unwrap_or(0);
    while idx < NGLOBALMARKS && marks[idx].fmark.mark.lnum == 0 {
        idx += 1;
    }
    if idx >= NGLOBALMARKS || marks[idx].fmark.mark.lnum == 0 {
        return None;
    }
    *name = if idx < NMARKS {
        b'A' + idx as u8
    } else {
        b'0' + (idx - NMARKS) as u8
    };
    *fm = marks[idx].clone();
    idx += 1;
    while idx < NGLOBALMARKS {
        if marks[idx].fmark.mark.lnum != 0 {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

#[inline]
fn next_buffer_mark<'a>(buf: &'a FileBuf, mark_name: &mut u8) -> Option<std::cell::Ref<'a, Fmark>> {
    match *mark_name {
        0 => {
            *mark_name = b'"';
            Some(buf.b_last_cursor.borrow())
        }
        b'"' => {
            *mark_name = b'^';
            Some(buf.b_last_insert.borrow())
        }
        b'^' => {
            *mark_name = b'.';
            Some(buf.b_last_change.borrow())
        }
        b'.' => {
            *mark_name = b'a';
            Some(std::cell::Ref::map(buf.b_namedm.borrow(), |m| &m[0]))
        }
        b'z' => None,
        ch => {
            *mark_name = ch + 1;
            let idx = (*mark_name - b'a') as usize;
            Some(std::cell::Ref::map(buf.b_namedm.borrow(), |m| &m[idx]))
        }
    }
}

/// Iterate over buffer marks (`mark_buffer_iter`).
pub fn mark_buffer_iter(
    iter: Option<u8>,
    buf: &FileBuf,
    name: &mut u8,
    fm: &mut Fmark,
) -> Option<u8> {
    *name = 0;
    let mut mark_name = iter.unwrap_or(0);
    let mut iter_mark = next_buffer_mark(buf, &mut mark_name);
    while let Some(m) = iter_mark {
        if m.mark.lnum != 0 {
            *name = mark_name;
            *fm = m.clone();
            return Some(mark_name);
        }
        iter_mark = next_buffer_mark(buf, &mut mark_name);
    }
    None
}

/// Set global mark (`mark_set_global`).
pub fn mark_set_global(name: u8, fm: Xfmark, update: bool) -> bool {
    let idx = mark_global_index(name);
    if idx == -1 {
        return false;
    }
    let fm_tgt = &mut g_st().namedfm.borrow_mut()[idx as usize];
    if update && fm.fmark.timestamp <= fm_tgt.fmark.timestamp {
        return false;
    }
    if fm_tgt.fmark.mark.lnum != 0 {
        free_xfmark(fm_tgt);
    }
    *fm_tgt = fm;
    true
}

/// Set local mark (`mark_set_local`).
pub fn mark_set_local(name: u8, buf: &FileBuf, fm: Fmark, update: bool) -> bool {
    let ch = i32::from(name);
    let fm_tgt = if ascii_islower(ch) {
        &mut buf.b_namedm.borrow_mut()[(ch - b'a' as i32) as usize]
    } else if name == b'"' {
        &mut buf.b_last_cursor.borrow_mut()
    } else if name == b'^' {
        &mut buf.b_last_insert.borrow_mut()
    } else if name == b':' {
        &mut buf.b_prompt_start.borrow_mut()
    } else if name == b'.' {
        &mut buf.b_last_change.borrow_mut()
    } else {
        return false;
    };
    if update && fm.timestamp <= fm_tgt.timestamp {
        return false;
    }
    *fm_tgt = fm;
    true
}

/// Free items in the jumplist of window `wp` (`free_jumplist`).
pub fn free_jumplist(wp: &WinInfoInfo) {
    for i in 0..(wp.w_jumplistlen.get() as usize) {
        free_xfmark(&mut wp.w_jumplist.borrow_mut()[i]);
    }
    wp.w_jumplistlen.set(0);
}

pub fn set_last_cursor(win: &WinInfoInfo) {
    if !win.w_buffer.get().is_null() {
        reset_fmark(
            &mut win.buf().b_last_cursor.borrow_mut(),
            win.w_cursor.get(),
            0,
            INIT_FMARKV,
        );
    }
}

pub fn free_all_marks() {
    for fm in &mut *g_st().namedfm.borrow_mut() {
        if fm.fmark.mark.lnum != 0 {
            free_xfmark(fm);
        }
        *fm = INIT_XFMARK;
    }
}

/// Adjust position to point to the first byte of a multi-byte character (`mark_mb_adjustpos`).
pub fn mark_mb_adjustpos(buf: &FileBuf, lp: &mut CursorPos) {
    if lp.col > 0 || lp.coladd > 1 {
        let line_slice = crate::memline::ml_get_buf_slice(buf, lp.lnum);
        if line_slice.is_empty() || (line_slice.len() as Colnr) < lp.col {
            lp.col = 0;
        } else {
            lp.col -= crate::mbyte::utf_head_off_slice(&line_slice, lp.col as usize);
        }
        let rem = line_slice.get(lp.col as usize..).unwrap_or(&[]);
        if lp.coladd == 1
            && rem.first().copied().unwrap_or(0) != TAB
            && vim_isprintc(crate::mbyte::utf_slice2char(rem))
            && slice2cells(rem) > 1
        {
            lp.coladd = 0;
        }
    }
}

/// Add information about mark `mname` to list `l` (`add_mark`).
fn add_mark(l: &TvList, mname: &CStr, pos: &CursorPos, bufnr: i32, fname: Option<&CStr>) -> bool {
    if pos.lnum <= 0 {
        return true;
    }

    let d = tv_dict_alloc();
    tv_list_append_dict(l, Some(&d));

    let lpos = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    tv_list_append_number(&lpos, i64::from(bufnr));
    tv_list_append_number(&lpos, i64::from(pos.lnum));
    tv_list_append_number(
        &lpos,
        i64::from(if pos.col < MAXCOL {
            pos.col + 1
        } else {
            MAXCOL
        }),
    );
    tv_list_append_number(&lpos, i64::from(pos.coladd));

    if !tv_dict_add_str_len(&d, c"mark", Some(mname.to_bytes()))
        || !tv_dict_add_list(&d, c"pos", Some(&lpos))
        || fname.is_some_and(|f| !tv_dict_add_str(&d, c"file", Some(f)))
    {
        return false;
    }

    true
}

/// Get information about marks local to a buffer (`get_buf_local_marks`).
pub fn get_buf_local_marks(buf: &FileBuf, l: &TvList) {
    let mut mname: [u8; 3] = [b'\'', b' ', 0];

    for i in 0..NMARKS {
        mname[1] = b'a' + i as u8;
        add_mark(
            l,
            CStr::from_bytes_with_nul(&mname).unwrap_or_default(),
            &buf.b_namedm.borrow()[i].mark,
            buf.b_fnum.get(),
            None,
        );
    }

    add_mark(
        l,
        c"''",
        &curwin().w_pcmark.get(),
        curbuf().b_fnum.get(),
        None,
    );
    add_mark(
        l,
        c"'\"",
        &buf.b_last_cursor.borrow().mark,
        buf.b_fnum.get(),
        None,
    );
    add_mark(l, c"'[", &buf.b_op_start.get(), buf.b_fnum.get(), None);
    add_mark(l, c"']", &buf.b_op_end.get(), buf.b_fnum.get(), None);
    add_mark(
        l,
        c"'^",
        &buf.b_last_insert.borrow().mark,
        buf.b_fnum.get(),
        None,
    );
    add_mark(
        l,
        c"'.",
        &buf.b_last_change.borrow().mark,
        buf.b_fnum.get(),
        None,
    );
    if bt_prompt(Some(buf)) {
        add_mark(
            l,
            c"':",
            &buf.b_prompt_start.borrow().mark,
            buf.b_fnum.get(),
            None,
        );
    }
    add_mark(
        l,
        c"'<",
        &buf.b_visual.borrow().vi_start,
        buf.b_fnum.get(),
        None,
    );
    add_mark(
        l,
        c"'>",
        &buf.b_visual.borrow().vi_end,
        buf.b_fnum.get(),
        None,
    );
}

/// Get a global mark (`get_raw_global_mark`).
#[cfg(test)]
pub fn get_raw_global_mark(name: u8) -> Xfmark {
    g_st().namedfm.borrow()[mark_global_index(name) as usize].clone()
}

/// Get information about global marks (`get_global_marks`).
pub fn get_global_marks(l: &TvList) {
    let mut mname: [u8; 3] = [b'\'', b' ', 0];

    for (i, fm) in g_st().namedfm.borrow()[..(NMARKS + EXTRA_MARKS)]
        .iter()
        .enumerate()
    {
        let owned_name = if fm.fmark.fnum != 0 {
            buflist_nr2name(fm.fmark.fnum, 1, 1)
        } else {
            None
        };
        let name = if fm.fmark.fnum != 0 {
            owned_name.as_deref()
        } else {
            fm.fname_cstr()
        };
        if let Some(name) = name {
            mname[1] = if i >= NMARKS {
                b'0' + (i - NMARKS) as u8
            } else {
                b'A' + i as u8
            };

            add_mark(
                l,
                CStr::from_bytes_with_nul(&mname).unwrap_or_default(),
                &fm.fmark.mark,
                fm.fmark.fnum,
                Some(name),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_mark_index_and_set_get_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(mark_global_index(b'A'), 0);
        assert_eq!(mark_global_index(b'Z'), 25);
        assert_eq!(mark_global_index(b'0'), 26);
        assert_eq!(mark_global_index(b'9'), 35);
        assert_eq!(mark_global_index(b'a'), -1);

        let mut xfm = INIT_XFMARK;
        xfm.fmark.mark = CursorPos {
            lnum: 42,
            col: 7,
            coladd: 0,
        };
        xfm.fmark.fnum = 1;
        xfm.fmark.timestamp = 100;
        assert!(mark_set_global(b'A', xfm, false));
        let got = get_raw_global_mark(b'A');
        assert_eq!(got.fmark.mark.lnum, 42);
        assert_eq!(got.fmark.mark.col, 7);
        free_all_marks();
    }
}
