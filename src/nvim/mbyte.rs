//! Multibyte UTF-8 character and encoding handling (`src/nvim/mbyte.c`).

use crate::arabic::arabic_combine;
use crate::charset::{
    ascii_iswhite, ascii2cells, tolower_asc, toupper_asc, vim_isprintc, vim_iswordc_tab,
};
use crate::cursor::{Colnr, CursorPos, get_cursor_pos_ptr};
use crate::drawscreen::{UPD_NOT_VALID, redraw_all_later};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, Listitem, TvList, TypVal, VarType, Varnumber, tv_check_for_string_arg,
    tv_get_string, tv_get_string_buf, tv_list_alloc, tv_list_alloc_ret, tv_list_append_list,
    tv_list_append_number, tv_list_first, tv_list_len,
};
#[cfg(windows)]
use crate::event::r#loop::uv_translate_sys_error;
use crate::fileio::{call_iconv, call_iconv_close, call_iconv_open};
use crate::globals::*;
use crate::grid::CharBoundsOff;
use crate::input::beep_flush;
use crate::mark::mark_mb_adjustpos;
use crate::memline::ml_get_buf;
use crate::message::{emsg, msg, semsg};
use crate::r#move::changed_window_setting_all;
use crate::option_vars::*;
use crate::optionstr::check_chars_options;
use crate::os::env::{ExpandCtx, os_getenv_buf};
use crate::os::fs::os_errno;
#[cfg(unix)]
use crate::os::lang::nl_langinfo;
use crate::os::lang::{LC_CTYPE, setlocale};
use crate::plines::{CharInfo, ClusterInfo, StrCharInfo};
use crate::strings::vim_strchr;
use crate::utf8proc::{
    UTF8PROC_BOUNDCLASS_CONTROL, UTF8PROC_BOUNDCLASS_CR, UTF8PROC_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    UTF8PROC_BOUNDCLASS_OTHER, UTF8PROC_BOUNDCLASS_PREPEND, UTF8PROC_BOUNDCLASS_REGIONAL_INDICATOR,
    UTF8PROC_BOUNDCLASS_SPACINGMARK, UTF8PROC_CASEFOLD, UTF8PROC_CATEGORY_LU, UTF8PROC_CATEGORY_ME,
    UTF8PROC_CATEGORY_MN, UTF8PROC_CATEGORY_NO, utf8proc_decompose_char, utf8proc_get_property,
    utf8proc_grapheme_break, utf8proc_grapheme_break_stateful, utf8proc_property_t,
    utf8proc_tolower, utf8proc_toupper,
};
use crate::window::WinInfoInfo;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;

pub type GraphemeState = i32;

#[cfg(windows)]
#[allow(non_camel_case_types)]
type wint_t = u16;
#[cfg(not(windows))]
#[allow(non_camel_case_types)]
type wint_t = u32;

const MAXCOL: usize = 0x7fff_ffff;
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

#[cfg(target_os = "linux")]
const CODESET: i32 = 14;
#[cfg(all(unix, not(target_os = "linux")))]
const CODESET: i32 = 0;

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
            call_iconv_close(fd as *mut c_void);
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
    width: c_char,
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
    cw_table: Vec<CwInterval>,
    mb_unescape_buf: [c_char; 6],
    iconv_working: WorkingStatus,
}

impl MbyteModuleState {
    const fn new() -> Self {
        Self {
            cw_table: Vec::new(),
            mb_unescape_buf: [0; 6],
            iconv_working: WorkingStatus::KUnknown,
        }
    }
}

static MBYTE_STATE: crate::globals::GlobalState<MbyteModuleState> =
    crate::globals::GlobalState::new(MbyteModuleState::new());

#[inline]
fn mbyte_st() -> &'static mut MbyteModuleState {
    unsafe { &mut *MBYTE_STATE.as_ptr() }
}

unsafe extern "C" {
    fn towupper(wc: wint_t) -> wint_t;
    fn towlower(wc: wint_t) -> wint_t;
}

#[cfg(windows)]
unsafe extern "system" {
    fn MultiByteToWideChar(
        CodePage: u32,
        dwFlags: u32,
        lpMultiByteStr: *const c_char,
        cbMultiByte: i32,
        lpWideCharStr: *mut u16,
        cchWideChar: i32,
    ) -> i32;
    fn WideCharToMultiByte(
        CodePage: u32,
        dwFlags: u32,
        lpWideCharStr: *const u16,
        cchWideChar: i32,
        lpMultiByteStr: *mut c_char,
        cbMultiByte: i32,
        lpDefaultChar: *const c_char,
        lpUsedDefaultChar: *mut i32,
    ) -> i32;
}

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::change::TAB;
use crate::charset::{NL, NUL};
use crate::fileio::{
    ENC_2BYTE, ENC_2WORD, ENC_4BYTE, ENC_ENDIAN_L, ENC_LATIN1, ENC_UNICODE, ICONV_EINVAL,
};
use crate::globals::IOSIZE;
use crate::keycodes::{K_SPECIAL, KE_FILLER, KS_SPECIAL};
#[cfg(windows)]
use crate::os::os_win_console::GetLastError;
use crate::regexp::{GRAPHEME_STATE_INIT, NUMBUFLEN};
use crate::viml::parser::parser::CONV_NONE;

#[inline]
pub fn mb_ptr_adv(p: &mut *mut c_char) {
    *p = (*p).wrapping_add(utfc_ptr2len(*p) as usize);
}

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
pub fn mb_ptr_back<T, U>(s: *const T, p: *mut *mut U) {
    unsafe {
        let prev = (*p).cast::<c_char>().wrapping_sub(1);
        let step = 1 + utf_head_off(s.cast(), prev) as usize;
        *p = (*p).cast::<u8>().wrapping_sub(step).cast();
    }
}

