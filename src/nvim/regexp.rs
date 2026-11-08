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
    GraphemeState, mb_get_class_tab, mb_islower, mb_isupper, mb_ptr2char_adv, mb_strnicmp,
    mb_tolower, mb_toupper, utf_char2bytes, utf_char2len, utf_composinglike, utf_fold,
    utf_iscomposing_legacy, utf_ptr2char, utf_ptr2len, utf_strnicmp, utfc_ptr2len,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::message::{emsg, iemsg, internal_error, msg_puts, semsg, verbose_enter, verbose_leave};
use crate::option_vars::*;
use crate::os::input::fast_breakcheck;
use crate::plines::{getvvcol, win_linetabsize};
use crate::profile::{Proftime, profile_passed_limit};
pub(super) use crate::strings::tr;
use crate::window::WinInfoInfo;
use std::ffi::CStr;
use std::ptr;

#[path = "regexp_bt.rs"]
mod bt;
#[path = "regexp_nfa.rs"]
mod nfa;

use self::bt::*;
use self::nfa::*;

use crate::option::OptInt;
pub type RegMagic = i32;

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RegAction {
    Fail,
    Cont,
    Break,
    Match,
    NoMatch,
}

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
pub(super) const CLASSCHARS: &CStr = c".iIkKfFpPsSdDxXoOwWhHaAlLuU";

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
    #[inline(always)]
    pub(super) fn bt_mut(&mut self) -> &mut BtRegprog {
        match &mut self.engine {
            RegProgEngine::Bt(bt) => bt,
            RegProgEngine::Nfa(_) => unreachable!(),
        }
    }

    #[inline(always)]
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
    pub regprog: crate::globals::GraphPtr<Regprog>,
    pub startpos: [Lpos; NSUBEXP],
    pub endpos: [Lpos; NSUBEXP],
    pub rmm_matchcol: Colnr,
    pub rmm_ic: bool,
    pub rmm_maxcol: Colnr,
}

impl Regmmatch {
    pub const fn new() -> Self {
        Self {
            regprog: crate::globals::GraphPtr::NULL,
            startpos: [Lpos::new(); NSUBEXP],
            endpos: [Lpos::new(); NSUBEXP],
            rmm_matchcol: 0,
            rmm_ic: false,
            rmm_maxcol: 0,
        }
    }

    #[inline(always)]
    pub fn set_prog(&mut self, prog: Option<Box<Regprog>>) {
        self.regprog = prog.map_or(crate::globals::GraphPtr::NULL, |b| {
            crate::globals::GraphPtr::new(Box::into_raw(b))
        });
    }

    #[inline(always)]
    pub fn borrow_prog(&mut self, prog: Option<&Regprog>) {
        self.regprog = prog.map_or(crate::globals::GraphPtr::NULL, |p| {
            crate::globals::GraphPtr::new(ptr::from_ref(p).cast_mut())
        });
    }

    #[inline(always)]
    pub fn free_prog(&mut self) {
        if !self.regprog.is_null() {
            crate::globals::GraphPtr::of(self.regprog.as_ptr()).drop_box();
            self.regprog = crate::globals::GraphPtr::NULL;
        }
    }
}

impl Default for Regmmatch {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

const REG_LINE_SLOTS: usize = 64;

#[derive(Clone, Copy, Default, Debug)]
pub struct RegLineSlice {
    slot: u16,
    is_some: bool,
    off: u32,
    addr: usize,
}

impl PartialEq for RegLineSlice {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (self.is_some, other.is_some) {
            (false, false) => true,
            (true, true) => self.addr == other.addr,
            _ => false,
        }
    }
}

impl Eq for RegLineSlice {}

pub const REG_LINE_NULL: RegLineSlice = RegLineSlice {
    slot: 0,
    is_some: false,
    off: 0,
    addr: 0,
};

impl RegLineSlice {
    #[inline(always)]
    pub const fn is_null(self) -> bool {
        !self.is_some
    }

    #[inline(always)]
    pub const fn addr(self) -> usize {
        if self.is_some { self.addr } else { 0 }
    }

    #[inline(always)]
    fn raw_slot_bytes(self) -> &'static [u8] {
        if !self.is_some {
            return &[];
        }
        let b = &re_st().borrow().line_slots[self.slot as usize];
        let start = (self.off as usize).min(b.len());
        &b[start..]
    }

    #[inline(always)]
    pub fn first_byte(self) -> u8 {
        self.raw_slot_bytes().first().copied().unwrap_or(0)
    }

    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.first_byte() == 0
    }

    #[inline(always)]
    pub fn as_bytes(self) -> &'static [u8] {
        if !self.is_some {
            return &[];
        }
        let b = &re_st().borrow().line_slots[self.slot as usize];
        let end = b.len().saturating_sub(1);
        let start = (self.off as usize).min(end);
        &b[start..end]
    }

    #[inline(always)]
    pub fn unwrap_or(self, default: &'static [u8]) -> &'static [u8] {
        if self.is_some {
            self.raw_slot_bytes()
        } else {
            default
        }
    }

    #[inline(always)]
    pub fn as_cstr(self) -> &'static CStr {
        let b = self.raw_slot_bytes();
        CStr::from_bytes_until_nul(b).unwrap_or(c"")
    }

    #[inline(always)]
    pub fn slice(self, len: usize) -> &'static [u8] {
        let b = self.raw_slot_bytes();
        &b[..len.min(b.len())]
    }

    #[inline(always)]
    pub fn wrapping_add(self, n: usize) -> Self {
        if !self.is_some {
            return REG_LINE_NULL;
        }
        let b = &re_st().borrow().line_slots[self.slot as usize];
        let max_off = b.len().saturating_sub(1);
        let cur_off = self.off as usize;
        let new_off = (cur_off + n).min(max_off);
        let delta = new_off.saturating_sub(cur_off);
        Self {
            slot: self.slot,
            is_some: true,
            off: new_off as u32,
            addr: self.addr.wrapping_add(delta),
        }
    }

    #[inline(always)]
    pub const fn offset_from(self, base: Self) -> usize {
        self.addr().saturating_sub(base.addr())
    }

    #[inline(always)]
    pub fn offset_in(self, base: Self, d: isize) -> Self {
        if d >= 0 {
            self.wrapping_add(d as usize)
        } else {
            let off = self.offset_from(base).saturating_sub((-d) as usize);
            base.wrapping_add(off)
        }
    }

    #[inline(always)]
    pub fn span_to(self, end: Self) -> &'static [u8] {
        self.slice(end.offset_from(self))
    }

    #[inline(always)]
    pub const fn before(self, other: Self) -> bool {
        self.is_some && other.is_some && self.addr < other.addr
    }
}

#[inline(always)]
pub fn reg_line_from_cstr(s: &std::ffi::CStr) -> RegLineSlice {
    let bytes = s.to_bytes_with_nul();
    let addr = s.as_ptr() as usize;
    let mut st = re_st();
    let cur = st.cur_line_slot;
    if st.line_slot_addrs[cur] != addr || st.line_slots[cur].as_slice() != bytes {
        let next = (cur + 1) % REG_LINE_SLOTS;
        st.cur_line_slot = next;
        st.line_slot_addrs[next] = addr;
        st.line_slots[next].clear();
        st.line_slots[next].extend_from_slice(bytes);
    }
    RegLineSlice {
        slot: st.cur_line_slot as u16,
        is_some: true,
        off: 0,
        addr,
    }
}

#[inline(always)]
pub fn reg_line_from_opt_cstr(s: Option<&std::ffi::CStr>) -> RegLineSlice {
    match s {
        Some(s) => reg_line_from_cstr(s),
        None => REG_LINE_NULL,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Regmatch {
    pub regprog: crate::globals::GraphPtr<Regprog>,
    pub startp: [RegLineSlice; NSUBEXP],
    pub endp: [RegLineSlice; NSUBEXP],
    pub rm_matchcol: Colnr,
    pub rm_ic: bool,
}

impl Regmatch {
    pub const fn new() -> Self {
        Self {
            regprog: crate::globals::GraphPtr::NULL,
            startp: [REG_LINE_NULL; NSUBEXP],
            endp: [REG_LINE_NULL; NSUBEXP],
            rm_matchcol: 0,
            rm_ic: false,
        }
    }

    #[inline(always)]
    pub fn span_len(&self, idx: usize) -> usize {
        self.endp[idx].offset_from(self.startp[idx])
    }

    #[inline(always)]
    pub fn span_bytes(&self, idx: usize) -> &[u8] {
        self.startp[idx].span_to(self.endp[idx])
    }

    #[inline(always)]
    pub fn start_offset(&self, idx: usize, base: &std::ffi::CStr) -> usize {
        self.startp[idx]
            .addr()
            .saturating_sub(base.as_ptr() as usize)
    }

    #[inline(always)]
    pub fn end_offset(&self, idx: usize, base: &std::ffi::CStr) -> usize {
        self.endp[idx].addr().saturating_sub(base.as_ptr() as usize)
    }

    #[inline(always)]
    pub fn set_prog(&mut self, prog: Option<Box<Regprog>>) {
        self.regprog = prog.map_or(crate::globals::GraphPtr::NULL, |b| {
            crate::globals::GraphPtr::new(Box::into_raw(b))
        });
    }

    #[inline(always)]
    pub fn borrow_prog(&mut self, prog: Option<&Regprog>) {
        self.regprog = prog.map_or(crate::globals::GraphPtr::NULL, |p| {
            crate::globals::GraphPtr::new(ptr::from_ref(p).cast_mut())
        });
    }

    #[inline(always)]
    pub fn free_prog(&mut self) {
        if !self.regprog.is_null() {
            crate::globals::GraphPtr::of(self.regprog.as_ptr()).drop_box();
            self.regprog = crate::globals::GraphPtr::NULL;
        }
    }
}

impl Default for Regmatch {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Default)]
pub struct RegExtmatch {
    pub matches: [Option<std::ffi::CString>; NSUBEXP],
}

#[inline(always)]
pub(super) fn cstring_from_bytes(bytes: &[u8]) -> Option<std::ffi::CString> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
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
    #[inline(always)]
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
    pub has_zend: bool,
    pub has_backref: bool,
    pub reghasz: i32,
    pub pattern: std::ffi::CString,
    pub nsubexp: i32,
    pub nstate: i32,
    pub listbuf: [Vec<NfaThread>; 2],
    pub state: Vec<NfaState>,
}

#[derive(Clone, Copy)]
pub(super) struct RegPos {
    pub ptr: RegLineSlice,
    pub pos: Lpos,
}

impl RegPos {
    pub(super) const fn new() -> Self {
        Self {
            ptr: REG_LINE_NULL,
            pos: Lpos::new(),
        }
    }

    #[inline(always)]
    pub(super) const fn from_pos(pos: Lpos) -> Self {
        Self {
            ptr: REG_LINE_NULL,
            pos,
        }
    }

    #[inline(always)]
    pub(super) const fn pos(self) -> Lpos {
        self.pos
    }

