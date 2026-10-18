//! Multibyte UTF-8 character and encoding handling (`src/nvim/mbyte.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_unsafe,
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::enum_variant_names
)]

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::tabpage::win_T;
use crate::arabic::arabic_combine;
use crate::charset::{varnumber_T, vim_isprintc, vim_iswordc_tab};
use crate::clipboard::{VarType, list_T, listitem_T, typval_T};
use crate::cursor::{colnr_T, get_cursor_pos_ptr, linenr_T};
use crate::eval::deprecated::{EvalFuncData, buf_T, pos_T};
use crate::grid::{CharBoundsOff, schar_T, schar_from_buf};
use crate::mark::mark_mb_adjustpos;
use crate::memory::{xfree, xmalloc, xstrdup};
use crate::os::env::{expand_T, os_getenv_noalloc};
use crate::plines::{CharInfo, ClusterInfo, StrCharInfo};

pub type GraphemeState = i32;
pub type iconv_t = *mut c_void;

#[cfg(windows)]
type wint_t = u16;
#[cfg(not(windows))]
type wint_t = u32;

const NUL: c_char = 0;
const TAB: c_char = b'\t' as c_char;
const NL: c_char = b'\n' as c_char;
const OK: c_int = 1;
const FAIL: c_int = 0;
const IOSIZE: usize = 1025;
const NUMBUFLEN: usize = 65;
const MAXCOL: usize = 0x7fff_ffff;
const MAX_SCHAR_SIZE: usize = 32;
const MB_MAXCHAR: c_int = 6;
const GRAPHEME_STATE_INIT: GraphemeState = 0;
const UPD_NOT_VALID: c_int = 40;

const K_SPECIAL: u8 = 0x80;
const KS_SPECIAL: u8 = 254;
const KE_FILLER: u8 = 0x58;

const ENC_8BIT: c_int = 0x01;
const ENC_DBCS: c_int = 0x02;
const ENC_UNICODE: c_int = 0x04;
const ENC_ENDIAN_B: c_int = 0x10;
const ENC_ENDIAN_L: c_int = 0x20;
const ENC_2BYTE: c_int = 0x40;
const ENC_4BYTE: c_int = 0x80;
const ENC_2WORD: c_int = 0x100;
const ENC_LATIN1: c_int = 0x200;
const ENC_LATIN9: c_int = 0x400;
const ENC_MACROMAN: c_int = 0x800;

const CONV_NONE: c_int = 0;
const CONV_TO_UTF8: c_int = 1;
const CONV_9_TO_UTF8: c_int = 2;
const CONV_TO_LATIN1: c_int = 3;
const CONV_TO_LATIN9: c_int = 4;
const CONV_ICONV: c_int = 5;

const DBCS_JPN: c_int = 932;
const DBCS_JPNU: c_int = 9932;
const DBCS_KOR: c_int = 949;
const DBCS_KORU: c_int = 9949;
const DBCS_CHS: c_int = 936;
const DBCS_CHSU: c_int = 9936;
const DBCS_CHT: c_int = 950;
const DBCS_CHTU: c_int = 9950;
const DBCS_DEBUG: c_int = -1;

const kOptCmpFlagInternal: c_uint = 0x01;
const kOptCmpFlagKeepascii: c_uint = 0x02;

const UTF8PROC_CASEFOLD: c_int = 1 << 10;

const UTF8PROC_CATEGORY_LU: i16 = 1;
const UTF8PROC_CATEGORY_MN: i16 = 6;
const UTF8PROC_CATEGORY_ME: i16 = 8;
const UTF8PROC_CATEGORY_NO: i16 = 11;

const UTF8PROC_BOUNDCLASS_OTHER: u32 = 1;
const UTF8PROC_BOUNDCLASS_CR: u32 = 2;
const UTF8PROC_BOUNDCLASS_CONTROL: u32 = 4;
const UTF8PROC_BOUNDCLASS_REGIONAL_INDICATOR: u32 = 11;
const UTF8PROC_BOUNDCLASS_SPACINGMARK: u32 = 12;
const UTF8PROC_BOUNDCLASS_PREPEND: u32 = 13;
const UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC: u32 = 19;

const ICONV_E2BIG: c_int = 7;
const ICONV_EINVAL: c_int = 22;
#[cfg(windows)]
const ICONV_EILSEQ: c_int = 42;
#[cfg(target_os = "linux")]
const ICONV_EILSEQ: c_int = 84;
#[cfg(all(not(windows), not(target_os = "linux")))]
const ICONV_EILSEQ: c_int = 92;

#[cfg(windows)]
const LC_CTYPE: c_int = 2;
#[cfg(target_os = "linux")]
const LC_CTYPE: c_int = 0;
#[cfg(all(not(windows), not(target_os = "linux")))]
const LC_CTYPE: c_int = 2;

#[cfg(target_os = "linux")]
const CODESET: c_int = 14;
#[cfg(all(unix, not(target_os = "linux")))]
const CODESET: c_int = 0;

