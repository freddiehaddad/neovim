//! LPeg virtual machine (`src/lpeg/lpvm.c`).

use std::ffi::{c_char, c_int, c_short, c_ushort, c_void};
use std::mem::size_of;
use std::ptr;

use crate::lpeg::lpcap::{finddyncap, runtimecap};
use crate::lpeg::{
    CapState, Capture, Cclose, Cgroup, Cruntime, IAny, IBackCommit, IBehind, ICall, IChar, IChoice,
    ICloseCapture, ICloseRunTime, ICommit, IEnd, IFail, IFailTwice, IFullCapture, IGiveup, IJmp,
    INITCAPSIZE, IOpenCapture, IPartialCommit, IRet, ISet, ISpan, ITestAny, ITestChar, ITestSet,
    IUTFR, Index_t, Inst, InstAux2, Instruction, MAXBACK, MAXINDT, MAXLOP, MAXSTACKIDX, byte,
    caplistidx, getkind, getoff, isopencap, lua_Integer, lua_State, stackidx, testchar, uint,
    utf_to,
};

use crate::lua::converter::{
    LUA_REGISTRYINDEX, lua_getfield, lua_gettop, lua_isboolean, lua_newuserdata, lua_pop,
    lua_pushlightuserdata, lua_remove, lua_replace, lua_settop, lua_toboolean, lua_tointeger,
    lua_touserdata, luaL_error,
};

const INITBACK: usize = MAXBACK as usize;

static GIVEUP: Instruction = Instruction {
    i: Inst {
        code: IGiveup,
        aux1: 0,
        aux2: InstAux2 { key: 0 },
    },
};

