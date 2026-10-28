//! Functions for spelling suggestions (`src/nvim/spellsuggest.c`).
use crate::buffer::buf_T;
use crate::change::inserted_bytes;
use crate::charset::{NUL, ascii_isdigit, ascii_iswhite, getdigits_int, skiptowhite, skipwhite};
use crate::cursor::{check_cursor, colnr_T, get_cursor_line_len, get_cursor_line_ptr};
use crate::errors::*;
use crate::eval::typval::{
    TypvalVval, VarType, tv_clear, tv_dict_add_bool, tv_dict_add_nr, tv_dict_add_str,
    tv_dict_add_str_len, tv_dict_alloc, tv_list_alloc_ret, tv_list_append_tv, tv_list_unref,
    tv_set_string, typval_T,
};
use crate::eval::vars::{eval_spell_expr, get_spellword};
use crate::fileio::{fclose, vim_fgets};
use crate::globals::*;
use crate::input::{beep_flush, prep_redo, redo_append_char, redo_append_lit, vgetc};
use crate::input_cmdatom::CmdSpec;
use crate::lua::executor::nlua_call_typval;
use crate::mbyte::{
    mb_charlen, mb_cptr2char_adv, mb_ptr_adv, mb_ptr_back, mb_stricmp, utf_byte2len,
    utf_char2bytes, utf_char2len, utf_head_off, utf_iscomposing_legacy, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len,
};
use crate::memline::{ml_get_buf, ml_replace};
use crate::memory::{xstrlcpy};
use crate::message::{emsg, internal_error, msg, msg_ext_set_kind, semsg, smsg};
use crate::normal::end_visual_mode;
use crate::option::{copy_option_part, option_next_keyval, option_slice_eq};
use crate::option_vars::*;
use crate::os::fs::{MAXPATHL, os_fopen};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::profile::{profile_passed_limit, profile_setlimit, proftime_T};
use crate::spell::{
    MAXWLEN, WF_ALLCAP, WF_BANNED, WF_CAPMASK, WF_KEEPCAP, WF_NEEDCOMP, WF_ONECAP, WF_RARE,
    WF_RAREPFX, WF_REGION, allcap_copy, byte_in_str, can_compound, captype, check_need_cap,
    make_case_word, match_checkcompoundpattern, match_compoundrule, nofold_len, parse_spelllang,
    spell_priv_st, spell_casefold, spell_check, spell_isupper, spell_iswordp,
    spell_iswordp_nmw, spell_move_to, spell_soundfold, spell_tofold, spell_valid_case,
    valid_word_prefix,
};
use crate::spellfile::{MAXREGIONS, WF_NOSUGGEST, suggest_load_files};
use crate::strings::{
    gettext, vim_snprintf,
    vim_strchr,
};
use crate::ui::vim_beep;
use crate::undo::u_save_cursor;
use crate::viml::parser::expressions::TAB;
use std::ffi::{CString, c_char, c_int, c_uint};
use std::ptr;
pub type idx_T = c_int;
pub type salfirst_T = c_int;
use crate::digraph::{kBoolVarFalse, kBoolVarTrue};
use crate::drawline::{HLF_COUNT, SMT_ALL};
use crate::ex_getln::hlf_T;
use crate::globals::IOSIZE;
use crate::keycodes::ESC;
use crate::search::FORWARD;
const WF_MIXCAP: c_int = 0x20;
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
const kOptBoFlagSpell: c_uint = 0x20000;
pub struct fromto_T {
    pub ft_from: std::ffi::CString,
    pub ft_to: std::ffi::CString,
}
#[repr(C)]
pub struct salitem_T {
    pub sm_leadlen: c_int,
    pub sm_rules: std::ffi::CString,
    pub sm_lead_w: Vec<c_int>,
    pub sm_oneof_w: Vec<c_int>,
    pub sm_to_w: Vec<c_int>,
}

impl Drop for slang_T {
    fn drop(&mut self) {
        crate::spell::slang_clear(self);
    }
}
#[repr(C)]
pub struct slang_T {
    pub sl_name: Option<std::ffi::CString>,
    pub sl_fname: Option<std::ffi::CString>,
    pub sl_add: bool,
    pub sl_fbyts: Vec<u8>,
    pub sl_fidxs: Vec<idx_T>,
    pub sl_kbyts: Vec<u8>,
    pub sl_kidxs: Vec<idx_T>,
    pub sl_pbyts: Vec<u8>,
    pub sl_pidxs: Vec<idx_T>,
    pub sl_info: Option<std::ffi::CString>,
    pub sl_regions: [c_char; MAXREGIONS * 2 + 1],
    pub sl_midword: Option<std::ffi::CString>,
    pub sl_wordcount: std::collections::BTreeMap<Vec<u8>, u16>,
    pub sl_compmax: c_int,
    pub sl_compminlen: c_int,
    pub sl_compsylmax: c_int,
    pub sl_compoptions: c_int,
    pub sl_comppat: Vec<std::ffi::CString>,
    pub sl_compprog: Option<Box<crate::regexp::regprog_T>>,
    pub sl_comprules: Option<std::ffi::CString>,
    pub sl_compstartflags: Option<std::ffi::CString>,
    pub sl_compallflags: Option<std::ffi::CString>,
    pub sl_nobreak: bool,
    pub sl_syllable: Option<std::ffi::CString>,
    pub sl_syl_items: Vec<crate::spell::syl_item_T>,
    pub sl_prefprog: Vec<Option<Box<crate::regexp::regprog_T>>>,
    pub sl_rep: Vec<fromto_T>,
    pub sl_rep_first: [i16; 256],
    pub sl_sal: Vec<salitem_T>,
    pub sl_sofo_sal: Vec<Vec<(c_int, c_int)>>,
    pub sl_sal_first: [salfirst_T; 256],
    pub sl_followup: bool,
    pub sl_collapse: bool,
    pub sl_rem_accents: bool,
    pub sl_sofo: bool,
    pub sl_repsal: Vec<fromto_T>,
    pub sl_repsal_first: [i16; 256],
    pub sl_nosplitsugs: bool,
    pub sl_nocompoundsugs: bool,
    pub sl_sugtime: i64,
    pub sl_sbyts: Vec<u8>,
    pub sl_sidxs: Vec<idx_T>,
    pub sl_sugbuf: Option<Box<buf_T>>,
    pub sl_sugloaded: bool,
    pub sl_has_map: bool,
    pub sl_map_hash: std::collections::BTreeMap<c_int, c_int>,
    pub sl_map_array: [c_int; 256],
    pub sl_sounddone: std::collections::BTreeMap<Vec<u8>, i16>,
}

