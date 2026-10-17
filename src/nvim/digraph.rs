//! Digraph and keymap functions (`src/nvim/digraph.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::command::exarg_T;
use crate::api::tabpage::win_T;
use crate::charset::{char2cells, getdigits_int, skiptowhite, skipwhite};
use crate::clipboard::{VarType, list_T, listitem_T, typval_T};
use crate::eval::deprecated::buf_T;
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::indent::EvalFuncData;
use crate::keycodes::K_BS;
use crate::memory::{xfree, xmalloc, xmemdupz, xstrdup};

pub type result_T = c_int;
pub type varnumber_T = i64;
pub type LineGetter = Option<unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct digr_T {
    pub char1: u8,
    pub char2: u8,
    pub result: result_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct kmap_T {
    pub from: *mut c_char,
    pub to: *mut c_char,
}

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const ESC: c_int = 0x1b;
const Ctrl_H: c_int = 8;
const MB_MAXCHAR: usize = 6;
const NUMBUFLEN: usize = 65;

const HLF_8: c_int = 1;
const HLF_CM: c_int = 10;

const kBoolVarFalse: c_int = 0;
const kBoolVarTrue: c_int = 1;

const KEYMAP_INIT: i16 = 1;
const KEYMAP_LOADED: i16 = 2;
const B_IMODE_LMAP: c_int = 1;

const MAPTYPE_MAP: c_int = 0;
const MAPTYPE_UNMAP: c_int = 1;
const MODE_LANGMAP: c_int = 0x20;

const KMAP_MAXLEN: usize = 20;
const KMAP_LLEN: usize = 200;

const DG_START_LATIN: result_T = 0xa1;
const DG_START_GREEK: result_T = 0x0386;
const DG_START_CYRILLIC: result_T = 0x0401;
const DG_START_HEBREW: result_T = 0x05d0;
const DG_START_ARABIC: result_T = 0x060c;
const DG_START_LATIN_EXTENDED: result_T = 0x1e02;
const DG_START_GREEK_EXTENDED: result_T = 0x1f00;
const DG_START_PUNCTUATION: result_T = 0x2002;
const DG_START_SUB_SUPER: result_T = 0x2070;
const DG_START_CURRENCY: result_T = 0x20a4;
const DG_START_OTHER1: result_T = 0x2103;
const DG_START_ROMAN: result_T = 0x2160;
const DG_START_ARROWS: result_T = 0x2190;
const DG_START_MATH: result_T = 0x2200;
const DG_START_TECHNICAL: result_T = 0x2302;
const DG_START_OTHER2: result_T = 0x2423;
const DG_START_DRAWING: result_T = 0x2500;
const DG_START_BLOCK: result_T = 0x2580;
const DG_START_SHAPES: result_T = 0x25a0;
const DG_START_SYMBOLS: result_T = 0x2605;
const DG_START_DINGBATS: result_T = 0x2713;
const DG_START_CJK_SYMBOLS: result_T = 0x3000;
const DG_START_HIRAGANA: result_T = 0x3041;
const DG_START_KATAKANA: result_T = 0x30a1;
const DG_START_BOPOMOFO: result_T = 0x3105;
const DG_START_OTHER3: result_T = 0x3220;

const E_DIGRAPH_MUST_BE_JUST_TWO_CHARACTERS_STR: &CStr =
    c"E1214: Digraph must be just two characters: %s";
const E_DIGRAPH_ARGUMENT_MUST_BE_ONE_CHARACTER_STR: &CStr =
    c"E1215: Digraph must be one character: %s";
const E_DIGRAPH_SETLIST_ARGUMENT_MUST_BE_LIST_OF_LISTS_WITH_TWO_ITEMS: &CStr =
    c"E1216: digraph_setlist() argument must be a list of lists with two items";

static mut USER_DIGRAPHS: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: size_of::<digr_T>() as c_int,
    ga_growsize: 10,
    ga_data: ptr::null_mut(),
};

static mut BACKSPACED: c_int = 0;
static mut LASTCHAR: c_int = 0;
static mut DIGRAPH_FOR_CHAR_BUF: [c_char; 3] = [0; 3];

const fn dg(c1: u8, c2: u8, res: result_T) -> digr_T {
    digr_T {
        char1: c1,
        char2: c2,
        result: res,
    }
}

