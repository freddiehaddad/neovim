//! LPeg pattern analysis and bytecode compiler (`src/lpeg/lpcode.c`).
#![forbid(unsafe_code)]

use std::ffi::CStr;

use crate::lpeg::lpcset::{charsettype, getbytefromcharset, tocharset, tree2cset};
use crate::lpeg::{
    CCLOSE, CGROUP, CHARSETSIZE, Charset, CharsetInfo, IANY, IBACK_COMMIT, IBEHIND, ICALL, ICHAR,
    ICHOICE, ICLOSE_CAPTURE, ICLOSE_RUN_TIME, ICOMMIT, IEMPTY, IEND, IFAIL, IFAIL_TWICE,
    IFULL_CAPTURE, IJMP, INST_BUFF_SIZE, IOPEN_CALL, IOPEN_CAPTURE, IPARTIAL_COMMIT, IRET, ISET,
    ISPAN, ITEST_ANY, ITEST_CHAR, ITEST_SET, IUTFR, Instruction, LpegByte, LpegUint, MAXBEHIND,
    MAXOFF, MAXRULES, NUMSIBLINGS, Opcode, Pattern, TAND, TANY, TBEHIND, TCALL, TCAPTURE, TCHAR,
    TCHOICE, TFALSE, TGRAMMAR, TNOT, TOPEN_CALL, TREP, TRULE, TRUN_TIME, TSEQ, TSET, TTRUE, TTree,
    TUTFR, TXINFO, clearset, instsize, joinkindoff, setchar, sib1_idx, sib2_idx,
};

pub const PENULLABLE: i32 = 0;
pub const PENOFAIL: i32 = 1;

const NOINST: i32 = -1;

static FULLSET: Charset = Charset { cs: [0xFF; 32] };

#[inline]
pub fn nofail(tree: &mut [TTree], idx: usize) -> bool {
    checkaux(tree, idx, PENOFAIL) != 0
}

#[inline]
pub fn nullable(tree: &mut [TTree], idx: usize) -> bool {
    checkaux(tree, idx, PENULLABLE) != 0
}

fn cs_complement(cs: &mut Charset) {
    for b in &mut cs.cs {
        *b = !*b;
    }
}

fn cs_disjoint(cs1: &Charset, cs2: &Charset) -> bool {
    for i in 0..CHARSETSIZE {
        if (cs1.cs[i] & cs2.cs[i]) != 0 {
            return false;
        }
    }
    true
}

fn callrecursive(
    tree: &mut [TTree],
    idx: usize,
    f: fn(&mut [TTree], usize) -> i32,
    def: i32,
) -> i32 {
    let key = tree[idx].key;
    debug_assert_eq!(tree[idx].tag, TCALL);
    let s2 = sib2_idx(tree, idx);
    debug_assert_eq!(tree[s2].tag, TRULE);
    if key == 0 {
        def
    } else {
        tree[idx].key = 0;
        let result = f(tree, s2);
        tree[idx].key = key;
        result
    }
}

pub fn hascaptures(tree: &mut [TTree], mut idx: usize) -> i32 {
    loop {
        match tree[idx].tag {
            TCAPTURE | TRUN_TIME => return 1,
            TCALL => return callrecursive(tree, idx, hascaptures, 0),
            TRULE => {
                idx = sib1_idx(idx);
            }
            TOPEN_CALL => {
                debug_assert!(false);
                return 0;
            }
            tag => match NUMSIBLINGS[tag as usize] {
                1 => {
                    idx = sib1_idx(idx);
                }
                2 => {
                    if hascaptures(tree, sib1_idx(idx)) != 0 {
                        return 1;
                    }
                    idx = sib2_idx(tree, idx);
                }
                _ => {
                    debug_assert_eq!(NUMSIBLINGS[tag as usize], 0);
                    return 0;
                }
            },
        }
    }
}

