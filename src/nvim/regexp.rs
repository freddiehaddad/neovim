//! Regular expression compilation, execution, and substitution (`src/nvim/regexp.c`).

use crate::buffer::FileBuf;
pub(super) use crate::charset::{ascii_isdigit, ascii_iswhite, ascii_isxdigit};
use crate::charset::{
    getdigits_int, hex2nr, vim_is_idc, vim_isfilec, vim_isprintc, vim_iswordc_buf, vim_iswordp_buf,
};
use crate::cursor::{Colnr, Linenr, Lpos, lt};
use crate::errors::*;
use crate::eval::eval_to_string;
use crate::eval::typval::{
    StaticList10, TvList, TypVal, VarType, tv_clear, tv_list_alloc, tv_list_append_string,
    tv_list_first, tv_list_init_static10, tv_set_string,
};
use crate::eval::userfunc::{Funcexe, UserFunc};
use crate::globals::*;
use crate::keycodes::{BS, CAR, ESC, K_SPECIAL, NL, NUL, TAB};
use crate::mark::mark_get;
use crate::mbyte::{
    GraphemeState, mb_get_class_tab, mb_islower, mb_isupper, mb_ptr_back, mb_ptr2char_adv,
    mb_strnicmp, mb_tolower, mb_toupper, utf_char2bytes, utf_char2len, utf_composinglike, utf_fold,
    utf_head_off, utf_iscomposing_legacy, utf_ptr2char, utf_ptr2len, utf_strnicmp, utfc_ptr2len,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::message::{emsg, iemsg, internal_error, msg_puts, semsg, verbose_enter, verbose_leave};
use crate::option_vars::*;
use crate::os::input::fast_breakcheck;
use crate::plines::{getvvcol, win_linetabsize};
use crate::profile::{Proftime, profile_passed_limit};
pub(super) use crate::strings::tr;
use crate::window::WinInfoInfo;
use std::ffi::{CStr, c_char};
use std::ptr;

#[path = "regexp_bt.rs"]
mod bt;
#[path = "regexp_nfa.rs"]
mod nfa;

use self::bt::*;
use self::nfa::*;

use crate::math::{FAIL, OK};
use crate::option::OptInt;
pub type RegMagic = i32;
pub type RegGetlineFlags = u32;

pub const NSUBEXP: usize = 10;

pub const RE_MAGIC: i32 = 1;
pub const RE_STRING: i32 = 2;
pub const RE_STRICT: i32 = 4;
pub const RE_AUTO: i32 = 8;
pub const RE_NOBREAK: i32 = 16;

pub const REX_SET: i32 = 1;
pub const REX_USE: i32 = 2;
pub const REX_ALL: i32 = REX_SET | REX_USE;

pub const MAGIC_NONE: RegMagic = 1;
pub const MAGIC_OFF: RegMagic = 2;
pub const MAGIC_ON: RegMagic = 3;
pub const MAGIC_ALL: RegMagic = 4;

pub const REGSUB_COPY: i32 = 1;
pub const REGSUB_MAGIC: i32 = 2;
pub const REGSUB_BACKSLASH: i32 = 4;

pub(super) const CTRL_H_CONST: i32 = 0x08;
pub(super) const CTRL_V_CONST: i32 = 0x16;

pub(super) const MAXCOL: Colnr = 0x7fffffff;
pub(super) const NUMBUFLEN: usize = 65;
pub(super) const MB_MAXBYTES: i32 = 30;
pub(super) const MAX_MCO: usize = 6;
pub(super) const GRAPHEME_STATE_INIT: GraphemeState = 0;
pub(super) const K_MARK_BUF_LOCAL: i32 = 0;
pub(super) const K_CPO_LITERAL: i32 = b'l' as i32;

pub(super) const RGLF_LINE: RegGetlineFlags = 0x01;
pub(super) const RGLF_LENGTH: RegGetlineFlags = 0x02;
pub(super) const RGLF_SUBMATCH: RegGetlineFlags = 0x04;

pub(super) const NFA_MAX_STATES: i32 = 100000;
pub(super) const NFA_TOO_EXPENSIVE: i32 = -1;

pub(super) const AUTOMATIC_ENGINE: i32 = 0;
pub(super) const BACKTRACKING_ENGINE: i32 = 1;
pub(super) const NFA_ENGINE: i32 = 2;

pub(super) const REGMAGIC: i32 = 0o234;
pub(super) const MAX_LIMIT: i32 = 32767 << 16;

pub(super) const NOT_MULTI: i32 = 0;
pub(super) const MULTI_ONE: i32 = 1;
pub(super) const MULTI_MULT: i32 = 2;

pub(super) const RA_FAIL: i32 = 1;
pub(super) const RA_CONT: i32 = 2;
pub(super) const RA_BREAK: i32 = 3;
pub(super) const RA_MATCH: i32 = 4;
pub(super) const RA_NOMATCH: i32 = 5;

pub(super) const CLASS_ALNUM: i32 = 0;
pub(super) const CLASS_ALPHA: i32 = 1;
pub(super) const CLASS_BLANK: i32 = 2;
pub(super) const CLASS_CNTRL: i32 = 3;
pub(super) const CLASS_DIGIT: i32 = 4;
pub(super) const CLASS_GRAPH: i32 = 5;
pub(super) const CLASS_LOWER: i32 = 6;
pub(super) const CLASS_PRINT: i32 = 7;
pub(super) const CLASS_PUNCT: i32 = 8;
pub(super) const CLASS_SPACE: i32 = 9;
pub(super) const CLASS_UPPER: i32 = 10;
pub(super) const CLASS_XDIGIT: i32 = 11;
pub(super) const CLASS_TAB: i32 = 12;
pub(super) const CLASS_RETURN: i32 = 13;
pub(super) const CLASS_BACKSPACE: i32 = 14;
pub(super) const CLASS_ESCAPE: i32 = 15;
pub(super) const CLASS_IDENT: i32 = 16;
pub(super) const CLASS_KEYWORD: i32 = 17;
pub(super) const CLASS_FNAME: i32 = 18;
pub(super) const CLASS_NONE: i32 = 99;

pub(super) const RI_DIGIT: i16 = 0x01;
pub(super) const RI_HEX: i16 = 0x02;
pub(super) const RI_OCTAL: i16 = 0x04;
pub(super) const RI_WORD: i16 = 0x08;
pub(super) const RI_HEAD: i16 = 0x10;
pub(super) const RI_ALPHA: i16 = 0x20;
pub(super) const RI_LOWER: i16 = 0x40;
pub(super) const RI_UPPER: i16 = 0x80;
pub(super) const RI_WHITE: i16 = 0x100;

pub(super) const RF_ICASE: u32 = 1;
pub(super) const RF_NOICASE: u32 = 2;
pub(super) const RF_HASNL: u32 = 4;
pub(super) const RF_ICOMBINE: u32 = 8;
pub(super) const RF_LOOKBH: u32 = 16;

pub(super) const REG_NOPAREN: i32 = 0;
pub(super) const REG_PAREN: i32 = 1;
pub(super) const REG_ZPAREN: i32 = 2;
pub(super) const REG_NPAREN: i32 = 3;

pub(super) const REG_MAX_PAREN_DEPTH: i32 = 1000;
pub(super) const MAX_REGSUB_NESTING: usize = 4;

pub(super) const CLASS_NOT: i32 = 0x80;
pub(super) const CLASS_AF_CONST: i32 = 0x40;
pub(super) const CLASS_AF: i32 = 0x20;
pub(super) const CLASS_AZ_CONST: i32 = 0x10;
pub(super) const CLASS_AZ: i32 = 0x08;
pub(super) const CLASS_O7: i32 = 0x04;
pub(super) const CLASS_O9: i32 = 0x02;
pub(super) const CLASS_UNDERSCORE: i32 = 0x01;

pub(super) const REGEXP_INRANGE: &CStr = c"]^-n\\";
pub(super) const REGEXP_ABBR: &CStr = c"nrtebdoxuU";
pub(super) const CLASSCHARS: &[u8; 28] = b".iIkKfFpPsSdDxXoOwWhHaAlLuU\0";

pub(super) const E_INVALID_CHARACTER_AFTER_STR_AT: &CStr = c"E59: Invalid character after %s@";
pub(super) const E_INVALID_USE_OF_UNDERSCORE: &CStr = c"E63: Invalid use of \\_";
pub(super) const E_PATTERN_USES_MORE_MEMORY_THAN_MAXMEMPATTERN: &CStr =
    c"E363: Pattern uses more memory than 'maxmempattern'";
pub(super) const E_INVALID_ITEM_IN_STR_BRACKETS: &CStr = c"E369: Invalid item in %s%%[]";
pub(super) const E_MISSING_DELIMITER_AFTER_SEARCH_PATTERN_STR: &CStr =
    c"E654: Missing delimiter after search pattern: %s";
pub(super) const E_MISSINGBRACKET: &CStr = c"E769: Missing ] after %s[";
pub(super) const E_REVERSE_RANGE: &CStr = c"E944: Reverse range in character class";
pub(super) const E_LARGE_CLASS: &CStr = c"E945: Range too large in character class";
pub(super) const E_UNMATCHED_Z: &CStr = c"E52: Unmatched \\z(";
pub(super) const E_UNMATCHED_STR_PERCENT_OPEN: &CStr = c"E53: Unmatched %s%%(";
pub(super) const E_UNMATCHED_STR_OPEN: &CStr = c"E54: Unmatched %s(";
pub(super) const E_UNMATCHED_STR_CLOSE: &CStr = c"E55: Unmatched %s)";
pub(super) const E_Z_NOT_ALLOWED: &CStr = c"E66: \\z( not allowed here";
pub(super) const E_Z1_NOT_ALLOWED: &CStr = c"E67: \\z1 - \\z9 not allowed here";
pub(super) const E_MISSING_SB: &CStr = c"E69: Missing ] after %s%%[";
pub(super) const E_EMPTY_SB: &CStr = c"E70: Empty %s%%[]";
pub(super) const E_RECURSIVE: &CStr = c"E956: Cannot use pattern recursively";
pub(super) const E_REGEXP_NUMBER_AFTER_DOT_POS_SEARCH_CHR: &CStr =
    c"E1204: No Number allowed after .: '\\%%%c'";
pub(super) const E_NFA_REGEXP_MISSING_VALUE_IN_CHR: &CStr =
    c"E1273: (NFA regexp) missing value in '\\%%%c'";
pub(super) const E_ATOM_ENGINE_MUST_BE_AT_START_OF_PATTERN: &CStr =
    c"E1281: Atom '\\%%#=%c' must be at the start of the pattern";
pub(super) const E_SUBSTITUTE_NESTING_TOO_DEEP: &CStr = c"E1290: substitute nesting too deep";
pub(super) const E_UNICODE_VAL_TOO_LARGE: &CStr =
    c"E1541: Value too large, max Unicode codepoint is U+10FFFF";

#[rustfmt::skip]
pub(super) static META_FLAGS: [u8; 127] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
//                 %  &     (  )  *  +        .
    0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 0, 1, 0,
//     1  2  3  4  5  6  7  8  9        <  =  >  ?
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1,
//  @  A     C  D     F     H  I     K  L  M     O
    1, 1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 1, 1, 0, 1,
//  P        S     U  V  W  X     Z  [           _
    1, 0, 0, 1, 0, 1, 1, 1, 1, 0, 1, 1, 0, 0, 0, 1,
//     a     c  d     f     h  i     k  l  m  n  o
    0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 1, 1, 1, 1,
//  p        s     u  v  w  x     z  {  |     ~
    1, 0, 0, 1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1,
];

pub(super) enum RegProgEngine {
    Bt(BtRegprog),
    Nfa(NfaRegprog),
}

pub struct Regprog {
    pub regflags: u32,
    pub re_engine: u32,
    pub re_flags: u32,
    pub re_in_use: bool,
    pub(super) engine: RegProgEngine,
}

