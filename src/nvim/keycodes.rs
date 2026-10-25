//! Keycode definitions and `<key>` notation translation (`src/nvim/keycodes.c`).

use crate::api::private::validate::NvimString;
use crate::charset::{
    ascii_isalpha, ascii_isdigit, is_special, transchar, vim_isprintc, vim_str2nr,
};
use crate::errors::{e_invarg, e_usingsid};
use crate::eval::vars::{get_var_value, scid_T};
use crate::globals::*;
use crate::mbyte::{
    utf_char2bytes, utf_char2len, utf_ptr2char, utf_ptr2len, utfc_ptr2len, utfc_ptr2len_len,
};
use crate::memory::{xfree, xmalloc};
use crate::message::emsg;
use crate::strings::{vim_snprintf, vim_strchr};
use std::ffi::{CStr, c_char, c_int, c_uint};

pub const NUL: u8 = 0;
pub const BS: c_int = 0x08;
pub const TAB: c_int = 0x09;
pub const NL: c_int = 0x0a;
pub const CAR: c_int = 0x0d;
pub const Ctrl_V: u8 = 22;
pub const ESC: c_int = 0x1b;
pub const DEL: c_int = 0x7f;
pub const CSI: c_int = 0x9b;

pub const K_SPECIAL: u8 = 0x80;
pub const KS_ZERO: u8 = 255;
pub const KS_SPECIAL: u8 = 254;
pub const KS_EXTRA: u8 = 253;
pub const KS_MODIFIER: u8 = 252;
pub const KS_MOUSE: u8 = 251;
pub const KS_KEY: u8 = 242;
pub const KE_FILLER: u8 = b'X';

pub const KE_S_UP: u8 = 4;
pub const KE_S_DOWN: u8 = 5;
pub const KE_S_F1: u8 = 6;
pub const KE_S_F2: u8 = 7;
pub const KE_S_F3: u8 = 8;
pub const KE_S_F4: u8 = 9;
pub const KE_S_F5: u8 = 10;
pub const KE_S_F6: u8 = 11;
pub const KE_S_F7: u8 = 12;
pub const KE_S_F8: u8 = 13;
pub const KE_S_F9: u8 = 14;
pub const KE_S_F10: u8 = 15;
pub const KE_S_F11: u8 = 16;
pub const KE_S_F12: u8 = 17;
pub const KE_S_F13: u8 = 18;
pub const KE_S_F14: u8 = 19;
pub const KE_S_F15: u8 = 20;
pub const KE_S_F16: u8 = 21;
pub const KE_S_F17: u8 = 22;
pub const KE_S_F18: u8 = 23;
pub const KE_S_F19: u8 = 24;
pub const KE_S_F20: u8 = 25;
pub const KE_S_F21: u8 = 26;
pub const KE_S_F22: u8 = 27;
pub const KE_S_F23: u8 = 28;
pub const KE_S_F24: u8 = 29;
pub const KE_S_F25: u8 = 30;
pub const KE_S_F26: u8 = 31;
pub const KE_S_F27: u8 = 32;
pub const KE_S_F28: u8 = 33;
pub const KE_S_F29: u8 = 34;
pub const KE_S_F30: u8 = 35;
pub const KE_S_F31: u8 = 36;
pub const KE_S_F32: u8 = 37;
pub const KE_S_F33: u8 = 38;
pub const KE_S_F34: u8 = 39;
pub const KE_S_F35: u8 = 40;
pub const KE_S_F36: u8 = 41;
pub const KE_S_F37: u8 = 42;
pub const KE_LEFTMOUSE: u8 = 44;
pub const KE_LEFTDRAG: u8 = 45;
pub const KE_LEFTRELEASE: u8 = 46;
pub const KE_MIDDLEMOUSE: u8 = 47;
pub const KE_MIDDLEDRAG: u8 = 48;
pub const KE_MIDDLERELEASE: u8 = 49;
pub const KE_RIGHTMOUSE: u8 = 50;
pub const KE_RIGHTDRAG: u8 = 51;
pub const KE_RIGHTRELEASE: u8 = 52;
pub const KE_IGNORE: u8 = 53;
pub const KE_TAB: u8 = 54;
pub const KE_XF1: u8 = 57;
pub const KE_XF2: u8 = 58;
pub const KE_XF3: u8 = 59;
pub const KE_XF4: u8 = 60;
pub const KE_XEND: u8 = 61;
pub const KE_ZEND: u8 = 62;
pub const KE_XHOME: u8 = 63;
pub const KE_ZHOME: u8 = 64;
pub const KE_XUP: u8 = 65;
pub const KE_XDOWN: u8 = 66;
pub const KE_XLEFT: u8 = 67;
pub const KE_XRIGHT: u8 = 68;
pub const KE_LEFTMOUSE_NM: u8 = 69;
pub const KE_LEFTRELEASE_NM: u8 = 70;
pub const KE_S_XF1: u8 = 71;
pub const KE_S_XF2: u8 = 72;
pub const KE_S_XF3: u8 = 73;
pub const KE_S_XF4: u8 = 74;
pub const KE_MOUSEDOWN: u8 = 75;
pub const KE_MOUSEUP: u8 = 76;
pub const KE_MOUSELEFT: u8 = 77;
pub const KE_MOUSERIGHT: u8 = 78;
pub const KE_KINS: u8 = 79;
pub const KE_KDEL: u8 = 80;
pub const KE_SNR: u8 = 82;
pub const KE_PLUG: u8 = 83;
pub const KE_C_LEFT: u8 = 85;
pub const KE_C_RIGHT: u8 = 86;
pub const KE_C_HOME: u8 = 87;
pub const KE_C_END: u8 = 88;
pub const KE_X1MOUSE: u8 = 89;
pub const KE_X1DRAG: u8 = 90;
pub const KE_X1RELEASE: u8 = 91;
pub const KE_X2MOUSE: u8 = 92;
pub const KE_X2DRAG: u8 = 93;
pub const KE_X2RELEASE: u8 = 94;
pub const KE_DROP: u8 = 95;
pub const KE_NOP: u8 = 97;
pub const KE_MOUSEMOVE: u8 = 100;
pub const KE_EVENT: u8 = 102;
pub const KE_LUA: u8 = 103;
pub const KE_COMMAND: u8 = 104;
pub const KE_WILD: u8 = 108;
pub const KE_COMPLETE_DELAY: u8 = 110;

#[inline]
pub const fn termcap2key(a: c_int, b: c_int) -> c_int {
    -(a + (b << 8))
}

#[inline]
pub const fn key2termcap0(x: c_int) -> c_int {
    (-x) & 0xff
}

#[inline]
pub const fn key2termcap1(x: c_int) -> c_int {
    (((-x) as u32 >> 8) & 0xff) as c_int
}

#[inline]
pub const fn k_second(c: c_int) -> u8 {
    if c == K_SPECIAL as c_int {
        KS_SPECIAL
    } else if c == 0 {
        KS_ZERO
    } else {
        key2termcap0(c) as u8
    }
}

#[inline]
pub const fn k_third(c: c_int) -> u8 {
    if c == K_SPECIAL as c_int || c == 0 {
        KE_FILLER
    } else {
        key2termcap1(c) as u8
    }
}

#[inline]
pub const fn ctrl_chr(c: c_int) -> c_int {
    crate::charset::toupper_asc(c) ^ 0x40
}

use crate::cmdexpand::MB_MAXBYTES;
use crate::eval::STR2NR_ALL;
use crate::mouse::{MOUSE_LEFT, MOUSE_MIDDLE, MOUSE_RELEASE, MOUSE_RIGHT, MOUSE_X1, MOUSE_X2};

pub const K_ZERO: c_int = termcap2key(KS_ZERO as c_int, KE_FILLER as c_int);
pub const K_UP: c_int = termcap2key(b'k' as c_int, b'u' as c_int);
pub const K_KUP: c_int = termcap2key(b'K' as c_int, b'u' as c_int);
pub const K_DOWN: c_int = termcap2key(b'k' as c_int, b'd' as c_int);
pub const K_KDOWN: c_int = termcap2key(b'K' as c_int, b'd' as c_int);
pub const K_LEFT: c_int = termcap2key(b'k' as c_int, b'l' as c_int);
pub const K_KLEFT: c_int = termcap2key(b'K' as c_int, b'l' as c_int);
pub const K_RIGHT: c_int = termcap2key(b'k' as c_int, b'r' as c_int);
pub const K_KRIGHT: c_int = termcap2key(b'K' as c_int, b'r' as c_int);
pub const K_S_UP: c_int = termcap2key(KS_EXTRA as c_int, KE_S_UP as c_int);
pub const K_S_DOWN: c_int = termcap2key(KS_EXTRA as c_int, KE_S_DOWN as c_int);
pub const K_S_LEFT: c_int = termcap2key(b'#' as c_int, b'4' as c_int);
pub const K_C_LEFT: c_int = termcap2key(KS_EXTRA as c_int, KE_C_LEFT as c_int);
pub const K_S_RIGHT: c_int = termcap2key(b'%' as c_int, b'i' as c_int);
pub const K_C_RIGHT: c_int = termcap2key(KS_EXTRA as c_int, KE_C_RIGHT as c_int);
pub const K_S_HOME: c_int = termcap2key(b'#' as c_int, b'2' as c_int);
pub const K_C_HOME: c_int = termcap2key(KS_EXTRA as c_int, KE_C_HOME as c_int);
pub const K_S_END: c_int = termcap2key(b'*' as c_int, b'7' as c_int);
pub const K_C_END: c_int = termcap2key(KS_EXTRA as c_int, KE_C_END as c_int);
pub const K_TAB: c_int = termcap2key(KS_EXTRA as c_int, KE_TAB as c_int);
pub const K_S_TAB: c_int = termcap2key(b'k' as c_int, b'B' as c_int);

pub const K_XF1: c_int = termcap2key(KS_EXTRA as c_int, KE_XF1 as c_int);
pub const K_XF2: c_int = termcap2key(KS_EXTRA as c_int, KE_XF2 as c_int);
pub const K_XF3: c_int = termcap2key(KS_EXTRA as c_int, KE_XF3 as c_int);
pub const K_XF4: c_int = termcap2key(KS_EXTRA as c_int, KE_XF4 as c_int);

pub const K_XUP: c_int = termcap2key(KS_EXTRA as c_int, KE_XUP as c_int);
pub const K_XDOWN: c_int = termcap2key(KS_EXTRA as c_int, KE_XDOWN as c_int);
pub const K_XLEFT: c_int = termcap2key(KS_EXTRA as c_int, KE_XLEFT as c_int);
pub const K_XRIGHT: c_int = termcap2key(KS_EXTRA as c_int, KE_XRIGHT as c_int);

pub const K_F1: c_int = termcap2key(b'k' as c_int, b'1' as c_int);
pub const K_F2: c_int = termcap2key(b'k' as c_int, b'2' as c_int);
pub const K_F3: c_int = termcap2key(b'k' as c_int, b'3' as c_int);
pub const K_F4: c_int = termcap2key(b'k' as c_int, b'4' as c_int);
pub const K_F5: c_int = termcap2key(b'k' as c_int, b'5' as c_int);
pub const K_F6: c_int = termcap2key(b'k' as c_int, b'6' as c_int);
pub const K_F7: c_int = termcap2key(b'k' as c_int, b'7' as c_int);
pub const K_F8: c_int = termcap2key(b'k' as c_int, b'8' as c_int);
pub const K_F9: c_int = termcap2key(b'k' as c_int, b'9' as c_int);
pub const K_F10: c_int = termcap2key(b'k' as c_int, b';' as c_int);

