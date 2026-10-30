//! NFA regular expression engine (`src/nvim/regexp.c`).

use crate::message::{semsg, siemsg};
use std::ffi::CStr;
use std::mem::size_of;
use std::ptr;

use super::*;

const NFA_ADD_NL: i32 = 31;

const NFA_SPLIT: i32 = -1024;
const NFA_MATCH: i32 = NFA_SPLIT + 1;
const NFA_EMPTY: i32 = NFA_SPLIT + 2;

const NFA_START_COLL: i32 = NFA_SPLIT + 3;
const NFA_END_COLL: i32 = NFA_SPLIT + 4;
const NFA_START_NEG_COLL: i32 = NFA_SPLIT + 5;
const NFA_END_NEG_COLL: i32 = NFA_SPLIT + 6;
const NFA_RANGE: i32 = NFA_SPLIT + 7;
const NFA_RANGE_MIN: i32 = NFA_SPLIT + 8;
const NFA_RANGE_MAX: i32 = NFA_SPLIT + 9;

const NFA_CONCAT: i32 = NFA_SPLIT + 10;
const NFA_OR: i32 = NFA_SPLIT + 11;
const NFA_STAR: i32 = NFA_SPLIT + 12;
const NFA_STAR_NONGREEDY: i32 = NFA_SPLIT + 13;
const NFA_QUEST: i32 = NFA_SPLIT + 14;
const NFA_QUEST_NONGREEDY: i32 = NFA_SPLIT + 15;

const NFA_BOL: i32 = NFA_SPLIT + 16;
const NFA_EOL: i32 = NFA_SPLIT + 17;
const NFA_BOW: i32 = NFA_SPLIT + 18;
const NFA_EOW: i32 = NFA_SPLIT + 19;
const NFA_BOF: i32 = NFA_SPLIT + 20;
const NFA_EOF: i32 = NFA_SPLIT + 21;
const NFA_NEWL: i32 = NFA_SPLIT + 22;
const NFA_ZSTART: i32 = NFA_SPLIT + 23;
const NFA_ZEND: i32 = NFA_SPLIT + 24;
const NFA_NOPEN: i32 = NFA_SPLIT + 25;
const NFA_NCLOSE: i32 = NFA_SPLIT + 26;
const NFA_START_INVISIBLE: i32 = NFA_SPLIT + 27;
const NFA_START_INVISIBLE_FIRST: i32 = NFA_SPLIT + 28;
const NFA_START_INVISIBLE_NEG: i32 = NFA_SPLIT + 29;
const NFA_START_INVISIBLE_NEG_FIRST: i32 = NFA_SPLIT + 30;
const NFA_START_INVISIBLE_BEFORE: i32 = NFA_SPLIT + 31;
const NFA_START_INVISIBLE_BEFORE_FIRST: i32 = NFA_SPLIT + 32;
const NFA_START_INVISIBLE_BEFORE_NEG: i32 = NFA_SPLIT + 33;
const NFA_START_INVISIBLE_BEFORE_NEG_FIRST: i32 = NFA_SPLIT + 34;
const NFA_START_PATTERN: i32 = NFA_SPLIT + 35;
const NFA_END_INVISIBLE: i32 = NFA_SPLIT + 36;
const NFA_END_INVISIBLE_NEG: i32 = NFA_SPLIT + 37;
const NFA_END_PATTERN: i32 = NFA_SPLIT + 38;
const NFA_COMPOSING: i32 = NFA_SPLIT + 39;
const NFA_END_COMPOSING: i32 = NFA_SPLIT + 40;
const NFA_ANY_COMPOSING: i32 = NFA_SPLIT + 41;
const NFA_OPT_CHARS: i32 = NFA_SPLIT + 42;

const NFA_PREV_ATOM_NO_WIDTH: i32 = NFA_SPLIT + 43;
const NFA_PREV_ATOM_NO_WIDTH_NEG: i32 = NFA_SPLIT + 44;
const NFA_PREV_ATOM_JUST_BEFORE: i32 = NFA_SPLIT + 45;
const NFA_PREV_ATOM_JUST_BEFORE_NEG: i32 = NFA_SPLIT + 46;
const NFA_PREV_ATOM_LIKE_PATTERN: i32 = NFA_SPLIT + 47;

const NFA_BACKREF1: i32 = NFA_SPLIT + 48;
const NFA_BACKREF9: i32 = NFA_SPLIT + 56;
const NFA_ZREF1: i32 = NFA_SPLIT + 57;
const NFA_ZREF9: i32 = NFA_SPLIT + 65;
const NFA_SKIP: i32 = NFA_SPLIT + 66;

const NFA_MOPEN: i32 = NFA_SPLIT + 67;
const NFA_MOPEN1: i32 = NFA_SPLIT + 68;
const NFA_MOPEN9: i32 = NFA_SPLIT + 76;

const NFA_MCLOSE: i32 = NFA_SPLIT + 77;
const NFA_MCLOSE9: i32 = NFA_SPLIT + 86;

const NFA_ZOPEN: i32 = NFA_SPLIT + 87;
const NFA_ZOPEN9: i32 = NFA_SPLIT + 96;

const NFA_ZCLOSE: i32 = NFA_SPLIT + 97;
const NFA_ZCLOSE1: i32 = NFA_SPLIT + 98;
const NFA_ZCLOSE9: i32 = NFA_SPLIT + 106;

const NFA_ANY: i32 = NFA_SPLIT + 107;
const NFA_IDENT: i32 = NFA_SPLIT + 108;
const NFA_SIDENT: i32 = NFA_SPLIT + 109;
const NFA_KWORD: i32 = NFA_SPLIT + 110;
const NFA_SKWORD: i32 = NFA_SPLIT + 111;
const NFA_FNAME: i32 = NFA_SPLIT + 112;
const NFA_SFNAME: i32 = NFA_SPLIT + 113;
const NFA_PRINT: i32 = NFA_SPLIT + 114;
const NFA_SPRINT: i32 = NFA_SPLIT + 115;
const NFA_WHITE: i32 = NFA_SPLIT + 116;
const NFA_NWHITE: i32 = NFA_SPLIT + 117;
const NFA_DIGIT: i32 = NFA_SPLIT + 118;
const NFA_NDIGIT: i32 = NFA_SPLIT + 119;
const NFA_HEX: i32 = NFA_SPLIT + 120;
const NFA_NHEX: i32 = NFA_SPLIT + 121;
const NFA_OCTAL: i32 = NFA_SPLIT + 122;
const NFA_NOCTAL: i32 = NFA_SPLIT + 123;
const NFA_WORD: i32 = NFA_SPLIT + 124;
const NFA_NWORD: i32 = NFA_SPLIT + 125;
const NFA_HEAD: i32 = NFA_SPLIT + 126;
const NFA_NHEAD: i32 = NFA_SPLIT + 127;
const NFA_ALPHA: i32 = NFA_SPLIT + 128;
const NFA_NALPHA: i32 = NFA_SPLIT + 129;
const NFA_LOWER: i32 = NFA_SPLIT + 130;
const NFA_NLOWER: i32 = NFA_SPLIT + 131;
const NFA_UPPER: i32 = NFA_SPLIT + 132;
const NFA_NUPPER: i32 = NFA_SPLIT + 133;
const NFA_LOWER_IC: i32 = NFA_SPLIT + 134;
const NFA_NLOWER_IC: i32 = NFA_SPLIT + 135;
const NFA_UPPER_IC: i32 = NFA_SPLIT + 136;
const NFA_NUPPER_IC: i32 = NFA_SPLIT + 137;

const NFA_FIRST_NL: i32 = NFA_ANY + NFA_ADD_NL;
const NFA_LAST_NL: i32 = NFA_NUPPER_IC + NFA_ADD_NL;

const NFA_CURSOR: i32 = NFA_LAST_NL + 1;
const NFA_LNUM: i32 = NFA_CURSOR + 1;
const NFA_LNUM_GT: i32 = NFA_CURSOR + 2;
const NFA_LNUM_LT: i32 = NFA_CURSOR + 3;
const NFA_COL: i32 = NFA_CURSOR + 4;
const NFA_COL_GT: i32 = NFA_CURSOR + 5;
const NFA_COL_LT: i32 = NFA_CURSOR + 6;
const NFA_VCOL: i32 = NFA_CURSOR + 7;
const NFA_VCOL_GT: i32 = NFA_CURSOR + 8;
const NFA_VCOL_LT: i32 = NFA_CURSOR + 9;
const NFA_MARK: i32 = NFA_CURSOR + 10;
const NFA_MARK_GT: i32 = NFA_CURSOR + 11;
const NFA_MARK_LT: i32 = NFA_CURSOR + 12;
const NFA_VISUAL: i32 = NFA_CURSOR + 13;

const NFA_CLASS_ALNUM: i32 = NFA_CURSOR + 14;
const NFA_CLASS_ALPHA: i32 = NFA_CURSOR + 15;
const NFA_CLASS_BLANK: i32 = NFA_CURSOR + 16;
const NFA_CLASS_CNTRL: i32 = NFA_CURSOR + 17;
const NFA_CLASS_DIGIT: i32 = NFA_CURSOR + 18;
const NFA_CLASS_GRAPH: i32 = NFA_CURSOR + 19;
const NFA_CLASS_LOWER: i32 = NFA_CURSOR + 20;
const NFA_CLASS_PRINT: i32 = NFA_CURSOR + 21;
const NFA_CLASS_PUNCT: i32 = NFA_CURSOR + 22;
const NFA_CLASS_SPACE: i32 = NFA_CURSOR + 23;
const NFA_CLASS_UPPER: i32 = NFA_CURSOR + 24;
const NFA_CLASS_XDIGIT: i32 = NFA_CURSOR + 25;
const NFA_CLASS_TAB: i32 = NFA_CURSOR + 26;
const NFA_CLASS_RETURN: i32 = NFA_CURSOR + 27;
const NFA_CLASS_BACKSPACE: i32 = NFA_CURSOR + 28;
const NFA_CLASS_ESCAPE: i32 = NFA_CURSOR + 29;
const NFA_CLASS_IDENT: i32 = NFA_CURSOR + 30;
const NFA_CLASS_KEYWORD: i32 = NFA_CURSOR + 31;
const NFA_CLASS_FNAME: i32 = NFA_CURSOR + 32;

const NFA_POSTFIX_KEEP: usize = 10000;

const NFA_PIM_UNUSED: i32 = 0;
const NFA_PIM_TODO: i32 = 1;
const NFA_PIM_MATCH: i32 = 2;
const NFA_PIM_NOMATCH: i32 = 3;

const ADDSTATE_HERE_OFFSET: i32 = 10;

static NFA_CLASSCODES: [i32; 27] = [
    NFA_ANY, NFA_IDENT, NFA_SIDENT, NFA_KWORD, NFA_SKWORD, NFA_FNAME, NFA_SFNAME, NFA_PRINT,
    NFA_SPRINT, NFA_WHITE, NFA_NWHITE, NFA_DIGIT, NFA_NDIGIT, NFA_HEX, NFA_NHEX, NFA_OCTAL,
    NFA_NOCTAL, NFA_WORD, NFA_NWORD, NFA_HEAD, NFA_NHEAD, NFA_ALPHA, NFA_NALPHA, NFA_LOWER,
    NFA_NLOWER, NFA_UPPER, NFA_NUPPER,
];

const E_NUL_FOUND: &CStr = c"E865: (NFA) Regexp end encountered prematurely";
const E_MISPLACED: &CStr = c"E866: (NFA regexp) Misplaced %c";
const E_ILL_CHAR_CLASS: &CStr = c"E877: (NFA regexp) Invalid character class: %lld";
const E_VALUE_TOO_LARGE: &CStr = c"E951: \\% value too large";

struct RegexpNfaState {
    temp_subs: Regsubs,
    nfa_match: i32,
    nfa_time_limit: Option<Proftime>,
    nfa_timed_out: *mut i32,
    nfa_time_count: i32,
    rec_listbuf: Vec<[Vec<NfaThread>; 2]>,
}

impl RegexpNfaState {
    const fn new() -> Self {
        Self {
            temp_subs: Regsubs::new(),
            nfa_match: 0,
            nfa_time_limit: None,
            nfa_timed_out: ptr::null_mut(),
            nfa_time_count: 0,
            rec_listbuf: Vec::new(),
        }
    }
}

static NFA_STATE: GlobalState<RegexpNfaState> = GlobalState::new(RegexpNfaState::new());

#[inline]
fn nfa_st() -> &'static mut RegexpNfaState {
    unsafe { &mut *NFA_STATE.as_ptr() }
}

#[inline]
fn emit_fn(c: i32) {
    re_st().post_vec.push(c);
}

#[inline]
fn emit2(c: i32) {
    emit_fn(c);
    emit_fn(NFA_CONCAT);
}

fn nfa_regcomp_start(expr: *mut u8, re_flags: i32) {
    re_st().nstate = 0;
    re_st().istate = 0;
    let nstate_max = (crate::memory::cstr_len(expr.cast()) + 1) * 25 + 1000;
    re_st().post_vec.clear();
    re_st().post_vec.reserve(nstate_max);
    re_st().wants_nfa = false;
    re_st().rex.nfa_has_zend = 0;
    re_st().rex.nfa_has_backref = 0;
    re_st().nfa_reg_parse_depth = 0;

    regcomp_start(expr, re_flags);
}

fn nfa_get_reganch(states: &[NfaState], start: Option<usize>, depth: i32) -> i32 {
    let mut p = start;
    if depth > 4 {
        return 0;
    }
    while let Some(idx) = p {
        let st = states[idx];
        match st.c {
            NFA_BOL | NFA_BOF => return 1,
            NFA_ZSTART | NFA_ZEND | NFA_CURSOR | NFA_VISUAL | NFA_NOPEN => {
                p = st.out;
            }
            c if (NFA_MOPEN..=NFA_MOPEN9).contains(&c) || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) => {
                p = st.out;
            }
            NFA_SPLIT => {
                return i32::from(
                    nfa_get_reganch(states, st.out, depth + 1) != 0
                        && nfa_get_reganch(states, st.out1, depth + 1) != 0,
                );
            }
            _ => return 0,
        }
    }
    0
}

fn nfa_get_regstart(states: &[NfaState], start: Option<usize>, depth: i32) -> i32 {
    let mut p = start;
    if depth > 4 {
        return 0;
    }
    while let Some(idx) = p {
        let st = states[idx];
        match st.c {
            NFA_BOL | NFA_BOF | NFA_BOW | NFA_EOW | NFA_ZSTART | NFA_ZEND | NFA_CURSOR
            | NFA_VISUAL | NFA_LNUM | NFA_LNUM_GT | NFA_LNUM_LT | NFA_COL | NFA_COL_GT
            | NFA_COL_LT | NFA_VCOL | NFA_VCOL_GT | NFA_VCOL_LT | NFA_MARK | NFA_MARK_GT
            | NFA_MARK_LT | NFA_NOPEN => {
                p = st.out;
            }
            c if (NFA_MOPEN..=NFA_MOPEN9).contains(&c) || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) => {
                p = st.out;
            }
            NFA_SPLIT => {
                let c1 = nfa_get_regstart(states, st.out, depth + 1);
                let c2 = nfa_get_regstart(states, st.out1, depth + 1);
                if c1 == c2 {
                    return c1;
                }
                return 0;
            }
            c => {
                if c > 0 {
                    return c;
                }
                return 0;
            }
        }
    }
    0
}

fn nfa_get_match_text(states: &[NfaState], start: Option<usize>) -> Option<std::ffi::CString> {
    let start_idx = start?;
    let mut p = start_idx;
    let mut len: usize = 0;

    if states[p].c != NFA_MOPEN {
        return None;
    }
    p = states[p].out?;
    while states[p].c > 0 {
        len += utf_char2len(states[p].c) as usize;
        p = states[p].out?;
    }
    let out_idx = states[p].out?;
    if states[p].c != NFA_MCLOSE || states[out_idx].c != NFA_MATCH {
        return None;
    }

    let mut ret = vec![0u8; len];
    p = states[states[start_idx].out?].out?;
    let mut s = ret.as_mut_ptr();
    while states[p].c > 0 {
        s = s.wrapping_add(utf_char2bytes(states[p].c, s.cast()) as usize);
        p = states[p].out?;
    }
    let actual_len = s as usize - ret.as_ptr() as usize;
    ret.truncate(actual_len);
    std::ffi::CString::new(ret).ok()
}

fn nfa_recognize_char_class(start: *mut u8, end: *const u8, mut extra_newl: i32) -> i32 {
    let mut newl = false;
    let config = get_char_class_bits(start, end, &mut newl);
    if config < 0 {
        return FAIL;
    }
    if extra_newl == 1 {
        newl = true;
    }
    if newl {
        extra_newl = NFA_ADD_NL;
    }
    if config == CLASS_O9 {
        extra_newl + NFA_DIGIT
    } else if config == (CLASS_NOT | CLASS_O9) {
        extra_newl + NFA_NDIGIT
    } else if config == (CLASS_AF_CONST | CLASS_AF | CLASS_O9) {
        extra_newl + NFA_HEX
    } else if config == (CLASS_NOT | CLASS_AF_CONST | CLASS_AF | CLASS_O9) {
        extra_newl + NFA_NHEX
    } else if config == CLASS_O7 {
        extra_newl + NFA_OCTAL
    } else if config == (CLASS_NOT | CLASS_O7) {
        extra_newl + NFA_NOCTAL
    } else if config == (CLASS_AZ_CONST | CLASS_AZ | CLASS_O9 | CLASS_UNDERSCORE) {
        extra_newl + NFA_WORD
    } else if config == (CLASS_NOT | CLASS_AZ_CONST | CLASS_AZ | CLASS_O9 | CLASS_UNDERSCORE) {
        extra_newl + NFA_NWORD
    } else if config == (CLASS_AZ_CONST | CLASS_AZ | CLASS_UNDERSCORE) {
        extra_newl + NFA_HEAD
    } else if config == (CLASS_NOT | CLASS_AZ_CONST | CLASS_AZ | CLASS_UNDERSCORE) {
        extra_newl + NFA_NHEAD
    } else if config == (CLASS_AZ_CONST | CLASS_AZ) {
        extra_newl + NFA_ALPHA
    } else if config == (CLASS_NOT | CLASS_AZ_CONST | CLASS_AZ) {
        extra_newl + NFA_NALPHA
    } else if config == CLASS_AZ_CONST {
        extra_newl + NFA_LOWER_IC
    } else if config == (CLASS_NOT | CLASS_AZ_CONST) {
        extra_newl + NFA_NLOWER_IC
    } else if config == CLASS_AZ {
        extra_newl + NFA_UPPER_IC
    } else if config == (CLASS_NOT | CLASS_AZ) {
        extra_newl + NFA_NUPPER_IC
    } else {
        FAIL
    }
}

