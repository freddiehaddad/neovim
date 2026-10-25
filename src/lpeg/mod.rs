//! LPeg - PEG pattern matching for Lua (`src/lpeg/`).

pub mod lpcap;
pub mod lpcode;
pub mod lpcset;
pub mod lptree;
pub mod lpvm;

use std::ffi::{CStr, c_char, c_int, c_short, c_uint, c_ushort};
use std::mem::{offset_of, size_of};
use std::ptr;

use crate::lua::converter::{lua_Integer, lua_Number, lua_State, luaL_Reg};

pub type byte = u8;
pub type uint = c_uint;
pub type Index_t = uint;
pub type lua_Unsigned = usize;

pub const PATTERN_T: &CStr = c"lpeg-pattern";
pub const MAXSTACKIDX: &CStr = c"lpeg-maxstack";

pub const MAXBACK: c_int = 400;
pub const MAXRULES: usize = 1000;
pub const INITCAPSIZE: usize = 32;
pub const SUBJIDX: c_int = 2;
pub const FIXEDARGS: c_int = 3;

#[inline]
pub const fn caplistidx(ptop: c_int) -> c_int {
    ptop + 2
}

#[inline]
pub const fn ktableidx(ptop: c_int) -> c_int {
    ptop + 3
}

#[inline]
pub const fn stackidx(ptop: c_int) -> c_int {
    ptop + 4
}

pub const BITSPERCHAR: usize = 8;
pub const CHARSETSIZE: usize = (u8::MAX as usize / BITSPERCHAR) + 1;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Charset {
    pub cs: [byte; CHARSETSIZE],
}

impl Charset {
    pub const fn new() -> Self {
        Self {
            cs: [0; CHARSETSIZE],
        }
    }
}