pub const K_F11: c_int = termcap2key(b'F' as c_int, b'1' as c_int);
pub const K_F12: c_int = termcap2key(b'F' as c_int, b'2' as c_int);
pub const K_F13: c_int = termcap2key(b'F' as c_int, b'3' as c_int);
pub const K_F14: c_int = termcap2key(b'F' as c_int, b'4' as c_int);
pub const K_F15: c_int = termcap2key(b'F' as c_int, b'5' as c_int);
pub const K_F16: c_int = termcap2key(b'F' as c_int, b'6' as c_int);
pub const K_F17: c_int = termcap2key(b'F' as c_int, b'7' as c_int);
pub const K_F18: c_int = termcap2key(b'F' as c_int, b'8' as c_int);
pub const K_F19: c_int = termcap2key(b'F' as c_int, b'9' as c_int);
pub const K_F20: c_int = termcap2key(b'F' as c_int, b'A' as c_int);

pub const K_F21: c_int = termcap2key(b'F' as c_int, b'B' as c_int);
pub const K_F22: c_int = termcap2key(b'F' as c_int, b'C' as c_int);
pub const K_F23: c_int = termcap2key(b'F' as c_int, b'D' as c_int);
pub const K_F24: c_int = termcap2key(b'F' as c_int, b'E' as c_int);
pub const K_F25: c_int = termcap2key(b'F' as c_int, b'F' as c_int);
pub const K_F26: c_int = termcap2key(b'F' as c_int, b'G' as c_int);
pub const K_F27: c_int = termcap2key(b'F' as c_int, b'H' as c_int);
pub const K_F28: c_int = termcap2key(b'F' as c_int, b'I' as c_int);
pub const K_F29: c_int = termcap2key(b'F' as c_int, b'J' as c_int);
pub const K_F30: c_int = termcap2key(b'F' as c_int, b'K' as c_int);

pub const K_F31: c_int = termcap2key(b'F' as c_int, b'L' as c_int);
pub const K_F32: c_int = termcap2key(b'F' as c_int, b'M' as c_int);
pub const K_F33: c_int = termcap2key(b'F' as c_int, b'N' as c_int);
pub const K_F34: c_int = termcap2key(b'F' as c_int, b'O' as c_int);
pub const K_F35: c_int = termcap2key(b'F' as c_int, b'P' as c_int);
pub const K_F36: c_int = termcap2key(b'F' as c_int, b'Q' as c_int);
pub const K_F37: c_int = termcap2key(b'F' as c_int, b'R' as c_int);
pub const K_F38: c_int = termcap2key(b'F' as c_int, b'S' as c_int);
pub const K_F39: c_int = termcap2key(b'F' as c_int, b'T' as c_int);
pub const K_F40: c_int = termcap2key(b'F' as c_int, b'U' as c_int);

pub const K_F41: c_int = termcap2key(b'F' as c_int, b'V' as c_int);
pub const K_F42: c_int = termcap2key(b'F' as c_int, b'W' as c_int);
pub const K_F43: c_int = termcap2key(b'F' as c_int, b'X' as c_int);
pub const K_F44: c_int = termcap2key(b'F' as c_int, b'Y' as c_int);
pub const K_F45: c_int = termcap2key(b'F' as c_int, b'Z' as c_int);
pub const K_F46: c_int = termcap2key(b'F' as c_int, b'a' as c_int);
pub const K_F47: c_int = termcap2key(b'F' as c_int, b'b' as c_int);
pub const K_F48: c_int = termcap2key(b'F' as c_int, b'c' as c_int);
pub const K_F49: c_int = termcap2key(b'F' as c_int, b'd' as c_int);
pub const K_F50: c_int = termcap2key(b'F' as c_int, b'e' as c_int);

pub const K_F51: c_int = termcap2key(b'F' as c_int, b'f' as c_int);
pub const K_F52: c_int = termcap2key(b'F' as c_int, b'g' as c_int);
pub const K_F53: c_int = termcap2key(b'F' as c_int, b'h' as c_int);
pub const K_F54: c_int = termcap2key(b'F' as c_int, b'i' as c_int);
pub const K_F55: c_int = termcap2key(b'F' as c_int, b'j' as c_int);
pub const K_F56: c_int = termcap2key(b'F' as c_int, b'k' as c_int);
pub const K_F57: c_int = termcap2key(b'F' as c_int, b'l' as c_int);
pub const K_F58: c_int = termcap2key(b'F' as c_int, b'm' as c_int);
pub const K_F59: c_int = termcap2key(b'F' as c_int, b'n' as c_int);
pub const K_F60: c_int = termcap2key(b'F' as c_int, b'o' as c_int);

pub const K_F61: c_int = termcap2key(b'F' as c_int, b'p' as c_int);
pub const K_F62: c_int = termcap2key(b'F' as c_int, b'q' as c_int);
pub const K_F63: c_int = termcap2key(b'F' as c_int, b'r' as c_int);

pub const K_S_XF1: c_int = termcap2key(KS_EXTRA as c_int, KE_S_XF1 as c_int);
pub const K_S_XF2: c_int = termcap2key(KS_EXTRA as c_int, KE_S_XF2 as c_int);
pub const K_S_XF3: c_int = termcap2key(KS_EXTRA as c_int, KE_S_XF3 as c_int);
pub const K_S_XF4: c_int = termcap2key(KS_EXTRA as c_int, KE_S_XF4 as c_int);

pub const K_S_F1: c_int = termcap2key(KS_EXTRA as c_int, KE_S_F1 as c_int);
pub const K_S_F2: c_int = termcap2key(KS_EXTRA as c_int, KE_S_F2 as c_int);
pub const K_S_F3: c_int = termcap2key(KS_EXTRA as c_int, KE_S_F3 as c_int);
pub const K_S_F4: c_int = termcap2key(KS_EXTRA as c_int, KE_S_F4 as c_int);

pub const K_HELP: c_int = termcap2key(b'%' as c_int, b'1' as c_int);
pub const K_UNDO: c_int = termcap2key(b'&' as c_int, b'8' as c_int);
pub const K_FIND: c_int = termcap2key(b'@' as c_int, b'0' as c_int);
pub const K_KSELECT: c_int = termcap2key(b'*' as c_int, b'6' as c_int);
pub const K_BS: c_int = termcap2key(b'k' as c_int, b'b' as c_int);
pub const K_INS: c_int = termcap2key(b'k' as c_int, b'I' as c_int);
pub const K_KINS: c_int = termcap2key(KS_EXTRA as c_int, KE_KINS as c_int);
pub const K_DEL: c_int = termcap2key(b'k' as c_int, b'D' as c_int);
pub const K_KDEL: c_int = termcap2key(KS_EXTRA as c_int, KE_KDEL as c_int);
pub const K_HOME: c_int = termcap2key(b'k' as c_int, b'h' as c_int);
pub const K_KHOME: c_int = termcap2key(b'K' as c_int, b'1' as c_int);
pub const K_XHOME: c_int = termcap2key(KS_EXTRA as c_int, KE_XHOME as c_int);
pub const K_ZHOME: c_int = termcap2key(KS_EXTRA as c_int, KE_ZHOME as c_int);
pub const K_END: c_int = termcap2key(b'@' as c_int, b'7' as c_int);
pub const K_KEND: c_int = termcap2key(b'K' as c_int, b'4' as c_int);
pub const K_XEND: c_int = termcap2key(KS_EXTRA as c_int, KE_XEND as c_int);
pub const K_ZEND: c_int = termcap2key(KS_EXTRA as c_int, KE_ZEND as c_int);
pub const K_PAGEUP: c_int = termcap2key(b'k' as c_int, b'P' as c_int);
pub const K_PAGEDOWN: c_int = termcap2key(b'k' as c_int, b'N' as c_int);
pub const K_KPAGEUP: c_int = termcap2key(b'K' as c_int, b'3' as c_int);
pub const K_KPAGEDOWN: c_int = termcap2key(b'K' as c_int, b'5' as c_int);
pub const K_KORIGIN: c_int = termcap2key(b'K' as c_int, b'2' as c_int);

pub const K_KPLUS: c_int = termcap2key(b'K' as c_int, b'6' as c_int);
pub const K_KMINUS: c_int = termcap2key(b'K' as c_int, b'7' as c_int);
pub const K_KDIVIDE: c_int = termcap2key(b'K' as c_int, b'8' as c_int);
pub const K_KMULTIPLY: c_int = termcap2key(b'K' as c_int, b'9' as c_int);
pub const K_KENTER: c_int = termcap2key(b'K' as c_int, b'A' as c_int);
pub const K_KPOINT: c_int = termcap2key(b'K' as c_int, b'B' as c_int);

pub const K_K0: c_int = termcap2key(b'K' as c_int, b'C' as c_int);
pub const K_K1: c_int = termcap2key(b'K' as c_int, b'D' as c_int);
pub const K_K2: c_int = termcap2key(b'K' as c_int, b'E' as c_int);
pub const K_K3: c_int = termcap2key(b'K' as c_int, b'F' as c_int);
pub const K_K4: c_int = termcap2key(b'K' as c_int, b'G' as c_int);
pub const K_K5: c_int = termcap2key(b'K' as c_int, b'H' as c_int);
pub const K_K6: c_int = termcap2key(b'K' as c_int, b'I' as c_int);
pub const K_K7: c_int = termcap2key(b'K' as c_int, b'J' as c_int);
pub const K_K8: c_int = termcap2key(b'K' as c_int, b'K' as c_int);
pub const K_K9: c_int = termcap2key(b'K' as c_int, b'L' as c_int);

pub const K_KCOMMA: c_int = termcap2key(b'K' as c_int, b'M' as c_int);
pub const K_KEQUAL: c_int = termcap2key(b'K' as c_int, b'N' as c_int);

pub const K_MOUSE: c_int = termcap2key(KS_MOUSE as c_int, KE_FILLER as c_int);
pub const K_LEFTMOUSE: c_int = termcap2key(KS_EXTRA as c_int, KE_LEFTMOUSE as c_int);
pub const K_LEFTMOUSE_NM: c_int = termcap2key(KS_EXTRA as c_int, KE_LEFTMOUSE_NM as c_int);
pub const K_LEFTDRAG: c_int = termcap2key(KS_EXTRA as c_int, KE_LEFTDRAG as c_int);
pub const K_LEFTRELEASE: c_int = termcap2key(KS_EXTRA as c_int, KE_LEFTRELEASE as c_int);
pub const K_LEFTRELEASE_NM: c_int = termcap2key(KS_EXTRA as c_int, KE_LEFTRELEASE_NM as c_int);
pub const K_MOUSEMOVE: c_int = termcap2key(KS_EXTRA as c_int, KE_MOUSEMOVE as c_int);
pub const K_MIDDLEMOUSE: c_int = termcap2key(KS_EXTRA as c_int, KE_MIDDLEMOUSE as c_int);
pub const K_MIDDLEDRAG: c_int = termcap2key(KS_EXTRA as c_int, KE_MIDDLEDRAG as c_int);
pub const K_MIDDLERELEASE: c_int = termcap2key(KS_EXTRA as c_int, KE_MIDDLERELEASE as c_int);
pub const K_RIGHTMOUSE: c_int = termcap2key(KS_EXTRA as c_int, KE_RIGHTMOUSE as c_int);
pub const K_RIGHTDRAG: c_int = termcap2key(KS_EXTRA as c_int, KE_RIGHTDRAG as c_int);
pub const K_RIGHTRELEASE: c_int = termcap2key(KS_EXTRA as c_int, KE_RIGHTRELEASE as c_int);
pub const K_X1MOUSE: c_int = termcap2key(KS_EXTRA as c_int, KE_X1MOUSE as c_int);
pub const K_X1DRAG: c_int = termcap2key(KS_EXTRA as c_int, KE_X1DRAG as c_int);
pub const K_X1RELEASE: c_int = termcap2key(KS_EXTRA as c_int, KE_X1RELEASE as c_int);
pub const K_X2MOUSE: c_int = termcap2key(KS_EXTRA as c_int, KE_X2MOUSE as c_int);
pub const K_X2DRAG: c_int = termcap2key(KS_EXTRA as c_int, KE_X2DRAG as c_int);
pub const K_X2RELEASE: c_int = termcap2key(KS_EXTRA as c_int, KE_X2RELEASE as c_int);

