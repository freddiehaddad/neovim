//! Functions for setting marks and jumping to them (`src/nvim/mark.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

use crate::api::command::{CMOD_KEEPJUMPS, CMOD_LOCKMARKS, exarg_T};
use crate::api::private::validate::{Dict, KeyValuePair, Object, ObjectData, ObjectType};
use crate::api::tabpage::{win_T, winopt_T, xfmark_T};
use crate::autocmd::{AUGROUP_ALL, EVENT_MARKSET, aucmd_defer, has_event};
use crate::buffer::{
    GETF_SETMARK, MAXLNUM, NMARKS, bt_prompt, buflist_findnr, buflist_getfile, buflist_new,
    buflist_nr2name, first_win_in_tab, kMarkAdjustNormal,
};
use crate::charset::{ascii_isdigit, ascii_islower, ascii_isupper, skipwhite, vim_isprintc};
use crate::clipboard::list_T;
use crate::cursor::{check_cursor, colnr_T, equalpos, linenr_T, lt};
use crate::diff::diff_mark_adjust;
use crate::errors::*;
use crate::eval::deprecated::{buf_T, fmark_T, fmarkv_T, pos_T};
use crate::eval::typval::{
    tv_dict_add_list, tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_list_alloc,
    tv_list_append_dict, tv_list_append_number,
};
use crate::extmark::extmark_adjust;
use crate::fold::{foldMarkAdjust, hasFolding};
use crate::garray::garray_T;
use crate::globals::*;
use crate::insert::beginline;
use crate::lua::executor::nlua_call_excmd;
use crate::map_glyph_cache::NvimString;
use crate::mbyte::{ptr2cells, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::mcursor::mc_replaying;
use crate::memline::{ml_get, ml_get_buf, ml_get_buf_len};
use crate::memory::{xfree, xstrdup, xstrlcpy};
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
use crate::path::{kPathCmpLiteral, path_equal, path_shorten_fname, vim_ispathsep_nocolon};
use crate::plines::linetabsize_eol;
use crate::quickfix::qf_mark_adjust;
use crate::strings::{gettext, xstrnsave};
use crate::tag::tagstack_clear_entry;
use crate::textformat::oparg_T;
use crate::textobject::{findpar, findsent};
use crate::undo::u_update_named_mark;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

pub type MarkMoveRes = c_int;
pub type MarkMove = c_int;
pub type MarkGet = c_int;
pub type MarkAdjustMode = c_int;
use crate::api::buffer::{FORWARD, NUL, kExtmarkNOOP, kMarkAdjustApi, kMarkAllNoResolve};
use crate::api::options::IOSIZE;
use crate::api::vim::{BACKWARD, MAXPATHL};
use crate::change::TAB;
use crate::cmdexpand::HLF_D;
use crate::eval::kListLenMayKnow;
use crate::ex_cmds::{BL_FIX, BL_WHITE};
pub use crate::extmark::ExtmarkOp;
use crate::math::{FAIL, OK};
pub use crate::path::PathCmpFlags;
use crate::regexp::{MAXCOL, kMarkBufLocal};

#[repr(C)]
pub struct WinInfo {
    pub wi_win: *mut win_T,
    pub wi_mark: fmark_T,
    pub wi_optset: bool,
    pub wi_opt: winopt_T,
    pub wi_fold_manual: bool,
    pub wi_folds: garray_T,
    pub wi_changelistidx: c_int,
}

pub(crate) const EXTRA_MARKS: usize = 10;
const NGLOBALMARKS: usize = NMARKS + EXTRA_MARKS;
const NMARK_LOCAL_MAX: c_int = 126;
const JUMPLISTSIZE: c_int = 100;

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

const INIT_FMARKV: fmarkv_T = fmarkv_T {
    topline_offset: MAXLNUM,
    skipcol: 0,
};

const INIT_FMARK: fmark_T = fmark_T {
    mark: pos_T {
        lnum: 0,
        col: 0,
        coladd: 0,
    },
    fnum: 0,
    timestamp: 0,
    view: INIT_FMARKV,
    additional_data: ptr::null_mut(),
};

const INIT_XFMARK: xfmark_T = xfmark_T {
    fmark: INIT_FMARK,
    fname: ptr::null_mut(),
};

static mut POS_TO_MARK_FMS: fmark_T = INIT_FMARK;
static mut MARK_MOVE_TO_FM_COPY: fmark_T = INIT_FMARK;

#[inline]
const fn mark_global_index(name: c_char) -> isize {
    let u = name as u8;
    if u.is_ascii_uppercase() {
        (u - b'A') as isize
    } else if u.is_ascii_digit() {
        (NMARKS + (u - b'0') as usize) as isize
    } else {
        -1
    }
}

#[inline]
unsafe fn set_fmark(fmarkp: *mut fmark_T, mark: pos_T, fnum: c_int, view: fmarkv_T) {
    unsafe {
        (*fmarkp).mark = mark;
        (*fmarkp).fnum = fnum;
        (*fmarkp).timestamp = os_time();
        (*fmarkp).view = view;
        (*fmarkp).additional_data = ptr::null_mut();
    }
}

#[inline]
unsafe fn reset_fmark(fmarkp: *mut fmark_T, mark: pos_T, fnum: c_int, view: fmarkv_T) {
    unsafe {
        free_fmark(*fmarkp);
        set_fmark(fmarkp, mark, fnum, view);
    }
}

#[inline]
unsafe fn set_xfmark(
    xfmarkp: *mut xfmark_T,
    mark: pos_T,
    fnum: c_int,
    view: fmarkv_T,
    fname: *mut c_char,
) {
    unsafe {
        (*xfmarkp).fname = fname;
        set_fmark(&raw mut (*xfmarkp).fmark, mark, fnum, view);
    }
}

#[inline]
unsafe fn reset_xfmark(
    xfmarkp: *mut xfmark_T,
    mark: pos_T,
    fnum: c_int,
    view: fmarkv_T,
    fname: *mut c_char,
) {
    unsafe {
        free_xfmark(*xfmarkp);
        set_xfmark(xfmarkp, mark, fnum, view, fname);
    }
}

/// Set named mark `c` at current cursor position (`setmark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn setmark(c: c_int) -> c_int {
    unsafe {
        let view = mark_view_make(curwin, (*curwin).w_cursor);
        setmark_pos(
            c,
            &raw mut (*curwin).w_cursor,
            (*curbuf).b_fnum,
            &raw const view as *mut fmarkv_T,
        )
    }
}

/// Free `fmark_T` item (`free_fmark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_fmark(fm: fmark_T) {
    unsafe {
        xfree(fm.additional_data.cast());
    }
}

/// Free `xfmark_T` item (`free_xfmark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_xfmark(fm: xfmark_T) {
    unsafe {
        xfree(fm.fname.cast());
        free_fmark(fm.fmark);
    }
}

/// Free and clear `fmark_T` item (`clear_fmark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clear_fmark(fm: *mut fmark_T, timestamp: Timestamp) {
    unsafe {
        free_fmark(*fm);
        *fm = INIT_FMARK;
        (*fm).timestamp = timestamp;
    }
}

/// Schedules `"MarkSet"` event (`do_markset_autocmd`).
unsafe fn do_markset_autocmd(c: c_char, pos: *const pos_T, buf: *mut buf_T) {
    unsafe {
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
                    data: ObjectData {
                        string: NvimString {
                            data: mark_str.as_mut_ptr(),
                            size: 1,
                        },
                    },
                },
            },
            KeyValuePair {
                key: NvimString {
                    data: c"line".as_ptr().cast_mut(),
                    size: 4,
                },
                value: Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: i64::from((*pos).lnum),
                    },
                },
            },
            KeyValuePair {
                key: NvimString {
                    data: c"col".as_ptr().cast_mut(),
                    size: 3,
                },
                value: Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: i64::from((*pos).col),
                    },
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
            data: ObjectData { dict: data },
        };
        aucmd_defer(
            EVENT_MARKSET,
            mark_str.as_mut_ptr(),
            ptr::null_mut(),
            AUGROUP_ALL,
            buf,
            ptr::null_mut(),
            &raw mut dict_obj,
        );
    }
}

