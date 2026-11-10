#![forbid(unsafe_code)]

//! Code for normal mode searching commands (`src/nvim/search.c`).

use crate::autocmd::{EVENT_SEARCHWRAPPED, apply_autocmds};
use crate::buffer::{FileBuf, K_UIMESSAGES, buf_get_changedtick};
use crate::change::{TAB, get_leader_len};
use crate::charset::{NUL, vim_isfilec, vim_iswordc, vim_iswordp};
use crate::cmdhist::{HIST_SEARCH, add_to_history};
use crate::cursor::{
    Colnr, CursorPos, Linenr, Lpos, check_cursor, clearpos, dec_cursor, equalpos,
    get_cursor_line_len, get_cursor_line_slice, inc_cursor, lt, ltoreq,
};
use crate::drawscreen::{
    UPD_INVERTED, UPD_SOME_VALID, UPD_VALID, redraw_all_later, redraw_curbuf_later, redraw_later,
    setcursor, show_cursor_info_later, showmode, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TypVal, VarType, Varnumber, tv_check_for_nonnull_dict_arg, tv_dict_add_nr,
    tv_dict_alloc_ret, tv_dict_find_opt, tv_dict_get_bool, tv_get_number_chk, tv_get_string_chk,
    tv_list_find_opt, tv_list_len,
};
use crate::eval::vars::{VV_SEARCHFORWARD, set_vim_var_nr};
use crate::ex_cmds::{HLF_R, RE_BOTH, RE_SUBST, SEARCH_HIS, getfile, prepare_tagpreview};
use crate::ex_docmd::{
    ACTION_SHOW, ACTION_SHOW_ALL, ACTION_SPLIT, CHECK_PATH, CMOD_KEEPPATTERNS, FIND_DEFINE,
    SEARCH_MSG, set_no_hlsearch,
};
use crate::ex_getln::gotocmdline;
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
use crate::file_search::{
    FNAME_EXP, FNAME_INCL, FNAME_REL, file_name_in_line, find_file_name_in_path,
};
use crate::fileio::vim_fgets;
use crate::fold::{fold_open_cursor, has_folding};
use crate::globals::*;
use crate::indent_c::{FM_BACKWARD, FM_BLOCKSTOP, FM_SKIPCOMM, check_linecomment};
use crate::input::char_avail;
use crate::input_cmdatom::Cmdarg;
use crate::insexpand::{
    ACTION_EXPAND, K_SHM_COMPLETIONSCAN, LSIZE, compl_status_adding, compl_status_sol,
    ctrl_x_mode_not_default, find_word_end, find_word_start, ins_compl_add_infercase,
    ins_compl_check_keys, ins_compl_interrupted, ins_compl_len,
};
use crate::mark::setpcmark;
use crate::r#match::K_CPO_SEARCH;
use crate::mbyte::{
    mb_isupper, mb_strcmp_ic, mb_strnicmp, utf_char2bytes, utf_iscomposing_first, utf_ptr2char,
    utfc_ptr2len,
};
use crate::memline::{decl, inc, incl, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len};
use crate::message::{
    emsg, give_warning, iemsg, messaging, msg, msg_check, msg_clr_eos, msg_end, msg_ext_set_kind,
    msg_home_replace, msg_outtrans, msg_outtrans_len, msg_prt_line, msg_putchar, msg_puts,
    msg_puts_hl, msg_puts_title, msg_start, msg_strtrunc, msg_trunc, semsg, smsg, verbose_enter,
    verbose_leave,
};
use crate::mouse::setmouse;
use crate::r#move::validate_cursor;
use crate::normal::{
    FM_FORWARD, K_CPO_MATCH, K_OPT_FDO_FLAG_SEARCH, K_SHM_SEARCHCOUNT, SEARCH_ECHO, SEARCH_MARK,
    SEARCH_REV, may_start_select,
};
use crate::option::shortmess;
use crate::option_vars::*;
use crate::os::fs::CFile;
use crate::os::input::{fast_breakcheck, line_breakcheck};
use crate::os::time::{Timestamp, os_delay, os_time};
use crate::path::{
    K_PATH_CMP_EXPAND, K_PATH_CMP_FULL, path_equal, path_try_shorten_fname, to_slash,
};
use crate::plines::getvcol;
use crate::profile::{Proftime, profile_passed_limit, profile_setlimit};
use crate::regexp::{
    MAGIC_ALL, MAGIC_ON, MAXCOL, RE_MAGIC, RegMagic, Regmatch, Regmmatch, skip_regexp_ex,
    skip_regexp_magic, vim_regcomp, vim_regexec, vim_regexec_multi,
};
use crate::state::MODE_SHOWMATCH;
use crate::strings::{format_to_buf, reverse_text, tr, vim_strchr};
use crate::textformat::Oparg;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_cursor_shape, ui_flush, ui_has, vim_beep};
use crate::window::{WinInfoInfo, reset_binding, win_enter, win_split, win_valid};
use std::ffi::{CStr, CString};
use std::ptr;

pub type Direction = i32;
pub(crate) const FORWARD: Direction = 1;
pub(crate) const BACKWARD: Direction = -1;
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::vim::HLF_N;
use crate::cmdexpand::{HLF_D, RE_LAST, SEARCH_NOOF, SEARCH_OPT, SEARCH_PEEK, SEARCH_START};
use crate::eval::funcs::{MT_LINE_WISE, RE_SEARCH, SEARCH_COL, SEARCH_END, SEARCH_KEEP};
use crate::globals::IOSIZE;
pub type AdditionalData = crate::msgpack_rpc::unpacker::AdditionalData;

const SEARCH_STAT_DEF_TIMEOUT: i32 = 40;
const SEARCH_STAT_BUF_LEN: usize = 16;

const K_CPO_LINEOFF: i32 = b'o' as i32;
const K_CPO_SCOLON: i32 = b';' as i32;
pub(crate) const K_CPO_MATCHBSL: i32 = b'M' as i32;
const K_CPO_SHOWMATCH: i32 = b'm' as i32;

pub(crate) const K_SHM_SEARCH: i32 = b's' as i32;

const K_OPT_BO_FLAG_SHOWMATCH: u32 = 0x2000;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SearchOffset {
    pub dir: u8,
    pub line: bool,
    pub end: bool,
    pub off: i64,
}

impl SearchOffset {
    pub const fn new() -> Self {
        Self {
            dir: 0,
            line: false,
            end: false,
            off: 0,
        }
    }
}

#[derive(Clone)]
pub struct SearchPattern {
    pub pat: Option<CString>,
    pub magic: bool,
    pub no_scs: bool,
    pub timestamp: Timestamp,
    pub off: SearchOffset,
    pub additional_data: Option<AdditionalData>,
}

impl SearchPattern {
    pub const fn new() -> Self {
        Self {
            pat: None,
            magic: false,
            no_scs: false,
            timestamp: 0,
            off: SearchOffset::new(),
            additional_data: None,
        }
    }

    #[inline(always)]
    pub fn pat_ptr(&self) -> Option<&CStr> {
        self.pat.as_deref()
    }

    #[inline(always)]
    pub fn patlen(&self) -> usize {
        self.pat.as_ref().map_or(0, |s| s.as_bytes().len())
    }
}