pub const K_IGNORE: c_int = termcap2key(KS_EXTRA as c_int, KE_IGNORE as c_int);
pub const K_NOP: c_int = termcap2key(KS_EXTRA as c_int, KE_NOP as c_int);
pub const K_MOUSEDOWN: c_int = termcap2key(KS_EXTRA as c_int, KE_MOUSEDOWN as c_int);
pub const K_MOUSEUP: c_int = termcap2key(KS_EXTRA as c_int, KE_MOUSEUP as c_int);
pub const K_MOUSELEFT: c_int = termcap2key(KS_EXTRA as c_int, KE_MOUSELEFT as c_int);
pub const K_MOUSERIGHT: c_int = termcap2key(KS_EXTRA as c_int, KE_MOUSERIGHT as c_int);
pub const K_SNR: c_int = termcap2key(KS_EXTRA as c_int, KE_SNR as c_int);
pub const K_PLUG: c_int = termcap2key(KS_EXTRA as c_int, KE_PLUG as c_int);
pub const K_DROP: c_int = termcap2key(KS_EXTRA as c_int, KE_DROP as c_int);
pub const K_COMMAND: c_int = termcap2key(KS_EXTRA as c_int, KE_COMMAND as c_int);

pub const MOD_MASK_SHIFT: u16 = 0x02;
pub const MOD_MASK_CTRL: u16 = 0x04;
pub const MOD_MASK_ALT: u16 = 0x08;
pub const MOD_MASK_META: u16 = 0x10;
pub const MOD_MASK_2CLICK: u16 = 0x20;
pub const MOD_MASK_3CLICK: u16 = 0x40;
pub const MOD_MASK_4CLICK: u16 = 0x60;
pub const MOD_MASK_CMD: u16 = 0x80;
pub const MOD_MASK_MULTI_CLICK: u16 = MOD_MASK_2CLICK | MOD_MASK_3CLICK | MOD_MASK_4CLICK;

pub const MAX_KEY_NAME_LEN: usize = 32;

pub const REPTERM_FROM_PART: c_int = 1;
pub const REPTERM_DO_LT: c_int = 2;
pub const REPTERM_NO_SPECIAL: c_int = 4;
pub const REPTERM_NO_SIMPLIFY: c_int = 8;

pub const FSK_KEYCODE: c_int = 0x01;
pub const FSK_KEEP_X_KEY: c_int = 0x02;
pub const FSK_IN_STRING: c_int = 0x04;
pub const FSK_SIMPLIFY: c_int = 0x08;

const K_CPO_BSLASH: c_int = b'B' as c_int;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct modmasktable {
    pub mod_mask: u16,
    pub mod_flag: u16,
    pub name: c_char,
}

#[repr(C)]
#[derive(Default)]
pub struct keychord {
    pub mods: c_int,
    pub key_mem: c_char,
    pub key: NvimString,
    pub key_alt: NvimString,
}

impl keychord {
    pub const fn new() -> Self {
        Self {
            mods: 0,
            key_mem: 0,
            key: NvimString::new(),
            key_alt: NvimString::new(),
        }
    }
}

pub static mod_mask_table: [modmasktable; 10] = [
    modmasktable {
        mod_mask: MOD_MASK_ALT,
        mod_flag: MOD_MASK_ALT,
        name: b'M' as c_char,
    },
    modmasktable {
        mod_mask: MOD_MASK_META,
        mod_flag: MOD_MASK_META,
        name: b'T' as c_char,
    },
    modmasktable {
        mod_mask: MOD_MASK_CTRL,
        mod_flag: MOD_MASK_CTRL,
        name: b'C' as c_char,
    },
    modmasktable {
        mod_mask: MOD_MASK_SHIFT,
        mod_flag: MOD_MASK_SHIFT,
        name: b'S' as c_char,
    },
    modmasktable {
        mod_mask: MOD_MASK_MULTI_CLICK,
        mod_flag: MOD_MASK_2CLICK,
        name: b'2' as c_char,
    },
    modmasktable {
        mod_mask: MOD_MASK_MULTI_CLICK,
        mod_flag: MOD_MASK_3CLICK,
        name: b'3' as c_char,
    },
    modmasktable {
        mod_mask: MOD_MASK_MULTI_CLICK,
        mod_flag: MOD_MASK_4CLICK,
        name: b'4' as c_char,
    },
    modmasktable {
        mod_mask: MOD_MASK_CMD,
        mod_flag: MOD_MASK_CMD,
        name: b'D' as c_char,
    },
    // 'A' must be the last one
    modmasktable {
        mod_mask: MOD_MASK_ALT,
        mod_flag: MOD_MASK_ALT,
        name: b'A' as c_char,
    },
    modmasktable {
        mod_mask: 0,
        mod_flag: 0,
        name: 0,
    },
];

const MOD_KEYS_ENTRY_SIZE: usize = 5;

const MODIFIER_KEYS_TABLE: &[u8] = &[
    // mod mask, with modifier (2), without modifier (2)
    MOD_MASK_SHIFT as u8,
    b'&',
    b'9',
    b'@',
    b'1', // begin
    MOD_MASK_SHIFT as u8,
    b'&',
    b'0',
    b'@',
    b'2', // cancel
    MOD_MASK_SHIFT as u8,
    b'*',
    b'1',
    b'@',
    b'4', // command
    MOD_MASK_SHIFT as u8,
    b'*',
    b'2',
    b'@',
    b'5', // copy
    MOD_MASK_SHIFT as u8,
    b'*',
    b'3',
    b'@',
    b'6', // create
    MOD_MASK_SHIFT as u8,
    b'*',
    b'4',
    b'k',
    b'D', // delete char
    MOD_MASK_SHIFT as u8,
    b'*',
    b'5',
    b'k',
    b'L', // delete line
    MOD_MASK_SHIFT as u8,
    b'*',
    b'7',
    b'@',
    b'7', // end
    MOD_MASK_CTRL as u8,
    KS_EXTRA,
    KE_C_END,
    b'@',
    b'7', // end
    MOD_MASK_SHIFT as u8,
    b'*',
    b'9',
    b'@',
    b'9', // exit
    MOD_MASK_SHIFT as u8,
    b'*',
    b'0',
    b'@',
    b'0', // find
    MOD_MASK_SHIFT as u8,
    b'#',
    b'1',
    b'%',
    b'1', // help
    MOD_MASK_SHIFT as u8,
    b'#',
    b'2',
    b'k',
    b'h', // home
    MOD_MASK_CTRL as u8,
    KS_EXTRA,
    KE_C_HOME,
    b'k',
    b'h', // home
    MOD_MASK_SHIFT as u8,
    b'#',
    b'3',
    b'k',
    b'I', // insert
    MOD_MASK_SHIFT as u8,
    b'#',
    b'4',
    b'k',
    b'l', // left arrow
    MOD_MASK_CTRL as u8,
    KS_EXTRA,
    KE_C_LEFT,
    b'k',
    b'l', // left arrow
    MOD_MASK_SHIFT as u8,
    b'%',
    b'a',
    b'%',
    b'3', // message
    MOD_MASK_SHIFT as u8,
    b'%',
    b'b',
    b'%',
    b'4', // move
    MOD_MASK_SHIFT as u8,
    b'%',
    b'c',
    b'%',
    b'5', // next
    MOD_MASK_SHIFT as u8,
    b'%',
    b'd',
    b'%',
    b'7', // options
    MOD_MASK_SHIFT as u8,
    b'%',
    b'e',
    b'%',
    b'8', // previous
    MOD_MASK_SHIFT as u8,
    b'%',
    b'f',
    b'%',
    b'9', // print
    MOD_MASK_SHIFT as u8,
    b'%',
    b'g',
    b'%',
    b'0', // redo
    MOD_MASK_SHIFT as u8,
    b'%',
    b'h',
    b'&',
    b'3', // replace
    MOD_MASK_SHIFT as u8,
    b'%',
    b'i',
    b'k',
    b'r', // right arr.
    MOD_MASK_CTRL as u8,
    KS_EXTRA,
    KE_C_RIGHT,
    b'k',
    b'r', // right arr.
    MOD_MASK_SHIFT as u8,
    b'%',
    b'j',
    b'&',
    b'5', // resume
    MOD_MASK_SHIFT as u8,
    b'!',
    b'1',
    b'&',
    b'6', // save
    MOD_MASK_SHIFT as u8,
    b'!',
    b'2',
    b'&',
    b'7', // suspend
    MOD_MASK_SHIFT as u8,
    b'!',
    b'3',
    b'&',
    b'8', // undo
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_UP,
    b'k',
    b'u', // up arrow
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_DOWN,
    b'k',
    b'd', // down arrow
    // vt100 F1
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_XF1,
    KS_EXTRA,
    KE_XF1,
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_XF2,
    KS_EXTRA,
    KE_XF2,
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_XF3,
    KS_EXTRA,
    KE_XF3,
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_XF4,
    KS_EXTRA,
    KE_XF4,
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F1,
    b'k',
    b'1', // F1
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F2,
    b'k',
    b'2',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F3,
    b'k',
    b'3',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F4,
    b'k',
    b'4',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F5,
    b'k',
    b'5',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F6,
    b'k',
    b'6',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F7,
    b'k',
    b'7',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F8,
    b'k',
    b'8',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F9,
    b'k',
    b'9',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F10,
    b'k',
    b';', // F10
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F11,
    b'F',
    b'1',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F12,
    b'F',
    b'2',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F13,
    b'F',
    b'3',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F14,
    b'F',
    b'4',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F15,
    b'F',
    b'5',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F16,
    b'F',
    b'6',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F17,
    b'F',
    b'7',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F18,
    b'F',
    b'8',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F19,
    b'F',
    b'9',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F20,
    b'F',
    b'A',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F21,
    b'F',
    b'B',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F22,
    b'F',
    b'C',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F23,
    b'F',
    b'D',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F24,
    b'F',
    b'E',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F25,
    b'F',
    b'F',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F26,
    b'F',
    b'G',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F27,
    b'F',
    b'H',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F28,
    b'F',
    b'I',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F29,
    b'F',
    b'J',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F30,
    b'F',
    b'K',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F31,
    b'F',
    b'L',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F32,
    b'F',
    b'M',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F33,
    b'F',
    b'N',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F34,
    b'F',
    b'O',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F35,
    b'F',
    b'P',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F36,
    b'F',
    b'Q',
    MOD_MASK_SHIFT as u8,
    KS_EXTRA,
    KE_S_F37,
    b'F',
    b'R',
    // TAB pseudo code
    MOD_MASK_SHIFT as u8,
    b'k',
    b'B',
    KS_EXTRA,
    KE_TAB,
    NUL,
];

#[derive(Clone, Copy)]
struct MouseTableEntry {
    pseudo_code: c_int,
    button: c_int,
    is_click: bool,
    is_drag: bool,
}

