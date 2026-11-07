//! Backtracking regular expression engine (`src/nvim/regexp.c`).

use crate::message::semsg;
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

const JUST_CALC_SIZE: usize = usize::MAX;

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Backpos {
    bp_scan: usize,
    bp_pos: Regsave,
}

struct RegexpBtState {
    bl_minval: i64,
    bl_maxval: i64,
    reg_program: Vec<u8>,
    regcode_idx: usize,
}

impl RegexpBtState {
    const fn new() -> Self {
        Self {
            bl_minval: 0,
            bl_maxval: 0,
            reg_program: Vec::new(),
            regcode_idx: JUST_CALC_SIZE,
        }
    }
}

static BT_STATE: GlobalState<RegexpBtState> = GlobalState::new(RegexpBtState::new());

#[inline(always)]
fn bt_st() -> crate::globals::GraphPtr<RegexpBtState> {
    BT_STATE.handle()
}

#[inline(always)]
fn with_nl(op: i32) -> bool {
    (FIRST_NL..=LAST_NL).contains(&op)
}

#[inline(always)]
fn op_fn(prog: &[u8], p: Option<usize>) -> i32 {
    p.and_then(|idx| prog.get(idx)).map_or(0, |&b| i32::from(b))
}

#[inline(always)]
fn next_fn(prog: &[u8], idx: usize) -> i32 {
    ((i32::from(prog[idx + 1]) & 0o377) << 8) + (i32::from(prog[idx + 2]) & 0o377)
}

#[inline(always)]
fn operand_idx(idx: usize) -> usize {
    idx + 3
}

#[inline(always)]
fn operand_min(prog: &[u8], idx: usize) -> i64 {
    i64::from(i32::from_be_bytes([
        prog[idx + 3],
        prog[idx + 4],
        prog[idx + 5],
        prog[idx + 6],
    ]))
}

#[inline(always)]
fn operand_max(prog: &[u8], idx: usize) -> i64 {
    operand_min(prog, idx + 4)
}

#[inline(always)]
fn operand_cmp(prog: &[u8], idx: usize) -> u8 {
    prog[idx + 7]
}

#[inline(always)]
fn advance_reginput() {
    let step = utfc_ptr2len(re_st().rex.input) as usize;
    re_st().rex.input = re_st().rex.input.wrapping_add(step);
}

#[inline(always)]
fn input_byte() -> u8 {
    re_st().rex.input.first_byte()
}

fn bt_recognize_char_class(slice: &[u8]) -> i32 {
    let mut newl = false;
    let config = get_char_class_bits(slice, &mut newl);
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
    if bt_st().regcode_idx == JUST_CALC_SIZE {
        re_st().regsize += 1;
    } else {
        let idx = bt_st().regcode_idx;
        bt_st().reg_program[idx] = b as u8;
        bt_st().regcode_idx = idx + 1;
    }
}

