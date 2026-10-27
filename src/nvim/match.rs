//! Functions for highlighting matches (`src/nvim/match.c`).

use crate::buffer::buf_T;
use crate::charset::{NUL, ascii_iswhite, skiptowhite, skipwhite};
use crate::drawscreen::{UPD_SOME_VALID, UPD_VALID, redraw_later, redraw_win_range_later};
use crate::errors::*;
use crate::eval::funcs::get_optional_window;
use crate::eval::typval::{
    EvalFuncData, VarType, list_T, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str,
    tv_dict_add_str_len, tv_dict_alloc, tv_dict_find, tv_dict_get_number, tv_dict_get_string,
    tv_dict_get_string_buf, tv_get_number, tv_get_number_chk, tv_get_string, tv_get_string_buf_chk,
    tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_append_number,
    tv_list_append_string, tv_list_append_tv, tv_list_first, tv_list_idx_of_item, tv_list_len,
    tv_list_ref, tv_list_unref, typval_T, varnumber_T,
};
use crate::eval::window::find_win_by_nr_or_id;
use crate::ex_docmd::{ends_excmd, ex_errmsg, exarg_T, find_nextcmd, set_no_hlsearch};
use crate::fold::hasFolding;
use crate::globals::*;
use crate::highlight_group::{syn_check_group, syn_id2attr, syn_id2name, syn_name2id};
use crate::mbyte::{utf_char2bytes, utf_ptr2char, utfc_ptr2len};
use crate::memline::ml_get_buf;
use crate::message::{emsg, semsg};
use crate::option_vars::*;
use crate::profile::{profile_passed_limit, profile_setlimit, proftime_T};
use crate::regexp::{
    MAXCOL, NUMBUFLEN, RE_MAGIC, re_multiline, regmmatch_T, regprog_T, skip_regexp, vim_regcomp,
    vim_regexec_multi, vim_regfree,
};
use crate::strings::{gettext, vim_snprintf, vim_strchr};
use crate::window::win_T;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::change::MB_MAXCHAR;
use crate::cursor::{colnr_T, linenr_T};
use crate::eval::kListLenMayKnow;
use crate::math::{FAIL, OK};

const SEARCH_HL_PRIORITY: c_int = 0;

pub(crate) const HLF_L: usize = 8;
pub(crate) const HLF_LC: usize = 9;
pub(crate) const kCpoSearch: c_int = b'c' as c_int;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct match_T {
    pub rm: regmmatch_T,
    pub buf: *mut buf_T,
    pub lnum: linenr_T,
    pub attr: c_int,
    pub attr_cur: c_int,
    pub first_lnum: linenr_T,
    pub startcol: colnr_T,
    pub endcol: colnr_T,
    pub is_addpos: bool,
    pub has_cursor: bool,
    pub tm: proftime_T,
}

impl match_T {
    pub const fn new() -> Self {
        Self {
            rm: regmmatch_T::new(),
            buf: ptr::null_mut(),
            lnum: 0,
            attr: 0,
            attr_cur: 0,
            first_lnum: 0,
            startcol: 0,
            endcol: 0,
            is_addpos: false,
            has_cursor: false,
            tm: 0,
        }
    }
}

impl Default for match_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct llpos_T {
    pub lnum: linenr_T,
    pub col: colnr_T,
    pub len: c_int,
}

#[repr(C)]
pub struct matchitem_T {
    pub mit_id: c_int,
    pub mit_priority: c_int,
    pub mit_pattern: Option<std::ffi::CString>,
    pub mit_match: regmmatch_T,
    pub mit_pos: Vec<llpos_T>,
    pub mit_pos_cur: c_int,
    pub mit_toplnum: linenr_T,
    pub mit_botlnum: linenr_T,
    pub mit_hl: match_T,
    pub mit_hlg_id: c_int,
    pub mit_conceal_char: c_int,
}

impl Drop for matchitem_T {
    fn drop(&mut self) {
        unsafe {
            vim_regfree(self.mit_match.regprog);
        }
    }
}

#[repr(C)]
pub struct SearchState {
    pub hl_match: bool,
    pub match_lines: linenr_T,
    pub match_endcol: colnr_T,
    pub first_line: linenr_T,
    pub last_line: linenr_T,
    pub no_smartcase: bool,
    pub cmdlen: c_int,
    pub no_hlsearch: bool,
}

use crate::highlight::win_hl_attr;

