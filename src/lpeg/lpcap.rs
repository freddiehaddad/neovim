//! Capture evaluation (`src/lpeg/lpcap.c`).
#![deny(unsafe_code)]

use std::ffi::CStr;

use crate::lpeg::{
    CACC, CARG, CBACKREF, CCLOSE, CCONST, CFOLD, CFUNCTION, CGROUP, CNUM, CPOSITION, CQUERY,
    CRUNTIME, CSIMPLE, CSTRING, CSUBST, CTABLE, CapState, Capture, FIXEDARGS, LpegIndex, MAXLOP,
    SUBJIDX, capinside, captype, isclosecap, isopencap, ktableidx, lua_Integer, lua_State,
};

const MAXRECLEVEL: i32 = 200;
const MAXSTRCAPS: usize = 10;

#[allow(unsafe_code)]
mod ffi {
    use std::ffi::CStr;
    use std::ptr;

    use crate::lpeg::{ktableidx, lua_Integer, lua_State};
    use crate::lua::converter::{
        LUA_MULTRET, LUAL_BUFFERSIZE, lua_call, lua_equal, lua_gettable, lua_gettop, lua_insert,
        lua_isnil, lua_isstring, lua_newtable, lua_pop, lua_pushinteger, lua_pushlstring,
        lua_pushvalue, lua_rawgeti, lua_rawseti, lua_remove, lua_replace, lua_settable,
        lua_tolstring, lua_tostring, luaL_addlstring, luaL_addvalue, luaL_buffinit,
        luaL_checkstack, luaL_error, luaL_prepbuffer, luaL_pushresult, luaL_typename,
    };
    use crate::strings::{RawCChar, RawCCharPtrMut};

    pub use crate::lua::converter::luaL_Buffer as LuaBuffer;

    #[inline]
    pub fn buf_init(l: *mut lua_State, b: &mut LuaBuffer) {
        unsafe {
            luaL_buffinit(l, b);
        }
    }

    #[inline]
    pub fn buf_addchar(b: &mut LuaBuffer, c: u8) {
        unsafe {
            let buf_start: RawCCharPtrMut = (&raw mut b.buffer).cast();
            if b.p >= buf_start.wrapping_add(LUAL_BUFFERSIZE) {
                luaL_prepbuffer(b);
            }
            *b.p = c as RawCChar;
            b.p = b.p.wrapping_add(1);
        }
    }

    #[inline]
    pub fn buf_addbytes(b: &mut LuaBuffer, bytes: &[u8]) {
        unsafe {
            luaL_addlstring(b, bytes.as_ptr().cast(), bytes.len());
        }
    }

    #[inline]
    pub fn buf_addvalue(b: &mut LuaBuffer) {
        unsafe {
            luaL_addvalue(b);
        }
    }

    #[inline]
    pub fn buf_pushresult(b: &mut LuaBuffer) {
        unsafe {
            luaL_pushresult(b);
        }
    }

    #[inline]
    pub fn rawgeti_ktable(l: *mut lua_State, ptop: i32, v: i32) {
        unsafe {
            lua_rawgeti(l, ktableidx(ptop), v);
        }
    }

    #[inline]
    pub fn replace(l: *mut lua_State, idx: i32) {
        unsafe {
            lua_replace(l, idx);
        }
    }

    #[inline]
    pub fn push_bytes(l: *mut lua_State, bytes: &[u8]) {
        unsafe {
            lua_pushlstring(l, bytes.as_ptr().cast(), bytes.len());
        }
    }

    #[inline]
    pub fn pop(l: *mut lua_State, n: i32) {
        lua_pop(l, n);
    }

    #[inline]
    pub fn equal(l: *mut lua_State, idx1: i32, idx2: i32) -> bool {
        unsafe { lua_equal(l, idx1, idx2) != 0 }
    }

    #[inline]
    pub fn error_backref(l: *mut lua_State) -> ! {
        unsafe {
            luaL_error(
                l,
                c"back reference '%s' not found".as_ptr(),
                lua_tostring(l, -1).map_or(ptr::null(), CStr::as_ptr),
            );
            unreachable!()
        }
    }

