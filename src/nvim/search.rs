//! Code for normal mode searching commands (`src/nvim/search.c`).

use crate::autocmd::{EVENT_SEARCHWRAPPED, apply_autocmds};
use crate::buffer::{FileBuf, K_UIMESSAGES, buf_get_changedtick};
use crate::change::{TAB, get_leader_len};
use crate::charset::{NUL, ascii_isdigit, skipwhite, vim_isfilec, vim_iswordc, vim_iswordp};
use crate::cmdhist::{HIST_SEARCH, add_to_history};
use crate::cursor::{
    Colnr, CursorPos, Linenr, Lpos, check_cursor, clearpos, dec_cursor, equalpos,
    get_cursor_line_len, get_cursor_line_ptr, inc_cursor, lt, ltoreq,
};
use crate::drawscreen::{
    UPD_INVERTED, UPD_SOME_VALID, UPD_VALID, redraw_all_later, redraw_curbuf_later, redraw_later,
    setcursor, show_cursor_info_later, showmode, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    Dictitem, EvalFuncData, Listitem, TvDict, TypVal, VarType, Varnumber,
    tv_check_for_nonnull_dict_arg, tv_dict_add_nr, tv_dict_alloc_ret, tv_dict_find,
    tv_dict_get_bool, tv_get_number_chk, tv_get_string_chk, tv_list_find, tv_list_len,
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
use crate::fileio::{FILE, vim_fgets};
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
    FM_FORWARD, K_CPO_MATCH, K_OPT_FDO_FLAG_SEARCH, K_SHM_SEARCHCOUNT, SEARCH_ECHO, SEARCH_MARK,
    SEARCH_REV, may_start_select,
};
use crate::option::{OptInt, shortmess};
use crate::option_vars::*;
use crate::os::fs::os_fopen;
use crate::os::input::{fast_breakcheck, line_breakcheck};
use crate::os::time::{Timestamp, os_delay, os_time};
use crate::path::{
    K_PATH_CMP_EXPAND, K_PATH_CMP_FULL, path_equal, path_try_shorten_fname, to_slash,
};
use crate::plines::getvcol;
use crate::profile::{Proftime, profile_passed_limit, profile_setlimit};
use crate::regexp::{
    MAGIC_ALL, MAGIC_ON, MAXCOL, RE_MAGIC, RegMagic, Regmatch, Regmmatch, skip_regexp_ex,
    vim_regcomp, vim_regexec, vim_regexec_multi, vim_regfree,
};
use crate::state::MODE_SHOWMATCH;
use crate::strings::{reverse_text, tr, vim_snprintf, vim_strchr};
use crate::textformat::Oparg;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_cursor_shape, ui_flush, ui_has, vim_beep};
use crate::window::{WinInfoInfo, reset_binding, win_enter, win_split, win_valid};
use std::ffi::{CStr, CString, c_char};
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
    fp: Option<crate::os::fs::CFile>,
    name: Option<CString>,
    lnum: Linenr,
    matched: i32,
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
    last_idx: i32,
    lastc: [u8; 2],
    lastcdir: Direction,
    last_t_cmd: bool,
    lastc_bytes: [c_char; MAX_SCHAR_SIZE + 1],
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
    findmatch_pos: CursorPos,
    stat_lastpos: CursorPos,
    stat_cur: i32,
    stat_cnt: i32,
    stat_exact_match: bool,
    stat_incomplete: i32,
    stat_last_maxcount: i32,
    stat_chgtick: i32,
    stat_lastpat: Option<CString>,
    stat_lbuf: *mut FileBuf,
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
            findmatch_pos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
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
const fn empty_pos(a: CursorPos) -> bool {
    a.lnum == 0 && a.col == 0 && a.coladd == 0
}

#[inline]
const fn getfile_success(x: i32) -> bool {
    x <= 0
}

fn find_substr(haystack: impl crate::strings::IntoCCharPtr, needle: &[u8]) -> bool {
    let haystack = haystack.into_c_char_ptr();
    {
        let bytes = crate::memory::cstr_as_bytes(haystack);
        bytes.windows(needle.len()).any(|w| w == needle)
    }
}

