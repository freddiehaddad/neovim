//! NFA regular expression engine (`src/nvim/regexp.c`).

use crate::message::{semsg, siemsg};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::mem::{offset_of, size_of, zeroed};
use std::ptr;

use super::*;

const NFA_ADD_NL: c_int = 31;

const NFA_SPLIT: c_int = -1024;
const NFA_MATCH: c_int = NFA_SPLIT + 1;
const NFA_EMPTY: c_int = NFA_SPLIT + 2;

const NFA_START_COLL: c_int = NFA_SPLIT + 3;
const NFA_END_COLL: c_int = NFA_SPLIT + 4;
const NFA_START_NEG_COLL: c_int = NFA_SPLIT + 5;
const NFA_END_NEG_COLL: c_int = NFA_SPLIT + 6;
const NFA_RANGE: c_int = NFA_SPLIT + 7;
const NFA_RANGE_MIN: c_int = NFA_SPLIT + 8;
const NFA_RANGE_MAX: c_int = NFA_SPLIT + 9;

const NFA_CONCAT: c_int = NFA_SPLIT + 10;
const NFA_OR: c_int = NFA_SPLIT + 11;
const NFA_STAR: c_int = NFA_SPLIT + 12;
const NFA_STAR_NONGREEDY: c_int = NFA_SPLIT + 13;
const NFA_QUEST: c_int = NFA_SPLIT + 14;
const NFA_QUEST_NONGREEDY: c_int = NFA_SPLIT + 15;

const NFA_BOL: c_int = NFA_SPLIT + 16;
const NFA_EOL: c_int = NFA_SPLIT + 17;
const NFA_BOW: c_int = NFA_SPLIT + 18;
const NFA_EOW: c_int = NFA_SPLIT + 19;
const NFA_BOF: c_int = NFA_SPLIT + 20;
const NFA_EOF: c_int = NFA_SPLIT + 21;
const NFA_NEWL: c_int = NFA_SPLIT + 22;
const NFA_ZSTART: c_int = NFA_SPLIT + 23;
const NFA_ZEND: c_int = NFA_SPLIT + 24;
const NFA_NOPEN: c_int = NFA_SPLIT + 25;
const NFA_NCLOSE: c_int = NFA_SPLIT + 26;
const NFA_START_INVISIBLE: c_int = NFA_SPLIT + 27;
const NFA_START_INVISIBLE_FIRST: c_int = NFA_SPLIT + 28;
const NFA_START_INVISIBLE_NEG: c_int = NFA_SPLIT + 29;
const NFA_START_INVISIBLE_NEG_FIRST: c_int = NFA_SPLIT + 30;
const NFA_START_INVISIBLE_BEFORE: c_int = NFA_SPLIT + 31;
const NFA_START_INVISIBLE_BEFORE_FIRST: c_int = NFA_SPLIT + 32;
const NFA_START_INVISIBLE_BEFORE_NEG: c_int = NFA_SPLIT + 33;
const NFA_START_INVISIBLE_BEFORE_NEG_FIRST: c_int = NFA_SPLIT + 34;
const NFA_START_PATTERN: c_int = NFA_SPLIT + 35;
const NFA_END_INVISIBLE: c_int = NFA_SPLIT + 36;
const NFA_END_INVISIBLE_NEG: c_int = NFA_SPLIT + 37;
const NFA_END_PATTERN: c_int = NFA_SPLIT + 38;
const NFA_COMPOSING: c_int = NFA_SPLIT + 39;
const NFA_END_COMPOSING: c_int = NFA_SPLIT + 40;
const NFA_ANY_COMPOSING: c_int = NFA_SPLIT + 41;
const NFA_OPT_CHARS: c_int = NFA_SPLIT + 42;

const NFA_PREV_ATOM_NO_WIDTH: c_int = NFA_SPLIT + 43;
const NFA_PREV_ATOM_NO_WIDTH_NEG: c_int = NFA_SPLIT + 44;
const NFA_PREV_ATOM_JUST_BEFORE: c_int = NFA_SPLIT + 45;
const NFA_PREV_ATOM_JUST_BEFORE_NEG: c_int = NFA_SPLIT + 46;
const NFA_PREV_ATOM_LIKE_PATTERN: c_int = NFA_SPLIT + 47;

const NFA_BACKREF1: c_int = NFA_SPLIT + 48;
const NFA_BACKREF9: c_int = NFA_SPLIT + 56;
const NFA_ZREF1: c_int = NFA_SPLIT + 57;
const NFA_ZREF9: c_int = NFA_SPLIT + 65;
const NFA_SKIP: c_int = NFA_SPLIT + 66;

const NFA_MOPEN: c_int = NFA_SPLIT + 67;
const NFA_MOPEN1: c_int = NFA_SPLIT + 68;
const NFA_MOPEN9: c_int = NFA_SPLIT + 76;

const NFA_MCLOSE: c_int = NFA_SPLIT + 77;
const NFA_MCLOSE9: c_int = NFA_SPLIT + 86;

const NFA_ZOPEN: c_int = NFA_SPLIT + 87;
const NFA_ZOPEN9: c_int = NFA_SPLIT + 96;

const NFA_ZCLOSE: c_int = NFA_SPLIT + 97;
const NFA_ZCLOSE1: c_int = NFA_SPLIT + 98;
const NFA_ZCLOSE9: c_int = NFA_SPLIT + 106;

const NFA_ANY: c_int = NFA_SPLIT + 107;
const NFA_IDENT: c_int = NFA_SPLIT + 108;
const NFA_SIDENT: c_int = NFA_SPLIT + 109;
const NFA_KWORD: c_int = NFA_SPLIT + 110;
const NFA_SKWORD: c_int = NFA_SPLIT + 111;
const NFA_FNAME: c_int = NFA_SPLIT + 112;
const NFA_SFNAME: c_int = NFA_SPLIT + 113;
const NFA_PRINT: c_int = NFA_SPLIT + 114;
const NFA_SPRINT: c_int = NFA_SPLIT + 115;
const NFA_WHITE: c_int = NFA_SPLIT + 116;
const NFA_NWHITE: c_int = NFA_SPLIT + 117;
const NFA_DIGIT: c_int = NFA_SPLIT + 118;
const NFA_NDIGIT: c_int = NFA_SPLIT + 119;
const NFA_HEX: c_int = NFA_SPLIT + 120;
const NFA_NHEX: c_int = NFA_SPLIT + 121;
const NFA_OCTAL: c_int = NFA_SPLIT + 122;
const NFA_NOCTAL: c_int = NFA_SPLIT + 123;
const NFA_WORD: c_int = NFA_SPLIT + 124;
const NFA_NWORD: c_int = NFA_SPLIT + 125;
const NFA_HEAD: c_int = NFA_SPLIT + 126;
const NFA_NHEAD: c_int = NFA_SPLIT + 127;
const NFA_ALPHA: c_int = NFA_SPLIT + 128;
const NFA_NALPHA: c_int = NFA_SPLIT + 129;
const NFA_LOWER: c_int = NFA_SPLIT + 130;
const NFA_NLOWER: c_int = NFA_SPLIT + 131;
const NFA_UPPER: c_int = NFA_SPLIT + 132;
const NFA_NUPPER: c_int = NFA_SPLIT + 133;
const NFA_LOWER_IC: c_int = NFA_SPLIT + 134;
const NFA_NLOWER_IC: c_int = NFA_SPLIT + 135;
const NFA_UPPER_IC: c_int = NFA_SPLIT + 136;
const NFA_NUPPER_IC: c_int = NFA_SPLIT + 137;

const NFA_FIRST_NL: c_int = NFA_ANY + NFA_ADD_NL;
const NFA_LAST_NL: c_int = NFA_NUPPER_IC + NFA_ADD_NL;

const NFA_CURSOR: c_int = NFA_LAST_NL + 1;
const NFA_LNUM: c_int = NFA_CURSOR + 1;
const NFA_LNUM_GT: c_int = NFA_CURSOR + 2;
const NFA_LNUM_LT: c_int = NFA_CURSOR + 3;
const NFA_COL: c_int = NFA_CURSOR + 4;
const NFA_COL_GT: c_int = NFA_CURSOR + 5;
const NFA_COL_LT: c_int = NFA_CURSOR + 6;
const NFA_VCOL: c_int = NFA_CURSOR + 7;
const NFA_VCOL_GT: c_int = NFA_CURSOR + 8;
const NFA_VCOL_LT: c_int = NFA_CURSOR + 9;
const NFA_MARK: c_int = NFA_CURSOR + 10;
const NFA_MARK_GT: c_int = NFA_CURSOR + 11;
const NFA_MARK_LT: c_int = NFA_CURSOR + 12;
const NFA_VISUAL: c_int = NFA_CURSOR + 13;

const NFA_CLASS_ALNUM: c_int = NFA_CURSOR + 14;
const NFA_CLASS_ALPHA: c_int = NFA_CURSOR + 15;
const NFA_CLASS_BLANK: c_int = NFA_CURSOR + 16;
const NFA_CLASS_CNTRL: c_int = NFA_CURSOR + 17;
const NFA_CLASS_DIGIT: c_int = NFA_CURSOR + 18;
const NFA_CLASS_GRAPH: c_int = NFA_CURSOR + 19;
const NFA_CLASS_LOWER: c_int = NFA_CURSOR + 20;
const NFA_CLASS_PRINT: c_int = NFA_CURSOR + 21;
const NFA_CLASS_PUNCT: c_int = NFA_CURSOR + 22;
const NFA_CLASS_SPACE: c_int = NFA_CURSOR + 23;
const NFA_CLASS_UPPER: c_int = NFA_CURSOR + 24;
const NFA_CLASS_XDIGIT: c_int = NFA_CURSOR + 25;
const NFA_CLASS_TAB: c_int = NFA_CURSOR + 26;
const NFA_CLASS_RETURN: c_int = NFA_CURSOR + 27;
const NFA_CLASS_BACKSPACE: c_int = NFA_CURSOR + 28;
const NFA_CLASS_ESCAPE: c_int = NFA_CURSOR + 29;
const NFA_CLASS_IDENT: c_int = NFA_CURSOR + 30;
const NFA_CLASS_KEYWORD: c_int = NFA_CURSOR + 31;
const NFA_CLASS_FNAME: c_int = NFA_CURSOR + 32;

const NFA_POSTFIX_KEEP: usize = 10000;
const NFA_STACK_KEEP: c_int = 4000;

const NFA_PIM_UNUSED: c_int = 0;
const NFA_PIM_TODO: c_int = 1;
const NFA_PIM_MATCH: c_int = 2;
const NFA_PIM_NOMATCH: c_int = 3;

const ADDSTATE_HERE_OFFSET: c_int = 10;

static NFA_CLASSCODES: [c_int; 27] = [
    NFA_ANY, NFA_IDENT, NFA_SIDENT, NFA_KWORD, NFA_SKWORD, NFA_FNAME, NFA_SFNAME, NFA_PRINT,
    NFA_SPRINT, NFA_WHITE, NFA_NWHITE, NFA_DIGIT, NFA_NDIGIT, NFA_HEX, NFA_NHEX, NFA_OCTAL,
    NFA_NOCTAL, NFA_WORD, NFA_NWORD, NFA_HEAD, NFA_NHEAD, NFA_ALPHA, NFA_NALPHA, NFA_LOWER,
    NFA_NLOWER, NFA_UPPER, NFA_NUPPER,
];

const e_nul_found: &CStr = c"E865: (NFA) Regexp end encountered prematurely";
const e_misplaced: &CStr = c"E866: (NFA regexp) Misplaced %c";
const e_ill_char_class: &CStr = c"E877: (NFA regexp) Invalid character class: %lld";
const e_value_too_large: &CStr = c"E951: \\% value too large";

static mut empty_frag: Frag_T = Frag_T {
    start: ptr::null_mut(),
    out: ptr::null_mut(),
};
static mut temp_subs: regsubs_T = unsafe { zeroed() };
static mut nfa_match: c_int = 0;
static mut nfa_time_limit: *mut proftime_T = ptr::null_mut();
static mut nfa_timed_out: *mut c_int = ptr::null_mut();
static mut nfa_time_count: c_int = 0;

#[inline]
unsafe fn EMIT(c: c_int) {
    unsafe {
        if post_ptr >= post_end {
            realloc_post_list();
        }
        *post_ptr = c;
        post_ptr = post_ptr.add(1);
    }
}

#[inline]
unsafe fn EMIT2(c: c_int) {
    unsafe {
        EMIT(c);
        EMIT(NFA_CONCAT);
    }
}

unsafe fn nfa_regcomp_start(expr: *mut u8, re_flags: c_int) {
    unsafe {
        nstate = 0;
        istate = 0;
        let nstate_max = (libc::strlen(expr.cast()) + 1) * 25 + 1000;
        let postfix_size = size_of::<c_int>() * nstate_max;

        if post_start.is_null() || post_start_len < nstate_max {
            xfree(post_start.cast());
            post_start = xmalloc(postfix_size).cast();
            post_start_len = nstate_max;
        }
        post_ptr = post_start;
        post_end = post_start.add(post_start_len);
        wants_nfa = false;
        rex.nfa_has_zend = 0;
        rex.nfa_has_backref = 0;
        nfa_reg_parse_depth = 0;

        regcomp_start(expr, re_flags);
    }
}

unsafe fn nfa_get_reganch(start: *mut nfa_state_T, depth: c_int) -> c_int {
    unsafe {
        let mut p = start;
        if depth > 4 {
            return 0;
        }
        while !p.is_null() {
            match (*p).c {
                NFA_BOL | NFA_BOF => return 1,
                NFA_ZSTART | NFA_ZEND | NFA_CURSOR | NFA_VISUAL | NFA_NOPEN => {
                    p = (*p).out;
                }
                c if (NFA_MOPEN..=NFA_MOPEN9).contains(&c)
                    || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) =>
                {
                    p = (*p).out;
                }
                NFA_SPLIT => {
                    return c_int::from(
                        nfa_get_reganch((*p).out, depth + 1) != 0
                            && nfa_get_reganch((*p).out1, depth + 1) != 0,
                    );
                }
                _ => return 0,
            }
        }
        0
    }
}