const MOUSE_TABLE: &[MouseTableEntry] = &[
    MouseTableEntry {
        pseudo_code: KE_LEFTMOUSE as c_int,
        button: MOUSE_LEFT,
        is_click: true,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_LEFTDRAG as c_int,
        button: MOUSE_LEFT,
        is_click: false,
        is_drag: true,
    },
    MouseTableEntry {
        pseudo_code: KE_LEFTRELEASE as c_int,
        button: MOUSE_LEFT,
        is_click: false,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_MIDDLEMOUSE as c_int,
        button: MOUSE_MIDDLE,
        is_click: true,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_MIDDLEDRAG as c_int,
        button: MOUSE_MIDDLE,
        is_click: false,
        is_drag: true,
    },
    MouseTableEntry {
        pseudo_code: KE_MIDDLERELEASE as c_int,
        button: MOUSE_MIDDLE,
        is_click: false,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_RIGHTMOUSE as c_int,
        button: MOUSE_RIGHT,
        is_click: true,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_RIGHTDRAG as c_int,
        button: MOUSE_RIGHT,
        is_click: false,
        is_drag: true,
    },
    MouseTableEntry {
        pseudo_code: KE_RIGHTRELEASE as c_int,
        button: MOUSE_RIGHT,
        is_click: false,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_X1MOUSE as c_int,
        button: MOUSE_X1,
        is_click: true,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_X1DRAG as c_int,
        button: MOUSE_X1,
        is_click: false,
        is_drag: true,
    },
    MouseTableEntry {
        pseudo_code: KE_X1RELEASE as c_int,
        button: MOUSE_X1,
        is_click: false,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_X2MOUSE as c_int,
        button: MOUSE_X2,
        is_click: true,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_X2DRAG as c_int,
        button: MOUSE_X2,
        is_click: false,
        is_drag: true,
    },
    MouseTableEntry {
        pseudo_code: KE_X2RELEASE as c_int,
        button: MOUSE_X2,
        is_click: false,
        is_drag: false,
    },
    MouseTableEntry {
        pseudo_code: KE_MOUSEMOVE as c_int,
        button: MOUSE_RELEASE,
        is_click: false,
        is_drag: true,
    },
    MouseTableEntry {
        pseudo_code: KE_IGNORE as c_int,
        button: MOUSE_RELEASE,
        is_click: false,
        is_drag: false,
    },
];

#[derive(Clone, Copy)]
struct KeyNameEntry {
    key: c_int,
    is_alt: bool,
    name: &'static CStr,
}

const KEY_NAMES_TABLE: &[KeyNameEntry; 187] = &[
    KeyNameEntry {
        key: K_K0,
        is_alt: false,
        name: c"k0",
    },
    KeyNameEntry {
        key: K_F1,
        is_alt: false,
        name: c"F1",
    },
    KeyNameEntry {
        key: K_K1,
        is_alt: false,
        name: c"k1",
    },
    KeyNameEntry {
        key: K_F2,
        is_alt: false,
        name: c"F2",
    },
    KeyNameEntry {
        key: K_K2,
        is_alt: false,
        name: c"k2",
    },
    KeyNameEntry {
        key: K_F3,
        is_alt: false,
        name: c"F3",
    },
    KeyNameEntry {
        key: K_K3,
        is_alt: false,
        name: c"k3",
    },
    KeyNameEntry {
        key: K_F4,
        is_alt: false,
        name: c"F4",
    },
    KeyNameEntry {
        key: K_K4,
        is_alt: false,
        name: c"k4",
    },
    KeyNameEntry {
        key: K_F5,
        is_alt: false,
        name: c"F5",
    },
    KeyNameEntry {
        key: K_K5,
        is_alt: false,
        name: c"k5",
    },
    KeyNameEntry {
        key: K_F6,
        is_alt: false,
        name: c"F6",
    },
    KeyNameEntry {
        key: K_K6,
        is_alt: false,
        name: c"k6",
    },
    KeyNameEntry {
        key: K_F7,
        is_alt: false,
        name: c"F7",
    },
    KeyNameEntry {
        key: K_K7,
        is_alt: false,
        name: c"k7",
    },
    KeyNameEntry {
        key: K_F8,
        is_alt: false,
        name: c"F8",
    },
    KeyNameEntry {
        key: K_K8,
        is_alt: false,
        name: c"k8",
    },
    KeyNameEntry {
        key: K_F9,
        is_alt: false,
        name: c"F9",
    },
    KeyNameEntry {
        key: K_K9,
        is_alt: false,
        name: c"k9",
    },
    KeyNameEntry {
        key: NL,
        is_alt: true,
        name: c"LF",
    },
    KeyNameEntry {
        key: NL,
        is_alt: false,
        name: c"NL",
    },
    KeyNameEntry {
        key: K_UP,
        is_alt: false,
        name: c"Up",
    },
    KeyNameEntry {
        key: CAR,
        is_alt: false,
        name: c"CR",
    },
    KeyNameEntry {
        key: K_BS,
        is_alt: false,
        name: c"BS",
    },
    KeyNameEntry {
        key: b'<' as c_int,
        is_alt: false,
        name: c"lt",
    },
    KeyNameEntry {
        key: K_F10,
        is_alt: false,
        name: c"F10",
    },
    KeyNameEntry {
        key: K_F20,
        is_alt: false,
        name: c"F20",
    },
    KeyNameEntry {
        key: K_F30,
        is_alt: false,
        name: c"F30",
    },
    KeyNameEntry {
        key: K_F40,
        is_alt: false,
        name: c"F40",
    },
    KeyNameEntry {
        key: K_F50,
        is_alt: false,
        name: c"F50",
    },
    KeyNameEntry {
        key: K_F60,
        is_alt: false,
        name: c"F60",
    },
    KeyNameEntry {
        key: K_KINS,
        is_alt: true,
        name: c"KP0",
    },
    KeyNameEntry {
        key: K_F11,
        is_alt: false,
        name: c"F11",
    },
    KeyNameEntry {
        key: K_F21,
        is_alt: false,
        name: c"F21",
    },
    KeyNameEntry {
        key: K_F31,
        is_alt: false,
        name: c"F31",
    },
    KeyNameEntry {
        key: K_F41,
        is_alt: false,
        name: c"F41",
    },
    KeyNameEntry {
        key: K_F51,
        is_alt: false,
        name: c"F51",
    },
    KeyNameEntry {
        key: K_F61,
        is_alt: false,
        name: c"F61",
    },
    KeyNameEntry {
        key: K_KEND,
        is_alt: true,
        name: c"KP1",
    },
    KeyNameEntry {
        key: K_XF1,
        is_alt: false,
        name: c"xF1",
    },
    KeyNameEntry {
        key: K_F12,
        is_alt: false,
        name: c"F12",
    },
    KeyNameEntry {
        key: K_F22,
        is_alt: false,
        name: c"F22",
    },
    KeyNameEntry {
        key: K_F32,
        is_alt: false,
        name: c"F32",
    },
    KeyNameEntry {
        key: K_F42,
        is_alt: false,
        name: c"F42",
    },
    KeyNameEntry {
        key: K_F52,
        is_alt: false,
        name: c"F52",
    },
    KeyNameEntry {
        key: K_F62,
        is_alt: false,
        name: c"F62",
    },
    KeyNameEntry {
        key: K_KDOWN,
        is_alt: true,
        name: c"KP2",
    },
    KeyNameEntry {
        key: K_XF2,
        is_alt: false,
        name: c"xF2",
    },
    KeyNameEntry {
        key: K_F13,
        is_alt: false,
        name: c"F13",
    },
    KeyNameEntry {
        key: K_F23,
        is_alt: false,
        name: c"F23",
    },
    KeyNameEntry {
        key: K_F33,
        is_alt: false,
        name: c"F33",
    },
    KeyNameEntry {
        key: K_F43,
        is_alt: false,
        name: c"F43",
    },
    KeyNameEntry {
        key: K_F53,
        is_alt: false,
        name: c"F53",
    },
    KeyNameEntry {
        key: K_F63,
        is_alt: false,
        name: c"F63",
    },
    KeyNameEntry {
        key: K_KPAGEDOWN,
        is_alt: true,
        name: c"KP3",
    },
    KeyNameEntry {
        key: K_XF3,
        is_alt: false,
        name: c"xF3",
    },
    KeyNameEntry {
        key: K_F14,
        is_alt: false,
        name: c"F14",
    },
    KeyNameEntry {
        key: K_F24,
        is_alt: false,
        name: c"F24",
    },
    KeyNameEntry {
        key: K_F34,
        is_alt: false,
        name: c"F34",
    },
    KeyNameEntry {
        key: K_F44,
        is_alt: false,
        name: c"F44",
    },
    KeyNameEntry {
        key: K_F54,
        is_alt: false,
        name: c"F54",
    },
    KeyNameEntry {
        key: K_KLEFT,
        is_alt: true,
        name: c"KP4",
    },
    KeyNameEntry {
        key: K_XF4,
        is_alt: false,
        name: c"xF4",
    },
    KeyNameEntry {
        key: K_F15,
        is_alt: false,
        name: c"F15",
    },
    KeyNameEntry {
        key: K_F25,
        is_alt: false,
        name: c"F25",
    },
    KeyNameEntry {
        key: K_F35,
        is_alt: false,
        name: c"F35",
    },
    KeyNameEntry {
        key: K_F45,
        is_alt: false,
        name: c"F45",
    },
    KeyNameEntry {
        key: K_F55,
        is_alt: false,
        name: c"F55",
    },
    KeyNameEntry {
        key: K_KORIGIN,
        is_alt: true,
        name: c"KP5",
    },
    KeyNameEntry {
        key: K_F16,
        is_alt: false,
        name: c"F16",
    },
    KeyNameEntry {
        key: K_F26,
        is_alt: false,
        name: c"F26",
    },
    KeyNameEntry {
        key: K_F36,
        is_alt: false,
        name: c"F36",
    },
    KeyNameEntry {
        key: K_F46,
        is_alt: false,
        name: c"F46",
    },
    KeyNameEntry {
        key: K_F56,
        is_alt: false,
        name: c"F56",
    },
    KeyNameEntry {
        key: K_KRIGHT,
        is_alt: true,
        name: c"KP6",
    },
    KeyNameEntry {
        key: K_F17,
        is_alt: false,
        name: c"F17",
    },
    KeyNameEntry {
        key: K_F27,
        is_alt: false,
        name: c"F27",
    },
    KeyNameEntry {
        key: K_F37,
        is_alt: false,
        name: c"F37",
    },
    KeyNameEntry {
        key: K_F47,
        is_alt: false,
        name: c"F47",
    },
    KeyNameEntry {
        key: K_F57,
        is_alt: false,
        name: c"F57",
    },
    KeyNameEntry {
        key: K_KHOME,
        is_alt: true,
        name: c"KP7",
    },
    KeyNameEntry {
        key: K_F18,
        is_alt: false,
        name: c"F18",
    },
    KeyNameEntry {
        key: K_F28,
        is_alt: false,
        name: c"F28",
    },
    KeyNameEntry {
        key: K_F38,
        is_alt: false,
        name: c"F38",
    },
    KeyNameEntry {
        key: K_F48,
        is_alt: false,
        name: c"F48",
    },
    KeyNameEntry {
        key: K_F58,
        is_alt: false,
        name: c"F58",
    },
    KeyNameEntry {
        key: K_KUP,
        is_alt: true,
        name: c"KP8",
    },
    KeyNameEntry {
        key: K_F19,
        is_alt: false,
        name: c"F19",
    },
    KeyNameEntry {
        key: K_F29,
        is_alt: false,
        name: c"F29",
    },
    KeyNameEntry {
        key: K_F39,
        is_alt: false,
        name: c"F39",
    },
    KeyNameEntry {
        key: K_F49,
        is_alt: false,
        name: c"F49",
    },
    KeyNameEntry {
        key: K_F59,
        is_alt: false,
        name: c"F59",
    },
    KeyNameEntry {
        key: K_KPAGEUP,
        is_alt: true,
        name: c"KP9",
    },
    KeyNameEntry {
        key: TAB,
        is_alt: false,
        name: c"Tab",
    },
    KeyNameEntry {
        key: K_TAB,
        is_alt: false,
        name: c"Tab",
    },
    KeyNameEntry {
        key: ESC,
        is_alt: false,
        name: c"Esc",
    },
    KeyNameEntry {
        key: K_COMMAND,
        is_alt: false,
        name: c"Cmd",
    },
    KeyNameEntry {
        key: K_END,
        is_alt: false,
        name: c"End",
    },
    KeyNameEntry {
        key: CSI,
        is_alt: false,
        name: c"CSI",
    },
    KeyNameEntry {
        key: K_DEL,
        is_alt: false,
        name: c"Del",
    },
    KeyNameEntry {
        key: K_ZERO,
        is_alt: false,
        name: c"Nul",
    },
    KeyNameEntry {
        key: K_KUP,
        is_alt: false,
        name: c"kUp",
    },
    KeyNameEntry {
        key: K_XUP,
        is_alt: false,
        name: c"xUp",
    },
    KeyNameEntry {
        key: b'|' as c_int,
        is_alt: false,
        name: c"Bar",
    },
    KeyNameEntry {
        key: K_SNR,
        is_alt: false,
        name: c"SNR",
    },
    KeyNameEntry {
        key: K_INS,
        is_alt: true,
        name: c"Ins",
    },
    KeyNameEntry {
        key: K_DOWN,
        is_alt: false,
        name: c"Down",
    },
    KeyNameEntry {
        key: K_DROP,
        is_alt: false,
        name: c"Drop",
    },
    KeyNameEntry {
        key: K_FIND,
        is_alt: false,
        name: c"Find",
    },
    KeyNameEntry {
        key: K_HELP,
        is_alt: false,
        name: c"Help",
    },
    KeyNameEntry {
        key: K_HOME,
        is_alt: false,
        name: c"Home",
    },
    KeyNameEntry {
        key: K_KDEL,
        is_alt: false,
        name: c"kDel",
    },
    KeyNameEntry {
        key: K_KEND,
        is_alt: false,
        name: c"kEnd",
    },
    KeyNameEntry {
        key: K_LEFT,
        is_alt: false,
        name: c"Left",
    },
    KeyNameEntry {
        key: K_PLUG,
        is_alt: false,
        name: c"Plug",
    },
    KeyNameEntry {
        key: K_UNDO,
        is_alt: false,
        name: c"Undo",
    },
    KeyNameEntry {
        key: K_XEND,
        is_alt: false,
        name: c"xEnd",
    },
    KeyNameEntry {
        key: K_ZEND,
        is_alt: false,
        name: c"zEnd",
    },
    KeyNameEntry {
        key: K_KDOWN,
        is_alt: false,
        name: c"kDown",
    },
    KeyNameEntry {
        key: K_XDOWN,
        is_alt: false,
        name: c"xDown",
    },
    KeyNameEntry {
        key: K_KHOME,
        is_alt: false,
        name: c"kHome",
    },
    KeyNameEntry {
        key: K_XHOME,
        is_alt: false,
        name: c"xHome",
    },
    KeyNameEntry {
        key: K_ZHOME,
        is_alt: false,
        name: c"zHome",
    },
    KeyNameEntry {
        key: K_RIGHT,
        is_alt: false,
        name: c"Right",
    },
    KeyNameEntry {
        key: K_KLEFT,
        is_alt: false,
        name: c"kLeft",
    },
    KeyNameEntry {
        key: K_XLEFT,
        is_alt: false,
        name: c"xLeft",
    },
    KeyNameEntry {
        key: CAR,
        is_alt: true,
        name: c"Enter",
    },
    KeyNameEntry {
        key: K_MOUSE,
        is_alt: false,
        name: c"Mouse",
    },
    KeyNameEntry {
        key: K_KDIVIDE,
        is_alt: true,
        name: c"KPDiv",
    },
    KeyNameEntry {
        key: K_KPLUS,
        is_alt: false,
        name: c"kPlus",
    },
    KeyNameEntry {
        key: b' ' as c_int,
        is_alt: false,
        name: c"Space",
    },
    KeyNameEntry {
        key: ESC,
        is_alt: true,
        name: c"Escape",
    },
    KeyNameEntry {
        key: K_X1DRAG,
        is_alt: false,
        name: c"X1Drag",
    },
    KeyNameEntry {
        key: K_X2DRAG,
        is_alt: false,
        name: c"X2Drag",
    },
    KeyNameEntry {
        key: K_PAGEUP,
        is_alt: false,
        name: c"PageUp",
    },
    KeyNameEntry {
        key: K_KMINUS,
        is_alt: false,
        name: c"kMinus",
    },
    KeyNameEntry {
        key: K_KRIGHT,
        is_alt: false,
        name: c"kRight",
    },
    KeyNameEntry {
        key: K_XRIGHT,
        is_alt: false,
        name: c"xRight",
    },
    KeyNameEntry {
        key: b'\\' as c_int,
        is_alt: false,
        name: c"Bslash",
    },
    KeyNameEntry {
        key: K_DEL,
        is_alt: true,
        name: c"Delete",
    },
    KeyNameEntry {
        key: K_KSELECT,
        is_alt: false,
        name: c"Select",
    },
    KeyNameEntry {
        key: K_KMULTIPLY,
        is_alt: true,
        name: c"KPMult",
    },
    KeyNameEntry {
        key: K_IGNORE,
        is_alt: false,
        name: c"Ignore",
    },
    KeyNameEntry {
        key: K_KENTER,
        is_alt: false,
        name: c"kEnter",
    },
    KeyNameEntry {
        key: K_KCOMMA,
        is_alt: false,
        name: c"kComma",
    },
    KeyNameEntry {
        key: K_KPOINT,
        is_alt: false,
        name: c"kPoint",
    },
    KeyNameEntry {
        key: K_KPLUS,
        is_alt: true,
        name: c"KPPlus",
    },
    KeyNameEntry {
        key: K_KEQUAL,
        is_alt: false,
        name: c"kEqual",
    },
    KeyNameEntry {
        key: K_INS,
        is_alt: false,
        name: c"Insert",
    },
    KeyNameEntry {
        key: CAR,
        is_alt: true,
        name: c"Return",
    },
    KeyNameEntry {
        key: K_KPAGEUP,
        is_alt: false,
        name: c"kPageUp",
    },
    KeyNameEntry {
        key: K_KCOMMA,
        is_alt: true,
        name: c"KPComma",
    },
    KeyNameEntry {
        key: K_KENTER,
        is_alt: true,
        name: c"KPEnter",
    },
    KeyNameEntry {
        key: K_KDIVIDE,
        is_alt: false,
        name: c"kDivide",
    },
    KeyNameEntry {
        key: K_KMINUS,
        is_alt: true,
        name: c"KPMinus",
    },
    KeyNameEntry {
        key: K_X1MOUSE,
        is_alt: false,
        name: c"X1Mouse",
    },
    KeyNameEntry {
        key: K_X2MOUSE,
        is_alt: false,
        name: c"X2Mouse",
    },
    KeyNameEntry {
        key: K_KINS,
        is_alt: false,
        name: c"kInsert",
    },
    KeyNameEntry {
        key: K_KORIGIN,
        is_alt: false,
        name: c"kOrigin",
    },
    KeyNameEntry {
        key: K_MOUSEUP,
        is_alt: true,
        name: c"MouseUp",
    },
    KeyNameEntry {
        key: NL,
        is_alt: true,
        name: c"NewLine",
    },
    KeyNameEntry {
        key: K_KEQUAL,
        is_alt: true,
        name: c"KPEquals",
    },
    KeyNameEntry {
        key: K_LEFTDRAG,
        is_alt: false,
        name: c"LeftDrag",
    },
    KeyNameEntry {
        key: K_PAGEDOWN,
        is_alt: false,
        name: c"PageDown",
    },
    KeyNameEntry {
        key: NL,
        is_alt: true,
        name: c"LineFeed",
    },
    KeyNameEntry {
        key: K_KDEL,
        is_alt: true,
        name: c"KPPeriod",
    },
    KeyNameEntry {
        key: K_BS,
        is_alt: true,
        name: c"BackSpace",
    },
    KeyNameEntry {
        key: K_KMULTIPLY,
        is_alt: false,
        name: c"kMultiply",
    },
    KeyNameEntry {
        key: K_KPAGEDOWN,
        is_alt: false,
        name: c"kPageDown",
    },
    KeyNameEntry {
        key: K_LEFTMOUSE,
        is_alt: false,
        name: c"LeftMouse",
    },
    KeyNameEntry {
        key: K_MOUSEDOWN,
        is_alt: true,
        name: c"MouseDown",
    },
    KeyNameEntry {
        key: K_MOUSEMOVE,
        is_alt: false,
        name: c"MouseMove",
    },
    KeyNameEntry {
        key: K_RIGHTDRAG,
        is_alt: false,
        name: c"RightDrag",
    },
    KeyNameEntry {
        key: K_X1RELEASE,
        is_alt: false,
        name: c"X1Release",
    },
    KeyNameEntry {
        key: K_X2RELEASE,
        is_alt: false,
        name: c"X2Release",
    },
    KeyNameEntry {
        key: K_MIDDLEDRAG,
        is_alt: false,
        name: c"MiddleDrag",
    },
    KeyNameEntry {
        key: K_RIGHTMOUSE,
        is_alt: false,
        name: c"RightMouse",
    },
    KeyNameEntry {
        key: K_MIDDLEMOUSE,
        is_alt: false,
        name: c"MiddleMouse",
    },
    KeyNameEntry {
        key: K_LEFTMOUSE_NM,
        is_alt: false,
        name: c"LeftMouseNM",
    },
    KeyNameEntry {
        key: K_LEFTRELEASE,
        is_alt: false,
        name: c"LeftRelease",
    },
    KeyNameEntry {
        key: K_RIGHTRELEASE,
        is_alt: false,
        name: c"RightRelease",
    },
    KeyNameEntry {
        key: K_LEFTRELEASE_NM,
        is_alt: false,
        name: c"LeftReleaseNM",
    },
    KeyNameEntry {
        key: K_MIDDLERELEASE,
        is_alt: false,
        name: c"MiddleRelease",
    },
    KeyNameEntry {
        key: K_MOUSEDOWN,
        is_alt: false,
        name: c"ScrollWheelUp",
    },
    KeyNameEntry {
        key: K_MOUSEUP,
        is_alt: false,
        name: c"ScrollWheelDown",
    },
    KeyNameEntry {
        key: K_MOUSERIGHT,
        is_alt: false,
        name: c"ScrollWheelLeft",
    },
    KeyNameEntry {
        key: K_MOUSELEFT,
        is_alt: false,
        name: c"ScrollWheelRight",
    },
];

