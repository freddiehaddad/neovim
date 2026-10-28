//! Functions for highlighting matches (`src/nvim/match.c`).

use crate::buffer::FileBuf;
use crate::charset::{NUL, ascii_iswhite, skiptowhite, skipwhite};
use crate::drawscreen::{UPD_SOME_VALID, UPD_VALID, redraw_later, redraw_win_range_later};
use crate::errors::*;
use crate::eval::funcs::get_optional_window;
use crate::eval::typval::{
    EvalFuncData, TvList, TypVal, VarType, Varnumber, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_dict_find, tv_dict_get_number,
    tv_dict_get_string, tv_dict_get_string_buf, tv_get_number, tv_get_number_chk, tv_get_string,
    tv_get_string_buf_chk, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_number, tv_list_append_string, tv_list_append_tv, tv_list_first,
    tv_list_idx_of_item, tv_list_len, tv_list_ref, tv_list_unref,
};
use crate::eval::window::find_win_by_nr_or_id;
use crate::ex_docmd::{Exarg, ends_excmd, ex_errmsg, find_nextcmd, set_no_hlsearch};
use crate::fold::has_folding;
use crate::globals::*;
use crate::highlight_group::{syn_check_group, syn_id2attr, syn_id2name, syn_name2id};
use crate::mbyte::{utf_char2bytes, utf_ptr2char, utfc_ptr2len};
use crate::memline::ml_get_buf;
use crate::message::{emsg, semsg};
use crate::option_vars::*;
use crate::profile::{Proftime, profile_passed_limit, profile_setlimit};
use crate::regexp::{
    MAXCOL, NUMBUFLEN, RE_MAGIC, Regmmatch, Regprog, re_multiline, skip_regexp, vim_regcomp,
    vim_regexec_multi, vim_regfree,
};
use crate::strings::{gettext, vim_snprintf};
use crate::window::WinInfoInfo;
use std::ffi::c_char;
use std::ptr;

use crate::change::MB_MAXCHAR;
use crate::cursor::{Colnr, Linenr};
use crate::eval::K_LIST_LEN_MAY_KNOW_CONST;

const SEARCH_HL_PRIORITY: i32 = 0;

pub(crate) const HLF_L: usize = 8;
pub(crate) const HLF_LC: usize = 9;
pub(crate) const K_CPO_SEARCH: i32 = b'c' as i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SynMatch {
    pub rm: Regmmatch,
    pub buf: *mut FileBuf,
    pub lnum: Linenr,
    pub attr: i32,
    pub attr_cur: i32,
    pub first_lnum: Linenr,
    pub startcol: Colnr,
    pub endcol: Colnr,
    pub is_addpos: bool,
    pub has_cursor: bool,
    pub tm: Proftime,
}