unsafe fn nfa_get_regstart(start: *mut nfa_state_T, depth: c_int) -> c_int {
    unsafe {
        let mut p = start;
        if depth > 4 {
            return 0;
        }
        while !p.is_null() {
            match (*p).c {
                NFA_BOL | NFA_BOF | NFA_BOW | NFA_EOW | NFA_ZSTART | NFA_ZEND | NFA_CURSOR
                | NFA_VISUAL | NFA_LNUM | NFA_LNUM_GT | NFA_LNUM_LT | NFA_COL | NFA_COL_GT
                | NFA_COL_LT | NFA_VCOL | NFA_VCOL_GT | NFA_VCOL_LT | NFA_MARK | NFA_MARK_GT
                | NFA_MARK_LT | NFA_NOPEN => {
                    p = (*p).out;
                }
                c if (NFA_MOPEN..=NFA_MOPEN9).contains(&c)
                    || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) =>
                {
                    p = (*p).out;
                }
                NFA_SPLIT => {
                    let c1 = nfa_get_regstart((*p).out, depth + 1);
                    let c2 = nfa_get_regstart((*p).out1, depth + 1);
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
}

unsafe fn nfa_get_match_text(start: *mut nfa_state_T) -> *mut u8 {
    unsafe {
        let mut p = start;
        let mut len: usize = 0;

        if (*p).c != NFA_MOPEN {
            return ptr::null_mut();
        }
        p = (*p).out;
        while (*p).c > 0 {
            len += utf_char2len((*p).c) as usize;
            p = (*p).out;
        }
        if (*p).c != NFA_MCLOSE || (*(*p).out).c != NFA_MATCH {
            return ptr::null_mut();
        }

        let ret = xmalloc(len).cast::<u8>();
        p = (*(*start).out).out;
        let mut s = ret;
        while (*p).c > 0 {
            s = s.add(utf_char2bytes((*p).c, s.cast()) as usize);
            p = (*p).out;
        }
        *s = NUL;
        ret
    }
}

unsafe fn realloc_post_list() {
    unsafe {
        let old_len = post_end.offset_from(post_start) as usize;
        let new_max = old_len * 3 / 2;
        let pos = post_ptr.offset_from(post_start) as usize;
        let new_start = xrealloc(post_start.cast(), new_max * size_of::<c_int>()).cast::<c_int>();
        post_ptr = new_start.add(pos);
        post_end = new_start.add(new_max);
        post_start = new_start;
        post_start_len = new_max;
    }
}

unsafe fn nfa_recognize_char_class(start: *mut u8, end: *const u8, mut extra_newl: c_int) -> c_int {
    unsafe {
        let mut newl = false;
        let config = get_char_class_bits(start, end, &raw mut newl);
        if config < 0 {
            return FAIL;
        }
        if extra_newl == 1 {
            newl = true;
        }
        if newl {
            extra_newl = NFA_ADD_NL;
        }
        if config == CLASS_o9 {
            extra_newl + NFA_DIGIT
        } else if config == (CLASS_not | CLASS_o9) {
            extra_newl + NFA_NDIGIT
        } else if config == (CLASS_af | CLASS_AF | CLASS_o9) {
            extra_newl + NFA_HEX
        } else if config == (CLASS_not | CLASS_af | CLASS_AF | CLASS_o9) {
            extra_newl + NFA_NHEX
        } else if config == CLASS_o7 {
            extra_newl + NFA_OCTAL
        } else if config == (CLASS_not | CLASS_o7) {
            extra_newl + NFA_NOCTAL
        } else if config == (CLASS_az | CLASS_AZ | CLASS_o9 | CLASS_underscore) {
            extra_newl + NFA_WORD
        } else if config == (CLASS_not | CLASS_az | CLASS_AZ | CLASS_o9 | CLASS_underscore) {
            extra_newl + NFA_NWORD
        } else if config == (CLASS_az | CLASS_AZ | CLASS_underscore) {
            extra_newl + NFA_HEAD
        } else if config == (CLASS_not | CLASS_az | CLASS_AZ | CLASS_underscore) {
            extra_newl + NFA_NHEAD
        } else if config == (CLASS_az | CLASS_AZ) {
            extra_newl + NFA_ALPHA
        } else if config == (CLASS_not | CLASS_az | CLASS_AZ) {
            extra_newl + NFA_NALPHA
        } else if config == CLASS_az {
            extra_newl + NFA_LOWER_IC
        } else if config == (CLASS_not | CLASS_az) {
            extra_newl + NFA_NLOWER_IC
        } else if config == CLASS_AZ {
            extra_newl + NFA_UPPER_IC
        } else if config == (CLASS_not | CLASS_AZ) {
            extra_newl + NFA_NUPPER_IC
        } else {
            FAIL
        }
    }
}

#[rustfmt::skip]
unsafe fn nfa_emit_equi_class(c: c_int) {
    unsafe {
        match c {
            0x41 | 0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc4 | 0xc5 | 0x100 | 0x102 | 0x104 | 0x1cd
            | 0x1de | 0x1e0 | 0x1fa | 0x200 | 0x202 | 0x226 | 0x23a | 0x1e00 | 0x1ea0 | 0x1ea2
            | 0x1ea4 | 0x1ea6 | 0x1ea8 | 0x1eaa | 0x1eac | 0x1eae | 0x1eb0 | 0x1eb2 | 0x1eb4
            | 0x1eb6 => {
                EMIT2(b'A' as c_int); EMIT2(0xc0); EMIT2(0xc1);
                EMIT2(0xc2); EMIT2(0xc3);
                EMIT2(0xc4); EMIT2(0xc5);
                EMIT2(0x100); EMIT2(0x102); EMIT2(0x104);
                EMIT2(0x1cd); EMIT2(0x1de); EMIT2(0x1e0);
                EMIT2(0x1fa); EMIT2(0x200); EMIT2(0x202);
                EMIT2(0x226); EMIT2(0x23a); EMIT2(0x1e00);
                EMIT2(0x1ea0); EMIT2(0x1ea2); EMIT2(0x1ea4);
                EMIT2(0x1ea6); EMIT2(0x1ea8); EMIT2(0x1eaa);
                EMIT2(0x1eac); EMIT2(0x1eae); EMIT2(0x1eb0);
                EMIT2(0x1eb2); EMIT2(0x1eb6); EMIT2(0x1eb4);
                return;
            }
            0x42 | 0x181 | 0x243 | 0x1e02 | 0x1e04 | 0x1e06 => {
                EMIT2(b'B' as c_int);
                EMIT2(0x181); EMIT2(0x243); EMIT2(0x1e02);
                EMIT2(0x1e04); EMIT2(0x1e06);
                return;
            }
            0x43 | 0xc7 | 0x106 | 0x108 | 0x10a | 0x10c | 0x187 | 0x23b | 0x1e08 | 0xa792 => {
                EMIT2(b'C' as c_int); EMIT2(0xc7);
                EMIT2(0x106); EMIT2(0x108); EMIT2(0x10a);
                EMIT2(0x10c); EMIT2(0x187); EMIT2(0x23b);
                EMIT2(0x1e08); EMIT2(0xa792);
                return;
            }
            0x44 | 0x10e | 0x110 | 0x18a | 0x1e0a | 0x1e0c | 0x1e0e | 0x1e10 | 0x1e12 => {
                EMIT2(b'D' as c_int); EMIT2(0x10e); EMIT2(0x110); EMIT2(0x18a);
                EMIT2(0x1e0a); EMIT2(0x1e0c); EMIT2(0x1e0e);
                EMIT2(0x1e10); EMIT2(0x1e12);
                return;
            }
            0x45 | 0xc8 | 0xc9 | 0xca | 0xcb | 0x112 | 0x114 | 0x116 | 0x118 | 0x11a | 0x204
            | 0x206 | 0x228 | 0x246 | 0x1e14 | 0x1e16 | 0x1e18 | 0x1e1a | 0x1e1c | 0x1eb8
            | 0x1eba | 0x1ebc | 0x1ebe | 0x1ec0 | 0x1ec2 | 0x1ec4 | 0x1ec6 => {
                EMIT2(b'E' as c_int); EMIT2(0xc8); EMIT2(0xc9);
                EMIT2(0xca); EMIT2(0xcb);
                EMIT2(0x112); EMIT2(0x114); EMIT2(0x116);
                EMIT2(0x118); EMIT2(0x11a); EMIT2(0x204);
                EMIT2(0x206); EMIT2(0x228); EMIT2(0x246);
                EMIT2(0x1e14); EMIT2(0x1e16); EMIT2(0x1e18);
                EMIT2(0x1e1a); EMIT2(0x1e1c); EMIT2(0x1eb8);
                EMIT2(0x1eba); EMIT2(0x1ebc); EMIT2(0x1ebe);
                EMIT2(0x1ec0); EMIT2(0x1ec2); EMIT2(0x1ec4);
                EMIT2(0x1ec6);
                return;
            }
            0x46 | 0x191 | 0x1e1e | 0xa798 => {
                EMIT2(b'F' as c_int); EMIT2(0x191); EMIT2(0x1e1e); EMIT2(0xa798);
                return;
            }
            0x47 | 0x11c | 0x11e | 0x120 | 0x122 | 0x193 | 0x1e4 | 0x1e6 | 0x1f4 | 0x1e20
            | 0xa7a0 => {
                EMIT2(b'G' as c_int); EMIT2(0x11c); EMIT2(0x11e); EMIT2(0x120);
                EMIT2(0x122); EMIT2(0x193); EMIT2(0x1e4);
                EMIT2(0x1e6); EMIT2(0x1f4); EMIT2(0x1e20);
                EMIT2(0xa7a0);
                return;
            }
            0x48 | 0x124 | 0x126 | 0x21e | 0x1e22 | 0x1e24 | 0x1e26 | 0x1e28 | 0x1e2a | 0x2c67 => {
                EMIT2(b'H' as c_int); EMIT2(0x124); EMIT2(0x126); EMIT2(0x21e);
                EMIT2(0x1e22); EMIT2(0x1e24); EMIT2(0x1e26);
                EMIT2(0x1e28); EMIT2(0x1e2a); EMIT2(0x2c67);
                return;
            }
            0x49 | 0xcc | 0xcd | 0xce | 0xcf | 0x128 | 0x12a | 0x12c | 0x12e | 0x130 | 0x197
            | 0x1cf | 0x208 | 0x20a | 0x1e2c | 0x1e2e | 0x1ec8 | 0x1eca => {
                EMIT2(b'I' as c_int); EMIT2(0xcc); EMIT2(0xcd);
                EMIT2(0xce); EMIT2(0xcf);
                EMIT2(0x128); EMIT2(0x12a); EMIT2(0x12c);
                EMIT2(0x12e); EMIT2(0x130); EMIT2(0x197);
                EMIT2(0x1cf); EMIT2(0x208); EMIT2(0x20a);
                EMIT2(0x1e2c); EMIT2(0x1e2e); EMIT2(0x1ec8);
                EMIT2(0x1eca);
                return;
            }
            0x4a | 0x134 | 0x248 => {
                EMIT2(b'J' as c_int); EMIT2(0x134); EMIT2(0x248);
                return;
            }
            0x4b | 0x136 | 0x198 | 0x1e8 | 0x1e30 | 0x1e32 | 0x1e34 | 0x2c69 | 0xa740 => {
                EMIT2(b'K' as c_int); EMIT2(0x136); EMIT2(0x198); EMIT2(0x1e8);
                EMIT2(0x1e30); EMIT2(0x1e32); EMIT2(0x1e34);
                EMIT2(0x2c69); EMIT2(0xa740);
                return;
            }
            0x4c | 0x139 | 0x13b | 0x13d | 0x13f | 0x141 | 0x23d | 0x1e36 | 0x1e38 | 0x1e3a
            | 0x1e3c | 0x2c60 => {
                EMIT2(b'L' as c_int); EMIT2(0x139); EMIT2(0x13b);
                EMIT2(0x13d); EMIT2(0x13f); EMIT2(0x141);
                EMIT2(0x23d); EMIT2(0x1e36); EMIT2(0x1e38);
                EMIT2(0x1e3a); EMIT2(0x1e3c); EMIT2(0x2c60);
                return;
            }
            0x4d | 0x1e3e | 0x1e40 | 0x1e42 => {
                EMIT2(b'M' as c_int); EMIT2(0x1e3e); EMIT2(0x1e40);
                EMIT2(0x1e42);
                return;
            }
            0x4e | 0xd1 | 0x143 | 0x145 | 0x147 | 0x1f8 | 0x1e44 | 0x1e46 | 0x1e48 | 0x1e4a
            | 0xa7a4 => {
                EMIT2(b'N' as c_int); EMIT2(0xd1);
                EMIT2(0x143); EMIT2(0x145); EMIT2(0x147);
                EMIT2(0x1f8); EMIT2(0x1e44); EMIT2(0x1e46);
                EMIT2(0x1e48); EMIT2(0x1e4a); EMIT2(0xa7a4);
                return;
            }
            0x4f | 0xd2 | 0xd3 | 0xd4 | 0xd5 | 0xd6 | 0xd8 | 0x14c | 0x14e | 0x150 | 0x19f
            | 0x1a0 | 0x1d1 | 0x1ea | 0x1ec | 0x1fe | 0x20c | 0x20e | 0x22a | 0x22c | 0x22e
            | 0x230 | 0x1e4c | 0x1e4e | 0x1e50 | 0x1e52 | 0x1ecc | 0x1ece | 0x1ed0 | 0x1ed2
            | 0x1ed4 | 0x1ed6 | 0x1ed8 | 0x1eda | 0x1edc | 0x1ede | 0x1ee0 | 0x1ee2 => {
                EMIT2(b'O' as c_int); EMIT2(0xd2); EMIT2(0xd3);
                EMIT2(0xd4); EMIT2(0xd5);
                EMIT2(0xd6); EMIT2(0xd8);
                EMIT2(0x14c); EMIT2(0x14e); EMIT2(0x150);
                EMIT2(0x19f); EMIT2(0x1a0); EMIT2(0x1d1);
                EMIT2(0x1ea); EMIT2(0x1ec); EMIT2(0x1fe);
                EMIT2(0x20c); EMIT2(0x20e); EMIT2(0x22a);
                EMIT2(0x22c); EMIT2(0x22e); EMIT2(0x230);
                EMIT2(0x1e4c); EMIT2(0x1e4e); EMIT2(0x1e50);
                EMIT2(0x1e52); EMIT2(0x1ecc); EMIT2(0x1ece);
                EMIT2(0x1ed0); EMIT2(0x1ed2); EMIT2(0x1ed4);
                EMIT2(0x1ed6); EMIT2(0x1ed8); EMIT2(0x1eda);
                EMIT2(0x1edc); EMIT2(0x1ede); EMIT2(0x1ee0);
                EMIT2(0x1ee2);
                return;
            }
            0x50 | 0x1a4 | 0x1e54 | 0x1e56 | 0x2c63 => {
                EMIT2(b'P' as c_int); EMIT2(0x1a4); EMIT2(0x1e54); EMIT2(0x1e56);
                EMIT2(0x2c63);
                return;
            }
            0x51 | 0x24a => {
                EMIT2(b'Q' as c_int); EMIT2(0x24a);
                return;
            }
            0x52 | 0x154 | 0x156 | 0x158 | 0x210 | 0x212 | 0x24c | 0x1e58 | 0x1e5a | 0x1e5c
            | 0x1e5e | 0x2c64 | 0xa7a6 => {
                EMIT2(b'R' as c_int); EMIT2(0x154); EMIT2(0x156); EMIT2(0x158);
                EMIT2(0x210); EMIT2(0x212); EMIT2(0x24c); EMIT2(0x1e58);
                EMIT2(0x1e5a); EMIT2(0x1e5c); EMIT2(0x1e5e); EMIT2(0x2c64);
                EMIT2(0xa7a6);
                return;
            }
            0x53 | 0x15a | 0x15c | 0x15e | 0x160 | 0x218 | 0x1e60 | 0x1e62 | 0x1e64 | 0x1e66
            | 0x1e68 | 0x2c7e | 0xa7a8 => {
                EMIT2(b'S' as c_int); EMIT2(0x15a); EMIT2(0x15c); EMIT2(0x15e);
                EMIT2(0x160); EMIT2(0x218); EMIT2(0x1e60); EMIT2(0x1e62);
                EMIT2(0x1e64); EMIT2(0x1e66); EMIT2(0x1e68); EMIT2(0x2c7e);
                EMIT2(0xa7a8);
                return;
            }
            0x54 | 0x162 | 0x164 | 0x166 | 0x1ac | 0x1ae | 0x21a | 0x23e | 0x1e6a | 0x1e6c
            | 0x1e6e | 0x1e70 => {
                EMIT2(b'T' as c_int); EMIT2(0x162); EMIT2(0x164); EMIT2(0x166);
                EMIT2(0x1ac); EMIT2(0x1ae); EMIT2(0x23e); EMIT2(0x21a);
                EMIT2(0x1e6a); EMIT2(0x1e6c); EMIT2(0x1e6e); EMIT2(0x1e70);
                return;
            }
            0x55 | 0xd9 | 0xda | 0xdb | 0xdc | 0x168 | 0x16a | 0x16c | 0x16e | 0x170 | 0x172
            | 0x1af | 0x1d3 | 0x1d5 | 0x1d7 | 0x1d9 | 0x1db | 0x214 | 0x216 | 0x244 | 0x1e72
            | 0x1e74 | 0x1e76 | 0x1e78 | 0x1e7a | 0x1ee4 | 0x1ee6 | 0x1ee8 | 0x1eea | 0x1eec
            | 0x1eee | 0x1ef0 => {
                EMIT2(b'U' as c_int); EMIT2(0xd9); EMIT2(0xda);
                EMIT2(0xdc); EMIT2(0xdb);
                EMIT2(0x168); EMIT2(0x16a);
                EMIT2(0x16c); EMIT2(0x16e); EMIT2(0x170);
                EMIT2(0x172); EMIT2(0x1af); EMIT2(0x1d3);
                EMIT2(0x1d5); EMIT2(0x1d7); EMIT2(0x1d9);
                EMIT2(0x1db); EMIT2(0x214); EMIT2(0x216);
                EMIT2(0x244); EMIT2(0x1e72); EMIT2(0x1e74);
                EMIT2(0x1e76); EMIT2(0x1e78); EMIT2(0x1e7a);
                EMIT2(0x1ee4); EMIT2(0x1ee6); EMIT2(0x1ee8);
                EMIT2(0x1eea); EMIT2(0x1eec); EMIT2(0x1eee);
                EMIT2(0x1ef0);
                return;
            }
            0x56 | 0x1b2 | 0x1e7c | 0x1e7e => {
                EMIT2(b'V' as c_int); EMIT2(0x1b2); EMIT2(0x1e7c); EMIT2(0x1e7e);
                return;
            }
            0x57 | 0x174 | 0x1e80 | 0x1e82 | 0x1e84 | 0x1e86 | 0x1e88 => {
                EMIT2(b'W' as c_int); EMIT2(0x174); EMIT2(0x1e80); EMIT2(0x1e82);
                EMIT2(0x1e84); EMIT2(0x1e86); EMIT2(0x1e88);
                return;
            }
            0x58 | 0x1e8a | 0x1e8c => {
                EMIT2(b'X' as c_int); EMIT2(0x1e8a); EMIT2(0x1e8c);
                return;
            }
            0x59 | 0xdd | 0x176 | 0x178 | 0x1b3 | 0x232 | 0x24e | 0x1e8e | 0x1ef2 | 0x1ef4
            | 0x1ef6 | 0x1ef8 => {
                EMIT2(b'Y' as c_int); EMIT2(0xdd);
                EMIT2(0x176); EMIT2(0x178); EMIT2(0x1b3);
                EMIT2(0x232); EMIT2(0x24e); EMIT2(0x1e8e);
                EMIT2(0x1ef2); EMIT2(0x1ef4); EMIT2(0x1ef6);
                EMIT2(0x1ef8);
                return;
            }
            0x5a | 0x179 | 0x17b | 0x17d | 0x1b5 | 0x1e90 | 0x1e92 | 0x1e94 | 0x2c6b => {
                EMIT2(b'Z' as c_int); EMIT2(0x179); EMIT2(0x17b); EMIT2(0x17d);
                EMIT2(0x1b5); EMIT2(0x1e90); EMIT2(0x1e92);
                EMIT2(0x1e94); EMIT2(0x2c6b);
                return;
            }
            0x61 | 0xe0 | 0xe1 | 0xe2 | 0xe3 | 0xe4 | 0xe5 | 0x101 | 0x103 | 0x105 | 0x1ce
            | 0x1df | 0x1e1 | 0x1fb | 0x201 | 0x203 | 0x227 | 0x1d8f | 0x1e01 | 0x1e9a
            | 0x1ea1 | 0x1ea3 | 0x1ea5 | 0x1ea7 | 0x1ea9 | 0x1eab | 0x1ead | 0x1eaf | 0x1eb1
            | 0x1eb3 | 0x1eb5 | 0x1eb7 | 0x2c65 => {
                EMIT2(b'a' as c_int); EMIT2(0xe0); EMIT2(0xe1);
                EMIT2(0xe2); EMIT2(0xe3);
                EMIT2(0xe4); EMIT2(0xe5);
                EMIT2(0x101); EMIT2(0x103); EMIT2(0x105);
                EMIT2(0x1ce); EMIT2(0x1df); EMIT2(0x1e1);
                EMIT2(0x1fb); EMIT2(0x201); EMIT2(0x203);
                EMIT2(0x227); EMIT2(0x1d8f); EMIT2(0x1e01);
                EMIT2(0x1e9a); EMIT2(0x1ea1); EMIT2(0x1ea3);
                EMIT2(0x1ea5); EMIT2(0x1ea7); EMIT2(0x1ea9);
                EMIT2(0x1eab); EMIT2(0x1ead); EMIT2(0x1eaf);
                EMIT2(0x1eb1); EMIT2(0x1eb3); EMIT2(0x1eb5);
                EMIT2(0x1eb7); EMIT2(0x2c65);
                return;
            }
            0x62 | 0x180 | 0x253 | 0x1d6c | 0x1d80 | 0x1e03 | 0x1e05 | 0x1e07 => {
                EMIT2(b'b' as c_int); EMIT2(0x180); EMIT2(0x253); EMIT2(0x1d6c);
                EMIT2(0x1d80); EMIT2(0x1e03); EMIT2(0x1e05); EMIT2(0x1e07);
                return;
            }
            0x63 | 0xe7 | 0x107 | 0x109 | 0x10b | 0x10d | 0x188 | 0x23c | 0x1e09 | 0xa793
            | 0xa794 => {
                EMIT2(b'c' as c_int); EMIT2(0xe7);
                EMIT2(0x107); EMIT2(0x109); EMIT2(0x10b);
                EMIT2(0x10d); EMIT2(0x188); EMIT2(0x23c);
                EMIT2(0x1e09); EMIT2(0xa793); EMIT2(0xa794);
                return;
            }
            0x64 | 0x10f | 0x111 | 0x257 | 0x1d6d | 0x1d81 | 0x1d91 | 0x1e0b | 0x1e0d | 0x1e0f
            | 0x1e11 | 0x1e13 => {
                EMIT2(b'd' as c_int); EMIT2(0x10f); EMIT2(0x111);
                EMIT2(0x257); EMIT2(0x1d6d); EMIT2(0x1d81);
                EMIT2(0x1d91); EMIT2(0x1e0b); EMIT2(0x1e0d);
                EMIT2(0x1e0f); EMIT2(0x1e11); EMIT2(0x1e13);
                return;
            }
            0x65 | 0xe8 | 0xe9 | 0xea | 0xeb | 0x113 | 0x115 | 0x117 | 0x119 | 0x11b | 0x205
            | 0x207 | 0x229 | 0x247 | 0x1d92 | 0x1e15 | 0x1e17 | 0x1e19 | 0x1e1b | 0x1e1d
            | 0x1eb9 | 0x1ebb | 0x1ebd | 0x1ebf | 0x1ec1 | 0x1ec3 | 0x1ec5 | 0x1ec7 => {
                EMIT2(b'e' as c_int); EMIT2(0xe8); EMIT2(0xe9);
                EMIT2(0xea); EMIT2(0xeb);
                EMIT2(0x113); EMIT2(0x115);
                EMIT2(0x117); EMIT2(0x119); EMIT2(0x11b);
                EMIT2(0x205); EMIT2(0x207); EMIT2(0x229);
                EMIT2(0x247); EMIT2(0x1d92); EMIT2(0x1e15);
                EMIT2(0x1e17); EMIT2(0x1e19); EMIT2(0x1e1b);
                EMIT2(0x1e1d); EMIT2(0x1eb9); EMIT2(0x1ebb);
                EMIT2(0x1ebd); EMIT2(0x1ebf); EMIT2(0x1ec1);
                EMIT2(0x1ec3); EMIT2(0x1ec5); EMIT2(0x1ec7);
                return;
            }
            0x66 | 0x192 | 0x1d6e | 0x1d82 | 0x1e1f | 0xa799 => {
                EMIT2(b'f' as c_int); EMIT2(0x192); EMIT2(0x1d6e); EMIT2(0x1d82);
                EMIT2(0x1e1f); EMIT2(0xa799);
                return;
            }
            0x67 | 0x11d | 0x11f | 0x121 | 0x123 | 0x1e5 | 0x1e7 | 0x1f5 | 0x260 | 0x1d83
            | 0x1e21 | 0xa7a1 => {
                EMIT2(b'g' as c_int); EMIT2(0x11d); EMIT2(0x11f); EMIT2(0x121);
                EMIT2(0x123); EMIT2(0x1e5); EMIT2(0x1e7);
                EMIT2(0x1f5); EMIT2(0x260); EMIT2(0x1d83);
                EMIT2(0x1e21); EMIT2(0xa7a1);
                return;
            }
            0x68 | 0x125 | 0x127 | 0x21f | 0x1e23 | 0x1e25 | 0x1e27 | 0x1e29 | 0x1e2b | 0x1e96
            | 0x2c68 | 0xa795 => {
                EMIT2(b'h' as c_int); EMIT2(0x125); EMIT2(0x127); EMIT2(0x21f);
                EMIT2(0x1e23); EMIT2(0x1e25); EMIT2(0x1e27);
                EMIT2(0x1e29); EMIT2(0x1e2b); EMIT2(0x1e96);
                EMIT2(0x2c68); EMIT2(0xa795);
                return;
            }
            0x69 | 0xec | 0xed | 0xee | 0xef | 0x129 | 0x12b | 0x12d | 0x12f | 0x1d0 | 0x209
            | 0x20b | 0x268 | 0x1d96 | 0x1e2d | 0x1e2f | 0x1ec9 | 0x1ecb => {
                EMIT2(b'i' as c_int); EMIT2(0xec); EMIT2(0xed);
                EMIT2(0xee); EMIT2(0xef);
                EMIT2(0x129); EMIT2(0x12b); EMIT2(0x12d);
                EMIT2(0x12f); EMIT2(0x1d0); EMIT2(0x209);
                EMIT2(0x20b); EMIT2(0x268); EMIT2(0x1d96);
                EMIT2(0x1e2d); EMIT2(0x1e2f); EMIT2(0x1ec9);
                EMIT2(0x1ecb);
                return;
            }
            0x6a | 0x135 | 0x1f0 | 0x249 => {
                EMIT2(b'j' as c_int); EMIT2(0x135); EMIT2(0x1f0); EMIT2(0x249);
                return;
            }
            0x6b | 0x137 | 0x199 | 0x1e9 | 0x1d84 | 0x1e31 | 0x1e33 | 0x1e35 | 0x2c6a | 0xa741 => {
                EMIT2(b'k' as c_int); EMIT2(0x137); EMIT2(0x199); EMIT2(0x1e9);
                EMIT2(0x1d84); EMIT2(0x1e31); EMIT2(0x1e33);
                EMIT2(0x1e35); EMIT2(0x2c6a); EMIT2(0xa741);
                return;
            }
            0x6c | 0x13a | 0x13c | 0x13e | 0x140 | 0x142 | 0x19a | 0x1e37 | 0x1e39 | 0x1e3b
            | 0x1e3d | 0x2c61 => {
                EMIT2(b'l' as c_int); EMIT2(0x13a); EMIT2(0x13c);
                EMIT2(0x13e); EMIT2(0x140); EMIT2(0x142);
                EMIT2(0x19a); EMIT2(0x1e37); EMIT2(0x1e39);
                EMIT2(0x1e3b); EMIT2(0x1e3d); EMIT2(0x2c61);
                return;
            }
            0x6d | 0x1d6f | 0x1e3f | 0x1e41 | 0x1e43 => {
                EMIT2(b'm' as c_int); EMIT2(0x1d6f); EMIT2(0x1e3f);
                EMIT2(0x1e41); EMIT2(0x1e43);
                return;
            }
            0x6e | 0xf1 | 0x144 | 0x146 | 0x148 | 0x149 | 0x1f9 | 0x1d70 | 0x1d87 | 0x1e45
            | 0x1e47 | 0x1e49 | 0x1e4b | 0xa7a5 => {
                EMIT2(b'n' as c_int); EMIT2(0xf1);
                EMIT2(0x144); EMIT2(0x146); EMIT2(0x148);
                EMIT2(0x149); EMIT2(0x1f9); EMIT2(0x1d70);
                EMIT2(0x1d87); EMIT2(0x1e45); EMIT2(0x1e47);
                EMIT2(0x1e49); EMIT2(0x1e4b); EMIT2(0xa7a5);
                return;
            }
            0x6f | 0xf2 | 0xf3 | 0xf4 | 0xf5 | 0xf6 | 0xf8 | 0x14d | 0x14f | 0x151 | 0x1a1
            | 0x1d2 | 0x1eb | 0x1ed | 0x1ff | 0x20d | 0x20f | 0x22b | 0x22d | 0x22f | 0x231
            | 0x275 | 0x1e4d | 0x1e4f | 0x1e51 | 0x1e53 | 0x1ecd | 0x1ecf | 0x1ed1 | 0x1ed3
            | 0x1ed5 | 0x1ed7 | 0x1ed9 | 0x1edb | 0x1edd | 0x1edf | 0x1ee1 | 0x1ee3 => {
                EMIT2(b'o' as c_int); EMIT2(0xf2); EMIT2(0xf3);
                EMIT2(0xf4); EMIT2(0xf5);
                EMIT2(0xf6); EMIT2(0xf8);
                EMIT2(0x14d); EMIT2(0x14f); EMIT2(0x151);
                EMIT2(0x1a1); EMIT2(0x1d2); EMIT2(0x1eb);
                EMIT2(0x1ed); EMIT2(0x1ff); EMIT2(0x20d);
                EMIT2(0x20f); EMIT2(0x22b); EMIT2(0x22d);
                EMIT2(0x22f); EMIT2(0x231); EMIT2(0x275);
                EMIT2(0x1e4d); EMIT2(0x1e4f); EMIT2(0x1e51);
                EMIT2(0x1e53); EMIT2(0x1ecd); EMIT2(0x1ecf);
                EMIT2(0x1ed1); EMIT2(0x1ed3); EMIT2(0x1ed5);
                EMIT2(0x1ed7); EMIT2(0x1ed9); EMIT2(0x1edb);
                EMIT2(0x1edd); EMIT2(0x1edf); EMIT2(0x1ee1);
                EMIT2(0x1ee3);
                return;
            }
            0x70 | 0x1a5 | 0x1d71 | 0x1d7d | 0x1d88 | 0x1e55 | 0x1e57 => {
                EMIT2(b'p' as c_int); EMIT2(0x1a5); EMIT2(0x1d71); EMIT2(0x1d7d);
                EMIT2(0x1d88); EMIT2(0x1e55); EMIT2(0x1e57);
                return;
            }
            0x71 | 0x24b | 0x2a0 => {
                EMIT2(b'q' as c_int); EMIT2(0x24b); EMIT2(0x2a0);
                return;
            }
            0x72 | 0x155 | 0x157 | 0x159 | 0x211 | 0x213 | 0x24d | 0x27d | 0x1d72 | 0x1d73
            | 0x1d89 | 0x1e59 | 0x1e5b | 0x1e5d | 0x1e5f | 0xa7a7 => {
                EMIT2(b'r' as c_int); EMIT2(0x155); EMIT2(0x157); EMIT2(0x159);
                EMIT2(0x211); EMIT2(0x213); EMIT2(0x24d); EMIT2(0x27d);
                EMIT2(0x1d72); EMIT2(0x1d73); EMIT2(0x1d89); EMIT2(0x1e59);
                EMIT2(0x1e5b); EMIT2(0x1e5d); EMIT2(0x1e5f); EMIT2(0xa7a7);
                return;
            }
            0x73 | 0x15b | 0x15d | 0x15f | 0x161 | 0x219 | 0x23f | 0x1d74 | 0x1d8a | 0x1e61
            | 0x1e63 | 0x1e65 | 0x1e67 | 0x1e69 | 0xa7a9 => {
                EMIT2(b's' as c_int); EMIT2(0x15b); EMIT2(0x15d); EMIT2(0x15f);
                EMIT2(0x161); EMIT2(0x219); EMIT2(0x23f); EMIT2(0x1d74);
                EMIT2(0x1d8a); EMIT2(0x1e61); EMIT2(0x1e63); EMIT2(0x1e65);
                EMIT2(0x1e67); EMIT2(0x1e69); EMIT2(0xa7a9);
                return;
            }
            0x74 | 0x163 | 0x165 | 0x167 | 0x1ab | 0x1ad | 0x21b | 0x288 | 0x1d75 | 0x1e6b
            | 0x1e6d | 0x1e6f | 0x1e71 | 0x1e97 | 0x2c66 => {
                EMIT2(b't' as c_int); EMIT2(0x163); EMIT2(0x165); EMIT2(0x167);
                EMIT2(0x1ab); EMIT2(0x1ad); EMIT2(0x21b); EMIT2(0x288);
                EMIT2(0x1d75); EMIT2(0x1e6b); EMIT2(0x1e6d); EMIT2(0x1e6f);
                EMIT2(0x1e71); EMIT2(0x1e97); EMIT2(0x2c66);
                return;
            }
            0x75 | 0xf9 | 0xfa | 0xfb | 0xfc | 0x169 | 0x16b | 0x16d | 0x16f | 0x171 | 0x173
            | 0x1b0 | 0x1d4 | 0x1d6 | 0x1d8 | 0x1da | 0x1dc | 0x215 | 0x217 | 0x289 | 0x1d7e
            | 0x1d99 | 0x1e73 | 0x1e75 | 0x1e77 | 0x1e79 | 0x1e7b | 0x1ee5 | 0x1ee7 | 0x1ee9
            | 0x1eeb | 0x1eed | 0x1eef | 0x1ef1 => {
                EMIT2(b'u' as c_int); EMIT2(0xf9); EMIT2(0xfa);
                EMIT2(0xfb); EMIT2(0xfc);
                EMIT2(0x169); EMIT2(0x16b);
                EMIT2(0x16d); EMIT2(0x16f); EMIT2(0x171);
                EMIT2(0x173); EMIT2(0x1d6); EMIT2(0x1d8);
                EMIT2(0x215); EMIT2(0x217); EMIT2(0x1b0);
                EMIT2(0x1d4); EMIT2(0x1da); EMIT2(0x1dc);
                EMIT2(0x289); EMIT2(0x1e73); EMIT2(0x1d7e);
                EMIT2(0x1d99); EMIT2(0x1e75); EMIT2(0x1e77);
                EMIT2(0x1e79); EMIT2(0x1e7b); EMIT2(0x1ee5);
                EMIT2(0x1ee7); EMIT2(0x1ee9); EMIT2(0x1eeb);
                EMIT2(0x1eed); EMIT2(0x1eef); EMIT2(0x1ef1);
                return;
            }
            0x76 | 0x28b | 0x1d8c | 0x1e7d | 0x1e7f => {
                EMIT2(b'v' as c_int); EMIT2(0x28b); EMIT2(0x1d8c); EMIT2(0x1e7d);
                EMIT2(0x1e7f);
                return;
            }
            0x77 | 0x175 | 0x1e81 | 0x1e83 | 0x1e85 | 0x1e87 | 0x1e89 | 0x1e98 => {
                EMIT2(b'w' as c_int); EMIT2(0x175); EMIT2(0x1e81); EMIT2(0x1e83);
                EMIT2(0x1e85); EMIT2(0x1e87); EMIT2(0x1e89); EMIT2(0x1e98);
                return;
            }
            0x78 | 0x1e8b | 0x1e8d => {
                EMIT2(b'x' as c_int); EMIT2(0x1e8b); EMIT2(0x1e8d);
                return;
            }
            0x79 | 0xfd | 0xff | 0x177 | 0x1b4 | 0x233 | 0x24f | 0x1e8f | 0x1e99 | 0x1ef3
            | 0x1ef5 | 0x1ef7 | 0x1ef9 => {
                EMIT2(b'y' as c_int); EMIT2(0xfd); EMIT2(0xff);
                EMIT2(0x177); EMIT2(0x1b4); EMIT2(0x233); EMIT2(0x24f);
                EMIT2(0x1e8f); EMIT2(0x1e99); EMIT2(0x1ef3);
                EMIT2(0x1ef5); EMIT2(0x1ef7); EMIT2(0x1ef9);
                return;
            }
            0x7a | 0x17a | 0x17c | 0x17e | 0x1b6 | 0x1d76 | 0x1d8e | 0x1e91 | 0x1e93 | 0x1e95
            | 0x2c6c => {
                EMIT2(b'z' as c_int); EMIT2(0x17a); EMIT2(0x17c); EMIT2(0x17e);
                EMIT2(0x1b6); EMIT2(0x1d76); EMIT2(0x1d8e); EMIT2(0x1e91);
                EMIT2(0x1e93); EMIT2(0x1e95); EMIT2(0x2c6c);
                return;
            }
            _ => {}
        }
        EMIT2(c);
    }
}

unsafe fn nfa_regatom() -> c_int {
    unsafe {
        let mut old_regparse = regparse.cast::<u8>();
        let mut extra: c_int = 0;
        let mut startc: c_int = -1;
        let save_prev_at_start = prev_at_start;

        let mut c = getchr();
        let mut goto_collection = false;
        let mut goto_multibyte = false;

        if c == NUL as c_int {
            emsg(tr(e_nul_found.as_ptr()));
            rc_did_emsg = true;
            return FAIL;
        } else if c == Magic(b'^' as c_int) {
            EMIT(NFA_BOL);
            return OK;
        } else if c == Magic(b'$' as c_int) {
            EMIT(NFA_EOL);
            had_eol = 1;
            return OK;
        } else if c == Magic(b'<' as c_int) {
            EMIT(NFA_BOW);
            return OK;
        } else if c == Magic(b'>' as c_int) {
            EMIT(NFA_EOW);
            return OK;
        } else if c == Magic(b'_' as c_int) {
            c = no_Magic(getchr());
            if c == NUL as c_int {
                emsg(tr(e_nul_found.as_ptr()));
                rc_did_emsg = true;
                return FAIL;
            }
            if c == b'^' as c_int {
                EMIT(NFA_BOL);
                return OK;
            }
            if c == b'$' as c_int {
                EMIT(NFA_EOL);
                had_eol = 1;
                return OK;
            }
            extra = NFA_ADD_NL;
            if c == b'[' as c_int {
                goto_collection = true;
            } else {
                let p = vim_strchr(classchars.as_ptr().cast(), no_Magic(c)).cast::<u8>();
                if p.is_null() {
                    if extra == NFA_ADD_NL {
                        semsg!(tr(e_ill_char_class.as_ptr()), i64::from(c));
                        rc_did_emsg = true;
                        return FAIL;
                    }
                    siemsg!(c"INTERNAL: Unknown character class char: %d".as_ptr(), c);
                    return FAIL;
                }
                if c == Magic(b'.' as c_int) && utf_iscomposing_legacy(peekchr()) {
                    old_regparse = regparse.cast();
                    c = getchr();
                    goto_multibyte = true;
                } else {
                    let idx = p.offset_from(classchars.as_ptr()) as usize;
                    EMIT(NFA_CLASSCODES[idx]);
                    if extra == NFA_ADD_NL {
                        EMIT(NFA_NEWL);
                        EMIT(NFA_OR);
                        regflags |= RF_HASNL;
                    }
                    return OK;
                }
            }
        }

        if !goto_collection && !goto_multibyte {
            if matches!(
                c,
                x if x == Magic(b'.' as c_int)
                    || x == Magic(b'i' as c_int)
                    || x == Magic(b'I' as c_int)
                    || x == Magic(b'k' as c_int)
                    || x == Magic(b'K' as c_int)
                    || x == Magic(b'f' as c_int)
                    || x == Magic(b'F' as c_int)
                    || x == Magic(b'p' as c_int)
                    || x == Magic(b'P' as c_int)
                    || x == Magic(b's' as c_int)
                    || x == Magic(b'S' as c_int)
                    || x == Magic(b'd' as c_int)
                    || x == Magic(b'D' as c_int)
                    || x == Magic(b'x' as c_int)
                    || x == Magic(b'X' as c_int)
                    || x == Magic(b'o' as c_int)
                    || x == Magic(b'O' as c_int)
                    || x == Magic(b'w' as c_int)
                    || x == Magic(b'W' as c_int)
                    || x == Magic(b'h' as c_int)
                    || x == Magic(b'H' as c_int)
                    || x == Magic(b'a' as c_int)
                    || x == Magic(b'A' as c_int)
                    || x == Magic(b'l' as c_int)
                    || x == Magic(b'L' as c_int)
                    || x == Magic(b'u' as c_int)
                    || x == Magic(b'U' as c_int)
            ) {
                let p = vim_strchr(classchars.as_ptr().cast(), no_Magic(c)).cast::<u8>();
                if p.is_null() {
                    if extra == NFA_ADD_NL {
                        semsg!(tr(e_ill_char_class.as_ptr()), i64::from(c));
                        rc_did_emsg = true;
                        return FAIL;
                    }
                    siemsg!(c"INTERNAL: Unknown character class char: %d".as_ptr(), c);
                    return FAIL;
                }
                if c == Magic(b'.' as c_int) && utf_iscomposing_legacy(peekchr()) {
                    old_regparse = regparse.cast();
                    c = getchr();
                } else {
                    let idx = p.offset_from(classchars.as_ptr()) as usize;
                    EMIT(NFA_CLASSCODES[idx]);
                    if extra == NFA_ADD_NL {
                        EMIT(NFA_NEWL);
                        EMIT(NFA_OR);
                        regflags |= RF_HASNL;
                    }
                    return OK;
                }
            } else if c == Magic(b'n' as c_int) {
                if reg_string != 0 {
                    EMIT(NL);
                } else {
                    EMIT(NFA_NEWL);
                    regflags |= RF_HASNL;
                }
                return OK;
            } else if c == Magic(b'(' as c_int) {
                if nfa_reg(REG_PAREN) == FAIL {
                    return FAIL;
                }
                return OK;
            } else if c == Magic(b'|' as c_int)
                || c == Magic(b'&' as c_int)
                || c == Magic(b')' as c_int)
                || c == Magic(b'=' as c_int)
                || c == Magic(b'?' as c_int)
                || c == Magic(b'+' as c_int)
                || c == Magic(b'@' as c_int)
                || c == Magic(b'*' as c_int)
                || c == Magic(b'{' as c_int)
            {
                semsg!(tr(e_misplaced.as_ptr()), no_Magic(c));
                return FAIL;
            } else if c == Magic(b'~' as c_int) {
                if reg_prev_sub.is_null() {
                    emsg(tr((&raw const e_nopresub).cast()));
                    return FAIL;
                }
                let mut lp = reg_prev_sub.cast::<u8>();
                while *lp != NUL {
                    EMIT(utf_ptr2char(lp.cast()));
                    if lp != reg_prev_sub.cast::<u8>() {
                        EMIT(NFA_CONCAT);
                    }
                    lp = lp.add(utf_ptr2len(lp.cast()) as usize);
                }
                EMIT(NFA_NOPEN);
                return OK;
            } else if (Magic(b'1' as c_int)..=Magic(b'9' as c_int)).contains(&c) {
                let refnum = no_Magic(c) - b'1' as c_int;
                if !seen_endbrace(refnum + 1) {
                    return FAIL;
                }
                EMIT(NFA_BACKREF1 + refnum);
                rex.nfa_has_backref = 1;
                return OK;
            } else if c == Magic(b'z' as c_int) {
                c = no_Magic(getchr());
                match c as u8 {
                    b's' => {
                        EMIT(NFA_ZSTART);
                        if !re_mult_next(c"\\zs".as_ptr()) {
                            return FAIL;
                        }
                        return OK;
                    }
                    b'e' => {
                        EMIT(NFA_ZEND);
                        rex.nfa_has_zend = 1;
                        if !re_mult_next(c"\\ze".as_ptr()) {
                            return FAIL;
                        }
                        return OK;
                    }
                    b'1'..=b'9' => {
                        if (reg_do_extmatch & REX_USE) == 0 {
                            emsg(tr(e_z1_not_allowed.as_ptr()));
                            rc_did_emsg = true;
                            return FAIL;
                        }
                        EMIT(NFA_ZREF1 + (no_Magic(c) - b'1' as c_int));
                        re_has_z = REX_USE;
                        return OK;
                    }
                    b'(' => {
                        if reg_do_extmatch != REX_SET {
                            emsg(tr(e_z_not_allowed.as_ptr()));
                            rc_did_emsg = true;
                            return FAIL;
                        }
                        if nfa_reg(REG_ZPAREN) == FAIL {
                            return FAIL;
                        }
                        re_has_z = REX_SET;
                        return OK;
                    }
                    _ => {
                        semsg!(
                            tr(c"E867: (NFA) Unknown operator '\\z%c'".as_ptr()),
                            no_Magic(c),
                        );
                        return FAIL;
                    }
                }
            } else if c == Magic(b'%' as c_int) {
                c = no_Magic(getchr());
                match c as u8 {
                    b'(' => {
                        if nfa_reg(REG_NPAREN) == FAIL {
                            return FAIL;
                        }
                        EMIT(NFA_NOPEN);
                        return OK;
                    }
                    b'd' | b'o' | b'x' | b'u' | b'U' => {
                        let nr = match c as u8 {
                            b'd' => getdecchrs(),
                            b'o' => getoctchrs(),
                            b'x' => gethexchrs(2),
                            b'u' => gethexchrs(4),
                            b'U' => gethexchrs(8),
                            _ => -1,
                        };
                        if nr < 0 || nr > i64::from(c_int::MAX) {
                            semsg!(
                                tr(c"E678: Invalid character after %s%%[dxouU]".as_ptr()),
                                if reg_magic == MAGIC_ALL {
                                    c"".as_ptr()
                                } else {
                                    c"\\".as_ptr()
                                },
                            );
                            rc_did_emsg = true;
                            return FAIL;
                        }
                        EMIT(if nr == 0 { 0x0a } else { nr as c_int });
                        return OK;
                    }
                    b'^' => {
                        EMIT(NFA_BOF);
                        return OK;
                    }
                    b'$' => {
                        EMIT(NFA_EOF);
                        return OK;
                    }
                    b'#' => {
                        if *regparse == b'=' as c_char
                            && (*regparse.add(1) as u8) >= 48
                            && (*regparse.add(1) as u8) <= 50
                        {
                            semsg!(
                                tr(e_atom_engine_must_be_at_start_of_pattern.as_ptr()),
                                *regparse.add(1) as c_int,
                            );
                            return FAIL;
                        }
                        EMIT(NFA_CURSOR);
                        return OK;
                    }
                    b'V' => {
                        EMIT(NFA_VISUAL);
                        return OK;
                    }
                    b'C' => {
                        EMIT(NFA_ANY_COMPOSING);
                        return OK;
                    }
                    b'[' => {
                        let mut n: c_int = 0;
                        loop {
                            c = peekchr();
                            if c == b']' as c_int {
                                break;
                            }
                            if c == NUL as c_int {
                                semsg!(
                                    tr(e_missing_sb.as_ptr()),
                                    if reg_magic == MAGIC_ALL {
                                        c"".as_ptr()
                                    } else {
                                        c"\\".as_ptr()
                                    },
                                );
                                rc_did_emsg = true;
                                return FAIL;
                            }
                            if nfa_regatom() == FAIL {
                                return FAIL;
                            }
                            n += 1;
                        }
                        let _ = getchr();
                        if n == 0 {
                            semsg!(
                                tr(e_empty_sb.as_ptr()),
                                if reg_magic == MAGIC_ALL {
                                    c"".as_ptr()
                                } else {
                                    c"\\".as_ptr()
                                },
                            );
                            rc_did_emsg = true;
                            return FAIL;
                        }
                        EMIT(NFA_OPT_CHARS);
                        EMIT(n);
                        EMIT(NFA_NOPEN);
                        return OK;
                    }
                    _ => {
                        let mut n: i64 = 0;
                        let cmp = c;
                        let mut cur = false;
                        let mut got_digit = false;

                        if c == b'<' as c_int || c == b'>' as c_int {
                            c = getchr();
                        }
                        if no_Magic(c) == b'.' as c_int {
                            cur = true;
                            c = getchr();
                        }
                        while ascii_isdigit(c) {
                            if cur {
                                semsg!(
                                    tr(e_regexp_number_after_dot_pos_search_chr.as_ptr()),
                                    no_Magic(c),
                                );
                                return FAIL;
                            }
                            if n > i64::from(i32::MAX - (c - b'0' as c_int)) / 10 {
                                emsg(tr(e_value_too_large.as_ptr()));
                                return FAIL;
                            }
                            n = n * 10 + i64::from(c - b'0' as c_int);
                            c = getchr();
                            got_digit = true;
                        }
                        if c == b'l' as c_int || c == b'c' as c_int || c == b'v' as c_int {
                            let mut limit = i32::MAX;
                            if !cur && !got_digit {
                                semsg!(tr(e_nfa_regexp_missing_value_in_chr.as_ptr()), no_Magic(c));
                                return FAIL;
                            }
                            if c == b'l' as c_int {
                                if cur {
                                    n = i64::from((*curwin).w_cursor.lnum);
                                }
                                EMIT(if cmp == b'<' as c_int {
                                    NFA_LNUM_LT
                                } else if cmp == b'>' as c_int {
                                    NFA_LNUM_GT
                                } else {
                                    NFA_LNUM
                                });
                                if save_prev_at_start != 0 {
                                    at_start = 1;
                                }
                            } else if c == b'c' as c_int {
                                if cur {
                                    n = i64::from((*curwin).w_cursor.col) + 1;
                                }
                                EMIT(if cmp == b'<' as c_int {
                                    NFA_COL_LT
                                } else if cmp == b'>' as c_int {
                                    NFA_COL_GT
                                } else {
                                    NFA_COL
                                });
                            } else {
                                if cur {
                                    let mut vcol: colnr_T = 0;
                                    getvvcol(
                                        curwin,
                                        &raw mut (*curwin).w_cursor,
                                        ptr::null_mut(),
                                        ptr::null_mut(),
                                        &raw mut vcol,
                                        0,
                                    );
                                    vcol += 1;
                                    n = i64::from(vcol);
                                }
                                EMIT(if cmp == b'<' as c_int {
                                    NFA_VCOL_LT
                                } else if cmp == b'>' as c_int {
                                    NFA_VCOL_GT
                                } else {
                                    NFA_VCOL
                                });
                                limit = i32::MAX / MB_MAXBYTES;
                            }
                            if n >= i64::from(limit) {
                                emsg(tr(e_value_too_large.as_ptr()));
                                return FAIL;
                            }
                            EMIT(n as c_int);
                            return OK;
                        } else if no_Magic(c) == b'\'' as c_int && n == 0 {
                            EMIT(if cmp == b'<' as c_int {
                                NFA_MARK_LT
                            } else if cmp == b'>' as c_int {
                                NFA_MARK_GT
                            } else {
                                NFA_MARK
                            });
                            EMIT(getchr());
                            return OK;
                        }
                        semsg!(
                            tr(c"E867: (NFA) Unknown operator '\\%%%c'".as_ptr()),
                            no_Magic(c),
                        );
                        return FAIL;
                    }
                }
            } else if c == Magic(b'[' as c_int) {
                goto_collection = true;
            }
        }

        if goto_collection {
            let p = regparse.cast::<u8>();
            let endp = skip_anyof(p.cast()).cast::<u8>();
            if *endp == b']' {
                let result = nfa_recognize_char_class(
                    regparse.cast(),
                    endp,
                    c_int::from(extra == NFA_ADD_NL),
                );
                if result != FAIL {
                    if (NFA_FIRST_NL..=NFA_LAST_NL).contains(&result) {
                        EMIT(result - NFA_ADD_NL);
                        EMIT(NFA_NEWL);
                        EMIT(NFA_OR);
                    } else {
                        EMIT(result);
                    }
                    regparse = endp.cast();
                    mb_ptr_adv(&raw mut regparse);
                    return OK;
                }

                let mut negated = false;
                if *regparse == b'^' as c_char {
                    negated = true;
                    mb_ptr_adv(&raw mut regparse);
                    EMIT(NFA_START_NEG_COLL);
                } else {
                    EMIT(NFA_START_COLL);
                }
                if *regparse == b'-' as c_char {
                    startc = b'-' as c_int;
                    EMIT(startc);
                    EMIT(NFA_CONCAT);
                    mb_ptr_adv(&raw mut regparse);
                }

                let mut emit_range = false;
                while (regparse.cast::<u8>() as *const u8) < endp {
                    let oldstartc = startc;
                    let mut range_endpoint = false;
                    startc = -1;
                    let mut got_coll_char = false;

                    if *regparse == b'[' as c_char {
                        let mut equiclass: c_int = 0;
                        let mut collclass: c_int = 0;
                        let charclass = get_char_class(&raw mut regparse);
                        if charclass == CLASS_NONE {
                            equiclass = get_equi_class(&raw mut regparse);
                            if equiclass == 0 {
                                collclass = get_coll_element(&raw mut regparse);
                            }
                        }

                        if charclass != CLASS_NONE {
                            match charclass {
                                CLASS_ALNUM => EMIT(NFA_CLASS_ALNUM),
                                CLASS_ALPHA => EMIT(NFA_CLASS_ALPHA),
                                CLASS_BLANK => EMIT(NFA_CLASS_BLANK),
                                CLASS_CNTRL => EMIT(NFA_CLASS_CNTRL),
                                CLASS_DIGIT => EMIT(NFA_CLASS_DIGIT),
                                CLASS_GRAPH => EMIT(NFA_CLASS_GRAPH),
                                CLASS_LOWER => {
                                    wants_nfa = true;
                                    EMIT(NFA_CLASS_LOWER);
                                }
                                CLASS_PRINT => EMIT(NFA_CLASS_PRINT),
                                CLASS_PUNCT => EMIT(NFA_CLASS_PUNCT),
                                CLASS_SPACE => EMIT(NFA_CLASS_SPACE),
                                CLASS_UPPER => {
                                    wants_nfa = true;
                                    EMIT(NFA_CLASS_UPPER);
                                }
                                CLASS_XDIGIT => EMIT(NFA_CLASS_XDIGIT),
                                CLASS_TAB => EMIT(NFA_CLASS_TAB),
                                CLASS_RETURN => EMIT(NFA_CLASS_RETURN),
                                CLASS_BACKSPACE => EMIT(NFA_CLASS_BACKSPACE),
                                CLASS_ESCAPE => EMIT(NFA_CLASS_ESCAPE),
                                CLASS_IDENT => EMIT(NFA_CLASS_IDENT),
                                CLASS_KEYWORD => EMIT(NFA_CLASS_KEYWORD),
                                CLASS_FNAME => EMIT(NFA_CLASS_FNAME),
                                _ => {}
                            }
                            EMIT(NFA_CONCAT);
                            continue;
                        }
                        if equiclass != 0 {
                            nfa_emit_equi_class(equiclass);
                            continue;
                        }
                        if collclass != 0 {
                            startc = collclass;
                        }
                    }

                    if *regparse == b'-' as c_char && oldstartc != -1 {
                        emit_range = true;
                        startc = oldstartc;
                        mb_ptr_adv(&raw mut regparse);
                        continue;
                    }

                    if *regparse == b'\\' as c_char
                        && (regparse.add(1).cast::<u8>() as *const u8) <= endp
                        && (!vim_strchr(REGEXP_INRANGE.as_ptr(), *regparse.add(1) as u8 as c_int)
                            .is_null()
                            || (reg_cpo_lit == 0
                                && !vim_strchr(
                                    REGEXP_ABBR.as_ptr(),
                                    *regparse.add(1) as u8 as c_int,
                                )
                                .is_null()))
                    {
                        mb_ptr_adv(&raw mut regparse);
                        if *regparse == b'n' as c_char {
                            startc = if reg_string != 0
                                || emit_range
                                || *regparse.add(1) == b'-' as c_char
                            {
                                NL
                            } else {
                                NFA_NEWL
                            };
                        } else if matches!(*regparse as u8, b'd' | b'o' | b'x' | b'u' | b'U') {
                            startc = coll_get_char();
                            if startc == c_int::MAX {
                                emsg(tr(e_unicode_val_too_large.as_ptr()));
                                rc_did_emsg = true;
                                return FAIL;
                            }
                            got_coll_char = true;
                            mb_ptr_back(old_regparse, &raw mut regparse);
                        } else {
                            startc = backslash_trans(*regparse as u8 as c_int);
                        }
                    }

                    if startc == -1 {
                        startc = utf_ptr2char(regparse);
                    }

                    if emit_range {
                        let endc = startc;
                        range_endpoint = true;
                        startc = oldstartc;
                        if startc > endc {
                            emsg(tr(e_reverse_range.as_ptr()));
                            rc_did_emsg = true;
                            return FAIL;
                        }
                        if endc > startc + 2 {
                            if startc == 0 {
                                EMIT(1);
                            } else {
                                post_ptr = post_ptr.sub(1);
                            }
                            EMIT(endc);
                            EMIT(NFA_RANGE);
                            EMIT(NFA_CONCAT);
                        } else {
                            for ch in (startc + 1)..=endc {
                                EMIT(ch);
                                EMIT(NFA_CONCAT);
                            }
                        }
                        emit_range = false;
                        startc = -1;
                    } else if startc == NFA_NEWL {
                        if !negated {
                            extra = NFA_ADD_NL;
                        }
                    } else if got_coll_char && startc == 0 {
                        EMIT(0x0a);
                        EMIT(NFA_CONCAT);
                    } else {
                        EMIT(startc);
                        if utf_ptr2len(regparse) == utfc_ptr2len(regparse) {
                            EMIT(NFA_CONCAT);
                        }
                    }

                    let plen = utfc_ptr2len(regparse);
                    if !range_endpoint && utf_ptr2len(regparse) != plen {
                        let mut i = utf_ptr2len(regparse);
                        c = utf_ptr2char(regparse.add(i as usize));
                        loop {
                            if c == 0 {
                                EMIT(1);
                            } else {
                                EMIT(c);
                            }
                            EMIT(NFA_CONCAT);
                            i += utf_char2len(c);
                            if i >= plen {
                                break;
                            }
                            c = utf_ptr2char(regparse.add(i as usize));
                        }
                        EMIT(NFA_COMPOSING);
                        EMIT(NFA_CONCAT);
                    }
                    mb_ptr_adv(&raw mut regparse);
                }

                mb_ptr_back(old_regparse, &raw mut regparse);
                if *regparse == b'-' as c_char {
                    EMIT(b'-' as c_int);
                    EMIT(NFA_CONCAT);
                }

                regparse = endp.cast();
                mb_ptr_adv(&raw mut regparse);

                if negated {
                    EMIT(NFA_END_NEG_COLL);
                } else {
                    EMIT(NFA_END_COLL);
                }

                if extra == NFA_ADD_NL {
                    EMIT(if reg_string != 0 { NL } else { NFA_NEWL });
                    EMIT(NFA_OR);
                }

                return OK;
            }

            if reg_strict != 0 {
                emsg(tr(e_missingbracket.as_ptr()));
                rc_did_emsg = true;
                return FAIL;
            }
        }

        let plen = utfc_ptr2len(old_regparse.cast());
        if utf_char2len(c) != plen || utf_iscomposing_legacy(c) {
            let mut i: c_int = 0;
            loop {
                EMIT(c);
                if i > 0 {
                    EMIT(NFA_CONCAT);
                }
                i += utf_char2len(c);
                if i >= plen {
                    break;
                }
                c = utf_ptr2char(old_regparse.add(i as usize).cast());
            }
            EMIT(NFA_COMPOSING);
            regparse = old_regparse.add(plen as usize).cast();
        } else {
            c = no_Magic(c);
            EMIT(c);
        }
        OK
    }
}

unsafe fn nfa_regpiece() -> c_int {
    unsafe {
        let mut minval: c_int = 0;
        let mut maxval: c_int = 0;
        let mut old_state: parse_state_T = zeroed();
        let mut new_state: parse_state_T = zeroed();

        save_parse_state(&raw mut old_state);
        let my_post_start = post_ptr.offset_from(post_start) as c_int;

        let ret = nfa_regatom();
        if ret == FAIL {
            return FAIL;
        }
        let op = peekchr();
        if re_multi_type(op) == NOT_MULTI {
            return OK;
        }

        skipchr();
        if op == Magic(b'*' as c_int) {
            EMIT(NFA_STAR);
        } else if op == Magic(b'+' as c_int) {
            restore_parse_state(&raw mut old_state);
            curchr = -1;
            if nfa_regatom() == FAIL {
                return FAIL;
            }
            EMIT(NFA_STAR);
            EMIT(NFA_CONCAT);
            skipchr();
        } else if op == Magic(b'@' as c_int) {
            let c2 = getdecchrs();
            let mut op2 = no_Magic(getchr());
            let mut i: c_int = 0;
            match op2 as u8 {
                b'=' => i = NFA_PREV_ATOM_NO_WIDTH,
                b'!' => i = NFA_PREV_ATOM_NO_WIDTH_NEG,
                b'<' => {
                    op2 = no_Magic(getchr());
                    if op2 == b'=' as c_int {
                        i = NFA_PREV_ATOM_JUST_BEFORE;
                    } else if op2 == b'!' as c_int {
                        i = NFA_PREV_ATOM_JUST_BEFORE_NEG;
                    }
                }
                b'>' => i = NFA_PREV_ATOM_LIKE_PATTERN,
                _ => {}
            }
            if i == 0 {
                semsg!(tr(c"E869: (NFA) Unknown operator '\\@%c'".as_ptr()), op2);
                return FAIL;
            }
            EMIT(i);
            if i == NFA_PREV_ATOM_JUST_BEFORE || i == NFA_PREV_ATOM_JUST_BEFORE_NEG {
                EMIT(c2 as c_int);
            }
        } else if op == Magic(b'?' as c_int) || op == Magic(b'=' as c_int) {
            EMIT(NFA_QUEST);
        } else if op == Magic(b'{' as c_int) {
            let mut greedy = true;
            let c2 = peekchr();
            if c2 == b'-' as c_int || c2 == Magic(b'-' as c_int) {
                skipchr();
                greedy = false;
            }
            if read_limits(&raw mut minval, &raw mut maxval) == 0 {
                emsg(tr(
                    c"E870: (NFA regexp) Error reading repetition limits".as_ptr()
                ));
                rc_did_emsg = true;
                return FAIL;
            }
            if minval == 0 && maxval == MAX_LIMIT {
                if greedy {
                    EMIT(NFA_STAR);
                } else {
                    EMIT(NFA_STAR_NONGREEDY);
                }
            } else if maxval == 0 {
                post_ptr = post_start.add(my_post_start as usize);
                EMIT(NFA_EMPTY);
                return OK;
            } else {
                if (nfa_re_flags & RE_AUTO) != 0
                    && (maxval > 500 || maxval > minval + 200)
                    && (maxval != MAX_LIMIT && minval < 200)
                    && !wants_nfa
                {
                    return FAIL;
                }
                post_ptr = post_start.add(my_post_start as usize);
                save_parse_state(&raw mut new_state);

                let quest = if greedy {
                    NFA_QUEST
                } else {
                    NFA_QUEST_NONGREEDY
                };
                for i in 0..maxval {
                    restore_parse_state(&raw mut old_state);
                    let old_post_pos = post_ptr.offset_from(post_start) as c_int;
                    if nfa_regatom() == FAIL {
                        return FAIL;
                    }
                    if i + 1 > minval {
                        if maxval == MAX_LIMIT {
                            if greedy {
                                EMIT(NFA_STAR);
                            } else {
                                EMIT(NFA_STAR_NONGREEDY);
                            }
                        } else {
                            EMIT(quest);
                        }
                    }
                    if old_post_pos != my_post_start {
                        EMIT(NFA_CONCAT);
                    }
                    if i + 1 > minval && maxval == MAX_LIMIT {
                        break;
                    }
                }
                restore_parse_state(&raw mut new_state);
                curchr = -1;
            }
        }

        if re_multi_type(peekchr()) != NOT_MULTI {
            emsg(tr(
                c"E871: (NFA regexp) Can't have a multi follow a multi".as_ptr()
            ));
            rc_did_emsg = true;
            return FAIL;
        }

        OK
    }
}

unsafe fn nfa_regconcat() -> c_int {
    unsafe {
        let mut cont = true;
        let mut first = true;

        while cont {
            let p = peekchr();
            if p == NUL as c_int
                || p == Magic(b'|' as c_int)
                || p == Magic(b'&' as c_int)
                || p == Magic(b')' as c_int)
            {
                cont = false;
            } else if p == Magic(b'Z' as c_int) {
                regflags |= RF_ICOMBINE;
                skipchr_keepstart();
            } else if p == Magic(b'c' as c_int) {
                regflags |= RF_ICASE;
                skipchr_keepstart();
            } else if p == Magic(b'C' as c_int) {
                regflags |= RF_NOICASE;
                skipchr_keepstart();
            } else if p == Magic(b'v' as c_int) {
                reg_magic = MAGIC_ALL;
                skipchr_keepstart();
                curchr = -1;
            } else if p == Magic(b'm' as c_int) {
                reg_magic = MAGIC_ON;
                skipchr_keepstart();
                curchr = -1;
            } else if p == Magic(b'M' as c_int) {
                reg_magic = MAGIC_OFF;
                skipchr_keepstart();
                curchr = -1;
            } else if p == Magic(b'V' as c_int) {
                reg_magic = MAGIC_NONE;
                skipchr_keepstart();
                curchr = -1;
            } else {
                if nfa_regpiece() == FAIL {
                    return FAIL;
                }
                if !first {
                    EMIT(NFA_CONCAT);
                } else {
                    first = false;
                }
            }
        }

        OK
    }
}

unsafe fn nfa_regbranch() -> c_int {
    unsafe {
        let mut old_post_pos = post_ptr.offset_from(post_start) as c_int;
        if nfa_regconcat() == FAIL {
            return FAIL;
        }
        while peekchr() == Magic(b'&' as c_int) {
            skipchr();
            if old_post_pos == post_ptr.offset_from(post_start) as c_int {
                EMIT(NFA_EMPTY);
            }
            EMIT(NFA_NOPEN);
            EMIT(NFA_PREV_ATOM_NO_WIDTH);
            old_post_pos = post_ptr.offset_from(post_start) as c_int;
            if nfa_regconcat() == FAIL {
                return FAIL;
            }
            if old_post_pos == post_ptr.offset_from(post_start) as c_int {
                EMIT(NFA_EMPTY);
            }
            EMIT(NFA_CONCAT);
        }
        if old_post_pos == post_ptr.offset_from(post_start) as c_int {
            EMIT(NFA_EMPTY);
        }
        OK
    }
}

unsafe fn nfa_reg(paren: c_int) -> c_int {
    unsafe {
        let mut parno: c_int = 0;

        if paren == REG_PAREN {
            if regnpar >= NSUBEXP as c_int {
                emsg(tr(c"E872: (NFA regexp) Too many '('".as_ptr()));
                rc_did_emsg = true;
                return FAIL;
            }
            parno = regnpar;
            regnpar += 1;
        } else if paren == REG_ZPAREN {
            if regnzpar >= NSUBEXP as c_int {
                emsg(tr(c"E879: (NFA regexp) Too many \\z(".as_ptr()));
                rc_did_emsg = true;
                return FAIL;
            }
            parno = regnzpar;
            regnzpar += 1;
        }

        if nfa_reg_parse_depth >= REG_MAX_PAREN_DEPTH {
            emsg(tr((&raw const e_command_too_complex).cast()));
            rc_did_emsg = true;
            return FAIL;
        }
        nfa_reg_parse_depth += 1;

        if nfa_regbranch() == FAIL {
            nfa_reg_parse_depth -= 1;
            return FAIL;
        }
        while peekchr() == Magic(b'|' as c_int) {
            skipchr();
            if nfa_regbranch() == FAIL {
                nfa_reg_parse_depth -= 1;
                return FAIL;
            }
            EMIT(NFA_OR);
        }

        if paren != REG_NOPAREN && getchr() != Magic(b')' as c_int) {
            if paren == REG_NPAREN {
                semsg!(
                    tr(e_unmatched_str_percent_open.as_ptr()),
                    if reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
            } else {
                semsg!(
                    tr(e_unmatched_str_open.as_ptr()),
                    if reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
            }
            rc_did_emsg = true;
            nfa_reg_parse_depth -= 1;
            return FAIL;
        } else if paren == REG_NOPAREN && peekchr() != NUL as c_int {
            if peekchr() == Magic(b')' as c_int) {
                semsg!(
                    tr(e_unmatched_str_close.as_ptr()),
                    if reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
            } else {
                emsg(tr(c"E873: (NFA regexp) proper termination error".as_ptr()));
            }
            rc_did_emsg = true;
            nfa_reg_parse_depth -= 1;
            return FAIL;
        }

        if paren == REG_PAREN {
            *(&raw mut had_endbrace).cast::<u8>().add(parno as usize) = 1;
            EMIT(NFA_MOPEN + parno);
        } else if paren == REG_ZPAREN {
            EMIT(NFA_ZOPEN + parno);
        }

        nfa_reg_parse_depth -= 1;
        OK
    }
}

unsafe fn re2post() -> *mut c_int {
    unsafe {
        if nfa_reg(REG_NOPAREN) == FAIL {
            return ptr::null_mut();
        }
        EMIT(NFA_MOPEN);
        post_start
    }
}

unsafe fn alloc_state(c: c_int, out: *mut nfa_state_T, out1: *mut nfa_state_T) -> *mut nfa_state_T {
    unsafe {
        if istate >= nstate {
            return ptr::null_mut();
        }
        let s = state_ptr.add(istate as usize);
        istate += 1;
        (*s).c = c;
        (*s).out = out;
        (*s).out1 = out1;
        (*s).val = 0;
        (*s).id = istate;
        (*s).lastlist[0] = 0;
        (*s).lastlist[1] = 0;
        s
    }
}

#[inline]
fn frag(start: *mut nfa_state_T, out: *mut Ptrlist) -> Frag_T {
    Frag_T { start, out }
}

#[inline]
unsafe fn list1(outp: *mut *mut nfa_state_T) -> *mut Ptrlist {
    unsafe {
        let l = outp.cast::<Ptrlist>();
        (*l).next = ptr::null_mut();
        l
    }
}

unsafe fn patch(mut l: *mut Ptrlist, s: *mut nfa_state_T) {
    unsafe {
        while !l.is_null() {
            let next = (*l).next;
            (*l).s = s;
            l = next;
        }
    }
}

unsafe fn append(mut l1: *mut Ptrlist, l2: *mut Ptrlist) -> *mut Ptrlist {
    unsafe {
        let oldl1 = l1;
        while !(*l1).next.is_null() {
            l1 = (*l1).next;
        }
        (*l1).next = l2;
        oldl1
    }
}

unsafe fn st_error(_postfix: *mut c_int, _end: *mut c_int, _p: *mut c_int) {
    unsafe {
        emsg(tr(c"E874: (NFA) Could not pop the stack!".as_ptr()));
    }
}

unsafe fn st_push(s: Frag_T, p: *mut *mut Frag_T, stack_end: *mut Frag_T) {
    unsafe {
        let stackp = *p;
        if stackp >= stack_end {
            return;
        }
        *stackp = s;
        *p = stackp.add(1);
    }
}

unsafe fn st_pop(p: *mut *mut Frag_T, stack: *mut Frag_T) -> Frag_T {
    unsafe {
        *p = (*p).wrapping_sub(1);
        let stackp = *p;
        if stackp < stack {
            return empty_frag;
        }
        *stackp
    }
}

unsafe fn nfa_max_width(startstate: *mut nfa_state_T, depth: c_int) -> c_int {
    unsafe {
        let mut state = startstate;
        let mut len: c_int = 0;

        if depth > 4 {
            return -1;
        }

        while !state.is_null() {
            match (*state).c {
                NFA_END_INVISIBLE | NFA_END_INVISIBLE_NEG => return len,
                NFA_SPLIT => {
                    let l = nfa_max_width((*state).out, depth + 1);
                    let r = nfa_max_width((*state).out1, depth + 1);
                    if l < 0 || r < 0 {
                        return -1;
                    }
                    return len + l.max(r);
                }
                NFA_ANY | NFA_START_COLL | NFA_START_NEG_COLL => {
                    len += MB_MAXBYTES;
                    if (*state).c != NFA_ANY {
                        if (*state).out1.is_null() || (*(*state).out1).out.is_null() {
                            return -1;
                        }
                        state = (*(*state).out1).out;
                        continue;
                    }
                }
                NFA_DIGIT | NFA_WHITE | NFA_HEX | NFA_OCTAL => {
                    len += 1;
                }
                NFA_IDENT | NFA_SIDENT | NFA_KWORD | NFA_SKWORD | NFA_FNAME | NFA_SFNAME
                | NFA_PRINT | NFA_SPRINT | NFA_NWHITE | NFA_NDIGIT | NFA_NHEX | NFA_NOCTAL
                | NFA_WORD | NFA_NWORD | NFA_HEAD | NFA_NHEAD | NFA_ALPHA | NFA_NALPHA
                | NFA_LOWER | NFA_NLOWER | NFA_UPPER | NFA_NUPPER | NFA_LOWER_IC
                | NFA_NLOWER_IC | NFA_UPPER_IC | NFA_NUPPER_IC | NFA_ANY_COMPOSING => {
                    len += 3;
                }
                NFA_START_INVISIBLE
                | NFA_START_INVISIBLE_NEG
                | NFA_START_INVISIBLE_BEFORE
                | NFA_START_INVISIBLE_BEFORE_NEG => {
                    state = (*(*state).out1).out;
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
            state = (*state).out;
        }
        -1
    }
}

unsafe fn post2nfa(postfix: *mut c_int, end: *mut c_int, nfa_calc_size: bool) -> *mut nfa_state_T {
    unsafe {
        if postfix.is_null() {
            return ptr::null_mut();
        }

        let mut stack: *mut Frag_T = ptr::null_mut();
        let mut stackp: *mut Frag_T = ptr::null_mut();
        let mut stack_end: *mut Frag_T = ptr::null_mut();

        if !nfa_calc_size {
            if nfa_stack.is_null() || nfa_stack_len < nstate + 1 {
                xfree(nfa_stack.cast());
                nfa_stack = xmalloc((nstate as usize + 1) * size_of::<Frag_T>()).cast::<Frag_T>();
                nfa_stack_len = nstate + 1;
            }
            stack = nfa_stack;
            stackp = stack;
            stack_end = stack.add(nstate as usize + 1);
        }

        macro_rules! push {
            ($s:expr) => {
                st_push($s, &raw mut stackp, stack_end)
            };
        }
        macro_rules! pop {
            ($p:ident) => {{
                let val = st_pop(&raw mut stackp, stack);
                if stackp < stack {
                    st_error(postfix, end, $p);
                    return ptr::null_mut();
                }
                val
            }};
        }

        let mut p = postfix;
        while p < end {
            match *p {
                NFA_CONCAT => {
                    if !nfa_calc_size {
                        let e2 = pop!(p);
                        let e1 = pop!(p);
                        patch(e1.out, e2.start);
                        push!(frag(e1.start, e2.out));
                    }
                }
                NFA_OR => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let e2 = pop!(p);
                        let e1 = pop!(p);
                        let s = alloc_state(NFA_SPLIT, e1.start, e2.start);
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        push!(frag(s, append(e1.out, e2.out)));
                    }
                }
                NFA_STAR => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let e = pop!(p);
                        let s = alloc_state(NFA_SPLIT, e.start, ptr::null_mut());
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        patch(e.out, s);
                        push!(frag(s, list1(&raw mut (*s).out1)));
                    }
                }
                NFA_STAR_NONGREEDY => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let e = pop!(p);
                        let s = alloc_state(NFA_SPLIT, ptr::null_mut(), e.start);
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        patch(e.out, s);
                        push!(frag(s, list1(&raw mut (*s).out)));
                    }
                }
                NFA_QUEST => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let e = pop!(p);
                        let s = alloc_state(NFA_SPLIT, e.start, ptr::null_mut());
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        push!(frag(s, append(e.out, list1(&raw mut (*s).out1))));
                    }
                }
                NFA_QUEST_NONGREEDY => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let e = pop!(p);
                        let s = alloc_state(NFA_SPLIT, ptr::null_mut(), e.start);
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        push!(frag(s, append(e.out, list1(&raw mut (*s).out))));
                    }
                }
                NFA_END_COLL | NFA_END_NEG_COLL => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let e = pop!(p);
                        let s = alloc_state(NFA_END_COLL, ptr::null_mut(), ptr::null_mut());
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        patch(e.out, s);
                        (*e.start).out1 = s;
                        push!(frag(e.start, list1(&raw mut (*s).out)));
                    }
                }
                NFA_RANGE => {
                    if !nfa_calc_size {
                        let e2 = pop!(p);
                        let e1 = pop!(p);
                        (*e2.start).val = (*e2.start).c;
                        (*e2.start).c = NFA_RANGE_MAX;
                        (*e1.start).val = (*e1.start).c;
                        (*e1.start).c = NFA_RANGE_MIN;
                        patch(e1.out, e2.start);
                        push!(frag(e1.start, e2.out));
                    }
                }
                NFA_EMPTY => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let s = alloc_state(NFA_EMPTY, ptr::null_mut(), ptr::null_mut());
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        push!(frag(s, list1(&raw mut (*s).out)));
                    }
                }
                NFA_OPT_CHARS => {
                    p = p.add(1);
                    let mut n = *p;
                    if nfa_calc_size {
                        nstate += n;
                    } else {
                        let mut s: *mut nfa_state_T = ptr::null_mut();
                        let mut e1 = Frag_T {
                            start: ptr::null_mut(),
                            out: ptr::null_mut(),
                        };
                        let mut s1: *mut nfa_state_T = ptr::null_mut();
                        while n > 0 {
                            n -= 1;
                            let e = pop!(p);
                            s = alloc_state(NFA_SPLIT, e.start, ptr::null_mut());
                            if s.is_null() {
                                return ptr::null_mut();
                            }
                            if e1.out.is_null() {
                                e1 = e;
                            }
                            patch(e.out, s1);
                            append(e1.out, list1(&raw mut (*s).out1));
                            s1 = s;
                        }
                        push!(frag(s, e1.out));
                    }
                }
                NFA_PREV_ATOM_NO_WIDTH
                | NFA_PREV_ATOM_NO_WIDTH_NEG
                | NFA_PREV_ATOM_JUST_BEFORE
                | NFA_PREV_ATOM_JUST_BEFORE_NEG
                | NFA_PREV_ATOM_LIKE_PATTERN => {
                    let op = *p;
                    let before =
                        op == NFA_PREV_ATOM_JUST_BEFORE || op == NFA_PREV_ATOM_JUST_BEFORE_NEG;
                    let pattern = op == NFA_PREV_ATOM_LIKE_PATTERN;
                    let (start_state, end_state) = match op {
                        NFA_PREV_ATOM_NO_WIDTH => (NFA_START_INVISIBLE, NFA_END_INVISIBLE),
                        NFA_PREV_ATOM_NO_WIDTH_NEG => {
                            (NFA_START_INVISIBLE_NEG, NFA_END_INVISIBLE_NEG)
                        }
                        NFA_PREV_ATOM_JUST_BEFORE => {
                            (NFA_START_INVISIBLE_BEFORE, NFA_END_INVISIBLE)
                        }
                        NFA_PREV_ATOM_JUST_BEFORE_NEG => {
                            (NFA_START_INVISIBLE_BEFORE_NEG, NFA_END_INVISIBLE_NEG)
                        }
                        _ => (NFA_START_PATTERN, NFA_END_PATTERN),
                    };
                    let mut n: c_int = 0;
                    if before {
                        p = p.add(1);
                        n = *p;
                    }
                    if nfa_calc_size {
                        nstate += if pattern { 4 } else { 2 };
                    } else {
                        let e = pop!(p);
                        let s1 = alloc_state(end_state, ptr::null_mut(), ptr::null_mut());
                        if s1.is_null() {
                            return ptr::null_mut();
                        }
                        let s = alloc_state(start_state, e.start, s1);
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        if pattern {
                            let skip = alloc_state(NFA_SKIP, ptr::null_mut(), ptr::null_mut());
                            if skip.is_null() {
                                return ptr::null_mut();
                            }
                            let zend = alloc_state(NFA_ZEND, s1, ptr::null_mut());
                            if zend.is_null() {
                                return ptr::null_mut();
                            }
                            (*s1).out = skip;
                            patch(e.out, zend);
                            push!(frag(s, list1(&raw mut (*skip).out)));
                        } else {
                            patch(e.out, s1);
                            push!(frag(s, list1(&raw mut (*s1).out)));
                            if before {
                                if n <= 0 {
                                    n = nfa_max_width(e.start, 0);
                                }
                                (*s).val = n;
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
                        nstate += 2;
                    } else {
                        let mopen = op;
                        let mclose = match op {
                            NFA_NOPEN => NFA_NCLOSE,
                            NFA_COMPOSING => NFA_END_COMPOSING,
                            c if (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) => {
                                NFA_ZCLOSE + (c - NFA_ZOPEN)
                            }
                            c => c + NSUBEXP as c_int,
                        };
                        if stackp == stack {
                            let s = alloc_state(mopen, ptr::null_mut(), ptr::null_mut());
                            if s.is_null() {
                                return ptr::null_mut();
                            }
                            let s1 = alloc_state(mclose, ptr::null_mut(), ptr::null_mut());
                            if s1.is_null() {
                                return ptr::null_mut();
                            }
                            patch(list1(&raw mut (*s).out), s1);
                            push!(frag(s, list1(&raw mut (*s1).out)));
                        } else {
                            let e = pop!(p);
                            let s = alloc_state(mopen, e.start, ptr::null_mut());
                            if s.is_null() {
                                return ptr::null_mut();
                            }
                            let s1 = alloc_state(mclose, ptr::null_mut(), ptr::null_mut());
                            if s1.is_null() {
                                return ptr::null_mut();
                            }
                            patch(e.out, s1);
                            if mopen == NFA_COMPOSING {
                                patch(list1(&raw mut (*s).out1), s1);
                            }
                            push!(frag(s, list1(&raw mut (*s1).out)));
                        }
                    }
                }
                op if (NFA_BACKREF1..=NFA_ZREF9).contains(&op) => {
                    if nfa_calc_size {
                        nstate += 2;
                    } else {
                        let s = alloc_state(op, ptr::null_mut(), ptr::null_mut());
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        let s1 = alloc_state(NFA_SKIP, ptr::null_mut(), ptr::null_mut());
                        if s1.is_null() {
                            return ptr::null_mut();
                        }
                        patch(list1(&raw mut (*s).out), s1);
                        push!(frag(s, list1(&raw mut (*s1).out)));
                    }
                }
                NFA_LNUM | NFA_LNUM_GT | NFA_LNUM_LT | NFA_VCOL | NFA_VCOL_GT | NFA_VCOL_LT
                | NFA_COL | NFA_COL_GT | NFA_COL_LT | NFA_MARK | NFA_MARK_GT | NFA_MARK_LT => {
                    let op = *p;
                    p = p.add(1);
                    let n = *p;
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let s = alloc_state(op, ptr::null_mut(), ptr::null_mut());
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        (*s).val = n;
                        push!(frag(s, list1(&raw mut (*s).out)));
                    }
                }
                op => {
                    if nfa_calc_size {
                        nstate += 1;
                    } else {
                        let s = alloc_state(op, ptr::null_mut(), ptr::null_mut());
                        if s.is_null() {
                            return ptr::null_mut();
                        }
                        push!(frag(s, list1(&raw mut (*s).out)));
                    }
                }
            }
            p = p.add(1);
        }

        if nfa_calc_size {
            nstate += 1;
            return ptr::null_mut();
        }

        let e = pop!(p);
        if stackp != stack {
            emsg(tr(
                c"E875: (NFA regexp) (While converting from postfix to NFA),too many states left on stack"
                    .as_ptr(),
            ));
            rc_did_emsg = true;
            return ptr::null_mut();
        }

        if istate >= nstate {
            emsg(tr(
                c"E876: (NFA regexp) Not enough space to store the whole NFA ".as_ptr(),
            ));
            rc_did_emsg = true;
            return ptr::null_mut();
        }

        let matchstate = state_ptr.add(istate as usize);
        istate += 1;
        (*matchstate).c = NFA_MATCH;
        (*matchstate).out = ptr::null_mut();
        (*matchstate).out1 = ptr::null_mut();
        (*matchstate).id = 0;

        patch(e.out, matchstate);
        e.start
    }
}

