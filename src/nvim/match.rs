//! Functions for highlighting matches (`src/nvim/match.c`).
#![forbid(unsafe_code)]

use std::ffi::CStr;

use crate::charset::ascii_iswhite;
use crate::drawscreen::{UPD_SOME_VALID, UPD_VALID, redraw_later, redraw_win_range_later};
use crate::errors::*;
use crate::eval::funcs::get_optional_window;
use crate::eval::typval::{
    EvalFuncData, TvList, TypVal, VarType, Varnumber, list_iter_ptr, tv_dict_add_list,
    tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_dict_find_opt,
    tv_dict_get_number, tv_dict_get_string, tv_dict_get_string_buf, tv_get_number,
    tv_get_number_chk, tv_get_string, tv_get_string_buf_chk, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_dict, tv_list_append_number, tv_list_append_string, tv_list_append_tv,
    tv_list_first, tv_list_idx_of_item, tv_list_iter, tv_list_len, tv_list_ref, tv_list_unref,
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
    MAXCOL, NUMBUFLEN, RE_MAGIC, Regmmatch, re_multiline, skip_regexp, vim_regcomp,
    vim_regexec_multi,
};
use crate::strings::{format_to_buf, gettext};
use crate::window::WinInfoInfo;

use crate::change::MB_MAXCHAR;
use crate::cursor::{Colnr, Linenr};
use crate::eval::K_LIST_LEN_MAY_KNOW_CONST;

const SEARCH_HL_PRIORITY: i32 = 0;

pub(crate) const HLF_L: usize = 8;
pub(crate) const HLF_LC: usize = 9;
pub(crate) const K_CPO_SEARCH: i32 = b'c' as i32;

#[derive(Clone)]
pub struct SynMatch {
    pub rm: Regmmatch,
    pub buf: crate::globals::BufPtr,
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
            buf: crate::globals::BufPtr::NULL,
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

#[derive(Clone, Copy, Default)]
pub struct Llpos {
    pub lnum: Linenr,
    pub col: Colnr,
    pub len: i32,
}

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
        self.mit_match.free_prog();
    }
}

pub struct SearchState {
    pub hl_match: std::cell::Cell<bool>,
    pub match_lines: std::cell::Cell<Linenr>,
    pub match_endcol: std::cell::Cell<Colnr>,
    pub first_line: std::cell::Cell<Linenr>,
    pub last_line: std::cell::Cell<Linenr>,
    pub no_smartcase: std::cell::Cell<bool>,
    pub cmdlen: std::cell::Cell<i32>,
    pub no_hlsearch: std::cell::Cell<bool>,
}

use crate::highlight::win_hl_attr;

