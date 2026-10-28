//! Functions for setting marks and jumping to them (`src/nvim/mark.c`).

use crate::api::private::validate::{
    Dict, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::autocmd::{AUGROUP_ALL, EVENT_MARKSET, aucmd_defer, has_event};
use crate::buffer::{
    GETF_SETMARK, MAXLNUM, NMARKS, bt_prompt, buf_T, buflist_findnr, buflist_getfile, buflist_new,
    buflist_nr2name, first_win_in_tab, kMarkAdjustNormal,
};
use crate::charset::{NUL, ascii_isdigit, ascii_islower, ascii_isupper, skipwhite, vim_isprintc};
use crate::context::{INIT_FMARK, INIT_FMARKV};
use crate::cursor::{check_cursor, colnr_T, equalpos, linenr_T, lt, pos_T};
use crate::diff::diff_mark_adjust;
use crate::errors::*;
use crate::eval::typval::{
    list_T, tv_dict_add_list, tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_list_alloc,
    tv_list_append_dict, tv_list_append_number,
};
use crate::ex_docmd::{CMOD_KEEPJUMPS, CMOD_LOCKMARKS, exarg_T};
use crate::extmark::{ExtmarkOp, extmark_adjust, kExtmarkNOOP};
use crate::fold::{fold_T, foldMarkAdjust, hasFolding};
use crate::globals::*;
use crate::insert::beginline;
use crate::lua::executor::nlua_call_excmd;
use crate::mbyte::{ptr2cells, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::mcursor::mc_replaying;
use crate::memline::{ml_get, ml_get_buf, ml_get_buf_len};
use crate::memory::xstrlcpy;
use crate::message::{
    emsg, message_filtered, msg_ext_set_kind, msg_outtrans, msg_putchar, msg_puts, msg_puts_title,
    semsg,
};
use crate::r#move::set_topline;
use crate::option_vars::*;
use crate::os::env::expand_env;
use crate::os::fs::{MAXPATHL, os_dirname};
use crate::os::input::os_breakcheck;
use crate::os::time::{Timestamp, os_time};
use crate::path::{kPathCmpLiteral, path_equal, path_shorten_fname, vim_ispathsep_nocolon};
use crate::plines::linetabsize_eol;
use crate::quickfix::qf_mark_adjust;
use crate::strings::gettext;
use crate::textformat::oparg_T;
use crate::textobject::{findpar, findsent};
use crate::undo::u_update_named_mark;
use crate::window::{win_T, winopt_T};
use std::ffi::{c_char, c_int, c_uint};
use std::ptr;

pub type MarkMoveRes = c_int;
pub type MarkMove = c_int;
pub type MarkGet = c_int;
pub type MarkAdjustMode = c_int;

pub(crate) const kMarkAdjustApi: MarkAdjustMode = 1;
pub(crate) const kMarkAllNoResolve: MarkGet = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fmarkv_T {
    pub topline_offset: i32,
    pub skipcol: i32,
}

#[derive(Clone)]
pub struct fmark_T {
    pub mark: pos_T,
    pub fnum: c_int,
    pub timestamp: u64,
    pub view: fmarkv_T,
    pub additional_data: Option<crate::search::AdditionalData>,
}

impl fmark_T {
    pub const ZERO: Self = Self {
        mark: pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        },
        fnum: 0,
        timestamp: 0,
        view: fmarkv_T {
            topline_offset: 0,
            skipcol: 0,
        },
        additional_data: None,
    };

    pub const fn new() -> Self {
        Self::ZERO
    }
}

impl Default for fmark_T {
    fn default() -> Self {
        Self::ZERO
    }
}

#[derive(Clone)]
pub struct xfmark_T {
    pub fmark: fmark_T,
    pub fname: Option<std::ffi::CString>,
}

impl xfmark_T {
    pub const ZERO: Self = Self {
        fmark: fmark_T::ZERO,
        fname: None,
    };

    pub const fn new() -> Self {
        Self::ZERO
    }

    #[inline]
    pub fn fname_ptr(&self) -> *mut c_char {
        self.fname
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

impl Default for xfmark_T {
    fn default() -> Self {
        Self::ZERO
    }
}
use crate::change::TAB;
use crate::cmdexpand::HLF_D;
use crate::eval::kListLenMayKnow;
use crate::ex_cmds::{BL_FIX, BL_WHITE};
use crate::globals::IOSIZE;
use crate::regexp::{MAXCOL, kMarkBufLocal};
use crate::search::{BACKWARD, FORWARD};

#[repr(C)]
pub struct WinInfo {
    pub wi_win: *mut win_T,
    pub wi_mark: fmark_T,
    pub wi_optset: bool,
    pub wi_opt: winopt_T,
    pub wi_fold_manual: bool,
    pub wi_folds: Vec<fold_T>,
    pub wi_changelistidx: c_int,
}

impl WinInfo {
    pub const fn new() -> Self {
        Self {
            wi_win: ptr::null_mut(),
            wi_mark: fmark_T::ZERO,
            wi_optset: false,
            wi_opt: winopt_T::ZERO,
            wi_fold_manual: false,
            wi_folds: Vec::new(),
            wi_changelistidx: 0,
        }
    }
}

impl Drop for WinInfo {
    fn drop(&mut self) {
        if self.wi_optset {
                crate::option::clear_winopt(&mut self.wi_opt);
            }
    }
}

pub(crate) const EXTRA_MARKS: usize = 10;
const NGLOBALMARKS: usize = NMARKS + EXTRA_MARKS;
const NMARK_LOCAL_MAX: c_int = 126;
pub(crate) const JUMPLISTSIZE: c_int = 100;

pub(crate) const kMarkMoveSuccess: MarkMoveRes = 1;
pub(crate) const kMarkMoveFailed: MarkMoveRes = 2;
pub(crate) const kMarkSwitchedBuf: MarkMoveRes = 4;
const kMarkChangedCol: MarkMoveRes = 8;
pub(crate) const kMarkChangedLine: MarkMoveRes = 16;
pub(crate) const kMarkChangedCursor: MarkMoveRes = 32;

pub(crate) const kMarkBeginLine: MarkMove = 1;
pub(crate) const kMarkContext: MarkMove = 2;
pub(crate) const kMarkSetView: MarkMove = 8;
pub(crate) const kMarkJumpList: MarkMove = 16;

pub(crate) const kMarkAdjustTerm: MarkAdjustMode = 2;

const kOptJopFlagStack: c_uint = 0x01;

pub(crate) const BUF_HAS_QF_ENTRY: c_int = 1;
pub(crate) const BUF_HAS_LL_ENTRY: c_int = 2;

const INIT_XFMARK: xfmark_T = xfmark_T {
    fmark: INIT_FMARK,
    fname: None,
};

struct MarkModuleState {
    POS_TO_MARK_FMS: fmark_T,
    MARK_MOVE_TO_FM_COPY: fmark_T,
}

impl MarkModuleState {
    const fn new() -> Self {
        Self {
            POS_TO_MARK_FMS: INIT_FMARK,
            MARK_MOVE_TO_FM_COPY: INIT_FMARK,
        }
    }
}

static MARK_STATE: crate::globals::GlobalState<MarkModuleState> = crate::globals::GlobalState::new(MarkModuleState::new());

#[inline]
fn mark_st() -> &'static mut MarkModuleState {
    unsafe { &mut *MARK_STATE.as_ptr() }
}

#[inline]
pub const fn mark_global_index(name: c_char) -> c_int {
    let u = name as u8;
    if u.is_ascii_uppercase() {
        (u - b'A') as c_int
    } else if u.is_ascii_digit() {
        (NMARKS as c_int) + ((u - b'0') as c_int)
    } else {
        -1
    }
}

#[inline]
fn set_fmark(fmarkp: &mut fmark_T, mark: pos_T, fnum: c_int, view: fmarkv_T) {
    fmarkp.mark = mark;
        fmarkp.fnum = fnum;
        fmarkp.timestamp = os_time();
        fmarkp.view = view;
        fmarkp.additional_data = None;
}

#[inline]
pub fn reset_fmark(fmarkp: &mut fmark_T, mark: pos_T, fnum: c_int, view: fmarkv_T) {
    set_fmark(fmarkp, mark, fnum, view);
}

#[inline]
fn set_xfmark(
    xfmarkp: &mut xfmark_T,
    mark: pos_T,
    fnum: c_int,
    view: fmarkv_T,
    fname: Option<std::ffi::CString>,
) {
    xfmarkp.fname = fname;
    set_fmark(&mut xfmarkp.fmark, mark, fnum, view);
}

#[inline]
fn reset_xfmark(
    xfmarkp: &mut xfmark_T,
    mark: pos_T,
    fnum: c_int,
    view: fmarkv_T,
    fname: Option<std::ffi::CString>,
) {
    {
        free_xfmark(&mut *xfmarkp);
        set_xfmark(&mut *xfmarkp, mark, fnum, view, fname);
    }
}

/// Set named mark `c` at current cursor position (`setmark`).
pub fn setmark(c: c_int) -> bool {
     {
        let view = mark_view_make(curwin_mut(), curwin_mut().w_cursor);
        setmark_pos(
            c,
            &mut curwin_mut().w_cursor,
            curbuf_mut().b_fnum,
            Some(&view),
        )
    }
}

/// Free `xfmark_T` item (`free_xfmark`).
pub fn free_xfmark(fm: &mut xfmark_T) {
    fm.fname = None;
    fm.fmark.additional_data = None;
}

/// Free and clear `fmark_T` item (`clear_fmark`).
pub fn clear_fmark(fm: &mut fmark_T, timestamp: Timestamp) {
    *fm = INIT_FMARK;
        fm.timestamp = timestamp;
}

/// Schedules `"MarkSet"` event (`do_markset_autocmd`).
fn do_markset_autocmd(c: c_char, pos: &pos_T, buf: &mut buf_T) {
     {
        if !has_event(EVENT_MARKSET) {
            return;
        }

        let mut mark_str: [c_char; 2] = [c, 0];
        let mut items: [KeyValuePair; 3] = [
            KeyValuePair {
                key: NvimString {
                    data: c"name".as_ptr().cast_mut(),
                    size: 4,
                },
                value: Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(NvimString {
                            data: mark_str.as_mut_ptr(),
                            size: 1,
                        }),
                },
            },
            KeyValuePair {
                key: NvimString {
                    data: c"line".as_ptr().cast_mut(),
                    size: 4,
                },
                value: Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData::Integer(i64::from(pos.lnum)),
                },
            },
            KeyValuePair {
                key: NvimString {
                    data: c"col".as_ptr().cast_mut(),
                    size: 3,
                },
                value: Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData::Integer(i64::from(pos.col)),
                },
            },
        ];
        let data = Dict {
            items: items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        let mut dict_obj = Object {
            obj_type: ObjectType::kObjectTypeDict,
            data: ObjectData::Dict(data),
        };
        aucmd_defer(
            EVENT_MARKSET,
            mark_str.as_mut_ptr(),
            ptr::null_mut(),
            AUGROUP_ALL,
            buf,
            Some(&mut dict_obj),
        );
    }
}

