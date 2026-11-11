//! Multibyte UTF-8 character and encoding handling (`src/nvim/mbyte.c`).
#![forbid(unsafe_code)]

use crate::arabic::arabic_combine;
use crate::charset::{
    ascii_iswhite, ascii2cells, tolower_asc, toupper_asc, vim_isprintc, vim_iswordc_tab,
};
use crate::cursor::{Colnr, CursorPos, get_cursor_pos_ptr};
use crate::drawscreen::{UPD_NOT_VALID, redraw_all_later};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TypVal, VarType, Varnumber, tv_check_for_string_arg, tv_get_string,
    tv_get_string_buf, tv_list_alloc, tv_list_alloc_ret, tv_list_append_list,
    tv_list_append_number, tv_list_first, tv_list_len,
};
use crate::fileio::{call_iconv, call_iconv_close, call_iconv_open};
use crate::globals::*;
use crate::grid::CharBoundsOff;
use crate::input::beep_flush;
use crate::mark::mark_mb_adjustpos;
use crate::message::{emsg, msg, semsg};
use crate::r#move::changed_window_setting_all;
use crate::option_vars::*;
use crate::optionstr::check_chars_options;
use crate::os::env::ExpandCtx;
use crate::os::fs::os_errno;
use crate::os::lang::{os_locale_codeset, os_towlower, os_towupper};
#[cfg(windows)]
pub use crate::os::lang::{utf8_to_utf16, utf16_to_utf8};
use crate::utf8proc::{
    UTF8PROC_BOUNDCLASS_CONTROL, UTF8PROC_BOUNDCLASS_CR, UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    UTF8PROC_BOUNDCLASS_OTHER, UTF8PROC_BOUNDCLASS_PREPEND, UTF8PROC_BOUNDCLASS_REGIONAL_INDICATOR,
    UTF8PROC_BOUNDCLASS_SPACINGMARK, UTF8PROC_CASEFOLD, UTF8PROC_CATEGORY_LU, UTF8PROC_CATEGORY_ME,
    UTF8PROC_CATEGORY_MN, UTF8PROC_CATEGORY_NO, utf8proc_decompose_char, utf8proc_get_property,
    utf8proc_grapheme_break, utf8proc_grapheme_break_stateful, utf8proc_property_t,
    utf8proc_tolower, utf8proc_toupper,
};
use crate::window::WinInfoInfo;
use std::ffi::CStr;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharInfo {
    pub value: i32,
    pub len: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StrCharInfo<'a> {
    pub ptr: &'a [u8],
    pub chr: CharInfo,
}

impl<'a> StrCharInfo<'a> {
    #[inline]
    pub const fn cstr(&self) -> &'a [u8] {
        self.ptr
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.ptr.is_empty()
    }

    #[inline]
    pub const fn first_byte(&self) -> u8 {
        match self.ptr.first() {
            Some(&b) => b,
            None => 0,
        }
    }

    #[inline]
    pub const fn offset_from(&self, base: &CStr) -> usize {
        base.to_bytes().len().saturating_sub(self.ptr.len())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClusterInfo<'a> {
    pub next: StrCharInfo<'a>,
    pub cells: i32,
    pub first_compose: bool,
}

pub type GraphemeState = i32;

const MB_MAXCHAR: i32 = 6;

const ENC_8BIT: i32 = 0x01;
const ENC_DBCS: i32 = 0x02;
const ENC_ENDIAN_B: i32 = 0x10;
const ENC_LATIN9: i32 = 0x400;
const ENC_MACROMAN: i32 = 0x800;

const CONV_TO_UTF8: i32 = 1;
const CONV_9_TO_UTF8: i32 = 2;
const CONV_TO_LATIN1: i32 = 3;
const CONV_TO_LATIN9: i32 = 4;
const CONV_ICONV: i32 = 5;

const DBCS_JPN: i32 = 932;
const DBCS_JPNU: i32 = 9932;
const DBCS_KOR: i32 = 949;
const DBCS_KORU: i32 = 9949;
const DBCS_CHS: i32 = 936;
const DBCS_CHSU: i32 = 9936;
const DBCS_CHT: i32 = 950;
const DBCS_CHTU: i32 = 9950;
const DBCS_DEBUG: i32 = -1;

pub(crate) const K_OPT_CMP_FLAG_INTERNAL: u32 = 0x01;
pub(crate) const K_OPT_CMP_FLAG_KEEPASCII: u32 = 0x02;

const ICONV_E2BIG: i32 = 7;
#[cfg(windows)]
const ICONV_EILSEQ: i32 = 42;
#[cfg(target_os = "linux")]
const ICONV_EILSEQ: i32 = 84;
#[cfg(all(not(windows), not(target_os = "linux")))]
const ICONV_EILSEQ: i32 = 92;

#[cfg(windows)]
pub(crate) const CP_UTF8: u32 = 65001;

#[repr(C)]
#[derive(Debug)]
pub struct Vimconv {
    pub vc_type: i32,
    pub vc_factor: i32,
    pub vc_fd: Option<usize>,
    pub vc_fail: bool,
}

impl Vimconv {
    pub const fn new() -> Self {
        Self {
            vc_type: 0,
            vc_factor: 0,
            vc_fd: None,
            vc_fail: false,
        }
    }
}

impl Default for Vimconv {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Vimconv {
    fn drop(&mut self) {
        if self.vc_type == CONV_ICONV
            && let Some(fd) = self.vc_fd.take()
        {
            call_iconv_close(fd as isize);
        }
        self.vc_type = CONV_NONE;
        self.vc_fd = None;
    }
}

#[derive(Clone, Copy)]
struct Interval {
    first: i32,
    last: i32,
}

#[derive(Clone, Copy)]
struct ClInterval {
    first: u32,
    last: u32,
    cls: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CwInterval {
    first: i64,
    last: i64,
    width: i8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkingStatus {
    KUnknown = 0,
    Working = 1,
    KBroken = 2,
}

#[repr(C)]
struct EncCanonEntry {
    name: &'static CStr,
    prop: i32,
    codepage: i32,
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

pub static UTF8LEN_TAB: [u8; 256] = [
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

pub static UTF8LEN_TAB_ZERO: [u8; 256] = [
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
const IDX_UTF8: usize = 16;
const IDX_UCS2: usize = 17;
const IDX_UCS2LE: usize = 18;
const IDX_UTF16: usize = 19;
const IDX_UTF16BE: usize = 20;
const IDX_UTF16LE: usize = 21;
const IDX_UCS4: usize = 22;
const IDX_UCS4LE: usize = 23;
const IDX_EUC_JP: usize = 25;
const IDX_SJIS: usize = 26;
const IDX_EUC_KR: usize = 27;
const IDX_EUC_CN: usize = 28;
const IDX_EUC_TW: usize = 29;
const IDX_BIG5: usize = 30;
const IDX_CP932: usize = 46;
const IDX_CP936: usize = 47;
const IDX_CP949: usize = 48;
const IDX_CP950: usize = 49;
const IDX_MACROMAN: usize = 58;
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

struct MbyteModuleState {
    cw_table: std::cell::RefCell<Vec<CwInterval>>,
    iconv_working: std::cell::Cell<WorkingStatus>,
}

impl MbyteModuleState {
    const fn new() -> Self {
        Self {
            cw_table: std::cell::RefCell::new(Vec::new()),
            iconv_working: std::cell::Cell::new(WorkingStatus::KUnknown),
        }
    }
}

std::thread_local! {
    static MBYTE_STATE: MbyteModuleState = const { MbyteModuleState::new() };
}

#[inline]
fn mbyte_st<R>(f: impl FnOnce(&MbyteModuleState) -> R) -> R {
    MBYTE_STATE.with(f)
}

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::change::TAB;
use crate::charset::NL;
use crate::fileio::{
    ENC_2BYTE, ENC_2WORD, ENC_4BYTE, ENC_ENDIAN_L, ENC_LATIN1, ENC_UNICODE, ICONV_EINVAL,
};
use crate::globals::IOSIZE;
use crate::keycodes::{K_SPECIAL, KE_FILLER, KS_SPECIAL};
use crate::regexp::{GRAPHEME_STATE_INIT, NUMBUFLEN};
use crate::viml::parser::parser::CONV_NONE;

#[inline]
const fn utf_is_trail_byte(byte: u8) -> bool {
    (byte & 0xC0) == 0x80
}

#[inline]
pub fn basechar_cells_impl(c: CharInfo) -> i32 {
    if c.value < 0 {
        4
    } else if c.value < 0x80 {
        ascii2cells(c.value)
    } else {
        utf_char2cells(c.value)
    }
}

#[inline]
pub fn mb_idx_back(bytes: &[u8], idx: &mut usize) {
    if *idx > 0 {
        let prev = *idx - 1;
        let step = 1 + utf_head_off_slice(bytes, prev) as usize;
        *idx = idx.saturating_sub(step);
    }
}

pub trait AsMbyteSlice {
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R;
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R;
}

impl AsMbyteSlice for &[u8] {
    #[inline]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self)
    }
    #[inline]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self)
    }
}

impl AsMbyteSlice for &mut [u8] {
    #[inline]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self)
    }
    #[inline]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self)
    }
}

impl<const N: usize> AsMbyteSlice for &[u8; N] {
    #[inline]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(*self)
    }
    #[inline]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(*self)
    }
}