impl Default for SearchPattern {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SearchitArg {
    pub sa_stop_lnum: Linenr,
    pub sa_tm: Option<Proftime>,
    pub sa_timed_out: i32,
    pub sa_wrapped: i32,
}

impl SearchitArg {
    pub const fn new() -> Self {
        Self {
            sa_stop_lnum: 0,
            sa_tm: None,
            sa_timed_out: 0,
            sa_wrapped: 0,
        }
    }
}

impl Default for SearchitArg {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Searchstat {
    pub cur: i32,
    pub cnt: i32,
    pub exact_match: bool,
    pub incomplete: i32,
    pub last_maxcount: i32,
}

#[derive(Default)]
struct SearchedFile {
    fp: Option<CFile>,
    name: Option<CString>,
    lnum: Linenr,
    matched: i32,
}

impl SearchedFile {
    fn name_ptr(&self) -> Option<&CStr> {
        self.name.as_deref()
    }
}

const E_SEARCH_HIT_TOP_WITHOUT_MATCH_FOR_STR: &CStr = c"E384: Search hit TOP without match for: %s";
const E_SEARCH_HIT_BOTTOM_WITHOUT_MATCH_FOR_STR: &CStr =
    c"E385: Search hit BOTTOM without match for: %s";

const EMPTY_SEARCH_OFFSET: SearchOffset = SearchOffset {
    dir: b'/',
    line: false,
    end: false,
    off: 0,
};

const EMPTY_SEARCH_PATTERN: SearchPattern = SearchPattern {
    pat: None,
    magic: true,
    no_scs: false,
    timestamp: 0,
    off: EMPTY_SEARCH_OFFSET,
    additional_data: None,
};

struct SearchModuleState {
    spats: [SearchPattern; 2],
    last_idx: i32,
    lastc: [u8; 2],
    lastcdir: Direction,
    last_t_cmd: bool,
    lastc_bytes: [u8; MAX_SCHAR_SIZE + 1],
    lastc_bytelen: i32,
    saved_spats: [SearchPattern; 2],
    saved_mr_pattern: Option<CString>,
    saved_spats_last_idx: i32,
    saved_spats_no_hlsearch: bool,
    mr_pattern: Option<CString>,
    save_level: i32,
    saved_last_search_spat: SearchPattern,
    did_save_last_search_spat: i32,
    saved_last_idx: i32,
    saved_no_hlsearch: bool,
    saved_search_match_endcol: Colnr,
    saved_search_match_lines: Linenr,
    stat_lastpos: CursorPos,
    stat_cur: i32,
    stat_cnt: i32,
    stat_exact_match: bool,
    stat_incomplete: i32,
    stat_last_maxcount: i32,
    stat_chgtick: i32,
    stat_lastpat: Option<CString>,
    stat_lbuf: crate::globals::BufPtr,
}

impl SearchModuleState {
    const fn new() -> Self {
        Self {
            spats: [EMPTY_SEARCH_PATTERN, EMPTY_SEARCH_PATTERN],
            last_idx: 0,
            lastc: [0, 0],
            lastcdir: FORWARD,
            last_t_cmd: true,
            lastc_bytes: [0; MAX_SCHAR_SIZE + 1],
            lastc_bytelen: 1,
            saved_spats: [EMPTY_SEARCH_PATTERN, EMPTY_SEARCH_PATTERN],
            saved_mr_pattern: None,
            saved_spats_last_idx: 0,
            saved_spats_no_hlsearch: false,
            mr_pattern: None,
            save_level: 0,
            saved_last_search_spat: EMPTY_SEARCH_PATTERN,
            did_save_last_search_spat: 0,
            saved_last_idx: 0,
            saved_no_hlsearch: false,
            saved_search_match_endcol: 0,
            saved_search_match_lines: 0,
            stat_lastpos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            stat_cur: 0,
            stat_cnt: 0,
            stat_exact_match: false,
            stat_incomplete: 0,
            stat_last_maxcount: 0,
            stat_chgtick: 0,
            stat_lastpat: None,
            stat_lbuf: crate::globals::BufPtr::NULL,
        }
    }
}

thread_local! {
    static SEARCH_STATE: std::cell::RefCell<SearchModuleState> =
        const { std::cell::RefCell::new(SearchModuleState::new()) };
}

#[inline(always)]
fn with_srch<R>(f: impl FnOnce(&SearchModuleState) -> R) -> R {
    SEARCH_STATE.with(|s| f(&s.borrow()))
}

#[inline(always)]
fn with_srch_mut<R>(f: impl FnOnce(&mut SearchModuleState) -> R) -> R {
    SEARCH_STATE.with(|s| f(&mut s.borrow_mut()))
}

#[inline(always)]
const fn empty_pos(a: CursorPos) -> bool {
    a.lnum == 0 && a.col == 0 && a.coladd == 0
}

#[inline(always)]
const fn getfile_success(x: crate::buffer::GetFileResult) -> bool {
    x.is_success()
}

fn find_substr(haystack: &CStr, needle: &[u8]) -> bool {
    haystack
        .to_bytes()
        .windows(needle.len())
        .any(|w| w == needle)
}

/// Translate search pattern for `vim_regcomp()`.
pub fn search_regcomp(
    pat_cstr: &CStr,
    mut patlen: usize,
    used_pat: Option<&mut CString>,
    pat_save: i32,
    pat_use: i32,
    options: i32,
    mut magic: bool,
    regmatch: &mut Regmmatch,
) -> bool {
    let owned_pat: Option<CString>;
    let mut pat_from_spat: Option<usize> = None;
    let mut pat_ref = pat_cstr;
    {
        g_st().rc_did_emsg = false;

        if pat_cstr.is_empty() {
            let (spat_idx, has_pat, cloned_pat, plen, pmagic, no_scs) = with_srch(|s| {
                let i = if pat_use == RE_LAST {
                    s.last_idx
                } else {
                    pat_use
                } as usize;
                (
                    i,
                    s.spats[i].pat.is_some(),
                    s.spats[i].pat.clone(),
                    s.spats[i].patlen(),
                    s.spats[i].magic,
                    s.spats[i].no_scs,
                )
            });
            if !has_pat {
                if pat_use == RE_SUBST {
                    emsg(tr(E_NOPRESUB));
                } else {
                    emsg(tr(E_NOPREVRE));
                }
                g_st().rc_did_emsg = true;
                return false;
            }
            pat_from_spat = Some(spat_idx);
            owned_pat = cloned_pat;
            pat_ref = owned_pat.as_deref().unwrap_or(c"");
            patlen = plen;
            magic = pmagic;
            g_st().search_global.no_smartcase = no_scs;
        } else if (options & SEARCH_HIS) != 0 {
            let pat_bytes = pat_ref.to_bytes();
            add_to_history(
                HIST_SEARCH,
                &pat_bytes[..patlen.min(pat_bytes.len())],
                true,
                i32::from(NUL),
            );
        }

        if let Some(used_pat) = used_pat {
            *used_pat = pat_ref.to_owned();
        }

        let mr = if curwin_mut().w_onebuf_opt.wo_rl
            && curwin_mut().w_onebuf_opt.wo_rlc.first_byte() == b's'
        {
            Some(reverse_text(pat_ref.to_bytes()))
        } else {
            let pat_bytes = pat_ref.to_bytes();
            let pat_slice = &pat_bytes[..patlen.min(pat_bytes.len())];
            Some(CString::new(pat_slice).unwrap_or_default())
        };
        with_srch_mut(|s| s.mr_pattern = mr);

        if (options & SEARCH_KEEP) == 0 && (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) == 0 {
            if (pat_save == RE_SEARCH || pat_save == RE_BOTH)
                && pat_from_spat != Some(RE_SEARCH as usize)
            {
                save_re_pat(RE_SEARCH, pat_ref, patlen, i32::from(magic));
            }
            if (pat_save == RE_SUBST || pat_save == RE_BOTH)
                && pat_from_spat != Some(RE_SUBST as usize)
            {
                save_re_pat(RE_SUBST, pat_ref, patlen, i32::from(magic));
            }
        }

        regmatch.rmm_ic = ignorecase(pat_ref);
        regmatch.rmm_maxcol = 0;
        regmatch.set_prog(vim_regcomp(pat_ref, if magic { RE_MAGIC } else { 0 }));
        if regmatch.regprog.is_null() {
            return false;
        }
        true
    }
}

/// Get search pattern used by `search_regcomp()`.
pub fn get_search_pat() -> Option<CString> {
    with_srch(|s| s.mr_pattern.clone())
}

pub fn save_re_pat(idx: i32, pat_cstr: &CStr, patlen: usize, magic: i32) {
    let pat = pat_cstr.as_ptr();
    let i = idx as usize;
    if with_srch(|s| {
        s.spats[i]
            .pat_ptr()
            .is_some_and(|p| ptr::eq(p.as_ptr(), pat))
    }) {
        return;
    }

    let pat_bytes = pat_cstr.to_bytes();
    let new_pat = CString::new(&pat_bytes[..patlen.min(pat_bytes.len())]).unwrap_or_default();
    let now = os_time();
    with_srch_mut(|s| {
        free_spat(&mut s.spats[i]);
        s.spats[i].pat = Some(new_pat);
        s.spats[i].magic = magic != 0;
        s.spats[i].no_scs = g_st().search_global.no_smartcase;
        s.spats[i].timestamp = now;
        s.spats[i].additional_data = None;
        s.last_idx = idx;
    });
    if opt_vars().p_hls {
        redraw_all_later(UPD_SOME_VALID);
    }
    set_no_hlsearch(false);
}

pub fn save_search_patterns() {
    with_srch_mut(|s| {
        let prev = s.save_level;
        s.save_level += 1;
        if prev != 0 {
            return;
        }

        for i in 0..2 {
            s.saved_spats[i] = s.spats[i].clone();
        }
        s.saved_mr_pattern = s.mr_pattern.clone();
        s.saved_spats_last_idx = s.last_idx;
        s.saved_spats_no_hlsearch = g_st().search_global.no_hlsearch;
    });
}

pub fn restore_search_patterns() {
    let Some(no_hlsearch) = with_srch_mut(|s| {
        s.save_level -= 1;
        if s.save_level != 0 {
            return None;
        }

        for i in 0..2 {
            free_spat(&mut s.spats[i]);
            s.spats[i] = std::mem::replace(&mut s.saved_spats[i], EMPTY_SEARCH_PATTERN);
        }
        s.mr_pattern = s.saved_mr_pattern.take();
        s.last_idx = s.saved_spats_last_idx;
        Some(s.saved_spats_no_hlsearch)
    }) else {
        return;
    };
    set_vv_searchforward();
    set_no_hlsearch(no_hlsearch);
}

#[inline(always)]
fn free_spat(spat: &mut SearchPattern) {
    spat.pat = None;
    spat.additional_data = None;
}

pub fn free_search_patterns() {
    with_srch_mut(|s| {
        free_spat(&mut s.spats[0]);
        free_spat(&mut s.spats[1]);
        s.spats = [EMPTY_SEARCH_PATTERN, EMPTY_SEARCH_PATTERN];
        s.mr_pattern = None;
    });
}

pub fn save_last_search_pattern() {
    with_srch_mut(|s| {
        s.did_save_last_search_spat += 1;
        if s.did_save_last_search_spat != 1 {
            return;
        }

        s.saved_last_search_spat = s.spats[RE_SEARCH as usize].clone();
        s.saved_last_idx = s.last_idx;
        s.saved_no_hlsearch = g_st().search_global.no_hlsearch;
    });
}

pub fn restore_last_search_pattern() {
    let outcome = with_srch_mut(|s| {
        s.did_save_last_search_spat -= 1;
        if s.did_save_last_search_spat > 0 {
            return Ok(None);
        }
        if s.did_save_last_search_spat != 0 {
            return Err(());
        }

        s.spats[RE_SEARCH as usize] =
            std::mem::replace(&mut s.saved_last_search_spat, EMPTY_SEARCH_PATTERN);
        s.last_idx = s.saved_last_idx;
        Ok(Some(s.saved_no_hlsearch))
    });
    match outcome {
        Ok(Some(no_hlsearch)) => {
            set_vv_searchforward();
            set_no_hlsearch(no_hlsearch);
        }
        Ok(None) => {}
        Err(()) => {
            iemsg(
                c"restore_last_search_pattern() called more often than save_last_search_pattern()",
            );
        }
    }
}

fn save_incsearch_state() {
    with_srch_mut(|s| {
        s.saved_search_match_endcol = g_st().search_global.match_endcol;
        s.saved_search_match_lines = g_st().search_global.match_lines;
    });
}

fn restore_incsearch_state() {
    let (endcol, lines) = with_srch(|s| (s.saved_search_match_endcol, s.saved_search_match_lines));
    g_st().search_global.match_endcol = endcol;
    g_st().search_global.match_lines = lines;
}

pub fn ignorecase(pat: &CStr) -> bool {
    ignorecase_opt(pat, opt_vars().p_ic, opt_vars().p_scs)
}

pub fn ignorecase_opt(pat: &CStr, ic_in: bool, scs: bool) -> bool {
    let mut ic = ic_in;
    if ic
        && !g_st().search_global.no_smartcase
        && scs
        && !(ctrl_x_mode_not_default() && curbuf_mut().b_p_inf)
    {
        ic = !pat_has_uppercase(pat);
    }
    g_st().search_global.no_smartcase = false;
    ic
}

pub fn pat_has_uppercase(pat: &CStr) -> bool {
    let mut magic_val: RegMagic = MAGIC_ON;

    skip_regexp_magic(
        pat.to_bytes(),
        i32::from(NUL),
        i32::from(opt_vars().p_magic),
        Some(&mut magic_val),
    );

    let mut rem = pat.to_bytes();
    while !rem.is_empty() {
        let l = crate::mbyte::utfc_slice2len(rem).min(rem.len());

        if l > 1 {
            if mb_isupper(crate::mbyte::utf_slice2char(rem)) {
                return true;
            }
            rem = &rem[l..];
        } else if rem[0] == b'\\' && magic_val <= MAGIC_ON {
            if matches!(rem.get(1), Some(&b'_' | &b'%')) && rem.len() >= 3 {
                rem = &rem[3..];
            } else if rem.len() >= 2 {
                rem = &rem[2..];
            } else {
                rem = &rem[1..];
            }
        } else if matches!(rem[0], b'%' | b'_') && magic_val == MAGIC_ALL {
            if rem.len() >= 2 {
                rem = &rem[2..];
            } else {
                rem = &rem[1..];
            }
        } else if mb_isupper(i32::from(rem[0])) {
            return true;
        } else {
            rem = &rem[1..];
        }
    }
    false
}

pub fn last_csearch() -> crate::strings::InlineCStr<{ MAX_SCHAR_SIZE + 1 }> {
    with_srch(|s| crate::strings::InlineCStr(s.lastc_bytes))
}

pub fn last_csearch_forward() -> i32 {
    with_srch(|s| i32::from(s.lastcdir == FORWARD))
}

pub fn last_csearch_until() -> i32 {
    with_srch(|s| i32::from(s.last_t_cmd))
}

pub fn set_last_csearch(c: i32, s: &[u8]) {
    let bytes = crate::memory::bytes_until_nul(s);
    let len = bytes.len().min(MAX_SCHAR_SIZE);
    with_srch_mut(|st| {
        st.lastc[0] = c as u8;
        st.lastc_bytelen = len as i32;
        st.lastc_bytes = [0; MAX_SCHAR_SIZE + 1];
        if len != 0 {
            st.lastc_bytes[..len].copy_from_slice(&bytes[..len]);
        }
    });
}

pub fn set_csearch_direction(cdir: Direction) {
    with_srch_mut(|s| s.lastcdir = cdir);
}

pub fn set_csearch_until(t_cmd: i32) {
    with_srch_mut(|s| s.last_t_cmd = t_cmd != 0);
}

pub fn last_search_pat() -> Option<CString> {
    with_srch(|s| s.spats[s.last_idx as usize].pat.clone())
}

pub fn reset_search_dir() {
    with_srch_mut(|s| s.spats[0].off.dir = b'/');
    set_vv_searchforward();
}

pub fn set_last_search_pat(s: Option<&CStr>, idx: i32, magic: i32, setlast: bool) {
    let i = idx as usize;
    let new_pat = s.filter(|s| !s.is_empty()).map(CStr::to_owned);
    let now = os_time();
    with_srch_mut(|st| {
        free_spat(&mut st.spats[i]);
        st.spats[i].pat = new_pat;
        st.spats[i].timestamp = now;
        st.spats[i].additional_data = None;
        st.spats[i].magic = magic != 0;
        st.spats[i].no_scs = false;
        st.spats[i].off.dir = b'/';
        st.spats[i].off.line = false;
        st.spats[i].off.end = false;
        st.spats[i].off.off = 0;
        if setlast {
            st.last_idx = idx;
        }
        if st.save_level != 0 {
            free_spat(&mut st.saved_spats[i]);
            st.saved_spats[i] = st.spats[0].clone();
            st.saved_spats[i].pat = st.spats[i].pat.clone();
            st.saved_spats_last_idx = st.last_idx;
        }
    });
    set_vv_searchforward();
    let is_last = with_srch(|s| idx == s.last_idx);
    if opt_vars().p_hls && is_last && !g_st().search_global.no_hlsearch {
        redraw_all_later(UPD_SOME_VALID);
    }
}

pub fn last_pat_prog(regmatch: &mut Regmmatch) {
    let (has_pat, last_idx) =
        with_srch(|s| (s.spats[s.last_idx as usize].pat.is_some(), s.last_idx));
    if !has_pat {
        regmatch.free_prog();
        return;
    }
    g_st().emsg_off += 1;
    search_regcomp(
        c"",
        0,
        None,
        0,
        last_idx,
        SEARCH_KEEP,
        opt_vars().p_magic,
        regmatch,
    );
    g_st().emsg_off -= 1;
}

pub fn searchit(
    win: Option<&mut WinInfoInfo>,
    buf: &mut FileBuf,
    pos: &mut CursorPos,
    mut end_pos: Option<&mut CursorPos>,
    dir: Direction,
    pat: &CStr,
    patlen: usize,
    mut count: i32,
    options: i32,
    pat_use: i32,
    magic: bool,
    mut extra_arg: Option<&mut SearchitArg>,
) -> i32 {
    let win = crate::globals::GraphPtr::from(win);
    {
        let mut found: i32;
        let mut lnum: Linenr;
        let mut regmatch: Regmmatch = Regmmatch::new();
        let mut ptr_line;
        let mut matchcol: Colnr;
        let mut endpos: Lpos;
        let mut matchpos: Lpos;
        let mut extra_col: i32;
        let mut start_char_len: i32;
        let mut match_ok: bool;
        let mut nmatched: i32;
        let mut submatch: i32 = 0;
        let mut first_match = true;
        let called_emsg_before = g_st().called_emsg;
        let mut break_loop = false;
        let mut stop_lnum: Linenr = 0;
        let mut tm: Option<Proftime> = None;

        if let Some(ref ea) = extra_arg {
            stop_lnum = ea.sa_stop_lnum;
            tm = ea.sa_tm;
        }

        if !search_regcomp(
            pat,
            patlen,
            None,
            RE_SEARCH,
            pat_use,
            options & (SEARCH_HIS + SEARCH_KEEP),
            magic,
            &mut regmatch,
        ) {
            if (options & SEARCH_MSG) != 0 && !g_st().rc_did_emsg {
                semsg!(tr(c"E383: Invalid search string: %s"), get_search_pat());
            }
            return 0;
        }

        let search_from_match_end =
            crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_SEARCH);

        loop {
            if pos.col == MAXCOL {
                start_char_len = 0;
            } else if pos.lnum >= 1 && pos.lnum <= buf.b_ml.ml_line_count && pos.col < MAXCOL - 2 {
                let line_slice = crate::memline::ml_get_buf_slice(buf, pos.lnum);
                if (line_slice.len() as Colnr) <= pos.col {
                    start_char_len = 1;
                } else {
                    start_char_len =
                        crate::mbyte::utfc_slice2len(&line_slice[pos.col as usize..]) as i32;
                }
            } else {
                start_char_len = 1;
            }
            if dir == FORWARD {
                extra_col = if (options & SEARCH_START) != 0 {
                    0
                } else {
                    start_char_len
                };
            } else {
                extra_col = if (options & SEARCH_START) != 0 {
                    start_char_len
                } else {
                    0
                };
            }

            let start_pos = *pos;
            found = 0;
            let mut at_first_line = true;
            if pos.lnum == 0 {
                pos.lnum = 1;
                pos.col = 0;
                at_first_line = false;
            }

            if dir == BACKWARD && start_pos.col == 0 && (options & SEARCH_START) == 0 {
                lnum = pos.lnum - 1;
                at_first_line = false;
            } else {
                lnum = pos.lnum;
            }

            for loop_idx in 0..=1 {
                while lnum > 0 && lnum <= buf.b_ml.ml_line_count {
                    if stop_lnum != 0
                        && (if dir == FORWARD {
                            lnum > stop_lnum
                        } else {
                            lnum < stop_lnum
                        })
                    {
                        break;
                    }
                    if tm.is_some_and(profile_passed_limit) {
                        break;
                    }

                    let col = if at_first_line && (options & SEARCH_COL) != 0 {
                        pos.col
                    } else {
                        0
                    };
                    nmatched = vim_regexec_multi(
                        &mut regmatch,
                        win.opt_mut(),
                        &mut *buf,
                        lnum,
                        col,
                        tm,
                        extra_arg.as_deref_mut().map(|ea| &mut ea.sa_timed_out),
                    );
                    if regmatch.regprog.is_null() {
                        break;
                    }
                    if g_st().called_emsg > called_emsg_before
                        || extra_arg.as_deref().is_some_and(|ea| ea.sa_timed_out != 0)
                    {
                        break;
                    }
                    if nmatched > 0 {
                        matchpos = regmatch.startpos[0];
                        endpos = regmatch.endpos[0];
                        submatch = first_submatch(&regmatch);
                        let mut ptr_line_cstr: &CStr =
                            if lnum + matchpos.lnum > buf.b_ml.ml_line_count {
                                c""
                            } else {
                                ml_get_buf(buf, lnum + matchpos.lnum)
                            };
                        ptr_line = ptr_line_cstr.as_ptr().cast_mut();

                        if dir == FORWARD && at_first_line {
                            match_ok = true;

                            while matchpos.lnum == 0
                                && (if (options & SEARCH_END) != 0 && first_match {
                                    nmatched == 1 && endpos.col - 1 < start_pos.col + extra_col
                                } else {
                                    matchpos.col
                                        - i32::from(
                                            *crate::globals::GraphPtr::of(
                                                ptr_line.wrapping_add(matchpos.col as usize),
                                            )
                                            .borrow()
                                                == 0,
                                        )
                                        < start_pos.col + extra_col
                                })
                            {
                                if search_from_match_end {
                                    if nmatched > 1 {
                                        match_ok = false;
                                        break;
                                    }
                                    matchcol = endpos.col;
                                    if matchcol == matchpos.col
                                        && *crate::globals::GraphPtr::of(
                                            ptr_line.wrapping_add(matchcol as usize),
                                        )
                                        .borrow()
                                            != 0
                                    {
                                        matchcol += utfc_ptr2len(
                                            &ptr_line_cstr[(matchcol as usize)
                                                .min(ptr_line_cstr.to_bytes().len())..],
                                        );
                                    }
                                } else {
                                    matchcol = regmatch.rmm_matchcol;
                                    if *crate::globals::GraphPtr::of(
                                        ptr_line.wrapping_add(matchcol as usize),
                                    )
                                    .borrow()
                                        != 0
                                    {
                                        matchcol += utfc_ptr2len(
                                            &ptr_line_cstr[(matchcol as usize)
                                                .min(ptr_line_cstr.to_bytes().len())..],
                                        );
                                    }
                                }
                                if matchcol == 0 && (options & SEARCH_START) != 0 {
                                    break;
                                }
                                if *crate::globals::GraphPtr::of(
                                    ptr_line.wrapping_add(matchcol as usize),
                                )
                                .borrow()
                                    == 0
                                    || {
                                        nmatched = vim_regexec_multi(
                                            &mut regmatch,
                                            win.opt_mut(),
                                            &mut *buf,
                                            lnum,
                                            matchcol,
                                            tm,
                                            extra_arg.as_deref_mut().map(|ea| &mut ea.sa_timed_out),
                                        );
                                        nmatched == 0
                                    }
                                {
                                    match_ok = false;
                                    break;
                                }
                                if regmatch.regprog.is_null() {
                                    break;
                                }
                                matchpos = regmatch.startpos[0];
                                endpos = regmatch.endpos[0];
                                submatch = first_submatch(&regmatch);

                                if matchpos.lnum != 0 {
                                    break;
                                }
                                ptr_line_cstr = ml_get_buf(buf, lnum);
                                ptr_line = ptr_line_cstr.as_ptr().cast_mut();
                            }
                            if !match_ok {
                                lnum += dir;
                                at_first_line = false;
                                continue;
                            }
                        }
                        if dir == BACKWARD {
                            match_ok = false;
                            loop {
                                if loop_idx != 0
                                    || (if (options & SEARCH_END) != 0 {
                                        lnum + regmatch.endpos[0].lnum < start_pos.lnum
                                            || (lnum + regmatch.endpos[0].lnum == start_pos.lnum
                                                && regmatch.endpos[0].col - 1
                                                    < start_pos.col + extra_col)
                                    } else {
                                        lnum + regmatch.startpos[0].lnum < start_pos.lnum
                                            || (lnum + regmatch.startpos[0].lnum == start_pos.lnum
                                                && regmatch.startpos[0].col
                                                    < start_pos.col + extra_col)
                                    })
                                {
                                    match_ok = true;
                                    matchpos = regmatch.startpos[0];
                                    endpos = regmatch.endpos[0];
                                    submatch = first_submatch(&regmatch);
                                } else {
                                    break;
                                }

                                if search_from_match_end {
                                    if nmatched > 1 {
                                        break;
                                    }
                                    matchcol = endpos.col;
                                    if matchcol == matchpos.col
                                        && *crate::globals::GraphPtr::of(
                                            ptr_line.wrapping_add(matchcol as usize),
                                        )
                                        .borrow()
                                            != 0
                                    {
                                        matchcol += utfc_ptr2len(
                                            &ptr_line_cstr[(matchcol as usize)
                                                .min(ptr_line_cstr.to_bytes().len())..],
                                        );
                                    }
                                } else {
                                    if matchpos.lnum > 0 {
                                        break;
                                    }
                                    matchcol = matchpos.col;
                                    if *crate::globals::GraphPtr::of(
                                        ptr_line.wrapping_add(matchcol as usize),
                                    )
                                    .borrow()
                                        != 0
                                    {
                                        matchcol += utfc_ptr2len(
                                            &ptr_line_cstr[(matchcol as usize)
                                                .min(ptr_line_cstr.to_bytes().len())..],
                                        );
                                    }
                                }
                                if *crate::globals::GraphPtr::of(
                                    ptr_line.wrapping_add(matchcol as usize),
                                )
                                .borrow()
                                    == 0
                                    || {
                                        nmatched = vim_regexec_multi(
                                            &mut regmatch,
                                            win.opt_mut(),
                                            &mut *buf,
                                            lnum + matchpos.lnum,
                                            matchcol,
                                            tm,
                                            extra_arg.as_deref_mut().map(|ea| &mut ea.sa_timed_out),
                                        );
                                        nmatched == 0
                                    }
                                {
                                    if tm.is_some_and(profile_passed_limit) {
                                        match_ok = false;
                                    }
                                    break;
                                }
                                if regmatch.regprog.is_null() {
                                    break;
                                }
                                ptr_line_cstr = ml_get_buf(buf, lnum + matchpos.lnum);
                                ptr_line = ptr_line_cstr.as_ptr().cast_mut();
                            }

                            if !match_ok {
                                lnum += dir;
                                at_first_line = false;
                                continue;
                            }
                        }

                        if (options & SEARCH_END) != 0
                            && (options & SEARCH_NOOF) == 0
                            && !(matchpos.lnum == endpos.lnum && matchpos.col == endpos.col)
                        {
                            pos.lnum = lnum + endpos.lnum;
                            pos.col = endpos.col;
                            if endpos.col == 0 {
                                if pos.lnum > 1 {
                                    pos.lnum -= 1;
                                    pos.col = ml_get_buf_len(buf, pos.lnum);
                                }
                            } else {
                                pos.col -= 1;
                                if pos.lnum <= buf.b_ml.ml_line_count {
                                    let line_bytes = ml_get_buf(buf, pos.lnum).to_bytes();
                                    pos.col -= crate::mbyte::utf_head_off_slice(
                                        line_bytes,
                                        pos.col as usize,
                                    );
                                }
                            }
                            if let Some(end_pos) = end_pos.as_deref_mut() {
                                end_pos.lnum = lnum + matchpos.lnum;
                                end_pos.col = matchpos.col;
                            }
                        } else {
                            pos.lnum = lnum + matchpos.lnum;
                            pos.col = matchpos.col;
                            if let Some(end_pos) = end_pos.as_deref_mut() {
                                end_pos.lnum = lnum + endpos.lnum;
                                end_pos.col = endpos.col;
                            }
                        }
                        pos.coladd = 0;
                        if let Some(end_pos) = end_pos.as_deref_mut() {
                            end_pos.coladd = 0;
                        }
                        found = 1;
                        first_match = false;

                        g_st().search_global.match_lines = endpos.lnum - matchpos.lnum;
                        g_st().search_global.match_endcol = endpos.col;
                        break;
                    }
                    line_breakcheck();
                    if g_st().got_int {
                        break;
                    }

                    if (options & SEARCH_PEEK) != 0
                        && ((lnum - pos.lnum) & 0x3f) == 0
                        && char_avail()
                    {
                        break_loop = true;
                        break;
                    }

                    if loop_idx != 0 && lnum == start_pos.lnum {
                        break;
                    }

                    lnum += dir;
                    at_first_line = false;
                }
                at_first_line = false;

                if regmatch.regprog.is_null() {
                    break;
                }

                if !opt_vars().p_ws
                    || stop_lnum != 0
                    || g_st().got_int
                    || g_st().called_emsg > called_emsg_before
                    || extra_arg.as_deref().is_some_and(|ea| ea.sa_timed_out != 0)
                    || break_loop
                    || found != 0
                    || loop_idx != 0
                {
                    break;
                }

                lnum = if dir == BACKWARD {
                    buf.b_ml.ml_line_count
                } else {
                    1
                };
                if !shortmess(K_SHM_SEARCH)
                    && shortmess(K_SHM_SEARCHCOUNT)
                    && (options & SEARCH_MSG) != 0
                {
                    give_warning(
                        tr(if dir == BACKWARD {
                            TOP_BOT_MSG
                        } else {
                            BOT_TOP_MSG
                        }),
                        true,
                        false,
                    );
                }
                if let Some(ref mut ea) = extra_arg {
                    ea.sa_wrapped = 1;
                }
            }
            if g_st().got_int
                || g_st().called_emsg > called_emsg_before
                || extra_arg.as_deref().is_some_and(|ea| ea.sa_timed_out != 0)
                || break_loop
            {
                break;
            }
            count -= 1;
            if !(count > 0 && found != 0) {
                break;
            }
        }

        regmatch.free_prog();

        if found == 0 {
            if g_st().got_int {
                emsg(tr(E_INTERR));
            } else if (options & SEARCH_MSG) == SEARCH_MSG {
                if opt_vars().p_ws {
                    semsg!(tr(E_PATNOTF2), get_search_pat());
                } else if lnum == 0 {
                    semsg!(tr(E_SEARCH_HIT_TOP_WITHOUT_MATCH_FOR_STR), get_search_pat(),);
                } else {
                    semsg!(
                        tr(E_SEARCH_HIT_BOTTOM_WITHOUT_MATCH_FOR_STR),
                        get_search_pat(),
                    );
                }
            }
            return 0;
        }

        if pos.lnum > buf.b_ml.ml_line_count {
            pos.lnum = buf.b_ml.ml_line_count;
            pos.col = ml_get_buf_len(buf, pos.lnum);
            if pos.col > 0 {
                pos.col -= 1;
            }
        }

        submatch + 1
    }
}

pub fn set_search_direction(cdir: i32) {
    with_srch_mut(|s| s.spats[0].off.dir = cdir as u8);
}

fn set_vv_searchforward() {
    set_vim_var_nr(
        VV_SEARCHFORWARD,
        Varnumber::from(with_srch(|s| s.spats[0].off.dir == b'/')),
    );
}

fn first_submatch(rp: &Regmmatch) -> i32 {
    for submatch in 1..=9 {
        if rp.startpos[submatch].lnum >= 0 {
            return submatch as i32;
        }
    }
    0
}

pub fn parse_search_pattern_offset(
    pat_buf: &mut Vec<u8>,
    pat_off: &mut usize,
    patlen: &mut usize,
    search_delim: i32,
    options: i32,
    had_copy: &mut bool,
    offset: &mut SearchOffset,
) -> (i32, CString) {
    if pat_buf.get(*pat_off).copied().unwrap_or(0) == 0 {
        return (0, CString::default());
    }

    let mut cmdlen: i32 = 0;
    let mut strcopy: Vec<u8> = Vec::new();

    let off = skip_regexp_ex(
        &mut pat_buf[*pat_off..],
        search_delim,
        i32::from(opt_vars().p_magic),
        (!*had_copy).then_some(&mut strcopy),
        None,
        None,
    );
    let mut p_off = if !*had_copy && !strcopy.is_empty() {
        *had_copy = true;
        let len = crate::memory::bytes_until_nul(&strcopy).len();
        cmdlen += (*patlen).wrapping_sub(len) as i32;
        *patlen = len;
        *pat_buf = strcopy;
        *pat_off = 0;
        off
    } else {
        off
    };

    let active_slice = crate::memory::bytes_until_nul(&pat_buf[*pat_off..]);
    let searchstrlen = if active_slice.get(p_off) == Some(&(search_delim as u8)) {
        let len = p_off;
        p_off += 1;
        len
    } else {
        *patlen
    };
    let searchstr =
        CString::new(&active_slice[..searchstrlen.min(active_slice.len())]).unwrap_or_default();

    let p_bytes = active_slice.get(p_off..).unwrap_or(&[]);
    let mut rem = p_bytes;
    offset.line = false;
    offset.end = false;
    offset.off = 0;
    if rem
        .first()
        .is_some_and(|&b| matches!(b, b'+' | b'-') || b.is_ascii_digit())
    {
        offset.line = true;
    } else if (options & SEARCH_OPT) != 0
        && rem
            .first()
            .is_some_and(|&b| matches!(b, b'e' | b's' | b'b'))
    {
        if rem[0] == b'e' {
            offset.end = true;
        }
        rem = &rem[1..];
    }
    if rem
        .first()
        .is_some_and(|&b| matches!(b, b'+' | b'-') || b.is_ascii_digit())
    {
        if rem[0].is_ascii_digit() || rem.get(1).is_some_and(u8::is_ascii_digit) {
            offset.off = crate::charset::parse_i64_prefix(rem) as _;
        } else if rem[0] == b'-' {
            offset.off = -1;
        } else {
            offset.off = 1;
        }
        rem = &rem[1..];
        while rem.first().is_some_and(u8::is_ascii_digit) {
            rem = &rem[1..];
        }
    }
    p_off += p_bytes.len() - rem.len();

    let consumed = p_off;
    cmdlen += consumed as i32;
    *patlen = (*patlen).wrapping_sub(consumed);
    *pat_off += consumed;

    (cmdlen, searchstr)
}

pub fn do_search(
    mut oap: Option<&mut Oparg>,
    mut dirc: i32,
    mut search_delim: i32,
    pat_opt: Option<&CStr>,
    mut patlen: usize,
    count: i32,
    options: i32,
    magic: bool,
    mut sia: Option<&mut SearchitArg>,
) -> i32 {
    let mut pat_buf: Vec<u8> = match pat_opt {
        Some(p) => {
            let mut v = p.to_bytes()[..patlen.min(p.to_bytes().len())].to_vec();
            v.push(0);
            v
        }
        None => vec![0],
    };
    let mut pat_off: usize = 0;
    let mut had_copy = false;
    let mut searchstr_cstr: CString;
    let mut searchstrlen: usize;
    let mut retval: i32;
    let mut c: i64;
    let mut msgbuf_vec: Vec<u8> = Vec::new();
    let mut msgbuflen: usize = 0;
    let mut has_offset = false;

    g_st().search_global.cmdlen = 0;

    if with_srch(|s| s.spats[0].off.line)
        && crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_LINEOFF)
    {
        with_srch_mut(|s| {
            s.spats[0].off.line = false;
            s.spats[0].off.off = 0;
        });
    }