/// Set named mark `c` to position `pos` (`setmark_pos`).
pub fn setmark_pos(c: c_int, pos: &mut pos_T, fnum: c_int,view_pt: Option<&fmarkv_T>) -> bool {
    unsafe {
        let view = if let Some(view_pt) = view_pt {
            *view_pt
        } else {
            INIT_FMARKV
        };

        if c < 0 {
            return false;
        }

        if c == b'\'' as c_int || c == b'`' as c_int {
            if ptr::eq(pos, &raw const (*curwin_ptr()).w_cursor) {
                setpcmark();
                curwin_mut().w_prev_pcmark = curwin_mut().w_pcmark;
            } else {
                curwin_mut().w_pcmark = *pos;
            }
            return true;
        }

        let buf = buflist_findnr(fnum);
        if buf.is_null() {
            return false;
        }

        if c == b'"' as c_int {
            reset_fmark(&mut (*buf).b_last_cursor, *pos, (*buf).b_fnum, view);
            do_markset_autocmd(c as c_char, pos,  &mut *buf);
            return true;
        }

        if c == b'[' as c_int {
            (*buf).b_op_start = *pos;
            do_markset_autocmd(c as c_char, pos,  &mut *buf);
            return true;
        }
        if c == b']' as c_int {
            (*buf).b_op_end = *pos;
            do_markset_autocmd(c as c_char, pos,  &mut *buf);
            return true;
        }

        if c == b'<' as c_int || c == b'>' as c_int {
            if c == b'<' as c_int {
                (*buf).b_visual.vi_start = *pos;
            } else {
                (*buf).b_visual.vi_end = *pos;
            }
            if (*buf).b_visual.vi_mode == c_int::from(NUL) {
                (*buf).b_visual.vi_mode = b'v' as c_int;
            }
            do_markset_autocmd(c as c_char, pos,  &mut *buf);
            return true;
        }

        if c == b':' as c_int && bt_prompt(buf.as_ref()) {
            reset_fmark(&mut (*buf).b_prompt_start, *pos, (*buf).b_fnum, view);
            return true;
        }

        if ascii_islower(c) {
            let i = (c - b'a' as c_int) as usize;
            reset_fmark(&mut (*buf).b_namedm[i], *pos, fnum, view);
            u_update_named_mark(&mut *buf, i as c_int);
            do_markset_autocmd(c as c_char, pos,  &mut *buf);
            return true;
        }
        if ascii_isupper(c) || ascii_isdigit(c) {
            let i = if ascii_isdigit(c) {
                (c - b'0' as c_int) as usize + NMARKS
            } else {
                (c - b'A' as c_int) as usize
            };
            reset_xfmark(&mut g_st().namedfm[i], *pos, fnum, view, None);
            do_markset_autocmd(c as c_char, pos,  &mut *buf);
            return true;
        }
        false
    }
}

/// Remove every jump list entry referring to a given buffer (`mark_jumplist_forget_file`).
pub fn mark_jumplist_forget_file(wp: &mut win_T, fnum: c_int) {
    let mut i = wp.w_jumplistlen - 1;
        while i >= 0 {
            let idx = i as usize;
            if wp.w_jumplist[idx].fmark.fnum == fnum {
                free_xfmark(&mut wp.w_jumplist[idx]);
                if wp.w_jumplistidx > i {
                    wp.w_jumplistidx -= 1;
                }
                let old_len = wp.w_jumplistlen as usize;
                (&mut wp.w_jumplist)[idx..old_len].rotate_left(1);
                wp.w_jumplist[old_len - 1] = xfmark_T::ZERO;
                wp.w_jumplistlen -= 1;
            }
            i -= 1;
        }
}

/// Delete every entry referring to file `fnum` from both the jumplist and the tag stack (`mark_forget_file`).
pub fn mark_forget_file(wp: &mut win_T, fnum: c_int) {
    mark_jumplist_forget_file(wp, fnum);

        let mut i = wp.w_tagstacklen - 1;
        while i >= 0 {
            let idx = i as usize;
            if wp.w_tagstack[idx].fmark.fnum == fnum {
                let old_len = wp.w_tagstacklen as usize;
                (&mut wp.w_tagstack)[idx..old_len].rotate_left(1);
                wp.w_tagstack[old_len - 1] = crate::tag::taggy_T::ZERO;
                if wp.w_tagstackidx > i {
                    wp.w_tagstackidx -= 1;
                }
                wp.w_tagstacklen -= 1;
            }
            i -= 1;
        }
}

/// Set the previous context mark to the current position and add it to the jump list (`setpcmark`).
pub fn setpcmark() {
    unsafe {
        if g_st().global_busy != 0
            || g_st().listcmd_busy
            || (g_st().cmdmod.cmod_flags & CMOD_KEEPJUMPS) != 0
            || mc_replaying()
        {
            return;
        }

        curwin_mut().w_prev_pcmark = curwin_mut().w_pcmark;
        curwin_mut().w_pcmark = curwin_mut().w_cursor;

        if curwin_mut().w_pcmark.lnum == 0 {
            curwin_mut().w_pcmark.lnum = 1;
        }

        if (opt_vars().jop_flags & kOptJopFlagStack) != 0
            && curwin_mut().w_jumplistidx < curwin_mut().w_jumplistlen - 1
        {
            curwin_mut().w_jumplistlen = curwin_mut().w_jumplistidx + 1;
        }

        curwin_mut().w_jumplistlen += 1;
        if curwin_mut().w_jumplistlen > JUMPLISTSIZE {
            curwin_mut().w_jumplistlen = JUMPLISTSIZE;
            free_xfmark(&mut curwin_mut().w_jumplist[0]);
            (&mut curwin_mut().w_jumplist)[..JUMPLISTSIZE as usize].rotate_left(1);
        }
        curwin_mut().w_jumplistidx = curwin_mut().w_jumplistlen;
        let fm = &raw mut (*curwin_ptr()).w_jumplist[(curwin_mut().w_jumplistlen - 1) as usize];

        let view = mark_view_make(curwin_mut(), curwin_mut().w_pcmark);
        set_xfmark(&mut *fm, curwin_mut().w_pcmark, curbuf_mut().b_fnum, view, None);
    }
}