use crate::charset::{ascii_isident, toupper_asc};
use crate::strings::vim_strnicmp_asc;

fn get_special_key_code_hash(name: &[u8]) -> isize {
    let (low, high): (usize, usize) = match name.len() {
        2 => match name[1] {
            b'0' => (0, 1),
            b'1' => (1, 3),
            b'2' => (3, 5),
            b'3' => (5, 7),
            b'4' => (7, 9),
            b'5' => (9, 11),
            b'6' => (11, 13),
            b'7' => (13, 15),
            b'8' => (15, 17),
            b'9' => (17, 19),
            b'F' | b'f' => (19, 20),
            b'L' | b'l' => (20, 21),
            b'P' | b'p' => (21, 22),
            b'R' | b'r' => (22, 23),
            b'S' | b's' => (23, 24),
            b'T' | b't' => (24, 25),
            _ => (0, 0),
        },
        3 => match name[2] {
            b'0' => (25, 32),
            b'1' => (32, 40),
            b'2' => (40, 48),
            b'3' => (48, 56),
            b'4' => (56, 63),
            b'5' => (63, 69),
            b'6' => (69, 75),
            b'7' => (75, 81),
            b'8' => (81, 87),
            b'9' => (87, 93),
            b'B' | b'b' => (93, 95),
            b'C' | b'c' => (95, 96),
            b'D' | b'd' => (96, 98),
            b'I' | b'i' => (98, 99),
            b'L' | b'l' => (99, 101),
            b'P' | b'p' => (101, 103),
            b'R' | b'r' => (103, 105),
            b'S' | b's' => (105, 106),
            _ => (0, 0),
        },
        4 => match name[0] {
            b'D' | b'd' => (106, 108),
            b'F' | b'f' => (108, 109),
            b'H' | b'h' => (109, 111),
            b'K' | b'k' => (111, 113),
            b'L' | b'l' => (113, 114),
            b'P' | b'p' => (114, 115),
            b'U' | b'u' => (115, 116),
            b'X' | b'x' => (116, 117),
            b'Z' | b'z' => (117, 118),
            _ => (0, 0),
        },
        5 => match name[1] {
            b'D' | b'd' => (118, 120),
            b'H' | b'h' => (120, 123),
            b'I' | b'i' => (123, 124),
            b'L' | b'l' => (124, 126),
            b'N' | b'n' => (126, 127),
            b'O' | b'o' => (127, 128),
            b'P' | b'p' => (128, 131),
            _ => (0, 0),
        },
        6 => match name[2] {
            b'C' | b'c' => (131, 132),
            b'D' | b'd' => (132, 134),
            b'G' | b'g' => (134, 135),
            b'I' | b'i' => (135, 138),
            b'L' | b'l' => (138, 141),
            b'M' | b'm' => (141, 142),
            b'N' | b'n' => (142, 144),
            b'O' | b'o' => (144, 146),
            b'P' | b'p' => (146, 147),
            b'Q' | b'q' => (147, 148),
            b'S' | b's' => (148, 149),
            b'T' | b't' => (149, 150),
            _ => (0, 0),
        },
        7 => match name[2] {
            b'A' | b'a' => (150, 151),
            b'C' | b'c' => (151, 152),
            b'E' | b'e' => (152, 153),
            b'I' | b'i' => (153, 154),
            b'M' | b'm' => (154, 157),
            b'N' | b'n' => (157, 158),
            b'R' | b'r' => (158, 159),
            b'U' | b'u' => (159, 160),
            b'W' | b'w' => (160, 161),
            _ => (0, 0),
        },
        8 => match name[2] {
            b'E' | b'e' => (161, 162),
            b'F' | b'f' => (162, 163),
            b'G' | b'g' => (163, 164),
            b'N' | b'n' => (164, 165),
            b'P' | b'p' => (165, 166),
            _ => (0, 0),
        },
        9 => match name[0] {
            b'B' | b'b' => (166, 167),
            b'K' | b'k' => (167, 169),
            b'L' | b'l' => (169, 170),
            b'M' | b'm' => (170, 172),
            b'R' | b'r' => (172, 173),
            b'X' | b'x' => (173, 175),
            _ => (0, 0),
        },
        10 => match name[0] {
            b'M' | b'm' => (175, 176),
            b'R' | b'r' => (176, 177),
            _ => (0, 0),
        },
        11 => match name[4] {
            b'L' | b'l' => (177, 178),
            b'M' | b'm' => (178, 179),
            b'R' | b'r' => (179, 180),
            _ => (0, 0),
        },
        12 => (180, 181),
        13 => match name[0] {
            b'L' | b'l' => (181, 182),
            b'M' | b'm' => (182, 183),
            b'S' | b's' => (183, 184),
            _ => (0, 0),
        },
        15 => match name[11] {
            b'D' | b'd' => (184, 185),
            b'L' | b'l' => (185, 186),
            _ => (0, 0),
        },
        16 => (186, 187),
        _ => (0, 0),
    };
    for (i, entry) in KEY_NAMES_TABLE[low..high].iter().enumerate() {
        if entry.name.to_bytes().eq_ignore_ascii_case(name) {
            return (low + i) as isize;
        }
    }
    -1
}