    let old_off = with_srch(|s| s.spats[0].off);

    let mut pos = curwin_mut().w_cursor;

    if dirc == 0 {
        dirc = i32::from(with_srch(|s| s.spats[0].off.dir));
    } else {
        with_srch_mut(|s| s.spats[0].off.dir = dirc as u8);
        set_vv_searchforward();
    }
    if (options & SEARCH_REV) != 0 {
        dirc = if dirc == b'/' as i32 {
            b'?' as i32
        } else {
            b'/' as i32
        };
    }

    if dirc == b'/' as i32 {
        if has_folding(curwin_mut(), pos.lnum, None, Some(&mut pos.lnum)) {
            pos.col = MAXCOL - 2;
        }
    } else if has_folding(curwin_mut(), pos.lnum, Some(&mut pos.lnum), None) {
        pos.col = 0;
    }

    if g_st().search_global.no_hlsearch && (options & SEARCH_KEEP) == 0 {
        redraw_all_later(UPD_SOME_VALID);
        set_no_hlsearch(false);
    }

    let mut success = true;
    loop {
        let mut show_top_bot_msg = false;

        let cur_pat_first = pat_buf.get(pat_off).copied().unwrap_or(0);
        searchstr_cstr = CString::new(crate::memory::bytes_until_nul(
            pat_buf.get(pat_off..).unwrap_or(&[]),
        ))
        .unwrap_or_default();
        searchstrlen = patlen;

        if cur_pat_first == 0 || i32::from(cur_pat_first) == search_delim {
            let (has_search, has_subst, subst_pat, subst_len) = with_srch(|s| {
                (
                    s.spats[RE_SEARCH as usize].pat.is_some(),
                    s.spats[RE_SUBST as usize].pat.is_some(),
                    s.spats[RE_SUBST as usize].pat.clone(),
                    s.spats[RE_SUBST as usize].patlen(),
                )
            });
            if !has_search {
                if !has_subst {
                    emsg(tr(E_NOPREVRE));
                    retval = 0;
                    success = false;
                    break;
                }
                searchstr_cstr = subst_pat.unwrap_or_default();
                searchstrlen = subst_len;
            } else {
                searchstr_cstr = CString::default();
                searchstrlen = 0;
            }
        }

        if cur_pat_first != 0 {
            let prev_searchstr = searchstr_cstr.clone();
            let prev_searchstrlen = searchstrlen;
            let (cmdlen_add, parsed_searchstr) = with_srch_mut(|s| {
                parse_search_pattern_offset(
                    &mut pat_buf,
                    &mut pat_off,
                    &mut patlen,
                    search_delim,
                    options,
                    &mut had_copy,
                    &mut s.spats[0].off,
                )
            });
            g_st().search_global.cmdlen += cmdlen_add;
            if parsed_searchstr.as_bytes().is_empty() {
                searchstr_cstr = prev_searchstr;
                searchstrlen = prev_searchstrlen;
            } else {
                searchstrlen = parsed_searchstr.as_bytes().len();
                searchstr_cstr = parsed_searchstr;
            }
        }

        let mut show_search_stats = false;
        if (options & SEARCH_ECHO) != 0
            && messaging()
            && g_st().msg_silent == 0
            && (!g_st().cmd_silent || !shortmess(K_SHM_SEARCHCOUNT))
        {
            let mut off_buf = [0u8; 40];
            let mut off_len: usize = 0;

            msg_start();
            msg_ext_set_kind(c"search_cmd");

            let off0 = with_srch(|s| s.spats[0].off);
            if !g_st().cmd_silent && (off0.line || off0.end || off0.off != 0) {
                off_buf[off_len] = dirc as u8;
                off_len += 1;
                if off0.end {
                    off_buf[off_len] = b'e';
                    off_len += 1;
                } else if !off0.line {
                    off_buf[off_len] = b's';
                    off_len += 1;
                }
                off_buf[off_len] = 0;
                if off0.off != 0 || off0.line {
                    let n = format_to_buf!(&mut off_buf[off_len..], c"%+lld", off0.off);
                    if n > 0 {
                        off_len += n as usize;
                    }
                }
            }

            let (p_bytes, plen) = if searchstr_cstr.as_bytes().is_empty() {
                with_srch(|s| {
                    let len = s.spats[0].patlen();
                    let bytes = s.spats[0].pat.as_ref().map_or(Vec::new(), |p| {
                        p.as_bytes()[..len.min(p.as_bytes().len())].to_vec()
                    });
                    (bytes, len)
                })
            } else {
                let bytes = searchstr_cstr.as_bytes();
                (
                    bytes[..searchstrlen.min(bytes.len())].to_vec(),
                    searchstrlen,
                )
            };

            let mut msgbufsize = if !shortmess(K_SHM_SEARCHCOUNT) || g_st().cmd_silent {
                let mut sz = if ui_has(K_UIMESSAGES) {
                    0
                } else if g_st().msg_scrolled != 0 && !g_st().cmd_silent {
                    ((g_st().rows_global - g_st().msg_row) * g_st().columns_global - 1) as usize
                } else {
                    ((g_st().rows_global - g_st().msg_row - 1) * g_st().columns_global
                        + g_st().sc_col
                        - 1) as usize
                };
                if sz < plen + off_len + SEARCH_STAT_BUF_LEN + 3 {
                    sz = plen + off_len + SEARCH_STAT_BUF_LEN + 3;
                }
                sz
            } else {
                plen + off_len + 3
            };
            let _ = &mut msgbufsize;

            msgbuf_vec = vec![b' '; msgbufsize];
            msgbuflen = msgbufsize - 1;
            msgbuf_vec[msgbuflen] = 0;

            if !g_st().cmd_silent {
                ui_busy_start();
                msgbuf_vec[0] = dirc as u8;
                if utf_iscomposing_first(utf_ptr2char(p_bytes.as_slice())) {
                    msgbuf_vec[1] = b' ';
                    msgbuf_vec[2..2 + plen].copy_from_slice(&p_bytes);
                } else {
                    msgbuf_vec[1..1 + plen].copy_from_slice(&p_bytes);
                }
                if off_len > 0 {
                    msgbuf_vec[plen + 1..plen + 1 + off_len].copy_from_slice(&off_buf[..off_len]);
                }

                if let Some(trunc) = msg_strtrunc(crate::memory::bytes_as_cstr(&msgbuf_vec), 1) {
                    msgbuflen = trunc.as_bytes().len();
                    msgbuf_vec = trunc.into_bytes_with_nul();
                }

                if curwin_mut().w_onebuf_opt.wo_rl
                    && curwin_mut().w_onebuf_opt.wo_rlc.first_byte() == b's'
                {
                    let rev = reverse_text(crate::memory::bytes_until_nul(&msgbuf_vec));
                    msgbuflen = rev.as_bytes().len();
                    msgbuf_vec = rev.into_bytes_with_nul();
                    let r_off = msgbuf_vec[..msgbuflen]
                        .iter()
                        .position(|&b| b != b' ')
                        .unwrap_or(msgbuflen);
                    let pat_len = msgbuflen - r_off;
                    msgbuf_vec.copy_within(r_off..msgbuflen, 0);
                    msgbuf_vec[pat_len..msgbuflen].fill(b' ');
                }
                msg_outtrans(crate::memory::bytes_as_cstr(&msgbuf_vec), 0, false);
                msg_clr_eos();
                msg_check();

                gotocmdline(false);
                ui_flush();
                ui_busy_stop();
                g_st().msg_nowait = true;
            }

            if !shortmess(K_SHM_SEARCHCOUNT) {
                show_search_stats = true;
            }
        }

        let off0 = with_srch(|s| s.spats[0].off);
        if !off0.line && off0.off != 0 && pos.col < MAXCOL - 2 {
            if off0.off > 0 {
                c = off0.off;
                while c != 0 {
                    if decl(&mut pos) == -1 {
                        break;
                    }
                    c -= 1;
                }
                if c != 0 {
                    pos.lnum = 0;
                    pos.col = MAXCOL;
                }
            } else {
                c = off0.off;
                while c != 0 {
                    if incl(&mut pos) == -1 {
                        break;
                    }
                    c += 1;
                }
                if c != 0 {
                    pos.lnum = curbuf_mut().b_ml.ml_line_count + 1;
                    pos.col = 0;
                }
            }
        }

        c = i64::from(searchit(
            Some(curwin_mut()),
            curbuf_mut(),
            &mut pos,
            None,
            if dirc == b'/' as i32 {
                FORWARD
            } else {
                BACKWARD
            },
            &searchstr_cstr,
            searchstrlen,
            count,
            i32::from(off0.end) * SEARCH_END
                + (options
                    & (SEARCH_KEEP
                        + SEARCH_PEEK
                        + SEARCH_HIS
                        + SEARCH_MSG
                        + SEARCH_START
                        + if pat_buf.get(pat_off) == Some(&b';') {
                            0
                        } else {
                            SEARCH_NOOF
                        })),
            RE_LAST,
            magic,
            (sia.is_some()).then(|| sia.as_deref_mut().unwrap()),
        ));

        if !shortmess(K_SHM_SEARCH) && sia.is_some() && sia.as_deref_mut().unwrap().sa_wrapped != 0
        {
            show_top_bot_msg = true;
        }

        if c == 0 {
            retval = 0;
            success = false;
            break;
        }
        let off0 = with_srch(|s| s.spats[0].off);
        if off0.end && oap.is_some() {
            oap.as_deref_mut().unwrap().inclusive = true;
        }
        retval = 1;

        if sia.is_some() && sia.as_deref_mut().unwrap().sa_wrapped != 0 {
            apply_autocmds(EVENT_SEARCHWRAPPED, None, None, false, None);
        }

        if (options & SEARCH_NOOF) == 0 || pat_buf.get(pat_off) == Some(&b';') {
            let org_pos = pos;
            let off0 = with_srch(|s| s.spats[0].off);

            if off0.line {
                c = i64::from(pos.lnum) + off0.off;
                if c < 1 {
                    pos.lnum = 1;
                } else if c > i64::from(curbuf_mut().b_ml.ml_line_count) {
                    pos.lnum = curbuf_mut().b_ml.ml_line_count;
                } else {
                    pos.lnum = c as Linenr;
                }
                pos.col = 0;

                retval = 2;
            } else if pos.col < MAXCOL - 2 {
                c = off0.off;
                if c > 0 {
                    while c > 0 {
                        c -= 1;
                        if incl(&mut pos) == -1 {
                            break;
                        }
                    }
                } else {
                    while c < 0 {
                        c += 1;
                        if decl(&mut pos) == -1 {
                            break;
                        }
                    }
                }
            }
            if !equalpos(pos, org_pos) {
                has_offset = true;
            }
        }

        if show_search_stats {
            let cursor_lnum = curwin_mut().w_cursor.lnum;
            let cursor_folded = (opt_vars().fdo_flags & K_OPT_FDO_FLAG_SEARCH) == 0
                && has_folding(curwin_mut(), cursor_lnum, None, None);
            cmdline_search_stat(
                dirc,
                &mut pos,
                curwin_mut().w_cursor,
                show_top_bot_msg,
                &mut msgbuf_vec,
                msgbuflen,
                count != 1 || has_offset || cursor_folded,
                opt_vars().p_msc as i32,
                SEARCH_STAT_DEF_TIMEOUT,
            );
        }

        if (options & SEARCH_OPT) == 0 || pat_buf.get(pat_off) != Some(&b';') {
            break;
        }

        pat_off += 1;
        dirc = i32::from(pat_buf.get(pat_off).copied().unwrap_or(0));
        patlen -= 1;
        search_delim = dirc;
        if dirc != b'?' as i32 && dirc != b'/' as i32 {
            retval = 0;
            emsg(tr(c"E386: Expected '?' or '/'  after ';'"));
            success = false;
            break;
        }
        pat_off += 1;
        patlen -= 1;
    }