#[rustfmt::skip]
fn nfa_emit_equi_class(c: i32) {
    match c {
            0x41 | 0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc4 | 0xc5 | 0x100 | 0x102 | 0x104 | 0x1cd
            | 0x1de | 0x1e0 | 0x1fa | 0x200 | 0x202 | 0x226 | 0x23a | 0x1e00 | 0x1ea0 | 0x1ea2
            | 0x1ea4 | 0x1ea6 | 0x1ea8 | 0x1eaa | 0x1eac | 0x1eae | 0x1eb0 | 0x1eb2 | 0x1eb4
            | 0x1eb6 => {
                emit2(b'A' as i32); emit2(0xc0); emit2(0xc1);
                emit2(0xc2); emit2(0xc3);
                emit2(0xc4); emit2(0xc5);
                emit2(0x100); emit2(0x102); emit2(0x104);
                emit2(0x1cd); emit2(0x1de); emit2(0x1e0);
                emit2(0x1fa); emit2(0x200); emit2(0x202);
                emit2(0x226); emit2(0x23a); emit2(0x1e00);
                emit2(0x1ea0); emit2(0x1ea2); emit2(0x1ea4);
                emit2(0x1ea6); emit2(0x1ea8); emit2(0x1eaa);
                emit2(0x1eac); emit2(0x1eae); emit2(0x1eb0);
                emit2(0x1eb2); emit2(0x1eb6); emit2(0x1eb4);
                return;
            }
            0x42 | 0x181 | 0x243 | 0x1e02 | 0x1e04 | 0x1e06 => {
                emit2(b'B' as i32);
                emit2(0x181); emit2(0x243); emit2(0x1e02);
                emit2(0x1e04); emit2(0x1e06);
                return;
            }
            0x43 | 0xc7 | 0x106 | 0x108 | 0x10a | 0x10c | 0x187 | 0x23b | 0x1e08 | 0xa792 => {
                emit2(b'C' as i32); emit2(0xc7);
                emit2(0x106); emit2(0x108); emit2(0x10a);
                emit2(0x10c); emit2(0x187); emit2(0x23b);
                emit2(0x1e08); emit2(0xa792);
                return;
            }
            0x44 | 0x10e | 0x110 | 0x18a | 0x1e0a | 0x1e0c | 0x1e0e | 0x1e10 | 0x1e12 => {
                emit2(b'D' as i32); emit2(0x10e); emit2(0x110); emit2(0x18a);
                emit2(0x1e0a); emit2(0x1e0c); emit2(0x1e0e);
                emit2(0x1e10); emit2(0x1e12);
                return;
            }
            0x45 | 0xc8 | 0xc9 | 0xca | 0xcb | 0x112 | 0x114 | 0x116 | 0x118 | 0x11a | 0x204
            | 0x206 | 0x228 | 0x246 | 0x1e14 | 0x1e16 | 0x1e18 | 0x1e1a | 0x1e1c | 0x1eb8
            | 0x1eba | 0x1ebc | 0x1ebe | 0x1ec0 | 0x1ec2 | 0x1ec4 | 0x1ec6 => {
                emit2(b'E' as i32); emit2(0xc8); emit2(0xc9);
                emit2(0xca); emit2(0xcb);
                emit2(0x112); emit2(0x114); emit2(0x116);
                emit2(0x118); emit2(0x11a); emit2(0x204);
                emit2(0x206); emit2(0x228); emit2(0x246);
                emit2(0x1e14); emit2(0x1e16); emit2(0x1e18);
                emit2(0x1e1a); emit2(0x1e1c); emit2(0x1eb8);
                emit2(0x1eba); emit2(0x1ebc); emit2(0x1ebe);
                emit2(0x1ec0); emit2(0x1ec2); emit2(0x1ec4);
                emit2(0x1ec6);
                return;
            }
            0x46 | 0x191 | 0x1e1e | 0xa798 => {
                emit2(b'F' as i32); emit2(0x191); emit2(0x1e1e); emit2(0xa798);
                return;
            }
            0x47 | 0x11c | 0x11e | 0x120 | 0x122 | 0x193 | 0x1e4 | 0x1e6 | 0x1f4 | 0x1e20
            | 0xa7a0 => {
                emit2(b'G' as i32); emit2(0x11c); emit2(0x11e); emit2(0x120);
                emit2(0x122); emit2(0x193); emit2(0x1e4);
                emit2(0x1e6); emit2(0x1f4); emit2(0x1e20);
                emit2(0xa7a0);
                return;
            }
            0x48 | 0x124 | 0x126 | 0x21e | 0x1e22 | 0x1e24 | 0x1e26 | 0x1e28 | 0x1e2a | 0x2c67 => {
                emit2(b'H' as i32); emit2(0x124); emit2(0x126); emit2(0x21e);
                emit2(0x1e22); emit2(0x1e24); emit2(0x1e26);
                emit2(0x1e28); emit2(0x1e2a); emit2(0x2c67);
                return;
            }
            0x49 | 0xcc | 0xcd | 0xce | 0xcf | 0x128 | 0x12a | 0x12c | 0x12e | 0x130 | 0x197
            | 0x1cf | 0x208 | 0x20a | 0x1e2c | 0x1e2e | 0x1ec8 | 0x1eca => {
                emit2(b'I' as i32); emit2(0xcc); emit2(0xcd);
                emit2(0xce); emit2(0xcf);
                emit2(0x128); emit2(0x12a); emit2(0x12c);
                emit2(0x12e); emit2(0x130); emit2(0x197);
                emit2(0x1cf); emit2(0x208); emit2(0x20a);
                emit2(0x1e2c); emit2(0x1e2e); emit2(0x1ec8);
                emit2(0x1eca);
                return;
            }
            0x4a | 0x134 | 0x248 => {
                emit2(b'J' as i32); emit2(0x134); emit2(0x248);
                return;
            }
            0x4b | 0x136 | 0x198 | 0x1e8 | 0x1e30 | 0x1e32 | 0x1e34 | 0x2c69 | 0xa740 => {
                emit2(b'K' as i32); emit2(0x136); emit2(0x198); emit2(0x1e8);
                emit2(0x1e30); emit2(0x1e32); emit2(0x1e34);
                emit2(0x2c69); emit2(0xa740);
                return;
            }
            0x4c | 0x139 | 0x13b | 0x13d | 0x13f | 0x141 | 0x23d | 0x1e36 | 0x1e38 | 0x1e3a
            | 0x1e3c | 0x2c60 => {
                emit2(b'L' as i32); emit2(0x139); emit2(0x13b);
                emit2(0x13d); emit2(0x13f); emit2(0x141);
                emit2(0x23d); emit2(0x1e36); emit2(0x1e38);
                emit2(0x1e3a); emit2(0x1e3c); emit2(0x2c60);
                return;
            }
            0x4d | 0x1e3e | 0x1e40 | 0x1e42 => {
                emit2(b'M' as i32); emit2(0x1e3e); emit2(0x1e40);
                emit2(0x1e42);
                return;
            }
            0x4e | 0xd1 | 0x143 | 0x145 | 0x147 | 0x1f8 | 0x1e44 | 0x1e46 | 0x1e48 | 0x1e4a
            | 0xa7a4 => {
                emit2(b'N' as i32); emit2(0xd1);
                emit2(0x143); emit2(0x145); emit2(0x147);
                emit2(0x1f8); emit2(0x1e44); emit2(0x1e46);
                emit2(0x1e48); emit2(0x1e4a); emit2(0xa7a4);
                return;
            }
            0x4f | 0xd2 | 0xd3 | 0xd4 | 0xd5 | 0xd6 | 0xd8 | 0x14c | 0x14e | 0x150 | 0x19f
            | 0x1a0 | 0x1d1 | 0x1ea | 0x1ec | 0x1fe | 0x20c | 0x20e | 0x22a | 0x22c | 0x22e
            | 0x230 | 0x1e4c | 0x1e4e | 0x1e50 | 0x1e52 | 0x1ecc | 0x1ece | 0x1ed0 | 0x1ed2
            | 0x1ed4 | 0x1ed6 | 0x1ed8 | 0x1eda | 0x1edc | 0x1ede | 0x1ee0 | 0x1ee2 => {
                emit2(b'O' as i32); emit2(0xd2); emit2(0xd3);
                emit2(0xd4); emit2(0xd5);
                emit2(0xd6); emit2(0xd8);
                emit2(0x14c); emit2(0x14e); emit2(0x150);
                emit2(0x19f); emit2(0x1a0); emit2(0x1d1);
                emit2(0x1ea); emit2(0x1ec); emit2(0x1fe);
                emit2(0x20c); emit2(0x20e); emit2(0x22a);
                emit2(0x22c); emit2(0x22e); emit2(0x230);
                emit2(0x1e4c); emit2(0x1e4e); emit2(0x1e50);
                emit2(0x1e52); emit2(0x1ecc); emit2(0x1ece);
                emit2(0x1ed0); emit2(0x1ed2); emit2(0x1ed4);
                emit2(0x1ed6); emit2(0x1ed8); emit2(0x1eda);
                emit2(0x1edc); emit2(0x1ede); emit2(0x1ee0);
                emit2(0x1ee2);
                return;
            }
            0x50 | 0x1a4 | 0x1e54 | 0x1e56 | 0x2c63 => {
                emit2(b'P' as i32); emit2(0x1a4); emit2(0x1e54); emit2(0x1e56);
                emit2(0x2c63);
                return;
            }
            0x51 | 0x24a => {
                emit2(b'Q' as i32); emit2(0x24a);
                return;
            }
            0x52 | 0x154 | 0x156 | 0x158 | 0x210 | 0x212 | 0x24c | 0x1e58 | 0x1e5a | 0x1e5c
            | 0x1e5e | 0x2c64 | 0xa7a6 => {
                emit2(b'R' as i32); emit2(0x154); emit2(0x156); emit2(0x158);
                emit2(0x210); emit2(0x212); emit2(0x24c); emit2(0x1e58);
                emit2(0x1e5a); emit2(0x1e5c); emit2(0x1e5e); emit2(0x2c64);
                emit2(0xa7a6);
                return;
            }
            0x53 | 0x15a | 0x15c | 0x15e | 0x160 | 0x218 | 0x1e60 | 0x1e62 | 0x1e64 | 0x1e66
            | 0x1e68 | 0x2c7e | 0xa7a8 => {
                emit2(b'S' as i32); emit2(0x15a); emit2(0x15c); emit2(0x15e);
                emit2(0x160); emit2(0x218); emit2(0x1e60); emit2(0x1e62);
                emit2(0x1e64); emit2(0x1e66); emit2(0x1e68); emit2(0x2c7e);
                emit2(0xa7a8);
                return;
            }
            0x54 | 0x162 | 0x164 | 0x166 | 0x1ac | 0x1ae | 0x21a | 0x23e | 0x1e6a | 0x1e6c
            | 0x1e6e | 0x1e70 => {
                emit2(b'T' as i32); emit2(0x162); emit2(0x164); emit2(0x166);
                emit2(0x1ac); emit2(0x1ae); emit2(0x23e); emit2(0x21a);
                emit2(0x1e6a); emit2(0x1e6c); emit2(0x1e6e); emit2(0x1e70);
                return;
            }
            0x55 | 0xd9 | 0xda | 0xdb | 0xdc | 0x168 | 0x16a | 0x16c | 0x16e | 0x170 | 0x172
            | 0x1af | 0x1d3 | 0x1d5 | 0x1d7 | 0x1d9 | 0x1db | 0x214 | 0x216 | 0x244 | 0x1e72
            | 0x1e74 | 0x1e76 | 0x1e78 | 0x1e7a | 0x1ee4 | 0x1ee6 | 0x1ee8 | 0x1eea | 0x1eec
            | 0x1eee | 0x1ef0 => {
                emit2(b'U' as i32); emit2(0xd9); emit2(0xda);
                emit2(0xdc); emit2(0xdb);
                emit2(0x168); emit2(0x16a);
                emit2(0x16c); emit2(0x16e); emit2(0x170);
                emit2(0x172); emit2(0x1af); emit2(0x1d3);
                emit2(0x1d5); emit2(0x1d7); emit2(0x1d9);
                emit2(0x1db); emit2(0x214); emit2(0x216);
                emit2(0x244); emit2(0x1e72); emit2(0x1e74);
                emit2(0x1e76); emit2(0x1e78); emit2(0x1e7a);
                emit2(0x1ee4); emit2(0x1ee6); emit2(0x1ee8);
                emit2(0x1eea); emit2(0x1eec); emit2(0x1eee);
                emit2(0x1ef0);
                return;
            }
            0x56 | 0x1b2 | 0x1e7c | 0x1e7e => {
                emit2(b'V' as i32); emit2(0x1b2); emit2(0x1e7c); emit2(0x1e7e);
                return;
            }
            0x57 | 0x174 | 0x1e80 | 0x1e82 | 0x1e84 | 0x1e86 | 0x1e88 => {
                emit2(b'W' as i32); emit2(0x174); emit2(0x1e80); emit2(0x1e82);
                emit2(0x1e84); emit2(0x1e86); emit2(0x1e88);
                return;
            }
            0x58 | 0x1e8a | 0x1e8c => {
                emit2(b'X' as i32); emit2(0x1e8a); emit2(0x1e8c);
                return;
            }
            0x59 | 0xdd | 0x176 | 0x178 | 0x1b3 | 0x232 | 0x24e | 0x1e8e | 0x1ef2 | 0x1ef4
            | 0x1ef6 | 0x1ef8 => {
                emit2(b'Y' as i32); emit2(0xdd);
                emit2(0x176); emit2(0x178); emit2(0x1b3);
                emit2(0x232); emit2(0x24e); emit2(0x1e8e);
                emit2(0x1ef2); emit2(0x1ef4); emit2(0x1ef6);
                emit2(0x1ef8);
                return;
            }
            0x5a | 0x179 | 0x17b | 0x17d | 0x1b5 | 0x1e90 | 0x1e92 | 0x1e94 | 0x2c6b => {
                emit2(b'Z' as i32); emit2(0x179); emit2(0x17b); emit2(0x17d);
                emit2(0x1b5); emit2(0x1e90); emit2(0x1e92);
                emit2(0x1e94); emit2(0x2c6b);
                return;
            }
            0x61 | 0xe0 | 0xe1 | 0xe2 | 0xe3 | 0xe4 | 0xe5 | 0x101 | 0x103 | 0x105 | 0x1ce
            | 0x1df | 0x1e1 | 0x1fb | 0x201 | 0x203 | 0x227 | 0x1d8f | 0x1e01 | 0x1e9a
            | 0x1ea1 | 0x1ea3 | 0x1ea5 | 0x1ea7 | 0x1ea9 | 0x1eab | 0x1ead | 0x1eaf | 0x1eb1
            | 0x1eb3 | 0x1eb5 | 0x1eb7 | 0x2c65 => {
                emit2(b'a' as i32); emit2(0xe0); emit2(0xe1);
                emit2(0xe2); emit2(0xe3);
                emit2(0xe4); emit2(0xe5);
                emit2(0x101); emit2(0x103); emit2(0x105);
                emit2(0x1ce); emit2(0x1df); emit2(0x1e1);
                emit2(0x1fb); emit2(0x201); emit2(0x203);
                emit2(0x227); emit2(0x1d8f); emit2(0x1e01);
                emit2(0x1e9a); emit2(0x1ea1); emit2(0x1ea3);
                emit2(0x1ea5); emit2(0x1ea7); emit2(0x1ea9);
                emit2(0x1eab); emit2(0x1ead); emit2(0x1eaf);
                emit2(0x1eb1); emit2(0x1eb3); emit2(0x1eb5);
                emit2(0x1eb7); emit2(0x2c65);
                return;
            }
            0x62 | 0x180 | 0x253 | 0x1d6c | 0x1d80 | 0x1e03 | 0x1e05 | 0x1e07 => {
                emit2(b'b' as i32); emit2(0x180); emit2(0x253); emit2(0x1d6c);
                emit2(0x1d80); emit2(0x1e03); emit2(0x1e05); emit2(0x1e07);
                return;
            }
            0x63 | 0xe7 | 0x107 | 0x109 | 0x10b | 0x10d | 0x188 | 0x23c | 0x1e09 | 0xa793
            | 0xa794 => {
                emit2(b'c' as i32); emit2(0xe7);
                emit2(0x107); emit2(0x109); emit2(0x10b);
                emit2(0x10d); emit2(0x188); emit2(0x23c);
                emit2(0x1e09); emit2(0xa793); emit2(0xa794);
                return;
            }
            0x64 | 0x10f | 0x111 | 0x257 | 0x1d6d | 0x1d81 | 0x1d91 | 0x1e0b | 0x1e0d | 0x1e0f
            | 0x1e11 | 0x1e13 => {
                emit2(b'd' as i32); emit2(0x10f); emit2(0x111);
                emit2(0x257); emit2(0x1d6d); emit2(0x1d81);
                emit2(0x1d91); emit2(0x1e0b); emit2(0x1e0d);
                emit2(0x1e0f); emit2(0x1e11); emit2(0x1e13);
                return;
            }
            0x65 | 0xe8 | 0xe9 | 0xea | 0xeb | 0x113 | 0x115 | 0x117 | 0x119 | 0x11b | 0x205
            | 0x207 | 0x229 | 0x247 | 0x1d92 | 0x1e15 | 0x1e17 | 0x1e19 | 0x1e1b | 0x1e1d
            | 0x1eb9 | 0x1ebb | 0x1ebd | 0x1ebf | 0x1ec1 | 0x1ec3 | 0x1ec5 | 0x1ec7 => {
                emit2(b'e' as i32); emit2(0xe8); emit2(0xe9);
                emit2(0xea); emit2(0xeb);
                emit2(0x113); emit2(0x115);
                emit2(0x117); emit2(0x119); emit2(0x11b);
                emit2(0x205); emit2(0x207); emit2(0x229);
                emit2(0x247); emit2(0x1d92); emit2(0x1e15);
                emit2(0x1e17); emit2(0x1e19); emit2(0x1e1b);
                emit2(0x1e1d); emit2(0x1eb9); emit2(0x1ebb);
                emit2(0x1ebd); emit2(0x1ebf); emit2(0x1ec1);
                emit2(0x1ec3); emit2(0x1ec5); emit2(0x1ec7);
                return;
            }
            0x66 | 0x192 | 0x1d6e | 0x1d82 | 0x1e1f | 0xa799 => {
                emit2(b'f' as i32); emit2(0x192); emit2(0x1d6e); emit2(0x1d82);
                emit2(0x1e1f); emit2(0xa799);
                return;
            }
            0x67 | 0x11d | 0x11f | 0x121 | 0x123 | 0x1e5 | 0x1e7 | 0x1f5 | 0x260 | 0x1d83
            | 0x1e21 | 0xa7a1 => {
                emit2(b'g' as i32); emit2(0x11d); emit2(0x11f); emit2(0x121);
                emit2(0x123); emit2(0x1e5); emit2(0x1e7);
                emit2(0x1f5); emit2(0x260); emit2(0x1d83);
                emit2(0x1e21); emit2(0xa7a1);
                return;
            }
            0x68 | 0x125 | 0x127 | 0x21f | 0x1e23 | 0x1e25 | 0x1e27 | 0x1e29 | 0x1e2b | 0x1e96
            | 0x2c68 | 0xa795 => {
                emit2(b'h' as i32); emit2(0x125); emit2(0x127); emit2(0x21f);
                emit2(0x1e23); emit2(0x1e25); emit2(0x1e27);
                emit2(0x1e29); emit2(0x1e2b); emit2(0x1e96);
                emit2(0x2c68); emit2(0xa795);
                return;
            }
            0x69 | 0xec | 0xed | 0xee | 0xef | 0x129 | 0x12b | 0x12d | 0x12f | 0x1d0 | 0x209
            | 0x20b | 0x268 | 0x1d96 | 0x1e2d | 0x1e2f | 0x1ec9 | 0x1ecb => {
                emit2(b'i' as i32); emit2(0xec); emit2(0xed);
                emit2(0xee); emit2(0xef);
                emit2(0x129); emit2(0x12b); emit2(0x12d);
                emit2(0x12f); emit2(0x1d0); emit2(0x209);
                emit2(0x20b); emit2(0x268); emit2(0x1d96);
                emit2(0x1e2d); emit2(0x1e2f); emit2(0x1ec9);
                emit2(0x1ecb);
                return;
            }
            0x6a | 0x135 | 0x1f0 | 0x249 => {
                emit2(b'j' as i32); emit2(0x135); emit2(0x1f0); emit2(0x249);
                return;
            }
            0x6b | 0x137 | 0x199 | 0x1e9 | 0x1d84 | 0x1e31 | 0x1e33 | 0x1e35 | 0x2c6a | 0xa741 => {
                emit2(b'k' as i32); emit2(0x137); emit2(0x199); emit2(0x1e9);
                emit2(0x1d84); emit2(0x1e31); emit2(0x1e33);
                emit2(0x1e35); emit2(0x2c6a); emit2(0xa741);
                return;
            }
            0x6c | 0x13a | 0x13c | 0x13e | 0x140 | 0x142 | 0x19a | 0x1e37 | 0x1e39 | 0x1e3b
            | 0x1e3d | 0x2c61 => {
                emit2(b'l' as i32); emit2(0x13a); emit2(0x13c);
                emit2(0x13e); emit2(0x140); emit2(0x142);
                emit2(0x19a); emit2(0x1e37); emit2(0x1e39);
                emit2(0x1e3b); emit2(0x1e3d); emit2(0x2c61);
                return;
            }
            0x6d | 0x1d6f | 0x1e3f | 0x1e41 | 0x1e43 => {
                emit2(b'm' as i32); emit2(0x1d6f); emit2(0x1e3f);
                emit2(0x1e41); emit2(0x1e43);
                return;
            }
            0x6e | 0xf1 | 0x144 | 0x146 | 0x148 | 0x149 | 0x1f9 | 0x1d70 | 0x1d87 | 0x1e45
            | 0x1e47 | 0x1e49 | 0x1e4b | 0xa7a5 => {
                emit2(b'n' as i32); emit2(0xf1);
                emit2(0x144); emit2(0x146); emit2(0x148);
                emit2(0x149); emit2(0x1f9); emit2(0x1d70);
                emit2(0x1d87); emit2(0x1e45); emit2(0x1e47);
                emit2(0x1e49); emit2(0x1e4b); emit2(0xa7a5);
                return;
            }
            0x6f | 0xf2 | 0xf3 | 0xf4 | 0xf5 | 0xf6 | 0xf8 | 0x14d | 0x14f | 0x151 | 0x1a1
            | 0x1d2 | 0x1eb | 0x1ed | 0x1ff | 0x20d | 0x20f | 0x22b | 0x22d | 0x22f | 0x231
            | 0x275 | 0x1e4d | 0x1e4f | 0x1e51 | 0x1e53 | 0x1ecd | 0x1ecf | 0x1ed1 | 0x1ed3
            | 0x1ed5 | 0x1ed7 | 0x1ed9 | 0x1edb | 0x1edd | 0x1edf | 0x1ee1 | 0x1ee3 => {
                emit2(b'o' as i32); emit2(0xf2); emit2(0xf3);
                emit2(0xf4); emit2(0xf5);
                emit2(0xf6); emit2(0xf8);
                emit2(0x14d); emit2(0x14f); emit2(0x151);
                emit2(0x1a1); emit2(0x1d2); emit2(0x1eb);
                emit2(0x1ed); emit2(0x1ff); emit2(0x20d);
                emit2(0x20f); emit2(0x22b); emit2(0x22d);
                emit2(0x22f); emit2(0x231); emit2(0x275);
                emit2(0x1e4d); emit2(0x1e4f); emit2(0x1e51);
                emit2(0x1e53); emit2(0x1ecd); emit2(0x1ecf);
                emit2(0x1ed1); emit2(0x1ed3); emit2(0x1ed5);
                emit2(0x1ed7); emit2(0x1ed9); emit2(0x1edb);
                emit2(0x1edd); emit2(0x1edf); emit2(0x1ee1);
                emit2(0x1ee3);
                return;
            }
            0x70 | 0x1a5 | 0x1d71 | 0x1d7d | 0x1d88 | 0x1e55 | 0x1e57 => {
                emit2(b'p' as i32); emit2(0x1a5); emit2(0x1d71); emit2(0x1d7d);
                emit2(0x1d88); emit2(0x1e55); emit2(0x1e57);
                return;
            }
            0x71 | 0x24b | 0x2a0 => {
                emit2(b'q' as i32); emit2(0x24b); emit2(0x2a0);
                return;
            }
            0x72 | 0x155 | 0x157 | 0x159 | 0x211 | 0x213 | 0x24d | 0x27d | 0x1d72 | 0x1d73
            | 0x1d89 | 0x1e59 | 0x1e5b | 0x1e5d | 0x1e5f | 0xa7a7 => {
                emit2(b'r' as i32); emit2(0x155); emit2(0x157); emit2(0x159);
                emit2(0x211); emit2(0x213); emit2(0x24d); emit2(0x27d);
                emit2(0x1d72); emit2(0x1d73); emit2(0x1d89); emit2(0x1e59);
                emit2(0x1e5b); emit2(0x1e5d); emit2(0x1e5f); emit2(0xa7a7);
                return;
            }
            0x73 | 0x15b | 0x15d | 0x15f | 0x161 | 0x219 | 0x23f | 0x1d74 | 0x1d8a | 0x1e61
            | 0x1e63 | 0x1e65 | 0x1e67 | 0x1e69 | 0xa7a9 => {
                emit2(b's' as i32); emit2(0x15b); emit2(0x15d); emit2(0x15f);
                emit2(0x161); emit2(0x219); emit2(0x23f); emit2(0x1d74);
                emit2(0x1d8a); emit2(0x1e61); emit2(0x1e63); emit2(0x1e65);
                emit2(0x1e67); emit2(0x1e69); emit2(0xa7a9);
                return;
            }
            0x74 | 0x163 | 0x165 | 0x167 | 0x1ab | 0x1ad | 0x21b | 0x288 | 0x1d75 | 0x1e6b
            | 0x1e6d | 0x1e6f | 0x1e71 | 0x1e97 | 0x2c66 => {
                emit2(b't' as i32); emit2(0x163); emit2(0x165); emit2(0x167);
                emit2(0x1ab); emit2(0x1ad); emit2(0x21b); emit2(0x288);
                emit2(0x1d75); emit2(0x1e6b); emit2(0x1e6d); emit2(0x1e6f);
                emit2(0x1e71); emit2(0x1e97); emit2(0x2c66);
                return;
            }
            0x75 | 0xf9 | 0xfa | 0xfb | 0xfc | 0x169 | 0x16b | 0x16d | 0x16f | 0x171 | 0x173
            | 0x1b0 | 0x1d4 | 0x1d6 | 0x1d8 | 0x1da | 0x1dc | 0x215 | 0x217 | 0x289 | 0x1d7e
            | 0x1d99 | 0x1e73 | 0x1e75 | 0x1e77 | 0x1e79 | 0x1e7b | 0x1ee5 | 0x1ee7 | 0x1ee9
            | 0x1eeb | 0x1eed | 0x1eef | 0x1ef1 => {
                emit2(b'u' as i32); emit2(0xf9); emit2(0xfa);
                emit2(0xfb); emit2(0xfc);
                emit2(0x169); emit2(0x16b);
                emit2(0x16d); emit2(0x16f); emit2(0x171);
                emit2(0x173); emit2(0x1d6); emit2(0x1d8);
                emit2(0x215); emit2(0x217); emit2(0x1b0);
                emit2(0x1d4); emit2(0x1da); emit2(0x1dc);
                emit2(0x289); emit2(0x1e73); emit2(0x1d7e);
                emit2(0x1d99); emit2(0x1e75); emit2(0x1e77);
                emit2(0x1e79); emit2(0x1e7b); emit2(0x1ee5);
                emit2(0x1ee7); emit2(0x1ee9); emit2(0x1eeb);
                emit2(0x1eed); emit2(0x1eef); emit2(0x1ef1);
                return;
            }
            0x76 | 0x28b | 0x1d8c | 0x1e7d | 0x1e7f => {
                emit2(b'v' as i32); emit2(0x28b); emit2(0x1d8c); emit2(0x1e7d);
                emit2(0x1e7f);
                return;
            }
            0x77 | 0x175 | 0x1e81 | 0x1e83 | 0x1e85 | 0x1e87 | 0x1e89 | 0x1e98 => {
                emit2(b'w' as i32); emit2(0x175); emit2(0x1e81); emit2(0x1e83);
                emit2(0x1e85); emit2(0x1e87); emit2(0x1e89); emit2(0x1e98);
                return;
            }
            0x78 | 0x1e8b | 0x1e8d => {
                emit2(b'x' as i32); emit2(0x1e8b); emit2(0x1e8d);
                return;
            }
            0x79 | 0xfd | 0xff | 0x177 | 0x1b4 | 0x233 | 0x24f | 0x1e8f | 0x1e99 | 0x1ef3
            | 0x1ef5 | 0x1ef7 | 0x1ef9 => {
                emit2(b'y' as i32); emit2(0xfd); emit2(0xff);
                emit2(0x177); emit2(0x1b4); emit2(0x233); emit2(0x24f);
                emit2(0x1e8f); emit2(0x1e99); emit2(0x1ef3);
                emit2(0x1ef5); emit2(0x1ef7); emit2(0x1ef9);
                return;
            }
            0x7a | 0x17a | 0x17c | 0x17e | 0x1b6 | 0x1d76 | 0x1d8e | 0x1e91 | 0x1e93 | 0x1e95
            | 0x2c6c => {
                emit2(b'z' as i32); emit2(0x17a); emit2(0x17c); emit2(0x17e);
                emit2(0x1b6); emit2(0x1d76); emit2(0x1d8e); emit2(0x1e91);
                emit2(0x1e93); emit2(0x1e95); emit2(0x2c6c);
                return;
            }
            _ => {}
        }
        emit2(c);
}