/// Return the modifier mask bit (`MOD_MASK_*`) corresponding to mod name (`name_to_mod_mask`).
pub fn name_to_mod_mask(c: c_int) -> c_int {
    let c = toupper_asc(c);
    let mut i = 0;
    while mod_mask_table[i].mod_mask != 0 {
        if c == c_int::from(mod_mask_table[i].name as u8) {
            return c_int::from(mod_mask_table[i].mod_flag);
        }
        i += 1;
    }
    0
}

/// Check if there is a special key code for `key` with specified modifiers (`simplify_key`).
pub unsafe fn simplify_key(key: c_int, modifiers: *mut c_int) -> c_int {
    let mut mods = unsafe { *modifiers };
    if (mods & c_int::from(MOD_MASK_SHIFT | MOD_MASK_CTRL)) == 0 {
        return key;
    }

    // TAB is a special case.
    if key == TAB && (mods & c_int::from(MOD_MASK_SHIFT)) != 0 {
        unsafe {
            *modifiers = mods & !c_int::from(MOD_MASK_SHIFT);
        }
        return K_S_TAB;
    }
    let key0 = key2termcap0(key);
    let key1 = key2termcap1(key);
    let mut i = 0;
    while MODIFIER_KEYS_TABLE[i] != NUL {
        let entry_mod = c_int::from(MODIFIER_KEYS_TABLE[i]);
        if key0 == c_int::from(MODIFIER_KEYS_TABLE[i + 3])
            && key1 == c_int::from(MODIFIER_KEYS_TABLE[i + 4])
            && (mods & entry_mod) != 0
        {
            mods &= !entry_mod;
            unsafe {
                *modifiers = mods;
            }
            return termcap2key(
                c_int::from(MODIFIER_KEYS_TABLE[i + 1]),
                c_int::from(MODIFIER_KEYS_TABLE[i + 2]),
            );
        }
        i += MOD_KEYS_ENTRY_SIZE;
    }
    key
}

/// Inverse of `simplify_key()` (`unsimplify_key`).
pub unsafe fn unsimplify_key(c: c_int, modifiers: *mut c_int) -> c_int {
    if !is_special(c) {
        return c;
    }

    let key0 = key2termcap0(c);
    let key1 = key2termcap1(c);
    let mut i = 0;
    while MODIFIER_KEYS_TABLE[i] != NUL {
        if key0 == c_int::from(MODIFIER_KEYS_TABLE[i + 1])
            && key1 == c_int::from(MODIFIER_KEYS_TABLE[i + 2])
        {
            unsafe {
                *modifiers |= c_int::from(MODIFIER_KEYS_TABLE[i]);
            }
            return termcap2key(
                c_int::from(MODIFIER_KEYS_TABLE[i + 3]),
                c_int::from(MODIFIER_KEYS_TABLE[i + 4]),
            );
        }
        i += MOD_KEYS_ENTRY_SIZE;
    }
    c
}

/// Change `<xKey>` to `<Key>` (`handle_x_keys`).
pub fn handle_x_keys(key: c_int) -> c_int {
    match key {
        K_XUP => K_UP,
        K_XDOWN => K_DOWN,
        K_XLEFT => K_LEFT,
        K_XRIGHT => K_RIGHT,
        K_XHOME | K_ZHOME => K_HOME,
        K_XEND | K_ZEND => K_END,
        K_XF1 => K_F1,
        K_XF2 => K_F2,
        K_XF3 => K_F3,
        K_XF4 => K_F4,
        K_S_XF1 => K_S_F1,
        K_S_XF2 => K_S_F2,
        K_S_XF3 => K_S_F3,
        K_S_XF4 => K_S_F4,
        _ => key,
    }
}

static mut SPECIAL_KEY_STRING: [c_char; MAX_KEY_NAME_LEN + 1] = [0; MAX_KEY_NAME_LEN + 1];

/// Gets a key "chord" (key + modifiers) as a string, and optionally as structured `data`
/// (`get_special_key`).
pub unsafe fn get_special_key(
    mut c: c_int,
    mut modifiers: c_int,
    data: *mut keychord,
) -> *mut c_char {
    unsafe {
        let string = std::ptr::addr_of_mut!(SPECIAL_KEY_STRING).cast::<c_char>();
        let mut is_ascii_ctrl = false;

        *string = b'<' as c_char;
        let mut idx: usize = 1;

        // Key that stands for a normal character.
        if is_special(c) && key2termcap0(c) == KS_KEY as c_int {
            c = key2termcap1(c);
        }

        // Translate shifted special keys into unshifted keys and set modifier.
        // Same for CTRL and ALT modifiers.
        c = unsimplify_key(c, &mut modifiers);

        // try to find the key in the special key table
        let table_idx = find_special_key_in_table(c);

        // When not a known special key, and not a printable character, try to
        // extract modifiers.
        if c > 0 && utf_char2len(c) == 1 && table_idx < 0 && !vim_isprintc(c) && c < b' ' as c_int {
            c += b'@' as c_int;
            modifiers |= c_int::from(MOD_MASK_CTRL);
            is_ascii_ctrl = true;
        }

        if !data.is_null() {
            (*data).mods = 0;
        }

        // translate the modifier into a string
        let mut i = 0;
        while mod_mask_table[i].name != b'A' as c_char {
            let mask = c_int::from(mod_mask_table[i].mod_mask);
            let flag = c_int::from(mod_mask_table[i].mod_flag);
            if (modifiers & mask) == flag {
                *string.add(idx) = mod_mask_table[i].name;
                idx += 1;
                *string.add(idx) = b'-' as c_char;
                idx += 1;

                if !data.is_null() {
                    (*data).mods |= flag;
                }
            }
            i += 1;
        }

        if table_idx < 0 {
            // unknown special key, may output t_xx
            if is_special(c) {
                if !data.is_null() {
                    (*data).key_alt = NvimString {
                        data: std::ptr::null_mut(),
                        size: 0,
                    };
                    (*data).key = NvimString {
                        data: string.add(idx),
                        size: 4,
                    };
                }

                *string.add(idx) = b't' as c_char;
                idx += 1;
                *string.add(idx) = b'_' as c_char;
                idx += 1;
                *string.add(idx) = (key2termcap0(c) as u8) as c_char;
                idx += 1;
                *string.add(idx) = (key2termcap1(c) as u8) as c_char;
                idx += 1;
            } else {
                // Not a special key, only modifiers, output directly.
                let len = utf_char2len(c);
                if len == 1 && vim_isprintc(c) {
                    if !data.is_null() {
                        if (b'A' as c_int..=b'Z' as c_int).contains(&c) {
                            (*data).key_mem = (c as u8 + 32) as c_char;
                            (*data).key = NvimString {
                                data: std::ptr::addr_of_mut!((*data).key_mem),
                                size: 1,
                            };
                            if !is_ascii_ctrl {
                                (*data).mods |= c_int::from(MOD_MASK_SHIFT);
                            }
                            (*data).key_alt = NvimString {
                                data: string.add(idx),
                                size: len as usize,
                            };
                        } else {
                            (*data).key_alt = NvimString {
                                data: std::ptr::null_mut(),
                                size: 0,
                            };
                            (*data).key = NvimString {
                                data: string.add(idx),
                                size: len as usize,
                            };
                        }
                    }

                    *string.add(idx) = (c as u8) as c_char;
                    idx += 1;
                } else if len > 1 {
                    if !data.is_null() {
                        (*data).key = NvimString {
                            data: string.add(idx),
                            size: len as usize,
                        };
                        (*data).key_alt = NvimString {
                            data: std::ptr::null_mut(),
                            size: 0,
                        };
                    }

                    idx += utf_char2bytes(c, string.add(idx)) as usize;
                } else {
                    let mut s = transchar(c);

                    if !data.is_null() {
                        (*data).key_mem = (c as u8) as c_char;
                        (*data).key = NvimString {
                            data: std::ptr::addr_of_mut!((*data).key_mem),
                            size: 1,
                        };
                        (*data).key_alt = NvimString {
                            data: string.add(idx),
                            size: CStr::from_ptr(s).to_bytes().len(),
                        };
                    }

                    while *s != 0 {
                        *string.add(idx) = *s;
                        idx += 1;
                        s = s.add(1);
                    }
                }
            }
        } else {
            // use name of special key
            let entry = &KEY_NAMES_TABLE[table_idx as usize];
            let s_bytes = entry.name.to_bytes();
            let s_str = NvimString {
                data: entry.name.as_ptr().cast_mut(),
                size: s_bytes.len(),
            };

            if !data.is_null() {
                if (0..=0x7f).contains(&c) {
                    (*data).key_mem = (c as u8) as c_char;
                    (*data).key = NvimString {
                        data: std::ptr::addr_of_mut!((*data).key_mem),
                        size: 1,
                    };
                    (*data).key_alt = s_str;
                } else {
                    (*data).key = s_str;
                    (*data).key_alt = NvimString {
                        data: std::ptr::null_mut(),
                        size: 0,
                    };
                }
            }

            if s_bytes.len() + idx + 2 <= MAX_KEY_NAME_LEN {
                std::ptr::copy_nonoverlapping(entry.name.as_ptr(), string.add(idx), s_bytes.len());
                idx += s_bytes.len();
            }
        }
        *string.add(idx) = b'>' as c_char;
        idx += 1;
        *string.add(idx) = 0;

        string
    }
}