    if success {
        if (options & SEARCH_MARK) != 0 {
            setpcmark();
        }
        curwin_mut().w_cursor = pos;
        curwin_mut().w_set_curswant = true;
    }

    if (options & SEARCH_KEEP) != 0 || (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) != 0 {
        with_srch_mut(|s| s.spats[0].off = old_off);
    }

    retval
}

pub fn search_for_exact_line(
    buf: &mut FileBuf,
    pos: &mut CursorPos,
    dir: Direction,
    pat: &CStr,
) -> bool {
    {
        let mut start: Linenr = 0;
        let compl_len = ins_compl_len();

        if buf.b_ml.ml_line_count == 0 {
            return false;
        }
        loop {
            pos.lnum += dir;
            if pos.lnum < 1 {
                if opt_vars().p_ws {
                    pos.lnum = buf.b_ml.ml_line_count;
                    if !shortmess(K_SHM_SEARCH) {
                        give_warning(tr(TOP_BOT_MSG), true, false);
                    }
                } else {
                    pos.lnum = 1;
                    break;
                }
            } else if pos.lnum > buf.b_ml.ml_line_count {
                if opt_vars().p_ws {
                    pos.lnum = 1;
                    if !shortmess(K_SHM_SEARCH) {
                        give_warning(tr(BOT_TOP_MSG), true, false);
                    }
                } else {
                    pos.lnum = 1;
                    break;
                }
            }
            if pos.lnum == start {
                break;
            }
            if start == 0 {
                start = pos.lnum;
            }
            let line_cstr = crate::memline::ml_get_buf(buf, pos.lnum);
            let line_slice = line_cstr.to_bytes();
            let p_slice = crate::charset::skipwhite_slice(line_slice);
            let skip_len = line_slice.len() - p_slice.len();
            pos.col = skip_len as Colnr;

            if compl_status_adding() && !compl_status_sol() {
                if mb_strcmp_ic(
                    opt_vars().p_ic,
                    crate::memory::CStrSubslice::advance(line_cstr, skip_len),
                    pat,
                ) == 0
                {
                    return true;
                }
            } else if !p_slice.is_empty() {
                debug_assert!(compl_len >= 0);
                let eq = if opt_vars().p_ic {
                    mb_strnicmp(
                        crate::memory::bytes_until_nul(p_slice),
                        pat.to_bytes(),
                        compl_len as usize,
                    ) == 0
                } else {
                    let pat_b = pat.to_bytes();
                    p_slice[..p_slice.len().min(compl_len as usize)]
                        == pat_b[..(compl_len as usize).min(pat_b.len())]
                };
                if eq {
                    return true;
                }
            }
        }
        false
    }
}

pub fn searchc(cap: &mut Cmdarg, mut t_cmd: bool) -> bool {
    let mut c = cap.nchar;
    let mut dir = cap.arg;
    let mut count = cap.count1;
    let mut stop = true;

    if c != i32::from(NUL) {
        if g_st().key_stuffed == 0 {
            set_csearch_direction(dir);
            set_csearch_until(i32::from(t_cmd));
            with_srch_mut(|s| {
                s.lastc[0] = c as u8;
                if cap.nchar_len != 0 {
                    let n = cap.nchar_len as usize;
                    s.lastc_bytelen = cap.nchar_len;
                    s.lastc_bytes[..n].copy_from_slice(&cap.nchar_composing[..n]);
                } else {
                    let mut ch_buf = [0u8; 6];
                    let n = utf_char2bytes(c, &mut ch_buf);
                    s.lastc_bytelen = n;
                    s.lastc_bytes[..n as usize].copy_from_slice(&ch_buf[..n as usize]);
                }
            });
        }
    } else {
        let Some((new_dir, new_t_cmd, new_c)) = with_srch(|s| {
            if s.lastc[0] == 0 && s.lastc_bytelen <= 1 {
                return None;
            }
            let new_dir = if dir != 0 { -s.lastcdir } else { s.lastcdir };
            Some((new_dir, s.last_t_cmd, i32::from(s.lastc[0])))
        }) else {
            return false;
        };
        dir = new_dir;
        t_cmd = new_t_cmd;
        c = new_c;

        if !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_SCOLON)
            && count == 1
            && t_cmd
        {
            stop = false;
        }
    }

    cap.oap_mut().inclusive = dir != BACKWARD;

    let p_bytes = get_cursor_line_slice();
    let mut col = curwin_mut().w_cursor.col;
    let len = get_cursor_line_len();
    let (lastc_bytelen, lastc_bytes) = with_srch(|s| (s.lastc_bytelen, s.lastc_bytes));

    while count > 0 {
        count -= 1;
        loop {
            if dir > 0 {
                col += crate::mbyte::utfc_slice2len(p_bytes.get(col as usize..).unwrap_or(&[]))
                    as Colnr;
                if col >= len {
                    return false;
                }
            } else {
                if col == 0 {
                    return false;
                }
                col -= crate::mbyte::utf_head_off_slice(p_bytes, (col - 1) as usize) + 1;
            }
            let rem = p_bytes.get(col as usize..).unwrap_or(&[]);
            if lastc_bytelen <= 1 {
                if i32::from(rem.first().copied().unwrap_or(0)) == c && stop {
                    break;
                }
            } else if rem.starts_with(&lastc_bytes[..lastc_bytelen as usize]) && stop {
                break;
            }
            stop = true;
        }
    }

    if t_cmd {
        col -= dir;
        if dir < 0 {
            col += lastc_bytelen - 1;
        } else {
            col -= crate::mbyte::utf_head_off_slice(p_bytes, col as usize);
        }
    }
    curwin_mut().w_cursor.col = col;

    true
}

pub fn findmatch(oap: Option<&mut Oparg>, initc: i32) -> Option<CursorPos> {
    findmatchlimit(oap, initc, 0, 0)
}

fn check_prevcol(line: &[u8], mut col: i32, ch: i32, prevcol: Option<&mut i32>) -> bool {
    col -= 1;
    if col > 0 {
        col -= crate::mbyte::utf_head_off_slice(line, col as usize);
    }
    if let Some(prevcol) = prevcol {
        *prevcol = col;
    }
    col >= 0 && line.get(col as usize).copied().map(i32::from) == Some(ch)
}

fn find_rawstring_end(line: &[u8], startpos: &CursorPos, endpos: &CursorPos) -> bool {
    let start_off = ((startpos.col + 1) as usize).min(line.len());
    let delim_len = line[start_off..]
        .iter()
        .position(|&b| b == b'(')
        .unwrap_or(line.len() - start_off);
    let delim_copy = line[start_off..start_off + delim_len].to_vec();
    for lnum in startpos.lnum..=endpos.lnum {
        let cur_line = crate::memline::ml_get_slice(lnum);
        let mut idx = if lnum == startpos.lnum {
            ((startpos.col + 1) as usize).min(cur_line.len())
        } else {
            0
        };
        let limit = if lnum == endpos.lnum {
            (endpos.col.max(0) as usize).min(cur_line.len())
        } else {
            cur_line.len()
        };
        while idx < limit {
            if cur_line[idx] == b')'
                && cur_line.get(idx + 1..idx + 1 + delim_len) == Some(delim_copy.as_slice())
                && cur_line.get(idx + 1 + delim_len) == Some(&b'"')
            {
                return true;
            }
            idx += 1;
        }
    }
    false
}

fn find_mps_values(initc: &mut i32, findc: &mut i32, backwards: &mut bool, switchit: bool) {
    let mut rem = curbuf_mut().b_p_mps.as_bytes();

    while !rem.is_empty() {
        let first_c = crate::mbyte::utf_slice2char(rem);
        let first_len = crate::mbyte::utfc_slice2len(rem).min(rem.len());
        let after_colon = &rem[(first_len + 1).min(rem.len())..];
        let second_c = crate::mbyte::utf_slice2char(after_colon);
        if first_c == *initc {
            if switchit {
                *findc = *initc;
                *initc = second_c;
                *backwards = true;
            } else {
                *findc = second_c;
                *backwards = false;
            }
            return;
        }
        if second_c == *initc {
            if switchit {
                *findc = *initc;
                *initc = first_c;
                *backwards = false;
            } else {
                *findc = first_c;
                *backwards = true;
            }
            return;
        }
        let second_len = crate::mbyte::utfc_slice2len(after_colon).min(after_colon.len());
        rem = &after_colon[second_len..];
        if let Some(after_comma) = rem.strip_prefix(b",") {
            rem = after_comma;
        }
    }
}

