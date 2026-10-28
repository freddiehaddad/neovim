//! Backtracking regular expression engine (`src/nvim/regexp.c`).

use crate::message::semsg;
use std::ffi::c_char;
use std::mem::size_of;
use std::ptr;

use super::*;

const END: i32 = 0;
const BOL: i32 = 1;
const EOL: i32 = 2;
const BRANCH: i32 = 3;
const BACK: i32 = 4;
const EXACTLY: i32 = 5;
const NOTHING: i32 = 6;
const STAR: i32 = 7;
const PLUS: i32 = 8;
const MATCH: i32 = 9;
const NOMATCH: i32 = 10;
const BEHIND: i32 = 11;
const NOBEHIND: i32 = 12;
const SUBPAT: i32 = 13;
const BRACE_SIMPLE: i32 = 14;
const BOW: i32 = 15;
const EOW: i32 = 16;
const BRACE_LIMITS: i32 = 17;
const NEWL: i32 = 18;
const BHPOS: i32 = 19;

const ADD_NL: i32 = 30;
const ANY: i32 = 20;
const ANYOF: i32 = 21;
const ANYBUT: i32 = 22;
const IDENT: i32 = 23;
const SIDENT: i32 = 24;
const KWORD: i32 = 25;
const SKWORD: i32 = 26;
const FNAME: i32 = 27;
const SFNAME: i32 = 28;
const PRINT: i32 = 29;
const SPRINT: i32 = 30;
const WHITE: i32 = 31;
const NWHITE: i32 = 32;
const DIGIT: i32 = 33;
const NDIGIT: i32 = 34;
const HEX: i32 = 35;
const NHEX: i32 = 36;
const OCTAL: i32 = 37;
const NOCTAL: i32 = 38;
const WORD: i32 = 39;
const NWORD: i32 = 40;
const HEAD: i32 = 41;
const NHEAD: i32 = 42;
const ALPHA: i32 = 43;
const NALPHA: i32 = 44;
const LOWER: i32 = 45;
const NLOWER: i32 = 46;
const UPPER: i32 = 47;
const NUPPER: i32 = 48;
const FIRST_NL: i32 = ANY + ADD_NL;
const LAST_NL: i32 = NUPPER + ADD_NL;

const MOPEN: i32 = 80;
const MCLOSE: i32 = 90;
const BACKREF: i32 = 100;
const ZOPEN: i32 = 110;
const ZCLOSE: i32 = 120;
const ZREF: i32 = 130;
const BRACE_COMPLEX: i32 = 140;
const NOPEN: i32 = 150;
const NCLOSE: i32 = 151;

const MULTIBYTECODE: i32 = 200;
const RE_BOF: i32 = 201;
const RE_EOF: i32 = 202;
const CURSOR: i32 = 203;
const RE_LNUM: i32 = 204;
const RE_COL: i32 = 205;
const RE_VCOL: i32 = 206;
const RE_MARK: i32 = 207;
const RE_VISUAL: i32 = 208;
const RE_COMPOSING: i32 = 209;

const HASWIDTH: i32 = 0x1;
const SIMPLE: i32 = 0x2;
const SPSTART: i32 = 0x4;
const HASNL: i32 = 0x8;
const HASLOOKBH: i32 = 0x10;
const WORST: i32 = 0;

const REGSTACK_INITIAL: i32 = 2048;
const BACKPOS_INITIAL: i32 = 64;

static CLASSCODES: [i32; 27] = [
    ANY, IDENT, SIDENT, KWORD, SKWORD, FNAME, SFNAME, PRINT, SPRINT, WHITE, NWHITE, DIGIT, NDIGIT,
    HEX, NHEX, OCTAL, NOCTAL, WORD, NWORD, HEAD, NHEAD, ALPHA, NALPHA, LOWER, NLOWER, UPPER,
    NUPPER,
];

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Regstar {
    nextb: i32,
    nextb_ic: i32,
    count: i64,
    minval: i64,
    maxval: i64,
}

