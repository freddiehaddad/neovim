//! Code for syntax highlighting (`src/nvim/syntax.c`).

use crate::autocmd::{EVENT_SYNTAX, apply_autocmds};
use crate::buffer::{MAXLNUM, buf_T, buf_get_changedtick};
use crate::charset::{
    ascii_isalpha, ascii_isdigit, ascii_iswhite, check_isopt, getdigits_int, getdigits_int32,
    parse_isopt, skiptowhite, skipwhite, str_foldcase, vim_isprintc, vim_iswordp_buf,
};
use crate::cmdexpand::{EXPAND_HIGHLIGHT, EXPAND_NOTHING, EXPAND_SYNTAX, HLF_D};
use crate::cursor::{colnr_T, linenr_T, lpos_T, pos_T};
use crate::drawscreen::{UPD_NOT_VALID, UPD_SOME_VALID, redraw_curbuf_later, redraw_later};
use crate::errors::*;
use crate::eval::vars::{do_unlet, get_var_value, set_internal_string_var};
use crate::ex_docmd::{
    EX_NOSPC, EX_XFILE, check_nextcmd, do_cmdline_cmd, ends_excmd, exarg_T, expand_filename,
    find_nextcmd, separate_nextcmd,
};
use crate::fold::{foldUpdateAll, foldmethodIsSyntax};
use crate::globals::*;
use crate::highlight_group::{
    highlight_group_name, highlight_link_id, highlight_num_groups, init_highlight, syn_check_group,
    syn_id2attr, syn_list_header, syn_name2id, syn_name2id_len,
};
use crate::indent_c::find_start_comment;
use crate::mbyte::{
    mb_ptr_adv, mb_ptr_back, mb_strcmp_ic, utf_head_off, utf_ptr2char, utfc_ptr2len,
};
use crate::memline::{ml_get, ml_get_buf, ml_get_buf_len, ml_get_len};
use crate::message::{
    emsg, msg, msg_advance, msg_ext_set_kind, msg_outnum, msg_outtrans, msg_outtrans_len,
    msg_putchar, msg_puts, msg_puts_hl, msg_puts_title, semsg,
};
use crate::option_vars::*;
use crate::optionstr::clear_string_option;
use crate::os::env::{EXPAND_BUF_LEN, expand_T};
use crate::os::input::line_breakcheck;
use crate::path::path_is_absolute;
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::profile::{
    profile_add, profile_cmp, profile_divide, profile_end, profile_msg, profile_start,
    profile_zero, proftime_T,
};
use crate::regexp::{
    MAXCOL, NSUBEXP, RE_MAGIC, REX_SET, REX_USE, reg_extmatch_T, regmatch_T, regmmatch_T,
    regprog_T, skip_regexp, vim_regcomp, vim_regcomp_had_eol, vim_regexec, vim_regexec_multi,
    vim_regfree,
};
use crate::runtime::{DIP_ALL, DOSO_NONE, do_source, source_runtime};
use crate::strings::{
    gettext, vim_snprintf, vim_strchr,
    };
use crate::window::win_T;
use std::ffi::{c_char, c_int, c_uint};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct syn_time_T {
    pub total: u64,
    pub slowest: u64,
    pub count: c_int,
    pub match_: c_int,
}

impl syn_time_T {
    pub const fn new() -> Self {
        Self {
            total: 0,
            slowest: 0,
            count: 0,
            match_: 0,
        }
    }
}

#[repr(C)]
pub struct synblock_T {
    pub b_keywtab: std::collections::BTreeMap<Vec<u8>, Vec<keyentry_T>>,
    pub b_keywtab_ic: std::collections::BTreeMap<Vec<u8>, Vec<keyentry_T>>,
    pub b_syn_error: bool,
    pub b_syn_slow: bool,
    pub b_syn_ic: c_int,
    pub b_syn_foldlevel: c_int,
    pub b_syn_spell: c_int,
    pub b_syn_patterns: Vec<synpat_T>,
    pub b_syn_clusters: Vec<syn_cluster_T>,
    pub b_spell_cluster_id: c_int,
    pub b_nospell_cluster_id: c_int,
    pub b_syn_containedin: c_int,
    pub b_syn_sync_flags: c_int,
    pub b_syn_sync_id: i16,
    pub b_syn_sync_minlines: i32,
    pub b_syn_sync_maxlines: i32,
    pub b_syn_sync_linebreaks: i32,
    pub b_syn_linecont_pat: Option<std::ffi::CString>,
    pub b_syn_linecont_prog: Option<Box<regprog_T>>,
    pub b_syn_linecont_time: syn_time_T,
    pub b_syn_linecont_ic: c_int,
    pub b_syn_topgrp: c_int,
    pub b_syn_conceal: c_int,
    pub b_syn_folditems: c_int,
    pub b_sst_array: Vec<synstate_T>,
    pub b_sst_max: c_int,
    pub b_sst_search: usize,
    pub b_sst_check_lnum: i32,
    pub b_sst_lasttick: u64,
    pub b_idlist_cache: Option<Box<idl_cache_T>>,
    pub b_langp: Vec<crate::spellsuggest::langp_T>,
    pub b_spell_ismw: [bool; 256],
    pub b_spell_ismw_mb: Option<std::ffi::CString>,
    pub b_p_spc: *mut c_char,
    pub b_cap_prog: Option<Box<regprog_T>>,
    pub b_p_spf: *mut c_char,
    pub b_p_spl: *mut c_char,
    pub b_p_spo: *mut c_char,
    pub b_p_spo_flags: c_uint,
    pub b_cjk: c_int,
    pub b_syn_chartab: [u64; 4],
    pub b_syn_isk: Option<std::ffi::CString>,
}

impl synblock_T {
    pub const fn new() -> Self {
        Self {
            b_keywtab: std::collections::BTreeMap::new(),
            b_keywtab_ic: std::collections::BTreeMap::new(),
            b_syn_error: false,
            b_syn_slow: false,
            b_syn_ic: 0,
            b_syn_foldlevel: 0,
            b_syn_spell: 0,
            b_syn_patterns: Vec::new(),
            b_syn_clusters: Vec::new(),
            b_spell_cluster_id: 0,
            b_nospell_cluster_id: 0,
            b_syn_containedin: 0,
            b_syn_sync_flags: 0,
            b_syn_sync_id: 0,
            b_syn_sync_minlines: 0,
            b_syn_sync_maxlines: 0,
            b_syn_sync_linebreaks: 0,
            b_syn_linecont_pat: None,
            b_syn_linecont_prog: None,
            b_syn_linecont_time: syn_time_T {
                total: 0,
                slowest: 0,
                count: 0,
                match_: 0,
            },
            b_syn_linecont_ic: 0,
            b_syn_topgrp: 0,
            b_syn_conceal: 0,
            b_syn_folditems: 0,
            b_sst_array: Vec::new(),
            b_sst_max: 0,
            b_sst_search: 0,
            b_sst_check_lnum: 0,
            b_sst_lasttick: 0,
            b_idlist_cache: None,
            b_langp: Vec::new(),
            b_spell_ismw: [false; 256],
            b_spell_ismw_mb: None,
            b_p_spc: ptr::null_mut(),
            b_cap_prog: None,
            b_p_spf: ptr::null_mut(),
            b_p_spl: ptr::null_mut(),
            b_p_spo: ptr::null_mut(),
            b_p_spo_flags: 0,
            b_cjk: 0,
            b_syn_chartab: [0; 4],
            b_syn_isk: None,
        }
    }
}

use crate::ex_getln::disptick_T;

pub const HL_CONTAINED: c_int = 0x01;
pub const HL_TRANSP: c_int = 0x02;
pub const HL_ONELINE: c_int = 0x04;
pub const HL_HAS_EOL: c_int = 0x08;
pub const HL_SYNC_HERE: c_int = 0x10;
pub const HL_SYNC_THERE: c_int = 0x20;
pub const HL_MATCH: c_int = 0x40;
pub const HL_SKIPNL: c_int = 0x80;
pub const HL_SKIPWHITE: c_int = 0x100;
pub const HL_SKIPEMPTY: c_int = 0x200;
pub const HL_KEEPEND: c_int = 0x400;
pub const HL_EXCLUDENL: c_int = 0x800;
pub const HL_DISPLAY: c_int = 0x1000;
pub const HL_FOLD: c_int = 0x2000;
pub const HL_EXTEND: c_int = 0x4000;
pub const HL_MATCHCONT: c_int = 0x8000;
pub const HL_TRANS_CONT: c_int = 0x10000;
pub const HL_CONCEAL: c_int = 0x20000;
pub const HL_CONCEALENDS: c_int = 0x40000;
pub const HL_INCLUDED_TOPLEVEL: c_int = 0x80000;

const SST_MIN_ENTRIES: c_int = 150;
const SST_MAX_ENTRIES: c_int = 1000;
const SST_DIST: c_int = 16;

const SPO_MS_OFF: usize = 0;
const SPO_ME_OFF: usize = 1;
const SPO_HS_OFF: usize = 2;
const SPO_HE_OFF: usize = 3;
const SPO_RS_OFF: usize = 4;
const SPO_RE_OFF: usize = 5;
const SPO_LC_OFF: usize = 6;
const SPO_COUNT: usize = 7;

const SPTYPE_MATCH: c_char = 1;
const SPTYPE_START: c_char = 2;
const SPTYPE_END: c_char = 3;
const SPTYPE_SKIP: c_char = 4;

const NONE_IDX: c_int = -2;
const KEYWORD_IDX: c_int = -1;

const SF_CCOMMENT: c_int = 0x01;
const SF_MATCH: c_int = 0x02;

const MAXKEYWLEN: usize = 80;

const CLUSTER_REPLACE: c_int = 1;
const CLUSTER_ADD: c_int = 2;
const CLUSTER_SUBTRACT: c_int = 3;

const MAX_HL_ID: i16 = 20000;
const SYNID_ALLBUT: i16 = MAX_HL_ID;
const SYNID_TOP: i16 = 21000;
const SYNID_CONTAINED: i16 = 22000;
const SYNID_CLUSTER: i16 = 23000;

const MAX_SYN_INC_TAG: c_int = 999;
const MAX_CLUSTER_ID: c_int = 32767 - SYNID_CLUSTER as c_int;

const SYNSPL_DEFAULT: c_int = 0;
const SYNSPL_TOP: c_int = 1;
const SYNSPL_NOTOP: c_int = 2;

const SYNFLD_START: c_int = 0;
const SYNFLD_MINIMUM: c_int = 1;

const IDL_CACHE_SIZE: usize = 16;

const ITEM_START: usize = 0;
const ITEM_SKIP: usize = 1;
const ITEM_END: usize = 2;
const ITEM_MATCHGROUP: usize = 3;

const EXP_CASE: c_int = 1;
const EXP_SPELL: c_int = 2;
const EXP_SYNC: c_int = 3;
const EXP_CLUSTER: c_int = 4;

const SYNTAX_FNAME: *const c_char = c"$VIMRUNTIME/syntax/%s.vim".as_ptr();
const e_illegal_arg: *const c_char = c"E390: Illegal argument: %s".as_ptr();
const e_contains_argument_not_accepted_here: *const c_char =
    c"E395: Contains argument not accepted here".as_ptr();
const e_invalid_cchar_value: *const c_char = c"E844: Invalid cchar value".as_ptr();
const e_trailing_char_after_rsb_str_str: *const c_char =
    c"E890: Trailing char after ']': %s]%s".as_ptr();
const msg_no_items: *const c_char = c"No Syntax items defined for this buffer".as_ptr();

const spo_name_tab: [*const c_char; SPO_COUNT] = [
    c"ms=".as_ptr(),
    c"me=".as_ptr(),
    c"hs=".as_ptr(),
    c"he=".as_ptr(),
    c"rs=".as_ptr(),
    c"re=".as_ptr(),
    c"lc=".as_ptr(),
];

use crate::charset::NUL;
use crate::sign::EXP_SUBCMD;

pub type SynIdList = Option<std::rc::Rc<[i16]>>;

#[inline]
fn syn_id_list_eq(a: &SynIdList, b: &SynIdList) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b) || (a.is_empty() && b.is_empty()),
        _ => false,
    }
}

#[inline]
fn is_id_list_all(list: &SynIdList) -> bool {
    list.as_ref().is_some_and(|rc| rc.is_empty())
}

#[repr(C)]
#[derive(Clone)]
pub struct sp_syn {
    pub inc_tag: c_int,
    pub id: i16,
    pub cont_in_list: SynIdList,
}

impl sp_syn {
    pub const fn new() -> Self {
        Self {
            inc_tag: 0,
            id: 0,
            cont_in_list: None,
        }
    }
}

impl Default for sp_syn {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct keyentry_T {
    pub k_syn: sp_syn,
    pub next_list: SynIdList,
    pub flags: c_int,
    pub k_char: c_int,
    pub keyword: Vec<c_char>,
}

#[repr(C)]
pub struct bufstate_T {
    pub bs_idx: c_int,
    pub bs_flags: c_int,
    pub bs_seqnr: c_int,
    pub bs_cchar: c_int,
    pub bs_extmatch: Option<std::rc::Rc<reg_extmatch_T>>,
}

#[repr(C)]
#[derive(Default)]
pub struct synstate_T {
    pub sst_lnum: linenr_T,
    pub sst_stack: Vec<bufstate_T>,
    pub sst_next_flags: c_int,
    pub sst_next_list: SynIdList,
    pub sst_tick: disptick_T,
    pub sst_change_lnum: linenr_T,
}

#[repr(C)]
pub struct synpat_T {
    sp_type: c_char,
    sp_syncing: bool,
    sp_syn_match_id: i16,
    sp_off_flags: i16,
    sp_offsets: [c_int; SPO_COUNT],
    sp_flags: c_int,
    sp_cchar: c_int,
    sp_ic: c_int,
    sp_sync_idx: c_int,
    sp_line_id: c_int,
    sp_startcol: c_int,
    sp_cont_list: SynIdList,
    sp_next_list: SynIdList,
    sp_syn: sp_syn,
    sp_pattern: Option<std::ffi::CString>,
    sp_prog: Option<Box<regprog_T>>,
    sp_time: syn_time_T,
}

impl synpat_T {
    pub const fn new() -> Self {
        Self {
            sp_type: 0,
            sp_syncing: false,
            sp_syn_match_id: 0,
            sp_off_flags: 0,
            sp_offsets: [0; SPO_COUNT],
            sp_flags: 0,
            sp_cchar: 0,
            sp_ic: 0,
            sp_sync_idx: 0,
            sp_line_id: 0,
            sp_startcol: 0,
            sp_cont_list: None,
            sp_next_list: None,
            sp_syn: sp_syn::new(),
            sp_pattern: None,
            sp_prog: None,
            sp_time: syn_time_T::new(),
        }
    }
}

impl Default for synpat_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct syn_cluster_T {
    scl_name: std::ffi::CString,
    scl_name_u: std::ffi::CString,
    scl_list: SynIdList,
}

#[repr(C)]
struct stateitem_T {
    si_idx: c_int,
    si_id: c_int,
    si_trans_id: c_int,
    si_m_lnum: c_int,
    si_m_startcol: c_int,
    si_m_endpos: lpos_T,
    si_h_startpos: lpos_T,
    si_h_endpos: lpos_T,
    si_eoe_pos: lpos_T,
    si_end_idx: c_int,
    si_ends: c_int,
    si_attr: c_int,
    si_flags: c_int,
    si_seqnr: c_int,
    si_cchar: c_int,
    si_cont_list: SynIdList,
    si_next_list: SynIdList,
    si_extmatch: Option<std::rc::Rc<reg_extmatch_T>>,
}

impl stateitem_T {
    const fn new() -> Self {
        Self {
            si_idx: 0,
            si_id: 0,
            si_trans_id: 0,
            si_m_lnum: 0,
            si_m_startcol: 0,
            si_m_endpos: lpos_T::new(),
            si_h_startpos: lpos_T::new(),
            si_h_endpos: lpos_T::new(),
            si_eoe_pos: lpos_T::new(),
            si_end_idx: 0,
            si_ends: 0,
            si_attr: 0,
            si_flags: 0,
            si_seqnr: 0,
            si_cchar: 0,
            si_cont_list: None,
            si_next_list: None,
            si_extmatch: None,
        }
    }
}

impl Default for stateitem_T {
    fn default() -> Self {
        Self::new()
    }
}

struct syn_opt_arg_T {
    flags: c_int,
    keyword: bool,
    sync_idx: Option<c_int>,
    has_cont_list: bool,
    cont_list: SynIdList,
    cont_in_list: SynIdList,
    next_list: SynIdList,
}

#[derive(Default)]
pub struct idl_entry_T {
    idl_key: SynIdList,
    idl_ids: Vec<i16>,
    idl_marker: i16,
    idl_usable: bool,
}

#[derive(Default)]
pub struct idl_cache_T {
    ent: [idl_entry_T; IDL_CACHE_SIZE],
    round: c_int,
    last: c_int,
}

#[repr(C)]
#[derive(Clone)]
struct time_entry_T {
    total: proftime_T,
    count: c_int,
    match_: c_int,
    slowest: proftime_T,
    average: proftime_T,
    id: c_int,
    pattern: Option<std::ffi::CString>,
}

struct pat_ptr_T {
    pp_synp: synpat_T,
    pp_matchgroup_id: c_int,
}

struct SyntaxState {
    did_syntax_onoff: bool,
    changedtick_last: i64,
    try_next_column: bool,
    in_id_list_depth: c_int,
    current_attr: c_int,
    current_id: c_int,
    current_trans_id: c_int,
    current_flags: c_int,
    current_seqnr: c_int,
    current_sub_char: c_int,
    current_syn_inc_tag: c_int,
    running_syn_inc_tag: c_int,
    keepend_level: c_int,
    next_seqnr: c_int,
    next_match_col: c_int,
    next_match_m_endpos: lpos_T,
    next_match_h_startpos: lpos_T,
    next_match_h_endpos: lpos_T,
    next_match_idx: c_int,
    next_match_flags: c_int,
    next_match_eos_pos: lpos_T,
    next_match_eoe_pos: lpos_T,
    next_match_end_idx: c_int,
    next_match_extmatch: Option<std::rc::Rc<reg_extmatch_T>>,
    syn_win: *mut win_T,
    syn_buf: *mut buf_T,
    syn_block: *mut synblock_T,
    syn_tm: Option<proftime_T>,
    current_lnum: linenr_T,
    current_col: colnr_T,
    current_state_stored: bool,
    current_finished: bool,
    current_state_valid: bool,
    current_state: Vec<stateitem_T>,
    current_next_list: SynIdList,
    current_next_flags: c_int,
    current_line_id: c_int,
    syn_time_on: bool,
    last_matchgroup: c_int,
    expand_what: c_int,
}

impl SyntaxState {
    const fn new() -> Self {
        Self {
            did_syntax_onoff: false,
            changedtick_last: 0,
            try_next_column: false,
            in_id_list_depth: 0,
            current_attr: 0,
            current_id: 0,
            current_trans_id: 0,
            current_flags: 0,
            current_seqnr: 0,
            current_sub_char: 0,
            current_syn_inc_tag: 0,
            running_syn_inc_tag: 0,
            keepend_level: -1,
            next_seqnr: 1,
            next_match_col: 0,
            next_match_m_endpos: lpos_T { lnum: 0, col: 0 },
            next_match_h_startpos: lpos_T { lnum: 0, col: 0 },
            next_match_h_endpos: lpos_T { lnum: 0, col: 0 },
            next_match_idx: 0,
            next_match_flags: 0,
            next_match_eos_pos: lpos_T { lnum: 0, col: 0 },
            next_match_eoe_pos: lpos_T { lnum: 0, col: 0 },
            next_match_end_idx: 0,
            next_match_extmatch: None,
            syn_win: ptr::null_mut(),
            syn_buf: ptr::null_mut(),
            syn_block: ptr::null_mut(),
            syn_tm: None,
            current_lnum: 0,
            current_col: 0,
            current_state_stored: false,
            current_finished: false,
            current_state_valid: false,
            current_state: Vec::new(),
            current_next_list: None,
            current_next_flags: 0,
            current_line_id: 0,
            syn_time_on: false,
            last_matchgroup: 0,
            expand_what: EXP_SUBCMD,
        }
    }
}

static SYN_STATE: GlobalState<SyntaxState> = GlobalState::new(SyntaxState::new());

#[inline]
fn syn_st() -> &'static mut SyntaxState {
    unsafe { &mut *SYN_STATE.as_ptr() }
}

#[inline]
pub fn win_s(wp: &mut win_T) -> *mut synblock_T {
    wp.w_s_ptr()
}

#[inline]
fn syn_items(block: &mut synblock_T) -> *mut synpat_T {
    block.b_syn_patterns.as_mut_ptr()
}

#[inline]
fn syn_clstr(block: &mut synblock_T) -> *mut syn_cluster_T {
    block.b_syn_clusters.as_mut_ptr()
}

#[inline]
fn cur_state_len() -> c_int {
    unsafe {
        let cs_ptr = &raw const syn_st().current_state;
        (*cs_ptr).len() as c_int
    }
}

#[inline]
fn cur_state_empty() -> bool {
    unsafe {
        let cs_ptr = &raw const syn_st().current_state;
        (*cs_ptr).is_empty()
    }
}

#[inline]
fn cur_state_at(idx: c_int) -> *mut stateitem_T {
    unsafe {
        let cs_ptr = &raw mut syn_st().current_state;
        (*cs_ptr).as_mut_ptr().offset(idx as isize)
    }
}

#[inline]
fn ke2hikey(kp: &keyentry_T) -> *const c_char {
    kp.keyword.as_ptr()
}

#[inline]
fn id_list_all() -> SynIdList {
    Some(std::rc::Rc::from([] as [i16; 0]))
}

#[inline]
fn invalid_state() -> bool {
    !syn_st().current_state_valid
}

#[inline]
fn valid_state() -> bool {
    syn_st().current_state_valid
}

/// Set the timeout used for syntax highlighting.
pub fn syn_set_timeout(tm: Option<proftime_T>) {
    syn_st().syn_tm = tm;
}