pub fn checkaux(tree: &mut [TTree], mut idx: usize, pred: i32) -> i32 {
    loop {
        match tree[idx].tag {
            TCHAR | TSET | TANY | TUTFR | TFALSE | TOPEN_CALL => return 0,
            TREP | TTRUE => return 1,
            TNOT | TBEHIND => {
                return if pred == PENOFAIL { 0 } else { 1 };
            }
            TAND => {
                if pred == PENULLABLE {
                    return 1;
                }
                idx = sib1_idx(idx);
            }
            TRUN_TIME => {
                if pred == PENOFAIL {
                    return 0;
                }
                idx = sib1_idx(idx);
            }
            TSEQ => {
                if checkaux(tree, sib1_idx(idx), pred) == 0 {
                    return 0;
                }
                idx = sib2_idx(tree, idx);
            }
            TCHOICE => {
                if checkaux(tree, sib2_idx(tree, idx), pred) != 0 {
                    return 1;
                }
                idx = sib1_idx(idx);
            }
            TCAPTURE | TGRAMMAR | TRULE | TXINFO => {
                idx = sib1_idx(idx);
            }
            TCALL => {
                idx = sib2_idx(tree, idx);
            }
            _ => {
                debug_assert!(false);
                return 0;
            }
        }
    }
}

pub fn fixedlen(tree: &mut [TTree], mut idx: usize) -> i32 {
    let mut len: i32 = 0;
    loop {
        match tree[idx].tag {
            TCHAR | TSET | TANY => return len + 1,
            TUTFR => {
                let s1 = sib1_idx(idx);
                return if tree[idx].cap == tree[s1].cap {
                    len + i32::from(tree[idx].cap)
                } else {
                    -1
                };
            }
            TFALSE | TTRUE | TNOT | TAND | TBEHIND => return len,
            TREP | TRUN_TIME | TOPEN_CALL => return -1,
            TCAPTURE | TRULE | TGRAMMAR | TXINFO => {
                idx = sib1_idx(idx);
            }
            TCALL => {
                let n1 = callrecursive(tree, idx, fixedlen, -1);
                return if n1 < 0 { -1 } else { len + n1 };
            }
            TSEQ => {
                let n1 = fixedlen(tree, sib1_idx(idx));
                if n1 < 0 {
                    return -1;
                }
                len += n1;
                idx = sib2_idx(tree, idx);
            }
            TCHOICE => {
                let n1 = fixedlen(tree, sib1_idx(idx));
                let n2 = fixedlen(tree, sib2_idx(tree, idx));
                return if n1 != n2 || n1 < 0 { -1 } else { len + n1 };
            }
            _ => {
                debug_assert!(false);
                return 0;
            }
        }
    }
}