#[inline]
unsafe fn getoffset(p: *const Instruction) -> isize {
    unsafe { (*p.add(1)).offset as isize }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn charinset(i: *const Instruction, buff: *const byte, mut c: uint) -> c_int {
    unsafe {
        c = c.wrapping_sub(uint::from((*i).i.aux2.set.offset));
        if c >= (uint::from((*i).i.aux2.set.size) * (size_of::<Instruction>() as uint) * 8) {
            return c_int::from((*i).i.aux1);
        }
        testchar(buff, c)
    }
}

unsafe fn utf8_decode(o: *const c_char, val: *mut c_int) -> *const c_char {
    const LIMITS: [uint; 4] = [0xFF, 0x7F, 0x7FF, 0xFFFF];
    let mut s = o.cast::<u8>();
    unsafe {
        let mut c = uint::from(*s);
        let mut res: uint = 0;
        if c < 0x80 {
            res = c;
        } else {
            let mut count: usize = 0;
            while (c & 0x40) != 0 {
                count += 1;
                let cc = uint::from(*s.add(count));
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
            s = s.add(count);
        }
        *val = res as c_int;
        s.add(1).cast::<c_char>()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct Stack {
    s: *const c_char,
    p: *const Instruction,
    caplevel: c_int,
}

#[inline]
unsafe fn getstackbase(l: *mut lua_State, ptop: c_int) -> *mut Stack {
    unsafe { lua_touserdata(l, stackidx(ptop)).cast::<Stack>() }
}

const MAXNEWSIZE: usize = {
    let max_by_type = usize::MAX / size_of::<Capture>();
    let int_max = c_int::MAX as usize;
    if int_max <= max_by_type {
        int_max
    } else {
        max_by_type
    }
};

unsafe fn growcap(
    l: *mut lua_State,
    capture: *mut Capture,
    capsize: *mut c_int,
    captop: c_int,
    n: c_int,
    ptop: c_int,
) -> *mut Capture {
    unsafe {
        if *capsize - captop > n {
            capture
        } else {
            let mut newsize = (captop + n + 1) as uint;
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
            *capsize = newsize as c_int;
            lua_replace(l, caplistidx(ptop));
            newc
        }
    }
}

unsafe fn doublestack(l: *mut lua_State, stacklimit: *mut *mut Stack, ptop: c_int) -> *mut Stack {
    unsafe {
        let stack = getstackbase(l, ptop);
        let n = (*stacklimit).offset_from(stack) as c_int;
        lua_getfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX.as_ptr());
        let max = lua_tointeger(l, -1) as c_int;
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
        *stacklimit = newstack.add(newn as usize);
        newstack.add(n as usize)
    }
}

unsafe fn resdyncaptures(l: *mut lua_State, fr: c_int, curr: c_int, limit: c_int) -> c_int {
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
        res as c_int
    }
}

unsafe fn adddyncaptures(index: Index_t, capture: *mut Capture, n: c_int, fd: c_int) {
    unsafe {
        let prev = capture.sub(1);
        debug_assert!((*prev).kind == Cgroup && (*prev).siz == 0);
        (*prev).idx = 0;
        for i in 0..(n as usize) {
            let cap = capture.add(i);
            (*cap).kind = Cruntime;
            (*cap).siz = 1;
            (*cap).idx = (fd + i as c_int) as c_ushort;
            (*cap).index = index;
        }
        let close = capture.add(n as usize);
        (*close).kind = Cclose;
        (*close).siz = 1;
        (*close).index = index;
    }
}

unsafe fn removedyncap(
    l: *mut lua_State,
    capture: *mut Capture,
    level: c_int,
    last: c_int,
) -> c_int {
    unsafe {
        let id = finddyncap(capture.add(level as usize), capture.add(last as usize));
        let top = lua_gettop(l);
        if id == 0 {
            return 0;
        }
        lua_settop(l, id - 1);
        top - id + 1
    }
}

unsafe fn findopen_full(mut cap: *mut Capture, currindex: Index_t) -> *mut Capture {
    unsafe {
        cap = cap.sub(1);
        if !isopencap(cap) && (*cap).index == currindex {
            return ptr::null_mut();
        }
        for _ in 0..MAXLOP {
            if currindex - (*cap).index >= Index_t::from(u8::MAX) {
                return ptr::null_mut();
            } else if isopencap(cap) {
                return cap;
            } else if (*cap).kind == Cclose {
                return ptr::null_mut();
            }
            cap = cap.sub(1);
        }
    }
    ptr::null_mut()
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C-unwind" fn r#match(
    l: *mut lua_State,
    o: *const c_char,
    mut s: *const c_char,
    e: *const c_char,
    op: *mut Instruction,
    mut capture: *mut Capture,
    ptop: c_int,
) -> *const c_char {
    unsafe {
        let mut stackbase = [Stack {
            s: ptr::null(),
            p: ptr::null(),
            caplevel: 0,
        }; INITBACK];
        let mut stacklimit = stackbase.as_mut_ptr().add(INITBACK);
        let mut stack = stackbase.as_mut_ptr();
        let mut capsize = INITCAPSIZE as c_int;
        let mut captop: c_int = 0;
        let mut ndyncap: c_int = 0;
        let mut p: *const Instruction = op;

        (*stack).p = &raw const GIVEUP;
        (*stack).s = s;
        (*stack).caplevel = 0;
        stack = stack.add(1);
        lua_pushlightuserdata(l, stackbase.as_mut_ptr().cast::<c_void>());

        loop {
            debug_assert!(stackidx(ptop) + ndyncap == lua_gettop(l) && ndyncap <= captop);

            match (*p).i.code {
                IEnd => {
                    debug_assert_eq!(stack, getstackbase(l, ptop).add(1));
                    let cap = capture.add(captop as usize);
                    (*cap).kind = Cclose;
                    (*cap).index = MAXINDT;
                    return s;
                }
                IGiveup => {
                    debug_assert_eq!(stack, getstackbase(l, ptop));
                    return ptr::null();
                }
                IRet => {
                    debug_assert!(stack > getstackbase(l, ptop) && (*stack.sub(1)).s.is_null());
                    stack = stack.sub(1);
                    p = (*stack).p;
                    continue;
                }
                IAny => {
                    if s < e {
                        p = p.add(1);
                        s = s.add(1);
                        continue;
                    }
                }
                IUTFR => {
                    if s < e {
                        let mut codepoint: c_int = 0;
                        s = utf8_decode(s, &mut codepoint);
                        if !s.is_null() && (*p.add(1)).offset <= codepoint && codepoint <= utf_to(p)
                        {
                            p = p.add(2);
                            continue;
                        }
                    }
                }
                ITestAny => {
                    if s < e {
                        p = p.add(2);
                    } else {
                        p = p.offset(getoffset(p));
                    }
                    continue;
                }
                IChar => {
                    if s < e && (*s as byte) == (*p).i.aux1 {
                        p = p.add(1);
                        s = s.add(1);
                        continue;
                    }
                }
                ITestChar => {
                    if s < e && (*s as byte) == (*p).i.aux1 {
                        p = p.add(2);
                    } else {
                        p = p.offset(getoffset(p));
                    }
                    continue;
                }
                ISet => {
                    if s < e {
                        let c = uint::from(*s as byte);
                        if charinset(p, p.add(1).cast::<byte>(), c) != 0 {
                            p = p.add(1 + (*p).i.aux2.set.size as usize);
                            s = s.add(1);
                            continue;
                        }
                    }
                }
                ITestSet => {
                    if s < e {
                        let c = uint::from(*s as byte);
                        if charinset(p, p.add(2).cast::<byte>(), c) != 0 {
                            p = p.add(2 + (*p).i.aux2.set.size as usize);
                            continue;
                        }
                    }
                    p = p.offset(getoffset(p));
                    continue;
                }
                IBehind => {
                    let n = (*p).i.aux1 as isize;
                    if n <= s.offset_from(o) {
                        s = s.offset(-n);
                        p = p.add(1);
                        continue;
                    }
                }
                ISpan => {
                    while s < e {
                        let c = uint::from(*s as byte);
                        if charinset(p, p.add(1).cast::<byte>(), c) == 0 {
                            break;
                        }
                        s = s.add(1);
                    }
                    p = p.add(1 + (*p).i.aux2.set.size as usize);
                    continue;
                }
                IJmp => {
                    p = p.offset(getoffset(p));
                    continue;
                }
                IChoice => {
                    if stack == stacklimit {
                        stack = doublestack(l, &mut stacklimit, ptop);
                    }
                    (*stack).p = p.offset(getoffset(p));
                    (*stack).s = s;
                    (*stack).caplevel = captop;
                    stack = stack.add(1);
                    p = p.add(2);
                    continue;
                }
                ICall => {
                    if stack == stacklimit {
                        stack = doublestack(l, &mut stacklimit, ptop);
                    }
                    (*stack).s = ptr::null();
                    (*stack).p = p.add(2);
                    stack = stack.add(1);
                    p = p.offset(getoffset(p));
                    continue;
                }
                ICommit => {
                    debug_assert!(stack > getstackbase(l, ptop) && !(*stack.sub(1)).s.is_null());
                    stack = stack.sub(1);
                    p = p.offset(getoffset(p));
                    continue;
                }
                IPartialCommit => {
                    debug_assert!(stack > getstackbase(l, ptop) && !(*stack.sub(1)).s.is_null());
                    (*stack.sub(1)).s = s;
                    (*stack.sub(1)).caplevel = captop;
                    p = p.offset(getoffset(p));
                    continue;
                }
                IBackCommit => {
                    debug_assert!(stack > getstackbase(l, ptop) && !(*stack.sub(1)).s.is_null());
                    stack = stack.sub(1);
                    s = (*stack).s;
                    if ndyncap > 0 {
                        ndyncap -= removedyncap(l, capture, (*stack).caplevel, captop);
                    }
                    captop = (*stack).caplevel;
                    p = p.offset(getoffset(p));
                    continue;
                }
                IFailTwice => {
                    debug_assert!(stack > getstackbase(l, ptop));
                    stack = stack.sub(1);
                }
                IFail => {}
                ICloseRunTime => {
                    let mut cs = CapState {
                        cap: ptr::null_mut(),
                        ocap: capture,
                        L: l,
                        ptop,
                        firstcap: 0,
                        s: o,
                        valuecached: 0,
                        reclevel: 0,
                    };
                    let mut rem: c_int = 0;
                    let mut fr = lua_gettop(l) + 1;
                    let mut n = runtimecap(&mut cs, capture.add(captop as usize), s, &mut rem);
                    captop -= n;
                    ndyncap -= rem;
                    fr -= rem;
                    let res =
                        resdyncaptures(l, fr, s.offset_from(o) as c_int, e.offset_from(o) as c_int);
                    if res != -1 {
                        s = o.add(res as usize);
                        n = lua_gettop(l) - fr + 1;
                        ndyncap += n;
                        if n == 0 {
                            captop -= 1;
                        } else {
                            if fr + n >= c_int::from(c_short::MAX) {
                                luaL_error(l, c"too many results in match-time capture".as_ptr());
                            }
                            capture = growcap(l, capture, &mut capsize, captop, n + 1, ptop);
                            adddyncaptures(
                                s.offset_from(o) as Index_t,
                                capture.add(captop as usize),
                                n,
                                fr,
                            );
                            captop += n + 1;
                        }
                        p = p.add(1);
                        continue;
                    }
                }
                ICloseCapture => {
                    let curr_idx = s.offset_from(o) as Index_t;
                    let open = findopen_full(capture.add(captop as usize), curr_idx);
                    debug_assert!(captop > 0);
                    if !open.is_null() {
                        (*open).siz = (curr_idx - (*open).index + 1) as byte;
                        p = p.add(1);
                        continue;
                    } else {
                        let cap = capture.add(captop as usize);
                        (*cap).siz = 1;
                        (*cap).index = curr_idx;
                        (*cap).idx = (*p).i.aux2.key as c_ushort;
                        (*cap).kind = getkind(p) as byte;
                        captop += 1;
                        capture = growcap(l, capture, &mut capsize, captop, 0, ptop);
                        p = p.add(1);
                        continue;
                    }
                }
                IOpenCapture => {
                    let cap = capture.add(captop as usize);
                    (*cap).siz = 0;
                    (*cap).index = s.offset_from(o) as Index_t;
                    (*cap).idx = (*p).i.aux2.key as c_ushort;
                    (*cap).kind = getkind(p) as byte;
                    captop += 1;
                    capture = growcap(l, capture, &mut capsize, captop, 0, ptop);
                    p = p.add(1);
                    continue;
                }
                IFullCapture => {
                    let off = getoff(p);
                    let cap = capture.add(captop as usize);
                    (*cap).siz = (off + 1) as byte;
                    (*cap).index = (s.offset_from(o) as c_int - off) as Index_t;
                    (*cap).idx = (*p).i.aux2.key as c_ushort;
                    (*cap).kind = getkind(p) as byte;
                    captop += 1;
                    capture = growcap(l, capture, &mut capsize, captop, 0, ptop);
                    p = p.add(1);
                    continue;
                }
                _ => {
                    debug_assert!(false);
                    return ptr::null();
                }
            }

            loop {
                debug_assert!(stack > getstackbase(l, ptop));
                stack = stack.sub(1);
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