/// Start the syntax recognition for a line.
pub fn syntax_start(wp: *mut win_T, lnum: linenr_T) {
    unsafe {
        let mut last_valid: Option<usize> = None;
        let mut last_min_valid: Option<usize> = None;
        let mut prev: Option<usize> = None;

        syn_st().current_sub_char = NUL as c_int;

        if syn_st().syn_block != win_s(&mut *wp)
            || syn_st().syn_buf != (*wp).w_buffer
            || syn_st().changedtick_last != buf_get_changedtick(&*syn_st().syn_buf)
        {
            invalidate_current_state();
            syn_st().syn_buf = (*wp).w_buffer;
            syn_st().syn_block = win_s(&mut *wp);
        }
        syn_st().changedtick_last = buf_get_changedtick(&*syn_st().syn_buf);
        syn_st().syn_win = wp;

        syn_stack_alloc();
        if (*syn_st().syn_block).b_sst_max == 0 {
            return;
        }
        (*syn_st().syn_block).b_sst_lasttick = *display_tick.as_ptr();

        if valid_state() && syn_st().current_lnum < lnum && syn_st().current_lnum < (*syn_st().syn_buf).b_ml.ml_line_count {
            syn_finish_line(false);
            if !syn_st().current_state_stored {
                syn_st().current_lnum += 1;
                store_current_state();
            }

            if syn_st().current_lnum != lnum {
                invalidate_current_state();
            }
        } else {
            invalidate_current_state();
        }

        if invalid_state() && !(*syn_st().syn_block).b_sst_array.is_empty() {
            for (idx, p) in (*syn_st().syn_block).b_sst_array.as_slice().iter().enumerate() {
                if p.sst_lnum > lnum {
                    break;
                }
                if p.sst_change_lnum == 0 {
                    last_valid = Some(idx);
                    if p.sst_lnum >= lnum - (*syn_st().syn_block).b_syn_sync_minlines {
                        last_min_valid = Some(idx);
                    }
                }
            }
            if let Some(idx) = last_min_valid {
                load_current_state(&(*syn_st().syn_block).b_sst_array.as_slice()[idx]);
            }
        }

        let first_stored = if invalid_state() {
            syn_sync(
                wp,
                lnum,
                last_valid.map_or(std::ptr::null(), |idx| {
                    &raw const (*syn_st().syn_block).b_sst_array.as_slice()[idx]
                }),
            );
            if syn_st().current_lnum == 1 {
                1
            } else {
                syn_st().current_lnum + (*syn_st().syn_block).b_syn_sync_minlines
            }
        } else {
            syn_st().current_lnum
        };

        let sst_len = (*syn_st().syn_block).b_sst_max;
        let dist = if sst_len <= g_st().Rows {
            999999
        } else {
            (*syn_st().syn_buf).b_ml.ml_line_count / (sst_len - g_st().Rows) + 1
        };

        while syn_st().current_lnum < lnum {
            syn_start_line();
            syn_finish_line(false);
            syn_st().current_lnum += 1;

            if syn_st().current_lnum >= first_stored {
                if prev.is_none() {
                    prev = syn_stack_find_entry(syn_st().current_lnum - 1);
                }
                let mut sp_idx = prev.unwrap_or(0);
                while sp_idx < (*syn_st().syn_block).b_sst_array.len()
                    && (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_lnum < syn_st().current_lnum
                {
                    sp_idx += 1;
                }
                if sp_idx < (*syn_st().syn_block).b_sst_array.len()
                    && (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_lnum == syn_st().current_lnum
                    && syn_stack_equal(&(*syn_st().syn_block).b_sst_array.as_slice()[sp_idx])
                {
                    let parsed_lnum = syn_st().current_lnum;
                    prev = Some(sp_idx);
                    while sp_idx < (*syn_st().syn_block).b_sst_array.len()
                        && (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_change_lnum
                            <= parsed_lnum
                    {
                        if (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_lnum <= lnum {
                            prev = Some(sp_idx);
                        } else if (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_change_lnum == 0 {
                            break;
                        }
                        (*syn_st().syn_block).b_sst_array.as_mut_slice()[sp_idx].sst_change_lnum = 0;
                        sp_idx += 1;
                    }
                    if let Some(pi) = prev {
                        load_current_state(&(*syn_st().syn_block).b_sst_array.as_slice()[pi]);
                    }
                } else if prev.is_none_or(|pi| {
                    syn_st().current_lnum == lnum
                        || syn_st().current_lnum >= (*syn_st().syn_block).b_sst_array.as_slice()[pi].sst_lnum + dist
                }) {
                    prev = store_current_state();
                }
            }

            line_breakcheck();
            if g_st().got_int {
                syn_st().current_lnum = lnum;
                break;
            }
        }

        syn_start_line();
    }
}

fn clear_current_state() {
    unsafe {
        let cs_ptr = &raw mut syn_st().current_state;
        *cs_ptr = Vec::new();
    }
}

fn syn_sync(wp: *mut win_T, mut start_lnum: linenr_T, last_valid: *const synstate_T) {
    unsafe {
        let mut found_flags: c_int = 0;
        let mut found_match_idx: c_int = 0;
        let mut found_current_lnum: linenr_T = 0;
        let mut found_current_col: colnr_T = 0;
        let mut found_m_endpos = lpos_T { lnum: 0, col: 0 };

        invalidate_current_state();

        if (*syn_st().syn_block).b_syn_sync_minlines > start_lnum {
            start_lnum = 1;
        } else {
            let mut lnum = if (*syn_st().syn_block).b_syn_sync_minlines == 1 {
                1
            } else if (*syn_st().syn_block).b_syn_sync_minlines < 10 {
                (*syn_st().syn_block).b_syn_sync_minlines * 2
            } else {
                (*syn_st().syn_block).b_syn_sync_minlines * 3 / 2
            };
            if (*syn_st().syn_block).b_syn_sync_maxlines != 0 && lnum > (*syn_st().syn_block).b_syn_sync_maxlines {
                lnum = (*syn_st().syn_block).b_syn_sync_maxlines;
            }
            if lnum >= start_lnum {
                start_lnum = 1;
            } else {
                start_lnum -= lnum;
            }
        }
        syn_st().current_lnum = start_lnum;

        if ((*syn_st().syn_block).b_syn_sync_flags & SF_CCOMMENT) != 0 {
            let curwin_save = curwin_ptr();
            set_curwin(wp);
            let curbuf_save = g_st().curbuf;
            g_st().curbuf = syn_st().syn_buf;

            while start_lnum > 1 {
                let l = ml_get(start_lnum - 1);
                if *l == NUL
                    || *l.offset((ml_get_len(start_lnum - 1) - 1) as isize) != b'\\' as c_char
                {
                    break;
                }
                start_lnum -= 1;
            }
            syn_st().current_lnum = start_lnum;

            let cursor_save: pos_T = (*wp).w_cursor;
            (*wp).w_cursor.lnum = start_lnum;
            (*wp).w_cursor.col = 0;

            if !find_start_comment((*syn_st().syn_block).b_syn_sync_maxlines).is_null() {
                let mut idx = (*syn_st().syn_block).b_syn_patterns.len() as c_int - 1;
                while idx >= 0 {
                    let spp = syn_items(&mut *syn_st().syn_block).offset(idx as isize);
                    if (*spp).sp_syn.id == (*syn_st().syn_block).b_syn_sync_id
                        && (*spp).sp_type == SPTYPE_START
                    {
                        validate_current_state();
                        push_current_state(idx);
                        update_si_attr(cur_state_len() - 1);
                        break;
                    }
                    idx -= 1;
                }
            }

            (*wp).w_cursor = cursor_save;
            set_curwin(curwin_save);
            g_st().curbuf = curbuf_save;
        } else if ((*syn_st().syn_block).b_syn_sync_flags & SF_MATCH) != 0 {
            let break_lnum = if (*syn_st().syn_block).b_syn_sync_maxlines != 0
                && start_lnum > (*syn_st().syn_block).b_syn_sync_maxlines
            {
                start_lnum - (*syn_st().syn_block).b_syn_sync_maxlines
            } else {
                0
            };

            let mut end_lnum = start_lnum;
            let mut lnum = start_lnum;
            while {
                lnum -= 1;
                lnum > break_lnum
            } {
                line_breakcheck();
                if g_st().got_int {
                    invalidate_current_state();
                    syn_st().current_lnum = start_lnum;
                    break;
                }

                if !last_valid.is_null() && lnum == (*last_valid).sst_lnum {
                    load_current_state(last_valid);
                    break;
                }

                if lnum > 1 && syn_match_linecont(lnum - 1) != 0 {
                    continue;
                }

                validate_current_state();

                syn_st().current_lnum = lnum;
                while syn_st().current_lnum < end_lnum {
                    syn_start_line();
                    loop {
                        let had_sync_point = syn_finish_line(true);
                        if had_sync_point && !cur_state_empty() {
                            let cur_si = cur_state_at(cur_state_len() - 1);
                            if (*cur_si).si_m_endpos.lnum > start_lnum {
                                syn_st().current_lnum = end_lnum;
                                break;
                            }
                            if (*cur_si).si_idx < 0 {
                                found_flags = 0;
                                found_match_idx = KEYWORD_IDX;
                            } else {
                                let spp = syn_items(&mut *syn_st().syn_block).offset((*cur_si).si_idx as isize);
                                found_flags = (*spp).sp_flags;
                                found_match_idx = (*spp).sp_sync_idx;
                            }
                            found_current_lnum = syn_st().current_lnum;
                            found_current_col = syn_st().current_col;
                            found_m_endpos = (*cur_si).si_m_endpos;
                            if found_m_endpos.lnum > syn_st().current_lnum {
                                syn_st().current_lnum = found_m_endpos.lnum;
                                syn_st().current_col = found_m_endpos.col;
                                if syn_st().current_lnum >= end_lnum {
                                    break;
                                }
                            } else if found_m_endpos.col > syn_st().current_col {
                                syn_st().current_col = found_m_endpos.col;
                            } else {
                                syn_st().current_col += 1;
                            }

                            let prev_current_col = syn_st().current_col;
                            if *syn_getcurline().offset(syn_st().current_col as isize) != NUL {
                                syn_st().current_col += 1;
                            }
                            check_state_ends();
                            syn_st().current_col = prev_current_col;
                        } else {
                            break;
                        }
                    }
                    syn_st().current_lnum += 1;
                }

                if found_flags != 0 {
                    clear_current_state();
                    if found_match_idx >= 0 {
                        push_current_state(found_match_idx);
                        update_si_attr(cur_state_len() - 1);
                    }

                    if (found_flags & HL_SYNC_HERE) != 0 {
                        syn_st().current_lnum = found_m_endpos.lnum;
                        syn_st().current_col = found_m_endpos.col;
                        if !cur_state_empty() {
                            let cur_si = cur_state_at(cur_state_len() - 1);
                            (*cur_si).si_h_startpos.lnum = found_current_lnum;
                            (*cur_si).si_h_startpos.col = found_current_col;
                            update_si_end(&mut *cur_si, syn_st().current_col, true);
                            check_keepend();
                        }
                        syn_finish_line(false);
                        syn_st().current_lnum += 1;
                    } else {
                        syn_st().current_lnum = start_lnum;
                    }

                    break;
                }

                end_lnum = lnum;
                invalidate_current_state();
            }

            if lnum <= break_lnum {
                invalidate_current_state();
                syn_st().current_lnum = break_lnum + 1;
            }
        }

        validate_current_state();
    }
}

fn save_chartab(chartab: *mut c_char) {
    unsafe {
        if (*syn_st().syn_block).b_syn_isk.is_none() {
            return;
        }

        ptr::copy(
            (&raw const (*syn_st().syn_buf).b_chartab).cast::<u8>(),
            chartab.cast::<u8>(),
            32,
        );
        ptr::copy(
            (&raw const (*win_s(&mut *syn_st().syn_win)).b_syn_chartab).cast::<u8>(),
            (&raw mut (*syn_st().syn_buf).b_chartab).cast::<u8>(),
            32,
        );
    }
}

fn restore_chartab(chartab: *const c_char) {
    unsafe {
        if (*win_s(&mut *syn_st().syn_win)).b_syn_isk.is_some() {
            ptr::copy(
                chartab.cast::<u8>(),
                (&raw mut (*syn_st().syn_buf).b_chartab).cast::<u8>(),
                32,
            );
        }
    }
}

fn syn_match_linecont(lnum: linenr_T) -> c_int {
    unsafe {
        if (*syn_st().syn_block).b_syn_linecont_prog.is_none() {
            return 0;
        }

        let mut regmatch: regmmatch_T = regmmatch_T::new();
        let mut buf_chartab = [0 as c_char; 32];
        save_chartab(buf_chartab.as_mut_ptr());

        regmatch.rmm_ic = (*syn_st().syn_block).b_syn_linecont_ic;
        regmatch.regprog = crate::regexp::regprog_as_ptr(&(*syn_st().syn_block).b_syn_linecont_prog);
        let r = syn_regexec(
            &mut regmatch,
            lnum,
            0,
            &mut (*syn_st().syn_block).b_syn_linecont_time,
        );

        restore_chartab(buf_chartab.as_ptr());
        c_int::from(r)
    }
}

fn syn_start_line() {
    syn_st().current_finished = false;
        syn_st().current_col = 0;

        if !cur_state_empty() {
            syn_update_ends(true);
            check_state_ends();
        }

        syn_st().next_match_idx = -1;
        syn_st().current_line_id += 1;
        syn_st().next_seqnr = 1;
}

fn syn_update_ends(startofline: bool) {
    unsafe {
        let len = cur_state_len();
        if startofline {
            for i in 0..len {
                let cur_si = cur_state_at(i);
                if (*cur_si).si_idx >= 0
                    && (*syn_items(&mut *syn_st().syn_block).offset((*cur_si).si_idx as isize)).sp_type
                        == SPTYPE_MATCH
                    && (*cur_si).si_m_endpos.lnum < syn_st().current_lnum
                {
                    (*cur_si).si_flags |= HL_MATCHCONT;
                    (*cur_si).si_m_endpos.lnum = 0;
                    (*cur_si).si_m_endpos.col = 0;
                    (*cur_si).si_h_endpos = (*cur_si).si_m_endpos;
                    (*cur_si).si_ends = 1;
                }
            }
        }

        let mut i = len - 1;
        if syn_st().keepend_level >= 0 {
            while i > syn_st().keepend_level {
                if ((*cur_state_at(i)).si_flags & HL_EXTEND) != 0 {
                    break;
                }
                i -= 1;
            }
        }

        let mut seen_keepend = false;
        while i < len {
            let cur_si = cur_state_at(i);
            if ((*cur_si).si_flags & HL_KEEPEND) != 0
                || (seen_keepend && !startofline)
                || (i == len - 1 && startofline)
            {
                (*cur_si).si_h_startpos.col = 0;
                (*cur_si).si_h_startpos.lnum = syn_st().current_lnum;

                if ((*cur_si).si_flags & HL_MATCHCONT) == 0 {
                    update_si_end(&mut *cur_si, syn_st().current_col, !startofline);
                }

                if !startofline && ((*cur_si).si_flags & HL_KEEPEND) != 0 {
                    seen_keepend = true;
                }
            }
            i += 1;
        }
        check_keepend();
    }
}

fn syn_stack_free_block(block: *mut synblock_T) {
    unsafe {
        idl_cache_clear(block);
        (&mut *block).b_sst_array = Vec::new();
        (*block).b_sst_max = 0;
        (*block).b_sst_search = 0;
    }
}

/// Free b_sst_array[] for buffer "buf".
pub fn syn_stack_free_all(block: *mut synblock_T) {
    unsafe {
        syn_stack_free_block(block);

        if !g_st().curtab.is_null() {
            let mut wp = g_st().firstwin;
            while !wp.is_null() {
                if win_s(&mut *wp) == block && foldmethodIsSyntax(&mut *wp) {
                    foldUpdateAll(&mut *wp);
                }
                wp = (*wp).w_next;
            }
        }
    }
}

fn syn_stack_alloc() {
    unsafe {
        let mut len = ((*syn_st().syn_buf).b_ml.ml_line_count / SST_DIST + g_st().Rows * 2)
            .clamp(SST_MIN_ENTRIES, SST_MAX_ENTRIES);
        let cur_len = (*syn_st().syn_block).b_sst_max;
        if cur_len > len * 2 || cur_len < len {
            len = (*syn_st().syn_buf).b_ml.ml_line_count;
            len = ((len + len / 2) / SST_DIST + g_st().Rows * 2).clamp(SST_MIN_ENTRIES, SST_MAX_ENTRIES);

            while ((*syn_st().syn_block).b_sst_array.len() as c_int) + 2 > len && syn_stack_cleanup() {}
            let needed = ((*syn_st().syn_block).b_sst_array.len() as c_int) + 2;
            if len < needed {
                len = needed;
            }

            (*syn_st().syn_block).b_sst_max = len;
            (*syn_st().syn_block).b_sst_search = 0;
        }
    }
}

/// Check for changes in a buffer to affect stored syntax states.
pub fn syn_stack_apply_changes(buf: &mut buf_T) {
    unsafe {
        let buf_ptr = &raw mut *buf;
        syn_stack_apply_changes_block(&mut buf.b_s, &mut *buf_ptr);

        if !g_st().curtab.is_null() {
            let mut wp = g_st().firstwin;
            while !wp.is_null() {
                if (*wp).w_buffer == buf && win_s(&mut *wp) != &raw mut buf.b_s {
                    syn_stack_apply_changes_block(&mut *win_s(&mut *wp), buf);
                }
                wp = (*wp).w_next;
            }
        }
    }
}

fn syn_stack_apply_changes_block(block: &mut synblock_T, buf: &mut buf_T) {
    let mut idx = 0;
        while idx < block.b_sst_array.len() {
            let p = &mut block.b_sst_array.as_mut_slice()[idx];
            if p.sst_lnum + block.b_syn_sync_linebreaks > buf.b_mod_top {
                let n = p.sst_lnum + buf.b_mod_xlines;
                if n <= buf.b_mod_bot {
                    if block.b_sst_search == idx {
                        block.b_sst_search = 0;
                    } else if block.b_sst_search > idx {
                        block.b_sst_search -= 1;
                    }
                    block.b_sst_array.remove(idx);
                    continue;
                }
                if p.sst_change_lnum != 0 && p.sst_change_lnum > buf.b_mod_top {
                    if p.sst_change_lnum + buf.b_mod_xlines > buf.b_mod_top {
                        p.sst_change_lnum += buf.b_mod_xlines;
                    } else {
                        p.sst_change_lnum = buf.b_mod_top;
                    }
                }
                if p.sst_change_lnum == 0 || p.sst_change_lnum < buf.b_mod_bot {
                    p.sst_change_lnum = buf.b_mod_bot;
                }

                p.sst_lnum = n;
            }
            idx += 1;
        }
}

fn syn_stack_cleanup() -> bool {
    unsafe {
        if (*syn_st().syn_block).b_sst_array.len() < 2 {
            return false;
        }

        let sst_len = (*syn_st().syn_block).b_sst_max;
        let dist = if sst_len <= g_st().Rows {
            999999
        } else {
            (*syn_st().syn_buf).b_ml.ml_line_count / (sst_len - g_st().Rows) + 1
        };

        let mut tick = (*syn_st().syn_block).b_sst_lasttick;
        let mut above = false;
        for w in (*syn_st().syn_block).b_sst_array.as_slice().windows(2) {
            let prev = &w[0];
            let p = &w[1];
            if prev.sst_lnum + dist > p.sst_lnum {
                if p.sst_tick > (*syn_st().syn_block).b_sst_lasttick {
                    if !above || p.sst_tick < tick {
                        tick = p.sst_tick;
                    }
                    above = true;
                } else if !above && p.sst_tick < tick {
                    tick = p.sst_tick;
                }
            }
        }

        let mut retval = false;
        let mut idx = 1;
        while idx < (*syn_st().syn_block).b_sst_array.len() {
            let prev_lnum = (*syn_st().syn_block).b_sst_array.as_slice()[idx - 1].sst_lnum;
            let p = &(*syn_st().syn_block).b_sst_array.as_slice()[idx];
            if p.sst_tick == tick && prev_lnum + dist > p.sst_lnum {
                if (*syn_st().syn_block).b_sst_search == idx {
                    (*syn_st().syn_block).b_sst_search = 0;
                } else if (*syn_st().syn_block).b_sst_search > idx {
                    (*syn_st().syn_block).b_sst_search -= 1;
                }
                (*syn_st().syn_block).b_sst_array.remove(idx);
                retval = true;
            } else {
                idx += 1;
            }
        }
        retval
    }
}

fn syn_stack_find_entry(lnum: linenr_T) -> Option<usize> {
    unsafe {
        let arr = (*syn_st().syn_block).b_sst_array.as_slice();
        if arr.is_empty() {
            return None;
        }

        let mut idx = 0;
        let search = (*syn_st().syn_block).b_sst_search;
        if search < arr.len() && arr[search].sst_lnum <= lnum {
            idx = search;
        }

        let mut prev: Option<usize> = None;
        while idx < arr.len() {
            if arr[idx].sst_lnum == lnum {
                (*syn_st().syn_block).b_sst_search = idx;
                return Some(idx);
            }
            if arr[idx].sst_lnum > lnum {
                break;
            }
            prev = Some(idx);
            idx += 1;
        }
        if let Some(p) = prev {
            (*syn_st().syn_block).b_sst_search = p;
        }
        prev
    }
}

fn store_current_state() -> Option<usize> {
    unsafe {
        let mut sp_idx = syn_stack_find_entry(syn_st().current_lnum);

        let mut i = cur_state_len() - 1;
        while i >= 0 {
            let cur_si = cur_state_at(i);
            if (*cur_si).si_h_startpos.lnum >= syn_st().current_lnum
                || (*cur_si).si_m_endpos.lnum >= syn_st().current_lnum
                || (*cur_si).si_h_endpos.lnum >= syn_st().current_lnum
                || ((*cur_si).si_end_idx != 0 && (*cur_si).si_eoe_pos.lnum >= syn_st().current_lnum)
            {
                break;
            }
            i -= 1;
        }
        if i >= 0 {
            if let Some(idx) = sp_idx {
                if (*syn_st().syn_block).b_sst_search == idx {
                    (*syn_st().syn_block).b_sst_search = 0;
                } else if (*syn_st().syn_block).b_sst_search > idx {
                    (*syn_st().syn_block).b_sst_search -= 1;
                }
                (*syn_st().syn_block).b_sst_array.remove(idx);
                sp_idx = None;
            }
        } else if sp_idx
            .is_none_or(|idx| (*syn_st().syn_block).b_sst_array.as_slice()[idx].sst_lnum != syn_st().current_lnum)
        {
            if ((*syn_st().syn_block).b_sst_array.len() as c_int) >= (*syn_st().syn_block).b_sst_max {
                syn_stack_cleanup();
                sp_idx = syn_stack_find_entry(syn_st().current_lnum);
            }
            if ((*syn_st().syn_block).b_sst_array.len() as c_int) >= (*syn_st().syn_block).b_sst_max {
                sp_idx = None;
            } else {
                let insert_idx = sp_idx.map_or(0, |idx| idx + 1);
                (*syn_st().syn_block).b_sst_array.insert(
                    insert_idx,
                    synstate_T {
                        sst_lnum: syn_st().current_lnum,
                        ..synstate_T::default()
                    },
                );
                (*syn_st().syn_block).b_sst_search = insert_idx;
                sp_idx = Some(insert_idx);
            }
        }
        if let Some(idx) = sp_idx {
            let sp = &mut (*syn_st().syn_block).b_sst_array.as_mut_slice()[idx];
            sp.sst_stack.clear();
            let len = cur_state_len();
            sp.sst_stack.reserve(len as usize);
            for j in 0..len {
                let cur_si = cur_state_at(j);
                sp.sst_stack.push(bufstate_T {
                    bs_idx: (*cur_si).si_idx,
                    bs_flags: (*cur_si).si_flags,
                    bs_seqnr: (*cur_si).si_seqnr,
                    bs_cchar: (*cur_si).si_cchar,
                    bs_extmatch: (*cur_si).si_extmatch.clone(),
                });
            }
            sp.sst_next_flags = syn_st().current_next_flags;
            sp.sst_next_list = syn_st().current_next_list.clone();
            sp.sst_tick = *display_tick.as_ptr();
            sp.sst_change_lnum = 0;
        }
        syn_st().current_state_stored = true;
        sp_idx
    }
}

fn load_current_state(from: *const synstate_T) {
    unsafe {
        clear_current_state();
        validate_current_state();
        syn_st().keepend_level = -1;
        let stacksize = (*from).sst_stack.len() as c_int;
        if stacksize != 0 {
            let cs_ptr = &raw mut syn_st().current_state;
            (*cs_ptr).reserve(stacksize as usize);
            for (i, b) in (*from).sst_stack.iter().enumerate() {
                let i_c = i as c_int;
                let mut item = stateitem_T::new();
                item.si_idx = b.bs_idx;
                item.si_flags = b.bs_flags;
                item.si_seqnr = b.bs_seqnr;
                item.si_cchar = b.bs_cchar;
                item.si_extmatch = b.bs_extmatch.clone();
                if syn_st().keepend_level < 0 && (item.si_flags & HL_KEEPEND) != 0 {
                    syn_st().keepend_level = i_c;
                }
                item.si_ends = 0;
                item.si_m_lnum = 0;
                if item.si_idx >= 0 {
                    item.si_next_list = (*syn_items(&mut *syn_st().syn_block).offset(item.si_idx as isize))
                        .sp_next_list
                        .clone();
                } else {
                    item.si_next_list = None;
                }
                (*cs_ptr).push(item);
                update_si_attr(i_c);
            }
        }
        syn_st().current_next_list = (*from).sst_next_list.clone();
        syn_st().current_next_flags = (*from).sst_next_flags;
        syn_st().current_lnum = (*from).sst_lnum;
    }
}

fn syn_stack_equal(sp: &synstate_T) -> bool {
    unsafe {
        if sp.sst_stack.len() as c_int != cur_state_len()
            || !syn_id_list_eq(
                &sp.sst_next_list,
                &syn_st().current_next_list,
            )
        {
            return false;
        }

        let bp = sp.sst_stack.as_ptr();

        let mut i = cur_state_len() - 1;
        while i >= 0 {
            let cur_si = cur_state_at(i);
            let b = &*bp.offset(i as isize);
            if b.bs_idx != (*cur_si).si_idx {
                break;
            }
            if match (&b.bs_extmatch, &(*cur_si).si_extmatch) {
                (None, None) => true,
                (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
                _ => false,
            } {
                i -= 1;
                continue;
            }
            let (Some(bsx), Some(six)) = (&b.bs_extmatch, &(*cur_si).si_extmatch) else {
                break;
            };
            let mut j: usize = 0;
            while j < NSUBEXP {
                if bsx.matches[j] != six.matches[j] {
                    let (Some(bs_m), Some(si_m)) = (&bsx.matches[j], &six.matches[j]) else {
                        break;
                    };
                    if mb_strcmp_ic(
                        (*syn_items(&mut *syn_st().syn_block).offset((*cur_si).si_idx as isize)).sp_ic != 0,
                        bs_m.as_ptr(),
                        si_m.as_ptr(),
                    ) != 0
                    {
                        break;
                    }
                }
                j += 1;
            }
            if j != NSUBEXP {
                break;
            }
            i -= 1;
        }
        i < 0
    }
}

/// We stop parsing syntax above line "lnum".
pub fn syntax_end_parsing(wp: &mut win_T, lnum: linenr_T) {
    unsafe {
        if syn_st().syn_block != win_s(wp) {
            return;
        }
        if let Some(mut sp_idx) = syn_stack_find_entry(lnum) {
            if (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_lnum < lnum {
                sp_idx += 1;
            }

            if sp_idx < (*syn_st().syn_block).b_sst_array.len()
                && (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_change_lnum != 0
            {
                (*syn_st().syn_block).b_sst_array.as_mut_slice()[sp_idx].sst_change_lnum = lnum;
            }
        }
    }
}

fn invalidate_current_state() {
    clear_current_state();
        syn_st().current_state_valid = false;
        syn_st().current_next_list = None;
        syn_st().keepend_level = -1;
}

fn validate_current_state() {
    syn_st().current_state_valid = true;
}

/// Check if the syntax at start of lnum changed since last time.
pub fn syntax_check_changed(lnum: linenr_T) -> bool {
    unsafe {
        let mut retval = true;

        if valid_state()
            && lnum == syn_st().current_lnum + 1
            && let Some(sp_idx) = syn_stack_find_entry(lnum)
            && (*syn_st().syn_block).b_sst_array.as_slice()[sp_idx].sst_lnum == lnum
        {
            syn_finish_line(false);

            if syn_stack_equal(&(*syn_st().syn_block).b_sst_array.as_slice()[sp_idx]) {
                retval = false;
            }

            syn_st().current_lnum += 1;
            store_current_state();
        }

        retval
    }
}

fn syn_finish_line(syncing: bool) -> bool {
    unsafe {
        while !syn_st().current_finished {
            syn_current_attr(syncing, false, ptr::null_mut(), false);

            if syncing && !cur_state_empty() {
                let cur_si = cur_state_at(cur_state_len() - 1);
                if (*cur_si).si_idx >= 0
                    && ((*syn_items(&mut *syn_st().syn_block).offset((*cur_si).si_idx as isize)).sp_flags
                        & (HL_SYNC_HERE | HL_SYNC_THERE))
                        != 0
                {
                    return true;
                }

                let prev_current_col = syn_st().current_col;
                if *syn_getcurline().offset(syn_st().current_col as isize) != NUL {
                    syn_st().current_col += 1;
                }
                check_state_ends();
                syn_st().current_col = prev_current_col;
            }
            syn_st().current_col += 1;
        }
        false
    }
}

/// Gets highlight attributes for next character.
pub fn get_syntax_attr(col: colnr_T, can_spell: *mut bool, keep_state: bool) -> c_int {
    unsafe {
        let mut attr: c_int = 0;

        if !can_spell.is_null() {
            *can_spell = if (*syn_st().syn_block).b_syn_spell == SYNSPL_DEFAULT {
                (*syn_st().syn_block).b_spell_cluster_id == 0
            } else {
                (*syn_st().syn_block).b_syn_spell == SYNSPL_TOP
            };
        }

        if (*syn_st().syn_block).b_sst_max == 0 {
            return 0;
        }

        if (*syn_st().syn_buf).b_p_smc > 0 && col >= (*syn_st().syn_buf).b_p_smc as colnr_T {
            clear_current_state();
            syn_st().current_id = 0;
            syn_st().current_trans_id = 0;
            syn_st().current_flags = 0;
            syn_st().current_seqnr = 0;
            return 0;
        }

        if invalid_state() {
            validate_current_state();
        }

        while syn_st().current_col <= col {
            attr = syn_current_attr(
                false,
                true,
                can_spell,
                if syn_st().current_col == col {
                    keep_state
                } else {
                    false
                },
            );
            syn_st().current_col += 1;
        }

        attr
    }
}

fn syn_current_attr(
    syncing: bool,
    displaying: bool,
    can_spell: *mut bool,
    keep_state: bool,
) -> c_int {
    unsafe {
        let mut endpos = lpos_T { lnum: 0, col: 0 };
        let mut hl_startpos = lpos_T { lnum: 0, col: 0 };
        let mut hl_endpos = lpos_T { lnum: 0, col: 0 };
        let mut eos_pos = lpos_T { lnum: 0, col: 0 };
        let mut eoe_pos = lpos_T { lnum: 0, col: 0 };
        let mut end_idx: c_int;
        let mut cur_si: *mut stateitem_T;
        let mut sip: *mut stateitem_T = ptr::null_mut();
        let mut startcol: c_int;
        let mut endcol: c_int = 0;
        let mut flags: c_int = 0;
        let mut cchar: c_int = 0;
        let mut next_list: SynIdList = None;
        let mut found_match: bool;
        let mut regmatch: regmmatch_T = regmmatch_T::new();
        let mut pos = lpos_T { lnum: 0, col: 0 };
        let mut cur_extmatch: Option<std::rc::Rc<reg_extmatch_T>> = None;
        let mut buf_chartab = [0 as c_char; 32];

        let mut keep_next_list: bool;
        let mut zero_width_next_list = false;
        let mut zero_width_next_ga: Vec<c_int> = Vec::new();

        let mut line = syn_getcurline();
        if *line.offset(syn_st().current_col as isize) == NUL && syn_st().current_col != 0 {
            if syn_st().next_match_idx >= 0 && syn_st().next_match_col >= syn_st().current_col && syn_st().next_match_col != MAXCOL {
                push_next_match();
            }

            syn_st().current_finished = true;
            syn_st().current_state_stored = false;
            return 0;
        }

        if *line.offset(syn_st().current_col as isize) == NUL
            || *line.offset((syn_st().current_col + 1) as isize) == NUL
        {
            syn_st().current_finished = true;
            syn_st().current_state_stored = false;
        }

        if syn_st().try_next_column {
            syn_st().next_match_idx = -1;
            syn_st().try_next_column = false;
        }

        let do_keywords = !syncing
            && (!(*syn_st().syn_block).b_keywtab.is_empty() || !(*syn_st().syn_block).b_keywtab_ic.is_empty());

        save_chartab(buf_chartab.as_mut_ptr());

        loop {
            found_match = false;
            keep_next_list = false;
            let mut syn_id: c_int = 0;

            if !cur_state_empty() {
                cur_si = cur_state_at(cur_state_len() - 1);
            } else {
                cur_si = ptr::null_mut();
            }

            if (*syn_st().syn_block).b_syn_containedin != 0
                || cur_si.is_null()
                || (*cur_si).si_cont_list.is_some()
            {
                if do_keywords {
                    line = syn_getcurline();
                    let cur_pos = line.offset(syn_st().current_col as isize);
                    if vim_iswordp_buf(cur_pos, syn_st().syn_buf)
                        && (syn_st().current_col == 0
                            || !vim_iswordp_buf(
                                cur_pos
                                    .sub(1)
                                    .sub(utf_head_off(line, cur_pos.sub(1)) as usize),
                                syn_st().syn_buf,
                            ))
                    {
                        syn_id = check_keyword_id(
                            line,
                            syn_st().current_col,
                            &mut endcol,
                            &mut flags,
                            &mut next_list,
                            cur_si,
                            &mut cchar,
                        );
                        if syn_id != 0 {
                            push_current_state(KEYWORD_IDX);
                            let len = cur_state_len();
                            cur_si = cur_state_at(len - 1);
                            (*cur_si).si_m_startcol = syn_st().current_col;
                            (*cur_si).si_h_startpos.lnum = syn_st().current_lnum;
                            (*cur_si).si_h_startpos.col = 0;
                            (*cur_si).si_m_endpos.lnum = syn_st().current_lnum;
                            (*cur_si).si_m_endpos.col = endcol;
                            (*cur_si).si_h_endpos.lnum = syn_st().current_lnum;
                            (*cur_si).si_h_endpos.col = endcol;
                            (*cur_si).si_ends = 1;
                            (*cur_si).si_end_idx = 0;
                            (*cur_si).si_flags = flags;
                            (*cur_si).si_seqnr = syn_st().next_seqnr;
                            syn_st().next_seqnr += 1;
                            (*cur_si).si_cchar = cchar;
                            if len > 1 {
                                (*cur_si).si_flags |=
                                    (*cur_state_at(len - 2)).si_flags & HL_CONCEAL;
                            }
                            (*cur_si).si_id = syn_id;
                            (*cur_si).si_trans_id = syn_id;
                            if (flags & HL_TRANSP) != 0 {
                                if len < 2 {
                                    (*cur_si).si_attr = 0;
                                    (*cur_si).si_trans_id = 0;
                                } else {
                                    (*cur_si).si_attr = (*cur_state_at(len - 2)).si_attr;
                                    (*cur_si).si_trans_id = (*cur_state_at(len - 2)).si_trans_id;
                                }
                            } else {
                                (*cur_si).si_attr = syn_id2attr(syn_id);
                            }
                            (*cur_si).si_cont_list = None;
                            (*cur_si).si_next_list = next_list.take();
                            check_keepend();
                        }
                    }
                }

                if syn_id == 0 && !(*syn_st().syn_block).b_syn_patterns.is_empty() {
                    if syn_st().next_match_idx < 0 || syn_st().next_match_col < syn_st().current_col {
                        syn_st().next_match_idx = 0;
                        syn_st().next_match_col = MAXCOL;
                        let mut idx = (*syn_st().syn_block).b_syn_patterns.len() as c_int - 1;
                        while idx >= 0 {
                            let spp = syn_items(&mut *syn_st().syn_block).offset(idx as isize);
                            let cur_next_list = &syn_st().current_next_list;
                            if (*spp).sp_syncing == syncing
                                && (displaying || ((*spp).sp_flags & HL_DISPLAY) == 0)
                                && ((*spp).sp_type == SPTYPE_MATCH
                                    || (*spp).sp_type == SPTYPE_START)
                                && if cur_next_list.is_some() {
                                    in_id_list(
                                        ptr::null_mut(),
                                        cur_next_list,
                                        &mut (*spp).sp_syn,
                                        0,
                                    ) != 0
                                } else if cur_si.is_null() {
                                    ((*spp).sp_flags & HL_CONTAINED) == 0
                                } else {
                                    in_id_list(
                                        cur_si,
                                        &(*cur_si).si_cont_list,
                                        &mut (*spp).sp_syn,
                                        (*spp).sp_flags,
                                    ) != 0
                                }
                            {
                                if (*spp).sp_line_id == syn_st().current_line_id
                                    && (*spp).sp_startcol >= syn_st().next_match_col
                                {
                                    idx -= 1;
                                    continue;
                                }
                                (*spp).sp_line_id = syn_st().current_line_id;

                                let mut lc_col = syn_st().current_col - (*spp).sp_offsets[SPO_LC_OFF];
                                if lc_col < 0 {
                                    lc_col = 0;
                                }

                                regmatch.rmm_ic = (*spp).sp_ic;
                                regmatch.regprog = crate::regexp::regprog_as_ptr(&(*spp).sp_prog);
                                let r = syn_regexec(
                                    &mut regmatch,
                                    syn_st().current_lnum,
                                    lc_col,
                                    &mut (*spp).sp_time,
                                );
                                if !r {
                                    (*spp).sp_startcol = MAXCOL;
                                    idx -= 1;
                                    continue;
                                }

                                syn_add_start_off(
                                    &mut pos,
                                    &mut regmatch,
                                    &mut *spp,
                                    SPO_MS_OFF,
                                    -1,
                                );
                                if pos.lnum > syn_st().current_lnum {
                                    (*spp).sp_startcol = MAXCOL;
                                    idx -= 1;
                                    continue;
                                }
                                startcol = pos.col;
                                (*spp).sp_startcol = startcol;

                                if startcol >= syn_st().next_match_col {
                                    idx -= 1;
                                    continue;
                                }

                                if did_match_already(idx, &zero_width_next_ga) {
                                    syn_st().try_next_column = true;
                                    idx -= 1;
                                    continue;
                                }

                                endpos.lnum = regmatch.endpos[0].lnum;
                                endpos.col = regmatch.endpos[0].col;

                                syn_add_start_off(
                                    &mut hl_startpos,
                                    &mut regmatch,
                                    &mut *spp,
                                    SPO_HS_OFF,
                                    -1,
                                );
                                syn_add_end_off(
                                    &mut eos_pos,
                                    &mut regmatch,
                                    &mut *spp,
                                    SPO_RS_OFF,
                                    0,
                                );

                                cur_extmatch = (&raw mut g_st().re_extmatch_out).replace(None);

                                flags = 0;
                                eoe_pos.lnum = 0;
                                eoe_pos.col = 0;
                                end_idx = 0;
                                hl_endpos.lnum = 0;

                                if (*spp).sp_type == SPTYPE_START
                                    && ((*spp).sp_flags & HL_ONELINE) != 0
                                {
                                    let mut startpos = endpos;
                                    find_endpos(
                                        idx,
                                        &mut startpos,
                                        &mut endpos,
                                        &mut hl_endpos,
                                        &mut flags,
                                        &mut eoe_pos,
                                        &mut end_idx,
                                        cur_extmatch.clone(),
                                    );
                                    if endpos.lnum == 0 {
                                        idx -= 1;
                                        continue;
                                    }
                                } else if (*spp).sp_type == SPTYPE_MATCH {
                                    syn_add_end_off(
                                        &mut hl_endpos,
                                        &mut regmatch,
                                        &mut *spp,
                                        SPO_HE_OFF,
                                        0,
                                    );
                                    syn_add_end_off(
                                        &mut endpos,
                                        &mut regmatch,
                                        &mut *spp,
                                        SPO_ME_OFF,
                                        0,
                                    );
                                    if endpos.lnum == syn_st().current_lnum
                                        && endpos.col + c_int::from(syncing) < startcol
                                    {
                                        if regmatch.startpos[0].col == regmatch.endpos[0].col {
                                            syn_st().try_next_column = true;
                                        }
                                        idx -= 1;
                                        continue;
                                    }
                                }

                                if hl_startpos.lnum == syn_st().current_lnum && hl_startpos.col < startcol {
                                    hl_startpos.col = startcol;
                                }
                                limit_pos_zero(&mut hl_endpos, &mut endpos);

                                syn_st().next_match_idx = idx;
                                syn_st().next_match_col = startcol;
                                syn_st().next_match_m_endpos = endpos;
                                syn_st().next_match_h_endpos = hl_endpos;
                                syn_st().next_match_h_startpos = hl_startpos;
                                syn_st().next_match_flags = flags;
                                syn_st().next_match_eos_pos = eos_pos;
                                syn_st().next_match_eoe_pos = eoe_pos;
                                syn_st().next_match_end_idx = end_idx;
                                syn_st().next_match_extmatch = cur_extmatch.take();
                            }
                            idx -= 1;
                        }
                    }

                    if syn_st().next_match_idx >= 0 && syn_st().next_match_col == syn_st().current_col {
                        let lspp = syn_items(&mut *syn_st().syn_block).offset(syn_st().next_match_idx as isize);
                        if syn_st().next_match_m_endpos.lnum == syn_st().current_lnum
                            && syn_st().next_match_m_endpos.col == syn_st().current_col
                            && (*lspp).sp_next_list.is_some()
                        {
                            syn_st().current_next_list = (*lspp).sp_next_list.clone();
                            syn_st().current_next_flags = (*lspp).sp_flags;
                            keep_next_list = true;
                            zero_width_next_list = true;

                            zero_width_next_ga.push(syn_st().next_match_idx);
                            syn_st().next_match_idx = -1;
                        } else {
                            cur_si = push_next_match();
                        }
                        found_match = true;
                    }
                }
            }

            if syn_st().current_next_list.is_some() && !keep_next_list {
                if !found_match {
                    line = syn_getcurline();
                    if ((syn_st().current_next_flags & HL_SKIPWHITE) != 0
                        && ascii_iswhite(*line.offset(syn_st().current_col as isize)))
                        || ((syn_st().current_next_flags & HL_SKIPEMPTY) != 0 && *line == NUL)
                    {
                        break;
                    }
                }

                syn_st().current_next_list = None;
                syn_st().next_match_idx = -1;
                if !zero_width_next_list {
                    found_match = true;
                }
            }

            if !found_match {
                break;
            }
        }

        restore_chartab(buf_chartab.as_ptr());

        syn_st().current_attr = 0;
        syn_st().current_id = 0;
        syn_st().current_trans_id = 0;
        syn_st().current_flags = 0;
        syn_st().current_seqnr = 0;
        if !cur_si.is_null() {
            let mut idx = cur_state_len() - 1;
            while idx >= 0 {
                sip = cur_state_at(idx);
                if (syn_st().current_lnum > (*sip).si_h_startpos.lnum
                    || (syn_st().current_lnum == (*sip).si_h_startpos.lnum
                        && syn_st().current_col >= (*sip).si_h_startpos.col))
                    && ((*sip).si_h_endpos.lnum == 0
                        || syn_st().current_lnum < (*sip).si_h_endpos.lnum
                        || (syn_st().current_lnum == (*sip).si_h_endpos.lnum
                            && syn_st().current_col < (*sip).si_h_endpos.col))
                {
                    syn_st().current_attr = (*sip).si_attr;
                    syn_st().current_id = (*sip).si_id;
                    syn_st().current_trans_id = (*sip).si_trans_id;
                    syn_st().current_flags = (*sip).si_flags;
                    syn_st().current_seqnr = (*sip).si_seqnr;
                    syn_st().current_sub_char = (*sip).si_cchar;
                    break;
                }
                idx -= 1;
            }

            if !can_spell.is_null() {
                let mut sps = sp_syn {
                    inc_tag: 0,
                    id: 0,
                    cont_in_list: None,
                };

                if (*syn_st().syn_block).b_spell_cluster_id == 0 {
                    if (*syn_st().syn_block).b_nospell_cluster_id == 0 || syn_st().current_trans_id == 0 {
                        *can_spell = (*syn_st().syn_block).b_syn_spell != SYNSPL_NOTOP;
                    } else {
                        sps.id = (*syn_st().syn_block).b_nospell_cluster_id as i16;
                        *can_spell = in_id_list(sip, &(*sip).si_cont_list, &mut sps, 0) == 0;
                    }
                } else if syn_st().current_trans_id == 0 {
                    *can_spell = (*syn_st().syn_block).b_syn_spell == SYNSPL_TOP;
                } else {
                    sps.id = (*syn_st().syn_block).b_spell_cluster_id as i16;
                    *can_spell = in_id_list(sip, &(*sip).si_cont_list, &mut sps, 0) != 0;

                    if (*syn_st().syn_block).b_nospell_cluster_id != 0 {
                        sps.id = (*syn_st().syn_block).b_nospell_cluster_id as i16;
                        if in_id_list(sip, &(*sip).si_cont_list, &mut sps, 0) != 0 {
                            *can_spell = false;
                        }
                    }
                }
            }

            if !syncing && !keep_state {
                check_state_ends();
                if !cur_state_empty() && *syn_getcurline().offset(syn_st().current_col as isize) != NUL {
                    syn_st().current_col += 1;
                    check_state_ends();
                    syn_st().current_col -= 1;
                }
            }
        } else if !can_spell.is_null() {
            *can_spell = if (*syn_st().syn_block).b_syn_spell == SYNSPL_DEFAULT {
                (*syn_st().syn_block).b_spell_cluster_id == 0
            } else {
                (*syn_st().syn_block).b_syn_spell == SYNSPL_TOP
            };
        }

        if syn_st().current_next_list.is_some() {
            line = syn_getcurline();
            if *line.offset(syn_st().current_col as isize) != NUL
                && *line.offset((syn_st().current_col + 1) as isize) == NUL
                && (syn_st().current_next_flags & (HL_SKIPNL | HL_SKIPEMPTY)) == 0
            {
                syn_st().current_next_list = None;
            }
        }

        drop(zero_width_next_ga);

        g_st().re_extmatch_out = None;
        drop(cur_extmatch);

        syn_st().current_attr
    }
}

fn did_match_already(idx: c_int, gap: &[c_int]) -> bool {
    unsafe {
        let mut i = cur_state_len() - 1;
        while i >= 0 {
            let cur_si = cur_state_at(i);
            if (*cur_si).si_m_startcol == syn_st().current_col
                && (*cur_si).si_m_lnum == syn_st().current_lnum
                && (*cur_si).si_idx == idx
            {
                return true;
            }
            i -= 1;
        }

        gap.contains(&idx)
    }
}

fn push_next_match() -> *mut stateitem_T {
    unsafe {
        let spp = syn_items(&mut *syn_st().syn_block).offset(syn_st().next_match_idx as isize);

        push_current_state(syn_st().next_match_idx);
        let len = cur_state_len();
        let mut cur_si = cur_state_at(len - 1);
        (*cur_si).si_h_startpos = syn_st().next_match_h_startpos;
        (*cur_si).si_m_startcol = syn_st().current_col;
        (*cur_si).si_m_lnum = syn_st().current_lnum;
        (*cur_si).si_flags = (*spp).sp_flags;
        (*cur_si).si_seqnr = syn_st().next_seqnr;
        syn_st().next_seqnr += 1;
        (*cur_si).si_cchar = (*spp).sp_cchar;
        if len > 1 {
            (*cur_si).si_flags |= (*cur_state_at(len - 2)).si_flags & HL_CONCEAL;
        }
        (*cur_si).si_next_list = (*spp).sp_next_list.clone();
        let next_ext = &raw const syn_st().next_match_extmatch;
        (*cur_si).si_extmatch = (*next_ext).clone();
        if (*spp).sp_type == SPTYPE_START && ((*spp).sp_flags & HL_ONELINE) == 0 {
            update_si_end(&mut *cur_si, syn_st().next_match_m_endpos.col, true);
            check_keepend();
        } else {
            (*cur_si).si_m_endpos = syn_st().next_match_m_endpos;
            (*cur_si).si_h_endpos = syn_st().next_match_h_endpos;
            (*cur_si).si_ends = 1;
            (*cur_si).si_flags |= syn_st().next_match_flags;
            (*cur_si).si_eoe_pos = syn_st().next_match_eoe_pos;
            (*cur_si).si_end_idx = syn_st().next_match_end_idx;
        }
        if syn_st().keepend_level < 0 && ((*cur_si).si_flags & HL_KEEPEND) != 0 {
            syn_st().keepend_level = len - 1;
        }
        check_keepend();
        update_si_attr(len - 1);

        let save_flags = (*cur_si).si_flags & (HL_CONCEAL | HL_CONCEALENDS);
        if (*spp).sp_type == SPTYPE_START && (*spp).sp_syn_match_id != 0 {
            push_current_state(syn_st().next_match_idx);
            let len2 = cur_state_len();
            cur_si = cur_state_at(len2 - 1);
            (*cur_si).si_h_startpos = syn_st().next_match_h_startpos;
            (*cur_si).si_m_startcol = syn_st().current_col;
            (*cur_si).si_m_lnum = syn_st().current_lnum;
            (*cur_si).si_m_endpos = syn_st().next_match_eos_pos;
            (*cur_si).si_h_endpos = syn_st().next_match_eos_pos;
            (*cur_si).si_ends = 1;
            (*cur_si).si_end_idx = 0;
            (*cur_si).si_flags = HL_MATCH;
            (*cur_si).si_seqnr = syn_st().next_seqnr;
            syn_st().next_seqnr += 1;
            (*cur_si).si_flags |= save_flags;
            if ((*cur_si).si_flags & HL_CONCEALENDS) != 0 {
                (*cur_si).si_flags |= HL_CONCEAL;
            }
            (*cur_si).si_next_list = None;
            check_keepend();
            update_si_attr(len2 - 1);
        }

        syn_st().next_match_idx = -1;
        cur_si
    }
}

fn check_state_ends() {
    unsafe {
        let mut cur_si = cur_state_at(cur_state_len() - 1);
        loop {
            if (*cur_si).si_ends != 0
                && ((*cur_si).si_m_endpos.lnum < syn_st().current_lnum
                    || ((*cur_si).si_m_endpos.lnum == syn_st().current_lnum
                        && (*cur_si).si_m_endpos.col <= syn_st().current_col))
            {
                if (*cur_si).si_end_idx != 0
                    && ((*cur_si).si_eoe_pos.lnum > syn_st().current_lnum
                        || ((*cur_si).si_eoe_pos.lnum == syn_st().current_lnum
                            && (*cur_si).si_eoe_pos.col > syn_st().current_col))
                {
                    (*cur_si).si_idx = (*cur_si).si_end_idx;
                    (*cur_si).si_end_idx = 0;
                    (*cur_si).si_m_endpos = (*cur_si).si_eoe_pos;
                    (*cur_si).si_h_endpos = (*cur_si).si_eoe_pos;
                    (*cur_si).si_flags |= HL_MATCH;
                    (*cur_si).si_seqnr = syn_st().next_seqnr;
                    syn_st().next_seqnr += 1;
                    if ((*cur_si).si_flags & HL_CONCEALENDS) != 0 {
                        (*cur_si).si_flags |= HL_CONCEAL;
                    }
                    update_si_attr(cur_state_len() - 1);

                    syn_st().current_next_list = None;
                    syn_st().next_match_idx = 0;
                    syn_st().next_match_col = MAXCOL;
                    break;
                }

                syn_st().current_next_list = (*cur_si).si_next_list.clone();
                syn_st().current_next_flags = (*cur_si).si_flags;
                if (syn_st().current_next_flags & (HL_SKIPNL | HL_SKIPEMPTY)) == 0
                    && *syn_getcurline().offset(syn_st().current_col as isize) == NUL
                {
                    syn_st().current_next_list = None;
                }

                let had_extend = ((*cur_si).si_flags & HL_EXTEND) != 0;

                pop_current_state();

                if cur_state_empty() {
                    break;
                }

                if had_extend && syn_st().keepend_level >= 0 {
                    syn_update_ends(false);
                    if cur_state_empty() {
                        break;
                    }
                }

                cur_si = cur_state_at(cur_state_len() - 1);

                if (*cur_si).si_idx >= 0
                    && (*syn_items(&mut *syn_st().syn_block).offset((*cur_si).si_idx as isize)).sp_type
                        == SPTYPE_START
                    && ((*cur_si).si_flags & (HL_MATCH | HL_KEEPEND)) == 0
                {
                    update_si_end(&mut *cur_si, syn_st().current_col, true);
                    check_keepend();
                    if (syn_st().current_next_flags & HL_HAS_EOL) != 0
                        && syn_st().keepend_level < 0
                        && *syn_getcurline().offset(syn_st().current_col as isize) == NUL
                    {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }
}

fn update_si_attr(idx: c_int) {
    unsafe {
        let sip = cur_state_at(idx);
        if (*sip).si_idx < 0 {
            return;
        }

        let spp = syn_items(&mut *syn_st().syn_block).offset((*sip).si_idx as isize);
        if ((*sip).si_flags & HL_MATCH) != 0 {
            (*sip).si_id = (*spp).sp_syn_match_id as c_int;
        } else {
            (*sip).si_id = (*spp).sp_syn.id as c_int;
        }
        (*sip).si_attr = syn_id2attr((*sip).si_id);
        (*sip).si_trans_id = (*sip).si_id;
        if ((*sip).si_flags & HL_MATCH) != 0 {
            (*sip).si_cont_list = None;
        } else {
            (*sip).si_cont_list = (*spp).sp_cont_list.clone();
        }

        if ((*spp).sp_flags & HL_TRANSP) != 0 && ((*sip).si_flags & HL_MATCH) == 0 {
            if idx == 0 {
                (*sip).si_attr = 0;
                (*sip).si_trans_id = 0;
                if (*sip).si_cont_list.is_none() {
                    (*sip).si_cont_list = id_list_all();
                }
            } else {
                let prev_si = cur_state_at(idx - 1);
                (*sip).si_attr = (*prev_si).si_attr;
                (*sip).si_trans_id = (*prev_si).si_trans_id;
                if (*sip).si_cont_list.is_none() {
                    (*sip).si_flags |= HL_TRANS_CONT;
                    (*sip).si_cont_list = (*prev_si).si_cont_list.clone();
                }
            }
        }
    }
}

fn check_keepend() {
    unsafe {
        if syn_st().keepend_level < 0 {
            return;
        }

        let len = cur_state_len();
        let mut i = len - 1;
        while i > syn_st().keepend_level {
            if ((*cur_state_at(i)).si_flags & HL_EXTEND) != 0 {
                break;
            }
            i -= 1;
        }

        let mut maxpos = lpos_T { lnum: 0, col: 0 };
        let mut maxpos_h = lpos_T { lnum: 0, col: 0 };
        while i < len {
            let sip = cur_state_at(i);
            if maxpos.lnum != 0 {
                limit_pos_zero(&mut (*sip).si_m_endpos, &mut maxpos);
                limit_pos_zero(&mut (*sip).si_h_endpos, &mut maxpos_h);
                limit_pos_zero(&mut (*sip).si_eoe_pos, &mut maxpos);
                (*sip).si_ends = 1;
            }
            if (*sip).si_ends != 0 && ((*sip).si_flags & HL_KEEPEND) != 0 {
                if maxpos.lnum == 0
                    || maxpos.lnum > (*sip).si_m_endpos.lnum
                    || (maxpos.lnum == (*sip).si_m_endpos.lnum
                        && maxpos.col > (*sip).si_m_endpos.col)
                {
                    maxpos = (*sip).si_m_endpos;
                }
                if maxpos_h.lnum == 0
                    || maxpos_h.lnum > (*sip).si_h_endpos.lnum
                    || (maxpos_h.lnum == (*sip).si_h_endpos.lnum
                        && maxpos_h.col > (*sip).si_h_endpos.col)
                {
                    maxpos_h = (*sip).si_h_endpos;
                }
            }
            i += 1;
        }
    }
}

fn update_si_end(sip: &mut stateitem_T, startcol: c_int, force: bool) {
    unsafe {
        if sip.si_idx < 0 {
            return;
        }

        if !force && sip.si_m_endpos.lnum >= syn_st().current_lnum {
            return;
        }

        let mut end_idx: c_int = 0;
        let mut startpos = lpos_T {
            lnum: syn_st().current_lnum,
            col: startcol,
        };
        let mut endpos = lpos_T { lnum: 0, col: 0 };
        let mut hl_endpos = lpos_T { lnum: 0, col: 0 };
        let mut end_endpos = lpos_T { lnum: 0, col: 0 };

        find_endpos(
            sip.si_idx,
            &mut startpos,
            &mut endpos,
            &mut hl_endpos,
            &mut sip.si_flags,
            &mut end_endpos,
            &mut end_idx,
            sip.si_extmatch.clone(),
        );

        if endpos.lnum == 0 {
            if ((*syn_items(&mut *syn_st().syn_block).offset(sip.si_idx as isize)).sp_flags & HL_ONELINE) != 0 {
                sip.si_ends = 1;
                sip.si_m_endpos.lnum = syn_st().current_lnum;
                sip.si_m_endpos.col = syn_getcurline_len();
            } else {
                sip.si_ends = 0;
                sip.si_m_endpos.lnum = 0;
            }
            sip.si_h_endpos = sip.si_m_endpos;
        } else {
            sip.si_m_endpos = endpos;
            sip.si_h_endpos = hl_endpos;
            sip.si_eoe_pos = end_endpos;
            sip.si_ends = 1;
            sip.si_end_idx = end_idx;
        }
    }
}

fn push_current_state(idx: c_int) {
    unsafe {
        let mut item = stateitem_T::new();
        item.si_idx = idx;
        let cs_ptr = &raw mut syn_st().current_state;
        (*cs_ptr).push(item);
    }
}

fn pop_current_state() {
    unsafe {
        let cs_ptr = &raw mut syn_st().current_state;
        let _ = (*cs_ptr).pop();
        syn_st().next_match_idx = -1;

        if syn_st().keepend_level >= (*cs_ptr).len() as c_int {
            syn_st().keepend_level = -1;
        }
    }
}

fn find_endpos(
    mut idx: c_int,
    startpos: &mut lpos_T,
    m_endpos: &mut lpos_T,
    hl_endpos: &mut lpos_T,
    flagsp: &mut c_int,
    end_endpos: &mut lpos_T,
    end_idx: &mut c_int,
    start_ext: Option<std::rc::Rc<reg_extmatch_T>>,
) {
    unsafe {
        if idx < 0 {
            return;
        }

        let mut spp = syn_items(&mut *syn_st().syn_block).offset(idx as isize);
        if (*spp).sp_type != SPTYPE_START {
            *hl_endpos = *startpos;
            return;
        }

        loop {
            spp = syn_items(&mut *syn_st().syn_block).offset(idx as isize);
            if (*spp).sp_type != SPTYPE_START {
                break;
            }
            idx += 1;
        }

        let spp_skip = if (*spp).sp_type == SPTYPE_SKIP {
            idx += 1;
            spp
        } else {
            ptr::null_mut()
        };

        g_st().re_extmatch_in = start_ext;

        let mut matchcol = startpos.col;
        let start_idx = idx;
        let mut regmatch: regmmatch_T = regmmatch_T::new();
        let mut best_regmatch: regmmatch_T = regmmatch_T::new();
        let mut pos = lpos_T { lnum: 0, col: 0 };
        let mut had_match = false;
        let mut buf_chartab = [0 as c_char; 32];

        save_chartab(buf_chartab.as_mut_ptr());

        loop {
            let mut best_idx: c_int = -1;
            idx = start_idx;
            while (idx as usize) < (*syn_st().syn_block).b_syn_patterns.len() {
                let mut lc_col = matchcol;
                spp = syn_items(&mut *syn_st().syn_block).offset(idx as isize);
                if (*spp).sp_type != SPTYPE_END {
                    break;
                }
                lc_col -= (*spp).sp_offsets[SPO_LC_OFF];
                if lc_col < 0 {
                    lc_col = 0;
                }

                regmatch.rmm_ic = (*spp).sp_ic;
                regmatch.regprog = crate::regexp::regprog_as_ptr(&(*spp).sp_prog);
                let r = syn_regexec(
                    &mut regmatch,
                    startpos.lnum,
                    lc_col,
                    &mut (*spp).sp_time,
                );
                if r && (best_idx == -1 || regmatch.startpos[0].col < best_regmatch.startpos[0].col)
                {
                    best_idx = idx;
                    best_regmatch.startpos[0] = regmatch.startpos[0];
                    best_regmatch.endpos[0] = regmatch.endpos[0];
                }
                idx += 1;
            }

            if best_idx == -1 {
                break;
            }

            if !spp_skip.is_null() {
                let mut lc_col = matchcol - (*spp_skip).sp_offsets[SPO_LC_OFF];
                if lc_col < 0 {
                    lc_col = 0;
                }
                regmatch.rmm_ic = (*spp_skip).sp_ic;
                regmatch.regprog = crate::regexp::regprog_as_ptr(&(*spp_skip).sp_prog);
                let r = syn_regexec(
                    &mut regmatch,
                    startpos.lnum,
                    lc_col,
                    &mut (*spp_skip).sp_time,
                );
                if r && regmatch.startpos[0].col <= best_regmatch.startpos[0].col {
                    syn_add_end_off(&mut pos, &mut regmatch, &mut *spp_skip, SPO_ME_OFF, 1);

                    if pos.lnum > startpos.lnum {
                        break;
                    }

                    let line_len = ml_get_buf_len(syn_st().syn_buf, startpos.lnum);

                    if pos.col <= matchcol {
                        matchcol += 1;
                    } else if pos.col <= regmatch.endpos[0].col {
                        matchcol = pos.col;
                    } else {
                        matchcol = regmatch.endpos[0].col;
                        while matchcol < line_len && matchcol < pos.col {
                            matchcol += 1;
                        }
                    }

                    if matchcol >= line_len {
                        break;
                    }

                    continue;
                }
            }

            spp = syn_items(&mut *syn_st().syn_block).offset(best_idx as isize);
            syn_add_end_off(m_endpos, &mut best_regmatch, &mut *spp, SPO_ME_OFF, 1);
            if m_endpos.lnum == startpos.lnum && m_endpos.col < startpos.col {
                m_endpos.col = startpos.col;
            }

            syn_add_end_off(end_endpos, &mut best_regmatch, &mut *spp, SPO_HE_OFF, 1);
            if end_endpos.lnum == startpos.lnum && end_endpos.col < startpos.col {
                end_endpos.col = startpos.col;
            }
            limit_pos(end_endpos, m_endpos);

            if (*spp).sp_syn_match_id != (*spp).sp_syn.id && (*spp).sp_syn_match_id != 0 {
                *end_idx = best_idx;
                if ((*spp).sp_off_flags & (1 << (SPO_RE_OFF + SPO_COUNT))) != 0 {
                    hl_endpos.lnum = best_regmatch.endpos[0].lnum;
                    hl_endpos.col = best_regmatch.endpos[0].col;
                } else {
                    hl_endpos.lnum = best_regmatch.startpos[0].lnum;
                    hl_endpos.col = best_regmatch.startpos[0].col;
                }
                hl_endpos.col += (*spp).sp_offsets[SPO_RE_OFF];

                if hl_endpos.lnum == startpos.lnum && hl_endpos.col < startpos.col {
                    hl_endpos.col = startpos.col;
                }
                limit_pos(hl_endpos, m_endpos);

                *m_endpos = *hl_endpos;
            } else {
                *end_idx = 0;
                *hl_endpos = *end_endpos;
            }

            *flagsp = (*spp).sp_flags;
            had_match = true;
            break;
        }

        if !had_match {
            m_endpos.lnum = 0;
        }

        restore_chartab(buf_chartab.as_ptr());

        g_st().re_extmatch_in = None;
    }
}

fn limit_pos(pos: &mut lpos_T, limit: &mut lpos_T) {
    if pos.lnum > limit.lnum {
            *pos = *limit;
        } else if pos.lnum == limit.lnum && pos.col > limit.col {
            pos.col = limit.col;
        }
}

fn limit_pos_zero(pos: &mut lpos_T, limit: &mut lpos_T) {
    if pos.lnum == 0 {
            *pos = *limit;
        } else {
            limit_pos(pos, limit);
        }
}

fn syn_add_end_off(
    result: &mut lpos_T,
    regmatch: &mut regmmatch_T,
    spp: &mut synpat_T,
    idx: usize,
    extra: c_int,
) {
    unsafe {
        let mut col: c_int;
        let mut off: c_int;

        if (spp.sp_off_flags & (1 << idx)) != 0 {
            result.lnum = regmatch.startpos[0].lnum;
            col = regmatch.startpos[0].col;
            off = spp.sp_offsets[idx] + extra;
        } else {
            result.lnum = regmatch.endpos[0].lnum;
            col = regmatch.endpos[0].col;
            off = spp.sp_offsets[idx];
        }

        if result.lnum > (*syn_st().syn_buf).b_ml.ml_line_count {
            col = 0;
        } else if off != 0 {
            let base = ml_get_buf(syn_st().syn_buf, result.lnum);
            let mut p = base.offset(col as isize);
            if off > 0 {
                while off > 0 && *p != NUL {
                    off -= 1;
                    mb_ptr_adv(&mut p);
                }
            } else {
                while off < 0 && base < p {
                    off += 1;
                    mb_ptr_back(base, &mut p);
                }
            }
            col = p.offset_from(base) as c_int;
        }
        result.col = col;
    }
}

fn syn_add_start_off(
    result: &mut lpos_T,
    regmatch: &mut regmmatch_T,
    spp: &mut synpat_T,
    idx: usize,
    extra: c_int,
) {
    unsafe {
        let mut col: c_int;
        let mut off: c_int;

        if (spp.sp_off_flags & (1 << (idx + SPO_COUNT))) != 0 {
            result.lnum = regmatch.endpos[0].lnum;
            col = regmatch.endpos[0].col;
            off = spp.sp_offsets[idx] + extra;
        } else {
            result.lnum = regmatch.startpos[0].lnum;
            col = regmatch.startpos[0].col;
            off = spp.sp_offsets[idx];
        }
        if result.lnum > (*syn_st().syn_buf).b_ml.ml_line_count {
            result.lnum = (*syn_st().syn_buf).b_ml.ml_line_count;
            col = ml_get_buf_len(syn_st().syn_buf, result.lnum);
        }
        if off != 0 {
            let base = ml_get_buf(syn_st().syn_buf, result.lnum);
            let mut p = base.offset(col as isize);
            if off > 0 {
                while off > 0 && *p != NUL {
                    off -= 1;
                    mb_ptr_adv(&mut p);
                }
            } else {
                while off < 0 && base < p {
                    off += 1;
                    mb_ptr_back(base, &mut p);
                }
            }
            col = p.offset_from(base) as c_int;
        }
        result.col = col;
    }
}

#[inline]
fn syn_getcurline() -> *mut c_char {
    ml_get_buf(syn_st().syn_buf, syn_st().current_lnum)
}

#[inline]
fn syn_getcurline_len() -> colnr_T {
    ml_get_buf_len(syn_st().syn_buf, syn_st().current_lnum)
}

fn syn_regexec(
    rmp: &mut regmmatch_T,
    lnum: linenr_T,
    col: colnr_T,st: &mut syn_time_T,
) -> bool {
    unsafe {
        let mut timed_out: c_int = 0;
        let mut pt: proftime_T = 0;
        let l_syn_time_on = syn_st().syn_time_on;

        if l_syn_time_on {
            pt = profile_start();
        }

        if rmp.regprog.is_null() {
            return false;
        }

        rmp.rmm_maxcol = (*syn_st().syn_buf).b_p_smc as colnr_T;
        let r = vim_regexec_multi(rmp, syn_st().syn_win, syn_st().syn_buf, lnum, col, syn_st().syn_tm, Some(&mut timed_out));

        if l_syn_time_on {
            pt = profile_end(pt);
            st.total = profile_add(st.total, pt);
            if profile_cmp(pt, st.slowest) < 0 {
                st.slowest = pt;
            }
            st.count += 1;
            if r > 0 {
                st.match_ += 1;
            }
        }
        if timed_out != 0 && !(*win_s(&mut *syn_st().syn_win)).b_syn_slow {
            (*win_s(&mut *syn_st().syn_win)).b_syn_slow = true;
            msg(
                gettext(c"'redrawtime' exceeded, syntax highlighting disabled"),
                0,
            );
        }

        if r > 0 {
            rmp.startpos[0].lnum += lnum;
            rmp.endpos[0].lnum += lnum;
            return true;
        }
        false
    }
}

fn check_keyword_id(
    line: *mut c_char,
    startcol: c_int,
    endcolp: &mut c_int,
    flagsp: &mut c_int,
    next_listp: &mut SynIdList,
    cur_si: *mut stateitem_T,
    ccharp: &mut c_int,
) -> c_int {
    unsafe {
        let kwp = line.offset(startcol as isize);
        let mut kwlen: c_int = 0;
        loop {
            kwlen += utfc_ptr2len(kwp.offset(kwlen as isize));
            if !vim_iswordp_buf(kwp.offset(kwlen as isize), syn_st().syn_buf) {
                break;
            }
        }

        if kwlen as usize > MAXKEYWLEN {
            return 0;
        }

        let mut keyword = [0 as c_char; MAXKEYWLEN + 1];
        crate::memory::cstr_ncpy(keyword.as_mut_ptr(), kwp, kwlen as usize);

        let mut kp: *const keyentry_T = std::ptr::null();

        if !(*syn_st().syn_block).b_keywtab.is_empty() {
            kp = match_keyword(keyword.as_mut_ptr(), &(*syn_st().syn_block).b_keywtab, cur_si);
        }

        if kp.is_null() && !(*syn_st().syn_block).b_keywtab_ic.is_empty() {
            str_foldcase(kwp, kwlen, keyword.as_mut_ptr(), (MAXKEYWLEN + 1) as c_int);
            kp = match_keyword(keyword.as_mut_ptr(), &(*syn_st().syn_block).b_keywtab_ic, cur_si);
        }

        if !kp.is_null() {
            *endcolp = startcol + kwlen;
            *flagsp = (*kp).flags;
            *next_listp = (*kp).next_list.clone();
            *ccharp = (*kp).k_char;
            return (*kp).k_syn.id as c_int;
        }

        0
    }
}

fn match_keyword(
    keyword: *mut c_char,
    ht: &std::collections::BTreeMap<Vec<u8>, Vec<keyentry_T>>,
    cur_si: *mut stateitem_T,
) -> *const keyentry_T {
    unsafe {
        let key_bytes = crate::memory::cstr_as_bytes(keyword);
        if let Some(entries) = ht.get(key_bytes) {
            let cur_next_list = &syn_st().current_next_list;
            for kp in entries {
                let mut k_syn = kp.k_syn.clone();
                if if cur_next_list.is_some() {
                    in_id_list(ptr::null_mut(), cur_next_list, &mut k_syn, 0) != 0
                } else if cur_si.is_null() {
                    (kp.flags & HL_CONTAINED) == 0
                } else {
                    in_id_list(cur_si, &(*cur_si).si_cont_list, &mut k_syn, kp.flags) != 0
                } {
                    return kp;
                }
            }
        }
        std::ptr::null()
    }
}

fn syn_cmd_conceal(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = eap.arg;
        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        let next = skiptowhite(arg);
        if *arg == NUL {
            if (*win_s(curwin_mut())).b_syn_conceal != 0 {
                msg(c"syntax conceal on", 0);
            } else {
                msg(c"syntax conceal off", 0);
            }
        } else if crate::memory::cstr_prefix_bytes(arg, 2).eq_ignore_ascii_case(b"on") && next.offset_from(arg) == 2 {
            (*win_s(curwin_mut())).b_syn_conceal = 1;
        } else if crate::memory::cstr_prefix_bytes(arg, 3).eq_ignore_ascii_case(b"off") && next.offset_from(arg) == 3 {
            (*win_s(curwin_mut())).b_syn_conceal = 0;
        } else {
            semsg!(gettext(e_illegal_arg), arg);
        }
    }
}

fn syn_cmd_case(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = eap.arg;
        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        let next = skiptowhite(arg);
        if *arg == NUL {
            if (*win_s(curwin_mut())).b_syn_ic != 0 {
                msg(c"syntax case ignore", 0);
            } else {
                msg(c"syntax case match", 0);
            }
        } else if crate::memory::cstr_prefix_bytes(arg, 5).eq_ignore_ascii_case(b"match") && next.offset_from(arg) == 5 {
            (*win_s(curwin_mut())).b_syn_ic = 0;
        } else if crate::memory::cstr_prefix_bytes(arg, 6).eq_ignore_ascii_case(b"ignore") && next.offset_from(arg) == 6 {
            (*win_s(curwin_mut())).b_syn_ic = 1;
        } else {
            semsg!(gettext(e_illegal_arg), arg);
        }
    }
}

fn syn_cmd_foldlevel(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg = eap.arg;
        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        if *arg == NUL {
            match (*win_s(curwin_mut())).b_syn_foldlevel {
                SYNFLD_START => {
                    msg(c"syntax foldlevel start", 0);
                }
                SYNFLD_MINIMUM => {
                    msg(c"syntax foldlevel minimum", 0);
                }
                _ => {}
            }
            return;
        }

        let arg_end = skiptowhite(arg);
        if crate::memory::cstr_prefix_bytes(arg, 5).eq_ignore_ascii_case(b"start") && arg_end.offset_from(arg) == 5 {
            (*win_s(curwin_mut())).b_syn_foldlevel = SYNFLD_START;
        } else if crate::memory::cstr_prefix_bytes(arg, 7).eq_ignore_ascii_case(b"minimum") && arg_end.offset_from(arg) == 7 {
            (*win_s(curwin_mut())).b_syn_foldlevel = SYNFLD_MINIMUM;
        } else {
            semsg!(gettext(e_illegal_arg), arg);
            return;
        }

        arg = skipwhite(arg_end);
        if *arg != NUL {
            semsg!(gettext(e_illegal_arg), arg);
        }
    }
}

fn syn_cmd_spell(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = eap.arg;
        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        let next = skiptowhite(arg);
        if *arg == NUL {
            if (*win_s(curwin_mut())).b_syn_spell == SYNSPL_TOP {
                msg(c"syntax spell toplevel", 0);
            } else if (*win_s(curwin_mut())).b_syn_spell == SYNSPL_NOTOP {
                msg(c"syntax spell notoplevel", 0);
            } else {
                msg(c"syntax spell default", 0);
            }
        } else if crate::memory::cstr_prefix_bytes(arg, 8).eq_ignore_ascii_case(b"toplevel") && next.offset_from(arg) == 8 {
            (*win_s(curwin_mut())).b_syn_spell = SYNSPL_TOP;
        } else if crate::memory::cstr_prefix_bytes(arg, 10).eq_ignore_ascii_case(b"notoplevel") && next.offset_from(arg) == 10
        {
            (*win_s(curwin_mut())).b_syn_spell = SYNSPL_NOTOP;
        } else if crate::memory::cstr_prefix_bytes(arg, 7).eq_ignore_ascii_case(b"default") && next.offset_from(arg) == 7 {
            (*win_s(curwin_mut())).b_syn_spell = SYNSPL_DEFAULT;
        } else {
            semsg!(gettext(e_illegal_arg), arg);
            return;
        }

        redraw_later(curwin_ptr(), UPD_NOT_VALID);
    }
}

fn syn_cmd_iskeyword(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg = eap.arg;
        if eap.skip != 0 {
            return;
        }

        arg = skipwhite(arg);
        if *arg == NUL {
            msg_puts(c"\n");
            if let Some(ref isk) = (*win_s(curwin_mut())).b_syn_isk {
                msg_puts(c"syntax iskeyword ");
                msg_outtrans(isk.as_ptr(), 0, false);
            } else {
                msg_outtrans(gettext(c"syntax iskeyword not set"), 0, false);
            }
        } else if crate::memory::cstr_prefix_bytes(arg, 5).eq_ignore_ascii_case(b"clear") {
            (*win_s(curwin_mut())).b_syn_chartab = (*g_st().curbuf).b_chartab;
            (*win_s(curwin_mut())).b_syn_isk = None;
        } else {
            if !check_isopt(arg) {
                emsg(gettext(e_invarg));
                return;
            }
            let isk = crate::memory::cstr_to_cstring(arg);
            let isk_ptr = isk.as_ptr().cast_mut();
            (*win_s(curwin_mut())).b_syn_isk = Some(isk);
            parse_isopt(
                isk_ptr,
                (&raw mut (*win_s(curwin_mut())).b_syn_chartab).cast(),
            );
        }
        redraw_later(curwin_ptr(), UPD_NOT_VALID);
    }
}

/// Clear all syntax info for one buffer.
pub fn syntax_clear(block: &mut synblock_T) {
    block.b_syn_error = false;
        block.b_syn_slow = false;
        block.b_syn_ic = 0;
        block.b_syn_foldlevel = SYNFLD_START;
        block.b_syn_spell = SYNSPL_DEFAULT;
        block.b_syn_containedin = 0;
        block.b_syn_conceal = 0;

        clear_keywtab(&mut block.b_keywtab);
        clear_keywtab(&mut block.b_keywtab_ic);

        block.b_syn_patterns = Vec::new();

        block.b_syn_clusters = Vec::new();
        block.b_spell_cluster_id = 0;
        block.b_nospell_cluster_id = 0;

        block.b_syn_sync_flags = 0;
        block.b_syn_sync_minlines = 0;
        block.b_syn_sync_maxlines = 0;
        block.b_syn_sync_linebreaks = 0;

        block.b_syn_linecont_prog = None;
        block.b_syn_linecont_pat = None;
        block.b_syn_folditems = 0;
        block.b_syn_isk = None;

        syn_stack_free_all(&raw mut *block);
        invalidate_current_state();

        syn_st().running_syn_inc_tag = 0;
}

impl Drop for synblock_T {
    fn drop(&mut self) {
        syntax_clear(self);
    }
}

/// Get rid of ownsyntax for window "wp".
pub fn reset_synblock(wp: &mut win_T) {
    wp.w_s_owned = None;
}

fn syntax_sync_clear() {
    unsafe {
        let mut i = (*win_s(curwin_mut())).b_syn_patterns.len() as c_int - 1;
        while i >= 0 {
            if (*syn_items(&mut *win_s(curwin_mut())).offset(i as isize)).sp_syncing {
                syn_remove_pattern(&mut *win_s(curwin_mut()), i);
            }
            i -= 1;
        }

        (*win_s(curwin_mut())).b_syn_sync_flags = 0;
        (*win_s(curwin_mut())).b_syn_sync_minlines = 0;
        (*win_s(curwin_mut())).b_syn_sync_maxlines = 0;
        (*win_s(curwin_mut())).b_syn_sync_linebreaks = 0;

        (*win_s(curwin_mut())).b_syn_linecont_prog = None;
        (*win_s(curwin_mut())).b_syn_linecont_pat = None;
        (*win_s(curwin_mut())).b_syn_isk = None;

        syn_stack_free_all(win_s(curwin_mut()));
    }
}

fn syn_remove_pattern(block: &mut synblock_T, idx: c_int) {
    unsafe {
        let spp = syn_items(block).offset(idx as isize);
        if ((*spp).sp_flags & HL_FOLD) != 0 {
            block.b_syn_folditems -= 1;
        }
        let pats = &mut block.b_syn_patterns;
        pats.remove(idx as usize);
    }
}

fn syn_cmd_clear(eap: &mut exarg_T, syncing: c_int) {
    unsafe {
        let mut arg = eap.arg;
        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        if (*win_s(curwin_mut())).b_syn_topgrp != 0 {
            return;
        }

        if ends_excmd(*arg as c_int) != 0 {
            if syncing != 0 {
                syntax_sync_clear();
            } else {
                syntax_clear(&mut *win_s(curwin_mut()));
                if win_s(curwin_mut()) == &raw mut (*curwin_mut().w_buffer).b_s {
                    do_unlet(c"b:current_syntax", 16, true);
                }
                do_unlet(c"w:current_syntax", 16, true);
            }
        } else {
            while ends_excmd(*arg as c_int) == 0 {
                let arg_end = skiptowhite(arg);
                if *arg == b'@' as c_char {
                    let id = syn_scl_namen2id(arg.add(1), arg_end.offset_from(arg) as c_int - 1);
                    if id == 0 {
                        semsg!(gettext(c"E391: No such syntax cluster: %s"), arg);
                        break;
                    }
                    let scl_id = id - SYNID_CLUSTER as c_int;
                    let clp = syn_clstr(&mut *win_s(curwin_mut())).offset(scl_id as isize);
                    (*clp).scl_list = None;
                } else {
                    let id = syn_name2id_len(arg, arg_end.offset_from(arg) as usize);
                    if id == 0 {
                        semsg!(gettext(e_nogroup), arg);
                        break;
                    }
                    syn_clear_one(id, syncing != 0);
                }
                arg = skipwhite(arg_end);
            }
        }
        redraw_curbuf_later(UPD_SOME_VALID);
        syn_stack_free_all(win_s(curwin_mut()));
    }
}

fn syn_clear_one(id: c_int, syncing: bool) {
    unsafe {
        if !syncing {
            syn_clear_keyword(id, &mut (*win_s(curwin_mut())).b_keywtab);
            syn_clear_keyword(id, &mut (*win_s(curwin_mut())).b_keywtab_ic);
        }

        let mut idx = (*win_s(curwin_mut())).b_syn_patterns.len() as c_int - 1;
        while idx >= 0 {
            let spp = syn_items(&mut *win_s(curwin_mut())).offset(idx as isize);
            if (*spp).sp_syn.id as c_int == id && (*spp).sp_syncing == syncing {
                syn_remove_pattern(&mut *win_s(curwin_mut()), idx);
            }
            idx -= 1;
        }
    }
}

fn syn_cmd_on(eap: &mut exarg_T, _syncing: c_int) {
    syn_cmd_onoff(eap, c"syntax");
}

fn syn_cmd_reset(eap: &mut exarg_T, _syncing: c_int) {
    eap.nextcmd = check_nextcmd(eap.arg);
        if eap.skip == 0 {
            init_highlight(true, true);
        }
}

fn syn_cmd_manual(eap: &mut exarg_T, _syncing: c_int) {
    syn_cmd_onoff(eap, c"manual");
}

fn syn_cmd_off(eap: &mut exarg_T, _syncing: c_int) {
    syn_cmd_onoff(eap, c"nosyntax");
}

fn syn_cmd_onoff(eap: &mut exarg_T, name: &std::ffi::CStr) {
    unsafe {
        eap.nextcmd = check_nextcmd(eap.arg);
        if eap.skip == 0 {
            syn_st().did_syntax_onoff = true;
            let mut buf = [0 as c_char; 100];
            ptr::copy_nonoverlapping(c"so ".as_ptr(), buf.as_mut_ptr(), 4);
            vim_snprintf!(buf.as_mut_ptr().add(3), 100 - 3, SYNTAX_FNAME, name.as_ptr());
            do_cmdline_cmd(buf.as_ptr());
        }
    }
}

pub fn syn_maybe_enable() {
    if !syn_st().did_syntax_onoff {
            let mut ea: exarg_T = exarg_T::new();
            ea.arg = c"".as_ptr().cast_mut();
            ea.skip = 0;
            syn_cmd_on(&mut ea, 0);
        }
}

fn syn_cmd_list(eap: &mut exarg_T, syncing: c_int) {
    unsafe {
        let mut arg = eap.arg;
        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        msg_ext_set_kind(c"list_cmd");
        if !syntax_present(curwin_ptr()) {
            msg(gettext(msg_no_items), 0);
            return;
        }

        if syncing != 0 {
            if ((*win_s(curwin_mut())).b_syn_sync_flags & SF_CCOMMENT) != 0 {
                msg_puts(gettext(c"syncing on C-style comments"));
                syn_lines_msg();
                syn_match_msg();
            } else if ((*win_s(curwin_mut())).b_syn_sync_flags & SF_MATCH) != 0 {
                msg_puts_title(gettext(c"\n--- Syntax sync items ---"));
                if (*win_s(curwin_mut())).b_syn_sync_minlines > 0
                    || (*win_s(curwin_mut())).b_syn_sync_maxlines > 0
                    || (*win_s(curwin_mut())).b_syn_sync_linebreaks > 0
                {
                    msg_puts(gettext(c"\nsyncing on items"));
                    syn_lines_msg();
                    syn_match_msg();
                }
                let mut id: c_int = 1;
                while id <= highlight_num_groups() && !g_st().got_int {
                    syn_list_one(id, true, false);
                    id += 1;
                }
            } else if (*win_s(curwin_mut())).b_syn_sync_minlines == 0 {
                msg_puts(gettext(c"no syncing"));
            } else {
                if (*win_s(curwin_mut())).b_syn_sync_minlines == MAXLNUM {
                    msg_puts(gettext(c"syncing starts at the first line"));
                } else {
                    msg_puts(gettext(c"syncing starts "));
                    msg_outnum((*win_s(curwin_mut())).b_syn_sync_minlines as c_int);
                    msg_puts(gettext(c" lines before top line"));
                }
                syn_match_msg();
            }
            return;
        }

        msg_puts_title(gettext(c"\n--- Syntax items ---"));
        if ends_excmd(*arg as c_int) != 0 {
            let mut id: c_int = 1;
            while id <= highlight_num_groups() && !g_st().got_int {
                syn_list_one(id, false, false);
                id += 1;
            }
            let mut cid: c_int = 0;
            while (cid as usize) < (*win_s(curwin_mut())).b_syn_clusters.len() && !g_st().got_int {
                syn_list_cluster(cid);
                cid += 1;
            }
        } else {
            while ends_excmd(*arg as c_int) == 0 && !g_st().got_int {
                let arg_end = skiptowhite(arg);
                if *arg == b'@' as c_char {
                    let id = syn_scl_namen2id(arg.add(1), arg_end.offset_from(arg) as c_int - 1);
                    if id == 0 {
                        semsg!(gettext(c"E392: No such syntax cluster: %s"), arg);
                    } else {
                        syn_list_cluster(id - SYNID_CLUSTER as c_int);
                    }
                } else {
                    let id = syn_name2id_len(arg, arg_end.offset_from(arg) as usize);
                    if id == 0 {
                        semsg!(gettext(e_nogroup), arg);
                    } else {
                        syn_list_one(id, false, true);
                    }
                }
                arg = skipwhite(arg_end);
            }
        }
        eap.nextcmd = check_nextcmd(arg);
    }
}

fn syn_lines_msg() {
    unsafe {
        if (*win_s(curwin_mut())).b_syn_sync_maxlines > 0 || (*win_s(curwin_mut())).b_syn_sync_minlines > 0 {
            msg_puts(c"; ");
            if (*win_s(curwin_mut())).b_syn_sync_minlines == MAXLNUM {
                msg_puts(gettext(c"from the first line"));
            } else {
                if (*win_s(curwin_mut())).b_syn_sync_minlines > 0 {
                    msg_puts(gettext(c"minimal "));
                    msg_outnum((*win_s(curwin_mut())).b_syn_sync_minlines as c_int);
                    if (*win_s(curwin_mut())).b_syn_sync_maxlines != 0 {
                        msg_puts(c", ");
                    }
                }
                if (*win_s(curwin_mut())).b_syn_sync_maxlines > 0 {
                    msg_puts(gettext(c"maximal "));
                    msg_outnum((*win_s(curwin_mut())).b_syn_sync_maxlines as c_int);
                }
                msg_puts(gettext(c" lines before top line"));
            }
        }
    }
}

fn syn_match_msg() {
    unsafe {
        if (*win_s(curwin_mut())).b_syn_sync_linebreaks > 0 {
            msg_puts(gettext(c"; match "));
            msg_outnum((*win_s(curwin_mut())).b_syn_sync_linebreaks as c_int);
            msg_puts(gettext(c" line breaks"));
        }
    }
}

fn syn_list_one(id: c_int, syncing: bool, link_only: bool) {
    unsafe {
        let mut did_header = false;
        const NAMELIST1: [(c_int, *const c_char); 10] = [
            (HL_DISPLAY, c"display".as_ptr()),
            (HL_CONTAINED, c"contained".as_ptr()),
            (HL_ONELINE, c"oneline".as_ptr()),
            (HL_KEEPEND, c"keepend".as_ptr()),
            (HL_EXTEND, c"extend".as_ptr()),
            (HL_EXCLUDENL, c"excludenl".as_ptr()),
            (HL_TRANSP, c"transparent".as_ptr()),
            (HL_FOLD, c"fold".as_ptr()),
            (HL_CONCEAL, c"conceal".as_ptr()),
            (HL_CONCEALENDS, c"concealends".as_ptr()),
        ];
        const NAMELIST2: [(c_int, *const c_char); 3] = [
            (HL_SKIPWHITE, c"skipwhite".as_ptr()),
            (HL_SKIPNL, c"skipnl".as_ptr()),
            (HL_SKIPEMPTY, c"skipempty".as_ptr()),
        ];

        let hl_id = HLF_D;

        if !syncing {
            did_header = syn_list_keywords(id, &(*win_s(curwin_mut())).b_keywtab, false, hl_id);
            did_header = syn_list_keywords(id, &(*win_s(curwin_mut())).b_keywtab_ic, did_header, hl_id);
        }

        let mut idx: c_int = 0;
        while (idx as usize) < (*win_s(curwin_mut())).b_syn_patterns.len() && !g_st().got_int {
            let spp = syn_items(&mut *win_s(curwin_mut())).offset(idx as isize);
            if (*spp).sp_syn.id as c_int != id || (*spp).sp_syncing != syncing {
                idx += 1;
                continue;
            }

            syn_list_header(did_header, 0, id, true);
            did_header = true;
            syn_st().last_matchgroup = 0;
            if (*spp).sp_type == SPTYPE_MATCH {
                put_pattern(c"match", b' ' as c_int, &*spp, hl_id);
                msg_putchar(b' ' as c_int);
            } else if (*spp).sp_type == SPTYPE_START {
                while (*syn_items(&mut *win_s(curwin_mut())).offset(idx as isize)).sp_type == SPTYPE_START {
                    put_pattern(
                        c"start",
                        b'=' as c_int,
                        &*syn_items(&mut *win_s(curwin_mut())).offset(idx as isize),
                        hl_id,
                    );
                    idx += 1;
                }
                if (*syn_items(&mut *win_s(curwin_mut())).offset(idx as isize)).sp_type == SPTYPE_SKIP {
                    put_pattern(
                        c"skip",
                        b'=' as c_int,
                        &*syn_items(&mut *win_s(curwin_mut())).offset(idx as isize),
                        hl_id,
                    );
                    idx += 1;
                }
                while (idx as usize) < (*win_s(curwin_mut())).b_syn_patterns.len()
                    && (*syn_items(&mut *win_s(curwin_mut())).offset(idx as isize)).sp_type == SPTYPE_END
                {
                    put_pattern(
                        c"end",
                        b'=' as c_int,
                        &*syn_items(&mut *win_s(curwin_mut())).offset(idx as isize),
                        hl_id,
                    );
                    idx += 1;
                }
                idx -= 1;
                msg_putchar(b' ' as c_int);
            }
            syn_list_flags(&NAMELIST1, (*spp).sp_flags, hl_id);

            if let Some(ref list) = (*spp).sp_cont_list {
                put_id_list(c"contains", list.as_ptr(), hl_id);
            }

            if let Some(ref list) = (*spp).sp_syn.cont_in_list {
                put_id_list(c"containedin", list.as_ptr(), hl_id);
            }

            if let Some(ref list) = (*spp).sp_next_list {
                put_id_list(c"nextgroup", list.as_ptr(), hl_id);
                syn_list_flags(&NAMELIST2, (*spp).sp_flags, hl_id);
            }
            if ((*spp).sp_flags & (HL_SYNC_HERE | HL_SYNC_THERE)) != 0 {
                if ((*spp).sp_flags & HL_SYNC_HERE) != 0 {
                    msg_puts_hl(c"grouphere", hl_id, false);
                } else {
                    msg_puts_hl(c"groupthere", hl_id, false);
                }
                msg_putchar(b' ' as c_int);
                if (*spp).sp_sync_idx >= 0 {
                    let sync_id = (*syn_items(&mut *win_s(curwin_mut())).offset((*spp).sp_sync_idx as isize))
                        .sp_syn
                        .id as c_int;
                    msg_outtrans(highlight_group_name(sync_id - 1), 0, false);
                } else {
                    msg_puts(c"NONE");
                }
                msg_putchar(b' ' as c_int);
            }
            idx += 1;
        }

        if highlight_link_id(id - 1) != 0 && (did_header || link_only) && !g_st().got_int {
            syn_list_header(did_header, 0, id, true);
            msg_puts_hl(c"links to", hl_id, false);
            msg_putchar(b' ' as c_int);
            msg_outtrans(
                highlight_group_name(highlight_link_id(id - 1) - 1),
                0,
                false,
            );
        }
    }
}

fn syn_list_flags(nlist: &[(c_int, *const c_char)], flags: c_int, hl_id: c_int) {
    for &(key, value) in nlist {
            if (flags & key) != 0 {
                msg_puts_hl(value, hl_id, false);
                msg_putchar(b' ' as c_int);
            }
        }
}

fn syn_list_cluster(id: c_int) {
    unsafe {
        let mut endcol: c_int = 15;

        msg_putchar(b'\n' as c_int);
        let clp = syn_clstr(&mut *win_s(curwin_mut())).offset(id as isize);
        msg_outtrans((*clp).scl_name.as_ptr(), 0, false);

        if g_st().msg_col >= endcol {
            endcol = g_st().msg_col + 1;
        }
        if g_st().Columns <= endcol {
            endcol = g_st().Columns - 1;
        }

        msg_advance(endcol);
        if let Some(ref list) = (*clp).scl_list {
            put_id_list(c"cluster", list.as_ptr(), HLF_D);
        } else {
            msg_puts_hl(c"cluster", HLF_D, false);
            msg_puts(c"=NONE");
        }
    }
}

fn put_id_list(name: &std::ffi::CStr, list: *const i16, hl_id: c_int) {
    unsafe {
        msg_puts_hl(name.as_ptr(), hl_id, false);
        msg_putchar(b'=' as c_int);
        let mut p = list;
        while *p != 0 {
            if *p >= SYNID_ALLBUT && *p < SYNID_TOP {
                if *p.add(1) != 0 {
                    msg_puts(c"ALLBUT");
                } else {
                    msg_puts(c"ALL");
                }
            } else if *p >= SYNID_TOP && *p < SYNID_CONTAINED {
                msg_puts(c"TOP");
            } else if *p >= SYNID_CONTAINED && *p < SYNID_CLUSTER {
                msg_puts(c"CONTAINED");
            } else if *p >= SYNID_CLUSTER {
                let scl_id = (*p - SYNID_CLUSTER) as isize;
                msg_putchar(b'@' as c_int);
                msg_outtrans(
                    (*syn_clstr(&mut *win_s(curwin_mut())).offset(scl_id)).scl_name.as_ptr(),
                    0,
                    false,
                );
            } else {
                msg_outtrans(highlight_group_name(*p as c_int - 1), 0, false);
            }
            if *p.add(1) != 0 {
                msg_putchar(b',' as c_int);
            }
            p = p.add(1);
        }
        msg_putchar(b' ' as c_int);
    }
}

fn put_pattern(s: &std::ffi::CStr, c: c_int, spp: &synpat_T, hl_id: c_int) {
    const SEPCHARS: &[u8] = b"/+=-#@\"|'^&\0";

        if syn_st().last_matchgroup != spp.sp_syn_match_id as c_int {
            syn_st().last_matchgroup = spp.sp_syn_match_id as c_int;
            msg_puts_hl(c"matchgroup", hl_id, false);
            msg_putchar(b'=' as c_int);
            if syn_st().last_matchgroup == 0 {
                msg_outtrans(c"NONE", 0, false);
            } else {
                msg_outtrans(highlight_group_name(syn_st().last_matchgroup - 1), 0, false);
            }
            msg_putchar(b' ' as c_int);
        }

        msg_puts_hl(s.as_ptr(), hl_id, false);
        msg_putchar(c);

        let pat_ptr = spp
            .sp_pattern
            .as_ref()
            .map_or(c"".as_ptr(), |s| s.as_ptr());
        let mut i: usize = 0;
        while !vim_strchr(pat_ptr, SEPCHARS[i] as c_int).is_null() {
            i += 1;
            if SEPCHARS[i] == 0 {
                i = 0;
                break;
            }
        }
        msg_putchar(SEPCHARS[i] as c_int);
        msg_outtrans(pat_ptr, 0, false);
        msg_putchar(SEPCHARS[i] as c_int);

        let mut first = true;
        for (j, &spo_name) in spo_name_tab.iter().enumerate().take(SPO_COUNT) {
            let mask: i16 = 1 << j;
            if (spp.sp_off_flags & (mask + (mask << SPO_COUNT))) == 0 {
                continue;
            }
            if !first {
                msg_putchar(b',' as c_int);
            }
            msg_puts(spo_name);
            let n = spp.sp_offsets[j];
            if j != SPO_LC_OFF {
                if (spp.sp_off_flags & mask) != 0 {
                    msg_putchar(b's' as c_int);
                } else {
                    msg_putchar(b'e' as c_int);
                }
                if n > 0 {
                    msg_putchar(b'+' as c_int);
                }
            }
            if n != 0 || j == SPO_LC_OFF {
                msg_outnum(n as c_int);
            }
            first = false;
        }
        msg_putchar(b' ' as c_int);
}

fn syn_list_keywords(
    id: c_int,
    ht: &std::collections::BTreeMap<Vec<u8>, Vec<keyentry_T>>,
    mut did_header: bool,
    hl_id: c_int,
) -> bool {
     {
        let mut prev_contained: c_int = 0;
        let mut prev_next_list: SynIdList = None;
        let mut prev_cont_in_list: SynIdList = None;
        let mut prev_skipnl: c_int = 0;
        let mut prev_skipwhite: c_int = 0;
        let mut prev_skipempty: c_int = 0;

        for entries in ht.values() {
            if g_st().got_int {
                break;
            }
            for kp in entries {
                if g_st().got_int {
                    break;
                }
                if kp.k_syn.id as c_int == id {
                    let mut outlen: c_int = 0;
                    let mut force_newline = false;
                    if prev_contained != (kp.flags & HL_CONTAINED)
                        || prev_skipnl != (kp.flags & HL_SKIPNL)
                        || prev_skipwhite != (kp.flags & HL_SKIPWHITE)
                        || prev_skipempty != (kp.flags & HL_SKIPEMPTY)
                        || !syn_id_list_eq(&prev_cont_in_list, &kp.k_syn.cont_in_list)
                        || !syn_id_list_eq(&prev_next_list, &kp.next_list)
                    {
                        force_newline = true;
                    } else {
                        outlen = crate::memory::cstr_len(ke2hikey(kp)) as c_int;
                    }
                    if syn_list_header(did_header, outlen, id, force_newline) {
                        prev_contained = 0;
                        prev_next_list = None;
                        prev_cont_in_list = None;
                        prev_skipnl = 0;
                        prev_skipwhite = 0;
                        prev_skipempty = 0;
                    }
                    did_header = true;
                    if prev_contained != (kp.flags & HL_CONTAINED) {
                        msg_puts_hl(c"contained", hl_id, false);
                        msg_putchar(b' ' as c_int);
                        prev_contained = kp.flags & HL_CONTAINED;
                    }
                    if !syn_id_list_eq(&kp.k_syn.cont_in_list, &prev_cont_in_list) {
                        if let Some(ref list) = kp.k_syn.cont_in_list {
                            put_id_list(c"containedin", list.as_ptr(), hl_id);
                            msg_putchar(b' ' as c_int);
                        }
                        prev_cont_in_list = kp.k_syn.cont_in_list.clone();
                    }
                    if !syn_id_list_eq(&kp.next_list, &prev_next_list) {
                        if let Some(ref list) = kp.next_list {
                            put_id_list(c"nextgroup", list.as_ptr(), hl_id);
                            msg_putchar(b' ' as c_int);
                        }
                        prev_next_list = kp.next_list.clone();
                        if (kp.flags & HL_SKIPNL) != 0 {
                            msg_puts_hl(c"skipnl", hl_id, false);
                            msg_putchar(b' ' as c_int);
                            prev_skipnl = kp.flags & HL_SKIPNL;
                        }
                        if (kp.flags & HL_SKIPWHITE) != 0 {
                            msg_puts_hl(c"skipwhite", hl_id, false);
                            msg_putchar(b' ' as c_int);
                            prev_skipwhite = kp.flags & HL_SKIPWHITE;
                        }
                        if (kp.flags & HL_SKIPEMPTY) != 0 {
                            msg_puts_hl(c"skipempty", hl_id, false);
                            msg_putchar(b' ' as c_int);
                            prev_skipempty = kp.flags & HL_SKIPEMPTY;
                        }
                    }
                    msg_outtrans(ke2hikey(kp), 0, false);
                }
            }
        }

        did_header
    }
}

fn syn_clear_keyword(
    id: c_int,
    ht: &mut std::collections::BTreeMap<Vec<u8>, Vec<keyentry_T>>,
) {
    ht.retain(|_, entries| {
        entries.retain(|kp| kp.k_syn.id as c_int != id);
        !entries.is_empty()
    });
}

fn clear_keywtab(ht: &mut std::collections::BTreeMap<Vec<u8>, Vec<keyentry_T>>) {
    ht.clear();
}

fn add_keyword(
    name: *mut c_char,
    namelen: usize,
    id: c_int,
    flags: c_int,
    cont_in_list: &SynIdList,
    next_list: &SynIdList,
    conceal_char: c_int,
) {
    unsafe {
        let mut name_folded = [0 as c_char; MAXKEYWLEN + 1];
        let (name_ic, name_iclen): (*const c_char, usize) = if (*win_s(curwin_mut())).b_syn_ic != 0 {
            str_foldcase(
                name,
                namelen as c_int,
                name_folded.as_mut_ptr(),
                (MAXKEYWLEN + 1) as c_int,
            );
            (name_folded.as_ptr(), crate::memory::cstr_len(name_folded.as_ptr()))
        } else {
            (name, namelen)
        };

        let mut kw_vec = Vec::with_capacity(name_iclen + 1);
        kw_vec.extend_from_slice(std::slice::from_raw_parts(name_ic, name_iclen));
        kw_vec.push(NUL);
        if cont_in_list.is_some() {
            (*win_s(curwin_mut())).b_syn_containedin = 1;
        }
        let kp = keyentry_T {
            k_syn: sp_syn {
                inc_tag: syn_st().current_syn_inc_tag,
                id: id as i16,
                cont_in_list: cont_in_list.clone(),
            },
            next_list: next_list.clone(),
            flags,
            k_char: conceal_char,
            keyword: kw_vec,
        };

        let ht = if (*win_s(curwin_mut())).b_syn_ic != 0 {
            &mut (*win_s(curwin_mut())).b_keywtab_ic
        } else {
            &mut (*win_s(curwin_mut())).b_keywtab
        };
        let key_bytes = std::slice::from_raw_parts(name_ic.cast::<u8>(), name_iclen).to_vec();
        ht.entry(key_bytes).or_default().insert(0, kp);
    }
}

fn get_group_name(arg: *mut c_char, name_end: *mut *mut c_char) -> *mut c_char {
    unsafe {
        *name_end = skiptowhite(arg);
        let rest = skipwhite(*name_end);
        if ends_excmd(*arg as c_int) != 0 || *rest == NUL {
            return ptr::null_mut();
        }
        rest
    }
}

fn get_syn_options(
    mut arg: *mut c_char,
    opt: &mut syn_opt_arg_T,
    conceal_char: &mut c_int,
    skip: c_int,
) -> *mut c_char {
    unsafe {
        const FLAGTAB: &[(&[u8], c_int, c_int)] = &[
            (b"cCoOnNtTaAiInNeEdD\0", 0, HL_CONTAINED),
            (b"oOnNeElLiInNeE\0", 0, HL_ONELINE),
            (b"kKeEeEpPeEnNdD\0", 0, HL_KEEPEND),
            (b"eExXtTeEnNdD\0", 0, HL_EXTEND),
            (b"eExXcClLuUdDeEnNlL\0", 0, HL_EXCLUDENL),
            (b"tTrRaAnNsSpPaArReEnNtT\0", 0, HL_TRANSP),
            (b"sSkKiIpPnNlL\0", 0, HL_SKIPNL),
            (b"sSkKiIpPwWhHiItTeE\0", 0, HL_SKIPWHITE),
            (b"sSkKiIpPeEmMpPtTyY\0", 0, HL_SKIPEMPTY),
            (b"gGrRoOuUpPhHeErReE\0", 0, HL_SYNC_HERE),
            (b"gGrRoOuUpPtThHeErReE\0", 0, HL_SYNC_THERE),
            (b"dDiIsSpPlLaAyY\0", 0, HL_DISPLAY),
            (b"fFoOlLdD\0", 0, HL_FOLD),
            (b"cCoOnNcCeEaAlL\0", 0, HL_CONCEAL),
            (b"cCoOnNcCeEaAlLeEnNdDsS\0", 0, HL_CONCEALENDS),
            (b"cCcChHaArR\0", 11, 0),
            (b"cCoOnNtTaAiInNsS\0", 1, 0),
            (b"cCoOnNtTaAiInNeEdDiInN\0", 2, 0),
            (b"nNeExXtTgGrRoOuUpP\0", 3, 0),
        ];
        const FIRST_LETTERS: *const c_char = c"cCoOkKeEtTsSgGdDfFnN".as_ptr();

        if arg.is_null() {
            return ptr::null_mut();
        }

        if (*win_s(curwin_mut())).b_syn_conceal != 0 {
            opt.flags |= HL_CONCEAL;
        }

        loop {
            if *arg == NUL || !crate::memory::cstr_as_bytes(FIRST_LETTERS).contains(&(*arg as u8)) {
                break;
            }

            let mut fidx: isize = FLAGTAB.len() as isize - 1;
            let mut len: usize = 0;
            while fidx >= 0 {
                let (p, argtype, flags) = FLAGTAB[fidx as usize];
                let mut i: usize = 0;
                len = 0;
                while p[i] != 0 {
                    let ch = *arg.add(len) as u8;
                    if ch != p[i] && ch != p[i + 1] {
                        break;
                    }
                    i += 2;
                    len += 1;
                }
                if p[i] == 0
                    && (ascii_iswhite(*arg.add(len))
                        || if argtype > 0 {
                            *arg.add(len) == b'=' as c_char
                        } else {
                            ends_excmd(*arg.add(len) as c_int) != 0
                        })
                {
                    if opt.keyword
                        && (flags == HL_DISPLAY || flags == HL_FOLD || flags == HL_EXTEND)
                    {
                        fidx = -1;
                    }
                    break;
                }
                fidx -= 1;
            }
            if fidx < 0 {
                break;
            }

            let (_, argtype, flags) = FLAGTAB[fidx as usize];
            if argtype == 1 {
                if !opt.has_cont_list {
                    emsg(gettext(e_contains_argument_not_accepted_here));
                    return ptr::null_mut();
                }
                if !get_id_list(&raw mut arg, 8, &mut opt.cont_list, skip != 0) {
                    return ptr::null_mut();
                }
            } else if argtype == 2 {
                if !get_id_list(&raw mut arg, 11, &mut opt.cont_in_list, skip != 0) {
                    return ptr::null_mut();
                }
            } else if argtype == 3 {
                if !get_id_list(&raw mut arg, 9, &mut opt.next_list, skip != 0) {
                    return ptr::null_mut();
                }
            } else if argtype == 11 && *arg.add(5) == b'=' as c_char {
                *conceal_char = utf_ptr2char(arg.add(6));
                arg = arg.offset((utfc_ptr2len(arg.add(6)) - 1) as isize);
                if !vim_isprintc(*conceal_char) {
                    emsg(gettext(e_invalid_cchar_value));
                    return ptr::null_mut();
                }
                arg = skipwhite(arg.add(7));
            } else {
                opt.flags |= flags;
                arg = skipwhite(arg.add(len));

                if flags == HL_SYNC_HERE || flags == HL_SYNC_THERE {
                    if opt.sync_idx.is_none() {
                        emsg(gettext(c"E393: group[t]here not accepted here"));
                        return ptr::null_mut();
                    }
                    let gname_start = arg;
                    arg = skiptowhite(arg);
                    if gname_start == arg {
                        return ptr::null_mut();
                    }
                    let gname_slice = std::slice::from_raw_parts(
                        gname_start.cast::<u8>(),
                        arg.offset_from(gname_start) as usize,
                    );
                    let gname = std::ffi::CString::new(gname_slice).unwrap();
                    if gname.as_bytes() == b"NONE" {
                        opt.sync_idx = Some(NONE_IDX);
                    } else {
                        let syn_id = syn_name2id(gname.as_ptr());
                        let mut i = (*win_s(curwin_mut())).b_syn_patterns.len() as c_int - 1;
                        while i >= 0 {
                            let spp = syn_items(&mut *win_s(curwin_mut())).offset(i as isize);
                            if (*spp).sp_syn.id as c_int == syn_id && (*spp).sp_type == SPTYPE_START
                            {
                                opt.sync_idx = Some(i);
                                break;
                            }
                            i -= 1;
                        }
                        if i < 0 {
                            semsg!(
                                gettext(c"E394: Didn't find region item for %s"),
                                gname.as_ptr(),
                            );
                            return ptr::null_mut();
                        }
                    }

                    arg = skipwhite(arg);
                } else if flags == HL_FOLD && foldmethodIsSyntax(curwin_mut()) {
                    foldUpdateAll(curwin_mut());
                }
            }
        }

        arg
    }
}

fn syn_incl_toplevel(id: c_int, flagsp: &mut c_int) {
    unsafe {
        if (*flagsp & HL_CONTAINED) != 0 || (*win_s(curwin_mut())).b_syn_topgrp == 0 {
            return;
        }
        *flagsp |= HL_CONTAINED | HL_INCLUDED_TOPLEVEL;
        if (*win_s(curwin_mut())).b_syn_topgrp >= SYNID_CLUSTER as c_int {
            let mut grp_list: SynIdList = Some(std::rc::Rc::from([id as i16, 0]));
            let tlg_id = (*win_s(curwin_mut())).b_syn_topgrp - SYNID_CLUSTER as c_int;
            let clp = syn_clstr(&mut *win_s(curwin_mut())).offset(tlg_id as isize);
            syn_combine_list(&mut (*clp).scl_list, &mut grp_list, CLUSTER_ADD);
        }
    }
}

fn syn_cmd_include(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg = eap.arg;
        let mut sgl_id: c_int = 1;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut errormsg: *const c_char = std::ptr::null();
        let mut source = false;

        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        if *arg == b'@' as c_char {
            arg = arg.add(1);
            let rest = get_group_name(arg, &raw mut group_name_end);
            if rest.is_null() {
                emsg(gettext(c"E397: Filename required"));
                return;
            }
            sgl_id = syn_check_cluster(arg, group_name_end.offset_from(arg) as c_int);
            if sgl_id == 0 {
                return;
            }
            eap.arg = rest;
        }

        eap.argt |= EX_XFILE | EX_NOSPC;
        separate_nextcmd(&mut *eap);
        if *eap.arg == b'<' as c_char
            || *eap.arg == b'$' as c_char
            || path_is_absolute(eap.arg)
        {
            source = true;
            if !expand_filename(eap, eap.cmdlinep, &mut errormsg) {
                if !errormsg.is_null() {
                    emsg(errormsg);
                }
                return;
            }
        } else {
            #[cfg(windows)]
            path_to_slash(eap.arg);
        }

        if syn_st().running_syn_inc_tag >= MAX_SYN_INC_TAG {
            emsg(gettext(c"E847: Too many syntax includes"));
            return;
        }
        let prev_syn_inc_tag = syn_st().current_syn_inc_tag;
        syn_st().running_syn_inc_tag += 1;
        syn_st().current_syn_inc_tag = syn_st().running_syn_inc_tag;
        let prev_toplvl_grp = (*win_s(curwin_mut())).b_syn_topgrp;
        (*win_s(curwin_mut())).b_syn_topgrp = sgl_id;
        let res = if source {
            do_source(eap.arg, false, DOSO_NONE, ptr::null_mut())
        } else {
            source_runtime(eap.arg, DIP_ALL)
        };
        if !res {
            semsg!(gettext(e_notopen), eap.arg);
        }
        (*win_s(curwin_mut())).b_syn_topgrp = prev_toplvl_grp;
        syn_st().current_syn_inc_tag = prev_syn_inc_tag;
    }
}

fn syn_cmd_keyword(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = eap.arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut conceal_char: c_int = NUL as c_int;

        let mut rest = get_group_name(arg, &raw mut group_name_end);

        if !rest.is_null() {
            let syn_id = if eap.skip != 0 {
                -1
            } else {
                syn_check_group(arg, group_name_end.offset_from(arg) as usize)
            };
            if syn_id != 0 {
                let mut keyword_buf = vec![0 as c_char; crate::memory::cstr_len(rest) + 1];
                let keyword_copy = keyword_buf.as_mut_ptr();
                let mut syn_opt_arg = syn_opt_arg_T {
                    flags: 0,
                    keyword: true,
                    sync_idx: None,
                    has_cont_list: false,
                    cont_list: None,
                    cont_in_list: None,
                    next_list: None,
                };

                let mut cnt: c_int = 0;
                let mut p = keyword_copy;
                while !rest.is_null() && ends_excmd(*rest as c_int) == 0 {
                    rest = get_syn_options(
                        rest,
                        &mut syn_opt_arg,
                        &mut conceal_char,
                        eap.skip,
                    );
                    if rest.is_null() || ends_excmd(*rest as c_int) != 0 {
                        break;
                    }
                    while *rest != NUL && !ascii_iswhite(*rest) {
                        if *rest == b'\\' as c_char && *rest.add(1) != NUL {
                            rest = rest.add(1);
                        }
                        *p = *rest;
                        p = p.add(1);
                        rest = rest.add(1);
                    }
                    *p = NUL;
                    p = p.add(1);
                    cnt += 1;
                    rest = skipwhite(rest);
                }

                if eap.skip == 0 {
                    syn_incl_toplevel(syn_id, &mut syn_opt_arg.flags);

                    let mut kw = keyword_copy;
                    let mut kwlen: usize;
                    'kw_loop: while cnt > 0 {
                        cnt -= 1;
                        p = vim_strchr(kw, b'[' as c_int);
                        loop {
                            if p.is_null() {
                                kwlen = crate::memory::cstr_len(kw);
                            } else {
                                *p = NUL;
                                kwlen = p.offset_from(kw) as usize;
                            }
                            add_keyword(
                                kw,
                                kwlen,
                                syn_id,
                                syn_opt_arg.flags,
                                &syn_opt_arg.cont_in_list,
                                &syn_opt_arg.next_list,
                                conceal_char,
                            );
                            if p.is_null() {
                                break;
                            }
                            if *p.add(1) == NUL {
                                semsg!(gettext(c"E789: Missing ']': %s"), kw);
                                break 'kw_loop;
                            }
                            if *p.add(1) == b']' as c_char {
                                if *p.add(2) != NUL {
                                    semsg!(
                                        gettext(e_trailing_char_after_rsb_str_str),
                                        kw,
                                        p.add(2)
                                    );
                                    break 'kw_loop;
                                }
                                kw = p.add(1);
                                kwlen = 1;
                                break;
                            }
                            let l = utfc_ptr2len(p.add(1)) as usize;
                            ptr::copy(p.add(1), p, l);
                            p = p.add(l);
                        }
                        kw = kw.add(kwlen + 1);
                    }
                }
            }
        }

        if !rest.is_null() {
            eap.nextcmd = check_nextcmd(rest);
        } else {
            semsg!(gettext(e_invarg2), arg);
        }

        redraw_curbuf_later(UPD_SOME_VALID);
        syn_stack_free_all(win_s(curwin_mut()));
    }
}

fn syn_cmd_match(eap: &mut exarg_T, syncing: c_int) {
    unsafe {
        let arg = eap.arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut item = synpat_T::new();
        let mut conceal_char: c_int = NUL as c_int;

        let mut rest = get_group_name(arg, &raw mut group_name_end);

        let mut syn_opt_arg = syn_opt_arg_T {
            flags: 0,
            keyword: false,
            sync_idx: (syncing != 0).then_some(0),
            has_cont_list: true,
            cont_list: None,
            cont_in_list: None,
            next_list: None,
        };
        rest = get_syn_options(
            rest,
            &mut syn_opt_arg,
            &mut conceal_char,
            eap.skip,
        );

        rest = get_syn_pattern(rest, &mut item);
        if vim_regcomp_had_eol() != 0 && (syn_opt_arg.flags & HL_EXCLUDENL) == 0 {
            syn_opt_arg.flags |= HL_HAS_EOL;
        }

        rest = get_syn_options(
            rest,
            &mut syn_opt_arg,
            &mut conceal_char,
            eap.skip,
        );

        if !rest.is_null() {
            eap.nextcmd = check_nextcmd(rest);
            if ends_excmd(*rest as c_int) == 0 || eap.skip != 0 {
                rest = ptr::null_mut();
            } else {
                let syn_id = syn_check_group(arg, group_name_end.offset_from(arg) as usize);
                if syn_id != 0 {
                    syn_incl_toplevel(syn_id, &mut syn_opt_arg.flags);
                    item.sp_syncing = syncing != 0;
                    item.sp_type = SPTYPE_MATCH;
                    item.sp_syn.id = syn_id as i16;
                    item.sp_syn.inc_tag = syn_st().current_syn_inc_tag;
                    item.sp_flags = syn_opt_arg.flags;
                    item.sp_sync_idx = syn_opt_arg.sync_idx.unwrap_or(0);
                    item.sp_cont_list = syn_opt_arg.cont_list.take();
                    item.sp_syn.cont_in_list = syn_opt_arg.cont_in_list.take();
                    item.sp_cchar = conceal_char;
                    if item.sp_syn.cont_in_list.is_some() {
                        (*win_s(curwin_mut())).b_syn_containedin = 1;
                    }
                    item.sp_next_list = syn_opt_arg.next_list.take();
                    let pats = &mut (*win_s(curwin_mut())).b_syn_patterns;
                    pats.push(item);

                    if (syn_opt_arg.flags & (HL_SYNC_HERE | HL_SYNC_THERE)) != 0 {
                        (*win_s(curwin_mut())).b_syn_sync_flags |= SF_MATCH;
                    }
                    if (syn_opt_arg.flags & HL_FOLD) != 0 {
                        (*win_s(curwin_mut())).b_syn_folditems += 1;
                    }

                    redraw_curbuf_later(UPD_SOME_VALID);
                    syn_stack_free_all(win_s(curwin_mut()));
                    return;
                }
            }
        }

        if rest.is_null() {
            semsg!(gettext(e_invarg2), arg);
        }
    }
}

fn syn_cmd_region(eap: &mut exarg_T, syncing: c_int) {
    unsafe {
        let arg = eap.arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut key: Option<std::ffi::CString> = None;
        let mut pat_ptrs: [Vec<pat_ptr_T>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut pat_count: c_int = 0;
        let mut matchgroup_id: c_int = 0;
        let mut not_enough = false;
        let mut illegal = false;
        let mut success = false;
        let mut conceal_char: c_int = NUL as c_int;

        let mut rest = get_group_name(arg, &raw mut group_name_end);

        let mut syn_opt_arg = syn_opt_arg_T {
            flags: 0,
            keyword: false,
            sync_idx: None,
            has_cont_list: true,
            cont_list: None,
            cont_in_list: None,
            next_list: None,
        };

        while !rest.is_null() && ends_excmd(*rest as c_int) == 0 {
            rest = get_syn_options(
                rest,
                &mut syn_opt_arg,
                &mut conceal_char,
                eap.skip,
            );
            if rest.is_null() || ends_excmd(*rest as c_int) != 0 {
                break;
            }

            let mut key_end = rest;
            while *key_end != NUL && !ascii_iswhite(*key_end) && *key_end != b'=' as c_char {
                key_end = key_end.add(1);
            }
            key = Some(crate::memory::cstr_from_bytes(
                &std::slice::from_raw_parts(rest.cast::<u8>(), key_end.offset_from(rest) as usize)
                    .to_ascii_uppercase(),
            ));
            let key_ptr = key.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr());
            let item = if crate::memory::cstr_as_bytes(key_ptr) == b"MATCHGROUP" {
                ITEM_MATCHGROUP
            } else if crate::memory::cstr_as_bytes(key_ptr) == b"START" {
                ITEM_START
            } else if crate::memory::cstr_as_bytes(key_ptr) == b"END" {
                ITEM_END
            } else if crate::memory::cstr_as_bytes(key_ptr) == b"SKIP" {
                if !pat_ptrs[ITEM_SKIP].is_empty() {
                    illegal = true;
                    break;
                }
                ITEM_SKIP
            } else {
                break;
            };
            rest = skipwhite(key_end);
            if *rest != b'=' as c_char {
                rest = ptr::null_mut();
                semsg!(gettext(c"E398: Missing '=': %s"), arg);
                break;
            }
            rest = skipwhite(rest.add(1));
            if *rest == NUL {
                not_enough = true;
                break;
            }

            if item == ITEM_MATCHGROUP {
                let p = skiptowhite(rest);
                if (p.offset_from(rest) == 4 && (crate::memory::cstr_prefix_bytes(rest, 4) == &b"NONE"[..4]))
                    || eap.skip != 0
                {
                    matchgroup_id = 0;
                } else {
                    matchgroup_id = syn_check_group(rest, p.offset_from(rest) as usize);
                    if matchgroup_id == 0 {
                        illegal = true;
                        break;
                    }
                }
                rest = skipwhite(p);
            } else {
                let mut synp = synpat_T::new();

                if item == ITEM_START {
                    g_st().reg_do_extmatch = REX_SET;
                } else {
                    g_st().reg_do_extmatch = REX_USE;
                }
                rest = get_syn_pattern(rest, &mut synp);
                g_st().reg_do_extmatch = 0;
                if item == ITEM_END
                    && vim_regcomp_had_eol() != 0
                    && (syn_opt_arg.flags & HL_EXCLUDENL) == 0
                {
                    synp.sp_flags |= HL_HAS_EOL;
                }
                pat_ptrs[item].insert(
                    0,
                    pat_ptr_T {
                        pp_synp: synp,
                        pp_matchgroup_id: matchgroup_id,
                    },
                );
                pat_count += 1;
            }
        }
        drop(key);
        if illegal || not_enough {
            rest = ptr::null_mut();
        }

        if !rest.is_null() && (pat_ptrs[ITEM_START].is_empty() || pat_ptrs[ITEM_END].is_empty()) {
            not_enough = true;
            rest = ptr::null_mut();
        }

        if !rest.is_null() {
            eap.nextcmd = check_nextcmd(rest);
            if ends_excmd(*rest as c_int) == 0 || eap.skip != 0 {
                rest = ptr::null_mut();
            } else {
                let syn_id = syn_check_group(arg, group_name_end.offset_from(arg) as usize);
                if syn_id != 0 {
                    syn_incl_toplevel(syn_id, &mut syn_opt_arg.flags);
                    let pats = &mut (*win_s(curwin_mut())).b_syn_patterns;
                    pats.reserve(pat_count as usize);
                    for (item, pat_vec) in pat_ptrs
                        .iter_mut()
                        .enumerate()
                        .take(ITEM_END + 1)
                        .skip(ITEM_START)
                    {
                        for ppp in pat_vec.drain(..) {
                            let mut pat = ppp.pp_synp;
                            pat.sp_syncing = syncing != 0;
                            pat.sp_type = if item == ITEM_START {
                                SPTYPE_START
                            } else if item == ITEM_SKIP {
                                SPTYPE_SKIP
                            } else {
                                SPTYPE_END
                            };
                            pat.sp_flags |= syn_opt_arg.flags;
                            pat.sp_syn.id = syn_id as i16;
                            pat.sp_syn.inc_tag = syn_st().current_syn_inc_tag;
                            pat.sp_syn_match_id = ppp.pp_matchgroup_id as i16;
                            pat.sp_cchar = conceal_char;
                            if item == ITEM_START {
                                pat.sp_cont_list = syn_opt_arg.cont_list.clone();
                                pat.sp_syn.cont_in_list = syn_opt_arg.cont_in_list.clone();
                                if syn_opt_arg.cont_in_list.is_some() {
                                    (*win_s(curwin_mut())).b_syn_containedin = 1;
                                }
                                pat.sp_next_list = syn_opt_arg.next_list.clone();
                            }
                            pats.push(pat);
                            if (syn_opt_arg.flags & HL_FOLD) != 0 {
                                (*win_s(curwin_mut())).b_syn_folditems += 1;
                            }
                        }
                    }

                    redraw_curbuf_later(UPD_SOME_VALID);
                    syn_stack_free_all(win_s(curwin_mut()));
                    success = true;
                }
            }
        }

        if !success {
            if not_enough {
                semsg!(
                    gettext(c"E399: Not enough arguments: syntax region %s"),
                    arg,
                );
            } else if illegal || rest.is_null() {
                semsg!(gettext(e_invarg2), arg);
            }
        }
    }
}

fn syn_combine_list(clstr1: &mut SynIdList, clstr2: &mut SynIdList, list_op: c_int) {
    let Some(rc2) = clstr2.take() else {
        return;
    };
    if clstr1.is_none() || list_op == CLUSTER_REPLACE {
        if list_op == CLUSTER_REPLACE || list_op == CLUSTER_ADD {
            *clstr1 = Some(rc2);
        }
        return;
    }

    let rc1 = clstr1.take().unwrap();
    let count1 = rc1.iter().position(|&x| x == 0).unwrap_or(rc1.len());
    let count2 = rc2.iter().position(|&x| x == 0).unwrap_or(rc2.len());

    let mut v1: Vec<i16> = rc1[..count1].to_vec();
    let mut v2: Vec<i16> = rc2[..count2].to_vec();
    v1.sort_unstable();
    v2.sort_unstable();

    let mut out: Vec<i16> = Vec::with_capacity(count1 + count2 + 1);
    let mut i1 = 0;
    let mut i2 = 0;
    while i1 < v1.len() && i2 < v2.len() {
        if v1[i1] < v2[i2] {
            out.push(v1[i1]);
            i1 += 1;
            continue;
        }
        if list_op == CLUSTER_ADD {
            out.push(v2[i2]);
        }
        if v1[i1] == v2[i2] {
            i1 += 1;
        }
        i2 += 1;
    }
    out.extend_from_slice(&v1[i1..]);
    if list_op == CLUSTER_ADD {
        out.extend_from_slice(&v2[i2..]);
    }

    if !out.is_empty() {
        out.push(0);
        *clstr1 = Some(std::rc::Rc::from(out));
    }
}

fn syn_scl_name2id(name: *const c_char) -> c_int {
    unsafe {
        let name_u = crate::memory::cstr_from_bytes(
            &crate::memory::cstr_as_bytes(name).to_ascii_uppercase(),
        );
        let mut i = (*win_s(curwin_mut())).b_syn_clusters.len() as c_int - 1;
        while i >= 0 {
            let clp = syn_clstr(&mut *win_s(curwin_mut())).offset(i as isize);
            if crate::memory::cstr_as_bytes(name_u.as_ptr()) == crate::memory::cstr_as_bytes((*clp).scl_name_u.as_ptr()) {
                break;
            }
            i -= 1;
        }
        if i < 0 { 0 } else { i + SYNID_CLUSTER as c_int }
    }
}

fn syn_scl_namen2id(linep: *const c_char, len: c_int) -> c_int {
    unsafe {
        let bytes = std::slice::from_raw_parts(linep.cast::<u8>(), len as usize);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let name = std::ffi::CString::new(&bytes[..end]).unwrap();
        syn_scl_name2id(name.as_ptr())
    }
}

fn syn_check_cluster(pp: *const c_char, len: c_int) -> c_int {
    unsafe {
        let bytes = std::slice::from_raw_parts(pp.cast::<u8>(), len as usize);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let name = std::ffi::CString::new(&bytes[..end]).unwrap();
        let mut id = syn_scl_name2id(name.as_ptr());
        if id == 0 {
            id = syn_add_cluster(name);
        }
        id
    }
}

fn syn_add_cluster(name: std::ffi::CString) -> c_int {
    unsafe {
        let len = (*win_s(curwin_mut())).b_syn_clusters.len() as c_int;
        if len >= MAX_CLUSTER_ID {
            emsg(gettext(c"E848: Too many syntax clusters"));
            return 0;
        }

        let name_ptr = name.as_ptr();
        let is_spell = crate::memory::cstr_as_bytes(name_ptr).eq_ignore_ascii_case(b"Spell");
        let is_nospell = crate::memory::cstr_as_bytes(name_ptr).eq_ignore_ascii_case(b"NoSpell");
        let scl_name_u = crate::memory::cstr_from_bytes(
            &crate::memory::cstr_as_bytes(name_ptr).to_ascii_uppercase(),
        );

        let clusters = &mut (*win_s(curwin_mut())).b_syn_clusters;
        clusters.push(syn_cluster_T {
            scl_name: name,
            scl_name_u,
            scl_list: None,
        });

        if is_spell {
            (*win_s(curwin_mut())).b_spell_cluster_id = len + SYNID_CLUSTER as c_int;
        }
        if is_nospell {
            (*win_s(curwin_mut())).b_nospell_cluster_id = len + SYNID_CLUSTER as c_int;
        }

        len + SYNID_CLUSTER as c_int
    }
}

fn syn_cmd_cluster(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = eap.arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut got_clstr = false;

        eap.nextcmd = find_nextcmd(arg);
        if eap.skip != 0 {
            return;
        }

        let mut rest = get_group_name(arg, &raw mut group_name_end);

        if !rest.is_null() {
            let mut scl_id = syn_check_cluster(arg, group_name_end.offset_from(arg) as c_int);
            if scl_id == 0 {
                return;
            }
            scl_id -= SYNID_CLUSTER as c_int;

            loop {
                let (opt_len, list_op) = if crate::memory::cstr_prefix_bytes(rest, 3).eq_ignore_ascii_case(b"add")
                    && (ascii_iswhite(*rest.add(3)) || *rest.add(3) == b'=' as c_char)
                {
                    (3, CLUSTER_ADD)
                } else if crate::memory::cstr_prefix_bytes(rest, 6).eq_ignore_ascii_case(b"remove")
                    && (ascii_iswhite(*rest.add(6)) || *rest.add(6) == b'=' as c_char)
                {
                    (6, CLUSTER_SUBTRACT)
                } else if crate::memory::cstr_prefix_bytes(rest, 8).eq_ignore_ascii_case(b"contains")
                    && (ascii_iswhite(*rest.add(8)) || *rest.add(8) == b'=' as c_char)
                {
                    (8, CLUSTER_REPLACE)
                } else {
                    break;
                };

                let mut clstr_list: SynIdList = None;
                if !get_id_list(
                    &raw mut rest,
                    opt_len,
                    &mut clstr_list,
                    eap.skip != 0,
                )
                {
                    semsg!(gettext(e_invarg2), rest);
                    break;
                }
                if scl_id >= 0 {
                    let clp = syn_clstr(&mut *win_s(curwin_mut())).offset(scl_id as isize);
                    syn_combine_list(&mut (*clp).scl_list, &mut clstr_list, list_op);
                }
                got_clstr = true;
            }

            if got_clstr {
                redraw_curbuf_later(UPD_SOME_VALID);
                syn_stack_free_all(win_s(curwin_mut()));
            }
        }

        if !got_clstr {
            emsg(gettext(c"E400: No cluster specified"));
        }
        if rest.is_null() || ends_excmd(*rest as c_int) == 0 {
            semsg!(gettext(e_invarg2), arg);
        }
    }
}

fn get_syn_pattern(arg: *mut c_char, ci: &mut synpat_T) -> *mut c_char {
    unsafe {
        if arg.is_null() || *arg == NUL || *arg.add(1) == NUL || *arg.add(2) == NUL {
            return ptr::null_mut();
        }

        let mut end = skip_regexp(arg.add(1), *arg as c_int, 1);
        if *end != *arg {
            semsg!(
                gettext(c"E401: Pattern delimiter not found: %s"),
                arg,
            );
            return ptr::null_mut();
        }
        let pat_slice =
            std::slice::from_raw_parts(arg.add(1).cast::<u8>(), end.offset_from(arg) as usize - 1);
        let pat_cstr = std::ffi::CString::new(pat_slice).unwrap();

        let cpo_save = opt_vars().p_cpo;
        opt_vars().p_cpo = empty_string_option.as_ptr().cast();
        ci.sp_prog = crate::regexp::vim_regcomp_box(pat_cstr.as_ptr(), RE_MAGIC);
        opt_vars().p_cpo = cpo_save;
        ci.sp_pattern = Some(pat_cstr);

        if ci.sp_prog.is_none() {
            return ptr::null_mut();
        }
        ci.sp_ic = (*win_s(curwin_mut())).b_syn_ic;
        syn_clear_time(&mut ci.sp_time);

        end = end.add(1);
        loop {
            let mut idx: isize = SPO_COUNT as isize - 1;
            while idx >= 0 {
                if crate::memory::cstr_prefix_bytes(end, 3) == crate::memory::cstr_prefix_bytes(spo_name_tab[idx as usize], 3) {
                    break;
                }
                idx -= 1;
            }
            if idx >= 0 {
                let base_idx = idx as usize;
                let p = &raw mut ci.sp_offsets[base_idx];
                if base_idx != SPO_LC_OFF {
                    match *end.add(3) as u8 {
                        b's' | b'b' => {}
                        b'e' => idx += SPO_COUNT as isize,
                        _ => idx = -1,
                    }
                }
                if idx >= 0 {
                    ci.sp_off_flags |= 1 << (idx as usize);
                    if base_idx == SPO_LC_OFF {
                        end = end.add(3);
                        *p = getdigits_int(&raw mut end, true, 0);

                        if (ci.sp_off_flags & (1 << SPO_MS_OFF)) == 0 {
                            ci.sp_off_flags |= 1 << SPO_MS_OFF;
                            ci.sp_offsets[SPO_MS_OFF] = *p;
                        }
                    } else {
                        end = end.add(4);
                        if *end == b'+' as c_char {
                            end = end.add(1);
                            *p = getdigits_int(&raw mut end, true, 0);
                        } else if *end == b'-' as c_char {
                            end = end.add(1);
                            *p = -getdigits_int(&raw mut end, true, 0);
                        }
                    }
                    if *end != b',' as c_char {
                        break;
                    }
                    end = end.add(1);
                }
            }
            if idx < 0 {
                break;
            }
        }

        if ends_excmd(*end as c_int) == 0 && !ascii_iswhite(*end) {
            semsg!(gettext(c"E402: Garbage after pattern: %s"), arg);
            return ptr::null_mut();
        }
        skipwhite(end)
    }
}

fn syn_cmd_sync(eap: &mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg_start = eap.arg;
        let mut key_owned: Option<std::ffi::CString>;
        let mut illegal = false;
        let mut finished = false;

        if ends_excmd(*arg_start as c_int) != 0 {
            syn_cmd_list(eap, 1);
            return;
        }

        while ends_excmd(*arg_start as c_int) == 0 {
            let mut arg_end = skiptowhite(arg_start);
            let mut next_arg = skipwhite(arg_end);
            key_owned = Some(crate::memory::cstr_from_bytes(
                &std::slice::from_raw_parts(
                    arg_start.cast::<u8>(),
                    arg_end.offset_from(arg_start) as usize,
                )
                .to_ascii_uppercase(),
            ));
            let key = key_owned.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr());
            if crate::memory::cstr_as_bytes(key) == b"CCOMMENT" {
                if eap.skip == 0 {
                    (*win_s(curwin_mut())).b_syn_sync_flags |= SF_CCOMMENT;
                }
                if ends_excmd(*next_arg as c_int) == 0 {
                    arg_end = skiptowhite(next_arg);
                    if eap.skip == 0 {
                        (*win_s(curwin_mut())).b_syn_sync_id =
                            syn_check_group(next_arg, arg_end.offset_from(next_arg) as usize)
                                as i16;
                    }
                    next_arg = skipwhite(arg_end);
                } else if eap.skip == 0 {
                    (*win_s(curwin_mut())).b_syn_sync_id = syn_name2id(c"Comment") as i16;
                }
            } else if (crate::memory::cstr_prefix_bytes(key, 5) == &b"LINES"[..5])
                || (crate::memory::cstr_prefix_bytes(key, 8) == &b"MINLINES"[..8])
                || (crate::memory::cstr_prefix_bytes(key, 8) == &b"MAXLINES"[..8])
                || (crate::memory::cstr_prefix_bytes(key, 10) == &b"LINEBREAKS"[..10])
            {
                arg_end = (if *key.add(4) == b'S' as c_char {
                    key.add(6)
                } else if *key == b'L' as c_char {
                    key.add(11)
                } else {
                    key.add(9)
                })
                .cast_mut();
                if *arg_end.sub(1) != b'=' as c_char || !ascii_isdigit(*arg_end) {
                    illegal = true;
                    break;
                }
                let n: linenr_T = getdigits_int32(&raw mut arg_end, false, 0);
                if eap.skip == 0 {
                    if *key.add(4) == b'B' as c_char {
                        (*win_s(curwin_mut())).b_syn_sync_linebreaks = n;
                    } else if *key.add(1) == b'A' as c_char {
                        (*win_s(curwin_mut())).b_syn_sync_maxlines = n;
                    } else {
                        (*win_s(curwin_mut())).b_syn_sync_minlines = n;
                    }
                }
            } else if crate::memory::cstr_as_bytes(key) == b"FROMSTART" {
                if eap.skip == 0 {
                    (*win_s(curwin_mut())).b_syn_sync_minlines = MAXLNUM;
                    (*win_s(curwin_mut())).b_syn_sync_maxlines = 0;
                }
            } else if crate::memory::cstr_as_bytes(key) == b"LINECONT" {
                if *next_arg == NUL {
                    illegal = true;
                    break;
                }
                if (*win_s(curwin_mut())).b_syn_linecont_pat.is_some() {
                    emsg(gettext(
                        c"E403: syntax sync: line continuations pattern specified twice",
                    ));
                    finished = true;
                    break;
                }
                arg_end = skip_regexp(next_arg.add(1), *next_arg as c_int, 1);
                if *arg_end != *next_arg {
                    illegal = true;
                    break;
                }

                if eap.skip == 0 {
                    let pat_slice = std::slice::from_raw_parts(
                        next_arg.add(1).cast::<u8>(),
                        arg_end.offset_from(next_arg) as usize - 1,
                    );
                    let pat_cstr = std::ffi::CString::new(pat_slice).unwrap();
                    (*win_s(curwin_mut())).b_syn_linecont_ic = (*win_s(curwin_mut())).b_syn_ic;

                    let cpo_save = opt_vars().p_cpo;
                    opt_vars().p_cpo = empty_string_option.as_ptr().cast();
                    (*win_s(curwin_mut())).b_syn_linecont_prog =
                        crate::regexp::vim_regcomp_box(pat_cstr.as_ptr(), RE_MAGIC);
                    opt_vars().p_cpo = cpo_save;
                    syn_clear_time(&mut (*win_s(curwin_mut())).b_syn_linecont_time);

                    if (*win_s(curwin_mut())).b_syn_linecont_prog.is_none() {
                        (*win_s(curwin_mut())).b_syn_linecont_pat = None;
                        finished = true;
                        break;
                    }
                    (*win_s(curwin_mut())).b_syn_linecont_pat = Some(pat_cstr);
                }
                next_arg = skipwhite(arg_end.add(1));
            } else {
                eap.arg = next_arg;
                if crate::memory::cstr_as_bytes(key) == b"MATCH" {
                    syn_cmd_match(eap, 1);
                } else if crate::memory::cstr_as_bytes(key) == b"REGION" {
                    syn_cmd_region(eap, 1);
                } else if crate::memory::cstr_as_bytes(key) == b"CLEAR" {
                    syn_cmd_clear(eap, 1);
                } else {
                    illegal = true;
                }
                finished = true;
                break;
            }
            arg_start = next_arg;
        }
        if illegal {
            semsg!(gettext(c"E404: Illegal arguments: %s"), arg_start);
        } else if !finished {
            eap.nextcmd = check_nextcmd(arg_start);
            redraw_curbuf_later(UPD_SOME_VALID);
            syn_stack_free_all(win_s(curwin_mut()));
        }
    }
}

fn get_id_list(
    arg: *mut *mut c_char,
    keylen: c_int,
    list: &mut SynIdList,
    skip: bool,
) -> bool {
    unsafe {
        let mut p: *mut c_char = ptr::null_mut();
        let mut total_count: c_int = 0;
        let mut retval: Vec<i16> = Vec::new();
        let mut regmatch: regmatch_T = regmatch_T::new();
        let mut failed = false;

        let mut round: c_int = 1;
        while round <= 2 {
            p = skipwhite((*arg).offset(keylen as isize));
            if *p != b'=' as c_char {
                semsg!(gettext(c"E405: Missing equal sign: %s"), *arg);
                break;
            }
            p = skipwhite(p.add(1));
            if ends_excmd(*p as c_int) != 0 {
                semsg!(gettext(c"E406: Empty argument: %s"), *arg);
                break;
            }

            let mut count: c_int = 0;
            loop {
                let mut end = p;
                while *end != NUL && !ascii_iswhite(*end) && *end != b',' as c_char {
                    end = end.add(1);
                }
                let mut name_buf = vec![0 as c_char; end.offset_from(p) as usize + 3];
                let name = name_buf.as_mut_ptr();
                crate::memory::cstr_ncpy(name.add(1), p, end.offset_from(p) as usize);
                let mut id: c_int;
                if (crate::memory::cstr_as_bytes(name.add(1)) == b"ALLBUT")
                    || (crate::memory::cstr_as_bytes(name.add(1)) == b"ALL")
                    || (crate::memory::cstr_as_bytes(name.add(1)) == b"TOP")
                    || (crate::memory::cstr_as_bytes(name.add(1)) == b"CONTAINED")
                {
                    if !(**arg as u8).eq_ignore_ascii_case(&b'C') {
                        semsg!(gettext(c"E407: %s not allowed here"), name.add(1));
                        failed = true;
                        break;
                    }
                    if count != 0 {
                        semsg!(
                            gettext(c"E408: %s must be first in contains list"),
                            name.add(1),
                        );
                        failed = true;
                        break;
                    }
                    id = if *name.add(1) == b'A' as c_char {
                        SYNID_ALLBUT as c_int
                    } else if *name.add(1) == b'T' as c_char {
                        SYNID_TOP as c_int
                    } else {
                        SYNID_CONTAINED as c_int
                    };
                    id += syn_st().current_syn_inc_tag;
                } else if *name.add(1) == b'@' as c_char {
                    if skip {
                        id = -1;
                    } else {
                        id = syn_check_cluster(name.add(2), end.offset_from(p) as c_int - 1);
                    }
                } else if !crate::memory::cstr_as_bytes(name.add(1)).iter().any(|b| b"\\.*^$~[".contains(b)) {
                    id = syn_check_group(name.add(1), end.offset_from(p) as usize);
                } else {
                    *name = b'^' as c_char;
                    crate::memory::cstr_cat(name.add(1), c"$");
                    regmatch.regprog = vim_regcomp(name, RE_MAGIC);
                    if regmatch.regprog.is_null() {
                        failed = true;
                        break;
                    }

                    regmatch.rm_ic = true;
                    id = 0;
                    let mut i = highlight_num_groups() - 1;
                    while i >= 0 {
                        if vim_regexec(&mut regmatch, highlight_group_name(i), 0) {
                            if round == 2 {
                                if count >= total_count {
                                    retval.clear();
                                    round = 1;
                                } else {
                                    retval[count as usize] = (i + 1) as i16;
                                }
                            }
                            count += 1;
                            id = -1;
                        }
                        i -= 1;
                    }
                    vim_regfree(regmatch.regprog);
                }
                if id == 0 {
                    semsg!(gettext(c"E409: Unknown group name: %s"), p);
                    failed = true;
                    break;
                }
                if id > 0 {
                    if round == 2 {
                        if count >= total_count {
                            retval.clear();
                            round = 1;
                        } else {
                            retval[count as usize] = id as i16;
                        }
                    }
                    count += 1;
                }
                p = skipwhite(end);
                if *p != b',' as c_char {
                    break;
                }
                p = skipwhite(p.add(1));
                if ends_excmd(*p as c_int) != 0 {
                    break;
                }
            }
            if failed {
                break;
            }
            if round == 1 {
                retval = vec![0; count as usize + 1];
                total_count = count;
            }
            round += 1;
        }

        *arg = p;
        if failed || retval.is_empty() {
            return false;
        }

        if list.is_none() {
            *list = Some(std::rc::Rc::from(retval));
        }
        true
    }
}

fn idl_collect(list: &[i16], ga: &mut Vec<i16>, seen: &mut Vec<i16>) -> bool {
    unsafe {
        if list.is_empty() {
            return false;
        }
        for &item in list {
            if item == 0 {
                break;
            }
            if (SYNID_ALLBUT..SYNID_CLUSTER).contains(&item) {
                return false;
            }
            if item >= SYNID_CLUSTER {
                if !seen.contains(&item) {
                    seen.push(item);
                    ga.push(item);

                    let scl = &(*syn_clstr(&mut *syn_st().syn_block).offset((item - SYNID_CLUSTER) as isize));
                    if !scl.scl_list.as_ref().is_some_and(|rc| !rc.is_empty())
                        || !idl_collect(scl.scl_list.as_deref().unwrap(), ga, seen)
                    {
                        return false;
                    }
                }
            } else {
                ga.push(item);
            }
        }
        true
    }
}

fn idl_get_entry(list: &SynIdList) -> *mut idl_entry_T {
    unsafe {
        let cache = (*syn_st().syn_block)
            .b_idlist_cache
            .get_or_insert_with(|| Box::new(idl_cache_T::default()));
        let last = cache.last as usize;
        if syn_id_list_eq(&cache.ent[last].idl_key, list) {
            return &raw mut cache.ent[last];
        }
        for i in 0..IDL_CACHE_SIZE {
            if syn_id_list_eq(&cache.ent[i].idl_key, list) {
                cache.last = i as c_int;
                return &raw mut cache.ent[i];
            }
        }

        let round = cache.round as usize;
        cache.last = cache.round;
        let e = &raw mut cache.ent[round];
        cache.round = (cache.round + 1) % (IDL_CACHE_SIZE as c_int);
        (*e).idl_ids.clear();
        (*e).idl_key = list.clone();
        (*e).idl_marker = 0;
        (*e).idl_usable = true;

        let mut slice: &[i16] = list.as_deref().unwrap_or(&[]);
        if let Some(&first) = slice.first()
            && (SYNID_ALLBUT..SYNID_CLUSTER).contains(&first)
        {
            (*e).idl_marker = first;
            slice = &slice[1..];
        }

        let mut ga: Vec<i16> = Vec::with_capacity(50);
        let mut seen: Vec<i16> = Vec::with_capacity(10);
        if !idl_collect(slice, &mut ga, &mut seen) {
            (*e).idl_usable = false;
            return e;
        }
        if !ga.is_empty() {
            ga.sort_unstable();
            (*e).idl_ids = ga;
        }
        e
    }
}

fn idl_contains(e: &idl_entry_T, id: i16) -> bool {
    e.idl_ids.binary_search(&id).is_ok()
}

fn idl_cache_clear(block: *mut synblock_T) {
    unsafe {
        (*block).b_idlist_cache = None;
    }
}

fn in_id_list(
    mut cur_si: *mut stateitem_T,
    list: &SynIdList,ssp: &mut sp_syn,
    flags: c_int,
) -> c_int {
    unsafe {
        let id = ssp.id;

        if !cur_si.is_null()
            && ssp.cont_in_list.is_some()
            && ((*cur_si).si_flags & HL_MATCH) == 0
        {
            while ((*cur_si).si_flags & HL_TRANS_CONT) != 0 && cur_si > cur_state_at(0) {
                cur_si = cur_si.sub(1);
            }
            if (*cur_si).si_idx >= 0 {
                let c_spp = syn_items(&mut *syn_st().syn_block).offset((*cur_si).si_idx as isize);
                if in_id_list(
                    ptr::null_mut(),
                    &ssp.cont_in_list,
                    &mut (*c_spp).sp_syn,
                    (*c_spp).sp_flags,
                ) != 0
                {
                    return 1;
                }
            }
        }

        let Some(ref list_rc) = *list else {
            return 0;
        };

        if is_id_list_all(list) {
            return c_int::from((flags & HL_CONTAINED) == 0);
        }

        let toplevel = (flags & HL_CONTAINED) == 0 || (flags & HL_INCLUDED_TOPLEVEL) != 0;

        let e = idl_get_entry(list);
        if !e.is_null() && (*e).idl_usable {
            let retval = if (*e).idl_marker != 0 {
                let item = (*e).idl_marker;
                if item < SYNID_TOP {
                    if (item - SYNID_ALLBUT) as c_int != ssp.inc_tag {
                        return 0;
                    }
                } else if item < SYNID_CONTAINED {
                    if (item - SYNID_TOP) as c_int != ssp.inc_tag || !toplevel {
                        return 0;
                    }
                } else if (item - SYNID_CONTAINED) as c_int != ssp.inc_tag || toplevel {
                    return 0;
                }
                false
            } else {
                true
            };
            return c_int::from(if idl_contains(&*e, id) { retval } else { !retval });
        }

        let mut slice: &[i16] = list_rc;
        let first = slice.first().copied().unwrap_or(0);
        let retval = if (SYNID_ALLBUT..SYNID_CLUSTER).contains(&first) {
            if first < SYNID_TOP {
                if (first - SYNID_ALLBUT) as c_int != ssp.inc_tag {
                    return 0;
                }
            } else if first < SYNID_CONTAINED {
                if (first - SYNID_TOP) as c_int != ssp.inc_tag || !toplevel {
                    return 0;
                }
            } else if (first - SYNID_CONTAINED) as c_int != ssp.inc_tag || toplevel {
                return 0;
            }
            slice = &slice[1..];
            false
        } else {
            true
        };

        for &item in slice {
            if item == 0 {
                break;
            }
            if item == id {
                return c_int::from(retval);
            }
            if item >= SYNID_CLUSTER {
                let scl_list =
                    &(*syn_clstr(&mut *syn_st().syn_block).offset((item - SYNID_CLUSTER) as isize)).scl_list;
                if scl_list.is_some() && syn_st().in_id_list_depth < 30 {
                    syn_st().in_id_list_depth += 1;
                    let r = in_id_list(ptr::null_mut(), scl_list, &mut *ssp, flags);
                    syn_st().in_id_list_depth -= 1;
                    if r != 0 {
                        return c_int::from(retval);
                    }
                }
            }
        }
        c_int::from(!retval)
    }
}

type SubcmdFn = fn(&mut exarg_T, c_int);

const SUBCOMMANDS: [(*const c_char, SubcmdFn); 19] = [
    (c"case".as_ptr(), syn_cmd_case),
    (c"clear".as_ptr(), syn_cmd_clear),
    (c"cluster".as_ptr(), syn_cmd_cluster),
    (c"conceal".as_ptr(), syn_cmd_conceal),
    (c"enable".as_ptr(), syn_cmd_on),
    (c"foldlevel".as_ptr(), syn_cmd_foldlevel),
    (c"include".as_ptr(), syn_cmd_include),
    (c"iskeyword".as_ptr(), syn_cmd_iskeyword),
    (c"keyword".as_ptr(), syn_cmd_keyword),
    (c"list".as_ptr(), syn_cmd_list),
    (c"manual".as_ptr(), syn_cmd_manual),
    (c"match".as_ptr(), syn_cmd_match),
    (c"on".as_ptr(), syn_cmd_on),
    (c"off".as_ptr(), syn_cmd_off),
    (c"region".as_ptr(), syn_cmd_region),
    (c"reset".as_ptr(), syn_cmd_reset),
    (c"spell".as_ptr(), syn_cmd_spell),
    (c"sync".as_ptr(), syn_cmd_sync),
    (c"".as_ptr(), syn_cmd_list),
];

/// ":syntax".
pub fn ex_syntax(eap: &mut exarg_T) {
    unsafe {
        let arg = eap.arg;

        let mut subcmd_end = arg;
        while ascii_isalpha(*subcmd_end) {
            subcmd_end = subcmd_end.add(1);
        }
        let subcmd_slice =
            std::slice::from_raw_parts(arg.cast::<u8>(), subcmd_end.offset_from(arg) as usize);
        let subcmd_name = std::ffi::CString::new(subcmd_slice).unwrap();
        if eap.skip != 0 {
            g_st().emsg_skip += 1;
        }
        let mut found = false;
        for &(name, func) in &SUBCOMMANDS {
            if crate::memory::cstr_as_bytes(subcmd_name.as_ptr()) == crate::memory::cstr_as_bytes(name) {
                eap.arg = skipwhite(subcmd_end);
                func(eap, 0);
                found = true;
                break;
            }
        }

        if !found {
            semsg!(
                gettext(c"E410: Invalid :syntax subcommand: %s"),
                subcmd_name.as_ptr(),
            );
        }

        if eap.skip != 0 {
            g_st().emsg_skip -= 1;
        }
    }
}

/// @deprecated
pub fn ex_ownsyntax(eap: &mut exarg_T) {
    unsafe {
        if curwin_mut().w_s_owned.is_none() {
            let mut new_s = Box::new(synblock_T::new());
            let new_s_ptr: *mut synblock_T = &raw mut *new_s;
            curwin_mut().w_s_owned = Some(new_s);
            curwin_mut().w_onebuf_opt.wo_spell = 0;
            clear_string_option(&raw mut (*new_s_ptr).b_p_spc);
            clear_string_option(&raw mut (*new_s_ptr).b_p_spf);
            clear_string_option(&raw mut (*new_s_ptr).b_p_spl);
            clear_string_option(&raw mut (*new_s_ptr).b_p_spo);
            (*new_s_ptr).b_syn_isk = None;
        }

        let raw_old = get_var_value(c"b:current_syntax");
        let old_value = if raw_old.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(raw_old))
        };

        apply_autocmds(EVENT_SYNTAX, eap.arg, (*g_st().curbuf).b_fname, true, g_st().curbuf);

        let new_value = get_var_value(c"b:current_syntax");
        if !new_value.is_null() {
            set_internal_string_var(c"w:current_syntax", new_value);
        }

        if let Some(ref old_cstr) = old_value {
            set_internal_string_var(c"b:current_syntax", old_cstr.as_ptr().cast_mut());
        } else {
            do_unlet(c"b:current_syntax", 16, true);
        }
    }
}