impl Regprog {
    #[inline]
    pub(super) fn bt_mut(&mut self) -> &mut BtRegprog {
        match &mut self.engine {
            RegProgEngine::Bt(bt) => bt,
            RegProgEngine::Nfa(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn nfa_mut(&mut self) -> &mut NfaRegprog {
        match &mut self.engine {
            RegProgEngine::Nfa(nfa) => nfa,
            RegProgEngine::Bt(_) => unreachable!(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Regmmatch {
    pub regprog: *mut Regprog,
    pub startpos: [Lpos; NSUBEXP],
    pub endpos: [Lpos; NSUBEXP],
    pub rmm_matchcol: Colnr,
    pub rmm_ic: i32,
    pub rmm_maxcol: Colnr,
}

impl Regmmatch {
    pub const fn new() -> Self {
        Self {
            regprog: ptr::null_mut(),
            startpos: [Lpos::new(); NSUBEXP],
            endpos: [Lpos::new(); NSUBEXP],
            rmm_matchcol: 0,
            rmm_ic: 0,
            rmm_maxcol: 0,
        }
    }
}

impl Default for Regmmatch {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Regmatch {
    pub regprog: *mut Regprog,
    pub startp: [*mut c_char; NSUBEXP],
    pub endp: [*mut c_char; NSUBEXP],
    pub rm_matchcol: Colnr,
    pub rm_ic: bool,
}

impl Regmatch {
    pub const fn new() -> Self {
        Self {
            regprog: ptr::null_mut(),
            startp: [ptr::null_mut(); NSUBEXP],
            endp: [ptr::null_mut(); NSUBEXP],
            rm_matchcol: 0,
            rm_ic: false,
        }
    }
}

impl Default for Regmatch {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Default)]
pub struct RegExtmatch {
    pub matches: [Option<std::ffi::CString>; NSUBEXP],
}

#[inline]
pub(super) fn cstring_from_bytes(
    ptr: impl crate::strings::IntoCCharPtr,
    len: usize,
) -> Option<std::ffi::CString> {
    let ptr = ptr.into_c_char_ptr();
    let bytes = { crate::memory::cstr_slice(ptr, len) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(len);
    std::ffi::CString::new(&bytes[..end]).ok()
}

pub(super) struct BtRegprog {
    pub regflags: u32,
    pub regstart: i32,
    pub reganch: u8,
    pub regmust_off: Option<usize>,
    pub regmlen: i32,
    pub reghasz: u8,
    pub program: Vec<u8>,
}

#[derive(Clone, Copy)]
pub(super) enum NfaOutSlot {
    Out(usize),
    Out1(usize),
}

impl NfaOutSlot {
    #[inline]
    pub(super) fn set(self, states: &mut [NfaState], target: Option<usize>) {
        match self {
            Self::Out(idx) => states[idx].out = target,
            Self::Out1(idx) => states[idx].out1 = target,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct NfaState {
    pub c: i32,
    pub out: Option<usize>,
    pub out1: Option<usize>,
    pub lastlist: [i32; 2],
    pub val: i32,
}

impl NfaState {
    pub(super) const fn new() -> Self {
        Self {
            c: 0,
            out: None,
            out1: None,
            lastlist: [0; 2],
            val: 0,
        }
    }
}

impl Default for NfaState {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) struct NfaRegprog {
    pub regflags: u32,
    pub re_engine: u32,
    pub start: Option<usize>,
    pub reganch: i32,
    pub regstart: i32,
    pub match_text: Option<std::ffi::CString>,
    pub has_zend: i32,
    pub has_backref: i32,
    pub reghasz: i32,
    pub pattern: std::ffi::CString,
    pub nsubexp: i32,
    pub nstate: i32,
    pub listbuf: [Vec<NfaThread>; 2],
    pub state: Vec<NfaState>,
}

#[derive(Clone, Copy)]
pub(super) enum RegPos {
    Ptr(*mut u8),
    Pos(Lpos),
}

impl RegPos {
    pub(super) const fn new() -> Self {
        Self::Ptr(ptr::null_mut())
    }

    #[inline]
    pub(super) const fn ptr(self) -> *mut u8 {
        match self {
            Self::Ptr(p) => p,
            Self::Pos(_) => ptr::null_mut(),
        }
    }

    #[inline]
    pub(super) fn ptr_mut(&mut self) -> &mut *mut u8 {
        if !matches!(*self, Self::Ptr(_)) {
            *self = Self::Ptr(ptr::null_mut());
        }
        match self {
            Self::Ptr(p) => p,
            Self::Pos(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) const fn pos(self) -> Lpos {
        match self {
            Self::Pos(pos) => pos,
            Self::Ptr(_) => Lpos::new(),
        }
    }

    #[inline]
    pub(super) fn pos_mut(&mut self) -> &mut Lpos {
        if !matches!(*self, Self::Pos(_)) {
            *self = Self::Pos(Lpos::new());
        }
        match self {
            Self::Pos(pos) => pos,
            Self::Ptr(_) => unreachable!(),
        }
    }
}

impl Default for RegPos {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Regsave {
    pub rs_u: RegPos,
    pub rs_len: i32,
}

impl Regsave {
    pub(super) const fn new() -> Self {
        Self {
            rs_u: RegPos::new(),
            rs_len: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct SaveSe {
    pub se_u: RegPos,
}

impl SaveSe {
    pub(super) const fn new() -> Self {
        Self {
            se_u: RegPos::new(),
        }
    }
}

pub(super) type Regstate = i32;
pub(super) const RS_NOPEN: Regstate = 0;
pub(super) const RS_MOPEN: Regstate = 1;
pub(super) const RS_MCLOSE: Regstate = 2;
pub(super) const RS_ZOPEN: Regstate = 3;
pub(super) const RS_ZCLOSE: Regstate = 4;
pub(super) const RS_BRANCH: Regstate = 5;
pub(super) const RS_BRCPLX_MORE: Regstate = 6;
pub(super) const RS_BRCPLX_LONG: Regstate = 7;
pub(super) const RS_BRCPLX_SHORT: Regstate = 8;
pub(super) const RS_NOMATCH: Regstate = 9;
pub(super) const RS_BEHIND1: Regstate = 10;
pub(super) const RS_BEHIND2: Regstate = 11;
pub(super) const RS_STAR_LONG: Regstate = 12;
pub(super) const RS_STAR_SHORT: Regstate = 13;

#[derive(Clone, Copy)]
pub(super) enum RegItemSave {
    Se(SaveSe),
    Reg(Regsave),
}

impl RegItemSave {
    pub(super) const fn new() -> Self {
        Self::Se(SaveSe::new())
    }

    #[inline]
    pub(super) fn sesave(&self) -> &SaveSe {
        match self {
            Self::Se(s) => s,
            Self::Reg(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn sesave_mut(&mut self) -> &mut SaveSe {
        if !matches!(*self, Self::Se(_)) {
            *self = Self::Se(SaveSe::new());
        }
        match self {
            Self::Se(s) => s,
            Self::Reg(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn regsave(&self) -> &Regsave {
        match self {
            Self::Reg(r) => r,
            Self::Se(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn regsave_mut(&mut self) -> &mut Regsave {
        if !matches!(*self, Self::Reg(_)) {
            *self = Self::Reg(Regsave::new());
        }
        match self {
            Self::Reg(r) => r,
            Self::Se(_) => unreachable!(),
        }
    }
}

impl Default for RegItemSave {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Regitem {
    pub rs_state: Regstate,
    pub rs_no: i16,
    pub rs_scan: *mut u8,
    pub rs_un: RegItemSave,
}

impl Regitem {
    pub(super) const fn new() -> Self {
        Self {
            rs_state: 0,
            rs_no: 0,
            rs_scan: ptr::null_mut(),
            rs_un: RegItemSave::new(),
        }
    }
}

impl Default for Regitem {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Regbehind {
    pub save_after: Regsave,
    pub save_behind: Regsave,
    pub save_need_clear_subexpr: i32,
    pub save_start: [SaveSe; NSUBEXP],
    pub save_end: [SaveSe; NSUBEXP],
}

pub(super) struct Frag {
    pub start: Option<usize>,
    pub out: Vec<NfaOutSlot>,
}

impl Frag {
    pub(super) const fn new() -> Self {
        Self {
            start: None,
            out: Vec::new(),
        }
    }
}

impl Default for Frag {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct MultiPos {
    pub start_lnum: Linenr,
    pub end_lnum: Linenr,
    pub start_col: Colnr,
    pub end_col: Colnr,
}

impl MultiPos {
    pub(super) const fn new() -> Self {
        Self {
            start_lnum: 0,
            end_lnum: 0,
            start_col: 0,
            end_col: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct LinePos {
    pub start: *mut u8,
    pub end: *mut u8,
}

impl LinePos {
    pub(super) const fn new() -> Self {
        Self {
            start: ptr::null_mut(),
            end: ptr::null_mut(),
        }
    }
}

impl Default for LinePos {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub(super) struct RegSubList {
    raw: [[u64; 2]; NSUBEXP],
}

impl RegSubList {
    pub(super) const fn new() -> Self {
        Self {
            raw: [[0; 2]; NSUBEXP],
        }
    }

    #[inline]
    pub(super) fn multi(&self) -> &[MultiPos; NSUBEXP] {
        unsafe { &*(&raw const self.raw).cast::<[MultiPos; NSUBEXP]>() }
    }

    #[inline]
    pub(super) fn multi_mut(&mut self) -> &mut [MultiPos; NSUBEXP] {
        unsafe { &mut *(&raw mut self.raw).cast::<[MultiPos; NSUBEXP]>() }
    }

    #[inline]
    pub(super) fn line(&self) -> &[LinePos; NSUBEXP] {
        unsafe { &*(&raw const self.raw).cast::<[LinePos; NSUBEXP]>() }
    }

    #[inline]
    pub(super) fn line_mut(&mut self) -> &mut [LinePos; NSUBEXP] {
        unsafe { &mut *(&raw mut self.raw).cast::<[LinePos; NSUBEXP]>() }
    }

    #[inline]
    pub(super) fn copy_first(&mut self, other: &Self) {
        self.raw[0] = other.raw[0];
    }

    #[inline]
    pub(super) fn copy_n(&mut self, other: &Self, n: usize) {
        let n = n.min(NSUBEXP);
        self.raw[..n].copy_from_slice(&other.raw[..n]);
    }

    #[inline]
    pub(super) fn copy_range_from_1(&mut self, other: &Self, n: usize) {
        let n = n.min(NSUBEXP);
        self.raw[1..n].copy_from_slice(&other.raw[1..n]);
    }
}

impl Default for RegSubList {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Regsub {
    pub in_use: i32,
    pub list: RegSubList,
    pub orig_start_col: Colnr,
}

impl Regsub {
    pub(super) const fn new() -> Self {
        Self {
            in_use: 0,
            list: RegSubList::new(),
            orig_start_col: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Regsubs {
    pub norm: Regsub,
    pub synt: Regsub,
}

impl Regsubs {
    pub(super) const fn new() -> Self {
        Self {
            norm: Regsub::new(),
            synt: Regsub::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct NfaPim {
    pub result: i32,
    pub state: Option<usize>,
    pub subs: Regsubs,
    pub end: RegPos,
}

impl NfaPim {
    pub(super) const fn new() -> Self {
        Self {
            result: 0,
            state: None,
            subs: Regsubs::new(),
            end: RegPos::new(),
        }
    }
}

impl Default for NfaPim {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub(super) struct NfaThread {
    pub state: Option<usize>,
    pub count: i32,
    pub pim: NfaPim,
    pub subs: Regsubs,
}

impl NfaThread {
    pub(super) const fn new() -> Self {
        Self {
            state: None,
            count: 0,
            pim: NfaPim::new(),
            subs: Regsubs::new(),
        }
    }
}

impl Default for NfaThread {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub(super) struct NfaList {
    pub t: Vec<NfaThread>,
    pub n: i32,
    pub id: i32,
    pub has_pim: i32,
}

#[derive(Clone, Copy)]
enum CaseConv {
    Upper,
    Lower,
}

impl CaseConv {
    #[inline]
    fn apply(self, c: i32) -> i32 {
        match self {
            Self::Upper => mb_toupper(c),
            Self::Lower => mb_tolower(c),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ParseState {
    pub regparse: *mut c_char,
    pub prevchr_len: i32,
    pub curchr: i32,
    pub prevchr: i32,
    pub prevprevchr: i32,
    pub nextchr: i32,
    pub at_start: i32,
    pub prev_at_start: i32,
    pub regnpar: i32,
}

impl ParseState {
    pub(super) const fn new() -> Self {
        Self {
            regparse: ptr::null_mut(),
            prevchr_len: 0,
            curchr: 0,
            prevchr: 0,
            prevprevchr: 0,
            nextchr: 0,
            at_start: 0,
            prev_at_start: 0,
            regnpar: 0,
        }
    }
}

impl Default for ParseState {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Regexec {
    pub reg_match: *mut Regmatch,
    pub reg_mmatch: *mut Regmmatch,
    pub reg_startp: *mut *mut u8,
    pub reg_endp: *mut *mut u8,
    pub reg_startpos: *mut Lpos,
    pub reg_endpos: *mut Lpos,
    pub reg_win: *mut WinInfoInfo,
    pub reg_buf: *mut FileBuf,
    pub reg_firstlnum: Linenr,
    pub reg_maxline: Linenr,
    pub reg_line_lbr: bool,
    pub lnum: Linenr,
    pub line: *mut u8,
    pub input: *mut u8,
    pub need_clear_subexpr: i32,
    pub need_clear_zsubexpr: i32,
    pub reg_ic: bool,
    pub reg_icombine: bool,
    pub reg_nobreak: bool,
    pub reg_maxcol: Colnr,
    pub nfa_has_zend: i32,
    pub nfa_has_backref: i32,
    pub nfa_nsubexpr: i32,
    pub nfa_listid: i32,
    pub nfa_alt_listid: i32,
    pub nfa_has_zsubexpr: i32,
}

impl Regexec {
    pub(super) const fn new() -> Self {
        Self {
            reg_match: ptr::null_mut(),
            reg_mmatch: ptr::null_mut(),
            reg_startp: ptr::null_mut(),
            reg_endp: ptr::null_mut(),
            reg_startpos: ptr::null_mut(),
            reg_endpos: ptr::null_mut(),
            reg_win: ptr::null_mut(),
            reg_buf: ptr::null_mut(),
            reg_firstlnum: 0,
            reg_maxline: 0,
            reg_line_lbr: false,
            lnum: 0,
            line: ptr::null_mut(),
            input: ptr::null_mut(),
            need_clear_subexpr: 0,
            need_clear_zsubexpr: 0,
            reg_ic: false,
            reg_icombine: false,
            reg_nobreak: false,
            reg_maxcol: 0,
            nfa_has_zend: 0,
            nfa_has_backref: 0,
            nfa_nsubexpr: 0,
            nfa_listid: 0,
            nfa_alt_listid: 0,
            nfa_has_zsubexpr: 0,
        }
    }
}

impl Default for Regexec {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Regsubmatch {
    pub sm_match: *mut Regmatch,
    pub sm_mmatch: *mut Regmmatch,
    pub sm_firstlnum: Linenr,
    pub sm_maxline: Linenr,
    pub sm_line_lbr: i32,
}

impl Regsubmatch {
    pub(super) const fn new() -> Self {
        Self {
            sm_match: ptr::null_mut(),
            sm_mmatch: ptr::null_mut(),
            sm_firstlnum: 0,
            sm_maxline: 0,
            sm_line_lbr: 0,
        }
    }
}

impl Default for Regsubmatch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct Decomp {
    a: i32,
    b: i32,
    c: i32,
}

static DECOMP_TABLE: [Decomp; 0xfb4f - 0xfb20 + 1] = [
    Decomp {
        a: 0x5e2,
        b: 0,
        c: 0,
    }, // 0xfb20       alt ayin
    Decomp {
        a: 0x5d0,
        b: 0,
        c: 0,
    }, // 0xfb21       alt alef
    Decomp {
        a: 0x5d3,
        b: 0,
        c: 0,
    }, // 0xfb22       alt dalet
    Decomp {
        a: 0x5d4,
        b: 0,
        c: 0,
    }, // 0xfb23       alt he
    Decomp {
        a: 0x5db,
        b: 0,
        c: 0,
    }, // 0xfb24       alt kaf
    Decomp {
        a: 0x5dc,
        b: 0,
        c: 0,
    }, // 0xfb25       alt lamed
    Decomp {
        a: 0x5dd,
        b: 0,
        c: 0,
    }, // 0xfb26       alt mem-sofit
    Decomp {
        a: 0x5e8,
        b: 0,
        c: 0,
    }, // 0xfb27       alt resh
    Decomp {
        a: 0x5ea,
        b: 0,
        c: 0,
    }, // 0xfb28       alt tav
    Decomp {
        a: b'+' as i32,
        b: 0,
        c: 0,
    }, // 0xfb29       alt plus
    Decomp {
        a: 0x5e9,
        b: 0x5c1,
        c: 0,
    }, // 0xfb2a       shin+shin-dot
    Decomp {
        a: 0x5e9,
        b: 0x5c2,
        c: 0,
    }, // 0xfb2b       shin+sin-dot
    Decomp {
        a: 0x5e9,
        b: 0x5c1,
        c: 0x5bc,
    }, // 0xfb2c       shin+shin-dot+dagesh
    Decomp {
        a: 0x5e9,
        b: 0x5c2,
        c: 0x5bc,
    }, // 0xfb2d       shin+sin-dot+dagesh
    Decomp {
        a: 0x5d0,
        b: 0x5b7,
        c: 0,
    }, // 0xfb2e       alef+patah
    Decomp {
        a: 0x5d0,
        b: 0x5b8,
        c: 0,
    }, // 0xfb2f       alef+qamats
    Decomp {
        a: 0x5d0,
        b: 0x5b4,
        c: 0,
    }, // 0xfb30       alef+hiriq
    Decomp {
        a: 0x5d1,
        b: 0x5bc,
        c: 0,
    }, // 0xfb31       bet+dagesh
    Decomp {
        a: 0x5d2,
        b: 0x5bc,
        c: 0,
    }, // 0xfb32       gimel+dagesh
    Decomp {
        a: 0x5d3,
        b: 0x5bc,
        c: 0,
    }, // 0xfb33       dalet+dagesh
    Decomp {
        a: 0x5d4,
        b: 0x5bc,
        c: 0,
    }, // 0xfb34       he+dagesh
    Decomp {
        a: 0x5d5,
        b: 0x5bc,
        c: 0,
    }, // 0xfb35       vav+dagesh
    Decomp {
        a: 0x5d6,
        b: 0x5bc,
        c: 0,
    }, // 0xfb36       zayin+dagesh
    Decomp {
        a: 0xfb37,
        b: 0,
        c: 0,
    }, // 0xfb37 -- UNUSED
    Decomp {
        a: 0x5d8,
        b: 0x5bc,
        c: 0,
    }, // 0xfb38       tet+dagesh
    Decomp {
        a: 0x5d9,
        b: 0x5bc,
        c: 0,
    }, // 0xfb39       yud+dagesh
    Decomp {
        a: 0x5da,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3a       kaf sofit+dagesh
    Decomp {
        a: 0x5db,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3b       kaf+dagesh
    Decomp {
        a: 0x5dc,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3c       lamed+dagesh
    Decomp {
        a: 0xfb3d,
        b: 0,
        c: 0,
    }, // 0xfb3d -- UNUSED
    Decomp {
        a: 0x5de,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3e       mem+dagesh
    Decomp {
        a: 0xfb3f,
        b: 0,
        c: 0,
    }, // 0xfb3f -- UNUSED
    Decomp {
        a: 0x5e0,
        b: 0x5bc,
        c: 0,
    }, // 0xfb40       nun+dagesh
    Decomp {
        a: 0x5e1,
        b: 0x5bc,
        c: 0,
    }, // 0xfb41       samech+dagesh
    Decomp {
        a: 0xfb42,
        b: 0,
        c: 0,
    }, // 0xfb42 -- UNUSED
    Decomp {
        a: 0x5e3,
        b: 0x5bc,
        c: 0,
    }, // 0xfb43       pe sofit+dagesh
    Decomp {
        a: 0x5e4,
        b: 0x5bc,
        c: 0,
    }, // 0xfb44       pe+dagesh
    Decomp {
        a: 0xfb45,
        b: 0,
        c: 0,
    }, // 0xfb45 -- UNUSED
    Decomp {
        a: 0x5e6,
        b: 0x5bc,
        c: 0,
    }, // 0xfb46       tsadi+dagesh
    Decomp {
        a: 0x5e7,
        b: 0x5bc,
        c: 0,
    }, // 0xfb47       qof+dagesh
    Decomp {
        a: 0x5e8,
        b: 0x5bc,
        c: 0,
    }, // 0xfb48       resh+dagesh
    Decomp {
        a: 0x5e9,
        b: 0x5bc,
        c: 0,
    }, // 0xfb49       shin+dagesh
    Decomp {
        a: 0x5ea,
        b: 0x5bc,
        c: 0,
    }, // 0xfb4a       tav+dagesh
    Decomp {
        a: 0x5d5,
        b: 0x5b9,
        c: 0,
    }, // 0xfb4b       vav+holam
    Decomp {
        a: 0x5d1,
        b: 0x5bf,
        c: 0,
    }, // 0xfb4c       bet+rafe
    Decomp {
        a: 0x5db,
        b: 0x5bf,
        c: 0,
    }, // 0xfb4d       kaf+rafe
    Decomp {
        a: 0x5e4,
        b: 0x5bf,
        c: 0,
    }, // 0xfb4e       pe+rafe
    Decomp {
        a: 0x5d0,
        b: 0x5dc,
        c: 0,
    }, // 0xfb4f       alef-lamed
];

// Global work variables
pub(super) struct RegexpState {
    pub(super) reg_prev_sub: Option<std::ffi::CString>,
    pub(super) class_tab: [i16; 256],
    pub(super) init_class_tab_done: bool,
    pub(super) regparse: *mut c_char,
    pub(super) regnpar: i32,
    pub(super) wants_nfa: bool,
    pub(super) regnzpar: i32,
    pub(super) re_has_z: i32,
    pub(super) regflags: u32,
    pub(super) had_eol: i32,
    pub(super) reg_magic: RegMagic,
    pub(super) reg_string: i32,
    pub(super) reg_strict: i32,
    pub(super) curchr: i32,
    pub(super) prevchr: i32,
    pub(super) prevprevchr: i32,
    pub(super) nextchr: i32,
    pub(super) prevchr_len: i32,
    pub(super) at_start: i32,
    pub(super) prev_at_start: i32,
    pub(super) peekchr_after_slash: i32,
    pub(super) reg_cpo_lit: i32,
    pub(super) reg_tofree: Vec<u8>,
    pub(super) rex: Regexec,
    pub(super) rex_in_use: bool,
    pub(super) can_f_submatch: bool,
    pub(super) rsm: Regsubmatch,
    pub(super) reg_startzp: [*mut u8; NSUBEXP],
    pub(super) reg_endzp: [*mut u8; NSUBEXP],
    pub(super) reg_startzpos: [Lpos; NSUBEXP],
    pub(super) reg_endzpos: [Lpos; NSUBEXP],
    pub(super) eval_result: [Option<std::ffi::CString>; MAX_REGSUB_NESTING],
    pub(super) regsub_nesting: usize,
    pub(super) had_endbrace: [u8; NSUBEXP],
    pub(super) num_complex_braces: i32,
    pub(super) regcode: *mut u8,
    pub(super) regsize: i64,
    pub(super) reg_toolong: i32,
    pub(super) bt_reg_parse_depth: i32,
    pub(super) brace_min: [i64; 10],
    pub(super) brace_max: [i64; 10],
    pub(super) brace_count: [i32; 10],
    pub(super) one_exactly: bool,
    pub(super) regstack: Vec<u64>,
    backpos: Vec<Backpos>,
    pub(super) behind_pos: Regsave,
    pub(super) nfa_re_flags: i32,
    pub(super) post_vec: Vec<i32>,
    pub(super) nfa_reg_parse_depth: i32,
    pub(super) nstate: i32,
    pub(super) istate: i32,
    pub(super) nfa_endp: *mut SaveSe,
    pub(super) nfa_ll_index: usize,
    pub(super) regexp_engine: i32,
}

impl RegexpState {
    const fn new() -> Self {
        Self {
            reg_prev_sub: None,
            class_tab: [0; 256],
            init_class_tab_done: false,
            regparse: ptr::null_mut(),
            regnpar: 0,
            wants_nfa: false,
            regnzpar: 0,
            re_has_z: 0,
            regflags: 0,
            had_eol: 0,
            reg_magic: 0,
            reg_string: 0,
            reg_strict: 0,
            curchr: 0,
            prevchr: 0,
            prevprevchr: 0,
            nextchr: 0,
            prevchr_len: 0,
            at_start: 0,
            prev_at_start: 0,
            peekchr_after_slash: 0,
            reg_cpo_lit: 0,
            reg_tofree: Vec::new(),
            rex: Regexec {
                reg_match: ptr::null_mut(),
                reg_mmatch: ptr::null_mut(),
                reg_startp: ptr::null_mut(),
                reg_endp: ptr::null_mut(),
                reg_startpos: ptr::null_mut(),
                reg_endpos: ptr::null_mut(),
                reg_win: ptr::null_mut(),
                reg_buf: ptr::null_mut(),
                reg_firstlnum: 0,
                reg_maxline: 0,
                reg_line_lbr: false,
                lnum: 0,
                line: ptr::null_mut(),
                input: ptr::null_mut(),
                need_clear_subexpr: 0,
                need_clear_zsubexpr: 0,
                reg_ic: false,
                reg_icombine: false,
                reg_nobreak: false,
                reg_maxcol: 0,
                nfa_has_zend: 0,
                nfa_has_backref: 0,
                nfa_nsubexpr: 0,
                nfa_listid: 0,
                nfa_alt_listid: 0,
                nfa_has_zsubexpr: 0,
            },
            rex_in_use: false,
            can_f_submatch: false,
            rsm: Regsubmatch {
                sm_match: ptr::null_mut(),
                sm_mmatch: ptr::null_mut(),
                sm_firstlnum: 0,
                sm_maxline: 0,
                sm_line_lbr: 0,
            },
            reg_startzp: [ptr::null_mut(); NSUBEXP],
            reg_endzp: [ptr::null_mut(); NSUBEXP],
            reg_startzpos: [Lpos { lnum: 0, col: 0 }; NSUBEXP],
            reg_endzpos: [Lpos { lnum: 0, col: 0 }; NSUBEXP],
            eval_result: [const { None }; MAX_REGSUB_NESTING],
            regsub_nesting: 0,
            had_endbrace: [0; NSUBEXP],
            num_complex_braces: 0,
            regcode: ptr::null_mut(),
            regsize: 0,
            reg_toolong: 0,
            bt_reg_parse_depth: 0,
            brace_min: [0; 10],
            brace_max: [0; 10],
            brace_count: [0; 10],
            one_exactly: false,
            regstack: Vec::new(),
            backpos: Vec::new(),
            behind_pos: Regsave::new(),
            nfa_re_flags: 0,
            post_vec: Vec::new(),
            nfa_reg_parse_depth: 0,
            nstate: 0,
            istate: 0,
            nfa_endp: ptr::null_mut(),
            nfa_ll_index: 0,
            regexp_engine: 0,
        }
    }
}

pub(super) static RE_STATE: GlobalState<RegexpState> = GlobalState::new(RegexpState::new());

#[inline]
pub(super) fn re_st() -> &'static mut RegexpState {
    unsafe { &mut *RE_STATE.as_ptr() }
}

#[inline]
pub(super) fn reg_prev_sub_ref() -> Option<&'static CStr> {
    unsafe { (*RE_STATE.as_ptr()).reg_prev_sub.as_deref() }
}

use crate::eval::partial_name;
use crate::eval::typval::tv_get_string_buf_chk;
use crate::eval::userfunc::call_func;
use crate::strings::{vim_strchr, vim_strsave_escaped};

#[inline]
pub(super) fn tv_list_ref(l: Option<&mut TvList>) {
    if let Some(l) = l {
        l.lv_refcount += 1;
    }
}

#[inline]
pub(super) const fn magic_fn(x: i32) -> i32 {
    x - 256
}

#[inline]
pub(super) const fn un_magic(x: i32) -> i32 {
    x + 256
}

#[inline]
pub(super) const fn is_magic_fn(x: i32) -> bool {
    x < 0
}

#[inline]
pub(super) const fn no_magic(x: i32) -> i32 {
    if is_magic_fn(x) { un_magic(x) } else { x }
}

#[inline]
pub(super) const fn toggle_magic(x: i32) -> i32 {
    if is_magic_fn(x) {
        un_magic(x)
    } else {
        magic_fn(x)
    }
}

#[inline]
pub(super) fn ucharat(p: *const u8) -> i32 {
    unsafe { *p as i32 }
}

#[inline]
pub(super) fn reg_multi() -> bool {
    re_st().rex.reg_match.is_null()
}

#[inline]
pub(super) fn mb_ptr_adv<T>(p: *mut *mut T) {
    unsafe {
        let step = utfc_ptr2len((*p).cast()) as usize;
        *p = (*p).cast::<u8>().wrapping_add(step).cast();
    }
}

/// Return NOT_MULTI if c is not a "multi" operator.
/// Return MULTI_ONE if c is a single "multi" operator.
/// Return MULTI_MULT if c is a multi "multi" operator.
#[inline]
pub(super) fn re_multi_type(c: i32) -> i32 {
    if c == magic_fn(b'@' as i32) || c == magic_fn(b'=' as i32) || c == magic_fn(b'?' as i32) {
        return MULTI_ONE;
    }
    if c == magic_fn(b'*' as i32) || c == magic_fn(b'+' as i32) || c == magic_fn(b'{' as i32) {
        return MULTI_MULT;
    }
    NOT_MULTI
}

/// Translate `\x` to its control character, except `\n`, which is Magic.
#[inline]
pub(super) fn backslash_trans(c: i32) -> i32 {
    match c as u8 {
        b'r' => CAR,
        b't' => TAB,
        b'e' => ESC,
        b'b' => BS,
        _ => c,
    }
}

/// Check for a character class name `[:name:]`. `pp` points to the `[`.
pub(super) fn get_char_class(pp: *mut *mut c_char) -> i32 {
    const CHAR_CLASS_TAB: &[(i32, &[u8])] = &[
        (CLASS_ALNUM, b"alnum:]"),
        (CLASS_ALPHA, b"alpha:]"),
        (CLASS_BACKSPACE, b"backspace:]"),
        (CLASS_BLANK, b"blank:]"),
        (CLASS_CNTRL, b"cntrl:]"),
        (CLASS_DIGIT, b"digit:]"),
        (CLASS_ESCAPE, b"escape:]"),
        (CLASS_FNAME, b"fname:]"),
        (CLASS_GRAPH, b"graph:]"),
        (CLASS_IDENT, b"ident:]"),
        (CLASS_KEYWORD, b"keyword:]"),
        (CLASS_LOWER, b"lower:]"),
        (CLASS_PRINT, b"print:]"),
        (CLASS_PUNCT, b"punct:]"),
        (CLASS_RETURN, b"return:]"),
        (CLASS_SPACE, b"space:]"),
        (CLASS_TAB, b"tab:]"),
        (CLASS_UPPER, b"upper:]"),
        (CLASS_XDIGIT, b"xdigit:]"),
    ];

    unsafe {
        let p = (*pp).cast::<u8>();
        if *p.wrapping_add(1) == b':'
            && (*p.wrapping_add(2)).is_ascii_lowercase()
            && (*p.wrapping_add(3)).is_ascii_lowercase()
            && (*p.wrapping_add(4)).is_ascii_lowercase()
        {
            let target = p.wrapping_add(2);
            for &(key, pat) in CHAR_CLASS_TAB {
                if crate::memory::xstrnlen(target.cast::<c_char>(), pat.len()) == pat.len()
                    && std::slice::from_raw_parts(target, pat.len()) == pat
                {
                    *pp = (*pp).wrapping_add(pat.len() + 2);
                    return key;
                }
            }
        }
        CLASS_NONE
    }
}

pub(super) fn init_class_tab() {
    unsafe {
        if re_st().init_class_tab_done {
            return;
        }
        let tab = (&raw mut re_st().class_tab).cast::<i16>();
        for i in 0..256usize {
            let val = if (b'0' as usize..=b'7' as usize).contains(&i) {
                RI_DIGIT + RI_HEX + RI_OCTAL + RI_WORD
            } else if (b'8' as usize..=b'9' as usize).contains(&i) {
                RI_DIGIT + RI_HEX + RI_WORD
            } else if (b'a' as usize..=b'f' as usize).contains(&i) {
                RI_HEX + RI_WORD + RI_HEAD + RI_ALPHA + RI_LOWER
            } else if (b'g' as usize..=b'z' as usize).contains(&i) {
                RI_WORD + RI_HEAD + RI_ALPHA + RI_LOWER
            } else if (b'A' as usize..=b'F' as usize).contains(&i) {
                RI_HEX + RI_WORD + RI_HEAD + RI_ALPHA + RI_UPPER
            } else if (b'G' as usize..=b'Z' as usize).contains(&i) {
                RI_WORD + RI_HEAD + RI_ALPHA + RI_UPPER
            } else if i == b'_' as usize {
                RI_WORD + RI_HEAD
            } else {
                0
            };
            *tab.wrapping_add(i) = val;
        }
        *tab.wrapping_add(b' ' as usize) |= RI_WHITE;
        *tab.wrapping_add(b'\t' as usize) |= RI_WHITE;
        re_st().init_class_tab_done = true;
    }
}

#[inline]
pub(super) fn ri_digit(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_DIGIT) != 0
}
#[inline]
pub(super) fn ri_hex(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_HEX) != 0
}
#[inline]
pub(super) fn ri_octal(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_OCTAL) != 0
}
#[inline]
pub(super) fn ri_word(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_WORD) != 0
}
#[inline]
pub(super) fn ri_head(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_HEAD) != 0
}
#[inline]
pub(super) fn ri_alpha(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_ALPHA) != 0
}
#[inline]
pub(super) fn ri_lower(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_LOWER) != 0
}
#[inline]
pub(super) fn ri_upper(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_UPPER) != 0
}

/// Return true if compiled regular expression `prog` can match a line break.
pub fn re_multiline(prog: &Regprog) -> i32 {
    (prog.regflags & RF_HASNL) as i32
}

/// Check for an equivalence class name `[=a=]`. `pp` points to the `[`.
pub(super) fn get_equi_class(pp: *mut *mut c_char) -> i32 {
    unsafe {
        let p = *pp;
        if crate::memory::cstr_byte_at(p, 1) == b'=' && crate::memory::cstr_byte_at(p, 2) != 0 {
            let l = utfc_ptr2len(p.wrapping_add(2)) as usize;
            if crate::memory::cstr_byte_at(p, l + 2) == b'='
                && crate::memory::cstr_byte_at(p, l + 3) == b']'
            {
                let c = utf_ptr2char(p.wrapping_add(2));
                *pp = (*pp).wrapping_add(l + 4);
                return c;
            }
        }
        0
    }
}

/// Check for a collating element `[.a.]`. `pp` points to the `[`.
pub(super) fn get_coll_element(pp: *mut *mut c_char) -> i32 {
    unsafe {
        let p = *pp;
        if !crate::memory::cstr_is_empty(p)
            && crate::memory::cstr_byte_at(p, 1) == b'.'
            && crate::memory::cstr_byte_at(p, 2) != 0
        {
            let l = utfc_ptr2len(p.wrapping_add(2)) as usize;
            if crate::memory::cstr_byte_at(p, l + 2) == b'.'
                && crate::memory::cstr_byte_at(p, l + 3) == b']'
            {
                let c = utf_ptr2char(p.wrapping_add(2));
                *pp = (*pp).wrapping_add(l + 4);
                return c;
            }
        }
        0
    }
}

pub(super) fn get_cpo_flags() {
    re_st().reg_cpo_lit = (crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_LITERAL)) as i32;
}

/// Skip over a `[]` range.
pub(super) fn skip_anyof(p: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let mut p = p.into_c_char_ptr().cast_mut();
    {
        if crate::memory::cstr_first_byte(p) == b'^' {
            p = p.wrapping_add(1);
        }
        if crate::memory::cstr_first_byte(p) == b']' || crate::memory::cstr_first_byte(p) == b'-' {
            p = p.wrapping_add(1);
        }
        while !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b']' {
            let l = utfc_ptr2len(p);
            if l > 1 {
                p = p.wrapping_add(l as usize);
            } else if crate::memory::cstr_first_byte(p) == b'-' {
                p = p.wrapping_add(1);
                if crate::memory::cstr_first_byte(p) != b']' && !crate::memory::cstr_is_empty(p) {
                    mb_ptr_adv(&raw mut p);
                }
            } else if crate::memory::cstr_first_byte(p) == b'\\'
                && (crate::strings::vim_haschr(
                    REGEXP_INRANGE,
                    crate::memory::cstr_byte_at(p, 1) as i32,
                ) || (re_st().reg_cpo_lit == 0
                    && crate::strings::vim_haschr(
                        REGEXP_ABBR,
                        crate::memory::cstr_byte_at(p, 1) as i32,
                    )))
            {
                p = p.wrapping_add(2);
            } else if crate::memory::cstr_first_byte(p) == b'[' {
                if get_char_class(&raw mut p) == CLASS_NONE
                    && get_equi_class(&raw mut p) == 0
                    && get_coll_element(&raw mut p) == 0
                    && !crate::memory::cstr_is_empty(p)
                {
                    p = p.wrapping_add(1);
                }
            } else {
                p = p.wrapping_add(1);
            }
        }
        p
    }
}

/// Skip past regular expression.
pub fn skip_regexp(startp: *mut c_char, delim: i32, magic: i32) -> *mut c_char {
    skip_regexp_ex(startp, delim, magic, false, None, None, None)
}

/// Call `skip_regexp()` and when the delimiter does not match give an error and return NULL.
pub fn skip_regexp_err(startp: *mut c_char, delim: i32, magic: i32) -> *mut c_char {
    {
        let p = skip_regexp(startp, delim, magic);
        if crate::memory::cstr_first_byte(p) as i32 != delim {
            semsg!(tr(E_MISSING_DELIMITER_AFTER_SEARCH_PATTERN_STR), startp,);
            return ptr::null_mut();
        }
        p
    }
}

/// `skip_regexp()` with extra arguments.
pub fn skip_regexp_ex(
    mut startp: *mut c_char,
    dirc: i32,
    magic: i32,
    in_place: bool,
    mut copy_buf: Option<&mut Vec<c_char>>,
    mut dropped: Option<&mut i32>,
    mut magic_val: Option<&mut RegMagic>,
) -> *mut c_char {
    unsafe {
        let mut mymagic = if magic != 0 { MAGIC_ON } else { MAGIC_OFF };
        let mut p = startp;
        let mut startplen: usize = 0;

        get_cpo_flags();

        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) as i32 == dirc {
                break;
            }
            if (crate::memory::cstr_first_byte(p) == b'[' && mymagic >= MAGIC_ON)
                || (crate::memory::cstr_first_byte(p) == b'\\'
                    && crate::memory::cstr_byte_at(p, 1) == b'['
                    && mymagic <= MAGIC_OFF)
            {
                p = skip_anyof(p.wrapping_add(1));
                if crate::memory::cstr_is_empty(p) {
                    break;
                }
            } else if crate::memory::cstr_first_byte(p) == b'\\'
                && crate::memory::cstr_byte_at(p, 1) != 0
            {
                if dirc == b'?' as i32 && in_place && crate::memory::cstr_byte_at(p, 1) == b'?' {
                    if startplen == 0 {
                        startplen = crate::memory::cstr_len(startp);
                    }
                    if let Some(buf) = copy_buf.as_deref_mut()
                        && buf.is_empty()
                    {
                        let off = p.offset_from(startp) as usize;
                        buf.extend_from_slice(std::slice::from_raw_parts(startp, startplen + 1));
                        startp = buf.as_mut_ptr();
                        p = startp.wrapping_add(off);
                    }
                    if let Some(ref mut p) = dropped {
                        **p += 1;
                    }
                    ptr::copy(
                        p.wrapping_add(1),
                        p,
                        startplen - (p.wrapping_add(1).offset_from(startp) as usize) + 1,
                    );
                } else {
                    p = p.wrapping_add(1);
                }
                if crate::memory::cstr_first_byte(p) == b'v' {
                    mymagic = MAGIC_ALL;
                } else if crate::memory::cstr_first_byte(p) == b'V' {
                    mymagic = MAGIC_NONE;
                }
            }
            mb_ptr_adv(&raw mut p);
        }
        if let Some(ref mut p) = magic_val {
            **p = mymagic;
        }
        p
    }
}

pub(super) fn initchr(str_: impl crate::strings::IntoCCharPtr) {
    let str_ = str_.into_c_char_ptr().cast_mut();
    re_st().regparse = str_;
    re_st().prevchr_len = 0;
    re_st().nextchr = -1;
    re_st().prevchr = -1;
    re_st().prevprevchr = -1;
    re_st().curchr = -1;
    re_st().at_start = 1;
    re_st().prev_at_start = 0;
}

pub(super) fn save_parse_state(ps: &mut ParseState) {
    ps.regparse = re_st().regparse;
    ps.prevchr_len = re_st().prevchr_len;
    ps.curchr = re_st().curchr;
    ps.prevchr = re_st().prevchr;
    ps.prevprevchr = re_st().prevprevchr;
    ps.nextchr = re_st().nextchr;
    ps.at_start = re_st().at_start;
    ps.prev_at_start = re_st().prev_at_start;
    ps.regnpar = re_st().regnpar;
}

pub(super) fn restore_parse_state(ps: &mut ParseState) {
    re_st().regparse = ps.regparse;
    re_st().prevchr_len = ps.prevchr_len;
    re_st().curchr = ps.curchr;
    re_st().prevchr = ps.prevchr;
    re_st().prevprevchr = ps.prevprevchr;
    re_st().nextchr = ps.nextchr;
    re_st().at_start = ps.at_start;
    re_st().prev_at_start = ps.prev_at_start;
    re_st().regnpar = ps.regnpar;
}

pub(super) fn peekchr() -> i32 {
    unsafe {
        if re_st().curchr != -1 {
            return re_st().curchr;
        }

        re_st().curchr = crate::memory::cstr_first_byte(re_st().regparse) as i32;
        match re_st().curchr as u8 {
            b'.' | b'[' | b'~' => {
                if re_st().reg_magic >= MAGIC_ON {
                    re_st().curchr = magic_fn(re_st().curchr);
                }
            }
            b'(' | b')' | b'{' | b'%' | b'+' | b'=' | b'?' | b'@' | b'!' | b'&' | b'|' | b'<'
            | b'>' | b'#' | b'"' | b'\'' | b',' | b'-' | b':' | b';' | b'`' | b'/' => {
                if re_st().reg_magic == MAGIC_ALL {
                    re_st().curchr = magic_fn(re_st().curchr);
                }
            }
            b'*' => {
                if re_st().reg_magic >= MAGIC_ON
                    && re_st().at_start == 0
                    && !(re_st().prev_at_start != 0 && re_st().prevchr == magic_fn(b'^' as i32))
                    && (re_st().peekchr_after_slash != 0
                        || (re_st().prevchr != magic_fn(b'(' as i32)
                            && re_st().prevchr != magic_fn(b'&' as i32)
                            && re_st().prevchr != magic_fn(b'|' as i32)))
                {
                    re_st().curchr = magic_fn(b'*' as i32);
                }
            }
            b'^' => {
                if re_st().reg_magic >= MAGIC_OFF
                    && (re_st().at_start != 0
                        || re_st().reg_magic == MAGIC_ALL
                        || re_st().prevchr == magic_fn(b'(' as i32)
                        || re_st().prevchr == magic_fn(b'|' as i32)
                        || re_st().prevchr == magic_fn(b'&' as i32)
                        || re_st().prevchr == magic_fn(b'n' as i32)
                        || (no_magic(re_st().prevchr) == b'(' as i32
                            && re_st().prevprevchr == magic_fn(b'%' as i32)))
                {
                    re_st().curchr = magic_fn(b'^' as i32);
                    re_st().at_start = 1;
                    re_st().prev_at_start = 0;
                }
            }
            b'$' => {
                if re_st().reg_magic >= MAGIC_OFF {
                    let mut p = re_st().regparse.wrapping_add(1).cast::<u8>();
                    let mut is_magic_all = re_st().reg_magic == MAGIC_ALL;

                    while *p == b'\\'
                        && matches!(
                            *p.wrapping_add(1),
                            b'c' | b'C' | b'm' | b'M' | b'v' | b'V' | b'Z'
                        )
                    {
                        if *p.wrapping_add(1) == b'v' {
                            is_magic_all = true;
                        } else if matches!(*p.wrapping_add(1), b'm' | b'M' | b'V') {
                            is_magic_all = false;
                        }
                        p = p.wrapping_add(2);
                    }
                    if *p == NUL
                        || (*p == b'\\' && matches!(*p.wrapping_add(1), b'|' | b'&' | b')' | b'n'))
                        || (is_magic_all && matches!(*p, b'|' | b'&' | b')'))
                        || re_st().reg_magic == MAGIC_ALL
                    {
                        re_st().curchr = magic_fn(b'$' as i32);
                    }
                }
            }
            b'\\' => {
                let c = crate::memory::cstr_byte_at(re_st().regparse, 1) as i32;
                if c == NUL as i32 {
                    re_st().curchr = b'\\' as i32;
                } else if c <= b'~' as i32 && META_FLAGS[c as usize] != 0 {
                    re_st().curchr = -1;
                    re_st().prev_at_start = re_st().at_start;
                    re_st().at_start = 0;
                    re_st().regparse = re_st().regparse.wrapping_add(1);
                    re_st().peekchr_after_slash += 1;
                    let _ = peekchr();
                    re_st().regparse = re_st().regparse.wrapping_sub(1);
                    re_st().peekchr_after_slash -= 1;
                    re_st().curchr = toggle_magic(re_st().curchr);
                } else if crate::strings::vim_haschr(REGEXP_ABBR, c) {
                    re_st().curchr = backslash_trans(c);
                } else if re_st().reg_magic == MAGIC_NONE && (c == b'$' as i32 || c == b'^' as i32)
                {
                    re_st().curchr = toggle_magic(c);
                } else {
                    re_st().curchr = if c >= 0x80 {
                        utf_ptr2char(re_st().regparse.wrapping_add(1))
                    } else {
                        c
                    };
                }
            }
            _ => {
                if re_st().curchr >= 0x80 {
                    re_st().curchr = utf_ptr2char(re_st().regparse);
                }
            }
        }

        re_st().curchr
    }
}

pub(super) fn skipchr() {
    unsafe {
        if crate::memory::cstr_first_byte(re_st().regparse) == b'\\' {
            re_st().prevchr_len = 1;
        } else {
            re_st().prevchr_len = 0;
        }
        if *re_st().regparse.wrapping_add(re_st().prevchr_len as usize) != NUL as c_char {
            if (*re_st().regparse.wrapping_add(re_st().prevchr_len as usize) as u8) < 0x80 {
                re_st().prevchr_len += 1;
            } else {
                re_st().prevchr_len +=
                    utf_ptr2len(re_st().regparse.wrapping_add(re_st().prevchr_len as usize));
            }
        }
        re_st().regparse = re_st().regparse.wrapping_add(re_st().prevchr_len as usize);
        re_st().prev_at_start = re_st().at_start;
        re_st().at_start = 0;
        re_st().prevprevchr = re_st().prevchr;
        re_st().prevchr = re_st().curchr;
        re_st().curchr = re_st().nextchr;
        re_st().nextchr = -1;
    }
}

pub(super) fn skipchr_keepstart() {
    let as_ = re_st().prev_at_start;
    let pr = re_st().prevchr;
    let prpr = re_st().prevprevchr;

    skipchr();
    re_st().at_start = as_;
    re_st().prevchr = pr;
    re_st().prevprevchr = prpr;
}

pub(super) fn getchr() -> i32 {
    let chr = peekchr();
    skipchr();
    chr
}

pub(super) fn ungetchr() {
    {
        re_st().nextchr = re_st().curchr;
        re_st().curchr = re_st().prevchr;
        re_st().prevchr = re_st().prevprevchr;
        re_st().at_start = re_st().prev_at_start;
        re_st().prev_at_start = 0;
        re_st().regparse = re_st().regparse.wrapping_sub(re_st().prevchr_len as usize);
    }
}

pub(super) fn gethexchrs(maxinputlen: i32) -> i64 {
    {
        let mut nr: i64 = 0;
        let mut i: i32 = 0;
        while i < maxinputlen {
            let c = crate::memory::cstr_first_byte(re_st().regparse) as i32;
            if !ascii_isxdigit(c) {
                break;
            }
            nr <<= 4;
            nr |= i64::from(hex2nr(c));
            re_st().regparse = re_st().regparse.wrapping_add(1);
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) fn getdecchrs() -> i64 {
    {
        let mut nr: i64 = 0;
        let mut i: i32 = 0;
        loop {
            let c = crate::memory::cstr_first_byte(re_st().regparse);
            if !c.is_ascii_digit() {
                break;
            }
            nr = nr.wrapping_mul(10).wrapping_add(i64::from(c - b'0'));
            re_st().regparse = re_st().regparse.wrapping_add(1);
            re_st().curchr = -1;
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) fn getoctchrs() -> i64 {
    {
        let mut nr: i64 = 0;
        let mut i: i32 = 0;
        while i < 3 && nr < 0o40 {
            let c = crate::memory::cstr_first_byte(re_st().regparse);
            if !(b'0'..=b'7').contains(&c) {
                break;
            }
            nr <<= 3;
            nr |= i64::from(hex2nr(c as i32));
            re_st().regparse = re_st().regparse.wrapping_add(1);
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) fn read_limits(minval: &mut i32, maxval: &mut i32) -> bool {
    unsafe {
        let mut reverse = false;
        if crate::memory::cstr_first_byte(re_st().regparse) == b'-' {
            re_st().regparse = re_st().regparse.wrapping_add(1);
            reverse = true;
        }
        let first_char = re_st().regparse;
        *minval = getdigits_int(&raw mut re_st().regparse, false, 0);
        if crate::memory::cstr_first_byte(re_st().regparse) == b',' {
            re_st().regparse = re_st().regparse.wrapping_add(1);
            if ascii_isdigit(crate::memory::cstr_first_byte(re_st().regparse) as i32) {
                *maxval = getdigits_int(&raw mut re_st().regparse, false, MAX_LIMIT);
            } else {
                *maxval = MAX_LIMIT;
            }
        } else if ascii_isdigit(crate::memory::cstr_first_byte(first_char) as i32) {
            *maxval = *minval;
        } else {
            *maxval = MAX_LIMIT;
        }
        if crate::memory::cstr_first_byte(re_st().regparse) == b'\\' {
            re_st().regparse = re_st().regparse.wrapping_add(1);
        }
        if crate::memory::cstr_first_byte(re_st().regparse) != b'}' {
            semsg!(
                tr(c"E554: Syntax error in %s{...}"),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
            g_st().rc_did_emsg = true;
            return false;
        }
        if (!reverse && *minval > *maxval) || (reverse && *minval < *maxval) {
            ptr::swap(minval, maxval);
        }
        skipchr();
        true
    }
}

#[inline]
pub(super) fn reg_breakcheck() {
    if !re_st().rex.reg_nobreak {
        fast_breakcheck();
    }
}

pub(super) fn reg_iswordc(c: i32) -> bool {
    unsafe { vim_iswordc_buf(c, &mut *re_st().rex.reg_buf) }
}

fn reg_getline_common(
    lnum: Linenr,
    flags: RegGetlineFlags,
    line: *mut *mut c_char,
    length: Option<&mut Colnr>,
) {
    unsafe {
        let get_line = (flags & RGLF_LINE) != 0;
        let get_length = (flags & RGLF_LENGTH) != 0;
        let (firstlnum, maxline) = if (flags & RGLF_SUBMATCH) != 0 {
            (re_st().rsm.sm_firstlnum + lnum, re_st().rsm.sm_maxline)
        } else {
            (re_st().rex.reg_firstlnum + lnum, re_st().rex.reg_maxline)
        };

        if firstlnum < 1 {
            if get_line {
                *line = ptr::null_mut();
            }
            if get_length && let Some(length) = length {
                *length = 0;
            }
            return;
        }

        if lnum > maxline {
            if get_line {
                *line = c"".as_ptr().cast_mut();
            }
            if get_length && let Some(length) = length {
                *length = 0;
            }
            return;
        }

        if get_line {
            *line = ml_get_buf(&mut *re_st().rex.reg_buf, firstlnum);
        }
        if get_length && let Some(length) = length {
            *length = ml_get_buf_len(&mut *re_st().rex.reg_buf, firstlnum);
        }
    }
}

pub(super) fn reg_getline(lnum: Linenr) -> *mut c_char {
    let mut line: *mut c_char = ptr::null_mut();
    reg_getline_common(lnum, RGLF_LINE, &raw mut line, None);
    line
}

pub(super) fn reg_getline_len(lnum: Linenr) -> Colnr {
    let mut length: Colnr = 0;
    reg_getline_common(lnum, RGLF_LENGTH, ptr::null_mut(), Some(&mut length));
    length
}

pub(super) fn reg_prev_class() -> i32 {
    {
        if re_st().rex.input > re_st().rex.line {
            let prev = re_st().rex.input.wrapping_sub(1).cast::<c_char>();
            let head = prev.wrapping_sub(utf_head_off(re_st().rex.line.cast(), prev) as usize);
            return mb_get_class_tab(head, (&raw const re_st().rex.buf_mut().b_chartab).cast());
        }
        -1
    }
}

pub(super) fn reg_match_visual() -> bool {
    unsafe {
        let wp = if re_st().rex.reg_win.is_null() {
            curwin_ptr()
        } else {
            re_st().rex.reg_win
        };
        let wp = { &mut *wp };
        let wp = { &mut *wp };
        if re_st().rex.reg_buf != g_st().curbuf
            || g_st().visual_global.start.lnum == 0
            || !reg_multi()
        {
            return false;
        }

        let (top, bot, mode, curswant) = if g_st().visual_global.active {
            let (t, b) = if lt(g_st().visual_global.start, wp.w_cursor) {
                (g_st().visual_global.start, wp.w_cursor)
            } else {
                (wp.w_cursor, g_st().visual_global.start)
            };
            (t, b, g_st().visual_global.mode, wp.w_curswant)
        } else {
            let (t, mut b) = if lt(curbuf_mut().b_visual.vi_start, curbuf_mut().b_visual.vi_end) {
                (curbuf_mut().b_visual.vi_start, curbuf_mut().b_visual.vi_end)
            } else {
                (curbuf_mut().b_visual.vi_end, curbuf_mut().b_visual.vi_start)
            };
            if b.lnum > curbuf_mut().b_ml.ml_line_count {
                b.lnum = curbuf_mut().b_ml.ml_line_count;
            }
            (
                t,
                b,
                curbuf_mut().b_visual.vi_mode,
                curbuf_mut().b_visual.vi_curswant,
            )
        };

        let lnum = re_st().rex.lnum + re_st().rex.reg_firstlnum;
        if lnum < top.lnum || lnum > bot.lnum {
            return false;
        }

        let col = re_st().rex.input.offset_from(re_st().rex.line) as Colnr;
        if mode == b'v' as i32 {
            if (lnum == top.lnum && col < top.col)
                || (lnum == bot.lnum
                    && col
                        >= bot.col
                            + Colnr::from(crate::memory::cstr_first_byte(opt_vars().p_sel) != b'e'))
            {
                return false;
            }
        } else if mode == CTRL_V_CONST {
            let mut start: Colnr = 0;
            let mut end: Colnr = 0;
            let mut start2: Colnr = 0;
            let mut end2: Colnr = 0;
            let mut top_copy = top;
            let mut bot_copy = bot;
            getvvcol(
                &mut *(wp),
                &mut top_copy,
                Some(&mut start),
                None,
                Some(&mut end),
                0,
            );
            getvvcol(
                &mut *(wp),
                &mut bot_copy,
                Some(&mut start2),
                None,
                Some(&mut end2),
                0,
            );
            if start2 < start {
                start = start2;
            }
            if end2 > end {
                end = end2;
            }
            if top.col == MAXCOL || bot.col == MAXCOL || curswant == MAXCOL {
                end = MAXCOL;
            }

            re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
            re_st().rex.input = re_st().rex.line.wrapping_add(col as usize);

            let cols = win_linetabsize(
                &mut *wp,
                re_st().rex.reg_firstlnum + re_st().rex.lnum,
                re_st().rex.line.cast(),
                col,
            );
            if cols < start
                || cols
                    > end - Colnr::from(crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e')
            {
                return false;
            }
        }
        true
    }
}

pub(super) fn prog_magic_wrong() -> bool {
    unsafe {
        let prog = if reg_multi() {
            re_st().rex.mmatch_mut().regprog
        } else {
            re_st().rex.match_mut().regprog
        };
        let RegProgEngine::Bt(ref bt_prog) = (*prog).engine else {
            return false;
        };
        if ucharat(bt_prog.program.as_ptr()) != REGMAGIC {
            emsg(tr(E_RE_CORR));
            return true;
        }
        false
    }
}

pub(super) fn cleanup_subexpr() {
    unsafe {
        if re_st().rex.need_clear_subexpr == 0 {
            return;
        }
        if reg_multi() {
            let uninit_pos = Lpos { lnum: -1, col: -1 };
            std::slice::from_raw_parts_mut(re_st().rex.reg_startpos, NSUBEXP).fill(uninit_pos);
            std::slice::from_raw_parts_mut(re_st().rex.reg_endpos, NSUBEXP).fill(uninit_pos);
        } else {
            std::slice::from_raw_parts_mut(re_st().rex.reg_startp, NSUBEXP).fill(ptr::null_mut());
            std::slice::from_raw_parts_mut(re_st().rex.reg_endp, NSUBEXP).fill(ptr::null_mut());
        }
        re_st().rex.need_clear_subexpr = 0;
    }
}

pub(super) fn cleanup_zsubexpr() {
    if re_st().rex.need_clear_zsubexpr == 0 {
        return;
    }
    if reg_multi() {
        let uninit_pos = Lpos { lnum: -1, col: -1 };
        re_st().reg_startzpos = [uninit_pos; NSUBEXP];
        re_st().reg_endzpos = [uninit_pos; NSUBEXP];
    } else {
        re_st().reg_startzp = [ptr::null_mut(); NSUBEXP];
        re_st().reg_endzp = [ptr::null_mut(); NSUBEXP];
    }
    re_st().rex.need_clear_zsubexpr = 0;
}

pub(super) fn reg_nextline() {
    re_st().rex.lnum += 1;
    re_st().rex.line = reg_getline(re_st().rex.lnum).cast();
    re_st().rex.input = re_st().rex.line;
    reg_breakcheck();
}

pub(super) fn match_with_backref(
    start_lnum: Linenr,
    start_col: Colnr,
    end_lnum: Linenr,
    end_col: Colnr,
    bytelen: &mut i32,
) -> i32 {
    unsafe {
        let mut clnum = start_lnum;
        let mut ccol = start_col;

        *bytelen = 0;
        loop {
            if re_st().reg_tofree.is_empty() || re_st().rex.line != re_st().reg_tofree.as_mut_ptr()
            {
                let len = crate::memory::cstr_len(re_st().rex.line.cast()) + 51;
                let off = re_st().rex.input.offset_from(re_st().rex.line) as usize;
                if re_st().reg_tofree.len() < len {
                    re_st().reg_tofree.resize(len, 0);
                }
                crate::memory::cstr_cpy(
                    re_st().reg_tofree.as_mut_ptr().cast(),
                    re_st().rex.line.cast(),
                );
                re_st().rex.line = re_st().reg_tofree.as_mut_ptr();
                re_st().rex.input = re_st().rex.line.wrapping_add(off);
            }

            let p = reg_getline(clnum);
            debug_assert!(!p.is_null());

            let mut len = if clnum == end_lnum {
                end_col - ccol
            } else {
                reg_getline_len(clnum) - ccol
            };

            if (!re_st().rex.reg_ic
                && cstrncmp(
                    p.wrapping_add(ccol as usize),
                    re_st().rex.input.cast(),
                    &mut len,
                ) != 0)
                || (re_st().rex.reg_ic
                    && mb_strnicmp(
                        p.wrapping_add(ccol as usize),
                        re_st().rex.input.cast(),
                        len as usize,
                    ) != 0)
            {
                return RA_NOMATCH;
            }
            *bytelen += len;
            if clnum == end_lnum {
                break;
            }
            if re_st().rex.lnum >= re_st().rex.reg_maxline {
                return RA_NOMATCH;
            }

            reg_nextline();
            *bytelen = 0;
            clnum += 1;
            ccol = 0;
            if g_st().got_int {
                return RA_FAIL;
            }
        }
        RA_MATCH
    }
}

pub(super) fn re_mult_next(what: &std::ffi::CStr) -> bool {
    if re_multi_type(peekchr()) == MULTI_MULT {
        semsg!(tr(c"E888: (NFA regexp) cannot repeat %s"), what.as_ptr());
        g_st().rc_did_emsg = true;
        return false;
    }
    true
}

fn mb_decompose(c: i32, c1: &mut i32, c2: &mut i32, c3: &mut i32) {
    if (0xfb20..=0xfb4f).contains(&c) {
        let d = DECOMP_TABLE[(c - 0xfb20) as usize];
        *c1 = d.a;
        *c2 = d.b;
        *c3 = d.c;
    } else {
        *c1 = c;
        *c2 = 0;
        *c3 = 0;
    }
}

pub(super) fn cstrncmp(
    s1: impl crate::strings::IntoCCharPtr,
    s2: impl crate::strings::IntoCCharPtr,
    n: &mut i32,
) -> i32 {
    let s1 = s1.into_c_char_ptr();
    let s2 = s2.into_c_char_ptr();
    unsafe {
        let mut result = if !re_st().rex.reg_ic {
            crate::memory::cstr_prefix_bytes(s1, *n as usize)
                .cmp(crate::memory::cstr_prefix_bytes(s2, *n as usize)) as i32
        } else {
            let mut p = s1.cast_mut();
            let mut n2: i32 = 0;
            let mut n1 = *n;
            while n1 > 0 && !crate::memory::cstr_is_empty(p) {
                n1 -= utfc_ptr2len(p);
                mb_ptr_adv(&raw mut p);
                n2 += 1;
            }
            p = s2.cast_mut();
            while n2 > 0 && !crate::memory::cstr_is_empty(p) {
                n2 -= 1;
                mb_ptr_adv(&raw mut p);
            }
            n2 = p.offset_from(s2) as i32;
            let r = utf_strnicmp(s1, s2, *n as usize, n2 as usize);
            if r == 0 {
                *n = n2;
            }
            r
        };

        if result != 0 && re_st().rex.reg_icombine {
            let mut str1: *const c_char = s1;
            let mut str2: *const c_char = s2;
            let mut c1: i32 = 0;
            let mut c2: i32 = 0;
            let mut c11: i32 = 0;
            let mut c12: i32 = 0;
            let mut junk: i32 = 0;
            let mut junk2: i32 = 0;

            while (str1.offset_from(s1) as i32) < *n {
                c1 = mb_ptr2char_adv(&raw mut str1);
                c2 = mb_ptr2char_adv(&raw mut str2);

                if c1 != c2 && (!re_st().rex.reg_ic || utf_fold(c1) != utf_fold(c2)) {
                    mb_decompose(c1, &mut c11, &mut junk, &mut junk2);
                    mb_decompose(c2, &mut c12, &mut junk, &mut junk2);
                    c1 = c11;
                    c2 = c12;
                    if c11 != c12 && (!re_st().rex.reg_ic || utf_fold(c11) != utf_fold(c12)) {
                        break;
                    }
                }
            }
            result = c2 - c1;
            if result == 0 {
                *n = str2.offset_from(s2) as i32;
            }
        }

        result
    }
}

#[inline]
pub(super) fn cstrchr(s: impl crate::strings::IntoCCharPtr, c: i32) -> *mut c_char {
    let s = s.into_c_char_ptr();
    {
        if !re_st().rex.reg_ic {
            return vim_strchr(s, c);
        }

        let (cc, lc) = if c > 0x80 {
            let f = utf_fold(c);
            (f, f)
        } else if (c as u8).is_ascii_uppercase() {
            let l = (c as u8).to_ascii_lowercase() as i32;
            (l, l)
        } else if (c as u8).is_ascii_lowercase() {
            let u = (c as u8).to_ascii_uppercase() as i32;
            (u, c)
        } else {
            return vim_strchr(s, c);
        };

        let mut p = s;
        while !crate::memory::cstr_is_empty(p) {
            let uc = utf_ptr2char(p);
            if c > 0x80 || uc > 0x80 {
                if (uc < 0x80 || uc != crate::memory::cstr_first_byte(p) as i32)
                    && utf_fold(uc) == lc
                {
                    return p.cast_mut();
                }
            } else if crate::memory::cstr_first_byte(p) as i32 == c
                || crate::memory::cstr_first_byte(p) as i32 == cc
            {
                return p.cast_mut();
            }
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
        }
        ptr::null_mut()
    }
}

/// Replace tildes in the pattern by the old pattern.
/// Returns `Some(CString)` if a new string was allocated, or `None` if `source` was modified in place.
pub fn regtilde(source: *mut c_char, magic: i32, preview: bool) -> Option<std::ffi::CString> {
    unsafe {
        let mut newsub_vec: Option<Vec<c_char>> = None;
        let mut newsub = source;
        let mut newsublen: usize = 0;
        let mut tilde: [c_char; 3] = [b'~' as c_char, NUL as c_char, NUL as c_char];
        let mut tildelen: usize = 1;
        let mut error = false;

        if magic == 0 {
            tilde[0] = b'\\' as c_char;
            tilde[1] = b'~' as c_char;
            tilde[2] = NUL as c_char;
            tildelen = 2;
        }

        let mut p = newsub;
        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_prefix_bytes(p, tildelen)
                == crate::memory::cstr_prefix_bytes(tilde.as_ptr(), tildelen)
            {
                let prefixlen = p.offset_from(newsub) as usize;
                let postfix = p.wrapping_add(tildelen);
                if newsublen == 0 {
                    newsublen = crate::memory::cstr_len(newsub);
                }
                newsublen -= tildelen;
                let postfixlen = newsublen - prefixlen;
                let prev_sub_ref = reg_prev_sub_ref();
                let reg_prev_sublen = prev_sub_ref.map_or(0, |s| s.to_bytes().len());
                let tmpsublen = prefixlen + reg_prev_sublen + postfixlen;

                if tmpsublen > 0
                    && let Some(prev_sub) = prev_sub_ref
                {
                    if tmpsublen > MAXCOL as usize {
                        emsg(tr(E_RESULTING_TEXT_TOO_LONG));
                        error = true;
                        break;
                    }

                    let mut tmpsub_vec: Vec<c_char> = vec![0; tmpsublen + 1];
                    let tmpsub = tmpsub_vec.as_mut_ptr();
                    ptr::copy_nonoverlapping(newsub, tmpsub, prefixlen);
                    ptr::copy_nonoverlapping(
                        prev_sub.as_ptr(),
                        tmpsub.wrapping_add(prefixlen),
                        reg_prev_sublen,
                    );
                    crate::memory::cstr_cpy(
                        tmpsub.wrapping_add(prefixlen + reg_prev_sublen),
                        postfix,
                    );

                    newsub_vec = Some(tmpsub_vec);
                    newsub = newsub_vec.as_mut().unwrap().as_mut_ptr();
                    newsublen = tmpsublen;
                    p = newsub.wrapping_add(prefixlen + reg_prev_sublen);
                } else {
                    ptr::copy(postfix, p, postfixlen + 1);
                }
                continue;
            } else {
                if crate::memory::cstr_first_byte(p) == b'\\'
                    && crate::memory::cstr_byte_at(p, 1) != 0
                {
                    p = p.wrapping_add(1);
                }
                p = p.wrapping_add(utfc_ptr2len(p) as usize);
            }
        }

        if error {
            return None;
        }

        if !preview {
            newsublen = p.offset_from(newsub) as usize;
            re_st().reg_prev_sub = if newsublen == 0 {
                None
            } else {
                cstring_from_bytes(newsub, newsublen)
            };
        }

        newsub_vec.map(|v| crate::memory::cstr_to_cstring(v.as_ptr()))
    }
}

pub(crate) fn fill_submatch_list(
    _argc: i32,
    argv: *mut TypVal,
    argskip: i32,
    fp: &mut UserFunc,
) -> i32 {
    unsafe {
        let listarg = argv.wrapping_add(argskip as usize);
        if fp.uf_varargs == 0 && (fp.uf_args.len() as i32) <= argskip {
            return argskip;
        }

        tv_list_init_static10(&mut *(*listarg).vval.v_list().cast::<StaticList10>());

        let mut li = tv_list_first(((*listarg).vval.v_list()).as_ref());
        for i in 0..10 {
            let s = re_st().rsm.match_mut().startp[i];
            if s.is_null() || re_st().rsm.match_mut().endp[i].is_null() {
                tv_set_string(&mut (*li).li_tv, ptr::null());
            } else {
                let len = re_st().rsm.match_mut().endp[i].offset_from(s) as usize;
                let cs = cstring_from_bytes(s, len).unwrap_or_default();
                tv_set_string(&mut (*li).li_tv, cs.as_ptr());
            }
            li = (*li).li_next;
        }
        argskip + 1
    }
}

pub fn vim_regsub(
    rmp: &mut Regmatch,
    source: *mut c_char,
    expr: Option<&TypVal>,
    dest: *mut c_char,
    destlen: i32,
    flags: i32,
) -> i32 {
    let mut rex_save = Regexec::new();
    let rex_in_use_save = re_st().rex_in_use;

    if re_st().rex_in_use {
        rex_save = re_st().rex;
    }
    re_st().rex_in_use = true;

    re_st().rex.reg_match = rmp;
    re_st().rex.reg_mmatch = ptr::null_mut();
    re_st().rex.reg_maxline = 0;
    re_st().rex.reg_buf = g_st().curbuf;
    re_st().rex.reg_line_lbr = true;
    let result = vim_regsub_both(source, expr, dest, destlen, flags);

    re_st().rex_in_use = rex_in_use_save;
    if re_st().rex_in_use {
        re_st().rex = rex_save;
    }

    result
}

pub fn vim_regsub_multi(
    rmp: &mut Regmmatch,
    lnum: Linenr,
    source: *mut c_char,
    dest: *mut c_char,
    destlen: i32,
    flags: i32,
) -> i32 {
    {
        let mut rex_save = Regexec::new();
        let rex_in_use_save = re_st().rex_in_use;

        if re_st().rex_in_use {
            rex_save = re_st().rex;
        }
        re_st().rex_in_use = true;

        re_st().rex.reg_match = ptr::null_mut();
        re_st().rex.reg_mmatch = rmp;
        re_st().rex.reg_buf = g_st().curbuf;
        re_st().rex.reg_firstlnum = lnum;
        re_st().rex.reg_maxline = curbuf_mut().b_ml.ml_line_count - lnum;
        re_st().rex.reg_line_lbr = false;
        let result = vim_regsub_both(source, None, dest, destlen, flags);

        re_st().rex_in_use = rex_in_use_save;
        if re_st().rex_in_use {
            re_st().rex = rex_save;
        }

        result
    }
}

fn vim_regsub_both(
    source: impl crate::strings::IntoCCharPtr,
    expr: Option<&TypVal>,
    dest: *mut c_char,
    destlen: i32,
    flags: i32,
) -> i32 {
    let source = source.into_c_char_ptr();
    unsafe {
        let mut no: i32 = -1;
        let mut func_all: Option<CaseConv> = None;
        let mut func_one: Option<CaseConv> = None;
        let mut clnum: Linenr = 0;
        let mut len: i32 = 0;
        let copy = (flags & REGSUB_COPY) != 0;

        if (source.is_null() && expr.is_none()) || dest.is_null() {
            emsg(tr(E_NULL));
            return 0;
        }
        if prog_magic_wrong() {
            return 0;
        }
        if re_st().regsub_nesting == MAX_REGSUB_NESTING {
            emsg(tr(E_SUBSTITUTE_NESTING_TOO_DEEP));
            return 0;
        }
        let nested = re_st().regsub_nesting;
        let eval_slot = &mut re_st().eval_result[nested];
        let mut src = source;
        let mut dst = dest;

        if expr.is_some()
            || (crate::memory::cstr_first_byte(source) == b'\\'
                && crate::memory::cstr_byte_at(source, 1) == b'=')
        {
            if copy {
                if let Some(ref eval_cstr) = *eval_slot {
                    let eval_len = eval_cstr.to_bytes().len();
                    if eval_len < destlen as usize {
                        crate::memory::cstr_cpy(dest, eval_cstr.as_ptr());
                        dst = dst.wrapping_add(eval_len);
                        *eval_slot = None;
                    }
                }
            } else {
                let prev_can_f_submatch = re_st().can_f_submatch;
                let mut rsm_save = Regsubmatch::new();

                *eval_slot = None;

                if re_st().can_f_submatch {
                    rsm_save = re_st().rsm;
                }
                re_st().can_f_submatch = true;
                re_st().rsm.sm_match = re_st().rex.reg_match;
                re_st().rsm.sm_mmatch = re_st().rex.reg_mmatch;
                re_st().rsm.sm_firstlnum = re_st().rex.reg_firstlnum;
                re_st().rsm.sm_maxline = re_st().rex.reg_maxline;
                re_st().rsm.sm_line_lbr = i32::from(re_st().rex.reg_line_lbr);

                re_st().regsub_nesting += 1;

                if let Some(expr) = expr {
                    let mut argv: [TypVal; 2] = [TypVal::new(); 2];
                    let mut rettv: TypVal = TypVal::new();
                    let mut match_list = StaticList10::new();
                    rettv.v_type = VarType::VarString;
                    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                    argv[0].v_type = VarType::VarList;
                    argv[0].vval =
                        crate::eval::typval::TypvalVval::List(&raw mut match_list.sl_list);
                    let mut funcexe = Funcexe {
                        fe_fill_submatch: true,
                        fe_firstline: 0,
                        fe_lastline: 0,
                        fe_doesrange: false,
                        fe_evaluate: true,
                        fe_partial: ptr::null_mut(),
                        fe_selfdict: ptr::null_mut(),
                        fe_basetv: ptr::null_mut(),
                        fe_found_var: false,
                    };
                    if expr.v_type == VarType::VarFunc {
                        let s = expr.vval.v_string();
                        call_func(s, -1, &mut rettv, 1, argv.as_mut_ptr(), &mut funcexe);
                    } else if expr.v_type == VarType::VarPartial {
                        let partial = expr.vval.v_partial();
                        let s = partial_name(partial.as_ref());
                        funcexe.fe_partial = partial;
                        call_func(s, -1, &mut rettv, 1, argv.as_mut_ptr(), &mut funcexe);
                    }
                    if rettv.v_type == VarType::VarUnknown {
                        *eval_slot = None;
                    } else {
                        let mut buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
                        let s = tv_get_string_buf_chk(&rettv, buf.as_mut_ptr());
                        if !s.is_null() {
                            *eval_slot = Some(crate::memory::cstr_to_cstring(s));
                        }
                    }
                    tv_clear(&mut rettv);
                } else {
                    *eval_slot = eval_to_string(source.wrapping_add(2), true, false);
                }
                re_st().regsub_nesting -= 1;

                if let Some(eval_cstr) = eval_slot.take() {
                    let mut eval_bytes = eval_cstr.into_bytes_with_nul();
                    let mut had_backslash = false;
                    let mut s = eval_bytes.as_mut_ptr().cast::<c_char>();
                    while !crate::memory::cstr_is_empty(s) {
                        if *s == NL as c_char && re_st().rsm.sm_line_lbr == 0 {
                            *s = CAR as c_char;
                        } else if crate::memory::cstr_first_byte(s) == b'\\'
                            && crate::memory::cstr_byte_at(s, 1) != 0
                        {
                            s = s.wrapping_add(1);
                            if *s == NL as c_char && re_st().rsm.sm_line_lbr == 0 {
                                *s = CAR as c_char;
                            }
                            had_backslash = true;
                        }
                        mb_ptr_adv(&raw mut s);
                    }
                    let final_cstr = if had_backslash && (flags & REGSUB_BACKSLASH) != 0 {
                        vim_strsave_escaped(eval_bytes.as_ptr().cast(), c"\\")
                    } else {
                        CStr::from_bytes_with_nul(&eval_bytes)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    dst = dst.wrapping_add(final_cstr.to_bytes().len());
                    *eval_slot = Some(final_cstr);
                }

                re_st().can_f_submatch = prev_can_f_submatch;
                if re_st().can_f_submatch {
                    re_st().rsm = rsm_save;
                }
            }
        } else {
            loop {
                let mut c = crate::memory::cstr_first_byte(src) as i32;
                src = src.wrapping_add(1);
                if c == NUL as i32 {
                    break;
                }
                if c == b'&' as i32 && (flags & REGSUB_MAGIC) != 0 {
                    no = 0;
                } else if c == b'\\' as i32 && !crate::memory::cstr_is_empty(src) {
                    if crate::memory::cstr_first_byte(src) == b'&' && (flags & REGSUB_MAGIC) == 0 {
                        src = src.wrapping_add(1);
                        no = 0;
                    } else if (crate::memory::cstr_first_byte(src)).is_ascii_digit() {
                        no = (crate::memory::cstr_first_byte(src) - b'0') as i32;
                        src = src.wrapping_add(1);
                    } else if crate::strings::vim_haschr(
                        c"uUlLeE",
                        crate::memory::cstr_first_byte(src) as i32,
                    ) {
                        let ch = crate::memory::cstr_first_byte(src);
                        src = src.wrapping_add(1);
                        match ch {
                            b'u' => {
                                func_one = Some(CaseConv::Upper);
                                continue;
                            }
                            b'U' => {
                                func_all = Some(CaseConv::Upper);
                                continue;
                            }
                            b'l' => {
                                func_one = Some(CaseConv::Lower);
                                continue;
                            }
                            b'L' => {
                                func_all = Some(CaseConv::Lower);
                                continue;
                            }
                            b'e' | b'E' => {
                                func_one = None;
                                func_all = None;
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
                if no < 0 {
                    if c == i32::from(K_SPECIAL)
                        && !crate::memory::cstr_is_empty(src)
                        && crate::memory::cstr_byte_at(src, 1) != 0
                    {
                        if copy {
                            if dst.wrapping_add(3) > dest.wrapping_add(destlen as usize) {
                                iemsg(c"vim_regsub_both(): not enough space");
                                return 0;
                            }
                            *dst = c as c_char;
                            dst = dst.wrapping_add(1);
                            *dst = *src;
                            dst = dst.wrapping_add(1);
                            src = src.wrapping_add(1);
                            *dst = *src;
                            dst = dst.wrapping_add(1);
                            src = src.wrapping_add(1);
                        } else {
                            dst = dst.wrapping_add(3);
                            src = src.wrapping_add(2);
                        }
                        continue;
                    }

                    if c == b'\\' as i32 && !crate::memory::cstr_is_empty(src) {
                        match crate::memory::cstr_first_byte(src) {
                            b'r' => {
                                c = CAR;
                                src = src.wrapping_add(1);
                            }
                            b'n' => {
                                c = NL;
                                src = src.wrapping_add(1);
                            }
                            b't' => {
                                c = TAB;
                                src = src.wrapping_add(1);
                            }
                            b'b' => {
                                c = CTRL_H_CONST;
                                src = src.wrapping_add(1);
                            }
                            _ => {
                                if (flags & REGSUB_BACKSLASH) != 0 {
                                    if copy {
                                        if dst.wrapping_add(1) > dest.wrapping_add(destlen as usize)
                                        {
                                            iemsg(c"vim_regsub_both(): not enough space");
                                            return 0;
                                        }
                                        *dst = b'\\' as c_char;
                                    }
                                    dst = dst.wrapping_add(1);
                                }
                                c = crate::memory::cstr_first_byte(src) as i32;
                                src = src.wrapping_add(1);
                            }
                        }
                    } else {
                        c = utf_ptr2char(src.wrapping_sub(1));
                    }

                    let cc = if let Some(f) = func_one.take() {
                        f.apply(c)
                    } else if let Some(f) = func_all {
                        f.apply(c)
                    } else {
                        c
                    };

                    let totlen = utfc_ptr2len(src.wrapping_sub(1)) as usize;
                    let charlen = utf_char2len(cc) as usize;

                    if copy {
                        if dst.wrapping_add(charlen) > dest.wrapping_add(destlen as usize) {
                            iemsg(c"vim_regsub_both(): not enough space");
                            return 0;
                        }
                        utf_char2bytes(cc, dst);
                    }
                    dst = dst.wrapping_offset(charlen as isize - 1);
                    let clen = utf_ptr2len(src.wrapping_sub(1)) as usize;

                    if clen < totlen {
                        if copy {
                            if dst.wrapping_add(totlen - clen) > dest.wrapping_add(destlen as usize)
                            {
                                iemsg(c"vim_regsub_both(): not enough space");
                                return 0;
                            }
                            ptr::copy(
                                src.wrapping_sub(1).wrapping_add(clen),
                                dst.wrapping_add(1),
                                totlen - clen,
                            );
                        }
                        dst = dst.wrapping_add(totlen - clen);
                    }
                    src = src.wrapping_add(totlen - 1);
                    dst = dst.wrapping_add(1);
                } else {
                    let mut s: *mut c_char;
                    if reg_multi() {
                        clnum = re_st().rex.mmatch_mut().startpos[no as usize].lnum;
                        if clnum < 0 || re_st().rex.mmatch_mut().endpos[no as usize].lnum < 0 {
                            s = ptr::null_mut();
                        } else {
                            s = reg_getline(clnum).wrapping_add(
                                re_st().rex.mmatch_mut().startpos[no as usize].col as usize,
                            );
                            if re_st().rex.mmatch_mut().endpos[no as usize].lnum == clnum {
                                len = re_st().rex.mmatch_mut().endpos[no as usize].col
                                    - re_st().rex.mmatch_mut().startpos[no as usize].col;
                            } else {
                                len = reg_getline_len(clnum)
                                    - re_st().rex.mmatch_mut().startpos[no as usize].col;
                            }
                        }
                    } else {
                        s = re_st().rex.match_mut().startp[no as usize];
                        if re_st().rex.match_mut().endp[no as usize].is_null() {
                            s = ptr::null_mut();
                        } else {
                            len = re_st().rex.match_mut().endp[no as usize].offset_from(s) as i32;
                        }
                    }
                    if !s.is_null() {
                        loop {
                            if len == 0 {
                                if reg_multi() {
                                    if re_st().rex.mmatch_mut().endpos[no as usize].lnum == clnum {
                                        break;
                                    }
                                    if copy {
                                        if dst.wrapping_add(1) > dest.wrapping_add(destlen as usize)
                                        {
                                            iemsg(c"vim_regsub_both(): not enough space");
                                            return 0;
                                        }
                                        *dst = CAR as c_char;
                                    }
                                    dst = dst.wrapping_add(1);
                                    clnum += 1;
                                    s = reg_getline(clnum);
                                    if re_st().rex.mmatch_mut().endpos[no as usize].lnum == clnum {
                                        len = re_st().rex.mmatch_mut().endpos[no as usize].col;
                                    } else {
                                        len = reg_getline_len(clnum);
                                    }
                                } else {
                                    break;
                                }
                            } else if crate::memory::cstr_is_empty(s) {
                                if copy {
                                    iemsg(tr(E_RE_DAMG));
                                }
                                return (dst.offset_from(dest) + 1) as i32;
                            } else {
                                if (flags & REGSUB_BACKSLASH) != 0
                                    && (*s == CAR as c_char
                                        || crate::memory::cstr_first_byte(s) == b'\\')
                                {
                                    if copy {
                                        if dst.wrapping_add(2) > dest.wrapping_add(destlen as usize)
                                        {
                                            iemsg(c"vim_regsub_both(): not enough space");
                                            return 0;
                                        }
                                        *dst = b'\\' as c_char;
                                        *dst.wrapping_add(1) = *s;
                                    }
                                    dst = dst.wrapping_add(2);
                                } else {
                                    c = utf_ptr2char(s);
                                    let cc = if let Some(f) = func_one.take() {
                                        f.apply(c)
                                    } else if let Some(f) = func_all {
                                        f.apply(c)
                                    } else {
                                        c
                                    };

                                    let l = utf_ptr2len(s) - 1;
                                    s = s.wrapping_add(l as usize);
                                    len -= l;
                                    let charlen = utf_char2len(cc) as usize;
                                    if copy {
                                        if dst.wrapping_add(charlen)
                                            > dest.wrapping_add(destlen as usize)
                                        {
                                            iemsg(c"vim_regsub_both(): not enough space");
                                            return 0;
                                        }
                                        utf_char2bytes(cc, dst);
                                    }
                                    dst = dst.wrapping_offset(charlen as isize - 1);
                                    dst = dst.wrapping_add(1);
                                }
                                s = s.wrapping_add(1);
                                len -= 1;
                            }
                        }
                    }
                    no = -1;
                }
            }
        }
        if copy {
            *dst = NUL as c_char;
        }
        (dst.offset_from(dest) + 1) as i32
    }
}

fn reg_getline_submatch(lnum: Linenr) -> *mut c_char {
    let mut line: *mut c_char = ptr::null_mut();
    reg_getline_common(lnum, RGLF_LINE | RGLF_SUBMATCH, &raw mut line, None);
    line
}

fn reg_getline_submatch_len(lnum: Linenr) -> Colnr {
    let mut length: Colnr = 0;
    reg_getline_common(
        lnum,
        RGLF_LENGTH | RGLF_SUBMATCH,
        ptr::null_mut(),
        Some(&mut length),
    );
    length
}

pub fn reg_submatch(no: i32) -> Option<std::ffi::CString> {
    unsafe {
        let mut retval_vec: Vec<c_char>;
        let mut retval: *mut c_char = ptr::null_mut();
        if !re_st().can_f_submatch || no < 0 {
            return None;
        }
        let no = no as usize;

        if re_st().rsm.sm_match.is_null() {
            for round in 1..=2 {
                let mut lnum = re_st().rsm.mmatch_mut().startpos[no].lnum;
                if lnum < 0 || re_st().rsm.mmatch_mut().endpos[no].lnum < 0 {
                    return None;
                }

                let mut s = reg_getline_submatch(lnum);
                if s.is_null() {
                    break;
                }
                s = s.wrapping_add(re_st().rsm.mmatch_mut().startpos[no].col as usize);
                let mut len: isize;
                if re_st().rsm.mmatch_mut().endpos[no].lnum == lnum {
                    len = (re_st().rsm.mmatch_mut().endpos[no].col
                        - re_st().rsm.mmatch_mut().startpos[no].col)
                        as isize;
                    if round == 2 {
                        crate::memory::cstr_ncpy(retval, s, len as usize);
                    }
                    len += 1;
                } else {
                    len = (reg_getline_submatch_len(lnum)
                        - re_st().rsm.mmatch_mut().startpos[no].col)
                        as isize;
                    if round == 2 {
                        crate::memory::cstr_cpy(retval, s);
                        *retval.wrapping_add(len as usize) = b'\n' as c_char;
                    }
                    len += 1;
                    lnum += 1;
                    while lnum < re_st().rsm.mmatch_mut().endpos[no].lnum {
                        s = reg_getline_submatch(lnum);
                        if round == 2 {
                            crate::memory::cstr_cpy(retval.wrapping_add(len as usize), s);
                        }
                        len += reg_getline_submatch_len(lnum) as isize;
                        if round == 2 {
                            *retval.wrapping_add(len as usize) = b'\n' as c_char;
                        }
                        len += 1;
                        lnum += 1;
                    }
                    if round == 2 {
                        {
                            let dst = retval.wrapping_add(len as usize);
                            let src = reg_getline_submatch(lnum);
                            let cap_n = re_st().rsm.mmatch_mut().endpos[no].col as usize;
                            let copy_n = crate::memory::xstrnlen(src, cap_n);
                            ptr::copy_nonoverlapping(src, dst, copy_n);
                            ptr::write_bytes(dst.wrapping_add(copy_n), 0, cap_n - copy_n);
                        }
                    }
                    len += re_st().rsm.mmatch_mut().endpos[no].col as isize;
                    if round == 2 {
                        *retval.wrapping_add(len as usize) = NUL as c_char;
                    }
                    len += 1;
                }

                if retval.is_null() {
                    retval_vec = vec![0; len as usize];
                    retval = retval_vec.as_mut_ptr();
                }
            }
            (!retval.is_null()).then(|| crate::memory::cstr_to_cstring(retval))
        } else {
            let s = re_st().rsm.match_mut().startp[no];
            if s.is_null() || re_st().rsm.match_mut().endp[no].is_null() {
                None
            } else {
                cstring_from_bytes(s, re_st().rsm.match_mut().endp[no].offset_from(s) as usize)
            }
        }
    }
}

pub fn reg_submatch_list(no: i32) -> *mut TvList {
    unsafe {
        if !re_st().can_f_submatch || no < 0 {
            return ptr::null_mut();
        }
        let no = no as usize;
        let list: *mut TvList;

        if re_st().rsm.sm_match.is_null() {
            let slnum = re_st().rsm.mmatch_mut().startpos[no].lnum;
            let elnum = re_st().rsm.mmatch_mut().endpos[no].lnum;
            if slnum < 0 || elnum < 0 {
                return ptr::null_mut();
            }

            let scol = re_st().rsm.mmatch_mut().startpos[no].col;
            let ecol = re_st().rsm.mmatch_mut().endpos[no].col;

            list = tv_list_alloc((elnum - slnum + 1) as isize);

            let mut s: *const c_char = reg_getline_submatch(slnum).wrapping_add(scol as usize);
            if slnum == elnum {
                tv_list_append_string(&mut *list, s, (ecol - scol) as isize);
            } else {
                let max_lnum = elnum - slnum;
                tv_list_append_string(&mut *list, s, -1);
                for i in 1..max_lnum {
                    s = reg_getline_submatch(slnum + i);
                    tv_list_append_string(&mut *list, s, -1);
                }
                s = reg_getline_submatch(elnum);
                tv_list_append_string(&mut *list, s, ecol as isize);
            }
        } else {
            let s = re_st().rsm.match_mut().startp[no];
            if s.is_null() || re_st().rsm.match_mut().endp[no].is_null() {
                return ptr::null_mut();
            }
            list = tv_list_alloc(1);
            tv_list_append_string(
                &mut *list,
                s,
                re_st().rsm.match_mut().endp[no].offset_from(s),
            );
        }

        tv_list_ref(list.as_mut());
        list
    }
}

pub(super) fn init_regexec_multi(
    rmp: &mut Regmmatch,
    win: *mut WinInfoInfo,
    buf: *mut FileBuf,
    lnum: Linenr,
) {
    {
        re_st().rex.reg_match = ptr::null_mut();
        re_st().rex.reg_mmatch = rmp;
        re_st().rex.reg_buf = buf;
        re_st().rex.reg_win = win;
        re_st().rex.reg_firstlnum = lnum;
        re_st().rex.reg_maxline = re_st().rex.buf_mut().b_ml.ml_line_count - lnum;
        re_st().rex.reg_line_lbr = false;
        re_st().rex.reg_ic = rmp.rmm_ic != 0;
        re_st().rex.reg_icombine = false;
        re_st().rex.reg_nobreak = (rmp.prog_mut().re_flags & (RE_NOBREAK as u32)) != 0;
        re_st().rex.reg_maxcol = rmp.rmm_maxcol;
    }
}

pub(super) fn get_char_class_bits(start: *const u8, end: *const u8, newl: &mut bool) -> i32 {
    unsafe {
        let mut config: i32 = 0;
        *newl = false;
        if *end != b']' {
            return -1;
        }
        let mut p = start;
        if *p == b'^' {
            config |= CLASS_NOT;
            p = p.wrapping_add(1);
        }

        while p < end {
            if p.wrapping_add(2) < end && *p.wrapping_add(1) == b'-' {
                match *p {
                    b'0' => {
                        if *p.wrapping_add(2) == b'9' {
                            config |= CLASS_O9;
                        } else if *p.wrapping_add(2) == b'7' {
                            config |= CLASS_O7;
                        } else {
                            return -1;
                        }
                    }
                    b'a' => {
                        if *p.wrapping_add(2) == b'z' {
                            config |= CLASS_AZ_CONST;
                        } else if *p.wrapping_add(2) == b'f' {
                            config |= CLASS_AF_CONST;
                        } else {
                            return -1;
                        }
                    }
                    b'A' => {
                        if *p.wrapping_add(2) == b'Z' {
                            config |= CLASS_AZ;
                        } else if *p.wrapping_add(2) == b'F' {
                            config |= CLASS_AF;
                        } else {
                            return -1;
                        }
                    }
                    _ => return -1,
                }
                p = p.wrapping_add(3);
            } else if p.wrapping_add(1) < end && *p == b'\\' && *p.wrapping_add(1) == b'n' {
                *newl = true;
                p = p.wrapping_add(2);
            } else if *p == b'_' {
                config |= CLASS_UNDERSCORE;
                p = p.wrapping_add(1);
            } else if *p == b'\n' {
                *newl = true;
                p = p.wrapping_add(1);
            } else {
                return -1;
            }
        }

        if p != end {
            return -1;
        }
        if (config & (CLASS_AZ_CONST | CLASS_AZ)) != 0 {
            return -1;
        }
        config
    }
}

pub(super) fn regcomp_start(expr: *mut u8, re_flags: i32) {
    initchr(expr.cast());
    if (re_flags & RE_MAGIC) != 0 {
        re_st().reg_magic = MAGIC_ON;
    } else {
        re_st().reg_magic = MAGIC_OFF;
    }
    re_st().reg_string = re_flags & RE_STRING;
    re_st().reg_strict = re_flags & RE_STRICT;
    get_cpo_flags();

    re_st().num_complex_braces = 0;
    re_st().regnpar = 1;
    re_st().had_endbrace = [0; NSUBEXP];
    re_st().regnzpar = 1;
    re_st().re_has_z = 0;
    re_st().regsize = 0;
    re_st().reg_toolong = 0;
    re_st().bt_reg_parse_depth = 0;
    re_st().regflags = 0;
    re_st().had_eol = 0;
}

pub(super) fn seen_endbrace(refnum: i32) -> bool {
    unsafe {
        if re_st().had_endbrace[refnum as usize] == 0 {
            let mut p = re_st().regparse.cast::<u8>();
            while *p != NUL {
                if *p == b'@'
                    && *p.wrapping_add(1) == b'<'
                    && (*p.wrapping_add(2) == b'!' || *p.wrapping_add(2) == b'=')
                {
                    break;
                }
                p = p.wrapping_add(1);
            }
            if *p == NUL {
                emsg(tr(c"E65: Illegal back reference"));
                g_st().rc_did_emsg = true;
                return false;
            }
        }
        true
    }
}

pub(super) fn coll_get_char() -> i32 {
    {
        let mut nr: i64 = -1;
        let ch = crate::memory::cstr_first_byte(re_st().regparse);
        re_st().regparse = re_st().regparse.wrapping_add(1);
        match ch {
            b'd' => nr = getdecchrs(),
            b'o' => nr = getoctchrs(),
            b'x' => nr = gethexchrs(2),
            b'u' => nr = gethexchrs(4),
            b'U' => nr = gethexchrs(8),
            _ => {}
        }
        if nr < 0 {
            re_st().regparse = re_st().regparse.wrapping_sub(1);
            nr = i64::from(b'\\');
        }
        if nr > i64::from(i32::MAX) {
            nr = i64::from(i32::MAX);
        }
        nr as i32
    }
}

pub fn vim_regcomp_had_eol() -> i32 {
    re_st().had_eol
}

/// Compile a regular expression into internal code.
pub fn vim_regcomp(expr_arg: impl crate::strings::IntoCCharPtr, re_flags: i32) -> *mut Regprog {
    let expr_arg = expr_arg.into_c_char_ptr();
    {
        let mut expr = expr_arg;

        re_st().regexp_engine = opt_vars().p_re as i32;

        if crate::memory::cstr_prefix_bytes(expr, 4) == &b"\\%#="[..4] {
            let newengine = (crate::memory::cstr_byte_at(expr, 4)).wrapping_sub(b'0') as i32;
            if newengine == AUTOMATIC_ENGINE
                || newengine == BACKTRACKING_ENGINE
                || newengine == NFA_ENGINE
            {
                re_st().regexp_engine = newengine;
                expr = expr.wrapping_add(5);
            } else {
                emsg(tr(
                    c"E864: \\%#= can only be followed by 0, 1, or 2. The automatic engine will be used ",
                ));
                re_st().regexp_engine = AUTOMATIC_ENGINE;
            }
        }

        re_st().rex.reg_buf = g_st().curbuf;

        let called_emsg_before = g_st().called_emsg;
        let mut prog_opt = if re_st().regexp_engine != BACKTRACKING_ENGINE {
            nfa_regcomp(
                expr.cast_mut().cast(),
                re_flags
                    + if re_st().regexp_engine == AUTOMATIC_ENGINE {
                        RE_AUTO
                    } else {
                        0
                    },
            )
            .map(|mut nfa| {
                nfa.re_engine = re_st().regexp_engine as u32;
                Regprog {
                    regflags: nfa.regflags,
                    re_engine: re_st().regexp_engine as u32,
                    re_flags: re_flags as u32,
                    re_in_use: false,
                    engine: RegProgEngine::Nfa(nfa),
                }
            })
        } else {
            bt_regcomp(expr.cast_mut().cast(), re_flags).map(|bt| Regprog {
                regflags: bt.regflags,
                re_engine: re_st().regexp_engine as u32,
                re_flags: re_flags as u32,
                re_in_use: false,
                engine: RegProgEngine::Bt(bt),
            })
        };

        if prog_opt.is_none()
            && re_st().regexp_engine == AUTOMATIC_ENGINE
            && g_st().called_emsg == called_emsg_before
        {
            re_st().regexp_engine = BACKTRACKING_ENGINE;
            report_re_switch(expr);
            prog_opt = bt_regcomp(expr.cast_mut().cast(), re_flags).map(|bt| Regprog {
                regflags: bt.regflags,
                re_engine: re_st().regexp_engine as u32,
                re_flags: re_flags as u32,
                re_in_use: false,
                engine: RegProgEngine::Bt(bt),
            });
        }

        if let Some(prog) = prog_opt {
            Box::into_raw(Box::new(prog))
        } else {
            ptr::null_mut()
        }
    }
}

#[inline]
pub fn vim_regcomp_box(
    expr_arg: impl crate::strings::IntoCCharPtr,
    re_flags: i32,
) -> Option<Box<Regprog>> {
    let expr_arg = expr_arg.into_c_char_ptr();
    unsafe {
        let p = vim_regcomp(expr_arg, re_flags);
        if p.is_null() {
            None
        } else {
            Some(Box::from_raw(p))
        }
    }
}

#[inline]
pub fn regprog_as_ptr(prog: &Option<Box<Regprog>>) -> *mut Regprog {
    prog.as_deref()
        .map_or(ptr::null_mut(), |p| ptr::from_ref(p).cast_mut())
}

/// Free a compiled regexp program, returned by `vim_regcomp()`.
pub fn vim_regfree(prog: *mut Regprog) {
    unsafe {
        if !prog.is_null() {
            drop(Box::from_raw(prog));
        }
    }
}

pub fn free_regexp_stuff() {
    {
        re_st().regstack = Vec::new();
        re_st().backpos = Vec::new();
        re_st().reg_tofree = Vec::new();
        re_st().reg_prev_sub = None;
        re_st().post_vec = Vec::new();
    }
}

fn report_re_switch(pat: impl crate::strings::IntoCCharPtr) {
    let pat = pat.into_c_char_ptr();
    if opt_vars().p_verbose > 0 {
        verbose_enter();
        msg_puts(tr(c"Switching to backtracking RE engine for pattern: "));
        msg_puts(pat);
        verbose_leave();
    }
}

fn vim_regexec_string(
    rmp: &mut Regmatch,
    line: impl crate::strings::IntoCCharPtr,
    col: Colnr,
    nl: bool,
) -> bool {
    let line = line.into_c_char_ptr();
    unsafe {
        let mut rex_save = Regexec::new();
        let rex_in_use_save = re_st().rex_in_use;

        if rmp.prog_mut().re_in_use {
            emsg(tr(E_RECURSIVE));
            return false;
        }
        rmp.prog_mut().re_in_use = true;

        if re_st().rex_in_use {
            rex_save = re_st().rex;
        }
        re_st().rex_in_use = true;

        re_st().rex.reg_startp = ptr::null_mut();
        re_st().rex.reg_endp = ptr::null_mut();
        re_st().rex.reg_startpos = ptr::null_mut();
        re_st().rex.reg_endpos = ptr::null_mut();

        let mut result = match rmp.prog_mut().engine {
            RegProgEngine::Bt(_) => bt_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
            RegProgEngine::Nfa(_) => nfa_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
        };
        rmp.prog_mut().re_in_use = false;

        if rmp.prog_mut().re_engine == AUTOMATIC_ENGINE as u32 && result == NFA_TOO_EXPENSIVE {
            let save_p_re = opt_vars().p_re;
            let re_flags = rmp.prog_mut().re_flags as i32;
            let prev_prog = rmp.regprog;
            let pat = (*prev_prog).nfa_mut().pattern.as_ptr();

            opt_vars().p_re = OptInt::from(BACKTRACKING_ENGINE);
            report_re_switch(pat);
            let new_prog = vim_regcomp(pat, re_flags);
            if !new_prog.is_null() {
                *prev_prog = *Box::from_raw(new_prog);
                rmp.prog_mut().re_in_use = true;
                result = match rmp.prog_mut().engine {
                    RegProgEngine::Bt(_) => bt_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
                    RegProgEngine::Nfa(_) => nfa_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
                };
                rmp.prog_mut().re_in_use = false;
            }

            opt_vars().p_re = save_p_re;
        }

        re_st().rex_in_use = rex_in_use_save;
        if re_st().rex_in_use {
            re_st().rex = rex_save;
        }

        result > 0
    }
}

pub fn vim_regexec_prog(
    prog: Option<&Regprog>,
    ignore_case: bool,
    line: impl crate::strings::IntoCCharPtr,
    col: Colnr,
) -> bool {
    let line = line.into_c_char_ptr();
    let mut regmatch = Regmatch {
        regprog: prog.map_or(ptr::null_mut(), |p| ptr::from_ref(p).cast_mut()),
        startp: [ptr::null_mut(); NSUBEXP],
        endp: [ptr::null_mut(); NSUBEXP],
        rm_matchcol: 0,
        rm_ic: ignore_case,
    };
    vim_regexec_string(&mut regmatch, line, col, false)
}

pub fn vim_regexec(
    rmp: &mut Regmatch,
    line: impl crate::strings::IntoCCharPtr,
    col: Colnr,
) -> bool {
    let line = line.into_c_char_ptr();
    vim_regexec_string(rmp, line, col, false)
}

pub fn vim_regexec_nl(
    rmp: &mut Regmatch,
    line: impl crate::strings::IntoCCharPtr,
    col: Colnr,
) -> bool {
    let line = line.into_c_char_ptr();
    vim_regexec_string(rmp, line, col, true)
}

pub fn vim_regexec_multi(
    rmp: &mut Regmmatch,
    win: *mut WinInfoInfo,
    buf: *mut FileBuf,
    lnum: Linenr,
    col: Colnr,
    tm: Option<Proftime>,
    mut timed_out: Option<&mut i32>,
) -> i32 {
    unsafe {
        let mut rex_save = Regexec::new();
        let rex_in_use_save = re_st().rex_in_use;

        if rmp.prog_mut().re_in_use {
            emsg(tr(E_RECURSIVE));
            return 0;
        }
        rmp.prog_mut().re_in_use = true;

        if re_st().rex_in_use {
            rex_save = re_st().rex;
        }
        re_st().rex_in_use = true;

        let mut result = match rmp.prog_mut().engine {
            RegProgEngine::Bt(_) => bt_regexec_multi(
                rmp,
                win,
                buf,
                lnum,
                col,
                tm,
                (timed_out.is_some()).then(|| &mut *timed_out.as_deref_mut().unwrap()),
            ),
            RegProgEngine::Nfa(_) => nfa_regexec_multi(
                rmp,
                win,
                buf,
                lnum,
                col,
                tm,
                (timed_out.is_some()).then(|| &mut *timed_out.as_deref_mut().unwrap()),
            ),
        };
        rmp.prog_mut().re_in_use = false;

        if rmp.prog_mut().re_engine == AUTOMATIC_ENGINE as u32 && result == NFA_TOO_EXPENSIVE {
            let save_p_re = opt_vars().p_re;
            let re_flags = rmp.prog_mut().re_flags as i32;
            let prev_prog = rmp.regprog;
            let pat = (*prev_prog).nfa_mut().pattern.as_ptr();

            opt_vars().p_re = OptInt::from(BACKTRACKING_ENGINE);

            report_re_switch(pat);
            g_st().reg_do_extmatch = REX_ALL;
            let new_prog = vim_regcomp(pat, re_flags);
            g_st().reg_do_extmatch = 0;

            if !new_prog.is_null() {
                *prev_prog = *Box::from_raw(new_prog);
                rmp.prog_mut().re_in_use = true;
                result = match rmp.prog_mut().engine {
                    RegProgEngine::Bt(_) => bt_regexec_multi(
                        rmp,
                        win,
                        buf,
                        lnum,
                        col,
                        tm,
                        (timed_out.is_some()).then(|| &mut *timed_out.as_deref_mut().unwrap()),
                    ),
                    RegProgEngine::Nfa(_) => nfa_regexec_multi(
                        rmp,
                        win,
                        buf,
                        lnum,
                        col,
                        tm,
                        (timed_out.is_some()).then(|| &mut *timed_out.unwrap()),
                    ),
                };
                rmp.prog_mut().re_in_use = false;
            }

            opt_vars().p_re = save_p_re;
        }

        re_st().rex_in_use = rex_in_use_save;
        if re_st().rex_in_use {
            re_st().rex = rex_save;
        }

        if result <= 0 { 0 } else { result }
    }
}

impl Regexec {
    #[inline]
    pub fn mmatch_mut(&self) -> &'static mut Regmmatch {
        unsafe { &mut *self.reg_mmatch }
    }
    #[inline]
    pub fn match_mut(&self) -> &'static mut Regmatch {
        unsafe { &mut *self.reg_match }
    }
    #[inline]
    pub fn buf_mut(&self) -> &'static mut FileBuf {
        unsafe { &mut *self.reg_buf }
    }
    #[inline]
    pub fn win_mut(&self) -> &'static mut WinInfoInfo {
        unsafe { &mut *self.reg_win }
    }
}

impl Regsubmatch {
    #[inline]
    pub fn mmatch_mut(&self) -> &'static mut Regmmatch {
        unsafe { &mut *self.sm_mmatch }
    }
    #[inline]
    pub fn match_mut(&self) -> &'static mut Regmatch {
        unsafe { &mut *self.sm_match }
    }
}

impl Regmmatch {
    #[inline]
    pub fn prog_mut(&self) -> &'static mut Regprog {
        unsafe { &mut *self.regprog }
    }
}

impl Regmatch {
    #[inline]
    pub fn prog_mut(&self) -> &'static mut Regprog {
        unsafe { &mut *self.regprog }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_helpers_round_trip() {
        let c = b'*' as i32;
        let m = magic_fn(c);
        assert!(is_magic_fn(m));
        assert_eq!(un_magic(m), c);
        assert_eq!(no_magic(m), c);
        assert_eq!(no_magic(c), c);
        assert_eq!(toggle_magic(c), m);
        assert_eq!(toggle_magic(m), c);
    }

    #[test]
    fn re_multi_type_classifies_operators() {
        assert_eq!(re_multi_type(magic_fn(b'*' as i32)), MULTI_MULT);
        assert_eq!(re_multi_type(magic_fn(b'+' as i32)), MULTI_MULT);
        assert_eq!(re_multi_type(magic_fn(b'{' as i32)), MULTI_MULT);
        assert_eq!(re_multi_type(magic_fn(b'=' as i32)), MULTI_ONE);
        assert_eq!(re_multi_type(magic_fn(b'?' as i32)), MULTI_ONE);
        assert_eq!(re_multi_type(magic_fn(b'@' as i32)), MULTI_ONE);
        assert_eq!(re_multi_type(b'*' as i32), NOT_MULTI);
    }

    #[test]
    fn get_char_class_bits_recognizes_digit_and_hex() {
        {
            let mut newl = false;
            let digits = b"0-9]";
            assert_eq!(
                get_char_class_bits(digits.as_ptr(), digits.as_ptr().wrapping_add(3), &mut newl),
                CLASS_O9
            );
            assert!(!newl);

            let hex_nl = b"^0-9a-fA-F\\n]";
            assert_eq!(
                get_char_class_bits(hex_nl.as_ptr(), hex_nl.as_ptr().wrapping_add(12), &mut newl),
                CLASS_NOT | CLASS_O9 | CLASS_AF_CONST | CLASS_AF
            );
            assert!(newl);
        }
    }
}
