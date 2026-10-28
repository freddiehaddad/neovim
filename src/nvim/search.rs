//! Code for normal mode searching commands (`src/nvim/search.c`).

use crate::autocmd::{EVENT_SEARCHWRAPPED, apply_autocmds};
use crate::buffer::{buf_T, buf_get_changedtick, kUIMessages};
use crate::change::{TAB, get_leader_len};
use crate::charset::{NUL, ascii_isdigit, skipwhite, vim_isfilec, vim_iswordc, vim_iswordp};
use crate::cmdhist::{HIST_SEARCH, add_to_history};
use crate::cursor::{
    check_cursor, clearpos, colnr_T, dec_cursor, equalpos, get_cursor_line_len,
    get_cursor_line_ptr, inc_cursor, linenr_T, lpos_T, lt, ltoreq, pos_T,
};
use crate::drawscreen::{
    UPD_INVERTED, UPD_SOME_VALID, UPD_VALID, redraw_all_later, redraw_curbuf_later, redraw_later,
    setcursor, show_cursor_info_later, showmode, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, VarType, dict_T, dictitem_T, listitem_T, tv_check_for_nonnull_dict_arg,
    tv_dict_add_nr, tv_dict_alloc_ret, tv_dict_find, tv_dict_get_bool, tv_get_number_chk,
    tv_get_string_chk, tv_list_find, tv_list_len, typval_T, varnumber_T,
};
use crate::eval::vars::{VV_SEARCHFORWARD, set_vim_var_nr};
use crate::ex_cmds::{HLF_R, RE_BOTH, RE_SUBST, SEARCH_HIS, getfile, prepare_tagpreview};
use crate::ex_docmd::{
    ACTION_SHOW, ACTION_SHOW_ALL, ACTION_SPLIT, CHECK_PATH, CMOD_KEEPPATTERNS, FIND_DEFINE,
    SEARCH_MSG, set_no_hlsearch,
};
use crate::ex_getln::gotocmdline;
use crate::extmark::{TriState, kFalse, kNone, kTrue};
use crate::file_search::{
    FNAME_EXP, FNAME_INCL, FNAME_REL, file_name_in_line, find_file_name_in_path,
};
use crate::fileio::{FILE, vim_fgets};
use crate::fold::{foldOpenCursor, hasFolding};
use crate::globals::*;
use crate::indent_c::{FM_BACKWARD, FM_BLOCKSTOP, FM_SKIPCOMM, check_linecomment};
use crate::input::char_avail;
use crate::input_cmdatom::cmdarg_T;
use crate::insexpand::{
    ACTION_EXPAND, LSIZE, compl_status_adding, compl_status_sol, ctrl_x_mode_not_default,
    find_word_end, find_word_start, ins_compl_add_infercase, ins_compl_check_keys,
    ins_compl_interrupted, ins_compl_len, kShmCompletionscan,
};
use crate::mark::setpcmark;
use crate::r#match::kCpoSearch;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_isupper, mb_strcmp_ic, mb_strnicmp, utf_char2bytes, utf_head_off, utf_iscomposing_first,
    utf_ptr2char, utfc_ptr2len,
};
use crate::memline::{decl, inc, incl, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len};
use crate::memory::xstrlcpy;
use crate::message::{
    emsg, give_warning, iemsg, messaging, msg, msg_check, msg_clr_eos, msg_end, msg_ext_set_kind,
    msg_home_replace, msg_outtrans, msg_prt_line, msg_putchar, msg_puts, msg_puts_hl,
    msg_puts_title, msg_start, msg_strtrunc, msg_trunc, semsg, smsg, verbose_enter, verbose_leave,
};
use crate::mouse::setmouse;
use crate::r#move::validate_cursor;
use crate::normal::{
    FM_FORWARD, SEARCH_ECHO, SEARCH_MARK, SEARCH_REV, kCpoMatch, kOptFdoFlagSearch,
    kShmSearchcount, may_start_select,
};
use crate::option::{OptInt, shortmess};
use crate::option_vars::*;
use crate::os::fs::os_fopen;
use crate::os::input::{fast_breakcheck, line_breakcheck};
use crate::os::time::{Timestamp, os_delay, os_time};
use crate::path::{kPathCmpExpand, kPathCmpFull, path_equal, path_try_shorten_fname, to_slash};
use crate::plines::getvcol;
use crate::profile::{profile_passed_limit, profile_setlimit, proftime_T};
use crate::regexp::{
    MAGIC_ALL, MAGIC_ON, MAXCOL, RE_MAGIC, magic_T, regmatch_T, regmmatch_T, skip_regexp_ex,
    vim_regcomp, vim_regexec, vim_regexec_multi, vim_regfree,
};
use crate::state::MODE_SHOWMATCH;
use crate::strings::{reverse_text, tr, vim_snprintf, vim_strchr};
use crate::textformat::oparg_T;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_cursor_shape, ui_flush, ui_has, vim_beep};
use crate::window::{reset_binding, win_T, win_enter, win_split, win_valid};
use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

pub type Direction = c_int;
pub(crate) const FORWARD: Direction = 1;
pub(crate) const BACKWARD: Direction = -1;
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::vim::HLF_N;
use crate::cmdexpand::{HLF_D, RE_LAST, SEARCH_NOOF, SEARCH_OPT, SEARCH_PEEK, SEARCH_START};
use crate::eval::funcs::{RE_SEARCH, SEARCH_COL, SEARCH_END, SEARCH_KEEP, kMTLineWise};
use crate::globals::IOSIZE;
pub type AdditionalData = crate::msgpack_rpc::unpacker::AdditionalData;

const SEARCH_STAT_DEF_TIMEOUT: c_int = 40;
const SEARCH_STAT_BUF_LEN: usize = 16;

const kCpoLineoff: c_int = b'o' as c_int;
const kCpoScolon: c_int = b';' as c_int;
pub(crate) const kCpoMatchbsl: c_int = b'M' as c_int;
const kCpoShowmatch: c_int = b'm' as c_int;

pub(crate) const kShmSearch: c_int = b's' as c_int;