pub fn syntax_present(win: *mut win_T) -> bool {
    unsafe {
        !(*win_s(&mut *win)).b_syn_patterns.is_empty()
            || !(*win_s(&mut *win)).b_syn_clusters.is_empty()
            || !(*win_s(&mut *win)).b_keywtab.is_empty()
            || !(*win_s(&mut *win)).b_keywtab_ic.is_empty()
    }
}

pub fn reset_expand_highlight() {
    g_st().include_none = 0;
        g_st().include_default = 0;
        g_st().include_link = 0;
}

pub fn set_context_in_echohl_cmd(xp: &mut expand_T, arg: *const c_char) {
    xp.xp_context = EXPAND_HIGHLIGHT;
        xp.xp_pattern = arg.cast_mut();
        g_st().include_none = 1;
}

pub fn set_context_in_syntax_cmd(xp: &mut expand_T, arg: *const c_char) {
    unsafe {
        xp.xp_context = EXPAND_SYNTAX;
        syn_st().expand_what = EXP_SUBCMD;
        xp.xp_pattern = arg.cast_mut();
        g_st().include_link = 0;
        g_st().include_default = 0;

        if *arg == NUL {
            return;
        }

        let p = skiptowhite(arg);
        if *p == NUL {
            return;
        }

        xp.xp_pattern = skipwhite(p);
        if *skiptowhite(xp.xp_pattern) != NUL {
            xp.xp_context = EXPAND_NOTHING;
        } else {
            let n = p.offset_from(arg) as usize;
            if crate::memory::cstr_prefix_bytes(arg, n).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"case", n)) {
                syn_st().expand_what = EXP_CASE;
            } else if crate::memory::cstr_prefix_bytes(arg, n).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"spell", n)) {
                syn_st().expand_what = EXP_SPELL;
            } else if crate::memory::cstr_prefix_bytes(arg, n).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"sync", n)) {
                syn_st().expand_what = EXP_SYNC;
            } else if crate::memory::cstr_prefix_bytes(arg, n).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"list", n)) {
                let p2 = skipwhite(p);
                if *p2 == b'@' as c_char {
                    syn_st().expand_what = EXP_CLUSTER;
                } else {
                    xp.xp_context = EXPAND_HIGHLIGHT;
                }
            } else if crate::memory::cstr_prefix_bytes(arg, n).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"keyword", n))
                || crate::memory::cstr_prefix_bytes(arg, n).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"region", n))
                || crate::memory::cstr_prefix_bytes(arg, n).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"match", n))
            {
                xp.xp_context = EXPAND_HIGHLIGHT;
            } else {
                xp.xp_context = EXPAND_NOTHING;
            }
        }
    }
}

