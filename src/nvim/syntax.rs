//! Code for syntax highlighting (`src/nvim/syntax.c`).

use crate::autocmd::{EVENT_SYNTAX, apply_autocmds};
use crate::buffer::{FileBuf, MAXLNUM, buf_get_changedtick};
use crate::charset::{
    ascii_isalpha, ascii_iswhite, check_isopt, getdigits_int, getdigits_int32, parse_isopt,
    str_foldcase, vim_isprintc, vim_iswordp_buf,
};
use crate::cmdexpand::{EXPAND_HIGHLIGHT, EXPAND_NOTHING, EXPAND_SYNTAX, HLF_D};
use crate::cursor::{Colnr, CursorPos, Linenr, Lpos};
use crate::drawscreen::{UPD_NOT_VALID, UPD_SOME_VALID, redraw_curbuf_later, redraw_later};
use crate::errors::*;
use crate::eval::vars::{do_unlet, get_var_value, set_internal_string_var};
use crate::ex_docmd::{
    EX_NOSPC, EX_XFILE, Exarg, check_nextcmd, do_cmdline_cmd, ends_excmd, expand_filename,
    find_nextcmd, separate_nextcmd,
};
use crate::fold::{fold_update_all, foldmethod_is_syntax};
use crate::globals::*;
use crate::highlight_group::{
    highlight_group_name, highlight_link_id, highlight_num_groups, init_highlight, syn_check_group,
    syn_id2attr, syn_list_header, syn_name2id, syn_name2id_len,
};
use crate::indent_c::find_start_comment;
use crate::mbyte::{mb_strcmp_ic, utf_ptr2char, utfc_ptr2len, utfc_slice2len};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::message::{
    emsg, msg, msg_advance, msg_ext_set_kind, msg_outnum, msg_outtrans, msg_outtrans_len,
    msg_putchar, msg_puts, msg_puts_hl, msg_puts_title, semsg,
};
use crate::option_vars::*;
use crate::optionstr::CStrOptionSlot;
use crate::os::env::ExpandCtx;
use crate::os::input::line_breakcheck;
use crate::path::path_is_absolute;
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::profile::{
    Proftime, profile_add, profile_cmp, profile_divide, profile_end, profile_msg, profile_start,
    profile_zero,
};
use crate::regexp::{
    MAXCOL, NSUBEXP, RE_MAGIC, REX_SET, REX_USE, RegExtmatch, Regmatch, Regmmatch, Regprog,
    skip_regexp, vim_regcomp, vim_regcomp_had_eol, vim_regexec, vim_regexec_multi,
};
use crate::runtime::{DIP_ALL, DOSO_NONE, do_source, source_runtime};
use crate::strings::{format_to_buf, gettext, vim_strchr};
use crate::window::WinInfoInfo;
use std::ffi::CStr;
use std::ptr;

pub use crate::option_vars::Synblock;

