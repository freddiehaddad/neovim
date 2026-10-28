//! Capture evaluation (`src/lpeg/lpcap.c`).

use std::ffi::c_char;
use std::ptr;

use crate::lpeg::{
    CACC, CARG, CBACKREF, CCLOSE, CCONST, CFOLD, CFUNCTION, CGROUP, CNUM, CPOSITION, CQUERY,
    CRUNTIME, CSIMPLE, CSTRING, CSUBST, CTABLE, CapState, Capture, FIXEDARGS, LpegIndex, MAXLOP,
    SUBJIDX, capinside, caplistidx, captype, isclosecap, isopencap, ktableidx, lua_Integer,
    lua_State,
};
use crate::lua::converter::{
    LUA_MULTRET, LUAL_BUFFERSIZE, lua_call, lua_equal, lua_gettable, lua_gettop, lua_insert,
    lua_isnil, lua_isstring, lua_newtable, lua_pop, lua_pushinteger, lua_pushlstring,
    lua_pushvalue, lua_rawgeti, lua_rawseti, lua_remove, lua_replace, lua_settable, lua_tolstring,
    lua_tostring, lua_touserdata, luaL_Buffer, luaL_addlstring, luaL_addvalue, luaL_buffinit,
    luaL_checkstack, luaL_error, luaL_prepbuffer, luaL_pushresult, luaL_typename,
};

const MAXRECLEVEL: i32 = 200;
const MAXSTRCAPS: usize = 10;

#[inline]
#[allow(non_snake_case)]
fn luaL_addchar(b: &mut luaL_Buffer, c: c_char) {
    unsafe {
        let buf_start = (&raw mut b.buffer).cast::<c_char>();
        if b.p >= buf_start.wrapping_add(LUAL_BUFFERSIZE) {
            luaL_prepbuffer(b);
        }
        *b.p = c;
        b.p = b.p.wrapping_add(1);
    }
}

#[inline]
fn getfromktable(cs: &mut CapState, v: i32) {
    unsafe {
        lua_rawgeti(cs.lstate, ktableidx(cs.ptop), v);
    }
}

#[inline]
fn pushluaval(cs: &mut CapState) {
    let idx = unsafe { i32::from((*cs.cap).idx) };
    getfromktable(cs, idx);
}

#[inline]
fn skipclose(cs: &mut CapState, head: &Capture) {
    if isopencap(head) {
        debug_assert!(isclosecap(cs.cap));
        cs.cap = cs.cap.wrapping_add(1);
    }
}

#[inline]
fn capsize(cap: &Capture, close: *const Capture) -> LpegIndex {
    unsafe {
        if isopencap(cap) {
            debug_assert!(isclosecap(close));
            (*close).index - cap.index
        } else {
            LpegIndex::from(cap.siz) - 1
        }
    }
}

#[inline]
fn closesize(cs: &CapState, head: &Capture) -> LpegIndex {
    capsize(head, cs.cap)
}

fn updatecache(cs: &mut CapState, v: i32) -> i32 {
    let idx = cs.ptop + 1;
    if v != cs.valuecached {
        getfromktable(cs, v);
        unsafe {
            lua_replace(cs.lstate, idx);
        }
        cs.valuecached = v;
    }
    idx
}

fn findopen(mut cap: *mut Capture) -> *mut Capture {
    let mut n: i32 = 0;
    unsafe {
        loop {
            cap = cap.wrapping_sub(1);
            if isclosecap(cap) {
                n += 1;
            } else if isopencap(&*cap) {
                if n == 0 {
                    return cap;
                }
                n -= 1;
            }
        }
    }
}

fn nextcap(cs: &mut CapState) {
    unsafe {
        let mut cap = cs.cap;
        if isopencap(&*cap) {
            let mut n: i32 = 0;
            loop {
                cap = cap.wrapping_add(1);
                if isopencap(&*cap) {
                    n += 1;
                } else if isclosecap(cap) {
                    if n == 0 {
                        break;
                    }
                    n -= 1;
                }
            }
            cs.cap = cap.wrapping_add(1);
        } else {
            let mut next = cap.wrapping_add(1);
            while capinside(&*cap, next) {
                next = next.wrapping_add(1);
            }
            cs.cap = next;
        }
    }
}

fn pushnestedvalues(cs: &mut CapState, addextra: bool) -> i32 {
    unsafe {
        let head = cs.cap;
        cs.cap = cs.cap.wrapping_add(1);
        let mut n: i32 = 0;
        while capinside(&*head, cs.cap) {
            n += pushcapture(cs);
        }
        if addextra || n == 0 {
            lua_pushlstring(
                cs.lstate,
                cs.s.wrapping_add((*head).index as usize),
                closesize(cs, &*head) as usize,
            );
            n += 1;
        }
        skipclose(cs, &*head);
        n
    }
}