    #[inline]
    pub fn error_msg(l: *mut lua_State, msg: &CStr) -> i32 {
        unsafe { luaL_error(l, msg.as_ptr()) }
    }

    #[inline]
    pub fn error_int(l: *mut lua_State, fmt: &CStr, val: i32) -> i32 {
        unsafe { luaL_error(l, fmt.as_ptr(), val) }
    }

    #[inline]
    pub fn error_invalid_val(l: *mut lua_State, what: &CStr) -> i32 {
        unsafe {
            luaL_error(
                l,
                c"invalid %s value (a %s)".as_ptr(),
                what.as_ptr(),
                luaL_typename(l, -1).as_ptr(),
            )
        }
    }

    #[inline]
    pub fn newtable(l: *mut lua_State) {
        lua_newtable(l);
    }

    #[inline]
    pub fn settable(l: *mut lua_State, idx: i32) {
        unsafe {
            lua_settable(l, idx);
        }
    }

    #[inline]
    pub fn rawseti(l: *mut lua_State, idx: i32, n: i32) {
        unsafe {
            lua_rawseti(l, idx, n);
        }
    }

    #[inline]
    pub fn gettable(l: *mut lua_State, idx: i32) {
        unsafe {
            lua_gettable(l, idx);
        }
    }

    #[inline]
    pub fn isnil(l: *mut lua_State, idx: i32) -> bool {
        lua_isnil(l, idx)
    }

    #[inline]
    pub fn isstring(l: *mut lua_State, idx: i32) -> bool {
        unsafe { lua_isstring(l, idx) != 0 }
    }

    #[inline]
    pub fn pushvalue(l: *mut lua_State, idx: i32) {
        unsafe {
            lua_pushvalue(l, idx);
        }
    }

    #[inline]
    pub fn insert(l: *mut lua_State, idx: i32) {
        unsafe {
            lua_insert(l, idx);
        }
    }

    #[inline]
    pub fn call(l: *mut lua_State, nargs: i32, nresults: i32) {
        unsafe {
            lua_call(l, nargs, nresults);
        }
    }

    #[inline]
    pub fn call_multret(l: *mut lua_State, nargs: i32) {
        unsafe {
            lua_call(l, nargs, LUA_MULTRET);
        }
    }

    #[inline]
    pub fn gettop(l: *mut lua_State) -> i32 {
        unsafe { lua_gettop(l) }
    }

    #[inline]
    pub fn remove(l: *mut lua_State, idx: i32) {
        unsafe {
            lua_remove(l, idx);
        }
    }

    #[inline]
    pub fn pushinteger(l: *mut lua_State, n: lua_Integer) {
        unsafe {
            lua_pushinteger(l, n);
        }
    }

    #[inline]
    pub fn checkstack(l: *mut lua_State, sz: i32, msg: &CStr) {
        luaL_checkstack(l, sz, msg);
    }

    #[inline]
    pub fn tolstring_vec(l: *mut lua_State, idx: i32) -> Vec<u8> {
        unsafe {
            let mut len: usize = 0;
            let p = lua_tolstring(l, idx, &mut len);
            if p.is_null() || len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(p.cast::<u8>(), len).to_vec()
            }
        }
    }
}

use self::ffi::*;

#[inline]
fn getfromktable(cs: &mut CapState<'_>, v: i32) {
    let _ = ktableidx;
    rawgeti_ktable(cs.lstate, cs.ptop, v);
}

#[inline]
fn pushluaval(cs: &mut CapState<'_>) {
    let idx = i32::from(cs.caps[cs.cap_idx].idx);
    getfromktable(cs, idx);
}

#[inline]
fn skipclose(cs: &mut CapState<'_>, head: &Capture) {
    if isopencap(head) {
        debug_assert!(isclosecap(&cs.caps[cs.cap_idx]));
        cs.cap_idx += 1;
    }
}