#[cfg(windows)]
const CP_UTF8: u32 = 65001;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct vimconv_T {
    pub vc_type: c_int,
    pub vc_factor: c_int,
    pub vc_fd: iconv_t,
    pub vc_fail: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct utf8proc_property_t {
    pub category: i16,
    pub combining_class: i16,
    pub bidi_class: i16,
    pub decomp_type: i16,
    pub decomp_seqindex: u16,
    pub casefold_seqindex: u16,
    pub uppercase_seqindex: u16,
    pub lowercase_seqindex: u16,
    pub titlecase_seqindex: u16,
    pub comb_bits: u16,
    pub prop_bits: u32,
}

impl utf8proc_property_t {
    #[inline]
    pub const fn charwidth(&self) -> u32 {
        (self.prop_bits >> 4) & 0x3
    }

    #[inline]
    pub const fn ambiguous_width(&self) -> bool {
        ((self.prop_bits >> 6) & 0x1) != 0
    }

    #[inline]
    pub const fn boundclass(&self) -> u32 {
        (self.prop_bits >> 8) & 0x3f
    }
}

#[derive(Clone, Copy)]
struct Interval {
    first: c_int,
    last: c_int,
}

#[derive(Clone, Copy)]
struct ClInterval {
    first: u32,
    last: u32,
    cls: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct cw_interval_T {
    first: i64,
    last: i64,
    width: c_char,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkingStatus {
    kUnknown = 0,
    kWorking = 1,
    kBroken = 2,
}

#[repr(C)]
struct EncCanonEntry {
    name: &'static CStr,
    prop: c_int,
    codepage: c_int,
}

#[repr(C)]
struct EncAliasEntry {
    name: &'static CStr,
    canon: usize,
}

const E_LIST_ITEM_NR_IS_NOT_LIST: &CStr = c"E1109: List item %d is not a List";
const E_LIST_ITEM_NR_DOES_NOT_CONTAIN_3_NUMBERS: &CStr =
    c"E1110: List item %d does not contain 3 numbers";
const E_LIST_ITEM_NR_RANGE_INVALID: &CStr = c"E1111: List item %d range invalid";
const E_LIST_ITEM_NR_CELL_WIDTH_INVALID: &CStr = c"E1112: List item %d cell width invalid";
const E_OVERLAPPING_RANGES_FOR_NR: &CStr = c"E1113: Overlapping ranges for 0x%lx";
const E_ONLY_VALUES_OF_0X80_AND_HIGHER_SUPPORTED: &CStr =
    c"E1114: Only values of 0x80 and higher supported";

#[unsafe(no_mangle)]
pub static utf8len_tab: [u8; 256] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 1?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 2?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 3?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 4?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 5?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 6?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 7?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 8?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 9?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // A?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // B?
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // C?
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // D?
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // E?
    4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 1, 1, // F?
];

#[unsafe(no_mangle)]
pub static utf8len_tab_zero: [u8; 256] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 1?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 2?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 3?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 4?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 5?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 6?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 7?
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 8?
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 9?
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // A?
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // B?
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // C?
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // D?
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // E?
    4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 0, 0, // F?
];

const IDX_LATIN_1: usize = 0;
const IDX_ISO_2: usize = 1;
const IDX_ISO_3: usize = 2;
const IDX_ISO_4: usize = 3;
const IDX_ISO_5: usize = 4;
const IDX_ISO_6: usize = 5;
const IDX_ISO_7: usize = 6;
const IDX_ISO_8: usize = 7;
const IDX_ISO_9: usize = 8;
const IDX_ISO_10: usize = 9;
const IDX_ISO_11: usize = 10;
const IDX_ISO_13: usize = 11;
const IDX_ISO_14: usize = 12;
const IDX_ISO_15: usize = 13;
const IDX_KOI8_R: usize = 14;
const IDX_KOI8_U: usize = 15;
const IDX_UTF8: usize = 16;
const IDX_UCS2: usize = 17;
const IDX_UCS2LE: usize = 18;
const IDX_UTF16: usize = 19;
const IDX_UTF16BE: usize = 20;
const IDX_UTF16LE: usize = 21;
const IDX_UCS4: usize = 22;
const IDX_UCS4LE: usize = 23;
const IDX_DEBUG: usize = 24;
const IDX_EUC_JP: usize = 25;
const IDX_SJIS: usize = 26;
const IDX_EUC_KR: usize = 27;
const IDX_EUC_CN: usize = 28;
const IDX_EUC_TW: usize = 29;
const IDX_BIG5: usize = 30;
const IDX_CP437: usize = 31;
const IDX_CP737: usize = 32;
const IDX_CP775: usize = 33;
const IDX_CP850: usize = 34;
const IDX_CP852: usize = 35;
const IDX_CP855: usize = 36;
const IDX_CP857: usize = 37;
const IDX_CP860: usize = 38;
const IDX_CP861: usize = 39;
const IDX_CP862: usize = 40;
const IDX_CP863: usize = 41;
const IDX_CP865: usize = 42;
const IDX_CP866: usize = 43;
const IDX_CP869: usize = 44;
const IDX_CP874: usize = 45;
const IDX_CP932: usize = 46;
const IDX_CP936: usize = 47;
const IDX_CP949: usize = 48;
const IDX_CP950: usize = 49;
const IDX_CP1250: usize = 50;
const IDX_CP1251: usize = 51;
const IDX_CP1253: usize = 52;
const IDX_CP1254: usize = 53;
const IDX_CP1255: usize = 54;
const IDX_CP1256: usize = 55;
const IDX_CP1257: usize = 56;
const IDX_CP1258: usize = 57;
const IDX_MACROMAN: usize = 58;
const IDX_HPROMAN8: usize = 59;
const IDX_COUNT: usize = 60;

static ENC_CANON_TABLE: [EncCanonEntry; IDX_COUNT] = [
    EncCanonEntry {
        name: c"latin1",
        prop: ENC_8BIT + ENC_LATIN1,
        codepage: 1252,
    },
    EncCanonEntry {
        name: c"iso-8859-2",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-3",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-4",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-5",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-6",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-7",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-8",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-9",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-10",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-11",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-13",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-14",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"iso-8859-15",
        prop: ENC_8BIT + ENC_LATIN9,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"koi8-r",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"koi8-u",
        prop: ENC_8BIT,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"utf-8",
        prop: ENC_UNICODE,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"ucs-2",
        prop: ENC_UNICODE + ENC_ENDIAN_B + ENC_2BYTE,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"ucs-2le",
        prop: ENC_UNICODE + ENC_ENDIAN_L + ENC_2BYTE,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"utf-16",
        prop: ENC_UNICODE + ENC_ENDIAN_B + ENC_2WORD,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"utf-16be",
        prop: ENC_UNICODE + ENC_ENDIAN_B + ENC_2WORD,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"utf-16le",
        prop: ENC_UNICODE + ENC_ENDIAN_L + ENC_2WORD,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"ucs-4",
        prop: ENC_UNICODE + ENC_ENDIAN_B + ENC_4BYTE,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"ucs-4le",
        prop: ENC_UNICODE + ENC_ENDIAN_L + ENC_4BYTE,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"debug",
        prop: ENC_DBCS,
        codepage: DBCS_DEBUG,
    },
    EncCanonEntry {
        name: c"euc-jp",
        prop: ENC_DBCS,
        codepage: DBCS_JPNU,
    },
    EncCanonEntry {
        name: c"sjis",
        prop: ENC_DBCS,
        codepage: DBCS_JPN,
    },
    EncCanonEntry {
        name: c"euc-kr",
        prop: ENC_DBCS,
        codepage: DBCS_KORU,
    },
    EncCanonEntry {
        name: c"euc-cn",
        prop: ENC_DBCS,
        codepage: DBCS_CHSU,
    },
    EncCanonEntry {
        name: c"euc-tw",
        prop: ENC_DBCS,
        codepage: DBCS_CHTU,
    },
    EncCanonEntry {
        name: c"big5",
        prop: ENC_DBCS,
        codepage: DBCS_CHT,
    },
    EncCanonEntry {
        name: c"cp437",
        prop: ENC_8BIT,
        codepage: 437,
    },
    EncCanonEntry {
        name: c"cp737",
        prop: ENC_8BIT,
        codepage: 737,
    },
    EncCanonEntry {
        name: c"cp775",
        prop: ENC_8BIT,
        codepage: 775,
    },
    EncCanonEntry {
        name: c"cp850",
        prop: ENC_8BIT,
        codepage: 850,
    },
    EncCanonEntry {
        name: c"cp852",
        prop: ENC_8BIT,
        codepage: 852,
    },
    EncCanonEntry {
        name: c"cp855",
        prop: ENC_8BIT,
        codepage: 855,
    },
    EncCanonEntry {
        name: c"cp857",
        prop: ENC_8BIT,
        codepage: 857,
    },
    EncCanonEntry {
        name: c"cp860",
        prop: ENC_8BIT,
        codepage: 860,
    },
    EncCanonEntry {
        name: c"cp861",
        prop: ENC_8BIT,
        codepage: 861,
    },
    EncCanonEntry {
        name: c"cp862",
        prop: ENC_8BIT,
        codepage: 862,
    },
    EncCanonEntry {
        name: c"cp863",
        prop: ENC_8BIT,
        codepage: 863,
    },
    EncCanonEntry {
        name: c"cp865",
        prop: ENC_8BIT,
        codepage: 865,
    },
    EncCanonEntry {
        name: c"cp866",
        prop: ENC_8BIT,
        codepage: 866,
    },
    EncCanonEntry {
        name: c"cp869",
        prop: ENC_8BIT,
        codepage: 869,
    },
    EncCanonEntry {
        name: c"cp874",
        prop: ENC_8BIT,
        codepage: 874,
    },
    EncCanonEntry {
        name: c"cp932",
        prop: ENC_DBCS,
        codepage: DBCS_JPN,
    },
    EncCanonEntry {
        name: c"cp936",
        prop: ENC_DBCS,
        codepage: DBCS_CHS,
    },
    EncCanonEntry {
        name: c"cp949",
        prop: ENC_DBCS,
        codepage: DBCS_KOR,
    },
    EncCanonEntry {
        name: c"cp950",
        prop: ENC_DBCS,
        codepage: DBCS_CHT,
    },
    EncCanonEntry {
        name: c"cp1250",
        prop: ENC_8BIT,
        codepage: 1250,
    },
    EncCanonEntry {
        name: c"cp1251",
        prop: ENC_8BIT,
        codepage: 1251,
    },
    EncCanonEntry {
        name: c"cp1253",
        prop: ENC_8BIT,
        codepage: 1253,
    },
    EncCanonEntry {
        name: c"cp1254",
        prop: ENC_8BIT,
        codepage: 1254,
    },
    EncCanonEntry {
        name: c"cp1255",
        prop: ENC_8BIT,
        codepage: 1255,
    },
    EncCanonEntry {
        name: c"cp1256",
        prop: ENC_8BIT,
        codepage: 1256,
    },
    EncCanonEntry {
        name: c"cp1257",
        prop: ENC_8BIT,
        codepage: 1257,
    },
    EncCanonEntry {
        name: c"cp1258",
        prop: ENC_8BIT,
        codepage: 1258,
    },
    EncCanonEntry {
        name: c"macroman",
        prop: ENC_8BIT + ENC_MACROMAN,
        codepage: 0,
    },
    EncCanonEntry {
        name: c"hp-roman8",
        prop: ENC_8BIT,
        codepage: 0,
    },
];

static ENC_ALIAS_TABLE: &[EncAliasEntry] = &[
    EncAliasEntry {
        name: c"ansi",
        canon: IDX_LATIN_1,
    },
    EncAliasEntry {
        name: c"iso-8859-1",
        canon: IDX_LATIN_1,
    },
    EncAliasEntry {
        name: c"iso-8859",
        canon: IDX_LATIN_1,
    },
    EncAliasEntry {
        name: c"latin2",
        canon: IDX_ISO_2,
    },
    EncAliasEntry {
        name: c"latin3",
        canon: IDX_ISO_3,
    },
    EncAliasEntry {
        name: c"latin4",
        canon: IDX_ISO_4,
    },
    EncAliasEntry {
        name: c"cyrillic",
        canon: IDX_ISO_5,
    },
    EncAliasEntry {
        name: c"arabic",
        canon: IDX_ISO_6,
    },
    EncAliasEntry {
        name: c"greek",
        canon: IDX_ISO_7,
    },
    EncAliasEntry {
        name: c"hebrew",
        canon: IDX_ISO_8,
    },
    EncAliasEntry {
        name: c"latin5",
        canon: IDX_ISO_9,
    },
    EncAliasEntry {
        name: c"turkish",
        canon: IDX_ISO_9,
    },
    EncAliasEntry {
        name: c"latin6",
        canon: IDX_ISO_10,
    },
    EncAliasEntry {
        name: c"nordic",
        canon: IDX_ISO_10,
    },
    EncAliasEntry {
        name: c"thai",
        canon: IDX_ISO_11,
    },
    EncAliasEntry {
        name: c"latin7",
        canon: IDX_ISO_13,
    },
    EncAliasEntry {
        name: c"latin8",
        canon: IDX_ISO_14,
    },
    EncAliasEntry {
        name: c"latin9",
        canon: IDX_ISO_15,
    },
    EncAliasEntry {
        name: c"utf8",
        canon: IDX_UTF8,
    },
    EncAliasEntry {
        name: c"unicode",
        canon: IDX_UCS2,
    },
    EncAliasEntry {
        name: c"ucs2",
        canon: IDX_UCS2,
    },
    EncAliasEntry {
        name: c"ucs2be",
        canon: IDX_UCS2,
    },
    EncAliasEntry {
        name: c"ucs-2be",
        canon: IDX_UCS2,
    },
    EncAliasEntry {
        name: c"ucs2le",
        canon: IDX_UCS2LE,
    },
    EncAliasEntry {
        name: c"utf16",
        canon: IDX_UTF16,
    },
    EncAliasEntry {
        name: c"utf16be",
        canon: IDX_UTF16BE,
    },
    EncAliasEntry {
        name: c"utf-16be",
        canon: IDX_UTF16BE,
    },
    EncAliasEntry {
        name: c"utf16le",
        canon: IDX_UTF16LE,
    },
    EncAliasEntry {
        name: c"ucs4",
        canon: IDX_UCS4,
    },
    EncAliasEntry {
        name: c"ucs4be",
        canon: IDX_UCS4,
    },
    EncAliasEntry {
        name: c"ucs-4be",
        canon: IDX_UCS4,
    },
    EncAliasEntry {
        name: c"ucs4le",
        canon: IDX_UCS4LE,
    },
    EncAliasEntry {
        name: c"utf32",
        canon: IDX_UCS4,
    },
    EncAliasEntry {
        name: c"utf-32",
        canon: IDX_UCS4,
    },
    EncAliasEntry {
        name: c"utf32be",
        canon: IDX_UCS4,
    },
    EncAliasEntry {
        name: c"utf-32be",
        canon: IDX_UCS4,
    },
    EncAliasEntry {
        name: c"utf32le",
        canon: IDX_UCS4LE,
    },
    EncAliasEntry {
        name: c"utf-32le",
        canon: IDX_UCS4LE,
    },
    EncAliasEntry {
        name: c"932",
        canon: IDX_CP932,
    },
    EncAliasEntry {
        name: c"949",
        canon: IDX_CP949,
    },
    EncAliasEntry {
        name: c"936",
        canon: IDX_CP936,
    },
    EncAliasEntry {
        name: c"gbk",
        canon: IDX_CP936,
    },
    EncAliasEntry {
        name: c"950",
        canon: IDX_CP950,
    },
    EncAliasEntry {
        name: c"eucjp",
        canon: IDX_EUC_JP,
    },
    EncAliasEntry {
        name: c"unix-jis",
        canon: IDX_EUC_JP,
    },
    EncAliasEntry {
        name: c"ujis",
        canon: IDX_EUC_JP,
    },
    EncAliasEntry {
        name: c"shift-jis",
        canon: IDX_SJIS,
    },
    EncAliasEntry {
        name: c"pck",
        canon: IDX_SJIS,
    },
    EncAliasEntry {
        name: c"euckr",
        canon: IDX_EUC_KR,
    },
    EncAliasEntry {
        name: c"5601",
        canon: IDX_EUC_KR,
    },
    EncAliasEntry {
        name: c"euccn",
        canon: IDX_EUC_CN,
    },
    EncAliasEntry {
        name: c"gb2312",
        canon: IDX_EUC_CN,
    },
    EncAliasEntry {
        name: c"euctw",
        canon: IDX_EUC_TW,
    },
    EncAliasEntry {
        name: c"japan",
        canon: IDX_EUC_JP,
    },
    EncAliasEntry {
        name: c"korea",
        canon: IDX_EUC_KR,
    },
    EncAliasEntry {
        name: c"prc",
        canon: IDX_EUC_CN,
    },
    EncAliasEntry {
        name: c"zh-cn",
        canon: IDX_EUC_CN,
    },
    EncAliasEntry {
        name: c"chinese",
        canon: IDX_EUC_CN,
    },
    EncAliasEntry {
        name: c"zh-tw",
        canon: IDX_EUC_TW,
    },
    EncAliasEntry {
        name: c"taiwan",
        canon: IDX_EUC_TW,
    },
    EncAliasEntry {
        name: c"cp950",
        canon: IDX_BIG5,
    },
    EncAliasEntry {
        name: c"950",
        canon: IDX_BIG5,
    },
    EncAliasEntry {
        name: c"mac",
        canon: IDX_MACROMAN,
    },
    EncAliasEntry {
        name: c"mac-roman",
        canon: IDX_MACROMAN,
    },
];

static mut cw_table: *mut cw_interval_T = ptr::null_mut();
static mut cw_table_size: usize = 0;
static mut mb_unescape_buf: [c_char; 6] = [0; 6];
static mut iconv_working: WorkingStatus = WorkingStatus::kUnknown;

unsafe extern "C" {
    static mut curbuf: *mut buf_T;
    static mut curwin: *mut win_T;
    static mut p_ambw: *mut c_char;
    static mut p_emoji: c_int;
    static mut p_enc: *mut c_char;
    static mut cmp_flags: c_uint;
    static mut dy_escape_width: c_int;
    static mut fenc_default: *mut c_char;
    static mut IObuff: [c_char; IOSIZE];
    static e_listreq: [c_char; 0];

    fn utf8proc_get_property(codepoint: i32) -> *const utf8proc_property_t;
    fn utf8proc_grapheme_break(codepoint1: i32, codepoint2: i32) -> bool;
    fn utf8proc_grapheme_break_stateful(
        codepoint1: i32,
        codepoint2: i32,
        state: *mut GraphemeState,
    ) -> bool;
    fn utf8proc_decompose_char(
        codepoint: i32,
        dst: *mut i32,
        bufsize: isize,
        options: c_int,
        last_boundclass: *mut c_int,
    ) -> isize;
    fn utf8proc_toupper(c: i32) -> i32;
    fn utf8proc_tolower(c: i32) -> i32;

    fn toupper(c: c_int) -> c_int;
    fn tolower(c: c_int) -> c_int;
    fn towupper(wc: wint_t) -> wint_t;
    fn towlower(wc: wint_t) -> wint_t;
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
    #[cfg(unix)]
    fn nl_langinfo(item: c_int) -> *mut c_char;

    #[cfg(windows)]
    fn libiconv_open(tocode: *const c_char, fromcode: *const c_char) -> iconv_t;
    #[cfg(windows)]
    fn libiconv(
        cd: iconv_t,
        inbuf: *mut *const c_char,
        inbytesleft: *mut usize,
        outbuf: *mut *mut c_char,
        outbytesleft: *mut usize,
    ) -> usize;
    #[cfg(windows)]
    fn libiconv_close(cd: iconv_t) -> c_int;
    #[cfg(windows)]
    fn _errno() -> *mut c_int;
    #[cfg(windows)]
    fn uv_translate_sys_error(sys_errno: c_int) -> c_int;

    #[cfg(not(windows))]
    fn iconv_open(tocode: *const c_char, fromcode: *const c_char) -> iconv_t;
    #[cfg(not(windows))]
    fn iconv(
        cd: iconv_t,
        inbuf: *mut *const c_char,
        inbytesleft: *mut usize,
        outbuf: *mut *mut c_char,
        outbytesleft: *mut usize,
    ) -> usize;
    #[cfg(not(windows))]
    fn iconv_close(cd: iconv_t) -> c_int;
    #[cfg(target_os = "linux")]
    fn __errno_location() -> *mut c_int;
    #[cfg(all(not(windows), not(target_os = "linux")))]
    fn __error() -> *mut c_int;

    fn ml_get_buf(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char;
    fn msg(s: *const c_char, attr: c_int) -> bool;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(s: *const c_char, ...) -> bool;
    fn beep_flush();
    fn tv_get_string(varp: *const typval_T) -> *const c_char;
    fn tv_get_string_buf(varp: *const typval_T, buf: *mut c_char) -> *const c_char;
    fn tv_check_for_string_arg(argvars: *const typval_T, idx: c_int) -> c_int;
    fn tv_list_alloc(len: isize) -> *mut list_T;
    fn tv_list_alloc_ret(ret_tv: *mut typval_T, len: isize) -> *mut list_T;
    fn tv_list_append_number(l: *mut list_T, n: varnumber_T) -> c_int;
    fn tv_list_append_list(l: *mut list_T, itemlist: *mut list_T) -> c_int;
    fn check_chars_options() -> *const c_char;
    fn changed_window_setting_all();
    fn redraw_all_later(type_: c_int);
}

#[inline]
const fn ascii_iswhite(c: c_int) -> bool {
    c == b' ' as c_int || c == b'\t' as c_int
}

#[inline]
const fn toupper_asc(c: c_int) -> c_int {
    if c >= b'a' as c_int && c <= b'z' as c_int {
        c - 0x20
    } else {
        c
    }
}

#[inline]
const fn tolower_asc(c: c_int) -> c_int {
    if c >= b'A' as c_int && c <= b'Z' as c_int {
        c + 0x20
    } else {
        c
    }
}

#[inline]
unsafe fn vim_strchr(string: *const c_char, c: c_int) -> *const c_char {
    if string.is_null() || c <= 0 {
        return ptr::null();
    }
    let mut p = string;
    unsafe {
        while *p != 0 {
            if c_int::from(*p.cast::<u8>()) == c {
                return p;
            }
            p = p.add(1);
        }
    }
    ptr::null()
}

#[inline]
unsafe fn vim_strnicmp_asc(s1: *const c_char, s2: *const c_char, mut n: usize) -> c_int {
    let mut p1 = s1.cast::<u8>();
    let mut p2 = s2.cast::<u8>();
    unsafe {
        while n > 0 {
            let c1 = tolower_asc(c_int::from(*p1));
            let c2 = tolower_asc(c_int::from(*p2));
            if c1 != c2 {
                return c1 - c2;
            }
            if c1 == 0 {
                return 0;
            }
            p1 = p1.add(1);
            p2 = p2.add(1);
            n -= 1;
        }
    }
    0
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

#[cfg(windows)]
unsafe extern "system" {
    fn MultiByteToWideChar(
        CodePage: u32,
        dwFlags: u32,
        lpMultiByteStr: *const c_char,
        cbMultiByte: c_int,
        lpWideCharStr: *mut u16,
        cchWideChar: c_int,
    ) -> c_int;
    fn WideCharToMultiByte(
        CodePage: u32,
        dwFlags: u32,
        lpWideCharStr: *const u16,
        cchWideChar: c_int,
        lpMultiByteStr: *mut c_char,
        cbMultiByte: c_int,
        lpDefaultChar: *const c_char,
        lpUsedDefaultChar: *mut c_int,
    ) -> c_int;
    fn GetLastError() -> u32;
}

#[inline]
unsafe fn call_iconv_open(tocode: *const c_char, fromcode: *const c_char) -> iconv_t {
    #[cfg(windows)]
    unsafe {
        libiconv_open(tocode, fromcode)
    }
    #[cfg(not(windows))]
    unsafe {
        iconv_open(tocode, fromcode)
    }
}

#[inline]
unsafe fn call_iconv(
    cd: iconv_t,
    inbuf: *mut *const c_char,
    inbytesleft: *mut usize,
    outbuf: *mut *mut c_char,
    outbytesleft: *mut usize,
) -> usize {
    #[cfg(windows)]
    unsafe {
        libiconv(cd, inbuf, inbytesleft, outbuf, outbytesleft)
    }
    #[cfg(not(windows))]
    unsafe {
        iconv(cd, inbuf, inbytesleft, outbuf, outbytesleft)
    }
}

#[inline]
unsafe fn call_iconv_close(cd: iconv_t) -> c_int {
    #[cfg(windows)]
    unsafe {
        libiconv_close(cd)
    }
    #[cfg(not(windows))]
    unsafe {
        iconv_close(cd)
    }
}

#[inline]
unsafe fn iconv_errno() -> c_int {
    #[cfg(windows)]
    unsafe {
        *_errno()
    }
    #[cfg(target_os = "linux")]
    unsafe {
        *__errno_location()
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    unsafe {
        *__error()
    }
}

#[inline]
const fn utf_is_trail_byte(byte: u8) -> bool {
    (byte & 0xC0) == 0x80
}

#[inline]
unsafe fn ascii2cells(b: c_int) -> c_int {
    if (b' ' as c_int..=b'~' as c_int).contains(&b) {
        1
    } else {
        unsafe { dy_escape_width }
    }
}

#[inline]
unsafe fn basechar_cells_impl(c: CharInfo) -> c_int {
    if c.value < 0 {
        4
    } else if c.value < 0x80 {
        unsafe { ascii2cells(c.value) }
    } else {
        unsafe { utf_char2cells(c.value) }
    }
}

#[inline]
unsafe fn utf_ptr2CharInfo(p_in: *const c_char) -> CharInfo {
    unsafe {
        let p = p_in.cast::<u8>();
        let first = *p;
        if first < 0x80 {
            CharInfo {
                value: i32::from(first),
                len: 1,
            }
        } else {
            let mut len = c_int::from(utf8len_tab[first as usize]);
            let code_point = utf_ptr2CharInfo_impl(p, len as usize);
            if code_point < 0 {
                len = 1;
            }
            CharInfo {
                value: code_point,
                len,
            }
        }
    }
}

#[inline]
unsafe fn utf_ptr2StrCharInfo(ptr: *const c_char) -> StrCharInfo {
    unsafe {
        StrCharInfo {
            ptr,
            chr: utf_ptr2CharInfo(ptr),
        }
    }
}

#[inline]
unsafe fn utf_ptr2CharInfo_len(p_in: *const c_char, size: c_int) -> CharInfo {
    unsafe {
        let p = p_in.cast::<u8>();
        assert!(size >= 0);
        if size == 0 {
            return CharInfo { value: -1, len: 1 };
        }
        let first = *p;
        if first < 0x80 {
            CharInfo {
                value: i32::from(first),
                len: 1,
            }
        } else {
            let mut len = c_int::from(utf8len_tab[first as usize]);
            if len > size {
                return CharInfo { value: -1, len: 1 };
            }
            let code_point = utf_ptr2CharInfo_impl(p, len as usize);
            if code_point < 0 {
                len = 1;
            }
            CharInfo {
                value: code_point,
                len,
            }
        }
    }
}

#[inline]
unsafe fn utf_ptr2StrCharInfo_len(ptr: *const c_char, size: c_int) -> StrCharInfo {
    unsafe {
        StrCharInfo {
            ptr,
            chr: utf_ptr2CharInfo_len(ptr, size),
        }
    }
}

#[inline]
unsafe fn utf_ClusterInfo(cur: StrCharInfo) -> ClusterInfo {
    unsafe {
        if *cur.ptr == NUL {
            return ClusterInfo {
                next: cur,
                cells: dy_escape_width,
            };
        }
        let next = cur.ptr.add(cur.chr.len as usize).cast::<u8>();
        if *next < 0x80 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
                    chr: CharInfo {
                        value: i32::from(*next),
                        len: 1,
                    },
                },
                cells: basechar_cells_impl(cur.chr),
            };
        }
        let mut scratch = c_int::MAX;
        utf_ClusterInfo_impl(cur, &raw mut scratch)
    }
}

#[inline]
unsafe fn utf_ClusterInfo_len(cur: StrCharInfo, size: *mut c_int) -> ClusterInfo {
    unsafe {
        *size -= cur.chr.len;
        let next = cur.ptr.add(cur.chr.len as usize).cast::<u8>();
        if *size == 0 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
                    chr: CharInfo { value: -1, len: 1 },
                },
                cells: basechar_cells_impl(cur.chr),
            };
        }
        if *next < 0x80 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
                    chr: CharInfo {
                        value: i32::from(*next),
                        len: 1,
                    },
                },
                cells: basechar_cells_impl(cur.chr),
            };
        }
        utf_ClusterInfo_impl(cur, size)
    }
}