/// Set named mark `c` to position `pos` (`setmark_pos`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn setmark_pos(
    c: c_int,
    pos: *mut pos_T,
    fnum: c_int,
    view_pt: *mut fmarkv_T,
) -> c_int {
    unsafe {
        let view = if !view_pt.is_null() {
            *view_pt
        } else {
            INIT_FMARKV
        };

        if c < 0 {
            return FAIL;
        }

        if c == b'\'' as c_int || c == b'`' as c_int {
            if ptr::eq(pos, &raw const (*curwin).w_cursor) {
                setpcmark();
                (*curwin).w_prev_pcmark = (*curwin).w_pcmark;
            } else {
                (*curwin).w_pcmark = *pos;
            }
            return OK;
        }

        let buf = buflist_findnr(fnum);
        if buf.is_null() {
            return FAIL;
        }

        if c == b'"' as c_int {
            reset_fmark(&raw mut (*buf).b_last_cursor, *pos, (*buf).b_fnum, view);
            do_markset_autocmd(c as c_char, pos, buf);
            return OK;
        }

        if c == b'[' as c_int {
            (*buf).b_op_start = *pos;
            do_markset_autocmd(c as c_char, pos, buf);
            return OK;
        }
        if c == b']' as c_int {
            (*buf).b_op_end = *pos;
            do_markset_autocmd(c as c_char, pos, buf);
            return OK;
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
            do_markset_autocmd(c as c_char, pos, buf);
            return OK;
        }

        if c == b':' as c_int && bt_prompt(buf) {
            reset_fmark(&raw mut (*buf).b_prompt_start, *pos, (*buf).b_fnum, view);
            return OK;
        }

        if ascii_islower(c) {
            let i = (c - b'a' as c_int) as usize;
            reset_fmark(&raw mut (*buf).b_namedm[i], *pos, fnum, view);
            u_update_named_mark(buf, i as c_int);
            do_markset_autocmd(c as c_char, pos, buf);
            return OK;
        }
        if ascii_isupper(c) || ascii_isdigit(c) {
            let i = if ascii_isdigit(c) {
                (c - b'0' as c_int) as usize + NMARKS
            } else {
                (c - b'A' as c_int) as usize
            };
            reset_xfmark(&raw mut namedfm[i], *pos, fnum, view, ptr::null_mut());
            do_markset_autocmd(c as c_char, pos, buf);
            return OK;
        }
        FAIL
    }
}

/// Remove every jump list entry referring to a given buffer (`mark_jumplist_forget_file`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_jumplist_forget_file(wp: *mut win_T, fnum: c_int) {
    unsafe {
        let mut i = (*wp).w_jumplistlen - 1;
        while i >= 0 {
            let idx = i as usize;
            if (*wp).w_jumplist[idx].fmark.fnum == fnum {
                free_xfmark((*wp).w_jumplist[idx]);
                if (*wp).w_jumplistidx > i {
                    (*wp).w_jumplistidx -= 1;
                }
                (*wp).w_jumplistlen -= 1;
                let dst = &raw mut (*wp).w_jumplist[idx];
                ptr::copy(dst.add(1), dst, ((*wp).w_jumplistlen - i) as usize);
            }
            i -= 1;
        }
    }
}

/// Delete every entry referring to file `fnum` from both the jumplist and the tag stack (`mark_forget_file`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_forget_file(wp: *mut win_T, fnum: c_int) {
    unsafe {
        mark_jumplist_forget_file(wp, fnum);

        let mut i = (*wp).w_tagstacklen - 1;
        while i >= 0 {
            let idx = i as usize;
            if (*wp).w_tagstack[idx].fmark.fnum == fnum {
                tagstack_clear_entry(&raw mut (*wp).w_tagstack[idx]);
                if (*wp).w_tagstackidx > i {
                    (*wp).w_tagstackidx -= 1;
                }
                (*wp).w_tagstacklen -= 1;
                let dst = &raw mut (*wp).w_tagstack[idx];
                ptr::copy(dst.add(1), dst, ((*wp).w_tagstacklen - i) as usize);
            }
            i -= 1;
        }
    }
}

/// Set the previous context mark to the current position and add it to the jump list (`setpcmark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn setpcmark() {
    unsafe {
        if global_busy != 0
            || listcmd_busy
            || (cmdmod.cmod_flags & CMOD_KEEPJUMPS) != 0
            || mc_replaying()
        {
            return;
        }

        (*curwin).w_prev_pcmark = (*curwin).w_pcmark;
        (*curwin).w_pcmark = (*curwin).w_cursor;

        if (*curwin).w_pcmark.lnum == 0 {
            (*curwin).w_pcmark.lnum = 1;
        }

        if (jop_flags & kOptJopFlagStack) != 0
            && (*curwin).w_jumplistidx < (*curwin).w_jumplistlen - 1
        {
            (*curwin).w_jumplistlen = (*curwin).w_jumplistidx + 1;
        }

        (*curwin).w_jumplistlen += 1;
        if (*curwin).w_jumplistlen > JUMPLISTSIZE {
            (*curwin).w_jumplistlen = JUMPLISTSIZE;
            free_xfmark((*curwin).w_jumplist[0]);
            let dst = &raw mut (*curwin).w_jumplist[0];
            ptr::copy(dst.add(1), dst, (JUMPLISTSIZE - 1) as usize);
        }
        (*curwin).w_jumplistidx = (*curwin).w_jumplistlen;
        let fm = &raw mut (*curwin).w_jumplist[((*curwin).w_jumplistlen - 1) as usize];

        let view = mark_view_make(curwin, (*curwin).w_pcmark);
        set_xfmark(
            fm,
            (*curwin).w_pcmark,
            (*curbuf).b_fnum,
            view,
            ptr::null_mut(),
        );
    }
}

/// Check previous context mark (`checkpcmark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn checkpcmark() {
    unsafe {
        if (*curwin).w_prev_pcmark.lnum != 0
            && (equalpos((*curwin).w_pcmark, (*curwin).w_cursor) || (*curwin).w_pcmark.lnum == 0)
        {
            (*curwin).w_pcmark = (*curwin).w_prev_pcmark;
        }
        (*curwin).w_prev_pcmark.lnum = 0;
    }
}

/// Get mark in `count` position in the jumplist relative to the current index (`get_jumplist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_jumplist(win: *mut win_T, mut count: c_int) -> *mut fmark_T {
    unsafe {
        cleanup_jumplist(win, true);

        if (*win).w_jumplistlen == 0 {
            return ptr::null_mut();
        }

        loop {
            if (*win).w_jumplistidx + count < 0
                || (*win).w_jumplistidx + count >= (*win).w_jumplistlen
            {
                return ptr::null_mut();
            }

            if (*win).w_jumplistidx == (*win).w_jumplistlen {
                setpcmark();
                (*win).w_jumplistidx -= 1;
                if (*win).w_jumplistidx + count < 0 {
                    return ptr::null_mut();
                }
            }

            (*win).w_jumplistidx += count;

            let jmp = &raw mut (*win).w_jumplist[(*win).w_jumplistidx as usize];
            if (*jmp).fmark.fnum == 0 {
                fname2fnum(jmp);
            }
            if (*jmp).fmark.fnum != (*curbuf).b_fnum && buflist_findnr((*jmp).fmark.fnum).is_null()
            {
                count += if count < 0 { -1 } else { 1 };
                continue;
            }
            return &raw mut (*jmp).fmark;
        }
    }
}

/// Get mark in `count` position in the changelist relative to the current index (`get_changelist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_changelist(
    buf: *mut buf_T,
    win: *mut win_T,
    count: c_int,
) -> *mut fmark_T {
    unsafe {
        if (*buf).b_changelistlen == 0 {
            return ptr::null_mut();
        }

        let mut n = (*win).w_changelistidx;
        if n + count < 0 {
            if n == 0 {
                return ptr::null_mut();
            }
            n = 0;
        } else if n + count >= (*buf).b_changelistlen {
            if n == (*buf).b_changelistlen - 1 {
                return ptr::null_mut();
            }
            n = (*buf).b_changelistlen - 1;
        } else {
            n += count;
        }
        (*win).w_changelistidx = n;
        let fm = &raw mut (*buf).b_changelist[n as usize];
        (*fm).fnum = (*curbuf).b_fnum;
        fm
    }
}