fn nfa_regatom() -> bool {
    let mut old_regparse = re_st().regparse.cast::<u8>();
    let mut extra: i32 = 0;
    let mut startc: i32 = -1;
    let save_prev_at_start = re_st().prev_at_start;

    let mut c = getchr();
    let mut goto_collection = false;
    let mut goto_multibyte = false;

    if c == NUL as i32 {
        emsg(tr(E_NUL_FOUND));
        g_st().rc_did_emsg = true;
        return false;
    } else if c == magic_fn(b'^' as i32) {
        emit_fn(NFA_BOL);
        return true;
    } else if c == magic_fn(b'$' as i32) {
        emit_fn(NFA_EOL);
        re_st().had_eol = 1;
        return true;
    } else if c == magic_fn(b'<' as i32) {
        emit_fn(NFA_BOW);
        return true;
    } else if c == magic_fn(b'>' as i32) {
        emit_fn(NFA_EOW);
        return true;
    } else if c == magic_fn(b'_' as i32) {
        c = no_magic(getchr());
        if c == NUL as i32 {
            emsg(tr(E_NUL_FOUND));
            g_st().rc_did_emsg = true;
            return false;
        }
        if c == b'^' as i32 {
            emit_fn(NFA_BOL);
            return true;
        }
        if c == b'$' as i32 {
            emit_fn(NFA_EOL);
            re_st().had_eol = 1;
            return true;
        }
        extra = NFA_ADD_NL;
        if c == b'[' as i32 {
            goto_collection = true;
        } else {
            let p = vim_strchr(CLASSCHARS.as_ptr().cast(), no_magic(c)).cast::<u8>();
            if p.is_null() {
                if extra == NFA_ADD_NL {
                    semsg!(tr(E_ILL_CHAR_CLASS), i64::from(c));
                    g_st().rc_did_emsg = true;
                    return false;
                }
                siemsg!(c"INTERNAL: Unknown character class char: %d", c);
                return false;
            }
            if c == magic_fn(b'.' as i32) && utf_iscomposing_legacy(peekchr()) {
                old_regparse = re_st().regparse.cast();
                c = getchr();
                goto_multibyte = true;
            } else {
                let idx = unsafe { p.offset_from(CLASSCHARS.as_ptr()) } as usize;
                emit_fn(NFA_CLASSCODES[idx]);
                if extra == NFA_ADD_NL {
                    emit_fn(NFA_NEWL);
                    emit_fn(NFA_OR);
                    re_st().regflags |= RF_HASNL;
                }
                return true;
            }
        }
    }

    if !goto_collection && !goto_multibyte {
        if matches!(
            c,
            x if x == magic_fn(b'.' as i32)
                || x == magic_fn(b'i' as i32)
                || x == magic_fn(b'I' as i32)
                || x == magic_fn(b'k' as i32)
                || x == magic_fn(b'K' as i32)
                || x == magic_fn(b'f' as i32)
                || x == magic_fn(b'F' as i32)
                || x == magic_fn(b'p' as i32)
                || x == magic_fn(b'P' as i32)
                || x == magic_fn(b's' as i32)
                || x == magic_fn(b'S' as i32)
                || x == magic_fn(b'd' as i32)
                || x == magic_fn(b'D' as i32)
                || x == magic_fn(b'x' as i32)
                || x == magic_fn(b'X' as i32)
                || x == magic_fn(b'o' as i32)
                || x == magic_fn(b'O' as i32)
                || x == magic_fn(b'w' as i32)
                || x == magic_fn(b'W' as i32)
                || x == magic_fn(b'h' as i32)
                || x == magic_fn(b'H' as i32)
                || x == magic_fn(b'a' as i32)
                || x == magic_fn(b'A' as i32)
                || x == magic_fn(b'l' as i32)
                || x == magic_fn(b'L' as i32)
                || x == magic_fn(b'u' as i32)
                || x == magic_fn(b'U' as i32)
        ) {
            let p = vim_strchr(CLASSCHARS.as_ptr().cast(), no_magic(c)).cast::<u8>();
            if p.is_null() {
                if extra == NFA_ADD_NL {
                    semsg!(tr(E_ILL_CHAR_CLASS), i64::from(c));
                    g_st().rc_did_emsg = true;
                    return false;
                }
                siemsg!(c"INTERNAL: Unknown character class char: %d", c);
                return false;
            }
            if c == magic_fn(b'.' as i32) && utf_iscomposing_legacy(peekchr()) {
                old_regparse = re_st().regparse.cast();
                c = getchr();
            } else {
                let idx = unsafe { p.offset_from(CLASSCHARS.as_ptr()) } as usize;
                emit_fn(NFA_CLASSCODES[idx]);
                if extra == NFA_ADD_NL {
                    emit_fn(NFA_NEWL);
                    emit_fn(NFA_OR);
                    re_st().regflags |= RF_HASNL;
                }
                return true;
            }
        } else if c == magic_fn(b'n' as i32) {
            if re_st().reg_string != 0 {
                emit_fn(NL);
            } else {
                emit_fn(NFA_NEWL);
                re_st().regflags |= RF_HASNL;
            }
            return true;
        } else if c == magic_fn(b'(' as i32) {
            if !nfa_reg(REG_PAREN) {
                return false;
            }
            return true;
        } else if c == magic_fn(b'|' as i32)
            || c == magic_fn(b'&' as i32)
            || c == magic_fn(b')' as i32)
            || c == magic_fn(b'=' as i32)
            || c == magic_fn(b'?' as i32)
            || c == magic_fn(b'+' as i32)
            || c == magic_fn(b'@' as i32)
            || c == magic_fn(b'*' as i32)
            || c == magic_fn(b'{' as i32)
        {
            semsg!(tr(E_MISPLACED), no_magic(c));
            return false;
        } else if c == magic_fn(b'~' as i32) {
            let Some(prev_sub) = reg_prev_sub_ref() else {
                emsg(tr(E_NOPRESUB));
                return false;
            };
            let start_ptr = prev_sub.as_ptr().cast::<u8>();
            let mut lp = start_ptr;
            while unsafe { *lp } != NUL {
                emit_fn(utf_ptr2char(lp.cast()));
                if lp != start_ptr {
                    emit_fn(NFA_CONCAT);
                }
                lp = lp.wrapping_add(utf_ptr2len(lp.cast()) as usize);
            }
            emit_fn(NFA_NOPEN);
            return true;
        } else if (magic_fn(b'1' as i32)..=magic_fn(b'9' as i32)).contains(&c) {
            let refnum = no_magic(c) - b'1' as i32;
            if !seen_endbrace(refnum + 1) {
                return false;
            }
            emit_fn(NFA_BACKREF1 + refnum);
            re_st().rex.nfa_has_backref = 1;
            return true;
        } else if c == magic_fn(b'z' as i32) {
            c = no_magic(getchr());
            match c as u8 {
                b's' => {
                    emit_fn(NFA_ZSTART);
                    if !re_mult_next(c"\\zs") {
                        return false;
                    }
                    return true;
                }
                b'e' => {
                    emit_fn(NFA_ZEND);
                    re_st().rex.nfa_has_zend = 1;
                    if !re_mult_next(c"\\ze") {
                        return false;
                    }
                    return true;
                }
                b'1'..=b'9' => {
                    if (g_st().reg_do_extmatch & REX_USE) == 0 {
                        emsg(tr(E_Z1_NOT_ALLOWED));
                        g_st().rc_did_emsg = true;
                        return false;
                    }
                    emit_fn(NFA_ZREF1 + (no_magic(c) - b'1' as i32));
                    re_st().re_has_z = REX_USE;
                    return true;
                }
                b'(' => {
                    if g_st().reg_do_extmatch != REX_SET {
                        emsg(tr(E_Z_NOT_ALLOWED));
                        g_st().rc_did_emsg = true;
                        return false;
                    }
                    if !nfa_reg(REG_ZPAREN) {
                        return false;
                    }
                    re_st().re_has_z = REX_SET;
                    return true;
                }
                _ => {
                    semsg!(tr(c"E867: (NFA) Unknown operator '\\z%c'"), no_magic(c),);
                    return false;
                }
            }
        } else if c == magic_fn(b'%' as i32) {
            c = no_magic(getchr());
            if c == b'(' as i32 {
                if !nfa_reg(REG_NPAREN) {
                    return false;
                }
                emit_fn(NFA_NOPEN);
                return true;
            }
            return nfa_regatom_percent(c, save_prev_at_start);
        } else if c == magic_fn(b'[' as i32) {
            goto_collection = true;
        }
    }

    if goto_collection {
        let mut mb_c = c;
        let res = nfa_regatom_collection(&mut old_regparse, &mut extra, &mut startc, &mut mb_c);
        if res != -2 {
            return res == OK;
        }
        c = mb_c;
    }

    let plen = utfc_ptr2len(old_regparse.cast());
    if utf_char2len(c) != plen || utf_iscomposing_legacy(c) {
        let mut i: i32 = 0;
        loop {
            emit_fn(c);
            if i > 0 {
                emit_fn(NFA_CONCAT);
            }
            i += utf_char2len(c);
            if i >= plen {
                break;
            }
            c = utf_ptr2char(old_regparse.wrapping_add(i as usize).cast());
        }
        emit_fn(NFA_COMPOSING);
        re_st().regparse = old_regparse.wrapping_add(plen as usize).cast();
    } else {
        c = no_magic(c);
        emit_fn(c);
    }
    true
}

