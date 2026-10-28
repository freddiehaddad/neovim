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

pub type LpegByte = u8;
pub type LpegUint = c_uint;
pub type LpegIndex = LpegUint;
#[allow(non_camel_case_types)]
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
    pub cs: [LpegByte; CHARSETSIZE],
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
pub fn fillset(s: &mut [LpegByte; CHARSETSIZE], c: LpegByte) {
    s.fill(c);
}

#[inline]
pub fn clearset(s: &mut [LpegByte; CHARSETSIZE]) {
    s.fill(0);
}

#[inline]
pub const fn bytes2slots(n: usize) -> usize {
    (n - 1) / size_of::<TTree>() + 1
}

#[inline]
pub fn setchar(cs: &mut [LpegByte; CHARSETSIZE], b: usize) {
    cs[b >> 3] |= 1u8 << (b & 7);
}

#[inline]
pub fn testchar(st: *const LpegByte, c: LpegUint) -> c_int {
    unsafe { (((*st.add((c >> 3) as usize) as LpegUint) >> (c & 7)) & 1) as c_int }
}

pub const MAXOFF: c_int = 0xF;
pub const MAXAUX: c_int = 0xFF;
pub const MAXBEHIND: c_int = MAXAUX;

#[inline]
pub const fn instsize(l: c_int) -> c_int {
    (l as usize).div_ceil(size_of::<Instruction>()) as c_int
}

pub type CapKind = LpegByte;
pub const CCLOSE: CapKind = 0;
pub const CPOSITION: CapKind = 1;
pub const CCONST: CapKind = 2;
pub const CBACKREF: CapKind = 3;
pub const CARG: CapKind = 4;
pub const CSIMPLE: CapKind = 5;
pub const CTABLE: CapKind = 6;
pub const CFUNCTION: CapKind = 7;
pub const CACC: CapKind = 8;
pub const CQUERY: CapKind = 9;
pub const CSTRING: CapKind = 10;
pub const CNUM: CapKind = 11;
pub const CSUBST: CapKind = 12;
pub const CFOLD: CapKind = 13;
pub const CRUNTIME: CapKind = 14;
pub const CGROUP: CapKind = 15;

pub const MAXINDT: LpegIndex = !0;
pub const MAXLOP: c_int = 20;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Capture {
    pub index: LpegIndex,
    pub idx: c_ushort,
    pub kind: LpegByte,
    pub siz: LpegByte,
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
    pub lstate: *mut lua_State,
    pub ptop: c_int,
    pub firstcap: c_int,
    pub s: *const c_char,
    pub valuecached: c_int,
    pub reclevel: c_int,
}

#[inline]
pub fn captype(cap: *const Capture) -> LpegByte {
    unsafe { (*cap).kind }
}

#[inline]
pub fn isclosecap(cap: *const Capture) -> bool {
    captype(cap) == CCLOSE
}

#[inline]
pub fn isopencap(cap: &Capture) -> bool {
     cap.siz == 0
}

#[inline]
pub fn capinside(c1: &Capture, c2: *const Capture) -> bool {
    unsafe {
        if isopencap( c1) {
            !isclosecap(c2)
        } else {
            (*c2).index < c1.index + LpegIndex::from(c1.siz) - 1
        }
    }
}

pub type TTag = LpegByte;
pub const TCHAR: TTag = 0;
pub const TSET: TTag = 1;
pub const TANY: TTag = 2;
pub const TTRUE: TTag = 3;
pub const TFALSE: TTag = 4;
pub const TUTFR: TTag = 5;
pub const TREP: TTag = 6;
pub const TSEQ: TTag = 7;
pub const TCHOICE: TTag = 8;
pub const TNOT: TTag = 9;
pub const TAND: TTag = 10;
pub const TCALL: TTag = 11;
pub const TOPEN_CALL: TTag = 12;
pub const TRULE: TTag = 13;
pub const TXINFO: TTag = 14;
pub const TGRAMMAR: TTag = 15;
pub const TBEHIND: TTag = 16;
pub const TCAPTURE: TTag = 17;
pub const TRUN_TIME: TTag = 18;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct TTreeSet {
    pub offset: LpegByte,
    pub size: LpegByte,
    pub deflt: LpegByte,
    pub bitmap: [LpegByte; 1],
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
    pub tag: LpegByte,
    pub cap: LpegByte,
    pub key: c_ushort,
    pub u: TTreeU,
}