/// Get a named mark (`mark_get`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_get(
    buf: *mut buf_T,
    win: *mut win_T,
    fmp: *mut fmark_T,
    flag: MarkGet,
    name: c_int,
) -> *mut fmark_T {
    unsafe {
        let mut fm: *mut fmark_T = ptr::null_mut();
        if ascii_isupper(name) || ascii_isdigit(name) {
            let xfm = mark_get_global(flag != kMarkAllNoResolve, name);
            fm = &raw mut (*xfm).fmark;
            if flag == kMarkBufLocal && (*xfm).fmark.fnum != (*buf).b_fnum {
                return pos_to_mark(
                    buf,
                    ptr::null_mut(),
                    pos_T {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    },
                );
            }
        } else if name > 0 && name < NMARK_LOCAL_MAX {
            fm = mark_get_local(buf, win, name);
        }
        if !fmp.is_null() && !fm.is_null() {
            *fmp = *fm;
            return fmp;
        }
        fm
    }
}

/// Get a global mark `{A-Z0-9}` (`mark_get_global`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_get_global(resolve: bool, mut name: c_int) -> *mut xfmark_T {
    unsafe {
        if ascii_isdigit(name) {
            name = name - b'0' as c_int + NMARKS as c_int;
        } else if ascii_isupper(name) {
            name -= b'A' as c_int;
        } else {
            debug_assert!(false);
        }
        let mark = &raw mut namedfm[name as usize];
        if resolve && (*mark).fmark.fnum == 0 {
            fname2fnum(mark);
        }
        mark
    }
}

/// Get a local mark (`mark_get_local`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_get_local(
    buf: *mut buf_T,
    win: *mut win_T,
    name: c_int,
) -> *mut fmark_T {
    unsafe {
        let mark = if ascii_islower(name) {
            &raw mut (*buf).b_namedm[(name - b'a' as c_int) as usize]
        } else if name == b'[' as c_int {
            pos_to_mark(buf, ptr::null_mut(), (*buf).b_op_start)
        } else if name == b']' as c_int {
            pos_to_mark(buf, ptr::null_mut(), (*buf).b_op_end)
        } else if name == b'<' as c_int || name == b'>' as c_int {
            mark_get_visual(buf, name)
        } else if name == b'\'' as c_int || name == b'`' as c_int {
            pos_to_mark(curbuf, ptr::null_mut(), (*win).w_pcmark)
        } else if name == b'"' as c_int {
            &raw mut (*buf).b_last_cursor
        } else if name == b'^' as c_int {
            &raw mut (*buf).b_last_insert
        } else if name == b'.' as c_int {
            &raw mut (*buf).b_last_change
        } else if name == b':' as c_int && bt_prompt(buf) {
            &raw mut (*buf).b_prompt_start
        } else {
            mark_get_motion(buf, win, name)
        };

        if !mark.is_null() {
            (*mark).fnum = (*buf).b_fnum;
        }

        mark
    }
}

/// Get marks that are actually motions (`mark_get_motion`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_get_motion(
    buf: *mut buf_T,
    win: *mut win_T,
    name: c_int,
) -> *mut fmark_T {
    unsafe {
        let mut mark: *mut fmark_T = ptr::null_mut();
        let pos = (*curwin).w_cursor;
        let slcb = listcmd_busy;
        listcmd_busy = true;
        if name == b'{' as c_int || name == b'}' as c_int {
            let mut oa: oparg_T = std::mem::zeroed();
            if findpar(
                &raw mut oa.inclusive,
                if name == b'}' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                1,
                c_int::from(NUL),
                false,
            ) {
                mark = pos_to_mark(buf, ptr::null_mut(), (*win).w_cursor);
            }
        } else if (name == b'(' as c_int || name == b')' as c_int)
            && findsent(
                if name == b')' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                1,
            ) != 0
        {
            mark = pos_to_mark(buf, ptr::null_mut(), (*win).w_cursor);
        }
        (*curwin).w_cursor = pos;
        listcmd_busy = slcb;
        mark
    }
}

/// Get visual marks `'<'`, `'>'` (`mark_get_visual`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_get_visual(buf: *mut buf_T, name: c_int) -> *mut fmark_T {
    unsafe {
        let mut mark: *mut fmark_T = ptr::null_mut();
        if name == b'<' as c_int || name == b'>' as c_int {
            let startp = (*buf).b_visual.vi_start;
            let endp = (*buf).b_visual.vi_end;
            mark = if ((name == b'<' as c_int) == lt(startp, endp) || endp.lnum == 0)
                && startp.lnum != 0
            {
                pos_to_mark(buf, ptr::null_mut(), startp)
            } else {
                pos_to_mark(buf, ptr::null_mut(), endp)
            };

            if (*buf).b_visual.vi_mode == b'V' as c_int {
                (*mark).mark.col = if name == b'<' as c_int { 0 } else { MAXCOL };
                (*mark).mark.coladd = 0;
            }
        }
        mark
    }
}

/// Wrap a `pos_T` into an `fmark_T` (`pos_to_mark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn pos_to_mark(
    buf: *mut buf_T,
    fmp: *mut fmark_T,
    pos: pos_T,
) -> *mut fmark_T {
    unsafe {
        let fm = if fmp.is_null() {
            &raw mut POS_TO_MARK_FMS
        } else {
            fmp
        };
        (*fm).fnum = (*buf).b_fnum;
        (*fm).mark = pos;
        fm
    }
}