    #[inline(always)]
    pub(super) const fn pos_mut(&mut self) -> &mut Lpos {
        &mut self.pos
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

    #[inline(always)]
    pub(super) fn sesave(&self) -> &SaveSe {
        match self {
            Self::Se(s) => s,
            Self::Reg(_) => unreachable!(),
        }
    }

    #[inline(always)]
    pub(super) fn sesave_mut(&mut self) -> &mut SaveSe {
        if !matches!(*self, Self::Se(_)) {
            *self = Self::Se(SaveSe::new());
        }
        match self {
            Self::Se(s) => s,
            Self::Reg(_) => unreachable!(),
        }
    }

    #[inline(always)]
    pub(super) fn regsave(&self) -> &Regsave {
        match self {
            Self::Reg(r) => r,
            Self::Se(_) => unreachable!(),
        }
    }

    #[inline(always)]
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
    pub rs_scan: Option<usize>,
    pub rs_un: RegItemSave,
}

impl Regitem {
    pub(super) const fn new() -> Self {
        Self {
            rs_state: 0,
            rs_no: 0,
            rs_scan: None,
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
    pub save_need_clear_subexpr: bool,
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
    pub start: RegLineSlice,
    pub end: RegLineSlice,
}

impl LinePos {
    pub(super) const fn new() -> Self {
        Self {
            start: REG_LINE_NULL,
            end: REG_LINE_NULL,
        }
    }
}

impl Default for LinePos {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub(super) struct RegSubList {
    multi_arr: [MultiPos; NSUBEXP],
    line_arr: [LinePos; NSUBEXP],
}

impl RegSubList {
    pub(super) const fn new() -> Self {
        Self {
            multi_arr: [MultiPos::new(); NSUBEXP],
            line_arr: [LinePos::new(); NSUBEXP],
        }
    }

    #[inline(always)]
    pub(super) fn multi(&self) -> &[MultiPos; NSUBEXP] {
        &self.multi_arr
    }

    #[inline(always)]
    pub(super) fn multi_mut(&mut self) -> &mut [MultiPos; NSUBEXP] {
        &mut self.multi_arr
    }

    #[inline(always)]
    pub(super) fn line(&self) -> &[LinePos; NSUBEXP] {
        &self.line_arr
    }

    #[inline(always)]
    pub(super) fn line_mut(&mut self) -> &mut [LinePos; NSUBEXP] {
        &mut self.line_arr
    }

    #[inline(always)]
    pub(super) fn copy_first(&mut self, other: &Self) {
        self.multi_arr[0] = other.multi_arr[0];
        self.line_arr[0] = other.line_arr[0];
    }

    #[inline(always)]
    pub(super) fn copy_n(&mut self, other: &Self, n: usize) {
        let n = n.min(NSUBEXP);
        self.multi_arr[..n].copy_from_slice(&other.multi_arr[..n]);
        self.line_arr[..n].copy_from_slice(&other.line_arr[..n]);
    }

    #[inline(always)]
    pub(super) fn copy_range_from_1(&mut self, other: &Self, n: usize) {
        let n = n.min(NSUBEXP);
        self.multi_arr[1..n].copy_from_slice(&other.multi_arr[1..n]);
        self.line_arr[1..n].copy_from_slice(&other.line_arr[1..n]);
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
    pub has_pim: bool,
}

#[derive(Clone, Copy)]
enum CaseConv {
    Upper,
    Lower,
}

impl CaseConv {
    #[inline(always)]
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
    pub regparse_off: usize,
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
            regparse_off: 0,
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
    pub reg_match: crate::globals::GraphPtr<Regmatch>,
    pub reg_mmatch: crate::globals::GraphPtr<Regmmatch>,
    pub reg_startpos: crate::globals::GraphPtr<[Lpos; NSUBEXP]>,
    pub reg_endpos: crate::globals::GraphPtr<[Lpos; NSUBEXP]>,
    pub reg_win: crate::globals::WinPtr,
    pub reg_buf: crate::globals::BufPtr,
    pub reg_firstlnum: Linenr,
    pub reg_maxline: Linenr,
    pub reg_line_lbr: bool,
    pub lnum: Linenr,
    pub line: RegLineSlice,
    pub input: RegLineSlice,
    pub need_clear_subexpr: bool,
    pub need_clear_zsubexpr: bool,
    pub reg_ic: bool,
    pub reg_icombine: bool,
    pub reg_nobreak: bool,
    pub reg_maxcol: Colnr,
    pub nfa_has_zend: bool,
    pub nfa_has_backref: bool,
    pub nfa_nsubexpr: i32,
    pub nfa_listid: i32,
    pub nfa_alt_listid: i32,
    pub nfa_has_zsubexpr: bool,
}

impl Regexec {
    pub(super) const fn new() -> Self {
        Self {
            reg_match: crate::globals::GraphPtr::NULL,
            reg_mmatch: crate::globals::GraphPtr::NULL,
            reg_startpos: crate::globals::GraphPtr::NULL,
            reg_endpos: crate::globals::GraphPtr::NULL,
            reg_win: crate::globals::WinPtr::NULL,
            reg_buf: crate::globals::BufPtr::NULL,
            reg_firstlnum: 0,
            reg_maxline: 0,
            reg_line_lbr: false,
            lnum: 0,
            line: REG_LINE_NULL,
            input: REG_LINE_NULL,
            need_clear_subexpr: false,
            need_clear_zsubexpr: false,
            reg_ic: false,
            reg_icombine: false,
            reg_nobreak: false,
            reg_maxcol: 0,
            nfa_has_zend: false,
            nfa_has_backref: false,
            nfa_nsubexpr: 0,
            nfa_listid: 0,
            nfa_alt_listid: 0,
            nfa_has_zsubexpr: false,
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
    pub sm_match: crate::globals::GraphPtr<Regmatch>,
    pub sm_mmatch: crate::globals::GraphPtr<Regmmatch>,
    pub sm_firstlnum: Linenr,
    pub sm_maxline: Linenr,
    pub sm_line_lbr: i32,
}

impl Regsubmatch {
    pub(super) const fn new() -> Self {
        Self {
            sm_match: crate::globals::GraphPtr::NULL,
            sm_mmatch: crate::globals::GraphPtr::NULL,
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
    pub(super) regparse_base: Vec<u8>,
    pub(super) regparse_off: usize,
    pub(super) regnpar: i32,
    pub(super) wants_nfa: bool,
    pub(super) regnzpar: i32,
    pub(super) re_has_z: i32,
    pub(super) regflags: u32,
    pub(super) had_eol: bool,
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
    pub(super) reg_startzp: [RegLineSlice; NSUBEXP],
    pub(super) reg_endzp: [RegLineSlice; NSUBEXP],
    pub(super) reg_startzpos: [Lpos; NSUBEXP],
    pub(super) reg_endzpos: [Lpos; NSUBEXP],
    pub(super) eval_result: [Option<std::ffi::CString>; MAX_REGSUB_NESTING],
    pub(super) regsub_nesting: usize,
    pub(super) had_endbrace: [u8; NSUBEXP],
    pub(super) num_complex_braces: i32,
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
    pub(super) nfa_endp: Option<SaveSe>,
    pub(super) nfa_ll_index: usize,
    pub(super) regexp_engine: i32,
    pub(super) line_slots: [Vec<u8>; REG_LINE_SLOTS],
    pub(super) line_slot_addrs: [usize; REG_LINE_SLOTS],
    pub(super) cur_line_slot: usize,
}

impl RegexpState {
    const fn new() -> Self {
        Self {
            reg_prev_sub: None,
            class_tab: [0; 256],
            init_class_tab_done: false,
            regparse_base: Vec::new(),
            regparse_off: 0,
            regnpar: 0,
            wants_nfa: false,
            regnzpar: 0,
            re_has_z: 0,
            regflags: 0,
            had_eol: false,
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
                reg_match: crate::globals::GraphPtr::NULL,
                reg_mmatch: crate::globals::GraphPtr::NULL,
                reg_startpos: crate::globals::GraphPtr::NULL,
                reg_endpos: crate::globals::GraphPtr::NULL,
                reg_win: crate::globals::WinPtr::NULL,
                reg_buf: crate::globals::BufPtr::NULL,
                reg_firstlnum: 0,
                reg_maxline: 0,
                reg_line_lbr: false,
                lnum: 0,
                line: REG_LINE_NULL,
                input: REG_LINE_NULL,
                need_clear_subexpr: false,
                need_clear_zsubexpr: false,
                reg_ic: false,
                reg_icombine: false,
                reg_nobreak: false,
                reg_maxcol: 0,
                nfa_has_zend: false,
                nfa_has_backref: false,
                nfa_nsubexpr: 0,
                nfa_listid: 0,
                nfa_alt_listid: 0,
                nfa_has_zsubexpr: false,
            },
            rex_in_use: false,
            can_f_submatch: false,
            rsm: Regsubmatch {
                sm_match: crate::globals::GraphPtr::NULL,
                sm_mmatch: crate::globals::GraphPtr::NULL,
                sm_firstlnum: 0,
                sm_maxline: 0,
                sm_line_lbr: 0,
            },
            reg_startzp: [REG_LINE_NULL; NSUBEXP],
            reg_endzp: [REG_LINE_NULL; NSUBEXP],
            reg_startzpos: [Lpos { lnum: 0, col: 0 }; NSUBEXP],
            reg_endzpos: [Lpos { lnum: 0, col: 0 }; NSUBEXP],
            eval_result: [const { None }; MAX_REGSUB_NESTING],
            regsub_nesting: 0,
            had_endbrace: [0; NSUBEXP],
            num_complex_braces: 0,
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
            nfa_endp: None,
            nfa_ll_index: 0,
            regexp_engine: 0,
            line_slots: [const { Vec::new() }; REG_LINE_SLOTS],
            line_slot_addrs: [0; REG_LINE_SLOTS],
            cur_line_slot: 0,
        }
    }
}

pub(super) static RE_STATE: GlobalState<RegexpState> = GlobalState::new(RegexpState::new());

#[inline(always)]
pub(super) fn re_st() -> crate::globals::GraphPtr<RegexpState> {
    RE_STATE.handle()
}

#[inline(always)]
pub(super) fn reg_prev_sub_ref() -> Option<&'static CStr> {
    re_st().borrow_mut().reg_prev_sub.as_deref()
}

use crate::eval::partial_name;
use crate::eval::typval::tv_get_string_buf_chk;
use crate::eval::userfunc::call_func;
use crate::strings::{vim_strchr, vim_strsave_escaped};

#[inline(always)]
pub(super) fn tv_list_ref(l: Option<&mut TvList>) {
    if let Some(l) = l {
        l.lv_refcount += 1;
    }
}

#[inline(always)]
pub(super) const fn magic_fn(x: i32) -> i32 {
    x - 256
}

#[inline(always)]
pub(super) const fn un_magic(x: i32) -> i32 {
    x + 256
}

#[inline(always)]
pub(super) const fn is_magic_fn(x: i32) -> bool {
    x < 0
}

#[inline(always)]
pub(super) const fn no_magic(x: i32) -> i32 {
    if is_magic_fn(x) { un_magic(x) } else { x }
}

#[inline(always)]
pub(super) const fn toggle_magic(x: i32) -> i32 {
    if is_magic_fn(x) {
        un_magic(x)
    } else {
        magic_fn(x)
    }
}

#[inline(always)]
pub(super) fn reg_multi() -> bool {
    re_st().rex.reg_match.is_null()
}

/// Return NOT_MULTI if c is not a "multi" operator.
/// Return MULTI_ONE if c is a single "multi" operator.
/// Return MULTI_MULT if c is a multi "multi" operator.
#[inline(always)]
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
#[inline(always)]
pub(super) fn backslash_trans(c: i32) -> i32 {
    match c as u8 {
        b'r' => CAR,
        b't' => TAB,
        b'e' => ESC,
        b'b' => BS,
        _ => c,
    }
}

/// Check for a character class name `[:name:]`. `rem` starts at the `[`.
pub(super) fn get_char_class(rem: &mut &[u8]) -> i32 {
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

    let bytes = *rem;
    if bytes.get(1) == Some(&b':')
        && bytes.get(2).is_some_and(u8::is_ascii_lowercase)
        && bytes.get(3).is_some_and(u8::is_ascii_lowercase)
        && bytes.get(4).is_some_and(u8::is_ascii_lowercase)
    {
        let target = &bytes[2..];
        for &(key, pat) in CHAR_CLASS_TAB {
            if let Some(rest) = target.strip_prefix(pat) {
                *rem = rest;
                return key;
            }
        }
    }
    CLASS_NONE
}

pub(super) fn init_class_tab() {
    if re_st().init_class_tab_done {
        return;
    }
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
        re_st().class_tab[i] = val;
    }
    re_st().class_tab[b' ' as usize] |= RI_WHITE;
    re_st().class_tab[b'\t' as usize] |= RI_WHITE;
    re_st().init_class_tab_done = true;
}

#[inline(always)]
pub(super) fn ri_digit(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_DIGIT) != 0
}
#[inline(always)]
pub(super) fn ri_hex(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_HEX) != 0
}
#[inline(always)]
pub(super) fn ri_octal(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_OCTAL) != 0
}
#[inline(always)]
pub(super) fn ri_word(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_WORD) != 0
}
#[inline(always)]
pub(super) fn ri_head(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_HEAD) != 0
}
#[inline(always)]
pub(super) fn ri_alpha(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_ALPHA) != 0
}
#[inline(always)]
pub(super) fn ri_lower(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_LOWER) != 0
}
#[inline(always)]
pub(super) fn ri_upper(c: i32) -> bool {
    (0..0x100).contains(&c) && (re_st().class_tab[c as usize] & RI_UPPER) != 0
}

