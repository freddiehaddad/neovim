//! LPeg virtual machine (`src/lpeg/lpvm.c`).

use std::ffi::c_char;
use std::mem::size_of;
use std::ptr;

use crate::lpeg::lpcap::{finddyncap, runtimecap};
use crate::lpeg::{
    CCLOSE, CGROUP, CRUNTIME, CapState, Capture, IANY, IBACK_COMMIT, IBEHIND, ICALL, ICHAR,
    ICHOICE, ICLOSE_CAPTURE, ICLOSE_RUN_TIME, ICOMMIT, IEND, IFAIL, IFAIL_TWICE, IFULL_CAPTURE,
    IGIVEUP, IJMP, INITCAPSIZE, IOPEN_CAPTURE, IPARTIAL_COMMIT, IRET, ISET, ISPAN, ITEST_ANY,
    ITEST_CHAR, ITEST_SET, IUTFR, Inst, InstAux2, Instruction, LpegByte, LpegIndex, LpegUint,
    MAXBACK, MAXINDT, MAXLOP, MAXSTACKIDX, caplistidx, getkind, getoff, isopencap, lua_Integer,
    lua_State, stackidx, testchar, utf_to,
};

use crate::lua::converter::{
    LUA_REGISTRYINDEX, lua_getfield, lua_gettop, lua_isboolean, lua_newuserdata, lua_pop,
    lua_pushlightuserdata, lua_remove, lua_replace, lua_settop, lua_toboolean, lua_tointeger,
    lua_touserdata, luaL_error,
};

const INITBACK: usize = MAXBACK as usize;

static GIVEUP: Instruction = Instruction {
    i: Inst {
        code: IGIVEUP,
        aux1: 0,
        aux2: InstAux2 { key: 0 },
    },
};

#[inline]
fn getoffset(p: *const Instruction) -> isize {
    unsafe { (*p.wrapping_add(1)).offset as isize }
}

pub fn charinset(i: &Instruction, buff: *const LpegByte, mut c: LpegUint) -> i32 {
    unsafe {
        c = c.wrapping_sub(LpegUint::from(i.i.aux2.set.offset));
        if c >= (LpegUint::from(i.i.aux2.set.size) * (size_of::<Instruction>() as LpegUint) * 8) {
            return i32::from(i.i.aux1);
        }
        testchar(buff, c)
    }
}