macro_rules! syn_getcurline {
    () => {
        ml_get_buf(syn_st().syn_buf_mut(), syn_st().current_lnum)
            .as_ptr()
            .cast_mut()
    };
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SynTime {
    pub total: u64,
    pub slowest: u64,
    pub count: i32,
    pub match_: i32,
}

impl SynTime {
    pub const fn new() -> Self {
        Self {
            total: 0,
            slowest: 0,
            count: 0,
            match_: 0,
        }
    }
}

use crate::ex_getln::Disptick;

pub const HL_CONTAINED: i32 = 0x01;
pub const HL_TRANSP: i32 = 0x02;
pub const HL_ONELINE: i32 = 0x04;
pub const HL_HAS_EOL: i32 = 0x08;
pub const HL_SYNC_HERE: i32 = 0x10;
pub const HL_SYNC_THERE: i32 = 0x20;
pub const HL_MATCH: i32 = 0x40;
pub const HL_SKIPNL: i32 = 0x80;
pub const HL_SKIPWHITE: i32 = 0x100;
pub const HL_SKIPEMPTY: i32 = 0x200;
pub const HL_KEEPEND: i32 = 0x400;
pub const HL_EXCLUDENL: i32 = 0x800;
pub const HL_DISPLAY: i32 = 0x1000;
pub const HL_FOLD: i32 = 0x2000;
pub const HL_EXTEND: i32 = 0x4000;
pub const HL_MATCHCONT: i32 = 0x8000;
pub const HL_TRANS_CONT: i32 = 0x10000;
pub const HL_CONCEAL: i32 = 0x20000;
pub const HL_CONCEALENDS: i32 = 0x40000;
pub const HL_INCLUDED_TOPLEVEL: i32 = 0x80000;

const SST_MIN_ENTRIES: i32 = 150;
const SST_MAX_ENTRIES: i32 = 1000;
const SST_DIST: i32 = 16;

const SPO_MS_OFF: usize = 0;
const SPO_ME_OFF: usize = 1;
const SPO_HS_OFF: usize = 2;
const SPO_HE_OFF: usize = 3;
const SPO_RS_OFF: usize = 4;
const SPO_RE_OFF: usize = 5;
const SPO_LC_OFF: usize = 6;
const SPO_COUNT: usize = 7;

const SPTYPE_MATCH: u8 = 1;
const SPTYPE_START: u8 = 2;
const SPTYPE_END: u8 = 3;
const SPTYPE_SKIP: u8 = 4;

const NONE_IDX: i32 = -2;
const KEYWORD_IDX: i32 = -1;

const SF_CCOMMENT: i32 = 0x01;
const SF_MATCH: i32 = 0x02;

const MAXKEYWLEN: usize = 80;

const CLUSTER_REPLACE: i32 = 1;
const CLUSTER_ADD: i32 = 2;
const CLUSTER_SUBTRACT: i32 = 3;

const MAX_HL_ID: i16 = 20000;
const SYNID_ALLBUT: i16 = MAX_HL_ID;
const SYNID_TOP: i16 = 21000;
const SYNID_CONTAINED: i16 = 22000;
const SYNID_CLUSTER: i16 = 23000;

const MAX_SYN_INC_TAG: i32 = 999;
const MAX_CLUSTER_ID: i32 = 32767 - SYNID_CLUSTER as i32;

const SYNSPL_DEFAULT: i32 = 0;
const SYNSPL_TOP: i32 = 1;
const SYNSPL_NOTOP: i32 = 2;

const SYNFLD_START: i32 = 0;
const SYNFLD_MINIMUM: i32 = 1;

const IDL_CACHE_SIZE: usize = 16;

const ITEM_START: usize = 0;
const ITEM_SKIP: usize = 1;
const ITEM_END: usize = 2;
const ITEM_MATCHGROUP: usize = 3;

const EXP_CASE: i32 = 1;
const EXP_SPELL: i32 = 2;
const EXP_SYNC: i32 = 3;
const EXP_CLUSTER: i32 = 4;

const SYNTAX_FNAME: &std::ffi::CStr = c"$VIMRUNTIME/syntax/%s.vim";
const E_ILLEGAL_ARG: &std::ffi::CStr = c"E390: Illegal argument: %s";
const E_CONTAINS_ARGUMENT_NOT_ACCEPTED_HERE: &std::ffi::CStr =
    c"E395: Contains argument not accepted here";
const E_INVALID_CCHAR_VALUE: &std::ffi::CStr = c"E844: Invalid cchar value";
const E_TRAILING_CHAR_AFTER_RSB_STR_STR: &std::ffi::CStr = c"E890: Trailing char after ']': %s]%s";
const MSG_NO_ITEMS: &std::ffi::CStr = c"No Syntax items defined for this buffer";

const SPO_NAME_TAB: [&std::ffi::CStr; SPO_COUNT] =
    [c"ms=", c"me=", c"hs=", c"he=", c"rs=", c"re=", c"lc="];

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
pub struct SpSyn {
    pub inc_tag: i32,
    pub id: i16,
    pub cont_in_list: SynIdList,
}

impl SpSyn {
    pub const fn new() -> Self {
        Self {
            inc_tag: 0,
            id: 0,
            cont_in_list: None,
        }
    }
}

impl Default for SpSyn {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct Keyentry {
    pub k_syn: SpSyn,
    pub next_list: SynIdList,
    pub flags: i32,
    pub k_char: i32,
    pub keyword: std::ffi::CString,
}

#[repr(C)]
pub struct Bufstate {
    pub bs_idx: i32,
    pub bs_flags: i32,
    pub bs_seqnr: i32,
    pub bs_cchar: i32,
    pub bs_extmatch: Option<std::rc::Rc<RegExtmatch>>,
}

#[repr(C)]
#[derive(Default)]
pub struct Synstate {
    pub sst_lnum: Linenr,
    pub sst_stack: Vec<Bufstate>,
    pub sst_next_flags: i32,
    pub sst_next_list: SynIdList,
    pub sst_tick: Disptick,
    pub sst_change_lnum: Linenr,
}

#[repr(C)]
pub struct Synpat {
    sp_type: u8,
    sp_syncing: bool,
    sp_syn_match_id: i16,
    sp_off_flags: i16,
    sp_offsets: [i32; SPO_COUNT],
    sp_flags: i32,
    sp_cchar: i32,
    sp_ic: bool,
    sp_sync_idx: i32,
    sp_line_id: i32,
    sp_startcol: i32,
    sp_cont_list: SynIdList,
    sp_next_list: SynIdList,
    sp_syn: SpSyn,
    sp_pattern: Option<std::ffi::CString>,
    sp_prog: Option<Box<Regprog>>,
    sp_time: SynTime,
}

impl Synpat {
    pub const fn new() -> Self {
        Self {
            sp_type: 0,
            sp_syncing: false,
            sp_syn_match_id: 0,
            sp_off_flags: 0,
            sp_offsets: [0; SPO_COUNT],
            sp_flags: 0,
            sp_cchar: 0,
            sp_ic: false,
            sp_sync_idx: 0,
            sp_line_id: 0,
            sp_startcol: 0,
            sp_cont_list: None,
            sp_next_list: None,
            sp_syn: SpSyn::new(),
            sp_pattern: None,
            sp_prog: None,
            sp_time: SynTime::new(),
        }
    }
}

impl Default for Synpat {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct SynCluster {
    scl_name: std::ffi::CString,
    scl_name_u: std::ffi::CString,
    scl_list: SynIdList,
}

#[repr(C)]
struct Stateitem {
    si_idx: i32,
    si_id: i32,
    si_trans_id: i32,
    si_m_lnum: i32,
    si_m_startcol: i32,
    si_m_endpos: Lpos,
    si_h_startpos: Lpos,
    si_h_endpos: Lpos,
    si_eoe_pos: Lpos,
    si_end_idx: i32,
    si_ends: i32,
    si_attr: i32,
    si_flags: i32,
    si_seqnr: i32,
    si_cchar: i32,
    si_cont_list: SynIdList,
    si_next_list: SynIdList,
    si_extmatch: Option<std::rc::Rc<RegExtmatch>>,
}

impl Stateitem {
    const fn new() -> Self {
        Self {
            si_idx: 0,
            si_id: 0,
            si_trans_id: 0,
            si_m_lnum: 0,
            si_m_startcol: 0,
            si_m_endpos: Lpos::new(),
            si_h_startpos: Lpos::new(),
            si_h_endpos: Lpos::new(),
            si_eoe_pos: Lpos::new(),
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

impl Default for Stateitem {
    fn default() -> Self {
        Self::new()
    }
}

struct SynOptArg {
    flags: i32,
    keyword: bool,
    sync_idx: Option<i32>,
    has_cont_list: bool,
    cont_list: SynIdList,
    cont_in_list: SynIdList,
    next_list: SynIdList,
}

#[derive(Default)]
pub struct IdlEntry {
    idl_key: SynIdList,
    idl_ids: Vec<i16>,
    idl_marker: i16,
    idl_usable: bool,
}

#[derive(Default)]
pub struct IdlCache {
    ent: [IdlEntry; IDL_CACHE_SIZE],
    round: i32,
    last: i32,
}

#[repr(C)]
#[derive(Clone)]
struct TimeEntry {
    total: Proftime,
    count: i32,
    match_: i32,
    slowest: Proftime,
    average: Proftime,
    id: i32,
    pattern: Option<std::ffi::CString>,
}

struct PatPtr {
    pp_synp: Synpat,
    pp_matchgroup_id: i32,
}

struct SyntaxState {
    did_syntax_onoff: bool,
    changedtick_last: i64,
    try_next_column: bool,
    in_id_list_depth: i32,
    current_attr: i32,
    current_id: i32,
    current_trans_id: i32,
    current_flags: i32,
    current_seqnr: i32,
    current_sub_char: i32,
    current_syn_inc_tag: i32,
    running_syn_inc_tag: i32,
    keepend_level: i32,
    next_seqnr: i32,
    next_match_col: i32,
    next_match_m_endpos: Lpos,
    next_match_h_startpos: Lpos,
    next_match_h_endpos: Lpos,
    next_match_idx: i32,
    next_match_flags: i32,
    next_match_eos_pos: Lpos,
    next_match_eoe_pos: Lpos,
    next_match_end_idx: i32,
    next_match_extmatch: Option<std::rc::Rc<RegExtmatch>>,
    syn_win: crate::globals::WinPtr,
    syn_buf: crate::globals::BufPtr,
    syn_block: crate::globals::GraphPtr<Synblock>,
    syn_tm: Option<Proftime>,
    current_lnum: Linenr,
    current_col: Colnr,
    current_state_stored: bool,
    current_finished: bool,
    current_state_valid: bool,
    current_state: Vec<Stateitem>,
    current_next_list: SynIdList,
    current_next_flags: i32,
    current_line_id: i32,
    syn_time_on: bool,
    last_matchgroup: i32,
    expand_what: i32,
}

impl SyntaxState {
    #[inline]
    fn syn_buf_mut(&self) -> &'static mut FileBuf {
        self.syn_buf.borrow_mut()
    }
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
            next_match_m_endpos: Lpos { lnum: 0, col: 0 },
            next_match_h_startpos: Lpos { lnum: 0, col: 0 },
            next_match_h_endpos: Lpos { lnum: 0, col: 0 },
            next_match_idx: 0,
            next_match_flags: 0,
            next_match_eos_pos: Lpos { lnum: 0, col: 0 },
            next_match_eoe_pos: Lpos { lnum: 0, col: 0 },
            next_match_end_idx: 0,
            next_match_extmatch: None,
            syn_win: crate::globals::WinPtr::NULL,
            syn_buf: crate::globals::BufPtr::NULL,
            syn_block: crate::globals::GraphPtr::NULL,
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
fn syn_st() -> crate::globals::GraphPtr<SyntaxState> {
    SYN_STATE.handle()
}

#[inline]
pub fn win_s(wp: &mut WinInfoInfo) -> &'static mut Synblock {
    &mut *wp.w_s_ptr()
}

#[inline]
fn syn_block_mut() -> &'static mut Synblock {
    syn_st().syn_block.borrow_mut()
}

#[inline]
fn syn_items(block: &mut Synblock, idx: isize) -> &'static mut Synpat {
    let ptr = crate::globals::GraphPtr::<Synpat>::from_ref(&block.b_syn_patterns[idx as usize]);
    ptr.borrow_mut()
}

#[inline]
fn syn_clstr(block: &mut Synblock, idx: isize) -> &'static mut SynCluster {
    let ptr = crate::globals::GraphPtr::<SynCluster>::from_ref(&block.b_syn_clusters[idx as usize]);
    ptr.borrow_mut()
}

#[inline]
fn cur_state_len() -> i32 {
    syn_st().current_state.len() as i32
}

#[inline]
fn cur_state_empty() -> bool {
    syn_st().current_state.is_empty()
}

#[inline]
fn cur_state_at(idx: i32) -> &'static mut Stateitem {
    &mut syn_st().borrow_mut().current_state[idx as usize]
}

#[inline]
fn ke2hikey(kp: &Keyentry) -> &std::ffi::CStr {
    kp.keyword.as_c_str()
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
pub fn syn_set_timeout(tm: Option<Proftime>) {
    syn_st().syn_tm = tm;
}

/// Start the syntax recognition for a line.
pub fn syntax_start(wp: &mut WinInfoInfo, lnum: Linenr) {
    let mut last_valid: Option<usize> = None;
    let mut last_min_valid: Option<usize> = None;
    let mut prev: Option<usize> = None;

    syn_st().current_sub_char = NUL as i32;

    if syn_st().syn_block != win_s(wp)
        || syn_st().syn_buf != wp.w_buffer
        || syn_st().changedtick_last != buf_get_changedtick(syn_st().syn_buf_mut())
    {
        invalidate_current_state();
        syn_st().syn_buf = wp.w_buffer;
        syn_st().syn_block = crate::globals::GraphPtr::new(win_s(wp));
    }
    syn_st().changedtick_last = buf_get_changedtick(syn_st().syn_buf_mut());
    syn_st().syn_win = crate::globals::WinPtr::from(&mut *wp);

    syn_stack_alloc();
    if syn_block_mut().b_sst_max == 0 {
        return;
    }
    syn_block_mut().b_sst_lasttick = display_tick_val();

    if valid_state()
        && syn_st().current_lnum < lnum
        && syn_st().current_lnum < syn_st().syn_buf_mut().b_ml.ml_line_count
    {
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

    if invalid_state() && !syn_block_mut().b_sst_array.is_empty() {
        for (idx, p) in syn_block_mut().b_sst_array.as_slice().iter().enumerate() {
            if p.sst_lnum > lnum {
                break;
            }
            if p.sst_change_lnum == 0 {
                last_valid = Some(idx);
                if p.sst_lnum >= lnum - syn_block_mut().b_syn_sync_minlines {
                    last_min_valid = Some(idx);
                }
            }
        }
        if let Some(idx) = last_min_valid {
            load_current_state(&syn_block_mut().b_sst_array.as_slice()[idx]);
        }
    }

    let first_stored = if invalid_state() {
        syn_sync(
            wp,
            lnum,
            last_valid.map(|idx| &syn_block_mut().b_sst_array.as_slice()[idx]),
        );
        if syn_st().current_lnum == 1 {
            1
        } else {
            syn_st().current_lnum + syn_block_mut().b_syn_sync_minlines
        }
    } else {
        syn_st().current_lnum
    };

    let sst_len = syn_block_mut().b_sst_max;
    let dist = if sst_len <= g_st().rows_global {
        999999
    } else {
        syn_st().syn_buf_mut().b_ml.ml_line_count / (sst_len - g_st().rows_global) + 1
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
            while sp_idx < syn_block_mut().b_sst_array.len()
                && syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_lnum < syn_st().current_lnum
            {
                sp_idx += 1;
            }
            if sp_idx < syn_block_mut().b_sst_array.len()
                && syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_lnum == syn_st().current_lnum
                && syn_stack_equal(&syn_block_mut().b_sst_array.as_slice()[sp_idx])
            {
                let parsed_lnum = syn_st().current_lnum;
                prev = Some(sp_idx);
                while sp_idx < syn_block_mut().b_sst_array.len()
                    && syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_change_lnum <= parsed_lnum
                {
                    if syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_lnum <= lnum {
                        prev = Some(sp_idx);
                    } else if syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_change_lnum == 0 {
                        break;
                    }
                    syn_block_mut().b_sst_array.as_mut_slice()[sp_idx].sst_change_lnum = 0;
                    sp_idx += 1;
                }
                if let Some(pi) = prev {
                    load_current_state(&syn_block_mut().b_sst_array.as_slice()[pi]);
                }
            } else if prev.is_none_or(|pi| {
                syn_st().current_lnum == lnum
                    || syn_st().current_lnum
                        >= syn_block_mut().b_sst_array.as_slice()[pi].sst_lnum + dist
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

fn clear_current_state() {
    syn_st().current_state = Vec::new();
}

fn syn_sync(wp: &mut WinInfoInfo, mut start_lnum: Linenr, last_valid: Option<&Synstate>) {
    let mut found_flags: i32 = 0;
    let mut found_match_idx: i32 = 0;
    let mut found_current_lnum: Linenr = 0;
    let mut found_current_col: Colnr = 0;
    let mut found_m_endpos = Lpos { lnum: 0, col: 0 };

    invalidate_current_state();

    if syn_block_mut().b_syn_sync_minlines > start_lnum {
        start_lnum = 1;
    } else {
        let mut lnum = if syn_block_mut().b_syn_sync_minlines == 1 {
            1
        } else if syn_block_mut().b_syn_sync_minlines < 10 {
            syn_block_mut().b_syn_sync_minlines * 2
        } else {
            syn_block_mut().b_syn_sync_minlines * 3 / 2
        };
        if syn_block_mut().b_syn_sync_maxlines != 0 && lnum > syn_block_mut().b_syn_sync_maxlines {
            lnum = syn_block_mut().b_syn_sync_maxlines;
        }
        if lnum >= start_lnum {
            start_lnum = 1;
        } else {
            start_lnum -= lnum;
        }
    }
    syn_st().current_lnum = start_lnum;

    if (syn_block_mut().b_syn_sync_flags & SF_CCOMMENT) != 0 {
        let curwin_save = curwin_ptr();
        set_curwin(&raw mut *wp);
        let curbuf_save = g_st().curbuf;
        g_st().curbuf = (syn_st().syn_buf.as_ptr()).into();

        while start_lnum > 1 {
            if crate::memline::ml_get_slice(start_lnum - 1).last() != Some(&b'\\') {
                break;
            }
            start_lnum -= 1;
        }
        syn_st().current_lnum = start_lnum;

        let cursor_save: CursorPos = wp.w_cursor;
        wp.w_cursor.lnum = start_lnum;
        wp.w_cursor.col = 0;

        if find_start_comment(syn_block_mut().b_syn_sync_maxlines).is_some() {
            let mut idx = syn_block_mut().b_syn_patterns.len() as i32 - 1;
            while idx >= 0 {
                let spp = syn_items(syn_block_mut(), idx as isize);
                if spp.sp_syn.id == syn_block_mut().b_syn_sync_id && spp.sp_type == SPTYPE_START {
                    validate_current_state();
                    push_current_state(idx);
                    update_si_attr(cur_state_len() - 1);
                    break;
                }
                idx -= 1;
            }
        }

        wp.w_cursor = cursor_save;
        set_curwin(curwin_save);
        g_st().curbuf = curbuf_save;
    } else if (syn_block_mut().b_syn_sync_flags & SF_MATCH) != 0 {
        let break_lnum = if syn_block_mut().b_syn_sync_maxlines != 0
            && start_lnum > syn_block_mut().b_syn_sync_maxlines
        {
            start_lnum - syn_block_mut().b_syn_sync_maxlines
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

            if let Some(lv) = last_valid
                && lnum == lv.sst_lnum
            {
                load_current_state(lv);
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
                        if cur_si.si_m_endpos.lnum > start_lnum {
                            syn_st().current_lnum = end_lnum;
                            break;
                        }
                        if cur_si.si_idx < 0 {
                            found_flags = 0;
                            found_match_idx = KEYWORD_IDX;
                        } else {
                            let spp = syn_items(syn_block_mut(), cur_si.si_idx as isize);
                            found_flags = spp.sp_flags;
                            found_match_idx = spp.sp_sync_idx;
                        }
                        found_current_lnum = syn_st().current_lnum;
                        found_current_col = syn_st().current_col;
                        found_m_endpos = cur_si.si_m_endpos;
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
                        if *crate::globals::GraphPtr::of(
                            syn_getcurline!().wrapping_offset(syn_st().current_col as isize),
                        )
                        .borrow()
                            != 0
                        {
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
                        cur_si.si_h_startpos.lnum = found_current_lnum;
                        cur_si.si_h_startpos.col = found_current_col;
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

fn save_chartab(chartab: &mut [u64; 4]) {
    if syn_block_mut().b_syn_isk.is_none() {
        return;
    }

    *chartab = syn_st().syn_buf_mut().b_chartab;
    syn_st().syn_buf_mut().b_chartab = win_s(syn_st().syn_win.borrow_mut()).b_syn_chartab;
}

fn restore_chartab(chartab: &[u64; 4]) {
    if win_s(syn_st().syn_win.borrow_mut()).b_syn_isk.is_some() {
        syn_st().syn_buf_mut().b_chartab = *chartab;
    }
}

fn syn_match_linecont(lnum: Linenr) -> i32 {
    if syn_block_mut().b_syn_linecont_prog.is_none() {
        return 0;
    }

    let mut regmatch: Regmmatch = Regmmatch::new();
    let mut buf_chartab = [0u64; 4];
    save_chartab(&mut buf_chartab);

    regmatch.rmm_ic = syn_block_mut().b_syn_linecont_ic;
    regmatch.borrow_prog(syn_block_mut().b_syn_linecont_prog.as_deref());
    let r = syn_regexec(
        &mut regmatch,
        lnum,
        0,
        &mut syn_block_mut().b_syn_linecont_time,
    );

    restore_chartab(&buf_chartab);
    i32::from(r)
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
    let len = cur_state_len();
    if startofline {
        for i in 0..len {
            let cur_si = cur_state_at(i);
            if cur_si.si_idx >= 0
                && syn_items(syn_block_mut(), cur_si.si_idx as isize).sp_type == SPTYPE_MATCH
                && cur_si.si_m_endpos.lnum < syn_st().current_lnum
            {
                cur_si.si_flags |= HL_MATCHCONT;
                cur_si.si_m_endpos.lnum = 0;
                cur_si.si_m_endpos.col = 0;
                cur_si.si_h_endpos = cur_si.si_m_endpos;
                cur_si.si_ends = 1;
            }
        }
    }

    let mut i = len - 1;
    if syn_st().keepend_level >= 0 {
        while i > syn_st().keepend_level {
            if (cur_state_at(i).si_flags & HL_EXTEND) != 0 {
                break;
            }
            i -= 1;
        }
    }

    let mut seen_keepend = false;
    while i < len {
        let cur_si = cur_state_at(i);
        if (cur_si.si_flags & HL_KEEPEND) != 0
            || (seen_keepend && !startofline)
            || (i == len - 1 && startofline)
        {
            cur_si.si_h_startpos.col = 0;
            cur_si.si_h_startpos.lnum = syn_st().current_lnum;

            if (cur_si.si_flags & HL_MATCHCONT) == 0 {
                update_si_end(&mut *cur_si, syn_st().current_col, !startofline);
            }

            if !startofline && (cur_si.si_flags & HL_KEEPEND) != 0 {
                seen_keepend = true;
            }
        }
        i += 1;
    }
    check_keepend();
}

fn syn_stack_free_block(block: &mut Synblock) {
    idl_cache_clear(block);
    (block).b_sst_array = Vec::new();
    block.b_sst_max = 0;
    block.b_sst_search = 0;
}

/// Free b_sst_array[] for buffer "buf".
pub fn syn_stack_free_all(block: &mut Synblock) {
    syn_stack_free_block(block);

    if !g_st().curtab.is_null() {
        for wp in for_all_windows_mut() {
            if (!wp.w_buffer.is_null() || wp.w_s_owned.is_some())
                && ptr::eq(win_s(wp), block)
                && foldmethod_is_syntax(wp)
            {
                fold_update_all(wp);
            }
        }
    }
}

fn syn_stack_alloc() {
    let mut len = (syn_st().syn_buf_mut().b_ml.ml_line_count / SST_DIST + g_st().rows_global * 2)
        .clamp(SST_MIN_ENTRIES, SST_MAX_ENTRIES);
    let cur_len = syn_block_mut().b_sst_max;
    if cur_len > len * 2 || cur_len < len {
        len = syn_st().syn_buf_mut().b_ml.ml_line_count;
        len = ((len + len / 2) / SST_DIST + g_st().rows_global * 2)
            .clamp(SST_MIN_ENTRIES, SST_MAX_ENTRIES);

        while (syn_block_mut().b_sst_array.len() as i32) + 2 > len && syn_stack_cleanup() {}
        let needed = (syn_block_mut().b_sst_array.len() as i32) + 2;
        if len < needed {
            len = needed;
        }

        syn_block_mut().b_sst_max = len;
        syn_block_mut().b_sst_search = 0;
    }
}

/// Check for changes in a buffer to affect stored syntax states.
pub fn syn_stack_apply_changes(buf: &mut FileBuf) {
    let buf_ptr = &raw mut *buf;
    syn_stack_apply_changes_block(
        &mut buf.b_s,
        crate::globals::GraphPtr::of(buf_ptr).borrow_mut(),
    );

    if !g_st().curtab.is_null() {
        for wp in for_all_windows_mut() {
            if wp.w_buffer == buf && !ptr::eq(win_s(wp), &raw mut buf.b_s) {
                syn_stack_apply_changes_block(&mut *win_s(wp), buf);
            }
        }
    }
}

fn syn_stack_apply_changes_block(block: &mut Synblock, buf: &mut FileBuf) {
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
    if syn_block_mut().b_sst_array.len() < 2 {
        return false;
    }

    let sst_len = syn_block_mut().b_sst_max;
    let dist = if sst_len <= g_st().rows_global {
        999999
    } else {
        syn_st().syn_buf_mut().b_ml.ml_line_count / (sst_len - g_st().rows_global) + 1
    };

    let mut tick = syn_block_mut().b_sst_lasttick;
    let mut above = false;
    for w in syn_block_mut().b_sst_array.as_slice().windows(2) {
        let prev = &w[0];
        let p = &w[1];
        if prev.sst_lnum + dist > p.sst_lnum {
            if p.sst_tick > syn_block_mut().b_sst_lasttick {
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
    while idx < syn_block_mut().b_sst_array.len() {
        let prev_lnum = syn_block_mut().b_sst_array.as_slice()[idx - 1].sst_lnum;
        let p = &syn_block_mut().b_sst_array.as_slice()[idx];
        if p.sst_tick == tick && prev_lnum + dist > p.sst_lnum {
            if syn_block_mut().b_sst_search == idx {
                syn_block_mut().b_sst_search = 0;
            } else if syn_block_mut().b_sst_search > idx {
                syn_block_mut().b_sst_search -= 1;
            }
            syn_block_mut().b_sst_array.remove(idx);
            retval = true;
        } else {
            idx += 1;
        }
    }
    retval
}

fn syn_stack_find_entry(lnum: Linenr) -> Option<usize> {
    let arr = syn_block_mut().b_sst_array.as_slice();
    if arr.is_empty() {
        return None;
    }

    let mut idx = 0;
    let search = syn_block_mut().b_sst_search;
    if search < arr.len() && arr[search].sst_lnum <= lnum {
        idx = search;
    }

    let mut prev: Option<usize> = None;
    while idx < arr.len() {
        if arr[idx].sst_lnum == lnum {
            syn_block_mut().b_sst_search = idx;
            return Some(idx);
        }
        if arr[idx].sst_lnum > lnum {
            break;
        }
        prev = Some(idx);
        idx += 1;
    }
    if let Some(p) = prev {
        syn_block_mut().b_sst_search = p;
    }
    prev
}

fn store_current_state() -> Option<usize> {
    let mut sp_idx = syn_stack_find_entry(syn_st().current_lnum);

    let mut i = cur_state_len() - 1;
    while i >= 0 {
        let cur_si = cur_state_at(i);
        if cur_si.si_h_startpos.lnum >= syn_st().current_lnum
            || cur_si.si_m_endpos.lnum >= syn_st().current_lnum
            || cur_si.si_h_endpos.lnum >= syn_st().current_lnum
            || (cur_si.si_end_idx != 0 && cur_si.si_eoe_pos.lnum >= syn_st().current_lnum)
        {
            break;
        }
        i -= 1;
    }
    if i >= 0 {
        if let Some(idx) = sp_idx {
            if syn_block_mut().b_sst_search == idx {
                syn_block_mut().b_sst_search = 0;
            } else if syn_block_mut().b_sst_search > idx {
                syn_block_mut().b_sst_search -= 1;
            }
            syn_block_mut().b_sst_array.remove(idx);
            sp_idx = None;
        }
    } else if sp_idx.is_none_or(|idx| {
        syn_block_mut().b_sst_array.as_slice()[idx].sst_lnum != syn_st().current_lnum
    }) {
        if (syn_block_mut().b_sst_array.len() as i32) >= syn_block_mut().b_sst_max {
            syn_stack_cleanup();
            sp_idx = syn_stack_find_entry(syn_st().current_lnum);
        }
        if (syn_block_mut().b_sst_array.len() as i32) >= syn_block_mut().b_sst_max {
            sp_idx = None;
        } else {
            let insert_idx = sp_idx.map_or(0, |idx| idx + 1);
            syn_block_mut().b_sst_array.insert(
                insert_idx,
                Synstate {
                    sst_lnum: syn_st().current_lnum,
                    ..Synstate::default()
                },
            );
            syn_block_mut().b_sst_search = insert_idx;
            sp_idx = Some(insert_idx);
        }
    }
    if let Some(idx) = sp_idx {
        let sp = &mut syn_block_mut().b_sst_array.as_mut_slice()[idx];
        sp.sst_stack.clear();
        let len = cur_state_len();
        sp.sst_stack.reserve(len as usize);
        for j in 0..len {
            let cur_si = cur_state_at(j);
            sp.sst_stack.push(Bufstate {
                bs_idx: cur_si.si_idx,
                bs_flags: cur_si.si_flags,
                bs_seqnr: cur_si.si_seqnr,
                bs_cchar: cur_si.si_cchar,
                bs_extmatch: cur_si.si_extmatch.clone(),
            });
        }
        sp.sst_next_flags = syn_st().current_next_flags;
        sp.sst_next_list = syn_st().current_next_list.clone();
        sp.sst_tick = display_tick_val();
        sp.sst_change_lnum = 0;
    }
    syn_st().current_state_stored = true;
    sp_idx
}

fn load_current_state(from: &Synstate) {
    clear_current_state();
    validate_current_state();
    syn_st().keepend_level = -1;
    let stacksize = from.sst_stack.len() as i32;
    if stacksize != 0 {
        syn_st().current_state.reserve(stacksize as usize);
        for (i, b) in from.sst_stack.iter().enumerate() {
            let i_c = i as i32;
            let mut item = Stateitem::new();
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
                item.si_next_list = syn_items(syn_block_mut(), item.si_idx as isize)
                    .sp_next_list
                    .clone();
            } else {
                item.si_next_list = None;
            }
            syn_st().current_state.push(item);
            update_si_attr(i_c);
        }
    }
    syn_st().current_next_list = from.sst_next_list.clone();
    syn_st().current_next_flags = from.sst_next_flags;
    syn_st().current_lnum = from.sst_lnum;
}

fn syn_stack_equal(sp: &Synstate) -> bool {
    if sp.sst_stack.len() as i32 != cur_state_len()
        || !syn_id_list_eq(&sp.sst_next_list, &syn_st().current_next_list)
    {
        return false;
    }

    let mut i = cur_state_len() - 1;
    while i >= 0 {
        let cur_si = cur_state_at(i);
        let b = &sp.sst_stack[i as usize];
        if b.bs_idx != cur_si.si_idx {
            break;
        }
        if match (&b.bs_extmatch, &cur_si.si_extmatch) {
            (None, None) => true,
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            _ => false,
        } {
            i -= 1;
            continue;
        }
        let (Some(bsx), Some(six)) = (&b.bs_extmatch, &cur_si.si_extmatch) else {
            break;
        };
        let mut j: usize = 0;
        while j < NSUBEXP {
            if bsx.matches[j] != six.matches[j] {
                let (Some(bs_m), Some(si_m)) = (&bsx.matches[j], &six.matches[j]) else {
                    break;
                };
                if mb_strcmp_ic(
                    syn_items(syn_block_mut(), cur_si.si_idx as isize).sp_ic,
                    bs_m.as_c_str(),
                    si_m.as_c_str(),
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

/// We stop parsing syntax above line "lnum".
pub fn syntax_end_parsing(wp: &mut WinInfoInfo, lnum: Linenr) {
    if syn_st().syn_block != win_s(wp) {
        return;
    }
    if let Some(mut sp_idx) = syn_stack_find_entry(lnum) {
        if syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_lnum < lnum {
            sp_idx += 1;
        }

        if sp_idx < syn_block_mut().b_sst_array.len()
            && syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_change_lnum != 0
        {
            syn_block_mut().b_sst_array.as_mut_slice()[sp_idx].sst_change_lnum = lnum;
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
pub fn syntax_check_changed(lnum: Linenr) -> bool {
    let mut retval = true;

    if valid_state()
        && lnum == syn_st().current_lnum + 1
        && let Some(sp_idx) = syn_stack_find_entry(lnum)
        && syn_block_mut().b_sst_array.as_slice()[sp_idx].sst_lnum == lnum
    {
        syn_finish_line(false);

        if syn_stack_equal(&syn_block_mut().b_sst_array.as_slice()[sp_idx]) {
            retval = false;
        }

        syn_st().current_lnum += 1;
        store_current_state();
    }

    retval
}

fn syn_finish_line(syncing: bool) -> bool {
    while !syn_st().current_finished {
        syn_current_attr(syncing, false, None, false);

        if syncing && !cur_state_empty() {
            let cur_si = cur_state_at(cur_state_len() - 1);
            if cur_si.si_idx >= 0
                && (syn_items(syn_block_mut(), cur_si.si_idx as isize).sp_flags
                    & (HL_SYNC_HERE | HL_SYNC_THERE))
                    != 0
            {
                return true;
            }

            let prev_current_col = syn_st().current_col;
            if *crate::globals::GraphPtr::of(
                syn_getcurline!().wrapping_offset(syn_st().current_col as isize),
            )
            .borrow()
                != 0
            {
                syn_st().current_col += 1;
            }
            check_state_ends();
            syn_st().current_col = prev_current_col;
        }
        syn_st().current_col += 1;
    }
    false
}

/// Gets highlight attributes for next character.
pub fn get_syntax_attr(col: Colnr, mut can_spell: Option<&mut bool>, keep_state: bool) -> i32 {
    let mut attr: i32 = 0;

    if let Some(ref mut cs) = can_spell {
        **cs = if syn_block_mut().b_syn_spell == SYNSPL_DEFAULT {
            syn_block_mut().b_spell_cluster_id == 0
        } else {
            syn_block_mut().b_syn_spell == SYNSPL_TOP
        };
    }

    if syn_block_mut().b_sst_max == 0 {
        return 0;
    }

    if syn_st().syn_buf_mut().b_p_smc > 0 && col >= syn_st().syn_buf_mut().b_p_smc as Colnr {
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
            can_spell.as_deref_mut(),
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

fn syn_current_attr(
    syncing: bool,
    displaying: bool,
    can_spell: Option<&mut bool>,
    keep_state: bool,
) -> i32 {
    let mut endpos = Lpos { lnum: 0, col: 0 };
    let mut hl_startpos = Lpos { lnum: 0, col: 0 };
    let mut hl_endpos = Lpos { lnum: 0, col: 0 };
    let mut eos_pos = Lpos { lnum: 0, col: 0 };
    let mut eoe_pos = Lpos { lnum: 0, col: 0 };
    let mut end_idx: i32;
    let mut cur_si: crate::globals::GraphPtr<Stateitem>;
    let mut sip = crate::globals::GraphPtr::<Stateitem>::NULL;
    let mut startcol: i32;
    let mut endcol: i32 = 0;
    let mut flags: i32 = 0;
    let mut cchar: i32 = 0;
    let mut next_list: SynIdList = None;
    let mut found_match: bool;
    let mut regmatch: Regmmatch = Regmmatch::new();
    let mut pos = Lpos { lnum: 0, col: 0 };
    let mut cur_extmatch: Option<std::rc::Rc<RegExtmatch>> = None;
    let mut buf_chartab = [0u64; 4];

    let mut keep_next_list: bool;
    let mut zero_width_next_list = false;
    let mut zero_width_next_ga: Vec<i32> = Vec::new();

    let mut line = syn_getcurline!();
    if *crate::globals::GraphPtr::of(line.wrapping_offset(syn_st().current_col as isize)).borrow()
        == 0
        && syn_st().current_col != 0
    {
        if syn_st().next_match_idx >= 0
            && syn_st().next_match_col >= syn_st().current_col
            && syn_st().next_match_col != MAXCOL
        {
            push_next_match();
        }

        syn_st().current_finished = true;
        syn_st().current_state_stored = false;
        return 0;
    }

    if *crate::globals::GraphPtr::of(line.wrapping_offset(syn_st().current_col as isize)).borrow()
        == 0
        || *crate::globals::GraphPtr::of(line.wrapping_offset((syn_st().current_col + 1) as isize))
            .borrow()
            == 0
    {
        syn_st().current_finished = true;
        syn_st().current_state_stored = false;
    }

    if syn_st().try_next_column {
        syn_st().next_match_idx = -1;
        syn_st().try_next_column = false;
    }

    let do_keywords = !syncing
        && (!syn_block_mut().b_keywtab.is_empty() || !syn_block_mut().b_keywtab_ic.is_empty());

    save_chartab(&mut buf_chartab);

    loop {
        found_match = false;
        keep_next_list = false;
        let mut syn_id: i32 = 0;

        if !cur_state_empty() {
            cur_si = crate::globals::GraphPtr::<Stateitem>::of(cur_state_at(cur_state_len() - 1));
        } else {
            cur_si = crate::globals::GraphPtr::NULL;
        }

        if syn_block_mut().b_syn_containedin != 0
            || cur_si.is_null()
            || cur_si.si_cont_list.is_some()
        {
            if do_keywords {
                line = syn_getcurline!();
                let line_bytes = crate::strings::ByteSlice::from_ptr(line).as_bytes();
                let cur_col = (syn_st().current_col as usize).min(line_bytes.len());
                if vim_iswordp_buf(&line_bytes[cur_col..], syn_st().syn_buf_mut())
                    && (cur_col == 0
                        || !vim_iswordp_buf(
                            &line_bytes[cur_col
                                - 1
                                - (crate::mbyte::utf_head_off_slice(line_bytes, cur_col - 1)
                                    as usize)..],
                            syn_st().syn_buf_mut(),
                        ))
                {
                    syn_id = check_keyword_id(
                        line_bytes,
                        syn_st().current_col,
                        &mut endcol,
                        &mut flags,
                        &mut next_list,
                        cur_si.opt(),
                        &mut cchar,
                    );
                    if syn_id != 0 {
                        push_current_state(KEYWORD_IDX);
                        let len = cur_state_len();
                        cur_si = crate::globals::GraphPtr::<Stateitem>::of(cur_state_at(len - 1));
                        let cur_si = cur_si.borrow_mut();
                        cur_si.si_m_startcol = syn_st().current_col;
                        cur_si.si_h_startpos.lnum = syn_st().current_lnum;
                        cur_si.si_h_startpos.col = 0;
                        cur_si.si_m_endpos.lnum = syn_st().current_lnum;
                        cur_si.si_m_endpos.col = endcol;
                        cur_si.si_h_endpos.lnum = syn_st().current_lnum;
                        cur_si.si_h_endpos.col = endcol;
                        cur_si.si_ends = 1;
                        cur_si.si_end_idx = 0;
                        cur_si.si_flags = flags;
                        cur_si.si_seqnr = syn_st().next_seqnr;
                        syn_st().next_seqnr += 1;
                        cur_si.si_cchar = cchar;
                        if len > 1 {
                            cur_si.si_flags |= cur_state_at(len - 2).si_flags & HL_CONCEAL;
                        }
                        cur_si.si_id = syn_id;
                        cur_si.si_trans_id = syn_id;
                        if (flags & HL_TRANSP) != 0 {
                            if len < 2 {
                                cur_si.si_attr = 0;
                                cur_si.si_trans_id = 0;
                            } else {
                                cur_si.si_attr = cur_state_at(len - 2).si_attr;
                                cur_si.si_trans_id = cur_state_at(len - 2).si_trans_id;
                            }
                        } else {
                            cur_si.si_attr = syn_id2attr(syn_id);
                        }
                        cur_si.si_cont_list = None;
                        cur_si.si_next_list = next_list.take();
                        check_keepend();
                    }
                }
            }

            if syn_id == 0 && !syn_block_mut().b_syn_patterns.is_empty() {
                if syn_st().next_match_idx < 0 || syn_st().next_match_col < syn_st().current_col {
                    syn_st().next_match_idx = 0;
                    syn_st().next_match_col = MAXCOL;
                    let mut idx = syn_block_mut().b_syn_patterns.len() as i32 - 1;
                    while idx >= 0 {
                        let spp = syn_items(syn_block_mut(), idx as isize);
                        let cur_next_list = &syn_st().current_next_list;
                        if spp.sp_syncing == syncing
                            && (displaying || (spp.sp_flags & HL_DISPLAY) == 0)
                            && (spp.sp_type == SPTYPE_MATCH || spp.sp_type == SPTYPE_START)
                            && if cur_next_list.is_some() {
                                in_id_list(None, cur_next_list, &mut spp.sp_syn, 0) != 0
                            } else if cur_si.is_null() {
                                (spp.sp_flags & HL_CONTAINED) == 0
                            } else {
                                in_id_list(
                                    cur_si.opt(),
                                    &(cur_si.borrow().si_cont_list),
                                    &mut spp.sp_syn,
                                    spp.sp_flags,
                                ) != 0
                            }
                        {
                            if spp.sp_line_id == syn_st().current_line_id
                                && spp.sp_startcol >= syn_st().next_match_col
                            {
                                idx -= 1;
                                continue;
                            }
                            spp.sp_line_id = syn_st().current_line_id;

                            let mut lc_col = syn_st().current_col - spp.sp_offsets[SPO_LC_OFF];
                            if lc_col < 0 {
                                lc_col = 0;
                            }

                            regmatch.rmm_ic = spp.sp_ic;
                            regmatch.borrow_prog(spp.sp_prog.as_deref());
                            let r = syn_regexec(
                                &mut regmatch,
                                syn_st().current_lnum,
                                lc_col,
                                &mut spp.sp_time,
                            );
                            if !r {
                                spp.sp_startcol = MAXCOL;
                                idx -= 1;
                                continue;
                            }

                            syn_add_start_off(&mut pos, &mut regmatch, &mut *spp, SPO_MS_OFF, -1);
                            if pos.lnum > syn_st().current_lnum {
                                spp.sp_startcol = MAXCOL;
                                idx -= 1;
                                continue;
                            }
                            startcol = pos.col;
                            spp.sp_startcol = startcol;

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
                            syn_add_end_off(&mut eos_pos, &mut regmatch, &mut *spp, SPO_RS_OFF, 0);

                            cur_extmatch =
                                crate::globals::GraphPtr::of(&raw mut g_st().re_extmatch_out)
                                    .replace_val(None);

                            flags = 0;
                            eoe_pos.lnum = 0;
                            eoe_pos.col = 0;
                            end_idx = 0;
                            hl_endpos.lnum = 0;

                            if spp.sp_type == SPTYPE_START && (spp.sp_flags & HL_ONELINE) != 0 {
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
                            } else if spp.sp_type == SPTYPE_MATCH {
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
                                    && endpos.col + i32::from(syncing) < startcol
                                {
                                    if regmatch.startpos[0].col == regmatch.endpos[0].col {
                                        syn_st().try_next_column = true;
                                    }
                                    idx -= 1;
                                    continue;
                                }
                            }

                            if hl_startpos.lnum == syn_st().current_lnum
                                && hl_startpos.col < startcol
                            {
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
                    let lspp = syn_items(syn_block_mut(), syn_st().next_match_idx as isize);
                    if syn_st().next_match_m_endpos.lnum == syn_st().current_lnum
                        && syn_st().next_match_m_endpos.col == syn_st().current_col
                        && lspp.sp_next_list.is_some()
                    {
                        syn_st().current_next_list = lspp.sp_next_list.clone();
                        syn_st().current_next_flags = lspp.sp_flags;
                        keep_next_list = true;
                        zero_width_next_list = true;

                        zero_width_next_ga.push(syn_st().next_match_idx);
                        syn_st().next_match_idx = -1;
                    } else {
                        cur_si = crate::globals::GraphPtr::<Stateitem>::of(push_next_match());
                    }
                    found_match = true;
                }
            }
        }

        if syn_st().current_next_list.is_some() && !keep_next_list {
            if !found_match {
                line = syn_getcurline!();
                if ((syn_st().current_next_flags & HL_SKIPWHITE) != 0
                    && ascii_iswhite(
                        *crate::globals::GraphPtr::of(
                            line.wrapping_offset(syn_st().current_col as isize),
                        )
                        .borrow(),
                    ))
                    || ((syn_st().current_next_flags & HL_SKIPEMPTY) != 0
                        && crate::strings::AsByteSlice::is_empty_val(line))
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

    restore_chartab(&buf_chartab);

    syn_st().current_attr = 0;
    syn_st().current_id = 0;
    syn_st().current_trans_id = 0;
    syn_st().current_flags = 0;
    syn_st().current_seqnr = 0;
    if !cur_si.is_null() {
        let mut idx = cur_state_len() - 1;
        while idx >= 0 {
            sip = crate::globals::GraphPtr::<Stateitem>::of(cur_state_at(idx));
            let sip = sip.borrow_mut();
            if (syn_st().current_lnum > sip.si_h_startpos.lnum
                || (syn_st().current_lnum == sip.si_h_startpos.lnum
                    && syn_st().current_col >= sip.si_h_startpos.col))
                && (sip.si_h_endpos.lnum == 0
                    || syn_st().current_lnum < sip.si_h_endpos.lnum
                    || (syn_st().current_lnum == sip.si_h_endpos.lnum
                        && syn_st().current_col < sip.si_h_endpos.col))
            {
                syn_st().current_attr = sip.si_attr;
                syn_st().current_id = sip.si_id;
                syn_st().current_trans_id = sip.si_trans_id;
                syn_st().current_flags = sip.si_flags;
                syn_st().current_seqnr = sip.si_seqnr;
                syn_st().current_sub_char = sip.si_cchar;
                break;
            }
            idx -= 1;
        }

        if let Some(can_spell) = can_spell {
            let mut sps = SpSyn {
                inc_tag: 0,
                id: 0,
                cont_in_list: None,
            };

            if syn_block_mut().b_spell_cluster_id == 0 {
                if syn_block_mut().b_nospell_cluster_id == 0 || syn_st().current_trans_id == 0 {
                    *can_spell = syn_block_mut().b_syn_spell != SYNSPL_NOTOP;
                } else {
                    sps.id = syn_block_mut().b_nospell_cluster_id as i16;

                    *can_spell =
                        in_id_list(sip.opt(), &(sip.borrow().si_cont_list), &mut sps, 0) == 0;
                }
            } else if syn_st().current_trans_id == 0 {
                *can_spell = syn_block_mut().b_syn_spell == SYNSPL_TOP;
            } else {
                sps.id = syn_block_mut().b_spell_cluster_id as i16;

                *can_spell = in_id_list(sip.opt(), &(sip.borrow().si_cont_list), &mut sps, 0) != 0;

                if syn_block_mut().b_nospell_cluster_id != 0 {
                    sps.id = syn_block_mut().b_nospell_cluster_id as i16;
                    if in_id_list(sip.opt(), &(sip.borrow().si_cont_list), &mut sps, 0) != 0 {
                        *can_spell = false;
                    }
                }
            }
        }

        if !syncing && !keep_state {
            check_state_ends();
            if !cur_state_empty()
                && *crate::globals::GraphPtr::of(
                    syn_getcurline!().wrapping_offset(syn_st().current_col as isize),
                )
                .borrow()
                    != NUL
            {
                syn_st().current_col += 1;
                check_state_ends();
                syn_st().current_col -= 1;
            }
        }
    } else if let Some(can_spell) = can_spell {
        *can_spell = if syn_block_mut().b_syn_spell == SYNSPL_DEFAULT {
            syn_block_mut().b_spell_cluster_id == 0
        } else {
            syn_block_mut().b_syn_spell == SYNSPL_TOP
        };
    }

    if syn_st().current_next_list.is_some() {
        line = syn_getcurline!();
        if *crate::globals::GraphPtr::of(line.wrapping_offset(syn_st().current_col as isize))
            .borrow()
            != 0
            && *crate::globals::GraphPtr::of(
                line.wrapping_offset((syn_st().current_col + 1) as isize),
            )
            .borrow()
                == NUL
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

fn did_match_already(idx: i32, gap: &[i32]) -> bool {
    let mut i = cur_state_len() - 1;
    while i >= 0 {
        let cur_si = cur_state_at(i);
        if cur_si.si_m_startcol == syn_st().current_col
            && cur_si.si_m_lnum == syn_st().current_lnum
            && cur_si.si_idx == idx
        {
            return true;
        }
        i -= 1;
    }

    gap.contains(&idx)
}

fn push_next_match() -> &'static mut Stateitem {
    let spp = syn_items(syn_block_mut(), syn_st().next_match_idx as isize);

    push_current_state(syn_st().next_match_idx);
    let len = cur_state_len();
    let mut cur_si = cur_state_at(len - 1);
    cur_si.si_h_startpos = syn_st().next_match_h_startpos;
    cur_si.si_m_startcol = syn_st().current_col;
    cur_si.si_m_lnum = syn_st().current_lnum;
    cur_si.si_flags = spp.sp_flags;
    cur_si.si_seqnr = syn_st().next_seqnr;
    syn_st().next_seqnr += 1;
    cur_si.si_cchar = spp.sp_cchar;
    if len > 1 {
        cur_si.si_flags |= cur_state_at(len - 2).si_flags & HL_CONCEAL;
    }
    cur_si.si_next_list = spp.sp_next_list.clone();
    cur_si.si_extmatch = syn_st().next_match_extmatch.clone();
    if spp.sp_type == SPTYPE_START && (spp.sp_flags & HL_ONELINE) == 0 {
        update_si_end(&mut *cur_si, syn_st().next_match_m_endpos.col, true);
        check_keepend();
    } else {
        cur_si.si_m_endpos = syn_st().next_match_m_endpos;
        cur_si.si_h_endpos = syn_st().next_match_h_endpos;
        cur_si.si_ends = 1;
        cur_si.si_flags |= syn_st().next_match_flags;
        cur_si.si_eoe_pos = syn_st().next_match_eoe_pos;
        cur_si.si_end_idx = syn_st().next_match_end_idx;
    }
    if syn_st().keepend_level < 0 && (cur_si.si_flags & HL_KEEPEND) != 0 {
        syn_st().keepend_level = len - 1;
    }
    check_keepend();
    update_si_attr(len - 1);

    let save_flags = cur_si.si_flags & (HL_CONCEAL | HL_CONCEALENDS);
    if spp.sp_type == SPTYPE_START && spp.sp_syn_match_id != 0 {
        push_current_state(syn_st().next_match_idx);
        let len2 = cur_state_len();
        cur_si = cur_state_at(len2 - 1);
        cur_si.si_h_startpos = syn_st().next_match_h_startpos;
        cur_si.si_m_startcol = syn_st().current_col;
        cur_si.si_m_lnum = syn_st().current_lnum;
        cur_si.si_m_endpos = syn_st().next_match_eos_pos;
        cur_si.si_h_endpos = syn_st().next_match_eos_pos;
        cur_si.si_ends = 1;
        cur_si.si_end_idx = 0;
        cur_si.si_flags = HL_MATCH;
        cur_si.si_seqnr = syn_st().next_seqnr;
        syn_st().next_seqnr += 1;
        cur_si.si_flags |= save_flags;
        if (cur_si.si_flags & HL_CONCEALENDS) != 0 {
            cur_si.si_flags |= HL_CONCEAL;
        }
        cur_si.si_next_list = None;
        check_keepend();
        update_si_attr(len2 - 1);
    }

    syn_st().next_match_idx = -1;
    cur_si
}

fn check_state_ends() {
    let mut cur_si = cur_state_at(cur_state_len() - 1);
    loop {
        if cur_si.si_ends != 0
            && (cur_si.si_m_endpos.lnum < syn_st().current_lnum
                || (cur_si.si_m_endpos.lnum == syn_st().current_lnum
                    && cur_si.si_m_endpos.col <= syn_st().current_col))
        {
            if cur_si.si_end_idx != 0
                && (cur_si.si_eoe_pos.lnum > syn_st().current_lnum
                    || (cur_si.si_eoe_pos.lnum == syn_st().current_lnum
                        && cur_si.si_eoe_pos.col > syn_st().current_col))
            {
                cur_si.si_idx = cur_si.si_end_idx;
                cur_si.si_end_idx = 0;
                cur_si.si_m_endpos = cur_si.si_eoe_pos;
                cur_si.si_h_endpos = cur_si.si_eoe_pos;
                cur_si.si_flags |= HL_MATCH;
                cur_si.si_seqnr = syn_st().next_seqnr;
                syn_st().next_seqnr += 1;
                if (cur_si.si_flags & HL_CONCEALENDS) != 0 {
                    cur_si.si_flags |= HL_CONCEAL;
                }
                update_si_attr(cur_state_len() - 1);

                syn_st().current_next_list = None;
                syn_st().next_match_idx = 0;
                syn_st().next_match_col = MAXCOL;
                break;
            }

            syn_st().current_next_list = cur_si.si_next_list.clone();
            syn_st().current_next_flags = cur_si.si_flags;
            if (syn_st().current_next_flags & (HL_SKIPNL | HL_SKIPEMPTY)) == 0
                && *crate::globals::GraphPtr::of(
                    syn_getcurline!().wrapping_offset(syn_st().current_col as isize),
                )
                .borrow()
                    == 0
            {
                syn_st().current_next_list = None;
            }

            let had_extend = (cur_si.si_flags & HL_EXTEND) != 0;

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

            if cur_si.si_idx >= 0
                && syn_items(syn_block_mut(), cur_si.si_idx as isize).sp_type == SPTYPE_START
                && (cur_si.si_flags & (HL_MATCH | HL_KEEPEND)) == 0
            {
                update_si_end(&mut *cur_si, syn_st().current_col, true);
                check_keepend();
                if (syn_st().current_next_flags & HL_HAS_EOL) != 0
                    && syn_st().keepend_level < 0
                    && *crate::globals::GraphPtr::of(
                        syn_getcurline!().wrapping_offset(syn_st().current_col as isize),
                    )
                    .borrow()
                        == 0
                {
                    break;
                }
            }
        } else {
            break;
        }
    }
}

fn update_si_attr(idx: i32) {
    let sip = cur_state_at(idx);
    if sip.si_idx < 0 {
        return;
    }

    let spp = syn_items(syn_block_mut(), sip.si_idx as isize);
    if (sip.si_flags & HL_MATCH) != 0 {
        sip.si_id = spp.sp_syn_match_id as i32;
    } else {
        sip.si_id = spp.sp_syn.id as i32;
    }
    sip.si_attr = syn_id2attr(sip.si_id);
    sip.si_trans_id = sip.si_id;
    if (sip.si_flags & HL_MATCH) != 0 {
        sip.si_cont_list = None;
    } else {
        sip.si_cont_list = spp.sp_cont_list.clone();
    }

    if (spp.sp_flags & HL_TRANSP) != 0 && (sip.si_flags & HL_MATCH) == 0 {
        if idx == 0 {
            sip.si_attr = 0;
            sip.si_trans_id = 0;
            if sip.si_cont_list.is_none() {
                sip.si_cont_list = id_list_all();
            }
        } else {
            let prev_si = cur_state_at(idx - 1);
            sip.si_attr = prev_si.si_attr;
            sip.si_trans_id = prev_si.si_trans_id;
            if sip.si_cont_list.is_none() {
                sip.si_flags |= HL_TRANS_CONT;
                sip.si_cont_list = prev_si.si_cont_list.clone();
            }
        }
    }
}

fn check_keepend() {
    if syn_st().keepend_level < 0 {
        return;
    }

    let len = cur_state_len();
    let mut i = len - 1;
    while i > syn_st().keepend_level {
        if (cur_state_at(i).si_flags & HL_EXTEND) != 0 {
            break;
        }
        i -= 1;
    }

    let mut maxpos = Lpos { lnum: 0, col: 0 };
    let mut maxpos_h = Lpos { lnum: 0, col: 0 };
    while i < len {
        let sip = cur_state_at(i);
        if maxpos.lnum != 0 {
            limit_pos_zero(&mut sip.si_m_endpos, &mut maxpos);
            limit_pos_zero(&mut sip.si_h_endpos, &mut maxpos_h);
            limit_pos_zero(&mut sip.si_eoe_pos, &mut maxpos);
            sip.si_ends = 1;
        }
        if sip.si_ends != 0 && (sip.si_flags & HL_KEEPEND) != 0 {
            if maxpos.lnum == 0
                || maxpos.lnum > sip.si_m_endpos.lnum
                || (maxpos.lnum == sip.si_m_endpos.lnum && maxpos.col > sip.si_m_endpos.col)
            {
                maxpos = sip.si_m_endpos;
            }
            if maxpos_h.lnum == 0
                || maxpos_h.lnum > sip.si_h_endpos.lnum
                || (maxpos_h.lnum == sip.si_h_endpos.lnum && maxpos_h.col > sip.si_h_endpos.col)
            {
                maxpos_h = sip.si_h_endpos;
            }
        }
        i += 1;
    }
}

fn update_si_end(sip: &mut Stateitem, startcol: i32, force: bool) {
    if sip.si_idx < 0 {
        return;
    }

    if !force && sip.si_m_endpos.lnum >= syn_st().current_lnum {
        return;
    }

    let mut end_idx: i32 = 0;
    let mut startpos = Lpos {
        lnum: syn_st().current_lnum,
        col: startcol,
    };
    let mut endpos = Lpos { lnum: 0, col: 0 };
    let mut hl_endpos = Lpos { lnum: 0, col: 0 };
    let mut end_endpos = Lpos { lnum: 0, col: 0 };

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
        if (syn_items(syn_block_mut(), sip.si_idx as isize).sp_flags & HL_ONELINE) != 0 {
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

fn push_current_state(idx: i32) {
    let mut item = Stateitem::new();
    item.si_idx = idx;
    syn_st().current_state.push(item);
}

fn pop_current_state() {
    let _ = syn_st().current_state.pop();
    syn_st().next_match_idx = -1;

    if syn_st().keepend_level >= syn_st().current_state.len() as i32 {
        syn_st().keepend_level = -1;
    }
}

fn find_endpos(
    mut idx: i32,
    startpos: &mut Lpos,
    m_endpos: &mut Lpos,
    hl_endpos: &mut Lpos,
    flagsp: &mut i32,
    end_endpos: &mut Lpos,
    end_idx: &mut i32,
    start_ext: Option<std::rc::Rc<RegExtmatch>>,
) {
    if idx < 0 {
        return;
    }

    let mut spp = syn_items(syn_block_mut(), idx as isize);
    if spp.sp_type != SPTYPE_START {
        *hl_endpos = *startpos;
        return;
    }

    loop {
        spp = syn_items(syn_block_mut(), idx as isize);
        if spp.sp_type != SPTYPE_START {
            break;
        }
        idx += 1;
    }

    let spp_skip = if spp.sp_type == SPTYPE_SKIP {
        idx += 1;
        spp
    } else {
        ptr::null_mut()
    };

    g_st().re_extmatch_in = start_ext;

    let mut matchcol = startpos.col;
    let start_idx = idx;
    let mut regmatch: Regmmatch = Regmmatch::new();
    let mut best_regmatch: Regmmatch = Regmmatch::new();
    let mut pos = Lpos { lnum: 0, col: 0 };
    let mut had_match = false;
    let mut buf_chartab = [0u64; 4];

    save_chartab(&mut buf_chartab);

    loop {
        let mut best_idx: i32 = -1;
        idx = start_idx;
        while (idx as usize) < syn_block_mut().b_syn_patterns.len() {
            let mut lc_col = matchcol;
            spp = syn_items(syn_block_mut(), idx as isize);
            if spp.sp_type != SPTYPE_END {
                break;
            }
            lc_col -= spp.sp_offsets[SPO_LC_OFF];
            if lc_col < 0 {
                lc_col = 0;
            }

            regmatch.rmm_ic = spp.sp_ic;
            regmatch.borrow_prog(spp.sp_prog.as_deref());
            let r = syn_regexec(&mut regmatch, startpos.lnum, lc_col, &mut spp.sp_time);
            if r && (best_idx == -1 || regmatch.startpos[0].col < best_regmatch.startpos[0].col) {
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
            let spp_skip = crate::globals::GraphPtr::of(spp_skip).borrow_mut();
            let mut lc_col = matchcol - spp_skip.sp_offsets[SPO_LC_OFF];
            if lc_col < 0 {
                lc_col = 0;
            }
            regmatch.rmm_ic = spp_skip.sp_ic;
            regmatch.borrow_prog(spp_skip.sp_prog.as_deref());
            let r = syn_regexec(&mut regmatch, startpos.lnum, lc_col, &mut spp_skip.sp_time);
            if r && regmatch.startpos[0].col <= best_regmatch.startpos[0].col {
                syn_add_end_off(&mut pos, &mut regmatch, &mut *spp_skip, SPO_ME_OFF, 1);

                if pos.lnum > startpos.lnum {
                    break;
                }

                let line_len = ml_get_buf_len(syn_st().syn_buf_mut(), startpos.lnum);

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

        spp = syn_items(syn_block_mut(), best_idx as isize);
        syn_add_end_off(m_endpos, &mut best_regmatch, &mut *spp, SPO_ME_OFF, 1);
        if m_endpos.lnum == startpos.lnum && m_endpos.col < startpos.col {
            m_endpos.col = startpos.col;
        }

        syn_add_end_off(end_endpos, &mut best_regmatch, &mut *spp, SPO_HE_OFF, 1);
        if end_endpos.lnum == startpos.lnum && end_endpos.col < startpos.col {
            end_endpos.col = startpos.col;
        }
        limit_pos(end_endpos, m_endpos);

        if spp.sp_syn_match_id != spp.sp_syn.id && spp.sp_syn_match_id != 0 {
            *end_idx = best_idx;
            if (spp.sp_off_flags & (1 << (SPO_RE_OFF + SPO_COUNT))) != 0 {
                hl_endpos.lnum = best_regmatch.endpos[0].lnum;
                hl_endpos.col = best_regmatch.endpos[0].col;
            } else {
                hl_endpos.lnum = best_regmatch.startpos[0].lnum;
                hl_endpos.col = best_regmatch.startpos[0].col;
            }
            hl_endpos.col += spp.sp_offsets[SPO_RE_OFF];

            if hl_endpos.lnum == startpos.lnum && hl_endpos.col < startpos.col {
                hl_endpos.col = startpos.col;
            }
            limit_pos(hl_endpos, m_endpos);

            *m_endpos = *hl_endpos;
        } else {
            *end_idx = 0;
            *hl_endpos = *end_endpos;
        }

        *flagsp = spp.sp_flags;
        had_match = true;
        break;
    }

    if !had_match {
        m_endpos.lnum = 0;
    }

    restore_chartab(&buf_chartab);

    g_st().re_extmatch_in = None;
}

fn limit_pos(pos: &mut Lpos, limit: &mut Lpos) {
    if pos.lnum > limit.lnum {
        *pos = *limit;
    } else if pos.lnum == limit.lnum && pos.col > limit.col {
        pos.col = limit.col;
    }
}

fn limit_pos_zero(pos: &mut Lpos, limit: &mut Lpos) {
    if pos.lnum == 0 {
        *pos = *limit;
    } else {
        limit_pos(pos, limit);
    }
}

fn syn_add_end_off(
    result: &mut Lpos,
    regmatch: &mut Regmmatch,
    spp: &mut Synpat,
    idx: usize,
    extra: i32,
) {
    let mut col: i32;
    let mut off: i32;

    if (spp.sp_off_flags & (1 << idx)) != 0 {
        result.lnum = regmatch.startpos[0].lnum;
        col = regmatch.startpos[0].col;
        off = spp.sp_offsets[idx] + extra;
    } else {
        result.lnum = regmatch.endpos[0].lnum;
        col = regmatch.endpos[0].col;
        off = spp.sp_offsets[idx];
    }

    if result.lnum > syn_st().syn_buf_mut().b_ml.ml_line_count {
        col = 0;
    } else if off != 0 {
        let line = crate::memline::ml_get_buf_slice(syn_st().syn_buf_mut(), result.lnum);
        let mut col_idx = (col.max(0) as usize).min(line.len());
        if off > 0 {
            while off > 0 && col_idx < line.len() {
                off -= 1;
                col_idx += utfc_slice2len(&line[col_idx..]).max(1);
                col_idx = col_idx.min(line.len());
            }
        } else {
            while off < 0 && col_idx > 0 {
                off += 1;
                crate::mbyte::mb_idx_back(line, &mut col_idx);
            }
        }
        col = col_idx as i32;
    }
    result.col = col;
}

fn syn_add_start_off(
    result: &mut Lpos,
    regmatch: &mut Regmmatch,
    spp: &mut Synpat,
    idx: usize,
    extra: i32,
) {
    let mut col: i32;
    let mut off: i32;

    if (spp.sp_off_flags & (1 << (idx + SPO_COUNT))) != 0 {
        result.lnum = regmatch.endpos[0].lnum;
        col = regmatch.endpos[0].col;
        off = spp.sp_offsets[idx] + extra;
    } else {
        result.lnum = regmatch.startpos[0].lnum;
        col = regmatch.startpos[0].col;
        off = spp.sp_offsets[idx];
    }
    if result.lnum > syn_st().syn_buf_mut().b_ml.ml_line_count {
        result.lnum = syn_st().syn_buf_mut().b_ml.ml_line_count;
        col = ml_get_buf_len(syn_st().syn_buf_mut(), result.lnum);
    }
    if off != 0 {
        let line = crate::memline::ml_get_buf_slice(syn_st().syn_buf_mut(), result.lnum);
        let mut col_idx = (col.max(0) as usize).min(line.len());
        if off > 0 {
            while off > 0 && col_idx < line.len() {
                off -= 1;
                col_idx += utfc_slice2len(&line[col_idx..]).max(1);
                col_idx = col_idx.min(line.len());
            }
        } else {
            while off < 0 && col_idx > 0 {
                off += 1;
                crate::mbyte::mb_idx_back(line, &mut col_idx);
            }
        }
        col = col_idx as i32;
    }
    result.col = col;
}

#[inline]
fn syn_getcurline_len() -> Colnr {
    ml_get_buf_len(syn_st().syn_buf_mut(), syn_st().current_lnum)
}

fn syn_regexec(rmp: &mut Regmmatch, lnum: Linenr, col: Colnr, st: &mut SynTime) -> bool {
    let mut timed_out: i32 = 0;
    let mut pt: Proftime = 0;
    let l_syn_time_on = syn_st().syn_time_on;

    if l_syn_time_on {
        pt = profile_start();
    }

    if rmp.regprog.is_null() {
        return false;
    }

    rmp.rmm_maxcol = syn_st().syn_buf_mut().b_p_smc as Colnr;
    let syn_win = syn_st().syn_win.opt_mut();
    let syn_buf = syn_st().syn_buf_mut();
    let r = vim_regexec_multi(
        rmp,
        syn_win,
        syn_buf,
        lnum,
        col,
        syn_st().syn_tm,
        Some(&mut timed_out),
    );

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
    if timed_out != 0 && !win_s(syn_st().syn_win.borrow_mut()).b_syn_slow {
        win_s(syn_st().syn_win.borrow_mut()).b_syn_slow = true;
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

fn check_keyword_id(
    line: &[u8],
    startcol: i32,
    endcolp: &mut i32,
    flagsp: &mut i32,
    next_listp: &mut SynIdList,
    cur_si: Option<&Stateitem>,
    ccharp: &mut i32,
) -> i32 {
    let kwp = &line[(startcol as usize).min(line.len())..];
    let mut kwlen: usize = 0;
    loop {
        kwlen += crate::mbyte::utfc_slice2len(&kwp[kwlen..]).max(1);
        if kwlen >= kwp.len() || !vim_iswordp_buf(&kwp[kwlen..], syn_st().syn_buf_mut()) {
            break;
        }
    }
    kwlen = kwlen.min(kwp.len());

    if kwlen > MAXKEYWLEN {
        return 0;
    }

    let mut kp: Option<&Keyentry> = None;

    if !syn_block_mut().b_keywtab.is_empty() {
        kp = match_keyword(&kwp[..kwlen], &syn_block_mut().b_keywtab, cur_si);
    }

    if kp.is_none() && !syn_block_mut().b_keywtab_ic.is_empty() {
        let mut keyword = [0u8; MAXKEYWLEN + 1];
        str_foldcase(&kwp[..kwlen], Some(&mut keyword));
        let folded_len = crate::memory::bytes_until_nul(&keyword).len();
        kp = match_keyword(
            &keyword[..folded_len],
            &syn_block_mut().b_keywtab_ic,
            cur_si,
        );
    }

    if let Some(kp) = kp {
        *endcolp = startcol + kwlen as i32;
        *flagsp = kp.flags;
        *next_listp = kp.next_list.clone();
        *ccharp = kp.k_char;
        return kp.k_syn.id as i32;
    }

    0
}

fn match_keyword<'a>(
    key_bytes: &[u8],
    ht: &'a std::collections::BTreeMap<Vec<u8>, Vec<Keyentry>>,
    cur_si: Option<&Stateitem>,
) -> Option<&'a Keyentry> {
    if let Some(entries) = ht.get(key_bytes) {
        let cur_next_list = &syn_st().current_next_list;
        for kp in entries {
            let mut k_syn = kp.k_syn.clone();
            if if cur_next_list.is_some() {
                in_id_list(None, cur_next_list, &mut k_syn, 0) != 0
            } else if let Some(cur_si_ref) = cur_si {
                in_id_list(
                    Some(cur_si_ref),
                    &cur_si_ref.si_cont_list,
                    &mut k_syn,
                    kp.flags,
                ) != 0
            } else {
                (kp.flags & HL_CONTAINED) == 0
            } {
                return Some(kp);
            }
        }
    }
    None
}

fn syn_cmd_conceal(eap: &mut Exarg, _syncing: i32) {
    let arg = eap.arg;
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    let next = arg.skiptowhite();
    if (arg).is_empty() {
        if win_s(curwin_mut()).b_syn_conceal != 0 {
            msg(c"syntax conceal on", 0);
        } else {
            msg(c"syntax conceal off", 0);
        }
    } else if ((next).offset_from(arg) as isize) == 2 && (arg).slice(2).eq_ignore_ascii_case(b"on")
    {
        win_s(curwin_mut()).b_syn_conceal = 1;
    } else if ((next).offset_from(arg) as isize) == 3 && (arg).slice(3).eq_ignore_ascii_case(b"off")
    {
        win_s(curwin_mut()).b_syn_conceal = 0;
    } else {
        semsg!(gettext(E_ILLEGAL_ARG), arg);
    }
}

fn syn_cmd_case(eap: &mut Exarg, _syncing: i32) {
    let arg = eap.arg;
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    let next = arg.skiptowhite();
    if (arg).is_empty() {
        if win_s(curwin_mut()).b_syn_ic {
            msg(c"syntax case ignore", 0);
        } else {
            msg(c"syntax case match", 0);
        }
    } else if ((next).offset_from(arg) as isize) == 5
        && (arg).slice(5).eq_ignore_ascii_case(b"match")
    {
        win_s(curwin_mut()).b_syn_ic = false;
    } else if ((next).offset_from(arg) as isize) == 6
        && (arg).slice(6).eq_ignore_ascii_case(b"ignore")
    {
        win_s(curwin_mut()).b_syn_ic = true;
    } else {
        semsg!(gettext(E_ILLEGAL_ARG), arg);
    }
}

fn syn_cmd_foldlevel(eap: &mut Exarg, _syncing: i32) {
    let mut arg = eap.arg;
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    if (arg).is_empty() {
        match win_s(curwin_mut()).b_syn_foldlevel {
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

    let arg_end = arg.skiptowhite();
    if ((arg_end).offset_from(arg) as isize) == 5 && (arg).slice(5).eq_ignore_ascii_case(b"start") {
        win_s(curwin_mut()).b_syn_foldlevel = SYNFLD_START;
    } else if ((arg_end).offset_from(arg) as isize) == 7
        && (arg).slice(7).eq_ignore_ascii_case(b"minimum")
    {
        win_s(curwin_mut()).b_syn_foldlevel = SYNFLD_MINIMUM;
    } else {
        semsg!(gettext(E_ILLEGAL_ARG), arg);
        return;
    }

    arg = arg_end.skipwhite();
    if !(arg).is_empty() {
        semsg!(gettext(E_ILLEGAL_ARG), arg);
    }
}

fn syn_cmd_spell(eap: &mut Exarg, _syncing: i32) {
    let arg = eap.arg;
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    let next = arg.skiptowhite();
    if (arg).is_empty() {
        if win_s(curwin_mut()).b_syn_spell == SYNSPL_TOP {
            msg(c"syntax spell toplevel", 0);
        } else if win_s(curwin_mut()).b_syn_spell == SYNSPL_NOTOP {
            msg(c"syntax spell notoplevel", 0);
        } else {
            msg(c"syntax spell default", 0);
        }
    } else if ((next).offset_from(arg) as isize) == 8
        && (arg).slice(8).eq_ignore_ascii_case(b"toplevel")
    {
        win_s(curwin_mut()).b_syn_spell = SYNSPL_TOP;
    } else if ((next).offset_from(arg) as isize) == 10
        && (arg).slice(10).eq_ignore_ascii_case(b"notoplevel")
    {
        win_s(curwin_mut()).b_syn_spell = SYNSPL_NOTOP;
    } else if ((next).offset_from(arg) as isize) == 7
        && (arg).slice(7).eq_ignore_ascii_case(b"default")
    {
        win_s(curwin_mut()).b_syn_spell = SYNSPL_DEFAULT;
    } else {
        semsg!(gettext(E_ILLEGAL_ARG), arg);
        return;
    }

    redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
}

fn syn_cmd_iskeyword(eap: &mut Exarg, _syncing: i32) {
    let mut arg = eap.arg;
    if eap.skip {
        return;
    }

    arg = arg.skipwhite();
    if (arg).is_empty() {
        msg_puts(c"\n");
        if let Some(ref isk) = win_s(curwin_mut()).b_syn_isk {
            msg_puts(c"syntax iskeyword ");
            msg_outtrans(isk, 0, false);
        } else {
            msg_outtrans(gettext(c"syntax iskeyword not set"), 0, false);
        }
    } else if (arg)
        .as_bytes()
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case(b"clear"))
    {
        win_s(curwin_mut()).b_syn_chartab = curbuf_mut().b_chartab;
        win_s(curwin_mut()).b_syn_isk = None;
    } else {
        let arg_bytes = (arg).as_bytes();
        if !check_isopt(arg_bytes) {
            emsg(gettext(E_INVARG));
            return;
        }
        let isk = crate::memory::bytes_to_cstring(arg_bytes);
        parse_isopt(isk.as_bytes(), Some(&mut win_s(curwin_mut()).b_syn_chartab));
        win_s(curwin_mut()).b_syn_isk = Some(isk);
    }
    redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
}

/// Clear all syntax info for one buffer.
pub fn syntax_clear(block: &mut Synblock) {
    block.b_syn_error = false;
    block.b_syn_slow = false;
    block.b_syn_ic = false;
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

    syn_stack_free_all(block);
    invalidate_current_state();

    syn_st().running_syn_inc_tag = 0;
}

impl Drop for Synblock {
    fn drop(&mut self) {
        syntax_clear(self);
    }
}

/// Get rid of ownsyntax for window "wp".
pub fn reset_synblock(wp: &mut WinInfoInfo) {
    wp.w_s_owned = None;
}

fn syntax_sync_clear() {
    let mut i = win_s(curwin_mut()).b_syn_patterns.len() as i32 - 1;
    while i >= 0 {
        if syn_items(&mut *win_s(curwin_mut()), i as isize).sp_syncing {
            syn_remove_pattern(&mut *win_s(curwin_mut()), i);
        }
        i -= 1;
    }

    win_s(curwin_mut()).b_syn_sync_flags = 0;
    win_s(curwin_mut()).b_syn_sync_minlines = 0;
    win_s(curwin_mut()).b_syn_sync_maxlines = 0;
    win_s(curwin_mut()).b_syn_sync_linebreaks = 0;

    win_s(curwin_mut()).b_syn_linecont_prog = None;
    win_s(curwin_mut()).b_syn_linecont_pat = None;
    win_s(curwin_mut()).b_syn_isk = None;

    syn_stack_free_all(win_s(curwin_mut()));
}

fn syn_remove_pattern(block: &mut Synblock, idx: i32) {
    let spp = syn_items(block, idx as isize);
    if (spp.sp_flags & HL_FOLD) != 0 {
        block.b_syn_folditems -= 1;
    }
    let pats = &mut block.b_syn_patterns;
    pats.remove(idx as usize);
}

fn syn_cmd_clear(eap: &mut Exarg, syncing: i32) {
    let mut arg = eap.arg;
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    if win_s(curwin_mut()).b_syn_topgrp != 0 {
        return;
    }

    if ends_excmd((arg).first_byte()) != 0 {
        if syncing != 0 {
            syntax_sync_clear();
        } else {
            syntax_clear(&mut *win_s(curwin_mut()));
            if ptr::eq(win_s(curwin_mut()), &raw mut curwin_mut().buf_mut().b_s) {
                do_unlet(b"b:current_syntax", true);
            }
            do_unlet(b"w:current_syntax", true);
        }
    } else {
        while ends_excmd((arg).first_byte()) == 0 {
            let arg_end = arg.skiptowhite();
            if (arg).first_byte() == b'@' {
                let id = syn_scl_namen2id(
                    arg.wrapping_add(1)
                        .slice(((arg_end).offset_from(arg) as isize).max(1) as usize - 1),
                );
                if id == 0 {
                    semsg!(gettext(c"E391: No such syntax cluster: %s"), arg);
                    break;
                }
                let scl_id = id - SYNID_CLUSTER as i32;
                let clp = syn_clstr(&mut *win_s(curwin_mut()), scl_id as isize);
                clp.scl_list = None;
            } else {
                let id = syn_name2id_len(arg.slice(((arg_end).offset_from(arg) as isize) as usize));
                if id == 0 {
                    semsg!(gettext(E_NOGROUP), arg);
                    break;
                }
                syn_clear_one(id, syncing != 0);
            }
            arg = arg_end.skipwhite();
        }
    }
    redraw_curbuf_later(UPD_SOME_VALID);
    syn_stack_free_all(win_s(curwin_mut()));
}

fn syn_clear_one(id: i32, syncing: bool) {
    if !syncing {
        syn_clear_keyword(id, &mut win_s(curwin_mut()).b_keywtab);
        syn_clear_keyword(id, &mut win_s(curwin_mut()).b_keywtab_ic);
    }

    let mut idx = win_s(curwin_mut()).b_syn_patterns.len() as i32 - 1;
    while idx >= 0 {
        let spp = syn_items(&mut *win_s(curwin_mut()), idx as isize);
        if spp.sp_syn.id as i32 == id && spp.sp_syncing == syncing {
            syn_remove_pattern(&mut *win_s(curwin_mut()), idx);
        }
        idx -= 1;
    }
}

fn syn_cmd_on(eap: &mut Exarg, _syncing: i32) {
    syn_cmd_onoff(eap, c"syntax");
}

fn syn_cmd_reset(eap: &mut Exarg, _syncing: i32) {
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(check_nextcmd(eap.arg.as_cstr()));
    if !eap.skip {
        init_highlight(true, true);
    }
}

fn syn_cmd_manual(eap: &mut Exarg, _syncing: i32) {
    syn_cmd_onoff(eap, c"manual");
}

fn syn_cmd_off(eap: &mut Exarg, _syncing: i32) {
    syn_cmd_onoff(eap, c"nosyntax");
}

fn syn_cmd_onoff(eap: &mut Exarg, name: &std::ffi::CStr) {
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(check_nextcmd(eap.arg.as_cstr()));
    if !eap.skip {
        syn_st().did_syntax_onoff = true;
        let mut buf = [0u8; 100];
        buf[..3].copy_from_slice(b"so ");
        format_to_buf!(&mut buf[3..], SYNTAX_FNAME, name);
        do_cmdline_cmd(crate::memory::bytes_as_cstr(&buf));
    }
}

pub fn syn_maybe_enable() {
    if !syn_st().did_syntax_onoff {
        let mut ea: Exarg = Exarg::new();
        ea.arg = crate::strings::ByteSlice::from_cstr(c"");
        ea.skip = false;
        syn_cmd_on(&mut ea, 0);
    }
}

fn syn_cmd_list(eap: &mut Exarg, syncing: i32) {
    let mut arg = eap.arg;
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    msg_ext_set_kind(c"list_cmd");
    if !syntax_present(curwin_mut()) {
        msg(gettext(MSG_NO_ITEMS), 0);
        return;
    }

    if syncing != 0 {
        if (win_s(curwin_mut()).b_syn_sync_flags & SF_CCOMMENT) != 0 {
            msg_puts(gettext(c"syncing on C-style comments"));
            syn_lines_msg();
            syn_match_msg();
        } else if (win_s(curwin_mut()).b_syn_sync_flags & SF_MATCH) != 0 {
            msg_puts_title(gettext(c"\n--- Syntax sync items ---"));
            if win_s(curwin_mut()).b_syn_sync_minlines > 0
                || win_s(curwin_mut()).b_syn_sync_maxlines > 0
                || win_s(curwin_mut()).b_syn_sync_linebreaks > 0
            {
                msg_puts(gettext(c"\nsyncing on items"));
                syn_lines_msg();
                syn_match_msg();
            }
            let mut id: i32 = 1;
            while id <= highlight_num_groups() && !g_st().got_int {
                syn_list_one(id, true, false);
                id += 1;
            }
        } else if win_s(curwin_mut()).b_syn_sync_minlines == 0 {
            msg_puts(gettext(c"no syncing"));
        } else {
            if win_s(curwin_mut()).b_syn_sync_minlines == MAXLNUM {
                msg_puts(gettext(c"syncing starts at the first line"));
            } else {
                msg_puts(gettext(c"syncing starts "));
                msg_outnum(win_s(curwin_mut()).b_syn_sync_minlines);
                msg_puts(gettext(c" lines before top line"));
            }
            syn_match_msg();
        }
        return;
    }

    msg_puts_title(gettext(c"\n--- Syntax items ---"));
    if ends_excmd((arg).first_byte()) != 0 {
        let mut id: i32 = 1;
        while id <= highlight_num_groups() && !g_st().got_int {
            syn_list_one(id, false, false);
            id += 1;
        }
        let mut cid: i32 = 0;
        while (cid as usize) < win_s(curwin_mut()).b_syn_clusters.len() && !g_st().got_int {
            syn_list_cluster(cid);
            cid += 1;
        }
    } else {
        while ends_excmd((arg).first_byte()) == 0 && !g_st().got_int {
            let arg_end = arg.skiptowhite();
            if (arg).first_byte() == b'@' {
                let id = syn_scl_namen2id(
                    arg.wrapping_add(1)
                        .slice(((arg_end).offset_from(arg) as isize).max(1) as usize - 1),
                );
                if id == 0 {
                    semsg!(gettext(c"E392: No such syntax cluster: %s"), arg);
                } else {
                    syn_list_cluster(id - SYNID_CLUSTER as i32);
                }
            } else {
                let id = syn_name2id_len(arg.slice(((arg_end).offset_from(arg) as isize) as usize));
                if id == 0 {
                    semsg!(gettext(E_NOGROUP), arg);
                } else {
                    syn_list_one(id, false, true);
                }
            }
            arg = arg_end.skipwhite();
        }
    }
    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(check_nextcmd((arg).as_cstr()));
}

fn syn_lines_msg() {
    if win_s(curwin_mut()).b_syn_sync_maxlines > 0 || win_s(curwin_mut()).b_syn_sync_minlines > 0 {
        msg_puts(c"; ");
        if win_s(curwin_mut()).b_syn_sync_minlines == MAXLNUM {
            msg_puts(gettext(c"from the first line"));
        } else {
            if win_s(curwin_mut()).b_syn_sync_minlines > 0 {
                msg_puts(gettext(c"minimal "));
                msg_outnum(win_s(curwin_mut()).b_syn_sync_minlines);
                if win_s(curwin_mut()).b_syn_sync_maxlines != 0 {
                    msg_puts(c", ");
                }
            }
            if win_s(curwin_mut()).b_syn_sync_maxlines > 0 {
                msg_puts(gettext(c"maximal "));
                msg_outnum(win_s(curwin_mut()).b_syn_sync_maxlines);
            }
            msg_puts(gettext(c" lines before top line"));
        }
    }
}

fn syn_match_msg() {
    if win_s(curwin_mut()).b_syn_sync_linebreaks > 0 {
        msg_puts(gettext(c"; match "));
        msg_outnum(win_s(curwin_mut()).b_syn_sync_linebreaks);
        msg_puts(gettext(c" line breaks"));
    }
}

fn syn_list_one(id: i32, syncing: bool, link_only: bool) {
    let mut did_header = false;
    const NAMELIST1: [(i32, &std::ffi::CStr); 10] = [
        (HL_DISPLAY, c"display"),
        (HL_CONTAINED, c"contained"),
        (HL_ONELINE, c"oneline"),
        (HL_KEEPEND, c"keepend"),
        (HL_EXTEND, c"extend"),
        (HL_EXCLUDENL, c"excludenl"),
        (HL_TRANSP, c"transparent"),
        (HL_FOLD, c"fold"),
        (HL_CONCEAL, c"conceal"),
        (HL_CONCEALENDS, c"concealends"),
    ];
    const NAMELIST2: [(i32, &std::ffi::CStr); 3] = [
        (HL_SKIPWHITE, c"skipwhite"),
        (HL_SKIPNL, c"skipnl"),
        (HL_SKIPEMPTY, c"skipempty"),
    ];

    let hl_id = HLF_D;

    if !syncing {
        did_header = syn_list_keywords(id, &win_s(curwin_mut()).b_keywtab, false, hl_id);
        did_header = syn_list_keywords(id, &win_s(curwin_mut()).b_keywtab_ic, did_header, hl_id);
    }

    let mut idx: i32 = 0;
    while (idx as usize) < win_s(curwin_mut()).b_syn_patterns.len() && !g_st().got_int {
        let spp = syn_items(&mut *win_s(curwin_mut()), idx as isize);
        if spp.sp_syn.id as i32 != id || spp.sp_syncing != syncing {
            idx += 1;
            continue;
        }

        syn_list_header(did_header, 0, id, true);
        did_header = true;
        syn_st().last_matchgroup = 0;
        if spp.sp_type == SPTYPE_MATCH {
            put_pattern(c"match", b' ' as i32, &*spp, hl_id);
            msg_putchar(b' ' as i32);
        } else if spp.sp_type == SPTYPE_START {
            while syn_items(&mut *win_s(curwin_mut()), idx as isize).sp_type == SPTYPE_START {
                put_pattern(
                    c"start",
                    b'=' as i32,
                    &*syn_items(&mut *win_s(curwin_mut()), idx as isize),
                    hl_id,
                );
                idx += 1;
            }
            if syn_items(&mut *win_s(curwin_mut()), idx as isize).sp_type == SPTYPE_SKIP {
                put_pattern(
                    c"skip",
                    b'=' as i32,
                    &*syn_items(&mut *win_s(curwin_mut()), idx as isize),
                    hl_id,
                );
                idx += 1;
            }
            while (idx as usize) < win_s(curwin_mut()).b_syn_patterns.len()
                && syn_items(&mut *win_s(curwin_mut()), idx as isize).sp_type == SPTYPE_END
            {
                put_pattern(
                    c"end",
                    b'=' as i32,
                    &*syn_items(&mut *win_s(curwin_mut()), idx as isize),
                    hl_id,
                );
                idx += 1;
            }
            idx -= 1;
            msg_putchar(b' ' as i32);
        }
        syn_list_flags(&NAMELIST1, spp.sp_flags, hl_id);

        if let Some(ref list) = spp.sp_cont_list {
            put_id_list(c"contains", list, hl_id);
        }

        if let Some(ref list) = spp.sp_syn.cont_in_list {
            put_id_list(c"containedin", list, hl_id);
        }

        if let Some(ref list) = spp.sp_next_list {
            put_id_list(c"nextgroup", list, hl_id);
            syn_list_flags(&NAMELIST2, spp.sp_flags, hl_id);
        }
        if (spp.sp_flags & (HL_SYNC_HERE | HL_SYNC_THERE)) != 0 {
            if (spp.sp_flags & HL_SYNC_HERE) != 0 {
                msg_puts_hl(c"grouphere", hl_id, false);
            } else {
                msg_puts_hl(c"groupthere", hl_id, false);
            }
            msg_putchar(b' ' as i32);
            if spp.sp_sync_idx >= 0 {
                let sync_id = syn_items(&mut *win_s(curwin_mut()), spp.sp_sync_idx as isize)
                    .sp_syn
                    .id as i32;
                msg_outtrans(highlight_group_name(sync_id - 1), 0, false);
            } else {
                msg_puts(c"NONE");
            }
            msg_putchar(b' ' as i32);
        }
        idx += 1;
    }

    if highlight_link_id(id - 1) != 0 && (did_header || link_only) && !g_st().got_int {
        syn_list_header(did_header, 0, id, true);
        msg_puts_hl(c"links to", hl_id, false);
        msg_putchar(b' ' as i32);
        msg_outtrans(
            highlight_group_name(highlight_link_id(id - 1) - 1),
            0,
            false,
        );
    }
}

fn syn_list_flags(nlist: &[(i32, &std::ffi::CStr)], flags: i32, hl_id: i32) {
    for &(key, value) in nlist {
        if (flags & key) != 0 {
            msg_puts_hl(value, hl_id, false);
            msg_putchar(b' ' as i32);
        }
    }
}

fn syn_list_cluster(id: i32) {
    let mut endcol: i32 = 15;

    msg_putchar(b'\n' as i32);
    let clp = syn_clstr(&mut *win_s(curwin_mut()), id as isize);
    msg_outtrans(&clp.scl_name, 0, false);

    if g_st().msg_col >= endcol {
        endcol = g_st().msg_col + 1;
    }
    if g_st().columns_global <= endcol {
        endcol = g_st().columns_global - 1;
    }

    msg_advance(endcol);
    if let Some(ref list) = clp.scl_list {
        put_id_list(c"cluster", list, HLF_D);
    } else {
        msg_puts_hl(c"cluster", HLF_D, false);
        msg_puts(c"=NONE");
    }
}

fn put_id_list(name: &std::ffi::CStr, list: &[i16], hl_id: i32) {
    msg_puts_hl(name, hl_id, false);
    msg_putchar(b'=' as i32);
    let mut i = 0;
    while let Some(&item) = list.get(i) {
        if item == 0 {
            break;
        }
        let next_non_zero = list.get(i + 1).copied().unwrap_or(0) != 0;
        if (SYNID_ALLBUT..SYNID_TOP).contains(&item) {
            if next_non_zero {
                msg_puts(c"ALLBUT");
            } else {
                msg_puts(c"ALL");
            }
        } else if (SYNID_TOP..SYNID_CONTAINED).contains(&item) {
            msg_puts(c"TOP");
        } else if (SYNID_CONTAINED..SYNID_CLUSTER).contains(&item) {
            msg_puts(c"CONTAINED");
        } else if item >= SYNID_CLUSTER {
            let scl_id = (item - SYNID_CLUSTER) as isize;
            msg_putchar(b'@' as i32);
            msg_outtrans(&syn_clstr(win_s(curwin_mut()), scl_id).scl_name, 0, false);
        } else {
            msg_outtrans(highlight_group_name(item as i32 - 1), 0, false);
        }
        if next_non_zero {
            msg_putchar(b',' as i32);
        }
        i += 1;
    }
    msg_putchar(b' ' as i32);
}

fn put_pattern(s: &std::ffi::CStr, c: i32, spp: &Synpat, hl_id: i32) {
    const SEPCHARS: &[u8] = b"/+=-#@\"|'^&\0";

    if syn_st().last_matchgroup != spp.sp_syn_match_id as i32 {
        syn_st().last_matchgroup = spp.sp_syn_match_id as i32;
        msg_puts_hl(c"matchgroup", hl_id, false);
        msg_putchar(b'=' as i32);
        if syn_st().last_matchgroup == 0 {
            msg_outtrans(c"NONE", 0, false);
        } else {
            msg_outtrans(highlight_group_name(syn_st().last_matchgroup - 1), 0, false);
        }
        msg_putchar(b' ' as i32);
    }

    msg_puts_hl(s, hl_id, false);
    msg_putchar(c);

    let pat_ptr = spp.sp_pattern.as_deref().unwrap_or(c"");
    let mut i: usize = 0;
    while crate::strings::vim_haschr(pat_ptr, SEPCHARS[i] as i32) {
        i += 1;
        if SEPCHARS[i] == 0 {
            i = 0;
            break;
        }
    }
    msg_putchar(SEPCHARS[i] as i32);
    msg_outtrans(pat_ptr, 0, false);
    msg_putchar(SEPCHARS[i] as i32);

    let mut first = true;
    for (j, &spo_name) in SPO_NAME_TAB.iter().enumerate().take(SPO_COUNT) {
        let mask: i16 = 1 << j;
        if (spp.sp_off_flags & (mask + (mask << SPO_COUNT))) == 0 {
            continue;
        }
        if !first {
            msg_putchar(b',' as i32);
        }
        msg_puts(spo_name);
        let n = spp.sp_offsets[j];
        if j != SPO_LC_OFF {
            if (spp.sp_off_flags & mask) != 0 {
                msg_putchar(b's' as i32);
            } else {
                msg_putchar(b'e' as i32);
            }
            if n > 0 {
                msg_putchar(b'+' as i32);
            }
        }
        if n != 0 || j == SPO_LC_OFF {
            msg_outnum(n);
        }
        first = false;
    }
    msg_putchar(b' ' as i32);
}

fn syn_list_keywords(
    id: i32,
    ht: &std::collections::BTreeMap<Vec<u8>, Vec<Keyentry>>,
    mut did_header: bool,
    hl_id: i32,
) -> bool {
    let mut prev_contained: i32 = 0;
    let mut prev_next_list: SynIdList = None;
    let mut prev_cont_in_list: SynIdList = None;
    let mut prev_skipnl: i32 = 0;
    let mut prev_skipwhite: i32 = 0;
    let mut prev_skipempty: i32 = 0;

    for entries in ht.values() {
        if g_st().got_int {
            break;
        }
        for kp in entries {
            if g_st().got_int {
                break;
            }
            if kp.k_syn.id as i32 == id {
                let mut outlen: i32 = 0;
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
                    outlen = ke2hikey(kp).to_bytes().len() as i32;
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
                    msg_putchar(b' ' as i32);
                    prev_contained = kp.flags & HL_CONTAINED;
                }
                if !syn_id_list_eq(&kp.k_syn.cont_in_list, &prev_cont_in_list) {
                    if let Some(ref list) = kp.k_syn.cont_in_list {
                        put_id_list(c"containedin", list, hl_id);
                        msg_putchar(b' ' as i32);
                    }
                    prev_cont_in_list = kp.k_syn.cont_in_list.clone();
                }
                if !syn_id_list_eq(&kp.next_list, &prev_next_list) {
                    if let Some(ref list) = kp.next_list {
                        put_id_list(c"nextgroup", list, hl_id);
                        msg_putchar(b' ' as i32);
                    }
                    prev_next_list = kp.next_list.clone();
                    if (kp.flags & HL_SKIPNL) != 0 {
                        msg_puts_hl(c"skipnl", hl_id, false);
                        msg_putchar(b' ' as i32);
                        prev_skipnl = kp.flags & HL_SKIPNL;
                    }
                    if (kp.flags & HL_SKIPWHITE) != 0 {
                        msg_puts_hl(c"skipwhite", hl_id, false);
                        msg_putchar(b' ' as i32);
                        prev_skipwhite = kp.flags & HL_SKIPWHITE;
                    }
                    if (kp.flags & HL_SKIPEMPTY) != 0 {
                        msg_puts_hl(c"skipempty", hl_id, false);
                        msg_putchar(b' ' as i32);
                        prev_skipempty = kp.flags & HL_SKIPEMPTY;
                    }
                }
                msg_outtrans(ke2hikey(kp), 0, false);
            }
        }
    }

    did_header
}

fn syn_clear_keyword(id: i32, ht: &mut std::collections::BTreeMap<Vec<u8>, Vec<Keyentry>>) {
    ht.retain(|_, entries| {
        entries.retain(|kp| kp.k_syn.id as i32 != id);
        !entries.is_empty()
    });
}

fn clear_keywtab(ht: &mut std::collections::BTreeMap<Vec<u8>, Vec<Keyentry>>) {
    ht.clear();
}

fn add_keyword(
    name: &[u8],
    id: i32,
    flags: i32,
    cont_in_list: &SynIdList,
    next_list: &SynIdList,
    conceal_char: i32,
) {
    let mut name_folded = [0u8; MAXKEYWLEN + 1];
    let kw_vec: &[u8] = if win_s(curwin_mut()).b_syn_ic {
        str_foldcase(name, Some(&mut name_folded));
        &name_folded[..crate::memory::bytes_until_nul(&name_folded).len()]
    } else {
        name
    };

    if cont_in_list.is_some() {
        win_s(curwin_mut()).b_syn_containedin = 1;
    }
    let kp = Keyentry {
        k_syn: SpSyn {
            inc_tag: syn_st().current_syn_inc_tag,
            id: id as i16,
            cont_in_list: cont_in_list.clone(),
        },
        next_list: next_list.clone(),
        flags,
        k_char: conceal_char,
        keyword: std::ffi::CString::new(kw_vec).unwrap_or_default(),
    };

    let ht = if win_s(curwin_mut()).b_syn_ic {
        &mut win_s(curwin_mut()).b_keywtab_ic
    } else {
        &mut win_s(curwin_mut()).b_keywtab
    };
    ht.entry(kw_vec.to_vec()).or_default().insert(0, kp);
}

fn get_group_name<'a>(arg: &'a CStr, name_len: &mut usize) -> Option<&'a CStr> {
    let arg_ptr = crate::strings::ByteSlice::from_cstr(arg);
    let name_end = arg_ptr.skiptowhite();
    *name_len = name_end.offset_from(arg_ptr);
    let rest = name_end.skipwhite();
    if ends_excmd(arg_ptr.first_byte()) != 0 || rest.is_empty() {
        return None;
    }
    Some(rest.as_cstr())
}

fn get_syn_options<'a>(
    arg: Option<&'a CStr>,
    opt: &mut SynOptArg,
    conceal_char: &mut i32,
    skip: bool,
) -> Option<&'a CStr> {
    let mut arg = arg?.as_ptr().cast_mut().cast::<u8>();
    {
        const FLAGTAB: &[(&[u8], i32, i32)] = &[
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
        const FIRST_LETTERS: &[u8] = b"cCoOkKeEtTsSgGdDfFnN";

        if arg.is_null() {
            return None;
        }

        if win_s(curwin_mut()).b_syn_conceal != 0 {
            opt.flags |= HL_CONCEAL;
        }

        loop {
            let arg_b = *crate::globals::GraphPtr::of(arg).borrow();
            if arg_b == 0 || !FIRST_LETTERS.contains(&arg_b) {
                break;
            }

            let mut fidx: isize = FLAGTAB.len() as isize - 1;
            let mut len: usize = 0;
            while fidx >= 0 {
                let (p, argtype, flags) = FLAGTAB[fidx as usize];
                let mut i: usize = 0;
                let arg_bytes = crate::strings::ByteSlice::from_ptr(arg).as_bytes();
                len = 0;
                while p[i] != 0 {
                    let ch = arg_bytes.get(len).copied().unwrap_or(0);
                    if ch != p[i] && ch != p[i + 1] {
                        break;
                    }
                    i += 2;
                    len += 1;
                }
                let next_ch = arg_bytes.get(len).copied().unwrap_or(0);
                if p[i] == 0
                    && (ascii_iswhite(next_ch)
                        || if argtype > 0 {
                            next_ch == b'='
                        } else {
                            ends_excmd(i32::from(next_ch)) != 0
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
                    emsg(gettext(E_CONTAINS_ARGUMENT_NOT_ACCEPTED_HERE));
                    return None;
                }
                let mut arg_cstr = crate::strings::ByteSlice::from_ptr(arg).as_cstr();
                if !get_id_list(&mut arg_cstr, 8, &mut opt.cont_list, skip) {
                    return None;
                }
                arg = arg_cstr.as_ptr().cast_mut().cast();
            } else if argtype == 2 {
                let mut arg_cstr = crate::strings::ByteSlice::from_ptr(arg).as_cstr();
                if !get_id_list(&mut arg_cstr, 11, &mut opt.cont_in_list, skip) {
                    return None;
                }
                arg = arg_cstr.as_ptr().cast_mut().cast();
            } else if argtype == 3 {
                let mut arg_cstr = crate::strings::ByteSlice::from_ptr(arg).as_cstr();
                if !get_id_list(&mut arg_cstr, 9, &mut opt.next_list, skip) {
                    return None;
                }
                arg = arg_cstr.as_ptr().cast_mut().cast();
            } else if argtype == 11
                && crate::strings::ByteSlice::from_ptr(arg).as_bytes().get(5) == Some(&b'=')
            {
                *conceal_char = utf_ptr2char(arg.wrapping_add(6));
                arg = arg.wrapping_offset((utfc_ptr2len(arg.wrapping_add(6)) - 1) as isize);
                if !vim_isprintc(*conceal_char) {
                    emsg(gettext(E_INVALID_CCHAR_VALUE));
                    return None;
                }
                arg = crate::charset::skipwhite_ptr(arg.wrapping_add(7).cast::<std::ffi::c_char>())
                    .cast();
            } else {
                opt.flags |= flags;
                arg =
                    crate::charset::skipwhite_ptr(arg.wrapping_add(len).cast::<std::ffi::c_char>())
                        .cast();

                if flags == HL_SYNC_HERE || flags == HL_SYNC_THERE {
                    if opt.sync_idx.is_none() {
                        emsg(gettext(c"E393: group[t]here not accepted here"));
                        return None;
                    }
                    let gname_start = arg;
                    arg = crate::charset::skiptowhite_ptr(arg.cast::<std::ffi::c_char>()).cast();
                    if gname_start == arg {
                        return None;
                    }
                    let gname_slice = crate::strings::AsByteSlice::slice_len(
                        gname_start,
                        (arg as usize) - (gname_start as usize),
                    );
                    let gname = std::ffi::CString::new(gname_slice).unwrap();
                    if gname.as_bytes() == b"NONE" {
                        opt.sync_idx = Some(NONE_IDX);
                    } else {
                        let syn_id = syn_name2id(&gname);
                        let mut i = win_s(curwin_mut()).b_syn_patterns.len() as i32 - 1;
                        while i >= 0 {
                            let spp = syn_items(&mut *win_s(curwin_mut()), i as isize);
                            if spp.sp_syn.id as i32 == syn_id && spp.sp_type == SPTYPE_START {
                                opt.sync_idx = Some(i);
                                break;
                            }
                            i -= 1;
                        }
                        if i < 0 {
                            semsg!(gettext(c"E394: Didn't find region item for %s"), &gname,);
                            return None;
                        }
                    }

                    arg = crate::charset::skipwhite_ptr(arg.cast::<std::ffi::c_char>()).cast();
                } else if flags == HL_FOLD && foldmethod_is_syntax(curwin_mut()) {
                    fold_update_all(curwin_mut());
                }
            }
        }

        Some(crate::strings::ByteSlice::from_ptr(arg).as_cstr())
    }
}

fn syn_incl_toplevel(id: i32, flagsp: &mut i32) {
    if (*flagsp & HL_CONTAINED) != 0 || win_s(curwin_mut()).b_syn_topgrp == 0 {
        return;
    }
    *flagsp |= HL_CONTAINED | HL_INCLUDED_TOPLEVEL;
    if win_s(curwin_mut()).b_syn_topgrp >= SYNID_CLUSTER as i32 {
        let mut grp_list: SynIdList = Some(std::rc::Rc::from([id as i16, 0]));
        let tlg_id = win_s(curwin_mut()).b_syn_topgrp - SYNID_CLUSTER as i32;
        let clp = syn_clstr(&mut *win_s(curwin_mut()), tlg_id as isize);
        syn_combine_list(&mut clp.scl_list, &mut grp_list, CLUSTER_ADD);
    }
}

fn syn_cmd_include(eap: &mut Exarg, _syncing: i32) {
    let mut arg = eap.arg;
    let mut sgl_id: i32 = 1;
    let mut errormsg = None;
    let mut source = false;

    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    if (arg).first_byte() == b'@' {
        arg = arg.wrapping_add(1);
        let mut group_name_len = 0usize;
        let Some(rest) = get_group_name((arg).as_cstr(), &mut group_name_len) else {
            emsg(gettext(c"E397: Filename required"));
            return;
        };
        sgl_id = syn_check_cluster((arg).slice(group_name_len));
        if sgl_id == 0 {
            return;
        }
        eap.arg = crate::strings::ByteSlice::from_cstr(rest);
    }

    eap.argt |= EX_XFILE | EX_NOSPC;
    separate_nextcmd(&mut *eap);
    if eap.arg.first_byte() == b'<'
        || eap.arg.first_byte() == b'$'
        || path_is_absolute(eap.arg.as_bytes())
    {
        source = true;
        if !expand_filename(eap, &mut errormsg) {
            if let Some(msg) = errormsg {
                emsg(msg);
            }
            return;
        }
    } else {
        #[cfg(windows)]
        path_to_slash(eap.arg_mut_bytes());
    }

    if syn_st().running_syn_inc_tag >= MAX_SYN_INC_TAG {
        emsg(gettext(c"E847: Too many syntax includes"));
        return;
    }
    let prev_syn_inc_tag = syn_st().current_syn_inc_tag;
    syn_st().running_syn_inc_tag += 1;
    syn_st().current_syn_inc_tag = syn_st().running_syn_inc_tag;
    let prev_toplvl_grp = win_s(curwin_mut()).b_syn_topgrp;
    win_s(curwin_mut()).b_syn_topgrp = sgl_id;
    let arg_cstr = eap.arg.as_cstr();
    let res = if source {
        do_source(arg_cstr, false, DOSO_NONE, None)
    } else {
        source_runtime(arg_cstr, DIP_ALL)
    };
    if !res {
        semsg!(gettext(E_NOTOPEN), eap.arg);
    }
    win_s(curwin_mut()).b_syn_topgrp = prev_toplvl_grp;
    syn_st().current_syn_inc_tag = prev_syn_inc_tag;
}

fn syn_cmd_keyword(eap: &mut Exarg, _syncing: i32) {
    let arg = eap.arg;
    let mut group_name_len = 0usize;
    let mut conceal_char: i32 = NUL as i32;

    let mut rest_opt = get_group_name((arg).as_cstr(), &mut group_name_len);

    if let Some(rest_initial) = rest_opt {
        let syn_id = if eap.skip {
            -1
        } else {
            syn_check_group((arg).slice(group_name_len))
        };
        if syn_id != 0 {
            let mut keyword_buf = vec![0u8; rest_initial.to_bytes().len() + 1];
            let keyword_copy = keyword_buf.as_mut_ptr();
            let mut syn_opt_arg = SynOptArg {
                flags: 0,
                keyword: true,
                sync_idx: None,
                has_cont_list: false,
                cont_list: None,
                cont_in_list: None,
                next_list: None,
            };

            let mut cnt: i32 = 0;
            let mut p = keyword_copy;
            while rest_opt
                .is_some_and(|r| ends_excmd(r.to_bytes().first().copied().unwrap_or(0)) == 0)
            {
                rest_opt = get_syn_options(rest_opt, &mut syn_opt_arg, &mut conceal_char, eap.skip);
                let Some(rest_cstr) = rest_opt else {
                    break;
                };
                let mut rest = rest_cstr.as_ptr().cast_mut();
                if ends_excmd(crate::strings::AsByteSlice::first_byte_val(rest)) != 0 {
                    break;
                }
                while *crate::globals::GraphPtr::of(rest).borrow() != 0
                    && !ascii_iswhite(*crate::globals::GraphPtr::of(rest).borrow() as u8)
                {
                    if *crate::globals::GraphPtr::of(rest).borrow() as u8 == b'\\'
                        && *crate::globals::GraphPtr::of(rest.wrapping_add(1)).borrow() != 0
                    {
                        rest = rest.wrapping_add(1);
                    }

                    *crate::globals::GraphPtr::of(p).borrow_mut() =
                        crate::strings::AsByteSlice::first_byte_val(rest);

                    p = p.wrapping_add(1);
                    rest = rest.wrapping_add(1);
                }

                *crate::globals::GraphPtr::of(p).borrow_mut() = 0;

                p = p.wrapping_add(1);
                cnt += 1;
                let next_rest = crate::charset::skipwhite_ptr(rest);
                rest_opt = Some(crate::strings::ByteSlice::from_ptr(next_rest).as_cstr());
            }

            if !eap.skip {
                syn_incl_toplevel(syn_id, &mut syn_opt_arg.flags);

                let mut kw = keyword_copy;
                let mut kwlen: usize;
                'kw_loop: while cnt > 0 {
                    cnt -= 1;
                    p = vim_strchr(
                        crate::strings::ByteSlice::from_ptr(kw).as_cstr(),
                        b'[' as i32,
                    )
                    .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
                    .cast();
                    loop {
                        if p.is_null() {
                            kwlen = crate::strings::ByteSlice::from_ptr(kw).as_bytes().len();
                        } else {
                            *crate::globals::GraphPtr::of(p).borrow_mut() = 0;

                            kwlen = ((crate::strings::AsByteSlice::byte_addr(p) as isize)
                                - (crate::strings::AsByteSlice::byte_addr(kw) as isize))
                                as usize;
                        }
                        add_keyword(
                            crate::strings::AsByteSlice::slice_len(kw, kwlen),
                            syn_id,
                            syn_opt_arg.flags,
                            &syn_opt_arg.cont_in_list,
                            &syn_opt_arg.next_list,
                            conceal_char,
                        );
                        if p.is_null() {
                            break;
                        }
                        let after_p =
                            crate::strings::ByteSlice::from_ptr(p.wrapping_add(1)).as_bytes();
                        if after_p.is_empty() {
                            semsg!(
                                gettext(c"E789: Missing ']': %s"),
                                crate::strings::ByteSlice::from_ptr(kw.cast::<std::ffi::c_void>())
                            );
                            break 'kw_loop;
                        }
                        if after_p[0] == b']' {
                            if after_p.len() > 1 {
                                semsg!(
                                    gettext(E_TRAILING_CHAR_AFTER_RSB_STR_STR),
                                    crate::strings::ByteSlice::from_ptr(
                                        kw.cast::<std::ffi::c_void>()
                                    ),
                                    crate::strings::ByteSlice::from_ptr(
                                        p.wrapping_add(2).cast::<std::ffi::c_void>()
                                    )
                                );
                                break 'kw_loop;
                            }
                            kw = p.wrapping_add(1);
                            kwlen = 1;
                            break;
                        }
                        let l = utfc_ptr2len(p.wrapping_add(1)) as usize;
                        crate::globals::GraphPtr::copy_to(p.wrapping_add(1), p, l);
                        p = p.wrapping_add(l);
                    }
                    kw = kw.wrapping_add(kwlen + 1);
                }
            }
        }
    }

    if let Some(rest_cstr) = rest_opt {
        eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(check_nextcmd(rest_cstr));
    } else {
        semsg!(gettext(E_INVARG2), arg);
    }

    redraw_curbuf_later(UPD_SOME_VALID);
    syn_stack_free_all(win_s(curwin_mut()));
}

fn syn_cmd_match(eap: &mut Exarg, syncing: i32) {
    let arg = eap.arg;
    let mut group_name_len = 0usize;
    let mut item = Synpat::new();
    let mut conceal_char: i32 = NUL as i32;

    let mut rest = get_group_name((arg).as_cstr(), &mut group_name_len);

    let mut syn_opt_arg = SynOptArg {
        flags: 0,
        keyword: false,
        sync_idx: (syncing != 0).then_some(0),
        has_cont_list: true,
        cont_list: None,
        cont_in_list: None,
        next_list: None,
    };
    rest = get_syn_options(rest, &mut syn_opt_arg, &mut conceal_char, eap.skip);

    rest = get_syn_pattern(rest, &mut item);
    if vim_regcomp_had_eol() && (syn_opt_arg.flags & HL_EXCLUDENL) == 0 {
        syn_opt_arg.flags |= HL_HAS_EOL;
    }

    rest = get_syn_options(rest, &mut syn_opt_arg, &mut conceal_char, eap.skip);

    if let Some(rest_cstr) = rest {
        eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(check_nextcmd(rest_cstr));
        if ends_excmd(rest_cstr.to_bytes().first().copied().unwrap_or(0)) == 0 || eap.skip {
            rest = None;
        } else {
            let syn_id = syn_check_group((arg).slice(group_name_len));
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
                    win_s(curwin_mut()).b_syn_containedin = 1;
                }
                item.sp_next_list = syn_opt_arg.next_list.take();
                let pats = &mut win_s(curwin_mut()).b_syn_patterns;
                pats.push(item);

                if (syn_opt_arg.flags & (HL_SYNC_HERE | HL_SYNC_THERE)) != 0 {
                    win_s(curwin_mut()).b_syn_sync_flags |= SF_MATCH;
                }
                if (syn_opt_arg.flags & HL_FOLD) != 0 {
                    win_s(curwin_mut()).b_syn_folditems += 1;
                }

                redraw_curbuf_later(UPD_SOME_VALID);
                syn_stack_free_all(win_s(curwin_mut()));
                return;
            }
        }
    }

    if rest.is_none() {
        semsg!(gettext(E_INVARG2), arg);
    }
}

fn syn_cmd_region(eap: &mut Exarg, syncing: i32) {
    let arg = eap.arg;
    let mut group_name_len = 0usize;
    let mut key: Option<std::ffi::CString> = None;
    let mut pat_ptrs: [Vec<PatPtr>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut pat_count: i32 = 0;
    let mut matchgroup_id: i32 = 0;
    let mut not_enough = false;
    let mut illegal = false;
    let mut success = false;
    let mut conceal_char: i32 = NUL as i32;

    let mut rest_opt = get_group_name((arg).as_cstr(), &mut group_name_len);

    let mut syn_opt_arg = SynOptArg {
        flags: 0,
        keyword: false,
        sync_idx: None,
        has_cont_list: true,
        cont_list: None,
        cont_in_list: None,
        next_list: None,
    };

    while rest_opt.is_some_and(|r| ends_excmd(r.to_bytes().first().copied().unwrap_or(0)) == 0) {
        rest_opt = get_syn_options(rest_opt, &mut syn_opt_arg, &mut conceal_char, eap.skip);
        let Some(rest_cstr) = rest_opt else {
            break;
        };
        let mut rest = crate::strings::ByteSlice::from_cstr(rest_cstr);
        if ends_excmd(rest.first_byte()) != 0 {
            break;
        }

        let mut key_end = rest;
        while !key_end.is_empty()
            && !ascii_iswhite(key_end.first_byte())
            && key_end.first_byte() != b'='
        {
            key_end = key_end.wrapping_add(1);
        }
        key = Some(crate::memory::bytes_to_cstring(
            &rest.slice(key_end.offset_from(rest)).to_ascii_uppercase(),
        ));
        let key_ptr = key.as_deref().unwrap_or(c"");
        let item = if key_ptr.to_bytes() == b"MATCHGROUP" {
            ITEM_MATCHGROUP
        } else if key_ptr.to_bytes() == b"START" {
            ITEM_START
        } else if key_ptr.to_bytes() == b"END" {
            ITEM_END
        } else if key_ptr.to_bytes() == b"SKIP" {
            if !pat_ptrs[ITEM_SKIP].is_empty() {
                illegal = true;
                break;
            }
            ITEM_SKIP
        } else {
            break;
        };
        rest = key_end.skipwhite();
        if rest.first_byte() != b'=' {
            rest_opt = None;
            semsg!(gettext(c"E398: Missing '=': %s"), arg);
            break;
        }
        rest = rest.wrapping_add(1).skipwhite();
        rest_opt = Some(rest.as_cstr());
        if rest.is_empty() {
            not_enough = true;
            break;
        }

        if item == ITEM_MATCHGROUP {
            let p_len = crate::charset::skiptowhite_len(rest.as_bytes());
            let p = rest.wrapping_add(p_len);
            if (p_len == 4 && rest.slice(4) == b"NONE") || eap.skip {
                matchgroup_id = 0;
            } else {
                matchgroup_id = syn_check_group(rest.slice(p_len));
                if matchgroup_id == 0 {
                    illegal = true;
                    break;
                }
            }
            rest = p.skipwhite();
            rest_opt = Some(rest.as_cstr());
        } else {
            let mut synp = Synpat::new();

            if item == ITEM_START {
                g_st().reg_do_extmatch = REX_SET;
            } else {
                g_st().reg_do_extmatch = REX_USE;
            }
            rest_opt = get_syn_pattern(rest_opt, &mut synp);
            g_st().reg_do_extmatch = 0;
            if item == ITEM_END && vim_regcomp_had_eol() && (syn_opt_arg.flags & HL_EXCLUDENL) == 0
            {
                synp.sp_flags |= HL_HAS_EOL;
            }
            pat_ptrs[item].insert(
                0,
                PatPtr {
                    pp_synp: synp,
                    pp_matchgroup_id: matchgroup_id,
                },
            );
            pat_count += 1;
        }
    }
    drop(key);
    let mut rest = crate::strings::ByteSlice::from_opt_cstr(rest_opt);
    if illegal || not_enough {
        rest = crate::strings::BYTE_SLICE_NULL;
    }

    if !rest.is_null() && (pat_ptrs[ITEM_START].is_empty() || pat_ptrs[ITEM_END].is_empty()) {
        not_enough = true;
        rest = crate::strings::BYTE_SLICE_NULL;
    }

    if !rest.is_null() {
        eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(check_nextcmd(rest.as_cstr()));
        if ends_excmd(rest.first_byte()) == 0 || eap.skip {
            rest = crate::strings::BYTE_SLICE_NULL;
        } else {
            let syn_id = syn_check_group((arg).slice(group_name_len));
            if syn_id != 0 {
                syn_incl_toplevel(syn_id, &mut syn_opt_arg.flags);
                let pats = &mut win_s(curwin_mut()).b_syn_patterns;
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
                                win_s(curwin_mut()).b_syn_containedin = 1;
                            }
                            pat.sp_next_list = syn_opt_arg.next_list.clone();
                        }
                        pats.push(pat);
                        if (syn_opt_arg.flags & HL_FOLD) != 0 {
                            win_s(curwin_mut()).b_syn_folditems += 1;
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
            semsg!(gettext(E_INVARG2), arg);
        }
    }
}

fn syn_combine_list(clstr1: &mut SynIdList, clstr2: &mut SynIdList, list_op: i32) {
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

fn syn_scl_name2id(name: &[u8]) -> i32 {
    let name_u = name.to_ascii_uppercase();
    let mut i = win_s(curwin_mut()).b_syn_clusters.len() as i32 - 1;
    while i >= 0 {
        let clp = syn_clstr(&mut *win_s(curwin_mut()), i as isize);
        if name_u.as_slice() == clp.scl_name_u.as_bytes() {
            break;
        }
        i -= 1;
    }
    if i < 0 { 0 } else { i + SYNID_CLUSTER as i32 }
}

fn syn_scl_namen2id(bytes: &[u8]) -> i32 {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    syn_scl_name2id(&bytes[..end])
}

fn syn_check_cluster(bytes: &[u8]) -> i32 {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut id = syn_scl_name2id(&bytes[..end]);
    if id == 0 {
        let name = std::ffi::CString::new(&bytes[..end]).unwrap();
        id = syn_add_cluster(name);
    }
    id
}

fn syn_add_cluster(name: std::ffi::CString) -> i32 {
    let len = win_s(curwin_mut()).b_syn_clusters.len() as i32;
    if len >= MAX_CLUSTER_ID {
        emsg(gettext(c"E848: Too many syntax clusters"));
        return 0;
    }

    let name_ptr = crate::strings::ByteSlice::from_cstr(&name);
    let is_spell = name_ptr.as_bytes().eq_ignore_ascii_case(b"Spell");
    let is_nospell = name_ptr.as_bytes().eq_ignore_ascii_case(b"NoSpell");
    let scl_name_u = crate::memory::bytes_to_cstring(&name_ptr.as_bytes().to_ascii_uppercase());

    let clusters = &mut win_s(curwin_mut()).b_syn_clusters;
    clusters.push(SynCluster {
        scl_name: name,
        scl_name_u,
        scl_list: None,
    });

    if is_spell {
        win_s(curwin_mut()).b_spell_cluster_id = len + SYNID_CLUSTER as i32;
    }
    if is_nospell {
        win_s(curwin_mut()).b_nospell_cluster_id = len + SYNID_CLUSTER as i32;
    }

    len + SYNID_CLUSTER as i32
}

fn syn_cmd_cluster(eap: &mut Exarg, _syncing: i32) {
    let arg = eap.arg;
    let mut group_name_len = 0usize;
    let mut got_clstr = false;

    eap.nextcmd = crate::strings::ByteSlice::from_opt_cstr(find_nextcmd((arg).as_cstr()));
    if eap.skip {
        return;
    }

    let mut rest_opt = get_group_name((arg).as_cstr(), &mut group_name_len);

    if let Some(mut rest_cstr) = rest_opt {
        let mut scl_id = syn_check_cluster((arg).slice(group_name_len));
        if scl_id == 0 {
            return;
        }
        scl_id -= SYNID_CLUSTER as i32;

        loop {
            let rest_bytes = rest_cstr.to_bytes();
            let is_sep = |idx: usize| {
                let b = rest_bytes.get(idx).copied().unwrap_or(0);
                ascii_iswhite(b) || b == b'='
            };
            let (opt_len, list_op) = if rest_bytes.len() >= 3
                && rest_bytes[..3].eq_ignore_ascii_case(b"add")
                && is_sep(3)
            {
                (3, CLUSTER_ADD)
            } else if rest_bytes.len() >= 6
                && rest_bytes[..6].eq_ignore_ascii_case(b"remove")
                && is_sep(6)
            {
                (6, CLUSTER_SUBTRACT)
            } else if rest_bytes.len() >= 8
                && rest_bytes[..8].eq_ignore_ascii_case(b"contains")
                && is_sep(8)
            {
                (8, CLUSTER_REPLACE)
            } else {
                break;
            };

            let mut clstr_list: SynIdList = None;
            if !get_id_list(&mut rest_cstr, opt_len, &mut clstr_list, eap.skip) {
                semsg!(gettext(E_INVARG2), rest_cstr);
                break;
            }
            if scl_id >= 0 {
                let clp = syn_clstr(&mut *win_s(curwin_mut()), scl_id as isize);
                syn_combine_list(&mut clp.scl_list, &mut clstr_list, list_op);
            }
            got_clstr = true;
        }
        rest_opt = Some(rest_cstr);

        if got_clstr {
            redraw_curbuf_later(UPD_SOME_VALID);
            syn_stack_free_all(win_s(curwin_mut()));
        }
    }

    if !got_clstr {
        emsg(gettext(c"E400: No cluster specified"));
    }
    if rest_opt.is_none_or(|r| ends_excmd(r.to_bytes().first().copied().unwrap_or(0)) == 0) {
        semsg!(gettext(E_INVARG2), arg);
    }
}

fn get_syn_pattern<'a>(arg: Option<&'a CStr>, ci: &mut Synpat) -> Option<&'a CStr> {
    let arg_cstr = arg?;
    let arg = crate::strings::ByteSlice::from_cstr(arg_cstr);
    let arg_bytes = arg_cstr.to_bytes();
    {
        if arg_bytes.len() < 3 {
            return None;
        }

        let delim = arg_bytes[0];
        let mut end = skip_regexp(arg.wrapping_add(1).as_cstr(), delim as i32, 1)
            .as_ptr()
            .cast_mut()
            .cast::<u8>();
        if *crate::globals::GraphPtr::of(end).borrow() != delim {
            semsg!(
                gettext(c"E401: Pattern delimiter not found: %s"),
                crate::strings::ByteSlice::from_ptr(arg.cast::<std::ffi::c_void>()),
            );
            return None;
        }
        let pat_slice = arg.wrapping_add(1).slice((end as usize) - arg.addr() - 1);
        let pat_cstr = std::ffi::CString::new(pat_slice).unwrap();

        let cpo_save = std::mem::take(&mut opt_vars().p_cpo);
        ci.sp_prog = crate::regexp::vim_regcomp_box(&pat_cstr, RE_MAGIC);
        opt_vars().p_cpo = cpo_save;
        ci.sp_pattern = Some(pat_cstr);

        ci.sp_prog.as_ref()?;
        ci.sp_ic = win_s(curwin_mut()).b_syn_ic;
        syn_clear_time(&mut ci.sp_time);

        end = end.wrapping_add(1);
        loop {
            let end_bytes = crate::strings::ByteSlice::from_ptr(end).as_bytes();
            let mut idx: isize = SPO_COUNT as isize - 1;
            while idx >= 0 {
                if end_bytes.starts_with(SPO_NAME_TAB[idx as usize].to_bytes()) {
                    break;
                }
                idx -= 1;
            }
            if idx >= 0 {
                let base_idx = idx as usize;
                if base_idx != SPO_LC_OFF {
                    match end_bytes.get(3).copied().unwrap_or(0) {
                        b's' | b'b' => {}
                        b'e' => idx += SPO_COUNT as isize,
                        _ => idx = -1,
                    }
                }
                if idx >= 0 {
                    ci.sp_off_flags |= 1 << (idx as usize);
                    if base_idx == SPO_LC_OFF {
                        end = end.wrapping_add(3);
                        let mut rem = &end_bytes[3..];
                        let val = getdigits_int(&mut rem, true, 0);
                        end = end.wrapping_add((end_bytes.len() - 3) - rem.len());
                        ci.sp_offsets[base_idx] = val;

                        if (ci.sp_off_flags & (1 << SPO_MS_OFF)) == 0 {
                            ci.sp_off_flags |= 1 << SPO_MS_OFF;
                            ci.sp_offsets[SPO_MS_OFF] = val;
                        }
                    } else {
                        end = end.wrapping_add(4);
                        let mut rem = &end_bytes[4..];
                        if let Some(after_plus) = rem.strip_prefix(b"+") {
                            rem = after_plus;
                            ci.sp_offsets[base_idx] = getdigits_int(&mut rem, true, 0);
                            end = end.wrapping_add((end_bytes.len() - 4) - rem.len());
                        } else if let Some(after_minus) = rem.strip_prefix(b"-") {
                            rem = after_minus;
                            ci.sp_offsets[base_idx] = -getdigits_int(&mut rem, true, 0);
                            end = end.wrapping_add((end_bytes.len() - 4) - rem.len());
                        }
                    }
                    if *crate::globals::GraphPtr::of(end).borrow() != b',' {
                        break;
                    }
                    end = end.wrapping_add(1);
                }
            }
            if idx < 0 {
                break;
            }
        }

        if ends_excmd(*crate::globals::GraphPtr::of(end).borrow()) == 0
            && !ascii_iswhite(*crate::globals::GraphPtr::of(end).borrow())
        {
            semsg!(
                gettext(c"E402: Garbage after pattern: %s"),
                crate::strings::ByteSlice::from_ptr(arg.cast::<std::ffi::c_void>())
            );
            return None;
        }
        Some(
            crate::strings::ByteSlice::from_ptr(crate::charset::skipwhite_ptr(
                end.cast::<std::ffi::c_char>(),
            ))
            .as_cstr(),
        )
    }
}

fn syn_cmd_sync(eap: &mut Exarg, _syncing: i32) {
    let mut arg_start = eap.arg;
    let mut key_owned: Option<std::ffi::CString>;
    let mut illegal = false;
    let mut finished = false;

    if ends_excmd((arg_start).first_byte()) != 0 {
        syn_cmd_list(eap, 1);
        return;
    }

    while ends_excmd((arg_start).first_byte()) == 0 {
        let mut arg_end = arg_start.skiptowhite();
        let mut next_arg = arg_end.skipwhite();
        key_owned = Some(crate::memory::bytes_to_cstring(
            &arg_start
                .slice(((arg_end).offset_from(arg_start) as isize) as usize)
                .to_ascii_uppercase(),
        ));
        let key = key_owned.as_deref().unwrap_or(c"");
        if key.to_bytes() == b"CCOMMENT" {
            if !eap.skip {
                win_s(curwin_mut()).b_syn_sync_flags |= SF_CCOMMENT;
            }
            if ends_excmd((next_arg).first_byte()) == 0 {
                arg_end = next_arg.skiptowhite();
                if !eap.skip {
                    win_s(curwin_mut()).b_syn_sync_id = syn_check_group(
                        next_arg.slice(((arg_end).offset_from(next_arg) as isize) as usize),
                    ) as i16;
                }
                next_arg = arg_end.skipwhite();
            } else if !eap.skip {
                win_s(curwin_mut()).b_syn_sync_id = syn_name2id(c"Comment") as i16;
            }
        } else if key_owned.as_ref().is_some_and(|k| {
            let kb = k.to_bytes();
            kb.starts_with(b"LINES")
                || kb.starts_with(b"MINLINES")
                || kb.starts_with(b"MAXLINES")
                || kb.starts_with(b"LINEBREAKS")
        }) {
            let key_bytes = key_owned.as_ref().unwrap().to_bytes();
            let val_off = if key_bytes.get(4) == Some(&b'S') {
                6
            } else if key_bytes[0] == b'L' {
                11
            } else {
                9
            };
            if key_bytes.get(val_off - 1) != Some(&b'=')
                || !key_bytes.get(val_off).is_some_and(u8::is_ascii_digit)
            {
                illegal = true;
                break;
            }
            let mut rem = &key_bytes[val_off..];
            let n: Linenr = getdigits_int32(&mut rem, false, 0);
            if !eap.skip {
                if key_bytes.get(4) == Some(&b'B') {
                    win_s(curwin_mut()).b_syn_sync_linebreaks = n;
                } else if key_bytes.get(1) == Some(&b'A') {
                    win_s(curwin_mut()).b_syn_sync_maxlines = n;
                } else {
                    win_s(curwin_mut()).b_syn_sync_minlines = n;
                }
            }
        } else if key.to_bytes() == b"FROMSTART" {
            if !eap.skip {
                win_s(curwin_mut()).b_syn_sync_minlines = MAXLNUM;
                win_s(curwin_mut()).b_syn_sync_maxlines = 0;
            }
        } else if key.to_bytes() == b"LINECONT" {
            if (next_arg).is_empty() {
                illegal = true;
                break;
            }
            if win_s(curwin_mut()).b_syn_linecont_pat.is_some() {
                emsg(gettext(
                    c"E403: syntax sync: line continuations pattern specified twice",
                ));
                finished = true;
                break;
            }
            arg_end = crate::strings::ByteSlice::from_cstr(skip_regexp(
                next_arg.wrapping_add(1).as_cstr(),
                i32::from((next_arg).first_byte()),
                1,
            ));
            if (arg_end).first_byte() != (next_arg).first_byte() {
                illegal = true;
                break;
            }

            if !eap.skip {
                let pat_slice = next_arg
                    .wrapping_add(1)
                    .slice(((arg_end).offset_from(next_arg) as isize) as usize - 1);
                let pat_cstr = std::ffi::CString::new(pat_slice).unwrap();
                win_s(curwin_mut()).b_syn_linecont_ic = win_s(curwin_mut()).b_syn_ic;

                let cpo_save = std::mem::take(&mut opt_vars().p_cpo);
                win_s(curwin_mut()).b_syn_linecont_prog =
                    crate::regexp::vim_regcomp_box(&pat_cstr, RE_MAGIC);
                opt_vars().p_cpo = cpo_save;
                syn_clear_time(&mut win_s(curwin_mut()).b_syn_linecont_time);

                if win_s(curwin_mut()).b_syn_linecont_prog.is_none() {
                    win_s(curwin_mut()).b_syn_linecont_pat = None;
                    finished = true;
                    break;
                }
                win_s(curwin_mut()).b_syn_linecont_pat = Some(pat_cstr);
            }
            next_arg = arg_end.wrapping_add(1).skipwhite();
        } else {
            eap.arg = next_arg;
            if key.to_bytes() == b"MATCH" {
                syn_cmd_match(eap, 1);
            } else if key.to_bytes() == b"REGION" {
                syn_cmd_region(eap, 1);
            } else if key.to_bytes() == b"CLEAR" {
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
        eap.nextcmd =
            crate::strings::ByteSlice::from_opt_cstr(check_nextcmd((arg_start).as_cstr()));
        redraw_curbuf_later(UPD_SOME_VALID);
        syn_stack_free_all(win_s(curwin_mut()));
    }
}

fn get_id_list(arg: &mut &CStr, keylen: i32, list: &mut SynIdList, skip: bool) -> bool {
    let arg_ptr = arg.as_ptr().cast_mut().cast::<u8>();
    {
        let mut p = crate::globals::GraphPtr::<u8>::NULL.as_ptr();
        let mut total_count: i32 = 0;
        let mut retval: Vec<i16> = Vec::new();
        let mut regmatch: Regmatch = Regmatch::new();
        let mut failed = false;

        let mut round: i32 = 1;
        while round <= 2 {
            p = crate::charset::skipwhite_ptr(
                arg_ptr
                    .wrapping_offset(keylen as isize)
                    .cast::<std::ffi::c_char>(),
            )
            .cast();
            if *crate::globals::GraphPtr::of(p).borrow() != b'=' {
                semsg!(
                    gettext(c"E405: Missing equal sign: %s"),
                    crate::strings::ByteSlice::from_ptr(arg_ptr.cast::<std::ffi::c_void>())
                );
                break;
            }
            p = crate::charset::skipwhite_ptr(p.wrapping_add(1).cast::<std::ffi::c_char>()).cast();
            if ends_excmd(*crate::globals::GraphPtr::of(p).borrow()) != 0 {
                semsg!(
                    gettext(c"E406: Empty argument: %s"),
                    crate::strings::ByteSlice::from_ptr(arg_ptr.cast::<std::ffi::c_void>())
                );
                break;
            }

            let mut count: i32 = 0;
            loop {
                let mut end = p;
                while *crate::globals::GraphPtr::of(end).borrow() != 0
                    && !ascii_iswhite(*crate::globals::GraphPtr::of(end).borrow())
                    && *crate::globals::GraphPtr::of(end).borrow() != b','
                {
                    end = end.wrapping_add(1);
                }
                let item_len = ((crate::strings::AsByteSlice::byte_addr(end) as isize)
                    - (crate::strings::AsByteSlice::byte_addr(p) as isize))
                    as usize;
                let mut name_buf = vec![0u8; item_len + 3];
                name_buf[1..=item_len]
                    .copy_from_slice(crate::strings::AsByteSlice::slice_len(p, item_len));
                let name = crate::strings::ByteSlice::from_bytes(&name_buf);
                let mut id: i32;
                if (name.wrapping_add(1).as_bytes() == b"ALLBUT")
                    || (name.wrapping_add(1).as_bytes() == b"ALL")
                    || (name.wrapping_add(1).as_bytes() == b"TOP")
                    || (name.wrapping_add(1).as_bytes() == b"CONTAINED")
                {
                    if !crate::globals::GraphPtr::of(arg_ptr)
                        .borrow()
                        .eq_ignore_ascii_case(&b'C')
                    {
                        semsg!(
                            gettext(c"E407: %s not allowed here"),
                            crate::strings::ByteSlice::from_ptr(
                                name.wrapping_add(1).cast::<std::ffi::c_void>()
                            )
                        );
                        failed = true;
                        break;
                    }
                    if count != 0 {
                        semsg!(
                            gettext(c"E408: %s must be first in contains list"),
                            crate::strings::ByteSlice::from_ptr(
                                name.wrapping_add(1).cast::<std::ffi::c_void>()
                            ),
                        );
                        failed = true;
                        break;
                    }
                    id = if name_buf[1] == b'A' {
                        SYNID_ALLBUT as i32
                    } else if name_buf[1] == b'T' {
                        SYNID_TOP as i32
                    } else {
                        SYNID_CONTAINED as i32
                    };
                    id += syn_st().current_syn_inc_tag;
                } else if name_buf[1] == b'@' {
                    if skip {
                        id = -1;
                    } else {
                        id = syn_check_cluster(
                            name.wrapping_add(2)
                                .slice(((end as usize) - (p as usize)).max(1) - 1),
                        );
                    }
                } else if !name
                    .wrapping_add(1)
                    .as_bytes()
                    .iter()
                    .any(|b| b"\\.*^$~[".contains(b))
                {
                    id = syn_check_group(name.wrapping_add(1).slice((end as usize) - (p as usize)));
                } else {
                    let item_len = ((crate::strings::AsByteSlice::byte_addr(end) as isize)
                        - (crate::strings::AsByteSlice::byte_addr(p) as isize))
                        as usize;
                    name_buf[0] = b'^';
                    name_buf[1 + item_len] = b'$';
                    name_buf[2 + item_len] = 0;
                    regmatch.set_prog(vim_regcomp(name.as_cstr(), RE_MAGIC));
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
                    regmatch.free_prog();
                }
                if id == 0 {
                    semsg!(
                        gettext(c"E409: Unknown group name: %s"),
                        crate::strings::ByteSlice::from_ptr(p.cast::<std::ffi::c_void>())
                    );
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
                p = crate::charset::skipwhite_ptr(end.cast::<std::ffi::c_char>()).cast();
                if *crate::globals::GraphPtr::of(p).borrow() != b',' {
                    break;
                }
                p = crate::charset::skipwhite_ptr(p.wrapping_add(1).cast::<std::ffi::c_char>())
                    .cast();
                if ends_excmd(*crate::globals::GraphPtr::of(p).borrow()) != 0 {
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

        *arg = crate::strings::ByteSlice::from_ptr(p).as_cstr();
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

                let scl = &(*syn_clstr(syn_block_mut(), (item - SYNID_CLUSTER) as isize));
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

fn idl_get_entry(list: &SynIdList) -> &'static mut IdlEntry {
    let cache = syn_block_mut()
        .b_idlist_cache
        .get_or_insert_with(|| Box::new(IdlCache::default()));
    let last = cache.last as usize;
    if syn_id_list_eq(&cache.ent[last].idl_key, list) {
        return &mut cache.ent[last];
    }
    for i in 0..IDL_CACHE_SIZE {
        if syn_id_list_eq(&cache.ent[i].idl_key, list) {
            cache.last = i as i32;
            return &mut cache.ent[i];
        }
    }

    let round = cache.round as usize;
    cache.last = cache.round;
    let e = &mut cache.ent[round];
    cache.round = (cache.round + 1) % (IDL_CACHE_SIZE as i32);
    e.idl_ids.clear();
    e.idl_key = list.clone();
    e.idl_marker = 0;
    e.idl_usable = true;

    let mut slice: &[i16] = list.as_deref().unwrap_or(&[]);
    if let Some(&first) = slice.first()
        && (SYNID_ALLBUT..SYNID_CLUSTER).contains(&first)
    {
        e.idl_marker = first;
        slice = &slice[1..];
    }

    let mut ga: Vec<i16> = Vec::with_capacity(50);
    let mut seen: Vec<i16> = Vec::with_capacity(10);
    if !idl_collect(slice, &mut ga, &mut seen) {
        e.idl_usable = false;
        return e;
    }
    if !ga.is_empty() {
        ga.sort_unstable();
        e.idl_ids = ga;
    }
    e
}

fn idl_contains(e: &IdlEntry, id: i16) -> bool {
    e.idl_ids.binary_search(&id).is_ok()
}

fn idl_cache_clear(block: &mut Synblock) {
    block.b_idlist_cache = None;
}

fn in_id_list(cur_si: Option<&Stateitem>, list: &SynIdList, ssp: &mut SpSyn, flags: i32) -> i32 {
    let id = ssp.id;

    if let Some(cur_si_ref) = cur_si
        && ssp.cont_in_list.is_some()
        && (cur_si_ref.si_flags & HL_MATCH) == 0
    {
        let mut cur_si_ptr = crate::globals::GraphPtr::<Stateitem>::of(cur_si_ref).as_ptr();
        let base_ptr = crate::globals::GraphPtr::<Stateitem>::of(cur_state_at(0)).as_ptr();
        while (crate::globals::GraphPtr::of(cur_si_ptr).borrow().si_flags & HL_TRANS_CONT) != 0
            && cur_si_ptr > base_ptr
        {
            cur_si_ptr = cur_si_ptr.wrapping_sub(1);
        }
        let si_idx = crate::globals::GraphPtr::of(cur_si_ptr).borrow().si_idx;
        if si_idx >= 0 {
            let c_spp = syn_items(syn_block_mut(), si_idx as isize);
            if in_id_list(None, &ssp.cont_in_list, &mut c_spp.sp_syn, c_spp.sp_flags) != 0 {
                return 1;
            }
        }
    }

    let Some(ref list_rc) = *list else {
        return 0;
    };

    if is_id_list_all(list) {
        return i32::from((flags & HL_CONTAINED) == 0);
    }

    let toplevel = (flags & HL_CONTAINED) == 0 || (flags & HL_INCLUDED_TOPLEVEL) != 0;

    let e = idl_get_entry(list);
    if e.idl_usable {
        let retval = if e.idl_marker != 0 {
            let item = e.idl_marker;
            if item < SYNID_TOP {
                if (item - SYNID_ALLBUT) as i32 != ssp.inc_tag {
                    return 0;
                }
            } else if item < SYNID_CONTAINED {
                if (item - SYNID_TOP) as i32 != ssp.inc_tag || !toplevel {
                    return 0;
                }
            } else if (item - SYNID_CONTAINED) as i32 != ssp.inc_tag || toplevel {
                return 0;
            }
            false
        } else {
            true
        };
        return i32::from(if idl_contains(e, id) { retval } else { !retval });
    }

    let mut slice: &[i16] = list_rc;
    let first = slice.first().copied().unwrap_or(0);
    let retval = if (SYNID_ALLBUT..SYNID_CLUSTER).contains(&first) {
        if first < SYNID_TOP {
            if (first - SYNID_ALLBUT) as i32 != ssp.inc_tag {
                return 0;
            }
        } else if first < SYNID_CONTAINED {
            if (first - SYNID_TOP) as i32 != ssp.inc_tag || !toplevel {
                return 0;
            }
        } else if (first - SYNID_CONTAINED) as i32 != ssp.inc_tag || toplevel {
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
            return i32::from(retval);
        }
        if item >= SYNID_CLUSTER {
            let scl_list = &syn_clstr(syn_block_mut(), (item - SYNID_CLUSTER) as isize).scl_list;
            if scl_list.is_some() && syn_st().in_id_list_depth < 30 {
                syn_st().in_id_list_depth += 1;
                let r = in_id_list(None, scl_list, &mut *ssp, flags);
                syn_st().in_id_list_depth -= 1;
                if r != 0 {
                    return i32::from(retval);
                }
            }
        }
    }
    i32::from(!retval)
}

type SubcmdFn = fn(&mut Exarg, i32);

const SUBCOMMANDS: [(&std::ffi::CStr, SubcmdFn); 19] = [
    (c"case", syn_cmd_case),
    (c"clear", syn_cmd_clear),
    (c"cluster", syn_cmd_cluster),
    (c"conceal", syn_cmd_conceal),
    (c"enable", syn_cmd_on),
    (c"foldlevel", syn_cmd_foldlevel),
    (c"include", syn_cmd_include),
    (c"iskeyword", syn_cmd_iskeyword),
    (c"keyword", syn_cmd_keyword),
    (c"list", syn_cmd_list),
    (c"manual", syn_cmd_manual),
    (c"match", syn_cmd_match),
    (c"on", syn_cmd_on),
    (c"off", syn_cmd_off),
    (c"region", syn_cmd_region),
    (c"reset", syn_cmd_reset),
    (c"spell", syn_cmd_spell),
    (c"sync", syn_cmd_sync),
    (c"", syn_cmd_list),
];

/// ":syntax".
pub fn ex_syntax(eap: &mut Exarg) {
    let arg = eap.arg;

    let mut subcmd_end = arg;
    while ascii_isalpha((subcmd_end).first_byte()) {
        subcmd_end = subcmd_end.wrapping_add(1);
    }
    let subcmd_slice = arg.slice(((subcmd_end).offset_from(arg) as isize) as usize);
    let subcmd_name = std::ffi::CString::new(subcmd_slice).unwrap();
    if eap.skip {
        g_st().emsg_skip += 1;
    }
    let mut found = false;
    for &(name, func) in &SUBCOMMANDS {
        if subcmd_slice == name.to_bytes() {
            eap.arg = subcmd_end.skipwhite();
            func(eap, 0);
            found = true;
            break;
        }
    }

    if !found {
        semsg!(
            gettext(c"E410: Invalid :syntax subcommand: %s"),
            &subcmd_name,
        );
    }

    if eap.skip {
        g_st().emsg_skip -= 1;
    }
}

/// @deprecated
pub fn ex_ownsyntax(eap: &mut Exarg) {
    if curwin_mut().w_s_owned.is_none() {
        let mut new_s = Box::new(Synblock::new());
        new_s.b_p_spc.clear_option_str();
        new_s.b_p_spf.clear_option_str();
        new_s.b_p_spl.clear_option_str();
        new_s.b_p_spo.clear_option_str();
        new_s.b_syn_isk = None;
        curwin_mut().w_s_owned = Some(new_s);
        curwin_mut().w_onebuf_opt.wo_spell = false;
    }

    let old_value = get_var_value(c"b:current_syntax").map(ToOwned::to_owned);

    apply_autocmds(
        EVENT_SYNTAX,
        eap.arg.as_opt_cstr(),
        curbuf_mut().fname_opt(),
        true,
        curbuf_opt_mut(),
    );

    if let Some(new_value) = get_var_value(c"b:current_syntax") {
        set_internal_string_var(c"w:current_syntax", new_value);
    }

    if let Some(ref old_cstr) = old_value {
        set_internal_string_var(c"b:current_syntax", old_cstr);
    } else {
        do_unlet(b"b:current_syntax", true);
    }
}

pub fn syntax_present(win: &mut WinInfoInfo) -> bool {
    !win_s(win).b_syn_patterns.is_empty()
        || !win_s(win).b_syn_clusters.is_empty()
        || !win_s(win).b_keywtab.is_empty()
        || !win_s(win).b_keywtab_ic.is_empty()
}

pub fn reset_expand_highlight() {
    g_st().include_none = 0;
    g_st().include_default = 0;
    g_st().include_link = 0;
}

pub fn set_context_in_echohl_cmd(xp: &mut ExpandCtx, arg: &CStr) {
    xp.xp_context = EXPAND_HIGHLIGHT;
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg);
    g_st().include_none = 1;
}

pub fn set_context_in_syntax_cmd(xp: &mut ExpandCtx, arg: &CStr) {
    let arg_ptr = crate::strings::OwnedSlice::from_cstr(arg);
    xp.xp_context = EXPAND_SYNTAX;
    syn_st().expand_what = EXP_SUBCMD;
    xp.xp_pattern = arg_ptr.clone();
    g_st().include_link = 0;
    g_st().include_default = 0;

    let bytes = arg.to_bytes();
    if bytes.is_empty() {
        return;
    }

    let p_rem = crate::charset::skiptowhite_slice(bytes);
    if p_rem.is_empty() {
        return;
    }

    let pat_rem = crate::charset::skipwhite_slice(p_rem);
    xp.xp_pattern = arg_ptr.wrapping_add(bytes.len() - pat_rem.len());
    if !crate::charset::skiptowhite_slice(pat_rem).is_empty() {
        xp.xp_context = EXPAND_NOTHING;
    } else {
        let subcmd = &bytes[..bytes.len() - p_rem.len()];
        let prefix_ic = |s: &[u8], target: &[u8]| {
            s.len() <= target.len() && s.eq_ignore_ascii_case(&target[..s.len()])
        };
        if prefix_ic(subcmd, b"case") {
            syn_st().expand_what = EXP_CASE;
        } else if prefix_ic(subcmd, b"spell") {
            syn_st().expand_what = EXP_SPELL;
        } else if prefix_ic(subcmd, b"sync") {
            syn_st().expand_what = EXP_SYNC;
        } else if prefix_ic(subcmd, b"list") {
            if pat_rem.first() == Some(&b'@') {
                syn_st().expand_what = EXP_CLUSTER;
            } else {
                xp.xp_context = EXPAND_HIGHLIGHT;
            }
        } else if prefix_ic(subcmd, b"keyword")
            || prefix_ic(subcmd, b"region")
            || prefix_ic(subcmd, b"match")
        {
            xp.xp_context = EXPAND_HIGHLIGHT;
        } else {
            xp.xp_context = EXPAND_NOTHING;
        }
    }
}

pub fn get_syntax_name(xp: &mut ExpandCtx, idx: i32) -> Option<&std::ffi::CStr> {
    if idx < 0 {
        return None;
    }
    let u = idx as usize;
    match syn_st().expand_what {
        EXP_SUBCMD => SUBCOMMANDS.get(u).map(|&(name, _)| name),
        EXP_CASE => {
            const CASE_ARGS: [&std::ffi::CStr; 2] = [c"match", c"ignore"];
            CASE_ARGS.get(u).copied()
        }
        EXP_SPELL => {
            const SPELL_ARGS: [&std::ffi::CStr; 3] = [c"toplevel", c"notoplevel", c"default"];
            SPELL_ARGS.get(u).copied()
        }
        EXP_SYNC => {
            const SYNC_ARGS: [&std::ffi::CStr; 10] = [
                c"ccomment",
                c"clear",
                c"fromstart",
                c"linebreaks=",
                c"linecont",
                c"lines=",
                c"match",
                c"maxlines=",
                c"minlines=",
                c"region",
            ];
            SYNC_ARGS.get(u).copied()
        }
        EXP_CLUSTER => {
            if (idx as usize) < win_s(curwin_mut()).b_syn_clusters.len() {
                format_to_buf!(
                    &mut xp.xp_buf[..],
                    c"@%s",
                    crate::strings::ByteSlice::from_ptr(
                        syn_clstr(&mut *win_s(curwin_mut()), idx as isize)
                            .scl_name
                            .as_ptr()
                    ),
                );
                Some(crate::memory::bytes_as_cstr(&xp.xp_buf))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Function called for expression evaluation: get syntax ID at file position.
pub fn syn_get_id(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    col: Colnr,
    trans: i32,
    spellp: Option<&mut bool>,
    keep_state: i32,
) -> i32 {
    if syn_st().syn_win != wp
        || syn_st().syn_buf != wp.w_buffer
        || lnum != syn_st().current_lnum
        || col < syn_st().current_col
    {
        syntax_start(wp, lnum);
    } else if col > syn_st().current_col {
        syn_st().next_match_idx = -1;
    }

    get_syntax_attr(col, spellp, keep_state != 0);

    if trans != 0 {
        syn_st().current_trans_id
    } else {
        syn_st().current_id
    }
}

pub fn get_syntax_info(seqnrp: &mut i32) -> i32 {
    *seqnrp = syn_st().current_seqnr;
    syn_st().current_flags
}

pub fn syn_get_sub_char() -> i32 {
    syn_st().current_sub_char
}

pub fn syn_get_stack_item(i: i32) -> i32 {
    if i >= cur_state_len() {
        invalidate_current_state();
        syn_st().current_col = MAXCOL;
        return -1;
    }
    cur_state_at(i).si_id
}

fn syn_cur_foldlevel() -> i32 {
    let mut level: i32 = 0;
    for i in 0..cur_state_len() {
        if (cur_state_at(i).si_flags & HL_FOLD) != 0 {
            level += 1;
        }
    }
    level
}

pub fn syn_get_foldlevel(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    let mut level: i32 = 0;

    if win_s(&mut *wp).b_syn_folditems != 0
        && !win_s(&mut *wp).b_syn_error
        && !win_s(&mut *wp).b_syn_slow
    {
        syntax_start(wp, lnum);

        level = syn_cur_foldlevel();

        if win_s(&mut *wp).b_syn_foldlevel == SYNFLD_MINIMUM {
            let mut cur_level = level;
            let mut low_level = cur_level;
            while !syn_st().current_finished {
                syn_current_attr(false, false, None, false);
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
        level = wp.w_onebuf_opt.wo_fdn as i32;
        if level < 0 {
            level = 0;
        }
    }
    level
}

/// ":syntime".
pub fn ex_syntime(eap: &mut Exarg) {
    if eap.arg.as_bytes() == b"on" {
        syn_st().syn_time_on = true;
    } else if eap.arg.as_bytes() == b"off" {
        syn_st().syn_time_on = false;
    } else if eap.arg.as_bytes() == b"clear" {
        syntime_clear();
    } else if eap.arg.as_bytes() == b"report" {
        syntime_report();
    } else {
        semsg!(gettext(E_INVARG2), eap.arg);
    }
}

fn syn_clear_time(st: &mut SynTime) {
    st.total = profile_zero();
    st.slowest = profile_zero();
    st.count = 0;
    st.match_ = 0;
}

fn syntime_clear() {
    if !syntax_present(curwin_mut()) {
        msg(gettext(MSG_NO_ITEMS), 0);
        return;
    }
    for pat in &mut win_s(curwin_mut()).b_syn_patterns {
        syn_clear_time(&mut pat.sp_time);
    }
}

pub fn get_syntime_arg(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static std::ffi::CStr> {
    match idx {
        0 => Some(c"on"),
        1 => Some(c"off"),
        2 => Some(c"clear"),
        3 => Some(c"report"),
        _ => None,
    }
}

fn syn_compare_syntime(s1: &TimeEntry, s2: &TimeEntry) -> std::cmp::Ordering {
    s2.total.cmp(&s1.total)
}

fn syntime_report() {
    if !syntax_present(curwin_mut()) {
        msg(gettext(MSG_NO_ITEMS), 0);
        return;
    }

    let mut ga: Vec<TimeEntry> = Vec::new();

    let mut total_total = profile_zero();
    let mut total_count: i32 = 0;
    for pat in &win_s(curwin_mut()).b_syn_patterns {
        if pat.sp_time.count > 0 {
            total_total = profile_add(total_total, pat.sp_time.total);
            total_count += pat.sp_time.count;
            ga.push(TimeEntry {
                total: pat.sp_time.total,
                count: pat.sp_time.count,
                match_: pat.sp_time.match_,
                slowest: pat.sp_time.slowest,
                average: profile_divide(pat.sp_time.total, pat.sp_time.count),
                id: pat.sp_syn.id as i32,
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
        let mut len = if g_st().columns_global < 80 {
            20
        } else {
            g_st().columns_global - 70
        };
        let pat_ptr = p.pattern.as_deref().unwrap_or(c"");
        let patlen = p.pattern.as_ref().map_or(0, |s| s.to_bytes().len() as i32);
        if patlen < len {
            len = patlen;
        }
        msg_outtrans_len(pat_ptr.to_bytes(), len, 0, false);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_syntime_arg_and_limit_pos_work() {
        assert_eq!(
            get_syntime_arg(&mut crate::os::env::ExpandCtx::new(), 0),
            Some(c"on")
        );
        assert_eq!(
            get_syntime_arg(&mut crate::os::env::ExpandCtx::new(), 3),
            Some(c"report")
        );
        assert!(get_syntime_arg(&mut crate::os::env::ExpandCtx::new(), 4).is_none());

        let mut pos = Lpos { lnum: 5, col: 10 };
        let mut limit = Lpos { lnum: 5, col: 4 };
        limit_pos(&mut pos, &mut limit);
        assert_eq!(pos.col, 4);
    }
}
