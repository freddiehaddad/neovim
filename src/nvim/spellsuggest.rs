//! Functions for spelling suggestions (`src/nvim/spellsuggest.c`).
#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]
use crate::api::tabpage::win_T;
use crate::change::inserted_bytes;
use crate::charset::{getdigits_int, skiptowhite, skipwhite};
use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::cursor::{check_cursor, colnr_T, get_cursor_line_len, get_cursor_line_ptr, linenr_T};
use crate::errors::*;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{buf_T, synblock_T};
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::hashtab::{
    hash_T, hash_add_item, hash_clear, hash_clear_all, hash_find, hash_hash, hash_init,
    hash_lookup, hashitem_empty, hashtab_T,
};
use crate::input::{beep_flush, prep_redo, redo_append_char, redo_append_lit, vgetc};
use crate::input_cmdatom::CmdSpec;
use crate::mbyte::{
    mb_charlen, mb_cptr2char_adv, mb_isupper, mb_stricmp, utf_byte2len, utf_char2bytes,
    utf_char2len, utf_fold, utf_head_off, utf_iscomposing_legacy, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len,
};
use crate::memory::{xfree, xmalloc, xmallocz, xmemcpyz, xmemdupz, xstrdup, xstrlcpy};
use crate::os::fs::os_fopen;
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::profile::{profile_passed_limit, profile_setlimit, proftime_T};
use crate::spell::{repl_from, repl_to, spelltab};
use crate::tag::BoolVarValue;
use crate::undo::u_save_cursor;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;
pub type idx_T = c_int;
pub type salfirst_T = c_int;
pub type hlf_T = c_int;
pub type smt_T = c_int;
pub type varnumber_T = i64;
pub type FILE = c_void;
const kBoolVarFalse: BoolVarValue = 0;
const kBoolVarTrue: BoolVarValue = 1;
const NUL: c_char = 0;
const TAB: u8 = b'\t';
const ESC: c_int = 0x1b;
const FAIL: c_int = 0;
const OK: c_int = 1;
const FORWARD: c_int = 1;
const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const MB_MAXCHAR: usize = 6;
const MAXWLEN: usize = 254;
const MAXREGIONS: usize = 8;
const WF_REGION: c_int = 0x01;
const WF_ONECAP: c_int = 0x02;
const WF_ALLCAP: c_int = 0x04;
const WF_RARE: c_int = 0x08;
const WF_BANNED: c_int = 0x10;
const WF_FIXCAP: c_int = 0x40;
const WF_KEEPCAP: c_int = 0x80;
const WF_CAPMASK: c_int = WF_ONECAP | WF_ALLCAP | WF_KEEPCAP | WF_FIXCAP;
const WF_MIXCAP: c_int = 0x20;
const WF_NEEDCOMP: c_int = 0x0200;
const WF_NOSUGGEST: c_int = 0x0400;
const WFP_RARE: c_int = 0x01;
const WF_RAREPFX: c_int = WFP_RARE << 24;
const SCORE_SPLIT: c_int = 149;
const SCORE_SPLIT_NO: c_int = 249;
const SCORE_ICASE: c_int = 52;
const SCORE_REGION: c_int = 200;
const SCORE_RARE: c_int = 180;
const SCORE_SWAP: c_int = 75;
const SCORE_SWAP3: c_int = 110;
const SCORE_REP: c_int = 65;
const SCORE_SUBST: c_int = 93;
const SCORE_SIMILAR: c_int = 33;
const SCORE_SUBCOMP: c_int = 33;
const SCORE_DEL: c_int = 94;
const SCORE_DELDUP: c_int = 66;
const SCORE_DELCOMP: c_int = 28;
const SCORE_INS: c_int = 96;
const SCORE_INSDUP: c_int = 67;
const SCORE_INSCOMP: c_int = 30;
const SCORE_NONWORD: c_int = 103;
const SCORE_FILE: c_int = 30;
const SCORE_MAXINIT: c_int = 350;
const SCORE_COMMON1: c_int = 30;
const SCORE_COMMON2: c_int = 40;
const SCORE_COMMON3: c_int = 50;
const SCORE_THRES2: u16 = 10;
const SCORE_THRES3: u16 = 100;
const SCORE_SFMAX1: c_int = 200;
const SCORE_SFMAX2: c_int = 300;
const SCORE_SFMAX3: c_int = 400;
const SCORE_BIG: c_int = SCORE_INS * 3;
const SCORE_MAXMAX: c_int = 999999;
const SCORE_LIMITMAX: c_int = 350;
const SCORE_EDIT_MIN: c_int = SCORE_SIMILAR;
const DIFF_NONE: u8 = 0;
const DIFF_YES: u8 = 1;
const DIFF_INSERT: u8 = 2;
const TSF_PREFIXOK: u8 = 1;
const TSF_DIDSPLIT: u8 = 2;
const TSF_DIDDEL: u8 = 4;
const PFD_NOPREFIX: u8 = 0xff;
const PFD_PREFIXTREE: u8 = 0xfe;
const PFD_NOTSPECIAL: u8 = 0xfd;
const SPS_BEST: c_int = 1;
const SPS_FAST: c_int = 2;
const SPS_DOUBLE: c_int = 4;
const SMT_ALL: smt_T = 0;
const HLF_COUNT: hlf_T = 76;
const kOptBoFlagSpell: c_uint = 0x20000;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct fromto_T {
    pub ft_from: *mut c_char,
    pub ft_to: *mut c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct salitem_T {
    pub sm_lead: *mut c_char,
    pub sm_leadlen: c_int,
    pub sm_oneof: *mut c_char,
    pub sm_rules: *mut c_char,
    pub sm_to: *mut c_char,
    pub sm_lead_w: *mut c_int,
    pub sm_oneof_w: *mut c_int,
    pub sm_to_w: *mut c_int,
}
#[repr(C)]
pub struct slang_T {
    pub sl_next: *mut slang_T,
    pub sl_name: *mut c_char,
    pub sl_fname: *mut c_char,
    pub sl_add: bool,
    pub sl_fbyts: *mut u8,
    pub sl_fbyts_len: c_int,
    pub sl_fidxs: *mut idx_T,
    pub sl_kbyts: *mut u8,
    pub sl_kidxs: *mut idx_T,
    pub sl_pbyts: *mut u8,
    pub sl_pidxs: *mut idx_T,
    pub sl_info: *mut c_char,
    pub sl_regions: [c_char; MAXREGIONS * 2 + 1],
    pub sl_midword: *mut c_char,
    pub sl_wordcount: hashtab_T,
    pub sl_compmax: c_int,
    pub sl_compminlen: c_int,
    pub sl_compsylmax: c_int,
    pub sl_compoptions: c_int,
    pub sl_comppat: garray_T,
    pub sl_compprog: *mut c_void,
    pub sl_comprules: *mut u8,
    pub sl_compstartflags: *mut u8,
    pub sl_compallflags: *mut u8,
    pub sl_nobreak: bool,
    pub sl_syllable: *mut c_char,
    pub sl_syl_items: garray_T,
    pub sl_prefixcnt: c_int,
    pub sl_prefprog: *mut *mut c_void,
    pub sl_rep: garray_T,
    pub sl_rep_first: [i16; 256],
    pub sl_sal: garray_T,
    pub sl_sal_first: [salfirst_T; 256],
    pub sl_followup: bool,
    pub sl_collapse: bool,
    pub sl_rem_accents: bool,
    pub sl_sofo: bool,
    pub sl_repsal: garray_T,
    pub sl_repsal_first: [i16; 256],
    pub sl_nosplitsugs: bool,
    pub sl_nocompoundsugs: bool,
    pub sl_sugtime: i64,
    pub sl_sbyts: *mut u8,
    pub sl_sbyts_len: c_int,
    pub sl_sidxs: *mut idx_T,
    pub sl_sugbuf: *mut buf_T,
    pub sl_sugloaded: bool,
    pub sl_has_map: bool,
    pub sl_map_hash: hashtab_T,
    pub sl_map_array: [c_int; 256],
    pub sl_sounddone: hashtab_T,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct langp_T {
    pub lp_slang: *mut slang_T,
    pub lp_sallang: *mut slang_T,
    pub lp_replang: *mut slang_T,
    pub lp_region: c_int,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct spelltab_T {
    pub st_isw: [bool; 256],
    pub st_isu: [bool; 256],
    pub st_fold: [u8; 256],
    pub st_upper: [u8; 256],
}
#[repr(C)]
pub struct wordcount_T {
    pub wc_count: u16,
    pub wc_word: [c_char; 0],
}
#[repr(C)]
struct suginfo_T {
    su_ga: garray_T,
    su_maxcount: c_int,
    su_maxscore: c_int,
    su_sfmaxscore: c_int,
    su_sga: garray_T,
    su_badptr: *mut c_char,
    su_badlen: c_int,
    su_badflags: c_int,
    su_badword: [c_char; MAXWLEN],
    su_fbadword: [c_char; MAXWLEN],
    su_sal_badword: [c_char; MAXWLEN],
    su_banned: hashtab_T,
    su_sallang: *mut slang_T,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct suggest_T {
    st_word: *mut c_char,
    st_wordlen: c_int,
    st_orglen: c_int,
    st_score: c_int,
    st_altscore: c_int,
    st_salscore: bool,
    st_had_bonus: bool,
    st_slang: *mut slang_T,
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum state_T {
    STATE_START = 0,
    STATE_NOPREFIX,
    STATE_SPLITUNDO,
    STATE_ENDNUL,
    STATE_PLAIN,
    STATE_DEL,
    STATE_INS_PREP,
    STATE_INS,
    STATE_SWAP,
    STATE_UNSWAP,
    STATE_SWAP3,
    STATE_UNSWAP3,
    STATE_UNROT3L,
    STATE_UNROT3R,
    STATE_REP_INI,
    STATE_REP,
    STATE_REP_UNDO,
    STATE_FINAL,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct trystate_T {
    ts_state: state_T,
    ts_score: c_int,
    ts_arridx: idx_T,
    ts_curi: i16,
    ts_fidx: u8,
    ts_fidxtry: u8,
    ts_twordlen: u8,
    ts_prefixdepth: u8,
    ts_flags: u8,
    ts_tcharlen: u8,
    ts_tcharidx: u8,
    ts_isdiff: u8,
    ts_fcharstart: u8,
    ts_prewordlen: u8,
    ts_splitoff: u8,
    ts_splitfidx: u8,
    ts_complen: u8,
    ts_compsplit: u8,
    ts_save_badflags: u8,
    ts_delidx: u8,
}
impl Default for trystate_T {
    fn default() -> Self {
        Self {
            ts_state: state_T::STATE_START,
            ts_score: 0,
            ts_arridx: 0,
            ts_curi: 0,
            ts_fidx: 0,
            ts_fidxtry: 0,
            ts_twordlen: 0,
            ts_prefixdepth: 0,
            ts_flags: 0,
            ts_tcharlen: 0,
            ts_tcharidx: 0,
            ts_isdiff: 0,
            ts_fcharstart: 0,
            ts_prewordlen: 0,
            ts_splitoff: 0,
            ts_splitfidx: 0,
            ts_complen: 0,
            ts_compsplit: 0,
            ts_save_badflags: 0,
            ts_delidx: 0,
        }
    }
}
#[repr(C)]
struct sftword_T {
    sft_score: i16,
    sft_word: [u8; 0],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct limitscore_T {
    badi: c_int,
    goodi: c_int,
    score: c_int,
}
static mut spell_suggest_timeout: i64 = 5000;
static mut sps_flags: c_int = SPS_BEST;
static mut sps_limit: c_int = 9999;
static mut expr_busy: bool = false;
unsafe extern "C" {
    static mut p_sps: *mut c_char;
    static mut IObuff: [c_char; IOSIZE];
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
    fn msg(s: *const c_char, hl_id: c_int) -> bool;
    fn smsg(hl_id: c_int, fmt: *const c_char, ...) -> bool;
    fn msg_ext_set_kind(msg_kind: *const c_char);
    fn internal_error(where_: *const c_char);
    fn option_next_keyval(
        p: *mut *const c_char,
        key: *mut *const c_char,
        keylen: *mut usize,
        val: *mut *const c_char,
        vallen: *mut usize,
    ) -> bool;
    fn option_slice_eq(slice: *const c_char, slicelen: usize, name: *const c_char) -> bool;
    fn copy_option_part(
        option: *mut *mut c_char,
        buf: *mut c_char,
        maxlen: usize,
        sep_chars: *const c_char,
    ) -> usize;
    fn tv_list_alloc_ret(ret_tv: *mut typval_T, len: isize) -> *mut list_T;
    fn tv_list_append_tv(l: *mut list_T, tv: *mut typval_T);
    fn tv_list_unref(l: *mut list_T);
    fn tv_dict_alloc() -> *mut dict_T;
    fn tv_dict_add_str(
        d: *mut dict_T,
        key: *const c_char,
        key_len: usize,
        val: *const c_char,
    ) -> c_int;
    fn tv_dict_add_allocated_str(
        d: *mut dict_T,
        key: *const c_char,
        key_len: usize,
        val: *mut c_char,
    ) -> c_int;
    fn tv_dict_add_nr(d: *mut dict_T, key: *const c_char, key_len: usize, nr: varnumber_T)
    -> c_int;
    fn tv_dict_add_bool(
        d: *mut dict_T,
        key: *const c_char,
        key_len: usize,
        val: BoolVarValue,
    ) -> c_int;
    fn tv_clear(tv: *mut typval_T);
    fn nlua_call_typval(
        module: *const c_char,
        func: *const c_char,
        argvars: *mut typval_T,
        rettv: *mut typval_T,
    );
    fn eval_spell_expr(badword: *mut c_char, expr: *mut c_char) -> *mut list_T;
    fn get_spellword(list: *mut list_T, ret_word: *mut *const c_char) -> c_int;
    fn end_visual_mode();
    fn vim_beep(val: c_uint);
    fn ml_get_buf(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char;
    fn ml_replace(lnum: linenr_T, line: *mut c_char, copy: bool) -> c_int;
    fn vim_fgets(buf: *mut c_char, size: c_int, fp: *mut FILE) -> bool;
    fn xstrnsave(string: *const c_char, len: usize) -> *mut c_char;
    fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char;
    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn byte_in_str(str: *mut u8, n: c_int) -> bool;
    fn match_compoundrule(slang: *mut slang_T, compflags: *const u8) -> bool;
    fn captype(word: *const c_char, end: *const c_char) -> c_int;
    fn parse_spelllang(wp: *mut win_T) -> *mut c_char;
    fn spell_move_to(
        wp: *mut win_T,
        dir: c_int,
        behaviour: smt_T,
        curline: bool,
        attrp: *mut hlf_T,
    ) -> usize;
    fn spell_iswordp(p: *const c_char, wp: *const win_T) -> bool;
    fn spell_iswordp_nmw(p: *const c_char, wp: *mut win_T) -> bool;
    fn check_need_cap(wp: *mut win_T, lnum: linenr_T, col: colnr_T) -> bool;
    fn spell_check(
        wp: *mut win_T,
        ptr: *mut c_char,
        attrp: *mut hlf_T,
        capcol: *mut c_int,
        docount: bool,
    ) -> usize;
    fn spell_casefold(
        wp: *const win_T,
        str: *const c_char,
        len: c_int,
        buf: *mut c_char,
        buflen: c_int,
    ) -> c_int;
    fn spell_soundfold(slang: *mut slang_T, inword: *mut c_char, folded: bool, res: *mut c_char);
    fn make_case_word(fword: *mut c_char, cword: *mut c_char, flags: c_int);
    fn suggest_load_files();
    fn nofold_len(fword: *mut c_char, flen: c_int, word: *mut c_char) -> c_int;
    fn valid_word_prefix(
        totprefcnt: c_int,
        arridx: c_int,
        flags: c_int,
        word: *mut c_char,
        slang: *mut slang_T,
        cond_req: bool,
    ) -> c_int;
    fn match_checkcompoundpattern(ptr: *mut c_char, wlen: c_int, gap: *mut garray_T) -> bool;
    fn can_compound(slang: *mut slang_T, word: *const c_char, flags: *const u8) -> bool;
    fn spell_valid_case(wordflags: c_int, treeflags: c_int) -> bool;
    fn allcap_copy(word: *const c_char, wcopy: *mut c_char);
    fn fclose(stream: *mut FILE) -> c_int;
    fn qsort(
        base: *mut c_void,
        nmemb: usize,
        size: usize,
        compar: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int,
    );
    fn strlen(s: *const c_char) -> usize;
    fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
    fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    fn strncpy(dst: *mut c_char, src: *const c_char, n: usize) -> *mut c_char;
    fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    fn atoi(s: *const c_char) -> c_int;
}
#[inline]
const fn ascii_isdigit(c: c_char) -> bool {
    (c as u8).is_ascii_digit()
}
#[inline]
const fn ascii_iswhite(c: c_char) -> bool {
    c == b' ' as c_char || c == b'\t' as c_char
}
#[inline]
const fn rescore(word_score: c_int, sound_score: c_int) -> c_int {
    (3 * word_score + sound_score) / 4
}
#[inline]
const fn maxscore(word_score: c_int, sound_score: c_int) -> c_int {
    (4 * word_score - sound_score) / 3
}
#[inline]
unsafe fn sug_clean_count(su: *const suginfo_T) -> c_int {
    unsafe {
        if (*su).su_maxcount < 130 {
            150
        } else {
            (*su).su_maxcount + 20
        }
    }
}
#[inline]
unsafe fn sug_max_count(su: *const suginfo_T) -> c_int {
    unsafe { sug_clean_count(su) + 50 }
}
#[inline]
unsafe fn spell_isupper(c: c_int) -> bool {
    unsafe {
        if c >= 128 {
            mb_isupper(c)
        } else if c >= 0 {
            spelltab.st_isu[c as usize]
        } else {
            false
        }
    }
}
#[inline]
unsafe fn spell_tofold(c: c_int) -> c_int {
    unsafe {
        if c >= 128 {
            utf_fold(c)
        } else if c >= 0 {
            spelltab.st_fold[c as usize] as c_int
        } else {
            c
        }
    }
}
#[inline]
unsafe fn mb_ptr_adv(p: &mut *mut c_char) {
    unsafe {
        *p = (*p).add(utfc_ptr2len(*p) as usize);
    }
}
#[inline]
unsafe fn mb_ptr_back(s: *const c_char, p: &mut *mut c_char) {
    unsafe {
        let off = utf_head_off(s, (*p).sub(1)) + 1;
        *p = (*p).sub(off as usize);
    }
}
#[inline]
unsafe fn mb_cptr_back(s: *const c_char, p: &mut *const c_char) {
    unsafe {
        let off = utf_head_off(s, (*p).sub(1)) + 1;
        *p = (*p).sub(off as usize);
    }
}
#[inline]
unsafe fn strmove(dst: *mut c_char, src: *const c_char) {
    unsafe {
        ptr::copy(src, dst, strlen(src) + 1);
    }
}
#[inline]
unsafe fn was_banned(su: *mut suginfo_T, word: *const c_char) -> bool {
    unsafe { !hashitem_empty(&*hash_find(&raw mut (*su).su_banned, word)) }
}
#[inline]
unsafe fn sug_at(ga: *const garray_T, i: usize) -> *mut suggest_T {
    unsafe { ((*ga).ga_data.cast::<suggest_T>()).add(i) }
}
#[inline]
unsafe fn langp_entry(ga: *const garray_T, i: usize) -> *mut langp_T {
    unsafe { ((*ga).ga_data.cast::<langp_T>()).add(i) }
}
unsafe fn can_be_compound(
    sp: *const trystate_T,
    slang: *mut slang_T,
    compflags: *mut u8,
    flag: c_int,
) -> bool {
    unsafe {
        if !byte_in_str(
            if (*sp).ts_complen == (*sp).ts_compsplit {
                (*slang).sl_compstartflags
            } else {
                (*slang).sl_compallflags
            },
            flag,
        ) {
            return false;
        }
        if !(*slang).sl_comprules.is_null() && (*sp).ts_complen > (*sp).ts_compsplit {
            let complen = (*sp).ts_complen as usize;
            *compflags.add(complen) = flag as u8;
            *compflags.add(complen + 1) = 0;
            let v = match_compoundrule(slang, compflags.add((*sp).ts_compsplit as usize));
            *compflags.add(complen) = 0;
            return v;
        }
        true
    }
}
unsafe fn score_wordcount_adj(
    slang: *mut slang_T,
    score: c_int,
    word: *const c_char,
    split: bool,
) -> c_int {
    unsafe {
        let hi = hash_find(&raw mut (*slang).sl_wordcount, word);
        if hashitem_empty(&*hi) {
            return score;
        }
        let wc = (*hi)
            .hi_key
            .sub(std::mem::offset_of!(wordcount_T, wc_word))
            .cast::<wordcount_T>();
        let bonus = if (*wc).wc_count < SCORE_THRES2 {
            SCORE_COMMON1
        } else if (*wc).wc_count < SCORE_THRES3 {
            SCORE_COMMON2
        } else {
            SCORE_COMMON3
        };
        let newscore = if split {
            score - bonus / 2
        } else {
            score - bonus
        };
        newscore.max(0)
    }
}
unsafe fn badword_captype(word: *mut c_char, end: *mut c_char) -> c_int {
    unsafe {
        let mut flags = captype(word, end);
        if (flags & WF_KEEPCAP) == 0 {
            return flags;
        }
        let mut l: c_int = 0;
        let mut u: c_int = 0;
        let mut first = false;
        let mut p = word;
        while p < end {
            let c = utf_ptr2char(p);
            if spell_isupper(c) {
                u += 1;
                if p == word {
                    first = true;
                }
            } else {
                l += 1;
            }
            mb_ptr_adv(&mut p);
        }
        if u > l && u > 2 {
            flags |= WF_ALLCAP;
        } else if first {
            flags |= WF_ONECAP;
        }
        if u >= 2 && l >= 2 {
            flags |= WF_MIXCAP;
        }
        flags
    }
}
unsafe fn bytes2offset(pp: *mut *mut c_char) -> c_int {
    unsafe {
        let mut p = (*pp).cast::<u8>();
        let c = *p as c_int;
        p = p.add(1);
        let nr = if (c & 0x80) == 0x00 {
            c - 1
        } else if (c & 0xc0) == 0x80 {
            let mut n = (c & 0x3f) - 1;
            n = n * 255 + (*p as c_int - 1);
            p = p.add(1);
            n
        } else if (c & 0xe0) == 0xc0 {
            let mut n = (c & 0x1f) - 1;
            n = n * 255 + (*p as c_int - 1);
            p = p.add(1);
            n = n * 255 + (*p as c_int - 1);
            p = p.add(1);
            n
        } else {
            let mut n = (c & 0x0f) - 1;
            n = n * 255 + (*p as c_int - 1);
            p = p.add(1);
            n = n * 255 + (*p as c_int - 1);
            p = p.add(1);
            n = n * 255 + (*p as c_int - 1);
            p = p.add(1);
            n
        };
        *pp = p.cast::<c_char>();
        nr
    }
}
/// Parse 'spellsuggest' without changing suggestion state.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_check_sps(
    value: *const c_char,
    ret_flags: *mut c_int,
    ret_limit: *mut c_int,
) -> c_int {
    unsafe {
        let mut flags: c_int = 0;
        let mut limit: c_int = 9999;
        let mut key: *const c_char = ptr::null();
        let mut val: *const c_char = ptr::null();
        let mut keylen: usize = 0;
        let mut vallen: usize = 0;
        let mut p = value;
        while option_next_keyval(
            &raw mut p,
            &raw mut key,
            &raw mut keylen,
            &raw mut val,
            &raw mut vallen,
        ) {
            let mut f: c_int = 0;
            if val.is_null() {
                if keylen > 0 && ascii_isdigit(*key) {
                    let mut s = key.cast_mut();
                    limit = getdigits_int(&raw mut s, true, 0);
                    if s.cast_const() != key.add(keylen) {
                        f = -1;
                    }
                } else if option_slice_eq(key, keylen, c"best".as_ptr()) {
                    f = SPS_BEST;
                } else if option_slice_eq(key, keylen, c"fast".as_ptr()) {
                    f = SPS_FAST;
                } else if option_slice_eq(key, keylen, c"double".as_ptr()) {
                    f = SPS_DOUBLE;
                } else {
                    f = -1;
                }
            } else if option_slice_eq(key, keylen, c"expr".as_ptr())
                || option_slice_eq(key, keylen, c"file".as_ptr())
            {
                // consumed later in spell_find_suggest()
            } else if option_slice_eq(key, keylen, c"timeout".as_ptr()) {
                let mut v = val;
                let mut vl = vallen;
                if vl > 0 && *v == b'-' as c_char {
                    v = v.add(1);
                    vl -= 1;
                }
                if vl == 0 || !ascii_isdigit(*v) {
                    f = -1;
                }
            } else {
                f = -1;
            }
            if f == -1 || (flags != 0 && f != 0) {
                return FAIL;
            }
            if f != 0 {
                flags = f;
            }
        }
        if flags == 0 {
            flags = SPS_BEST;
        }
        *ret_flags = flags;
        *ret_limit = limit;
        OK
    }
}
/// Apply validated 'spellsuggest' flags and suggestion limit.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_apply_sps(flags: c_int, limit: c_int) {
    unsafe {
        sps_flags = flags;
        sps_limit = limit;
    }
}
unsafe fn select_spell_suggestion(sug: *mut suginfo_T) {
    unsafe {
        let mut items_tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval { v_number: 0 },
        };
        tv_list_alloc_ret(&raw mut items_tv, (*sug).su_ga.ga_len as isize);
        for i in 0..((*sug).su_ga.ga_len as usize) {
            let stp = sug_at(&raw const (*sug).su_ga, i);
            let d = tv_dict_alloc();
            let el = (*sug).su_badlen - (*stp).st_orglen;
            if el > 0 {
                let word = xmallocz(((*stp).st_wordlen + el) as usize).cast::<c_char>();
                ptr::copy_nonoverlapping((*stp).st_word, word, (*stp).st_wordlen as usize);
                ptr::copy_nonoverlapping(
                    (*sug).su_badptr.add((*stp).st_orglen as usize),
                    word.add((*stp).st_wordlen as usize),
                    el as usize,
                );
                tv_dict_add_allocated_str(d, c"word".as_ptr(), 4, word);
            } else {
                tv_dict_add_str(d, c"word".as_ptr(), 4, (*stp).st_word);
            }
            if (*sug).su_badlen < (*stp).st_orglen {
                let extra = xstrnsave((*sug).su_badptr, (*stp).st_orglen as usize);
                tv_dict_add_allocated_str(d, c"extra".as_ptr(), 5, extra);
            }
            tv_dict_add_nr(d, c"score".as_ptr(), 5, (*stp).st_score as i64);
            if (sps_flags & (SPS_DOUBLE | SPS_BEST)) != 0 {
                tv_dict_add_nr(d, c"altscore".as_ptr(), 8, (*stp).st_altscore as i64);
                tv_dict_add_bool(
                    d,
                    c"salscore".as_ptr(),
                    8,
                    if (*stp).st_salscore {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
            }
            let mut item = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: 0,
                vval: TypvalVval { v_dict: d },
            };
            tv_list_append_tv(items_tv.vval.v_list, &raw mut item);
        }
        let mut bad_tv = typval_T {
            v_type: VarType::VAR_STRING,
            v_lock: 0,
            vval: TypvalVval {
                v_string: xstrnsave((*sug).su_badptr, (*sug).su_badlen as usize),
            },
        };
        let mut lua_args = [
            items_tv,
            bad_tv,
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        nlua_call_typval(
            c"vim._core.spell".as_ptr(),
            c"select_suggest".as_ptr(),
            (&raw mut lua_args).cast(),
            ptr::null_mut(),
        );
        tv_clear(&raw mut items_tv);
        tv_clear(&raw mut bad_tv);
    }
}
/// "z=": Find badly spelled word under or after the cursor.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_suggest(count: c_int) {
    unsafe {
        let mut badlen: c_int = 0;
        let msg_scroll_save = msg_scroll;
        let wo_spell_save = (*curwin).w_onebuf_opt.wo_spell;
        if (*curwin).w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin);
            (*curwin).w_onebuf_opt.wo_spell = 1;
        }
        check_cursor(curwin);
        let prev_cursor = (*curwin).w_cursor;
        'skip: {
            let w_s = (*curwin).w_s.cast::<synblock_T>();
            if *(*w_s).b_p_spl == NUL {
                emsg(gettext((&raw const e_no_spell).cast()));
                break 'skip;
            }
            if Visual.active {
                if (*curwin).w_cursor.lnum != Visual.start.lnum {
                    vim_beep(kOptBoFlagSpell);
                    break 'skip;
                }
                badlen = (*curwin).w_cursor.col - Visual.start.col;
                if badlen < 0 {
                    badlen = -badlen;
                } else {
                    (*curwin).w_cursor.col = Visual.start.col;
                }
                badlen += 1;
                end_visual_mode();
                badlen = badlen.min(get_cursor_line_len() - (*curwin).w_cursor.col);
            } else if spell_move_to(curwin, FORWARD, SMT_ALL, true, ptr::null_mut()) == 0
                || (*curwin).w_cursor.col > prev_cursor.col
            {
                (*curwin).w_cursor = prev_cursor;
                let curline = get_cursor_line_ptr();
                let mut p = curline.add((*curwin).w_cursor.col as usize);
                while p > curline && spell_iswordp_nmw(p, curwin) {
                    mb_ptr_back(curline, &mut p);
                }
                while *p != NUL && !spell_iswordp_nmw(p, curwin) {
                    mb_ptr_adv(&mut p);
                }
                if !spell_iswordp_nmw(p, curwin) {
                    beep_flush();
                    break 'skip;
                }
                (*curwin).w_cursor.col = p.offset_from(curline) as colnr_T;
            }
            let need_cap = check_need_cap(curwin, (*curwin).w_cursor.lnum, (*curwin).w_cursor.col);
            let line = xstrnsave(get_cursor_line_ptr(), get_cursor_line_len() as usize);
            spell_suggest_timeout = 5000;
            let mut sug: suginfo_T = std::mem::MaybeUninit::zeroed().assume_init();
            let limit = sps_limit.min(Rows - 2);
            spell_find_suggest(
                line.add((*curwin).w_cursor.col as usize),
                badlen,
                &raw mut sug,
                limit,
                true,
                need_cap,
                true,
            );
            let selected = count;
            if sug.su_ga.ga_len <= 0 {
                msg_ext_set_kind(c"wmsg".as_ptr());
                msg(gettext(c"No suggestions".as_ptr()), 0);
            } else if count > 0 {
                if count > sug.su_ga.ga_len {
                    msg_ext_set_kind(c"wmsg".as_ptr());
                    smsg(
                        0,
                        gettext(c"Only %lld suggestions".as_ptr()),
                        sug.su_ga.ga_len as i64,
                    );
                }
            } else {
                (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
                select_spell_suggestion(&raw mut sug);
                lines_left = Rows;
                msg_scroll = msg_scroll_save;
                spell_find_cleanup(&raw mut sug);
                xfree(line.cast());
                return;
            }
            if selected > 0 && selected <= sug.su_ga.ga_len && u_save_cursor() == OK {
                xfree(repl_from.cast());
                repl_from = ptr::null_mut();
                xfree(repl_to.cast());
                repl_to = ptr::null_mut();
                let stp = sug_at(&raw const sug.su_ga, (selected - 1) as usize);
                if sug.su_badlen > (*stp).st_orglen {
                    repl_from = xstrnsave(sug.su_badptr, sug.su_badlen as usize);
                    vim_snprintf(
                        (&raw mut IObuff).cast(),
                        IOSIZE,
                        c"%s%.*s".as_ptr(),
                        (*stp).st_word,
                        sug.su_badlen - (*stp).st_orglen,
                        sug.su_badptr.add((*stp).st_orglen as usize),
                    );
                    repl_to = xstrdup((&raw const IObuff).cast());
                } else {
                    repl_from = xstrnsave(sug.su_badptr, (*stp).st_orglen as usize);
                    repl_to = xstrdup((*stp).st_word);
                }
                let p = xmalloc(
                    strlen(line) - (*stp).st_orglen as usize + (*stp).st_wordlen as usize + 1,
                )
                .cast::<c_char>();
                let c = sug.su_badptr.offset_from(line) as c_int;
                ptr::copy(line, p, c as usize);
                strcpy(p.add(c as usize), (*stp).st_word);
                strcat(p, sug.su_badptr.add((*stp).st_orglen as usize));
                prep_redo(
                    false,
                    false,
                    CmdSpec {
                        op: b'c' as c_int,
                        cmd: b'i' as c_int,
                        cmd2: b'w' as c_int,
                        ..CmdSpec::default()
                    },
                );
                redo_append_lit(
                    p.add(c as usize),
                    (*stp).st_wordlen + sug.su_badlen - (*stp).st_orglen,
                );
                redo_append_char(ESC);
                let _ = ml_replace((*curwin).w_cursor.lnum, p, false);
                (*curwin).w_cursor.col = c;
                inserted_bytes(
                    (*curwin).w_cursor.lnum,
                    c,
                    (*stp).st_orglen,
                    (*stp).st_wordlen,
                );
            } else {
                (*curwin).w_cursor = prev_cursor;
            }
            spell_find_cleanup(&raw mut sug);
            xfree(line.cast());
        }
        (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
    }
}
/// Find spell suggestions for "word".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_suggest_list(
    gap: *mut garray_T,
    word: *mut c_char,
    maxcount: c_int,
    need_cap: bool,
    interactive: bool,
) {
    unsafe {
        let mut sug: suginfo_T = std::mem::MaybeUninit::zeroed().assume_init();
        spell_find_suggest(
            word,
            0,
            &raw mut sug,
            maxcount,
            false,
            need_cap,
            interactive,
        );
        ga_init(gap, size_of::<*mut c_char>() as c_int, sug.su_ga.ga_len + 1);
        ga_grow(gap, sug.su_ga.ga_len);
        for i in 0..(sug.su_ga.ga_len as usize) {
            let stp = sug_at(&raw const sug.su_ga, i);
            let wcopy = xmalloc(
                (*stp).st_wordlen as usize
                    + strlen(sug.su_badptr.add((*stp).st_orglen as usize))
                    + 1,
            )
            .cast::<c_char>();
            strcpy(wcopy, (*stp).st_word);
            strcpy(
                wcopy.add((*stp).st_wordlen as usize),
                sug.su_badptr.add((*stp).st_orglen as usize),
            );
            *((*gap).ga_data.cast::<*mut c_char>()).add((*gap).ga_len as usize) = wcopy;
            (*gap).ga_len += 1;
        }
        spell_find_cleanup(&raw mut sug);
    }
}
unsafe fn spell_find_suggest(
    badptr: *mut c_char,
    badlen: c_int,
    su: *mut suginfo_T,
    maxcount: c_int,
    banbadword: bool,
    need_cap: bool,
    interactive: bool,
) {
    unsafe {
        let mut attr: hlf_T = HLF_COUNT;
        let mut buf: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut do_combine = false;
        let mut did_intern = false;
        ptr::write_bytes(su, 0, 1);
        ga_init(&raw mut (*su).su_ga, size_of::<suggest_T>() as c_int, 10);
        ga_init(&raw mut (*su).su_sga, size_of::<suggest_T>() as c_int, 10);
        if *badptr == NUL {
            return;
        }
        hash_init(&raw mut (*su).su_banned);
        (*su).su_badptr = badptr;
        if badlen != 0 {
            (*su).su_badlen = badlen;
        } else {
            let tmplen = spell_check(
                curwin,
                (*su).su_badptr,
                &raw mut attr,
                ptr::null_mut(),
                false,
            );
            (*su).su_badlen = tmplen as c_int;
        }
        (*su).su_maxcount = maxcount;
        (*su).su_maxscore = SCORE_MAXINIT;
        (*su).su_badlen = (*su).su_badlen.min(MAXWLEN as c_int - 1);
        xmemcpyz(
            (&raw mut (*su).su_badword).cast(),
            (*su).su_badptr.cast(),
            (*su).su_badlen as usize,
        );
        spell_casefold(
            curwin,
            (*su).su_badptr,
            (*su).su_badlen,
            (&raw mut (*su).su_fbadword).cast(),
            MAXWLEN as c_int,
        );
        (*su).su_fbadword[(*su).su_badlen as usize] = NUL;
        (*su).su_badflags = badword_captype(
            (*su).su_badptr,
            (*su).su_badptr.add((*su).su_badlen as usize),
        );
        if need_cap {
            (*su).su_badflags |= WF_ONECAP;
        }
        for i in 0..((*curbuf).b_s.b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*curbuf).b_s.b_langp, i);
            if !(*lp).lp_sallang.is_null() {
                (*su).su_sallang = (*lp).lp_sallang;
                break;
            }
        }
        if !(*su).su_sallang.is_null() {
            spell_soundfold(
                (*su).su_sallang,
                (&raw mut (*su).su_fbadword).cast(),
                true,
                (&raw mut (*su).su_sal_badword).cast(),
            );
        }
        let c = utf_ptr2char((*su).su_badptr);
        if !spell_isupper(c) && attr == HLF_COUNT {
            make_case_word(
                (&raw mut (*su).su_badword).cast(),
                (&raw mut buf).cast(),
                WF_ONECAP,
            );
            add_suggestion(
                su,
                &raw mut (*su).su_ga,
                (&raw const buf).cast(),
                (*su).su_badlen,
                SCORE_ICASE,
                0,
                true,
                (*su).su_sallang,
                false,
            );
        }
        if banbadword {
            add_banned(su, (&raw mut (*su).su_badword).cast());
        }
        let sps_copy = xstrdup(p_sps);
        let mut p = sps_copy;
        while *p != NUL {
            copy_option_part(&raw mut p, (&raw mut buf).cast(), MAXPATHL, c",".as_ptr());
            let buf_ptr = (&raw mut buf).cast::<c_char>();
            if strncmp(buf_ptr, c"expr:".as_ptr(), 5) == 0 {
                if !expr_busy {
                    expr_busy = true;
                    spell_suggest_expr(su, buf_ptr.add(5));
                    expr_busy = false;
                }
            } else if strncmp(buf_ptr, c"file:".as_ptr(), 5) == 0 {
                spell_suggest_file(su, buf_ptr.add(5));
            } else if strncmp(buf_ptr, c"timeout:".as_ptr(), 8) == 0 {
                spell_suggest_timeout = atoi(buf_ptr.add(8)) as i64;
            } else if !did_intern {
                spell_suggest_intern(su, interactive);
                if (sps_flags & SPS_DOUBLE) != 0 {
                    do_combine = true;
                }
                did_intern = true;
            }
        }
        xfree(sps_copy.cast());
        if do_combine {
            score_combine(su);
        }
    }
}
unsafe fn spell_suggest_expr(su: *mut suginfo_T, expr: *mut c_char) {
    unsafe {
        let mut p: *const c_char = ptr::null();
        let list = eval_spell_expr((&raw mut (*su).su_badword).cast(), expr);
        if !list.is_null() {
            let mut li: *mut listitem_T = (*list).lv_first;
            while !li.is_null() {
                if (*li).li_tv.v_type == VarType::VAR_LIST {
                    let score = get_spellword((*li).li_tv.vval.v_list, &raw mut p);
                    if score >= 0 && score <= (*su).su_maxscore {
                        add_suggestion(
                            su,
                            &raw mut (*su).su_ga,
                            p,
                            (*su).su_badlen,
                            score,
                            0,
                            true,
                            (*su).su_sallang,
                            false,
                        );
                    }
                }
                li = (*li).li_next;
            }
            tv_list_unref(list);
        }
        check_suggestions(su, &raw mut (*su).su_ga);
        cleanup_suggestions(&raw mut (*su).su_ga, (*su).su_maxscore, (*su).su_maxcount);
    }
}
unsafe fn spell_suggest_file(su: *mut suginfo_T, fname: *mut c_char) {
    unsafe {
        let mut line: [c_char; MAXWLEN * 2] = [0; MAXWLEN * 2];
        let mut cword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let fd = os_fopen(fname, c"r".as_ptr());
        if fd.is_null() {
            semsg(gettext((&raw const e_notopen).cast()), fname);
            return;
        }
        while !vim_fgets((&raw mut line).cast(), (MAXWLEN * 2) as c_int, fd) && !got_int {
            line_breakcheck();
            let mut p = vim_strchr((&raw const line).cast(), b'/' as c_int);
            if p.is_null() {
                continue;
            }
            *p = NUL;
            p = p.add(1);
            if mb_stricmp(
                (&raw const (*su).su_badword).cast(),
                (&raw const line).cast(),
            ) == 0
            {
                let mut len: usize = 0;
                while (*p.add(len) as u8) >= b' ' {
                    len += 1;
                }
                *p.add(len) = NUL;
                if captype(p, ptr::null()) == 0 {
                    make_case_word(p, (&raw mut cword).cast(), (*su).su_badflags);
                    p = (&raw mut cword).cast();
                }
                add_suggestion(
                    su,
                    &raw mut (*su).su_ga,
                    p,
                    (*su).su_badlen,
                    SCORE_FILE,
                    0,
                    true,
                    (*su).su_sallang,
                    false,
                );
            }
        }
        fclose(fd);
        check_suggestions(su, &raw mut (*su).su_ga);
        cleanup_suggestions(&raw mut (*su).su_ga, (*su).su_maxscore, (*su).su_maxcount);
    }
}
unsafe fn spell_suggest_intern(su: *mut suginfo_T, interactive: bool) {
    unsafe {
        suggest_load_files();
        suggest_try_special(su);
        suggest_try_change(su);
        if (sps_flags & SPS_DOUBLE) != 0 {
            score_comp_sal(su);
        }
        if (sps_flags & SPS_FAST) == 0 {
            if (sps_flags & SPS_BEST) != 0 {
                rescore_suggestions(su);
            }
            suggest_try_soundalike_prep();
            (*su).su_maxscore = SCORE_SFMAX1;
            (*su).su_sfmaxscore = SCORE_MAXINIT * 3;
            suggest_try_soundalike(su);
            if (*su).su_ga.ga_len < sug_clean_count(su) {
                (*su).su_maxscore = SCORE_SFMAX2;
                suggest_try_soundalike(su);
                if (*su).su_ga.ga_len < sug_clean_count(su) {
                    (*su).su_maxscore = SCORE_SFMAX3;
                    suggest_try_soundalike(su);
                }
            }
            (*su).su_maxscore = (*su).su_sfmaxscore;
            suggest_try_soundalike_finish();
        }
        os_breakcheck();
        if interactive && got_int {
            let _ = vgetc();
            got_int = false;
        }
        if (sps_flags & SPS_DOUBLE) == 0 && (*su).su_ga.ga_len != 0 {
            if (sps_flags & SPS_BEST) != 0 {
                rescore_suggestions(su);
            }
            check_suggestions(su, &raw mut (*su).su_ga);
            cleanup_suggestions(&raw mut (*su).su_ga, (*su).su_maxscore, (*su).su_maxcount);
        }
    }
}
unsafe fn spell_find_cleanup(su: *mut suginfo_T) {
    unsafe {
        for i in 0..((*su).su_ga.ga_len as usize) {
            xfree((*sug_at(&raw const (*su).su_ga, i)).st_word.cast());
        }
        ga_clear(&raw mut (*su).su_ga);
        for i in 0..((*su).su_sga.ga_len as usize) {
            xfree((*sug_at(&raw const (*su).su_sga, i)).st_word.cast());
        }
        ga_clear(&raw mut (*su).su_sga);
        hash_clear_all(&raw mut (*su).su_banned, 0);
    }
}
unsafe fn suggest_try_special(su: *mut suginfo_T) {
    unsafe {
        let mut word: [c_char; MAXWLEN] = [0; MAXWLEN];
        let fbadword_ptr = (&raw mut (*su).su_fbadword).cast::<c_char>();
        let mut p = skiptowhite(fbadword_ptr);
        let len = p.offset_from(fbadword_ptr) as usize;
        p = skipwhite(p);
        if strlen(p) == len && strncmp(fbadword_ptr, p, len) == 0 {
            let c = (*su).su_fbadword[len];
            (*su).su_fbadword[len] = NUL;
            make_case_word(fbadword_ptr, (&raw mut word).cast(), (*su).su_badflags);
            (*su).su_fbadword[len] = c;
            add_suggestion(
                su,
                &raw mut (*su).su_ga,
                (&raw const word).cast(),
                (*su).su_badlen,
                rescore(SCORE_REP, 0),
                0,
                true,
                (*su).su_sallang,
                false,
            );
        }
    }
}
unsafe fn suggest_try_change(su: *mut suginfo_T) {
    unsafe {
        let mut fword: [c_char; MAXWLEN] = [0; MAXWLEN];
        strcpy(
            (&raw mut fword).cast(),
            (&raw const (*su).su_fbadword).cast(),
        );
        let mut n = strlen((&raw const fword).cast()) as c_int;
        let p = (*su).su_badptr.add((*su).su_badlen as usize);
        spell_casefold(
            curwin,
            p,
            strlen(p) as c_int,
            (&raw mut fword).cast::<c_char>().add(n as usize),
            MAXWLEN as c_int - n,
        );
        n = strlen((*su).su_badptr) as c_int;
        if (n as usize) < MAXWLEN {
            fword[n as usize] = NUL;
        }
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            if (*(*lp).lp_slang).sl_fbyts.is_null() {
                continue;
            }
            suggest_trie_walk(su, lp, (&raw mut fword).cast(), false);
        }
    }
}
#[inline]
unsafe fn try_deeper(
    su: *const suginfo_T,
    stack: &[trystate_T; MAXWLEN],
    depth: usize,
    add: c_int,
) -> bool {
    unsafe { depth < MAXWLEN - 1 && stack[depth].ts_score + add < (*su).su_maxscore }
}
unsafe fn suggest_trie_walk(
    su: *mut suginfo_T,
    lp: *mut langp_T,
    fword: *mut c_char,
    soundfold: bool,
) {
    unsafe {
        let mut tword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut stack: [trystate_T; MAXWLEN] = [trystate_T::default(); MAXWLEN];
        let mut preword: [c_char; MAXWLEN * 3] = [0; MAXWLEN * 3];
        let mut compflags: [u8; MAXWLEN] = [0; MAXWLEN];
        let mut byts: *mut u8;
        let fbyts: *mut u8;
        let pbyts: *mut u8;
        let mut idxs: *mut idx_T;
        let fidxs: *mut idx_T;
        let pidxs: *mut idx_T;
        let mut c: c_int;
        let mut c2: c_int;
        let mut c3: c_int;
        let mut n: c_int;
        let mut arridx: idx_T;
        let mut fl: c_int;
        let mut tl: c_int;
        let mut repextra: c_int = 0;
        let slang = (*lp).lp_slang;
        let mut goodword_ends: bool;
        let mut breakcheckcount: c_int = 1000;
        let mut depth: isize = 0;
        stack[0] = trystate_T::default();
        stack[0].ts_curi = 1;
        if soundfold {
            fbyts = (*slang).sl_sbyts;
            byts = fbyts;
            fidxs = (*slang).sl_sidxs;
            idxs = fidxs;
            pbyts = ptr::null_mut();
            pidxs = ptr::null_mut();
            stack[0].ts_prefixdepth = PFD_NOPREFIX;
            stack[0].ts_state = state_T::STATE_START;
        } else {
            fbyts = (*slang).sl_fbyts;
            fidxs = (*slang).sl_fidxs;
            pbyts = (*slang).sl_pbyts;
            pidxs = (*slang).sl_pidxs;
            if !pbyts.is_null() {
                byts = pbyts;
                idxs = pidxs;
                stack[0].ts_prefixdepth = PFD_PREFIXTREE;
                stack[0].ts_state = state_T::STATE_NOPREFIX;
            } else {
                byts = fbyts;
                idxs = fidxs;
                stack[0].ts_prefixdepth = PFD_NOPREFIX;
                stack[0].ts_state = state_T::STATE_START;
            }
        }
        let mut time_limit: proftime_T = 0;
        if spell_suggest_timeout > 0 {
            time_limit = profile_setlimit(spell_suggest_timeout);
        }
        while depth >= 0 && !got_int {
            let mut udepth = depth as usize;
            let state = stack[udepth].ts_state;
            match state {
                state_T::STATE_START | state_T::STATE_NOPREFIX => {
                    arridx = stack[udepth].ts_arridx;
                    let mut len = *byts.add(arridx as usize) as c_int;
                    arridx += stack[udepth].ts_curi as idx_T;
                    if stack[udepth].ts_prefixdepth == PFD_PREFIXTREE {
                        n = 0;
                        while n < len && *byts.add((arridx + n) as usize) == 0 {
                            n += 1;
                        }
                        stack[udepth].ts_curi += n as i16;
                        let was_noprefix = stack[udepth].ts_state == state_T::STATE_NOPREFIX;
                        stack[udepth].ts_state = state_T::STATE_ENDNUL;
                        stack[udepth].ts_save_badflags = (*su).su_badflags as u8;
                        if udepth < MAXWLEN - 1 && (*byts.add(arridx as usize) == 0 || was_noprefix)
                        {
                            n = nofold_len(fword, stack[udepth].ts_fidx as c_int, (*su).su_badptr);
                            let flags =
                                badword_captype((*su).su_badptr, (*su).su_badptr.add(n as usize));
                            (*su).su_badflags = badword_captype(
                                (*su).su_badptr.add(n as usize),
                                (*su).su_badptr.add((*su).su_badlen as usize),
                            );
                            go_deeper(&mut stack, udepth, 0);
                            depth += 1;
                            udepth = depth as usize;
                            stack[udepth].ts_prefixdepth = (udepth - 1) as u8;
                            byts = fbyts;
                            idxs = fidxs;
                            stack[udepth].ts_arridx = 0;
                            tword[stack[udepth].ts_twordlen as usize] = NUL;
                            make_case_word(
                                (&raw mut tword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_splitoff as usize),
                                (&raw mut preword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_prewordlen as usize),
                                flags,
                            );
                            stack[udepth].ts_prewordlen = strlen((&raw const preword).cast()) as u8;
                            stack[udepth].ts_splitoff = stack[udepth].ts_twordlen;
                        }
                        continue;
                    }
                    if stack[udepth].ts_curi as c_int > len || *byts.add(arridx as usize) != 0 {
                        stack[udepth].ts_state = state_T::STATE_ENDNUL;
                        stack[udepth].ts_save_badflags = (*su).su_badflags as u8;
                        continue;
                    }
                    stack[udepth].ts_curi += 1;
                    let mut flags = *idxs.add(arridx as usize);
                    if (flags & WF_NOSUGGEST) != 0 {
                        continue;
                    }
                    let fword_ends = *fword.add(stack[udepth].ts_fidx as usize) == NUL
                        || if soundfold {
                            ascii_iswhite(*fword.add(stack[udepth].ts_fidx as usize))
                        } else {
                            !spell_iswordp(fword.add(stack[udepth].ts_fidx as usize), curwin)
                        };
                    tword[stack[udepth].ts_twordlen as usize] = NUL;
                    if stack[udepth].ts_prefixdepth <= PFD_NOTSPECIAL
                        && (stack[udepth].ts_flags & TSF_PREFIXOK) == 0
                        && !pbyts.is_null()
                    {
                        n = stack[stack[udepth].ts_prefixdepth as usize].ts_arridx;
                        len = *pbyts.add(n as usize) as c_int;
                        n += 1;
                        c = 0;
                        while c < len && *pbyts.add((n + c) as usize) == 0 {
                            c += 1;
                        }
                        if c > 0 {
                            c = valid_word_prefix(
                                c,
                                n,
                                flags,
                                (&raw mut tword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_splitoff as usize),
                                slang,
                                false,
                            );
                            if c == 0 {
                                continue;
                            }
                            if (c & WF_RAREPFX) != 0 {
                                flags |= WF_RARE;
                            }
                            stack[udepth].ts_flags |= TSF_PREFIXOK;
                        }
                    }
                    goodword_ends = !(stack[udepth].ts_complen == stack[udepth].ts_compsplit
                        && fword_ends
                        && (flags & WF_NEEDCOMP) != 0);
                    let mut p: *mut c_char = ptr::null_mut();
                    let mut compound_ok = true;
                    if stack[udepth].ts_complen > stack[udepth].ts_compsplit {
                        if (*slang).sl_nobreak {
                            if stack[udepth].ts_fidx - stack[udepth].ts_splitfidx
                                == stack[udepth].ts_twordlen - stack[udepth].ts_splitoff
                                && strncmp(
                                    fword.add(stack[udepth].ts_splitfidx as usize),
                                    (&raw const tword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_splitoff as usize),
                                    (stack[udepth].ts_fidx - stack[udepth].ts_splitfidx) as usize,
                                ) == 0
                            {
                                preword[stack[udepth].ts_prewordlen as usize] = NUL;
                                let newscore = score_wordcount_adj(
                                    slang,
                                    stack[udepth].ts_score,
                                    (&raw const preword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_prewordlen as usize),
                                    stack[udepth].ts_prewordlen > 0,
                                );
                                if newscore <= (*su).su_maxscore {
                                    add_suggestion(
                                        su,
                                        &raw mut (*su).su_ga,
                                        (&raw const preword).cast(),
                                        stack[udepth].ts_splitfidx as c_int - repextra,
                                        newscore,
                                        0,
                                        false,
                                        (*lp).lp_sallang,
                                        false,
                                    );
                                }
                                continue;
                            }
                        } else {
                            if ((flags as u32) >> 24) == 0
                                || c_int::from(
                                    stack[udepth].ts_twordlen - stack[udepth].ts_splitoff,
                                ) < (*slang).sl_compminlen
                            {
                                continue;
                            }
                            if (*slang).sl_compminlen > 0
                                && mb_charlen(
                                    (&raw const tword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_splitoff as usize),
                                ) < (*slang).sl_compminlen
                            {
                                continue;
                            }
                            compflags[stack[udepth].ts_complen as usize] =
                                ((flags as u32) >> 24) as u8;
                            compflags[stack[udepth].ts_complen as usize + 1] = 0;
                            xmemcpyz(
                                (&raw mut preword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_prewordlen as usize)
                                    .cast(),
                                (&raw const tword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_splitoff as usize)
                                    .cast(),
                                (stack[udepth].ts_twordlen - stack[udepth].ts_splitoff) as usize,
                            );
                            if match_checkcompoundpattern(
                                (&raw mut preword).cast(),
                                stack[udepth].ts_prewordlen as c_int,
                                &raw mut (*slang).sl_comppat,
                            ) {
                                compound_ok = false;
                            }
                            if compound_ok {
                                p = (&raw mut preword).cast();
                                while *skiptowhite(p) != NUL {
                                    p = skipwhite(skiptowhite(p));
                                }
                                if fword_ends
                                    && !can_compound(
                                        slang,
                                        p,
                                        (&raw const compflags)
                                            .cast::<u8>()
                                            .add(stack[udepth].ts_compsplit as usize),
                                    )
                                {
                                    compound_ok = false;
                                }
                            }
                            p = (&raw mut preword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_prewordlen as usize);
                            mb_ptr_back((&raw const preword).cast(), &mut p);
                        }
                    }
                    if soundfold {
                        strcpy(
                            (&raw mut preword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_prewordlen as usize),
                            (&raw const tword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_splitoff as usize),
                        );
                    } else if (flags & WF_KEEPCAP) != 0 {
                        find_keepcap_word(
                            slang,
                            (&raw mut tword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_splitoff as usize),
                            (&raw mut preword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_prewordlen as usize),
                        );
                    } else {
                        c = (*su).su_badflags;
                        if (c & WF_ALLCAP) != 0 && (*su).su_badlen == utfc_ptr2len((*su).su_badptr)
                        {
                            c = WF_ONECAP;
                        }
                        c |= flags;
                        if !p.is_null() && spell_iswordp_nmw(p, curwin) {
                            c &= !WF_ONECAP;
                        }
                        make_case_word(
                            (&raw mut tword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_splitoff as usize),
                            (&raw mut preword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_prewordlen as usize),
                            c,
                        );
                    }
                    if !soundfold {
                        if (flags & WF_BANNED) != 0 {
                            add_banned(
                                su,
                                (&raw mut preword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_prewordlen as usize),
                            );
                            continue;
                        }
                        if (stack[udepth].ts_complen == stack[udepth].ts_compsplit
                            && was_banned(
                                su,
                                (&raw const preword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_prewordlen as usize),
                            ))
                            || was_banned(su, (&raw const preword).cast())
                        {
                            if (*slang).sl_compprog.is_null() {
                                continue;
                            }
                            goodword_ends = false;
                        }
                    }
                    let mut newscore: c_int = 0;
                    if !soundfold {
                        if (flags & WF_REGION) != 0
                            && (((flags as u32) >> 16) & ((*lp).lp_region as u32)) == 0
                        {
                            newscore += SCORE_REGION;
                        }
                        if (flags & WF_RARE) != 0 {
                            newscore += SCORE_RARE;
                        }
                        if !spell_valid_case(
                            (*su).su_badflags,
                            captype(
                                (&raw const preword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_prewordlen as usize),
                                ptr::null(),
                            ),
                        ) {
                            newscore += SCORE_ICASE;
                        }
                    }
                    if fword_ends
                        && goodword_ends
                        && stack[udepth].ts_fidx >= stack[udepth].ts_fidxtry
                        && compound_ok
                    {
                        if soundfold {
                            add_sound_suggest(
                                su,
                                (&raw mut preword).cast(),
                                stack[udepth].ts_score,
                                lp,
                            );
                        } else if stack[udepth].ts_fidx > 0 {
                            p = fword.add(stack[udepth].ts_fidx as usize);
                            mb_ptr_back(fword, &mut p);
                            if !spell_iswordp(p, curwin) && preword[0] != NUL {
                                p = (&raw mut preword)
                                    .cast::<c_char>()
                                    .add(strlen((&raw const preword).cast()));
                                mb_ptr_back((&raw const preword).cast(), &mut p);
                                if spell_iswordp(p, curwin) {
                                    newscore += SCORE_NONWORD;
                                }
                            }
                            let score = score_wordcount_adj(
                                slang,
                                stack[udepth].ts_score + newscore,
                                (&raw const preword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_prewordlen as usize),
                                stack[udepth].ts_prewordlen > 0,
                            );
                            if score <= (*su).su_maxscore {
                                add_suggestion(
                                    su,
                                    &raw mut (*su).su_ga,
                                    (&raw const preword).cast(),
                                    stack[udepth].ts_fidx as c_int - repextra,
                                    score,
                                    0,
                                    false,
                                    (*lp).lp_sallang,
                                    false,
                                );
                                if ((*su).su_badflags & WF_MIXCAP) != 0 {
                                    c = captype((&raw const preword).cast(), ptr::null());
                                    if c == 0 || c == WF_ALLCAP {
                                        make_case_word(
                                            (&raw mut tword)
                                                .cast::<c_char>()
                                                .add(stack[udepth].ts_splitoff as usize),
                                            (&raw mut preword)
                                                .cast::<c_char>()
                                                .add(stack[udepth].ts_prewordlen as usize),
                                            if c == 0 { WF_ALLCAP } else { 0 },
                                        );
                                        add_suggestion(
                                            su,
                                            &raw mut (*su).su_ga,
                                            (&raw const preword).cast(),
                                            stack[udepth].ts_fidx as c_int - repextra,
                                            score + SCORE_ICASE,
                                            0,
                                            false,
                                            (*lp).lp_sallang,
                                            false,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    if (stack[udepth].ts_fidx >= stack[udepth].ts_fidxtry || fword_ends)
                        && stack[udepth].ts_tcharlen == 0
                    {
                        let try_split = (stack[udepth].ts_fidx as c_int - repextra)
                            < (*su).su_badlen
                            && !soundfold;
                        let mut try_compound = false;
                        if !soundfold
                            && !(*slang).sl_nocompoundsugs
                            && !(*slang).sl_compprog.is_null()
                            && ((flags as u32) >> 24) != 0
                            && (stack[udepth].ts_twordlen - stack[udepth].ts_splitoff) as c_int
                                >= (*slang).sl_compminlen
                            && ((*slang).sl_compminlen == 0
                                || mb_charlen(
                                    (&raw const tword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_splitoff as usize),
                                ) >= (*slang).sl_compminlen)
                            && ((*slang).sl_compsylmax < MAXWLEN as c_int
                                || c_int::from(
                                    stack[udepth].ts_complen + 1 - stack[udepth].ts_compsplit,
                                ) < (*slang).sl_compmax)
                            && can_be_compound(
                                &raw const stack[udepth],
                                slang,
                                (&raw mut compflags).cast(),
                                ((flags as u32) >> 24) as c_int,
                            )
                        {
                            try_compound = true;
                            compflags[stack[udepth].ts_complen as usize] =
                                ((flags as u32) >> 24) as u8;
                            compflags[stack[udepth].ts_complen as usize + 1] = 0;
                        }
                        if (*slang).sl_nobreak && !(*slang).sl_nocompoundsugs {
                            try_compound = true;
                        } else if !fword_ends
                            && try_compound
                            && (stack[udepth].ts_flags & TSF_DIDSPLIT) == 0
                        {
                            try_compound = false;
                            stack[udepth].ts_flags |= TSF_DIDSPLIT;
                            stack[udepth].ts_curi -= 1;
                            compflags[stack[udepth].ts_complen as usize] = 0;
                        } else {
                            stack[udepth].ts_flags &= !TSF_DIDSPLIT;
                        }
                        if try_split || try_compound {
                            if !try_compound && (!fword_ends || !goodword_ends) {
                                if stack[udepth].ts_complen == stack[udepth].ts_compsplit
                                    && (flags & WF_NEEDCOMP) != 0
                                {
                                    continue;
                                }
                                p = (&raw mut preword).cast();
                                while *skiptowhite(p) != NUL {
                                    p = skipwhite(skiptowhite(p));
                                }
                                if stack[udepth].ts_complen > stack[udepth].ts_compsplit
                                    && !can_compound(
                                        slang,
                                        p,
                                        (&raw const compflags)
                                            .cast::<u8>()
                                            .add(stack[udepth].ts_compsplit as usize),
                                    )
                                {
                                    continue;
                                }
                                if (*slang).sl_nosplitsugs {
                                    newscore += SCORE_SPLIT_NO;
                                } else {
                                    newscore += SCORE_SPLIT;
                                }
                                newscore = score_wordcount_adj(
                                    slang,
                                    newscore,
                                    (&raw const preword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_prewordlen as usize),
                                    true,
                                );
                            }
                            if try_deeper(su, &stack, udepth, newscore) {
                                go_deeper(&mut stack, udepth, newscore);
                                stack[udepth].ts_save_badflags = (*su).su_badflags as u8;
                                stack[udepth].ts_state = state_T::STATE_SPLITUNDO;
                                depth += 1;
                                udepth = depth as usize;
                                if !try_compound && !fword_ends {
                                    strcat((&raw mut preword).cast(), c" ".as_ptr());
                                }
                                stack[udepth].ts_prewordlen =
                                    strlen((&raw const preword).cast()) as u8;
                                stack[udepth].ts_splitoff = stack[udepth].ts_twordlen;
                                stack[udepth].ts_splitfidx = stack[udepth].ts_fidx;
                                if ((!try_compound
                                    && !spell_iswordp_nmw(
                                        fword.add(stack[udepth].ts_fidx as usize),
                                        curwin,
                                    ))
                                    || fword_ends)
                                    && *fword.add(stack[udepth].ts_fidx as usize) != NUL
                                    && goodword_ends
                                {
                                    let l = utfc_ptr2len(fword.add(stack[udepth].ts_fidx as usize));
                                    if fword_ends {
                                        ptr::copy(
                                            fword.add(stack[udepth].ts_fidx as usize),
                                            (&raw mut preword)
                                                .cast::<c_char>()
                                                .add(stack[udepth].ts_prewordlen as usize),
                                            l as usize,
                                        );
                                        stack[udepth].ts_prewordlen += l as u8;
                                        preword[stack[udepth].ts_prewordlen as usize] = NUL;
                                    } else {
                                        stack[udepth].ts_score -= SCORE_SPLIT - SCORE_SUBST;
                                    }
                                    stack[udepth].ts_fidx += l as u8;
                                }
                                if try_compound {
                                    stack[udepth].ts_complen += 1;
                                } else {
                                    stack[udepth].ts_compsplit = stack[udepth].ts_complen;
                                }
                                stack[udepth].ts_prefixdepth = PFD_NOPREFIX;
                                n = nofold_len(
                                    fword,
                                    stack[udepth].ts_fidx as c_int,
                                    (*su).su_badptr,
                                );
                                (*su).su_badflags = badword_captype(
                                    (*su).su_badptr.add(n as usize),
                                    (*su).su_badptr.add((*su).su_badlen as usize),
                                );
                                stack[udepth].ts_arridx = 0;
                                if !pbyts.is_null() {
                                    byts = pbyts;
                                    idxs = pidxs;
                                    stack[udepth].ts_prefixdepth = PFD_PREFIXTREE;
                                    stack[udepth].ts_state = state_T::STATE_NOPREFIX;
                                }
                            }
                        }
                    }
                }
                state_T::STATE_SPLITUNDO => {
                    (*su).su_badflags = stack[udepth].ts_save_badflags as c_int;
                    stack[udepth].ts_state = state_T::STATE_START;
                    byts = fbyts;
                    idxs = fidxs;
                }
                state_T::STATE_ENDNUL | state_T::STATE_PLAIN => {
                    let mut goto_del = false;
                    if state == state_T::STATE_ENDNUL {
                        (*su).su_badflags = stack[udepth].ts_save_badflags as c_int;
                        if *fword.add(stack[udepth].ts_fidx as usize) == NUL
                            && stack[udepth].ts_tcharlen == 0
                        {
                            stack[udepth].ts_state = state_T::STATE_DEL;
                            goto_del = true;
                        } else {
                            stack[udepth].ts_state = state_T::STATE_PLAIN;
                        }
                    }
                    if !goto_del {
                        arridx = stack[udepth].ts_arridx;
                        if stack[udepth].ts_curi as c_int > *byts.add(arridx as usize) as c_int {
                            stack[udepth].ts_state =
                                if stack[udepth].ts_fidx >= stack[udepth].ts_fidxtry {
                                    state_T::STATE_DEL
                                } else {
                                    state_T::STATE_FINAL
                                };
                        } else {
                            arridx += stack[udepth].ts_curi as idx_T;
                            stack[udepth].ts_curi += 1;
                            c = *byts.add(arridx as usize) as c_int;
                            let newscore = if c
                                == *fword.add(stack[udepth].ts_fidx as usize) as u8 as c_int
                                || (stack[udepth].ts_tcharlen > 0
                                    && stack[udepth].ts_isdiff != DIFF_NONE)
                            {
                                0
                            } else {
                                SCORE_SUBST
                            };
                            if (newscore == 0
                                || (stack[udepth].ts_fidx >= stack[udepth].ts_fidxtry
                                    && ((stack[udepth].ts_flags & TSF_DIDDEL) == 0
                                        || c != *fword.add(stack[udepth].ts_delidx as usize) as u8
                                            as c_int)))
                                && try_deeper(su, &stack, udepth, newscore)
                            {
                                go_deeper(&mut stack, udepth, newscore);
                                depth += 1;
                                udepth = depth as usize;
                                if *fword.add(stack[udepth].ts_fidx as usize) != NUL {
                                    stack[udepth].ts_fidx += 1;
                                }
                                let twl = stack[udepth].ts_twordlen as usize;
                                tword[twl] = c as c_char;
                                stack[udepth].ts_twordlen += 1;
                                stack[udepth].ts_arridx = *idxs.add(arridx as usize);
                                if newscore == SCORE_SUBST {
                                    stack[udepth].ts_isdiff = DIFF_YES;
                                }
                                if stack[udepth].ts_tcharlen == 0 {
                                    stack[udepth].ts_tcharidx = 0;
                                    stack[udepth].ts_tcharlen = utf_byte2len(c) as u8;
                                    stack[udepth].ts_fcharstart = stack[udepth].ts_fidx - 1;
                                    stack[udepth].ts_isdiff =
                                        if newscore != 0 { DIFF_YES } else { DIFF_NONE };
                                } else if stack[udepth].ts_isdiff == DIFF_INSERT
                                    && stack[udepth].ts_fidx > 0
                                {
                                    stack[udepth].ts_fidx -= 1;
                                }
                                stack[udepth].ts_tcharidx += 1;
                                if stack[udepth].ts_tcharidx == stack[udepth].ts_tcharlen {
                                    if stack[udepth].ts_isdiff == DIFF_YES {
                                        stack[udepth].ts_fidx = stack[udepth].ts_fcharstart
                                            + utfc_ptr2len(
                                                fword.add(stack[udepth].ts_fcharstart as usize),
                                            ) as u8;
                                        let tw_ptr = (&raw const tword).cast::<c_char>().add(
                                            (stack[udepth].ts_twordlen - stack[udepth].ts_tcharlen)
                                                as usize,
                                        );
                                        let fw_ptr =
                                            fword.add(stack[udepth].ts_fcharstart as usize);
                                        if utf_iscomposing_legacy(utf_ptr2char(tw_ptr))
                                            && utf_iscomposing_legacy(utf_ptr2char(fw_ptr))
                                        {
                                            stack[udepth].ts_score -= SCORE_SUBST - SCORE_SUBCOMP;
                                        } else if !soundfold
                                            && (*slang).sl_has_map
                                            && similar_chars(
                                                slang,
                                                utf_ptr2char(tw_ptr),
                                                utf_ptr2char(fw_ptr),
                                            )
                                        {
                                            stack[udepth].ts_score -= SCORE_SUBST - SCORE_SIMILAR;
                                        }
                                    } else if stack[udepth].ts_isdiff == DIFF_INSERT
                                        && stack[udepth].ts_twordlen > stack[udepth].ts_tcharlen
                                    {
                                        let mut p = (&raw mut tword).cast::<c_char>().add(
                                            (stack[udepth].ts_twordlen - stack[udepth].ts_tcharlen)
                                                as usize,
                                        );
                                        c = utf_ptr2char(p);
                                        if utf_iscomposing_legacy(c) {
                                            stack[udepth].ts_score -= SCORE_INS - SCORE_INSCOMP;
                                        } else {
                                            mb_ptr_back((&raw const tword).cast(), &mut p);
                                            if c == utf_ptr2char(p) {
                                                stack[udepth].ts_score -= SCORE_INS - SCORE_INSDUP;
                                            }
                                        }
                                    }
                                    stack[udepth].ts_tcharlen = 0;
                                }
                            }
                        }
                    }
                }
                state_T::STATE_DEL | state_T::STATE_INS_PREP => {
                    let mut do_ins_prep = true;
                    if state == state_T::STATE_DEL {
                        if stack[udepth].ts_tcharlen > 0 {
                            stack[udepth].ts_state = state_T::STATE_FINAL;
                            do_ins_prep = false;
                        } else {
                            stack[udepth].ts_state = state_T::STATE_INS_PREP;
                            stack[udepth].ts_curi = 1;
                            let newscore = if soundfold
                                && stack[udepth].ts_fidx == 0
                                && *fword.add(stack[udepth].ts_fidx as usize) == b'*' as c_char
                            {
                                2 * SCORE_DEL / 3
                            } else {
                                SCORE_DEL
                            };
                            if *fword.add(stack[udepth].ts_fidx as usize) != NUL
                                && try_deeper(su, &stack, udepth, newscore)
                            {
                                go_deeper(&mut stack, udepth, newscore);
                                let old_fidx = stack[udepth].ts_fidx;
                                depth += 1;
                                udepth = depth as usize;
                                stack[udepth].ts_flags |= TSF_DIDDEL;
                                stack[udepth].ts_delidx = old_fidx;
                                c = utf_ptr2char(fword.add(old_fidx as usize));
                                stack[udepth].ts_fidx +=
                                    utfc_ptr2len(fword.add(old_fidx as usize)) as u8;
                                if utf_iscomposing_legacy(c) {
                                    stack[udepth].ts_score -= SCORE_DEL - SCORE_DELCOMP;
                                } else if c
                                    == utf_ptr2char(fword.add(stack[udepth].ts_fidx as usize))
                                {
                                    stack[udepth].ts_score -= SCORE_DEL - SCORE_DELDUP;
                                }
                                do_ins_prep = false;
                            }
                        }
                    }
                    if do_ins_prep {
                        if (stack[udepth].ts_flags & TSF_DIDDEL) != 0 {
                            stack[udepth].ts_state = state_T::STATE_SWAP;
                        } else {
                            n = stack[udepth].ts_arridx;
                            loop {
                                if stack[udepth].ts_curi as c_int > *byts.add(n as usize) as c_int {
                                    stack[udepth].ts_state = state_T::STATE_SWAP;
                                    break;
                                }
                                if *byts.add((n + stack[udepth].ts_curi as c_int) as usize) != 0 {
                                    stack[udepth].ts_state = state_T::STATE_INS;
                                    break;
                                }
                                stack[udepth].ts_curi += 1;
                            }
                        }
                    }
                }
                state_T::STATE_INS => {
                    n = stack[udepth].ts_arridx;
                    if stack[udepth].ts_curi as c_int > *byts.add(n as usize) as c_int {
                        stack[udepth].ts_state = state_T::STATE_SWAP;
                        continue;
                    }
                    n += stack[udepth].ts_curi as c_int;
                    stack[udepth].ts_curi += 1;
                    if byts == (*slang).sl_fbyts && n >= (*slang).sl_fbyts_len {
                        got_int = true;
                        continue;
                    }
                    c = *byts.add(n as usize) as c_int;
                    let newscore =
                        if soundfold && stack[udepth].ts_twordlen == 0 && c == b'*' as c_int {
                            2 * SCORE_INS / 3
                        } else {
                            SCORE_INS
                        };
                    if c != *fword.add(stack[udepth].ts_fidx as usize) as u8 as c_int
                        && try_deeper(su, &stack, udepth, newscore)
                    {
                        go_deeper(&mut stack, udepth, newscore);
                        depth += 1;
                        udepth = depth as usize;
                        let twl = stack[udepth].ts_twordlen as usize;
                        tword[twl] = c as c_char;
                        stack[udepth].ts_twordlen += 1;
                        stack[udepth].ts_arridx = *idxs.add(n as usize);
                        fl = utf_byte2len(c);
                        if fl > 1 {
                            stack[udepth].ts_tcharlen = fl as u8;
                            stack[udepth].ts_tcharidx = 1;
                            stack[udepth].ts_isdiff = DIFF_INSERT;
                        }
                        if fl == 1
                            && stack[udepth].ts_twordlen >= 2
                            && tword[stack[udepth].ts_twordlen as usize - 2] as u8 as c_int == c
                        {
                            stack[udepth].ts_score -= SCORE_INS - SCORE_INSDUP;
                        }
                    }
                }
                state_T::STATE_SWAP => {
                    let p = fword.add(stack[udepth].ts_fidx as usize);
                    c = *p as u8 as c_int;
                    if c == NUL as c_int {
                        stack[udepth].ts_state = state_T::STATE_FINAL;
                        continue;
                    }
                    if !soundfold && !spell_iswordp(p, curwin) {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                        continue;
                    }
                    n = utf_ptr2len(p);
                    c = utf_ptr2char(p);
                    if *p.add(n as usize) == NUL {
                        c2 = NUL as c_int;
                    } else if !soundfold && !spell_iswordp(p.add(n as usize), curwin) {
                        c2 = c;
                    } else {
                        c2 = utf_ptr2char(p.add(n as usize));
                    }
                    if c2 == NUL as c_int {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                        continue;
                    }
                    if c == c2 {
                        stack[udepth].ts_state = state_T::STATE_SWAP3;
                        continue;
                    }
                    if try_deeper(su, &stack, udepth, SCORE_SWAP) {
                        go_deeper(&mut stack, udepth, SCORE_SWAP);
                        stack[udepth].ts_state = state_T::STATE_UNSWAP;
                        let old_fidx = stack[udepth].ts_fidx;
                        depth += 1;
                        udepth = depth as usize;
                        fl = utf_char2len(c2);
                        ptr::copy(p.add(n as usize), p, fl as usize);
                        utf_char2bytes(c, p.add(fl as usize));
                        stack[udepth].ts_fidxtry = old_fidx + (n + fl) as u8;
                    } else {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                    }
                }
                state_T::STATE_UNSWAP | state_T::STATE_SWAP3 => {
                    if state == state_T::STATE_UNSWAP {
                        let p = fword.add(stack[udepth].ts_fidx as usize);
                        n = utfc_ptr2len(p);
                        c = utf_ptr2char(p.add(n as usize));
                        ptr::copy(
                            p,
                            p.add(utfc_ptr2len(p.add(n as usize)) as usize),
                            n as usize,
                        );
                        utf_char2bytes(c, p);
                    }
                    let p = fword.add(stack[udepth].ts_fidx as usize);
                    n = utf_ptr2len(p);
                    c = utf_ptr2char(p);
                    fl = utf_ptr2len(p.add(n as usize));
                    c2 = utf_ptr2char(p.add(n as usize));
                    if !soundfold && !spell_iswordp(p.add((n + fl) as usize), curwin) {
                        c3 = c;
                    } else {
                        c3 = utf_ptr2char(p.add((n + fl) as usize));
                    }
                    if c == c3 || c3 == NUL as c_int {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                        continue;
                    }
                    if try_deeper(su, &stack, udepth, SCORE_SWAP3) {
                        go_deeper(&mut stack, udepth, SCORE_SWAP3);
                        stack[udepth].ts_state = state_T::STATE_UNSWAP3;
                        let old_fidx = stack[udepth].ts_fidx;
                        depth += 1;
                        udepth = depth as usize;
                        tl = utf_char2len(c3);
                        ptr::copy(p.add((n + fl) as usize), p, tl as usize);
                        utf_char2bytes(c2, p.add(tl as usize));
                        utf_char2bytes(c, p.add((fl + tl) as usize));
                        stack[udepth].ts_fidxtry = old_fidx + (n + fl + tl) as u8;
                    } else {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                    }
                }
                state_T::STATE_UNSWAP3 => {
                    let mut p = fword.add(stack[udepth].ts_fidx as usize);
                    n = utfc_ptr2len(p);
                    c2 = utf_ptr2char(p.add(n as usize));
                    fl = utfc_ptr2len(p.add(n as usize));
                    c = utf_ptr2char(p.add((n + fl) as usize));
                    tl = utfc_ptr2len(p.add((n + fl) as usize));
                    ptr::copy(p, p.add((fl + tl) as usize), n as usize);
                    utf_char2bytes(c, p);
                    utf_char2bytes(c2, p.add(tl as usize));
                    p = p.add(tl as usize);
                    if !soundfold && !spell_iswordp(p, curwin) {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                        continue;
                    }
                    if try_deeper(su, &stack, udepth, SCORE_SWAP3) {
                        go_deeper(&mut stack, udepth, SCORE_SWAP3);
                        stack[udepth].ts_state = state_T::STATE_UNROT3L;
                        let old_fidx = stack[udepth].ts_fidx;
                        depth += 1;
                        udepth = depth as usize;
                        p = fword.add(old_fidx as usize);
                        n = utf_ptr2len(p);
                        c = utf_ptr2char(p);
                        fl = utf_ptr2len(p.add(n as usize));
                        fl += utf_ptr2len(p.add((n + fl) as usize));
                        ptr::copy(p.add(n as usize), p, fl as usize);
                        utf_char2bytes(c, p.add(fl as usize));
                        stack[udepth].ts_fidxtry = old_fidx + (n + fl) as u8;
                    } else {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                    }
                }
                state_T::STATE_UNROT3L => {
                    let mut p = fword.add(stack[udepth].ts_fidx as usize);
                    n = utfc_ptr2len(p);
                    n += utfc_ptr2len(p.add(n as usize));
                    c = utf_ptr2char(p.add(n as usize));
                    tl = utfc_ptr2len(p.add(n as usize));
                    ptr::copy(p, p.add(tl as usize), n as usize);
                    utf_char2bytes(c, p);
                    if try_deeper(su, &stack, udepth, SCORE_SWAP3) {
                        go_deeper(&mut stack, udepth, SCORE_SWAP3);
                        stack[udepth].ts_state = state_T::STATE_UNROT3R;
                        let old_fidx = stack[udepth].ts_fidx;
                        depth += 1;
                        udepth = depth as usize;
                        p = fword.add(old_fidx as usize);
                        n = utf_ptr2len(p);
                        n += utf_ptr2len(p.add(n as usize));
                        c = utf_ptr2char(p.add(n as usize));
                        tl = utf_ptr2len(p.add(n as usize));
                        ptr::copy(p, p.add(tl as usize), n as usize);
                        utf_char2bytes(c, p);
                        stack[udepth].ts_fidxtry = old_fidx + (n + tl) as u8;
                    } else {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                    }
                }
                state_T::STATE_UNROT3R | state_T::STATE_REP_INI | state_T::STATE_REP => {
                    let mut do_rep = true;
                    if state != state_T::STATE_REP {
                        if state == state_T::STATE_UNROT3R {
                            let p = fword.add(stack[udepth].ts_fidx as usize);
                            c = utf_ptr2char(p);
                            tl = utfc_ptr2len(p);
                            n = utfc_ptr2len(p.add(tl as usize));
                            n += utfc_ptr2len(p.add((tl + n) as usize));
                            ptr::copy(p.add(tl as usize), p, n as usize);
                            utf_char2bytes(c, p.add(n as usize));
                        }
                        if ((*lp).lp_replang.is_null() && !soundfold)
                            || stack[udepth].ts_score + SCORE_REP >= (*su).su_maxscore
                            || stack[udepth].ts_fidx < stack[udepth].ts_fidxtry
                        {
                            stack[udepth].ts_state = state_T::STATE_FINAL;
                            do_rep = false;
                        } else {
                            let first_byte =
                                *fword.add(stack[udepth].ts_fidx as usize) as u8 as usize;
                            stack[udepth].ts_curi = if soundfold {
                                (*slang).sl_repsal_first[first_byte]
                            } else {
                                (*(*lp).lp_replang).sl_rep_first[first_byte]
                            };
                            if stack[udepth].ts_curi < 0 {
                                stack[udepth].ts_state = state_T::STATE_FINAL;
                                do_rep = false;
                            } else {
                                stack[udepth].ts_state = state_T::STATE_REP;
                            }
                        }
                    }
                    if do_rep {
                        let p = fword.add(stack[udepth].ts_fidx as usize);
                        let gap = if soundfold {
                            &raw mut (*slang).sl_repsal
                        } else {
                            &raw mut (*(*lp).lp_replang).sl_rep
                        };
                        while (stack[udepth].ts_curi as c_int) < (*gap).ga_len {
                            let curi = stack[udepth].ts_curi;
                            stack[udepth].ts_curi += 1;
                            let ftp = ((*gap).ga_data.cast::<fromto_T>()).add(curi as usize);
                            if *(*ftp).ft_from != *p {
                                stack[udepth].ts_curi = (*gap).ga_len as i16;
                                break;
                            }
                            if strncmp((*ftp).ft_from, p, strlen((*ftp).ft_from)) == 0
                                && try_deeper(su, &stack, udepth, SCORE_REP)
                            {
                                go_deeper(&mut stack, udepth, SCORE_REP);
                                stack[udepth].ts_state = state_T::STATE_REP_UNDO;
                                let old_fidx = stack[udepth].ts_fidx;
                                depth += 1;
                                udepth = depth as usize;
                                fl = strlen((*ftp).ft_from) as c_int;
                                tl = strlen((*ftp).ft_to) as c_int;
                                if fl != tl {
                                    strmove(p.add(tl as usize), p.add(fl as usize));
                                    repextra += tl - fl;
                                }
                                ptr::copy((*ftp).ft_to, p, tl as usize);
                                stack[udepth].ts_fidxtry = old_fidx + tl as u8;
                                stack[udepth].ts_tcharlen = 0;
                                break;
                            }
                        }
                        if (stack[udepth].ts_curi as c_int) >= (*gap).ga_len
                            && stack[udepth].ts_state == state_T::STATE_REP
                        {
                            stack[udepth].ts_state = state_T::STATE_FINAL;
                        }
                    }
                }
                state_T::STATE_REP_UNDO => {
                    let gap = if soundfold {
                        &raw mut (*slang).sl_repsal
                    } else {
                        &raw mut (*(*lp).lp_replang).sl_rep
                    };
                    let ftp = ((*gap).ga_data.cast::<fromto_T>())
                        .add((stack[udepth].ts_curi - 1) as usize);
                    fl = strlen((*ftp).ft_from) as c_int;
                    tl = strlen((*ftp).ft_to) as c_int;
                    let p = fword.add(stack[udepth].ts_fidx as usize);
                    if fl != tl {
                        strmove(p.add(fl as usize), p.add(tl as usize));
                        repextra -= tl - fl;
                    }
                    ptr::copy((*ftp).ft_from, p, fl as usize);
                    stack[udepth].ts_state = state_T::STATE_REP;
                }
                state_T::STATE_FINAL => {
                    depth -= 1;
                    if depth >= 0 && stack[depth as usize].ts_prefixdepth == PFD_PREFIXTREE {
                        byts = pbyts;
                        idxs = pidxs;
                    }
                    breakcheckcount -= 1;
                    if breakcheckcount == 0 {
                        os_breakcheck();
                        breakcheckcount = 1000;
                        if spell_suggest_timeout > 0 && profile_passed_limit(time_limit) {
                            got_int = true;
                        }
                    }
                }
            }
        }
    }
}
#[inline]
fn go_deeper(stack: &mut [trystate_T; MAXWLEN], depth: usize, score_add: c_int) {
    stack[depth + 1] = stack[depth];
    stack[depth + 1].ts_state = state_T::STATE_START;
    stack[depth + 1].ts_score = stack[depth].ts_score + score_add;
    stack[depth + 1].ts_curi = 1;
    stack[depth + 1].ts_flags = 0;
}
unsafe fn find_keepcap_word(slang: *mut slang_T, fword: *mut c_char, kword: *mut c_char) {
    unsafe {
        let mut uword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut arridx: [idx_T; MAXWLEN] = [0; MAXWLEN];
        let mut round: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut fwordidx: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut uwordidx: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut kwordlen: [c_int; MAXWLEN] = [0; MAXWLEN];
        let byts = (*slang).sl_kbyts;
        let idxs = (*slang).sl_kidxs;
        if byts.is_null() {
            *kword = NUL;
            return;
        }
        allcap_copy(fword, (&raw mut uword).cast());
        let mut depth: isize = 0;
        while depth >= 0 {
            let udepth = depth as usize;
            if *fword.add(fwordidx[udepth] as usize) == NUL {
                if *byts.add((arridx[udepth] + 1) as usize) == 0 {
                    *kword.add(kwordlen[udepth] as usize) = NUL;
                    return;
                }
                depth -= 1;
            } else {
                round[udepth] += 1;
                if round[udepth] > 2 {
                    depth -= 1;
                } else {
                    let flen = utf_ptr2len(fword.add(fwordidx[udepth] as usize));
                    let ulen = utf_ptr2len(
                        (&raw const uword)
                            .cast::<c_char>()
                            .add(uwordidx[udepth] as usize),
                    );
                    let (mut p, mut l): (*const c_char, c_int) = if round[udepth] == 1 {
                        (fword.add(fwordidx[udepth] as usize), flen)
                    } else {
                        (
                            (&raw const uword)
                                .cast::<c_char>()
                                .add(uwordidx[udepth] as usize),
                            ulen,
                        )
                    };
                    let mut tryidx = arridx[udepth];
                    while l > 0 {
                        let len = *byts.add(tryidx as usize) as c_int;
                        tryidx += 1;
                        let c = *p as u8 as c_int;
                        p = p.add(1);
                        let mut lo = tryidx;
                        let mut hi = tryidx + len - 1;
                        while lo < hi {
                            let m = (lo + hi) / 2;
                            if (*byts.add(m as usize) as c_int) > c {
                                hi = m - 1;
                            } else if (*byts.add(m as usize) as c_int) < c {
                                lo = m + 1;
                            } else {
                                lo = m;
                                hi = m;
                                break;
                            }
                        }
                        if hi < lo || *byts.add(lo as usize) as c_int != c {
                            break;
                        }
                        tryidx = *idxs.add(lo as usize);
                        l -= 1;
                    }
                    if l == 0 {
                        if round[udepth] == 1 {
                            strncpy(
                                kword.add(kwordlen[udepth] as usize),
                                fword.add(fwordidx[udepth] as usize),
                                flen as usize,
                            );
                            kwordlen[udepth + 1] = kwordlen[udepth] + flen;
                        } else {
                            strncpy(
                                kword.add(kwordlen[udepth] as usize),
                                (&raw const uword)
                                    .cast::<c_char>()
                                    .add(uwordidx[udepth] as usize),
                                ulen as usize,
                            );
                            kwordlen[udepth + 1] = kwordlen[udepth] + ulen;
                        }
                        fwordidx[udepth + 1] = fwordidx[udepth] + flen;
                        uwordidx[udepth + 1] = uwordidx[udepth] + ulen;
                        depth += 1;
                        arridx[depth as usize] = tryidx;
                        round[depth as usize] = 0;
                    }
                }
            }
        }
        *kword = NUL;
    }
}
unsafe fn score_comp_sal(su: *mut suginfo_T) {
    unsafe {
        let mut badsound: [c_char; MAXWLEN] = [0; MAXWLEN];
        ga_grow(&raw mut (*su).su_sga, (*su).su_ga.ga_len);
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            if (*(*lp).lp_slang).sl_sal.ga_len > 0 {
                spell_soundfold(
                    (*lp).lp_slang,
                    (&raw mut (*su).su_fbadword).cast(),
                    true,
                    (&raw mut badsound).cast(),
                );
                for i in 0..((*su).su_ga.ga_len as usize) {
                    let stp = sug_at(&raw const (*su).su_ga, i);
                    let score = stp_sal_score(stp, su, (*lp).lp_slang, (&raw mut badsound).cast());
                    if score < SCORE_MAXMAX {
                        let sstp = sug_at(&raw const (*su).su_sga, (*su).su_sga.ga_len as usize);
                        (*sstp).st_word = xstrdup((*stp).st_word);
                        (*sstp).st_wordlen = (*stp).st_wordlen;
                        (*sstp).st_score = score;
                        (*sstp).st_altscore = 0;
                        (*sstp).st_orglen = (*stp).st_orglen;
                        (*su).su_sga.ga_len += 1;
                    }
                }
                break;
            }
        }
    }
}
unsafe fn score_combine(su: *mut suginfo_T) {
    unsafe {
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        let mut badsound: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut slang: *mut slang_T = ptr::null_mut();
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            if (*(*lp).lp_slang).sl_sal.ga_len > 0 {
                slang = (*lp).lp_slang;
                spell_soundfold(
                    slang,
                    (&raw mut (*su).su_fbadword).cast(),
                    true,
                    (&raw mut badsound).cast(),
                );
                for i in 0..((*su).su_ga.ga_len as usize) {
                    let stp = sug_at(&raw const (*su).su_ga, i);
                    (*stp).st_altscore = stp_sal_score(stp, su, slang, (&raw mut badsound).cast());
                    if (*stp).st_altscore == SCORE_MAXMAX {
                        (*stp).st_score = ((*stp).st_score * 3 + SCORE_BIG) / 4;
                    } else {
                        (*stp).st_score = ((*stp).st_score * 3 + (*stp).st_altscore) / 4;
                    }
                    (*stp).st_salscore = false;
                }
                break;
            }
        }
        if slang.is_null() {
            cleanup_suggestions(&raw mut (*su).su_ga, (*su).su_maxscore, (*su).su_maxcount);
            return;
        }
        for i in 0..((*su).su_sga.ga_len as usize) {
            let stp = sug_at(&raw const (*su).su_sga, i);
            (*stp).st_altscore =
                spell_edit_score(slang, (&raw const (*su).su_badword).cast(), (*stp).st_word);
            if (*stp).st_score == SCORE_MAXMAX {
                (*stp).st_score = (SCORE_BIG * 7 + (*stp).st_altscore) / 8;
            } else {
                (*stp).st_score = ((*stp).st_score * 7 + (*stp).st_altscore) / 8;
            }
            (*stp).st_salscore = true;
        }
        check_suggestions(su, &raw mut (*su).su_ga);
        cleanup_suggestions(&raw mut (*su).su_ga, (*su).su_maxscore, (*su).su_maxcount);
        check_suggestions(su, &raw mut (*su).su_sga);
        cleanup_suggestions(&raw mut (*su).su_sga, (*su).su_maxscore, (*su).su_maxcount);
        ga_init(&raw mut ga, size_of::<suginfo_T>() as c_int, 1);
        ga_grow(&raw mut ga, (*su).su_ga.ga_len + (*su).su_sga.ga_len);
        let stp = sug_at(&raw const ga, 0);
        let mut i: c_int = 0;
        while i < (*su).su_ga.ga_len || i < (*su).su_sga.ga_len {
            for round in 1..=2 {
                let gap = if round == 1 {
                    &raw mut (*su).su_ga
                } else {
                    &raw mut (*su).su_sga
                };
                if i < (*gap).ga_len {
                    let p = (*sug_at(gap, i as usize)).st_word;
                    let mut j: c_int = 0;
                    while j < ga.ga_len {
                        if strcmp((*stp.add(j as usize)).st_word, p) == 0 {
                            break;
                        }
                        j += 1;
                    }
                    if j == ga.ga_len {
                        *stp.add(ga.ga_len as usize) = *sug_at(gap, i as usize);
                        ga.ga_len += 1;
                    } else {
                        xfree(p.cast());
                    }
                }
            }
            i += 1;
        }
        ga_clear(&raw mut (*su).su_ga);
        ga_clear(&raw mut (*su).su_sga);
        if ga.ga_len > (*su).su_maxcount {
            for idx in (*su).su_maxcount..ga.ga_len {
                xfree((*stp.add(idx as usize)).st_word.cast());
            }
            ga.ga_len = (*su).su_maxcount;
        }
        (*su).su_ga = ga;
    }
}
unsafe fn stp_sal_score(
    stp: *const suggest_T,
    su: *const suginfo_T,
    slang: *mut slang_T,
    badsound: *mut c_char,
) -> c_int {
    unsafe {
        let mut badsound2: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut fword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut goodsound: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut goodword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let lendiff = (*su).su_badlen - (*stp).st_orglen;
        let pbad = if lendiff >= 0 {
            badsound
        } else {
            spell_casefold(
                curwin,
                (*su).su_badptr,
                (*stp).st_orglen,
                (&raw mut fword).cast(),
                MAXWLEN as c_int,
            );
            if ascii_iswhite(*(*su).su_badptr.add((*su).su_badlen as usize))
                && *skiptowhite((*stp).st_word) == NUL
            {
                let mut p = (&raw mut fword).cast::<c_char>();
                loop {
                    p = skiptowhite(p);
                    if *p == NUL {
                        break;
                    }
                    strmove(p, p.add(1));
                }
            }
            spell_soundfold(
                slang,
                (&raw mut fword).cast(),
                true,
                (&raw mut badsound2).cast(),
            );
            (&raw mut badsound2).cast()
        };
        let pgood = if lendiff > 0 && ((*stp).st_wordlen + lendiff) < MAXWLEN as c_int {
            strcpy((&raw mut goodword).cast(), (*stp).st_word);
            xmemcpyz(
                (&raw mut goodword)
                    .cast::<c_char>()
                    .add((*stp).st_wordlen as usize)
                    .cast(),
                (*su)
                    .su_badptr
                    .add(((*su).su_badlen - lendiff) as usize)
                    .cast(),
                lendiff as usize,
            );
            (&raw mut goodword).cast()
        } else {
            (*stp).st_word
        };
        spell_soundfold(slang, pgood, false, (&raw mut goodsound).cast());
        soundalike_score((&raw mut goodsound).cast(), pbad)
    }
}
unsafe fn suggest_try_soundalike_prep() {
    unsafe {
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            let slang = (*lp).lp_slang;
            if (*slang).sl_sal.ga_len > 0 && !(*slang).sl_sbyts.is_null() {
                hash_init(&raw mut (*slang).sl_sounddone);
            }
        }
    }
}
unsafe fn suggest_try_soundalike(su: *mut suginfo_T) {
    unsafe {
        let mut salword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            let slang = (*lp).lp_slang;
            if (*slang).sl_sal.ga_len > 0 && !(*slang).sl_sbyts.is_null() {
                spell_soundfold(
                    slang,
                    (&raw mut (*su).su_fbadword).cast(),
                    true,
                    (&raw mut salword).cast(),
                );
                suggest_trie_walk(su, lp, (&raw mut salword).cast(), true);
            }
        }
    }
}
unsafe fn suggest_try_soundalike_finish() {
    unsafe {
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            let slang = (*lp).lp_slang;
            if (*slang).sl_sal.ga_len > 0 && !(*slang).sl_sbyts.is_null() {
                let mut todo = (*slang).sl_sounddone.ht_used as isize;
                let mut hi = (*slang).sl_sounddone.ht_array;
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        let sft = (*hi)
                            .hi_key
                            .sub(std::mem::offset_of!(sftword_T, sft_word))
                            .cast::<sftword_T>();
                        xfree(sft.cast());
                        todo -= 1;
                    }
                    hi = hi.add(1);
                }
                hash_clear(&raw mut (*slang).sl_sounddone);
                hash_init(&raw mut (*slang).sl_sounddone);
            }
        }
    }
}
unsafe fn add_sound_suggest(
    su: *mut suginfo_T,
    goodword: *mut c_char,
    score: c_int,
    lp: *mut langp_T,
) {
    unsafe {
        let slang = (*lp).lp_slang;
        let mut theword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let hash: hash_T = hash_hash(goodword);
        let goodword_len = strlen(goodword);
        let hi = hash_lookup(&raw mut (*slang).sl_sounddone, goodword, goodword_len, hash);
        if hashitem_empty(&*hi) {
            let sft_off = std::mem::offset_of!(sftword_T, sft_word);
            let sft = xmalloc(sft_off + goodword_len + 1).cast::<sftword_T>();
            (*sft).sft_score = score as i16;
            let sft_word_ptr = sft.cast::<u8>().add(sft_off).cast::<c_char>();
            ptr::copy_nonoverlapping(goodword, sft_word_ptr, goodword_len + 1);
            hash_add_item(&raw mut (*slang).sl_sounddone, hi, sft_word_ptr, hash);
        } else {
            let sft = (*hi)
                .hi_key
                .sub(std::mem::offset_of!(sftword_T, sft_word))
                .cast::<sftword_T>();
            if score >= (*sft).sft_score as c_int {
                return;
            }
            (*sft).sft_score = score as i16;
        }
        let sfwordnr = soundfold_find(slang, goodword);
        if sfwordnr < 0 {
            internal_error(c"add_sound_suggest()".as_ptr());
            return;
        }
        let mut nrline = ml_get_buf((*slang).sl_sugbuf, sfwordnr + 1);
        let mut orgnr: c_int = 0;
        while *nrline != NUL {
            orgnr += bytes2offset(&raw mut nrline);
            let byts = (*slang).sl_fbyts;
            let idxs = (*slang).sl_fidxs;
            let mut n: idx_T = 0;
            let mut wordcount: c_int = 0;
            let mut wlen: usize = 0;
            let mut i: c_int = 1;
            'badword: {
                while wlen < MAXWLEN - 3 {
                    i = 1;
                    if wordcount == orgnr && *byts.add((n + 1) as usize) == 0 {
                        break;
                    }
                    if *byts.add((n + 1) as usize) == 0 {
                        wordcount += 1;
                    }
                    while *byts.add((n + i) as usize) == 0 {
                        if i > *byts.add(n as usize) as c_int {
                            strcpy(
                                (&raw mut theword).cast::<c_char>().add(wlen),
                                c"BAD".as_ptr(),
                            );
                            wlen += 3;
                            break 'badword;
                        }
                        i += 1;
                    }
                    while i < *byts.add(n as usize) as c_int {
                        let wc = *idxs.add(*idxs.add((n + i) as usize) as usize);
                        if wordcount + wc > orgnr {
                            break;
                        }
                        wordcount += wc;
                        i += 1;
                    }
                    theword[wlen] = *byts.add((n + i) as usize) as c_char;
                    n = *idxs.add((n + i) as usize);
                    wlen += 1;
                }
            }
            theword[wlen] = NUL;
            while i <= *byts.add(n as usize) as c_int && *byts.add((n + i) as usize) == 0 {
                let mut cword: [c_char; MAXWLEN] = [0; MAXWLEN];
                let mut flags = *idxs.add((n + i) as usize);
                if (flags & WF_NOSUGGEST) != 0 {
                    i += 1;
                    continue;
                }
                let p: *mut c_char = if (flags & WF_KEEPCAP) != 0 {
                    find_keepcap_word(slang, (&raw mut theword).cast(), (&raw mut cword).cast());
                    (&raw mut cword).cast()
                } else {
                    flags |= (*su).su_badflags;
                    if (flags & WF_CAPMASK) != 0 {
                        make_case_word((&raw mut theword).cast(), (&raw mut cword).cast(), flags);
                        (&raw mut cword).cast()
                    } else {
                        (&raw mut theword).cast()
                    }
                };
                if (sps_flags & SPS_DOUBLE) != 0 {
                    if score <= (*su).su_maxscore {
                        add_suggestion(
                            su,
                            &raw mut (*su).su_sga,
                            p,
                            (*su).su_badlen,
                            score,
                            0,
                            false,
                            slang,
                            false,
                        );
                    }
                } else {
                    let mut goodscore = if (flags & WF_REGION) != 0
                        && (((flags as u32) >> 16) & ((*lp).lp_region as u32)) == 0
                    {
                        SCORE_REGION
                    } else {
                        0
                    };
                    let gc = utf_ptr2char(p);
                    if spell_isupper(gc) {
                        let bc = utf_ptr2char((&raw const (*su).su_badword).cast());
                        if !spell_isupper(bc) && spell_tofold(bc) != spell_tofold(gc) {
                            goodscore += SCORE_ICASE / 2;
                        }
                    }
                    let limit = maxscore((*su).su_sfmaxscore - goodscore, score);
                    if limit > SCORE_LIMITMAX {
                        goodscore +=
                            spell_edit_score(slang, (&raw const (*su).su_badword).cast(), p);
                    } else {
                        goodscore += spell_edit_score_limit(
                            slang,
                            (&raw mut (*su).su_badword).cast(),
                            p,
                            limit,
                        );
                    }
                    if goodscore < SCORE_MAXMAX {
                        goodscore = score_wordcount_adj(slang, goodscore, p, false);
                        goodscore = rescore(goodscore, score);
                        if goodscore <= (*su).su_sfmaxscore {
                            add_suggestion(
                                su,
                                &raw mut (*su).su_ga,
                                p,
                                (*su).su_badlen,
                                goodscore,
                                score,
                                true,
                                slang,
                                true,
                            );
                        }
                    }
                }
                i += 1;
            }
        }
    }
}
unsafe fn soundfold_find(slang: *mut slang_T, word: *mut c_char) -> c_int {
    unsafe {
        let mut arridx: idx_T = 0;
        let mut wlen: usize = 0;
        let ptr = word.cast::<u8>();
        let mut wordnr: c_int = 0;
        let byts = (*slang).sl_sbyts;
        let idxs = (*slang).sl_sidxs;
        loop {
            let mut len = *byts.add(arridx as usize) as c_int;
            arridx += 1;
            let mut c = *ptr.add(wlen);
            if *byts.add(arridx as usize) == 0 {
                if c == 0 {
                    break;
                }
                while len > 0 && *byts.add(arridx as usize) == 0 {
                    arridx += 1;
                    len -= 1;
                }
                if len == 0 {
                    return -1;
                }
                wordnr += 1;
            }
            if c == 0 {
                return -1;
            }
            if c == TAB {
                c = b' ';
            }
            while *byts.add(arridx as usize) < c {
                wordnr += *idxs.add(*idxs.add(arridx as usize) as usize);
                arridx += 1;
                len -= 1;
                if len == 0 {
                    return -1;
                }
            }
            if *byts.add(arridx as usize) != c {
                return -1;
            }
            arridx = *idxs.add(arridx as usize);
            wlen += 1;
            if c == b' ' {
                while *ptr.add(wlen) == b' ' || *ptr.add(wlen) == TAB {
                    wlen += 1;
                }
            }
        }
        wordnr
    }
}
unsafe fn similar_chars(slang: *mut slang_T, c1: c_int, c2: c_int) -> bool {
    unsafe {
        let mut buf: [c_char; MB_MAXCHAR + 1] = [0; MB_MAXCHAR + 1];
        let m1 = if c1 >= 256 {
            let blen = utf_char2bytes(c1, (&raw mut buf).cast()) as usize;
            buf[blen] = 0;
            let hi = hash_find(&raw mut (*slang).sl_map_hash, (&raw const buf).cast());
            if hashitem_empty(&*hi) {
                0
            } else {
                utf_ptr2char((*hi).hi_key.add(strlen((*hi).hi_key) + 1))
            }
        } else if c1 >= 0 {
            (*slang).sl_map_array[c1 as usize]
        } else {
            0
        };
        if m1 == 0 {
            return false;
        }
        let m2 = if c2 >= 256 {
            let blen = utf_char2bytes(c2, (&raw mut buf).cast()) as usize;
            buf[blen] = 0;
            let hi = hash_find(&raw mut (*slang).sl_map_hash, (&raw const buf).cast());
            if hashitem_empty(&*hi) {
                0
            } else {
                utf_ptr2char((*hi).hi_key.add(strlen((*hi).hi_key) + 1))
            }
        } else if c2 >= 0 {
            (*slang).sl_map_array[c2 as usize]
        } else {
            0
        };
        m1 == m2
    }
}
unsafe fn add_suggestion(
    su: *mut suginfo_T,
    gap: *mut garray_T,
    goodword: *const c_char,
    badlenarg: c_int,
    score: c_int,
    altscore: c_int,
    had_bonus: bool,
    slang: *mut slang_T,
    maxsf: bool,
) {
    unsafe {
        let mut goodlen: c_int;
        let mut badlen: c_int;
        let mut pgood = goodword.add(strlen(goodword));
        let mut pbad: *mut c_char = (*su).su_badptr.add(badlenarg as usize);
        loop {
            goodlen = pgood.offset_from(goodword) as c_int;
            badlen = pbad.offset_from((*su).su_badptr) as c_int;
            if goodlen <= 0 || badlen <= 0 {
                break;
            }
            mb_cptr_back(goodword, &mut pgood);
            mb_ptr_back((*su).su_badptr, &mut pbad);
            if utf_ptr2char(pgood) != utf_ptr2char(pbad) {
                break;
            }
        }
        if badlen == 0 && goodlen == 0 {
            return;
        }
        let mut i: c_int;
        if (*gap).ga_len <= 0 {
            i = -1;
        } else {
            let mut stp = sug_at(gap, 0);
            i = (*gap).ga_len - 1;
            while i >= 0 {
                if (*stp).st_wordlen == goodlen
                    && (*stp).st_orglen == badlen
                    && strncmp((*stp).st_word, goodword, goodlen as usize) == 0
                {
                    if (*stp).st_slang.is_null() {
                        (*stp).st_slang = slang;
                    }
                    let mut new_sug = suggest_T {
                        st_word: ptr::null_mut(),
                        st_wordlen: 0,
                        st_orglen: 0,
                        st_score: score,
                        st_altscore: altscore,
                        st_salscore: false,
                        st_had_bonus: had_bonus,
                        st_slang: ptr::null_mut(),
                    };
                    if (*stp).st_had_bonus != had_bonus {
                        if had_bonus {
                            rescore_one(su, stp);
                        } else {
                            new_sug.st_word = (*stp).st_word;
                            new_sug.st_wordlen = (*stp).st_wordlen;
                            new_sug.st_slang = (*stp).st_slang;
                            new_sug.st_orglen = badlen;
                            rescore_one(su, &raw mut new_sug);
                        }
                    }
                    if (*stp).st_score > new_sug.st_score {
                        (*stp).st_score = new_sug.st_score;
                        (*stp).st_altscore = new_sug.st_altscore;
                        (*stp).st_had_bonus = new_sug.st_had_bonus;
                    }
                    break;
                }
                stp = stp.add(1);
                i -= 1;
            }
        }
        if i < 0 {
            ga_grow(gap, 1);
            let stp = sug_at(gap, (*gap).ga_len as usize);
            (*gap).ga_len += 1;
            (*stp).st_word = xmemdupz(goodword.cast(), goodlen as usize).cast();
            (*stp).st_wordlen = goodlen;
            (*stp).st_score = score;
            (*stp).st_altscore = altscore;
            (*stp).st_salscore = false;
            (*stp).st_had_bonus = had_bonus;
            (*stp).st_orglen = badlen;
            (*stp).st_slang = slang;
            if (*gap).ga_len > sug_max_count(su) {
                if maxsf {
                    (*su).su_sfmaxscore =
                        cleanup_suggestions(gap, (*su).su_sfmaxscore, sug_clean_count(su));
                } else {
                    (*su).su_maxscore =
                        cleanup_suggestions(gap, (*su).su_maxscore, sug_clean_count(su));
                }
            }
        }
    }
}
unsafe fn check_suggestions(su: *mut suginfo_T, gap: *mut garray_T) {
    unsafe {
        let mut longword: [c_char; MAXWLEN + 1] = [0; MAXWLEN + 1];
        if (*gap).ga_len == 0 {
            return;
        }
        let stp = sug_at(gap, 0);
        let mut i = (*gap).ga_len - 1;
        while i >= 0 {
            let ui = i as usize;
            xstrlcpy(
                (&raw mut longword).cast(),
                (*stp.add(ui)).st_word,
                MAXWLEN + 1,
            );
            let len = (*stp.add(ui)).st_wordlen as usize;
            xstrlcpy(
                (&raw mut longword).cast::<c_char>().add(len),
                (*su).su_badptr.add((*stp.add(ui)).st_orglen as usize),
                MAXWLEN + 1 - len,
            );
            let mut attr: hlf_T = HLF_COUNT;
            let _ = spell_check(
                curwin,
                (&raw mut longword).cast(),
                &raw mut attr,
                ptr::null_mut(),
                false,
            );
            if attr != HLF_COUNT {
                xfree((*stp.add(ui)).st_word.cast());
                (*gap).ga_len -= 1;
                if i < (*gap).ga_len {
                    ptr::copy(stp.add(ui + 1), stp.add(ui), ((*gap).ga_len - i) as usize);
                }
            }
            i -= 1;
        }
    }
}
unsafe fn add_banned(su: *mut suginfo_T, word: *mut c_char) {
    unsafe {
        let hash: hash_T = hash_hash(word);
        let word_len = strlen(word);
        let hi = hash_lookup(&raw mut (*su).su_banned, word, word_len, hash);
        if !hashitem_empty(&*hi) {
            return;
        }
        let s = xmemdupz(word.cast(), word_len).cast::<c_char>();
        hash_add_item(&raw mut (*su).su_banned, hi, s, hash);
    }
}
unsafe fn rescore_suggestions(su: *mut suginfo_T) {
    unsafe {
        if !(*su).su_sallang.is_null() {
            for i in 0..((*su).su_ga.ga_len as usize) {
                rescore_one(su, sug_at(&raw const (*su).su_ga, i));
            }
        }
    }
}
unsafe fn rescore_one(su: *mut suginfo_T, stp: *mut suggest_T) {
    unsafe {
        let slang = (*stp).st_slang;
        let mut sal_badword: [c_char; MAXWLEN] = [0; MAXWLEN];
        if !slang.is_null() && (*slang).sl_sal.ga_len > 0 && !(*stp).st_had_bonus {
            let p: *mut c_char = if slang == (*su).su_sallang {
                (&raw mut (*su).su_sal_badword).cast()
            } else {
                spell_soundfold(
                    slang,
                    (&raw mut (*su).su_fbadword).cast(),
                    true,
                    (&raw mut sal_badword).cast(),
                );
                (&raw mut sal_badword).cast()
            };
            (*stp).st_altscore = stp_sal_score(stp, su, slang, p);
            if (*stp).st_altscore == SCORE_MAXMAX {
                (*stp).st_altscore = SCORE_BIG;
            }
            (*stp).st_score = rescore((*stp).st_score, (*stp).st_altscore);
            (*stp).st_had_bonus = true;
        }
    }
}
unsafe extern "C" fn sug_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let p1 = s1.cast::<suggest_T>();
        let p2 = s2.cast::<suggest_T>();
        let mut n = if (*p1).st_score == (*p2).st_score {
            0
        } else if (*p1).st_score > (*p2).st_score {
            1
        } else {
            -1
        };
        if n == 0 {
            n = if (*p1).st_altscore == (*p2).st_altscore {
                0
            } else if (*p1).st_altscore > (*p2).st_altscore {
                1
            } else {
                -1
            };
            if n == 0 {
                n = mb_stricmp((*p1).st_word, (*p2).st_word);
            }
        }
        n
    }
}
unsafe fn cleanup_suggestions(gap: *mut garray_T, maxscore: c_int, keep: c_int) -> c_int {
    unsafe {
        if (*gap).ga_len <= 0 {
            return maxscore;
        }
        qsort(
            (*gap).ga_data,
            (*gap).ga_len as usize,
            size_of::<suggest_T>(),
            sug_compare,
        );
        if (*gap).ga_len > keep {
            let stp = sug_at(gap, 0);
            for i in keep..(*gap).ga_len {
                xfree((*stp.add(i as usize)).st_word.cast());
            }
            (*gap).ga_len = keep;
            if keep >= 1 {
                return (*stp.add((keep - 1) as usize)).st_score;
            }
        }
        maxscore
    }
}
unsafe fn soundalike_score(goodstart: *mut c_char, badstart: *mut c_char) -> c_int {
    unsafe {
        let mut goodsound = goodstart;
        let mut badsound = badstart;
        let mut score: c_int = 0;
        if (*badsound == b'*' as c_char || *goodsound == b'*' as c_char) && *badsound != *goodsound
        {
            if (*badsound == NUL && *goodsound.add(1) == NUL)
                || (*goodsound == NUL && *badsound.add(1) == NUL)
            {
                return SCORE_DEL;
            }
            if *badsound == NUL || *goodsound == NUL {
                return SCORE_MAXMAX;
            }
            if *badsound.add(1) == *goodsound.add(1)
                || (*badsound.add(1) != NUL
                    && *goodsound.add(1) != NUL
                    && *badsound.add(2) == *goodsound.add(2))
            {
                // handle like a substitute
            } else {
                score = 2 * SCORE_DEL / 3;
                if *badsound == b'*' as c_char {
                    badsound = badsound.add(1);
                } else {
                    goodsound = goodsound.add(1);
                }
            }
        }
        let goodlen = strlen(goodsound) as c_int;
        let badlen = strlen(badsound) as c_int;
        let n = goodlen - badlen;
        if !(-2..=2).contains(&n) {
            return SCORE_MAXMAX;
        }
        let mut pl = if n > 0 { goodsound } else { badsound };
        let mut ps = if n > 0 { badsound } else { goodsound };
        while *pl == *ps && *pl != NUL {
            pl = pl.add(1);
            ps = ps.add(1);
        }
        match n {
            -2 | 2 => {
                pl = pl.add(1);
                while *pl == *ps {
                    pl = pl.add(1);
                    ps = ps.add(1);
                }
                if strcmp(pl.add(1), ps) == 0 {
                    return score + SCORE_DEL * 2;
                }
            }
            -1 | 1 => {
                let mut pl2 = pl.add(1);
                let mut ps2 = ps;
                while *pl2 == *ps2 {
                    if *pl2 == NUL {
                        return score + SCORE_DEL;
                    }
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if *pl2 == *ps2.add(1) && *pl2.add(1) == *ps2 && strcmp(pl2.add(2), ps2.add(2)) == 0
                {
                    return score + SCORE_DEL + SCORE_SWAP;
                }
                if strcmp(pl2.add(1), ps2.add(1)) == 0 {
                    return score + SCORE_DEL + SCORE_SUBST;
                }
                if *pl == *ps.add(1) && *pl.add(1) == *ps {
                    pl2 = pl.add(2);
                    ps2 = ps.add(2);
                    while *pl2 == *ps2 {
                        pl2 = pl2.add(1);
                        ps2 = ps2.add(1);
                    }
                    if strcmp(pl2.add(1), ps2) == 0 {
                        return score + SCORE_SWAP + SCORE_DEL;
                    }
                }
                pl2 = pl.add(1);
                ps2 = ps.add(1);
                while *pl2 == *ps2 {
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if strcmp(pl2.add(1), ps2) == 0 {
                    return score + SCORE_SUBST + SCORE_DEL;
                }
            }
            0 => {
                if *pl == NUL {
                    return score;
                }
                if *pl == *ps.add(1) && *pl.add(1) == *ps {
                    let mut pl2 = pl.add(2);
                    let mut ps2 = ps.add(2);
                    while *pl2 == *ps2 {
                        if *pl2 == NUL {
                            return score + SCORE_SWAP;
                        }
                        pl2 = pl2.add(1);
                        ps2 = ps2.add(1);
                    }
                    if *pl2 == *ps2.add(1)
                        && *pl2.add(1) == *ps2
                        && strcmp(pl2.add(2), ps2.add(2)) == 0
                    {
                        return score + SCORE_SWAP + SCORE_SWAP;
                    }
                    if strcmp(pl2.add(1), ps2.add(1)) == 0 {
                        return score + SCORE_SWAP + SCORE_SUBST;
                    }
                }
                let mut pl2 = pl.add(1);
                let mut ps2 = ps.add(1);
                while *pl2 == *ps2 {
                    if *pl2 == NUL {
                        return score + SCORE_SUBST;
                    }
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if *pl2 == *ps2.add(1) && *pl2.add(1) == *ps2 && strcmp(pl2.add(2), ps2.add(2)) == 0
                {
                    return score + SCORE_SUBST + SCORE_SWAP;
                }
                if strcmp(pl2.add(1), ps2.add(1)) == 0 {
                    return score + SCORE_SUBST + SCORE_SUBST;
                }
                pl2 = pl;
                ps2 = ps.add(1);
                while *pl2 == *ps2 {
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if strcmp(pl2.add(1), ps2) == 0 {
                    return score + SCORE_INS + SCORE_DEL;
                }
                pl2 = pl.add(1);
                ps2 = ps;
                while *pl2 == *ps2 {
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if strcmp(pl2, ps2.add(1)) == 0 {
                    return score + SCORE_INS + SCORE_DEL;
                }
            }
            _ => {}
        }
        SCORE_MAXMAX
    }
}
unsafe fn spell_edit_score(
    slang: *mut slang_T,
    badword: *const c_char,
    goodword: *const c_char,
) -> c_int {
    unsafe {
        let mut wbadword: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut wgoodword: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut badlen: usize = 0;
        let mut p = badword;
        while *p != NUL {
            wbadword[badlen] = mb_cptr2char_adv(&raw mut p);
            badlen += 1;
        }
        wbadword[badlen] = 0;
        badlen += 1;
        let mut goodlen: usize = 0;
        p = goodword;
        while *p != NUL {
            wgoodword[goodlen] = mb_cptr2char_adv(&raw mut p);
            goodlen += 1;
        }
        wgoodword[goodlen] = 0;
        goodlen += 1;
        let stride = badlen + 1;
        let cnt = xmalloc(size_of::<c_int>() * stride * (goodlen + 1)).cast::<c_int>();
        *cnt = 0;
        for j in 1..=goodlen {
            *cnt.add(j * stride) = *cnt.add((j - 1) * stride) + SCORE_INS;
        }
        for i in 1..=badlen {
            *cnt.add(i) = *cnt.add(i - 1) + SCORE_DEL;
            for j in 1..=goodlen {
                let bc = wbadword[i - 1];
                let gc = wgoodword[j - 1];
                let idx_ij = i + j * stride;
                if bc == gc {
                    *cnt.add(idx_ij) = *cnt.add((i - 1) + (j - 1) * stride);
                } else {
                    let prev_diag = *cnt.add((i - 1) + (j - 1) * stride);
                    let mut val = if spell_tofold(bc) == spell_tofold(gc) {
                        SCORE_ICASE + prev_diag
                    } else if !slang.is_null()
                        && (*slang).sl_has_map
                        && similar_chars(slang, gc, bc)
                    {
                        SCORE_SIMILAR + prev_diag
                    } else {
                        SCORE_SUBST + prev_diag
                    };
                    if i > 1 && j > 1 {
                        let pbc = wbadword[i - 2];
                        let pgc = wgoodword[j - 2];
                        if bc == pgc && pbc == gc {
                            let t = SCORE_SWAP + *cnt.add((i - 2) + (j - 2) * stride);
                            val = val.min(t);
                        }
                    }
                    let t_del = SCORE_DEL + *cnt.add((i - 1) + j * stride);
                    val = val.min(t_del);
                    let t_ins = SCORE_INS + *cnt.add(i + (j - 1) * stride);
                    val = val.min(t_ins);
                    *cnt.add(idx_ij) = val;
                }
            }
        }
        let res = *cnt.add((badlen - 1) + (goodlen - 1) * stride);
        xfree(cnt.cast());
        res
    }
}
unsafe fn spell_edit_score_limit(
    slang: *mut slang_T,
    badword: *mut c_char,
    goodword: *mut c_char,
    limit: c_int,
) -> c_int {
    unsafe { spell_edit_score_limit_w(slang, badword, goodword, limit) }
}
unsafe fn spell_edit_score_limit_w(
    slang: *mut slang_T,
    badword: *const c_char,
    goodword: *const c_char,
    limit: c_int,
) -> c_int {
    unsafe {
        let mut stack: [limitscore_T; 10] = [limitscore_T::default(); 10];
        let mut wbadword: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut wgoodword: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut bi: usize = 0;
        let mut p = badword;
        while *p != NUL {
            wbadword[bi] = mb_cptr2char_adv(&raw mut p);
            bi += 1;
        }
        wbadword[bi] = 0;
        let mut gi: usize = 0;
        p = goodword;
        while *p != NUL {
            wgoodword[gi] = mb_cptr2char_adv(&raw mut p);
            gi += 1;
        }
        wgoodword[gi] = 0;
        let mut stackidx: usize = 0;
        bi = 0;
        gi = 0;
        let mut score: c_int = 0;
        let mut minscore = limit + 1;
        loop {
            let mut bc: c_int;
            let mut gc: c_int;
            let mut do_pop = false;
            loop {
                bc = wbadword[bi];
                gc = wgoodword[gi];
                if bc != gc {
                    break;
                }
                if bc == 0 {
                    if score < minscore {
                        minscore = score;
                    }
                    do_pop = true;
                    break;
                }
                bi += 1;
                gi += 1;
            }
            if !do_pop {
                if gc == 0 {
                    loop {
                        score += SCORE_DEL;
                        if score >= minscore {
                            do_pop = true;
                            break;
                        }
                        bi += 1;
                        if wbadword[bi] == 0 {
                            break;
                        }
                    }
                    if !do_pop {
                        minscore = score;
                    }
                } else if bc == 0 {
                    loop {
                        score += SCORE_INS;
                        if score >= minscore {
                            do_pop = true;
                            break;
                        }
                        gi += 1;
                        if wgoodword[gi] == 0 {
                            break;
                        }
                    }
                    if !do_pop {
                        minscore = score;
                    }
                } else {
                    for round in 0..=1usize {
                        let score_off = score + if round == 0 { SCORE_DEL } else { SCORE_INS };
                        if score_off < minscore {
                            if score_off + SCORE_EDIT_MIN >= minscore {
                                let mut bi2 = bi + 1 - round;
                                let mut gi2 = gi + round;
                                while wgoodword[gi2] == wbadword[bi2] {
                                    if wgoodword[gi2] == 0 {
                                        minscore = score_off;
                                        break;
                                    }
                                    bi2 += 1;
                                    gi2 += 1;
                                }
                            } else {
                                stack[stackidx].badi = (bi + 1 - round) as c_int;
                                stack[stackidx].goodi = (gi + round) as c_int;
                                stack[stackidx].score = score_off;
                                stackidx += 1;
                            }
                        }
                    }
                    if score + SCORE_SWAP < minscore
                        && gc == wbadword[bi + 1]
                        && bc == wgoodword[gi + 1]
                    {
                        gi += 2;
                        bi += 2;
                        score += SCORE_SWAP;
                        continue;
                    }
                    if spell_tofold(bc) == spell_tofold(gc) {
                        score += SCORE_ICASE;
                    } else if !slang.is_null()
                        && (*slang).sl_has_map
                        && similar_chars(slang, gc, bc)
                    {
                        score += SCORE_SIMILAR;
                    } else {
                        score += SCORE_SUBST;
                    }
                    if score < minscore {
                        gi += 1;
                        bi += 1;
                        continue;
                    }
                }
            }
            if stackidx == 0 {
                break;
            }
            stackidx -= 1;
            gi = stack[stackidx].goodi as usize;
            bi = stack[stackidx].badi as usize;
            score = stack[stackidx].score;
        }
        if minscore > limit {
            return SCORE_MAXMAX;
        }
        minscore
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bytes2offset_and_soundalike_score_work() {
        unsafe {
            let mut raw = [5u8, 0u8];
            let mut p = raw.as_mut_ptr().cast::<c_char>();
            assert_eq!(bytes2offset(&raw mut p), 4);
            let mut s1 = *b"tht\0";
            let mut s2 = *b"tht\0";
            assert_eq!(
                soundalike_score(s1.as_mut_ptr().cast(), s2.as_mut_ptr().cast()),
                0
            );
            let mut s3 = *b"ht\0";
            assert_eq!(
                soundalike_score(s1.as_mut_ptr().cast(), s3.as_mut_ptr().cast()),
                SCORE_DEL
            );
        }
    }
}