#[inline]
pub fn utf_ptr2_char_info(p_in: impl crate::strings::IntoCCharPtr) -> CharInfo {
    let p_in = p_in.into_c_char_ptr();
    {
        let p = p_in.cast::<u8>();
        let first = unsafe { *p };
        if first < 0x80 {
            CharInfo {
                value: i32::from(first),
                len: 1,
            }
        } else {
            let mut len = i32::from(UTF8LEN_TAB[first as usize]);
            let code_point = utf_ptr2_char_info_impl(p, len as usize);
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
pub fn utf_ptr2_str_char_info(ptr: impl crate::strings::IntoCCharPtr) -> StrCharInfo {
    let ptr = ptr.into_c_char_ptr();
    StrCharInfo {
        ptr,
        chr: utf_ptr2_char_info(ptr),
    }
}

#[inline]
pub fn utf_ptr2_char_info_len(p_in: impl crate::strings::IntoCCharPtr, size: i32) -> CharInfo {
    let p_in = p_in.into_c_char_ptr();
    {
        let p = p_in.cast::<u8>();
        assert!(size >= 0);
        if size == 0 {
            return CharInfo { value: -1, len: 1 };
        }
        let first = unsafe { *p };
        if first < 0x80 {
            CharInfo {
                value: i32::from(first),
                len: 1,
            }
        } else {
            let mut len = i32::from(UTF8LEN_TAB[first as usize]);
            if len > size {
                return CharInfo { value: -1, len: 1 };
            }
            let code_point = utf_ptr2_char_info_impl(p, len as usize);
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

#[cfg(test)]
#[inline]
pub fn utf_slice2_char_info(bytes: &[u8]) -> CharInfo {
    utf_ptr2_char_info_len(bytes.as_ptr().cast::<c_char>(), bytes.len() as i32)
}

#[inline]
pub fn utf_cluster_cells(bytes: &[u8]) -> i32 {
    let mut scratch = bytes.len() as i32;
    let ci = utf_ptr2_str_char_info_len(bytes.as_ptr().cast::<c_char>(), scratch);
    utf_cluster_info_len(ci, &mut scratch).cells
}

#[inline]
pub fn utf_slice2char(bytes: &[u8]) -> i32 {
    utf_ptr2char(bytes.as_ptr().cast::<c_char>())
}

#[inline]
pub fn utf_slice2len(bytes: &[u8]) -> usize {
    utf_ptr2len(bytes.as_ptr().cast::<c_char>()) as usize
}

#[inline]
pub fn utf_ptr2_str_char_info_len(
    ptr: impl crate::strings::IntoCCharPtr,
    size: i32,
) -> StrCharInfo {
    let ptr = ptr.into_c_char_ptr();
    StrCharInfo {
        ptr,
        chr: utf_ptr2_char_info_len(ptr, size),
    }
}

#[inline]
pub fn utf_cluster_info(cur: StrCharInfo) -> ClusterInfo {
    if crate::memory::cstr_is_empty(cur.ptr) {
        return ClusterInfo {
            next: cur,
            cells: opt_vars().dy_escape_width,
            first_compose: false,
        };
    }
    let next = cur.ptr.wrapping_add(cur.chr.len as usize).cast::<u8>();
    if unsafe { *next } < 0x80 {
        return ClusterInfo {
            next: StrCharInfo {
                ptr: next.cast::<c_char>(),
                chr: CharInfo {
                    value: i32::from(unsafe { *next }),
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
pub fn utf_cluster_info_len(cur: StrCharInfo, size: &mut i32) -> ClusterInfo {
    *size -= cur.chr.len;
    let nextptr = cur.ptr.wrapping_add(cur.chr.len as usize).cast::<u8>();
    let nextval = if *size == 0 {
        -1
    } else {
        i32::from(unsafe { *nextptr })
    };
    if nextval < 0x80 {
        return ClusterInfo {
            next: StrCharInfo {
                ptr: nextptr.cast::<c_char>(),
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

fn enc_canon_search(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(name);
    for (i, entry) in ENC_CANON_TABLE.iter().enumerate() {
        if bytes == entry.name.to_bytes() {
            return i as i32;
        }
    }
    -1
}

pub fn enc_canon_props(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    {
        let i = enc_canon_search(name);
        if i >= 0 {
            return ENC_CANON_TABLE[i as usize].prop;
        }
        let bytes = crate::memory::cstr_as_bytes(name);
        if bytes.starts_with(b"2byte-") {
            ENC_DBCS
        } else if bytes.starts_with(b"8bit-") || bytes.starts_with(b"iso-8859-") {
            ENC_8BIT
        } else {
            0
        }
    }
}

pub fn bomb_size() -> i32 {
    let mut n = 0;
    if curbuf_mut().b_p_bomb != 0 && curbuf_mut().b_p_bin == 0 {
        let fenc = crate::memory::cstr_as_bytes(curbuf_mut().b_p_fenc);
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

pub fn remove_bom(s: *mut c_char) {
    unsafe {
        let mut p = s.cast::<u8>();
        while *p != 0 {
            if *p == 0xef && *p.wrapping_add(1) == 0xbb && *p.wrapping_add(2) == 0xbf {
                crate::memory::cstr_move(p.cast(), p.wrapping_add(3).cast::<c_char>());
            } else {
                p = p.wrapping_add(1);
            }
        }
    }
}

pub fn mb_get_class(p: impl crate::strings::IntoCCharPtr) -> i32 {
    let p = p.into_c_char_ptr();
    mb_get_class_tab(p, curbuf_mut().b_chartab.as_ptr())
}

pub fn mb_get_class_tab(p: impl crate::strings::IntoCCharPtr, chartab: *const u64) -> i32 {
    let p = p.into_c_char_ptr();
    {
        let b0 = unsafe { *p.cast::<u8>() };
        if UTF8LEN_TAB[b0 as usize] == 1 {
            if b0 == 0 || ascii_iswhite(i32::from(b0)) {
                return 0;
            }
            if vim_iswordc_tab(i32::from(b0), chartab) {
                return 2;
            }
            return 1;
        }
        utf_class_tab(utf_ptr2char(p), chartab)
    }
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
        && crate::memory::cstr_first_byte(opt_vars().p_ambw) == b'd'
        && prop.ambiguous_width()
    {
        return 2;
    }

    if opt_vars().p_emoji != 0 && c >= 0x1f000 && !prop.ambiguous_width() && prop_is_emojilike(prop)
    {
        return 2;
    }

    1
}

pub fn ptr2cells(p_in: impl crate::strings::IntoCCharPtr) -> i32 {
    let p_in = p_in.into_c_char_ptr();
    utf_cluster_info(utf_ptr2_str_char_info(p_in)).cells
}

pub fn utf_ptr2cells(p_in: impl crate::strings::IntoCCharPtr) -> i32 {
    let p_in = p_in.into_c_char_ptr();
    {
        if unsafe { *p_in.cast::<u8>() } < 0x80 {
            return 1;
        }
        ptr2cells(p_in)
    }
}

pub fn utf_ptr2_char_info_impl(p: *const u8, len: usize) -> i32 {
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
    let mut cur = unsafe { *p.wrapping_add(1) };
    let mut code_point = (u32::from(unsafe { *p }) << 6).wrapping_add(u32::from(cur));
    if (cur & 0xC0) != 0x80 {
        return -1;
    }
    if (len as u32) < 3 {
        return code_point.wrapping_add(corr) as i32;
    }

    cur = unsafe { *p.wrapping_add(2) };
    code_point = (code_point << 6).wrapping_add(u32::from(cur));
    if (cur & 0xC0) != 0x80 {
        return -1;
    }
    if (len as u32) == 3 {
        return code_point.wrapping_add(corr) as i32;
    }

    cur = unsafe { *p.wrapping_add(3) };
    code_point = (code_point << 6).wrapping_add(u32::from(cur));
    if (cur & 0xC0) != 0x80 {
        return -1;
    }
    if (len as u32) == 4 {
        return code_point.wrapping_add(corr) as i32;
    }

    cur = unsafe { *p.wrapping_add(4) };
    code_point = (code_point << 6).wrapping_add(u32::from(cur));
    if (cur & 0xC0) != 0x80 {
        return -1;
    }
    if (len as u32) == 5 {
        return code_point.wrapping_add(corr) as i32;
    }

    cur = unsafe { *p.wrapping_add(5) };
    code_point = (code_point << 6).wrapping_add(u32::from(cur));
    if (cur & 0xC0) != 0x80 {
        return -1;
    }

    code_point.wrapping_add(corr) as i32
}

pub fn mb_string2cells(str_: impl crate::strings::IntoCCharPtr) -> usize {
    let str_ = str_.into_c_char_ptr();
    {
        let mut clen = 0usize;
        let mut ci = utf_ptr2_str_char_info(str_);
        while !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_cluster_info(ci);
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

pub fn mb_string2cells_len(str_: impl crate::strings::IntoCCharPtr, size: usize) -> usize {
    let str_ = str_.into_c_char_ptr();
    let mut clen = 0usize;
    let mut len = size as i32;
    let mut ci = utf_ptr2_str_char_info_len(str_, len);
    while len > 0 {
        let cli = utf_cluster_info_len(ci, &mut len);
        clen += cli.cells as usize;
        ci = cli.next;
    }
    clen
}

pub fn utf_ptr2char(p_in: impl crate::strings::IntoCCharPtr) -> i32 {
    let p_in = p_in.into_c_char_ptr();
    {
        let p = p_in.cast::<u8>();
        let v0 = u32::from(unsafe { *p });
        if v0 < 0x80 {
            return v0 as i32;
        }

        let len = UTF8LEN_TAB[v0 as usize];
        if len < 2 {
            return v0 as i32;
        }

        let v1 = u32::from(unsafe { *p.wrapping_add(1) });
        if (v1 & 0xC0) != 0x80 {
            return v0 as i32;
        }
        if len == 2 {
            return ((v0 << 6) + v1 - ((0xC0 << 6) + 0x80)) as i32;
        }

        let v2 = u32::from(unsafe { *p.wrapping_add(2) });
        if (v2 & 0xC0) != 0x80 {
            return v0 as i32;
        }
        if len == 3 {
            return ((v0 << 12) + (v1 << 6) + v2 - ((0xE0 << 12) + (0x80 << 6) + 0x80)) as i32;
        }

        let v3 = u32::from(unsafe { *p.wrapping_add(3) });
        if (v3 & 0xC0) != 0x80 {
            return v0 as i32;
        }
        if len == 4 {
            return ((v0 << 18) + (v1 << 12) + (v2 << 6) + v3
                - ((0xF0 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80)) as i32;
        }

        let v4 = u32::from(unsafe { *p.wrapping_add(4) });
        if (v4 & 0xC0) != 0x80 {
            return v0 as i32;
        }
        if len == 5 {
            return ((v0 << 24) + (v1 << 18) + (v2 << 12) + (v3 << 6) + v4
                - ((0xF8 << 24) + (0x80 << 18) + (0x80 << 12) + (0x80 << 6) + 0x80))
                as i32;
        }

        let v5 = u32::from(unsafe { *p.wrapping_add(5) });
        if (v5 & 0xC0) != 0x80 {
            return v0 as i32;
        }
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

fn utf_safe_read_char_adv(s: &mut *const c_char, n: &mut usize) -> i32 {
    if *n == 0 {
        return 0;
    }

    let b0 = unsafe { *(*s).cast::<u8>() };
    let k = usize::from(UTF8LEN_TAB_ZERO[b0 as usize]);
    if k == 1 {
        *n -= 1;
        *s = (*s).wrapping_add(1);
        return i32::from(b0);
    }

    if k <= *n {
        let c = utf_ptr2char(*s);
        if c != i32::from(b0)
            || (c == 0xC3 && unsafe { *(*s).wrapping_add(1).cast::<u8>() } == 0x83)
        {
            *s = (*s).wrapping_add(k);
            *n -= k;
            return c;
        }
    }

    -1
}

pub fn mb_ptr2char_adv(pp: *mut *const c_char) -> i32 {
    unsafe {
        let c = utf_ptr2char(*pp);
        *pp = (*pp).wrapping_add(utfc_ptr2len(*pp) as usize);
        c
    }
}

pub fn mb_cptr2char_adv(pp: *mut *const c_char) -> i32 {
    unsafe {
        let c = utf_ptr2char(*pp);
        *pp = (*pp).wrapping_add(utf_ptr2len(*pp) as usize);
        c
    }
}

pub fn utf_iscomposing_first(c: i32) -> bool {
    c >= 128 && !utf8proc_grapheme_break(b' ' as i32, c)
}

pub fn utf_composinglike(
    p1: impl crate::strings::IntoCCharPtr,
    p2: impl crate::strings::IntoCCharPtr,
    state: Option<&mut GraphemeState>,
) -> bool {
    let p1 = p1.into_c_char_ptr();
    let p2 = p2.into_c_char_ptr();
    {
        if unsafe { *p2.cast::<u8>() } < 128 {
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

pub fn utf_iscomposing(c1: i32, c2: i32, state: &mut GraphemeState) -> bool {
    !utf8proc_grapheme_break_stateful(c1, c2, Some(state)) || arabic_combine(c1, c2)
}

pub fn utf_ptr2len(p_in: impl crate::strings::IntoCCharPtr) -> i32 {
    let p_in = p_in.into_c_char_ptr();
    {
        let p = p_in.cast::<u8>();
        if unsafe { *p } == 0 {
            return 0;
        }
        let len = i32::from(UTF8LEN_TAB[unsafe { *p } as usize]);
        for i in 1..len as usize {
            if (unsafe { *p.wrapping_add(i) } & 0xc0) != 0x80 {
                return 1;
            }
        }
        len
    }
}

pub fn utf_byte2len(b: i32) -> i32 {
    i32::from(UTF8LEN_TAB[(b as u8) as usize])
}

pub fn utf_ptr2len_len(p: impl crate::strings::IntoCCharPtr, size: i32) -> i32 {
    let p = p.into_c_char_ptr();
    {
        let bytes = p.cast::<u8>();
        let len = i32::from(UTF8LEN_TAB[unsafe { *bytes } as usize]);
        if len == 1 {
            return 1;
        }
        let m = len.min(size);
        for i in 1..m as usize {
            if (unsafe { *bytes.wrapping_add(i) } & 0xc0) != 0x80 {
                return 1;
            }
        }
        len
    }
}

pub fn utfc_ptr2len(p: impl crate::strings::IntoCCharPtr) -> i32 {
    let p = p.into_c_char_ptr();
    {
        let b0 = unsafe { *p.cast::<u8>() };
        if b0 == 0 {
            return 0;
        }
        if b0 < 0x80 && unsafe { *p.wrapping_add(1).cast::<u8>() } < 0x80 {
            return 1;
        }

        let mut len = utf_ptr2len(p);
        if len == 1 && b0 >= 0x80 {
            return 1;
        }

        let mut prevlen = 0usize;
        let mut state: GraphemeState = GRAPHEME_STATE_INIT;
        loop {
            let next_ptr = p.wrapping_add(len as usize);
            if unsafe { *next_ptr.cast::<u8>() } < 0x80
                || !utf_composinglike(p.wrapping_add(prevlen), next_ptr, Some(&mut state))
            {
                return len;
            }
            prevlen = len as usize;
            len += utf_ptr2len(next_ptr);
        }
    }
}

pub fn utfc_slice2len(bytes: &[u8]) -> usize {
    utfc_ptr2len_len(bytes.as_ptr().cast::<c_char>(), bytes.len() as i32) as usize
}

pub fn mb_copy_slice_char(src: &[u8], dst: &mut [u8]) -> usize {
    let l = utfc_slice2len(src).min(src.len());
    dst[..l].copy_from_slice(&src[..l]);
    l
}

pub fn utfc_ptr2len_len(p: impl crate::strings::IntoCCharPtr, size: i32) -> i32 {
    let p = p.into_c_char_ptr();
    {
        if size < 1 || crate::memory::cstr_is_empty(p) {
            return 0;
        }
        let b0 = unsafe { *p.cast::<u8>() };
        if b0 < 0x80 && (size == 1 || unsafe { *p.wrapping_add(1).cast::<u8>() } < 0x80) {
            return 1;
        }

        let mut len = utf_ptr2len_len(p, size);
        if (len == 1 && b0 >= 0x80) || len > size {
            return 1;
        }

        let mut prevlen = 0usize;
        let mut state: GraphemeState = GRAPHEME_STATE_INIT;
        while len < size {
            let next_ptr = p.wrapping_add(len as usize);
            if unsafe { *next_ptr.cast::<u8>() } < 0x80 {
                break;
            }
            let len_next_char = utf_ptr2len_len(next_ptr, size - len);
            if len_next_char > size - len {
                break;
            }
            if !utf_composinglike(p.wrapping_add(prevlen), next_ptr, Some(&mut state)) {
                break;
            }
            prevlen = len as usize;
            len += len_next_char;
        }
        len
    }
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

pub fn utf_char2bytes(c: i32, buf: *mut c_char) -> i32 {
    let slice = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), 6) };
    utf_char2slice(c, slice) as i32
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
    utf_class_tab(c, curbuf_mut().b_chartab.as_ptr())
}

pub fn utf_class_tab(c: i32, chartab: *const u64) -> i32 {
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
    if a < 128 && (opt_vars().cmp_flags & K_OPT_CMP_FLAG_KEEPASCII) != 0 {
        return toupper_asc(a);
    }
    if (opt_vars().cmp_flags & K_OPT_CMP_FLAG_INTERNAL) == 0 {
        return unsafe { towupper(a as wint_t) } as i32;
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
    if a < 128 && (opt_vars().cmp_flags & K_OPT_CMP_FLAG_KEEPASCII) != 0 {
        return tolower_asc(a);
    }
    if (opt_vars().cmp_flags & K_OPT_CMP_FLAG_INTERNAL) == 0 {
        return unsafe { towlower(a as wint_t) } as i32;
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

pub fn utf_strnicmp(
    s1: impl crate::strings::IntoCCharPtr,
    s2: impl crate::strings::IntoCCharPtr,
    mut n1: usize,
    mut n2: usize,
) -> i32 {
    let mut s1 = s1.into_c_char_ptr();
    let mut s2 = s2.into_c_char_ptr();
    {
        let mut c1: i32;
        let mut c2: i32;
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

        while n1 > 0
            && n2 > 0
            && !crate::memory::cstr_is_empty(s1)
            && !crate::memory::cstr_is_empty(s2)
        {
            let cdiff =
                i32::from(unsafe { *s1.cast::<u8>() }) - i32::from(unsafe { *s2.cast::<u8>() });
            if cdiff != 0 {
                return cdiff;
            }
            s1 = s1.wrapping_add(1);
            s2 = s2.wrapping_add(1);
            n1 -= 1;
            n2 -= 1;
        }

        if n1 > 0 && crate::memory::cstr_is_empty(s1) {
            n1 = 0;
        }
        if n2 > 0 && crate::memory::cstr_is_empty(s2) {
            n2 = 0;
        }
        if n1 == 0 && n2 == 0 {
            return 0;
        }
        if n1 == 0 { -1 } else { 1 }
    }
}

#[cfg(windows)]
pub fn utf8_to_utf16(
    utf8: impl crate::strings::IntoCCharPtr,
    utf8len: i32,
    utf16: &mut Vec<u16>,
) -> i32 {
    let utf8 = utf8.into_c_char_ptr();
    unsafe {
        utf16.clear();
        let mut bufsize = MultiByteToWideChar(CP_UTF8, 0, utf8, utf8len, ptr::null_mut(), 0);
        if bufsize == 0 {
            return uv_translate_sys_error(GetLastError() as i32);
        }

        utf16.resize((bufsize as usize) + 1, 0);
        bufsize = MultiByteToWideChar(CP_UTF8, 0, utf8, utf8len, utf16.as_mut_ptr(), bufsize);
        if bufsize == 0 {
            utf16.clear();
            return uv_translate_sys_error(GetLastError() as i32);
        }

        utf16[bufsize as usize] = 0;
        0
    }
}

#[cfg(windows)]
pub fn utf16_to_utf8(
    utf16: *const u16,
    utf16len: i32,
    utf8: &mut Option<std::ffi::CString>,
) -> i32 {
    let mut bufsize = unsafe {
        WideCharToMultiByte(
            CP_UTF8,
            0,
            utf16,
            utf16len,
            ptr::null_mut(),
            0,
            ptr::null(),
            ptr::null_mut(),
        )
    };
    if bufsize == 0 {
        *utf8 = None;
        return unsafe { uv_translate_sys_error(GetLastError() as i32) };
    }

    let mut vec = vec![0 as c_char; (bufsize as usize) + 1];
    bufsize = unsafe {
        WideCharToMultiByte(
            CP_UTF8,
            0,
            utf16,
            utf16len,
            vec.as_mut_ptr(),
            bufsize,
            ptr::null(),
            ptr::null_mut(),
        )
    };
    if bufsize == 0 {
        *utf8 = None;
        return unsafe { uv_translate_sys_error(GetLastError() as i32) };
    }

    vec[bufsize as usize] = NUL;
    *utf8 = Some(crate::memory::cstr_to_cstring(vec.as_ptr()));
    0
}

pub fn mb_utflen(
    s: impl crate::strings::IntoCCharPtr,
    len: usize,
    codepoints: &mut usize,
    codeunits: &mut usize,
) {
    let s = s.into_c_char_ptr();
    {
        let mut count = 0usize;
        let mut extra = 0usize;
        let mut i = 0usize;
        while i < len {
            let clen = utf_ptr2len_len(s.wrapping_add(i), (len - i) as i32) as usize;
            let c = if clen > 1 {
                utf_ptr2char(s.wrapping_add(i))
            } else {
                i32::from(unsafe { *s.wrapping_add(i).cast::<u8>() })
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

pub fn mb_utf_index_to_bytes(
    s: impl crate::strings::IntoCCharPtr,
    len: usize,
    index: usize,
    use_utf16_units: bool,
) -> isize {
    let s = s.into_c_char_ptr();
    {
        if index == 0 {
            return 0;
        }
        let mut count = 0usize;
        let mut i = 0usize;
        while i < len {
            let clen = utf_ptr2len_len(s.wrapping_add(i), (len - i) as i32) as usize;
            let c = if clen > 1 {
                utf_ptr2char(s.wrapping_add(i))
            } else {
                i32::from(unsafe { *s.wrapping_add(i).cast::<u8>() })
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

pub fn mb_strnicmp(
    s1: impl crate::strings::IntoCCharPtr,
    s2: impl crate::strings::IntoCCharPtr,
    nn: usize,
) -> i32 {
    let s1 = s1.into_c_char_ptr();
    let s2 = s2.into_c_char_ptr();
    utf_strnicmp(s1, s2, nn, nn)
}

pub fn mb_stricmp(
    s1: impl crate::strings::IntoCCharPtr,
    s2: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let s1 = s1.into_c_char_ptr();
    let s2 = s2.into_c_char_ptr();
    mb_strnicmp(s1, s2, MAXCOL)
}

pub fn show_utf8() {
    let line = get_cursor_pos_ptr();
    let len = utfc_ptr2len(line);
    if len == 0 {
        msg(c"NUL", 0);
        return;
    }

    let mut out = String::new();
    let mut clen = 0;
    for i in 0..len as usize {
        if clen == 0 {
            if i > 0 {
                out.push_str("+ ");
            }
            clen = utf_ptr2len(line.wrapping_add(i));
        }
        let b = unsafe { *line.wrapping_add(i) };
        let byte_val: u8 = if b == NL { 0 } else { b as u8 };
        use std::fmt::Write as _;
        let _ = write!(out, "{byte_val:02x} ");
        clen -= 1;
        if out.len() > IOSIZE - 20 {
            break;
        }
    }

    let copy_len = out.len().min(IOSIZE - 1);
    let io_ptr = g_st().iobuff.as_mut_ptr();
    unsafe { ptr::copy_nonoverlapping(out.as_ptr().cast::<c_char>(), io_ptr, copy_len) };
    unsafe {
        *io_ptr.wrapping_add(copy_len) = NUL;
    }
    msg(io_ptr, 0);
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

pub fn utf_head_off(
    base_in: impl crate::strings::IntoCCharPtr,
    p_in: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let base_in = base_in.into_c_char_ptr();
    let p_in = p_in.into_c_char_ptr();
    {
        if unsafe { *p_in.cast::<u8>() } < 0x80 {
            return 0;
        }

        let base = base_in.cast::<u8>();
        let p = p_in.cast::<u8>();
        let mut start = p;

        while start > base
            && (unsafe { *start } & 0xc0) == 0x80
            && (crate::memory::ptr_offset_from(p, start) as usize) < 6
        {
            start = start.wrapping_sub(1);
        }

        let last_len = UTF8LEN_TAB[unsafe { *start } as usize];
        let mut cur_code = utf_ptr2_char_info_impl(start, usize::from(last_len));
        if cur_code < 0 || crate::memory::ptr_offset_from(p, start) >= isize::from(last_len) {
            return 0;
        }
        let safe_end = start.wrapping_add(usize::from(last_len));

        let mut cur_bc = utf8proc_get_property(cur_code).boundclass();
        if always_break(cur_bc) || start == base {
            return crate::memory::ptr_offset_from(p, start) as i32;
        }

        let mut cur_pos = start;
        let p_start = start;

        loop {
            if unsafe { *start.wrapping_sub(1) } == 0 {
                break;
            }
            start = start.wrapping_sub(1);
            if unsafe { *start } < 0x80 {
                break;
            }

            while start > base
                && (unsafe { *start } & 0xc0) == 0x80
                && (crate::memory::ptr_offset_from(cur_pos, start) as usize) < 6
            {
                start = start.wrapping_sub(1);
            }

            let prev_len = i32::from(UTF8LEN_TAB[unsafe { *start } as usize]);
            let prev_code = utf_ptr2_char_info_impl(start, prev_len as usize);
            if prev_code < 0 || (prev_len as isize) < crate::memory::ptr_offset_from(cur_pos, start)
            {
                start = cur_pos;
                break;
            }

            let prev_bc = utf8proc_get_property(prev_code).boundclass();
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

        if start == p_start && isize::from(last_len) > crate::memory::ptr_offset_from(p, start) {
            return crate::memory::ptr_offset_from(p, start) as i32;
        }

        let mut q = start;
        while q < p {
            let len = utfc_ptr2len_len(
                q.cast::<c_char>(),
                crate::memory::ptr_offset_from(safe_end, q) as i32,
            );
            if q.wrapping_add(len as usize) > p {
                return crate::memory::ptr_offset_from(p, q) as i32;
            }
            q = q.wrapping_add(len as usize);
        }

        0
    }
}

pub fn utf_cluster_info_impl(base: StrCharInfo, max_len: &mut i32) -> ClusterInfo {
    let mut cells = basechar_cells_impl(base.chr);
    let first_compose = utf_iscomposing_first(base.chr.value);
    let mut prev_code = base.chr.value;
    let mut next = base.ptr.wrapping_add(base.chr.len as usize).cast::<u8>();
    let mut state: GraphemeState = GRAPHEME_STATE_INIT;
    assert!(unsafe { *next } >= 0x80 && *max_len > 0);
    let mut cluster_len = base.chr.len;

    let mut check_emoji = cells == 1
        && opt_vars().p_emoji != 0
        && prop_is_emojilike(utf8proc_get_property(base.chr.value));

    loop {
        let next_len = UTF8LEN_TAB[unsafe { *next } as usize];
        if i32::from(next_len) > *max_len {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
                    chr: CharInfo { value: -1, len: 1 },
                },
                cells,
                first_compose,
            };
        }

        let next_code = utf_ptr2_char_info_impl(next, usize::from(next_len));
        if next_code < 0 || !utf_iscomposing(prev_code, next_code, &mut state) {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
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
        next = next.wrapping_add(usize::from(next_len));
        *max_len -= i32::from(next_len);
        if *max_len > 0 && unsafe { *next } < 0x80 {
            return ClusterInfo {
                next: StrCharInfo {
                    ptr: next.cast::<c_char>(),
                    chr: CharInfo {
                        value: i32::from(unsafe { *next }),
                        len: 1,
                    },
                },
                cells,
                first_compose,
            };
        }
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

pub fn mb_copy_char(fp: *mut *const c_char, tp: *mut *mut c_char) {
    unsafe {
        let l = utfc_ptr2len(*fp) as usize;
        ptr::copy(*fp, *tp, l);
        *tp = (*tp).wrapping_add(l);
        *fp = (*fp).wrapping_add(l);
    }
}

pub fn mb_off_next(
    base: impl crate::strings::IntoCCharPtr,
    p: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let base = base.into_c_char_ptr();
    let p = p.into_c_char_ptr();
    {
        let head_off = utf_head_off(base, p);
        if head_off == 0 {
            return 0;
        }
        utfc_ptr2len(p.wrapping_sub(head_off as usize)) - head_off
    }
}

pub fn utf_cp_bounds_len(
    base: impl crate::strings::IntoCCharPtr,
    p_in: impl crate::strings::IntoCCharPtr,
    p_len: i32,
) -> CharBoundsOff {
    let base = base.into_c_char_ptr();
    let p_in = p_in.into_c_char_ptr();
    {
        assert!(base <= p_in && p_len > 0);
        let b = base.cast::<u8>();
        let p = p_in.cast::<u8>();
        if unsafe { *p } < 0x80 {
            return CharBoundsOff {
                begin_off: 0,
                end_off: 1,
            };
        }

        let max_first_off = -((crate::memory::ptr_offset_from(p, b) as i32).min(MB_MAXCHAR - 1));
        let mut first_off: i32 = 0;
        while utf_is_trail_byte(unsafe { *p.wrapping_offset(first_off as isize) }) {
            if first_off == max_first_off {
                return CharBoundsOff {
                    begin_off: 0,
                    end_off: 1,
                };
            }
            first_off -= 1;
        }

        let lead = unsafe { *p.wrapping_offset(first_off as isize) };
        let max_end_off = i32::from(UTF8LEN_TAB[lead as usize]) + first_off;
        if max_end_off <= 0 || max_end_off > p_len {
            return CharBoundsOff {
                begin_off: 0,
                end_off: 1,
            };
        }

        for end_off in 1..max_end_off {
            if !utf_is_trail_byte(unsafe { *p.wrapping_offset(end_off as isize) }) {
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

pub fn utf_cp_bounds(
    base: impl crate::strings::IntoCCharPtr,
    p_in: impl crate::strings::IntoCCharPtr,
) -> CharBoundsOff {
    let base = base.into_c_char_ptr();
    let p_in = p_in.into_c_char_ptr();
    utf_cp_bounds_len(base, p_in, i32::MAX)
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

    if (enc_canon_props(curbuf_mut().b_p_fenc) & ENC_8BIT) != 0 {
        convert_setup(&mut vimconv, opt_vars().p_enc, curbuf_mut().b_p_fenc);
    }

    curwin_mut().w_cursor.coladd = 0;
    'search: loop {
        let mut p = get_cursor_pos_ptr();
        if vimconv.vc_type != CONV_NONE {
            tofree = string_convert(&vimconv, p, None);
            let Some(ref tf) = tofree else {
                break;
            };
            p = tf.as_ptr().cast::<c_char>().cast_mut();
        }

        while !crate::memory::cstr_is_empty(p) {
            let mut len = utf_ptr2len(p);
            if unsafe { *p.cast::<u8>() } >= 0x80
                && (len == 1 || utf_char2len(utf_ptr2char(p)) != len)
            {
                if vimconv.vc_type == CONV_NONE {
                    curwin_mut().w_cursor.col +=
                        crate::memory::ptr_offset_from(p, get_cursor_pos_ptr()) as Colnr;
                } else {
                    len = crate::memory::ptr_offset_from(
                        p,
                        tofree.as_ref().unwrap().as_ptr().cast::<c_char>(),
                    ) as i32;
                    let mut cur_p = get_cursor_pos_ptr();
                    while !crate::memory::cstr_is_empty(cur_p) && len > 0 {
                        len -= 1;
                        let l = utf_ptr2len(cur_p);
                        curwin_mut().w_cursor.col += l;
                        cur_p = cur_p.wrapping_add(l as usize);
                    }
                }
                curwin_mut().w_set_curswant = true;
                break 'search;
            }
            p = p.wrapping_add(len as usize);
        }
        if curwin_mut().w_cursor.lnum == curbuf_mut().b_ml.ml_line_count {
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

pub fn utf_valid_string(
    s: impl crate::strings::IntoCCharPtr,
    end: impl crate::strings::IntoCCharPtr,
) -> bool {
    let s = s.into_c_char_ptr();
    let end = end.into_c_char_ptr();
    unsafe {
        let mut p = s.cast::<u8>();
        let end_u8 = end.cast::<u8>();

        while if end_u8.is_null() {
            *p != 0
        } else {
            p < end_u8
        } {
            let mut l = usize::from(UTF8LEN_TAB_ZERO[*p as usize]);
            if l == 0 {
                return false;
            }
            if !end_u8.is_null() && p.wrapping_add(l) > end_u8 {
                return false;
            }
            p = p.wrapping_add(1);
            l -= 1;
            while l > 0 {
                if (*p & 0xc0) != 0x80 {
                    return false;
                }
                p = p.wrapping_add(1);
                l -= 1;
            }
        }
        true
    }
}

pub fn mb_adjust_cursor() {
    mark_mb_adjustpos(g_st().curbuf.cast(), &mut curwin_mut().w_cursor);
}

pub fn mb_check_adjust_col(win: &mut WinInfoInfo) {
    let oldcol = win.w_cursor.col;

    if oldcol != 0 {
        let p = ml_get_buf(win.buf_mut(), win.w_cursor.lnum);
        let len = crate::memory::cstr_len(p) as Colnr;

        if len == 0 || oldcol < 0 {
            win.w_cursor.col = 0;
        } else {
            if oldcol > len {
                win.w_cursor.col = len - 1;
            }
            win.w_cursor.col -= utf_head_off(p, p.wrapping_add(win.w_cursor.col as usize));
        }

        let cur_col = win.w_cursor.col as usize;
        if win.w_cursor.coladd == 1
            && unsafe { *p.wrapping_add(cur_col) } != TAB
            && vim_isprintc(utf_ptr2char(p.wrapping_add(cur_col)))
            && ptr2cells(p.wrapping_add(cur_col)) > 1
        {
            win.w_cursor.coladd = 0;
        }
    }
}

pub fn mb_prevptr(line: *mut c_char, mut p: *mut c_char) -> *mut c_char {
    if p > line {
        let off = utf_head_off(line, p.wrapping_sub(1)) + 1;
        p = p.wrapping_sub(off as usize);
    }
    p
}

pub fn mb_charlen(str_: impl crate::strings::IntoCCharPtr) -> i32 {
    let str_ = str_.into_c_char_ptr();
    {
        if str_.is_null() {
            return 0;
        }
        let mut p = str_;
        let mut count = 0;
        while !crate::memory::cstr_is_empty(p) {
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
            count += 1;
        }
        count
    }
}

pub fn mb_charlen_len(str_: impl crate::strings::IntoCCharPtr, len: i32) -> i32 {
    let str_ = str_.into_c_char_ptr();
    {
        let mut p = str_;
        let end = str_.wrapping_add(len.max(0) as usize);
        let mut count = 0;
        while !crate::memory::cstr_is_empty(p) && p < end {
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
            count += 1;
        }
        count
    }
}

pub fn mb_unescape(pp: *mut *const c_char) -> *const c_char {
    unsafe {
        let mut buf_idx = 0usize;
        let str_ = (*pp).cast::<u8>();
        let mut str_idx = 0usize;

        while *str_.wrapping_add(str_idx) != 0 && buf_idx < 4 {
            if *str_.wrapping_add(str_idx) == K_SPECIAL
                && *str_.wrapping_add(str_idx + 1) == KS_SPECIAL
                && *str_.wrapping_add(str_idx + 2) == KE_FILLER
            {
                mbyte_st().mb_unescape_buf[buf_idx] = K_SPECIAL as c_char;
                buf_idx += 1;
                str_idx += 2;
            } else if *str_.wrapping_add(str_idx) == K_SPECIAL {
                break;
            } else {
                mbyte_st().mb_unescape_buf[buf_idx] = *str_.wrapping_add(str_idx) as c_char;
                buf_idx += 1;
            }
            mbyte_st().mb_unescape_buf[buf_idx] = NUL;

            if utf_ptr2len((&raw const mbyte_st().mb_unescape_buf).cast::<c_char>()) > 1 {
                *pp = str_.wrapping_add(str_idx + 1).cast::<c_char>();
                return (&raw const mbyte_st().mb_unescape_buf).cast::<c_char>();
            }

            if (mbyte_st().mb_unescape_buf[0] as u8) < 128 {
                break;
            }
            str_idx += 1;
        }
        ptr::null()
    }
}

pub fn enc_skip(p: *mut c_char) -> *mut c_char {
    let bytes = crate::memory::cstr_as_bytes(p);
    if bytes.starts_with(b"2byte-") {
        p.wrapping_add(6)
    } else if bytes.starts_with(b"8bit-") {
        p.wrapping_add(5)
    } else {
        p
    }
}

fn enc_alias_search(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(name);
    for entry in ENC_ALIAS_TABLE {
        if bytes == entry.name.to_bytes() {
            return entry.canon as i32;
        }
    }
    -1
}

pub fn enc_canonize(enc: impl crate::strings::IntoCCharPtr) -> std::ffi::CString {
    let enc = enc.into_c_char_ptr();
    unsafe {
        let enc_bytes = crate::memory::cstr_as_bytes(enc);
        if enc_bytes == b"default" {
            return (&raw const g_st().fenc_default)
                .as_ref()
                .unwrap()
                .clone()
                .unwrap_or_default();
        }

        let mut r_vec: Vec<c_char> = vec![0; enc_bytes.len() + 3];
        let r = r_vec.as_mut_ptr();
        let mut p = r;
        for &b in enc_bytes {
            *p = if b == b'_' {
                b'-' as c_char
            } else {
                tolower_asc(i32::from(b)) as u8 as c_char
            };
            p = p.wrapping_add(1);
        }
        *p = NUL;

        p = enc_skip(r);

        let p_bytes = crate::memory::cstr_as_bytes(p);
        if p_bytes.starts_with(b"microsoft-cp") {
            crate::memory::cstr_move(p, p.wrapping_add(10));
        }

        let p_bytes = crate::memory::cstr_as_bytes(p);
        if p_bytes.starts_with(b"iso8859") {
            crate::memory::cstr_move(p.wrapping_add(4), p.wrapping_add(3));
            *p.wrapping_add(3) = b'-' as c_char;
        }

        let p_bytes = crate::memory::cstr_as_bytes(p);
        if p_bytes.starts_with(b"iso-8859") && p_bytes.len() > 8 && p_bytes[8].is_ascii_digit() {
            crate::memory::cstr_move(p.wrapping_add(9), p.wrapping_add(8));
            *p.wrapping_add(8) = b'-' as c_char;
        }

        let p_bytes = crate::memory::cstr_as_bytes(p);
        if p_bytes.starts_with(b"latin-") {
            crate::memory::cstr_move(p.wrapping_add(5), p.wrapping_add(6));
        }

        if enc_canon_search(p) >= 0 {
            if p != r {
                crate::memory::cstr_move(r, p);
            }
        } else {
            let i = enc_alias_search(p);
            if i >= 0 {
                return ENC_CANON_TABLE[i as usize].name.to_owned();
            }
        }
        crate::memory::cstr_to_cstring(r)
    }
}

pub fn enc_locale() -> Option<std::ffi::CString> {
    #[cfg(unix)]
    let mut s: *const c_char = unsafe { nl_langinfo(CODESET).cast() };
    #[cfg(not(unix))]
    let mut s: *const c_char = ptr::null();

    let mut env_buf = [0 as c_char; 64];
    if crate::memory::cstr_is_empty(s) {
        s = unsafe { setlocale(LC_CTYPE, ptr::null()).cast() };
        if crate::memory::cstr_is_empty(s) {
            s = os_getenv_buf(c"LC_ALL", env_buf.as_mut_ptr(), env_buf.len());
            if s.is_null() {
                s = os_getenv_buf(c"LC_CTYPE", env_buf.as_mut_ptr(), env_buf.len());
                if s.is_null() {
                    s = os_getenv_buf(c"LANG", env_buf.as_mut_ptr(), env_buf.len());
                }
            }
        }
    }

    if s.is_null() {
        return None;
    }

    let mut buf = [0 as c_char; 50];
    let p = vim_strchr(s, b'.' as i32);
    if !p.is_null() {
        if p.cast_const() > s.wrapping_add(2)
            && crate::memory::cstr_prefix_bytes(p.wrapping_add(1), 3).eq_ignore_ascii_case(b"EUC")
            && !(crate::memory::cstr_byte_at(p, 4)).is_ascii_alphanumeric()
            && crate::memory::cstr_byte_at(p, 4) != b'-'
            && unsafe { *p.wrapping_sub(3) } == b'_' as c_char
        {
            unsafe {
                ptr::copy_nonoverlapping(b"euc-".as_ptr().cast::<c_char>(), buf.as_mut_ptr(), 4)
            };
            let c2 = unsafe { *p.wrapping_sub(2) } as u8;
            let c1 = unsafe { *p.wrapping_sub(1) } as u8;
            buf[4] = if c2.is_ascii_alphanumeric() {
                tolower_asc(i32::from(c2)) as u8 as c_char
            } else {
                0
            };
            buf[5] = if c1.is_ascii_alphanumeric() {
                tolower_asc(i32::from(c1)) as u8 as c_char
            } else {
                0
            };
            buf[6] = NUL;
            return Some(enc_canonize(buf.as_ptr()));
        }
        s = p.wrapping_add(1);
    }

    let mut i = 0usize;
    while i < buf.len() - 1 && crate::memory::cstr_byte_at(s, i) != 0 {
        let ch = crate::memory::cstr_byte_at(s, i);
        if ch == b'_' || ch == b'-' {
            buf[i] = b'-' as c_char;
        } else if ch.is_ascii_alphanumeric() {
            buf[i] = tolower_asc(i32::from(ch)) as u8 as c_char;
        } else {
            break;
        }
        i += 1;
    }
    buf[i] = NUL;

    Some(enc_canonize(buf.as_ptr()))
}

pub fn my_iconv_open(
    to: impl crate::strings::IntoCCharPtr,
    from: impl crate::strings::IntoCCharPtr,
) -> Option<usize> {
    let from = from.into_c_char_ptr().cast_mut();
    let to = to.into_c_char_ptr().cast_mut();
    const ICONV_TESTLEN: usize = 400;
    if mbyte_st().iconv_working == WorkingStatus::KBroken {
        return None;
    }
    let mut fd = call_iconv_open(enc_skip(to), enc_skip(from));
    if fd != usize::MAX as *mut c_void && mbyte_st().iconv_working == WorkingStatus::KUnknown {
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
            mbyte_st().iconv_working = WorkingStatus::KBroken;
            call_iconv_close(fd);
            fd = usize::MAX as *mut c_void;
        } else {
            mbyte_st().iconv_working = WorkingStatus::Working;
        }
    }
    (fd != usize::MAX as *mut c_void).then_some(fd as usize)
}

fn iconv_string(
    vcp: &Vimconv,
    str_: impl crate::strings::IntoCCharPtr,
    slen: usize,
    mut unconvlenp: Option<&mut usize>,
    mut resultlenp: Option<&mut usize>,
) -> Option<Vec<u8>> {
    let str_ = str_.into_c_char_ptr();
    {
        let mut to: *mut c_char;
        let mut len = 0usize;
        let mut done = 0usize;
        let mut result_vec: Vec<u8> = Vec::new();
        let fd = vcp.vc_fd.unwrap_or(usize::MAX) as *mut c_void;

        let mut from = str_;
        let mut fromlen = slen;
        loop {
            if len == 0 || os_errno() == ICONV_E2BIG {
                len = len + fromlen * 2 + 40;
                result_vec.resize(len, 0);
            }

            let result = result_vec.as_mut_ptr().cast::<c_char>();
            to = result.wrapping_add(done);
            let mut tolen = len - done - 2;
            if call_iconv(
                fd,
                &raw mut from,
                &raw mut fromlen,
                &raw mut to,
                &raw mut tolen,
            ) != usize::MAX
            {
                unsafe {
                    *to = NUL;
                }
                done = crate::memory::ptr_offset_from(to, result) as usize;
                break;
            }

            let err = os_errno();
            if !vcp.vc_fail && unconvlenp.is_some() && err == ICONV_EINVAL {
                unsafe {
                    *to = NUL;
                }
                *unconvlenp.as_deref_mut().unwrap() = fromlen;
                done = crate::memory::ptr_offset_from(to, result) as usize;
                break;
            } else if !vcp.vc_fail && (err == ICONV_EILSEQ || err == ICONV_EINVAL) {
                unsafe {
                    *to = b'?' as c_char;
                }
                to = to.wrapping_add(1);
                if utf_ptr2cells(from) > 1 {
                    unsafe {
                        *to = b'?' as c_char;
                    }
                    to = to.wrapping_add(1);
                }
                let l = utfc_ptr2len_len(from, fromlen as i32) as usize;
                from = from.wrapping_add(l);
                fromlen -= l;
            } else if err != ICONV_E2BIG {
                return None;
            }
            done = crate::memory::ptr_offset_from(to, result) as usize;
        }

        result_vec.truncate(done);
        result_vec.push(0);
        if let Some(ref mut p) = resultlenp {
            **p = done;
        }
        Some(result_vec)
    }
}

pub fn f_iconv(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

    let str_ = tv_get_string(&argvars[0]);
    let mut buf1 = [0 as c_char; NUMBUFLEN];
    let from = enc_canonize(enc_skip(
        tv_get_string_buf(&argvars[1], buf1.as_mut_ptr()).cast_mut(),
    ));
    let mut buf2 = [0 as c_char; NUMBUFLEN];
    let to = enc_canonize(enc_skip(
        tv_get_string_buf(&argvars[2], buf2.as_mut_ptr()).cast_mut(),
    ));
    let mut vimconv = Vimconv {
        vc_type: CONV_NONE,
        vc_factor: 1,
        vc_fd: None,
        vc_fail: false,
    };
    convert_setup(
        &mut vimconv,
        from.as_ptr().cast_mut(),
        to.as_ptr().cast_mut(),
    );

    if vimconv.vc_type == CONV_NONE {
        crate::eval::typval::tv_set_string(rettv, str_);
    } else if let Some(conv_bytes) = string_convert(&vimconv, str_.cast_mut(), None) {
        crate::eval::typval::tv_set_string(rettv, conv_bytes.as_ptr().cast());
    }
}

pub fn convert_setup(
    vcp: &mut Vimconv,
    from: impl crate::strings::IntoCCharPtr,
    to: impl crate::strings::IntoCCharPtr,
) -> bool {
    let to = to.into_c_char_ptr().cast_mut();
    let from = from.into_c_char_ptr().cast_mut();
    convert_setup_ext(vcp, from, true, to, true)
}

pub fn convert_setup_ext(
    vcp: &mut Vimconv,
    from: impl crate::strings::IntoCCharPtr,
    from_unicode_is_utf8: bool,
    to: impl crate::strings::IntoCCharPtr,
    to_unicode_is_utf8: bool,
) -> bool {
    let from = from.into_c_char_ptr();
    let to = to.into_c_char_ptr();
    {
        *vcp = Vimconv {
            vc_type: CONV_NONE,
            vc_factor: 1,
            vc_fd: None,
            vc_fail: false,
        };

        if crate::memory::cstr_is_empty(from)
            || crate::memory::cstr_is_empty(to)
            || crate::memory::cstr_as_bytes(from) == crate::memory::cstr_as_bytes(to)
        {
            return true;
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
            vcp.vc_fd = my_iconv_open(to_arg, from_arg);
            if vcp.vc_fd.is_some() {
                vcp.vc_type = CONV_ICONV;
                vcp.vc_factor = 4;
            }
        }

        vcp.vc_type != CONV_NONE
    }
}

pub fn string_convert_slice(vcp: &Vimconv, bytes: &[u8], lenp: &mut usize) -> Option<Vec<u8>> {
    *lenp = bytes.len();
    string_convert(vcp, bytes.as_ptr().cast::<c_char>(), Some(lenp))
}

pub fn string_convert(
    vcp: &Vimconv,
    ptr: impl crate::strings::IntoCCharPtr,
    lenp: Option<&mut usize>,
) -> Option<Vec<u8>> {
    let ptr = ptr.into_c_char_ptr();
    string_convert_ext(vcp, ptr, lenp, None)
}

pub fn string_convert_ext(
    vcp: &Vimconv,
    ptr: impl crate::strings::IntoCCharPtr,
    mut lenp: Option<&mut usize>,
    mut unconvlenp: Option<&mut usize>,
) -> Option<Vec<u8>> {
    let ptr = ptr.into_c_char_ptr();
    {
        let len = if lenp.is_none() {
            crate::memory::cstr_len(ptr)
        } else {
            *lenp.as_deref_mut().unwrap()
        };
        if len == 0 {
            return Some(vec![0]);
        }

        match vcp.vc_type {
            CONV_TO_UTF8 => {
                let mut out: Vec<u8> = Vec::with_capacity(len * 2 + 1);
                for i in 0..len {
                    let c = unsafe { *ptr.wrapping_add(i).cast::<u8>() };
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
                let mut ch_buf = [0 as c_char; MB_MAXCHAR as usize];
                for i in 0..len {
                    let c = match unsafe { *ptr.wrapping_add(i).cast::<u8>() } {
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
                    let n = utf_char2bytes(c, ch_buf.as_mut_ptr()) as usize;
                    out.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(ch_buf.as_ptr().cast::<u8>(), n)
                    });
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
                    let l = utf_ptr2len_len(ptr.wrapping_add(i), (len - i) as i32);
                    if l == 0 {
                        out.push(0);
                    } else if l == 1 {
                        let b = unsafe { *ptr.wrapping_add(i).cast::<u8>() };
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
                        let mut c = utf_ptr2char(ptr.wrapping_add(i));
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
            CONV_ICONV => iconv_string(
                vcp,
                ptr,
                len,
                unconvlenp,
                (lenp.is_some()).then(|| &mut *lenp.unwrap()),
            ),
            _ => None,
        }
    }
}

fn cw_value(c: i32) -> i32 {
    let table = mbyte_st().cw_table.as_slice();
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
}

fn first_list_number(l: &TvList) -> Varnumber {
    unsafe {
        let li = tv_list_first(Some(l));
        (*li).li_tv.vval.v_number()
    }
}

pub fn f_setcellwidths(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        if argvars[0].v_type != VarType::VarList || argvars[0].vval.v_list().is_null() {
            emsg((&raw const E_LISTREQ).cast::<c_char>());
            return;
        }

        let l = argvars[0].vval.v_list();
        let table_size = tv_list_len(l.as_ref()) as usize;
        let mut table: Vec<CwInterval> = Vec::with_capacity(table_size);
        if table_size != 0 {
            let mut ptrs: Vec<*const TvList> = Vec::with_capacity(table_size);
            for (item, li) in (0_i32..).zip(crate::eval::typval::list_iter_mut((*l).lv_first)) {
                let li_tv = &li.li_tv;
                if li_tv.v_type != VarType::VarList || li_tv.vval.v_list().is_null() {
                    semsg!(E_LIST_ITEM_NR_IS_NOT_LIST, item);
                    return;
                }

                let li_l = li_tv.vval.v_list();
                ptrs.push(li_l);
                let mut lili: *const Listitem = tv_list_first(li_l.as_ref());
                let mut i = 0;
                let mut n1: Varnumber = 0;
                while !lili.is_null() {
                    let lili_tv = &(*lili).li_tv;
                    if lili_tv.v_type != VarType::VarNumber {
                        break;
                    }
                    if i == 0 {
                        n1 = lili_tv.vval.v_number();
                        if n1 < 0x80 {
                            emsg(E_ONLY_VALUES_OF_0X80_AND_HIGHER_SUPPORTED);
                            return;
                        }
                    } else if i == 1 && lili_tv.vval.v_number() < n1 {
                        semsg!(E_LIST_ITEM_NR_RANGE_INVALID, item);
                        return;
                    } else if i == 2 && (lili_tv.vval.v_number() < 1 || lili_tv.vval.v_number() > 2)
                    {
                        semsg!(E_LIST_ITEM_NR_CELL_WIDTH_INVALID, item);
                        return;
                    }
                    lili = (*lili).li_next;
                    i += 1;
                }

                if i != 3 {
                    semsg!(E_LIST_ITEM_NR_DOES_NOT_CONTAIN_3_NUMBERS, item);
                    return;
                }
            }

            ptrs.sort_by_key(|&a| first_list_number(&*a));

            for (idx, &li_l) in ptrs.iter().enumerate() {
                let lili0 = tv_list_first(li_l.as_ref());
                let n1 = (*lili0).li_tv.vval.v_number();
                if idx > 0 && n1 <= table[idx - 1].last {
                    semsg!(E_OVERLAPPING_RANGES_FOR_NR, n1 as usize);
                    return;
                }
                let lili1 = (*lili0).li_next;
                let lili2 = (*lili1).li_next;
                table.push(CwInterval {
                    first: n1,
                    last: (*lili1).li_tv.vval.v_number(),
                    width: (*lili2).li_tv.vval.v_number() as c_char,
                });
            }
        }

        let cw_table_save = std::mem::replace(&mut mbyte_st().cw_table, table);

        let error = check_chars_options();
        if !error.is_null() {
            emsg(error);
            mbyte_st().cw_table = cw_table_save;
            return;
        }

        changed_window_setting_all();
        redraw_all_later(UPD_NOT_VALID);
    }
}

pub fn f_getcellwidths(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let table = mbyte_st().cw_table.as_slice();
    tv_list_alloc_ret(rettv, table.len() as isize);
    for &entry_data in table {
        let entry = tv_list_alloc(3);
        tv_list_append_number(&mut *entry, entry_data.first);
        tv_list_append_number(&mut *entry, entry_data.last);
        tv_list_append_number(&mut *entry, Varnumber::from(entry_data.width));
        tv_list_append_list(rettv.vval.v_list_ref(), entry);
    }
}

pub fn f_charclass(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if !tv_check_for_string_arg(argvars, 0) || argvars[0].vval.v_string().is_null() {
        return;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(mb_get_class(
        argvars[0].vval.v_string(),
    )));
}

pub fn get_encoding_name(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if idx < 0 || (idx as usize) >= ENC_CANON_TABLE.len() {
        return ptr::null_mut();
    }
    ENC_CANON_TABLE[idx as usize].name.as_ptr().cast_mut()
}

pub fn mb_strcmp_ic(
    ic: bool,
    s1: impl crate::strings::IntoCCharPtr,
    s2: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let s1 = s1.into_c_char_ptr();
    let s2 = s2.into_c_char_ptr();
    {
        if ic {
            mb_stricmp(s1, s2)
        } else {
            let b1 = crate::memory::cstr_as_bytes(s1);
            let b2 = crate::memory::cstr_as_bytes(s2);
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
    use crate::grid::schar_get;

    #[test]
    fn utf8_char_encoding_and_bounds_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let mut buf = [0 as c_char; 8];
            let len = utf_char2bytes(0x20ac, buf.as_mut_ptr());
            assert_eq!(len, 3);
            assert_eq!(utf_ptr2len(buf.as_ptr()), 3);
            assert_eq!(utf_ptr2char(buf.as_ptr()), 0x20ac);
            assert!(utf_valid_string(buf.as_ptr(), ptr::null()));
            assert!(utf_printable(0x20ac));
            assert!(!utf_printable(0xd800));
            assert_eq!(enc_canon_props(c"utf-8"), ENC_UNICODE);

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
                assert_eq!(utf_ptr2cells(s.as_ptr()), exp_cells);
                assert_eq!(
                    mb_string2cells_len(s.as_ptr(), s.to_bytes().len()),
                    exp_cells as usize
                );
            }
            assert_eq!(utf_ptr2cells(c"\xff"), 4);

            let mut firstc: i32 = 0;
            let mut sbuf = [0u8; 32];
            let sc = crate::grid::unittest_schar_from_cluster(c")\xcc\x80\xcc\x81", &mut firstc);
            schar_get(sbuf.as_mut_ptr(), sc);
            assert_eq!(firstc, 0x29);
            assert_eq!(
                CStr::from_bytes_until_nul(&sbuf).unwrap().to_bytes(),
                b")\xcc\x80\xcc\x81"
            );

            let emoji = c"🏳️‍⚧️🧑‍🌾❤️😂🏴‍☠️";
            let e_bytes = emoji.to_bytes();
            let mut pos = 0usize;
            while pos < e_bytes.len() {
                let clen = utfc_ptr2len(emoji.as_ptr().wrapping_add(pos)) as usize;
                for p in pos..(pos + clen) {
                    assert_eq!(
                        utf_head_off(emoji.as_ptr(), emoji.as_ptr().wrapping_add(p)),
                        (p - pos) as i32
                    );
                }
                pos += clen;
            }

            let cp_str = c"iÀⱠ𐀀i";
            let b0 = utf_cp_bounds_len(cp_str.as_ptr(), cp_str.as_ptr().wrapping_add(2), 9);
            assert_eq!((b0.begin_off, b0.end_off), (1, 1));
        }
    }
}