static DIGRAPHDEFAULT: &[digr_T] = &[
    dg(b'N', b'U', 0x0a),
    dg(b'S', b'H', 0x01),
    dg(b'S', b'X', 0x02),
    dg(b'E', b'X', 0x03),
    dg(b'E', b'T', 0x04),
    dg(b'E', b'Q', 0x05),
    dg(b'A', b'K', 0x06),
    dg(b'B', b'L', 0x07),
    dg(b'B', b'S', 0x08),
    dg(b'H', b'T', 0x09),
    dg(b'L', b'F', 0x0a),
    dg(b'V', b'T', 0x0b),
    dg(b'F', b'F', 0x0c),
    dg(b'C', b'R', 0x0d),
    dg(b'S', b'O', 0x0e),
    dg(b'S', b'I', 0x0f),
    dg(b'D', b'L', 0x10),
    dg(b'D', b'1', 0x11),
    dg(b'D', b'2', 0x12),
    dg(b'D', b'3', 0x13),
    dg(b'D', b'4', 0x14),
    dg(b'N', b'K', 0x15),
    dg(b'S', b'Y', 0x16),
    dg(b'E', b'B', 0x17),
    dg(b'C', b'N', 0x18),
    dg(b'E', b'M', 0x19),
    dg(b'S', b'B', 0x1a),
    dg(b'E', b'C', 0x1b),
    dg(b'F', b'S', 0x1c),
    dg(b'G', b'S', 0x1d),
    dg(b'R', b'S', 0x1e),
    dg(b'U', b'S', 0x1f),
    dg(b'S', b'P', 0x20),
    dg(b'N', b'b', 0x23),
    dg(b'D', b'O', 0x24),
    dg(b'A', b't', 0x40),
    dg(b'<', b'(', 0x5b),
    dg(b'/', b'/', 0x5c),
    dg(b')', b'>', 0x5d),
    dg(b'\'', b'>', 0x5e),
    dg(b'\'', b'!', 0x60),
    dg(b'(', b'!', 0x7b),
    dg(b'!', b'!', 0x7c),
    dg(b'!', b')', 0x7d),
    dg(b'\'', b'?', 0x7e),
    dg(b'D', b'T', 0x7f),
    dg(b'P', b'A', 0x80),
    dg(b'H', b'O', 0x81),
    dg(b'B', b'H', 0x82),
    dg(b'N', b'H', 0x83),
    dg(b'I', b'N', 0x84),
    dg(b'N', b'L', 0x85),
    dg(b'S', b'A', 0x86),
    dg(b'E', b'S', 0x87),
    dg(b'H', b'S', 0x88),
    dg(b'H', b'J', 0x89),
    dg(b'V', b'S', 0x8a),
    dg(b'P', b'D', 0x8b),
    dg(b'P', b'U', 0x8c),
    dg(b'R', b'I', 0x8d),
    dg(b'S', b'2', 0x8e),
    dg(b'S', b'3', 0x8f),
    dg(b'D', b'C', 0x90),
    dg(b'P', b'1', 0x91),
    dg(b'P', b'2', 0x92),
    dg(b'T', b'S', 0x93),
    dg(b'C', b'C', 0x94),
    dg(b'M', b'W', 0x95),
    dg(b'S', b'G', 0x96),
    dg(b'E', b'G', 0x97),
    dg(b'S', b'S', 0x98),
    dg(b'G', b'C', 0x99),
    dg(b'S', b'C', 0x9a),
    dg(b'C', b'I', 0x9b),
    dg(b'S', b'T', 0x9c),
    dg(b'O', b'C', 0x9d),
    dg(b'P', b'M', 0x9e),
    dg(b'A', b'C', 0x9f),
    dg(b'N', b'S', 0xa0),
    dg(b'!', b'I', 0xa1),
    dg(b'~', b'!', 0xa1),
    dg(b'C', b't', 0xa2),
    dg(b'c', b'|', 0xa2),
    dg(b'P', b'd', 0xa3),
    dg(b'$', b'$', 0xa3),
    dg(b'C', b'u', 0xa4),
    dg(b'o', b'x', 0xa4),
    dg(b'Y', b'e', 0xa5),
    dg(b'Y', b'-', 0xa5),
    dg(b'B', b'B', 0xa6),
    dg(b'|', b'|', 0xa6),
    dg(b'S', b'E', 0xa7),
    dg(b'\'', b':', 0xa8),
    dg(b'C', b'o', 0xa9),
    dg(b'c', b'O', 0xa9),
    dg(b'-', b'a', 0xaa),
    dg(b'<', b'<', 0xab),
    dg(b'N', b'O', 0xac),
    dg(b'-', b',', 0xac),
    dg(b'-', b'-', 0xad),
    dg(b'R', b'g', 0xae),
    dg(b'\'', b'm', 0xaf),
    dg(b'-', b'=', 0xaf),
    dg(b'D', b'G', 0xb0),
    dg(b'~', b'o', 0xb0),
    dg(b'+', b'-', 0xb1),
    dg(b'2', b'S', 0xb2),
    dg(b'2', b'2', 0xb2),
    dg(b'3', b'S', 0xb3),
    dg(b'3', b'3', 0xb3),
    dg(b'\'', b'\'', 0xb4),
    dg(b'M', b'y', 0xb5),
    dg(b'P', b'I', 0xb6),
    dg(b'p', b'p', 0xb6),
    dg(b'.', b'M', 0xb7),
    dg(b'~', b'.', 0xb7),
    dg(b'\'', b',', 0xb8),
    dg(b'1', b'S', 0xb9),
    dg(b'1', b'1', 0xb9),
    dg(b'-', b'o', 0xba),
    dg(b'>', b'>', 0xbb),
    dg(b'1', b'4', 0xbc),
    dg(b'1', b'2', 0xbd),
    dg(b'3', b'4', 0xbe),
    dg(b'?', b'I', 0xbf),
    dg(b'~', b'?', 0xbf),
    dg(b'A', b'!', 0xc0),
    dg(b'A', b'`', 0xc0),
    dg(b'A', b'\'', 0xc1),
    dg(b'A', b'>', 0xc2),
    dg(b'A', b'^', 0xc2),
    dg(b'A', b'?', 0xc3),
    dg(b'A', b'~', 0xc3),
    dg(b'A', b':', 0xc4),
    dg(b'A', b'"', 0xc4),
    dg(b'A', b'A', 0xc5),
    dg(b'A', b'@', 0xc5),
    dg(b'A', b'E', 0xc6),
    dg(b'C', b',', 0xc7),
    dg(b'E', b'!', 0xc8),
    dg(b'E', b'`', 0xc8),
    dg(b'E', b'\'', 0xc9),
    dg(b'E', b'>', 0xca),
    dg(b'E', b'^', 0xca),
    dg(b'E', b':', 0xcb),
    dg(b'E', b'"', 0xcb),
    dg(b'I', b'!', 0xcc),
    dg(b'I', b'`', 0xcc),
    dg(b'I', b'\'', 0xcd),
    dg(b'I', b'>', 0xce),
    dg(b'I', b'^', 0xce),
    dg(b'I', b':', 0xcf),
    dg(b'I', b'"', 0xcf),
    dg(b'D', b'-', 0xd0),
    dg(b'N', b'?', 0xd1),
    dg(b'N', b'~', 0xd1),
    dg(b'O', b'!', 0xd2),
    dg(b'O', b'`', 0xd2),
    dg(b'O', b'\'', 0xd3),
    dg(b'O', b'>', 0xd4),
    dg(b'O', b'^', 0xd4),
    dg(b'O', b'?', 0xd5),
    dg(b'O', b'~', 0xd5),
    dg(b'O', b':', 0xd6),
    dg(b'*', b'X', 0xd7),
    dg(b'/', b'\\', 0xd7),
    dg(b'O', b'/', 0xd8),
    dg(b'U', b'!', 0xd9),
    dg(b'U', b'`', 0xd9),
    dg(b'U', b'\'', 0xda),
    dg(b'U', b'>', 0xdb),
    dg(b'U', b'^', 0xdb),
    dg(b'U', b':', 0xdc),
    dg(b'Y', b'\'', 0xdd),
    dg(b'T', b'H', 0xde),
    dg(b'I', b'p', 0xde),
    dg(b's', b's', 0xdf),
    dg(b'a', b'!', 0xe0),
    dg(b'a', b'`', 0xe0),
    dg(b'a', b'\'', 0xe1),
    dg(b'a', b'>', 0xe2),
    dg(b'a', b'^', 0xe2),
    dg(b'a', b'?', 0xe3),
    dg(b'a', b'~', 0xe3),
    dg(b'a', b':', 0xe4),
    dg(b'a', b'"', 0xe4),
    dg(b'a', b'a', 0xe5),
    dg(b'a', b'@', 0xe5),
    dg(b'a', b'e', 0xe6),
    dg(b'c', b',', 0xe7),
    dg(b'e', b'!', 0xe8),
    dg(b'e', b'`', 0xe8),
    dg(b'e', b'\'', 0xe9),
    dg(b'e', b'>', 0xea),
    dg(b'e', b'^', 0xea),
    dg(b'e', b':', 0xeb),
    dg(b'e', b'"', 0xeb),
    dg(b'i', b'!', 0xec),
    dg(b'i', b'`', 0xec),
    dg(b'i', b'\'', 0xed),
    dg(b'i', b'>', 0xee),
    dg(b'i', b'^', 0xee),
    dg(b'i', b':', 0xef),
    dg(b'd', b'-', 0xf0),
    dg(b'n', b'?', 0xf1),
    dg(b'n', b'~', 0xf1),
    dg(b'o', b'!', 0xf2),
    dg(b'o', b'`', 0xf2),
    dg(b'o', b'\'', 0xf3),
    dg(b'o', b'>', 0xf4),
    dg(b'o', b'^', 0xf4),
    dg(b'o', b'?', 0xf5),
    dg(b'o', b'~', 0xf5),
    dg(b'o', b':', 0xf6),
    dg(b'-', b':', 0xf7),
    dg(b'o', b'/', 0xf8),
    dg(b'u', b'!', 0xf9),
    dg(b'u', b'`', 0xf9),
    dg(b'u', b'\'', 0xfa),
    dg(b'u', b'>', 0xfb),
    dg(b'u', b'^', 0xfb),
    dg(b'u', b':', 0xfc),
    dg(b'y', b'\'', 0xfd),
    dg(b't', b'h', 0xfe),
    dg(b'y', b':', 0xff),
    dg(b'y', b'"', 0xff),
    dg(b'A', b'-', 0x0100),
    dg(b'a', b'-', 0x0101),
    dg(b'A', b'(', 0x0102),
    dg(b'a', b'(', 0x0103),
    dg(b'A', b';', 0x0104),
    dg(b'a', b';', 0x0105),
    dg(b'C', b'\'', 0x0106),
    dg(b'c', b'\'', 0x0107),
    dg(b'C', b'>', 0x0108),
    dg(b'c', b'>', 0x0109),
    dg(b'C', b'.', 0x010a),
    dg(b'c', b'.', 0x010b),
    dg(b'C', b'<', 0x010c),
    dg(b'c', b'<', 0x010d),
    dg(b'D', b'<', 0x010e),
    dg(b'd', b'<', 0x010f),
    dg(b'D', b'/', 0x0110),
    dg(b'd', b'/', 0x0111),
    dg(b'E', b'-', 0x0112),
    dg(b'e', b'-', 0x0113),
    dg(b'E', b'(', 0x0114),
    dg(b'e', b'(', 0x0115),
    dg(b'E', b'.', 0x0116),
    dg(b'e', b'.', 0x0117),
    dg(b'E', b';', 0x0118),
    dg(b'e', b';', 0x0119),
    dg(b'E', b'<', 0x011a),
    dg(b'e', b'<', 0x011b),
    dg(b'G', b'>', 0x011c),
    dg(b'g', b'>', 0x011d),
    dg(b'G', b'(', 0x011e),
    dg(b'g', b'(', 0x011f),
    dg(b'G', b'.', 0x0120),
    dg(b'g', b'.', 0x0121),
    dg(b'G', b',', 0x0122),
    dg(b'g', b',', 0x0123),
    dg(b'H', b'>', 0x0124),
    dg(b'h', b'>', 0x0125),
    dg(b'H', b'/', 0x0126),
    dg(b'h', b'/', 0x0127),
    dg(b'I', b'?', 0x0128),
    dg(b'i', b'?', 0x0129),
    dg(b'I', b'-', 0x012a),
    dg(b'i', b'-', 0x012b),
    dg(b'I', b'(', 0x012c),
    dg(b'i', b'(', 0x012d),
    dg(b'I', b';', 0x012e),
    dg(b'i', b';', 0x012f),
    dg(b'I', b'.', 0x0130),
    dg(b'i', b'.', 0x0131),
    dg(b'I', b'J', 0x0132),
    dg(b'i', b'j', 0x0133),
    dg(b'J', b'>', 0x0134),
    dg(b'j', b'>', 0x0135),
    dg(b'K', b',', 0x0136),
    dg(b'k', b',', 0x0137),
    dg(b'k', b'k', 0x0138),
    dg(b'L', b'\'', 0x0139),
    dg(b'l', b'\'', 0x013a),
    dg(b'L', b',', 0x013b),
    dg(b'l', b',', 0x013c),
    dg(b'L', b'<', 0x013d),
    dg(b'l', b'<', 0x013e),
    dg(b'L', b'.', 0x013f),
    dg(b'l', b'.', 0x0140),
    dg(b'L', b'/', 0x0141),
    dg(b'l', b'/', 0x0142),
    dg(b'N', b'\'', 0x0143),
    dg(b'n', b'\'', 0x0144),
    dg(b'N', b',', 0x0145),
    dg(b'n', b',', 0x0146),
    dg(b'N', b'<', 0x0147),
    dg(b'n', b'<', 0x0148),
    dg(b'\'', b'n', 0x0149),
    dg(b'N', b'G', 0x014a),
    dg(b'n', b'g', 0x014b),
    dg(b'O', b'-', 0x014c),
    dg(b'o', b'-', 0x014d),
    dg(b'O', b'(', 0x014e),
    dg(b'o', b'(', 0x014f),
    dg(b'O', b'"', 0x0150),
    dg(b'o', b'"', 0x0151),
    dg(b'O', b'E', 0x0152),
    dg(b'o', b'e', 0x0153),
    dg(b'R', b'\'', 0x0154),
    dg(b'r', b'\'', 0x0155),
    dg(b'R', b',', 0x0156),
    dg(b'r', b',', 0x0157),
    dg(b'R', b'<', 0x0158),
    dg(b'r', b'<', 0x0159),
    dg(b'S', b'\'', 0x015a),
    dg(b's', b'\'', 0x015b),
    dg(b'S', b'>', 0x015c),
    dg(b's', b'>', 0x015d),
    dg(b'S', b',', 0x015e),
    dg(b's', b',', 0x015f),
    dg(b'S', b'<', 0x0160),
    dg(b's', b'<', 0x0161),
    dg(b'T', b',', 0x0162),
    dg(b't', b',', 0x0163),
    dg(b'T', b'<', 0x0164),
    dg(b't', b'<', 0x0165),
    dg(b'T', b'/', 0x0166),
    dg(b't', b'/', 0x0167),
    dg(b'U', b'?', 0x0168),
    dg(b'u', b'?', 0x0169),
    dg(b'U', b'-', 0x016a),
    dg(b'u', b'-', 0x016b),
    dg(b'U', b'(', 0x016c),
    dg(b'u', b'(', 0x016d),
    dg(b'U', b'0', 0x016e),
    dg(b'u', b'0', 0x016f),
    dg(b'U', b'"', 0x0170),
    dg(b'u', b'"', 0x0171),
    dg(b'U', b';', 0x0172),
    dg(b'u', b';', 0x0173),
    dg(b'W', b'>', 0x0174),
    dg(b'w', b'>', 0x0175),
    dg(b'Y', b'>', 0x0176),
    dg(b'y', b'>', 0x0177),
    dg(b'Y', b':', 0x0178),
    dg(b'Z', b'\'', 0x0179),
    dg(b'z', b'\'', 0x017a),
    dg(b'Z', b'.', 0x017b),
    dg(b'z', b'.', 0x017c),
    dg(b'Z', b'<', 0x017d),
    dg(b'z', b'<', 0x017e),
    dg(b'O', b'9', 0x01a0),
    dg(b'o', b'9', 0x01a1),
    dg(b'O', b'I', 0x01a2),
    dg(b'o', b'i', 0x01a3),
    dg(b'y', b'r', 0x01a6),
    dg(b'U', b'9', 0x01af),
    dg(b'u', b'9', 0x01b0),
    dg(b'Z', b'/', 0x01b5),
    dg(b'z', b'/', 0x01b6),
    dg(b'E', b'D', 0x01b7),
    dg(b'A', b'<', 0x01cd),
    dg(b'a', b'<', 0x01ce),
    dg(b'I', b'<', 0x01cf),
    dg(b'i', b'<', 0x01d0),
    dg(b'O', b'<', 0x01d1),
    dg(b'o', b'<', 0x01d2),
    dg(b'U', b'<', 0x01d3),
    dg(b'u', b'<', 0x01d4),
    dg(b'A', b'1', 0x01de),
    dg(b'a', b'1', 0x01df),
    dg(b'A', b'7', 0x01e0),
    dg(b'a', b'7', 0x01e1),
    dg(b'A', b'3', 0x01e2),
    dg(b'a', b'3', 0x01e3),
    dg(b'G', b'/', 0x01e4),
    dg(b'g', b'/', 0x01e5),
    dg(b'G', b'<', 0x01e6),
    dg(b'g', b'<', 0x01e7),
    dg(b'K', b'<', 0x01e8),
    dg(b'k', b'<', 0x01e9),
    dg(b'O', b';', 0x01ea),
    dg(b'o', b';', 0x01eb),
    dg(b'O', b'1', 0x01ec),
    dg(b'o', b'1', 0x01ed),
    dg(b'E', b'Z', 0x01ee),
    dg(b'e', b'z', 0x01ef),
    dg(b'j', b'<', 0x01f0),
    dg(b'G', b'\'', 0x01f4),
    dg(b'g', b'\'', 0x01f5),
    dg(b';', b'S', 0x02bf),
    dg(b'\'', b'<', 0x02c7),
    dg(b'\'', b'(', 0x02d8),
    dg(b'\'', b'.', 0x02d9),
    dg(b'\'', b'0', 0x02da),
    dg(b'\'', b';', 0x02db),
    dg(b'\'', b'"', 0x02dd),
    dg(b'A', b'%', 0x0386),
    dg(b'E', b'%', 0x0388),
    dg(b'Y', b'%', 0x0389),
    dg(b'I', b'%', 0x038a),
    dg(b'O', b'%', 0x038c),
    dg(b'U', b'%', 0x038e),
    dg(b'W', b'%', 0x038f),
    dg(b'i', b'3', 0x0390),
    dg(b'A', b'*', 0x0391),
    dg(b'B', b'*', 0x0392),
    dg(b'G', b'*', 0x0393),
    dg(b'D', b'*', 0x0394),
    dg(b'E', b'*', 0x0395),
    dg(b'Z', b'*', 0x0396),
    dg(b'Y', b'*', 0x0397),
    dg(b'H', b'*', 0x0398),
    dg(b'I', b'*', 0x0399),
    dg(b'K', b'*', 0x039a),
    dg(b'L', b'*', 0x039b),
    dg(b'M', b'*', 0x039c),
    dg(b'N', b'*', 0x039d),
    dg(b'C', b'*', 0x039e),
    dg(b'O', b'*', 0x039f),
    dg(b'P', b'*', 0x03a0),
    dg(b'R', b'*', 0x03a1),
    dg(b'S', b'*', 0x03a3),
    dg(b'T', b'*', 0x03a4),
    dg(b'U', b'*', 0x03a5),
    dg(b'F', b'*', 0x03a6),
    dg(b'X', b'*', 0x03a7),
    dg(b'Q', b'*', 0x03a8),
    dg(b'W', b'*', 0x03a9),
    dg(b'J', b'*', 0x03aa),
    dg(b'V', b'*', 0x03ab),
    dg(b'a', b'%', 0x03ac),
    dg(b'e', b'%', 0x03ad),
    dg(b'y', b'%', 0x03ae),
    dg(b'i', b'%', 0x03af),
    dg(b'u', b'3', 0x03b0),
    dg(b'a', b'*', 0x03b1),
    dg(b'b', b'*', 0x03b2),
    dg(b'g', b'*', 0x03b3),
    dg(b'd', b'*', 0x03b4),
    dg(b'e', b'*', 0x03b5),
    dg(b'z', b'*', 0x03b6),
    dg(b'y', b'*', 0x03b7),
    dg(b'h', b'*', 0x03b8),
    dg(b'i', b'*', 0x03b9),
    dg(b'k', b'*', 0x03ba),
    dg(b'l', b'*', 0x03bb),
    dg(b'm', b'*', 0x03bc),
    dg(b'n', b'*', 0x03bd),
    dg(b'c', b'*', 0x03be),
    dg(b'o', b'*', 0x03bf),
    dg(b'p', b'*', 0x03c0),
    dg(b'r', b'*', 0x03c1),
    dg(b'*', b's', 0x03c2),
    dg(b's', b'*', 0x03c3),
    dg(b't', b'*', 0x03c4),
    dg(b'u', b'*', 0x03c5),
    dg(b'f', b'*', 0x03c6),
    dg(b'x', b'*', 0x03c7),
    dg(b'q', b'*', 0x03c8),
    dg(b'w', b'*', 0x03c9),
    dg(b'j', b'*', 0x03ca),
    dg(b'v', b'*', 0x03cb),
    dg(b'o', b'%', 0x03cc),
    dg(b'u', b'%', 0x03cd),
    dg(b'w', b'%', 0x03ce),
    dg(b'\'', b'G', 0x03d8),
    dg(b',', b'G', 0x03d9),
    dg(b'T', b'3', 0x03da),
    dg(b't', b'3', 0x03db),
    dg(b'M', b'3', 0x03dc),
    dg(b'm', b'3', 0x03dd),
    dg(b'K', b'3', 0x03de),
    dg(b'k', b'3', 0x03df),
    dg(b'P', b'3', 0x03e0),
    dg(b'p', b'3', 0x03e1),
    dg(b'\'', b'%', 0x03f4),
    dg(b'j', b'3', 0x03f5),
    dg(b'I', b'O', 0x0401),
    dg(b'D', b'%', 0x0402),
    dg(b'G', b'%', 0x0403),
    dg(b'I', b'E', 0x0404),
    dg(b'D', b'S', 0x0405),
    dg(b'I', b'I', 0x0406),
    dg(b'Y', b'I', 0x0407),
    dg(b'J', b'%', 0x0408),
    dg(b'L', b'J', 0x0409),
    dg(b'N', b'J', 0x040a),
    dg(b'T', b's', 0x040b),
    dg(b'K', b'J', 0x040c),
    dg(b'V', b'%', 0x040e),
    dg(b'D', b'Z', 0x040f),
    dg(b'A', b'=', 0x0410),
    dg(b'B', b'=', 0x0411),
    dg(b'V', b'=', 0x0412),
    dg(b'G', b'=', 0x0413),
    dg(b'D', b'=', 0x0414),
    dg(b'E', b'=', 0x0415),
    dg(b'Z', b'%', 0x0416),
    dg(b'Z', b'=', 0x0417),
    dg(b'I', b'=', 0x0418),
    dg(b'J', b'=', 0x0419),
    dg(b'K', b'=', 0x041a),
    dg(b'L', b'=', 0x041b),
    dg(b'M', b'=', 0x041c),
    dg(b'N', b'=', 0x041d),
    dg(b'O', b'=', 0x041e),
    dg(b'P', b'=', 0x041f),
    dg(b'R', b'=', 0x0420),
    dg(b'S', b'=', 0x0421),
    dg(b'T', b'=', 0x0422),
    dg(b'U', b'=', 0x0423),
    dg(b'F', b'=', 0x0424),
    dg(b'H', b'=', 0x0425),
    dg(b'C', b'=', 0x0426),
    dg(b'C', b'%', 0x0427),
    dg(b'S', b'%', 0x0428),
    dg(b'S', b'c', 0x0429),
    dg(b'=', b'"', 0x042a),
    dg(b'Y', b'=', 0x042b),
    dg(b'%', b'"', 0x042c),
    dg(b'J', b'E', 0x042d),
    dg(b'J', b'U', 0x042e),
    dg(b'J', b'A', 0x042f),
    dg(b'a', b'=', 0x0430),
    dg(b'b', b'=', 0x0431),
    dg(b'v', b'=', 0x0432),
    dg(b'g', b'=', 0x0433),
    dg(b'd', b'=', 0x0434),
    dg(b'e', b'=', 0x0435),
    dg(b'z', b'%', 0x0436),
    dg(b'z', b'=', 0x0437),
    dg(b'i', b'=', 0x0438),
    dg(b'j', b'=', 0x0439),
    dg(b'k', b'=', 0x043a),
    dg(b'l', b'=', 0x043b),
    dg(b'm', b'=', 0x043c),
    dg(b'n', b'=', 0x043d),
    dg(b'o', b'=', 0x043e),
    dg(b'p', b'=', 0x043f),
    dg(b'r', b'=', 0x0440),
    dg(b's', b'=', 0x0441),
    dg(b't', b'=', 0x0442),
    dg(b'u', b'=', 0x0443),
    dg(b'f', b'=', 0x0444),
    dg(b'h', b'=', 0x0445),
    dg(b'c', b'=', 0x0446),
    dg(b'c', b'%', 0x0447),
    dg(b's', b'%', 0x0448),
    dg(b's', b'c', 0x0449),
    dg(b'=', b'\'', 0x044a),
    dg(b'y', b'=', 0x044b),
    dg(b'%', b'\'', 0x044c),
    dg(b'j', b'e', 0x044d),
    dg(b'j', b'u', 0x044e),
    dg(b'j', b'a', 0x044f),
    dg(b'i', b'o', 0x0451),
    dg(b'd', b'%', 0x0452),
    dg(b'g', b'%', 0x0453),
    dg(b'i', b'e', 0x0454),
    dg(b'd', b's', 0x0455),
    dg(b'i', b'i', 0x0456),
    dg(b'y', b'i', 0x0457),
    dg(b'j', b'%', 0x0458),
    dg(b'l', b'j', 0x0459),
    dg(b'n', b'j', 0x045a),
    dg(b't', b's', 0x045b),
    dg(b'k', b'j', 0x045c),
    dg(b'v', b'%', 0x045e),
    dg(b'd', b'z', 0x045f),
    dg(b'Y', b'3', 0x0462),
    dg(b'y', b'3', 0x0463),
    dg(b'O', b'3', 0x046a),
    dg(b'o', b'3', 0x046b),
    dg(b'F', b'3', 0x0472),
    dg(b'f', b'3', 0x0473),
    dg(b'V', b'3', 0x0474),
    dg(b'v', b'3', 0x0475),
    dg(b'C', b'3', 0x0480),
    dg(b'c', b'3', 0x0481),
    dg(b'G', b'3', 0x0490),
    dg(b'g', b'3', 0x0491),
    dg(b'A', b'+', 0x05d0),
    dg(b'B', b'+', 0x05d1),
    dg(b'G', b'+', 0x05d2),
    dg(b'D', b'+', 0x05d3),
    dg(b'H', b'+', 0x05d4),
    dg(b'W', b'+', 0x05d5),
    dg(b'Z', b'+', 0x05d6),
    dg(b'X', b'+', 0x05d7),
    dg(b'T', b'j', 0x05d8),
    dg(b'J', b'+', 0x05d9),
    dg(b'K', b'%', 0x05da),
    dg(b'K', b'+', 0x05db),
    dg(b'L', b'+', 0x05dc),
    dg(b'M', b'%', 0x05dd),
    dg(b'M', b'+', 0x05de),
    dg(b'N', b'%', 0x05df),
    dg(b'N', b'+', 0x05e0),
    dg(b'S', b'+', 0x05e1),
    dg(b'E', b'+', 0x05e2),
    dg(b'P', b'%', 0x05e3),
    dg(b'P', b'+', 0x05e4),
    dg(b'Z', b'j', 0x05e5),
    dg(b'Z', b'J', 0x05e6),
    dg(b'Q', b'+', 0x05e7),
    dg(b'R', b'+', 0x05e8),
    dg(b'S', b'h', 0x05e9),
    dg(b'T', b'+', 0x05ea),
    dg(b',', b'+', 0x060c),
    dg(b';', b'+', 0x061b),
    dg(b'?', b'+', 0x061f),
    dg(b'H', b'\'', 0x0621),
    dg(b'a', b'M', 0x0622),
    dg(b'a', b'H', 0x0623),
    dg(b'w', b'H', 0x0624),
    dg(b'a', b'h', 0x0625),
    dg(b'y', b'H', 0x0626),
    dg(b'a', b'+', 0x0627),
    dg(b'b', b'+', 0x0628),
    dg(b't', b'm', 0x0629),
    dg(b't', b'+', 0x062a),
    dg(b't', b'k', 0x062b),
    dg(b'g', b'+', 0x062c),
    dg(b'h', b'k', 0x062d),
    dg(b'x', b'+', 0x062e),
    dg(b'd', b'+', 0x062f),
    dg(b'd', b'k', 0x0630),
    dg(b'r', b'+', 0x0631),
    dg(b'z', b'+', 0x0632),
    dg(b's', b'+', 0x0633),
    dg(b's', b'n', 0x0634),
    dg(b'c', b'+', 0x0635),
    dg(b'd', b'd', 0x0636),
    dg(b't', b'j', 0x0637),
    dg(b'z', b'H', 0x0638),
    dg(b'e', b'+', 0x0639),
    dg(b'i', b'+', 0x063a),
    dg(b'+', b'+', 0x0640),
    dg(b'f', b'+', 0x0641),
    dg(b'q', b'+', 0x0642),
    dg(b'k', b'+', 0x0643),
    dg(b'l', b'+', 0x0644),
    dg(b'm', b'+', 0x0645),
    dg(b'n', b'+', 0x0646),
    dg(b'h', b'+', 0x0647),
    dg(b'w', b'+', 0x0648),
    dg(b'j', b'+', 0x0649),
    dg(b'y', b'+', 0x064a),
    dg(b':', b'+', 0x064b),
    dg(b'"', b'+', 0x064c),
    dg(b'=', b'+', 0x064d),
    dg(b'/', b'+', 0x064e),
    dg(b'\'', b'+', 0x064f),
    dg(b'1', b'+', 0x0650),
    dg(b'3', b'+', 0x0651),
    dg(b'0', b'+', 0x0652),
    dg(b'a', b'S', 0x0670),
    dg(b'p', b'+', 0x067e),
    dg(b'v', b'+', 0x06a4),
    dg(b'g', b'f', 0x06af),
    dg(b'0', b'a', 0x06f0),
    dg(b'1', b'a', 0x06f1),
    dg(b'2', b'a', 0x06f2),
    dg(b'3', b'a', 0x06f3),
    dg(b'4', b'a', 0x06f4),
    dg(b'5', b'a', 0x06f5),
    dg(b'6', b'a', 0x06f6),
    dg(b'7', b'a', 0x06f7),
    dg(b'8', b'a', 0x06f8),
    dg(b'9', b'a', 0x06f9),
    dg(b'B', b'.', 0x1e02),
    dg(b'b', b'.', 0x1e03),
    dg(b'B', b'_', 0x1e06),
    dg(b'b', b'_', 0x1e07),
    dg(b'D', b'.', 0x1e0a),
    dg(b'd', b'.', 0x1e0b),
    dg(b'D', b'_', 0x1e0e),
    dg(b'd', b'_', 0x1e0f),
    dg(b'D', b',', 0x1e10),
    dg(b'd', b',', 0x1e11),
    dg(b'F', b'.', 0x1e1e),
    dg(b'f', b'.', 0x1e1f),
    dg(b'G', b'-', 0x1e20),
    dg(b'g', b'-', 0x1e21),
    dg(b'H', b'.', 0x1e22),
    dg(b'h', b'.', 0x1e23),
    dg(b'H', b':', 0x1e26),
    dg(b'h', b':', 0x1e27),
    dg(b'H', b',', 0x1e28),
    dg(b'h', b',', 0x1e29),
    dg(b'K', b'\'', 0x1e30),
    dg(b'k', b'\'', 0x1e31),
    dg(b'K', b'_', 0x1e34),
    dg(b'k', b'_', 0x1e35),
    dg(b'L', b'_', 0x1e3a),
    dg(b'l', b'_', 0x1e3b),
    dg(b'M', b'\'', 0x1e3e),
    dg(b'm', b'\'', 0x1e3f),
    dg(b'M', b'.', 0x1e40),
    dg(b'm', b'.', 0x1e41),
    dg(b'N', b'.', 0x1e44),
    dg(b'n', b'.', 0x1e45),
    dg(b'N', b'_', 0x1e48),
    dg(b'n', b'_', 0x1e49),
    dg(b'P', b'\'', 0x1e54),
    dg(b'p', b'\'', 0x1e55),
    dg(b'P', b'.', 0x1e56),
    dg(b'p', b'.', 0x1e57),
    dg(b'R', b'.', 0x1e58),
    dg(b'r', b'.', 0x1e59),
    dg(b'R', b'_', 0x1e5e),
    dg(b'r', b'_', 0x1e5f),
    dg(b'S', b'.', 0x1e60),
    dg(b's', b'.', 0x1e61),
    dg(b'T', b'.', 0x1e6a),
    dg(b't', b'.', 0x1e6b),
    dg(b'T', b'_', 0x1e6e),
    dg(b't', b'_', 0x1e6f),
    dg(b'V', b'?', 0x1e7c),
    dg(b'v', b'?', 0x1e7d),
    dg(b'W', b'!', 0x1e80),
    dg(b'W', b'`', 0x1e80),
    dg(b'w', b'!', 0x1e81),
    dg(b'w', b'`', 0x1e81),
    dg(b'W', b'\'', 0x1e82),
    dg(b'w', b'\'', 0x1e83),
    dg(b'W', b':', 0x1e84),
    dg(b'w', b':', 0x1e85),
    dg(b'W', b'.', 0x1e86),
    dg(b'w', b'.', 0x1e87),
    dg(b'X', b'.', 0x1e8a),
    dg(b'x', b'.', 0x1e8b),
    dg(b'X', b':', 0x1e8c),
    dg(b'x', b':', 0x1e8d),
    dg(b'Y', b'.', 0x1e8e),
    dg(b'y', b'.', 0x1e8f),
    dg(b'Z', b'>', 0x1e90),
    dg(b'z', b'>', 0x1e91),
    dg(b'Z', b'_', 0x1e94),
    dg(b'z', b'_', 0x1e95),
    dg(b'h', b'_', 0x1e96),
    dg(b't', b':', 0x1e97),
    dg(b'w', b'0', 0x1e98),
    dg(b'y', b'0', 0x1e99),
    dg(b'A', b'2', 0x1ea2),
    dg(b'a', b'2', 0x1ea3),
    dg(b'E', b'2', 0x1eba),
    dg(b'e', b'2', 0x1ebb),
    dg(b'E', b'?', 0x1ebc),
    dg(b'e', b'?', 0x1ebd),
    dg(b'I', b'2', 0x1ec8),
    dg(b'i', b'2', 0x1ec9),
    dg(b'O', b'2', 0x1ece),
    dg(b'o', b'2', 0x1ecf),
    dg(b'U', b'2', 0x1ee6),
    dg(b'u', b'2', 0x1ee7),
    dg(b'Y', b'!', 0x1ef2),
    dg(b'Y', b'`', 0x1ef2),
    dg(b'y', b'!', 0x1ef3),
    dg(b'y', b'`', 0x1ef3),
    dg(b'Y', b'2', 0x1ef6),
    dg(b'y', b'2', 0x1ef7),
    dg(b'Y', b'?', 0x1ef8),
    dg(b'y', b'?', 0x1ef9),
    dg(b';', b'\'', 0x1f00),
    dg(b',', b'\'', 0x1f01),
    dg(b';', b'!', 0x1f02),
    dg(b',', b'!', 0x1f03),
    dg(b'?', b';', 0x1f04),
    dg(b'?', b',', 0x1f05),
    dg(b'!', b':', 0x1f06),
    dg(b'?', b':', 0x1f07),
    dg(b'1', b'N', 0x2002),
    dg(b'1', b'M', 0x2003),
    dg(b'3', b'M', 0x2004),
    dg(b'4', b'M', 0x2005),
    dg(b'6', b'M', 0x2006),
    dg(b'1', b'T', 0x2009),
    dg(b'1', b'H', 0x200a),
    dg(b'-', b'1', 0x2010),
    dg(b'-', b'N', 0x2013),
    dg(b'-', b'M', 0x2014),
    dg(b'-', b'3', 0x2015),
    dg(b'!', b'2', 0x2016),
    dg(b'=', b'2', 0x2017),
    dg(b'\'', b'6', 0x2018),
    dg(b'\'', b'9', 0x2019),
    dg(b'.', b'9', 0x201a),
    dg(b'9', b'\'', 0x201b),
    dg(b'"', b'6', 0x201c),
    dg(b'"', b'9', 0x201d),
    dg(b':', b'9', 0x201e),
    dg(b'9', b'"', 0x201f),
    dg(b'/', b'-', 0x2020),
    dg(b'/', b'=', 0x2021),
    dg(b'o', b'o', 0x2022),
    dg(b'.', b'.', 0x2025),
    dg(b',', b'.', 0x2026),
    dg(b'%', b'0', 0x2030),
    dg(b'1', b'\'', 0x2032),
    dg(b'2', b'\'', 0x2033),
    dg(b'3', b'\'', 0x2034),
    dg(b'4', b'\'', 0x2057),
    dg(b'1', b'"', 0x2035),
    dg(b'2', b'"', 0x2036),
    dg(b'3', b'"', 0x2037),
    dg(b'C', b'a', 0x2038),
    dg(b'<', b'1', 0x2039),
    dg(b'>', b'1', 0x203a),
    dg(b':', b'X', 0x203b),
    dg(b'\'', b'-', 0x203e),
    dg(b'/', b'f', 0x2044),
    dg(b'0', b'S', 0x2070),
    dg(b'4', b'S', 0x2074),
    dg(b'5', b'S', 0x2075),
    dg(b'6', b'S', 0x2076),
    dg(b'7', b'S', 0x2077),
    dg(b'8', b'S', 0x2078),
    dg(b'9', b'S', 0x2079),
    dg(b'+', b'S', 0x207a),
    dg(b'-', b'S', 0x207b),
    dg(b'=', b'S', 0x207c),
    dg(b'(', b'S', 0x207d),
    dg(b')', b'S', 0x207e),
    dg(b'n', b'S', 0x207f),
    dg(b'0', b's', 0x2080),
    dg(b'1', b's', 0x2081),
    dg(b'2', b's', 0x2082),
    dg(b'3', b's', 0x2083),
    dg(b'4', b's', 0x2084),
    dg(b'5', b's', 0x2085),
    dg(b'6', b's', 0x2086),
    dg(b'7', b's', 0x2087),
    dg(b'8', b's', 0x2088),
    dg(b'9', b's', 0x2089),
    dg(b'+', b's', 0x208a),
    dg(b'-', b's', 0x208b),
    dg(b'=', b's', 0x208c),
    dg(b'(', b's', 0x208d),
    dg(b')', b's', 0x208e),
    dg(b'L', b'i', 0x20a4),
    dg(b'P', b't', 0x20a7),
    dg(b'W', b'=', 0x20a9),
    dg(b'=', b'e', 0x20ac),
    dg(b'E', b'u', 0x20ac),
    dg(b'=', b'R', 0x20bd),
    dg(b'=', b'P', 0x20bd),
    dg(b'o', b'C', 0x2103),
    dg(b'c', b'o', 0x2105),
    dg(b'o', b'F', 0x2109),
    dg(b'N', b'0', 0x2116),
    dg(b'P', b'O', 0x2117),
    dg(b'R', b'x', 0x211e),
    dg(b'S', b'M', 0x2120),
    dg(b'T', b'M', 0x2122),
    dg(b'O', b'm', 0x2126),
    dg(b'A', b'O', 0x212b),
    dg(b'1', b'3', 0x2153),
    dg(b'2', b'3', 0x2154),
    dg(b'1', b'5', 0x2155),
    dg(b'2', b'5', 0x2156),
    dg(b'3', b'5', 0x2157),
    dg(b'4', b'5', 0x2158),
    dg(b'1', b'6', 0x2159),
    dg(b'5', b'6', 0x215a),
    dg(b'1', b'8', 0x215b),
    dg(b'3', b'8', 0x215c),
    dg(b'5', b'8', 0x215d),
    dg(b'7', b'8', 0x215e),
    dg(b'1', b'R', 0x2160),
    dg(b'2', b'R', 0x2161),
    dg(b'3', b'R', 0x2162),
    dg(b'4', b'R', 0x2163),
    dg(b'5', b'R', 0x2164),
    dg(b'6', b'R', 0x2165),
    dg(b'7', b'R', 0x2166),
    dg(b'8', b'R', 0x2167),
    dg(b'9', b'R', 0x2168),
    dg(b'a', b'R', 0x2169),
    dg(b'b', b'R', 0x216a),
    dg(b'c', b'R', 0x216b),
    dg(b'1', b'r', 0x2170),
    dg(b'2', b'r', 0x2171),
    dg(b'3', b'r', 0x2172),
    dg(b'4', b'r', 0x2173),
    dg(b'5', b'r', 0x2174),
    dg(b'6', b'r', 0x2175),
    dg(b'7', b'r', 0x2176),
    dg(b'8', b'r', 0x2177),
    dg(b'9', b'r', 0x2178),
    dg(b'a', b'r', 0x2179),
    dg(b'b', b'r', 0x217a),
    dg(b'c', b'r', 0x217b),
    dg(b'<', b'-', 0x2190),
    dg(b'-', b'!', 0x2191),
    dg(b'-', b'>', 0x2192),
    dg(b'-', b'v', 0x2193),
    dg(b'<', b'>', 0x2194),
    dg(b'U', b'D', 0x2195),
    dg(b'<', b'=', 0x21d0),
    dg(b'=', b'>', 0x21d2),
    dg(b'=', b'=', 0x21d4),
    dg(b'F', b'A', 0x2200),
    dg(b'd', b'P', 0x2202),
    dg(b'T', b'E', 0x2203),
    dg(b'/', b'0', 0x2205),
    dg(b'D', b'E', 0x2206),
    dg(b'N', b'B', 0x2207),
    dg(b'(', b'-', 0x2208),
    dg(b'-', b')', 0x220b),
    dg(b'*', b'P', 0x220f),
    dg(b'+', b'Z', 0x2211),
    dg(b'-', b'2', 0x2212),
    dg(b'-', b'+', 0x2213),
    dg(b'*', b'-', 0x2217),
    dg(b'O', b'b', 0x2218),
    dg(b'S', b'b', 0x2219),
    dg(b'R', b'T', 0x221a),
    dg(b'0', b'(', 0x221d),
    dg(b'0', b'0', 0x221e),
    dg(b'-', b'L', 0x221f),
    dg(b'-', b'V', 0x2220),
    dg(b'P', b'P', 0x2225),
    dg(b'A', b'N', 0x2227),
    dg(b'O', b'R', 0x2228),
    dg(b'(', b'U', 0x2229),
    dg(b')', b'U', 0x222a),
    dg(b'I', b'n', 0x222b),
    dg(b'D', b'I', 0x222c),
    dg(b'I', b'o', 0x222e),
    dg(b'.', b':', 0x2234),
    dg(b':', b'.', 0x2235),
    dg(b':', b'R', 0x2236),
    dg(b':', b':', 0x2237),
    dg(b'?', b'1', 0x223c),
    dg(b'C', b'G', 0x223e),
    dg(b'?', b'-', 0x2243),
    dg(b'?', b'=', 0x2245),
    dg(b'?', b'2', 0x2248),
    dg(b'=', b'?', 0x224c),
    dg(b'.', b'=', 0x2250),
    dg(b'H', b'I', 0x2253),
    dg(b'!', b'=', 0x2260),
    dg(b'=', b'3', 0x2261),
    dg(b'=', b'<', 0x2264),
    dg(b'>', b'=', 0x2265),
    dg(b'<', b'*', 0x226a),
    dg(b'*', b'>', 0x226b),
    dg(b'!', b'<', 0x226e),
    dg(b'!', b'>', 0x226f),
    dg(b'(', b'C', 0x2282),
    dg(b')', b'C', 0x2283),
    dg(b'(', b'_', 0x2286),
    dg(b')', b'_', 0x2287),
    dg(b'0', b'.', 0x2299),
    dg(b'0', b'2', 0x229a),
    dg(b'-', b'T', 0x22a5),
    dg(b'.', b'P', 0x22c5),
    dg(b':', b'3', 0x22ee),
    dg(b'.', b'3', 0x22ef),
    dg(b'E', b'h', 0x2302),
    dg(b'<', b'7', 0x2308),
    dg(b'>', b'7', 0x2309),
    dg(b'7', b'<', 0x230a),
    dg(b'7', b'>', 0x230b),
    dg(b'N', b'I', 0x2310),
    dg(b'(', b'A', 0x2312),
    dg(b'T', b'R', 0x2315),
    dg(b'I', b'u', 0x2320),
    dg(b'I', b'l', 0x2321),
    dg(b'<', b'[', 0x27e8),
    dg(b']', b'>', 0x27e9),
    dg(b'V', b's', 0x2423),
    dg(b'1', b'h', 0x2440),
    dg(b'3', b'h', 0x2441),
    dg(b'2', b'h', 0x2442),
    dg(b'4', b'h', 0x2443),
    dg(b'1', b'j', 0x2446),
    dg(b'2', b'j', 0x2447),
    dg(b'3', b'j', 0x2448),
    dg(b'4', b'j', 0x2449),
    dg(b'1', b'.', 0x2488),
    dg(b'2', b'.', 0x2489),
    dg(b'3', b'.', 0x248a),
    dg(b'4', b'.', 0x248b),
    dg(b'5', b'.', 0x248c),
    dg(b'6', b'.', 0x248d),
    dg(b'7', b'.', 0x248e),
    dg(b'8', b'.', 0x248f),
    dg(b'9', b'.', 0x2490),
    dg(b'h', b'h', 0x2500),
    dg(b'H', b'H', 0x2501),
    dg(b'v', b'v', 0x2502),
    dg(b'V', b'V', 0x2503),
    dg(b'3', b'-', 0x2504),
    dg(b'3', b'_', 0x2505),
    dg(b'3', b'!', 0x2506),
    dg(b'3', b'/', 0x2507),
    dg(b'4', b'-', 0x2508),
    dg(b'4', b'_', 0x2509),
    dg(b'4', b'!', 0x250a),
    dg(b'4', b'/', 0x250b),
    dg(b'd', b'r', 0x250c),
    dg(b'd', b'R', 0x250d),
    dg(b'D', b'r', 0x250e),
    dg(b'D', b'R', 0x250f),
    dg(b'd', b'l', 0x2510),
    dg(b'd', b'L', 0x2511),
    dg(b'D', b'l', 0x2512),
    dg(b'L', b'D', 0x2513),
    dg(b'u', b'r', 0x2514),
    dg(b'u', b'R', 0x2515),
    dg(b'U', b'r', 0x2516),
    dg(b'U', b'R', 0x2517),
    dg(b'u', b'l', 0x2518),
    dg(b'u', b'L', 0x2519),
    dg(b'U', b'l', 0x251a),
    dg(b'U', b'L', 0x251b),
    dg(b'v', b'r', 0x251c),
    dg(b'v', b'R', 0x251d),
    dg(b'V', b'r', 0x2520),
    dg(b'V', b'R', 0x2523),
    dg(b'v', b'l', 0x2524),
    dg(b'v', b'L', 0x2525),
    dg(b'V', b'l', 0x2528),
    dg(b'V', b'L', 0x252b),
    dg(b'd', b'h', 0x252c),
    dg(b'd', b'H', 0x252f),
    dg(b'D', b'h', 0x2530),
    dg(b'D', b'H', 0x2533),
    dg(b'u', b'h', 0x2534),
    dg(b'u', b'H', 0x2537),
    dg(b'U', b'h', 0x2538),
    dg(b'U', b'H', 0x253b),
    dg(b'v', b'h', 0x253c),
    dg(b'v', b'H', 0x253f),
    dg(b'V', b'h', 0x2542),
    dg(b'V', b'H', 0x254b),
    dg(b'F', b'D', 0x2571),
    dg(b'B', b'D', 0x2572),
    dg(b'T', b'B', 0x2580),
    dg(b'L', b'B', 0x2584),
    dg(b'F', b'B', 0x2588),
    dg(b'l', b'B', 0x258c),
    dg(b'R', b'B', 0x2590),
    dg(b'.', b'S', 0x2591),
    dg(b':', b'S', 0x2592),
    dg(b'?', b'S', 0x2593),
    dg(b'f', b'S', 0x25a0),
    dg(b'O', b'S', 0x25a1),
    dg(b'R', b'O', 0x25a2),
    dg(b'R', b'r', 0x25a3),
    dg(b'R', b'F', 0x25a4),
    dg(b'R', b'Y', 0x25a5),
    dg(b'R', b'H', 0x25a6),
    dg(b'R', b'Z', 0x25a7),
    dg(b'R', b'K', 0x25a8),
    dg(b'R', b'X', 0x25a9),
    dg(b's', b'B', 0x25aa),
    dg(b'S', b'R', 0x25ac),
    dg(b'O', b'r', 0x25ad),
    dg(b'U', b'T', 0x25b2),
    dg(b'u', b'T', 0x25b3),
    dg(b'P', b'R', 0x25b6),
    dg(b'T', b'r', 0x25b7),
    dg(b'D', b't', 0x25bc),
    dg(b'd', b'T', 0x25bd),
    dg(b'P', b'L', 0x25c0),
    dg(b'T', b'l', 0x25c1),
    dg(b'D', b'b', 0x25c6),
    dg(b'D', b'w', 0x25c7),
    dg(b'L', b'Z', 0x25ca),
    dg(b'0', b'm', 0x25cb),
    dg(b'0', b'o', 0x25ce),
    dg(b'0', b'M', 0x25cf),
    dg(b'0', b'L', 0x25d0),
    dg(b'0', b'R', 0x25d1),
    dg(b'S', b'n', 0x25d8),
    dg(b'I', b'c', 0x25d9),
    dg(b'F', b'd', 0x25e2),
    dg(b'B', b'd', 0x25e3),
    dg(b'*', b'2', 0x2605),
    dg(b'*', b'1', 0x2606),
    dg(b'<', b'H', 0x261c),
    dg(b'>', b'H', 0x261e),
    dg(b'0', b'u', 0x263a),
    dg(b'0', b'U', 0x263b),
    dg(b'S', b'U', 0x263c),
    dg(b'F', b'm', 0x2640),
    dg(b'M', b'l', 0x2642),
    dg(b'c', b'S', 0x2660),
    dg(b'c', b'H', 0x2661),
    dg(b'c', b'D', 0x2662),
    dg(b'c', b'C', 0x2663),
    dg(b'M', b'd', 0x2669),
    dg(b'M', b'8', 0x266a),
    dg(b'M', b'2', 0x266b),
    dg(b'M', b'b', 0x266d),
    dg(b'M', b'x', 0x266e),
    dg(b'M', b'X', 0x266f),
    dg(b'O', b'K', 0x2713),
    dg(b'X', b'X', 0x2717),
    dg(b'-', b'X', 0x2720),
    dg(b'I', b'S', 0x3000),
    dg(b',', b'_', 0x3001),
    dg(b'.', b'_', 0x3002),
    dg(b'+', b'"', 0x3003),
    dg(b'+', b'_', 0x3004),
    dg(b'*', b'_', 0x3005),
    dg(b';', b'_', 0x3006),
    dg(b'0', b'_', 0x3007),
    dg(b'<', b'/', 0x3008),
    dg(b'/', b'>', 0x3009),
    dg(b'<', b'+', 0x300a),
    dg(b'>', b'+', 0x300b),
    dg(b'<', b'\'', 0x300c),
    dg(b'>', b'\'', 0x300d),
    dg(b'<', b'"', 0x300e),
    dg(b'>', b'"', 0x300f),
    dg(b'(', b'"', 0x3010),
    dg(b')', b'"', 0x3011),
    dg(b'=', b'T', 0x3012),
    dg(b'=', b'_', 0x3013),
    dg(b'(', b'\'', 0x3014),
    dg(b')', b'\'', 0x3015),
    dg(b'(', b'I', 0x3016),
    dg(b')', b'I', 0x3017),
    dg(b'-', b'?', 0x301c),
    dg(b'A', b'5', 0x3041),
    dg(b'a', b'5', 0x3042),
    dg(b'I', b'5', 0x3043),
    dg(b'i', b'5', 0x3044),
    dg(b'U', b'5', 0x3045),
    dg(b'u', b'5', 0x3046),
    dg(b'E', b'5', 0x3047),
    dg(b'e', b'5', 0x3048),
    dg(b'O', b'5', 0x3049),
    dg(b'o', b'5', 0x304a),
    dg(b'k', b'a', 0x304b),
    dg(b'g', b'a', 0x304c),
    dg(b'k', b'i', 0x304d),
    dg(b'g', b'i', 0x304e),
    dg(b'k', b'u', 0x304f),
    dg(b'g', b'u', 0x3050),
    dg(b'k', b'e', 0x3051),
    dg(b'g', b'e', 0x3052),
    dg(b'k', b'o', 0x3053),
    dg(b'g', b'o', 0x3054),
    dg(b's', b'a', 0x3055),
    dg(b'z', b'a', 0x3056),
    dg(b's', b'i', 0x3057),
    dg(b'z', b'i', 0x3058),
    dg(b's', b'u', 0x3059),
    dg(b'z', b'u', 0x305a),
    dg(b's', b'e', 0x305b),
    dg(b'z', b'e', 0x305c),
    dg(b's', b'o', 0x305d),
    dg(b'z', b'o', 0x305e),
    dg(b't', b'a', 0x305f),
    dg(b'd', b'a', 0x3060),
    dg(b't', b'i', 0x3061),
    dg(b'd', b'i', 0x3062),
    dg(b't', b'U', 0x3063),
    dg(b't', b'u', 0x3064),
    dg(b'd', b'u', 0x3065),
    dg(b't', b'e', 0x3066),
    dg(b'd', b'e', 0x3067),
    dg(b't', b'o', 0x3068),
    dg(b'd', b'o', 0x3069),
    dg(b'n', b'a', 0x306a),
    dg(b'n', b'i', 0x306b),
    dg(b'n', b'u', 0x306c),
    dg(b'n', b'e', 0x306d),
    dg(b'n', b'o', 0x306e),
    dg(b'h', b'a', 0x306f),
    dg(b'b', b'a', 0x3070),
    dg(b'p', b'a', 0x3071),
    dg(b'h', b'i', 0x3072),
    dg(b'b', b'i', 0x3073),
    dg(b'p', b'i', 0x3074),
    dg(b'h', b'u', 0x3075),
    dg(b'b', b'u', 0x3076),
    dg(b'p', b'u', 0x3077),
    dg(b'h', b'e', 0x3078),
    dg(b'b', b'e', 0x3079),
    dg(b'p', b'e', 0x307a),
    dg(b'h', b'o', 0x307b),
    dg(b'b', b'o', 0x307c),
    dg(b'p', b'o', 0x307d),
    dg(b'm', b'a', 0x307e),
    dg(b'm', b'i', 0x307f),
    dg(b'm', b'u', 0x3080),
    dg(b'm', b'e', 0x3081),
    dg(b'm', b'o', 0x3082),
    dg(b'y', b'A', 0x3083),
    dg(b'y', b'a', 0x3084),
    dg(b'y', b'U', 0x3085),
    dg(b'y', b'u', 0x3086),
    dg(b'y', b'O', 0x3087),
    dg(b'y', b'o', 0x3088),
    dg(b'r', b'a', 0x3089),
    dg(b'r', b'i', 0x308a),
    dg(b'r', b'u', 0x308b),
    dg(b'r', b'e', 0x308c),
    dg(b'r', b'o', 0x308d),
    dg(b'w', b'A', 0x308e),
    dg(b'w', b'a', 0x308f),
    dg(b'w', b'i', 0x3090),
    dg(b'w', b'e', 0x3091),
    dg(b'w', b'o', 0x3092),
    dg(b'n', b'5', 0x3093),
    dg(b'v', b'u', 0x3094),
    dg(b'"', b'5', 0x309b),
    dg(b'0', b'5', 0x309c),
    dg(b'*', b'5', 0x309d),
    dg(b'+', b'5', 0x309e),
    dg(b'a', b'6', 0x30a1),
    dg(b'A', b'6', 0x30a2),
    dg(b'i', b'6', 0x30a3),
    dg(b'I', b'6', 0x30a4),
    dg(b'u', b'6', 0x30a5),
    dg(b'U', b'6', 0x30a6),
    dg(b'e', b'6', 0x30a7),
    dg(b'E', b'6', 0x30a8),
    dg(b'o', b'6', 0x30a9),
    dg(b'O', b'6', 0x30aa),
    dg(b'K', b'a', 0x30ab),
    dg(b'G', b'a', 0x30ac),
    dg(b'K', b'i', 0x30ad),
    dg(b'G', b'i', 0x30ae),
    dg(b'K', b'u', 0x30af),
    dg(b'G', b'u', 0x30b0),
    dg(b'K', b'e', 0x30b1),
    dg(b'G', b'e', 0x30b2),
    dg(b'K', b'o', 0x30b3),
    dg(b'G', b'o', 0x30b4),
    dg(b'S', b'a', 0x30b5),
    dg(b'Z', b'a', 0x30b6),
    dg(b'S', b'i', 0x30b7),
    dg(b'Z', b'i', 0x30b8),
    dg(b'S', b'u', 0x30b9),
    dg(b'Z', b'u', 0x30ba),
    dg(b'S', b'e', 0x30bb),
    dg(b'Z', b'e', 0x30bc),
    dg(b'S', b'o', 0x30bd),
    dg(b'Z', b'o', 0x30be),
    dg(b'T', b'a', 0x30bf),
    dg(b'D', b'a', 0x30c0),
    dg(b'T', b'i', 0x30c1),
    dg(b'D', b'i', 0x30c2),
    dg(b'T', b'U', 0x30c3),
    dg(b'T', b'u', 0x30c4),
    dg(b'D', b'u', 0x30c5),
    dg(b'T', b'e', 0x30c6),
    dg(b'D', b'e', 0x30c7),
    dg(b'T', b'o', 0x30c8),
    dg(b'D', b'o', 0x30c9),
    dg(b'N', b'a', 0x30ca),
    dg(b'N', b'i', 0x30cb),
    dg(b'N', b'u', 0x30cc),
    dg(b'N', b'e', 0x30cd),
    dg(b'N', b'o', 0x30ce),
    dg(b'H', b'a', 0x30cf),
    dg(b'B', b'a', 0x30d0),
    dg(b'P', b'a', 0x30d1),
    dg(b'H', b'i', 0x30d2),
    dg(b'B', b'i', 0x30d3),
    dg(b'P', b'i', 0x30d4),
    dg(b'H', b'u', 0x30d5),
    dg(b'B', b'u', 0x30d6),
    dg(b'P', b'u', 0x30d7),
    dg(b'H', b'e', 0x30d8),
    dg(b'B', b'e', 0x30d9),
    dg(b'P', b'e', 0x30da),
    dg(b'H', b'o', 0x30db),
    dg(b'B', b'o', 0x30dc),
    dg(b'P', b'o', 0x30dd),
    dg(b'M', b'a', 0x30de),
    dg(b'M', b'i', 0x30df),
    dg(b'M', b'u', 0x30e0),
    dg(b'M', b'e', 0x30e1),
    dg(b'M', b'o', 0x30e2),
    dg(b'Y', b'A', 0x30e3),
    dg(b'Y', b'a', 0x30e4),
    dg(b'Y', b'U', 0x30e5),
    dg(b'Y', b'u', 0x30e6),
    dg(b'Y', b'O', 0x30e7),
    dg(b'Y', b'o', 0x30e8),
    dg(b'R', b'a', 0x30e9),
    dg(b'R', b'i', 0x30ea),
    dg(b'R', b'u', 0x30eb),
    dg(b'R', b'e', 0x30ec),
    dg(b'R', b'o', 0x30ed),
    dg(b'W', b'A', 0x30ee),
    dg(b'W', b'a', 0x30ef),
    dg(b'W', b'i', 0x30f0),
    dg(b'W', b'e', 0x30f1),
    dg(b'W', b'o', 0x30f2),
    dg(b'N', b'6', 0x30f3),
    dg(b'V', b'u', 0x30f4),
    dg(b'K', b'A', 0x30f5),
    dg(b'K', b'E', 0x30f6),
    dg(b'V', b'a', 0x30f7),
    dg(b'V', b'i', 0x30f8),
    dg(b'V', b'e', 0x30f9),
    dg(b'V', b'o', 0x30fa),
    dg(b'.', b'6', 0x30fb),
    dg(b'-', b'6', 0x30fc),
    dg(b'*', b'6', 0x30fd),
    dg(b'+', b'6', 0x30fe),
    dg(b'b', b'4', 0x3105),
    dg(b'p', b'4', 0x3106),
    dg(b'm', b'4', 0x3107),
    dg(b'f', b'4', 0x3108),
    dg(b'd', b'4', 0x3109),
    dg(b't', b'4', 0x310a),
    dg(b'n', b'4', 0x310b),
    dg(b'l', b'4', 0x310c),
    dg(b'g', b'4', 0x310d),
    dg(b'k', b'4', 0x310e),
    dg(b'h', b'4', 0x310f),
    dg(b'j', b'4', 0x3110),
    dg(b'q', b'4', 0x3111),
    dg(b'x', b'4', 0x3112),
    dg(b'z', b'h', 0x3113),
    dg(b'c', b'h', 0x3114),
    dg(b's', b'h', 0x3115),
    dg(b'r', b'4', 0x3116),
    dg(b'z', b'4', 0x3117),
    dg(b'c', b'4', 0x3118),
    dg(b's', b'4', 0x3119),
    dg(b'a', b'4', 0x311a),
    dg(b'o', b'4', 0x311b),
    dg(b'e', b'4', 0x311c),
    dg(b'a', b'i', 0x311e),
    dg(b'e', b'i', 0x311f),
    dg(b'a', b'u', 0x3120),
    dg(b'o', b'u', 0x3121),
    dg(b'a', b'n', 0x3122),
    dg(b'e', b'n', 0x3123),
    dg(b'a', b'N', 0x3124),
    dg(b'e', b'N', 0x3125),
    dg(b'e', b'r', 0x3126),
    dg(b'i', b'4', 0x3127),
    dg(b'u', b'4', 0x3128),
    dg(b'i', b'u', 0x3129),
    dg(b'v', b'4', 0x312a),
    dg(b'n', b'G', 0x312b),
    dg(b'g', b'n', 0x312c),
    dg(b'1', b'c', 0x3220),
    dg(b'2', b'c', 0x3221),
    dg(b'3', b'c', 0x3222),
    dg(b'4', b'c', 0x3223),
    dg(b'5', b'c', 0x3224),
    dg(b'6', b'c', 0x3225),
    dg(b'7', b'c', 0x3226),
    dg(b'8', b'c', 0x3227),
    dg(b'9', b'c', 0x3228),
    dg(b'f', b'f', 0xfb00),
    dg(b'f', b'i', 0xfb01),
    dg(b'f', b'l', 0xfb02),
    dg(b'f', b't', 0xfb05),
    dg(b's', b't', 0xfb06),
    dg(0, 0, 0),
];