unsafe fn enc_canon_search(name: *const c_char) -> c_int {
    let bytes = unsafe { CStr::from_ptr(name).to_bytes() };
    for (i, entry) in ENC_CANON_TABLE.iter().enumerate() {
        if bytes == entry.name.to_bytes() {
            return i as c_int;
        }
    }
    -1
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn enc_canon_props(name: *const c_char) -> c_int {
    unsafe {
        let i = enc_canon_search(name);
        if i >= 0 {
            return ENC_CANON_TABLE[i as usize].prop;
        }
        let bytes = CStr::from_ptr(name).to_bytes();
        if bytes.starts_with(b"2byte-") {
            ENC_DBCS
        } else if bytes.starts_with(b"8bit-") || bytes.starts_with(b"iso-8859-") {
            ENC_8BIT
        } else {
            0
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bomb_size() -> c_int {
    unsafe {
        let mut n = 0;
        if (*curbuf).b_p_bomb != 0 && (*curbuf).b_p_bin == 0 {
            let fenc = CStr::from_ptr((*curbuf).b_p_fenc).to_bytes();
            if fenc.is_empty() || fenc == b"utf-8" {
                n = 3;
            } else if fenc.starts_with(b"ucs-2") || fenc.starts_with(b"utf-16") {
                n = 2;
            } else if fenc.starts_with(b"ucs-4") {
                n = 4;
            }
        }
        n
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remove_bom(s: *mut c_char) {
    unsafe {
        let mut p = s.cast::<u8>();
        while *p != 0 {
            if *p == 0xef && *p.add(1) == 0xbb && *p.add(2) == 0xbf {
                let tail_len = CStr::from_ptr(p.add(3).cast()).to_bytes_with_nul().len();
                ptr::copy(p.add(3), p, tail_len);
            } else {
                p = p.add(1);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_get_class(p: *const c_char) -> c_int {
    unsafe { mb_get_class_tab(p, (*curbuf).b_chartab.as_ptr()) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_get_class_tab(p: *const c_char, chartab: *const u64) -> c_int {
    unsafe {
        let b0 = *p.cast::<u8>();
        if utf8len_tab[b0 as usize] == 1 {
            if b0 == 0 || ascii_iswhite(c_int::from(b0)) {
                return 0;
            }
            if vim_iswordc_tab(c_int::from(b0), chartab) {
                return 2;
            }
            return 1;
        }
        utf_class_tab(utf_ptr2char(p), chartab)
    }
}

#[inline]
unsafe fn prop_is_emojilike(prop: *const utf8proc_property_t) -> bool {
    let bc = unsafe { (*prop).boundclass() };
    bc == UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC || bc == UTF8PROC_BOUNDCLASS_REGIONAL_INDICATOR
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_char2cells(c: c_int) -> c_int {
    unsafe {
        if c < 0x80 {
            return 1;
        }

        if !vim_isprintc(c) {
            assert!(c <= 0xFFFF);
            return if c > 0xFF { 6 } else { 4 };
        }

        let n = cw_value(c);
        if n != 0 {
            return n;
        }

        let prop = utf8proc_get_property(c);
        if (*prop).charwidth() == 2 {
            return 2;
        }
        if *p_ambw == b'd' as c_char && (*prop).ambiguous_width() {
            return 2;
        }

        if p_emoji != 0 && c >= 0x1f000 && !(*prop).ambiguous_width() && prop_is_emojilike(prop) {
            return 2;
        }

        1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ptr2cells(p_in: *const c_char) -> c_int {
    unsafe { utf_ClusterInfo(utf_ptr2StrCharInfo(p_in)).cells }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_ptr2cells(p_in: *const c_char) -> c_int {
    unsafe {
        if *p_in.cast::<u8>() < 0x80 {
            return 1;
        }
        ptr2cells(p_in)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_ptr2CharInfo_impl(p: *const u8, len: usize) -> i32 {
    unsafe {
        const CORRECTIONS: [u32; 7] = [
            1u32 << 31,
            1u32 << 31,
            0u32.wrapping_sub(0x80u32 + (0xC0u32 << 6)),
            0u32.wrapping_sub(0x80u32 + (0x80u32 << 6) + (0xE0u32 << 12)),
            0u32.wrapping_sub(0x80u32 + (0x80u32 << 6) + (0x80u32 << 12) + (0xF0u32 << 18)),
            0u32.wrapping_sub(
                0x80u32 + (0x80u32 << 6) + (0x80u32 << 12) + (0x80u32 << 18) + (0xF8u32 << 24),
            ),
            0u32.wrapping_sub(
                0x80u32 + (0x80u32 << 6) + (0x80u32 << 12) + (0x80u32 << 18) + (0x80u32 << 24),
            ),
        ];

        let corr = CORRECTIONS[len];
        let mut cur = *p.add(1);
        let mut code_point = (u32::from(*p) << 6).wrapping_add(u32::from(cur));
        if (cur & 0xC0) != 0x80 {
            return -1;
        }
        if (len as u32) < 3 {
            return code_point.wrapping_add(corr) as i32;
        }

        cur = *p.add(2);
        code_point = (code_point << 6).wrapping_add(u32::from(cur));
        if (cur & 0xC0) != 0x80 {
            return -1;
        }
        if (len as u32) == 3 {
            return code_point.wrapping_add(corr) as i32;
        }

        cur = *p.add(3);
        code_point = (code_point << 6).wrapping_add(u32::from(cur));
        if (cur & 0xC0) != 0x80 {
            return -1;
        }
        if (len as u32) == 4 {
            return code_point.wrapping_add(corr) as i32;
        }

        cur = *p.add(4);
        code_point = (code_point << 6).wrapping_add(u32::from(cur));
        if (cur & 0xC0) != 0x80 {
            return -1;
        }
        if (len as u32) == 5 {
            return code_point.wrapping_add(corr) as i32;
        }

        cur = *p.add(5);
        code_point = (code_point << 6).wrapping_add(u32::from(cur));
        if (cur & 0xC0) != 0x80 {
            return -1;
        }

        code_point.wrapping_add(corr) as i32
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_string2cells(str_: *const c_char) -> usize {
    unsafe {
        let mut clen = 0usize;
        let mut ci = utf_ptr2StrCharInfo(str_);
        while *ci.ptr != NUL {
            let cli = utf_ClusterInfo(ci);
            clen += if ci.chr.value < 0x80 {
                1
            } else {
                cli.cells as usize
            };
            ci = cli.next;
        }
        clen
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_string2cells_len(str_: *const c_char, size: usize) -> usize {
    unsafe {
        let mut clen = 0usize;
        let mut len = size as c_int;
        let mut ci = utf_ptr2StrCharInfo_len(str_, len);
        while len > 0 {
            let cli = utf_ClusterInfo_len(ci, &raw mut len);
            clen += cli.cells as usize;
            ci = cli.next;
        }
        clen
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_ptr2char(p_in: *const c_char) -> c_int {
    unsafe {
        let p = p_in.cast::<u8>();
        let v0 = u32::from(*p);
        if v0 < 0x80 {
            return v0 as c_int;
        }

        let len = utf8len_tab[v0 as usize];
        if len < 2 {
            return v0 as c_int;
        }

        let v1 = u32::from(*p.add(1));
        if (v1 & 0xC0) != 0x80 {
            return v0 as c_int;
        }
        if len == 2 {
            return ((v0 << 6) + v1 - ((0xC0 << 6) + 0x80)) as c_int;
        }

        let v2 = u32::from(*p.add(2));
        if (v2 & 0xC0) != 0x80 {
            return v0 as c_int;
        }
        if len == 3 {
            return ((v0 << 12) + (v1 << 6) + v2 - ((0xE0 << 12) + (0x80 << 6) + 0x80)) as c_int;
        }

        let v3 = u32::from(*p.add(3));
        if (v3 & 0xC0) != 0x80 {
            return v0 as c_int;
        }
        if len == 4 {
            return ((v0 << 18) + (v1 << 12) + (v2 << 6) + v3
                - ((0xF0 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80)) as c_int;
        }

        let v4 = u32::from(*p.add(4));
        if (v4 & 0xC0) != 0x80 {
            return v0 as c_int;
        }
        if len == 5 {
            return ((v0 << 24) + (v1 << 18) + (v2 << 12) + (v3 << 6) + v4
                - ((0xF8 << 24) + (0x80 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80))
                as c_int;
        }

        let v5 = u32::from(*p.add(5));
        if (v5 & 0xC0) != 0x80 {
            return v0 as c_int;
        }
        ((v0 << 30)
            .wrapping_add(v1 << 24)
            .wrapping_add(v2 << 18)
            .wrapping_add(v3 << 12)
            .wrapping_add(v4 << 6)
            .wrapping_add(v5)
            .wrapping_sub((0x80 << 24) + (0x80 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80))
            as c_int
    }
}

unsafe fn utf_safe_read_char_adv(s: &mut *const c_char, n: &mut usize) -> c_int {
    unsafe {
        if *n == 0 {
            return 0;
        }

        let b0 = *(*s).cast::<u8>();
        let k = usize::from(utf8len_tab_zero[b0 as usize]);
        if k == 1 {
            *n -= 1;
            *s = (*s).add(1);
            return c_int::from(b0);
        }

        if k <= *n {
            let c = utf_ptr2char(*s);
            if c != c_int::from(b0) || (c == 0xC3 && *(*s).add(1).cast::<u8>() == 0x83) {
                *s = (*s).add(k);
                *n -= k;
                return c;
            }
        }

        -1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_ptr2char_adv(pp: *mut *const c_char) -> c_int {
    unsafe {
        let c = utf_ptr2char(*pp);
        *pp = (*pp).add(utfc_ptr2len(*pp) as usize);
        c
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_cptr2char_adv(pp: *mut *const c_char) -> c_int {
    unsafe {
        let c = utf_ptr2char(*pp);
        *pp = (*pp).add(utf_ptr2len(*pp) as usize);
        c
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_iscomposing_first(c: c_int) -> bool {
    unsafe { c >= 128 && !utf8proc_grapheme_break(b' ' as i32, c) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_composinglike(
    p1: *const c_char,
    p2: *const c_char,
    state: *mut GraphemeState,
) -> bool {
    unsafe {
        if *p2.cast::<u8>() < 128 {
            return false;
        }
        let first = utf_ptr2char(p1);
        let second = utf_ptr2char(p2);
        if !utf8proc_grapheme_break_stateful(first, second, state) {
            return true;
        }
        arabic_combine(first, second)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_iscomposing(c1: c_int, c2: c_int, state: *mut GraphemeState) -> bool {
    unsafe { !utf8proc_grapheme_break_stateful(c1, c2, state) || arabic_combine(c1, c2) }
}

unsafe fn schar_from_buf_first(buf: *const c_char, len: usize, first_compose: bool) -> schar_T {
    unsafe {
        if first_compose {
            let mut cbuf = [0 as c_char; MAX_SCHAR_SIZE];
            cbuf[0] = b' ' as c_char;
            ptr::copy_nonoverlapping(buf, cbuf.as_mut_ptr().add(1), len);
            schar_from_buf(cbuf.as_ptr(), len + 1)
        } else {
            schar_from_buf(buf, len)
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utfc_ptr2schar(p: *const c_char, firstc: *mut c_int) -> schar_T {
    unsafe {
        let c = utf_ptr2char(p);
        *firstc = c;
        let first_compose = utf_iscomposing_first(c);
        let maxlen = MAX_SCHAR_SIZE - 1 - usize::from(first_compose);
        let len = utfc_ptr2len_len(p, maxlen as c_int) as usize;
        if len == 1 && *p.cast::<u8>() >= 0x80 {
            return 0;
        }
        schar_from_buf_first(p, len, first_compose)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utfc_ptrlen2schar(
    p: *const c_char,
    mut len: c_int,
    firstc: *mut c_int,
) -> schar_T {
    unsafe {
        if (len == 1 && *p.cast::<u8>() >= 0x80) || len == 0 {
            *firstc = c_int::from(*p.cast::<u8>());
            return 0;
        }
        let c = utf_ptr2char(p);
        *firstc = c;
        let first_compose = utf_iscomposing_first(c);
        let maxlen = (MAX_SCHAR_SIZE - 1 - usize::from(first_compose)) as c_int;
        if len > maxlen {
            len = utfc_ptr2len_len(p, maxlen);
        }
        schar_from_buf_first(p, len as usize, first_compose)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_ptr2len(p_in: *const c_char) -> c_int {
    unsafe {
        let p = p_in.cast::<u8>();
        if *p == 0 {
            return 0;
        }
        let len = c_int::from(utf8len_tab[*p as usize]);
        for i in 1..len as usize {
            if (*p.add(i) & 0xc0) != 0x80 {
                return 1;
            }
        }
        len
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_byte2len(b: c_int) -> c_int {
    c_int::from(utf8len_tab[b as usize])
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_ptr2len_len(p: *const c_char, size: c_int) -> c_int {
    unsafe {
        let bytes = p.cast::<u8>();
        let len = c_int::from(utf8len_tab[*bytes as usize]);
        if len == 1 {
            return 1;
        }
        let m = len.min(size);
        for i in 1..m as usize {
            if (*bytes.add(i) & 0xc0) != 0x80 {
                return 1;
            }
        }
        len
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utfc_ptr2len(p: *const c_char) -> c_int {
    unsafe {
        let b0 = *p.cast::<u8>();
        if b0 == 0 {
            return 0;
        }
        if b0 < 0x80 && *p.add(1).cast::<u8>() < 0x80 {
            return 1;
        }

        let mut len = utf_ptr2len(p);
        if len == 1 && b0 >= 0x80 {
            return 1;
        }

        let mut prevlen = 0usize;
        let mut state: GraphemeState = GRAPHEME_STATE_INIT;
        loop {
            let next_ptr = p.add(len as usize);
            if *next_ptr.cast::<u8>() < 0x80
                || !utf_composinglike(p.add(prevlen), next_ptr, &raw mut state)
            {
                return len;
            }
            prevlen = len as usize;
            len += utf_ptr2len(next_ptr);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utfc_ptr2len_len(p: *const c_char, size: c_int) -> c_int {
    unsafe {
        if size < 1 || *p == NUL {
            return 0;
        }
        let b0 = *p.cast::<u8>();
        if b0 < 0x80 && (size == 1 || *p.add(1).cast::<u8>() < 0x80) {
            return 1;
        }

        let mut len = utf_ptr2len_len(p, size);
        if (len == 1 && b0 >= 0x80) || len > size {
            return 1;
        }

        let mut prevlen = 0usize;
        let mut state: GraphemeState = GRAPHEME_STATE_INIT;
        while len < size {
            let next_ptr = p.add(len as usize);
            if *next_ptr.cast::<u8>() < 0x80 {
                break;
            }
            let len_next_char = utf_ptr2len_len(next_ptr, size - len);
            if len_next_char > size - len {
                break;
            }
            if !utf_composinglike(p.add(prevlen), next_ptr, &raw mut state) {
                break;
            }
            prevlen = len as usize;
            len += len_next_char;
        }
        len
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_char2len(c: c_int) -> c_int {
    if c < 0x80 {
        1
    } else if c < 0x800 {
        2
    } else if c < 0x10000 {
        3
    } else if c < 0x200000 {
        4
    } else if c < 0x4000000 {
        5
    } else {
        6
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_char2bytes(c: c_int, buf: *mut c_char) -> c_int {
    unsafe {
        let u = c as u32;
        if c < 0x80 {
            *buf = c as c_char;
            1
        } else if c < 0x800 {
            *buf = (0xc0 + (u >> 6)) as u8 as c_char;
            *buf.add(1) = (0x80 + (u & 0x3f)) as u8 as c_char;
            2
        } else if c < 0x10000 {
            *buf = (0xe0 + (u >> 12)) as u8 as c_char;
            *buf.add(1) = (0x80 + ((u >> 6) & 0x3f)) as u8 as c_char;
            *buf.add(2) = (0x80 + (u & 0x3f)) as u8 as c_char;
            3
        } else if c < 0x200000 {
            *buf = (0xf0 + (u >> 18)) as u8 as c_char;
            *buf.add(1) = (0x80 + ((u >> 12) & 0x3f)) as u8 as c_char;
            *buf.add(2) = (0x80 + ((u >> 6) & 0x3f)) as u8 as c_char;
            *buf.add(3) = (0x80 + (u & 0x3f)) as u8 as c_char;
            4
        } else if c < 0x4000000 {
            *buf = (0xf8 + (u >> 24)) as u8 as c_char;
            *buf.add(1) = (0x80 + ((u >> 18) & 0x3f)) as u8 as c_char;
            *buf.add(2) = (0x80 + ((u >> 12) & 0x3f)) as u8 as c_char;
            *buf.add(3) = (0x80 + ((u >> 6) & 0x3f)) as u8 as c_char;
            *buf.add(4) = (0x80 + (u & 0x3f)) as u8 as c_char;
            5
        } else {
            *buf = (0xfc + (u >> 30)) as u8 as c_char;
            *buf.add(1) = (0x80 + ((u >> 24) & 0x3f)) as u8 as c_char;
            *buf.add(2) = (0x80 + ((u >> 18) & 0x3f)) as u8 as c_char;
            *buf.add(3) = (0x80 + ((u >> 12) & 0x3f)) as u8 as c_char;
            *buf.add(4) = (0x80 + ((u >> 6) & 0x3f)) as u8 as c_char;
            *buf.add(5) = (0x80 + (u & 0x3f)) as u8 as c_char;
            6
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_iscomposing_legacy(c: c_int) -> bool {
    unsafe {
        let prop = utf8proc_get_property(c);
        (*prop).category == UTF8PROC_CATEGORY_MN || (*prop).category == UTF8PROC_CATEGORY_ME
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_isalnum(c: c_int) -> bool {
    unsafe {
        let prop = utf8proc_get_property(c);
        (*prop).category >= UTF8PROC_CATEGORY_LU && (*prop).category <= UTF8PROC_CATEGORY_NO
    }
}

fn intable(table: &[Interval], c: c_int) -> bool {
    if c < table[0].first {
        return false;
    }
    let mut bot = 0usize;
    let mut top = table.len();
    while top > bot {
        let mid = (bot + top) >> 1;
        if table[mid].last < c {
            bot = mid + 1;
        } else if table[mid].first > c {
            top = mid;
        } else {
            return true;
        }
    }
    false
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_printable(c: c_int) -> bool {
    const NONPRINT: &[Interval] = &[
        Interval {
            first: 0x070f,
            last: 0x070f,
        },
        Interval {
            first: 0x180b,
            last: 0x180e,
        },
        Interval {
            first: 0x200b,
            last: 0x200f,
        },
        Interval {
            first: 0x202a,
            last: 0x202e,
        },
        Interval {
            first: 0x2060,
            last: 0x206f,
        },
        Interval {
            first: 0xd800,
            last: 0xdfff,
        },
        Interval {
            first: 0xfeff,
            last: 0xfeff,
        },
        Interval {
            first: 0xfff9,
            last: 0xfffb,
        },
        Interval {
            first: 0xfffe,
            last: 0xffff,
        },
    ];
    !intable(NONPRINT, c)
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_class(c: c_int) -> c_int {
    unsafe { utf_class_tab(c, (*curbuf).b_chartab.as_ptr()) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_class_tab(c: c_int, chartab: *const u64) -> c_int {
    const CLASSES: &[ClInterval] = &[
        ClInterval {
            first: 0x037e,
            last: 0x037e,
            cls: 1,
        },
        ClInterval {
            first: 0x0387,
            last: 0x0387,
            cls: 1,
        },
        ClInterval {
            first: 0x055a,
            last: 0x055f,
            cls: 1,
        },
        ClInterval {
            first: 0x0589,
            last: 0x0589,
            cls: 1,
        },
        ClInterval {
            first: 0x05be,
            last: 0x05be,
            cls: 1,
        },
        ClInterval {
            first: 0x05c0,
            last: 0x05c0,
            cls: 1,
        },
        ClInterval {
            first: 0x05c3,
            last: 0x05c3,
            cls: 1,
        },
        ClInterval {
            first: 0x05f3,
            last: 0x05f4,
            cls: 1,
        },
        ClInterval {
            first: 0x060c,
            last: 0x060c,
            cls: 1,
        },
        ClInterval {
            first: 0x061b,
            last: 0x061b,
            cls: 1,
        },
        ClInterval {
            first: 0x061f,
            last: 0x061f,
            cls: 1,
        },
        ClInterval {
            first: 0x066a,
            last: 0x066d,
            cls: 1,
        },
        ClInterval {
            first: 0x06d4,
            last: 0x06d4,
            cls: 1,
        },
        ClInterval {
            first: 0x0700,
            last: 0x070d,
            cls: 1,
        },
        ClInterval {
            first: 0x0964,
            last: 0x0965,
            cls: 1,
        },
        ClInterval {
            first: 0x0970,
            last: 0x0970,
            cls: 1,
        },
        ClInterval {
            first: 0x0df4,
            last: 0x0df4,
            cls: 1,
        },
        ClInterval {
            first: 0x0e4f,
            last: 0x0e4f,
            cls: 1,
        },
        ClInterval {
            first: 0x0e5a,
            last: 0x0e5b,
            cls: 1,
        },
        ClInterval {
            first: 0x0f04,
            last: 0x0f12,
            cls: 1,
        },
        ClInterval {
            first: 0x0f3a,
            last: 0x0f3d,
            cls: 1,
        },
        ClInterval {
            first: 0x0f85,
            last: 0x0f85,
            cls: 1,
        },
        ClInterval {
            first: 0x104a,
            last: 0x104f,
            cls: 1,
        },
        ClInterval {
            first: 0x10fb,
            last: 0x10fb,
            cls: 1,
        },
        ClInterval {
            first: 0x1361,
            last: 0x1368,
            cls: 1,
        },
        ClInterval {
            first: 0x166d,
            last: 0x166e,
            cls: 1,
        },
        ClInterval {
            first: 0x1680,
            last: 0x1680,
            cls: 0,
        },
        ClInterval {
            first: 0x169b,
            last: 0x169c,
            cls: 1,
        },
        ClInterval {
            first: 0x16eb,
            last: 0x16ed,
            cls: 1,
        },
        ClInterval {
            first: 0x1735,
            last: 0x1736,
            cls: 1,
        },
        ClInterval {
            first: 0x17d4,
            last: 0x17dc,
            cls: 1,
        },
        ClInterval {
            first: 0x1800,
            last: 0x180a,
            cls: 1,
        },
        ClInterval {
            first: 0x2000,
            last: 0x200b,
            cls: 0,
        },
        ClInterval {
            first: 0x200c,
            last: 0x2027,
            cls: 1,
        },
        ClInterval {
            first: 0x2028,
            last: 0x2029,
            cls: 0,
        },
        ClInterval {
            first: 0x202a,
            last: 0x202e,
            cls: 1,
        },
        ClInterval {
            first: 0x202f,
            last: 0x202f,
            cls: 0,
        },
        ClInterval {
            first: 0x2030,
            last: 0x205e,
            cls: 1,
        },
        ClInterval {
            first: 0x205f,
            last: 0x205f,
            cls: 0,
        },
        ClInterval {
            first: 0x2060,
            last: 0x206f,
            cls: 1,
        },
        ClInterval {
            first: 0x2070,
            last: 0x207f,
            cls: 0x2070,
        },
        ClInterval {
            first: 0x2080,
            last: 0x2094,
            cls: 0x2080,
        },
        ClInterval {
            first: 0x20a0,
            last: 0x27ff,
            cls: 1,
        },
        ClInterval {
            first: 0x2800,
            last: 0x28ff,
            cls: 0x2800,
        },
        ClInterval {
            first: 0x2900,
            last: 0x2998,
            cls: 1,
        },
        ClInterval {
            first: 0x29d8,
            last: 0x29db,
            cls: 1,
        },
        ClInterval {
            first: 0x29fc,
            last: 0x29fd,
            cls: 1,
        },
        ClInterval {
            first: 0x2e00,
            last: 0x2e7f,
            cls: 1,
        },
        ClInterval {
            first: 0x3000,
            last: 0x3000,
            cls: 0,
        },
        ClInterval {
            first: 0x3001,
            last: 0x3020,
            cls: 1,
        },
        ClInterval {
            first: 0x3030,
            last: 0x3030,
            cls: 1,
        },
        ClInterval {
            first: 0x303d,
            last: 0x303d,
            cls: 1,
        },
        ClInterval {
            first: 0x3040,
            last: 0x309f,
            cls: 0x3040,
        },
        ClInterval {
            first: 0x30a0,
            last: 0x30ff,
            cls: 0x30a0,
        },
        ClInterval {
            first: 0x3300,
            last: 0x9fff,
            cls: 0x4e00,
        },
        ClInterval {
            first: 0xac00,
            last: 0xd7a3,
            cls: 0xac00,
        },
        ClInterval {
            first: 0xf900,
            last: 0xfaff,
            cls: 0x4e00,
        },
        ClInterval {
            first: 0xfd3e,
            last: 0xfd3f,
            cls: 1,
        },
        ClInterval {
            first: 0xfe30,
            last: 0xfe6b,
            cls: 1,
        },
        ClInterval {
            first: 0xff00,
            last: 0xff0f,
            cls: 1,
        },
        ClInterval {
            first: 0xff1a,
            last: 0xff20,
            cls: 1,
        },
        ClInterval {
            first: 0xff3b,
            last: 0xff40,
            cls: 1,
        },
        ClInterval {
            first: 0xff5b,
            last: 0xff65,
            cls: 1,
        },
        ClInterval {
            first: 0x1d000,
            last: 0x1d24f,
            cls: 1,
        },
        ClInterval {
            first: 0x1d400,
            last: 0x1d7ff,
            cls: 1,
        },
        ClInterval {
            first: 0x1f000,
            last: 0x1f2ff,
            cls: 1,
        },
        ClInterval {
            first: 0x1f300,
            last: 0x1f9ff,
            cls: 1,
        },
        ClInterval {
            first: 0x20000,
            last: 0x2a6df,
            cls: 0x4e00,
        },
        ClInterval {
            first: 0x2a700,
            last: 0x2b73f,
            cls: 0x4e00,
        },
        ClInterval {
            first: 0x2b740,
            last: 0x2b81f,
            cls: 0x4e00,
        },
        ClInterval {
            first: 0x2f800,
            last: 0x2fa1f,
            cls: 0x4e00,
        },
    ];

    unsafe {
        if c < 0x100 {
            if c == b' ' as c_int || c == b'\t' as c_int || c == 0 || c == 0xa0 {
                return 0;
            }
            if vim_iswordc_tab(c, chartab) {
                return 2;
            }
            return 1;
        }

        let prop = utf8proc_get_property(c);
        if prop_is_emojilike(prop) {
            return 3;
        }

        let cu = c as u32;
        let mut bot: isize = 0;
        let mut top: isize = (CLASSES.len() as isize) - 1;
        while top >= bot {
            let mid = ((bot + top) / 2) as usize;
            if CLASSES[mid].last < cu {
                bot = (mid as isize) + 1;
            } else if CLASSES[mid].first > cu {
                top = (mid as isize) - 1;
            } else {
                return CLASSES[mid].cls as c_int;
            }
        }

        2
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_ambiguous_width(p: *const c_char) -> bool {
    unsafe {
        if *p == NUL || *p.add(1) == NUL {
            return false;
        }
        let info = utf_ptr2CharInfo(p);
        if info.value >= 0x80 {
            let prop = utf8proc_get_property(info.value);
            if (*prop).ambiguous_width() || prop_is_emojilike(prop) {
                return true;
            }
        }
        let next = p.add(info.len as usize).cast::<u8>();
        *next == 0xef && *next.add(1) == 0xb8 && *next.add(2) == 0x8f
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_fold(a: c_int) -> c_int {
    unsafe {
        if a < 0x80 {
            return if (0x41..=0x5a).contains(&a) {
                a + 32
            } else {
                a
            };
        }
        if a == 0xdf || a == 0x130 {
            return a;
        }
        let mut result: [i32; 1] = [0];
        let res = utf8proc_decompose_char(
            a,
            result.as_mut_ptr(),
            1,
            UTF8PROC_CASEFOLD,
            ptr::null_mut(),
        );
        if res == 1 { result[0] } else { a }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_toupper(a: c_int) -> c_int {
    unsafe {
        if a < 128 && (cmp_flags & kOptCmpFlagKeepascii) != 0 {
            return toupper_asc(a);
        }
        if (cmp_flags & kOptCmpFlagInternal) == 0 {
            return towupper(a as wint_t) as c_int;
        }
        if a < 128 {
            return toupper(a);
        }
        utf8proc_toupper(a)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_islower(a: c_int) -> bool {
    unsafe { mb_toupper(a) != a }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_tolower(a: c_int) -> c_int {
    unsafe {
        if a < 128 && (cmp_flags & kOptCmpFlagKeepascii) != 0 {
            return tolower_asc(a);
        }
        if (cmp_flags & kOptCmpFlagInternal) == 0 {
            return towlower(a as wint_t) as c_int;
        }
        if a < 128 {
            return tolower(a);
        }
        utf8proc_tolower(a)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_isupper(a: c_int) -> bool {
    unsafe { mb_tolower(a) != a }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_isalpha(a: c_int) -> bool {
    unsafe { mb_islower(a) || mb_isupper(a) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_strnicmp(
    mut s1: *const c_char,
    mut s2: *const c_char,
    mut n1: usize,
    mut n2: usize,
) -> c_int {
    unsafe {
        let mut c1: c_int;
        let mut c2: c_int;
        let mut buffer = [0 as c_char; 6];

        loop {
            c1 = utf_safe_read_char_adv(&mut s1, &mut n1);
            c2 = utf_safe_read_char_adv(&mut s2, &mut n2);
            if c1 <= 0 || c2 <= 0 {
                break;
            }
            if c1 == c2 {
                continue;
            }
            let cdiff = utf_fold(c1) - utf_fold(c2);
            if cdiff != 0 {
                return cdiff;
            }
        }

        if c1 == 0 || c2 == 0 {
            if c1 == 0 && c2 == 0 {
                return 0;
            }
            return if c1 == 0 { -1 } else { 1 };
        }

        if c1 != -1 && c2 == -1 {
            n1 = utf_char2bytes(utf_fold(c1), buffer.as_mut_ptr()) as usize;
            s1 = buffer.as_ptr();
        } else if c2 != -1 && c1 == -1 {
            n2 = utf_char2bytes(utf_fold(c2), buffer.as_mut_ptr()) as usize;
            s2 = buffer.as_ptr();
        }

        while n1 > 0 && n2 > 0 && *s1 != NUL && *s2 != NUL {
            let cdiff = c_int::from(*s1.cast::<u8>()) - c_int::from(*s2.cast::<u8>());
            if cdiff != 0 {
                return cdiff;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            n1 -= 1;
            n2 -= 1;
        }

        if n1 > 0 && *s1 == NUL {
            n1 = 0;
        }
        if n2 > 0 && *s2 == NUL {
            n2 = 0;
        }
        if n1 == 0 && n2 == 0 {
            return 0;
        }
        if n1 == 0 { -1 } else { 1 }
    }
}

#[cfg(windows)]
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf8_to_utf16(
    utf8: *const c_char,
    utf8len: c_int,
    utf16: *mut *mut u16,
) -> c_int {
    unsafe {
        let mut bufsize = MultiByteToWideChar(CP_UTF8, 0, utf8, utf8len, ptr::null_mut(), 0);
        if bufsize == 0 {
            *utf16 = ptr::null_mut();
            return uv_translate_sys_error(GetLastError() as c_int);
        }

        *utf16 = xmalloc(size_of::<u16>() * ((bufsize as usize) + 1)).cast::<u16>();
        bufsize = MultiByteToWideChar(CP_UTF8, 0, utf8, utf8len, *utf16, bufsize);
        if bufsize == 0 {
            xfree((*utf16).cast());
            *utf16 = ptr::null_mut();
            return uv_translate_sys_error(GetLastError() as c_int);
        }

        *(*utf16).add(bufsize as usize) = 0;
        0
    }
}

#[cfg(windows)]
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf16_to_utf8(
    utf16: *const u16,
    utf16len: c_int,
    utf8: *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut bufsize = WideCharToMultiByte(
            CP_UTF8,
            0,
            utf16,
            utf16len,
            ptr::null_mut(),
            0,
            ptr::null(),
            ptr::null_mut(),
        );
        if bufsize == 0 {
            *utf8 = ptr::null_mut();
            return uv_translate_sys_error(GetLastError() as c_int);
        }

        *utf8 = xmalloc((bufsize as usize) + 1).cast::<c_char>();
        bufsize = WideCharToMultiByte(
            CP_UTF8,
            0,
            utf16,
            utf16len,
            *utf8,
            bufsize,
            ptr::null(),
            ptr::null_mut(),
        );
        if bufsize == 0 {
            xfree((*utf8).cast());
            *utf8 = ptr::null_mut();
            return uv_translate_sys_error(GetLastError() as c_int);
        }

        *(*utf8).add(bufsize as usize) = NUL;
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_utflen(
    s: *const c_char,
    len: usize,
    codepoints: *mut usize,
    codeunits: *mut usize,
) {
    unsafe {
        let mut count = 0usize;
        let mut extra = 0usize;
        let mut i = 0usize;
        while i < len {
            let clen = utf_ptr2len_len(s.add(i), (len - i) as c_int) as usize;
            let c = if clen > 1 {
                utf_ptr2char(s.add(i))
            } else {
                c_int::from(*s.add(i).cast::<u8>())
            };
            count += 1;
            if c > 0xFFFF {
                extra += 1;
            }
            i += clen;
        }
        *codepoints += count;
        *codeunits += count + extra;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_utf_index_to_bytes(
    s: *const c_char,
    len: usize,
    index: usize,
    use_utf16_units: bool,
) -> isize {
    unsafe {
        if index == 0 {
            return 0;
        }
        let mut count = 0usize;
        let mut i = 0usize;
        while i < len {
            let clen = utf_ptr2len_len(s.add(i), (len - i) as c_int) as usize;
            let c = if clen > 1 {
                utf_ptr2char(s.add(i))
            } else {
                c_int::from(*s.add(i).cast::<u8>())
            };
            count += 1;
            if use_utf16_units && c > 0xFFFF {
                count += 1;
            }
            if count >= index {
                return (i + clen).min(len) as isize;
            }
            i += clen;
        }
        -1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_strnicmp(s1: *const c_char, s2: *const c_char, nn: usize) -> c_int {
    unsafe { utf_strnicmp(s1, s2, nn, nn) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_stricmp(s1: *const c_char, s2: *const c_char) -> c_int {
    unsafe { mb_strnicmp(s1, s2, MAXCOL) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn show_utf8() {
    unsafe {
        let line = get_cursor_pos_ptr();
        let len = utfc_ptr2len(line);
        if len == 0 {
            msg(c"NUL".as_ptr(), 0);
            return;
        }

        let mut out = String::new();
        let mut clen = 0;
        for i in 0..len as usize {
            if clen == 0 {
                if i > 0 {
                    out.push_str("+ ");
                }
                clen = utf_ptr2len(line.add(i));
            }
            let b = *line.add(i);
            let byte_val: u8 = if b == NL { 0 } else { b as u8 };
            use std::fmt::Write as _;
            let _ = write!(out, "{byte_val:02x} ");
            clen -= 1;
            if out.len() > IOSIZE - 20 {
                break;
            }
        }

        let copy_len = out.len().min(IOSIZE - 1);
        let io_ptr = (&raw mut IObuff).cast::<c_char>();
        ptr::copy_nonoverlapping(out.as_ptr().cast::<c_char>(), io_ptr, copy_len);
        *io_ptr.add(copy_len) = NUL;
        msg(io_ptr, 0);
    }
}

#[inline]
const fn always_break(bc: u32) -> bool {
    bc == UTF8PROC_BOUNDCLASS_CONTROL
}

#[inline]
const fn always_break_two(bc1: u32, bc2: u32) -> bool {
    (bc1 != UTF8PROC_BOUNDCLASS_PREPEND && bc2 == UTF8PROC_BOUNDCLASS_OTHER)
        || (bc1 >= UTF8PROC_BOUNDCLASS_CR && bc1 <= UTF8PROC_BOUNDCLASS_CONTROL)
        || (bc2 == UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC
            && (bc1 == UTF8PROC_BOUNDCLASS_OTHER
                || bc1 == UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC))
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_head_off(base_in: *const c_char, p_in: *const c_char) -> c_int {
    unsafe {
        if *p_in.cast::<u8>() < 0x80 {
            return 0;
        }

        let base = base_in.cast::<u8>();
        let p = p_in.cast::<u8>();
        let mut start = p;

        while start > base && (*start & 0xc0) == 0x80 && (p.offset_from(start) as usize) < 6 {
            start = start.sub(1);
        }

        let last_len = utf8len_tab[*start as usize];
        let mut cur_code = utf_ptr2CharInfo_impl(start, usize::from(last_len));
        if cur_code < 0 || p.offset_from(start) >= isize::from(last_len) {
            return 0;
        }
        let safe_end = start.add(usize::from(last_len));

        let mut cur_bc = (*utf8proc_get_property(cur_code)).boundclass();
        if always_break(cur_bc) || start == base {
            return p.offset_from(start) as c_int;
        }

        let mut cur_pos = start;
        let p_start = start;

        loop {
            if *start.sub(1) == 0 {
                break;
            }
            start = start.sub(1);
            if *start < 0x80 {
                break;
            }

            while start > base
                && (*start & 0xc0) == 0x80
                && (cur_pos.offset_from(start) as usize) < 6
            {
                start = start.sub(1);
            }

            let prev_len = c_int::from(utf8len_tab[*start as usize]);
            let prev_code = utf_ptr2CharInfo_impl(start, prev_len as usize);
            if prev_code < 0 || (prev_len as isize) < cur_pos.offset_from(start) {
                start = cur_pos;
                break;
            }

            let prev_bc = (*utf8proc_get_property(prev_code)).boundclass();
            if always_break_two(prev_bc, cur_bc) && !arabic_combine(prev_code, cur_code) {
                start = cur_pos;
                break;
            } else if start == base {
                break;
            }
            cur_pos = start;
            cur_bc = prev_bc;
            cur_code = prev_code;
        }

        if start == p_start && isize::from(last_len) > p.offset_from(start) {
            return p.offset_from(start) as c_int;
        }

        let mut q = start;
        while q < p {
            let len = utfc_ptr2len_len(q.cast::<c_char>(), safe_end.offset_from(q) as c_int);
            if q.add(len as usize) > p {
                return p.offset_from(q) as c_int;
            }
            q = q.add(len as usize);
        }

        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_ClusterInfo_impl(
    base: StrCharInfo,
    max_len: *mut c_int,
) -> ClusterInfo {
    unsafe {
        let mut cells = basechar_cells_impl(base.chr);
        let mut prev_code = base.chr.value;
        let mut next = base.ptr.add(base.chr.len as usize).cast::<u8>();
        let mut state: GraphemeState = GRAPHEME_STATE_INIT;
        assert!(*next >= 0x80 && *max_len > 0);

        let mut check_emoji =
            cells == 1 && p_emoji != 0 && prop_is_emojilike(utf8proc_get_property(base.chr.value));

        loop {
            let next_len = utf8len_tab[*next as usize];
            if c_int::from(next_len) > *max_len {
                return ClusterInfo {
                    next: StrCharInfo {
                        ptr: next.cast::<c_char>(),
                        chr: CharInfo { value: -1, len: 1 },
                    },
                    cells,
                };
            }

            let next_code = utf_ptr2CharInfo_impl(next, usize::from(next_len));
            if !utf_iscomposing(prev_code, next_code, &raw mut state) {
                return ClusterInfo {
                    next: StrCharInfo {
                        ptr: next.cast::<c_char>(),
                        chr: CharInfo {
                            value: next_code,
                            len: if next_code < 0 {
                                1
                            } else {
                                c_int::from(next_len)
                            },
                        },
                    },
                    cells,
                };
            }

            if check_emoji {
                if next_code == 0xFE0F {
                    cells = 2;
                }
                check_emoji = false;
            }

            if cells == 1 {
                if (*utf8proc_get_property(next_code)).boundclass()
                    == UTF8PROC_BOUNDCLASS_SPACINGMARK
                    || (next_code & !1) == 0xFF9E
                {
                    if base.chr.len != 1 {
                        cells = 2;
                    }
                }
            }

            prev_code = next_code;
            next = next.add(usize::from(next_len));
            *max_len -= c_int::from(next_len);
            if *max_len > 0 && *next < 0x80 {
                return ClusterInfo {
                    next: StrCharInfo {
                        ptr: next.cast::<c_char>(),
                        chr: CharInfo {
                            value: i32::from(*next),
                            len: 1,
                        },
                    },
                    cells,
                };
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_eat_space(cc: c_int) -> bool {
    (0x2000..=0x206F).contains(&cc)
        || (0x2e00..=0x2e7f).contains(&cc)
        || (0x3000..=0x303f).contains(&cc)
        || (0xff01..=0xff0f).contains(&cc)
        || (0xff1a..=0xff20).contains(&cc)
        || (0xff3b..=0xff40).contains(&cc)
        || (0xff5b..=0xff65).contains(&cc)
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_allow_break_before(cc: c_int) -> bool {
    const BOL_PROHIBITION_PUNCT: &[c_int] = &[
        b'!' as c_int,
        b'%' as c_int,
        b')' as c_int,
        b',' as c_int,
        b':' as c_int,
        b';' as c_int,
        b'>' as c_int,
        b'?' as c_int,
        b']' as c_int,
        b'}' as c_int,
        0x2019,
        0x201d,
        0x2020,
        0x2021,
        0x2026,
        0x2030,
        0x2031,
        0x203c,
        0x2047,
        0x2048,
        0x2049,
        0x2103,
        0x2109,
        0x3001,
        0x3002,
        0x3009,
        0x300b,
        0x300d,
        0x300f,
        0x3011,
        0x3015,
        0x3017,
        0x3019,
        0x301b,
        0xff01,
        0xff09,
        0xff0c,
        0xff0e,
        0xff1a,
        0xff1b,
        0xff1f,
        0xff3d,
        0xff5d,
    ];
    BOL_PROHIBITION_PUNCT.binary_search(&cc).is_err()
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_allow_break_after(cc: c_int) -> bool {
    const EOL_PROHIBITION_PUNCT: &[c_int] = &[
        b'(' as c_int,
        b'<' as c_int,
        b'[' as c_int,
        b'`' as c_int,
        b'{' as c_int,
        0x2018,
        0x201c,
        0x3008,
        0x300a,
        0x300c,
        0x300e,
        0x3010,
        0x3014,
        0x3016,
        0x3018,
        0x301a,
        0xff08,
        0xff3b,
        0xff5b,
    ];
    EOL_PROHIBITION_PUNCT.binary_search(&cc).is_err()
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_allow_break(cc: c_int, ncc: c_int) -> bool {
    unsafe {
        if cc == ncc && (cc == 0x2014 || cc == 0x2026) {
            return false;
        }
        utf_allow_break_after(cc) && utf_allow_break_before(ncc)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_copy_char(fp: *mut *const c_char, tp: *mut *mut c_char) {
    unsafe {
        let l = utfc_ptr2len(*fp) as usize;
        ptr::copy(*fp, *tp, l);
        *tp = (*tp).add(l);
        *fp = (*fp).add(l);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_off_next(base: *const c_char, p: *const c_char) -> c_int {
    unsafe {
        let head_off = utf_head_off(base, p);
        if head_off == 0 {
            return 0;
        }
        utfc_ptr2len(p.sub(head_off as usize)) - head_off
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_cp_bounds_len(
    base: *const c_char,
    p_in: *const c_char,
    p_len: c_int,
) -> CharBoundsOff {
    unsafe {
        assert!(base <= p_in && p_len > 0);
        let b = base.cast::<u8>();
        let p = p_in.cast::<u8>();
        if *p < 0x80 {
            return CharBoundsOff {
                begin_off: 0,
                end_off: 1,
            };
        }

        let max_first_off = -((p.offset_from(b) as c_int).min(MB_MAXCHAR - 1));
        let mut first_off: c_int = 0;
        while utf_is_trail_byte(*p.offset(first_off as isize)) {
            if first_off == max_first_off {
                return CharBoundsOff {
                    begin_off: 0,
                    end_off: 1,
                };
            }
            first_off -= 1;
        }

        let lead = *p.offset(first_off as isize);
        let max_end_off = c_int::from(utf8len_tab[lead as usize]) + first_off;
        if max_end_off <= 0 || max_end_off > p_len {
            return CharBoundsOff {
                begin_off: 0,
                end_off: 1,
            };
        }

        for end_off in 1..max_end_off {
            if !utf_is_trail_byte(*p.offset(end_off as isize)) {
                return CharBoundsOff {
                    begin_off: 0,
                    end_off: 1,
                };
            }
        }

        CharBoundsOff {
            begin_off: (-first_off) as i8,
            end_off: max_end_off as i8,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_cp_bounds(base: *const c_char, p_in: *const c_char) -> CharBoundsOff {
    unsafe { utf_cp_bounds_len(base, p_in, c_int::MAX) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_find_illegal() {
    unsafe {
        let pos: pos_T = (*curwin).w_cursor;
        let mut vimconv = vimconv_T {
            vc_type: CONV_NONE,
            vc_factor: 1,
            vc_fd: ptr::null_mut(),
            vc_fail: false,
        };
        let mut tofree: *mut c_char = ptr::null_mut();

        if (enc_canon_props((*curbuf).b_p_fenc) & ENC_8BIT) != 0 {
            convert_setup(&raw mut vimconv, p_enc, (*curbuf).b_p_fenc);
        }

        (*curwin).w_cursor.coladd = 0;
        'search: loop {
            let mut p = get_cursor_pos_ptr();
            if vimconv.vc_type != CONV_NONE {
                xfree(tofree.cast());
                tofree = string_convert(&raw const vimconv, p, ptr::null_mut());
                if tofree.is_null() {
                    break;
                }
                p = tofree;
            }

            while *p != NUL {
                let mut len = utf_ptr2len(p);
                if *p.cast::<u8>() >= 0x80 && (len == 1 || utf_char2len(utf_ptr2char(p)) != len) {
                    if vimconv.vc_type == CONV_NONE {
                        (*curwin).w_cursor.col += p.offset_from(get_cursor_pos_ptr()) as colnr_T;
                    } else {
                        len = p.offset_from(tofree) as c_int;
                        let mut cur_p = get_cursor_pos_ptr();
                        while *cur_p != NUL && len > 0 {
                            len -= 1;
                            let l = utf_ptr2len(cur_p);
                            (*curwin).w_cursor.col += l;
                            cur_p = cur_p.add(l as usize);
                        }
                    }
                    (*curwin).w_set_curswant = true;
                    break 'search;
                }
                p = p.add(len as usize);
            }
            if (*curwin).w_cursor.lnum == (*curbuf).b_ml.ml_line_count {
                (*curwin).w_cursor = pos;
                beep_flush();
                break;
            }
            (*curwin).w_cursor.lnum += 1;
            (*curwin).w_cursor.col = 0;
        }

        if vimconv.vc_type != CONV_NONE && tofree.is_null() {
            (*curwin).w_cursor = pos;
            beep_flush();
        }

        xfree(tofree.cast());
        convert_setup(&raw mut vimconv, ptr::null_mut(), ptr::null_mut());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn utf_valid_string(s: *const c_char, end: *const c_char) -> bool {
    unsafe {
        let mut p = s.cast::<u8>();
        let end_u8 = end.cast::<u8>();

        while if end_u8.is_null() {
            *p != 0
        } else {
            p < end_u8
        } {
            let mut l = usize::from(utf8len_tab_zero[*p as usize]);
            if l == 0 {
                return false;
            }
            if !end_u8.is_null() && p.add(l) > end_u8 {
                return false;
            }
            p = p.add(1);
            l -= 1;
            while l > 0 {
                if (*p & 0xc0) != 0x80 {
                    return false;
                }
                p = p.add(1);
                l -= 1;
            }
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_adjust_cursor() {
    unsafe {
        mark_mb_adjustpos(curbuf.cast(), (&raw mut (*curwin).w_cursor).cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_check_adjust_col(win_: *mut c_void) {
    unsafe {
        let win = win_.cast::<win_T>();
        let oldcol = (*win).w_cursor.col;

        if oldcol != 0 {
            let p = ml_get_buf((*win).w_buffer, (*win).w_cursor.lnum);
            let len = CStr::from_ptr(p).to_bytes().len() as colnr_T;

            if len == 0 || oldcol < 0 {
                (*win).w_cursor.col = 0;
            } else {
                if oldcol > len {
                    (*win).w_cursor.col = len - 1;
                }
                (*win).w_cursor.col -= utf_head_off(p, p.add((*win).w_cursor.col as usize));
            }

            let cur_col = (*win).w_cursor.col as usize;
            if (*win).w_cursor.coladd == 1
                && *p.add(cur_col) != TAB
                && vim_isprintc(utf_ptr2char(p.add(cur_col)))
                && ptr2cells(p.add(cur_col)) > 1
            {
                (*win).w_cursor.coladd = 0;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_prevptr(line: *mut c_char, mut p: *mut c_char) -> *mut c_char {
    unsafe {
        if p > line {
            let off = utf_head_off(line, p.sub(1)) + 1;
            p = p.sub(off as usize);
        }
        p
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_charlen(str_: *const c_char) -> c_int {
    unsafe {
        if str_.is_null() {
            return 0;
        }
        let mut p = str_;
        let mut count = 0;
        while *p != NUL {
            p = p.add(utfc_ptr2len(p) as usize);
            count += 1;
        }
        count
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_charlen_len(str_: *const c_char, len: c_int) -> c_int {
    unsafe {
        let mut p = str_;
        let end = str_.add(len.max(0) as usize);
        let mut count = 0;
        while *p != NUL && p < end {
            p = p.add(utfc_ptr2len(p) as usize);
            count += 1;
        }
        count
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_unescape(pp: *mut *const c_char) -> *const c_char {
    unsafe {
        let mut buf_idx = 0usize;
        let str_ = (*pp).cast::<u8>();
        let mut str_idx = 0usize;

        while *str_.add(str_idx) != 0 && buf_idx < 4 {
            if *str_.add(str_idx) == K_SPECIAL
                && *str_.add(str_idx + 1) == KS_SPECIAL
                && *str_.add(str_idx + 2) == KE_FILLER
            {
                mb_unescape_buf[buf_idx] = K_SPECIAL as c_char;
                buf_idx += 1;
                str_idx += 2;
            } else if *str_.add(str_idx) == K_SPECIAL {
                break;
            } else {
                mb_unescape_buf[buf_idx] = *str_.add(str_idx) as c_char;
                buf_idx += 1;
            }
            mb_unescape_buf[buf_idx] = NUL;

            if utf_ptr2len((&raw const mb_unescape_buf).cast::<c_char>()) > 1 {
                *pp = str_.add(str_idx + 1).cast::<c_char>();
                return (&raw const mb_unescape_buf).cast::<c_char>();
            }

            if (mb_unescape_buf[0] as u8) < 128 {
                break;
            }
            str_idx += 1;
        }
        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn enc_skip(p: *mut c_char) -> *mut c_char {
    unsafe {
        let bytes = CStr::from_ptr(p).to_bytes();
        if bytes.starts_with(b"2byte-") {
            p.add(6)
        } else if bytes.starts_with(b"8bit-") {
            p.add(5)
        } else {
            p
        }
    }
}

unsafe fn enc_alias_search(name: *const c_char) -> c_int {
    let bytes = unsafe { CStr::from_ptr(name).to_bytes() };
    for entry in ENC_ALIAS_TABLE {
        if bytes == entry.name.to_bytes() {
            return entry.canon as c_int;
        }
    }
    -1
}

unsafe fn strmove_c(dst: *mut c_char, src: *const c_char) {
    unsafe {
        let len = CStr::from_ptr(src).to_bytes_with_nul().len();
        ptr::copy(src, dst, len);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn enc_canonize(enc: *mut c_char) -> *mut c_char {
    unsafe {
        let enc_bytes = CStr::from_ptr(enc).to_bytes();
        if enc_bytes == b"default" {
            return xstrdup(fenc_default);
        }

        let mut r = xmalloc(enc_bytes.len() + 3).cast::<c_char>();
        let mut p = r;
        for &b in enc_bytes {
            *p = if b == b'_' {
                b'-' as c_char
            } else {
                tolower_asc(c_int::from(b)) as u8 as c_char
            };
            p = p.add(1);
        }
        *p = NUL;

        p = enc_skip(r);

        let p_bytes = CStr::from_ptr(p).to_bytes();
        if p_bytes.starts_with(b"microsoft-cp") {
            strmove_c(p, p.add(10));
        }

        let p_bytes = CStr::from_ptr(p).to_bytes();
        if p_bytes.starts_with(b"iso8859") {
            strmove_c(p.add(4), p.add(3));
            *p.add(3) = b'-' as c_char;
        }

        let p_bytes = CStr::from_ptr(p).to_bytes();
        if p_bytes.starts_with(b"iso-8859") && p_bytes.len() > 8 && p_bytes[8].is_ascii_digit() {
            strmove_c(p.add(9), p.add(8));
            *p.add(8) = b'-' as c_char;
        }

        let p_bytes = CStr::from_ptr(p).to_bytes();
        if p_bytes.starts_with(b"latin-") {
            strmove_c(p.add(5), p.add(6));
        }

        if enc_canon_search(p) >= 0 {
            if p != r {
                strmove_c(r, p);
            }
        } else {
            let i = enc_alias_search(p);
            if i >= 0 {
                xfree(r.cast());
                r = xstrdup(ENC_CANON_TABLE[i as usize].name.as_ptr());
            }
        }
        r
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn enc_locale() -> *mut c_char {
    unsafe {
        let mut s: *const c_char = ptr::null();

        #[cfg(unix)]
        {
            s = nl_langinfo(CODESET);
        }

        if s.is_null() || *s == NUL {
            s = setlocale(LC_CTYPE, ptr::null());
            if s.is_null() || *s == NUL {
                s = os_getenv_noalloc(c"LC_ALL".as_ptr());
                if !s.is_null() {
                    s = os_getenv_noalloc(c"LC_CTYPE".as_ptr());
                    if !s.is_null() {
                        s = os_getenv_noalloc(c"LANG".as_ptr());
                    }
                }
            }
        }

        if s.is_null() {
            return ptr::null_mut();
        }

        let mut buf = [0 as c_char; 50];
        let p = vim_strchr(s, b'.' as c_int);
        if !p.is_null() {
            if p > s.add(2)
                && vim_strnicmp_asc(p.add(1), c"EUC".as_ptr(), 3) == 0
                && !(*p.add(4) as u8).is_ascii_alphanumeric()
                && *p.add(4) != b'-' as c_char
                && *p.sub(3) == b'_' as c_char
            {
                ptr::copy_nonoverlapping(b"euc-".as_ptr().cast::<c_char>(), buf.as_mut_ptr(), 4);
                let c2 = *p.sub(2) as u8;
                let c1 = *p.sub(1) as u8;
                buf[4] = if c2.is_ascii_alphanumeric() {
                    tolower_asc(c_int::from(c2)) as u8 as c_char
                } else {
                    0
                };
                buf[5] = if c1.is_ascii_alphanumeric() {
                    tolower_asc(c_int::from(c1)) as u8 as c_char
                } else {
                    0
                };
                buf[6] = NUL;
                return enc_canonize(buf.as_mut_ptr());
            }
            s = p.add(1);
        }

        let mut i = 0usize;
        while i < buf.len() - 1 && *s.add(i) != NUL {
            let ch = *s.add(i) as u8;
            if ch == b'_' || ch == b'-' {
                buf[i] = b'-' as c_char;
            } else if ch.is_ascii_alphanumeric() {
                buf[i] = tolower_asc(c_int::from(ch)) as u8 as c_char;
            } else {
                break;
            }
            i += 1;
        }
        buf[i] = NUL;

        enc_canonize(buf.as_mut_ptr())
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn my_iconv_open(to: *mut c_char, from: *mut c_char) -> *mut c_void {
    unsafe {
        const ICONV_TESTLEN: usize = 400;
        if iconv_working == WorkingStatus::kBroken {
            return usize::MAX as *mut c_void;
        }
        let mut fd = call_iconv_open(enc_skip(to), enc_skip(from));
        if fd != usize::MAX as iconv_t && iconv_working == WorkingStatus::kUnknown {
            let mut tobuf = [0 as c_char; ICONV_TESTLEN];
            let mut p = tobuf.as_mut_ptr();
            let mut tolen = ICONV_TESTLEN;
            call_iconv(
                fd,
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut p,
                &raw mut tolen,
            );
            if p.is_null() {
                iconv_working = WorkingStatus::kBroken;
                call_iconv_close(fd);
                fd = usize::MAX as iconv_t;
            } else {
                iconv_working = WorkingStatus::kWorking;
            }
        }
        fd
    }
}

unsafe fn iconv_string(
    vcp: *const vimconv_T,
    str_: *const c_char,
    slen: usize,
    unconvlenp: *mut usize,
    resultlenp: *mut usize,
) -> *mut c_char {
    unsafe {
        let mut to: *mut c_char;
        let mut len = 0usize;
        let mut done = 0usize;
        let mut result: *mut c_char = ptr::null_mut();

        let mut from = str_;
        let mut fromlen = slen;
        loop {
            if len == 0 || iconv_errno() == ICONV_E2BIG {
                len = len + fromlen * 2 + 40;
                let p = xmalloc(len).cast::<c_char>();
                if done > 0 {
                    ptr::copy_nonoverlapping(result, p, done);
                }
                xfree(result.cast());
                result = p;
            }

            to = result.add(done);
            let mut tolen = len - done - 2;
            if call_iconv(
                (*vcp).vc_fd,
                &raw mut from,
                &raw mut fromlen,
                &raw mut to,
                &raw mut tolen,
            ) != usize::MAX
            {
                *to = NUL;
                break;
            }

            let err = iconv_errno();
            if !(*vcp).vc_fail && !unconvlenp.is_null() && err == ICONV_EINVAL {
                *to = NUL;
                *unconvlenp = fromlen;
                break;
            } else if !(*vcp).vc_fail && (err == ICONV_EILSEQ || err == ICONV_EINVAL) {
                *to = b'?' as c_char;
                to = to.add(1);
                if utf_ptr2cells(from) > 1 {
                    *to = b'?' as c_char;
                    to = to.add(1);
                }
                let l = utfc_ptr2len_len(from, fromlen as c_int) as usize;
                from = from.add(l);
                fromlen -= l;
            } else if err != ICONV_E2BIG {
                xfree(result.cast());
                result = ptr::null_mut();
                break;
            }
            done = to.offset_from(result) as usize;
        }

        if !resultlenp.is_null() && !result.is_null() {
            *resultlenp = to.offset_from(result) as usize;
        }
        result
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_iconv(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let str_ = tv_get_string(argvars);
        let mut buf1 = [0 as c_char; NUMBUFLEN];
        let from = enc_canonize(enc_skip(
            tv_get_string_buf(argvars.add(1), buf1.as_mut_ptr()).cast_mut(),
        ));
        let mut buf2 = [0 as c_char; NUMBUFLEN];
        let to = enc_canonize(enc_skip(
            tv_get_string_buf(argvars.add(2), buf2.as_mut_ptr()).cast_mut(),
        ));
        let mut vimconv = vimconv_T {
            vc_type: CONV_NONE,
            vc_factor: 1,
            vc_fd: ptr::null_mut(),
            vc_fail: false,
        };
        convert_setup(&raw mut vimconv, from, to);

        if vimconv.vc_type == CONV_NONE {
            (*rettv).vval.v_string = xstrdup(str_);
        } else {
            (*rettv).vval.v_string =
                string_convert(&raw const vimconv, str_.cast_mut(), ptr::null_mut());
        }

        convert_setup(&raw mut vimconv, ptr::null_mut(), ptr::null_mut());
        xfree(from.cast());
        xfree(to.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn convert_setup(
    vcp: *mut vimconv_T,
    from: *mut c_char,
    to: *mut c_char,
) -> c_int {
    unsafe { convert_setup_ext(vcp, from, true, to, true) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn convert_setup_ext(
    vcp: *mut vimconv_T,
    from: *mut c_char,
    from_unicode_is_utf8: bool,
    to: *mut c_char,
    to_unicode_is_utf8: bool,
) -> c_int {
    unsafe {
        if (*vcp).vc_type == CONV_ICONV && (*vcp).vc_fd != usize::MAX as iconv_t {
            call_iconv_close((*vcp).vc_fd);
        }
        *vcp = vimconv_T {
            vc_type: CONV_NONE,
            vc_factor: 1,
            vc_fd: ptr::null_mut(),
            vc_fail: false,
        };

        if from.is_null()
            || *from == NUL
            || to.is_null()
            || *to == NUL
            || CStr::from_ptr(from).to_bytes() == CStr::from_ptr(to).to_bytes()
        {
            return OK;
        }

        let from_prop = enc_canon_props(from);
        let to_prop = enc_canon_props(to);
        let from_is_utf8 = if from_unicode_is_utf8 {
            (from_prop & ENC_UNICODE) != 0
        } else {
            from_prop == ENC_UNICODE
        };
        let to_is_utf8 = if to_unicode_is_utf8 {
            (to_prop & ENC_UNICODE) != 0
        } else {
            to_prop == ENC_UNICODE
        };

        if (from_prop & ENC_LATIN1) != 0 && to_is_utf8 {
            (*vcp).vc_type = CONV_TO_UTF8;
            (*vcp).vc_factor = 2;
        } else if (from_prop & ENC_LATIN9) != 0 && to_is_utf8 {
            (*vcp).vc_type = CONV_9_TO_UTF8;
            (*vcp).vc_factor = 3;
        } else if from_is_utf8 && (to_prop & ENC_LATIN1) != 0 {
            (*vcp).vc_type = CONV_TO_LATIN1;
        } else if from_is_utf8 && (to_prop & ENC_LATIN9) != 0 {
            (*vcp).vc_type = CONV_TO_LATIN9;
        } else {
            let to_arg = if to_is_utf8 {
                c"utf-8".as_ptr().cast_mut()
            } else {
                to
            };
            let from_arg = if from_is_utf8 {
                c"utf-8".as_ptr().cast_mut()
            } else {
                from
            };
            (*vcp).vc_fd = my_iconv_open(to_arg, from_arg);
            if (*vcp).vc_fd != usize::MAX as iconv_t {
                (*vcp).vc_type = CONV_ICONV;
                (*vcp).vc_factor = 4;
            }
        }

        if (*vcp).vc_type == CONV_NONE {
            FAIL
        } else {
            OK
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn string_convert(
    vcp: *const vimconv_T,
    ptr: *mut c_char,
    lenp: *mut usize,
) -> *mut c_char {
    unsafe { string_convert_ext(vcp, ptr, lenp, ptr::null_mut()) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn string_convert_ext(
    vcp: *const vimconv_T,
    ptr: *mut c_char,
    lenp: *mut usize,
    unconvlenp: *mut usize,
) -> *mut c_char {
    unsafe {
        let len = if lenp.is_null() {
            CStr::from_ptr(ptr).to_bytes().len()
        } else {
            *lenp
        };
        if len == 0 {
            return xstrdup(c"".as_ptr());
        }

        let mut retval: *mut u8 = ptr::null_mut();
        match (*vcp).vc_type {
            CONV_TO_UTF8 => {
                retval = xmalloc(len * 2 + 1).cast::<u8>();
                let mut d = retval;
                for i in 0..len {
                    let c = *ptr.add(i).cast::<u8>();
                    if c < 0x80 {
                        *d = c;
                        d = d.add(1);
                    } else {
                        *d = 0xc0 + (c >> 6);
                        *d.add(1) = 0x80 + (c & 0x3f);
                        d = d.add(2);
                    }
                }
                *d = 0;
                if !lenp.is_null() {
                    *lenp = d.offset_from(retval) as usize;
                }
            }
            CONV_9_TO_UTF8 => {
                retval = xmalloc(len * 3 + 1).cast::<u8>();
                let mut d = retval;
                for i in 0..len {
                    let c = match *ptr.add(i).cast::<u8>() {
                        0xa4 => 0x20ac,
                        0xa6 => 0x0160,
                        0xa8 => 0x0161,
                        0xb4 => 0x017d,
                        0xb8 => 0x017e,
                        0xbc => 0x0152,
                        0xbd => 0x0153,
                        0xbe => 0x0178,
                        b => c_int::from(b),
                    };
                    let n = utf_char2bytes(c, d.cast::<c_char>());
                    d = d.add(n as usize);
                }
                *d = 0;
                if !lenp.is_null() {
                    *lenp = d.offset_from(retval) as usize;
                }
            }
            CONV_TO_LATIN1 | CONV_TO_LATIN9 => {
                retval = xmalloc(len + 1).cast::<u8>();
                let mut d = retval;
                let mut i = 0usize;
                while i < len {
                    let l = utf_ptr2len_len(ptr.add(i), (len - i) as c_int);
                    if l == 0 {
                        *d = 0;
                        d = d.add(1);
                    } else if l == 1 {
                        let b = *ptr.add(i).cast::<u8>();
                        let l_w = usize::from(utf8len_tab_zero[b as usize]);
                        if l_w == 0 {
                            xfree(retval.cast());
                            return ptr::null_mut();
                        }
                        if !unconvlenp.is_null() && l_w > len - i {
                            *unconvlenp = len - i;
                            break;
                        }
                        *d = b;
                        d = d.add(1);
                    } else {
                        let mut c = utf_ptr2char(ptr.add(i));
                        if (*vcp).vc_type == CONV_TO_LATIN9 {
                            c = match c {
                                0x20ac => 0xa4,
                                0x0160 => 0xa6,
                                0x0161 => 0xa8,
                                0x017d => 0xb4,
                                0x017e => 0xb8,
                                0x0152 => 0xbc,
                                0x0153 => 0xbd,
                                0x0178 => 0xbe,
                                0xa4 | 0xa6 | 0xa8 | 0xb4 | 0xb8 | 0xbc | 0xbd | 0xbe => 0x100,
                                other => other,
                            };
                        }
                        if !utf_iscomposing_legacy(c) {
                            if c < 0x100 {
                                *d = c as u8;
                                d = d.add(1);
                            } else if (*vcp).vc_fail {
                                xfree(retval.cast());
                                return ptr::null_mut();
                            } else {
                                *d = 0xbf;
                                d = d.add(1);
                                if utf_char2cells(c) > 1 {
                                    *d = b'?';
                                    d = d.add(1);
                                }
                            }
                        }
                        i += (l as usize) - 1;
                    }
                    i += 1;
                }
                *d = 0;
                if !lenp.is_null() {
                    *lenp = d.offset_from(retval) as usize;
                }
            }
            CONV_ICONV => {
                retval = iconv_string(vcp, ptr, len, unconvlenp, lenp).cast::<u8>();
            }
            _ => {}
        }

        retval.cast::<c_char>()
    }
}

unsafe fn cw_value(c: c_int) -> c_int {
    unsafe {
        if cw_table.is_null() {
            return 0;
        }
        let c64 = i64::from(c);
        if c64 < (*cw_table).first {
            return 0;
        }
        let mut bot: isize = 0;
        let mut top: isize = (cw_table_size as isize) - 1;
        while top >= bot {
            let mid = ((bot + top) / 2) as usize;
            let entry = *cw_table.add(mid);
            if entry.last < c64 {
                bot = (mid as isize) + 1;
            } else if entry.first > c64 {
                top = (mid as isize) - 1;
            } else {
                return c_int::from(entry.width);
            }
        }
        0
    }
}

unsafe fn first_list_number(l: *const list_T) -> varnumber_T {
    unsafe {
        let li = tv_list_first(l);
        (*li).li_tv.vval.v_number
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setcellwidths(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST || (*argvars).vval.v_list.is_null() {
            emsg((&raw const e_listreq).cast::<c_char>());
            return;
        }

        let l = (*argvars).vval.v_list;
        let mut table: *mut cw_interval_T = ptr::null_mut();
        let table_size = tv_list_len(l) as usize;
        if table_size != 0 {
            let mut ptrs: Vec<*const list_T> = Vec::with_capacity(table_size);
            let mut item: c_int = 0;
            let mut li: *const listitem_T = (*l).lv_first;
            while !li.is_null() {
                let li_tv = &(*li).li_tv;
                if li_tv.v_type != VarType::VAR_LIST || li_tv.vval.v_list.is_null() {
                    semsg(E_LIST_ITEM_NR_IS_NOT_LIST.as_ptr(), item);
                    return;
                }

                let li_l = li_tv.vval.v_list;
                ptrs.push(li_l);
                let mut lili: *const listitem_T = tv_list_first(li_l);
                let mut i = 0;
                let mut n1: varnumber_T = 0;
                while !lili.is_null() {
                    let lili_tv = &(*lili).li_tv;
                    if lili_tv.v_type != VarType::VAR_NUMBER {
                        break;
                    }
                    if i == 0 {
                        n1 = lili_tv.vval.v_number;
                        if n1 < 0x80 {
                            emsg(E_ONLY_VALUES_OF_0X80_AND_HIGHER_SUPPORTED.as_ptr());
                            return;
                        }
                    } else if i == 1 && lili_tv.vval.v_number < n1 {
                        semsg(E_LIST_ITEM_NR_RANGE_INVALID.as_ptr(), item);
                        return;
                    } else if i == 2 && (lili_tv.vval.v_number < 1 || lili_tv.vval.v_number > 2) {
                        semsg(E_LIST_ITEM_NR_CELL_WIDTH_INVALID.as_ptr(), item);
                        return;
                    }
                    lili = (*lili).li_next;
                    i += 1;
                }

                if i != 3 {
                    semsg(E_LIST_ITEM_NR_DOES_NOT_CONTAIN_3_NUMBERS.as_ptr(), item);
                    return;
                }

                item += 1;
                li = (*li).li_next;
            }

            ptrs.sort_by_key(|&a| first_list_number(a));

            table = xmalloc(size_of::<cw_interval_T>() * table_size).cast::<cw_interval_T>();
            for (idx, &li_l) in ptrs.iter().enumerate() {
                let lili0 = tv_list_first(li_l);
                let n1 = (*lili0).li_tv.vval.v_number;
                if idx > 0 && n1 <= (*table.add(idx - 1)).last {
                    semsg(E_OVERLAPPING_RANGES_FOR_NR.as_ptr(), n1 as usize);
                    xfree(table.cast());
                    return;
                }
                let lili1 = (*lili0).li_next;
                let lili2 = (*lili1).li_next;
                *table.add(idx) = cw_interval_T {
                    first: n1,
                    last: (*lili1).li_tv.vval.v_number,
                    width: (*lili2).li_tv.vval.v_number as c_char,
                };
            }
        }

        let cw_table_save = cw_table;
        let cw_table_size_save = cw_table_size;
        cw_table = table;
        cw_table_size = table_size;

        let error = check_chars_options();
        if !error.is_null() {
            emsg(error);
            cw_table = cw_table_save;
            cw_table_size = cw_table_size_save;
            xfree(table.cast());
            return;
        }

        xfree(cw_table_save.cast());
        changed_window_setting_all();
        redraw_all_later(UPD_NOT_VALID);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getcellwidths(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, cw_table_size as isize);
        for i in 0..cw_table_size {
            let entry_data = *cw_table.add(i);
            let entry = tv_list_alloc(3);
            tv_list_append_number(entry, entry_data.first);
            tv_list_append_number(entry, entry_data.last);
            tv_list_append_number(entry, varnumber_T::from(entry_data.width));
            tv_list_append_list((*rettv).vval.v_list, entry);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_charclass(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_string_arg(argvars, 0) == FAIL || (*argvars).vval.v_string.is_null() {
            return;
        }
        (*rettv).vval.v_number = varnumber_T::from(mb_get_class((*argvars).vval.v_string));
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_encoding_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if idx < 0 || (idx as usize) >= ENC_CANON_TABLE.len() {
        return ptr::null_mut();
    }
    ENC_CANON_TABLE[idx as usize].name.as_ptr().cast_mut()
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_strcmp_ic(ic: bool, s1: *const c_char, s2: *const c_char) -> c_int {
    unsafe {
        if ic {
            mb_stricmp(s1, s2)
        } else {
            let b1 = CStr::from_ptr(s1).to_bytes();
            let b2 = CStr::from_ptr(s2).to_bytes();
            match b1.cmp(b2) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_char_encoding_and_bounds_work() {
        unsafe {
            let mut buf = [0 as c_char; 8];
            let len = utf_char2bytes(0x20ac, buf.as_mut_ptr());
            assert_eq!(len, 3);
            assert_eq!(utf_ptr2len(buf.as_ptr()), 3);
            assert_eq!(utf_ptr2char(buf.as_ptr()), 0x20ac);
            assert!(utf_valid_string(buf.as_ptr(), ptr::null()));
            assert!(utf_printable(0x20ac));
            assert!(!utf_printable(0xd800));
            assert_eq!(enc_canon_props(c"utf-8".as_ptr()), ENC_UNICODE);
        }
    }
}