/// Return string representing the key "chord" (key + modifiers) (`get_special_key_name`).
pub unsafe fn get_special_key_name(c: c_int, modifiers: c_int) -> *mut c_char {
    unsafe { get_special_key(c, modifiers, std::ptr::null_mut()) }
}

/// Try translating a `<>` name ("keycode") (`trans_special`).
pub unsafe fn trans_special(
    srcp: *mut *const c_char,
    src_len: usize,
    dst: *mut c_char,
    flags: c_int,
    escape_ks: bool,
    did_simplify: *mut bool,
) -> c_uint {
    let mut modifiers: c_int = 0;
    let key = unsafe { find_special_key(srcp, src_len, &mut modifiers, flags, did_simplify) };
    if key == 0 {
        return 0;
    }

    unsafe { special_to_buf(key, modifiers, escape_ks, dst) }
}

/// Put the character sequence for `key` with `modifiers` into `dst` and return
/// the resulting length (`special_to_buf`).
pub unsafe fn special_to_buf(
    key: c_int,
    modifiers: c_int,
    escape_ks: bool,
    dst: *mut c_char,
) -> c_uint {
    unsafe {
        let mut dlen: c_uint = 0;

        // Put the appropriate modifier in a string.
        if modifiers != 0 {
            *dst.add(dlen as usize) = K_SPECIAL as c_char;
            dlen += 1;
            *dst.add(dlen as usize) = KS_MODIFIER as c_char;
            dlen += 1;
            *dst.add(dlen as usize) = (modifiers as u8) as c_char;
            dlen += 1;
        }

        if is_special(key) {
            *dst.add(dlen as usize) = K_SPECIAL as c_char;
            dlen += 1;
            *dst.add(dlen as usize) = (key2termcap0(key) as u8) as c_char;
            dlen += 1;
            *dst.add(dlen as usize) = (key2termcap1(key) as u8) as c_char;
            dlen += 1;
        } else if escape_ks {
            let after = add_char2buf(key, dst.add(dlen as usize));
            debug_assert!(after as usize >= dst as usize);
            dlen = (after as usize - dst as usize) as c_uint;
        } else {
            dlen += utf_char2bytes(key, dst.add(dlen as usize)) as c_uint;
        }

        dlen
    }
}

/// Try to include modifiers (except alt/meta) in the key (`extract_modifiers`).
fn extract_modifiers(
    mut key: c_int,
    modp: &mut c_int,
    simplify: bool,
    did_simplify: *mut bool,
) -> c_int {
    let mut modifiers = *modp;

    if (modifiers & c_int::from(MOD_MASK_SHIFT)) != 0 && ascii_isalpha(key) {
        key = toupper_asc(key);
        // With <C-S-a> we keep the shift modifier.
        // With <S-a>, <A-S-a> and <S-A> we don't keep the shift modifier.
        if (modifiers & c_int::from(MOD_MASK_CTRL)) == 0 {
            modifiers &= !c_int::from(MOD_MASK_SHIFT);
        }
    }

    // <C-H> and <C-h> mean the same thing, always use "H"
    if (modifiers & c_int::from(MOD_MASK_CTRL)) != 0 && ascii_isalpha(key) {
        key = toupper_asc(key);
    }

    if simplify
        && (modifiers & c_int::from(MOD_MASK_CTRL)) != 0
        && ((b'?' as c_int..=b'_' as c_int).contains(&key) || ascii_isalpha(key))
    {
        key = ctrl_chr(key);
        modifiers &= !c_int::from(MOD_MASK_CTRL);
        if key == 0 {
            // <C-@> is <Nul>
            key = K_ZERO;
        }
        if !did_simplify.is_null() {
            unsafe {
                *did_simplify = true;
            }
        }
    }

    *modp = modifiers;
    key
}

/// Try translating a `<>` name (`find_special_key`).
pub unsafe fn find_special_key(
    srcp: *mut *const c_char,
    src_len: usize,
    modp: *mut c_int,
    flags: c_int,
    did_simplify: *mut bool,
) -> c_int {
    unsafe {
        if src_len == 0 {
            return 0;
        }

        let mut src = *srcp;
        if *src != b'<' as c_char {
            return 0;
        }
        let end = src.add(src_len - 1);
        let in_string = (flags & FSK_IN_STRING) != 0;
        if (src as usize) < (end as usize) && *src.add(1) == b'*' as c_char {
            // <*xxx>: do not simplify
            src = src.add(1);
        }

        // Find end of modifier list
        let mut last_dash = src;
        let mut bp = src.add(1);
        while (bp as usize) <= (end as usize) && (*bp == b'-' as c_char || ascii_isident(*bp)) {
            if *bp == b'-' as c_char {
                last_dash = bp;
                if (bp.add(1) as usize) <= (end as usize) {
                    let end_minus_bp = end as usize - bp as usize;
                    let l = utfc_ptr2len_len(bp.add(1), (end_minus_bp + 1) as c_int) as usize;
                    if end_minus_bp > l
                        && !(in_string && *bp.add(1) == b'"' as c_char)
                        && *bp.add(l + 1) == b'>' as c_char
                    {
                        bp = bp.add(l);
                    } else if end_minus_bp > 2
                        && in_string
                        && *bp.add(1) == b'\\' as c_char
                        && *bp.add(2) == b'"' as c_char
                        && *bp.add(3) == b'>' as c_char
                    {
                        bp = bp.add(2);
                    }
                }
            }
            if (bp as usize) <= (end as usize) {
                let end_minus_bp = end as usize - bp as usize;
                if end_minus_bp > 3 && *bp == b't' as c_char && *bp.add(1) == b'_' as c_char {
                    bp = bp.add(3); // skip t_xx, xx may be '-' or '>'
                } else if end_minus_bp > 4 && vim_strnicmp_asc(bp, c"char-".as_ptr(), 5) == 0 {
                    let mut l: c_int = 0;
                    vim_str2nr(
                        bp.add(5),
                        std::ptr::null_mut(),
                        &mut l,
                        STR2NR_ALL,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        0,
                        true,
                        std::ptr::null_mut(),
                    );
                    if l == 0 {
                        emsg(e_invarg.as_ptr());
                        return 0;
                    }
                    bp = bp.add(l as usize + 5);
                    break;
                }
            }
            bp = bp.add(1);
        }

        if (bp as usize) <= (end as usize) && *bp == b'>' as c_char {
            // found matching '>'
            let end_of_name = bp.add(1);

            // Which modifiers are given?
            let mut modifiers: c_int = 0;
            bp = src.add(1);
            while (bp as usize) < (last_dash as usize) {
                if *bp != b'-' as c_char {
                    let bit = name_to_mod_mask(c_int::from(*bp as u8));
                    if bit == 0 {
                        break; // Illegal modifier name
                    }
                    modifiers |= bit;
                }
                bp = bp.add(1);
            }

            // Legal modifier name.
            if (bp as usize) >= (last_dash as usize) {
                let mut key: c_int;
                if vim_strnicmp_asc(last_dash.add(1), c"char-".as_ptr(), 5) == 0
                    && ascii_isdigit(*last_dash.add(6))
                {
                    // <Char-123> or <Char-033> or <Char-0x33>
                    let mut l: c_int = 0;
                    let mut n: u64 = 0;
                    vim_str2nr(
                        last_dash.add(6),
                        std::ptr::null_mut(),
                        &mut l,
                        STR2NR_ALL,
                        std::ptr::null_mut(),
                        &mut n,
                        0,
                        true,
                        std::ptr::null_mut(),
                    );
                    if l == 0 {
                        emsg(e_invarg.as_ptr());
                        return 0;
                    }
                    key = n as c_int;
                } else {
                    let off: usize;
                    let l: usize;

                    // Modifier with single letter, or special key name.
                    if in_string
                        && *last_dash.add(1) == b'\\' as c_char
                        && *last_dash.add(2) == b'"' as c_char
                    {
                        // Special case for a double-quoted string
                        off = 2;
                        l = 2;
                    } else {
                        off = 1;
                        l = utfc_ptr2len(last_dash.add(1)) as usize;
                    }
                    if modifiers != 0 && *last_dash.add(l + 1) == b'>' as c_char {
                        key = utf_ptr2char(last_dash.add(off));
                    } else {
                        key = get_special_key_code(last_dash.add(off));
                        if (flags & FSK_KEEP_X_KEY) == 0 {
                            key = handle_x_keys(key);
                        }
                    }
                }

                // get_special_key_code() may return NUL for invalid special key name.
                if key != 0 {
                    // Only use a modifier when there is no special key code that
                    // includes the modifier.
                    key = simplify_key(key, &mut modifiers);

                    if (flags & FSK_KEYCODE) == 0 {
                        // don't want keycode, use single byte code
                        if key == K_BS {
                            key = BS;
                        } else if key == K_DEL || key == K_KDEL {
                            key = DEL;
                        }
                    }

                    // Normal Key with modifier:
                    // Try to make a single byte code (except for Alt/Meta modifiers).
                    if !is_special(key) {
                        key = extract_modifiers(
                            key,
                            &mut modifiers,
                            (flags & FSK_SIMPLIFY) != 0,
                            did_simplify,
                        );
                    }

                    *modp = modifiers;
                    *srcp = end_of_name;
                    return key;
                }
            }
        }
        0
    }
}

/// Try to find key `c` in the special key table (`find_special_key_in_table`).
pub fn find_special_key_in_table(c: c_int) -> c_int {
    for (i, entry) in KEY_NAMES_TABLE.iter().enumerate() {
        if c == entry.key && !entry.is_alt {
            return i as c_int;
        }
    }
    -1
}

/// Find the special key with the given name (`get_special_key_code`).
pub unsafe fn get_special_key_code(name: *const c_char) -> c_int {
    unsafe {
        if *name == b't' as c_char
            && *name.add(1) == b'_' as c_char
            && *name.add(2) != 0
            && *name.add(3) != 0
        {
            return termcap2key(
                c_int::from(*name.add(2) as u8),
                c_int::from(*name.add(3) as u8),
            );
        }

        let mut len = 0usize;
        while ascii_isident(*name.add(len)) {
            len += 1;
        }

        let slice = std::slice::from_raw_parts(name.cast::<u8>(), len);
        let idx = get_special_key_code_hash(slice);
        if idx >= 0 {
            KEY_NAMES_TABLE[idx as usize].key
        } else {
            0
        }
    }
}

/// Look up the given mouse code to return the relevant information (`get_mouse_button`).
pub unsafe fn get_mouse_button(code: c_int, is_click: *mut bool, is_drag: *mut bool) -> c_int {
    for entry in MOUSE_TABLE {
        if code == entry.pseudo_code {
            unsafe {
                *is_click = entry.is_click;
                *is_drag = entry.is_drag;
            }
            return entry.button;
        }
    }
    0
}