impl slang_T {
    #[inline]
    pub fn has_sal(&self) -> bool {
        !self.sl_sal.is_empty() || !self.sl_sofo_sal.is_empty()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct langp_T {
    pub lp_slang: *mut slang_T,
    pub lp_sallang: *mut slang_T,
    pub lp_replang: *mut slang_T,
    pub lp_region: c_int,
}

impl langp_T {
    pub const fn new() -> Self {
        Self {
            lp_slang: ptr::null_mut(),
            lp_sallang: ptr::null_mut(),
            lp_replang: ptr::null_mut(),
            lp_region: 0,
        }
    }
}

impl Default for langp_T {
    fn default() -> Self {
        Self::new()
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct spelltab_T {
    pub st_isw: [bool; 256],
    pub st_isu: [bool; 256],
    pub st_fold: [u8; 256],
    pub st_upper: [u8; 256],
}

impl spelltab_T {
    pub const fn new() -> Self {
        Self {
            st_isw: [false; 256],
            st_isu: [false; 256],
            st_fold: [0; 256],
            st_upper: [0; 256],
        }
    }
}

impl Default for spelltab_T {
    fn default() -> Self {
        Self::new()
    }
}
#[repr(C)]
struct suginfo_T {
    su_ga: Vec<suggest_T>,
    su_maxcount: c_int,
    su_maxscore: c_int,
    su_sfmaxscore: c_int,
    su_sga: Vec<suggest_T>,
    su_badptr: *mut c_char,
    su_badlen: c_int,
    su_badflags: c_int,
    su_badword: [c_char; MAXWLEN],
    su_fbadword: [c_char; MAXWLEN],
    su_sal_badword: [c_char; MAXWLEN],
    su_banned: std::collections::HashSet<Vec<u8>>,
    su_sallang: *mut slang_T,
}

impl Default for suginfo_T {
    fn default() -> Self {
        Self {
            su_ga: Vec::new(),
            su_maxcount: 0,
            su_maxscore: 0,
            su_sfmaxscore: 0,
            su_sga: Vec::new(),
            su_badptr: ptr::null_mut(),
            su_badlen: 0,
            su_badflags: 0,
            su_badword: [0; MAXWLEN],
            su_fbadword: [0; MAXWLEN],
            su_sal_badword: [0; MAXWLEN],
            su_banned: std::collections::HashSet::new(),
            su_sallang: ptr::null_mut(),
        }
    }
}
#[repr(C)]
struct suggest_T {
    st_word: std::ffi::CString,
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
#[derive(Clone, Copy, Default)]
struct limitscore_T {
    badi: c_int,
    goodi: c_int,
    score: c_int,
}
struct SpellSuggestState {
    spell_suggest_timeout: i64,
    sps_flags: c_int,
    sps_limit: c_int,
    expr_busy: bool,
}

impl SpellSuggestState {
    const fn new() -> Self {
        Self {
            spell_suggest_timeout: 5000,
            sps_flags: SPS_BEST,
            sps_limit: 9999,
            expr_busy: false,
        }
    }
}

static SPS_STATE: GlobalState<SpellSuggestState> = GlobalState::new(SpellSuggestState::new());

#[inline]
fn sps_st() -> &'static mut SpellSuggestState {
    unsafe { &mut *SPS_STATE.as_ptr() }
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
fn sug_clean_count(su: &suginfo_T) -> c_int {
    if su.su_maxcount < 130 {
            150
        } else {
            su.su_maxcount + 20
        }
}
#[inline]
fn sug_max_count(su: &suginfo_T) -> c_int {
    sug_clean_count(su) + 50
}

#[inline]
fn mb_cptr_back(s: *const c_char, p: &mut *const c_char) {
    unsafe {
        let off = utf_head_off(s, (*p).sub(1)) + 1;
        *p = (*p).sub(off as usize);
    }
}

#[inline]
fn was_banned(su: &mut suginfo_T, word: *const c_char) -> bool {
     {
        su
            .su_banned
            .contains(crate::memory::cstr_as_bytes(word))
    }
}

fn can_be_compound(
    sp: &trystate_T,
    slang: &mut slang_T,
    compflags: *mut u8,
    flag: c_int,
) -> bool {
    unsafe {
        let flags_cstr = if sp.ts_complen == sp.ts_compsplit {
            &slang.sl_compstartflags
        } else {
            &slang.sl_compallflags
        };
        if !byte_in_str(
            flags_cstr
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr().cast()),
            flag,
        ) {
            return false;
        }
        if slang.sl_comprules.is_some() && sp.ts_complen > sp.ts_compsplit {
            let complen = sp.ts_complen as usize;
            *compflags.add(complen) = flag as u8;
            *compflags.add(complen + 1) = 0;
            let v = match_compoundrule(slang, compflags.add(sp.ts_compsplit as usize));
            *compflags.add(complen) = 0;
            return v;
        }
        true
    }
}
fn score_wordcount_adj(
    slang: &mut slang_T,
    score: c_int,
    word: *const c_char,
    split: bool,
) -> c_int {
     {
        let Some(&wc_count) = slang
            .sl_wordcount
            .get(crate::memory::cstr_as_bytes(word))
        else {
            return score;
        };
        let bonus = if wc_count < SCORE_THRES2 {
            SCORE_COMMON1
        } else if wc_count < SCORE_THRES3 {
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
fn badword_captype(word: *mut c_char, end: *mut c_char) -> c_int {
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
fn bytes2offset(pp: *mut *mut c_char) -> c_int {
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
pub fn spell_check_sps(
    value: *const c_char,
    ret_flags: &mut c_int,
    ret_limit: &mut c_int,
) -> bool {
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
            &mut keylen,
            &raw mut val,
            &mut vallen,
        ) {
            let mut f: c_int = 0;
            if val.is_null() {
                if keylen > 0 && ascii_isdigit(*key) {
                    let mut s = key.cast_mut();
                    limit = getdigits_int(&raw mut s, true, 0);
                    if s.cast_const() != key.add(keylen) {
                        f = -1;
                    }
                } else if option_slice_eq(key, keylen, c"best") {
                    f = SPS_BEST;
                } else if option_slice_eq(key, keylen, c"fast") {
                    f = SPS_FAST;
                } else if option_slice_eq(key, keylen, c"double") {
                    f = SPS_DOUBLE;
                } else {
                    f = -1;
                }
            } else if option_slice_eq(key, keylen, c"expr")
                || option_slice_eq(key, keylen, c"file")
            {
                // consumed later in spell_find_suggest()
            } else if option_slice_eq(key, keylen, c"timeout") {
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
                return false;
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
        true
    }
}
/// Apply validated 'spellsuggest' flags and suggestion limit.
pub fn spell_apply_sps(flags: c_int, limit: c_int) {
    sps_st().sps_flags = flags;
        sps_st().sps_limit = limit;
}
fn select_spell_suggestion(sug: &mut suginfo_T) {
    unsafe {
        let mut items_tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        };
        tv_list_alloc_ret(&mut items_tv, sug.su_ga.len() as isize);
        for stp in &sug.su_ga {
            let d = tv_dict_alloc();
            let el = sug.su_badlen - stp.st_orglen;
            if el > 0 {
                let mut word_vec: Vec<c_char> = vec![0; (stp.st_wordlen + el + 1) as usize];
                let word = word_vec.as_mut_ptr();
                ptr::copy_nonoverlapping(stp.st_word.as_ptr(), word, stp.st_wordlen as usize);
                ptr::copy_nonoverlapping(
                    sug.su_badptr.add(stp.st_orglen as usize),
                    word.add(stp.st_wordlen as usize),
                    el as usize,
                );
                tv_dict_add_str(&mut *d, c"word", word);
            } else {
                tv_dict_add_str(&mut *d, c"word", stp.st_word.as_ptr());
            }
            if sug.su_badlen < stp.st_orglen {
                tv_dict_add_str_len(&mut *d, c"extra", sug.su_badptr, stp.st_orglen);
            }
            tv_dict_add_nr(&mut *d, c"score", stp.st_score as i64);
            if (sps_st().sps_flags & (SPS_DOUBLE | SPS_BEST)) != 0 {
                tv_dict_add_nr(&mut *d, c"altscore", stp.st_altscore as i64);
                tv_dict_add_bool(
                    &mut *d,
                    c"salscore",
                    if stp.st_salscore {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
            }
            let mut item = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: 0,
                vval: TypvalVval::Dict(d),
            };
            tv_list_append_tv( &mut *items_tv.vval.v_list(), &mut item);
        }
        let bad_cstr = CString::new(std::slice::from_raw_parts(
            sug.su_badptr.cast::<u8>(),
            sug.su_badlen as usize,
        ))
        .unwrap_or_default();
        let mut bad_tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        };
        tv_set_string(&mut bad_tv, bad_cstr.as_ptr());
        let mut lua_args = [
            items_tv,
            bad_tv,
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            },
        ];
        nlua_call_typval(
            c"vim._core.spell",
            c"select_suggest",
            (&raw mut lua_args).cast(),
            None,
        );
        tv_clear(&mut items_tv);
        tv_clear(&mut bad_tv);
    }
}
/// "z=": Find badly spelled word under or after the cursor.
pub fn spell_suggest(count: c_int) {
    unsafe {
        let mut badlen: c_int = 0;
        let msg_scroll_save = g_st().msg_scroll;
        let wo_spell_save = curwin_mut().w_onebuf_opt.wo_spell;
        if curwin_mut().w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin_mut());
            curwin_mut().w_onebuf_opt.wo_spell = 1;
        }
        check_cursor(curwin_mut());
        let prev_cursor = curwin_mut().w_cursor;
        'skip: {
            let w_s = curwin_mut().w_s_ptr();
            if *(*w_s).b_p_spl == NUL {
                emsg(gettext(e_no_spell));
                break 'skip;
            }
            if g_st().Visual.active {
                if curwin_mut().w_cursor.lnum != g_st().Visual.start.lnum {
                    vim_beep(kOptBoFlagSpell);
                    break 'skip;
                }
                badlen = curwin_mut().w_cursor.col - g_st().Visual.start.col;
                if badlen < 0 {
                    badlen = -badlen;
                } else {
                    curwin_mut().w_cursor.col = g_st().Visual.start.col;
                }
                badlen += 1;
                end_visual_mode();
                badlen = badlen.min(get_cursor_line_len() - curwin_mut().w_cursor.col);
            } else if spell_move_to(curwin_mut(), FORWARD, SMT_ALL, true, ptr::null_mut()) == 0
                || curwin_mut().w_cursor.col > prev_cursor.col
            {
                curwin_mut().w_cursor = prev_cursor;
                let curline = get_cursor_line_ptr();
                let mut p = curline.add(curwin_mut().w_cursor.col as usize);
                while p > curline && spell_iswordp_nmw(p, curwin_mut()) {
                    mb_ptr_back(curline, &mut p);
                }
                while *p != NUL && !spell_iswordp_nmw(p, curwin_mut()) {
                    mb_ptr_adv(&mut p);
                }
                if !spell_iswordp_nmw(p, curwin_mut()) {
                    beep_flush();
                    break 'skip;
                }
                curwin_mut().w_cursor.col = p.offset_from(curline) as colnr_T;
            }
            let need_cap = check_need_cap(curwin_mut(), curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.col);
            let mut line_vec: Vec<c_char> = std::slice::from_raw_parts(
                get_cursor_line_ptr(),
                get_cursor_line_len() as usize + 1,
            )
            .to_vec();
            line_vec[get_cursor_line_len() as usize] = NUL;
            let line = line_vec.as_mut_ptr();
            sps_st().spell_suggest_timeout = 5000;
            let mut sug = suginfo_T::default();
            let limit = sps_st().sps_limit.min(g_st().Rows - 2);
            spell_find_suggest(
                line.add(curwin_mut().w_cursor.col as usize),
                badlen,
                &mut sug,
                limit,
                true,
                need_cap,
                true,
            );
            let selected = count;
            let sug_len = sug.su_ga.len() as c_int;
            if sug_len <= 0 {
                msg_ext_set_kind(c"wmsg");
                msg(gettext(c"No suggestions"), 0);
            } else if count > 0 {
                if count > sug_len {
                    msg_ext_set_kind(c"wmsg");
                    smsg!(
                        0,
                        gettext(c"Only %lld suggestions"),
                        sug_len as i64,
                    );
                }
            } else {
                curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
                select_spell_suggestion(&mut sug);
                g_st().lines_left = g_st().Rows;
                g_st().msg_scroll = msg_scroll_save;
                return;
            }
            if selected > 0 && selected <= sug_len && u_save_cursor() {
                let stp = &sug.su_ga[(selected - 1) as usize];
                if sug.su_badlen > stp.st_orglen {
                    spell_priv_st().repl_from = Some(
                        CString::new(std::slice::from_raw_parts(
                            sug.su_badptr.cast::<u8>(),
                            sug.su_badlen as usize,
                        ))
                        .unwrap_or_default(),
                    );
                    vim_snprintf!(
                        (&raw mut g_st().IObuff).cast(),
                        IOSIZE,
                        c"%s%.*s",
                        stp.st_word.as_ptr(),
                        sug.su_badlen - stp.st_orglen,
                        sug.su_badptr.add(stp.st_orglen as usize),
                    );
                    spell_priv_st().repl_to = Some(crate::memory::cstr_to_cstring((&raw const g_st().IObuff).cast()));
                } else {
                    spell_priv_st().repl_from = Some(
                        CString::new(std::slice::from_raw_parts(
                            sug.su_badptr.cast::<u8>(),
                            stp.st_orglen as usize,
                        ))
                        .unwrap_or_default(),
                    );
                    spell_priv_st().repl_to = Some(crate::memory::cstr_to_cstring(stp.st_word.as_ptr()));
                }
                let mut p_vec: Vec<c_char> =
                    vec![0; crate::memory::cstr_len(line) - stp.st_orglen as usize + stp.st_wordlen as usize + 1];
                let p = p_vec.as_mut_ptr();
                let c = sug.su_badptr.offset_from(line) as c_int;
                ptr::copy(line, p, c as usize);
                crate::memory::cstr_cpy(p.add(c as usize), stp.st_word.as_ptr());
                crate::memory::cstr_cat(p, sug.su_badptr.add(stp.st_orglen as usize));
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
                    stp.st_wordlen + sug.su_badlen - stp.st_orglen,
                );
                redo_append_char(ESC);
                let _ = ml_replace(curwin_mut().w_cursor.lnum, p, true);
                curwin_mut().w_cursor.col = c;
                inserted_bytes(curwin_mut().w_cursor.lnum, c, stp.st_orglen, stp.st_wordlen);
            } else {
                curwin_mut().w_cursor = prev_cursor;
            }
        }
        curwin_mut().w_onebuf_opt.wo_spell = wo_spell_save;
    }
}
/// Find spell suggestions for "word".
pub fn spell_suggest_list(
    word: *mut c_char,
    maxcount: c_int,
    need_cap: bool,
    interactive: bool,
) -> Vec<CString> {
    unsafe {
        let mut sug = suginfo_T::default();
        spell_find_suggest(
            word,
            0,
            &mut sug,
            maxcount,
            false,
            need_cap,
            interactive,
        );
        let mut res = Vec::with_capacity(sug.su_ga.len());
        for stp in &sug.su_ga {
            let word_bytes = std::slice::from_raw_parts(
                stp.st_word.as_ptr().cast::<u8>(),
                stp.st_wordlen as usize,
            );
            let tail_bytes = crate::memory::cstr_as_bytes(sug.su_badptr.add(stp.st_orglen as usize));
            let mut bytes = Vec::with_capacity(word_bytes.len() + tail_bytes.len());
            bytes.extend_from_slice(word_bytes);
            bytes.extend_from_slice(tail_bytes);
            res.push(CString::new(bytes).unwrap_or_default());
        }
        res
    }
}
fn spell_find_suggest(
    badptr: *mut c_char,
    badlen: c_int,
    su: &mut suginfo_T,
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
        *su = suginfo_T::default();
        if *badptr == NUL {
            return;
        }
        su.su_badptr = badptr;
        if badlen != 0 {
            su.su_badlen = badlen;
        } else {
            let tmplen = spell_check(
                curwin_mut(),
                su.su_badptr,
                &mut attr,
                None,
                false,
            );
            su.su_badlen = tmplen as c_int;
        }
        su.su_maxcount = maxcount;
        su.su_maxscore = SCORE_MAXINIT;
        su.su_badlen = su.su_badlen.min(MAXWLEN as c_int - 1);
        crate::memory::cstr_ncpy(su.su_badword.as_mut_ptr(), su.su_badptr, su.su_badlen as usize);
        spell_casefold(
            curwin_mut(),
            su.su_badptr,
            su.su_badlen,
            (&raw mut su.su_fbadword).cast(),
            MAXWLEN as c_int,
        );
        su.su_fbadword[su.su_badlen as usize] = NUL;
        su.su_badflags = badword_captype(
            su.su_badptr,
            su.su_badptr.add(su.su_badlen as usize),
        );
        if need_cap {
            su.su_badflags |= WF_ONECAP;
        }
        for lp in &curbuf_mut().b_s.b_langp {
            if !lp.lp_sallang.is_null() {
                su.su_sallang = lp.lp_sallang;
                break;
            }
        }
        if !su.su_sallang.is_null() {
            spell_soundfold(
                &mut *su.su_sallang,
                (&raw mut su.su_fbadword).cast(),
                true,
                (&raw mut su.su_sal_badword).cast(),
            );
        }
        let c = utf_ptr2char(su.su_badptr);
        if !spell_isupper(c) && attr == HLF_COUNT {
            make_case_word(
                (&raw mut su.su_badword).cast(),
                (&raw mut buf).cast(),
                WF_ONECAP,
            );
            add_suggestion(
                su,
                false,
                (&raw const buf).cast(),
                su.su_badlen,
                SCORE_ICASE,
                0,
                true,
                su.su_sallang,
                false,
            );
        }
        if banbadword {
            add_banned(su, su.su_badword.as_ptr());
        }
        let sps_copy = crate::memory::cstr_to_cstring(opt_vars().p_sps);
        let mut p = sps_copy.as_ptr().cast_mut();
        while *p != NUL {
            copy_option_part(&raw mut p, (&raw mut buf).cast(), MAXPATHL, c",");
            let buf_ptr = (&raw mut buf).cast::<c_char>();
            if crate::memory::cstr_prefix_bytes(buf_ptr, 5) == &b"expr:"[..5] {
                if !sps_st().expr_busy {
                    sps_st().expr_busy = true;
                    spell_suggest_expr(su, buf_ptr.add(5));
                    sps_st().expr_busy = false;
                }
            } else if crate::memory::cstr_prefix_bytes(buf_ptr, 5) == &b"file:"[..5] {
                spell_suggest_file(su, buf_ptr.add(5));
            } else if crate::memory::cstr_prefix_bytes(buf_ptr, 8) == &b"timeout:"[..8] {
                sps_st().spell_suggest_timeout = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(buf_ptr.add(8))) as c_int as i64;
            } else if !did_intern {
                spell_suggest_intern(su, interactive);
                if (sps_st().sps_flags & SPS_DOUBLE) != 0 {
                    do_combine = true;
                }
                did_intern = true;
            }
        }
        if do_combine {
            score_combine(su);
        }
    }
}
fn spell_suggest_expr(su: &mut suginfo_T, expr: *mut c_char) {
    unsafe {
        let mut p: *const c_char = ptr::null();
        let list = eval_spell_expr((&raw mut su.su_badword).cast(), expr);
        if !list.is_null() {
            for li in crate::eval::typval::list_iter_mut((*list).lv_first) {
                if li.li_tv.v_type == VarType::VAR_LIST {
                    let score = get_spellword(&mut *li.li_tv.vval.v_list(), &raw mut p);
                    if score >= 0 && score <= su.su_maxscore {
                        add_suggestion(
                            su,
                            false,
                            p,
                            su.su_badlen,
                            score,
                            0,
                            true,
                            su.su_sallang,
                            false,
                        );
                    }
                }
            }
            tv_list_unref(list);
        }
        check_suggestions(su, false);
        cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
    }
}
fn spell_suggest_file(su: &mut suginfo_T, fname: *mut c_char) {
    unsafe {
        let mut line: [c_char; MAXWLEN * 2] = [0; MAXWLEN * 2];
        let mut cword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let fd = os_fopen(fname, c"r");
        if fd.is_null() {
            semsg!(gettext(e_notopen), fname);
            return;
        }
        while !vim_fgets((&raw mut line).cast(), (MAXWLEN * 2) as c_int, fd) && !g_st().got_int {
            line_breakcheck();
            let mut p = vim_strchr((&raw const line).cast(), b'/' as c_int);
            if p.is_null() {
                continue;
            }
            *p = NUL;
            p = p.add(1);
            if mb_stricmp(
                (&raw const su.su_badword).cast(),
                (&raw const line).cast(),
            ) == 0
            {
                let mut len: usize = 0;
                while (*p.add(len) as u8) >= b' ' {
                    len += 1;
                }
                *p.add(len) = NUL;
                if captype(p, ptr::null()) == 0 {
                    make_case_word(p, (&raw mut cword).cast(), su.su_badflags);
                    p = (&raw mut cword).cast();
                }
                add_suggestion(
                    su,
                    false,
                    p,
                    su.su_badlen,
                    SCORE_FILE,
                    0,
                    true,
                    su.su_sallang,
                    false,
                );
            }
        }
        fclose(fd);
        check_suggestions(su, false);
        cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
    }
}
fn spell_suggest_intern(su: &mut suginfo_T, interactive: bool) {
    suggest_load_files();
        suggest_try_special(su);
        suggest_try_change(su);
        if (sps_st().sps_flags & SPS_DOUBLE) != 0 {
            score_comp_sal(su);
        }
        if (sps_st().sps_flags & SPS_FAST) == 0 {
            if (sps_st().sps_flags & SPS_BEST) != 0 {
                rescore_suggestions(su);
            }
            suggest_try_soundalike_prep();
            su.su_maxscore = SCORE_SFMAX1;
            su.su_sfmaxscore = SCORE_MAXINIT * 3;
            suggest_try_soundalike(su);
            if (su.su_ga.len() as c_int) < sug_clean_count(su) {
                su.su_maxscore = SCORE_SFMAX2;
                suggest_try_soundalike(su);
                if (su.su_ga.len() as c_int) < sug_clean_count(su) {
                    su.su_maxscore = SCORE_SFMAX3;
                    suggest_try_soundalike(su);
                }
            }
            su.su_maxscore = su.su_sfmaxscore;
            suggest_try_soundalike_finish();
        }
        os_breakcheck();
        if interactive && g_st().got_int {
            let _ = vgetc();
            g_st().got_int = false;
        }
        if (sps_st().sps_flags & SPS_DOUBLE) == 0 && !su.su_ga.is_empty() {
            if (sps_st().sps_flags & SPS_BEST) != 0 {
                rescore_suggestions(su);
            }
            check_suggestions(su, false);
            cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
        }
}
fn suggest_try_special(su: &mut suginfo_T) {
    unsafe {
        let mut word: [c_char; MAXWLEN] = [0; MAXWLEN];
        let fbadword_ptr = (&raw mut su.su_fbadword).cast::<c_char>();
        let mut p = skiptowhite(fbadword_ptr);
        let len = p.offset_from(fbadword_ptr) as usize;
        p = skipwhite(p);
        if crate::memory::cstr_len(p) == len && (crate::memory::cstr_prefix_bytes(fbadword_ptr, len) == crate::memory::cstr_prefix_bytes(p, len)) {
            let c = su.su_fbadword[len];
            su.su_fbadword[len] = NUL;
            make_case_word(fbadword_ptr, (&raw mut word).cast(), su.su_badflags);
            su.su_fbadword[len] = c;
            add_suggestion(
                su,
                false,
                (&raw const word).cast(),
                su.su_badlen,
                rescore(SCORE_REP, 0),
                0,
                true,
                su.su_sallang,
                false,
            );
        }
    }
}
fn suggest_try_change(su: &mut suginfo_T) {
    unsafe {
        let mut fword: [c_char; MAXWLEN] = [0; MAXWLEN];
        crate::memory::cstr_cpy((&raw mut fword).cast(), (&raw const su.su_fbadword).cast());
        let mut n = crate::memory::cstr_len((&raw const fword).cast()) as c_int;
        let p = su.su_badptr.add(su.su_badlen as usize);
        spell_casefold(
            curwin_mut(),
            p,
            crate::memory::cstr_len(p) as c_int,
            (&raw mut fword).cast::<c_char>().add(n as usize),
            MAXWLEN as c_int - n,
        );
        n = crate::memory::cstr_len(su.su_badptr) as c_int;
        if (n as usize) < MAXWLEN {
            fword[n as usize] = NUL;
        }
        let w_s = curwin_mut().w_s_ptr();
        for lp in &(*w_s).b_langp {
            if (*lp.lp_slang).sl_fbyts.is_empty() {
                continue;
            }
            suggest_trie_walk(su, lp, (&raw mut fword).cast(), false);
        }
    }
}
#[inline]
fn try_deeper(
    su: &suginfo_T,
    stack: &[trystate_T; MAXWLEN],
    depth: usize,
    add: c_int,
) -> bool {
    depth < MAXWLEN - 1 && stack[depth].ts_score + add < su.su_maxscore
}
fn suggest_trie_walk(su: &mut suginfo_T, lp: &langp_T, fword: *mut c_char, soundfold: bool) {
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
        let slang = &mut *lp.lp_slang;
        let mut goodword_ends: bool;
        let mut breakcheckcount: c_int = 1000;
        let mut depth: isize = 0;
        stack[0] = trystate_T::default();
        stack[0].ts_curi = 1;
        if soundfold {
            fbyts = slang.sl_sbyts.as_mut_ptr();
            byts = fbyts;
            fidxs = slang.sl_sidxs.as_mut_ptr();
            idxs = fidxs;
            pbyts = ptr::null_mut();
            pidxs = ptr::null_mut();
            stack[0].ts_prefixdepth = PFD_NOPREFIX;
            stack[0].ts_state = state_T::STATE_START;
        } else {
            fbyts = slang.sl_fbyts.as_mut_ptr();
            fidxs = slang.sl_fidxs.as_mut_ptr();
            if !slang.sl_pbyts.is_empty() {
                pbyts = slang.sl_pbyts.as_mut_ptr();
                pidxs = slang.sl_pidxs.as_mut_ptr();
                byts = pbyts;
                idxs = pidxs;
                stack[0].ts_prefixdepth = PFD_PREFIXTREE;
                stack[0].ts_state = state_T::STATE_NOPREFIX;
            } else {
                pbyts = ptr::null_mut();
                pidxs = ptr::null_mut();
                byts = fbyts;
                idxs = fidxs;
                stack[0].ts_prefixdepth = PFD_NOPREFIX;
                stack[0].ts_state = state_T::STATE_START;
            }
        }
        let mut time_limit: proftime_T = 0;
        if sps_st().spell_suggest_timeout > 0 {
            time_limit = profile_setlimit(sps_st().spell_suggest_timeout);
        }
        while depth >= 0 && !g_st().got_int {
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
                        stack[udepth].ts_save_badflags = su.su_badflags as u8;
                        if udepth < MAXWLEN - 1 && (*byts.add(arridx as usize) == 0 || was_noprefix)
                        {
                            n = nofold_len(fword, stack[udepth].ts_fidx as c_int, su.su_badptr);
                            let flags =
                                badword_captype(su.su_badptr, su.su_badptr.add(n as usize));
                            su.su_badflags = badword_captype(
                                su.su_badptr.add(n as usize),
                                su.su_badptr.add(su.su_badlen as usize),
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
                            stack[udepth].ts_prewordlen = crate::memory::cstr_len((&raw const preword).cast()) as u8;
                            stack[udepth].ts_splitoff = stack[udepth].ts_twordlen;
                        }
                        continue;
                    }
                    if stack[udepth].ts_curi as c_int > len || *byts.add(arridx as usize) != 0 {
                        stack[udepth].ts_state = state_T::STATE_ENDNUL;
                        stack[udepth].ts_save_badflags = su.su_badflags as u8;
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
                            !spell_iswordp(fword.add(stack[udepth].ts_fidx as usize), curwin_mut())
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
                        if slang.sl_nobreak {
                            if stack[udepth].ts_fidx - stack[udepth].ts_splitfidx
                                == stack[udepth].ts_twordlen - stack[udepth].ts_splitoff
                                && (crate::memory::cstr_prefix_bytes(fword.add(stack[udepth].ts_splitfidx as usize), (stack[udepth].ts_fidx - stack[udepth].ts_splitfidx) as usize) == crate::memory::cstr_prefix_bytes((&raw const tword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_splitoff as usize), (stack[udepth].ts_fidx - stack[udepth].ts_splitfidx) as usize))
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
                                if newscore <= su.su_maxscore {
                                    add_suggestion(
                                        su,
                                        false,
                                        (&raw const preword).cast(),
                                        stack[udepth].ts_splitfidx as c_int - repextra,
                                        newscore,
                                        0,
                                        false,
                                        lp.lp_sallang,
                                        false,
                                    );
                                }
                                continue;
                            }
                        } else {
                            if ((flags as u32) >> 24) == 0
                                || c_int::from(
                                    stack[udepth].ts_twordlen - stack[udepth].ts_splitoff,
                                ) < slang.sl_compminlen
                            {
                                continue;
                            }
                            if slang.sl_compminlen > 0
                                && mb_charlen(
                                    (&raw const tword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_splitoff as usize),
                                ) < slang.sl_compminlen
                            {
                                continue;
                            }
                            compflags[stack[udepth].ts_complen as usize] =
                                ((flags as u32) >> 24) as u8;
                            compflags[stack[udepth].ts_complen as usize + 1] = 0;
                            crate::memory::cstr_ncpy((&raw mut preword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_prewordlen as usize), (&raw const tword)
                                    .cast::<c_char>()
                                    .add(stack[udepth].ts_splitoff as usize), (stack[udepth].ts_twordlen - stack[udepth].ts_splitoff) as usize);
                            if match_checkcompoundpattern(
                                (&raw mut preword).cast(),
                                stack[udepth].ts_prewordlen as c_int,
                                &slang.sl_comppat,
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
                            mb_ptr_back(&raw const preword, &mut p);
                        }
                    }
                    if soundfold {
                        crate::memory::cstr_cpy((&raw mut preword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_prewordlen as usize), (&raw const tword)
                                .cast::<c_char>()
                                .add(stack[udepth].ts_splitoff as usize));
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
                        c = su.su_badflags;
                        if (c & WF_ALLCAP) != 0 && su.su_badlen == utfc_ptr2len(su.su_badptr)
                        {
                            c = WF_ONECAP;
                        }
                        c |= flags;
                        if !p.is_null() && spell_iswordp_nmw(p, curwin_mut()) {
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
                            if slang.sl_compprog.is_none() {
                                continue;
                            }
                            goodword_ends = false;
                        }
                    }
                    let mut newscore: c_int = 0;
                    if !soundfold {
                        if (flags & WF_REGION) != 0
                            && (((flags as u32) >> 16) & (lp.lp_region as u32)) == 0
                        {
                            newscore += SCORE_REGION;
                        }
                        if (flags & WF_RARE) != 0 {
                            newscore += SCORE_RARE;
                        }
                        if !spell_valid_case(
                            su.su_badflags,
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
                            if !spell_iswordp(p, curwin_mut()) && preword[0] != NUL {
                                p = (&raw mut preword)
                                    .cast::<c_char>()
                                    .add(crate::memory::cstr_len((&raw const preword).cast()));
                                mb_ptr_back(&raw const preword, &mut p);
                                if spell_iswordp(p, curwin_mut()) {
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
                            if score <= su.su_maxscore {
                                add_suggestion(
                                    su,
                                    false,
                                    (&raw const preword).cast(),
                                    stack[udepth].ts_fidx as c_int - repextra,
                                    score,
                                    0,
                                    false,
                                    lp.lp_sallang,
                                    false,
                                );
                                if (su.su_badflags & WF_MIXCAP) != 0 {
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
                                            false,
                                            (&raw const preword).cast(),
                                            stack[udepth].ts_fidx as c_int - repextra,
                                            score + SCORE_ICASE,
                                            0,
                                            false,
                                            lp.lp_sallang,
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
                            < su.su_badlen
                            && !soundfold;
                        let mut try_compound = false;
                        if !soundfold
                            && !slang.sl_nocompoundsugs
                            && slang.sl_compprog.is_some()
                            && ((flags as u32) >> 24) != 0
                            && (stack[udepth].ts_twordlen - stack[udepth].ts_splitoff) as c_int
                                >= slang.sl_compminlen
                            && (slang.sl_compminlen == 0
                                || mb_charlen(
                                    (&raw const tword)
                                        .cast::<c_char>()
                                        .add(stack[udepth].ts_splitoff as usize),
                                ) >= slang.sl_compminlen)
                            && (slang.sl_compsylmax < MAXWLEN as c_int
                                || c_int::from(
                                    stack[udepth].ts_complen + 1 - stack[udepth].ts_compsplit,
                                ) < slang.sl_compmax)
                            && can_be_compound(
                                &stack[udepth],
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
                        if slang.sl_nobreak && !slang.sl_nocompoundsugs {
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
                                if slang.sl_nosplitsugs {
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
                                stack[udepth].ts_save_badflags = su.su_badflags as u8;
                                stack[udepth].ts_state = state_T::STATE_SPLITUNDO;
                                depth += 1;
                                udepth = depth as usize;
                                if !try_compound && !fword_ends {
                                    crate::memory::cstr_cat((&raw mut preword).cast::<c_char>(), c" ");
                                }
                                stack[udepth].ts_prewordlen =
                                    crate::memory::cstr_len((&raw const preword).cast()) as u8;
                                stack[udepth].ts_splitoff = stack[udepth].ts_twordlen;
                                stack[udepth].ts_splitfidx = stack[udepth].ts_fidx;
                                if ((!try_compound
                                    && !spell_iswordp_nmw(
                                        fword.add(stack[udepth].ts_fidx as usize),
                                        curwin_mut(),
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
                                    su.su_badptr,
                                );
                                su.su_badflags = badword_captype(
                                    su.su_badptr.add(n as usize),
                                    su.su_badptr.add(su.su_badlen as usize),
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
                    su.su_badflags = stack[udepth].ts_save_badflags as c_int;
                    stack[udepth].ts_state = state_T::STATE_START;
                    byts = fbyts;
                    idxs = fidxs;
                }
                state_T::STATE_ENDNUL | state_T::STATE_PLAIN => {
                    let mut goto_del = false;
                    if state == state_T::STATE_ENDNUL {
                        su.su_badflags = stack[udepth].ts_save_badflags as c_int;
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
                                            && slang.sl_has_map
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
                                            mb_ptr_back(&raw const tword, &mut p);
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
                    if byts == slang.sl_fbyts.as_mut_ptr()
                        && n >= slang.sl_fbyts.len() as c_int
                    {
                        g_st().got_int = true;
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
                    if !soundfold && !spell_iswordp(p, curwin_mut()) {
                        stack[udepth].ts_state = state_T::STATE_REP_INI;
                        continue;
                    }
                    n = utf_ptr2len(p);
                    c = utf_ptr2char(p);
                    if *p.add(n as usize) == NUL {
                        c2 = NUL as c_int;
                    } else if !soundfold && !spell_iswordp(p.add(n as usize), curwin_mut()) {
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
                    if !soundfold && !spell_iswordp(p.add((n + fl) as usize), curwin_mut()) {
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
                    if !soundfold && !spell_iswordp(p, curwin_mut()) {
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
                        if (lp.lp_replang.is_null() && !soundfold)
                            || stack[udepth].ts_score + SCORE_REP >= su.su_maxscore
                            || stack[udepth].ts_fidx < stack[udepth].ts_fidxtry
                        {
                            stack[udepth].ts_state = state_T::STATE_FINAL;
                            do_rep = false;
                        } else {
                            let first_byte =
                                *fword.add(stack[udepth].ts_fidx as usize) as u8 as usize;
                            stack[udepth].ts_curi = if soundfold {
                                slang.sl_repsal_first[first_byte]
                            } else {
                                (*lp.lp_replang).sl_rep_first[first_byte]
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
                            &slang.sl_repsal
                        } else {
                            &(*lp.lp_replang).sl_rep
                        };
                        while (stack[udepth].ts_curi as usize) < gap.len() {
                            let curi = stack[udepth].ts_curi;
                            stack[udepth].ts_curi += 1;
                            let ftp = &gap[curi as usize];
                            if *ftp.ft_from.as_ptr() != *p {
                                stack[udepth].ts_curi = gap.len() as i16;
                                break;
                            }
                            if (crate::memory::cstr_prefix_bytes(ftp.ft_from.as_ptr(), ftp.ft_from.as_bytes().len()) == crate::memory::cstr_prefix_bytes(p, ftp.ft_from.as_bytes().len()))
                                && try_deeper(su, &stack, udepth, SCORE_REP)
                            {
                                go_deeper(&mut stack, udepth, SCORE_REP);
                                stack[udepth].ts_state = state_T::STATE_REP_UNDO;
                                let old_fidx = stack[udepth].ts_fidx;
                                depth += 1;
                                udepth = depth as usize;
                                fl = ftp.ft_from.as_bytes().len() as c_int;
                                tl = ftp.ft_to.as_bytes().len() as c_int;
                                if fl != tl {
                                    crate::memory::cstr_move(p.add(tl as usize), p.add(fl as usize));
                                    repextra += tl - fl;
                                }
                                ptr::copy(ftp.ft_to.as_ptr(), p, tl as usize);
                                stack[udepth].ts_fidxtry = old_fidx + tl as u8;
                                stack[udepth].ts_tcharlen = 0;
                                break;
                            }
                        }
                        if (stack[udepth].ts_curi as usize) >= gap.len()
                            && stack[udepth].ts_state == state_T::STATE_REP
                        {
                            stack[udepth].ts_state = state_T::STATE_FINAL;
                        }
                    }
                }
                state_T::STATE_REP_UNDO => {
                    let gap = if soundfold {
                        &slang.sl_repsal
                    } else {
                        &(*lp.lp_replang).sl_rep
                    };
                    let ftp = &gap[(stack[udepth].ts_curi - 1) as usize];
                    fl = ftp.ft_from.as_bytes().len() as c_int;
                    tl = ftp.ft_to.as_bytes().len() as c_int;
                    let p = fword.add(stack[udepth].ts_fidx as usize);
                    if fl != tl {
                        crate::memory::cstr_move(p.add(fl as usize), p.add(tl as usize));
                        repextra -= tl - fl;
                    }
                    ptr::copy(ftp.ft_from.as_ptr(), p, fl as usize);
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
                        if sps_st().spell_suggest_timeout > 0 && profile_passed_limit(time_limit) {
                            g_st().got_int = true;
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
fn find_keepcap_word(slang: &mut slang_T, fword: *mut c_char, kword: *mut c_char) {
    unsafe {
        let mut uword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut arridx: [idx_T; MAXWLEN] = [0; MAXWLEN];
        let mut round: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut fwordidx: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut uwordidx: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut kwordlen: [c_int; MAXWLEN] = [0; MAXWLEN];
        if slang.sl_kbyts.is_empty() {
            *kword = NUL;
            return;
        }
        let byts = slang.sl_kbyts.as_mut_ptr();
        let idxs = slang.sl_kidxs.as_mut_ptr();
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
                            ptr::copy_nonoverlapping(
                                fword.add(fwordidx[udepth] as usize),
                                kword.add(kwordlen[udepth] as usize),
                                flen as usize,
                            );
                            kwordlen[udepth + 1] = kwordlen[udepth] + flen;
                        } else {
                            ptr::copy_nonoverlapping(
                                (&raw const uword)
                                    .cast::<c_char>()
                                    .add(uwordidx[udepth] as usize),
                                kword.add(kwordlen[udepth] as usize),
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
fn score_comp_sal(su: &mut suginfo_T) {
    unsafe {
        let mut badsound: [c_char; MAXWLEN] = [0; MAXWLEN];
        let sga = &mut su.su_sga;
        sga.reserve(su.su_ga.len());
        let w_s = curwin_mut().w_s_ptr();
        for lp in &(*w_s).b_langp {
            if (*lp.lp_slang).has_sal() {
                spell_soundfold(
                    &mut *lp.lp_slang,
                    (&raw mut su.su_fbadword).cast(),
                    true,
                    (&raw mut badsound).cast(),
                );
                for stp in &su.su_ga {
                    let score = stp_sal_score(stp, su, &mut *lp.lp_slang, (&raw mut badsound).cast());
                    if score < SCORE_MAXMAX {
                        let sga = &mut su.su_sga;
                        sga.push(suggest_T {
                            st_word: stp.st_word.clone(),
                            st_wordlen: stp.st_wordlen,
                            st_orglen: stp.st_orglen,
                            st_score: score,
                            st_altscore: 0,
                            st_salscore: false,
                            st_had_bonus: false,
                            st_slang: ptr::null_mut(),
                        });
                    }
                }
                break;
            }
        }
    }
}
fn score_combine(su: &mut suginfo_T) {
    unsafe {
        let mut badsound: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut slang: *mut slang_T = ptr::null_mut();
        let w_s = curwin_mut().w_s_ptr();
        for lp in &(*w_s).b_langp {
            if (*lp.lp_slang).has_sal() {
                slang = lp.lp_slang;
                spell_soundfold(
                    &mut *slang,
                    (&raw mut su.su_fbadword).cast(),
                    true,
                    (&raw mut badsound).cast(),
                );
                let mut su_ga = std::mem::take(&mut su.su_ga);
                for stp in &mut su_ga {
                    stp.st_altscore = stp_sal_score(stp, su, &mut *slang, (&raw mut badsound).cast());
                    if stp.st_altscore == SCORE_MAXMAX {
                        stp.st_score = (stp.st_score * 3 + SCORE_BIG) / 4;
                    } else {
                        stp.st_score = (stp.st_score * 3 + stp.st_altscore) / 4;
                    }
                    stp.st_salscore = false;
                }
                su.su_ga = su_ga;
                break;
            }
        }
        if slang.is_null() {
            cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
            return;
        }
        for stp in &mut su.su_sga {
            stp.st_altscore = spell_edit_score(
                slang.as_mut(),
                (&raw const su.su_badword).cast(),
                stp.st_word.as_ptr(),
            );
            if stp.st_score == SCORE_MAXMAX {
                stp.st_score = (SCORE_BIG * 7 + stp.st_altscore) / 8;
            } else {
                stp.st_score = (stp.st_score * 7 + stp.st_altscore) / 8;
            }
            stp.st_salscore = true;
        }
        check_suggestions(su, false);
        cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
        check_suggestions(su, true);
        cleanup_suggestions(&mut su.su_sga, su.su_maxscore, su.su_maxcount);
        let mut ga: Vec<suggest_T> = Vec::with_capacity(su.su_ga.len() + su.su_sga.len());
        let mut old_ga = std::mem::take(&mut su.su_ga).into_iter();
        let mut old_sga = std::mem::take(&mut su.su_sga).into_iter();
        loop {
            let mut any = false;
            if let Some(cand) = old_ga.next() {
                any = true;
                if !ga.iter().any(|existing| existing.st_word == cand.st_word) {
                    ga.push(cand);
                }
            }
            if let Some(cand) = old_sga.next() {
                any = true;
                if !ga.iter().any(|existing| existing.st_word == cand.st_word) {
                    ga.push(cand);
                }
            }
            if !any {
                break;
            }
        }
        let maxcount = su.su_maxcount.max(0) as usize;
        ga.truncate(maxcount);
        su.su_ga = ga;
    }
}
fn stp_sal_score(
    stp: &suggest_T,
    su: &suginfo_T,
    slang: &mut slang_T,
    badsound: *mut c_char,
) -> c_int {
    unsafe {
        let mut badsound2: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut fword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut goodsound: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut goodword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let lendiff = su.su_badlen - stp.st_orglen;
        let pbad = if lendiff >= 0 {
            badsound
        } else {
            spell_casefold(
                curwin_mut(),
                su.su_badptr,
                stp.st_orglen,
                (&raw mut fword).cast(),
                MAXWLEN as c_int,
            );
            if ascii_iswhite(*su.su_badptr.add(su.su_badlen as usize))
                && *skiptowhite(stp.st_word.as_ptr()) == NUL
            {
                let mut p = (&raw mut fword).cast::<c_char>();
                loop {
                    p = skiptowhite(p);
                    if *p == NUL {
                        break;
                    }
                    crate::memory::cstr_move(p, p.add(1));
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
        let pgood = if lendiff > 0 && (stp.st_wordlen + lendiff) < MAXWLEN as c_int {
            crate::memory::cstr_cpy((&raw mut goodword).cast(), stp.st_word.as_ptr());
            crate::memory::cstr_ncpy((&raw mut goodword)
                    .cast::<c_char>()
                    .add(stp.st_wordlen as usize), su
                    .su_badptr
                    .add((su.su_badlen - lendiff) as usize), lendiff as usize);
            (&raw mut goodword).cast()
        } else {
            stp.st_word.as_ptr().cast_mut()
        };
        spell_soundfold(slang, pgood, false, (&raw mut goodsound).cast());
        soundalike_score((&raw mut goodsound).cast(), pbad)
    }
}
fn suggest_try_soundalike_prep() {
    unsafe {
        let w_s = curwin_mut().w_s_ptr();
        for lp in &(*w_s).b_langp {
            let slang = &mut *lp.lp_slang;
            if slang.has_sal() && !slang.sl_sbyts.is_empty() {
                slang.sl_sounddone.clear();
            }
        }
    }
}
fn suggest_try_soundalike(su: &mut suginfo_T) {
    unsafe {
        let mut salword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let w_s = curwin_mut().w_s_ptr();
        for lp in &(*w_s).b_langp {
            let slang = &mut *lp.lp_slang;
            if slang.has_sal() && !slang.sl_sbyts.is_empty() {
                spell_soundfold(
                    &mut *slang,
                    (&raw mut su.su_fbadword).cast(),
                    true,
                    (&raw mut salword).cast(),
                );
                suggest_trie_walk(su, lp, (&raw mut salword).cast(), true);
            }
        }
    }
}
fn suggest_try_soundalike_finish() {
    unsafe {
        let w_s = curwin_mut().w_s_ptr();
        for lp in &(*w_s).b_langp {
            let slang = &mut *lp.lp_slang;
            if slang.has_sal() && !slang.sl_sbyts.is_empty() {
                slang.sl_sounddone.clear();
            }
        }
    }
}
fn add_sound_suggest(su: &mut suginfo_T, goodword: *mut c_char, score: c_int, lp: &langp_T) {
    unsafe {
        let slang = &mut *lp.lp_slang;
        let mut theword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let goodword_bytes = crate::memory::cstr_as_bytes(goodword);
        if let Some(prev_score) = slang.sl_sounddone.get_mut(goodword_bytes) {
            if score >= *prev_score as c_int {
                return;
            }
            *prev_score = score as i16;
        } else {
            slang
                .sl_sounddone
                .insert(goodword_bytes.to_vec(), score as i16);
        }
        let sfwordnr = soundfold_find(slang, goodword);
        if sfwordnr < 0 {
            internal_error(c"add_sound_suggest()");
            return;
        }
        let sugbuf = slang
            .sl_sugbuf
            .as_deref_mut()
            .map_or(ptr::null_mut(), |b| &raw mut *b);
        let mut nrline = ml_get_buf(&mut *sugbuf, sfwordnr + 1);
        let mut orgnr: c_int = 0;
        while *nrline != NUL {
            orgnr += bytes2offset(&raw mut nrline);
            let byts = slang.sl_fbyts.as_mut_ptr();
            let idxs = slang.sl_fidxs.as_mut_ptr();
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
                            crate::memory::cstr_cpy((&raw mut theword).cast::<c_char>().add(wlen), c"BAD");
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
                    flags |= su.su_badflags;
                    if (flags & WF_CAPMASK) != 0 {
                        make_case_word((&raw mut theword).cast(), (&raw mut cword).cast(), flags);
                        (&raw mut cword).cast()
                    } else {
                        (&raw mut theword).cast()
                    }
                };
                if (sps_st().sps_flags & SPS_DOUBLE) != 0 {
                    if score <= su.su_maxscore {
                        add_suggestion(
                            su,
                            true,
                            p,
                            su.su_badlen,
                            score,
                            0,
                            false,
                            slang,
                            false,
                        );
                    }
                } else {
                    let mut goodscore = if (flags & WF_REGION) != 0
                        && (((flags as u32) >> 16) & (lp.lp_region as u32)) == 0
                    {
                        SCORE_REGION
                    } else {
                        0
                    };
                    let gc = utf_ptr2char(p);
                    if spell_isupper(gc) {
                        let bc = utf_ptr2char((&raw const su.su_badword).cast());
                        if !spell_isupper(bc) && spell_tofold(bc) != spell_tofold(gc) {
                            goodscore += SCORE_ICASE / 2;
                        }
                    }
                    let limit = maxscore(su.su_sfmaxscore - goodscore, score);
                    if limit > SCORE_LIMITMAX {
                        goodscore +=
                            spell_edit_score(Some(&mut *slang), (&raw const su.su_badword).cast(), p);
                    } else {
                        goodscore += spell_edit_score_limit(
                            Some(&mut *slang),
                            (&raw mut su.su_badword).cast(),
                            p,
                            limit,
                        );
                    }
                    if goodscore < SCORE_MAXMAX {
                        goodscore = score_wordcount_adj(slang, goodscore, p, false);
                        goodscore = rescore(goodscore, score);
                        if goodscore <= su.su_sfmaxscore {
                            add_suggestion(
                                su,
                                false,
                                p,
                                su.su_badlen,
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
fn soundfold_find(slang: &mut slang_T, word: *mut c_char) -> c_int {
    unsafe {
        let mut arridx: idx_T = 0;
        let mut wlen: usize = 0;
        let ptr = word.cast::<u8>();
        let mut wordnr: c_int = 0;
        let byts = slang.sl_sbyts.as_mut_ptr();
        let idxs = slang.sl_sidxs.as_mut_ptr();
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
fn similar_chars(slang: &mut slang_T, c1: c_int, c2: c_int) -> bool {
    let m1 = if c1 >= 256 {
            slang.sl_map_hash.get(&c1).copied().unwrap_or(0)
        } else if c1 >= 0 {
            slang.sl_map_array[c1 as usize]
        } else {
            0
        };
        if m1 == 0 {
            return false;
        }
        let m2 = if c2 >= 256 {
            slang.sl_map_hash.get(&c2).copied().unwrap_or(0)
        } else if c2 >= 0 {
            slang.sl_map_array[c2 as usize]
        } else {
            0
        };
        m1 == m2
}
fn add_suggestion(
    su: &mut suginfo_T,
    use_sga: bool,
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
        let mut pgood = goodword.add(crate::memory::cstr_len(goodword));
        let mut pbad: *mut c_char = su.su_badptr.add(badlenarg as usize);
        loop {
            goodlen = pgood.offset_from(goodword) as c_int;
            badlen = pbad.offset_from(su.su_badptr) as c_int;
            if goodlen <= 0 || badlen <= 0 {
                break;
            }
            mb_cptr_back(goodword, &mut pgood);
            mb_ptr_back(su.su_badptr, &mut pbad);
            if utf_ptr2char(pgood) != utf_ptr2char(pbad) {
                break;
            }
        }
        if badlen == 0 && goodlen == 0 {
            return;
        }
        let mut found = false;
        let mut gap_vec = if use_sga { std::mem::take(&mut su.su_sga) } else { std::mem::take(&mut su.su_ga) };
        let gap = &mut gap_vec;
        for stp in gap.iter_mut() {
            if stp.st_wordlen == goodlen
                && stp.st_orglen == badlen
                && (crate::memory::cstr_prefix_bytes(stp.st_word.as_ptr(), goodlen as usize) == crate::memory::cstr_prefix_bytes(goodword, goodlen as usize))
            {
                if stp.st_slang.is_null() {
                    stp.st_slang = slang;
                }
                let mut new_sug = suggest_T {
                    st_word: std::ffi::CString::default(),
                    st_wordlen: 0,
                    st_orglen: 0,
                    st_score: score,
                    st_altscore: altscore,
                    st_salscore: false,
                    st_had_bonus: had_bonus,
                    st_slang: ptr::null_mut(),
                };
                if stp.st_had_bonus != had_bonus {
                    if had_bonus {
                        rescore_one(su, stp);
                    } else {
                        new_sug.st_word = stp.st_word.clone();
                        new_sug.st_wordlen = stp.st_wordlen;
                        new_sug.st_slang = stp.st_slang;
                        new_sug.st_orglen = badlen;
                        rescore_one(su, &mut new_sug);
                    }
                }
                if stp.st_score > new_sug.st_score {
                    stp.st_score = new_sug.st_score;
                    stp.st_altscore = new_sug.st_altscore;
                    stp.st_had_bonus = new_sug.st_had_bonus;
                }
                found = true;
                break;
            }
        }
        if !found {
            let bytes = std::slice::from_raw_parts(goodword.cast::<u8>(), goodlen as usize);
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            gap.push(suggest_T {
                st_word: std::ffi::CString::new(&bytes[..end]).unwrap(),
                st_wordlen: goodlen,
                st_score: score,
                st_altscore: altscore,
                st_salscore: false,
                st_had_bonus: had_bonus,
                st_orglen: badlen,
                st_slang: slang,
            });
            if (gap.len() as c_int) > sug_max_count(su) {
                if maxsf {
                    su.su_sfmaxscore =
                        cleanup_suggestions(gap, su.su_sfmaxscore, sug_clean_count(su));
                } else {
                    su.su_maxscore =
                        cleanup_suggestions(gap, su.su_maxscore, sug_clean_count(su));
                }
            }
        }
        if use_sga { su.su_sga = gap_vec; } else { su.su_ga = gap_vec; }
    }
}
fn check_suggestions(su: &mut suginfo_T, use_sga: bool) {
    unsafe {
        let badptr = su.su_badptr;
        let gap = if use_sga { &mut su.su_sga } else { &mut su.su_ga };
        let mut longword: [c_char; MAXWLEN + 1] = [0; MAXWLEN + 1];
        if gap.is_empty() {
            return;
        }
        let mut i = gap.len() as isize - 1;
        while i >= 0 {
            let ui = i as usize;
            let st = &gap[ui];
            xstrlcpy((&raw mut longword).cast(), st.st_word.as_ptr(), MAXWLEN + 1);
            let len = st.st_wordlen as usize;
            xstrlcpy(
                (&raw mut longword).cast::<c_char>().add(len),
                badptr.add(st.st_orglen as usize),
                MAXWLEN + 1 - len,
            );
            let mut attr: hlf_T = HLF_COUNT;
            let _ = spell_check(
                curwin_mut(),
                (&raw mut longword).cast(),
                &mut attr,
                None,
                false,
            );
            if attr != HLF_COUNT {
                gap.remove(ui);
            }
            i -= 1;
        }
    }
}
fn add_banned(su: &mut suginfo_T, word: *const c_char) {
     {
        let word_bytes = crate::memory::cstr_as_bytes(word);
        if !su.su_banned.contains(word_bytes) {
            su.su_banned.insert(word_bytes.to_vec());
        }
    }
}
fn rescore_suggestions(su: &mut suginfo_T) {
    if !su.su_sallang.is_null() {
            let mut su_ga = std::mem::take(&mut su.su_ga);
            for stp in &mut su_ga {
                rescore_one(su, stp);
            }
            su.su_ga = su_ga;
        }
}
fn rescore_one(su: &mut suginfo_T, stp: &mut suggest_T) {
    unsafe {
        let slang = stp.st_slang;
        let mut sal_badword: [c_char; MAXWLEN] = [0; MAXWLEN];
        if !slang.is_null() && (*slang).has_sal() && !stp.st_had_bonus {
            let p: *mut c_char = if slang == su.su_sallang {
                (&raw mut su.su_sal_badword).cast()
            } else {
                spell_soundfold(
                    &mut *slang,
                    (&raw mut su.su_fbadword).cast(),
                    true,
                    (&raw mut sal_badword).cast(),
                );
                (&raw mut sal_badword).cast()
            };
            stp.st_altscore = stp_sal_score(stp, su, &mut *slang, p);
            if stp.st_altscore == SCORE_MAXMAX {
                stp.st_altscore = SCORE_BIG;
            }
            stp.st_score = rescore(stp.st_score, stp.st_altscore);
            stp.st_had_bonus = true;
        }
    }
}
fn sug_compare(p1: &suggest_T, p2: &suggest_T) -> std::cmp::Ordering {
    p1.st_score
            .cmp(&p2.st_score)
            .then_with(|| p1.st_altscore.cmp(&p2.st_altscore))
            .then_with(|| mb_stricmp(p1.st_word.as_ptr(), p2.st_word.as_ptr()).cmp(&0))
}
fn cleanup_suggestions(gap: &mut Vec<suggest_T>, maxscore: c_int, keep: c_int) -> c_int {
    if gap.is_empty() {
            return maxscore;
        }
        gap.sort_unstable_by(sug_compare);
        let keep_usize = keep.max(0) as usize;
        if gap.len() > keep_usize {
            gap.truncate(keep_usize);
            if keep >= 1 {
                return gap[keep_usize - 1].st_score;
            }
        }
        maxscore
}
fn soundalike_score(goodstart: *mut c_char, badstart: *mut c_char) -> c_int {
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
        let goodlen = crate::memory::cstr_len(goodsound) as c_int;
        let badlen = crate::memory::cstr_len(badsound) as c_int;
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
                if crate::memory::cstr_as_bytes(pl.add(1)) == crate::memory::cstr_as_bytes(ps) {
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
                if *pl2 == *ps2.add(1) && *pl2.add(1) == *ps2 && (crate::memory::cstr_as_bytes(pl2.add(2)) == crate::memory::cstr_as_bytes(ps2.add(2)))
                {
                    return score + SCORE_DEL + SCORE_SWAP;
                }
                if crate::memory::cstr_as_bytes(pl2.add(1)) == crate::memory::cstr_as_bytes(ps2.add(1)) {
                    return score + SCORE_DEL + SCORE_SUBST;
                }
                if *pl == *ps.add(1) && *pl.add(1) == *ps {
                    pl2 = pl.add(2);
                    ps2 = ps.add(2);
                    while *pl2 == *ps2 {
                        pl2 = pl2.add(1);
                        ps2 = ps2.add(1);
                    }
                    if crate::memory::cstr_as_bytes(pl2.add(1)) == crate::memory::cstr_as_bytes(ps2) {
                        return score + SCORE_SWAP + SCORE_DEL;
                    }
                }
                pl2 = pl.add(1);
                ps2 = ps.add(1);
                while *pl2 == *ps2 {
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if crate::memory::cstr_as_bytes(pl2.add(1)) == crate::memory::cstr_as_bytes(ps2) {
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
                        && (crate::memory::cstr_as_bytes(pl2.add(2)) == crate::memory::cstr_as_bytes(ps2.add(2)))
                    {
                        return score + SCORE_SWAP + SCORE_SWAP;
                    }
                    if crate::memory::cstr_as_bytes(pl2.add(1)) == crate::memory::cstr_as_bytes(ps2.add(1)) {
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
                if *pl2 == *ps2.add(1) && *pl2.add(1) == *ps2 && (crate::memory::cstr_as_bytes(pl2.add(2)) == crate::memory::cstr_as_bytes(ps2.add(2)))
                {
                    return score + SCORE_SUBST + SCORE_SWAP;
                }
                if crate::memory::cstr_as_bytes(pl2.add(1)) == crate::memory::cstr_as_bytes(ps2.add(1)) {
                    return score + SCORE_SUBST + SCORE_SUBST;
                }
                pl2 = pl;
                ps2 = ps.add(1);
                while *pl2 == *ps2 {
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if crate::memory::cstr_as_bytes(pl2.add(1)) == crate::memory::cstr_as_bytes(ps2) {
                    return score + SCORE_INS + SCORE_DEL;
                }
                pl2 = pl.add(1);
                ps2 = ps;
                while *pl2 == *ps2 {
                    pl2 = pl2.add(1);
                    ps2 = ps2.add(1);
                }
                if crate::memory::cstr_as_bytes(pl2) == crate::memory::cstr_as_bytes(ps2.add(1)) {
                    return score + SCORE_INS + SCORE_DEL;
                }
            }
            _ => {}
        }
        SCORE_MAXMAX
    }
}
fn spell_edit_score(
    mut slang: Option<&mut slang_T>,
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
        let mut cnt = vec![0; stride * (goodlen + 1)];
        for j in 1..=goodlen {
            cnt[j * stride] = cnt[(j - 1) * stride] + SCORE_INS;
        }
        for i in 1..=badlen {
            cnt[i] = cnt[i - 1] + SCORE_DEL;
            for j in 1..=goodlen {
                let bc = wbadword[i - 1];
                let gc = wgoodword[j - 1];
                let idx_ij = i + j * stride;
                if bc == gc {
                    cnt[idx_ij] = cnt[(i - 1) + (j - 1) * stride];
                } else {
                    let prev_diag = cnt[(i - 1) + (j - 1) * stride];
                    let mut val = if spell_tofold(bc) == spell_tofold(gc) {
                        SCORE_ICASE + prev_diag
                    } else if slang.is_some()
                        && slang.as_deref_mut().unwrap().sl_has_map
                        && similar_chars(slang.as_deref_mut().unwrap(), gc, bc)
                    {
                        SCORE_SIMILAR + prev_diag
                    } else {
                        SCORE_SUBST + prev_diag
                    };
                    if i > 1 && j > 1 {
                        let pbc = wbadword[i - 2];
                        let pgc = wgoodword[j - 2];
                        if bc == pgc && pbc == gc {
                            let t = SCORE_SWAP + cnt[(i - 2) + (j - 2) * stride];
                            val = val.min(t);
                        }
                    }
                    let t_del = SCORE_DEL + cnt[(i - 1) + j * stride];
                    val = val.min(t_del);
                    let t_ins = SCORE_INS + cnt[i + (j - 1) * stride];
                    val = val.min(t_ins);
                    cnt[idx_ij] = val;
                }
            }
        }
        cnt[(badlen - 1) + (goodlen - 1) * stride]
    }
}
fn spell_edit_score_limit(
    slang: Option<&mut slang_T>,
    badword: *mut c_char,
    goodword: *mut c_char,
    limit: c_int,
) -> c_int {
    spell_edit_score_limit_w(slang, badword, goodword, limit)
}
fn spell_edit_score_limit_w(
    mut slang: Option<&mut slang_T>,
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
                    } else if slang.is_some()
                        && slang.as_deref_mut().unwrap().sl_has_map
                        && similar_chars(slang.as_deref_mut().unwrap(), gc, bc)
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