/// Return true if compiled regular expression `prog` can match a line break.
pub fn re_multiline(prog: &Regprog) -> i32 {
    (prog.regflags & RF_HASNL) as i32
}

/// Check for an equivalence class name `[=a=]`. `rem` starts at the `[`.
pub(super) fn get_equi_class(rem: &mut &[u8]) -> i32 {
    let bytes = *rem;
    if bytes.len() >= 3 && bytes[1] == b'=' {
        let l = crate::mbyte::utfc_slice2len(&bytes[2..]);
        if bytes.get(l + 2..l + 4) == Some(b"=]") {
            let c = crate::mbyte::utf_slice2char(&bytes[2..]);
            *rem = &bytes[l + 4..];
            return c;
        }
    }
    0
}

/// Check for a collating element `[.a.]`. `rem` starts at the `[`.
pub(super) fn get_coll_element(rem: &mut &[u8]) -> i32 {
    let bytes = *rem;
    if bytes.len() >= 3 && bytes[1] == b'.' {
        let l = crate::mbyte::utfc_slice2len(&bytes[2..]);
        if bytes.get(l + 2..l + 4) == Some(b".]") {
            let c = crate::mbyte::utf_slice2char(&bytes[2..]);
            *rem = &bytes[l + 4..];
            return c;
        }
    }
    0
}

pub(super) fn get_cpo_flags() {
    re_st().reg_cpo_lit =
        (crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_LITERAL)) as i32;
}

/// Skip over a `[]` range in a byte slice.
pub(super) fn skip_anyof_slice(bytes: &[u8]) -> &[u8] {
    let mut rem = crate::memory::bytes_until_nul(bytes);
    if let Some(rest) = rem.strip_prefix(b"^") {
        rem = rest;
    }
    if matches!(rem.first(), Some(b']' | b'-')) {
        rem = &rem[1..];
    }
    while let Some(&b0) = rem.first() {
        if b0 == b']' {
            break;
        }
        let l = crate::mbyte::utfc_slice2len(rem);
        if l > 1 {
            rem = &rem[l..];
        } else if b0 == b'-' {
            rem = &rem[1..];
            if let Some(&b1) = rem.first()
                && b1 != b']'
            {
                let step = crate::mbyte::utfc_slice2len(rem);
                rem = &rem[step..];
            }
        } else if b0 == b'\\'
            && let Some(&b1) = rem.get(1)
            && (REGEXP_INRANGE.to_bytes().contains(&b1)
                || (re_st().reg_cpo_lit == 0 && REGEXP_ABBR.to_bytes().contains(&b1)))
        {
            rem = &rem[2..];
        } else if b0 == b'[' {
            if get_char_class(&mut rem) == CLASS_NONE
                && get_equi_class(&mut rem) == 0
                && get_coll_element(&mut rem) == 0
                && !rem.is_empty()
            {
                rem = &rem[1..];
            }
        } else {
            rem = &rem[1..];
        }
    }
    rem
}

/// Skip past regular expression in a byte slice.
pub fn skip_regexp_slice(start: &[u8], dirc: i32, magic: i32) -> &[u8] {
    let mut rem = crate::memory::bytes_until_nul(start);
    let mut mymagic = if magic != 0 { MAGIC_ON } else { MAGIC_OFF };

    get_cpo_flags();

    while let Some(&b0) = rem.first() {
        if i32::from(b0) == dirc {
            break;
        }
        if (b0 == b'[' && mymagic >= MAGIC_ON) || (rem.starts_with(b"\\[") && mymagic <= MAGIC_OFF)
        {
            rem = skip_anyof_slice(&rem[1..]);
            if rem.is_empty() {
                break;
            }
        } else if b0 == b'\\' && rem.len() > 1 {
            rem = &rem[1..];
            let next_b = rem.first().copied().unwrap_or(0);
            if next_b == b'v' {
                mymagic = MAGIC_ALL;
            } else if next_b == b'V' {
                mymagic = MAGIC_NONE;
            }
        }
        let step = crate::mbyte::utfc_slice2len(rem).max(1);
        rem = &rem[step.min(rem.len())..];
    }
    rem
}

/// Skip past regular expression.
pub fn skip_regexp(start: &std::ffi::CStr, delim: i32, magic: i32) -> &std::ffi::CStr {
    let bytes = start.to_bytes();
    let rem = skip_regexp_slice(bytes, delim, magic);
    crate::memory::CStrSubslice::advance(start, bytes.len() - rem.len())
}

/// Call `skip_regexp()` and when the delimiter does not match give an error and return `None`.
pub fn skip_regexp_err(start: &std::ffi::CStr, delim: i32, magic: i32) -> Option<&std::ffi::CStr> {
    let p = skip_regexp(start, delim, magic);
    if p.to_bytes().first().copied().unwrap_or(0) as i32 != delim {
        semsg!(tr(E_MISSING_DELIMITER_AFTER_SEARCH_PATTERN_STR), start,);
        return None;
    }
    Some(p)
}

/// Skip past regular expression without mutating `start`, returning the byte offset and final magic state.
pub fn skip_regexp_magic(
    start: &[u8],
    dirc: i32,
    magic: i32,
    mut magic_val: Option<&mut RegMagic>,
) -> usize {
    let bytes = crate::memory::bytes_until_nul(start);
    let mut rem = bytes;
    let mut mymagic = if magic != 0 { MAGIC_ON } else { MAGIC_OFF };

    get_cpo_flags();

    while let Some(&b0) = rem.first() {
        if i32::from(b0) == dirc {
            break;
        }
        if (b0 == b'[' && mymagic >= MAGIC_ON) || (rem.starts_with(b"\\[") && mymagic <= MAGIC_OFF)
        {
            rem = skip_anyof_slice(&rem[1..]);
            if rem.is_empty() {
                break;
            }
        } else if b0 == b'\\' && rem.len() > 1 {
            rem = &rem[1..];
            let next_b = rem.first().copied().unwrap_or(0);
            if next_b == b'v' {
                mymagic = MAGIC_ALL;
            } else if next_b == b'V' {
                mymagic = MAGIC_NONE;
            }
        }
        let step = crate::mbyte::utfc_slice2len(rem).max(1);
        rem = &rem[step.min(rem.len())..];
    }
    if let Some(ref mut p) = magic_val {
        **p = mymagic;
    }
    bytes.len() - rem.len()
}

/// `skip_regexp()` with in-place backslash-question removal, returning the byte offset.
pub fn skip_regexp_ex(
    start: &mut [u8],
    dirc: i32,
    magic: i32,
    mut copy_buf: Option<&mut Vec<u8>>,
    mut dropped: Option<&mut i32>,
    mut magic_val: Option<&mut RegMagic>,
) -> usize {
    let mut mymagic = if magic != 0 { MAGIC_ON } else { MAGIC_OFF };
    let mut startplen = crate::memory::bytes_until_nul(start).len();
    let mut off: usize = 0;
    let mut use_copy = false;

    get_cpo_flags();

    loop {
        let active: &mut [u8] = if use_copy {
            copy_buf.as_deref_mut().unwrap().as_mut_slice()
        } else {
            start
        };
        let p_bytes = &active[off..startplen];
        let Some(&b0) = p_bytes.first() else {
            break;
        };
        if b0 as i32 == dirc {
            break;
        }
        if (b0 == b'[' && mymagic >= MAGIC_ON)
            || (p_bytes.starts_with(b"\\[") && mymagic <= MAGIC_OFF)
        {
            let rem = skip_anyof_slice(&p_bytes[1..]);
            off += 1 + (p_bytes.len() - 1 - rem.len());
            if rem.is_empty() {
                break;
            }
        } else if b0 == b'\\' && p_bytes.len() > 1 {
            if dirc == b'?' as i32 && p_bytes[1] == b'?' {
                if let Some(buf) = copy_buf.as_deref_mut()
                    && !use_copy
                {
                    buf.clear();
                    buf.extend_from_slice(&start[..=startplen]);
                    use_copy = true;
                }
                if let Some(ref mut p) = dropped {
                    **p += 1;
                }
                let target: &mut [u8] = if use_copy {
                    copy_buf.as_deref_mut().unwrap().as_mut_slice()
                } else {
                    start
                };
                target.copy_within(off + 1..=startplen, off);
                startplen -= 1;
            } else {
                off += 1;
            }
            let target: &[u8] = if use_copy {
                copy_buf.as_deref().unwrap().as_slice()
            } else {
                start
            };
            let next_b = target.get(off).copied().unwrap_or(0);
            if next_b == b'v' {
                mymagic = MAGIC_ALL;
            } else if next_b == b'V' {
                mymagic = MAGIC_NONE;
            }
        }
        let target: &[u8] = if use_copy {
            copy_buf.as_deref().unwrap().as_slice()
        } else {
            start
        };
        let rem = &target[off..startplen];
        if rem.is_empty() {
            break;
        }
        let step = crate::mbyte::utfc_slice2len(rem).max(1).min(rem.len());
        off += step;
    }
    if let Some(ref mut p) = magic_val {
        **p = mymagic;
    }
    off
}