/// Translate search pattern for `vim_regcomp()`.
pub fn search_regcomp(
    pat: impl crate::strings::IntoCCharPtr,
    mut patlen: usize,
    used_pat: *mut *mut c_char,
    pat_save: i32,
    pat_use: i32,
    options: i32,
    mut magic: bool,
    regmatch: &mut Regmmatch,
) -> bool {
    let mut pat = pat.into_c_char_ptr().cast_mut();
    unsafe {
        g_st().rc_did_emsg = false;

        if crate::memory::cstr_is_empty(pat) {
            let i = if pat_use == RE_LAST {
                srch().last_idx
            } else {
                pat_use
            };
            if srch().spats[i as usize].pat.is_none() {
                if pat_use == RE_SUBST {
                    emsg(tr(E_NOPRESUB));
                } else {
                    emsg(tr(E_NOPREVRE));
                }
                g_st().rc_did_emsg = true;
                return false;
            }
            pat = srch().spats[i as usize].pat_ptr();
            patlen = srch().spats[i as usize].patlen();
            magic = srch().spats[i as usize].magic;
            g_st().search_global.no_smartcase = srch().spats[i as usize].no_scs;
        } else if (options & SEARCH_HIS) != 0 {
            add_to_history(HIST_SEARCH, pat, patlen, true, i32::from(NUL));
        }

        if !used_pat.is_null() {
            *used_pat = pat;
        }

        srch().mr_pattern = if curwin_mut().w_onebuf_opt.wo_rl != 0
            && crate::memory::cstr_first_byte(curwin_mut().w_onebuf_opt.wo_rlc) == b's'
        {
            Some(reverse_text(pat))
        } else {
            Some(CString::new(crate::memory::cstr_slice(pat, patlen)).unwrap_or_default())
        };

        if (options & SEARCH_KEEP) == 0 && (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) == 0 {
            if pat_save == RE_SEARCH || pat_save == RE_BOTH {
                save_re_pat(RE_SEARCH, pat, patlen, i32::from(magic));
            }
            if pat_save == RE_SUBST || pat_save == RE_BOTH {
                save_re_pat(RE_SUBST, pat, patlen, i32::from(magic));
            }
        }

        regmatch.rmm_ic = ignorecase(pat);
        regmatch.rmm_maxcol = 0;
        regmatch.regprog = vim_regcomp(pat, if magic { RE_MAGIC } else { 0 });
        if regmatch.regprog.is_null() {
            return false;
        }
        true
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

pub fn save_re_pat(idx: i32, pat: *mut c_char, patlen: usize, magic: i32) {
    {
        let i = idx as usize;
        if srch().spats[i].pat_ptr() == pat {
            return;
        }

        let new_pat = CString::new(crate::memory::cstr_slice(pat, patlen)).unwrap_or_default();
        free_spat(&mut srch().spats[i]);
        srch().spats[i].pat = Some(new_pat);
        srch().spats[i].magic = magic != 0;
        srch().spats[i].no_scs = g_st().search_global.no_smartcase;
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
    srch().saved_spats_no_hlsearch = g_st().search_global.no_hlsearch;
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
    srch().saved_no_hlsearch = g_st().search_global.no_hlsearch;
}

pub fn restore_last_search_pattern() {
    srch().did_save_last_search_spat -= 1;
    if srch().did_save_last_search_spat > 0 {
        return;
    }
    if srch().did_save_last_search_spat != 0 {
        iemsg(c"restore_last_search_pattern() called more often than save_last_search_pattern()");
        return;
    }

    srch().spats[RE_SEARCH as usize] =
        std::mem::replace(&mut srch().saved_last_search_spat, EMPTY_SEARCH_PATTERN);
    set_vv_searchforward();
    srch().last_idx = srch().saved_last_idx;
    set_no_hlsearch(srch().saved_no_hlsearch);
}

fn save_incsearch_state() {
    srch().saved_search_match_endcol = g_st().search_global.match_endcol;
    srch().saved_search_match_lines = g_st().search_global.match_lines;
}

fn restore_incsearch_state() {
    g_st().search_global.match_endcol = srch().saved_search_match_endcol;
    g_st().search_global.match_lines = srch().saved_search_match_lines;
}

pub fn ignorecase(pat: *mut c_char) -> i32 {
    ignorecase_opt(pat, opt_vars().p_ic, opt_vars().p_scs)
}

pub fn ignorecase_opt(pat: *mut c_char, ic_in: i32, scs: i32) -> i32 {
    {
        let mut ic = ic_in;
        if ic != 0
            && !g_st().search_global.no_smartcase
            && scs != 0
            && !(ctrl_x_mode_not_default() && curbuf_mut().b_p_inf != 0)
        {
            ic = i32::from(!pat_has_uppercase(pat));
        }
        g_st().search_global.no_smartcase = false;
        ic
    }
}

pub fn pat_has_uppercase(pat: *mut c_char) -> bool {
    {
        let mut p = pat;
        let mut magic_val: RegMagic = MAGIC_ON;

        skip_regexp_ex(
            pat,
            i32::from(NUL),
            opt_vars().p_magic,
            false,
            None,
            None,
            Some(&mut magic_val),
        );

        while !crate::memory::cstr_is_empty(p) {
            let l = utfc_ptr2len(p);

            if l > 1 {
                if mb_isupper(utf_ptr2char(p)) {
                    return true;
                }
                p = p.wrapping_add(l as usize);
            } else if crate::memory::cstr_first_byte(p) == b'\\' && magic_val <= MAGIC_ON {
                if (crate::memory::cstr_byte_at(p, 1) == b'_'
                    || crate::memory::cstr_byte_at(p, 1) == b'%')
                    && crate::memory::cstr_byte_at(p, 2) != 0
                {
                    p = p.wrapping_add(3);
                } else if crate::memory::cstr_byte_at(p, 1) != 0 {
                    p = p.wrapping_add(2);
                } else {
                    p = p.wrapping_add(1);
                }
            } else if (crate::memory::cstr_first_byte(p) == b'%'
                || crate::memory::cstr_first_byte(p) == b'_')
                && magic_val == MAGIC_ALL
            {
                if crate::memory::cstr_byte_at(p, 1) != 0 {
                    p = p.wrapping_add(2);
                } else {
                    p = p.wrapping_add(1);
                }
            } else if mb_isupper(i32::from(crate::memory::cstr_first_byte(p))) {
                return true;
            } else {
                p = p.wrapping_add(1);
            }
        }
        false
    }
}

pub fn last_csearch() -> *const c_char {
    (&raw const srch().lastc_bytes).cast::<c_char>()
}

pub fn last_csearch_forward() -> i32 {
    i32::from(srch().lastcdir == FORWARD)
}

pub fn last_csearch_until() -> i32 {
    i32::from(srch().last_t_cmd)
}

pub fn set_last_csearch(c: i32, s: impl crate::strings::IntoCCharPtr, len: i32) {
    let s = s.into_c_char_ptr();
    unsafe {
        srch().lastc[0] = c as u8;
        srch().lastc_bytelen = len;
        if len != 0 {
            ptr::copy_nonoverlapping(
                s,
                (&raw mut srch().lastc_bytes).cast::<c_char>(),
                len as usize,
            );
        } else {
            srch().lastc_bytes = [0; MAX_SCHAR_SIZE + 1];
        }
    }
}

pub fn set_csearch_direction(cdir: Direction) {
    srch().lastcdir = cdir;
}

pub fn set_csearch_until(t_cmd: i32) {
    srch().last_t_cmd = t_cmd != 0;
}

pub fn last_search_pat() -> *mut c_char {
    srch().spats[srch().last_idx as usize].pat_ptr()
}

pub fn reset_search_dir() {
    srch().spats[0].off.dir = b'/' as c_char;
    set_vv_searchforward();
}

pub fn set_last_search_pat(
    s: impl crate::strings::IntoCCharPtr,
    idx: i32,
    magic: i32,
    setlast: bool,
) {
    let s = s.into_c_char_ptr();
    {
        let i = idx as usize;
        free_spat(&mut srch().spats[i]);
        if crate::memory::cstr_is_empty(s) {
            srch().spats[i].pat = None;
        } else {
            srch().spats[i].pat = Some(crate::memory::cstr_to_cstring(s));
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
        if opt_vars().p_hls != 0 && idx == srch().last_idx && !g_st().search_global.no_hlsearch {
            redraw_all_later(UPD_SOME_VALID);
        }
    }
}

pub fn last_pat_prog(regmatch: &mut Regmmatch) {
    if srch().spats[srch().last_idx as usize].pat.is_none() {
        regmatch.regprog = ptr::null_mut();
        return;
    }
    g_st().emsg_off += 1;
    search_regcomp(
        c"",
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
    win: *mut WinInfoInfo,
    buf: &mut FileBuf,
    pos: &mut CursorPos,
    mut end_pos: Option<&mut CursorPos>,
    dir: Direction,
    pat: impl crate::strings::IntoCCharPtr,
    patlen: usize,
    mut count: i32,
    options: i32,
    pat_use: i32,
    magic: bool,
    mut extra_arg: Option<&mut SearchitArg>,
) -> i32 {
    let pat = pat.into_c_char_ptr().cast_mut();
    {
        let mut found: i32;
        let mut lnum: Linenr;
        let mut regmatch: Regmmatch = Regmmatch::new();
        let mut ptr_line: *mut c_char;
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
            ptr::null_mut(),
            RE_SEARCH,
            pat_use,
            options & (SEARCH_HIS + SEARCH_KEEP),
            magic,
            &mut regmatch,
        ) {
            if (options & SEARCH_MSG) != 0 && !g_st().rc_did_emsg {
                semsg!(tr(c"E383: Invalid search string: %s"), get_search_pat());
            }
            return FAIL;
        }

        let search_from_match_end = crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_SEARCH);

        loop {
            if pos.col == MAXCOL {
                start_char_len = 0;
            } else if pos.lnum >= 1 && pos.lnum <= buf.b_ml.ml_line_count && pos.col < MAXCOL - 2 {
                ptr_line = ml_get_buf(buf, pos.lnum);
                if ml_get_buf_len(buf, pos.lnum) <= pos.col {
                    start_char_len = 1;
                } else {
                    start_char_len = utfc_ptr2len(ptr_line.wrapping_add(pos.col as usize));
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
                        win,
                        (&raw mut *buf),
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
                        if lnum + matchpos.lnum > buf.b_ml.ml_line_count {
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
                                        - i32::from(
                                            crate::memory::cstr_byte_at(
                                                ptr_line,
                                                matchpos.col as usize,
                                            ) == 0,
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
                                        && crate::memory::cstr_byte_at(ptr_line, matchcol as usize)
                                            != 0
                                    {
                                        matchcol +=
                                            utfc_ptr2len(ptr_line.wrapping_add(matchcol as usize));
                                    }
                                } else {
                                    matchcol = regmatch.rmm_matchcol;
                                    if crate::memory::cstr_byte_at(ptr_line, matchcol as usize) != 0
                                    {
                                        matchcol +=
                                            utfc_ptr2len(ptr_line.wrapping_add(matchcol as usize));
                                    }
                                }
                                if matchcol == 0 && (options & SEARCH_START) != 0 {
                                    break;
                                }
                                if crate::memory::cstr_byte_at(ptr_line, matchcol as usize) == 0
                                    || {
                                        nmatched = vim_regexec_multi(
                                            &mut regmatch,
                                            win,
                                            (&raw mut *buf),
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
                                        && crate::memory::cstr_byte_at(ptr_line, matchcol as usize)
                                            != 0
                                    {
                                        matchcol +=
                                            utfc_ptr2len(ptr_line.wrapping_add(matchcol as usize));
                                    }
                                } else {
                                    if matchpos.lnum > 0 {
                                        break;
                                    }
                                    matchcol = matchpos.col;
                                    if crate::memory::cstr_byte_at(ptr_line, matchcol as usize) != 0
                                    {
                                        matchcol +=
                                            utfc_ptr2len(ptr_line.wrapping_add(matchcol as usize));
                                    }
                                }
                                if crate::memory::cstr_byte_at(ptr_line, matchcol as usize) == 0
                                    || {
                                        nmatched = vim_regexec_multi(
                                            &mut regmatch,
                                            win,
                                            (&raw mut *buf),
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
                                    ptr_line = ml_get_buf(buf, pos.lnum);
                                    pos.col -= utf_head_off(
                                        ptr_line,
                                        ptr_line.wrapping_add(pos.col as usize),
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

                if opt_vars().p_ws == 0
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
                            TOP_BOT_MSG.as_ptr()
                        } else {
                            BOT_TOP_MSG.as_ptr()
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

        vim_regfree(regmatch.regprog);

        if found == 0 {
            if g_st().got_int {
                emsg(tr(E_INTERR));
            } else if (options & SEARCH_MSG) == SEARCH_MSG {
                if opt_vars().p_ws != 0 {
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
            return FAIL;
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
    srch().spats[0].off.dir = cdir as c_char;
}

fn set_vv_searchforward() {
    set_vim_var_nr(
        VV_SEARCHFORWARD,
        Varnumber::from(srch().spats[0].off.dir == b'/' as c_char),
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
    pat: *mut *mut c_char,
    patlen: &mut usize,
    search_delim: i32,
    options: i32,
    strcopy: &mut Vec<c_char>,
    searchstr: *mut *mut c_char,
    searchstrlen: &mut usize,
    dircp: *mut *mut c_char,
    offset: &mut SearchOffset,
) -> i32 {
    unsafe {
        if (*pat).is_null() || **pat == NUL {
            return 0;
        }

        let mut cmdlen: i32 = 0;
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
            None,
            None,
        );
        if !had_copy && !strcopy.is_empty() {
            let new_ptr = strcopy.as_mut_ptr();
            let len = crate::memory::cstr_len(new_ptr);
            cmdlen += (*patlen).wrapping_sub(len) as i32;
            *pat = new_ptr;
            *patlen = len;
            *searchstr = new_ptr;
            *searchstrlen = len;
        }
        if i32::from(crate::memory::cstr_first_byte(p)) == search_delim {
            *searchstrlen = p.offset_from(*pat) as usize;
            *dircp = p;
            *p = NUL;
            p = p.wrapping_add(1);
        }

        offset.line = false;
        offset.end = false;
        offset.off = 0;
        if crate::memory::cstr_first_byte(p) == b'+'
            || crate::memory::cstr_first_byte(p) == b'-'
            || ascii_isdigit(crate::memory::cstr_first_char(p))
        {
            offset.line = true;
        } else if (options & SEARCH_OPT) != 0
            && (crate::memory::cstr_first_byte(p) == b'e'
                || crate::memory::cstr_first_byte(p) == b's'
                || crate::memory::cstr_first_byte(p) == b'b')
        {
            if crate::memory::cstr_first_byte(p) == b'e' {
                offset.end = true;
            }
            p = p.wrapping_add(1);
        }
        if ascii_isdigit(crate::memory::cstr_first_char(p))
            || crate::memory::cstr_first_byte(p) == b'+'
            || crate::memory::cstr_first_byte(p) == b'-'
        {
            if ascii_isdigit(crate::memory::cstr_first_char(p))
                || ascii_isdigit(crate::memory::cstr_char_at(p, 1))
            {
                {
                    offset.off =
                        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(p)) as _;
                }
            } else if crate::memory::cstr_first_byte(p) == b'-' {
                offset.off = -1;
            } else {
                offset.off = 1;
            }
            p = p.wrapping_add(1);
            while ascii_isdigit(crate::memory::cstr_first_char(p)) {
                p = p.wrapping_add(1);
            }
        }

        let consumed = p.offset_from(*pat) as usize;
        cmdlen += consumed as i32;
        *patlen = (*patlen).wrapping_sub(consumed);
        *pat = p;

        cmdlen
    }
}

pub fn do_search(
    mut oap: Option<&mut Oparg>,
    mut dirc: i32,
    mut search_delim: i32,
    mut pat: *mut c_char,
    mut patlen: usize,
    count: i32,
    options: i32,
    magic: bool,
    mut sia: Option<&mut SearchitArg>,
) -> i32 {
    unsafe {
        let mut searchstr: *mut c_char;
        let mut searchstrlen: usize;
        let mut retval: i32;
        let mut c: i64;
        let mut dircp: *mut c_char;
        let mut strcopy: Vec<c_char> = Vec::new();
        let mut msgbuf_vec: Vec<c_char> = Vec::new();
        let mut msgbuflen: usize = 0;
        let mut has_offset = false;

        g_st().search_global.cmdlen = 0;

        if srch().spats[0].off.line && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_LINEOFF) {
            srch().spats[0].off.line = false;
            srch().spats[0].off.off = 0;
        }

        let old_off = srch().spats[0].off;

        let mut pos = curwin_mut().w_cursor;

        if dirc == 0 {
            dirc = i32::from(srch().spats[0].off.dir as u8);
        } else {
            srch().spats[0].off.dir = dirc as c_char;
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
            if has_folding(curwin_ptr(), pos.lnum, None, &raw mut pos.lnum) {
                pos.col = MAXCOL - 2;
            }
        } else if has_folding(curwin_ptr(), pos.lnum, Some(&mut pos.lnum), ptr::null_mut()) {
            pos.col = 0;
        }

        if g_st().search_global.no_hlsearch && (options & SEARCH_KEEP) == 0 {
            redraw_all_later(UPD_SOME_VALID);
            set_no_hlsearch(false);
        }

        let mut success = true;
        loop {
            let mut show_top_bot_msg = false;

            searchstr = pat;
            searchstrlen = patlen;

            dircp = ptr::null_mut();
            if crate::memory::cstr_is_empty(pat)
                || i32::from(crate::memory::cstr_first_byte(pat)) == search_delim
            {
                if srch().spats[RE_SEARCH as usize].pat.is_none() {
                    if srch().spats[RE_SUBST as usize].pat.is_none() {
                        emsg(tr(E_NOPREVRE));
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

            if !crate::memory::cstr_is_empty(pat) {
                let prev_searchstr = searchstr;
                let prev_searchstrlen = searchstrlen;
                g_st().search_global.cmdlen += parse_search_pattern_offset(
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
                if crate::memory::cstr_is_empty(searchstr) {
                    searchstr = prev_searchstr;
                    searchstrlen = prev_searchstrlen;
                }
            }

            let mut show_search_stats = false;
            if (options & SEARCH_ECHO) != 0
                && messaging()
                && g_st().msg_silent == 0
                && (!g_st().cmd_silent || !shortmess(K_SHM_SEARCHCOUNT))
            {
                let mut off_buf = [0 as c_char; 40];
                let off_buf_ptr = (&raw mut off_buf).cast::<c_char>();
                let mut off_len: usize = 0;

                msg_start();
                msg_ext_set_kind(c"search_cmd");

                if !g_st().cmd_silent
                    && (srch().spats[0].off.line
                        || srch().spats[0].off.end
                        || srch().spats[0].off.off != 0)
                {
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
                            off_buf_ptr.wrapping_add(off_len),
                            off_buf.len() - off_len,
                            c"%+lld",
                            srch().spats[0].off.off,
                        );
                        if n > 0 {
                            off_len += n as usize;
                        }
                    }
                }

                let (p, plen) = if crate::memory::cstr_is_empty(searchstr) {
                    (srch().spats[0].pat_ptr(), srch().spats[0].patlen())
                } else {
                    (searchstr, searchstrlen)
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

                msgbuf_vec = vec![b' ' as c_char; msgbufsize];
                msgbuflen = msgbufsize - 1;
                msgbuf_vec[msgbuflen] = NUL;

                if !g_st().cmd_silent {
                    let mut msgbuf = msgbuf_vec.as_mut_ptr();
                    ui_busy_start();
                    *msgbuf.wrapping_add(0) = dirc as c_char;
                    if utf_iscomposing_first(utf_ptr2char(p)) {
                        *msgbuf.wrapping_add(1) = b' ' as c_char;
                        ptr::copy(p, msgbuf.wrapping_add(2), plen);
                    } else {
                        ptr::copy(p, msgbuf.wrapping_add(1), plen);
                    }
                    if off_len > 0 {
                        ptr::copy(off_buf_ptr, msgbuf.wrapping_add(plen + 1), off_len);
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

                    if curwin_mut().w_onebuf_opt.wo_rl != 0
                        && crate::memory::cstr_first_byte(curwin_mut().w_onebuf_opt.wo_rlc) == b's'
                    {
                        let rev = reverse_text(msgbuf);
                        msgbuflen = rev.as_bytes().len();
                        msgbuf_vec = rev
                            .into_bytes_with_nul()
                            .into_iter()
                            .map(|b| b as c_char)
                            .collect();
                        msgbuf = msgbuf_vec.as_mut_ptr();
                        let mut r = msgbuf;
                        while crate::memory::cstr_first_byte(r) == b' ' {
                            r = r.wrapping_add(1);
                        }
                        let pat_len = msgbuf.wrapping_add(msgbuflen).offset_from(r) as usize;
                        ptr::copy(r, msgbuf, pat_len);
                        let r_off = r.offset_from(msgbuf) as usize;
                        if r_off >= pat_len {
                            ptr::write_bytes(r, b' ', pat_len);
                        } else {
                            ptr::write_bytes(msgbuf.wrapping_add(pat_len), b' ', r_off);
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

                if !shortmess(K_SHM_SEARCHCOUNT) {
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
                        pos.lnum = curbuf_mut().b_ml.ml_line_count + 1;
                        pos.col = 0;
                    }
                }
            }

            c = i64::from(searchit(
                curwin_ptr(),
                curbuf_mut(),
                &mut pos,
                None,
                if dirc == b'/' as i32 {
                    FORWARD
                } else {
                    BACKWARD
                },
                searchstr,
                searchstrlen,
                count,
                i32::from(srch().spats[0].off.end) * SEARCH_END
                    + (options
                        & (SEARCH_KEEP
                            + SEARCH_PEEK
                            + SEARCH_HIS
                            + SEARCH_MSG
                            + SEARCH_START
                            + if !pat.is_null() && crate::memory::cstr_first_byte(pat) == b';' {
                                0
                            } else {
                                SEARCH_NOOF
                            })),
                RE_LAST,
                magic,
                (sia.is_some()).then(|| sia.as_deref_mut().unwrap()),
            ));

            if !dircp.is_null() {
                *dircp = search_delim as c_char;
            }

            if !shortmess(K_SHM_SEARCH)
                && sia.is_some()
                && sia.as_deref_mut().unwrap().sa_wrapped != 0
            {
                show_top_bot_msg = true;
            }

            if c == i64::from(FAIL) {
                retval = 0;
                success = false;
                break;
            }
            if srch().spats[0].off.end && oap.is_some() {
                oap.as_deref_mut().unwrap().inclusive = true;
            }
            retval = 1;

            if sia.is_some() && sia.as_deref_mut().unwrap().sa_wrapped != 0 {
                apply_autocmds(
                    EVENT_SEARCHWRAPPED,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    ptr::null_mut(),
                );
            }

            if (options & SEARCH_NOOF) == 0
                || (!pat.is_null() && crate::memory::cstr_first_byte(pat) == b';')
            {
                let org_pos = pos;

                if srch().spats[0].off.line {
                    c = i64::from(pos.lnum) + srch().spats[0].off.off;
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
                    &mut curwin_mut().w_cursor,
                    show_top_bot_msg,
                    msgbuf_vec.as_mut_ptr(),
                    msgbuflen,
                    count != 1
                        || has_offset
                        || ((opt_vars().fdo_flags & K_OPT_FDO_FLAG_SEARCH) == 0
                            && has_folding(
                                curwin_ptr(),
                                curwin_mut().w_cursor.lnum,
                                None,
                                ptr::null_mut(),
                            )),
                    opt_vars().p_msc as i32,
                    SEARCH_STAT_DEF_TIMEOUT,
                );
            }

            if (options & SEARCH_OPT) == 0
                || pat.is_null()
                || crate::memory::cstr_first_byte(pat) != b';'
            {
                break;
            }

            pat = pat.wrapping_add(1);
            dirc = i32::from(crate::memory::cstr_first_byte(pat));
            patlen -= 1;
            search_delim = dirc;
            if dirc != b'?' as i32 && dirc != b'/' as i32 {
                retval = 0;
                emsg(tr(c"E386: Expected '?' or '/'  after ';'"));
                success = false;
                break;
            }
            pat = pat.wrapping_add(1);
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
            srch().spats[0].off = old_off;
        }

        retval
    }
}

pub fn search_for_exact_line(
    buf: &mut FileBuf,
    pos: &mut CursorPos,
    dir: Direction,
    pat: impl crate::strings::IntoCCharPtr,
) -> bool {
    let pat = pat.into_c_char_ptr();
    unsafe {
        let mut start: Linenr = 0;
        let compl_len = ins_compl_len();

        if buf.b_ml.ml_line_count == 0 {
            return false;
        }
        loop {
            pos.lnum += dir;
            if pos.lnum < 1 {
                if opt_vars().p_ws != 0 {
                    pos.lnum = buf.b_ml.ml_line_count;
                    if !shortmess(K_SHM_SEARCH) {
                        give_warning(tr(TOP_BOT_MSG), true, false);
                    }
                } else {
                    pos.lnum = 1;
                    break;
                }
            } else if pos.lnum > buf.b_ml.ml_line_count {
                if opt_vars().p_ws != 0 {
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
            let ptr_line = ml_get_buf(buf, pos.lnum);
            let p = skipwhite(ptr_line);
            pos.col = p.offset_from(ptr_line) as Colnr;

            if compl_status_adding() && !compl_status_sol() {
                if mb_strcmp_ic(opt_vars().p_ic != 0, p, pat) == 0 {
                    return true;
                }
            } else if !crate::memory::cstr_is_empty(p) {
                debug_assert!(compl_len >= 0);
                let eq = if opt_vars().p_ic != 0 {
                    mb_strnicmp(p, pat, compl_len as usize) == 0
                } else {
                    crate::memory::cstr_prefix_bytes(p, compl_len as usize)
                        == crate::memory::cstr_prefix_bytes(pat, compl_len as usize)
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
    unsafe {
        let mut c = cap.nchar;
        let mut dir = cap.arg;
        let mut count = cap.count1;
        let mut stop = true;

        if c != i32::from(NUL) {
            if g_st().key_stuffed == 0 {
                srch().lastc[0] = c as u8;
                set_csearch_direction(dir);
                set_csearch_until(i32::from(t_cmd));
                if cap.nchar_len != 0 {
                    srch().lastc_bytelen = cap.nchar_len;
                    ptr::copy_nonoverlapping(
                        cap.nchar_composing.as_ptr(),
                        (&raw mut srch().lastc_bytes).cast::<c_char>(),
                        cap.nchar_len as usize,
                    );
                } else {
                    srch().lastc_bytelen =
                        utf_char2bytes(c, (&raw mut srch().lastc_bytes).cast::<c_char>());
                }
            }
        } else {
            if srch().lastc[0] == 0 && srch().lastc_bytelen <= 1 {
                return false;
            }
            dir = if dir != 0 {
                -srch().lastcdir
            } else {
                srch().lastcdir
            };
            t_cmd = srch().last_t_cmd;
            c = i32::from(srch().lastc[0]);

            if !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_SCOLON) && count == 1 && t_cmd {
                stop = false;
            }
        }

        cap.oap_mut().inclusive = dir != BACKWARD;

        let p = get_cursor_line_ptr();
        let mut col = curwin_mut().w_cursor.col;
        let len = get_cursor_line_len();

        while count > 0 {
            count -= 1;
            loop {
                if dir > 0 {
                    col += utfc_ptr2len(p.wrapping_add(col as usize));
                    if col >= len {
                        return false;
                    }
                } else {
                    if col == 0 {
                        return false;
                    }
                    col -= utf_head_off(p, p.wrapping_add((col - 1) as usize)) + 1;
                }
                if srch().lastc_bytelen <= 1 {
                    if i32::from(crate::memory::cstr_byte_at(p, col as usize)) == c && stop {
                        break;
                    }
                } else if (crate::memory::cstr_prefix_bytes(
                    p.wrapping_add(col as usize),
                    srch().lastc_bytelen as usize,
                ) == crate::memory::cstr_prefix_bytes(
                    (&raw const srch().lastc_bytes).cast::<c_char>(),
                    srch().lastc_bytelen as usize,
                )) && stop
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
                col -= utf_head_off(p, p.wrapping_add(col as usize));
            }
        }
        curwin_mut().w_cursor.col = col;

        true
    }
}

pub fn findmatch(oap: Option<&mut Oparg>, initc: i32) -> *mut CursorPos {
    findmatchlimit(oap, initc, 0, 0)
}

fn check_prevcol(
    linep: impl crate::strings::IntoCCharPtr,
    mut col: i32,
    ch: i32,
    prevcol: Option<&mut i32>,
) -> bool {
    let linep = linep.into_c_char_ptr();
    {
        col -= 1;
        if col > 0 {
            col -= utf_head_off(linep, linep.wrapping_add(col as usize));
        }
        if let Some(prevcol) = prevcol {
            *prevcol = col;
        }
        col >= 0 && i32::from(crate::memory::cstr_byte_at(linep, col as usize)) == ch
    }
}

fn find_rawstring_end(
    linep: impl crate::strings::IntoCCharPtr,
    startpos: &CursorPos,
    endpos: &CursorPos,
) -> bool {
    let linep = linep.into_c_char_ptr();
    unsafe {
        let mut p = linep.wrapping_add((startpos.col + 1) as usize);
        while !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b'(' {
            p = p.wrapping_add(1);
        }

        let delim_len = (p.offset_from(linep) as usize) - (startpos.col as usize) - 1;
        let mut delim_copy: Vec<c_char> = vec![0; delim_len + 1];
        ptr::copy_nonoverlapping(
            linep.wrapping_add((startpos.col + 1) as usize),
            delim_copy.as_mut_ptr(),
            delim_len,
        );
        let mut found = false;
        for lnum in startpos.lnum..=endpos.lnum {
            let line = ml_get(lnum);
            p = line.wrapping_add(if lnum == startpos.lnum {
                (startpos.col + 1) as usize
            } else {
                0
            });
            while !crate::memory::cstr_is_empty(p) {
                if lnum == endpos.lnum && (p.offset_from(line) as Colnr) >= endpos.col {
                    break;
                }
                if crate::memory::cstr_first_byte(p) == b')'
                    && (crate::memory::cstr_prefix_bytes(delim_copy.as_ptr(), delim_len)
                        == (&crate::memory::cstr_slice(p, (1) + (delim_len))[1..]))
                    && crate::memory::cstr_byte_at(p, delim_len + 1) == b'"'
                {
                    found = true;
                    break;
                }
                p = p.wrapping_add(1);
            }
            if found {
                break;
            }
        }
        found
    }
}

fn find_mps_values(initc: &mut i32, findc: &mut i32, backwards: &mut bool, switchit: bool) {
    {
        let mut ptr_mps = curbuf_mut().b_p_mps;

        while !crate::memory::cstr_is_empty(ptr_mps) {
            if utf_ptr2char(ptr_mps) == *initc {
                if switchit {
                    *findc = *initc;
                    *initc = utf_ptr2char(ptr_mps.wrapping_add(utfc_ptr2len(ptr_mps) as usize + 1));
                    *backwards = true;
                } else {
                    *findc = utf_ptr2char(ptr_mps.wrapping_add(utfc_ptr2len(ptr_mps) as usize + 1));
                    *backwards = false;
                }
                return;
            }
            let prev = ptr_mps;
            ptr_mps = ptr_mps.wrapping_add(utfc_ptr2len(ptr_mps) as usize + 1);
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
            ptr_mps = ptr_mps.wrapping_add(utfc_ptr2len(ptr_mps) as usize);
            if crate::memory::cstr_first_byte(ptr_mps) == b',' {
                ptr_mps = ptr_mps.wrapping_add(1);
            }
        }
    }
}

pub fn findmatchlimit(
    oap: Option<&mut Oparg>,
    mut initc: i32,
    flags: i32,
    maxtravel: i64,
) -> *mut CursorPos {
    unsafe {
        let mut findc: i32 = 0;
        let mut count: i32 = 0;
        let mut backwards = false;
        let mut raw_string = false;
        let mut inquote = false;
        let mut ptr_line: *mut c_char;
        let mut hash_dir: i32 = 0;
        let mut comment_dir: i32 = 0;
        let mut traveled: i64 = 0;
        let mut ignore_cend = false;
        let mut match_escaped: i32 = 0;
        let mut comment_col: i32 = MAXCOL;
        let mut lispcomm = false;
        let lisp = curbuf_mut().b_p_lisp != 0;
        let skip_comments = (flags & FM_SKIPCOMM) != 0;
        let mut in_block_comment = false;

        srch().findmatch_pos = curwin_mut().w_cursor;
        srch().findmatch_pos.coladd = 0;
        let mut linep = ml_get(srch().findmatch_pos.lnum);

        let cpo_match = crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_MATCH);
        let cpo_bsl = crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_MATCHBSL);

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
                return ptr::null_mut();
            }
        } else {
            if initc == b'#' as i32 {
                hash_dir = dir;
            } else {
                if !cpo_match {
                    ptr_line = skipwhite(linep);
                    if crate::memory::cstr_first_byte(ptr_line) == b'#'
                        && srch().findmatch_pos.col <= (ptr_line.offset_from(linep) as Colnr)
                    {
                        ptr_line = skipwhite(ptr_line.wrapping_add(1));
                        if (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"if"[..2])
                            || (crate::memory::cstr_prefix_bytes(ptr_line, 5) == &b"endif"[..5])
                            || (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"el"[..2])
                        {
                            hash_dir = 1;
                        }
                    } else if *linep.wrapping_add(srch().findmatch_pos.col as usize)
                        == b'/' as c_char
                    {
                        if *linep.wrapping_add((srch().findmatch_pos.col + 1) as usize)
                            == b'*' as c_char
                        {
                            comment_dir = FORWARD;
                            backwards = false;
                            srch().findmatch_pos.col += 1;
                        } else if srch().findmatch_pos.col > 0
                            && *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                                == b'*' as c_char
                        {
                            comment_dir = BACKWARD;
                            backwards = true;
                            srch().findmatch_pos.col -= 1;
                        }
                    } else if *linep.wrapping_add(srch().findmatch_pos.col as usize)
                        == b'*' as c_char
                    {
                        if *linep.wrapping_add((srch().findmatch_pos.col + 1) as usize)
                            == b'/' as c_char
                        {
                            comment_dir = BACKWARD;
                            backwards = true;
                        } else if srch().findmatch_pos.col > 0
                            && *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                                == b'/' as c_char
                        {
                            comment_dir = FORWARD;
                            backwards = false;
                        }
                    }
                }

                if hash_dir == 0 && comment_dir == 0 {
                    if *linep.wrapping_add(srch().findmatch_pos.col as usize) == NUL
                        && srch().findmatch_pos.col != 0
                    {
                        srch().findmatch_pos.col -= 1;
                    }
                    loop {
                        initc = utf_ptr2char(linep.wrapping_add(srch().findmatch_pos.col as usize));
                        if initc == i32::from(NUL) {
                            break;
                        }

                        find_mps_values(&mut initc, &mut findc, &mut backwards, false);
                        if findc != 0 {
                            break;
                        }
                        srch().findmatch_pos.col +=
                            utfc_ptr2len(linep.wrapping_add(srch().findmatch_pos.col as usize));
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
                        while check_prevcol(linep, col, b'\\' as i32, Some(&mut col)) {
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
                    ptr_line = skipwhite(skipwhite(linep).wrapping_add(1));
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
                        if srch().findmatch_pos.lnum == curbuf_mut().b_ml.ml_line_count {
                            break;
                        }
                    } else if srch().findmatch_pos.lnum == 1 {
                        break;
                    }
                    srch().findmatch_pos.lnum += hash_dir;
                    linep = ml_get(srch().findmatch_pos.lnum);
                    line_breakcheck();
                    ptr_line = skipwhite(linep);
                    if crate::memory::cstr_first_byte(ptr_line) != b'#' {
                        continue;
                    }
                    srch().findmatch_pos.col = ptr_line.offset_from(linep) as Colnr;
                    ptr_line = skipwhite(ptr_line.wrapping_add(1));
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
                    } else if initc == b'#' as i32
                        && (crate::memory::cstr_prefix_bytes(ptr_line, 2) == &b"el"[..2])
                    {
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

        if curwin_mut().w_onebuf_opt.wo_rl != 0 && crate::strings::vim_haschr(c"()[]{}<>", initc) {
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
                        || (skip_comments
                            && !in_block_comment
                            && srch().findmatch_pos.col > comment_col))
                        && comment_col != MAXCOL
                    {
                        srch().findmatch_pos.col = comment_col;
                    }
                } else {
                    srch().findmatch_pos.col -= 1;
                    srch().findmatch_pos.col -=
                        utf_head_off(linep, linep.wrapping_add(srch().findmatch_pos.col as usize));
                }
            } else if *linep.wrapping_add(srch().findmatch_pos.col as usize) == NUL
                || (lisp && comment_col != MAXCOL && srch().findmatch_pos.col == comment_col)
            {
                if srch().findmatch_pos.lnum == curbuf_mut().b_ml.ml_line_count || lispcomm {
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
                srch().findmatch_pos.col +=
                    utfc_ptr2len(linep.wrapping_add(srch().findmatch_pos.col as usize));
            }

            if skip_comments && comment_dir == 0 && !inquote {
                if backwards {
                    if !in_block_comment
                        && srch().findmatch_pos.col > 0
                        && *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                            == b'*' as c_char
                        && *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'/' as c_char
                        && (comment_col == MAXCOL || srch().findmatch_pos.col < comment_col)
                    {
                        in_block_comment = true;
                    } else if in_block_comment
                        && srch().findmatch_pos.col > 0
                        && *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                            == b'/' as c_char
                        && *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'*' as c_char
                    {
                        in_block_comment = false;
                    }
                } else if !in_block_comment
                    && *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'/' as c_char
                    && *linep.wrapping_add((srch().findmatch_pos.col + 1) as usize)
                        == b'*' as c_char
                    && (comment_col == MAXCOL || srch().findmatch_pos.col < comment_col)
                {
                    in_block_comment = true;
                } else if in_block_comment
                    && srch().findmatch_pos.col > 0
                    && *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                        == b'*' as c_char
                    && *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'/' as c_char
                {
                    in_block_comment = false;
                }
            }

            if srch().findmatch_pos.col == 0
                && (flags & FM_BLOCKSTOP) != 0
                && (crate::memory::cstr_first_byte(linep) == b'{'
                    || crate::memory::cstr_first_byte(linep) == b'}')
            {
                if i32::from(crate::memory::cstr_first_byte(linep)) == findc && count == 0 {
                    return &raw mut srch().findmatch_pos;
                }
                break;
            }

            if comment_dir != 0 {
                if comment_dir == FORWARD {
                    if *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'*' as c_char
                        && *linep.wrapping_add((srch().findmatch_pos.col + 1) as usize)
                            == b'/' as c_char
                    {
                        srch().findmatch_pos.col += 1;
                        return &raw mut srch().findmatch_pos;
                    }
                } else if srch().findmatch_pos.col == 0 {
                    continue;
                } else if raw_string {
                    if *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                        == b'R' as c_char
                        && *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'"' as c_char
                        && !vim_strchr(
                            linep.wrapping_add((srch().findmatch_pos.col + 1) as usize),
                            b'(' as i32,
                        )
                        .is_null()
                    {
                        if !find_rawstring_end(
                            linep,
                            &srch().findmatch_pos,
                            if count > 0 {
                                &match_pos
                            } else {
                                &curwin_mut().w_cursor
                            },
                        ) {
                            count += 1;
                            match_pos = srch().findmatch_pos;
                            match_pos.col -= 1;
                        }
                        linep = ml_get(srch().findmatch_pos.lnum);
                    }
                } else if *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                    == b'/' as c_char
                    && *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'*' as c_char
                    && (srch().findmatch_pos.col == 1
                        || *linep.wrapping_add((srch().findmatch_pos.col - 2) as usize)
                            != b'*' as c_char)
                    && srch().findmatch_pos.col < comment_col
                {
                    count += 1;
                    match_pos = srch().findmatch_pos;
                    match_pos.col -= 1;
                } else if *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                    == b'*' as c_char
                    && *linep.wrapping_add(srch().findmatch_pos.col as usize) == b'/' as c_char
                {
                    if count > 0 {
                        srch().findmatch_pos = match_pos;
                    } else if srch().findmatch_pos.col > 1
                        && *linep.wrapping_add((srch().findmatch_pos.col - 2) as usize)
                            == b'/' as c_char
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
                while !crate::memory::cstr_is_empty(ptr_line) {
                    if ptr_line
                        == linep.wrapping_add(
                            (srch().findmatch_pos.col + i32::from(backwards)) as usize,
                        )
                    {
                        at_start = do_quotes & 1;
                    }
                    if crate::memory::cstr_first_byte(ptr_line) == b'"'
                        && (ptr_line == linep
                            || *ptr_line.wrapping_sub(1) != b'\'' as c_char
                            || crate::memory::cstr_byte_at(ptr_line, 1) != b'\'')
                    {
                        do_quotes += 1;
                    }
                    if crate::memory::cstr_first_byte(ptr_line) == b'\\'
                        && crate::memory::cstr_byte_at(ptr_line, 1) != 0
                    {
                        ptr_line = ptr_line.wrapping_add(1);
                    }
                    ptr_line = ptr_line.wrapping_add(1);
                }
                do_quotes &= 1;

                if do_quotes == 0 {
                    inquote = false;
                    if ptr_line > linep && *ptr_line.wrapping_sub(1) == b'\\' as c_char {
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
                    if srch().findmatch_pos.lnum > 1 {
                        ptr_line = ml_get(srch().findmatch_pos.lnum - 1);
                        if !crate::memory::cstr_is_empty(ptr_line)
                            && *ptr_line.wrapping_add(
                                (ml_get_len(srch().findmatch_pos.lnum - 1) - 1) as usize,
                            ) == b'\\' as c_char
                        {
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
                        linep = ml_get(srch().findmatch_pos.lnum);
                    }
                }
            }
            if start_in_quotes == K_NONE {
                start_in_quotes = K_FALSE;
            }

            let c = utf_ptr2char(linep.wrapping_add(srch().findmatch_pos.col as usize));
            if c == i32::from(NUL) {
                if srch().findmatch_pos.col == 0
                    || *linep.wrapping_add((srch().findmatch_pos.col - 1) as usize)
                        != b'\\' as c_char
                {
                    inquote = false;
                    start_in_quotes = K_FALSE;
                }
            } else if c == b'"' as i32 {
                if do_quotes != 0 {
                    let mut col = srch().findmatch_pos.col - 1;
                    while col >= 0 {
                        if crate::memory::cstr_byte_at(linep, col as usize) != b'\\' {
                            break;
                        }
                        col -= 1;
                    }
                    if ((srch().findmatch_pos.col - 1 - col) & 1) == 0 {
                        inquote = !inquote;
                        start_in_quotes = K_FALSE;
                    }
                }
            } else {
                let mut skipped_single_quote = false;
                if c == b'\'' as i32 && !cpo_match && initc != b'\'' as i32 && findc != b'\'' as i32
                {
                    if backwards {
                        if srch().findmatch_pos.col > 1 {
                            if *linep.wrapping_add((srch().findmatch_pos.col - 2) as usize)
                                == b'\'' as c_char
                            {
                                srch().findmatch_pos.col -= 2;
                                skipped_single_quote = true;
                            } else if *linep.wrapping_add((srch().findmatch_pos.col - 2) as usize)
                                == b'\\' as c_char
                                && srch().findmatch_pos.col > 2
                                && *linep.wrapping_add((srch().findmatch_pos.col - 3) as usize)
                                    == b'\'' as c_char
                            {
                                srch().findmatch_pos.col -= 3;
                                skipped_single_quote = true;
                            }
                        }
                    } else if *linep.wrapping_add((srch().findmatch_pos.col + 1) as usize) != NUL {
                        if *linep.wrapping_add((srch().findmatch_pos.col + 1) as usize)
                            == b'\\' as c_char
                            && *linep.wrapping_add((srch().findmatch_pos.col + 2) as usize) != NUL
                            && *linep.wrapping_add((srch().findmatch_pos.col + 3) as usize)
                                == b'\'' as c_char
                        {
                            srch().findmatch_pos.col += 3;
                            skipped_single_quote = true;
                        } else if *linep.wrapping_add((srch().findmatch_pos.col + 2) as usize)
                            == b'\'' as c_char
                        {
                            srch().findmatch_pos.col += 2;
                            skipped_single_quote = true;
                        }
                    }
                }

                if !skipped_single_quote {
                    if curbuf_mut().b_p_lisp != 0
                        && !vim_strchr(c"(){}[]", c).is_null()
                        && srch().findmatch_pos.col > 1
                        && check_prevcol(linep, srch().findmatch_pos.col, b'\\' as i32, None)
                        && check_prevcol(linep, srch().findmatch_pos.col - 1, b'#' as i32, None)
                    {
                        continue;
                    }

                    if skip_comments
                        && (in_block_comment
                            || (comment_col != MAXCOL && srch().findmatch_pos.col >= comment_col))
                    {
                        continue;
                    }

                    if (!inquote || start_in_quotes == K_TRUE) && (c == initc || c == findc) {
                        let mut bslcnt = 0;

                        if !cpo_bsl {
                            let mut col = srch().findmatch_pos.col;
                            while check_prevcol(linep, col, b'\\' as i32, Some(&mut col)) {
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

pub fn showmatch(c: i32) {
    unsafe {
        let mut vcol: Colnr = 0;
        let so: *mut OptInt = if curwin_mut().w_onebuf_opt.wo_so >= 0 {
            &raw mut curwin_mut().w_onebuf_opt.wo_so
        } else {
            &raw mut opt_vars().p_so
        };
        let siso: *mut OptInt = if curwin_mut().w_onebuf_opt.wo_siso >= 0 {
            &raw mut curwin_mut().w_onebuf_opt.wo_siso
        } else {
            &raw mut opt_vars().p_siso
        };

        let mut p = curbuf_mut().b_p_mps;
        while !crate::memory::cstr_is_empty(p) {
            if utf_ptr2char(p) == c
                && ((curwin_mut().w_onebuf_opt.wo_rl != 0) ^ (opt_vars().p_ri != 0))
            {
                break;
            }
            p = p.wrapping_add(utfc_ptr2len(p) as usize + 1);
            if utf_ptr2char(p) == c
                && !((curwin_mut().w_onebuf_opt.wo_rl != 0) ^ (opt_vars().p_ri != 0))
            {
                break;
            }
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
            if crate::memory::cstr_is_empty(p) {
                return;
            }
            p = p.wrapping_add(1);
        }
        if crate::memory::cstr_is_empty(p) {
            return;
        }

        let lpos = findmatch(None, i32::from(NUL));
        if lpos.is_null() {
            vim_beep(K_OPT_BO_FLAG_SHOWMATCH);
            return;
        }
        let lpos = { &mut *lpos };

        if lpos.lnum < curwin_mut().w_topline || lpos.lnum >= curwin_mut().w_botline {
            return;
        }

        if curwin_mut().w_onebuf_opt.wo_wrap == 0 {
            getvcol(curwin_mut(), &mut *lpos, None, Some(&mut vcol), None, 0);
        }

        let col_visible = curwin_mut().w_onebuf_opt.wo_wrap != 0
            || (vcol >= curwin_mut().w_leftcol
                && vcol < curwin_mut().w_leftcol + curwin_mut().w_view_width);
        if !col_visible {
            return;
        }

        let mpos = *lpos;
        let save_cursor = curwin_mut().w_cursor;
        let save_so = *so;
        let save_siso = *siso;
        if g_st().dollar_vcol >= 0 && g_st().dollar_vcol == curwin_mut().w_virtcol {
            g_st().dollar_vcol = -1;
        }
        curwin_mut().w_virtcol += 1;

        let save_dollar_vcol = g_st().dollar_vcol;
        let save_state = g_st().state_global;
        g_st().state_global = MODE_SHOWMATCH;
        ui_cursor_shape();
        curwin_mut().w_cursor = mpos;
        *so = 0;
        *siso = 0;
        show_cursor_info_later(false);
        update_screen();
        setcursor();
        ui_flush();
        g_st().dollar_vcol = save_dollar_vcol;

        if !vim_strchr(opt_vars().p_cpo, K_CPO_SHOWMATCH).is_null() {
            os_delay((opt_vars().p_mat as u64) * 100 + 8, true);
        } else if !char_avail() {
            os_delay((opt_vars().p_mat as u64) * 100 + 9, false);
        }
        curwin_mut().w_cursor = save_cursor;
        *so = save_so;
        *siso = save_siso;
        g_st().state_global = save_state;
        ui_cursor_shape();
    }
}

pub fn current_search(count: i32, forward: bool) -> bool {
    {
        let old_p_ws = opt_vars().p_ws;
        let save_visual = g_st().visual_global.start;

        if g_st().visual_global.active
            && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
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

        let zero_width = is_zero_width(
            srch().spats[srch().last_idx as usize].pat_ptr(),
            srch().spats[srch().last_idx as usize].patlen(),
            true,
            &mut curwin_mut().w_cursor,
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
                opt_vars().p_ws = 0;
            }

            result = searchit(
                curwin_ptr(),
                curbuf_mut(),
                &mut pos,
                Some(&mut end_pos),
                if dir { FORWARD } else { BACKWARD },
                srch().spats[srch().last_idx as usize].pat_ptr(),
                srch().spats[srch().last_idx as usize].patlen(),
                if i != 0 { count } else { 1 },
                SEARCH_KEEP | flags,
                RE_SEARCH,
                opt_vars().p_magic != 0,
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

        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
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
}

fn is_zero_width(
    pattern: impl crate::strings::IntoCCharPtr,
    mut patternlen: usize,
    move_to_start: bool,
    cur: &mut CursorPos,
    direction: Direction,
) -> i32 {
    let mut pattern = pattern.into_c_char_ptr();
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

        if pattern.is_null() {
            pattern = srch().spats[srch().last_idx as usize].pat_ptr();
            patternlen = srch().spats[srch().last_idx as usize].patlen();
        }

        if !search_regcomp(
            pattern,
            patternlen,
            ptr::null_mut(),
            RE_SEARCH,
            RE_SEARCH,
            SEARCH_KEEP,
            opt_vars().p_magic != 0,
            &mut regmatch,
        ) {
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
            curwin_ptr(),
            curbuf_mut(),
            &mut pos,
            None,
            direction,
            pattern,
            patternlen,
            1,
            SEARCH_KEEP + flag,
            RE_SEARCH,
            opt_vars().p_magic != 0,
            None,
        ) != FAIL
        {
            let mut nmatched: i32;
            loop {
                regmatch.startpos[0].col += 1;
                let start_col = regmatch.startpos[0].col;
                nmatched = vim_regexec_multi(
                    &mut regmatch,
                    curwin_ptr(),
                    g_st().curbuf,
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

        vim_regfree(regmatch.regprog);
        result
    }
}

pub fn linewhite(lnum: Linenr) -> bool {
    {
        let p = skipwhite(ml_get(lnum));
        crate::memory::cstr_is_empty(p)
    }
}

fn cmdline_search_stat(
    dirc: i32,
    pos: &mut CursorPos,
    cursor_pos: &mut CursorPos,
    show_top_bot_msg: bool,
    msgbuf: impl crate::strings::IntoCCharPtr,
    msgbuflen: usize,
    recompute: bool,
    maxcount: i32,
    timeout: i32,
) {
    let msgbuf = msgbuf.into_c_char_ptr().cast_mut();
    unsafe {
        let mut stat = Searchstat::default();

        update_search_stat(
            dirc, pos, cursor_pos, &mut stat, recompute, maxcount, timeout,
        );
        if stat.cur <= 0 {
            return;
        }

        let mut t = [0 as c_char; SEARCH_STAT_BUF_LEN];
        let t_ptr = (&raw mut t).cast::<c_char>();
        let mut len: usize;

        if curwin_mut().w_onebuf_opt.wo_rl != 0
            && crate::memory::cstr_first_byte(curwin_mut().w_onebuf_opt.wo_rlc) == b's'
        {
            if stat.incomplete == 1 {
                len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[?/??]") as usize;
            } else if stat.cnt > maxcount && stat.cur > maxcount {
                len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[>%d/>%d]", maxcount, maxcount,)
                    as usize;
            } else if stat.cnt > maxcount {
                len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[>%d/%d]", maxcount, stat.cur,)
                    as usize;
            } else {
                len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[%d/%d]", stat.cnt, stat.cur,)
                    as usize;
            }
        } else if stat.incomplete == 1 {
            len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[?/??]") as usize;
        } else if stat.cnt > maxcount && stat.cur > maxcount {
            len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[>%d/>%d]", maxcount, maxcount,)
                as usize;
        } else if stat.cnt > maxcount {
            len = vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[%d/>%d]", stat.cur, maxcount,)
                as usize;
        } else {
            len =
                vim_snprintf!(t_ptr, SEARCH_STAT_BUF_LEN, c"[%d/%d]", stat.cur, stat.cnt,) as usize;
        }

        if show_top_bot_msg && len + 2 < SEARCH_STAT_BUF_LEN {
            ptr::copy(t_ptr, t_ptr.wrapping_add(2), len);
            *t_ptr.wrapping_add(0) = b'W' as c_char;
            *t_ptr.wrapping_add(1) = b' ' as c_char;
            len += 2;
        }

        if len > msgbuflen {
            len = msgbuflen;
        }
        ptr::copy(t_ptr, msgbuf.wrapping_add(msgbuflen - len), len);

        g_st().msg_ext_overwrite = true;
        msg_ext_set_kind(c"search_count");
        give_warning(msgbuf, false, false);
    }
}

fn update_search_stat(
    dirc: i32,
    pos: &mut CursorPos,
    cursor_pos: &mut CursorPos,
    stat: &mut Searchstat,
    recompute: bool,
    maxcount: i32,
    timeout: i32,
) {
    {
        let save_ws = opt_vars().p_ws;
        let p = *pos;

        *stat = Searchstat::default();

        if dirc == 0 && !recompute && !empty_pos(srch().stat_lastpos) {
            stat.cur = srch().stat_cur;
            stat.cnt = srch().stat_cnt;
            stat.exact_match = srch().stat_exact_match;
            stat.incomplete = srch().stat_incomplete;
            stat.last_maxcount = srch().stat_last_maxcount;
            return;
        }
        srch().stat_last_maxcount = maxcount;
        let wraparound = (dirc == b'?' as i32 && lt(srch().stat_lastpos, p))
            || (dirc == b'/' as i32 && lt(p, srch().stat_lastpos));

        if !(i64::from(srch().stat_chgtick) == buf_get_changedtick(curbuf_mut())
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
            srch().stat_lastpos = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            srch().stat_lbuf = g_st().curbuf;
        }

        if equalpos(srch().stat_lastpos, *cursor_pos)
            && !wraparound
            && (if dirc == 0 || dirc == b'/' as i32 {
                srch().stat_cur < srch().stat_cnt
            } else {
                srch().stat_cur > 1
            })
        {
            srch().stat_cur += if dirc == 0 {
                0
            } else if dirc == b'/' as i32 {
                1
            } else {
                -1
            };
        } else {
            let mut start: Proftime = 0;
            let mut done_search = false;
            let mut endpos = CursorPos {
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
                    curwin_ptr(),
                    curbuf_mut(),
                    &mut srch().stat_lastpos,
                    Some(&mut endpos),
                    FORWARD,
                    ptr::null_mut(),
                    0,
                    1,
                    SEARCH_KEEP,
                    RE_LAST,
                    opt_vars().p_magic != 0,
                    None,
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
                srch().stat_chgtick = buf_get_changedtick(curbuf_mut()) as i32;
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

pub fn f_searchcount(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut pos = curwin_mut().w_cursor;
        let mut pattern: *mut c_char = ptr::null_mut();
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
            let dict: *mut TvDict = argvars[0].vval.v_dict();
            let mut di: *mut Dictitem = tv_dict_find(dict.as_ref(), c"timeout", -1);
            if !di.is_null() {
                timeout = tv_get_number_chk(&(*di).di_tv, Some(&mut error)) as i32;
                if error {
                    return;
                }
            }
            di = tv_dict_find(dict.as_ref(), c"maxcount", -1);
            if !di.is_null() {
                maxcount = tv_get_number_chk(&(*di).di_tv, Some(&mut error)) as i32;
                if error {
                    return;
                }
            }
            recompute = tv_dict_get_bool(dict.as_ref(), c"recompute", i32::from(recompute)) != 0;
            di = tv_dict_find(dict.as_ref(), c"pattern", -1);
            if !di.is_null() {
                pattern = tv_get_string_chk(&(*di).di_tv).cast_mut();
                if pattern.is_null() {
                    return;
                }
            }
            di = tv_dict_find(dict.as_ref(), c"pos", -1);
            if !di.is_null() {
                let di = { &mut *di };
                let di = { &mut *di };
                if di.di_tv.v_type != VarType::VarList {
                    semsg!(tr(E_INVARG2), c"pos");
                    return;
                }
                if tv_list_len((di.di_tv.vval.v_list()).as_ref()) != 3 {
                    semsg!(tr(E_INVARG2), c"List format should be [lnum, col, off]",);
                    return;
                }
                let mut li: *mut Listitem = tv_list_find((di.di_tv.vval.v_list()).as_mut(), 0);
                if !li.is_null() {
                    pos.lnum = tv_get_number_chk(&(*li).li_tv, Some(&mut error)) as Linenr;
                    if error {
                        return;
                    }
                }
                li = tv_list_find((di.di_tv.vval.v_list()).as_mut(), 1);
                if !li.is_null() {
                    pos.col = tv_get_number_chk(&(*li).li_tv, Some(&mut error)) as Colnr - 1;
                    if error {
                        return;
                    }
                }
                li = tv_list_find((di.di_tv.vval.v_list()).as_mut(), 2);
                if !li.is_null() {
                    pos.coladd = tv_get_number_chk(&(*li).li_tv, Some(&mut error)) as Colnr;
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
            if crate::memory::cstr_is_empty(pattern) {
                do_update = false;
            } else {
                srch().spats[srch().last_idx as usize].pat =
                    Some(crate::memory::cstr_to_cstring(pattern));
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
                &mut *rettv.vval.v_dict(),
                c"current",
                Varnumber::from(stat.cur),
            );
            tv_dict_add_nr(
                &mut *rettv.vval.v_dict(),
                c"total",
                Varnumber::from(stat.cnt),
            );
            tv_dict_add_nr(
                &mut *rettv.vval.v_dict(),
                c"exact_match",
                Varnumber::from(stat.exact_match),
            );
            tv_dict_add_nr(
                &mut *rettv.vval.v_dict(),
                c"incomplete",
                Varnumber::from(stat.incomplete),
            );
            tv_dict_add_nr(
                &mut *rettv.vval.v_dict(),
                c"maxcount",
                Varnumber::from(stat.last_maxcount),
            );
        }

        restore_last_search_pattern();
        restore_incsearch_state();
    }
}

fn get_line_and_copy(lnum: Linenr, buf: *mut c_char) -> *mut c_char {
    let line = ml_get(lnum);
    xstrlcpy(buf, line, LSIZE);
    buf
}

pub fn find_pattern_in_path(
    ptr_pat: impl crate::strings::IntoCCharPtr,
    mut dir: Direction,
    len: usize,
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
    let ptr_pat = ptr_pat.into_c_char_ptr();
    unsafe {
        let mut max_path_depth: i32 = 50;
        let mut match_count: i32 = 1;

        let mut curr_fname = curbuf_mut().b_fname;
        let mut prev_fname: *mut c_char = ptr::null_mut();
        let mut p: *mut c_char;
        let mut define_matched: bool;
        let mut regmatch: Regmatch = Regmatch::new();
        let mut incl_regmatch: Regmatch = Regmatch::new();
        let mut def_regmatch: Regmatch = Regmatch::new();
        let mut matched = false;
        let mut did_show = false;
        let mut found = false;
        let mut i: i32;
        let mut already: *mut c_char = ptr::null_mut();
        let mut startp: *mut c_char = ptr::null_mut();
        let mut curwin_save: *mut WinInfoInfo = ptr::null_mut();
        let l_g_do_tagpreview = g_st().g_do_tagpreview;

        let mut file_line_vec: Vec<c_char> = vec![0; LSIZE];
        let file_line = file_line_vec.as_mut_ptr();

        if type_ != CHECK_PATH && type_ != FIND_DEFINE && !compl_status_sol() {
            let patsize = len + 5;
            let mut pat_vec: Vec<c_char> = vec![0; patsize];
            let pat = pat_vec.as_mut_ptr();
            debug_assert!(len <= i32::MAX as usize);
            vim_snprintf!(
                pat,
                patsize,
                if whole {
                    c"\\<%.*s\\>".as_ptr()
                } else {
                    c"%.*s".as_ptr()
                },
                len as i32,
                ptr_pat,
            );
            regmatch.rm_ic = ignorecase(pat) != 0;
            regmatch.regprog = vim_regcomp(pat, if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                return;
            }
        }
        let inc_opt = if crate::memory::cstr_is_empty(curbuf_mut().b_p_inc) {
            opt_vars().p_inc
        } else {
            curbuf_mut().b_p_inc
        };
        if !crate::memory::cstr_is_empty(inc_opt) {
            incl_regmatch.regprog =
                vim_regcomp(inc_opt, if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });
            if incl_regmatch.regprog.is_null() {
                vim_regfree(regmatch.regprog);
                return;
            }
            incl_regmatch.rm_ic = false;
        }
        if type_ == FIND_DEFINE
            && (!crate::memory::cstr_is_empty(curbuf_mut().b_p_def)
                || !crate::memory::cstr_is_empty(opt_vars().p_def))
        {
            def_regmatch.regprog = vim_regcomp(
                if crate::memory::cstr_is_empty(curbuf_mut().b_p_def) {
                    opt_vars().p_def
                } else {
                    curbuf_mut().b_p_def
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
        let mut depth: i32 = -1;
        let mut depth_displayed: i32 = -1;

        end_lnum = end_lnum.min(curbuf_mut().b_ml.ml_line_count);
        let mut lnum = start_lnum.min(end_lnum);
        let mut line = get_line_and_copy(lnum, file_line);

        loop {
            if !incl_regmatch.regprog.is_null() && vim_regexec(&mut incl_regmatch, line, 0) {
                let p_fname = if curr_fname == curbuf_mut().b_fname {
                    curbuf_mut().b_ffname
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
                        None,
                    )
                }
                .map(|cstr| {
                    let mut v: Vec<c_char> = cstr
                        .into_bytes_with_nul()
                        .into_iter()
                        .map(|b| b as c_char)
                        .collect();
                    to_slash(v.as_mut_ptr());
                    crate::memory::cstr_to_cstring(v.as_ptr())
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
                            K_PATH_CMP_EXPAND | K_PATH_CMP_FULL,
                        ) {
                            if type_ != CHECK_PATH
                                && action == ACTION_SHOW_ALL
                                && files[i as usize].matched != 0
                            {
                                msg_putchar(b'\n' as i32);
                                if !g_st().got_int {
                                    msg_home_replace(nf_ptr);
                                    msg_puts(tr(c" (includes previously listed match)"));
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
                            msg_outtrans(nf.as_ptr(), HLF_D, false);
                        } else {
                            if find_substr(inc_opt, b"\\zs") {
                                p = incl_regmatch.startp[0];
                                i = incl_regmatch.endp[0].offset_from(incl_regmatch.startp[0])
                                    as i32;
                            } else {
                                p = incl_regmatch.endp[0];
                                while !crate::memory::cstr_is_empty(p)
                                    && !vim_isfilec(i32::from(crate::memory::cstr_first_byte(p)))
                                {
                                    p = p.wrapping_add(1);
                                }
                                i = 0;
                                while vim_isfilec(i32::from(crate::memory::cstr_byte_at(
                                    p, i as usize,
                                ))) {
                                    i += 1;
                                }
                            }

                            if i == 0 {
                                p = incl_regmatch.endp[0];
                                i = crate::memory::cstr_len(p) as i32;
                            } else if p > line {
                                if *p.wrapping_sub(1) == b'"' as c_char
                                    || *p.wrapping_sub(1) == b'<' as c_char
                                {
                                    p = p.wrapping_sub(1);
                                    i += 1;
                                }
                                if crate::memory::cstr_byte_at(p, i as usize) == b'"'
                                    || crate::memory::cstr_byte_at(p, i as usize) == b'>'
                                {
                                    i += 1;
                                }
                            }
                            let save_char = *p.wrapping_add(i as usize);
                            *p.wrapping_add(i as usize) = NUL;
                            msg_outtrans(p, HLF_D, false);
                            *p.wrapping_add(i as usize) = save_char;
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
                    let fp = crate::os::fs::CFile::from_raw(
                        os_fopen(nf_cstr.as_ptr(), c"r").cast::<FILE>(),
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
                        if action == ACTION_EXPAND && !shortmess(K_SHM_COMPLETIONSCAN) && !silent {
                            let iobuff_ptr = g_st().iobuff.as_mut_ptr();
                            g_st().msg_hist_off = true;
                            vim_snprintf!(
                                iobuff_ptr,
                                IOSIZE,
                                tr(c"Scanning included file: %s"),
                                curr_fname,
                            );
                            msg_trunc(iobuff_ptr, true, HLF_R);
                        } else if opt_vars().p_verbose >= 5 {
                            verbose_enter();
                            smsg!(0, tr(c"Searching included file %s"), curr_fname);
                            verbose_leave();
                        }
                    }
                }
            } else {
                p = line;
                loop {
                    define_matched = false;
                    if !def_regmatch.regprog.is_null() && vim_regexec(&mut def_regmatch, line, 0) {
                        p = def_regmatch.endp[0];
                        while !crate::memory::cstr_is_empty(p)
                            && !vim_iswordc(i32::from(crate::memory::cstr_first_byte(p)))
                        {
                            p = p.wrapping_add(1);
                        }
                        define_matched = true;
                    }

                    if def_regmatch.regprog.is_null() || define_matched {
                        if define_matched || compl_status_sol() {
                            startp = skipwhite(p);
                            if opt_vars().p_ic != 0 {
                                matched = mb_strnicmp(startp, ptr_pat, len) == 0;
                            } else {
                                matched = crate::memory::cstr_prefix_bytes(startp, len)
                                    == crate::memory::cstr_prefix_bytes(ptr_pat, len);
                            }
                            if matched
                                && define_matched
                                && whole
                                && vim_iswordc(i32::from(crate::memory::cstr_byte_at(startp, len)))
                            {
                                matched = false;
                            }
                        } else if !regmatch.regprog.is_null()
                            && vim_regexec(&mut regmatch, line, p.offset_from(line) as Colnr)
                        {
                            matched = true;
                            startp = regmatch.startp[0];
                            if skip_comments {
                                if (crate::memory::cstr_first_byte(line) != b'#'
                                    || !(crate::memory::cstr_prefix_bytes(
                                        skipwhite(line.wrapping_add(1)),
                                        6,
                                    ) == &b"define"[..6]))
                                    && get_leader_len(line, ptr::null_mut(), false, true) != 0
                                {
                                    matched = false;
                                }

                                p = skipwhite(line);
                                if matched
                                    || (crate::memory::cstr_first_byte(p) == b'/'
                                        && crate::memory::cstr_byte_at(p, 1) == b'*')
                                    || crate::memory::cstr_first_byte(p) == b'*'
                                {
                                    p = line;
                                    while !crate::memory::cstr_is_empty(p) && p < startp {
                                        if matched
                                            && crate::memory::cstr_first_byte(p) == b'/'
                                            && (crate::memory::cstr_byte_at(p, 1) == b'*'
                                                || crate::memory::cstr_byte_at(p, 1) == b'/')
                                        {
                                            matched = false;
                                            if crate::memory::cstr_byte_at(p, 1) == b'/' {
                                                break;
                                            }
                                            p = p.wrapping_add(1);
                                        } else if !matched
                                            && crate::memory::cstr_first_byte(p) == b'*'
                                            && crate::memory::cstr_byte_at(p, 1) == b'/'
                                        {
                                            matched = true;
                                            p = p.wrapping_add(1);
                                        }
                                        p = p.wrapping_add(1);
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
                            let mut aux = startp;
                            p = startp;
                            let mut skip_add = false;
                            if compl_status_adding()
                                && (crate::memory::cstr_len(p) as i32) >= ins_compl_len()
                            {
                                p = p.wrapping_add(ins_compl_len() as usize);
                                if vim_iswordp(p) {
                                    skip_add = true;
                                } else {
                                    p = find_word_start(p);
                                }
                            }
                            if !skip_add {
                                p = find_word_end(p);
                                i = p.offset_from(aux) as i32;

                                if compl_status_adding() && i == ins_compl_len() {
                                    let iobuff_ptr = g_st().iobuff.as_mut_ptr();
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
                                            LSIZE as i32,
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
                                            if crate::memory::cstr_first_byte(aux) != b')'
                                                && g_st().iobuff[(i - 1) as usize] != TAB
                                            {
                                                if g_st().iobuff[(i - 1) as usize] != b' ' as c_char
                                                {
                                                    g_st().iobuff[i as usize] = b' ' as c_char;
                                                    i += 1;
                                                }
                                                if opt_vars().p_js != 0
                                                    && (g_st().iobuff[(i - 2) as usize]
                                                        == b'.' as c_char
                                                        || g_st().iobuff[(i - 2) as usize]
                                                            == b'?' as c_char
                                                        || g_st().iobuff[(i - 2) as usize]
                                                            == b'!' as c_char)
                                                {
                                                    g_st().iobuff[i as usize] = b' ' as c_char;
                                                    i += 1;
                                                }
                                            }
                                            let mut copy_len = p.offset_from(aux) as usize;
                                            if copy_len >= IOSIZE - (i as usize) {
                                                copy_len = IOSIZE - (i as usize) - 1;
                                            }
                                            ptr::copy_nonoverlapping(
                                                aux,
                                                iobuff_ptr.wrapping_add(i as usize),
                                                copy_len,
                                            );
                                            i += copy_len as i32;
                                            cont_s_ipos = true;
                                        }
                                        g_st().iobuff[i as usize] = NUL;
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
                                        if curr_fname == curbuf_mut().b_fname {
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
                                msg_putchar(b'\n' as i32);
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
                                && lnum == curwin_mut().w_cursor.lnum
                                && l_g_do_tagpreview == 0
                            {
                                emsg(tr(c"E387: Match is on current line"));
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
                                    curwin_save = curwin_ptr();
                                    prepare_tagpreview(
                                        true,
                                        !crate::memory::cstr_is_empty(opt_vars().p_pvp),
                                    );
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
                                            if !win_valid(curwin_save)
                                                || !getfile_success(getfile(
                                                    (*curwin_save).buf_mut().b_fnum,
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
                                            curwin_mut().w_cursor.lnum = lnum;
                                            check_cursor(curwin_mut());
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
                                        curwin_mut().w_cursor.lnum = files[depth as usize].lnum;
                                    }
                                }
                            }
                            if !stop_fpip_loop {
                                if action != ACTION_SHOW {
                                    curwin_mut().w_cursor.col = startp.offset_from(line) as Colnr;
                                    curwin_mut().w_set_curswant = true;
                                }

                                if l_g_do_tagpreview != 0
                                    && curwin_ptr() != curwin_save
                                    && win_valid(curwin_save)
                                {
                                    validate_cursor(curwin_mut());
                                    redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
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
                        depth_displayed = i32::MIN;
                        break;
                    }
                    if def_regmatch.regprog.is_null()
                        && action == ACTION_EXPAND
                        && !compl_status_sol()
                        && !crate::memory::cstr_is_empty(startp)
                        && *startp.wrapping_add(utfc_ptr2len(startp) as usize) != NUL
                    {
                        p = startp.wrapping_add(utfc_ptr2len(startp) as usize);
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

            while depth >= 0 && already.is_null() && {
                line = file_line;
                vim_fgets(line, LSIZE as i32, files[depth as usize].fp_ptr())
            } {
                files[depth as usize].fp = None;
                old_files -= 1;
                files[old_files as usize].name = files[depth as usize].name.take();
                files[old_files as usize].matched = files[depth as usize].matched;
                depth -= 1;
                curr_fname = if depth == -1 {
                    curbuf_mut().b_fname
                } else {
                    files[depth as usize].name_ptr()
                };
                depth_displayed = depth_displayed.min(depth);
            }
            if depth >= 0 {
                files[depth as usize].lnum += 1;
                i = crate::memory::cstr_len(line) as i32;
                if i > 0 && *line.wrapping_add((i - 1) as usize) == b'\n' as c_char {
                    i -= 1;
                    *line.wrapping_add(i as usize) = NUL;
                }
                if i > 0 && *line.wrapping_add((i - 1) as usize) == b'\r' as c_char {
                    i -= 1;
                    *line.wrapping_add(i as usize) = NUL;
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

        vim_regfree(regmatch.regprog);
        vim_regfree(incl_regmatch.regprog);
        vim_regfree(def_regmatch.regprog);
    }
}

fn show_pat_in_path(
    line: impl crate::strings::IntoCCharPtr,
    type_: i32,
    did_show: bool,
    action: i32,
    fp: *mut FILE,
    lnum: *mut Linenr,
    count: i32,
) {
    let mut line = line.into_c_char_ptr().cast_mut();
    unsafe {
        if did_show {
            msg_putchar(b'\n' as i32);
        } else if g_st().msg_silent == 0 {
            gotocmdline(true);
        }
        if g_st().got_int {
            return;
        }
        let mut linelen = crate::memory::cstr_len(line);
        loop {
            let mut p = if linelen > 0 {
                line.wrapping_add(linelen - 1)
            } else {
                line.wrapping_sub(1)
            };
            if !fp.is_null() {
                if p >= line && crate::memory::cstr_first_byte(p) == b'\n' {
                    p = p.wrapping_sub(1);
                }
                if p >= line && crate::memory::cstr_first_byte(p) == b'\r' {
                    p = p.wrapping_sub(1);
                }
                *p.wrapping_add(1) = NUL;
            }
            if action == ACTION_SHOW_ALL {
                let iobuff_ptr = g_st().iobuff.as_mut_ptr();
                vim_snprintf!(iobuff_ptr, IOSIZE, c"%3d: ", count);
                msg_puts(iobuff_ptr);
                vim_snprintf!(iobuff_ptr, IOSIZE, c"%4d", *lnum);
                msg_puts_hl(iobuff_ptr, HLF_N, false);
                msg_puts(c" ");
            }
            msg_prt_line(line, false);

            if g_st().got_int
                || type_ != FIND_DEFINE
                || p < line
                || crate::memory::cstr_first_byte(p) != b'\\'
            {
                break;
            }

            if !fp.is_null() {
                if vim_fgets(line, LSIZE as i32, fp) {
                    break;
                }
                linelen = crate::memory::cstr_len(line);
                *lnum += 1;
            } else {
                *lnum += 1;
                if *lnum > curbuf_mut().b_ml.ml_line_count {
                    break;
                }
                line = ml_get(*lnum);
                linelen = ml_get_len(*lnum) as usize;
            }
            msg_putchar(b'\n' as i32);
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
    srch().last_idx = i32::from(is_substitute_pattern);
}

pub fn search_was_last_used() -> bool {
    srch().last_idx == 0
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
            assert_eq!(pat_has_uppercase(pat.as_ptr().cast_mut()), expected);
        }
        opt_vars().p_magic = save_magic;
        opt_vars().cmp_flags = save_cmp;
    }
}