/// Encode `<key>` notation into Nvim's internal key representation (`replace_termcodes`).
pub unsafe fn replace_termcodes(
    from: *const c_char,
    from_len: usize,
    bufp: *mut *mut c_char,
    sid_arg: scid_T,
    flags: c_int,
    did_simplify: *mut bool,
    cpo_val: *const c_char,
) -> *mut c_char {
    unsafe {
        let mut dlen: usize = 0;
        let end_exclusive = from.add(from_len);

        // backslash is a special character
        let do_backslash = vim_strchr(cpo_val, K_CPO_BSLASH).is_null();
        let do_special = (flags & REPTERM_NO_SPECIAL) == 0;

        let allocated = (*bufp).is_null();

        // Allocate space for the translation. Worst case a single character is
        // replaced by 6 bytes (shifted special key), plus a NUL at the end.
        let buf_len: usize = if allocated { from_len * 6 + 1 } else { 128 };
        let result: *mut c_char = if allocated {
            xmalloc(buf_len).cast::<c_char>()
        } else {
            *bufp
        };

        let mut src = from;

        // Copy each byte from *from to result[dlen]
        while (src as usize) < (end_exclusive as usize) {
            if !allocated && dlen + 64 > buf_len {
                return std::ptr::null_mut();
            }
            let rem = (end_exclusive as usize) - (src as usize);

            // Check for special <> keycodes, like "<C-S-LeftMouse>"
            if do_special
                && ((flags & REPTERM_DO_LT) != 0
                    || (rem >= 4 && std::slice::from_raw_parts(src.cast::<u8>(), 4) != b"<lt>"))
            {
                // Change <SID>Func to K_SNR <script-nr> _Func. This name is used
                // for script-local user functions.
                if rem >= 5 && vim_strnicmp_asc(src, c"<SID>".as_ptr(), 5) == 0 {
                    if sid_arg < 0 || (sid_arg == 0 && current_sctx.sc_sid <= 0) {
                        emsg(e_usingsid.as_ptr());
                    } else {
                        let sid: scid_T = if sid_arg != 0 {
                            sid_arg
                        } else {
                            current_sctx.sc_sid
                        };
                        src = src.add(5);
                        *result.add(dlen) = K_SPECIAL as c_char;
                        dlen += 1;
                        *result.add(dlen) = KS_EXTRA as c_char;
                        dlen += 1;
                        *result.add(dlen) = KE_SNR as c_char;
                        dlen += 1;
                        vim_snprintf!(
                            result.add(dlen),
                            buf_len - dlen,
                            c"%d".as_ptr(),
                            sid as c_int,
                        );
                        dlen += CStr::from_ptr(result.add(dlen)).to_bytes().len();
                        *result.add(dlen) = b'_' as c_char;
                        dlen += 1;
                        continue;
                    }
                }

                let fsk_flags = FSK_KEYCODE
                    | if (flags & REPTERM_NO_SIMPLIFY) != 0 {
                        0
                    } else {
                        FSK_SIMPLIFY
                    };
                let slen = trans_special(
                    &mut src,
                    rem,
                    result.add(dlen),
                    fsk_flags,
                    true,
                    did_simplify,
                ) as usize;
                if slen != 0 {
                    dlen += slen;
                    continue;
                }
            }

            if do_special {
                // Replace <Leader> by the value of "mapleader".
                // Replace <LocalLeader> by the value of "maplocalleader".
                // If "mapleader" or "maplocalleader" isn't set use a backslash.
                let (len, p): (usize, *mut c_char) = if rem >= 8
                    && vim_strnicmp_asc(src, c"<Leader>".as_ptr(), 8) == 0
                {
                    (8, get_var_value(c"g:mapleader".as_ptr()))
                } else if rem >= 13 && vim_strnicmp_asc(src, c"<LocalLeader>".as_ptr(), 13) == 0 {
                    (13, get_var_value(c"g:maplocalleader".as_ptr()))
                } else {
                    (0, std::ptr::null_mut())
                };

                if len != 0 {
                    let mut s: *const c_char =
                        if p.is_null() || *p == 0 || CStr::from_ptr(p).to_bytes().len() > 8 * 6 {
                            c"\\".as_ptr()
                        } else {
                            p
                        };
                    while *s != 0 {
                        *result.add(dlen) = *s;
                        dlen += 1;
                        s = s.add(1);
                    }
                    src = src.add(len);
                    continue;
                }
            }

            // Remove CTRL-V and ignore the next character.
            // For "from" side the CTRL-V at the end is included, for the "to"
            // part it is removed.
            // If 'cpoptions' does not contain 'B', also accept a backslash.
            let key = *src;
            if key as u8 == Ctrl_V || (do_backslash && key == b'\\' as c_char) {
                src = src.add(1); // skip CTRL-V or backslash
                if (src as usize) >= (end_exclusive as usize) {
                    if (flags & REPTERM_FROM_PART) != 0 {
                        *result.add(dlen) = key;
                        dlen += 1;
                    }
                    break;
                }
            }

            // skip multibyte char correctly
            let rem_after = (end_exclusive as usize) - (src as usize);
            let mut i = utfc_ptr2len_len(src, rem_after as c_int) as isize;
            while i > 0 {
                // If the character is K_SPECIAL, replace it with K_SPECIAL
                // KS_SPECIAL KE_FILLER.
                if *src as u8 == K_SPECIAL {
                    *result.add(dlen) = K_SPECIAL as c_char;
                    dlen += 1;
                    *result.add(dlen) = KS_SPECIAL as c_char;
                    dlen += 1;
                    *result.add(dlen) = KE_FILLER as c_char;
                    dlen += 1;
                } else {
                    *result.add(dlen) = *src;
                    dlen += 1;
                }
                src = src.add(1);
                i -= 1;
            }
        }
        *result.add(dlen) = 0;

        if allocated {
            *bufp = crate::memory::xmemdupz(result.cast(), dlen).cast::<c_char>();
            xfree(result.cast());
        }

        *bufp
    }
}

/// Add character `c` to buffer `s`, escaping `K_SPECIAL` (`add_char2buf`).
pub unsafe fn add_char2buf(c: c_int, s: *mut c_char) -> *mut c_char {
    unsafe {
        let mut s = s;
        let mut temp = [0 as c_char; MB_MAXBYTES + 1];
        let len = utf_char2bytes(c, temp.as_mut_ptr()) as usize;
        for &byte in &temp[..len] {
            let b = byte as u8;
            if b == K_SPECIAL {
                *s = K_SPECIAL as c_char;
                s = s.add(1);
                *s = KS_SPECIAL as c_char;
                s = s.add(1);
                *s = KE_FILLER as c_char;
                s = s.add(1);
            } else {
                *s = byte;
                s = s.add(1);
            }
        }
        s
    }
}

/// Copy `p` to allocated memory, escaping `K_SPECIAL` (`vim_strsave_escape_ks`).
pub unsafe fn vim_strsave_escape_ks(p: *mut c_char) -> *mut c_char {
    unsafe {
        let res = xmalloc(CStr::from_ptr(p).to_bytes().len() * 4 + 1).cast::<c_char>();
        let mut d = res;
        let mut s = p;
        while *s != 0 {
            if *s as u8 == K_SPECIAL && *s.add(1) != 0 && *s.add(2) != 0 {
                // Copy special key unmodified.
                *d = *s;
                d = d.add(1);
                s = s.add(1);
                *d = *s;
                d = d.add(1);
                s = s.add(1);
                *d = *s;
                d = d.add(1);
                s = s.add(1);
            } else {
                // Add character, possibly multi-byte to destination, escaping
                // K_SPECIAL. Be careful, it can be an illegal byte!
                d = add_char2buf(utf_ptr2char(s), d);
                s = s.add(utf_ptr2len(s) as usize);
            }
        }
        *d = 0;
        res
    }
}

/// Remove escaping from `K_SPECIAL` characters in-place (`vim_unescape_ks`).
pub unsafe fn vim_unescape_ks(p: *mut c_char) -> usize {
    unsafe {
        let mut s = p.cast::<u8>();
        let mut d = p.cast::<u8>();

        while *s != NUL {
            if *s == K_SPECIAL && *s.add(1) == KS_SPECIAL && *s.add(2) == KE_FILLER {
                *d = K_SPECIAL;
                d = d.add(1);
                s = s.add(3);
            } else {
                *d = *s;
                d = d.add(1);
                s = s.add(1);
            }
        }
        *d = NUL;
        (d as usize) - (p as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_to_mod_mask_recognizes_modifiers_case_insensitively() {
        assert_eq!(name_to_mod_mask(b'S' as c_int), MOD_MASK_SHIFT as c_int);
        assert_eq!(name_to_mod_mask(b's' as c_int), MOD_MASK_SHIFT as c_int);
        assert_eq!(name_to_mod_mask(b'C' as c_int), MOD_MASK_CTRL as c_int);
        assert_eq!(name_to_mod_mask(b'M' as c_int), MOD_MASK_ALT as c_int);
        assert_eq!(name_to_mod_mask(b'A' as c_int), MOD_MASK_ALT as c_int);
        assert_eq!(name_to_mod_mask(b'D' as c_int), MOD_MASK_CMD as c_int);
        assert_eq!(name_to_mod_mask(b'Z' as c_int), 0);
    }

    #[test]
    fn simplify_and_unsimplify_key_roundtrip() {
        let mut mods = MOD_MASK_SHIFT as c_int;
        let simplified = unsafe { simplify_key(K_F1, &mut mods) };
        assert_eq!(simplified, K_S_F1);
        assert_eq!(mods, 0);

        let unsimplified = unsafe { unsimplify_key(simplified, &mut mods) };
        assert_eq!(unsimplified, K_F1);
        assert_eq!(mods, MOD_MASK_SHIFT as c_int);
    }

    #[test]
    fn key_names_table_hash_resolves_all_entries() {
        for entry in KEY_NAMES_TABLE {
            let code = unsafe { get_special_key_code(entry.name.as_ptr()) };
            assert_ne!(code, 0);
        }
        assert_eq!(unsafe { get_special_key_code(c"CR".as_ptr()) }, CAR);
        assert_eq!(unsafe { get_special_key_code(c"Esc".as_ptr()) }, ESC);
        assert_eq!(unsafe { get_special_key_code(c"NotAKey".as_ptr()) }, 0);
    }

    #[test]
    fn vim_unescape_ks_unescapes_in_place() {
        let mut buf = [K_SPECIAL, KS_SPECIAL, KE_FILLER, b'a', 0];
        let len = unsafe { vim_unescape_ks(buf.as_mut_ptr().cast()) };
        assert_eq!(len, 2);
        assert_eq!(buf[0], K_SPECIAL);
        assert_eq!(buf[1], b'a');
        assert_eq!(buf[2], 0);
    }

    #[test]
    fn find_special_key_handles_modifiers_case_and_quotes() {
        unsafe {
            let mut modp: c_int = 0;
            let mut p = c"abc".as_ptr();
            assert_eq!(
                find_special_key(&mut p, 3, &mut modp, 0, std::ptr::null_mut()),
                0
            );

            p = c"<C-M-S-A>".as_ptr();
            assert_ne!(
                find_special_key(&mut p, 9, &mut modp, 0, std::ptr::null_mut()),
                0
            );
            assert_ne!(modp, 0);

            p = c"<C-A>".as_ptr();
            let k_upper = find_special_key(&mut p, 5, &mut modp, 0, std::ptr::null_mut());
            let m_upper = modp;
            for variant in [c"<C-a>", c"<c-A>", c"<c-a>"] {
                p = variant.as_ptr();
                assert_eq!(
                    find_special_key(&mut p, 5, &mut modp, 0, std::ptr::null_mut()),
                    k_upper
                );
                assert_eq!(modp, m_upper);
            }

            p = c"<C-\">".as_ptr();
            assert_eq!(
                find_special_key(&mut p, 5, &mut modp, 0, std::ptr::null_mut()),
                b'"' as c_int
            );
            p = c"<C-\">".as_ptr();
            assert_eq!(
                find_special_key(&mut p, 5, &mut modp, FSK_IN_STRING, std::ptr::null_mut()),
                0
            );
            p = c"<C-\\\">".as_ptr();
            assert_eq!(
                find_special_key(&mut p, 6, &mut modp, 0, std::ptr::null_mut()),
                0
            );
            p = c"<C-\\\">".as_ptr();
            assert_eq!(
                find_special_key(&mut p, 6, &mut modp, FSK_IN_STRING, std::ptr::null_mut()),
                b'"' as c_int
            );
        }
    }
}