/// Add match to the match list of window `wp` (`match_add`).
unsafe fn match_add(
    wp: *mut win_T,
    grp: *const c_char,
    pat: *const c_char,
    prio: c_int,
    mut id: c_int,
    pos_list: *mut list_T,
    conceal_char: *const c_char,
) -> c_int {
    unsafe {
        let mut regprog: *mut regprog_T = ptr::null_mut();
        let mut rtype = UPD_SOME_VALID;

        if *grp == NUL || (!pat.is_null() && *pat == NUL) {
            return -1;
        }
        if id < -1 || id == 0 {
            semsg!(
                gettext(c"E799: Invalid ID: %lld (must be greater than or equal to 1)".as_ptr()),
                i64::from(id),
            );
            return -1;
        }
        if id == -1 {
            id = (*wp).w_next_match_id;
            (*wp).w_next_match_id += 1;
        } else {
            if (*wp).w_match_head.iter().any(|cur| cur.mit_id == id) {
                semsg!(
                    gettext(c"E801: ID already taken: %lld".as_ptr()),
                    i64::from(id),
                );
                return -1;
            }

            if (*wp).w_next_match_id < id + 100 {
                (*wp).w_next_match_id = id + 100;
            }
        }

        let grp_len = CStr::from_ptr(grp).to_bytes().len();
        let hlg_id = syn_check_group(grp, grp_len);
        if hlg_id == 0 {
            return -1;
        }
        if !pat.is_null() {
            regprog = vim_regcomp(pat, RE_MAGIC);
            if regprog.is_null() {
                semsg!(gettext((&raw const e_invarg2).cast()), pat);
                return -1;
            }
        }

        let pos_len = tv_list_len(pos_list);
        let mut m = matchitem_T {
            mit_id: id,
            mit_priority: prio,
            mit_pattern: (!pat.is_null()).then(|| CStr::from_ptr(pat).to_owned()),
            mit_match: regmmatch_T {
                regprog,
                startpos: [crate::cursor::lpos_T { lnum: 0, col: 0 }; 10],
                endpos: [crate::cursor::lpos_T { lnum: 0, col: 0 }; 10],
                rmm_matchcol: 0,
                rmm_ic: 0,
                rmm_maxcol: 0,
            },
            mit_pos: if pos_len > 0 {
                vec![llpos_T::default(); pos_len as usize]
            } else {
                Vec::new()
            },
            mit_pos_cur: 0,
            mit_toplnum: 0,
            mit_botlnum: 0,
            mit_hl: match_T::new(),
            mit_hlg_id: hlg_id,
            mit_conceal_char: if conceal_char.is_null() {
                0
            } else {
                utf_ptr2char(conceal_char)
            },
        };

        if !pos_list.is_null() {
            let mut toplnum: linenr_T = 0;
            let mut botlnum: linenr_T = 0;
            let mut i: usize = 0;
            let mut failed = false;

            let mut li = tv_list_first(pos_list);
            while !li.is_null() {
                let mut lnum: linenr_T = 0;
                let mut col: colnr_T = 0;
                let mut len: c_int = 1;
                let mut error = false;

                if (*li).li_tv.v_type == VarType::VAR_LIST {
                    let subl = (*li).li_tv.vval.v_list();
                    let mut subli = tv_list_first(subl);
                    if subli.is_null() {
                        semsg!(
                            gettext(c"E5030: Empty list at position %d".as_ptr()),
                            tv_list_idx_of_item(pos_list, li),
                        );
                        failed = true;
                        break;
                    }
                    lnum = tv_get_number_chk(&raw const (*subli).li_tv, &mut error) as linenr_T;
                    if error {
                        failed = true;
                        break;
                    }
                    if lnum <= 0 {
                        li = (*li).li_next;
                        continue;
                    }
                    m.mit_pos[i].lnum = lnum;
                    subli = (*subli).li_next;
                    if !subli.is_null() {
                        col =
                            tv_get_number_chk(&raw const (*subli).li_tv, &mut error) as colnr_T;
                        if error {
                            failed = true;
                            break;
                        }
                        if col < 0 {
                            li = (*li).li_next;
                            continue;
                        }
                        subli = (*subli).li_next;
                        if !subli.is_null() {
                            len = tv_get_number_chk(&raw const (*subli).li_tv, &mut error)
                                as colnr_T;
                            if len < 0 {
                                li = (*li).li_next;
                                continue;
                            }
                            if error {
                                failed = true;
                                break;
                            }
                        }
                    }
                    m.mit_pos[i].col = col;
                    m.mit_pos[i].len = len;
                } else if (*li).li_tv.v_type == VarType::VAR_NUMBER {
                    if (*li).li_tv.vval.v_number() <= 0 {
                        li = (*li).li_next;
                        continue;
                    }
                    m.mit_pos[i].lnum = (*li).li_tv.vval.v_number() as linenr_T;
                    m.mit_pos[i].col = 0;
                    m.mit_pos[i].len = 0;
                } else {
                    semsg!(
                        gettext(c"E5031: List or number required at position %d".as_ptr()),
                        tv_list_idx_of_item(pos_list, li),
                    );
                    failed = true;
                    break;
                }
                if toplnum == 0 || lnum < toplnum {
                    toplnum = lnum;
                }
                if botlnum == 0 || lnum >= botlnum {
                    botlnum = lnum + 1;
                }
                i += 1;
                li = (*li).li_next;
            }

            if failed {
                return -1;
            }

            if toplnum != 0 {
                redraw_win_range_later(wp, toplnum, botlnum);
                m.mit_toplnum = toplnum;
                m.mit_botlnum = botlnum;
                rtype = UPD_VALID;
            }
        }

        let insert_idx = (*wp)
            .w_match_head
            .iter()
            .position(|cur| prio < cur.mit_priority)
            .unwrap_or((*wp).w_match_head.len());
        (*wp).w_match_head.insert(insert_idx, m);

        redraw_later(wp, rtype);
        id
    }
}

