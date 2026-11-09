//! LPeg virtual machine (`src/lpeg/lpvm.c`).
#![forbid(unsafe_code)]

use crate::lpeg::lpcap::{finddyncap, runtimecap};
use crate::lpeg::{
    CCLOSE, CGROUP, CRUNTIME, Capture, IANY, IBACK_COMMIT, IBEHIND, ICALL, ICHAR, ICHOICE,
    ICLOSE_CAPTURE, ICLOSE_RUN_TIME, ICOMMIT, IEND, IFAIL, IFAIL_TWICE, IFULL_CAPTURE, IGIVEUP,
    IJMP, INST_BUFF_SIZE, IOPEN_CAPTURE, IPARTIAL_COMMIT, IRET, ISET, ISPAN, ITEST_ANY, ITEST_CHAR,
    ITEST_SET, IUTFR, Instruction, LpegByte, LpegIndex, LpegUint, MAXBACK, MAXINDT, MAXLOP,
    MAXSTACKIDX, caplistidx, getkind, getoff, isopencap, lua_Integer, lua_State, stackidx, utf_to,
};

const INITBACK: usize = MAXBACK as usize;
const GIVEUP_PC: usize = usize::MAX;

#[derive(Copy, Clone, Default)]
pub(crate) struct Stack {
    s: Option<usize>,
    p: usize,
    caplevel: usize,
}

mod ffi {
    use std::ffi::CStr;

    use super::Stack;
    use crate::lpeg::{Capture, MAXSTACKIDX, caplistidx, lua_Integer, lua_State, stackidx};
    use crate::lua::converter::{LUA_REGISTRYINDEX, lua_getfield, lua_isboolean, lua_pop};
    use crate::lua::ffi::{
        lua_newuserdata_slice_copy, lua_pushnil_safe, lua_remove_safe, lua_settop_safe,
        lua_toboolean_safe, lua_tointeger_safe, luaL_error_cstr, luaL_error_int, nlua_gettop,
    };

    #[inline]
    pub fn push_nil_stack_slot(l: *mut lua_State) {
        lua_pushnil_safe(l);
    }

    #[inline]
    pub fn gettop(l: *mut lua_State) -> i32 {
        nlua_gettop(l)
    }

    #[inline]
    pub fn settop(l: *mut lua_State, idx: i32) {
        lua_settop_safe(l, idx);
    }

    #[inline]
    pub fn remove(l: *mut lua_State, idx: i32) {
        lua_remove_safe(l, idx);
    }

    #[inline]
    pub fn toboolean(l: *mut lua_State, idx: i32) -> bool {
        lua_toboolean_safe(l, idx)
    }

    #[inline]
    pub fn isboolean(l: *mut lua_State, idx: i32) -> bool {
        lua_isboolean(l, idx)
    }

    #[inline]
    pub fn tointeger(l: *mut lua_State, idx: i32) -> lua_Integer {
        lua_tointeger_safe(l, idx)
    }

    #[inline]
    pub fn error_msg(l: *mut lua_State, msg: &CStr) -> ! {
        luaL_error_cstr(l, msg);
        unreachable!()
    }

    #[inline]
    pub fn error_int(l: *mut lua_State, fmt: &CStr, val: i32) -> ! {
        luaL_error_int(l, fmt, val)
    }

    #[inline]
    pub fn get_maxstack(l: *mut lua_State) -> usize {
        lua_getfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX);
        let max = lua_tointeger_safe(l, -1) as usize;
        lua_pop(l, 1);
        max
    }

    #[inline]
    pub fn alloc_capture_slice<'a>(
        l: *mut lua_State,
        old: &[Capture],
        captop: usize,
        newsize: usize,
        ptop: i32,
    ) -> &'a mut [Capture] {
        lua_newuserdata_slice_copy(l, old, captop, newsize, Capture::new(), caplistidx(ptop))
    }

    #[inline]
    pub fn alloc_stack_slice<'a>(
        l: *mut lua_State,
        old: &[Stack],
        n: usize,
        newn: usize,
        ptop: i32,
    ) -> &'a mut [Stack] {
        lua_newuserdata_slice_copy(l, old, n, newn, Stack::default(), stackidx(ptop))
    }
}

use self::ffi::*;