fn utf8_decode(o: *const c_char, val: &mut i32) -> *const c_char {
    const LIMITS: [LpegUint; 4] = [0xFF, 0x7F, 0x7FF, 0xFFFF];
    let mut s = o.cast::<u8>();
    unsafe {
        let mut c = LpegUint::from(*s);
        let mut res: LpegUint = 0;
        if c < 0x80 {
            res = c;
        } else {
            let mut count: usize = 0;
            while (c & 0x40) != 0 {
                count += 1;
                let cc = LpegUint::from(*s.wrapping_add(count));
                if (cc & 0xC0) != 0x80 {
                    return ptr::null();
                }
                res = (res << 6) | (cc & 0x3F);
                c <<= 1;
            }
            res |= (c & 0x7F) << (count * 5);
            if count > 3 || res > 0x10FFFF || res <= LIMITS[count] {
                return ptr::null();
            }
            s = s.wrapping_add(count);
        }
        *val = res as i32;
        s.wrapping_add(1).cast::<c_char>()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct Stack {
    s: *const c_char,
    p: *const Instruction,
    caplevel: i32,
}

#[inline]
fn getstackbase(l: *mut lua_State, ptop: i32) -> *mut Stack {
    unsafe { lua_touserdata(l, stackidx(ptop)).cast::<Stack>() }
}

const MAXNEWSIZE: usize = {
    let max_by_type = usize::MAX / size_of::<Capture>();
    let int_max = i32::MAX as usize;
    if int_max <= max_by_type {
        int_max
    } else {
        max_by_type
    }
};

fn growcap(
    l: *mut lua_State,
    capture: *mut Capture,
    capsize: &mut i32,
    captop: i32,
    n: i32,
    ptop: i32,
) -> *mut Capture {
    unsafe {
        if *capsize - captop > n {
            capture
        } else {
            let mut newsize = (captop + n + 1) as LpegUint;
            if (newsize as usize) < (MAXNEWSIZE / 3) * 2 {
                newsize += newsize / 2;
            } else if (newsize as usize) < (MAXNEWSIZE / 9) * 8 {
                newsize += newsize / 8;
            } else {
                luaL_error(l, c"too many captures".as_ptr());
            }
            let newc =
                lua_newuserdata(l, (newsize as usize) * size_of::<Capture>()).cast::<Capture>();
            ptr::copy_nonoverlapping(capture, newc, captop as usize);
            *capsize = newsize as i32;
            lua_replace(l, caplistidx(ptop));
            newc
        }
    }
}

fn doublestack(l: *mut lua_State, stacklimit: &mut *mut Stack, ptop: i32) -> *mut Stack {
    unsafe {
        let stack = getstackbase(l, ptop);
        let n = (*stacklimit).offset_from(stack) as i32;
        lua_getfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX);
        let max = lua_tointeger(l, -1) as i32;
        lua_pop(l, 1);
        if n >= max {
            luaL_error(
                l,
                c"backtrack stack overflow (current limit is %d)".as_ptr(),
                max,
            );
        }
        let mut newn = 2 * n;
        if newn > max {
            newn = max;
        }
        let newstack = lua_newuserdata(l, (newn as usize) * size_of::<Stack>()).cast::<Stack>();
        ptr::copy_nonoverlapping(stack, newstack, n as usize);
        lua_replace(l, stackidx(ptop));
        *stacklimit = newstack.wrapping_add(newn as usize);
        newstack.wrapping_add(n as usize)
    }
}

fn resdyncaptures(l: *mut lua_State, fr: i32, curr: i32, limit: i32) -> i32 {
    unsafe {
        let res: lua_Integer;
        if lua_toboolean(l, fr) == 0 {
            lua_settop(l, fr - 1);
            return -1;
        } else if lua_isboolean(l, fr) {
            res = curr as lua_Integer;
        } else {
            res = lua_tointeger(l, fr) - 1;
            if res < (curr as lua_Integer) || res > (limit as lua_Integer) {
                luaL_error(
                    l,
                    c"invalid position returned by match-time capture".as_ptr(),
                );
            }
        }
        lua_remove(l, fr);
        res as i32
    }
}

fn adddyncaptures(index: LpegIndex, capture: *mut Capture, n: i32, fd: i32) {
    unsafe {
        let prev = capture.wrapping_sub(1);
        let prev = { &mut *prev };
        debug_assert!(prev.kind == CGROUP && prev.siz == 0);
        prev.idx = 0;
        for i in 0..(n as usize) {
            let cap = capture.wrapping_add(i);
            let cap = { &mut *cap };
            cap.kind = CRUNTIME;
            cap.siz = 1;
            cap.idx = (fd + i as i32) as u16;
            cap.index = index;
        }
        let close = capture.wrapping_add(n as usize);
        let close = { &mut *close };
        close.kind = CCLOSE;
        close.siz = 1;
        close.index = index;
    }
}

fn removedyncap(l: *mut lua_State, capture: *mut Capture, level: i32, last: i32) -> i32 {
    unsafe {
        let id = finddyncap(
            capture.wrapping_add(level as usize),
            capture.wrapping_add(last as usize),
        );
        let top = lua_gettop(l);
        if id == 0 {
            return 0;
        }
        lua_settop(l, id - 1);
        top - id + 1
    }
}

fn findopen_full(mut cap: *mut Capture, currindex: LpegIndex) -> *mut Capture {
    unsafe {
        cap = cap.wrapping_sub(1);
        if !isopencap(&*cap) && (*cap).index == currindex {
            return ptr::null_mut();
        }
        for _ in 0..MAXLOP {
            if currindex - (*cap).index >= LpegIndex::from(u8::MAX) {
                return ptr::null_mut();
            } else if isopencap(&*cap) {
                return cap;
            } else if (*cap).kind == CCLOSE {
                return ptr::null_mut();
            }
            cap = cap.wrapping_sub(1);
        }
    }
    ptr::null_mut()
}

pub fn r#match(
    l: *mut lua_State,
    o: *const c_char,
    mut s: *const c_char,
    e: *const c_char,
    op: *mut Instruction,
    mut capture: *mut Capture,
    ptop: i32,
) -> *const c_char {
    unsafe {
        let mut stackbase = [Stack {
            s: ptr::null(),
            p: ptr::null(),
            caplevel: 0,
        }; INITBACK];
        let mut stacklimit = stackbase.as_mut_ptr().wrapping_add(INITBACK);
        let mut stack = stackbase.as_mut_ptr();
        let mut capsize = INITCAPSIZE as i32;
        let mut captop: i32 = 0;
        let mut ndyncap: i32 = 0;
        let mut p: *const Instruction = op;

        (*stack).p = &raw const GIVEUP;
        (*stack).s = s;
        (*stack).caplevel = 0;
        stack = stack.wrapping_add(1);
        lua_pushlightuserdata(l, stackbase.as_mut_ptr().cast());

        loop {
            debug_assert!(stackidx(ptop) + ndyncap == lua_gettop(l) && ndyncap <= captop);

            match (*p).i.code {
                IEND => {
                    debug_assert_eq!(stack, getstackbase(l, ptop).wrapping_add(1));
                    let cap = capture.wrapping_add(captop as usize);
                    (*cap).kind = CCLOSE;
                    (*cap).index = MAXINDT;
                    return s;
                }
                IGIVEUP => {
                    debug_assert_eq!(stack, getstackbase(l, ptop));
                    return ptr::null();
                }
                IRET => {
                    debug_assert!(
                        stack > getstackbase(l, ptop) && (*stack.wrapping_sub(1)).s.is_null()
                    );
                    stack = stack.wrapping_sub(1);
                    p = (*stack).p;
                    continue;
                }
                IANY => {
                    if s < e {
                        p = p.wrapping_add(1);
                        s = s.wrapping_add(1);
                        continue;
                    }
                }
                IUTFR => {
                    if s < e {
                        let mut codepoint: i32 = 0;
                        s = utf8_decode(s, &mut codepoint);
                        if !s.is_null()
                            && (*p.wrapping_add(1)).offset <= codepoint
                            && codepoint <= utf_to(&*p)
                        {
                            p = p.wrapping_add(2);
                            continue;
                        }
                    }
                }
                ITEST_ANY => {
                    if s < e {
                        p = p.wrapping_add(2);
                    } else {
                        p = p.wrapping_offset(getoffset(p));
                    }
                    continue;
                }
                ICHAR => {
                    if s < e && (*s as LpegByte) == (*p).i.aux1 {
                        p = p.wrapping_add(1);
                        s = s.wrapping_add(1);
                        continue;
                    }
                }
                ITEST_CHAR => {
                    if s < e && (*s as LpegByte) == (*p).i.aux1 {
                        p = p.wrapping_add(2);
                    } else {
                        p = p.wrapping_offset(getoffset(p));
                    }
                    continue;
                }
                ISET => {
                    if s < e {
                        let c = LpegUint::from(*s as LpegByte);
                        if charinset(&*p, p.wrapping_add(1).cast::<LpegByte>(), c) != 0 {
                            p = p.wrapping_add(1 + (*p).i.aux2.set.size as usize);
                            s = s.wrapping_add(1);
                            continue;
                        }
                    }
                }
                ITEST_SET => {
                    if s < e {
                        let c = LpegUint::from(*s as LpegByte);
                        if charinset(&*p, p.wrapping_add(2).cast::<LpegByte>(), c) != 0 {
                            p = p.wrapping_add(2 + (*p).i.aux2.set.size as usize);
                            continue;
                        }
                    }
                    p = p.wrapping_offset(getoffset(p));
                    continue;
                }
                IBEHIND => {
                    let n = (*p).i.aux1 as isize;
                    if n <= s.offset_from(o) {
                        s = s.wrapping_offset(-n);
                        p = p.wrapping_add(1);
                        continue;
                    }
                }
                ISPAN => {
                    while s < e {
                        let c = LpegUint::from(*s as LpegByte);
                        if charinset(&*p, p.wrapping_add(1).cast::<LpegByte>(), c) == 0 {
                            break;
                        }
                        s = s.wrapping_add(1);
                    }
                    p = p.wrapping_add(1 + (*p).i.aux2.set.size as usize);
                    continue;
                }
                IJMP => {
                    p = p.wrapping_offset(getoffset(p));
                    continue;
                }
                ICHOICE => {
                    if stack == stacklimit {
                        stack = doublestack(l, &mut stacklimit, ptop);
                    }
                    (*stack).p = p.wrapping_offset(getoffset(p));
                    (*stack).s = s;
                    (*stack).caplevel = captop;
                    stack = stack.wrapping_add(1);
                    p = p.wrapping_add(2);
                    continue;
                }
                ICALL => {
                    if stack == stacklimit {
                        stack = doublestack(l, &mut stacklimit, ptop);
                    }
                    (*stack).s = ptr::null();
                    (*stack).p = p.wrapping_add(2);
                    stack = stack.wrapping_add(1);
                    p = p.wrapping_offset(getoffset(p));
                    continue;
                }
                ICOMMIT => {
                    debug_assert!(
                        stack > getstackbase(l, ptop) && !(*stack.wrapping_sub(1)).s.is_null()
                    );
                    stack = stack.wrapping_sub(1);
                    p = p.wrapping_offset(getoffset(p));
                    continue;
                }
                IPARTIAL_COMMIT => {
                    debug_assert!(
                        stack > getstackbase(l, ptop) && !(*stack.wrapping_sub(1)).s.is_null()
                    );
                    (*stack.wrapping_sub(1)).s = s;
                    (*stack.wrapping_sub(1)).caplevel = captop;
                    p = p.wrapping_offset(getoffset(p));
                    continue;
                }
                IBACK_COMMIT => {
                    debug_assert!(
                        stack > getstackbase(l, ptop) && !(*stack.wrapping_sub(1)).s.is_null()
                    );
                    stack = stack.wrapping_sub(1);
                    s = (*stack).s;
                    if ndyncap > 0 {
                        ndyncap -= removedyncap(l, capture, (*stack).caplevel, captop);
                    }
                    captop = (*stack).caplevel;
                    p = p.wrapping_offset(getoffset(p));
                    continue;
                }
                IFAIL_TWICE => {
                    debug_assert!(stack > getstackbase(l, ptop));
                    stack = stack.wrapping_sub(1);
                }
                IFAIL => {}
                ICLOSE_RUN_TIME => {
                    let mut cs = CapState {
                        cap: ptr::null_mut(),
                        ocap: capture,
                        lstate: l,
                        ptop,
                        firstcap: 0,
                        s: o,
                        valuecached: 0,
                        reclevel: 0,
                    };
                    let mut rem: i32 = 0;
                    let mut fr = lua_gettop(l) + 1;
                    let mut n =
                        runtimecap(&mut cs, capture.wrapping_add(captop as usize), s, &mut rem);
                    captop -= n;
                    ndyncap -= rem;
                    fr -= rem;
                    let res =
                        resdyncaptures(l, fr, s.offset_from(o) as i32, e.offset_from(o) as i32);
                    if res != -1 {
                        s = o.wrapping_add(res as usize);
                        n = lua_gettop(l) - fr + 1;
                        ndyncap += n;
                        if n == 0 {
                            captop -= 1;
                        } else {
                            if fr + n >= i32::from(i16::MAX) {
                                luaL_error(l, c"too many results in match-time capture".as_ptr());
                            }
                            capture = growcap(l, capture, &mut capsize, captop, n + 1, ptop);
                            adddyncaptures(
                                s.offset_from(o) as LpegIndex,
                                capture.wrapping_add(captop as usize),
                                n,
                                fr,
                            );
                            captop += n + 1;
                        }
                        p = p.wrapping_add(1);
                        continue;
                    }
                }
                ICLOSE_CAPTURE => {
                    let curr_idx = s.offset_from(o) as LpegIndex;
                    let open = findopen_full(capture.wrapping_add(captop as usize), curr_idx);
                    debug_assert!(captop > 0);
                    if !open.is_null() {
                        let open = { &mut *open };
                        open.siz = (curr_idx - open.index + 1) as LpegByte;
                        p = p.wrapping_add(1);
                        continue;
                    } else {
                        let cap = capture.wrapping_add(captop as usize);
                        (*cap).siz = 1;
                        (*cap).index = curr_idx;
                        (*cap).idx = (*p).i.aux2.key as u16;
                        (*cap).kind = getkind(&*p) as LpegByte;
                        captop += 1;
                        capture = growcap(l, capture, &mut capsize, captop, 0, ptop);
                        p = p.wrapping_add(1);
                        continue;
                    }
                }
                IOPEN_CAPTURE => {
                    let cap = capture.wrapping_add(captop as usize);
                    (*cap).siz = 0;
                    (*cap).index = s.offset_from(o) as LpegIndex;
                    (*cap).idx = (*p).i.aux2.key as u16;
                    (*cap).kind = getkind(&*p) as LpegByte;
                    captop += 1;
                    capture = growcap(l, capture, &mut capsize, captop, 0, ptop);
                    p = p.wrapping_add(1);
                    continue;
                }
                IFULL_CAPTURE => {
                    let off = getoff(&*p);
                    let cap = capture.wrapping_add(captop as usize);
                    (*cap).siz = (off + 1) as LpegByte;
                    (*cap).index = (s.offset_from(o) as i32 - off) as LpegIndex;
                    (*cap).idx = (*p).i.aux2.key as u16;
                    (*cap).kind = getkind(&*p) as LpegByte;
                    captop += 1;
                    capture = growcap(l, capture, &mut capsize, captop, 0, ptop);
                    p = p.wrapping_add(1);
                    continue;
                }
                _ => {
                    debug_assert!(false);
                    return ptr::null();
                }
            }

            loop {
                debug_assert!(stack > getstackbase(l, ptop));
                stack = stack.wrapping_sub(1);
                s = (*stack).s;
                if !s.is_null() {
                    break;
                }
            }
            if ndyncap > 0 {
                ndyncap -= removedyncap(l, capture, (*stack).caplevel, captop);
            }
            captop = (*stack).caplevel;
            p = (*stack).p;
        }
    }
}