pub fn get_syntax_name(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx < 0 {
            return ptr::null_mut();
        }
        let u = idx as usize;
        match syn_st().expand_what {
            EXP_SUBCMD => {
                if u >= SUBCOMMANDS.len() {
                    ptr::null_mut()
                } else {
                    SUBCOMMANDS[u].0.cast_mut()
                }
            }
            EXP_CASE => {
                const CASE_ARGS: [*const c_char; 3] =
                    [c"match".as_ptr(), c"ignore".as_ptr(), std::ptr::null()];
                if u >= CASE_ARGS.len() {
                    ptr::null_mut()
                } else {
                    CASE_ARGS[u].cast_mut()
                }
            }
            EXP_SPELL => {
                const SPELL_ARGS: [*const c_char; 4] = [
                    c"toplevel".as_ptr(),
                    c"notoplevel".as_ptr(),
                    c"default".as_ptr(),
                    std::ptr::null(),
                ];
                if u >= SPELL_ARGS.len() {
                    ptr::null_mut()
                } else {
                    SPELL_ARGS[u].cast_mut()
                }
            }
            EXP_SYNC => {
                const SYNC_ARGS: [*const c_char; 11] = [
                    c"ccomment".as_ptr(),
                    c"clear".as_ptr(),
                    c"fromstart".as_ptr(),
                    c"linebreaks=".as_ptr(),
                    c"linecont".as_ptr(),
                    c"lines=".as_ptr(),
                    c"match".as_ptr(),
                    c"maxlines=".as_ptr(),
                    c"minlines=".as_ptr(),
                    c"region".as_ptr(),
                    std::ptr::null(),
                ];
                if u >= SYNC_ARGS.len() {
                    ptr::null_mut()
                } else {
                    SYNC_ARGS[u].cast_mut()
                }
            }
            EXP_CLUSTER => {
                if (idx as usize) < (*win_s(curwin_mut())).b_syn_clusters.len() {
                    let xp_buf = xp.xp_buf.as_mut_ptr();
                    vim_snprintf!(
                        xp_buf,
                        EXPAND_BUF_LEN,
                        c"@%s",
                        (*syn_clstr(&mut *win_s(curwin_mut())).offset(idx as isize))
                            .scl_name
                            .as_ptr(),
                    );
                    xp_buf
                } else {
                    ptr::null_mut()
                }
            }
            _ => ptr::null_mut(),
        }
    }
}