pub fn findmatchlimit(
    oap: Option<&mut Oparg>,
    mut initc: i32,
    flags: i32,
    maxtravel: i64,
) -> Option<CursorPos> {
    let mut findc: i32 = 0;
    let mut count: i32 = 0;
    let mut backwards = false;
    let mut raw_string = false;
    let mut inquote = false;
    let mut hash_dir: i32 = 0;
    let mut comment_dir: i32 = 0;
    let mut traveled: i64 = 0;
    let mut ignore_cend = false;
    let mut match_escaped: i32 = 0;
    let mut comment_col: i32 = MAXCOL;
    let mut lispcomm = false;
    let lisp = curbuf_mut().b_p_lisp;
    let skip_comments = (flags & FM_SKIPCOMM) != 0;
    let mut in_block_comment = false;

    let mut pos = CursorPos {
        coladd: 0,
        ..curwin_mut().w_cursor
    };
    let mut line = crate::memline::ml_get_slice(pos.lnum);

    let cpo_match = crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_MATCH);
    let cpo_bsl = crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_MATCHBSL);

    let dir = if (flags & FM_BACKWARD) != 0 {
        BACKWARD
    } else if (flags & FM_FORWARD) != 0 {
        FORWARD
    } else {
        0
    };

    if initc == b'/' as i32 || initc == b'*' as i32 || initc == b'R' as i32 {
        comment_dir = dir;
        if initc == b'/' as i32 {
            ignore_cend = true;
        }
        backwards = dir != FORWARD;
        raw_string = initc == b'R' as i32;
        initc = i32::from(NUL);
    } else if initc != b'#' as i32 && initc != i32::from(NUL) {
        find_mps_values(&mut initc, &mut findc, &mut backwards, true);
        if dir != 0 {
            backwards = dir != FORWARD;
        }
        if findc == i32::from(NUL) {
            return None;
        }
    } else {
        if initc == b'#' as i32 {
            hash_dir = dir;
        } else {
            if !cpo_match {
                let trim = crate::charset::skipwhite_slice(line);
                let col_u = pos.col as usize;
                if trim.first() == Some(&b'#') && col_u <= line.len() - trim.len() {
                    let after_hash = crate::charset::skipwhite_slice(&trim[1..]);
                    if after_hash.starts_with(b"if")
                        || after_hash.starts_with(b"endif")
                        || after_hash.starts_with(b"el")
                    {
                        hash_dir = 1;
                    }
                } else if line.get(col_u) == Some(&b'/') {
                    if line.get(col_u + 1) == Some(&b'*') {
                        comment_dir = FORWARD;
                        backwards = false;
                        pos.col += 1;
                    } else if col_u > 0 && line.get(col_u - 1) == Some(&b'*') {
                        comment_dir = BACKWARD;
                        backwards = true;
                        pos.col -= 1;
                    }
                } else if line.get(col_u) == Some(&b'*') {
                    if line.get(col_u + 1) == Some(&b'/') {
                        comment_dir = BACKWARD;
                        backwards = true;
                    } else if col_u > 0 && line.get(col_u - 1) == Some(&b'/') {
                        comment_dir = FORWARD;
                        backwards = false;
                    }
                }
            }

            if hash_dir == 0 && comment_dir == 0 {
                if (pos.col as usize) >= line.len() && pos.col != 0 {
                    pos.col -= 1;
                }
                loop {
                    let col_u = (pos.col as usize).min(line.len());
                    initc = crate::mbyte::utf_slice2char(&line[col_u..]);
                    if initc == i32::from(NUL) {
                        break;
                    }

                    find_mps_values(&mut initc, &mut findc, &mut backwards, false);
                    if findc != 0 {
                        break;
                    }
                    pos.col += crate::mbyte::utfc_slice2len(&line[col_u..]) as Colnr;
                }
                if findc == 0 {
                    if !cpo_match && crate::charset::skipwhite_slice(line).first() == Some(&b'#') {
                        hash_dir = 1;
                    } else {
                        return None;
                    }
                } else if !cpo_bsl {
                    let mut bslcnt = 0;
                    let mut col = pos.col;
                    while check_prevcol(line, col, b'\\' as i32, Some(&mut col)) {
                        bslcnt += 1;
                    }
                    match_escaped = bslcnt & 1;
                }
            }
        }
        if hash_dir != 0 {
            if let Some(oap) = oap {
                oap.motion_type = MT_LINE_WISE;
            }
            if initc != b'#' as i32 {
                let trim = crate::charset::skipwhite_slice(line);
                let after_hash = crate::charset::skipwhite_slice(&trim[1.min(trim.len())..]);
                if after_hash.starts_with(b"if") || after_hash.starts_with(b"el") {
                    hash_dir = 1;
                } else if after_hash.starts_with(b"endif") {
                    hash_dir = -1;
                } else {
                    return None;
                }
            }
            pos.col = 0;
            while !g_st().got_int {
                if hash_dir > 0 {
                    if pos.lnum == curbuf_mut().b_ml.ml_line_count {
                        break;
                    }
                } else if pos.lnum == 1 {
                    break;
                }
                pos.lnum += hash_dir;
                line = crate::memline::ml_get_slice(pos.lnum);
                line_breakcheck();
                let trim = crate::charset::skipwhite_slice(line);
                if trim.first() != Some(&b'#') {
                    continue;
                }
                pos.col = (line.len() - trim.len()) as Colnr;
                let after_hash = crate::charset::skipwhite_slice(&trim[1..]);
                if hash_dir > 0 {
                    if after_hash.starts_with(b"if") {
                        count += 1;
                    } else if after_hash.starts_with(b"el") {
                        if count == 0 {
                            return Some(pos);
                        }
                    } else if after_hash.starts_with(b"endif") {
                        if count == 0 {
                            return Some(pos);
                        }
                        count -= 1;
                    }
                } else if after_hash.starts_with(b"if") {
                    if count == 0 {
                        return Some(pos);
                    }
                    count -= 1;
                } else if initc == b'#' as i32 && after_hash.starts_with(b"el") {
                    if count == 0 {
                        return Some(pos);
                    }
                } else if after_hash.starts_with(b"endif") {
                    count += 1;
                }
            }
            return None;
        }
    }

    if curwin_mut().w_onebuf_opt.wo_rl && crate::strings::vim_haschr(c"()[]{}<>", initc) {
        backwards = !backwards;
    }

    let mut do_quotes: i32 = -1;
    let mut at_start: i32;
    let mut start_in_quotes: TriState = K_NONE;
    let mut match_pos = CursorPos {
        lnum: 0,
        col: 0,
        coladd: 0,
    };

    if (backwards && comment_dir != 0) || lisp || skip_comments {
        line = crate::memline::ml_get_slice(pos.lnum);
        comment_col = check_linecomment(line);
    }
    if lisp && comment_col != MAXCOL && pos.col > comment_col {
        lispcomm = true;
    }
    if skip_comments
        && !in_block_comment
        && comment_col != MAXCOL
        && backwards
        && pos.col > comment_col
    {
        pos.col = comment_col;
    }

    while !g_st().got_int {
        if backwards {
            if lispcomm && pos.col < comment_col {
                break;
            }
            if pos.col == 0 {
                if pos.lnum == 1 {
                    break;
                }
                pos.lnum -= 1;

                traveled += 1;
                if maxtravel > 0 && traveled > maxtravel {
                    break;
                }

                line = crate::memline::ml_get_slice(pos.lnum);
                pos.col = line.len() as Colnr;
                do_quotes = -1;
                line_breakcheck();

                if comment_dir != 0 || lisp || skip_comments {
                    comment_col = check_linecomment(line);
                }
                if (lisp || (skip_comments && !in_block_comment && pos.col > comment_col))
                    && comment_col != MAXCOL
                {
                    pos.col = comment_col;
                }
            } else {
                pos.col -= 1;
                pos.col -= crate::mbyte::utf_head_off_slice(line, pos.col as usize);
            }
        } else if (pos.col as usize) >= line.len()
            || (lisp && comment_col != MAXCOL && pos.col == comment_col)
        {
            if pos.lnum == curbuf_mut().b_ml.ml_line_count || lispcomm {
                break;
            }
            pos.lnum += 1;

            let prev_traveled = traveled;
            traveled += 1;
            if maxtravel != 0 && prev_traveled > maxtravel {
                break;
            }

            line = crate::memline::ml_get_slice(pos.lnum);
            pos.col = 0;
            do_quotes = -1;
            line_breakcheck();
            if lisp || skip_comments {
                comment_col = check_linecomment(line);
            }
        } else {
            let col_u = (pos.col as usize).min(line.len());
            pos.col += crate::mbyte::utfc_slice2len(&line[col_u..]) as Colnr;
        }

        let col_u = pos.col as usize;
        if skip_comments && comment_dir == 0 && !inquote {
            if backwards {
                if !in_block_comment
                    && col_u > 0
                    && line.get(col_u - 1) == Some(&b'*')
                    && line.get(col_u) == Some(&b'/')
                    && (comment_col == MAXCOL || pos.col < comment_col)
                {
                    in_block_comment = true;
                } else if in_block_comment
                    && col_u > 0
                    && line.get(col_u - 1) == Some(&b'/')
                    && line.get(col_u) == Some(&b'*')
                {
                    in_block_comment = false;
                }
            } else if !in_block_comment
                && line.get(col_u) == Some(&b'/')
                && line.get(col_u + 1) == Some(&b'*')
                && (comment_col == MAXCOL || pos.col < comment_col)
            {
                in_block_comment = true;
            } else if in_block_comment
                && col_u > 0
                && line.get(col_u - 1) == Some(&b'*')
                && line.get(col_u) == Some(&b'/')
            {
                in_block_comment = false;
            }
        }

        if pos.col == 0
            && (flags & FM_BLOCKSTOP) != 0
            && matches!(line.first(), Some(&b'{' | &b'}'))
        {
            if i32::from(line[0]) == findc && count == 0 {
                return Some(pos);
            }
            break;
        }

        if comment_dir != 0 {
            if comment_dir == FORWARD {
                if line.get(col_u) == Some(&b'*') && line.get(col_u + 1) == Some(&b'/') {
                    pos.col += 1;
                    return Some(pos);
                }
            } else if pos.col == 0 {
                continue;
            } else if raw_string {
                if line.get(col_u - 1) == Some(&b'R')
                    && line.get(col_u) == Some(&b'"')
                    && line.get(col_u + 1..).is_some_and(|s| s.contains(&b'('))
                {
                    if !find_rawstring_end(
                        line,
                        &pos,
                        if count > 0 {
                            &match_pos
                        } else {
                            &curwin_mut().w_cursor
                        },
                    ) {
                        count += 1;
                        match_pos = pos;
                        match_pos.col -= 1;
                    }
                    line = crate::memline::ml_get_slice(pos.lnum);
                }
            } else if line.get(col_u - 1) == Some(&b'/')
                && line.get(col_u) == Some(&b'*')
                && (col_u == 1 || line.get(col_u - 2) != Some(&b'*'))
                && pos.col < comment_col
            {
                count += 1;
                match_pos = pos;
                match_pos.col -= 1;
            } else if line.get(col_u - 1) == Some(&b'*') && line.get(col_u) == Some(&b'/') {
                if count > 0 {
                    pos = match_pos;
                } else if col_u > 1 && line.get(col_u - 2) == Some(&b'/') && pos.col <= comment_col
                {
                    pos.col -= 2;
                } else if ignore_cend {
                    continue;
                } else {
                    return None;
                }
                return Some(pos);
            }
            continue;
        }

        if cpo_match {
            do_quotes = 0;
        } else if do_quotes == -1 {
            at_start = do_quotes;
            let target_idx = (pos.col + i32::from(backwards)) as usize;
            let mut idx = 0usize;
            while idx < line.len() {
                if idx == target_idx {
                    at_start = do_quotes & 1;
                }
                if line[idx] == b'"'
                    && (idx == 0 || line[idx - 1] != b'\'' || line.get(idx + 1) != Some(&b'\''))
                {
                    do_quotes += 1;
                }
                if line[idx] == b'\\' && idx + 1 < line.len() {
                    idx += 1;
                }
                idx += 1;
            }
            do_quotes &= 1;

            if do_quotes == 0 {
                inquote = false;
                if line.last() == Some(&b'\\') {
                    do_quotes = 1;
                    if start_in_quotes == K_NONE {
                        inquote = at_start != 0;
                        if inquote {
                            start_in_quotes = K_TRUE;
                        }
                    } else if backwards {
                        inquote = true;
                    }
                }
                if pos.lnum > 1 {
                    let prev_line = crate::memline::ml_get_slice(pos.lnum - 1);
                    if prev_line.last() == Some(&b'\\') {
                        do_quotes = 1;
                        if start_in_quotes == K_NONE {
                            inquote = at_start != 0;
                            if inquote {
                                start_in_quotes = K_TRUE;
                            }
                        } else if !backwards {
                            inquote = true;
                        }
                    }
                    line = crate::memline::ml_get_slice(pos.lnum);
                }
            }
        }
        if start_in_quotes == K_NONE {
            start_in_quotes = K_FALSE;
        }

        let col_u = (pos.col as usize).min(line.len());
        let c = crate::mbyte::utf_slice2char(&line[col_u..]);
        if c == i32::from(NUL) {
            if col_u == 0 || line.get(col_u - 1) != Some(&b'\\') {
                inquote = false;
                start_in_quotes = K_FALSE;
            }
        } else if c == b'"' as i32 {
            if do_quotes != 0 {
                let mut col = pos.col - 1;
                while col >= 0 {
                    if line.get(col as usize) != Some(&b'\\') {
                        break;
                    }
                    col -= 1;
                }
                if ((pos.col - 1 - col) & 1) == 0 {
                    inquote = !inquote;
                    start_in_quotes = K_FALSE;
                }
            }
        } else {
            let mut skipped_single_quote = false;
            if c == b'\'' as i32 && !cpo_match && initc != b'\'' as i32 && findc != b'\'' as i32 {
                if backwards {
                    if col_u > 1 {
                        if line.get(col_u - 2) == Some(&b'\'') {
                            pos.col -= 2;
                            skipped_single_quote = true;
                        } else if line.get(col_u - 2) == Some(&b'\\')
                            && col_u > 2
                            && line.get(col_u - 3) == Some(&b'\'')
                        {
                            pos.col -= 3;
                            skipped_single_quote = true;
                        }
                    }
                } else if col_u + 1 < line.len() {
                    if line.get(col_u + 1) == Some(&b'\\')
                        && col_u + 2 < line.len()
                        && line.get(col_u + 3) == Some(&b'\'')
                    {
                        pos.col += 3;
                        skipped_single_quote = true;
                    } else if line.get(col_u + 2) == Some(&b'\'') {
                        pos.col += 2;
                        skipped_single_quote = true;
                    }
                }
            }

            if !skipped_single_quote {
                if curbuf_mut().b_p_lisp
                    && matches!(c as u8, b'(' | b')' | b'{' | b'}' | b'[' | b']')
                    && pos.col > 1
                    && check_prevcol(line, pos.col, b'\\' as i32, None)
                    && check_prevcol(line, pos.col - 1, b'#' as i32, None)
                {
                    continue;
                }

                if skip_comments
                    && (in_block_comment || (comment_col != MAXCOL && pos.col >= comment_col))
                {
                    continue;
                }

                if (!inquote || start_in_quotes == K_TRUE) && (c == initc || c == findc) {
                    let mut bslcnt = 0;

                    if !cpo_bsl {
                        let mut col = pos.col;
                        while check_prevcol(line, col, b'\\' as i32, Some(&mut col)) {
                            bslcnt += 1;
                        }
                    }
                    if cpo_bsl || (bslcnt & 1) == match_escaped {
                        if c == initc {
                            count += 1;
                        } else {
                            if count == 0 {
                                return Some(pos);
                            }
                            count -= 1;
                        }
                    }
                }
            }
        }
    }

    if comment_dir == BACKWARD && count > 0 {
        return Some(match_pos);
    }
    None
}

pub fn showmatch(c: i32) {
    let mut vcol: Colnr = 0;
    let use_win_so = curwin_mut().w_onebuf_opt.wo_so >= 0;
    let use_win_siso = curwin_mut().w_onebuf_opt.wo_siso >= 0;

    let mut p = curbuf_mut().b_p_mps.as_bytes();
    while !p.is_empty() {
        if utf_ptr2char(p) == c && ((curwin_mut().w_onebuf_opt.wo_rl) ^ (opt_vars().p_ri)) {
            break;
        }
        p = p.get(utfc_ptr2len(p) as usize + 1..).unwrap_or(&[]);
        if utf_ptr2char(p) == c && !((curwin_mut().w_onebuf_opt.wo_rl) ^ (opt_vars().p_ri)) {
            break;
        }
        p = p.get(utfc_ptr2len(p) as usize..).unwrap_or(&[]);
        if p.is_empty() {
            return;
        }
        p = &p[1..];
    }
    if p.is_empty() {
        return;
    }

    let Some(mut lpos) = findmatch(None, i32::from(NUL)) else {
        vim_beep(K_OPT_BO_FLAG_SHOWMATCH);
        return;
    };

    if lpos.lnum < curwin_mut().w_topline || lpos.lnum >= curwin_mut().w_botline {
        return;
    }

    if !curwin_mut().w_onebuf_opt.wo_wrap {
        getvcol(curwin_mut(), &mut lpos, None, Some(&mut vcol), None, 0);
    }

    let col_visible = curwin_mut().w_onebuf_opt.wo_wrap
        || (vcol >= curwin_mut().w_leftcol
            && vcol < curwin_mut().w_leftcol + curwin_mut().w_view_width);
    if !col_visible {
        return;
    }

    let mpos = lpos;
    let save_cursor = curwin_mut().w_cursor;
    let save_so = if use_win_so {
        curwin_mut().w_onebuf_opt.wo_so
    } else {
        opt_vars().p_so
    };
    let save_siso = if use_win_siso {
        curwin_mut().w_onebuf_opt.wo_siso
    } else {
        opt_vars().p_siso
    };
    if g_st().dollar_vcol >= 0 && g_st().dollar_vcol == curwin_mut().w_virtcol {
        g_st().dollar_vcol = -1;
    }
    curwin_mut().w_virtcol += 1;

    let save_dollar_vcol = g_st().dollar_vcol;
    let save_state = g_st().state_global;
    g_st().state_global = MODE_SHOWMATCH;
    ui_cursor_shape();
    curwin_mut().w_cursor = mpos;
    if use_win_so {
        curwin_mut().w_onebuf_opt.wo_so = 0;
    } else {
        opt_vars().p_so = 0;
    }
    if use_win_siso {
        curwin_mut().w_onebuf_opt.wo_siso = 0;
    } else {
        opt_vars().p_siso = 0;
    }
    show_cursor_info_later(false);
    update_screen();
    setcursor();
    ui_flush();
    g_st().dollar_vcol = save_dollar_vcol;

    if !vim_strchr(opt_vars().p_cpo.as_cstr(), K_CPO_SHOWMATCH)
        .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
        .is_null()
    {
        os_delay((opt_vars().p_mat as u64) * 100 + 8, true);
    } else if !char_avail() {
        os_delay((opt_vars().p_mat as u64) * 100 + 9, false);
    }
    curwin_mut().w_cursor = save_cursor;
    if use_win_so {
        curwin_mut().w_onebuf_opt.wo_so = save_so;
    } else {
        opt_vars().p_so = save_so;
    }
    if use_win_siso {
        curwin_mut().w_onebuf_opt.wo_siso = save_siso;
    } else {
        opt_vars().p_siso = save_siso;
    }
    g_st().state_global = save_state;
    ui_cursor_shape();
}