#[inline]
fn capsize(cap: &Capture, close: &Capture) -> LpegIndex {
    if isopencap(cap) {
        debug_assert!(isclosecap(close));
        close.index - cap.index
    } else {
        LpegIndex::from(cap.siz) - 1
    }
}

#[inline]
fn closesize(cs: &CapState<'_>, head: &Capture) -> LpegIndex {
    capsize(head, &cs.caps[cs.cap_idx])
}

fn updatecache(cs: &mut CapState<'_>, v: i32) -> i32 {
    let idx = cs.ptop + 1;
    if v != cs.valuecached {
        getfromktable(cs, v);
        replace(cs.lstate, idx);
        cs.valuecached = v;
    }
    idx
}

fn findopen(caps: &[Capture], mut idx: usize) -> usize {
    let mut n: i32 = 0;
    loop {
        idx -= 1;
        if isclosecap(&caps[idx]) {
            n += 1;
        } else if isopencap(&caps[idx]) {
            if n == 0 {
                return idx;
            }
            n -= 1;
        }
    }
}

fn nextcap(cs: &mut CapState<'_>) {
    let mut idx = cs.cap_idx;
    if isopencap(&cs.caps[idx]) {
        let mut n: i32 = 0;
        loop {
            idx += 1;
            if isopencap(&cs.caps[idx]) {
                n += 1;
            } else if isclosecap(&cs.caps[idx]) {
                if n == 0 {
                    break;
                }
                n -= 1;
            }
        }
        cs.cap_idx = idx + 1;
    } else {
        let head = cs.caps[idx];
        let mut next = idx + 1;
        while capinside(&head, &cs.caps[next]) {
            next += 1;
        }
        cs.cap_idx = next;
    }
}

fn pushnestedvalues(cs: &mut CapState<'_>, addextra: bool) -> i32 {
    let head = cs.caps[cs.cap_idx];
    cs.cap_idx += 1;
    let mut n: i32 = 0;
    while capinside(&head, &cs.caps[cs.cap_idx]) {
        n += pushcapture(cs);
    }
    if addextra || n == 0 {
        let start = head.index as usize;
        let len = closesize(cs, &head) as usize;
        push_bytes(cs.lstate, &cs.s[start..start + len]);
        n += 1;
    }
    skipclose(cs, &head);
    n
}

fn pushonenestedvalue(cs: &mut CapState<'_>) {
    let n = pushnestedvalues(cs, false);
    if n > 1 {
        pop(cs.lstate, n - 1);
    }
}

