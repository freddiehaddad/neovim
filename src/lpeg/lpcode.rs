//! LPeg pattern analysis and bytecode compiler (`src/lpeg/lpcode.c`).

use std::ffi::{c_int, c_short};
use std::mem::size_of;

use crate::lpeg::lpcset::{charsettype, getbytefromcharset, tocharset, tree2cset};
use crate::lpeg::{
    CCLOSE, CGROUP, CHARSETSIZE, Charset, CharsetInfo, IANY, IBACK_COMMIT, IBEHIND, ICALL, ICHAR,
    ICHOICE, ICLOSE_CAPTURE, ICLOSE_RUN_TIME, ICOMMIT, IEMPTY, IEND, IFAIL, IFAIL_TWICE,
    IFULL_CAPTURE, IJMP, IOPEN_CALL, IOPEN_CAPTURE, IPARTIAL_COMMIT, IRET, ISET, ISPAN, ITEST_ANY,
    ITEST_CHAR, ITEST_SET, IUTFR, Instruction, LpegByte, LpegUint, MAXBEHIND, MAXOFF, MAXRULES,
    NUMSIBLINGS, Opcode, Pattern, TAND, TANY, TBEHIND, TCALL, TCAPTURE, TCHAR, TCHOICE, TFALSE,
    TGRAMMAR, TNOT, TOPEN_CALL, TREP, TRULE, TRUN_TIME, TSEQ, TSET, TTRUE, TTree, TUTFR, TXINFO,
    clearset, instsize, joinkindoff, lua_State, setchar, sib1, sib2,
};

use crate::lua::converter::luaL_error;

pub const PENULLABLE: c_int = 0;
pub const PENOFAIL: c_int = 1;

const NOINST: c_int = -1;

static FULLSET: Charset = Charset { cs: [0xFF; 32] };

#[inline]
pub fn nofail(t: *mut TTree) -> bool {
    checkaux(t, PENOFAIL) != 0
}