unsafe fn switch_to_mark_buf(fm: *mut fmark_T, pcmark_on_switch: bool) -> MarkMoveRes {
    unsafe {
        if (*fm).fnum != (*curbuf).b_fnum {
            let getfile_flag = if pcmark_on_switch { GETF_SETMARK } else { 0 };
            let res = buflist_getfile((*fm).fnum, (*fm).mark.lnum, getfile_flag, 0) == OK;
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_move_to(mut fm: *mut fmark_T, flags: MarkMove) -> MarkMoveRes {
    unsafe {
        let mut res: MarkMoveRes = kMarkMoveSuccess;
        let mut errormsg: *const c_char = ptr::null();
        if !mark_check(fm, &raw mut errormsg) {
            if !errormsg.is_null() {
                emsg(errormsg);
            }
            return kMarkMoveFailed;
        }

        if (*fm).fnum != (*curbuf).b_fnum {
            MARK_MOVE_TO_FM_COPY = *fm;
            fm = &raw mut MARK_MOVE_TO_FM_COPY;
            res |= switch_to_mark_buf(fm, (flags & kMarkJumpList) == 0);
            if (res & kMarkMoveFailed) != 0 {
                return res;
            }
            if !mark_check_line_bounds(curbuf, fm, &raw mut errormsg) {
                if !errormsg.is_null() {
                    emsg(errormsg);
                }
                res |= kMarkMoveFailed;
                return res;
            }
        } else if (flags & kMarkContext) != 0 {
            setpcmark();
        }

        let prev_pos = (*curwin).w_cursor;
        let pos = (*fm).mark;
        (*curwin).w_cursor = (*fm).mark;
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
            mark_view_restore(fm);
        }

        if (res & kMarkSwitchedBuf) != 0 || (res & kMarkChangedCursor) != 0 {
            check_cursor(curwin);
        }
        res
    }
}

/// Restore the mark view (`mark_view_restore`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_view_restore(fm: *mut fmark_T) {
    unsafe {
        if !fm.is_null() && (*fm).view.topline_offset >= 0 {
            let topline = (*fm).mark.lnum - (*fm).view.topline_offset;
            if topline >= 1 {
                set_topline(curwin, topline);
                (*curwin).w_skipcol = if (*fm).view.skipcol > 0
                    && !hasFolding(curwin, topline, ptr::null_mut(), ptr::null_mut())
                    && (*fm).view.skipcol < linetabsize_eol(curwin, topline)
                {
                    (*fm).view.skipcol
                } else {
                    0
                };
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_view_make(wp: *const win_T, pos: pos_T) -> fmarkv_T {
    unsafe {
        fmarkv_T {
            topline_offset: pos.lnum - (*wp).w_topline,
            skipcol: (*wp).w_skipcol,
        }
    }
}

/// Search for the next named mark in the current file from a start position (`getnextmark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getnextmark(
    startpos: *mut pos_T,
    dir: c_int,
    begin_line: c_int,
) -> *mut fmark_T {
    unsafe {
        let mut result: *mut fmark_T = ptr::null_mut();
        let mut pos = *startpos;

        if dir == BACKWARD && begin_line != 0 {
            pos.col = 0;
        } else if dir == FORWARD && begin_line != 0 {
            pos.col = MAXCOL;
        }

        for i in 0..NMARKS {
            if (*curbuf).b_namedm[i].mark.lnum > 0 {
                if dir == FORWARD {
                    if (result.is_null() || lt((*curbuf).b_namedm[i].mark, (*result).mark))
                        && lt(pos, (*curbuf).b_namedm[i].mark)
                    {
                        result = &raw mut (*curbuf).b_namedm[i];
                    }
                } else if (result.is_null() || lt((*result).mark, (*curbuf).b_namedm[i].mark))
                    && lt((*curbuf).b_namedm[i].mark, pos)
                {
                    result = &raw mut (*curbuf).b_namedm[i];
                }
            }
        }

        result
    }
}

/// For an extended filemark: set the `fnum` from the `fname` (`fname2fnum`).
unsafe fn fname2fnum(fm: *mut xfmark_T) {
    unsafe {
        if (*fm).fname.is_null() {
            return;
        }

        if *(*fm).fname == b'~' as c_char
            && vim_ispathsep_nocolon(c_int::from(*(*fm).fname.add(1) as u8))
        {
            let len = expand_env(
                c"~/".as_ptr().cast_mut(),
                &raw mut NameBuff[0],
                MAXPATHL as c_int,
            );
            xstrlcpy(
                (&raw mut NameBuff[0]).add(len),
                (*fm).fname.add(2),
                MAXPATHL - len,
            );
        } else {
            xstrlcpy(&raw mut NameBuff[0], (*fm).fname, MAXPATHL);
        }

        os_dirname(&raw mut IObuff[0], IOSIZE);
        let p = path_shorten_fname(&raw mut NameBuff[0], &raw mut IObuff[0]);

        let _ = buflist_new(&raw mut NameBuff[0], p, 1, 0);
    }
}

/// Check all file marks for a name that matches the file name in `buf` (`fmarks_check_names`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fmarks_check_names(buf: *mut buf_T) {
    unsafe {
        let name = (*buf).b_ffname;
        if name.is_null() {
            return;
        }

        for i in 0..NGLOBALMARKS {
            fmarks_check_one(&raw mut namedfm[i], name, buf);
        }

        let mut wp = first_win_in_tab(curtab);
        while !wp.is_null() {
            for i in 0..((*wp).w_jumplistlen as usize) {
                fmarks_check_one(&raw mut (*wp).w_jumplist[i], name, buf);
            }
            wp = (*wp).w_next;
        }
    }
}

unsafe fn fmarks_check_one(fm: *mut xfmark_T, name: *mut c_char, buf: *mut buf_T) {
    unsafe {
        if (*fm).fmark.fnum == 0
            && !(*fm).fname.is_null()
            && path_equal(name, (*fm).fname, kPathCmpLiteral)
        {
            (*fm).fmark.fnum = (*buf).b_fnum;
            xfree((*fm).fname.cast());
            (*fm).fname = ptr::null_mut();
        }
    }
}

/// Check the position in `fm` is valid (`mark_check`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_check(fm: *mut fmark_T, errormsg: *mut *const c_char) -> bool {
    unsafe {
        if fm.is_null() {
            *errormsg = gettext(e_umark.as_ptr());
            return false;
        } else if (*fm).mark.lnum <= 0 {
            if (*fm).mark.lnum == 0 {
                *errormsg = gettext(e_marknotset.as_ptr());
            }
            return false;
        }
        if (*fm).fnum == (*curbuf).b_fnum && !mark_check_line_bounds(curbuf, fm, errormsg) {
            return false;
        }
        true
    }
}

/// Check if a mark line number is greater than the buffer line count (`mark_check_line_bounds`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_check_line_bounds(
    buf: *mut buf_T,
    fm: *mut fmark_T,
    errormsg: *mut *const c_char,
) -> bool {
    unsafe {
        if !buf.is_null() && (*fm).mark.lnum > (*buf).b_ml.ml_line_count {
            *errormsg = gettext(e_markinval.as_ptr());
            return false;
        }
        true
    }
}

/// Clear all marks and change list in the given buffer (`clrallmarks`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clrallmarks(buf: *mut buf_T, timestamp: Timestamp) {
    unsafe {
        for i in 0..NMARKS {
            clear_fmark(&raw mut (*buf).b_namedm[i], timestamp);
        }
        clear_fmark(&raw mut (*buf).b_last_cursor, timestamp);
        (*buf).b_last_cursor.mark.lnum = 1;
        clear_fmark(&raw mut (*buf).b_last_insert, timestamp);
        clear_fmark(&raw mut (*buf).b_last_change, timestamp);
        (*buf).b_op_start.lnum = 0;
        (*buf).b_op_end.lnum = 0;
        for i in 0..((*buf).b_changelistlen as usize) {
            clear_fmark(&raw mut (*buf).b_changelist[i], timestamp);
        }
        (*buf).b_changelistlen = 0;
    }
}

/// Get name of file from a filemark (`fm_getname`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fm_getname(fmark: *mut fmark_T, lead_len: c_int) -> *mut c_char {
    unsafe {
        if (*fmark).fnum == (*curbuf).b_fnum {
            return mark_line(&raw mut (*fmark).mark, lead_len);
        }
        buflist_nr2name((*fmark).fnum, 0, 1)
    }
}