struct DgHeaderEntry {
    dg_start: result_T,
    dg_header: Option<&'static CStr>,
}

const HEADER_TABLE: [DgHeaderEntry; 27] = [
    DgHeaderEntry {
        dg_start: DG_START_LATIN,
        dg_header: Some(c"Latin supplement"),
    },
    DgHeaderEntry {
        dg_start: DG_START_GREEK,
        dg_header: Some(c"Greek and Coptic"),
    },
    DgHeaderEntry {
        dg_start: DG_START_CYRILLIC,
        dg_header: Some(c"Cyrillic"),
    },
    DgHeaderEntry {
        dg_start: DG_START_HEBREW,
        dg_header: Some(c"Hebrew"),
    },
    DgHeaderEntry {
        dg_start: DG_START_ARABIC,
        dg_header: Some(c"Arabic"),
    },
    DgHeaderEntry {
        dg_start: DG_START_LATIN_EXTENDED,
        dg_header: Some(c"Latin extended"),
    },
    DgHeaderEntry {
        dg_start: DG_START_GREEK_EXTENDED,
        dg_header: Some(c"Greek extended"),
    },
    DgHeaderEntry {
        dg_start: DG_START_PUNCTUATION,
        dg_header: Some(c"Punctuation"),
    },
    DgHeaderEntry {
        dg_start: DG_START_SUB_SUPER,
        dg_header: Some(c"Super- and subscripts"),
    },
    DgHeaderEntry {
        dg_start: DG_START_CURRENCY,
        dg_header: Some(c"Currency"),
    },
    DgHeaderEntry {
        dg_start: DG_START_OTHER1,
        dg_header: Some(c"Other"),
    },
    DgHeaderEntry {
        dg_start: DG_START_ROMAN,
        dg_header: Some(c"Roman numbers"),
    },
    DgHeaderEntry {
        dg_start: DG_START_ARROWS,
        dg_header: Some(c"Arrows"),
    },
    DgHeaderEntry {
        dg_start: DG_START_MATH,
        dg_header: Some(c"Mathematical operators"),
    },
    DgHeaderEntry {
        dg_start: DG_START_TECHNICAL,
        dg_header: Some(c"Technical"),
    },
    DgHeaderEntry {
        dg_start: DG_START_OTHER2,
        dg_header: Some(c"Other"),
    },
    DgHeaderEntry {
        dg_start: DG_START_DRAWING,
        dg_header: Some(c"Box drawing"),
    },
    DgHeaderEntry {
        dg_start: DG_START_BLOCK,
        dg_header: Some(c"Block elements"),
    },
    DgHeaderEntry {
        dg_start: DG_START_SHAPES,
        dg_header: Some(c"Geometric shapes"),
    },
    DgHeaderEntry {
        dg_start: DG_START_SYMBOLS,
        dg_header: Some(c"Symbols"),
    },
    DgHeaderEntry {
        dg_start: DG_START_DINGBATS,
        dg_header: Some(c"Dingbats"),
    },
    DgHeaderEntry {
        dg_start: DG_START_CJK_SYMBOLS,
        dg_header: Some(c"CJK symbols and punctuation"),
    },
    DgHeaderEntry {
        dg_start: DG_START_HIRAGANA,
        dg_header: Some(c"Hiragana"),
    },
    DgHeaderEntry {
        dg_start: DG_START_KATAKANA,
        dg_header: Some(c"Katakana"),
    },
    DgHeaderEntry {
        dg_start: DG_START_BOPOMOFO,
        dg_header: Some(c"Bopomofo"),
    },
    DgHeaderEntry {
        dg_start: DG_START_OTHER3,
        dg_header: Some(c"Other"),
    },
    DgHeaderEntry {
        dg_start: 0xfffffff,
        dg_header: None,
    },
];