pub(super) fn initchr(str_: &CStr) {
    re_st().regparse_base.clear();
    re_st().regparse_base.extend_from_slice(str_.to_bytes());
    re_st().regparse_off = 0;
    re_st().prevchr_len = 0;
    re_st().nextchr = -1;
    re_st().prevchr = -1;
    re_st().prevprevchr = -1;
    re_st().curchr = -1;
    re_st().at_start = 1;
    re_st().prev_at_start = 0;
}

pub(super) fn save_parse_state(ps: &mut ParseState) {
    ps.regparse_off = re_st().regparse_off;
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
    re_st().regparse_off = ps.regparse_off;
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
    if re_st().curchr != -1 {
        return re_st().curchr;
    }

    re_st().curchr = re_st().regparse_byte() as i32;
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
                let re_ref = re_st().borrow();
                let mut p_bytes = &re_ref.regparse_bytes()[1..];
                let mut is_magic_all = re_ref.reg_magic == MAGIC_ALL;

                while let Some(&next_b) = p_bytes.get(1) {
                    if p_bytes[0] != b'\\'
                        || !matches!(next_b, b'c' | b'C' | b'm' | b'M' | b'v' | b'V' | b'Z')
                    {
                        break;
                    }
                    if next_b == b'v' {
                        is_magic_all = true;
                    } else if matches!(next_b, b'm' | b'M' | b'V') {
                        is_magic_all = false;
                    }
                    p_bytes = &p_bytes[2..];
                }
                let p_b = p_bytes.first().copied().unwrap_or(0);
                if p_b == NUL
                    || (p_b == b'\\'
                        && p_bytes
                            .get(1)
                            .is_some_and(|&b| matches!(b, b'|' | b'&' | b')' | b'n')))
                    || (is_magic_all && matches!(p_b, b'|' | b'&' | b')'))
                    || re_st().reg_magic == MAGIC_ALL
                {
                    re_st().curchr = magic_fn(b'$' as i32);
                }
            }
        }
        b'\\' => {
            let c = i32::from(re_st().regparse_byte_at(1));
            if c == NUL as i32 {
                re_st().curchr = b'\\' as i32;
            } else if c <= b'~' as i32 && META_FLAGS[c as usize] != 0 {
                re_st().curchr = -1;
                re_st().prev_at_start = re_st().at_start;
                re_st().at_start = 0;
                re_st().regparse_off += 1;
                re_st().peekchr_after_slash += 1;
                let _ = peekchr();
                re_st().regparse_off -= 1;
                re_st().peekchr_after_slash -= 1;
                re_st().curchr = toggle_magic(re_st().curchr);
            } else if crate::strings::vim_haschr(REGEXP_ABBR, c) {
                re_st().curchr = backslash_trans(c);
            } else if re_st().reg_magic == MAGIC_NONE && (c == b'$' as i32 || c == b'^' as i32) {
                re_st().curchr = toggle_magic(c);
            } else {
                re_st().curchr = if c >= 0x80 {
                    utf_ptr2char(&re_st().regparse_bytes()[1..])
                } else {
                    c
                };
            }
        }
        _ => {
            if re_st().curchr >= 0x80 {
                re_st().curchr = utf_ptr2char(re_st().regparse_bytes());
            }
        }
    }

    re_st().curchr
}

pub(super) fn skipchr() {
    if re_st().regparse_byte() == b'\\' {
        re_st().prevchr_len = 1;
    } else {
        re_st().prevchr_len = 0;
    }
    let next_b = re_st().regparse_byte_at(re_st().prevchr_len as usize);
    if next_b != NUL {
        if next_b < 0x80 {
            re_st().prevchr_len += 1;
        } else {
            let off = re_st().prevchr_len as usize;
            re_st().prevchr_len += utf_ptr2len(&re_st().regparse_bytes()[off..]);
        }
    }
    re_st().regparse_off += re_st().prevchr_len as usize;
    re_st().prev_at_start = re_st().at_start;
    re_st().at_start = 0;
    re_st().prevprevchr = re_st().prevchr;
    re_st().prevchr = re_st().curchr;
    re_st().curchr = re_st().nextchr;
    re_st().nextchr = -1;
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
    re_st().nextchr = re_st().curchr;
    re_st().curchr = re_st().prevchr;
    re_st().prevchr = re_st().prevprevchr;
    re_st().at_start = re_st().prev_at_start;
    re_st().prev_at_start = 0;
    re_st().regparse_off -= re_st().prevchr_len as usize;
}

pub(super) fn gethexchrs(maxinputlen: i32) -> i64 {
    let mut nr: i64 = 0;
    let mut i: i32 = 0;
    while i < maxinputlen {
        let c = re_st().regparse_byte() as i32;
        if !ascii_isxdigit(c) {
            break;
        }
        nr <<= 4;
        nr |= i64::from(hex2nr(c));
        re_st().regparse_off += 1;
        i += 1;
    }
    if i == 0 { -1 } else { nr }
}

pub(super) fn getdecchrs() -> i64 {
    let mut nr: i64 = 0;
    let mut i: i32 = 0;
    loop {
        let c = re_st().regparse_byte();
        if !c.is_ascii_digit() {
            break;
        }
        nr = nr.wrapping_mul(10).wrapping_add(i64::from(c - b'0'));
        re_st().regparse_off += 1;
        re_st().curchr = -1;
        i += 1;
    }
    if i == 0 { -1 } else { nr }
}

pub(super) fn getoctchrs() -> i64 {
    let mut nr: i64 = 0;
    let mut i: i32 = 0;
    while i < 3 && nr < 0o40 {
        let c = re_st().regparse_byte();
        if !(b'0'..=b'7').contains(&c) {
            break;
        }
        nr <<= 3;
        nr |= i64::from(hex2nr(c as i32));
        re_st().regparse_off += 1;
        i += 1;
    }
    if i == 0 { -1 } else { nr }
}

pub(super) fn read_limits(minval: &mut i32, maxval: &mut i32) -> bool {
    let mut reverse = false;
    if re_st().regparse_byte() == b'-' {
        re_st().regparse_off += 1;
        reverse = true;
    }
    let first_char_b = re_st().regparse_byte();
    {
        let adv = {
            let re_ref = re_st().borrow();
            let start_bytes = re_ref.regparse_bytes();
            let mut rem = start_bytes;
            *minval = getdigits_int(&mut rem, false, 0);
            start_bytes.len() - rem.len()
        };
        re_st().regparse_off += adv;
    }
    if re_st().regparse_byte() == b',' {
        re_st().regparse_off += 1;
        if ascii_isdigit(re_st().regparse_byte() as i32) {
            let adv = {
                let re_ref = re_st().borrow();
                let start_bytes = re_ref.regparse_bytes();
                let mut rem = start_bytes;
                *maxval = getdigits_int(&mut rem, false, MAX_LIMIT);
                start_bytes.len() - rem.len()
            };
            re_st().regparse_off += adv;
        } else {
            *maxval = MAX_LIMIT;
        }
    } else if ascii_isdigit(first_char_b as i32) {
        *maxval = *minval;
    } else {
        *maxval = MAX_LIMIT;
    }
    if re_st().regparse_byte() == b'\\' {
        re_st().regparse_off += 1;
    }
    if re_st().regparse_byte() != b'}' {
        semsg!(
            tr(c"E554: Syntax error in %s{...}"),
            if re_st().reg_magic == MAGIC_ALL {
                c""
            } else {
                c"\\"
            },
        );
        g_st().rc_did_emsg = true;
        return false;
    }
    if (!reverse && *minval > *maxval) || (reverse && *minval < *maxval) {
        std::mem::swap(minval, maxval);
    }
    skipchr();
    true
}

#[inline(always)]
pub(super) fn reg_breakcheck() {
    if !re_st().rex.reg_nobreak {
        fast_breakcheck();
    }
}

pub(super) fn reg_iswordc(c: i32) -> bool {
    vim_iswordc_buf(c, re_st().rex.buf_mut())
}

fn reg_getline_bounds(lnum: Linenr, submatch: bool) -> (Linenr, Linenr) {
    if submatch {
        (re_st().rsm.sm_firstlnum + lnum, re_st().rsm.sm_maxline)
    } else {
        (re_st().rex.reg_firstlnum + lnum, re_st().rex.reg_maxline)
    }
}

pub(super) fn reg_getline(lnum: Linenr) -> Option<&'static std::ffi::CStr> {
    let (firstlnum, maxline) = reg_getline_bounds(lnum, false);
    if firstlnum < 1 {
        return None;
    }
    if lnum > maxline {
        return Some(c"");
    }
    Some(ml_get_buf(re_st().rex.buf_mut(), firstlnum))
}

pub(super) fn reg_getline_len(lnum: Linenr) -> Colnr {
    let (firstlnum, maxline) = reg_getline_bounds(lnum, false);
    if firstlnum < 1 || lnum > maxline {
        return 0;
    }
    ml_get_buf_len(re_st().rex.buf_mut(), firstlnum)
}

pub(super) fn reg_prev_class() -> i32 {
    let off = re_st().rex.input_off();
    if off > 0 {
        let line_bytes = re_st().rex.line.unwrap_or(&[]);
        let prev_off = off - 1;
        let head_off = prev_off - crate::mbyte::utf_head_off_slice(line_bytes, prev_off) as usize;
        return mb_get_class_tab(&line_bytes[head_off..], &re_st().rex.buf_mut().b_chartab);
    }
    -1
}

pub(super) fn reg_match_visual() -> bool {
    let wp = if re_st().rex.reg_win.is_null() {
        curwin_ptr().as_ptr()
    } else {
        re_st().rex.reg_win.as_ptr()
    };
    let wp = crate::globals::GraphPtr::of(wp).borrow_mut();
    if re_st().rex.reg_buf != g_st().curbuf || g_st().visual_global.start.lnum == 0 || !reg_multi()
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

    let col = re_st().rex.input_col();
    if mode == b'v' as i32 {
        if (lnum == top.lnum && col < top.col)
            || (lnum == bot.lnum
                && col >= bot.col + Colnr::from(opt_vars().p_sel.first_byte() != b'e'))
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

        let cur_lnum = re_st().rex.lnum;
        re_st().rex.load_line(cur_lnum);
        re_st().rex.input = re_st().rex.line.wrapping_add(col as usize);

        let cols = win_linetabsize(
            &mut *wp,
            re_st().rex.reg_firstlnum + re_st().rex.lnum,
            re_st().rex.line.as_cstr(),
            col,
        );
        if cols < start || cols > end - Colnr::from(opt_vars().p_sel.first_byte() == b'e') {
            return false;
        }
    }
    true
}