#[inline]
pub const fn ttree_bitmap_offset() -> usize {
    offset_of!(TTree, u) + offset_of!(TTreeSet, bitmap)
}

#[inline]
pub fn treebuffer(t: *mut TTree) -> *mut LpegByte {
    unsafe { t.cast::<LpegByte>().add(ttree_bitmap_offset()) }
}

#[inline]
pub fn sib1(t: *mut TTree) -> *mut TTree {
    unsafe { t.add(1) }
}

#[inline]
pub fn sib2(t: *mut TTree) -> *mut TTree {
    unsafe { t.offset((*t).u.ps as isize) }
}

#[repr(C)]
pub struct Pattern {
    pub code: Vec<Instruction>,
    pub tree: [TTree; 1],
}

pub const NUMSIBLINGS: [LpegByte; 19] = [
    0, 0, 0, // char, set, any
    0, 0, 0, // true, false, utf-range
    1, // rep
    2, 2, // seq, choice
    1, 1, // not, and
    0, 0, 2, 1, 1, // call, opencall, rule, xinfo, grammar
    1, // behind
    1, 1, // capture, runtime capture
];

pub type Opcode = LpegByte;
pub const IANY: Opcode = 0;
pub const ICHAR: Opcode = 1;
pub const ISET: Opcode = 2;
pub const ITEST_ANY: Opcode = 3;
pub const ITEST_CHAR: Opcode = 4;
pub const ITEST_SET: Opcode = 5;
pub const ISPAN: Opcode = 6;
pub const IUTFR: Opcode = 7;
pub const IBEHIND: Opcode = 8;
pub const IRET: Opcode = 9;
pub const IEND: Opcode = 10;
pub const ICHOICE: Opcode = 11;
pub const IJMP: Opcode = 12;
pub const ICALL: Opcode = 13;
pub const IOPEN_CALL: Opcode = 14;
pub const ICOMMIT: Opcode = 15;
pub const IPARTIAL_COMMIT: Opcode = 16;
pub const IBACK_COMMIT: Opcode = 17;
pub const IFAIL_TWICE: Opcode = 18;
pub const IFAIL: Opcode = 19;
pub const IGIVEUP: Opcode = 20;
pub const IFULL_CAPTURE: Opcode = 21;
pub const IOPEN_CAPTURE: Opcode = 22;
pub const ICLOSE_CAPTURE: Opcode = 23;
pub const ICLOSE_RUN_TIME: Opcode = 24;
pub const IEMPTY: Opcode = 25;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct InstAux2Set {
    pub offset: LpegByte,
    pub size: LpegByte,
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
    pub code: LpegByte,
    pub aux1: LpegByte,
    pub aux2: InstAux2,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union Instruction {
    pub i: Inst,
    pub offset: c_int,
    pub buff: [LpegByte; 1],
}

#[inline]
pub fn getkind(op: &Instruction) -> c_int {
    unsafe { c_int::from(op.i.aux1 & 0xF) }
}

#[inline]
pub fn getoff(op: &Instruction) -> c_int {
    unsafe { c_int::from((op.i.aux1 >> 4) & 0xF) }
}

#[inline]
pub const fn joinkindoff(k: c_int, o: c_int) -> c_int {
    k | (o << 4)
}

#[inline]
pub fn utf_to(inst: &Instruction) -> c_int {
    unsafe { (c_int::from(inst.i.aux2.key) << 8) | c_int::from(inst.i.aux1) }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CharsetInfo {
    pub cs: *const LpegByte,
    pub offset: c_int,
    pub size: c_int,
    pub deflt: c_int,
}

impl CharsetInfo {
    pub const fn new() -> Self {
        Self {
            cs: ptr::null(),
            offset: 0,
            size: 0,
            deflt: 0,
        }
    }
}

impl Default for CharsetInfo {
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
        let mut cs = [0u8; CHARSETSIZE];
        let mut info = CharsetInfo::new();
        assert_eq!(charsettype(&cs, &mut info), IFAIL);

        setchar(&mut cs, b'A' as usize);
        assert_eq!(testchar(cs.as_ptr(), b'A' as LpegUint), 1);
        assert_eq!(charsettype(&cs, &mut info), ICHAR);
        assert_eq!(info.offset, c_int::from(b'A'));

        fillset(&mut cs, 0xFF);
        assert_eq!(charsettype(&cs, &mut info), IANY);
    }
}
