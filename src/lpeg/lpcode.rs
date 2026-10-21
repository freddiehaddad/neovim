//! LPeg pattern analysis and bytecode compiler (`src/lpeg/lpcode.c`).

use std::ffi::{c_int, c_short, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

use crate::lpeg::lpcset::{charsettype, getbytefromcharset, tocharset, tree2cset};
use crate::lpeg::{
    CHARSETSIZE, Cclose, Cgroup, Charset, IAny, IBackCommit, IBehind, ICall, IChar, IChoice,
    ICloseCapture, ICloseRunTime, ICommit, IEmpty, IEnd, IFail, IFailTwice, IFullCapture, IJmp,
    IOpenCall, IOpenCapture, IPartialCommit, IRet, ISet, ISpan, ITestAny, ITestChar, ITestSet,
    IUTFR, Instruction, MAXBEHIND, MAXOFF, MAXRULES, NUMSIBLINGS, Opcode, Pattern, TAnd, TAny,
    TBehind, TCall, TCapture, TChar, TChoice, TFalse, TGrammar, TNot, TOpenCall, TRep, TRule,
    TRunTime, TSeq, TSet, TTree, TTrue, TUTFR, TXInfo, byte, charsetinfo, clearset, instsize,
    joinkindoff, lua_State, setchar, sib1, sib2, uint,
};

use crate::lua::converter::{lua_getallocf, luaL_error};

pub const PEnullable: c_int = 0;
pub const PEnofail: c_int = 1;

const NOINST: c_int = -1;

static FULLSET: Charset = Charset { cs: [0xFF; 32] };

#[inline]
pub unsafe fn nofail(t: *mut TTree) -> bool {
    unsafe { checkaux(t, PEnofail) != 0 }
}

#[inline]
pub unsafe fn nullable(t: *mut TTree) -> bool {
    unsafe { checkaux(t, PEnullable) != 0 }
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

unsafe fn callrecursive(tree: *mut TTree, f: unsafe fn(*mut TTree) -> c_int, def: c_int) -> c_int {
    unsafe {
        let key = (*tree).key;
        debug_assert_eq!((*tree).tag, TCall);
        debug_assert_eq!((*sib2(tree)).tag, TRule);
        if key == 0 {
            def
        } else {
            (*tree).key = 0;
            let result = f(sib2(tree));
            (*tree).key = key;
            result
        }
    }
}

pub unsafe fn hascaptures(mut tree: *mut TTree) -> c_int {
    unsafe {
        loop {
            match (*tree).tag {
                TCapture | TRunTime => return 1,
                TCall => return callrecursive(tree, hascaptures, 0),
                TRule => {
                    tree = sib1(tree);
                }
                TOpenCall => {
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

pub unsafe fn checkaux(mut tree: *mut TTree, pred: c_int) -> c_int {
    unsafe {
        loop {
            match (*tree).tag {
                TChar | TSet | TAny | TUTFR | TFalse | TOpenCall => return 0,
                TRep | TTrue => return 1,
                TNot | TBehind => {
                    return if pred == PEnofail { 0 } else { 1 };
                }
                TAnd => {
                    if pred == PEnullable {
                        return 1;
                    }
                    tree = sib1(tree);
                }
                TRunTime => {
                    if pred == PEnofail {
                        return 0;
                    }
                    tree = sib1(tree);
                }
                TSeq => {
                    if checkaux(sib1(tree), pred) == 0 {
                        return 0;
                    }
                    tree = sib2(tree);
                }
                TChoice => {
                    if checkaux(sib2(tree), pred) != 0 {
                        return 1;
                    }
                    tree = sib1(tree);
                }
                TCapture | TGrammar | TRule | TXInfo => {
                    tree = sib1(tree);
                }
                TCall => {
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

pub unsafe fn fixedlen(mut tree: *mut TTree) -> c_int {
    let mut len: c_int = 0;
    unsafe {
        loop {
            match (*tree).tag {
                TChar | TSet | TAny => return len + 1,
                TUTFR => {
                    return if (*tree).cap == (*sib1(tree)).cap {
                        len + c_int::from((*tree).cap)
                    } else {
                        -1
                    };
                }
                TFalse | TTrue | TNot | TAnd | TBehind => return len,
                TRep | TRunTime | TOpenCall => return -1,
                TCapture | TRule | TGrammar | TXInfo => {
                    tree = sib1(tree);
                }
                TCall => {
                    let n1 = callrecursive(tree, fixedlen, -1);
                    return if n1 < 0 { -1 } else { len + n1 };
                }
                TSeq => {
                    let n1 = fixedlen(sib1(tree));
                    if n1 < 0 {
                        return -1;
                    }
                    len += n1;
                    tree = sib2(tree);
                }
                TChoice => {
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

unsafe fn getfirst(
    mut tree: *mut TTree,
    mut follow: *const Charset,
    firstset: *mut Charset,
) -> c_int {
    unsafe {
        loop {
            match (*tree).tag {
                TChar | TSet | TAny | TFalse => {
                    tocharset(tree, firstset);
                    return 0;
                }
                TUTFR => {
                    clearset((*firstset).cs.as_mut_ptr());
                    let start = usize::from((*tree).key);
                    let end = usize::from((*sib1(tree)).key);
                    for c in start..=end {
                        setchar((*firstset).cs.as_mut_ptr(), c);
                    }
                    return 0;
                }
                TTrue => {
                    (*firstset).cs = (*follow).cs;
                    return 1;
                }
                TChoice => {
                    let mut csaux: Charset = zeroed();
                    let e1 = getfirst(sib1(tree), follow, firstset);
                    let e2 = getfirst(sib2(tree), follow, &mut csaux);
                    for i in 0..CHARSETSIZE {
                        (*firstset).cs[i] |= csaux.cs[i];
                    }
                    return e1 | e2;
                }
                TSeq => {
                    if !nullable(sib1(tree)) {
                        tree = sib1(tree);
                        follow = &raw const FULLSET;
                    } else {
                        let mut csaux: Charset = zeroed();
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
                TRep => {
                    getfirst(sib1(tree), follow, firstset);
                    for i in 0..CHARSETSIZE {
                        (*firstset).cs[i] |= (*follow).cs[i];
                    }
                    return 1;
                }
                TCapture | TGrammar | TRule | TXInfo => {
                    tree = sib1(tree);
                }
                TRunTime => {
                    let e = getfirst(sib1(tree), &raw const FULLSET, firstset);
                    return if e != 0 { 2 } else { 0 };
                }
                TCall => {
                    tree = sib2(tree);
                }
                TAnd => {
                    let e = getfirst(sib1(tree), follow, firstset);
                    for i in 0..CHARSETSIZE {
                        (*firstset).cs[i] &= (*follow).cs[i];
                    }
                    return e;
                }
                TNot => {
                    if tocharset(sib1(tree), firstset) != 0 {
                        cs_complement(&mut *firstset);
                        return 1;
                    }
                    let e = getfirst(sib1(tree), follow, firstset);
                    (*firstset).cs = (*follow).cs;
                    return e | 1;
                }
                TBehind => {
                    let e = getfirst(sib1(tree), follow, firstset);
                    (*firstset).cs = (*follow).cs;
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

unsafe fn headfail(mut tree: *mut TTree) -> bool {
    unsafe {
        loop {
            match (*tree).tag {
                TChar | TSet | TAny | TFalse => return true,
                TTrue | TRep | TRunTime | TNot | TBehind | TUTFR => return false,
                TCapture | TGrammar | TRule | TXInfo | TAnd => {
                    tree = sib1(tree);
                }
                TCall => {
                    tree = sib2(tree);
                }
                TSeq => {
                    if !nofail(sib2(tree)) {
                        return false;
                    }
                    tree = sib1(tree);
                }
                TChoice => {
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

unsafe fn needfollow(mut tree: *mut TTree) -> bool {
    unsafe {
        loop {
            match (*tree).tag {
                TChar | TSet | TAny | TUTFR | TFalse | TTrue | TAnd | TNot | TRunTime
                | TGrammar | TCall | TBehind => return false,
                TChoice | TRep => return true,
                TCapture => {
                    tree = sib1(tree);
                }
                TSeq => {
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

pub unsafe fn sizei(i: *const Instruction) -> c_int {
    unsafe {
        match (*i).i.code {
            ISet | ISpan => 1 + c_int::from((*i).i.aux2.set.size),
            ITestSet => 2 + c_int::from((*i).i.aux2.set.size),
            ITestChar | ITestAny | IChoice | IJmp | ICall | IOpenCall | ICommit
            | IPartialCommit | IBackCommit | IUTFR => 2,
            _ => 1,
        }
    }
}

#[repr(C)]
struct CompileState {
    p: *mut Pattern,
    ncode: c_int,
    L: *mut lua_State,
}

unsafe fn finishrelcode(l: *mut lua_State, p: *mut Pattern, block: *mut Instruction, size: c_int) {
    unsafe {
        if block.is_null() {
            luaL_error(l, c"not enough memory".as_ptr());
        }
        (*block).codesize = size as uint;
        (*p).code = block.add(1);
    }
}

unsafe fn newcode(l: *mut lua_State, p: *mut Pattern, mut size: c_int) {
    unsafe {
        let mut ud: *mut c_void = ptr::null_mut();
        let f = lua_getallocf(l, &mut ud).unwrap_unchecked();
        size += 1;
        let block = f(
            ud,
            ptr::null_mut(),
            0,
            (size as usize) * size_of::<Instruction>(),
        )
        .cast::<Instruction>();
        finishrelcode(l, p, block, size);
    }
}

pub unsafe extern "C-unwind" fn freecode(l: *mut lua_State, p: *mut Pattern) {
    unsafe {
        if !(*p).code.is_null() {
            let mut ud: *mut c_void = ptr::null_mut();
            let f = lua_getallocf(l, &mut ud).unwrap_unchecked();
            let block = (*p).code.sub(1);
            let osize = (*block).codesize as usize;
            f(
                ud,
                block.cast::<c_void>(),
                osize * size_of::<Instruction>(),
                0,
            );
        }
    }
}

unsafe fn realloccode(l: *mut lua_State, p: *mut Pattern, mut nsize: c_int) {
    unsafe {
        let mut ud: *mut c_void = ptr::null_mut();
        let f = lua_getallocf(l, &mut ud).unwrap_unchecked();
        let mut block = (*p).code.sub(1);
        let osize = (*block).codesize as usize;
        nsize += 1;
        block = f(
            ud,
            block.cast::<c_void>(),
            osize * size_of::<Instruction>(),
            (nsize as usize) * size_of::<Instruction>(),
        )
        .cast::<Instruction>();
        finishrelcode(l, p, block, nsize);
    }
}

unsafe fn nextinstruction(compst: *mut CompileState, n: c_int) -> c_int {
    unsafe {
        let size = ((*(*(*compst).p).code.sub(1)).codesize as c_int) - 1;
        let ncode = (*compst).ncode;
        if ncode > size - n {
            let nsize = (size as uint) + ((size as uint) >> 1) + (n as uint);
            if nsize >= (c_int::MAX as uint) {
                luaL_error((*compst).L, c"pattern code too large".as_ptr());
            }
            realloccode((*compst).L, (*compst).p, nsize as c_int);
        }
        (*compst).ncode = ncode + n;
        ncode
    }
}

#[inline]
unsafe fn getinstr(cs: *mut CompileState, i: c_int) -> *mut Instruction {
    unsafe { (*(*cs).p).code.add(i as usize) }
}

unsafe fn addinstruction(compst: *mut CompileState, op: Opcode, aux: c_int) -> c_int {
    unsafe {
        let i = nextinstruction(compst, 1);
        let inst = getinstr(compst, i);
        (*inst).i.code = op;
        (*inst).i.aux1 = aux as byte;
        i
    }
}

unsafe fn addoffsetinst(compst: *mut CompileState, op: Opcode) -> c_int {
    unsafe {
        let i = addinstruction(compst, op, 0);
        addinstruction(compst, 0, 0);
        debug_assert!(op == ITestSet || sizei(getinstr(compst, i)) == 2);
        i
    }
}

#[inline]
unsafe fn setoffset(compst: *mut CompileState, instruction: c_int, offset: c_int) {
    unsafe {
        (*getinstr(compst, instruction + 1)).offset = offset;
    }
}

unsafe fn codeutfr(compst: *mut CompileState, tree: *mut TTree) {
    unsafe {
        let i = addoffsetinst(compst, IUTFR);
        let to = (*sib1(tree)).u.n;
        debug_assert_eq!((*sib1(tree)).tag, TXInfo);
        (*getinstr(compst, i + 1)).offset = (*tree).u.n;
        let inst = getinstr(compst, i);
        (*inst).i.aux1 = (to & 0xff) as byte;
        (*inst).i.aux2.key = (to >> 8) as c_short;
    }
}

unsafe fn addinstcap(
    compst: *mut CompileState,
    op: Opcode,
    cap: c_int,
    key: c_int,
    aux: c_int,
) -> c_int {
    unsafe {
        let i = addinstruction(compst, op, joinkindoff(cap, aux));
        (*getinstr(compst, i)).i.aux2.key = key as c_short;
        i
    }
}

#[inline]
unsafe fn target(code: *const Instruction, i: c_int) -> c_int {
    unsafe { i + (*code.add((i + 1) as usize)).offset }
}

#[inline]
unsafe fn jumptothere(compst: *mut CompileState, instruction: c_int, target_pos: c_int) {
    if instruction >= 0 {
        unsafe {
            setoffset(compst, instruction, target_pos - instruction);
        }
    }
}

#[inline]
unsafe fn jumptohere(compst: *mut CompileState, instruction: c_int) {
    unsafe {
        jumptothere(compst, instruction, (*compst).ncode);
    }
}

unsafe fn codechar(compst: *mut CompileState, c: c_int, tt: c_int) {
    unsafe {
        if tt >= 0
            && (*getinstr(compst, tt)).i.code == ITestChar
            && c_int::from((*getinstr(compst, tt)).i.aux1) == c
        {
            addinstruction(compst, IAny, 0);
        } else {
            addinstruction(compst, IChar, c);
        }
    }
}

unsafe fn addcharset(compst: *mut CompileState, inst: c_int, info: *const charsetinfo) {
    unsafe {
        let isize = instsize((*info).size);
        {
            let i_ptr = getinstr(compst, inst);
            (*i_ptr).i.aux2.set.offset = ((*info).offset * 8) as byte;
            (*i_ptr).i.aux2.set.size = isize as byte;
            (*i_ptr).i.aux1 = (*info).deflt as byte;
        }
        let p = nextinstruction(compst, isize);
        let charset = getinstr(compst, p).cast::<byte>();
        for i in 0..((isize as usize) * size_of::<Instruction>()) {
            *charset.add(i) = getbytefromcharset(info, i as c_int);
        }
    }
}

unsafe fn cs_equal(p: *const Instruction, info: *const charsetinfo) -> bool {
    unsafe {
        if (*p).i.code != ITestSet {
            return false;
        }
        let isize = instsize((*info).size);
        if c_int::from((*p).i.aux2.set.offset) != (*info).offset * 8
            || c_int::from((*p).i.aux2.set.size) != isize
            || c_int::from((*p).i.aux1) != (*info).deflt
        {
            return false;
        }
        let buff = p.add(2).cast::<byte>();
        for i in 0..((isize as usize) * size_of::<Instruction>()) {
            if *buff.add(i) != getbytefromcharset(info, i as c_int) {
                return false;
            }
        }
        true
    }
}

unsafe fn codecharset(compst: *mut CompileState, tree: *mut TTree, tt: c_int) {
    unsafe {
        let mut info: charsetinfo = zeroed();
        tree2cset(tree, &mut info);
        if tt >= 0 && cs_equal(getinstr(compst, tt), &info) {
            addinstruction(compst, IAny, 0);
        } else {
            let i = addinstruction(compst, ISet, 0);
            addcharset(compst, i, &info);
        }
    }
}

unsafe fn codetestset(compst: *mut CompileState, cs: *const Charset, e: c_int) -> c_int {
    if e != 0 {
        NOINST
    } else {
        unsafe {
            let mut info: charsetinfo = zeroed();
            let op = charsettype((*cs).cs.as_ptr(), &mut info);
            match op {
                IFail => addoffsetinst(compst, IJmp),
                IAny => addoffsetinst(compst, ITestAny),
                IChar => {
                    let i = addoffsetinst(compst, ITestChar);
                    (*getinstr(compst, i)).i.aux1 = info.offset as byte;
                    i
                }
                _ => {
                    debug_assert_eq!(op, ISet);
                    let i = addoffsetinst(compst, ITestSet);
                    addcharset(compst, i, &info);
                    i
                }
            }
        }
    }
}

unsafe fn finaltarget(code: *const Instruction, mut i: c_int) -> c_int {
    unsafe {
        while (*code.add(i as usize)).i.code == IJmp {
            i = target(code, i);
        }
        i
    }
}

unsafe fn finallabel(code: *const Instruction, i: c_int) -> c_int {
    unsafe { finaltarget(code, target(code, i)) }
}

unsafe fn codebehind(compst: *mut CompileState, tree: *mut TTree) {
    unsafe {
        if (*tree).u.n > 0 {
            addinstruction(compst, IBehind, (*tree).u.n);
        }
        codegen(compst, sib1(tree), false, NOINST, &raw const FULLSET);
    }
}

unsafe fn codechoice(
    compst: *mut CompileState,
    p1: *mut TTree,
    p2: *mut TTree,
    opt: bool,
    fl: *const Charset,
) {
    unsafe {
        let emptyp2 = (*p2).tag == TTrue;
        let mut cs1: Charset = zeroed();
        let mut cs2: Charset = zeroed();
        let e1 = getfirst(p1, &raw const FULLSET, &mut cs1);
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
                jmp = addoffsetinst(compst, IJmp);
            }
            jumptohere(compst, test);
            codegen(compst, p2, opt, NOINST, fl);
            jumptohere(compst, jmp);
        } else if opt && emptyp2 {
            let pc = addoffsetinst(compst, IPartialCommit);
            jumptohere(compst, pc);
            codegen(compst, p1, true, NOINST, &raw const FULLSET);
        } else {
            let test = codetestset(compst, &cs1, e1);
            let pchoice = addoffsetinst(compst, IChoice);
            codegen(compst, p1, emptyp2, test, &raw const FULLSET);
            let pcommit = addoffsetinst(compst, ICommit);
            jumptohere(compst, pchoice);
            jumptohere(compst, test);
            codegen(compst, p2, opt, NOINST, fl);
            jumptohere(compst, pcommit);
        }
    }
}

unsafe fn codeand(compst: *mut CompileState, tree: *mut TTree, tt: c_int) {
    unsafe {
        let n = fixedlen(tree);
        if (0..=MAXBEHIND).contains(&n) && hascaptures(tree) == 0 {
            codegen(compst, tree, false, tt, &raw const FULLSET);
            if n > 0 {
                addinstruction(compst, IBehind, n);
            }
        } else {
            let pchoice = addoffsetinst(compst, IChoice);
            codegen(compst, tree, false, tt, &raw const FULLSET);
            let pcommit = addoffsetinst(compst, IBackCommit);
            jumptohere(compst, pchoice);
            addinstruction(compst, IFail, 0);
            jumptohere(compst, pcommit);
        }
    }
}

unsafe fn codecapture(compst: *mut CompileState, tree: *mut TTree, tt: c_int, fl: *const Charset) {
    unsafe {
        let len = fixedlen(sib1(tree));
        if (0..=MAXOFF).contains(&len) && hascaptures(sib1(tree)) == 0 {
            codegen(compst, sib1(tree), false, tt, fl);
            addinstcap(
                compst,
                IFullCapture,
                c_int::from((*tree).cap),
                c_int::from((*tree).key),
                len,
            );
        } else {
            addinstcap(
                compst,
                IOpenCapture,
                c_int::from((*tree).cap),
                c_int::from((*tree).key),
                0,
            );
            codegen(compst, sib1(tree), false, tt, fl);
            addinstcap(compst, ICloseCapture, c_int::from(Cclose), 0, 0);
        }
    }
}

unsafe fn coderuntime(compst: *mut CompileState, tree: *mut TTree, tt: c_int) {
    unsafe {
        addinstcap(
            compst,
            IOpenCapture,
            c_int::from(Cgroup),
            c_int::from((*tree).key),
            0,
        );
        codegen(compst, sib1(tree), false, tt, &raw const FULLSET);
        addinstcap(compst, ICloseRunTime, c_int::from(Cclose), 0, 0);
    }
}

unsafe fn closeloop(compst: *mut CompileState, test: c_int) {
    unsafe {
        let jmp = addoffsetinst(compst, IJmp);
        jumptohere(compst, test);
        jumptothere(compst, jmp, test);
    }
}

unsafe fn coderepcharset(compst: *mut CompileState, tree: *mut TTree) -> bool {
    unsafe {
        match (*tree).tag {
            TFalse => true,
            TAny => {
                let test = addoffsetinst(compst, ITestAny);
                addinstruction(compst, IAny, 0);
                closeloop(compst, test);
                true
            }
            TChar => {
                let test = addoffsetinst(compst, ITestChar);
                (*getinstr(compst, test)).i.aux1 = (*tree).u.n as byte;
                addinstruction(compst, IAny, 0);
                closeloop(compst, test);
                true
            }
            TSet => {
                let mut info: charsetinfo = zeroed();
                let i = addinstruction(compst, ISpan, 0);
                tree2cset(tree, &mut info);
                addcharset(compst, i, &info);
                true
            }
            _ => false,
        }
    }
}

unsafe fn coderep(compst: *mut CompileState, tree: *mut TTree, opt: bool, fl: *const Charset) {
    unsafe {
        if !coderepcharset(compst, tree) {
            let mut st: Charset = zeroed();
            let e1 = getfirst(tree, &raw const FULLSET, &mut st);
            if headfail(tree) || (e1 == 0 && cs_disjoint(&st, &*fl)) {
                let test = codetestset(compst, &st, 0);
                codegen(compst, tree, false, test, &raw const FULLSET);
                closeloop(compst, test);
            } else {
                let test = codetestset(compst, &st, e1);
                let mut pchoice = NOINST;
                if opt {
                    let pc = addoffsetinst(compst, IPartialCommit);
                    jumptohere(compst, pc);
                } else {
                    pchoice = addoffsetinst(compst, IChoice);
                }
                let l2 = (*compst).ncode;
                codegen(compst, tree, false, NOINST, &raw const FULLSET);
                let commit = addoffsetinst(compst, IPartialCommit);
                jumptothere(compst, commit, l2);
                jumptohere(compst, pchoice);
                jumptohere(compst, test);
            }
        }
    }
}

unsafe fn codenot(compst: *mut CompileState, tree: *mut TTree) {
    unsafe {
        let mut st: Charset = zeroed();
        let e = getfirst(tree, &raw const FULLSET, &mut st);
        let test = codetestset(compst, &st, e);
        if headfail(tree) {
            addinstruction(compst, IFail, 0);
        } else {
            let pchoice = addoffsetinst(compst, IChoice);
            codegen(compst, tree, false, NOINST, &raw const FULLSET);
            addinstruction(compst, IFailTwice, 0);
            jumptohere(compst, pchoice);
        }
        jumptohere(compst, test);
    }
}

unsafe fn correctcalls(compst: *mut CompileState, positions: *const c_int, from: c_int, to: c_int) {
    unsafe {
        let code = (*(*compst).p).code;
        let mut i = from;
        while i < to {
            let inst = code.add(i as usize);
            if (*inst).i.code == IOpenCall {
                let n = (*inst).i.aux2.key as isize;
                let rule = *positions.offset(n);
                debug_assert!(rule == from || (*code.add((rule - 1) as usize)).i.code == IRet);
                if (*code.add(finaltarget(code, i + 2) as usize)).i.code == IRet {
                    (*inst).i.code = IJmp;
                } else {
                    (*inst).i.code = ICall;
                }
                jumptothere(compst, i, rule);
            }
            i += sizei(inst);
        }
        debug_assert_eq!(i, to);
    }
}

unsafe fn codegrammar(compst: *mut CompileState, grammar: *mut TTree) {
    unsafe {
        let mut positions = [0 as c_int; MAXRULES];
        let mut rulenumber = 0usize;
        let firstcall = addoffsetinst(compst, ICall);
        let jumptoend = addoffsetinst(compst, IJmp);
        let start = (*compst).ncode;
        jumptohere(compst, firstcall);
        let mut rule = sib1(grammar);
        while (*rule).tag == TRule {
            let r = sib1(rule);
            debug_assert_eq!((*r).tag, TXInfo);
            positions[rulenumber] = (*compst).ncode;
            rulenumber += 1;
            codegen(compst, sib1(r), false, NOINST, &raw const FULLSET);
            addinstruction(compst, IRet, 0);
            rule = sib2(rule);
        }
        debug_assert_eq!((*rule).tag, TTrue);
        jumptohere(compst, jumptoend);
        correctcalls(compst, positions.as_ptr(), start, (*compst).ncode);
    }
}

unsafe fn codecall(compst: *mut CompileState, call: *mut TTree) {
    unsafe {
        let c = addoffsetinst(compst, IOpenCall);
        debug_assert_eq!((*sib1(sib2(call))).tag, TXInfo);
        (*getinstr(compst, c)).i.aux2.key = (*sib1(sib2(call))).u.n as c_short;
    }
}

unsafe fn codeseq1(
    compst: *mut CompileState,
    p1: *mut TTree,
    p2: *mut TTree,
    tt: c_int,
    fl: *const Charset,
) -> c_int {
    unsafe {
        if needfollow(p1) {
            let mut fl1: Charset = zeroed();
            getfirst(p2, fl, &mut fl1);
            codegen(compst, p1, false, tt, &fl1);
        } else {
            codegen(compst, p1, false, tt, &raw const FULLSET);
        }
        if fixedlen(p1) != 0 { NOINST } else { tt }
    }
}

unsafe fn codegen(
    compst: *mut CompileState,
    mut tree: *mut TTree,
    opt: bool,
    mut tt: c_int,
    fl: *const Charset,
) {
    unsafe {
        loop {
            match (*tree).tag {
                TChar => {
                    codechar(compst, (*tree).u.n, tt);
                    return;
                }
                TAny => {
                    addinstruction(compst, IAny, 0);
                    return;
                }
                TSet => {
                    codecharset(compst, tree, tt);
                    return;
                }
                TTrue => return,
                TFalse => {
                    addinstruction(compst, IFail, 0);
                    return;
                }
                TUTFR => {
                    codeutfr(compst, tree);
                    return;
                }
                TChoice => {
                    codechoice(compst, sib1(tree), sib2(tree), opt, fl);
                    return;
                }
                TRep => {
                    coderep(compst, sib1(tree), opt, fl);
                    return;
                }
                TBehind => {
                    codebehind(compst, tree);
                    return;
                }
                TNot => {
                    codenot(compst, sib1(tree));
                    return;
                }
                TAnd => {
                    codeand(compst, sib1(tree), tt);
                    return;
                }
                TCapture => {
                    codecapture(compst, tree, tt, fl);
                    return;
                }
                TRunTime => {
                    coderuntime(compst, tree, tt);
                    return;
                }
                TGrammar => {
                    codegrammar(compst, tree);
                    return;
                }
                TCall => {
                    codecall(compst, tree);
                    return;
                }
                TSeq => {
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

unsafe fn peephole(compst: *mut CompileState) {
    unsafe {
        let code = (*(*compst).p).code;
        let mut i: c_int = 0;
        while i < (*compst).ncode {
            loop {
                let inst = code.add(i as usize);
                match (*inst).i.code {
                    IChoice | ICall | ICommit | IPartialCommit | IBackCommit | ITestChar
                    | ITestSet | ITestAny => {
                        jumptothere(compst, i, finallabel(code, i));
                        break;
                    }
                    IJmp => {
                        let ft = finaltarget(code, i);
                        let ft_inst = code.add(ft as usize);
                        match (*ft_inst).i.code {
                            IRet | IFail | IFailTwice | IEnd => {
                                *inst = *ft_inst;
                                (*code.add((i + 1) as usize)).i.code = IEmpty;
                                break;
                            }
                            ICommit | IPartialCommit | IBackCommit => {
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
        debug_assert_eq!((*code.add((i - 1) as usize)).i.code, IEnd);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn compile(
    l: *mut lua_State,
    p: *mut Pattern,
    size: uint,
) -> *mut Instruction {
    unsafe {
        let mut compst = CompileState { p, ncode: 0, L: l };
        newcode(l, p, ((size / 2) + 2) as c_int);
        let tree_ptr = (&raw mut (*p).tree).cast::<TTree>();
        codegen(&mut compst, tree_ptr, false, NOINST, &raw const FULLSET);
        addinstruction(&mut compst, IEnd, 0);
        realloccode(l, p, compst.ncode);
        peephole(&mut compst);
        (*p).code
    }
}