unsafe fn nfa_postprocess(prog: *mut nfa_regprog_T) {
    unsafe {
        let states = (&raw mut (*prog).state).cast::<nfa_state_T>();
        for i in 0..(*prog).nstate as usize {
            let st = states.add(i);
            let c = (*st).c;
            if c == NFA_START_INVISIBLE
                || c == NFA_START_INVISIBLE_NEG
                || c == NFA_START_INVISIBLE_BEFORE
                || c == NFA_START_INVISIBLE_BEFORE_NEG
            {
                let directly = if match_follows((*(*st).out1).out, 0) {
                    true
                } else {
                    let ch_invisible = failure_chance((*st).out, 0);
                    let ch_follows = failure_chance((*(*st).out1).out, 0);
                    if c == NFA_START_INVISIBLE_BEFORE || c == NFA_START_INVISIBLE_BEFORE_NEG {
                        if (*st).val <= 0 && ch_follows > 0 {
                            false
                        } else {
                            ch_follows * 10 < ch_invisible
                        }
                    } else {
                        ch_follows < ch_invisible
                    }
                };
                if directly {
                    (*st).c += 1;
                }
            }
        }
    }
}

unsafe fn copy_pim(to: *mut nfa_pim_T, from: *mut nfa_pim_T) {
    unsafe {
        (*to).result = (*from).result;
        (*to).state = (*from).state;
        copy_sub(&raw mut (*to).subs.norm, &raw mut (*from).subs.norm);
        if rex.nfa_has_zsubexpr != 0 {
            copy_sub(&raw mut (*to).subs.synt, &raw mut (*from).subs.synt);
        }
        (*to).end = (*from).end;
    }
}