#[inline(never)]
fn nfa_regatom_percent(mut c: i32, save_prev_at_start: i32) -> bool {
    match c as u8 {
        b'd' | b'o' | b'x' | b'u' | b'U' => {
            let nr = match c as u8 {
                b'd' => getdecchrs(),
                b'o' => getoctchrs(),
                b'x' => gethexchrs(2),
                b'u' => gethexchrs(4),
                b'U' => gethexchrs(8),
                _ => -1,
            };
            if nr < 0 || nr > i64::from(i32::MAX) {
                semsg!(
                    tr(c"E678: Invalid character after %s%%[dxouU]"),
                    if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
                g_st().rc_did_emsg = true;
                return false;
            }
            emit_fn(if nr == 0 { 0x0a } else { nr as i32 });
            true
        }
        b'^' => {
            emit_fn(NFA_BOF);
            true
        }
        b'$' => {
            emit_fn(NFA_EOF);
            true
        }
        b'#' => {
            if unsafe { *re_st().regparse } == b'='
                && (b'0'..=b'2').contains(&unsafe { *re_st().regparse.wrapping_add(1) })
            {
                semsg!(
                    tr(E_ATOM_ENGINE_MUST_BE_AT_START_OF_PATTERN),
                    i32::from(unsafe { *re_st().regparse.wrapping_add(1) }),
                );
                return false;
            }
            emit_fn(NFA_CURSOR);
            true
        }
        b'V' => {
            emit_fn(NFA_VISUAL);
            true
        }
        b'C' => {
            emit_fn(NFA_ANY_COMPOSING);
            true
        }
        b'[' => {
            let mut n: i32 = 0;
            loop {
                c = peekchr();
                if c == b']' as i32 {
                    break;
                }
                if c == NUL as i32 {
                    semsg!(
                        tr(E_MISSING_SB),
                        if re_st().reg_magic == MAGIC_ALL {
                            c"".as_ptr()
                        } else {
                            c"\\".as_ptr()
                        },
                    );
                    g_st().rc_did_emsg = true;
                    return false;
                }
                if !nfa_regatom() {
                    return false;
                }
                n += 1;
            }
            let _ = getchr();
            if n == 0 {
                semsg!(
                    tr(E_EMPTY_SB),
                    if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
                g_st().rc_did_emsg = true;
                return false;
            }
            emit_fn(NFA_OPT_CHARS);
            emit_fn(n);
            emit_fn(NFA_NOPEN);
            true
        }
        _ => {
            let mut n: i64 = 0;
            let cmp = c;
            let mut cur = false;
            let mut got_digit = false;

            if c == b'<' as i32 || c == b'>' as i32 {
                c = getchr();
            }
            if no_magic(c) == b'.' as i32 {
                cur = true;
                c = getchr();
            }
            while ascii_isdigit(c) {
                if cur {
                    semsg!(tr(E_REGEXP_NUMBER_AFTER_DOT_POS_SEARCH_CHR), no_magic(c),);
                    return false;
                }
                if n > i64::from(i32::MAX - (c - b'0' as i32)) / 10 {
                    emsg(tr(E_VALUE_TOO_LARGE));
                    return false;
                }
                n = n * 10 + i64::from(c - b'0' as i32);
                c = getchr();
                got_digit = true;
            }
            if c == b'l' as i32 || c == b'c' as i32 || c == b'v' as i32 {
                let mut limit = i32::MAX;
                if !cur && !got_digit {
                    semsg!(tr(E_NFA_REGEXP_MISSING_VALUE_IN_CHR), no_magic(c));
                    return false;
                }
                if c == b'l' as i32 {
                    if cur {
                        n = i64::from(crate::globals::curwin_mut().w_cursor.lnum);
                    }
                    emit_fn(if cmp == b'<' as i32 {
                        NFA_LNUM_LT
                    } else if cmp == b'>' as i32 {
                        NFA_LNUM_GT
                    } else {
                        NFA_LNUM
                    });
                    if save_prev_at_start != 0 {
                        re_st().at_start = 1;
                    }
                } else if c == b'c' as i32 {
                    if cur {
                        n = i64::from(crate::globals::curwin_mut().w_cursor.col) + 1;
                    }
                    emit_fn(if cmp == b'<' as i32 {
                        NFA_COL_LT
                    } else if cmp == b'>' as i32 {
                        NFA_COL_GT
                    } else {
                        NFA_COL
                    });
                } else {
                    if cur {
                        let mut vcol: Colnr = 0;
                        getvvcol(
                            unsafe { &mut *(crate::globals::curwin_ptr()) },
                            &mut crate::globals::curwin_mut().w_cursor,
                            None,
                            None,
                            Some(&mut vcol),
                            0,
                        );
                        vcol += 1;
                        n = i64::from(vcol);
                    }
                    emit_fn(if cmp == b'<' as i32 {
                        NFA_VCOL_LT
                    } else if cmp == b'>' as i32 {
                        NFA_VCOL_GT
                    } else {
                        NFA_VCOL
                    });
                    limit = i32::MAX / MB_MAXBYTES;
                }
                if n >= i64::from(limit) {
                    emsg(tr(E_VALUE_TOO_LARGE));
                    return false;
                }
                emit_fn(n as i32);
                return true;
            } else if no_magic(c) == b'\'' as i32 && n == 0 {
                emit_fn(if cmp == b'<' as i32 {
                    NFA_MARK_LT
                } else if cmp == b'>' as i32 {
                    NFA_MARK_GT
                } else {
                    NFA_MARK
                });
                emit_fn(getchr());
                return true;
            }
            semsg!(tr(c"E867: (NFA) Unknown operator '\\%%%c'"), no_magic(c),);
            false
        }
    }
}

#[inline(never)]
fn nfa_regatom_collection(
    old_regparse: &mut *mut u8,
    extra: &mut i32,
    startc: &mut i32,
    c: &mut i32,
) -> i32 {
    let p = re_st().regparse;
    let endp = skip_anyof(p).cast_const();
    if unsafe { *endp } == b']' {
        let result = nfa_recognize_char_class(
            re_st().regparse,
            endp,
            i32::from(*extra == NFA_ADD_NL),
        );
        if result != FAIL {
            if (NFA_FIRST_NL..=NFA_LAST_NL).contains(&result) {
                emit_fn(result - NFA_ADD_NL);
                emit_fn(NFA_NEWL);
                emit_fn(NFA_OR);
            } else {
                emit_fn(result);
            }
            re_st().regparse = endp.cast_mut();
            mb_ptr_adv(&mut re_st().regparse);
            return OK;
        }

        let mut negated = false;
        if unsafe { *re_st().regparse } == b'^' {
            negated = true;
            mb_ptr_adv(&mut re_st().regparse);
            emit_fn(NFA_START_NEG_COLL);
        } else {
            emit_fn(NFA_START_COLL);
        }
        if unsafe { *re_st().regparse } == b'-' {
            *startc = b'-' as i32;
            emit_fn(*startc);
            emit_fn(NFA_CONCAT);
            mb_ptr_adv(&mut re_st().regparse);
        }

        let mut emit_range = false;
        while (re_st().regparse as *const u8) < endp {
            let oldstartc = *startc;
            let mut range_endpoint = false;
            *startc = -1;
            let mut got_coll_char = false;

            if unsafe { *re_st().regparse } == b'[' {
                let mut equiclass: i32 = 0;
                let mut collclass: i32 = 0;
                let charclass = get_char_class(&raw mut re_st().regparse);
                if charclass == CLASS_NONE {
                    equiclass = get_equi_class(&raw mut re_st().regparse);
                    if equiclass == 0 {
                        collclass = get_coll_element(&raw mut re_st().regparse);
                    }
                }

                if charclass != CLASS_NONE {
                    match charclass {
                        CLASS_ALNUM => emit_fn(NFA_CLASS_ALNUM),
                        CLASS_ALPHA => emit_fn(NFA_CLASS_ALPHA),
                        CLASS_BLANK => emit_fn(NFA_CLASS_BLANK),
                        CLASS_CNTRL => emit_fn(NFA_CLASS_CNTRL),
                        CLASS_DIGIT => emit_fn(NFA_CLASS_DIGIT),
                        CLASS_GRAPH => emit_fn(NFA_CLASS_GRAPH),
                        CLASS_LOWER => {
                            re_st().wants_nfa = true;
                            emit_fn(NFA_CLASS_LOWER);
                        }
                        CLASS_PRINT => emit_fn(NFA_CLASS_PRINT),
                        CLASS_PUNCT => emit_fn(NFA_CLASS_PUNCT),
                        CLASS_SPACE => emit_fn(NFA_CLASS_SPACE),
                        CLASS_UPPER => {
                            re_st().wants_nfa = true;
                            emit_fn(NFA_CLASS_UPPER);
                        }
                        CLASS_XDIGIT => emit_fn(NFA_CLASS_XDIGIT),
                        CLASS_TAB => emit_fn(NFA_CLASS_TAB),
                        CLASS_RETURN => emit_fn(NFA_CLASS_RETURN),
                        CLASS_BACKSPACE => emit_fn(NFA_CLASS_BACKSPACE),
                        CLASS_ESCAPE => emit_fn(NFA_CLASS_ESCAPE),
                        CLASS_IDENT => emit_fn(NFA_CLASS_IDENT),
                        CLASS_KEYWORD => emit_fn(NFA_CLASS_KEYWORD),
                        CLASS_FNAME => emit_fn(NFA_CLASS_FNAME),
                        _ => {}
                    }
                    emit_fn(NFA_CONCAT);
                    continue;
                }
                if equiclass != 0 {
                    nfa_emit_equi_class(equiclass);
                    continue;
                }
                if collclass != 0 {
                    *startc = collclass;
                }
            }

            if unsafe { *re_st().regparse } == b'-' && oldstartc != -1 {
                emit_range = true;
                *startc = oldstartc;
                mb_ptr_adv(&mut re_st().regparse);
                continue;
            }

            if unsafe { *re_st().regparse } == b'\\'
                && (re_st().regparse.wrapping_add(1) as *const u8) <= endp
                && (crate::strings::vim_haschr(
                    REGEXP_INRANGE,
                    i32::from(unsafe { *re_st().regparse.wrapping_add(1) }),
                ) || (re_st().reg_cpo_lit == 0
                    && crate::strings::vim_haschr(
                        REGEXP_ABBR,
                        i32::from(unsafe { *re_st().regparse.wrapping_add(1) }),
                    )))
            {
                mb_ptr_adv(&mut re_st().regparse);
                if unsafe { *re_st().regparse } == b'n' {
                    *startc = if re_st().reg_string != 0
                        || emit_range
                        || unsafe { *re_st().regparse.wrapping_add(1) } == b'-'
                    {
                        NL
                    } else {
                        NFA_NEWL
                    };
                } else if matches!(
                    unsafe { *re_st().regparse },
                    b'd' | b'o' | b'x' | b'u' | b'U'
                ) {
                    *startc = coll_get_char();
                    if *startc == i32::MAX {
                        emsg(tr(E_UNICODE_VAL_TOO_LARGE));
                        g_st().rc_did_emsg = true;
                        return FAIL;
                    }
                    got_coll_char = true;
                    mb_ptr_back(*old_regparse, &raw mut re_st().regparse);
                } else {
                    *startc = backslash_trans(i32::from(unsafe { *re_st().regparse }));
                }
            }

            if *startc == -1 {
                *startc = utf_ptr2char(re_st().regparse.cast());
            }

            if emit_range {
                let endc = *startc;
                range_endpoint = true;
                *startc = oldstartc;
                if *startc > endc {
                    emsg(tr(E_REVERSE_RANGE));
                    g_st().rc_did_emsg = true;
                    return FAIL;
                }
                if endc > *startc + 2 {
                    if *startc == 0 {
                        emit_fn(1);
                    } else {
                        re_st().post_vec.pop();
                    }
                    emit_fn(endc);
                    emit_fn(NFA_RANGE);
                    emit_fn(NFA_CONCAT);
                } else {
                    for ch in (*startc + 1)..=endc {
                        emit_fn(ch);
                        emit_fn(NFA_CONCAT);
                    }
                }
                emit_range = false;
                *startc = -1;
            } else if *startc == NFA_NEWL {
                if !negated {
                    *extra = NFA_ADD_NL;
                }
            } else if got_coll_char && *startc == 0 {
                emit_fn(0x0a);
                emit_fn(NFA_CONCAT);
            } else {
                emit_fn(*startc);
                if utf_ptr2len(re_st().regparse.cast()) == utfc_ptr2len(re_st().regparse.cast()) {
                    emit_fn(NFA_CONCAT);
                }
            }

            let plen = utfc_ptr2len(re_st().regparse.cast());
            if !range_endpoint && utf_ptr2len(re_st().regparse.cast()) != plen {
                let mut i = utf_ptr2len(re_st().regparse.cast());
                *c = utf_ptr2char(re_st().regparse.wrapping_add(i as usize).cast());
                loop {
                    if *c == 0 {
                        emit_fn(1);
                    } else {
                        emit_fn(*c);
                    }
                    emit_fn(NFA_CONCAT);
                    i += utf_char2len(*c);
                    if i >= plen {
                        break;
                    }
                    *c = utf_ptr2char(re_st().regparse.wrapping_add(i as usize).cast());
                }
                emit_fn(NFA_COMPOSING);
                emit_fn(NFA_CONCAT);
            }
            mb_ptr_adv(&mut re_st().regparse);
        }

        mb_ptr_back(*old_regparse, &raw mut re_st().regparse);
        if unsafe { *re_st().regparse } == b'-' {
            emit_fn(b'-' as i32);
            emit_fn(NFA_CONCAT);
        }

        re_st().regparse = endp.cast_mut();
        mb_ptr_adv(&mut re_st().regparse);

        if negated {
            emit_fn(NFA_END_NEG_COLL);
        } else {
            emit_fn(NFA_END_COLL);
        }

        if *extra == NFA_ADD_NL {
            emit_fn(if re_st().reg_string != 0 {
                NL
            } else {
                NFA_NEWL
            });
            emit_fn(NFA_OR);
        }

        return OK;
    }

    if re_st().reg_strict != 0 {
        emsg(tr(E_MISSINGBRACKET));
        g_st().rc_did_emsg = true;
        return FAIL;
    }
    -2
}

fn nfa_regpiece() -> bool {
    let mut old_state = ParseState::new();

    save_parse_state(&mut old_state);
    let my_post_start = re_st().post_vec.len();

    if !nfa_regatom() {
        return false;
    }
    let op = peekchr();
    if re_multi_type(op) == NOT_MULTI {
        return true;
    }

    nfa_regpiece_multi(op, &mut old_state, my_post_start)
}

#[inline(never)]
fn nfa_regpiece_multi(op: i32, old_state: &mut ParseState, my_post_start: usize) -> bool {
    let mut minval: i32 = 0;
    let mut maxval: i32 = 0;
    let mut new_state = ParseState::new();
    skipchr();
    if op == magic_fn(b'*' as i32) {
        emit_fn(NFA_STAR);
    } else if op == magic_fn(b'+' as i32) {
        restore_parse_state(&mut *old_state);
        re_st().curchr = -1;
        if !nfa_regatom() {
            return false;
        }
        emit_fn(NFA_STAR);
        emit_fn(NFA_CONCAT);
        skipchr();
    } else if op == magic_fn(b'@' as i32) {
        let c2 = getdecchrs();
        let mut op2 = no_magic(getchr());
        let mut i: i32 = 0;
        match op2 as u8 {
            b'=' => i = NFA_PREV_ATOM_NO_WIDTH,
            b'!' => i = NFA_PREV_ATOM_NO_WIDTH_NEG,
            b'<' => {
                op2 = no_magic(getchr());
                if op2 == b'=' as i32 {
                    i = NFA_PREV_ATOM_JUST_BEFORE;
                } else if op2 == b'!' as i32 {
                    i = NFA_PREV_ATOM_JUST_BEFORE_NEG;
                }
            }
            b'>' => i = NFA_PREV_ATOM_LIKE_PATTERN,
            _ => {}
        }
        if i == 0 {
            semsg!(tr(c"E869: (NFA) Unknown operator '\\@%c'"), op2);
            return false;
        }
        emit_fn(i);
        if i == NFA_PREV_ATOM_JUST_BEFORE || i == NFA_PREV_ATOM_JUST_BEFORE_NEG {
            emit_fn(c2 as i32);
        }
    } else if op == magic_fn(b'?' as i32) || op == magic_fn(b'=' as i32) {
        emit_fn(NFA_QUEST);
    } else if op == magic_fn(b'{' as i32) {
        let mut greedy = true;
        let c2 = peekchr();
        if c2 == b'-' as i32 || c2 == magic_fn(b'-' as i32) {
            skipchr();
            greedy = false;
        }
        if !read_limits(&mut minval, &mut maxval) {
            emsg(tr(c"E870: (NFA regexp) Error reading repetition limits"));
            g_st().rc_did_emsg = true;
            return false;
        }
        if minval == 0 && maxval == MAX_LIMIT {
            if greedy {
                emit_fn(NFA_STAR);
            } else {
                emit_fn(NFA_STAR_NONGREEDY);
            }
        } else if maxval == 0 {
            re_st().post_vec.truncate(my_post_start);
            emit_fn(NFA_EMPTY);
            return true;
        } else {
            if (re_st().nfa_re_flags & RE_AUTO) != 0
                && (maxval > 500 || maxval > minval + 200)
                && (maxval != MAX_LIMIT && minval < 200)
                && !re_st().wants_nfa
            {
                return false;
            }
            re_st().post_vec.truncate(my_post_start);
            save_parse_state(&mut new_state);

            let quest = if greedy {
                NFA_QUEST
            } else {
                NFA_QUEST_NONGREEDY
            };
            for i in 0..maxval {
                restore_parse_state(&mut *old_state);
                let old_post_pos = re_st().post_vec.len();
                if !nfa_regatom() {
                    return false;
                }
                if i + 1 > minval {
                    if maxval == MAX_LIMIT {
                        if greedy {
                            emit_fn(NFA_STAR);
                        } else {
                            emit_fn(NFA_STAR_NONGREEDY);
                        }
                    } else {
                        emit_fn(quest);
                    }
                }
                if old_post_pos != my_post_start {
                    emit_fn(NFA_CONCAT);
                }
                if i + 1 > minval && maxval == MAX_LIMIT {
                    break;
                }
            }
            restore_parse_state(&mut new_state);
            re_st().curchr = -1;
        }
    }

    if re_multi_type(peekchr()) != NOT_MULTI {
        emsg(tr(c"E871: (NFA regexp) Can't have a multi follow a multi"));
        g_st().rc_did_emsg = true;
        return false;
    }

    true
}

fn nfa_regconcat() -> bool {
    let mut cont = true;
    let mut first = true;

    while cont {
        let p = peekchr();
        if p == NUL as i32
            || p == magic_fn(b'|' as i32)
            || p == magic_fn(b'&' as i32)
            || p == magic_fn(b')' as i32)
        {
            cont = false;
        } else if p == magic_fn(b'Z' as i32) {
            re_st().regflags |= RF_ICOMBINE;
            skipchr_keepstart();
        } else if p == magic_fn(b'c' as i32) {
            re_st().regflags |= RF_ICASE;
            skipchr_keepstart();
        } else if p == magic_fn(b'C' as i32) {
            re_st().regflags |= RF_NOICASE;
            skipchr_keepstart();
        } else if p == magic_fn(b'v' as i32) {
            re_st().reg_magic = MAGIC_ALL;
            skipchr_keepstart();
            re_st().curchr = -1;
        } else if p == magic_fn(b'm' as i32) {
            re_st().reg_magic = MAGIC_ON;
            skipchr_keepstart();
            re_st().curchr = -1;
        } else if p == magic_fn(b'M' as i32) {
            re_st().reg_magic = MAGIC_OFF;
            skipchr_keepstart();
            re_st().curchr = -1;
        } else if p == magic_fn(b'V' as i32) {
            re_st().reg_magic = MAGIC_NONE;
            skipchr_keepstart();
            re_st().curchr = -1;
        } else {
            if !nfa_regpiece() {
                return false;
            }
            if !first {
                emit_fn(NFA_CONCAT);
            } else {
                first = false;
            }
        }
    }

    true
}

fn nfa_regbranch() -> bool {
    let mut old_post_pos = re_st().post_vec.len();
    if !nfa_regconcat() {
        return false;
    }
    while peekchr() == magic_fn(b'&' as i32) {
        skipchr();
        if old_post_pos == re_st().post_vec.len() {
            emit_fn(NFA_EMPTY);
        }
        emit_fn(NFA_NOPEN);
        emit_fn(NFA_PREV_ATOM_NO_WIDTH);
        old_post_pos = re_st().post_vec.len();
        if !nfa_regconcat() {
            return false;
        }
        if old_post_pos == re_st().post_vec.len() {
            emit_fn(NFA_EMPTY);
        }
        emit_fn(NFA_CONCAT);
    }
    if old_post_pos == re_st().post_vec.len() {
        emit_fn(NFA_EMPTY);
    }
    true
}

fn nfa_reg(paren: i32) -> bool {
    let mut parno: i32 = 0;

    if paren == REG_PAREN {
        if re_st().regnpar >= NSUBEXP as i32 {
            emsg(tr(c"E872: (NFA regexp) Too many '('"));
            g_st().rc_did_emsg = true;
            return false;
        }
        parno = re_st().regnpar;
        re_st().regnpar += 1;
    } else if paren == REG_ZPAREN {
        if re_st().regnzpar >= NSUBEXP as i32 {
            emsg(tr(c"E879: (NFA regexp) Too many \\z("));
            g_st().rc_did_emsg = true;
            return false;
        }
        parno = re_st().regnzpar;
        re_st().regnzpar += 1;
    }

    if re_st().nfa_reg_parse_depth >= REG_MAX_PAREN_DEPTH {
        emsg(tr(E_COMMAND_TOO_COMPLEX));
        g_st().rc_did_emsg = true;
        return false;
    }
    re_st().nfa_reg_parse_depth += 1;

    if !nfa_regbranch() {
        re_st().nfa_reg_parse_depth -= 1;
        return false;
    }
    while peekchr() == magic_fn(b'|' as i32) {
        skipchr();
        if !nfa_regbranch() {
            re_st().nfa_reg_parse_depth -= 1;
            return false;
        }
        emit_fn(NFA_OR);
    }

    if paren != REG_NOPAREN && getchr() != magic_fn(b')' as i32) {
        if paren == REG_NPAREN {
            semsg!(
                tr(E_UNMATCHED_STR_PERCENT_OPEN),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
        } else {
            semsg!(
                tr(E_UNMATCHED_STR_OPEN),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
        }
        g_st().rc_did_emsg = true;
        re_st().nfa_reg_parse_depth -= 1;
        return false;
    } else if paren == REG_NOPAREN && peekchr() != NUL as i32 {
        if peekchr() == magic_fn(b')' as i32) {
            semsg!(
                tr(E_UNMATCHED_STR_CLOSE),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
        } else {
            emsg(tr(c"E873: (NFA regexp) proper termination error"));
        }
        g_st().rc_did_emsg = true;
        re_st().nfa_reg_parse_depth -= 1;
        return false;
    }

    if paren == REG_PAREN {
        unsafe {
            *(&raw mut re_st().had_endbrace)
                .cast::<u8>()
                .wrapping_add(parno as usize) = 1;
        }
        emit_fn(NFA_MOPEN + parno);
    } else if paren == REG_ZPAREN {
        emit_fn(NFA_ZOPEN + parno);
    }

    re_st().nfa_reg_parse_depth -= 1;
    true
}

fn re2post() -> bool {
    if !nfa_reg(REG_NOPAREN) {
        return false;
    }
    emit_fn(NFA_MOPEN);
    true
}

fn alloc_state(
    states: &mut [NfaState],
    c: i32,
    out: Option<usize>,
    out1: Option<usize>,
) -> Option<usize> {
    if re_st().istate >= re_st().nstate {
        return None;
    }
    let idx = re_st().istate as usize;
    re_st().istate += 1;
    states[idx] = NfaState {
        c,
        out,
        out1,
        val: 0,
        lastlist: [0; 2],
    };
    Some(idx)
}

#[inline]
fn frag(start: Option<usize>, out: Vec<NfaOutSlot>) -> Frag {
    Frag { start, out }
}

#[inline]
fn list1(slot: NfaOutSlot) -> Vec<NfaOutSlot> {
    vec![slot]
}

fn patch(states: &mut [NfaState], l: Vec<NfaOutSlot>, s: Option<usize>) {
    for slot in l {
        slot.set(states, s);
    }
}

fn append(mut l1: Vec<NfaOutSlot>, l2: Vec<NfaOutSlot>) -> Vec<NfaOutSlot> {
    l1.extend(l2);
    l1
}

fn st_error() {
    emsg(tr(c"E874: (NFA) Could not pop the stack!"));
}

fn nfa_max_width(states: &[NfaState], startstate: Option<usize>, depth: i32) -> i32 {
    let mut state = startstate;
    let mut len: i32 = 0;

    if depth > 4 {
        return -1;
    }

    while let Some(idx) = state {
        let st = states[idx];
        match st.c {
            NFA_END_INVISIBLE | NFA_END_INVISIBLE_NEG => return len,
            NFA_SPLIT => {
                let l = nfa_max_width(states, st.out, depth + 1);
                let r = nfa_max_width(states, st.out1, depth + 1);
                if l < 0 || r < 0 {
                    return -1;
                }
                return len + l.max(r);
            }
            NFA_ANY | NFA_START_COLL | NFA_START_NEG_COLL => {
                len += MB_MAXBYTES;
                if st.c != NFA_ANY {
                    let Some(out1_idx) = st.out1 else {
                        return -1;
                    };
                    if states[out1_idx].out.is_none() {
                        return -1;
                    }
                    state = states[out1_idx].out;
                    continue;
                }
            }
            NFA_DIGIT | NFA_WHITE | NFA_HEX | NFA_OCTAL => {
                len += 1;
            }
            NFA_IDENT | NFA_SIDENT | NFA_KWORD | NFA_SKWORD | NFA_FNAME | NFA_SFNAME
            | NFA_PRINT | NFA_SPRINT | NFA_NWHITE | NFA_NDIGIT | NFA_NHEX | NFA_NOCTAL
            | NFA_WORD | NFA_NWORD | NFA_HEAD | NFA_NHEAD | NFA_ALPHA | NFA_NALPHA | NFA_LOWER
            | NFA_NLOWER | NFA_UPPER | NFA_NUPPER | NFA_LOWER_IC | NFA_NLOWER_IC | NFA_UPPER_IC
            | NFA_NUPPER_IC | NFA_ANY_COMPOSING => {
                len += 3;
            }
            NFA_START_INVISIBLE
            | NFA_START_INVISIBLE_NEG
            | NFA_START_INVISIBLE_BEFORE
            | NFA_START_INVISIBLE_BEFORE_NEG => {
                state = st.out1.and_then(|o1| states[o1].out);
                continue;
            }
            c if (NFA_BACKREF1..=NFA_ZREF9).contains(&c) || c == NFA_NEWL || c == NFA_SKIP => {
                return -1;
            }
            c if matches!(
                c,
                NFA_BOL
                    | NFA_EOL
                    | NFA_BOF
                    | NFA_EOF
                    | NFA_BOW
                    | NFA_EOW
                    | NFA_NOPEN
                    | NFA_NCLOSE
                    | NFA_LNUM_GT
                    | NFA_LNUM_LT
                    | NFA_COL_GT
                    | NFA_COL_LT
                    | NFA_VCOL_GT
                    | NFA_VCOL_LT
                    | NFA_MARK_GT
                    | NFA_MARK_LT
                    | NFA_VISUAL
                    | NFA_LNUM
                    | NFA_CURSOR
                    | NFA_COL
                    | NFA_VCOL
                    | NFA_MARK
                    | NFA_ZSTART
                    | NFA_ZEND
                    | NFA_OPT_CHARS
                    | NFA_EMPTY
                    | NFA_START_PATTERN
                    | NFA_END_PATTERN
                    | NFA_COMPOSING
                    | NFA_END_COMPOSING
            ) || (NFA_MOPEN..=NFA_ZCLOSE9).contains(&c) => {}
            c => {
                if c < 0 {
                    return -1;
                }
                len += utf_char2len(c);
            }
        }
        state = st.out;
    }
    -1
}

fn post2nfa(
    states: &mut [NfaState],
    postfix: *mut i32,
    end: *mut i32,
    nfa_calc_size: bool,
) -> Option<usize> {
    if postfix.is_null() {
        return None;
    }

    let max_stack = if !nfa_calc_size {
        re_st().nstate as usize + 1
    } else {
        0
    };
    let mut stack: Vec<Frag> = Vec::with_capacity(max_stack);

    macro_rules! push {
        ($s:expr) => {
            if stack.len() < max_stack {
                stack.push($s);
            }
        };
    }
    macro_rules! pop {
        ($p:ident) => {{
            let Some(val) = stack.pop() else {
                st_error();
                return None;
            };
            val
        }};
    }

    let mut p = postfix;
    while p < end {
        match unsafe { *p } {
            NFA_CONCAT => {
                if !nfa_calc_size {
                    let e2 = pop!(p);
                    let e1 = pop!(p);
                    patch(states, e1.out, e2.start);
                    push!(frag(e1.start, e2.out));
                }
            }
            NFA_OR => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let e2 = pop!(p);
                    let e1 = pop!(p);
                    let s = alloc_state(states, NFA_SPLIT, e1.start, e2.start)?;
                    push!(frag(Some(s), append(e1.out, e2.out)));
                }
            }
            NFA_STAR => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let e = pop!(p);
                    let s = alloc_state(states, NFA_SPLIT, e.start, None)?;
                    patch(states, e.out, Some(s));
                    push!(frag(Some(s), list1(NfaOutSlot::Out1(s))));
                }
            }
            NFA_STAR_NONGREEDY => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let e = pop!(p);
                    let s = alloc_state(states, NFA_SPLIT, None, e.start)?;
                    patch(states, e.out, Some(s));
                    push!(frag(Some(s), list1(NfaOutSlot::Out(s))));
                }
            }
            NFA_QUEST => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let e = pop!(p);
                    let s = alloc_state(states, NFA_SPLIT, e.start, None)?;
                    push!(frag(Some(s), append(list1(NfaOutSlot::Out1(s)), e.out)));
                }
            }
            NFA_QUEST_NONGREEDY => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let e = pop!(p);
                    let s = alloc_state(states, NFA_SPLIT, None, e.start)?;
                    push!(frag(Some(s), append(list1(NfaOutSlot::Out(s)), e.out)));
                }
            }
            NFA_END_COLL | NFA_END_NEG_COLL => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let e = pop!(p);
                    let s = alloc_state(states, NFA_END_COLL, None, None)?;
                    patch(states, e.out, Some(s));
                    if let Some(estart) = e.start {
                        states[estart].out1 = Some(s);
                    }
                    push!(frag(e.start, list1(NfaOutSlot::Out(s))));
                }
            }
            NFA_RANGE => {
                if !nfa_calc_size {
                    let e2 = pop!(p);
                    let e1 = pop!(p);
                    let s2 = e2.start.unwrap();
                    let s1 = e1.start.unwrap();
                    states[s2].val = states[s2].c;
                    states[s2].c = NFA_RANGE_MAX;
                    states[s1].val = states[s1].c;
                    states[s1].c = NFA_RANGE_MIN;
                    patch(states, e1.out, e2.start);
                    push!(frag(e1.start, e2.out));
                }
            }
            NFA_EMPTY => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let s = alloc_state(states, NFA_EMPTY, None, None)?;
                    push!(frag(Some(s), list1(NfaOutSlot::Out(s))));
                }
            }
            NFA_OPT_CHARS => {
                p = p.wrapping_add(1);
                let mut n = unsafe { *p };
                if nfa_calc_size {
                    re_st().nstate += n;
                } else {
                    let mut s: Option<usize> = None;
                    let mut e1 = Frag::new();
                    let mut s1: Option<usize> = None;
                    while n > 0 {
                        n -= 1;
                        let e = pop!(p);
                        let cur_s = alloc_state(states, NFA_SPLIT, e.start, None)?;
                        s = Some(cur_s);
                        if e1.out.is_empty() {
                            e1 = e;
                        } else {
                            patch(states, e.out, s1);
                        }
                        e1.out.push(NfaOutSlot::Out1(cur_s));
                        s1 = Some(cur_s);
                    }
                    push!(frag(s, e1.out));
                }
            }
            NFA_PREV_ATOM_NO_WIDTH
            | NFA_PREV_ATOM_NO_WIDTH_NEG
            | NFA_PREV_ATOM_JUST_BEFORE
            | NFA_PREV_ATOM_JUST_BEFORE_NEG
            | NFA_PREV_ATOM_LIKE_PATTERN => {
                let op = unsafe { *p };
                let before = op == NFA_PREV_ATOM_JUST_BEFORE || op == NFA_PREV_ATOM_JUST_BEFORE_NEG;
                let pattern = op == NFA_PREV_ATOM_LIKE_PATTERN;
                let (start_state, end_state) = match op {
                    NFA_PREV_ATOM_NO_WIDTH => (NFA_START_INVISIBLE, NFA_END_INVISIBLE),
                    NFA_PREV_ATOM_NO_WIDTH_NEG => (NFA_START_INVISIBLE_NEG, NFA_END_INVISIBLE_NEG),
                    NFA_PREV_ATOM_JUST_BEFORE => (NFA_START_INVISIBLE_BEFORE, NFA_END_INVISIBLE),
                    NFA_PREV_ATOM_JUST_BEFORE_NEG => {
                        (NFA_START_INVISIBLE_BEFORE_NEG, NFA_END_INVISIBLE_NEG)
                    }
                    _ => (NFA_START_PATTERN, NFA_END_PATTERN),
                };
                let mut n: i32 = 0;
                if before {
                    p = p.wrapping_add(1);
                    n = unsafe { *p };
                }
                if nfa_calc_size {
                    re_st().nstate += if pattern { 4 } else { 2 };
                } else {
                    let e = pop!(p);
                    let s1 = alloc_state(states, end_state, None, None)?;
                    let s = alloc_state(states, start_state, e.start, Some(s1))?;
                    if pattern {
                        let skip = alloc_state(states, NFA_SKIP, None, None)?;
                        let zend = alloc_state(states, NFA_ZEND, Some(s1), None)?;
                        states[s1].out = Some(skip);
                        patch(states, e.out, Some(zend));
                        push!(frag(Some(s), list1(NfaOutSlot::Out(skip))));
                    } else {
                        patch(states, e.out, Some(s1));
                        push!(frag(Some(s), list1(NfaOutSlot::Out(s1))));
                        if before {
                            if n <= 0 {
                                n = nfa_max_width(states, e.start, 0);
                            }
                            states[s].val = n;
                        }
                    }
                }
            }
            op if op == NFA_COMPOSING
                || op == NFA_NOPEN
                || (NFA_MOPEN..=NFA_MOPEN9).contains(&op)
                || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&op) =>
            {
                if nfa_calc_size {
                    re_st().nstate += 2;
                } else {
                    let mopen = op;
                    let mclose = match op {
                        NFA_NOPEN => NFA_NCLOSE,
                        NFA_COMPOSING => NFA_END_COMPOSING,
                        c if (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) => NFA_ZCLOSE + (c - NFA_ZOPEN),
                        c => c + NSUBEXP as i32,
                    };
                    if stack.is_empty() {
                        let s = alloc_state(states, mopen, None, None)?;
                        let s1 = alloc_state(states, mclose, None, None)?;
                        patch(states, list1(NfaOutSlot::Out(s)), Some(s1));
                        push!(frag(Some(s), list1(NfaOutSlot::Out(s1))));
                    } else {
                        let e = pop!(p);
                        let s = alloc_state(states, mopen, e.start, None)?;
                        let s1 = alloc_state(states, mclose, None, None)?;
                        patch(states, e.out, Some(s1));
                        if mopen == NFA_COMPOSING {
                            patch(states, list1(NfaOutSlot::Out1(s)), Some(s1));
                        }
                        push!(frag(Some(s), list1(NfaOutSlot::Out(s1))));
                    }
                }
            }
            op if (NFA_BACKREF1..=NFA_ZREF9).contains(&op) => {
                if nfa_calc_size {
                    re_st().nstate += 2;
                } else {
                    let s = alloc_state(states, op, None, None)?;
                    let s1 = alloc_state(states, NFA_SKIP, None, None)?;
                    patch(states, list1(NfaOutSlot::Out(s)), Some(s1));
                    push!(frag(Some(s), list1(NfaOutSlot::Out(s1))));
                }
            }
            NFA_LNUM | NFA_LNUM_GT | NFA_LNUM_LT | NFA_VCOL | NFA_VCOL_GT | NFA_VCOL_LT
            | NFA_COL | NFA_COL_GT | NFA_COL_LT | NFA_MARK | NFA_MARK_GT | NFA_MARK_LT => {
                let op = unsafe { *p };
                p = p.wrapping_add(1);
                let n = unsafe { *p };
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let s = alloc_state(states, op, None, None)?;
                    states[s].val = n;
                    push!(frag(Some(s), list1(NfaOutSlot::Out(s))));
                }
            }
            op => {
                if nfa_calc_size {
                    re_st().nstate += 1;
                } else {
                    let s = alloc_state(states, op, None, None)?;
                    push!(frag(Some(s), list1(NfaOutSlot::Out(s))));
                }
            }
        }
        p = p.wrapping_add(1);
    }

    if nfa_calc_size {
        re_st().nstate += 1;
        return None;
    }

    let e = pop!(p);
    if !stack.is_empty() {
        emsg(tr(
                c"E875: (NFA regexp) (While converting from postfix to NFA),too many states left on stack",
            ));
        g_st().rc_did_emsg = true;
        return None;
    }

    if re_st().istate >= re_st().nstate {
        emsg(tr(
            c"E876: (NFA regexp) Not enough space to store the whole NFA ",
        ));
        g_st().rc_did_emsg = true;
        return None;
    }

    let match_idx = re_st().istate as usize;
    re_st().istate += 1;
    states[match_idx].c = NFA_MATCH;
    states[match_idx].out = None;
    states[match_idx].out1 = None;

    patch(states, e.out, Some(match_idx));
    e.start
}