/// Check previous context mark (`checkpcmark`).
pub fn checkpcmark() {
     {
        if curwin_mut().w_prev_pcmark.lnum != 0
            && (equalpos(curwin_mut().w_pcmark, curwin_mut().w_cursor) || curwin_mut().w_pcmark.lnum == 0)
        {
            curwin_mut().w_pcmark = curwin_mut().w_prev_pcmark;
        }
        curwin_mut().w_prev_pcmark.lnum = 0;
    }
}

/// Get mark in `count` position in the jumplist relative to the current index (`get_jumplist`).
pub fn get_jumplist(win: &mut win_T, mut count: c_int) -> *mut fmark_T {
    unsafe {
        cleanup_jumplist(win, true);

        if win.w_jumplistlen == 0 {
            return ptr::null_mut();
        }

        loop {
            if win.w_jumplistidx + count < 0
                || win.w_jumplistidx + count >= win.w_jumplistlen
            {
                return ptr::null_mut();
            }

            if win.w_jumplistidx == win.w_jumplistlen {
                setpcmark();
                win.w_jumplistidx -= 1;
                if win.w_jumplistidx + count < 0 {
                    return ptr::null_mut();
                }
            }

            win.w_jumplistidx += count;

            let jmp = &raw mut win.w_jumplist[win.w_jumplistidx as usize];
            if (*jmp).fmark.fnum == 0 {
                fname2fnum(&mut *jmp);
            }
            if (*jmp).fmark.fnum != curbuf_mut().b_fnum && buflist_findnr((*jmp).fmark.fnum).is_null()
            {
                count += if count < 0 { -1 } else { 1 };
                continue;
            }
            return &raw mut (*jmp).fmark;
        }
    }
}

/// Get mark in `count` position in the changelist relative to the current index (`get_changelist`).
pub fn get_changelist(buf: &mut buf_T, win: &mut win_T, count: c_int) -> *mut fmark_T {
    unsafe {
        if buf.b_changelistlen == 0 {
            return ptr::null_mut();
        }

        let mut n = win.w_changelistidx;
        if n + count < 0 {
            if n == 0 {
                return ptr::null_mut();
            }
            n = 0;
        } else if n + count >= buf.b_changelistlen {
            if n == buf.b_changelistlen - 1 {
                return ptr::null_mut();
            }
            n = buf.b_changelistlen - 1;
        } else {
            n += count;
        }
        win.w_changelistidx = n;
        let fm = &raw mut buf.b_changelist[n as usize];
        (*fm).fnum = curbuf_mut().b_fnum;
        fm
    }
}

/// Get a named mark (`mark_get`).
pub fn mark_get(
    buf: &mut buf_T,
    win: &mut win_T,fmp: Option<&mut fmark_T>,
    flag: MarkGet,
    name: c_int,
) -> *mut fmark_T {
    unsafe {
        let mut fm: *mut fmark_T = ptr::null_mut();
        if ascii_isupper(name) || ascii_isdigit(name) {
            let xfm = mark_get_global(flag != kMarkAllNoResolve, name);
            fm = &raw mut (*xfm).fmark;
            if flag == kMarkBufLocal && (*xfm).fmark.fnum != buf.b_fnum {
                return pos_to_mark(
                    &*buf,
                    None,
                    pos_T {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    },
                );
            }
        } else if name > 0 && name < NMARK_LOCAL_MAX {
            fm = mark_get_local(buf,  win, name);
        }
        if let Some(fmp) = fmp
            && !fm.is_null()
        {
            *fmp = (*fm).clone();
            return &raw mut *fmp;
        }
        fm
    }
}

/// Get a global mark `{A-Z0-9}` (`mark_get_global`).
pub fn mark_get_global(resolve: bool, mut name: c_int) -> *mut xfmark_T {
    unsafe {
        if ascii_isdigit(name) {
            name = name - b'0' as c_int + NMARKS as c_int;
        } else if ascii_isupper(name) {
            name -= b'A' as c_int;
        } else {
            debug_assert!(false);
        }
        let mark = &raw mut g_st().namedfm[name as usize];
        if resolve && (*mark).fmark.fnum == 0 {
            fname2fnum(&mut *mark);
        }
        mark
    }
}

/// Get a local mark (`mark_get_local`).
pub fn mark_get_local(buf: &mut buf_T, win: &mut win_T, name: c_int) -> *mut fmark_T {
    unsafe {
        let mark = if ascii_islower(name) {
            &raw mut buf.b_namedm[(name - b'a' as c_int) as usize]
        } else if name == b'[' as c_int {
            pos_to_mark(buf, None, buf.b_op_start)
        } else if name == b']' as c_int {
            pos_to_mark(buf, None, buf.b_op_end)
        } else if name == b'<' as c_int || name == b'>' as c_int {
            mark_get_visual(buf, name)
        } else if name == b'\'' as c_int || name == b'`' as c_int {
            pos_to_mark(curbuf_mut(), None, win.w_pcmark)
        } else if name == b'"' as c_int {
            &raw mut buf.b_last_cursor
        } else if name == b'^' as c_int {
            &raw mut buf.b_last_insert
        } else if name == b'.' as c_int {
            &raw mut buf.b_last_change
        } else if name == b':' as c_int && bt_prompt(Some(&*buf)) {
            &raw mut buf.b_prompt_start
        } else {
            mark_get_motion(&mut *buf,  win, name)
        };

        if !mark.is_null() {
            (*mark).fnum = buf.b_fnum;
        }

        mark
    }
}

/// Get marks that are actually motions (`mark_get_motion`).
pub fn mark_get_motion(buf: &mut buf_T, win: &mut win_T, name: c_int) -> *mut fmark_T {
     {
        let mut mark: *mut fmark_T = ptr::null_mut();
        let pos = curwin_mut().w_cursor;
        let slcb = g_st().listcmd_busy;
        g_st().listcmd_busy = true;
        if name == b'{' as c_int || name == b'}' as c_int {
            let mut oa = oparg_T::new();
            if findpar(
                &mut oa.inclusive,
                if name == b'}' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                1,
                c_int::from(NUL),
                false,
            ) {
                mark = pos_to_mark( &*buf, None, win.w_cursor);
            }
        } else if (name == b'(' as c_int || name == b')' as c_int)
            && findsent(
                if name == b')' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                1,
            )
        {
            mark = pos_to_mark( &*buf, None, win.w_cursor);
        }
        curwin_mut().w_cursor = pos;
        g_st().listcmd_busy = slcb;
        mark
    }
}

/// Get visual marks `'<'`, `'>'` (`mark_get_visual`).
pub fn mark_get_visual(buf: &mut buf_T, name: c_int) -> *mut fmark_T {
    unsafe {
        let mut mark: *mut fmark_T = ptr::null_mut();
        if name == b'<' as c_int || name == b'>' as c_int {
            let startp = buf.b_visual.vi_start;
            let endp = buf.b_visual.vi_end;
            mark = if ((name == b'<' as c_int) == lt(startp, endp) || endp.lnum == 0)
                && startp.lnum != 0
            {
                pos_to_mark(&*buf, None, startp)
            } else {
                pos_to_mark(&*buf, None, endp)
            };

            if buf.b_visual.vi_mode == b'V' as c_int {
                (*mark).mark.col = if name == b'<' as c_int { 0 } else { MAXCOL };
                (*mark).mark.coladd = 0;
            }
        }
        mark
    }
}

/// Wrap a `pos_T` into an `fmark_T` (`pos_to_mark`).
pub fn pos_to_mark(buf: &buf_T, fmp: Option<&mut fmark_T>, pos: pos_T) -> *mut fmark_T {
    let fm = fmp.unwrap_or(&mut mark_st().POS_TO_MARK_FMS);
    fm.fnum = buf.b_fnum;
    fm.mark = pos;
    &raw mut *fm
}

fn switch_to_mark_buf(fm: &mut fmark_T, pcmark_on_switch: bool) -> MarkMoveRes {
     {
        if fm.fnum != curbuf_mut().b_fnum {
            let getfile_flag = if pcmark_on_switch { GETF_SETMARK } else { 0 };
            let res = buflist_getfile(fm.fnum, fm.mark.lnum, getfile_flag, 0);
            return if res {
                kMarkSwitchedBuf
            } else {
                kMarkMoveFailed
            };
        }
        0
    }
}