pub(super) fn prog_magic_wrong() -> bool {
    let prog = if reg_multi() {
        re_st().rex.mmatch_mut().prog_mut()
    } else {
        re_st().rex.match_mut().prog_mut()
    };
    let RegProgEngine::Bt(ref bt_prog) = prog.engine else {
        return false;
    };
    if bt_prog.program.first().copied() != Some(REGMAGIC as u8) {
        emsg(tr(E_RE_CORR));
        return true;
    }
    false
}

pub(super) fn cleanup_subexpr() {
    if !re_st().rex.need_clear_subexpr {
        return;
    }
    if reg_multi() {
        let uninit_pos = Lpos { lnum: -1, col: -1 };
        re_st().rex.startpos_slice_mut().fill(uninit_pos);
        re_st().rex.endpos_slice_mut().fill(uninit_pos);
    } else {
        let m = re_st().rex.match_mut();
        m.startp.fill(REG_LINE_NULL);
        m.endp.fill(REG_LINE_NULL);
    }
    re_st().rex.need_clear_subexpr = false;
}

pub(super) fn cleanup_zsubexpr() {
    if !re_st().rex.need_clear_zsubexpr {
        return;
    }
    if reg_multi() {
        let uninit_pos = Lpos { lnum: -1, col: -1 };
        re_st().reg_startzpos = [uninit_pos; NSUBEXP];
        re_st().reg_endzpos = [uninit_pos; NSUBEXP];
    } else {
        re_st().reg_startzp = [REG_LINE_NULL; NSUBEXP];
        re_st().reg_endzp = [REG_LINE_NULL; NSUBEXP];
    }
    re_st().rex.need_clear_zsubexpr = false;
}

pub(super) fn reg_nextline() {
    re_st().rex.lnum += 1;
    let cur_lnum = re_st().rex.lnum;
    re_st().rex.load_line(cur_lnum);
    re_st().rex.input = re_st().rex.line;
    reg_breakcheck();
}

pub(super) fn match_with_backref(
    start_lnum: Linenr,
    start_col: Colnr,
    end_lnum: Linenr,
    end_col: Colnr,
    bytelen: &mut i32,
) -> RegAction {
    let mut clnum = start_lnum;
    let mut ccol = start_col;

    *bytelen = 0;
    loop {
        if re_st().reg_tofree.is_empty()
            || re_st().rex.line.addr() != re_st().reg_tofree.as_ptr() as usize
        {
            let line_bytes = re_st().rex.line.as_cstr().to_bytes_with_nul().to_vec();
            let len = line_bytes.len() - 1 + 51;
            let off = re_st().rex.input_off();
            if re_st().reg_tofree.len() < len {
                re_st().reg_tofree.resize(len, 0);
            }
            re_st().reg_tofree[..line_bytes.len()].copy_from_slice(&line_bytes);
            re_st().rex.line =
                reg_line_from_cstr(crate::memory::bytes_as_cstr(&re_st().reg_tofree));
            re_st().rex.input = re_st().rex.line.wrapping_add(off);
        }

        let p_cstr = reg_getline(clnum).unwrap();
        let p_bytes = p_cstr.to_bytes_with_nul();

        let mut len = if clnum == end_lnum {
            end_col - ccol
        } else {
            reg_getline_len(clnum) - ccol
        };

        let input_bytes = re_st().rex.input.as_bytes();
        if (!re_st().rex.reg_ic && cstrncmp(&p_bytes[ccol as usize..], input_bytes, &mut len) != 0)
            || (re_st().rex.reg_ic
                && mb_strnicmp(
                    &p_bytes[ccol as usize..],
                    re_st().rex.input.as_bytes(),
                    len as usize,
                ) != 0)
        {
            return RegAction::NoMatch;
        }
        *bytelen += len;
        if clnum == end_lnum {
            break;
        }
        if re_st().rex.lnum >= re_st().rex.reg_maxline {
            return RegAction::NoMatch;
        }

        reg_nextline();
        *bytelen = 0;
        clnum += 1;
        ccol = 0;
        if g_st().got_int {
            return RegAction::Fail;
        }
    }
    RegAction::Match
}