unsafe fn clear_sub(sub: *mut regsub_T) {
    unsafe {
        if REG_MULTI() {
            ptr::write_bytes(
                (&raw mut (*sub).list.multi).cast::<u8>(),
                0xff,
                size_of::<multipos>() * rex.nfa_nsubexpr as usize,
            );
        } else {
            ptr::write_bytes(
                (&raw mut (*sub).list.line).cast::<u8>(),
                0,
                size_of::<linepos>() * rex.nfa_nsubexpr as usize,
            );
        }
        (*sub).in_use = 0;
    }
}

#[inline]
unsafe fn copy_sub(to: *mut regsub_T, from: *mut regsub_T) {
    unsafe {
        (*to).in_use = (*from).in_use;
        if (*from).in_use <= 0 {
            return;
        }
        if REG_MULTI() {
            if (*from).in_use == 1 {
                (*to).list.multi[0] = (*from).list.multi[0];
            } else {
                ptr::copy(
                    (&raw const (*from).list.multi).cast::<multipos>(),
                    (&raw mut (*to).list.multi).cast::<multipos>(),
                    (*from).in_use as usize,
                );
            }
            (*to).orig_start_col = (*from).orig_start_col;
        } else if (*from).in_use == 1 {
            (*to).list.line[0] = (*from).list.line[0];
        } else {
            ptr::copy(
                (&raw const (*from).list.line).cast::<linepos>(),
                (&raw mut (*to).list.line).cast::<linepos>(),
                (*from).in_use as usize,
            );
        }
    }
}