pub fn current_search(count: i32, forward: bool) -> bool {
    let old_p_ws = opt_vars().p_ws;
    let save_visual = g_st().visual_global.start;

    if g_st().visual_global.active
        && opt_vars().p_sel.first_byte() == b'e'
        && lt(g_st().visual_global.start, curwin_mut().w_cursor)
    {
        dec_cursor();
    }

    let skip_first_backward = forward
        && g_st().visual_global.active
        && lt(curwin_mut().w_cursor, g_st().visual_global.start);

    let mut pos = curwin_mut().w_cursor;
    let orig_pos = curwin_mut().w_cursor;
    if g_st().visual_global.active {
        if forward {
            incl(&mut pos);
        } else {
            decl(&mut pos);
        }
    }

    let (last_pat, last_patlen) = with_srch(|s| {
        let idx = s.last_idx as usize;
        (s.spats[idx].pat.clone(), s.spats[idx].patlen())
    });
    let zero_width = is_zero_width(
        last_pat.as_deref(),
        last_patlen,
        true,
        curwin_mut().w_cursor,
        FORWARD,
    );
    if zero_width == -1 {
        return false;
    }

    let mut end_pos = pos;
    let mut result: i32;

    for i in 0..2 {
        let dir = if forward {
            if i == 0 && skip_first_backward {
                continue;
            }
            i != 0
        } else {
            i == 0
        };

        let mut flags = 0;
        if !dir && zero_width == 0 {
            flags = SEARCH_END;
        }
        end_pos = pos;

        if i == 0 {
            opt_vars().p_ws = false;
        }

        result = searchit(
            Some(curwin_mut()),
            curbuf_mut(),
            &mut pos,
            Some(&mut end_pos),
            if dir { FORWARD } else { BACKWARD },
            last_pat.as_deref().unwrap_or(c""),
            last_patlen,
            if i != 0 { count } else { 1 },
            SEARCH_KEEP | flags,
            RE_SEARCH,
            opt_vars().p_magic,
            None,
        );

        opt_vars().p_ws = old_p_ws;

        if i == 1 && result == 0 {
            curwin_mut().w_cursor = orig_pos;
            if g_st().visual_global.active {
                g_st().visual_global.start = save_visual;
            }
            return false;
        } else if i == 0 && result == 0 {
            if forward {
                clearpos(&mut pos);
            } else {
                pos.lnum = curwin_mut().buf_mut().b_ml.ml_line_count;
                pos.col = ml_get_len(curwin_mut().buf_mut().b_ml.ml_line_count);
            }
        }
    }

    let start_pos = pos;

    if !g_st().visual_global.active {
        g_st().visual_global.start = start_pos;
    }

    curwin_mut().w_cursor = end_pos;
    if lt(g_st().visual_global.start, end_pos) && forward {
        if skip_first_backward {
            curwin_mut().w_cursor = pos;
        } else {
            dec_cursor();
        }
    } else if g_st().visual_global.active
        && lt(curwin_mut().w_cursor, g_st().visual_global.start)
        && forward
    {
        curwin_mut().w_cursor = pos;
    }
    g_st().visual_global.active = true;
    g_st().visual_global.mode = b'v' as i32;

    if opt_vars().p_sel.first_byte() == b'e' {
        if forward && ltoreq(g_st().visual_global.start, curwin_mut().w_cursor) {
            inc_cursor();
        } else if !forward && ltoreq(curwin_mut().w_cursor, g_st().visual_global.start) {
            inc(&mut g_st().visual_global.start);
        }
    }

    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_SEARCH) != 0 && g_st().key_typed {
        fold_open_cursor();
    }

    may_start_select(b'c' as i32);
    setmouse();
    redraw_curbuf_later(UPD_INVERTED);
    showmode();

    true
}

fn is_zero_width(
    pattern_opt: Option<&CStr>,
    mut patternlen: usize,
    move_to_start: bool,
    cur: CursorPos,
    direction: Direction,
) -> i32 {
    {
        let mut regmatch: Regmmatch = Regmmatch::new();
        let mut result: i32 = -1;
        let mut pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let called_emsg_before = g_st().called_emsg;
        let mut flag = 0;

        let pattern_cstr = match pattern_opt {
            Some(p) => p.to_owned(),
            None => with_srch(|s| {
                let idx = s.last_idx as usize;
                patternlen = s.spats[idx].patlen();
                s.spats[idx].pat.clone().unwrap_or_default()
            }),
        };

        if !search_regcomp(
            &pattern_cstr,
            patternlen,
            None,
            RE_SEARCH,
            RE_SEARCH,
            SEARCH_KEEP,
            opt_vars().p_magic,
            &mut regmatch,
        ) {
            return -1;
        }

        regmatch.startpos[0].col = -1;
        if move_to_start {
            clearpos(&mut pos);
        } else {
            pos = cur;
            flag = SEARCH_START;
        }
        if searchit(
            Some(curwin_mut()),
            curbuf_mut(),
            &mut pos,
            None,
            direction,
            &pattern_cstr,
            patternlen,
            1,
            SEARCH_KEEP + flag,
            RE_SEARCH,
            opt_vars().p_magic,
            None,
        ) != 0
        {
            let mut nmatched: i32;
            loop {
                regmatch.startpos[0].col += 1;
                let start_col = regmatch.startpos[0].col;
                nmatched = vim_regexec_multi(
                    &mut regmatch,
                    Some(curwin_mut()),
                    curbuf_mut(),
                    pos.lnum,
                    start_col,
                    None,
                    None,
                );
                if nmatched != 0 {
                    break;
                }
                if regmatch.regprog.is_null()
                    || !(if direction == FORWARD {
                        regmatch.startpos[0].col < pos.col
                    } else {
                        regmatch.startpos[0].col > pos.col
                    })
                {
                    break;
                }
            }

            if g_st().called_emsg == called_emsg_before {
                result = i32::from(
                    nmatched != 0
                        && regmatch.startpos[0].lnum == regmatch.endpos[0].lnum
                        && regmatch.startpos[0].col == regmatch.endpos[0].col,
                );
            }
        }

        regmatch.free_prog();
        result
    }
}

pub fn linewhite(lnum: Linenr) -> bool {
    crate::charset::skipwhite_slice(crate::memline::ml_get_slice(lnum)).is_empty()
}

fn cmdline_search_stat(
    dirc: i32,
    pos: &mut CursorPos,
    cursor_pos: CursorPos,
    show_top_bot_msg: bool,
    msgbuf: &mut [u8],
    msgbuflen: usize,
    recompute: bool,
    maxcount: i32,
    timeout: i32,
) {
    let mut stat = Searchstat::default();

    update_search_stat(
        dirc, pos, cursor_pos, &mut stat, recompute, maxcount, timeout,
    );
    if stat.cur <= 0 {
        return;
    }

    let mut t = [0u8; SEARCH_STAT_BUF_LEN];
    let mut len: usize;

    if curwin_mut().w_onebuf_opt.wo_rl && curwin_mut().w_onebuf_opt.wo_rlc.first_byte() == b's' {
        if stat.incomplete == 1 {
            len = format_to_buf!(&mut t[..], c"[?/??]") as usize;
        } else if stat.cnt > maxcount && stat.cur > maxcount {
            len = format_to_buf!(&mut t[..], c"[>%d/>%d]", maxcount, maxcount,) as usize;
        } else if stat.cnt > maxcount {
            len = format_to_buf!(&mut t[..], c"[>%d/%d]", maxcount, stat.cur,) as usize;
        } else {
            len = format_to_buf!(&mut t[..], c"[%d/%d]", stat.cnt, stat.cur,) as usize;
        }
    } else if stat.incomplete == 1 {
        len = format_to_buf!(&mut t[..], c"[?/??]") as usize;
    } else if stat.cnt > maxcount && stat.cur > maxcount {
        len = format_to_buf!(&mut t[..], c"[>%d/>%d]", maxcount, maxcount,) as usize;
    } else if stat.cnt > maxcount {
        len = format_to_buf!(&mut t[..], c"[%d/>%d]", stat.cur, maxcount,) as usize;
    } else {
        len = format_to_buf!(&mut t[..], c"[%d/%d]", stat.cur, stat.cnt,) as usize;
    }

    if show_top_bot_msg && len + 2 < SEARCH_STAT_BUF_LEN {
        t.copy_within(0..len, 2);
        t[0] = b'W';
        t[1] = b' ';
        len += 2;
    }

    if len > msgbuflen {
        len = msgbuflen;
    }
    msgbuf[msgbuflen - len..msgbuflen].copy_from_slice(&t[..len]);

    g_st().msg_ext_overwrite = true;
    msg_ext_set_kind(c"search_count");
    give_warning(crate::memory::bytes_as_cstr(msgbuf), false, false);
}

fn update_search_stat(
    dirc: i32,
    pos: &mut CursorPos,
    cursor_pos: CursorPos,
    stat: &mut Searchstat,
    recompute: bool,
    maxcount: i32,
    timeout: i32,
) {
    let save_ws = opt_vars().p_ws;
    let p = *pos;

    *stat = Searchstat::default();

    if dirc == 0 && !recompute && !empty_pos(with_srch(|s| s.stat_lastpos)) {
        with_srch(|s| {
            stat.cur = s.stat_cur;
            stat.cnt = s.stat_cnt;
            stat.exact_match = s.stat_exact_match;
            stat.incomplete = s.stat_incomplete;
            stat.last_maxcount = s.stat_last_maxcount;
        });
        return;
    }
    with_srch_mut(|s| s.stat_last_maxcount = maxcount);
    let stat_lastpos = with_srch(|s| s.stat_lastpos);
    let wraparound = (dirc == b'?' as i32 && lt(stat_lastpos, p))
        || (dirc == b'/' as i32 && lt(p, stat_lastpos));

    let chgtick = buf_get_changedtick(curbuf_mut());
    let valid_cache = with_srch(|s| {
        i64::from(s.stat_chgtick) == chgtick
            && s.stat_lastpat.is_some()
            && s.stat_lastpat.as_deref() == s.spats[s.last_idx as usize].pat.as_deref()
            && equalpos(s.stat_lastpos, cursor_pos)
            && s.stat_lbuf == g_st().curbuf
            && !wraparound
            && s.stat_cur >= 0
            && (maxcount <= 0 || s.stat_cur <= maxcount)
            && !recompute
    });
    if !valid_cache {
        with_srch_mut(|s| {
            s.stat_cur = 0;
            s.stat_cnt = 0;
            s.stat_exact_match = false;
            s.stat_incomplete = 0;
            s.stat_lastpos = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            s.stat_lbuf = g_st().curbuf;
        });
    }

    if equalpos(with_srch(|s| s.stat_lastpos), cursor_pos)
        && !wraparound
        && with_srch(|s| {
            if dirc == 0 || dirc == b'/' as i32 {
                s.stat_cur < s.stat_cnt
            } else {
                s.stat_cur > 1
            }
        })
    {
        with_srch_mut(|s| {
            s.stat_cur += if dirc == 0 {
                0
            } else if dirc == b'/' as i32 {
                1
            } else {
                -1
            };
        });
    } else {
        let mut start: Proftime = 0;
        let mut done_search = false;
        let mut endpos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        opt_vars().p_ws = false;
        if timeout > 0 {
            start = profile_setlimit(i64::from(timeout));
        }
        let mut lastpos = with_srch(|s| s.stat_lastpos);
        while !g_st().got_int
            && searchit(
                Some(curwin_mut()),
                curbuf_mut(),
                &mut lastpos,
                Some(&mut endpos),
                FORWARD,
                c"",
                0,
                1,
                SEARCH_KEEP,
                RE_LAST,
                opt_vars().p_magic,
                None,
            ) != 0
        {
            with_srch_mut(|s| s.stat_lastpos = lastpos);
            done_search = true;
            if timeout > 0 && profile_passed_limit(start) {
                with_srch_mut(|s| s.stat_incomplete = 1);
                break;
            }
            let cnt = with_srch_mut(|s| {
                s.stat_cnt += 1;
                s.stat_cnt
            });
            if ltoreq(lastpos, p) {
                with_srch_mut(|s| {
                    s.stat_cur = cnt;
                    if lt(p, endpos) {
                        s.stat_exact_match = true;
                    }
                });
            }
            fast_breakcheck();
            if maxcount > 0 && cnt > maxcount {
                with_srch_mut(|s| s.stat_incomplete = 2);
                break;
            }
        }
        with_srch_mut(|s| s.stat_lastpos = lastpos);
        if g_st().got_int {
            with_srch_mut(|s| s.stat_cur = -1);
        }
        if done_search {
            let chgtick = buf_get_changedtick(curbuf_mut()) as i32;
            with_srch_mut(|s| {
                let last_idx = s.last_idx as usize;
                s.stat_lastpat = s.spats[last_idx].pat.clone();
                s.stat_chgtick = chgtick;
                s.stat_lbuf = g_st().curbuf;
                s.stat_lastpos = p;
            });
        }
    }
    with_srch(|s| {
        stat.cur = s.stat_cur;
        stat.cnt = s.stat_cnt;
        stat.exact_match = s.stat_exact_match;
        stat.incomplete = s.stat_incomplete;
        stat.last_maxcount = s.stat_last_maxcount;
    });
    opt_vars().p_ws = save_ws;
}

pub fn f_searchcount(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut pos = curwin_mut().w_cursor;
    let mut pattern: Option<CString> = None;
    let mut maxcount = opt_vars().p_msc as i32;
    let mut timeout = SEARCH_STAT_DEF_TIMEOUT;
    let mut recompute = true;
    let mut stat = Searchstat::default();

    tv_dict_alloc_ret(rettv);

    if shortmess(K_SHM_SEARCHCOUNT) {
        recompute = true;
    }

    if argvars[0].v_type != VarType::VarUnknown {
        let mut error = false;

        if !tv_check_for_nonnull_dict_arg(argvars, 0) {
            return;
        }
        let dict = argvars[0].vval.v_dict_opt();
        if let Some(di) = tv_dict_find_opt(dict.as_deref(), c"timeout", -1) {
            timeout = tv_get_number_chk(&di.di_tv, Some(&mut error)) as i32;
            if error {
                return;
            }
        }
        if let Some(di) = tv_dict_find_opt(dict.as_deref(), c"maxcount", -1) {
            maxcount = tv_get_number_chk(&di.di_tv, Some(&mut error)) as i32;
            if error {
                return;
            }
        }
        recompute = tv_dict_get_bool(dict.as_deref(), c"recompute", i32::from(recompute)) != 0;
        if let Some(di) = tv_dict_find_opt(dict.as_deref(), c"pattern", -1) {
            let Some(pat) = tv_get_string_chk(&di.di_tv) else {
                return;
            };
            pattern = Some(pat.to_owned());
        }
        if let Some(di) = tv_dict_find_opt(dict.as_deref(), c"pos", -1) {
            if di.di_tv.v_type != VarType::VarList {
                semsg!(tr(E_INVARG2), c"pos");
                return;
            }
            if tv_list_len(di.di_tv.vval.v_list_opt().as_deref()) != 3 {
                semsg!(tr(E_INVARG2), c"List format should be [lnum, col, off]",);
                return;
            }
            if let Some(li) = tv_list_find_opt(di.di_tv.vval.v_list_opt(), 0) {
                pos.lnum = tv_get_number_chk(&li.li_tv, Some(&mut error)) as Linenr;
                if error {
                    return;
                }
            }
            if let Some(li) = tv_list_find_opt(di.di_tv.vval.v_list_opt(), 1) {
                pos.col = tv_get_number_chk(&li.li_tv, Some(&mut error)) as Colnr - 1;
                if error {
                    return;
                }
            }
            if let Some(li) = tv_list_find_opt(di.di_tv.vval.v_list_opt(), 2) {
                pos.coladd = tv_get_number_chk(&li.li_tv, Some(&mut error)) as Colnr;
                if error {
                    return;
                }
            }
        }
    }

    save_last_search_pattern();
    save_incsearch_state();
    let mut do_update = true;
    if let Some(pat_cstr) = pattern {
        if pat_cstr.is_empty() {
            do_update = false;
        } else {
            with_srch_mut(|s| {
                let idx = s.last_idx as usize;
                s.spats[idx].pat = Some(pat_cstr);
            });
        }
    }
    if do_update
        && with_srch(|s| {
            s.spats[s.last_idx as usize]
                .pat_ptr()
                .is_some_and(|p| !p.is_empty())
        })
    {
        let cursor_pos = pos;
        update_search_stat(
            0, &mut pos, cursor_pos, &mut stat, recompute, maxcount, timeout,
        );

        tv_dict_add_nr(
            rettv.vval.v_dict_ref(),
            c"current",
            Varnumber::from(stat.cur),
        );
        tv_dict_add_nr(rettv.vval.v_dict_ref(), c"total", Varnumber::from(stat.cnt));
        tv_dict_add_nr(
            rettv.vval.v_dict_ref(),
            c"exact_match",
            Varnumber::from(stat.exact_match),
        );
        tv_dict_add_nr(
            rettv.vval.v_dict_ref(),
            c"incomplete",
            Varnumber::from(stat.incomplete),
        );
        tv_dict_add_nr(
            rettv.vval.v_dict_ref(),
            c"maxcount",
            Varnumber::from(stat.last_maxcount),
        );
    }

    restore_last_search_pattern();
    restore_incsearch_state();
}