/// Move to the given file mark, changing the buffer and cursor position (`mark_move_to`).
pub fn mark_move_to(mut fm: *mut fmark_T, flags: MarkMove) -> MarkMoveRes {
    unsafe {
        let mut res: MarkMoveRes = kMarkMoveSuccess;
        let mut errormsg: *const c_char = ptr::null();
        if !mark_check(fm.as_mut(), &mut errormsg) {
            if !errormsg.is_null() {
                emsg(errormsg);
            }
            return kMarkMoveFailed;
        }

        if (*fm).fnum != curbuf_mut().b_fnum {
            mark_st().MARK_MOVE_TO_FM_COPY = (*fm).clone();
            fm = &raw mut mark_st().MARK_MOVE_TO_FM_COPY;
            res |= switch_to_mark_buf( &mut *fm, (flags & kMarkJumpList) == 0);
            if (res & kMarkMoveFailed) != 0 {
                return res;
            }
            if !mark_check_line_bounds(Some(crate::globals::curbuf_mut()),  &mut *fm, &mut errormsg) {
                if !errormsg.is_null() {
                    emsg(errormsg);
                }
                res |= kMarkMoveFailed;
                return res;
            }
        } else if (flags & kMarkContext) != 0 {
            setpcmark();
        }

        let prev_pos = curwin_mut().w_cursor;
        let pos = (*fm).mark;
        curwin_mut().w_cursor = (*fm).mark;
        if (flags & kMarkBeginLine) != 0 {
            beginline(BL_WHITE | BL_FIX);
        }
        if prev_pos.lnum != pos.lnum {
            res |= kMarkChangedLine | kMarkChangedCursor;
        }
        if prev_pos.col != pos.col {
            res |= kMarkChangedCol | kMarkChangedCursor;
        }
        if (flags & kMarkSetView) != 0 {
            mark_view_restore(fm.as_ref());
        }

        if (res & kMarkSwitchedBuf) != 0 || (res & kMarkChangedCursor) != 0 {
            check_cursor(curwin_mut());
        }
        res
    }
}

/// Restore the mark view (`mark_view_restore`).
pub fn mark_view_restore(fm: Option<&fmark_T>) {
     {
        if let Some(fm) = fm && fm.view.topline_offset >= 0 {
            let topline = fm.mark.lnum - fm.view.topline_offset;
            if topline >= 1 {
                set_topline(curwin_mut(), topline);
                curwin_mut().w_skipcol = if fm.view.skipcol > 0
                    && !hasFolding(curwin_ptr(), topline, None, ptr::null_mut())
                    && fm.view.skipcol < linetabsize_eol(curwin_mut(), topline)
                {
                    fm.view.skipcol
                } else {
                    0
                };
            }
        }
    }
}

pub fn mark_view_make(wp: &win_T, pos: pos_T) -> fmarkv_T {
    fmarkv_T {
            topline_offset: pos.lnum - wp.w_topline,
            skipcol: wp.w_skipcol,
        }
}

/// Search for the next named mark in the current file from a start position (`getnextmark`).
pub fn getnextmark(startpos: &mut pos_T, dir: c_int, begin_line: c_int) -> *mut fmark_T {
    unsafe {
        let mut result: *mut fmark_T = ptr::null_mut();
        let mut pos = *startpos;

        if dir == BACKWARD && begin_line != 0 {
            pos.col = 0;
        } else if dir == FORWARD && begin_line != 0 {
            pos.col = MAXCOL;
        }

        for i in 0..NMARKS {
            if curbuf_mut().b_namedm[i].mark.lnum > 0 {
                if dir == FORWARD {
                    if (result.is_null() || lt(curbuf_mut().b_namedm[i].mark, (*result).mark))
                        && lt(pos, curbuf_mut().b_namedm[i].mark)
                    {
                        result = &raw mut (*g_st().curbuf).b_namedm[i];
                    }
                } else if (result.is_null() || lt((*result).mark, curbuf_mut().b_namedm[i].mark))
                    && lt(curbuf_mut().b_namedm[i].mark, pos)
                {
                    result = &raw mut (*g_st().curbuf).b_namedm[i];
                }
            }
        }

        result
    }
}

/// For an extended filemark: set the `fnum` from the `fname` (`fname2fnum`).
fn fname2fnum(fm: &mut xfmark_T) {
    unsafe {
        let Some(ref fname_cstr) = fm.fname else {
            return;
        };
        let fname = fname_cstr.as_ptr();

        if crate::memory::cstr_first_byte(fname) == b'~' && vim_ispathsep_nocolon(c_int::from(crate::memory::cstr_byte_at(fname, 1))) {
            let len = expand_env(
                c"~/",
                g_st().NameBuff.as_mut_ptr(),
                MAXPATHL as c_int,
            );
            xstrlcpy(
                (g_st().NameBuff.as_mut_ptr()).add(len),
                fname.add(2),
                MAXPATHL - len,
            );
        } else {
            xstrlcpy(g_st().NameBuff.as_mut_ptr(), fname, MAXPATHL);
        }

        os_dirname(g_st().IObuff.as_mut_ptr(), IOSIZE);
        let p = path_shorten_fname(g_st().NameBuff.as_mut_ptr(), g_st().IObuff.as_mut_ptr());

        let _ = buflist_new(g_st().NameBuff.as_mut_ptr(), p, 1, 0);
    }
}

/// Check all file marks for a name that matches the file name in `buf` (`fmarks_check_names`).
pub fn fmarks_check_names(buf: &mut buf_T) {
    unsafe {
        let name = buf.b_ffname;
        if name.is_null() {
            return;
        }

        for i in 0..NGLOBALMARKS {
            fmarks_check_one(&mut *(&raw mut g_st().namedfm).cast::<xfmark_T>().add(i), name, buf);
        }

        for wp in win_iter_mut(first_win_in_tab(curtab_mut())) {
            for i in 0..(wp.w_jumplistlen as usize) {
                fmarks_check_one(&mut wp.w_jumplist[i], name, buf);
            }
        }
    }
}

fn fmarks_check_one(fm: &mut xfmark_T, name: *mut c_char, buf: &mut buf_T) {
    {
        if fm.fmark.fnum == 0
            && fm
                .fname
                .as_ref()
                .is_some_and(|f| path_equal(name, f.as_ptr(), kPathCmpLiteral))
        {
            fm.fmark.fnum = buf.b_fnum;
            fm.fname = None;
        }
    }
}

/// Check the position in `fm` is valid (`mark_check`).
pub fn mark_check(mut fm: Option<&mut fmark_T>, errormsg: *mut *const c_char) -> bool {
    unsafe {
        if fm.is_none() {
            *errormsg = gettext(e_umark);
            return false;
        } else if fm.as_deref_mut().unwrap().mark.lnum <= 0 {
            if fm.as_deref_mut().unwrap().mark.lnum == 0 {
                *errormsg = gettext(e_marknotset);
            }
            return false;
        }
        if fm.as_deref_mut().unwrap().fnum == curbuf_mut().b_fnum && !mark_check_line_bounds(Some(crate::globals::curbuf_mut()),  fm.unwrap(), errormsg) {
            return false;
        }
        true
    }
}

/// Check if a mark line number is greater than the buffer line count (`mark_check_line_bounds`).
pub fn mark_check_line_bounds(
    buf: Option<&buf_T>,
    fm: &mut fmark_T,
    errormsg: *mut *const c_char,
) -> bool {
    unsafe {
        if let Some(buf) = buf && fm.mark.lnum > buf.b_ml.ml_line_count {
            *errormsg = gettext(e_markinval);
            return false;
        }
        true
    }
}

/// Clear all marks and change list in the given buffer (`clrallmarks`).
pub fn clrallmarks(buf: &mut buf_T, timestamp: Timestamp) {
    for i in 0..NMARKS {
            clear_fmark(&mut buf.b_namedm[i], timestamp);
        }
        clear_fmark(&mut buf.b_last_cursor, timestamp);
        buf.b_last_cursor.mark.lnum = 1;
        clear_fmark(&mut buf.b_last_insert, timestamp);
        clear_fmark(&mut buf.b_last_change, timestamp);
        buf.b_op_start.lnum = 0;
        buf.b_op_end.lnum = 0;
        for i in 0..(buf.b_changelistlen as usize) {
            clear_fmark(&mut buf.b_changelist[i], timestamp);
        }
        buf.b_changelistlen = 0;
}

/// Get name of file from a filemark (`fm_getname`).
pub fn fm_getname(fmark: &mut fmark_T, lead_len: c_int) -> Option<std::ffi::CString> {
     {
        if fmark.fnum == curbuf_mut().b_fnum {
            return Some(mark_line(&mut fmark.mark, lead_len));
        }
        buflist_nr2name(fmark.fnum, 0, 1)
    }
}