unsafe extern "C" {
    static mut curwin: *mut win_T;
    static mut curbuf: *mut buf_T;
    static mut p_dg: c_int;
    static mut p_enc: *mut c_char;
    static mut p_cpo: *mut c_char;
    static mut no_mapping: c_int;
    static mut allow_keys: c_int;
    static mut cmdline_star: c_int;
    static mut msg_col: c_int;
    static mut Columns: c_int;
    static mut got_int: bool;
    static mut emsg_skip: c_int;
    static e_number_exp: [c_char; 1];

    fn plain_vgetc() -> c_int;
    fn putcmdline(c: c_char, shift: bool);
    fn add_to_showcmd(c: c_int) -> bool;
    fn utf_char2bytes(c: c_int, buf: *mut c_char) -> c_int;
    fn utf_iscomposing_first(c: c_int) -> bool;
    fn mb_cptr2char_adv(pp: *mut *const c_char) -> c_int;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(s: *const c_char, ...) -> bool;
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn msg_putchar(c: c_int);
    fn msg_outtrans(str: *const c_char, attr: c_int, recurse: bool) -> c_int;
    fn msg_ext_set_kind(msg_sep: *const c_char);
    fn msg_advance(col: c_int);
    fn fast_breakcheck();
    fn tv_list_alloc(len: isize) -> *mut list_T;
    fn tv_list_alloc_ret(ret_tv: *mut typval_T, len: isize) -> *mut list_T;
    fn tv_list_append_list(l: *mut list_T, itemlist: *mut list_T);
    fn tv_list_append_string(l: *mut list_T, str: *const c_char, len: isize);
    fn tv_get_string_buf_chk(tv: *const typval_T, buf: *mut c_char) -> *const c_char;
    fn tv_get_string_chk(tv: *const typval_T) -> *const c_char;
    fn tv_check_for_opt_bool_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_get_bool(tv: *const typval_T) -> varnumber_T;
    fn do_cmdline_cmd(cmd: *const c_char) -> c_int;
    fn source_runtime(name: *mut c_char, flags: c_int) -> c_int;
    fn getline_equal(fgetline: LineGetter, cookie: *mut c_void, func: LineGetter) -> bool;
    fn getsourceline(c: c_int, cookie: *mut c_void, indent: c_int, do_concat: bool) -> *mut c_char;
    fn do_map(maptype: c_int, arg: *mut c_char, mode: c_int, is_abbrev: bool) -> c_int;
    fn status_redraw_curbuf();
    fn eval_to_string(arg: *mut c_char, join_list: bool, use_simple_function: bool) -> *mut c_char;
    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
}