/// Return the line at mark `mp`, truncated to fit in window (`mark_line`).
unsafe fn mark_line(mp: *mut pos_T, lead_len: c_int) -> *mut c_char {
    unsafe {
        if (*mp).lnum == 0 || (*mp).lnum > (*curbuf).b_ml.ml_line_count {
            return xstrdup(c"-invalid-".as_ptr());
        }
        debug_assert!(Columns >= 0);
        let s = xstrnsave(skipwhite(ml_get((*mp).lnum)), (Columns as usize) * 5);

        let mut len: c_int = 0;
        let mut p = s;
        while *p != NUL {
            len += ptr2cells(p);
            if len >= Columns - lead_len {
                break;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        *p = NUL;
        s
    }
}

/// `:marks {arg}` (`ex_marks`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_marks(eap: *mut exarg_T) {
    unsafe {
        nlua_call_excmd(
            c"vim._core.marks".as_ptr(),
            c"ex_marks".as_ptr(),
            eap,
            &raw const cmdmod,
            ptr::null_mut(),
        );
    }
}

/// `:delmarks[!] [marks]` (`ex_delmarks`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_delmarks(eap: *mut exarg_T) {
    unsafe {
        let pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };

        if *(*eap).arg == NUL && (*eap).forceit != 0 {
            for i in 0..NMARKS {
                if (*curbuf).b_namedm[i].mark.lnum != 0 {
                    do_markset_autocmd((i as u8 + b'a') as c_char, &raw const pos, curbuf);
                }
            }
            if (*curbuf).b_last_cursor.mark.lnum != 0 {
                do_markset_autocmd(b'"' as c_char, &raw const pos, curbuf);
            }
            if (*curbuf).b_last_insert.mark.lnum != 0 {
                do_markset_autocmd(b'^' as c_char, &raw const pos, curbuf);
            }
            if (*curbuf).b_last_change.mark.lnum != 0 {
                do_markset_autocmd(b'.' as c_char, &raw const pos, curbuf);
            }
            if (*curbuf).b_op_start.lnum != 0 {
                do_markset_autocmd(b'[' as c_char, &raw const pos, curbuf);
            }
            if (*curbuf).b_op_end.lnum != 0 {
                do_markset_autocmd(b']' as c_char, &raw const pos, curbuf);
            }
            clrallmarks(curbuf, os_time());
        } else if (*eap).forceit != 0 {
            emsg(gettext(e_invarg.as_ptr()));
        } else if *(*eap).arg == NUL {
            emsg(gettext(e_argreq.as_ptr()));
        } else {
            let timestamp = os_time();
            let mut p = (*eap).arg;
            while *p != NUL {
                let ch = c_int::from(*p as u8);
                let lower = ascii_islower(ch);
                let digit = ascii_isdigit(ch);
                if lower || digit || ascii_isupper(ch) {
                    let from: c_int;
                    let to: c_int;
                    if *p.add(1) == b'-' as c_char {
                        from = ch;
                        to = c_int::from(*p.add(2) as u8);
                        let valid_to = if lower {
                            ascii_islower(to)
                        } else if digit {
                            ascii_isdigit(to)
                        } else {
                            ascii_isupper(to)
                        };
                        if !valid_to || to < from {
                            semsg!(gettext(e_invarg2.as_ptr()), p);
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
                            if (*curbuf).b_namedm[idx].mark.lnum != 0 {
                                do_markset_autocmd(i as c_char, &raw const pos, curbuf);
                            }
                            (*curbuf).b_namedm[idx].mark.lnum = 0;
                            (*curbuf).b_namedm[idx].timestamp = timestamp;
                        } else {
                            let n = if digit {
                                (i - b'0' as c_int) as usize + NMARKS
                            } else {
                                (i - b'A' as c_int) as usize
                            };
                            if namedfm[n].fmark.mark.lnum != 0 {
                                let mut buf = buflist_findnr(namedfm[n].fmark.fnum);
                                if buf.is_null() {
                                    buf = curbuf;
                                }
                                do_markset_autocmd(i as c_char, &raw const pos, buf);
                            }
                            namedfm[n].fmark.mark.lnum = 0;
                            namedfm[n].fmark.fnum = 0;
                            namedfm[n].fmark.timestamp = timestamp;
                            xfree(namedfm[n].fname.cast());
                            namedfm[n].fname = ptr::null_mut();
                        }
                    }
                } else {
                    match *p as u8 {
                        b'"' => {
                            if (*curbuf).b_last_cursor.mark.lnum != 0 {
                                do_markset_autocmd(*p, &raw const pos, curbuf);
                            }
                            clear_fmark(&raw mut (*curbuf).b_last_cursor, timestamp);
                        }
                        b'^' => {
                            if (*curbuf).b_last_insert.mark.lnum != 0 {
                                do_markset_autocmd(*p, &raw const pos, curbuf);
                            }
                            clear_fmark(&raw mut (*curbuf).b_last_insert, timestamp);
                        }
                        b':' => {}
                        b'.' => {
                            if (*curbuf).b_last_change.mark.lnum != 0 {
                                do_markset_autocmd(*p, &raw const pos, curbuf);
                            }
                            clear_fmark(&raw mut (*curbuf).b_last_change, timestamp);
                        }
                        b'[' => {
                            if (*curbuf).b_op_start.lnum != 0 {
                                do_markset_autocmd(*p, &raw const pos, curbuf);
                            }
                            (*curbuf).b_op_start.lnum = 0;
                        }
                        b']' => {
                            if (*curbuf).b_op_end.lnum != 0 {
                                do_markset_autocmd(*p, &raw const pos, curbuf);
                            }
                            (*curbuf).b_op_end.lnum = 0;
                        }
                        b'<' => {
                            if (*curbuf).b_visual.vi_start.lnum != 0 {
                                do_markset_autocmd(*p, &raw const pos, curbuf);
                            }
                            (*curbuf).b_visual.vi_start.lnum = 0;
                        }
                        b'>' => {
                            if (*curbuf).b_visual.vi_end.lnum != 0 {
                                do_markset_autocmd(*p, &raw const pos, curbuf);
                            }
                            (*curbuf).b_visual.vi_end.lnum = 0;
                        }
                        b' ' => {}
                        _ => {
                            semsg!(gettext(e_invarg2.as_ptr()), p);
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_jumps(_eap: *mut exarg_T) {
    unsafe {
        cleanup_jumplist(curwin, true);
        msg_ext_set_kind(c"list_cmd".as_ptr());
        msg_puts_title(gettext(c"\n jump line  col file/text".as_ptr()));
        let mut i: c_int = 0;
        while i < (*curwin).w_jumplistlen && !got_int {
            let idx = i as usize;
            if (*curwin).w_jumplist[idx].fmark.mark.lnum != 0 {
                let mut name = fm_getname(&raw mut (*curwin).w_jumplist[idx].fmark, 16);

                if name.is_null() && i == (*curwin).w_jumplistidx {
                    name = xstrdup(c"-invalid-".as_ptr());
                }
                if name.is_null() || message_filtered(name) {
                    xfree(name.cast());
                    i += 1;
                    continue;
                }

                msg_putchar(b'\n' as c_int);
                if got_int {
                    xfree(name.cast());
                    break;
                }
                let indicator = if i == (*curwin).w_jumplistidx {
                    '>'
                } else {
                    ' '
                };
                let dist = (i - (*curwin).w_jumplistidx).abs();
                let lnum = (*curwin).w_jumplist[idx].fmark.mark.lnum;
                let col = (*curwin).w_jumplist[idx].fmark.mark.col;
                let s = format!("{indicator} {dist:2} {lnum:5} {col:4} ");
                let bytes = s.as_bytes();
                let copy_len = bytes.len().min(IOSIZE - 1);
                ptr::copy_nonoverlapping(
                    bytes.as_ptr().cast::<c_char>(),
                    &raw mut IObuff[0],
                    copy_len,
                );
                IObuff[copy_len] = NUL;

                msg_outtrans(&raw const IObuff[0], 0, false);
                msg_outtrans(
                    name,
                    if (*curwin).w_jumplist[idx].fmark.fnum == (*curbuf).b_fnum {
                        HLF_D
                    } else {
                        0
                    },
                    false,
                );
                xfree(name.cast());
                os_breakcheck();
            }
            i += 1;
        }
        if (*curwin).w_jumplistidx == (*curwin).w_jumplistlen {
            msg_puts(c"\n>".as_ptr());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_clearjumps(_eap: *mut exarg_T) {
    unsafe {
        free_jumplist(curwin);
        (*curwin).w_jumplistlen = 0;
        (*curwin).w_jumplistidx = 0;
    }
}

/// Print the changelist (`ex_changes`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_changes(_eap: *mut exarg_T) {
    unsafe {
        msg_ext_set_kind(c"list_cmd".as_ptr());
        msg_puts_title(gettext(c"\nchange line  col text".as_ptr()));

        let mut i: c_int = 0;
        while i < (*curbuf).b_changelistlen && !got_int {
            let idx = i as usize;
            if (*curbuf).b_changelist[idx].mark.lnum != 0 {
                msg_putchar(b'\n' as c_int);
                if got_int {
                    break;
                }
                let indicator = if i == (*curwin).w_changelistidx {
                    '>'
                } else {
                    ' '
                };
                let dist = (i - (*curwin).w_changelistidx).abs();
                let lnum = (*curbuf).b_changelist[idx].mark.lnum;
                let col = (*curbuf).b_changelist[idx].mark.col;
                let s = format!("{indicator} {dist:3} {lnum:5} {col:4} ");
                let bytes = s.as_bytes();
                let copy_len = bytes.len().min(IOSIZE - 1);
                ptr::copy_nonoverlapping(
                    bytes.as_ptr().cast::<c_char>(),
                    &raw mut IObuff[0],
                    copy_len,
                );
                IObuff[copy_len] = NUL;

                msg_outtrans(&raw const IObuff[0], 0, false);
                let name = mark_line(&raw mut (*curbuf).b_changelist[idx].mark, 17);
                msg_outtrans(name, HLF_D, false);
                xfree(name.cast());
                os_breakcheck();
            }
            i += 1;
        }
        if (*curwin).w_changelistidx == (*curbuf).b_changelistlen {
            msg_puts(c"\n>".as_ptr());
        }
    }
}

#[inline]
unsafe fn one_adjust(
    lp: *mut linenr_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
    unsafe {
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
}

#[inline]
unsafe fn one_adjust_nodel(
    lp: *mut linenr_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
    unsafe {
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
}

#[inline]
unsafe fn one_adjust_cursor(
    posp: *mut pos_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
    unsafe {
        if (*posp).lnum >= line1 && (*posp).lnum <= line2 {
            if amount == MAXLNUM {
                (*posp).lnum = (line1 - 1).max(1);
                (*posp).col = 0;
            } else {
                (*posp).lnum += amount;
            }
        } else if amount_after != 0 && (*posp).lnum > line2 {
            (*posp).lnum += amount_after;
        }
    }
}

/// Adjust marks between `line1` and `line2` (inclusive) to move `amount` lines (`mark_adjust`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_adjust(
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
    op: ExtmarkOp,
) {
    unsafe {
        mark_adjust_buf(
            curbuf,
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_adjust_nofold(
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
    op: ExtmarkOp,
) {
    unsafe {
        mark_adjust_buf(
            curbuf,
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_adjust_buf(
    buf: *mut buf_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
    adjust_folds: bool,
    mode: MarkAdjustMode,
    op: ExtmarkOp,
) {
    unsafe {
        let fnum = (*buf).b_fnum;
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

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            for i in 0..NMARKS {
                one_adjust(
                    &raw mut (*buf).b_namedm[i].mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
                if namedfm[i].fmark.fnum == fnum {
                    one_adjust_nodel(
                        &raw mut namedfm[i].fmark.mark.lnum,
                        line1,
                        line2,
                        amount,
                        amount_after,
                    );
                }
            }
            for i in NMARKS..NGLOBALMARKS {
                if namedfm[i].fmark.fnum == fnum {
                    one_adjust_nodel(
                        &raw mut namedfm[i].fmark.mark.lnum,
                        line1,
                        line2,
                        amount,
                        amount_after,
                    );
                }
            }

            one_adjust(
                &raw mut (*buf).b_last_insert.mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
            one_adjust(
                &raw mut (*buf).b_last_change.mark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );

            if !equalpos((*buf).b_last_cursor.mark, INITPOS)
                && (!by_term || (*buf).b_last_cursor.mark.lnum < (*buf).b_ml.ml_line_count)
            {
                one_adjust(
                    &raw mut (*buf).b_last_cursor.mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }

            if bt_prompt(buf) {
                one_adjust_nodel(
                    &raw mut (*buf).b_prompt_start.mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }

            for i in 0..((*buf).b_changelistlen as usize) {
                one_adjust_nodel(
                    &raw mut (*buf).b_changelist[i].mark.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }

            one_adjust_nodel(
                &raw mut (*buf).b_visual.vi_start.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
            one_adjust_nodel(
                &raw mut (*buf).b_visual.vi_end.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );

            if !qf_mark_adjust(buf, ptr::null_mut(), line1, line2, amount, amount_after) {
                (*buf).b_has_qf_entry &= !BUF_HAS_QF_ENTRY;
            }
            let mut found_one = false;
            let mut tab = first_tabpage;
            while !tab.is_null() {
                let mut win = first_win_in_tab(tab);
                while !win.is_null() {
                    found_one |= qf_mark_adjust(buf, win, line1, line2, amount, amount_after);
                    win = (*win).w_next;
                }
                tab = (*tab).tp_next;
            }
            if !found_one {
                (*buf).b_has_qf_entry &= !BUF_HAS_LL_ENTRY;
            }
        }

        if op != kExtmarkNOOP {
            extmark_adjust(buf, line1, line2, amount, amount_after, op);
        }

        if (*curwin).w_buffer == buf {
            one_adjust(
                &raw mut (*curwin).w_pcmark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );
            one_adjust(
                &raw mut (*curwin).w_prev_pcmark.lnum,
                line1,
                line2,
                amount,
                amount_after,
            );

            if saved_cursor.lnum != 0 {
                one_adjust_nodel(
                    &raw mut saved_cursor.lnum,
                    line1,
                    line2,
                    amount,
                    amount_after,
                );
            }
        }

        let mut tab = first_tabpage;
        while !tab.is_null() {
            let mut win = first_win_in_tab(tab);
            while !win.is_null() {
                if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                    for i in 0..((*win).w_jumplistlen as usize) {
                        if (*win).w_jumplist[i].fmark.fnum == fnum {
                            one_adjust_nodel(
                                &raw mut (*win).w_jumplist[i].fmark.mark.lnum,
                                line1,
                                line2,
                                amount,
                                amount_after,
                            );
                        }
                    }
                }

                if (*win).w_buffer == buf {
                    if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                        for i in 0..((*win).w_tagstacklen as usize) {
                            if (*win).w_tagstack[i].fmark.fnum == fnum {
                                one_adjust_nodel(
                                    &raw mut (*win).w_tagstack[i].fmark.mark.lnum,
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
                            &raw mut (*win).w_old_cursor_lnum,
                            line1,
                            line2,
                            amount,
                            amount_after,
                        );
                        one_adjust_nodel(
                            &raw mut (*win).w_old_visual_lnum,
                            line1,
                            line2,
                            amount,
                            amount_after,
                        );
                    }

                    if by_api
                        || (if by_term {
                            (*win).w_cursor.lnum < (*buf).b_ml.ml_line_count
                        } else {
                            win != curwin
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
                            (*win).w_cursor.lnum < (*buf).b_ml.ml_line_count
                        } else {
                            win != curwin
                        })
                    {
                        one_adjust_cursor(
                            &raw mut (*win).w_cursor,
                            line1,
                            line2,
                            amount,
                            amount_after,
                        );
                    }

                    if adjust_folds {
                        foldMarkAdjust(win, line1, line2, amount, amount_after);
                    }
                }
                win = (*win).w_next;
            }
            tab = (*tab).tp_next;
        }

        diff_mark_adjust(buf, line1, line2, amount, amount_after);

        for i in 0..(*buf).b_wininfo.size {
            let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
            if !by_term || (*wip).wi_mark.mark.lnum < (*buf).b_ml.ml_line_count {
                one_adjust_cursor(
                    &raw mut (*wip).wi_mark.mark,
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
unsafe fn col_adjust(
    posp: *mut pos_T,
    lnum: linenr_T,
    mincol: colnr_T,
    lnum_amount: linenr_T,
    col_amount: colnr_T,
    spaces_removed: c_int,
) {
    unsafe {
        if (*posp).lnum == lnum && (*posp).col >= mincol {
            (*posp).lnum += lnum_amount;
            if col_amount < 0 && (*posp).col <= -col_amount {
                (*posp).col = 0;
            } else if (*posp).col < spaces_removed {
                (*posp).col = col_amount + spaces_removed;
            } else {
                (*posp).col += col_amount;
            }
        }
    }
}

/// Adjust marks in line `lnum` at column `mincol` and further (`mark_col_adjust`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_col_adjust(
    lnum: linenr_T,
    mincol: colnr_T,
    lnum_amount: linenr_T,
    col_amount: colnr_T,
    spaces_removed: c_int,
) {
    unsafe {
        let fnum = (*curbuf).b_fnum;

        if (col_amount == 0 && lnum_amount == 0) || (cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            return;
        }

        for i in 0..NMARKS {
            col_adjust(
                &raw mut (*curbuf).b_namedm[i].mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
            if namedfm[i].fmark.fnum == fnum {
                col_adjust(
                    &raw mut namedfm[i].fmark.mark,
                    lnum,
                    mincol,
                    lnum_amount,
                    col_amount,
                    spaces_removed,
                );
            }
        }
        for i in NMARKS..NGLOBALMARKS {
            if namedfm[i].fmark.fnum == fnum {
                col_adjust(
                    &raw mut namedfm[i].fmark.mark,
                    lnum,
                    mincol,
                    lnum_amount,
                    col_amount,
                    spaces_removed,
                );
            }
        }

        col_adjust(
            &raw mut (*curbuf).b_last_insert.mark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &raw mut (*curbuf).b_last_change.mark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );

        if bt_prompt(curbuf) {
            col_adjust(
                &raw mut (*curbuf).b_prompt_start.mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
        }

        for i in 0..((*curbuf).b_changelistlen as usize) {
            col_adjust(
                &raw mut (*curbuf).b_changelist[i].mark,
                lnum,
                mincol,
                lnum_amount,
                col_amount,
                spaces_removed,
            );
        }

        col_adjust(
            &raw mut (*curbuf).b_visual.vi_start,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &raw mut (*curbuf).b_visual.vi_end,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );

        col_adjust(
            &raw mut (*curwin).w_pcmark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &raw mut (*curwin).w_prev_pcmark,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );
        col_adjust(
            &raw mut saved_cursor,
            lnum,
            mincol,
            lnum_amount,
            col_amount,
            spaces_removed,
        );

        let mut win = first_win_in_tab(curtab);
        while !win.is_null() {
            for i in 0..((*win).w_jumplistlen as usize) {
                if (*win).w_jumplist[i].fmark.fnum == fnum {
                    col_adjust(
                        &raw mut (*win).w_jumplist[i].fmark.mark,
                        lnum,
                        mincol,
                        lnum_amount,
                        col_amount,
                        spaces_removed,
                    );
                }
            }

            if (*win).w_buffer == curbuf {
                for i in 0..((*win).w_tagstacklen as usize) {
                    if (*win).w_tagstack[i].fmark.fnum == fnum {
                        col_adjust(
                            &raw mut (*win).w_tagstack[i].fmark.mark,
                            lnum,
                            mincol,
                            lnum_amount,
                            col_amount,
                            spaces_removed,
                        );
                    }
                }

                if win != curwin {
                    col_adjust(
                        &raw mut (*win).w_cursor,
                        lnum,
                        mincol,
                        lnum_amount,
                        col_amount,
                        spaces_removed,
                    );
                }
            }
            win = (*win).w_next;
        }
    }
}

/// Remove duplicate marks in the jumplist (`cleanup_jumplist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cleanup_jumplist(wp: *mut win_T, loadfiles: bool) {
    unsafe {
        if loadfiles {
            for i in 0..((*wp).w_jumplistlen as usize) {
                if (*wp).w_jumplist[i].fmark.fnum == 0 && (*wp).w_jumplist[i].fmark.mark.lnum != 0 {
                    fname2fnum(&raw mut (*wp).w_jumplist[i]);
                }
            }
        }

        let mut to: c_int = 0;
        for from in 0..(*wp).w_jumplistlen {
            let from_u = from as usize;
            if (*wp).w_jumplistidx == from {
                (*wp).w_jumplistidx = to;
            }
            let mut i = from + 1;
            while i < (*wp).w_jumplistlen {
                let i_u = i as usize;
                if (*wp).w_jumplist[i_u].fmark.fnum == (*wp).w_jumplist[from_u].fmark.fnum
                    && (*wp).w_jumplist[from_u].fmark.fnum != 0
                    && (*wp).w_jumplist[i_u].fmark.mark.lnum
                        == (*wp).w_jumplist[from_u].fmark.mark.lnum
                {
                    break;
                }
                i += 1;
            }

            let mustfree = if i >= (*wp).w_jumplistlen {
                false
            } else if i > from + 1 {
                (jop_flags & kOptJopFlagStack) == 0
            } else {
                true
            };

            if mustfree {
                xfree((*wp).w_jumplist[from_u].fname.cast());
            } else {
                if to != from {
                    (*wp).w_jumplist[to as usize] = (*wp).w_jumplist[from_u];
                }
                to += 1;
            }
        }
        if (*wp).w_jumplistidx == (*wp).w_jumplistlen {
            (*wp).w_jumplistidx = to;
        }
        (*wp).w_jumplistlen = to;

        if loadfiles && (*wp).w_jumplistlen != 0 && (*wp).w_jumplistidx == (*wp).w_jumplistlen {
            let fm_last = &(*wp).w_jumplist[((*wp).w_jumplistlen - 1) as usize];
            if fm_last.fmark.fnum == (*curbuf).b_fnum
                && fm_last.fmark.mark.lnum == (*wp).w_cursor.lnum
            {
                xfree(fm_last.fname.cast());
                (*wp).w_jumplistlen -= 1;
                (*wp).w_jumplistidx -= 1;
            }
        }
    }
}

/// Copy the jumplist from window `from` to window `to` (`copy_jumplist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copy_jumplist(from: *mut win_T, to: *mut win_T) {
    unsafe {
        for i in 0..((*from).w_jumplistlen as usize) {
            (*to).w_jumplist[i] = (*from).w_jumplist[i];
            if !(*from).w_jumplist[i].fname.is_null() {
                (*to).w_jumplist[i].fname = xstrdup((*from).w_jumplist[i].fname);
            }
        }
        (*to).w_jumplistlen = (*from).w_jumplistlen;
        (*to).w_jumplistidx = (*from).w_jumplistidx;
    }
}

/// Iterate over jumplist items (`mark_jumplist_iter`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_jumplist_iter(
    iter: *const c_void,
    win: *const win_T,
    fm: *mut xfmark_T,
) -> *const c_void {
    unsafe {
        if iter.is_null() && (*win).w_jumplistlen == 0 {
            *fm = INIT_XFMARK;
            return ptr::null();
        }
        let iter_mark = if iter.is_null() {
            &raw const (*win).w_jumplist[0]
        } else {
            iter.cast::<xfmark_T>()
        };
        *fm = *iter_mark;
        if ptr::eq(
            iter_mark,
            &raw const (*win).w_jumplist[((*win).w_jumplistlen - 1) as usize],
        ) {
            return ptr::null();
        }
        iter_mark.add(1).cast()
    }
}

/// Iterate over global marks (`mark_global_iter`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_global_iter(
    iter: *const c_void,
    name: *mut c_char,
    fm: *mut xfmark_T,
) -> *const c_void {
    unsafe {
        *name = NUL;
        let base = &raw const namedfm[0];
        let mut iter_mark = if iter.is_null() {
            base
        } else {
            iter.cast::<xfmark_T>()
        };
        while (iter_mark.offset_from(base) as usize) < NGLOBALMARKS
            && (*iter_mark).fmark.mark.lnum == 0
        {
            iter_mark = iter_mark.add(1);
        }
        if (iter_mark.offset_from(base) as usize) == NGLOBALMARKS
            || (*iter_mark).fmark.mark.lnum == 0
        {
            return ptr::null();
        }
        let iter_off = iter_mark.offset_from(base) as usize;
        *name = if iter_off < NMARKS {
            (b'A' + iter_off as u8) as c_char
        } else {
            (b'0' + (iter_off - NMARKS) as u8) as c_char
        };
        *fm = *iter_mark;
        loop {
            iter_mark = iter_mark.add(1);
            if (iter_mark.offset_from(base) as usize) >= NGLOBALMARKS {
                break;
            }
            if (*iter_mark).fmark.mark.lnum != 0 {
                return iter_mark.cast();
            }
        }
        ptr::null()
    }
}

#[inline]
unsafe fn next_buffer_mark(buf: *const buf_T, mark_name: &mut c_char) -> *const fmark_T {
    unsafe {
        match *mark_name as u8 {
            0 => {
                *mark_name = b'"' as c_char;
                &raw const (*buf).b_last_cursor
            }
            b'"' => {
                *mark_name = b'^' as c_char;
                &raw const (*buf).b_last_insert
            }
            b'^' => {
                *mark_name = b'.' as c_char;
                &raw const (*buf).b_last_change
            }
            b'.' => {
                *mark_name = b'a' as c_char;
                &raw const (*buf).b_namedm[0]
            }
            b'z' => ptr::null(),
            ch => {
                *mark_name = (ch + 1) as c_char;
                &raw const (*buf).b_namedm[(*mark_name as u8 - b'a') as usize]
            }
        }
    }
}

/// Iterate over buffer marks (`mark_buffer_iter`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_buffer_iter(
    iter: *const c_void,
    buf: *const buf_T,
    name: *mut c_char,
    fm: *mut fmark_T,
) -> *const c_void {
    unsafe {
        *name = NUL;
        let mut mark_name: c_char = if iter.is_null() {
            NUL
        } else if ptr::eq(iter.cast::<fmark_T>(), &raw const (*buf).b_last_cursor) {
            b'"' as c_char
        } else if ptr::eq(iter.cast::<fmark_T>(), &raw const (*buf).b_last_insert) {
            b'^' as c_char
        } else if ptr::eq(iter.cast::<fmark_T>(), &raw const (*buf).b_last_change) {
            b'.' as c_char
        } else {
            (b'a' as isize
                + iter
                    .cast::<fmark_T>()
                    .offset_from(&raw const (*buf).b_namedm[0])) as c_char
        };
        let mut iter_mark = next_buffer_mark(buf, &mut mark_name);
        while !iter_mark.is_null() && (*iter_mark).mark.lnum == 0 {
            iter_mark = next_buffer_mark(buf, &mut mark_name);
        }
        if iter_mark.is_null() {
            return ptr::null();
        }
        if mark_name != NUL {
            *name = mark_name;
        } else {
            let iter_off = iter_mark.offset_from(&raw const (*buf).b_namedm[0]) as usize;
            *name = (b'a' + iter_off as u8) as c_char;
        }
        *fm = *iter_mark;
        iter_mark.cast()
    }
}

/// Set global mark (`mark_set_global`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_set_global(name: c_char, fm: xfmark_T, update: bool) -> bool {
    unsafe {
        let idx = mark_global_index(name);
        if idx == -1 {
            return false;
        }
        let fm_tgt = &raw mut namedfm[idx as usize];
        if update && fm.fmark.timestamp <= (*fm_tgt).fmark.timestamp {
            return false;
        }
        if (*fm_tgt).fmark.mark.lnum != 0 {
            free_xfmark(*fm_tgt);
        }
        *fm_tgt = fm;
        true
    }
}

/// Set local mark (`mark_set_local`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_set_local(
    name: c_char,
    buf: *mut buf_T,
    fm: fmark_T,
    update: bool,
) -> bool {
    unsafe {
        let ch = c_int::from(name as u8);
        let fm_tgt = if ascii_islower(ch) {
            &raw mut (*buf).b_namedm[(ch - b'a' as c_int) as usize]
        } else if name == b'"' as c_char {
            &raw mut (*buf).b_last_cursor
        } else if name == b'^' as c_char {
            &raw mut (*buf).b_last_insert
        } else if name == b':' as c_char {
            &raw mut (*buf).b_prompt_start
        } else if name == b'.' as c_char {
            &raw mut (*buf).b_last_change
        } else {
            return false;
        };
        if update && fm.timestamp <= (*fm_tgt).timestamp {
            return false;
        }
        if (*fm_tgt).mark.lnum != 0 {
            free_fmark(*fm_tgt);
        }
        *fm_tgt = fm;
        true
    }
}

/// Free items in the jumplist of window `wp` (`free_jumplist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_jumplist(wp: *mut win_T) {
    unsafe {
        for i in 0..((*wp).w_jumplistlen as usize) {
            free_xfmark((*wp).w_jumplist[i]);
        }
        (*wp).w_jumplistlen = 0;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_last_cursor(win: *mut win_T) {
    unsafe {
        if !(*win).w_buffer.is_null() {
            reset_fmark(
                &raw mut (*(*win).w_buffer).b_last_cursor,
                (*win).w_cursor,
                0,
                INIT_FMARKV,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_all_marks() {
    unsafe {
        for i in 0..NGLOBALMARKS {
            if namedfm[i].fmark.mark.lnum != 0 {
                free_xfmark(namedfm[i]);
            }
            namedfm[i] = INIT_XFMARK;
        }
    }
}

/// Adjust position to point to the first byte of a multi-byte character (`mark_mb_adjustpos`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mark_mb_adjustpos(buf: *mut buf_T, lp: *mut pos_T) {
    unsafe {
        if (*lp).col > 0 || (*lp).coladd > 1 {
            let p = ml_get_buf(buf, (*lp).lnum);
            if *p == NUL || ml_get_buf_len(buf, (*lp).lnum) < (*lp).col {
                (*lp).col = 0;
            } else {
                (*lp).col -= utf_head_off(p, p.add((*lp).col as usize));
            }
            let p_at = p.add((*lp).col as usize);
            if (*lp).coladd == 1
                && *p_at != TAB
                && vim_isprintc(utf_ptr2char(p_at))
                && ptr2cells(p_at) > 1
            {
                (*lp).coladd = 0;
            }
        }
    }
}

/// Add information about mark `mname` to list `l` (`add_mark`).
unsafe fn add_mark(
    l: *mut list_T,
    mname: *const c_char,
    mnamelen: usize,
    pos: *const pos_T,
    bufnr: c_int,
    fname: *const c_char,
) -> c_int {
    unsafe {
        if (*pos).lnum <= 0 {
            return OK;
        }

        let d = tv_dict_alloc();
        tv_list_append_dict(l, d);

        let lpos = tv_list_alloc(kListLenMayKnow);
        tv_list_append_number(lpos, i64::from(bufnr));
        tv_list_append_number(lpos, i64::from((*pos).lnum));
        tv_list_append_number(
            lpos,
            i64::from(if (*pos).col < MAXCOL {
                (*pos).col + 1
            } else {
                MAXCOL
            }),
        );
        tv_list_append_number(lpos, i64::from((*pos).coladd));

        if tv_dict_add_str_len(d, c"mark".as_ptr(), 4, mname, mnamelen as c_int) == FAIL
            || tv_dict_add_list(d, c"pos".as_ptr(), 3, lpos) == FAIL
            || (!fname.is_null() && tv_dict_add_str(d, c"file".as_ptr(), 4, fname) == FAIL)
        {
            return FAIL;
        }

        OK
    }
}

/// Get information about marks local to a buffer (`get_buf_local_marks`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_buf_local_marks(buf: *const buf_T, l: *mut list_T) {
    unsafe {
        let mut mname: [c_char; 3] = [b'\'' as c_char, b' ' as c_char, 0];

        for i in 0..NMARKS {
            mname[1] = (b'a' + i as u8) as c_char;
            add_mark(
                l,
                mname.as_ptr(),
                2,
                &raw const (*buf).b_namedm[i].mark,
                (*buf).b_fnum,
                ptr::null(),
            );
        }

        add_mark(
            l,
            c"''".as_ptr(),
            2,
            &raw const (*curwin).w_pcmark,
            (*curbuf).b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'\"".as_ptr(),
            2,
            &raw const (*buf).b_last_cursor.mark,
            (*buf).b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'[".as_ptr(),
            2,
            &raw const (*buf).b_op_start,
            (*buf).b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"']".as_ptr(),
            2,
            &raw const (*buf).b_op_end,
            (*buf).b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'^".as_ptr(),
            2,
            &raw const (*buf).b_last_insert.mark,
            (*buf).b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'.".as_ptr(),
            2,
            &raw const (*buf).b_last_change.mark,
            (*buf).b_fnum,
            ptr::null(),
        );
        if bt_prompt(buf.cast_mut()) {
            add_mark(
                l,
                c"':".as_ptr(),
                2,
                &raw const (*buf).b_prompt_start.mark,
                (*buf).b_fnum,
                ptr::null(),
            );
        }
        add_mark(
            l,
            c"'<".as_ptr(),
            2,
            &raw const (*buf).b_visual.vi_start,
            (*buf).b_fnum,
            ptr::null(),
        );
        add_mark(
            l,
            c"'>".as_ptr(),
            2,
            &raw const (*buf).b_visual.vi_end,
            (*buf).b_fnum,
            ptr::null(),
        );
    }
}

/// Get a global mark (`get_raw_global_mark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_raw_global_mark(name: c_char) -> xfmark_T {
    unsafe { namedfm[mark_global_index(name) as usize] }
}

/// Get information about global marks (`get_global_marks`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_global_marks(l: *mut list_T) {
    unsafe {
        let mut mname: [c_char; 3] = [b'\'' as c_char, b' ' as c_char, 0];

        for i in 0..(NMARKS + EXTRA_MARKS) {
            let name = if namedfm[i].fmark.fnum != 0 {
                buflist_nr2name(namedfm[i].fmark.fnum, 1, 1)
            } else {
                namedfm[i].fname
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
                    2,
                    &raw const namedfm[i].fmark.mark,
                    namedfm[i].fmark.fnum,
                    name,
                );
                if namedfm[i].fmark.fnum != 0 {
                    xfree(name.cast());
                }
            }
        }
    }
}