impl<const N: usize> AsMbyteSlice for &mut [u8; N] {
    #[inline]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(*self)
    }
    #[inline]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(*self)
    }
}

impl AsMbyteSlice for &CStr {
    #[inline]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.to_bytes())
    }
    #[inline]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.to_bytes())
    }
}

impl AsMbyteSlice for crate::strings::OwnedSlice {
    #[inline]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
    #[inline]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
}

impl AsMbyteSlice for &crate::strings::OwnedSlice {
    #[inline]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
    #[inline]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
}

impl AsMbyteSlice for crate::regexp::RegLineSlice {
    #[inline(always)]
    fn with_char_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let b0 = self.first_byte();
        if b0 == 0 {
            f(&[])
        } else if b0 < 0x80 {
            f(&[b0])
        } else {
            self.with_bytes(|b| f(&b[..b.len().min(MB_MAXCHAR as usize)]))
        }
    }
    #[inline(always)]
    fn with_cluster_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let b0 = self.first_byte();
        if b0 == 0 {
            f(&[])
        } else if b0 < 0x80 && self.second_byte() < 0x80 {
            f(&[b0])
        } else {
            self.with_bytes(|b| f(&b[..b.len().min(64)]))
        }
    }
}

#[inline]
pub fn utf_ptr2_char_info(bytes: impl AsMbyteSlice) -> CharInfo {
    bytes.with_char_bytes(|bytes| {
        let slice = crate::memory::bytes_until_nul(&bytes[..bytes.len().min(MB_MAXCHAR as usize)]);
        if slice.is_empty() {
            return CharInfo { value: 0, len: 1 };
        }
        utf_slice2_char_info(slice)
    })
}

#[inline]
pub fn utf_ptr2_str_char_info(s: &CStr) -> StrCharInfo<'_> {
    let ptr = s.to_bytes();
    StrCharInfo {
        ptr,
        chr: utf_ptr2_char_info(ptr),
    }
}

#[inline]
pub fn utf_ptr2_char_info_len(bytes: &[u8], size: i32) -> CharInfo {
    assert!(size >= 0);
    if size == 0 {
        return CharInfo { value: -1, len: 1 };
    }
    let max_len = (size as usize).min(MB_MAXCHAR as usize).min(bytes.len());
    let slice = crate::memory::bytes_until_nul(&bytes[..max_len]);
    if slice.is_empty() {
        return CharInfo { value: 0, len: 1 };
    }
    utf_slice2_char_info(slice)
}

#[inline]
pub fn utf_slice2_char_info(bytes: &[u8]) -> CharInfo {
    let Some(&first) = bytes.first() else {
        return CharInfo { value: -1, len: 1 };
    };
    if first < 0x80 {
        CharInfo {
            value: i32::from(first),
            len: 1,
        }
    } else {
        let len = usize::from(UTF8LEN_TAB[first as usize]);
        let Some(chunk) = bytes.get(..len) else {
            return CharInfo { value: -1, len: 1 };
        };
        let code_point = utf_slice2_char_info_impl(chunk);
        CharInfo {
            value: code_point,
            len: if code_point < 0 { 1 } else { len as i32 },
        }
    }
}

#[inline]
pub fn utf_cluster_cells(bytes: &[u8]) -> i32 {
    if bytes.is_empty() {
        return opt_vars().dy_escape_width.get();
    }
    let ci = utf_ptr2_str_char_info_len(bytes, bytes.len() as i32);
    let mut scratch = bytes.len() as i32;
    utf_cluster_info_len(ci, &mut scratch).cells
}