fn get_line_and_copy(lnum: Linenr, buf: &mut [u8]) {
    let line = crate::memline::ml_get_slice(lnum);
    let copy_len = line.len().min(buf.len().saturating_sub(1));
    buf[..copy_len].copy_from_slice(&line[..copy_len]);
    buf[copy_len] = 0;
}

pub fn find_pattern_in_path(
    ptr_pat_slice: &[u8],
    mut dir: Direction,
    whole: bool,
    skip_comments: bool,
    type_: i32,
    mut count: i32,
    action: i32,
    start_lnum: Linenr,
    mut end_lnum: Linenr,
    forceit: bool,
    silent: bool,
) {
    let len = ptr_pat_slice.len();
    let ptr_pat_cstr =
        CString::new(crate::memory::bytes_until_nul(ptr_pat_slice)).unwrap_or_default();
    let ptr_pat = ptr_pat_cstr.as_c_str();
    {
        let mut max_path_depth: i32 = 50;
        let mut match_count: i32 = 1;

        let mut curr_fname: Option<std::ffi::CString> =
            curbuf_mut().fname_opt().map(CStr::to_owned);
        let mut prev_fname: Option<std::ffi::CString> = None;
        let mut p_idx: usize;
        let mut define_matched: bool;
        let mut regmatch: Regmatch = Regmatch::new();
        let mut incl_regmatch: Regmatch = Regmatch::new();
        let mut def_regmatch: Regmatch = Regmatch::new();
        let mut matched = false;
        let mut did_show = false;
        let mut found = false;
        let mut i: i32;
        let mut already = false;
        let mut startp_idx: Option<usize> = None;
        let mut curwin_save = crate::globals::GraphPtr::<WinInfoInfo>::NULL;
        let l_g_do_tagpreview = g_st().g_do_tagpreview;

        let mut file_line_vec: Vec<u8> = vec![0; LSIZE];

        if type_ != CHECK_PATH && type_ != FIND_DEFINE && !compl_status_sol() {
            let patsize = len + 5;
            let mut pat_vec: Vec<u8> = vec![0; patsize];
            debug_assert!(len <= i32::MAX as usize);
            format_to_buf!(
                &mut pat_vec[..patsize],
                if whole { c"\\<%.*s\\>" } else { c"%.*s" },
                len as i32,
                ptr_pat,
            );
            let pat_cstr = crate::memory::bytes_as_cstr(&pat_vec);
            regmatch.rm_ic = ignorecase(pat_cstr);
            regmatch.set_prog(vim_regcomp(
                pat_cstr,
                if opt_vars().p_magic { RE_MAGIC } else { 0 },
            ));
            if regmatch.regprog.is_null() {
                return;
            }
        }
        let inc_opt_owned = if curbuf_mut().b_p_inc.is_empty() {
            opt_vars().p_inc.to_owned()
        } else {
            curbuf_mut().b_p_inc.to_owned()
        };
        let inc_opt_cstr = inc_opt_owned.as_c_str();
        if !inc_opt_cstr.to_bytes().is_empty() {
            incl_regmatch.set_prog(vim_regcomp(
                inc_opt_cstr,
                if opt_vars().p_magic { RE_MAGIC } else { 0 },
            ));
            if incl_regmatch.regprog.is_null() {
                regmatch.free_prog();
                return;
            }
            incl_regmatch.rm_ic = false;
        }
        if type_ == FIND_DEFINE
            && (!curbuf_mut().b_p_def.is_empty() || !opt_vars().p_def.is_empty())
        {
            let def_opt_owned = if curbuf_mut().b_p_def.is_empty() {
                opt_vars().p_def.to_owned()
            } else {
                curbuf_mut().b_p_def.to_owned()
            };
            def_regmatch.set_prog(vim_regcomp(
                &def_opt_owned,
                if opt_vars().p_magic { RE_MAGIC } else { 0 },
            ));
            if def_regmatch.regprog.is_null() {
                regmatch.free_prog();
                incl_regmatch.free_prog();
                return;
            }
            def_regmatch.rm_ic = false;
        }
        let mut files: Vec<SearchedFile> = (0..max_path_depth as usize)
            .map(|_| SearchedFile::default())
            .collect();
        let mut old_files = max_path_depth;
        let mut depth: i32 = -1;
        let mut depth_displayed: i32 = -1;

        end_lnum = end_lnum.min(curbuf_mut().b_ml.ml_line_count);
        let mut lnum = start_lnum.min(end_lnum);
        get_line_and_copy(lnum, &mut file_line_vec);

        loop {
            let line_cstr = crate::memory::bytes_as_cstr(&file_line_vec);
            if !incl_regmatch.regprog.is_null() && vim_regexec(&mut incl_regmatch, line_cstr, 0) {
                let p_fname_opt = if curr_fname.as_deref() == curbuf_mut().fname_opt() {
                    curbuf_mut().ffname_opt()
                } else {
                    curr_fname.as_deref()
                };

                let mut new_fname_cstr = if find_substr(inc_opt_cstr, b"\\zs") {
                    find_file_name_in_path(
                        incl_regmatch.span_bytes(0),
                        FNAME_EXP | FNAME_INCL | FNAME_REL,
                        1,
                        p_fname_opt,
                    )
                } else {
                    file_name_in_line(
                        incl_regmatch.endp[0].as_bytes(),
                        0,
                        FNAME_EXP | FNAME_INCL | FNAME_REL,
                        1,
                        p_fname_opt,
                        None,
                    )
                }
                .map(|cstr| {
                    let mut v: Vec<u8> = cstr.into_bytes_with_nul();
                    to_slash(&mut v);
                    crate::memory::cstring_until_nul(&v)
                });
                let mut already_searched = false;
                if let Some(ref nf) = new_fname_cstr {
                    i = 0;
                    loop {
                        if i == depth + 1 {
                            i = old_files;
                        }
                        if i == max_path_depth {
                            break;
                        }
                        if path_equal(
                            nf,
                            files[i as usize].name_ptr().unwrap_or(c""),
                            K_PATH_CMP_EXPAND | K_PATH_CMP_FULL,
                        ) {
                            if type_ != CHECK_PATH
                                && action == ACTION_SHOW_ALL
                                && files[i as usize].matched != 0
                            {
                                msg_putchar(b'\n' as i32);
                                if !g_st().got_int {
                                    msg_home_replace(Some(nf.as_c_str()));
                                    msg_puts(tr(c" (includes previously listed match)"));
                                    prev_fname = None;
                                }
                            }
                            new_fname_cstr = None;
                            already_searched = true;
                            break;
                        }
                        i += 1;
                    }
                }

                if type_ == CHECK_PATH
                    && (action == ACTION_SHOW_ALL
                        || (new_fname_cstr.is_none() && !already_searched))
                {
                    if did_show {
                        msg_putchar(b'\n' as i32);
                    } else {
                        gotocmdline(true);
                        msg_puts_title(tr(c"--- Included files "));
                        if action != ACTION_SHOW_ALL {
                            msg_puts_title(tr(c"not found "));
                        }
                        msg_puts_title(tr(c"in path ---\n"));
                    }
                    did_show = true;
                    while depth_displayed < depth && !g_st().got_int {
                        depth_displayed += 1;
                        for _ in 0..depth_displayed {
                            msg_puts(c"  ");
                        }
                        msg_home_replace(files[depth_displayed as usize].name_ptr());
                        msg_puts(c" -->\n");
                    }
                    if !g_st().got_int {
                        for _ in 0..=depth_displayed {
                            msg_puts(c"  ");
                        }
                        if let Some(ref nf) = new_fname_cstr {
                            msg_outtrans(nf, HLF_D, false);
                        } else {
                            let line_cstr = crate::memory::bytes_as_cstr(&file_line_vec);
                            let line_bytes = line_cstr.to_bytes();
                            if find_substr(inc_opt_cstr, b"\\zs") {
                                p_idx = incl_regmatch.start_offset(0, line_cstr);
                                i = incl_regmatch.span_len(0) as i32;
                            } else {
                                p_idx = incl_regmatch.end_offset(0, line_cstr);
                                let p_bytes = &line_bytes[p_idx.min(line_bytes.len())..];
                                let skip_n = p_bytes
                                    .iter()
                                    .take_while(|&&b| !vim_isfilec(i32::from(b)))
                                    .count();
                                p_idx += skip_n;
                                i = p_bytes[skip_n..]
                                    .iter()
                                    .take_while(|&&b| vim_isfilec(i32::from(b)))
                                    .count() as i32;
                            }

                            if i == 0 {
                                let end_off = incl_regmatch.end_offset(0, line_cstr);
                                p_idx = end_off;
                                i = (line_bytes.len() - end_off) as i32;
                            } else if p_idx > 0 {
                                let prev_b = file_line_vec[p_idx - 1];
                                if matches!(prev_b, b'"' | b'<') {
                                    p_idx -= 1;
                                    i += 1;
                                }
                                let next_b =
                                    file_line_vec.get(p_idx + i as usize).copied().unwrap_or(0);
                                if matches!(next_b, b'"' | b'>') {
                                    i += 1;
                                }
                            }
                            let span = i.max(0) as usize;
                            msg_outtrans_len(
                                &file_line_vec[p_idx..(p_idx + span).min(file_line_vec.len())],
                                i,
                                HLF_D,
                                false,
                            );
                        }

                        if new_fname_cstr.is_none() && action == ACTION_SHOW_ALL {
                            if already_searched {
                                msg_puts(tr(c"  (Already listed)"));
                            } else {
                                msg_puts(tr(c"  NOT FOUND"));
                            }
                        }
                    }
                }

                if let Some(nf_cstr) = new_fname_cstr {
                    if depth + 1 == old_files {
                        let mut bigger: Vec<SearchedFile> = (0..(max_path_depth as usize) * 2)
                            .map(|_| SearchedFile::default())
                            .collect();
                        for idx in 0..=depth {
                            bigger[idx as usize] = std::mem::take(&mut files[idx as usize]);
                        }
                        for idx in old_files..max_path_depth {
                            bigger[(idx + max_path_depth) as usize] =
                                std::mem::take(&mut files[idx as usize]);
                        }
                        old_files += max_path_depth;
                        max_path_depth *= 2;
                        files = bigger;
                    }
                    let fp = CFile::open(&nf_cstr, c"r");
                    let opened = fp.is_some();
                    files[(depth + 1) as usize].fp = fp;
                    if opened {
                        depth += 1;
                        if depth == old_files {
                            files[old_files as usize].name = None;
                            old_files += 1;
                        }
                        files[depth as usize].name = Some(nf_cstr.clone());
                        curr_fname = Some(nf_cstr);
                        files[depth as usize].lnum = 0;
                        files[depth as usize].matched = 0;
                        if action == ACTION_EXPAND && !shortmess(K_SHM_COMPLETIONSCAN) && !silent {
                            g_st().msg_hist_off = true;
                            format_to_buf!(
                                g_st().iobuff_bytes_mut(),
                                tr(c"Scanning included file: %s"),
                                curr_fname.as_deref().unwrap_or(c""),
                            );
                            msg_trunc(g_st().iobuff_bytes_mut(), true, HLF_R);
                        } else if opt_vars().p_verbose >= 5 {
                            verbose_enter();
                            smsg!(
                                0,
                                tr(c"Searching included file %s"),
                                curr_fname.as_deref().unwrap_or(c""),
                            );
                            verbose_leave();
                        }
                    }
                }
            } else {
                let pat_bytes = ptr_pat.to_bytes();
                let pat_slice = &pat_bytes[..len.min(pat_bytes.len())];
                p_idx = 0;
                loop {
                    define_matched = false;
                    let line_cstr = crate::memory::bytes_as_cstr(&file_line_vec);
                    let line_bytes = line_cstr.to_bytes();
                    if !def_regmatch.regprog.is_null()
                        && vim_regexec(&mut def_regmatch, line_cstr, 0)
                    {
                        p_idx = def_regmatch.end_offset(0, line_cstr);
                        let p_bytes = &line_bytes[p_idx.min(line_bytes.len())..];
                        let skip_n = p_bytes
                            .iter()
                            .take_while(|&&b| !vim_iswordc(i32::from(b)))
                            .count();
                        p_idx += skip_n;
                        define_matched = true;
                    }

                    if def_regmatch.regprog.is_null() || define_matched {
                        if define_matched || compl_status_sol() {
                            let p_bytes = &line_bytes[p_idx.min(line_bytes.len())..];
                            let sp_idx = p_idx + crate::charset::skipwhite_len(p_bytes);
                            startp_idx = Some(sp_idx);
                            let start_bytes = &line_bytes[sp_idx.min(line_bytes.len())..];
                            if opt_vars().p_ic {
                                matched = mb_strnicmp(start_bytes, pat_bytes, len) == 0;
                            } else {
                                matched = start_bytes.starts_with(pat_slice);
                            }
                            if matched
                                && define_matched
                                && whole
                                && start_bytes
                                    .get(len)
                                    .is_some_and(|&b| vim_iswordc(i32::from(b)))
                            {
                                matched = false;
                            }
                        } else if !regmatch.regprog.is_null()
                            && vim_regexec(&mut regmatch, line_cstr, p_idx as Colnr)
                        {
                            matched = true;
                            let sp_idx = regmatch.start_offset(0, line_cstr);
                            startp_idx = Some(sp_idx);
                            if skip_comments {
                                if (line_bytes.first() != Some(&b'#')
                                    || !crate::charset::skipwhite_slice(&line_bytes[1..])
                                        .starts_with(b"define"))
                                    && get_leader_len(line_bytes, None, false, true) != 0
                                {
                                    matched = false;
                                }

                                let trim_line = crate::charset::skipwhite_slice(line_bytes);
                                if matched
                                    || trim_line.starts_with(b"/*")
                                    || trim_line.first() == Some(&b'*')
                                {
                                    let prefix = &line_bytes[..sp_idx.min(line_bytes.len())];
                                    let mut idx = 0usize;
                                    while idx < prefix.len() {
                                        if matched
                                            && prefix[idx] == b'/'
                                            && matches!(prefix.get(idx + 1), Some(&b'*' | &b'/'))
                                        {
                                            matched = false;
                                            if prefix.get(idx + 1) == Some(&b'/') {
                                                break;
                                            }
                                            idx += 1;
                                        } else if !matched
                                            && prefix[idx] == b'*'
                                            && prefix.get(idx + 1) == Some(&b'/')
                                        {
                                            matched = true;
                                            idx += 1;
                                        }
                                        idx += 1;
                                    }
                                }
                            }
                        }
                    }

                    if !matched {
                        break;
                    }

                    let mut stop_fpip_loop = false;
                    if action == ACTION_EXPAND {
                        let mut cont_s_ipos = false;

                        if depth == -1 && lnum == curwin_mut().w_cursor.lnum {
                            stop_fpip_loop = true;
                        } else {
                            found = true;
                            let mut aux_idx = startp_idx.unwrap_or(0);
                            p_idx = aux_idx;
                            let mut skip_add = false;
                            let line_bytes = crate::memory::bytes_until_nul(&file_line_vec);
                            let p_rem = &line_bytes[p_idx.min(line_bytes.len())..];
                            if compl_status_adding() && (p_rem.len() as i32) >= ins_compl_len() {
                                p_idx += ins_compl_len() as usize;
                                let p_bytes = &line_bytes[p_idx.min(line_bytes.len())..];
                                if vim_iswordp(p_bytes) {
                                    skip_add = true;
                                } else {
                                    let rem = find_word_start(p_bytes);
                                    p_idx = line_bytes.len() - rem.len();
                                }
                            }
                            if !skip_add {
                                let p_bytes = &line_bytes[p_idx.min(line_bytes.len())..];
                                let rem = find_word_end(p_bytes);
                                p_idx = line_bytes.len() - rem.len();
                                i = (p_idx as isize - aux_idx as isize) as i32;
                                let mut use_iobuff = false;

                                if compl_status_adding() && i == ins_compl_len() {
                                    let i_u = i as usize;
                                    g_st().iobuff[..i_u]
                                        .copy_from_slice(&file_line_vec[aux_idx..aux_idx + i_u]);
                                    g_st().iobuff[i_u] = 0;

                                    if depth < 0 {
                                        if lnum >= end_lnum {
                                            skip_add = true;
                                        } else {
                                            lnum += 1;
                                            get_line_and_copy(lnum, &mut file_line_vec);
                                        }
                                    } else if vim_fgets(
                                        &mut file_line_vec,
                                        files[depth as usize].fp.as_ref().unwrap(),
                                    ) {
                                        skip_add = true;
                                    }

                                    if !skip_add {
                                        let next_bytes =
                                            crate::memory::bytes_until_nul(&file_line_vec);
                                        p_idx = crate::charset::skipwhite_len(next_bytes);
                                        aux_idx = p_idx;
                                        already = true;
                                        let rem1 = find_word_start(&next_bytes[p_idx..]);
                                        let rem2 = find_word_end(rem1);
                                        p_idx = next_bytes.len() - rem2.len();
                                        if p_idx > aux_idx {
                                            if file_line_vec[aux_idx] != b')'
                                                && g_st().iobuff[(i - 1) as usize] != TAB
                                            {
                                                if g_st().iobuff[(i - 1) as usize] != b' ' {
                                                    g_st().iobuff[i as usize] = b' ';
                                                    i += 1;
                                                }
                                                if opt_vars().p_js
                                                    && matches!(
                                                        g_st().iobuff[(i - 2) as usize],
                                                        b'.' | b'?' | b'!'
                                                    )
                                                {
                                                    g_st().iobuff[i as usize] = b' ';
                                                    i += 1;
                                                }
                                            }
                                            let mut copy_len = p_idx - aux_idx;
                                            if copy_len >= IOSIZE - (i as usize) {
                                                copy_len = IOSIZE - (i as usize) - 1;
                                            }
                                            let dst_start = i as usize;
                                            g_st().iobuff[dst_start..dst_start + copy_len]
                                                .copy_from_slice(
                                                    &file_line_vec[aux_idx..aux_idx + copy_len],
                                                );
                                            i += copy_len as i32;
                                            cont_s_ipos = true;
                                        }
                                        g_st().iobuff[i as usize] = 0;
                                        use_iobuff = true;

                                        if i == ins_compl_len() {
                                            skip_add = true;
                                        }
                                    }
                                }

                                if !skip_add {
                                    let len_u = i.max(0) as usize;
                                    let fname_short =
                                        if curr_fname.as_deref() == curbuf_mut().fname_opt() {
                                            None
                                        } else {
                                            path_try_shorten_fname(curr_fname.as_deref())
                                        };
                                    let add_r = if use_iobuff {
                                        let word = g_st().iobuff[..len_u].to_vec();
                                        ins_compl_add_infercase(
                                            &word,
                                            i,
                                            opt_vars().p_ic,
                                            fname_short,
                                            dir,
                                            cont_s_ipos,
                                            0,
                                        )
                                    } else {
                                        ins_compl_add_infercase(
                                            &file_line_vec[aux_idx..aux_idx + len_u],
                                            i,
                                            opt_vars().p_ic,
                                            fname_short,
                                            dir,
                                            cont_s_ipos,
                                            0,
                                        )
                                    };
                                    if add_r == Some(true) {
                                        dir = FORWARD;
                                    } else if add_r == Some(false) {
                                        stop_fpip_loop = true;
                                    }
                                }
                            }
                        }
                    } else if action == ACTION_SHOW_ALL {
                        found = true;
                        if !did_show {
                            gotocmdline(true);
                        }
                        if curr_fname != prev_fname {
                            if did_show {
                                msg_putchar(b'\n' as i32);
                            }
                            if !g_st().got_int {
                                msg_home_replace(curr_fname.as_deref());
                            }
                            prev_fname = curr_fname.clone();
                        }
                        did_show = true;
                        if !g_st().got_int {
                            let (fp_arg, lnum_arg) = if depth == -1 {
                                (None, &mut lnum)
                            } else {
                                let entry = &mut files[depth as usize];
                                (entry.fp.as_ref(), &mut entry.lnum)
                            };
                            show_pat_in_path(
                                &mut file_line_vec,
                                type_,
                                true,
                                action,
                                fp_arg,
                                lnum_arg,
                                match_count,
                            );
                            match_count += 1;
                        }

                        for idx in 0..=depth {
                            files[idx as usize].matched = 1;
                        }
                    } else {
                        count -= 1;
                        if count <= 0 {
                            found = true;
                            if depth == -1
                                && lnum == curwin_mut().w_cursor.lnum
                                && l_g_do_tagpreview == 0
                            {
                                emsg(tr(c"E387: Match is on current line"));
                            } else if action == ACTION_SHOW {
                                let (fp_arg, lnum_arg) = if depth == -1 {
                                    (None, &mut lnum)
                                } else {
                                    let entry = &mut files[depth as usize];
                                    (entry.fp.as_ref(), &mut entry.lnum)
                                };
                                show_pat_in_path(
                                    &mut file_line_vec,
                                    type_,
                                    did_show,
                                    action,
                                    fp_arg,
                                    lnum_arg,
                                    1,
                                );
                                did_show = true;
                            } else {
                                if l_g_do_tagpreview != 0 {
                                    curwin_save =
                                        crate::globals::GraphPtr::<WinInfoInfo>::of(curwin_ptr());
                                    prepare_tagpreview(true, !opt_vars().p_pvp.is_empty());
                                }
                                if action == ACTION_SPLIT {
                                    if !win_split(0, 0) {
                                        stop_fpip_loop = true;
                                    } else {
                                        reset_binding(curwin_mut());
                                    }
                                }
                                if !stop_fpip_loop {
                                    if depth == -1 {
                                        if l_g_do_tagpreview != 0 {
                                            if !win_valid(curwin_save.opt())
                                                || !getfile_success(getfile(
                                                    curwin_save.buf_mut().b_fnum,
                                                    None,
                                                    None,
                                                    true,
                                                    lnum,
                                                    forceit,
                                                ))
                                            {
                                                stop_fpip_loop = true;
                                            }
                                        } else {
                                            setpcmark();
                                        }
                                        if !stop_fpip_loop {
                                            curwin_mut().w_cursor.lnum = lnum;
                                            check_cursor(curwin_mut());
                                        }
                                    } else if !getfile_success(getfile(
                                        0,
                                        files[depth as usize].name_ptr(),
                                        None,
                                        true,
                                        files[depth as usize].lnum,
                                        forceit,
                                    )) {
                                        stop_fpip_loop = true;
                                    } else {
                                        curwin_mut().w_cursor.lnum = files[depth as usize].lnum;
                                    }
                                }
                            }
                            if !stop_fpip_loop {
                                if action != ACTION_SHOW {
                                    curwin_mut().w_cursor.col = startp_idx.unwrap_or(0) as Colnr;
                                    curwin_mut().w_set_curswant = true;
                                }

                                if l_g_do_tagpreview != 0
                                    && curwin_ptr() != curwin_save
                                    && win_valid(curwin_save.opt())
                                {
                                    validate_cursor(curwin_mut());
                                    redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                                    win_enter(curwin_save.borrow_mut(), true);
                                }
                                stop_fpip_loop = true;
                            }
                        }
                    }

                    matched = false;
                    if stop_fpip_loop {
                        already = false;
                        // break outer loop too
                        depth_displayed = i32::MIN;
                        break;
                    }
                    if def_regmatch.regprog.is_null()
                        && action == ACTION_EXPAND
                        && !compl_status_sol()
                        && let Some(sp_idx) = startp_idx
                        && file_line_vec.get(sp_idx).copied().unwrap_or(0) != 0
                        && {
                            let start_cstr = crate::memory::bytes_as_cstr(&file_line_vec[sp_idx..]);
                            let step = utfc_ptr2len(start_cstr) as usize;
                            start_cstr.to_bytes().len() > step
                        }
                    {
                        let start_cstr = crate::memory::bytes_as_cstr(&file_line_vec[sp_idx..]);
                        p_idx = sp_idx + (utfc_ptr2len(start_cstr) as usize);
                        continue;
                    }
                    break;
                }
                if depth_displayed == i32::MIN {
                    break;
                }
            }
            line_breakcheck();
            if action == ACTION_EXPAND {
                ins_compl_check_keys(30, false);
            }
            if g_st().got_int || ins_compl_interrupted() {
                break;
            }

            while depth >= 0
                && !already
                && vim_fgets(
                    &mut file_line_vec,
                    files[depth as usize].fp.as_ref().unwrap(),
                )
            {
                files[depth as usize].fp = None;
                old_files -= 1;
                files[old_files as usize].name = files[depth as usize].name.take();
                files[old_files as usize].matched = files[depth as usize].matched;
                depth -= 1;
                curr_fname = if depth == -1 {
                    curbuf_mut().fname_opt().map(CStr::to_owned)
                } else {
                    files[depth as usize].name.clone()
                };
                depth_displayed = depth_displayed.min(depth);
            }
            if depth >= 0 {
                files[depth as usize].lnum += 1;
                i = crate::memory::bytes_until_nul(&file_line_vec).len() as i32;
                if i > 0 && file_line_vec[(i - 1) as usize] == b'\n' {
                    i -= 1;
                    file_line_vec[i as usize] = 0;
                }
                if i > 0 && file_line_vec[(i - 1) as usize] == b'\r' {
                    i -= 1;
                    file_line_vec[i as usize] = 0;
                }
            } else if !already {
                lnum += 1;
                if lnum > end_lnum {
                    break;
                }
                get_line_and_copy(lnum, &mut file_line_vec);
            }
            already = false;
        }

        for idx in 0..=depth {
            files[idx as usize].fp = None;
        }

        if type_ == CHECK_PATH {
            if !did_show {
                if action != ACTION_SHOW_ALL {
                    msg(tr(c"All included files were found"), 0);
                } else {
                    msg(tr(c"No included files"), 0);
                }
            }
        } else if !found && action != ACTION_EXPAND && !silent {
            if g_st().got_int || ins_compl_interrupted() {
                emsg(tr(E_INTERR));
            } else if type_ == FIND_DEFINE {
                emsg(tr(c"E388: Couldn't find definition"));
            } else {
                emsg(tr(c"E389: Couldn't find pattern"));
            }
        }
        if action == ACTION_SHOW || action == ACTION_SHOW_ALL {
            msg_end();
        }

        regmatch.free_prog();
        incl_regmatch.free_prog();
        def_regmatch.free_prog();
    }
}