/// Delete match with ID `id` in the match list of window `wp` (`match_delete`).
unsafe fn match_delete(wp: *mut win_T, id: c_int, perr: bool) -> c_int {
    unsafe {
        let mut rtype = UPD_SOME_VALID;

        if id < 1 {
            if perr {
                semsg!(
                    gettext(
                        c"E802: Invalid ID: %lld (must be greater than or equal to 1)".as_ptr(),
                    ),
                    i64::from(id),
                );
            }
            return -1;
        }
        let Some(idx) = (*wp).w_match_head.iter().position(|cur| cur.mit_id == id) else {
            if perr {
                semsg!(gettext(c"E803: ID not found: %lld".as_ptr()), i64::from(id));
            }
            return -1;
        };
        let cur = (*wp).w_match_head.remove(idx);
        if cur.mit_toplnum != 0 {
            redraw_win_range_later(wp, cur.mit_toplnum, cur.mit_botlnum);
            rtype = UPD_VALID;
        }
        redraw_later(wp, rtype);
        0
    }
}

/// Delete all matches in the match list of window `wp` (`clear_matches`).
pub unsafe fn clear_matches(wp: *mut win_T) {
    unsafe {
        (*wp).w_match_head.clear();
        redraw_later(wp, UPD_SOME_VALID);
    }
}

/// Get match from ID `id` in window `wp` (`get_match`).
unsafe fn get_match(wp: *mut win_T, id: c_int) -> *mut matchitem_T {
    unsafe {
        (*wp)
            .w_match_head
            .iter_mut()
            .find(|cur| cur.mit_id == id)
            .map_or(ptr::null_mut(), |cur| &raw mut *cur)
    }
}

/// Init for calling `prepare_search_hl()` (`init_search_hl`).
pub unsafe fn init_search_hl(wp: *mut win_T, search_hl: &mut match_T) {
    unsafe {
        let buf = (*wp).w_buffer;
        for cur in &mut (*wp).w_match_head {
            cur.mit_hl.rm = cur.mit_match;
            cur.mit_hl.attr = if cur.mit_hlg_id == 0 {
                0
            } else {
                syn_id2attr(cur.mit_hlg_id)
            };
            cur.mit_hl.buf = buf;
            cur.mit_hl.lnum = 0;
            cur.mit_hl.first_lnum = 0;
            cur.mit_hl.tm = profile_setlimit(opt_vars().p_rdt);
        }
        search_hl.buf = (*wp).w_buffer;
        search_hl.lnum = 0;
        search_hl.first_lnum = 0;
        search_hl.attr = win_hl_attr(wp, HLF_L as c_int);
    }
}

unsafe fn next_search_hl_pos(
    shl: &mut match_T,
    lnum: linenr_T,
    match_item: *mut matchitem_T,
    mincol: colnr_T,
) -> c_int {
    unsafe {
        let mut found: isize = -1;

        shl.lnum = 0;
        let mit_pos = &mut (*match_item).mit_pos;
        let pos_count = mit_pos.len() as c_int;
        for i in (*match_item).mit_pos_cur..pos_count {
            let pos = mit_pos[i as usize];
            if pos.lnum == 0 {
                break;
            }
            if pos.len == 0 && pos.col < mincol {
                continue;
            }
            if pos.lnum == lnum {
                if found >= 0 {
                    if pos.col < mit_pos[found as usize].col {
                        mit_pos.swap(i as usize, found as usize);
                    }
                } else {
                    found = i as isize;
                }
            }
        }
        (*match_item).mit_pos_cur = 0;
        if found >= 0 {
            let found_pos = mit_pos[found as usize];
            let start = if found_pos.col == 0 {
                0
            } else {
                found_pos.col - 1
            };
            let end = if found_pos.col == 0 {
                MAXCOL
            } else {
                start + found_pos.len
            };

            shl.lnum = lnum;
            shl.rm.startpos[0].lnum = 0;
            shl.rm.startpos[0].col = start;
            shl.rm.endpos[0].lnum = 0;
            shl.rm.endpos[0].col = end;
            shl.is_addpos = true;
            shl.has_cursor = false;
            (*match_item).mit_pos_cur = (found as c_int) + 1;
            return 1;
        }
        0
    }
}

/// Search for a next `'hlsearch'` or match (`next_search_hl`).
unsafe fn next_search_hl(
    win: *mut win_T,
    search_hl: &mut match_T,
    shl: &mut match_T,
    lnum: linenr_T,
    mincol: colnr_T,
    cur: *mut matchitem_T,
) {
    unsafe {
        let mut matchcol: colnr_T;
        let mut nmatched: c_int = 0;
        let called_emsg_before = g_st().called_emsg;

        if (lnum < g_st().Search.first_line || lnum > g_st().Search.last_line) && cur.is_null() {
            shl.lnum = 0;
            return;
        }

        if shl.lnum != 0 {
            let l = shl.lnum + shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum;
            if lnum > l {
                shl.lnum = 0;
            } else if lnum < l || shl.rm.endpos[0].col > mincol {
                return;
            }
        }

        loop {
            if profile_passed_limit(shl.tm) {
                shl.lnum = 0;
                break;
            }
            if shl.lnum == 0 {
                matchcol = 0;
            } else if vim_strchr(opt_vars().p_cpo, kCpoSearch).is_null()
                || (shl.rm.endpos[0].lnum == 0
                    && shl.rm.endpos[0].col <= shl.rm.startpos[0].col)
            {
                matchcol = shl.rm.startpos[0].col;
                let ml = ml_get_buf(shl.buf, lnum).add(matchcol as usize);
                if *ml == NUL {
                    shl.lnum = 0;
                    break;
                }
                matchcol += utfc_ptr2len(ml);
            } else {
                matchcol = shl.rm.endpos[0].col;
            }

            shl.lnum = lnum;
            if !shl.rm.regprog.is_null() {
                let regprog_is_copy = !ptr::eq(shl, search_hl)
                    && !cur.is_null()
                    && ptr::eq(shl, &raw const (*cur).mit_hl)
                    && (*cur).mit_match.regprog == (*cur).mit_hl.rm.regprog;
                let mut timed_out: c_int = 0;

                nmatched = vim_regexec_multi(
                    &mut shl.rm,
                    win,
                    shl.buf,
                    lnum,
                    matchcol,
                    Some(shl.tm),
                    &raw mut timed_out,
                );
                if regprog_is_copy {
                    (*cur).mit_match.regprog = (*cur).mit_hl.rm.regprog;
                }
                if g_st().called_emsg > called_emsg_before || g_st().got_int || timed_out != 0 {
                    if ptr::eq(shl, search_hl) {
                        vim_regfree(shl.rm.regprog);
                        set_no_hlsearch(true);
                    }
                    shl.rm.regprog = ptr::null_mut();
                    shl.lnum = 0;
                    g_st().got_int = false;
                    break;
                }
            } else if !cur.is_null() {
                nmatched = next_search_hl_pos(shl, lnum, cur, matchcol);
            }
            if nmatched == 0 {
                shl.lnum = 0;
                break;
            }
            if shl.rm.startpos[0].lnum > 0
                || shl.rm.startpos[0].col >= mincol
                || nmatched > 1
                || shl.rm.endpos[0].col > mincol
            {
                shl.lnum += shl.rm.startpos[0].lnum;
                break;
            }
        }
    }
}