fn pushonenestedvalue(cs: &mut CapState) {
    let n = pushnestedvalues(cs, false);
    if n > 1 {
        lua_pop(cs.lstate, n - 1);
    }
}

fn capvisible(cs: &CapState, grp: &Capture, r#ref: *const Capture) -> bool {
    let mut cap = (&raw const *grp);
    let mut i = MAXLOP;
    unsafe {
        while i > 0 {
            i -= 1;
            cap = cap.wrapping_sub(1);
            if cap <= cs.ocap {
                break;
            }
            if isclosecap(cap) || grp.index - (*cap).index >= LpegIndex::from(u8::MAX) {
                return true;
            } else if capinside(&*cap, (&raw const *grp)) {
                return capinside(&*cap, r#ref);
            }
        }
    }
    true
}

fn findback(cs: &mut CapState, r#ref: *mut Capture) -> *mut Capture {
    unsafe {
        let l = cs.lstate;
        let mut cap = r#ref;
        while cap > cs.ocap {
            cap = cap.wrapping_sub(1);
            if isclosecap(cap) {
                cap = findopen(cap);
            } else if capinside(&*cap, r#ref) {
                continue;
            }
            if captype(cap) == CGROUP && capvisible(cs, &*cap, r#ref) {
                getfromktable(cs, i32::from((*cap).idx));
                if lua_equal(l, -2, -1) != 0 {
                    lua_pop(l, 2);
                    return cap;
                } else {
                    lua_pop(l, 1);
                }
            }
        }
        luaL_error(
            l,
            c"back reference '%s' not found".as_ptr(),
            lua_tostring(l, -1),
        );
        ptr::null_mut()
    }
}

fn backrefcap(cs: &mut CapState) -> i32 {
    let curr = cs.cap;
    pushluaval(cs);
    cs.cap = findback(cs, curr);
    let n = pushnestedvalues(cs, false);
    cs.cap = curr.wrapping_add(1);
    n
}

fn tablecap(cs: &mut CapState) -> i32 {
    unsafe {
        let l = cs.lstate;
        let head = cs.cap;
        cs.cap = cs.cap.wrapping_add(1);
        let mut n: i32 = 0;
        lua_newtable(l);
        while capinside(&*head, cs.cap) {
            if captype(cs.cap) == CGROUP && (*cs.cap).idx != 0 {
                pushluaval(cs);
                pushonenestedvalue(cs);
                lua_settable(l, -3);
            } else {
                let k = pushcapture(cs);
                let mut i = k;
                while i > 0 {
                    lua_rawseti(l, -(i + 1), n + i);
                    i -= 1;
                }
                n += k;
            }
        }
        skipclose(cs, &*head);
        1
    }
}

fn querycap(cs: &mut CapState) -> i32 {
    unsafe {
        let idx = i32::from((*cs.cap).idx);
        pushonenestedvalue(cs);
        let cache_idx = updatecache(cs, idx);
        lua_gettable(cs.lstate, cache_idx);
        if !lua_isnil(cs.lstate, -1) {
            1
        } else {
            lua_pop(cs.lstate, 1);
            0
        }
    }
}

fn foldcap(cs: &mut CapState) -> i32 {
    unsafe {
        let l = cs.lstate;
        let head = cs.cap;
        cs.cap = cs.cap.wrapping_add(1);
        let idx = i32::from((*head).idx);
        if isclosecap(cs.cap) {
            return luaL_error(l, c"no initial value for fold capture".as_ptr());
        }
        let mut n = pushcapture(cs);
        if n == 0 {
            return luaL_error(l, c"no initial value for fold capture".as_ptr());
        }
        if n > 1 {
            lua_pop(l, n - 1);
        }
        while capinside(&*head, cs.cap) {
            let cache_idx = updatecache(cs, idx);
            lua_pushvalue(l, cache_idx);
            lua_insert(l, -2);
            n = pushcapture(cs);
            lua_call(l, n + 1, 1);
        }
        skipclose(cs, &*head);
        1
    }
}

fn functioncap(cs: &mut CapState) -> i32 {
    unsafe {
        let top = lua_gettop(cs.lstate);
        pushluaval(cs);
        let n = pushnestedvalues(cs, false);
        lua_call(cs.lstate, n, LUA_MULTRET);
        lua_gettop(cs.lstate) - top
    }
}

fn accumulatorcap(cs: &mut CapState) -> i32 {
    unsafe {
        let l = cs.lstate;
        if lua_gettop(l) < cs.firstcap {
            luaL_error(l, c"no previous value for accumulator capture".as_ptr());
        }
        pushluaval(cs);
        lua_insert(l, -2);
        let n = pushnestedvalues(cs, false);
        lua_call(l, n + 1, 1);
        0
    }
}

fn numcap(cs: &mut CapState) -> i32 {
    unsafe {
        let idx = i32::from((*cs.cap).idx);
        if idx == 0 {
            nextcap(cs);
            0
        } else {
            let n = pushnestedvalues(cs, false);
            if n < idx {
                luaL_error(cs.lstate, c"no capture '%d'".as_ptr(), idx)
            } else {
                lua_pushvalue(cs.lstate, -(n - idx + 1));
                lua_replace(cs.lstate, -(n + 1));
                lua_pop(cs.lstate, n - 1);
                1
            }
        }
    }
}

pub fn finddyncap(mut cap: *mut Capture, last: *mut Capture) -> i32 {
    unsafe {
        while cap < last {
            if (*cap).kind == CRUNTIME {
                return i32::from((*cap).idx);
            }
            cap = cap.wrapping_add(1);
        }
    }
    0
}

pub fn runtimecap(cs: &mut CapState, close: *mut Capture, s: *const c_char, rem: &mut i32) -> i32 {
    unsafe {
        let l = cs.lstate;
        let otop = lua_gettop(l);
        let open = findopen(close);
        debug_assert_eq!(captype(open), CGROUP);
        let id = finddyncap(open, close);
        (*close).kind = CCLOSE;
        (*close).index = s.offset_from(cs.s) as LpegIndex;
        cs.cap = open;
        cs.valuecached = 0;
        luaL_checkstack(l, 4, c"too many runtime captures");
        pushluaval(cs);
        lua_pushvalue(l, SUBJIDX);
        lua_pushinteger(l, (s.offset_from(cs.s) + 1) as lua_Integer);
        let n = pushnestedvalues(cs, false);
        lua_call(l, n + 2, LUA_MULTRET);
        if id > 0 {
            for _ in id..=otop {
                lua_remove(l, id);
            }
            *rem = otop - id + 1;
        } else {
            *rem = 0;
        }
        (close.offset_from(open) - 1) as i32
    }
}

#[derive(Copy, Clone)]
enum StrAux {
    Capture(*mut Capture),
    Span { idx: LpegIndex, siz: LpegIndex },
}

impl StrAux {
    const fn new() -> Self {
        Self::Capture(ptr::null_mut())
    }
}

impl Default for StrAux {
    fn default() -> Self {
        Self::new()
    }
}

fn getstrcaps(cs: &mut CapState, cps: &mut [StrAux; MAXSTRCAPS], mut n: i32) -> i32 {
    unsafe {
        let k = n as usize;
        n += 1;
        let head = cs.cap;
        let head_idx = (*head).index;
        cs.cap = cs.cap.wrapping_add(1);
        while capinside(&*head, cs.cap) {
            if (n as usize) >= MAXSTRCAPS {
                nextcap(cs);
            } else if captype(cs.cap) == CSIMPLE {
                n = getstrcaps(cs, cps, n);
            } else {
                cps[n as usize] = StrAux::Capture(cs.cap);
                nextcap(cs);
                n += 1;
            }
        }
        cps[k] = StrAux::Span {
            idx: head_idx,
            siz: closesize(cs, &*head),
        };
        skipclose(cs, &*head);
        n
    }
}

fn stringcap(b: &mut luaL_Buffer, cs: &mut CapState) {
    unsafe {
        let mut cps: [StrAux; MAXSTRCAPS] = [StrAux::new(); MAXSTRCAPS];
        let mut len: usize = 0;
        let cap_idx = i32::from((*cs.cap).idx);
        let cache_idx = updatecache(cs, cap_idx);
        let fmt = lua_tolstring(cs.lstate, cache_idx, &mut len);
        let n = getstrcaps(cs, &mut cps, 0) - 1;
        let mut i = 0usize;
        while i < len {
            let ch = *fmt.wrapping_add(i);
            if ch != b'%' as c_char {
                luaL_addchar(b, ch);
            } else {
                i += 1;
                let next_ch = *fmt.wrapping_add(i) as u8;
                if !next_ch.is_ascii_digit() {
                    luaL_addchar(b, *fmt.wrapping_add(i));
                } else {
                    let l = i32::from(next_ch - b'0');
                    if l > n {
                        luaL_error(cs.lstate, c"invalid capture index (%d)".as_ptr(), l);
                    } else {
                        match cps[l as usize] {
                            StrAux::Span { idx, siz } => {
                                luaL_addlstring(b, cs.s.wrapping_add(idx as usize), siz as usize);
                            }
                            StrAux::Capture(cp) => {
                                let curr = cs.cap;
                                cs.cap = cp;
                                if addonestring(b, cs, c"capture") == 0 {
                                    luaL_error(
                                        cs.lstate,
                                        c"no values in capture index %d".as_ptr(),
                                        l,
                                    );
                                }
                                cs.cap = curr;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
    }
}

fn substcap(b: &mut luaL_Buffer, cs: &mut CapState) {
    unsafe {
        let mut curr = cs.s.wrapping_add((*cs.cap).index as usize);
        let head = cs.cap;
        cs.cap = cs.cap.wrapping_add(1);
        while capinside(&*head, cs.cap) {
            let cap = cs.cap;
            let caps = cs.s.wrapping_add((*cap).index as usize);
            luaL_addlstring(b, curr, caps.offset_from(curr) as usize);
            if addonestring(b, cs, c"replacement") != 0 {
                curr = caps.wrapping_add(capsize(&*cap, cs.cap.wrapping_sub(1)) as usize);
            } else {
                curr = caps;
            }
        }
        let end =
            cs.s.wrapping_add(((*head).index + closesize(cs, &*head)) as usize);
        luaL_addlstring(b, curr, end.offset_from(curr) as usize);
        skipclose(cs, &*head);
    }
}

fn addonestring(b: &mut luaL_Buffer, cs: &mut CapState, what: &std::ffi::CStr) -> i32 {
    unsafe {
        match captype(cs.cap) {
            CSTRING => {
                stringcap(b, cs);
                1
            }
            CSUBST => {
                substcap(b, cs);
                1
            }
            CACC => luaL_error(
                cs.lstate,
                c"invalid context for an accumulator capture".as_ptr(),
            ),
            _ => {
                let l = cs.lstate;
                let n = pushcapture(cs);
                if n > 0 {
                    if n > 1 {
                        lua_pop(l, n - 1);
                    }
                    if lua_isstring(l, -1) == 0 {
                        return luaL_error(
                            l,
                            c"invalid %s value (a %s)".as_ptr(),
                            what.as_ptr(),
                            luaL_typename(l, -1),
                        );
                    }
                    luaL_addvalue(b);
                }
                n
            }
        }
    }
}

fn pushcapture(cs: &mut CapState) -> i32 {
    unsafe {
        let l = cs.lstate;
        luaL_checkstack(l, 4, c"too many captures");
        let old_rec = cs.reclevel;
        cs.reclevel += 1;
        if old_rec > MAXRECLEVEL {
            return luaL_error(l, c"subcapture nesting too deep".as_ptr());
        }
        let res = match captype(cs.cap) {
            CPOSITION => {
                lua_pushinteger(l, ((*cs.cap).index + 1) as lua_Integer);
                cs.cap = cs.cap.wrapping_add(1);
                1
            }
            CCONST => {
                pushluaval(cs);
                cs.cap = cs.cap.wrapping_add(1);
                1
            }
            CARG => {
                let arg = i32::from((*cs.cap).idx);
                cs.cap = cs.cap.wrapping_add(1);
                if arg + FIXEDARGS > cs.ptop {
                    return luaL_error(l, c"reference to absent extra argument #%d".as_ptr(), arg);
                }
                lua_pushvalue(l, arg + FIXEDARGS);
                1
            }
            CSIMPLE => {
                let k = pushnestedvalues(cs, true);
                lua_insert(l, -k);
                k
            }
            CRUNTIME => {
                let idx = i32::from((*cs.cap).idx);
                cs.cap = cs.cap.wrapping_add(1);
                lua_pushvalue(l, idx);
                1
            }
            CSTRING => {
                let mut b = luaL_Buffer::new();
                luaL_buffinit(l, &mut b);
                stringcap(&mut b, cs);
                luaL_pushresult(&mut b);
                1
            }
            CSUBST => {
                let mut b = luaL_Buffer::new();
                luaL_buffinit(l, &mut b);
                substcap(&mut b, cs);
                luaL_pushresult(&mut b);
                1
            }
            CGROUP => {
                if (*cs.cap).idx == 0 {
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
}

pub fn getcaptures(l: *mut lua_State, s: *const c_char, r: *const c_char, ptop: i32) -> i32 {
    unsafe {
        let capture = lua_touserdata(l, caplistidx(ptop)).cast::<Capture>();
        let mut n: i32 = 0;
        if !isclosecap(capture) {
            let mut cs = CapState {
                ocap: capture,
                cap: capture,
                lstate: l,
                ptop,
                firstcap: lua_gettop(l) + 1,
                s,
                valuecached: 0,
                reclevel: 0,
            };
            loop {
                n += pushcapture(&mut cs);
                if isclosecap(cs.cap) {
                    break;
                }
            }
            debug_assert_eq!(lua_gettop(l) - cs.firstcap, n - 1);
        }
        if n == 0 {
            lua_pushinteger(l, (r.offset_from(s) + 1) as lua_Integer);
            n = 1;
        }
        n
    }
}
