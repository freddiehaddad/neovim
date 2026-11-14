//! Functions for spelling suggestions (`src/nvim/spellsuggest.c`).
#![forbid(unsafe_code)]

use crate::buffer::FileBuf;
use crate::change::inserted_bytes;
use crate::charset::{ascii_iswhite, getdigits_int};
use crate::cursor::{Colnr, check_cursor, get_cursor_line_len, get_cursor_line_slice};
use crate::errors::*;
use crate::eval::typval::{
    TypVal, TypvalVval, VarType, tv_clear, tv_dict_add_bool, tv_dict_add_nr, tv_dict_add_str,
    tv_dict_add_str_len, tv_dict_alloc, tv_list_alloc_ret, tv_list_append_tv, tv_list_unref,
    tv_set_string,
};
use crate::eval::vars::{eval_spell_expr, get_spellword};
use crate::fileio::vim_fgets;
use crate::globals::*;
use crate::input::{beep_flush, prep_redo, redo_append_char, redo_append_lit, vgetc};
use crate::input_cmdatom::CmdSpec;
use crate::lua::executor::nlua_call_typval;
use crate::mbyte::{
    mb_charlen, mb_cptr2char_adv, mb_stricmp, utf_byte2len, utf_char2bytes, utf_char2len,
    utf_iscomposing_legacy, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::memline::ml_replace;
use crate::message::{emsg, internal_error, msg, msg_ext_set_kind, semsg, smsg};
use crate::normal::end_visual_mode;
use crate::option::{copy_option_part, option_next_keyval};
use crate::option_vars::*;
use crate::os::fs::{CFile, MAXPATHL};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::profile::{Proftime, profile_passed_limit, profile_setlimit};
use crate::spell::{
    MAXWLEN, WF_ALLCAP, WF_BANNED, WF_CAPMASK, WF_KEEPCAP, WF_NEEDCOMP, WF_ONECAP, WF_RARE,
    WF_RAREPFX, WF_REGION, allcap_copy, byte_in_str, can_compound, captype, check_need_cap,
    make_case_word, match_checkcompoundpattern, match_compoundrule, nofold_len, parse_spelllang,
    spell_casefold, spell_check, spell_isupper, spell_iswordp, spell_iswordp_nmw, spell_move_to,
    spell_soundfold, spell_tofold, spell_valid_case, valid_word_prefix, with_spell_ext,
};
use crate::spellfile::{MAXREGIONS, WF_NOSUGGEST, suggest_load_files};
use crate::strings::{format_to_buf, gettext};
use crate::ui::vim_beep;
use crate::undo::u_save_cursor;
use crate::viml::parser::expressions::TAB;
use std::ffi::{CStr, CString};
use std::ptr;
pub type Idx = i32;
pub type Salfirst = i32;
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};
use crate::drawline::{HLF_COUNT, SMT_ALL};
use crate::ex_getln::Hlf;
use crate::keycodes::ESC;
use crate::search::FORWARD;
const WF_MIXCAP: i32 = 0x20;
const SCORE_SPLIT: i32 = 149;
const SCORE_SPLIT_NO: i32 = 249;
const SCORE_ICASE: i32 = 52;
const SCORE_REGION: i32 = 200;
const SCORE_RARE: i32 = 180;
const SCORE_SWAP: i32 = 75;
const SCORE_SWAP3: i32 = 110;
const SCORE_REP: i32 = 65;
const SCORE_SUBST: i32 = 93;
const SCORE_SIMILAR: i32 = 33;
const SCORE_SUBCOMP: i32 = 33;
const SCORE_DEL: i32 = 94;
const SCORE_DELDUP: i32 = 66;
const SCORE_DELCOMP: i32 = 28;
const SCORE_INS: i32 = 96;
const SCORE_INSDUP: i32 = 67;
const SCORE_INSCOMP: i32 = 30;
const SCORE_NONWORD: i32 = 103;
const SCORE_FILE: i32 = 30;
const SCORE_MAXINIT: i32 = 350;
const SCORE_COMMON1: i32 = 30;
const SCORE_COMMON2: i32 = 40;
const SCORE_COMMON3: i32 = 50;
const SCORE_THRES2: u16 = 10;
const SCORE_THRES3: u16 = 100;
const SCORE_SFMAX1: i32 = 200;
const SCORE_SFMAX2: i32 = 300;
const SCORE_SFMAX3: i32 = 400;
const SCORE_BIG: i32 = SCORE_INS * 3;
const SCORE_MAXMAX: i32 = 999999;
const SCORE_LIMITMAX: i32 = 350;
const SCORE_EDIT_MIN: i32 = SCORE_SIMILAR;
const DIFF_NONE: u8 = 0;
const DIFF_YES: u8 = 1;
const DIFF_INSERT: u8 = 2;
const TSF_PREFIXOK: u8 = 1;
const TSF_DIDSPLIT: u8 = 2;
const TSF_DIDDEL: u8 = 4;
const PFD_NOPREFIX: u8 = 0xff;
const PFD_PREFIXTREE: u8 = 0xfe;
const PFD_NOTSPECIAL: u8 = 0xfd;
const SPS_BEST: i32 = 1;
const SPS_FAST: i32 = 2;
const SPS_DOUBLE: i32 = 4;
const K_OPT_BO_FLAG_SPELL: u32 = 0x20000;
pub struct Fromto {
    pub ft_from: std::ffi::CString,
    pub ft_to: std::ffi::CString,
}
pub struct Salitem {
    pub sm_leadlen: i32,
    pub sm_rules: std::ffi::CString,
    pub sm_lead_w: Vec<i32>,
    pub sm_oneof_w: Vec<i32>,
    pub sm_to_w: Vec<i32>,
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct SlangId(pub Option<std::num::NonZeroU32>);

impl SlangId {
    pub const NULL: Self = Self(None);

    #[inline]
    pub const fn from_slot(idx: usize) -> Self {
        Self(std::num::NonZeroU32::new((idx as u32) + 1))
    }

    #[inline]
    pub const fn is_null(self) -> bool {
        self.0.is_none()
    }

    #[inline]
    pub fn opt_rc(self) -> Option<std::rc::Rc<Slang>> {
        let idx = (self.0?.get() - 1) as usize;
        with_spell_ext(|ext| ext.first_lang.borrow().get(idx).and_then(|s| s.clone()))
    }

    #[inline]
    pub fn rc(self) -> std::rc::Rc<Slang> {
        self.opt_rc().expect("null SlangId")
    }
}

impl Drop for Slang {
    fn drop(&mut self) {
        if crate::globals::g_st().exiting.get() {
            return;
        }
        crate::spell::slang_clear(self);
    }
}
pub struct Slang {
    pub sl_name: Option<std::ffi::CString>,
    pub sl_fname: Option<std::ffi::CString>,
    pub sl_add: bool,
    pub sl_fbyts: Vec<u8>,
    pub sl_fidxs: Vec<Idx>,
    pub sl_kbyts: Vec<u8>,
    pub sl_kidxs: Vec<Idx>,
    pub sl_pbyts: Vec<u8>,
    pub sl_pidxs: Vec<Idx>,
    pub sl_info: Option<std::ffi::CString>,
    pub sl_regions: [u8; MAXREGIONS * 2 + 1],
    pub sl_midword: Option<std::ffi::CString>,
    pub sl_wordcount: std::cell::RefCell<crate::memory::FxHashMap<Vec<u8>, u16>>,
    pub sl_compmax: i32,
    pub sl_compminlen: i32,
    pub sl_compsylmax: i32,
    pub sl_compoptions: i32,
    pub sl_comppat: Vec<std::ffi::CString>,
    pub sl_compprog: Option<Box<crate::regexp::Regprog>>,
    pub sl_comprules: Option<std::ffi::CString>,
    pub sl_compstartflags: Option<std::ffi::CString>,
    pub sl_compallflags: Option<std::ffi::CString>,
    pub sl_nobreak: std::cell::Cell<bool>,
    pub sl_syllable: Option<std::ffi::CString>,
    pub sl_syl_items: Vec<crate::spell::SylItem>,
    pub sl_prefprog: Vec<Option<Box<crate::regexp::Regprog>>>,
    pub sl_rep: Vec<Fromto>,
    pub sl_rep_first: [i16; 256],
    pub sl_sal: Vec<Salitem>,
    pub sl_sofo_sal: Vec<Vec<(i32, i32)>>,
    pub sl_sal_first: [Salfirst; 256],
    pub sl_followup: bool,
    pub sl_collapse: bool,
    pub sl_rem_accents: bool,
    pub sl_sofo: bool,
    pub sl_repsal: Vec<Fromto>,
    pub sl_repsal_first: [i16; 256],
    pub sl_nosplitsugs: bool,
    pub sl_nocompoundsugs: bool,
    pub sl_sugtime: i64,
    pub sl_sbyts: Vec<u8>,
    pub sl_sidxs: Vec<Idx>,
    pub sl_sugbuf: std::cell::RefCell<Option<std::rc::Rc<FileBuf>>>,
    pub sl_sugloaded: bool,
    pub sl_has_map: bool,
    pub sl_map_hash: crate::memory::FxHashMap<i32, i32>,
    pub sl_map_array: [i32; 256],
    pub sl_sounddone: std::cell::RefCell<crate::memory::FxHashMap<Vec<u8>, i16>>,
}

impl Slang {
    #[inline]
    pub fn has_sal(&self) -> bool {
        !self.sl_sal.is_empty() || !self.sl_sofo_sal.is_empty()
    }
}

#[derive(Clone, Copy)]
pub struct Langp {
    pub lp_slang: SlangId,
    pub lp_sallang: SlangId,
    pub lp_replang: SlangId,
    pub lp_region: i32,
}

impl Langp {
    pub const fn new() -> Self {
        Self {
            lp_slang: SlangId::NULL,
            lp_sallang: SlangId::NULL,
            lp_replang: SlangId::NULL,
            lp_region: 0,
        }
    }

    #[inline]
    pub fn slang(&self) -> std::rc::Rc<Slang> {
        self.lp_slang.rc()
    }
}

impl Default for Langp {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Copy)]
pub struct Spelltab {
    pub st_isw: [bool; 256],
    pub st_isu: [bool; 256],
    pub st_fold: [u8; 256],
    pub st_upper: [u8; 256],
}

impl Spelltab {
    pub const fn new() -> Self {
        Self {
            st_isw: [false; 256],
            st_isu: [false; 256],
            st_fold: [0; 256],
            st_upper: [0; 256],
        }
    }
}

impl Default for Spelltab {
    fn default() -> Self {
        Self::new()
    }
}
struct Suginfo {
    su_ga: Vec<Suggest>,
    su_maxcount: i32,
    su_maxscore: i32,
    su_sfmaxscore: i32,
    su_sga: Vec<Suggest>,
    su_badptr: std::ffi::CString,
    su_badlen: i32,
    su_badflags: i32,
    su_badword: [u8; MAXWLEN],
    su_fbadword: [u8; MAXWLEN],
    su_sal_badword: [u8; MAXWLEN],
    su_banned: std::collections::HashSet<Vec<u8>>,
    su_sallang: SlangId,
}

