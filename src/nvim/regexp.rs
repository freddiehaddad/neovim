//! Regular expression compilation, execution, and substitution (`src/nvim/regexp.c`).

use crate::buffer::buf_T;
pub(super) use crate::charset::{ascii_isdigit, ascii_iswhite, ascii_isxdigit};
use crate::charset::{
    getdigits_int, hex2nr, vim_isIDc, vim_isfilec, vim_isprintc, vim_iswordc_buf, vim_iswordp_buf,
};
use crate::cursor::{colnr_T, linenr_T, lpos_T, lt};
use crate::errors::*;
use crate::eval::eval_to_string;
use crate::eval::typval::{
    VarType, list_T, staticList10_T, tv_clear, tv_list_alloc, tv_list_append_string, tv_list_first,
    tv_list_init_static10, tv_set_string, typval_T,
};
use crate::eval::userfunc::{funcexe_T, ufunc_T};
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
use crate::profile::{profile_passed_limit, proftime_T};
pub(super) use crate::strings::tr;
use crate::window::win_T;
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

#[path = "regexp_bt.rs"]
mod bt;
#[path = "regexp_nfa.rs"]
mod nfa;

use self::bt::*;
use self::nfa::*;

use crate::math::{FAIL, OK};
use crate::option::OptInt;
pub type magic_T = c_int;
pub type reg_getline_flags_T = c_uint;

pub const NSUBEXP: usize = 10;

pub const RE_MAGIC: c_int = 1;
pub const RE_STRING: c_int = 2;
pub const RE_STRICT: c_int = 4;
pub const RE_AUTO: c_int = 8;
pub const RE_NOBREAK: c_int = 16;

pub const REX_SET: c_int = 1;
pub const REX_USE: c_int = 2;
pub const REX_ALL: c_int = REX_SET | REX_USE;

pub const MAGIC_NONE: magic_T = 1;
pub const MAGIC_OFF: magic_T = 2;
pub const MAGIC_ON: magic_T = 3;
pub const MAGIC_ALL: magic_T = 4;

pub const REGSUB_COPY: c_int = 1;
pub const REGSUB_MAGIC: c_int = 2;
pub const REGSUB_BACKSLASH: c_int = 4;

pub(super) const Ctrl_H: c_int = 0x08;
pub(super) const Ctrl_V: c_int = 0x16;

pub(super) const MAXCOL: colnr_T = 0x7fffffff;
pub(super) const NUMBUFLEN: usize = 65;
pub(super) const MB_MAXBYTES: c_int = 30;
pub(super) const MAX_MCO: usize = 6;
pub(super) const GRAPHEME_STATE_INIT: GraphemeState = 0;
pub(super) const kMarkBufLocal: c_int = 0;
pub(super) const kCpoLiteral: c_int = b'l' as c_int;

pub(super) const RGLF_LINE: reg_getline_flags_T = 0x01;
pub(super) const RGLF_LENGTH: reg_getline_flags_T = 0x02;
pub(super) const RGLF_SUBMATCH: reg_getline_flags_T = 0x04;

pub(super) const NFA_MAX_STATES: c_int = 100000;
pub(super) const NFA_TOO_EXPENSIVE: c_int = -1;

pub(super) const AUTOMATIC_ENGINE: c_int = 0;
pub(super) const BACKTRACKING_ENGINE: c_int = 1;
pub(super) const NFA_ENGINE: c_int = 2;

pub(super) const REGMAGIC: c_int = 0o234;
pub(super) const MAX_LIMIT: c_int = 32767 << 16;

pub(super) const NOT_MULTI: c_int = 0;
pub(super) const MULTI_ONE: c_int = 1;
pub(super) const MULTI_MULT: c_int = 2;

pub(super) const RA_FAIL: c_int = 1;
pub(super) const RA_CONT: c_int = 2;
pub(super) const RA_BREAK: c_int = 3;
pub(super) const RA_MATCH: c_int = 4;
pub(super) const RA_NOMATCH: c_int = 5;

pub(super) const CLASS_ALNUM: c_int = 0;
pub(super) const CLASS_ALPHA: c_int = 1;
pub(super) const CLASS_BLANK: c_int = 2;
pub(super) const CLASS_CNTRL: c_int = 3;
pub(super) const CLASS_DIGIT: c_int = 4;
pub(super) const CLASS_GRAPH: c_int = 5;
pub(super) const CLASS_LOWER: c_int = 6;
pub(super) const CLASS_PRINT: c_int = 7;
pub(super) const CLASS_PUNCT: c_int = 8;
pub(super) const CLASS_SPACE: c_int = 9;
pub(super) const CLASS_UPPER: c_int = 10;
pub(super) const CLASS_XDIGIT: c_int = 11;
pub(super) const CLASS_TAB: c_int = 12;
pub(super) const CLASS_RETURN: c_int = 13;
pub(super) const CLASS_BACKSPACE: c_int = 14;
pub(super) const CLASS_ESCAPE: c_int = 15;
pub(super) const CLASS_IDENT: c_int = 16;
pub(super) const CLASS_KEYWORD: c_int = 17;
pub(super) const CLASS_FNAME: c_int = 18;
pub(super) const CLASS_NONE: c_int = 99;

pub(super) const RI_DIGIT: i16 = 0x01;
pub(super) const RI_HEX: i16 = 0x02;
pub(super) const RI_OCTAL: i16 = 0x04;
pub(super) const RI_WORD: i16 = 0x08;
pub(super) const RI_HEAD: i16 = 0x10;
pub(super) const RI_ALPHA: i16 = 0x20;
pub(super) const RI_LOWER: i16 = 0x40;
pub(super) const RI_UPPER: i16 = 0x80;
pub(super) const RI_WHITE: i16 = 0x100;

pub(super) const RF_ICASE: c_uint = 1;
pub(super) const RF_NOICASE: c_uint = 2;
pub(super) const RF_HASNL: c_uint = 4;
pub(super) const RF_ICOMBINE: c_uint = 8;
pub(super) const RF_LOOKBH: c_uint = 16;

pub(super) const REG_NOPAREN: c_int = 0;
pub(super) const REG_PAREN: c_int = 1;
pub(super) const REG_ZPAREN: c_int = 2;
pub(super) const REG_NPAREN: c_int = 3;

pub(super) const REG_MAX_PAREN_DEPTH: c_int = 1000;
pub(super) const MAX_REGSUB_NESTING: usize = 4;

pub(super) const CLASS_not: c_int = 0x80;
pub(super) const CLASS_af: c_int = 0x40;
pub(super) const CLASS_AF: c_int = 0x20;
pub(super) const CLASS_az: c_int = 0x10;
pub(super) const CLASS_AZ: c_int = 0x08;
pub(super) const CLASS_o7: c_int = 0x04;
pub(super) const CLASS_o9: c_int = 0x02;
pub(super) const CLASS_underscore: c_int = 0x01;

pub(super) const REGEXP_INRANGE: &CStr = c"]^-n\\";
pub(super) const REGEXP_ABBR: &CStr = c"nrtebdoxuU";
pub(super) const classchars: &[u8; 28] = b".iIkKfFpPsSdDxXoOwWhHaAlLuU\0";

pub(super) const e_invalid_character_after_str_at: &CStr = c"E59: Invalid character after %s@";
pub(super) const e_invalid_use_of_underscore: &CStr = c"E63: Invalid use of \\_";
pub(super) const e_pattern_uses_more_memory_than_maxmempattern: &CStr =
    c"E363: Pattern uses more memory than 'maxmempattern'";
pub(super) const e_invalid_item_in_str_brackets: &CStr = c"E369: Invalid item in %s%%[]";
pub(super) const e_missing_delimiter_after_search_pattern_str: &CStr =
    c"E654: Missing delimiter after search pattern: %s";
pub(super) const e_missingbracket: &CStr = c"E769: Missing ] after %s[";
pub(super) const e_reverse_range: &CStr = c"E944: Reverse range in character class";
pub(super) const e_large_class: &CStr = c"E945: Range too large in character class";
pub(super) const e_unmatched_z: &CStr = c"E52: Unmatched \\z(";
pub(super) const e_unmatched_str_percent_open: &CStr = c"E53: Unmatched %s%%(";
pub(super) const e_unmatched_str_open: &CStr = c"E54: Unmatched %s(";
pub(super) const e_unmatched_str_close: &CStr = c"E55: Unmatched %s)";
pub(super) const e_z_not_allowed: &CStr = c"E66: \\z( not allowed here";
pub(super) const e_z1_not_allowed: &CStr = c"E67: \\z1 - \\z9 not allowed here";
pub(super) const e_missing_sb: &CStr = c"E69: Missing ] after %s%%[";
pub(super) const e_empty_sb: &CStr = c"E70: Empty %s%%[]";
pub(super) const e_recursive: &CStr = c"E956: Cannot use pattern recursively";
pub(super) const e_regexp_number_after_dot_pos_search_chr: &CStr =
    c"E1204: No Number allowed after .: '\\%%%c'";
pub(super) const e_nfa_regexp_missing_value_in_chr: &CStr =
    c"E1273: (NFA regexp) missing value in '\\%%%c'";
pub(super) const e_atom_engine_must_be_at_start_of_pattern: &CStr =
    c"E1281: Atom '\\%%#=%c' must be at the start of the pattern";
pub(super) const e_substitute_nesting_too_deep: &CStr = c"E1290: substitute nesting too deep";
pub(super) const e_unicode_val_too_large: &CStr =
    c"E1541: Value too large, max Unicode codepoint is U+10FFFF";

#[rustfmt::skip]
pub(super) static META_flags: [u8; 127] = [
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
    Bt(bt_regprog_T),
    Nfa(nfa_regprog_T),
}

pub struct regprog_T {
    pub regflags: c_uint,
    pub re_engine: c_uint,
    pub re_flags: c_uint,
    pub re_in_use: bool,
    pub(super) engine: RegProgEngine,
}