impl Regstar {
    const fn new() -> Self {
        Self {
            nextb: 0,
            nextb_ic: 0,
            count: 0,
            minval: 0,
            maxval: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Backpos {
    bp_scan: *mut u8,
    bp_pos: Regsave,
}

struct RegexpBtState {
    bl_minval: i64,
    bl_maxval: i64,
}

impl RegexpBtState {
    const fn new() -> Self {
        Self {
            bl_minval: 0,
            bl_maxval: 0,
        }
    }
}

static BT_STATE: GlobalState<RegexpBtState> = GlobalState::new(RegexpBtState::new());

#[inline]
fn bt_st() -> &'static mut RegexpBtState {
    unsafe { &mut *BT_STATE.as_ptr() }
}

#[inline]
fn with_nl(op: i32) -> bool {
    (FIRST_NL..=LAST_NL).contains(&op)
}

#[inline]
fn just_calc_size() -> *mut u8 {
    ptr::without_provenance_mut(usize::MAX)
}

#[inline]
fn op_fn(p: *const u8) -> i32 {
    unsafe { *p as i32 }
}

#[inline]
fn next_fn(p: *const u8) -> i32 {
    ((i32::from(crate::memory::cstr_byte_at(p.cast(), 1)) & 0o377) << 8)
        + (i32::from(crate::memory::cstr_byte_at(p.cast(), 2)) & 0o377)
}

#[inline]
fn operand_fn(p: *mut u8) -> *mut u8 {
    p.wrapping_add(3)
}

#[inline]
fn operand_min(p: *const u8) -> i64 {
    unsafe {
        ((*p.wrapping_add(3) as i64) << 24)
            + ((*p.wrapping_add(4) as i64) << 16)
            + ((*p.wrapping_add(5) as i64) << 8)
            + (*p.wrapping_add(6) as i64)
    }
}

#[inline]
fn operand_max(p: *const u8) -> i64 {
    operand_min(p.wrapping_add(4))
}

#[inline]
fn operand_cmp(p: *const u8) -> u8 {
    unsafe { *p.wrapping_add(7) }
}

#[inline]
fn advance_reginput() {
    mb_ptr_adv(&raw mut re_st().rex.input);
}

fn bt_recognize_char_class(start: *const u8, end: *const u8) -> i32 {
    let mut newl = false;
    let config = get_char_class_bits(start, end, &mut newl);
    if config < 0 || newl {
        return 0;
    }
    if config == CLASS_O9 {
        DIGIT
    } else if config == (CLASS_NOT | CLASS_O9) {
        NDIGIT
    } else if config == (CLASS_AF_CONST | CLASS_AF | CLASS_O9) {
        HEX
    } else if config == (CLASS_NOT | CLASS_AF_CONST | CLASS_AF | CLASS_O9) {
        NHEX
    } else if config == CLASS_O7 {
        OCTAL
    } else if config == (CLASS_NOT | CLASS_O7) {
        NOCTAL
    } else if config == (CLASS_AZ_CONST | CLASS_AZ | CLASS_O9 | CLASS_UNDERSCORE) {
        WORD
    } else if config == (CLASS_NOT | CLASS_AZ_CONST | CLASS_AZ | CLASS_O9 | CLASS_UNDERSCORE) {
        NWORD
    } else if config == (CLASS_AZ_CONST | CLASS_AZ | CLASS_UNDERSCORE) {
        HEAD
    } else if config == (CLASS_NOT | CLASS_AZ_CONST | CLASS_AZ | CLASS_UNDERSCORE) {
        NHEAD
    } else if config == (CLASS_AZ_CONST | CLASS_AZ) {
        ALPHA
    } else if config == (CLASS_NOT | CLASS_AZ_CONST | CLASS_AZ) {
        NALPHA
    } else {
        0
    }
}

fn use_multibytecode(c: i32) -> bool {
    utf_char2len(c) > 1 && (re_multi_type(peekchr()) != NOT_MULTI || utf_iscomposing_legacy(c))
}

fn regc(b: i32) {
    if re_st().regcode == just_calc_size() {
        re_st().regsize += 1;
    } else {
        unsafe {
            *re_st().regcode = b as u8;
        }
        re_st().regcode = re_st().regcode.wrapping_add(1);
    }
}

fn regmbc(c: i32) {
    if re_st().regcode == just_calc_size() {
        re_st().regsize += i64::from(utf_char2len(c));
    } else {
        re_st().regcode = re_st()
            .regcode
            .wrapping_add(utf_char2bytes(c, re_st().regcode.cast()) as usize);
    }
}

#[rustfmt::skip]
fn reg_equi_class(c: i32) {
    match c {
            0x41 | 0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc4 | 0xc5 | 0x100 | 0x102 | 0x104 | 0x1cd
            | 0x1de | 0x1e0 | 0x1fa | 0x202 | 0x226 | 0x23a | 0x1e00 | 0x1ea0 | 0x1ea2
            | 0x1ea4 | 0x1ea6 | 0x1ea8 | 0x1eaa | 0x1eac | 0x1eae | 0x1eb0 | 0x1eb2 | 0x1eb4
            | 0x1eb6 => {
                regmbc(b'A' as i32); regmbc(0xc0); regmbc(0xc1); regmbc(0xc2);
                regmbc(0xc3); regmbc(0xc4); regmbc(0xc5);
                regmbc(0x100); regmbc(0x102); regmbc(0x104);
                regmbc(0x1cd); regmbc(0x1de); regmbc(0x1e0);
                regmbc(0x1fa); regmbc(0x202); regmbc(0x226);
                regmbc(0x23a); regmbc(0x1e00); regmbc(0x1ea0);
                regmbc(0x1ea2); regmbc(0x1ea4); regmbc(0x1ea6);
                regmbc(0x1ea8); regmbc(0x1eaa); regmbc(0x1eac);
                regmbc(0x1eae); regmbc(0x1eb0); regmbc(0x1eb2);
                regmbc(0x1eb4); regmbc(0x1eb6);
                return;
            }
            0x42 | 0x181 | 0x243 | 0x1e02 | 0x1e04 | 0x1e06 => {
                regmbc(b'B' as i32);
                regmbc(0x181); regmbc(0x243); regmbc(0x1e02);
                regmbc(0x1e04); regmbc(0x1e06);
                return;
            }
            0x43 | 0xc7 | 0x106 | 0x108 | 0x10a | 0x10c | 0x187 | 0x23b | 0x1e08 | 0xa792 => {
                regmbc(b'C' as i32); regmbc(0xc7);
                regmbc(0x106); regmbc(0x108); regmbc(0x10a);
                regmbc(0x10c); regmbc(0x187); regmbc(0x23b);
                regmbc(0x1e08); regmbc(0xa792);
                return;
            }
            0x44 | 0x10e | 0x110 | 0x18a | 0x1e0a | 0x1e0c | 0x1e0e | 0x1e10 | 0x1e12 => {
                regmbc(b'D' as i32); regmbc(0x10e); regmbc(0x110);
                regmbc(0x18a); regmbc(0x1e0a); regmbc(0x1e0c);
                regmbc(0x1e0e); regmbc(0x1e10); regmbc(0x1e12);
                return;
            }
            0x45 | 0xc8 | 0xc9 | 0xca | 0xcb | 0x112 | 0x114 | 0x116 | 0x118 | 0x11a | 0x204
            | 0x206 | 0x228 | 0x246 | 0x1e14 | 0x1e16 | 0x1e18 | 0x1e1a | 0x1e1c | 0x1eb8
            | 0x1eba | 0x1ebc | 0x1ebe | 0x1ec0 | 0x1ec2 | 0x1ec4 | 0x1ec6 => {
                regmbc(b'E' as i32); regmbc(0xc8); regmbc(0xc9);
                regmbc(0xca); regmbc(0xcb); regmbc(0x112);
                regmbc(0x114); regmbc(0x116); regmbc(0x118);
                regmbc(0x11a); regmbc(0x204); regmbc(0x206);
                regmbc(0x228); regmbc(0x246); regmbc(0x1e14);
                regmbc(0x1e16); regmbc(0x1e18); regmbc(0x1e1a);
                regmbc(0x1e1c); regmbc(0x1eb8); regmbc(0x1eba);
                regmbc(0x1ebc); regmbc(0x1ebe); regmbc(0x1ec0);
                regmbc(0x1ec2); regmbc(0x1ec4); regmbc(0x1ec6);
                return;
            }
            0x46 | 0x191 | 0x1e1e | 0xa798 => {
                regmbc(b'F' as i32); regmbc(0x191); regmbc(0x1e1e);
                regmbc(0xa798);
                return;
            }
            0x47 | 0x11c | 0x11e | 0x120 | 0x122 | 0x193 | 0x1e4 | 0x1e6 | 0x1f4 | 0x1e20
            | 0xa7a0 => {
                regmbc(b'G' as i32); regmbc(0x11c); regmbc(0x11e);
                regmbc(0x120); regmbc(0x122); regmbc(0x193);
                regmbc(0x1e4); regmbc(0x1e6); regmbc(0x1f4);
                regmbc(0x1e20); regmbc(0xa7a0);
                return;
            }
            0x48 | 0x124 | 0x126 | 0x21e | 0x1e22 | 0x1e24 | 0x1e26 | 0x1e28 | 0x1e2a | 0x2c67 => {
                regmbc(b'H' as i32); regmbc(0x124); regmbc(0x126);
                regmbc(0x21e); regmbc(0x1e22); regmbc(0x1e24);
                regmbc(0x1e26); regmbc(0x1e28); regmbc(0x1e2a);
                regmbc(0x2c67);
                return;
            }
            0x49 | 0xcc | 0xcd | 0xce | 0xcf | 0x128 | 0x12a | 0x12c | 0x12e | 0x130 | 0x197
            | 0x1cf | 0x208 | 0x20a | 0x1e2c | 0x1e2e | 0x1ec8 | 0x1eca => {
                regmbc(b'I' as i32); regmbc(0xcc); regmbc(0xcd);
                regmbc(0xce); regmbc(0xcf); regmbc(0x128);
                regmbc(0x12a); regmbc(0x12c); regmbc(0x12e);
                regmbc(0x130); regmbc(0x197); regmbc(0x1cf);
                regmbc(0x208); regmbc(0x20a); regmbc(0x1e2c);
                regmbc(0x1e2e); regmbc(0x1ec8); regmbc(0x1eca);
                return;
            }
            0x4a | 0x134 | 0x248 => {
                regmbc(b'J' as i32); regmbc(0x134); regmbc(0x248);
                return;
            }
            0x4b | 0x136 | 0x198 | 0x1e8 | 0x1e30 | 0x1e32 | 0x1e34 | 0x2c69 | 0xa740 => {
                regmbc(b'K' as i32); regmbc(0x136); regmbc(0x198);
                regmbc(0x1e8); regmbc(0x1e30); regmbc(0x1e32);
                regmbc(0x1e34); regmbc(0x2c69); regmbc(0xa740);
                return;
            }
            0x4c | 0x139 | 0x13b | 0x13d | 0x13f | 0x141 | 0x23d | 0x1e36 | 0x1e38 | 0x1e3a
            | 0x1e3c | 0x2c60 => {
                regmbc(b'L' as i32); regmbc(0x139); regmbc(0x13b);
                regmbc(0x13d); regmbc(0x13f); regmbc(0x141);
                regmbc(0x23d); regmbc(0x1e36); regmbc(0x1e38);
                regmbc(0x1e3a); regmbc(0x1e3c); regmbc(0x2c60);
                return;
            }
            0x4d | 0x1e3e | 0x1e40 | 0x1e42 => {
                regmbc(b'M' as i32); regmbc(0x1e3e); regmbc(0x1e40);
                regmbc(0x1e42);
                return;
            }
            0x4e | 0xd1 | 0x143 | 0x145 | 0x147 | 0x1f8 | 0x1e44 | 0x1e46 | 0x1e48 | 0x1e4a
            | 0xa7a4 => {
                regmbc(b'N' as i32); regmbc(0xd1);
                regmbc(0x143); regmbc(0x145); regmbc(0x147);
                regmbc(0x1f8); regmbc(0x1e44); regmbc(0x1e46);
                regmbc(0x1e48); regmbc(0x1e4a); regmbc(0xa7a4);
                return;
            }
            0x4f | 0xd2 | 0xd3 | 0xd4 | 0xd5 | 0xd6 | 0xd8 | 0x14c | 0x14e | 0x150 | 0x19f
            | 0x1a0 | 0x1d1 | 0x1ea | 0x1ec | 0x1fe | 0x20c | 0x20e | 0x22a | 0x22c | 0x22e
            | 0x230 | 0x1e4c | 0x1e4e | 0x1e50 | 0x1e52 | 0x1ecc | 0x1ece | 0x1ed0 | 0x1ed2
            | 0x1ed4 | 0x1ed6 | 0x1ed8 | 0x1eda | 0x1edc | 0x1ede | 0x1ee0 | 0x1ee2 => {
                regmbc(b'O' as i32); regmbc(0xd2); regmbc(0xd3); regmbc(0xd4);
                regmbc(0xd5); regmbc(0xd6); regmbc(0xd8);
                regmbc(0x14c); regmbc(0x14e); regmbc(0x150);
                regmbc(0x19f); regmbc(0x1a0); regmbc(0x1d1);
                regmbc(0x1ea); regmbc(0x1ec); regmbc(0x1fe);
                regmbc(0x20c); regmbc(0x20e); regmbc(0x22a);
                regmbc(0x22c); regmbc(0x22e); regmbc(0x230);
                regmbc(0x1e4c); regmbc(0x1e4e); regmbc(0x1e50);
                regmbc(0x1e52); regmbc(0x1ecc); regmbc(0x1ece);
                regmbc(0x1ed0); regmbc(0x1ed2); regmbc(0x1ed4);
                regmbc(0x1ed6); regmbc(0x1ed8); regmbc(0x1eda);
                regmbc(0x1edc); regmbc(0x1ede); regmbc(0x1ee0);
                regmbc(0x1ee2);
                return;
            }
            0x50 | 0x1a4 | 0x1e54 | 0x1e56 | 0x2c63 => {
                regmbc(b'P' as i32); regmbc(0x1a4); regmbc(0x1e54);
                regmbc(0x1e56); regmbc(0x2c63);
                return;
            }
            0x51 | 0x24a => {
                regmbc(b'Q' as i32); regmbc(0x24a);
                return;
            }
            0x52 | 0x154 | 0x156 | 0x158 | 0x210 | 0x212 | 0x24c | 0x1e58 | 0x1e5a | 0x1e5c
            | 0x1e5e | 0x2c64 | 0xa7a6 => {
                regmbc(b'R' as i32); regmbc(0x154); regmbc(0x156);
                regmbc(0x210); regmbc(0x212); regmbc(0x158);
                regmbc(0x24c); regmbc(0x1e58); regmbc(0x1e5a);
                regmbc(0x1e5c); regmbc(0x1e5e); regmbc(0x2c64);
                regmbc(0xa7a6);
                return;
            }
            0x53 | 0x15a | 0x15c | 0x15e | 0x160 | 0x218 | 0x1e60 | 0x1e62 | 0x1e64 | 0x1e66
            | 0x1e68 | 0x2c7e | 0xa7a8 => {
                regmbc(b'S' as i32); regmbc(0x15a); regmbc(0x15c);
                regmbc(0x15e); regmbc(0x160); regmbc(0x218);
                regmbc(0x1e60); regmbc(0x1e62); regmbc(0x1e64);
                regmbc(0x1e66); regmbc(0x1e68); regmbc(0x2c7e);
                regmbc(0xa7a8);
                return;
            }
            0x54 | 0x162 | 0x164 | 0x166 | 0x1ac | 0x1ae | 0x21a | 0x23e | 0x1e6a | 0x1e6c
            | 0x1e6e | 0x1e70 => {
                regmbc(b'T' as i32); regmbc(0x162); regmbc(0x164);
                regmbc(0x166); regmbc(0x1ac); regmbc(0x23e);
                regmbc(0x1ae); regmbc(0x21a); regmbc(0x1e6a);
                regmbc(0x1e6c); regmbc(0x1e6e); regmbc(0x1e70);
                return;
            }
            0x55 | 0xd9 | 0xda | 0xdb | 0xdc | 0x168 | 0x16a | 0x16c | 0x16e | 0x170 | 0x172
            | 0x1af | 0x1d3 | 0x1d5 | 0x1d7 | 0x1d9 | 0x1db | 0x214 | 0x216 | 0x244 | 0x1e72
            | 0x1e74 | 0x1e76 | 0x1e78 | 0x1e7a | 0x1ee4 | 0x1ee6 | 0x1ee8 | 0x1eea | 0x1eec
            | 0x1eee | 0x1ef0 => {
                regmbc(b'U' as i32); regmbc(0xd9); regmbc(0xda);
                regmbc(0xdb); regmbc(0xdc); regmbc(0x168);
                regmbc(0x16a); regmbc(0x16c); regmbc(0x16e);
                regmbc(0x170); regmbc(0x172); regmbc(0x1af);
                regmbc(0x1d3); regmbc(0x1d5); regmbc(0x1d7);
                regmbc(0x1d9); regmbc(0x1db); regmbc(0x214);
                regmbc(0x216); regmbc(0x244); regmbc(0x1e72);
                regmbc(0x1e74); regmbc(0x1e76); regmbc(0x1e78);
                regmbc(0x1e7a); regmbc(0x1ee4); regmbc(0x1ee6);
                regmbc(0x1ee8); regmbc(0x1eea); regmbc(0x1eec);
                regmbc(0x1eee); regmbc(0x1ef0);
                return;
            }
            0x56 | 0x1b2 | 0x1e7c | 0x1e7e => {
                regmbc(b'V' as i32); regmbc(0x1b2); regmbc(0x1e7c);
                regmbc(0x1e7e);
                return;
            }
            0x57 | 0x174 | 0x1e80 | 0x1e82 | 0x1e84 | 0x1e86 | 0x1e88 => {
                regmbc(b'W' as i32); regmbc(0x174); regmbc(0x1e80);
                regmbc(0x1e82); regmbc(0x1e84); regmbc(0x1e86);
                regmbc(0x1e88);
                return;
            }
            0x58 | 0x1e8a | 0x1e8c => {
                regmbc(b'X' as i32); regmbc(0x1e8a); regmbc(0x1e8c);
                return;
            }
            0x59 | 0xdd | 0x176 | 0x178 | 0x1b3 | 0x232 | 0x24e | 0x1e8e | 0x1ef2 | 0x1ef4
            | 0x1ef6 | 0x1ef8 => {
                regmbc(b'Y' as i32); regmbc(0xdd); regmbc(0x176);
                regmbc(0x178); regmbc(0x1b3); regmbc(0x232);
                regmbc(0x24e); regmbc(0x1e8e); regmbc(0x1ef2);
                regmbc(0x1ef4); regmbc(0x1ef6); regmbc(0x1ef8);
                return;
            }
            0x5a | 0x179 | 0x17b | 0x17d | 0x1b5 | 0x1e90 | 0x1e92 | 0x1e94 | 0x2c6b => {
                regmbc(b'Z' as i32); regmbc(0x179); regmbc(0x17b);
                regmbc(0x17d); regmbc(0x1b5); regmbc(0x1e90);
                regmbc(0x1e92); regmbc(0x1e94); regmbc(0x2c6b);
                return;
            }
            0x61 | 0xe0 | 0xe1 | 0xe2 | 0xe3 | 0xe4 | 0xe5 | 0x101 | 0x103 | 0x105 | 0x1ce
            | 0x1df | 0x1e1 | 0x1fb | 0x201 | 0x203 | 0x227 | 0x1d8f | 0x1e01 | 0x1e9a
            | 0x1ea1 | 0x1ea3 | 0x1ea5 | 0x1ea7 | 0x1ea9 | 0x1eab | 0x1ead | 0x1eaf | 0x1eb1
            | 0x1eb3 | 0x1eb5 | 0x1eb7 | 0x2c65 => {
                regmbc(b'a' as i32); regmbc(0xe0); regmbc(0xe1);
                regmbc(0xe2); regmbc(0xe3); regmbc(0xe4);
                regmbc(0xe5); regmbc(0x101); regmbc(0x103);
                regmbc(0x105); regmbc(0x1ce); regmbc(0x1df);
                regmbc(0x1e1); regmbc(0x1fb); regmbc(0x201);
                regmbc(0x203); regmbc(0x227); regmbc(0x1d8f);
                regmbc(0x1e01); regmbc(0x1e9a); regmbc(0x1ea1);
                regmbc(0x1ea3); regmbc(0x1ea5); regmbc(0x1ea7);
                regmbc(0x1ea9); regmbc(0x1eab); regmbc(0x1ead);
                regmbc(0x1eaf); regmbc(0x1eb1); regmbc(0x1eb3);
                regmbc(0x1eb5); regmbc(0x1eb7); regmbc(0x2c65);
                return;
            }
            0x62 | 0x180 | 0x253 | 0x1d6c | 0x1d80 | 0x1e03 | 0x1e05 | 0x1e07 => {
                regmbc(b'b' as i32);
                regmbc(0x180); regmbc(0x253); regmbc(0x1d6c);
                regmbc(0x1d80); regmbc(0x1e03); regmbc(0x1e05);
                regmbc(0x1e07);
                return;
            }
            0x63 | 0xe7 | 0x107 | 0x109 | 0x10b | 0x10d | 0x188 | 0x23c | 0x1e09 | 0xa793
            | 0xa794 => {
                regmbc(b'c' as i32); regmbc(0xe7); regmbc(0x107);
                regmbc(0x109); regmbc(0x10b); regmbc(0x10d);
                regmbc(0x188); regmbc(0x23c); regmbc(0x1e09);
                regmbc(0xa793); regmbc(0xa794);
                return;
            }
            0x64 | 0x10f | 0x111 | 0x257 | 0x1d6d | 0x1d81 | 0x1d91 | 0x1e0b | 0x1e0d | 0x1e0f
            | 0x1e11 | 0x1e13 => {
                regmbc(b'd' as i32); regmbc(0x10f); regmbc(0x111);
                regmbc(0x257); regmbc(0x1d6d); regmbc(0x1d81);
                regmbc(0x1d91); regmbc(0x1e0b); regmbc(0x1e0d);
                regmbc(0x1e0f); regmbc(0x1e11); regmbc(0x1e13);
                return;
            }
            0x65 | 0xe8 | 0xe9 | 0xea | 0xeb | 0x113 | 0x115 | 0x117 | 0x119 | 0x11b | 0x205
            | 0x207 | 0x229 | 0x247 | 0x1d92 | 0x1e15 | 0x1e17 | 0x1e19 | 0x1e1b | 0x1e1d
            | 0x1eb9 | 0x1ebb | 0x1ebd | 0x1ebf | 0x1ec1 | 0x1ec3 | 0x1ec5 | 0x1ec7 => {
                regmbc(b'e' as i32); regmbc(0xe8); regmbc(0xe9);
                regmbc(0xea); regmbc(0xeb); regmbc(0x113);
                regmbc(0x115); regmbc(0x117); regmbc(0x119);
                regmbc(0x11b); regmbc(0x205); regmbc(0x207);
                regmbc(0x229); regmbc(0x247); regmbc(0x1d92);
                regmbc(0x1e15); regmbc(0x1e17); regmbc(0x1e19);
                regmbc(0x1e1b); regmbc(0x1e1d); regmbc(0x1eb9);
                regmbc(0x1ebb); regmbc(0x1ebd); regmbc(0x1ebf);
                regmbc(0x1ec1); regmbc(0x1ec3); regmbc(0x1ec5);
                regmbc(0x1ec7);
                return;
            }
            0x66 | 0x192 | 0x1d6e | 0x1d82 | 0x1e1f | 0xa799 => {
                regmbc(b'f' as i32); regmbc(0x192); regmbc(0x1d6e);
                regmbc(0x1d82); regmbc(0x1e1f); regmbc(0xa799);
                return;
            }
            0x67 | 0x11d | 0x11f | 0x121 | 0x123 | 0x1e5 | 0x1e7 | 0x1f5 | 0x260 | 0x1d83
            | 0x1e21 | 0xa7a1 => {
                regmbc(b'g' as i32); regmbc(0x11d); regmbc(0x11f);
                regmbc(0x121); regmbc(0x123); regmbc(0x1e5);
                regmbc(0x1e7); regmbc(0x1f5); regmbc(0x260);
                regmbc(0x1d83); regmbc(0x1e21); regmbc(0xa7a1);
                return;
            }
            0x68 | 0x125 | 0x127 | 0x21f | 0x1e23 | 0x1e25 | 0x1e27 | 0x1e29 | 0x1e2b | 0x1e96
            | 0x2c68 | 0xa795 => {
                regmbc(b'h' as i32); regmbc(0x125); regmbc(0x127);
                regmbc(0x21f); regmbc(0x1e23); regmbc(0x1e25);
                regmbc(0x1e27); regmbc(0x1e29); regmbc(0x1e2b);
                regmbc(0x1e96); regmbc(0x2c68); regmbc(0xa795);
                return;
            }
            0x69 | 0xec | 0xed | 0xee | 0xef | 0x129 | 0x12b | 0x12d | 0x12f | 0x1d0 | 0x209
            | 0x20b | 0x268 | 0x1d96 | 0x1e2d | 0x1e2f | 0x1ec9 | 0x1ecb => {
                regmbc(b'i' as i32); regmbc(0xec); regmbc(0xed);
                regmbc(0xee); regmbc(0xef); regmbc(0x129);
                regmbc(0x12b); regmbc(0x12d); regmbc(0x12f);
                regmbc(0x1d0); regmbc(0x209); regmbc(0x20b);
                regmbc(0x268); regmbc(0x1d96); regmbc(0x1e2d);
                regmbc(0x1e2f); regmbc(0x1ec9); regmbc(0x1ecb);
                return;
            }
            0x6a | 0x135 | 0x1f0 | 0x249 => {
                regmbc(b'j' as i32); regmbc(0x135); regmbc(0x1f0);
                regmbc(0x249);
                return;
            }
            0x6b | 0x137 | 0x199 | 0x1e9 | 0x1d84 | 0x1e31 | 0x1e33 | 0x1e35 | 0x2c6a | 0xa741 => {
                regmbc(b'k' as i32); regmbc(0x137); regmbc(0x199);
                regmbc(0x1e9); regmbc(0x1d84); regmbc(0x1e31);
                regmbc(0x1e33); regmbc(0x1e35); regmbc(0x2c6a);
                regmbc(0xa741);
                return;
            }
            0x6c | 0x13a | 0x13c | 0x13e | 0x140 | 0x142 | 0x19a | 0x1e37 | 0x1e39 | 0x1e3b
            | 0x1e3d | 0x2c61 => {
                regmbc(b'l' as i32); regmbc(0x13a); regmbc(0x13c);
                regmbc(0x13e); regmbc(0x140); regmbc(0x142);
                regmbc(0x19a); regmbc(0x1e37); regmbc(0x1e39);
                regmbc(0x1e3b); regmbc(0x1e3d); regmbc(0x2c61);
                return;
            }
            0x6d | 0x1d6f | 0x1e3f | 0x1e41 | 0x1e43 => {
                regmbc(b'm' as i32); regmbc(0x1d6f); regmbc(0x1e3f);
                regmbc(0x1e41); regmbc(0x1e43);
                return;
            }
            0x6e | 0xf1 | 0x144 | 0x146 | 0x148 | 0x149 | 0x1f9 | 0x1d70 | 0x1d87 | 0x1e45
            | 0x1e47 | 0x1e49 | 0x1e4b | 0xa7a5 => {
                regmbc(b'n' as i32); regmbc(0xf1); regmbc(0x144);
                regmbc(0x146); regmbc(0x148); regmbc(0x149);
                regmbc(0x1f9); regmbc(0x1d70); regmbc(0x1d87);
                regmbc(0x1e45); regmbc(0x1e47); regmbc(0x1e49);
                regmbc(0x1e4b); regmbc(0xa7a5);
                return;
            }
            0x6f | 0xf2 | 0xf3 | 0xf4 | 0xf5 | 0xf6 | 0xf8 | 0x14d | 0x14f | 0x151 | 0x1a1
            | 0x1d2 | 0x1eb | 0x1ed | 0x1ff | 0x20d | 0x20f | 0x22b | 0x22d | 0x22f | 0x231
            | 0x275 | 0x1e4d | 0x1e4f | 0x1e51 | 0x1e53 | 0x1ecd | 0x1ecf | 0x1ed1 | 0x1ed3
            | 0x1ed5 | 0x1ed7 | 0x1ed9 | 0x1edb | 0x1edd | 0x1edf | 0x1ee1 | 0x1ee3 => {
                regmbc(b'o' as i32); regmbc(0xf2); regmbc(0xf3);
                regmbc(0xf4); regmbc(0xf5); regmbc(0xf6);
                regmbc(0xf8); regmbc(0x14d); regmbc(0x14f);
                regmbc(0x151); regmbc(0x1a1); regmbc(0x1d2);
                regmbc(0x1eb); regmbc(0x1ed); regmbc(0x1ff);
                regmbc(0x20d); regmbc(0x20f); regmbc(0x22b);
                regmbc(0x22d); regmbc(0x22f); regmbc(0x231);
                regmbc(0x275); regmbc(0x1e4d); regmbc(0x1e4f);
                regmbc(0x1e51); regmbc(0x1e53); regmbc(0x1ecd);
                regmbc(0x1ecf); regmbc(0x1ed1); regmbc(0x1ed3);
                regmbc(0x1ed5); regmbc(0x1ed7); regmbc(0x1ed9);
                regmbc(0x1edb); regmbc(0x1edd); regmbc(0x1edf);
                regmbc(0x1ee1); regmbc(0x1ee3);
                return;
            }
            0x70 | 0x1a5 | 0x1d71 | 0x1d7d | 0x1d88 | 0x1e55 | 0x1e57 => {
                regmbc(b'p' as i32); regmbc(0x1a5); regmbc(0x1d71);
                regmbc(0x1d7d); regmbc(0x1d88); regmbc(0x1e55);
                regmbc(0x1e57);
                return;
            }
            0x71 | 0x24b | 0x2a0 => {
                regmbc(b'q' as i32); regmbc(0x24b); regmbc(0x2a0);
                return;
            }
            0x72 | 0x155 | 0x157 | 0x159 | 0x211 | 0x213 | 0x24d | 0x27d | 0x1d72 | 0x1d73
            | 0x1d89 | 0x1e59 | 0x1e5b | 0x1e5d | 0x1e5f | 0xa7a7 => {
                regmbc(b'r' as i32); regmbc(0x155); regmbc(0x157);
                regmbc(0x159); regmbc(0x211); regmbc(0x213);
                regmbc(0x24d); regmbc(0x1d72); regmbc(0x1d73);
                regmbc(0x1d89); regmbc(0x1e59); regmbc(0x27d);
                regmbc(0x1e5b); regmbc(0x1e5d); regmbc(0x1e5f);
                regmbc(0xa7a7);
                return;
            }
            0x73 | 0x15b | 0x15d | 0x15f | 0x161 | 0x219 | 0x23f | 0x1d74 | 0x1d8a | 0x1e61
            | 0x1e63 | 0x1e65 | 0x1e67 | 0x1e69 | 0xa7a9 => {
                regmbc(b's' as i32); regmbc(0x15b); regmbc(0x15d);
                regmbc(0x15f); regmbc(0x161); regmbc(0x23f);
                regmbc(0x219); regmbc(0x1d74); regmbc(0x1d8a);
                regmbc(0x1e61); regmbc(0x1e63); regmbc(0x1e65);
                regmbc(0x1e67); regmbc(0x1e69); regmbc(0xa7a9);
                return;
            }
            0x74 | 0x163 | 0x165 | 0x167 | 0x1ab | 0x1ad | 0x21b | 0x288 | 0x1d75 | 0x1e6b
            | 0x1e6d | 0x1e6f | 0x1e71 | 0x1e97 | 0x2c66 => {
                regmbc(b't' as i32); regmbc(0x163); regmbc(0x165);
                regmbc(0x167); regmbc(0x1ab); regmbc(0x21b);
                regmbc(0x1ad); regmbc(0x288); regmbc(0x1d75);
                regmbc(0x1e6b); regmbc(0x1e6d); regmbc(0x1e6f);
                regmbc(0x1e71); regmbc(0x1e97); regmbc(0x2c66);
                return;
            }
            0x75 | 0xf9 | 0xfa | 0xfb | 0xfc | 0x169 | 0x16b | 0x16d | 0x16f | 0x171 | 0x173
            | 0x1b0 | 0x1d4 | 0x1d6 | 0x1d8 | 0x1da | 0x1dc | 0x215 | 0x217 | 0x289 | 0x1d7e
            | 0x1d99 | 0x1e73 | 0x1e75 | 0x1e77 | 0x1e79 | 0x1e7b | 0x1ee5 | 0x1ee7 | 0x1ee9
            | 0x1eeb | 0x1eed | 0x1eef | 0x1ef1 => {
                regmbc(b'u' as i32); regmbc(0xf9); regmbc(0xfa);
                regmbc(0xfb); regmbc(0xfc); regmbc(0x169);
                regmbc(0x16b); regmbc(0x16d); regmbc(0x16f);
                regmbc(0x171); regmbc(0x173); regmbc(0x1d6);
                regmbc(0x1d8); regmbc(0x1da); regmbc(0x1dc);
                regmbc(0x215); regmbc(0x217); regmbc(0x1b0);
                regmbc(0x1d4); regmbc(0x289); regmbc(0x1d7e);
                regmbc(0x1d99); regmbc(0x1e73); regmbc(0x1e75);
                regmbc(0x1e77); regmbc(0x1e79); regmbc(0x1e7b);
                regmbc(0x1ee5); regmbc(0x1ee7); regmbc(0x1ee9);
                regmbc(0x1eeb); regmbc(0x1eed); regmbc(0x1eef);
                regmbc(0x1ef1);
                return;
            }
            0x76 | 0x28b | 0x1d8c | 0x1e7d | 0x1e7f => {
                regmbc(b'v' as i32); regmbc(0x28b); regmbc(0x1d8c);
                regmbc(0x1e7d); regmbc(0x1e7f);
                return;
            }
            0x77 | 0x175 | 0x1e81 | 0x1e83 | 0x1e85 | 0x1e87 | 0x1e89 | 0x1e98 => {
                regmbc(b'w' as i32); regmbc(0x175); regmbc(0x1e81);
                regmbc(0x1e83); regmbc(0x1e85); regmbc(0x1e87);
                regmbc(0x1e89); regmbc(0x1e98);
                return;
            }
            0x78 | 0x1e8b | 0x1e8d => {
                regmbc(b'x' as i32); regmbc(0x1e8b); regmbc(0x1e8d);
                return;
            }
            0x79 | 0xfd | 0xff | 0x177 | 0x1b4 | 0x233 | 0x24f | 0x1e8f | 0x1e99 | 0x1ef3
            | 0x1ef5 | 0x1ef7 | 0x1ef9 => {
                regmbc(b'y' as i32); regmbc(0xfd); regmbc(0xff);
                regmbc(0x177); regmbc(0x1b4); regmbc(0x233);
                regmbc(0x24f); regmbc(0x1e8f); regmbc(0x1e99);
                regmbc(0x1ef3); regmbc(0x1ef5); regmbc(0x1ef7);
                regmbc(0x1ef9);
                return;
            }
            0x7a | 0x17a | 0x17c | 0x17e | 0x1b6 | 0x1d76 | 0x1d8e | 0x1e91 | 0x1e93 | 0x1e95
            | 0x2c6c => {
                regmbc(b'z' as i32); regmbc(0x17a); regmbc(0x17c);
                regmbc(0x17e); regmbc(0x1b6); regmbc(0x1d76);
                regmbc(0x1d8e); regmbc(0x1e91); regmbc(0x1e93);
                regmbc(0x1e95); regmbc(0x2c6c);
                return;
            }
            _ => {}
        }
        regmbc(c);
}

fn regnode(op: i32) -> *mut u8 {
    unsafe {
        let ret = re_st().regcode;
        if ret == just_calc_size() {
            re_st().regsize += 3;
        } else {
            *re_st().regcode = op as u8;
            *re_st().regcode.wrapping_add(1) = NUL;
            *re_st().regcode.wrapping_add(2) = NUL;
            re_st().regcode = re_st().regcode.wrapping_add(3);
        }
        ret
    }
}

fn re_put_uint32(p: *mut u8, val: u32) -> *mut u8 {
    unsafe {
        *p = ((val >> 24) & 0o377) as u8;
        *p.wrapping_add(1) = ((val >> 16) & 0o377) as u8;
        *p.wrapping_add(2) = ((val >> 8) & 0o377) as u8;
        *p.wrapping_add(3) = (val & 0o377) as u8;
        p.wrapping_add(4)
    }
}

fn regnext(p: *mut u8) -> *mut u8 {
    if p == just_calc_size() || re_st().reg_toolong != 0 {
        return ptr::null_mut();
    }
    let offset = next_fn(p);
    if offset == 0 {
        return ptr::null_mut();
    }
    if op_fn(p) == BACK {
        p.wrapping_sub(offset as usize)
    } else {
        p.wrapping_add(offset as usize)
    }
}

fn regtail(p: *mut u8, val: *const u8) {
    unsafe {
        if p == just_calc_size() {
            return;
        }
        let mut scan = p;
        loop {
            let temp = regnext(scan);
            if temp.is_null() {
                break;
            }
            scan = temp;
        }
        let offset = if op_fn(scan) == BACK {
            scan.offset_from(val) as i32
        } else {
            val.offset_from(scan) as i32
        };
        if offset > 0xffff {
            re_st().reg_toolong = 1;
        } else {
            *scan.wrapping_add(1) = (((offset as u32) >> 8) & 0o377) as u8;
            *scan.wrapping_add(2) = (offset & 0o377) as u8;
        }
    }
}

fn regoptail(p: *mut u8, val: *mut u8) {
    if p.is_null()
        || p == just_calc_size()
        || (op_fn(p) != BRANCH && (op_fn(p) < BRACE_COMPLEX || op_fn(p) > BRACE_COMPLEX + 9))
    {
        return;
    }
    regtail(operand_fn(p), val);
}

fn reginsert(op: i32, opnd: *mut u8) {
    unsafe {
        if re_st().regcode == just_calc_size() {
            re_st().regsize += 3;
            return;
        }
        let mut src = re_st().regcode;
        re_st().regcode = re_st().regcode.wrapping_add(3);
        let mut dst = re_st().regcode;
        while src > opnd {
            src = src.wrapping_sub(1);
            dst = dst.wrapping_sub(1);
            *dst = *src;
        }
        *opnd = op as u8;
        *opnd.wrapping_add(1) = NUL;
        *opnd.wrapping_add(2) = NUL;
    }
}

fn reginsert_nr(op: i32, val: i64, opnd: *mut u8) {
    unsafe {
        if re_st().regcode == just_calc_size() {
            re_st().regsize += 7;
            return;
        }
        let mut src = re_st().regcode;
        re_st().regcode = re_st().regcode.wrapping_add(7);
        let mut dst = re_st().regcode;
        while src > opnd {
            src = src.wrapping_sub(1);
            dst = dst.wrapping_sub(1);
            *dst = *src;
        }
        *opnd = op as u8;
        *opnd.wrapping_add(1) = NUL;
        *opnd.wrapping_add(2) = NUL;
        re_put_uint32(opnd.wrapping_add(3), val as u32);
    }
}

fn reginsert_limits(op: i32, minval: i64, maxval: i64, opnd: *mut u8) {
    unsafe {
        if re_st().regcode == just_calc_size() {
            re_st().regsize += 11;
            return;
        }
        let mut src = re_st().regcode;
        re_st().regcode = re_st().regcode.wrapping_add(11);
        let mut dst = re_st().regcode;
        while src > opnd {
            src = src.wrapping_sub(1);
            dst = dst.wrapping_sub(1);
            *dst = *src;
        }
        *opnd = op as u8;
        *opnd.wrapping_add(1) = NUL;
        *opnd.wrapping_add(2) = NUL;
        let place = re_put_uint32(opnd.wrapping_add(3), minval as u32);
        let place = re_put_uint32(place, maxval as u32);
        regtail(opnd, place);
    }
}

fn regatom(flagp: &mut i32) -> *mut u8 {
    let mut ret: *mut u8;
    let mut flags: i32 = 0;
    let mut extra: i32 = 0;
    let save_prev_at_start = re_st().prev_at_start;

    *flagp = WORST;

    let mut c = getchr();
    let mut goto_collection = false;
    let mut goto_multibyte = false;

    if c == magic_fn(b'^' as i32) {
        return regnode(BOL);
    } else if c == magic_fn(b'$' as i32) {
        re_st().had_eol = 1;
        return regnode(EOL);
    } else if c == magic_fn(b'<' as i32) {
        return regnode(BOW);
    } else if c == magic_fn(b'>' as i32) {
        return regnode(EOW);
    } else if c == magic_fn(b'_' as i32) {
        c = no_magic(getchr());
        if c == b'^' as i32 {
            return regnode(BOL);
        }
        if c == b'$' as i32 {
            re_st().had_eol = 1;
            return regnode(EOL);
        }
        extra = ADD_NL;
        *flagp |= HASNL;
        if c == b'[' as i32 {
            goto_collection = true;
        } else {
            // fallthrough to character classes
            let p = vim_strchr(CLASSCHARS.as_ptr().cast(), no_magic(c)).cast::<u8>();
            if p.is_null() {
                emsg(tr(E_INVALID_USE_OF_UNDERSCORE));
                g_st().rc_did_emsg = true;
                return ptr::null_mut();
            }
            if c == magic_fn(b'.' as i32) && utf_iscomposing_legacy(peekchr()) {
                c = getchr();
                goto_multibyte = true;
            } else {
                let idx = unsafe { p.offset_from(CLASSCHARS.as_ptr()) } as usize;
                ret = regnode(CLASSCODES[idx] + extra);
                *flagp |= HASWIDTH | SIMPLE;
                return ret;
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
                emsg(tr(E_INVALID_USE_OF_UNDERSCORE));
                g_st().rc_did_emsg = true;
                return ptr::null_mut();
            }
            if c == magic_fn(b'.' as i32) && utf_iscomposing_legacy(peekchr()) {
                c = getchr();
                goto_multibyte = true;
            } else {
                let idx = unsafe { p.offset_from(CLASSCHARS.as_ptr()) } as usize;
                ret = regnode(CLASSCODES[idx] + extra);
                *flagp |= HASWIDTH | SIMPLE;
                return ret;
            }
        } else if c == magic_fn(b'n' as i32) {
            if re_st().reg_string != 0 {
                ret = regnode(EXACTLY);
                regc(NL);
                regc(NUL as i32);
                *flagp |= HASWIDTH | SIMPLE;
            } else {
                ret = regnode(NEWL);
                *flagp |= HASWIDTH | HASNL;
            }
            return ret;
        } else if c == magic_fn(b'(' as i32) {
            if re_st().one_exactly {
                semsg!(
                    tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                    if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
                g_st().rc_did_emsg = true;
                return ptr::null_mut();
            }
            ret = reg(REG_PAREN, &mut flags);
            if ret.is_null() {
                return ptr::null_mut();
            }
            *flagp |= flags & (HASWIDTH | SPSTART | HASNL | HASLOOKBH);
            return ret;
        } else if c == NUL as i32
            || c == magic_fn(b'|' as i32)
            || c == magic_fn(b'&' as i32)
            || c == magic_fn(b')' as i32)
        {
            if re_st().one_exactly {
                semsg!(
                    tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                    if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
                g_st().rc_did_emsg = true;
                return ptr::null_mut();
            }
            iemsg(tr(E_INTERNAL_ERROR_IN_REGEXP));
            g_st().rc_did_emsg = true;
            return ptr::null_mut();
        } else if c == magic_fn(b'=' as i32)
            || c == magic_fn(b'?' as i32)
            || c == magic_fn(b'+' as i32)
            || c == magic_fn(b'@' as i32)
            || c == magic_fn(b'{' as i32)
            || c == magic_fn(b'*' as i32)
        {
            c = no_magic(c);
            let magic_cond = if c == b'*' as i32 {
                re_st().reg_magic >= MAGIC_ON
            } else {
                re_st().reg_magic == MAGIC_ALL
            };
            semsg!(
                tr(c"E64: %s%c follows nothing"),
                if magic_cond {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
                c,
            );
            g_st().rc_did_emsg = true;
            return ptr::null_mut();
        } else if c == magic_fn(b'~' as i32) {
            if let Some(prev_sub) = reg_prev_sub_ref() {
                ret = regnode(EXACTLY);
                for &b in prev_sub.to_bytes() {
                    regc(b as i32);
                }
                regc(NUL as i32);
                if !prev_sub.to_bytes().is_empty() {
                    *flagp |= HASWIDTH;
                    if prev_sub.to_bytes().len() == 1 {
                        *flagp |= SIMPLE;
                    }
                }
                return ret;
            } else {
                emsg(tr(E_NOPRESUB));
                g_st().rc_did_emsg = true;
                return ptr::null_mut();
            }
        } else if (magic_fn(b'1' as i32)..=magic_fn(b'9' as i32)).contains(&c) {
            let refnum = c - magic_fn(b'0' as i32);
            if !seen_endbrace(refnum) {
                return ptr::null_mut();
            }
            return regnode(BACKREF + refnum);
        } else if c == magic_fn(b'z' as i32) {
            c = no_magic(getchr());
            match c as u8 {
                b'(' => {
                    if (g_st().reg_do_extmatch & REX_SET) == 0 {
                        emsg(tr(E_Z_NOT_ALLOWED));
                        g_st().rc_did_emsg = true;
                        return ptr::null_mut();
                    }
                    if re_st().one_exactly {
                        semsg!(
                            tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                            if re_st().reg_magic == MAGIC_ALL {
                                c"".as_ptr()
                            } else {
                                c"\\".as_ptr()
                            },
                        );
                        g_st().rc_did_emsg = true;
                        return ptr::null_mut();
                    }
                    ret = reg(REG_ZPAREN, &mut flags);
                    if ret.is_null() {
                        return ptr::null_mut();
                    }
                    *flagp |= flags & (HASWIDTH | SPSTART | HASNL | HASLOOKBH);
                    re_st().re_has_z = REX_SET;
                    return ret;
                }
                b'1'..=b'9' => {
                    if (g_st().reg_do_extmatch & REX_USE) == 0 {
                        emsg(tr(E_Z1_NOT_ALLOWED));
                        g_st().rc_did_emsg = true;
                        return ptr::null_mut();
                    }
                    ret = regnode(ZREF + c - b'0' as i32);
                    re_st().re_has_z = REX_USE;
                    return ret;
                }
                b's' => {
                    ret = regnode(MOPEN);
                    if !re_mult_next(c"\\zs") {
                        return ptr::null_mut();
                    }
                    return ret;
                }
                b'e' => {
                    ret = regnode(MCLOSE);
                    if !re_mult_next(c"\\ze") {
                        return ptr::null_mut();
                    }
                    return ret;
                }
                _ => {
                    emsg(tr(c"E68: Invalid character after \\z"));
                    g_st().rc_did_emsg = true;
                    return ptr::null_mut();
                }
            }
        } else if c == magic_fn(b'%' as i32) {
            c = no_magic(getchr());
            match c as u8 {
                b'(' => {
                    if re_st().one_exactly {
                        semsg!(
                            tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                            if re_st().reg_magic == MAGIC_ALL {
                                c"".as_ptr()
                            } else {
                                c"\\".as_ptr()
                            },
                        );
                        g_st().rc_did_emsg = true;
                        return ptr::null_mut();
                    }
                    ret = reg(REG_NPAREN, &mut flags);
                    if ret.is_null() {
                        return ptr::null_mut();
                    }
                    *flagp |= flags & (HASWIDTH | SPSTART | HASNL | HASLOOKBH);
                    return ret;
                }
                b'^' => return regnode(RE_BOF),
                b'$' => return regnode(RE_EOF),
                b'#' => {
                    if crate::memory::cstr_first_byte(re_st().regparse) == b'='
                        && (crate::memory::cstr_byte_at(re_st().regparse, 1)) >= 48
                        && (crate::memory::cstr_byte_at(re_st().regparse, 1)) <= 50
                    {
                        semsg!(
                            tr(E_ATOM_ENGINE_MUST_BE_AT_START_OF_PATTERN),
                            crate::memory::cstr_char_at(re_st().regparse, 1),
                        );
                        return ptr::null_mut();
                    }
                    return regnode(CURSOR);
                }
                b'V' => return regnode(RE_VISUAL),
                b'C' => return regnode(RE_COMPOSING),
                b'[' => {
                    if re_st().one_exactly {
                        semsg!(
                            tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                            if re_st().reg_magic == MAGIC_ALL {
                                c"".as_ptr()
                            } else {
                                c"\\".as_ptr()
                            },
                        );
                        g_st().rc_did_emsg = true;
                        return ptr::null_mut();
                    }
                    let mut lastnode: *mut u8 = ptr::null_mut();
                    ret = ptr::null_mut();
                    loop {
                        c = getchr();
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
                            return ptr::null_mut();
                        }
                        let br = regnode(BRANCH);
                        if ret.is_null() {
                            ret = br;
                        } else {
                            regtail(lastnode, br);
                            if re_st().reg_toolong != 0 {
                                return ptr::null_mut();
                            }
                        }
                        ungetchr();
                        re_st().one_exactly = true;
                        lastnode = regatom(flagp);
                        re_st().one_exactly = false;
                        if lastnode.is_null() {
                            return ptr::null_mut();
                        }
                    }
                    if ret.is_null() {
                        semsg!(
                            tr(E_EMPTY_SB),
                            if re_st().reg_magic == MAGIC_ALL {
                                c"".as_ptr()
                            } else {
                                c"\\".as_ptr()
                            },
                        );
                        g_st().rc_did_emsg = true;
                        return ptr::null_mut();
                    }
                    let lastbranch = regnode(BRANCH);
                    let mut br = regnode(NOTHING);
                    if ret != just_calc_size() {
                        regtail(lastnode, br);
                        regtail(lastbranch, br);
                        br = ret;
                        while br != lastnode {
                            if op_fn(br) == BRANCH {
                                regtail(br, lastbranch);
                                if re_st().reg_toolong != 0 {
                                    return ptr::null_mut();
                                }
                                br = operand_fn(br);
                            } else {
                                br = regnext(br);
                            }
                        }
                    }
                    *flagp &= !(HASWIDTH | SIMPLE);
                    return ret;
                }
                b'd' | b'o' | b'x' | b'u' | b'U' => {
                    let i = match c as u8 {
                        b'd' => getdecchrs(),
                        b'o' => getoctchrs(),
                        b'x' => gethexchrs(2),
                        b'u' => gethexchrs(4),
                        b'U' => gethexchrs(8),
                        _ => -1,
                    };
                    if i < 0 || i > i64::from(i32::MAX) {
                        semsg!(
                            tr(c"E678: Invalid character after %s%%[dxouU]"),
                            if re_st().reg_magic == MAGIC_ALL {
                                c"".as_ptr()
                            } else {
                                c"\\".as_ptr()
                            },
                        );
                        g_st().rc_did_emsg = true;
                        return ptr::null_mut();
                    }
                    if use_multibytecode(i as i32) {
                        ret = regnode(MULTIBYTECODE);
                    } else {
                        ret = regnode(EXACTLY);
                    }
                    if i == 0 {
                        regc(0x0a);
                    } else {
                        regmbc(i as i32);
                    }
                    regc(NUL as i32);
                    *flagp |= HASWIDTH;
                    return ret;
                }
                _ => {
                    if ascii_isdigit(c)
                        || c == b'<' as i32
                        || c == b'>' as i32
                        || c == b'\'' as i32
                        || c == b'.' as i32
                    {
                        let mut n: u32 = 0;
                        let cmp = c;
                        let mut cur = false;
                        let mut got_digit = false;

                        if cmp == b'<' as i32 || cmp == b'>' as i32 {
                            c = getchr();
                        }
                        if no_magic(c) == b'.' as i32 {
                            cur = true;
                            c = getchr();
                        }
                        while ascii_isdigit(c) {
                            got_digit = true;
                            n = n.wrapping_mul(10).wrapping_add((c - b'0' as i32) as u32);
                            c = getchr();
                        }
                        if no_magic(c) == b'\'' as i32 && n == 0 {
                            c = getchr();
                            ret = regnode(RE_MARK);
                            if ret == just_calc_size() {
                                re_st().regsize += 2;
                            } else {
                                unsafe {
                                    *re_st().regcode = c as u8;
                                }
                                unsafe {
                                    *re_st().regcode.wrapping_add(1) = cmp as u8;
                                }
                                re_st().regcode = re_st().regcode.wrapping_add(2);
                            }
                            return ret;
                        } else if (c == b'l' as i32 || c == b'c' as i32 || c == b'v' as i32)
                            && (cur || got_digit)
                        {
                            if cur && n != 0 {
                                semsg!(tr(E_REGEXP_NUMBER_AFTER_DOT_POS_SEARCH_CHR), no_magic(c),);
                                g_st().rc_did_emsg = true;
                                return ptr::null_mut();
                            }
                            if c == b'l' as i32 {
                                if cur {
                                    n = crate::globals::curwin_mut().w_cursor.lnum as u32;
                                }
                                ret = regnode(RE_LNUM);
                                if save_prev_at_start != 0 {
                                    re_st().at_start = 1;
                                }
                            } else if c == b'c' as i32 {
                                if cur {
                                    n = crate::globals::curwin_mut().w_cursor.col as u32 + 1;
                                }
                                ret = regnode(RE_COL);
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
                                    n = vcol as u32;
                                }
                                ret = regnode(RE_VCOL);
                            }
                            if ret == just_calc_size() {
                                re_st().regsize += 5;
                            } else {
                                re_st().regcode = re_put_uint32(re_st().regcode, n);
                                unsafe {
                                    *re_st().regcode = cmp as u8;
                                }
                                re_st().regcode = re_st().regcode.wrapping_add(1);
                            }
                            return ret;
                        }
                    }
                    semsg!(
                        tr(c"E71: Invalid character after %s%%"),
                        if re_st().reg_magic == MAGIC_ALL {
                            c"".as_ptr()
                        } else {
                            c"\\".as_ptr()
                        },
                    );
                    g_st().rc_did_emsg = true;
                    return ptr::null_mut();
                }
            }
        } else if c == magic_fn(b'[' as i32) {
            goto_collection = true;
        }
    }

    if goto_collection {
        let lp = skip_anyof(re_st().regparse).cast::<u8>();
        if unsafe { *lp } == b']' {
            let mut startc: i32 = -1;
            if extra == 0 {
                let cl = bt_recognize_char_class(re_st().regparse.cast(), lp);
                if cl != 0 {
                    ret = regnode(cl);
                    re_st().regparse = lp.cast();
                    skipchr();
                    *flagp |= HASWIDTH | SIMPLE;
                    return ret;
                }
            }

            if crate::memory::cstr_first_byte(re_st().regparse) == b'^' {
                ret = regnode(ANYBUT + extra);
                re_st().regparse = re_st().regparse.wrapping_add(1);
            } else {
                ret = regnode(ANYOF + extra);
            }

            if crate::memory::cstr_first_byte(re_st().regparse) == b']'
                || crate::memory::cstr_first_byte(re_st().regparse) == b'-'
            {
                startc = crate::memory::cstr_first_byte(re_st().regparse) as i32;
                regc(startc);
                re_st().regparse = re_st().regparse.wrapping_add(1);
            }

            while !crate::memory::cstr_is_empty(re_st().regparse)
                && crate::memory::cstr_first_byte(re_st().regparse) != b']'
            {
                if crate::memory::cstr_first_byte(re_st().regparse) == b'-' {
                    re_st().regparse = re_st().regparse.wrapping_add(1);
                    if crate::memory::cstr_first_byte(re_st().regparse) == b']'
                        || crate::memory::cstr_is_empty(re_st().regparse)
                        || startc == -1
                        || (crate::memory::cstr_first_byte(re_st().regparse) == b'\\'
                            && crate::memory::cstr_byte_at(re_st().regparse, 1) == b'n')
                    {
                        regc(b'-' as i32);
                        startc = b'-' as i32;
                    } else {
                        let mut endc: i32 = 0;
                        if crate::memory::cstr_first_byte(re_st().regparse) == b'[' {
                            endc = get_coll_element(&raw mut re_st().regparse);
                        }
                        if endc == 0 {
                            let mut rp_const: *const c_char = re_st().regparse;
                            endc = mb_ptr2char_adv(&raw mut rp_const);
                            re_st().regparse = rp_const.cast_mut();
                        }
                        if endc == b'\\' as i32 && re_st().reg_cpo_lit == 0 {
                            endc = coll_get_char();
                        }
                        if startc > endc {
                            emsg(tr(E_REVERSE_RANGE));
                            g_st().rc_did_emsg = true;
                            return ptr::null_mut();
                        }
                        if utf_char2len(startc) > 1 || utf_char2len(endc) > 1 {
                            if endc > startc + 256 {
                                emsg(tr(E_LARGE_CLASS));
                                g_st().rc_did_emsg = true;
                                return ptr::null_mut();
                            }
                            while {
                                startc += 1;
                                startc <= endc
                            } {
                                regmbc(startc);
                            }
                        } else {
                            while {
                                startc += 1;
                                startc <= endc
                            } {
                                regc(startc);
                            }
                        }
                        startc = -1;
                    }
                } else if crate::memory::cstr_first_byte(re_st().regparse) == b'\\'
                    && (crate::strings::vim_haschr(
                        REGEXP_INRANGE,
                        crate::memory::cstr_byte_at(re_st().regparse, 1) as i32,
                    ) || (re_st().reg_cpo_lit == 0
                        && crate::strings::vim_haschr(
                            REGEXP_ABBR,
                            crate::memory::cstr_byte_at(re_st().regparse, 1) as i32,
                        )))
                {
                    re_st().regparse = re_st().regparse.wrapping_add(1);
                    if crate::memory::cstr_first_byte(re_st().regparse) == b'n' {
                        if ret != just_calc_size() && unsafe { *ret } == ANYOF as u8 {
                            unsafe {
                                *ret = (ANYOF + ADD_NL) as u8;
                            }
                            *flagp |= HASNL;
                        }
                        re_st().regparse = re_st().regparse.wrapping_add(1);
                        startc = -1;
                    } else if matches!(
                        crate::memory::cstr_first_byte(re_st().regparse),
                        b'd' | b'o' | b'x' | b'u' | b'U'
                    ) {
                        startc = coll_get_char();
                        if startc == i32::MAX {
                            emsg(tr(E_UNICODE_VAL_TOO_LARGE));
                            g_st().rc_did_emsg = true;
                            return ptr::null_mut();
                        }
                        if startc == 0 {
                            regc(0x0a);
                        } else {
                            regmbc(startc);
                        }
                    } else {
                        startc = backslash_trans(
                            crate::memory::cstr_first_byte(re_st().regparse) as i32
                        );
                        re_st().regparse = re_st().regparse.wrapping_add(1);
                        regc(startc);
                    }
                } else if crate::memory::cstr_first_byte(re_st().regparse) == b'[' {
                    let mut c_class = get_char_class(&raw mut re_st().regparse);
                    startc = -1;
                    match c_class {
                        CLASS_NONE => {
                            c_class = get_equi_class(&raw mut re_st().regparse);
                            if c_class != 0 {
                                reg_equi_class(c_class);
                            } else {
                                c_class = get_coll_element(&raw mut re_st().regparse);
                                if c_class != 0 {
                                    regmbc(c_class);
                                } else {
                                    startc =
                                        crate::memory::cstr_first_byte(re_st().regparse) as i32;
                                    re_st().regparse = re_st().regparse.wrapping_add(1);
                                    regc(startc);
                                }
                            }
                        }
                        CLASS_ALNUM => {
                            for cu in 1..128u8 {
                                if cu.is_ascii_alphanumeric() {
                                    regmbc(cu as i32);
                                }
                            }
                        }
                        CLASS_ALPHA => {
                            for cu in 1..128u8 {
                                if cu.is_ascii_alphabetic() {
                                    regmbc(cu as i32);
                                }
                            }
                        }
                        CLASS_BLANK => {
                            regc(b' ' as i32);
                            regc(b'\t' as i32);
                        }
                        CLASS_CNTRL => {
                            for cu in 1..=127u8 {
                                if cu.is_ascii_control() {
                                    regmbc(cu as i32);
                                }
                            }
                        }
                        CLASS_DIGIT => {
                            for cu in 1..=127u8 {
                                if ascii_isdigit(cu as i32) {
                                    regmbc(cu as i32);
                                }
                            }
                        }
                        CLASS_GRAPH => {
                            for cu in 1..=127u8 {
                                if cu.is_ascii_graphic() {
                                    regmbc(cu as i32);
                                }
                            }
                        }
                        CLASS_LOWER => {
                            for cu in 1..=255 {
                                if mb_islower(cu) && cu != 170 && cu != 186 {
                                    regmbc(cu);
                                }
                            }
                        }
                        CLASS_PRINT => {
                            for cu in 1..=255 {
                                if vim_isprintc(cu) {
                                    regmbc(cu);
                                }
                            }
                        }
                        CLASS_PUNCT => {
                            for cu in 1..128u8 {
                                if cu.is_ascii_punctuation() {
                                    regmbc(cu as i32);
                                }
                            }
                        }
                        CLASS_SPACE => {
                            for cu in 9..=13 {
                                regc(cu);
                            }
                            regc(b' ' as i32);
                        }
                        CLASS_UPPER => {
                            for cu in 1..=255 {
                                if mb_isupper(cu) {
                                    regmbc(cu);
                                }
                            }
                        }
                        CLASS_XDIGIT => {
                            for cu in 1..=255 {
                                if ascii_isxdigit(cu) {
                                    regmbc(cu);
                                }
                            }
                        }
                        CLASS_TAB => regc(b'\t' as i32),
                        CLASS_RETURN => regc(b'\r' as i32),
                        CLASS_BACKSPACE => regc(BS),
                        CLASS_ESCAPE => regc(ESC),
                        CLASS_IDENT => {
                            for cu in 1..=255 {
                                if vim_is_idc(cu) {
                                    regmbc(cu);
                                }
                            }
                        }
                        CLASS_KEYWORD => {
                            for cu in 1..=255 {
                                if reg_iswordc(cu) {
                                    regmbc(cu);
                                }
                            }
                        }
                        CLASS_FNAME => {
                            for cu in 1..=255 {
                                if vim_isfilec(cu) {
                                    regmbc(cu);
                                }
                            }
                        }
                        _ => {}
                    }
                } else {
                    startc = utf_ptr2char(re_st().regparse);
                    let mut len = utfc_ptr2len(re_st().regparse);
                    if utf_char2len(startc) != len {
                        startc = -1;
                    }
                    while {
                        len -= 1;
                        len >= 0
                    } {
                        regc(crate::memory::cstr_first_byte(re_st().regparse) as i32);
                        re_st().regparse = re_st().regparse.wrapping_add(1);
                    }
                }
            }
            regc(NUL as i32);
            re_st().prevchr_len = 1;
            if crate::memory::cstr_first_byte(re_st().regparse) != b']' {
                emsg(tr(E_TOOMSBRA));
                g_st().rc_did_emsg = true;
                return ptr::null_mut();
            }
            skipchr();
            *flagp |= HASWIDTH | SIMPLE;
            return ret;
        } else if re_st().reg_strict != 0 {
            semsg!(
                tr(E_MISSINGBRACKET),
                if re_st().reg_magic > MAGIC_OFF {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
            g_st().rc_did_emsg = true;
            return ptr::null_mut();
        }
    }

    if goto_multibyte || use_multibytecode(c) {
        ret = regnode(MULTIBYTECODE);
        regmbc(c);
        *flagp |= HASWIDTH | SIMPLE;
        return ret;
    }

    ret = regnode(EXACTLY);
    let mut len: i32 = 0;
    while c != NUL as i32
        && (len == 0
            || (re_multi_type(peekchr()) == NOT_MULTI && !re_st().one_exactly && !is_magic_fn(c)))
    {
        c = no_magic(c);
        regmbc(c);
        let mut state: GraphemeState = GRAPHEME_STATE_INIT;
        loop {
            let l = utf_ptr2len(re_st().regparse);
            if !utf_composinglike(
                re_st().regparse,
                re_st().regparse.wrapping_add(l as usize),
                Some(&mut state),
            ) {
                break;
            }
            regmbc(utf_ptr2char(re_st().regparse));
            skipchr();
        }
        c = getchr();
        len += 1;
    }
    ungetchr();

    regc(NUL as i32);
    *flagp |= HASWIDTH;
    if len == 1 {
        *flagp |= SIMPLE;
    }
    ret
}

fn regpiece(flagp: &mut i32) -> *mut u8 {
    let mut flags: i32 = 0;
    let mut minval: i32 = 0;
    let mut maxval: i32 = 0;

    let ret = regatom(&mut flags);
    if ret.is_null() {
        return ptr::null_mut();
    }

    let op = peekchr();
    if re_multi_type(op) == NOT_MULTI {
        *flagp = flags;
        return ret;
    }
    *flagp = WORST | SPSTART | (flags & (HASNL | HASLOOKBH));

    skipchr();
    if op == magic_fn(b'*' as i32) {
        if (flags & SIMPLE) != 0 {
            reginsert(STAR, ret);
        } else {
            reginsert(BRANCH, ret);
            regoptail(ret, regnode(BACK));
            regoptail(ret, ret);
            regtail(ret, regnode(BRANCH));
            regtail(ret, regnode(NOTHING));
        }
    } else if op == magic_fn(b'+' as i32) {
        if (flags & SIMPLE) != 0 {
            reginsert(PLUS, ret);
        } else {
            let next = regnode(BRANCH);
            regtail(ret, next);
            regtail(regnode(BACK), ret);
            regtail(next, regnode(BRANCH));
            regtail(ret, regnode(NOTHING));
        }
        *flagp = WORST | HASWIDTH | (flags & (HASNL | HASLOOKBH));
    } else if op == magic_fn(b'@' as i32) {
        let mut lop = END;
        let mut nr = getdecchrs();
        match no_magic(getchr()) as u8 {
            b'=' => lop = MATCH,
            b'!' => lop = NOMATCH,
            b'>' => lop = SUBPAT,
            b'<' => match no_magic(getchr()) as u8 {
                b'=' => lop = BEHIND,
                b'!' => lop = NOBEHIND,
                _ => {}
            },
            _ => {}
        }
        if lop == END {
            semsg!(
                tr(E_INVALID_CHARACTER_AFTER_STR_AT),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
            g_st().rc_did_emsg = true;
            return ptr::null_mut();
        }
        if lop == BEHIND || lop == NOBEHIND {
            regtail(ret, regnode(BHPOS));
            *flagp |= HASLOOKBH;
        }
        regtail(ret, regnode(END));
        if lop == BEHIND || lop == NOBEHIND {
            if nr < 0 {
                nr = 0;
            }
            reginsert_nr(lop, i64::from(nr as u32), ret);
        } else {
            reginsert(lop, ret);
        }
    } else if op == magic_fn(b'?' as i32) || op == magic_fn(b'=' as i32) {
        reginsert(BRANCH, ret);
        regtail(ret, regnode(BRANCH));
        let next = regnode(NOTHING);
        regtail(ret, next);
        regoptail(ret, next);
    } else if op == magic_fn(b'{' as i32) {
        if !read_limits(&mut minval, &mut maxval) {
            return ptr::null_mut();
        }
        if (flags & SIMPLE) != 0 {
            reginsert(BRACE_SIMPLE, ret);
            reginsert_limits(BRACE_LIMITS, i64::from(minval), i64::from(maxval), ret);
        } else {
            if re_st().num_complex_braces >= 10 {
                semsg!(
                    tr(c"E60: Too many complex %s{...}s"),
                    if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    },
                );
                g_st().rc_did_emsg = true;
                return ptr::null_mut();
            }
            reginsert(BRACE_COMPLEX + re_st().num_complex_braces, ret);
            regoptail(ret, regnode(BACK));
            regoptail(ret, ret);
            reginsert_limits(BRACE_LIMITS, i64::from(minval), i64::from(maxval), ret);
            re_st().num_complex_braces += 1;
        }
        if minval > 0 && maxval > 0 {
            *flagp = HASWIDTH | (flags & (HASNL | HASLOOKBH));
        }
    }

    if re_multi_type(peekchr()) != NOT_MULTI {
        if peekchr() == magic_fn(b'*' as i32) {
            semsg!(
                tr(c"E61: Nested %s*"),
                if re_st().reg_magic >= MAGIC_ON {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
            g_st().rc_did_emsg = true;
            return ptr::null_mut();
        }
        semsg!(
            tr(c"E62: Nested %s%c"),
            if re_st().reg_magic == MAGIC_ALL {
                c"".as_ptr()
            } else {
                c"\\".as_ptr()
            },
            no_magic(peekchr()),
        );
        g_st().rc_did_emsg = true;
        return ptr::null_mut();
    }

    ret
}

fn regconcat(flagp: &mut i32) -> *mut u8 {
    let mut first: *mut u8 = ptr::null_mut();
    let mut chain: *mut u8 = ptr::null_mut();
    let mut flags: i32 = 0;
    let mut cont = true;

    *flagp = WORST;

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
            let latest = regpiece(&mut flags);
            if latest.is_null() || re_st().reg_toolong != 0 {
                return ptr::null_mut();
            }
            *flagp |= flags & (HASWIDTH | HASNL | HASLOOKBH);
            if chain.is_null() {
                *flagp |= flags & SPSTART;
            } else {
                regtail(chain, latest);
            }
            chain = latest;
            if first.is_null() {
                first = latest;
            }
        }
    }
    if first.is_null() {
        first = regnode(NOTHING);
    }
    first
}

fn regbranch(flagp: &mut i32) -> *mut u8 {
    let mut chain: *mut u8 = ptr::null_mut();
    let mut flags: i32 = 0;

    *flagp = WORST | HASNL;

    let ret = regnode(BRANCH);
    loop {
        let latest = regconcat(&mut flags);
        if latest.is_null() {
            return ptr::null_mut();
        }
        *flagp |= flags & (HASWIDTH | SPSTART | HASLOOKBH);
        *flagp &= !HASNL | (flags & HASNL);
        if !chain.is_null() {
            regtail(chain, latest);
        }
        if peekchr() != magic_fn(b'&' as i32) {
            break;
        }
        skipchr();
        regtail(latest, regnode(END));
        if re_st().reg_toolong != 0 {
            break;
        }
        reginsert(MATCH, latest);
        chain = latest;
    }
    ret
}

fn reg(paren: i32, flagp: &mut i32) -> *mut u8 {
    let mut ret: *mut u8;
    let mut parno: i32 = 0;
    let mut flags: i32 = 0;

    *flagp = HASWIDTH;

    if paren == REG_ZPAREN {
        if re_st().regnzpar >= NSUBEXP as i32 {
            emsg(tr(c"E50: Too many \\z("));
            g_st().rc_did_emsg = true;
            return ptr::null_mut();
        }
        parno = re_st().regnzpar;
        re_st().regnzpar += 1;
        ret = regnode(ZOPEN + parno);
    } else if paren == REG_PAREN {
        if re_st().regnpar >= NSUBEXP as i32 {
            semsg!(
                tr(c"E51: Too many %s("),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
            g_st().rc_did_emsg = true;
            return ptr::null_mut();
        }
        parno = re_st().regnpar;
        re_st().regnpar += 1;
        ret = regnode(MOPEN + parno);
    } else if paren == REG_NPAREN {
        ret = regnode(NOPEN);
    } else {
        ret = ptr::null_mut();
    }

    if re_st().bt_reg_parse_depth >= REG_MAX_PAREN_DEPTH {
        emsg(tr(E_COMMAND_TOO_COMPLEX));
        g_st().rc_did_emsg = true;
        return ptr::null_mut();
    }
    re_st().bt_reg_parse_depth += 1;

    let mut br = regbranch(&mut flags);
    if br.is_null() {
        re_st().bt_reg_parse_depth -= 1;
        return ptr::null_mut();
    }
    if !ret.is_null() {
        regtail(ret, br);
    } else {
        ret = br;
    }
    if (flags & HASWIDTH) == 0 {
        *flagp &= !HASWIDTH;
    }
    *flagp |= flags & (SPSTART | HASNL | HASLOOKBH);
    while peekchr() == magic_fn(b'|' as i32) {
        skipchr();
        br = regbranch(&mut flags);
        if br.is_null() || re_st().reg_toolong != 0 {
            re_st().bt_reg_parse_depth -= 1;
            return ptr::null_mut();
        }
        regtail(ret, br);
        if (flags & HASWIDTH) == 0 {
            *flagp &= !HASWIDTH;
        }
        *flagp |= flags & (SPSTART | HASNL | HASLOOKBH);
    }

    let ender = regnode(if paren == REG_ZPAREN {
        ZCLOSE + parno
    } else if paren == REG_PAREN {
        MCLOSE + parno
    } else if paren == REG_NPAREN {
        NCLOSE
    } else {
        END
    });
    regtail(ret, ender);

    br = ret;
    while !br.is_null() {
        regoptail(br, ender);
        br = regnext(br);
    }

    if paren != REG_NOPAREN && getchr() != magic_fn(b')' as i32) {
        if paren == REG_ZPAREN {
            emsg(tr(E_UNMATCHED_Z));
        } else if paren == REG_NPAREN {
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
        re_st().bt_reg_parse_depth -= 1;
        return ptr::null_mut();
    } else if paren == REG_NOPAREN && peekchr() != NUL as i32 {
        if re_st().curchr == magic_fn(b')' as i32) {
            semsg!(
                tr(E_UNMATCHED_STR_CLOSE),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
        } else {
            emsg(tr(E_TRAILING));
        }
        g_st().rc_did_emsg = true;
        re_st().bt_reg_parse_depth -= 1;
        return ptr::null_mut();
    }

    if paren == REG_PAREN {
        unsafe {
            *(&raw mut re_st().had_endbrace)
                .cast::<u8>()
                .wrapping_add(parno as usize) = 1;
        }
    }

    re_st().bt_reg_parse_depth -= 1;
    ret
}

pub(super) fn bt_regcomp(expr: *mut u8, re_flags: i32) -> Option<BtRegprog> {
    let mut flags: i32 = 0;

    if expr.is_null() {
        iemsg(tr(E_NULL));
        g_st().rc_did_emsg = true;
        return None;
    }

    init_class_tab();

    regcomp_start(expr, re_flags);
    re_st().regcode = just_calc_size();
    regc(REGMAGIC);
    if reg(REG_NOPAREN, &mut flags).is_null() {
        return None;
    }

    let mut r = BtRegprog {
        regflags: 0,
        regstart: 0,
        reganch: 0,
        regmust_off: None,
        regmlen: 0,
        reghasz: 0,
        program: vec![0u8; re_st().regsize as usize],
    };

    regcomp_start(expr, re_flags);
    re_st().regcode = r.program.as_mut_ptr();
    regc(REGMAGIC);
    if reg(REG_NOPAREN, &mut flags).is_null() || re_st().reg_toolong != 0 {
        if re_st().reg_toolong != 0 {
            emsg(tr(c"E339: Pattern too long"));
            g_st().rc_did_emsg = true;
        }
        return None;
    }

    r.regstart = NUL as i32;
    r.reganch = 0;
    r.regmust_off = None;
    r.regmlen = 0;
    r.regflags = re_st().regflags;
    if (flags & HASNL) != 0 {
        r.regflags |= RF_HASNL;
    }
    if (flags & HASLOOKBH) != 0 {
        r.regflags |= RF_LOOKBH;
    }
    r.reghasz = re_st().re_has_z as u8;

    let mut scan = r.program.as_mut_ptr().wrapping_add(1);
    if op_fn(regnext(scan)) == END {
        scan = operand_fn(scan);

        if op_fn(scan) == BOL || op_fn(scan) == RE_BOF {
            r.reganch += 1;
            scan = regnext(scan);
        }

        if op_fn(scan) == EXACTLY {
            r.regstart = utf_ptr2char(operand_fn(scan).cast());
        } else if op_fn(scan) == BOW
            || op_fn(scan) == EOW
            || op_fn(scan) == NOTHING
            || op_fn(scan) == MOPEN
            || op_fn(scan) == NOPEN
            || op_fn(scan) == MCLOSE
            || op_fn(scan) == NCLOSE
        {
            let regnext_scan = regnext(scan);
            if op_fn(regnext_scan) == EXACTLY {
                r.regstart = utf_ptr2char(operand_fn(regnext_scan).cast());
            }
        }

        if ((flags & SPSTART) != 0 || op_fn(scan) == BOW || op_fn(scan) == EOW)
            && (flags & HASNL) == 0
        {
            let mut longest: *mut u8 = ptr::null_mut();
            let mut len: i32 = 0;
            while !scan.is_null() {
                if op_fn(scan) == EXACTLY {
                    let scanlen = crate::memory::cstr_len(operand_fn(scan).cast());
                    if scanlen >= len as usize {
                        longest = operand_fn(scan);
                        len = scanlen as i32;
                    }
                }
                scan = regnext(scan);
            }
            if !longest.is_null() {
                r.regmust_off = Some(unsafe { longest.offset_from(r.program.as_ptr()) } as usize);
            }
            r.regmlen = len;
        }
    }
    Some(r)
}

fn reg_save(save: &mut Regsave) {
    let len = re_st().backpos.len() as i32;
    if reg_multi() {
        save.rs_u = crate::regexp::RegPos::Pos(Lpos {
            col: unsafe { re_st().rex.input.offset_from(re_st().rex.line) } as Colnr,
            lnum: re_st().rex.lnum,
        });
    } else {
        save.rs_u = crate::regexp::RegPos::Ptr(re_st().rex.input);
    }
    save.rs_len = len;
}

fn reg_restore(save: &mut Regsave) {
    if reg_multi() {
        let pos = save.rs_u.pos();
        if re_st().rex.lnum != pos.lnum {
            re_st().rex.lnum = pos.lnum;
            re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
        }
        re_st().rex.input = re_st().rex.line.wrapping_add(pos.col as usize);
    } else {
        re_st().rex.input = save.rs_u.ptr();
    }
    re_st().backpos.truncate(save.rs_len as usize);
}

fn reg_save_equal(save: &Regsave) -> bool {
    if reg_multi() {
        let pos = save.rs_u.pos();
        re_st().rex.lnum == pos.lnum
            && re_st().rex.input == re_st().rex.line.wrapping_add(pos.col as usize)
    } else {
        re_st().rex.input == save.rs_u.ptr()
    }
}

fn save_se_multi(savep: &mut SaveSe, posp: &mut Lpos) {
    savep.se_u = crate::regexp::RegPos::Pos(*posp);
    posp.lnum = re_st().rex.lnum;
    posp.col = unsafe { re_st().rex.input.offset_from(re_st().rex.line) } as Colnr;
}

fn save_se_one(savep: &mut SaveSe, pp: *mut *mut u8) {
    unsafe {
        savep.se_u = crate::regexp::RegPos::Ptr(*pp);
        *pp = re_st().rex.input;
    }
}

#[inline]
fn save_se(savep: *mut SaveSe, posp: *mut Lpos, pp: *mut *mut u8) {
    if reg_multi() {
        save_se_multi(unsafe { &mut *savep }, unsafe { &mut *posp });
    } else {
        save_se_one(unsafe { &mut *savep }, pp);
    }
}

#[inline]
fn restore_se(savep: *const SaveSe, posp: *mut Lpos, pp: *mut *mut u8) {
    unsafe {
        if reg_multi() {
            *posp = (*savep).se_u.pos();
        } else {
            *pp = (*savep).se_u.ptr();
        }
    }
}

fn regrepeat(p: *mut u8, maxcount: i64) -> i32 {
    unsafe {
        let mut count: i64 = 0;
        let mut testval: i16 = 0;
        let mut scan = re_st().rex.input;
        let opnd = operand_fn(p);
        let op = op_fn(p);

        let do_class = |mask: i16, testval: i16, scan: &mut *mut u8, count: &mut i64| {
            while *count < maxcount {
                if **scan == NUL {
                    if !reg_multi()
                        || !with_nl(op)
                        || re_st().rex.lnum > re_st().rex.reg_maxline
                        || re_st().rex.reg_line_lbr
                    {
                        break;
                    }
                    reg_nextline();
                    *scan = re_st().rex.input;
                    if g_st().got_int {
                        break;
                    }
                } else {
                    let l = utfc_ptr2len((*scan).cast());
                    if l > 1 {
                        if testval != 0 {
                            break;
                        }
                        *scan = (*scan).wrapping_add(l as usize);
                    } else if ((re_st().class_tab[**scan as usize] & mask) == testval)
                        || (re_st().rex.reg_line_lbr && **scan == b'\n' && with_nl(op))
                    {
                        *scan = (*scan).wrapping_add(1);
                    } else {
                        break;
                    }
                }
                *count += 1;
            }
        };

        if op == ANY || op == ANY + ADD_NL {
            while count < maxcount {
                while *scan != NUL && count < maxcount {
                    count += 1;
                    mb_ptr_adv(&raw mut scan);
                }
                if !reg_multi()
                    || !with_nl(op)
                    || re_st().rex.lnum > re_st().rex.reg_maxline
                    || re_st().rex.reg_line_lbr
                    || count == maxcount
                {
                    break;
                }
                count += 1;
                reg_nextline();
                scan = re_st().rex.input;
                if g_st().got_int {
                    break;
                }
            }
        } else if op == IDENT || op == IDENT + ADD_NL || op == SIDENT || op == SIDENT + ADD_NL {
            if op == IDENT || op == IDENT + ADD_NL {
                testval = 1;
            }
            while count < maxcount {
                if vim_is_idc(utf_ptr2char(scan.cast()))
                    && (testval != 0 || !ascii_isdigit(*scan as i32))
                {
                    mb_ptr_adv(&raw mut scan);
                } else if *scan == NUL {
                    if !reg_multi()
                        || !with_nl(op)
                        || re_st().rex.lnum > re_st().rex.reg_maxline
                        || re_st().rex.reg_line_lbr
                    {
                        break;
                    }
                    reg_nextline();
                    scan = re_st().rex.input;
                    if g_st().got_int {
                        break;
                    }
                } else if re_st().rex.reg_line_lbr && *scan == b'\n' && with_nl(op) {
                    scan = scan.wrapping_add(1);
                } else {
                    break;
                }
                count += 1;
            }
        } else if op == KWORD || op == KWORD + ADD_NL || op == SKWORD || op == SKWORD + ADD_NL {
            if op == KWORD || op == KWORD + ADD_NL {
                testval = 1;
            }
            while count < maxcount {
                if vim_iswordp_buf(scan.cast(), &mut *re_st().rex.reg_buf)
                    && (testval != 0 || !ascii_isdigit(*scan as i32))
                {
                    mb_ptr_adv(&raw mut scan);
                } else if *scan == NUL {
                    if !reg_multi()
                        || !with_nl(op)
                        || re_st().rex.lnum > re_st().rex.reg_maxline
                        || re_st().rex.reg_line_lbr
                    {
                        break;
                    }
                    reg_nextline();
                    scan = re_st().rex.input;
                    if g_st().got_int {
                        break;
                    }
                } else if re_st().rex.reg_line_lbr && *scan == b'\n' && with_nl(op) {
                    scan = scan.wrapping_add(1);
                } else {
                    break;
                }
                count += 1;
            }
        } else if op == FNAME || op == FNAME + ADD_NL || op == SFNAME || op == SFNAME + ADD_NL {
            if op == FNAME || op == FNAME + ADD_NL {
                testval = 1;
            }
            while count < maxcount {
                if vim_isfilec(utf_ptr2char(scan.cast()))
                    && (testval != 0 || !ascii_isdigit(*scan as i32))
                {
                    mb_ptr_adv(&raw mut scan);
                } else if *scan == NUL {
                    if !reg_multi()
                        || !with_nl(op)
                        || re_st().rex.lnum > re_st().rex.reg_maxline
                        || re_st().rex.reg_line_lbr
                    {
                        break;
                    }
                    reg_nextline();
                    scan = re_st().rex.input;
                    if g_st().got_int {
                        break;
                    }
                } else if re_st().rex.reg_line_lbr && *scan == b'\n' && with_nl(op) {
                    scan = scan.wrapping_add(1);
                } else {
                    break;
                }
                count += 1;
            }
        } else if op == PRINT || op == PRINT + ADD_NL || op == SPRINT || op == SPRINT + ADD_NL {
            if op == PRINT || op == PRINT + ADD_NL {
                testval = 1;
            }
            while count < maxcount {
                if *scan == NUL {
                    if !reg_multi()
                        || !with_nl(op)
                        || re_st().rex.lnum > re_st().rex.reg_maxline
                        || re_st().rex.reg_line_lbr
                    {
                        break;
                    }
                    reg_nextline();
                    scan = re_st().rex.input;
                    if g_st().got_int {
                        break;
                    }
                } else if vim_isprintc(utf_ptr2char(scan.cast()))
                    && (testval != 0 || !ascii_isdigit(*scan as i32))
                {
                    mb_ptr_adv(&raw mut scan);
                } else if re_st().rex.reg_line_lbr && *scan == b'\n' && with_nl(op) {
                    scan = scan.wrapping_add(1);
                } else {
                    break;
                }
                count += 1;
            }
        } else if op == WHITE || op == WHITE + ADD_NL {
            do_class(RI_WHITE, RI_WHITE, &mut scan, &mut count);
        } else if op == NWHITE || op == NWHITE + ADD_NL {
            do_class(RI_WHITE, 0, &mut scan, &mut count);
        } else if op == DIGIT || op == DIGIT + ADD_NL {
            do_class(RI_DIGIT, RI_DIGIT, &mut scan, &mut count);
        } else if op == NDIGIT || op == NDIGIT + ADD_NL {
            do_class(RI_DIGIT, 0, &mut scan, &mut count);
        } else if op == HEX || op == HEX + ADD_NL {
            do_class(RI_HEX, RI_HEX, &mut scan, &mut count);
        } else if op == NHEX || op == NHEX + ADD_NL {
            do_class(RI_HEX, 0, &mut scan, &mut count);
        } else if op == OCTAL || op == OCTAL + ADD_NL {
            do_class(RI_OCTAL, RI_OCTAL, &mut scan, &mut count);
        } else if op == NOCTAL || op == NOCTAL + ADD_NL {
            do_class(RI_OCTAL, 0, &mut scan, &mut count);
        } else if op == WORD || op == WORD + ADD_NL {
            do_class(RI_WORD, RI_WORD, &mut scan, &mut count);
        } else if op == NWORD || op == NWORD + ADD_NL {
            do_class(RI_WORD, 0, &mut scan, &mut count);
        } else if op == HEAD || op == HEAD + ADD_NL {
            do_class(RI_HEAD, RI_HEAD, &mut scan, &mut count);
        } else if op == NHEAD || op == NHEAD + ADD_NL {
            do_class(RI_HEAD, 0, &mut scan, &mut count);
        } else if op == ALPHA || op == ALPHA + ADD_NL {
            do_class(RI_ALPHA, RI_ALPHA, &mut scan, &mut count);
        } else if op == NALPHA || op == NALPHA + ADD_NL {
            do_class(RI_ALPHA, 0, &mut scan, &mut count);
        } else if op == LOWER || op == LOWER + ADD_NL {
            do_class(RI_LOWER, RI_LOWER, &mut scan, &mut count);
        } else if op == NLOWER || op == NLOWER + ADD_NL {
            do_class(RI_LOWER, 0, &mut scan, &mut count);
        } else if op == UPPER || op == UPPER + ADD_NL {
            do_class(RI_UPPER, RI_UPPER, &mut scan, &mut count);
        } else if op == NUPPER || op == NUPPER + ADD_NL {
            do_class(RI_UPPER, 0, &mut scan, &mut count);
        } else if op == EXACTLY {
            if re_st().rex.reg_ic {
                let cu = mb_toupper(*opnd as i32);
                let cl = mb_tolower(*opnd as i32);
                while count < maxcount && (*scan as i32 == cu || *scan as i32 == cl) {
                    count += 1;
                    scan = scan.wrapping_add(1);
                }
            } else {
                let cu = *opnd;
                while count < maxcount && *scan == cu {
                    count += 1;
                    scan = scan.wrapping_add(1);
                }
            }
        } else if op == MULTIBYTECODE {
            let len = utfc_ptr2len(opnd.cast()) as usize;
            if len > 1 {
                let cf = if re_st().rex.reg_ic {
                    utf_fold(utf_ptr2char(opnd.cast()))
                } else {
                    0
                };
                while count < maxcount && (utfc_ptr2len(scan.cast()) as usize) >= len {
                    let mut i = 0;
                    while i < len {
                        if *opnd.wrapping_add(i) != *scan.wrapping_add(i) {
                            break;
                        }
                        i += 1;
                    }
                    if i < len && (!re_st().rex.reg_ic || utf_fold(utf_ptr2char(scan.cast())) != cf)
                    {
                        break;
                    }
                    scan = scan.wrapping_add(len);
                    count += 1;
                }
            }
        } else if op == ANYOF || op == ANYOF + ADD_NL || op == ANYBUT || op == ANYBUT + ADD_NL {
            let is_anyof = op == ANYOF || op == ANYOF + ADD_NL;
            while count < maxcount {
                if *scan == NUL {
                    if !reg_multi()
                        || !with_nl(op)
                        || re_st().rex.lnum > re_st().rex.reg_maxline
                        || re_st().rex.reg_line_lbr
                    {
                        break;
                    }
                    reg_nextline();
                    scan = re_st().rex.input;
                    if g_st().got_int {
                        break;
                    }
                } else if re_st().rex.reg_line_lbr && *scan == b'\n' && with_nl(op) {
                    scan = scan.wrapping_add(1);
                } else {
                    let len = utfc_ptr2len(scan.cast());
                    if len > 1 {
                        if cstrchr(opnd.cast(), utf_ptr2char(scan.cast())).is_null() == is_anyof {
                            break;
                        }
                        scan = scan.wrapping_add(len as usize);
                    } else {
                        if cstrchr(opnd.cast(), *scan as i32).is_null() == is_anyof {
                            break;
                        }
                        scan = scan.wrapping_add(1);
                    }
                }
                count += 1;
            }
        } else if op == NEWL {
            while count < maxcount
                && ((*scan == NUL
                    && re_st().rex.lnum <= re_st().rex.reg_maxline
                    && !re_st().rex.reg_line_lbr
                    && reg_multi())
                    || (*scan == b'\n' && re_st().rex.reg_line_lbr))
            {
                count += 1;
                if re_st().rex.reg_line_lbr {
                    advance_reginput();
                } else {
                    reg_nextline();
                }
                scan = re_st().rex.input;
                if g_st().got_int {
                    break;
                }
            }
        } else {
            iemsg(tr(E_RE_CORR));
        }

        re_st().rex.input = scan;
        count as i32
    }
}

fn regstack_push(state: Regstate, scan: *mut u8) -> *mut Regitem {
    const {
        assert!(size_of::<Regitem>().is_multiple_of(8));
        assert!(size_of::<Regstar>().is_multiple_of(8));
        assert!(size_of::<Regbehind>().is_multiple_of(8));
    }
    {
        if i64::from(((re_st().regstack.len() * 8) as u32) >> 10) >= opt_vars().p_mmp {
            emsg(tr(E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN));
            return ptr::null_mut();
        }
        let old_len = re_st().regstack.len();
        re_st()
            .regstack
            .resize(old_len + size_of::<Regitem>() / 8, 0);

        let rp = unsafe {
            &mut *re_st()
                .regstack
                .as_mut_ptr()
                .wrapping_add(old_len)
                .cast::<Regitem>()
        };
        rp.rs_state = state;
        rp.rs_scan = scan;
        rp
    }
}

fn regstack_pop(scan: *mut *mut u8) {
    let rp = re_st()
        .regstack
        .as_mut_ptr()
        .wrapping_add(re_st().regstack.len())
        .cast::<Regitem>()
        .wrapping_sub(1);
    unsafe {
        *scan = (*rp).rs_scan;
    }
    re_st()
        .regstack
        .truncate(re_st().regstack.len() - size_of::<Regitem>() / 8);
}

fn save_subexpr(bp: &mut Regbehind) {
    bp.save_need_clear_subexpr = re_st().rex.need_clear_subexpr;
    if re_st().rex.need_clear_subexpr != 0 {
        return;
    }
    for i in 0..NSUBEXP {
        if reg_multi() {
            bp.save_start[i].se_u =
                crate::regexp::RegPos::Pos(unsafe { *re_st().rex.reg_startpos.wrapping_add(i) });
            bp.save_end[i].se_u =
                crate::regexp::RegPos::Pos(unsafe { *re_st().rex.reg_endpos.wrapping_add(i) });
        } else {
            bp.save_start[i].se_u =
                crate::regexp::RegPos::Ptr(unsafe { *re_st().rex.reg_startp.wrapping_add(i) });
            bp.save_end[i].se_u =
                crate::regexp::RegPos::Ptr(unsafe { *re_st().rex.reg_endp.wrapping_add(i) });
        }
    }
}

fn restore_subexpr(bp: &mut Regbehind) {
    unsafe {
        re_st().rex.need_clear_subexpr = bp.save_need_clear_subexpr;
        if re_st().rex.need_clear_subexpr != 0 {
            return;
        }
        for i in 0..NSUBEXP {
            if reg_multi() {
                *re_st().rex.reg_startpos.wrapping_add(i) = bp.save_start[i].se_u.pos();
                *re_st().rex.reg_endpos.wrapping_add(i) = bp.save_end[i].se_u.pos();
            } else {
                *re_st().rex.reg_startp.wrapping_add(i) = bp.save_start[i].se_u.ptr();
                *re_st().rex.reg_endp.wrapping_add(i) = bp.save_end[i].se_u.ptr();
            }
        }
    }
}

fn regmatch(mut scan: *mut u8, tm: Option<Proftime>, mut timed_out: Option<&mut i32>) -> bool {
    unsafe {
        let mut status: i32;
        let mut tm_count: i32 = 0;

        re_st().regstack.clear();
        let bp = &mut re_st().backpos;
        (*bp).clear();

        loop {
            reg_breakcheck();

            loop {
                if g_st().got_int || scan.is_null() {
                    status = RA_FAIL;
                    break;
                }
                if let Some(tm_val) = tm {
                    tm_count += 1;
                    if tm_count == 100 {
                        tm_count = 0;
                        if profile_passed_limit(tm_val) {
                            if let Some(ref mut p) = timed_out {
                                **p = 1;
                            }
                            status = RA_FAIL;
                            break;
                        }
                    }
                }
                status = RA_CONT;

                let mut next = regnext(scan);
                let mut op = op_fn(scan);

                if !re_st().rex.reg_line_lbr
                    && with_nl(op)
                    && reg_multi()
                    && *re_st().rex.input == NUL
                    && re_st().rex.lnum <= re_st().rex.reg_maxline
                {
                    reg_nextline();
                } else if re_st().rex.reg_line_lbr && with_nl(op) && *re_st().rex.input == b'\n' {
                    advance_reginput();
                } else {
                    if with_nl(op) {
                        op -= ADD_NL;
                    }
                    let c = utf_ptr2char(re_st().rex.input.cast());
                    match op {
                        BOL => {
                            if re_st().rex.input != re_st().rex.line {
                                status = RA_NOMATCH;
                            }
                        }
                        EOL => {
                            if c != NUL as i32 {
                                status = RA_NOMATCH;
                            }
                        }
                        RE_BOF => {
                            if re_st().rex.lnum != 0
                                || re_st().rex.input != re_st().rex.line
                                || (reg_multi() && re_st().rex.reg_firstlnum > 1)
                            {
                                status = RA_NOMATCH;
                            }
                        }
                        RE_EOF => {
                            if re_st().rex.lnum != re_st().rex.reg_maxline || c != NUL as i32 {
                                status = RA_NOMATCH;
                            }
                        }
                        CURSOR => {
                            if re_st().rex.reg_win.is_null()
                                || (re_st().rex.lnum + re_st().rex.reg_firstlnum
                                    != re_st().rex.win_mut().w_cursor.lnum)
                                || (re_st().rex.input.offset_from(re_st().rex.line) as Colnr
                                    != re_st().rex.win_mut().w_cursor.col)
                            {
                                status = RA_NOMATCH;
                            }
                        }
                        RE_MARK => {
                            let mark = *operand_fn(scan) as i32;
                            let cmp = *operand_fn(scan).wrapping_add(1) as i32;
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
                                mark,
                            );
                            if reg_multi() {
                                re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
                                re_st().rex.input = re_st().rex.line.wrapping_add(col);
                            }
                            if fm.is_null() || (*fm).mark.lnum <= 0 {
                                status = RA_NOMATCH;
                            } else {
                                let pos = &raw const (*fm).mark;
                                let pos_col = if (*pos).lnum
                                    == re_st().rex.lnum + re_st().rex.reg_firstlnum
                                    && (*pos).col == MAXCOL
                                {
                                    reg_getline_len((*pos).lnum - re_st().rex.reg_firstlnum)
                                } else {
                                    (*pos).col
                                };
                                let cur_col =
                                    re_st().rex.input.offset_from(re_st().rex.line) as Colnr;
                                let nomatch = if (*pos).lnum
                                    == re_st().rex.lnum + re_st().rex.reg_firstlnum
                                {
                                    if pos_col == cur_col {
                                        cmp == b'<' as i32 || cmp == b'>' as i32
                                    } else if pos_col < cur_col {
                                        cmp != b'>' as i32
                                    } else {
                                        cmp != b'<' as i32
                                    }
                                } else if (*pos).lnum < re_st().rex.lnum + re_st().rex.reg_firstlnum
                                {
                                    cmp != b'>' as i32
                                } else {
                                    cmp != b'<' as i32
                                };
                                if nomatch {
                                    status = RA_NOMATCH;
                                }
                            }
                        }
                        RE_VISUAL => {
                            if !reg_match_visual() {
                                status = RA_NOMATCH;
                            }
                        }
                        RE_LNUM => {
                            if !reg_multi()
                                || !re_num_cmp(
                                    (re_st().rex.lnum + re_st().rex.reg_firstlnum) as u32,
                                    scan,
                                )
                            {
                                status = RA_NOMATCH;
                            }
                        }
                        RE_COL => {
                            if !re_num_cmp(
                                (re_st().rex.input.offset_from(re_st().rex.line) + 1) as u32,
                                scan,
                            ) {
                                status = RA_NOMATCH;
                            }
                        }
                        RE_VCOL => {
                            let wp = if re_st().rex.reg_win.is_null() {
                                crate::globals::curwin_ptr()
                            } else {
                                re_st().rex.reg_win
                            };
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
                            let vcol = win_linetabsize(
                                &mut *wp,
                                lnum,
                                re_st().rex.line.cast(),
                                re_st().rex.input.offset_from(re_st().rex.line) as Colnr,
                            );
                            if !re_num_cmp((vcol as u32).wrapping_add(1), scan) {
                                status = RA_NOMATCH;
                            }
                        }
                        BOW => {
                            if c == NUL as i32 {
                                status = RA_NOMATCH;
                            } else {
                                let this_class = mb_get_class_tab(
                                    re_st().rex.input.cast(),
                                    (&raw const re_st().rex.buf_mut().b_chartab).cast(),
                                );
                                if this_class <= 1 || reg_prev_class() == this_class {
                                    status = RA_NOMATCH;
                                }
                            }
                        }
                        EOW => {
                            if re_st().rex.input == re_st().rex.line {
                                status = RA_NOMATCH;
                            } else {
                                let this_class = mb_get_class_tab(
                                    re_st().rex.input.cast(),
                                    (&raw const re_st().rex.buf_mut().b_chartab).cast(),
                                );
                                let prev_class = reg_prev_class();
                                if this_class == prev_class || prev_class == 0 || prev_class == 1 {
                                    status = RA_NOMATCH;
                                }
                            }
                        }
                        ANY => {
                            if c == NUL as i32 {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        IDENT => {
                            if !vim_is_idc(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        SIDENT => {
                            if ascii_isdigit(*re_st().rex.input as i32) || !vim_is_idc(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        KWORD => {
                            if !vim_iswordp_buf(re_st().rex.input.cast(), &mut *re_st().rex.reg_buf)
                            {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        SKWORD => {
                            if ascii_isdigit(*re_st().rex.input as i32)
                                || !vim_iswordp_buf(
                                    re_st().rex.input.cast(),
                                    &mut *re_st().rex.reg_buf,
                                )
                            {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        FNAME => {
                            if !vim_isfilec(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        SFNAME => {
                            if ascii_isdigit(*re_st().rex.input as i32) || !vim_isfilec(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        PRINT => {
                            if !vim_isprintc(utf_ptr2char(re_st().rex.input.cast())) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        SPRINT => {
                            if ascii_isdigit(*re_st().rex.input as i32)
                                || !vim_isprintc(utf_ptr2char(re_st().rex.input.cast()))
                            {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        WHITE => {
                            if !ascii_iswhite(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NWHITE => {
                            if c == NUL as i32 || ascii_iswhite(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        DIGIT => {
                            if !ri_digit(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NDIGIT => {
                            if c == NUL as i32 || ri_digit(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        HEX => {
                            if !ri_hex(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NHEX => {
                            if c == NUL as i32 || ri_hex(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        OCTAL => {
                            if !ri_octal(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NOCTAL => {
                            if c == NUL as i32 || ri_octal(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        WORD => {
                            if !ri_word(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NWORD => {
                            if c == NUL as i32 || ri_word(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        HEAD => {
                            if !ri_head(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NHEAD => {
                            if c == NUL as i32 || ri_head(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        ALPHA => {
                            if !ri_alpha(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NALPHA => {
                            if c == NUL as i32 || ri_alpha(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        LOWER => {
                            if !ri_lower(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NLOWER => {
                            if c == NUL as i32 || ri_lower(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        UPPER => {
                            if !ri_upper(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        NUPPER => {
                            if c == NUL as i32 || ri_upper(c) {
                                status = RA_NOMATCH;
                            } else {
                                advance_reginput();
                            }
                        }
                        EXACTLY => {
                            let opnd = operand_fn(scan);
                            if *opnd != *re_st().rex.input && !re_st().rex.reg_ic {
                                status = RA_NOMATCH;
                            } else if *opnd != NUL {
                                let len = if crate::memory::cstr_byte_at(opnd.cast(), 1) == 0
                                    && !re_st().rex.reg_ic
                                {
                                    1
                                } else {
                                    let mut l = crate::memory::cstr_len(opnd.cast()) as i32;
                                    if cstrncmp(opnd.cast(), re_st().rex.input.cast(), &mut l) != 0
                                    {
                                        status = RA_NOMATCH;
                                    }
                                    l
                                };
                                if status != RA_NOMATCH
                                    && utf_composinglike(
                                        re_st().rex.input.cast(),
                                        re_st().rex.input.wrapping_add(len as usize).cast(),
                                        None,
                                    )
                                    && !re_st().rex.reg_icombine
                                    && op_fn(next) != RE_COMPOSING
                                {
                                    status = RA_NOMATCH;
                                }
                                if status != RA_NOMATCH {
                                    re_st().rex.input =
                                        re_st().rex.input.wrapping_add(len as usize);
                                }
                            }
                        }
                        ANYOF | ANYBUT => {
                            let mut q = operand_fn(scan);
                            if c == NUL as i32 || (cstrchr(q.cast(), c).is_null() == (op == ANYOF))
                            {
                                status = RA_NOMATCH;
                            } else {
                                let len = (utfc_ptr2len(q.cast()) - utf_ptr2len(q.cast())) as usize;
                                re_st().rex.input = re_st()
                                    .rex
                                    .input
                                    .wrapping_add(utf_ptr2len(re_st().rex.input.cast()) as usize);
                                q = q.wrapping_add(utf_ptr2len(q.cast()) as usize);
                                if len != 0 {
                                    for i in 0..len {
                                        if *q.wrapping_add(i) != *re_st().rex.input.wrapping_add(i)
                                        {
                                            status = RA_NOMATCH;
                                            break;
                                        }
                                    }
                                    re_st().rex.input = re_st().rex.input.wrapping_add(len);
                                }
                            }
                        }
                        MULTIBYTECODE => {
                            let opnd = operand_fn(scan);
                            let mut len = utfc_ptr2len(opnd.cast());
                            if len < 2 {
                                status = RA_NOMATCH;
                            } else {
                                let opndc = utf_ptr2char(opnd.cast());
                                if utf_iscomposing_legacy(opndc) {
                                    status = RA_NOMATCH;
                                    let mut i: usize = 0;
                                    while *re_st().rex.input.wrapping_add(i) != NUL {
                                        let step =
                                            utf_ptr2len(re_st().rex.input.wrapping_add(i).cast())
                                                as usize;
                                        let inpc =
                                            utf_ptr2char(re_st().rex.input.wrapping_add(i).cast());
                                        if !utf_iscomposing_legacy(inpc) {
                                            if i > 0 {
                                                break;
                                            }
                                        } else if opndc == inpc {
                                            len = (i as i32)
                                                + utfc_ptr2len(
                                                    re_st().rex.input.wrapping_add(i).cast(),
                                                );
                                            status = RA_MATCH;
                                            break;
                                        }
                                        i += step;
                                    }
                                    if status != RA_NOMATCH {
                                        re_st().rex.input =
                                            re_st().rex.input.wrapping_add(len as usize);
                                    }
                                } else if cstrncmp(opnd.cast(), re_st().rex.input.cast(), &mut len)
                                    != 0
                                {
                                    status = RA_NOMATCH;
                                } else {
                                    re_st().rex.input =
                                        re_st().rex.input.wrapping_add(len as usize);
                                }
                            }
                        }
                        RE_COMPOSING => {
                            while utf_iscomposing_legacy(utf_ptr2char(re_st().rex.input.cast())) {
                                re_st().rex.input = re_st()
                                    .rex
                                    .input
                                    .wrapping_add(utf_ptr2len(re_st().rex.input.cast()) as usize);
                            }
                        }
                        NOTHING => {}
                        BACK => {
                            let bp = &mut re_st().backpos;
                            let idx = if let Some(pos) =
                                (*bp).iter().position(|item| item.bp_scan == scan)
                            {
                                if reg_save_equal(&(*bp).as_slice()[pos].bp_pos) {
                                    status = RA_NOMATCH;
                                }
                                pos
                            } else {
                                let pos = (*bp).len();
                                (*bp).push(Backpos {
                                    bp_scan: scan,
                                    bp_pos: Regsave::new(),
                                });
                                pos
                            };
                            if status != RA_NOMATCH {
                                let pos_ptr = &mut (*bp).as_mut_slice()[idx].bp_pos;
                                reg_save(pos_ptr);
                            }
                        }
                        x if (MOPEN..=MOPEN + 9).contains(&x) => {
                            let no = (op - MOPEN) as usize;
                            cleanup_subexpr();
                            let rp = regstack_push(RS_MOPEN, scan);
                            if rp.is_null() {
                                status = RA_FAIL;
                            } else {
                                (*rp).rs_no = no as i16;
                                save_se(
                                    (*rp).rs_un.sesave_mut(),
                                    if re_st().rex.reg_startpos.is_null() {
                                        ptr::null_mut()
                                    } else {
                                        re_st().rex.reg_startpos.wrapping_add(no)
                                    },
                                    if re_st().rex.reg_startp.is_null() {
                                        ptr::null_mut()
                                    } else {
                                        re_st().rex.reg_startp.wrapping_add(no)
                                    },
                                );
                            }
                        }
                        NOPEN | NCLOSE => {
                            if regstack_push(RS_NOPEN, scan).is_null() {
                                status = RA_FAIL;
                            }
                        }
                        x if (ZOPEN + 1..=ZOPEN + 9).contains(&x) => {
                            let no = (op - ZOPEN) as usize;
                            cleanup_zsubexpr();
                            let rp = regstack_push(RS_ZOPEN, scan);
                            if rp.is_null() {
                                status = RA_FAIL;
                            } else {
                                (*rp).rs_no = no as i16;
                                save_se(
                                    (*rp).rs_un.sesave_mut(),
                                    (&raw mut re_st().reg_startzpos)
                                        .cast::<Lpos>()
                                        .wrapping_add(no),
                                    (&raw mut re_st().reg_startzp)
                                        .cast::<*mut u8>()
                                        .wrapping_add(no),
                                );
                            }
                        }
                        x if (MCLOSE..=MCLOSE + 9).contains(&x) => {
                            let no = (op - MCLOSE) as usize;
                            cleanup_subexpr();
                            let rp = regstack_push(RS_MCLOSE, scan);
                            if rp.is_null() {
                                status = RA_FAIL;
                            } else {
                                (*rp).rs_no = no as i16;
                                save_se(
                                    (*rp).rs_un.sesave_mut(),
                                    if re_st().rex.reg_endpos.is_null() {
                                        ptr::null_mut()
                                    } else {
                                        re_st().rex.reg_endpos.wrapping_add(no)
                                    },
                                    if re_st().rex.reg_endp.is_null() {
                                        ptr::null_mut()
                                    } else {
                                        re_st().rex.reg_endp.wrapping_add(no)
                                    },
                                );
                            }
                        }
                        x if (ZCLOSE + 1..=ZCLOSE + 9).contains(&x) => {
                            let no = (op - ZCLOSE) as usize;
                            cleanup_zsubexpr();
                            let rp = regstack_push(RS_ZCLOSE, scan);
                            if rp.is_null() {
                                status = RA_FAIL;
                            } else {
                                (*rp).rs_no = no as i16;
                                save_se(
                                    (*rp).rs_un.sesave_mut(),
                                    (&raw mut re_st().reg_endzpos)
                                        .cast::<Lpos>()
                                        .wrapping_add(no),
                                    (&raw mut re_st().reg_endzp)
                                        .cast::<*mut u8>()
                                        .wrapping_add(no),
                                );
                            }
                        }
                        x if (BACKREF + 1..=BACKREF + 9).contains(&x) => {
                            let no = (op - BACKREF) as usize;
                            let mut len: i32 = 0;
                            cleanup_subexpr();
                            if !reg_multi() {
                                if !(*re_st().rex.reg_startp.wrapping_add(no)).is_null()
                                    && !(*re_st().rex.reg_endp.wrapping_add(no)).is_null()
                                {
                                    len = (*re_st().rex.reg_endp.wrapping_add(no))
                                        .offset_from(*re_st().rex.reg_startp.wrapping_add(no))
                                        as i32;
                                    if cstrncmp(
                                        (*re_st().rex.reg_startp.wrapping_add(no)).cast(),
                                        re_st().rex.input.cast(),
                                        &mut len,
                                    ) != 0
                                    {
                                        status = RA_NOMATCH;
                                    }
                                }
                            } else {
                                let spos = *re_st().rex.reg_startpos.wrapping_add(no);
                                let epos = *re_st().rex.reg_endpos.wrapping_add(no);
                                if spos.lnum >= 0 && epos.lnum >= 0 {
                                    if spos.lnum == re_st().rex.lnum
                                        && epos.lnum == re_st().rex.lnum
                                    {
                                        len = epos.col - spos.col;
                                        if cstrncmp(
                                            re_st().rex.line.wrapping_add(spos.col as usize).cast(),
                                            re_st().rex.input.cast(),
                                            &mut len,
                                        ) != 0
                                        {
                                            status = RA_NOMATCH;
                                        }
                                    } else {
                                        let r = match_with_backref(
                                            spos.lnum, spos.col, epos.lnum, epos.col, &mut len,
                                        );
                                        if r != RA_MATCH {
                                            status = r;
                                        }
                                    }
                                }
                            }
                            re_st().rex.input = re_st().rex.input.wrapping_add(len as usize);
                        }
                        x if (ZREF + 1..=ZREF + 9).contains(&x) => {
                            cleanup_zsubexpr();
                            let no = (op - ZREF) as usize;
                            let ext_in = &raw const g_st().re_extmatch_in;
                            if let Some(ref em) = *ext_in
                                && let Some(ref m_cstr) = em.matches[no]
                            {
                                let m = m_cstr.as_ptr();
                                let mut len = m_cstr.as_bytes().len() as i32;
                                if cstrncmp(m, re_st().rex.input.cast(), &mut len) != 0 {
                                    status = RA_NOMATCH;
                                } else {
                                    re_st().rex.input =
                                        re_st().rex.input.wrapping_add(len as usize);
                                }
                            }
                        }
                        BRANCH => {
                            if op_fn(next) != BRANCH {
                                next = operand_fn(scan);
                            } else {
                                let rp = regstack_push(RS_BRANCH, scan);
                                if rp.is_null() {
                                    status = RA_FAIL;
                                } else {
                                    status = RA_BREAK;
                                }
                            }
                        }
                        BRACE_LIMITS => {
                            if op_fn(next) == BRACE_SIMPLE {
                                bt_st().bl_minval = operand_min(scan);
                                bt_st().bl_maxval = operand_max(scan);
                            } else if op_fn(next) >= BRACE_COMPLEX
                                && op_fn(next) < BRACE_COMPLEX + 10
                            {
                                let no = (op_fn(next) - BRACE_COMPLEX) as usize;
                                *(&raw mut re_st().brace_min).cast::<i64>().wrapping_add(no) =
                                    operand_min(scan);
                                *(&raw mut re_st().brace_max).cast::<i64>().wrapping_add(no) =
                                    operand_max(scan);
                                *(&raw mut re_st().brace_count)
                                    .cast::<i32>()
                                    .wrapping_add(no) = 0;
                            } else {
                                internal_error(c"BRACE_LIMITS");
                                status = RA_FAIL;
                            }
                        }
                        x if (BRACE_COMPLEX..=BRACE_COMPLEX + 9).contains(&x) => {
                            let no = (op - BRACE_COMPLEX) as usize;
                            let bcount = (&raw mut re_st().brace_count)
                                .cast::<i32>()
                                .wrapping_add(no);
                            let bmin = re_st().brace_min[no];
                            let bmax = re_st().brace_max[no];
                            *bcount += 1;

                            if i64::from(*bcount) <= bmin.min(bmax) {
                                let rp = regstack_push(RS_BRCPLX_MORE, scan);
                                if rp.is_null() {
                                    status = RA_FAIL;
                                } else {
                                    (*rp).rs_no = no as i16;
                                    reg_save((*rp).rs_un.regsave_mut());
                                    next = operand_fn(scan);
                                }
                            } else if bmin <= bmax {
                                if i64::from(*bcount) <= bmax {
                                    let rp = regstack_push(RS_BRCPLX_LONG, scan);
                                    if rp.is_null() {
                                        status = RA_FAIL;
                                    } else {
                                        (*rp).rs_no = no as i16;
                                        reg_save((*rp).rs_un.regsave_mut());
                                        next = operand_fn(scan);
                                    }
                                }
                            } else if i64::from(*bcount) <= bmin {
                                let rp = regstack_push(RS_BRCPLX_SHORT, scan);
                                if rp.is_null() {
                                    status = RA_FAIL;
                                } else {
                                    reg_save((*rp).rs_un.regsave_mut());
                                }
                            }
                        }
                        BRACE_SIMPLE | STAR | PLUS => {
                            let mut rst = Regstar::new();
                            if op_fn(next) == EXACTLY {
                                rst.nextb = *operand_fn(next) as i32;
                                if re_st().rex.reg_ic {
                                    if mb_isupper(rst.nextb) {
                                        rst.nextb_ic = mb_tolower(rst.nextb);
                                    } else {
                                        rst.nextb_ic = mb_toupper(rst.nextb);
                                    }
                                } else {
                                    rst.nextb_ic = rst.nextb;
                                }
                            } else {
                                rst.nextb = NUL as i32;
                                rst.nextb_ic = NUL as i32;
                            }
                            if op != BRACE_SIMPLE {
                                rst.minval = if op == STAR { 0 } else { 1 };
                                rst.maxval = i64::from(MAX_LIMIT);
                            } else {
                                rst.minval = bt_st().bl_minval;
                                rst.maxval = bt_st().bl_maxval;
                            }

                            rst.count = i64::from(regrepeat(operand_fn(scan), rst.maxval));
                            if g_st().got_int {
                                status = RA_FAIL;
                                break;
                            }
                            if if rst.minval <= rst.maxval {
                                rst.count >= rst.minval
                            } else {
                                rst.count >= rst.maxval
                            } {
                                if i64::from(((re_st().regstack.len() * 8) as u32) >> 10)
                                    >= opt_vars().p_mmp
                                {
                                    emsg(tr(E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN));
                                    status = RA_FAIL;
                                } else {
                                    re_st().regstack.resize(
                                        re_st().regstack.len() + size_of::<Regstar>() / 8,
                                        0,
                                    );
                                    let rp = regstack_push(
                                        if rst.minval <= rst.maxval {
                                            RS_STAR_LONG
                                        } else {
                                            RS_STAR_SHORT
                                        },
                                        scan,
                                    );
                                    if rp.is_null() {
                                        status = RA_FAIL;
                                    } else {
                                        *rp.cast::<Regstar>().wrapping_sub(1) = rst;
                                        status = RA_BREAK;
                                    }
                                }
                            } else {
                                status = RA_NOMATCH;
                            }
                        }
                        NOMATCH | MATCH | SUBPAT => {
                            let rp = regstack_push(RS_NOMATCH, scan);
                            if rp.is_null() {
                                status = RA_FAIL;
                            } else {
                                (*rp).rs_no = op as i16;
                                reg_save((*rp).rs_un.regsave_mut());
                                next = operand_fn(scan);
                            }
                        }
                        BEHIND | NOBEHIND => {
                            if i64::from(((re_st().regstack.len() * 8) as u32) >> 10)
                                >= opt_vars().p_mmp
                            {
                                emsg(tr(E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN));
                                status = RA_FAIL;
                            } else {
                                re_st()
                                    .regstack
                                    .resize(re_st().regstack.len() + size_of::<Regbehind>() / 8, 0);
                                let rp = regstack_push(RS_BEHIND1, scan);
                                if rp.is_null() {
                                    status = RA_FAIL;
                                } else {
                                    save_subexpr(&mut *rp.cast::<Regbehind>().wrapping_sub(1));
                                    (*rp).rs_no = op as i16;
                                    reg_save((*rp).rs_un.regsave_mut());
                                }
                            }
                        }
                        BHPOS => {
                            if reg_multi() {
                                let bpos = re_st().behind_pos.rs_u.pos();
                                if bpos.col
                                    != re_st().rex.input.offset_from(re_st().rex.line) as Colnr
                                    || bpos.lnum != re_st().rex.lnum
                                {
                                    status = RA_NOMATCH;
                                }
                            } else if re_st().behind_pos.rs_u.ptr() != re_st().rex.input {
                                status = RA_NOMATCH;
                            }
                        }
                        NEWL => {
                            if (c != NUL as i32
                                || !reg_multi()
                                || re_st().rex.lnum > re_st().rex.reg_maxline
                                || re_st().rex.reg_line_lbr)
                                && (c != b'\n' as i32 || !re_st().rex.reg_line_lbr)
                            {
                                status = RA_NOMATCH;
                            } else if re_st().rex.reg_line_lbr {
                                advance_reginput();
                            } else {
                                reg_nextline();
                            }
                        }
                        END => {
                            status = RA_MATCH;
                        }
                        _ => {
                            iemsg(tr(E_RE_CORR));
                            status = RA_FAIL;
                        }
                    }
                }

                if status != RA_CONT {
                    break;
                }
                scan = next;
            }

            while !re_st().regstack.is_empty() && status != RA_FAIL {
                let rp = re_st()
                    .regstack
                    .as_mut_ptr()
                    .wrapping_add(re_st().regstack.len())
                    .cast::<Regitem>()
                    .wrapping_sub(1);
                match (*rp).rs_state {
                    RS_NOPEN => {
                        regstack_pop(&raw mut scan);
                    }
                    RS_MOPEN => {
                        if status == RA_NOMATCH {
                            let no = (*rp).rs_no as usize;
                            restore_se(
                                (*rp).rs_un.sesave(),
                                if re_st().rex.reg_startpos.is_null() {
                                    ptr::null_mut()
                                } else {
                                    re_st().rex.reg_startpos.wrapping_add(no)
                                },
                                if re_st().rex.reg_startp.is_null() {
                                    ptr::null_mut()
                                } else {
                                    re_st().rex.reg_startp.wrapping_add(no)
                                },
                            );
                        }
                        regstack_pop(&raw mut scan);
                    }
                    RS_ZOPEN => {
                        if status == RA_NOMATCH {
                            let no = (*rp).rs_no as usize;
                            restore_se(
                                (*rp).rs_un.sesave(),
                                (&raw mut re_st().reg_startzpos)
                                    .cast::<Lpos>()
                                    .wrapping_add(no),
                                (&raw mut re_st().reg_startzp)
                                    .cast::<*mut u8>()
                                    .wrapping_add(no),
                            );
                        }
                        regstack_pop(&raw mut scan);
                    }
                    RS_MCLOSE => {
                        if status == RA_NOMATCH {
                            let no = (*rp).rs_no as usize;
                            restore_se(
                                (*rp).rs_un.sesave(),
                                if re_st().rex.reg_endpos.is_null() {
                                    ptr::null_mut()
                                } else {
                                    re_st().rex.reg_endpos.wrapping_add(no)
                                },
                                if re_st().rex.reg_endp.is_null() {
                                    ptr::null_mut()
                                } else {
                                    re_st().rex.reg_endp.wrapping_add(no)
                                },
                            );
                        }
                        regstack_pop(&raw mut scan);
                    }
                    RS_ZCLOSE => {
                        if status == RA_NOMATCH {
                            let no = (*rp).rs_no as usize;
                            restore_se(
                                (*rp).rs_un.sesave(),
                                (&raw mut re_st().reg_endzpos)
                                    .cast::<Lpos>()
                                    .wrapping_add(no),
                                (&raw mut re_st().reg_endzp)
                                    .cast::<*mut u8>()
                                    .wrapping_add(no),
                            );
                        }
                        regstack_pop(&raw mut scan);
                    }
                    RS_BRANCH => {
                        if status == RA_MATCH {
                            regstack_pop(&raw mut scan);
                        } else {
                            if status != RA_BREAK {
                                reg_restore((*rp).rs_un.regsave_mut());
                                scan = (*rp).rs_scan;
                            }
                            if scan.is_null() || op_fn(scan) != BRANCH {
                                status = RA_NOMATCH;
                                regstack_pop(&raw mut scan);
                            } else {
                                (*rp).rs_scan = regnext(scan);
                                reg_save((*rp).rs_un.regsave_mut());
                                scan = operand_fn(scan);
                            }
                        }
                    }
                    RS_BRCPLX_MORE => {
                        if status == RA_NOMATCH {
                            reg_restore((*rp).rs_un.regsave_mut());
                            *(&raw mut re_st().brace_count)
                                .cast::<i32>()
                                .wrapping_add((*rp).rs_no as usize) -= 1;
                        }
                        regstack_pop(&raw mut scan);
                    }
                    RS_BRCPLX_LONG => {
                        if status == RA_NOMATCH {
                            reg_restore((*rp).rs_un.regsave_mut());
                            *(&raw mut re_st().brace_count)
                                .cast::<i32>()
                                .wrapping_add((*rp).rs_no as usize) -= 1;
                            status = RA_CONT;
                        }
                        regstack_pop(&raw mut scan);
                        if status == RA_CONT {
                            scan = regnext(scan);
                        }
                    }
                    RS_BRCPLX_SHORT => {
                        if status == RA_NOMATCH {
                            reg_restore((*rp).rs_un.regsave_mut());
                        }
                        regstack_pop(&raw mut scan);
                        if status == RA_NOMATCH {
                            scan = operand_fn(scan);
                            status = RA_CONT;
                        }
                    }
                    RS_NOMATCH => {
                        if status
                            == if (*rp).rs_no == NOMATCH as i16 {
                                RA_MATCH
                            } else {
                                RA_NOMATCH
                            }
                        {
                            status = RA_NOMATCH;
                        } else {
                            status = RA_CONT;
                            if (*rp).rs_no != SUBPAT as i16 {
                                reg_restore((*rp).rs_un.regsave_mut());
                            }
                        }
                        regstack_pop(&raw mut scan);
                        if status == RA_CONT {
                            scan = regnext(scan);
                        }
                    }
                    RS_BEHIND1 => {
                        if status == RA_NOMATCH {
                            regstack_pop(&raw mut scan);
                            re_st()
                                .regstack
                                .truncate(re_st().regstack.len() - size_of::<Regbehind>() / 8);
                        } else {
                            let bp = &mut *rp.cast::<Regbehind>().wrapping_sub(1);
                            reg_save(&mut bp.save_after);
                            bp.save_behind = re_st().behind_pos;
                            re_st().behind_pos = *(*rp).rs_un.regsave();
                            (*rp).rs_state = RS_BEHIND2;
                            reg_restore((*rp).rs_un.regsave_mut());
                            scan = operand_fn((*rp).rs_scan).wrapping_add(4);
                        }
                    }
                    RS_BEHIND2 => {
                        let bp = &mut *rp.cast::<Regbehind>().wrapping_sub(1);
                        if status == RA_MATCH && reg_save_equal(&re_st().behind_pos) {
                            re_st().behind_pos = bp.save_behind;
                            if (*rp).rs_no == BEHIND as i16 {
                                reg_restore(&mut bp.save_after);
                            } else {
                                status = RA_NOMATCH;
                                restore_subexpr(&mut *bp);
                            }
                            regstack_pop(&raw mut scan);
                            re_st()
                                .regstack
                                .truncate(re_st().regstack.len() - size_of::<Regbehind>() / 8);
                        } else {
                            let mut no = OK;
                            let limit = operand_min((*rp).rs_scan);
                            if reg_multi() {
                                let rp_pos = (*rp).rs_un.regsave().rs_u.pos();
                                let bpos = re_st().behind_pos.rs_u.pos();
                                if limit > 0
                                    && i64::from(
                                        (if rp_pos.lnum < bpos.lnum {
                                            crate::memory::cstr_len(re_st().rex.line.cast())
                                                as Colnr
                                        } else {
                                            bpos.col
                                        }) - rp_pos.col,
                                    ) >= limit
                                {
                                    no = FAIL;
                                } else if rp_pos.col == 0 {
                                    (*rp).rs_un.regsave_mut().rs_u.pos_mut().lnum -= 1;
                                    let new_lnum = (*rp).rs_un.regsave().rs_u.pos().lnum;
                                    if new_lnum + 1 < bpos.lnum || reg_getline(new_lnum).is_null() {
                                        no = FAIL;
                                    } else {
                                        reg_restore((*rp).rs_un.regsave_mut());
                                        (*rp).rs_un.regsave_mut().rs_u.pos_mut().col =
                                            crate::memory::cstr_len(re_st().rex.line.cast())
                                                as Colnr;
                                    }
                                } else {
                                    let line = reg_getline(rp_pos.lnum);
                                    (*rp).rs_un.regsave_mut().rs_u.pos_mut().col -= utf_head_off(
                                        line,
                                        line.wrapping_add(rp_pos.col as usize - 1),
                                    ) + 1;
                                }
                            } else if (*rp).rs_un.regsave().rs_u.ptr() == re_st().rex.line {
                                no = FAIL;
                            } else {
                                mb_ptr_back(
                                    re_st().rex.line,
                                    (*rp).rs_un.regsave_mut().rs_u.ptr_mut(),
                                );
                                if limit > 0
                                    && re_st()
                                        .behind_pos
                                        .rs_u
                                        .ptr()
                                        .offset_from((*rp).rs_un.regsave().rs_u.ptr())
                                        > limit as isize
                                {
                                    no = FAIL;
                                }
                            }
                            if no == OK {
                                reg_restore((*rp).rs_un.regsave_mut());
                                scan = operand_fn((*rp).rs_scan).wrapping_add(4);
                                if status == RA_MATCH {
                                    status = RA_NOMATCH;
                                    restore_subexpr(&mut *bp);
                                }
                            } else {
                                re_st().behind_pos = bp.save_behind;
                                if (*rp).rs_no == NOBEHIND as i16 {
                                    reg_restore(&mut bp.save_after);
                                    status = RA_MATCH;
                                } else if status == RA_MATCH {
                                    status = RA_NOMATCH;
                                    restore_subexpr(&mut *bp);
                                }
                                regstack_pop(&raw mut scan);
                                re_st()
                                    .regstack
                                    .truncate(re_st().regstack.len() - size_of::<Regbehind>() / 8);
                            }
                        }
                    }
                    RS_STAR_LONG | RS_STAR_SHORT => {
                        let rst = &mut *rp.cast::<Regstar>().wrapping_sub(1);
                        if status == RA_MATCH {
                            regstack_pop(&raw mut scan);
                            re_st()
                                .regstack
                                .truncate(re_st().regstack.len() - size_of::<Regstar>() / 8);
                        } else {
                            if status != RA_BREAK {
                                reg_restore((*rp).rs_un.regsave_mut());
                            }
                            loop {
                                if status != RA_BREAK {
                                    if (*rp).rs_state == RS_STAR_LONG {
                                        rst.count -= 1;
                                        if rst.count < rst.minval {
                                            break;
                                        }
                                        if re_st().rex.input == re_st().rex.line {
                                            if re_st().rex.lnum == 0 {
                                                status = RA_NOMATCH;
                                                break;
                                            }
                                            re_st().rex.lnum -= 1;
                                            re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
                                            if re_st().rex.line.is_null() {
                                                break;
                                            }
                                            re_st().rex.input =
                                                re_st()
                                                    .rex
                                                    .line
                                                    .wrapping_add(
                                                        reg_getline_len(re_st().rex.lnum) as usize,
                                                    );
                                            reg_breakcheck();
                                        } else {
                                            mb_ptr_back(
                                                re_st().rex.line,
                                                &raw mut re_st().rex.input,
                                            );
                                        }
                                    } else {
                                        if rst.count == rst.minval
                                            || regrepeat(operand_fn((*rp).rs_scan), 1) == 0
                                        {
                                            break;
                                        }
                                        rst.count += 1;
                                    }
                                    if g_st().got_int {
                                        break;
                                    }
                                } else {
                                    status = RA_NOMATCH;
                                }

                                if rst.nextb == NUL as i32
                                    || *re_st().rex.input as i32 == rst.nextb
                                    || *re_st().rex.input as i32 == rst.nextb_ic
                                {
                                    reg_save((*rp).rs_un.regsave_mut());
                                    scan = regnext((*rp).rs_scan);
                                    status = RA_CONT;
                                    break;
                                }
                            }
                            if status != RA_CONT {
                                regstack_pop(&raw mut scan);
                                re_st()
                                    .regstack
                                    .truncate(re_st().regstack.len() - size_of::<Regstar>() / 8);
                                status = RA_NOMATCH;
                            }
                        }
                    }
                    _ => {}
                }

                let top = re_st()
                    .regstack
                    .as_mut_ptr()
                    .wrapping_add(re_st().regstack.len())
                    .cast::<Regitem>()
                    .wrapping_sub(1);
                if status == RA_CONT || rp == top {
                    break;
                }
            }

            if status == RA_CONT {
                continue;
            }

            if re_st().regstack.is_empty() || status == RA_FAIL {
                if scan.is_null() {
                    iemsg(tr(E_RE_CORR));
                }
                return status == RA_MATCH;
            }
        }
    }
}

fn regtry(
    prog: &mut BtRegprog,
    col: Colnr,
    tm: Option<Proftime>,
    timed_out: Option<&mut i32>,
) -> i32 {
    unsafe {
        re_st().rex.input = re_st().rex.line.wrapping_add(col as usize);
        re_st().rex.need_clear_subexpr = 1;
        re_st().rex.need_clear_zsubexpr = i32::from(prog.reghasz == REX_SET as u8);

        if !regmatch(
            prog.program.as_mut_ptr().wrapping_add(1),
            tm,
            (timed_out.is_some()).then(|| &mut *timed_out.unwrap()),
        ) {
            return 0;
        }

        cleanup_subexpr();
        if reg_multi() {
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
            if (*re_st().rex.reg_startp).is_null() {
                *re_st().rex.reg_startp = re_st().rex.line.wrapping_add(col as usize);
            }
            if (*re_st().rex.reg_endp).is_null() {
                *re_st().rex.reg_endp = re_st().rex.input;
            }
        }

        g_st().re_extmatch_out = None;

        if prog.reghasz == REX_SET as u8 {
            cleanup_zsubexpr();
            let mut extmatch = RegExtmatch::default();
            for i in 0..NSUBEXP {
                if reg_multi() {
                    let sz = re_st().reg_startzpos[i];
                    let ez = re_st().reg_endzpos[i];
                    if sz.lnum >= 0 && ez.lnum == sz.lnum && ez.col >= sz.col {
                        extmatch.matches[i] = cstring_from_bytes(
                            reg_getline(sz.lnum).wrapping_add(sz.col as usize),
                            (ez.col - sz.col) as usize,
                        );
                    }
                } else {
                    let szp = re_st().reg_startzp[i];
                    let ezp = re_st().reg_endzp[i];
                    if !szp.is_null() && !ezp.is_null() {
                        extmatch.matches[i] =
                            cstring_from_bytes(szp.cast(), ezp.offset_from(szp) as usize);
                    }
                }
            }
            g_st().re_extmatch_out = Some(std::rc::Rc::new(extmatch));
        }
        1 + re_st().rex.lnum
    }
}

fn bt_regexec_both(
    mut line: *mut u8,
    startcol: Colnr,
    tm: Option<Proftime>,
    mut timed_out: Option<&mut i32>,
) -> i32 {
    unsafe {
        let mut col = startcol;
        let mut retval: i32 = 0;

        if re_st().regstack.capacity() == 0 {
            re_st().regstack.reserve((REGSTACK_INITIAL as usize) / 8);
        }

        if re_st().backpos.capacity() == 0 {
            re_st().backpos.reserve(BACKPOS_INITIAL as usize);
        }

        let prog: *mut BtRegprog = if reg_multi() {
            let p = re_st().rex.mmatch_mut().prog_mut().bt_mut();
            line = reg_getline(0).cast();
            re_st().rex.reg_startpos = (&raw mut re_st().rex.mmatch_mut().startpos).cast();
            re_st().rex.reg_endpos = (&raw mut re_st().rex.mmatch_mut().endpos).cast();
            p
        } else {
            let p = (*re_st().rex.match_mut().regprog).bt_mut();
            re_st().rex.reg_startp = (&raw mut re_st().rex.match_mut().startp).cast();
            re_st().rex.reg_endp = (&raw mut re_st().rex.match_mut().endp).cast();
            p
        };
        if prog.is_null() || line.is_null() {
            iemsg(tr(E_NULL));
            return finish_bt_regexec(retval, col);
        }
        let prog = &mut *prog;

        if prog_magic_wrong() {
            return finish_bt_regexec(retval, col);
        }

        if re_st().rex.reg_maxcol > 0 && col >= re_st().rex.reg_maxcol {
            return finish_bt_regexec(retval, col);
        }

        if (prog.regflags & RF_ICASE) != 0 {
            re_st().rex.reg_ic = true;
        } else if (prog.regflags & RF_NOICASE) != 0 {
            re_st().rex.reg_ic = false;
        }

        if (prog.regflags & RF_ICOMBINE) != 0 {
            re_st().rex.reg_icombine = true;
        }

        if let Some(off) = prog.regmust_off {
            let regmust = prog.program.as_ptr().wrapping_add(off).cast::<c_char>();
            let c = utf_ptr2char(regmust);
            let mut s = line.wrapping_add(col as usize);
            if !re_st().rex.reg_ic {
                loop {
                    s = vim_strchr(s.cast(), c).cast();
                    if s.is_null() {
                        break;
                    }
                    let mut len = prog.regmlen;
                    if cstrncmp(s.cast(), regmust.cast_mut(), &mut len) == 0 {
                        break;
                    }
                    mb_ptr_adv(&raw mut s);
                }
            } else {
                loop {
                    s = cstrchr(s.cast(), c).cast();
                    if s.is_null() {
                        break;
                    }
                    let mut len = prog.regmlen;
                    if cstrncmp(s.cast(), regmust.cast_mut(), &mut len) == 0 {
                        break;
                    }
                    mb_ptr_adv(&raw mut s);
                }
            }
            if s.is_null() {
                return finish_bt_regexec(retval, col);
            }
        }

        re_st().rex.line = line;
        re_st().rex.lnum = 0;
        re_st().reg_toolong = 0;

        if prog.reganch != 0 {
            let c = utf_ptr2char(re_st().rex.line.wrapping_add(col as usize).cast());
            if prog.regstart == NUL as i32
                || prog.regstart == c
                || (re_st().rex.reg_ic
                    && (utf_fold(prog.regstart) == utf_fold(c)
                        || (c < 255
                            && prog.regstart < 255
                            && mb_tolower(prog.regstart) == mb_tolower(c))))
            {
                retval = regtry(
                    &mut *prog,
                    col,
                    tm,
                    (timed_out.is_some()).then(|| &mut *timed_out.as_deref_mut().unwrap()),
                );
            } else {
                retval = 0;
            }
        } else {
            let mut tm_count: i32 = 0;
            while !g_st().got_int {
                if prog.regstart != NUL as i32 {
                    let s = cstrchr(
                        re_st().rex.line.wrapping_add(col as usize).cast(),
                        prog.regstart,
                    );
                    if s.is_null() {
                        retval = 0;
                        break;
                    }
                    col = s.cast::<u8>().offset_from(re_st().rex.line) as Colnr;
                }

                if re_st().rex.reg_maxcol > 0 && col >= re_st().rex.reg_maxcol {
                    retval = 0;
                    break;
                }

                retval = regtry(
                    &mut *prog,
                    col,
                    tm,
                    (timed_out.is_some()).then(|| &mut *timed_out.as_deref_mut().unwrap()),
                );
                if retval > 0 {
                    break;
                }

                if re_st().rex.lnum != 0 {
                    re_st().rex.lnum = 0;
                    re_st().rex.line = reg_getline(0).cast();
                }
                if *re_st().rex.line.wrapping_add(col as usize) == NUL {
                    break;
                }
                col += utfc_ptr2len(re_st().rex.line.wrapping_add(col as usize).cast());
                if let Some(tm_val) = tm {
                    tm_count += 1;
                    if tm_count == 20 {
                        tm_count = 0;
                        if profile_passed_limit(tm_val) {
                            if let Some(ref mut p) = timed_out {
                                **p = 1;
                            }
                            break;
                        }
                    }
                }
            }
        }

        finish_bt_regexec(retval, col)
    }
}

fn finish_bt_regexec(retval: i32, col: Colnr) -> i32 {
    if re_st().reg_tofree.capacity() > 400 {
        re_st().reg_tofree = Vec::new();
    }
    if re_st().regstack.capacity() * 8 > REGSTACK_INITIAL as usize {
        re_st().regstack = Vec::new();
    }
    if re_st().backpos.capacity() > BACKPOS_INITIAL as usize {
        re_st().backpos = Vec::new();
    }

    if retval > 0 {
        if reg_multi() {
            let start = re_st().rex.mmatch_mut().startpos[0];
            let end = re_st().rex.mmatch_mut().endpos[0];
            if end.lnum < start.lnum || (end.lnum == start.lnum && end.col < start.col) {
                re_st().rex.mmatch_mut().endpos[0] = start;
            }
            re_st().rex.mmatch_mut().rmm_matchcol = col;
        } else {
            if re_st().rex.match_mut().endp[0] < re_st().rex.match_mut().startp[0] {
                re_st().rex.match_mut().endp[0] = re_st().rex.match_mut().startp[0];
            }
            re_st().rex.match_mut().rm_matchcol = col;
        }
    }

    retval
}

pub(super) fn bt_regexec_nl(rmp: &mut Regmatch, line: *mut u8, col: Colnr, line_lbr: bool) -> i32 {
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

    bt_regexec_both(line, col, None, None)
}

pub(super) fn bt_regexec_multi(
    rmp: &mut Regmmatch,
    win: *mut WinInfoInfo,
    buf: *mut FileBuf,
    lnum: Linenr,
    col: Colnr,
    tm: Option<Proftime>,
    timed_out: Option<&mut i32>,
) -> i32 {
    init_regexec_multi(rmp, win, buf, lnum);
    bt_regexec_both(ptr::null_mut(), col, tm, timed_out)
}

fn re_num_cmp(val: u32, scan: *const u8) -> bool {
    let n = operand_min(scan) as u32;
    let cmp = operand_cmp(scan);
    if cmp == b'>' {
        val > n
    } else if cmp == b'<' {
        val < n
    } else {
        val == n
    }
}