fn capvisible(cs: &CapState<'_>, grp_idx: usize, ref_idx: usize) -> bool {
    let grp = &cs.caps[grp_idx];
    let r#ref = &cs.caps[ref_idx];
    let mut idx = grp_idx;
    let mut i = MAXLOP;
    while i > 0 && idx > 0 {
        i -= 1;
        idx -= 1;
        let cap = &cs.caps[idx];
        if isclosecap(cap) || grp.index - cap.index >= LpegIndex::from(u8::MAX) {
            return true;
        } else if capinside(cap, grp) {
            return capinside(cap, r#ref);
        }
    }
    true
}

fn findback(cs: &mut CapState<'_>, ref_idx: usize) -> usize {
    let l = cs.lstate;
    let r#ref = cs.caps[ref_idx];
    let mut idx = ref_idx;
    while idx > 0 {
        idx -= 1;
        if isclosecap(&cs.caps[idx]) {
            idx = findopen(cs.caps, idx);
        } else if capinside(&cs.caps[idx], &r#ref) {
            continue;
        }
        if captype(&cs.caps[idx]) == CGROUP && capvisible(cs, idx, ref_idx) {
            let kidx = i32::from(cs.caps[idx].idx);
            getfromktable(cs, kidx);
            if equal(l, -2, -1) {
                pop(l, 2);
                return idx;
            } else {
                pop(l, 1);
            }
        }
    }
    error_backref(l)
}

fn backrefcap(cs: &mut CapState<'_>) -> i32 {
    let curr = cs.cap_idx;
    pushluaval(cs);
    cs.cap_idx = findback(cs, curr);
    let n = pushnestedvalues(cs, false);
    cs.cap_idx = curr + 1;
    n
}

fn tablecap(cs: &mut CapState<'_>) -> i32 {
    let l = cs.lstate;
    let head = cs.caps[cs.cap_idx];
    cs.cap_idx += 1;
    let mut n: i32 = 0;
    newtable(l);
    while capinside(&head, &cs.caps[cs.cap_idx]) {
        if captype(&cs.caps[cs.cap_idx]) == CGROUP && cs.caps[cs.cap_idx].idx != 0 {
            pushluaval(cs);
            pushonenestedvalue(cs);
            settable(l, -3);
        } else {
            let k = pushcapture(cs);
            let mut i = k;
            while i > 0 {
                rawseti(l, -(i + 1), n + i);
                i -= 1;
            }
            n += k;
        }
    }
    skipclose(cs, &head);
    1
}

fn querycap(cs: &mut CapState<'_>) -> i32 {
    let idx = i32::from(cs.caps[cs.cap_idx].idx);
    pushonenestedvalue(cs);
    let cache_idx = updatecache(cs, idx);
    gettable(cs.lstate, cache_idx);
    if !isnil(cs.lstate, -1) {
        1
    } else {
        pop(cs.lstate, 1);
        0
    }
}

fn foldcap(cs: &mut CapState<'_>) -> i32 {
    let l = cs.lstate;
    let head = cs.caps[cs.cap_idx];
    cs.cap_idx += 1;
    let idx = i32::from(head.idx);
    if isclosecap(&cs.caps[cs.cap_idx]) {
        return error_msg(l, c"no initial value for fold capture");
    }
    let mut n = pushcapture(cs);
    if n == 0 {
        return error_msg(l, c"no initial value for fold capture");
    }
    if n > 1 {
        pop(l, n - 1);
    }
    while capinside(&head, &cs.caps[cs.cap_idx]) {
        let cache_idx = updatecache(cs, idx);
        pushvalue(l, cache_idx);
        insert(l, -2);
        n = pushcapture(cs);
        call(l, n + 1, 1);
    }
    skipclose(cs, &head);
    1
}

fn functioncap(cs: &mut CapState<'_>) -> i32 {
    let top = gettop(cs.lstate);
    pushluaval(cs);
    let n = pushnestedvalues(cs, false);
    call_multret(cs.lstate, n);
    gettop(cs.lstate) - top
}

fn accumulatorcap(cs: &mut CapState<'_>) -> i32 {
    let l = cs.lstate;
    if gettop(l) < cs.firstcap {
        error_msg(l, c"no previous value for accumulator capture");
    }
    pushluaval(cs);
    insert(l, -2);
    let n = pushnestedvalues(cs, false);
    call(l, n + 1, 1);
    0
}

fn numcap(cs: &mut CapState<'_>) -> i32 {
    let idx = i32::from(cs.caps[cs.cap_idx].idx);
    if idx == 0 {
        nextcap(cs);
        0
    } else {
        let n = pushnestedvalues(cs, false);
        if n < idx {
            error_int(cs.lstate, c"no capture '%d'", idx)
        } else {
            pushvalue(cs.lstate, -(n - idx + 1));
            replace(cs.lstate, -(n + 1));
            pop(cs.lstate, n - 1);
            1
        }
    }
}

pub fn finddyncap(caps: &[Capture], from: usize, to: usize) -> i32 {
    for cap in &caps[from..to] {
        if cap.kind == CRUNTIME {
            return i32::from(cap.idx);
        }
    }
    0
}

pub fn runtimecap(
    l: *mut lua_State,
    caps: &mut [Capture],
    close_idx: usize,
    s: &[u8],
    s_pos: usize,
    ptop: i32,
    rem: &mut i32,
) -> i32 {
    let otop = gettop(l);
    let open_idx = findopen(caps, close_idx);
    debug_assert_eq!(captype(&caps[open_idx]), CGROUP);
    let id = finddyncap(caps, open_idx, close_idx);
    caps[close_idx].kind = CCLOSE;
    caps[close_idx].index = s_pos as LpegIndex;
    let mut cs = CapState {
        caps,
        cap_idx: open_idx,
        lstate: l,
        ptop,
        firstcap: 0,
        s,
        valuecached: 0,
        reclevel: 0,
    };
    checkstack(l, 4, c"too many runtime captures");
    pushluaval(&mut cs);
    pushvalue(l, SUBJIDX);
    pushinteger(l, (s_pos + 1) as lua_Integer);
    let n = pushnestedvalues(&mut cs, false);
    call_multret(l, n + 2);
    if id > 0 {
        for _ in id..=otop {
            remove(l, id);
        }
        *rem = otop - id + 1;
    } else {
        *rem = 0;
    }
    (close_idx - open_idx - 1) as i32
}

#[derive(Copy, Clone, Default)]
enum StrAux {
    #[default]
    None,
    Capture(usize),
    Span {
        idx: LpegIndex,
        siz: LpegIndex,
    },
}

fn getstrcaps(cs: &mut CapState<'_>, cps: &mut [StrAux; MAXSTRCAPS], mut n: i32) -> i32 {
    let k = n as usize;
    n += 1;
    let head = cs.caps[cs.cap_idx];
    let head_idx = head.index;
    cs.cap_idx += 1;
    while capinside(&head, &cs.caps[cs.cap_idx]) {
        if (n as usize) >= MAXSTRCAPS {
            nextcap(cs);
        } else if captype(&cs.caps[cs.cap_idx]) == CSIMPLE {
            n = getstrcaps(cs, cps, n);
        } else {
            cps[n as usize] = StrAux::Capture(cs.cap_idx);
            nextcap(cs);
            n += 1;
        }
    }
    cps[k] = StrAux::Span {
        idx: head_idx,
        siz: closesize(cs, &head),
    };
    skipclose(cs, &head);
    n
}

fn stringcap(b: &mut LuaBuffer, cs: &mut CapState<'_>) {
    let mut cps: [StrAux; MAXSTRCAPS] = [StrAux::None; MAXSTRCAPS];
    let cap_idx = i32::from(cs.caps[cs.cap_idx].idx);
    let cache_idx = updatecache(cs, cap_idx);
    let fmt = tolstring_vec(cs.lstate, cache_idx);
    let n = getstrcaps(cs, &mut cps, 0) - 1;
    let mut i = 0usize;
    while i < fmt.len() {
        let ch = fmt[i];
        if ch != b'%' {
            buf_addchar(b, ch);
        } else {
            i += 1;
            let next_ch = fmt.get(i).copied().unwrap_or(0);
            if !next_ch.is_ascii_digit() {
                buf_addchar(b, next_ch);
            } else {
                let l = i32::from(next_ch - b'0');
                if l > n {
                    error_int(cs.lstate, c"invalid capture index (%d)", l);
                } else {
                    match cps[l as usize] {
                        StrAux::Span { idx, siz } => {
                            let start = idx as usize;
                            let end = start + (siz as usize);
                            buf_addbytes(b, &cs.s[start..end]);
                        }
                        StrAux::Capture(cp) => {
                            let curr = cs.cap_idx;
                            cs.cap_idx = cp;
                            if addonestring(b, cs, c"capture") == 0 {
                                error_int(cs.lstate, c"no values in capture index %d", l);
                            }
                            cs.cap_idx = curr;
                        }
                        StrAux::None => {}
                    }
                }
            }
        }
        i += 1;
    }
}

fn substcap(b: &mut LuaBuffer, cs: &mut CapState<'_>) {
    let head = cs.caps[cs.cap_idx];
    let mut curr = head.index as usize;
    cs.cap_idx += 1;
    while capinside(&head, &cs.caps[cs.cap_idx]) {
        let cap = cs.caps[cs.cap_idx];
        let cap_start = cap.index as usize;
        buf_addbytes(b, &cs.s[curr..cap_start]);
        if addonestring(b, cs, c"replacement") != 0 {
            curr = cap_start + (capsize(&cap, &cs.caps[cs.cap_idx - 1]) as usize);
        } else {
            curr = cap_start;
        }
    }
    let end = (head.index + closesize(cs, &head)) as usize;
    buf_addbytes(b, &cs.s[curr..end]);
    skipclose(cs, &head);
}

fn addonestring(b: &mut LuaBuffer, cs: &mut CapState<'_>, what: &CStr) -> i32 {
    match captype(&cs.caps[cs.cap_idx]) {
        CSTRING => {
            stringcap(b, cs);
            1
        }
        CSUBST => {
            substcap(b, cs);
            1
        }
        CACC => error_msg(cs.lstate, c"invalid context for an accumulator capture"),
        _ => {
            let l = cs.lstate;
            let n = pushcapture(cs);
            if n > 0 {
                if n > 1 {
                    pop(l, n - 1);
                }
                if !isstring(l, -1) {
                    return error_invalid_val(l, what);
                }
                buf_addvalue(b);
            }
            n
        }
    }
}

fn pushcapture(cs: &mut CapState<'_>) -> i32 {
    let l = cs.lstate;
    checkstack(l, 4, c"too many captures");
    let old_rec = cs.reclevel;
    cs.reclevel += 1;
    if old_rec > MAXRECLEVEL {
        return error_msg(l, c"subcapture nesting too deep");
    }
    let res = match captype(&cs.caps[cs.cap_idx]) {
        CPOSITION => {
            pushinteger(l, (cs.caps[cs.cap_idx].index + 1) as lua_Integer);
            cs.cap_idx += 1;
            1
        }
        CCONST => {
            pushluaval(cs);
            cs.cap_idx += 1;
            1
        }
        CARG => {
            let arg = i32::from(cs.caps[cs.cap_idx].idx);
            cs.cap_idx += 1;
            if arg + FIXEDARGS > cs.ptop {
                return error_int(l, c"reference to absent extra argument #%d", arg);
            }
            pushvalue(l, arg + FIXEDARGS);
            1
        }
        CSIMPLE => {
            let k = pushnestedvalues(cs, true);
            insert(l, -k);
            k
        }
        CRUNTIME => {
            let idx = i32::from(cs.caps[cs.cap_idx].idx);
            cs.cap_idx += 1;
            pushvalue(l, idx);
            1
        }
        CSTRING => {
            let mut b = LuaBuffer::new();
            buf_init(l, &mut b);
            stringcap(&mut b, cs);
            buf_pushresult(&mut b);
            1
        }
        CSUBST => {
            let mut b = LuaBuffer::new();
            buf_init(l, &mut b);
            substcap(&mut b, cs);
            buf_pushresult(&mut b);
            1
        }
        CGROUP => {
            if cs.caps[cs.cap_idx].idx == 0 {
                pushnestedvalues(cs, false)
            } else {
                nextcap(cs);
                0
            }
        }
        CBACKREF => backrefcap(cs),
        CTABLE => tablecap(cs),
        CFUNCTION => functioncap(cs),
        CACC => accumulatorcap(cs),
        CNUM => numcap(cs),
        CQUERY => querycap(cs),
        CFOLD => foldcap(cs),
        _ => {
            debug_assert!(false);
            0
        }
    };
    cs.reclevel -= 1;
    res
}

pub fn getcaptures(
    l: *mut lua_State,
    s: &[u8],
    r_pos: usize,
    capture: &[Capture],
    ptop: i32,
) -> i32 {
    let mut n: i32 = 0;
    if !isclosecap(&capture[0]) {
        let mut cs = CapState {
            caps: capture,
            cap_idx: 0,
            lstate: l,
            ptop,
            firstcap: gettop(l) + 1,
            s,
            valuecached: 0,
            reclevel: 0,
        };
        loop {
            n += pushcapture(&mut cs);
            if isclosecap(&cs.caps[cs.cap_idx]) {
                break;
            }
        }
        debug_assert_eq!(gettop(l) - cs.firstcap, n - 1);
    }
    if n == 0 {
        pushinteger(l, (r_pos + 1) as lua_Integer);
        n = 1;
    }
    n
}