/// Add match to the match list of window `wp` (`match_add`).
fn match_add(
    wp: &WinInfoInfo,
    grp: Option<&std::ffi::CStr>,
    pat: Option<&std::ffi::CStr>,
    prio: i32,
    mut id: i32,
    pos_list: Option<&TvList>,
    conceal_char: Option<&std::ffi::CStr>,
) -> i32 {
    let mut regprog = crate::regexp::RegprogPtr::NULL;
    let mut rtype = UPD_SOME_VALID;

    let Some(grp) = grp.filter(|s| !s.is_empty()) else {
        return -1;
    };
    if pat.is_some_and(std::ffi::CStr::is_empty) {
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
        id = wp.w_next_match_id.get();
        wp.w_next_match_id.set(wp.w_next_match_id.get() + (1));
    } else {
        if wp.with_match_head(|matches| matches.iter().any(|cur| cur.mit_id == id)) {
            semsg!(gettext(c"E801: ID already taken: %lld"), i64::from(id),);
            return -1;
        }

        if wp.w_next_match_id.get() < id + 100 {
            wp.w_next_match_id.set(id + 100);
        }
    }

    let hlg_id = syn_check_group(grp.to_bytes());
    if hlg_id == 0 {
        return -1;
    }
    if let Some(pat) = pat {
        let Some(prog_box) = vim_regcomp(pat.to_bytes(), RE_MAGIC) else {
            semsg!(gettext(E_INVARG2), pat);
            return -1;
        };
        regprog = crate::regexp::RegprogPtr::from_box(Some(prog_box));
    }

    let pos_len = tv_list_len(pos_list);
    let mut m = Matchitem {
        mit_id: id,
        mit_priority: prio,
        mit_pattern: pat.map(std::ffi::CStr::to_owned),
        mit_match: Regmmatch {
            regprog,
            startpos: [crate::cursor::Lpos { lnum: 0, col: 0 }; 10],
            endpos: [crate::cursor::Lpos { lnum: 0, col: 0 }; 10],
            rmm_matchcol: 0,
            rmm_ic: false,
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
        mit_conceal_char: conceal_char.map_or(0, utf_ptr2char),
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

            if li.tv().var_type() == VarType::VarList {
                let subl = li.tv().value.list();
                let mut sub_iter = tv_list_iter(subl.opt());
                let Some(subli1) = sub_iter.next() else {
                    semsg!(
                        gettext(c"E5030: Empty list at position %d"),
                        tv_list_idx_of_item(pos_list, Some(&li)).unwrap_or(-1),
                    );
                    failed = true;
                    break;
                };
                lnum = tv_get_number_chk(&subli1.tv(), Some(&mut error)) as Linenr;
                if error {
                    failed = true;
                    break;
                }
                if lnum <= 0 {
                    continue;
                }
                m.mit_pos[i].lnum = lnum;
                if let Some(subli2) = sub_iter.next() {
                    col = tv_get_number_chk(&subli2.tv(), Some(&mut error)) as Colnr;
                    if error {
                        failed = true;
                        break;
                    }
                    if col < 0 {
                        continue;
                    }
                    if let Some(subli3) = sub_iter.next() {
                        len = tv_get_number_chk(&subli3.tv(), Some(&mut error)) as Colnr;
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
            } else if li.tv().var_type() == VarType::VarNumber {
                if li.tv().value.number() <= 0 {
                    continue;
                }
                m.mit_pos[i].lnum = li.tv().value.number() as Linenr;
                m.mit_pos[i].col = 0;
                m.mit_pos[i].len = 0;
            } else {
                semsg!(
                    gettext(c"E5031: List or number required at position %d"),
                    tv_list_idx_of_item(pos_list, Some(&li)).unwrap_or(-1),
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

    wp.with_match_head_mut(|matches| {
        let insert_idx = matches
            .iter()
            .position(|cur| prio < cur.mit_priority)
            .unwrap_or(matches.len());
        matches.insert(insert_idx, m);
    });

    redraw_later(Some(wp), rtype);
    id
}

/// Delete match with ID `id` in the match list of window `wp` (`match_delete`).
fn match_delete(wp: &WinInfoInfo, id: i32, perr: bool) -> bool {
    let mut rtype = UPD_SOME_VALID;

    if id < 1 {
        if perr {
            semsg!(
                gettext(c"E802: Invalid ID: %lld (must be greater than or equal to 1)",),
                i64::from(id),
            );
        }
        return false;
    }
    let Some(cur) = wp.with_match_head_mut(|matches| {
        let idx = matches.iter().position(|cur| cur.mit_id == id)?;
        Some(matches.remove(idx))
    }) else {
        if perr {
            semsg!(gettext(c"E803: ID not found: %lld"), i64::from(id));
        }
        return false;
    };
    if cur.mit_toplnum != 0 {
        redraw_win_range_later(wp, cur.mit_toplnum, cur.mit_botlnum);
        rtype = UPD_VALID;
    }
    redraw_later(Some(wp), rtype);
    true
}

/// Delete all matches in the match list of window `wp` (`clear_matches`).
pub fn clear_matches(wp: &WinInfoInfo) {
    wp.with_match_head_mut(Vec::clear);
    redraw_later(Some(wp), UPD_SOME_VALID);
}

/// Get match from ID `id` in window `wp` (`get_match`).
fn get_match(wp: &WinInfoInfo, id: i32) -> Option<(i32, Option<std::ffi::CString>)> {
    wp.with_match_head(|matches| {
        matches
            .iter()
            .find(|cur| cur.mit_id == id)
            .map(|cur| (cur.mit_hlg_id, cur.mit_pattern.clone()))
    })
}

/// Init for calling `prepare_search_hl()` (`init_search_hl`).
pub fn init_search_hl(wp: &WinInfoInfo, search_hl: &mut SynMatch) {
    let buf = wp.w_buffer.get();
    wp.with_match_head_mut(|matches| {
        for cur in matches {
            cur.mit_hl.rm = cur.mit_match.clone();
            cur.mit_hl.attr = if cur.mit_hlg_id == 0 {
                0
            } else {
                syn_id2attr(cur.mit_hlg_id)
            };
            cur.mit_hl.buf = buf.clone();
            cur.mit_hl.lnum = 0;
            cur.mit_hl.first_lnum = 0;
            cur.mit_hl.tm = profile_setlimit(opt_vars().p_rdt.get());
        }
    });
    search_hl.buf = wp.w_buffer.get();
    search_hl.lnum = 0;
    search_hl.first_lnum = 0;
    search_hl.attr = win_hl_attr(wp, HLF_L as i32);
}

fn next_search_hl_pos(
    shl: &mut SynMatch,
    lnum: Linenr,
    mit_pos: &mut [Llpos],
    mit_pos_cur: &mut i32,
    mincol: Colnr,
) -> bool {
    let mut found: isize = -1;

    shl.lnum = 0;
    let pos_count = mit_pos.len() as i32;
    for i in *mit_pos_cur..pos_count {
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
    *mit_pos_cur = 0;
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
        *mit_pos_cur = (found as i32) + 1;
        return true;
    }
    false
}

#[inline]
fn find_match(v: &[Matchitem], idx_hint: usize, id: i32) -> Option<&Matchitem> {
    if let Some(m) = v.get(idx_hint)
        && m.mit_id == id
    {
        return Some(m);
    }
    v.iter().find(|m| m.mit_id == id)
}

#[inline]
fn find_match_mut(v: &mut [Matchitem], idx_hint: usize, id: i32) -> Option<&mut Matchitem> {
    if v.get(idx_hint).is_some_and(|m| m.mit_id == id) {
        return Some(&mut v[idx_hint]);
    }
    v.iter_mut().find(|m| m.mit_id == id)
}

#[inline]
fn match_key_at(wp: &WinInfoInfo, idx: usize) -> Option<(usize, i32)> {
    wp.with_match_head(|v| v.get(idx).map(|m| (idx, m.mit_id)))
}

#[inline]
fn with_shl<R>(
    wp: &WinInfoInfo,
    search_hl: &SynMatch,
    match_key: Option<(usize, i32)>,
    f: impl FnOnce(&SynMatch) -> R,
) -> Option<R> {
    match match_key {
        Some((idx, id)) => wp.with_match_head(|v| find_match(v, idx, id).map(|m| f(&m.mit_hl))),
        None => Some(f(search_hl)),
    }
}

#[inline]
fn with_shl_mut<R>(
    wp: &WinInfoInfo,
    search_hl: &mut SynMatch,
    match_key: Option<(usize, i32)>,
    f: impl FnOnce(&mut SynMatch) -> R,
) -> Option<R> {
    match match_key {
        Some((idx, id)) => {
            wp.with_match_head_mut(|v| find_match_mut(v, idx, id).map(|m| f(&mut m.mit_hl)))
        }
        None => Some(f(search_hl)),
    }
}

/// Search for a next `'hlsearch'` or match (`next_search_hl`).
fn next_search_hl(
    win: &WinInfoInfo,
    search_hl: &mut SynMatch,
    lnum: Linenr,
    mincol: Colnr,
    match_key: Option<(usize, i32)>,
) {
    let mut matchcol: Colnr;
    let mut nmatched: i32 = 0;
    let called_emsg_before = msg_state().called_emsg.get();

    if (lnum < search_state().search_global.first_line.get()
        || lnum > search_state().search_global.last_line.get())
        && match_key.is_none()
    {
        with_shl_mut(win, search_hl, match_key, |shl| shl.lnum = 0);
        return;
    }

    let Some(early_return) = with_shl_mut(win, search_hl, match_key, |shl| {
        if shl.lnum != 0 {
            let l = shl.lnum + shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum;
            if lnum > l {
                shl.lnum = 0;
            } else if lnum < l || shl.rm.endpos[0].col > mincol {
                return true;
            }
        }
        false
    }) else {
        return;
    };
    if early_return {
        return;
    }

    loop {
        let Some((shl_tm, shl_lnum, start_col, end_lnum, end_col, shl_buf)) =
            with_shl(win, search_hl, match_key, |shl| {
                (
                    shl.tm,
                    shl.lnum,
                    shl.rm.startpos[0].col,
                    shl.rm.endpos[0].lnum,
                    shl.rm.endpos[0].col,
                    shl.buf.clone(),
                )
            })
        else {
            break;
        };
        if profile_passed_limit(shl_tm) {
            with_shl_mut(win, search_hl, match_key, |shl| shl.lnum = 0);
            break;
        }
        if shl_lnum == 0 {
            matchcol = 0;
        } else if !crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_SEARCH)
            || (end_lnum == 0 && end_col <= start_col)
        {
            matchcol = start_col;
            let line_slice = crate::memline::ml_get_buf_slice(shl_buf.borrow(), lnum);
            if (matchcol as usize) >= line_slice.len() {
                with_shl_mut(win, search_hl, match_key, |shl| shl.lnum = 0);
                break;
            }
            matchcol += crate::mbyte::utfc_slice2len(&line_slice[matchcol as usize..]) as Colnr;
        } else {
            matchcol = end_col;
        }

        if with_shl_mut(win, search_hl, match_key, |shl| shl.lnum = lnum).is_none() {
            break;
        }

        let has_regprog =
            with_shl(win, search_hl, match_key, |shl| !shl.rm.regprog.is_null()).unwrap_or(false);
        if has_regprog {
            let mut timed_out: i32 = 0;
            let Some((mut rm, shl_buf, shl_tm)) = with_shl_mut(win, search_hl, match_key, |shl| {
                (std::mem::take(&mut shl.rm), shl.buf.clone(), shl.tm)
            }) else {
                break;
            };
            nmatched = vim_regexec_multi(
                &mut rm,
                Some(win),
                &shl_buf,
                lnum,
                matchcol,
                Some(shl_tm),
                Some(&mut timed_out),
            );
            let mut rm_opt = Some(rm);
            let should_break = with_shl_mut(win, search_hl, match_key, |shl| {
                shl.rm = rm_opt.take().unwrap();
                if msg_state().called_emsg.get() > called_emsg_before
                    || input_state().got_int.get()
                    || timed_out != 0
                {
                    if match_key.is_none() {
                        shl.rm.free_prog();
                        set_no_hlsearch(true);
                    }
                    shl.rm.free_prog();
                    shl.lnum = 0;
                    input_state().got_int.set(false);
                    true
                } else {
                    false
                }
            });
            if let Some(mut orphan_rm) = rm_opt {
                orphan_rm.free_prog();
                break;
            }
            if should_break.unwrap_or(true) {
                break;
            }
        } else if let Some((idx, id)) = match_key {
            let Some(matched) = win.with_match_head_mut(|matches| {
                find_match_mut(matches, idx, id).map(|cur| {
                    i32::from(next_search_hl_pos(
                        &mut cur.mit_hl,
                        lnum,
                        &mut cur.mit_pos,
                        &mut cur.mit_pos_cur,
                        matchcol,
                    ))
                })
            }) else {
                break;
            };
            nmatched = matched;
        }
        let done = with_shl_mut(win, search_hl, match_key, |shl| {
            if nmatched == 0 {
                shl.lnum = 0;
                true
            } else if shl.rm.startpos[0].lnum > 0
                || shl.rm.startpos[0].col >= mincol
                || nmatched > 1
                || shl.rm.endpos[0].col > mincol
            {
                shl.lnum += shl.rm.startpos[0].lnum;
                true
            } else {
                false
            }
        })
        .unwrap_or(true);
        if done {
            break;
        }
    }
}

/// Advance to the match in window `wp` line `lnum` or past it (`prepare_search_hl`).
pub fn prepare_search_hl(wp: &WinInfoInfo, search_hl: &mut SynMatch, lnum: Linenr) {
    let mut idx = 0usize;
    let mut shl_flag = false;

    while idx < wp.match_head_len() || !shl_flag {
        let match_key = if !shl_flag {
            shl_flag = true;
            None
        } else {
            let Some(key) = match_key_at(wp, idx) else {
                break;
            };
            Some(key)
        };
        let is_search_hl = match_key.is_none();
        let should_prep = with_shl(wp, search_hl, match_key, |shl| {
            !shl.rm.regprog.is_null() && shl.lnum == 0 && re_multiline(&shl.rm.regprog) != 0
        })
        .unwrap_or(false);
        if should_prep {
            let mut first_lnum =
                with_shl(wp, search_hl, match_key, |shl| shl.first_lnum).unwrap_or(0);
            if first_lnum == 0 {
                first_lnum = lnum;
                while first_lnum > wp.w_topline.get() {
                    if has_folding(wp, first_lnum - 1, None, None) {
                        break;
                    }
                    first_lnum -= 1;
                }
                with_shl_mut(wp, search_hl, match_key, |shl| shl.first_lnum = first_lnum);
            }
            if let Some((m_idx, m_id)) = match_key {
                wp.with_match_head_mut(|matches| {
                    if let Some(m) = find_match_mut(matches, m_idx, m_id) {
                        m.mit_pos_cur = 0;
                    }
                });
            }
            let mut pos_inprogress = true;
            let mut n: Colnr = 0;
            while with_shl(wp, search_hl, match_key, |shl| {
                shl.first_lnum < lnum
                    && (!shl.rm.regprog.is_null() || (match_key.is_some() && pos_inprogress))
            })
            .unwrap_or(false)
            {
                let Some(cur_first_lnum) = with_shl(wp, search_hl, match_key, |shl| shl.first_lnum)
                else {
                    break;
                };
                next_search_hl(wp, search_hl, cur_first_lnum, n, match_key);

                pos_inprogress = match_key.is_some_and(|(m_idx, m_id)| {
                    wp.with_match_head(|matches| {
                        find_match(matches, m_idx, m_id).is_some_and(|m| m.mit_pos_cur != 0)
                    })
                });

                if with_shl_mut(wp, search_hl, match_key, |shl| {
                    if shl.lnum != 0 {
                        shl.first_lnum = shl.lnum + shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum;
                        n = shl.rm.endpos[0].col;
                    } else {
                        shl.first_lnum += 1;
                        n = 0;
                    }
                })
                .is_none()
                {
                    break;
                }
            }
        }
        if !is_search_hl {
            idx += 1;
        }
    }
}

/// Update `shl->has_cursor` based on the match in `shl` and the cursor position (`check_cur_search_hl`).
fn check_cur_search_hl(wp: &WinInfoInfo, shl: &mut SynMatch) {
    let linecount = shl.rm.endpos[0].lnum - shl.rm.startpos[0].lnum;
    shl.has_cursor = wp.w_cursor.get().lnum >= shl.lnum
        && wp.w_cursor.get().lnum <= shl.lnum + linecount
        && (wp.w_cursor.get().lnum > shl.lnum || wp.w_cursor.get().col >= shl.rm.startpos[0].col)
        && (wp.w_cursor.get().lnum < shl.lnum + linecount
            || wp.w_cursor.get().col < shl.rm.endpos[0].col);
}

/// Prepare for `'hlsearch'` and match highlighting in one window line (`prepare_search_hl_line`).
pub fn prepare_search_hl_line(
    wp: &WinInfoInfo,
    lnum: Linenr,
    mincol: Colnr,
    search_hl: &mut SynMatch,
    search_attr: &mut i32,
    search_attr_from_match: &mut bool,
) -> bool {
    let mut idx = 0usize;
    if wp.match_head_is_empty() && search_hl.rm.regprog.is_null() {
        search_hl.startcol = MAXCOL;
        search_hl.endcol = MAXCOL;
        search_hl.attr_cur = 0;
        search_hl.is_addpos = false;
        search_hl.has_cursor = false;
        search_hl.lnum = 0;
        return false;
    }
    let mut shl_flag = false;
    let mut area_highlighting = false;

    while idx < wp.match_head_len() || !shl_flag {
        let match_key = if !shl_flag {
            shl_flag = true;
            None
        } else {
            let Some(key) = match_key_at(wp, idx) else {
                break;
            };
            Some(key)
        };
        let is_search_hl = match_key.is_none();
        with_shl_mut(wp, search_hl, match_key, |shl| {
            shl.startcol = MAXCOL;
            shl.endcol = MAXCOL;
            shl.attr_cur = 0;
            shl.is_addpos = false;
            shl.has_cursor = false;
        });
        if let Some((m_idx, m_id)) = match_key {
            wp.with_match_head_mut(|matches| {
                if let Some(m) = find_match_mut(matches, m_idx, m_id) {
                    m.mit_pos_cur = 0;
                }
            });
        }

        next_search_hl(wp, search_hl, lnum, mincol, match_key);

        let shl_lnum = with_shl(wp, search_hl, match_key, |shl| shl.lnum).unwrap_or(0);
        if shl_lnum != 0 && shl_lnum <= lnum {
            with_shl_mut(wp, search_hl, match_key, |shl| {
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
            });

            if is_search_hl {
                check_cur_search_hl(wp, search_hl);
            }

            let Some((startcol, endcol)) =
                with_shl(wp, search_hl, match_key, |shl| (shl.startcol, shl.endcol))
            else {
                break;
            };
            if startcol == endcol {
                let line_cstr = ml_get_buf(&wp.buf(), lnum);
                let line_bytes = line_cstr.to_bytes();
                let end_off = endcol as usize;
                let step = if end_off < line_bytes.len() {
                    utfc_ptr2len(&line_bytes[end_off..])
                } else {
                    1
                };
                with_shl_mut(wp, search_hl, match_key, |shl| shl.endcol += step);
            }
            with_shl_mut(wp, search_hl, match_key, |shl| {
                if shl.startcol < mincol {
                    shl.attr_cur = shl.attr;
                    *search_attr = shl.attr;
                    *search_attr_from_match = !is_search_hl;
                }
            });
            area_highlighting = true;
        }
        if !is_search_hl {
            idx += 1;
        }
    }
    area_highlighting
}

/// For a position in a line: Check for start/end of `'hlsearch'` and other matches (`update_search_hl`).
pub fn update_search_hl(
    wp: &WinInfoInfo,
    lnum: Linenr,
    col: Colnr,
    search_hl: &mut SynMatch,
    has_match_conc: &mut i32,
    match_conc: &mut i32,
    lcs_eol_todo: bool,
    on_last_col: &mut bool,
    search_attr_from_match: &mut bool,
) -> i32 {
    let mut idx = 0usize;
    if wp.match_head_is_empty() && search_hl.rm.regprog.is_null() {
        *search_attr_from_match = false;
        return search_hl.attr_cur;
    }
    let mut shl_flag = false;

    while idx < wp.match_head_len() || !shl_flag {
        let match_key = if !shl_flag
            && wp.with_match_head(|matches| {
                matches
                    .get(idx)
                    .is_none_or(|m| m.mit_priority > SEARCH_HL_PRIORITY)
            }) {
            shl_flag = true;
            None
        } else {
            let Some(key) = match_key_at(wp, idx) else {
                break;
            };
            Some(key)
        };
        let is_search_hl = match_key.is_none();
        if let Some((m_idx, m_id)) = match_key {
            wp.with_match_head_mut(|matches| {
                if let Some(m) = find_match_mut(matches, m_idx, m_id) {
                    m.mit_pos_cur = 0;
                }
            });
        }
        let mut pos_inprogress = true;
        while with_shl(wp, search_hl, match_key, |shl| {
            !shl.rm.regprog.is_null() || (match_key.is_some() && pos_inprogress)
        })
        .unwrap_or(false)
        {
            let Some((startcol, endcol, has_cursor, attr)) =
                with_shl(wp, search_hl, match_key, |shl| {
                    (shl.startcol, shl.endcol, shl.has_cursor, shl.attr)
                })
            else {
                break;
            };
            if startcol != MAXCOL && col >= startcol && col < endcol {
                let next_col = col
                    + wp.with_buf(|buf| {
                        crate::memline::with_ml_buf_line(buf, lnum, |line_bytes| {
                            utfc_ptr2len(line_bytes.get(col as usize..).unwrap_or(&[]))
                        })
                    });
                if endcol < next_col {
                    with_shl_mut(wp, search_hl, match_key, |shl| shl.endcol = next_col);
                }
                if is_search_hl && has_cursor {
                    let attr_cur = win_hl_attr(wp, HLF_LC as i32);
                    search_hl.attr_cur = attr_cur;
                    if attr_cur != attr {
                        search_state().search_hl_has_cursor_lnum.set(lnum);
                    }
                } else {
                    with_shl_mut(wp, search_hl, match_key, |shl| shl.attr_cur = attr);
                }
                if let Some((m_idx, m_id)) = match_key {
                    let conceal_id = syn_name2id(c"Conceal");
                    let hlg_conc = wp.with_match_head(|matches| {
                        find_match(matches, m_idx, m_id).map(|m| (m.mit_hlg_id, m.mit_conceal_char))
                    });
                    if let Some((hlg_id, conceal_char)) = hlg_conc
                        && conceal_id == hlg_id
                    {
                        *has_match_conc = if col == startcol { 2 } else { 1 };
                        *match_conc = conceal_char;
                    } else {
                        *has_match_conc = 0;
                    }
                } else {
                    *has_match_conc = 0;
                }
            } else if col == endcol {
                with_shl_mut(wp, search_hl, match_key, |shl| shl.attr_cur = 0);

                next_search_hl(wp, search_hl, lnum, col, match_key);

                pos_inprogress = match_key.is_some_and(|(m_idx, m_id)| {
                    wp.with_match_head(|matches| {
                        find_match(matches, m_idx, m_id).is_some_and(|m| m.mit_pos_cur != 0)
                    })
                });

                if with_shl(wp, search_hl, match_key, |shl| shl.lnum).unwrap_or(0) == lnum {
                    with_shl_mut(wp, search_hl, match_key, |shl| {
                        shl.startcol = shl.rm.startpos[0].col;
                        shl.endcol = if shl.rm.endpos[0].lnum == 0 {
                            shl.rm.endpos[0].col
                        } else {
                            MAXCOL
                        };
                    });

                    if is_search_hl {
                        check_cur_search_hl(wp, search_hl);
                    }

                    let Some((new_startcol, new_endcol)) =
                        with_shl(wp, search_hl, match_key, |shl| (shl.startcol, shl.endcol))
                    else {
                        break;
                    };
                    if new_startcol == new_endcol {
                        let end_off = new_endcol as usize;
                        let step = wp.with_buf(|buf| {
                            crate::memline::with_ml_buf_line(buf, lnum, |line_bytes| {
                                if end_off >= line_bytes.len() {
                                    1
                                } else {
                                    utfc_ptr2len(&line_bytes[end_off..])
                                }
                            })
                        });
                        with_shl_mut(wp, search_hl, match_key, |shl| shl.endcol += step);
                    }
                    continue;
                }
            }
            break;
        }
        if !is_search_hl {
            idx += 1;
        }
    }

    *search_attr_from_match = false;
    let mut search_attr = search_hl.attr_cur;
    idx = 0;
    shl_flag = false;
    while idx < wp.match_head_len() || !shl_flag {
        let match_key = if !shl_flag
            && wp.with_match_head(|matches| {
                matches
                    .get(idx)
                    .is_none_or(|m| m.mit_priority > SEARCH_HL_PRIORITY)
            }) {
            shl_flag = true;
            None
        } else {
            let Some(key) = match_key_at(wp, idx) else {
                break;
            };
            Some(key)
        };
        let is_search_hl = match_key.is_none();
        with_shl(wp, search_hl, match_key, |shl| {
            if shl.attr_cur != 0 {
                search_attr = shl.attr_cur;
                *on_last_col = col + 1 >= shl.endcol;
                *search_attr_from_match = !is_search_hl;
            }
        });
        if !is_search_hl {
            idx += 1;
        }
    }
    if wp.w_onebuf_opt.wo_list.get()
        && !lcs_eol_todo
        && col >= wp.with_buf(|buf| crate::memline::ml_get_buf_len(buf, lnum))
    {
        search_attr = 0;
    }
    search_attr
}

pub fn get_prevcol_hl_flag(wp: &WinInfoInfo, search_hl: &mut SynMatch, curcol: Colnr) -> bool {
    let mut prevcol = curcol;
    let skip_or_left = if wp.w_onebuf_opt.wo_wrap.get() {
        wp.w_skipcol.get()
    } else {
        wp.w_leftcol.get()
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
    wp.with_match_head(|matches| {
        matches.iter().any(|cur| {
            !cur.mit_hl.is_addpos
                && (prevcol == cur.mit_hl.startcol
                    || (prevcol > cur.mit_hl.startcol && cur.mit_hl.endcol == MAXCOL))
        })
    })
}

/// Get highlighting for the char after the text in `char_attr` (`get_search_match_hl`).
pub fn get_search_match_hl(
    wp: &WinInfoInfo,
    search_hl: &mut SynMatch,
    col: Colnr,
    char_attr: &mut i32,
) {
    let mut idx = 0usize;
    let mut shl_flag = false;

    while idx < wp.match_head_len() || !shl_flag {
        let match_key = if !shl_flag
            && wp.with_match_head(|matches| {
                matches
                    .get(idx)
                    .is_none_or(|m| m.mit_priority > SEARCH_HL_PRIORITY)
            }) {
            shl_flag = true;
            None
        } else {
            let Some(key) = match_key_at(wp, idx) else {
                break;
            };
            Some(key)
        };
        let is_search_hl = match_key.is_none();
        with_shl(wp, search_hl, match_key, |shl| {
            if col - 1 == shl.startcol && (is_search_hl || !shl.is_addpos) {
                *char_attr = shl.attr;
            }
        });
        if !is_search_hl {
            idx += 1;
        }
    }
}

fn matchadd_dict_arg<'a>(
    tv: &'a TypVal,
    conceal_char: &mut Option<std::borrow::Cow<'a, std::ffi::CStr>>,
    win: &mut crate::globals::WinPtr,
) -> bool {
    if tv.var_type() != VarType::VarDict {
        emsg(gettext(E_DICTREQ));
        return false;
    }

    if let Some(di) = tv_dict_find_opt(tv.value.dict_opt(), c"conceal", 7) {
        *conceal_char = Some(std::borrow::Cow::Owned(
            tv_get_string(&di.tv()).into_owned(),
        ));
    }

    let Some(di) = tv_dict_find_opt(tv.value.dict_opt(), c"window", 6) else {
        return true;
    };

    let Some(found_win) = find_win_by_nr_or_id(&di.tv()) else {
        emsg(gettext(E_INVALWINDOW));
        return false;
    };
    *win = found_win;

    true
}

/// `clearmatches()` function (`f_clearmatches`).
pub fn f_clearmatches(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if let Some(win) = get_optional_window(argvars, 0) {
        clear_matches(&win);
    }
}

/// `getmatches()` function (`f_getmatches`).
pub fn f_getmatches(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let win = get_optional_window(argvars, 0);

    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
    let Some(win) = win else {
        return;
    };

    win.with_match_head(|matches| {
        for cur in matches {
            let dict = tv_dict_alloc();
            if cur.mit_match.regprog.is_null() {
                for (i, llpos) in cur.mit_pos.iter().enumerate() {
                    if llpos.lnum == 0 {
                        break;
                    }
                    let l = tv_list_alloc(1 + if llpos.col > 0 { 2 } else { 0 });
                    tv_list_append_number(&l, llpos.lnum as Varnumber);
                    if llpos.col > 0 {
                        tv_list_append_number(&l, llpos.col as Varnumber);
                        tv_list_append_number(&l, llpos.len as Varnumber);
                    }
                    let mut buf = [0u8; 30];
                    let len = format_to_buf!(&mut buf, c"pos%d", (i + 1) as i32);
                    let _ = len;
                    tv_dict_add_list(
                        &dict,
                        std::ffi::CStr::from_bytes_until_nul(&buf).unwrap_or_default(),
                        Some(&l),
                    );
                }
            } else {
                tv_dict_add_str(&dict, c"pattern", cur.mit_pattern.as_deref());
            }
            let group_name = syn_id2name(cur.mit_hlg_id);
            tv_dict_add_str(&dict, c"group", Some(&group_name));
            tv_dict_add_nr(&dict, c"priority", cur.mit_priority as Varnumber);
            tv_dict_add_nr(&dict, c"id", cur.mit_id as Varnumber);

            if cur.mit_conceal_char != 0 {
                let mut buf = [0u8; MB_MAXCHAR + 1];
                let buflen = utf_char2bytes(cur.mit_conceal_char, &mut buf) as usize;
                buf[buflen] = 0;
                tv_dict_add_str_len(&dict, c"conceal", Some(&buf[..buflen]));
            }

            tv_list_append_dict(rettv.value.list_ref(), Some(&dict));
        }
    });
}

/// `setmatches()` function (`f_setmatches`).
pub fn f_setmatches(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut s = crate::eval::typval::ListId::NULL;
    let win = get_optional_window(argvars, 1);

    rettv.value = crate::eval::typval::TvValue::Number(-1);
    if argvars[0].var_type() != VarType::VarList {
        emsg(gettext(E_LISTREQ));
        return;
    }
    let Some(win) = win else {
        return;
    };

    for (li_idx, li) in (0_i32..).zip(list_iter_ptr(tv_list_first(argvars[0].value.list_opt()))) {
        let li_tv = li.tv();
        let Some(d) = li_tv
            .value
            .dict_opt()
            .filter(|_| li_tv.var_type() == VarType::VarDict)
        else {
            semsg!(
                gettext(c"E474: List item %d is either not a dictionary or an empty one",),
                li_idx,
            );
            return;
        };
        if tv_dict_find_opt(Some(d), c"group", 5).is_none()
            || (tv_dict_find_opt(Some(d), c"pattern", 7).is_none()
                && tv_dict_find_opt(Some(d), c"pos1", 4).is_none())
            || tv_dict_find_opt(Some(d), c"priority", 8).is_none()
            || tv_dict_find_opt(Some(d), c"id", 2).is_none()
        {
            semsg!(
                gettext(c"E474: List item %d is missing one of the required keys"),
                li_idx,
            );
            return;
        }
    }

    clear_matches(&win);
    let mut match_add_failed = false;
    for li in list_iter_ptr(tv_list_first(argvars[0].value.list_opt())) {
        let mut i: i32 = 0;
        let d_id = li.tv().value.dict();
        let d = d_id.borrow();
        if tv_dict_find_opt(Some(d), c"pattern", 7).is_none() {
            if s.is_null() {
                let list = tv_list_alloc(9);
                tv_list_ref(Some(&list));
                s = list;
            }

            i = 1;
            while i < 9 {
                let mut buf = [0u8; 30];
                format_to_buf!(&mut buf, c"pos%d", i);
                if let Some(pos_di) = tv_dict_find_opt(
                    Some(d),
                    std::ffi::CStr::from_bytes_until_nul(&buf).unwrap_or_default(),
                    -1,
                ) {
                    if pos_di.tv().var_type() != VarType::VarList {
                        tv_list_unref(s.opt());
                        return;
                    }
                    tv_list_append_tv(&s, &pos_di.tv());
                } else {
                    break;
                }
                i += 1;
            }
        }

        let mut group_buf = [0u8; NUMBUFLEN];
        let group = tv_dict_get_string_buf(Some(d), c"group", &mut group_buf);
        let priority = tv_dict_get_number(Some(d), c"priority") as i32;
        let id = tv_dict_get_number(Some(d), c"id") as i32;
        let conceal = tv_dict_find_opt(Some(d), c"conceal", 7)
            .map(|di| std::borrow::Cow::Owned(tv_get_string(&di.tv()).into_owned()));
        if i == 0 {
            let pat = tv_dict_get_string(Some(d), c"pattern");
            if match_add(
                &win,
                group.as_deref(),
                pat.as_deref(),
                priority,
                id,
                None,
                conceal.as_deref(),
            ) != id
            {
                match_add_failed = true;
            }
        } else {
            if match_add(
                &win,
                group.as_deref(),
                None,
                priority,
                id,
                s.opt(),
                conceal.as_deref(),
            ) != id
            {
                match_add_failed = true;
            }
            tv_list_unref(s.opt());
            s = crate::eval::typval::ListId::NULL;
        }
    }
    if !match_add_failed {
        rettv.value = crate::eval::typval::TvValue::Number(0);
    }
}

/// `matchadd()` function (`f_matchadd`).
pub fn f_matchadd(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut grpbuf = [0u8; NUMBUFLEN];
    let mut patbuf = [0u8; NUMBUFLEN];
    let grp = tv_get_string_buf_chk(&argvars[0], &mut grpbuf);
    let pat = tv_get_string_buf_chk(&argvars[1], &mut patbuf);
    let mut prio: i32 = 10;
    let mut id: i32 = -1;
    let mut error = false;
    let mut conceal_char: Option<std::borrow::Cow<'_, std::ffi::CStr>> = None;
    let mut win = curwin();

    rettv.value = crate::eval::typval::TvValue::Number(-1);

    let (Some(grp), Some(pat)) = (grp, pat) else {
        return;
    };
    if argvars[2].var_type() != VarType::VarUnknown {
        prio = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
        if argvars[3].var_type() != VarType::VarUnknown {
            id = tv_get_number_chk(&argvars[3], Some(&mut error)) as i32;
            if argvars[4].var_type() != VarType::VarUnknown
                && !matchadd_dict_arg(&argvars[4], &mut conceal_char, &mut win)
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

    rettv.value = crate::eval::typval::TvValue::Number(match_add(
        &win,
        Some(grp),
        Some(pat),
        prio,
        id,
        None,
        conceal_char.as_deref(),
    ) as Varnumber);
}

/// `matchaddpos()` function (`f_matchaddpos`).
pub fn f_matchaddpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.value = crate::eval::typval::TvValue::Number(-1);

    let mut buf = [0u8; NUMBUFLEN];
    let Some(group) = tv_get_string_buf_chk(&argvars[0], &mut buf) else {
        return;
    };

    if argvars[1].var_type() != VarType::VarList {
        semsg!(gettext(E_LISTARG), c"matchaddpos()",);
        return;
    }

    let l = argvars[1].value.list_opt();
    if tv_list_len(l) == 0 {
        return;
    }

    let mut error = false;
    let mut prio: i32 = 10;
    let mut id: i32 = -1;
    let mut conceal_char: Option<std::borrow::Cow<'_, std::ffi::CStr>> = None;
    let mut win = curwin();

    if argvars[2].var_type() != VarType::VarUnknown {
        prio = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
        if argvars[3].var_type() != VarType::VarUnknown {
            id = tv_get_number_chk(&argvars[3], Some(&mut error)) as i32;
            if argvars[4].var_type() != VarType::VarUnknown
                && !matchadd_dict_arg(&argvars[4], &mut conceal_char, &mut win)
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

    let l = argvars[1].value.list_opt();
    rettv.value = crate::eval::typval::TvValue::Number(match_add(
        &win,
        Some(group),
        None,
        prio,
        id,
        l,
        conceal_char.as_deref(),
    ) as Varnumber);
}

/// `matcharg()` function (`f_matcharg`).
pub fn f_matcharg(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let id = tv_get_number(&argvars[0]) as i32;

    tv_list_alloc_ret(rettv, if (1..=3).contains(&id) { 2 } else { 0 });

    if (1..=3).contains(&id) {
        if let Some((mit_hlg_id, mit_pattern)) = get_match(&curwin(), id) {
            let group_name = syn_id2name(mit_hlg_id);
            tv_list_append_string(rettv.value.list_ref(), Some(&group_name), -1);
            tv_list_append_string(rettv.value.list_ref(), mit_pattern.as_deref(), -1);
        } else {
            tv_list_append_string(rettv.value.list_ref(), None, 0);
            tv_list_append_string(rettv.value.list_ref(), None, 0);
        }
    }
}

/// `matchdelete()` function (`f_matchdelete`).
pub fn f_matchdelete(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.value = crate::eval::typval::TvValue::Number(
        if let Some(win) = get_optional_window(argvars, 1)
            && match_delete(&win, tv_get_number(&argvars[0]) as i32, true)
        {
            0
        } else {
            -1
        },
    );
}

/// `:[N]match {group} {pattern}` (`ex_match`).
pub fn ex_match(eap: &mut Exarg) {
    let mut g = None;

    let id: i32 = if eap.line2 <= 3 {
        eap.line2
    } else {
        emsg(E_INVCMD);
        return;
    };

    if !eap.skip {
        match_delete(&curwin(), id, false);
    }

    let arg_slice = eap.arg.clone();
    let arg = arg_slice.as_cstr();
    let end: &CStr;
    if ends_excmd(i32::from(arg.to_bytes().first().copied().unwrap_or(0))) != 0 {
        end = arg;
    } else {
        let arg_bytes = arg.to_bytes();
        if arg_bytes.len() >= 4 && arg_bytes[..4].eq_ignore_ascii_case(b"none") && {
            let b4 = arg_bytes.get(4).copied().unwrap_or(0);
            ascii_iswhite(b4) || ends_excmd(i32::from(b4)) != 0
        } {
            end = &arg[4..];
        } else {
            let wlen = crate::charset::skiptowhite_len(arg_bytes);
            if !eap.skip {
                g = Some(std::ffi::CString::new(&arg_bytes[..wlen]).unwrap_or_default());
            }
            let p = crate::charset::skipwhite(&arg[wlen..]);
            if p.is_empty() {
                semsg!(gettext(E_INVARG2), arg);
                return;
            }
            let p_bytes = p.to_bytes();
            let p0 = p_bytes[0];
            let rem = skip_regexp(&p_bytes[1..], i32::from(p0), 1);
            end = &p[p_bytes.len() - rem.len()..];
            if !eap.skip {
                if !end.is_empty()
                    && ends_excmd(i32::from(
                        crate::charset::skipwhite(&end[1..])
                            .to_bytes()
                            .first()
                            .copied()
                            .unwrap_or(0),
                    )) == 0
                {
                    eap.errmsg = Some(ex_errmsg(E_TRAILING_ARG, end));
                    return;
                }
                if end.to_bytes().first() != Some(&p0) {
                    semsg!(gettext(E_INVARG2), p);
                    return;
                }

                let p_next = &p[1..];
                let pat_len = (end.as_ptr() as usize).wrapping_sub(p_next.as_ptr() as usize);
                let pat_cstr = crate::memory::bytes_to_cstring(&p_next.to_bytes()[..pat_len]);
                match_add(
                    &curwin(),
                    g.as_deref(),
                    Some(pat_cstr.as_c_str()),
                    10,
                    id,
                    None,
                    None,
                );
            }
        }
    }
    eap.nextcmd = arg_slice.advance_to_opt_slice(find_nextcmd(end.to_bytes()));
}