fn getfirst(
    tree: &mut [TTree],
    mut idx: usize,
    mut follow: &Charset,
    firstset: &mut Charset,
) -> i32 {
    loop {
        match tree[idx].tag {
            TCHAR | TSET | TANY | TFALSE => {
                tocharset(&tree[idx], firstset);
                return 0;
            }
            TUTFR => {
                clearset(&mut firstset.cs);
                let start = usize::from(tree[idx].key);
                let end = usize::from(tree[sib1_idx(idx)].key);
                for c in start..=end {
                    setchar(&mut firstset.cs, c);
                }
                return 0;
            }
            TTRUE => {
                firstset.cs = follow.cs;
                return 1;
            }
            TCHOICE => {
                let mut csaux = Charset::new();
                let e1 = getfirst(tree, sib1_idx(idx), follow, firstset);
                let e2 = getfirst(tree, sib2_idx(tree, idx), follow, &mut csaux);
                for i in 0..CHARSETSIZE {
                    firstset.cs[i] |= csaux.cs[i];
                }
                return e1 | e2;
            }
            TSEQ => {
                if !nullable(tree, sib1_idx(idx)) {
                    idx = sib1_idx(idx);
                    follow = &FULLSET;
                } else {
                    let mut csaux = Charset::new();
                    let e2 = getfirst(tree, sib2_idx(tree, idx), follow, &mut csaux);
                    let e1 = getfirst(tree, sib1_idx(idx), &csaux, firstset);
                    return if e1 == 0 {
                        0
                    } else if ((e1 | e2) & 2) != 0 {
                        2
                    } else {
                        e2
                    };
                }
            }
            TREP => {
                getfirst(tree, sib1_idx(idx), follow, firstset);
                for i in 0..CHARSETSIZE {
                    firstset.cs[i] |= follow.cs[i];
                }
                return 1;
            }
            TCAPTURE | TGRAMMAR | TRULE | TXINFO => {
                idx = sib1_idx(idx);
            }
            TRUN_TIME => {
                let e = getfirst(tree, sib1_idx(idx), &FULLSET, firstset);
                return if e != 0 { 2 } else { 0 };
            }
            TCALL => {
                idx = sib2_idx(tree, idx);
            }
            TAND => {
                let e = getfirst(tree, sib1_idx(idx), follow, firstset);
                for i in 0..CHARSETSIZE {
                    firstset.cs[i] &= follow.cs[i];
                }
                return e;
            }
            TNOT => {
                let s1 = sib1_idx(idx);
                if tocharset(&tree[s1], firstset) != 0 {
                    cs_complement(firstset);
                    return 1;
                }
                let e = getfirst(tree, s1, follow, firstset);
                firstset.cs = follow.cs;
                return e | 1;
            }
            TBEHIND => {
                let e = getfirst(tree, sib1_idx(idx), follow, firstset);
                firstset.cs = follow.cs;
                return e | 1;
            }
            _ => {
                debug_assert!(false);
                return 0;
            }
        }
    }
}

fn headfail(tree: &mut [TTree], mut idx: usize) -> bool {
    loop {
        match tree[idx].tag {
            TCHAR | TSET | TANY | TFALSE => return true,
            TTRUE | TREP | TRUN_TIME | TNOT | TBEHIND | TUTFR => return false,
            TCAPTURE | TGRAMMAR | TRULE | TXINFO | TAND => {
                idx = sib1_idx(idx);
            }
            TCALL => {
                idx = sib2_idx(tree, idx);
            }
            TSEQ => {
                if !nofail(tree, sib2_idx(tree, idx)) {
                    return false;
                }
                idx = sib1_idx(idx);
            }
            TCHOICE => {
                if !headfail(tree, sib1_idx(idx)) {
                    return false;
                }
                idx = sib2_idx(tree, idx);
            }
            _ => {
                debug_assert!(false);
                return false;
            }
        }
    }
}

fn needfollow(tree: &[TTree], mut idx: usize) -> bool {
    loop {
        match tree[idx].tag {
            TCHAR | TSET | TANY | TUTFR | TFALSE | TTRUE | TAND | TNOT | TRUN_TIME | TGRAMMAR
            | TCALL | TBEHIND => return false,
            TCHOICE | TREP => return true,
            TCAPTURE => {
                idx = sib1_idx(idx);
            }
            TSEQ => {
                idx = sib2_idx(tree, idx);
            }
            _ => {
                debug_assert!(false);
                return false;
            }
        }
    }
}

pub fn sizei(i: &Instruction) -> i32 {
    match i.code {
        ISET | ISPAN => 1 + i32::from(i.set_size),
        ITEST_SET => 2 + i32::from(i.set_size),
        ITEST_CHAR | ITEST_ANY | ICHOICE | IJMP | ICALL | IOPEN_CALL | ICOMMIT
        | IPARTIAL_COMMIT | IBACK_COMMIT | IUTFR => 2,
        _ => 1,
    }
}

struct CompileState {
    code: Vec<Instruction>,
    ncode: i32,
    err: Option<&'static CStr>,
}