pub(super) fn re_mult_next(what: &std::ffi::CStr) -> bool {
    if re_multi_type(peekchr()) == MULTI_MULT {
        semsg!(tr(c"E888: (NFA regexp) cannot repeat %s"), what);
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

pub(super) fn cstrncmp(s1: &[u8], s2: &[u8], n: &mut i32) -> i32 {
    let s1_b = crate::memory::bytes_until_nul(s1);
    let s2_b = crate::memory::bytes_until_nul(s2);
    let mut result = if !re_st().rex.reg_ic {
        let max_n = *n as usize;
        if s1_b.len() < max_n && s1_b.len() == s2_b.len() {
            s1_b.cmp(s2_b) as i32
        } else if s1_b.len() < max_n {
            if s2_b.starts_with(s1_b) {
                -1
            } else {
                s1_b.cmp(&s2_b[..s1_b.len().min(s2_b.len())]) as i32
            }
        } else if s2_b.len() < max_n {
            if s1_b.starts_with(s2_b) {
                1
            } else {
                s1_b[..s2_b.len()].cmp(s2_b) as i32
            }
        } else {
            s1_b[..max_n].cmp(&s2_b[..max_n]) as i32
        }
    } else {
        let mut rem1 = s1_b;
        let mut n2: i32 = 0;
        let mut n1 = *n;
        while n1 > 0 && !rem1.is_empty() {
            let step = crate::mbyte::utf_slice2len(rem1);
            n1 -= step as i32;
            rem1 = &rem1[step..];
            n2 += 1;
        }
        let mut rem2 = s2_b;
        while n2 > 0 && !rem2.is_empty() {
            n2 -= 1;
            let step = crate::mbyte::utf_slice2len(rem2);
            rem2 = &rem2[step..];
        }
        n2 = (s2_b.len() - rem2.len()) as i32;
        let r = utf_strnicmp(s1_b, s2_b, *n as usize, n2 as usize);
        if r == 0 {
            *n = n2;
        }
        r
    };

    if result != 0 && re_st().rex.reg_icombine {
        let mut rem1 = s1_b;
        let mut rem2 = s2_b;
        let mut c1: i32 = 0;
        let mut c2: i32 = 0;
        let mut c11: i32 = 0;
        let mut c12: i32 = 0;
        let mut junk: i32 = 0;
        let mut junk2: i32 = 0;

        while ((s1_b.len() - rem1.len()) as i32) < *n {
            c1 = mb_ptr2char_adv(&mut rem1);
            c2 = mb_ptr2char_adv(&mut rem2);

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
            *n = (s2_b.len() - rem2.len()) as i32;
        }
    }

    result
}

#[inline(always)]
pub(super) fn cstrchr(s: &CStr, c: i32) -> Option<usize> {
    let s_ptr = s.as_ptr();
    if !re_st().rex.reg_ic {
        return vim_strchr(s, c).map(|found| (found.as_ptr() as usize) - (s_ptr as usize));
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
        return vim_strchr(s, c).map(|found| (found.as_ptr() as usize) - (s_ptr as usize));
    };

    let s_bytes = s.to_bytes();
    let mut idx = 0usize;
    while idx < s_bytes.len() {
        let p = s_ptr.wrapping_add(idx);
        let b0 = s_bytes[idx] as i32;
        let uc = utf_ptr2char(p);
        if c > 0x80 || uc > 0x80 {
            if (uc < 0x80 || uc != b0) && utf_fold(uc) == lc {
                return Some(idx);
            }
        } else if b0 == c || b0 == cc {
            return Some(idx);
        }
        idx += utfc_ptr2len(p) as usize;
    }
    None
}

/// Replace tildes in the pattern by the old pattern.
/// Returns `Some(CString)` if a new string was allocated, or `None` if `source` was modified in place.
pub fn regtilde(source: &mut [u8], magic: i32, preview: bool) -> Option<std::ffi::CString> {
    let mut newsub_vec: Option<Vec<u8>> = None;
    let mut newsublen = crate::memory::bytes_until_nul(source).len();
    let tilde: &[u8] = if magic == 0 { b"\\~" } else { b"~" };
    let tildelen = tilde.len();
    let mut error = false;

    let mut idx = 0usize;
    while idx < newsublen {
        let cur_slice: &[u8] = match newsub_vec {
            Some(ref v) => &v[..newsublen],
            None => &source[..newsublen],
        };
        if cur_slice[idx..].starts_with(tilde) {
            let prefixlen = idx;
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

                let mut tmpsub_vec: Vec<u8> = Vec::with_capacity(tmpsublen + 1);
                tmpsub_vec.extend_from_slice(&cur_slice[..prefixlen]);
                tmpsub_vec.extend_from_slice(prev_sub.to_bytes());
                tmpsub_vec.extend_from_slice(&cur_slice[prefixlen + tildelen..]);
                tmpsub_vec.push(0);

                newsub_vec = Some(tmpsub_vec);
                newsublen = tmpsublen;
                idx = prefixlen + reg_prev_sublen;
            } else {
                let buf: &mut [u8] = match newsub_vec {
                    Some(ref mut v) => v.as_mut_slice(),
                    None => source,
                };
                buf.copy_within(
                    prefixlen + tildelen..=prefixlen + tildelen + postfixlen,
                    prefixlen,
                );
            }
            continue;
        }
        if cur_slice[idx] == b'\\' && idx + 1 < newsublen {
            idx += 1;
        }
        idx += crate::mbyte::utfc_slice2len(&cur_slice[idx..]);
    }

    if error {
        return None;
    }

    if !preview {
        re_st().reg_prev_sub = if newsublen == 0 {
            None
        } else {
            let cur_slice: &[u8] = match newsub_vec {
                Some(ref v) => &v[..newsublen],
                None => &source[..newsublen],
            };
            cstring_from_bytes(cur_slice)
        };
    }

    newsub_vec.map(|v| crate::memory::cstring_until_nul(&v))
}

pub(crate) fn fill_submatch_list(
    _argc: i32,
    argv: &mut [TypVal],
    argskip: i32,
    fp: &mut UserFunc,
) -> i32 {
    if fp.uf_varargs == 0 && (fp.uf_args.len() as i32) <= argskip {
        return argskip;
    }
    let listarg = &mut argv[argskip as usize];

    let static_list =
        crate::globals::GraphPtr::of(listarg.vval.v_list().cast::<StaticList10>()).borrow_mut();
    tv_list_init_static10(static_list);

    let mut li = crate::globals::GraphPtr::from(tv_list_first(listarg.vval.v_list().opt()));
    for i in 0..10 {
        let s = re_st().rsm.match_mut().startp[i];
        let e = re_st().rsm.match_mut().endp[i];
        let li_mut = li.borrow_mut();
        if s.is_null() || e.is_null() {
            tv_set_string(&mut li_mut.li_tv, None);
        } else {
            let bytes = s.span_to(e);
            let cs = cstring_from_bytes(bytes).unwrap_or_default();

            tv_set_string(&mut li_mut.li_tv, Some(cs.as_c_str()));
        }

        li = li_mut.li_next;
    }
    argskip + 1
}

pub fn vim_regsub(
    rmp: &mut Regmatch,
    source: &CStr,
    expr: Option<&TypVal>,
    dest: &mut [u8],
    flags: i32,
) -> i32 {
    let mut rex_save = Regexec::new();
    let rex_in_use_save = re_st().rex_in_use;

    if re_st().rex_in_use {
        rex_save = re_st().rex;
    }
    re_st().rex_in_use = true;

    re_st().rex.reg_match = crate::globals::GraphPtr::from(&mut *rmp);
    re_st().rex.reg_mmatch = crate::globals::GraphPtr::NULL;
    re_st().rex.reg_maxline = 0;
    re_st().rex.reg_buf = g_st().curbuf;
    re_st().rex.reg_line_lbr = true;
    let result = vim_regsub_both(Some(source), expr, dest, flags);

    re_st().rex_in_use = rex_in_use_save;
    if re_st().rex_in_use {
        re_st().rex = rex_save;
    }

    result
}

pub fn vim_regsub_multi(
    rmp: &mut Regmmatch,
    lnum: Linenr,
    source: &CStr,
    dest: &mut [u8],
    flags: i32,
) -> i32 {
    let mut rex_save = Regexec::new();
    let rex_in_use_save = re_st().rex_in_use;

    if re_st().rex_in_use {
        rex_save = re_st().rex;
    }
    re_st().rex_in_use = true;

    re_st().rex.reg_match = crate::globals::GraphPtr::NULL;
    re_st().rex.reg_mmatch = crate::globals::GraphPtr::from(&mut *rmp);
    re_st().rex.reg_buf = g_st().curbuf;
    re_st().rex.reg_firstlnum = lnum;
    re_st().rex.reg_maxline = curbuf_mut().b_ml.ml_line_count - lnum;
    re_st().rex.reg_line_lbr = false;
    let result = vim_regsub_both(Some(source), None, dest, flags);

    re_st().rex_in_use = rex_in_use_save;
    if re_st().rex_in_use {
        re_st().rex = rex_save;
    }

    result
}

fn vim_regsub_both(
    source: Option<&CStr>,
    expr: Option<&TypVal>,
    dest: &mut [u8],
    flags: i32,
) -> i32 {
    let mut no: i32 = -1;
    let mut func_all: Option<CaseConv> = None;
    let mut func_one: Option<CaseConv> = None;
    let mut clnum: Linenr = 0;
    let mut len: i32 = 0;
    let copy = (flags & REGSUB_COPY) != 0;

    if source.is_none() && expr.is_none() {
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
    let max_dst = dest.len();
    let mut dest_slice = copy.then_some(dest);
    let mut dst_off = 0usize;

    if expr.is_some() || source.is_some_and(|s| s.to_bytes().starts_with(b"\\=")) {
        if copy {
            if let Some(ref eval_cstr) = *eval_slot {
                let eval_bytes = eval_cstr.to_bytes_with_nul();
                let eval_len = eval_bytes.len() - 1;
                if eval_len < max_dst {
                    if let Some(ref mut out) = dest_slice {
                        out[..eval_bytes.len()].copy_from_slice(eval_bytes);
                    }
                    dst_off += eval_len;
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
                let mut argv: [TypVal; 2] = [const { TypVal::new() }; 2];
                let mut rettv: TypVal = TypVal::new();
                let mut match_list = StaticList10::new();
                rettv.v_type = VarType::VarString;
                rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
                argv[0].v_type = VarType::VarList;
                argv[0].vval = crate::eval::typval::TypvalVval::List(
                    crate::globals::GraphPtr::new(&raw mut match_list.sl_list),
                );
                let mut funcexe = Funcexe {
                    fe_fill_submatch: true,
                    fe_firstline: 0,
                    fe_lastline: 0,
                    fe_doesrange: false,
                    fe_evaluate: true,
                    fe_partial: crate::globals::GraphPtr::NULL,
                    fe_selfdict: crate::globals::GraphPtr::NULL,
                    fe_basetv: crate::globals::GraphPtr::NULL,
                    fe_found_var: false,
                };
                if expr.v_type == VarType::VarFunc {
                    let s_cstr = expr.vval.v_string_cstr_or_empty();
                    call_func(s_cstr, -1, &mut rettv, 1, &mut argv, &mut funcexe);
                } else if expr.v_type == VarType::VarPartial {
                    let partial = expr.vval.v_partial();
                    let s = partial_name(partial.opt());
                    funcexe.fe_partial = partial;
                    call_func(s, -1, &mut rettv, 1, &mut argv, &mut funcexe);
                }
                if rettv.v_type == VarType::VarUnknown {
                    *eval_slot = None;
                } else {
                    let mut buf = [0u8; NUMBUFLEN];
                    if let Some(s) = tv_get_string_buf_chk(&rettv, &mut buf) {
                        *eval_slot = Some(s.to_owned());
                    }
                }
                tv_clear(&mut rettv);
            } else if let Some(src) = source {
                *eval_slot =
                    eval_to_string(crate::memory::CStrSubslice::advance(src, 2), true, false);
            }
            re_st().regsub_nesting -= 1;

            if let Some(eval_cstr) = eval_slot.take() {
                let mut eval_bytes = eval_cstr.into_bytes_with_nul();
                let mut had_backslash = false;
                let eval_len = eval_bytes.len().saturating_sub(1);
                let mut s_idx = 0usize;
                while s_idx < eval_len {
                    if eval_bytes[s_idx] == NL as u8 && re_st().rsm.sm_line_lbr == 0 {
                        eval_bytes[s_idx] = CAR as u8;
                    } else if eval_bytes[s_idx] == b'\\' && s_idx + 1 < eval_len {
                        s_idx += 1;
                        if eval_bytes[s_idx] == NL as u8 && re_st().rsm.sm_line_lbr == 0 {
                            eval_bytes[s_idx] = CAR as u8;
                        }
                        had_backslash = true;
                    }
                    s_idx += crate::mbyte::utfc_slice2len(&eval_bytes[s_idx..eval_len]);
                }
                let final_cstr = if had_backslash && (flags & REGSUB_BACKSLASH) != 0 {
                    vim_strsave_escaped(&eval_bytes, b"\\")
                } else {
                    CStr::from_bytes_with_nul(&eval_bytes)
                        .unwrap_or_default()
                        .to_owned()
                };
                dst_off += final_cstr.to_bytes().len();
                *eval_slot = Some(final_cstr);
            }

            re_st().can_f_submatch = prev_can_f_submatch;
            if re_st().can_f_submatch {
                re_st().rsm = rsm_save;
            }
        }
    } else if let Some(src) = source {
        let src_bytes = src.to_bytes();
        let mut idx = 0usize;
        while idx < src_bytes.len() {
            let mut c = src_bytes[idx] as i32;
            idx += 1;
            if c == b'&' as i32 && (flags & REGSUB_MAGIC) != 0 {
                no = 0;
            } else if c == b'\\' as i32
                && let Some(&next_b) = src_bytes.get(idx)
            {
                if next_b == b'&' && (flags & REGSUB_MAGIC) == 0 {
                    idx += 1;
                    no = 0;
                } else if next_b.is_ascii_digit() {
                    no = (next_b - b'0') as i32;
                    idx += 1;
                } else if matches!(next_b, b'u' | b'U' | b'l' | b'L' | b'e' | b'E') {
                    idx += 1;
                    match next_b {
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
                if c == i32::from(K_SPECIAL) && idx + 2 <= src_bytes.len() {
                    if let Some(ref mut out) = dest_slice {
                        if dst_off + 3 > max_dst {
                            iemsg(c"vim_regsub_both(): not enough space");
                            return 0;
                        }
                        out[dst_off] = c as u8;
                        out[dst_off + 1] = src_bytes[idx];
                        out[dst_off + 2] = src_bytes[idx + 1];
                    }
                    dst_off += 3;
                    idx += 2;
                    continue;
                }

                if c == b'\\' as i32
                    && let Some(&next_b) = src_bytes.get(idx)
                {
                    match next_b {
                        b'r' => {
                            c = CAR;
                            idx += 1;
                        }
                        b'n' => {
                            c = NL;
                            idx += 1;
                        }
                        b't' => {
                            c = TAB;
                            idx += 1;
                        }
                        b'b' => {
                            c = CTRL_H_CONST;
                            idx += 1;
                        }
                        _ => {
                            if (flags & REGSUB_BACKSLASH) != 0 {
                                if let Some(ref mut out) = dest_slice {
                                    if dst_off + 1 > max_dst {
                                        iemsg(c"vim_regsub_both(): not enough space");
                                        return 0;
                                    }
                                    out[dst_off] = b'\\';
                                }
                                dst_off += 1;
                            }
                            c = next_b as i32;
                            idx += 1;
                        }
                    }
                } else {
                    c = crate::mbyte::utf_slice2char(&src_bytes[idx - 1..]);
                }

                let cc = if let Some(f) = func_one.take() {
                    f.apply(c)
                } else if let Some(f) = func_all {
                    f.apply(c)
                } else {
                    c
                };

                let cur_slice = &src_bytes[idx - 1..];
                let totlen = crate::mbyte::utfc_slice2len(cur_slice);
                let charlen = utf_char2len(cc) as usize;

                if let Some(ref mut out) = dest_slice {
                    if dst_off + charlen > max_dst {
                        iemsg(c"vim_regsub_both(): not enough space");
                        return 0;
                    }
                    crate::mbyte::utf_char2slice(cc, &mut out[dst_off..]);
                }
                dst_off = (dst_off + charlen).wrapping_sub(1);
                let clen = crate::mbyte::utf_slice2len(cur_slice);

                if clen < totlen {
                    let extra_len = totlen - clen;
                    if let Some(ref mut out) = dest_slice {
                        if dst_off + extra_len > max_dst {
                            iemsg(c"vim_regsub_both(): not enough space");
                            return 0;
                        }
                        out[dst_off + 1..dst_off + 1 + extra_len]
                            .copy_from_slice(&cur_slice[clen..totlen]);
                    }
                    dst_off += extra_len;
                }
                idx += totlen - 1;
                dst_off = dst_off.wrapping_add(1);
            } else {
                let mut s: RegLineSlice;
                if reg_multi() {
                    clnum = re_st().rex.mmatch_mut().startpos[no as usize].lnum;
                    if clnum < 0 || re_st().rex.mmatch_mut().endpos[no as usize].lnum < 0 {
                        s = REG_LINE_NULL;
                    } else {
                        let line_ptr = reg_line_from_opt_cstr(reg_getline(clnum));
                        s = line_ptr.wrapping_add(
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
                    let start_no = re_st().rex.match_mut().startp[no as usize];
                    let end_no = re_st().rex.match_mut().endp[no as usize];
                    if start_no.is_null() || end_no.is_null() {
                        s = REG_LINE_NULL;
                    } else {
                        s = start_no;
                        len = end_no.offset_from(start_no) as i32;
                    }
                }
                if !s.is_null() {
                    loop {
                        if len == 0 {
                            if reg_multi() {
                                if re_st().rex.mmatch_mut().endpos[no as usize].lnum == clnum {
                                    break;
                                }
                                if let Some(ref mut out) = dest_slice {
                                    if dst_off + 1 > max_dst {
                                        iemsg(c"vim_regsub_both(): not enough space");
                                        return 0;
                                    }
                                    out[dst_off] = CAR as u8;
                                }
                                dst_off += 1;
                                clnum += 1;
                                s = reg_line_from_opt_cstr(reg_getline(clnum));
                                if re_st().rex.mmatch_mut().endpos[no as usize].lnum == clnum {
                                    len = re_st().rex.mmatch_mut().endpos[no as usize].col;
                                } else {
                                    len = reg_getline_len(clnum);
                                }
                            } else {
                                break;
                            }
                        } else if s.is_empty() {
                            if copy {
                                iemsg(tr(E_RE_DAMG));
                            }
                            return (dst_off + 1) as i32;
                        } else {
                            let s_b0 = s.first_byte();
                            if (flags & REGSUB_BACKSLASH) != 0
                                && (s_b0 == CAR as u8 || s_b0 == b'\\')
                            {
                                if let Some(ref mut out) = dest_slice {
                                    if dst_off + 2 > max_dst {
                                        iemsg(c"vim_regsub_both(): not enough space");
                                        return 0;
                                    }
                                    out[dst_off] = b'\\';
                                    out[dst_off + 1] = s_b0;
                                }
                                dst_off += 2;
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
                                if let Some(ref mut out) = dest_slice {
                                    if dst_off + charlen > max_dst {
                                        iemsg(c"vim_regsub_both(): not enough space");
                                        return 0;
                                    }
                                    crate::mbyte::utf_char2slice(cc, &mut out[dst_off..]);
                                }
                                dst_off += charlen;
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
    if let Some(ref mut out) = dest_slice {
        out[dst_off] = NUL;
    }
    (dst_off + 1) as i32
}

fn reg_getline_submatch_slice(lnum: Linenr) -> Option<&'static [u8]> {
    let (firstlnum, maxline) = reg_getline_bounds(lnum, true);
    if firstlnum < 1 {
        return None;
    }
    if lnum > maxline {
        return Some(b"");
    }
    let line = ml_get_buf(re_st().rex.buf_mut(), firstlnum)
        .as_ptr()
        .cast::<u8>();
    let len = ml_get_buf_len(re_st().rex.buf_mut(), firstlnum) as usize;
    Some(unsafe { std::slice::from_raw_parts(line, len) })
}

pub fn reg_submatch(no: i32) -> Option<std::ffi::CString> {
    if !re_st().can_f_submatch || no < 0 {
        return None;
    }
    let no = no as usize;

    if re_st().rsm.sm_match.is_null() {
        let mut lnum = re_st().rsm.mmatch_mut().startpos[no].lnum;
        let end_lnum = re_st().rsm.mmatch_mut().endpos[no].lnum;
        if lnum < 0 || end_lnum < 0 {
            return None;
        }
        let first_line = reg_getline_submatch_slice(lnum)?;
        let scol = (re_st().rsm.mmatch_mut().startpos[no].col as usize).min(first_line.len());
        let ecol = re_st().rsm.mmatch_mut().endpos[no].col as usize;
        let mut buf: Vec<u8> = Vec::new();
        if end_lnum == lnum {
            let end_idx = ecol.min(first_line.len());
            buf.extend_from_slice(&first_line[scol..end_idx.max(scol)]);
        } else {
            buf.extend_from_slice(&first_line[scol..]);
            buf.push(b'\n');
            lnum += 1;
            while lnum < end_lnum {
                let line = reg_getline_submatch_slice(lnum).unwrap_or(b"");
                buf.extend_from_slice(line);
                buf.push(b'\n');
                lnum += 1;
            }
            let src = reg_getline_submatch_slice(lnum).unwrap_or(b"");
            buf.extend_from_slice(&src[..ecol.min(src.len())]);
        }
        Some(crate::memory::bytes_to_cstring(&buf))
    } else {
        let s = re_st().rsm.match_mut().startp[no];
        let e = re_st().rsm.match_mut().endp[no];
        if s.is_null() || e.is_null() {
            None
        } else {
            cstring_from_bytes(s.span_to(e))
        }
    }
}

pub fn reg_submatch_list(no: i32) -> Option<&'static mut TvList> {
    if !re_st().can_f_submatch || no < 0 {
        return None;
    }
    let no = no as usize;
    let list: &'static mut TvList;

    if re_st().rsm.sm_match.is_null() {
        let slnum = re_st().rsm.mmatch_mut().startpos[no].lnum;
        let elnum = re_st().rsm.mmatch_mut().endpos[no].lnum;
        if slnum < 0 || elnum < 0 {
            return None;
        }

        let scol = re_st().rsm.mmatch_mut().startpos[no].col;
        let ecol = re_st().rsm.mmatch_mut().endpos[no].col;

        list = tv_list_alloc((elnum - slnum + 1) as isize);

        let first_line = reg_getline_submatch_slice(slnum).unwrap_or_else(|| c"".to_bytes());
        let s = &first_line[(scol as usize).min(first_line.len())..];
        if slnum == elnum {
            crate::eval::typval::tv_list_append_bytes(
                list,
                &s[..((ecol - scol) as usize).min(s.len())],
            );
        } else {
            let max_lnum = elnum - slnum;
            crate::eval::typval::tv_list_append_bytes(list, s);
            for i in 1..max_lnum {
                let line = reg_getline_submatch_slice(slnum + i).unwrap_or_else(|| c"".to_bytes());
                crate::eval::typval::tv_list_append_bytes(list, line);
            }
            let last_line = reg_getline_submatch_slice(elnum).unwrap_or_else(|| c"".to_bytes());
            crate::eval::typval::tv_list_append_bytes(
                list,
                &last_line[..(ecol as usize).min(last_line.len())],
            );
        }
    } else {
        let s = re_st().rsm.match_mut().startp[no];
        let e = re_st().rsm.match_mut().endp[no];
        if s.is_null() || e.is_null() {
            return None;
        }
        list = tv_list_alloc(1);
        tv_list_append_string(list, Some(s.as_cstr()), e.offset_from(s) as isize);
    }

    tv_list_ref(Some(list));
    Some(list)
}

pub(super) fn init_regexec_multi(
    rmp: &mut Regmmatch,
    win: Option<&mut WinInfoInfo>,
    buf: &mut FileBuf,
    lnum: Linenr,
) {
    re_st().rex.reg_match = crate::globals::GraphPtr::NULL;
    re_st().rex.reg_mmatch = crate::globals::GraphPtr::from(&mut *rmp);
    re_st().rex.reg_buf = crate::globals::BufPtr::from(buf);
    re_st().rex.reg_win = win.map_or(crate::globals::WinPtr::NULL, crate::globals::WinPtr::from);
    re_st().rex.reg_firstlnum = lnum;
    re_st().rex.reg_maxline = re_st().rex.buf_mut().b_ml.ml_line_count - lnum;
    re_st().rex.reg_line_lbr = false;
    re_st().rex.reg_ic = rmp.rmm_ic;
    re_st().rex.reg_icombine = false;
    re_st().rex.reg_nobreak = (rmp.prog_mut().re_flags & (RE_NOBREAK as u32)) != 0;
    re_st().rex.reg_maxcol = rmp.rmm_maxcol;
}

pub(super) fn get_char_class_bits(slice: &[u8], newl: &mut bool) -> i32 {
    let mut config: i32 = 0;
    *newl = false;
    let Some((&b']', body)) = slice.split_last() else {
        return -1;
    };
    let len = body.len();
    let mut i = 0;
    if body.first() == Some(&b'^') {
        config |= CLASS_NOT;
        i += 1;
    }

    while i < len {
        if i + 2 < len && body[i + 1] == b'-' {
            match body[i] {
                b'0' => {
                    if body[i + 2] == b'9' {
                        config |= CLASS_O9;
                    } else if body[i + 2] == b'7' {
                        config |= CLASS_O7;
                    } else {
                        return -1;
                    }
                }
                b'a' => {
                    if body[i + 2] == b'z' {
                        config |= CLASS_AZ_CONST;
                    } else if body[i + 2] == b'f' {
                        config |= CLASS_AF_CONST;
                    } else {
                        return -1;
                    }
                }
                b'A' => {
                    if body[i + 2] == b'Z' {
                        config |= CLASS_AZ;
                    } else if body[i + 2] == b'F' {
                        config |= CLASS_AF;
                    } else {
                        return -1;
                    }
                }
                _ => return -1,
            }
            i += 3;
        } else if i + 1 < len && body[i] == b'\\' && body[i + 1] == b'n' {
            *newl = true;
            i += 2;
        } else if body[i] == b'_' {
            config |= CLASS_UNDERSCORE;
            i += 1;
        } else if body[i] == b'\n' {
            *newl = true;
            i += 1;
        } else {
            return -1;
        }
    }

    if i != len {
        return -1;
    }
    if (config & (CLASS_AZ_CONST | CLASS_AZ)) != 0 {
        return -1;
    }
    config
}

pub(super) fn regcomp_start(expr: &CStr, re_flags: i32) {
    initchr(expr);
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
    re_st().had_eol = false;
}

pub(super) fn seen_endbrace(refnum: i32) -> bool {
    if re_st().had_endbrace[refnum as usize] == 0 {
        let has_lookbehind = re_st()
            .regparse_bytes()
            .windows(3)
            .any(|w| w[0] == b'@' && w[1] == b'<' && (w[2] == b'!' || w[2] == b'='));
        if !has_lookbehind {
            emsg(tr(c"E65: Illegal back reference"));
            g_st().rc_did_emsg = true;
            return false;
        }
    }
    true
}

pub(super) fn coll_get_char() -> i32 {
    let mut nr: i64 = -1;
    let ch = re_st().regparse_byte();
    re_st().regparse_off += 1;
    match ch {
        b'd' => nr = getdecchrs(),
        b'o' => nr = getoctchrs(),
        b'x' => nr = gethexchrs(2),
        b'u' => nr = gethexchrs(4),
        b'U' => nr = gethexchrs(8),
        _ => {}
    }
    if nr < 0 {
        re_st().regparse_off -= 1;
        nr = i64::from(b'\\');
    }
    if nr > i64::from(i32::MAX) {
        nr = i64::from(i32::MAX);
    }
    nr as i32
}

pub fn vim_regcomp_had_eol() -> bool {
    re_st().had_eol
}

/// Compile a regular expression into internal code.
pub fn vim_regcomp(expr_arg: &CStr, re_flags: i32) -> Option<Box<Regprog>> {
    let mut expr = expr_arg;

    re_st().regexp_engine = opt_vars().p_re as i32;

    let expr_bytes = expr.to_bytes();
    if expr_bytes.starts_with(b"\\%#=") {
        let newengine = expr_bytes.get(4).copied().unwrap_or(0).wrapping_sub(b'0') as i32;
        if newengine == AUTOMATIC_ENGINE
            || newengine == BACKTRACKING_ENGINE
            || newengine == NFA_ENGINE
        {
            re_st().regexp_engine = newengine;
            expr = crate::memory::CStrSubslice::advance(expr, 5);
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
            expr,
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
        bt_regcomp(expr, re_flags).map(|bt| Regprog {
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
        prog_opt = bt_regcomp(expr, re_flags).map(|bt| Regprog {
            regflags: bt.regflags,
            re_engine: re_st().regexp_engine as u32,
            re_flags: re_flags as u32,
            re_in_use: false,
            engine: RegProgEngine::Bt(bt),
        });
    }

    prog_opt.map(Box::new)
}

#[inline(always)]
pub fn vim_regcomp_box(expr_arg: &CStr, re_flags: i32) -> Option<Box<Regprog>> {
    vim_regcomp(expr_arg, re_flags)
}

pub fn free_regexp_stuff() {
    re_st().regstack = Vec::new();
    re_st().backpos = Vec::new();
    re_st().reg_tofree = Vec::new();
    re_st().reg_prev_sub = None;
    re_st().post_vec = Vec::new();
}

fn report_re_switch(pat: &CStr) {
    if opt_vars().p_verbose > 0 {
        verbose_enter();
        msg_puts(tr(c"Switching to backtracking RE engine for pattern: "));
        msg_puts(pat);
        verbose_leave();
    }
}

fn vim_regexec_string(rmp: &mut Regmatch, line: Option<&CStr>, col: Colnr, nl: bool) -> bool {
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

    re_st().rex.reg_startpos = crate::globals::GraphPtr::NULL;
    re_st().rex.reg_endpos = crate::globals::GraphPtr::NULL;

    let mut result = match rmp.prog_mut().engine {
        RegProgEngine::Bt(_) => bt_regexec_nl(rmp, line, col, nl),
        RegProgEngine::Nfa(_) => nfa_regexec_nl(rmp, line, col, nl),
    };
    rmp.prog_mut().re_in_use = false;

    if rmp.prog_mut().re_engine == AUTOMATIC_ENGINE as u32 && result == NFA_TOO_EXPENSIVE {
        let save_p_re = opt_vars().p_re;
        let re_flags = rmp.prog_mut().re_flags as i32;
        let prev_prog = rmp.regprog.borrow_mut();
        let pat_owned = prev_prog.nfa_mut().pattern.clone();

        opt_vars().p_re = OptInt::from(BACKTRACKING_ENGINE);
        report_re_switch(&pat_owned);
        if let Some(new_prog) = vim_regcomp(&pat_owned, re_flags) {
            *prev_prog = *new_prog;
            rmp.prog_mut().re_in_use = true;
            result = match rmp.prog_mut().engine {
                RegProgEngine::Bt(_) => bt_regexec_nl(rmp, line, col, nl),
                RegProgEngine::Nfa(_) => nfa_regexec_nl(rmp, line, col, nl),
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

pub fn vim_regexec_prog(
    prog: Option<&Regprog>,
    ignore_case: bool,
    line: &CStr,
    col: Colnr,
) -> bool {
    let mut regmatch = Regmatch {
        regprog: prog.map_or(crate::globals::GraphPtr::NULL, |p| {
            crate::globals::GraphPtr::new(ptr::from_ref(p).cast_mut())
        }),
        rm_ic: ignore_case,
        ..Regmatch::new()
    };
    vim_regexec_string(&mut regmatch, Some(line), col, false)
}

pub fn vim_regexec(rmp: &mut Regmatch, line: &CStr, col: Colnr) -> bool {
    vim_regexec_string(rmp, Some(line), col, false)
}

pub fn vim_regexec_nl(rmp: &mut Regmatch, line: Option<&CStr>, col: Colnr) -> bool {
    vim_regexec_string(rmp, line, col, true)
}

pub fn vim_regexec_multi(
    rmp: &mut Regmmatch,
    mut win: Option<&mut WinInfoInfo>,
    buf: &mut FileBuf,
    lnum: Linenr,
    col: Colnr,
    tm: Option<Proftime>,
    mut timed_out: Option<&mut i32>,
) -> i32 {
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
            win.as_deref_mut(),
            buf,
            lnum,
            col,
            tm,
            timed_out.as_deref_mut(),
        ),
        RegProgEngine::Nfa(_) => nfa_regexec_multi(
            rmp,
            win.as_deref_mut(),
            buf,
            lnum,
            col,
            tm,
            timed_out.as_deref_mut(),
        ),
    };
    rmp.prog_mut().re_in_use = false;

    if rmp.prog_mut().re_engine == AUTOMATIC_ENGINE as u32 && result == NFA_TOO_EXPENSIVE {
        let save_p_re = opt_vars().p_re;
        let re_flags = rmp.prog_mut().re_flags as i32;
        let prev_prog = rmp.regprog.borrow_mut();
        let pat_owned = prev_prog.nfa_mut().pattern.clone();

        opt_vars().p_re = OptInt::from(BACKTRACKING_ENGINE);

        report_re_switch(&pat_owned);
        g_st().reg_do_extmatch = REX_ALL;
        let new_prog = vim_regcomp(&pat_owned, re_flags);
        g_st().reg_do_extmatch = 0;

        if let Some(new_prog) = new_prog {
            *prev_prog = *new_prog;

            rmp.prog_mut().re_in_use = true;
            result = match rmp.prog_mut().engine {
                RegProgEngine::Bt(_) => bt_regexec_multi(
                    rmp,
                    win.as_deref_mut(),
                    buf,
                    lnum,
                    col,
                    tm,
                    timed_out.as_deref_mut(),
                ),
                RegProgEngine::Nfa(_) => nfa_regexec_multi(rmp, win, buf, lnum, col, tm, timed_out),
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

impl Regexec {
    #[inline(always)]
    pub fn mmatch_mut(&self) -> &'static mut Regmmatch {
        self.reg_mmatch.borrow_mut()
    }
    #[inline(always)]
    pub fn match_mut(&self) -> &'static mut Regmatch {
        self.reg_match.borrow_mut()
    }
    #[inline(always)]
    pub fn buf_mut(&self) -> &'static mut FileBuf {
        self.reg_buf.borrow_mut()
    }
    #[inline(always)]
    pub fn win_mut(&self) -> &'static mut WinInfoInfo {
        self.reg_win.borrow_mut()
    }
    #[inline(always)]
    pub fn load_line(&mut self, lnum: Linenr) {
        self.line = reg_line_from_opt_cstr(reg_getline(lnum));
    }
    #[inline(always)]
    pub fn input_col(&self) -> Colnr {
        self.input_off() as Colnr
    }
    #[inline(always)]
    pub fn input_off(&self) -> usize {
        self.input.offset_from(self.line)
    }
    #[inline(always)]
    pub fn startpos_slice_mut(&self) -> &'static mut [Lpos] {
        self.reg_startpos.borrow_mut()
    }
    #[inline(always)]
    pub fn endpos_slice_mut(&self) -> &'static mut [Lpos] {
        self.reg_endpos.borrow_mut()
    }
}

impl RegexpState {
    #[inline(always)]
    pub fn regparse_bytes(&self) -> &[u8] {
        self.regparse_base.get(self.regparse_off..).unwrap_or(&[])
    }
    #[inline(always)]
    pub fn regparse_byte(&self) -> u8 {
        self.regparse_bytes().first().copied().unwrap_or(0)
    }
    #[inline(always)]
    pub fn regparse_byte_at(&self, idx: usize) -> u8 {
        self.regparse_bytes().get(idx).copied().unwrap_or(0)
    }
    #[inline(always)]
    pub fn regparse_mb_adv(&mut self) {
        let step = utfc_ptr2len(self.regparse_bytes()) as usize;
        self.regparse_off += step;
    }
    #[inline(always)]
    pub fn regparse_prev_char(&mut self) {
        let base = &self.regparse_base;
        self.regparse_off -=
            1 + crate::mbyte::utf_head_off_slice(base, self.regparse_off - 1) as usize;
    }
    #[inline(always)]
    pub fn regparse_get_char_class(&mut self) -> i32 {
        let bytes = self.regparse_bytes();
        let bytes_len = bytes.len();
        let mut rem = bytes;
        let res = get_char_class(&mut rem);
        let rem_len = rem.len();
        self.regparse_off += bytes_len - rem_len;
        res
    }
    #[inline(always)]
    pub fn regparse_get_equi_class(&mut self) -> i32 {
        let bytes = self.regparse_bytes();
        let bytes_len = bytes.len();
        let mut rem = bytes;
        let res = get_equi_class(&mut rem);
        let rem_len = rem.len();
        self.regparse_off += bytes_len - rem_len;
        res
    }
    #[inline(always)]
    pub fn regparse_get_coll_element(&mut self) -> i32 {
        let bytes = self.regparse_bytes();
        let bytes_len = bytes.len();
        let mut rem = bytes;
        let res = get_coll_element(&mut rem);
        let rem_len = rem.len();
        self.regparse_off += bytes_len - rem_len;
        res
    }
}

impl Regsubmatch {
    #[inline(always)]
    pub fn mmatch_mut(&self) -> &'static mut Regmmatch {
        self.sm_mmatch.borrow_mut()
    }
    #[inline(always)]
    pub fn match_mut(&self) -> &'static mut Regmatch {
        self.sm_match.borrow_mut()
    }
}

impl Regmmatch {
    #[inline(always)]
    pub fn prog_mut(&self) -> &'static mut Regprog {
        self.regprog.borrow_mut()
    }
}

impl Regmatch {
    #[inline(always)]
    pub fn prog_mut(&self) -> &'static mut Regprog {
        self.regprog.borrow_mut()
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
        let mut newl = false;
        let digits = b"0-9]";
        assert_eq!(get_char_class_bits(digits, &mut newl), CLASS_O9);
        assert!(!newl);

        let hex_nl = b"^0-9a-fA-F\\n]";
        assert_eq!(
            get_char_class_bits(hex_nl, &mut newl),
            CLASS_NOT | CLASS_O9 | CLASS_AF_CONST | CLASS_AF
        );
        assert!(newl);
    }
}