/// Advance to the match in window `wp` line `lnum` or past it (`prepare_search_hl`).
pub unsafe fn prepare_search_hl(wp: *mut win_T, search_hl: &mut match_T, lnum: linenr_T) {
    unsafe {
        let mut idx = 0usize;
        let len = (*wp).w_match_head.len();
        let mut shl: *mut match_T;
        let mut shl_flag = false;

        while idx < len || !shl_flag {
            let cur: *mut matchitem_T = if idx < len {
                &raw mut (&mut (*wp).w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            let search_hl_ptr = &raw mut *search_hl;
            if !shl_flag {
                shl = search_hl_ptr;
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            if !(*shl).rm.regprog.is_null()
                && (*shl).lnum == 0
                && re_multiline((*shl).rm.regprog) != 0
            {
                if (*shl).first_lnum == 0 {
                    (*shl).first_lnum = lnum;
                    while (*shl).first_lnum > (*wp).w_topline {
                        if hasFolding(wp, (*shl).first_lnum - 1, ptr::null_mut(), ptr::null_mut()) {
                            break;
                        }
                        (*shl).first_lnum -= 1;
                    }
                }
                if !cur.is_null() {
                    (*cur).mit_pos_cur = 0;
                }
                let mut pos_inprogress = true;
                let mut n: colnr_T = 0;
                while (*shl).first_lnum < lnum
                    && (!(*shl).rm.regprog.is_null() || (!cur.is_null() && pos_inprogress))
                {
                    next_search_hl(
                        wp,
                        &mut *search_hl_ptr,
                        &mut *shl,
                        (*shl).first_lnum,
                        n,
                        if ptr::eq(shl, search_hl_ptr) {
                            ptr::null_mut()
                        } else {
                            cur
                        },
                    );
                    pos_inprogress = !cur.is_null() && (*cur).mit_pos_cur != 0;
                    if (*shl).lnum != 0 {
                        (*shl).first_lnum =
                            (*shl).lnum + (*shl).rm.endpos[0].lnum - (*shl).rm.startpos[0].lnum;
                        n = (*shl).rm.endpos[0].col;
                    } else {
                        (*shl).first_lnum += 1;
                        n = 0;
                    }
                }
            }
            if !ptr::eq(shl, search_hl_ptr) && !cur.is_null() {
                idx += 1;
            }
        }
    }
}

/// Update `shl->has_cursor` based on the match in `shl` and the cursor position (`check_cur_search_hl`).
unsafe fn check_cur_search_hl(wp: *mut win_T, shl: &mut match_T) {
    unsafe {
        let linecount = shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum;
        shl.has_cursor = (*wp).w_cursor.lnum >= shl.lnum
            && (*wp).w_cursor.lnum <= shl.lnum + linecount
            && ((*wp).w_cursor.lnum > shl.lnum
                || (*wp).w_cursor.col >= shl.rm.startpos[0].col)
            && ((*wp).w_cursor.lnum < shl.lnum + linecount
                || (*wp).w_cursor.col < shl.rm.endpos[0].col);
    }
}

/// Prepare for `'hlsearch'` and match highlighting in one window line (`prepare_search_hl_line`).
pub unsafe fn prepare_search_hl_line(
    wp: *mut win_T,
    lnum: linenr_T,
    mincol: colnr_T,
    line: *mut *mut c_char,
    search_hl: &mut match_T,
    search_attr: *mut c_int,
    search_attr_from_match: *mut bool,
) -> bool {
    unsafe {
        let mut idx = 0usize;
        let len = (*wp).w_match_head.len();
        let mut shl: *mut match_T;
        let mut shl_flag = false;
        let mut area_highlighting = false;

        while idx < len || !shl_flag {
            let cur: *mut matchitem_T = if idx < len {
                &raw mut (&mut (*wp).w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            let search_hl_ptr = &raw mut *search_hl;
            if !shl_flag {
                shl = search_hl_ptr;
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            (*shl).startcol = MAXCOL;
            (*shl).endcol = MAXCOL;
            (*shl).attr_cur = 0;
            (*shl).is_addpos = false;
            (*shl).has_cursor = false;
            if !cur.is_null() {
                (*cur).mit_pos_cur = 0;
            }
            next_search_hl(
                wp,
                &mut *search_hl_ptr,
                &mut *shl,
                lnum,
                mincol,
                if ptr::eq(shl, search_hl_ptr) {
                    ptr::null_mut()
                } else {
                    cur
                },
            );

            *line = ml_get_buf((*wp).w_buffer, lnum);

            if (*shl).lnum != 0 && (*shl).lnum <= lnum {
                (*shl).startcol = if (*shl).lnum == lnum {
                    (*shl).rm.startpos[0].col
                } else {
                    0
                };
                (*shl).endcol = if lnum
                    == (*shl).lnum + (*shl).rm.endpos[0].lnum - (*shl).rm.startpos[0].lnum
                {
                    (*shl).rm.endpos[0].col
                } else {
                    MAXCOL
                };

                if ptr::eq(shl, search_hl) {
                    check_cur_search_hl(wp, &mut *shl);
                }

                if (*shl).startcol == (*shl).endcol {
                    let p = (*line).add((*shl).endcol as usize);
                    if *p != NUL {
                        (*shl).endcol += utfc_ptr2len(p);
                    } else {
                        (*shl).endcol += 1;
                    }
                }
                if (*shl).startcol < mincol {
                    (*shl).attr_cur = (*shl).attr;
                    *search_attr = (*shl).attr;
                    *search_attr_from_match = shl != search_hl;
                }
                area_highlighting = true;
            }
            if !ptr::eq(shl, search_hl_ptr) && !cur.is_null() {
                idx += 1;
            }
        }
        area_highlighting
    }
}

/// For a position in a line: Check for start/end of `'hlsearch'` and other matches (`update_search_hl`).
pub unsafe fn update_search_hl(
    wp: *mut win_T,
    lnum: linenr_T,
    col: colnr_T,
    line: *mut *mut c_char,
    search_hl: &mut match_T,
    has_match_conc: *mut c_int,
    match_conc: *mut c_int,
    lcs_eol_todo: bool,
    on_last_col: *mut bool,
    search_attr_from_match: *mut bool,
) -> c_int {
    unsafe {
        let mut idx = 0usize;
        let len = (*wp).w_match_head.len();
        let search_hl_ptr = &raw mut *search_hl;
        let mut shl: *mut match_T;
        let mut shl_flag = false;

        while idx < len || !shl_flag {
            let cur: *mut matchitem_T = if idx < len {
                &raw mut (&mut (*wp).w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag && (cur.is_null() || (*cur).mit_priority > SEARCH_HL_PRIORITY) {
                shl = search_hl_ptr;
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            if !cur.is_null() {
                (*cur).mit_pos_cur = 0;
            }
            let mut pos_inprogress = true;
            while !(*shl).rm.regprog.is_null() || (!cur.is_null() && pos_inprogress) {
                if (*shl).startcol != MAXCOL && col >= (*shl).startcol && col < (*shl).endcol {
                    let next_col = col + utfc_ptr2len((*line).add(col as usize));
                    if (*shl).endcol < next_col {
                        (*shl).endcol = next_col;
                    }
                    if ptr::eq(shl, search_hl_ptr) && (*shl).has_cursor {
                        (*shl).attr_cur = win_hl_attr(wp, HLF_LC as c_int);
                        if (*shl).attr_cur != (*shl).attr {
                            g_st().search_hl_has_cursor_lnum = lnum;
                        }
                    } else {
                        (*shl).attr_cur = (*shl).attr;
                    }
                    if !cur.is_null()
                        && !ptr::eq(shl, search_hl_ptr)
                        && syn_name2id(c"Conceal".as_ptr()) == (*cur).mit_hlg_id
                    {
                        *has_match_conc = if col == (*shl).startcol { 2 } else { 1 };
                        *match_conc = (*cur).mit_conceal_char;
                    } else {
                        *has_match_conc = 0;
                    }
                } else if col == (*shl).endcol {
                    (*shl).attr_cur = 0;

                    next_search_hl(
                        wp,
                        &mut *search_hl_ptr,
                        &mut *shl,
                        lnum,
                        col,
                        if ptr::eq(shl, search_hl_ptr) {
                            ptr::null_mut()
                        } else {
                            cur
                        },
                    );
                    pos_inprogress = !cur.is_null() && (*cur).mit_pos_cur != 0;

                    *line = ml_get_buf((*wp).w_buffer, lnum);

                    if (*shl).lnum == lnum {
                        (*shl).startcol = (*shl).rm.startpos[0].col;
                        (*shl).endcol = if (*shl).rm.endpos[0].lnum == 0 {
                            (*shl).rm.endpos[0].col
                        } else {
                            MAXCOL
                        };

                        if ptr::eq(shl, search_hl_ptr) {
                            check_cur_search_hl(wp, &mut *shl);
                        }

                        if (*shl).startcol == (*shl).endcol {
                            let p = (*line).add((*shl).endcol as usize);
                            if *p == NUL {
                                (*shl).endcol += 1;
                            } else {
                                (*shl).endcol += utfc_ptr2len(p);
                            }
                        }
                        continue;
                    }
                }
                break;
            }
            if !ptr::eq(shl, search_hl_ptr) && !cur.is_null() {
                idx += 1;
            }
        }

        *search_attr_from_match = false;
        let mut search_attr = (*search_hl_ptr).attr_cur;
        idx = 0;
        shl_flag = false;
        while idx < len || !shl_flag {
            let cur: *mut matchitem_T = if idx < len {
                &raw mut (&mut (*wp).w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag && (cur.is_null() || (*cur).mit_priority > SEARCH_HL_PRIORITY) {
                shl = search_hl_ptr;
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            if (*shl).attr_cur != 0 {
                search_attr = (*shl).attr_cur;
                *on_last_col = col + 1 >= (*shl).endcol;
                *search_attr_from_match = !ptr::eq(shl, search_hl_ptr);
            }
            if !ptr::eq(shl, search_hl_ptr) && !cur.is_null() {
                idx += 1;
            }
        }
        if *(*line).add(col as usize) == NUL && ((*wp).w_onebuf_opt.wo_list != 0 && !lcs_eol_todo) {
            search_attr = 0;
        }
        search_attr
    }
}

pub unsafe fn get_prevcol_hl_flag(
    wp: *mut win_T,
    search_hl: &mut match_T,
    curcol: colnr_T,
) -> bool {
    unsafe {
        let mut prevcol = curcol;
        let skip_or_left = if (*wp).w_onebuf_opt.wo_wrap != 0 {
            (*wp).w_skipcol
        } else {
            (*wp).w_leftcol
        };
        if skip_or_left > prevcol {
            prevcol += 1;
        }

        if !search_hl.is_addpos
            && (prevcol == search_hl.startcol
                || (prevcol > search_hl.startcol && search_hl.endcol == MAXCOL))
        {
            return true;
        }
        for cur in &(*wp).w_match_head {
            if !cur.mit_hl.is_addpos
                && (prevcol == cur.mit_hl.startcol
                    || (prevcol > cur.mit_hl.startcol && cur.mit_hl.endcol == MAXCOL))
            {
                return true;
            }
        }
        false
    }
}

/// Get highlighting for the char after the text in `char_attr` (`get_search_match_hl`).
pub unsafe fn get_search_match_hl(
    wp: *mut win_T,
    search_hl: &mut match_T,
    col: colnr_T,
    char_attr: *mut c_int,
) {
    unsafe {
        let mut idx = 0usize;
        let len = (*wp).w_match_head.len();
        let search_hl_ptr = &raw mut *search_hl;
        let mut shl: *mut match_T;
        let mut shl_flag = false;

        while idx < len || !shl_flag {
            let cur: *mut matchitem_T = if idx < len {
                &raw mut (&mut (*wp).w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag && (cur.is_null() || (*cur).mit_priority > SEARCH_HL_PRIORITY) {
                shl = search_hl_ptr;
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            if col - 1 == (*shl).startcol && (ptr::eq(shl, search_hl_ptr) || !(*shl).is_addpos) {
                *char_attr = (*shl).attr;
            }
            if !ptr::eq(shl, search_hl_ptr) && !cur.is_null() {
                idx += 1;
            }
        }
    }
}

unsafe fn matchadd_dict_arg(
    tv: *mut typval_T,
    conceal_char: *mut *const c_char,
    win: *mut *mut win_T,
) -> c_int {
    unsafe {
        if (*tv).v_type != VarType::VAR_DICT {
            emsg(gettext((&raw const e_dictreq).cast()));
            return FAIL;
        }

        let di = tv_dict_find((*tv).vval.v_dict(), c"conceal".as_ptr(), 7);
        if !di.is_null() {
            *conceal_char = tv_get_string(&raw const (*di).di_tv);
        }

        let di = tv_dict_find((*tv).vval.v_dict(), c"window".as_ptr(), 6);
        if di.is_null() {
            return OK;
        }

        *win = find_win_by_nr_or_id(&raw mut (*di).di_tv);
        if (*win).is_null() {
            emsg(gettext((&raw const e_invalwindow).cast()));
            return FAIL;
        }

        OK
    }
}

/// `clearmatches()` function (`f_clearmatches`).
pub unsafe fn f_clearmatches(argvars: *mut typval_T, _rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let win = get_optional_window(argvars, 0);
        if !win.is_null() {
            clear_matches(win);
        }
    }
}

/// `getmatches()` function (`f_getmatches`).
pub unsafe fn f_getmatches(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let win = get_optional_window(argvars, 0);

        tv_list_alloc_ret(rettv, kListLenMayKnow);
        if win.is_null() {
            return;
        }

        for cur in &(*win).w_match_head {
            let dict = tv_dict_alloc();
            if cur.mit_match.regprog.is_null() {
                for (i, llpos) in cur.mit_pos.iter().enumerate() {
                    if llpos.lnum == 0 {
                        break;
                    }
                    let l = tv_list_alloc(1 + if llpos.col > 0 { 2 } else { 0 });
                    tv_list_append_number(l, llpos.lnum as varnumber_T);
                    if llpos.col > 0 {
                        tv_list_append_number(l, llpos.col as varnumber_T);
                        tv_list_append_number(l, llpos.len as varnumber_T);
                    }
                    let mut buf = [0 as c_char; 30];
                    let len =
                        vim_snprintf!(buf.as_mut_ptr(), 30, c"pos%d".as_ptr(), (i + 1) as c_int);
                    tv_dict_add_list(dict, buf.as_ptr(), len as usize, l);
                }
            } else {
                tv_dict_add_str(
                    dict,
                    c"pattern".as_ptr(),
                    7,
                    cur.mit_pattern.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                );
            }
            tv_dict_add_str(dict, c"group".as_ptr(), 5, syn_id2name(cur.mit_hlg_id));
            tv_dict_add_nr(
                dict,
                c"priority".as_ptr(),
                8,
                cur.mit_priority as varnumber_T,
            );
            tv_dict_add_nr(dict, c"id".as_ptr(), 2, cur.mit_id as varnumber_T);

            if cur.mit_conceal_char != 0 {
                let mut buf = [0 as c_char; MB_MAXCHAR + 1];
                let buflen = utf_char2bytes(cur.mit_conceal_char, buf.as_mut_ptr());
                buf[buflen as usize] = NUL;
                tv_dict_add_str_len(dict, c"conceal".as_ptr(), 7, buf.as_ptr(), buflen);
            }

            tv_list_append_dict(rettv.vval.v_list(), dict);
        }
    }
}

/// `setmatches()` function (`f_setmatches`).
pub unsafe fn f_setmatches(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut s: *mut list_T = ptr::null_mut();
        let win = get_optional_window(argvars, 1);

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(gettext((&raw const e_listreq).cast()));
            return;
        }
        if win.is_null() {
            return;
        }

        let l = (*argvars).vval.v_list();
        let mut li_idx: c_int = 0;
        let mut li = tv_list_first(l);
        while !li.is_null() {
            if (*li).li_tv.v_type != VarType::VAR_DICT || (*li).li_tv.vval.v_dict().is_null() {
                semsg!(
                    gettext(
                        c"E474: List item %d is either not a dictionary or an empty one".as_ptr(),
                    ),
                    li_idx,
                );
                return;
            }
            let d = (*li).li_tv.vval.v_dict();
            if tv_dict_find(d, c"group".as_ptr(), 5).is_null()
                || (tv_dict_find(d, c"pattern".as_ptr(), 7).is_null()
                    && tv_dict_find(d, c"pos1".as_ptr(), 4).is_null())
                || tv_dict_find(d, c"priority".as_ptr(), 8).is_null()
                || tv_dict_find(d, c"id".as_ptr(), 2).is_null()
            {
                semsg!(
                    gettext(c"E474: List item %d is missing one of the required keys".as_ptr()),
                    li_idx,
                );
                return;
            }
            li_idx += 1;
            li = (*li).li_next;
        }

        clear_matches(win);
        let mut match_add_failed = false;
        let mut li = tv_list_first(l);
        while !li.is_null() {
            let mut i: c_int = 0;
            let d = (*li).li_tv.vval.v_dict();
            let di = tv_dict_find(d, c"pattern".as_ptr(), 7);
            if di.is_null() {
                if s.is_null() {
                    s = tv_list_alloc(9);
                    tv_list_ref(s);
                }

                i = 1;
                while i < 9 {
                    let mut buf = [0 as c_char; 30];
                    vim_snprintf!(buf.as_mut_ptr(), 30, c"pos%d".as_ptr(), i);
                    let pos_di = tv_dict_find(d, buf.as_ptr(), -1);
                    if !pos_di.is_null() {
                        if (*pos_di).di_tv.v_type != VarType::VAR_LIST {
                            tv_list_unref(s);
                            return;
                        }
                        tv_list_append_tv(s, &raw mut (*pos_di).di_tv);
                    } else {
                        break;
                    }
                    i += 1;
                }
            }

            let mut group_buf = [0 as c_char; NUMBUFLEN];
            let group = tv_dict_get_string_buf(d, c"group".as_ptr(), group_buf.as_mut_ptr());
            let priority = tv_dict_get_number(d, c"priority".as_ptr()) as c_int;
            let id = tv_dict_get_number(d, c"id".as_ptr()) as c_int;
            let conceal_di = tv_dict_find(d, c"conceal".as_ptr(), 7);
            let conceal = if !conceal_di.is_null() {
                tv_get_string(&raw const (*conceal_di).di_tv)
            } else {
                ptr::null()
            };
            if i == 0 {
                let pat = tv_dict_get_string(d, c"pattern".as_ptr());
                if match_add(win, group, pat, priority, id, ptr::null_mut(), conceal) != id {
                    match_add_failed = true;
                }
            } else {
                if match_add(win, group, ptr::null(), priority, id, s, conceal) != id {
                    match_add_failed = true;
                }
                tv_list_unref(s);
                s = ptr::null_mut();
            }
            li = (*li).li_next;
        }
        if !match_add_failed {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
    }
}

/// `matchadd()` function (`f_matchadd`).
pub unsafe fn f_matchadd(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut grpbuf = [0 as c_char; NUMBUFLEN];
        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let grp = tv_get_string_buf_chk(argvars, grpbuf.as_mut_ptr());
        let pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
        let mut prio: c_int = 10;
        let mut id: c_int = -1;
        let mut error = false;
        let mut conceal_char: *const c_char = ptr::null();
        let mut win = *curwin.as_ptr();

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if grp.is_null() || pat.is_null() {
            return;
        }
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            prio = tv_get_number_chk(argvars.add(2), &mut error) as c_int;
            if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                id = tv_get_number_chk(argvars.add(3), &mut error) as c_int;
                if (*argvars.add(4)).v_type != VarType::VAR_UNKNOWN
                    && matchadd_dict_arg(argvars.add(4), &raw mut conceal_char, &raw mut win)
                        == FAIL
                {
                    return;
                }
            }
        }
        if error {
            return;
        }
        if (1..=3).contains(&id) {
            semsg!(
                gettext(c"E798: ID is reserved for \":match\": %d".as_ptr()),
                id,
            );
            return;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(match_add(win, grp, pat, prio, id, ptr::null_mut(), conceal_char) as varnumber_T);
    }
}

/// `matchaddpos()` function (`f_matchaddpos`).
pub unsafe fn f_matchaddpos(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        let mut buf = [0 as c_char; NUMBUFLEN];
        let group = tv_get_string_buf_chk(argvars, buf.as_mut_ptr());
        if group.is_null() {
            return;
        }

        if (*argvars.add(1)).v_type != VarType::VAR_LIST {
            semsg!(
                gettext((&raw const e_listarg).cast()),
                c"matchaddpos()".as_ptr(),
            );
            return;
        }

        let l = (*argvars.add(1)).vval.v_list();
        if tv_list_len(l) == 0 {
            return;
        }

        let mut error = false;
        let mut prio: c_int = 10;
        let mut id: c_int = -1;
        let mut conceal_char: *const c_char = ptr::null();
        let mut win = *curwin.as_ptr();

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            prio = tv_get_number_chk(argvars.add(2), &mut error) as c_int;
            if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                id = tv_get_number_chk(argvars.add(3), &mut error) as c_int;
                if (*argvars.add(4)).v_type != VarType::VAR_UNKNOWN
                    && matchadd_dict_arg(argvars.add(4), &raw mut conceal_char, &raw mut win)
                        == FAIL
                {
                    return;
                }
            }
        }
        if error {
            return;
        }

        if id == 1 || id == 2 {
            semsg!(
                gettext(c"E798: ID is reserved for \"match\": %d".as_ptr()),
                id,
            );
            return;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(match_add(win, group, ptr::null(), prio, id, l, conceal_char) as varnumber_T);
    }
}

/// `matcharg()` function (`f_matcharg`).
pub unsafe fn f_matcharg(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let id = tv_get_number(argvars) as c_int;

        tv_list_alloc_ret(rettv, if (1..=3).contains(&id) { 2 } else { 0 });

        if (1..=3).contains(&id) {
            let m = get_match(*curwin.as_ptr(), id);
            if !m.is_null() {
                tv_list_append_string(rettv.vval.v_list(), syn_id2name((*m).mit_hlg_id), -1);
                tv_list_append_string(
                    rettv.vval.v_list(),
                    (*m).mit_pattern
                        .as_ref()
                        .map_or(ptr::null(), |p| p.as_ptr()),
                    -1,
                );
            } else {
                tv_list_append_string(rettv.vval.v_list(), ptr::null(), 0);
                tv_list_append_string(rettv.vval.v_list(), ptr::null(), 0);
            }
        }
    }
}

/// `matchdelete()` function (`f_matchdelete`).
pub unsafe fn f_matchdelete(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let win = get_optional_window(argvars, 1);
        rettv.vval = crate::eval::typval::TypvalVval::Number(if win.is_null() {
            -1
        } else {
            match_delete(win, tv_get_number(argvars) as c_int, true) as varnumber_T
        });
    }
}

/// `:[N]match {group} {pattern}` (`ex_match`).
pub unsafe fn ex_match(eap: &mut exarg_T) {
    unsafe {
        let mut g = None;
        let end: *mut c_char;

        let id: c_int = if eap.line2 <= 3 {
            eap.line2 as c_int
        } else {
            emsg((&raw const e_invcmd).cast());
            return;
        };

        if eap.skip == 0 {
            match_delete(*curwin.as_ptr(), id, false);
        }

        let arg = eap.arg;
        if ends_excmd(c_int::from(*arg)) != 0 {
            end = arg;
        } else {
            let arg_bytes = CStr::from_ptr(arg).to_bytes();
            if arg_bytes.len() >= 4
                && arg_bytes[..4].eq_ignore_ascii_case(b"none")
                && (ascii_iswhite(*arg.add(4)) || ends_excmd(c_int::from(*arg.add(4))) != 0)
            {
                end = arg.add(4);
            } else {
                let mut p = skiptowhite(arg);
                if eap.skip == 0 {
                    g = Some(
                        std::ffi::CString::new(std::slice::from_raw_parts(
                            arg.cast::<u8>(),
                            p.offset_from(arg) as usize,
                        ))
                        .unwrap_or_default(),
                    );
                }
                p = skipwhite(p);
                if *p == NUL {
                    semsg!(gettext((&raw const e_invarg2).cast()), arg);
                    return;
                }
                end = skip_regexp(p.add(1), c_int::from(*p as u8), 1);
                if eap.skip == 0 {
                    if *end != NUL && ends_excmd(c_int::from(*skipwhite(end.add(1)))) == 0 {
                        eap.errmsg = ex_errmsg((&raw const e_trailing_arg).cast(), end);
                        return;
                    }
                    if *end != *p {
                        semsg!(gettext((&raw const e_invarg2).cast()), p);
                        return;
                    }

                    let c = *end;
                    *end = NUL;
                    match_add(
                        *curwin.as_ptr(),
                        g.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                        p.add(1),
                        10,
                        id,
                        ptr::null_mut(),
                        ptr::null(),
                    );
                    *end = c;
                }
            }
        }
        eap.nextcmd = find_nextcmd(end);
    }
}
