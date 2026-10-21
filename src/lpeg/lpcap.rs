//! Capture evaluation (`src/lpeg/lpcap.c`).

use std::ffi::{c_char, c_int};
use std::mem::zeroed;
use std::ptr;

use crate::lpeg::{
    Cacc, CapState, Capture, Carg, Cbackref, Cclose, Cconst, Cfold, Cfunction, Cgroup, Cnum,
    Cposition, Cquery, Cruntime, Csimple, Cstring, Csubst, Ctable, FIXEDARGS, Index_t, MAXLOP,
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

const MAXRECLEVEL: c_int = 200;
const MAXSTRCAPS: usize = 10;

#[inline]
unsafe fn luaL_addchar(b: *mut luaL_Buffer, c: c_char) {
    unsafe {
        let buf_start = (&raw mut (*b).buffer).cast::<c_char>();
        if (*b).p >= buf_start.add(LUAL_BUFFERSIZE) {
            luaL_prepbuffer(b);
        }
        *(*b).p = c;
        (*b).p = (*b).p.add(1);
    }
}

#[inline]
unsafe fn getfromktable(cs: *mut CapState, v: c_int) {
    unsafe {
        lua_rawgeti((*cs).L, ktableidx((*cs).ptop), v);
    }
}

#[inline]
unsafe fn pushluaval(cs: *mut CapState) {
    unsafe {
        getfromktable(cs, c_int::from((*(*cs).cap).idx));
    }
}

#[inline]
unsafe fn skipclose(cs: *mut CapState, head: *const Capture) {
    unsafe {
        if isopencap(head) {
            debug_assert!(isclosecap((*cs).cap));
            (*cs).cap = (*cs).cap.add(1);
        }
    }
}

#[inline]
unsafe fn capsize(cap: *const Capture, close: *const Capture) -> Index_t {
    unsafe {
        if isopencap(cap) {
            debug_assert!(isclosecap(close));
            (*close).index - (*cap).index
        } else {
            Index_t::from((*cap).siz) - 1
        }
    }
}

#[inline]
unsafe fn closesize(cs: *const CapState, head: *const Capture) -> Index_t {
    unsafe { capsize(head, (*cs).cap) }
}

unsafe fn updatecache(cs: *mut CapState, v: c_int) -> c_int {
    unsafe {
        let idx = (*cs).ptop + 1;
        if v != (*cs).valuecached {
            getfromktable(cs, v);
            lua_replace((*cs).L, idx);
            (*cs).valuecached = v;
        }
        idx
    }
}

unsafe fn findopen(mut cap: *mut Capture) -> *mut Capture {
    let mut n: c_int = 0;
    unsafe {
        loop {
            cap = cap.sub(1);
            if isclosecap(cap) {
                n += 1;
            } else if isopencap(cap) {
                if n == 0 {
                    return cap;
                }
                n -= 1;
            }
        }
    }
}

unsafe fn nextcap(cs: *mut CapState) {
    unsafe {
        let mut cap = (*cs).cap;
        if isopencap(cap) {
            let mut n: c_int = 0;
            loop {
                cap = cap.add(1);
                if isopencap(cap) {
                    n += 1;
                } else if isclosecap(cap) {
                    if n == 0 {
                        break;
                    }
                    n -= 1;
                }
            }
            (*cs).cap = cap.add(1);
        } else {
            let mut next = cap.add(1);
            while capinside(cap, next) {
                next = next.add(1);
            }
            (*cs).cap = next;
        }
    }
}

unsafe fn pushnestedvalues(cs: *mut CapState, addextra: bool) -> c_int {
    unsafe {
        let head = (*cs).cap;
        (*cs).cap = (*cs).cap.add(1);
        let mut n: c_int = 0;
        while capinside(head, (*cs).cap) {
            n += pushcapture(cs);
        }
        if addextra || n == 0 {
            lua_pushlstring(
                (*cs).L,
                (*cs).s.add((*head).index as usize),
                closesize(cs, head) as usize,
            );
            n += 1;
        }
        skipclose(cs, head);
        n
    }
}

unsafe fn pushonenestedvalue(cs: *mut CapState) {
    unsafe {
        let n = pushnestedvalues(cs, false);
        if n > 1 {
            lua_pop((*cs).L, n - 1);
        }
    }
}

unsafe fn capvisible(cs: *const CapState, grp: *const Capture, r#ref: *const Capture) -> bool {
    let mut cap = grp;
    let mut i = MAXLOP;
    unsafe {
        while i > 0 {
            i -= 1;
            cap = cap.sub(1);
            if cap <= (*cs).ocap {
                break;
            }
            if isclosecap(cap) || (*grp).index - (*cap).index >= Index_t::from(u8::MAX) {
                return true;
            } else if capinside(cap, grp) {
                return capinside(cap, r#ref);
            }
        }
    }
    true
}

unsafe fn findback(cs: *mut CapState, r#ref: *mut Capture) -> *mut Capture {
    unsafe {
        let l = (*cs).L;
        let mut cap = r#ref;
        while cap > (*cs).ocap {
            cap = cap.sub(1);
            if isclosecap(cap) {
                cap = findopen(cap);
            } else if capinside(cap, r#ref) {
                continue;
            }
            if captype(cap) == Cgroup && capvisible(cs, cap, r#ref) {
                getfromktable(cs, c_int::from((*cap).idx));
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

unsafe fn backrefcap(cs: *mut CapState) -> c_int {
    unsafe {
        let curr = (*cs).cap;
        pushluaval(cs);
        (*cs).cap = findback(cs, curr);
        let n = pushnestedvalues(cs, false);
        (*cs).cap = curr.add(1);
        n
    }
}

unsafe fn tablecap(cs: *mut CapState) -> c_int {
    unsafe {
        let l = (*cs).L;
        let head = (*cs).cap;
        (*cs).cap = (*cs).cap.add(1);
        let mut n: c_int = 0;
        lua_newtable(l);
        while capinside(head, (*cs).cap) {
            if captype((*cs).cap) == Cgroup && (*(*cs).cap).idx != 0 {
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
        skipclose(cs, head);
        1
    }
}

unsafe fn querycap(cs: *mut CapState) -> c_int {
    unsafe {
        let idx = c_int::from((*(*cs).cap).idx);
        pushonenestedvalue(cs);
        let cache_idx = updatecache(cs, idx);
        lua_gettable((*cs).L, cache_idx);
        if !lua_isnil((*cs).L, -1) {
            1
        } else {
            lua_pop((*cs).L, 1);
            0
        }
    }
}

unsafe fn foldcap(cs: *mut CapState) -> c_int {
    unsafe {
        let l = (*cs).L;
        let head = (*cs).cap;
        (*cs).cap = (*cs).cap.add(1);
        let idx = c_int::from((*head).idx);
        if isclosecap((*cs).cap) {
            return luaL_error(l, c"no initial value for fold capture".as_ptr());
        }
        let mut n = pushcapture(cs);
        if n == 0 {
            return luaL_error(l, c"no initial value for fold capture".as_ptr());
        }
        if n > 1 {
            lua_pop(l, n - 1);
        }
        while capinside(head, (*cs).cap) {
            let cache_idx = updatecache(cs, idx);
            lua_pushvalue(l, cache_idx);
            lua_insert(l, -2);
            n = pushcapture(cs);
            lua_call(l, n + 1, 1);
        }
        skipclose(cs, head);
        1
    }
}

unsafe fn functioncap(cs: *mut CapState) -> c_int {
    unsafe {
        let top = lua_gettop((*cs).L);
        pushluaval(cs);
        let n = pushnestedvalues(cs, false);
        lua_call((*cs).L, n, LUA_MULTRET);
        lua_gettop((*cs).L) - top
    }
}

unsafe fn accumulatorcap(cs: *mut CapState) -> c_int {
    unsafe {
        let l = (*cs).L;
        if lua_gettop(l) < (*cs).firstcap {
            luaL_error(l, c"no previous value for accumulator capture".as_ptr());
        }
        pushluaval(cs);
        lua_insert(l, -2);
        let n = pushnestedvalues(cs, false);
        lua_call(l, n + 1, 1);
        0
    }
}

unsafe fn numcap(cs: *mut CapState) -> c_int {
    unsafe {
        let idx = c_int::from((*(*cs).cap).idx);
        if idx == 0 {
            nextcap(cs);
            0
        } else {
            let n = pushnestedvalues(cs, false);
            if n < idx {
                luaL_error((*cs).L, c"no capture '%d'".as_ptr(), idx)
            } else {
                lua_pushvalue((*cs).L, -(n - idx + 1));
                lua_replace((*cs).L, -(n + 1));
                lua_pop((*cs).L, n - 1);
                1
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn finddyncap(mut cap: *mut Capture, last: *mut Capture) -> c_int {
    unsafe {
        while cap < last {
            if (*cap).kind == Cruntime {
                return c_int::from((*cap).idx);
            }
            cap = cap.add(1);
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn runtimecap(
    cs: *mut CapState,
    close: *mut Capture,
    s: *const c_char,
    rem: *mut c_int,
) -> c_int {
    unsafe {
        let l = (*cs).L;
        let otop = lua_gettop(l);
        let open = findopen(close);
        debug_assert_eq!(captype(open), Cgroup);
        let id = finddyncap(open, close);
        (*close).kind = Cclose;
        (*close).index = s.offset_from((*cs).s) as Index_t;
        (*cs).cap = open;
        (*cs).valuecached = 0;
        luaL_checkstack(l, 4, c"too many runtime captures".as_ptr());
        pushluaval(cs);
        lua_pushvalue(l, SUBJIDX);
        lua_pushinteger(l, (s.offset_from((*cs).s) + 1) as lua_Integer);
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
        (close.offset_from(open) - 1) as c_int
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct StrAuxS {
    idx: Index_t,
    siz: Index_t,
}

#[repr(C)]
#[derive(Copy, Clone)]
union StrAuxU {
    cp: *mut Capture,
    s: StrAuxS,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct StrAux {
    isstring: c_int,
    u: StrAuxU,
}

unsafe fn getstrcaps(cs: *mut CapState, cps: *mut StrAux, mut n: c_int) -> c_int {
    unsafe {
        let k = n as usize;
        n += 1;
        let head = (*cs).cap;
        (*cs).cap = (*cs).cap.add(1);
        (*cps.add(k)).isstring = 1;
        (*cps.add(k)).u.s.idx = (*head).index;
        while capinside(head, (*cs).cap) {
            if (n as usize) >= MAXSTRCAPS {
                nextcap(cs);
            } else if captype((*cs).cap) == Csimple {
                n = getstrcaps(cs, cps, n);
            } else {
                (*cps.add(n as usize)).isstring = 0;
                (*cps.add(n as usize)).u.cp = (*cs).cap;
                nextcap(cs);
                n += 1;
            }
        }
        (*cps.add(k)).u.s.siz = closesize(cs, head);
        skipclose(cs, head);
        n
    }
}

unsafe fn stringcap(b: *mut luaL_Buffer, cs: *mut CapState) {
    unsafe {
        let mut cps: [StrAux; MAXSTRCAPS] = zeroed();
        let mut len: usize = 0;
        let cache_idx = updatecache(cs, c_int::from((*(*cs).cap).idx));
        let fmt = lua_tolstring((*cs).L, cache_idx, &mut len);
        let n = getstrcaps(cs, cps.as_mut_ptr(), 0) - 1;
        let mut i = 0usize;
        while i < len {
            let ch = *fmt.add(i);
            if ch != b'%' as c_char {
                luaL_addchar(b, ch);
            } else {
                i += 1;
                let next_ch = *fmt.add(i) as u8;
                if !next_ch.is_ascii_digit() {
                    luaL_addchar(b, *fmt.add(i));
                } else {
                    let l = c_int::from(next_ch - b'0');
                    if l > n {
                        luaL_error((*cs).L, c"invalid capture index (%d)".as_ptr(), l);
                    } else if cps[l as usize].isstring != 0 {
                        luaL_addlstring(
                            b,
                            (*cs).s.add(cps[l as usize].u.s.idx as usize),
                            cps[l as usize].u.s.siz as usize,
                        );
                    } else {
                        let curr = (*cs).cap;
                        (*cs).cap = cps[l as usize].u.cp;
                        if addonestring(b, cs, c"capture".as_ptr()) == 0 {
                            luaL_error((*cs).L, c"no values in capture index %d".as_ptr(), l);
                        }
                        (*cs).cap = curr;
                    }
                }
            }
            i += 1;
        }
    }
}

unsafe fn substcap(b: *mut luaL_Buffer, cs: *mut CapState) {
    unsafe {
        let mut curr = (*cs).s.add((*(*cs).cap).index as usize);
        let head = (*cs).cap;
        (*cs).cap = (*cs).cap.add(1);
        while capinside(head, (*cs).cap) {
            let cap = (*cs).cap;
            let caps = (*cs).s.add((*cap).index as usize);
            luaL_addlstring(b, curr, caps.offset_from(curr) as usize);
            if addonestring(b, cs, c"replacement".as_ptr()) != 0 {
                curr = caps.add(capsize(cap, (*cs).cap.sub(1)) as usize);
            } else {
                curr = caps;
            }
        }
        let end = (*cs).s.add(((*head).index + closesize(cs, head)) as usize);
        luaL_addlstring(b, curr, end.offset_from(curr) as usize);
        skipclose(cs, head);
    }
}

unsafe fn addonestring(b: *mut luaL_Buffer, cs: *mut CapState, what: *const c_char) -> c_int {
    unsafe {
        match captype((*cs).cap) {
            Cstring => {
                stringcap(b, cs);
                1
            }
            Csubst => {
                substcap(b, cs);
                1
            }
            Cacc => luaL_error(
                (*cs).L,
                c"invalid context for an accumulator capture".as_ptr(),
            ),
            _ => {
                let l = (*cs).L;
                let n = pushcapture(cs);
                if n > 0 {
                    if n > 1 {
                        lua_pop(l, n - 1);
                    }
                    if lua_isstring(l, -1) == 0 {
                        return luaL_error(
                            l,
                            c"invalid %s value (a %s)".as_ptr(),
                            what,
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

unsafe fn pushcapture(cs: *mut CapState) -> c_int {
    unsafe {
        let l = (*cs).L;
        luaL_checkstack(l, 4, c"too many captures".as_ptr());
        let old_rec = (*cs).reclevel;
        (*cs).reclevel += 1;
        if old_rec > MAXRECLEVEL {
            return luaL_error(l, c"subcapture nesting too deep".as_ptr());
        }
        let res = match captype((*cs).cap) {
            Cposition => {
                lua_pushinteger(l, ((*(*cs).cap).index + 1) as lua_Integer);
                (*cs).cap = (*cs).cap.add(1);
                1
            }
            Cconst => {
                pushluaval(cs);
                (*cs).cap = (*cs).cap.add(1);
                1
            }
            Carg => {
                let arg = c_int::from((*(*cs).cap).idx);
                (*cs).cap = (*cs).cap.add(1);
                if arg + FIXEDARGS > (*cs).ptop {
                    return luaL_error(l, c"reference to absent extra argument #%d".as_ptr(), arg);
                }
                lua_pushvalue(l, arg + FIXEDARGS);
                1
            }
            Csimple => {
                let k = pushnestedvalues(cs, true);
                lua_insert(l, -k);
                k
            }
            Cruntime => {
                let idx = c_int::from((*(*cs).cap).idx);
                (*cs).cap = (*cs).cap.add(1);
                lua_pushvalue(l, idx);
                1
            }
            Cstring => {
                let mut b: luaL_Buffer = zeroed();
                luaL_buffinit(l, &mut b);
                stringcap(&mut b, cs);
                luaL_pushresult(&mut b);
                1
            }
            Csubst => {
                let mut b: luaL_Buffer = zeroed();
                luaL_buffinit(l, &mut b);
                substcap(&mut b, cs);
                luaL_pushresult(&mut b);
                1
            }
            Cgroup => {
                if (*(*cs).cap).idx == 0 {
                    pushnestedvalues(cs, false)
                } else {
                    nextcap(cs);
                    0
                }
            }
            Cbackref => backrefcap(cs),
            Ctable => tablecap(cs),
            Cfunction => functioncap(cs),
            Cacc => accumulatorcap(cs),
            Cnum => numcap(cs),
            Cquery => querycap(cs),
            Cfold => foldcap(cs),
            _ => {
                debug_assert!(false);
                0
            }
        };
        (*cs).reclevel -= 1;
        res
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn getcaptures(
    l: *mut lua_State,
    s: *const c_char,
    r: *const c_char,
    ptop: c_int,
) -> c_int {
    unsafe {
        let capture = lua_touserdata(l, caplistidx(ptop)).cast::<Capture>();
        let mut n: c_int = 0;
        if !isclosecap(capture) {
            let mut cs = CapState {
                ocap: capture,
                cap: capture,
                L: l,
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