/// Function called for expression evaluation: get syntax ID at file position.
pub fn syn_get_id(
    wp: *mut win_T,
    lnum: linenr_T,
    col: colnr_T,
    trans: c_int,spellp: Option<&mut bool>,
    keep_state: c_int,
) -> c_int {
    unsafe {
        if wp != syn_st().syn_win || (*wp).w_buffer != syn_st().syn_buf || lnum != syn_st().current_lnum || col < syn_st().current_col {
            syntax_start(wp, lnum);
        } else if col > syn_st().current_col {
            syn_st().next_match_idx = -1;
        }

        get_syntax_attr(
            col,
            spellp.map_or(ptr::null_mut(), |p| &raw mut *p),
            keep_state != 0,
        );

        if trans != 0 {
            syn_st().current_trans_id
        } else {
            syn_st().current_id
        }
    }
}

pub fn get_syntax_info(seqnrp: &mut c_int) -> c_int {
    *seqnrp = syn_st().current_seqnr;
        syn_st().current_flags
}

pub fn syn_get_sub_char() -> c_int {
    syn_st().current_sub_char
}

pub fn syn_get_stack_item(i: c_int) -> c_int {
    unsafe {
        if i >= cur_state_len() {
            invalidate_current_state();
            syn_st().current_col = MAXCOL;
            return -1;
        }
        (*cur_state_at(i)).si_id
    }
}