#[inline]
const fn is_special(c: c_int) -> bool {
    c < 0
}

#[inline]
const fn ascii_isdigit(c: c_char) -> bool {
    (c as u8).is_ascii_digit()
}

#[inline]
unsafe fn tv_list_len(l: *const list_T) -> c_int {
    if l.is_null() {
        0
    } else {
        unsafe { (*l).lv_len }
    }
}

#[inline]
unsafe fn tv_list_first(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*l).lv_first }
    }
}

/// Handle digraphs after typing a character (`do_digraph`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_digraph(mut c: c_int) -> c_int {
    unsafe {
        if c == -1 {
            BACKSPACED = -1;
        } else if p_dg != 0 {
            if BACKSPACED >= 0 {
                c = digraph_get(BACKSPACED, c, false);
            }
            BACKSPACED = -1;

            if (c == K_BS || c == Ctrl_H) && LASTCHAR >= 0 {
                BACKSPACED = LASTCHAR;
            }
        }
        LASTCHAR = c;
        c
    }
}

/// Find a digraph for `val` (`get_digraph_for_char`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_digraph_for_char(val_arg: c_int) -> *mut c_char {
    unsafe {
        let val = val_arg;

        for use_defaults in 0..=1 {
            let mut dp = if use_defaults == 0 {
                USER_DIGRAPHS.ga_data.cast::<digr_T>().cast_const()
            } else {
                DIGRAPHDEFAULT.as_ptr()
            };
            let mut i: c_int = 0;
            while if use_defaults != 0 {
                (*dp).char1 != 0
            } else {
                i < USER_DIGRAPHS.ga_len
            } {
                if (*dp).result == val {
                    DIGRAPH_FOR_CHAR_BUF[0] = (*dp).char1 as c_char;
                    DIGRAPH_FOR_CHAR_BUF[1] = (*dp).char2 as c_char;
                    DIGRAPH_FOR_CHAR_BUF[2] = NUL;
                    return &raw mut DIGRAPH_FOR_CHAR_BUF[0];
                }
                dp = dp.add(1);
                i += 1;
            }
        }
        ptr::null_mut()
    }
}