impl Default for Charset {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub unsafe fn fillset(s: *mut byte, c: byte) {
    unsafe {
        ptr::write_bytes(s, c, CHARSETSIZE);
    }
}

#[inline]
pub unsafe fn clearset(s: *mut byte) {
    unsafe {
        fillset(s, 0);
    }
}

#[inline]
pub const fn bytes2slots(n: usize) -> usize {
    (n - 1) / size_of::<TTree>() + 1
}

#[inline]
pub unsafe fn setchar(cs: *mut byte, b: usize) {
    unsafe {
        *cs.add(b >> 3) |= 1u8 << (b & 7);
    }
}

#[inline]
pub unsafe fn testchar(st: *const byte, c: uint) -> c_int {
    unsafe { (((*st.add((c >> 3) as usize) as uint) >> (c & 7)) & 1) as c_int }
}

pub const MAXOFF: c_int = 0xF;
pub const MAXAUX: c_int = 0xFF;
pub const MAXBEHIND: c_int = MAXAUX;

#[inline]
pub const fn instsize(l: c_int) -> c_int {
    (l as usize).div_ceil(size_of::<Instruction>()) as c_int
}

pub type CapKind = byte;
pub const Cclose: CapKind = 0;
pub const Cposition: CapKind = 1;
pub const Cconst: CapKind = 2;
pub const Cbackref: CapKind = 3;
pub const Carg: CapKind = 4;
pub const Csimple: CapKind = 5;
pub const Ctable: CapKind = 6;
pub const Cfunction: CapKind = 7;
pub const Cacc: CapKind = 8;
pub const Cquery: CapKind = 9;
pub const Cstring: CapKind = 10;
pub const Cnum: CapKind = 11;
pub const Csubst: CapKind = 12;
pub const Cfold: CapKind = 13;
pub const Cruntime: CapKind = 14;
pub const Cgroup: CapKind = 15;

pub const MAXINDT: Index_t = !0;
pub const MAXLOP: c_int = 20;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Capture {
    pub index: Index_t,
    pub idx: c_ushort,
    pub kind: byte,
    pub siz: byte,
}

impl Capture {
    pub const fn new() -> Self {
        Self {
            index: 0,
            idx: 0,
            kind: 0,
            siz: 0,
        }
    }
}

impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct CapState {
    pub cap: *mut Capture,
    pub ocap: *mut Capture,
    pub L: *mut lua_State,
    pub ptop: c_int,
    pub firstcap: c_int,
    pub s: *const c_char,
    pub valuecached: c_int,
    pub reclevel: c_int,
}

#[inline]
pub unsafe fn captype(cap: *const Capture) -> byte {
    unsafe { (*cap).kind }
}

#[inline]
pub unsafe fn isclosecap(cap: *const Capture) -> bool {
    unsafe { captype(cap) == Cclose }
}

#[inline]
pub unsafe fn isopencap(cap: *const Capture) -> bool {
    unsafe { (*cap).siz == 0 }
}

#[inline]
pub unsafe fn capinside(c1: *const Capture, c2: *const Capture) -> bool {
    unsafe {
        if isopencap(c1) {
            !isclosecap(c2)
        } else {
            (*c2).index < (*c1).index + Index_t::from((*c1).siz) - 1
        }
    }
}

pub type TTag = byte;
pub const TChar: TTag = 0;
pub const TSet: TTag = 1;
pub const TAny: TTag = 2;
pub const TTrue: TTag = 3;
pub const TFalse: TTag = 4;
pub const TUTFR: TTag = 5;
pub const TRep: TTag = 6;
pub const TSeq: TTag = 7;
pub const TChoice: TTag = 8;
pub const TNot: TTag = 9;
pub const TAnd: TTag = 10;
pub const TCall: TTag = 11;
pub const TOpenCall: TTag = 12;
pub const TRule: TTag = 13;
pub const TXInfo: TTag = 14;
pub const TGrammar: TTag = 15;
pub const TBehind: TTag = 16;
pub const TCapture: TTag = 17;
pub const TRunTime: TTag = 18;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct TTreeSet {
    pub offset: byte,
    pub size: byte,
    pub deflt: byte,
    pub bitmap: [byte; 1],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union TTreeU {
    pub ps: c_int,
    pub n: c_int,
    pub set: TTreeSet,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct TTree {
    pub tag: byte,
    pub cap: byte,
    pub key: c_ushort,
    pub u: TTreeU,
}

#[inline]
pub const fn ttree_bitmap_offset() -> usize {
    offset_of!(TTree, u) + offset_of!(TTreeSet, bitmap)
}

#[inline]
pub unsafe fn treebuffer(t: *mut TTree) -> *mut byte {
    unsafe { t.cast::<byte>().add(ttree_bitmap_offset()) }
}

#[inline]
pub unsafe fn sib1(t: *mut TTree) -> *mut TTree {
    unsafe { t.add(1) }
}

#[inline]
pub unsafe fn sib2(t: *mut TTree) -> *mut TTree {
    unsafe { t.offset((*t).u.ps as isize) }
}

#[repr(C)]
pub struct Pattern {
    pub code: Vec<Instruction>,
    pub tree: [TTree; 1],
}

pub const NUMSIBLINGS: [byte; 19] = [
    0, 0, 0, // char, set, any
    0, 0, 0, // true, false, utf-range
    1, // rep
    2, 2, // seq, choice
    1, 1, // not, and
    0, 0, 2, 1, 1, // call, opencall, rule, xinfo, grammar
    1, // behind
    1, 1, // capture, runtime capture
];

pub type Opcode = byte;
pub const IAny: Opcode = 0;
pub const IChar: Opcode = 1;
pub const ISet: Opcode = 2;
pub const ITestAny: Opcode = 3;
pub const ITestChar: Opcode = 4;
pub const ITestSet: Opcode = 5;
pub const ISpan: Opcode = 6;
pub const IUTFR: Opcode = 7;
pub const IBehind: Opcode = 8;
pub const IRet: Opcode = 9;
pub const IEnd: Opcode = 10;
pub const IChoice: Opcode = 11;
pub const IJmp: Opcode = 12;
pub const ICall: Opcode = 13;
pub const IOpenCall: Opcode = 14;
pub const ICommit: Opcode = 15;
pub const IPartialCommit: Opcode = 16;
pub const IBackCommit: Opcode = 17;
pub const IFailTwice: Opcode = 18;
pub const IFail: Opcode = 19;
pub const IGiveup: Opcode = 20;
pub const IFullCapture: Opcode = 21;
pub const IOpenCapture: Opcode = 22;
pub const ICloseCapture: Opcode = 23;
pub const ICloseRunTime: Opcode = 24;
pub const IEmpty: Opcode = 25;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct InstAux2Set {
    pub offset: byte,
    pub size: byte,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union InstAux2 {
    pub key: c_short,
    pub set: InstAux2Set,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Inst {
    pub code: byte,
    pub aux1: byte,
    pub aux2: InstAux2,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union Instruction {
    pub i: Inst,
    pub offset: c_int,
    pub buff: [byte; 1],
}

#[inline]
pub unsafe fn getkind(op: *const Instruction) -> c_int {
    unsafe { c_int::from((*op).i.aux1 & 0xF) }
}

#[inline]
pub unsafe fn getoff(op: *const Instruction) -> c_int {
    unsafe { c_int::from(((*op).i.aux1 >> 4) & 0xF) }
}

#[inline]
pub const fn joinkindoff(k: c_int, o: c_int) -> c_int {
    k | (o << 4)
}

#[inline]
pub unsafe fn utf_to(inst: *const Instruction) -> c_int {
    unsafe { (c_int::from((*inst).i.aux2.key) << 8) | c_int::from((*inst).i.aux1) }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct charsetinfo {
    pub cs: *const byte,
    pub offset: c_int,
    pub size: c_int,
    pub deflt: c_int,
}

impl charsetinfo {
    pub const fn new() -> Self {
        Self {
            cs: ptr::null(),
            offset: 0,
            size: 0,
            deflt: 0,
        }
    }
}

impl Default for charsetinfo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lpeg::lpcset::charsettype;

    #[test]
    fn test_charsettype_and_helpers() {
        unsafe {
            let mut cs = [0u8; CHARSETSIZE];
            let mut info = charsetinfo::new();
            assert_eq!(charsettype(cs.as_ptr(), &mut info), IFail);

            setchar(cs.as_mut_ptr(), b'A' as usize);
            assert_eq!(testchar(cs.as_ptr(), b'A' as uint), 1);
            assert_eq!(charsettype(cs.as_ptr(), &mut info), IChar);
            assert_eq!(info.offset, c_int::from(b'A'));

            fillset(cs.as_mut_ptr(), 0xFF);
            assert_eq!(charsettype(cs.as_ptr(), &mut info), IAny);
        }
    }
}