#[inline]
pub fn nullable(t: *mut TTree) -> bool {
    checkaux(t, PENULLABLE) != 0
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

fn callrecursive(tree: &mut TTree, f: fn(*mut TTree) -> c_int, def: c_int) -> c_int {
    unsafe {
        let key = tree.key;
        debug_assert_eq!(tree.tag, TCALL);
        debug_assert_eq!((*sib2(&raw mut *tree)).tag, TRULE);
        if key == 0 {
            def
        } else {
            tree.key = 0;
            let result = f(sib2(&raw mut *tree));
            tree.key = key;
            result
        }
    }
}

pub fn hascaptures(mut tree: *mut TTree) -> c_int {
    unsafe {
        loop {
            match (*tree).tag {
                TCAPTURE | TRUN_TIME => return 1,
                TCALL => return callrecursive(&mut *tree, hascaptures, 0),
                TRULE => {
                    tree = sib1(tree);
                }
                TOPEN_CALL => {
                    debug_assert!(false);
                    return 0;
                }
                tag => match NUMSIBLINGS[tag as usize] {
                    1 => {
                        tree = sib1(tree);
                    }
                    2 => {
                        if hascaptures(sib1(tree)) != 0 {
                            return 1;
                        }
                        tree = sib2(tree);
                    }
                    _ => {
                        debug_assert_eq!(NUMSIBLINGS[tag as usize], 0);
                        return 0;
                    }
                },
            }
        }
    }
}

pub fn checkaux(mut tree: *mut TTree, pred: c_int) -> c_int {
    unsafe {
        loop {
            match (*tree).tag {
                TCHAR | TSET | TANY | TUTFR | TFALSE | TOPEN_CALL => return 0,
                TREP | TTRUE => return 1,
                TNOT | TBEHIND => {
                    return if pred == PENOFAIL { 0 } else { 1 };
                }
                TAND => {
                    if pred == PENULLABLE {
                        return 1;
                    }
                    tree = sib1(tree);
                }
                TRUN_TIME => {
                    if pred == PENOFAIL {
                        return 0;
                    }
                    tree = sib1(tree);
                }
                TSEQ => {
                    if checkaux(sib1(tree), pred) == 0 {
                        return 0;
                    }
                    tree = sib2(tree);
                }
                TCHOICE => {
                    if checkaux(sib2(tree), pred) != 0 {
                        return 1;
                    }
                    tree = sib1(tree);
                }
                TCAPTURE | TGRAMMAR | TRULE | TXINFO => {
                    tree = sib1(tree);
                }
                TCALL => {
                    tree = sib2(tree);
                }
                _ => {
                    debug_assert!(false);
                    return 0;
                }
            }
        }
    }
}

pub fn fixedlen(mut tree: *mut TTree) -> c_int {
    let mut len: c_int = 0;
    unsafe {
        loop {
            match (*tree).tag {
                TCHAR | TSET | TANY => return len + 1,
                TUTFR => {
                    return if (*tree).cap == (*sib1(tree)).cap {
                        len + c_int::from((*tree).cap)
                    } else {
                        -1
                    };
                }
                TFALSE | TTRUE | TNOT | TAND | TBEHIND => return len,
                TREP | TRUN_TIME | TOPEN_CALL => return -1,
                TCAPTURE | TRULE | TGRAMMAR | TXINFO => {
                    tree = sib1(tree);
                }
                TCALL => {
                    let n1 = callrecursive(&mut *tree, fixedlen, -1);
                    return if n1 < 0 { -1 } else { len + n1 };
                }
                TSEQ => {
                    let n1 = fixedlen(sib1(tree));
                    if n1 < 0 {
                        return -1;
                    }
                    len += n1;
                    tree = sib2(tree);
                }
                TCHOICE => {
                    let n1 = fixedlen(sib1(tree));
                    let n2 = fixedlen(sib2(tree));
                    return if n1 != n2 || n1 < 0 { -1 } else { len + n1 };
                }
                _ => {
                    debug_assert!(false);
                    return 0;
                }
            }
        }
    }
}

fn getfirst(mut tree: *mut TTree, mut follow: &Charset, firstset: &mut Charset) -> c_int {
    unsafe {
        loop {
            match (*tree).tag {
                TCHAR | TSET | TANY | TFALSE => {
                    tocharset(tree, firstset);
                    return 0;
                }
                TUTFR => {
                    clearset(&mut firstset.cs);
                    let start = usize::from((*tree).key);
                    let end = usize::from((*sib1(tree)).key);
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
                    let e1 = getfirst(sib1(tree), follow, firstset);
                    let e2 = getfirst(sib2(tree), follow, &mut csaux);
                    for i in 0..CHARSETSIZE {
                        firstset.cs[i] |= csaux.cs[i];
                    }
                    return e1 | e2;
                }
                TSEQ => {
                    if !nullable(sib1(tree)) {
                        tree = sib1(tree);
                        follow = &FULLSET;
                    } else {
                        let mut csaux = Charset::new();
                        let e2 = getfirst(sib2(tree), follow, &mut csaux);
                        let e1 = getfirst(sib1(tree), &csaux, firstset);
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
                    getfirst(sib1(tree), follow, firstset);
                    for i in 0..CHARSETSIZE {
                        firstset.cs[i] |= follow.cs[i];
                    }
                    return 1;
                }
                TCAPTURE | TGRAMMAR | TRULE | TXINFO => {
                    tree = sib1(tree);
                }
                TRUN_TIME => {
                    let e = getfirst(sib1(tree), &FULLSET, firstset);
                    return if e != 0 { 2 } else { 0 };
                }
                TCALL => {
                    tree = sib2(tree);
                }
                TAND => {
                    let e = getfirst(sib1(tree), follow, firstset);
                    for i in 0..CHARSETSIZE {
                        firstset.cs[i] &= follow.cs[i];
                    }
                    return e;
                }
                TNOT => {
                    if tocharset(sib1(tree), firstset) != 0 {
                        cs_complement(firstset);
                        return 1;
                    }
                    let e = getfirst(sib1(tree), follow, firstset);
                    firstset.cs = follow.cs;
                    return e | 1;
                }
                TBEHIND => {
                    let e = getfirst(sib1(tree), follow, firstset);
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
}

fn headfail(mut tree: *mut TTree) -> bool {
    unsafe {
        loop {
            match (*tree).tag {
                TCHAR | TSET | TANY | TFALSE => return true,
                TTRUE | TREP | TRUN_TIME | TNOT | TBEHIND | TUTFR => return false,
                TCAPTURE | TGRAMMAR | TRULE | TXINFO | TAND => {
                    tree = sib1(tree);
                }
                TCALL => {
                    tree = sib2(tree);
                }
                TSEQ => {
                    if !nofail(sib2(tree)) {
                        return false;
                    }
                    tree = sib1(tree);
                }
                TCHOICE => {
                    if !headfail(sib1(tree)) {
                        return false;
                    }
                    tree = sib2(tree);
                }
                _ => {
                    debug_assert!(false);
                    return false;
                }
            }
        }
    }
}

fn needfollow(mut tree: *mut TTree) -> bool {
    unsafe {
        loop {
            match (*tree).tag {
                TCHAR | TSET | TANY | TUTFR | TFALSE | TTRUE | TAND | TNOT | TRUN_TIME
                | TGRAMMAR | TCALL | TBEHIND => return false,
                TCHOICE | TREP => return true,
                TCAPTURE => {
                    tree = sib1(tree);
                }
                TSEQ => {
                    tree = sib2(tree);
                }
                _ => {
                    debug_assert!(false);
                    return false;
                }
            }
        }
    }
}

pub fn sizei(i: *const Instruction) -> c_int {
    unsafe {
        match (*i).i.code {
            ISET | ISPAN => 1 + c_int::from((*i).i.aux2.set.size),
            ITEST_SET => 2 + c_int::from((*i).i.aux2.set.size),
            ITEST_CHAR | ITEST_ANY | ICHOICE | IJMP | ICALL | IOPEN_CALL | ICOMMIT
            | IPARTIAL_COMMIT | IBACK_COMMIT | IUTFR => 2,
            _ => 1,
        }
    }
}

#[repr(C)]
struct CompileState {
    p: *mut Pattern,
    ncode: c_int,
    lstate: *mut lua_State,
}

fn nextinstruction(compst: &mut CompileState, n: c_int) -> c_int {
    unsafe {
        let code = &mut (*compst.p).code;
        let size = code.len() as c_int;
        let ncode = compst.ncode;
        if ncode > size - n {
            let nsize = (size as LpegUint) + ((size as LpegUint) >> 1) + (n as LpegUint);
            if nsize >= (c_int::MAX as LpegUint) {
                luaL_error(compst.lstate, c"pattern code too large".as_ptr());
            }
            code.resize(nsize as usize, Instruction { offset: 0 });
        }
        compst.ncode = ncode + n;
        ncode
    }
}

#[inline]
fn getinstr(cs: &mut CompileState, i: c_int) -> *mut Instruction {
    unsafe { (*cs.p).code.as_mut_ptr().add(i as usize) }
}

fn addinstruction(compst: &mut CompileState, op: Opcode, aux: c_int) -> c_int {
    unsafe {
        let i = nextinstruction(compst, 1);
        let inst = getinstr(compst, i);
        (*inst).i.code = op;
        (*inst).i.aux1 = aux as LpegByte;
        i
    }
}

fn addoffsetinst(compst: &mut CompileState, op: Opcode) -> c_int {
    let i = addinstruction(compst, op, 0);
    addinstruction(compst, 0, 0);
    debug_assert!(op == ITEST_SET || sizei(getinstr(compst, i)) == 2);
    i
}

#[inline]
fn setoffset(compst: &mut CompileState, instruction: c_int, offset: c_int) {
    unsafe {
        (*getinstr(compst, instruction + 1)).offset = offset;
    }
}

fn codeutfr(compst: &mut CompileState, tree: &mut TTree) {
    unsafe {
        let i = addoffsetinst(compst, IUTFR);
        let to = (*sib1(&raw mut *tree)).u.n;
        debug_assert_eq!((*sib1(&raw mut *tree)).tag, TXINFO);
        (*getinstr(compst, i + 1)).offset = tree.u.n;
        let inst = getinstr(compst, i);
        (*inst).i.aux1 = (to & 0xff) as LpegByte;
        (*inst).i.aux2.key = (to >> 8) as c_short;
    }
}

fn addinstcap(compst: &mut CompileState, op: Opcode, cap: c_int, key: c_int, aux: c_int) -> c_int {
    unsafe {
        let i = addinstruction(compst, op, joinkindoff(cap, aux));
        (*getinstr(compst, i)).i.aux2.key = key as c_short;
        i
    }
}

#[inline]
fn target(code: *const Instruction, i: c_int) -> c_int {
    unsafe { i + (*code.add((i + 1) as usize)).offset }
}

#[inline]
fn jumptothere(compst: &mut CompileState, instruction: c_int, target_pos: c_int) {
    if instruction >= 0 {
        setoffset(compst, instruction, target_pos - instruction);
    }
}

#[inline]
fn jumptohere(compst: &mut CompileState, instruction: c_int) {
    let ncode = compst.ncode;
    jumptothere(compst, instruction, ncode);
}

fn codechar(compst: &mut CompileState, c: c_int, tt: c_int) {
    unsafe {
        if tt >= 0
            && (*getinstr(compst, tt)).i.code == ITEST_CHAR
            && c_int::from((*getinstr(compst, tt)).i.aux1) == c
        {
            addinstruction(compst, IANY, 0);
        } else {
            addinstruction(compst, ICHAR, c);
        }
    }
}

fn addcharset(compst: &mut CompileState, inst: c_int, info: &CharsetInfo) {
    unsafe {
        let isize = instsize(info.size);
        {
            let i_ptr = getinstr(compst, inst);
            (*i_ptr).i.aux2.set.offset = (info.offset * 8) as LpegByte;
            (*i_ptr).i.aux2.set.size = isize as LpegByte;
            (*i_ptr).i.aux1 = info.deflt as LpegByte;
        }
        let p = nextinstruction(compst, isize);
        let charset = getinstr(compst, p).cast::<LpegByte>();
        for i in 0..((isize as usize) * size_of::<Instruction>()) {
            *charset.add(i) = getbytefromcharset(info, i as c_int);
        }
    }
}

fn cs_equal(p: *const Instruction, info: &CharsetInfo) -> bool {
    unsafe {
        if (*p).i.code != ITEST_SET {
            return false;
        }
        let isize = instsize(info.size);
        if c_int::from((*p).i.aux2.set.offset) != info.offset * 8
            || c_int::from((*p).i.aux2.set.size) != isize
            || c_int::from((*p).i.aux1) != info.deflt
        {
            return false;
        }
        let buff = p.add(2).cast::<LpegByte>();
        for i in 0..((isize as usize) * size_of::<Instruction>()) {
            if *buff.add(i) != getbytefromcharset(info, i as c_int) {
                return false;
            }
        }
        true
    }
}

fn codecharset(compst: &mut CompileState, tree: &mut TTree, tt: c_int) {
    let mut info = CharsetInfo::new();
    tree2cset(tree, &mut info);
    if tt >= 0 && cs_equal(getinstr(compst, tt), &info) {
        addinstruction(compst, IANY, 0);
    } else {
        let i = addinstruction(compst, ISET, 0);
        addcharset(compst, i, &info);
    }
}

fn codetestset(compst: &mut CompileState, cs: &Charset, e: c_int) -> c_int {
    if e != 0 {
        NOINST
    } else {
        unsafe {
            let mut info = CharsetInfo::new();
            let op = charsettype(&cs.cs, &mut info);
            match op {
                IFAIL => addoffsetinst(compst, IJMP),
                IANY => addoffsetinst(compst, ITEST_ANY),
                ICHAR => {
                    let i = addoffsetinst(compst, ITEST_CHAR);
                    (*getinstr(compst, i)).i.aux1 = info.offset as LpegByte;
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
}

fn finaltarget(code: *const Instruction, mut i: c_int) -> c_int {
    unsafe {
        while (*code.add(i as usize)).i.code == IJMP {
            i = target(code, i);
        }
        i
    }
}

fn finallabel(code: *const Instruction, i: c_int) -> c_int {
    finaltarget(code, target(code, i))
}

fn codebehind(compst: &mut CompileState, tree: &mut TTree) {
    unsafe {
        if tree.u.n > 0 {
            addinstruction(compst, IBEHIND, tree.u.n);
        }
        codegen(compst, sib1(&raw mut *tree), false, NOINST, &FULLSET);
    }
}

fn codechoice(compst: &mut CompileState, p1: *mut TTree, p2: *mut TTree, opt: bool, fl: &Charset) {
    unsafe {
        let emptyp2 = (*p2).tag == TTRUE;
        let mut cs1 = Charset::new();
        let mut cs2 = Charset::new();
        let e1 = getfirst(p1, &FULLSET, &mut cs1);
        if headfail(p1)
            || (e1 == 0 && {
                getfirst(p2, fl, &mut cs2);
                cs_disjoint(&cs1, &cs2)
            })
        {
            let test = codetestset(compst, &cs1, 0);
            let mut jmp = NOINST;
            codegen(compst, p1, false, test, fl);
            if !emptyp2 {
                jmp = addoffsetinst(compst, IJMP);
            }
            jumptohere(compst, test);
            codegen(compst, p2, opt, NOINST, fl);
            jumptohere(compst, jmp);
        } else if opt && emptyp2 {
            let pc = addoffsetinst(compst, IPARTIAL_COMMIT);
            jumptohere(compst, pc);
            codegen(compst, p1, true, NOINST, &FULLSET);
        } else {
            let test = codetestset(compst, &cs1, e1);
            let pchoice = addoffsetinst(compst, ICHOICE);
            codegen(compst, p1, emptyp2, test, &FULLSET);
            let pcommit = addoffsetinst(compst, ICOMMIT);
            jumptohere(compst, pchoice);
            jumptohere(compst, test);
            codegen(compst, p2, opt, NOINST, fl);
            jumptohere(compst, pcommit);
        }
    }
}

fn codeand(compst: &mut CompileState, tree: *mut TTree, tt: c_int) {
    let n = fixedlen(tree);
    if (0..=MAXBEHIND).contains(&n) && hascaptures(tree) == 0 {
        codegen(compst, tree, false, tt, &FULLSET);
        if n > 0 {
            addinstruction(compst, IBEHIND, n);
        }
    } else {
        let pchoice = addoffsetinst(compst, ICHOICE);
        codegen(compst, tree, false, tt, &FULLSET);
        let pcommit = addoffsetinst(compst, IBACK_COMMIT);
        jumptohere(compst, pchoice);
        addinstruction(compst, IFAIL, 0);
        jumptohere(compst, pcommit);
    }
}

fn codecapture(compst: &mut CompileState, tree: &mut TTree, tt: c_int, fl: &Charset) {
    {
        let len = fixedlen(sib1(&raw mut *tree));
        if (0..=MAXOFF).contains(&len) && hascaptures(sib1(&raw mut *tree)) == 0 {
            codegen(compst, sib1(&raw mut *tree), false, tt, fl);
            addinstcap(
                compst,
                IFULL_CAPTURE,
                c_int::from(tree.cap),
                c_int::from(tree.key),
                len,
            );
        } else {
            addinstcap(
                compst,
                IOPEN_CAPTURE,
                c_int::from(tree.cap),
                c_int::from(tree.key),
                0,
            );
            codegen(compst, sib1(&raw mut *tree), false, tt, fl);
            addinstcap(compst, ICLOSE_CAPTURE, c_int::from(CCLOSE), 0, 0);
        }
    }
}

fn coderuntime(compst: &mut CompileState, tree: &mut TTree, tt: c_int) {
    {
        addinstcap(
            compst,
            IOPEN_CAPTURE,
            c_int::from(CGROUP),
            c_int::from(tree.key),
            0,
        );
        codegen(compst, sib1(&raw mut *tree), false, tt, &FULLSET);
        addinstcap(compst, ICLOSE_RUN_TIME, c_int::from(CCLOSE), 0, 0);
    }
}

fn closeloop(compst: &mut CompileState, test: c_int) {
    let jmp = addoffsetinst(compst, IJMP);
    jumptohere(compst, test);
    jumptothere(compst, jmp, test);
}

fn coderepcharset(compst: &mut CompileState, tree: &mut TTree) -> bool {
    unsafe {
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
                (*getinstr(compst, test)).i.aux1 = tree.u.n as LpegByte;
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
}

fn coderep(compst: &mut CompileState, tree: *mut TTree, opt: bool, fl: &Charset) {
    if !coderepcharset(compst, unsafe { &mut *tree }) {
        let mut st = Charset::new();
        let e1 = getfirst(tree, &FULLSET, &mut st);
        if headfail(tree) || (e1 == 0 && cs_disjoint(&st, fl)) {
            let test = codetestset(compst, &st, 0);
            codegen(compst, tree, false, test, &FULLSET);
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
            codegen(compst, tree, false, NOINST, &FULLSET);
            let commit = addoffsetinst(compst, IPARTIAL_COMMIT);
            jumptothere(compst, commit, l2);
            jumptohere(compst, pchoice);
            jumptohere(compst, test);
        }
    }
}

fn codenot(compst: &mut CompileState, tree: *mut TTree) {
    let mut st = Charset::new();
    let e = getfirst(tree, &FULLSET, &mut st);
    let test = codetestset(compst, &st, e);
    if headfail(tree) {
        addinstruction(compst, IFAIL, 0);
    } else {
        let pchoice = addoffsetinst(compst, ICHOICE);
        codegen(compst, tree, false, NOINST, &FULLSET);
        addinstruction(compst, IFAIL_TWICE, 0);
        jumptohere(compst, pchoice);
    }
    jumptohere(compst, test);
}

fn correctcalls(compst: &mut CompileState, positions: &[c_int; MAXRULES], from: c_int, to: c_int) {
    unsafe {
        let code = (*compst.p).code.as_mut_ptr();
        let mut i = from;
        while i < to {
            let inst = code.add(i as usize);
            if (*inst).i.code == IOPEN_CALL {
                let n = (*inst).i.aux2.key as usize;
                let rule = positions[n];
                debug_assert!(rule == from || (*code.add((rule - 1) as usize)).i.code == IRET);
                if (*code.add(finaltarget(code, i + 2) as usize)).i.code == IRET {
                    (*inst).i.code = IJMP;
                } else {
                    (*inst).i.code = ICALL;
                }
                jumptothere(compst, i, rule);
            }
            i += sizei(inst);
        }
        debug_assert_eq!(i, to);
    }
}

fn codegrammar(compst: &mut CompileState, grammar: *mut TTree) {
    unsafe {
        let mut positions = [0 as c_int; MAXRULES];
        let mut rulenumber = 0usize;
        let firstcall = addoffsetinst(compst, ICALL);
        let jumptoend = addoffsetinst(compst, IJMP);
        let start = compst.ncode;
        jumptohere(compst, firstcall);
        let mut rule = sib1(grammar);
        while (*rule).tag == TRULE {
            let r = sib1(rule);
            debug_assert_eq!((*r).tag, TXINFO);
            positions[rulenumber] = compst.ncode;
            rulenumber += 1;
            codegen(compst, sib1(r), false, NOINST, &FULLSET);
            addinstruction(compst, IRET, 0);
            rule = sib2(rule);
        }
        debug_assert_eq!((*rule).tag, TTRUE);
        jumptohere(compst, jumptoend);
        let ncode = compst.ncode;
        correctcalls(compst, &positions, start, ncode);
    }
}

fn codecall(compst: &mut CompileState, call: *mut TTree) {
    unsafe {
        let c = addoffsetinst(compst, IOPEN_CALL);
        debug_assert_eq!((*sib1(sib2(call))).tag, TXINFO);
        (*getinstr(compst, c)).i.aux2.key = (*sib1(sib2(call))).u.n as c_short;
    }
}

fn codeseq1(
    compst: &mut CompileState,
    p1: *mut TTree,
    p2: *mut TTree,
    tt: c_int,
    fl: &Charset,
) -> c_int {
    if needfollow(p1) {
        let mut fl1 = Charset::new();
        getfirst(p2, fl, &mut fl1);
        codegen(compst, p1, false, tt, &fl1);
    } else {
        codegen(compst, p1, false, tt, &FULLSET);
    }
    if fixedlen(p1) != 0 { NOINST } else { tt }
}

fn codegen(
    compst: &mut CompileState,
    mut tree: *mut TTree,
    opt: bool,
    mut tt: c_int,
    fl: &Charset,
) {
    unsafe {
        loop {
            match (*tree).tag {
                TCHAR => {
                    codechar(compst, (*tree).u.n, tt);
                    return;
                }
                TANY => {
                    addinstruction(compst, IANY, 0);
                    return;
                }
                TSET => {
                    codecharset(compst, &mut *tree, tt);
                    return;
                }
                TTRUE => return,
                TFALSE => {
                    addinstruction(compst, IFAIL, 0);
                    return;
                }
                TUTFR => {
                    codeutfr(compst, &mut *tree);
                    return;
                }
                TCHOICE => {
                    codechoice(compst, sib1(tree), sib2(tree), opt, fl);
                    return;
                }
                TREP => {
                    coderep(compst, sib1(tree), opt, fl);
                    return;
                }
                TBEHIND => {
                    codebehind(compst, &mut *tree);
                    return;
                }
                TNOT => {
                    codenot(compst, sib1(tree));
                    return;
                }
                TAND => {
                    codeand(compst, sib1(tree), tt);
                    return;
                }
                TCAPTURE => {
                    codecapture(compst, &mut *tree, tt, fl);
                    return;
                }
                TRUN_TIME => {
                    coderuntime(compst, &mut *tree, tt);
                    return;
                }
                TGRAMMAR => {
                    codegrammar(compst, tree);
                    return;
                }
                TCALL => {
                    codecall(compst, tree);
                    return;
                }
                TSEQ => {
                    tt = codeseq1(compst, sib1(tree), sib2(tree), tt, fl);
                    tree = sib2(tree);
                }
                _ => {
                    debug_assert!(false);
                    return;
                }
            }
        }
    }
}

fn peephole(compst: &mut CompileState) {
    unsafe {
        let code = (*compst.p).code.as_mut_ptr();
        let mut i: c_int = 0;
        while i < compst.ncode {
            loop {
                let inst = code.add(i as usize);
                match (*inst).i.code {
                    ICHOICE | ICALL | ICOMMIT | IPARTIAL_COMMIT | IBACK_COMMIT | ITEST_CHAR
                    | ITEST_SET | ITEST_ANY => {
                        jumptothere(compst, i, finallabel(code, i));
                        break;
                    }
                    IJMP => {
                        let ft = finaltarget(code, i);
                        let ft_inst = code.add(ft as usize);
                        match (*ft_inst).i.code {
                            IRET | IFAIL | IFAIL_TWICE | IEND => {
                                *inst = *ft_inst;
                                (*code.add((i + 1) as usize)).i.code = IEMPTY;
                                break;
                            }
                            ICOMMIT | IPARTIAL_COMMIT | IBACK_COMMIT => {
                                let fft = finallabel(code, ft);
                                *inst = *ft_inst;
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
            i += sizei(code.add(i as usize));
        }
        debug_assert_eq!((*code.add((i - 1) as usize)).i.code, IEND);
    }
}

pub fn compile(l: *mut lua_State, p: &mut Pattern, size: LpegUint) -> *mut Instruction {
    let mut compst = CompileState {
        p,
        ncode: 0,
        lstate: l,
    };
    let init_size = ((size / 2) + 2) as usize;
    p.code = vec![Instruction { offset: 0 }; init_size];
    let tree_ptr = (&raw mut p.tree).cast::<TTree>();
    codegen(&mut compst, tree_ptr, false, NOINST, &FULLSET);
    addinstruction(&mut compst, IEND, 0);
    p.code.truncate(compst.ncode as usize);
    p.code.shrink_to_fit();
    peephole(&mut compst);
    p.code.as_mut_ptr()
}