fn nfa_postprocess(prog: &mut NfaRegprog) {
    for i in 0..prog.nstate as usize {
        let st = prog.state[i];
        let c = st.c;
        if c == NFA_START_INVISIBLE
            || c == NFA_START_INVISIBLE_NEG
            || c == NFA_START_INVISIBLE_BEFORE
            || c == NFA_START_INVISIBLE_BEFORE_NEG
        {
            let out1_out = st.out1.and_then(|o1| prog.state[o1].out);
            let directly = if match_follows(&prog.state, out1_out, 0) {
                true
            } else {
                let ch_invisible = failure_chance(&prog.state, st.out, 0);
                let ch_follows = failure_chance(&prog.state, out1_out, 0);
                if c == NFA_START_INVISIBLE_BEFORE || c == NFA_START_INVISIBLE_BEFORE_NEG {
                    if st.val <= 0 && ch_follows > 0 {
                        false
                    } else {
                        ch_follows * 10 < ch_invisible
                    }
                } else {
                    ch_follows < ch_invisible
                }
            };
            if directly {
                prog.state[i].c += 1;
            }
        }
    }
}

#[inline]
fn copy_pim(to: &mut NfaPim, from: &mut NfaPim) {
    to.result = from.result;
    to.state = from.state;
    copy_sub(&mut to.subs.norm, &mut from.subs.norm);
    if re_st().rex.nfa_has_zsubexpr != 0 {
        copy_sub(&mut to.subs.synt, &mut from.subs.synt);
    }
    to.end = from.end;
}

fn clear_sub(sub: &mut Regsub) {
    let n = re_st().rex.nfa_nsubexpr as usize;
    if reg_multi() {
        sub.list.multi_mut()[..n].fill(MultiPos {
            start_lnum: -1,
            end_lnum: -1,
            start_col: -1,
            end_col: -1,
        });
    } else {
        sub.list.line_mut()[..n].fill(LinePos::new());
    }
    sub.in_use = 0;
}

#[inline]
fn copy_sub(to: &mut Regsub, from: &mut Regsub) {
    let in_use = from.in_use;
    to.in_use = in_use;
    if in_use <= 0 {
        return;
    }
    if reg_multi() {
        to.orig_start_col = from.orig_start_col;
    }
    if in_use == 1 {
        to.list.copy_first(&from.list);
    } else {
        to.list.copy_n(&from.list, in_use as usize);
    }
}

#[inline]
fn copy_sub_off(to: &mut Regsub, from: &mut Regsub) {
    if to.in_use < from.in_use {
        to.in_use = from.in_use;
    }
    if from.in_use <= 1 {
        return;
    }
    to.list.copy_range_from_1(&from.list, from.in_use as usize);
}

fn copy_ze_off(to: &mut Regsub, from: &mut Regsub) {
    if re_st().rex.nfa_has_zend == 0 {
        return;
    }
    if reg_multi() {
        let from_m = from.list.multi()[0];
        if from_m.end_lnum >= 0 {
            to.list.multi_mut()[0].end_lnum = from_m.end_lnum;
            to.list.multi_mut()[0].end_col = from_m.end_col;
        }
    } else {
        let from_end = from.list.line()[0].end;
        if !from_end.is_null() {
            to.list.line_mut()[0].end = from_end;
        }
    }
}

#[inline]
fn sub_equal(sub1: &mut Regsub, sub2: &mut Regsub) -> bool {
    let todo = sub1.in_use.max(sub2.in_use) as usize;
    if todo == 1 && sub1.in_use == 1 && sub2.in_use == 1 && re_st().rex.nfa_has_backref == 0 {
        if reg_multi() {
            let m1 = sub1.list.multi()[0];
            let m2 = sub2.list.multi()[0];
            return m1.start_lnum == m2.start_lnum
                && (m1.start_lnum == -1 || m1.start_col == m2.start_col);
        } else {
            return sub1.list.line()[0].start == sub2.list.line()[0].start;
        }
    }
    if reg_multi() {
        let m1 = sub1.list.multi();
        let m2 = sub2.list.multi();
        for i in 0..todo {
            let s1 = if (i as i32) < sub1.in_use {
                m1[i].start_lnum
            } else {
                -1
            };
            let s2 = if (i as i32) < sub2.in_use {
                m2[i].start_lnum
            } else {
                -1
            };
            if s1 != s2 {
                return false;
            }
            if s1 != -1 && m1[i].start_col != m2[i].start_col {
                return false;
            }
            if re_st().rex.nfa_has_backref != 0 {
                let e1 = if (i as i32) < sub1.in_use {
                    m1[i].end_lnum
                } else {
                    -1
                };
                let e2 = if (i as i32) < sub2.in_use {
                    m2[i].end_lnum
                } else {
                    -1
                };
                if e1 != e2 {
                    return false;
                }
                if e1 != -1 && m1[i].end_col != m2[i].end_col {
                    return false;
                }
            }
        }
    } else {
        let l1 = sub1.list.line();
        let l2 = sub2.list.line();
        for i in 0..todo {
            let sp1 = if (i as i32) < sub1.in_use {
                l1[i].start
            } else {
                ptr::null_mut()
            };
            let sp2 = if (i as i32) < sub2.in_use {
                l2[i].start
            } else {
                ptr::null_mut()
            };
            if sp1 != sp2 {
                return false;
            }
            if re_st().rex.nfa_has_backref != 0 {
                let ep1 = if (i as i32) < sub1.in_use {
                    l1[i].end
                } else {
                    ptr::null_mut()
                };
                let ep2 = if (i as i32) < sub2.in_use {
                    l2[i].end
                } else {
                    ptr::null_mut()
                };
                if ep1 != ep2 {
                    return false;
                }
            }
        }
    }
    true
}

#[inline]
fn has_state_with_pos(
    l: &mut NfaList,
    state: usize,
    subs: &mut Regsubs,
    pim: Option<&NfaPim>,
) -> bool {
    let tp = l.t.as_mut_ptr();
    for i in 0..l.n as usize {
        let thread = tp.wrapping_add(i);
        let thread = unsafe { &mut *thread };
        if thread.state == Some(state)
            && sub_equal(&mut thread.subs.norm, &mut subs.norm)
            && (re_st().rex.nfa_has_zsubexpr == 0
                || sub_equal(&mut thread.subs.synt, &mut subs.synt))
            && pim_equal(Some(&thread.pim), pim)
        {
            return true;
        }
    }
    false
}

#[inline]
fn pim_equal(one: Option<&NfaPim>, two: Option<&NfaPim>) -> bool {
    let one_unused = one.is_none() || one.unwrap().result == NFA_PIM_UNUSED;
    let two_unused = two.is_none() || two.unwrap().result == NFA_PIM_UNUSED;
    if one_unused {
        return two_unused;
    }
    if two_unused {
        return false;
    }
    if one.unwrap().state != two.unwrap().state {
        return false;
    }
    if reg_multi() {
        let p1 = one.unwrap().end.pos();
        let p2 = two.unwrap().end.pos();
        p1.lnum == p2.lnum && p1.col == p2.col
    } else {
        one.unwrap().end.ptr() == two.unwrap().end.ptr()
    }
}

fn match_follows(states: &[NfaState], startstate: Option<usize>, depth: i32) -> bool {
    let mut state = startstate;
    if depth > 10 {
        return false;
    }
    while let Some(idx) = state {
        let st = states[idx];
        match st.c {
            NFA_MATCH
            | NFA_MCLOSE
            | NFA_END_INVISIBLE
            | NFA_END_INVISIBLE_NEG
            | NFA_END_PATTERN => return true,
            NFA_SPLIT => {
                return match_follows(states, st.out, depth + 1)
                    || match_follows(states, st.out1, depth + 1);
            }
            NFA_START_INVISIBLE
            | NFA_START_INVISIBLE_FIRST
            | NFA_START_INVISIBLE_BEFORE
            | NFA_START_INVISIBLE_BEFORE_FIRST
            | NFA_START_INVISIBLE_NEG
            | NFA_START_INVISIBLE_NEG_FIRST
            | NFA_START_INVISIBLE_BEFORE_NEG
            | NFA_START_INVISIBLE_BEFORE_NEG_FIRST
            | NFA_COMPOSING => {
                state = st.out1.and_then(|o1| states[o1].out);
                continue;
            }
            NFA_ANY | NFA_ANY_COMPOSING | NFA_IDENT | NFA_SIDENT | NFA_KWORD | NFA_SKWORD
            | NFA_FNAME | NFA_SFNAME | NFA_PRINT | NFA_SPRINT | NFA_WHITE | NFA_NWHITE
            | NFA_DIGIT | NFA_NDIGIT | NFA_HEX | NFA_NHEX | NFA_OCTAL | NFA_NOCTAL | NFA_WORD
            | NFA_NWORD | NFA_HEAD | NFA_NHEAD | NFA_ALPHA | NFA_NALPHA | NFA_LOWER
            | NFA_NLOWER | NFA_UPPER | NFA_NUPPER | NFA_LOWER_IC | NFA_NLOWER_IC | NFA_UPPER_IC
            | NFA_NUPPER_IC | NFA_START_COLL | NFA_START_NEG_COLL | NFA_NEWL => return false,
            c => {
                if c > 0 {
                    return false;
                }
            }
        }
        state = st.out;
    }
    false
}

fn state_in_list(
    states: &[NfaState],
    l: &mut NfaList,
    state: Option<usize>,
    subs: &mut Regsubs,
) -> bool {
    let Some(state_idx) = state else {
        return false;
    };
    if states[state_idx].lastlist[re_st().nfa_ll_index] == l.id
        && (re_st().rex.nfa_has_backref == 0 || has_state_with_pos(l, state_idx, &mut *subs, None))
    {
        return true;
    }
    false
}

#[inline(always)]
fn addstate(
    states: &mut [NfaState],
    l: &mut NfaList,
    state: Option<usize>,
    subs_arg: Option<&mut Regsubs>,
    pim: *mut NfaPim,
    off_arg: i32,
    depth: i32,
) -> *mut Regsubs {
    let subs_ptr = subs_arg.map_or(ptr::null_mut(), |p| &raw mut *p);
    let Some(state_idx) = state else {
        if depth >= 5000 || subs_ptr.is_null() {
            return ptr::null_mut();
        }
        return subs_ptr;
    };
    addstate_idx(
        states.as_mut_ptr(),
        l,
        state_idx,
        subs_ptr,
        pim,
        off_arg,
        depth,
    )
}

#[inline(always)]
fn addstate_idx(
    states: *mut NfaState,
    l: &mut NfaList,
    state_idx: usize,
    subs_arg: *mut Regsubs,
    pim: *mut NfaPim,
    off_arg: i32,
    depth: i32,
) -> *mut Regsubs {
    unsafe {
        if depth >= 5000 || subs_arg.is_null() {
            return ptr::null_mut();
        }

        let st_ptr = states.wrapping_add(state_idx);
        let st_ptr = &mut *st_ptr;
        let st_c = st_ptr.c;
        match st_c {
            NFA_NCLOSE | NFA_MOPEN | NFA_ZEND | NFA_SPLIT | NFA_EMPTY => {
                addstate_eps(states, l, state_idx, st_c, subs_arg, pim, off_arg, depth)
            }
            c if (NFA_MCLOSE..=NFA_MCLOSE9).contains(&c)
                || (NFA_ZCLOSE1..=NFA_ZCLOSE9).contains(&c) =>
            {
                addstate_eps(states, l, state_idx, st_c, subs_arg, pim, off_arg, depth)
            }
            c => {
                if (c == NFA_BOL || c == NFA_BOF)
                    && re_st().rex.input > re_st().rex.line
                    && *re_st().rex.input != NUL
                    && (re_st().nfa_endp.is_null()
                        || !reg_multi()
                        || re_st().rex.lnum == (*re_st().nfa_endp).se_u.pos().lnum)
                {
                    return subs_arg;
                }

                let ll_idx = re_st().nfa_ll_index;
                if st_ptr.lastlist[ll_idx] == l.id && st_c != NFA_SKIP {
                    if re_st().rex.nfa_has_backref == 0
                        && pim.is_null()
                        && l.has_pim == 0
                        && st_c != NFA_MATCH
                    {
                        if off_arg <= -ADDSTATE_HERE_OFFSET {
                            let listindex = -(off_arg + ADDSTATE_HERE_OFFSET);
                            let tp = l.t.as_ptr();
                            let limit = l.n.min(listindex) as usize;
                            let mut found = false;
                            for k in 0..limit {
                                if (*tp.wrapping_add(k)).state == Some(state_idx) {
                                    found = true;
                                    break;
                                }
                            }
                            if found {
                                return subs_arg;
                            }
                        } else {
                            return subs_arg;
                        }
                    } else if has_state_with_pos(l, state_idx, &mut *subs_arg, pim.as_ref()) {
                        return subs_arg;
                    }
                }

                let mut subs = subs_arg;
                if l.n as usize == l.t.capacity() {
                    let newlen = l.t.capacity() * 3 / 2 + 50;
                    let newsize = newlen * size_of::<NfaThread>();
                    if ((newsize >> 10) as i64) >= opt_vars().p_mmp {
                        emsg(tr(E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN));
                        return ptr::null_mut();
                    }
                    if subs != &raw mut nfa_st().temp_subs {
                        copy_sub(&mut nfa_st().temp_subs.norm, &mut (*subs).norm);
                        if re_st().rex.nfa_has_zsubexpr != 0 {
                            copy_sub(&mut nfa_st().temp_subs.synt, &mut (*subs).synt);
                        }
                        subs = &raw mut nfa_st().temp_subs;
                    }
                    l.t.reserve(newlen - l.t.len());
                }

                st_ptr.lastlist[ll_idx] = l.id;
                let thread_idx = l.n as usize;
                l.n += 1;
                if thread_idx == l.t.len() {
                    l.t.push(NfaThread::new());
                }
                let thread = &mut *l.t.as_mut_ptr().wrapping_add(thread_idx);
                thread.state = Some(state_idx);
                if pim.is_null() {
                    thread.pim.result = NFA_PIM_UNUSED;
                } else {
                    copy_pim(&mut thread.pim, &mut *pim);
                    l.has_pim = 1;
                }
                copy_sub(&mut thread.subs.norm, &mut (*subs).norm);
                if re_st().rex.nfa_has_zsubexpr != 0 {
                    copy_sub(&mut thread.subs.synt, &mut (*subs).synt);
                }

                if st_c == NFA_NOPEN
                    || st_c == NFA_ZSTART
                    || st_c == NFA_ZCLOSE
                    || (NFA_MOPEN1..=NFA_MOPEN9).contains(&st_c)
                    || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&st_c)
                {
                    return addstate_eps(states, l, state_idx, st_c, subs, pim, off_arg, depth);
                }
                subs
            }
        }
    }
}

