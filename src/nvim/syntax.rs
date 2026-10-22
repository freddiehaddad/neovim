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
use crate::garray::{ga_clear, ga_grow, ga_init, ga_set_growsize, garray_T};
use crate::globals::*;
use crate::hashtab::{
    hash_T, hash_add_item, hash_clear, hash_find, hash_hash, hash_init, hash_lock, hash_lookup,
    hash_remove, hash_unlock, hashitem_T, hashitem_empty, hashtab_T,
};
use crate::highlight_group::{
    highlight_group_name, highlight_link_id, highlight_num_groups, init_highlight, syn_check_group,
    syn_id2attr, syn_list_header, syn_name2id, syn_name2id_len,
};
use crate::indent_c::find_start_comment;
use crate::mbyte::{
    mb_ptr_adv, mb_ptr_back, mb_strcmp_ic, utf_head_off, utf_ptr2char, utfc_ptr2len,
};
use crate::memline::{ml_get, ml_get_buf, ml_get_buf_len, ml_get_len};
use crate::memory::{xcalloc, xfree, xmalloc, xmemcpyz, xstrdup};
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
    MAXCOL, NSUBEXP, RE_MAGIC, REX_SET, REX_USE, ref_extmatch, reg_extmatch_T, regmatch_T,
    regmmatch_T, regprog_T, skip_regexp, unref_extmatch, vim_regcomp, vim_regcomp_had_eol,
    vim_regexec, vim_regexec_multi, vim_regfree,
};
use crate::runtime::{DIP_ALL, DOSO_NONE, do_source, source_runtime};
use crate::strings::{
    gettext, sort_slice_by, strcat, strchr, strcmp, strcpy, strlen, strncmp, strpbrk, vim_snprintf,
    vim_strchr, vim_stricmp, vim_strnicmp, vim_strnsave_up, vim_strsave_up, xstrnsave,
};
use crate::window::win_T;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::mem::{offset_of, size_of};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct syn_time_T {
    pub total: u64,
    pub slowest: u64,
    pub count: c_int,
    pub match_: c_int,
}

