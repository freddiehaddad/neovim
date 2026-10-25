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
    VarType, list_T, staticList10_T, tv_clear, tv_list_alloc, tv_list_append_string,
    tv_list_first, tv_list_init_static10, tv_list_len, typval_T,
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
use crate::memory::{xfree, xmalloc, xmemcpyz, xstrdup};
use crate::message::{emsg, iemsg, internal_error, msg_puts, semsg, verbose_enter, verbose_leave};
use crate::option_vars::*;
use crate::os::input::fast_breakcheck;
use crate::plines::{getvvcol, win_linetabsize};
use crate::profile::{profile_passed_limit, proftime_T};
pub(super) use crate::strings::{strcpy, strlen, strncmp, strncpy};
use crate::strings::{tr, xstrnsave};
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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct regprog_T {
    pub engine: *mut regengine_T,
    pub regflags: c_uint,
    pub re_engine: c_uint,
    pub re_flags: c_uint,
    pub re_in_use: bool,
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
pub struct reg_extmatch_T {
    pub refcnt: i16,
    pub matches: [*mut u8; NSUBEXP],
}

impl Drop for reg_extmatch_T {
    fn drop(&mut self) {
        for &m in &self.matches {
            unsafe {
                xfree(m.cast());
            }
        }
    }
}

#[repr(C)]
pub(super) struct bt_regprog_T {
    pub engine: *mut regengine_T,
    pub regflags: c_uint,
    pub re_engine: c_uint,
    pub re_flags: c_uint,
    pub re_in_use: bool,
    pub regstart: c_int,
    pub reganch: u8,
    pub regmust: *mut u8,
    pub regmlen: c_int,
    pub reghasz: u8,
    pub program: Vec<u8>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct nfa_state_T {
    pub c: c_int,
    pub out: *mut nfa_state_T,
    pub out1: *mut nfa_state_T,
    pub id: c_int,
    pub lastlist: [c_int; 2],
    pub val: c_int,
}

impl nfa_state_T {
    pub(super) const fn new() -> Self {
        Self {
            c: 0,
            out: ptr::null_mut(),
            out1: ptr::null_mut(),
            id: 0,
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

#[repr(C)]
pub(super) struct nfa_regprog_T {
    pub engine: *mut regengine_T,
    pub regflags: c_uint,
    pub re_engine: c_uint,
    pub re_flags: c_uint,
    pub re_in_use: bool,
    pub start: *mut nfa_state_T,
    pub reganch: c_int,
    pub regstart: c_int,
    pub match_text: *mut u8,
    pub has_zend: c_int,
    pub has_backref: c_int,
    pub reghasz: c_int,
    pub pattern: *mut c_char,
    pub nsubexp: c_int,
    pub nstate: c_int,
    pub listbuf: [Vec<nfa_thread_T>; 2],
    pub state: Vec<nfa_state_T>,
}

impl Drop for nfa_regprog_T {
    fn drop(&mut self) {
        unsafe {
            xfree(self.match_text.cast());
            xfree(self.pattern.cast());
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct regengine_T {
    pub regcomp: unsafe fn(*mut u8, c_int) -> *mut regprog_T,
    pub regfree: unsafe fn(*mut regprog_T),
    pub regexec_nl: unsafe fn(*mut regmatch_T, *mut u8, colnr_T, bool) -> c_int,
    pub regexec_multi: unsafe fn(
        *mut regmmatch_T,
        *mut win_T,
        *mut buf_T,
        linenr_T,
        colnr_T,
        *mut proftime_T,
        *mut c_int,
    ) -> c_int,
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

#[repr(transparent)]
#[derive(Clone, Copy)]
pub(super) struct Ptrlist(pub *mut Ptrlist);

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Frag_T {
    pub start: *mut nfa_state_T,
    pub out: *mut Ptrlist,
}

impl Frag_T {
    pub(super) const fn new() -> Self {
        Self {
            start: ptr::null_mut(),
            out: ptr::null_mut(),
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

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct nfa_pim_T {
    pub result: c_int,
    pub state: *mut nfa_state_T,
    pub subs: regsubs_T,
    pub end: RegPos,
}

impl nfa_pim_T {
    pub(super) const fn new() -> Self {
        Self {
            result: 0,
            state: ptr::null_mut(),
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

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct nfa_thread_T {
    pub state: *mut nfa_state_T,
    pub count: c_int,
    pub pim: nfa_pim_T,
    pub subs: regsubs_T,
}

impl nfa_thread_T {
    pub(super) const fn new() -> Self {
        Self {
            state: ptr::null_mut(),
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

pub(super) type fptr_T = Option<unsafe fn(*mut c_int, c_int)>;

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
pub(super) static mut reg_prev_sub: *mut c_char = ptr::null_mut();
pub(super) static mut reg_prev_sublen: usize = 0;

pub(super) static mut class_tab: [i16; 256] = [0; 256];
static mut init_class_tab_done: bool = false;

pub(super) static mut regparse: *mut c_char = ptr::null_mut();
pub(super) static mut regnpar: c_int = 0;
pub(super) static mut wants_nfa: bool = false;
pub(super) static mut regnzpar: c_int = 0;
pub(super) static mut re_has_z: c_int = 0;
pub(super) static mut regflags: c_uint = 0;
pub(super) static mut had_eol: c_int = 0;
pub(super) static mut reg_magic: magic_T = 0;
pub(super) static mut reg_string: c_int = 0;
pub(super) static mut reg_strict: c_int = 0;

pub(super) static mut curchr: c_int = 0;
pub(super) static mut prevchr: c_int = 0;
pub(super) static mut prevprevchr: c_int = 0;
pub(super) static mut nextchr: c_int = 0;
pub(super) static mut prevchr_len: c_int = 0;
pub(super) static mut at_start: c_int = 0;
pub(super) static mut prev_at_start: c_int = 0;
static mut peekchr_after_slash: c_int = 0;

pub(super) static mut reg_cpo_lit: c_int = 0;

pub(super) static mut reg_tofree: *mut u8 = ptr::null_mut();
pub(super) static mut reg_tofreelen: c_uint = 0;

pub(super) static mut rex: regexec_T = regexec_T {
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
};
pub(super) static mut rex_in_use: bool = false;
pub(super) static mut can_f_submatch: bool = false;
pub(super) static mut rsm: regsubmatch_T = regsubmatch_T {
    sm_match: ptr::null_mut(),
    sm_mmatch: ptr::null_mut(),
    sm_firstlnum: 0,
    sm_maxline: 0,
    sm_line_lbr: 0,
};

pub(super) static mut reg_startzp: [*mut u8; NSUBEXP] = [ptr::null_mut(); NSUBEXP];
pub(super) static mut reg_endzp: [*mut u8; NSUBEXP] = [ptr::null_mut(); NSUBEXP];
pub(super) static mut reg_startzpos: [lpos_T; NSUBEXP] = [lpos_T { lnum: 0, col: 0 }; NSUBEXP];
pub(super) static mut reg_endzpos: [lpos_T; NSUBEXP] = [lpos_T { lnum: 0, col: 0 }; NSUBEXP];

static mut eval_result: [*mut c_char; MAX_REGSUB_NESTING] = [ptr::null_mut(); MAX_REGSUB_NESTING];
static mut regsub_nesting: usize = 0;

pub(super) static mut had_endbrace: [u8; NSUBEXP] = [0; NSUBEXP];
pub(super) static mut num_complex_braces: c_int = 0;
pub(super) static mut regcode: *mut u8 = ptr::null_mut();
pub(super) static mut regsize: i64 = 0;
pub(super) static mut reg_toolong: c_int = 0;
pub(super) static mut bt_reg_parse_depth: c_int = 0;
pub(super) static mut brace_min: [i64; 10] = [0; 10];
pub(super) static mut brace_max: [i64; 10] = [0; 10];
pub(super) static mut brace_count: [c_int; 10] = [0; 10];
pub(super) static mut one_exactly: bool = false;

pub(super) static mut regstack: Vec<u64> = Vec::new();
static mut backpos: Vec<backpos_T> = Vec::new();
pub(super) static mut behind_pos: regsave_T = regsave_T::new();

pub(super) static mut nfa_re_flags: c_int = 0;
pub(super) static mut post_vec: Vec<c_int> = Vec::new();
pub(super) static mut nfa_reg_parse_depth: c_int = 0;
pub(super) static mut nstate: c_int = 0;
pub(super) static mut istate: c_int = 0;
pub(super) static mut nfa_endp: *mut save_se_T = ptr::null_mut();
pub(super) static mut nfa_ll_index: usize = 0;
pub(super) static mut state_ptr: *mut nfa_state_T = ptr::null_mut();
pub(super) static mut nfa_stack: Vec<Frag_T> = Vec::new();

pub(super) const BT_REGENGINE: regengine_T = regengine_T {
    regcomp: bt_regcomp,
    regfree: bt_regfree,
    regexec_nl: bt_regexec_nl,
    regexec_multi: bt_regexec_multi,
};

pub(super) const NFA_REGENGINE: regengine_T = regengine_T {
    regcomp: nfa_regcomp,
    regfree: nfa_regfree,
    regexec_nl: nfa_regexec_nl,
    regexec_multi: nfa_regexec_multi,
};

static BT_REGENGINE_STATIC: regengine_T = BT_REGENGINE;
static NFA_REGENGINE_STATIC: regengine_T = NFA_REGENGINE;

#[inline]
pub(super) fn bt_regengine_ptr() -> *mut regengine_T {
    (&raw const BT_REGENGINE_STATIC).cast_mut()
}

#[inline]
pub(super) fn nfa_regengine_ptr() -> *mut regengine_T {
    (&raw const NFA_REGENGINE_STATIC).cast_mut()
}

static mut regexp_engine: c_int = 0;

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
pub(super) unsafe fn REG_MULTI() -> bool {
    unsafe { rex.reg_match.is_null() }
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
                if strncmp(target.cast(), pat.as_ptr().cast(), pat.len()) == 0 {
                    *pp = (*pp).add(pat.len() + 2);
                    return key;
                }
            }
        }
        CLASS_NONE
    }
}

pub(super) unsafe fn init_class_tab() {
    unsafe {
        if init_class_tab_done {
            return;
        }
        let tab = (&raw mut class_tab).cast::<i16>();
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
        init_class_tab_done = true;
    }
}

#[inline]
pub(super) unsafe fn ri_digit(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_DIGIT) != 0
    }
}
#[inline]
pub(super) unsafe fn ri_hex(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_HEX) != 0
    }
}
#[inline]
pub(super) unsafe fn ri_octal(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_OCTAL) != 0
    }
}
#[inline]
pub(super) unsafe fn ri_word(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_WORD) != 0
    }
}
#[inline]
pub(super) unsafe fn ri_head(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_HEAD) != 0
    }
}
#[inline]
pub(super) unsafe fn ri_alpha(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_ALPHA) != 0
    }
}
#[inline]
pub(super) unsafe fn ri_lower(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_LOWER) != 0
    }
}
#[inline]
pub(super) unsafe fn ri_upper(c: c_int) -> bool {
    unsafe {
        (0..0x100).contains(&c)
            && (*(&raw const class_tab).cast::<i16>().add(c as usize) & RI_UPPER) != 0
    }
}

/// Return true if compiled regular expression `prog` can match a line break.
pub unsafe fn re_multiline(prog: *const regprog_T) -> c_int {
    unsafe { ((*prog).regflags & RF_HASNL) as c_int }
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

pub(super) unsafe fn get_cpo_flags() {
    unsafe {
        reg_cpo_lit = (!vim_strchr(p_cpo, kCpoLiteral).is_null()) as c_int;
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
                    || (reg_cpo_lit == 0
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
            ptr::null_mut(),
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
    newp: *mut *mut c_char,
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
                if dirc == b'?' as c_int && !newp.is_null() && *p.add(1) == b'?' as c_char {
                    if startplen == 0 {
                        startplen = strlen(startp);
                    }
                    if (*newp).is_null() {
                        *newp = xstrnsave(startp, startplen);
                        p = (*newp).add(p.offset_from(startp) as usize);
                        startp = *newp;
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
    unsafe {
        regparse = str_;
        prevchr_len = 0;
        nextchr = -1;
        prevchr = -1;
        prevprevchr = -1;
        curchr = -1;
        at_start = 1;
        prev_at_start = 0;
    }
}

pub(super) unsafe fn save_parse_state(ps: *mut parse_state_T) {
    unsafe {
        (*ps).regparse = regparse;
        (*ps).prevchr_len = prevchr_len;
        (*ps).curchr = curchr;
        (*ps).prevchr = prevchr;
        (*ps).prevprevchr = prevprevchr;
        (*ps).nextchr = nextchr;
        (*ps).at_start = at_start;
        (*ps).prev_at_start = prev_at_start;
        (*ps).regnpar = regnpar;
    }
}

pub(super) unsafe fn restore_parse_state(ps: *mut parse_state_T) {
    unsafe {
        regparse = (*ps).regparse;
        prevchr_len = (*ps).prevchr_len;
        curchr = (*ps).curchr;
        prevchr = (*ps).prevchr;
        prevprevchr = (*ps).prevprevchr;
        nextchr = (*ps).nextchr;
        at_start = (*ps).at_start;
        prev_at_start = (*ps).prev_at_start;
        regnpar = (*ps).regnpar;
    }
}

pub(super) unsafe fn peekchr() -> c_int {
    unsafe {
        if curchr != -1 {
            return curchr;
        }

        curchr = *regparse as u8 as c_int;
        match curchr as u8 {
            b'.' | b'[' | b'~' => {
                if reg_magic >= MAGIC_ON {
                    curchr = Magic(curchr);
                }
            }
            b'(' | b')' | b'{' | b'%' | b'+' | b'=' | b'?' | b'@' | b'!' | b'&' | b'|' | b'<'
            | b'>' | b'#' | b'"' | b'\'' | b',' | b'-' | b':' | b';' | b'`' | b'/' => {
                if reg_magic == MAGIC_ALL {
                    curchr = Magic(curchr);
                }
            }
            b'*' => {
                if reg_magic >= MAGIC_ON
                    && at_start == 0
                    && !(prev_at_start != 0 && prevchr == Magic(b'^' as c_int))
                    && (peekchr_after_slash != 0
                        || (prevchr != Magic(b'(' as c_int)
                            && prevchr != Magic(b'&' as c_int)
                            && prevchr != Magic(b'|' as c_int)))
                {
                    curchr = Magic(b'*' as c_int);
                }
            }
            b'^' => {
                if reg_magic >= MAGIC_OFF
                    && (at_start != 0
                        || reg_magic == MAGIC_ALL
                        || prevchr == Magic(b'(' as c_int)
                        || prevchr == Magic(b'|' as c_int)
                        || prevchr == Magic(b'&' as c_int)
                        || prevchr == Magic(b'n' as c_int)
                        || (no_Magic(prevchr) == b'(' as c_int
                            && prevprevchr == Magic(b'%' as c_int)))
                {
                    curchr = Magic(b'^' as c_int);
                    at_start = 1;
                    prev_at_start = 0;
                }
            }
            b'$' => {
                if reg_magic >= MAGIC_OFF {
                    let mut p = regparse.add(1).cast::<u8>();
                    let mut is_magic_all = reg_magic == MAGIC_ALL;

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
                        || reg_magic == MAGIC_ALL
                    {
                        curchr = Magic(b'$' as c_int);
                    }
                }
            }
            b'\\' => {
                let c = *regparse.add(1) as u8 as c_int;
                if c == NUL as c_int {
                    curchr = b'\\' as c_int;
                } else if c <= b'~' as c_int && META_flags[c as usize] != 0 {
                    curchr = -1;
                    prev_at_start = at_start;
                    at_start = 0;
                    regparse = regparse.add(1);
                    peekchr_after_slash += 1;
                    let _ = peekchr();
                    regparse = regparse.sub(1);
                    peekchr_after_slash -= 1;
                    curchr = toggle_Magic(curchr);
                } else if !vim_strchr(REGEXP_ABBR.as_ptr(), c).is_null() {
                    curchr = backslash_trans(c);
                } else if reg_magic == MAGIC_NONE && (c == b'$' as c_int || c == b'^' as c_int) {
                    curchr = toggle_Magic(c);
                } else {
                    curchr = if c >= 0x80 {
                        utf_ptr2char(regparse.add(1))
                    } else {
                        c
                    };
                }
            }
            _ => {
                if curchr >= 0x80 {
                    curchr = utf_ptr2char(regparse);
                }
            }
        }

        curchr
    }
}

pub(super) unsafe fn skipchr() {
    unsafe {
        if *regparse == b'\\' as c_char {
            prevchr_len = 1;
        } else {
            prevchr_len = 0;
        }
        if *regparse.add(prevchr_len as usize) != NUL as c_char {
            if (*regparse.add(prevchr_len as usize) as u8) < 0x80 {
                prevchr_len += 1;
            } else {
                prevchr_len += utf_ptr2len(regparse.add(prevchr_len as usize));
            }
        }
        regparse = regparse.add(prevchr_len as usize);
        prev_at_start = at_start;
        at_start = 0;
        prevprevchr = prevchr;
        prevchr = curchr;
        curchr = nextchr;
        nextchr = -1;
    }
}

pub(super) unsafe fn skipchr_keepstart() {
    unsafe {
        let as_ = prev_at_start;
        let pr = prevchr;
        let prpr = prevprevchr;

        skipchr();
        at_start = as_;
        prevchr = pr;
        prevprevchr = prpr;
    }
}

pub(super) unsafe fn getchr() -> c_int {
    unsafe {
        let chr = peekchr();
        skipchr();
        chr
    }
}

pub(super) unsafe fn ungetchr() {
    unsafe {
        nextchr = curchr;
        curchr = prevchr;
        prevchr = prevprevchr;
        at_start = prev_at_start;
        prev_at_start = 0;
        regparse = regparse.sub(prevchr_len as usize);
    }
}

pub(super) unsafe fn gethexchrs(maxinputlen: c_int) -> i64 {
    unsafe {
        let mut nr: i64 = 0;
        let mut i: c_int = 0;
        while i < maxinputlen {
            let c = *regparse as u8 as c_int;
            if !ascii_isxdigit(c) {
                break;
            }
            nr <<= 4;
            nr |= i64::from(hex2nr(c));
            regparse = regparse.add(1);
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) unsafe fn getdecchrs() -> i64 {
    unsafe {
        let mut nr: i64 = 0;
        let mut i: c_int = 0;
        loop {
            let c = *regparse as u8;
            if !c.is_ascii_digit() {
                break;
            }
            nr = nr.wrapping_mul(10).wrapping_add(i64::from(c - b'0'));
            regparse = regparse.add(1);
            curchr = -1;
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) unsafe fn getoctchrs() -> i64 {
    unsafe {
        let mut nr: i64 = 0;
        let mut i: c_int = 0;
        while i < 3 && nr < 0o40 {
            let c = *regparse as u8;
            if !(b'0'..=b'7').contains(&c) {
                break;
            }
            nr <<= 3;
            nr |= i64::from(hex2nr(c as c_int));
            regparse = regparse.add(1);
            i += 1;
        }
        if i == 0 { -1 } else { nr }
    }
}

pub(super) unsafe fn read_limits(minval: *mut c_int, maxval: *mut c_int) -> c_int {
    unsafe {
        let mut reverse = false;
        if *regparse == b'-' as c_char {
            regparse = regparse.add(1);
            reverse = true;
        }
        let first_char = regparse;
        *minval = getdigits_int(&raw mut regparse, false, 0);
        if *regparse == b',' as c_char {
            regparse = regparse.add(1);
            if ascii_isdigit(*regparse as u8 as c_int) {
                *maxval = getdigits_int(&raw mut regparse, false, MAX_LIMIT);
            } else {
                *maxval = MAX_LIMIT;
            }
        } else if ascii_isdigit(*first_char as u8 as c_int) {
            *maxval = *minval;
        } else {
            *maxval = MAX_LIMIT;
        }
        if *regparse == b'\\' as c_char {
            regparse = regparse.add(1);
        }
        if *regparse != b'}' as c_char {
            semsg!(
                tr(c"E554: Syntax error in %s{...}".as_ptr()),
                if reg_magic == MAGIC_ALL {
                    c"".as_ptr()
                } else {
                    c"\\".as_ptr()
                },
            );
            rc_did_emsg = true;
            return FAIL;
        }
        if (!reverse && *minval > *maxval) || (reverse && *minval < *maxval) {
            ptr::swap(minval, maxval);
        }
        skipchr();
        OK
    }
}

pub(super) unsafe fn reg_breakcheck() {
    unsafe {
        if !rex.reg_nobreak {
            fast_breakcheck();
        }
    }
}

pub(super) unsafe fn reg_iswordc(c: c_int) -> bool {
    unsafe { vim_iswordc_buf(c, rex.reg_buf) }
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
            (rsm.sm_firstlnum + lnum, rsm.sm_maxline)
        } else {
            (rex.reg_firstlnum + lnum, rex.reg_maxline)
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
            *line = ml_get_buf(rex.reg_buf, firstlnum);
        }
        if get_length {
            *length = ml_get_buf_len(rex.reg_buf, firstlnum);
        }
    }
}

pub(super) unsafe fn reg_getline(lnum: linenr_T) -> *mut c_char {
    unsafe {
        let mut line: *mut c_char = ptr::null_mut();
        reg_getline_common(lnum, RGLF_LINE, &raw mut line, ptr::null_mut());
        line
    }
}

pub(super) unsafe fn reg_getline_len(lnum: linenr_T) -> colnr_T {
    unsafe {
        let mut length: colnr_T = 0;
        reg_getline_common(lnum, RGLF_LENGTH, ptr::null_mut(), &raw mut length);
        length
    }
}

pub(super) unsafe fn make_extmatch() -> *mut reg_extmatch_T {
    Box::into_raw(Box::new(reg_extmatch_T {
        refcnt: 1,
        matches: [ptr::null_mut(); NSUBEXP],
    }))
}

pub unsafe fn ref_extmatch(em: *mut reg_extmatch_T) -> *mut reg_extmatch_T {
    unsafe {
        if !em.is_null() {
            (*em).refcnt += 1;
        }
        em
    }
}

pub unsafe fn unref_extmatch(em: *mut reg_extmatch_T) {
    unsafe {
        if !em.is_null() {
            (*em).refcnt -= 1;
            if (*em).refcnt <= 0 {
                drop(Box::from_raw(em));
            }
        }
    }
}

pub(super) unsafe fn reg_prev_class() -> c_int {
    unsafe {
        if rex.input > rex.line {
            let prev = rex.input.sub(1).cast::<c_char>();
            let head = prev.sub(utf_head_off(rex.line.cast(), prev) as usize);
            return mb_get_class_tab(head, (&raw const (*rex.reg_buf).b_chartab).cast());
        }
        -1
    }
}

pub(super) unsafe fn reg_match_visual() -> bool {
    unsafe {
        let wp = if rex.reg_win.is_null() {
            curwin
        } else {
            rex.reg_win
        };
        if rex.reg_buf != curbuf || Visual.start.lnum == 0 || !REG_MULTI() {
            return false;
        }

        let (top, bot, mode, curswant) = if Visual.active {
            let (t, b) = if lt(Visual.start, (*wp).w_cursor) {
                (Visual.start, (*wp).w_cursor)
            } else {
                ((*wp).w_cursor, Visual.start)
            };
            (t, b, Visual.mode, (*wp).w_curswant)
        } else {
            let (t, mut b) = if lt((*curbuf).b_visual.vi_start, (*curbuf).b_visual.vi_end) {
                ((*curbuf).b_visual.vi_start, (*curbuf).b_visual.vi_end)
            } else {
                ((*curbuf).b_visual.vi_end, (*curbuf).b_visual.vi_start)
            };
            if b.lnum > (*curbuf).b_ml.ml_line_count {
                b.lnum = (*curbuf).b_ml.ml_line_count;
            }
            (
                t,
                b,
                (*curbuf).b_visual.vi_mode,
                (*curbuf).b_visual.vi_curswant,
            )
        };

        let lnum = rex.lnum + rex.reg_firstlnum;
        if lnum < top.lnum || lnum > bot.lnum {
            return false;
        }

        let col = rex.input.offset_from(rex.line) as colnr_T;
        if mode == b'v' as c_int {
            if (lnum == top.lnum && col < top.col)
                || (lnum == bot.lnum && col >= bot.col + colnr_T::from(*p_sel != b'e' as c_char))
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
                &raw mut top_copy,
                &raw mut start,
                ptr::null_mut(),
                &raw mut end,
                0,
            );
            getvvcol(
                wp,
                &raw mut bot_copy,
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

            rex.line = reg_getline(rex.lnum).cast();
            rex.input = rex.line.add(col as usize);

            let cols = win_linetabsize(wp, rex.reg_firstlnum + rex.lnum, rex.line.cast(), col);
            if cols < start || cols > end - colnr_T::from(*p_sel == b'e' as c_char) {
                return false;
            }
        }
        true
    }
}

pub(super) unsafe fn prog_magic_wrong() -> bool {
    unsafe {
        let prog = if REG_MULTI() {
            (*rex.reg_mmatch).regprog
        } else {
            (*rex.reg_match).regprog
        };
        if (*prog).engine == nfa_regengine_ptr() {
            return false;
        }
        let bt_prog = prog.cast::<bt_regprog_T>();
        if UCHARAT((*bt_prog).program.as_ptr()) != REGMAGIC {
            emsg(tr((&raw const e_re_corr).cast()));
            return true;
        }
        false
    }
}

pub(super) unsafe fn cleanup_subexpr() {
    unsafe {
        if rex.need_clear_subexpr == 0 {
            return;
        }
        if REG_MULTI() {
            let uninit_pos = lpos_T { lnum: -1, col: -1 };
            std::slice::from_raw_parts_mut(rex.reg_startpos, NSUBEXP).fill(uninit_pos);
            std::slice::from_raw_parts_mut(rex.reg_endpos, NSUBEXP).fill(uninit_pos);
        } else {
            std::slice::from_raw_parts_mut(rex.reg_startp, NSUBEXP).fill(ptr::null_mut());
            std::slice::from_raw_parts_mut(rex.reg_endp, NSUBEXP).fill(ptr::null_mut());
        }
        rex.need_clear_subexpr = 0;
    }
}

pub(super) unsafe fn cleanup_zsubexpr() {
    unsafe {
        if rex.need_clear_zsubexpr == 0 {
            return;
        }
        if REG_MULTI() {
            let uninit_pos = lpos_T { lnum: -1, col: -1 };
            reg_startzpos = [uninit_pos; NSUBEXP];
            reg_endzpos = [uninit_pos; NSUBEXP];
        } else {
            reg_startzp = [ptr::null_mut(); NSUBEXP];
            reg_endzp = [ptr::null_mut(); NSUBEXP];
        }
        rex.need_clear_zsubexpr = 0;
    }
}

pub(super) unsafe fn reg_nextline() {
    unsafe {
        rex.lnum += 1;
        rex.line = reg_getline(rex.lnum).cast();
        rex.input = rex.line;
        reg_breakcheck();
    }
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
            if rex.line != reg_tofree {
                let mut len = strlen(rex.line.cast()) as c_int;
                if reg_tofree.is_null() || len >= reg_tofreelen as c_int {
                    len += 50;
                    xfree(reg_tofree.cast());
                    reg_tofree = xmalloc(len as usize).cast();
                    reg_tofreelen = len as c_uint;
                }
                strcpy(reg_tofree.cast(), rex.line.cast());
                rex.input = reg_tofree.add(rex.input.offset_from(rex.line) as usize);
                rex.line = reg_tofree;
            }

            let p = reg_getline(clnum);
            debug_assert!(!p.is_null());

            let mut len = if clnum == end_lnum {
                end_col - ccol
            } else {
                reg_getline_len(clnum) - ccol
            };

            if (!rex.reg_ic && cstrncmp(p.add(ccol as usize), rex.input.cast(), &raw mut len) != 0)
                || (rex.reg_ic
                    && mb_strnicmp(p.add(ccol as usize), rex.input.cast(), len as usize) != 0)
            {
                return RA_NOMATCH;
            }
            if !bytelen.is_null() {
                *bytelen += len;
            }
            if clnum == end_lnum {
                break;
            }
            if rex.lnum >= rex.reg_maxline {
                return RA_NOMATCH;
            }

            reg_nextline();
            if !bytelen.is_null() {
                *bytelen = 0;
            }
            clnum += 1;
            ccol = 0;
            if got_int {
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
            rc_did_emsg = true;
            return false;
        }
        true
    }
}

unsafe fn mb_decompose(c: c_int, c1: *mut c_int, c2: *mut c_int, c3: *mut c_int) {
    unsafe {
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
}

pub(super) unsafe fn cstrncmp(s1: *mut c_char, s2: *mut c_char, n: *mut c_int) -> c_int {
    unsafe {
        let mut result = if !rex.reg_ic {
            strncmp(s1, s2, *n as usize)
        } else {
            let mut p = s1;
            let mut n2: c_int = 0;
            let mut n1 = *n;
            while n1 > 0 && *p != NUL as c_char {
                n1 -= utfc_ptr2len(p);
                mb_ptr_adv(&raw mut p);
                n2 += 1;
            }
            p = s2;
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

        if result != 0 && rex.reg_icombine {
            let mut str1: *const c_char = s1;
            let mut str2: *const c_char = s2;
            let mut c1: c_int = 0;
            let mut c2: c_int = 0;
            let mut c11: c_int = 0;
            let mut c12: c_int = 0;
            let mut junk: c_int = 0;

            while (str1.offset_from(s1) as c_int) < *n {
                c1 = mb_ptr2char_adv(&raw mut str1);
                c2 = mb_ptr2char_adv(&raw mut str2);

                if c1 != c2 && (!rex.reg_ic || utf_fold(c1) != utf_fold(c2)) {
                    mb_decompose(c1, &raw mut c11, &raw mut junk, &raw mut junk);
                    mb_decompose(c2, &raw mut c12, &raw mut junk, &raw mut junk);
                    c1 = c11;
                    c2 = c12;
                    if c11 != c12 && (!rex.reg_ic || utf_fold(c11) != utf_fold(c12)) {
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
        if !rex.reg_ic {
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

unsafe fn do_upper(d: *mut c_int, c: c_int) {
    unsafe {
        *d = mb_toupper(c);
    }
}

unsafe fn do_lower(d: *mut c_int, c: c_int) {
    unsafe {
        *d = mb_tolower(c);
    }
}

/// Replace tildes in the pattern by the old pattern.
pub unsafe fn regtilde(source: *mut c_char, magic: c_int, preview: bool) -> *mut c_char {
    unsafe {
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
            if strncmp(p, tilde.as_ptr(), tildelen) == 0 {
                let prefixlen = p.offset_from(newsub) as usize;
                let postfix = p.add(tildelen);
                if newsublen == 0 {
                    newsublen = strlen(newsub);
                }
                newsublen -= tildelen;
                let postfixlen = newsublen - prefixlen;
                let tmpsublen = prefixlen + reg_prev_sublen + postfixlen;

                if tmpsublen > 0 && !reg_prev_sub.is_null() {
                    if tmpsublen > MAXCOL as usize {
                        emsg(tr((&raw const e_resulting_text_too_long).cast()));
                        error = true;
                        break;
                    }

                    let tmpsub = xmalloc(tmpsublen + 1).cast::<c_char>();
                    ptr::copy_nonoverlapping(newsub, tmpsub, prefixlen);
                    ptr::copy_nonoverlapping(reg_prev_sub, tmpsub.add(prefixlen), reg_prev_sublen);
                    strcpy(tmpsub.add(prefixlen + reg_prev_sublen), postfix);

                    if newsub != source {
                        xfree(newsub.cast());
                    }
                    newsub = tmpsub;
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
            if newsub != source {
                xfree(newsub.cast());
            }
            return source;
        }

        if !preview {
            newsublen = p.offset_from(newsub) as usize;
            if newsublen == 0 {
                xfree(reg_prev_sub.cast());
                reg_prev_sub = ptr::null_mut();
            } else {
                xfree(reg_prev_sub.cast());
                reg_prev_sub = xstrnsave(newsub, newsublen);
            }
            reg_prev_sublen = newsublen;
        }

        newsub
    }
}

unsafe fn fill_submatch_list(
    _argc: c_int,
    argv: *mut typval_T,
    argskip: c_int,
    fp: *mut ufunc_T,
) -> c_int {
    unsafe {
        let listarg = argv.add(argskip as usize);
        if (*fp).uf_varargs == 0 && ((*fp).uf_args.len() as c_int) <= argskip {
            return argskip;
        }

        tv_list_init_static10((*listarg).vval.v_list.cast::<staticList10_T>());

        let mut li = tv_list_first((*listarg).vval.v_list);
        for i in 0..10 {
            let mut s = (*rsm.sm_match).startp[i];
            if s.is_null() || (*rsm.sm_match).endp[i].is_null() {
                s = ptr::null_mut();
            } else {
                s = xstrnsave(s, (*rsm.sm_match).endp[i].offset_from(s) as usize);
            }
            (*li).li_tv.v_type = VarType::VAR_STRING;
            (*li).li_tv.vval.v_string = s;
            li = (*li).li_next;
        }
        argskip + 1
    }
}

unsafe fn clear_submatch_list(sl: *mut staticList10_T) {
    unsafe {
        let mut li = (*sl).sl_list.lv_first;
        while !li.is_null() {
            xfree((*li).li_tv.vval.v_string.cast());
            li = (*li).li_next;
        }
    }
}

pub unsafe fn vim_regsub(
    rmp: *mut regmatch_T,
    source: *mut c_char,
    expr: *mut typval_T,
    dest: *mut c_char,
    destlen: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut rex_save = regexec_T::new();
        let rex_in_use_save = rex_in_use;

        if rex_in_use {
            rex_save = rex;
        }
        rex_in_use = true;

        rex.reg_match = rmp;
        rex.reg_mmatch = ptr::null_mut();
        rex.reg_maxline = 0;
        rex.reg_buf = curbuf;
        rex.reg_line_lbr = true;
        let result = vim_regsub_both(source, expr, dest, destlen, flags);

        rex_in_use = rex_in_use_save;
        if rex_in_use {
            rex = rex_save;
        }

        result
    }
}

pub unsafe fn vim_regsub_multi(
    rmp: *mut regmmatch_T,
    lnum: linenr_T,
    source: *mut c_char,
    dest: *mut c_char,
    destlen: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut rex_save = regexec_T::new();
        let rex_in_use_save = rex_in_use;

        if rex_in_use {
            rex_save = rex;
        }
        rex_in_use = true;

        rex.reg_match = ptr::null_mut();
        rex.reg_mmatch = rmp;
        rex.reg_buf = curbuf;
        rex.reg_firstlnum = lnum;
        rex.reg_maxline = (*curbuf).b_ml.ml_line_count - lnum;
        rex.reg_line_lbr = false;
        let result = vim_regsub_both(source, ptr::null_mut(), dest, destlen, flags);

        rex_in_use = rex_in_use_save;
        if rex_in_use {
            rex = rex_save;
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
        let mut func_all: fptr_T = None;
        let mut func_one: fptr_T = None;
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
        if regsub_nesting == MAX_REGSUB_NESTING {
            emsg(tr(e_substitute_nesting_too_deep.as_ptr()));
            return 0;
        }
        let nested = regsub_nesting;
        let eval_slot = (&raw mut eval_result).cast::<*mut c_char>().add(nested);
        let mut src = source;
        let mut dst = dest;

        if !expr.is_null() || (*source == b'\\' as c_char && *source.add(1) == b'=' as c_char) {
            if copy {
                if !(*eval_slot).is_null() {
                    let eval_len = strlen(*eval_slot);
                    if eval_len < destlen as usize {
                        strcpy(dest, *eval_slot);
                        dst = dst.add(eval_len);
                        xfree((*eval_slot).cast());
                        *eval_slot = ptr::null_mut();
                    }
                }
            } else {
                let prev_can_f_submatch = can_f_submatch;
                let mut rsm_save = regsubmatch_T::new();

                xfree((*eval_slot).cast());
                *eval_slot = ptr::null_mut();

                if can_f_submatch {
                    rsm_save = rsm;
                }
                can_f_submatch = true;
                rsm.sm_match = rex.reg_match;
                rsm.sm_mmatch = rex.reg_mmatch;
                rsm.sm_firstlnum = rex.reg_firstlnum;
                rsm.sm_maxline = rex.reg_maxline;
                rsm.sm_line_lbr = c_int::from(rex.reg_line_lbr);

                regsub_nesting += 1;

                if !expr.is_null() {
                    let mut argv: [typval_T; 2] = [typval_T::new(); 2];
                    let mut rettv: typval_T = typval_T::new();
                    let mut matchList = staticList10_T::new();
                    rettv.v_type = VarType::VAR_STRING;
                    rettv.vval.v_string = ptr::null_mut();
                    argv[0].v_type = VarType::VAR_LIST;
                    argv[0].vval.v_list = &raw mut matchList.sl_list;
                    let mut funcexe = funcexe_T {
                        fe_argv_func: Some(fill_submatch_list),
                        fe_firstline: 0,
                        fe_lastline: 0,
                        fe_doesrange: ptr::null_mut(),
                        fe_evaluate: true,
                        fe_partial: ptr::null_mut(),
                        fe_selfdict: ptr::null_mut(),
                        fe_basetv: ptr::null_mut(),
                        fe_found_var: false,
                    };
                    if (*expr).v_type == VarType::VAR_FUNC {
                        let s = (*expr).vval.v_string;
                        call_func(
                            s,
                            -1,
                            &raw mut rettv,
                            1,
                            argv.as_mut_ptr(),
                            &raw mut funcexe,
                        );
                    } else if (*expr).v_type == VarType::VAR_PARTIAL {
                        let partial = (*expr).vval.v_partial;
                        let s = partial_name(partial);
                        funcexe.fe_partial = partial;
                        call_func(
                            s,
                            -1,
                            &raw mut rettv,
                            1,
                            argv.as_mut_ptr(),
                            &raw mut funcexe,
                        );
                    }
                    if tv_list_len(&raw const matchList.sl_list) > 0 {
                        clear_submatch_list(&raw mut matchList);
                    }
                    if rettv.v_type == VarType::VAR_UNKNOWN {
                        *eval_slot = ptr::null_mut();
                    } else {
                        let mut buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
                        *eval_slot =
                            tv_get_string_buf_chk(&raw const rettv, buf.as_mut_ptr()).cast_mut();
                        if !(*eval_slot).is_null() {
                            *eval_slot = xstrdup(*eval_slot);
                        }
                    }
                    tv_clear(&raw mut rettv);
                } else {
                    *eval_slot = eval_to_string(source.add(2), true, false);
                }
                regsub_nesting -= 1;

                if !(*eval_slot).is_null() {
                    let mut had_backslash = false;
                    let mut s = *eval_slot;
                    while *s != NUL as c_char {
                        if *s == NL as c_char && rsm.sm_line_lbr == 0 {
                            *s = CAR as c_char;
                        } else if *s == b'\\' as c_char && *s.add(1) != NUL as c_char {
                            s = s.add(1);
                            if *s == NL as c_char && rsm.sm_line_lbr == 0 {
                                *s = CAR as c_char;
                            }
                            had_backslash = true;
                        }
                        mb_ptr_adv(&raw mut s);
                    }
                    if had_backslash && (flags & REGSUB_BACKSLASH) != 0 {
                        let escaped = vim_strsave_escaped(*eval_slot, c"\\".as_ptr());
                        xfree((*eval_slot).cast());
                        *eval_slot = escaped;
                    }
                    dst = dst.add(strlen(*eval_slot));
                }

                can_f_submatch = prev_can_f_submatch;
                if can_f_submatch {
                    rsm = rsm_save;
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
                                func_one = Some(do_upper);
                                continue;
                            }
                            b'U' => {
                                func_all = Some(do_upper);
                                continue;
                            }
                            b'l' => {
                                func_one = Some(do_lower);
                                continue;
                            }
                            b'L' => {
                                func_all = Some(do_lower);
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

                    let mut cc: c_int = 0;
                    if let Some(f) = func_one {
                        f(&raw mut cc, c);
                        func_one = None;
                    } else if let Some(f) = func_all {
                        f(&raw mut cc, c);
                    } else {
                        cc = c;
                    }

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
                        clnum = (*rex.reg_mmatch).startpos[no as usize].lnum;
                        if clnum < 0 || (*rex.reg_mmatch).endpos[no as usize].lnum < 0 {
                            s = ptr::null_mut();
                        } else {
                            s = reg_getline(clnum)
                                .add((*rex.reg_mmatch).startpos[no as usize].col as usize);
                            if (*rex.reg_mmatch).endpos[no as usize].lnum == clnum {
                                len = (*rex.reg_mmatch).endpos[no as usize].col
                                    - (*rex.reg_mmatch).startpos[no as usize].col;
                            } else {
                                len = reg_getline_len(clnum)
                                    - (*rex.reg_mmatch).startpos[no as usize].col;
                            }
                        }
                    } else {
                        s = (*rex.reg_match).startp[no as usize];
                        if (*rex.reg_match).endp[no as usize].is_null() {
                            s = ptr::null_mut();
                        } else {
                            len = (*rex.reg_match).endp[no as usize].offset_from(s) as c_int;
                        }
                    }
                    if !s.is_null() {
                        loop {
                            if len == 0 {
                                if REG_MULTI() {
                                    if (*rex.reg_mmatch).endpos[no as usize].lnum == clnum {
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
                                    if (*rex.reg_mmatch).endpos[no as usize].lnum == clnum {
                                        len = (*rex.reg_mmatch).endpos[no as usize].col;
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
                                    let mut cc: c_int = 0;
                                    if let Some(f) = func_one {
                                        f(&raw mut cc, c);
                                        func_one = None;
                                    } else if let Some(f) = func_all {
                                        f(&raw mut cc, c);
                                    } else {
                                        cc = c;
                                    }

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

unsafe fn reg_getline_submatch(lnum: linenr_T) -> *mut c_char {
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

unsafe fn reg_getline_submatch_len(lnum: linenr_T) -> colnr_T {
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

pub unsafe fn reg_submatch(no: c_int) -> *mut c_char {
    unsafe {
        let mut retval: *mut c_char = ptr::null_mut();
        if !can_f_submatch || no < 0 {
            return ptr::null_mut();
        }
        let no = no as usize;

        if rsm.sm_match.is_null() {
            for round in 1..=2 {
                let mut lnum = (*rsm.sm_mmatch).startpos[no].lnum;
                if lnum < 0 || (*rsm.sm_mmatch).endpos[no].lnum < 0 {
                    return ptr::null_mut();
                }

                let mut s = reg_getline_submatch(lnum);
                if s.is_null() {
                    break;
                }
                s = s.add((*rsm.sm_mmatch).startpos[no].col as usize);
                let mut len: isize;
                if (*rsm.sm_mmatch).endpos[no].lnum == lnum {
                    len = ((*rsm.sm_mmatch).endpos[no].col - (*rsm.sm_mmatch).startpos[no].col)
                        as isize;
                    if round == 2 {
                        xmemcpyz(retval.cast(), s.cast(), len as usize);
                    }
                    len += 1;
                } else {
                    len = (reg_getline_submatch_len(lnum) - (*rsm.sm_mmatch).startpos[no].col)
                        as isize;
                    if round == 2 {
                        strcpy(retval, s);
                        *retval.add(len as usize) = b'\n' as c_char;
                    }
                    len += 1;
                    lnum += 1;
                    while lnum < (*rsm.sm_mmatch).endpos[no].lnum {
                        s = reg_getline_submatch(lnum);
                        if round == 2 {
                            strcpy(retval.add(len as usize), s);
                        }
                        len += reg_getline_submatch_len(lnum) as isize;
                        if round == 2 {
                            *retval.add(len as usize) = b'\n' as c_char;
                        }
                        len += 1;
                        lnum += 1;
                    }
                    if round == 2 {
                        strncpy(
                            retval.add(len as usize),
                            reg_getline_submatch(lnum),
                            (*rsm.sm_mmatch).endpos[no].col as usize,
                        );
                    }
                    len += (*rsm.sm_mmatch).endpos[no].col as isize;
                    if round == 2 {
                        *retval.add(len as usize) = NUL as c_char;
                    }
                    len += 1;
                }

                if retval.is_null() {
                    retval = xmalloc(len as usize).cast();
                }
            }
        } else {
            let s = (*rsm.sm_match).startp[no];
            if s.is_null() || (*rsm.sm_match).endp[no].is_null() {
                retval = ptr::null_mut();
            } else {
                retval = xstrnsave(s, (*rsm.sm_match).endp[no].offset_from(s) as usize);
            }
        }

        retval
    }
}

pub unsafe fn reg_submatch_list(no: c_int) -> *mut list_T {
    unsafe {
        if !can_f_submatch || no < 0 {
            return ptr::null_mut();
        }
        let no = no as usize;
        let list: *mut list_T;

        if rsm.sm_match.is_null() {
            let slnum = (*rsm.sm_mmatch).startpos[no].lnum;
            let elnum = (*rsm.sm_mmatch).endpos[no].lnum;
            if slnum < 0 || elnum < 0 {
                return ptr::null_mut();
            }

            let scol = (*rsm.sm_mmatch).startpos[no].col;
            let ecol = (*rsm.sm_mmatch).endpos[no].col;

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
            let s = (*rsm.sm_match).startp[no];
            if s.is_null() || (*rsm.sm_match).endp[no].is_null() {
                return ptr::null_mut();
            }
            list = tv_list_alloc(1);
            tv_list_append_string(list, s, (*rsm.sm_match).endp[no].offset_from(s));
        }

        tv_list_ref(list);
        list
    }
}

pub(super) unsafe fn init_regexec_multi(
    rmp: *mut regmmatch_T,
    win: *mut win_T,
    buf: *mut buf_T,
    lnum: linenr_T,
) {
    unsafe {
        rex.reg_match = ptr::null_mut();
        rex.reg_mmatch = rmp;
        rex.reg_buf = buf;
        rex.reg_win = win;
        rex.reg_firstlnum = lnum;
        rex.reg_maxline = (*rex.reg_buf).b_ml.ml_line_count - lnum;
        rex.reg_line_lbr = false;
        rex.reg_ic = (*rmp).rmm_ic != 0;
        rex.reg_icombine = false;
        rex.reg_nobreak = ((*(*rmp).regprog).re_flags & (RE_NOBREAK as c_uint)) != 0;
        rex.reg_maxcol = (*rmp).rmm_maxcol;
    }
}

pub(super) unsafe fn get_char_class_bits(
    start: *const u8,
    end: *const u8,
    newl: *mut bool,
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
            reg_magic = MAGIC_ON;
        } else {
            reg_magic = MAGIC_OFF;
        }
        reg_string = re_flags & RE_STRING;
        reg_strict = re_flags & RE_STRICT;
        get_cpo_flags();

        num_complex_braces = 0;
        regnpar = 1;
        had_endbrace = [0; NSUBEXP];
        regnzpar = 1;
        re_has_z = 0;
        regsize = 0;
        reg_toolong = 0;
        bt_reg_parse_depth = 0;
        regflags = 0;
        had_eol = 0;
    }
}

pub(super) unsafe fn seen_endbrace(refnum: c_int) -> bool {
    unsafe {
        if *(&raw const had_endbrace).cast::<u8>().add(refnum as usize) == 0 {
            let mut p = regparse.cast::<u8>();
            while *p != NUL {
                if *p == b'@' && *p.add(1) == b'<' && (*p.add(2) == b'!' || *p.add(2) == b'=') {
                    break;
                }
                p = p.add(1);
            }
            if *p == NUL {
                emsg(tr(c"E65: Illegal back reference".as_ptr()));
                rc_did_emsg = true;
                return false;
            }
        }
        true
    }
}

pub(super) unsafe fn coll_get_char() -> c_int {
    unsafe {
        let mut nr: i64 = -1;
        let ch = *regparse as u8;
        regparse = regparse.add(1);
        match ch {
            b'd' => nr = getdecchrs(),
            b'o' => nr = getoctchrs(),
            b'x' => nr = gethexchrs(2),
            b'u' => nr = gethexchrs(4),
            b'U' => nr = gethexchrs(8),
            _ => {}
        }
        if nr < 0 {
            regparse = regparse.sub(1);
            nr = i64::from(b'\\');
        }
        if nr > i64::from(c_int::MAX) {
            nr = i64::from(c_int::MAX);
        }
        nr as c_int
    }
}

pub unsafe fn vim_regcomp_had_eol() -> c_int {
    unsafe { had_eol }
}

/// Compile a regular expression into internal code.
pub unsafe fn vim_regcomp(expr_arg: *const c_char, re_flags: c_int) -> *mut regprog_T {
    unsafe {
        let mut prog: *mut regprog_T;
        let mut expr = expr_arg;

        regexp_engine = p_re as c_int;

        if strncmp(expr, c"\\%#=".as_ptr(), 4) == 0 {
            let newengine = (*expr.add(4) as u8).wrapping_sub(b'0') as c_int;
            if newengine == AUTOMATIC_ENGINE
                || newengine == BACKTRACKING_ENGINE
                || newengine == NFA_ENGINE
            {
                regexp_engine = newengine;
                expr = expr.add(5);
            } else {
                emsg(tr(
                    c"E864: \\%#= can only be followed by 0, 1, or 2. The automatic engine will be used "
                        .as_ptr(),
                ));
                regexp_engine = AUTOMATIC_ENGINE;
            }
        }

        rex.reg_buf = curbuf;

        let called_emsg_before = called_emsg;
        if regexp_engine != BACKTRACKING_ENGINE {
            prog = (NFA_REGENGINE.regcomp)(
                expr.cast_mut().cast(),
                re_flags
                    + if regexp_engine == AUTOMATIC_ENGINE {
                        RE_AUTO
                    } else {
                        0
                    },
            );
        } else {
            prog = (BT_REGENGINE.regcomp)(expr.cast_mut().cast(), re_flags);
        }

        if prog.is_null() && regexp_engine == AUTOMATIC_ENGINE && called_emsg == called_emsg_before
        {
            regexp_engine = BACKTRACKING_ENGINE;
            report_re_switch(expr);
            prog = (BT_REGENGINE.regcomp)(expr.cast_mut().cast(), re_flags);
        }

        if !prog.is_null() {
            (*prog).re_engine = regexp_engine as c_uint;
            (*prog).re_flags = re_flags as c_uint;
        }

        prog
    }
}

/// Free a compiled regexp program, returned by `vim_regcomp()`.
pub unsafe fn vim_regfree(prog: *mut regprog_T) {
    unsafe {
        if !prog.is_null() {
            ((*(*prog).engine).regfree)(prog);
        }
    }
}

pub unsafe fn free_regexp_stuff() {
    unsafe {
        let rs = &raw mut regstack;
        *rs = Vec::new();
        let bp = &raw mut backpos;
        *bp = Vec::new();
        xfree(reg_tofree.cast());
        xfree(reg_prev_sub.cast());
        let pv = &raw mut post_vec;
        *pv = Vec::new();
        let ns = &raw mut nfa_stack;
        *ns = Vec::new();
    }
}

unsafe fn report_re_switch(pat: *const c_char) {
    unsafe {
        if p_verbose > 0 {
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
    rmp: *mut regmatch_T,
    line: *const c_char,
    col: colnr_T,
    nl: bool,
) -> bool {
    unsafe {
        let mut rex_save = regexec_T::new();
        let rex_in_use_save = rex_in_use;

        if (*(*rmp).regprog).re_in_use {
            emsg(tr(e_recursive.as_ptr()));
            return false;
        }
        (*(*rmp).regprog).re_in_use = true;

        if rex_in_use {
            rex_save = rex;
        }
        rex_in_use = true;

        rex.reg_startp = ptr::null_mut();
        rex.reg_endp = ptr::null_mut();
        rex.reg_startpos = ptr::null_mut();
        rex.reg_endpos = ptr::null_mut();

        let mut result =
            ((*(*(*rmp).regprog).engine).regexec_nl)(rmp, line.cast_mut().cast(), col, nl);
        (*(*rmp).regprog).re_in_use = false;

        if (*(*rmp).regprog).re_engine == AUTOMATIC_ENGINE as c_uint && result == NFA_TOO_EXPENSIVE
        {
            let save_p_re = p_re;
            let re_flags = (*(*rmp).regprog).re_flags as c_int;
            let pat = xstrdup((*((*rmp).regprog.cast::<nfa_regprog_T>())).pattern);

            p_re = OptInt::from(BACKTRACKING_ENGINE);
            let prev_prog = (*rmp).regprog;
            report_re_switch(pat);
            (*rmp).regprog = vim_regcomp(pat, re_flags);
            if (*rmp).regprog.is_null() {
                (*rmp).regprog = prev_prog;
            } else {
                vim_regfree(prev_prog);
                (*(*rmp).regprog).re_in_use = true;
                result =
                    ((*(*(*rmp).regprog).engine).regexec_nl)(rmp, line.cast_mut().cast(), col, nl);
                (*(*rmp).regprog).re_in_use = false;
            }

            xfree(pat.cast());
            p_re = save_p_re;
        }

        rex_in_use = rex_in_use_save;
        if rex_in_use {
            rex = rex_save;
        }

        result > 0
    }
}

pub unsafe fn vim_regexec_prog(
    prog: *mut *mut regprog_T,
    ignore_case: bool,
    line: *const c_char,
    col: colnr_T,
) -> bool {
    unsafe {
        let mut regmatch = regmatch_T {
            regprog: *prog,
            startp: [ptr::null_mut(); NSUBEXP],
            endp: [ptr::null_mut(); NSUBEXP],
            rm_matchcol: 0,
            rm_ic: ignore_case,
        };
        let r = vim_regexec_string(&raw mut regmatch, line, col, false);
        *prog = regmatch.regprog;
        r
    }
}

pub unsafe fn vim_regexec(rmp: *mut regmatch_T, line: *const c_char, col: colnr_T) -> bool {
    unsafe { vim_regexec_string(rmp, line, col, false) }
}

pub unsafe fn vim_regexec_nl(rmp: *mut regmatch_T, line: *const c_char, col: colnr_T) -> bool {
    unsafe { vim_regexec_string(rmp, line, col, true) }
}

pub unsafe fn vim_regexec_multi(
    rmp: *mut regmmatch_T,
    win: *mut win_T,
    buf: *mut buf_T,
    lnum: linenr_T,
    col: colnr_T,
    tm: *mut proftime_T,
    timed_out: *mut c_int,
) -> c_int {
    unsafe {
        let mut rex_save = regexec_T::new();
        let rex_in_use_save = rex_in_use;

        if (*(*rmp).regprog).re_in_use {
            emsg(tr(e_recursive.as_ptr()));
            return 0;
        }
        (*(*rmp).regprog).re_in_use = true;

        if rex_in_use {
            rex_save = rex;
        }
        rex_in_use = true;

        let mut result =
            ((*(*(*rmp).regprog).engine).regexec_multi)(rmp, win, buf, lnum, col, tm, timed_out);
        (*(*rmp).regprog).re_in_use = false;

        if (*(*rmp).regprog).re_engine == AUTOMATIC_ENGINE as c_uint && result == NFA_TOO_EXPENSIVE
        {
            let save_p_re = p_re;
            let re_flags = (*(*rmp).regprog).re_flags as c_int;
            let pat = xstrdup((*((*rmp).regprog.cast::<nfa_regprog_T>())).pattern);

            p_re = OptInt::from(BACKTRACKING_ENGINE);
            let prev_prog = (*rmp).regprog;

            report_re_switch(pat);
            reg_do_extmatch = REX_ALL;
            (*rmp).regprog = vim_regcomp(pat, re_flags);
            reg_do_extmatch = 0;

            if (*rmp).regprog.is_null() {
                (*rmp).regprog = prev_prog;
            } else {
                vim_regfree(prev_prog);
                (*(*rmp).regprog).re_in_use = true;
                result = ((*(*(*rmp).regprog).engine).regexec_multi)(
                    rmp, win, buf, lnum, col, tm, timed_out,
                );
                (*(*rmp).regprog).re_in_use = false;
            }

            xfree(pat.cast());
            p_re = save_p_re;
        }

        rex_in_use = rex_in_use_save;
        if rex_in_use {
            rex = rex_save;
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
                get_char_class_bits(digits.as_ptr(), digits.as_ptr().add(3), &raw mut newl),
                CLASS_o9
            );
            assert!(!newl);

            let hex_nl = b"^0-9a-fA-F\\n]";
            assert_eq!(
                get_char_class_bits(hex_nl.as_ptr(), hex_nl.as_ptr().add(12), &raw mut newl),
                CLASS_not | CLASS_o9 | CLASS_af | CLASS_AF
            );
            assert!(newl);
        }
    }
}