#[inline]
fn getoffset(code: &[Instruction], pc: usize) -> isize {
    code[pc + 1].offset as isize
}

#[inline]
fn advance_pc(pc: usize, offset: isize) -> usize {
    (pc as isize + offset) as usize
}

pub fn charinset(inst: &Instruction, charset_insts: &[Instruction], mut c: LpegUint) -> i32 {
    c = c.wrapping_sub(LpegUint::from(inst.set_offset));
    if c >= (LpegUint::from(inst.set_size) * (INST_BUFF_SIZE as LpegUint) * 8) {
        return i32::from(inst.aux1);
    }
    let byte_idx = (c >> 3) as usize;
    let b = charset_insts[byte_idx / INST_BUFF_SIZE].buff[byte_idx % INST_BUFF_SIZE];
    (((b as LpegUint) >> (c & 7)) & 1) as i32
}

fn utf8_decode(s: &[u8], val: &mut i32) -> Option<usize> {
    const LIMITS: [LpegUint; 4] = [0xFF, 0x7F, 0x7FF, 0xFFFF];
    let mut c = LpegUint::from(*s.first()?);
    let mut res: LpegUint = 0;
    if c < 0x80 {
        *val = c as i32;
        return Some(1);
    }
    let mut count: usize = 0;
    while (c & 0x40) != 0 {
        count += 1;
        let cc = LpegUint::from(*s.get(count)?);
        if (cc & 0xC0) != 0x80 {
            return None;
        }
        res = (res << 6) | (cc & 0x3F);
        c <<= 1;
    }
    res |= (c & 0x7F) << (count * 5);
    if count > 3 || res > 0x10FFFF || res <= LIMITS[count] {
        return None;
    }
    *val = res as i32;
    Some(count + 1)
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
    capture: &mut [Capture],
    captop: usize,
    n: usize,
    ptop: i32,
) -> &mut [Capture] {
    let _ = (MAXSTACKIDX, caplistidx);
    if capture.len() - captop > n {
        capture
    } else {
        let mut newsize = captop + n + 1;
        if newsize < (MAXNEWSIZE / 3) * 2 {
            newsize += newsize / 2;
        } else if newsize < (MAXNEWSIZE / 9) * 8 {
            newsize += newsize / 8;
        } else {
            error_msg(l, c"too many captures");
        }
        alloc_capture_slice(l, capture, captop, newsize, ptop)
    }
}

fn doublestack(l: *mut lua_State, stack: &mut [Stack], sp: usize, ptop: i32) -> &mut [Stack] {
    let max = get_maxstack(l);
    if sp >= max {
        error_int(
            l,
            c"backtrack stack overflow (current limit is %d)",
            max as i32,
        );
    }
    let mut newn = 2 * sp;
    if newn > max {
        newn = max;
    }
    alloc_stack_slice(l, stack, sp, newn, ptop)
}

fn resdyncaptures(l: *mut lua_State, fr: i32, curr: usize, limit: usize) -> Option<usize> {
    let res: lua_Integer;
    if !toboolean(l, fr) {
        settop(l, fr - 1);
        return None;
    } else if isboolean(l, fr) {
        res = curr as lua_Integer;
    } else {
        res = tointeger(l, fr) - 1;
        if res < (curr as lua_Integer) || res > (limit as lua_Integer) {
            error_msg(l, c"invalid position returned by match-time capture");
        }
    }
    remove(l, fr);
    Some(res as usize)
}

fn adddyncaptures(index: LpegIndex, capture: &mut [Capture], captop: usize, n: usize, fd: i32) {
    debug_assert!(capture[captop - 1].kind == CGROUP && capture[captop - 1].siz == 0);
    capture[captop - 1].idx = 0;
    for i in 0..n {
        let cap = &mut capture[captop + i];
        cap.kind = CRUNTIME;
        cap.siz = 1;
        cap.idx = (fd + i as i32) as u16;
        cap.index = index;
    }
    let close = &mut capture[captop + n];
    close.kind = CCLOSE;
    close.siz = 1;
    close.index = index;
}

fn removedyncap(l: *mut lua_State, capture: &[Capture], level: usize, last: usize) -> i32 {
    let id = finddyncap(capture, level, last);
    let top = gettop(l);
    if id == 0 {
        return 0;
    }
    settop(l, id - 1);
    top - id + 1
}