fn show_pat_in_path(
    line_buf: &mut [u8],
    type_: i32,
    did_show: bool,
    action: i32,
    fp: Option<&CFile>,
    lnum: &mut Linenr,
    count: i32,
) {
    if did_show {
        msg_putchar(b'\n' as i32);
    } else if g_st().msg_silent == 0 {
        gotocmdline(true);
    }
    if g_st().got_int {
        return;
    }
    let mut use_line_buf = true;
    let mut linelen = crate::memory::bytes_until_nul(line_buf).len();
    loop {
        if fp.is_some() {
            if linelen > 0 && line_buf[linelen - 1] == b'\n' {
                linelen -= 1;
            }
            if linelen > 0 && line_buf[linelen - 1] == b'\r' {
                linelen -= 1;
            }
            line_buf[linelen] = 0;
        }
        let cur_bytes = if use_line_buf {
            &line_buf[..linelen]
        } else {
            &ml_get(*lnum).to_bytes()[..linelen]
        };
        let has_trailing_bsl = cur_bytes.last() == Some(&b'\\');
        if action == ACTION_SHOW_ALL {
            format_to_buf!(g_st().iobuff_bytes_mut(), c"%3d: ", count);
            msg_puts(crate::memory::bytes_as_cstr(g_st().iobuff_bytes()));
            format_to_buf!(g_st().iobuff_bytes_mut(), c"%4d", *lnum);
            msg_puts_hl(
                crate::memory::bytes_as_cstr(g_st().iobuff_bytes()),
                HLF_N,
                false,
            );
            msg_puts(c" ");
        }
        if use_line_buf {
            msg_prt_line(crate::memory::bytes_until_nul(line_buf), false);
        } else {
            msg_prt_line(ml_get(*lnum).to_bytes(), false);
        }

        if g_st().got_int || type_ != FIND_DEFINE || !has_trailing_bsl {
            break;
        }

        if let Some(fp) = fp {
            if vim_fgets(line_buf, fp) {
                break;
            }
            use_line_buf = true;
            linelen = crate::memory::bytes_until_nul(line_buf).len();
            *lnum += 1;
        } else {
            *lnum += 1;
            if *lnum > curbuf_mut().b_ml.ml_line_count {
                break;
            }
            use_line_buf = false;
            linelen = ml_get_len(*lnum) as usize;
        }
        msg_putchar(b'\n' as i32);
    }
}

pub fn get_search_pattern(pat: &mut SearchPattern) {
    *pat = with_srch(|s| s.spats[0].clone());
}

pub fn get_substitute_pattern(pat: &mut SearchPattern) {
    *pat = with_srch(|s| s.spats[1].clone());
    pat.off = SearchOffset {
        dir: 0,
        line: false,
        end: false,
        off: 0,
    };
}

pub fn get_search_pattern_timestamp(substitute: bool) -> Timestamp {
    with_srch(|s| {
        s.spats[if substitute {
            RE_SUBST as usize
        } else {
            RE_SEARCH as usize
        }]
        .timestamp
    })
}

pub fn search_pattern_cleared(substitute: bool) -> bool {
    with_srch(|s| {
        s.spats[if substitute {
            RE_SUBST as usize
        } else {
            RE_SEARCH as usize
        }]
        .pat
        .is_none()
    })
}

pub fn set_search_pattern(pat: SearchPattern) {
    with_srch_mut(|s| {
        free_spat(&mut s.spats[0]);
        s.spats[0] = pat;
    });
    set_vv_searchforward();
}

pub fn set_substitute_pattern(pat: SearchPattern) {
    with_srch_mut(|s| {
        free_spat(&mut s.spats[1]);
        s.spats[1] = pat;
        s.spats[1].off = SearchOffset {
            dir: 0,
            line: false,
            end: false,
            off: 0,
        };
    });
}

pub fn set_last_used_pattern(is_substitute_pattern: bool) {
    with_srch_mut(|s| s.last_idx = i32::from(is_substitute_pattern));
}

pub fn search_was_last_used() -> bool {
    with_srch(|s| s.last_idx == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_submatch_and_pos_helpers() {
        let mut rm: Regmmatch = Regmmatch::new();
        for i in 0..10 {
            rm.startpos[i].lnum = -1;
        }
        assert_eq!(first_submatch(&rm), 0);
        rm.startpos[3].lnum = 0;
        assert_eq!(first_submatch(&rm), 3);

        let mut p = CursorPos {
            lnum: 2,
            col: 5,
            coladd: 1,
        };
        assert!(!empty_pos(p));
        clearpos(&mut p);
        assert!(empty_pos(p));
        assert!(getfile_success(crate::buffer::GetFileResult::SameFile));
        assert!(getfile_success(crate::buffer::GetFileResult::OpenOther));
        assert!(!getfile_success(crate::buffer::GetFileResult::Error));
    }

    #[test]
    fn pat_has_uppercase_handles_utf8_and_escapes() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let save_magic = opt_vars().p_magic;
        let save_cmp = opt_vars().cmp_flags;
        opt_vars().p_magic = true;
        opt_vars().cmp_flags =
            crate::mbyte::K_OPT_CMP_FLAG_INTERNAL | crate::mbyte::K_OPT_CMP_FLAG_KEEPASCII;
        for (pat, expected) in [
            (c"", false),
            (c"ä", false),
            (c"Ä", true),
            (c"äaÅ", true),
            (c"\\", false),
            (c"ab$\\", false),
            (c"\\Ab", false),
            (c"\\AU", true),
            (c"\\_Ab", false),
            (c"\\_AU", true),
            (c"aa\\%Ab", false),
            (c"aab\\%AU", true),
        ] {
            assert_eq!(pat_has_uppercase(pat), expected);
        }
        opt_vars().p_magic = save_magic;
        opt_vars().cmp_flags = save_cmp;
    }
}