fn nextinstruction(compst: &mut CompileState, n: i32) -> i32 {
    let size = compst.code.len() as i32;
    let ncode = compst.ncode;
    if ncode > size - n {
        let nsize = (size as LpegUint) + ((size as LpegUint) >> 1) + (n as LpegUint);
        if nsize >= (i32::MAX as LpegUint) {
            compst.err = Some(c"pattern code too large");
            compst.code.resize((ncode + n) as usize, Instruction::new());
        } else {
            compst.code.resize(nsize as usize, Instruction::new());
        }
    }
    compst.ncode = ncode + n;
    ncode
}

#[inline]
fn getinstr(cs: &mut CompileState, i: i32) -> &mut Instruction {
    &mut cs.code[i as usize]
}

fn addinstruction(compst: &mut CompileState, op: Opcode, aux: i32) -> i32 {
    let i = nextinstruction(compst, 1);
    let inst = getinstr(compst, i);
    inst.code = op;
    inst.aux1 = aux as LpegByte;
    i
}

fn addoffsetinst(compst: &mut CompileState, op: Opcode) -> i32 {
    let i = addinstruction(compst, op, 0);
    addinstruction(compst, 0, 0);
    debug_assert!(op == ITEST_SET || sizei(getinstr(compst, i)) == 2);
    i
}

#[inline]
fn setoffset(compst: &mut CompileState, instruction: i32, offset: i32) {
    getinstr(compst, instruction + 1).offset = offset;
}

fn codeutfr(compst: &mut CompileState, tree: &[TTree], idx: usize) {
    let i = addoffsetinst(compst, IUTFR);
    let s1 = sib1_idx(idx);
    let to = tree[s1].u_n;
    debug_assert_eq!(tree[s1].tag, TXINFO);
    getinstr(compst, i + 1).offset = tree[idx].u_n;
    let inst = getinstr(compst, i);
    inst.aux1 = (to & 0xff) as LpegByte;
    inst.key = (to >> 8) as i16;
}

fn addinstcap(compst: &mut CompileState, op: Opcode, cap: i32, key: i32, aux: i32) -> i32 {
    let i = addinstruction(compst, op, joinkindoff(cap, aux));
    getinstr(compst, i).key = key as i16;
    i
}

#[inline]
fn target(code: &[Instruction], i: i32) -> i32 {
    i + code[(i + 1) as usize].offset
}

#[inline]
fn jumptothere(compst: &mut CompileState, instruction: i32, target_pos: i32) {
    if instruction >= 0 {
        setoffset(compst, instruction, target_pos - instruction);
    }
}

#[inline]
fn jumptohere(compst: &mut CompileState, instruction: i32) {
    let ncode = compst.ncode;
    jumptothere(compst, instruction, ncode);
}

fn codechar(compst: &mut CompileState, c: i32, tt: i32) {
    if tt >= 0
        && compst.code[tt as usize].code == ITEST_CHAR
        && i32::from(compst.code[tt as usize].aux1) == c
    {
        addinstruction(compst, IANY, 0);
    } else {
        addinstruction(compst, ICHAR, c);
    }
}

fn addcharset(compst: &mut CompileState, inst: i32, info: &CharsetInfo) {
    let isize = instsize(info.size);
    {
        let i_ref = getinstr(compst, inst);
        i_ref.set_offset = (info.offset * 8) as LpegByte;
        i_ref.set_size = isize as LpegByte;
        i_ref.aux1 = info.deflt as LpegByte;
    }
    let p = nextinstruction(compst, isize) as usize;
    for i in 0..((isize as usize) * INST_BUFF_SIZE) {
        compst.code[p + (i / INST_BUFF_SIZE)].buff[i % INST_BUFF_SIZE] =
            getbytefromcharset(info, i as i32);
    }
}