fn regmbc(c: i32) {
    if bt_st().regcode_idx == JUST_CALC_SIZE {
        re_st().regsize += i64::from(utf_char2len(c));
    } else {
        let idx = bt_st().regcode_idx;
        let len = crate::mbyte::utf_char2slice(c, &mut bt_st().reg_program[idx..]);
        bt_st().regcode_idx = idx + len;
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

fn regnode(op: i32) -> Option<usize> {
    let ret = bt_st().regcode_idx;
    if ret == JUST_CALC_SIZE {
        re_st().regsize += 3;
    } else {
        let prog = &mut bt_st().reg_program;
        prog[ret] = op as u8;
        prog[ret + 1] = NUL;
        prog[ret + 2] = NUL;
        bt_st().regcode_idx = ret + 3;
    }
    Some(ret)
}

fn re_put_uint32(prog: &mut [u8], p: usize, val: u32) -> usize {
    prog[p..p + 4].copy_from_slice(&val.to_be_bytes());
    p + 4
}

fn regnext(prog: &[u8], p: Option<usize>) -> Option<usize> {
    let p = p?;
    if p == JUST_CALC_SIZE || re_st().reg_toolong != 0 {
        return None;
    }
    let offset = next_fn(prog, p);
    if offset == 0 {
        return None;
    }
    if op_fn(prog, Some(p)) == BACK {
        Some(p - offset as usize)
    } else {
        Some(p + offset as usize)
    }
}

fn regtail(p: Option<usize>, val: Option<usize>) {
    let Some(p) = p else {
        return;
    };
    if p == JUST_CALC_SIZE {
        return;
    }
    let val = val.unwrap();
    let mut scan = p;
    loop {
        let temp = regnext(&bt_st().reg_program, Some(scan));
        let Some(t) = temp else {
            break;
        };
        scan = t;
    }
    let offset = if op_fn(&bt_st().reg_program, Some(scan)) == BACK {
        (scan as isize - val as isize) as i32
    } else {
        (val as isize - scan as isize) as i32
    };
    if offset > 0xffff {
        re_st().reg_toolong = 1;
    } else {
        let prog = &mut bt_st().reg_program;
        prog[scan + 1] = (((offset as u32) >> 8) & 0o377) as u8;
        prog[scan + 2] = (offset & 0o377) as u8;
    }
}

fn regoptail(p: Option<usize>, val: Option<usize>) {
    let Some(p) = p else {
        return;
    };
    if p == JUST_CALC_SIZE {
        return;
    }
    let op = op_fn(&bt_st().reg_program, Some(p));
    if op != BRANCH && !(BRACE_COMPLEX..=BRACE_COMPLEX + 9).contains(&op) {
        return;
    }
    regtail(Some(operand_idx(p)), val);
}

fn reginsert(op: i32, opnd: Option<usize>) {
    if bt_st().regcode_idx == JUST_CALC_SIZE {
        re_st().regsize += 3;
        return;
    }
    let opnd = opnd.unwrap();
    let end = bt_st().regcode_idx;
    bt_st().regcode_idx = end + 3;
    let prog = &mut bt_st().reg_program;
    prog.copy_within(opnd..end, opnd + 3);
    prog[opnd] = op as u8;
    prog[opnd + 1] = NUL;
    prog[opnd + 2] = NUL;
}

fn reginsert_nr(op: i32, val: i64, opnd: Option<usize>) {
    if bt_st().regcode_idx == JUST_CALC_SIZE {
        re_st().regsize += 7;
        return;
    }
    let opnd = opnd.unwrap();
    let end = bt_st().regcode_idx;
    bt_st().regcode_idx = end + 7;
    let prog = &mut bt_st().reg_program;
    prog.copy_within(opnd..end, opnd + 7);
    prog[opnd] = op as u8;
    prog[opnd + 1] = NUL;
    prog[opnd + 2] = NUL;
    re_put_uint32(prog, opnd + 3, val as u32);
}

fn reginsert_limits(op: i32, minval: i64, maxval: i64, opnd: Option<usize>) {
    if bt_st().regcode_idx == JUST_CALC_SIZE {
        re_st().regsize += 11;
        return;
    }
    let opnd = opnd.unwrap();
    let end = bt_st().regcode_idx;
    bt_st().regcode_idx = end + 11;
    let prog = &mut bt_st().reg_program;
    prog.copy_within(opnd..end, opnd + 11);
    prog[opnd] = op as u8;
    prog[opnd + 1] = NUL;
    prog[opnd + 2] = NUL;
    let place = re_put_uint32(prog, opnd + 3, minval as u32);
    let place = re_put_uint32(prog, place, maxval as u32);
    regtail(Some(opnd), Some(place));
}

fn regatom(flagp: &mut i32) -> Option<usize> {
    let mut ret: Option<usize>;
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
        re_st().had_eol = true;
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
            re_st().had_eol = true;
            return regnode(EOL);
        }
        extra = ADD_NL;
        *flagp |= HASNL;
        if c == b'[' as i32 {
            goto_collection = true;
        } else {
            // fallthrough to character classes
            let p = vim_strchr(CLASSCHARS, no_magic(c))
                .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
                .cast::<u8>();
            if p.is_null() {
                emsg(tr(E_INVALID_USE_OF_UNDERSCORE));
                g_st().rc_did_emsg = true;
                return None;
            }
            if c == magic_fn(b'.' as i32) && utf_iscomposing_legacy(peekchr()) {
                c = getchr();
                goto_multibyte = true;
            } else {
                let idx = ((crate::strings::AsByteSlice::byte_addr(p) as isize)
                    - (crate::strings::AsByteSlice::byte_addr(CLASSCHARS.as_ptr().cast::<u8>())
                        as isize)) as usize;
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
            let p = vim_strchr(CLASSCHARS, no_magic(c))
                .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
                .cast::<u8>();
            if p.is_null() {
                emsg(tr(E_INVALID_USE_OF_UNDERSCORE));
                g_st().rc_did_emsg = true;
                return None;
            }
            if c == magic_fn(b'.' as i32) && utf_iscomposing_legacy(peekchr()) {
                c = getchr();
                goto_multibyte = true;
            } else {
                let idx = ((crate::strings::AsByteSlice::byte_addr(p) as isize)
                    - (crate::strings::AsByteSlice::byte_addr(CLASSCHARS.as_ptr().cast::<u8>())
                        as isize)) as usize;
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
                    crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    }),
                );
                g_st().rc_did_emsg = true;
                return None;
            }
            ret = reg(REG_PAREN, &mut flags);
            ret?;
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
                    crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    }),
                );
                g_st().rc_did_emsg = true;
                return None;
            }
            iemsg(tr(E_INTERNAL_ERROR_IN_REGEXP));
            g_st().rc_did_emsg = true;
            return None;
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
                crate::strings::ByteSlice::from_ptr(if magic_cond {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
                c,
            );
            g_st().rc_did_emsg = true;
            return None;
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
                return None;
            }
        } else if (magic_fn(b'1' as i32)..=magic_fn(b'9' as i32)).contains(&c) {
            let refnum = c - magic_fn(b'0' as i32);
            if !seen_endbrace(refnum) {
                return None;
            }
            return regnode(BACKREF + refnum);
        } else if c == magic_fn(b'z' as i32) {
            c = no_magic(getchr());
            match c as u8 {
                b'(' => {
                    if (g_st().reg_do_extmatch & REX_SET) == 0 {
                        emsg(tr(E_Z_NOT_ALLOWED));
                        g_st().rc_did_emsg = true;
                        return None;
                    }
                    if re_st().one_exactly {
                        semsg!(
                            tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                            crate::strings::ByteSlice::from_ptr(
                                if re_st().reg_magic == MAGIC_ALL {
                                    c"".as_ptr()
                                } else {
                                    c"\\".as_ptr()
                                }
                            ),
                        );
                        g_st().rc_did_emsg = true;
                        return None;
                    }
                    ret = reg(REG_ZPAREN, &mut flags);
                    ret?;
                    *flagp |= flags & (HASWIDTH | SPSTART | HASNL | HASLOOKBH);
                    re_st().re_has_z = REX_SET;
                    return ret;
                }
                b'1'..=b'9' => {
                    if (g_st().reg_do_extmatch & REX_USE) == 0 {
                        emsg(tr(E_Z1_NOT_ALLOWED));
                        g_st().rc_did_emsg = true;
                        return None;
                    }
                    ret = regnode(ZREF + c - b'0' as i32);
                    re_st().re_has_z = REX_USE;
                    return ret;
                }
                b's' => {
                    ret = regnode(MOPEN);
                    if !re_mult_next(c"\\zs") {
                        return None;
                    }
                    return ret;
                }
                b'e' => {
                    ret = regnode(MCLOSE);
                    if !re_mult_next(c"\\ze") {
                        return None;
                    }
                    return ret;
                }
                _ => {
                    emsg(tr(c"E68: Invalid character after \\z"));
                    g_st().rc_did_emsg = true;
                    return None;
                }
            }
        } else if c == magic_fn(b'%' as i32) {
            c = no_magic(getchr());
            match c as u8 {
                b'(' => {
                    if re_st().one_exactly {
                        semsg!(
                            tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                            crate::strings::ByteSlice::from_ptr(
                                if re_st().reg_magic == MAGIC_ALL {
                                    c"".as_ptr()
                                } else {
                                    c"\\".as_ptr()
                                }
                            ),
                        );
                        g_st().rc_did_emsg = true;
                        return None;
                    }
                    ret = reg(REG_NPAREN, &mut flags);
                    ret?;
                    *flagp |= flags & (HASWIDTH | SPSTART | HASNL | HASLOOKBH);
                    return ret;
                }
                b'^' => return regnode(RE_BOF),
                b'$' => return regnode(RE_EOF),
                b'#' => {
                    if re_st().regparse_byte() == b'='
                        && (b'0'..=b'2').contains(&re_st().regparse_byte_at(1))
                    {
                        semsg!(
                            tr(E_ATOM_ENGINE_MUST_BE_AT_START_OF_PATTERN),
                            i32::from(re_st().regparse_byte_at(1)),
                        );
                        return None;
                    }
                    return regnode(CURSOR);
                }
                b'V' => return regnode(RE_VISUAL),
                b'C' => return regnode(RE_COMPOSING),
                b'[' => {
                    if re_st().one_exactly {
                        semsg!(
                            tr(E_INVALID_ITEM_IN_STR_BRACKETS),
                            crate::strings::ByteSlice::from_ptr(
                                if re_st().reg_magic == MAGIC_ALL {
                                    c"".as_ptr()
                                } else {
                                    c"\\".as_ptr()
                                }
                            ),
                        );
                        g_st().rc_did_emsg = true;
                        return None;
                    }
                    let mut lastnode: Option<usize> = None;
                    ret = None;
                    loop {
                        c = getchr();
                        if c == b']' as i32 {
                            break;
                        }
                        if c == NUL as i32 {
                            semsg!(
                                tr(E_MISSING_SB),
                                crate::strings::ByteSlice::from_ptr(
                                    if re_st().reg_magic == MAGIC_ALL {
                                        c"".as_ptr()
                                    } else {
                                        c"\\".as_ptr()
                                    }
                                ),
                            );
                            g_st().rc_did_emsg = true;
                            return None;
                        }
                        let br = regnode(BRANCH);
                        if ret.is_none() {
                            ret = br;
                        } else {
                            regtail(lastnode, br);
                            if re_st().reg_toolong != 0 {
                                return None;
                            }
                        }
                        ungetchr();
                        re_st().one_exactly = true;
                        lastnode = regatom(flagp);
                        re_st().one_exactly = false;
                        lastnode?;
                    }
                    if ret.is_none() {
                        semsg!(
                            tr(E_EMPTY_SB),
                            crate::strings::ByteSlice::from_ptr(
                                if re_st().reg_magic == MAGIC_ALL {
                                    c"".as_ptr()
                                } else {
                                    c"\\".as_ptr()
                                }
                            ),
                        );
                        g_st().rc_did_emsg = true;
                        return None;
                    }
                    let lastbranch = regnode(BRANCH);
                    let mut br = regnode(NOTHING);
                    if ret != Some(JUST_CALC_SIZE) {
                        regtail(lastnode, br);
                        regtail(lastbranch, br);
                        br = ret;
                        while br != lastnode {
                            let br_idx = br.unwrap();
                            if op_fn(&bt_st().reg_program, br) == BRANCH {
                                regtail(br, lastbranch);
                                if re_st().reg_toolong != 0 {
                                    return None;
                                }
                                br = Some(operand_idx(br_idx));
                            } else {
                                br = regnext(&bt_st().reg_program, br);
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
                            crate::strings::ByteSlice::from_ptr(
                                if re_st().reg_magic == MAGIC_ALL {
                                    c"".as_ptr()
                                } else {
                                    c"\\".as_ptr()
                                }
                            ),
                        );
                        g_st().rc_did_emsg = true;
                        return None;
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
                            if ret == Some(JUST_CALC_SIZE) {
                                re_st().regsize += 2;
                            } else {
                                let idx = bt_st().regcode_idx;
                                bt_st().reg_program[idx] = c as u8;
                                bt_st().reg_program[idx + 1] = cmp as u8;
                                bt_st().regcode_idx = idx + 2;
                            }
                            return ret;
                        } else if (c == b'l' as i32 || c == b'c' as i32 || c == b'v' as i32)
                            && (cur || got_digit)
                        {
                            if cur && n != 0 {
                                semsg!(tr(E_REGEXP_NUMBER_AFTER_DOT_POS_SEARCH_CHR), no_magic(c),);
                                g_st().rc_did_emsg = true;
                                return None;
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
                                    let mut cursor = crate::globals::curwin_mut().w_cursor;
                                    getvvcol(
                                        crate::globals::curwin_mut(),
                                        &mut cursor,
                                        None,
                                        None,
                                        Some(&mut vcol),
                                        0,
                                    );
                                    crate::globals::curwin_mut().w_cursor = cursor;
                                    vcol += 1;
                                    n = vcol as u32;
                                }
                                ret = regnode(RE_VCOL);
                            }
                            if ret == Some(JUST_CALC_SIZE) {
                                re_st().regsize += 5;
                            } else {
                                let st = bt_st().borrow_mut();
                                let idx = re_put_uint32(&mut st.reg_program, st.regcode_idx, n);
                                st.reg_program[idx] = cmp as u8;
                                st.regcode_idx = idx + 1;
                            }
                            return ret;
                        }
                    }
                    semsg!(
                        tr(c"E71: Invalid character after %s%%"),
                        crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                            c"".as_ptr()
                        } else {
                            c"\\".as_ptr()
                        }),
                    );
                    g_st().rc_did_emsg = true;
                    return None;
                }
            }
        } else if c == magic_fn(b'[' as i32) {
            goto_collection = true;
        }
    }

    if goto_collection {
        let (len, is_closing, char_class) = {
            let re_ref = re_st().borrow();
            let start_bytes = re_ref.regparse_bytes();
            let rem_bytes = skip_anyof_slice(start_bytes);
            let len = start_bytes.len() - rem_bytes.len();
            let is_closing = rem_bytes.first() == Some(&b']');
            let char_class = if is_closing && extra == 0 {
                bt_recognize_char_class(&start_bytes[..len + 1])
            } else {
                0
            };
            (len, is_closing, char_class)
        };
        let lp_off = re_st().regparse_off + len;
        if is_closing {
            let mut startc: i32 = -1;
            if extra == 0 && char_class != 0 {
                ret = regnode(char_class);
                re_st().regparse_off = lp_off;
                skipchr();
                *flagp |= HASWIDTH | SIMPLE;
                return ret;
            }

            if re_st().regparse_byte() == b'^' {
                ret = regnode(ANYBUT + extra);
                re_st().regparse_off += 1;
            } else {
                ret = regnode(ANYOF + extra);
            }

            if re_st().regparse_byte() == b']' || re_st().regparse_byte() == b'-' {
                startc = i32::from(re_st().regparse_byte());
                regc(startc);
                re_st().regparse_off += 1;
            }

            while re_st().regparse_byte() != 0 && re_st().regparse_byte() != b']' {
                if re_st().regparse_byte() == b'-' {
                    re_st().regparse_off += 1;
                    if re_st().regparse_byte() == b']'
                        || re_st().regparse_byte() == 0
                        || startc == -1
                        || (re_st().regparse_byte() == b'\\' && re_st().regparse_byte_at(1) == b'n')
                    {
                        regc(b'-' as i32);
                        startc = b'-' as i32;
                    } else {
                        let mut endc: i32 = 0;
                        if re_st().regparse_byte() == b'[' {
                            endc = re_st().regparse_get_coll_element();
                        }
                        if endc == 0 {
                            let adv = {
                                let re_ref = re_st().borrow();
                                let start_bytes = re_ref.regparse_bytes();
                                let mut rem = start_bytes;
                                endc = mb_ptr2char_adv(&mut rem);
                                start_bytes.len() - rem.len()
                            };
                            re_st().regparse_off += adv;
                        }
                        if endc == b'\\' as i32 && re_st().reg_cpo_lit == 0 {
                            endc = coll_get_char();
                        }
                        if startc > endc {
                            emsg(tr(E_REVERSE_RANGE));
                            g_st().rc_did_emsg = true;
                            return None;
                        }
                        if utf_char2len(startc) > 1 || utf_char2len(endc) > 1 {
                            if endc > startc + 256 {
                                emsg(tr(E_LARGE_CLASS));
                                g_st().rc_did_emsg = true;
                                return None;
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
                } else if re_st().regparse_byte() == b'\\'
                    && (crate::strings::vim_haschr(
                        REGEXP_INRANGE,
                        i32::from(re_st().regparse_byte_at(1)),
                    ) || (re_st().reg_cpo_lit == 0
                        && crate::strings::vim_haschr(
                            REGEXP_ABBR,
                            i32::from(re_st().regparse_byte_at(1)),
                        )))
                {
                    re_st().regparse_off += 1;
                    if re_st().regparse_byte() == b'n' {
                        if let Some(ret_idx) = ret
                            && ret_idx != JUST_CALC_SIZE
                            && bt_st().reg_program[ret_idx] == ANYOF as u8
                        {
                            bt_st().reg_program[ret_idx] = (ANYOF + ADD_NL) as u8;
                            *flagp |= HASNL;
                        }
                        re_st().regparse_off += 1;
                        startc = -1;
                    } else if matches!(re_st().regparse_byte(), b'd' | b'o' | b'x' | b'u' | b'U') {
                        startc = coll_get_char();
                        if startc == i32::MAX {
                            emsg(tr(E_UNICODE_VAL_TOO_LARGE));
                            g_st().rc_did_emsg = true;
                            return None;
                        }
                        if startc == 0 {
                            regc(0x0a);
                        } else {
                            regmbc(startc);
                        }
                    } else {
                        startc = backslash_trans(i32::from(re_st().regparse_byte()));
                        re_st().regparse_off += 1;
                        regc(startc);
                    }
                } else if re_st().regparse_byte() == b'[' {
                    let mut c_class = re_st().regparse_get_char_class();
                    startc = -1;
                    match c_class {
                        CLASS_NONE => {
                            c_class = re_st().regparse_get_equi_class();
                            if c_class != 0 {
                                reg_equi_class(c_class);
                            } else {
                                c_class = re_st().regparse_get_coll_element();
                                if c_class != 0 {
                                    regmbc(c_class);
                                } else {
                                    startc = i32::from(re_st().regparse_byte());
                                    re_st().regparse_off += 1;
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
                    startc = utf_ptr2char(re_st().regparse_bytes());
                    let mut len = utfc_ptr2len(re_st().regparse_bytes());
                    if utf_char2len(startc) != len {
                        startc = -1;
                    }
                    while {
                        len -= 1;
                        len >= 0
                    } {
                        regc(i32::from(re_st().regparse_byte()));
                        re_st().regparse_off += 1;
                    }
                }
            }
            regc(NUL as i32);
            re_st().prevchr_len = 1;
            if re_st().regparse_byte() != b']' {
                emsg(tr(E_TOOMSBRA));
                g_st().rc_did_emsg = true;
                return None;
            }
            skipchr();
            *flagp |= HASWIDTH | SIMPLE;
            return ret;
        } else if re_st().reg_strict != 0 {
            semsg!(
                tr(E_MISSINGBRACKET),
                crate::strings::ByteSlice::from_ptr(if re_st().reg_magic > MAGIC_OFF {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
            );
            g_st().rc_did_emsg = true;
            return None;
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
            let next_ch = {
                let re_ref = re_st().borrow();
                let p_bytes = re_ref.regparse_bytes();
                let l = utf_ptr2len(p_bytes) as usize;
                if !utf_composinglike(p_bytes, p_bytes.get(l..).unwrap_or(&[]), Some(&mut state)) {
                    None
                } else {
                    Some(utf_ptr2char(p_bytes))
                }
            };
            let Some(ch) = next_ch else {
                break;
            };
            regmbc(ch);
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

fn regpiece(flagp: &mut i32) -> Option<usize> {
    let mut flags: i32 = 0;
    let mut minval: i32 = 0;
    let mut maxval: i32 = 0;

    let ret = regatom(&mut flags)?;

    let op = peekchr();
    if re_multi_type(op) == NOT_MULTI {
        *flagp = flags;
        return Some(ret);
    }
    *flagp = WORST | SPSTART | (flags & (HASNL | HASLOOKBH));

    skipchr();
    if op == magic_fn(b'*' as i32) {
        if (flags & SIMPLE) != 0 {
            reginsert(STAR, Some(ret));
        } else {
            reginsert(BRANCH, Some(ret));
            regoptail(Some(ret), regnode(BACK));
            regoptail(Some(ret), Some(ret));
            regtail(Some(ret), regnode(BRANCH));
            regtail(Some(ret), regnode(NOTHING));
        }
    } else if op == magic_fn(b'+' as i32) {
        if (flags & SIMPLE) != 0 {
            reginsert(PLUS, Some(ret));
        } else {
            let next = regnode(BRANCH);
            regtail(Some(ret), next);
            regtail(regnode(BACK), Some(ret));
            regtail(next, regnode(BRANCH));
            regtail(Some(ret), regnode(NOTHING));
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
                crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
            );
            g_st().rc_did_emsg = true;
            return None;
        }
        if lop == BEHIND || lop == NOBEHIND {
            regtail(Some(ret), regnode(BHPOS));
            *flagp |= HASLOOKBH;
        }
        regtail(Some(ret), regnode(END));
        if lop == BEHIND || lop == NOBEHIND {
            if nr < 0 {
                nr = 0;
            }
            reginsert_nr(lop, i64::from(nr as u32), Some(ret));
        } else {
            reginsert(lop, Some(ret));
        }
    } else if op == magic_fn(b'?' as i32) || op == magic_fn(b'=' as i32) {
        reginsert(BRANCH, Some(ret));
        regtail(Some(ret), regnode(BRANCH));
        let next = regnode(NOTHING);
        regtail(Some(ret), next);
        regoptail(Some(ret), next);
    } else if op == magic_fn(b'{' as i32) {
        if !read_limits(&mut minval, &mut maxval) {
            return None;
        }
        if (flags & SIMPLE) != 0 {
            reginsert(BRACE_SIMPLE, Some(ret));
            reginsert_limits(
                BRACE_LIMITS,
                i64::from(minval),
                i64::from(maxval),
                Some(ret),
            );
        } else {
            if re_st().num_complex_braces >= 10 {
                semsg!(
                    tr(c"E60: Too many complex %s{...}s"),
                    crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                        c"".as_ptr()
                    } else {
                        c"\\".as_ptr()
                    }),
                );
                g_st().rc_did_emsg = true;
                return None;
            }
            reginsert(BRACE_COMPLEX + re_st().num_complex_braces, Some(ret));
            regoptail(Some(ret), regnode(BACK));
            regoptail(Some(ret), Some(ret));
            reginsert_limits(
                BRACE_LIMITS,
                i64::from(minval),
                i64::from(maxval),
                Some(ret),
            );
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
                crate::strings::ByteSlice::from_ptr(if re_st().reg_magic >= MAGIC_ON {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
            );
            g_st().rc_did_emsg = true;
            return None;
        }
        semsg!(
            tr(c"E62: Nested %s%c"),
            crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                c"".as_ptr()
            } else {
                c"\\".as_ptr()
            }),
            no_magic(peekchr()),
        );
        g_st().rc_did_emsg = true;
        return None;
    }

    Some(ret)
}

fn regconcat(flagp: &mut i32) -> Option<usize> {
    let mut first: Option<usize> = None;
    let mut chain: Option<usize> = None;
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
            if latest.is_none() || re_st().reg_toolong != 0 {
                return None;
            }
            *flagp |= flags & (HASWIDTH | HASNL | HASLOOKBH);
            if chain.is_none() {
                *flagp |= flags & SPSTART;
            } else {
                regtail(chain, latest);
            }
            chain = latest;
            if first.is_none() {
                first = latest;
            }
        }
    }
    if first.is_none() {
        first = regnode(NOTHING);
    }
    first
}

fn regbranch(flagp: &mut i32) -> Option<usize> {
    let mut chain: Option<usize> = None;
    let mut flags: i32 = 0;

    *flagp = WORST | HASNL;

    let ret = regnode(BRANCH);
    loop {
        let latest = regconcat(&mut flags)?;
        *flagp |= flags & (HASWIDTH | SPSTART | HASLOOKBH);
        *flagp &= !HASNL | (flags & HASNL);
        if chain.is_some() {
            regtail(chain, Some(latest));
        }
        if peekchr() != magic_fn(b'&' as i32) {
            break;
        }
        skipchr();
        regtail(Some(latest), regnode(END));
        if re_st().reg_toolong != 0 {
            break;
        }
        reginsert(MATCH, Some(latest));
        chain = Some(latest);
    }
    ret
}

fn reg(paren: i32, flagp: &mut i32) -> Option<usize> {
    let mut ret: Option<usize>;
    let mut parno: i32 = 0;
    let mut flags: i32 = 0;

    *flagp = HASWIDTH;

    if paren == REG_ZPAREN {
        if re_st().regnzpar >= NSUBEXP as i32 {
            emsg(tr(c"E50: Too many \\z("));
            g_st().rc_did_emsg = true;
            return None;
        }
        parno = re_st().regnzpar;
        re_st().regnzpar += 1;
        ret = regnode(ZOPEN + parno);
    } else if paren == REG_PAREN {
        if re_st().regnpar >= NSUBEXP as i32 {
            semsg!(
                tr(c"E51: Too many %s("),
                crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
            );
            g_st().rc_did_emsg = true;
            return None;
        }
        parno = re_st().regnpar;
        re_st().regnpar += 1;
        ret = regnode(MOPEN + parno);
    } else if paren == REG_NPAREN {
        ret = regnode(NOPEN);
    } else {
        ret = None;
    }

    if re_st().bt_reg_parse_depth >= REG_MAX_PAREN_DEPTH {
        emsg(tr(E_COMMAND_TOO_COMPLEX));
        g_st().rc_did_emsg = true;
        return None;
    }
    re_st().bt_reg_parse_depth += 1;

    let mut br = regbranch(&mut flags);
    if br.is_none() {
        re_st().bt_reg_parse_depth -= 1;
        return None;
    }
    if ret.is_some() {
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
        if br.is_none() || re_st().reg_toolong != 0 {
            re_st().bt_reg_parse_depth -= 1;
            return None;
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
    while br.is_some() {
        regoptail(br, ender);
        br = regnext(&bt_st().reg_program, br);
    }

    if paren != REG_NOPAREN && getchr() != magic_fn(b')' as i32) {
        if paren == REG_ZPAREN {
            emsg(tr(E_UNMATCHED_Z));
        } else if paren == REG_NPAREN {
            semsg!(
                tr(E_UNMATCHED_STR_PERCENT_OPEN),
                crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
            );
        } else {
            semsg!(
                tr(E_UNMATCHED_STR_OPEN),
                crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
            );
        }
        g_st().rc_did_emsg = true;
        re_st().bt_reg_parse_depth -= 1;
        return None;
    } else if paren == REG_NOPAREN && peekchr() != NUL as i32 {
        if re_st().curchr == magic_fn(b')' as i32) {
            semsg!(
                tr(E_UNMATCHED_STR_CLOSE),
                crate::strings::ByteSlice::from_ptr(if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                }),
            );
        } else {
            emsg(tr(E_TRAILING));
        }
        g_st().rc_did_emsg = true;
        re_st().bt_reg_parse_depth -= 1;
        return None;
    }

    if paren == REG_PAREN {
        re_st().had_endbrace[parno as usize] = 1;
    }

    re_st().bt_reg_parse_depth -= 1;
    ret
}

pub(super) fn bt_regcomp(expr: &CStr, re_flags: i32) -> Option<BtRegprog> {
    let mut flags: i32 = 0;

    init_class_tab();

    regcomp_start(expr, re_flags);
    bt_st().regcode_idx = JUST_CALC_SIZE;
    regc(REGMAGIC);
    reg(REG_NOPAREN, &mut flags)?;

    bt_st().reg_program = vec![0u8; re_st().regsize as usize];
    regcomp_start(expr, re_flags);
    bt_st().regcode_idx = 0;
    regc(REGMAGIC);
    if reg(REG_NOPAREN, &mut flags).is_none() || re_st().reg_toolong != 0 {
        bt_st().reg_program.clear();
        if re_st().reg_toolong != 0 {
            emsg(tr(c"E339: Pattern too long"));
            g_st().rc_did_emsg = true;
        }
        return None;
    }

    let mut r = BtRegprog {
        regflags: re_st().regflags,
        regstart: NUL as i32,
        reganch: 0,
        regmust_off: None,
        regmlen: 0,
        reghasz: re_st().re_has_z as u8,
        program: std::mem::take(&mut bt_st().reg_program),
    };
    if (flags & HASNL) != 0 {
        r.regflags |= RF_HASNL;
    }
    if (flags & HASLOOKBH) != 0 {
        r.regflags |= RF_LOOKBH;
    }

    let mut scan = Some(1usize);
    if op_fn(&r.program, regnext(&r.program, scan)) == END {
        scan = Some(operand_idx(1));

        if op_fn(&r.program, scan) == BOL || op_fn(&r.program, scan) == RE_BOF {
            r.reganch += 1;
            scan = regnext(&r.program, scan);
        }

        if let Some(s_idx) = scan {
            if op_fn(&r.program, scan) == EXACTLY {
                r.regstart = crate::mbyte::utf_slice2char(&r.program[operand_idx(s_idx)..]);
            } else if matches!(
                op_fn(&r.program, scan),
                BOW | EOW | NOTHING | MOPEN | NOPEN | MCLOSE | NCLOSE
            ) {
                let regnext_scan = regnext(&r.program, scan);
                if let Some(rn_idx) = regnext_scan
                    && op_fn(&r.program, regnext_scan) == EXACTLY
                {
                    r.regstart = crate::mbyte::utf_slice2char(&r.program[operand_idx(rn_idx)..]);
                }
            }
        }

        if ((flags & SPSTART) != 0
            || op_fn(&r.program, scan) == BOW
            || op_fn(&r.program, scan) == EOW)
            && (flags & HASNL) == 0
        {
            let mut longest: Option<usize> = None;
            let mut len: i32 = 0;
            while let Some(s_idx) = scan {
                if op_fn(&r.program, scan) == EXACTLY {
                    let opnd = operand_idx(s_idx);
                    let scanlen = crate::memory::bytes_until_nul(&r.program[opnd..]).len();
                    if scanlen >= len as usize {
                        longest = Some(opnd);
                        len = scanlen as i32;
                    }
                }
                scan = regnext(&r.program, scan);
            }
            r.regmust_off = longest;
            r.regmlen = len;
        }
    }
    Some(r)
}

fn reg_save(save: &mut Regsave) {
    let len = re_st().backpos.len() as i32;
    if reg_multi() {
        save.rs_u = crate::regexp::RegPos::from_pos(Lpos {
            col: re_st().rex.input_col(),
            lnum: re_st().rex.lnum,
        });
    } else {
        save.rs_u.ptr = re_st().rex.input;
    }
    save.rs_len = len;
}

fn reg_restore(save: &mut Regsave) {
    if reg_multi() {
        let pos = save.rs_u.pos();
        if re_st().rex.lnum != pos.lnum {
            re_st().rex.lnum = pos.lnum;
            re_st().rex.load_line(pos.lnum);
        }
        re_st().rex.input = re_st().rex.line.wrapping_add(pos.col as usize);
    } else {
        re_st().rex.input = save.rs_u.ptr;
    }
    re_st().backpos.truncate(save.rs_len as usize);
}

fn reg_save_equal(save: &Regsave) -> bool {
    if reg_multi() {
        let pos = save.rs_u.pos();
        re_st().rex.lnum == pos.lnum
            && re_st().rex.input.addr() == re_st().rex.line.wrapping_add(pos.col as usize).addr()
    } else {
        re_st().rex.input.addr() == save.rs_u.ptr.addr()
    }
}

fn save_se_multi(savep: &mut SaveSe, posp: &mut Lpos) {
    savep.se_u = crate::regexp::RegPos::from_pos(*posp);
    posp.lnum = re_st().rex.lnum;
    posp.col = re_st().rex.input_col();
}

#[inline(always)]
fn save_se_z(savep: &mut SaveSe, no: usize, is_end: bool) {
    if reg_multi() {
        let posp = if is_end {
            &mut re_st().reg_endzpos[no]
        } else {
            &mut re_st().reg_startzpos[no]
        };
        save_se_multi(savep, posp);
    } else {
        let pp = if is_end {
            &mut re_st().reg_endzp[no]
        } else {
            &mut re_st().reg_startzp[no]
        };
        savep.se_u.ptr = *pp;
        *pp = re_st().rex.input;
    }
}

#[inline(always)]
fn restore_se_z(savep: &SaveSe, no: usize, is_end: bool) {
    if reg_multi() {
        if is_end {
            re_st().reg_endzpos[no] = savep.se_u.pos();
        } else {
            re_st().reg_startzpos[no] = savep.se_u.pos();
        }
    } else if is_end {
        re_st().reg_endzp[no] = savep.se_u.ptr;
    } else {
        re_st().reg_startzp[no] = savep.se_u.ptr;
    }
}

#[inline(always)]
fn save_se_m(savep: &mut SaveSe, no: usize, is_end: bool) {
    if reg_multi() {
        let slice = if is_end {
            re_st().rex.endpos_slice_mut()
        } else {
            re_st().rex.startpos_slice_mut()
        };
        save_se_multi(savep, &mut slice[no]);
    } else {
        let pp = if is_end {
            &mut re_st().rex.match_mut().endp[no]
        } else {
            &mut re_st().rex.match_mut().startp[no]
        };
        savep.se_u.ptr = *pp;
        *pp = re_st().rex.input;
    }
}

#[inline(always)]
fn restore_se_m(savep: &SaveSe, no: usize, is_end: bool) {
    if reg_multi() {
        let slice = if is_end {
            re_st().rex.endpos_slice_mut()
        } else {
            re_st().rex.startpos_slice_mut()
        };
        slice[no] = savep.se_u.pos();
    } else if is_end {
        re_st().rex.match_mut().endp[no] = savep.se_u.ptr;
    } else {
        re_st().rex.match_mut().startp[no] = savep.se_u.ptr;
    }
}

fn regrepeat(prog: &[u8], p: usize, maxcount: i64) -> i32 {
    let mut count: i64 = 0;
    let mut testval: i16 = 0;
    let mut scan = re_st().rex.input;
    let opnd = &prog[operand_idx(p)..];
    let op = op_fn(prog, Some(p));

    let do_class =
        |mask: i16, testval: i16, scan: &mut crate::regexp::RegLineSlice, count: &mut i64| {
            while *count < maxcount {
                let b0 = (*scan).first_byte();
                if b0 == NUL {
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
                    let l = utfc_ptr2len(*scan);
                    if l > 1 {
                        if testval != 0 {
                            break;
                        }
                        *scan = (*scan).wrapping_add(l as usize);
                    } else if ((re_st().class_tab[b0 as usize] & mask) == testval)
                        || (re_st().rex.reg_line_lbr && b0 == b'\n' && with_nl(op))
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
            while (scan).first_byte() != NUL && count < maxcount {
                count += 1;
                let step = utfc_ptr2len(scan) as usize;
                scan = scan.wrapping_add(step);
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
            let b0 = (scan).first_byte();
            if vim_is_idc(utf_ptr2char(scan)) && (testval != 0 || !ascii_isdigit(b0 as i32)) {
                let step = utfc_ptr2len(scan) as usize;
                scan = scan.wrapping_add(step);
            } else if b0 == NUL {
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
            } else if re_st().rex.reg_line_lbr && b0 == b'\n' && with_nl(op) {
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
            let b0 = (scan).first_byte();
            if vim_iswordp_buf((scan).as_bytes(), re_st().rex.buf_mut())
                && (testval != 0 || !ascii_isdigit(b0 as i32))
            {
                let step = utfc_ptr2len(scan) as usize;
                scan = scan.wrapping_add(step);
            } else if b0 == NUL {
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
            } else if re_st().rex.reg_line_lbr && b0 == b'\n' && with_nl(op) {
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
            let b0 = (scan).first_byte();
            if vim_isfilec(utf_ptr2char(scan)) && (testval != 0 || !ascii_isdigit(b0 as i32)) {
                let step = utfc_ptr2len(scan) as usize;
                scan = scan.wrapping_add(step);
            } else if b0 == NUL {
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
            } else if re_st().rex.reg_line_lbr && b0 == b'\n' && with_nl(op) {
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
            let b0 = (scan).first_byte();
            if b0 == NUL {
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
            } else if vim_isprintc(utf_ptr2char(scan))
                && (testval != 0 || !ascii_isdigit(b0 as i32))
            {
                let step = utfc_ptr2len(scan) as usize;
                scan = scan.wrapping_add(step);
            } else if re_st().rex.reg_line_lbr && b0 == b'\n' && with_nl(op) {
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
        let opnd_b0 = opnd[0];
        if re_st().rex.reg_ic {
            let cu = mb_toupper(opnd_b0 as i32);
            let cl = mb_tolower(opnd_b0 as i32);
            while count < maxcount {
                let sb = (scan).first_byte() as i32;
                if sb != cu && sb != cl {
                    break;
                }
                count += 1;
                scan = scan.wrapping_add(1);
            }
        } else {
            while count < maxcount && (scan).first_byte() == opnd_b0 {
                count += 1;
                scan = scan.wrapping_add(1);
            }
        }
    } else if op == MULTIBYTECODE {
        let len = crate::mbyte::utfc_slice2len(opnd);
        if len > 1 {
            let cf = if re_st().rex.reg_ic {
                utf_fold(crate::mbyte::utf_slice2char(opnd))
            } else {
                0
            };
            while count < maxcount && (utfc_ptr2len(scan) as usize) >= len {
                let scan_slice = (scan).slice(len);
                if scan_slice != &opnd[..len]
                    && (!re_st().rex.reg_ic || utf_fold(utf_ptr2char(scan)) != cf)
                {
                    break;
                }
                scan = scan.wrapping_add(len);
                count += 1;
            }
        }
    } else if op == ANYOF || op == ANYOF + ADD_NL || op == ANYBUT || op == ANYBUT + ADD_NL {
        let is_anyof = op == ANYOF || op == ANYOF + ADD_NL;
        let opnd_cstr = crate::memory::bytes_as_cstr(opnd);
        while count < maxcount {
            let b0 = (scan).first_byte();
            if b0 == NUL {
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
            } else if re_st().rex.reg_line_lbr && b0 == b'\n' && with_nl(op) {
                scan = scan.wrapping_add(1);
            } else {
                let len = utfc_ptr2len(scan);
                if len > 1 {
                    if cstrchr(opnd_cstr, utf_ptr2char(scan)).is_none() == is_anyof {
                        break;
                    }
                    scan = scan.wrapping_add(len as usize);
                } else {
                    if cstrchr(opnd_cstr, b0 as i32).is_none() == is_anyof {
                        break;
                    }
                    scan = scan.wrapping_add(1);
                }
            }
            count += 1;
        }
    } else if op == NEWL {
        while count < maxcount {
            let b0 = (scan).first_byte();
            if !((b0 == NUL
                && re_st().rex.lnum <= re_st().rex.reg_maxline
                && !re_st().rex.reg_line_lbr
                && reg_multi())
                || (b0 == b'\n' && re_st().rex.reg_line_lbr))
            {
                break;
            }
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

fn regstack_push(state: Regstate, scan: Option<usize>) -> Option<&'static mut Regitem> {
    const {
        assert!(size_of::<Regitem>().is_multiple_of(8));
        assert!(size_of::<Regstar>().is_multiple_of(8));
        assert!(size_of::<Regbehind>().is_multiple_of(8));
    }
    if i64::from(((re_st().regstack.len() * 8) as u32) >> 10) >= opt_vars().p_mmp {
        emsg(tr(E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN));
        return None;
    }
    let old_len = re_st().regstack.len();
    re_st()
        .regstack
        .resize(old_len + size_of::<Regitem>() / 8, 0);

    let rp = crate::globals::GraphPtr::of(
        re_st()
            .regstack
            .as_mut_ptr()
            .wrapping_add(old_len)
            .cast::<Regitem>(),
    )
    .borrow_mut();
    rp.rs_state = state;
    rp.rs_scan = scan;
    Some(rp)
}

fn regstack_pop(scan: &mut Option<usize>) {
    let rp = crate::globals::GraphPtr::of(
        re_st()
            .regstack
            .as_ptr()
            .wrapping_add(re_st().regstack.len())
            .cast::<Regitem>()
            .wrapping_sub(1),
    )
    .borrow();
    *scan = rp.rs_scan;
    re_st()
        .regstack
        .truncate(re_st().regstack.len() - size_of::<Regitem>() / 8);
}

fn save_subexpr(bp: &mut Regbehind) {
    bp.save_need_clear_subexpr = re_st().rex.need_clear_subexpr;
    if re_st().rex.need_clear_subexpr {
        return;
    }
    if reg_multi() {
        let startpos = re_st().rex.startpos_slice_mut();
        let endpos = re_st().rex.endpos_slice_mut();
        for i in 0..NSUBEXP {
            bp.save_start[i].se_u = crate::regexp::RegPos::from_pos(startpos[i]);
            bp.save_end[i].se_u = crate::regexp::RegPos::from_pos(endpos[i]);
        }
    } else {
        let m = re_st().rex.match_mut();
        for i in 0..NSUBEXP {
            bp.save_start[i].se_u.ptr = m.startp[i];
            bp.save_end[i].se_u.ptr = m.endp[i];
        }
    }
}

fn restore_subexpr(bp: &mut Regbehind) {
    re_st().rex.need_clear_subexpr = bp.save_need_clear_subexpr;
    if re_st().rex.need_clear_subexpr {
        return;
    }
    if reg_multi() {
        let startpos = re_st().rex.startpos_slice_mut();
        let endpos = re_st().rex.endpos_slice_mut();
        for i in 0..NSUBEXP {
            startpos[i] = bp.save_start[i].se_u.pos();
            endpos[i] = bp.save_end[i].se_u.pos();
        }
    } else {
        let m = re_st().rex.match_mut();
        for i in 0..NSUBEXP {
            m.startp[i] = bp.save_start[i].se_u.ptr;
            m.endp[i] = bp.save_end[i].se_u.ptr;
        }
    }
}

fn regmatch(
    prog: &[u8],
    mut scan: Option<usize>,
    tm: Option<Proftime>,
    mut timed_out: Option<&mut i32>,
) -> bool {
    let mut status: RegAction;
    let mut tm_count: i32 = 0;

    re_st().regstack.clear();
    re_st().backpos.clear();

    loop {
        reg_breakcheck();

        loop {
            let Some(scan_idx) = scan else {
                status = RegAction::Fail;
                break;
            };
            if g_st().got_int {
                status = RegAction::Fail;
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
                        status = RegAction::Fail;
                        break;
                    }
                }
            }
            status = RegAction::Cont;

            let mut next = regnext(prog, scan);
            let mut op = op_fn(prog, scan);

            if !re_st().rex.reg_line_lbr
                && with_nl(op)
                && reg_multi()
                && input_byte() == NUL
                && re_st().rex.lnum <= re_st().rex.reg_maxline
            {
                reg_nextline();
            } else if re_st().rex.reg_line_lbr && with_nl(op) && input_byte() == b'\n' {
                advance_reginput();
            } else {
                if with_nl(op) {
                    op -= ADD_NL;
                }
                let c = utf_ptr2char(re_st().rex.input);
                match op {
                    BOL => {
                        if re_st().rex.input.addr() != re_st().rex.line.addr() {
                            status = RegAction::NoMatch;
                        }
                    }
                    EOL => {
                        if c != NUL as i32 {
                            status = RegAction::NoMatch;
                        }
                    }
                    RE_BOF => {
                        if re_st().rex.lnum != 0
                            || re_st().rex.input.addr() != re_st().rex.line.addr()
                            || (reg_multi() && re_st().rex.reg_firstlnum > 1)
                        {
                            status = RegAction::NoMatch;
                        }
                    }
                    RE_EOF => {
                        if re_st().rex.lnum != re_st().rex.reg_maxline || c != NUL as i32 {
                            status = RegAction::NoMatch;
                        }
                    }
                    CURSOR => {
                        if re_st().rex.reg_win.is_null()
                            || (re_st().rex.lnum + re_st().rex.reg_firstlnum
                                != re_st().rex.win_mut().w_cursor.lnum)
                            || (re_st().rex.input_col() != re_st().rex.win_mut().w_cursor.col)
                        {
                            status = RegAction::NoMatch;
                        }
                    }
                    RE_MARK => {
                        let opnd = operand_idx(scan_idx);
                        let mark = i32::from(prog[opnd]);
                        let cmp = i32::from(prog[opnd + 1]);
                        let col = if reg_multi() {
                            re_st().rex.input_off()
                        } else {
                            0
                        };
                        let fm = mark_get(
                            re_st().rex.buf_mut(),
                            crate::globals::curwin_mut(),
                            None,
                            K_MARK_BUF_LOCAL,
                            mark,
                        );
                        if reg_multi() {
                            let cur_lnum = re_st().rex.lnum;
                            re_st().rex.load_line(cur_lnum);
                            re_st().rex.input = re_st().rex.line.wrapping_add(col);
                        }
                        if fm.as_ref().is_none_or(|f| f.mark.lnum <= 0) {
                            status = RegAction::NoMatch;
                        } else {
                            let pos = fm.unwrap().mark;
                            let pos_col = if pos.lnum
                                == re_st().rex.lnum + re_st().rex.reg_firstlnum
                                && pos.col == MAXCOL
                            {
                                reg_getline_len(pos.lnum - re_st().rex.reg_firstlnum)
                            } else {
                                pos.col
                            };
                            let cur_col = re_st().rex.input_col();
                            let nomatch =
                                if pos.lnum == re_st().rex.lnum + re_st().rex.reg_firstlnum {
                                    if pos_col == cur_col {
                                        cmp == b'<' as i32 || cmp == b'>' as i32
                                    } else if pos_col < cur_col {
                                        cmp != b'>' as i32
                                    } else {
                                        cmp != b'<' as i32
                                    }
                                } else if pos.lnum < re_st().rex.lnum + re_st().rex.reg_firstlnum {
                                    cmp != b'>' as i32
                                } else {
                                    cmp != b'<' as i32
                                };
                            if nomatch {
                                status = RegAction::NoMatch;
                            }
                        }
                    }
                    RE_VISUAL => {
                        if !reg_match_visual() {
                            status = RegAction::NoMatch;
                        }
                    }
                    RE_LNUM => {
                        if !reg_multi()
                            || !re_num_cmp(
                                (re_st().rex.lnum + re_st().rex.reg_firstlnum) as u32,
                                prog,
                                scan_idx,
                            )
                        {
                            status = RegAction::NoMatch;
                        }
                    }
                    RE_COL => {
                        if !re_num_cmp((re_st().rex.input_off() + 1) as u32, prog, scan_idx) {
                            status = RegAction::NoMatch;
                        }
                    }
                    RE_VCOL => {
                        let wp = if re_st().rex.reg_win.is_null() {
                            crate::globals::curwin_mut()
                        } else {
                            re_st().rex.win_mut()
                        };
                        let mut lnum = if reg_multi() {
                            re_st().rex.reg_firstlnum + re_st().rex.lnum
                        } else {
                            1
                        };
                        if reg_multi() && (lnum <= 0 || lnum > wp.buf_mut().b_ml.ml_line_count) {
                            lnum = 1;
                        }
                        let vcol = win_linetabsize(
                            wp,
                            lnum,
                            re_st().rex.line.as_cstr(),
                            re_st().rex.input_col(),
                        );
                        if !re_num_cmp((vcol as u32).wrapping_add(1), prog, scan_idx) {
                            status = RegAction::NoMatch;
                        }
                    }
                    BOW => {
                        if c == NUL as i32 {
                            status = RegAction::NoMatch;
                        } else {
                            let this_class = mb_get_class_tab(
                                re_st().rex.input.unwrap_or(&[]),
                                &re_st().rex.buf_mut().b_chartab,
                            );
                            if this_class <= 1 || reg_prev_class() == this_class {
                                status = RegAction::NoMatch;
                            }
                        }
                    }
                    EOW => {
                        if re_st().rex.input.addr() == re_st().rex.line.addr() {
                            status = RegAction::NoMatch;
                        } else {
                            let this_class = mb_get_class_tab(
                                re_st().rex.input.unwrap_or(&[]),
                                &re_st().rex.buf_mut().b_chartab,
                            );
                            let prev_class = reg_prev_class();
                            if this_class == prev_class || prev_class == 0 || prev_class == 1 {
                                status = RegAction::NoMatch;
                            }
                        }
                    }
                    ANY => {
                        if c == NUL as i32 {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    IDENT => {
                        if !vim_is_idc(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    SIDENT => {
                        if ascii_isdigit(input_byte() as i32) || !vim_is_idc(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    KWORD => {
                        if !vim_iswordp_buf(re_st().rex.input.as_bytes(), re_st().rex.buf_mut()) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    SKWORD => {
                        if ascii_isdigit(input_byte() as i32)
                            || !vim_iswordp_buf(re_st().rex.input.as_bytes(), re_st().rex.buf_mut())
                        {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    FNAME => {
                        if !vim_isfilec(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    SFNAME => {
                        if ascii_isdigit(input_byte() as i32) || !vim_isfilec(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    PRINT => {
                        if !vim_isprintc(utf_ptr2char(re_st().rex.input)) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    SPRINT => {
                        if ascii_isdigit(input_byte() as i32)
                            || !vim_isprintc(utf_ptr2char(re_st().rex.input))
                        {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    WHITE => {
                        if !ascii_iswhite(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NWHITE => {
                        if c == NUL as i32 || ascii_iswhite(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    DIGIT => {
                        if !ri_digit(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NDIGIT => {
                        if c == NUL as i32 || ri_digit(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    HEX => {
                        if !ri_hex(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NHEX => {
                        if c == NUL as i32 || ri_hex(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    OCTAL => {
                        if !ri_octal(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NOCTAL => {
                        if c == NUL as i32 || ri_octal(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    WORD => {
                        if !ri_word(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NWORD => {
                        if c == NUL as i32 || ri_word(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    HEAD => {
                        if !ri_head(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NHEAD => {
                        if c == NUL as i32 || ri_head(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    ALPHA => {
                        if !ri_alpha(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NALPHA => {
                        if c == NUL as i32 || ri_alpha(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    LOWER => {
                        if !ri_lower(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NLOWER => {
                        if c == NUL as i32 || ri_lower(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    UPPER => {
                        if !ri_upper(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    NUPPER => {
                        if c == NUL as i32 || ri_upper(c) {
                            status = RegAction::NoMatch;
                        } else {
                            advance_reginput();
                        }
                    }
                    EXACTLY => {
                        let opnd = &prog[operand_idx(scan_idx)..];
                        let opnd_bytes = crate::memory::bytes_until_nul(opnd);
                        if opnd[0] != input_byte() && !re_st().rex.reg_ic {
                            status = RegAction::NoMatch;
                        } else if !opnd_bytes.is_empty() {
                            let len = if opnd_bytes.len() == 1 && !re_st().rex.reg_ic {
                                1
                            } else {
                                let mut l = opnd_bytes.len() as i32;
                                if cstrncmp(opnd_bytes, re_st().rex.input.as_bytes(), &mut l) != 0 {
                                    status = RegAction::NoMatch;
                                }
                                l
                            };
                            if status != RegAction::NoMatch
                                && utf_composinglike(
                                    re_st().rex.input.as_bytes(),
                                    re_st().rex.input.wrapping_add(len as usize).as_bytes(),
                                    None,
                                )
                                && !re_st().rex.reg_icombine
                                && op_fn(prog, next) != RE_COMPOSING
                            {
                                status = RegAction::NoMatch;
                            }
                            if status != RegAction::NoMatch {
                                re_st().rex.input = re_st().rex.input.wrapping_add(len as usize);
                            }
                        }
                    }
                    ANYOF | ANYBUT => {
                        let q_slice = &prog[operand_idx(scan_idx)..];
                        let q_cstr = crate::memory::bytes_as_cstr(q_slice);
                        let matched = cstrchr(q_cstr, c);
                        if c == NUL as i32 || (matched.is_none() == (op == ANYOF)) {
                            status = RegAction::NoMatch;
                        } else {
                            let q_utfc = crate::mbyte::utfc_slice2len(q_slice);
                            let q_utf = crate::mbyte::utf_slice2len(q_slice);
                            let len = q_utfc - q_utf;
                            re_st().rex.input = re_st()
                                .rex
                                .input
                                .wrapping_add(utf_ptr2len(re_st().rex.input) as usize);
                            if len != 0 {
                                let q_comp = &q_slice[q_utf..q_utf + len];
                                let inp_comp = re_st().rex.input.slice(len);
                                if q_comp != inp_comp {
                                    status = RegAction::NoMatch;
                                }
                                re_st().rex.input = re_st().rex.input.wrapping_add(len);
                            }
                        }
                    }
                    MULTIBYTECODE => {
                        let opnd = &prog[operand_idx(scan_idx)..];
                        let mut len = crate::mbyte::utfc_slice2len(opnd) as i32;
                        if len < 2 {
                            status = RegAction::NoMatch;
                        } else {
                            let opndc = crate::mbyte::utf_slice2char(opnd);
                            if utf_iscomposing_legacy(opndc) {
                                status = RegAction::NoMatch;
                                let mut i: usize = 0;
                                while re_st().rex.input.wrapping_add(i).first_byte() != NUL {
                                    let step =
                                        utf_ptr2len(re_st().rex.input.wrapping_add(i)) as usize;
                                    let inpc = utf_ptr2char(re_st().rex.input.wrapping_add(i));
                                    if !utf_iscomposing_legacy(inpc) {
                                        if i > 0 {
                                            break;
                                        }
                                    } else if opndc == inpc {
                                        len = (i as i32)
                                            + utfc_ptr2len(re_st().rex.input.wrapping_add(i));
                                        status = RegAction::Match;
                                        break;
                                    }
                                    i += step;
                                }
                                if status != RegAction::NoMatch {
                                    re_st().rex.input =
                                        re_st().rex.input.wrapping_add(len as usize);
                                }
                            } else if cstrncmp(opnd, re_st().rex.input.as_bytes(), &mut len) != 0 {
                                status = RegAction::NoMatch;
                            } else {
                                re_st().rex.input = re_st().rex.input.wrapping_add(len as usize);
                            }
                        }
                    }
                    RE_COMPOSING => {
                        while utf_iscomposing_legacy(utf_ptr2char(re_st().rex.input)) {
                            re_st().rex.input = re_st()
                                .rex
                                .input
                                .wrapping_add(utf_ptr2len(re_st().rex.input) as usize);
                        }
                    }
                    NOTHING => {}
                    BACK => {
                        let bp = &mut re_st().backpos;
                        let idx = if let Some(pos) =
                            bp.iter().position(|item| item.bp_scan == scan_idx)
                        {
                            if reg_save_equal(&bp.as_slice()[pos].bp_pos) {
                                status = RegAction::NoMatch;
                            }
                            pos
                        } else {
                            let pos = bp.len();
                            bp.push(Backpos {
                                bp_scan: scan_idx,
                                bp_pos: Regsave::new(),
                            });
                            pos
                        };
                        if status != RegAction::NoMatch {
                            let pos_ptr = &mut bp.as_mut_slice()[idx].bp_pos;
                            reg_save(pos_ptr);
                        }
                    }
                    x if (MOPEN..=MOPEN + 9).contains(&x) => {
                        let no = (op - MOPEN) as usize;
                        cleanup_subexpr();
                        if let Some(rp) = regstack_push(RS_MOPEN, scan) {
                            rp.rs_no = no as i16;
                            save_se_m(rp.rs_un.sesave_mut(), no, false);
                        } else {
                            status = RegAction::Fail;
                        }
                    }
                    NOPEN | NCLOSE => {
                        if regstack_push(RS_NOPEN, scan).is_none() {
                            status = RegAction::Fail;
                        }
                    }
                    x if (ZOPEN + 1..=ZOPEN + 9).contains(&x) => {
                        let no = (op - ZOPEN) as usize;
                        cleanup_zsubexpr();
                        if let Some(rp) = regstack_push(RS_ZOPEN, scan) {
                            rp.rs_no = no as i16;
                            save_se_z(rp.rs_un.sesave_mut(), no, false);
                        } else {
                            status = RegAction::Fail;
                        }
                    }
                    x if (MCLOSE..=MCLOSE + 9).contains(&x) => {
                        let no = (op - MCLOSE) as usize;
                        cleanup_subexpr();
                        if let Some(rp) = regstack_push(RS_MCLOSE, scan) {
                            rp.rs_no = no as i16;
                            save_se_m(rp.rs_un.sesave_mut(), no, true);
                        } else {
                            status = RegAction::Fail;
                        }
                    }
                    x if (ZCLOSE + 1..=ZCLOSE + 9).contains(&x) => {
                        let no = (op - ZCLOSE) as usize;
                        cleanup_zsubexpr();
                        if let Some(rp) = regstack_push(RS_ZCLOSE, scan) {
                            rp.rs_no = no as i16;
                            save_se_z(rp.rs_un.sesave_mut(), no, true);
                        } else {
                            status = RegAction::Fail;
                        }
                    }
                    x if (BACKREF + 1..=BACKREF + 9).contains(&x) => {
                        let no = (op - BACKREF) as usize;
                        let mut len: i32 = 0;
                        cleanup_subexpr();
                        if !reg_multi() {
                            let start_no = re_st().rex.match_mut().startp[no];
                            let end_no = re_st().rex.match_mut().endp[no];
                            if !start_no.is_null() && !end_no.is_null() {
                                len = end_no.offset_from(start_no) as i32;

                                if cstrncmp(
                                    (start_no).as_bytes(),
                                    re_st().rex.input.as_bytes(),
                                    &mut len,
                                ) != 0
                                {
                                    status = RegAction::NoMatch;
                                }
                            }
                        } else {
                            let spos = re_st().rex.startpos_slice_mut()[no];
                            let epos = re_st().rex.endpos_slice_mut()[no];
                            if spos.lnum >= 0 && epos.lnum >= 0 {
                                if spos.lnum == re_st().rex.lnum && epos.lnum == re_st().rex.lnum {
                                    len = epos.col - spos.col;
                                    if cstrncmp(
                                        re_st().rex.line.wrapping_add(spos.col as usize).as_bytes(),
                                        re_st().rex.input.as_bytes(),
                                        &mut len,
                                    ) != 0
                                    {
                                        status = RegAction::NoMatch;
                                    }
                                } else {
                                    let r = match_with_backref(
                                        spos.lnum, spos.col, epos.lnum, epos.col, &mut len,
                                    );
                                    if r != RegAction::Match {
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
                        if let Some(ref em) = *crate::globals::GraphPtr::of(ext_in).borrow()
                            && let Some(ref m_cstr) = em.matches[no]
                        {
                            let mut len = m_cstr.as_bytes().len() as i32;
                            if cstrncmp(m_cstr.to_bytes(), re_st().rex.input.as_bytes(), &mut len)
                                != 0
                            {
                                status = RegAction::NoMatch;
                            } else {
                                re_st().rex.input = re_st().rex.input.wrapping_add(len as usize);
                            }
                        }
                    }
                    BRANCH => {
                        if op_fn(prog, next) != BRANCH {
                            next = Some(operand_idx(scan_idx));
                        } else if regstack_push(RS_BRANCH, scan).is_some() {
                            status = RegAction::Break;
                        } else {
                            status = RegAction::Fail;
                        }
                    }
                    BRACE_LIMITS => {
                        if op_fn(prog, next) == BRACE_SIMPLE {
                            bt_st().bl_minval = operand_min(prog, scan_idx);
                            bt_st().bl_maxval = operand_max(prog, scan_idx);
                        } else if op_fn(prog, next) >= BRACE_COMPLEX
                            && op_fn(prog, next) < BRACE_COMPLEX + 10
                        {
                            let no = (op_fn(prog, next) - BRACE_COMPLEX) as usize;
                            re_st().brace_min[no] = operand_min(prog, scan_idx);
                            re_st().brace_max[no] = operand_max(prog, scan_idx);
                            re_st().brace_count[no] = 0;
                        } else {
                            internal_error(c"BRACE_LIMITS");
                            status = RegAction::Fail;
                        }
                    }
                    x if (BRACE_COMPLEX..=BRACE_COMPLEX + 9).contains(&x) => {
                        let no = (op - BRACE_COMPLEX) as usize;
                        let bmin = re_st().brace_min[no];
                        let bmax = re_st().brace_max[no];
                        re_st().brace_count[no] += 1;
                        let bcount = re_st().brace_count[no];

                        if i64::from(bcount) <= bmin.min(bmax) {
                            if let Some(rp) = regstack_push(RS_BRCPLX_MORE, scan) {
                                rp.rs_no = no as i16;
                                reg_save(rp.rs_un.regsave_mut());
                                next = Some(operand_idx(scan_idx));
                            } else {
                                status = RegAction::Fail;
                            }
                        } else if bmin <= bmax {
                            if i64::from(bcount) <= bmax {
                                if let Some(rp) = regstack_push(RS_BRCPLX_LONG, scan) {
                                    rp.rs_no = no as i16;
                                    reg_save(rp.rs_un.regsave_mut());
                                    next = Some(operand_idx(scan_idx));
                                } else {
                                    status = RegAction::Fail;
                                }
                            }
                        } else if i64::from(bcount) <= bmin {
                            if let Some(rp) = regstack_push(RS_BRCPLX_SHORT, scan) {
                                reg_save(rp.rs_un.regsave_mut());
                            } else {
                                status = RegAction::Fail;
                            }
                        }
                    }
                    BRACE_SIMPLE | STAR | PLUS => {
                        let mut rst = Regstar::new();
                        if let Some(next_idx) = next
                            && op_fn(prog, next) == EXACTLY
                        {
                            rst.nextb = i32::from(prog[operand_idx(next_idx)]);

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

                        rst.count = i64::from(regrepeat(prog, operand_idx(scan_idx), rst.maxval));
                        if g_st().got_int {
                            status = RegAction::Fail;
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
                                status = RegAction::Fail;
                            } else {
                                re_st()
                                    .regstack
                                    .resize(re_st().regstack.len() + size_of::<Regstar>() / 8, 0);
                                if let Some(rp) = regstack_push(
                                    if rst.minval <= rst.maxval {
                                        RS_STAR_LONG
                                    } else {
                                        RS_STAR_SHORT
                                    },
                                    scan,
                                ) {
                                    {
                                        *crate::globals::GraphPtr::of(
                                            ptr::from_mut(rp).cast::<Regstar>().wrapping_sub(1),
                                        )
                                        .borrow_mut() = rst;
                                    }
                                    status = RegAction::Break;
                                } else {
                                    status = RegAction::Fail;
                                }
                            }
                        } else {
                            status = RegAction::NoMatch;
                        }
                    }
                    NOMATCH | MATCH | SUBPAT => {
                        if let Some(rp) = regstack_push(RS_NOMATCH, scan) {
                            rp.rs_no = op as i16;
                            reg_save(rp.rs_un.regsave_mut());
                            next = Some(operand_idx(scan_idx));
                        } else {
                            status = RegAction::Fail;
                        }
                    }
                    BEHIND | NOBEHIND => {
                        if i64::from(((re_st().regstack.len() * 8) as u32) >> 10)
                            >= opt_vars().p_mmp
                        {
                            emsg(tr(E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN));
                            status = RegAction::Fail;
                        } else {
                            re_st()
                                .regstack
                                .resize(re_st().regstack.len() + size_of::<Regbehind>() / 8, 0);
                            if let Some(rp) = regstack_push(RS_BEHIND1, scan) {
                                let bp = crate::globals::GraphPtr::from(&*rp)
                                    .cast_ptr::<Regbehind>()
                                    .wrapping_sub(1)
                                    .borrow_mut();
                                save_subexpr(bp);
                                rp.rs_no = op as i16;
                                reg_save(rp.rs_un.regsave_mut());
                            } else {
                                status = RegAction::Fail;
                            }
                        }
                    }
                    BHPOS => {
                        if reg_multi() {
                            let bpos = re_st().behind_pos.rs_u.pos();
                            if bpos.col != re_st().rex.input_col() || bpos.lnum != re_st().rex.lnum
                            {
                                status = RegAction::NoMatch;
                            }
                        } else if re_st().behind_pos.rs_u.ptr.addr() != re_st().rex.input.addr() {
                            status = RegAction::NoMatch;
                        }
                    }
                    NEWL => {
                        if (c != NUL as i32
                            || !reg_multi()
                            || re_st().rex.lnum > re_st().rex.reg_maxline
                            || re_st().rex.reg_line_lbr)
                            && (c != b'\n' as i32 || !re_st().rex.reg_line_lbr)
                        {
                            status = RegAction::NoMatch;
                        } else if re_st().rex.reg_line_lbr {
                            advance_reginput();
                        } else {
                            reg_nextline();
                        }
                    }
                    END => {
                        status = RegAction::Match;
                    }
                    _ => {
                        iemsg(tr(E_RE_CORR));
                        status = RegAction::Fail;
                    }
                }
            }

            if status != RegAction::Cont {
                break;
            }
            scan = next;
        }

        while !re_st().regstack.is_empty() && status != RegAction::Fail {
            let rp_ptr = re_st()
                .regstack
                .as_mut_ptr()
                .wrapping_add(re_st().regstack.len())
                .cast::<Regitem>()
                .wrapping_sub(1);
            let rp = crate::globals::GraphPtr::of(rp_ptr).borrow_mut();
            match rp.rs_state {
                RS_NOPEN => {
                    regstack_pop(&mut scan);
                }
                RS_MOPEN => {
                    if status == RegAction::NoMatch {
                        let no = rp.rs_no as usize;
                        restore_se_m(rp.rs_un.sesave(), no, false);
                    }
                    regstack_pop(&mut scan);
                }
                RS_ZOPEN => {
                    if status == RegAction::NoMatch {
                        let no = rp.rs_no as usize;
                        restore_se_z(rp.rs_un.sesave(), no, false);
                    }
                    regstack_pop(&mut scan);
                }
                RS_MCLOSE => {
                    if status == RegAction::NoMatch {
                        let no = rp.rs_no as usize;
                        restore_se_m(rp.rs_un.sesave(), no, true);
                    }
                    regstack_pop(&mut scan);
                }
                RS_ZCLOSE => {
                    if status == RegAction::NoMatch {
                        let no = rp.rs_no as usize;
                        restore_se_z(rp.rs_un.sesave(), no, true);
                    }
                    regstack_pop(&mut scan);
                }
                RS_BRANCH => {
                    if status == RegAction::Match {
                        regstack_pop(&mut scan);
                    } else {
                        if status != RegAction::Break {
                            reg_restore(rp.rs_un.regsave_mut());
                            scan = rp.rs_scan;
                        }
                        if scan.is_none() || op_fn(prog, scan) != BRANCH {
                            status = RegAction::NoMatch;
                            regstack_pop(&mut scan);
                        } else {
                            rp.rs_scan = regnext(prog, scan);
                            reg_save(rp.rs_un.regsave_mut());
                            scan = scan.map(operand_idx);
                        }
                    }
                }
                RS_BRCPLX_MORE => {
                    if status == RegAction::NoMatch {
                        reg_restore(rp.rs_un.regsave_mut());
                        re_st().brace_count[rp.rs_no as usize] -= 1;
                    }
                    regstack_pop(&mut scan);
                }
                RS_BRCPLX_LONG => {
                    if status == RegAction::NoMatch {
                        reg_restore(rp.rs_un.regsave_mut());
                        re_st().brace_count[rp.rs_no as usize] -= 1;
                        status = RegAction::Cont;
                    }
                    regstack_pop(&mut scan);
                    if status == RegAction::Cont {
                        scan = regnext(prog, scan);
                    }
                }
                RS_BRCPLX_SHORT => {
                    if status == RegAction::NoMatch {
                        reg_restore(rp.rs_un.regsave_mut());
                    }
                    regstack_pop(&mut scan);
                    if status == RegAction::NoMatch {
                        scan = scan.map(operand_idx);
                        status = RegAction::Cont;
                    }
                }
                RS_NOMATCH => {
                    if status
                        == if rp.rs_no == NOMATCH as i16 {
                            RegAction::Match
                        } else {
                            RegAction::NoMatch
                        }
                    {
                        status = RegAction::NoMatch;
                    } else {
                        status = RegAction::Cont;
                        if rp.rs_no != SUBPAT as i16 {
                            reg_restore(rp.rs_un.regsave_mut());
                        }
                    }
                    regstack_pop(&mut scan);
                    if status == RegAction::Cont {
                        scan = regnext(prog, scan);
                    }
                }
                RS_BEHIND1 => {
                    if status == RegAction::NoMatch {
                        regstack_pop(&mut scan);
                        re_st()
                            .regstack
                            .truncate(re_st().regstack.len() - size_of::<Regbehind>() / 8);
                    } else {
                        let bp = crate::globals::GraphPtr::of(
                            rp_ptr.cast::<Regbehind>().wrapping_sub(1),
                        )
                        .borrow_mut();
                        reg_save(&mut bp.save_after);
                        bp.save_behind = re_st().behind_pos;

                        re_st().behind_pos = *rp.rs_un.regsave();
                        rp.rs_state = RS_BEHIND2;
                        reg_restore(rp.rs_un.regsave_mut());

                        scan = rp.rs_scan.map(|idx| operand_idx(idx) + 4);
                    }
                }
                RS_BEHIND2 => {
                    let bp =
                        crate::globals::GraphPtr::of(rp_ptr.cast::<Regbehind>().wrapping_sub(1))
                            .borrow_mut();
                    if status == RegAction::Match && reg_save_equal(&re_st().behind_pos) {
                        re_st().behind_pos = bp.save_behind;
                        if rp.rs_no == BEHIND as i16 {
                            reg_restore(&mut bp.save_after);
                        } else {
                            status = RegAction::NoMatch;
                            restore_subexpr(&mut *bp);
                        }
                        regstack_pop(&mut scan);
                        re_st()
                            .regstack
                            .truncate(re_st().regstack.len() - size_of::<Regbehind>() / 8);
                    } else {
                        let mut no = true;
                        let limit = operand_min(prog, rp.rs_scan.unwrap());
                        if reg_multi() {
                            let rp_pos = rp.rs_un.regsave().rs_u.pos();
                            let bpos = re_st().behind_pos.rs_u.pos();
                            if limit > 0
                                && i64::from(
                                    (if rp_pos.lnum < bpos.lnum {
                                        reg_getline_len(re_st().rex.lnum)
                                    } else {
                                        bpos.col
                                    }) - rp_pos.col,
                                ) >= limit
                            {
                                no = false;
                            } else if rp_pos.col == 0 {
                                rp.rs_un.regsave_mut().rs_u.pos_mut().lnum -= 1;

                                let new_lnum = rp.rs_un.regsave().rs_u.pos().lnum;
                                if new_lnum + 1 < bpos.lnum || reg_getline(new_lnum).is_none() {
                                    no = false;
                                } else {
                                    reg_restore(rp.rs_un.regsave_mut());
                                    rp.rs_un.regsave_mut().rs_u.pos_mut().col =
                                        reg_getline_len(re_st().rex.lnum);
                                }
                            } else {
                                let line_bytes =
                                    reg_getline(rp_pos.lnum).map_or(&[][..], CStr::to_bytes);
                                rp.rs_un.regsave_mut().rs_u.pos_mut().col -=
                                    crate::mbyte::utf_head_off_slice(
                                        line_bytes,
                                        rp_pos.col as usize - 1,
                                    ) + 1;
                            }
                        } else if rp.rs_un.regsave().rs_u.ptr.addr() == re_st().rex.line.addr() {
                            no = false;
                        } else {
                            let line = re_st().rex.line;
                            let p_off = rp.rs_un.regsave().rs_u.ptr.offset_from(line);
                            let prev_off = p_off
                                - 1
                                - crate::mbyte::utf_head_off_slice(line.unwrap_or(&[]), p_off - 1)
                                    as usize;
                            let p = line.wrapping_add(prev_off);
                            rp.rs_un.regsave_mut().rs_u.ptr = p;

                            if limit > 0
                                && re_st()
                                    .behind_pos
                                    .rs_u
                                    .ptr
                                    .offset_from(rp.rs_un.regsave().rs_u.ptr)
                                    > limit as usize
                            {
                                no = false;
                            }
                        }
                        if no {
                            reg_restore(rp.rs_un.regsave_mut());
                            scan = rp.rs_scan.map(|idx| operand_idx(idx) + 4);

                            if status == RegAction::Match {
                                status = RegAction::NoMatch;
                                restore_subexpr(&mut *bp);
                            }
                        } else {
                            re_st().behind_pos = bp.save_behind;
                            if rp.rs_no == NOBEHIND as i16 {
                                reg_restore(&mut bp.save_after);
                                status = RegAction::Match;
                            } else if status == RegAction::Match {
                                status = RegAction::NoMatch;
                                restore_subexpr(&mut *bp);
                            }
                            regstack_pop(&mut scan);
                            re_st()
                                .regstack
                                .truncate(re_st().regstack.len() - size_of::<Regbehind>() / 8);
                        }
                    }
                }
                RS_STAR_LONG | RS_STAR_SHORT => {
                    let rst =
                        crate::globals::GraphPtr::of(rp_ptr.cast::<Regstar>().wrapping_sub(1))
                            .borrow_mut();
                    if status == RegAction::Match {
                        regstack_pop(&mut scan);
                        re_st()
                            .regstack
                            .truncate(re_st().regstack.len() - size_of::<Regstar>() / 8);
                    } else {
                        if status != RegAction::Break {
                            reg_restore(rp.rs_un.regsave_mut());
                        }
                        loop {
                            if status != RegAction::Break {
                                if rp.rs_state == RS_STAR_LONG {
                                    rst.count -= 1;
                                    if rst.count < rst.minval {
                                        break;
                                    }
                                    if re_st().rex.input.addr() == re_st().rex.line.addr() {
                                        if re_st().rex.lnum == 0 {
                                            status = RegAction::NoMatch;
                                            break;
                                        }
                                        re_st().rex.lnum -= 1;
                                        let prev_lnum = re_st().rex.lnum;
                                        re_st().rex.load_line(prev_lnum);
                                        if re_st().rex.line.is_null() {
                                            break;
                                        }
                                        re_st().rex.input =
                                            re_st()
                                                .rex
                                                .line
                                                .wrapping_add(
                                                    reg_getline_len(re_st().rex.lnum) as usize
                                                );
                                        reg_breakcheck();
                                    } else {
                                        let line = re_st().rex.line;
                                        let inp_off = re_st().rex.input_off();
                                        let prev_off = inp_off
                                            - 1
                                            - crate::mbyte::utf_head_off_slice(
                                                line.unwrap_or(&[]),
                                                inp_off - 1,
                                            )
                                                as usize;
                                        re_st().rex.input = line.wrapping_add(prev_off);
                                    }
                                } else {
                                    if rst.count == rst.minval
                                        || regrepeat(prog, operand_idx(rp.rs_scan.unwrap()), 1) == 0
                                    {
                                        break;
                                    }
                                    rst.count += 1;
                                }
                                if g_st().got_int {
                                    break;
                                }
                            } else {
                                status = RegAction::NoMatch;
                            }

                            if rst.nextb == NUL as i32
                                || input_byte() as i32 == rst.nextb
                                || input_byte() as i32 == rst.nextb_ic
                            {
                                reg_save(rp.rs_un.regsave_mut());
                                scan = regnext(prog, rp.rs_scan);
                                status = RegAction::Cont;
                                break;
                            }
                        }
                        if status != RegAction::Cont {
                            regstack_pop(&mut scan);
                            re_st()
                                .regstack
                                .truncate(re_st().regstack.len() - size_of::<Regstar>() / 8);
                            status = RegAction::NoMatch;
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
            if status == RegAction::Cont || rp_ptr == top {
                break;
            }
        }

        if status == RegAction::Cont {
            continue;
        }

        if re_st().regstack.is_empty() || status == RegAction::Fail {
            if scan.is_none() {
                iemsg(tr(E_RE_CORR));
            }
            return status == RegAction::Match;
        }
    }
}

fn regtry(
    prog: &mut BtRegprog,
    col: Colnr,
    tm: Option<Proftime>,
    timed_out: Option<&mut i32>,
) -> i32 {
    re_st().rex.input = re_st().rex.line.wrapping_add(col as usize);
    re_st().rex.need_clear_subexpr = true;
    re_st().rex.need_clear_zsubexpr = prog.reghasz == REX_SET as u8;

    if !regmatch(
        &prog.program,
        Some(1),
        tm,
        (timed_out.is_some()).then(|| &mut *timed_out.unwrap()),
    ) {
        return 0;
    }

    cleanup_subexpr();
    if reg_multi() {
        let startpos = re_st().rex.startpos_slice_mut();
        let endpos = re_st().rex.endpos_slice_mut();
        if startpos[0].lnum < 0 {
            startpos[0].lnum = 0;
            startpos[0].col = col;
        }
        if endpos[0].lnum < 0 {
            endpos[0].lnum = re_st().rex.lnum;
            endpos[0].col = re_st().rex.input_col();
        } else {
            re_st().rex.lnum = endpos[0].lnum;
        }
    } else {
        let m = re_st().rex.match_mut();
        if m.startp[0].is_null() {
            m.startp[0] = re_st().rex.line.wrapping_add(col as usize);
        }
        if m.endp[0].is_null() {
            m.endp[0] = re_st().rex.input;
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
                if sz.lnum >= 0
                    && ez.lnum == sz.lnum
                    && ez.col >= sz.col
                    && let Some(line_cstr) = reg_getline(sz.lnum)
                {
                    extmatch.matches[i] =
                        cstring_from_bytes(crate::strings::AsByteSlice::slice_len(
                            line_cstr.as_ptr().wrapping_add(sz.col as usize),
                            (ez.col - sz.col) as usize,
                        ));
                }
            } else {
                let szp = re_st().reg_startzp[i];
                let ezp = re_st().reg_endzp[i];
                if !szp.is_null() && !ezp.is_null() {
                    extmatch.matches[i] = cstring_from_bytes(szp.span_to(ezp));
                }
            }
        }
        g_st().re_extmatch_out = Some(std::rc::Rc::new(extmatch));
    }
    1 + re_st().rex.lnum
}

fn bt_regexec_both(
    line_opt: Option<&CStr>,
    startcol: Colnr,
    tm: Option<Proftime>,
    mut timed_out: Option<&mut i32>,
) -> i32 {
    let mut line = reg_line_from_opt_cstr(line_opt);
    let mut col = startcol;
    let mut retval: i32 = 0;

    if re_st().regstack.capacity() == 0 {
        re_st().regstack.reserve((REGSTACK_INITIAL as usize) / 8);
    }

    if re_st().backpos.capacity() == 0 {
        re_st().backpos.reserve(BACKPOS_INITIAL as usize);
    }

    let prog = crate::globals::GraphPtr::<BtRegprog>::of(if reg_multi() {
        let p = re_st().rex.mmatch_mut().prog_mut().bt_mut();
        line = reg_line_from_opt_cstr(reg_getline(0));
        re_st().rex.reg_startpos =
            crate::globals::GraphPtr::from(&mut re_st().rex.mmatch_mut().startpos);
        re_st().rex.reg_endpos =
            crate::globals::GraphPtr::from(&mut re_st().rex.mmatch_mut().endpos);
        p
    } else {
        re_st().rex.match_mut().prog_mut().bt_mut()
    })
    .as_ptr();
    if prog.is_null() || line.is_null() {
        iemsg(tr(E_NULL));
        return finish_bt_regexec(retval, col);
    }
    let prog = crate::globals::GraphPtr::of(prog).borrow_mut();

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
        let regmust_slice = crate::memory::bytes_until_nul(&prog.program[off..]);
        let c = crate::mbyte::utf_slice2char(regmust_slice);
        let mut s = line
            .as_cstr()
            .as_ptr()
            .cast_mut()
            .cast::<u8>()
            .wrapping_add(col as usize);
        if !re_st().rex.reg_ic {
            loop {
                s = vim_strchr(crate::strings::ByteSlice::from_ptr(s).as_cstr(), c)
                    .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
                    .cast();
                if s.is_null() {
                    break;
                }
                let mut len = prog.regmlen;
                if cstrncmp(
                    crate::strings::ByteSlice::from_ptr(s).as_bytes(),
                    regmust_slice,
                    &mut len,
                ) == 0
                {
                    break;
                }
                let step = utfc_ptr2len(s) as usize;
                s = s.wrapping_add(step);
            }
        } else {
            loop {
                let s_cstr = crate::strings::ByteSlice::from_ptr(s).as_cstr();
                let Some(found_off) = cstrchr(s_cstr, c) else {
                    s = ptr::null_mut();
                    break;
                };
                s = s.wrapping_add(found_off);
                let mut len = prog.regmlen;
                if cstrncmp(
                    crate::strings::ByteSlice::from_ptr(s).as_bytes(),
                    regmust_slice,
                    &mut len,
                ) == 0
                {
                    break;
                }
                let step = utfc_ptr2len(s) as usize;
                s = s.wrapping_add(step);
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
        let c = utf_ptr2char(re_st().rex.line.wrapping_add(col as usize));
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
                let rem_cstr = re_st().rex.line.wrapping_add(col as usize).as_cstr();
                let Some(found_off) = cstrchr(rem_cstr, prog.regstart) else {
                    retval = 0;
                    break;
                };
                col += found_off as Colnr;
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
                re_st().rex.load_line(0);
            }
            if re_st().rex.line.wrapping_add(col as usize).is_empty() {
                break;
            }
            col += utfc_ptr2len(re_st().rex.line.wrapping_add(col as usize));
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
            if re_st().rex.match_mut().endp[0].before(re_st().rex.match_mut().startp[0]) {
                re_st().rex.match_mut().endp[0] = re_st().rex.match_mut().startp[0];
            }
            re_st().rex.match_mut().rm_matchcol = col;
        }
    }

    retval
}

pub(super) fn bt_regexec_nl(
    rmp: &mut Regmatch,
    line: Option<&CStr>,
    col: Colnr,
    line_lbr: bool,
) -> i32 {
    re_st().rex.reg_match = crate::globals::GraphPtr::from(&mut *rmp);
    re_st().rex.reg_mmatch = crate::globals::GraphPtr::NULL;
    re_st().rex.reg_maxline = 0;
    re_st().rex.reg_line_lbr = line_lbr;
    re_st().rex.reg_buf = g_st().curbuf;
    re_st().rex.reg_win = crate::globals::WinPtr::NULL;
    re_st().rex.reg_ic = rmp.rm_ic;
    re_st().rex.reg_icombine = false;
    re_st().rex.reg_nobreak = (rmp.prog_mut().re_flags & (RE_NOBREAK as u32)) != 0;
    re_st().rex.reg_maxcol = 0;

    bt_regexec_both(line, col, None, None)
}

pub(super) fn bt_regexec_multi(
    rmp: &mut Regmmatch,
    win: Option<&mut WinInfoInfo>,
    buf: &mut FileBuf,
    lnum: Linenr,
    col: Colnr,
    tm: Option<Proftime>,
    timed_out: Option<&mut i32>,
) -> i32 {
    init_regexec_multi(rmp, win, buf, lnum);
    bt_regexec_both(None, col, tm, timed_out)
}

fn re_num_cmp(val: u32, prog: &[u8], scan: usize) -> bool {
    let n = operand_min(prog, scan) as u32;
    let cmp = operand_cmp(prog, scan);
    if cmp == b'>' {
        val > n
    } else if cmp == b'<' {
        val < n
    } else {
        val == n
    }
}