/// Get a digraph (`get_digraph`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_digraph(cmdline: bool) -> c_int {
    unsafe {
        no_mapping += 1;
        allow_keys += 1;
        let c = plain_vgetc();
        no_mapping -= 1;
        allow_keys -= 1;

        if c == ESC {
            return c_int::from(NUL);
        }

        if is_special(c) {
            return c;
        }

        if cmdline {
            if char2cells(c) == 1 && c < 128 && cmdline_star == 0 {
                putcmdline(c as c_char, true);
            }
        } else {
            add_to_showcmd(c);
        }
        no_mapping += 1;
        allow_keys += 1;
        let cc = plain_vgetc();
        no_mapping -= 1;
        allow_keys -= 1;

        if cc != ESC {
            return digraph_get(c, cc, true);
        }
        c_int::from(NUL)
    }
}

/// Lookup the pair `char1`, `char2` in the digraph tables (`getexactdigraph`).
unsafe fn getexactdigraph(char1: c_int, char2: c_int, meta_char: bool) -> c_int {
    unsafe {
        let mut retval: c_int = 0;

        if is_special(char1) || is_special(char2) {
            return char2;
        }

        let mut dp = USER_DIGRAPHS.ga_data.cast::<digr_T>().cast_const();
        for _ in 0..USER_DIGRAPHS.ga_len {
            if c_int::from((*dp).char1) == char1 && c_int::from((*dp).char2) == char2 {
                retval = (*dp).result;
                break;
            }
            dp = dp.add(1);
        }

        if retval == 0 {
            dp = DIGRAPHDEFAULT.as_ptr();
            while (*dp).char1 != 0 {
                if c_int::from((*dp).char1) == char1 && c_int::from((*dp).char2) == char2 {
                    retval = (*dp).result;
                    break;
                }
                dp = dp.add(1);
            }
        }

        if retval == 0 {
            if char1 == b' ' as c_int && meta_char {
                return char2 | 0x80;
            }
            return char2;
        }
        retval
    }
}