fn cs_equal(code: &[Instruction], tt: usize, info: &CharsetInfo) -> bool {
    let p = &code[tt];
    if p.code != ITEST_SET {
        return false;
    }
    let isize = instsize(info.size);
    if i32::from(p.set_offset) != info.offset * 8
        || i32::from(p.set_size) != isize
        || i32::from(p.aux1) != info.deflt
    {
        return false;
    }
    for i in 0..((isize as usize) * INST_BUFF_SIZE) {
        if code[tt + 2 + (i / INST_BUFF_SIZE)].buff[i % INST_BUFF_SIZE]
            != getbytefromcharset(info, i as i32)
        {
            return false;
        }
    }
    true
}

fn codecharset(compst: &mut CompileState, tree: &TTree, tt: i32) {
    let mut info = CharsetInfo::new();
    tree2cset(tree, &mut info);
    if tt >= 0 && cs_equal(&compst.code, tt as usize, &info) {
        addinstruction(compst, IANY, 0);
    } else {
        let i = addinstruction(compst, ISET, 0);
        addcharset(compst, i, &info);
    }
}

fn codetestset(compst: &mut CompileState, cs: &Charset, e: i32) -> i32 {
    if e != 0 {
        NOINST
    } else {
        let mut info = CharsetInfo::new();
        let op = charsettype(&cs.cs, &mut info);
        match op {
            IFAIL => addoffsetinst(compst, IJMP),
            IANY => addoffsetinst(compst, ITEST_ANY),
            ICHAR => {
                let i = addoffsetinst(compst, ITEST_CHAR);
                getinstr(compst, i).aux1 = info.offset as LpegByte;
                i
            }
            _ => {
                debug_assert_eq!(op, ISET);
                let i = addoffsetinst(compst, ITEST_SET);
                addcharset(compst, i, &info);
                i
            }
        }
    }
}

fn finaltarget(code: &[Instruction], mut i: i32) -> i32 {
    while code[i as usize].code == IJMP {
        i = target(code, i);
    }
    i
}

fn finallabel(code: &[Instruction], i: i32) -> i32 {
    finaltarget(code, target(code, i))
}

fn codebehind(compst: &mut CompileState, tree: &mut [TTree], idx: usize) {
    if tree[idx].u_n > 0 {
        addinstruction(compst, IBEHIND, tree[idx].u_n);
    }
    codegen(compst, tree, sib1_idx(idx), false, NOINST, &FULLSET);
}

fn codechoice(
    compst: &mut CompileState,
    tree: &mut [TTree],
    p1: usize,
    p2: usize,
    opt: bool,
    fl: &Charset,
) {
    let emptyp2 = tree[p2].tag == TTRUE;
    let mut cs1 = Charset::new();
    let mut cs2 = Charset::new();
    let e1 = getfirst(tree, p1, &FULLSET, &mut cs1);
    if headfail(tree, p1)
        || (e1 == 0 && {
            getfirst(tree, p2, fl, &mut cs2);
            cs_disjoint(&cs1, &cs2)
        })
    {
        let test = codetestset(compst, &cs1, 0);
        let mut jmp = NOINST;
        codegen(compst, tree, p1, false, test, fl);
        if !emptyp2 {
            jmp = addoffsetinst(compst, IJMP);
        }
        jumptohere(compst, test);
        codegen(compst, tree, p2, opt, NOINST, fl);
        jumptohere(compst, jmp);
    } else if opt && emptyp2 {
        let pc = addoffsetinst(compst, IPARTIAL_COMMIT);
        jumptohere(compst, pc);
        codegen(compst, tree, p1, true, NOINST, &FULLSET);
    } else {
        let test = codetestset(compst, &cs1, e1);
        let pchoice = addoffsetinst(compst, ICHOICE);
        codegen(compst, tree, p1, emptyp2, test, &FULLSET);
        let pcommit = addoffsetinst(compst, ICOMMIT);
        jumptohere(compst, pchoice);
        jumptohere(compst, test);
        codegen(compst, tree, p2, opt, NOINST, fl);
        jumptohere(compst, pcommit);
    }
}