/// Return the line at mark `mp`, truncated to fit in window (`mark_line`).
fn mark_line(mp: &mut pos_T, lead_len: c_int) -> std::ffi::CString {
    unsafe {
        if mp.lnum == 0 || mp.lnum > curbuf_mut().b_ml.ml_line_count {
            return c"-invalid-".to_owned();
        }
        debug_assert!(g_st().Columns >= 0);
        let line = skipwhite(ml_get(mp.lnum));
        let max_bytes = crate::memory::cstr_len(line)
            .min((g_st().Columns as usize) * 5);

        let mut len: c_int = 0;
        let mut p = line;
        let end = line.add(max_bytes);
        while p < end && !crate::memory::cstr_is_empty(p) {
            len += ptr2cells(p);
            if len >= g_st().Columns - lead_len {
                break;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        let keep = (p.offset_from(line) as usize).min(max_bytes);
        std::ffi::CString::new(crate::memory::cstr_slice(line, keep))
            .unwrap_or_default()
    }
}

/// `:marks {arg}` (`ex_marks`).
pub fn ex_marks(eap: &mut exarg_T) {
    nlua_call_excmd(
            c"vim._core.marks",
            c"ex_marks",
            eap,
            &g_st().cmdmod,
            None,
        );
}

/// `:delmarks[!] [marks]` (`ex_delmarks`).
pub fn ex_delmarks(eap: &mut exarg_T) {
    unsafe {
        let pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };

        if crate::memory::cstr_is_empty(eap.arg) && eap.forceit != 0 {
            for i in 0..NMARKS {
                if curbuf_mut().b_namedm[i].mark.lnum != 0 {
                    do_markset_autocmd((i as u8 + b'a') as c_char, &pos, curbuf_mut());
                }
            }
            if curbuf_mut().b_last_cursor.mark.lnum != 0 {
                do_markset_autocmd(b'"' as c_char, &pos, curbuf_mut());
            }
            if curbuf_mut().b_last_insert.mark.lnum != 0 {
                do_markset_autocmd(b'^' as c_char, &pos, curbuf_mut());
            }
            if curbuf_mut().b_last_change.mark.lnum != 0 {
                do_markset_autocmd(b'.' as c_char, &pos, curbuf_mut());
            }
            if curbuf_mut().b_op_start.lnum != 0 {
                do_markset_autocmd(b'[' as c_char, &pos, curbuf_mut());
            }
            if curbuf_mut().b_op_end.lnum != 0 {
                do_markset_autocmd(b']' as c_char, &pos, curbuf_mut());
            }
            clrallmarks(curbuf_mut(), os_time());
        } else if eap.forceit != 0 {
            emsg(gettext(e_invarg));
        } else if crate::memory::cstr_is_empty(eap.arg) {
            emsg(gettext(e_argreq));
        } else {
            let timestamp = os_time();
            let mut p = eap.arg;
            while !crate::memory::cstr_is_empty(p) {
                let ch = c_int::from(crate::memory::cstr_first_byte(p));
                let lower = ascii_islower(ch);
                let digit = ascii_isdigit(ch);
                if lower || digit || ascii_isupper(ch) {
                    let from: c_int;
                    let to: c_int;
                    if crate::memory::cstr_byte_at(p, 1) == b'-' {
                        from = ch;
                        to = c_int::from(crate::memory::cstr_byte_at(p, 2));
                        let valid_to = if lower {
                            ascii_islower(to)
                        } else if digit {
                            ascii_isdigit(to)
                        } else {
                            ascii_isupper(to)
                        };
                        if !valid_to || to < from {
                            semsg!(gettext(e_invarg2), p);
                            return;
                        }
                        p = p.add(2);
                    } else {
                        from = ch;
                        to = ch;
                    }

                    for i in from..=to {
                        if lower {
                            let idx = (i - b'a' as c_int) as usize;
                            if curbuf_mut().b_namedm[idx].mark.lnum != 0 {
                                do_markset_autocmd(i as c_char, &pos, curbuf_mut());
                            }
                            curbuf_mut().b_namedm[idx].mark.lnum = 0;
                            curbuf_mut().b_namedm[idx].timestamp = timestamp;
                        } else {
                            let n = if digit {
                                (i - b'0' as c_int) as usize + NMARKS
                            } else {
                                (i - b'A' as c_int) as usize
                            };
                            if g_st().namedfm[n].fmark.mark.lnum != 0 {
                                let mut buf = buflist_findnr(g_st().namedfm[n].fmark.fnum);
                                if buf.is_null() {
                                    buf = g_st().curbuf;
                                }
                                do_markset_autocmd(i as c_char, &pos,  &mut *buf);
                            }
                            g_st().namedfm[n].fmark.mark.lnum = 0;
                            g_st().namedfm[n].fmark.fnum = 0;
                            g_st().namedfm[n].fmark.timestamp = timestamp;
                            g_st().namedfm[n].fname = None;
                        }
                    }
                } else {
                    match crate::memory::cstr_first_byte(p) {
                        b'"' => {
                            if curbuf_mut().b_last_cursor.mark.lnum != 0 {
                                do_markset_autocmd(*p, &pos, curbuf_mut());
                            }
                            clear_fmark(&mut curbuf_mut().b_last_cursor, timestamp);
                        }
                        b'^' => {
                            if curbuf_mut().b_last_insert.mark.lnum != 0 {
                                do_markset_autocmd(*p, &pos, curbuf_mut());
                            }
                            clear_fmark(&mut curbuf_mut().b_last_insert, timestamp);
                        }
                        b':' => {}
                        b'.' => {
                            if curbuf_mut().b_last_change.mark.lnum != 0 {
                                do_markset_autocmd(*p, &pos, curbuf_mut());
                            }
                            clear_fmark(&mut curbuf_mut().b_last_change, timestamp);
                        }
                        b'[' => {
                            if curbuf_mut().b_op_start.lnum != 0 {
                                do_markset_autocmd(*p, &pos, curbuf_mut());
                            }
                            curbuf_mut().b_op_start.lnum = 0;
                        }
                        b']' => {
                            if curbuf_mut().b_op_end.lnum != 0 {
                                do_markset_autocmd(*p, &pos, curbuf_mut());
                            }
                            curbuf_mut().b_op_end.lnum = 0;
                        }
                        b'<' => {
                            if curbuf_mut().b_visual.vi_start.lnum != 0 {
                                do_markset_autocmd(*p, &pos, curbuf_mut());
                            }
                            curbuf_mut().b_visual.vi_start.lnum = 0;
                        }
                        b'>' => {
                            if curbuf_mut().b_visual.vi_end.lnum != 0 {
                                do_markset_autocmd(*p, &pos, curbuf_mut());
                            }
                            curbuf_mut().b_visual.vi_end.lnum = 0;
                        }
                        b' ' => {}
                        _ => {
                            semsg!(gettext(e_invarg2), p);
                            return;
                        }
                    }
                }
                p = p.add(1);
            }
        }
    }
}

/// Print the jumplist (`ex_jumps`).
pub fn ex_jumps(_eap: &mut exarg_T) {
    unsafe {
        cleanup_jumplist(curwin_mut(), true);
        msg_ext_set_kind(c"list_cmd");
        msg_puts_title(gettext(c"\n jump line  col file/text"));
        let mut i: c_int = 0;
        while i < curwin_mut().w_jumplistlen && !g_st().got_int {
            let idx = i as usize;
            if curwin_mut().w_jumplist[idx].fmark.mark.lnum != 0 {
                let mut name = fm_getname(&mut curwin_mut().w_jumplist[idx].fmark, 16);

                if name.is_none() && i == curwin_mut().w_jumplistidx {
                    name = Some(c"-invalid-".to_owned());
                }
                let Some(name_cstr) = name else {
                    i += 1;
                    continue;
                };
                if message_filtered(name_cstr.as_ptr()) {
                    i += 1;
                    continue;
                }

                msg_putchar(b'\n' as c_int);
                if g_st().got_int {
                    break;
                }
                let indicator = if i == curwin_mut().w_jumplistidx {
                    '>'
                } else {
                    ' '
                };
                let dist = (i - curwin_mut().w_jumplistidx).abs();
                let lnum = curwin_mut().w_jumplist[idx].fmark.mark.lnum;
                let col = curwin_mut().w_jumplist[idx].fmark.mark.col;
                let s = format!("{indicator} {dist:2} {lnum:5} {col:4} ");
                let bytes = s.as_bytes();
                let copy_len = bytes.len().min(IOSIZE - 1);
                ptr::copy_nonoverlapping(
                    bytes.as_ptr().cast::<c_char>(),
                    g_st().IObuff.as_mut_ptr(),
                    copy_len,
                );
                g_st().IObuff[copy_len] = NUL;

                msg_outtrans(g_st().IObuff.as_ptr(), 0, false);
                msg_outtrans(
                    name_cstr.as_ptr(),
                    if curwin_mut().w_jumplist[idx].fmark.fnum == curbuf_mut().b_fnum {
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
        if curwin_mut().w_jumplistidx == curwin_mut().w_jumplistlen {
            msg_puts(c"\n>");
        }
    }
}

pub fn ex_clearjumps(_eap: &mut exarg_T) {
     {
        free_jumplist(curwin_mut());
        curwin_mut().w_jumplistlen = 0;
        curwin_mut().w_jumplistidx = 0;
    }
}

/// Print the changelist (`ex_changes`).
pub fn ex_changes(_eap: &mut exarg_T) {
    unsafe {
        msg_ext_set_kind(c"list_cmd");
        msg_puts_title(gettext(c"\nchange line  col text"));

        let mut i: c_int = 0;
        while i < curbuf_mut().b_changelistlen && !g_st().got_int {
            let idx = i as usize;
            if curbuf_mut().b_changelist[idx].mark.lnum != 0 {
                msg_putchar(b'\n' as c_int);
                if g_st().got_int {
                    break;
                }
                let indicator = if i == curwin_mut().w_changelistidx {
                    '>'
                } else {
                    ' '
                };
                let dist = (i - curwin_mut().w_changelistidx).abs();
                let lnum = curbuf_mut().b_changelist[idx].mark.lnum;
                let col = curbuf_mut().b_changelist[idx].mark.col;
                let s = format!("{indicator} {dist:3} {lnum:5} {col:4} ");
                let bytes = s.as_bytes();
                let copy_len = bytes.len().min(IOSIZE - 1);
                ptr::copy_nonoverlapping(
                    bytes.as_ptr().cast::<c_char>(),
                    g_st().IObuff.as_mut_ptr(),
                    copy_len,
                );
                g_st().IObuff[copy_len] = NUL;

                msg_outtrans(g_st().IObuff.as_ptr(), 0, false);
                let name = mark_line(&mut curbuf_mut().b_changelist[idx].mark, 17);
                msg_outtrans(name.as_ptr(), HLF_D, false);
                os_breakcheck();
            }
            i += 1;
        }
        if curwin_mut().w_changelistidx == curbuf_mut().b_changelistlen {
            msg_puts(c"\n>");
        }
    }
}

#[inline]
fn one_adjust(
    lp: &mut linenr_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
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
    lp: &mut linenr_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
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
    posp: &mut pos_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
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
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
    op: ExtmarkOp,
) {
     {
        mark_adjust_buf(
            curbuf_mut(),
            line1,
            line2,
            amount,
            amount_after,
            true,
            kMarkAdjustNormal,
            op,
        );
    }
}

/// Adjust marks without adjusting folds (`mark_adjust_nofold`).
pub fn mark_adjust_nofold(
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
    op: ExtmarkOp,
) {
     {
        mark_adjust_buf(
            curbuf_mut(),
            line1,
            line2,
            amount,
            amount_after,
            false,
            kMarkAdjustNormal,
            op,
        );
    }
}

pub fn mark_adjust_buf(
    buf: &mut buf_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
    adjust_folds: bool,
    mode: MarkAdjustMode,
    op: ExtmarkOp,
) {
    unsafe {
        let fnum = buf.b_fnum;
        const INITPOS: pos_T = pos_T {
            lnum: 1,
            col: 0,
            coladd: 0,
        };

        if line2 < line1 && amount_after == 0 {
            return;
        }

        let by_api = mode == kMarkAdjustApi;
        let by_term = mode == kMarkAdjustTerm;

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            let fm_base = (&raw mut g_st().namedfm).cast::<xfmark_T>();
            for i in 0..NMARKS {
                one_adjust(
                    &mut buf.b_namedm[i].mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
                let fm = fm_base.add(i);
                if (*fm).fmark.fnum == fnum {
                    one_adjust_nodel(
                        &mut (*fm).fmark.mark.lnum,
                        line1,
                        line2,
                        amount,
                        amount_after,
                    );
                }
            }
            for i in NMARKS..NGLOBALMARKS {
                let fm = fm_base.add(i);
                if (*fm).fmark.fnum == fnum {
                    one_adjust_nodel(
                        &mut (*fm).fmark.mark.lnum,
                        line1,
                        line2,
                        amount,
                        amount_after,
                    );
                }
            }

            one_adjust(
                &mut buf.b_last_insert.mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
            one_adjust(
                &mut buf.b_last_change.mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );

            if !equalpos(buf.b_last_cursor.mark, INITPOS)
                && (!by_term || buf.b_last_cursor.mark.lnum < buf.b_ml.ml_line_count)
            {
                one_adjust(
                    &mut buf.b_last_cursor.mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }

            if bt_prompt(Some(&*buf)) {
                one_adjust_nodel(
                    &mut buf.b_prompt_start.mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }

            for i in 0..(buf.b_changelistlen as usize) {
                one_adjust_nodel(
                    &mut buf.b_changelist[i].mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }

            one_adjust_nodel(
                &mut buf.b_visual.vi_start.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
            one_adjust_nodel(
                &mut buf.b_visual.vi_end.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );

            if !qf_mark_adjust(buf, None, line1, line2, amount, amount_after) {
                buf.b_has_qf_entry &= !BUF_HAS_QF_ENTRY;
            }
            let mut found_one = false;
            for tab in for_all_tabpages_ptr() {
                for win in win_iter_ptr(first_win_in_tab( &mut *tab)) {
                    found_one |= qf_mark_adjust(buf, win.as_ref(), line1, line2, amount, amount_after);
                }
            }
            if !found_one {
                buf.b_has_qf_entry &= !BUF_HAS_LL_ENTRY;
            }
        }

        if op != kExtmarkNOOP {
            extmark_adjust( &mut *buf, line1, line2, amount, amount_after, op);
        }

        if curwin_mut().w_buffer == buf {
            one_adjust(
                &mut curwin_mut().w_pcmark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
            one_adjust(
                &mut curwin_mut().w_prev_pcmark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );

            if g_st().saved_cursor.lnum != 0 {
                one_adjust_nodel(
                    &mut g_st().saved_cursor.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }
        }

        for tab in for_all_tabpages_ptr() {
            for win in win_iter_ptr(first_win_in_tab( &mut *tab)) {
                if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                    for i in 0..((*win).w_jumplistlen as usize) {
                        if (*win).w_jumplist[i].fmark.fnum == fnum {
                            one_adjust_nodel(
                                &mut (*win).w_jumplist[i].fmark.mark.lnum,
                                line1,
                                line2,
                                amount,
                                amount_after,
                            );
                        }
                    }
                }

                if (*win).w_buffer == buf {
                    if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                        for i in 0..((*win).w_tagstacklen as usize) {
                            if (*win).w_tagstack[i].fmark.fnum == fnum {
                                one_adjust_nodel(
                                    &mut (*win).w_tagstack[i].fmark.mark.lnum,
                                    line1,
                                    line2,
                                    amount,
                                    amount_after,
                                );
                            }
                        }
                    }

                    if (*win).w_old_cursor_lnum != 0 {
                        one_adjust_nodel(
                            &mut (*win).w_old_cursor_lnum,
                            line1,
                            line2,
                            amount,
                            amount_after,
                        );
                        one_adjust_nodel(
                            &mut (*win).w_old_visual_lnum,
                            line1,
                            line2,
                            amount,
                            amount_after,
                        );
                    }

                    if by_api
                        || (if by_term {
                            (*win).w_cursor.lnum < buf.b_ml.ml_line_count
                        } else {
                            win != curwin_ptr()
                        })
                    {
                        if (*win).w_topline >= line1 && (*win).w_topline <= line2 {
                            if amount == MAXLNUM {
                                if !(by_api && amount_after > line1 - line2 - 1) {
                                    (*win).w_topline = (line1 - 1).max(1);
                                }
                            } else if (*win).w_topline > line1 {
                                (*win).w_topline += amount;
                            }
                            (*win).w_topfill = 0;
                        } else if amount_after != 0
                            && (*win).w_topline
                                > line2 + if by_api && line2 < line1 { 1 } else { 0 }
                        {
                            (*win).w_topline += amount_after;
                            (*win).w_topfill = 0;
                        }
                    }
                    if !by_api
                        && (if by_term {
                            (*win).w_cursor.lnum < buf.b_ml.ml_line_count
                        } else {
                            win != curwin_ptr()
                        })
                    {
                        one_adjust_cursor(
                            &mut (*win).w_cursor,
                            line1,
                            line2,
                            amount,
                            amount_after,
                        );
                    }

                    if adjust_folds {
                        foldMarkAdjust(&mut *win, line1, line2, amount, amount_after);
                    }
                }
            }
        }

        diff_mark_adjust(buf, line1, line2, amount, amount_after);

        for wip in &mut buf.b_wininfo {
            if !by_term || wip.wi_mark.mark.lnum < buf.b_ml.ml_line_count {
                one_adjust_cursor(
                    &mut wip.wi_mark.mark,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }
        }
    }
}

#[inline]
fn col_adjust(
    posp: &mut pos_T,
    lnum: linenr_T,
    mincol: colnr_T,
    lnum_amount: linenr_T,
    col_amount: colnr_T,
    spaces_removed: c_int,
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
    lnum: linenr_T,
    mincol: colnr_T,
    lnum_amount: linenr_T,
    col_amount: colnr_T,
    spaces_removed: c_int,
) {
    unsafe {
        let fnum = curbuf_mut().b_fnum;

        if (col_amount == 0 && lnum_amount == 0) || (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            return;
        }

        let fm_base = (&raw mut g_st().namedfm).cast::<xfmark_T>();
        for i in 0..NMARKS {
            col_adjust(
                &mut curbuf_mut().b_namedm[i].mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
            let fm = fm_base.add(i);
            if (*fm).fmark.fnum == fnum {
                col_adjust(
                    &mut (*fm).fmark.mark,
                    lnum,
                    mincol,
                    lnum_amount,
                    col_amount,
                    spaces_removed,
                );
            }
        }
        for i in NMARKS..NGLOBALMARKS {
            let fm = fm_base.add(i);
            if (*fm).fmark.fnum == fnum {
                col_adjust(
                    &mut (*fm).fmark.mark,
                    lnum,
                    mincol,
                    lnum_amount,
                    col_amount,
                    spaces_removed,
                );
            }
        }

        col_adjust(
            &mut curbuf_mut().b_last_insert.mark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &mut curbuf_mut().b_last_change.mark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );

        if bt_prompt(Some(crate::globals::curbuf_mut())) {
            col_adjust(
                &mut curbuf_mut().b_prompt_start.mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
        }

        for i in 0..(curbuf_mut().b_changelistlen as usize) {
            col_adjust(
                &mut curbuf_mut().b_changelist[i].mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
        }

        col_adjust(
            &mut curbuf_mut().b_visual.vi_start,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &mut curbuf_mut().b_visual.vi_end,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );

        col_adjust(
            &mut curwin_mut().w_pcmark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &mut curwin_mut().w_prev_pcmark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &mut g_st().saved_cursor,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );

        for win in win_iter_ptr(first_win_in_tab(curtab_mut())) {
            for i in 0..((*win).w_jumplistlen as usize) {
                if (*win).w_jumplist[i].fmark.fnum == fnum {
                    col_adjust(
                        &mut (*win).w_jumplist[i].fmark.mark,
                        lnum,
                        mincol,
                        lnum_amount,
                        col_amount,
                        spaces_removed,
                    );
                }
            }

            if (*win).w_buffer == g_st().curbuf {
                for i in 0..((*win).w_tagstacklen as usize) {
                    if (*win).w_tagstack[i].fmark.fnum == fnum {
                        col_adjust(
                            &mut (*win).w_tagstack[i].fmark.mark,
                            lnum,
                            mincol,
                            lnum_amount,
                            col_amount,
                            spaces_removed,
                        );
                    }
                }

                if win != curwin_ptr() {
                    col_adjust(
                        &mut (*win).w_cursor,
                        lnum,
                        mincol,
                        lnum_amount,
                        col_amount,
                        spaces_removed,
                    );
                }
            }
        }
    }
}

/// Remove duplicate marks in the jumplist (`cleanup_jumplist`).
pub fn cleanup_jumplist(wp: &mut win_T, loadfiles: bool) {
     {
        if loadfiles {
            for i in 0..(wp.w_jumplistlen as usize) {
                if wp.w_jumplist[i].fmark.fnum == 0 && wp.w_jumplist[i].fmark.mark.lnum != 0 {
                    fname2fnum(&mut wp.w_jumplist[i]);
                }
            }
        }

        let mut to: c_int = 0;
        for from in 0..wp.w_jumplistlen {
            let from_u = from as usize;
            if wp.w_jumplistidx == from {
                wp.w_jumplistidx = to;
            }
            let mut i = from + 1;
            while i < wp.w_jumplistlen {
                let i_u = i as usize;
                if wp.w_jumplist[i_u].fmark.fnum == wp.w_jumplist[from_u].fmark.fnum
                    && wp.w_jumplist[from_u].fmark.fnum != 0
                    && wp.w_jumplist[i_u].fmark.mark.lnum
                        == wp.w_jumplist[from_u].fmark.mark.lnum
                {
                    break;
                }
                i += 1;
            }

            let mustfree = if i >= wp.w_jumplistlen {
                false
            } else if i > from + 1 {
                (opt_vars().jop_flags & kOptJopFlagStack) == 0
            } else {
                true
            };

            if mustfree {
                wp.w_jumplist[from_u].fname = None;
            } else {
                if to != from {
                    wp.w_jumplist[to as usize] = wp.w_jumplist[from_u].clone();
                }
                to += 1;
            }
        }
        if wp.w_jumplistidx == wp.w_jumplistlen {
            wp.w_jumplistidx = to;
        }
        wp.w_jumplistlen = to;

        if loadfiles && wp.w_jumplistlen != 0 && wp.w_jumplistidx == wp.w_jumplistlen {
            let last_idx = (wp.w_jumplistlen - 1) as usize;
            if wp.w_jumplist[last_idx].fmark.fnum == curbuf_mut().b_fnum
                && wp.w_jumplist[last_idx].fmark.mark.lnum == wp.w_cursor.lnum
            {
                wp.w_jumplist[last_idx].fname = None;
                wp.w_jumplistlen -= 1;
                wp.w_jumplistidx -= 1;
            }
        }
    }
}

/// Copy the jumplist from window `from` to window `to` (`copy_jumplist`).
pub fn copy_jumplist(from: &mut win_T, to: &mut win_T) {
    for i in 0..(from.w_jumplistlen as usize) {
            to.w_jumplist[i] = from.w_jumplist[i].clone();
        }
        to.w_jumplistlen = from.w_jumplistlen;
        to.w_jumplistidx = from.w_jumplistidx;
}

/// Iterate over jumplist items (`mark_jumplist_iter`).
pub fn mark_jumplist_iter(iter: Option<usize>, win: &win_T, fm: &mut xfmark_T) -> Option<usize> {
    if iter.is_none() && win.w_jumplistlen == 0 {
        *fm = INIT_XFMARK;
        return None;
    }
    let idx = iter.unwrap_or(0);
    *fm = win.w_jumplist[idx].clone();
    if idx + 1 >= win.w_jumplistlen as usize {
        None
    } else {
        Some(idx + 1)
    }
}

/// Iterate over global marks (`mark_global_iter`).
pub fn mark_global_iter(
    iter: Option<usize>,
    name: &mut c_char,
    fm: &mut xfmark_T,
) -> Option<usize> {
    *name = NUL;
        let marks = &g_st().namedfm;
        let mut idx = iter.unwrap_or(0);
        while idx < NGLOBALMARKS && marks[idx].fmark.mark.lnum == 0 {
            idx += 1;
        }
        if idx >= NGLOBALMARKS || marks[idx].fmark.mark.lnum == 0 {
            return None;
        }
        *name = if idx < NMARKS {
            (b'A' + idx as u8) as c_char
        } else {
            (b'0' + (idx - NMARKS) as u8) as c_char
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
fn next_buffer_mark<'a>(buf: &'a buf_T, mark_name: &mut c_char) -> Option<&'a fmark_T> {
    match *mark_name as u8 {
        0 => {
            *mark_name = b'"' as c_char;
            Some(&buf.b_last_cursor)
        }
        b'"' => {
            *mark_name = b'^' as c_char;
            Some(&buf.b_last_insert)
        }
        b'^' => {
            *mark_name = b'.' as c_char;
            Some(&buf.b_last_change)
        }
        b'.' => {
            *mark_name = b'a' as c_char;
            Some(&buf.b_namedm[0])
        }
        b'z' => None,
        ch => {
            *mark_name = (ch + 1) as c_char;
            Some(&buf.b_namedm[(*mark_name as u8 - b'a') as usize])
        }
    }
}

/// Iterate over buffer marks (`mark_buffer_iter`).
pub fn mark_buffer_iter(
    iter: Option<c_char>,
    buf: &buf_T,
    name: &mut c_char,
    fm: &mut fmark_T,
) -> Option<c_char> {
    *name = NUL;
    let mut mark_name = iter.unwrap_or(NUL);
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
pub fn mark_set_global(name: c_char, fm: xfmark_T, update: bool) -> bool {
    unsafe {
        let idx = mark_global_index(name);
        if idx == -1 {
            return false;
        }
        let fm_tgt = &raw mut g_st().namedfm[idx as usize];
        if update && fm.fmark.timestamp <= (*fm_tgt).fmark.timestamp {
            return false;
        }
        if (*fm_tgt).fmark.mark.lnum != 0 {
            free_xfmark(&mut *fm_tgt);
        }
        *fm_tgt = fm;
        true
    }
}

/// Set local mark (`mark_set_local`).
pub fn mark_set_local(name: c_char, buf: &mut buf_T, fm: fmark_T, update: bool) -> bool {
    unsafe {
        let ch = c_int::from(name as u8);
        let fm_tgt = if ascii_islower(ch) {
            &raw mut buf.b_namedm[(ch - b'a' as c_int) as usize]
        } else if name == b'"' as c_char {
            &raw mut buf.b_last_cursor
        } else if name == b'^' as c_char {
            &raw mut buf.b_last_insert
        } else if name == b':' as c_char {
            &raw mut buf.b_prompt_start
        } else if name == b'.' as c_char {
            &raw mut buf.b_last_change
        } else {
            return false;
        };
        if update && fm.timestamp <= (*fm_tgt).timestamp {
            return false;
        }
        *fm_tgt = fm;
        true
    }
}

/// Free items in the jumplist of window `wp` (`free_jumplist`).
pub fn free_jumplist(wp: &mut win_T) {
    for i in 0..(wp.w_jumplistlen as usize) {
            free_xfmark(&mut wp.w_jumplist[i]);
        }
        wp.w_jumplistlen = 0;
}

pub fn set_last_cursor(win: &mut win_T) {
     {
        if !win.w_buffer.is_null() {
            reset_fmark(
                &mut win.buf_mut().b_last_cursor,
                win.w_cursor,
                0,
                INIT_FMARKV,
            );
        }
    }
}

pub fn free_all_marks() {
    unsafe {
        let fm_base = (&raw mut g_st().namedfm).cast::<xfmark_T>();
        for i in 0..NGLOBALMARKS {
            let fm = fm_base.add(i);
            if (*fm).fmark.mark.lnum != 0 {
                free_xfmark(&mut *fm);
            }
            *fm = INIT_XFMARK;
        }
    }
}

/// Adjust position to point to the first byte of a multi-byte character (`mark_mb_adjustpos`).
pub fn mark_mb_adjustpos(buf: *mut buf_T, lp: &mut pos_T) {
    unsafe {
        if lp.col > 0 || lp.coladd > 1 {
            let p = ml_get_buf(&mut *buf, lp.lnum);
            if crate::memory::cstr_is_empty(p) || ml_get_buf_len(&mut *buf, lp.lnum) < lp.col {
                lp.col = 0;
            } else {
                lp.col -= utf_head_off(p, p.add(lp.col as usize));
            }
            let p_at = p.add(lp.col as usize);
            if lp.coladd == 1
                && *p_at != TAB
                && vim_isprintc(utf_ptr2char(p_at))
                && ptr2cells(p_at) > 1
            {
                lp.coladd = 0;
            }
        }
    }
}

/// Add information about mark `mname` to list `l` (`add_mark`).
fn add_mark<S: crate::strings::IntoCCharPtr>(
    l: &mut list_T,
    mname: S,
    pos: &pos_T,
    bufnr: c_int,
    fname: *const c_char,
) -> bool {
    let mname = mname.into_c_char_ptr();
    unsafe {
        if pos.lnum <= 0 {
            return true;
        }

        let d = tv_dict_alloc();
        tv_list_append_dict(l, d);

        let lpos = tv_list_alloc(kListLenMayKnow);
        tv_list_append_number( &mut *lpos, i64::from(bufnr));
        tv_list_append_number( &mut *lpos, i64::from(pos.lnum));
        tv_list_append_number(
             &mut *lpos,
            i64::from(if pos.col < MAXCOL {
                pos.col + 1
            } else {
                MAXCOL
            }),
        );
        tv_list_append_number( &mut *lpos, i64::from(pos.coladd));

        if !tv_dict_add_str_len(&mut *d, c"mark", mname, crate::memory::cstr_len(mname) as c_int)
            || !tv_dict_add_list(&mut *d, c"pos", lpos)
            || (!fname.is_null() && !tv_dict_add_str(&mut *d, c"file", fname))
        {
            return false;
        }

        true
    }
}

/// Get information about marks local to a buffer (`get_buf_local_marks`).
pub fn get_buf_local_marks(buf: &buf_T, l: &mut list_T) {
     {
        let mut mname: [c_char; 3] = [b'\'' as c_char, b' ' as c_char, 0];

        for i in 0..NMARKS {
            mname[1] = (b'a' + i as u8) as c_char;
            add_mark(
                l,
                mname.as_ptr(),
                &buf.b_namedm[i].mark,
                buf.b_fnum,
                ptr::null(),
            );
        }

        add_mark(
            l,
            c"''",
            &curwin_mut().w_pcmark,
            curbuf_mut().b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'\"",
            &buf.b_last_cursor.mark,
            buf.b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'[",
            &buf.b_op_start,
            buf.b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"']",
            &buf.b_op_end,
            buf.b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'^",
            &buf.b_last_insert.mark,
            buf.b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'.",
            &buf.b_last_change.mark,
            buf.b_fnum,
            ptr::null(),
        );
        if bt_prompt(Some(buf)) {
            add_mark(
                l,
                c"':",
                &buf.b_prompt_start.mark,
                buf.b_fnum,
                ptr::null(),
            );
        }
        add_mark(
            l,
            c"'<",
            &buf.b_visual.vi_start,
            buf.b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'>",
            &buf.b_visual.vi_end,
            buf.b_fnum,
            ptr::null(),
        );
    }
}

/// Get a global mark (`get_raw_global_mark`).
#[cfg(test)]
pub fn get_raw_global_mark(name: c_char) -> xfmark_T {
    g_st().namedfm[mark_global_index(name) as usize].clone()
}

/// Get information about global marks (`get_global_marks`).
pub fn get_global_marks(l: &mut list_T) {
    unsafe {
        let mut mname: [c_char; 3] = [b'\'' as c_char, b' ' as c_char, 0];

        let fm_base = (&raw const g_st().namedfm).cast::<xfmark_T>();
        for i in 0..(NMARKS + EXTRA_MARKS) {
            let fm = &*fm_base.add(i);
            let owned_name = if fm.fmark.fnum != 0 {
                buflist_nr2name(fm.fmark.fnum, 1, 1)
            } else {
                None
            };
            let name = if fm.fmark.fnum != 0 {
                owned_name
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            } else {
                fm.fname_ptr()
            };
            if !name.is_null() {
                mname[1] = if i >= NMARKS {
                    (b'0' + (i - NMARKS) as u8) as c_char
                } else {
                    (b'A' + i as u8) as c_char
                };

                add_mark(
                    l,
                    mname.as_ptr(),
                    &fm.fmark.mark,
                    fm.fmark.fnum,
                    name,
                );
            }
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
        assert_eq!(mark_global_index(b'A' as c_char), 0);
            assert_eq!(mark_global_index(b'Z' as c_char), 25);
            assert_eq!(mark_global_index(b'0' as c_char), 26);
            assert_eq!(mark_global_index(b'9' as c_char), 35);
            assert_eq!(mark_global_index(b'a' as c_char), -1);

            let mut xfm = INIT_XFMARK;
            xfm.fmark.mark = pos_T {
                lnum: 42,
                col: 7,
                coladd: 0,
            };
            xfm.fmark.fnum = 1;
            xfm.fmark.timestamp = 100;
            assert!(mark_set_global(b'A' as c_char, xfm, false));
            let got = get_raw_global_mark(b'A' as c_char);
            assert_eq!(got.fmark.mark.lnum, 42);
            assert_eq!(got.fmark.mark.col, 7);
            free_all_marks();
    }
}