fn syn_cur_foldlevel() -> c_int {
    unsafe {
        let mut level: c_int = 0;
        for i in 0..cur_state_len() {
            if ((*cur_state_at(i)).si_flags & HL_FOLD) != 0 {
                level += 1;
            }
        }
        level
    }
}

pub fn syn_get_foldlevel(wp: &mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        let mut level: c_int = 0;

        if (*win_s(&mut *wp)).b_syn_folditems != 0
            && !(*win_s(&mut *wp)).b_syn_error
            && !(*win_s(&mut *wp)).b_syn_slow
        {
            syntax_start(&raw mut *wp, lnum);

            level = syn_cur_foldlevel();

            if (*win_s(&mut *wp)).b_syn_foldlevel == SYNFLD_MINIMUM {
                let mut cur_level = level;
                let mut low_level = cur_level;
                while !syn_st().current_finished {
                    syn_current_attr(false, false, ptr::null_mut(), false);
                    cur_level = syn_cur_foldlevel();
                    if cur_level < low_level {
                        low_level = cur_level;
                    } else if cur_level > low_level {
                        level = low_level;
                    }
                    syn_st().current_col += 1;
                }
            }
        }
        if (level as i64) > wp.w_onebuf_opt.wo_fdn {
            level = wp.w_onebuf_opt.wo_fdn as c_int;
            if level < 0 {
                level = 0;
            }
        }
        level
    }
}