const kOptBoFlagShowmatch: c_uint = 0x2000;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SearchOffset {
    pub dir: c_char,
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

    #[inline]
    pub fn pat_ptr(&self) -> *mut c_char {
        self.pat
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
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
pub struct searchit_arg_T {
    pub sa_stop_lnum: linenr_T,
    pub sa_tm: Option<proftime_T>,
    pub sa_timed_out: c_int,
    pub sa_wrapped: c_int,
}

impl searchit_arg_T {
    pub const fn new() -> Self {
        Self {
            sa_stop_lnum: 0,
            sa_tm: None,
            sa_timed_out: 0,
            sa_wrapped: 0,
        }
    }
}

impl Default for searchit_arg_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct searchstat_T {
    pub cur: c_int,
    pub cnt: c_int,
    pub exact_match: bool,
    pub incomplete: c_int,
    pub last_maxcount: c_int,
}

#[derive(Default)]
struct SearchedFile {
    fp: Option<crate::os::fs::CFile>,
    name: Option<CString>,
    lnum: linenr_T,
    matched: c_int,
}

impl SearchedFile {
    fn fp_ptr(&self) -> *mut FILE {
        self.fp.as_ref().map_or(ptr::null_mut(), |f| f.as_ptr())
    }

    fn name_ptr(&self) -> *mut c_char {
        self.name
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

const E_SEARCH_HIT_TOP_WITHOUT_MATCH_FOR_STR: &CStr = c"E384: Search hit TOP without match for: %s";
const E_SEARCH_HIT_BOTTOM_WITHOUT_MATCH_FOR_STR: &CStr =
    c"E385: Search hit BOTTOM without match for: %s";

const EMPTY_SEARCH_OFFSET: SearchOffset = SearchOffset {
    dir: b'/' as c_char,
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
    last_idx: c_int,
    lastc: [u8; 2],
    lastcdir: Direction,
    last_t_cmd: bool,
    lastc_bytes: [c_char; MAX_SCHAR_SIZE + 1],
    lastc_bytelen: c_int,
    saved_spats: [SearchPattern; 2],
    saved_mr_pattern: Option<CString>,
    saved_spats_last_idx: c_int,
    saved_spats_no_hlsearch: bool,
    mr_pattern: Option<CString>,
    save_level: c_int,
    saved_last_search_spat: SearchPattern,
    did_save_last_search_spat: c_int,
    saved_last_idx: c_int,
    saved_no_hlsearch: bool,
    saved_search_match_endcol: colnr_T,
    saved_search_match_lines: linenr_T,
    findmatch_pos: pos_T,
    stat_lastpos: pos_T,
    stat_cur: c_int,
    stat_cnt: c_int,
    stat_exact_match: bool,
    stat_incomplete: c_int,
    stat_last_maxcount: c_int,
    stat_chgtick: c_int,
    stat_lastpat: Option<CString>,
    stat_lbuf: *mut buf_T,
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
            findmatch_pos: pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            stat_lastpos: pos_T {
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
            stat_lbuf: ptr::null_mut(),
        }
    }
}

static SEARCH_STATE: GlobalState<SearchModuleState> = GlobalState::new(SearchModuleState::new());

#[inline]
fn srch() -> &'static mut SearchModuleState {
    unsafe { &mut *SEARCH_STATE.as_ptr() }
}

#[inline]
const fn empty_pos(a: pos_T) -> bool {
    a.lnum == 0 && a.col == 0 && a.coladd == 0
}

#[inline]
const fn getfile_success(x: c_int) -> bool {
    x <= 0
}

fn find_substr(haystack: *const c_char, needle: &[u8]) -> bool {
    unsafe {
        let bytes = CStr::from_ptr(haystack).to_bytes();
        bytes.windows(needle.len()).any(|w| w == needle)
    }
}

/// Translate search pattern for `vim_regcomp()`.
pub fn search_regcomp(
    mut pat: *mut c_char,
    mut patlen: usize,
    used_pat: *mut *mut c_char,
    pat_save: c_int,
    pat_use: c_int,
    options: c_int,
    mut magic: bool,
    regmatch: &mut regmmatch_T,
) -> c_int {
    unsafe {
        g_st().rc_did_emsg = false;

        if pat.is_null() || *pat == NUL {
            let i = if pat_use == RE_LAST {
                srch().last_idx
            } else {
                pat_use
            };
            if srch().spats[i as usize].pat.is_none() {
                if pat_use == RE_SUBST {
                    emsg(tr(e_nopresub.as_ptr()));
                } else {
                    emsg(tr(e_noprevre.as_ptr()));
                }
                g_st().rc_did_emsg = true;
                return FAIL;
            }
            pat = srch().spats[i as usize].pat_ptr();
            patlen = srch().spats[i as usize].patlen();
            magic = srch().spats[i as usize].magic;
            g_st().Search.no_smartcase = srch().spats[i as usize].no_scs;
        } else if (options & SEARCH_HIS) != 0 {
            add_to_history(HIST_SEARCH, pat, patlen, true, c_int::from(NUL));
        }

        if !used_pat.is_null() {
            *used_pat = pat;
        }

        srch().mr_pattern = if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0
            && *(**curwin.as_ptr()).w_onebuf_opt.wo_rlc == b's' as c_char
        {
            Some(reverse_text(std::ffi::CStr::from_ptr(pat)))
        } else {
            Some(
                CString::new(std::slice::from_raw_parts(pat.cast::<u8>(), patlen))
                    .unwrap_or_default(),
            )
        };

        if (options & SEARCH_KEEP) == 0 && (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) == 0 {
            if pat_save == RE_SEARCH || pat_save == RE_BOTH {
                save_re_pat(RE_SEARCH, pat, patlen, c_int::from(magic));
            }
            if pat_save == RE_SUBST || pat_save == RE_BOTH {
                save_re_pat(RE_SUBST, pat, patlen, c_int::from(magic));
            }
        }

        regmatch.rmm_ic = ignorecase(pat);
        regmatch.rmm_maxcol = 0;
        regmatch.regprog = vim_regcomp(pat, if magic { RE_MAGIC } else { 0 });
        if regmatch.regprog.is_null() {
            return FAIL;
        }
        OK
    }
}

/// Get search pattern used by `search_regcomp()`.
pub fn get_search_pat() -> *mut c_char {
    unsafe {
        (&raw const srch().mr_pattern)
            .as_ref()
            .unwrap()
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

pub fn save_re_pat(idx: c_int, pat: *mut c_char, patlen: usize, magic: c_int) {
    unsafe {
        let i = idx as usize;
        if srch().spats[i].pat_ptr() == pat {
            return;
        }

        let new_pat =
            CString::new(std::slice::from_raw_parts(pat.cast::<u8>(), patlen)).unwrap_or_default();
        free_spat(&mut srch().spats[i]);
        srch().spats[i].pat = Some(new_pat);
        srch().spats[i].magic = magic != 0;
        srch().spats[i].no_scs = g_st().Search.no_smartcase;
        srch().spats[i].timestamp = os_time();
        srch().spats[i].additional_data = None;
        srch().last_idx = idx;
        if opt_vars().p_hls != 0 {
            redraw_all_later(UPD_SOME_VALID);
        }
        set_no_hlsearch(false);
    }
}

pub fn save_search_patterns() {
    let prev = srch().save_level;
        srch().save_level += 1;
        if prev != 0 {
            return;
        }

        for i in 0..2 {
            srch().saved_spats[i] = srch().spats[i].clone();
        }
        srch().saved_mr_pattern = srch().mr_pattern.clone();
        srch().saved_spats_last_idx = srch().last_idx;
        srch().saved_spats_no_hlsearch = g_st().Search.no_hlsearch;
}

pub fn restore_search_patterns() {
    srch().save_level -= 1;
        if srch().save_level != 0 {
            return;
        }

        for i in 0..2 {
            free_spat(&mut srch().spats[i]);
            srch().spats[i] = std::mem::replace(&mut srch().saved_spats[i], EMPTY_SEARCH_PATTERN);
        }
        set_vv_searchforward();
        srch().mr_pattern = srch().saved_mr_pattern.take();
        srch().last_idx = srch().saved_spats_last_idx;
        set_no_hlsearch(srch().saved_spats_no_hlsearch);
}

#[inline]
fn free_spat(spat: &mut SearchPattern) {
    spat.pat = None;
        spat.additional_data = None;
}

pub fn free_search_patterns() {
    free_spat(&mut srch().spats[0]);
        free_spat(&mut srch().spats[1]);
        srch().spats = [EMPTY_SEARCH_PATTERN, EMPTY_SEARCH_PATTERN];
        srch().mr_pattern = None;
}

pub fn save_last_search_pattern() {
    srch().did_save_last_search_spat += 1;
        if srch().did_save_last_search_spat != 1 {
            return;
        }

        srch().saved_last_search_spat = srch().spats[RE_SEARCH as usize].clone();
        srch().saved_last_idx = srch().last_idx;
        srch().saved_no_hlsearch = g_st().Search.no_hlsearch;
}

pub fn restore_last_search_pattern() {
    srch().did_save_last_search_spat -= 1;
        if srch().did_save_last_search_spat > 0 {
            return;
        }
        if srch().did_save_last_search_spat != 0 {
            iemsg(
                c"restore_last_search_pattern() called more often than save_last_search_pattern()"
                    .as_ptr(),
            );
            return;
        }

        srch().spats[RE_SEARCH as usize] = std::mem::replace(
            &mut srch().saved_last_search_spat,
            EMPTY_SEARCH_PATTERN,
        );
        set_vv_searchforward();
        srch().last_idx = srch().saved_last_idx;
        set_no_hlsearch(srch().saved_no_hlsearch);
}

fn save_incsearch_state() {
    srch().saved_search_match_endcol = g_st().Search.match_endcol;
        srch().saved_search_match_lines = g_st().Search.match_lines;
}

fn restore_incsearch_state() {
    g_st().Search.match_endcol = srch().saved_search_match_endcol;
        g_st().Search.match_lines = srch().saved_search_match_lines;
}

pub fn ignorecase(pat: *mut c_char) -> c_int {
    ignorecase_opt(pat, opt_vars().p_ic, opt_vars().p_scs)
}

pub fn ignorecase_opt(pat: *mut c_char, ic_in: c_int, scs: c_int) -> c_int {
    unsafe {
        let mut ic = ic_in;
        if ic != 0
            && !g_st().Search.no_smartcase
            && scs != 0
            && !(ctrl_x_mode_not_default() && (*g_st().curbuf).b_p_inf != 0)
        {
            ic = c_int::from(!pat_has_uppercase(pat));
        }
        g_st().Search.no_smartcase = false;
        ic
    }
}

pub fn pat_has_uppercase(pat: *mut c_char) -> bool {
    unsafe {
        let mut p = pat;
        let mut magic_val: magic_T = MAGIC_ON;

        skip_regexp_ex(
            pat,
            c_int::from(NUL),
            opt_vars().p_magic,
            false,
            None,
            ptr::null_mut(),
            &raw mut magic_val,
        );

        while *p != NUL {
            let l = utfc_ptr2len(p);

            if l > 1 {
                if mb_isupper(utf_ptr2char(p)) {
                    return true;
                }
                p = p.add(l as usize);
            } else if *p == b'\\' as c_char && magic_val <= MAGIC_ON {
                if (*p.add(1) == b'_' as c_char || *p.add(1) == b'%' as c_char) && *p.add(2) != NUL
                {
                    p = p.add(3);
                } else if *p.add(1) != NUL {
                    p = p.add(2);
                } else {
                    p = p.add(1);
                }
            } else if (*p == b'%' as c_char || *p == b'_' as c_char) && magic_val == MAGIC_ALL {
                if *p.add(1) != NUL {
                    p = p.add(2);
                } else {
                    p = p.add(1);
                }
            } else if mb_isupper(c_int::from(*p as u8)) {
                return true;
            } else {
                p = p.add(1);
            }
        }
        false
    }
}

pub fn last_csearch() -> *const c_char {
    (&raw const srch().lastc_bytes).cast::<c_char>()
}

pub fn last_csearch_forward() -> c_int {
    c_int::from(srch().lastcdir == FORWARD)
}

pub fn last_csearch_until() -> c_int {
    c_int::from(srch().last_t_cmd)
}

pub fn set_last_csearch(c: c_int, s: *mut c_char, len: c_int) {
    unsafe {
        srch().lastc[0] = c as u8;
        srch().lastc_bytelen = len;
        if len != 0 {
            ptr::copy_nonoverlapping(s, (&raw mut srch().lastc_bytes).cast::<c_char>(), len as usize);
        } else {
            srch().lastc_bytes = [0; MAX_SCHAR_SIZE + 1];
        }
    }
}

pub fn set_csearch_direction(cdir: Direction) {
    srch().lastcdir = cdir;
}

pub fn set_csearch_until(t_cmd: c_int) {
    srch().last_t_cmd = t_cmd != 0;
}

pub fn last_search_pat() -> *mut c_char {
    srch().spats[srch().last_idx as usize].pat_ptr()
}

pub fn reset_search_dir() {
    srch().spats[0].off.dir = b'/' as c_char;
        set_vv_searchforward();
}

pub fn set_last_search_pat(s: *const c_char, idx: c_int, magic: c_int, setlast: bool) {
    unsafe {
        let i = idx as usize;
        free_spat(&mut srch().spats[i]);
        if *s == NUL {
            srch().spats[i].pat = None;
        } else {
            srch().spats[i].pat = Some(CStr::from_ptr(s).to_owned());
        }
        srch().spats[i].timestamp = os_time();
        srch().spats[i].additional_data = None;
        srch().spats[i].magic = magic != 0;
        srch().spats[i].no_scs = false;
        srch().spats[i].off.dir = b'/' as c_char;
        set_vv_searchforward();
        srch().spats[i].off.line = false;
        srch().spats[i].off.end = false;
        srch().spats[i].off.off = 0;
        if setlast {
            srch().last_idx = idx;
        }
        if srch().save_level != 0 {
            free_spat(&mut srch().saved_spats[i]);
            srch().saved_spats[i] = srch().spats[0].clone();
            srch().saved_spats[i].pat = srch().spats[i].pat.clone();
            srch().saved_spats_last_idx = srch().last_idx;
        }
        if opt_vars().p_hls != 0 && idx == srch().last_idx && !g_st().Search.no_hlsearch {
            redraw_all_later(UPD_SOME_VALID);
        }
    }
}

pub fn last_pat_prog(regmatch: &mut regmmatch_T) {
    if srch().spats[srch().last_idx as usize].pat.is_none() {
            regmatch.regprog = ptr::null_mut();
            return;
        }
        g_st().emsg_off += 1;
        search_regcomp(
            c"".as_ptr().cast_mut(),
            0,
            ptr::null_mut(),
            0,
            srch().last_idx,
            SEARCH_KEEP,
            opt_vars().p_magic != 0,
            regmatch,
        );
        g_st().emsg_off -= 1;
}

pub fn searchit(
    win: *mut win_T,
    buf: *mut buf_T,
    pos: *mut pos_T,
    mut end_pos: Option<&mut pos_T>,
    dir: Direction,
    pat: *mut c_char,
    patlen: usize,
    mut count: c_int,
    options: c_int,
    pat_use: c_int,
    magic: bool,
    extra_arg: *mut searchit_arg_T,
) -> c_int {
    unsafe {
        let mut found: c_int;
        let mut lnum: linenr_T;
        let mut regmatch: regmmatch_T = regmmatch_T::new();
        let mut ptr_line: *mut c_char;
        let mut matchcol: colnr_T;
        let mut endpos: lpos_T;
        let mut matchpos: lpos_T;
        let mut extra_col: c_int;
        let mut start_char_len: c_int;
        let mut match_ok: bool;
        let mut nmatched: c_int;
        let mut submatch: c_int = 0;
        let mut first_match = true;
        let called_emsg_before = g_st().called_emsg;
        let mut break_loop = false;
        let mut stop_lnum: linenr_T = 0;
        let mut tm: Option<proftime_T> = None;
        let mut timed_out: *mut c_int = ptr::null_mut();

        if !extra_arg.is_null() {
            stop_lnum = (*extra_arg).sa_stop_lnum;
            tm = (*extra_arg).sa_tm;
            timed_out = &raw mut (*extra_arg).sa_timed_out;
        }

        if search_regcomp(
            pat,
            patlen,
            ptr::null_mut(),
            RE_SEARCH,
            pat_use,
            options & (SEARCH_HIS + SEARCH_KEEP),
            magic,
            &mut regmatch,
        ) == FAIL
        {
            if (options & SEARCH_MSG) != 0 && !g_st().rc_did_emsg {
                semsg!(
                    tr(c"E383: Invalid search string: %s".as_ptr()),
                    get_search_pat()
                );
            }
            return FAIL;
        }

        let search_from_match_end = !vim_strchr(opt_vars().p_cpo, kCpoSearch).is_null();

        loop {
            if (*pos).col == MAXCOL {
                start_char_len = 0;
            } else if (*pos).lnum >= 1
                && (*pos).lnum <= (*buf).b_ml.ml_line_count
                && (*pos).col < MAXCOL - 2
            {
                ptr_line = ml_get_buf(buf, (*pos).lnum);
                if ml_get_buf_len(buf, (*pos).lnum) <= (*pos).col {
                    start_char_len = 1;
                } else {
                    start_char_len = utfc_ptr2len(ptr_line.add((*pos).col as usize));
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
            if (*pos).lnum == 0 {
                (*pos).lnum = 1;
                (*pos).col = 0;
                at_first_line = false;
            }

            if dir == BACKWARD && start_pos.col == 0 && (options & SEARCH_START) == 0 {
                lnum = (*pos).lnum - 1;
                at_first_line = false;
            } else {
                lnum = (*pos).lnum;
            }

            for loop_idx in 0..=1 {
                while lnum > 0 && lnum <= (*buf).b_ml.ml_line_count {
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
                        (*pos).col
                    } else {
                        0
                    };
                    nmatched =
                        vim_regexec_multi(&mut regmatch, win, buf, lnum, col, tm, timed_out);
                    if regmatch.regprog.is_null() {
                        break;
                    }
                    if g_st().called_emsg > called_emsg_before || (!timed_out.is_null() && *timed_out != 0)
                    {
                        break;
                    }
                    if nmatched > 0 {
                        matchpos = regmatch.startpos[0];
                        endpos = regmatch.endpos[0];
                        submatch = first_submatch(&regmatch);
                        if lnum + matchpos.lnum > (*buf).b_ml.ml_line_count {
                            ptr_line = c"".as_ptr().cast_mut();
                        } else {
                            ptr_line = ml_get_buf(buf, lnum + matchpos.lnum);
                        }

                        if dir == FORWARD && at_first_line {
                            match_ok = true;

                            while matchpos.lnum == 0
                                && (if (options & SEARCH_END) != 0 && first_match {
                                    nmatched == 1 && endpos.col - 1 < start_pos.col + extra_col
                                } else {
                                    matchpos.col
                                        - c_int::from(*ptr_line.add(matchpos.col as usize) == NUL)
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
                                        && *ptr_line.add(matchcol as usize) != NUL
                                    {
                                        matchcol += utfc_ptr2len(ptr_line.add(matchcol as usize));
                                    }
                                } else {
                                    matchcol = regmatch.rmm_matchcol;
                                    if *ptr_line.add(matchcol as usize) != NUL {
                                        matchcol += utfc_ptr2len(ptr_line.add(matchcol as usize));
                                    }
                                }
                                if matchcol == 0 && (options & SEARCH_START) != 0 {
                                    break;
                                }
                                if *ptr_line.add(matchcol as usize) == NUL || {
                                    nmatched = vim_regexec_multi(
                                        &mut regmatch,
                                        win,
                                        buf,
                                        lnum,
                                        matchcol,
                                        tm,
                                        timed_out,
                                    );
                                    nmatched == 0
                                } {
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
                                ptr_line = ml_get_buf(buf, lnum);
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
                                        && *ptr_line.add(matchcol as usize) != NUL
                                    {
                                        matchcol += utfc_ptr2len(ptr_line.add(matchcol as usize));
                                    }
                                } else {
                                    if matchpos.lnum > 0 {
                                        break;
                                    }
                                    matchcol = matchpos.col;
                                    if *ptr_line.add(matchcol as usize) != NUL {
                                        matchcol += utfc_ptr2len(ptr_line.add(matchcol as usize));
                                    }
                                }
                                if *ptr_line.add(matchcol as usize) == NUL || {
                                    nmatched = vim_regexec_multi(
                                        &mut regmatch,
                                        win,
                                        buf,
                                        lnum + matchpos.lnum,
                                        matchcol,
                                        tm,
                                        timed_out,
                                    );
                                    nmatched == 0
                                } {
                                    if tm.is_some_and(profile_passed_limit) {
                                        match_ok = false;
                                    }
                                    break;
                                }
                                if regmatch.regprog.is_null() {
                                    break;
                                }
                                ptr_line = ml_get_buf(buf, lnum + matchpos.lnum);
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
                            (*pos).lnum = lnum + endpos.lnum;
                            (*pos).col = endpos.col;
                            if endpos.col == 0 {
                                if (*pos).lnum > 1 {
                                    (*pos).lnum -= 1;
                                    (*pos).col = ml_get_buf_len(buf, (*pos).lnum);
                                }
                            } else {
                                (*pos).col -= 1;
                                if (*pos).lnum <= (*buf).b_ml.ml_line_count {
                                    ptr_line = ml_get_buf(buf, (*pos).lnum);
                                    (*pos).col -=
                                        utf_head_off(ptr_line, ptr_line.add((*pos).col as usize));
                                }
                            }
                            if let Some(end_pos) = end_pos.as_deref_mut() {
                                end_pos.lnum = lnum + matchpos.lnum;
                                end_pos.col = matchpos.col;
                            }
                        } else {
                            (*pos).lnum = lnum + matchpos.lnum;
                            (*pos).col = matchpos.col;
                            if let Some(end_pos) = end_pos.as_deref_mut() {
                                end_pos.lnum = lnum + endpos.lnum;
                                end_pos.col = endpos.col;
                            }
                        }
                        (*pos).coladd = 0;
                        if let Some(end_pos) = end_pos.as_deref_mut() {
                            end_pos.coladd = 0;
                        }
                        found = 1;
                        first_match = false;

                        g_st().Search.match_lines = endpos.lnum - matchpos.lnum;
                        g_st().Search.match_endcol = endpos.col;
                        break;
                    }
                    line_breakcheck();
                    if g_st().got_int {
                        break;
                    }

                    if (options & SEARCH_PEEK) != 0
                        && ((lnum - (*pos).lnum) & 0x3f) == 0
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

                if opt_vars().p_ws == 0
                    || stop_lnum != 0
                    || g_st().got_int
                    || g_st().called_emsg > called_emsg_before
                    || (!timed_out.is_null() && *timed_out != 0)
                    || break_loop
                    || found != 0
                    || loop_idx != 0
                {
                    break;
                }

                lnum = if dir == BACKWARD {
                    (*buf).b_ml.ml_line_count
                } else {
                    1
                };
                if !shortmess(kShmSearch)
                    && shortmess(kShmSearchcount)
                    && (options & SEARCH_MSG) != 0
                {
                    give_warning(
                        tr(if dir == BACKWARD {
                            top_bot_msg.as_ptr()
                        } else {
                            bot_top_msg.as_ptr()
                        }),
                        true,
                        false,
                    );
                }
                if !extra_arg.is_null() {
                    (*extra_arg).sa_wrapped = 1;
                }
            }
            if g_st().got_int
                || g_st().called_emsg > called_emsg_before
                || (!timed_out.is_null() && *timed_out != 0)
                || break_loop
            {
                break;
            }
            count -= 1;
            if !(count > 0 && found != 0) {
                break;
            }
        }

        vim_regfree(regmatch.regprog);

        if found == 0 {
            if g_st().got_int {
                emsg(tr(e_interr.as_ptr()));
            } else if (options & SEARCH_MSG) == SEARCH_MSG {
                if opt_vars().p_ws != 0 {
                    semsg!(tr(e_patnotf2.as_ptr()), get_search_pat());
                } else if lnum == 0 {
                    semsg!(
                        tr(E_SEARCH_HIT_TOP_WITHOUT_MATCH_FOR_STR.as_ptr()),
                        get_search_pat(),
                    );
                } else {
                    semsg!(
                        tr(E_SEARCH_HIT_BOTTOM_WITHOUT_MATCH_FOR_STR.as_ptr()),
                        get_search_pat(),
                    );
                }
            }
            return FAIL;
        }

        if (*pos).lnum > (*buf).b_ml.ml_line_count {
            (*pos).lnum = (*buf).b_ml.ml_line_count;
            (*pos).col = ml_get_buf_len(buf, (*pos).lnum);
            if (*pos).col > 0 {
                (*pos).col -= 1;
            }
        }

        submatch + 1
    }
}

pub fn set_search_direction(cdir: c_int) {
    srch().spats[0].off.dir = cdir as c_char;
}

fn set_vv_searchforward() {
    set_vim_var_nr(
            VV_SEARCHFORWARD,
            varnumber_T::from(srch().spats[0].off.dir == b'/' as c_char),
        );
}

fn first_submatch(rp: &regmmatch_T) -> c_int {
    for submatch in 1..=9 {
        if rp.startpos[submatch].lnum >= 0 {
            return submatch as c_int;
        }
    }
    0
}

pub fn parse_search_pattern_offset(
    pat: *mut *mut c_char,
    patlen: &mut usize,
    search_delim: c_int,
    options: c_int,
    strcopy: &mut Vec<c_char>,
    searchstr: *mut *mut c_char,
    searchstrlen: &mut usize,
    dircp: *mut *mut c_char,
    offset: &mut SearchOffset,
) -> c_int {
    unsafe {
        if (*pat).is_null() || **pat == NUL {
            return 0;
        }

        let mut cmdlen: c_int = 0;
        let had_copy = !strcopy.is_empty();

        *searchstr = *pat;
        *searchstrlen = *patlen;
        *dircp = ptr::null_mut();

        let mut p = skip_regexp_ex(
            *pat,
            search_delim,
            opt_vars().p_magic,
            true,
            Some(strcopy),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if !had_copy && !strcopy.is_empty() {
            let new_ptr = strcopy.as_mut_ptr();
            let len = CStr::from_ptr(new_ptr).to_bytes().len();
            cmdlen += (*patlen).wrapping_sub(len) as c_int;
            *pat = new_ptr;
            *patlen = len;
            *searchstr = new_ptr;
            *searchstrlen = len;
        }
        if c_int::from(*p as u8) == search_delim {
            *searchstrlen = p.offset_from(*pat) as usize;
            *dircp = p;
            *p = NUL;
            p = p.add(1);
        }

        offset.line = false;
        offset.end = false;
        offset.off = 0;
        if *p == b'+' as c_char || *p == b'-' as c_char || ascii_isdigit(*p) {
            offset.line = true;
        } else if (options & SEARCH_OPT) != 0
            && (*p == b'e' as c_char || *p == b's' as c_char || *p == b'b' as c_char)
        {
            if *p == b'e' as c_char {
                offset.end = true;
            }
            p = p.add(1);
        }
        if ascii_isdigit(*p) || *p == b'+' as c_char || *p == b'-' as c_char {
            if ascii_isdigit(*p) || ascii_isdigit(*p.add(1)) {
                {
                    offset.off = crate::charset::parse_i64_prefix(std::ffi::CStr::from_ptr(p).to_bytes()) as std::ffi::c_long as _;
                }
            } else if *p == b'-' as c_char {
                offset.off = -1;
            } else {
                offset.off = 1;
            }
            p = p.add(1);
            while ascii_isdigit(*p) {
                p = p.add(1);
            }
        }

        let consumed = p.offset_from(*pat) as usize;
        cmdlen += consumed as c_int;
        *patlen = (*patlen).wrapping_sub(consumed);
        *pat = p;

        cmdlen
    }
}

pub fn do_search(
    oap: *mut oparg_T,
    mut dirc: c_int,
    mut search_delim: c_int,
    mut pat: *mut c_char,
    mut patlen: usize,
    count: c_int,
    options: c_int,
    magic: bool,
    sia: *mut searchit_arg_T,
) -> c_int {
    unsafe {
        let mut searchstr: *mut c_char;
        let mut searchstrlen: usize;
        let mut retval: c_int;
        let mut c: i64;
        let mut dircp: *mut c_char;
        let mut strcopy: Vec<c_char> = Vec::new();
        let mut msgbuf_vec: Vec<c_char> = Vec::new();
        let mut msgbuflen: usize = 0;
        let mut has_offset = false;

        g_st().Search.cmdlen = 0;

        if srch().spats[0].off.line && !vim_strchr(opt_vars().p_cpo, kCpoLineoff).is_null() {
            srch().spats[0].off.line = false;
            srch().spats[0].off.off = 0;
        }

        let old_off = srch().spats[0].off;

        let mut pos = (**curwin.as_ptr()).w_cursor;

        if dirc == 0 {
            dirc = c_int::from(srch().spats[0].off.dir as u8);
        } else {
            srch().spats[0].off.dir = dirc as c_char;
            set_vv_searchforward();
        }
        if (options & SEARCH_REV) != 0 {
            dirc = if dirc == b'/' as c_int {
                b'?' as c_int
            } else {
                b'/' as c_int
            };
        }

        if dirc == b'/' as c_int {
            if hasFolding(*curwin.as_ptr(), pos.lnum, ptr::null_mut(), &raw mut pos.lnum) {
                pos.col = MAXCOL - 2;
            }
        } else if hasFolding(*curwin.as_ptr(), pos.lnum, &raw mut pos.lnum, ptr::null_mut()) {
            pos.col = 0;
        }

        if g_st().Search.no_hlsearch && (options & SEARCH_KEEP) == 0 {
            redraw_all_later(UPD_SOME_VALID);
            set_no_hlsearch(false);
        }

        let mut success = true;
        loop {
            let mut show_top_bot_msg = false;

            searchstr = pat;
            searchstrlen = patlen;

            dircp = ptr::null_mut();
            if pat.is_null() || *pat == NUL || c_int::from(*pat as u8) == search_delim {
                if srch().spats[RE_SEARCH as usize].pat.is_none() {
                    if srch().spats[RE_SUBST as usize].pat.is_none() {
                        emsg(tr(e_noprevre.as_ptr()));
                        retval = 0;
                        success = false;
                        break;
                    }
                    searchstr = srch().spats[RE_SUBST as usize].pat_ptr();
                    searchstrlen = srch().spats[RE_SUBST as usize].patlen();
                } else {
                    searchstr = c"".as_ptr().cast_mut();
                    searchstrlen = 0;
                }
            }

            if !pat.is_null() && *pat != NUL {
                let prev_searchstr = searchstr;
                let prev_searchstrlen = searchstrlen;
                g_st().Search.cmdlen += parse_search_pattern_offset(
                    &raw mut pat,
                    &mut patlen,
                    search_delim,
                    options,
                    &mut strcopy,
                    &raw mut searchstr,
                    &mut searchstrlen,
                    &raw mut dircp,
                    &mut srch().spats[0].off,
                );
                if *searchstr == NUL {
                    searchstr = prev_searchstr;
                    searchstrlen = prev_searchstrlen;
                }
            }

            let mut show_search_stats = false;
            if (options & SEARCH_ECHO) != 0
                && messaging()
                && g_st().msg_silent == 0
                && (!g_st().cmd_silent || !shortmess(kShmSearchcount))
            {
                let mut off_buf = [0 as c_char; 40];
                let off_buf_ptr = (&raw mut off_buf).cast::<c_char>();
                let mut off_len: usize = 0;

                msg_start();
                msg_ext_set_kind(c"search_cmd".as_ptr());

                if !g_st().cmd_silent && (srch().spats[0].off.line || srch().spats[0].off.end || srch().spats[0].off.off != 0) {
                    off_buf[off_len] = dirc as c_char;
                    off_len += 1;
                    if srch().spats[0].off.end {
                        off_buf[off_len] = b'e' as c_char;
                        off_len += 1;
                    } else if !srch().spats[0].off.line {
                        off_buf[off_len] = b's' as c_char;
                        off_len += 1;
                    }
                    off_buf[off_len] = NUL;
                    if srch().spats[0].off.off != 0 || srch().spats[0].off.line {
                        let n = vim_snprintf!(
                            off_buf_ptr.add(off_len),
                            off_buf.len() - off_len,
                            c"%+lld".as_ptr(),
                            srch().spats[0].off.off as std::ffi::c_longlong,
                        );
                        if n > 0 {
                            off_len += n as usize;
                        }
                    }
                }

                let (p, plen) = if *searchstr == NUL {
                    (srch().spats[0].pat_ptr(), srch().spats[0].patlen())
                } else {
                    (searchstr, searchstrlen)
                };

                let mut msgbufsize = if !shortmess(kShmSearchcount) || g_st().cmd_silent {
                    let mut sz = if ui_has(kUIMessages) {
                        0
                    } else if g_st().msg_scrolled != 0 && !g_st().cmd_silent {
                        ((g_st().Rows - g_st().msg_row) * g_st().Columns - 1) as usize
                    } else {
                        ((g_st().Rows - g_st().msg_row - 1) * g_st().Columns + g_st().sc_col - 1) as usize
                    };
                    if sz < plen + off_len + SEARCH_STAT_BUF_LEN + 3 {
                        sz = plen + off_len + SEARCH_STAT_BUF_LEN + 3;
                    }
                    sz
                } else {
                    plen + off_len + 3
                };
                let _ = &mut msgbufsize;

                msgbuf_vec = vec![b' ' as c_char; msgbufsize];
                msgbuflen = msgbufsize - 1;
                msgbuf_vec[msgbuflen] = NUL;

                if !g_st().cmd_silent {
                    let mut msgbuf = msgbuf_vec.as_mut_ptr();
                    ui_busy_start();
                    *msgbuf.add(0) = dirc as c_char;
                    if utf_iscomposing_first(utf_ptr2char(p)) {
                        *msgbuf.add(1) = b' ' as c_char;
                        ptr::copy(p, msgbuf.add(2), plen);
                    } else {
                        ptr::copy(p, msgbuf.add(1), plen);
                    }
                    if off_len > 0 {
                        ptr::copy(off_buf_ptr, msgbuf.add(plen + 1), off_len);
                    }

                    if let Some(trunc) = msg_strtrunc(msgbuf, 1) {
                        msgbuflen = trunc.as_bytes().len();
                        msgbuf_vec = trunc
                            .into_bytes_with_nul()
                            .into_iter()
                            .map(|b| b as c_char)
                            .collect();
                        msgbuf = msgbuf_vec.as_mut_ptr();
                    }

                    if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0
                        && *(**curwin.as_ptr()).w_onebuf_opt.wo_rlc == b's' as c_char
                    {
                        let rev = reverse_text(std::ffi::CStr::from_ptr(msgbuf));
                        msgbuflen = rev.as_bytes().len();
                        msgbuf_vec = rev
                            .into_bytes_with_nul()
                            .into_iter()
                            .map(|b| b as c_char)
                            .collect();
                        msgbuf = msgbuf_vec.as_mut_ptr();
                        let mut r = msgbuf;
                        while *r == b' ' as c_char {
                            r = r.add(1);
                        }
                        let pat_len = msgbuf.add(msgbuflen).offset_from(r) as usize;
                        ptr::copy(r, msgbuf, pat_len);
                        let r_off = r.offset_from(msgbuf) as usize;
                        if r_off >= pat_len {
                            ptr::write_bytes(r, b' ', pat_len);
                        } else {
                            ptr::write_bytes(msgbuf.add(pat_len), b' ', r_off);
                        }
                    }
                    msg_outtrans(msgbuf, 0, false);
                    msg_clr_eos();
                    msg_check();

                    gotocmdline(false);
                    ui_flush();
                    ui_busy_stop();
                    g_st().msg_nowait = true;
                }

                if !shortmess(kShmSearchcount) {
                    show_search_stats = true;
                }
            }

            if !srch().spats[0].off.line && srch().spats[0].off.off != 0 && pos.col < MAXCOL - 2 {
                if srch().spats[0].off.off > 0 {
                    c = srch().spats[0].off.off;
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
                    c = srch().spats[0].off.off;
                    while c != 0 {
                        if incl(&mut pos) == -1 {
                            break;
                        }
                        c += 1;
                    }
                    if c != 0 {
                        pos.lnum = (*g_st().curbuf).b_ml.ml_line_count + 1;
                        pos.col = 0;
                    }
                }
            }

            c = i64::from(searchit(
                *curwin.as_ptr(),
                g_st().curbuf,
                &raw mut pos,
                None,
                if dirc == b'/' as c_int {
                    FORWARD
                } else {
                    BACKWARD
                },
                searchstr,
                searchstrlen,
                count,
                c_int::from(srch().spats[0].off.end) * SEARCH_END
                    + (options
                        & (SEARCH_KEEP
                            + SEARCH_PEEK
                            + SEARCH_HIS
                            + SEARCH_MSG
                            + SEARCH_START
                            + if !pat.is_null() && *pat == b';' as c_char {
                                0
                            } else {
                                SEARCH_NOOF
                            })),
                RE_LAST,
                magic,
                sia,
            ));

            if !dircp.is_null() {
                *dircp = search_delim as c_char;
            }

            if !shortmess(kShmSearch) && !sia.is_null() && (*sia).sa_wrapped != 0 {
                show_top_bot_msg = true;
            }

            if c == i64::from(FAIL) {
                retval = 0;
                success = false;
                break;
            }
            if srch().spats[0].off.end && !oap.is_null() {
                (*oap).inclusive = true;
            }
            retval = 1;

            if !sia.is_null() && (*sia).sa_wrapped != 0 {
                apply_autocmds(
                    EVENT_SEARCHWRAPPED,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    ptr::null_mut(),
                );
            }

            if (options & SEARCH_NOOF) == 0 || (!pat.is_null() && *pat == b';' as c_char) {
                let org_pos = pos;

                if srch().spats[0].off.line {
                    c = i64::from(pos.lnum) + srch().spats[0].off.off;
                    if c < 1 {
                        pos.lnum = 1;
                    } else if c > i64::from((*g_st().curbuf).b_ml.ml_line_count) {
                        pos.lnum = (*g_st().curbuf).b_ml.ml_line_count;
                    } else {
                        pos.lnum = c as linenr_T;
                    }
                    pos.col = 0;

                    retval = 2;
                } else if pos.col < MAXCOL - 2 {
                    c = srch().spats[0].off.off;
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
                cmdline_search_stat(
                    dirc,
                    &mut pos,
                    &mut (**curwin.as_ptr()).w_cursor,
                    show_top_bot_msg,
                    msgbuf_vec.as_mut_ptr(),
                    msgbuflen,
                    count != 1
                        || has_offset
                        || ((opt_vars().fdo_flags & kOptFdoFlagSearch) == 0
                            && hasFolding(
                                *curwin.as_ptr(),
                                (**curwin.as_ptr()).w_cursor.lnum,
                                ptr::null_mut(),
                                ptr::null_mut(),
                            )),
                    opt_vars().p_msc as c_int,
                    SEARCH_STAT_DEF_TIMEOUT,
                );
            }

            if (options & SEARCH_OPT) == 0 || pat.is_null() || *pat != b';' as c_char {
                break;
            }

            pat = pat.add(1);
            dirc = c_int::from(*pat as u8);
            patlen -= 1;
            search_delim = dirc;
            if dirc != b'?' as c_int && dirc != b'/' as c_int {
                retval = 0;
                emsg(tr(c"E386: Expected '?' or '/'  after ';'".as_ptr()));
                success = false;
                break;
            }
            pat = pat.add(1);
            patlen -= 1;
        }

        if success {
            if (options & SEARCH_MARK) != 0 {
                setpcmark();
            }
            (**curwin.as_ptr()).w_cursor = pos;
            (**curwin.as_ptr()).w_set_curswant = true;
        }

        if (options & SEARCH_KEEP) != 0 || (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) != 0 {
            srch().spats[0].off = old_off;
        }

        retval
    }
}

pub fn search_for_exact_line(
    buf: &mut buf_T,
    pos: &mut pos_T,
    dir: Direction,
    pat: *mut c_char,
) -> c_int {
    unsafe {
        let mut start: linenr_T = 0;
        let compl_len = ins_compl_len();

        if buf.b_ml.ml_line_count == 0 {
            return FAIL;
        }
        loop {
            pos.lnum += dir;
            if pos.lnum < 1 {
                if opt_vars().p_ws != 0 {
                    pos.lnum = buf.b_ml.ml_line_count;
                    if !shortmess(kShmSearch) {
                        give_warning(tr(top_bot_msg.as_ptr()), true, false);
                    }
                } else {
                    pos.lnum = 1;
                    break;
                }
            } else if pos.lnum > buf.b_ml.ml_line_count {
                if opt_vars().p_ws != 0 {
                    pos.lnum = 1;
                    if !shortmess(kShmSearch) {
                        give_warning(tr(bot_top_msg.as_ptr()), true, false);
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
            let ptr_line = ml_get_buf(&raw mut *buf, pos.lnum);
            let p = skipwhite(ptr_line);
            pos.col = p.offset_from(ptr_line) as colnr_T;

            if compl_status_adding() && !compl_status_sol() {
                if mb_strcmp_ic(opt_vars().p_ic != 0, p, pat) == 0 {
                    return OK;
                }
            } else if *p != NUL {
                debug_assert!(compl_len >= 0);
                let eq = if opt_vars().p_ic != 0 {
                    mb_strnicmp(p, pat, compl_len as usize) == 0
                } else {
                    crate::memory::cstr_prefix_bytes(p, compl_len as usize) == crate::memory::cstr_prefix_bytes(pat, compl_len as usize)
                };
                if eq {
                    return OK;
                }
            }
        }
        FAIL
    }
}

pub fn searchc(cap: &mut cmdarg_T, mut t_cmd: bool) -> c_int {
    unsafe {
        let mut c = cap.nchar;
        let mut dir = cap.arg;
        let mut count = cap.count1;
        let mut stop = true;

        if c != c_int::from(NUL) {
            if g_st().KeyStuffed == 0 {
                srch().lastc[0] = c as u8;
                set_csearch_direction(dir);
                set_csearch_until(c_int::from(t_cmd));
                if cap.nchar_len != 0 {
                    srch().lastc_bytelen = cap.nchar_len;
                    ptr::copy_nonoverlapping(
                        cap.nchar_composing.as_ptr(),
                        (&raw mut srch().lastc_bytes).cast::<c_char>(),
                        cap.nchar_len as usize,
                    );
                } else {
                    srch().lastc_bytelen = utf_char2bytes(c, (&raw mut srch().lastc_bytes).cast::<c_char>());
                }
            }
        } else {
            if srch().lastc[0] == 0 && srch().lastc_bytelen <= 1 {
                return FAIL;
            }
            dir = if dir != 0 { -srch().lastcdir } else { srch().lastcdir };
            t_cmd = srch().last_t_cmd;
            c = c_int::from(srch().lastc[0]);

            if vim_strchr(opt_vars().p_cpo, kCpoScolon).is_null() && count == 1 && t_cmd {
                stop = false;
            }
        }

        (*cap.oap).inclusive = dir != BACKWARD;

        let p = get_cursor_line_ptr();
        let mut col = (**curwin.as_ptr()).w_cursor.col;
        let len = get_cursor_line_len();

        while count > 0 {
            count -= 1;
            loop {
                if dir > 0 {
                    col += utfc_ptr2len(p.add(col as usize));
                    if col >= len {
                        return FAIL;
                    }
                } else {
                    if col == 0 {
                        return FAIL;
                    }
                    col -= utf_head_off(p, p.add((col - 1) as usize)) + 1;
                }
                if srch().lastc_bytelen <= 1 {
                    if c_int::from(*p.add(col as usize) as u8) == c && stop {
                        break;
                    }
                } else if (crate::memory::cstr_prefix_bytes(p.add(col as usize), srch().lastc_bytelen as usize) == crate::memory::cstr_prefix_bytes((&raw const srch().lastc_bytes).cast::<c_char>(), srch().lastc_bytelen as usize))
                    && stop
                {
                    break;
                }
                stop = true;
            }
        }

        if t_cmd {
            col -= dir;
            if dir < 0 {
                col += srch().lastc_bytelen - 1;
            } else {
                col -= utf_head_off(p, p.add(col as usize));
            }
        }
        (**curwin.as_ptr()).w_cursor.col = col;

        OK
    }
}

pub fn findmatch(oap: *mut oparg_T, initc: c_int) -> *mut pos_T {
    findmatchlimit(oap, initc, 0, 0)
}

fn check_prevcol(
    linep: *mut c_char,
    mut col: c_int,
    ch: c_int,
    prevcol: Option<&mut c_int>,
) -> bool {
    unsafe {
        col -= 1;
        if col > 0 {
            col -= utf_head_off(linep, linep.add(col as usize));
        }
        if let Some(prevcol) = prevcol {
            *prevcol = col;
        }
        col >= 0 && c_int::from(*linep.add(col as usize) as u8) == ch
    }
}

fn find_rawstring_end(
    linep: *mut c_char,
    startpos: &pos_T,
    endpos: &pos_T,
) -> bool {
    unsafe {
        let mut p = linep.add((startpos.col + 1) as usize);
        while *p != NUL && *p != b'(' as c_char {
            p = p.add(1);
        }

        let delim_len = (p.offset_from(linep) as usize) - (startpos.col as usize) - 1;
        let mut delim_copy: Vec<c_char> = vec![0; delim_len + 1];
        ptr::copy_nonoverlapping(
            linep.add((startpos.col + 1) as usize),
            delim_copy.as_mut_ptr(),
            delim_len,
        );
        let mut found = false;
        for lnum in startpos.lnum..=endpos.lnum {
            let line = ml_get(lnum);
            p = line.add(if lnum == startpos.lnum {
                (startpos.col + 1) as usize
            } else {
                0
            });
            while *p != NUL {
                if lnum == endpos.lnum && (p.offset_from(line) as colnr_T) >= endpos.col {
                    break;
                }
                if *p == b')' as c_char
                    && (crate::memory::cstr_prefix_bytes(delim_copy.as_ptr(), delim_len) == crate::memory::cstr_prefix_bytes(p.add(1), delim_len))
                    && *p.add(delim_len + 1) == b'"' as c_char
                {
                    found = true;
                    break;
                }
                p = p.add(1);
            }
            if found {
                break;
            }
        }
        found
    }
}

fn find_mps_values(
    initc: &mut c_int,
    findc: &mut c_int,
    backwards: &mut bool,
    switchit: bool,
) {
    unsafe {
        let mut ptr_mps = (*g_st().curbuf).b_p_mps;

        while *ptr_mps != NUL {
            if utf_ptr2char(ptr_mps) == *initc {
                if switchit {
                    *findc = *initc;
                    *initc = utf_ptr2char(ptr_mps.add(utfc_ptr2len(ptr_mps) as usize + 1));
                    *backwards = true;
                } else {
                    *findc = utf_ptr2char(ptr_mps.add(utfc_ptr2len(ptr_mps) as usize + 1));
                    *backwards = false;
                }
                return;
            }
            let prev = ptr_mps;
            ptr_mps = ptr_mps.add(utfc_ptr2len(ptr_mps) as usize + 1);
            if utf_ptr2char(ptr_mps) == *initc {
                if switchit {
                    *findc = *initc;
                    *initc = utf_ptr2char(prev);
                    *backwards = false;
                } else {
                    *findc = utf_ptr2char(prev);
                    *backwards = true;
                }
                return;
            }
            ptr_mps = ptr_mps.add(utfc_ptr2len(ptr_mps) as usize);
            if *ptr_mps == b',' as c_char {
                ptr_mps = ptr_mps.add(1);
            }
        }
    }
}

pub fn findmatchlimit(
    oap: *mut oparg_T,
    mut initc: c_int,
    flags: c_int,
    maxtravel: i64,
) -> *mut pos_T {
    unsafe {
        let mut findc: c_int = 0;
        let mut count: c_int = 0;
        let mut backwards = false;
        let mut raw_string = false;
        let mut inquote = false;
        let mut ptr_line: *mut c_char;
        let mut hash_dir: c_int = 0;
        let mut comment_dir: c_int = 0;
        let mut traveled: i64 = 0;
        let mut ignore_cend = false;
        let mut match_escaped: c_int = 0;
        let mut comment_col: c_int = MAXCOL;
        let mut lispcomm = false;
        let lisp = (*g_st().curbuf).b_p_lisp != 0;
        let skip_comments = (flags & FM_SKIPCOMM) != 0;
        let mut in_block_comment = false;

        srch().findmatch_pos = (**curwin.as_ptr()).w_cursor;
        srch().findmatch_pos.coladd = 0;
        let mut linep = ml_get(srch().findmatch_pos.lnum);

        let cpo_match = !vim_strchr(opt_vars().p_cpo, kCpoMatch).is_null();
        let cpo_bsl = !vim_strchr(opt_vars().p_cpo, kCpoMatchbsl).is_null();

        let dir = if (flags & FM_BACKWARD) != 0 {
            BACKWARD
        } else if (flags & FM_FORWARD) != 0 {
            FORWARD
        } else {
            0
        };

        if initc == b'/' as c_int || initc == b'*' as c_int || initc == b'R' as c_int {
            comment_dir = dir;
            if initc == b'/' as c_int {
                ignore_cend = true;
            }
            backwards = dir != FORWARD;
            raw_string = initc == b'R' as c_int;
            initc = c_int::from(NUL);
        } else if initc != b'#' as c_int && initc != c_int::from(NUL) {
            find_mps_values(&mut initc, &mut findc, &mut backwards, true);
            if dir != 0 {
                backwards = dir != FORWARD;
            }
            if findc == c_int::from(NUL) {
                return ptr::null_mut();
            }
        } else {
            if initc == b'#' as c_int {
                hash_dir = dir;
            } else {
                if !cpo_match {
                    ptr_line = skipwhite(linep);
                    if *ptr_line == b'#' as c_char
                        && srch().findmatch_pos.col <= (ptr_line.offset_from(linep) as colnr_T)
                    {
                        ptr_line = skipwhite(ptr_line.add(1));
                        if (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"if"[..2])
                            || (crate::memory::cstr_prefix_bytes(ptr_line, 5) == &b"endif"[..5])
                            || (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"el"[..2])
                        {
                            hash_dir = 1;
                        }
                    } else if *linep.add(srch().findmatch_pos.col as usize) == b'/' as c_char {
                        if *linep.add((srch().findmatch_pos.col + 1) as usize) == b'*' as c_char {
                            comment_dir = FORWARD;
                            backwards = false;
                            srch().findmatch_pos.col += 1;
                        } else if srch().findmatch_pos.col > 0
                            && *linep.add((srch().findmatch_pos.col - 1) as usize) == b'*' as c_char
                        {
                            comment_dir = BACKWARD;
                            backwards = true;
                            srch().findmatch_pos.col -= 1;
                        }
                    } else if *linep.add(srch().findmatch_pos.col as usize) == b'*' as c_char {
                        if *linep.add((srch().findmatch_pos.col + 1) as usize) == b'/' as c_char {
                            comment_dir = BACKWARD;
                            backwards = true;
                        } else if srch().findmatch_pos.col > 0
                            && *linep.add((srch().findmatch_pos.col - 1) as usize) == b'/' as c_char
                        {
                            comment_dir = FORWARD;
                            backwards = false;
                        }
                    }
                }

                if hash_dir == 0 && comment_dir == 0 {
                    if *linep.add(srch().findmatch_pos.col as usize) == NUL && srch().findmatch_pos.col != 0 {
                        srch().findmatch_pos.col -= 1;
                    }
                    loop {
                        initc = utf_ptr2char(linep.add(srch().findmatch_pos.col as usize));
                        if initc == c_int::from(NUL) {
                            break;
                        }

                        find_mps_values(&mut initc, &mut findc, &mut backwards, false);
                        if findc != 0 {
                            break;
                        }
                        srch().findmatch_pos.col += utfc_ptr2len(linep.add(srch().findmatch_pos.col as usize));
                    }
                    if findc == 0 {
                        if !cpo_match && *skipwhite(linep) == b'#' as c_char {
                            hash_dir = 1;
                        } else {
                            return ptr::null_mut();
                        }
                    } else if !cpo_bsl {
                        let mut bslcnt = 0;
                        let mut col = srch().findmatch_pos.col;
                        while check_prevcol(linep, col, b'\\' as c_int, Some(&mut col)) {
                            bslcnt += 1;
                        }
                        match_escaped = bslcnt & 1;
                    }
                }
            }
            if hash_dir != 0 {
                if !oap.is_null() {
                    (*oap).motion_type = kMTLineWise;
                }
                if initc != b'#' as c_int {
                    ptr_line = skipwhite(skipwhite(linep).add(1));
                    if (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"if"[..2])
                        || (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"el"[..2])
                    {
                        hash_dir = 1;
                    } else if crate::memory::cstr_prefix_bytes(ptr_line, 5) == &b"endif"[..5] {
                        hash_dir = -1;
                    } else {
                        return ptr::null_mut();
                    }
                }
                srch().findmatch_pos.col = 0;
                while !g_st().got_int {
                    if hash_dir > 0 {
                        if srch().findmatch_pos.lnum == (*g_st().curbuf).b_ml.ml_line_count {
                            break;
                        }
                    } else if srch().findmatch_pos.lnum == 1 {
                        break;
                    }
                    srch().findmatch_pos.lnum += hash_dir;
                    linep = ml_get(srch().findmatch_pos.lnum);
                    line_breakcheck();
                    ptr_line = skipwhite(linep);
                    if *ptr_line != b'#' as c_char {
                        continue;
                    }
                    srch().findmatch_pos.col = ptr_line.offset_from(linep) as colnr_T;
                    ptr_line = skipwhite(ptr_line.add(1));
                    if hash_dir > 0 {
                        if crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"if"[..2] {
                            count += 1;
                        } else if crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"el"[..2] {
                            if count == 0 {
                                return &raw mut srch().findmatch_pos;
                            }
                        } else if crate::memory::cstr_prefix_bytes(ptr_line, 5) == &b"endif"[..5] {
                            if count == 0 {
                                return &raw mut srch().findmatch_pos;
                            }
                            count -= 1;
                        }
                    } else if crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"if"[..2] {
                        if count == 0 {
                            return &raw mut srch().findmatch_pos;
                        }
                        count -= 1;
                    } else if initc == b'#' as c_int && (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"el"[..2]) {
                        if count == 0 {
                            return &raw mut srch().findmatch_pos;
                        }
                    } else if crate::memory::cstr_prefix_bytes(ptr_line, 5) == &b"endif"[..5] {
                        count += 1;
                    }
                }
                return ptr::null_mut();
            }
        }

        if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0 && !vim_strchr(c"()[]{}<>".as_ptr(), initc).is_null() {
            backwards = !backwards;
        }

        let mut do_quotes: c_int = -1;
        let mut at_start: c_int;
        let mut start_in_quotes: TriState = kNone;
        let mut match_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };

        if (backwards && comment_dir != 0) || lisp || skip_comments {
            comment_col = check_linecomment(linep);
        }
        if lisp && comment_col != MAXCOL && srch().findmatch_pos.col > comment_col {
            lispcomm = true;
        }
        if skip_comments
            && !in_block_comment
            && comment_col != MAXCOL
            && backwards
            && srch().findmatch_pos.col > comment_col
        {
            srch().findmatch_pos.col = comment_col;
        }

        while !g_st().got_int {
            if backwards {
                if lispcomm && srch().findmatch_pos.col < comment_col {
                    break;
                }
                if srch().findmatch_pos.col == 0 {
                    if srch().findmatch_pos.lnum == 1 {
                        break;
                    }
                    srch().findmatch_pos.lnum -= 1;

                    traveled += 1;
                    if maxtravel > 0 && traveled > maxtravel {
                        break;
                    }

                    linep = ml_get(srch().findmatch_pos.lnum);
                    srch().findmatch_pos.col = ml_get_len(srch().findmatch_pos.lnum);
                    do_quotes = -1;
                    line_breakcheck();

                    if comment_dir != 0 || lisp || skip_comments {
                        comment_col = check_linecomment(linep);
                    }
                    if (lisp
                        || (skip_comments && !in_block_comment && srch().findmatch_pos.col > comment_col))
                        && comment_col != MAXCOL
                    {
                        srch().findmatch_pos.col = comment_col;
                    }
                } else {
                    srch().findmatch_pos.col -= 1;
                    srch().findmatch_pos.col -= utf_head_off(linep, linep.add(srch().findmatch_pos.col as usize));
                }
            } else if *linep.add(srch().findmatch_pos.col as usize) == NUL
                || (lisp && comment_col != MAXCOL && srch().findmatch_pos.col == comment_col)
            {
                if srch().findmatch_pos.lnum == (*g_st().curbuf).b_ml.ml_line_count || lispcomm {
                    break;
                }
                srch().findmatch_pos.lnum += 1;

                let prev_traveled = traveled;
                traveled += 1;
                if maxtravel != 0 && prev_traveled > maxtravel {
                    break;
                }

                linep = ml_get(srch().findmatch_pos.lnum);
                srch().findmatch_pos.col = 0;
                do_quotes = -1;
                line_breakcheck();
                if lisp || skip_comments {
                    comment_col = check_linecomment(linep);
                }
            } else {
                srch().findmatch_pos.col += utfc_ptr2len(linep.add(srch().findmatch_pos.col as usize));
            }

            if skip_comments && comment_dir == 0 && !inquote {
                if backwards {
                    if !in_block_comment
                        && srch().findmatch_pos.col > 0
                        && *linep.add((srch().findmatch_pos.col - 1) as usize) == b'*' as c_char
                        && *linep.add(srch().findmatch_pos.col as usize) == b'/' as c_char
                        && (comment_col == MAXCOL || srch().findmatch_pos.col < comment_col)
                    {
                        in_block_comment = true;
                    } else if in_block_comment
                        && srch().findmatch_pos.col > 0
                        && *linep.add((srch().findmatch_pos.col - 1) as usize) == b'/' as c_char
                        && *linep.add(srch().findmatch_pos.col as usize) == b'*' as c_char
                    {
                        in_block_comment = false;
                    }
                } else if !in_block_comment
                    && *linep.add(srch().findmatch_pos.col as usize) == b'/' as c_char
                    && *linep.add((srch().findmatch_pos.col + 1) as usize) == b'*' as c_char
                    && (comment_col == MAXCOL || srch().findmatch_pos.col < comment_col)
                {
                    in_block_comment = true;
                } else if in_block_comment
                    && srch().findmatch_pos.col > 0
                    && *linep.add((srch().findmatch_pos.col - 1) as usize) == b'*' as c_char
                    && *linep.add(srch().findmatch_pos.col as usize) == b'/' as c_char
                {
                    in_block_comment = false;
                }
            }

            if srch().findmatch_pos.col == 0
                && (flags & FM_BLOCKSTOP) != 0
                && (*linep == b'{' as c_char || *linep == b'}' as c_char)
            {
                if c_int::from(*linep as u8) == findc && count == 0 {
                    return &raw mut srch().findmatch_pos;
                }
                break;
            }

            if comment_dir != 0 {
                if comment_dir == FORWARD {
                    if *linep.add(srch().findmatch_pos.col as usize) == b'*' as c_char
                        && *linep.add((srch().findmatch_pos.col + 1) as usize) == b'/' as c_char
                    {
                        srch().findmatch_pos.col += 1;
                        return &raw mut srch().findmatch_pos;
                    }
                } else if srch().findmatch_pos.col == 0 {
                    continue;
                } else if raw_string {
                    if *linep.add((srch().findmatch_pos.col - 1) as usize) == b'R' as c_char
                        && *linep.add(srch().findmatch_pos.col as usize) == b'"' as c_char
                        && !vim_strchr(linep.add((srch().findmatch_pos.col + 1) as usize), b'(' as c_int)
                            .is_null()
                    {
                        if !find_rawstring_end(
                            linep,
                            &srch().findmatch_pos,
                            if count > 0 {
                                &match_pos
                            } else {
                                &(**curwin.as_ptr()).w_cursor
                            },
                        ) {
                            count += 1;
                            match_pos = srch().findmatch_pos;
                            match_pos.col -= 1;
                        }
                        linep = ml_get(srch().findmatch_pos.lnum);
                    }
                } else if *linep.add((srch().findmatch_pos.col - 1) as usize) == b'/' as c_char
                    && *linep.add(srch().findmatch_pos.col as usize) == b'*' as c_char
                    && (srch().findmatch_pos.col == 1
                        || *linep.add((srch().findmatch_pos.col - 2) as usize) != b'*' as c_char)
                    && srch().findmatch_pos.col < comment_col
                {
                    count += 1;
                    match_pos = srch().findmatch_pos;
                    match_pos.col -= 1;
                } else if *linep.add((srch().findmatch_pos.col - 1) as usize) == b'*' as c_char
                    && *linep.add(srch().findmatch_pos.col as usize) == b'/' as c_char
                {
                    if count > 0 {
                        srch().findmatch_pos = match_pos;
                    } else if srch().findmatch_pos.col > 1
                        && *linep.add((srch().findmatch_pos.col - 2) as usize) == b'/' as c_char
                        && srch().findmatch_pos.col <= comment_col
                    {
                        srch().findmatch_pos.col -= 2;
                    } else if ignore_cend {
                        continue;
                    } else {
                        return ptr::null_mut();
                    }
                    return &raw mut srch().findmatch_pos;
                }
                continue;
            }

            if cpo_match {
                do_quotes = 0;
            } else if do_quotes == -1 {
                at_start = do_quotes;
                ptr_line = linep;
                while *ptr_line != NUL {
                    if ptr_line == linep.add((srch().findmatch_pos.col + c_int::from(backwards)) as usize)
                    {
                        at_start = do_quotes & 1;
                    }
                    if *ptr_line == b'"' as c_char
                        && (ptr_line == linep
                            || *ptr_line.sub(1) != b'\'' as c_char
                            || *ptr_line.add(1) != b'\'' as c_char)
                    {
                        do_quotes += 1;
                    }
                    if *ptr_line == b'\\' as c_char && *ptr_line.add(1) != NUL {
                        ptr_line = ptr_line.add(1);
                    }
                    ptr_line = ptr_line.add(1);
                }
                do_quotes &= 1;

                if do_quotes == 0 {
                    inquote = false;
                    if ptr_line > linep && *ptr_line.sub(1) == b'\\' as c_char {
                        do_quotes = 1;
                        if start_in_quotes == kNone {
                            inquote = at_start != 0;
                            if inquote {
                                start_in_quotes = kTrue;
                            }
                        } else if backwards {
                            inquote = true;
                        }
                    }
                    if srch().findmatch_pos.lnum > 1 {
                        ptr_line = ml_get(srch().findmatch_pos.lnum - 1);
                        if *ptr_line != NUL
                            && *ptr_line.add((ml_get_len(srch().findmatch_pos.lnum - 1) - 1) as usize)
                                == b'\\' as c_char
                        {
                            do_quotes = 1;
                            if start_in_quotes == kNone {
                                inquote = at_start != 0;
                                if inquote {
                                    start_in_quotes = kTrue;
                                }
                            } else if !backwards {
                                inquote = true;
                            }
                        }
                        linep = ml_get(srch().findmatch_pos.lnum);
                    }
                }
            }
            if start_in_quotes == kNone {
                start_in_quotes = kFalse;
            }

            let c = utf_ptr2char(linep.add(srch().findmatch_pos.col as usize));
            if c == c_int::from(NUL) {
                if srch().findmatch_pos.col == 0
                    || *linep.add((srch().findmatch_pos.col - 1) as usize) != b'\\' as c_char
                {
                    inquote = false;
                    start_in_quotes = kFalse;
                }
            } else if c == b'"' as c_int {
                if do_quotes != 0 {
                    let mut col = srch().findmatch_pos.col - 1;
                    while col >= 0 {
                        if *linep.add(col as usize) != b'\\' as c_char {
                            break;
                        }
                        col -= 1;
                    }
                    if ((srch().findmatch_pos.col - 1 - col) & 1) == 0 {
                        inquote = !inquote;
                        start_in_quotes = kFalse;
                    }
                }
            } else {
                let mut skipped_single_quote = false;
                if c == b'\'' as c_int
                    && !cpo_match
                    && initc != b'\'' as c_int
                    && findc != b'\'' as c_int
                {
                    if backwards {
                        if srch().findmatch_pos.col > 1 {
                            if *linep.add((srch().findmatch_pos.col - 2) as usize) == b'\'' as c_char {
                                srch().findmatch_pos.col -= 2;
                                skipped_single_quote = true;
                            } else if *linep.add((srch().findmatch_pos.col - 2) as usize)
                                == b'\\' as c_char
                                && srch().findmatch_pos.col > 2
                                && *linep.add((srch().findmatch_pos.col - 3) as usize) == b'\'' as c_char
                            {
                                srch().findmatch_pos.col -= 3;
                                skipped_single_quote = true;
                            }
                        }
                    } else if *linep.add((srch().findmatch_pos.col + 1) as usize) != NUL {
                        if *linep.add((srch().findmatch_pos.col + 1) as usize) == b'\\' as c_char
                            && *linep.add((srch().findmatch_pos.col + 2) as usize) != NUL
                            && *linep.add((srch().findmatch_pos.col + 3) as usize) == b'\'' as c_char
                        {
                            srch().findmatch_pos.col += 3;
                            skipped_single_quote = true;
                        } else if *linep.add((srch().findmatch_pos.col + 2) as usize) == b'\'' as c_char {
                            srch().findmatch_pos.col += 2;
                            skipped_single_quote = true;
                        }
                    }
                }

                if !skipped_single_quote {
                    if (*g_st().curbuf).b_p_lisp != 0
                        && !vim_strchr(c"(){}[]".as_ptr(), c).is_null()
                        && srch().findmatch_pos.col > 1
                        && check_prevcol(linep, srch().findmatch_pos.col, b'\\' as c_int, None)
                        && check_prevcol(
                            linep,
                            srch().findmatch_pos.col - 1,
                            b'#' as c_int,
                            None,
                        )
                    {
                        continue;
                    }

                    if skip_comments
                        && (in_block_comment
                            || (comment_col != MAXCOL && srch().findmatch_pos.col >= comment_col))
                    {
                        continue;
                    }

                    if (!inquote || start_in_quotes == kTrue) && (c == initc || c == findc) {
                        let mut bslcnt = 0;

                        if !cpo_bsl {
                            let mut col = srch().findmatch_pos.col;
                            while check_prevcol(linep, col, b'\\' as c_int, Some(&mut col)) {
                                bslcnt += 1;
                            }
                        }
                        if cpo_bsl || (bslcnt & 1) == match_escaped {
                            if c == initc {
                                count += 1;
                            } else {
                                if count == 0 {
                                    return &raw mut srch().findmatch_pos;
                                }
                                count -= 1;
                            }
                        }
                    }
                }
            }
        }

        if comment_dir == BACKWARD && count > 0 {
            srch().findmatch_pos = match_pos;
            return &raw mut srch().findmatch_pos;
        }
        ptr::null_mut()
    }
}

pub fn showmatch(c: c_int) {
    unsafe {
        let mut vcol: colnr_T = 0;
        let so: *mut OptInt = if (**curwin.as_ptr()).w_onebuf_opt.wo_so >= 0 {
            &raw mut (**curwin.as_ptr()).w_onebuf_opt.wo_so
        } else {
            &raw mut opt_vars().p_so
        };
        let siso: *mut OptInt = if (**curwin.as_ptr()).w_onebuf_opt.wo_siso >= 0 {
            &raw mut (**curwin.as_ptr()).w_onebuf_opt.wo_siso
        } else {
            &raw mut opt_vars().p_siso
        };

        let mut p = (*g_st().curbuf).b_p_mps;
        while *p != NUL {
            if utf_ptr2char(p) == c && (((**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0) ^ (opt_vars().p_ri != 0)) {
                break;
            }
            p = p.add(utfc_ptr2len(p) as usize + 1);
            if utf_ptr2char(p) == c && !(((**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0) ^ (opt_vars().p_ri != 0)) {
                break;
            }
            p = p.add(utfc_ptr2len(p) as usize);
            if *p == NUL {
                return;
            }
            p = p.add(1);
        }
        if *p == NUL {
            return;
        }

        let lpos = findmatch(ptr::null_mut(), c_int::from(NUL));
        if lpos.is_null() {
            vim_beep(kOptBoFlagShowmatch);
            return;
        }

        if (*lpos).lnum < (**curwin.as_ptr()).w_topline || (*lpos).lnum >= (**curwin.as_ptr()).w_botline {
            return;
        }

        if (**curwin.as_ptr()).w_onebuf_opt.wo_wrap == 0 {
            getvcol(
                &mut **curwin.as_ptr(),
                &mut *lpos,
                ptr::null_mut(),
                &raw mut vcol,
                ptr::null_mut(),
                0,
            );
        }

        let col_visible = (**curwin.as_ptr()).w_onebuf_opt.wo_wrap != 0
            || (vcol >= (**curwin.as_ptr()).w_leftcol && vcol < (**curwin.as_ptr()).w_leftcol + (**curwin.as_ptr()).w_view_width);
        if !col_visible {
            return;
        }

        let mpos = *lpos;
        let save_cursor = (**curwin.as_ptr()).w_cursor;
        let save_so = *so;
        let save_siso = *siso;
        if g_st().dollar_vcol >= 0 && g_st().dollar_vcol == (**curwin.as_ptr()).w_virtcol {
            g_st().dollar_vcol = -1;
        }
        (**curwin.as_ptr()).w_virtcol += 1;

        let save_dollar_vcol = g_st().dollar_vcol;
        let save_state = g_st().State;
        g_st().State = MODE_SHOWMATCH;
        ui_cursor_shape();
        (**curwin.as_ptr()).w_cursor = mpos;
        *so = 0;
        *siso = 0;
        show_cursor_info_later(false);
        update_screen();
        setcursor();
        ui_flush();
        g_st().dollar_vcol = save_dollar_vcol;

        if !vim_strchr(opt_vars().p_cpo, kCpoShowmatch).is_null() {
            os_delay((opt_vars().p_mat as u64) * 100 + 8, true);
        } else if !char_avail() {
            os_delay((opt_vars().p_mat as u64) * 100 + 9, false);
        }
        (**curwin.as_ptr()).w_cursor = save_cursor;
        *so = save_so;
        *siso = save_siso;
        g_st().State = save_state;
        ui_cursor_shape();
    }
}

pub fn current_search(count: c_int, forward: bool) -> c_int {
    unsafe {
        let old_p_ws = opt_vars().p_ws;
        let save_VIsual = g_st().Visual.start;

        if g_st().Visual.active && *opt_vars().p_sel == b'e' as c_char && lt(g_st().Visual.start, (**curwin.as_ptr()).w_cursor) {
            dec_cursor();
        }

        let skip_first_backward = forward && g_st().Visual.active && lt((**curwin.as_ptr()).w_cursor, g_st().Visual.start);

        let mut pos = (**curwin.as_ptr()).w_cursor;
        let orig_pos = (**curwin.as_ptr()).w_cursor;
        if g_st().Visual.active {
            if forward {
                incl(&mut pos);
            } else {
                decl(&mut pos);
            }
        }

        let zero_width = is_zero_width(
            srch().spats[srch().last_idx as usize].pat_ptr(),
            srch().spats[srch().last_idx as usize].patlen(),
            true,
            &mut (**curwin.as_ptr()).w_cursor,
            FORWARD,
        );
        if zero_width == -1 {
            return FAIL;
        }

        let mut end_pos = pos;
        let mut result: c_int;

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
                opt_vars().p_ws = 0;
            }

            result = searchit(
                *curwin.as_ptr(),
                g_st().curbuf,
                &raw mut pos,
                Some(&mut end_pos),
                if dir { FORWARD } else { BACKWARD },
                srch().spats[srch().last_idx as usize].pat_ptr(),
                srch().spats[srch().last_idx as usize].patlen(),
                if i != 0 { count } else { 1 },
                SEARCH_KEEP | flags,
                RE_SEARCH,
                opt_vars().p_magic != 0,
                ptr::null_mut(),
            );

            opt_vars().p_ws = old_p_ws;

            if i == 1 && result == 0 {
                (**curwin.as_ptr()).w_cursor = orig_pos;
                if g_st().Visual.active {
                    g_st().Visual.start = save_VIsual;
                }
                return FAIL;
            } else if i == 0 && result == 0 {
                if forward {
                    clearpos(&mut pos);
                } else {
                    pos.lnum = (*(**curwin.as_ptr()).w_buffer).b_ml.ml_line_count;
                    pos.col = ml_get_len((*(**curwin.as_ptr()).w_buffer).b_ml.ml_line_count);
                }
            }
        }

        let start_pos = pos;

        if !g_st().Visual.active {
            g_st().Visual.start = start_pos;
        }

        (**curwin.as_ptr()).w_cursor = end_pos;
        if lt(g_st().Visual.start, end_pos) && forward {
            if skip_first_backward {
                (**curwin.as_ptr()).w_cursor = pos;
            } else {
                dec_cursor();
            }
        } else if g_st().Visual.active && lt((**curwin.as_ptr()).w_cursor, g_st().Visual.start) && forward {
            (**curwin.as_ptr()).w_cursor = pos;
        }
        g_st().Visual.active = true;
        g_st().Visual.mode = b'v' as c_int;

        if *opt_vars().p_sel == b'e' as c_char {
            if forward && ltoreq(g_st().Visual.start, (**curwin.as_ptr()).w_cursor) {
                inc_cursor();
            } else if !forward && ltoreq((**curwin.as_ptr()).w_cursor, g_st().Visual.start) {
                inc(&mut g_st().Visual.start);
            }
        }

        if (opt_vars().fdo_flags & kOptFdoFlagSearch) != 0 && g_st().KeyTyped {
            foldOpenCursor();
        }

        may_start_select(b'c' as c_int);
        setmouse();
        redraw_curbuf_later(UPD_INVERTED);
        showmode();

        OK
    }
}

fn is_zero_width(
    mut pattern: *mut c_char,
    mut patternlen: usize,
    move_to_start: bool,
    cur: &mut pos_T,
    direction: Direction,
) -> c_int {
    unsafe {
        let mut regmatch: regmmatch_T = regmmatch_T::new();
        let mut result: c_int = -1;
        let mut pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let called_emsg_before = g_st().called_emsg;
        let mut flag = 0;

        if pattern.is_null() {
            pattern = srch().spats[srch().last_idx as usize].pat_ptr();
            patternlen = srch().spats[srch().last_idx as usize].patlen();
        }

        if search_regcomp(
            pattern,
            patternlen,
            ptr::null_mut(),
            RE_SEARCH,
            RE_SEARCH,
            SEARCH_KEEP,
            opt_vars().p_magic != 0,
            &mut regmatch,
        ) == FAIL
        {
            return -1;
        }

        regmatch.startpos[0].col = -1;
        if move_to_start {
            clearpos(&mut pos);
        } else {
            pos = *cur;
            flag = SEARCH_START;
        }
        if searchit(
            *curwin.as_ptr(),
            g_st().curbuf,
            &raw mut pos,
            None,
            direction,
            pattern,
            patternlen,
            1,
            SEARCH_KEEP + flag,
            RE_SEARCH,
            opt_vars().p_magic != 0,
            ptr::null_mut(),
        ) != FAIL
        {
            let mut nmatched: c_int;
            loop {
                regmatch.startpos[0].col += 1;
                let start_col = regmatch.startpos[0].col;
                nmatched = vim_regexec_multi(
                    &mut regmatch,
                    *curwin.as_ptr(),
                    g_st().curbuf,
                    pos.lnum,
                    start_col,
                    None,
                    ptr::null_mut(),
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
                result = c_int::from(
                    nmatched != 0
                        && regmatch.startpos[0].lnum == regmatch.endpos[0].lnum
                        && regmatch.startpos[0].col == regmatch.endpos[0].col,
                );
            }
        }

        vim_regfree(regmatch.regprog);
        result
    }
}

pub fn linewhite(lnum: linenr_T) -> bool {
    unsafe {
        let p = skipwhite(ml_get(lnum));
        *p == NUL
    }
}

fn cmdline_search_stat(
    dirc: c_int,
    pos: &mut pos_T,
    cursor_pos: &mut pos_T,
    show_top_bot_msg: bool,
    msgbuf: *mut c_char,
    msgbuflen: usize,
    recompute: bool,
    maxcount: c_int,
    timeout: c_int,
) {
    unsafe {
        let mut stat = searchstat_T::default();

        update_search_stat(
            dirc,
            pos,
            cursor_pos,
            &mut stat,
            recompute,
            maxcount,
            timeout,
        );
        if stat.cur <= 0 {
            return;
        }

        let mut t = [0 as c_char; SEARCH_STAT_BUF_LEN];
        let t_ptr = (&raw mut t).cast::<c_char>();
        let mut len: usize;

        if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0 && *(**curwin.as_ptr()).w_onebuf_opt.wo_rlc == b's' as c_char {
            if stat.incomplete == 1 {
                len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[?/??]".as_ptr()) as usize;
            } else if stat.cnt > maxcount && stat.cur > maxcount {
                len = vim_snprintf!(
                    t_ptr,
                    SEARCH_STAT_BUF_LEN,
                    c"[>%d/>%d]".as_ptr(),
                    maxcount,
                    maxcount,
                ) as usize;
            } else if stat.cnt > maxcount {
                len = vim_snprintf!(
                    t_ptr,
                    SEARCH_STAT_BUF_LEN,
                    c"[>%d/%d]".as_ptr(),
                    maxcount,
                    stat.cur,
                ) as usize;
            } else {
                len = vim_snprintf!(
                    t_ptr,
                    SEARCH_STAT_BUF_LEN,
                    c"[%d/%d]".as_ptr(),
                    stat.cnt,
                    stat.cur,
                ) as usize;
            }
        } else if stat.incomplete == 1 {
            len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[?/??]".as_ptr()) as usize;
        } else if stat.cnt > maxcount && stat.cur > maxcount {
            len = vim_snprintf!(
                t_ptr,
                SEARCH_STAT_BUF_LEN,
                c"[>%d/>%d]".as_ptr(),
                maxcount,
                maxcount,
            ) as usize;
        } else if stat.cnt > maxcount {
            len = vim_snprintf!(
                t_ptr,
                SEARCH_STAT_BUF_LEN,
                c"[%d/>%d]".as_ptr(),
                stat.cur,
                maxcount,
            ) as usize;
        } else {
            len = vim_snprintf!(
                t_ptr,
                SEARCH_STAT_BUF_LEN,
                c"[%d/%d]".as_ptr(),
                stat.cur,
                stat.cnt,
            ) as usize;
        }

        if show_top_bot_msg && len + 2 < SEARCH_STAT_BUF_LEN {
            ptr::copy(t_ptr, t_ptr.add(2), len);
            *t_ptr.add(0) = b'W' as c_char;
            *t_ptr.add(1) = b' ' as c_char;
            len += 2;
        }

        if len > msgbuflen {
            len = msgbuflen;
        }
        ptr::copy(t_ptr, msgbuf.add(msgbuflen - len), len);

        g_st().msg_ext_overwrite = true;
        msg_ext_set_kind(c"search_count".as_ptr());
        give_warning(msgbuf, false, false);
    }
}

fn update_search_stat(
    dirc: c_int,
    pos: &mut pos_T,
    cursor_pos: &mut pos_T,
    stat: &mut searchstat_T,
    recompute: bool,
    maxcount: c_int,
    timeout: c_int,
) {
    unsafe {
        let save_ws = opt_vars().p_ws;
        let p = *pos;

        *stat = searchstat_T::default();

        if dirc == 0 && !recompute && !empty_pos(srch().stat_lastpos) {
            stat.cur = srch().stat_cur;
            stat.cnt = srch().stat_cnt;
            stat.exact_match = srch().stat_exact_match;
            stat.incomplete = srch().stat_incomplete;
            stat.last_maxcount = srch().stat_last_maxcount;
            return;
        }
        srch().stat_last_maxcount = maxcount;
        let wraparound = (dirc == b'?' as c_int && lt(srch().stat_lastpos, p))
            || (dirc == b'/' as c_int && lt(p, srch().stat_lastpos));

        if !(i64::from(srch().stat_chgtick) == buf_get_changedtick(&*g_st().curbuf)
            && srch().stat_lastpat.is_some()
            && srch().stat_lastpat.as_deref()
                == srch().spats[srch().last_idx as usize].pat.as_deref()
            && equalpos(srch().stat_lastpos, *cursor_pos)
            && srch().stat_lbuf == g_st().curbuf)
            || wraparound
            || srch().stat_cur < 0
            || (maxcount > 0 && srch().stat_cur > maxcount)
            || recompute
        {
            srch().stat_cur = 0;
            srch().stat_cnt = 0;
            srch().stat_exact_match = false;
            srch().stat_incomplete = 0;
            srch().stat_lastpos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            srch().stat_lbuf = g_st().curbuf;
        }

        if equalpos(srch().stat_lastpos, *cursor_pos)
            && !wraparound
            && (if dirc == 0 || dirc == b'/' as c_int {
                srch().stat_cur < srch().stat_cnt
            } else {
                srch().stat_cur > 1
            })
        {
            srch().stat_cur += if dirc == 0 {
                0
            } else if dirc == b'/' as c_int {
                1
            } else {
                -1
            };
        } else {
            let mut start: proftime_T = 0;
            let mut done_search = false;
            let mut endpos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            opt_vars().p_ws = 0;
            if timeout > 0 {
                start = profile_setlimit(i64::from(timeout));
            }
            while !g_st().got_int
                && searchit(
                    *curwin.as_ptr(),
                    g_st().curbuf,
                    &raw mut srch().stat_lastpos,
                    Some(&mut endpos),
                    FORWARD,
                    ptr::null_mut(),
                    0,
                    1,
                    SEARCH_KEEP,
                    RE_LAST,
                    opt_vars().p_magic != 0,
                    ptr::null_mut(),
                ) != FAIL
            {
                done_search = true;
                if timeout > 0 && profile_passed_limit(start) {
                    srch().stat_incomplete = 1;
                    break;
                }
                srch().stat_cnt += 1;
                if ltoreq(srch().stat_lastpos, p) {
                    srch().stat_cur = srch().stat_cnt;
                    if lt(p, endpos) {
                        srch().stat_exact_match = true;
                    }
                }
                fast_breakcheck();
                if maxcount > 0 && srch().stat_cnt > maxcount {
                    srch().stat_incomplete = 2;
                    break;
                }
            }
            if g_st().got_int {
                srch().stat_cur = -1;
            }
            if done_search {
                srch().stat_lastpat = srch().spats[srch().last_idx as usize].pat.clone();
                srch().stat_chgtick = buf_get_changedtick(&*g_st().curbuf) as c_int;
                srch().stat_lbuf = g_st().curbuf;
                srch().stat_lastpos = p;
            }
        }
        stat.cur = srch().stat_cur;
        stat.cnt = srch().stat_cnt;
        stat.exact_match = srch().stat_exact_match;
        stat.incomplete = srch().stat_incomplete;
        stat.last_maxcount = srch().stat_last_maxcount;
        opt_vars().p_ws = save_ws;
    }
}

pub fn f_searchcount(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    unsafe {
        let mut pos = (**curwin.as_ptr()).w_cursor;
        let mut pattern: *mut c_char = ptr::null_mut();
        let mut maxcount = opt_vars().p_msc as c_int;
        let mut timeout = SEARCH_STAT_DEF_TIMEOUT;
        let mut recompute = true;
        let mut stat = searchstat_T::default();

        tv_dict_alloc_ret(rettv);

        if shortmess(kShmSearchcount) {
            recompute = true;
        }

        if (*argvars.add(0)).v_type != VarType::VAR_UNKNOWN {
            let mut error = false;

            if tv_check_for_nonnull_dict_arg(argvars, 0) == FAIL {
                return;
            }
            let dict: *mut dict_T = (*argvars.add(0)).vval.v_dict();
            let mut di: *mut dictitem_T = tv_dict_find(dict, c"timeout".as_ptr(), -1);
            if !di.is_null() {
                timeout = tv_get_number_chk(&raw const (*di).di_tv, &mut error) as c_int;
                if error {
                    return;
                }
            }
            di = tv_dict_find(dict, c"maxcount".as_ptr(), -1);
            if !di.is_null() {
                maxcount = tv_get_number_chk(&raw const (*di).di_tv, &mut error) as c_int;
                if error {
                    return;
                }
            }
            recompute = tv_dict_get_bool(dict, c"recompute".as_ptr(), c_int::from(recompute)) != 0;
            di = tv_dict_find(dict, c"pattern".as_ptr(), -1);
            if !di.is_null() {
                pattern = tv_get_string_chk(&raw const (*di).di_tv).cast_mut();
                if pattern.is_null() {
                    return;
                }
            }
            di = tv_dict_find(dict, c"pos".as_ptr(), -1);
            if !di.is_null() {
                if (*di).di_tv.v_type != VarType::VAR_LIST {
                    semsg!(tr(e_invarg2.as_ptr()), c"pos".as_ptr());
                    return;
                }
                if tv_list_len((*di).di_tv.vval.v_list()) != 3 {
                    semsg!(
                        tr(e_invarg2.as_ptr()),
                        c"List format should be [lnum, col, off]".as_ptr(),
                    );
                    return;
                }
                let mut li: *mut listitem_T = tv_list_find((*di).di_tv.vval.v_list(), 0);
                if !li.is_null() {
                    pos.lnum =
                        tv_get_number_chk(&raw const (*li).li_tv, &mut error) as linenr_T;
                    if error {
                        return;
                    }
                }
                li = tv_list_find((*di).di_tv.vval.v_list(), 1);
                if !li.is_null() {
                    pos.col =
                        tv_get_number_chk(&raw const (*li).li_tv, &mut error) as colnr_T - 1;
                    if error {
                        return;
                    }
                }
                li = tv_list_find((*di).di_tv.vval.v_list(), 2);
                if !li.is_null() {
                    pos.coladd =
                        tv_get_number_chk(&raw const (*li).li_tv, &mut error) as colnr_T;
                    if error {
                        return;
                    }
                }
            }
        }

        save_last_search_pattern();
        save_incsearch_state();
        let mut do_update = true;
        if !pattern.is_null() {
            if *pattern == NUL {
                do_update = false;
            } else {
                srch().spats[srch().last_idx as usize].pat = Some(CStr::from_ptr(pattern).to_owned());
            }
        }
        if do_update
            && !srch().spats[srch().last_idx as usize].pat_ptr().is_null()
            && *srch().spats[srch().last_idx as usize].pat_ptr() != NUL
        {
            let cursor_pos = pos;
            update_search_stat(
                0,
                &mut pos,
                &mut { cursor_pos },
                &mut stat,
                recompute,
                maxcount,
                timeout,
            );

            tv_dict_add_nr(
                rettv.vval.v_dict(),
                c"current".as_ptr(),
                7,
                varnumber_T::from(stat.cur),
            );
            tv_dict_add_nr(
                rettv.vval.v_dict(),
                c"total".as_ptr(),
                5,
                varnumber_T::from(stat.cnt),
            );
            tv_dict_add_nr(
                rettv.vval.v_dict(),
                c"exact_match".as_ptr(),
                11,
                varnumber_T::from(stat.exact_match),
            );
            tv_dict_add_nr(
                rettv.vval.v_dict(),
                c"incomplete".as_ptr(),
                10,
                varnumber_T::from(stat.incomplete),
            );
            tv_dict_add_nr(
                rettv.vval.v_dict(),
                c"maxcount".as_ptr(),
                8,
                varnumber_T::from(stat.last_maxcount),
            );
        }

        restore_last_search_pattern();
        restore_incsearch_state();
    }
}

fn get_line_and_copy(lnum: linenr_T, buf: *mut c_char) -> *mut c_char {
    let line = ml_get(lnum);
        xstrlcpy(buf, line, LSIZE);
        buf
}

pub fn find_pattern_in_path(
    ptr_pat: *mut c_char,
    mut dir: Direction,
    len: usize,
    whole: bool,
    skip_comments: bool,
    type_: c_int,
    mut count: c_int,
    action: c_int,
    start_lnum: linenr_T,
    mut end_lnum: linenr_T,
    forceit: bool,
    silent: bool,
) {
    unsafe {
        let mut max_path_depth: c_int = 50;
        let mut match_count: c_int = 1;

        let mut curr_fname = (*g_st().curbuf).b_fname;
        let mut prev_fname: *mut c_char = ptr::null_mut();
        let mut p: *mut c_char;
        let mut define_matched: bool;
        let mut regmatch: regmatch_T = regmatch_T::new();
        let mut incl_regmatch: regmatch_T = regmatch_T::new();
        let mut def_regmatch: regmatch_T = regmatch_T::new();
        let mut matched = false;
        let mut did_show = false;
        let mut found = false;
        let mut i: c_int;
        let mut already: *mut c_char = ptr::null_mut();
        let mut startp: *mut c_char = ptr::null_mut();
        let mut curwin_save: *mut win_T = ptr::null_mut();
        let l_g_do_tagpreview = g_st().g_do_tagpreview;

        let mut file_line_vec: Vec<c_char> = vec![0; LSIZE];
        let file_line = file_line_vec.as_mut_ptr();

        if type_ != CHECK_PATH && type_ != FIND_DEFINE && !compl_status_sol() {
            let patsize = len + 5;
            let mut pat_vec: Vec<c_char> = vec![0; patsize];
            let pat = pat_vec.as_mut_ptr();
            debug_assert!(len <= c_int::MAX as usize);
            vim_snprintf!(
                pat,
                patsize,
                if whole {
                    c"\\<%.*s\\>".as_ptr()
                } else {
                    c"%.*s".as_ptr()
                },
                len as c_int,
                ptr_pat,
            );
            regmatch.rm_ic = ignorecase(pat) != 0;
            regmatch.regprog = vim_regcomp(pat, if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                return;
            }
        }
        let inc_opt = if *(*g_st().curbuf).b_p_inc == NUL {
            opt_vars().p_inc
        } else {
            (*g_st().curbuf).b_p_inc
        };
        if *inc_opt != NUL {
            incl_regmatch.regprog = vim_regcomp(inc_opt, if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });
            if incl_regmatch.regprog.is_null() {
                vim_regfree(regmatch.regprog);
                return;
            }
            incl_regmatch.rm_ic = false;
        }
        if type_ == FIND_DEFINE && (*(*g_st().curbuf).b_p_def != NUL || *opt_vars().p_def != NUL) {
            def_regmatch.regprog = vim_regcomp(
                if *(*g_st().curbuf).b_p_def == NUL {
                    opt_vars().p_def
                } else {
                    (*g_st().curbuf).b_p_def
                },
                if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 },
            );
            if def_regmatch.regprog.is_null() {
                vim_regfree(regmatch.regprog);
                vim_regfree(incl_regmatch.regprog);
                return;
            }
            def_regmatch.rm_ic = false;
        }
        let mut files: Vec<SearchedFile> = (0..max_path_depth as usize)
            .map(|_| SearchedFile::default())
            .collect();
        let mut old_files = max_path_depth;
        let mut depth: c_int = -1;
        let mut depth_displayed: c_int = -1;

        end_lnum = end_lnum.min((*g_st().curbuf).b_ml.ml_line_count);
        let mut lnum = start_lnum.min(end_lnum);
        let mut line = get_line_and_copy(lnum, file_line);

        loop {
            if !incl_regmatch.regprog.is_null() && vim_regexec(&mut incl_regmatch, line, 0) {
                let p_fname = if curr_fname == (*g_st().curbuf).b_fname {
                    (*g_st().curbuf).b_ffname
                } else {
                    curr_fname
                };

                let mut new_fname_cstr = if find_substr(inc_opt, b"\\zs") {
                    find_file_name_in_path(
                        incl_regmatch.startp[0],
                        incl_regmatch.endp[0].offset_from(incl_regmatch.startp[0]) as usize,
                        FNAME_EXP | FNAME_INCL | FNAME_REL,
                        1,
                        p_fname,
                    )
                } else {
                    file_name_in_line(
                        incl_regmatch.endp[0],
                        0,
                        FNAME_EXP | FNAME_INCL | FNAME_REL,
                        1,
                        p_fname,
                        ptr::null_mut(),
                    )
                }
                .map(|cstr| {
                    let mut v: Vec<c_char> = cstr
                        .into_bytes_with_nul()
                        .into_iter()
                        .map(|b| b as c_char)
                        .collect();
                    to_slash(v.as_mut_ptr());
                    CStr::from_ptr(v.as_ptr()).to_owned()
                });
                let mut already_searched = false;
                if let Some(ref nf) = new_fname_cstr {
                    let nf_ptr = nf.as_ptr().cast_mut();
                    i = 0;
                    loop {
                        if i == depth + 1 {
                            i = old_files;
                        }
                        if i == max_path_depth {
                            break;
                        }
                        if path_equal(
                            nf_ptr,
                            files[i as usize].name_ptr(),
                            kPathCmpExpand | kPathCmpFull,
                        ) {
                            if type_ != CHECK_PATH
                                && action == ACTION_SHOW_ALL
                                && files[i as usize].matched != 0
                            {
                                msg_putchar(b'\n' as c_int);
                                if !g_st().got_int {
                                    msg_home_replace(nf_ptr);
                                    msg_puts(tr(c" (includes previously listed match)".as_ptr()));
                                    prev_fname = ptr::null_mut();
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
                        msg_putchar(b'\n' as c_int);
                    } else {
                        gotocmdline(true);
                        msg_puts_title(tr(c"--- Included files ".as_ptr()));
                        if action != ACTION_SHOW_ALL {
                            msg_puts_title(tr(c"not found ".as_ptr()));
                        }
                        msg_puts_title(tr(c"in path ---\n".as_ptr()));
                    }
                    did_show = true;
                    while depth_displayed < depth && !g_st().got_int {
                        depth_displayed += 1;
                        for _ in 0..depth_displayed {
                            msg_puts(c"  ".as_ptr());
                        }
                        msg_home_replace(files[depth_displayed as usize].name_ptr());
                        msg_puts(c" -->\n".as_ptr());
                    }
                    if !g_st().got_int {
                        for _ in 0..=depth_displayed {
                            msg_puts(c"  ".as_ptr());
                        }
                        if let Some(ref nf) = new_fname_cstr {
                            msg_outtrans(nf.as_ptr(), HLF_D, false);
                        } else {
                            if find_substr(inc_opt, b"\\zs") {
                                p = incl_regmatch.startp[0];
                                i = incl_regmatch.endp[0].offset_from(incl_regmatch.startp[0])
                                    as c_int;
                            } else {
                                p = incl_regmatch.endp[0];
                                while *p != NUL && !vim_isfilec(c_int::from(*p as u8)) {
                                    p = p.add(1);
                                }
                                i = 0;
                                while vim_isfilec(c_int::from(*p.add(i as usize) as u8)) {
                                    i += 1;
                                }
                            }

                            if i == 0 {
                                p = incl_regmatch.endp[0];
                                i = CStr::from_ptr(p).to_bytes().len() as c_int;
                            } else if p > line {
                                if *p.sub(1) == b'"' as c_char || *p.sub(1) == b'<' as c_char {
                                    p = p.sub(1);
                                    i += 1;
                                }
                                if *p.add(i as usize) == b'"' as c_char
                                    || *p.add(i as usize) == b'>' as c_char
                                {
                                    i += 1;
                                }
                            }
                            let save_char = *p.add(i as usize);
                            *p.add(i as usize) = NUL;
                            msg_outtrans(p, HLF_D, false);
                            *p.add(i as usize) = save_char;
                        }

                        if new_fname_cstr.is_none() && action == ACTION_SHOW_ALL {
                            if already_searched {
                                msg_puts(tr(c"  (Already listed)".as_ptr()));
                            } else {
                                msg_puts(tr(c"  NOT FOUND".as_ptr()));
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
                    let fp = crate::os::fs::CFile::from_raw(
                        os_fopen(nf_cstr.as_ptr(), c"r".as_ptr()).cast::<FILE>(),
                    );
                    let opened = fp.is_some();
                    files[(depth + 1) as usize].fp = fp;
                    if opened {
                        depth += 1;
                        if depth == old_files {
                            files[old_files as usize].name = None;
                            old_files += 1;
                        }
                        files[depth as usize].name = Some(nf_cstr);
                        curr_fname = files[depth as usize].name_ptr();
                        files[depth as usize].lnum = 0;
                        files[depth as usize].matched = 0;
                        if action == ACTION_EXPAND && !shortmess(kShmCompletionscan) && !silent {
                            let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                            g_st().msg_hist_off = true;
                            vim_snprintf!(
                                iobuff_ptr,
                                IOSIZE,
                                tr(c"Scanning included file: %s".as_ptr()),
                                curr_fname,
                            );
                            msg_trunc(iobuff_ptr, true, HLF_R);
                        } else if opt_vars().p_verbose >= 5 {
                            verbose_enter();
                            smsg!(0, tr(c"Searching included file %s".as_ptr()), curr_fname);
                            verbose_leave();
                        }
                    }
                }
            } else {
                p = line;
                loop {
                    define_matched = false;
                    if !def_regmatch.regprog.is_null()
                        && vim_regexec(&mut def_regmatch, line, 0)
                    {
                        p = def_regmatch.endp[0];
                        while *p != NUL && !vim_iswordc(c_int::from(*p as u8)) {
                            p = p.add(1);
                        }
                        define_matched = true;
                    }

                    if def_regmatch.regprog.is_null() || define_matched {
                        if define_matched || compl_status_sol() {
                            startp = skipwhite(p);
                            if opt_vars().p_ic != 0 {
                                matched = mb_strnicmp(startp, ptr_pat, len) == 0;
                            } else {
                                matched = crate::memory::cstr_prefix_bytes(startp, len) == crate::memory::cstr_prefix_bytes(ptr_pat, len);
                            }
                            if matched
                                && define_matched
                                && whole
                                && vim_iswordc(c_int::from(*startp.add(len) as u8))
                            {
                                matched = false;
                            }
                        } else if !regmatch.regprog.is_null()
                            && vim_regexec(&mut regmatch, line, p.offset_from(line) as colnr_T)
                        {
                            matched = true;
                            startp = regmatch.startp[0];
                            if skip_comments {
                                if (*line != b'#' as c_char
                                    || !(crate::memory::cstr_prefix_bytes(skipwhite(line.add(1)), 6) == &b"define"[..6]))
                                    && get_leader_len(line, ptr::null_mut(), false, true) != 0
                                {
                                    matched = false;
                                }

                                p = skipwhite(line);
                                if matched
                                    || (*p == b'/' as c_char && *p.add(1) == b'*' as c_char)
                                    || *p == b'*' as c_char
                                {
                                    p = line;
                                    while *p != NUL && p < startp {
                                        if matched
                                            && *p == b'/' as c_char
                                            && (*p.add(1) == b'*' as c_char
                                                || *p.add(1) == b'/' as c_char)
                                        {
                                            matched = false;
                                            if *p.add(1) == b'/' as c_char {
                                                break;
                                            }
                                            p = p.add(1);
                                        } else if !matched
                                            && *p == b'*' as c_char
                                            && *p.add(1) == b'/' as c_char
                                        {
                                            matched = true;
                                            p = p.add(1);
                                        }
                                        p = p.add(1);
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

                        if depth == -1 && lnum == (**curwin.as_ptr()).w_cursor.lnum {
                            stop_fpip_loop = true;
                        } else {
                            found = true;
                            let mut aux = startp;
                            p = startp;
                            let mut skip_add = false;
                            if compl_status_adding()
                                && (CStr::from_ptr(p).to_bytes().len() as c_int) >= ins_compl_len()
                            {
                                p = p.add(ins_compl_len() as usize);
                                if vim_iswordp(p) {
                                    skip_add = true;
                                } else {
                                    p = find_word_start(p);
                                }
                            }
                            if !skip_add {
                                p = find_word_end(p);
                                i = p.offset_from(aux) as c_int;

                                if compl_status_adding() && i == ins_compl_len() {
                                    let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                                    ptr::copy_nonoverlapping(aux, iobuff_ptr, i as usize);

                                    if depth < 0 {
                                        if lnum >= end_lnum {
                                            skip_add = true;
                                        } else {
                                            lnum += 1;
                                            line = get_line_and_copy(lnum, file_line);
                                        }
                                    } else {
                                        line = file_line;
                                        if vim_fgets(
                                            line,
                                            LSIZE as c_int,
                                            files[depth as usize].fp_ptr(),
                                        ) {
                                            skip_add = true;
                                        }
                                    }

                                    if !skip_add {
                                        p = skipwhite(line);
                                        aux = p;
                                        already = p;
                                        p = find_word_start(p);
                                        p = find_word_end(p);
                                        if p > aux {
                                            if *aux != b')' as c_char
                                                && g_st().IObuff[(i - 1) as usize] != TAB
                                            {
                                                if g_st().IObuff[(i - 1) as usize] != b' ' as c_char {
                                                    g_st().IObuff[i as usize] = b' ' as c_char;
                                                    i += 1;
                                                }
                                                if opt_vars().p_js != 0
                                                    && (g_st().IObuff[(i - 2) as usize] == b'.' as c_char
                                                        || g_st().IObuff[(i - 2) as usize]
                                                            == b'?' as c_char
                                                        || g_st().IObuff[(i - 2) as usize]
                                                            == b'!' as c_char)
                                                {
                                                    g_st().IObuff[i as usize] = b' ' as c_char;
                                                    i += 1;
                                                }
                                            }
                                            let mut copy_len = p.offset_from(aux) as usize;
                                            if copy_len >= IOSIZE - (i as usize) {
                                                copy_len = IOSIZE - (i as usize) - 1;
                                            }
                                            ptr::copy_nonoverlapping(
                                                aux,
                                                iobuff_ptr.add(i as usize),
                                                copy_len,
                                            );
                                            i += copy_len as c_int;
                                            cont_s_ipos = true;
                                        }
                                        g_st().IObuff[i as usize] = NUL;
                                        aux = iobuff_ptr;

                                        if i == ins_compl_len() {
                                            skip_add = true;
                                        }
                                    }
                                }

                                if !skip_add {
                                    let add_r = ins_compl_add_infercase(
                                        aux,
                                        i,
                                        opt_vars().p_ic != 0,
                                        if curr_fname == (*g_st().curbuf).b_fname {
                                            ptr::null_mut()
                                        } else {
                                            path_try_shorten_fname(curr_fname)
                                        },
                                        dir,
                                        cont_s_ipos,
                                        0,
                                    );
                                    if add_r == OK {
                                        dir = FORWARD;
                                    } else if add_r == FAIL {
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
                                msg_putchar(b'\n' as c_int);
                            }
                            if !g_st().got_int {
                                msg_home_replace(curr_fname);
                            }
                            prev_fname = curr_fname;
                        }
                        did_show = true;
                        if !g_st().got_int {
                            show_pat_in_path(
                                line,
                                type_,
                                true,
                                action,
                                if depth == -1 {
                                    ptr::null_mut()
                                } else {
                                    files[depth as usize].fp_ptr()
                                },
                                if depth == -1 {
                                    &raw mut lnum
                                } else {
                                    &raw mut files[depth as usize].lnum
                                },
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
                                && lnum == (**curwin.as_ptr()).w_cursor.lnum
                                && l_g_do_tagpreview == 0
                            {
                                emsg(tr(c"E387: Match is on current line".as_ptr()));
                            } else if action == ACTION_SHOW {
                                show_pat_in_path(
                                    line,
                                    type_,
                                    did_show,
                                    action,
                                    if depth == -1 {
                                        ptr::null_mut()
                                    } else {
                                        files[depth as usize].fp_ptr()
                                    },
                                    if depth == -1 {
                                        &raw mut lnum
                                    } else {
                                        &raw mut files[depth as usize].lnum
                                    },
                                    1,
                                );
                                did_show = true;
                            } else {
                                if l_g_do_tagpreview != 0 {
                                    curwin_save = *curwin.as_ptr();
                                    prepare_tagpreview(true, *opt_vars().p_pvp != NUL);
                                }
                                if action == ACTION_SPLIT {
                                    if win_split(0, 0) == FAIL {
                                        stop_fpip_loop = true;
                                    } else {
                                        reset_binding(&mut **curwin.as_ptr());
                                    }
                                }
                                if !stop_fpip_loop {
                                    if depth == -1 {
                                        if l_g_do_tagpreview != 0 {
                                            if !win_valid(curwin_save)
                                                || !getfile_success(getfile(
                                                    (*(*curwin_save).w_buffer).b_fnum,
                                                    ptr::null_mut(),
                                                    ptr::null_mut(),
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
                                            (**curwin.as_ptr()).w_cursor.lnum = lnum;
                                            check_cursor(*curwin.as_ptr());
                                        }
                                    } else if !getfile_success(getfile(
                                        0,
                                        files[depth as usize].name_ptr(),
                                        ptr::null_mut(),
                                        true,
                                        files[depth as usize].lnum,
                                        forceit,
                                    )) {
                                        stop_fpip_loop = true;
                                    } else {
                                        (**curwin.as_ptr()).w_cursor.lnum = files[depth as usize].lnum;
                                    }
                                }
                            }
                            if !stop_fpip_loop {
                                if action != ACTION_SHOW {
                                    (**curwin.as_ptr()).w_cursor.col = startp.offset_from(line) as colnr_T;
                                    (**curwin.as_ptr()).w_set_curswant = true;
                                }

                                if l_g_do_tagpreview != 0
                                    && *curwin.as_ptr() != curwin_save
                                    && win_valid(curwin_save)
                                {
                                    validate_cursor(*curwin.as_ptr());
                                    redraw_later(*curwin.as_ptr(), UPD_VALID);
                                    win_enter(curwin_save, true);
                                }
                                stop_fpip_loop = true;
                            }
                        }
                    }

                    matched = false;
                    if stop_fpip_loop {
                        already = ptr::null_mut();
                        // break outer loop too
                        depth_displayed = c_int::MIN;
                        break;
                    }
                    if def_regmatch.regprog.is_null()
                        && action == ACTION_EXPAND
                        && !compl_status_sol()
                        && *startp != NUL
                        && *startp.add(utfc_ptr2len(startp) as usize) != NUL
                    {
                        p = startp.add(utfc_ptr2len(startp) as usize);
                        continue;
                    }
                    break;
                }
                if depth_displayed == c_int::MIN {
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

            while depth >= 0 && already.is_null() && {
                line = file_line;
                vim_fgets(line, LSIZE as c_int, files[depth as usize].fp_ptr())
            } {
                files[depth as usize].fp = None;
                old_files -= 1;
                files[old_files as usize].name = files[depth as usize].name.take();
                files[old_files as usize].matched = files[depth as usize].matched;
                depth -= 1;
                curr_fname = if depth == -1 {
                    (*g_st().curbuf).b_fname
                } else {
                    files[depth as usize].name_ptr()
                };
                depth_displayed = depth_displayed.min(depth);
            }
            if depth >= 0 {
                files[depth as usize].lnum += 1;
                i = CStr::from_ptr(line).to_bytes().len() as c_int;
                if i > 0 && *line.add((i - 1) as usize) == b'\n' as c_char {
                    i -= 1;
                    *line.add(i as usize) = NUL;
                }
                if i > 0 && *line.add((i - 1) as usize) == b'\r' as c_char {
                    i -= 1;
                    *line.add(i as usize) = NUL;
                }
            } else if already.is_null() {
                lnum += 1;
                if lnum > end_lnum {
                    break;
                }
                line = get_line_and_copy(lnum, file_line);
            }
            already = ptr::null_mut();
        }

        for idx in 0..=depth {
            files[idx as usize].fp = None;
        }

        if type_ == CHECK_PATH {
            if !did_show {
                if action != ACTION_SHOW_ALL {
                    msg(tr(c"All included files were found".as_ptr()), 0);
                } else {
                    msg(tr(c"No included files".as_ptr()), 0);
                }
            }
        } else if !found && action != ACTION_EXPAND && !silent {
            if g_st().got_int || ins_compl_interrupted() {
                emsg(tr(e_interr.as_ptr()));
            } else if type_ == FIND_DEFINE {
                emsg(tr(c"E388: Couldn't find definition".as_ptr()));
            } else {
                emsg(tr(c"E389: Couldn't find pattern".as_ptr()));
            }
        }
        if action == ACTION_SHOW || action == ACTION_SHOW_ALL {
            msg_end();
        }

        vim_regfree(regmatch.regprog);
        vim_regfree(incl_regmatch.regprog);
        vim_regfree(def_regmatch.regprog);
    }
}

fn show_pat_in_path(
    mut line: *mut c_char,
    type_: c_int,
    did_show: bool,
    action: c_int,
    fp: *mut FILE,
    lnum: *mut linenr_T,
    count: c_int,
) {
    unsafe {
        if did_show {
            msg_putchar(b'\n' as c_int);
        } else if g_st().msg_silent == 0 {
            gotocmdline(true);
        }
        if g_st().got_int {
            return;
        }
        let mut linelen = CStr::from_ptr(line).to_bytes().len();
        loop {
            let mut p = if linelen > 0 {
                line.add(linelen - 1)
            } else {
                line.sub(1)
            };
            if !fp.is_null() {
                if p >= line && *p == b'\n' as c_char {
                    p = p.sub(1);
                }
                if p >= line && *p == b'\r' as c_char {
                    p = p.sub(1);
                }
                *p.add(1) = NUL;
            }
            if action == ACTION_SHOW_ALL {
                let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                vim_snprintf!(iobuff_ptr, IOSIZE, c"%3d: ".as_ptr(), count);
                msg_puts(iobuff_ptr);
                vim_snprintf!(iobuff_ptr, IOSIZE, c"%4d".as_ptr(), *lnum);
                msg_puts_hl(iobuff_ptr, HLF_N, false);
                msg_puts(c" ".as_ptr());
            }
            msg_prt_line(line, false);

            if g_st().got_int || type_ != FIND_DEFINE || p < line || *p != b'\\' as c_char {
                break;
            }

            if !fp.is_null() {
                if vim_fgets(line, LSIZE as c_int, fp) {
                    break;
                }
                linelen = CStr::from_ptr(line).to_bytes().len();
                *lnum += 1;
            } else {
                *lnum += 1;
                if *lnum > (*g_st().curbuf).b_ml.ml_line_count {
                    break;
                }
                line = ml_get(*lnum);
                linelen = ml_get_len(*lnum) as usize;
            }
            msg_putchar(b'\n' as c_int);
        }
    }
}

pub fn get_search_pattern(pat: &mut SearchPattern) {
    *pat = srch().spats[0].clone();
}

pub fn get_substitute_pattern(pat: &mut SearchPattern) {
    *pat = srch().spats[1].clone();
        pat.off = SearchOffset {
            dir: 0,
            line: false,
            end: false,
            off: 0,
        };
}

pub fn get_search_pattern_ptr(substitute: bool) -> *const c_char {
    srch().spats[if substitute {
            RE_SUBST as usize
        } else {
            RE_SEARCH as usize
        }]
        .pat_ptr()
}

pub fn get_search_pattern_timestamp(substitute: bool) -> Timestamp {
    srch().spats[if substitute {
            RE_SUBST as usize
        } else {
            RE_SEARCH as usize
        }]
        .timestamp
}

pub fn search_pattern_cleared(substitute: bool) -> bool {
    srch().spats[if substitute {
            RE_SUBST as usize
        } else {
            RE_SEARCH as usize
        }]
        .pat
        .is_none()
}

pub fn set_search_pattern(pat: SearchPattern) {
    free_spat(&mut srch().spats[0]);
        srch().spats[0] = pat;
        set_vv_searchforward();
}

pub fn set_substitute_pattern(pat: SearchPattern) {
    free_spat(&mut srch().spats[1]);
        srch().spats[1] = pat;
        srch().spats[1].off = SearchOffset {
            dir: 0,
            line: false,
            end: false,
            off: 0,
        };
}

pub fn set_last_used_pattern(is_substitute_pattern: bool) {
    srch().last_idx = c_int::from(is_substitute_pattern);
}

pub fn search_was_last_used() -> bool {
    srch().last_idx == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_submatch_and_pos_helpers() {
        let mut rm: regmmatch_T = regmmatch_T::new();
        for i in 0..10 {
            rm.startpos[i].lnum = -1;
        }
        assert_eq!(first_submatch(&rm), 0);
        rm.startpos[3].lnum = 0;
        assert_eq!(first_submatch(&rm), 3);

        let mut p = pos_T {
            lnum: 2,
            col: 5,
            coladd: 1,
        };
        assert!(!empty_pos(p));
        clearpos(&mut p);
        assert!(empty_pos(p));
        assert!(getfile_success(0));
        assert!(getfile_success(-1));
        assert!(!getfile_success(1));
    }

    #[test]
    fn pat_has_uppercase_handles_utf8_and_escapes() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let save_magic = opt_vars().p_magic;
            let save_cmp = opt_vars().cmp_flags;
            opt_vars().p_magic = 1;
            opt_vars().cmp_flags = crate::mbyte::kOptCmpFlagInternal | crate::mbyte::kOptCmpFlagKeepascii;
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
                assert_eq!(pat_has_uppercase(pat.as_ptr().cast_mut()), expected);
            }
            opt_vars().p_magic = save_magic;
            opt_vars().cmp_flags = save_cmp;
    }
}