unsafe fn copy_sub_off(to: *mut regsub_T, from: *mut regsub_T) {
    unsafe {
        if (*to).in_use < (*from).in_use {
            (*to).in_use = (*from).in_use;
        }
        if (*from).in_use <= 1 {
            return;
        }
        if REG_MULTI() {
            ptr::copy(
                (&raw const (*from).list.multi).cast::<multipos>().add(1),
                (&raw mut (*to).list.multi).cast::<multipos>().add(1),
                (*from).in_use as usize - 1,
            );
        } else {
            ptr::copy(
                (&raw const (*from).list.line).cast::<linepos>().add(1),
                (&raw mut (*to).list.line).cast::<linepos>().add(1),
                (*from).in_use as usize - 1,
            );
        }
    }
}

unsafe fn copy_ze_off(to: *mut regsub_T, from: *mut regsub_T) {
    unsafe {
        if rex.nfa_has_zend == 0 {
            return;
        }
        if REG_MULTI() {
            if (*from).list.multi[0].end_lnum >= 0 {
                (*to).list.multi[0].end_lnum = (*from).list.multi[0].end_lnum;
                (*to).list.multi[0].end_col = (*from).list.multi[0].end_col;
            }
        } else if !(*from).list.line[0].end.is_null() {
            (*to).list.line[0].end = (*from).list.line[0].end;
        }
    }
}