/// ":syntime".
pub fn ex_syntime(eap: &mut exarg_T) {
     {
        if crate::memory::cstr_as_bytes(eap.arg) == b"on" {
            syn_st().syn_time_on = true;
        } else if crate::memory::cstr_as_bytes(eap.arg) == b"off" {
            syn_st().syn_time_on = false;
        } else if crate::memory::cstr_as_bytes(eap.arg) == b"clear" {
            syntime_clear();
        } else if crate::memory::cstr_as_bytes(eap.arg) == b"report" {
            syntime_report();
        } else {
            semsg!(gettext(e_invarg2), eap.arg);
        }
    }
}

fn syn_clear_time(st: &mut syn_time_T) {
    st.total = profile_zero();
        st.slowest = profile_zero();
        st.count = 0;
        st.match_ = 0;
}

fn syntime_clear() {
    unsafe {
        if !syntax_present(curwin_ptr()) {
            msg(gettext(msg_no_items), 0);
            return;
        }
        for pat in &mut (*win_s(curwin_mut())).b_syn_patterns {
            syn_clear_time(&mut pat.sp_time);
        }
    }
}

pub fn get_syntime_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    match idx {
        0 => c"on".as_ptr().cast_mut(),
        1 => c"off".as_ptr().cast_mut(),
        2 => c"clear".as_ptr().cast_mut(),
        3 => c"report".as_ptr().cast_mut(),
        _ => ptr::null_mut(),
    }
}