#[repr(C)]
pub struct synblock_T {
    pub b_keywtab: hashtab_T,
    pub b_keywtab_ic: hashtab_T,
    pub b_syn_error: bool,
    pub b_syn_slow: bool,
    pub b_syn_ic: c_int,
    pub b_syn_foldlevel: c_int,
    pub b_syn_spell: c_int,
    pub b_syn_patterns: garray_T,
    pub b_syn_clusters: garray_T,
    pub b_spell_cluster_id: c_int,
    pub b_nospell_cluster_id: c_int,
    pub b_syn_containedin: c_int,
    pub b_syn_sync_flags: c_int,
    pub b_syn_sync_id: i16,
    pub b_syn_sync_minlines: i32,
    pub b_syn_sync_maxlines: i32,
    pub b_syn_sync_linebreaks: i32,
    pub b_syn_linecont_pat: *mut c_char,
    pub b_syn_linecont_prog: *mut regprog_T,
    pub b_syn_linecont_time: syn_time_T,
    pub b_syn_linecont_ic: c_int,
    pub b_syn_topgrp: c_int,
    pub b_syn_conceal: c_int,
    pub b_syn_folditems: c_int,
    pub b_sst_array: *mut c_void,
    pub b_sst_len: c_int,
    pub b_sst_first: *mut c_void,
    pub b_sst_search: *mut c_void,
    pub b_sst_firstfree: *mut c_void,
    pub b_sst_freecount: c_int,
    pub b_sst_check_lnum: i32,
    pub b_sst_lasttick: u64,
    pub b_idlist_cache: *mut c_void,
    pub b_langp: garray_T,
    pub b_spell_ismw: [bool; 256],
    pub b_spell_ismw_mb: *mut c_char,
    pub b_p_spc: *mut c_char,
    pub b_cap_prog: *mut c_void,
    pub b_p_spf: *mut c_char,
    pub b_p_spl: *mut c_char,
    pub b_p_spo: *mut c_char,
    pub b_p_spo_flags: c_uint,
    pub b_cjk: c_int,
    pub b_syn_chartab: [u64; 4],
    pub b_syn_isk: *mut c_char,
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
const SST_FIX_STATES: usize = 7;
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
use crate::math::{FAIL, OK};
use crate::sign::EXP_SUBCMD;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct sp_syn {
    pub inc_tag: c_int,
    pub id: i16,
    pub cont_in_list: *mut i16,
}

#[repr(C)]
pub struct keyentry_T {
    pub ke_next: *mut keyentry_T,
    pub k_syn: sp_syn,
    pub next_list: *mut i16,
    pub flags: c_int,
    pub k_char: c_int,
    pub keyword: [c_char; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct bufstate_T {
    pub bs_idx: c_int,
    pub bs_flags: c_int,
    pub bs_seqnr: c_int,
    pub bs_cchar: c_int,
    pub bs_extmatch: *mut reg_extmatch_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union synstate_union_T {
    pub sst_stack: [bufstate_T; SST_FIX_STATES],
    pub sst_ga: garray_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct synstate_T {
    pub sst_next: *mut synstate_T,
    pub sst_lnum: linenr_T,
    pub sst_union: synstate_union_T,
    pub sst_next_flags: c_int,
    pub sst_stacksize: c_int,
    pub sst_next_list: *mut i16,
    pub sst_tick: disptick_T,
    pub sst_change_lnum: linenr_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct synpat_T {
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
    sp_cont_list: *mut i16,
    sp_next_list: *mut i16,
    sp_syn: sp_syn,
    sp_pattern: *mut c_char,
    sp_prog: *mut regprog_T,
    sp_time: syn_time_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct syn_cluster_T {
    scl_name: *mut c_char,
    scl_name_u: *mut c_char,
    scl_list: *mut i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
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
    si_cont_list: *mut i16,
    si_next_list: *mut i16,
    si_extmatch: *mut reg_extmatch_T,
}

struct syn_opt_arg_T {
    flags: c_int,
    keyword: bool,
    sync_idx: *mut c_int,
    has_cont_list: bool,
    cont_list: *mut i16,
    cont_in_list: *mut i16,
    next_list: *mut i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct idl_entry_T {
    idl_key: *mut i16,
    idl_ids: *mut i16,
    idl_count: c_int,
    idl_marker: i16,
    idl_usable: bool,
}

#[repr(C)]
struct idl_cache_T {
    ent: [idl_entry_T; IDL_CACHE_SIZE],
    round: c_int,
    last: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct time_entry_T {
    total: proftime_T,
    count: c_int,
    match_: c_int,
    slowest: proftime_T,
    average: proftime_T,
    id: c_int,
    pattern: *mut c_char,
}

#[repr(C)]
struct pat_ptr_T {
    pp_synp: *mut synpat_T,
    pp_matchgroup_id: c_int,
    pp_next: *mut pat_ptr_T,
}

static mut did_syntax_onoff: bool = false;
static mut changedtick_last: i64 = 0;
static mut try_next_column: bool = false;
static mut in_id_list_depth: c_int = 0;

static mut current_attr: c_int = 0;
static mut current_id: c_int = 0;
static mut current_trans_id: c_int = 0;
static mut current_flags: c_int = 0;
static mut current_seqnr: c_int = 0;
static mut current_sub_char: c_int = 0;

static mut syn_cmdlinep: *mut *mut c_char = ptr::null_mut();
static mut current_syn_inc_tag: c_int = 0;
static mut running_syn_inc_tag: c_int = 0;
static mut keepend_level: c_int = -1;
static mut next_seqnr: c_int = 1;

static mut next_match_col: c_int = 0;
static mut next_match_m_endpos: lpos_T = lpos_T { lnum: 0, col: 0 };
static mut next_match_h_startpos: lpos_T = lpos_T { lnum: 0, col: 0 };
static mut next_match_h_endpos: lpos_T = lpos_T { lnum: 0, col: 0 };
static mut next_match_idx: c_int = 0;
static mut next_match_flags: c_int = 0;
static mut next_match_eos_pos: lpos_T = lpos_T { lnum: 0, col: 0 };
static mut next_match_eoe_pos: lpos_T = lpos_T { lnum: 0, col: 0 };
static mut next_match_end_idx: c_int = 0;
static mut next_match_extmatch: *mut reg_extmatch_T = ptr::null_mut();

static mut syn_win: *mut win_T = ptr::null_mut();
static mut syn_buf: *mut buf_T = ptr::null_mut();
static mut syn_block: *mut synblock_T = ptr::null_mut();
static mut syn_tm: *mut proftime_T = ptr::null_mut();
static mut current_lnum: linenr_T = 0;
static mut current_col: colnr_T = 0;
static mut current_state_stored: bool = false;
static mut current_finished: bool = false;
static mut current_state: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};
static mut current_next_list: *mut i16 = ptr::null_mut();
static mut current_next_flags: c_int = 0;
static mut current_line_id: c_int = 0;
static mut syn_time_on: bool = false;
static mut last_matchgroup: c_int = 0;
static mut expand_what: c_int = EXP_SUBCMD;

#[inline]
pub unsafe fn win_s(wp: *mut win_T) -> *mut synblock_T {
    unsafe { (*wp).w_s.cast::<synblock_T>() }
}

#[inline]
pub unsafe fn ga_empty(gap: *const garray_T) -> bool {
    unsafe { (*gap).ga_len <= 0 }
}

#[inline]
pub unsafe fn hi_empty(hi: *const hashitem_T) -> bool {
    unsafe { hashitem_empty(&*hi) }
}

#[inline]
unsafe fn syn_items(block: *mut synblock_T) -> *mut synpat_T {
    unsafe { (*block).b_syn_patterns.ga_data.cast::<synpat_T>() }
}

#[inline]
unsafe fn syn_clstr(block: *mut synblock_T) -> *mut syn_cluster_T {
    unsafe { (*block).b_syn_clusters.ga_data.cast::<syn_cluster_T>() }
}

#[inline]
unsafe fn syn_state_p(ssp: *mut garray_T) -> *mut bufstate_T {
    unsafe { (*ssp).ga_data.cast::<bufstate_T>() }
}

#[inline]
unsafe fn cur_state_at(idx: c_int) -> *mut stateitem_T {
    unsafe {
        current_state
            .ga_data
            .cast::<stateitem_T>()
            .offset(idx as isize)
    }
}

#[inline]
unsafe fn ke2hikey(kp: *mut keyentry_T) -> *mut c_char {
    unsafe { (&raw mut (*kp).keyword).cast::<c_char>() }
}

#[inline]
unsafe fn hikey2ke(p: *mut c_char) -> *mut keyentry_T {
    unsafe {
        p.byte_sub(offset_of!(keyentry_T, keyword))
            .cast::<keyentry_T>()
    }
}

#[inline]
unsafe fn hi2ke(hi: *const hashitem_T) -> *mut keyentry_T {
    unsafe { hikey2ke((*hi).hi_key) }
}

#[inline]
fn id_list_all() -> *mut i16 {
    usize::MAX as *mut i16
}

#[inline]
unsafe fn invalid_state(ssp: *const garray_T) -> bool {
    unsafe { (*ssp).ga_itemsize == 0 }
}

#[inline]
unsafe fn valid_state(ssp: *const garray_T) -> bool {
    unsafe { (*ssp).ga_itemsize != 0 }
}

/// Set the timeout used for syntax highlighting.
pub unsafe fn syn_set_timeout(tm: *mut proftime_T) {
    unsafe {
        syn_tm = tm;
    }
}

/// Start the syntax recognition for a line.
pub unsafe fn syntax_start(wp: *mut win_T, lnum: linenr_T) {
    unsafe {
        let mut last_valid: *mut synstate_T = ptr::null_mut();
        let mut last_min_valid: *mut synstate_T = ptr::null_mut();
        let mut prev: *mut synstate_T = ptr::null_mut();

        current_sub_char = NUL as c_int;

        if syn_block != win_s(wp)
            || syn_buf != (*wp).w_buffer
            || changedtick_last != buf_get_changedtick(syn_buf)
        {
            invalidate_current_state();
            syn_buf = (*wp).w_buffer;
            syn_block = win_s(wp);
        }
        changedtick_last = buf_get_changedtick(syn_buf);
        syn_win = wp;

        syn_stack_alloc();
        if (*syn_block).b_sst_array.is_null() {
            return;
        }
        (*syn_block).b_sst_lasttick = display_tick;

        if valid_state(&raw const current_state)
            && current_lnum < lnum
            && current_lnum < (*syn_buf).b_ml.ml_line_count
        {
            syn_finish_line(false);
            if !current_state_stored {
                current_lnum += 1;
                store_current_state();
            }

            if current_lnum != lnum {
                invalidate_current_state();
            }
        } else {
            invalidate_current_state();
        }

        if invalid_state(&raw const current_state) && !(*syn_block).b_sst_array.is_null() {
            let mut p = (*syn_block).b_sst_first.cast::<synstate_T>();
            while !p.is_null() {
                if (*p).sst_lnum > lnum {
                    break;
                }
                if (*p).sst_change_lnum == 0 {
                    last_valid = p;
                    if (*p).sst_lnum >= lnum - (*syn_block).b_syn_sync_minlines {
                        last_min_valid = p;
                    }
                }
                p = (*p).sst_next;
            }
            if !last_min_valid.is_null() {
                load_current_state(last_min_valid);
            }
        }

        let first_stored = if invalid_state(&raw const current_state) {
            syn_sync(wp, lnum, last_valid);
            if current_lnum == 1 {
                1
            } else {
                current_lnum + (*syn_block).b_syn_sync_minlines
            }
        } else {
            current_lnum
        };

        let dist = if (*syn_block).b_sst_len <= Rows {
            999999
        } else {
            (*syn_buf).b_ml.ml_line_count / ((*syn_block).b_sst_len - Rows) + 1
        };

        while current_lnum < lnum {
            syn_start_line();
            syn_finish_line(false);
            current_lnum += 1;

            if current_lnum >= first_stored {
                if prev.is_null() {
                    prev = syn_stack_find_entry(current_lnum - 1);
                }
                let mut sp = if prev.is_null() {
                    (*syn_block).b_sst_first.cast::<synstate_T>()
                } else {
                    prev
                };
                while !sp.is_null() && (*sp).sst_lnum < current_lnum {
                    sp = (*sp).sst_next;
                }
                if !sp.is_null() && (*sp).sst_lnum == current_lnum && syn_stack_equal(sp) {
                    let parsed_lnum = current_lnum;
                    prev = sp;
                    while !sp.is_null() && (*sp).sst_change_lnum <= parsed_lnum {
                        if (*sp).sst_lnum <= lnum {
                            prev = sp;
                        } else if (*sp).sst_change_lnum == 0 {
                            break;
                        }
                        (*sp).sst_change_lnum = 0;
                        sp = (*sp).sst_next;
                    }
                    load_current_state(prev);
                } else if prev.is_null()
                    || current_lnum == lnum
                    || current_lnum >= (*prev).sst_lnum + dist
                {
                    prev = store_current_state();
                }
            }

            line_breakcheck();
            if got_int {
                current_lnum = lnum;
                break;
            }
        }

        syn_start_line();
    }
}

unsafe fn clear_syn_state(p: *mut synstate_T) {
    unsafe {
        if (*p).sst_stacksize > SST_FIX_STATES as c_int {
            let gap = &raw mut (*p).sst_union.sst_ga;
            for i in 0..(*gap).ga_len {
                let bs = (*gap).ga_data.cast::<bufstate_T>().offset(i as isize);
                unref_extmatch((*bs).bs_extmatch);
            }
            ga_clear(gap);
        } else {
            for i in 0..((*p).sst_stacksize as usize) {
                unref_extmatch((*p).sst_union.sst_stack[i].bs_extmatch);
            }
        }
    }
}

unsafe fn clear_current_state() {
    unsafe {
        for i in 0..current_state.ga_len {
            let si = cur_state_at(i);
            unref_extmatch((*si).si_extmatch);
        }
        ga_clear(&raw mut current_state);
    }
}

unsafe fn syn_sync(wp: *mut win_T, mut start_lnum: linenr_T, last_valid: *mut synstate_T) {
    unsafe {
        let mut found_flags: c_int = 0;
        let mut found_match_idx: c_int = 0;
        let mut found_current_lnum: linenr_T = 0;
        let mut found_current_col: colnr_T = 0;
        let mut found_m_endpos = lpos_T { lnum: 0, col: 0 };

        invalidate_current_state();

        if (*syn_block).b_syn_sync_minlines > start_lnum {
            start_lnum = 1;
        } else {
            let mut lnum = if (*syn_block).b_syn_sync_minlines == 1 {
                1
            } else if (*syn_block).b_syn_sync_minlines < 10 {
                (*syn_block).b_syn_sync_minlines * 2
            } else {
                (*syn_block).b_syn_sync_minlines * 3 / 2
            };
            if (*syn_block).b_syn_sync_maxlines != 0 && lnum > (*syn_block).b_syn_sync_maxlines {
                lnum = (*syn_block).b_syn_sync_maxlines;
            }
            if lnum >= start_lnum {
                start_lnum = 1;
            } else {
                start_lnum -= lnum;
            }
        }
        current_lnum = start_lnum;

        if ((*syn_block).b_syn_sync_flags & SF_CCOMMENT) != 0 {
            let curwin_save = curwin;
            curwin = wp;
            let curbuf_save = curbuf;
            curbuf = syn_buf;

            while start_lnum > 1 {
                let l = ml_get(start_lnum - 1);
                if *l == NUL
                    || *l.offset((ml_get_len(start_lnum - 1) - 1) as isize) != b'\\' as c_char
                {
                    break;
                }
                start_lnum -= 1;
            }
            current_lnum = start_lnum;

            let cursor_save: pos_T = (*wp).w_cursor;
            (*wp).w_cursor.lnum = start_lnum;
            (*wp).w_cursor.col = 0;

            if !find_start_comment((*syn_block).b_syn_sync_maxlines).is_null() {
                let mut idx = (*syn_block).b_syn_patterns.ga_len - 1;
                while idx >= 0 {
                    let spp = syn_items(syn_block).offset(idx as isize);
                    if (*spp).sp_syn.id == (*syn_block).b_syn_sync_id
                        && (*spp).sp_type == SPTYPE_START
                    {
                        validate_current_state();
                        push_current_state(idx);
                        update_si_attr(current_state.ga_len - 1);
                        break;
                    }
                    idx -= 1;
                }
            }

            (*wp).w_cursor = cursor_save;
            curwin = curwin_save;
            curbuf = curbuf_save;
        } else if ((*syn_block).b_syn_sync_flags & SF_MATCH) != 0 {
            let break_lnum = if (*syn_block).b_syn_sync_maxlines != 0
                && start_lnum > (*syn_block).b_syn_sync_maxlines
            {
                start_lnum - (*syn_block).b_syn_sync_maxlines
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
                if got_int {
                    invalidate_current_state();
                    current_lnum = start_lnum;
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

                current_lnum = lnum;
                while current_lnum < end_lnum {
                    syn_start_line();
                    loop {
                        let had_sync_point = syn_finish_line(true);
                        if had_sync_point && current_state.ga_len != 0 {
                            let cur_si = cur_state_at(current_state.ga_len - 1);
                            if (*cur_si).si_m_endpos.lnum > start_lnum {
                                current_lnum = end_lnum;
                                break;
                            }
                            if (*cur_si).si_idx < 0 {
                                found_flags = 0;
                                found_match_idx = KEYWORD_IDX;
                            } else {
                                let spp = syn_items(syn_block).offset((*cur_si).si_idx as isize);
                                found_flags = (*spp).sp_flags;
                                found_match_idx = (*spp).sp_sync_idx;
                            }
                            found_current_lnum = current_lnum;
                            found_current_col = current_col;
                            found_m_endpos = (*cur_si).si_m_endpos;
                            if found_m_endpos.lnum > current_lnum {
                                current_lnum = found_m_endpos.lnum;
                                current_col = found_m_endpos.col;
                                if current_lnum >= end_lnum {
                                    break;
                                }
                            } else if found_m_endpos.col > current_col {
                                current_col = found_m_endpos.col;
                            } else {
                                current_col += 1;
                            }

                            let prev_current_col = current_col;
                            if *syn_getcurline().offset(current_col as isize) != NUL {
                                current_col += 1;
                            }
                            check_state_ends();
                            current_col = prev_current_col;
                        } else {
                            break;
                        }
                    }
                    current_lnum += 1;
                }

                if found_flags != 0 {
                    clear_current_state();
                    if found_match_idx >= 0 {
                        push_current_state(found_match_idx);
                        update_si_attr(current_state.ga_len - 1);
                    }

                    if (found_flags & HL_SYNC_HERE) != 0 {
                        current_lnum = found_m_endpos.lnum;
                        current_col = found_m_endpos.col;
                        if !ga_empty(&raw const current_state) {
                            let cur_si = cur_state_at(current_state.ga_len - 1);
                            (*cur_si).si_h_startpos.lnum = found_current_lnum;
                            (*cur_si).si_h_startpos.col = found_current_col;
                            update_si_end(cur_si, current_col, true);
                            check_keepend();
                        }
                        syn_finish_line(false);
                        current_lnum += 1;
                    } else {
                        current_lnum = start_lnum;
                    }

                    break;
                }

                end_lnum = lnum;
                invalidate_current_state();
            }

            if lnum <= break_lnum {
                invalidate_current_state();
                current_lnum = break_lnum + 1;
            }
        }

        validate_current_state();
    }
}

unsafe fn save_chartab(chartab: *mut c_char) {
    unsafe {
        if (*syn_block).b_syn_isk == (&raw mut empty_string_option).cast() {
            return;
        }

        ptr::copy(
            (&raw const (*syn_buf).b_chartab).cast::<u8>(),
            chartab.cast::<u8>(),
            32,
        );
        ptr::copy(
            (&raw const (*win_s(syn_win)).b_syn_chartab).cast::<u8>(),
            (&raw mut (*syn_buf).b_chartab).cast::<u8>(),
            32,
        );
    }
}

unsafe fn restore_chartab(chartab: *const c_char) {
    unsafe {
        if (*win_s(syn_win)).b_syn_isk != (&raw mut empty_string_option).cast() {
            ptr::copy(
                chartab.cast::<u8>(),
                (&raw mut (*syn_buf).b_chartab).cast::<u8>(),
                32,
            );
        }
    }
}

unsafe fn syn_match_linecont(lnum: linenr_T) -> c_int {
    unsafe {
        if (*syn_block).b_syn_linecont_prog.is_null() {
            return 0;
        }

        let mut regmatch: regmmatch_T = std::mem::zeroed();
        let mut buf_chartab = [0 as c_char; 32];
        save_chartab(buf_chartab.as_mut_ptr());

        regmatch.rmm_ic = (*syn_block).b_syn_linecont_ic;
        regmatch.regprog = (*syn_block).b_syn_linecont_prog;
        let r = syn_regexec(
            &raw mut regmatch,
            lnum,
            0,
            &raw mut (*syn_block).b_syn_linecont_time,
        );
        (*syn_block).b_syn_linecont_prog = regmatch.regprog;

        restore_chartab(buf_chartab.as_ptr());
        c_int::from(r)
    }
}

unsafe fn syn_start_line() {
    unsafe {
        current_finished = false;
        current_col = 0;

        if !ga_empty(&raw const current_state) {
            syn_update_ends(true);
            check_state_ends();
        }

        next_match_idx = -1;
        current_line_id += 1;
        next_seqnr = 1;
    }
}

unsafe fn syn_update_ends(startofline: bool) {
    unsafe {
        if startofline {
            for i in 0..current_state.ga_len {
                let cur_si = cur_state_at(i);
                if (*cur_si).si_idx >= 0
                    && (*syn_items(syn_block).offset((*cur_si).si_idx as isize)).sp_type
                        == SPTYPE_MATCH
                    && (*cur_si).si_m_endpos.lnum < current_lnum
                {
                    (*cur_si).si_flags |= HL_MATCHCONT;
                    (*cur_si).si_m_endpos.lnum = 0;
                    (*cur_si).si_m_endpos.col = 0;
                    (*cur_si).si_h_endpos = (*cur_si).si_m_endpos;
                    (*cur_si).si_ends = 1;
                }
            }
        }

        let mut i = current_state.ga_len - 1;
        if keepend_level >= 0 {
            while i > keepend_level {
                if ((*cur_state_at(i)).si_flags & HL_EXTEND) != 0 {
                    break;
                }
                i -= 1;
            }
        }

        let mut seen_keepend = false;
        while i < current_state.ga_len {
            let cur_si = cur_state_at(i);
            if ((*cur_si).si_flags & HL_KEEPEND) != 0
                || (seen_keepend && !startofline)
                || (i == current_state.ga_len - 1 && startofline)
            {
                (*cur_si).si_h_startpos.col = 0;
                (*cur_si).si_h_startpos.lnum = current_lnum;

                if ((*cur_si).si_flags & HL_MATCHCONT) == 0 {
                    update_si_end(cur_si, current_col, !startofline);
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

unsafe fn syn_stack_free_block(block: *mut synblock_T) {
    unsafe {
        idl_cache_clear(block);

        if (*block).b_sst_array.is_null() {
            return;
        }

        let mut p = (*block).b_sst_first.cast::<synstate_T>();
        while !p.is_null() {
            clear_syn_state(p);
            p = (*p).sst_next;
        }
        xfree((*block).b_sst_array);
        (*block).b_sst_array = ptr::null_mut();
        (*block).b_sst_first = ptr::null_mut();
        (*block).b_sst_search = ptr::null_mut();
        (*block).b_sst_len = 0;
    }
}

/// Free b_sst_array[] for buffer "buf".
pub unsafe fn syn_stack_free_all(block: *mut synblock_T) {
    unsafe {
        syn_stack_free_block(block);

        if !curtab.is_null() {
            let mut wp = firstwin;
            while !wp.is_null() {
                if win_s(wp) == block && foldmethodIsSyntax(wp) {
                    foldUpdateAll(wp);
                }
                wp = (*wp).w_next;
            }
        }
    }
}

unsafe fn syn_stack_alloc() {
    unsafe {
        let mut len = ((*syn_buf).b_ml.ml_line_count / SST_DIST + Rows * 2)
            .clamp(SST_MIN_ENTRIES, SST_MAX_ENTRIES);
        if (*syn_block).b_sst_len > len * 2 || (*syn_block).b_sst_len < len {
            len = (*syn_buf).b_ml.ml_line_count;
            len = ((len + len / 2) / SST_DIST + Rows * 2).clamp(SST_MIN_ENTRIES, SST_MAX_ENTRIES);

            if !(*syn_block).b_sst_array.is_null() {
                while (*syn_block).b_sst_len - (*syn_block).b_sst_freecount + 2 > len
                    && syn_stack_cleanup()
                {}
                if len < (*syn_block).b_sst_len - (*syn_block).b_sst_freecount + 2 {
                    len = (*syn_block).b_sst_len - (*syn_block).b_sst_freecount + 2;
                }
            }

            debug_assert!(len >= 0);
            let sstp = xcalloc(len as usize, size_of::<synstate_T>()).cast::<synstate_T>();

            let mut copied: usize = 0;
            if !(*syn_block).b_sst_array.is_null() {
                let mut from = (*syn_block).b_sst_first.cast::<synstate_T>();
                while !from.is_null() {
                    let to = sstp.add(copied);
                    *to = *from;
                    (*to).sst_next = to.add(1);
                    copied += 1;
                    from = (*from).sst_next;
                }
            }
            if copied > 0 {
                (*sstp.add(copied - 1)).sst_next = ptr::null_mut();
                (*syn_block).b_sst_first = sstp.cast();
                (*syn_block).b_sst_freecount = len - copied as c_int;
            } else {
                (*syn_block).b_sst_first = ptr::null_mut();
                (*syn_block).b_sst_freecount = len;
            }

            (*syn_block).b_sst_firstfree = sstp.add(copied).cast();
            let ulen = len as usize;
            let mut idx = copied;
            while idx + 1 < ulen {
                (*sstp.add(idx)).sst_next = sstp.add(idx + 1);
                idx += 1;
            }
            (*sstp.add(ulen - 1)).sst_next = ptr::null_mut();

            xfree((*syn_block).b_sst_array);
            (*syn_block).b_sst_array = sstp.cast();
            (*syn_block).b_sst_len = len;
            (*syn_block).b_sst_search = ptr::null_mut();
        }
    }
}

/// Check for changes in a buffer to affect stored syntax states.
pub unsafe fn syn_stack_apply_changes(buf: *mut buf_T) {
    unsafe {
        syn_stack_apply_changes_block(&raw mut (*buf).b_s, buf);

        if !curtab.is_null() {
            let mut wp = firstwin;
            while !wp.is_null() {
                if (*wp).w_buffer == buf && win_s(wp) != &raw mut (*buf).b_s {
                    syn_stack_apply_changes_block(win_s(wp), buf);
                }
                wp = (*wp).w_next;
            }
        }
    }
}

unsafe fn syn_stack_apply_changes_block(block: *mut synblock_T, buf: *mut buf_T) {
    unsafe {
        let mut prev: *mut synstate_T = ptr::null_mut();
        let mut p = (*block).b_sst_first.cast::<synstate_T>();
        while !p.is_null() {
            if (*p).sst_lnum + (*block).b_syn_sync_linebreaks > (*buf).b_mod_top {
                let n = (*p).sst_lnum + (*buf).b_mod_xlines;
                if n <= (*buf).b_mod_bot {
                    let np = (*p).sst_next;
                    if prev.is_null() {
                        (*block).b_sst_first = np.cast();
                    } else {
                        (*prev).sst_next = np;
                    }
                    syn_stack_free_entry(block, p);
                    p = np;
                    continue;
                }
                if (*p).sst_change_lnum != 0 && (*p).sst_change_lnum > (*buf).b_mod_top {
                    if (*p).sst_change_lnum + (*buf).b_mod_xlines > (*buf).b_mod_top {
                        (*p).sst_change_lnum += (*buf).b_mod_xlines;
                    } else {
                        (*p).sst_change_lnum = (*buf).b_mod_top;
                    }
                }
                if (*p).sst_change_lnum == 0 || (*p).sst_change_lnum < (*buf).b_mod_bot {
                    (*p).sst_change_lnum = (*buf).b_mod_bot;
                }

                (*p).sst_lnum = n;
            }
            prev = p;
            p = (*p).sst_next;
        }
    }
}

unsafe fn syn_stack_cleanup() -> bool {
    unsafe {
        let mut retval = false;

        if (*syn_block).b_sst_first.is_null() {
            return retval;
        }

        let dist = if (*syn_block).b_sst_len <= Rows {
            999999
        } else {
            (*syn_buf).b_ml.ml_line_count / ((*syn_block).b_sst_len - Rows) + 1
        };

        let mut tick = (*syn_block).b_sst_lasttick;
        let mut above = false;
        let mut prev = (*syn_block).b_sst_first.cast::<synstate_T>();
        let mut p = (*prev).sst_next;
        while !p.is_null() {
            if (*prev).sst_lnum + dist > (*p).sst_lnum {
                if (*p).sst_tick > (*syn_block).b_sst_lasttick {
                    if !above || (*p).sst_tick < tick {
                        tick = (*p).sst_tick;
                    }
                    above = true;
                } else if !above && (*p).sst_tick < tick {
                    tick = (*p).sst_tick;
                }
            }
            prev = p;
            p = (*p).sst_next;
        }

        prev = (*syn_block).b_sst_first.cast::<synstate_T>();
        p = (*prev).sst_next;
        while !p.is_null() {
            if (*p).sst_tick == tick && (*prev).sst_lnum + dist > (*p).sst_lnum {
                (*prev).sst_next = (*p).sst_next;
                syn_stack_free_entry(syn_block, p);
                p = (*prev).sst_next;
                retval = true;
            } else {
                prev = p;
                p = (*p).sst_next;
            }
        }
        retval
    }
}

unsafe fn syn_stack_free_entry(block: *mut synblock_T, p: *mut synstate_T) {
    unsafe {
        if (*block).b_sst_search == p.cast() {
            (*block).b_sst_search = ptr::null_mut();
        }
        clear_syn_state(p);
        (*p).sst_next = (*block).b_sst_firstfree.cast();
        (*block).b_sst_firstfree = p.cast();
        (*block).b_sst_freecount += 1;
    }
}

unsafe fn syn_stack_find_entry(lnum: linenr_T) -> *mut synstate_T {
    unsafe {
        let mut prev: *mut synstate_T = ptr::null_mut();
        let mut p = (*syn_block).b_sst_first.cast::<synstate_T>();

        let search = (*syn_block).b_sst_search.cast::<synstate_T>();
        if !search.is_null() && (*search).sst_lnum <= lnum {
            p = search;
        }

        while !p.is_null() {
            if (*p).sst_lnum == lnum {
                (*syn_block).b_sst_search = p.cast();
                return p;
            }
            if (*p).sst_lnum > lnum {
                break;
            }
            prev = p;
            p = (*p).sst_next;
        }
        if !prev.is_null() {
            (*syn_block).b_sst_search = prev.cast();
        }
        prev
    }
}

unsafe fn store_current_state() -> *mut synstate_T {
    unsafe {
        let mut sp = syn_stack_find_entry(current_lnum);

        let mut i = current_state.ga_len - 1;
        while i >= 0 {
            let cur_si = cur_state_at(i);
            if (*cur_si).si_h_startpos.lnum >= current_lnum
                || (*cur_si).si_m_endpos.lnum >= current_lnum
                || (*cur_si).si_h_endpos.lnum >= current_lnum
                || ((*cur_si).si_end_idx != 0 && (*cur_si).si_eoe_pos.lnum >= current_lnum)
            {
                break;
            }
            i -= 1;
        }
        if i >= 0 {
            if !sp.is_null() {
                if (*syn_block).b_sst_first == sp.cast() {
                    (*syn_block).b_sst_first = (*sp).sst_next.cast();
                } else {
                    let mut p = (*syn_block).b_sst_first.cast::<synstate_T>();
                    while !p.is_null() {
                        if (*p).sst_next == sp {
                            break;
                        }
                        p = (*p).sst_next;
                    }
                    if !p.is_null() {
                        (*p).sst_next = (*sp).sst_next;
                    }
                }
                syn_stack_free_entry(syn_block, sp);
                sp = ptr::null_mut();
            }
        } else if sp.is_null() || (*sp).sst_lnum != current_lnum {
            if (*syn_block).b_sst_freecount == 0 {
                syn_stack_cleanup();
                sp = syn_stack_find_entry(current_lnum);
            }
            if (*syn_block).b_sst_freecount == 0 {
                sp = ptr::null_mut();
            } else {
                let p = (*syn_block).b_sst_firstfree.cast::<synstate_T>();
                (*syn_block).b_sst_firstfree = (*p).sst_next.cast();
                (*syn_block).b_sst_freecount -= 1;
                if sp.is_null() {
                    (*p).sst_next = (*syn_block).b_sst_first.cast();
                    (*syn_block).b_sst_first = p.cast();
                } else {
                    (*p).sst_next = (*sp).sst_next;
                    (*sp).sst_next = p;
                }
                sp = p;
                (*sp).sst_stacksize = 0;
                (*sp).sst_lnum = current_lnum;
                (*syn_block).b_sst_search = sp.cast();
            }
        }
        if !sp.is_null() {
            clear_syn_state(sp);
            (*sp).sst_stacksize = current_state.ga_len;
            let bp = if current_state.ga_len > SST_FIX_STATES as c_int {
                ga_init(
                    &raw mut (*sp).sst_union.sst_ga,
                    size_of::<bufstate_T>() as c_int,
                    1,
                );
                ga_grow(&raw mut (*sp).sst_union.sst_ga, current_state.ga_len);
                (*sp).sst_union.sst_ga.ga_len = current_state.ga_len;
                syn_state_p(&raw mut (*sp).sst_union.sst_ga)
            } else {
                (&raw mut (*sp).sst_union.sst_stack).cast::<bufstate_T>()
            };
            for j in 0..(*sp).sst_stacksize {
                let cur_si = cur_state_at(j);
                let b = bp.offset(j as isize);
                (*b).bs_idx = (*cur_si).si_idx;
                (*b).bs_flags = (*cur_si).si_flags;
                (*b).bs_seqnr = (*cur_si).si_seqnr;
                (*b).bs_cchar = (*cur_si).si_cchar;
                (*b).bs_extmatch = ref_extmatch((*cur_si).si_extmatch);
            }
            (*sp).sst_next_flags = current_next_flags;
            (*sp).sst_next_list = current_next_list;
            (*sp).sst_tick = display_tick;
            (*sp).sst_change_lnum = 0;
        }
        current_state_stored = true;
        sp
    }
}

unsafe fn load_current_state(from: *mut synstate_T) {
    unsafe {
        clear_current_state();
        validate_current_state();
        keepend_level = -1;
        if (*from).sst_stacksize != 0 {
            ga_grow(&raw mut current_state, (*from).sst_stacksize);
            let bp = if (*from).sst_stacksize > SST_FIX_STATES as c_int {
                syn_state_p(&raw mut (*from).sst_union.sst_ga)
            } else {
                (&raw mut (*from).sst_union.sst_stack).cast::<bufstate_T>()
            };
            for i in 0..(*from).sst_stacksize {
                let cur_si = cur_state_at(i);
                let b = bp.offset(i as isize);
                (*cur_si).si_idx = (*b).bs_idx;
                (*cur_si).si_flags = (*b).bs_flags;
                (*cur_si).si_seqnr = (*b).bs_seqnr;
                (*cur_si).si_cchar = (*b).bs_cchar;
                (*cur_si).si_extmatch = ref_extmatch((*b).bs_extmatch);
                if keepend_level < 0 && ((*cur_si).si_flags & HL_KEEPEND) != 0 {
                    keepend_level = i;
                }
                (*cur_si).si_ends = 0;
                (*cur_si).si_m_lnum = 0;
                if (*cur_si).si_idx >= 0 {
                    (*cur_si).si_next_list =
                        (*syn_items(syn_block).offset((*cur_si).si_idx as isize)).sp_next_list;
                } else {
                    (*cur_si).si_next_list = ptr::null_mut();
                }
                update_si_attr(i);
            }
            current_state.ga_len = (*from).sst_stacksize;
        }
        current_next_list = (*from).sst_next_list;
        current_next_flags = (*from).sst_next_flags;
        current_lnum = (*from).sst_lnum;
    }
}

unsafe fn syn_stack_equal(sp: *mut synstate_T) -> bool {
    unsafe {
        if (*sp).sst_stacksize != current_state.ga_len || (*sp).sst_next_list != current_next_list {
            return false;
        }

        let bp = if (*sp).sst_stacksize > SST_FIX_STATES as c_int {
            syn_state_p(&raw mut (*sp).sst_union.sst_ga)
        } else {
            (&raw mut (*sp).sst_union.sst_stack).cast::<bufstate_T>()
        };

        let mut i = current_state.ga_len - 1;
        while i >= 0 {
            let cur_si = cur_state_at(i);
            let b = bp.offset(i as isize);
            if (*b).bs_idx != (*cur_si).si_idx {
                break;
            }
            if (*b).bs_extmatch == (*cur_si).si_extmatch {
                i -= 1;
                continue;
            }
            let bsx = (*b).bs_extmatch;
            let six = (*cur_si).si_extmatch;
            if bsx.is_null() || six.is_null() {
                break;
            }
            let mut j: usize = 0;
            while j < NSUBEXP {
                if (*bsx).matches[j] != (*six).matches[j] {
                    if (*bsx).matches[j].is_null() || (*six).matches[j].is_null() {
                        break;
                    }
                    if mb_strcmp_ic(
                        (*syn_items(syn_block).offset((*cur_si).si_idx as isize)).sp_ic != 0,
                        (*bsx).matches[j].cast(),
                        (*six).matches[j].cast(),
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
pub unsafe fn syntax_end_parsing(wp: *mut win_T, lnum: linenr_T) {
    unsafe {
        if syn_block != win_s(wp) {
            return;
        }
        let mut sp = syn_stack_find_entry(lnum);
        if !sp.is_null() && (*sp).sst_lnum < lnum {
            sp = (*sp).sst_next;
        }

        if !sp.is_null() && (*sp).sst_change_lnum != 0 {
            (*sp).sst_change_lnum = lnum;
        }
    }
}

unsafe fn invalidate_current_state() {
    unsafe {
        clear_current_state();
        current_state.ga_itemsize = 0;
        current_next_list = ptr::null_mut();
        keepend_level = -1;
    }
}

unsafe fn validate_current_state() {
    unsafe {
        current_state.ga_itemsize = size_of::<stateitem_T>() as c_int;
        ga_set_growsize(&raw mut current_state, 3);
    }
}

/// Check if the syntax at start of lnum changed since last time.
pub unsafe fn syntax_check_changed(lnum: linenr_T) -> bool {
    unsafe {
        let mut retval = true;

        if valid_state(&raw const current_state) && lnum == current_lnum + 1 {
            let sp = syn_stack_find_entry(lnum);
            if !sp.is_null() && (*sp).sst_lnum == lnum {
                syn_finish_line(false);

                if syn_stack_equal(sp) {
                    retval = false;
                }

                current_lnum += 1;
                store_current_state();
            }
        }

        retval
    }
}

unsafe fn syn_finish_line(syncing: bool) -> bool {
    unsafe {
        while !current_finished {
            syn_current_attr(syncing, false, ptr::null_mut(), false);

            if syncing && current_state.ga_len != 0 {
                let cur_si = cur_state_at(current_state.ga_len - 1);
                if (*cur_si).si_idx >= 0
                    && ((*syn_items(syn_block).offset((*cur_si).si_idx as isize)).sp_flags
                        & (HL_SYNC_HERE | HL_SYNC_THERE))
                        != 0
                {
                    return true;
                }

                let prev_current_col = current_col;
                if *syn_getcurline().offset(current_col as isize) != NUL {
                    current_col += 1;
                }
                check_state_ends();
                current_col = prev_current_col;
            }
            current_col += 1;
        }
        false
    }
}

/// Gets highlight attributes for next character.
pub unsafe fn get_syntax_attr(col: colnr_T, can_spell: *mut bool, keep_state: bool) -> c_int {
    unsafe {
        let mut attr: c_int = 0;

        if !can_spell.is_null() {
            *can_spell = if (*syn_block).b_syn_spell == SYNSPL_DEFAULT {
                (*syn_block).b_spell_cluster_id == 0
            } else {
                (*syn_block).b_syn_spell == SYNSPL_TOP
            };
        }

        if (*syn_block).b_sst_array.is_null() {
            return 0;
        }

        if (*syn_buf).b_p_smc > 0 && col >= (*syn_buf).b_p_smc as colnr_T {
            clear_current_state();
            current_id = 0;
            current_trans_id = 0;
            current_flags = 0;
            current_seqnr = 0;
            return 0;
        }

        if invalid_state(&raw const current_state) {
            validate_current_state();
        }

        while current_col <= col {
            attr = syn_current_attr(
                false,
                true,
                can_spell,
                if current_col == col {
                    keep_state
                } else {
                    false
                },
            );
            current_col += 1;
        }

        attr
    }
}

unsafe fn syn_current_attr(
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
        let mut next_list: *mut i16 = ptr::null_mut();
        let mut found_match: bool;
        let mut regmatch: regmmatch_T = std::mem::zeroed();
        let mut pos = lpos_T { lnum: 0, col: 0 };
        let mut cur_extmatch: *mut reg_extmatch_T = ptr::null_mut();
        let mut buf_chartab = [0 as c_char; 32];

        let mut keep_next_list: bool;
        let mut zero_width_next_list = false;
        let mut zero_width_next_ga: garray_T = std::mem::zeroed();

        let mut line = syn_getcurline();
        if *line.offset(current_col as isize) == NUL && current_col != 0 {
            if next_match_idx >= 0 && next_match_col >= current_col && next_match_col != MAXCOL {
                push_next_match();
            }

            current_finished = true;
            current_state_stored = false;
            return 0;
        }

        if *line.offset(current_col as isize) == NUL
            || *line.offset((current_col + 1) as isize) == NUL
        {
            current_finished = true;
            current_state_stored = false;
        }

        if try_next_column {
            next_match_idx = -1;
            try_next_column = false;
        }

        let do_keywords = !syncing
            && ((*syn_block).b_keywtab.ht_used > 0 || (*syn_block).b_keywtab_ic.ht_used > 0);

        ga_init(&raw mut zero_width_next_ga, size_of::<c_int>() as c_int, 10);

        save_chartab(buf_chartab.as_mut_ptr());

        loop {
            found_match = false;
            keep_next_list = false;
            let mut syn_id: c_int = 0;

            if current_state.ga_len != 0 {
                cur_si = cur_state_at(current_state.ga_len - 1);
            } else {
                cur_si = ptr::null_mut();
            }

            if (*syn_block).b_syn_containedin != 0
                || cur_si.is_null()
                || !(*cur_si).si_cont_list.is_null()
            {
                if do_keywords {
                    line = syn_getcurline();
                    let cur_pos = line.offset(current_col as isize);
                    if vim_iswordp_buf(cur_pos, syn_buf)
                        && (current_col == 0
                            || !vim_iswordp_buf(
                                cur_pos
                                    .sub(1)
                                    .sub(utf_head_off(line, cur_pos.sub(1)) as usize),
                                syn_buf,
                            ))
                    {
                        syn_id = check_keyword_id(
                            line,
                            current_col,
                            &raw mut endcol,
                            &raw mut flags,
                            &raw mut next_list,
                            cur_si,
                            &raw mut cchar,
                        );
                        if syn_id != 0 {
                            push_current_state(KEYWORD_IDX);
                            cur_si = cur_state_at(current_state.ga_len - 1);
                            (*cur_si).si_m_startcol = current_col;
                            (*cur_si).si_h_startpos.lnum = current_lnum;
                            (*cur_si).si_h_startpos.col = 0;
                            (*cur_si).si_m_endpos.lnum = current_lnum;
                            (*cur_si).si_m_endpos.col = endcol;
                            (*cur_si).si_h_endpos.lnum = current_lnum;
                            (*cur_si).si_h_endpos.col = endcol;
                            (*cur_si).si_ends = 1;
                            (*cur_si).si_end_idx = 0;
                            (*cur_si).si_flags = flags;
                            (*cur_si).si_seqnr = next_seqnr;
                            next_seqnr += 1;
                            (*cur_si).si_cchar = cchar;
                            if current_state.ga_len > 1 {
                                (*cur_si).si_flags |=
                                    (*cur_state_at(current_state.ga_len - 2)).si_flags & HL_CONCEAL;
                            }
                            (*cur_si).si_id = syn_id;
                            (*cur_si).si_trans_id = syn_id;
                            if (flags & HL_TRANSP) != 0 {
                                if current_state.ga_len < 2 {
                                    (*cur_si).si_attr = 0;
                                    (*cur_si).si_trans_id = 0;
                                } else {
                                    (*cur_si).si_attr =
                                        (*cur_state_at(current_state.ga_len - 2)).si_attr;
                                    (*cur_si).si_trans_id =
                                        (*cur_state_at(current_state.ga_len - 2)).si_trans_id;
                                }
                            } else {
                                (*cur_si).si_attr = syn_id2attr(syn_id);
                            }
                            (*cur_si).si_cont_list = ptr::null_mut();
                            (*cur_si).si_next_list = next_list;
                            check_keepend();
                        }
                    }
                }

                if syn_id == 0 && (*syn_block).b_syn_patterns.ga_len != 0 {
                    if next_match_idx < 0 || next_match_col < current_col {
                        next_match_idx = 0;
                        next_match_col = MAXCOL;
                        let mut idx = (*syn_block).b_syn_patterns.ga_len - 1;
                        while idx >= 0 {
                            let spp = syn_items(syn_block).offset(idx as isize);
                            if (*spp).sp_syncing == syncing
                                && (displaying || ((*spp).sp_flags & HL_DISPLAY) == 0)
                                && ((*spp).sp_type == SPTYPE_MATCH
                                    || (*spp).sp_type == SPTYPE_START)
                                && if !current_next_list.is_null() {
                                    in_id_list(
                                        ptr::null_mut(),
                                        current_next_list,
                                        &raw mut (*spp).sp_syn,
                                        0,
                                    ) != 0
                                } else if cur_si.is_null() {
                                    ((*spp).sp_flags & HL_CONTAINED) == 0
                                } else {
                                    in_id_list(
                                        cur_si,
                                        (*cur_si).si_cont_list,
                                        &raw mut (*spp).sp_syn,
                                        (*spp).sp_flags,
                                    ) != 0
                                }
                            {
                                if (*spp).sp_line_id == current_line_id
                                    && (*spp).sp_startcol >= next_match_col
                                {
                                    idx -= 1;
                                    continue;
                                }
                                (*spp).sp_line_id = current_line_id;

                                let mut lc_col = current_col - (*spp).sp_offsets[SPO_LC_OFF];
                                if lc_col < 0 {
                                    lc_col = 0;
                                }

                                regmatch.rmm_ic = (*spp).sp_ic;
                                regmatch.regprog = (*spp).sp_prog;
                                let r = syn_regexec(
                                    &raw mut regmatch,
                                    current_lnum,
                                    lc_col,
                                    &raw mut (*spp).sp_time,
                                );
                                (*spp).sp_prog = regmatch.regprog;
                                if !r {
                                    (*spp).sp_startcol = MAXCOL;
                                    idx -= 1;
                                    continue;
                                }

                                syn_add_start_off(
                                    &raw mut pos,
                                    &raw mut regmatch,
                                    spp,
                                    SPO_MS_OFF,
                                    -1,
                                );
                                if pos.lnum > current_lnum {
                                    (*spp).sp_startcol = MAXCOL;
                                    idx -= 1;
                                    continue;
                                }
                                startcol = pos.col;
                                (*spp).sp_startcol = startcol;

                                if startcol >= next_match_col {
                                    idx -= 1;
                                    continue;
                                }

                                if did_match_already(idx, &raw mut zero_width_next_ga) {
                                    try_next_column = true;
                                    idx -= 1;
                                    continue;
                                }

                                endpos.lnum = regmatch.endpos[0].lnum;
                                endpos.col = regmatch.endpos[0].col;

                                syn_add_start_off(
                                    &raw mut hl_startpos,
                                    &raw mut regmatch,
                                    spp,
                                    SPO_HS_OFF,
                                    -1,
                                );
                                syn_add_end_off(
                                    &raw mut eos_pos,
                                    &raw mut regmatch,
                                    spp,
                                    SPO_RS_OFF,
                                    0,
                                );

                                unref_extmatch(cur_extmatch);
                                cur_extmatch = re_extmatch_out;
                                re_extmatch_out = ptr::null_mut();

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
                                        &raw mut startpos,
                                        &raw mut endpos,
                                        &raw mut hl_endpos,
                                        &raw mut flags,
                                        &raw mut eoe_pos,
                                        &raw mut end_idx,
                                        cur_extmatch,
                                    );
                                    if endpos.lnum == 0 {
                                        idx -= 1;
                                        continue;
                                    }
                                } else if (*spp).sp_type == SPTYPE_MATCH {
                                    syn_add_end_off(
                                        &raw mut hl_endpos,
                                        &raw mut regmatch,
                                        spp,
                                        SPO_HE_OFF,
                                        0,
                                    );
                                    syn_add_end_off(
                                        &raw mut endpos,
                                        &raw mut regmatch,
                                        spp,
                                        SPO_ME_OFF,
                                        0,
                                    );
                                    if endpos.lnum == current_lnum
                                        && endpos.col + c_int::from(syncing) < startcol
                                    {
                                        if regmatch.startpos[0].col == regmatch.endpos[0].col {
                                            try_next_column = true;
                                        }
                                        idx -= 1;
                                        continue;
                                    }
                                }

                                if hl_startpos.lnum == current_lnum && hl_startpos.col < startcol {
                                    hl_startpos.col = startcol;
                                }
                                limit_pos_zero(&raw mut hl_endpos, &raw mut endpos);

                                next_match_idx = idx;
                                next_match_col = startcol;
                                next_match_m_endpos = endpos;
                                next_match_h_endpos = hl_endpos;
                                next_match_h_startpos = hl_startpos;
                                next_match_flags = flags;
                                next_match_eos_pos = eos_pos;
                                next_match_eoe_pos = eoe_pos;
                                next_match_end_idx = end_idx;
                                unref_extmatch(next_match_extmatch);
                                next_match_extmatch = cur_extmatch;
                                cur_extmatch = ptr::null_mut();
                            }
                            idx -= 1;
                        }
                    }

                    if next_match_idx >= 0 && next_match_col == current_col {
                        let lspp = syn_items(syn_block).offset(next_match_idx as isize);
                        if next_match_m_endpos.lnum == current_lnum
                            && next_match_m_endpos.col == current_col
                            && !(*lspp).sp_next_list.is_null()
                        {
                            current_next_list = (*lspp).sp_next_list;
                            current_next_flags = (*lspp).sp_flags;
                            keep_next_list = true;
                            zero_width_next_list = true;

                            ga_grow(&raw mut zero_width_next_ga, 1);
                            *zero_width_next_ga
                                .ga_data
                                .cast::<c_int>()
                                .offset(zero_width_next_ga.ga_len as isize) = next_match_idx;
                            zero_width_next_ga.ga_len += 1;
                            next_match_idx = -1;
                        } else {
                            cur_si = push_next_match();
                        }
                        found_match = true;
                    }
                }
            }

            if !current_next_list.is_null() && !keep_next_list {
                if !found_match {
                    line = syn_getcurline();
                    if ((current_next_flags & HL_SKIPWHITE) != 0
                        && ascii_iswhite(*line.offset(current_col as isize)))
                        || ((current_next_flags & HL_SKIPEMPTY) != 0 && *line == NUL)
                    {
                        break;
                    }
                }

                current_next_list = ptr::null_mut();
                next_match_idx = -1;
                if !zero_width_next_list {
                    found_match = true;
                }
            }

            if !found_match {
                break;
            }
        }

        restore_chartab(buf_chartab.as_ptr());

        current_attr = 0;
        current_id = 0;
        current_trans_id = 0;
        current_flags = 0;
        current_seqnr = 0;
        if !cur_si.is_null() {
            let mut idx = current_state.ga_len - 1;
            while idx >= 0 {
                sip = cur_state_at(idx);
                if (current_lnum > (*sip).si_h_startpos.lnum
                    || (current_lnum == (*sip).si_h_startpos.lnum
                        && current_col >= (*sip).si_h_startpos.col))
                    && ((*sip).si_h_endpos.lnum == 0
                        || current_lnum < (*sip).si_h_endpos.lnum
                        || (current_lnum == (*sip).si_h_endpos.lnum
                            && current_col < (*sip).si_h_endpos.col))
                {
                    current_attr = (*sip).si_attr;
                    current_id = (*sip).si_id;
                    current_trans_id = (*sip).si_trans_id;
                    current_flags = (*sip).si_flags;
                    current_seqnr = (*sip).si_seqnr;
                    current_sub_char = (*sip).si_cchar;
                    break;
                }
                idx -= 1;
            }

            if !can_spell.is_null() {
                let mut sps = sp_syn {
                    inc_tag: 0,
                    id: 0,
                    cont_in_list: ptr::null_mut(),
                };

                if (*syn_block).b_spell_cluster_id == 0 {
                    if (*syn_block).b_nospell_cluster_id == 0 || current_trans_id == 0 {
                        *can_spell = (*syn_block).b_syn_spell != SYNSPL_NOTOP;
                    } else {
                        sps.id = (*syn_block).b_nospell_cluster_id as i16;
                        *can_spell = in_id_list(sip, (*sip).si_cont_list, &raw mut sps, 0) == 0;
                    }
                } else if current_trans_id == 0 {
                    *can_spell = (*syn_block).b_syn_spell == SYNSPL_TOP;
                } else {
                    sps.id = (*syn_block).b_spell_cluster_id as i16;
                    *can_spell = in_id_list(sip, (*sip).si_cont_list, &raw mut sps, 0) != 0;

                    if (*syn_block).b_nospell_cluster_id != 0 {
                        sps.id = (*syn_block).b_nospell_cluster_id as i16;
                        if in_id_list(sip, (*sip).si_cont_list, &raw mut sps, 0) != 0 {
                            *can_spell = false;
                        }
                    }
                }
            }

            if !syncing && !keep_state {
                check_state_ends();
                if !ga_empty(&raw const current_state)
                    && *syn_getcurline().offset(current_col as isize) != NUL
                {
                    current_col += 1;
                    check_state_ends();
                    current_col -= 1;
                }
            }
        } else if !can_spell.is_null() {
            *can_spell = if (*syn_block).b_syn_spell == SYNSPL_DEFAULT {
                (*syn_block).b_spell_cluster_id == 0
            } else {
                (*syn_block).b_syn_spell == SYNSPL_TOP
            };
        }

        if !current_next_list.is_null() {
            line = syn_getcurline();
            if *line.offset(current_col as isize) != NUL
                && *line.offset((current_col + 1) as isize) == NUL
                && (current_next_flags & (HL_SKIPNL | HL_SKIPEMPTY)) == 0
            {
                current_next_list = ptr::null_mut();
            }
        }

        if !ga_empty(&raw const zero_width_next_ga) {
            ga_clear(&raw mut zero_width_next_ga);
        }

        unref_extmatch(re_extmatch_out);
        re_extmatch_out = ptr::null_mut();
        unref_extmatch(cur_extmatch);

        current_attr
    }
}

unsafe fn did_match_already(idx: c_int, gap: *mut garray_T) -> bool {
    unsafe {
        let mut i = current_state.ga_len - 1;
        while i >= 0 {
            let cur_si = cur_state_at(i);
            if (*cur_si).si_m_startcol == current_col
                && (*cur_si).si_m_lnum == current_lnum
                && (*cur_si).si_idx == idx
            {
                return true;
            }
            i -= 1;
        }

        let mut j = (*gap).ga_len - 1;
        while j >= 0 {
            if *(*gap).ga_data.cast::<c_int>().offset(j as isize) == idx {
                return true;
            }
            j -= 1;
        }

        false
    }
}

unsafe fn push_next_match() -> *mut stateitem_T {
    unsafe {
        let spp = syn_items(syn_block).offset(next_match_idx as isize);

        push_current_state(next_match_idx);
        let mut cur_si = cur_state_at(current_state.ga_len - 1);
        (*cur_si).si_h_startpos = next_match_h_startpos;
        (*cur_si).si_m_startcol = current_col;
        (*cur_si).si_m_lnum = current_lnum;
        (*cur_si).si_flags = (*spp).sp_flags;
        (*cur_si).si_seqnr = next_seqnr;
        next_seqnr += 1;
        (*cur_si).si_cchar = (*spp).sp_cchar;
        if current_state.ga_len > 1 {
            (*cur_si).si_flags |= (*cur_state_at(current_state.ga_len - 2)).si_flags & HL_CONCEAL;
        }
        (*cur_si).si_next_list = (*spp).sp_next_list;
        (*cur_si).si_extmatch = ref_extmatch(next_match_extmatch);
        if (*spp).sp_type == SPTYPE_START && ((*spp).sp_flags & HL_ONELINE) == 0 {
            update_si_end(cur_si, next_match_m_endpos.col, true);
            check_keepend();
        } else {
            (*cur_si).si_m_endpos = next_match_m_endpos;
            (*cur_si).si_h_endpos = next_match_h_endpos;
            (*cur_si).si_ends = 1;
            (*cur_si).si_flags |= next_match_flags;
            (*cur_si).si_eoe_pos = next_match_eoe_pos;
            (*cur_si).si_end_idx = next_match_end_idx;
        }
        if keepend_level < 0 && ((*cur_si).si_flags & HL_KEEPEND) != 0 {
            keepend_level = current_state.ga_len - 1;
        }
        check_keepend();
        update_si_attr(current_state.ga_len - 1);

        let save_flags = (*cur_si).si_flags & (HL_CONCEAL | HL_CONCEALENDS);
        if (*spp).sp_type == SPTYPE_START && (*spp).sp_syn_match_id != 0 {
            push_current_state(next_match_idx);
            cur_si = cur_state_at(current_state.ga_len - 1);
            (*cur_si).si_h_startpos = next_match_h_startpos;
            (*cur_si).si_m_startcol = current_col;
            (*cur_si).si_m_lnum = current_lnum;
            (*cur_si).si_m_endpos = next_match_eos_pos;
            (*cur_si).si_h_endpos = next_match_eos_pos;
            (*cur_si).si_ends = 1;
            (*cur_si).si_end_idx = 0;
            (*cur_si).si_flags = HL_MATCH;
            (*cur_si).si_seqnr = next_seqnr;
            next_seqnr += 1;
            (*cur_si).si_flags |= save_flags;
            if ((*cur_si).si_flags & HL_CONCEALENDS) != 0 {
                (*cur_si).si_flags |= HL_CONCEAL;
            }
            (*cur_si).si_next_list = ptr::null_mut();
            check_keepend();
            update_si_attr(current_state.ga_len - 1);
        }

        next_match_idx = -1;
        cur_si
    }
}

unsafe fn check_state_ends() {
    unsafe {
        let mut cur_si = cur_state_at(current_state.ga_len - 1);
        loop {
            if (*cur_si).si_ends != 0
                && ((*cur_si).si_m_endpos.lnum < current_lnum
                    || ((*cur_si).si_m_endpos.lnum == current_lnum
                        && (*cur_si).si_m_endpos.col <= current_col))
            {
                if (*cur_si).si_end_idx != 0
                    && ((*cur_si).si_eoe_pos.lnum > current_lnum
                        || ((*cur_si).si_eoe_pos.lnum == current_lnum
                            && (*cur_si).si_eoe_pos.col > current_col))
                {
                    (*cur_si).si_idx = (*cur_si).si_end_idx;
                    (*cur_si).si_end_idx = 0;
                    (*cur_si).si_m_endpos = (*cur_si).si_eoe_pos;
                    (*cur_si).si_h_endpos = (*cur_si).si_eoe_pos;
                    (*cur_si).si_flags |= HL_MATCH;
                    (*cur_si).si_seqnr = next_seqnr;
                    next_seqnr += 1;
                    if ((*cur_si).si_flags & HL_CONCEALENDS) != 0 {
                        (*cur_si).si_flags |= HL_CONCEAL;
                    }
                    update_si_attr(current_state.ga_len - 1);

                    current_next_list = ptr::null_mut();
                    next_match_idx = 0;
                    next_match_col = MAXCOL;
                    break;
                }

                current_next_list = (*cur_si).si_next_list;
                current_next_flags = (*cur_si).si_flags;
                if (current_next_flags & (HL_SKIPNL | HL_SKIPEMPTY)) == 0
                    && *syn_getcurline().offset(current_col as isize) == NUL
                {
                    current_next_list = ptr::null_mut();
                }

                let had_extend = ((*cur_si).si_flags & HL_EXTEND) != 0;

                pop_current_state();

                if ga_empty(&raw const current_state) {
                    break;
                }

                if had_extend && keepend_level >= 0 {
                    syn_update_ends(false);
                    if ga_empty(&raw const current_state) {
                        break;
                    }
                }

                cur_si = cur_state_at(current_state.ga_len - 1);

                if (*cur_si).si_idx >= 0
                    && (*syn_items(syn_block).offset((*cur_si).si_idx as isize)).sp_type
                        == SPTYPE_START
                    && ((*cur_si).si_flags & (HL_MATCH | HL_KEEPEND)) == 0
                {
                    update_si_end(cur_si, current_col, true);
                    check_keepend();
                    if (current_next_flags & HL_HAS_EOL) != 0
                        && keepend_level < 0
                        && *syn_getcurline().offset(current_col as isize) == NUL
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

unsafe fn update_si_attr(idx: c_int) {
    unsafe {
        let sip = cur_state_at(idx);
        if (*sip).si_idx < 0 {
            return;
        }

        let spp = syn_items(syn_block).offset((*sip).si_idx as isize);
        if ((*sip).si_flags & HL_MATCH) != 0 {
            (*sip).si_id = (*spp).sp_syn_match_id as c_int;
        } else {
            (*sip).si_id = (*spp).sp_syn.id as c_int;
        }
        (*sip).si_attr = syn_id2attr((*sip).si_id);
        (*sip).si_trans_id = (*sip).si_id;
        if ((*sip).si_flags & HL_MATCH) != 0 {
            (*sip).si_cont_list = ptr::null_mut();
        } else {
            (*sip).si_cont_list = (*spp).sp_cont_list;
        }

        if ((*spp).sp_flags & HL_TRANSP) != 0 && ((*sip).si_flags & HL_MATCH) == 0 {
            if idx == 0 {
                (*sip).si_attr = 0;
                (*sip).si_trans_id = 0;
                if (*sip).si_cont_list.is_null() {
                    (*sip).si_cont_list = id_list_all();
                }
            } else {
                let prev_si = cur_state_at(idx - 1);
                (*sip).si_attr = (*prev_si).si_attr;
                (*sip).si_trans_id = (*prev_si).si_trans_id;
                if (*sip).si_cont_list.is_null() {
                    (*sip).si_flags |= HL_TRANS_CONT;
                    (*sip).si_cont_list = (*prev_si).si_cont_list;
                }
            }
        }
    }
}

unsafe fn check_keepend() {
    unsafe {
        if keepend_level < 0 {
            return;
        }

        let mut i = current_state.ga_len - 1;
        while i > keepend_level {
            if ((*cur_state_at(i)).si_flags & HL_EXTEND) != 0 {
                break;
            }
            i -= 1;
        }

        let mut maxpos = lpos_T { lnum: 0, col: 0 };
        let mut maxpos_h = lpos_T { lnum: 0, col: 0 };
        while i < current_state.ga_len {
            let sip = cur_state_at(i);
            if maxpos.lnum != 0 {
                limit_pos_zero(&raw mut (*sip).si_m_endpos, &raw mut maxpos);
                limit_pos_zero(&raw mut (*sip).si_h_endpos, &raw mut maxpos_h);
                limit_pos_zero(&raw mut (*sip).si_eoe_pos, &raw mut maxpos);
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

unsafe fn update_si_end(sip: *mut stateitem_T, startcol: c_int, force: bool) {
    unsafe {
        if (*sip).si_idx < 0 {
            return;
        }

        if !force && (*sip).si_m_endpos.lnum >= current_lnum {
            return;
        }

        let mut end_idx: c_int = 0;
        let mut startpos = lpos_T {
            lnum: current_lnum,
            col: startcol,
        };
        let mut endpos = lpos_T { lnum: 0, col: 0 };
        let mut hl_endpos = lpos_T { lnum: 0, col: 0 };
        let mut end_endpos = lpos_T { lnum: 0, col: 0 };

        find_endpos(
            (*sip).si_idx,
            &raw mut startpos,
            &raw mut endpos,
            &raw mut hl_endpos,
            &raw mut (*sip).si_flags,
            &raw mut end_endpos,
            &raw mut end_idx,
            (*sip).si_extmatch,
        );

        if endpos.lnum == 0 {
            if ((*syn_items(syn_block).offset((*sip).si_idx as isize)).sp_flags & HL_ONELINE) != 0 {
                (*sip).si_ends = 1;
                (*sip).si_m_endpos.lnum = current_lnum;
                (*sip).si_m_endpos.col = syn_getcurline_len();
            } else {
                (*sip).si_ends = 0;
                (*sip).si_m_endpos.lnum = 0;
            }
            (*sip).si_h_endpos = (*sip).si_m_endpos;
        } else {
            (*sip).si_m_endpos = endpos;
            (*sip).si_h_endpos = hl_endpos;
            (*sip).si_eoe_pos = end_endpos;
            (*sip).si_ends = 1;
            (*sip).si_end_idx = end_idx;
        }
    }
}

unsafe fn push_current_state(idx: c_int) {
    unsafe {
        ga_grow(&raw mut current_state, 1);
        let p = cur_state_at(current_state.ga_len);
        current_state.ga_len += 1;
        ptr::write_bytes(p, 0, 1);
        (*p).si_idx = idx;
    }
}

unsafe fn pop_current_state() {
    unsafe {
        if !ga_empty(&raw const current_state) {
            unref_extmatch((*cur_state_at(current_state.ga_len - 1)).si_extmatch);
            current_state.ga_len -= 1;
        }
        next_match_idx = -1;

        if keepend_level >= current_state.ga_len {
            keepend_level = -1;
        }
    }
}

unsafe fn find_endpos(
    mut idx: c_int,
    startpos: *mut lpos_T,
    m_endpos: *mut lpos_T,
    hl_endpos: *mut lpos_T,
    flagsp: *mut c_int,
    end_endpos: *mut lpos_T,
    end_idx: *mut c_int,
    start_ext: *mut reg_extmatch_T,
) {
    unsafe {
        if idx < 0 {
            return;
        }

        let mut spp = syn_items(syn_block).offset(idx as isize);
        if (*spp).sp_type != SPTYPE_START {
            *hl_endpos = *startpos;
            return;
        }

        loop {
            spp = syn_items(syn_block).offset(idx as isize);
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

        unref_extmatch(re_extmatch_in);
        re_extmatch_in = ref_extmatch(start_ext);

        let mut matchcol = (*startpos).col;
        let start_idx = idx;
        let mut regmatch: regmmatch_T = std::mem::zeroed();
        let mut best_regmatch: regmmatch_T = std::mem::zeroed();
        let mut pos = lpos_T { lnum: 0, col: 0 };
        let mut had_match = false;
        let mut buf_chartab = [0 as c_char; 32];

        save_chartab(buf_chartab.as_mut_ptr());

        loop {
            let mut best_idx: c_int = -1;
            idx = start_idx;
            while idx < (*syn_block).b_syn_patterns.ga_len {
                let mut lc_col = matchcol;
                spp = syn_items(syn_block).offset(idx as isize);
                if (*spp).sp_type != SPTYPE_END {
                    break;
                }
                lc_col -= (*spp).sp_offsets[SPO_LC_OFF];
                if lc_col < 0 {
                    lc_col = 0;
                }

                regmatch.rmm_ic = (*spp).sp_ic;
                regmatch.regprog = (*spp).sp_prog;
                let r = syn_regexec(
                    &raw mut regmatch,
                    (*startpos).lnum,
                    lc_col,
                    &raw mut (*spp).sp_time,
                );
                (*spp).sp_prog = regmatch.regprog;
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
                regmatch.regprog = (*spp_skip).sp_prog;
                let r = syn_regexec(
                    &raw mut regmatch,
                    (*startpos).lnum,
                    lc_col,
                    &raw mut (*spp_skip).sp_time,
                );
                (*spp_skip).sp_prog = regmatch.regprog;
                if r && regmatch.startpos[0].col <= best_regmatch.startpos[0].col {
                    syn_add_end_off(&raw mut pos, &raw mut regmatch, spp_skip, SPO_ME_OFF, 1);

                    if pos.lnum > (*startpos).lnum {
                        break;
                    }

                    let line_len = ml_get_buf_len(syn_buf, (*startpos).lnum);

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

            spp = syn_items(syn_block).offset(best_idx as isize);
            syn_add_end_off(m_endpos, &raw mut best_regmatch, spp, SPO_ME_OFF, 1);
            if (*m_endpos).lnum == (*startpos).lnum && (*m_endpos).col < (*startpos).col {
                (*m_endpos).col = (*startpos).col;
            }

            syn_add_end_off(end_endpos, &raw mut best_regmatch, spp, SPO_HE_OFF, 1);
            if (*end_endpos).lnum == (*startpos).lnum && (*end_endpos).col < (*startpos).col {
                (*end_endpos).col = (*startpos).col;
            }
            limit_pos(end_endpos, m_endpos);

            if (*spp).sp_syn_match_id != (*spp).sp_syn.id && (*spp).sp_syn_match_id != 0 {
                *end_idx = best_idx;
                if ((*spp).sp_off_flags & (1 << (SPO_RE_OFF + SPO_COUNT))) != 0 {
                    (*hl_endpos).lnum = best_regmatch.endpos[0].lnum;
                    (*hl_endpos).col = best_regmatch.endpos[0].col;
                } else {
                    (*hl_endpos).lnum = best_regmatch.startpos[0].lnum;
                    (*hl_endpos).col = best_regmatch.startpos[0].col;
                }
                (*hl_endpos).col += (*spp).sp_offsets[SPO_RE_OFF];

                if (*hl_endpos).lnum == (*startpos).lnum && (*hl_endpos).col < (*startpos).col {
                    (*hl_endpos).col = (*startpos).col;
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
            (*m_endpos).lnum = 0;
        }

        restore_chartab(buf_chartab.as_ptr());

        unref_extmatch(re_extmatch_in);
        re_extmatch_in = ptr::null_mut();
    }
}

unsafe fn limit_pos(pos: *mut lpos_T, limit: *mut lpos_T) {
    unsafe {
        if (*pos).lnum > (*limit).lnum {
            *pos = *limit;
        } else if (*pos).lnum == (*limit).lnum && (*pos).col > (*limit).col {
            (*pos).col = (*limit).col;
        }
    }
}

unsafe fn limit_pos_zero(pos: *mut lpos_T, limit: *mut lpos_T) {
    unsafe {
        if (*pos).lnum == 0 {
            *pos = *limit;
        } else {
            limit_pos(pos, limit);
        }
    }
}

unsafe fn syn_add_end_off(
    result: *mut lpos_T,
    regmatch: *mut regmmatch_T,
    spp: *mut synpat_T,
    idx: usize,
    extra: c_int,
) {
    unsafe {
        let mut col: c_int;
        let mut off: c_int;

        if ((*spp).sp_off_flags & (1 << idx)) != 0 {
            (*result).lnum = (*regmatch).startpos[0].lnum;
            col = (*regmatch).startpos[0].col;
            off = (*spp).sp_offsets[idx] + extra;
        } else {
            (*result).lnum = (*regmatch).endpos[0].lnum;
            col = (*regmatch).endpos[0].col;
            off = (*spp).sp_offsets[idx];
        }

        if (*result).lnum > (*syn_buf).b_ml.ml_line_count {
            col = 0;
        } else if off != 0 {
            let base = ml_get_buf(syn_buf, (*result).lnum);
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
        (*result).col = col;
    }
}

unsafe fn syn_add_start_off(
    result: *mut lpos_T,
    regmatch: *mut regmmatch_T,
    spp: *mut synpat_T,
    idx: usize,
    extra: c_int,
) {
    unsafe {
        let mut col: c_int;
        let mut off: c_int;

        if ((*spp).sp_off_flags & (1 << (idx + SPO_COUNT))) != 0 {
            (*result).lnum = (*regmatch).endpos[0].lnum;
            col = (*regmatch).endpos[0].col;
            off = (*spp).sp_offsets[idx] + extra;
        } else {
            (*result).lnum = (*regmatch).startpos[0].lnum;
            col = (*regmatch).startpos[0].col;
            off = (*spp).sp_offsets[idx];
        }
        if (*result).lnum > (*syn_buf).b_ml.ml_line_count {
            (*result).lnum = (*syn_buf).b_ml.ml_line_count;
            col = ml_get_buf_len(syn_buf, (*result).lnum);
        }
        if off != 0 {
            let base = ml_get_buf(syn_buf, (*result).lnum);
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
        (*result).col = col;
    }
}

#[inline]
unsafe fn syn_getcurline() -> *mut c_char {
    unsafe { ml_get_buf(syn_buf, current_lnum) }
}

#[inline]
unsafe fn syn_getcurline_len() -> colnr_T {
    unsafe { ml_get_buf_len(syn_buf, current_lnum) }
}

unsafe fn syn_regexec(
    rmp: *mut regmmatch_T,
    lnum: linenr_T,
    col: colnr_T,
    st: *mut syn_time_T,
) -> bool {
    unsafe {
        let mut timed_out: c_int = 0;
        let mut pt: proftime_T = 0;
        let l_syn_time_on = syn_time_on;

        if l_syn_time_on {
            pt = profile_start();
        }

        if (*rmp).regprog.is_null() {
            return false;
        }

        (*rmp).rmm_maxcol = (*syn_buf).b_p_smc as colnr_T;
        let r = vim_regexec_multi(rmp, syn_win, syn_buf, lnum, col, syn_tm, &raw mut timed_out);

        if l_syn_time_on {
            pt = profile_end(pt);
            (*st).total = profile_add((*st).total, pt);
            if profile_cmp(pt, (*st).slowest) < 0 {
                (*st).slowest = pt;
            }
            (*st).count += 1;
            if r > 0 {
                (*st).match_ += 1;
            }
        }
        if timed_out != 0 && !(*win_s(syn_win)).b_syn_slow {
            (*win_s(syn_win)).b_syn_slow = true;
            msg(
                gettext(c"'redrawtime' exceeded, syntax highlighting disabled".as_ptr()),
                0,
            );
        }

        if r > 0 {
            (*rmp).startpos[0].lnum += lnum;
            (*rmp).endpos[0].lnum += lnum;
            return true;
        }
        false
    }
}

unsafe fn check_keyword_id(
    line: *mut c_char,
    startcol: c_int,
    endcolp: *mut c_int,
    flagsp: *mut c_int,
    next_listp: *mut *mut i16,
    cur_si: *mut stateitem_T,
    ccharp: *mut c_int,
) -> c_int {
    unsafe {
        let kwp = line.offset(startcol as isize);
        let mut kwlen: c_int = 0;
        loop {
            kwlen += utfc_ptr2len(kwp.offset(kwlen as isize));
            if !vim_iswordp_buf(kwp.offset(kwlen as isize), syn_buf) {
                break;
            }
        }

        if kwlen as usize > MAXKEYWLEN {
            return 0;
        }

        let mut keyword = [0 as c_char; MAXKEYWLEN + 1];
        xmemcpyz(keyword.as_mut_ptr().cast(), kwp.cast(), kwlen as usize);

        let mut kp: *mut keyentry_T = ptr::null_mut();

        if (*syn_block).b_keywtab.ht_used != 0 {
            kp = match_keyword(
                keyword.as_mut_ptr(),
                &raw mut (*syn_block).b_keywtab,
                cur_si,
            );
        }

        if kp.is_null() && (*syn_block).b_keywtab_ic.ht_used != 0 {
            str_foldcase(kwp, kwlen, keyword.as_mut_ptr(), (MAXKEYWLEN + 1) as c_int);
            kp = match_keyword(
                keyword.as_mut_ptr(),
                &raw mut (*syn_block).b_keywtab_ic,
                cur_si,
            );
        }

        if !kp.is_null() {
            *endcolp = startcol + kwlen;
            *flagsp = (*kp).flags;
            *next_listp = (*kp).next_list;
            *ccharp = (*kp).k_char;
            return (*kp).k_syn.id as c_int;
        }

        0
    }
}

unsafe fn match_keyword(
    keyword: *mut c_char,
    ht: *mut hashtab_T,
    cur_si: *mut stateitem_T,
) -> *mut keyentry_T {
    unsafe {
        let hi = hash_find(ht, keyword);
        if !hi_empty(hi) {
            let mut kp = hi2ke(hi);
            while !kp.is_null() {
                if if !current_next_list.is_null() {
                    in_id_list(ptr::null_mut(), current_next_list, &raw mut (*kp).k_syn, 0) != 0
                } else if cur_si.is_null() {
                    ((*kp).flags & HL_CONTAINED) == 0
                } else {
                    in_id_list(
                        cur_si,
                        (*cur_si).si_cont_list,
                        &raw mut (*kp).k_syn,
                        (*kp).flags,
                    ) != 0
                } {
                    return kp;
                }
                kp = (*kp).ke_next;
            }
        }
        ptr::null_mut()
    }
}

unsafe fn syn_cmd_conceal(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = (*eap).arg;
        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
            return;
        }

        let next = skiptowhite(arg);
        if *arg == NUL {
            if (*win_s(curwin)).b_syn_conceal != 0 {
                msg(c"syntax conceal on".as_ptr(), 0);
            } else {
                msg(c"syntax conceal off".as_ptr(), 0);
            }
        } else if vim_strnicmp(arg, c"on".as_ptr(), 2) == 0 && next.offset_from(arg) == 2 {
            (*win_s(curwin)).b_syn_conceal = 1;
        } else if vim_strnicmp(arg, c"off".as_ptr(), 3) == 0 && next.offset_from(arg) == 3 {
            (*win_s(curwin)).b_syn_conceal = 0;
        } else {
            semsg!(gettext(e_illegal_arg), arg);
        }
    }
}

unsafe fn syn_cmd_case(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = (*eap).arg;
        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
            return;
        }

        let next = skiptowhite(arg);
        if *arg == NUL {
            if (*win_s(curwin)).b_syn_ic != 0 {
                msg(c"syntax case ignore".as_ptr(), 0);
            } else {
                msg(c"syntax case match".as_ptr(), 0);
            }
        } else if vim_strnicmp(arg, c"match".as_ptr(), 5) == 0 && next.offset_from(arg) == 5 {
            (*win_s(curwin)).b_syn_ic = 0;
        } else if vim_strnicmp(arg, c"ignore".as_ptr(), 6) == 0 && next.offset_from(arg) == 6 {
            (*win_s(curwin)).b_syn_ic = 1;
        } else {
            semsg!(gettext(e_illegal_arg), arg);
        }
    }
}

unsafe fn syn_cmd_foldlevel(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg = (*eap).arg;
        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
            return;
        }

        if *arg == NUL {
            match (*win_s(curwin)).b_syn_foldlevel {
                SYNFLD_START => {
                    msg(c"syntax foldlevel start".as_ptr(), 0);
                }
                SYNFLD_MINIMUM => {
                    msg(c"syntax foldlevel minimum".as_ptr(), 0);
                }
                _ => {}
            }
            return;
        }

        let arg_end = skiptowhite(arg);
        if vim_strnicmp(arg, c"start".as_ptr(), 5) == 0 && arg_end.offset_from(arg) == 5 {
            (*win_s(curwin)).b_syn_foldlevel = SYNFLD_START;
        } else if vim_strnicmp(arg, c"minimum".as_ptr(), 7) == 0 && arg_end.offset_from(arg) == 7 {
            (*win_s(curwin)).b_syn_foldlevel = SYNFLD_MINIMUM;
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

unsafe fn syn_cmd_spell(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = (*eap).arg;
        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
            return;
        }

        let next = skiptowhite(arg);
        if *arg == NUL {
            if (*win_s(curwin)).b_syn_spell == SYNSPL_TOP {
                msg(c"syntax spell toplevel".as_ptr(), 0);
            } else if (*win_s(curwin)).b_syn_spell == SYNSPL_NOTOP {
                msg(c"syntax spell notoplevel".as_ptr(), 0);
            } else {
                msg(c"syntax spell default".as_ptr(), 0);
            }
        } else if vim_strnicmp(arg, c"toplevel".as_ptr(), 8) == 0 && next.offset_from(arg) == 8 {
            (*win_s(curwin)).b_syn_spell = SYNSPL_TOP;
        } else if vim_strnicmp(arg, c"notoplevel".as_ptr(), 10) == 0 && next.offset_from(arg) == 10
        {
            (*win_s(curwin)).b_syn_spell = SYNSPL_NOTOP;
        } else if vim_strnicmp(arg, c"default".as_ptr(), 7) == 0 && next.offset_from(arg) == 7 {
            (*win_s(curwin)).b_syn_spell = SYNSPL_DEFAULT;
        } else {
            semsg!(gettext(e_illegal_arg), arg);
            return;
        }

        redraw_later(curwin, UPD_NOT_VALID);
    }
}

unsafe fn syn_cmd_iskeyword(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg = (*eap).arg;
        if (*eap).skip != 0 {
            return;
        }

        arg = skipwhite(arg);
        if *arg == NUL {
            msg_puts(c"\n".as_ptr());
            if (*win_s(curwin)).b_syn_isk != (&raw mut empty_string_option).cast() {
                msg_puts(c"syntax iskeyword ".as_ptr());
                msg_outtrans((*win_s(curwin)).b_syn_isk, 0, false);
            } else {
                msg_outtrans(gettext(c"syntax iskeyword not set".as_ptr()), 0, false);
            }
        } else if vim_strnicmp(arg, c"clear".as_ptr(), 5) == 0 {
            (*win_s(curwin)).b_syn_chartab = (*curbuf).b_chartab;
            clear_string_option(&raw mut (*win_s(curwin)).b_syn_isk);
        } else {
            if check_isopt(arg) == FAIL {
                emsg(gettext((&raw const e_invarg).cast()));
                return;
            }
            clear_string_option(&raw mut (*win_s(curwin)).b_syn_isk);
            (*win_s(curwin)).b_syn_isk = xstrdup(arg);
            parse_isopt(
                (*win_s(curwin)).b_syn_isk,
                (&raw mut (*win_s(curwin)).b_syn_chartab).cast(),
            );
        }
        redraw_later(curwin, UPD_NOT_VALID);
    }
}

/// Clear all syntax info for one buffer.
pub unsafe fn syntax_clear(block: *mut synblock_T) {
    unsafe {
        (*block).b_syn_error = false;
        (*block).b_syn_slow = false;
        (*block).b_syn_ic = 0;
        (*block).b_syn_foldlevel = SYNFLD_START;
        (*block).b_syn_spell = SYNSPL_DEFAULT;
        (*block).b_syn_containedin = 0;
        (*block).b_syn_conceal = 0;

        clear_keywtab(&raw mut (*block).b_keywtab);
        clear_keywtab(&raw mut (*block).b_keywtab_ic);

        let mut i = (*block).b_syn_patterns.ga_len - 1;
        while i >= 0 {
            syn_clear_pattern(block, i);
            i -= 1;
        }
        ga_clear(&raw mut (*block).b_syn_patterns);

        let mut j = (*block).b_syn_clusters.ga_len - 1;
        while j >= 0 {
            syn_clear_cluster(block, j);
            j -= 1;
        }
        ga_clear(&raw mut (*block).b_syn_clusters);
        (*block).b_spell_cluster_id = 0;
        (*block).b_nospell_cluster_id = 0;

        (*block).b_syn_sync_flags = 0;
        (*block).b_syn_sync_minlines = 0;
        (*block).b_syn_sync_maxlines = 0;
        (*block).b_syn_sync_linebreaks = 0;

        vim_regfree((*block).b_syn_linecont_prog);
        (*block).b_syn_linecont_prog = ptr::null_mut();
        xfree((*block).b_syn_linecont_pat.cast());
        (*block).b_syn_linecont_pat = ptr::null_mut();
        (*block).b_syn_folditems = 0;
        clear_string_option(&raw mut (*block).b_syn_isk);

        syn_stack_free_all(block);
        invalidate_current_state();

        running_syn_inc_tag = 0;
    }
}

/// Get rid of ownsyntax for window "wp".
pub unsafe fn reset_synblock(wp: *mut win_T) {
    unsafe {
        if win_s(wp) != &raw mut (*(*wp).w_buffer).b_s {
            syntax_clear(win_s(wp));
            xfree((*wp).w_s);
            (*wp).w_s = (&raw mut (*(*wp).w_buffer).b_s).cast();
        }
    }
}

unsafe fn syntax_sync_clear() {
    unsafe {
        let mut i = (*win_s(curwin)).b_syn_patterns.ga_len - 1;
        while i >= 0 {
            if (*syn_items(win_s(curwin)).offset(i as isize)).sp_syncing {
                syn_remove_pattern(win_s(curwin), i);
            }
            i -= 1;
        }

        (*win_s(curwin)).b_syn_sync_flags = 0;
        (*win_s(curwin)).b_syn_sync_minlines = 0;
        (*win_s(curwin)).b_syn_sync_maxlines = 0;
        (*win_s(curwin)).b_syn_sync_linebreaks = 0;

        vim_regfree((*win_s(curwin)).b_syn_linecont_prog);
        (*win_s(curwin)).b_syn_linecont_prog = ptr::null_mut();
        xfree((*win_s(curwin)).b_syn_linecont_pat.cast());
        (*win_s(curwin)).b_syn_linecont_pat = ptr::null_mut();
        clear_string_option(&raw mut (*win_s(curwin)).b_syn_isk);

        syn_stack_free_all(win_s(curwin));
    }
}

unsafe fn syn_remove_pattern(block: *mut synblock_T, idx: c_int) {
    unsafe {
        let spp = syn_items(block).offset(idx as isize);
        if ((*spp).sp_flags & HL_FOLD) != 0 {
            (*block).b_syn_folditems -= 1;
        }
        syn_clear_pattern(block, idx);
        ptr::copy(
            spp.add(1),
            spp,
            ((*block).b_syn_patterns.ga_len - idx - 1) as usize,
        );
        (*block).b_syn_patterns.ga_len -= 1;
    }
}

unsafe fn syn_clear_pattern(block: *mut synblock_T, i: c_int) {
    unsafe {
        let items = syn_items(block);
        let spp = items.offset(i as isize);
        xfree((*spp).sp_pattern.cast());
        vim_regfree((*spp).sp_prog);
        if i == 0 || (*items.offset((i - 1) as isize)).sp_type != SPTYPE_START {
            xfree((*spp).sp_cont_list.cast());
            xfree((*spp).sp_next_list.cast());
            xfree((*spp).sp_syn.cont_in_list.cast());
        }
    }
}

unsafe fn syn_clear_cluster(block: *mut synblock_T, i: c_int) {
    unsafe {
        let clp = syn_clstr(block).offset(i as isize);
        xfree((*clp).scl_name.cast());
        xfree((*clp).scl_name_u.cast());
        xfree((*clp).scl_list.cast());
    }
}

unsafe fn syn_cmd_clear(eap: *mut exarg_T, syncing: c_int) {
    unsafe {
        let mut arg = (*eap).arg;
        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
            return;
        }

        if (*win_s(curwin)).b_syn_topgrp != 0 {
            return;
        }

        if ends_excmd(*arg as c_int) != 0 {
            if syncing != 0 {
                syntax_sync_clear();
            } else {
                syntax_clear(win_s(curwin));
                if win_s(curwin) == &raw mut (*(*curwin).w_buffer).b_s {
                    do_unlet(c"b:current_syntax".as_ptr(), 16, true);
                }
                do_unlet(c"w:current_syntax".as_ptr(), 16, true);
            }
        } else {
            while ends_excmd(*arg as c_int) == 0 {
                let arg_end = skiptowhite(arg);
                if *arg == b'@' as c_char {
                    let id = syn_scl_namen2id(arg.add(1), arg_end.offset_from(arg) as c_int - 1);
                    if id == 0 {
                        semsg!(gettext(c"E391: No such syntax cluster: %s".as_ptr()), arg);
                        break;
                    }
                    let scl_id = id - SYNID_CLUSTER as c_int;
                    let clp = syn_clstr(win_s(curwin)).offset(scl_id as isize);
                    xfree((*clp).scl_list.cast());
                    (*clp).scl_list = ptr::null_mut();
                } else {
                    let id = syn_name2id_len(arg, arg_end.offset_from(arg) as usize);
                    if id == 0 {
                        semsg!(gettext((&raw const e_nogroup).cast()), arg);
                        break;
                    }
                    syn_clear_one(id, syncing != 0);
                }
                arg = skipwhite(arg_end);
            }
        }
        redraw_curbuf_later(UPD_SOME_VALID);
        syn_stack_free_all(win_s(curwin));
    }
}

unsafe fn syn_clear_one(id: c_int, syncing: bool) {
    unsafe {
        if !syncing {
            syn_clear_keyword(id, &raw mut (*win_s(curwin)).b_keywtab);
            syn_clear_keyword(id, &raw mut (*win_s(curwin)).b_keywtab_ic);
        }

        let mut idx = (*win_s(curwin)).b_syn_patterns.ga_len - 1;
        while idx >= 0 {
            let spp = syn_items(win_s(curwin)).offset(idx as isize);
            if (*spp).sp_syn.id as c_int == id && (*spp).sp_syncing == syncing {
                syn_remove_pattern(win_s(curwin), idx);
            }
            idx -= 1;
        }
    }
}

unsafe fn syn_cmd_on(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        syn_cmd_onoff(eap, c"syntax".as_ptr());
    }
}

unsafe fn syn_cmd_reset(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        (*eap).nextcmd = check_nextcmd((*eap).arg);
        if (*eap).skip == 0 {
            init_highlight(true, true);
        }
    }
}

unsafe fn syn_cmd_manual(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        syn_cmd_onoff(eap, c"manual".as_ptr());
    }
}

unsafe fn syn_cmd_off(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        syn_cmd_onoff(eap, c"nosyntax".as_ptr());
    }
}

unsafe fn syn_cmd_onoff(eap: *mut exarg_T, name: *const c_char) {
    unsafe {
        (*eap).nextcmd = check_nextcmd((*eap).arg);
        if (*eap).skip == 0 {
            did_syntax_onoff = true;
            let mut buf = [0 as c_char; 100];
            ptr::copy_nonoverlapping(c"so ".as_ptr(), buf.as_mut_ptr(), 4);
            vim_snprintf!(buf.as_mut_ptr().add(3), 100 - 3, SYNTAX_FNAME, name);
            do_cmdline_cmd(buf.as_ptr());
        }
    }
}

pub unsafe fn syn_maybe_enable() {
    unsafe {
        if !did_syntax_onoff {
            let mut ea: exarg_T = std::mem::zeroed();
            ea.arg = c"".as_ptr().cast_mut();
            ea.skip = 0;
            syn_cmd_on(&raw mut ea, 0);
        }
    }
}

unsafe fn syn_cmd_list(eap: *mut exarg_T, syncing: c_int) {
    unsafe {
        let mut arg = (*eap).arg;
        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
            return;
        }

        msg_ext_set_kind(c"list_cmd".as_ptr());
        if !syntax_present(curwin) {
            msg(gettext(msg_no_items), 0);
            return;
        }

        if syncing != 0 {
            if ((*win_s(curwin)).b_syn_sync_flags & SF_CCOMMENT) != 0 {
                msg_puts(gettext(c"syncing on C-style comments".as_ptr()));
                syn_lines_msg();
                syn_match_msg();
            } else if ((*win_s(curwin)).b_syn_sync_flags & SF_MATCH) != 0 {
                msg_puts_title(gettext(c"\n--- Syntax sync items ---".as_ptr()));
                if (*win_s(curwin)).b_syn_sync_minlines > 0
                    || (*win_s(curwin)).b_syn_sync_maxlines > 0
                    || (*win_s(curwin)).b_syn_sync_linebreaks > 0
                {
                    msg_puts(gettext(c"\nsyncing on items".as_ptr()));
                    syn_lines_msg();
                    syn_match_msg();
                }
                let mut id: c_int = 1;
                while id <= highlight_num_groups() && !got_int {
                    syn_list_one(id, true, false);
                    id += 1;
                }
            } else if (*win_s(curwin)).b_syn_sync_minlines == 0 {
                msg_puts(gettext(c"no syncing".as_ptr()));
            } else {
                if (*win_s(curwin)).b_syn_sync_minlines == MAXLNUM {
                    msg_puts(gettext(c"syncing starts at the first line".as_ptr()));
                } else {
                    msg_puts(gettext(c"syncing starts ".as_ptr()));
                    msg_outnum((*win_s(curwin)).b_syn_sync_minlines as c_int);
                    msg_puts(gettext(c" lines before top line".as_ptr()));
                }
                syn_match_msg();
            }
            return;
        }

        msg_puts_title(gettext(c"\n--- Syntax items ---".as_ptr()));
        if ends_excmd(*arg as c_int) != 0 {
            let mut id: c_int = 1;
            while id <= highlight_num_groups() && !got_int {
                syn_list_one(id, false, false);
                id += 1;
            }
            let mut cid: c_int = 0;
            while cid < (*win_s(curwin)).b_syn_clusters.ga_len && !got_int {
                syn_list_cluster(cid);
                cid += 1;
            }
        } else {
            while ends_excmd(*arg as c_int) == 0 && !got_int {
                let arg_end = skiptowhite(arg);
                if *arg == b'@' as c_char {
                    let id = syn_scl_namen2id(arg.add(1), arg_end.offset_from(arg) as c_int - 1);
                    if id == 0 {
                        semsg!(gettext(c"E392: No such syntax cluster: %s".as_ptr()), arg);
                    } else {
                        syn_list_cluster(id - SYNID_CLUSTER as c_int);
                    }
                } else {
                    let id = syn_name2id_len(arg, arg_end.offset_from(arg) as usize);
                    if id == 0 {
                        semsg!(gettext((&raw const e_nogroup).cast()), arg);
                    } else {
                        syn_list_one(id, false, true);
                    }
                }
                arg = skipwhite(arg_end);
            }
        }
        (*eap).nextcmd = check_nextcmd(arg);
    }
}

unsafe fn syn_lines_msg() {
    unsafe {
        if (*win_s(curwin)).b_syn_sync_maxlines > 0 || (*win_s(curwin)).b_syn_sync_minlines > 0 {
            msg_puts(c"; ".as_ptr());
            if (*win_s(curwin)).b_syn_sync_minlines == MAXLNUM {
                msg_puts(gettext(c"from the first line".as_ptr()));
            } else {
                if (*win_s(curwin)).b_syn_sync_minlines > 0 {
                    msg_puts(gettext(c"minimal ".as_ptr()));
                    msg_outnum((*win_s(curwin)).b_syn_sync_minlines as c_int);
                    if (*win_s(curwin)).b_syn_sync_maxlines != 0 {
                        msg_puts(c", ".as_ptr());
                    }
                }
                if (*win_s(curwin)).b_syn_sync_maxlines > 0 {
                    msg_puts(gettext(c"maximal ".as_ptr()));
                    msg_outnum((*win_s(curwin)).b_syn_sync_maxlines as c_int);
                }
                msg_puts(gettext(c" lines before top line".as_ptr()));
            }
        }
    }
}

unsafe fn syn_match_msg() {
    unsafe {
        if (*win_s(curwin)).b_syn_sync_linebreaks > 0 {
            msg_puts(gettext(c"; match ".as_ptr()));
            msg_outnum((*win_s(curwin)).b_syn_sync_linebreaks as c_int);
            msg_puts(gettext(c" line breaks".as_ptr()));
        }
    }
}

unsafe fn syn_list_one(id: c_int, syncing: bool, link_only: bool) {
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
            did_header = syn_list_keywords(id, &raw const (*win_s(curwin)).b_keywtab, false, hl_id);
            did_header = syn_list_keywords(
                id,
                &raw const (*win_s(curwin)).b_keywtab_ic,
                did_header,
                hl_id,
            );
        }

        let mut idx: c_int = 0;
        while idx < (*win_s(curwin)).b_syn_patterns.ga_len && !got_int {
            let spp = syn_items(win_s(curwin)).offset(idx as isize);
            if (*spp).sp_syn.id as c_int != id || (*spp).sp_syncing != syncing {
                idx += 1;
                continue;
            }

            syn_list_header(did_header, 0, id, true);
            did_header = true;
            last_matchgroup = 0;
            if (*spp).sp_type == SPTYPE_MATCH {
                put_pattern(c"match".as_ptr(), b' ' as c_int, spp, hl_id);
                msg_putchar(b' ' as c_int);
            } else if (*spp).sp_type == SPTYPE_START {
                while (*syn_items(win_s(curwin)).offset(idx as isize)).sp_type == SPTYPE_START {
                    put_pattern(
                        c"start".as_ptr(),
                        b'=' as c_int,
                        syn_items(win_s(curwin)).offset(idx as isize),
                        hl_id,
                    );
                    idx += 1;
                }
                if (*syn_items(win_s(curwin)).offset(idx as isize)).sp_type == SPTYPE_SKIP {
                    put_pattern(
                        c"skip".as_ptr(),
                        b'=' as c_int,
                        syn_items(win_s(curwin)).offset(idx as isize),
                        hl_id,
                    );
                    idx += 1;
                }
                while idx < (*win_s(curwin)).b_syn_patterns.ga_len
                    && (*syn_items(win_s(curwin)).offset(idx as isize)).sp_type == SPTYPE_END
                {
                    put_pattern(
                        c"end".as_ptr(),
                        b'=' as c_int,
                        syn_items(win_s(curwin)).offset(idx as isize),
                        hl_id,
                    );
                    idx += 1;
                }
                idx -= 1;
                msg_putchar(b' ' as c_int);
            }
            syn_list_flags(&NAMELIST1, (*spp).sp_flags, hl_id);

            if !(*spp).sp_cont_list.is_null() {
                put_id_list(c"contains".as_ptr(), (*spp).sp_cont_list, hl_id);
            }

            if !(*spp).sp_syn.cont_in_list.is_null() {
                put_id_list(c"containedin".as_ptr(), (*spp).sp_syn.cont_in_list, hl_id);
            }

            if !(*spp).sp_next_list.is_null() {
                put_id_list(c"nextgroup".as_ptr(), (*spp).sp_next_list, hl_id);
                syn_list_flags(&NAMELIST2, (*spp).sp_flags, hl_id);
            }
            if ((*spp).sp_flags & (HL_SYNC_HERE | HL_SYNC_THERE)) != 0 {
                if ((*spp).sp_flags & HL_SYNC_HERE) != 0 {
                    msg_puts_hl(c"grouphere".as_ptr(), hl_id, false);
                } else {
                    msg_puts_hl(c"groupthere".as_ptr(), hl_id, false);
                }
                msg_putchar(b' ' as c_int);
                if (*spp).sp_sync_idx >= 0 {
                    let sync_id = (*syn_items(win_s(curwin)).offset((*spp).sp_sync_idx as isize))
                        .sp_syn
                        .id as c_int;
                    msg_outtrans(highlight_group_name(sync_id - 1), 0, false);
                } else {
                    msg_puts(c"NONE".as_ptr());
                }
                msg_putchar(b' ' as c_int);
            }
            idx += 1;
        }

        if highlight_link_id(id - 1) != 0 && (did_header || link_only) && !got_int {
            syn_list_header(did_header, 0, id, true);
            msg_puts_hl(c"links to".as_ptr(), hl_id, false);
            msg_putchar(b' ' as c_int);
            msg_outtrans(
                highlight_group_name(highlight_link_id(id - 1) - 1),
                0,
                false,
            );
        }
    }
}

unsafe fn syn_list_flags(nlist: &[(c_int, *const c_char)], flags: c_int, hl_id: c_int) {
    unsafe {
        for &(key, value) in nlist {
            if (flags & key) != 0 {
                msg_puts_hl(value, hl_id, false);
                msg_putchar(b' ' as c_int);
            }
        }
    }
}

unsafe fn syn_list_cluster(id: c_int) {
    unsafe {
        let mut endcol: c_int = 15;

        msg_putchar(b'\n' as c_int);
        let clp = syn_clstr(win_s(curwin)).offset(id as isize);
        msg_outtrans((*clp).scl_name, 0, false);

        if msg_col >= endcol {
            endcol = msg_col + 1;
        }
        if Columns <= endcol {
            endcol = Columns - 1;
        }

        msg_advance(endcol);
        if !(*clp).scl_list.is_null() {
            put_id_list(c"cluster".as_ptr(), (*clp).scl_list, HLF_D);
        } else {
            msg_puts_hl(c"cluster".as_ptr(), HLF_D, false);
            msg_puts(c"=NONE".as_ptr());
        }
    }
}

unsafe fn put_id_list(name: *const c_char, list: *const i16, hl_id: c_int) {
    unsafe {
        msg_puts_hl(name, hl_id, false);
        msg_putchar(b'=' as c_int);
        let mut p = list;
        while *p != 0 {
            if *p >= SYNID_ALLBUT && *p < SYNID_TOP {
                if *p.add(1) != 0 {
                    msg_puts(c"ALLBUT".as_ptr());
                } else {
                    msg_puts(c"ALL".as_ptr());
                }
            } else if *p >= SYNID_TOP && *p < SYNID_CONTAINED {
                msg_puts(c"TOP".as_ptr());
            } else if *p >= SYNID_CONTAINED && *p < SYNID_CLUSTER {
                msg_puts(c"CONTAINED".as_ptr());
            } else if *p >= SYNID_CLUSTER {
                let scl_id = (*p - SYNID_CLUSTER) as isize;
                msg_putchar(b'@' as c_int);
                msg_outtrans(
                    (*syn_clstr(win_s(curwin)).offset(scl_id)).scl_name,
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

unsafe fn put_pattern(s: *const c_char, c: c_int, spp: *const synpat_T, hl_id: c_int) {
    unsafe {
        const SEPCHARS: &[u8] = b"/+=-#@\"|'^&\0";

        if last_matchgroup != (*spp).sp_syn_match_id as c_int {
            last_matchgroup = (*spp).sp_syn_match_id as c_int;
            msg_puts_hl(c"matchgroup".as_ptr(), hl_id, false);
            msg_putchar(b'=' as c_int);
            if last_matchgroup == 0 {
                msg_outtrans(c"NONE".as_ptr(), 0, false);
            } else {
                msg_outtrans(highlight_group_name(last_matchgroup - 1), 0, false);
            }
            msg_putchar(b' ' as c_int);
        }

        msg_puts_hl(s, hl_id, false);
        msg_putchar(c);

        let mut i: usize = 0;
        while !vim_strchr((*spp).sp_pattern, SEPCHARS[i] as c_int).is_null() {
            i += 1;
            if SEPCHARS[i] == 0 {
                i = 0;
                break;
            }
        }
        msg_putchar(SEPCHARS[i] as c_int);
        msg_outtrans((*spp).sp_pattern, 0, false);
        msg_putchar(SEPCHARS[i] as c_int);

        let mut first = true;
        for (j, &spo_name) in spo_name_tab.iter().enumerate().take(SPO_COUNT) {
            let mask: i16 = 1 << j;
            if ((*spp).sp_off_flags & (mask + (mask << SPO_COUNT))) == 0 {
                continue;
            }
            if !first {
                msg_putchar(b',' as c_int);
            }
            msg_puts(spo_name);
            let n = (*spp).sp_offsets[j];
            if j != SPO_LC_OFF {
                if ((*spp).sp_off_flags & mask) != 0 {
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
}

unsafe fn syn_list_keywords(
    id: c_int,
    ht: *const hashtab_T,
    mut did_header: bool,
    hl_id: c_int,
) -> bool {
    unsafe {
        let mut prev_contained: c_int = 0;
        let mut prev_next_list: *const i16 = ptr::null();
        let mut prev_cont_in_list: *const i16 = ptr::null();
        let mut prev_skipnl: c_int = 0;
        let mut prev_skipwhite: c_int = 0;
        let mut prev_skipempty: c_int = 0;

        let mut todo = (*ht).ht_used;
        let mut hi: *const hashitem_T = (*ht).ht_array;
        while todo > 0 && !got_int {
            if !hi_empty(hi) {
                todo -= 1;
                let mut kp = hi2ke(hi);
                while !kp.is_null() && !got_int {
                    if (*kp).k_syn.id as c_int == id {
                        let mut outlen: c_int = 0;
                        let mut force_newline = false;
                        if prev_contained != ((*kp).flags & HL_CONTAINED)
                            || prev_skipnl != ((*kp).flags & HL_SKIPNL)
                            || prev_skipwhite != ((*kp).flags & HL_SKIPWHITE)
                            || prev_skipempty != ((*kp).flags & HL_SKIPEMPTY)
                            || prev_cont_in_list != (*kp).k_syn.cont_in_list
                            || prev_next_list != (*kp).next_list
                        {
                            force_newline = true;
                        } else {
                            outlen = strlen(ke2hikey(kp)) as c_int;
                        }
                        if syn_list_header(did_header, outlen, id, force_newline) {
                            prev_contained = 0;
                            prev_next_list = ptr::null();
                            prev_cont_in_list = ptr::null();
                            prev_skipnl = 0;
                            prev_skipwhite = 0;
                            prev_skipempty = 0;
                        }
                        did_header = true;
                        if prev_contained != ((*kp).flags & HL_CONTAINED) {
                            msg_puts_hl(c"contained".as_ptr(), hl_id, false);
                            msg_putchar(b' ' as c_int);
                            prev_contained = (*kp).flags & HL_CONTAINED;
                        }
                        if !ptr::eq((*kp).k_syn.cont_in_list, prev_cont_in_list) {
                            put_id_list(c"containedin".as_ptr(), (*kp).k_syn.cont_in_list, hl_id);
                            msg_putchar(b' ' as c_int);
                            prev_cont_in_list = (*kp).k_syn.cont_in_list;
                        }
                        if !ptr::eq((*kp).next_list, prev_next_list) {
                            put_id_list(c"nextgroup".as_ptr(), (*kp).next_list, hl_id);
                            msg_putchar(b' ' as c_int);
                            prev_next_list = (*kp).next_list;
                            if ((*kp).flags & HL_SKIPNL) != 0 {
                                msg_puts_hl(c"skipnl".as_ptr(), hl_id, false);
                                msg_putchar(b' ' as c_int);
                                prev_skipnl = (*kp).flags & HL_SKIPNL;
                            }
                            if ((*kp).flags & HL_SKIPWHITE) != 0 {
                                msg_puts_hl(c"skipwhite".as_ptr(), hl_id, false);
                                msg_putchar(b' ' as c_int);
                                prev_skipwhite = (*kp).flags & HL_SKIPWHITE;
                            }
                            if ((*kp).flags & HL_SKIPEMPTY) != 0 {
                                msg_puts_hl(c"skipempty".as_ptr(), hl_id, false);
                                msg_putchar(b' ' as c_int);
                                prev_skipempty = (*kp).flags & HL_SKIPEMPTY;
                            }
                        }
                        msg_outtrans(ke2hikey(kp), 0, false);
                    }
                    kp = (*kp).ke_next;
                }
            }
            hi = hi.add(1);
        }

        did_header
    }
}

unsafe fn syn_clear_keyword(id: c_int, ht: *mut hashtab_T) {
    unsafe {
        hash_lock(ht);
        let mut todo = (*ht).ht_used as c_int;
        let mut hi = (*ht).ht_array;
        while todo > 0 {
            if !hi_empty(hi) {
                todo -= 1;
                let mut kp_prev: *mut keyentry_T = ptr::null_mut();
                let mut kp = hi2ke(hi);
                while !kp.is_null() {
                    if (*kp).k_syn.id as c_int == id {
                        let kp_next = (*kp).ke_next;
                        if kp_prev.is_null() {
                            if kp_next.is_null() {
                                hash_remove(ht, hi);
                            } else {
                                (*hi).hi_key = ke2hikey(kp_next);
                            }
                        } else {
                            (*kp_prev).ke_next = kp_next;
                        }
                        xfree((*kp).next_list.cast());
                        xfree((*kp).k_syn.cont_in_list.cast());
                        xfree(kp.cast());
                        kp = kp_next;
                    } else {
                        kp_prev = kp;
                        kp = (*kp).ke_next;
                    }
                }
            }
            hi = hi.add(1);
        }
        hash_unlock(ht);
    }
}

unsafe fn clear_keywtab(ht: *mut hashtab_T) {
    unsafe {
        let mut todo = (*ht).ht_used as c_int;
        let mut hi = (*ht).ht_array;
        while todo > 0 {
            if !hi_empty(hi) {
                todo -= 1;
                let mut kp = hi2ke(hi);
                while !kp.is_null() {
                    let kp_next = (*kp).ke_next;
                    xfree((*kp).next_list.cast());
                    xfree((*kp).k_syn.cont_in_list.cast());
                    xfree(kp.cast());
                    kp = kp_next;
                }
            }
            hi = hi.add(1);
        }
        hash_clear(ht);
        hash_init(ht);
    }
}

unsafe fn add_keyword(
    name: *mut c_char,
    namelen: usize,
    id: c_int,
    flags: c_int,
    cont_in_list: *mut i16,
    next_list: *mut i16,
    conceal_char: c_int,
) {
    unsafe {
        let mut name_folded = [0 as c_char; MAXKEYWLEN + 1];
        let (name_ic, name_iclen): (*const c_char, usize) = if (*win_s(curwin)).b_syn_ic != 0 {
            let folded = str_foldcase(
                name,
                namelen as c_int,
                name_folded.as_mut_ptr(),
                (MAXKEYWLEN + 1) as c_int,
            );
            (folded, strlen(folded))
        } else {
            (name, namelen)
        };

        let kp = xmalloc(offset_of!(keyentry_T, keyword) + name_iclen + 1).cast::<keyentry_T>();
        strcpy(ke2hikey(kp), name_ic);
        (*kp).k_syn.id = id as i16;
        (*kp).k_syn.inc_tag = current_syn_inc_tag;
        (*kp).flags = flags;
        (*kp).k_char = conceal_char;
        (*kp).k_syn.cont_in_list = copy_id_list(cont_in_list);
        if !cont_in_list.is_null() {
            (*win_s(curwin)).b_syn_containedin = 1;
        }
        (*kp).next_list = copy_id_list(next_list);

        let hash: hash_T = hash_hash(ke2hikey(kp));
        let ht = if (*win_s(curwin)).b_syn_ic != 0 {
            &raw mut (*win_s(curwin)).b_keywtab_ic
        } else {
            &raw mut (*win_s(curwin)).b_keywtab
        };
        let hi = hash_lookup(ht, ke2hikey(kp), strlen(ke2hikey(kp)), hash);

        if hi_empty(hi) {
            (*kp).ke_next = ptr::null_mut();
            hash_add_item(ht, hi, ke2hikey(kp), hash);
        } else {
            (*kp).ke_next = hi2ke(hi);
            (*hi).hi_key = ke2hikey(kp);
        }
    }
}

unsafe fn get_group_name(arg: *mut c_char, name_end: *mut *mut c_char) -> *mut c_char {
    unsafe {
        *name_end = skiptowhite(arg);
        let rest = skipwhite(*name_end);
        if ends_excmd(*arg as c_int) != 0 || *rest == NUL {
            return ptr::null_mut();
        }
        rest
    }
}

unsafe fn get_syn_options(
    mut arg: *mut c_char,
    opt: *mut syn_opt_arg_T,
    conceal_char: *mut c_int,
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

        if (*win_s(curwin)).b_syn_conceal != 0 {
            (*opt).flags |= HL_CONCEAL;
        }

        loop {
            if strchr(FIRST_LETTERS, *arg as c_int).is_null() {
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
                    if (*opt).keyword
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
                if !(*opt).has_cont_list {
                    emsg(gettext(e_contains_argument_not_accepted_here));
                    return ptr::null_mut();
                }
                if get_id_list(&raw mut arg, 8, &raw mut (*opt).cont_list, skip != 0) == FAIL {
                    return ptr::null_mut();
                }
            } else if argtype == 2 {
                if get_id_list(&raw mut arg, 11, &raw mut (*opt).cont_in_list, skip != 0) == FAIL {
                    return ptr::null_mut();
                }
            } else if argtype == 3 {
                if get_id_list(&raw mut arg, 9, &raw mut (*opt).next_list, skip != 0) == FAIL {
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
                (*opt).flags |= flags;
                arg = skipwhite(arg.add(len));

                if flags == HL_SYNC_HERE || flags == HL_SYNC_THERE {
                    if (*opt).sync_idx.is_null() {
                        emsg(gettext(c"E393: group[t]here not accepted here".as_ptr()));
                        return ptr::null_mut();
                    }
                    let gname_start = arg;
                    arg = skiptowhite(arg);
                    if gname_start == arg {
                        return ptr::null_mut();
                    }
                    let gname = xstrnsave(gname_start, arg.offset_from(gname_start) as usize);
                    if strcmp(gname, c"NONE".as_ptr()) == 0 {
                        *(*opt).sync_idx = NONE_IDX;
                    } else {
                        let syn_id = syn_name2id(gname);
                        let mut i = (*win_s(curwin)).b_syn_patterns.ga_len - 1;
                        while i >= 0 {
                            let spp = syn_items(win_s(curwin)).offset(i as isize);
                            if (*spp).sp_syn.id as c_int == syn_id && (*spp).sp_type == SPTYPE_START
                            {
                                *(*opt).sync_idx = i;
                                break;
                            }
                            i -= 1;
                        }
                        if i < 0 {
                            semsg!(
                                gettext(c"E394: Didn't find region item for %s".as_ptr()),
                                gname,
                            );
                            xfree(gname.cast());
                            return ptr::null_mut();
                        }
                    }

                    xfree(gname.cast());
                    arg = skipwhite(arg);
                } else if flags == HL_FOLD && foldmethodIsSyntax(curwin) {
                    foldUpdateAll(curwin);
                }
            }
        }

        arg
    }
}

unsafe fn syn_incl_toplevel(id: c_int, flagsp: *mut c_int) {
    unsafe {
        if (*flagsp & HL_CONTAINED) != 0 || (*win_s(curwin)).b_syn_topgrp == 0 {
            return;
        }
        *flagsp |= HL_CONTAINED | HL_INCLUDED_TOPLEVEL;
        if (*win_s(curwin)).b_syn_topgrp >= SYNID_CLUSTER as c_int {
            let mut grp_list = xmalloc(2 * size_of::<i16>()).cast::<i16>();
            let tlg_id = (*win_s(curwin)).b_syn_topgrp - SYNID_CLUSTER as c_int;
            *grp_list.add(0) = id as i16;
            *grp_list.add(1) = 0;
            let clp = syn_clstr(win_s(curwin)).offset(tlg_id as isize);
            syn_combine_list(&raw mut (*clp).scl_list, &raw mut grp_list, CLUSTER_ADD);
        }
    }
}

unsafe fn syn_cmd_include(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg = (*eap).arg;
        let mut sgl_id: c_int = 1;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut errormsg: *const c_char = ptr::null();
        let mut source = false;

        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
            return;
        }

        if *arg == b'@' as c_char {
            arg = arg.add(1);
            let rest = get_group_name(arg, &raw mut group_name_end);
            if rest.is_null() {
                emsg(gettext(c"E397: Filename required".as_ptr()));
                return;
            }
            sgl_id = syn_check_cluster(arg, group_name_end.offset_from(arg) as c_int);
            if sgl_id == 0 {
                return;
            }
            (*eap).arg = rest;
        }

        (*eap).argt |= EX_XFILE | EX_NOSPC;
        separate_nextcmd(eap);
        if *(*eap).arg == b'<' as c_char
            || *(*eap).arg == b'$' as c_char
            || path_is_absolute((*eap).arg)
        {
            source = true;
            if expand_filename(eap, syn_cmdlinep, &raw mut errormsg) == FAIL {
                if !errormsg.is_null() {
                    emsg(errormsg);
                }
                return;
            }
        } else {
            #[cfg(windows)]
            path_to_slash((*eap).arg);
        }

        if running_syn_inc_tag >= MAX_SYN_INC_TAG {
            emsg(gettext(c"E847: Too many syntax includes".as_ptr()));
            return;
        }
        let prev_syn_inc_tag = current_syn_inc_tag;
        running_syn_inc_tag += 1;
        current_syn_inc_tag = running_syn_inc_tag;
        let prev_toplvl_grp = (*win_s(curwin)).b_syn_topgrp;
        (*win_s(curwin)).b_syn_topgrp = sgl_id;
        let res = if source {
            do_source((*eap).arg, false, DOSO_NONE, ptr::null_mut())
        } else {
            source_runtime((*eap).arg, DIP_ALL)
        };
        if res == FAIL {
            semsg!(gettext((&raw const e_notopen).cast()), (*eap).arg);
        }
        (*win_s(curwin)).b_syn_topgrp = prev_toplvl_grp;
        current_syn_inc_tag = prev_syn_inc_tag;
    }
}

unsafe fn syn_cmd_keyword(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = (*eap).arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut keyword_copy: *mut c_char = ptr::null_mut();
        let mut conceal_char: c_int = NUL as c_int;

        let mut rest = get_group_name(arg, &raw mut group_name_end);

        if !rest.is_null() {
            let syn_id = if (*eap).skip != 0 {
                -1
            } else {
                syn_check_group(arg, group_name_end.offset_from(arg) as usize)
            };
            if syn_id != 0 {
                keyword_copy = xmalloc(strlen(rest) + 1).cast::<c_char>();
            }
            if !keyword_copy.is_null() {
                let mut syn_opt_arg = syn_opt_arg_T {
                    flags: 0,
                    keyword: true,
                    sync_idx: ptr::null_mut(),
                    has_cont_list: false,
                    cont_list: ptr::null_mut(),
                    cont_in_list: ptr::null_mut(),
                    next_list: ptr::null_mut(),
                };

                let mut cnt: c_int = 0;
                let mut p = keyword_copy;
                while !rest.is_null() && ends_excmd(*rest as c_int) == 0 {
                    rest = get_syn_options(
                        rest,
                        &raw mut syn_opt_arg,
                        &raw mut conceal_char,
                        (*eap).skip,
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

                if (*eap).skip == 0 {
                    syn_incl_toplevel(syn_id, &raw mut syn_opt_arg.flags);

                    let mut kw = keyword_copy;
                    let mut kwlen: usize;
                    'kw_loop: while cnt > 0 {
                        cnt -= 1;
                        p = vim_strchr(kw, b'[' as c_int);
                        loop {
                            if p.is_null() {
                                kwlen = strlen(kw);
                            } else {
                                *p = NUL;
                                kwlen = p.offset_from(kw) as usize;
                            }
                            add_keyword(
                                kw,
                                kwlen,
                                syn_id,
                                syn_opt_arg.flags,
                                syn_opt_arg.cont_in_list,
                                syn_opt_arg.next_list,
                                conceal_char,
                            );
                            if p.is_null() {
                                break;
                            }
                            if *p.add(1) == NUL {
                                semsg!(gettext(c"E789: Missing ']': %s".as_ptr()), kw);
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

                xfree(keyword_copy.cast());
                xfree(syn_opt_arg.cont_in_list.cast());
                xfree(syn_opt_arg.next_list.cast());
            }
        }

        if !rest.is_null() {
            (*eap).nextcmd = check_nextcmd(rest);
        } else {
            semsg!(gettext((&raw const e_invarg2).cast()), arg);
        }

        redraw_curbuf_later(UPD_SOME_VALID);
        syn_stack_free_all(win_s(curwin));
    }
}

unsafe fn syn_cmd_match(eap: *mut exarg_T, syncing: c_int) {
    unsafe {
        let arg = (*eap).arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut item: synpat_T = std::mem::zeroed();
        let mut sync_idx: c_int = 0;
        let mut conceal_char: c_int = NUL as c_int;

        let mut rest = get_group_name(arg, &raw mut group_name_end);

        let mut syn_opt_arg = syn_opt_arg_T {
            flags: 0,
            keyword: false,
            sync_idx: if syncing != 0 {
                &raw mut sync_idx
            } else {
                ptr::null_mut()
            },
            has_cont_list: true,
            cont_list: ptr::null_mut(),
            cont_in_list: ptr::null_mut(),
            next_list: ptr::null_mut(),
        };
        rest = get_syn_options(
            rest,
            &raw mut syn_opt_arg,
            &raw mut conceal_char,
            (*eap).skip,
        );

        init_syn_patterns();
        rest = get_syn_pattern(rest, &raw mut item);
        if vim_regcomp_had_eol() != 0 && (syn_opt_arg.flags & HL_EXCLUDENL) == 0 {
            syn_opt_arg.flags |= HL_HAS_EOL;
        }

        rest = get_syn_options(
            rest,
            &raw mut syn_opt_arg,
            &raw mut conceal_char,
            (*eap).skip,
        );

        if !rest.is_null() {
            (*eap).nextcmd = check_nextcmd(rest);
            if ends_excmd(*rest as c_int) == 0 || (*eap).skip != 0 {
                rest = ptr::null_mut();
            } else {
                let syn_id = syn_check_group(arg, group_name_end.offset_from(arg) as usize);
                if syn_id != 0 {
                    syn_incl_toplevel(syn_id, &raw mut syn_opt_arg.flags);
                    ga_grow(&raw mut (*win_s(curwin)).b_syn_patterns, 1);
                    let spp = syn_items(win_s(curwin))
                        .offset((*win_s(curwin)).b_syn_patterns.ga_len as isize);
                    (*win_s(curwin)).b_syn_patterns.ga_len += 1;
                    *spp = item;
                    (*spp).sp_syncing = syncing != 0;
                    (*spp).sp_type = SPTYPE_MATCH;
                    (*spp).sp_syn.id = syn_id as i16;
                    (*spp).sp_syn.inc_tag = current_syn_inc_tag;
                    (*spp).sp_flags = syn_opt_arg.flags;
                    (*spp).sp_sync_idx = sync_idx;
                    (*spp).sp_cont_list = syn_opt_arg.cont_list;
                    (*spp).sp_syn.cont_in_list = syn_opt_arg.cont_in_list;
                    (*spp).sp_cchar = conceal_char;
                    if !syn_opt_arg.cont_in_list.is_null() {
                        (*win_s(curwin)).b_syn_containedin = 1;
                    }
                    (*spp).sp_next_list = syn_opt_arg.next_list;

                    if (syn_opt_arg.flags & (HL_SYNC_HERE | HL_SYNC_THERE)) != 0 {
                        (*win_s(curwin)).b_syn_sync_flags |= SF_MATCH;
                    }
                    if (syn_opt_arg.flags & HL_FOLD) != 0 {
                        (*win_s(curwin)).b_syn_folditems += 1;
                    }

                    redraw_curbuf_later(UPD_SOME_VALID);
                    syn_stack_free_all(win_s(curwin));
                    return;
                }
            }
        }

        vim_regfree(item.sp_prog);
        xfree(item.sp_pattern.cast());
        xfree(syn_opt_arg.cont_list.cast());
        xfree(syn_opt_arg.cont_in_list.cast());
        xfree(syn_opt_arg.next_list.cast());

        if rest.is_null() {
            semsg!(gettext((&raw const e_invarg2).cast()), arg);
        }
    }
}

unsafe fn syn_cmd_region(eap: *mut exarg_T, syncing: c_int) {
    unsafe {
        let arg = (*eap).arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut key: *mut c_char = ptr::null_mut();
        let mut pat_ptrs = [ptr::null_mut::<pat_ptr_T>(); 3];
        let mut pat_count: c_int = 0;
        let mut matchgroup_id: c_int = 0;
        let mut not_enough = false;
        let mut illegal = false;
        let mut success = false;
        let mut conceal_char: c_int = NUL as c_int;

        let mut rest = get_group_name(arg, &raw mut group_name_end);

        init_syn_patterns();

        let mut syn_opt_arg = syn_opt_arg_T {
            flags: 0,
            keyword: false,
            sync_idx: ptr::null_mut(),
            has_cont_list: true,
            cont_list: ptr::null_mut(),
            cont_in_list: ptr::null_mut(),
            next_list: ptr::null_mut(),
        };

        while !rest.is_null() && ends_excmd(*rest as c_int) == 0 {
            rest = get_syn_options(
                rest,
                &raw mut syn_opt_arg,
                &raw mut conceal_char,
                (*eap).skip,
            );
            if rest.is_null() || ends_excmd(*rest as c_int) != 0 {
                break;
            }

            let mut key_end = rest;
            while *key_end != NUL && !ascii_iswhite(*key_end) && *key_end != b'=' as c_char {
                key_end = key_end.add(1);
            }
            xfree(key.cast());
            key = vim_strnsave_up(rest, key_end.offset_from(rest) as usize);
            let item = if strcmp(key, c"MATCHGROUP".as_ptr()) == 0 {
                ITEM_MATCHGROUP
            } else if strcmp(key, c"START".as_ptr()) == 0 {
                ITEM_START
            } else if strcmp(key, c"END".as_ptr()) == 0 {
                ITEM_END
            } else if strcmp(key, c"SKIP".as_ptr()) == 0 {
                if !pat_ptrs[ITEM_SKIP].is_null() {
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
                semsg!(gettext(c"E398: Missing '=': %s".as_ptr()), arg);
                break;
            }
            rest = skipwhite(rest.add(1));
            if *rest == NUL {
                not_enough = true;
                break;
            }

            if item == ITEM_MATCHGROUP {
                let p = skiptowhite(rest);
                if (p.offset_from(rest) == 4 && strncmp(rest, c"NONE".as_ptr(), 4) == 0)
                    || (*eap).skip != 0
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
                let ppp = xmalloc(size_of::<pat_ptr_T>()).cast::<pat_ptr_T>();
                (*ppp).pp_next = pat_ptrs[item];
                pat_ptrs[item] = ppp;
                (*ppp).pp_synp = xcalloc(1, size_of::<synpat_T>()).cast::<synpat_T>();

                if item == ITEM_START {
                    reg_do_extmatch = REX_SET;
                } else {
                    reg_do_extmatch = REX_USE;
                }
                rest = get_syn_pattern(rest, (*ppp).pp_synp);
                reg_do_extmatch = 0;
                if item == ITEM_END
                    && vim_regcomp_had_eol() != 0
                    && (syn_opt_arg.flags & HL_EXCLUDENL) == 0
                {
                    (*(*ppp).pp_synp).sp_flags |= HL_HAS_EOL;
                }
                (*ppp).pp_matchgroup_id = matchgroup_id;
                pat_count += 1;
            }
        }
        xfree(key.cast());
        if illegal || not_enough {
            rest = ptr::null_mut();
        }

        if !rest.is_null() && (pat_ptrs[ITEM_START].is_null() || pat_ptrs[ITEM_END].is_null()) {
            not_enough = true;
            rest = ptr::null_mut();
        }

        if !rest.is_null() {
            (*eap).nextcmd = check_nextcmd(rest);
            if ends_excmd(*rest as c_int) == 0 || (*eap).skip != 0 {
                rest = ptr::null_mut();
            } else {
                ga_grow(&raw mut (*win_s(curwin)).b_syn_patterns, pat_count);
                let syn_id = syn_check_group(arg, group_name_end.offset_from(arg) as usize);
                if syn_id != 0 {
                    syn_incl_toplevel(syn_id, &raw mut syn_opt_arg.flags);
                    let mut idx = (*win_s(curwin)).b_syn_patterns.ga_len;
                    for (item, &pat_ptr) in pat_ptrs
                        .iter()
                        .enumerate()
                        .take(ITEM_END + 1)
                        .skip(ITEM_START)
                    {
                        let mut ppp = pat_ptr;
                        while !ppp.is_null() {
                            let spp = syn_items(win_s(curwin)).offset(idx as isize);
                            *spp = *(*ppp).pp_synp;
                            (*spp).sp_syncing = syncing != 0;
                            (*spp).sp_type = if item == ITEM_START {
                                SPTYPE_START
                            } else if item == ITEM_SKIP {
                                SPTYPE_SKIP
                            } else {
                                SPTYPE_END
                            };
                            (*spp).sp_flags |= syn_opt_arg.flags;
                            (*spp).sp_syn.id = syn_id as i16;
                            (*spp).sp_syn.inc_tag = current_syn_inc_tag;
                            (*spp).sp_syn_match_id = (*ppp).pp_matchgroup_id as i16;
                            (*spp).sp_cchar = conceal_char;
                            if item == ITEM_START {
                                (*spp).sp_cont_list = syn_opt_arg.cont_list;
                                (*spp).sp_syn.cont_in_list = syn_opt_arg.cont_in_list;
                                if !syn_opt_arg.cont_in_list.is_null() {
                                    (*win_s(curwin)).b_syn_containedin = 1;
                                }
                                (*spp).sp_next_list = syn_opt_arg.next_list;
                            }
                            (*win_s(curwin)).b_syn_patterns.ga_len += 1;
                            idx += 1;
                            if (syn_opt_arg.flags & HL_FOLD) != 0 {
                                (*win_s(curwin)).b_syn_folditems += 1;
                            }
                            ppp = (*ppp).pp_next;
                        }
                    }

                    redraw_curbuf_later(UPD_SOME_VALID);
                    syn_stack_free_all(win_s(curwin));
                    success = true;
                }
            }
        }

        for &pat_ptr in pat_ptrs.iter().take(ITEM_END + 1).skip(ITEM_START) {
            let mut ppp = pat_ptr;
            while !ppp.is_null() {
                let ppp_next = (*ppp).pp_next;
                if !success && !(*ppp).pp_synp.is_null() {
                    vim_regfree((*(*ppp).pp_synp).sp_prog);
                    xfree((*(*ppp).pp_synp).sp_pattern.cast());
                }
                xfree((*ppp).pp_synp.cast());
                xfree(ppp.cast());
                ppp = ppp_next;
            }
        }

        if !success {
            xfree(syn_opt_arg.cont_list.cast());
            xfree(syn_opt_arg.cont_in_list.cast());
            xfree(syn_opt_arg.next_list.cast());
            if not_enough {
                semsg!(
                    gettext(c"E399: Not enough arguments: syntax region %s".as_ptr()),
                    arg,
                );
            } else if illegal || rest.is_null() {
                semsg!(gettext((&raw const e_invarg2).cast()), arg);
            }
        }
    }
}

unsafe fn syn_combine_list(clstr1: *mut *mut i16, clstr2: *mut *mut i16, list_op: c_int) {
    unsafe {
        let mut count1: usize = 0;
        let mut count2: usize = 0;
        let mut clstr: *mut i16 = ptr::null_mut();

        if (*clstr2).is_null() {
            return;
        }
        if (*clstr1).is_null() || list_op == CLUSTER_REPLACE {
            if list_op == CLUSTER_REPLACE {
                xfree((*clstr1).cast());
            }
            if list_op == CLUSTER_REPLACE || list_op == CLUSTER_ADD {
                *clstr1 = *clstr2;
            } else {
                xfree((*clstr2).cast());
            }
            return;
        }

        let mut g1: *const i16 = *clstr1;
        while *g1 != 0 {
            count1 += 1;
            g1 = g1.add(1);
        }
        let mut g2: *const i16 = *clstr2;
        while *g2 != 0 {
            count2 += 1;
            g2 = g2.add(1);
        }

        std::slice::from_raw_parts_mut(*clstr1, count1).sort_unstable();
        std::slice::from_raw_parts_mut(*clstr2, count2).sort_unstable();

        for round in 1..=2 {
            g1 = *clstr1;
            g2 = *clstr2;
            let mut count: usize = 0;

            while *g1 != 0 && *g2 != 0 {
                if *g1 < *g2 {
                    if round == 2 {
                        *clstr.add(count) = *g1;
                    }
                    count += 1;
                    g1 = g1.add(1);
                    continue;
                }
                if list_op == CLUSTER_ADD {
                    if round == 2 {
                        *clstr.add(count) = *g2;
                    }
                    count += 1;
                }
                if *g1 == *g2 {
                    g1 = g1.add(1);
                }
                g2 = g2.add(1);
            }

            while *g1 != 0 {
                if round == 2 {
                    *clstr.add(count) = *g1;
                }
                g1 = g1.add(1);
                count += 1;
            }
            if list_op == CLUSTER_ADD {
                while *g2 != 0 {
                    if round == 2 {
                        *clstr.add(count) = *g2;
                    }
                    g2 = g2.add(1);
                    count += 1;
                }
            }

            if round == 1 {
                if count == 0 {
                    clstr = ptr::null_mut();
                    break;
                }
                clstr = xmalloc((count + 1) * size_of::<i16>()).cast::<i16>();
                *clstr.add(count) = 0;
            }
        }

        xfree((*clstr1).cast());
        xfree((*clstr2).cast());
        *clstr1 = clstr;
    }
}

unsafe fn syn_scl_name2id(name: *const c_char) -> c_int {
    unsafe {
        let name_u = vim_strsave_up(name);
        let mut i = (*win_s(curwin)).b_syn_clusters.ga_len - 1;
        while i >= 0 {
            let clp = syn_clstr(win_s(curwin)).offset(i as isize);
            if !(*clp).scl_name_u.is_null() && strcmp(name_u, (*clp).scl_name_u) == 0 {
                break;
            }
            i -= 1;
        }
        xfree(name_u.cast());
        if i < 0 { 0 } else { i + SYNID_CLUSTER as c_int }
    }
}

unsafe fn syn_scl_namen2id(linep: *const c_char, len: c_int) -> c_int {
    unsafe {
        let name = xstrnsave(linep, len as usize);
        let id = syn_scl_name2id(name);
        xfree(name.cast());
        id
    }
}

unsafe fn syn_check_cluster(pp: *const c_char, len: c_int) -> c_int {
    unsafe {
        let name = xstrnsave(pp, len as usize);
        let mut id = syn_scl_name2id(name);
        if id == 0 {
            id = syn_add_cluster(name);
        } else {
            xfree(name.cast());
        }
        id
    }
}

unsafe fn syn_add_cluster(name: *mut c_char) -> c_int {
    unsafe {
        if (*win_s(curwin)).b_syn_clusters.ga_data.is_null() {
            (*win_s(curwin)).b_syn_clusters.ga_itemsize = size_of::<syn_cluster_T>() as c_int;
            ga_set_growsize(&raw mut (*win_s(curwin)).b_syn_clusters, 10);
        }

        let len = (*win_s(curwin)).b_syn_clusters.ga_len;
        if len >= MAX_CLUSTER_ID {
            emsg(gettext(c"E848: Too many syntax clusters".as_ptr()));
            xfree(name.cast());
            return 0;
        }

        ga_grow(&raw mut (*win_s(curwin)).b_syn_clusters, 1);
        let scp = syn_clstr(win_s(curwin)).offset(len as isize);
        (*win_s(curwin)).b_syn_clusters.ga_len += 1;
        ptr::write_bytes(scp, 0, 1);
        (*scp).scl_name = name;
        (*scp).scl_name_u = vim_strsave_up(name);
        (*scp).scl_list = ptr::null_mut();

        if vim_stricmp(name, c"Spell".as_ptr()) == 0 {
            (*win_s(curwin)).b_spell_cluster_id = len + SYNID_CLUSTER as c_int;
        }
        if vim_stricmp(name, c"NoSpell".as_ptr()) == 0 {
            (*win_s(curwin)).b_nospell_cluster_id = len + SYNID_CLUSTER as c_int;
        }

        len + SYNID_CLUSTER as c_int
    }
}

unsafe fn syn_cmd_cluster(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let arg = (*eap).arg;
        let mut group_name_end: *mut c_char = ptr::null_mut();
        let mut got_clstr = false;

        (*eap).nextcmd = find_nextcmd(arg);
        if (*eap).skip != 0 {
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
                let (opt_len, list_op) = if vim_strnicmp(rest, c"add".as_ptr(), 3) == 0
                    && (ascii_iswhite(*rest.add(3)) || *rest.add(3) == b'=' as c_char)
                {
                    (3, CLUSTER_ADD)
                } else if vim_strnicmp(rest, c"remove".as_ptr(), 6) == 0
                    && (ascii_iswhite(*rest.add(6)) || *rest.add(6) == b'=' as c_char)
                {
                    (6, CLUSTER_SUBTRACT)
                } else if vim_strnicmp(rest, c"contains".as_ptr(), 8) == 0
                    && (ascii_iswhite(*rest.add(8)) || *rest.add(8) == b'=' as c_char)
                {
                    (8, CLUSTER_REPLACE)
                } else {
                    break;
                };

                let mut clstr_list: *mut i16 = ptr::null_mut();
                if get_id_list(
                    &raw mut rest,
                    opt_len,
                    &raw mut clstr_list,
                    (*eap).skip != 0,
                ) == FAIL
                {
                    semsg!(gettext((&raw const e_invarg2).cast()), rest);
                    break;
                }
                if scl_id >= 0 {
                    let clp = syn_clstr(win_s(curwin)).offset(scl_id as isize);
                    syn_combine_list(&raw mut (*clp).scl_list, &raw mut clstr_list, list_op);
                } else {
                    xfree(clstr_list.cast());
                }
                got_clstr = true;
            }

            if got_clstr {
                redraw_curbuf_later(UPD_SOME_VALID);
                syn_stack_free_all(win_s(curwin));
            }
        }

        if !got_clstr {
            emsg(gettext(c"E400: No cluster specified".as_ptr()));
        }
        if rest.is_null() || ends_excmd(*rest as c_int) == 0 {
            semsg!(gettext((&raw const e_invarg2).cast()), arg);
        }
    }
}

unsafe fn init_syn_patterns() {
    unsafe {
        (*win_s(curwin)).b_syn_patterns.ga_itemsize = size_of::<synpat_T>() as c_int;
        ga_set_growsize(&raw mut (*win_s(curwin)).b_syn_patterns, 10);
    }
}

unsafe fn get_syn_pattern(arg: *mut c_char, ci: *mut synpat_T) -> *mut c_char {
    unsafe {
        if arg.is_null() || *arg == NUL || *arg.add(1) == NUL || *arg.add(2) == NUL {
            return ptr::null_mut();
        }

        let mut end = skip_regexp(arg.add(1), *arg as c_int, 1);
        if *end != *arg {
            semsg!(
                gettext(c"E401: Pattern delimiter not found: %s".as_ptr()),
                arg,
            );
            return ptr::null_mut();
        }
        (*ci).sp_pattern = xstrnsave(arg.add(1), end.offset_from(arg) as usize - 1);

        let cpo_save = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();
        (*ci).sp_prog = vim_regcomp((*ci).sp_pattern, RE_MAGIC);
        p_cpo = cpo_save;

        if (*ci).sp_prog.is_null() {
            return ptr::null_mut();
        }
        (*ci).sp_ic = (*win_s(curwin)).b_syn_ic;
        syn_clear_time(&raw mut (*ci).sp_time);

        end = end.add(1);
        loop {
            let mut idx: isize = SPO_COUNT as isize - 1;
            while idx >= 0 {
                if strncmp(end, spo_name_tab[idx as usize], 3) == 0 {
                    break;
                }
                idx -= 1;
            }
            if idx >= 0 {
                let base_idx = idx as usize;
                let p = &raw mut (*ci).sp_offsets[base_idx];
                if base_idx != SPO_LC_OFF {
                    match *end.add(3) as u8 {
                        b's' | b'b' => {}
                        b'e' => idx += SPO_COUNT as isize,
                        _ => idx = -1,
                    }
                }
                if idx >= 0 {
                    (*ci).sp_off_flags |= 1 << (idx as usize);
                    if base_idx == SPO_LC_OFF {
                        end = end.add(3);
                        *p = getdigits_int(&raw mut end, true, 0);

                        if ((*ci).sp_off_flags & (1 << SPO_MS_OFF)) == 0 {
                            (*ci).sp_off_flags |= 1 << SPO_MS_OFF;
                            (*ci).sp_offsets[SPO_MS_OFF] = *p;
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
            semsg!(gettext(c"E402: Garbage after pattern: %s".as_ptr()), arg);
            return ptr::null_mut();
        }
        skipwhite(end)
    }
}

unsafe fn syn_cmd_sync(eap: *mut exarg_T, _syncing: c_int) {
    unsafe {
        let mut arg_start = (*eap).arg;
        let mut key: *mut c_char = ptr::null_mut();
        let mut illegal = false;
        let mut finished = false;

        if ends_excmd(*arg_start as c_int) != 0 {
            syn_cmd_list(eap, 1);
            return;
        }

        while ends_excmd(*arg_start as c_int) == 0 {
            let mut arg_end = skiptowhite(arg_start);
            let mut next_arg = skipwhite(arg_end);
            xfree(key.cast());
            key = vim_strnsave_up(arg_start, arg_end.offset_from(arg_start) as usize);
            if strcmp(key, c"CCOMMENT".as_ptr()) == 0 {
                if (*eap).skip == 0 {
                    (*win_s(curwin)).b_syn_sync_flags |= SF_CCOMMENT;
                }
                if ends_excmd(*next_arg as c_int) == 0 {
                    arg_end = skiptowhite(next_arg);
                    if (*eap).skip == 0 {
                        (*win_s(curwin)).b_syn_sync_id =
                            syn_check_group(next_arg, arg_end.offset_from(next_arg) as usize)
                                as i16;
                    }
                    next_arg = skipwhite(arg_end);
                } else if (*eap).skip == 0 {
                    (*win_s(curwin)).b_syn_sync_id = syn_name2id(c"Comment".as_ptr()) as i16;
                }
            } else if strncmp(key, c"LINES".as_ptr(), 5) == 0
                || strncmp(key, c"MINLINES".as_ptr(), 8) == 0
                || strncmp(key, c"MAXLINES".as_ptr(), 8) == 0
                || strncmp(key, c"LINEBREAKS".as_ptr(), 10) == 0
            {
                arg_end = if *key.add(4) == b'S' as c_char {
                    key.add(6)
                } else if *key == b'L' as c_char {
                    key.add(11)
                } else {
                    key.add(9)
                };
                if *arg_end.sub(1) != b'=' as c_char || !ascii_isdigit(*arg_end) {
                    illegal = true;
                    break;
                }
                let n: linenr_T = getdigits_int32(&raw mut arg_end, false, 0);
                if (*eap).skip == 0 {
                    if *key.add(4) == b'B' as c_char {
                        (*win_s(curwin)).b_syn_sync_linebreaks = n;
                    } else if *key.add(1) == b'A' as c_char {
                        (*win_s(curwin)).b_syn_sync_maxlines = n;
                    } else {
                        (*win_s(curwin)).b_syn_sync_minlines = n;
                    }
                }
            } else if strcmp(key, c"FROMSTART".as_ptr()) == 0 {
                if (*eap).skip == 0 {
                    (*win_s(curwin)).b_syn_sync_minlines = MAXLNUM;
                    (*win_s(curwin)).b_syn_sync_maxlines = 0;
                }
            } else if strcmp(key, c"LINECONT".as_ptr()) == 0 {
                if *next_arg == NUL {
                    illegal = true;
                    break;
                }
                if !(*win_s(curwin)).b_syn_linecont_pat.is_null() {
                    emsg(gettext(
                        c"E403: syntax sync: line continuations pattern specified twice".as_ptr(),
                    ));
                    finished = true;
                    break;
                }
                arg_end = skip_regexp(next_arg.add(1), *next_arg as c_int, 1);
                if *arg_end != *next_arg {
                    illegal = true;
                    break;
                }

                if (*eap).skip == 0 {
                    (*win_s(curwin)).b_syn_linecont_pat =
                        xstrnsave(next_arg.add(1), arg_end.offset_from(next_arg) as usize - 1);
                    (*win_s(curwin)).b_syn_linecont_ic = (*win_s(curwin)).b_syn_ic;

                    let cpo_save = p_cpo;
                    p_cpo = (&raw mut empty_string_option).cast();
                    (*win_s(curwin)).b_syn_linecont_prog =
                        vim_regcomp((*win_s(curwin)).b_syn_linecont_pat, RE_MAGIC);
                    p_cpo = cpo_save;
                    syn_clear_time(&raw mut (*win_s(curwin)).b_syn_linecont_time);

                    if (*win_s(curwin)).b_syn_linecont_prog.is_null() {
                        xfree((*win_s(curwin)).b_syn_linecont_pat.cast());
                        (*win_s(curwin)).b_syn_linecont_pat = ptr::null_mut();
                        finished = true;
                        break;
                    }
                }
                next_arg = skipwhite(arg_end.add(1));
            } else {
                (*eap).arg = next_arg;
                if strcmp(key, c"MATCH".as_ptr()) == 0 {
                    syn_cmd_match(eap, 1);
                } else if strcmp(key, c"REGION".as_ptr()) == 0 {
                    syn_cmd_region(eap, 1);
                } else if strcmp(key, c"CLEAR".as_ptr()) == 0 {
                    syn_cmd_clear(eap, 1);
                } else {
                    illegal = true;
                }
                finished = true;
                break;
            }
            arg_start = next_arg;
        }
        xfree(key.cast());
        if illegal {
            semsg!(gettext(c"E404: Illegal arguments: %s".as_ptr()), arg_start);
        } else if !finished {
            (*eap).nextcmd = check_nextcmd(arg_start);
            redraw_curbuf_later(UPD_SOME_VALID);
            syn_stack_free_all(win_s(curwin));
        }
    }
}

unsafe fn get_id_list(
    arg: *mut *mut c_char,
    keylen: c_int,
    list: *mut *mut i16,
    skip: bool,
) -> c_int {
    unsafe {
        let mut p: *mut c_char = ptr::null_mut();
        let mut total_count: c_int = 0;
        let mut retval: *mut i16 = ptr::null_mut();
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let mut failed = false;

        let mut round: c_int = 1;
        while round <= 2 {
            p = skipwhite((*arg).offset(keylen as isize));
            if *p != b'=' as c_char {
                semsg!(gettext(c"E405: Missing equal sign: %s".as_ptr()), *arg);
                break;
            }
            p = skipwhite(p.add(1));
            if ends_excmd(*p as c_int) != 0 {
                semsg!(gettext(c"E406: Empty argument: %s".as_ptr()), *arg);
                break;
            }

            let mut count: c_int = 0;
            loop {
                let mut end = p;
                while *end != NUL && !ascii_iswhite(*end) && *end != b',' as c_char {
                    end = end.add(1);
                }
                let name = xmalloc(end.offset_from(p) as usize + 3).cast::<c_char>();
                xmemcpyz(name.add(1).cast(), p.cast(), end.offset_from(p) as usize);
                let mut id: c_int;
                if strcmp(name.add(1), c"ALLBUT".as_ptr()) == 0
                    || strcmp(name.add(1), c"ALL".as_ptr()) == 0
                    || strcmp(name.add(1), c"TOP".as_ptr()) == 0
                    || strcmp(name.add(1), c"CONTAINED".as_ptr()) == 0
                {
                    if !(**arg as u8).eq_ignore_ascii_case(&b'C') {
                        semsg!(gettext(c"E407: %s not allowed here".as_ptr()), name.add(1));
                        failed = true;
                        xfree(name.cast());
                        break;
                    }
                    if count != 0 {
                        semsg!(
                            gettext(c"E408: %s must be first in contains list".as_ptr()),
                            name.add(1),
                        );
                        failed = true;
                        xfree(name.cast());
                        break;
                    }
                    id = if *name.add(1) == b'A' as c_char {
                        SYNID_ALLBUT as c_int
                    } else if *name.add(1) == b'T' as c_char {
                        SYNID_TOP as c_int
                    } else {
                        SYNID_CONTAINED as c_int
                    };
                    id += current_syn_inc_tag;
                } else if *name.add(1) == b'@' as c_char {
                    if skip {
                        id = -1;
                    } else {
                        id = syn_check_cluster(name.add(2), end.offset_from(p) as c_int - 1);
                    }
                } else if strpbrk(name.add(1), c"\\.*^$~[".as_ptr()).is_null() {
                    id = syn_check_group(name.add(1), end.offset_from(p) as usize);
                } else {
                    *name = b'^' as c_char;
                    strcat(name.add(1), c"$".as_ptr());
                    regmatch.regprog = vim_regcomp(name, RE_MAGIC);
                    if regmatch.regprog.is_null() {
                        failed = true;
                        xfree(name.cast());
                        break;
                    }

                    regmatch.rm_ic = true;
                    id = 0;
                    let mut i = highlight_num_groups() - 1;
                    while i >= 0 {
                        if vim_regexec(&raw mut regmatch, highlight_group_name(i), 0) {
                            if round == 2 {
                                if count >= total_count {
                                    xfree(retval.cast());
                                    retval = ptr::null_mut();
                                    round = 1;
                                } else {
                                    *retval.offset(count as isize) = (i + 1) as i16;
                                }
                            }
                            count += 1;
                            id = -1;
                        }
                        i -= 1;
                    }
                    vim_regfree(regmatch.regprog);
                }
                xfree(name.cast());
                if id == 0 {
                    semsg!(gettext(c"E409: Unknown group name: %s".as_ptr()), p);
                    failed = true;
                    break;
                }
                if id > 0 {
                    if round == 2 {
                        if count >= total_count {
                            xfree(retval.cast());
                            retval = ptr::null_mut();
                            round = 1;
                        } else {
                            *retval.offset(count as isize) = id as i16;
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
                retval = xmalloc((count as usize + 1) * size_of::<i16>()).cast::<i16>();
                *retval.offset(count as isize) = 0;
                total_count = count;
            }
            round += 1;
        }

        *arg = p;
        if failed || retval.is_null() {
            xfree(retval.cast());
            return FAIL;
        }

        if (*list).is_null() {
            *list = retval;
        } else {
            xfree(retval.cast());
        }
        OK
    }
}

unsafe fn copy_id_list(list: *const i16) -> *mut i16 {
    unsafe {
        if list.is_null() {
            return ptr::null_mut();
        }

        let mut count: usize = 0;
        while *list.add(count) != 0 {
            count += 1;
        }
        let len = (count + 1) * size_of::<i16>();
        let retval = xmalloc(len).cast::<i16>();
        ptr::copy(list, retval, count + 1);
        retval
    }
}

unsafe fn idl_collect(list: *mut i16, ga: *mut garray_T, seen: *mut garray_T) -> bool {
    unsafe {
        if list.is_null() || list == id_list_all() {
            return false;
        }
        let mut cur = list;
        while *cur != 0 {
            let item = *cur;
            if (SYNID_ALLBUT..SYNID_CLUSTER).contains(&item) {
                return false;
            }
            if item >= SYNID_CLUSTER {
                let mut seen_it = false;
                for i in 0..(*seen).ga_len {
                    if *(*seen).ga_data.cast::<i16>().offset(i as isize) == item {
                        seen_it = true;
                        break;
                    }
                }
                if !seen_it {
                    ga_grow(seen, 1);
                    *(*seen)
                        .ga_data
                        .cast::<i16>()
                        .offset((*seen).ga_len as isize) = item;
                    (*seen).ga_len += 1;

                    ga_grow(ga, 1);
                    *(*ga).ga_data.cast::<i16>().offset((*ga).ga_len as isize) = item;
                    (*ga).ga_len += 1;

                    let scl_list =
                        (*syn_clstr(syn_block).offset((item - SYNID_CLUSTER) as isize)).scl_list;
                    if !scl_list.is_null() && !idl_collect(scl_list, ga, seen) {
                        return false;
                    }
                }
            } else {
                ga_grow(ga, 1);
                *(*ga).ga_data.cast::<i16>().offset((*ga).ga_len as isize) = item;
                (*ga).ga_len += 1;
            }
            cur = cur.add(1);
        }
        true
    }
}

unsafe fn idl_get_entry(mut list: *mut i16) -> *mut idl_entry_T {
    unsafe {
        let mut cache = (*syn_block).b_idlist_cache.cast::<idl_cache_T>();
        if cache.is_null() {
            cache = xcalloc(1, size_of::<idl_cache_T>()).cast::<idl_cache_T>();
            (*syn_block).b_idlist_cache = cache.cast();
        }
        let last = (*cache).last as usize;
        if (*cache).ent[last].idl_key == list {
            return &raw mut (*cache).ent[last];
        }
        for i in 0..IDL_CACHE_SIZE {
            if (*cache).ent[i].idl_key == list {
                (*cache).last = i as c_int;
                return &raw mut (*cache).ent[i];
            }
        }

        let round = (*cache).round as usize;
        (*cache).last = (*cache).round;
        let e = &raw mut (*cache).ent[round];
        (*cache).round = ((*cache).round + 1) % (IDL_CACHE_SIZE as c_int);
        xfree((*e).idl_ids.cast());
        (*e).idl_ids = ptr::null_mut();
        (*e).idl_count = 0;
        (*e).idl_key = list;
        (*e).idl_marker = 0;
        (*e).idl_usable = true;

        if *list >= SYNID_ALLBUT && *list < SYNID_CLUSTER {
            (*e).idl_marker = *list;
            list = list.add(1);
        }

        let mut ga: garray_T = std::mem::zeroed();
        let mut seen: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, size_of::<i16>() as c_int, 50);
        ga_init(&raw mut seen, size_of::<i16>() as c_int, 10);
        if !idl_collect(list, &raw mut ga, &raw mut seen) {
            (*e).idl_usable = false;
            ga_clear(&raw mut ga);
            ga_clear(&raw mut seen);
            return e;
        }
        ga_clear(&raw mut seen);
        if ga.ga_len > 0 {
            let slice =
                std::slice::from_raw_parts_mut(ga.ga_data.cast::<i16>(), ga.ga_len as usize);
            slice.sort_unstable();
            (*e).idl_ids = ga.ga_data.cast::<i16>();
            (*e).idl_count = ga.ga_len;
        } else {
            ga_clear(&raw mut ga);
        }
        e
    }
}

unsafe fn idl_contains(e: *const idl_entry_T, id: i16) -> bool {
    unsafe {
        let mut lo: c_int = 0;
        let mut hi = (*e).idl_count - 1;

        while lo <= hi {
            let mid = (lo + hi) / 2;
            let v = *(*e).idl_ids.offset(mid as isize);
            if v == id {
                return true;
            }
            if v < id {
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        false
    }
}

unsafe fn idl_cache_clear(block: *mut synblock_T) {
    unsafe {
        let cache = (*block).b_idlist_cache.cast::<idl_cache_T>();
        if cache.is_null() {
            return;
        }
        for i in 0..IDL_CACHE_SIZE {
            xfree((*cache).ent[i].idl_ids.cast());
        }
        xfree(cache.cast());
        (*block).b_idlist_cache = ptr::null_mut();
    }
}

unsafe fn in_id_list(
    mut cur_si: *mut stateitem_T,
    mut list: *mut i16,
    ssp: *mut sp_syn,
    flags: c_int,
) -> c_int {
    unsafe {
        let id = (*ssp).id;

        if !cur_si.is_null()
            && !(*ssp).cont_in_list.is_null()
            && ((*cur_si).si_flags & HL_MATCH) == 0
        {
            while ((*cur_si).si_flags & HL_TRANS_CONT) != 0
                && cur_si > current_state.ga_data.cast::<stateitem_T>()
            {
                cur_si = cur_si.sub(1);
            }
            if (*cur_si).si_idx >= 0 {
                let c_spp = syn_items(syn_block).offset((*cur_si).si_idx as isize);
                if in_id_list(
                    ptr::null_mut(),
                    (*ssp).cont_in_list,
                    &raw mut (*c_spp).sp_syn,
                    (*c_spp).sp_flags,
                ) != 0
                {
                    return 1;
                }
            }
        }

        if list.is_null() {
            return 0;
        }

        if list == id_list_all() {
            return c_int::from((flags & HL_CONTAINED) == 0);
        }

        let toplevel = (flags & HL_CONTAINED) == 0 || (flags & HL_INCLUDED_TOPLEVEL) != 0;

        let e = idl_get_entry(list);
        if !e.is_null() && (*e).idl_usable {
            let retval = if (*e).idl_marker != 0 {
                let item = (*e).idl_marker;
                if item < SYNID_TOP {
                    if (item - SYNID_ALLBUT) as c_int != (*ssp).inc_tag {
                        return 0;
                    }
                } else if item < SYNID_CONTAINED {
                    if (item - SYNID_TOP) as c_int != (*ssp).inc_tag || !toplevel {
                        return 0;
                    }
                } else if (item - SYNID_CONTAINED) as c_int != (*ssp).inc_tag || toplevel {
                    return 0;
                }
                false
            } else {
                true
            };
            return c_int::from(if idl_contains(e, id) { retval } else { !retval });
        }

        let mut item = *list;
        let retval = if (SYNID_ALLBUT..SYNID_CLUSTER).contains(&item) {
            if item < SYNID_TOP {
                if (item - SYNID_ALLBUT) as c_int != (*ssp).inc_tag {
                    return 0;
                }
            } else if item < SYNID_CONTAINED {
                if (item - SYNID_TOP) as c_int != (*ssp).inc_tag || !toplevel {
                    return 0;
                }
            } else if (item - SYNID_CONTAINED) as c_int != (*ssp).inc_tag || toplevel {
                return 0;
            }
            list = list.add(1);
            item = *list;
            false
        } else {
            true
        };

        while item != 0 {
            if item == id {
                return c_int::from(retval);
            }
            if item >= SYNID_CLUSTER {
                let scl_list =
                    (*syn_clstr(syn_block).offset((item - SYNID_CLUSTER) as isize)).scl_list;
                if !scl_list.is_null() && in_id_list_depth < 30 {
                    in_id_list_depth += 1;
                    let r = in_id_list(ptr::null_mut(), scl_list, ssp, flags);
                    in_id_list_depth -= 1;
                    if r != 0 {
                        return c_int::from(retval);
                    }
                }
            }
            list = list.add(1);
            item = *list;
        }
        c_int::from(!retval)
    }
}

type SubcmdFn = unsafe fn(*mut exarg_T, c_int);

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
pub unsafe fn ex_syntax(eap: *mut exarg_T) {
    unsafe {
        let arg = (*eap).arg;
        syn_cmdlinep = (*eap).cmdlinep;

        let mut subcmd_end = arg;
        while ascii_isalpha(*subcmd_end) {
            subcmd_end = subcmd_end.add(1);
        }
        let subcmd_name = xstrnsave(arg, subcmd_end.offset_from(arg) as usize);
        if (*eap).skip != 0 {
            emsg_skip += 1;
        }
        let mut found = false;
        for &(name, func) in &SUBCOMMANDS {
            if strcmp(subcmd_name, name) == 0 {
                (*eap).arg = skipwhite(subcmd_end);
                func(eap, 0);
                found = true;
                break;
            }
        }

        if !found {
            semsg!(
                gettext(c"E410: Invalid :syntax subcommand: %s".as_ptr()),
                subcmd_name,
            );
        }

        xfree(subcmd_name.cast());
        if (*eap).skip != 0 {
            emsg_skip -= 1;
        }
    }
}

/// @deprecated
pub unsafe fn ex_ownsyntax(eap: *mut exarg_T) {
    unsafe {
        if win_s(curwin) == &raw mut (*(*curwin).w_buffer).b_s {
            let new_s = xcalloc(1, size_of::<synblock_T>()).cast::<synblock_T>();
            (*curwin).w_s = new_s.cast();
            hash_init(&raw mut (*new_s).b_keywtab);
            hash_init(&raw mut (*new_s).b_keywtab_ic);
            (*curwin).w_onebuf_opt.wo_spell = 0;
            clear_string_option(&raw mut (*new_s).b_p_spc);
            clear_string_option(&raw mut (*new_s).b_p_spf);
            clear_string_option(&raw mut (*new_s).b_p_spl);
            clear_string_option(&raw mut (*new_s).b_p_spo);
            clear_string_option(&raw mut (*new_s).b_syn_isk);
        }

        let mut old_value = get_var_value(c"b:current_syntax".as_ptr());
        if !old_value.is_null() {
            old_value = xstrdup(old_value);
        }

        apply_autocmds(EVENT_SYNTAX, (*eap).arg, (*curbuf).b_fname, true, curbuf);

        let new_value = get_var_value(c"b:current_syntax".as_ptr());
        if !new_value.is_null() {
            set_internal_string_var(c"w:current_syntax".as_ptr(), new_value);
        }

        if old_value.is_null() {
            do_unlet(c"b:current_syntax".as_ptr(), 16, true);
        } else {
            set_internal_string_var(c"b:current_syntax".as_ptr(), old_value);
            xfree(old_value.cast());
        }
    }
}

pub unsafe fn syntax_present(win: *mut win_T) -> bool {
    unsafe {
        (*win_s(win)).b_syn_patterns.ga_len != 0
            || (*win_s(win)).b_syn_clusters.ga_len != 0
            || (*win_s(win)).b_keywtab.ht_used > 0
            || (*win_s(win)).b_keywtab_ic.ht_used > 0
    }
}

pub unsafe fn reset_expand_highlight() {
    unsafe {
        include_none = 0;
        include_default = 0;
        include_link = 0;
    }
}

pub unsafe fn set_context_in_echohl_cmd(xp: *mut expand_T, arg: *const c_char) {
    unsafe {
        (*xp).xp_context = EXPAND_HIGHLIGHT;
        (*xp).xp_pattern = arg.cast_mut();
        include_none = 1;
    }
}

pub unsafe fn set_context_in_syntax_cmd(xp: *mut expand_T, arg: *const c_char) {
    unsafe {
        (*xp).xp_context = EXPAND_SYNTAX;
        expand_what = EXP_SUBCMD;
        (*xp).xp_pattern = arg.cast_mut();
        include_link = 0;
        include_default = 0;

        if *arg == NUL {
            return;
        }

        let p = skiptowhite(arg);
        if *p == NUL {
            return;
        }

        (*xp).xp_pattern = skipwhite(p);
        if *skiptowhite((*xp).xp_pattern) != NUL {
            (*xp).xp_context = EXPAND_NOTHING;
        } else {
            let n = p.offset_from(arg) as usize;
            if vim_strnicmp(arg, c"case".as_ptr(), n) == 0 {
                expand_what = EXP_CASE;
            } else if vim_strnicmp(arg, c"spell".as_ptr(), n) == 0 {
                expand_what = EXP_SPELL;
            } else if vim_strnicmp(arg, c"sync".as_ptr(), n) == 0 {
                expand_what = EXP_SYNC;
            } else if vim_strnicmp(arg, c"list".as_ptr(), n) == 0 {
                let p2 = skipwhite(p);
                if *p2 == b'@' as c_char {
                    expand_what = EXP_CLUSTER;
                } else {
                    (*xp).xp_context = EXPAND_HIGHLIGHT;
                }
            } else if vim_strnicmp(arg, c"keyword".as_ptr(), n) == 0
                || vim_strnicmp(arg, c"region".as_ptr(), n) == 0
                || vim_strnicmp(arg, c"match".as_ptr(), n) == 0
            {
                (*xp).xp_context = EXPAND_HIGHLIGHT;
            } else {
                (*xp).xp_context = EXPAND_NOTHING;
            }
        }
    }
}

pub unsafe fn get_syntax_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx < 0 {
            return ptr::null_mut();
        }
        let u = idx as usize;
        match expand_what {
            EXP_SUBCMD => {
                if u >= SUBCOMMANDS.len() {
                    ptr::null_mut()
                } else {
                    SUBCOMMANDS[u].0.cast_mut()
                }
            }
            EXP_CASE => {
                const CASE_ARGS: [*const c_char; 3] =
                    [c"match".as_ptr(), c"ignore".as_ptr(), ptr::null()];
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
                    ptr::null(),
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
                    ptr::null(),
                ];
                if u >= SYNC_ARGS.len() {
                    ptr::null_mut()
                } else {
                    SYNC_ARGS[u].cast_mut()
                }
            }
            EXP_CLUSTER => {
                if idx < (*win_s(curwin)).b_syn_clusters.ga_len {
                    let xp_buf = (&raw mut (*xp).xp_buf).cast::<c_char>();
                    vim_snprintf!(
                        xp_buf,
                        EXPAND_BUF_LEN,
                        c"@%s".as_ptr(),
                        (*syn_clstr(win_s(curwin)).offset(idx as isize)).scl_name,
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
pub unsafe fn syn_get_id(
    wp: *mut win_T,
    lnum: linenr_T,
    col: colnr_T,
    trans: c_int,
    spellp: *mut bool,
    keep_state: c_int,
) -> c_int {
    unsafe {
        if wp != syn_win || (*wp).w_buffer != syn_buf || lnum != current_lnum || col < current_col {
            syntax_start(wp, lnum);
        } else if col > current_col {
            next_match_idx = -1;
        }

        get_syntax_attr(col, spellp, keep_state != 0);

        if trans != 0 {
            current_trans_id
        } else {
            current_id
        }
    }
}

pub unsafe fn get_syntax_info(seqnrp: *mut c_int) -> c_int {
    unsafe {
        *seqnrp = current_seqnr;
        current_flags
    }
}

pub unsafe fn syn_get_sub_char() -> c_int {
    unsafe { current_sub_char }
}

pub unsafe fn syn_get_stack_item(i: c_int) -> c_int {
    unsafe {
        if i >= current_state.ga_len {
            invalidate_current_state();
            current_col = MAXCOL;
            return -1;
        }
        (*cur_state_at(i)).si_id
    }
}

unsafe fn syn_cur_foldlevel() -> c_int {
    unsafe {
        let mut level: c_int = 0;
        for i in 0..current_state.ga_len {
            if ((*cur_state_at(i)).si_flags & HL_FOLD) != 0 {
                level += 1;
            }
        }
        level
    }
}

pub unsafe fn syn_get_foldlevel(wp: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        let mut level: c_int = 0;

        if (*win_s(wp)).b_syn_folditems != 0
            && !(*win_s(wp)).b_syn_error
            && !(*win_s(wp)).b_syn_slow
        {
            syntax_start(wp, lnum);

            level = syn_cur_foldlevel();

            if (*win_s(wp)).b_syn_foldlevel == SYNFLD_MINIMUM {
                let mut cur_level = level;
                let mut low_level = cur_level;
                while !current_finished {
                    syn_current_attr(false, false, ptr::null_mut(), false);
                    cur_level = syn_cur_foldlevel();
                    if cur_level < low_level {
                        low_level = cur_level;
                    } else if cur_level > low_level {
                        level = low_level;
                    }
                    current_col += 1;
                }
            }
        }
        if (level as i64) > (*wp).w_onebuf_opt.wo_fdn {
            level = (*wp).w_onebuf_opt.wo_fdn as c_int;
            if level < 0 {
                level = 0;
            }
        }
        level
    }
}

/// ":syntime".
pub unsafe fn ex_syntime(eap: *mut exarg_T) {
    unsafe {
        if strcmp((*eap).arg, c"on".as_ptr()) == 0 {
            syn_time_on = true;
        } else if strcmp((*eap).arg, c"off".as_ptr()) == 0 {
            syn_time_on = false;
        } else if strcmp((*eap).arg, c"clear".as_ptr()) == 0 {
            syntime_clear();
        } else if strcmp((*eap).arg, c"report".as_ptr()) == 0 {
            syntime_report();
        } else {
            semsg!(gettext((&raw const e_invarg2).cast()), (*eap).arg);
        }
    }
}

unsafe fn syn_clear_time(st: *mut syn_time_T) {
    unsafe {
        (*st).total = profile_zero();
        (*st).slowest = profile_zero();
        (*st).count = 0;
        (*st).match_ = 0;
    }
}

unsafe fn syntime_clear() {
    unsafe {
        if !syntax_present(curwin) {
            msg(gettext(msg_no_items), 0);
            return;
        }
        for idx in 0..(*win_s(curwin)).b_syn_patterns.ga_len {
            let spp = syn_items(win_s(curwin)).offset(idx as isize);
            syn_clear_time(&raw mut (*spp).sp_time);
        }
    }
}

pub unsafe fn get_syntime_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    match idx {
        0 => c"on".as_ptr().cast_mut(),
        1 => c"off".as_ptr().cast_mut(),
        2 => c"clear".as_ptr().cast_mut(),
        3 => c"report".as_ptr().cast_mut(),
        _ => ptr::null_mut(),
    }
}

fn syn_compare_syntime(s1: &time_entry_T, s2: &time_entry_T) -> c_int {
    profile_cmp(s1.total, s2.total)
}

unsafe fn syntime_report() {
    unsafe {
        if !syntax_present(curwin) {
            msg(gettext(msg_no_items), 0);
            return;
        }

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, size_of::<time_entry_T>() as c_int, 50);

        let mut total_total = profile_zero();
        let mut total_count: c_int = 0;
        for idx in 0..(*win_s(curwin)).b_syn_patterns.ga_len {
            let spp = syn_items(win_s(curwin)).offset(idx as isize);
            if (*spp).sp_time.count > 0 {
                ga_grow(&raw mut ga, 1);
                let p = ga.ga_data.cast::<time_entry_T>().offset(ga.ga_len as isize);
                ga.ga_len += 1;
                (*p).total = (*spp).sp_time.total;
                total_total = profile_add(total_total, (*spp).sp_time.total);
                (*p).count = (*spp).sp_time.count;
                (*p).match_ = (*spp).sp_time.match_;
                total_count += (*spp).sp_time.count;
                (*p).slowest = (*spp).sp_time.slowest;
                (*p).average = profile_divide((*spp).sp_time.total, (*spp).sp_time.count);
                (*p).id = (*spp).sp_syn.id as c_int;
                (*p).pattern = (*spp).sp_pattern;
            }
        }

        if ga.ga_len > 1 {
            let slice = std::slice::from_raw_parts_mut(
                ga.ga_data.cast::<time_entry_T>(),
                ga.ga_len as usize,
            );
            sort_slice_by(slice, syn_compare_syntime);
        }

        msg_puts_title(gettext(
            c"  TOTAL      COUNT  MATCH   SLOWEST     AVERAGE   NAME               PATTERN"
                .as_ptr(),
        ));
        msg_puts(c"\n".as_ptr());
        let mut idx: c_int = 0;
        while idx < ga.ga_len && !got_int {
            let p = ga.ga_data.cast::<time_entry_T>().offset(idx as isize);

            msg_puts(profile_msg((*p).total));
            msg_puts(c" ".as_ptr());
            msg_advance(13);
            msg_outnum((*p).count);
            msg_puts(c" ".as_ptr());
            msg_advance(20);
            msg_outnum((*p).match_);
            msg_puts(c" ".as_ptr());
            msg_advance(26);
            msg_puts(profile_msg((*p).slowest));
            msg_puts(c" ".as_ptr());
            msg_advance(38);
            msg_puts(profile_msg((*p).average));
            msg_puts(c" ".as_ptr());
            msg_advance(50);
            msg_outtrans(highlight_group_name((*p).id - 1), 0, false);
            msg_puts(c" ".as_ptr());

            msg_advance(69);
            let mut len = if Columns < 80 { 20 } else { Columns - 70 };
            let patlen = strlen((*p).pattern) as c_int;
            if patlen < len {
                len = patlen;
            }
            msg_outtrans_len((*p).pattern, len, 0, false);
            msg_puts(c"\n".as_ptr());
            idx += 1;
        }
        ga_clear(&raw mut ga);
        if !got_int {
            msg_puts(c"\n".as_ptr());
            msg_puts(profile_msg(total_total));
            msg_advance(13);
            msg_outnum(total_count);
            msg_puts(c"\n".as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn get_syntime_arg_and_limit_pos_work() {
        unsafe {
            assert_eq!(
                CStr::from_ptr(get_syntime_arg(ptr::null_mut(), 0)).to_bytes(),
                b"on"
            );
            assert_eq!(
                CStr::from_ptr(get_syntime_arg(ptr::null_mut(), 3)).to_bytes(),
                b"report"
            );
            assert!(get_syntime_arg(ptr::null_mut(), 4).is_null());

            let mut pos = lpos_T { lnum: 5, col: 10 };
            let mut limit = lpos_T { lnum: 5, col: 4 };
            limit_pos(&raw mut pos, &raw mut limit);
            assert_eq!(pos.col, 4);
        }
    }
}