fn addstate_eps(
    states: *mut NfaState,
    l: &mut NfaList,
    state_idx: usize,
    st_c: i32,
    mut subs: *mut Regsubs,
    pim: *mut NfaPim,
    off_arg: i32,
    depth: i32,
) -> *mut Regsubs {
    unsafe {
        let off = if off_arg <= -ADDSTATE_HERE_OFFSET {
            0
        } else {
            off_arg
        };
        let st_out = (*states.wrapping_add(state_idx)).out;
        let st_out1 = (*states.wrapping_add(state_idx)).out1;
        match st_c {
            NFA_MATCH => {}
            NFA_SPLIT => {
                if let Some(o) = st_out {
                    subs = addstate_idx(states, l, o, subs, pim, off_arg, depth + 1);
                }
                if let Some(o1) = st_out1 {
                    subs = addstate_idx(states, l, o1, subs, pim, off_arg, depth + 1);
                }
            }
            NFA_EMPTY | NFA_NOPEN | NFA_NCLOSE => {
                if let Some(o) = st_out {
                    subs = addstate_idx(states, l, o, subs, pim, off_arg, depth + 1);
                }
            }
            c if (NFA_MOPEN..=NFA_MOPEN9).contains(&c)
                || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c)
                || c == NFA_ZSTART =>
            {
                let (subidx, mut sub) = if c == NFA_ZSTART {
                    (0usize, &mut (*subs).norm)
                } else if (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) {
                    ((c - NFA_ZOPEN) as usize, &mut (*subs).synt)
                } else {
                    ((c - NFA_MOPEN) as usize, &mut (*subs).norm)
                };

                let mut save_ptr: *mut u8 = ptr::null_mut();
                let mut save_multipos = MultiPos::new();
                let save_in_use: i32;

                if reg_multi() {
                    let multi = sub.list.multi_mut();
                    if (subidx as i32) < sub.in_use {
                        save_multipos = multi[subidx];
                        save_in_use = -1;
                    } else {
                        save_in_use = sub.in_use;
                        for item in &mut multi[sub.in_use as usize..subidx] {
                            item.start_lnum = -1;
                            item.end_lnum = -1;
                        }
                        sub.in_use = subidx as i32 + 1;
                    }
                    if off == -1 {
                        multi[subidx].start_lnum = re_st().rex.lnum + 1;
                        multi[subidx].start_col = 0;
                    } else {
                        multi[subidx].start_lnum = re_st().rex.lnum;
                        multi[subidx].start_col =
                            (re_st().rex.input.offset_from(re_st().rex.line) as Colnr) + off;
                    }
                    multi[subidx].end_lnum = -1;
                } else {
                    let line = sub.list.line_mut();
                    if (subidx as i32) < sub.in_use {
                        save_ptr = line[subidx].start;
                        save_in_use = -1;
                    } else {
                        save_in_use = sub.in_use;
                        for item in &mut line[sub.in_use as usize..subidx] {
                            item.start = ptr::null_mut();
                            item.end = ptr::null_mut();
                        }
                        sub.in_use = subidx as i32 + 1;
                    }
                    line[subidx].start = re_st().rex.input.wrapping_offset(off as isize);
                }

                subs = if let Some(o) = st_out {
                    addstate_idx(states, l, o, subs, pim, off_arg, depth + 1)
                } else {
                    subs
                };
                if !subs.is_null() {
                    sub = if (NFA_ZOPEN..=NFA_ZOPEN9).contains(&st_c) {
                        &mut (*subs).synt
                    } else {
                        &mut (*subs).norm
                    };
                    if save_in_use == -1 {
                        if reg_multi() {
                            sub.list.multi_mut()[subidx] = save_multipos;
                        } else {
                            sub.list.line_mut()[subidx].start = save_ptr;
                        }
                    } else {
                        sub.in_use = save_in_use;
                    }
                }
            }
            c if (NFA_MCLOSE..=NFA_MCLOSE9).contains(&c)
                || (NFA_ZCLOSE..=NFA_ZCLOSE9).contains(&c)
                || c == NFA_ZEND =>
            {
                if c == NFA_MCLOSE
                    && re_st().rex.nfa_has_zend != 0
                    && if reg_multi() {
                        (*subs).norm.list.multi()[0].end_lnum >= 0
                    } else {
                        !(*subs).norm.list.line()[0].end.is_null()
                    }
                {
                    if let Some(o) = st_out {
                        subs = addstate_idx(states, l, o, subs, pim, off_arg, depth + 1);
                    }
                } else {
                    let (subidx, mut sub) = if c == NFA_ZEND {
                        (0usize, &mut (*subs).norm)
                    } else if (NFA_ZCLOSE..=NFA_ZCLOSE9).contains(&c) {
                        ((c - NFA_ZCLOSE) as usize, &mut (*subs).synt)
                    } else {
                        ((c - NFA_MCLOSE) as usize, &mut (*subs).norm)
                    };

                    let save_in_use = sub.in_use;
                    if sub.in_use <= subidx as i32 {
                        sub.in_use = subidx as i32 + 1;
                    }
                    let mut save_ptr: *mut u8 = ptr::null_mut();
                    let mut save_multipos = MultiPos::new();
                    if reg_multi() {
                        let multi = sub.list.multi_mut();
                        save_multipos = multi[subidx];
                        if off == -1 {
                            multi[subidx].end_lnum = re_st().rex.lnum + 1;
                            multi[subidx].end_col = 0;
                        } else {
                            multi[subidx].end_lnum = re_st().rex.lnum;
                            multi[subidx].end_col =
                                (re_st().rex.input.offset_from(re_st().rex.line) as Colnr) + off;
                        }
                    } else {
                        let line = sub.list.line_mut();
                        save_ptr = line[subidx].end;
                        line[subidx].end = re_st().rex.input.wrapping_offset(off as isize);
                    }

                    subs = if let Some(o) = st_out {
                        addstate_idx(states, l, o, subs, pim, off_arg, depth + 1)
                    } else {
                        subs
                    };
                    if !subs.is_null() {
                        sub = if (NFA_ZCLOSE..=NFA_ZCLOSE9).contains(&st_c) {
                            &mut (*subs).synt
                        } else {
                            &mut (*subs).norm
                        };
                        if reg_multi() {
                            sub.list.multi_mut()[subidx] = save_multipos;
                        } else {
                            sub.list.line_mut()[subidx].end = save_ptr;
                        }
                        sub.in_use = save_in_use;
                    }
                }
            }
            _ => {}
        }
        subs
    }
}

fn addstate_here(
    states: &mut [NfaState],
    l: &mut NfaList,
    state: Option<usize>,
    subs: &mut Regsubs,
    pim: *mut NfaPim,
    ip: &mut i32,
) -> *mut Regsubs {
    let tlen = l.n;
    let listidx = *ip;

    let r = addstate(
        states,
        l,
        state,
        Some(subs),
        pim,
        -listidx - ADDSTATE_HERE_OFFSET,
        0,
    );
    if r.is_null() {
        return ptr::null_mut();
    }
    if listidx + 1 == tlen {
        return r;
    }
    let count = l.n - tlen;
    if count == 0 {
        return r;
    }
    if count == 1 {
        let tp = l.t.as_mut_ptr();
        unsafe {
            *tp.wrapping_add(listidx as usize) = *tp.wrapping_add((l.n - 1) as usize);
        }
    } else if count > 1 {
        if (l.n + count) as usize > l.t.len() {
            let newlen = l.t.len() * 3 / 2 + 50;
            let newsize = newlen * size_of::<NfaThread>();
            if ((newsize >> 10) as i64) >= opt_vars().p_mmp {
                emsg(tr(E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN));
                return ptr::null_mut();
            }
            l.t.resize(newlen, NfaThread::new());
        }
        let tp = l.t.as_mut_ptr();
        unsafe {
            ptr::copy(
                tp.wrapping_add(listidx as usize + 1),
                tp.wrapping_add((listidx + count) as usize),
                (l.n - listidx - 1) as usize,
            )
        };
        unsafe {
            ptr::copy(
                tp.wrapping_add((l.n - 1) as usize),
                tp.wrapping_add(listidx as usize),
                count as usize,
            )
        };
    }
    l.n -= 1;
    *ip = listidx - 1;
    r
}

fn check_char_class(cls: i32, c: i32) -> bool {
    match cls {
        NFA_CLASS_ALNUM => {
            if (1..128).contains(&c) && (c as u8).is_ascii_alphanumeric() {
                return true;
            }
        }
        NFA_CLASS_ALPHA => {
            if (1..128).contains(&c) && (c as u8).is_ascii_alphabetic() {
                return true;
            }
        }
        NFA_CLASS_BLANK => {
            if c == b' ' as i32 || c == b'\t' as i32 {
                return true;
            }
        }
        NFA_CLASS_CNTRL => {
            if (1..=127).contains(&c) && (c as u8).is_ascii_control() {
                return true;
            }
        }
        NFA_CLASS_DIGIT => {
            if ascii_isdigit(c) {
                return true;
            }
        }
        NFA_CLASS_GRAPH => {
            if (1..=127).contains(&c) && (c as u8).is_ascii_graphic() {
                return true;
            }
        }
        NFA_CLASS_LOWER => {
            if mb_islower(c) && c != 170 && c != 186 {
                return true;
            }
        }
        NFA_CLASS_PRINT => {
            if vim_isprintc(c) {
                return true;
            }
        }
        NFA_CLASS_PUNCT => {
            if (1..128).contains(&c) && (c as u8).is_ascii_punctuation() {
                return true;
            }
        }
        NFA_CLASS_SPACE => {
            if (9..=13).contains(&c) || c == b' ' as i32 {
                return true;
            }
        }
        NFA_CLASS_UPPER => {
            if mb_isupper(c) {
                return true;
            }
        }
        NFA_CLASS_XDIGIT => {
            if ascii_isxdigit(c) {
                return true;
            }
        }
        NFA_CLASS_TAB => {
            if c == b'\t' as i32 {
                return true;
            }
        }
        NFA_CLASS_RETURN => {
            if c == b'\r' as i32 {
                return true;
            }
        }
        NFA_CLASS_BACKSPACE => {
            if c == BS {
                return true;
            }
        }
        NFA_CLASS_ESCAPE => {
            if c == ESC {
                return true;
            }
        }
        NFA_CLASS_IDENT => {
            if vim_is_idc(c) {
                return true;
            }
        }
        NFA_CLASS_KEYWORD => {
            if reg_iswordc(c) {
                return true;
            }
        }
        NFA_CLASS_FNAME => {
            if vim_isfilec(c) {
                return true;
            }
        }
        _ => {
            siemsg!(tr(E_ILL_CHAR_CLASS), i64::from(cls));
            return false;
        }
    }
    false
}

fn match_backref(sub: &mut Regsub, subidx: i32, bytelen: &mut i32) -> bool {
    if sub.in_use <= subidx {
        *bytelen = 0;
        return true;
    }
    let idx = subidx as usize;
    if reg_multi() {
        let mpos = sub.list.multi()[idx];
        if mpos.start_lnum < 0 || mpos.end_lnum < 0 {
            *bytelen = 0;
            return true;
        }
        if mpos.start_lnum == re_st().rex.lnum && mpos.end_lnum == re_st().rex.lnum {
            let mut len = mpos.end_col - mpos.start_col;
            if cstrncmp(
                re_st()
                    .rex
                    .line
                    .wrapping_add(mpos.start_col as usize)
                    .cast(),
                re_st().rex.input.cast(),
                &mut len,
            ) == 0
            {
                *bytelen = len;
                return true;
            }
        } else if match_with_backref(
            mpos.start_lnum,
            mpos.start_col,
            mpos.end_lnum,
            mpos.end_col,
            &mut *bytelen,
        ) == RA_MATCH
        {
            return true;
        }
    } else {
        let lpos = sub.list.line()[idx];
        if lpos.start.is_null() || lpos.end.is_null() {
            *bytelen = 0;
            return true;
        }
        let mut len = unsafe { lpos.end.offset_from(lpos.start) } as i32;
        if cstrncmp(lpos.start.cast(), re_st().rex.input.cast(), &mut len) == 0 {
            *bytelen = len;
            return true;
        }
    }
    false
}

fn match_zref(subidx: i32, bytelen: &mut i32) -> bool {
    unsafe {
        cleanup_zsubexpr();
        let idx = subidx as usize;
        let ext_in = &raw const g_st().re_extmatch_in;
        let Some(ref em) = *ext_in else {
            *bytelen = 0;
            return true;
        };
        let Some(ref m_cstr) = em.matches[idx] else {
            *bytelen = 0;
            return true;
        };
        let m = m_cstr.as_ptr();
        let mut len = m_cstr.as_bytes().len() as i32;
        if cstrncmp(m, re_st().rex.input.cast(), &mut len) == 0 {
            *bytelen = len;
            return true;
        }
        false
    }
}

fn nfa_save_listids(prog: &mut NfaRegprog, list: *mut i32) {
    unsafe {
        let mut p = prog.state.as_mut_ptr();
        let mut i = prog.nstate;
        while {
            i -= 1;
            i >= 0
        } {
            *list.wrapping_add(i as usize) = (*p).lastlist[1];
            (*p).lastlist[1] = 0;
            p = p.wrapping_add(1);
        }
    }
}

fn nfa_restore_listids(prog: &mut NfaRegprog, list: *const i32) {
    unsafe {
        let mut p = prog.state.as_mut_ptr();
        let mut i = prog.nstate;
        while {
            i -= 1;
            i >= 0
        } {
            (*p).lastlist[1] = *list.wrapping_add(i as usize);
            p = p.wrapping_add(1);
        }
    }
}

#[inline]
fn nfa_re_num_cmp(val: u64, op: i32, pos: u64) -> bool {
    if op == 1 {
        pos > val
    } else if op == 2 {
        pos < val
    } else {
        val == pos
    }
}

fn recursive_regmatch(
    state: usize,
    pim: Option<&NfaPim>,
    prog: &mut NfaRegprog,
    submatch: &mut Regsubs,
    m: &mut Regsubs,
    listids: &mut Vec<i32>,
) -> i32 {
    let save_reginput_col = unsafe { re_st().rex.input.offset_from(re_st().rex.line) } as usize;
    let save_reglnum = re_st().rex.lnum;
    let save_nfa_match = nfa_st().nfa_match;
    let save_nfa_listid = re_st().rex.nfa_listid;
    let save_nfa_endp = re_st().nfa_endp;
    let mut endpos = SaveSe::new();
    let mut endposp: *mut SaveSe = ptr::null_mut();
    let mut need_restore = false;

    if let Some(pim) = pim {
        if reg_multi() {
            re_st().rex.input = re_st().rex.line.wrapping_add(pim.end.pos().col as usize);
        } else {
            re_st().rex.input = pim.end.ptr();
        }
    }

    let st = (&prog.state)[state];
    if st.c == NFA_START_INVISIBLE_BEFORE
        || st.c == NFA_START_INVISIBLE_BEFORE_FIRST
        || st.c == NFA_START_INVISIBLE_BEFORE_NEG
        || st.c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST
    {
        if let Some(pim) = pim {
            endpos.se_u = pim.end;
        } else if reg_multi() {
            endpos.se_u = crate::regexp::RegPos::Pos(Lpos {
                col: unsafe { re_st().rex.input.offset_from(re_st().rex.line) } as Colnr,
                lnum: re_st().rex.lnum,
            });
        } else {
            endpos.se_u = crate::regexp::RegPos::Ptr(re_st().rex.input);
        }
        endposp = &raw mut endpos;

        if st.val <= 0 {
            if reg_multi() {
                re_st().rex.lnum -= 1;
                re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
                if re_st().rex.line.is_null() {
                    re_st().rex.lnum += 1;
                    re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
                }
            }
            re_st().rex.input = re_st().rex.line;
        } else {
            if reg_multi()
                && (unsafe { re_st().rex.input.offset_from(re_st().rex.line) } as i32) < st.val
            {
                re_st().rex.lnum -= 1;
                re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
                if re_st().rex.line.is_null() {
                    re_st().rex.lnum += 1;
                    re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
                    re_st().rex.input = re_st().rex.line;
                } else {
                    re_st().rex.input = re_st()
                        .rex
                        .line
                        .wrapping_add(reg_getline_len(re_st().rex.lnum) as usize);
                }
            }
            if (unsafe { re_st().rex.input.offset_from(re_st().rex.line) } as i32) >= st.val {
                re_st().rex.input = re_st().rex.input.wrapping_sub(st.val as usize);
                re_st().rex.input =
                    re_st().rex.input.wrapping_sub(utf_head_off(
                        re_st().rex.line.cast(),
                        re_st().rex.input.cast(),
                    ) as usize);
            } else {
                re_st().rex.input = re_st().rex.line;
            }
        }
    }

    if re_st().nfa_ll_index == 1 {
        let nstate_len = prog.nstate as usize;
        if listids.len() < nstate_len {
            listids.resize(nstate_len, 0);
        }
        nfa_save_listids(&mut *prog, listids.as_mut_ptr());
        need_restore = true;
    } else {
        re_st().nfa_ll_index += 1;
        if re_st().rex.nfa_listid <= re_st().rex.nfa_alt_listid {
            re_st().rex.nfa_listid = re_st().rex.nfa_alt_listid;
        }
    }

    re_st().nfa_endp = endposp;
    let result = nfa_regmatch(&mut *prog, st.out, &mut *submatch, &mut *m);

    if need_restore {
        nfa_restore_listids(&mut *prog, listids.as_ptr());
    } else {
        re_st().nfa_ll_index -= 1;
        re_st().rex.nfa_alt_listid = re_st().rex.nfa_listid;
    }

    re_st().rex.lnum = save_reglnum;
    if reg_multi() {
        re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
    }
    re_st().rex.input = re_st().rex.line.wrapping_add(save_reginput_col);
    if result != NFA_TOO_EXPENSIVE {
        nfa_st().nfa_match = save_nfa_match;
        re_st().rex.nfa_listid = save_nfa_listid;
    }
    re_st().nfa_endp = save_nfa_endp;

    result
}

fn failure_chance(states: &[NfaState], state: Option<usize>, depth: i32) -> i32 {
    let Some(state_idx) = state else {
        return 1;
    };
    let st = states[state_idx];
    let c = st.c;
    if depth > 4 {
        return 1;
    }
    match c {
        NFA_SPLIT => {
            if st.out.is_some_and(|o| states[o].c == NFA_SPLIT)
                || st.out1.is_some_and(|o1| states[o1].c == NFA_SPLIT)
            {
                return 1;
            }
            let l = failure_chance(states, st.out, depth + 1);
            let r = failure_chance(states, st.out1, depth + 1);
            l.min(r)
        }
        NFA_ANY => 1,
        NFA_MATCH | NFA_MCLOSE | NFA_ANY_COMPOSING => 0,
        NFA_START_INVISIBLE
        | NFA_START_INVISIBLE_FIRST
        | NFA_START_INVISIBLE_NEG
        | NFA_START_INVISIBLE_NEG_FIRST
        | NFA_START_INVISIBLE_BEFORE
        | NFA_START_INVISIBLE_BEFORE_FIRST
        | NFA_START_INVISIBLE_BEFORE_NEG
        | NFA_START_INVISIBLE_BEFORE_NEG_FIRST
        | NFA_START_PATTERN => 5,
        NFA_BOL | NFA_EOL | NFA_BOF | NFA_EOF | NFA_NEWL => 99,
        NFA_BOW | NFA_EOW => 90,
        x if (NFA_MOPEN..=NFA_ZCLOSE9).contains(&x) || x == NFA_NOPEN || x == NFA_NCLOSE => {
            failure_chance(states, st.out, depth + 1)
        }
        x if (NFA_BACKREF1..=NFA_ZREF9).contains(&x) => 94,
        NFA_LNUM_GT | NFA_LNUM_LT | NFA_COL_GT | NFA_COL_LT | NFA_VCOL_GT | NFA_VCOL_LT
        | NFA_MARK_GT | NFA_MARK_LT | NFA_VISUAL => 85,
        NFA_LNUM => 90,
        NFA_CURSOR | NFA_COL | NFA_VCOL | NFA_MARK => 98,
        NFA_COMPOSING => 95,
        x if x > 0 => 95,
        _ => 50,
    }
}

fn skip_to_start(c: i32, colp: &mut Colnr) -> bool {
    let s = cstrchr(re_st().rex.line.wrapping_add(*colp as usize).cast(), c);
    if s.is_null() {
        return false;
    }
    *colp = unsafe { s.cast::<u8>().offset_from(re_st().rex.line) } as Colnr;
    true
}

fn find_match_text(startcol: &mut Colnr, regstart: i32, match_text: *const u8) -> i32 {
    let mut col = *startcol;
    loop {
        let mut matched = true;
        let mut s1 = match_text;
        let regstart_len2 = utf_ptr2len(re_st().rex.line.wrapping_add(col as usize).cast());
        let mut s2 = re_st()
            .rex
            .line
            .wrapping_add((col + regstart_len2) as usize);
        while unsafe { *s1 } != NUL {
            let c1_len = utf_ptr2len(s1.cast()) as usize;
            let c1 = utf_ptr2char(s1.cast());
            let c2_len = utf_ptr2len(s2.cast()) as usize;
            let c2 = utf_ptr2char(s2.cast());
            if c1 != c2 && (!re_st().rex.reg_ic || utf_fold(c1) != utf_fold(c2)) {
                matched = false;
                break;
            }
            s1 = s1.wrapping_add(c1_len);
            s2 = s2.wrapping_add(c2_len);
        }
        if matched && !utf_iscomposing_legacy(utf_ptr2char(s2.cast())) {
            cleanup_subexpr();
            if reg_multi() {
                unsafe {
                    (*re_st().rex.reg_startpos).lnum = re_st().rex.lnum;
                }
                unsafe {
                    (*re_st().rex.reg_startpos).col = col;
                }
                unsafe {
                    (*re_st().rex.reg_endpos).lnum = re_st().rex.lnum;
                }
                unsafe {
                    (*re_st().rex.reg_endpos).col = s2.offset_from(re_st().rex.line) as Colnr;
                }
            } else {
                unsafe {
                    *re_st().rex.reg_startp = re_st().rex.line.wrapping_add(col as usize);
                }
                unsafe {
                    *re_st().rex.reg_endp = s2;
                }
            }
            *startcol = col;
            return 1;
        }

        col += regstart_len2;
        if !skip_to_start(regstart, &mut col) {
            break;
        }
    }
    *startcol = col;
    0
}