fn syn_compare_syntime(s1: &time_entry_T, s2: &time_entry_T) -> std::cmp::Ordering {
    profile_cmp(s1.total, s2.total).cmp(&0)
}

fn syntime_report() {
    unsafe {
        if !syntax_present(curwin_ptr()) {
            msg(gettext(msg_no_items), 0);
            return;
        }

        let mut ga: Vec<time_entry_T> = Vec::new();

        let mut total_total = profile_zero();
        let mut total_count: c_int = 0;
        for pat in &(*win_s(curwin_mut())).b_syn_patterns {
            if pat.sp_time.count > 0 {
                total_total = profile_add(total_total, pat.sp_time.total);
                total_count += pat.sp_time.count;
                ga.push(time_entry_T {
                    total: pat.sp_time.total,
                    count: pat.sp_time.count,
                    match_: pat.sp_time.match_,
                    slowest: pat.sp_time.slowest,
                    average: profile_divide(pat.sp_time.total, pat.sp_time.count),
                    id: pat.sp_syn.id as c_int,
                    pattern: pat.sp_pattern.clone(),
                });
            }
        }

        if ga.len() > 1 {
            ga.sort_unstable_by(syn_compare_syntime);
        }

        msg_puts_title(gettext(
            c"  TOTAL      COUNT  MATCH   SLOWEST     AVERAGE   NAME               PATTERN",
        ));
        msg_puts(c"\n");
        for p in &ga {
            if g_st().got_int {
                break;
            }

            msg_puts(profile_msg(p.total));
            msg_puts(c" ");
            msg_advance(13);
            msg_outnum(p.count);
            msg_puts(c" ");
            msg_advance(20);
            msg_outnum(p.match_);
            msg_puts(c" ");
            msg_advance(26);
            msg_puts(profile_msg(p.slowest));
            msg_puts(c" ");
            msg_advance(38);
            msg_puts(profile_msg(p.average));
            msg_puts(c" ");
            msg_advance(50);
            msg_outtrans(highlight_group_name(p.id - 1), 0, false);
            msg_puts(c" ");

            msg_advance(69);
            let mut len = if g_st().Columns < 80 { 20 } else { g_st().Columns - 70 };
            let pat_ptr = p.pattern.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr());
            let patlen = p.pattern.as_ref().map_or(0, |s| s.to_bytes().len() as c_int);
            if patlen < len {
                len = patlen;
            }
            msg_outtrans_len(pat_ptr, len, 0, false);
            msg_puts(c"\n");
        }
        if !g_st().got_int {
            msg_puts(c"\n");
            msg_puts(profile_msg(total_total));
            msg_advance(13);
            msg_outnum(total_count);
            msg_puts(c"\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_syntime_arg_and_limit_pos_work() {
         {
            assert_eq!(
                crate::memory::cstr_as_bytes(get_syntime_arg(&mut crate::os::env::expand_T::new(), 0)),
                b"on"
            );
            assert_eq!(
                crate::memory::cstr_as_bytes(get_syntime_arg(&mut crate::os::env::expand_T::new(), 3)),
                b"report"
            );
            assert!(get_syntime_arg(&mut crate::os::env::expand_T::new(), 4).is_null());

            let mut pos = lpos_T { lnum: 5, col: 10 };
            let mut limit = lpos_T { lnum: 5, col: 4 };
            limit_pos(&mut pos, &mut limit);
            assert_eq!(pos.col, 4);
        }
    }
}