impl Default for Suginfo {
    fn default() -> Self {
        Self {
            su_ga: Vec::new(),
            su_maxcount: 0,
            su_maxscore: 0,
            su_sfmaxscore: 0,
            su_sga: Vec::new(),
            su_badptr: std::ffi::CString::default(),
            su_badlen: 0,
            su_badflags: 0,
            su_badword: [0; MAXWLEN],
            su_fbadword: [0; MAXWLEN],
            su_sal_badword: [0; MAXWLEN],
            su_banned: std::collections::HashSet::new(),
            su_sallang: SlangId::NULL,
        }
    }
}
struct Suggest {
    st_word: std::ffi::CString,
    st_wordlen: i32,
    st_orglen: i32,
    st_score: i32,
    st_altscore: i32,
    st_salscore: bool,
    st_had_bonus: bool,
    st_slang: SlangId,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SugState {
    StateStart = 0,
    StateNoprefix,
    StateSplitundo,
    StateEndnul,
    StatePlain,
    StateDel,
    StateInsPrep,
    StateIns,
    StateSwap,
    StateUnswap,
    StateSwap3,
    StateUnswap3,
    StateUnrot3l,
    StateUnrot3r,
    StateRepIni,
    StateRep,
    StateRepUndo,
    StateFinal,
}
#[derive(Clone, Copy)]
struct Trystate {
    ts_state: SugState,
    ts_score: i32,
    ts_arridx: Idx,
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
impl Default for Trystate {
    fn default() -> Self {
        Self {
            ts_state: SugState::StateStart,
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
#[derive(Clone, Copy, Default)]
struct Limitscore {
    badi: i32,
    goodi: i32,
    score: i32,
}
struct SpellSuggestState {
    spell_suggest_timeout: std::cell::Cell<i64>,
    sps_flags: std::cell::Cell<i32>,
    sps_limit: std::cell::Cell<i32>,
    expr_busy: std::cell::Cell<bool>,
}

impl SpellSuggestState {
    const fn new() -> Self {
        Self {
            spell_suggest_timeout: std::cell::Cell::new(5000),
            sps_flags: std::cell::Cell::new(SPS_BEST),
            sps_limit: std::cell::Cell::new(9999),
            expr_busy: std::cell::Cell::new(false),
        }
    }
}

std::thread_local! {
    static SPS_STATE: SpellSuggestState = const { SpellSuggestState::new() };
}

#[inline]
fn sps_st<R>(f: impl FnOnce(&SpellSuggestState) -> R) -> R {
    SPS_STATE.with(f)
}
#[inline]
const fn rescore(word_score: i32, sound_score: i32) -> i32 {
    (3 * word_score + sound_score) / 4
}
#[inline]
const fn maxscore(word_score: i32, sound_score: i32) -> i32 {
    (4 * word_score - sound_score) / 3
}
#[inline]
fn sug_clean_count(su: &Suginfo) -> i32 {
    if su.su_maxcount < 130 {
        150
    } else {
        su.su_maxcount + 20
    }
}
#[inline]
fn sug_max_count(su: &Suginfo) -> i32 {
    sug_clean_count(su) + 50
}

#[inline]
fn mb_cptr_back(s: &[u8], p: &mut usize) {
    crate::mbyte::mb_idx_back(s, p);
}

#[inline]
fn was_banned(su: &mut Suginfo, word: &[u8]) -> bool {
    su.su_banned.contains(crate::memory::bytes_until_nul(word))
}

fn can_be_compound(sp: &Trystate, slang: &Slang, compflags: &mut [u8], flag: i32) -> bool {
    let flags_cstr = if sp.ts_complen == sp.ts_compsplit {
        &slang.sl_compstartflags
    } else {
        &slang.sl_compallflags
    };
    if !byte_in_str(flags_cstr.as_deref(), flag) {
        return false;
    }
    if slang.sl_comprules.is_some() && sp.ts_complen > sp.ts_compsplit {
        let complen = sp.ts_complen as usize;
        compflags[complen] = flag as u8;
        compflags[complen + 1] = 0;
        let v = match_compoundrule(slang, &compflags[sp.ts_compsplit as usize..]);
        compflags[complen] = 0;
        return v;
    }
    true
}
fn score_wordcount_adj(slang: &Slang, score: i32, word: &[u8], split: bool) -> i32 {
    let Some(&wc_count) = slang
        .sl_wordcount
        .borrow()
        .get(crate::memory::bytes_until_nul(word))
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
fn badword_captype(word: &[u8]) -> i32 {
    let word = crate::memory::bytes_until_nul(word);
    let mut flags = captype(word);
    if (flags & WF_KEEPCAP) == 0 {
        return flags;
    }
    let mut l: i32 = 0;
    let mut u: i32 = 0;
    let mut first = false;
    let mut rem = word;
    while !rem.is_empty() {
        let is_first = rem.len() == word.len();
        let c = mb_cptr2char_adv(&mut rem);
        if spell_isupper(c) {
            u += 1;
            if is_first {
                first = true;
            }
        } else {
            l += 1;
        }
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
fn bytes2offset(pp: &mut &[u8]) -> i32 {
    let p = *pp;
    let c = p[0] as i32;
    let (nr, adv) = if (c & 0x80) == 0x00 {
        (c - 1, 1)
    } else if (c & 0xc0) == 0x80 {
        let mut n = (c & 0x3f) - 1;
        n = n * 255 + (p[1] as i32 - 1);
        (n, 2)
    } else if (c & 0xe0) == 0xc0 {
        let mut n = (c & 0x1f) - 1;
        n = n * 255 + (p[1] as i32 - 1);
        n = n * 255 + (p[2] as i32 - 1);
        (n, 3)
    } else {
        let mut n = (c & 0x0f) - 1;
        n = n * 255 + (p[1] as i32 - 1);
        n = n * 255 + (p[2] as i32 - 1);
        n = n * 255 + (p[3] as i32 - 1);
        (n, 4)
    };
    *pp = &p[adv..];
    nr
}
/// Parse 'spellsuggest' without changing suggestion state.
pub fn spell_check_sps(value: &CStr, ret_flags: &mut i32, ret_limit: &mut i32) -> bool {
    let mut flags: i32 = 0;
    let mut limit: i32 = 9999;
    let mut p = value.to_bytes();
    while let Some((key, val)) = option_next_keyval(&mut p) {
        let mut f: i32 = 0;
        if let Some(val) = val {
            if key == b"expr" || key == b"file" {
                // consumed later in spell_find_suggest()
            } else if key == b"timeout" {
                let v = val.strip_prefix(b"-").unwrap_or(val);
                if !v.first().is_some_and(u8::is_ascii_digit) {
                    f = -1;
                }
            } else {
                f = -1;
            }
        } else if key.first().is_some_and(u8::is_ascii_digit) {
            let mut rem = key;
            limit = getdigits_int(&mut rem, true, 0);
            if !rem.is_empty() {
                f = -1;
            }
        } else if key == b"best" {
            f = SPS_BEST;
        } else if key == b"fast" {
            f = SPS_FAST;
        } else if key == b"double" {
            f = SPS_DOUBLE;
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
/// Apply validated 'spellsuggest' flags and suggestion limit.
pub fn spell_apply_sps(flags: i32, limit: i32) {
    sps_st(|st| {
        st.sps_flags.set(flags);
        st.sps_limit.set(limit);
    });
}
fn select_spell_suggestion(sug: &mut Suginfo) {
    let mut items_tv = TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    };
    tv_list_alloc_ret(&mut items_tv, sug.su_ga.len() as isize);
    for stp in &sug.su_ga {
        let d = tv_dict_alloc();
        let el = sug.su_badlen - stp.st_orglen;
        let badptr_bytes = sug.su_badptr.as_bytes();
        if el > 0 {
            let mut word_vec: Vec<u8> = Vec::with_capacity((stp.st_wordlen + el + 1) as usize);
            word_vec.extend_from_slice(&stp.st_word.as_bytes()[..stp.st_wordlen as usize]);
            let org_off = (stp.st_orglen as usize).min(badptr_bytes.len());
            let end_off = (org_off + el as usize).min(badptr_bytes.len());
            word_vec.extend_from_slice(&badptr_bytes[org_off..end_off]);
            word_vec.push(0);
            tv_dict_add_str(
                &d,
                c"word",
                Some(CStr::from_bytes_until_nul(&word_vec).unwrap_or_default()),
            );
        } else {
            tv_dict_add_str(&d, c"word", Some(stp.st_word.as_c_str()));
        }
        if sug.su_badlen < stp.st_orglen {
            let org_len = (stp.st_orglen as usize).min(badptr_bytes.len());
            tv_dict_add_str_len(&d, c"extra", Some(&badptr_bytes[..org_len]));
        }
        tv_dict_add_nr(&d, c"score", stp.st_score as i64);
        if (sps_st(|st| st.sps_flags.get()) & (SPS_DOUBLE | SPS_BEST)) != 0 {
            tv_dict_add_nr(&d, c"altscore", stp.st_altscore as i64);
            tv_dict_add_bool(
                &d,
                c"salscore",
                if stp.st_salscore {
                    K_BOOL_VAR_TRUE_CONST
                } else {
                    K_BOOL_VAR_FALSE_CONST
                },
            );
        }
        let item = TypVal {
            v_type: VarType::VarDict,
            v_lock: 0,
            vval: TypvalVval::Dict(d),
        };
        tv_list_append_tv(items_tv.vval.v_list_ref(), &item);
    }
    let badptr_bytes = sug.su_badptr.as_bytes();
    let bad_cstr = crate::memory::bytes_to_cstring(
        &badptr_bytes[..(sug.su_badlen as usize).min(badptr_bytes.len())],
    );
    let mut bad_tv = TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    };
    tv_set_string(&mut bad_tv, Some(bad_cstr.as_c_str()));
    let mut lua_args = [
        items_tv,
        bad_tv,
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
    ];
    nlua_call_typval(c"vim._core.spell", c"select_suggest", &mut lua_args, None);
    tv_clear(&mut lua_args[0]);
    tv_clear(&mut lua_args[1]);
}
/// "z=": Find badly spelled word under or after the cursor.
pub fn spell_suggest(count: i32) {
    let mut badlen: i32 = 0;
    let msg_scroll_save = g_st().msg_scroll.get();
    let wo_spell_save = curwin().w_onebuf_opt.wo_spell.get();
    if !curwin().w_onebuf_opt.wo_spell.get() {
        parse_spelllang(&curwin());
        curwin().w_onebuf_opt.wo_spell.set(true);
    }
    check_cursor(&curwin());
    let prev_cursor = curwin().w_cursor.get();
    'skip: {
        let w_s = curwin().w_s_ptr();
        if w_s.b_p_spl.is_empty() {
            emsg(gettext(E_NO_SPELL));
            break 'skip;
        }
        if g_st().visual_global.borrow().active {
            if curwin().w_cursor.get().lnum != g_st().visual_global.borrow().start.lnum {
                vim_beep(K_OPT_BO_FLAG_SPELL);
                break 'skip;
            }
            badlen = curwin().w_cursor.get().col - g_st().visual_global.borrow().start.col;
            if badlen < 0 {
                badlen = -badlen;
            } else {
                curwin()
                    .w_cursor
                    .set_col(g_st().visual_global.borrow().start.col);
            }
            badlen += 1;
            end_visual_mode();
            badlen = badlen.min(get_cursor_line_len() - curwin().w_cursor.get().col);
        } else if spell_move_to(&curwin(), FORWARD, SMT_ALL, true, None) == 0
            || curwin().w_cursor.get().col > prev_cursor.col
        {
            curwin().w_cursor.set(prev_cursor);
            let curline_bytes = get_cursor_line_slice();
            let mut p_idx = (curwin().w_cursor.get().col as usize).min(curline_bytes.len());
            while p_idx > 0 && spell_iswordp_nmw(&curline_bytes[p_idx..], &curwin()) {
                crate::mbyte::mb_idx_back(&curline_bytes, &mut p_idx);
            }
            while p_idx < curline_bytes.len()
                && !spell_iswordp_nmw(&curline_bytes[p_idx..], &curwin())
            {
                p_idx += crate::mbyte::utfc_slice2len(&curline_bytes[p_idx..]);
            }
            if !spell_iswordp_nmw(&curline_bytes[p_idx..], &curwin()) {
                beep_flush();
                break 'skip;
            }
            curwin().w_cursor.set_col(p_idx as Colnr);
        }
        let need_cap = check_need_cap(
            &curwin(),
            curwin().w_cursor.get().lnum,
            curwin().w_cursor.get().col,
        );
        let mut line_vec = get_cursor_line_slice().to_vec();
        let line_len = line_vec.len();
        line_vec.push(0);
        let line_cstr = crate::memory::bytes_as_cstr(&line_vec);
        let limit = sps_st(|st| {
            st.spell_suggest_timeout.set(5000);
            st.sps_limit.get().min(g_st().rows_global.get() - 2)
        });
        let mut sug = Suginfo::default();
        let cursor_col = (curwin().w_cursor.get().col as usize).min(line_len);
        spell_find_suggest(
            &line_cstr[cursor_col..],
            badlen,
            &mut sug,
            limit,
            true,
            need_cap,
            true,
        );
        let selected = count;
        let sug_len = sug.su_ga.len() as i32;
        if sug_len <= 0 {
            msg_ext_set_kind(c"wmsg");
            msg(gettext(c"No suggestions"), 0);
        } else if count > 0 {
            if count > sug_len {
                msg_ext_set_kind(c"wmsg");
                smsg!(0, gettext(c"Only %lld suggestions"), sug_len as i64,);
            }
        } else {
            curwin().w_onebuf_opt.wo_spell.set(wo_spell_save);
            select_spell_suggestion(&mut sug);
            g_st().lines_left.set(g_st().rows_global.get());
            g_st().msg_scroll.set(msg_scroll_save);
            return;
        }
        if selected > 0 && selected <= sug_len && u_save_cursor() {
            let stp = &sug.su_ga[(selected - 1) as usize];
            let badptr_bytes = sug.su_badptr.as_bytes();
            let org_off = (stp.st_orglen as usize).min(badptr_bytes.len());
            if sug.su_badlen > stp.st_orglen {
                let bad_off = (sug.su_badlen as usize).min(badptr_bytes.len());
                let from = Some(crate::memory::bytes_to_cstring(&badptr_bytes[..bad_off]));
                let tail_cstr = &sug.su_badptr.as_c_str()[org_off..];
                let mut iobuff = [0u8; IOSIZE];
                format_to_buf!(
                    &mut iobuff,
                    c"%s%.*s",
                    &stp.st_word,
                    sug.su_badlen - stp.st_orglen,
                    tail_cstr,
                );
                let to = Some(crate::memory::cstring_until_nul(&iobuff));
                with_spell_ext(|ext| {
                    *ext.repl_from.borrow_mut() = from;
                    *ext.repl_to.borrow_mut() = to;
                });
            } else {
                let from = Some(crate::memory::bytes_to_cstring(&badptr_bytes[..org_off]));
                let to = Some((*stp.st_word).to_owned());
                with_spell_ext(|ext| {
                    *ext.repl_from.borrow_mut() = from;
                    *ext.repl_to.borrow_mut() = to;
                });
            }
            let c = (line_len - badptr_bytes.len()) as i32;
            let mut p_vec: Vec<u8> =
                Vec::with_capacity(line_len - stp.st_orglen as usize + stp.st_wordlen as usize + 1);
            p_vec.extend_from_slice(&line_vec[..c as usize]);
            p_vec.extend_from_slice(stp.st_word.to_bytes());
            p_vec.extend_from_slice(sug.su_badptr.as_c_str()[org_off..].to_bytes_with_nul());
            prep_redo(
                false,
                false,
                CmdSpec {
                    op: b'c' as i32,
                    cmd: b'i' as i32,
                    cmd2: b'w' as i32,
                    ..CmdSpec::default()
                },
            );
            let rep_len = (stp.st_wordlen + sug.su_badlen - stp.st_orglen).max(0) as usize;
            redo_append_lit(&p_vec[c as usize..c as usize + rep_len]);
            redo_append_char(ESC);
            let _ = ml_replace(
                curwin().w_cursor.get().lnum,
                crate::memory::bytes_until_nul(&p_vec),
                true,
            );
            curwin().w_cursor.set_col(c);
            inserted_bytes(
                curwin().w_cursor.get().lnum,
                c,
                stp.st_orglen,
                stp.st_wordlen,
            );
        } else {
            curwin().w_cursor.set(prev_cursor);
        }
    }
    curwin().w_onebuf_opt.wo_spell.set(wo_spell_save);
}
/// Find spell suggestions for "word".
pub fn spell_suggest_list(
    word: &CStr,
    maxcount: i32,
    need_cap: bool,
    interactive: bool,
) -> Vec<CString> {
    let mut sug = Suginfo::default();
    spell_find_suggest(word, 0, &mut sug, maxcount, false, need_cap, interactive);
    let mut res = Vec::with_capacity(sug.su_ga.len());
    for stp in &sug.su_ga {
        let word_bytes = &stp.st_word.as_bytes()[..stp.st_wordlen as usize];
        let badptr_bytes = sug.su_badptr.as_bytes();
        let org_off = (stp.st_orglen as usize).min(badptr_bytes.len());
        let tail_bytes = &badptr_bytes[org_off..];
        let mut bytes = Vec::with_capacity(word_bytes.len() + tail_bytes.len());
        bytes.extend_from_slice(word_bytes);
        bytes.extend_from_slice(tail_bytes);
        res.push(CString::new(bytes).unwrap_or_default());
    }
    res
}
fn spell_find_suggest(
    bad_cstr: &CStr,
    badlen: i32,
    su: &mut Suginfo,
    maxcount: i32,
    banbadword: bool,
    need_cap: bool,
    interactive: bool,
) {
    let mut attr: Hlf = HLF_COUNT;
    let mut buf: [u8; MAXPATHL] = [0; MAXPATHL];
    let mut do_combine = false;
    let mut did_intern = false;
    *su = Suginfo::default();
    if bad_cstr.to_bytes().is_empty() {
        return;
    }
    su.su_badptr = bad_cstr.to_owned();
    if badlen != 0 {
        su.su_badlen = badlen;
    } else {
        let tmplen = spell_check(&curwin(), &su.su_badptr, &mut attr, None, false);
        su.su_badlen = tmplen as i32;
    }
    su.su_maxcount = maxcount;
    su.su_maxscore = SCORE_MAXINIT;
    su.su_badlen = su.su_badlen.min(MAXWLEN as i32 - 1);
    let bad_len = (su.su_badlen as usize).min(su.su_badptr.as_bytes().len());
    let bad_slice = &su.su_badptr.as_bytes()[..bad_len];
    su.su_badword[..bad_len].copy_from_slice(bad_slice);
    su.su_badword[bad_len] = 0;
    spell_casefold(&curwin(), bad_slice, &mut su.su_fbadword);
    su.su_fbadword[su.su_badlen as usize] = 0;
    su.su_badflags = badword_captype(bad_slice);
    if need_cap {
        su.su_badflags |= WF_ONECAP;
    }
    for lp in &*curbuf().b_s.b_langp.borrow() {
        if !lp.lp_sallang.is_null() {
            su.su_sallang = lp.lp_sallang;
            break;
        }
    }
    if !su.su_sallang.is_null() {
        spell_soundfold(
            &su.su_sallang.rc(),
            &su.su_fbadword,
            true,
            &mut su.su_sal_badword,
        );
    }
    let c = crate::mbyte::utf_slice2char(su.su_badptr.to_bytes());
    if !spell_isupper(c) && attr == HLF_COUNT {
        make_case_word(&su.su_badword, &mut buf, WF_ONECAP);
        add_suggestion(
            su,
            false,
            &buf,
            su.su_badlen,
            SCORE_ICASE,
            0,
            true,
            su.su_sallang,
            false,
        );
    }
    if banbadword {
        let bw = su.su_badword;
        add_banned(su, &bw);
    }
    let sps_copy = opt_vars().p_sps.to_owned();
    let mut p = sps_copy.as_bytes();
    while !p.is_empty() {
        copy_option_part(&mut p, &mut buf[..MAXPATHL], c",");
        let buf_cstr = crate::memory::bytes_as_cstr(&buf);
        let buf_bytes = buf_cstr.to_bytes();
        if buf_bytes.starts_with(b"expr:") {
            if !sps_st(|st| st.expr_busy.replace(true)) {
                spell_suggest_expr(su, crate::memory::bytes_as_cstr(&buf[5..]));
                sps_st(|st| st.expr_busy.set(false));
            }
        } else if buf_bytes.starts_with(b"file:") {
            spell_suggest_file(su, crate::memory::bytes_as_cstr(&buf[5..]));
        } else if let Some(rest) = buf_bytes.strip_prefix(b"timeout:") {
            let timeout = crate::charset::parse_i64_prefix(rest) as i32 as i64;
            sps_st(|st| st.spell_suggest_timeout.set(timeout));
        } else if !did_intern {
            spell_suggest_intern(su, interactive);
            if (sps_st(|st| st.sps_flags.get()) & SPS_DOUBLE) != 0 {
                do_combine = true;
            }
            did_intern = true;
        }
    }
    if do_combine {
        score_combine(su);
    }
}
fn spell_suggest_expr(su: &mut Suginfo, expr: &CStr) {
    let mut p: Option<std::borrow::Cow<'_, CStr>> = None;
    let list_opt = eval_spell_expr(crate::memory::bytes_as_cstr(&su.su_badword), expr);
    if let Some(list) = list_opt {
        for li in crate::eval::typval::list_iter_ptr(list.first_opt()) {
            let li_tv = li.tv();
            if li_tv.v_type == VarType::VarList
                && let Some(sublist) = li_tv.vval.v_list_opt()
            {
                let score = get_spellword(sublist, &mut p);
                if score >= 0 && score <= su.su_maxscore {
                    add_suggestion(
                        su,
                        false,
                        p.as_deref().map_or(&[][..], CStr::to_bytes),
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
        tv_list_unref(Some(&list));
    }
    check_suggestions(su, false);
    cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
}
fn spell_suggest_file(su: &mut Suginfo, fname: &CStr) {
    let mut line: [u8; MAXWLEN * 2] = [0; MAXWLEN * 2];
    let mut cword: [u8; MAXWLEN] = [0; MAXWLEN];
    let Some(fd) = CFile::open(fname, c"r") else {
        semsg!(gettext(E_NOTOPEN), fname);
        return;
    };
    while !vim_fgets(&mut line, &fd) && !g_st().got_int.get() {
        line_breakcheck();
        let Some(slash_pos) = crate::memory::bytes_as_cstr(&line)
            .to_bytes()
            .iter()
            .position(|&b| b == b'/')
        else {
            continue;
        };
        line[slash_pos] = 0;
        if mb_stricmp(
            crate::memory::bytes_as_cstr(&su.su_badword),
            crate::memory::bytes_as_cstr(&line),
        ) == 0
        {
            let len = crate::memory::bytes_as_cstr(&line[slash_pos + 1..])
                .to_bytes()
                .iter()
                .take_while(|&&b| b >= b' ')
                .count();
            line[slash_pos + 1 + len] = 0;
            let p_slice = &line[slash_pos + 1..slash_pos + 1 + len];
            let sug_word: &[u8] = if captype(p_slice) == 0 {
                make_case_word(p_slice, &mut cword, su.su_badflags);
                &cword
            } else {
                p_slice
            };
            add_suggestion(
                su,
                false,
                sug_word,
                su.su_badlen,
                SCORE_FILE,
                0,
                true,
                su.su_sallang,
                false,
            );
        }
    }
    drop(fd);
    check_suggestions(su, false);
    cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
}
fn spell_suggest_intern(su: &mut Suginfo, interactive: bool) {
    suggest_load_files();
    suggest_try_special(su);
    suggest_try_change(su);
    let flags = sps_st(|st| st.sps_flags.get());
    if (flags & SPS_DOUBLE) != 0 {
        score_comp_sal(su);
    }
    if (flags & SPS_FAST) == 0 {
        if (flags & SPS_BEST) != 0 {
            rescore_suggestions(su);
        }
        suggest_try_soundalike_prep();
        su.su_maxscore = SCORE_SFMAX1;
        su.su_sfmaxscore = SCORE_MAXINIT * 3;
        suggest_try_soundalike(su);
        if (su.su_ga.len() as i32) < sug_clean_count(su) {
            su.su_maxscore = SCORE_SFMAX2;
            suggest_try_soundalike(su);
            if (su.su_ga.len() as i32) < sug_clean_count(su) {
                su.su_maxscore = SCORE_SFMAX3;
                suggest_try_soundalike(su);
            }
        }
        su.su_maxscore = su.su_sfmaxscore;
        suggest_try_soundalike_finish();
    }
    os_breakcheck();
    if interactive && g_st().got_int.get() {
        let _ = vgetc();
        g_st().got_int.set(false);
    }
    let flags = sps_st(|st| st.sps_flags.get());
    if (flags & SPS_DOUBLE) == 0 && !su.su_ga.is_empty() {
        if (flags & SPS_BEST) != 0 {
            rescore_suggestions(su);
        }
        check_suggestions(su, false);
        cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
    }
}
fn suggest_try_special(su: &mut Suginfo) {
    let mut word: [u8; MAXWLEN] = [0; MAXWLEN];
    let fbadword_bytes = crate::memory::bytes_until_nul(&su.su_fbadword);
    let len = crate::charset::skiptowhite_len(fbadword_bytes);
    let rest = crate::charset::skipwhite_slice(&fbadword_bytes[len..]);
    if rest == &fbadword_bytes[..len] {
        make_case_word(&su.su_fbadword[..len], &mut word, su.su_badflags);
        add_suggestion(
            su,
            false,
            &word,
            su.su_badlen,
            rescore(SCORE_REP, 0),
            0,
            true,
            su.su_sallang,
            false,
        );
    }
}
fn suggest_try_change(su: &mut Suginfo) {
    let mut fword: [u8; MAXWLEN] = su.su_fbadword;
    let mut n = crate::memory::bytes_until_nul(&fword).len() as i32;
    let badptr_bytes = su.su_badptr.as_bytes();
    let badlen_off = (su.su_badlen as usize).min(badptr_bytes.len());
    spell_casefold(
        &curwin(),
        &badptr_bytes[badlen_off..],
        &mut fword[n as usize..],
    );
    n = su.su_badptr.to_bytes().len() as i32;
    if (n as usize) < MAXWLEN {
        fword[n as usize] = 0;
    }
    let w_s = curwin().w_s_ptr();
    let langps = w_s.b_langp.borrow().clone();
    for lp in &langps {
        if lp.slang().sl_fbyts.is_empty() {
            continue;
        }
        suggest_trie_walk(su, lp, &mut fword, false);
    }
}
#[inline]
fn try_deeper(su: &Suginfo, stack: &[Trystate; MAXWLEN], depth: usize, add: i32) -> bool {
    depth < MAXWLEN - 1 && stack[depth].ts_score + add < su.su_maxscore
}
fn suggest_trie_walk(su: &mut Suginfo, lp: &Langp, fword_buf: &mut [u8; MAXWLEN], soundfold: bool) {
    let mut tword: [u8; MAXWLEN] = [0; MAXWLEN];
    let mut stack: [Trystate; MAXWLEN] = [Trystate::default(); MAXWLEN];
    let mut preword: [u8; MAXWLEN * 3] = [0; MAXWLEN * 3];
    let mut compflags: [u8; MAXWLEN] = [0; MAXWLEN];
    let mut byts: &[u8];
    let fbyts: &[u8];
    let pbyts: &[u8];
    let mut idxs: &[Idx];
    let fidxs: &[Idx];
    let pidxs: &[Idx];
    let mut c: i32;
    let mut c2: i32;
    let mut c3: i32;
    let mut n: i32;
    let mut arridx: Idx;
    let mut fl: i32;
    let mut tl: i32;
    let mut repextra: i32 = 0;
    let slang_rc = lp.lp_slang.rc();
    let slang = &*slang_rc;
    let replang_rc = lp.lp_replang.opt_rc();
    let mut goodword_ends: bool;
    let mut breakcheckcount: i32 = 1000;
    let mut depth: isize = 0;
    stack[0] = Trystate::default();
    stack[0].ts_curi = 1;
    if soundfold {
        fbyts = &slang.sl_sbyts;
        byts = fbyts;
        fidxs = &slang.sl_sidxs;
        idxs = fidxs;
        pbyts = &[];
        pidxs = &[];
        stack[0].ts_prefixdepth = PFD_NOPREFIX;
        stack[0].ts_state = SugState::StateStart;
    } else {
        fbyts = &slang.sl_fbyts;
        fidxs = &slang.sl_fidxs;
        if !slang.sl_pbyts.is_empty() {
            pbyts = &slang.sl_pbyts;
            pidxs = &slang.sl_pidxs;
            byts = pbyts;
            idxs = pidxs;
            stack[0].ts_prefixdepth = PFD_PREFIXTREE;
            stack[0].ts_state = SugState::StateNoprefix;
        } else {
            pbyts = &[];
            pidxs = &[];
            byts = fbyts;
            idxs = fidxs;
            stack[0].ts_prefixdepth = PFD_NOPREFIX;
            stack[0].ts_state = SugState::StateStart;
        }
    }
    let mut time_limit: Proftime = 0;
    let timeout = sps_st(|st| st.spell_suggest_timeout.get());
    if timeout > 0 {
        time_limit = profile_setlimit(timeout);
    }
    while depth >= 0 && !g_st().got_int.get() {
        let mut udepth = depth as usize;
        let state = stack[udepth].ts_state;
        match state {
            SugState::StateStart | SugState::StateNoprefix => {
                arridx = stack[udepth].ts_arridx;
                let mut len = i32::from(byts[arridx as usize]);
                arridx += stack[udepth].ts_curi as Idx;
                if stack[udepth].ts_prefixdepth == PFD_PREFIXTREE {
                    n = 0;
                    while n < len && byts[(arridx + n) as usize] == 0 {
                        n += 1;
                    }
                    stack[udepth].ts_curi += n as i16;
                    let was_noprefix = stack[udepth].ts_state == SugState::StateNoprefix;
                    stack[udepth].ts_state = SugState::StateEndnul;
                    stack[udepth].ts_save_badflags = su.su_badflags as u8;
                    if udepth < MAXWLEN - 1 && (byts[arridx as usize] == 0 || was_noprefix) {
                        n = nofold_len(
                            &*fword_buf,
                            stack[udepth].ts_fidx as i32,
                            su.su_badptr.to_bytes(),
                        );
                        let badptr_bytes = su.su_badptr.as_bytes();
                        let bad_slice =
                            &badptr_bytes[..(su.su_badlen.max(0) as usize).min(badptr_bytes.len())];
                        let n_usize = (n.max(0) as usize).min(bad_slice.len());
                        let flags = badword_captype(&bad_slice[..n_usize]);
                        su.su_badflags = badword_captype(&bad_slice[n_usize..]);
                        go_deeper(&mut stack, udepth, 0);
                        depth += 1;
                        udepth = depth as usize;
                        stack[udepth].ts_prefixdepth = (udepth - 1) as u8;
                        byts = fbyts;
                        idxs = fidxs;
                        stack[udepth].ts_arridx = 0;
                        tword[stack[udepth].ts_twordlen as usize] = 0;
                        make_case_word(
                            &tword[stack[udepth].ts_splitoff as usize..],
                            &mut preword[stack[udepth].ts_prewordlen as usize..],
                            flags,
                        );
                        stack[udepth].ts_prewordlen =
                            crate::memory::bytes_until_nul(&preword).len() as u8;
                        stack[udepth].ts_splitoff = stack[udepth].ts_twordlen;
                    }
                    continue;
                }
                if stack[udepth].ts_curi as i32 > len || byts[arridx as usize] != 0 {
                    stack[udepth].ts_state = SugState::StateEndnul;
                    stack[udepth].ts_save_badflags = su.su_badflags as u8;
                    continue;
                }
                stack[udepth].ts_curi += 1;
                let mut flags = idxs[arridx as usize];
                if (flags & WF_NOSUGGEST) != 0 {
                    continue;
                }
                let fbyte = fword_buf[stack[udepth].ts_fidx as usize];
                let fword_ends = fbyte == 0
                    || if soundfold {
                        ascii_iswhite(i32::from(fbyte))
                    } else {
                        !spell_iswordp(&fword_buf[stack[udepth].ts_fidx as usize..], &curwin())
                    };
                tword[stack[udepth].ts_twordlen as usize] = 0;
                if stack[udepth].ts_prefixdepth <= PFD_NOTSPECIAL
                    && (stack[udepth].ts_flags & TSF_PREFIXOK) == 0
                    && !pbyts.is_empty()
                {
                    n = stack[stack[udepth].ts_prefixdepth as usize].ts_arridx;
                    len = i32::from(pbyts[n as usize]);
                    n += 1;
                    c = 0;
                    while c < len && pbyts[(n + c) as usize] == 0 {
                        c += 1;
                    }
                    if c > 0 {
                        c = valid_word_prefix(
                            c,
                            n,
                            flags,
                            crate::memory::bytes_as_cstr(
                                &tword[stack[udepth].ts_splitoff as usize..],
                            ),
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
                let mut p_off: Option<usize> = None;
                let mut compound_ok = true;
                if stack[udepth].ts_complen > stack[udepth].ts_compsplit {
                    if slang.sl_nobreak.get() {
                        let split_len =
                            (stack[udepth].ts_fidx - stack[udepth].ts_splitfidx) as usize;
                        let sf_off = stack[udepth].ts_splitfidx as usize;
                        let so_off = stack[udepth].ts_splitoff as usize;
                        if stack[udepth].ts_fidx - stack[udepth].ts_splitfidx
                            == stack[udepth].ts_twordlen - stack[udepth].ts_splitoff
                            && fword_buf[sf_off..sf_off + split_len]
                                == tword[so_off..so_off + split_len]
                        {
                            preword[stack[udepth].ts_prewordlen as usize] = 0;
                            let newscore = score_wordcount_adj(
                                slang,
                                stack[udepth].ts_score,
                                &preword[stack[udepth].ts_prewordlen as usize..],
                                stack[udepth].ts_prewordlen > 0,
                            );
                            if newscore <= su.su_maxscore {
                                add_suggestion(
                                    su,
                                    false,
                                    &preword,
                                    stack[udepth].ts_splitfidx as i32 - repextra,
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
                            || i32::from(stack[udepth].ts_twordlen - stack[udepth].ts_splitoff)
                                < slang.sl_compminlen
                        {
                            continue;
                        }
                        if slang.sl_compminlen > 0
                            && mb_charlen(
                                crate::memory::bytes_as_cstr(
                                    &tword[stack[udepth].ts_splitoff as usize..],
                                )
                                .to_bytes(),
                            ) < slang.sl_compminlen
                        {
                            continue;
                        }
                        compflags[stack[udepth].ts_complen as usize] = ((flags as u32) >> 24) as u8;
                        compflags[stack[udepth].ts_complen as usize + 1] = 0;
                        let pw_off = stack[udepth].ts_prewordlen as usize;
                        let tw_off = stack[udepth].ts_splitoff as usize;
                        let tw_len =
                            (stack[udepth].ts_twordlen - stack[udepth].ts_splitoff) as usize;
                        preword[pw_off..pw_off + tw_len]
                            .copy_from_slice(&tword[tw_off..tw_off + tw_len]);
                        preword[pw_off + tw_len] = 0;
                        if match_checkcompoundpattern(
                            &preword,
                            stack[udepth].ts_prewordlen as i32,
                            &slang.sl_comppat,
                        ) {
                            compound_ok = false;
                        }
                        if compound_ok {
                            let mut p_slice = crate::memory::bytes_until_nul(&preword);
                            while !crate::charset::skiptowhite_slice(p_slice).is_empty() {
                                p_slice = crate::charset::skipwhite_slice(
                                    crate::charset::skiptowhite_slice(p_slice),
                                );
                            }
                            if fword_ends
                                && !can_compound(
                                    slang,
                                    p_slice,
                                    &compflags[stack[udepth].ts_compsplit as usize..],
                                )
                            {
                                compound_ok = false;
                            }
                        }
                        let mut p_idx = stack[udepth].ts_prewordlen as usize;
                        mb_cptr_back(&preword, &mut p_idx);
                        p_off = Some(p_idx);
                    }
                }
                if soundfold {
                    let pw_off = stack[udepth].ts_prewordlen as usize;
                    let tw_off = stack[udepth].ts_splitoff as usize;
                    let tw_bytes =
                        crate::memory::bytes_as_cstr(&tword[tw_off..]).to_bytes_with_nul();
                    preword[pw_off..pw_off + tw_bytes.len()].copy_from_slice(tw_bytes);
                } else if (flags & WF_KEEPCAP) != 0 {
                    find_keepcap_word(
                        slang,
                        &tword[stack[udepth].ts_splitoff as usize..],
                        &mut preword[stack[udepth].ts_prewordlen as usize..],
                    );
                } else {
                    c = su.su_badflags;
                    if (c & WF_ALLCAP) != 0
                        && su.su_badlen
                            == crate::mbyte::utfc_slice2len(su.su_badptr.to_bytes()) as i32
                    {
                        c = WF_ONECAP;
                    }
                    c |= flags;
                    if let Some(off) = p_off
                        && spell_iswordp_nmw(
                            crate::memory::bytes_until_nul(&preword[off..]),
                            &curwin(),
                        )
                    {
                        c &= !WF_ONECAP;
                    }
                    make_case_word(
                        &tword[stack[udepth].ts_splitoff as usize..],
                        &mut preword[stack[udepth].ts_prewordlen as usize..],
                        c,
                    );
                }
                if !soundfold {
                    if (flags & WF_BANNED) != 0 {
                        add_banned(su, &preword[stack[udepth].ts_prewordlen as usize..]);
                        continue;
                    }
                    if (stack[udepth].ts_complen == stack[udepth].ts_compsplit
                        && was_banned(su, &preword[stack[udepth].ts_prewordlen as usize..]))
                        || was_banned(su, &preword)
                    {
                        if slang.sl_compprog.is_none() {
                            continue;
                        }
                        goodword_ends = false;
                    }
                }
                let mut newscore: i32 = 0;
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
                        captype(&preword[stack[udepth].ts_prewordlen as usize..]),
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
                        add_sound_suggest(su, &preword, stack[udepth].ts_score, lp);
                    } else if stack[udepth].ts_fidx > 0 {
                        let mut p_idx = stack[udepth].ts_fidx as usize;
                        mb_cptr_back(&*fword_buf, &mut p_idx);
                        if !spell_iswordp(&fword_buf[p_idx..], &curwin()) && preword[0] != 0 {
                            let mut pw_idx = crate::memory::bytes_until_nul(&preword).len();
                            mb_cptr_back(&preword, &mut pw_idx);
                            if spell_iswordp(&preword[pw_idx..], &curwin()) {
                                newscore += SCORE_NONWORD;
                            }
                        }
                        let score = score_wordcount_adj(
                            slang,
                            stack[udepth].ts_score + newscore,
                            &preword[stack[udepth].ts_prewordlen as usize..],
                            stack[udepth].ts_prewordlen > 0,
                        );
                        if score <= su.su_maxscore {
                            add_suggestion(
                                su,
                                false,
                                &preword,
                                stack[udepth].ts_fidx as i32 - repextra,
                                score,
                                0,
                                false,
                                lp.lp_sallang,
                                false,
                            );
                            if (su.su_badflags & WF_MIXCAP) != 0 {
                                c = captype(&preword);
                                if c == 0 || c == WF_ALLCAP {
                                    make_case_word(
                                        &tword[stack[udepth].ts_splitoff as usize..],
                                        &mut preword[stack[udepth].ts_prewordlen as usize..],
                                        if c == 0 { WF_ALLCAP } else { 0 },
                                    );
                                    add_suggestion(
                                        su,
                                        false,
                                        &preword,
                                        stack[udepth].ts_fidx as i32 - repextra,
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
                    let try_split =
                        (stack[udepth].ts_fidx as i32 - repextra) < su.su_badlen && !soundfold;
                    let mut try_compound = false;
                    if !soundfold
                        && !slang.sl_nocompoundsugs
                        && slang.sl_compprog.is_some()
                        && ((flags as u32) >> 24) != 0
                        && (stack[udepth].ts_twordlen - stack[udepth].ts_splitoff) as i32
                            >= slang.sl_compminlen
                        && (slang.sl_compminlen == 0
                            || mb_charlen(
                                crate::memory::bytes_as_cstr(
                                    &tword[stack[udepth].ts_splitoff as usize..],
                                )
                                .to_bytes(),
                            ) >= slang.sl_compminlen)
                        && (slang.sl_compsylmax < MAXWLEN as i32
                            || i32::from(stack[udepth].ts_complen + 1 - stack[udepth].ts_compsplit)
                                < slang.sl_compmax)
                        && can_be_compound(
                            &stack[udepth],
                            slang,
                            &mut compflags,
                            ((flags as u32) >> 24) as i32,
                        )
                    {
                        try_compound = true;
                        compflags[stack[udepth].ts_complen as usize] = ((flags as u32) >> 24) as u8;
                        compflags[stack[udepth].ts_complen as usize + 1] = 0;
                    }
                    if slang.sl_nobreak.get() && !slang.sl_nocompoundsugs {
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
                            let mut p_slice = crate::memory::bytes_until_nul(&preword);
                            while !crate::charset::skiptowhite_slice(p_slice).is_empty() {
                                p_slice = crate::charset::skipwhite_slice(
                                    crate::charset::skiptowhite_slice(p_slice),
                                );
                            }
                            if stack[udepth].ts_complen > stack[udepth].ts_compsplit
                                && !can_compound(
                                    slang,
                                    p_slice,
                                    &compflags[stack[udepth].ts_compsplit as usize..],
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
                                &preword[stack[udepth].ts_prewordlen as usize..],
                                true,
                            );
                        }
                        if try_deeper(su, &stack, udepth, newscore) {
                            go_deeper(&mut stack, udepth, newscore);
                            stack[udepth].ts_save_badflags = su.su_badflags as u8;
                            stack[udepth].ts_state = SugState::StateSplitundo;
                            depth += 1;
                            udepth = depth as usize;
                            let mut preword_len = crate::memory::bytes_until_nul(&preword).len();
                            if !try_compound && !fword_ends {
                                preword[preword_len] = b' ';
                                preword_len += 1;
                                preword[preword_len] = 0;
                            }
                            stack[udepth].ts_prewordlen = preword_len as u8;
                            stack[udepth].ts_splitoff = stack[udepth].ts_twordlen;
                            stack[udepth].ts_splitfidx = stack[udepth].ts_fidx;
                            if ((!try_compound
                                && !spell_iswordp_nmw(
                                    &fword_buf[stack[udepth].ts_fidx as usize..],
                                    &curwin(),
                                ))
                                || fword_ends)
                                && fword_buf[stack[udepth].ts_fidx as usize] != 0
                                && goodword_ends
                            {
                                let fidx = stack[udepth].ts_fidx as usize;
                                let l = utfc_ptr2len(&fword_buf[fidx..]);
                                if fword_ends {
                                    let pw_off = stack[udepth].ts_prewordlen as usize;
                                    let l_usize = l as usize;
                                    preword[pw_off..pw_off + l_usize]
                                        .copy_from_slice(&fword_buf[fidx..fidx + l_usize]);
                                    stack[udepth].ts_prewordlen += l as u8;
                                    preword[stack[udepth].ts_prewordlen as usize] = 0;
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
                                &*fword_buf,
                                stack[udepth].ts_fidx as i32,
                                su.su_badptr.to_bytes(),
                            );
                            let badptr_bytes = su.su_badptr.as_bytes();
                            let bad_slice = &badptr_bytes
                                [..(su.su_badlen.max(0) as usize).min(badptr_bytes.len())];
                            let n_usize = (n.max(0) as usize).min(bad_slice.len());
                            su.su_badflags = badword_captype(&bad_slice[n_usize..]);
                            stack[udepth].ts_arridx = 0;
                            if !pbyts.is_empty() {
                                byts = pbyts;
                                idxs = pidxs;
                                stack[udepth].ts_prefixdepth = PFD_PREFIXTREE;
                                stack[udepth].ts_state = SugState::StateNoprefix;
                            }
                        }
                    }
                }
            }
            SugState::StateSplitundo => {
                su.su_badflags = stack[udepth].ts_save_badflags as i32;
                stack[udepth].ts_state = SugState::StateStart;
                byts = fbyts;
                idxs = fidxs;
            }
            SugState::StateEndnul | SugState::StatePlain => {
                let mut goto_del = false;
                if state == SugState::StateEndnul {
                    su.su_badflags = stack[udepth].ts_save_badflags as i32;
                    if fword_buf[stack[udepth].ts_fidx as usize] == 0
                        && stack[udepth].ts_tcharlen == 0
                    {
                        stack[udepth].ts_state = SugState::StateDel;
                        goto_del = true;
                    } else {
                        stack[udepth].ts_state = SugState::StatePlain;
                    }
                }
                if !goto_del {
                    arridx = stack[udepth].ts_arridx;
                    if stack[udepth].ts_curi as i32 > i32::from(byts[arridx as usize]) {
                        stack[udepth].ts_state =
                            if stack[udepth].ts_fidx >= stack[udepth].ts_fidxtry {
                                SugState::StateDel
                            } else {
                                SugState::StateFinal
                            };
                    } else {
                        arridx += stack[udepth].ts_curi as Idx;
                        stack[udepth].ts_curi += 1;
                        c = i32::from(byts[arridx as usize]);
                        let newscore = if c == i32::from(fword_buf[stack[udepth].ts_fidx as usize])
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
                                    || c != i32::from(
                                        fword_buf[stack[udepth].ts_delidx as usize],
                                    ))))
                            && try_deeper(su, &stack, udepth, newscore)
                        {
                            go_deeper(&mut stack, udepth, newscore);
                            depth += 1;
                            udepth = depth as usize;
                            if fword_buf[stack[udepth].ts_fidx as usize] != 0 {
                                stack[udepth].ts_fidx += 1;
                            }
                            let twl = stack[udepth].ts_twordlen as usize;
                            tword[twl] = c as u8;
                            stack[udepth].ts_twordlen += 1;
                            stack[udepth].ts_arridx = idxs[arridx as usize];
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
                                    let fw_off = stack[udepth].ts_fcharstart as usize;
                                    stack[udepth].ts_fidx = stack[udepth].ts_fcharstart
                                        + utfc_ptr2len(&fword_buf[fw_off..]) as u8;
                                    let tw_off = (stack[udepth].ts_twordlen
                                        - stack[udepth].ts_tcharlen)
                                        as usize;
                                    let tw_c = utf_ptr2char(&tword[tw_off..]);
                                    let fw_c = utf_ptr2char(&fword_buf[fw_off..]);
                                    if utf_iscomposing_legacy(tw_c) && utf_iscomposing_legacy(fw_c)
                                    {
                                        stack[udepth].ts_score -= SCORE_SUBST - SCORE_SUBCOMP;
                                    } else if !soundfold
                                        && slang.sl_has_map
                                        && similar_chars(slang, tw_c, fw_c)
                                    {
                                        stack[udepth].ts_score -= SCORE_SUBST - SCORE_SIMILAR;
                                    }
                                } else if stack[udepth].ts_isdiff == DIFF_INSERT
                                    && stack[udepth].ts_twordlen > stack[udepth].ts_tcharlen
                                {
                                    let mut p_idx = (stack[udepth].ts_twordlen
                                        - stack[udepth].ts_tcharlen)
                                        as usize;
                                    c = utf_ptr2char(&tword[p_idx..]);
                                    if utf_iscomposing_legacy(c) {
                                        stack[udepth].ts_score -= SCORE_INS - SCORE_INSCOMP;
                                    } else {
                                        mb_cptr_back(&tword, &mut p_idx);
                                        if c == utf_ptr2char(&tword[p_idx..]) {
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
            SugState::StateDel | SugState::StateInsPrep => {
                let mut do_ins_prep = true;
                if state == SugState::StateDel {
                    if stack[udepth].ts_tcharlen > 0 {
                        stack[udepth].ts_state = SugState::StateFinal;
                        do_ins_prep = false;
                    } else {
                        stack[udepth].ts_state = SugState::StateInsPrep;
                        stack[udepth].ts_curi = 1;
                        let newscore = if soundfold
                            && stack[udepth].ts_fidx == 0
                            && fword_buf[stack[udepth].ts_fidx as usize] == b'*'
                        {
                            2 * SCORE_DEL / 3
                        } else {
                            SCORE_DEL
                        };
                        if fword_buf[stack[udepth].ts_fidx as usize] != 0
                            && try_deeper(su, &stack, udepth, newscore)
                        {
                            go_deeper(&mut stack, udepth, newscore);
                            let old_fidx = stack[udepth].ts_fidx;
                            depth += 1;
                            udepth = depth as usize;
                            stack[udepth].ts_flags |= TSF_DIDDEL;
                            stack[udepth].ts_delidx = old_fidx;
                            c = utf_ptr2char(&fword_buf[old_fidx as usize..]);
                            stack[udepth].ts_fidx +=
                                utfc_ptr2len(&fword_buf[old_fidx as usize..]) as u8;
                            if utf_iscomposing_legacy(c) {
                                stack[udepth].ts_score -= SCORE_DEL - SCORE_DELCOMP;
                            } else if c
                                == utf_ptr2char(&fword_buf[stack[udepth].ts_fidx as usize..])
                            {
                                stack[udepth].ts_score -= SCORE_DEL - SCORE_DELDUP;
                            }
                            do_ins_prep = false;
                        }
                    }
                }
                if do_ins_prep {
                    if (stack[udepth].ts_flags & TSF_DIDDEL) != 0 {
                        stack[udepth].ts_state = SugState::StateSwap;
                    } else {
                        n = stack[udepth].ts_arridx;
                        loop {
                            if stack[udepth].ts_curi as i32 > i32::from(byts[n as usize]) {
                                stack[udepth].ts_state = SugState::StateSwap;
                                break;
                            }
                            if byts[(n + stack[udepth].ts_curi as i32) as usize] != 0 {
                                stack[udepth].ts_state = SugState::StateIns;
                                break;
                            }
                            stack[udepth].ts_curi += 1;
                        }
                    }
                }
            }
            SugState::StateIns => {
                n = stack[udepth].ts_arridx;
                if stack[udepth].ts_curi as i32 > i32::from(byts[n as usize]) {
                    stack[udepth].ts_state = SugState::StateSwap;
                    continue;
                }
                n += stack[udepth].ts_curi as i32;
                stack[udepth].ts_curi += 1;
                if ptr::eq(byts.as_ptr(), slang.sl_fbyts.as_ptr())
                    && n >= slang.sl_fbyts.len() as i32
                {
                    g_st().got_int.set(true);
                    continue;
                }
                c = i32::from(byts[n as usize]);
                let newscore = if soundfold && stack[udepth].ts_twordlen == 0 && c == b'*' as i32 {
                    2 * SCORE_INS / 3
                } else {
                    SCORE_INS
                };
                if c != i32::from(fword_buf[stack[udepth].ts_fidx as usize])
                    && try_deeper(su, &stack, udepth, newscore)
                {
                    go_deeper(&mut stack, udepth, newscore);
                    depth += 1;
                    udepth = depth as usize;
                    let twl = stack[udepth].ts_twordlen as usize;
                    tword[twl] = c as u8;
                    stack[udepth].ts_twordlen += 1;
                    stack[udepth].ts_arridx = idxs[n as usize];
                    fl = utf_byte2len(c);
                    if fl > 1 {
                        stack[udepth].ts_tcharlen = fl as u8;
                        stack[udepth].ts_tcharidx = 1;
                        stack[udepth].ts_isdiff = DIFF_INSERT;
                    }
                    if fl == 1
                        && stack[udepth].ts_twordlen >= 2
                        && tword[stack[udepth].ts_twordlen as usize - 2] as i32 == c
                    {
                        stack[udepth].ts_score -= SCORE_INS - SCORE_INSDUP;
                    }
                }
            }
            SugState::StateSwap => {
                let fidx = stack[udepth].ts_fidx as usize;
                c = fword_buf[fidx] as i32;
                if c == 0 {
                    stack[udepth].ts_state = SugState::StateFinal;
                    continue;
                }
                if !soundfold && !spell_iswordp(&fword_buf[fidx..], &curwin()) {
                    stack[udepth].ts_state = SugState::StateRepIni;
                    continue;
                }
                n = utf_ptr2len(&fword_buf[fidx..]);
                c = utf_ptr2char(&fword_buf[fidx..]);
                if fword_buf[fidx + n as usize] == 0 {
                    c2 = 0;
                } else if !soundfold && !spell_iswordp(&fword_buf[fidx + n as usize..], &curwin()) {
                    c2 = c;
                } else {
                    c2 = utf_ptr2char(&fword_buf[fidx + n as usize..]);
                }
                if c2 == 0 {
                    stack[udepth].ts_state = SugState::StateRepIni;
                    continue;
                }
                if c == c2 {
                    stack[udepth].ts_state = SugState::StateSwap3;
                    continue;
                }
                if try_deeper(su, &stack, udepth, SCORE_SWAP) {
                    go_deeper(&mut stack, udepth, SCORE_SWAP);
                    stack[udepth].ts_state = SugState::StateUnswap;
                    let old_fidx = stack[udepth].ts_fidx;
                    depth += 1;
                    udepth = depth as usize;
                    fl = utf_char2len(c2);
                    let n_u = n as usize;
                    let fl_u = fl as usize;
                    fword_buf.copy_within(fidx + n_u..fidx + n_u + fl_u, fidx);
                    utf_char2bytes(c, &mut fword_buf[fidx + fl_u..fidx + fl_u + 6]);
                    stack[udepth].ts_fidxtry = old_fidx + (n + fl) as u8;
                } else {
                    stack[udepth].ts_state = SugState::StateRepIni;
                }
            }
            SugState::StateUnswap | SugState::StateSwap3 => {
                if state == SugState::StateUnswap {
                    let fidx = stack[udepth].ts_fidx as usize;
                    n = utfc_ptr2len(&fword_buf[fidx..]);
                    c = utf_ptr2char(&fword_buf[fidx + n as usize..]);
                    let n_u = n as usize;
                    let dst_off = utfc_ptr2len(&fword_buf[fidx + n_u..]) as usize;
                    fword_buf.copy_within(fidx..fidx + n_u, fidx + dst_off);
                    utf_char2bytes(c, &mut fword_buf[fidx..fidx + 6]);
                }
                let fidx = stack[udepth].ts_fidx as usize;
                n = utf_ptr2len(&fword_buf[fidx..]);
                c = utf_ptr2char(&fword_buf[fidx..]);
                fl = utf_ptr2len(&fword_buf[fidx + n as usize..]);
                c2 = utf_ptr2char(&fword_buf[fidx + n as usize..]);
                if !soundfold && !spell_iswordp(&fword_buf[fidx + (n + fl) as usize..], &curwin()) {
                    c3 = c;
                } else {
                    c3 = utf_ptr2char(&fword_buf[fidx + (n + fl) as usize..]);
                }
                if c == c3 || c3 == 0 {
                    stack[udepth].ts_state = SugState::StateRepIni;
                    continue;
                }
                if try_deeper(su, &stack, udepth, SCORE_SWAP3) {
                    go_deeper(&mut stack, udepth, SCORE_SWAP3);
                    stack[udepth].ts_state = SugState::StateUnswap3;
                    let old_fidx = stack[udepth].ts_fidx;
                    depth += 1;
                    udepth = depth as usize;
                    tl = utf_char2len(c3);
                    let nfl_u = (n + fl) as usize;
                    let tl_u = tl as usize;
                    let fl_tl_u = (fl + tl) as usize;
                    fword_buf.copy_within(fidx + nfl_u..fidx + nfl_u + tl_u, fidx);
                    utf_char2bytes(c2, &mut fword_buf[fidx + tl_u..fidx + tl_u + 6]);
                    utf_char2bytes(c, &mut fword_buf[fidx + fl_tl_u..fidx + fl_tl_u + 6]);
                    stack[udepth].ts_fidxtry = old_fidx + (n + fl + tl) as u8;
                } else {
                    stack[udepth].ts_state = SugState::StateRepIni;
                }
            }
            SugState::StateUnswap3 => {
                let fidx = stack[udepth].ts_fidx as usize;
                n = utfc_ptr2len(&fword_buf[fidx..]);
                c2 = utf_ptr2char(&fword_buf[fidx + n as usize..]);
                fl = utfc_ptr2len(&fword_buf[fidx + n as usize..]);
                c = utf_ptr2char(&fword_buf[fidx + (n + fl) as usize..]);
                tl = utfc_ptr2len(&fword_buf[fidx + (n + fl) as usize..]);
                let n_u = n as usize;
                let tl_u = tl as usize;
                let fl_tl_u = (fl + tl) as usize;
                fword_buf.copy_within(fidx..fidx + n_u, fidx + fl_tl_u);
                utf_char2bytes(c, &mut fword_buf[fidx..fidx + 6]);
                utf_char2bytes(c2, &mut fword_buf[fidx + tl_u..fidx + tl_u + 6]);
                if !soundfold && !spell_iswordp(&fword_buf[fidx + tl_u..], &curwin()) {
                    stack[udepth].ts_state = SugState::StateRepIni;
                    continue;
                }
                if try_deeper(su, &stack, udepth, SCORE_SWAP3) {
                    go_deeper(&mut stack, udepth, SCORE_SWAP3);
                    stack[udepth].ts_state = SugState::StateUnrot3l;
                    let old_fidx = stack[udepth].ts_fidx;
                    let of_idx = old_fidx as usize;
                    depth += 1;
                    udepth = depth as usize;
                    n = utf_ptr2len(&fword_buf[of_idx..]);
                    c = utf_ptr2char(&fword_buf[of_idx..]);
                    fl = utf_ptr2len(&fword_buf[of_idx + n as usize..]);
                    fl += utf_ptr2len(&fword_buf[of_idx + (n + fl) as usize..]);
                    let n_u = n as usize;
                    let fl_u = fl as usize;
                    fword_buf.copy_within(of_idx + n_u..of_idx + n_u + fl_u, of_idx);
                    utf_char2bytes(c, &mut fword_buf[of_idx + fl_u..of_idx + fl_u + 6]);
                    stack[udepth].ts_fidxtry = old_fidx + (n + fl) as u8;
                } else {
                    stack[udepth].ts_state = SugState::StateRepIni;
                }
            }
            SugState::StateUnrot3l => {
                let fidx = stack[udepth].ts_fidx as usize;
                n = utfc_ptr2len(&fword_buf[fidx..]);
                n += utfc_ptr2len(&fword_buf[fidx + n as usize..]);
                c = utf_ptr2char(&fword_buf[fidx + n as usize..]);
                tl = utfc_ptr2len(&fword_buf[fidx + n as usize..]);
                let n_u = n as usize;
                let tl_u = tl as usize;
                fword_buf.copy_within(fidx..fidx + n_u, fidx + tl_u);
                utf_char2bytes(c, &mut fword_buf[fidx..fidx + 6]);
                if try_deeper(su, &stack, udepth, SCORE_SWAP3) {
                    go_deeper(&mut stack, udepth, SCORE_SWAP3);
                    stack[udepth].ts_state = SugState::StateUnrot3r;
                    let old_fidx = stack[udepth].ts_fidx;
                    let of_idx = old_fidx as usize;
                    depth += 1;
                    udepth = depth as usize;
                    n = utf_ptr2len(&fword_buf[of_idx..]);
                    n += utf_ptr2len(&fword_buf[of_idx + n as usize..]);
                    c = utf_ptr2char(&fword_buf[of_idx + n as usize..]);
                    tl = utf_ptr2len(&fword_buf[of_idx + n as usize..]);
                    let n_u = n as usize;
                    let tl_u = tl as usize;
                    fword_buf.copy_within(of_idx..of_idx + n_u, of_idx + tl_u);
                    utf_char2bytes(c, &mut fword_buf[of_idx..of_idx + 6]);
                    stack[udepth].ts_fidxtry = old_fidx + (n + tl) as u8;
                } else {
                    stack[udepth].ts_state = SugState::StateRepIni;
                }
            }
            SugState::StateUnrot3r | SugState::StateRepIni | SugState::StateRep => {
                let mut do_rep = true;
                if state != SugState::StateRep {
                    if state == SugState::StateUnrot3r {
                        let fidx = stack[udepth].ts_fidx as usize;
                        c = utf_ptr2char(&fword_buf[fidx..]);
                        tl = utfc_ptr2len(&fword_buf[fidx..]);
                        n = utfc_ptr2len(&fword_buf[fidx + tl as usize..]);
                        n += utfc_ptr2len(&fword_buf[fidx + (tl + n) as usize..]);
                        let tl_u = tl as usize;
                        let n_u = n as usize;
                        fword_buf.copy_within(fidx + tl_u..fidx + tl_u + n_u, fidx);
                        utf_char2bytes(c, &mut fword_buf[fidx + n_u..fidx + n_u + 6]);
                    }
                    if (lp.lp_replang.is_null() && !soundfold)
                        || stack[udepth].ts_score + SCORE_REP >= su.su_maxscore
                        || stack[udepth].ts_fidx < stack[udepth].ts_fidxtry
                    {
                        stack[udepth].ts_state = SugState::StateFinal;
                        do_rep = false;
                    } else {
                        let first_byte = fword_buf[stack[udepth].ts_fidx as usize] as usize;
                        stack[udepth].ts_curi = if soundfold {
                            slang.sl_repsal_first[first_byte]
                        } else {
                            replang_rc.as_ref().unwrap().sl_rep_first[first_byte]
                        };
                        if stack[udepth].ts_curi < 0 {
                            stack[udepth].ts_state = SugState::StateFinal;
                            do_rep = false;
                        } else {
                            stack[udepth].ts_state = SugState::StateRep;
                        }
                    }
                }
                if do_rep {
                    let fidx_cur = stack[udepth].ts_fidx as usize;
                    let gap = if soundfold {
                        &slang.sl_repsal
                    } else {
                        &replang_rc.as_ref().unwrap().sl_rep
                    };
                    while (stack[udepth].ts_curi as usize) < gap.len() {
                        let curi = stack[udepth].ts_curi;
                        stack[udepth].ts_curi += 1;
                        let ftp = &gap[curi as usize];
                        if ftp.ft_from.to_bytes().first().copied().unwrap_or(0)
                            != fword_buf[fidx_cur]
                        {
                            stack[udepth].ts_curi = gap.len() as i16;
                            break;
                        }
                        if crate::memory::bytes_until_nul(&fword_buf[fidx_cur..])
                            .starts_with(ftp.ft_from.as_bytes())
                            && try_deeper(su, &stack, udepth, SCORE_REP)
                        {
                            go_deeper(&mut stack, udepth, SCORE_REP);
                            stack[udepth].ts_state = SugState::StateRepUndo;
                            let old_fidx = stack[udepth].ts_fidx;
                            depth += 1;
                            udepth = depth as usize;
                            let fidx = old_fidx as usize;
                            fl = ftp.ft_from.as_bytes().len() as i32;
                            tl = ftp.ft_to.as_bytes().len() as i32;
                            if fl != tl {
                                let tail_len =
                                    crate::memory::bytes_as_cstr(&fword_buf[fidx + fl as usize..])
                                        .to_bytes()
                                        .len();
                                fword_buf.copy_within(
                                    fidx + fl as usize..=fidx + fl as usize + tail_len,
                                    fidx + tl as usize,
                                );
                                repextra += tl - fl;
                            }
                            fword_buf[fidx..fidx + tl as usize]
                                .copy_from_slice(ftp.ft_to.to_bytes());
                            stack[udepth].ts_fidxtry = old_fidx + tl as u8;
                            stack[udepth].ts_tcharlen = 0;
                            break;
                        }
                    }
                    if (stack[udepth].ts_curi as usize) >= gap.len()
                        && stack[udepth].ts_state == SugState::StateRep
                    {
                        stack[udepth].ts_state = SugState::StateFinal;
                    }
                }
            }
            SugState::StateRepUndo => {
                let gap = if soundfold {
                    &slang.sl_repsal
                } else {
                    &replang_rc.as_ref().unwrap().sl_rep
                };
                let ftp = &gap[(stack[udepth].ts_curi - 1) as usize];
                fl = ftp.ft_from.as_bytes().len() as i32;
                tl = ftp.ft_to.as_bytes().len() as i32;
                let fidx = stack[udepth].ts_fidx as usize;
                if fl != tl {
                    let tail_len = crate::memory::bytes_as_cstr(&fword_buf[fidx + tl as usize..])
                        .to_bytes()
                        .len();
                    fword_buf.copy_within(
                        fidx + tl as usize..=fidx + tl as usize + tail_len,
                        fidx + fl as usize,
                    );
                    repextra -= tl - fl;
                }
                fword_buf[fidx..fidx + fl as usize].copy_from_slice(ftp.ft_from.to_bytes());
                stack[udepth].ts_state = SugState::StateRep;
            }
            SugState::StateFinal => {
                depth -= 1;
                if depth >= 0 && stack[depth as usize].ts_prefixdepth == PFD_PREFIXTREE {
                    byts = pbyts;
                    idxs = pidxs;
                }
                breakcheckcount -= 1;
                if breakcheckcount == 0 {
                    os_breakcheck();
                    breakcheckcount = 1000;
                    if sps_st(|st| st.spell_suggest_timeout.get()) > 0
                        && profile_passed_limit(time_limit)
                    {
                        g_st().got_int.set(true);
                    }
                }
            }
        }
    }
}
#[inline]
fn go_deeper(stack: &mut [Trystate; MAXWLEN], depth: usize, score_add: i32) {
    stack[depth + 1] = stack[depth];
    stack[depth + 1].ts_state = SugState::StateStart;
    stack[depth + 1].ts_score = stack[depth].ts_score + score_add;
    stack[depth + 1].ts_curi = 1;
    stack[depth + 1].ts_flags = 0;
}
fn find_keepcap_word(slang: &Slang, fword: &[u8], kword: &mut [u8]) {
    let mut uword: [u8; MAXWLEN] = [0; MAXWLEN];
    let mut arridx: [Idx; MAXWLEN] = [0; MAXWLEN];
    let mut round: [i32; MAXWLEN] = [0; MAXWLEN];
    let mut fwordidx: [i32; MAXWLEN] = [0; MAXWLEN];
    let mut uwordidx: [i32; MAXWLEN] = [0; MAXWLEN];
    let mut kwordlen: [i32; MAXWLEN] = [0; MAXWLEN];
    if slang.sl_kbyts.is_empty() {
        kword[0] = 0;
        return;
    }
    let byts = &slang.sl_kbyts;
    let idxs = &slang.sl_kidxs;
    let fword_bytes = crate::memory::bytes_until_nul(fword);
    allcap_copy(fword_bytes, &mut uword);
    let mut depth: isize = 0;
    while depth >= 0 {
        let udepth = depth as usize;
        if fwordidx[udepth] as usize >= fword_bytes.len() {
            if byts[(arridx[udepth] + 1) as usize] == 0 {
                kword[kwordlen[udepth] as usize] = 0;
                return;
            }
            depth -= 1;
        } else {
            round[udepth] += 1;
            if round[udepth] > 2 {
                depth -= 1;
            } else {
                let flen = utf_ptr2len(&fword[fwordidx[udepth] as usize..]);
                let ulen = utf_ptr2len(&uword[uwordidx[udepth] as usize..]);
                let (mut p_slice, mut l): (&[u8], i32) = if round[udepth] == 1 {
                    (&fword[fwordidx[udepth] as usize..], flen)
                } else {
                    (&uword[uwordidx[udepth] as usize..], ulen)
                };
                let mut tryidx = arridx[udepth];
                while l > 0 {
                    let len = i32::from(byts[tryidx as usize]);
                    tryidx += 1;
                    let c = p_slice.first().copied().unwrap_or(0) as i32;
                    p_slice = &p_slice[1..];
                    let mut lo = tryidx;
                    let mut hi = tryidx + len - 1;
                    while lo < hi {
                        let m = (lo + hi) / 2;
                        if i32::from(byts[m as usize]) > c {
                            hi = m - 1;
                        } else if i32::from(byts[m as usize]) < c {
                            lo = m + 1;
                        } else {
                            lo = m;
                            hi = m;
                            break;
                        }
                    }
                    if hi < lo || i32::from(byts[lo as usize]) != c {
                        break;
                    }
                    tryidx = idxs[lo as usize];
                    l -= 1;
                }
                if l == 0 {
                    let dst_off = kwordlen[udepth] as usize;
                    if round[udepth] == 1 {
                        let foff = fwordidx[udepth] as usize;
                        kword[dst_off..dst_off + (flen as usize)]
                            .copy_from_slice(&fword[foff..foff + (flen as usize)]);
                        kwordlen[udepth + 1] = kwordlen[udepth] + flen;
                    } else {
                        let uoff = uwordidx[udepth] as usize;
                        kword[dst_off..dst_off + (ulen as usize)]
                            .copy_from_slice(&uword[uoff..uoff + (ulen as usize)]);
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
    kword[0] = 0;
}
fn score_comp_sal(su: &mut Suginfo) {
    let mut badsound: [u8; MAXWLEN] = [0; MAXWLEN];
    let sga = &mut su.su_sga;
    sga.reserve(su.su_ga.len());
    let w_s = curwin().w_s_ptr();
    let langps = w_s.b_langp.borrow().clone();
    for lp in &langps {
        let slang = lp.slang();
        if slang.has_sal() {
            spell_soundfold(&slang, &su.su_fbadword, true, &mut badsound);
            for stp in &su.su_ga {
                let score = stp_sal_score(stp, su, &slang, &badsound);
                if score < SCORE_MAXMAX {
                    let sga = &mut su.su_sga;
                    sga.push(Suggest {
                        st_word: stp.st_word.clone(),
                        st_wordlen: stp.st_wordlen,
                        st_orglen: stp.st_orglen,
                        st_score: score,
                        st_altscore: 0,
                        st_salscore: false,
                        st_had_bonus: false,
                        st_slang: SlangId::NULL,
                    });
                }
            }
            break;
        }
    }
}
fn score_combine(su: &mut Suginfo) {
    let mut badsound: [u8; MAXWLEN] = [0; MAXWLEN];
    let mut slang_opt: Option<std::rc::Rc<Slang>> = None;
    let w_s = curwin().w_s_ptr();
    let langps = w_s.b_langp.borrow().clone();
    for lp in &langps {
        let s = lp.slang();
        if s.has_sal() {
            spell_soundfold(&s, &su.su_fbadword, true, &mut badsound);
            let mut su_ga = std::mem::take(&mut su.su_ga);
            for stp in &mut su_ga {
                stp.st_altscore = stp_sal_score(stp, su, &s, &badsound);
                if stp.st_altscore == SCORE_MAXMAX {
                    stp.st_score = (stp.st_score * 3 + SCORE_BIG) / 4;
                } else {
                    stp.st_score = (stp.st_score * 3 + stp.st_altscore) / 4;
                }
                stp.st_salscore = false;
            }
            su.su_ga = su_ga;
            slang_opt = Some(s);
            break;
        }
    }
    let Some(slang) = slang_opt else {
        cleanup_suggestions(&mut su.su_ga, su.su_maxscore, su.su_maxcount);
        return;
    };
    for stp in &mut su.su_sga {
        stp.st_altscore = spell_edit_score(Some(&slang), &su.su_badword, stp.st_word.to_bytes());
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
    let mut ga: Vec<Suggest> = Vec::with_capacity(su.su_ga.len() + su.su_sga.len());
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
fn stp_sal_score(stp: &Suggest, su: &Suginfo, slang: &Slang, badsound: &[u8]) -> i32 {
    let mut badsound2: [u8; MAXWLEN] = [0; MAXWLEN];
    let mut fword: [u8; MAXWLEN] = [0; MAXWLEN];
    let mut goodsound: [u8; MAXWLEN] = [0; MAXWLEN];
    let mut goodword: [u8; MAXWLEN] = [0; MAXWLEN];
    let lendiff = su.su_badlen - stp.st_orglen;
    let pbad: &[u8] = if lendiff >= 0 {
        badsound
    } else {
        let badptr_bytes = su.su_badptr.as_bytes();
        let org_len = (stp.st_orglen.max(0) as usize).min(badptr_bytes.len());
        spell_casefold(&curwin(), &badptr_bytes[..org_len], &mut fword);
        if ascii_iswhite(
            su.su_badptr
                .to_bytes()
                .get(su.su_badlen as usize)
                .copied()
                .unwrap_or(0),
        ) && crate::charset::skiptowhite_slice(stp.st_word.as_bytes()).is_empty()
        {
            let mut fword_len = crate::memory::bytes_until_nul(&fword).len();
            let mut idx = 0;
            while idx < fword_len {
                let rem = crate::charset::skiptowhite_slice(&fword[idx..fword_len]);
                idx = fword_len - rem.len();
                if idx >= fword_len {
                    break;
                }
                fword.copy_within(idx + 1..=fword_len, idx);
                fword_len -= 1;
            }
        }
        spell_soundfold(slang, &fword, true, &mut badsound2);
        &badsound2
    };
    let pgood_bytes: &[u8] = if lendiff > 0 && (stp.st_wordlen + lendiff) < MAXWLEN as i32 {
        let wlen = stp.st_wordlen as usize;
        let ldiff = lendiff as usize;
        goodword[..wlen].copy_from_slice(stp.st_word.to_bytes());
        let bad_off = (su.su_badlen - lendiff) as usize;
        goodword[wlen..wlen + ldiff]
            .copy_from_slice(&su.su_badptr.to_bytes()[bad_off..bad_off + ldiff]);
        goodword[wlen + ldiff] = 0;
        &goodword[..wlen + ldiff]
    } else {
        stp.st_word.to_bytes()
    };
    spell_soundfold(slang, pgood_bytes, false, &mut goodsound);
    soundalike_score(&goodsound, pbad)
}
fn suggest_try_soundalike_prep() {
    let w_s = curwin().w_s_ptr();
    for lp in &*w_s.b_langp.borrow() {
        let slang = lp.slang();
        if slang.has_sal() && !slang.sl_sbyts.is_empty() {
            slang.sl_sounddone.borrow_mut().clear();
        }
    }
}
fn suggest_try_soundalike(su: &mut Suginfo) {
    let mut salword: [u8; MAXWLEN] = [0; MAXWLEN];
    let w_s = curwin().w_s_ptr();
    let langps = w_s.b_langp.borrow().clone();
    for lp in &langps {
        let slang = lp.slang();
        if slang.has_sal() && !slang.sl_sbyts.is_empty() {
            spell_soundfold(&slang, &su.su_fbadword, true, &mut salword);
            suggest_trie_walk(su, lp, &mut salword, true);
        }
    }
}
fn suggest_try_soundalike_finish() {
    let w_s = curwin().w_s_ptr();
    for lp in &*w_s.b_langp.borrow() {
        let slang = lp.slang();
        if slang.has_sal() && !slang.sl_sbyts.is_empty() {
            slang.sl_sounddone.borrow_mut().clear();
        }
    }
}
fn add_sound_suggest(su: &mut Suginfo, goodword: &[u8], score: i32, lp: &Langp) {
    let slang = lp.slang();
    let mut theword: [u8; MAXWLEN] = [0; MAXWLEN];
    let goodword_bytes = crate::memory::bytes_until_nul(goodword);
    {
        let mut sounddone = slang.sl_sounddone.borrow_mut();
        if let Some(prev_score) = sounddone.get_mut(goodword_bytes) {
            if score >= *prev_score as i32 {
                return;
            }
            *prev_score = score as i16;
        } else {
            sounddone.insert(goodword_bytes.to_vec(), score as i16);
        }
    }
    let Some(sfwordnr) = soundfold_find(&slang, goodword_bytes) else {
        internal_error(c"add_sound_suggest()");
        return;
    };
    let nrline_vec = {
        let sugbuf_borrow = slang.sl_sugbuf.borrow();
        let sugbuf = sugbuf_borrow.as_deref().unwrap();
        crate::memline::ml_get_buf_slice(sugbuf, sfwordnr + 1).to_vec()
    };
    let mut nrline: &[u8] = &nrline_vec;
    let mut orgnr: i32 = 0;
    while !nrline.is_empty() {
        orgnr += bytes2offset(&mut nrline);
        let byts = &slang.sl_fbyts;
        let idxs = &slang.sl_fidxs;
        let mut n: Idx = 0;
        let mut wordcount: i32 = 0;
        let mut wlen: usize = 0;
        let mut i: i32 = 1;
        'badword: {
            while wlen < MAXWLEN - 3 {
                i = 1;
                if wordcount == orgnr && byts[(n + 1) as usize] == 0 {
                    break;
                }
                if byts[(n + 1) as usize] == 0 {
                    wordcount += 1;
                }
                while byts[(n + i) as usize] == 0 {
                    if i > i32::from(byts[n as usize]) {
                        theword[wlen..wlen + 4].copy_from_slice(b"BAD\0");
                        wlen += 3;
                        break 'badword;
                    }
                    i += 1;
                }
                while i < i32::from(byts[n as usize]) {
                    let wc = idxs[idxs[(n + i) as usize] as usize];
                    if wordcount + wc > orgnr {
                        break;
                    }
                    wordcount += wc;
                    i += 1;
                }
                theword[wlen] = byts[(n + i) as usize];
                n = idxs[(n + i) as usize];
                wlen += 1;
            }
        }
        theword[wlen] = 0;
        while i <= i32::from(byts[n as usize]) && byts[(n + i) as usize] == 0 {
            let mut cword: [u8; MAXWLEN] = [0; MAXWLEN];
            let mut flags = idxs[(n + i) as usize];
            if (flags & WF_NOSUGGEST) != 0 {
                i += 1;
                continue;
            }
            let p_slice: &[u8] = if (flags & WF_KEEPCAP) != 0 {
                find_keepcap_word(&slang, &theword, &mut cword);
                &cword
            } else {
                flags |= su.su_badflags;
                if (flags & WF_CAPMASK) != 0 {
                    make_case_word(&theword, &mut cword, flags);
                    &cword
                } else {
                    &theword
                }
            };
            if (sps_st(|st| st.sps_flags.get()) & SPS_DOUBLE) != 0 {
                if score <= su.su_maxscore {
                    add_suggestion(
                        su,
                        true,
                        p_slice,
                        su.su_badlen,
                        score,
                        0,
                        false,
                        lp.lp_slang,
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
                let gc = utf_ptr2char(p_slice);
                if spell_isupper(gc) {
                    let bc = utf_ptr2char(&su.su_badword);
                    if !spell_isupper(bc) && spell_tofold(bc) != spell_tofold(gc) {
                        goodscore += SCORE_ICASE / 2;
                    }
                }
                let limit = maxscore(su.su_sfmaxscore - goodscore, score);
                if limit > SCORE_LIMITMAX {
                    goodscore += spell_edit_score(Some(&slang), &su.su_badword, p_slice);
                } else {
                    goodscore +=
                        spell_edit_score_limit(Some(&slang), &su.su_badword, p_slice, limit);
                }
                if goodscore < SCORE_MAXMAX {
                    goodscore = score_wordcount_adj(&slang, goodscore, p_slice, false);
                    goodscore = rescore(goodscore, score);
                    if goodscore <= su.su_sfmaxscore {
                        add_suggestion(
                            su,
                            false,
                            p_slice,
                            su.su_badlen,
                            goodscore,
                            score,
                            true,
                            lp.lp_slang,
                            true,
                        );
                    }
                }
            }
            i += 1;
        }
    }
}
fn soundfold_find(slang: &Slang, word: &[u8]) -> Option<i32> {
    let word_bytes = crate::memory::bytes_until_nul(word);
    let mut arridx: Idx = 0;
    let mut wlen: usize = 0;
    let mut wordnr: i32 = 0;
    let byts = &slang.sl_sbyts;
    let idxs = &slang.sl_sidxs;
    loop {
        let mut len = i32::from(byts[arridx as usize]);
        arridx += 1;
        let mut c = word_bytes.get(wlen).copied().unwrap_or(0);
        if byts[arridx as usize] == 0 {
            if c == 0 {
                break;
            }
            while len > 0 && byts[arridx as usize] == 0 {
                arridx += 1;
                len -= 1;
            }
            if len == 0 {
                return None;
            }
            wordnr += 1;
        }
        if c == 0 {
            return None;
        }
        if c == TAB {
            c = b' ';
        }
        while byts[arridx as usize] < c {
            wordnr += idxs[idxs[arridx as usize] as usize];
            arridx += 1;
            len -= 1;
            if len == 0 {
                return None;
            }
        }
        if byts[arridx as usize] != c {
            return None;
        }
        arridx = idxs[arridx as usize];
        wlen += 1;
        if c == b' ' {
            while matches!(word_bytes.get(wlen), Some(&(b' ' | TAB))) {
                wlen += 1;
            }
        }
    }
    Some(wordnr)
}
fn similar_chars(slang: &Slang, c1: i32, c2: i32) -> bool {
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
    su: &mut Suginfo,
    use_sga: bool,
    goodword: &[u8],
    badlenarg: i32,
    score: i32,
    altscore: i32,
    had_bonus: bool,
    slang_ptr: SlangId,
    maxsf: bool,
) {
    let goodword_bytes = crate::memory::bytes_until_nul(goodword);
    let bad_slice = &su.su_badptr.to_bytes()[..badlenarg.max(0) as usize];
    let mut pgood_idx = goodword_bytes.len();
    let mut pbad_idx = bad_slice.len();
    let mut goodlen: i32;
    let mut badlen: i32;
    loop {
        goodlen = pgood_idx as i32;
        badlen = pbad_idx as i32;
        if goodlen <= 0 || badlen <= 0 {
            break;
        }
        mb_cptr_back(goodword_bytes, &mut pgood_idx);
        mb_cptr_back(bad_slice, &mut pbad_idx);
        if utf_ptr2char(&goodword_bytes[pgood_idx..]) != utf_ptr2char(&bad_slice[pbad_idx..]) {
            break;
        }
    }
    if badlen == 0 && goodlen == 0 {
        return;
    }
    let mut found = false;
    let mut gap_vec = if use_sga {
        std::mem::take(&mut su.su_sga)
    } else {
        std::mem::take(&mut su.su_ga)
    };
    let gap = &mut gap_vec;
    let gw_slice = &goodword_bytes[..goodlen as usize];
    for stp in gap.iter_mut() {
        let st_b = stp.st_word.as_bytes();
        if stp.st_wordlen == goodlen
            && stp.st_orglen == badlen
            && &st_b[..(goodlen as usize).min(st_b.len())] == gw_slice
        {
            if stp.st_slang.is_null() {
                stp.st_slang = slang_ptr;
            }
            let mut new_sug = Suggest {
                st_word: std::ffi::CString::default(),
                st_wordlen: 0,
                st_orglen: 0,
                st_score: score,
                st_altscore: altscore,
                st_salscore: false,
                st_had_bonus: had_bonus,
                st_slang: SlangId::NULL,
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
        gap.push(Suggest {
            st_word: std::ffi::CString::new(gw_slice).unwrap(),
            st_wordlen: goodlen,
            st_score: score,
            st_altscore: altscore,
            st_salscore: false,
            st_had_bonus: had_bonus,
            st_orglen: badlen,
            st_slang: slang_ptr,
        });
        if (gap.len() as i32) > sug_max_count(su) {
            if maxsf {
                su.su_sfmaxscore = cleanup_suggestions(gap, su.su_sfmaxscore, sug_clean_count(su));
            } else {
                su.su_maxscore = cleanup_suggestions(gap, su.su_maxscore, sug_clean_count(su));
            }
        }
    }
    if use_sga {
        su.su_sga = gap_vec;
    } else {
        su.su_ga = gap_vec;
    }
}
fn check_suggestions(su: &mut Suginfo, use_sga: bool) {
    let badptr_bytes = su.su_badptr.as_bytes();
    let gap = if use_sga {
        &mut su.su_sga
    } else {
        &mut su.su_ga
    };
    let mut longword: [u8; MAXWLEN + 1] = [0; MAXWLEN + 1];
    if gap.is_empty() {
        return;
    }
    let mut i = gap.len() as isize - 1;
    while i >= 0 {
        let ui = i as usize;
        let st = &gap[ui];
        crate::memory::copy_bytes_to_c_buf(&mut longword, st.st_word.as_bytes());
        let len = st.st_wordlen as usize;
        let org_off = (st.st_orglen as usize).min(badptr_bytes.len());
        crate::memory::copy_bytes_to_c_buf(&mut longword[len..], &badptr_bytes[org_off..]);
        let mut attr: Hlf = HLF_COUNT;
        let _ = spell_check(
            &curwin(),
            crate::memory::bytes_as_cstr(&longword),
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
fn add_banned(su: &mut Suginfo, word: &[u8]) {
    let word_bytes = crate::memory::bytes_until_nul(word);
    if !su.su_banned.contains(word_bytes) {
        su.su_banned.insert(word_bytes.to_vec());
    }
}
fn rescore_suggestions(su: &mut Suginfo) {
    if !su.su_sallang.is_null() {
        let mut su_ga = std::mem::take(&mut su.su_ga);
        for stp in &mut su_ga {
            rescore_one(su, stp);
        }
        su.su_ga = su_ga;
    }
}
fn rescore_one(su: &mut Suginfo, stp: &mut Suggest) {
    let slang_id = stp.st_slang;
    let mut sal_badword: [u8; MAXWLEN] = [0; MAXWLEN];
    if let Some(slang) = slang_id.opt_rc()
        && slang.has_sal()
        && !stp.st_had_bonus
    {
        let p_slice: &[u8] = if su.su_sallang == slang_id {
            &su.su_sal_badword
        } else {
            spell_soundfold(&slang, &su.su_fbadword, true, &mut sal_badword);

            &sal_badword
        };

        stp.st_altscore = stp_sal_score(stp, su, &slang, p_slice);

        if stp.st_altscore == SCORE_MAXMAX {
            stp.st_altscore = SCORE_BIG;
        }
        stp.st_score = rescore(stp.st_score, stp.st_altscore);
        stp.st_had_bonus = true;
    }
}
fn sug_compare(p1: &Suggest, p2: &Suggest) -> std::cmp::Ordering {
    p1.st_score
        .cmp(&p2.st_score)
        .then_with(|| p1.st_altscore.cmp(&p2.st_altscore))
        .then_with(|| mb_stricmp(p1.st_word.as_c_str(), p2.st_word.as_c_str()).cmp(&0))
}
fn cleanup_suggestions(gap: &mut Vec<Suggest>, maxscore: i32, keep: i32) -> i32 {
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
fn soundalike_score(goodstart: &[u8], badstart: &[u8]) -> i32 {
    let mut goodsound = crate::memory::bytes_until_nul(goodstart);
    let mut badsound = crate::memory::bytes_until_nul(badstart);
    let mut score: i32 = 0;
    let b0 = badsound.first().copied().unwrap_or(0);
    let g0 = goodsound.first().copied().unwrap_or(0);
    if (b0 == b'*' || g0 == b'*') && b0 != g0 {
        if (badsound.is_empty() && goodsound.len() == 1)
            || (goodsound.is_empty() && badsound.len() == 1)
        {
            return SCORE_DEL;
        }
        if badsound.is_empty() || goodsound.is_empty() {
            return SCORE_MAXMAX;
        }
        let b1 = badsound.get(1).copied().unwrap_or(0);
        let g1 = goodsound.get(1).copied().unwrap_or(0);
        if b1 == g1
            || (b1 != 0
                && g1 != 0
                && badsound.get(2).copied().unwrap_or(0) == goodsound.get(2).copied().unwrap_or(0))
        {
            // handle like a substitute
        } else {
            score = 2 * SCORE_DEL / 3;
            if b0 == b'*' {
                badsound = &badsound[1..];
            } else {
                goodsound = &goodsound[1..];
            }
        }
    }
    let goodlen = goodsound.len() as i32;
    let badlen = badsound.len() as i32;
    let n = goodlen - badlen;
    if !(-2..=2).contains(&n) {
        return SCORE_MAXMAX;
    }
    let (pl_full, ps_full) = if n > 0 {
        (goodsound, badsound)
    } else {
        (badsound, goodsound)
    };
    let common = pl_full
        .iter()
        .zip(ps_full)
        .take_while(|(a, b)| a == b)
        .count();
    let pl = &pl_full[common..];
    let ps = &ps_full[common..];
    match n {
        -2 | 2 => {
            let pl1 = &pl[1..];
            let c2 = pl1.iter().zip(ps).take_while(|(a, b)| a == b).count();
            if pl1[c2 + 1..] == ps[c2..] {
                return score + SCORE_DEL * 2;
            }
        }
        -1 | 1 => {
            let pl1 = &pl[1..];
            if pl1 == ps {
                return score + SCORE_DEL;
            }
            let c2 = pl1.iter().zip(ps).take_while(|(a, b)| a == b).count();
            let pl2 = &pl1[c2..];
            let ps2 = &ps[c2..];
            if pl2.len() >= 2 && pl2[0] == ps2[1] && pl2[1] == ps2[0] && pl2[2..] == ps2[2..] {
                return score + SCORE_DEL + SCORE_SWAP;
            }
            if pl2[1..] == ps2[1..] {
                return score + SCORE_DEL + SCORE_SUBST;
            }
            if ps.len() >= 2 && pl[0] == ps[1] && pl[1] == ps[0] {
                let pl2 = &pl[2..];
                let ps2 = &ps[2..];
                let c3 = pl2.iter().zip(ps2).take_while(|(a, b)| a == b).count();
                if pl2[c3 + 1..] == ps2[c3..] {
                    return score + SCORE_SWAP + SCORE_DEL;
                }
            }
            if !ps.is_empty() {
                let pl2 = &pl[1..];
                let ps2 = &ps[1..];
                let c4 = pl2.iter().zip(ps2).take_while(|(a, b)| a == b).count();
                if pl2[c4 + 1..] == ps2[c4..] {
                    return score + SCORE_SUBST + SCORE_DEL;
                }
            }
        }
        0 => {
            if pl.is_empty() {
                return score;
            }
            if pl.len() >= 2 && pl[0] == ps[1] && pl[1] == ps[0] {
                if pl[2..] == ps[2..] {
                    return score + SCORE_SWAP;
                }
                let c2 = pl[2..]
                    .iter()
                    .zip(&ps[2..])
                    .take_while(|(a, b)| a == b)
                    .count();
                let pl2 = &pl[2 + c2..];
                let ps2 = &ps[2 + c2..];
                if pl2.len() >= 2 && pl2[0] == ps2[1] && pl2[1] == ps2[0] && pl2[2..] == ps2[2..] {
                    return score + SCORE_SWAP + SCORE_SWAP;
                }
                if pl2[1..] == ps2[1..] {
                    return score + SCORE_SWAP + SCORE_SUBST;
                }
            }
            if pl[1..] == ps[1..] {
                return score + SCORE_SUBST;
            }
            let c2 = pl[1..]
                .iter()
                .zip(&ps[1..])
                .take_while(|(a, b)| a == b)
                .count();
            let pl2 = &pl[1 + c2..];
            let ps2 = &ps[1 + c2..];
            if pl2.len() >= 2 && pl2[0] == ps2[1] && pl2[1] == ps2[0] && pl2[2..] == ps2[2..] {
                return score + SCORE_SUBST + SCORE_SWAP;
            }
            if pl2[1..] == ps2[1..] {
                return score + SCORE_SUBST + SCORE_SUBST;
            }
            let ps1 = &ps[1..];
            let c3 = pl.iter().zip(ps1).take_while(|(a, b)| a == b).count();
            if pl[c3 + 1..] == ps1[c3..] {
                return score + SCORE_INS + SCORE_DEL;
            }
            let pl1 = &pl[1..];
            let c4 = pl1.iter().zip(ps).take_while(|(a, b)| a == b).count();
            if pl1[c4..] == ps[c4 + 1..] {
                return score + SCORE_INS + SCORE_DEL;
            }
        }
        _ => {}
    }
    SCORE_MAXMAX
}
fn spell_edit_score(slang: Option<&Slang>, badword: &[u8], goodword: &[u8]) -> i32 {
    let mut wbadword: [i32; MAXWLEN] = [0; MAXWLEN];
    let mut wgoodword: [i32; MAXWLEN] = [0; MAXWLEN];
    let mut badlen: usize = 0;
    let mut rem = crate::memory::bytes_until_nul(badword);
    while !rem.is_empty() {
        wbadword[badlen] = mb_cptr2char_adv(&mut rem);
        badlen += 1;
    }
    wbadword[badlen] = 0;
    badlen += 1;
    let mut goodlen: usize = 0;
    let mut rem = crate::memory::bytes_until_nul(goodword);
    while !rem.is_empty() {
        wgoodword[goodlen] = mb_cptr2char_adv(&mut rem);
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
                } else if let Some(slang) = slang
                    && slang.sl_has_map
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
fn spell_edit_score_limit(
    slang: Option<&Slang>,
    badword: &[u8],
    goodword: &[u8],
    limit: i32,
) -> i32 {
    spell_edit_score_limit_w(slang, badword, goodword, limit)
}
fn spell_edit_score_limit_w(
    slang: Option<&Slang>,
    badword: &[u8],
    goodword: &[u8],
    limit: i32,
) -> i32 {
    let mut stack: [Limitscore; 10] = [Limitscore::default(); 10];
    let mut wbadword: [i32; MAXWLEN] = [0; MAXWLEN];
    let mut wgoodword: [i32; MAXWLEN] = [0; MAXWLEN];
    let mut bi: usize = 0;
    let mut rem = crate::memory::bytes_until_nul(badword);
    while !rem.is_empty() {
        wbadword[bi] = mb_cptr2char_adv(&mut rem);
        bi += 1;
    }
    wbadword[bi] = 0;
    let mut gi: usize = 0;
    let mut rem = crate::memory::bytes_until_nul(goodword);
    while !rem.is_empty() {
        wgoodword[gi] = mb_cptr2char_adv(&mut rem);
        gi += 1;
    }
    wgoodword[gi] = 0;
    let mut stackidx: usize = 0;
    bi = 0;
    gi = 0;
    let mut score: i32 = 0;
    let mut minscore = limit + 1;
    loop {
        let mut bc: i32;
        let mut gc: i32;
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
                            stack[stackidx].badi = (bi + 1 - round) as i32;
                            stack[stackidx].goodi = (gi + round) as i32;
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
                } else if let Some(slang) = slang
                    && slang.sl_has_map
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bytes2offset_and_soundalike_score_work() {
        let raw = [5u8, 0u8];
        let mut p: &[u8] = &raw;
        assert_eq!(bytes2offset(&mut p), 4);
        assert_eq!(soundalike_score(b"tht", b"tht"), 0);
        assert_eq!(soundalike_score(b"tht", b"ht"), SCORE_DEL);
    }
}