#[inline]
pub fn utf_slice2char(bytes: &[u8]) -> i32 {
    let Some(&b0) = bytes.first() else {
        return 0;
    };
    if b0 < 0x80 {
        return i32::from(b0);
    }
    let len = usize::from(UTF8LEN_TAB[b0 as usize]);
    if len < 2 {
        return i32::from(b0);
    }
    let Some(s) = bytes.get(..len) else {
        return i32::from(b0);
    };
    if !s[1..].iter().all(|&b| (b & 0xC0) == 0x80) {
        return i32::from(b0);
    }
    let v0 = u32::from(s[0]);
    let v1 = u32::from(s[1]);
    match len {
        2 => ((v0 << 6) + v1 - ((0xC0 << 6) + 0x80)) as i32,
        3 => {
            let v2 = u32::from(s[2]);
            ((v0 << 12) + (v1 << 6) + v2 - ((0xE0 << 12) + (0x80 << 6) + 0x80)) as i32
        }
        4 => {
            let v2 = u32::from(s[2]);
            let v3 = u32::from(s[3]);
            ((v0 << 18) + (v1 << 12) + (v2 << 6) + v3
                - ((0xF0 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80)) as i32
        }
        5 => {
            let v2 = u32::from(s[2]);
            let v3 = u32::from(s[3]);
            let v4 = u32::from(s[4]);
            ((v0 << 24) + (v1 << 18) + (v2 << 12) + (v3 << 6) + v4
                - ((0xF8 << 24) + (0x80 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80))
                as i32
        }
        _ => {
            let v2 = u32::from(s[2]);
            let v3 = u32::from(s[3]);
            let v4 = u32::from(s[4]);
            let v5 = u32::from(s[5]);
            ((v0 << 30)
                .wrapping_add(v1 << 24)
                .wrapping_add(v2 << 18)
                .wrapping_add(v3 << 12)
                .wrapping_add(v4 << 6)
                .wrapping_add(v5)
                .wrapping_sub((0x80 << 24) + (0x80 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80))
                as i32
        }
    }
}

#[inline]
pub fn utf_slice2len(bytes: &[u8]) -> usize {
    let Some(&b0) = bytes.first() else {
        return 0;
    };
    if b0 == 0 {
        return 0;
    }
    let len = usize::from(UTF8LEN_TAB[b0 as usize]);
    if bytes
        .get(1..len)
        .is_some_and(|tail| tail.iter().all(|&b| (b & 0xc0) == 0x80))
    {
        len
    } else {
        1
    }
}

#[inline]
pub fn utf_ptr2_str_char_info_len(bytes: &[u8], size: i32) -> StrCharInfo<'_> {
    let n = (size.max(0) as usize).min(bytes.len());
    let ptr = &bytes[..n];
    StrCharInfo {
        ptr,
        chr: utf_ptr2_char_info_len(bytes, size),
    }
}

#[inline]
pub fn utf_cluster_info(cur: StrCharInfo<'_>) -> ClusterInfo<'_> {
    if cur.is_empty() {
        return ClusterInfo {
            next: cur,
            cells: opt_vars().dy_escape_width.get(),
            first_compose: false,
        };
    }
    let adv = (cur.chr.len.max(0) as usize).min(cur.ptr.len());
    let next = &cur.ptr[adv..];
    let next_b = next.first().copied().unwrap_or(0);
    if next_b < 0x80 {
        return ClusterInfo {
            next: StrCharInfo {
                ptr: next,
                chr: CharInfo {
                    value: i32::from(next_b),
                    len: 1,
                },
            },
            cells: basechar_cells_impl(cur.chr),
            first_compose: utf_iscomposing_first(cur.chr.value),
        };
    }
    let mut scratch = i32::MAX;
    utf_cluster_info_impl(cur, &mut scratch)
}

#[inline]
pub fn utf_cluster_info_len<'a>(cur: StrCharInfo<'a>, size: &mut i32) -> ClusterInfo<'a> {
    *size -= cur.chr.len;
    let adv = (cur.chr.len.max(0) as usize).min(cur.ptr.len());
    let nextptr = &cur.ptr[adv..];
    let nextval = if *size == 0 {
        -1
    } else {
        i32::from(nextptr.first().copied().unwrap_or(0))
    };
    if nextval < 0x80 {
        return ClusterInfo {
            next: StrCharInfo {
                ptr: nextptr,
                chr: CharInfo {
                    value: nextval,
                    len: 1,
                },
            },
            cells: basechar_cells_impl(cur.chr),
            first_compose: utf_iscomposing_first(cur.chr.value),
        };
    }
    utf_cluster_info_impl(cur, size)
}

fn enc_canon_search(bytes: &[u8]) -> i32 {
    for (i, entry) in ENC_CANON_TABLE.iter().enumerate() {
        if bytes == entry.name.to_bytes() {
            return i as i32;
        }
    }
    -1
}

pub fn enc_canon_props(bytes: &[u8]) -> i32 {
    let i = enc_canon_search(bytes);
    if i >= 0 {
        return ENC_CANON_TABLE[i as usize].prop;
    }
    if bytes.starts_with(b"2byte-") {
        ENC_DBCS
    } else if bytes.starts_with(b"8bit-") || bytes.starts_with(b"iso-8859-") {
        ENC_8BIT
    } else {
        0
    }
}

pub fn bomb_size() -> i32 {
    let mut n = 0;
    if curbuf_mut().b_p_bomb.get() && !curbuf_mut().b_p_bin.get() {
        let fenc = curbuf_mut().b_p_fenc.as_bytes();
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

pub fn remove_bom(buf: &mut [u8]) {
    let mut len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let mut i = 0usize;
    while i < len {
        if buf[i..len].starts_with(&[0xef, 0xbb, 0xbf]) {
            buf.copy_within(i + 3..len, i);
            len -= 3;
            buf[len] = 0;
        } else {
            i += 1;
        }
    }
}

pub fn mb_get_class(bytes: &[u8]) -> i32 {
    mb_get_class_tab(bytes, &curbuf_mut().b_chartab.get())
}

pub fn mb_get_class_tab(bytes: &[u8], chartab: &[u64; 4]) -> i32 {
    let b0 = bytes.first().copied().unwrap_or(0);
    if UTF8LEN_TAB[b0 as usize] == 1 {
        if b0 == 0 || ascii_iswhite(i32::from(b0)) {
            return 0;
        }
        if vim_iswordc_tab(i32::from(b0), chartab) {
            return 2;
        }
        return 1;
    }
    utf_class_tab(utf_slice2char(bytes), chartab)
}

#[inline]
fn prop_is_emojilike(prop: &utf8proc_property_t) -> bool {
    let bc = prop.boundclass();
    bc == UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC || bc == UTF8PROC_BOUNDCLASS_REGIONAL_INDICATOR
}

pub fn utf_char2cells(c: i32) -> i32 {
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
    if prop.charwidth() == 2 {
        return 2;
    }
    if !opt_vars().p_ambw.is_null()
        && opt_vars().p_ambw.first_byte() == b'd'
        && prop.ambiguous_width()
    {
        return 2;
    }

    if opt_vars().p_emoji.get()
        && c >= 0x1f000
        && !prop.ambiguous_width()
        && prop_is_emojilike(prop)
    {
        return 2;
    }

    1
}

#[inline]
pub fn slice2cells(s: &[u8]) -> i32 {
    utf_cluster_cells(s)
}

#[inline]
pub fn utf_slice2cells(s: &[u8]) -> i32 {
    if s.first().is_some_and(|&b| b < 0x80 && b != 0) {
        return 1;
    }
    slice2cells(s)
}

#[inline]
pub fn mb_slice2char_adv(s: &mut &[u8]) -> i32 {
    let c = utf_slice2char(s);
    let len = utfc_slice2len(s).max(1).min(s.len());
    *s = &s[len..];
    c
}

#[inline]
pub fn mb_cslice2char_adv(s: &mut &[u8]) -> i32 {
    let c = utf_slice2char(s);
    let len = utf_slice2len(s).max(1).min(s.len());
    *s = &s[len..];
    c
}

pub fn ptr2cells(bytes: impl AsMbyteSlice) -> i32 {
    bytes.with_cluster_bytes(|bytes| slice2cells(crate::memory::bytes_until_nul(bytes)))
}

pub fn utf_ptr2cells(bytes: impl AsMbyteSlice) -> i32 {
    bytes.with_cluster_bytes(|bytes| utf_slice2cells(crate::memory::bytes_until_nul(bytes)))
}

pub fn utf_slice2_char_info_impl(bytes: &[u8]) -> i32 {
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

    let len = bytes.len();
    let Some(corr) = CORRECTIONS.get(len).copied() else {
        return -1;
    };
    let Some((&b0, tail)) = bytes.split_first() else {
        return -1;
    };
    if len < 2 {
        return -1;
    }
    let mut code_point = u32::from(b0);
    for &cur in tail {
        if (cur & 0xC0) != 0x80 {
            return -1;
        }
        code_point = (code_point << 6).wrapping_add(u32::from(cur));
    }
    code_point.wrapping_add(corr) as i32
}

pub fn mb_string2cells(bytes: &[u8]) -> usize {
    let bytes = match bytes.iter().position(|&b| b == 0) {
        Some(pos) => &bytes[..pos],
        None => bytes,
    };
    let mut clen = 0usize;
    let mut len = bytes.len() as i32;
    if len <= 0 {
        return 0;
    }
    let mut ci = utf_ptr2_str_char_info_len(bytes, len);
    while len > 0 {
        let cli = utf_cluster_info_len(ci, &mut len);
        clen += if ci.chr.value < 0x80 {
            1
        } else {
            cli.cells as usize
        };
        ci = cli.next;
    }
    clen
}

pub fn mb_string2cells_len(bytes: &[u8]) -> usize {
    let mut clen = 0usize;
    let mut len = bytes.len() as i32;
    if len <= 0 {
        return 0;
    }
    let mut ci = utf_ptr2_str_char_info_len(bytes, len);
    while len > 0 {
        let cli = utf_cluster_info_len(ci, &mut len);
        clen += cli.cells as usize;
        ci = cli.next;
    }
    clen
}

pub fn utf_ptr2char(bytes: impl AsMbyteSlice) -> i32 {
    bytes.with_char_bytes(|bytes| {
        let slice = crate::memory::bytes_until_nul(&bytes[..bytes.len().min(MB_MAXCHAR as usize)]);
        utf_slice2char(slice)
    })
}

fn utf_safe_read_char_adv(s: &mut &[u8]) -> i32 {
    let Some(&b0) = s.first() else {
        return 0;
    };
    if b0 == 0 {
        return 0;
    }

    let k = usize::from(UTF8LEN_TAB_ZERO[b0 as usize]);
    if k == 1 {
        *s = &s[1..];
        return i32::from(b0);
    }

    if k > 0 && k <= s.len() {
        let c = utf_slice2char(s);
        if c != i32::from(b0) || (c == 0xC3 && s.get(1) == Some(&0x83)) {
            *s = &s[k..];
            return c;
        }
    }

    -1
}

pub fn mb_ptr2char_adv(s: &mut &[u8]) -> i32 {
    mb_slice2char_adv(s)
}

pub fn mb_cptr2char_adv(s: &mut &[u8]) -> i32 {
    mb_cslice2char_adv(s)
}

#[inline]
pub fn utf_iscomposing_first(c: i32) -> bool {
    c >= 128 && !utf8proc_grapheme_break(b' ' as i32, c)
}

pub fn utf_composinglike_slice(p1: &[u8], p2: &[u8], state: Option<&mut GraphemeState>) -> bool {
    if p2.first().is_none_or(|&b| b < 128) {
        return false;
    }
    let first = utf_slice2char(p1);
    let second = utf_slice2char(p2);
    if !utf8proc_grapheme_break_stateful(first, second, state) {
        return true;
    }
    arabic_combine(first, second)
}

pub fn utf_composinglike(p1: &[u8], p2: &[u8], state: Option<&mut GraphemeState>) -> bool {
    let b1 = crate::memory::bytes_until_nul(&p1[..p1.len().min(MB_MAXCHAR as usize)]);
    let b2 = crate::memory::bytes_until_nul(&p2[..p2.len().min(MB_MAXCHAR as usize)]);
    utf_composinglike_slice(b1, b2, state)
}

pub fn utf_iscomposing(c1: i32, c2: i32, state: &mut GraphemeState) -> bool {
    !utf8proc_grapheme_break_stateful(c1, c2, Some(state)) || arabic_combine(c1, c2)
}

#[inline(always)]
pub fn utf_ptr2len(bytes: impl AsMbyteSlice) -> i32 {
    bytes.with_char_bytes(|bytes| {
        if let Some(&b0) = bytes.first()
            && b0 != 0
            && b0 < 0x80
        {
            return 1;
        }
        let slice = crate::memory::bytes_until_nul(&bytes[..bytes.len().min(MB_MAXCHAR as usize)]);
        utf_slice2len(slice) as i32
    })
}

pub fn utf_byte2len(b: i32) -> i32 {
    i32::from(UTF8LEN_TAB[(b as u8) as usize])
}

#[inline]
fn utf_slice2len_incomplete(bytes: &[u8]) -> usize {
    let Some(&b0) = bytes.first() else {
        return 1;
    };
    let len = usize::from(UTF8LEN_TAB[b0 as usize]);
    if len == 1 {
        return 1;
    }
    let m = len.min(bytes.len());
    if bytes[1..m].iter().all(|&b| (b & 0xc0) == 0x80) {
        len
    } else {
        1
    }
}

pub fn utf_ptr2len_len(bytes: &[u8], size: i32) -> i32 {
    let Some(&b0) = bytes.first() else {
        return 1;
    };
    let len = usize::from(UTF8LEN_TAB[b0 as usize]);
    if size <= 0 || len == 1 {
        return len as i32;
    }
    let m = len.min(size as usize).min(bytes.len());
    utf_slice2len_incomplete(&bytes[..m]) as i32
}

#[inline(always)]
pub fn utfc_ptr2len(bytes: impl AsMbyteSlice) -> i32 {
    bytes.with_cluster_bytes(|bytes| {
        if let Some(&b0) = bytes.first()
            && b0 != 0
            && b0 < 0x80
            && bytes.get(1).is_none_or(|&b1| b1 < 0x80)
        {
            return 1;
        }
        utfc_slice2len(crate::memory::bytes_until_nul(bytes)) as i32
    })
}

pub fn utfc_slice2len(bytes: &[u8]) -> usize {
    let Some(&b0) = bytes.first() else {
        return 0;
    };
    if b0 == 0 {
        return 0;
    }
    if b0 < 0x80 && bytes.get(1).is_none_or(|&b1| b1 < 0x80) {
        return 1;
    }

    let mut len = utf_slice2len_incomplete(bytes);
    if (len == 1 && b0 >= 0x80) || len > bytes.len() {
        return 1;
    }

    let mut prevlen = 0usize;
    let mut state: GraphemeState = GRAPHEME_STATE_INIT;
    while len < bytes.len() {
        let next_bytes = &bytes[len..];
        if next_bytes[0] < 0x80 {
            break;
        }
        let len_next_char = utf_slice2len_incomplete(next_bytes);
        if len_next_char > next_bytes.len() {
            break;
        }
        if !utf_composinglike_slice(&bytes[prevlen..], next_bytes, Some(&mut state)) {
            break;
        }
        prevlen = len;
        len += len_next_char;
    }
    len
}

pub fn mb_copy_slice_char(src: &[u8], dst: &mut [u8]) -> usize {
    let l = utfc_slice2len(src).min(src.len());
    dst[..l].copy_from_slice(&src[..l]);
    l
}

pub fn utfc_ptr2len_len(bytes: &[u8], size: i32) -> i32 {
    if size < 1 || bytes.is_empty() {
        return 0;
    }
    let b0 = bytes[0];
    if b0 == 0 {
        return 0;
    }
    if b0 < 0x80 && (size == 1 || bytes.get(1).is_none_or(|&b1| b1 < 0x80)) {
        return 1;
    }
    let m = (size as usize).min(bytes.len());
    utfc_slice2len(&bytes[..m]) as i32
}

pub const fn utf_char2len(c: i32) -> i32 {
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

pub fn utf_char2slice(c: i32, buf: &mut [u8]) -> usize {
    let u = c as u32;
    if c < 0x80 {
        buf[0] = c as u8;
        1
    } else if c < 0x800 {
        buf[0] = (0xc0 + (u >> 6)) as u8;
        buf[1] = (0x80 + (u & 0x3f)) as u8;
        2
    } else if c < 0x10000 {
        buf[0] = (0xe0 + (u >> 12)) as u8;
        buf[1] = (0x80 + ((u >> 6) & 0x3f)) as u8;
        buf[2] = (0x80 + (u & 0x3f)) as u8;
        3
    } else if c < 0x200000 {
        buf[0] = (0xf0 + (u >> 18)) as u8;
        buf[1] = (0x80 + ((u >> 12) & 0x3f)) as u8;
        buf[2] = (0x80 + ((u >> 6) & 0x3f)) as u8;
        buf[3] = (0x80 + (u & 0x3f)) as u8;
        4
    } else if c < 0x4000000 {
        buf[0] = (0xf8 + (u >> 24)) as u8;
        buf[1] = (0x80 + ((u >> 18) & 0x3f)) as u8;
        buf[2] = (0x80 + ((u >> 12) & 0x3f)) as u8;
        buf[3] = (0x80 + ((u >> 6) & 0x3f)) as u8;
        buf[4] = (0x80 + (u & 0x3f)) as u8;
        5
    } else {
        buf[0] = (0xfc + (u >> 30)) as u8;
        buf[1] = (0x80 + ((u >> 24) & 0x3f)) as u8;
        buf[2] = (0x80 + ((u >> 18) & 0x3f)) as u8;
        buf[3] = (0x80 + ((u >> 12) & 0x3f)) as u8;
        buf[4] = (0x80 + ((u >> 6) & 0x3f)) as u8;
        buf[5] = (0x80 + (u & 0x3f)) as u8;
        6
    }
}

pub fn utf_char2bytes(c: i32, buf: &mut [u8]) -> i32 {
    utf_char2slice(c, buf) as i32
}

pub fn utf_iscomposing_legacy(c: i32) -> bool {
    let prop = utf8proc_get_property(c);
    prop.category == UTF8PROC_CATEGORY_MN || prop.category == UTF8PROC_CATEGORY_ME
}

pub fn utf_isalnum(c: i32) -> bool {
    let prop = utf8proc_get_property(c);
    prop.category >= UTF8PROC_CATEGORY_LU && prop.category <= UTF8PROC_CATEGORY_NO
}

fn intable(table: &[Interval], c: i32) -> bool {
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

pub fn utf_printable(c: i32) -> bool {
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

pub fn utf_class(c: i32) -> i32 {
    utf_class_tab(c, &curbuf_mut().b_chartab.get())
}

pub fn utf_class_tab(c: i32, chartab: &[u64; 4]) -> i32 {
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

    if c < 0x100 {
        if c == b' ' as i32 || c == b'\t' as i32 || c == 0 || c == 0xa0 {
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
            return CLASSES[mid].cls as i32;
        }
    }

    2
}

pub fn utf_ambiguous_width(bytes: &[u8]) -> bool {
    if bytes.len() < 2 || bytes[0] == 0 || bytes[1] == 0 {
        return false;
    }
    let c = utf_slice2char(bytes);
    let len = utf_slice2len(bytes);
    if c >= 0x80 {
        let prop = utf8proc_get_property(c);
        if prop.ambiguous_width() || prop_is_emojilike(prop) {
            return true;
        }
    }
    bytes.get(len..len + 3) == Some(&[0xef, 0xb8, 0x8f])
}

pub fn utf_fold(a: i32) -> i32 {
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
    let res = utf8proc_decompose_char(a, &mut result, UTF8PROC_CASEFOLD, None);
    if res == 1 { result[0] } else { a }
}

pub fn mb_toupper(a: i32) -> i32 {
    if a < 128 && (opt_vars().cmp_flags.get() & K_OPT_CMP_FLAG_KEEPASCII) != 0 {
        return toupper_asc(a);
    }
    if (opt_vars().cmp_flags.get() & K_OPT_CMP_FLAG_INTERNAL) == 0 {
        return os_towupper(a);
    }
    if a < 128 {
        return toupper_asc(a);
    }
    utf8proc_toupper(a)
}

pub fn mb_islower(a: i32) -> bool {
    mb_toupper(a) != a
}

pub fn mb_tolower(a: i32) -> i32 {
    if a < 128 && (opt_vars().cmp_flags.get() & K_OPT_CMP_FLAG_KEEPASCII) != 0 {
        return tolower_asc(a);
    }
    if (opt_vars().cmp_flags.get() & K_OPT_CMP_FLAG_INTERNAL) == 0 {
        return os_towlower(a);
    }
    if a < 128 {
        return tolower_asc(a);
    }
    utf8proc_tolower(a)
}

pub fn mb_isupper(a: i32) -> bool {
    mb_tolower(a) != a
}

#[cfg(unix)]
pub fn mb_isalpha(a: i32) -> bool {
    mb_islower(a) || mb_isupper(a)
}

pub fn utf_strnicmp_slice(mut b1: &[u8], mut b2: &[u8]) -> i32 {
    let mut c1: i32;
    let mut c2: i32;
    let mut buffer = [0u8; 6];

    loop {
        c1 = utf_safe_read_char_adv(&mut b1);
        c2 = utf_safe_read_char_adv(&mut b2);
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

    let (mut r1, mut r2) = if c1 != -1 && c2 == -1 {
        let n1 = utf_char2slice(utf_fold(c1), &mut buffer);
        (&buffer[..n1], b2)
    } else if c2 != -1 && c1 == -1 {
        let n2 = utf_char2slice(utf_fold(c2), &mut buffer);
        (b1, &buffer[..n2])
    } else {
        (b1, b2)
    };

    while let (Some(&x1), Some(&x2)) = (r1.first(), r2.first()) {
        if x1 == 0 || x2 == 0 {
            break;
        }
        let cdiff = i32::from(x1) - i32::from(x2);
        if cdiff != 0 {
            return cdiff;
        }
        r1 = &r1[1..];
        r2 = &r2[1..];
    }

    let r1_empty = r1.first().is_none_or(|&b| b == 0);
    let r2_empty = r2.first().is_none_or(|&b| b == 0);
    if r1_empty && r2_empty {
        return 0;
    }
    if r1_empty { -1 } else { 1 }
}

pub fn utf_strnicmp(s1: &[u8], s2: &[u8], n1: usize, n2: usize) -> i32 {
    let b1 = crate::memory::bytes_until_nul(&s1[..n1.min(s1.len())]);
    let b2 = crate::memory::bytes_until_nul(&s2[..n2.min(s2.len())]);
    utf_strnicmp_slice(b1, b2)
}

pub fn mb_utflen(bytes: &[u8], len: usize, codepoints: &mut usize, codeunits: &mut usize) {
    let target = len.min(bytes.len());
    let mut count = 0usize;
    let mut extra = 0usize;
    let mut i = 0usize;
    while i < target {
        let clen = utf_slice2len_incomplete(&bytes[i..target]);
        let c = if clen > 1 {
            utf_slice2char(&bytes[i..])
        } else {
            i32::from(bytes[i])
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

pub fn mb_utf_index_to_bytes(bytes: &[u8], index: usize, use_utf16_units: bool) -> isize {
    if index == 0 {
        return 0;
    }
    let len = bytes.len();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < len {
        let rest = &bytes[i..];
        let clen = utf_slice2len_incomplete(rest);
        let c = if clen > 1 {
            utf_slice2char(rest)
        } else {
            i32::from(rest[0])
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

pub fn mb_strnicmp(s1: &[u8], s2: &[u8], nn: usize) -> i32 {
    utf_strnicmp(s1, s2, nn, nn)
}

pub fn mb_stricmp(s1: &CStr, s2: &CStr) -> i32 {
    utf_strnicmp_slice(s1.to_bytes(), s2.to_bytes())
}

pub fn show_utf8() {
    let line = crate::cursor::get_cursor_pos_slice();
    let len = utfc_slice2len(line);
    if len == 0 {
        msg(c"NUL", 0);
        return;
    }

    let mut out = String::new();
    let mut clen = 0usize;
    for i in 0..len {
        if clen == 0 {
            if i > 0 {
                out.push_str("+ ");
            }
            clen = utf_slice2len(&line[i..]);
        }
        let b = line[i];
        let byte_val: u8 = if b == NL as u8 { 0 } else { b };
        use std::fmt::Write as _;
        let _ = write!(out, "{byte_val:02x} ");
        clen -= 1;
        if out.len() > IOSIZE - 20 {
            break;
        }
    }

    let copy_len = out.len().min(IOSIZE - 1);
    let msg_cstr = crate::memory::bytes_to_cstring(&out.as_bytes()[..copy_len]);
    msg(msg_cstr.as_c_str(), 0);
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

pub fn utf_head_off_slice(bytes: &[u8], idx: usize) -> i32 {
    if bytes.get(idx).is_none_or(|&b| b < 0x80) {
        return 0;
    }

    let mut start = idx;
    while start > 0 && (bytes[start] & 0xc0) == 0x80 && idx - start < 6 {
        start -= 1;
    }

    let last_len = usize::from(UTF8LEN_TAB[bytes[start] as usize]);
    let Some(last_chunk) = bytes.get(start..start + last_len) else {
        return 0;
    };
    let mut cur_code = utf_slice2_char_info_impl(last_chunk);
    if cur_code < 0 || idx - start >= last_len {
        return 0;
    }
    let safe_end = start + last_len;

    let mut cur_bc = utf8proc_get_property(cur_code).boundclass();
    if always_break(cur_bc) || start == 0 {
        return (idx - start) as i32;
    }

    let mut cur_pos = start;
    let p_start = start;

    loop {
        if bytes[start - 1] == 0 {
            break;
        }
        start -= 1;
        if bytes[start] < 0x80 {
            break;
        }

        while start > 0 && (bytes[start] & 0xc0) == 0x80 && cur_pos - start < 6 {
            start -= 1;
        }

        let prev_len = usize::from(UTF8LEN_TAB[bytes[start] as usize]);
        let prev_code = bytes
            .get(start..start + prev_len)
            .map_or(-1, utf_slice2_char_info_impl);
        if prev_code < 0 || prev_len < cur_pos - start {
            start = cur_pos;
            break;
        }

        let prev_bc = utf8proc_get_property(prev_code).boundclass();
        if always_break_two(prev_bc, cur_bc) && !arabic_combine(prev_code, cur_code) {
            start = cur_pos;
            break;
        } else if start == 0 {
            break;
        }
        cur_pos = start;
        cur_bc = prev_bc;
        cur_code = prev_code;
    }

    if start == p_start && last_len > idx - start {
        return (idx - start) as i32;
    }

    let mut q = start;
    while q < idx {
        let len = utfc_slice2len(&bytes[q..safe_end]);
        if q + len > idx {
            return (idx - q) as i32;
        }
        q += len;
    }

    0
}

pub fn utf_head_off(base: &[u8], idx: usize) -> i32 {
    utf_head_off_slice(base, idx)
}

pub fn utf_cluster_info_impl<'a>(base: StrCharInfo<'a>, max_len: &mut i32) -> ClusterInfo<'a> {
    let mut cells = basechar_cells_impl(base.chr);
    let first_compose = utf_iscomposing_first(base.chr.value);
    let mut prev_code = base.chr.value;
    let adv = (base.chr.len.max(0) as usize).min(base.ptr.len());
    let mut next = &base.ptr[adv..];
    let mut state: GraphemeState = GRAPHEME_STATE_INIT;
    assert!(*max_len > 0);
    let mut cluster_len = base.chr.len;

    let mut check_emoji = cells == 1
        && opt_vars().p_emoji.get()
        && prop_is_emojilike(utf8proc_get_property(base.chr.value));

    loop {
        if *max_len <= 0 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next,
                    chr: CharInfo { value: -1, len: 1 },
                },
                cells,
                first_compose,
            };
        }
        let max_chunk = (*max_len as usize).min(MB_MAXCHAR as usize).min(next.len());
        let chunk = crate::memory::bytes_until_nul(&next[..max_chunk]);
        let b0 = chunk.first().copied().unwrap_or(0);
        if b0 < 0x80 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next,
                    chr: CharInfo {
                        value: i32::from(b0),
                        len: 1,
                    },
                },
                cells,
                first_compose,
            };
        }

        let next_len = UTF8LEN_TAB[b0 as usize];
        if i32::from(next_len) > *max_len {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next,
                    chr: CharInfo { value: -1, len: 1 },
                },
                cells,
                first_compose,
            };
        }

        let next_code = if chunk.len() < usize::from(next_len) {
            -1
        } else {
            utf_slice2_char_info_impl(&chunk[..usize::from(next_len)])
        };
        if next_code < 0 || !utf_iscomposing(prev_code, next_code, &mut state) {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next,
                    chr: CharInfo {
                        value: next_code,
                        len: if next_code < 0 {
                            1
                        } else {
                            i32::from(next_len)
                        },
                    },
                },
                cells,
                first_compose,
            };
        }

        if check_emoji {
            if next_code == 0xFE0F {
                cells = 2;
            }
            check_emoji = false;
        }

        cluster_len += i32::from(next_len);
        if cells == 1
            && (utf8proc_get_property(next_code).boundclass() == UTF8PROC_BOUNDCLASS_SPACINGMARK
                || (next_code & !1) == 0xFF9E)
            && base.chr.len != 1
            && (cluster_len as usize) < MAX_SCHAR_SIZE - 1 - usize::from(first_compose)
        {
            cells = 2;
        }

        prev_code = next_code;
        next = &next[usize::from(next_len).min(next.len())..];
        *max_len -= i32::from(next_len);
    }
}

pub fn utf_eat_space(cc: i32) -> bool {
    (0x2000..=0x206F).contains(&cc)
        || (0x2e00..=0x2e7f).contains(&cc)
        || (0x3000..=0x303f).contains(&cc)
        || (0xff01..=0xff0f).contains(&cc)
        || (0xff1a..=0xff20).contains(&cc)
        || (0xff3b..=0xff40).contains(&cc)
        || (0xff5b..=0xff65).contains(&cc)
}

pub fn utf_allow_break_before(cc: i32) -> bool {
    const BOL_PROHIBITION_PUNCT: &[i32] = &[
        b'!' as i32,
        b'%' as i32,
        b')' as i32,
        b',' as i32,
        b':' as i32,
        b';' as i32,
        b'>' as i32,
        b'?' as i32,
        b']' as i32,
        b'}' as i32,
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

pub fn utf_allow_break_after(cc: i32) -> bool {
    const EOL_PROHIBITION_PUNCT: &[i32] = &[
        b'(' as i32,
        b'<' as i32,
        b'[' as i32,
        b'`' as i32,
        b'{' as i32,
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

pub fn utf_allow_break(cc: i32, ncc: i32) -> bool {
    if cc == ncc && (cc == 0x2014 || cc == 0x2026) {
        return false;
    }
    utf_allow_break_after(cc) && utf_allow_break_before(ncc)
}

pub fn mb_copy_char(fp: &mut &[u8], out: &mut Vec<u8>) {
    let l = utfc_slice2len(fp).min(fp.len());
    out.extend_from_slice(&fp[..l]);
    *fp = &fp[l..];
}

pub fn mb_off_next(base: &[u8], idx: usize) -> i32 {
    let head_off = utf_head_off_slice(base, idx);
    if head_off == 0 {
        return 0;
    }
    let start = idx - (head_off as usize);
    (utfc_slice2len(&base[start..]) as i32) - head_off
}

pub fn utf_cp_bounds(bytes: &[u8], idx: usize) -> CharBoundsOff {
    if bytes.get(idx).is_none_or(|&b| b < 0x80) {
        return CharBoundsOff {
            begin_off: 0,
            end_off: 1,
        };
    }

    let max_back = idx.min((MB_MAXCHAR - 1) as usize);
    let mut back = 0usize;
    while utf_is_trail_byte(bytes[idx - back]) {
        if back == max_back {
            return CharBoundsOff {
                begin_off: 0,
                end_off: 1,
            };
        }
        back += 1;
    }

    let lead_idx = idx - back;
    let cp_len = usize::from(UTF8LEN_TAB[bytes[lead_idx] as usize]);
    if cp_len <= back || lead_idx + cp_len > bytes.len() {
        return CharBoundsOff {
            begin_off: 0,
            end_off: 1,
        };
    }

    if !bytes[idx + 1..lead_idx + cp_len]
        .iter()
        .copied()
        .all(utf_is_trail_byte)
    {
        return CharBoundsOff {
            begin_off: 0,
            end_off: 1,
        };
    }

    CharBoundsOff {
        begin_off: back as i8,
        end_off: (cp_len - back) as i8,
    }
}

pub fn utf_find_illegal() {
    let pos: CursorPos = curwin_mut().w_cursor;
    let mut vimconv = Vimconv {
        vc_type: CONV_NONE,
        vc_factor: 1,
        vc_fd: None,
        vc_fail: false,
    };
    let mut tofree: Option<Vec<u8>> = None;

    if (enc_canon_props(curbuf_mut().b_p_fenc.as_bytes()) & ENC_8BIT) != 0 {
        convert_setup(
            &mut vimconv,
            Some(opt_vars().p_enc.as_cstr()),
            Some(curbuf_mut().b_p_fenc.as_cstr()),
        );
    }

    curwin_mut().w_cursor.coladd = 0;
    'search: loop {
        let cursor_slice = crate::cursor::get_cursor_pos_slice();
        let search_bytes: &[u8] = if vimconv.vc_type != CONV_NONE {
            tofree = string_convert(&vimconv, get_cursor_pos_ptr().to_bytes(), None);
            let Some(ref tf) = tofree else {
                break;
            };
            let nul_pos = tf.iter().position(|&b| b == 0).unwrap_or(tf.len());
            &tf[..nul_pos]
        } else {
            cursor_slice
        };

        let mut idx = 0usize;
        while idx < search_bytes.len() && search_bytes[idx] != 0 {
            let rest = &search_bytes[idx..];
            let len = utf_slice2len(rest);
            if rest[0] >= 0x80 && (len == 1 || utf_char2len(utf_slice2char(rest)) as usize != len) {
                if vimconv.vc_type == CONV_NONE {
                    curwin_mut().w_cursor.col += idx as Colnr;
                } else {
                    let mut rem = idx as i32;
                    let mut cur_idx = 0usize;
                    while cur_idx < cursor_slice.len() && cursor_slice[cur_idx] != 0 && rem > 0 {
                        rem -= 1;
                        let l = utf_slice2len(&cursor_slice[cur_idx..]);
                        curwin_mut().w_cursor.col += l as Colnr;
                        cur_idx += l;
                    }
                }
                curwin_mut().w_set_curswant.set(true);
                break 'search;
            }
            idx += len;
        }
        if curwin_mut().w_cursor.lnum == curbuf_mut().b_ml.ml_line_count.get() {
            curwin_mut().w_cursor = pos;
            beep_flush();
            break;
        }
        curwin_mut().w_cursor.lnum += 1;
        curwin_mut().w_cursor.col = 0;
    }

    if vimconv.vc_type != CONV_NONE && tofree.is_none() {
        curwin_mut().w_cursor = pos;
        beep_flush();
    }
}

pub fn utf_valid_string(bytes: &[u8]) -> bool {
    let mut i = 0usize;
    while i < bytes.len() {
        let l = usize::from(UTF8LEN_TAB_ZERO[bytes[i] as usize]);
        if l == 0 || i + l > bytes.len() {
            return false;
        }
        if !bytes[i + 1..i + l].iter().all(|&b| (b & 0xc0) == 0x80) {
            return false;
        }
        i += l;
    }
    true
}

pub fn mb_adjust_cursor() {
    mark_mb_adjustpos(curbuf_mut(), &mut curwin_mut().w_cursor);
}

pub fn mb_check_adjust_col(win: &mut WinInfoInfo) {
    let oldcol = win.w_cursor.col;

    if oldcol != 0 {
        let line = crate::memline::ml_get_buf_slice(win.buf_mut(), win.w_cursor.lnum);
        let len = line.len() as Colnr;

        if len == 0 || oldcol < 0 {
            win.w_cursor.col = 0;
        } else {
            if oldcol > len {
                win.w_cursor.col = len - 1;
            }
            win.w_cursor.col -= utf_head_off_slice(line, win.w_cursor.col as usize) as Colnr;
        }

        let cur_col = win.w_cursor.col as usize;
        if win.w_cursor.coladd == 1
            && let Some(rem) = line.get(cur_col..)
            && !rem.is_empty()
            && rem[0] != TAB
            && vim_isprintc(utf_slice2char(rem))
            && slice2cells(rem) > 1
        {
            win.w_cursor.coladd = 0;
        }
    }
}

pub fn mb_charlen(mut rem: &[u8]) -> i32 {
    let mut count = 0;
    while !rem.is_empty() {
        let l = utfc_slice2len(rem).max(1);
        rem = &rem[l..];
        count += 1;
    }
    count
}

pub fn mb_unescape(s: &mut &[u8]) -> Option<crate::strings::InlineCStr<6>> {
    let bytes = *s;
    let mut buf = crate::strings::InlineCStr::<6>::new();
    let mut buf_idx = 0usize;
    let mut str_idx = 0usize;

    while str_idx < bytes.len() && bytes[str_idx] != 0 && buf_idx < 4 {
        if bytes[str_idx] == K_SPECIAL
            && bytes.get(str_idx + 1) == Some(&KS_SPECIAL)
            && bytes.get(str_idx + 2) == Some(&KE_FILLER)
        {
            buf.0[buf_idx] = K_SPECIAL;
            buf_idx += 1;
            str_idx += 2;
        } else if bytes[str_idx] == K_SPECIAL {
            break;
        } else {
            buf.0[buf_idx] = bytes[str_idx];
            buf_idx += 1;
        }
        buf.0[buf_idx] = 0;

        if utf_slice2len(&buf.0[..buf_idx]) > 1 {
            *s = &bytes[str_idx + 1..];
            return Some(buf);
        }

        if buf.0[0] < 128 {
            break;
        }
        str_idx += 1;
    }
    None
}

pub fn enc_skip(p: &CStr) -> &CStr {
    let bytes = p.to_bytes_with_nul();
    let skipped = bytes
        .strip_prefix(b"2byte-")
        .or_else(|| bytes.strip_prefix(b"8bit-"))
        .unwrap_or(bytes);
    CStr::from_bytes_with_nul(skipped).unwrap_or(c"")
}

fn enc_alias_search(bytes: &[u8]) -> i32 {
    for entry in ENC_ALIAS_TABLE {
        if bytes == entry.name.to_bytes() {
            return entry.canon as i32;
        }
    }
    -1
}

pub fn enc_canonize(enc: &CStr) -> std::ffi::CString {
    let enc_bytes = enc.to_bytes();
    if enc_bytes == b"default" {
        return g_st().fenc_default.get().clone().unwrap_or_default();
    }

    let mut r_vec: Vec<u8> = enc_bytes
        .iter()
        .map(|&b| {
            if b == b'_' {
                b'-'
            } else {
                tolower_asc(i32::from(b)) as u8
            }
        })
        .collect();

    if r_vec.starts_with(b"2byte-") {
        r_vec.drain(..6);
    } else if r_vec.starts_with(b"8bit-") {
        r_vec.drain(..5);
    }

    if r_vec.starts_with(b"microsoft-cp") {
        r_vec.drain(..10);
    }

    if r_vec.starts_with(b"iso8859") {
        r_vec.insert(3, b'-');
    }

    if r_vec.starts_with(b"iso-8859") && r_vec.get(8).is_some_and(u8::is_ascii_digit) {
        r_vec.insert(8, b'-');
    }

    if r_vec.starts_with(b"latin-") {
        r_vec.remove(5);
    }

    let r_cstr = crate::memory::bytes_to_cstring(&r_vec);
    if enc_canon_search(r_cstr.as_bytes()) >= 0 {
        return r_cstr;
    }
    let i = enc_alias_search(r_cstr.as_bytes());
    if i >= 0 {
        return ENC_CANON_TABLE[i as usize].name.to_owned();
    }
    r_cstr
}

pub fn enc_locale() -> Option<std::ffi::CString> {
    let mut env_buf = [0u8; 64];
    let s = os_locale_codeset(&mut env_buf)?;

    let mut buf = [0u8; 50];
    let mut s_full = s.to_bytes();
    if let Some(dot_idx) = s_full.iter().position(|&b| b == b'.') {
        let p_bytes = &s_full[dot_idx..];
        if dot_idx > 2
            && p_bytes.len() >= 4
            && p_bytes[1..4].eq_ignore_ascii_case(b"EUC")
            && !p_bytes.get(4).is_some_and(u8::is_ascii_alphanumeric)
            && p_bytes.get(4) != Some(&b'-')
            && s_full[dot_idx - 3] == b'_'
        {
            buf[..4].copy_from_slice(b"euc-");
            let c2 = s_full[dot_idx - 2];
            let c1 = s_full[dot_idx - 1];
            buf[4] = if c2.is_ascii_alphanumeric() {
                tolower_asc(i32::from(c2)) as u8
            } else {
                0
            };
            buf[5] = if c1.is_ascii_alphanumeric() {
                tolower_asc(i32::from(c1)) as u8
            } else {
                0
            };
            buf[6] = 0;
            return Some(enc_canonize(crate::memory::bytes_as_cstr(&buf)));
        }
        s_full = &s_full[dot_idx + 1..];
    }

    let s_bytes = &s_full[..(buf.len() - 1).min(s_full.len())];
    let mut i = 0usize;
    while i < s_bytes.len() {
        let ch = s_bytes[i];
        if ch == b'_' || ch == b'-' {
            buf[i] = b'-';
        } else if ch.is_ascii_alphanumeric() {
            buf[i] = tolower_asc(i32::from(ch)) as u8;
        } else {
            break;
        }
        i += 1;
    }
    buf[i] = 0;

    Some(enc_canonize(crate::memory::bytes_as_cstr(&buf)))
}

pub fn my_iconv_open(to: &CStr, from: &CStr) -> Option<usize> {
    const ICONV_TESTLEN: usize = 400;
    if mbyte_st(|st| st.iconv_working.get()) == WorkingStatus::KBroken {
        return None;
    }
    let fd = call_iconv_open(enc_skip(to), enc_skip(from));
    if fd != -1 && mbyte_st(|st| st.iconv_working.get()) == WorkingStatus::KUnknown {
        let mut tobuf = [0u8; ICONV_TESTLEN];
        let mut to_rem: &mut [u8] = &mut tobuf;
        call_iconv(fd, None, &mut to_rem);
        mbyte_st(|st| st.iconv_working.set(WorkingStatus::Working));
    }
    (fd != -1).then_some(fd as usize)
}

fn iconv_string(
    vcp: &Vimconv,
    src: &[u8],
    mut unconvlenp: Option<&mut usize>,
    mut resultlenp: Option<&mut usize>,
) -> Option<Vec<u8>> {
    let mut len = 0usize;
    let mut done = 0usize;
    let mut result_vec: Vec<u8> = Vec::new();
    let fd = vcp.vc_fd.unwrap_or(usize::MAX) as isize;

    let mut from_rem = src;
    loop {
        if len == 0 || os_errno() == ICONV_E2BIG {
            len = len + from_rem.len() * 2 + 40;
            result_vec.resize(len, 0);
        }

        let avail = len - done - 2;
        let mut to_rem: &mut [u8] = &mut result_vec[done..done + avail];
        let ok = call_iconv(fd, Some(&mut from_rem), &mut to_rem) != usize::MAX;
        let written = avail - to_rem.len();
        done += written;
        if ok {
            result_vec[done] = 0;
            break;
        }

        let err = os_errno();
        if !vcp.vc_fail && unconvlenp.is_some() && err == ICONV_EINVAL {
            result_vec[done] = 0;
            *unconvlenp.as_deref_mut().unwrap() = from_rem.len();
            break;
        } else if !vcp.vc_fail && (err == ICONV_EILSEQ || err == ICONV_EINVAL) {
            result_vec[done] = b'?';
            done += 1;
            if utf_ptr2cells(from_rem) > 1 {
                result_vec[done] = b'?';
                done += 1;
            }
            let l = utfc_ptr2len_len(from_rem, from_rem.len() as i32) as usize;
            from_rem = &from_rem[l.min(from_rem.len())..];
        } else if err != ICONV_E2BIG {
            return None;
        }
    }

    result_vec.truncate(done);
    result_vec.push(0);
    if let Some(ref mut p) = resultlenp {
        **p = done;
    }
    Some(result_vec)
}

pub fn f_iconv(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    let str_ = tv_get_string(&argvars[0]);
    let mut buf1 = [0u8; NUMBUFLEN];
    let from = enc_canonize(enc_skip(tv_get_string_buf(&argvars[1], &mut buf1)));
    let mut buf2 = [0u8; NUMBUFLEN];
    let to = enc_canonize(enc_skip(tv_get_string_buf(&argvars[2], &mut buf2)));
    let mut vimconv = Vimconv {
        vc_type: CONV_NONE,
        vc_factor: 1,
        vc_fd: None,
        vc_fail: false,
    };
    convert_setup(&mut vimconv, Some(from.as_c_str()), Some(to.as_c_str()));

    if vimconv.vc_type == CONV_NONE {
        crate::eval::typval::tv_set_string(rettv, Some(&str_));
    } else if let Some(conv_bytes) = string_convert(&vimconv, str_.to_bytes(), None) {
        crate::eval::typval::tv_set_string(rettv, Some(crate::memory::bytes_as_cstr(&conv_bytes)));
    }
}

pub fn convert_setup(vcp: &mut Vimconv, from: Option<&CStr>, to: Option<&CStr>) -> bool {
    convert_setup_ext(vcp, from, true, to, true)
}

pub fn convert_setup_ext(
    vcp: &mut Vimconv,
    from_opt: Option<&CStr>,
    from_unicode_is_utf8: bool,
    to_opt: Option<&CStr>,
    to_unicode_is_utf8: bool,
) -> bool {
    *vcp = Vimconv {
        vc_type: CONV_NONE,
        vc_factor: 1,
        vc_fd: None,
        vc_fail: false,
    };

    let Some(from) = from_opt else {
        return true;
    };
    let Some(to) = to_opt else {
        return true;
    };
    if from.to_bytes().is_empty() || to.to_bytes().is_empty() || from.to_bytes() == to.to_bytes() {
        return true;
    }

    let from_prop = enc_canon_props(from.to_bytes());
    let to_prop = enc_canon_props(to.to_bytes());
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
        vcp.vc_type = CONV_TO_UTF8;
        vcp.vc_factor = 2;
    } else if (from_prop & ENC_LATIN9) != 0 && to_is_utf8 {
        vcp.vc_type = CONV_9_TO_UTF8;
        vcp.vc_factor = 3;
    } else if from_is_utf8 && (to_prop & ENC_LATIN1) != 0 {
        vcp.vc_type = CONV_TO_LATIN1;
    } else if from_is_utf8 && (to_prop & ENC_LATIN9) != 0 {
        vcp.vc_type = CONV_TO_LATIN9;
    } else {
        let to_arg = if to_is_utf8 { c"utf-8" } else { to };
        let from_arg = if from_is_utf8 { c"utf-8" } else { from };
        vcp.vc_fd = my_iconv_open(to_arg, from_arg);
        if vcp.vc_fd.is_some() {
            vcp.vc_type = CONV_ICONV;
            vcp.vc_factor = 4;
        }
    }

    vcp.vc_type != CONV_NONE
}

pub fn string_convert_slice(vcp: &Vimconv, bytes: &[u8], lenp: &mut usize) -> Option<Vec<u8>> {
    *lenp = bytes.len();
    string_convert(vcp, bytes, Some(lenp))
}

pub fn string_convert(vcp: &Vimconv, src: &[u8], lenp: Option<&mut usize>) -> Option<Vec<u8>> {
    string_convert_ext(vcp, src, lenp, None)
}

pub fn string_convert_ext(
    vcp: &Vimconv,
    src_in: &[u8],
    mut lenp: Option<&mut usize>,
    mut unconvlenp: Option<&mut usize>,
) -> Option<Vec<u8>> {
    let len = if lenp.is_none() {
        crate::memory::bytes_until_nul(src_in).len()
    } else {
        (*lenp.as_deref_mut().unwrap()).min(src_in.len())
    };
    if len == 0 {
        return Some(vec![0]);
    }
    let src = &src_in[..len];

    match vcp.vc_type {
        CONV_TO_UTF8 => {
            let mut out: Vec<u8> = Vec::with_capacity(len * 2 + 1);
            for &c in src {
                if c < 0x80 {
                    out.push(c);
                } else {
                    out.push(0xc0 + (c >> 6));
                    out.push(0x80 + (c & 0x3f));
                }
            }
            if let Some(ref mut p) = lenp {
                **p = out.len();
            }
            out.push(0);
            Some(out)
        }
        CONV_9_TO_UTF8 => {
            let mut out: Vec<u8> = Vec::with_capacity(len * 3 + 1);
            let mut ch_buf = [0u8; MB_MAXCHAR as usize];
            for &b in src {
                let c = match b {
                    0xa4 => 0x20ac,
                    0xa6 => 0x0160,
                    0xa8 => 0x0161,
                    0xb4 => 0x017d,
                    0xb8 => 0x017e,
                    0xbc => 0x0152,
                    0xbd => 0x0153,
                    0xbe => 0x0178,
                    b => i32::from(b),
                };
                let n = utf_char2slice(c, &mut ch_buf);
                out.extend_from_slice(&ch_buf[..n]);
            }
            if let Some(ref mut p) = lenp {
                **p = out.len();
            }
            out.push(0);
            Some(out)
        }
        CONV_TO_LATIN1 | CONV_TO_LATIN9 => {
            let mut out: Vec<u8> = Vec::with_capacity(len + 1);
            let mut i = 0usize;
            while i < len {
                let rest = &src[i..];
                let l = utf_slice2len_incomplete(rest) as i32;
                if l == 0 {
                    out.push(0);
                } else if l == 1 {
                    let b = rest[0];
                    let l_w = usize::from(UTF8LEN_TAB_ZERO[b as usize]);
                    if l_w == 0 {
                        return None;
                    }
                    if let Some(ref mut ulen) = unconvlenp
                        && l_w > len - i
                    {
                        **ulen = len - i;
                        break;
                    }
                    out.push(b);
                } else {
                    let mut c = utf_slice2char(rest);
                    if vcp.vc_type == CONV_TO_LATIN9 {
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
                            out.push(c as u8);
                        } else if vcp.vc_fail {
                            return None;
                        } else {
                            out.push(0xbf);
                            if utf_char2cells(c) > 1 {
                                out.push(b'?');
                            }
                        }
                    }
                    i += (l as usize) - 1;
                }
                i += 1;
            }
            if let Some(ref mut p) = lenp {
                **p = out.len();
            }
            out.push(0);
            Some(out)
        }
        CONV_ICONV => iconv_string(vcp, src, unconvlenp, lenp),
        _ => None,
    }
}

fn cw_value(c: i32) -> i32 {
    mbyte_st(|st| {
        let table = st.cw_table.borrow();
        let c64 = i64::from(c);
        table
            .binary_search_by(|entry| {
                if entry.last < c64 {
                    std::cmp::Ordering::Less
                } else if entry.first > c64 {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .map_or(0, |idx| i32::from(table[idx].width))
    })
}

pub fn f_setcellwidths(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type != VarType::VarList || argvars[0].vval.v_list().is_null() {
        emsg(crate::strings::gettext(E_LISTREQ));
        return;
    }

    let l = argvars[0].vval.v_list_ref();
    let table_size = tv_list_len(Some(l)) as usize;
    let mut table: Vec<CwInterval> = Vec::with_capacity(table_size);
    if table_size != 0 {
        for (item, li) in (0_i32..).zip(crate::eval::typval::list_iter_mut(l.first_opt())) {
            let li_tv = li.tv();
            if li_tv.v_type != VarType::VarList || li_tv.vval.v_list().is_null() {
                semsg!(E_LIST_ITEM_NR_IS_NOT_LIST, item);
                return;
            }

            let li_l = li_tv.vval.v_list_ref();
            let mut i = 0;
            let mut n1: Varnumber = 0;
            let mut n2: Varnumber = 0;
            let mut n3: Varnumber = 0;
            for lili in crate::eval::typval::list_iter_ptr(tv_list_first(Some(li_l))) {
                let lili_tv = lili.tv();
                if lili_tv.v_type != VarType::VarNumber {
                    break;
                }
                let val = lili_tv.vval.v_number();
                if i == 0 {
                    n1 = val;
                    if n1 < 0x80 {
                        emsg(E_ONLY_VALUES_OF_0X80_AND_HIGHER_SUPPORTED);
                        return;
                    }
                } else if i == 1 {
                    n2 = val;
                    if n2 < n1 {
                        semsg!(E_LIST_ITEM_NR_RANGE_INVALID, item);
                        return;
                    }
                } else if i == 2 {
                    n3 = val;
                    if !(1..=2).contains(&n3) {
                        semsg!(E_LIST_ITEM_NR_CELL_WIDTH_INVALID, item);
                        return;
                    }
                }
                i += 1;
            }

            if i != 3 {
                semsg!(E_LIST_ITEM_NR_DOES_NOT_CONTAIN_3_NUMBERS, item);
                return;
            }
            table.push(CwInterval {
                first: n1,
                last: n2,
                width: n3 as i8,
            });
        }

        table.sort_by_key(|entry| entry.first);

        for pair in table.windows(2) {
            if pair[1].first <= pair[0].last {
                semsg!(E_OVERLAPPING_RANGES_FOR_NR, pair[1].first as usize);
                return;
            }
        }
    }

    let cw_table_save = mbyte_st(|st| std::mem::replace(&mut *st.cw_table.borrow_mut(), table));

    if let Some(error) = check_chars_options() {
        emsg(error);
        mbyte_st(|st| *st.cw_table.borrow_mut() = cw_table_save);
        return;
    }

    changed_window_setting_all();
    redraw_all_later(UPD_NOT_VALID);
}

pub fn f_getcellwidths(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let table = mbyte_st(|st| st.cw_table.borrow().clone());
    tv_list_alloc_ret(rettv, table.len() as isize);
    for &entry_data in &table {
        let entry = tv_list_alloc(3);
        tv_list_append_number(&entry, entry_data.first);
        tv_list_append_number(&entry, entry_data.last);
        tv_list_append_number(&entry, Varnumber::from(entry_data.width));
        tv_list_append_list(rettv.vval.v_list_ref(), Some(&entry));
    }
}

pub fn f_charclass(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if !tv_check_for_string_arg(argvars, 0) || argvars[0].vval.v_string_is_null() {
        return;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(mb_get_class(
        argvars[0].vval.v_string_bytes(),
    )));
}

pub fn get_encoding_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if idx < 0 || (idx as usize) >= ENC_CANON_TABLE.len() {
        return None;
    }
    Some(ENC_CANON_TABLE[idx as usize].name)
}

pub fn mb_strcmp_ic(ic: bool, s1: &CStr, s2: &CStr) -> i32 {
    if ic {
        mb_stricmp(s1, s2)
    } else {
        match s1.to_bytes().cmp(s2.to_bytes()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::schar_get;

    #[test]
    fn utf8_char_encoding_and_bounds_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let mut buf = [0u8; 8];
            let len = utf_char2bytes(0x20ac, &mut buf);
            assert_eq!(len, 3);
            assert_eq!(utf_ptr2len(&buf), 3);
            assert_eq!(utf_ptr2char(&buf), 0x20ac);
            assert_eq!(utf_slice2len(&buf[..3]), 3);
            assert_eq!(utf_slice2char(&buf[..3]), 0x20ac);
            // Truncated non-NUL-terminated multibyte slices must not overread or panic:
            assert_eq!(utf_slice2len(&[]), 0);
            assert_eq!(utf_slice2char(&[]), 0);
            assert_eq!(utf_slice2len(&[0xe2]), 1);
            assert_eq!(utf_slice2char(&[0xe2]), 0xe2);
            assert_eq!(utf_slice2len(&[0xe2, 0x82]), 1);
            assert_eq!(utf_slice2char(&[0xe2, 0x82]), 0xe2);
            assert_eq!(utfc_slice2len(&[0xe2, 0x82]), 1);
            assert_eq!(
                utf_slice2_char_info(&[0xe2, 0x82]),
                CharInfo { value: -1, len: 1 }
            );
            assert_eq!(
                utf_slice2_char_info(&buf[..3]),
                CharInfo {
                    value: 0x20ac,
                    len: 3
                }
            );
            assert!(utf_valid_string(&buf[..3]));
            assert!(utf_printable(0x20ac));
            assert!(!utf_printable(0xd800));
            assert_eq!(enc_canon_props(b"utf-8"), ENC_UNICODE);

            assert_eq!(utf_fold(0xddfb), 0xddfb);
            assert_eq!(utf_fold(0xd800), 0xd800);
            assert_eq!(utf_fold(9_000_000), 9_000_000);
            assert_eq!(utf_fold(0), 0);

            for (s, exp_cells) in [
                (c"ำ", 1),
                (c"ท", 1),
                (c"ทำ", 2),
                (c"का", 2),
                (c"ｶ", 1),
                (c"ｶﾞ", 2),
                (c"ﾊﾟ", 2),
                (c"ท่", 1),
                (c"é", 1),
                (c"ทำำ", 2),
                (c"漢", 2),
            ] {
                assert_eq!(utf_ptr2cells(s.to_bytes()), exp_cells);
                assert_eq!(utf_slice2cells(s.to_bytes()), exp_cells);
                assert_eq!(mb_string2cells_len(s.to_bytes()), exp_cells as usize);
            }
            assert_eq!(utf_ptr2cells(b"\xff"), 4);

            let mut firstc: i32 = 0;
            let mut sbuf = [0u8; 32];
            let sc = crate::grid::unittest_schar_from_cluster(c")\xcc\x80\xcc\x81", &mut firstc);
            schar_get(&mut sbuf, sc);
            assert_eq!(firstc, 0x29);
            assert_eq!(
                CStr::from_bytes_until_nul(&sbuf).unwrap().to_bytes(),
                b")\xcc\x80\xcc\x81"
            );

            let emoji = c"🏳️‍⚧️🧑‍🌾❤️😂🏴‍☠️";
            let e_bytes = emoji.to_bytes();
            let mut pos = 0usize;
            while pos < e_bytes.len() {
                let clen = utfc_ptr2len(&e_bytes[pos..]) as usize;
                for p in pos..(pos + clen) {
                    assert_eq!(utf_head_off(e_bytes, p), (p - pos) as i32);
                }
                pos += clen;
            }

            let cp_str = c"iÀⱠ𐀀i";
            let b0 = utf_cp_bounds(&cp_str.to_bytes()[..11], 2);
            assert_eq!((b0.begin_off, b0.end_off), (1, 1));
        }
    }
}