fn match_composing(states: &[NfaState], mut sta: Option<usize>, curc: i32, clen: i32) -> bool {
    let mut mc = curc;
    let mut len: i32 = 0;
    let mut cchars: [i32; MAX_MCO] = [0; MAX_MCO];
    let mut ccount: usize = 0;

    let Some(sta_idx) = sta else {
        return false;
    };
    if utf_iscomposing_legacy(states[sta_idx].c) {
        len += utf_char2len(mc);
    }
    if re_st().rex.reg_icombine && len == 0 {
        return states[sta_idx].c == curc;
    }
    if len == 0 && mc != states[sta_idx].c {
        return false;
    }
    if len == 0 {
        len += utf_char2len(mc);
        sta = states[sta_idx].out;
    }

    while len < clen {
        mc = utf_ptr2char(re_st().rex.input.wrapping_add(len as usize).cast());
        cchars[ccount] = mc;
        ccount += 1;
        len += utf_char2len(mc);
        if ccount == MAX_MCO {
            break;
        }
    }

    while let Some(s_idx) = sta {
        if states[s_idx].c == NFA_END_COMPOSING {
            break;
        }
        let mut found = false;
        for &cc in cchars.iter().take(ccount) {
            if cc == states[s_idx].c {
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
        sta = states[s_idx].out;
    }
    true
}

fn nfa_did_time_out() -> bool {
    if nfa_st().nfa_time_limit.is_some_and(profile_passed_limit) {
        if !nfa_st().nfa_timed_out.is_null() {
            unsafe {
                *nfa_st().nfa_timed_out = 1;
            }
        }
        return true;
    }
    false
}

fn nfa_regmatch(
    prog: &mut NfaRegprog,
    start: Option<usize>,
    submatch: &mut Regsubs,
    m: &mut Regsubs,
) -> i32 {
    unsafe {
        let Some(start_idx) = start else {
            return 0;
        };
        let mut flag: usize = 0;
        let mut go_to_nextline = false;
        let mut listids: Vec<i32> = Vec::new();
        let toplevel = (&prog.state)[start_idx].c == NFA_MOPEN;

        reg_breakcheck();
        if g_st().got_int || nfa_did_time_out() {
            return 0;
        }

        nfa_st().nfa_match = 0;

        let init_len = prog.nstate as usize + 1;
        let (mut t0, mut t1) = if toplevel && prog.listbuf[0].capacity() > 0 {
            (
                std::mem::take(&mut prog.listbuf[0]),
                std::mem::take(&mut prog.listbuf[1]),
            )
        } else if let Some([b0, b1]) = nfa_st().rec_listbuf.pop() {
            (b0, b1)
        } else {
            (Vec::with_capacity(init_len), Vec::with_capacity(init_len))
        };
        if t0.capacity() < init_len {
            t0.reserve(init_len - t0.len());
        }
        if t1.capacity() < init_len {
            t1.reserve(init_len - t1.len());
        }
        let mut list: [NfaList; 2] = [
            NfaList {
                t: t0,
                n: 0,
                id: re_st().rex.nfa_listid + 1,
                has_pim: 0,
            },
            NfaList {
                t: t1,
                n: 0,
                id: 0,
                has_pim: 0,
            },
        ];

        let r = if toplevel {
            if reg_multi() {
                let start_col = re_st().rex.input.offset_from(re_st().rex.line) as Colnr;
                m.norm.list.multi_mut()[0].start_lnum = re_st().rex.lnum;
                m.norm.list.multi_mut()[0].start_col = start_col;
                m.norm.orig_start_col = start_col;
            } else {
                m.norm.list.line_mut()[0].start = re_st().rex.input;
            }
            m.norm.in_use = 1;
            let start_out = (&prog.state)[start_idx].out;
            addstate(
                &mut prog.state,
                &mut list[0],
                start_out,
                Some(m),
                ptr::null_mut(),
                0,
                0,
            )
        } else {
            addstate(
                &mut prog.state,
                &mut list[0],
                start,
                Some(m),
                ptr::null_mut(),
                0,
                0,
            )
        };
        if r.is_null() {
            nfa_st().nfa_match = NFA_TOO_EXPENSIVE;
            return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
        }

        loop {
            let curc: i32;
            let mut clen: i32;
            if *re_st().rex.input != NUL
                && *re_st().rex.input < 0x80
                && *re_st().rex.input.wrapping_add(1) < 0x80
            {
                curc = *re_st().rex.input as i32;
                clen = 1;
            } else {
                curc = utf_ptr2char(re_st().rex.input.cast());
                clen = utfc_ptr2len(re_st().rex.input.cast());
            }
            if curc == NUL as i32 {
                clen = 0;
                go_to_nextline = false;
            }

            let this_idx = flag;
            flag ^= 1;
            let next_idx = flag;
            let thislist = (&raw mut list).cast::<NfaList>().wrapping_add(this_idx);
            let thislist = &mut *thislist;
            let nextlist = &mut *(&raw mut list).cast::<NfaList>().wrapping_add(next_idx);
            nextlist.n = 0;
            nextlist.has_pim = 0;
            re_st().rex.nfa_listid += 1;
            if prog.re_engine == AUTOMATIC_ENGINE as u32 && re_st().rex.nfa_listid >= NFA_MAX_STATES
            {
                nfa_st().nfa_match = NFA_TOO_EXPENSIVE;
                return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
            }

            thislist.id = re_st().rex.nfa_listid;
            nextlist.id = re_st().rex.nfa_listid + 1;

            if thislist.n == 0 {
                break;
            }

            let mut goto_nextchar = false;
            let mut listidx: i32 = 0;
            while listidx < thislist.n {
                reg_breakcheck();
                if g_st().got_int {
                    break;
                }
                if { nfa_st().nfa_time_limit }.is_some() {
                    nfa_st().nfa_time_count += 1;
                    if nfa_st().nfa_time_count == 20 {
                        nfa_st().nfa_time_count = 0;
                        if nfa_did_time_out() {
                            break;
                        }
                    }
                }
                let t = &mut *thislist.t.as_mut_ptr().wrapping_add(listidx as usize);
                let t_state_idx = t.state.unwrap_unchecked();
                let t_st = &prog.state[t_state_idx];

                let mut add_state: Option<usize> = None;
                let mut add_here = false;
                let mut add_count: i32 = 0;
                let mut add_off: i32 = 0;

                match t_st.c {
                    NFA_MATCH => {
                        if !re_st().rex.reg_icombine
                            && re_st().rex.input != re_st().rex.line
                            && utf_iscomposing_legacy(curc)
                        {
                            listidx += 1;
                            continue;
                        }
                        nfa_st().nfa_match = 1;
                        copy_sub(&mut submatch.norm, &mut t.subs.norm);
                        if re_st().rex.nfa_has_zsubexpr != 0 {
                            copy_sub(&mut submatch.synt, &mut t.subs.synt);
                        }
                        if nextlist.n == 0 {
                            clen = 0;
                        }
                        goto_nextchar = true;
                        break;
                    }
                    NFA_END_INVISIBLE | NFA_END_INVISIBLE_NEG | NFA_END_PATTERN => {
                        if !re_st().nfa_endp.is_null()
                            && if reg_multi() {
                                let end_pos = (*re_st().nfa_endp).se_u.pos();
                                re_st().rex.lnum != end_pos.lnum
                                    || (re_st().rex.input.offset_from(re_st().rex.line) as Colnr)
                                        != end_pos.col
                            } else {
                                re_st().rex.input != (*re_st().nfa_endp).se_u.ptr()
                            }
                        {
                            listidx += 1;
                            continue;
                        }
                        if t_st.c != NFA_END_INVISIBLE_NEG {
                            copy_sub(&mut m.norm, &mut t.subs.norm);
                            if re_st().rex.nfa_has_zsubexpr != 0 {
                                copy_sub(&mut m.synt, &mut t.subs.synt);
                            }
                        }
                        nfa_st().nfa_match = 1;
                        if nextlist.n == 0 {
                            clen = 0;
                        }
                        goto_nextchar = true;
                        break;
                    }
                    NFA_START_INVISIBLE
                    | NFA_START_INVISIBLE_FIRST
                    | NFA_START_INVISIBLE_NEG
                    | NFA_START_INVISIBLE_NEG_FIRST
                    | NFA_START_INVISIBLE_BEFORE
                    | NFA_START_INVISIBLE_BEFORE_FIRST
                    | NFA_START_INVISIBLE_BEFORE_NEG
                    | NFA_START_INVISIBLE_BEFORE_NEG_FIRST => {
                        let st_c = t_st.c;
                        let out1_out = t_st.out1.and_then(|o1| (&prog.state)[o1].out);
                        if t.pim.result != NFA_PIM_UNUSED
                            || st_c == NFA_START_INVISIBLE_FIRST
                            || st_c == NFA_START_INVISIBLE_NEG_FIRST
                            || st_c == NFA_START_INVISIBLE_BEFORE_FIRST
                            || st_c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST
                        {
                            let in_use = m.norm.in_use;
                            copy_sub_off(&mut m.norm, &mut t.subs.norm);
                            if re_st().rex.nfa_has_zsubexpr != 0 {
                                copy_sub_off(&mut m.synt, &mut t.subs.synt);
                            }
                            let res = recursive_regmatch(
                                t_state_idx,
                                None,
                                &mut *prog,
                                &mut *submatch,
                                m,
                                &mut listids,
                            );
                            if res == NFA_TOO_EXPENSIVE {
                                nfa_st().nfa_match = res;
                                return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
                            }
                            let is_neg = st_c == NFA_START_INVISIBLE_NEG
                                || st_c == NFA_START_INVISIBLE_NEG_FIRST
                                || st_c == NFA_START_INVISIBLE_BEFORE_NEG
                                || st_c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST;
                            if (res != 0) != is_neg {
                                copy_sub_off(&mut t.subs.norm, &mut m.norm);
                                if re_st().rex.nfa_has_zsubexpr != 0 {
                                    copy_sub_off(&mut t.subs.synt, &mut m.synt);
                                }
                                copy_ze_off(&mut t.subs.norm, &mut m.norm);
                                add_here = true;
                                add_state = out1_out;
                            }
                            m.norm.in_use = in_use;
                        } else {
                            let mut pim = NfaPim::new();
                            pim.state = Some(t_state_idx);
                            pim.result = NFA_PIM_TODO;
                            pim.subs.norm.in_use = 0;
                            pim.subs.synt.in_use = 0;
                            if reg_multi() {
                                pim.end = crate::regexp::RegPos::Pos(Lpos {
                                    col: re_st().rex.input.offset_from(re_st().rex.line) as Colnr,
                                    lnum: re_st().rex.lnum,
                                });
                            } else {
                                pim.end = crate::regexp::RegPos::Ptr(re_st().rex.input);
                            }
                            if addstate_here(
                                &mut prog.state,
                                &mut *thislist,
                                out1_out,
                                &mut t.subs,
                                &raw mut pim,
                                &mut listidx,
                            )
                            .is_null()
                            {
                                nfa_st().nfa_match = NFA_TOO_EXPENSIVE;
                                return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
                            }
                        }
                    }
                    NFA_START_PATTERN => {
                        let out1_out = t_st.out1.and_then(|o1| (&prog.state)[o1].out);
                        let out1_out_out = out1_out.and_then(|oo| (&prog.state)[oo].out);
                        let skip =
                            if state_in_list(&prog.state, &mut *nextlist, out1_out, &mut t.subs) {
                                out1_out
                            } else if state_in_list(
                                &prog.state,
                                &mut *nextlist,
                                out1_out_out,
                                &mut t.subs,
                            ) || state_in_list(
                                &prog.state,
                                &mut *thislist,
                                out1_out_out,
                                &mut t.subs,
                            ) {
                                out1_out_out
                            } else {
                                None
                            };
                        if skip.is_none() {
                            copy_sub_off(&mut m.norm, &mut t.subs.norm);
                            if re_st().rex.nfa_has_zsubexpr != 0 {
                                copy_sub_off(&mut m.synt, &mut t.subs.synt);
                            }
                            let res = recursive_regmatch(
                                t_state_idx,
                                None,
                                &mut *prog,
                                &mut *submatch,
                                m,
                                &mut listids,
                            );
                            if res == NFA_TOO_EXPENSIVE {
                                nfa_st().nfa_match = res;
                                return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
                            }
                            if res != 0 {
                                copy_sub_off(&mut t.subs.norm, &mut m.norm);
                                if re_st().rex.nfa_has_zsubexpr != 0 {
                                    copy_sub_off(&mut t.subs.synt, &mut m.synt);
                                }
                                let bytelen = if reg_multi() {
                                    m.norm.list.multi()[0].end_col
                                        - (re_st().rex.input.offset_from(re_st().rex.line) as Colnr)
                                } else {
                                    m.norm.list.line()[0].end.offset_from(re_st().rex.input) as i32
                                };
                                if bytelen == 0 {
                                    add_here = true;
                                    add_state = out1_out_out;
                                } else if bytelen <= clen {
                                    add_state = out1_out_out;
                                    add_off = clen;
                                } else {
                                    add_state = out1_out;
                                    add_off = bytelen;
                                    add_count = bytelen - clen;
                                }
                            }
                        }
                    }
                    NFA_BOL => {
                        if re_st().rex.input == re_st().rex.line {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    NFA_EOL => {
                        if curc == NUL as i32 {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    NFA_BOW => {
                        if curc != NUL as i32 {
                            let this_class = mb_get_class_tab(
                                re_st().rex.input.cast(),
                                (&raw const re_st().rex.buf_mut().b_chartab).cast(),
                            );
                            if this_class > 1 && reg_prev_class() != this_class {
                                add_here = true;
                                add_state = t_st.out;
                            }
                        }
                    }
                    NFA_EOW => {
                        if re_st().rex.input != re_st().rex.line {
                            let this_class = mb_get_class_tab(
                                re_st().rex.input.cast(),
                                (&raw const re_st().rex.buf_mut().b_chartab).cast(),
                            );
                            let prev_class = reg_prev_class();
                            if this_class != prev_class && prev_class != 0 && prev_class != 1 {
                                add_here = true;
                                add_state = t_st.out;
                            }
                        }
                    }
                    NFA_BOF => {
                        if re_st().rex.lnum == 0
                            && re_st().rex.input == re_st().rex.line
                            && (!reg_multi() || re_st().rex.reg_firstlnum == 1)
                        {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    NFA_EOF => {
                        if re_st().rex.lnum == re_st().rex.reg_maxline && curc == NUL as i32 {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    NFA_COMPOSING => {
                        if match_composing(&prog.state, t_st.out, curc, clen) {
                            add_state = t_st.out1.and_then(|o1| (&prog.state)[o1].out);
                            add_off = clen;
                        }
                    }
                    NFA_NEWL => {
                        if curc == NUL as i32
                            && !re_st().rex.reg_line_lbr
                            && reg_multi()
                            && re_st().rex.lnum <= re_st().rex.reg_maxline
                        {
                            go_to_nextline = true;
                            add_state = t_st.out;
                            add_off = -1;
                        } else if curc == b'\n' as i32 && re_st().rex.reg_line_lbr {
                            add_state = t_st.out;
                            add_off = 1;
                        }
                    }
                    NFA_START_COLL | NFA_START_NEG_COLL => {
                        if curc != NUL as i32 {
                            let mut state = t_st.out;
                            let result_if_matched = t_st.c == NFA_START_COLL;
                            let mut matched = false;
                            while let Some(s_idx) = state {
                                let st = (&prog.state)[s_idx];
                                if st.c == NFA_COMPOSING {
                                    let t_out = t_st.out.unwrap();
                                    let t_out_st = (&prog.state)[t_out];
                                    matched =
                                        match_composing(&prog.state, t_out_st.out, curc, clen);
                                    if matched
                                        && let Some(end_idx) = t_out_st.out1
                                        && (&prog.state)[end_idx].c == NFA_END_COMPOSING
                                    {
                                        add_state = (&prog.state)[end_idx].out;
                                        add_off = clen;
                                    }
                                    break;
                                }
                                if st.c == NFA_END_COLL {
                                    matched = !result_if_matched;
                                    break;
                                }
                                if st.c == NFA_RANGE_MIN {
                                    let mut c1 = st.val;
                                    let next_idx = st.out.unwrap();
                                    let next_st = (&prog.state)[next_idx];
                                    let c2 = next_st.val;
                                    if curc >= c1 && curc <= c2 {
                                        matched = result_if_matched;
                                        break;
                                    }
                                    if re_st().rex.reg_ic {
                                        let curc_low = utf_fold(curc);
                                        let mut done = false;
                                        while c1 <= c2 {
                                            if utf_fold(c1) == curc_low {
                                                matched = result_if_matched;
                                                done = true;
                                                break;
                                            }
                                            c1 += 1;
                                        }
                                        if done {
                                            break;
                                        }
                                    }
                                    state = next_st.out;
                                    continue;
                                } else if if st.c < 0 {
                                    check_char_class(st.c, curc)
                                } else {
                                    curc == st.c
                                        || (re_st().rex.reg_ic && utf_fold(curc) == utf_fold(st.c))
                                } {
                                    matched = result_if_matched;
                                    break;
                                }
                                state = st.out;
                            }
                            if matched {
                                add_state = t_st.out1.and_then(|o1| (&prog.state)[o1].out);
                                add_off = clen;
                            }
                        }
                    }
                    NFA_ANY => {
                        if curc > 0 {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_ANY_COMPOSING => {
                        if utf_iscomposing_legacy(curc) {
                            add_off = clen;
                        } else {
                            add_here = true;
                            add_off = 0;
                        }
                        add_state = t_st.out;
                    }
                    NFA_IDENT => {
                        if vim_is_idc(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_SIDENT => {
                        if !ascii_isdigit(curc) && vim_is_idc(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_KWORD => {
                        if vim_iswordc_buf(curc, &mut *re_st().rex.reg_buf) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_SKWORD => {
                        if !ascii_isdigit(curc) && vim_iswordc_buf(curc, &mut *re_st().rex.reg_buf)
                        {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_FNAME => {
                        if vim_isfilec(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_SFNAME => {
                        if !ascii_isdigit(curc) && vim_isfilec(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_PRINT => {
                        if vim_isprintc(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_SPRINT => {
                        if !ascii_isdigit(curc) && vim_isprintc(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_WHITE => {
                        if ascii_iswhite(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NWHITE => {
                        if curc != NUL as i32 && !ascii_iswhite(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_DIGIT => {
                        if ri_digit(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NDIGIT => {
                        if curc != NUL as i32 && !ri_digit(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_HEX => {
                        if ri_hex(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NHEX => {
                        if curc != NUL as i32 && !ri_hex(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_OCTAL => {
                        if ri_octal(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NOCTAL => {
                        if curc != NUL as i32 && !ri_octal(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_WORD => {
                        if ri_word(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NWORD => {
                        if curc != NUL as i32 && !ri_word(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_HEAD => {
                        if ri_head(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NHEAD => {
                        if curc != NUL as i32 && !ri_head(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_ALPHA => {
                        if ri_alpha(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NALPHA => {
                        if curc != NUL as i32 && !ri_alpha(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_LOWER => {
                        if ri_lower(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NLOWER => {
                        if curc != NUL as i32 && !ri_lower(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_UPPER => {
                        if ri_upper(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NUPPER => {
                        if curc != NUL as i32 && !ri_upper(curc) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_LOWER_IC => {
                        if ri_lower(curc) || (re_st().rex.reg_ic && ri_upper(curc)) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NLOWER_IC => {
                        if curc != NUL as i32
                            && !(ri_lower(curc) || (re_st().rex.reg_ic && ri_upper(curc)))
                        {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_UPPER_IC => {
                        if ri_upper(curc) || (re_st().rex.reg_ic && ri_lower(curc)) {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    NFA_NUPPER_IC => {
                        if curc != NUL as i32
                            && !(ri_upper(curc) || (re_st().rex.reg_ic && ri_lower(curc)))
                        {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                    c if (NFA_BACKREF1..=NFA_ZREF9).contains(&c) => {
                        let mut bytelen: i32 = 0;
                        let matched = if (NFA_BACKREF1..=NFA_BACKREF9).contains(&c) {
                            let subidx = c - NFA_BACKREF1 + 1;
                            match_backref(&mut t.subs.norm, subidx, &mut bytelen)
                        } else {
                            let subidx = c - NFA_ZREF1 + 1;
                            match_zref(subidx, &mut bytelen)
                        };
                        if matched {
                            let out_out = t_st.out.and_then(|o| (&prog.state)[o].out);
                            if bytelen == 0 {
                                add_here = true;
                                add_state = out_out;
                            } else if bytelen <= clen {
                                add_state = out_out;
                                add_off = clen;
                            } else {
                                add_state = t_st.out;
                                add_off = bytelen;
                                add_count = bytelen - clen;
                            }
                        }
                    }
                    NFA_SKIP => {
                        if t.count - clen <= 0 {
                            add_state = t_st.out;
                            add_off = clen;
                        } else {
                            add_state = Some(t_state_idx);
                            add_off = 0;
                            add_count = t.count - clen;
                        }
                    }
                    NFA_LNUM | NFA_LNUM_GT | NFA_LNUM_LT => {
                        if reg_multi()
                            && nfa_re_num_cmp(
                                t_st.val as u64,
                                t_st.c - NFA_LNUM,
                                (re_st().rex.lnum as u64)
                                    .wrapping_add(re_st().rex.reg_firstlnum as u64),
                            )
                        {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    NFA_COL | NFA_COL_GT | NFA_COL_LT => {
                        if nfa_re_num_cmp(
                            t_st.val as u64,
                            t_st.c - NFA_COL,
                            (re_st().rex.input.offset_from(re_st().rex.line) + 1) as u64,
                        ) {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    NFA_VCOL | NFA_VCOL_GT | NFA_VCOL_LT => {
                        let op = t_st.c - NFA_VCOL;
                        let col = re_st().rex.input.offset_from(re_st().rex.line) as Colnr;
                        if !(op != 1 && col > t_st.val * MB_MAXBYTES) {
                            let mut matched = false;
                            let wp = if re_st().rex.reg_win.is_null() {
                                crate::globals::curwin_ptr()
                            } else {
                                re_st().rex.reg_win
                            };
                            if op == 1 && col - 1 > t_st.val && col > 100 {
                                let mut ts = (*wp).buf_mut().b_p_ts;
                                if ts < 4 {
                                    ts = 4;
                                }
                                matched = i64::from(col) > i64::from(t_st.val) * ts;
                            }
                            if !matched {
                                let mut lnum = if reg_multi() {
                                    re_st().rex.reg_firstlnum + re_st().rex.lnum
                                } else {
                                    1
                                };
                                if reg_multi()
                                    && (lnum <= 0 || lnum > (*wp).buf_mut().b_ml.ml_line_count)
                                {
                                    lnum = 1;
                                }
                                let vcol =
                                    win_linetabsize(&mut *wp, lnum, re_st().rex.line.cast(), col);
                                matched = nfa_re_num_cmp(
                                    t_st.val as u64,
                                    op,
                                    (vcol as u64).wrapping_add(1),
                                );
                            }
                            if matched {
                                add_here = true;
                                add_state = t_st.out;
                            }
                        }
                    }
                    NFA_MARK | NFA_MARK_GT | NFA_MARK_LT => {
                        let col = if reg_multi() {
                            re_st().rex.input.offset_from(re_st().rex.line) as usize
                        } else {
                            0
                        };
                        let fm = mark_get(
                            &mut *re_st().rex.reg_buf,
                            crate::globals::curwin_mut(),
                            None,
                            K_MARK_BUF_LOCAL,
                            t_st.val,
                        );
                        if reg_multi() {
                            re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
                            re_st().rex.input = re_st().rex.line.wrapping_add(col);
                        }
                        if !fm.is_null() && (*fm).mark.lnum > 0 {
                            let pos = &(*fm).mark;
                            let pos_col = if pos.lnum
                                == re_st().rex.lnum + re_st().rex.reg_firstlnum
                                && pos.col == MAXCOL
                            {
                                reg_getline_len(pos.lnum - re_st().rex.reg_firstlnum)
                            } else {
                                pos.col
                            };
                            let cur_col = re_st().rex.input.offset_from(re_st().rex.line) as Colnr;
                            let st_c = t_st.c;
                            let matched =
                                if pos.lnum == re_st().rex.lnum + re_st().rex.reg_firstlnum {
                                    if pos_col == cur_col {
                                        st_c == NFA_MARK
                                    } else if pos_col < cur_col {
                                        st_c == NFA_MARK_GT
                                    } else {
                                        st_c == NFA_MARK_LT
                                    }
                                } else if pos.lnum < re_st().rex.lnum + re_st().rex.reg_firstlnum {
                                    st_c == NFA_MARK_GT
                                } else {
                                    st_c == NFA_MARK_LT
                                };
                            if matched {
                                add_here = true;
                                add_state = t_st.out;
                            }
                        }
                    }
                    NFA_CURSOR => {
                        if !re_st().rex.reg_win.is_null()
                            && re_st().rex.lnum + re_st().rex.reg_firstlnum
                                == re_st().rex.win_mut().w_cursor.lnum
                            && (re_st().rex.input.offset_from(re_st().rex.line) as Colnr)
                                == re_st().rex.win_mut().w_cursor.col
                        {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    NFA_VISUAL => {
                        if reg_match_visual() {
                            add_here = true;
                            add_state = t_st.out;
                        }
                    }
                    c if (NFA_MOPEN1..=NFA_MOPEN9).contains(&c)
                        || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c)
                        || c == NFA_NOPEN
                        || c == NFA_ZSTART => {}
                    c => {
                        let mut matched = c == curc;
                        if !matched && re_st().rex.reg_ic {
                            matched = utf_fold(c) == utf_fold(curc);
                        }
                        if matched && !re_st().rex.reg_icombine {
                            clen = utf_ptr2len(re_st().rex.input.cast());
                        }
                        if matched {
                            add_state = t_st.out;
                            add_off = clen;
                        }
                    }
                }

                if let Some(add_state_idx) = add_state {
                    let mut pim: *mut NfaPim = if t.pim.result == NFA_PIM_UNUSED {
                        ptr::null_mut()
                    } else {
                        &raw mut t.pim
                    };
                    let mut pim_copy: NfaPim;

                    if !pim.is_null() && (clen == 0 || match_follows(&prog.state, add_state, 0)) {
                        let pim_state_idx = (*pim).state.unwrap();
                        let pim_c = (&prog.state)[pim_state_idx].c;
                        let is_neg = pim_c == NFA_START_INVISIBLE_NEG
                            || pim_c == NFA_START_INVISIBLE_NEG_FIRST
                            || pim_c == NFA_START_INVISIBLE_BEFORE_NEG
                            || pim_c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST;
                        let res = if (*pim).result == NFA_PIM_TODO {
                            let r = recursive_regmatch(
                                pim_state_idx,
                                pim.as_ref(),
                                &mut *prog,
                                &mut *submatch,
                                m,
                                &mut listids,
                            );
                            (*pim).result = if r != 0 {
                                NFA_PIM_MATCH
                            } else {
                                NFA_PIM_NOMATCH
                            };
                            if (r != 0) != is_neg {
                                copy_sub_off(&mut (*pim).subs.norm, &mut m.norm);
                                if re_st().rex.nfa_has_zsubexpr != 0 {
                                    copy_sub_off(&mut (*pim).subs.synt, &mut m.synt);
                                }
                            }
                            r != 0
                        } else {
                            (*pim).result == NFA_PIM_MATCH
                        };

                        if res != is_neg {
                            copy_sub_off(&mut t.subs.norm, &mut (*pim).subs.norm);
                            if re_st().rex.nfa_has_zsubexpr != 0 {
                                copy_sub_off(&mut t.subs.synt, &mut (*pim).subs.synt);
                            }
                        } else {
                            listidx += 1;
                            continue;
                        }
                        pim = ptr::null_mut();
                    }

                    if pim == &raw mut t.pim {
                        pim_copy = NfaPim::new();
                        copy_pim(&mut pim_copy, &mut *pim);
                        pim = &raw mut pim_copy;
                    }

                    let r = if add_here {
                        addstate_here(
                            &mut prog.state,
                            &mut *thislist,
                            add_state,
                            &mut t.subs,
                            pim,
                            &mut listidx,
                        )
                    } else {
                        let r = addstate_idx(
                            prog.state.as_mut_ptr(),
                            &mut *nextlist,
                            add_state_idx,
                            &raw mut t.subs,
                            pim,
                            add_off,
                            0,
                        );
                        if add_count > 0 && !r.is_null() {
                            let last_idx = (nextlist.n - 1) as usize;
                            (&mut nextlist.t)[last_idx].count = add_count;
                        }
                        r
                    };
                    if r.is_null() {
                        nfa_st().nfa_match = NFA_TOO_EXPENSIVE;
                        return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
                    }
                }

                listidx += 1;
            }

            if !goto_nextchar
                && nfa_st().nfa_match == 0
                && ((toplevel
                    && re_st().rex.lnum == 0
                    && clen != 0
                    && (re_st().rex.reg_maxcol == 0
                        || (re_st().rex.input.offset_from(re_st().rex.line) as Colnr)
                            < re_st().rex.reg_maxcol))
                    || (!re_st().nfa_endp.is_null()
                        && if reg_multi() {
                            let end_pos = (*re_st().nfa_endp).se_u.pos();
                            re_st().rex.lnum < end_pos.lnum
                                || (re_st().rex.lnum == end_pos.lnum
                                    && (re_st().rex.input.offset_from(re_st().rex.line) as Colnr)
                                        < end_pos.col)
                        } else {
                            re_st().rex.input < (*re_st().nfa_endp).se_u.ptr()
                        }))
            {
                if toplevel {
                    let mut add = true;
                    if prog.regstart != NUL as i32 && clen != 0 {
                        if nextlist.n == 0 {
                            let mut col =
                                (re_st().rex.input.offset_from(re_st().rex.line) as Colnr) + clen;
                            if !skip_to_start(prog.regstart, &mut col) {
                                break;
                            }
                            re_st().rex.input =
                                re_st().rex.line.wrapping_add((col - clen) as usize);
                        } else {
                            let c =
                                utf_ptr2char(re_st().rex.input.wrapping_add(clen as usize).cast());
                            if c != prog.regstart
                                && (!re_st().rex.reg_ic || utf_fold(c) != utf_fold(prog.regstart))
                            {
                                add = false;
                            }
                        }
                    }
                    if add {
                        if reg_multi() {
                            let start_col =
                                (re_st().rex.input.offset_from(re_st().rex.line) as Colnr) + clen;
                            m.norm.list.multi_mut()[0].start_col = start_col;
                            m.norm.orig_start_col = start_col;
                        } else {
                            m.norm.list.line_mut()[0].start =
                                re_st().rex.input.wrapping_add(clen as usize);
                        }
                        let start_out = (&prog.state)[start_idx].out;
                        if addstate(
                            &mut prog.state,
                            &mut *nextlist,
                            start_out,
                            Some(m),
                            ptr::null_mut(),
                            clen,
                            0,
                        )
                        .is_null()
                        {
                            nfa_st().nfa_match = NFA_TOO_EXPENSIVE;
                            return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
                        }
                    }
                } else {
                    let save_line = re_st().rex.line;
                    let save_input = re_st().rex.input;
                    let save_lnum = re_st().rex.lnum;

                    if reg_multi()
                        && clen == 0
                        && !re_st().nfa_endp.is_null()
                        && re_st().rex.lnum < (*re_st().nfa_endp).se_u.pos().lnum
                    {
                        let next_line = reg_getline(re_st().rex.lnum + 1);
                        if !next_line.is_null() {
                            re_st().rex.line = next_line.cast();
                            re_st().rex.input = re_st().rex.line;
                            re_st().rex.lnum += 1;
                        }
                    }

                    let r = addstate_idx(
                        prog.state.as_mut_ptr(),
                        &mut *nextlist,
                        start_idx,
                        m,
                        ptr::null_mut(),
                        clen,
                        0,
                    );

                    re_st().rex.line = save_line;
                    re_st().rex.input = save_input;
                    re_st().rex.lnum = save_lnum;

                    if r.is_null() {
                        nfa_st().nfa_match = NFA_TOO_EXPENSIVE;
                        return finish_nfa_regmatch(&mut *prog, toplevel, &mut list);
                    }
                }
            }

            if clen != 0 {
                re_st().rex.input = re_st().rex.input.wrapping_add(clen as usize);
            } else if go_to_nextline
                || (!re_st().nfa_endp.is_null()
                    && reg_multi()
                    && re_st().rex.lnum < (*re_st().nfa_endp).se_u.pos().lnum)
            {
                reg_nextline();
            } else {
                break;
            }

            reg_breakcheck();
            if g_st().got_int {
                break;
            }
            if { nfa_st().nfa_time_limit }.is_some() {
                nfa_st().nfa_time_count += 1;
                if nfa_st().nfa_time_count == 20 {
                    nfa_st().nfa_time_count = 0;
                    if nfa_did_time_out() {
                        break;
                    }
                }
            }
        }

        finish_nfa_regmatch(&mut *prog, toplevel, &mut list)
    }
}

fn finish_nfa_regmatch(prog: &mut NfaRegprog, toplevel: bool, list: &mut [NfaList; 2]) -> i32 {
    if toplevel
        && prog.listbuf[0].capacity() == 0
        && list[0].t.capacity() > 0
        && list[1].t.capacity() > 0
    {
        prog.listbuf[0] = std::mem::take(&mut list[0].t);
        prog.listbuf[1] = std::mem::take(&mut list[1].t);
    } else if !toplevel && nfa_st().rec_listbuf.len() < 8 {
        nfa_st().rec_listbuf.push([
            std::mem::take(&mut list[0].t),
            std::mem::take(&mut list[1].t),
        ]);
    }
    nfa_st().nfa_match
}

fn nfa_regtry(
    prog: &mut NfaRegprog,
    col: Colnr,
    tm: Option<Proftime>,
    timed_out: Option<&mut i32>,
) -> i32 {
    unsafe {
        let mut subs = Regsubs::new();
        let mut m = Regsubs::new();
        let start = prog.start;

        re_st().rex.input = re_st().rex.line.wrapping_add(col as usize);
        nfa_st().nfa_time_limit = tm;
        nfa_st().nfa_timed_out = timed_out.map_or(ptr::null_mut(), |p| &raw mut *p);
        nfa_st().nfa_time_count = 0;

        clear_sub(&mut subs.norm);
        clear_sub(&mut m.norm);
        clear_sub(&mut subs.synt);
        clear_sub(&mut m.synt);

        let result = nfa_regmatch(&mut *prog, start, &mut subs, &mut m);
        if result == 0 {
            return 0;
        } else if result == NFA_TOO_EXPENSIVE {
            return result;
        }

        cleanup_subexpr();
        if reg_multi() {
            let norm_multi = subs.norm.list.multi();
            for (i, item) in norm_multi[..subs.norm.in_use as usize].iter().enumerate() {
                (*re_st().rex.reg_startpos.wrapping_add(i)).lnum = item.start_lnum;
                (*re_st().rex.reg_startpos.wrapping_add(i)).col = item.start_col;
                (*re_st().rex.reg_endpos.wrapping_add(i)).lnum = item.end_lnum;
                (*re_st().rex.reg_endpos.wrapping_add(i)).col = item.end_col;
            }
            if !re_st().rex.reg_mmatch.is_null() {
                re_st().rex.mmatch_mut().rmm_matchcol = subs.norm.orig_start_col;
            }
            if (*re_st().rex.reg_startpos).lnum < 0 {
                (*re_st().rex.reg_startpos).lnum = 0;
                (*re_st().rex.reg_startpos).col = col;
            }
            if (*re_st().rex.reg_endpos).lnum < 0 {
                (*re_st().rex.reg_endpos).lnum = re_st().rex.lnum;
                (*re_st().rex.reg_endpos).col =
                    re_st().rex.input.offset_from(re_st().rex.line) as Colnr;
            } else {
                re_st().rex.lnum = (*re_st().rex.reg_endpos).lnum;
            }
        } else {
            let norm_line = subs.norm.list.line();
            for (i, item) in norm_line[..subs.norm.in_use as usize].iter().enumerate() {
                *re_st().rex.reg_startp.wrapping_add(i) = item.start;
                *re_st().rex.reg_endp.wrapping_add(i) = item.end;
            }
            if (*re_st().rex.reg_startp).is_null() {
                *re_st().rex.reg_startp = re_st().rex.line.wrapping_add(col as usize);
            }
            if (*re_st().rex.reg_endp).is_null() {
                *re_st().rex.reg_endp = re_st().rex.input;
            }
        }

        g_st().re_extmatch_out = None;

        if prog.reghasz == REX_SET {
            cleanup_zsubexpr();
            let mut extmatch = RegExtmatch::default();
            for i in 1..subs.synt.in_use as usize {
                if reg_multi() {
                    let mpos = subs.synt.list.multi()[i];
                    if mpos.start_lnum >= 0
                        && mpos.start_lnum == mpos.end_lnum
                        && mpos.end_col >= mpos.start_col
                    {
                        extmatch.matches[i] = cstring_from_bytes(
                            reg_getline(mpos.start_lnum).wrapping_add(mpos.start_col as usize),
                            (mpos.end_col - mpos.start_col) as usize,
                        );
                    }
                } else {
                    let lpos = subs.synt.list.line()[i];
                    if !lpos.start.is_null() && !lpos.end.is_null() {
                        extmatch.matches[i] = cstring_from_bytes(
                            lpos.start,
                            lpos.end.offset_from(lpos.start) as usize,
                        );
                    }
                }
            }
            g_st().re_extmatch_out = Some(std::rc::Rc::new(extmatch));
        }

        1 + re_st().rex.lnum
    }
}

fn nfa_regexec_both(
    mut line: *mut u8,
    startcol: Colnr,
    tm: Option<Proftime>,
    timed_out: Option<&mut i32>,
) -> i32 {
    unsafe {
        let mut retval: i32 = 0;
        let mut col = startcol;

        let prog: *mut NfaRegprog = if reg_multi() {
            let p = re_st().rex.mmatch_mut().prog_mut().nfa_mut();
            line = reg_getline(0).cast();
            re_st().rex.reg_startpos = (&raw mut re_st().rex.mmatch_mut().startpos).cast();
            re_st().rex.reg_endpos = (&raw mut re_st().rex.mmatch_mut().endpos).cast();
            p
        } else {
            let p = (*re_st().rex.match_mut().regprog).nfa_mut();
            re_st().rex.reg_startp = (&raw mut re_st().rex.match_mut().startp).cast();
            re_st().rex.reg_endp = (&raw mut re_st().rex.match_mut().endp).cast();
            p
        };
        if prog.is_null() || line.is_null() {
            iemsg(tr(E_NULL));
            return finish_nfa_regexec(retval, col);
        }
        let prog = &mut *prog;

        if (prog.regflags & RF_ICASE) != 0 {
            re_st().rex.reg_ic = true;
        } else if (prog.regflags & RF_NOICASE) != 0 {
            re_st().rex.reg_ic = false;
        }

        if (prog.regflags & RF_ICOMBINE) != 0 {
            re_st().rex.reg_icombine = true;
        }

        re_st().rex.line = line;
        re_st().rex.lnum = 0;

        re_st().rex.nfa_has_zend = prog.has_zend;
        re_st().rex.nfa_has_backref = prog.has_backref;
        re_st().rex.nfa_nsubexpr = prog.nsubexp;
        re_st().rex.nfa_listid = 1;
        re_st().rex.nfa_alt_listid = 2;

        if prog.reganch != 0 && col > 0 {
            return 0;
        }

        re_st().rex.need_clear_subexpr = 1;
        if prog.reghasz == REX_SET {
            re_st().rex.nfa_has_zsubexpr = 1;
            re_st().rex.need_clear_zsubexpr = 1;
        } else {
            re_st().rex.nfa_has_zsubexpr = 0;
            re_st().rex.need_clear_zsubexpr = 0;
        }

        if prog.regstart != NUL as i32 {
            if !skip_to_start(prog.regstart, &mut col) {
                return 0;
            }
            if let Some(ref mt) = prog.match_text
                && !mt.as_bytes().is_empty()
                && !re_st().rex.reg_icombine
            {
                retval = find_match_text(&mut col, prog.regstart, mt.as_ptr().cast::<u8>());
                if reg_multi() {
                    re_st().rex.mmatch_mut().rmm_matchcol = col;
                } else {
                    re_st().rex.match_mut().rm_matchcol = col;
                }
                return retval;
            }
        }

        if re_st().rex.reg_maxcol > 0 && col >= re_st().rex.reg_maxcol {
            return finish_nfa_regexec(retval, col);
        }

        re_st().nstate = 0;
        for st in &mut (&mut prog.state)[..prog.nstate as usize] {
            st.lastlist[0] = 0;
            st.lastlist[1] = 0;
        }

        retval = nfa_regtry(
            &mut *prog,
            col,
            tm,
            (timed_out.is_some()).then(|| &mut *timed_out.unwrap()),
        );
        finish_nfa_regexec(retval, col)
    }
}

fn finish_nfa_regexec(retval: i32, col: Colnr) -> i32 {
    if retval > 0 {
        if reg_multi() {
            let start = re_st().rex.mmatch_mut().startpos[0];
            let end = re_st().rex.mmatch_mut().endpos[0];
            if end.lnum < start.lnum || (end.lnum == start.lnum && end.col < start.col) {
                re_st().rex.mmatch_mut().endpos[0] = start;
            }
        } else {
            if re_st().rex.match_mut().endp[0] < re_st().rex.match_mut().startp[0] {
                re_st().rex.match_mut().endp[0] = re_st().rex.match_mut().startp[0];
            }
            re_st().rex.match_mut().rm_matchcol = col;
        }
    }
    retval
}

pub(super) fn nfa_regcomp(expr: *mut u8, re_flags: i32) -> Option<NfaRegprog> {
    if expr.is_null() {
        return None;
    }

    re_st().nfa_re_flags = re_flags;
    init_class_tab();
    nfa_regcomp_start(expr, re_flags);

    if !re2post() {
        return cleanup_nfa_regcomp(None);
    }

    let postfix = re_st().post_vec.as_ptr().cast_mut();
    let post_end_ptr = postfix.wrapping_add(re_st().post_vec.len());
    post2nfa(&mut [], postfix, post_end_ptr, true);

    let mut prog = NfaRegprog {
        regflags: 0,
        re_engine: 0,
        start: None,
        reganch: 0,
        regstart: 0,
        match_text: None,
        has_zend: 0,
        has_backref: 0,
        reghasz: 0,
        pattern: std::ffi::CString::default(),
        nsubexp: 0,
        nstate: 0,
        listbuf: [Vec::new(), Vec::new()],
        state: vec![NfaState::new(); re_st().nstate as usize],
    };

    prog.start = post2nfa(&mut prog.state, postfix, post_end_ptr, false);
    if prog.start.is_none() {
        return cleanup_nfa_regcomp(None);
    }
    prog.regflags = re_st().regflags;
    prog.nstate = re_st().nstate;
    prog.has_zend = re_st().rex.nfa_has_zend;
    prog.has_backref = re_st().rex.nfa_has_backref;
    prog.nsubexp = re_st().regnpar;

    nfa_postprocess(&mut prog);

    prog.reganch = nfa_get_reganch(&prog.state, prog.start, 0);
    prog.regstart = nfa_get_regstart(&prog.state, prog.start, 0);
    prog.match_text = nfa_get_match_text(&prog.state, prog.start);
    prog.reghasz = re_st().re_has_z;
    prog.pattern = crate::memory::cstr_to_cstring(expr.cast());

    cleanup_nfa_regcomp(Some(prog))
}

fn cleanup_nfa_regcomp(prog: Option<NfaRegprog>) -> Option<NfaRegprog> {
    if re_st().post_vec.capacity() > NFA_POSTFIX_KEEP {
        re_st().post_vec = Vec::new();
    } else {
        re_st().post_vec.clear();
    }
    prog
}

pub(super) fn nfa_regexec_nl(rmp: &mut Regmatch, line: *mut u8, col: Colnr, line_lbr: bool) -> i32 {
    re_st().rex.reg_match = rmp;
    re_st().rex.reg_mmatch = ptr::null_mut();
    re_st().rex.reg_maxline = 0;
    re_st().rex.reg_line_lbr = line_lbr;
    re_st().rex.reg_buf = g_st().curbuf;
    re_st().rex.reg_win = ptr::null_mut();
    re_st().rex.reg_ic = rmp.rm_ic;
    re_st().rex.reg_icombine = false;
    re_st().rex.reg_nobreak = (unsafe { (*rmp.regprog).re_flags } & (RE_NOBREAK as u32)) != 0;
    re_st().rex.reg_maxcol = 0;
    nfa_regexec_both(line, col, None, None)
}

pub(super) fn nfa_regexec_multi(
    rmp: &mut Regmmatch,
    win: *mut WinInfoInfo,
    buf: *mut FileBuf,
    lnum: Linenr,
    col: Colnr,
    tm: Option<Proftime>,
    timed_out: Option<&mut i32>,
) -> i32 {
    init_regexec_multi(rmp, win, buf, lnum);
    nfa_regexec_both(ptr::null_mut(), col, tm, timed_out)
}