/// Get digraph (`digraph_get`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn digraph_get(char1: c_int, char2: c_int, meta_char: bool) -> c_int {
    unsafe {
        let mut retval = getexactdigraph(char1, char2, meta_char);
        if retval == char2 && char1 != char2 {
            retval = getexactdigraph(char2, char1, meta_char);
            if retval == char1 {
                return char2;
            }
        }
        retval
    }
}

/// Add a digraph to the digraph table (`registerdigraph`).
unsafe fn registerdigraph(char1: c_int, char2: c_int, n: c_int) {
    unsafe {
        let mut dp = USER_DIGRAPHS.ga_data.cast::<digr_T>();
        for _ in 0..USER_DIGRAPHS.ga_len {
            if c_int::from((*dp).char1) == char1 && c_int::from((*dp).char2) == char2 {
                (*dp).result = n;
                return;
            }
            dp = dp.add(1);
        }

        ga_grow(&raw mut USER_DIGRAPHS, 1);
        let new_dp = USER_DIGRAPHS
            .ga_data
            .cast::<digr_T>()
            .add(USER_DIGRAPHS.ga_len as usize);
        USER_DIGRAPHS.ga_len += 1;
        (*new_dp).char1 = char1 as u8;
        (*new_dp).char2 = char2 as u8;
        (*new_dp).result = n;
    }
}

/// Check the characters are valid for a digraph (`check_digraph_chars_valid`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_digraph_chars_valid(char1: c_int, char2: c_int) -> bool {
    unsafe {
        if char2 == 0 {
            let mut msg = [0 as c_char; MB_MAXCHAR + 1];
            let len = utf_char2bytes(char1, msg.as_mut_ptr()) as usize;
            msg[len] = NUL;
            semsg(
                gettext(E_DIGRAPH_MUST_BE_JUST_TWO_CHARACTERS_STR.as_ptr()),
                msg.as_ptr(),
            );
            return false;
        }
        if char1 == ESC || char2 == ESC {
            emsg(gettext(c"E104: Escape not allowed in digraph".as_ptr()));
            return false;
        }
        true
    }
}

/// Add the digraphs in the argument to the digraph table (`putdigraph`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn putdigraph(mut str: *mut c_char) {
    unsafe {
        while *str != NUL {
            str = skipwhite(str);
            if *str == NUL {
                return;
            }
            let char1 = *str as u8;
            str = str.add(1);
            let char2 = *str as u8;
            str = str.add(1);

            if !check_digraph_chars_valid(c_int::from(char1), c_int::from(char2)) {
                return;
            }

            str = skipwhite(str);

            if !ascii_isdigit(*str) {
                emsg(gettext(e_number_exp.as_ptr()));
                return;
            }
            let n = getdigits_int(&raw mut str, true, 0);

            registerdigraph(c_int::from(char1), c_int::from(char2), n);
        }
    }
}

unsafe fn digraph_header(msg: *const c_char) {
    unsafe {
        if msg_col > 0 {
            msg_putchar(b'\n' as c_int);
        }
        msg_outtrans(msg, HLF_CM, false);
        msg_putchar(b'\n' as c_int);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn listdigraphs(use_headers: bool) {
    unsafe {
        let mut previous: result_T = 0;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        msg_putchar(b'\n' as c_int);

        let mut dp = DIGRAPHDEFAULT.as_ptr();
        while (*dp).char1 != 0 && !got_int {
            let tmp = digr_T {
                char1: (*dp).char1,
                char2: (*dp).char2,
                result: getexactdigraph(c_int::from((*dp).char1), c_int::from((*dp).char2), false),
            };

            if tmp.result != 0 && tmp.result != c_int::from(tmp.char2) {
                printdigraph(
                    &raw const tmp,
                    if use_headers {
                        &raw mut previous
                    } else {
                        ptr::null_mut()
                    },
                );
            }
            dp = dp.add(1);
            fast_breakcheck();
        }

        dp = USER_DIGRAPHS.ga_data.cast::<digr_T>().cast_const();
        let mut i: c_int = 0;
        while i < USER_DIGRAPHS.ga_len && !got_int {
            if previous >= 0 && use_headers {
                digraph_header(gettext(c"Custom".as_ptr()));
            }
            previous = -1;
            printdigraph(dp, ptr::null_mut());
            fast_breakcheck();
            dp = dp.add(1);
            i += 1;
        }
    }
}

unsafe fn digraph_getlist_appendpair(dp: *const digr_T, l: *mut list_T) {
    unsafe {
        let l2 = tv_list_alloc(2);
        tv_list_append_list(l, l2);

        let mut buf = [0 as c_char; 30];
        buf[0] = (*dp).char1 as c_char;
        buf[1] = (*dp).char2 as c_char;
        buf[2] = NUL;
        tv_list_append_string(l2, buf.as_ptr(), -1);

        let len = utf_char2bytes((*dp).result, buf.as_mut_ptr()) as usize;
        buf[len] = NUL;
        tv_list_append_string(l2, buf.as_ptr(), -1);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn digraph_getlist_common(list_all: bool, rettv: *mut typval_T) {
    unsafe {
        let cap = (size_of_val(DIGRAPHDEFAULT) as isize) + (USER_DIGRAPHS.ga_len as isize);
        tv_list_alloc_ret(rettv, cap);

        if list_all {
            let mut dp = DIGRAPHDEFAULT.as_ptr();
            while (*dp).char1 != 0 && !got_int {
                let tmp = digr_T {
                    char1: (*dp).char1,
                    char2: (*dp).char2,
                    result: getexactdigraph(
                        c_int::from((*dp).char1),
                        c_int::from((*dp).char2),
                        false,
                    ),
                };
                if tmp.result != 0 && tmp.result != c_int::from(tmp.char2) {
                    digraph_getlist_appendpair(&raw const tmp, (*rettv).vval.v_list);
                }
                dp = dp.add(1);
            }
        }

        let mut dp = USER_DIGRAPHS.ga_data.cast::<digr_T>().cast_const();
        let mut i: c_int = 0;
        while i < USER_DIGRAPHS.ga_len && !got_int {
            digraph_getlist_appendpair(dp, (*rettv).vval.v_list);
            dp = dp.add(1);
            i += 1;
        }
    }
}

unsafe fn printdigraph(dp: *const digr_T, previous: *mut result_T) {
    unsafe {
        let mut buf = [0 as c_char; 30];
        let list_width: c_int = 13;

        if (*dp).result == 0 {
            return;
        }

        if !previous.is_null() {
            for i in 0..HEADER_TABLE.len() {
                let Some(header) = HEADER_TABLE[i].dg_header else {
                    break;
                };
                if *previous < HEADER_TABLE[i].dg_start
                    && (*dp).result >= HEADER_TABLE[i].dg_start
                    && (*dp).result < HEADER_TABLE[i + 1].dg_start
                {
                    digraph_header(gettext(header.as_ptr()));
                    break;
                }
            }
            *previous = (*dp).result;
        }
        if msg_col > Columns - list_width {
            msg_putchar(b'\n' as c_int);
        }

        if msg_col % list_width != 0 {
            msg_advance((msg_col / list_width + 1) * list_width);
        }

        buf[0] = (*dp).char1 as c_char;
        buf[1] = (*dp).char2 as c_char;
        buf[2] = b' ' as c_char;
        buf[3] = NUL;
        msg_outtrans(buf.as_ptr(), 0, false);

        let mut p = buf.as_mut_ptr();
        if utf_iscomposing_first((*dp).result) {
            *p = b' ' as c_char;
            p = p.add(1);
        }
        p = p.add(utf_char2bytes((*dp).result, p) as usize);
        *p = NUL;
        msg_outtrans(buf.as_ptr(), HLF_8, false);

        p = buf.as_mut_ptr();
        if char2cells((*dp).result) == 1 {
            *p = b' ' as c_char;
            p = p.add(1);
        }
        let used = p.offset_from(buf.as_ptr()) as usize;
        vim_snprintf(p, buf.len() - used, c" %3d".as_ptr(), (*dp).result);
        msg_outtrans(buf.as_ptr(), 0, false);
    }
}

/// Get the two digraph characters from a typval (`get_digraph_chars`).
unsafe fn get_digraph_chars(arg: *const typval_T, char1: *mut c_int, char2: *mut c_int) -> c_int {
    unsafe {
        let mut buf_chars = [0 as c_char; NUMBUFLEN];
        let chars = tv_get_string_buf_chk(arg, buf_chars.as_mut_ptr());
        let mut p = chars;

        if !p.is_null() && *p != NUL {
            *char1 = mb_cptr2char_adv(&raw mut p);
            if *p != NUL {
                *char2 = mb_cptr2char_adv(&raw mut p);
                if *p == NUL {
                    if check_digraph_chars_valid(*char1, *char2) {
                        return OK;
                    }
                    return FAIL;
                }
            }
        }
        semsg(
            gettext(E_DIGRAPH_MUST_BE_JUST_TWO_CHARACTERS_STR.as_ptr()),
            chars,
        );
        FAIL
    }
}

unsafe fn digraph_set_common(argchars: *const typval_T, argdigraph: *const typval_T) -> bool {
    unsafe {
        let mut char1: c_int = 0;
        let mut char2: c_int = 0;
        if get_digraph_chars(argchars, &raw mut char1, &raw mut char2) == FAIL {
            return false;
        }

        let mut buf_digraph = [0 as c_char; NUMBUFLEN];
        let digraph = tv_get_string_buf_chk(argdigraph, buf_digraph.as_mut_ptr());
        if digraph.is_null() {
            return false;
        }
        let mut p = digraph;
        let n = mb_cptr2char_adv(&raw mut p);
        if *p != NUL {
            semsg(
                gettext(E_DIGRAPH_ARGUMENT_MUST_BE_ONE_CHARACTER_STR.as_ptr()),
                digraph,
            );
            return false;
        }

        registerdigraph(char1, char2, n);
        true
    }
}

/// `digraph_get()` function (`f_digraph_get`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_digraph_get(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        let digraphs = tv_get_string_chk(argvars);

        if digraphs.is_null() {
            return;
        }
        if CStr::from_ptr(digraphs).to_bytes().len() != 2 {
            semsg(
                gettext(E_DIGRAPH_MUST_BE_JUST_TWO_CHARACTERS_STR.as_ptr()),
                digraphs,
            );
            return;
        }
        let code = digraph_get(c_int::from(*digraphs), c_int::from(*digraphs.add(1)), false);

        let mut buf = [0 as c_char; NUMBUFLEN];
        let len = utf_char2bytes(code, buf.as_mut_ptr()) as usize;
        buf[len] = NUL;
        (*rettv).vval.v_string = xstrdup(buf.as_ptr());
    }
}

/// `digraph_getlist()` function (`f_digraph_getlist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_digraph_getlist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_opt_bool_arg(argvars, 0) == FAIL {
            return;
        }

        let flag_list_all = if (*argvars).v_type == VarType::VAR_UNKNOWN {
            false
        } else {
            tv_get_bool(argvars) != 0
        };

        digraph_getlist_common(flag_list_all, rettv);
    }
}