fn codeand(compst: &mut CompileState, tree: &mut [TTree], idx: usize, tt: i32) {
    let n = fixedlen(tree, idx);
    if (0..=MAXBEHIND).contains(&n) && hascaptures(tree, idx) == 0 {
        codegen(compst, tree, idx, false, tt, &FULLSET);
        if n > 0 {
            addinstruction(compst, IBEHIND, n);
        }
    } else {
        let pchoice = addoffsetinst(compst, ICHOICE);
        codegen(compst, tree, idx, false, tt, &FULLSET);
        let pcommit = addoffsetinst(compst, IBACK_COMMIT);
        jumptohere(compst, pchoice);
        addinstruction(compst, IFAIL, 0);
        jumptohere(compst, pcommit);
    }
}

fn codecapture(compst: &mut CompileState, tree: &mut [TTree], idx: usize, tt: i32, fl: &Charset) {
    let s1 = sib1_idx(idx);
    let len = fixedlen(tree, s1);
    let cap = i32::from(tree[idx].cap);
    let key = i32::from(tree[idx].key);
    if (0..=MAXOFF).contains(&len) && hascaptures(tree, s1) == 0 {
        codegen(compst, tree, s1, false, tt, fl);
        addinstcap(compst, IFULL_CAPTURE, cap, key, len);
    } else {
        addinstcap(compst, IOPEN_CAPTURE, cap, key, 0);
        codegen(compst, tree, s1, false, tt, fl);
        addinstcap(compst, ICLOSE_CAPTURE, i32::from(CCLOSE), 0, 0);
    }
}

fn coderuntime(compst: &mut CompileState, tree: &mut [TTree], idx: usize, tt: i32) {
    let key = i32::from(tree[idx].key);
    addinstcap(compst, IOPEN_CAPTURE, i32::from(CGROUP), key, 0);
    codegen(compst, tree, sib1_idx(idx), false, tt, &FULLSET);
    addinstcap(compst, ICLOSE_RUN_TIME, i32::from(CCLOSE), 0, 0);
}

fn closeloop(compst: &mut CompileState, test: i32) {
    let jmp = addoffsetinst(compst, IJMP);
    jumptohere(compst, test);
    jumptothere(compst, jmp, test);
}

fn coderepcharset(compst: &mut CompileState, tree: &TTree) -> bool {
    match tree.tag {
        TFALSE => true,
        TANY => {
            let test = addoffsetinst(compst, ITEST_ANY);
            addinstruction(compst, IANY, 0);
            closeloop(compst, test);
            true
        }
        TCHAR => {
            let test = addoffsetinst(compst, ITEST_CHAR);
            getinstr(compst, test).aux1 = tree.u_n as LpegByte;
            addinstruction(compst, IANY, 0);
            closeloop(compst, test);
            true
        }
        TSET => {
            let mut info = CharsetInfo::new();
            let i = addinstruction(compst, ISPAN, 0);
            tree2cset(tree, &mut info);
            addcharset(compst, i, &info);
            true
        }
        _ => false,
    }
}

fn coderep(compst: &mut CompileState, tree: &mut [TTree], idx: usize, opt: bool, fl: &Charset) {
    if !coderepcharset(compst, &tree[idx]) {
        let mut st = Charset::new();
        let e1 = getfirst(tree, idx, &FULLSET, &mut st);
        if headfail(tree, idx) || (e1 == 0 && cs_disjoint(&st, fl)) {
            let test = codetestset(compst, &st, 0);
            codegen(compst, tree, idx, false, test, &FULLSET);
            closeloop(compst, test);
        } else {
            let test = codetestset(compst, &st, e1);
            let mut pchoice = NOINST;
            if opt {
                let pc = addoffsetinst(compst, IPARTIAL_COMMIT);
                jumptohere(compst, pc);
            } else {
                pchoice = addoffsetinst(compst, ICHOICE);
            }
            let l2 = compst.ncode;
            codegen(compst, tree, idx, false, NOINST, &FULLSET);
            let commit = addoffsetinst(compst, IPARTIAL_COMMIT);
            jumptothere(compst, commit, l2);
            jumptohere(compst, pchoice);
            jumptohere(compst, test);
        }
    }
}