impl regprog_T {
    #[inline]
    pub(super) fn bt_mut(&mut self) -> &mut bt_regprog_T {
        match &mut self.engine {
            RegProgEngine::Bt(bt) => bt,
            RegProgEngine::Nfa(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn nfa_mut(&mut self) -> &mut nfa_regprog_T {
        match &mut self.engine {
            RegProgEngine::Nfa(nfa) => nfa,
            RegProgEngine::Bt(_) => unreachable!(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct regmmatch_T {
    pub regprog: *mut regprog_T,
    pub startpos: [lpos_T; NSUBEXP],
    pub endpos: [lpos_T; NSUBEXP],
    pub rmm_matchcol: colnr_T,
    pub rmm_ic: c_int,
    pub rmm_maxcol: colnr_T,
}

impl regmmatch_T {
    pub const fn new() -> Self {
        Self {
            regprog: ptr::null_mut(),
            startpos: [lpos_T::new(); NSUBEXP],
            endpos: [lpos_T::new(); NSUBEXP],
            rmm_matchcol: 0,
            rmm_ic: 0,
            rmm_maxcol: 0,
        }
    }
}

impl Default for regmmatch_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct regmatch_T {
    pub regprog: *mut regprog_T,
    pub startp: [*mut c_char; NSUBEXP],
    pub endp: [*mut c_char; NSUBEXP],
    pub rm_matchcol: colnr_T,
    pub rm_ic: bool,
}

impl regmatch_T {
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

impl Default for regmatch_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Default)]
pub struct reg_extmatch_T {
    pub matches: [Option<std::ffi::CString>; NSUBEXP],
}

#[inline]
pub(super) unsafe fn cstring_from_bytes(
    ptr: *const c_char,
    len: usize,
) -> Option<std::ffi::CString> {
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(len);
    std::ffi::CString::new(&bytes[..end]).ok()
}

pub(super) struct bt_regprog_T {
    pub regflags: c_uint,
    pub regstart: c_int,
    pub reganch: u8,
    pub regmust_off: Option<usize>,
    pub regmlen: c_int,
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
    pub(super) fn set(self, states: &mut [nfa_state_T], target: Option<usize>) {
        match self {
            Self::Out(idx) => states[idx].out = target,
            Self::Out1(idx) => states[idx].out1 = target,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct nfa_state_T {
    pub c: c_int,
    pub out: Option<usize>,
    pub out1: Option<usize>,
    pub lastlist: [c_int; 2],
    pub val: c_int,
}

impl nfa_state_T {
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

impl Default for nfa_state_T {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) struct nfa_regprog_T {
    pub regflags: c_uint,
    pub re_engine: c_uint,
    pub start: Option<usize>,
    pub reganch: c_int,
    pub regstart: c_int,
    pub match_text: Option<std::ffi::CString>,
    pub has_zend: c_int,
    pub has_backref: c_int,
    pub reghasz: c_int,
    pub pattern: std::ffi::CString,
    pub nsubexp: c_int,
    pub nstate: c_int,
    pub listbuf: [Vec<nfa_thread_T>; 2],
    pub state: Vec<nfa_state_T>,
}

#[derive(Clone, Copy)]
pub(super) enum RegPos {
    Ptr(*mut u8),
    Pos(lpos_T),
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
    pub(super) const fn pos(self) -> lpos_T {
        match self {
            Self::Pos(pos) => pos,
            Self::Ptr(_) => lpos_T::new(),
        }
    }

    #[inline]
    pub(super) fn pos_mut(&mut self) -> &mut lpos_T {
        if !matches!(*self, Self::Pos(_)) {
            *self = Self::Pos(lpos_T::new());
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
pub(super) struct regsave_T {
    pub rs_u: RegPos,
    pub rs_len: c_int,
}

impl regsave_T {
    pub(super) const fn new() -> Self {
        Self {
            rs_u: RegPos::new(),
            rs_len: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct save_se_T {
    pub se_u: RegPos,
}

impl save_se_T {
    pub(super) const fn new() -> Self {
        Self {
            se_u: RegPos::new(),
        }
    }
}

pub(super) type regstate_T = c_int;
pub(super) const RS_NOPEN: regstate_T = 0;
pub(super) const RS_MOPEN: regstate_T = 1;
pub(super) const RS_MCLOSE: regstate_T = 2;
pub(super) const RS_ZOPEN: regstate_T = 3;
pub(super) const RS_ZCLOSE: regstate_T = 4;
pub(super) const RS_BRANCH: regstate_T = 5;
pub(super) const RS_BRCPLX_MORE: regstate_T = 6;
pub(super) const RS_BRCPLX_LONG: regstate_T = 7;
pub(super) const RS_BRCPLX_SHORT: regstate_T = 8;
pub(super) const RS_NOMATCH: regstate_T = 9;
pub(super) const RS_BEHIND1: regstate_T = 10;
pub(super) const RS_BEHIND2: regstate_T = 11;
pub(super) const RS_STAR_LONG: regstate_T = 12;
pub(super) const RS_STAR_SHORT: regstate_T = 13;

#[derive(Clone, Copy)]
pub(super) enum RegItemSave {
    Se(save_se_T),
    Reg(regsave_T),
}

impl RegItemSave {
    pub(super) const fn new() -> Self {
        Self::Se(save_se_T::new())
    }

    #[inline]
    pub(super) fn sesave(&self) -> &save_se_T {
        match self {
            Self::Se(s) => s,
            Self::Reg(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn sesave_mut(&mut self) -> &mut save_se_T {
        if !matches!(*self, Self::Se(_)) {
            *self = Self::Se(save_se_T::new());
        }
        match self {
            Self::Se(s) => s,
            Self::Reg(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn regsave(&self) -> &regsave_T {
        match self {
            Self::Reg(r) => r,
            Self::Se(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn regsave_mut(&mut self) -> &mut regsave_T {
        if !matches!(*self, Self::Reg(_)) {
            *self = Self::Reg(regsave_T::new());
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
pub(super) struct regitem_T {
    pub rs_state: regstate_T,
    pub rs_no: i16,
    pub rs_scan: *mut u8,
    pub rs_un: RegItemSave,
}

impl regitem_T {
    pub(super) const fn new() -> Self {
        Self {
            rs_state: 0,
            rs_no: 0,
            rs_scan: ptr::null_mut(),
            rs_un: RegItemSave::new(),
        }
    }
}

impl Default for regitem_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct regbehind_T {
    pub save_after: regsave_T,
    pub save_behind: regsave_T,
    pub save_need_clear_subexpr: c_int,
    pub save_start: [save_se_T; NSUBEXP],
    pub save_end: [save_se_T; NSUBEXP],
}

pub(super) struct Frag_T {
    pub start: Option<usize>,
    pub out: Vec<NfaOutSlot>,
}

impl Frag_T {
    pub(super) const fn new() -> Self {
        Self {
            start: None,
            out: Vec::new(),
        }
    }
}

impl Default for Frag_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct multipos {
    pub start_lnum: linenr_T,
    pub end_lnum: linenr_T,
    pub start_col: colnr_T,
    pub end_col: colnr_T,
}

impl multipos {
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
pub(super) struct linepos {
    pub start: *mut u8,
    pub end: *mut u8,
}

impl linepos {
    pub(super) const fn new() -> Self {
        Self {
            start: ptr::null_mut(),
            end: ptr::null_mut(),
        }
    }
}

impl Default for linepos {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub(super) enum RegSubList {
    Multi([multipos; NSUBEXP]),
    Line([linepos; NSUBEXP]),
}

impl RegSubList {
    pub(super) const fn new() -> Self {
        Self::Line([linepos::new(); NSUBEXP])
    }

    #[inline]
    pub(super) fn multi(&self) -> &[multipos; NSUBEXP] {
        match self {
            Self::Multi(m) => m,
            Self::Line(_) => const { &[multipos::new(); NSUBEXP] },
        }
    }

    #[inline]
    pub(super) fn multi_mut(&mut self) -> &mut [multipos; NSUBEXP] {
        if !matches!(*self, Self::Multi(_)) {
            *self = Self::Multi([multipos::new(); NSUBEXP]);
        }
        match self {
            Self::Multi(m) => m,
            Self::Line(_) => unreachable!(),
        }
    }

    #[inline]
    pub(super) fn line(&self) -> &[linepos; NSUBEXP] {
        match self {
            Self::Line(l) => l,
            Self::Multi(_) => const { &[linepos::new(); NSUBEXP] },
        }
    }

    #[inline]
    pub(super) fn line_mut(&mut self) -> &mut [linepos; NSUBEXP] {
        if !matches!(*self, Self::Line(_)) {
            *self = Self::Line([linepos::new(); NSUBEXP]);
        }
        match self {
            Self::Line(l) => l,
            Self::Multi(_) => unreachable!(),
        }
    }
}

impl Default for RegSubList {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct regsub_T {
    pub in_use: c_int,
    pub list: RegSubList,
    pub orig_start_col: colnr_T,
}

impl regsub_T {
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
pub(super) struct regsubs_T {
    pub norm: regsub_T,
    pub synt: regsub_T,
}

impl regsubs_T {
    pub(super) const fn new() -> Self {
        Self {
            norm: regsub_T::new(),
            synt: regsub_T::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct nfa_pim_T {
    pub result: c_int,
    pub state: Option<usize>,
    pub subs: regsubs_T,
    pub end: RegPos,
}

impl nfa_pim_T {
    pub(super) const fn new() -> Self {
        Self {
            result: 0,
            state: None,
            subs: regsubs_T::new(),
            end: RegPos::new(),
        }
    }
}

impl Default for nfa_pim_T {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub(super) struct nfa_thread_T {
    pub state: Option<usize>,
    pub count: c_int,
    pub pim: nfa_pim_T,
    pub subs: regsubs_T,
}

impl nfa_thread_T {
    pub(super) const fn new() -> Self {
        Self {
            state: None,
            count: 0,
            pim: nfa_pim_T::new(),
            subs: regsubs_T::new(),
        }
    }
}

impl Default for nfa_thread_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub(super) struct nfa_list_T {
    pub t: Vec<nfa_thread_T>,
    pub n: c_int,
    pub id: c_int,
    pub has_pim: c_int,
}

#[derive(Clone, Copy)]
enum CaseConv {
    Upper,
    Lower,
}

impl CaseConv {
    #[inline]
    fn apply(self, c: c_int) -> c_int {
        match self {
                Self::Upper => mb_toupper(c),
                Self::Lower => mb_tolower(c),
            }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct parse_state_T {
    pub regparse: *mut c_char,
    pub prevchr_len: c_int,
    pub curchr: c_int,
    pub prevchr: c_int,
    pub prevprevchr: c_int,
    pub nextchr: c_int,
    pub at_start: c_int,
    pub prev_at_start: c_int,
    pub regnpar: c_int,
}

impl parse_state_T {
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

impl Default for parse_state_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct regexec_T {
    pub reg_match: *mut regmatch_T,
    pub reg_mmatch: *mut regmmatch_T,
    pub reg_startp: *mut *mut u8,
    pub reg_endp: *mut *mut u8,
    pub reg_startpos: *mut lpos_T,
    pub reg_endpos: *mut lpos_T,
    pub reg_win: *mut win_T,
    pub reg_buf: *mut buf_T,
    pub reg_firstlnum: linenr_T,
    pub reg_maxline: linenr_T,
    pub reg_line_lbr: bool,
    pub lnum: linenr_T,
    pub line: *mut u8,
    pub input: *mut u8,
    pub need_clear_subexpr: c_int,
    pub need_clear_zsubexpr: c_int,
    pub reg_ic: bool,
    pub reg_icombine: bool,
    pub reg_nobreak: bool,
    pub reg_maxcol: colnr_T,
    pub nfa_has_zend: c_int,
    pub nfa_has_backref: c_int,
    pub nfa_nsubexpr: c_int,
    pub nfa_listid: c_int,
    pub nfa_alt_listid: c_int,
    pub nfa_has_zsubexpr: c_int,
}

impl regexec_T {
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

impl Default for regexec_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct regsubmatch_T {
    pub sm_match: *mut regmatch_T,
    pub sm_mmatch: *mut regmmatch_T,
    pub sm_firstlnum: linenr_T,
    pub sm_maxline: linenr_T,
    pub sm_line_lbr: c_int,
}

impl regsubmatch_T {
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

impl Default for regsubmatch_T {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct decomp_T {
    a: c_int,
    b: c_int,
    c: c_int,
}

static DECOMP_TABLE: [decomp_T; 0xfb4f - 0xfb20 + 1] = [
    decomp_T {
        a: 0x5e2,
        b: 0,
        c: 0,
    }, // 0xfb20       alt ayin
    decomp_T {
        a: 0x5d0,
        b: 0,
        c: 0,
    }, // 0xfb21       alt alef
    decomp_T {
        a: 0x5d3,
        b: 0,
        c: 0,
    }, // 0xfb22       alt dalet
    decomp_T {
        a: 0x5d4,
        b: 0,
        c: 0,
    }, // 0xfb23       alt he
    decomp_T {
        a: 0x5db,
        b: 0,
        c: 0,
    }, // 0xfb24       alt kaf
    decomp_T {
        a: 0x5dc,
        b: 0,
        c: 0,
    }, // 0xfb25       alt lamed
    decomp_T {
        a: 0x5dd,
        b: 0,
        c: 0,
    }, // 0xfb26       alt mem-sofit
    decomp_T {
        a: 0x5e8,
        b: 0,
        c: 0,
    }, // 0xfb27       alt resh
    decomp_T {
        a: 0x5ea,
        b: 0,
        c: 0,
    }, // 0xfb28       alt tav
    decomp_T {
        a: b'+' as c_int,
        b: 0,
        c: 0,
    }, // 0xfb29       alt plus
    decomp_T {
        a: 0x5e9,
        b: 0x5c1,
        c: 0,
    }, // 0xfb2a       shin+shin-dot
    decomp_T {
        a: 0x5e9,
        b: 0x5c2,
        c: 0,
    }, // 0xfb2b       shin+sin-dot
    decomp_T {
        a: 0x5e9,
        b: 0x5c1,
        c: 0x5bc,
    }, // 0xfb2c       shin+shin-dot+dagesh
    decomp_T {
        a: 0x5e9,
        b: 0x5c2,
        c: 0x5bc,
    }, // 0xfb2d       shin+sin-dot+dagesh
    decomp_T {
        a: 0x5d0,
        b: 0x5b7,
        c: 0,
    }, // 0xfb2e       alef+patah
    decomp_T {
        a: 0x5d0,
        b: 0x5b8,
        c: 0,
    }, // 0xfb2f       alef+qamats
    decomp_T {
        a: 0x5d0,
        b: 0x5b4,
        c: 0,
    }, // 0xfb30       alef+hiriq
    decomp_T {
        a: 0x5d1,
        b: 0x5bc,
        c: 0,
    }, // 0xfb31       bet+dagesh
    decomp_T {
        a: 0x5d2,
        b: 0x5bc,
        c: 0,
    }, // 0xfb32       gimel+dagesh
    decomp_T {
        a: 0x5d3,
        b: 0x5bc,
        c: 0,
    }, // 0xfb33       dalet+dagesh
    decomp_T {
        a: 0x5d4,
        b: 0x5bc,
        c: 0,
    }, // 0xfb34       he+dagesh
    decomp_T {
        a: 0x5d5,
        b: 0x5bc,
        c: 0,
    }, // 0xfb35       vav+dagesh
    decomp_T {
        a: 0x5d6,
        b: 0x5bc,
        c: 0,
    }, // 0xfb36       zayin+dagesh
    decomp_T {
        a: 0xfb37,
        b: 0,
        c: 0,
    }, // 0xfb37 -- UNUSED
    decomp_T {
        a: 0x5d8,
        b: 0x5bc,
        c: 0,
    }, // 0xfb38       tet+dagesh
    decomp_T {
        a: 0x5d9,
        b: 0x5bc,
        c: 0,
    }, // 0xfb39       yud+dagesh
    decomp_T {
        a: 0x5da,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3a       kaf sofit+dagesh
    decomp_T {
        a: 0x5db,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3b       kaf+dagesh
    decomp_T {
        a: 0x5dc,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3c       lamed+dagesh
    decomp_T {
        a: 0xfb3d,
        b: 0,
        c: 0,
    }, // 0xfb3d -- UNUSED
    decomp_T {
        a: 0x5de,
        b: 0x5bc,
        c: 0,
    }, // 0xfb3e       mem+dagesh
    decomp_T {
        a: 0xfb3f,
        b: 0,
        c: 0,
    }, // 0xfb3f -- UNUSED
    decomp_T {
        a: 0x5e0,
        b: 0x5bc,
        c: 0,
    }, // 0xfb40       nun+dagesh
    decomp_T {
        a: 0x5e1,
        b: 0x5bc,
        c: 0,
    }, // 0xfb41       samech+dagesh
    decomp_T {
        a: 0xfb42,
        b: 0,
        c: 0,
    }, // 0xfb42 -- UNUSED
    decomp_T {
        a: 0x5e3,
        b: 0x5bc,
        c: 0,
    }, // 0xfb43       pe sofit+dagesh
    decomp_T {
        a: 0x5e4,
        b: 0x5bc,
        c: 0,
    }, // 0xfb44       pe+dagesh
    decomp_T {
        a: 0xfb45,
        b: 0,
        c: 0,
    }, // 0xfb45 -- UNUSED
    decomp_T {
        a: 0x5e6,
        b: 0x5bc,
        c: 0,
    }, // 0xfb46       tsadi+dagesh
    decomp_T {
        a: 0x5e7,
        b: 0x5bc,
        c: 0,
    }, // 0xfb47       qof+dagesh
    decomp_T {
        a: 0x5e8,
        b: 0x5bc,
        c: 0,
    }, // 0xfb48       resh+dagesh
    decomp_T {
        a: 0x5e9,
        b: 0x5bc,
        c: 0,
    }, // 0xfb49       shin+dagesh
    decomp_T {
        a: 0x5ea,
        b: 0x5bc,
        c: 0,
    }, // 0xfb4a       tav+dagesh
    decomp_T {
        a: 0x5d5,
        b: 0x5b9,
        c: 0,
    }, // 0xfb4b       vav+holam
    decomp_T {
        a: 0x5d1,
        b: 0x5bf,
        c: 0,
    }, // 0xfb4c       bet+rafe
    decomp_T {
        a: 0x5db,
        b: 0x5bf,
        c: 0,
    }, // 0xfb4d       kaf+rafe
    decomp_T {
        a: 0x5e4,
        b: 0x5bf,
        c: 0,
    }, // 0xfb4e       pe+rafe
    decomp_T {
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
    pub(super) regnpar: c_int,
    pub(super) wants_nfa: bool,
    pub(super) regnzpar: c_int,
    pub(super) re_has_z: c_int,
    pub(super) regflags: c_uint,
    pub(super) had_eol: c_int,
    pub(super) reg_magic: magic_T,
    pub(super) reg_string: c_int,
    pub(super) reg_strict: c_int,
    pub(super) curchr: c_int,
    pub(super) prevchr: c_int,
    pub(super) prevprevchr: c_int,
    pub(super) nextchr: c_int,
    pub(super) prevchr_len: c_int,
    pub(super) at_start: c_int,
    pub(super) prev_at_start: c_int,
    pub(super) peekchr_after_slash: c_int,
    pub(super) reg_cpo_lit: c_int,
    pub(super) reg_tofree: Vec<u8>,
    pub(super) rex: regexec_T,
    pub(super) rex_in_use: bool,
    pub(super) can_f_submatch: bool,
    pub(super) rsm: regsubmatch_T,
    pub(super) reg_startzp: [*mut u8; NSUBEXP],
    pub(super) reg_endzp: [*mut u8; NSUBEXP],
    pub(super) reg_startzpos: [lpos_T; NSUBEXP],
    pub(super) reg_endzpos: [lpos_T; NSUBEXP],
    pub(super) eval_result: [Option<std::ffi::CString>; MAX_REGSUB_NESTING],
    pub(super) regsub_nesting: usize,
    pub(super) had_endbrace: [u8; NSUBEXP],
    pub(super) num_complex_braces: c_int,
    pub(super) regcode: *mut u8,
    pub(super) regsize: i64,
    pub(super) reg_toolong: c_int,
    pub(super) bt_reg_parse_depth: c_int,
    pub(super) brace_min: [i64; 10],
    pub(super) brace_max: [i64; 10],
    pub(super) brace_count: [c_int; 10],
    pub(super) one_exactly: bool,
    pub(super) regstack: Vec<u64>,
    backpos: Vec<backpos_T>,
    pub(super) behind_pos: regsave_T,
    pub(super) nfa_re_flags: c_int,
    pub(super) post_vec: Vec<c_int>,
    pub(super) nfa_reg_parse_depth: c_int,
    pub(super) nstate: c_int,
    pub(super) istate: c_int,
    pub(super) nfa_endp: *mut save_se_T,
    pub(super) nfa_ll_index: usize,
    pub(super) regexp_engine: c_int,
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
            rex: regexec_T {
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
            rsm: regsubmatch_T {
                sm_match: ptr::null_mut(),
                sm_mmatch: ptr::null_mut(),
                sm_firstlnum: 0,
                sm_maxline: 0,
                sm_line_lbr: 0,
            },
            reg_startzp: [ptr::null_mut(); NSUBEXP],
            reg_endzp: [ptr::null_mut(); NSUBEXP],
            reg_startzpos: [lpos_T { lnum: 0, col: 0 }; NSUBEXP],
            reg_endzpos: [lpos_T { lnum: 0, col: 0 }; NSUBEXP],
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
            behind_pos: regsave_T::new(),
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
pub(super) unsafe fn tv_list_ref(l: *mut list_T) {
    unsafe {
        if !l.is_null() {
            (*l).lv_refcount += 1;
        }
    }
}

#[inline]
pub(super) const fn Magic(x: c_int) -> c_int {
    x - 256
}

#[inline]
pub(super) const fn un_Magic(x: c_int) -> c_int {
    x + 256
}

#[inline]
pub(super) const fn is_Magic(x: c_int) -> bool {
    x < 0
}

#[inline]
pub(super) const fn no_Magic(x: c_int) -> c_int {
    if is_Magic(x) { un_Magic(x) } else { x }
}

#[inline]
pub(super) const fn toggle_Magic(x: c_int) -> c_int {
    if is_Magic(x) { un_Magic(x) } else { Magic(x) }
}

#[inline]
pub(super) unsafe fn UCHARAT(p: *const u8) -> c_int {
    unsafe { *p as c_int }
}

#[inline]
pub(super) fn REG_MULTI() -> bool {
    re_st().rex.reg_match.is_null()
}

#[inline]
pub(super) unsafe fn mb_ptr_adv<T>(p: *mut *mut T) {
    unsafe {
        let step = utfc_ptr2len((*p).cast()) as usize;
        *p = (*p).cast::<u8>().add(step).cast();
    }
}

/// Return NOT_MULTI if c is not a "multi" operator.
/// Return MULTI_ONE if c is a single "multi" operator.
/// Return MULTI_MULT if c is a multi "multi" operator.
#[inline]
pub(super) fn re_multi_type(c: c_int) -> c_int {
    if c == Magic(b'@' as c_int) || c == Magic(b'=' as c_int) || c == Magic(b'?' as c_int) {
        return MULTI_ONE;
    }
    if c == Magic(b'*' as c_int) || c == Magic(b'+' as c_int) || c == Magic(b'{' as c_int) {
        return MULTI_MULT;
    }
    NOT_MULTI
}

/// Translate `\x` to its control character, except `\n`, which is Magic.
#[inline]
pub(super) fn backslash_trans(c: c_int) -> c_int {
    match c as u8 {
        b'r' => CAR,
        b't' => TAB,
        b'e' => ESC,
        b'b' => BS,
        _ => c,
    }
}

/// Check for a character class name `[:name:]`. `pp` points to the `[`.
pub(super) unsafe fn get_char_class(pp: *mut *mut c_char) -> c_int {
    const CHAR_CLASS_TAB: &[(c_int, &[u8])] = &[
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
        if *p.add(1) == b':'
            && (*p.add(2)).is_ascii_lowercase()
            && (*p.add(3)).is_ascii_lowercase()
            && (*p.add(4)).is_ascii_lowercase()
        {
            let target = p.add(2);
            for &(key, pat) in CHAR_CLASS_TAB {
                if crate::memory::xstrnlen(target.cast::<c_char>(), pat.len()) == pat.len()
                    && std::slice::from_raw_parts(target, pat.len()) == pat
                {
                    *pp = (*pp).add(pat.len() + 2);
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
            *tab.add(i) = val;
        }
        *tab.add(b' ' as usize) |= RI_WHITE;
        *tab.add(b'\t' as usize) |= RI_WHITE;
        re_st().init_class_tab_done = true;
    }
}

#[inline]
pub(super) fn ri_digit(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_DIGIT) != 0
}
#[inline]
pub(super) fn ri_hex(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_HEX) != 0
}
#[inline]
pub(super) fn ri_octal(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_OCTAL) != 0
}
#[inline]
pub(super) fn ri_word(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_WORD) != 0
}
#[inline]
pub(super) fn ri_head(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_HEAD) != 0
}
#[inline]
pub(super) fn ri_alpha(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_ALPHA) != 0
}
#[inline]
pub(super) fn ri_lower(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_LOWER) != 0
}
#[inline]
pub(super) fn ri_upper(c: c_int) -> bool {
    (0..0x100).contains(&c)
            && (re_st().class_tab[c as usize] & RI_UPPER) != 0
}

/// Return true if compiled regular expression `prog` can match a line break.
pub fn re_multiline(prog: &regprog_T) -> c_int {
    (prog.regflags & RF_HASNL) as c_int
}

/// Check for an equivalence class name `[=a=]`. `pp` points to the `[`.
pub(super) unsafe fn get_equi_class(pp: *mut *mut c_char) -> c_int {
    unsafe {
        let p = *pp;
        if *p.add(1) == b'=' as c_char && *p.add(2) != NUL as c_char {
            let l = utfc_ptr2len(p.add(2)) as usize;
            if *p.add(l + 2) == b'=' as c_char && *p.add(l + 3) == b']' as c_char {
                let c = utf_ptr2char(p.add(2));
                *pp = (*pp).add(l + 4);
                return c;
            }
        }
        0
    }
}

/// Check for a collating element `[.a.]`. `pp` points to the `[`.
pub(super) unsafe fn get_coll_element(pp: *mut *mut c_char) -> c_int {
    unsafe {
        let p = *pp;
        if *p != NUL as c_char && *p.add(1) == b'.' as c_char && *p.add(2) != NUL as c_char {
            let l = utfc_ptr2len(p.add(2)) as usize;
            if *p.add(l + 2) == b'.' as c_char && *p.add(l + 3) == b']' as c_char {
                let c = utf_ptr2char(p.add(2));
                *pp = (*pp).add(l + 4);
                return c;
            }
        }
        0
    }
}

pub(super) fn get_cpo_flags() {
    unsafe {
        re_st().reg_cpo_lit = (!vim_strchr(opt_vars().p_cpo, kCpoLiteral).is_null()) as c_int;
    }
}

/// Skip over a `[]` range.
pub(super) unsafe fn skip_anyof(mut p: *mut c_char) -> *mut c_char {
    unsafe {
        if *p == b'^' as c_char {
            p = p.add(1);
        }
        if *p == b']' as c_char || *p == b'-' as c_char {
            p = p.add(1);
        }
        while *p != NUL as c_char && *p != b']' as c_char {
            let l = utfc_ptr2len(p);
            if l > 1 {
                p = p.add(l as usize);
            } else if *p == b'-' as c_char {
                p = p.add(1);
                if *p != b']' as c_char && *p != NUL as c_char {
                    mb_ptr_adv(&raw mut p);
                }
            } else if *p == b'\\' as c_char
                && (!vim_strchr(REGEXP_INRANGE.as_ptr(), *p.add(1) as u8 as c_int).is_null()
                    || (re_st().reg_cpo_lit == 0
                        && !vim_strchr(REGEXP_ABBR.as_ptr(), *p.add(1) as u8 as c_int).is_null()))
            {
                p = p.add(2);
            } else if *p == b'[' as c_char {
                if get_char_class(&raw mut p) == CLASS_NONE
                    && get_equi_class(&raw mut p) == 0
                    && get_coll_element(&raw mut p) == 0
                    && *p != NUL as c_char
                {
                    p = p.add(1);
                }
            } else {
                p = p.add(1);
            }
        }
        p
    }
}

/// Skip past regular expression.
pub unsafe fn skip_regexp(startp: *mut c_char, delim: c_int, magic: c_int) -> *mut c_char {
    unsafe {
        skip_regexp_ex(
            startp,
            delim,
            magic,
            false,
            None,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    }
}

/// Call `skip_regexp()` and when the delimiter does not match give an error and return NULL.
pub unsafe fn skip_regexp_err(startp: *mut c_char, delim: c_int, magic: c_int) -> *mut c_char {
    unsafe {
        let p = skip_regexp(startp, delim, magic);
        if *p as u8 as c_int != delim {
            semsg!(
                tr(e_missing_delimiter_after_search_pattern_str.as_ptr()),
                startp,
            );
            return ptr::null_mut();
        }
        p
    }
}

/// `skip_regexp()` with extra arguments.
pub unsafe fn skip_regexp_ex(
    mut startp: *mut c_char,
    dirc: c_int,
    magic: c_int,
    in_place: bool,
    mut copy_buf: Option<&mut Vec<c_char>>,
    dropped: *mut c_int,
    magic_val: *mut magic_T,
) -> *mut c_char {
    unsafe {
        let mut mymagic = if magic != 0 { MAGIC_ON } else { MAGIC_OFF };
        let mut p = startp;
        let mut startplen: usize = 0;

        get_cpo_flags();

        while *p != NUL as c_char {
            if *p as u8 as c_int == dirc {
                break;
            }
            if (*p == b'[' as c_char && mymagic >= MAGIC_ON)
                || (*p == b'\\' as c_char && *p.add(1) == b'[' as c_char && mymagic <= MAGIC_OFF)
            {
                p = skip_anyof(p.add(1));
                if *p == NUL as c_char {
                    break;
                }
            } else if *p == b'\\' as c_char && *p.add(1) != NUL as c_char {
                if dirc == b'?' as c_int && in_place && *p.add(1) == b'?' as c_char {
                    if startplen == 0 {
                        startplen = std::ffi::CStr::from_ptr(startp).to_bytes().len();
                    }
                    if let Some(buf) = copy_buf.as_deref_mut()
                        && buf.is_empty()
                    {
                        let off = p.offset_from(startp) as usize;
                        buf.extend_from_slice(std::slice::from_raw_parts(startp, startplen + 1));
                        startp = buf.as_mut_ptr();
                        p = startp.add(off);
                    }
                    if !dropped.is_null() {
                        *dropped += 1;
                    }
                    ptr::copy(
                        p.add(1),
                        p,
                        startplen - (p.add(1).offset_from(startp) as usize) + 1,
                    );
                } else {
                    p = p.add(1);
                }
                if *p == b'v' as c_char {
                    mymagic = MAGIC_ALL;
                } else if *p == b'V' as c_char {
                    mymagic = MAGIC_NONE;
                }
            }
            mb_ptr_adv(&raw mut p);
        }
        if !magic_val.is_null() {
            *magic_val = mymagic;
        }
        p
    }
}

pub(super) unsafe fn initchr(str_: *mut c_char) {
    re_st().regparse = str_;
        re_st().prevchr_len = 0;
        re_st().nextchr = -1;
        re_st().prevchr = -1;
        re_st().prevprevchr = -1;
        re_st().curchr = -1;
        re_st().at_start = 1;
        re_st().prev_at_start = 0;
}

pub(super) fn save_parse_state(ps: &mut parse_state_T) {
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

pub(super) fn restore_parse_state(ps: &mut parse_state_T) {
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

pub(super) fn peekchr() -> c_int {
    unsafe {
        if re_st().curchr != -1 {
            return re_st().curchr;
        }

        re_st().curchr = *re_st().regparse as u8 as c_int;
        match re_st().curchr as u8 {
            b'.' | b'[' | b'~' => {
                if re_st().reg_magic >= MAGIC_ON {
                    re_st().curchr = Magic(re_st().curchr);
                }
            }
            b'(' | b')' | b'{' | b'%' | b'+' | b'=' | b'?' | b'@' | b'!' | b'&' | b'|' | b'<'
            | b'>' | b'#' | b'"' | b'\'' | b',' | b'-' | b':' | b';' | b'`' | b'/' => {
                if re_st().reg_magic == MAGIC_ALL {
                    re_st().curchr = Magic(re_st().curchr);
                }
            }
            b'*' => {
                if re_st().reg_magic >= MAGIC_ON
                    && re_st().at_start == 0
                    && !(re_st().prev_at_start != 0 && re_st().prevchr == Magic(b'^' as c_int))
                    && (re_st().peekchr_after_slash != 0
                        || (re_st().prevchr != Magic(b'(' as c_int)
                            && re_st().prevchr != Magic(b'&' as c_int)
                            && re_st().prevchr != Magic(b'|' as c_int)))
                {
                    re_st().curchr = Magic(b'*' as c_int);
                }
            }
            b'^' => {
                if re_st().reg_magic >= MAGIC_OFF
                    && (re_st().at_start != 0
                        || re_st().reg_magic == MAGIC_ALL
                        || re_st().prevchr == Magic(b'(' as c_int)
                        || re_st().prevchr == Magic(b'|' as c_int)
                        || re_st().prevchr == Magic(b'&' as c_int)
                        || re_st().prevchr == Magic(b'n' as c_int)
                        || (no_Magic(re_st().prevchr) == b'(' as c_int
                            && re_st().prevprevchr == Magic(b'%' as c_int)))
                {
                    re_st().curchr = Magic(b'^' as c_int);
                    re_st().at_start = 1;
                    re_st().prev_at_start = 0;
                }
            }
            b'$' => {
                if re_st().reg_magic >= MAGIC_OFF {
                    let mut p = re_st().regparse.add(1).cast::<u8>();
                    let mut is_magic_all = re_st().reg_magic == MAGIC_ALL;

                    while *p == b'\\'
                        && matches!(*p.add(1), b'c' | b'C' | b'm' | b'M' | b'v' | b'V' | b'Z')
                    {
                        if *p.add(1) == b'v' {
                            is_magic_all = true;
                        } else if matches!(*p.add(1), b'm' | b'M' | b'V') {
                            is_magic_all = false;
                        }
                        p = p.add(2);
                    }
                    if *p == NUL
                        || (*p == b'\\' && matches!(*p.add(1), b'|' | b'&' | b')' | b'n'))
                        || (is_magic_all && matches!(*p, b'|' | b'&' | b')'))
                        || re_st().reg_magic == MAGIC_ALL
                    {
                        re_st().curchr = Magic(b'$' as c_int);
                    }
                }
            }
            b'\\' => {
                let c = *re_st().regparse.add(1) as u8 as c_int;
                if c == NUL as c_int {
                    re_st().curchr = b'\\' as c_int;
                } else if c <= b'~' as c_int && META_flags[c as usize] != 0 {
                    re_st().curchr = -1;
                    re_st().prev_at_start = re_st().at_start;
                    re_st().at_start = 0;
                    re_st().regparse = re_st().regparse.add(1);
                    re_st().peekchr_after_slash += 1;
                    let _ = peekchr();
                    re_st().regparse = re_st().regparse.sub(1);
                    re_st().peekchr_after_slash -= 1;
                    re_st().curchr = toggle_Magic(re_st().curchr);
                } else if !vim_strchr(REGEXP_ABBR.as_ptr(), c).is_null() {
                    re_st().curchr = backslash_trans(c);
                } else if re_st().reg_magic == MAGIC_NONE && (c == b'$' as c_int || c == b'^' as c_int) {
                    re_st().curchr = toggle_Magic(c);
                } else {
                    re_st().curchr = if c >= 0x80 {
                        utf_ptr2char(re_st().regparse.add(1))
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
        if *re_st().regparse == b'\\' as c_char {
            re_st().prevchr_len = 1;
        } else {
            re_st().prevchr_len = 0;
        }
        if *re_st().regparse.add(re_st().prevchr_len as usize) != NUL as c_char {
            if (*re_st().regparse.add(re_st().prevchr_len as usize) as u8) < 0x80 {
                re_st().prevchr_len += 1;
            } else {
                re_st().prevchr_len += utf_ptr2len(re_st().regparse.add(re_st().prevchr_len as usize));
            }
        }
        re_st().regparse = re_st().regparse.add(re_st().prevchr_len as usize);
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

pub(super) fn getchr() -> c_int {
    let chr = peekchr();
        skipchr();
        chr
}

pub(super) fn ungetchr() {
    unsafe {
        re_st().nextchr = re_st().curchr;
        re_st().curchr = re_st().prevchr;
        re_st().prevchr = re_st().prevprevchr;
        re_st().at_start = re_st().prev_at_start;
        re_st().prev_at_start = 0;
        re_st().regparse = re_st().regparse.sub(re_st().prevchr_len as usize);
    }
}

pub(super) fn gethexchrs(maxinputlen: c_int) -> i64 {
    unsafe {
        let mut nr: i64 = 0;
        let mut i: c_int = 0;
        while i < maxinputlen {
            let c = *re_st().regparse as u8 as c_int;
            if !ascii_isxdigit(c) {
                break;
            }
            nr <<= 4;
            nr |= i64::from(hex2nr(c));
            re_st().regparse = re_st().regparse.add(1);
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) fn getdecchrs() -> i64 {
    unsafe {
        let mut nr: i64 = 0;
        let mut i: c_int = 0;
        loop {
            let c = *re_st().regparse as u8;
            if !c.is_ascii_digit() {
                break;
            }
            nr = nr.wrapping_mul(10).wrapping_add(i64::from(c - b'0'));
            re_st().regparse = re_st().regparse.add(1);
            re_st().curchr = -1;
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) fn getoctchrs() -> i64 {
    unsafe {
        let mut nr: i64 = 0;
        let mut i: c_int = 0;
        while i < 3 && nr < 0o40 {
            let c = *re_st().regparse as u8;
            if !(b'0'..=b'7').contains(&c) {
                break;
            }
            nr <<= 3;
            nr |= i64::from(hex2nr(c as c_int));
            re_st().regparse = re_st().regparse.add(1);
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) fn read_limits(minval: &mut c_int, maxval: &mut c_int) -> c_int {
    unsafe {
        let mut reverse = false;
        if *re_st().regparse == b'-' as c_char {
            re_st().regparse = re_st().regparse.add(1);
            reverse = true;
        }
        let first_char = re_st().regparse;
        *minval = getdigits_int(&raw mut re_st().regparse, false, 0);
        if *re_st().regparse == b',' as c_char {
            re_st().regparse = re_st().regparse.add(1);
            if ascii_isdigit(*re_st().regparse as u8 as c_int) {
                *maxval = getdigits_int(&raw mut re_st().regparse, false, MAX_LIMIT);
            } else {
                *maxval = MAX_LIMIT;
            }
        } else if ascii_isdigit(*first_char as u8 as c_int) {
            *maxval = *minval;
        } else {
            *maxval = MAX_LIMIT;
        }
        if *re_st().regparse == b'\\' as c_char {
            re_st().regparse = re_st().regparse.add(1);
        }
        if *re_st().regparse != b'}' as c_char {
            semsg!(
                tr(c"E554: Syntax error in %s{...}".as_ptr()),
                if re_st().reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
            g_st().rc_did_emsg = true;
            return FAIL;
        }
        if (!reverse && *minval > *maxval) || (reverse && *minval < *maxval) {
            ptr::swap(minval, maxval);
        }
        skipchr();
        OK
    }
}

pub(super) fn reg_breakcheck() {
    if !re_st().rex.reg_nobreak {
            fast_breakcheck();
        }
}

pub(super) fn reg_iswordc(c: c_int) -> bool {
    unsafe { vim_iswordc_buf(c, re_st().rex.reg_buf) }
}

unsafe fn reg_getline_common(
    lnum: linenr_T,
    flags: reg_getline_flags_T,
    line: *mut *mut c_char,
    length: *mut colnr_T,
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
            if get_length {
                *length = 0;
            }
            return;
        }

        if lnum > maxline {
            if get_line {
                *line = c"".as_ptr().cast_mut();
            }
            if get_length {
                *length = 0;
            }
            return;
        }

        if get_line {
            *line = ml_get_buf(re_st().rex.reg_buf, firstlnum);
        }
        if get_length {
            *length = ml_get_buf_len(re_st().rex.reg_buf, firstlnum);
        }
    }
}

pub(super) fn reg_getline(lnum: linenr_T) -> *mut c_char {
    unsafe {
        let mut line: *mut c_char = ptr::null_mut();
        reg_getline_common(lnum, RGLF_LINE, &raw mut line, ptr::null_mut());
        line
    }
}

pub(super) fn reg_getline_len(lnum: linenr_T) -> colnr_T {
    unsafe {
        let mut length: colnr_T = 0;
        reg_getline_common(lnum, RGLF_LENGTH, ptr::null_mut(), &raw mut length);
        length
    }
}

pub(super) fn reg_prev_class() -> c_int {
    unsafe {
        if re_st().rex.input > re_st().rex.line {
            let prev = re_st().rex.input.sub(1).cast::<c_char>();
            let head = prev.sub(utf_head_off(re_st().rex.line.cast(), prev) as usize);
            return mb_get_class_tab(head, (&raw const (*re_st().rex.reg_buf).b_chartab).cast());
        }
        -1
    }
}

pub(super) fn reg_match_visual() -> bool {
    unsafe {
        let wp = if re_st().rex.reg_win.is_null() {
            *curwin.as_ptr()
        } else {
            re_st().rex.reg_win
        };
        if re_st().rex.reg_buf != g_st().curbuf || g_st().Visual.start.lnum == 0 || !REG_MULTI() {
            return false;
        }

        let (top, bot, mode, curswant) = if g_st().Visual.active {
            let (t, b) = if lt(g_st().Visual.start, (*wp).w_cursor) {
                (g_st().Visual.start, (*wp).w_cursor)
            } else {
                ((*wp).w_cursor, g_st().Visual.start)
            };
            (t, b, g_st().Visual.mode, (*wp).w_curswant)
        } else {
            let (t, mut b) = if lt((*g_st().curbuf).b_visual.vi_start, (*g_st().curbuf).b_visual.vi_end) {
                ((*g_st().curbuf).b_visual.vi_start, (*g_st().curbuf).b_visual.vi_end)
            } else {
                ((*g_st().curbuf).b_visual.vi_end, (*g_st().curbuf).b_visual.vi_start)
            };
            if b.lnum > (*g_st().curbuf).b_ml.ml_line_count {
                b.lnum = (*g_st().curbuf).b_ml.ml_line_count;
            }
            (
                t,
                b,
                (*g_st().curbuf).b_visual.vi_mode,
                (*g_st().curbuf).b_visual.vi_curswant,
            )
        };

        let lnum = re_st().rex.lnum + re_st().rex.reg_firstlnum;
        if lnum < top.lnum || lnum > bot.lnum {
            return false;
        }

        let col = re_st().rex.input.offset_from(re_st().rex.line) as colnr_T;
        if mode == b'v' as c_int {
            if (lnum == top.lnum && col < top.col)
                || (lnum == bot.lnum && col >= bot.col + colnr_T::from(*opt_vars().p_sel != b'e' as c_char))
            {
                return false;
            }
        } else if mode == Ctrl_V {
            let mut start: colnr_T = 0;
            let mut end: colnr_T = 0;
            let mut start2: colnr_T = 0;
            let mut end2: colnr_T = 0;
            let mut top_copy = top;
            let mut bot_copy = bot;
            getvvcol(
                wp,
                &mut top_copy,
                &raw mut start,
                ptr::null_mut(),
                &raw mut end,
                0,
            );
            getvvcol(
                wp,
                &mut bot_copy,
                &raw mut start2,
                ptr::null_mut(),
                &raw mut end2,
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
            re_st().rex.input = re_st().rex.line.add(col as usize);

            let cols = win_linetabsize(wp, re_st().rex.reg_firstlnum + re_st().rex.lnum, re_st().rex.line.cast(), col);
            if cols < start || cols > end - colnr_T::from(*opt_vars().p_sel == b'e' as c_char) {
                return false;
            }
        }
        true
    }
}

pub(super) fn prog_magic_wrong() -> bool {
    unsafe {
        let prog = if REG_MULTI() {
            (*re_st().rex.reg_mmatch).regprog
        } else {
            (*re_st().rex.reg_match).regprog
        };
        let RegProgEngine::Bt(ref bt_prog) = (*prog).engine else {
            return false;
        };
        if UCHARAT(bt_prog.program.as_ptr()) != REGMAGIC {
            emsg(tr((&raw const e_re_corr).cast()));
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
        if REG_MULTI() {
            let uninit_pos = lpos_T { lnum: -1, col: -1 };
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
        if REG_MULTI() {
            let uninit_pos = lpos_T { lnum: -1, col: -1 };
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

pub(super) unsafe fn match_with_backref(
    start_lnum: linenr_T,
    start_col: colnr_T,
    end_lnum: linenr_T,
    end_col: colnr_T,
    bytelen: *mut c_int,
) -> c_int {
    unsafe {
        let mut clnum = start_lnum;
        let mut ccol = start_col;

        if !bytelen.is_null() {
            *bytelen = 0;
        }
        loop {
            let tf = &raw mut re_st().reg_tofree;
            if (*tf).is_empty() || re_st().rex.line != (*tf).as_mut_ptr() {
                let len = std::ffi::CStr::from_ptr(re_st().rex.line.cast()).to_bytes().len() + 51;
                let off = re_st().rex.input.offset_from(re_st().rex.line) as usize;
                if (*tf).len() < len {
                    (*tf).resize(len, 0);
                }
                { let s = re_st().rex.line.cast(); std::ptr::copy_nonoverlapping(s, (*tf).as_mut_ptr().cast(), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                re_st().rex.line = (*tf).as_mut_ptr();
                re_st().rex.input = re_st().rex.line.add(off);
            }

            let p = reg_getline(clnum);
            debug_assert!(!p.is_null());

            let mut len = if clnum == end_lnum {
                end_col - ccol
            } else {
                reg_getline_len(clnum) - ccol
            };

            if (!re_st().rex.reg_ic && cstrncmp(p.add(ccol as usize), re_st().rex.input.cast(), &mut len) != 0)
                || (re_st().rex.reg_ic
                    && mb_strnicmp(p.add(ccol as usize), re_st().rex.input.cast(), len as usize) != 0)
            {
                return RA_NOMATCH;
            }
            if !bytelen.is_null() {
                *bytelen += len;
            }
            if clnum == end_lnum {
                break;
            }
            if re_st().rex.lnum >= re_st().rex.reg_maxline {
                return RA_NOMATCH;
            }

            reg_nextline();
            if !bytelen.is_null() {
                *bytelen = 0;
            }
            clnum += 1;
            ccol = 0;
            if g_st().got_int {
                return RA_FAIL;
            }
        }
        RA_MATCH
    }
}

pub(super) unsafe fn re_mult_next(what: *const c_char) -> bool {
    unsafe {
        if re_multi_type(peekchr()) == MULTI_MULT {
            semsg!(tr(c"E888: (NFA regexp) cannot repeat %s".as_ptr()), what);
            g_st().rc_did_emsg = true;
            return false;
        }
        true
    }
}

fn mb_decompose(c: c_int, c1: &mut c_int, c2: &mut c_int, c3: &mut c_int) {
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

pub(super) unsafe fn cstrncmp(s1: *const c_char, s2: *const c_char, n: &mut c_int) -> c_int {
    unsafe {
        let mut result = if !re_st().rex.reg_ic {
            { let _p1 = s1; let _p2 = s2; let _n = *n as usize; std::slice::from_raw_parts(_p1.cast::<u8>(), crate::memory::xstrnlen(_p1, _n)).cmp(std::slice::from_raw_parts(_p2.cast::<u8>(), crate::memory::xstrnlen(_p2, _n))) as c_int }
        } else {
            let mut p = s1.cast_mut();
            let mut n2: c_int = 0;
            let mut n1 = *n;
            while n1 > 0 && *p != NUL as c_char {
                n1 -= utfc_ptr2len(p);
                mb_ptr_adv(&raw mut p);
                n2 += 1;
            }
            p = s2.cast_mut();
            while n2 > 0 && *p != NUL as c_char {
                n2 -= 1;
                mb_ptr_adv(&raw mut p);
            }
            n2 = p.offset_from(s2) as c_int;
            let r = utf_strnicmp(s1, s2, *n as usize, n2 as usize);
            if r == 0 {
                *n = n2;
            }
            r
        };

        if result != 0 && re_st().rex.reg_icombine {
            let mut str1: *const c_char = s1;
            let mut str2: *const c_char = s2;
            let mut c1: c_int = 0;
            let mut c2: c_int = 0;
            let mut c11: c_int = 0;
            let mut c12: c_int = 0;
            let mut junk: c_int = 0;
            let mut junk2: c_int = 0;

            while (str1.offset_from(s1) as c_int) < *n {
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
                *n = str2.offset_from(s2) as c_int;
            }
        }

        result
    }
}

#[inline]
pub(super) unsafe fn cstrchr(s: *const c_char, c: c_int) -> *mut c_char {
    unsafe {
        if !re_st().rex.reg_ic {
            return vim_strchr(s, c);
        }

        let (cc, lc) = if c > 0x80 {
            let f = utf_fold(c);
            (f, f)
        } else if (c as u8).is_ascii_uppercase() {
            let l = (c as u8).to_ascii_lowercase() as c_int;
            (l, l)
        } else if (c as u8).is_ascii_lowercase() {
            let u = (c as u8).to_ascii_uppercase() as c_int;
            (u, c)
        } else {
            return vim_strchr(s, c);
        };

        let mut p = s;
        while *p != NUL as c_char {
            let uc = utf_ptr2char(p);
            if c > 0x80 || uc > 0x80 {
                if (uc < 0x80 || uc != *p as u8 as c_int) && utf_fold(uc) == lc {
                    return p.cast_mut();
                }
            } else if *p as u8 as c_int == c || *p as u8 as c_int == cc {
                return p.cast_mut();
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        ptr::null_mut()
    }
}

/// Replace tildes in the pattern by the old pattern.
/// Returns `Some(CString)` if a new string was allocated, or `None` if `source` was modified in place.
pub unsafe fn regtilde(
    source: *mut c_char,
    magic: c_int,
    preview: bool,
) -> Option<std::ffi::CString> {
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
        while *p != NUL as c_char {
            if crate::memory::cstr_prefix_bytes(p, tildelen) == crate::memory::cstr_prefix_bytes(tilde.as_ptr(), tildelen) {
                let prefixlen = p.offset_from(newsub) as usize;
                let postfix = p.add(tildelen);
                if newsublen == 0 {
                    newsublen = std::ffi::CStr::from_ptr(newsub).to_bytes().len();
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
                        emsg(tr((&raw const e_resulting_text_too_long).cast()));
                        error = true;
                        break;
                    }

                    let mut tmpsub_vec: Vec<c_char> = vec![0; tmpsublen + 1];
                    let tmpsub = tmpsub_vec.as_mut_ptr();
                    ptr::copy_nonoverlapping(newsub, tmpsub, prefixlen);
                    ptr::copy_nonoverlapping(
                        prev_sub.as_ptr(),
                        tmpsub.add(prefixlen),
                        reg_prev_sublen,
                    );
                    { let s = postfix; std::ptr::copy_nonoverlapping(s, tmpsub.add(prefixlen + reg_prev_sublen), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };

                    newsub_vec = Some(tmpsub_vec);
                    newsub = newsub_vec.as_mut().unwrap().as_mut_ptr();
                    newsublen = tmpsublen;
                    p = newsub.add(prefixlen + reg_prev_sublen);
                } else {
                    ptr::copy(postfix, p, postfixlen + 1);
                }
                continue;
            } else {
                if *p == b'\\' as c_char && *p.add(1) != NUL as c_char {
                    p = p.add(1);
                }
                p = p.add(utfc_ptr2len(p) as usize);
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

        newsub_vec.map(|v| CStr::from_ptr(v.as_ptr()).to_owned())
    }
}

pub(crate) unsafe fn fill_submatch_list(
    _argc: c_int,
    argv: *mut typval_T,
    argskip: c_int,
    fp: &mut ufunc_T,
) -> c_int {
    unsafe {
        let listarg = argv.add(argskip as usize);
        if fp.uf_varargs == 0 && (fp.uf_args.len() as c_int) <= argskip {
            return argskip;
        }

        tv_list_init_static10(&mut *(*listarg).vval.v_list().cast::<staticList10_T>());

        let mut li = tv_list_first((*listarg).vval.v_list());
        for i in 0..10 {
            let s = (*re_st().rsm.sm_match).startp[i];
            if s.is_null() || (*re_st().rsm.sm_match).endp[i].is_null() {
                tv_set_string(&raw mut (*li).li_tv, ptr::null());
            } else {
                let len = (*re_st().rsm.sm_match).endp[i].offset_from(s) as usize;
                let cs = cstring_from_bytes(s, len).unwrap_or_default();
                tv_set_string(&raw mut (*li).li_tv, cs.as_ptr());
            }
            li = (*li).li_next;
        }
        argskip + 1
    }
}

pub unsafe fn vim_regsub(
    rmp: &mut regmatch_T,
    source: *mut c_char,
    expr: *mut typval_T,
    dest: *mut c_char,
    destlen: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut rex_save = regexec_T::new();
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
}

pub unsafe fn vim_regsub_multi(
    rmp: &mut regmmatch_T,
    lnum: linenr_T,
    source: *mut c_char,
    dest: *mut c_char,
    destlen: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut rex_save = regexec_T::new();
        let rex_in_use_save = re_st().rex_in_use;

        if re_st().rex_in_use {
            rex_save = re_st().rex;
        }
        re_st().rex_in_use = true;

        re_st().rex.reg_match = ptr::null_mut();
        re_st().rex.reg_mmatch = rmp;
        re_st().rex.reg_buf = g_st().curbuf;
        re_st().rex.reg_firstlnum = lnum;
        re_st().rex.reg_maxline = (*g_st().curbuf).b_ml.ml_line_count - lnum;
        re_st().rex.reg_line_lbr = false;
        let result = vim_regsub_both(source, ptr::null_mut(), dest, destlen, flags);

        re_st().rex_in_use = rex_in_use_save;
        if re_st().rex_in_use {
            re_st().rex = rex_save;
        }

        result
    }
}

unsafe fn vim_regsub_both(
    source: *mut c_char,
    expr: *mut typval_T,
    dest: *mut c_char,
    destlen: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut no: c_int = -1;
        let mut func_all: Option<CaseConv> = None;
        let mut func_one: Option<CaseConv> = None;
        let mut clnum: linenr_T = 0;
        let mut len: c_int = 0;
        let copy = (flags & REGSUB_COPY) != 0;

        if (source.is_null() && expr.is_null()) || dest.is_null() {
            emsg(tr((&raw const e_null).cast()));
            return 0;
        }
        if prog_magic_wrong() {
            return 0;
        }
        if re_st().regsub_nesting == MAX_REGSUB_NESTING {
            emsg(tr(e_substitute_nesting_too_deep.as_ptr()));
            return 0;
        }
        let nested = re_st().regsub_nesting;
        let eval_slot = &mut re_st().eval_result[nested];
        let mut src = source;
        let mut dst = dest;

        if !expr.is_null() || (*source == b'\\' as c_char && *source.add(1) == b'=' as c_char) {
            if copy {
                if let Some(ref eval_cstr) = *eval_slot {
                    let eval_len = eval_cstr.to_bytes().len();
                    if eval_len < destlen as usize {
                        { let s = eval_cstr.as_ptr(); std::ptr::copy_nonoverlapping(s, dest, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                        dst = dst.add(eval_len);
                        *eval_slot = None;
                    }
                }
            } else {
                let prev_can_f_submatch = re_st().can_f_submatch;
                let mut rsm_save = regsubmatch_T::new();

                *eval_slot = None;

                if re_st().can_f_submatch {
                    rsm_save = re_st().rsm;
                }
                re_st().can_f_submatch = true;
                re_st().rsm.sm_match = re_st().rex.reg_match;
                re_st().rsm.sm_mmatch = re_st().rex.reg_mmatch;
                re_st().rsm.sm_firstlnum = re_st().rex.reg_firstlnum;
                re_st().rsm.sm_maxline = re_st().rex.reg_maxline;
                re_st().rsm.sm_line_lbr = c_int::from(re_st().rex.reg_line_lbr);

                re_st().regsub_nesting += 1;

                if !expr.is_null() {
                    let mut argv: [typval_T; 2] = [typval_T::new(); 2];
                    let mut rettv: typval_T = typval_T::new();
                    let mut matchList = staticList10_T::new();
                    rettv.v_type = VarType::VAR_STRING;
                    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                    argv[0].v_type = VarType::VAR_LIST;
                    argv[0].vval = crate::eval::typval::TypvalVval::List(&raw mut matchList.sl_list);
                    let mut funcexe = funcexe_T {
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
                    if (*expr).v_type == VarType::VAR_FUNC {
                        let s = (*expr).vval.v_string();
                        call_func(
                            s,
                            -1,
                            &raw mut rettv,
                            1,
                            argv.as_mut_ptr(),
                            &mut funcexe,
                        );
                    } else if (*expr).v_type == VarType::VAR_PARTIAL {
                        let partial = (*expr).vval.v_partial();
                        let s = partial_name(partial);
                        funcexe.fe_partial = partial;
                        call_func(
                            s,
                            -1,
                            &raw mut rettv,
                            1,
                            argv.as_mut_ptr(),
                            &mut funcexe,
                        );
                    }
                    if rettv.v_type == VarType::VAR_UNKNOWN {
                        *eval_slot = None;
                    } else {
                        let mut buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
                        let s = tv_get_string_buf_chk(&raw const rettv, buf.as_mut_ptr());
                        if !s.is_null() {
                            *eval_slot = Some(CStr::from_ptr(s).to_owned());
                        }
                    }
                    tv_clear(&raw mut rettv);
                } else {
                    *eval_slot = eval_to_string(source.add(2), true, false);
                }
                re_st().regsub_nesting -= 1;

                if let Some(eval_cstr) = eval_slot.take() {
                    let mut eval_bytes = eval_cstr.into_bytes_with_nul();
                    let mut had_backslash = false;
                    let mut s = eval_bytes.as_mut_ptr().cast::<c_char>();
                    while *s != NUL as c_char {
                        if *s == NL as c_char && re_st().rsm.sm_line_lbr == 0 {
                            *s = CAR as c_char;
                        } else if *s == b'\\' as c_char && *s.add(1) != NUL as c_char {
                            s = s.add(1);
                            if *s == NL as c_char && re_st().rsm.sm_line_lbr == 0 {
                                *s = CAR as c_char;
                            }
                            had_backslash = true;
                        }
                        mb_ptr_adv(&raw mut s);
                    }
                    let final_cstr = if had_backslash && (flags & REGSUB_BACKSLASH) != 0 {
                        vim_strsave_escaped(std::ffi::CStr::from_ptr(eval_bytes.as_ptr().cast()), c"\\")
                    } else {
                        CStr::from_bytes_with_nul(&eval_bytes)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    dst = dst.add(final_cstr.to_bytes().len());
                    *eval_slot = Some(final_cstr);
                }

                re_st().can_f_submatch = prev_can_f_submatch;
                if re_st().can_f_submatch {
                    re_st().rsm = rsm_save;
                }
            }
        } else {
            loop {
                let mut c = *src as u8 as c_int;
                src = src.add(1);
                if c == NUL as c_int {
                    break;
                }
                if c == b'&' as c_int && (flags & REGSUB_MAGIC) != 0 {
                    no = 0;
                } else if c == b'\\' as c_int && *src != NUL as c_char {
                    if *src == b'&' as c_char && (flags & REGSUB_MAGIC) == 0 {
                        src = src.add(1);
                        no = 0;
                    } else if (*src as u8).is_ascii_digit() {
                        no = (*src as u8 - b'0') as c_int;
                        src = src.add(1);
                    } else if !vim_strchr(c"uUlLeE".as_ptr(), *src as u8 as c_int).is_null() {
                        let ch = *src as u8;
                        src = src.add(1);
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
                    if c == c_int::from(K_SPECIAL)
                        && *src != NUL as c_char
                        && *src.add(1) != NUL as c_char
                    {
                        if copy {
                            if dst.add(3) > dest.add(destlen as usize) {
                                iemsg(c"vim_regsub_both(): not enough space".as_ptr());
                                return 0;
                            }
                            *dst = c as c_char;
                            dst = dst.add(1);
                            *dst = *src;
                            dst = dst.add(1);
                            src = src.add(1);
                            *dst = *src;
                            dst = dst.add(1);
                            src = src.add(1);
                        } else {
                            dst = dst.add(3);
                            src = src.add(2);
                        }
                        continue;
                    }

                    if c == b'\\' as c_int && *src != NUL as c_char {
                        match *src as u8 {
                            b'r' => {
                                c = CAR;
                                src = src.add(1);
                            }
                            b'n' => {
                                c = NL;
                                src = src.add(1);
                            }
                            b't' => {
                                c = TAB;
                                src = src.add(1);
                            }
                            b'b' => {
                                c = Ctrl_H;
                                src = src.add(1);
                            }
                            _ => {
                                if (flags & REGSUB_BACKSLASH) != 0 {
                                    if copy {
                                        if dst.add(1) > dest.add(destlen as usize) {
                                            iemsg(c"vim_regsub_both(): not enough space".as_ptr());
                                            return 0;
                                        }
                                        *dst = b'\\' as c_char;
                                    }
                                    dst = dst.add(1);
                                }
                                c = *src as u8 as c_int;
                                src = src.add(1);
                            }
                        }
                    } else {
                        c = utf_ptr2char(src.sub(1));
                    }

                    let cc = if let Some(f) = func_one.take() {
                        f.apply(c)
                    } else if let Some(f) = func_all {
                        f.apply(c)
                    } else {
                        c
                    };

                    let totlen = utfc_ptr2len(src.sub(1)) as usize;
                    let charlen = utf_char2len(cc) as usize;

                    if copy {
                        if dst.add(charlen) > dest.add(destlen as usize) {
                            iemsg(c"vim_regsub_both(): not enough space".as_ptr());
                            return 0;
                        }
                        utf_char2bytes(cc, dst);
                    }
                    dst = dst.offset(charlen as isize - 1);
                    let clen = utf_ptr2len(src.sub(1)) as usize;

                    if clen < totlen {
                        if copy {
                            if dst.add(totlen - clen) > dest.add(destlen as usize) {
                                iemsg(c"vim_regsub_both(): not enough space".as_ptr());
                                return 0;
                            }
                            ptr::copy(src.sub(1).add(clen), dst.add(1), totlen - clen);
                        }
                        dst = dst.add(totlen - clen);
                    }
                    src = src.add(totlen - 1);
                    dst = dst.add(1);
                } else {
                    let mut s: *mut c_char;
                    if REG_MULTI() {
                        clnum = (*re_st().rex.reg_mmatch).startpos[no as usize].lnum;
                        if clnum < 0 || (*re_st().rex.reg_mmatch).endpos[no as usize].lnum < 0 {
                            s = ptr::null_mut();
                        } else {
                            s = reg_getline(clnum)
                                .add((*re_st().rex.reg_mmatch).startpos[no as usize].col as usize);
                            if (*re_st().rex.reg_mmatch).endpos[no as usize].lnum == clnum {
                                len = (*re_st().rex.reg_mmatch).endpos[no as usize].col
                                    - (*re_st().rex.reg_mmatch).startpos[no as usize].col;
                            } else {
                                len = reg_getline_len(clnum)
                                    - (*re_st().rex.reg_mmatch).startpos[no as usize].col;
                            }
                        }
                    } else {
                        s = (*re_st().rex.reg_match).startp[no as usize];
                        if (*re_st().rex.reg_match).endp[no as usize].is_null() {
                            s = ptr::null_mut();
                        } else {
                            len = (*re_st().rex.reg_match).endp[no as usize].offset_from(s) as c_int;
                        }
                    }
                    if !s.is_null() {
                        loop {
                            if len == 0 {
                                if REG_MULTI() {
                                    if (*re_st().rex.reg_mmatch).endpos[no as usize].lnum == clnum {
                                        break;
                                    }
                                    if copy {
                                        if dst.add(1) > dest.add(destlen as usize) {
                                            iemsg(c"vim_regsub_both(): not enough space".as_ptr());
                                            return 0;
                                        }
                                        *dst = CAR as c_char;
                                    }
                                    dst = dst.add(1);
                                    clnum += 1;
                                    s = reg_getline(clnum);
                                    if (*re_st().rex.reg_mmatch).endpos[no as usize].lnum == clnum {
                                        len = (*re_st().rex.reg_mmatch).endpos[no as usize].col;
                                    } else {
                                        len = reg_getline_len(clnum);
                                    }
                                } else {
                                    break;
                                }
                            } else if *s == NUL as c_char {
                                if copy {
                                    iemsg(tr((&raw const e_re_damg).cast()));
                                }
                                return (dst.offset_from(dest) + 1) as c_int;
                            } else {
                                if (flags & REGSUB_BACKSLASH) != 0
                                    && (*s == CAR as c_char || *s == b'\\' as c_char)
                                {
                                    if copy {
                                        if dst.add(2) > dest.add(destlen as usize) {
                                            iemsg(c"vim_regsub_both(): not enough space".as_ptr());
                                            return 0;
                                        }
                                        *dst = b'\\' as c_char;
                                        *dst.add(1) = *s;
                                    }
                                    dst = dst.add(2);
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
                                    s = s.add(l as usize);
                                    len -= l;
                                    let charlen = utf_char2len(cc) as usize;
                                    if copy {
                                        if dst.add(charlen) > dest.add(destlen as usize) {
                                            iemsg(c"vim_regsub_both(): not enough space".as_ptr());
                                            return 0;
                                        }
                                        utf_char2bytes(cc, dst);
                                    }
                                    dst = dst.offset(charlen as isize - 1);
                                    dst = dst.add(1);
                                }
                                s = s.add(1);
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
        (dst.offset_from(dest) + 1) as c_int
    }
}

fn reg_getline_submatch(lnum: linenr_T) -> *mut c_char {
    unsafe {
        let mut line: *mut c_char = ptr::null_mut();
        reg_getline_common(
            lnum,
            RGLF_LINE | RGLF_SUBMATCH,
            &raw mut line,
            ptr::null_mut(),
        );
        line
    }
}

fn reg_getline_submatch_len(lnum: linenr_T) -> colnr_T {
    unsafe {
        let mut length: colnr_T = 0;
        reg_getline_common(
            lnum,
            RGLF_LENGTH | RGLF_SUBMATCH,
            ptr::null_mut(),
            &raw mut length,
        );
        length
    }
}

pub fn reg_submatch(no: c_int) -> Option<std::ffi::CString> {
    unsafe {
        let mut retval_vec: Vec<c_char>;
        let mut retval: *mut c_char = ptr::null_mut();
        if !re_st().can_f_submatch || no < 0 {
            return None;
        }
        let no = no as usize;

        if re_st().rsm.sm_match.is_null() {
            for round in 1..=2 {
                let mut lnum = (*re_st().rsm.sm_mmatch).startpos[no].lnum;
                if lnum < 0 || (*re_st().rsm.sm_mmatch).endpos[no].lnum < 0 {
                    return None;
                }

                let mut s = reg_getline_submatch(lnum);
                if s.is_null() {
                    break;
                }
                s = s.add((*re_st().rsm.sm_mmatch).startpos[no].col as usize);
                let mut len: isize;
                if (*re_st().rsm.sm_mmatch).endpos[no].lnum == lnum {
                    len = ((*re_st().rsm.sm_mmatch).endpos[no].col - (*re_st().rsm.sm_mmatch).startpos[no].col)
                        as isize;
                    if round == 2 {
                        { let _d = (retval).cast::<u8>(); let _n = len as usize; if _n > 0 { std::ptr::copy_nonoverlapping((s).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
                    }
                    len += 1;
                } else {
                    len = (reg_getline_submatch_len(lnum) - (*re_st().rsm.sm_mmatch).startpos[no].col)
                        as isize;
                    if round == 2 {
                        { std::ptr::copy_nonoverlapping(s, retval, std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                        *retval.add(len as usize) = b'\n' as c_char;
                    }
                    len += 1;
                    lnum += 1;
                    while lnum < (*re_st().rsm.sm_mmatch).endpos[no].lnum {
                        s = reg_getline_submatch(lnum);
                        if round == 2 {
                            { std::ptr::copy_nonoverlapping(s, retval.add(len as usize), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                        }
                        len += reg_getline_submatch_len(lnum) as isize;
                        if round == 2 {
                            *retval.add(len as usize) = b'\n' as c_char;
                        }
                        len += 1;
                        lnum += 1;
                    }
                    if round == 2 {
                        {
                            let dst = retval.add(len as usize);
                            let src = reg_getline_submatch(lnum);
                            let cap_n = (*re_st().rsm.sm_mmatch).endpos[no].col as usize;
                            let copy_n = crate::memory::xstrnlen(src, cap_n);
                            ptr::copy_nonoverlapping(src, dst, copy_n);
                            ptr::write_bytes(dst.add(copy_n), 0, cap_n - copy_n);
                        }
                    }
                    len += (*re_st().rsm.sm_mmatch).endpos[no].col as isize;
                    if round == 2 {
                        *retval.add(len as usize) = NUL as c_char;
                    }
                    len += 1;
                }

                if retval.is_null() {
                    retval_vec = vec![0; len as usize];
                    retval = retval_vec.as_mut_ptr();
                }
            }
            (!retval.is_null()).then(|| CStr::from_ptr(retval).to_owned())
        } else {
            let s = (*re_st().rsm.sm_match).startp[no];
            if s.is_null() || (*re_st().rsm.sm_match).endp[no].is_null() {
                None
            } else {
                cstring_from_bytes(s, (*re_st().rsm.sm_match).endp[no].offset_from(s) as usize)
            }
        }
    }
}

pub fn reg_submatch_list(no: c_int) -> *mut list_T {
    unsafe {
        if !re_st().can_f_submatch || no < 0 {
            return ptr::null_mut();
        }
        let no = no as usize;
        let list: *mut list_T;

        if re_st().rsm.sm_match.is_null() {
            let slnum = (*re_st().rsm.sm_mmatch).startpos[no].lnum;
            let elnum = (*re_st().rsm.sm_mmatch).endpos[no].lnum;
            if slnum < 0 || elnum < 0 {
                return ptr::null_mut();
            }

            let scol = (*re_st().rsm.sm_mmatch).startpos[no].col;
            let ecol = (*re_st().rsm.sm_mmatch).endpos[no].col;

            list = tv_list_alloc((elnum - slnum + 1) as isize);

            let mut s: *const c_char = reg_getline_submatch(slnum).add(scol as usize);
            if slnum == elnum {
                tv_list_append_string(list, s, (ecol - scol) as isize);
            } else {
                let max_lnum = elnum - slnum;
                tv_list_append_string(list, s, -1);
                for i in 1..max_lnum {
                    s = reg_getline_submatch(slnum + i);
                    tv_list_append_string(list, s, -1);
                }
                s = reg_getline_submatch(elnum);
                tv_list_append_string(list, s, ecol as isize);
            }
        } else {
            let s = (*re_st().rsm.sm_match).startp[no];
            if s.is_null() || (*re_st().rsm.sm_match).endp[no].is_null() {
                return ptr::null_mut();
            }
            list = tv_list_alloc(1);
            tv_list_append_string(list, s, (*re_st().rsm.sm_match).endp[no].offset_from(s));
        }

        tv_list_ref(list);
        list
    }
}

pub(super) unsafe fn init_regexec_multi(
    rmp: &mut regmmatch_T,
    win: *mut win_T,
    buf: *mut buf_T,
    lnum: linenr_T,
) {
    unsafe {
        re_st().rex.reg_match = ptr::null_mut();
        re_st().rex.reg_mmatch = rmp;
        re_st().rex.reg_buf = buf;
        re_st().rex.reg_win = win;
        re_st().rex.reg_firstlnum = lnum;
        re_st().rex.reg_maxline = (*re_st().rex.reg_buf).b_ml.ml_line_count - lnum;
        re_st().rex.reg_line_lbr = false;
        re_st().rex.reg_ic = rmp.rmm_ic != 0;
        re_st().rex.reg_icombine = false;
        re_st().rex.reg_nobreak = ((*rmp.regprog).re_flags & (RE_NOBREAK as c_uint)) != 0;
        re_st().rex.reg_maxcol = rmp.rmm_maxcol;
    }
}

pub(super) unsafe fn get_char_class_bits(
    start: *const u8,
    end: *const u8,
    newl: &mut bool,
) -> c_int {
    unsafe {
        let mut config: c_int = 0;
        *newl = false;
        if *end != b']' {
            return -1;
        }
        let mut p = start;
        if *p == b'^' {
            config |= CLASS_not;
            p = p.add(1);
        }

        while p < end {
            if p.add(2) < end && *p.add(1) == b'-' {
                match *p {
                    b'0' => {
                        if *p.add(2) == b'9' {
                            config |= CLASS_o9;
                        } else if *p.add(2) == b'7' {
                            config |= CLASS_o7;
                        } else {
                            return -1;
                        }
                    }
                    b'a' => {
                        if *p.add(2) == b'z' {
                            config |= CLASS_az;
                        } else if *p.add(2) == b'f' {
                            config |= CLASS_af;
                        } else {
                            return -1;
                        }
                    }
                    b'A' => {
                        if *p.add(2) == b'Z' {
                            config |= CLASS_AZ;
                        } else if *p.add(2) == b'F' {
                            config |= CLASS_AF;
                        } else {
                            return -1;
                        }
                    }
                    _ => return -1,
                }
                p = p.add(3);
            } else if p.add(1) < end && *p == b'\\' && *p.add(1) == b'n' {
                *newl = true;
                p = p.add(2);
            } else if *p == b'_' {
                config |= CLASS_underscore;
                p = p.add(1);
            } else if *p == b'\n' {
                *newl = true;
                p = p.add(1);
            } else {
                return -1;
            }
        }

        if p != end {
            return -1;
        }
        if (config & (CLASS_az | CLASS_AZ)) != 0 {
            return -1;
        }
        config
    }
}

pub(super) unsafe fn regcomp_start(expr: *mut u8, re_flags: c_int) {
    unsafe {
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
}

pub(super) fn seen_endbrace(refnum: c_int) -> bool {
    unsafe {
        if re_st().had_endbrace[refnum as usize] == 0 {
            let mut p = re_st().regparse.cast::<u8>();
            while *p != NUL {
                if *p == b'@' && *p.add(1) == b'<' && (*p.add(2) == b'!' || *p.add(2) == b'=') {
                    break;
                }
                p = p.add(1);
            }
            if *p == NUL {
                emsg(tr(c"E65: Illegal back reference".as_ptr()));
                g_st().rc_did_emsg = true;
                return false;
            }
        }
        true
    }
}

pub(super) fn coll_get_char() -> c_int {
    unsafe {
        let mut nr: i64 = -1;
        let ch = *re_st().regparse as u8;
        re_st().regparse = re_st().regparse.add(1);
        match ch {
            b'd' => nr = getdecchrs(),
            b'o' => nr = getoctchrs(),
            b'x' => nr = gethexchrs(2),
            b'u' => nr = gethexchrs(4),
            b'U' => nr = gethexchrs(8),
            _ => {}
        }
        if nr < 0 {
            re_st().regparse = re_st().regparse.sub(1);
            nr = i64::from(b'\\');
        }
        if nr > i64::from(c_int::MAX) {
            nr = i64::from(c_int::MAX);
        }
        nr as c_int
    }
}

pub fn vim_regcomp_had_eol() -> c_int {
    re_st().had_eol
}

/// Compile a regular expression into internal code.
pub unsafe fn vim_regcomp(expr_arg: *const c_char, re_flags: c_int) -> *mut regprog_T {
    unsafe {
        let mut expr = expr_arg;

        re_st().regexp_engine = opt_vars().p_re as c_int;

        if crate::memory::cstr_prefix_bytes(expr, 4) == &b"\\%#="[..4] {
            let newengine = (*expr.add(4) as u8).wrapping_sub(b'0') as c_int;
            if newengine == AUTOMATIC_ENGINE
                || newengine == BACKTRACKING_ENGINE
                || newengine == NFA_ENGINE
            {
                re_st().regexp_engine = newengine;
                expr = expr.add(5);
            } else {
                emsg(tr(
                    c"E864: \\%#= can only be followed by 0, 1, or 2. The automatic engine will be used "
                        .as_ptr(),
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
                nfa.re_engine = re_st().regexp_engine as c_uint;
                regprog_T {
                    regflags: nfa.regflags,
                    re_engine: re_st().regexp_engine as c_uint,
                    re_flags: re_flags as c_uint,
                    re_in_use: false,
                    engine: RegProgEngine::Nfa(nfa),
                }
            })
        } else {
            bt_regcomp(expr.cast_mut().cast(), re_flags).map(|bt| regprog_T {
                regflags: bt.regflags,
                re_engine: re_st().regexp_engine as c_uint,
                re_flags: re_flags as c_uint,
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
            prog_opt = bt_regcomp(expr.cast_mut().cast(), re_flags).map(|bt| regprog_T {
                regflags: bt.regflags,
                re_engine: re_st().regexp_engine as c_uint,
                re_flags: re_flags as c_uint,
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
pub unsafe fn vim_regcomp_box(expr_arg: *const c_char, re_flags: c_int) -> Option<Box<regprog_T>> {
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
pub fn regprog_as_ptr(prog: &Option<Box<regprog_T>>) -> *mut regprog_T {
    prog.as_deref()
        .map_or(ptr::null_mut(), |p| ptr::from_ref(p).cast_mut())
}

/// Free a compiled regexp program, returned by `vim_regcomp()`.
pub unsafe fn vim_regfree(prog: *mut regprog_T) {
    unsafe {
        if !prog.is_null() {
            drop(Box::from_raw(prog));
        }
    }
}

pub fn free_regexp_stuff() {
    unsafe {
        let rs = &raw mut re_st().regstack;
        *rs = Vec::new();
        let bp = &raw mut re_st().backpos;
        *bp = Vec::new();
        let tf = &raw mut re_st().reg_tofree;
        *tf = Vec::new();
        re_st().reg_prev_sub = None;
        let pv = &raw mut re_st().post_vec;
        *pv = Vec::new();
    }
}

unsafe fn report_re_switch(pat: *const c_char) {
    unsafe {
        if opt_vars().p_verbose > 0 {
            verbose_enter();
            msg_puts(tr(
                c"Switching to backtracking RE engine for pattern: ".as_ptr()
            ));
            msg_puts(pat);
            verbose_leave();
        }
    }
}

unsafe fn vim_regexec_string(
    rmp: &mut regmatch_T,
    line: *const c_char,
    col: colnr_T,
    nl: bool,
) -> bool {
    unsafe {
        let mut rex_save = regexec_T::new();
        let rex_in_use_save = re_st().rex_in_use;

        if (*rmp.regprog).re_in_use {
            emsg(tr(e_recursive.as_ptr()));
            return false;
        }
        (*rmp.regprog).re_in_use = true;

        if re_st().rex_in_use {
            rex_save = re_st().rex;
        }
        re_st().rex_in_use = true;

        re_st().rex.reg_startp = ptr::null_mut();
        re_st().rex.reg_endp = ptr::null_mut();
        re_st().rex.reg_startpos = ptr::null_mut();
        re_st().rex.reg_endpos = ptr::null_mut();

        let mut result = match (*rmp.regprog).engine {
            RegProgEngine::Bt(_) => bt_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
            RegProgEngine::Nfa(_) => nfa_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
        };
        (*rmp.regprog).re_in_use = false;

        if (*rmp.regprog).re_engine == AUTOMATIC_ENGINE as c_uint && result == NFA_TOO_EXPENSIVE
        {
            let save_p_re = opt_vars().p_re;
            let re_flags = (*rmp.regprog).re_flags as c_int;
            let prev_prog = rmp.regprog;
            let pat = (*prev_prog).nfa_mut().pattern.as_ptr();

            opt_vars().p_re = OptInt::from(BACKTRACKING_ENGINE);
            report_re_switch(pat);
            let new_prog = vim_regcomp(pat, re_flags);
            if !new_prog.is_null() {
                *prev_prog = *Box::from_raw(new_prog);
                (*rmp.regprog).re_in_use = true;
                result = match (*rmp.regprog).engine {
                    RegProgEngine::Bt(_) => bt_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
                    RegProgEngine::Nfa(_) => nfa_regexec_nl(rmp, line.cast_mut().cast(), col, nl),
                };
                (*rmp.regprog).re_in_use = false;
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

pub unsafe fn vim_regexec_prog(
    prog: Option<&regprog_T>,
    ignore_case: bool,
    line: *const c_char,
    col: colnr_T,
) -> bool {
    unsafe {
        let mut regmatch = regmatch_T {
            regprog: prog.map_or(ptr::null_mut(), |p| ptr::from_ref(p).cast_mut()),
            startp: [ptr::null_mut(); NSUBEXP],
            endp: [ptr::null_mut(); NSUBEXP],
            rm_matchcol: 0,
            rm_ic: ignore_case,
        };
        vim_regexec_string(&mut regmatch, line, col, false)
    }
}

pub unsafe fn vim_regexec(rmp: &mut regmatch_T, line: *const c_char, col: colnr_T) -> bool {
    unsafe { vim_regexec_string(rmp, line, col, false) }
}

pub unsafe fn vim_regexec_nl(rmp: &mut regmatch_T, line: *const c_char, col: colnr_T) -> bool {
    unsafe { vim_regexec_string(rmp, line, col, true) }
}

pub unsafe fn vim_regexec_multi(
    rmp: &mut regmmatch_T,
    win: *mut win_T,
    buf: *mut buf_T,
    lnum: linenr_T,
    col: colnr_T,
    tm: Option<proftime_T>,
    timed_out: *mut c_int,
) -> c_int {
    unsafe {
        let mut rex_save = regexec_T::new();
        let rex_in_use_save = re_st().rex_in_use;

        if (*rmp.regprog).re_in_use {
            emsg(tr(e_recursive.as_ptr()));
            return 0;
        }
        (*rmp.regprog).re_in_use = true;

        if re_st().rex_in_use {
            rex_save = re_st().rex;
        }
        re_st().rex_in_use = true;

        let mut result = match (*rmp.regprog).engine {
            RegProgEngine::Bt(_) => bt_regexec_multi(rmp, win, buf, lnum, col, tm, timed_out),
            RegProgEngine::Nfa(_) => nfa_regexec_multi(rmp, win, buf, lnum, col, tm, timed_out),
        };
        (*rmp.regprog).re_in_use = false;

        if (*rmp.regprog).re_engine == AUTOMATIC_ENGINE as c_uint && result == NFA_TOO_EXPENSIVE
        {
            let save_p_re = opt_vars().p_re;
            let re_flags = (*rmp.regprog).re_flags as c_int;
            let prev_prog = rmp.regprog;
            let pat = (*prev_prog).nfa_mut().pattern.as_ptr();

            opt_vars().p_re = OptInt::from(BACKTRACKING_ENGINE);

            report_re_switch(pat);
            g_st().reg_do_extmatch = REX_ALL;
            let new_prog = vim_regcomp(pat, re_flags);
            g_st().reg_do_extmatch = 0;

            if !new_prog.is_null() {
                *prev_prog = *Box::from_raw(new_prog);
                (*rmp.regprog).re_in_use = true;
                result = match (*rmp.regprog).engine {
                    RegProgEngine::Bt(_) => {
                        bt_regexec_multi(rmp, win, buf, lnum, col, tm, timed_out)
                    }
                    RegProgEngine::Nfa(_) => {
                        nfa_regexec_multi(rmp, win, buf, lnum, col, tm, timed_out)
                    }
                };
                (*rmp.regprog).re_in_use = false;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_helpers_round_trip() {
        let c = b'*' as c_int;
        let m = Magic(c);
        assert!(is_Magic(m));
        assert_eq!(un_Magic(m), c);
        assert_eq!(no_Magic(m), c);
        assert_eq!(no_Magic(c), c);
        assert_eq!(toggle_Magic(c), m);
        assert_eq!(toggle_Magic(m), c);
    }

    #[test]
    fn re_multi_type_classifies_operators() {
        assert_eq!(re_multi_type(Magic(b'*' as c_int)), MULTI_MULT);
        assert_eq!(re_multi_type(Magic(b'+' as c_int)), MULTI_MULT);
        assert_eq!(re_multi_type(Magic(b'{' as c_int)), MULTI_MULT);
        assert_eq!(re_multi_type(Magic(b'=' as c_int)), MULTI_ONE);
        assert_eq!(re_multi_type(Magic(b'?' as c_int)), MULTI_ONE);
        assert_eq!(re_multi_type(Magic(b'@' as c_int)), MULTI_ONE);
        assert_eq!(re_multi_type(b'*' as c_int), NOT_MULTI);
    }

    #[test]
    fn get_char_class_bits_recognizes_digit_and_hex() {
        unsafe {
            let mut newl = false;
            let digits = b"0-9]";
            assert_eq!(
                get_char_class_bits(digits.as_ptr(), digits.as_ptr().add(3), &mut newl),
                CLASS_o9
            );
            assert!(!newl);

            let hex_nl = b"^0-9a-fA-F\\n]";
            assert_eq!(
                get_char_class_bits(hex_nl.as_ptr(), hex_nl.as_ptr().add(12), &mut newl),
                CLASS_not | CLASS_o9 | CLASS_af | CLASS_AF
            );
            assert!(newl);
        }
    }
}