impl SynMatch {
    pub const fn new() -> Self {
        Self {
            rm: Regmmatch::new(),
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

impl Default for SynMatch {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Llpos {
    pub lnum: Linenr,
    pub col: Colnr,
    pub len: i32,
}

#[repr(C)]
pub struct Matchitem {
    pub mit_id: i32,
    pub mit_priority: i32,
    pub mit_pattern: Option<std::ffi::CString>,
    pub mit_match: Regmmatch,
    pub mit_pos: Vec<Llpos>,
    pub mit_pos_cur: i32,
    pub mit_toplnum: Linenr,
    pub mit_botlnum: Linenr,
    pub mit_hl: SynMatch,
    pub mit_hlg_id: i32,
    pub mit_conceal_char: i32,
}

impl Drop for Matchitem {
    fn drop(&mut self) {
        vim_regfree(self.mit_match.regprog);
    }
}

#[repr(C)]
pub struct SearchState {
    pub hl_match: bool,
    pub match_lines: Linenr,
    pub match_endcol: Colnr,
    pub first_line: Linenr,
    pub last_line: Linenr,
    pub no_smartcase: bool,
    pub cmdlen: i32,
    pub no_hlsearch: bool,
}

use crate::highlight::win_hl_attr;

/// Add match to the match list of window `wp` (`match_add`).
fn match_add(
    wp: &mut WinInfoInfo,
    grp: impl crate::strings::IntoCCharPtr,
    pat: impl crate::strings::IntoCCharPtr,
    prio: i32,
    mut id: i32,
    pos_list: Option<&TvList>,
    conceal_char: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let grp = grp.into_c_char_ptr();
    let pat = pat.into_c_char_ptr();
    let conceal_char = conceal_char.into_c_char_ptr();
    {
        let mut regprog: *mut Regprog = ptr::null_mut();
        let mut rtype = UPD_SOME_VALID;

        if crate::memory::cstr_is_empty(grp)
            || (!pat.is_null() && crate::memory::cstr_is_empty(pat))
        {
            return -1;
        }
        if id < -1 || id == 0 {
            semsg!(
                gettext(c"E799: Invalid ID: %lld (must be greater than or equal to 1)"),
                i64::from(id),
            );
            return -1;
        }
        if id == -1 {
            id = wp.w_next_match_id;
            wp.w_next_match_id += 1;
        } else {
            if wp.w_match_head.iter().any(|cur| cur.mit_id == id) {
                semsg!(gettext(c"E801: ID already taken: %lld"), i64::from(id),);
                return -1;
            }

            if wp.w_next_match_id < id + 100 {
                wp.w_next_match_id = id + 100;
            }
        }

        let grp_len = crate::memory::cstr_len(grp);
        let hlg_id = syn_check_group(grp, grp_len);
        if hlg_id == 0 {
            return -1;
        }
        if !pat.is_null() {
            regprog = vim_regcomp(pat, RE_MAGIC);
            if regprog.is_null() {
                semsg!(gettext(E_INVARG2), pat);
                return -1;
            }
        }

        let pos_len = tv_list_len(pos_list);
        let mut m = Matchitem {
            mit_id: id,
            mit_priority: prio,
            mit_pattern: (!pat.is_null()).then(|| crate::memory::cstr_to_cstring(pat)),
            mit_match: Regmmatch {
                regprog,
                startpos: [crate::cursor::Lpos { lnum: 0, col: 0 }; 10],
                endpos: [crate::cursor::Lpos { lnum: 0, col: 0 }; 10],
                rmm_matchcol: 0,
                rmm_ic: 0,
                rmm_maxcol: 0,
            },
            mit_pos: if pos_len > 0 {
                vec![Llpos::default(); pos_len as usize]
            } else {
                Vec::new()
            },
            mit_pos_cur: 0,
            mit_toplnum: 0,
            mit_botlnum: 0,
            mit_hl: SynMatch::new(),
            mit_hlg_id: hlg_id,
            mit_conceal_char: if conceal_char.is_null() {
                0
            } else {
                utf_ptr2char(conceal_char)
            },
        };

        if pos_list.is_some() {
            let mut toplnum: Linenr = 0;
            let mut botlnum: Linenr = 0;
            let mut i: usize = 0;
            let mut failed = false;

            for li in crate::eval::typval::list_iter_ptr(tv_list_first(pos_list)) {
                let mut lnum: Linenr = 0;
                let mut col: Colnr = 0;
                let mut len: i32 = 1;
                let mut error = false;

                if li.li_tv.v_type == VarType::VarList {
                    let subl = li.li_tv.vval.v_list();
                    let mut subli = tv_list_first(unsafe { subl.as_ref() });
                    if subli.is_null() {
                        semsg!(
                            gettext(c"E5030: Empty list at position %d"),
                            tv_list_idx_of_item(pos_list, li),
                        );
                        failed = true;
                        break;
                    }
                    lnum =
                        tv_get_number_chk(unsafe { &(*subli).li_tv }, Some(&mut error)) as Linenr;
                    if error {
                        failed = true;
                        break;
                    }
                    if lnum <= 0 {
                        continue;
                    }
                    m.mit_pos[i].lnum = lnum;
                    subli = unsafe { (*subli).li_next };
                    if !subli.is_null() {
                        col = tv_get_number_chk(unsafe { &(*subli).li_tv }, Some(&mut error))
                            as Colnr;
                        if error {
                            failed = true;
                            break;
                        }
                        if col < 0 {
                            continue;
                        }
                        subli = unsafe { (*subli).li_next };
                        if !subli.is_null() {
                            len = tv_get_number_chk(unsafe { &(*subli).li_tv }, Some(&mut error))
                                as Colnr;
                            if len < 0 {
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
                } else if li.li_tv.v_type == VarType::VarNumber {
                    if li.li_tv.vval.v_number() <= 0 {
                        continue;
                    }
                    m.mit_pos[i].lnum = li.li_tv.vval.v_number() as Linenr;
                    m.mit_pos[i].col = 0;
                    m.mit_pos[i].len = 0;
                } else {
                    semsg!(
                        gettext(c"E5031: List or number required at position %d"),
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

        let insert_idx = wp
            .w_match_head
            .iter()
            .position(|cur| prio < cur.mit_priority)
            .unwrap_or(wp.w_match_head.len());
        wp.w_match_head.insert(insert_idx, m);

        redraw_later(Some(&mut *wp), rtype);
        id
    }
}

/// Delete match with ID `id` in the match list of window `wp` (`match_delete`).
fn match_delete(wp: &mut WinInfoInfo, id: i32, perr: bool) -> i32 {
    let mut rtype = UPD_SOME_VALID;

    if id < 1 {
        if perr {
            semsg!(
                gettext(c"E802: Invalid ID: %lld (must be greater than or equal to 1)",),
                i64::from(id),
            );
        }
        return -1;
    }
    let Some(idx) = wp.w_match_head.iter().position(|cur| cur.mit_id == id) else {
        if perr {
            semsg!(gettext(c"E803: ID not found: %lld"), i64::from(id));
        }
        return -1;
    };
    let cur = wp.w_match_head.remove(idx);
    if cur.mit_toplnum != 0 {
        redraw_win_range_later(wp, cur.mit_toplnum, cur.mit_botlnum);
        rtype = UPD_VALID;
    }
    redraw_later(Some(&mut *wp), rtype);
    0
}

/// Delete all matches in the match list of window `wp` (`clear_matches`).
pub fn clear_matches(wp: &mut WinInfoInfo) {
    wp.w_match_head.clear();
    redraw_later(Some(&mut *wp), UPD_SOME_VALID);
}

/// Get match from ID `id` in window `wp` (`get_match`).
fn get_match(wp: &mut WinInfoInfo, id: i32) -> *mut Matchitem {
    wp.w_match_head
        .iter_mut()
        .find(|cur| cur.mit_id == id)
        .map_or(ptr::null_mut(), |cur| &raw mut *cur)
}

/// Init for calling `prepare_search_hl()` (`init_search_hl`).
pub fn init_search_hl(wp: &mut WinInfoInfo, search_hl: &mut SynMatch) {
    let buf = wp.w_buffer;
    for cur in &mut wp.w_match_head {
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
    search_hl.buf = wp.w_buffer;
    search_hl.lnum = 0;
    search_hl.first_lnum = 0;
    search_hl.attr = win_hl_attr(&mut *wp, HLF_L as i32);
}

fn next_search_hl_pos(
    shl: &mut SynMatch,
    lnum: Linenr,
    match_item: &mut Matchitem,
    mincol: Colnr,
) -> i32 {
    let mut found: isize = -1;

    shl.lnum = 0;
    let mit_pos = &mut match_item.mit_pos;
    let pos_count = mit_pos.len() as i32;
    for i in match_item.mit_pos_cur..pos_count {
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
    match_item.mit_pos_cur = 0;
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
        match_item.mit_pos_cur = (found as i32) + 1;
        return 1;
    }
    0
}

/// Search for a next `'hlsearch'` or match (`next_search_hl`).
fn next_search_hl(
    win: &mut WinInfoInfo,
    shl: &mut SynMatch,
    lnum: Linenr,
    mincol: Colnr,
    mut cur: Option<&mut Matchitem>,
) {
    let mut matchcol: Colnr;
    let mut nmatched: i32 = 0;
    let called_emsg_before = g_st().called_emsg;

    if (lnum < g_st().search_global.first_line || lnum > g_st().search_global.last_line)
        && cur.is_none()
    {
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
        } else if !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_SEARCH)
            || (shl.rm.endpos[0].lnum == 0 && shl.rm.endpos[0].col <= shl.rm.startpos[0].col)
        {
            matchcol = shl.rm.startpos[0].col;
            let ml = ml_get_buf(unsafe { &mut *shl.buf }, lnum).wrapping_add(matchcol as usize);
            if crate::memory::cstr_is_empty(ml) {
                shl.lnum = 0;
                break;
            }
            matchcol += utfc_ptr2len(ml);
        } else {
            matchcol = shl.rm.endpos[0].col;
        }

        shl.lnum = lnum;
        if !shl.rm.regprog.is_null() {
            let regprog_is_copy = cur.is_some()
                && ptr::eq(shl, &raw const cur.as_deref_mut().unwrap().mit_hl)
                && cur.as_deref_mut().unwrap().mit_match.regprog
                    == cur.as_deref_mut().unwrap().mit_hl.rm.regprog;
            let mut timed_out: i32 = 0;

            nmatched = vim_regexec_multi(
                &mut shl.rm,
                win,
                shl.buf,
                lnum,
                matchcol,
                Some(shl.tm),
                Some(&mut timed_out),
            );
            if regprog_is_copy {
                cur.as_deref_mut().unwrap().mit_match.regprog =
                    cur.as_deref_mut().unwrap().mit_hl.rm.regprog;
            }
            if g_st().called_emsg > called_emsg_before || g_st().got_int || timed_out != 0 {
                if cur.is_none() {
                    vim_regfree(shl.rm.regprog);
                    set_no_hlsearch(true);
                }
                shl.rm.regprog = ptr::null_mut();
                shl.lnum = 0;
                g_st().got_int = false;
                break;
            }
        } else if cur.is_some() {
            nmatched = next_search_hl_pos(shl, lnum, cur.as_deref_mut().unwrap(), matchcol);
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

/// Advance to the match in window `wp` line `lnum` or past it (`prepare_search_hl`).
pub fn prepare_search_hl(wp: &mut WinInfoInfo, search_hl: &mut SynMatch, lnum: Linenr) {
    unsafe {
        let mut idx = 0usize;
        let len = wp.w_match_head.len();
        let mut shl: *mut SynMatch;
        let mut shl_flag = false;

        while idx < len || !shl_flag {
            let cur: *mut Matchitem = if idx < len {
                &raw mut (&mut wp.w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag {
                shl = (&raw mut *search_hl);
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            let is_search_hl = ptr::eq(shl, &raw mut *search_hl);
            let shl = &mut *shl;
            if !shl.rm.regprog.is_null() && shl.lnum == 0 && re_multiline(&*shl.rm.regprog) != 0 {
                if shl.first_lnum == 0 {
                    shl.first_lnum = lnum;
                    while shl.first_lnum > wp.w_topline {
                        if has_folding(&raw mut *wp, shl.first_lnum - 1, None, ptr::null_mut()) {
                            break;
                        }
                        shl.first_lnum -= 1;
                    }
                }
                if !cur.is_null() {
                    (*cur).mit_pos_cur = 0;
                }
                let mut pos_inprogress = true;
                let mut n: Colnr = 0;
                while shl.first_lnum < lnum
                    && (!shl.rm.regprog.is_null() || (!cur.is_null() && pos_inprogress))
                {
                    next_search_hl(
                        wp,
                        shl,
                        shl.first_lnum,
                        n,
                        (if is_search_hl { ptr::null_mut() } else { cur }).as_mut(),
                    );
                    pos_inprogress = !cur.is_null() && (*cur).mit_pos_cur != 0;
                    if shl.lnum != 0 {
                        shl.first_lnum = shl.lnum + shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum;
                        n = shl.rm.endpos[0].col;
                    } else {
                        shl.first_lnum += 1;
                        n = 0;
                    }
                }
            }
            if !is_search_hl && !cur.is_null() {
                idx += 1;
            }
        }
    }
}

/// Update `shl->has_cursor` based on the match in `shl` and the cursor position (`check_cur_search_hl`).
fn check_cur_search_hl(wp: &mut WinInfoInfo, shl: &mut SynMatch) {
    let linecount = shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum;
    shl.has_cursor = wp.w_cursor.lnum >= shl.lnum
        && wp.w_cursor.lnum <= shl.lnum + linecount
        && (wp.w_cursor.lnum > shl.lnum || wp.w_cursor.col >= shl.rm.startpos[0].col)
        && (wp.w_cursor.lnum < shl.lnum + linecount || wp.w_cursor.col < shl.rm.endpos[0].col);
}

/// Prepare for `'hlsearch'` and match highlighting in one window line (`prepare_search_hl_line`).
pub fn prepare_search_hl_line(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    mincol: Colnr,
    line: *mut *mut c_char,
    search_hl: &mut SynMatch,
    search_attr: &mut i32,
    search_attr_from_match: &mut bool,
) -> bool {
    unsafe {
        let mut idx = 0usize;
        let len = wp.w_match_head.len();
        let mut shl: *mut SynMatch;
        let mut shl_flag = false;
        let mut area_highlighting = false;

        while idx < len || !shl_flag {
            let cur: *mut Matchitem = if idx < len {
                &raw mut (&mut wp.w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag {
                shl = (&raw mut *search_hl);
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            let is_search_hl = ptr::eq(shl, &raw mut *search_hl);
            let shl = &mut *shl;
            shl.startcol = MAXCOL;
            shl.endcol = MAXCOL;
            shl.attr_cur = 0;
            shl.is_addpos = false;
            shl.has_cursor = false;
            if !cur.is_null() {
                (*cur).mit_pos_cur = 0;
            }
            next_search_hl(
                &mut *wp,
                &mut *shl,
                lnum,
                mincol,
                (if is_search_hl { ptr::null_mut() } else { cur }).as_mut(),
            );

            *line = ml_get_buf(wp.buf_mut(), lnum);

            if shl.lnum != 0 && shl.lnum <= lnum {
                shl.startcol = if shl.lnum == lnum {
                    shl.rm.startpos[0].col
                } else {
                    0
                };
                shl.endcol = if lnum == shl.lnum + shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum {
                    shl.rm.endpos[0].col
                } else {
                    MAXCOL
                };

                if is_search_hl {
                    check_cur_search_hl(wp, &mut *shl);
                }

                if shl.startcol == shl.endcol {
                    let p = (*line).wrapping_add(shl.endcol as usize);
                    if !crate::memory::cstr_is_empty(p) {
                        shl.endcol += utfc_ptr2len(p);
                    } else {
                        shl.endcol += 1;
                    }
                }
                if shl.startcol < mincol {
                    shl.attr_cur = shl.attr;
                    *search_attr = shl.attr;
                    *search_attr_from_match = !is_search_hl;
                }
                area_highlighting = true;
            }
            if !is_search_hl && !cur.is_null() {
                idx += 1;
            }
        }
        area_highlighting
    }
}

/// For a position in a line: Check for start/end of `'hlsearch'` and other matches (`update_search_hl`).
pub fn update_search_hl(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    col: Colnr,
    line: *mut *mut c_char,
    search_hl: &mut SynMatch,
    has_match_conc: &mut i32,
    match_conc: &mut i32,
    lcs_eol_todo: bool,
    on_last_col: &mut bool,
    search_attr_from_match: &mut bool,
) -> i32 {
    unsafe {
        let mut idx = 0usize;
        let len = wp.w_match_head.len();
        let mut shl: *mut SynMatch;
        let mut shl_flag = false;

        while idx < len || !shl_flag {
            let cur: *mut Matchitem = if idx < len {
                &raw mut (&mut wp.w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag && (cur.is_null() || (*cur).mit_priority > SEARCH_HL_PRIORITY) {
                shl = (&raw mut *search_hl);
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            let is_search_hl = ptr::eq(shl, &raw mut *search_hl);
            let shl = &mut *shl;
            if !cur.is_null() {
                (*cur).mit_pos_cur = 0;
            }
            let mut pos_inprogress = true;
            while !shl.rm.regprog.is_null() || (!cur.is_null() && pos_inprogress) {
                if shl.startcol != MAXCOL && col >= shl.startcol && col < shl.endcol {
                    let next_col = col + utfc_ptr2len((*line).wrapping_add(col as usize));
                    if shl.endcol < next_col {
                        shl.endcol = next_col;
                    }
                    if is_search_hl && shl.has_cursor {
                        shl.attr_cur = win_hl_attr(&mut *wp, HLF_LC as i32);
                        if shl.attr_cur != shl.attr {
                            g_st().search_hl_has_cursor_lnum = lnum;
                        }
                    } else {
                        shl.attr_cur = shl.attr;
                    }
                    if !cur.is_null()
                        && !is_search_hl
                        && syn_name2id(c"Conceal") == (*cur).mit_hlg_id
                    {
                        *has_match_conc = if col == shl.startcol { 2 } else { 1 };
                        *match_conc = (*cur).mit_conceal_char;
                    } else {
                        *has_match_conc = 0;
                    }
                } else if col == shl.endcol {
                    shl.attr_cur = 0;

                    next_search_hl(
                        &mut *wp,
                        &mut *shl,
                        lnum,
                        col,
                        (if is_search_hl { ptr::null_mut() } else { cur }).as_mut(),
                    );
                    pos_inprogress = !cur.is_null() && (*cur).mit_pos_cur != 0;

                    *line = ml_get_buf(wp.buf_mut(), lnum);

                    if shl.lnum == lnum {
                        shl.startcol = shl.rm.startpos[0].col;
                        shl.endcol = if shl.rm.endpos[0].lnum == 0 {
                            shl.rm.endpos[0].col
                        } else {
                            MAXCOL
                        };

                        if is_search_hl {
                            check_cur_search_hl(wp, &mut *shl);
                        }

                        if shl.startcol == shl.endcol {
                            let p = (*line).wrapping_add(shl.endcol as usize);
                            if crate::memory::cstr_is_empty(p) {
                                shl.endcol += 1;
                            } else {
                                shl.endcol += utfc_ptr2len(p);
                            }
                        }
                        continue;
                    }
                }
                break;
            }
            if !is_search_hl && !cur.is_null() {
                idx += 1;
            }
        }

        *search_attr_from_match = false;
        let mut search_attr = search_hl.attr_cur;
        idx = 0;
        shl_flag = false;
        while idx < len || !shl_flag {
            let cur: *mut Matchitem = if idx < len {
                &raw mut (&mut wp.w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag && (cur.is_null() || (*cur).mit_priority > SEARCH_HL_PRIORITY) {
                shl = (&raw mut *search_hl);
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            let is_search_hl = ptr::eq(shl, &raw mut *search_hl);
            let shl = &mut *shl;
            if shl.attr_cur != 0 {
                search_attr = shl.attr_cur;
                *on_last_col = col + 1 >= shl.endcol;
                *search_attr_from_match = !is_search_hl;
            }
            if !is_search_hl && !cur.is_null() {
                idx += 1;
            }
        }
        if crate::memory::cstr_byte_at(*line, col as usize) == 0
            && (wp.w_onebuf_opt.wo_list != 0 && !lcs_eol_todo)
        {
            search_attr = 0;
        }
        search_attr
    }
}

pub fn get_prevcol_hl_flag(wp: &mut WinInfoInfo, search_hl: &mut SynMatch, curcol: Colnr) -> bool {
    let mut prevcol = curcol;
    let skip_or_left = if wp.w_onebuf_opt.wo_wrap != 0 {
        wp.w_skipcol
    } else {
        wp.w_leftcol
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
    for cur in &wp.w_match_head {
        if !cur.mit_hl.is_addpos
            && (prevcol == cur.mit_hl.startcol
                || (prevcol > cur.mit_hl.startcol && cur.mit_hl.endcol == MAXCOL))
        {
            return true;
        }
    }
    false
}

/// Get highlighting for the char after the text in `char_attr` (`get_search_match_hl`).
pub fn get_search_match_hl(
    wp: &mut WinInfoInfo,
    search_hl: &mut SynMatch,
    col: Colnr,
    char_attr: &mut i32,
) {
    unsafe {
        let mut idx = 0usize;
        let len = wp.w_match_head.len();
        let mut shl: *mut SynMatch;
        let mut shl_flag = false;

        while idx < len || !shl_flag {
            let cur: *mut Matchitem = if idx < len {
                &raw mut (&mut wp.w_match_head)[idx]
            } else {
                ptr::null_mut()
            };
            if !shl_flag && (cur.is_null() || (*cur).mit_priority > SEARCH_HL_PRIORITY) {
                shl = (&raw mut *search_hl);
                shl_flag = true;
            } else {
                shl = &raw mut (*cur).mit_hl;
            }
            let is_search_hl = ptr::eq(shl, &raw mut *search_hl);
            let shl = &mut *shl;
            if col - 1 == shl.startcol && (is_search_hl || !shl.is_addpos) {
                *char_attr = shl.attr;
            }
            if !is_search_hl && !cur.is_null() {
                idx += 1;
            }
        }
    }
}

fn matchadd_dict_arg(
    tv: &mut TypVal,
    conceal_char: *mut *const c_char,
    win: *mut *mut WinInfoInfo,
) -> bool {
    unsafe {
        if tv.v_type != VarType::VarDict {
            emsg(gettext(E_DICTREQ));
            return false;
        }

        let di = tv_dict_find(tv.vval.v_dict().as_ref(), c"conceal", 7);
        if !di.is_null() {
            *conceal_char = tv_get_string(&(*di).di_tv);
        }

        let di = tv_dict_find(tv.vval.v_dict().as_ref(), c"window", 6);
        if di.is_null() {
            return true;
        }

        *win = find_win_by_nr_or_id(&(*di).di_tv);
        if (*win).is_null() {
            emsg(gettext(E_INVALWINDOW));
            return false;
        }

        true
    }
}

/// `clearmatches()` function (`f_clearmatches`).
pub fn f_clearmatches(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    let win = get_optional_window(argvars, 0);
    if !win.is_null() {
        clear_matches(unsafe { &mut *win });
    }
}

/// `getmatches()` function (`f_getmatches`).
pub fn f_getmatches(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let win = get_optional_window(argvars, 0);

    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
    if win.is_null() {
        return;
    }

    for cur in unsafe { &(*win).w_match_head } {
        let dict = tv_dict_alloc();
        if cur.mit_match.regprog.is_null() {
            for (i, llpos) in cur.mit_pos.iter().enumerate() {
                if llpos.lnum == 0 {
                    break;
                }
                let l = tv_list_alloc(1 + if llpos.col > 0 { 2 } else { 0 });
                tv_list_append_number(&mut *l, llpos.lnum as Varnumber);
                if llpos.col > 0 {
                    tv_list_append_number(&mut *l, llpos.col as Varnumber);
                    tv_list_append_number(&mut *l, llpos.len as Varnumber);
                }
                let mut buf = [0 as c_char; 30];
                let len = vim_snprintf!(buf.as_mut_ptr(), 30, c"pos%d", (i + 1) as i32);
                let _ = len;
                tv_dict_add_list(&mut *dict, buf.as_ptr(), l);
            }
        } else {
            tv_dict_add_str(
                &mut *dict,
                c"pattern",
                cur.mit_pattern.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
            );
        }
        tv_dict_add_str(&mut *dict, c"group", syn_id2name(cur.mit_hlg_id));
        tv_dict_add_nr(&mut *dict, c"priority", cur.mit_priority as Varnumber);
        tv_dict_add_nr(&mut *dict, c"id", cur.mit_id as Varnumber);

        if cur.mit_conceal_char != 0 {
            let mut buf = [0 as c_char; MB_MAXCHAR + 1];
            let buflen = utf_char2bytes(cur.mit_conceal_char, buf.as_mut_ptr());
            buf[buflen as usize] = NUL;
            tv_dict_add_str_len(&mut *dict, c"conceal", buf.as_ptr(), buflen);
        }

        tv_list_append_dict(rettv.vval.v_list_ref(), dict);
    }
}

/// `setmatches()` function (`f_setmatches`).
pub fn f_setmatches(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut s: *mut TvList = ptr::null_mut();
        let win = get_optional_window(argvars, 1);

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        if argvars[0].v_type != VarType::VarList {
            emsg(gettext(E_LISTREQ));
            return;
        }
        if win.is_null() {
            return;
        }

        let l = argvars[0].vval.v_list();
        for (li_idx, li) in (0_i32..).zip(crate::eval::typval::list_iter_mut(tv_list_first(
            l.as_ref(),
        ))) {
            if li.li_tv.v_type != VarType::VarDict || li.li_tv.vval.v_dict().is_null() {
                semsg!(
                    gettext(c"E474: List item %d is either not a dictionary or an empty one",),
                    li_idx,
                );
                return;
            }
            let d = li.li_tv.vval.v_dict();
            if tv_dict_find(d.as_ref(), c"group", 5).is_null()
                || (tv_dict_find(d.as_ref(), c"pattern", 7).is_null()
                    && tv_dict_find(d.as_ref(), c"pos1", 4).is_null())
                || tv_dict_find(d.as_ref(), c"priority", 8).is_null()
                || tv_dict_find(d.as_ref(), c"id", 2).is_null()
            {
                semsg!(
                    gettext(c"E474: List item %d is missing one of the required keys"),
                    li_idx,
                );
                return;
            }
        }

        clear_matches(&mut *win);
        let mut match_add_failed = false;
        for li in crate::eval::typval::list_iter_mut(tv_list_first(l.as_ref())) {
            let mut i: i32 = 0;
            let d = li.li_tv.vval.v_dict();
            let di = tv_dict_find(d.as_ref(), c"pattern", 7);
            if di.is_null() {
                if s.is_null() {
                    s = tv_list_alloc(9);
                    tv_list_ref(s.as_mut());
                }

                i = 1;
                while i < 9 {
                    let mut buf = [0 as c_char; 30];
                    vim_snprintf!(buf.as_mut_ptr(), 30, c"pos%d", i);
                    let pos_di = tv_dict_find(d.as_ref(), buf.as_ptr(), -1);
                    if !pos_di.is_null() {
                        let pos_di = { &mut *pos_di };
                        if pos_di.di_tv.v_type != VarType::VarList {
                            tv_list_unref(s);
                            return;
                        }
                        tv_list_append_tv(&mut *s, &mut pos_di.di_tv);
                    } else {
                        break;
                    }
                    i += 1;
                }
            }

            let mut group_buf = [0 as c_char; NUMBUFLEN];
            let group = tv_dict_get_string_buf(d.as_ref(), c"group", group_buf.as_mut_ptr());
            let priority = tv_dict_get_number(d.as_ref(), c"priority") as i32;
            let id = tv_dict_get_number(d.as_ref(), c"id") as i32;
            let conceal_di = tv_dict_find(d.as_ref(), c"conceal", 7);
            let conceal = if !conceal_di.is_null() {
                tv_get_string(&(*conceal_di).di_tv)
            } else {
                ptr::null()
            };
            if i == 0 {
                let pat = tv_dict_get_string(d.as_ref(), c"pattern");
                if match_add(&mut *win, group, pat, priority, id, None, conceal) != id {
                    match_add_failed = true;
                }
            } else {
                if match_add(
                    &mut *win,
                    group,
                    ptr::null(),
                    priority,
                    id,
                    s.as_ref(),
                    conceal,
                ) != id
                {
                    match_add_failed = true;
                }
                tv_list_unref(s);
                s = ptr::null_mut();
            }
        }
        if !match_add_failed {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
    }
}

/// `matchadd()` function (`f_matchadd`).
pub fn f_matchadd(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut grpbuf = [0 as c_char; NUMBUFLEN];
    let mut patbuf = [0 as c_char; NUMBUFLEN];
    let grp = tv_get_string_buf_chk(&argvars[0], grpbuf.as_mut_ptr());
    let pat = tv_get_string_buf_chk(&argvars[1], patbuf.as_mut_ptr());
    let mut prio: i32 = 10;
    let mut id: i32 = -1;
    let mut error = false;
    let mut conceal_char: *const c_char = ptr::null();
    let mut win = curwin_ptr();

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if grp.is_null() || pat.is_null() {
        return;
    }
    if argvars[2].v_type != VarType::VarUnknown {
        prio = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
        if argvars[3].v_type != VarType::VarUnknown {
            id = tv_get_number_chk(&argvars[3], Some(&mut error)) as i32;
            if argvars[4].v_type != VarType::VarUnknown
                && !matchadd_dict_arg(&mut argvars[4], &raw mut conceal_char, &raw mut win)
            {
                return;
            }
        }
    }
    if error {
        return;
    }
    if (1..=3).contains(&id) {
        semsg!(gettext(c"E798: ID is reserved for \":match\": %d"), id,);
        return;
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(match_add(
        unsafe { &mut *win },
        grp,
        pat,
        prio,
        id,
        None,
        conceal_char,
    ) as Varnumber);
}

/// `matchaddpos()` function (`f_matchaddpos`).
pub fn f_matchaddpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    let mut buf = [0 as c_char; NUMBUFLEN];
    let group = tv_get_string_buf_chk(&argvars[0], buf.as_mut_ptr());
    if group.is_null() {
        return;
    }

    if argvars[1].v_type != VarType::VarList {
        semsg!(gettext(E_LISTARG), c"matchaddpos()",);
        return;
    }

    let l = argvars[1].vval.v_list();
    if tv_list_len(unsafe { l.as_ref() }) == 0 {
        return;
    }

    let mut error = false;
    let mut prio: i32 = 10;
    let mut id: i32 = -1;
    let mut conceal_char: *const c_char = ptr::null();
    let mut win = curwin_ptr();

    if argvars[2].v_type != VarType::VarUnknown {
        prio = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
        if argvars[3].v_type != VarType::VarUnknown {
            id = tv_get_number_chk(&argvars[3], Some(&mut error)) as i32;
            if argvars[4].v_type != VarType::VarUnknown
                && !matchadd_dict_arg(&mut argvars[4], &raw mut conceal_char, &raw mut win)
            {
                return;
            }
        }
    }
    if error {
        return;
    }

    if id == 1 || id == 2 {
        semsg!(gettext(c"E798: ID is reserved for \"match\": %d"), id,);
        return;
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(match_add(
        unsafe { &mut *win },
        group,
        ptr::null(),
        prio,
        id,
        unsafe { l.as_ref() },
        conceal_char,
    ) as Varnumber);
}

/// `matcharg()` function (`f_matcharg`).
pub fn f_matcharg(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let id = tv_get_number(&argvars[0]) as i32;

    tv_list_alloc_ret(rettv, if (1..=3).contains(&id) { 2 } else { 0 });

    if (1..=3).contains(&id) {
        let m = get_match(curwin_mut(), id);
        if !m.is_null() {
            let m = unsafe { &mut *m };
            tv_list_append_string(rettv.vval.v_list_ref(), syn_id2name(m.mit_hlg_id), -1);
            tv_list_append_string(
                rettv.vval.v_list_ref(),
                m.mit_pattern.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                -1,
            );
        } else {
            tv_list_append_string(rettv.vval.v_list_ref(), ptr::null(), 0);
            tv_list_append_string(rettv.vval.v_list_ref(), ptr::null(), 0);
        }
    }
}

/// `matchdelete()` function (`f_matchdelete`).
pub fn f_matchdelete(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let win = get_optional_window(argvars, 1);
    rettv.vval = crate::eval::typval::TypvalVval::Number(if win.is_null() {
        -1
    } else {
        match_delete(
            unsafe { &mut *win },
            tv_get_number(&argvars[0]) as i32,
            true,
        ) as Varnumber
    });
}

/// `:[N]match {group} {pattern}` (`ex_match`).
pub fn ex_match(eap: &mut Exarg) {
    let mut g = None;
    let end: *mut c_char;

    let id: i32 = if eap.line2 <= 3 {
        eap.line2
    } else {
        emsg(E_INVCMD);
        return;
    };

    if eap.skip == 0 {
        match_delete(curwin_mut(), id, false);
    }

    let arg = eap.arg;
    if ends_excmd(crate::memory::cstr_first_char(arg)) != 0 {
        end = arg;
    } else {
        let arg_bytes = crate::memory::cstr_as_bytes(arg);
        if arg_bytes.len() >= 4
            && arg_bytes[..4].eq_ignore_ascii_case(b"none")
            && (ascii_iswhite(crate::memory::cstr_char_at(arg, 4))
                || ends_excmd(crate::memory::cstr_char_at(arg, 4)) != 0)
        {
            end = arg.wrapping_add(4);
        } else {
            let mut p = skiptowhite(arg);
            if eap.skip == 0 {
                g = Some(
                    std::ffi::CString::new(crate::memory::cstr_slice(
                        arg,
                        crate::memory::ptr_offset_from(p, arg) as usize,
                    ))
                    .unwrap_or_default(),
                );
            }
            p = skipwhite(p);
            if crate::memory::cstr_is_empty(p) {
                semsg!(gettext(E_INVARG2), arg);
                return;
            }
            end = skip_regexp(
                p.wrapping_add(1),
                i32::from(crate::memory::cstr_first_byte(p)),
                1,
            );
            if eap.skip == 0 {
                if !crate::memory::cstr_is_empty(end)
                    && ends_excmd(i32::from(unsafe { *skipwhite(end.wrapping_add(1)) })) == 0
                {
                    eap.errmsg = ex_errmsg(E_TRAILING_ARG, end);
                    return;
                }
                if unsafe { *end } != unsafe { *p } {
                    semsg!(gettext(E_INVARG2), p);
                    return;
                }

                let c = unsafe { *end };
                unsafe {
                    *end = NUL;
                }
                match_add(
                    curwin_mut(),
                    g.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                    p.wrapping_add(1),
                    10,
                    id,
                    None,
                    ptr::null(),
                );
                unsafe {
                    *end = c;
                }
            }
        }
    }
    eap.nextcmd = find_nextcmd(end);
}