fn codenot(compst: &mut CompileState, tree: &mut [TTree], idx: usize) {
    let mut st = Charset::new();
    let e = getfirst(tree, idx, &FULLSET, &mut st);
    let test = codetestset(compst, &st, e);
    if headfail(tree, idx) {
        addinstruction(compst, IFAIL, 0);
    } else {
        let pchoice = addoffsetinst(compst, ICHOICE);
        codegen(compst, tree, idx, false, NOINST, &FULLSET);
        addinstruction(compst, IFAIL_TWICE, 0);
        jumptohere(compst, pchoice);
    }
    jumptohere(compst, test);
}

fn correctcalls(compst: &mut CompileState, positions: &[i32; MAXRULES], from: i32, to: i32) {
    let mut i = from;
    while i < to {
        let step = sizei(&compst.code[i as usize]);
        if compst.code[i as usize].code == IOPEN_CALL {
            let n = compst.code[i as usize].key as usize;
            let rule = positions[n];
            debug_assert!(rule == from || compst.code[(rule - 1) as usize].code == IRET);
            let ft = finaltarget(&compst.code, i + 2) as usize;
            if compst.code[ft].code == IRET {
                compst.code[i as usize].code = IJMP;
            } else {
                compst.code[i as usize].code = ICALL;
            }
            jumptothere(compst, i, rule);
        }
        i += step;
    }
    debug_assert_eq!(i, to);
}

fn codegrammar(compst: &mut CompileState, tree: &mut [TTree], grammar: usize) {
    let mut positions = [0_i32; MAXRULES];
    let mut rulenumber = 0usize;
    let firstcall = addoffsetinst(compst, ICALL);
    let jumptoend = addoffsetinst(compst, IJMP);
    let start = compst.ncode;
    jumptohere(compst, firstcall);
    let mut rule = sib1_idx(grammar);
    while tree[rule].tag == TRULE {
        let r = sib1_idx(rule);
        debug_assert_eq!(tree[r].tag, TXINFO);
        positions[rulenumber] = compst.ncode;
        rulenumber += 1;
        codegen(compst, tree, sib1_idx(r), false, NOINST, &FULLSET);
        addinstruction(compst, IRET, 0);
        rule = sib2_idx(tree, rule);
    }
    debug_assert_eq!(tree[rule].tag, TTRUE);
    jumptohere(compst, jumptoend);
    let ncode = compst.ncode;
    correctcalls(compst, &positions, start, ncode);
}

fn codecall(compst: &mut CompileState, tree: &[TTree], call: usize) {
    let c = addoffsetinst(compst, IOPEN_CALL);
    let xinfo = sib1_idx(sib2_idx(tree, call));
    debug_assert_eq!(tree[xinfo].tag, TXINFO);
    getinstr(compst, c).key = tree[xinfo].u_n as i16;
}

fn codeseq1(
    compst: &mut CompileState,
    tree: &mut [TTree],
    p1: usize,
    p2: usize,
    tt: i32,
    fl: &Charset,
) -> i32 {
    if needfollow(tree, p1) {
        let mut fl1 = Charset::new();
        getfirst(tree, p2, fl, &mut fl1);
        codegen(compst, tree, p1, false, tt, &fl1);
    } else {
        codegen(compst, tree, p1, false, tt, &FULLSET);
    }
    if fixedlen(tree, p1) != 0 { NOINST } else { tt }
}