fn findopen_full(capture: &[Capture], mut idx: usize, currindex: LpegIndex) -> Option<usize> {
    idx -= 1;
    if !isopencap(&capture[idx]) && capture[idx].index == currindex {
        return None;
    }
    for _ in 0..MAXLOP {
        if currindex - capture[idx].index >= LpegIndex::from(u8::MAX) {
            return None;
        } else if isopencap(&capture[idx]) {
            return Some(idx);
        } else if capture[idx].kind == CCLOSE {
            return None;
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    None
}

pub fn r#match<'a>(
    l: *mut lua_State,
    subject: &[u8],
    mut s: usize,
    code: &[Instruction],
    mut capture: &'a mut [Capture],
    ptop: i32,
) -> Option<(usize, &'a [Capture])> {
    let mut stackbase = [Stack::default(); INITBACK];
    let mut stack: &mut [Stack] = &mut stackbase;
    let mut sp: usize = 0;
    let mut captop: usize = 0;
    let mut ndyncap: i32 = 0;
    let mut p: usize = 0;
    let e = subject.len();

    stack[sp] = Stack {
        p: GIVEUP_PC,
        s: Some(s),
        caplevel: 0,
    };
    sp += 1;
    push_nil_stack_slot(l);

    loop {
        debug_assert!(stackidx(ptop) + ndyncap == gettop(l) && (ndyncap as usize) <= captop);

        if p == GIVEUP_PC {
            debug_assert_eq!(sp, 0);
            return None;
        }

        let inst = &code[p];
        match inst.code {
            IEND => {
                debug_assert_eq!(sp, 1);
                capture[captop].kind = CCLOSE;
                capture[captop].index = MAXINDT;
                return Some((s, capture));
            }
            IGIVEUP => {
                debug_assert_eq!(sp, 0);
                return None;
            }
            IRET => {
                debug_assert!(sp > 0 && stack[sp - 1].s.is_none());
                sp -= 1;
                p = stack[sp].p;
                continue;
            }
            IANY => {
                if s < e {
                    p += 1;
                    s += 1;
                    continue;
                }
            }
            IUTFR => {
                if s < e {
                    let mut codepoint: i32 = 0;
                    if let Some(adv) = utf8_decode(&subject[s..], &mut codepoint) {
                        s += adv;
                        if code[p + 1].offset <= codepoint && codepoint <= utf_to(inst) {
                            p += 2;
                            continue;
                        }
                    }
                }
            }
            ITEST_ANY => {
                if s < e {
                    p += 2;
                } else {
                    p = advance_pc(p, getoffset(code, p));
                }
                continue;
            }
            ICHAR => {
                if s < e && subject[s] == inst.aux1 {
                    p += 1;
                    s += 1;
                    continue;
                }
            }
            ITEST_CHAR => {
                if s < e && subject[s] == inst.aux1 {
                    p += 2;
                } else {
                    p = advance_pc(p, getoffset(code, p));
                }
                continue;
            }
            ISET => {
                if s < e {
                    let c = LpegUint::from(subject[s]);
                    if charinset(inst, &code[p + 1..], c) != 0 {
                        p += 1 + (inst.set_size as usize);
                        s += 1;
                        continue;
                    }
                }
            }
            ITEST_SET => {
                if s < e {
                    let c = LpegUint::from(subject[s]);
                    if charinset(inst, &code[p + 2..], c) != 0 {
                        p += 2 + (inst.set_size as usize);
                        continue;
                    }
                }
                p = advance_pc(p, getoffset(code, p));
                continue;
            }
            IBEHIND => {
                let n = inst.aux1 as usize;
                if n <= s {
                    s -= n;
                    p += 1;
                    continue;
                }
            }
            ISPAN => {
                while s < e {
                    let c = LpegUint::from(subject[s]);
                    if charinset(inst, &code[p + 1..], c) == 0 {
                        break;
                    }
                    s += 1;
                }
                p += 1 + (inst.set_size as usize);
                continue;
            }
            IJMP => {
                p = advance_pc(p, getoffset(code, p));
                continue;
            }
            ICHOICE => {
                if sp == stack.len() {
                    stack = doublestack(l, stack, sp, ptop);
                }
                stack[sp] = Stack {
                    p: advance_pc(p, getoffset(code, p)),
                    s: Some(s),
                    caplevel: captop,
                };
                sp += 1;
                p += 2;
                continue;
            }
            ICALL => {
                if sp == stack.len() {
                    stack = doublestack(l, stack, sp, ptop);
                }
                stack[sp] = Stack {
                    s: None,
                    p: p + 2,
                    caplevel: 0,
                };
                sp += 1;
                p = advance_pc(p, getoffset(code, p));
                continue;
            }
            ICOMMIT => {
                debug_assert!(sp > 0 && stack[sp - 1].s.is_some());
                sp -= 1;
                p = advance_pc(p, getoffset(code, p));
                continue;
            }
            IPARTIAL_COMMIT => {
                debug_assert!(sp > 0 && stack[sp - 1].s.is_some());
                stack[sp - 1].s = Some(s);
                stack[sp - 1].caplevel = captop;
                p = advance_pc(p, getoffset(code, p));
                continue;
            }
            IBACK_COMMIT => {
                debug_assert!(sp > 0 && stack[sp - 1].s.is_some());
                sp -= 1;
                s = stack[sp].s.unwrap();
                if ndyncap > 0 {
                    ndyncap -= removedyncap(l, capture, stack[sp].caplevel, captop);
                }
                captop = stack[sp].caplevel;
                p = advance_pc(p, getoffset(code, p));
                continue;
            }
            IFAIL_TWICE => {
                debug_assert!(sp > 0);
                sp -= 1;
            }
            IFAIL => {}
            ICLOSE_RUN_TIME => {
                let mut rem: i32 = 0;
                let mut fr = gettop(l) + 1;
                let n_removed = runtimecap(l, capture, captop, subject, s, ptop, &mut rem);
                captop -= n_removed as usize;
                ndyncap -= rem;
                fr -= rem;
                if let Some(res) = resdyncaptures(l, fr, s, e) {
                    s = res;
                    let n = gettop(l) - fr + 1;
                    ndyncap += n;
                    if n == 0 {
                        captop -= 1;
                    } else {
                        if fr + n >= i32::from(i16::MAX) {
                            error_msg(l, c"too many results in match-time capture");
                        }
                        let nu = n as usize;
                        capture = growcap(l, capture, captop, nu + 1, ptop);
                        adddyncaptures(s as LpegIndex, capture, captop, nu, fr);
                        captop += nu + 1;
                    }
                    p += 1;
                    continue;
                }
            }
            ICLOSE_CAPTURE => {
                let curr_idx = s as LpegIndex;
                debug_assert!(captop > 0);
                if let Some(open_idx) = findopen_full(capture, captop, curr_idx) {
                    capture[open_idx].siz = (curr_idx - capture[open_idx].index + 1) as LpegByte;
                    p += 1;
                    continue;
                } else {
                    capture[captop].siz = 1;
                    capture[captop].index = curr_idx;
                    capture[captop].idx = inst.key as u16;
                    capture[captop].kind = getkind(inst) as LpegByte;
                    captop += 1;
                    capture = growcap(l, capture, captop, 0, ptop);
                    p += 1;
                    continue;
                }
            }
            IOPEN_CAPTURE => {
                capture[captop].siz = 0;
                capture[captop].index = s as LpegIndex;
                capture[captop].idx = inst.key as u16;
                capture[captop].kind = getkind(inst) as LpegByte;
                captop += 1;
                capture = growcap(l, capture, captop, 0, ptop);
                p += 1;
                continue;
            }
            IFULL_CAPTURE => {
                let off = getoff(inst);
                capture[captop].siz = (off + 1) as LpegByte;
                capture[captop].index = (s as i32 - off) as LpegIndex;
                capture[captop].idx = inst.key as u16;
                capture[captop].kind = getkind(inst) as LpegByte;
                captop += 1;
                capture = growcap(l, capture, captop, 0, ptop);
                p += 1;
                continue;
            }
            _ => {
                debug_assert!(false);
                return None;
            }
        }

        loop {
            debug_assert!(sp > 0);
            sp -= 1;
            if let Some(saved_s) = stack[sp].s {
                s = saved_s;
                break;
            }
        }
        if ndyncap > 0 {
            ndyncap -= removedyncap(l, capture, stack[sp].caplevel, captop);
        }
        captop = stack[sp].caplevel;
        p = stack[sp].p;
    }
}