/// `digraph_set()` function (`f_digraph_set`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_digraph_set(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_BOOL;
        (*rettv).vval.v_bool = kBoolVarFalse;

        if !digraph_set_common(argvars, argvars.add(1)) {
            return;
        }

        (*rettv).vval.v_bool = kBoolVarTrue;
    }
}

/// `digraph_setlist()` function (`f_digraph_setlist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_digraph_setlist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_BOOL;
        (*rettv).vval.v_bool = kBoolVarFalse;

        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(gettext(
                E_DIGRAPH_SETLIST_ARGUMENT_MUST_BE_LIST_OF_LISTS_WITH_TWO_ITEMS.as_ptr(),
            ));
            return;
        }

        let pl = (*argvars).vval.v_list;
        if pl.is_null() {
            (*rettv).vval.v_bool = kBoolVarTrue;
            return;
        }

        let mut pli = tv_list_first(pl);
        while !pli.is_null() {
            let tv = &raw const (*pli).li_tv;
            if (*tv).v_type != VarType::VAR_LIST {
                emsg(gettext(
                    E_DIGRAPH_SETLIST_ARGUMENT_MUST_BE_LIST_OF_LISTS_WITH_TWO_ITEMS.as_ptr(),
                ));
                return;
            }

            let l = (*tv).vval.v_list;
            if l.is_null() || tv_list_len(l) != 2 {
                emsg(gettext(
                    E_DIGRAPH_SETLIST_ARGUMENT_MUST_BE_LIST_OF_LISTS_WITH_TWO_ITEMS.as_ptr(),
                ));
                return;
            }

            let first = tv_list_first(l);
            let second = if ptr::eq(first, (*l).lv_last) {
                ptr::null_mut()
            } else {
                (*first).li_next
            };
            if !digraph_set_common(&raw const (*first).li_tv, &raw const (*second).li_tv) {
                return;
            }
            pli = if ptr::eq(pli, (*pl).lv_last) {
                ptr::null_mut()
            } else {
                (*pli).li_next
            };
        }

        (*rettv).vval.v_bool = kBoolVarTrue;
    }
}

/// Set up key mapping tables for the `'keymap'` option (`keymap_init`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn keymap_init() -> *mut c_char {
    unsafe {
        (*curbuf).b_kmap_state &= !KEYMAP_INIT;

        if *(*curbuf).b_p_keymap == NUL {
            keymap_unload();
            do_cmdline_cmd(c"unlet! b:keymap_name".as_ptr());
        } else {
            let buflen = CStr::from_ptr((*curbuf).b_p_keymap).to_bytes().len()
                + CStr::from_ptr(p_enc).to_bytes().len()
                + 14;
            let buf = xmalloc(buflen).cast::<c_char>();

            vim_snprintf(
                buf,
                buflen,
                c"keymap/%s_%s.vim".as_ptr(),
                (*curbuf).b_p_keymap,
                p_enc,
            );

            if source_runtime(buf, 0) == FAIL {
                vim_snprintf(buf, buflen, c"keymap/%s.vim".as_ptr(), (*curbuf).b_p_keymap);

                if source_runtime(buf, 0) == FAIL {
                    xfree(buf.cast());
                    return c"E544: Keymap file not found".as_ptr().cast_mut();
                }
            }
            xfree(buf.cast());
        }

        ptr::null_mut()
    }
}

/// `:loadkeymap` command: load the following lines as the keymap (`ex_loadkeymap`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_loadkeymap(eap: *mut exarg_T) {
    unsafe {
        let mut buf = [0 as c_char; KMAP_LLEN + 11];
        let save_cpo = p_cpo;
        let fgetline: LineGetter = if (*eap).ea_getline.is_null() {
            None
        } else {
            Some(std::mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char,
            >((*eap).ea_getline))
        };

        if !getline_equal(fgetline, (*eap).cookie, Some(getsourceline)) {
            emsg(gettext(
                c"E105: Using :loadkeymap not in a sourced file".as_ptr(),
            ));
            return;
        }

        let Some(getline_fn) = fgetline else {
            return;
        };

        if (*eap).skip != 0 {
            loop {
                let line = getline_fn(0, (*eap).cookie, 0, true);
                if line.is_null() {
                    break;
                }
                xfree(line.cast());
            }
            return;
        }

        keymap_unload();

        (*curbuf).b_kmap_state = 0;
        ga_init(
            &raw mut (*curbuf).b_kmap_ga,
            size_of::<kmap_T>() as c_int,
            20,
        );

        p_cpo = c"C".as_ptr().cast_mut();

        loop {
            let line = getline_fn(0, (*eap).cookie, 0, true);
            if line.is_null() {
                break;
            }

            let mut p = skipwhite(line);
            if *p != b'"' as c_char && *p != NUL {
                ga_grow(&raw mut (*curbuf).b_kmap_ga, 1);
                let kp = (*curbuf)
                    .b_kmap_ga
                    .ga_data
                    .cast::<kmap_T>()
                    .add((*curbuf).b_kmap_ga.ga_len as usize);
                (*curbuf).b_kmap_ga.ga_len += 1;

                let mut s = skiptowhite(p);
                (*kp).from = xmemdupz(p.cast(), s.offset_from(p) as usize).cast();
                p = skipwhite(s);
                s = skiptowhite(p);
                (*kp).to = xmemdupz(p.cast(), s.offset_from(p) as usize).cast();

                if CStr::from_ptr((*kp).from).to_bytes().len()
                    + CStr::from_ptr((*kp).to).to_bytes().len()
                    >= KMAP_LLEN
                    || *(*kp).from == NUL
                    || *(*kp).to == NUL
                {
                    if *(*kp).to == NUL {
                        emsg(gettext(c"E791: Empty keymap entry".as_ptr()));
                    }
                    xfree((*kp).from.cast());
                    xfree((*kp).to.cast());
                    (*curbuf).b_kmap_ga.ga_len -= 1;
                }
            }
            xfree(line.cast());
        }

        for i in 0..(*curbuf).b_kmap_ga.ga_len {
            let kp = (*curbuf).b_kmap_ga.ga_data.cast::<kmap_T>().add(i as usize);
            vim_snprintf(
                buf.as_mut_ptr(),
                buf.len(),
                c"<buffer> %s %s".as_ptr(),
                (*kp).from,
                (*kp).to,
            );
            do_map(MAPTYPE_MAP, buf.as_mut_ptr(), MODE_LANGMAP, false);
        }

        p_cpo = save_cpo;

        (*curbuf).b_kmap_state |= KEYMAP_LOADED;
        status_redraw_curbuf();
    }
}

/// Frees the `buf_T.b_kmap_ga` field of a buffer (`keymap_ga_clear`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn keymap_ga_clear(kmap_ga: *mut garray_T) {
    unsafe {
        let kp = (*kmap_ga).ga_data.cast::<kmap_T>();
        for i in 0..(*kmap_ga).ga_len {
            let entry = kp.add(i as usize);
            xfree((*entry).from.cast());
            xfree((*entry).to.cast());
        }
    }
}

/// Stop using `'keymap'` (`keymap_unload`).
unsafe fn keymap_unload() {
    unsafe {
        let mut buf = [0 as c_char; KMAP_MAXLEN + 10];
        let save_cpo = p_cpo;

        if ((*curbuf).b_kmap_state & KEYMAP_LOADED) == 0 {
            return;
        }

        p_cpo = c"C".as_ptr().cast_mut();

        let kp = (*curbuf).b_kmap_ga.ga_data.cast::<kmap_T>();
        for i in 0..(*curbuf).b_kmap_ga.ga_len {
            let entry = kp.add(i as usize);
            vim_snprintf(
                buf.as_mut_ptr(),
                buf.len(),
                c"<buffer> %s".as_ptr(),
                (*entry).from,
            );
            do_map(MAPTYPE_UNMAP, buf.as_mut_ptr(), MODE_LANGMAP, false);
        }
        keymap_ga_clear(&raw mut (*curbuf).b_kmap_ga);

        p_cpo = save_cpo;

        ga_clear(&raw mut (*curbuf).b_kmap_ga);
        (*curbuf).b_kmap_state &= !KEYMAP_LOADED;
        status_redraw_curbuf();
    }
}

/// Get the value to show for the language mappings, active `'keymap'` (`get_keymap_str`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_keymap_str(
    wp: *mut win_T,
    fmt: *mut c_char,
    buf: *mut c_char,
    len: c_int,
) -> c_int {
    unsafe {
        if (*(*wp).w_buffer).b_p_iminsert != i64::from(B_IMODE_LMAP) {
            return 0;
        }

        let old_curbuf = curbuf;
        let old_curwin = curwin;
        let mut to_evaluate: [c_char; 14] = [
            b'b' as c_char,
            b':' as c_char,
            b'k' as c_char,
            b'e' as c_char,
            b'y' as c_char,
            b'm' as c_char,
            b'a' as c_char,
            b'p' as c_char,
            b'_' as c_char,
            b'n' as c_char,
            b'a' as c_char,
            b'm' as c_char,
            b'e' as c_char,
            NUL,
        ];

        curbuf = (*wp).w_buffer;
        curwin = wp;
        emsg_skip += 1;
        let s = eval_to_string(to_evaluate.as_mut_ptr(), false, false);
        let mut p: *const c_char = s;
        emsg_skip -= 1;
        curbuf = old_curbuf;
        curwin = old_curwin;
        if p.is_null() || *p == NUL {
            if ((*(*wp).w_buffer).b_kmap_state & KEYMAP_LOADED) != 0 {
                p = (*(*wp).w_buffer).b_p_keymap;
            } else {
                p = c"lang".as_ptr();
            }
        }
        let mut plen = vim_snprintf(buf, len as usize, fmt, p);
        xfree(s.cast());
        if plen < 0 || plen > len - 1 {
            *buf = NUL;
            plen = 0;
        }

        plen
    }
}