fn codegen(
    compst: &mut CompileState,
    tree: &mut [TTree],
    mut idx: usize,
    opt: bool,
    mut tt: i32,
    fl: &Charset,
) {
    loop {
        if compst.err.is_some() {
            return;
        }
        match tree[idx].tag {
            TCHAR => {
                codechar(compst, tree[idx].u_n, tt);
                return;
            }
            TANY => {
                addinstruction(compst, IANY, 0);
                return;
            }
            TSET => {
                codecharset(compst, &tree[idx], tt);
                return;
            }
            TTRUE => return,
            TFALSE => {
                addinstruction(compst, IFAIL, 0);
                return;
            }
            TUTFR => {
                codeutfr(compst, tree, idx);
                return;
            }
            TCHOICE => {
                codechoice(compst, tree, sib1_idx(idx), sib2_idx(tree, idx), opt, fl);
                return;
            }
            TREP => {
                coderep(compst, tree, sib1_idx(idx), opt, fl);
                return;
            }
            TBEHIND => {
                codebehind(compst, tree, idx);
                return;
            }
            TNOT => {
                codenot(compst, tree, sib1_idx(idx));
                return;
            }
            TAND => {
                codeand(compst, tree, sib1_idx(idx), tt);
                return;
            }
            TCAPTURE => {
                codecapture(compst, tree, idx, tt, fl);
                return;
            }
            TRUN_TIME => {
                coderuntime(compst, tree, idx, tt);
                return;
            }
            TGRAMMAR => {
                codegrammar(compst, tree, idx);
                return;
            }
            TCALL => {
                codecall(compst, tree, idx);
                return;
            }
            TSEQ => {
                let s1 = sib1_idx(idx);
                let s2 = sib2_idx(tree, idx);
                tt = codeseq1(compst, tree, s1, s2, tt, fl);
                idx = s2;
            }
            _ => {
                debug_assert!(false);
                return;
            }
        }
    }
}

fn peephole(compst: &mut CompileState) {
    let mut i: i32 = 0;
    while i < compst.ncode {
        loop {
            let iu = i as usize;
            match compst.code[iu].code {
                ICHOICE | ICALL | ICOMMIT | IPARTIAL_COMMIT | IBACK_COMMIT | ITEST_CHAR
                | ITEST_SET | ITEST_ANY => {
                    let fl = finallabel(&compst.code, i);
                    jumptothere(compst, i, fl);
                    break;
                }
                IJMP => {
                    let ft = finaltarget(&compst.code, i);
                    let ft_inst = compst.code[ft as usize];
                    match ft_inst.code {
                        IRET | IFAIL | IFAIL_TWICE | IEND => {
                            compst.code[iu] = ft_inst;
                            compst.code[iu + 1].code = IEMPTY;
                            break;
                        }
                        ICOMMIT | IPARTIAL_COMMIT | IBACK_COMMIT => {
                            let fft = finallabel(&compst.code, ft);
                            compst.code[iu] = ft_inst;
                            jumptothere(compst, i, fft);
                            continue;
                        }
                        _ => {
                            jumptothere(compst, i, ft);
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        i += sizei(&compst.code[i as usize]);
    }
    debug_assert_eq!(compst.code[(i - 1) as usize].code, IEND);
}

pub fn compile(p: &mut Pattern, size: LpegUint) -> Result<(), &'static CStr> {
    let init_size = ((size / 2) + 2) as usize;
    let mut compst = CompileState {
        code: vec![Instruction::new(); init_size],
        ncode: 0,
        err: None,
    };
    codegen(&mut compst, &mut p.tree, 0, false, NOINST, &FULLSET);
    if let Some(err) = compst.err {
        return Err(err);
    }
    addinstruction(&mut compst, IEND, 0);
    if let Some(err) = compst.err {
        return Err(err);
    }
    compst.code.truncate(compst.ncode as usize);
    compst.code.shrink_to_fit();
    peephole(&mut compst);
    p.code = compst.code;
    Ok(())
}