unsafe fn sub_equal(sub1: *mut regsub_T, sub2: *mut regsub_T) -> bool {
    unsafe {
        let todo = (*sub1).in_use.max((*sub2).in_use) as usize;
        if REG_MULTI() {
            for i in 0..todo {
                let s1 = if (i as c_int) < (*sub1).in_use {
                    (*sub1).list.multi[i].start_lnum
                } else {
                    -1
                };
                let s2 = if (i as c_int) < (*sub2).in_use {
                    (*sub2).list.multi[i].start_lnum
                } else {
                    -1
                };
                if s1 != s2 {
                    return false;
                }
                if s1 != -1 && (*sub1).list.multi[i].start_col != (*sub2).list.multi[i].start_col {
                    return false;
                }
                if rex.nfa_has_backref != 0 {
                    let e1 = if (i as c_int) < (*sub1).in_use {
                        (*sub1).list.multi[i].end_lnum
                    } else {
                        -1
                    };
                    let e2 = if (i as c_int) < (*sub2).in_use {
                        (*sub2).list.multi[i].end_lnum
                    } else {
                        -1
                    };
                    if e1 != e2 {
                        return false;
                    }
                    if e1 != -1 && (*sub1).list.multi[i].end_col != (*sub2).list.multi[i].end_col {
                        return false;
                    }
                }
            }
        } else {
            for i in 0..todo {
                let sp1 = if (i as c_int) < (*sub1).in_use {
                    (*sub1).list.line[i].start
                } else {
                    ptr::null_mut()
                };
                let sp2 = if (i as c_int) < (*sub2).in_use {
                    (*sub2).list.line[i].start
                } else {
                    ptr::null_mut()
                };
                if sp1 != sp2 {
                    return false;
                }
                if rex.nfa_has_backref != 0 {
                    let ep1 = if (i as c_int) < (*sub1).in_use {
                        (*sub1).list.line[i].end
                    } else {
                        ptr::null_mut()
                    };
                    let ep2 = if (i as c_int) < (*sub2).in_use {
                        (*sub2).list.line[i].end
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
}

unsafe fn has_state_with_pos(
    l: *mut nfa_list_T,
    state: *mut nfa_state_T,
    subs: *mut regsubs_T,
    pim: *mut nfa_pim_T,
) -> bool {
    unsafe {
        for i in 0..(*l).n as usize {
            let thread = (*l).t.add(i);
            if (*(*thread).state).id == (*state).id
                && sub_equal(&raw mut (*thread).subs.norm, &raw mut (*subs).norm)
                && (rex.nfa_has_zsubexpr == 0
                    || sub_equal(&raw mut (*thread).subs.synt, &raw mut (*subs).synt))
                && pim_equal(&raw const (*thread).pim, pim)
            {
                return true;
            }
        }
        false
    }
}

unsafe fn pim_equal(one: *const nfa_pim_T, two: *const nfa_pim_T) -> bool {
    unsafe {
        let one_unused = one.is_null() || (*one).result == NFA_PIM_UNUSED;
        let two_unused = two.is_null() || (*two).result == NFA_PIM_UNUSED;
        if one_unused {
            return two_unused;
        }
        if two_unused {
            return false;
        }
        if (*(*one).state).id != (*(*two).state).id {
            return false;
        }
        if REG_MULTI() {
            (*one).end.pos.lnum == (*two).end.pos.lnum && (*one).end.pos.col == (*two).end.pos.col
        } else {
            (*one).end.ptr == (*two).end.ptr
        }
    }
}

unsafe fn match_follows(startstate: *const nfa_state_T, depth: c_int) -> bool {
    unsafe {
        let mut state = startstate;
        if depth > 10 {
            return false;
        }
        while !state.is_null() {
            match (*state).c {
                NFA_MATCH
                | NFA_MCLOSE
                | NFA_END_INVISIBLE
                | NFA_END_INVISIBLE_NEG
                | NFA_END_PATTERN => return true,
                NFA_SPLIT => {
                    return match_follows((*state).out, depth + 1)
                        || match_follows((*state).out1, depth + 1);
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
                    state = (*(*state).out1).out;
                    continue;
                }
                NFA_ANY | NFA_ANY_COMPOSING | NFA_IDENT | NFA_SIDENT | NFA_KWORD | NFA_SKWORD
                | NFA_FNAME | NFA_SFNAME | NFA_PRINT | NFA_SPRINT | NFA_WHITE | NFA_NWHITE
                | NFA_DIGIT | NFA_NDIGIT | NFA_HEX | NFA_NHEX | NFA_OCTAL | NFA_NOCTAL
                | NFA_WORD | NFA_NWORD | NFA_HEAD | NFA_NHEAD | NFA_ALPHA | NFA_NALPHA
                | NFA_LOWER | NFA_NLOWER | NFA_UPPER | NFA_NUPPER | NFA_LOWER_IC
                | NFA_NLOWER_IC | NFA_UPPER_IC | NFA_NUPPER_IC | NFA_START_COLL
                | NFA_START_NEG_COLL | NFA_NEWL => return false,
                c => {
                    if c > 0 {
                        return false;
                    }
                }
            }
            state = (*state).out;
        }
        false
    }
}

unsafe fn state_in_list(l: *mut nfa_list_T, state: *mut nfa_state_T, subs: *mut regsubs_T) -> bool {
    unsafe {
        if (*state).lastlist[nfa_ll_index] == (*l).id
            && (rex.nfa_has_backref == 0 || has_state_with_pos(l, state, subs, ptr::null_mut()))
        {
            return true;
        }
        false
    }
}

unsafe fn addstate(
    l: *mut nfa_list_T,
    state: *mut nfa_state_T,
    subs_arg: *mut regsubs_T,
    pim: *mut nfa_pim_T,
    off_arg: c_int,
    depth: c_int,
) -> *mut regsubs_T {
    unsafe {
        let mut off = off_arg;
        let mut add_here = false;
        let mut listindex: c_int = 0;
        let mut found = false;
        let mut subs = subs_arg;

        if depth >= 5000 || subs.is_null() {
            return ptr::null_mut();
        }

        if off_arg <= -ADDSTATE_HERE_OFFSET {
            add_here = true;
            off = 0;
            listindex = -(off_arg + ADDSTATE_HERE_OFFSET);
        }

        match (*state).c {
            NFA_NCLOSE | NFA_MOPEN | NFA_ZEND | NFA_SPLIT | NFA_EMPTY => {}
            c if (NFA_MCLOSE..=NFA_MCLOSE9).contains(&c)
                || (NFA_ZCLOSE1..=NFA_ZCLOSE9).contains(&c) => {}
            c => {
                if (c == NFA_BOL || c == NFA_BOF)
                    && rex.input > rex.line
                    && *rex.input != NUL
                    && (nfa_endp.is_null() || !REG_MULTI() || rex.lnum == (*nfa_endp).se_u.pos.lnum)
                {
                    return subs;
                }

                if (*state).lastlist[nfa_ll_index] == (*l).id && (*state).c != NFA_SKIP {
                    if rex.nfa_has_backref == 0
                        && pim.is_null()
                        && (*l).has_pim == 0
                        && (*state).c != NFA_MATCH
                    {
                        if add_here {
                            let mut k: c_int = 0;
                            while k < (*l).n && k < listindex {
                                if (*(*(*l).t.add(k as usize)).state).id == (*state).id {
                                    found = true;
                                    break;
                                }
                                k += 1;
                            }
                        }
                        if !add_here || found {
                            return subs;
                        }
                    }
                    if has_state_with_pos(l, state, subs, pim) {
                        return subs;
                    }
                }

                if (*l).n == (*l).len {
                    let newlen = (*l).len * 3 / 2 + 50;
                    let newsize = (newlen as usize) * size_of::<nfa_thread_T>();
                    if ((newsize >> 10) as i64) >= p_mmp {
                        emsg(tr(e_pattern_uses_more_memory_than_maxmempattern.as_ptr()));
                        return ptr::null_mut();
                    }
                    if subs != &raw mut temp_subs {
                        copy_sub(&raw mut temp_subs.norm, &raw mut (*subs).norm);
                        if rex.nfa_has_zsubexpr != 0 {
                            copy_sub(&raw mut temp_subs.synt, &raw mut (*subs).synt);
                        }
                        subs = &raw mut temp_subs;
                    }
                    (*l).t = xrealloc((*l).t.cast(), newsize).cast();
                    (*l).len = newlen;
                }

                (*state).lastlist[nfa_ll_index] = (*l).id;
                let thread = (*l).t.add((*l).n as usize);
                (*l).n += 1;
                (*thread).state = state;
                if pim.is_null() {
                    (*thread).pim.result = NFA_PIM_UNUSED;
                } else {
                    copy_pim(&raw mut (*thread).pim, pim);
                    (*l).has_pim = 1;
                }
                copy_sub(&raw mut (*thread).subs.norm, &raw mut (*subs).norm);
                if rex.nfa_has_zsubexpr != 0 {
                    copy_sub(&raw mut (*thread).subs.synt, &raw mut (*subs).synt);
                }
            }
        }

        match (*state).c {
            NFA_MATCH => {}
            NFA_SPLIT => {
                subs = addstate(l, (*state).out, subs, pim, off_arg, depth + 1);
                subs = addstate(l, (*state).out1, subs, pim, off_arg, depth + 1);
            }
            NFA_EMPTY | NFA_NOPEN | NFA_NCLOSE => {
                subs = addstate(l, (*state).out, subs, pim, off_arg, depth + 1);
            }
            c if (NFA_MOPEN..=NFA_MOPEN9).contains(&c)
                || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c)
                || c == NFA_ZSTART =>
            {
                let (subidx, mut sub) = if c == NFA_ZSTART {
                    (0usize, &raw mut (*subs).norm)
                } else if (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c) {
                    ((c - NFA_ZOPEN) as usize, &raw mut (*subs).synt)
                } else {
                    ((c - NFA_MOPEN) as usize, &raw mut (*subs).norm)
                };

                let mut save_ptr: *mut u8 = ptr::null_mut();
                let mut save_multipos: multipos = zeroed();
                let save_in_use: c_int;

                if REG_MULTI() {
                    if (subidx as c_int) < (*sub).in_use {
                        save_multipos = (*sub).list.multi[subidx];
                        save_in_use = -1;
                    } else {
                        save_in_use = (*sub).in_use;
                        for i in (*sub).in_use as usize..subidx {
                            (*sub).list.multi[i].start_lnum = -1;
                            (*sub).list.multi[i].end_lnum = -1;
                        }
                        (*sub).in_use = subidx as c_int + 1;
                    }
                    if off == -1 {
                        (*sub).list.multi[subidx].start_lnum = rex.lnum + 1;
                        (*sub).list.multi[subidx].start_col = 0;
                    } else {
                        (*sub).list.multi[subidx].start_lnum = rex.lnum;
                        (*sub).list.multi[subidx].start_col =
                            (rex.input.offset_from(rex.line) as colnr_T) + off;
                    }
                    (*sub).list.multi[subidx].end_lnum = -1;
                } else {
                    if (subidx as c_int) < (*sub).in_use {
                        save_ptr = (*sub).list.line[subidx].start;
                        save_in_use = -1;
                    } else {
                        save_in_use = (*sub).in_use;
                        for i in (*sub).in_use as usize..subidx {
                            (*sub).list.line[i].start = ptr::null_mut();
                            (*sub).list.line[i].end = ptr::null_mut();
                        }
                        (*sub).in_use = subidx as c_int + 1;
                    }
                    (*sub).list.line[subidx].start = rex.input.offset(off as isize);
                }

                subs = addstate(l, (*state).out, subs, pim, off_arg, depth + 1);
                if !subs.is_null() {
                    sub = if (NFA_ZOPEN..=NFA_ZOPEN9).contains(&(*state).c) {
                        &raw mut (*subs).synt
                    } else {
                        &raw mut (*subs).norm
                    };
                    if save_in_use == -1 {
                        if REG_MULTI() {
                            (*sub).list.multi[subidx] = save_multipos;
                        } else {
                            (*sub).list.line[subidx].start = save_ptr;
                        }
                    } else {
                        (*sub).in_use = save_in_use;
                    }
                }
            }
            c if (NFA_MCLOSE..=NFA_MCLOSE9).contains(&c)
                || (NFA_ZCLOSE..=NFA_ZCLOSE9).contains(&c)
                || c == NFA_ZEND =>
            {
                if c == NFA_MCLOSE
                    && rex.nfa_has_zend != 0
                    && if REG_MULTI() {
                        (*subs).norm.list.multi[0].end_lnum >= 0
                    } else {
                        !(*subs).norm.list.line[0].end.is_null()
                    }
                {
                    subs = addstate(l, (*state).out, subs, pim, off_arg, depth + 1);
                } else {
                    let (subidx, mut sub) = if c == NFA_ZEND {
                        (0usize, &raw mut (*subs).norm)
                    } else if (NFA_ZCLOSE..=NFA_ZCLOSE9).contains(&c) {
                        ((c - NFA_ZCLOSE) as usize, &raw mut (*subs).synt)
                    } else {
                        ((c - NFA_MCLOSE) as usize, &raw mut (*subs).norm)
                    };

                    let save_in_use = (*sub).in_use;
                    if (*sub).in_use <= subidx as c_int {
                        (*sub).in_use = subidx as c_int + 1;
                    }
                    let mut save_ptr: *mut u8 = ptr::null_mut();
                    let mut save_multipos: multipos = zeroed();
                    if REG_MULTI() {
                        save_multipos = (*sub).list.multi[subidx];
                        if off == -1 {
                            (*sub).list.multi[subidx].end_lnum = rex.lnum + 1;
                            (*sub).list.multi[subidx].end_col = 0;
                        } else {
                            (*sub).list.multi[subidx].end_lnum = rex.lnum;
                            (*sub).list.multi[subidx].end_col =
                                (rex.input.offset_from(rex.line) as colnr_T) + off;
                        }
                    } else {
                        save_ptr = (*sub).list.line[subidx].end;
                        (*sub).list.line[subidx].end = rex.input.offset(off as isize);
                    }

                    subs = addstate(l, (*state).out, subs, pim, off_arg, depth + 1);
                    if !subs.is_null() {
                        sub = if (NFA_ZCLOSE..=NFA_ZCLOSE9).contains(&(*state).c) {
                            &raw mut (*subs).synt
                        } else {
                            &raw mut (*subs).norm
                        };
                        if REG_MULTI() {
                            (*sub).list.multi[subidx] = save_multipos;
                        } else {
                            (*sub).list.line[subidx].end = save_ptr;
                        }
                        (*sub).in_use = save_in_use;
                    }
                }
            }
            _ => {}
        }
        subs
    }
}

unsafe fn addstate_here(
    l: *mut nfa_list_T,
    state: *mut nfa_state_T,
    subs: *mut regsubs_T,
    pim: *mut nfa_pim_T,
    ip: *mut c_int,
) -> *mut regsubs_T {
    unsafe {
        let tlen = (*l).n;
        let listidx = *ip;

        let r = addstate(l, state, subs, pim, -listidx - ADDSTATE_HERE_OFFSET, 0);
        if r.is_null() {
            return ptr::null_mut();
        }
        if listidx + 1 == tlen {
            return r;
        }
        let count = (*l).n - tlen;
        if count == 0 {
            return r;
        }
        if count == 1 {
            *(*l).t.add(listidx as usize) = *(*l).t.add(((*l).n - 1) as usize);
        } else if count > 1 {
            if (*l).n + count > (*l).len {
                let newlen = (*l).len * 3 / 2 + 50;
                let newsize = (newlen as usize) * size_of::<nfa_thread_T>();
                if ((newsize >> 10) as i64) >= p_mmp {
                    emsg(tr(e_pattern_uses_more_memory_than_maxmempattern.as_ptr()));
                    return ptr::null_mut();
                }
                let newl = xmalloc(newsize).cast::<nfa_thread_T>();
                (*l).len = newlen;
                ptr::copy((*l).t, newl, listidx as usize);
                ptr::copy(
                    (*l).t.add(((*l).n - count) as usize),
                    newl.add(listidx as usize),
                    count as usize,
                );
                ptr::copy(
                    (*l).t.add(listidx as usize + 1),
                    newl.add((listidx + count) as usize),
                    ((*l).n - count - listidx - 1) as usize,
                );
                xfree((*l).t.cast());
                (*l).t = newl;
            } else {
                ptr::copy(
                    (*l).t.add(listidx as usize + 1),
                    (*l).t.add((listidx + count) as usize),
                    ((*l).n - listidx - 1) as usize,
                );
                ptr::copy(
                    (*l).t.add(((*l).n - 1) as usize),
                    (*l).t.add(listidx as usize),
                    count as usize,
                );
            }
        }
        (*l).n -= 1;
        *ip = listidx - 1;
        r
    }
}

unsafe fn check_char_class(cls: c_int, c: c_int) -> c_int {
    unsafe {
        match cls {
            NFA_CLASS_ALNUM => {
                if (1..128).contains(&c) && (c as u8).is_ascii_alphanumeric() {
                    return OK;
                }
            }
            NFA_CLASS_ALPHA => {
                if (1..128).contains(&c) && (c as u8).is_ascii_alphabetic() {
                    return OK;
                }
            }
            NFA_CLASS_BLANK => {
                if c == b' ' as c_int || c == b'\t' as c_int {
                    return OK;
                }
            }
            NFA_CLASS_CNTRL => {
                if (1..=127).contains(&c) && (c as u8).is_ascii_control() {
                    return OK;
                }
            }
            NFA_CLASS_DIGIT => {
                if ascii_isdigit(c) {
                    return OK;
                }
            }
            NFA_CLASS_GRAPH => {
                if (1..=127).contains(&c) && (c as u8).is_ascii_graphic() {
                    return OK;
                }
            }
            NFA_CLASS_LOWER => {
                if mb_islower(c) && c != 170 && c != 186 {
                    return OK;
                }
            }
            NFA_CLASS_PRINT => {
                if vim_isprintc(c) {
                    return OK;
                }
            }
            NFA_CLASS_PUNCT => {
                if (1..128).contains(&c) && (c as u8).is_ascii_punctuation() {
                    return OK;
                }
            }
            NFA_CLASS_SPACE => {
                if (9..=13).contains(&c) || c == b' ' as c_int {
                    return OK;
                }
            }
            NFA_CLASS_UPPER => {
                if mb_isupper(c) {
                    return OK;
                }
            }
            NFA_CLASS_XDIGIT => {
                if ascii_isxdigit(c) {
                    return OK;
                }
            }
            NFA_CLASS_TAB => {
                if c == b'\t' as c_int {
                    return OK;
                }
            }
            NFA_CLASS_RETURN => {
                if c == b'\r' as c_int {
                    return OK;
                }
            }
            NFA_CLASS_BACKSPACE => {
                if c == BS {
                    return OK;
                }
            }
            NFA_CLASS_ESCAPE => {
                if c == ESC {
                    return OK;
                }
            }
            NFA_CLASS_IDENT => {
                if vim_isIDc(c) {
                    return OK;
                }
            }
            NFA_CLASS_KEYWORD => {
                if reg_iswordc(c) {
                    return OK;
                }
            }
            NFA_CLASS_FNAME => {
                if vim_isfilec(c) {
                    return OK;
                }
            }
            _ => {
                siemsg!(tr(e_ill_char_class.as_ptr()), i64::from(cls));
                return FAIL;
            }
        }
        FAIL
    }
}

unsafe fn match_backref(sub: *mut regsub_T, subidx: c_int, bytelen: *mut c_int) -> bool {
    unsafe {
        if (*sub).in_use <= subidx {
            *bytelen = 0;
            return true;
        }
        let idx = subidx as usize;
        if REG_MULTI() {
            let mpos = (*sub).list.multi[idx];
            if mpos.start_lnum < 0 || mpos.end_lnum < 0 {
                *bytelen = 0;
                return true;
            }
            if mpos.start_lnum == rex.lnum && mpos.end_lnum == rex.lnum {
                let mut len = mpos.end_col - mpos.start_col;
                if cstrncmp(
                    rex.line.add(mpos.start_col as usize).cast(),
                    rex.input.cast(),
                    &raw mut len,
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
                bytelen,
            ) == RA_MATCH
            {
                return true;
            }
        } else {
            let lpos = (*sub).list.line[idx];
            if lpos.start.is_null() || lpos.end.is_null() {
                *bytelen = 0;
                return true;
            }
            let mut len = lpos.end.offset_from(lpos.start) as c_int;
            if cstrncmp(lpos.start.cast(), rex.input.cast(), &raw mut len) == 0 {
                *bytelen = len;
                return true;
            }
        }
        false
    }
}

unsafe fn match_zref(subidx: c_int, bytelen: *mut c_int) -> bool {
    unsafe {
        cleanup_zsubexpr();
        let idx = subidx as usize;
        if re_extmatch_in.is_null() || (*re_extmatch_in).matches[idx].is_null() {
            *bytelen = 0;
            return true;
        }
        let m = (*re_extmatch_in).matches[idx];
        let mut len = libc::strlen(m.cast()) as c_int;
        if cstrncmp(m.cast(), rex.input.cast(), &raw mut len) == 0 {
            *bytelen = len;
            return true;
        }
        false
    }
}

unsafe fn nfa_save_listids(prog: *mut nfa_regprog_T, list: *mut c_int) {
    unsafe {
        let mut p = (&raw mut (*prog).state).cast::<nfa_state_T>();
        let mut i = (*prog).nstate;
        while {
            i -= 1;
            i >= 0
        } {
            *list.add(i as usize) = (*p).lastlist[1];
            (*p).lastlist[1] = 0;
            p = p.add(1);
        }
    }
}

unsafe fn nfa_restore_listids(prog: *mut nfa_regprog_T, list: *const c_int) {
    unsafe {
        let mut p = (&raw mut (*prog).state).cast::<nfa_state_T>();
        let mut i = (*prog).nstate;
        while {
            i -= 1;
            i >= 0
        } {
            (*p).lastlist[1] = *list.add(i as usize);
            p = p.add(1);
        }
    }
}

#[inline]
fn nfa_re_num_cmp(val: u64, op: c_int, pos: u64) -> bool {
    if op == 1 {
        pos > val
    } else if op == 2 {
        pos < val
    } else {
        val == pos
    }
}

unsafe fn recursive_regmatch(
    state: *mut nfa_state_T,
    pim: *mut nfa_pim_T,
    prog: *mut nfa_regprog_T,
    submatch: *mut regsubs_T,
    m: *mut regsubs_T,
    listids: *mut *mut c_int,
    listids_len: *mut c_int,
) -> c_int {
    unsafe {
        let save_reginput_col = rex.input.offset_from(rex.line) as usize;
        let save_reglnum = rex.lnum;
        let save_nfa_match = nfa_match;
        let save_nfa_listid = rex.nfa_listid;
        let save_nfa_endp = nfa_endp;
        let mut endpos: save_se_T = zeroed();
        let mut endposp: *mut save_se_T = ptr::null_mut();
        let mut need_restore = false;

        if !pim.is_null() {
            if REG_MULTI() {
                rex.input = rex.line.add((*pim).end.pos.col as usize);
            } else {
                rex.input = (*pim).end.ptr;
            }
        }

        if (*state).c == NFA_START_INVISIBLE_BEFORE
            || (*state).c == NFA_START_INVISIBLE_BEFORE_FIRST
            || (*state).c == NFA_START_INVISIBLE_BEFORE_NEG
            || (*state).c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST
        {
            if REG_MULTI() {
                if pim.is_null() {
                    endpos.se_u.pos.col = rex.input.offset_from(rex.line) as colnr_T;
                    endpos.se_u.pos.lnum = rex.lnum;
                } else {
                    endpos.se_u.pos = (*pim).end.pos;
                }
            } else if pim.is_null() {
                endpos.se_u.ptr = rex.input;
            } else {
                endpos.se_u.ptr = (*pim).end.ptr;
            }
            endposp = &raw mut endpos;

            if (*state).val <= 0 {
                if REG_MULTI() {
                    rex.lnum -= 1;
                    rex.line = reg_getline(rex.lnum).cast();
                    if rex.line.is_null() {
                        rex.lnum += 1;
                        rex.line = reg_getline(rex.lnum).cast();
                    }
                }
                rex.input = rex.line;
            } else {
                if REG_MULTI() && (rex.input.offset_from(rex.line) as c_int) < (*state).val {
                    rex.lnum -= 1;
                    rex.line = reg_getline(rex.lnum).cast();
                    if rex.line.is_null() {
                        rex.lnum += 1;
                        rex.line = reg_getline(rex.lnum).cast();
                        rex.input = rex.line;
                    } else {
                        rex.input = rex.line.add(reg_getline_len(rex.lnum) as usize);
                    }
                }
                if (rex.input.offset_from(rex.line) as c_int) >= (*state).val {
                    rex.input = rex.input.sub((*state).val as usize);
                    rex.input = rex
                        .input
                        .sub(utf_head_off(rex.line.cast(), rex.input.cast()) as usize);
                } else {
                    rex.input = rex.line;
                }
            }
        }

        if nfa_ll_index == 1 {
            if (*listids).is_null() || *listids_len < (*prog).nstate {
                xfree((*listids).cast());
                *listids = xmalloc(size_of::<c_int>() * ((*prog).nstate as usize)).cast::<c_int>();
                *listids_len = (*prog).nstate;
            }
            nfa_save_listids(prog, *listids);
            need_restore = true;
        } else {
            nfa_ll_index += 1;
            if rex.nfa_listid <= rex.nfa_alt_listid {
                rex.nfa_listid = rex.nfa_alt_listid;
            }
        }

        nfa_endp = endposp;
        let result = nfa_regmatch(prog, (*state).out, submatch, m);

        if need_restore {
            nfa_restore_listids(prog, *listids);
        } else {
            nfa_ll_index -= 1;
            rex.nfa_alt_listid = rex.nfa_listid;
        }

        rex.lnum = save_reglnum;
        if REG_MULTI() {
            rex.line = reg_getline(rex.lnum).cast();
        }
        rex.input = rex.line.add(save_reginput_col);
        if result != NFA_TOO_EXPENSIVE {
            nfa_match = save_nfa_match;
            rex.nfa_listid = save_nfa_listid;
        }
        nfa_endp = save_nfa_endp;

        result
    }
}

unsafe fn failure_chance(state: *mut nfa_state_T, depth: c_int) -> c_int {
    unsafe {
        let c = (*state).c;
        if depth > 4 {
            return 1;
        }
        match c {
            NFA_SPLIT => {
                if (*(*state).out).c == NFA_SPLIT || (*(*state).out1).c == NFA_SPLIT {
                    return 1;
                }
                let l = failure_chance((*state).out, depth + 1);
                let r = failure_chance((*state).out1, depth + 1);
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
                failure_chance((*state).out, depth + 1)
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
}

unsafe fn skip_to_start(c: c_int, colp: *mut colnr_T) -> c_int {
    unsafe {
        let s = cstrchr(rex.line.add(*colp as usize).cast(), c);
        if s.is_null() {
            return FAIL;
        }
        *colp = s.cast::<u8>().offset_from(rex.line) as colnr_T;
        OK
    }
}

unsafe fn find_match_text(startcol: *mut colnr_T, regstart: c_int, match_text: *mut u8) -> c_int {
    unsafe {
        let mut col = *startcol;
        loop {
            let mut matched = true;
            let mut s1 = match_text;
            let regstart_len2 = utf_ptr2len(rex.line.add(col as usize).cast());
            let mut s2 = rex.line.add((col + regstart_len2) as usize);
            while *s1 != NUL {
                let c1_len = utf_ptr2len(s1.cast()) as usize;
                let c1 = utf_ptr2char(s1.cast());
                let c2_len = utf_ptr2len(s2.cast()) as usize;
                let c2 = utf_ptr2char(s2.cast());
                if c1 != c2 && (!rex.reg_ic || utf_fold(c1) != utf_fold(c2)) {
                    matched = false;
                    break;
                }
                s1 = s1.add(c1_len);
                s2 = s2.add(c2_len);
            }
            if matched && !utf_iscomposing_legacy(utf_ptr2char(s2.cast())) {
                cleanup_subexpr();
                if REG_MULTI() {
                    (*rex.reg_startpos).lnum = rex.lnum;
                    (*rex.reg_startpos).col = col;
                    (*rex.reg_endpos).lnum = rex.lnum;
                    (*rex.reg_endpos).col = s2.offset_from(rex.line) as colnr_T;
                } else {
                    *rex.reg_startp = rex.line.add(col as usize);
                    *rex.reg_endp = s2;
                }
                *startcol = col;
                return 1;
            }

            col += regstart_len2;
            if skip_to_start(regstart, &raw mut col) == FAIL {
                break;
            }
        }
        *startcol = col;
        0
    }
}

unsafe fn match_composing(mut sta: *mut nfa_state_T, curc: c_int, clen: c_int) -> bool {
    unsafe {
        let mut mc = curc;
        let mut len: c_int = 0;
        let mut cchars: [c_int; MAX_MCO] = [0; MAX_MCO];
        let mut ccount: usize = 0;

        if utf_iscomposing_legacy((*sta).c) {
            len += utf_char2len(mc);
        }
        if rex.reg_icombine && len == 0 {
            return (*sta).c == curc;
        }
        if len == 0 && mc != (*sta).c {
            return false;
        }
        if len == 0 {
            len += utf_char2len(mc);
            sta = (*sta).out;
        }

        while len < clen {
            mc = utf_ptr2char(rex.input.add(len as usize).cast());
            cchars[ccount] = mc;
            ccount += 1;
            len += utf_char2len(mc);
            if ccount == MAX_MCO {
                break;
            }
        }

        while (*sta).c != NFA_END_COMPOSING {
            let mut found = false;
            for &cc in cchars.iter().take(ccount) {
                if cc == (*sta).c {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
            sta = (*sta).out;
        }
        true
    }
}

unsafe fn nfa_did_time_out() -> bool {
    unsafe {
        if !nfa_time_limit.is_null() && profile_passed_limit(*nfa_time_limit) {
            if !nfa_timed_out.is_null() {
                *nfa_timed_out = 1;
            }
            return true;
        }
        false
    }
}

unsafe fn nfa_regmatch(
    prog: *mut nfa_regprog_T,
    start: *mut nfa_state_T,
    submatch: *mut regsubs_T,
    m: *mut regsubs_T,
) -> c_int {
    unsafe {
        let mut flag: usize = 0;
        let mut go_to_nextline = false;
        let mut list: [nfa_list_T; 2] = [zeroed(); 2];
        let mut listids: *mut c_int = ptr::null_mut();
        let mut listids_len: c_int = 0;
        let toplevel = (*start).c == NFA_MOPEN;

        reg_breakcheck();
        if got_int || nfa_did_time_out() {
            return 0;
        }

        nfa_match = 0;

        let size = ((*prog).nstate as usize + 1) * size_of::<nfa_thread_T>();
        if toplevel && !(*prog).listbuf[0].is_null() {
            list[0].t = (*prog).listbuf[0].cast();
            list[1].t = (*prog).listbuf[1].cast();
            (*prog).listbuf[0] = ptr::null_mut();
            (*prog).listbuf[1] = ptr::null_mut();
        } else {
            list[0].t = xmalloc(size).cast();
            list[1].t = xmalloc(size).cast();
        }
        list[0].len = (*prog).nstate + 1;
        list[1].len = (*prog).nstate + 1;
        list[0].n = 0;
        list[0].has_pim = 0;
        list[1].n = 0;
        list[1].has_pim = 0;
        list[0].id = rex.nfa_listid + 1;

        let r = if toplevel {
            if REG_MULTI() {
                (*m).norm.list.multi[0].start_lnum = rex.lnum;
                (*m).norm.list.multi[0].start_col = rex.input.offset_from(rex.line) as colnr_T;
                (*m).norm.orig_start_col = (*m).norm.list.multi[0].start_col;
            } else {
                (*m).norm.list.line[0].start = rex.input;
            }
            (*m).norm.in_use = 1;
            addstate(&raw mut list[0], (*start).out, m, ptr::null_mut(), 0, 0)
        } else {
            addstate(&raw mut list[0], start, m, ptr::null_mut(), 0, 0)
        };
        if r.is_null() {
            nfa_match = NFA_TOO_EXPENSIVE;
            return finish_nfa_regmatch(prog, &mut list, listids);
        }

        loop {
            let curc: c_int;
            let mut clen: c_int;
            if *rex.input != NUL && *rex.input < 0x80 && *rex.input.add(1) < 0x80 {
                curc = *rex.input as c_int;
                clen = 1;
            } else {
                curc = utf_ptr2char(rex.input.cast());
                clen = utfc_ptr2len(rex.input.cast());
            }
            if curc == NUL as c_int {
                clen = 0;
                go_to_nextline = false;
            }

            let this_idx = flag;
            flag ^= 1;
            let next_idx = flag;
            let thislist = (&raw mut list).cast::<nfa_list_T>().add(this_idx);
            let nextlist = (&raw mut list).cast::<nfa_list_T>().add(next_idx);
            (*nextlist).n = 0;
            (*nextlist).has_pim = 0;
            rex.nfa_listid += 1;
            if (*prog).re_engine == AUTOMATIC_ENGINE as c_uint && rex.nfa_listid >= NFA_MAX_STATES {
                nfa_match = NFA_TOO_EXPENSIVE;
                return finish_nfa_regmatch(prog, &mut list, listids);
            }

            (*thislist).id = rex.nfa_listid;
            (*nextlist).id = rex.nfa_listid + 1;

            if (*thislist).n == 0 {
                break;
            }

            let mut goto_nextchar = false;
            let mut listidx: c_int = 0;
            while listidx < (*thislist).n {
                reg_breakcheck();
                if got_int {
                    break;
                }
                if !nfa_time_limit.is_null() {
                    nfa_time_count += 1;
                    if nfa_time_count == 20 {
                        nfa_time_count = 0;
                        if nfa_did_time_out() {
                            break;
                        }
                    }
                }
                let t = (*thislist).t.add(listidx as usize);

                let mut add_state: *mut nfa_state_T = ptr::null_mut();
                let mut add_here = false;
                let mut add_count: c_int = 0;
                let mut add_off: c_int = 0;

                match (*(*t).state).c {
                    NFA_MATCH => {
                        if !rex.reg_icombine
                            && rex.input != rex.line
                            && utf_iscomposing_legacy(curc)
                        {
                            listidx += 1;
                            continue;
                        }
                        nfa_match = 1;
                        copy_sub(&raw mut (*submatch).norm, &raw mut (*t).subs.norm);
                        if rex.nfa_has_zsubexpr != 0 {
                            copy_sub(&raw mut (*submatch).synt, &raw mut (*t).subs.synt);
                        }
                        if (*nextlist).n == 0 {
                            clen = 0;
                        }
                        goto_nextchar = true;
                        break;
                    }
                    NFA_END_INVISIBLE | NFA_END_INVISIBLE_NEG | NFA_END_PATTERN => {
                        if !nfa_endp.is_null()
                            && if REG_MULTI() {
                                rex.lnum != (*nfa_endp).se_u.pos.lnum
                                    || (rex.input.offset_from(rex.line) as colnr_T)
                                        != (*nfa_endp).se_u.pos.col
                            } else {
                                rex.input != (*nfa_endp).se_u.ptr
                            }
                        {
                            listidx += 1;
                            continue;
                        }
                        if (*(*t).state).c != NFA_END_INVISIBLE_NEG {
                            copy_sub(&raw mut (*m).norm, &raw mut (*t).subs.norm);
                            if rex.nfa_has_zsubexpr != 0 {
                                copy_sub(&raw mut (*m).synt, &raw mut (*t).subs.synt);
                            }
                        }
                        nfa_match = 1;
                        if (*nextlist).n == 0 {
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
                        let st_c = (*(*t).state).c;
                        if (*t).pim.result != NFA_PIM_UNUSED
                            || st_c == NFA_START_INVISIBLE_FIRST
                            || st_c == NFA_START_INVISIBLE_NEG_FIRST
                            || st_c == NFA_START_INVISIBLE_BEFORE_FIRST
                            || st_c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST
                        {
                            let in_use = (*m).norm.in_use;
                            copy_sub_off(&raw mut (*m).norm, &raw mut (*t).subs.norm);
                            if rex.nfa_has_zsubexpr != 0 {
                                copy_sub_off(&raw mut (*m).synt, &raw mut (*t).subs.synt);
                            }
                            let res = recursive_regmatch(
                                (*t).state,
                                ptr::null_mut(),
                                prog,
                                submatch,
                                m,
                                &raw mut listids,
                                &raw mut listids_len,
                            );
                            if res == NFA_TOO_EXPENSIVE {
                                nfa_match = res;
                                return finish_nfa_regmatch(prog, &mut list, listids);
                            }
                            let is_neg = st_c == NFA_START_INVISIBLE_NEG
                                || st_c == NFA_START_INVISIBLE_NEG_FIRST
                                || st_c == NFA_START_INVISIBLE_BEFORE_NEG
                                || st_c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST;
                            if (res != 0) != is_neg {
                                copy_sub_off(&raw mut (*t).subs.norm, &raw mut (*m).norm);
                                if rex.nfa_has_zsubexpr != 0 {
                                    copy_sub_off(&raw mut (*t).subs.synt, &raw mut (*m).synt);
                                }
                                copy_ze_off(&raw mut (*t).subs.norm, &raw mut (*m).norm);
                                add_here = true;
                                add_state = (*(*(*t).state).out1).out;
                            }
                            (*m).norm.in_use = in_use;
                        } else {
                            let mut pim: nfa_pim_T = zeroed();
                            pim.state = (*t).state;
                            pim.result = NFA_PIM_TODO;
                            pim.subs.norm.in_use = 0;
                            pim.subs.synt.in_use = 0;
                            if REG_MULTI() {
                                pim.end.pos.col = rex.input.offset_from(rex.line) as colnr_T;
                                pim.end.pos.lnum = rex.lnum;
                            } else {
                                pim.end.ptr = rex.input;
                            }
                            if addstate_here(
                                thislist,
                                (*(*(*t).state).out1).out,
                                &raw mut (*t).subs,
                                &raw mut pim,
                                &raw mut listidx,
                            )
                            .is_null()
                            {
                                nfa_match = NFA_TOO_EXPENSIVE;
                                return finish_nfa_regmatch(prog, &mut list, listids);
                            }
                        }
                    }
                    NFA_START_PATTERN => {
                        let out1_out = (*(*(*t).state).out1).out;
                        let skip = if state_in_list(nextlist, out1_out, &raw mut (*t).subs) {
                            out1_out
                        } else if state_in_list(nextlist, (*out1_out).out, &raw mut (*t).subs)
                            || state_in_list(thislist, (*out1_out).out, &raw mut (*t).subs)
                        {
                            (*out1_out).out
                        } else {
                            ptr::null_mut()
                        };
                        if skip.is_null() {
                            copy_sub_off(&raw mut (*m).norm, &raw mut (*t).subs.norm);
                            if rex.nfa_has_zsubexpr != 0 {
                                copy_sub_off(&raw mut (*m).synt, &raw mut (*t).subs.synt);
                            }
                            let res = recursive_regmatch(
                                (*t).state,
                                ptr::null_mut(),
                                prog,
                                submatch,
                                m,
                                &raw mut listids,
                                &raw mut listids_len,
                            );
                            if res == NFA_TOO_EXPENSIVE {
                                nfa_match = res;
                                return finish_nfa_regmatch(prog, &mut list, listids);
                            }
                            if res != 0 {
                                copy_sub_off(&raw mut (*t).subs.norm, &raw mut (*m).norm);
                                if rex.nfa_has_zsubexpr != 0 {
                                    copy_sub_off(&raw mut (*t).subs.synt, &raw mut (*m).synt);
                                }
                                let bytelen = if REG_MULTI() {
                                    (*m).norm.list.multi[0].end_col
                                        - (rex.input.offset_from(rex.line) as colnr_T)
                                } else {
                                    (*m).norm.list.line[0].end.offset_from(rex.input) as c_int
                                };
                                if bytelen == 0 {
                                    add_here = true;
                                    add_state = (*(*(*(*t).state).out1).out).out;
                                } else if bytelen <= clen {
                                    add_state = (*(*(*(*t).state).out1).out).out;
                                    add_off = clen;
                                } else {
                                    add_state = (*(*(*t).state).out1).out;
                                    add_off = bytelen;
                                    add_count = bytelen - clen;
                                }
                            }
                        }
                    }
                    NFA_BOL => {
                        if rex.input == rex.line {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    NFA_EOL => {
                        if curc == NUL as c_int {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    NFA_BOW => {
                        if curc != NUL as c_int {
                            let this_class = mb_get_class_tab(
                                rex.input.cast(),
                                (&raw const (*rex.reg_buf).b_chartab).cast(),
                            );
                            if this_class > 1 && reg_prev_class() != this_class {
                                add_here = true;
                                add_state = (*(*t).state).out;
                            }
                        }
                    }
                    NFA_EOW => {
                        if rex.input != rex.line {
                            let this_class = mb_get_class_tab(
                                rex.input.cast(),
                                (&raw const (*rex.reg_buf).b_chartab).cast(),
                            );
                            let prev_class = reg_prev_class();
                            if this_class != prev_class && prev_class != 0 && prev_class != 1 {
                                add_here = true;
                                add_state = (*(*t).state).out;
                            }
                        }
                    }
                    NFA_BOF => {
                        if rex.lnum == 0
                            && rex.input == rex.line
                            && (!REG_MULTI() || rex.reg_firstlnum == 1)
                        {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    NFA_EOF => {
                        if rex.lnum == rex.reg_maxline && curc == NUL as c_int {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    NFA_COMPOSING => {
                        if match_composing((*(*t).state).out, curc, clen) {
                            let end = (*(*t).state).out1;
                            add_state = (*end).out;
                            add_off = clen;
                        }
                    }
                    NFA_NEWL => {
                        if curc == NUL as c_int
                            && !rex.reg_line_lbr
                            && REG_MULTI()
                            && rex.lnum <= rex.reg_maxline
                        {
                            go_to_nextline = true;
                            add_state = (*(*t).state).out;
                            add_off = -1;
                        } else if curc == b'\n' as c_int && rex.reg_line_lbr {
                            add_state = (*(*t).state).out;
                            add_off = 1;
                        }
                    }
                    NFA_START_COLL | NFA_START_NEG_COLL => {
                        if curc != NUL as c_int {
                            let mut state = (*(*t).state).out;
                            let result_if_matched = (*(*t).state).c == NFA_START_COLL;
                            let mut matched = false;
                            loop {
                                if (*state).c == NFA_COMPOSING {
                                    matched = match_composing((*(*(*t).state).out).out, curc, clen);
                                    if matched
                                        && !(*(*(*t).state).out).out1.is_null()
                                        && (*(*(*(*t).state).out).out1).c == NFA_END_COMPOSING
                                    {
                                        let end = (*(*(*t).state).out).out1;
                                        add_state = (*end).out;
                                        add_off = clen;
                                    }
                                    break;
                                }
                                if (*state).c == NFA_END_COLL {
                                    matched = !result_if_matched;
                                    break;
                                }
                                if (*state).c == NFA_RANGE_MIN {
                                    let mut c1 = (*state).val;
                                    state = (*state).out;
                                    let c2 = (*state).val;
                                    if curc >= c1 && curc <= c2 {
                                        matched = result_if_matched;
                                        break;
                                    }
                                    if rex.reg_ic {
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
                                } else if if (*state).c < 0 {
                                    check_char_class((*state).c, curc) != 0
                                } else {
                                    curc == (*state).c
                                        || (rex.reg_ic && utf_fold(curc) == utf_fold((*state).c))
                                } {
                                    matched = result_if_matched;
                                    break;
                                }
                                state = (*state).out;
                            }
                            if matched {
                                add_state = (*(*(*t).state).out1).out;
                                add_off = clen;
                            }
                        }
                    }
                    NFA_ANY => {
                        if curc > 0 {
                            add_state = (*(*t).state).out;
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
                        add_state = (*(*t).state).out;
                    }
                    NFA_IDENT => {
                        if vim_isIDc(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_SIDENT => {
                        if !ascii_isdigit(curc) && vim_isIDc(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_KWORD => {
                        if vim_iswordp_buf(rex.input.cast(), rex.reg_buf) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_SKWORD => {
                        if !ascii_isdigit(curc) && vim_iswordp_buf(rex.input.cast(), rex.reg_buf) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_FNAME => {
                        if vim_isfilec(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_SFNAME => {
                        if !ascii_isdigit(curc) && vim_isfilec(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_PRINT => {
                        if vim_isprintc(utf_ptr2char(rex.input.cast())) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_SPRINT => {
                        if !ascii_isdigit(curc) && vim_isprintc(utf_ptr2char(rex.input.cast())) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_WHITE => {
                        if ascii_iswhite(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NWHITE => {
                        if curc != NUL as c_int && !ascii_iswhite(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_DIGIT => {
                        if ri_digit(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NDIGIT => {
                        if curc != NUL as c_int && !ri_digit(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_HEX => {
                        if ri_hex(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NHEX => {
                        if curc != NUL as c_int && !ri_hex(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_OCTAL => {
                        if ri_octal(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NOCTAL => {
                        if curc != NUL as c_int && !ri_octal(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_WORD => {
                        if ri_word(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NWORD => {
                        if curc != NUL as c_int && !ri_word(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_HEAD => {
                        if ri_head(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NHEAD => {
                        if curc != NUL as c_int && !ri_head(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_ALPHA => {
                        if ri_alpha(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NALPHA => {
                        if curc != NUL as c_int && !ri_alpha(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_LOWER => {
                        if ri_lower(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NLOWER => {
                        if curc != NUL as c_int && !ri_lower(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_UPPER => {
                        if ri_upper(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NUPPER => {
                        if curc != NUL as c_int && !ri_upper(curc) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_LOWER_IC => {
                        if ri_lower(curc) || (rex.reg_ic && ri_upper(curc)) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NLOWER_IC => {
                        if curc != NUL as c_int
                            && !(ri_lower(curc) || (rex.reg_ic && ri_upper(curc)))
                        {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_UPPER_IC => {
                        if ri_upper(curc) || (rex.reg_ic && ri_lower(curc)) {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    NFA_NUPPER_IC => {
                        if curc != NUL as c_int
                            && !(ri_upper(curc) || (rex.reg_ic && ri_lower(curc)))
                        {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                    c if (NFA_BACKREF1..=NFA_ZREF9).contains(&c) => {
                        let mut bytelen: c_int = 0;
                        let matched = if (NFA_BACKREF1..=NFA_BACKREF9).contains(&c) {
                            let subidx = c - NFA_BACKREF1 + 1;
                            match_backref(&raw mut (*t).subs.norm, subidx, &raw mut bytelen)
                        } else {
                            let subidx = c - NFA_ZREF1 + 1;
                            match_zref(subidx, &raw mut bytelen)
                        };
                        if matched {
                            if bytelen == 0 {
                                add_here = true;
                                add_state = (*(*(*t).state).out).out;
                            } else if bytelen <= clen {
                                add_state = (*(*(*t).state).out).out;
                                add_off = clen;
                            } else {
                                add_state = (*(*t).state).out;
                                add_off = bytelen;
                                add_count = bytelen - clen;
                            }
                        }
                    }
                    NFA_SKIP => {
                        if (*t).count - clen <= 0 {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        } else {
                            add_state = (*t).state;
                            add_off = 0;
                            add_count = (*t).count - clen;
                        }
                    }
                    NFA_LNUM | NFA_LNUM_GT | NFA_LNUM_LT => {
                        if REG_MULTI()
                            && nfa_re_num_cmp(
                                (*(*t).state).val as u64,
                                (*(*t).state).c - NFA_LNUM,
                                (rex.lnum as u64).wrapping_add(rex.reg_firstlnum as u64),
                            )
                        {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    NFA_COL | NFA_COL_GT | NFA_COL_LT => {
                        if nfa_re_num_cmp(
                            (*(*t).state).val as u64,
                            (*(*t).state).c - NFA_COL,
                            (rex.input.offset_from(rex.line) + 1) as u64,
                        ) {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    NFA_VCOL | NFA_VCOL_GT | NFA_VCOL_LT => {
                        let op = (*(*t).state).c - NFA_VCOL;
                        let col = rex.input.offset_from(rex.line) as colnr_T;
                        if !(op != 1 && col > (*(*t).state).val * MB_MAXBYTES) {
                            let mut matched = false;
                            let wp = if rex.reg_win.is_null() {
                                curwin
                            } else {
                                rex.reg_win
                            };
                            if op == 1 && col - 1 > (*(*t).state).val && col > 100 {
                                let mut ts = (*(*wp).w_buffer).b_p_ts;
                                if ts < 4 {
                                    ts = 4;
                                }
                                matched = i64::from(col) > i64::from((*(*t).state).val) * ts;
                            }
                            if !matched {
                                let mut lnum = if REG_MULTI() {
                                    rex.reg_firstlnum + rex.lnum
                                } else {
                                    1
                                };
                                if REG_MULTI()
                                    && (lnum <= 0 || lnum > (*(*wp).w_buffer).b_ml.ml_line_count)
                                {
                                    lnum = 1;
                                }
                                let vcol = win_linetabsize(wp, lnum, rex.line.cast(), col);
                                matched = nfa_re_num_cmp(
                                    (*(*t).state).val as u64,
                                    op,
                                    (vcol as u64).wrapping_add(1),
                                );
                            }
                            if matched {
                                add_here = true;
                                add_state = (*(*t).state).out;
                            }
                        }
                    }
                    NFA_MARK | NFA_MARK_GT | NFA_MARK_LT => {
                        let col = if REG_MULTI() {
                            rex.input.offset_from(rex.line) as usize
                        } else {
                            0
                        };
                        let fm = mark_get(
                            rex.reg_buf,
                            curwin,
                            ptr::null_mut(),
                            kMarkBufLocal,
                            (*(*t).state).val,
                        );
                        if REG_MULTI() {
                            rex.line = reg_getline(rex.lnum).cast();
                            rex.input = rex.line.add(col);
                        }
                        if !fm.is_null() && (*fm).mark.lnum > 0 {
                            let pos = &raw const (*fm).mark;
                            let pos_col = if (*pos).lnum == rex.lnum + rex.reg_firstlnum
                                && (*pos).col == MAXCOL
                            {
                                reg_getline_len((*pos).lnum - rex.reg_firstlnum)
                            } else {
                                (*pos).col
                            };
                            let cur_col = rex.input.offset_from(rex.line) as colnr_T;
                            let st_c = (*(*t).state).c;
                            let matched = if (*pos).lnum == rex.lnum + rex.reg_firstlnum {
                                if pos_col == cur_col {
                                    st_c == NFA_MARK
                                } else if pos_col < cur_col {
                                    st_c == NFA_MARK_GT
                                } else {
                                    st_c == NFA_MARK_LT
                                }
                            } else if (*pos).lnum < rex.lnum + rex.reg_firstlnum {
                                st_c == NFA_MARK_GT
                            } else {
                                st_c == NFA_MARK_LT
                            };
                            if matched {
                                add_here = true;
                                add_state = (*(*t).state).out;
                            }
                        }
                    }
                    NFA_CURSOR => {
                        if !rex.reg_win.is_null()
                            && rex.lnum + rex.reg_firstlnum == (*rex.reg_win).w_cursor.lnum
                            && (rex.input.offset_from(rex.line) as colnr_T)
                                == (*rex.reg_win).w_cursor.col
                        {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    NFA_VISUAL => {
                        if reg_match_visual() {
                            add_here = true;
                            add_state = (*(*t).state).out;
                        }
                    }
                    c if (NFA_MOPEN1..=NFA_MOPEN9).contains(&c)
                        || (NFA_ZOPEN..=NFA_ZOPEN9).contains(&c)
                        || c == NFA_NOPEN
                        || c == NFA_ZSTART => {}
                    c => {
                        let mut matched = c == curc;
                        if !matched && rex.reg_ic {
                            matched = utf_fold(c) == utf_fold(curc);
                        }
                        if matched && !rex.reg_icombine {
                            clen = utf_ptr2len(rex.input.cast());
                        }
                        if matched {
                            add_state = (*(*t).state).out;
                            add_off = clen;
                        }
                    }
                }

                if !add_state.is_null() {
                    let mut pim: *mut nfa_pim_T = if (*t).pim.result == NFA_PIM_UNUSED {
                        ptr::null_mut()
                    } else {
                        &raw mut (*t).pim
                    };
                    let mut pim_copy: nfa_pim_T = zeroed();

                    if !pim.is_null() && (clen == 0 || match_follows(add_state, 0)) {
                        let pim_c = (*(*pim).state).c;
                        let is_neg = pim_c == NFA_START_INVISIBLE_NEG
                            || pim_c == NFA_START_INVISIBLE_NEG_FIRST
                            || pim_c == NFA_START_INVISIBLE_BEFORE_NEG
                            || pim_c == NFA_START_INVISIBLE_BEFORE_NEG_FIRST;
                        let res = if (*pim).result == NFA_PIM_TODO {
                            let r = recursive_regmatch(
                                (*pim).state,
                                pim,
                                prog,
                                submatch,
                                m,
                                &raw mut listids,
                                &raw mut listids_len,
                            );
                            (*pim).result = if r != 0 {
                                NFA_PIM_MATCH
                            } else {
                                NFA_PIM_NOMATCH
                            };
                            if (r != 0) != is_neg {
                                copy_sub_off(&raw mut (*pim).subs.norm, &raw mut (*m).norm);
                                if rex.nfa_has_zsubexpr != 0 {
                                    copy_sub_off(&raw mut (*pim).subs.synt, &raw mut (*m).synt);
                                }
                            }
                            r != 0
                        } else {
                            (*pim).result == NFA_PIM_MATCH
                        };

                        if res != is_neg {
                            copy_sub_off(&raw mut (*t).subs.norm, &raw mut (*pim).subs.norm);
                            if rex.nfa_has_zsubexpr != 0 {
                                copy_sub_off(&raw mut (*t).subs.synt, &raw mut (*pim).subs.synt);
                            }
                        } else {
                            listidx += 1;
                            continue;
                        }
                        pim = ptr::null_mut();
                    }

                    if pim == &raw mut (*t).pim {
                        copy_pim(&raw mut pim_copy, pim);
                        pim = &raw mut pim_copy;
                    }

                    let r = if add_here {
                        addstate_here(
                            thislist,
                            add_state,
                            &raw mut (*t).subs,
                            pim,
                            &raw mut listidx,
                        )
                    } else {
                        let r = addstate(nextlist, add_state, &raw mut (*t).subs, pim, add_off, 0);
                        if add_count > 0 && !r.is_null() {
                            (*(*nextlist).t.add(((*nextlist).n - 1) as usize)).count = add_count;
                        }
                        r
                    };
                    if r.is_null() {
                        nfa_match = NFA_TOO_EXPENSIVE;
                        return finish_nfa_regmatch(prog, &mut list, listids);
                    }
                }

                listidx += 1;
            }

            if !goto_nextchar
                && nfa_match == 0
                && ((toplevel
                    && rex.lnum == 0
                    && clen != 0
                    && (rex.reg_maxcol == 0
                        || (rex.input.offset_from(rex.line) as colnr_T) < rex.reg_maxcol))
                    || (!nfa_endp.is_null()
                        && if REG_MULTI() {
                            rex.lnum < (*nfa_endp).se_u.pos.lnum
                                || (rex.lnum == (*nfa_endp).se_u.pos.lnum
                                    && (rex.input.offset_from(rex.line) as colnr_T)
                                        < (*nfa_endp).se_u.pos.col)
                        } else {
                            rex.input < (*nfa_endp).se_u.ptr
                        }))
            {
                if toplevel {
                    let mut add = true;
                    if (*prog).regstart != NUL as c_int && clen != 0 {
                        if (*nextlist).n == 0 {
                            let mut col = (rex.input.offset_from(rex.line) as colnr_T) + clen;
                            if skip_to_start((*prog).regstart, &raw mut col) == FAIL {
                                break;
                            }
                            rex.input = rex.line.add((col - clen) as usize);
                        } else {
                            let c = utf_ptr2char(rex.input.add(clen as usize).cast());
                            if c != (*prog).regstart
                                && (!rex.reg_ic || utf_fold(c) != utf_fold((*prog).regstart))
                            {
                                add = false;
                            }
                        }
                    }
                    if add {
                        if REG_MULTI() {
                            (*m).norm.list.multi[0].start_col =
                                (rex.input.offset_from(rex.line) as colnr_T) + clen;
                            (*m).norm.orig_start_col = (*m).norm.list.multi[0].start_col;
                        } else {
                            (*m).norm.list.line[0].start = rex.input.add(clen as usize);
                        }
                        if addstate(nextlist, (*start).out, m, ptr::null_mut(), clen, 0).is_null() {
                            nfa_match = NFA_TOO_EXPENSIVE;
                            return finish_nfa_regmatch(prog, &mut list, listids);
                        }
                    }
                } else {
                    let save_line = rex.line;
                    let save_input = rex.input;
                    let save_lnum = rex.lnum;

                    if REG_MULTI()
                        && clen == 0
                        && !nfa_endp.is_null()
                        && rex.lnum < (*nfa_endp).se_u.pos.lnum
                    {
                        let next_line = reg_getline(rex.lnum + 1);
                        if !next_line.is_null() {
                            rex.line = next_line.cast();
                            rex.input = rex.line;
                            rex.lnum += 1;
                        }
                    }

                    let r = addstate(nextlist, start, m, ptr::null_mut(), clen, 0);

                    rex.line = save_line;
                    rex.input = save_input;
                    rex.lnum = save_lnum;

                    if r.is_null() {
                        nfa_match = NFA_TOO_EXPENSIVE;
                        return finish_nfa_regmatch(prog, &mut list, listids);
                    }
                }
            }

            if clen != 0 {
                rex.input = rex.input.add(clen as usize);
            } else if go_to_nextline
                || (!nfa_endp.is_null() && REG_MULTI() && rex.lnum < (*nfa_endp).se_u.pos.lnum)
            {
                reg_nextline();
            } else {
                break;
            }

            reg_breakcheck();
            if got_int {
                break;
            }
            if !nfa_time_limit.is_null() {
                nfa_time_count += 1;
                if nfa_time_count == 20 {
                    nfa_time_count = 0;
                    if nfa_did_time_out() {
                        break;
                    }
                }
            }
        }

        finish_nfa_regmatch(prog, &mut list, listids)
    }
}

unsafe fn finish_nfa_regmatch(
    prog: *mut nfa_regprog_T,
    list: &mut [nfa_list_T; 2],
    listids: *mut c_int,
) -> c_int {
    unsafe {
        if (*prog).listbuf[0].is_null() && !list[0].t.is_null() && !list[1].t.is_null() {
            (*prog).listbuf[0] = list[0].t.cast();
            (*prog).listbuf[1] = list[1].t.cast();
        } else {
            xfree(list[0].t.cast());
            xfree(list[1].t.cast());
        }
        xfree(listids.cast());
        nfa_match
    }
}

unsafe fn nfa_regtry(
    prog: *mut nfa_regprog_T,
    col: colnr_T,
    tm: *mut proftime_T,
    timed_out: *mut c_int,
) -> c_int {
    unsafe {
        let mut subs: regsubs_T = zeroed();
        let mut m: regsubs_T = zeroed();
        let start = (*prog).start;

        rex.input = rex.line.add(col as usize);
        nfa_time_limit = tm;
        nfa_timed_out = timed_out;
        nfa_time_count = 0;

        clear_sub(&raw mut subs.norm);
        clear_sub(&raw mut m.norm);
        clear_sub(&raw mut subs.synt);
        clear_sub(&raw mut m.synt);

        let result = nfa_regmatch(prog, start, &raw mut subs, &raw mut m);
        if result == 0 {
            return 0;
        } else if result == NFA_TOO_EXPENSIVE {
            return result;
        }

        cleanup_subexpr();
        if REG_MULTI() {
            for i in 0..subs.norm.in_use as usize {
                (*rex.reg_startpos.add(i)).lnum = subs.norm.list.multi[i].start_lnum;
                (*rex.reg_startpos.add(i)).col = subs.norm.list.multi[i].start_col;
                (*rex.reg_endpos.add(i)).lnum = subs.norm.list.multi[i].end_lnum;
                (*rex.reg_endpos.add(i)).col = subs.norm.list.multi[i].end_col;
            }
            if !rex.reg_mmatch.is_null() {
                (*rex.reg_mmatch).rmm_matchcol = subs.norm.orig_start_col;
            }
            if (*rex.reg_startpos).lnum < 0 {
                (*rex.reg_startpos).lnum = 0;
                (*rex.reg_startpos).col = col;
            }
            if (*rex.reg_endpos).lnum < 0 {
                (*rex.reg_endpos).lnum = rex.lnum;
                (*rex.reg_endpos).col = rex.input.offset_from(rex.line) as colnr_T;
            } else {
                rex.lnum = (*rex.reg_endpos).lnum;
            }
        } else {
            for i in 0..subs.norm.in_use as usize {
                *rex.reg_startp.add(i) = subs.norm.list.line[i].start;
                *rex.reg_endp.add(i) = subs.norm.list.line[i].end;
            }
            if (*rex.reg_startp).is_null() {
                *rex.reg_startp = rex.line.add(col as usize);
            }
            if (*rex.reg_endp).is_null() {
                *rex.reg_endp = rex.input;
            }
        }

        unref_extmatch(re_extmatch_out);
        re_extmatch_out = ptr::null_mut();

        if (*prog).reghasz == REX_SET {
            cleanup_zsubexpr();
            re_extmatch_out = make_extmatch();
            for i in 1..subs.synt.in_use as usize {
                if REG_MULTI() {
                    let mpos = subs.synt.list.multi[i];
                    if mpos.start_lnum >= 0
                        && mpos.start_lnum == mpos.end_lnum
                        && mpos.end_col >= mpos.start_col
                    {
                        (*re_extmatch_out).matches[i] = xstrnsave(
                            reg_getline(mpos.start_lnum).add(mpos.start_col as usize),
                            (mpos.end_col - mpos.start_col) as usize,
                        )
                        .cast();
                    }
                } else {
                    let lpos = subs.synt.list.line[i];
                    if !lpos.start.is_null() && !lpos.end.is_null() {
                        (*re_extmatch_out).matches[i] =
                            xstrnsave(lpos.start.cast(), lpos.end.offset_from(lpos.start) as usize)
                                .cast();
                    }
                }
            }
        }

        1 + rex.lnum
    }
}

unsafe fn nfa_regexec_both(
    mut line: *mut u8,
    startcol: colnr_T,
    tm: *mut proftime_T,
    timed_out: *mut c_int,
) -> c_int {
    unsafe {
        let mut retval: c_int = 0;
        let mut col = startcol;

        let prog = if REG_MULTI() {
            let p = (*rex.reg_mmatch).regprog.cast::<nfa_regprog_T>();
            line = reg_getline(0).cast();
            rex.reg_startpos = (&raw mut (*rex.reg_mmatch).startpos).cast();
            rex.reg_endpos = (&raw mut (*rex.reg_mmatch).endpos).cast();
            p
        } else {
            let p = (*rex.reg_match).regprog.cast::<nfa_regprog_T>();
            rex.reg_startp = (&raw mut (*rex.reg_match).startp).cast();
            rex.reg_endp = (&raw mut (*rex.reg_match).endp).cast();
            p
        };

        if prog.is_null() || line.is_null() {
            iemsg(tr((&raw const e_null).cast()));
            return finish_nfa_regexec(retval, col);
        }

        if ((*prog).regflags & RF_ICASE) != 0 {
            rex.reg_ic = true;
        } else if ((*prog).regflags & RF_NOICASE) != 0 {
            rex.reg_ic = false;
        }

        if ((*prog).regflags & RF_ICOMBINE) != 0 {
            rex.reg_icombine = true;
        }

        rex.line = line;
        rex.lnum = 0;

        rex.nfa_has_zend = (*prog).has_zend;
        rex.nfa_has_backref = (*prog).has_backref;
        rex.nfa_nsubexpr = (*prog).nsubexp;
        rex.nfa_listid = 1;
        rex.nfa_alt_listid = 2;

        if (*prog).reganch != 0 && col > 0 {
            return 0;
        }

        rex.need_clear_subexpr = 1;
        if (*prog).reghasz == REX_SET {
            rex.nfa_has_zsubexpr = 1;
            rex.need_clear_zsubexpr = 1;
        } else {
            rex.nfa_has_zsubexpr = 0;
            rex.need_clear_zsubexpr = 0;
        }

        if (*prog).regstart != NUL as c_int {
            if skip_to_start((*prog).regstart, &raw mut col) == FAIL {
                return 0;
            }
            if !(*prog).match_text.is_null() && *(*prog).match_text != NUL && !rex.reg_icombine {
                retval = find_match_text(&raw mut col, (*prog).regstart, (*prog).match_text);
                if REG_MULTI() {
                    (*rex.reg_mmatch).rmm_matchcol = col;
                } else {
                    (*rex.reg_match).rm_matchcol = col;
                }
                return retval;
            }
        }

        if rex.reg_maxcol > 0 && col >= rex.reg_maxcol {
            return finish_nfa_regexec(retval, col);
        }

        nstate = 0;
        let states = (&raw mut (*prog).state).cast::<nfa_state_T>();
        for i in 0..(*prog).nstate as usize {
            let st = states.add(i);
            (*st).id = i as c_int;
            (*st).lastlist[0] = 0;
            (*st).lastlist[1] = 0;
        }

        retval = nfa_regtry(prog, col, tm, timed_out);
        finish_nfa_regexec(retval, col)
    }
}

unsafe fn finish_nfa_regexec(retval: c_int, col: colnr_T) -> c_int {
    unsafe {
        if retval > 0 {
            if REG_MULTI() {
                let start = (*rex.reg_mmatch).startpos[0];
                let end = (*rex.reg_mmatch).endpos[0];
                if end.lnum < start.lnum || (end.lnum == start.lnum && end.col < start.col) {
                    (*rex.reg_mmatch).endpos[0] = start;
                }
            } else {
                if (*rex.reg_match).endp[0] < (*rex.reg_match).startp[0] {
                    (*rex.reg_match).endp[0] = (*rex.reg_match).startp[0];
                }
                (*rex.reg_match).rm_matchcol = col;
            }
        }
        retval
    }
}

pub(super) unsafe extern "C" fn nfa_regcomp(expr: *mut u8, re_flags: c_int) -> *mut regprog_T {
    unsafe {
        let mut prog: *mut nfa_regprog_T = ptr::null_mut();

        if expr.is_null() {
            return ptr::null_mut();
        }

        nfa_re_flags = re_flags;
        init_class_tab();
        nfa_regcomp_start(expr, re_flags);

        let postfix = re2post();
        if postfix.is_null() {
            return cleanup_nfa_regcomp(prog);
        }

        post2nfa(postfix, post_ptr, true);

        let prog_size =
            offset_of!(nfa_regprog_T, state) + size_of::<nfa_state_T>() * (nstate as usize);
        prog = xmalloc(prog_size).cast::<nfa_regprog_T>();
        state_ptr = (&raw mut (*prog).state).cast::<nfa_state_T>();
        (*prog).re_in_use = false;
        (*prog).listbuf[0] = ptr::null_mut();
        (*prog).listbuf[1] = ptr::null_mut();

        (*prog).start = post2nfa(postfix, post_ptr, false);
        if (*prog).start.is_null() {
            xfree(prog.cast());
            prog = ptr::null_mut();
            return cleanup_nfa_regcomp(prog);
        }
        (*prog).regflags = regflags;
        (*prog).engine = nfa_regengine_ptr();
        (*prog).nstate = nstate;
        (*prog).has_zend = rex.nfa_has_zend;
        (*prog).has_backref = rex.nfa_has_backref;
        (*prog).nsubexp = regnpar;

        nfa_postprocess(prog);

        (*prog).reganch = nfa_get_reganch((*prog).start, 0);
        (*prog).regstart = nfa_get_regstart((*prog).start, 0);
        (*prog).match_text = nfa_get_match_text((*prog).start);
        (*prog).reghasz = re_has_z;
        (*prog).pattern = xstrdup(expr.cast());

        cleanup_nfa_regcomp(prog)
    }
}

unsafe fn cleanup_nfa_regcomp(prog: *mut nfa_regprog_T) -> *mut regprog_T {
    unsafe {
        if post_start_len > NFA_POSTFIX_KEEP {
            xfree(post_start.cast());
            post_start = ptr::null_mut();
            post_start_len = 0;
        }
        post_ptr = ptr::null_mut();
        post_end = ptr::null_mut();
        if nfa_stack_len > NFA_STACK_KEEP {
            xfree(nfa_stack.cast());
            nfa_stack = ptr::null_mut();
            nfa_stack_len = 0;
        }
        state_ptr = ptr::null_mut();
        prog.cast()
    }
}

pub(super) unsafe extern "C" fn nfa_regfree(prog: *mut regprog_T) {
    unsafe {
        if prog.is_null() {
            return;
        }
        let nfa_prog = prog.cast::<nfa_regprog_T>();
        xfree((*nfa_prog).match_text.cast());
        xfree((*nfa_prog).pattern.cast());
        xfree((*nfa_prog).listbuf[0]);
        xfree((*nfa_prog).listbuf[1]);
        xfree(prog.cast());
    }
}

pub(super) unsafe extern "C" fn nfa_regexec_nl(
    rmp: *mut regmatch_T,
    line: *mut u8,
    col: colnr_T,
    line_lbr: bool,
) -> c_int {
    unsafe {
        rex.reg_match = rmp;
        rex.reg_mmatch = ptr::null_mut();
        rex.reg_maxline = 0;
        rex.reg_line_lbr = line_lbr;
        rex.reg_buf = curbuf;
        rex.reg_win = ptr::null_mut();
        rex.reg_ic = (*rmp).rm_ic;
        rex.reg_icombine = false;
        rex.reg_nobreak = ((*(*rmp).regprog).re_flags & (RE_NOBREAK as c_uint)) != 0;
        rex.reg_maxcol = 0;
        nfa_regexec_both(line, col, ptr::null_mut(), ptr::null_mut())
    }
}

pub(super) unsafe extern "C" fn nfa_regexec_multi(
    rmp: *mut regmmatch_T,
    win: *mut win_T,
    buf: *mut buf_T,
    lnum: linenr_T,
    col: colnr_T,
    tm: *mut proftime_T,
    timed_out: *mut c_int,
) -> c_int {
    unsafe {
        init_regexec_multi(rmp, win, buf, lnum);
        nfa_regexec_both(ptr::null_mut(), col, tm, timed_out)
    }
}
