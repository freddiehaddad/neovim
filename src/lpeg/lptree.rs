//! LPeg pattern trees and Lua API (`src/lpeg/lptree.c`).

use std::ffi::{c_char, c_int, c_short, c_ushort};
use std::mem::{offset_of, size_of};
use std::ptr;

use crate::lpeg::lpcap::getcaptures;
use crate::lpeg::lpcode::{compile, fixedlen, hascaptures, nofail, nullable};
use crate::lpeg::lpcset::{charsettype, tocharset};
use crate::lpeg::lpvm::r#match;
use crate::lpeg::{
    CHARSETSIZE, Cacc, Capture, Carg, Cbackref, Cconst, Cfold, Cfunction, Cgroup, Charset, Cnum,
    Cposition, Cquery, Csimple, Cstring, Csubst, Ctable, IAny, IChar, IFail, INITCAPSIZE, ISet,
    Instruction, MAXBACK, MAXBEHIND, MAXINDT, MAXRULES, MAXSTACKIDX, NUMSIBLINGS, PATTERN_T,
    Pattern, SUBJIDX, TAnd, TAny, TBehind, TCall, TCapture, TChar, TChoice, TFalse, TGrammar, TNot,
    TOpenCall, TRep, TRule, TRunTime, TSeq, TSet, TTag, TTree, TTrue, TUTFR, TXInfo, byte,
    bytes2slots, charsetinfo, clearset, lua_Integer, lua_Number, lua_State, lua_Unsigned, luaL_Reg,
    setchar, sib1, sib2, treebuffer, ttree_bitmap_offset, uint,
};

use crate::lua::converter::{
    LUA_MINSTACK, LUA_REGISTRYINDEX, LUA_TBOOLEAN, LUA_TFUNCTION, LUA_TNUMBER, LUA_TSTRING,
    LUA_TTABLE, lua_createtable, lua_equal, lua_getfenv, lua_getmetatable, lua_gettable,
    lua_gettop, lua_insert, lua_isnil, lua_isnoneornil, lua_isstring, lua_istable, lua_newtable,
    lua_newuserdata, lua_next, lua_objlen, lua_pop, lua_pushfstring, lua_pushinteger,
    lua_pushlightuserdata, lua_pushlstring, lua_pushnil, lua_pushnumber, lua_pushvalue,
    lua_rawequal, lua_rawgeti, lua_rawseti, lua_replace, lua_setfenv, lua_setfield,
    lua_setmetatable, lua_settable, lua_settop, lua_toboolean, lua_tointeger, lua_tolstring,
    lua_tonumber, lua_tostring, lua_touserdata, lua_type, luaL_argcheck, luaL_checkany,
    luaL_checkinteger, luaL_checklstring, luaL_checkstack, luaL_checktype, luaL_checkudata,
    luaL_error, luaL_getmetatable, luaL_newmetatable, luaL_optinteger, luaL_register,
    luaL_typename,
};

const MAXLIM: lua_Integer = (c_int::MAX / 100) as lua_Integer;

#[inline]
fn pattern_tree_ptr(p: &mut Pattern) -> *mut TTree {
    (&raw mut p.tree).cast::<TTree>()
}

fn val2str(l: *mut lua_State, idx: c_int) -> *const c_char {
    unsafe {
        let k = lua_tostring(l, idx);
        if !k.is_null() {
            lua_pushfstring(l, c"%s".as_ptr(), k)
        } else {
            lua_pushfstring(l, c"(a %s)".as_ptr(), luaL_typename(l, idx))
        }
    }
}

fn fixonecall(l: *mut lua_State, postable: c_int, g: *mut TTree, t: *mut TTree) {
    unsafe {
        lua_rawgeti(l, -1, c_int::from((*t).key));
        lua_gettable(l, postable);
        let n = lua_tonumber(l, -1) as c_int;
        lua_pop(l, 1);
        if n == 0 {
            lua_rawgeti(l, -1, c_int::from((*t).key));
            luaL_error(
                l,
                c"rule '%s' undefined in given grammar".as_ptr(),
                val2str(l, -1),
            );
        }
        (*t).tag = TCall;
        (*t).u.ps = n - (t.offset_from(g) as c_int);
        debug_assert_eq!((*sib2(t)).tag, TRule);
        (*sib2(t)).key = (*t).key;
    }
}

fn correctassociativity(tree: *mut TTree) {
    unsafe {
        let t1 = sib1(tree);
        debug_assert!((*tree).tag == TChoice || (*tree).tag == TSeq);
        while (*t1).tag == (*tree).tag {
            let n1size = (*tree).u.ps - 1;
            let n11size = (*t1).u.ps - 1;
            let n12size = n1size - n11size - 1;
            ptr::copy(sib1(t1), sib1(tree), n11size as usize);
            (*tree).u.ps = n11size + 1;
            (*sib2(tree)).tag = (*tree).tag;
            (*sib2(tree)).u.ps = n12size + 1;
        }
    }
}

fn finalfix(l: *mut lua_State, postable: c_int, g: *mut TTree, mut t: *mut TTree) {
    unsafe {
        loop {
            match (*t).tag {
                TGrammar => return,
                TOpenCall => {
                    if !g.is_null() {
                        fixonecall(l, postable, g, t);
                    } else {
                        lua_rawgeti(l, -1, c_int::from((*t).key));
                        luaL_error(
                            l,
                            c"rule '%s' used outside a grammar".as_ptr(),
                            val2str(l, -1),
                        );
                    }
                }
                TSeq | TChoice => {
                    correctassociativity(t);
                }
                _ => {}
            }
            match NUMSIBLINGS[(*t).tag as usize] {
                1 => {
                    t = sib1(t);
                }
                2 => {
                    finalfix(l, postable, g, sib1(t));
                    t = sib2(t);
                }
                _ => {
                    debug_assert_eq!(NUMSIBLINGS[(*t).tag as usize], 0);
                    return;
                }
            }
        }
    }
}

fn newktable(l: *mut lua_State, n: c_int) {
    unsafe {
        lua_createtable(l, n, 0);
        lua_setfenv(l, -2);
    }
}

fn addtoktable(l: *mut lua_State, idx: c_int) -> c_int {
    unsafe {
        if lua_isnil(l, idx) {
            0
        } else {
            lua_getfenv(l, -1);
            let mut n = lua_objlen(l, -1) as c_int;
            if n >= c_int::from(c_ushort::MAX) {
                luaL_error(l, c"too many Lua values in pattern".as_ptr());
            }
            lua_pushvalue(l, idx);
            n += 1;
            lua_rawseti(l, -2, n);
            lua_pop(l, 1);
            n
        }
    }
}

fn ktablelen(l: *mut lua_State, idx: c_int) -> c_int {
    unsafe {
        if !lua_istable(l, idx) {
            0
        } else {
            lua_objlen(l, idx) as c_int
        }
    }
}

fn concattable(l: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int {
    unsafe {
        let n1 = ktablelen(l, idx1);
        let n2 = ktablelen(l, idx2);
        if n1 + n2 > c_int::from(c_ushort::MAX) {
            luaL_error(l, c"too many Lua values in pattern".as_ptr());
        }
        if n1 == 0 {
            return 0;
        }
        for i in 1..=n1 {
            lua_rawgeti(l, idx1, i);
            lua_rawseti(l, idx2 - 1, n2 + i);
        }
        n2
    }
}

fn correctkeys(mut tree: *mut TTree, n: c_int) {
    if n == 0 {
        return;
    }
    unsafe {
        loop {
            match (*tree).tag {
                TOpenCall | TCall | TRunTime | TRule => {
                    if (*tree).key > 0 {
                        (*tree).key = (*tree).key.wrapping_add(n as c_ushort);
                    }
                }
                TCapture if (*tree).key > 0 && (*tree).cap != Carg && (*tree).cap != Cnum => {
                    (*tree).key = (*tree).key.wrapping_add(n as c_ushort);
                }
                _ => {}
            }
            match NUMSIBLINGS[(*tree).tag as usize] {
                1 => {
                    tree = sib1(tree);
                }
                2 => {
                    correctkeys(sib1(tree), n);
                    tree = sib2(tree);
                }
                _ => {
                    debug_assert_eq!(NUMSIBLINGS[(*tree).tag as usize], 0);
                    return;
                }
            }
        }
    }
}

fn joinktables(l: *mut lua_State, p1: c_int, t2: *mut TTree, p2: c_int) {
    unsafe {
        lua_getfenv(l, p1);
        lua_getfenv(l, p2);
        let n1 = ktablelen(l, -2);
        let n2 = ktablelen(l, -1);
        if n1 == 0 && n2 == 0 {
            lua_pop(l, 2);
        } else if n2 == 0 || lua_equal(l, -2, -1) != 0 {
            lua_pop(l, 1);
            lua_setfenv(l, -2);
        } else if n1 == 0 {
            lua_setfenv(l, -3);
            lua_pop(l, 1);
        } else {
            lua_createtable(l, n1 + n2, 0);
            concattable(l, -3, -1);
            concattable(l, -2, -1);
            lua_setfenv(l, -4);
            lua_pop(l, 2);
            correctkeys(t2, n1);
        }
    }
}

fn copyktable(l: *mut lua_State, idx: c_int) {
    unsafe {
        lua_getfenv(l, idx);
        lua_setfenv(l, -2);
    }
}

fn mergektable(l: *mut lua_State, idx: c_int, stree: *mut TTree) {
    unsafe {
        lua_getfenv(l, -1);
        lua_getfenv(l, idx);
        let n = concattable(l, -1, -2);
        lua_pop(l, 2);
        if !stree.is_null() {
            correctkeys(stree, n);
        }
    }
}

fn addtonewktable(l: *mut lua_State, p: c_int, idx: c_int) -> c_int {
    newktable(l, 1);
        if p != 0 {
            mergektable(l, p, ptr::null_mut());
        }
        addtoktable(l, idx)
}

fn testpattern(l: *mut lua_State, idx: c_int) -> bool {
    unsafe {
        if !lua_touserdata(l, idx).is_null() && lua_getmetatable(l, idx) != 0 {
            luaL_getmetatable(l, PATTERN_T.as_ptr());
            if lua_rawequal(l, -1, -2) != 0 {
                lua_pop(l, 2);
                return true;
            }
            lua_pop(l, 2);
        }
        false
    }
}

fn getpattern(l: *mut lua_State, idx: c_int) -> &'static mut Pattern {
    unsafe { &mut *luaL_checkudata(l, idx, PATTERN_T.as_ptr()).cast::<Pattern>() }
}

fn getsize(l: *mut lua_State, idx: c_int) -> c_int {
    unsafe { ((lua_objlen(l, idx) - offset_of!(Pattern, tree)) / size_of::<TTree>()) as c_int }
}

fn gettree(l: *mut lua_State, idx: c_int, len: Option<&mut c_int>) -> *mut TTree {
    let p = getpattern(l, idx);
    if let Some(len) = len {
        *len = getsize(l, idx);
    }
    pattern_tree_ptr(p)
}

fn newtree(l: *mut lua_State, len: c_int) -> *mut TTree {
    unsafe {
        let size = offset_of!(Pattern, tree) + (len as usize) * size_of::<TTree>();
        let p = lua_newuserdata(l, size).cast::<Pattern>();
        luaL_getmetatable(l, PATTERN_T.as_ptr());
        lua_pushvalue(l, -1);
        lua_setfenv(l, -3);
        lua_setmetatable(l, -2);
        (&raw mut (*p).code).write(Vec::new());
        pattern_tree_ptr(&mut *p)
    }
}

fn newleaf(l: *mut lua_State, tag: TTag) -> *mut TTree {
    unsafe {
        let tree = newtree(l, 1);
        (*tree).tag = tag;
        tree
    }
}

fn newcharset(l: *mut lua_State, cs: &[byte; CHARSETSIZE]) -> *mut TTree {
    unsafe {
        let mut info = charsetinfo::new();
        let op = charsettype(cs, &mut info);
        match op {
            IFail => newleaf(l, TFalse),
            IAny => newleaf(l, TAny),
            IChar => {
                let tree = newleaf(l, TChar);
                (*tree).u.n = info.offset;
                tree
            }
            _ => {
                debug_assert_eq!(op, ISet);
                let bsize = ttree_bitmap_offset() + (info.size as usize);
                let tree = newtree(l, bytes2slots(bsize) as c_int);
                (*tree).tag = TSet;
                (*tree).u.set.offset = info.offset as byte;
                (*tree).u.set.size = info.size as byte;
                (*tree).u.set.deflt = info.deflt as byte;
                let tb = treebuffer(tree);
                for i in 0..(info.size as usize) {
                    *tb.add(i) = cs[(info.offset as usize) + i];
                }
                tree
            }
        }
    }
}

fn seqaux(tree: *mut TTree, sib: *const TTree, sibsize: c_int) -> *mut TTree {
    unsafe {
        (*tree).tag = TSeq;
        (*tree).u.ps = sibsize + 1;
        ptr::copy_nonoverlapping(sib, sib1(tree), sibsize as usize);
        sib2(tree)
    }
}

fn fillseq(mut tree: *mut TTree, tag: TTag, n: c_int, s: *const c_char) {
    unsafe {
        for i in 0..(n - 1) {
            (*tree).tag = TSeq;
            (*tree).u.ps = 2;
            (*sib1(tree)).tag = tag;
            (*sib1(tree)).u.n = if !s.is_null() {
                c_int::from(*s.add(i as usize) as byte)
            } else {
                0
            };
            tree = sib2(tree);
        }
        (*tree).tag = tag;
        (*tree).u.n = if !s.is_null() {
            c_int::from(*s.add((n - 1) as usize) as byte)
        } else {
            0
        };
    }
}

fn numtree(l: *mut lua_State, mut n: c_int) -> *mut TTree {
    unsafe {
        if n == 0 {
            newleaf(l, TTrue)
        } else {
            let tree: *mut TTree;
            let nd: *mut TTree = if n > 0 {
                tree = newtree(l, 2 * n - 1);
                tree
            } else {
                n = -n;
                tree = newtree(l, 2 * n);
                (*tree).tag = TNot;
                sib1(tree)
            };
            fillseq(nd, TAny, n, ptr::null());
            tree
        }
    }
}

fn getpatt(l: *mut lua_State, idx: c_int, len: Option<&mut c_int>) -> *mut TTree {
    unsafe {
        let tree = match lua_type(l, idx) {
            LUA_TSTRING => {
                let mut slen: usize = 0;
                let s = lua_tolstring(l, idx, &mut slen);
                if slen == 0 {
                    newleaf(l, TTrue)
                } else {
                    let t = newtree(l, (2 * (slen - 1) + 1) as c_int);
                    fillseq(t, TChar, slen as c_int, s);
                    t
                }
            }
            LUA_TNUMBER => {
                let n = lua_tointeger(l, idx) as c_int;
                numtree(l, n)
            }
            LUA_TBOOLEAN => {
                if lua_toboolean(l, idx) != 0 {
                    newleaf(l, TTrue)
                } else {
                    newleaf(l, TFalse)
                }
            }
            LUA_TTABLE => newgrammar(l, idx),
            LUA_TFUNCTION => {
                let t = newtree(l, 2);
                (*t).tag = TRunTime;
                (*t).key = addtonewktable(l, 0, idx) as c_ushort;
                (*sib1(t)).tag = TTrue;
                t
            }
            _ => return gettree(l, idx, len),
        };
        lua_replace(l, idx);
        if let Some(len) = len {
            *len = getsize(l, idx);
        }
        tree
    }
}

fn newroot1sib(l: *mut lua_State, tag: TTag) -> *mut TTree {
    unsafe {
        let mut s1: c_int = 0;
        let tree1 = getpatt(l, 1, Some(&mut s1));
        let tree = newtree(l, 1 + s1);
        (*tree).tag = tag;
        ptr::copy_nonoverlapping(tree1, sib1(tree), s1 as usize);
        copyktable(l, 1);
        tree
    }
}

fn newroot2sib(l: *mut lua_State, tag: TTag) -> *mut TTree {
    unsafe {
        let mut s1: c_int = 0;
        let mut s2: c_int = 0;
        let tree1 = getpatt(l, 1, Some(&mut s1));
        let tree2 = getpatt(l, 2, Some(&mut s2));
        let tree = newtree(l, 1 + s1 + s2);
        (*tree).tag = tag;
        (*tree).u.ps = 1 + s1;
        ptr::copy_nonoverlapping(tree1, sib1(tree), s1 as usize);
        ptr::copy_nonoverlapping(tree2, sib2(tree), s2 as usize);
        joinktables(l, 1, sib2(tree), 2);
        tree
    }
}

unsafe extern "C-unwind" fn lp_P(l: *mut lua_State) -> c_int {
    unsafe {
        luaL_checkany(l, 1);
        getpatt(l, 1, None);
        lua_settop(l, 1);
        1
    }
}

unsafe extern "C-unwind" fn lp_seq(l: *mut lua_State) -> c_int {
    unsafe {
        let tree1 = getpatt(l, 1, None);
        let tree2 = getpatt(l, 2, None);
        if (*tree1).tag == TFalse || (*tree2).tag == TTrue {
            lua_pushvalue(l, 1);
        } else if (*tree1).tag == TTrue {
            lua_pushvalue(l, 2);
        } else {
            newroot2sib(l, TSeq);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_choice(l: *mut lua_State) -> c_int {
    unsafe {
        let mut st1 = Charset::new();
        let mut st2 = Charset::new();
        let t1 = getpatt(l, 1, None);
        let t2 = getpatt(l, 2, None);
        if tocharset(t1, &mut st1) != 0 && tocharset(t2, &mut st2) != 0 {
            for i in 0..CHARSETSIZE {
                st1.cs[i] |= st2.cs[i];
            }
            newcharset(l, &st1.cs);
        } else if nofail(t1) || (*t2).tag == TFalse {
            lua_pushvalue(l, 1);
        } else if (*t1).tag == TFalse {
            lua_pushvalue(l, 2);
        } else {
            newroot2sib(l, TChoice);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_star(l: *mut lua_State) -> c_int {
    unsafe {
        let mut size1: c_int = 0;
        let mut n = luaL_checkinteger(l, 2) as c_int;
        let tree1 = getpatt(l, 1, Some(&mut size1));
        if n >= 0 {
            let mut tree = newtree(l, (n + 1) * (size1 + 1));
            if nullable(tree1) {
                luaL_error(l, c"loop body may accept empty string".as_ptr());
            }
            while n > 0 {
                n -= 1;
                tree = seqaux(tree, tree1, size1);
            }
            (*tree).tag = TRep;
            ptr::copy_nonoverlapping(tree1, sib1(tree), size1 as usize);
        } else {
            n = -n;
            let mut tree = newtree(l, n * (size1 + 3) - 1);
            while n > 1 {
                (*tree).tag = TChoice;
                (*tree).u.ps = n * (size1 + 3) - 2;
                (*sib2(tree)).tag = TTrue;
                tree = sib1(tree);
                tree = seqaux(tree, tree1, size1);
                n -= 1;
            }
            (*tree).tag = TChoice;
            (*tree).u.ps = size1 + 1;
            (*sib2(tree)).tag = TTrue;
            ptr::copy_nonoverlapping(tree1, sib1(tree), size1 as usize);
        }
        copyktable(l, 1);
        1
    }
}

unsafe extern "C-unwind" fn lp_and(l: *mut lua_State) -> c_int {
    newroot1sib(l, TAnd);
        1
}

unsafe extern "C-unwind" fn lp_not(l: *mut lua_State) -> c_int {
    newroot1sib(l, TNot);
        1
}

unsafe extern "C-unwind" fn lp_sub(l: *mut lua_State) -> c_int {
    unsafe {
        let mut st1 = Charset::new();
        let mut st2 = Charset::new();
        let mut s1: c_int = 0;
        let mut s2: c_int = 0;
        let t1 = getpatt(l, 1, Some(&mut s1));
        let t2 = getpatt(l, 2, Some(&mut s2));
        if tocharset(t1, &mut st1) != 0 && tocharset(t2, &mut st2) != 0 {
            for i in 0..CHARSETSIZE {
                st1.cs[i] &= !st2.cs[i];
            }
            newcharset(l, &st1.cs);
        } else {
            let tree = newtree(l, 2 + s1 + s2);
            (*tree).tag = TSeq;
            (*tree).u.ps = 2 + s2;
            (*sib1(tree)).tag = TNot;
            ptr::copy_nonoverlapping(t2, sib1(sib1(tree)), s2 as usize);
            ptr::copy_nonoverlapping(t1, sib2(tree), s1 as usize);
            joinktables(l, 1, sib1(tree), 2);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_set(l: *mut lua_State) -> c_int {
    unsafe {
        let mut len: usize = 0;
        let mut s = luaL_checklstring(l, 1, &mut len);
        let mut buff = [0 as byte; CHARSETSIZE];
        clearset(&mut buff);
        while len > 0 {
            len -= 1;
            setchar(&mut buff, (*s as byte) as usize);
            s = s.add(1);
        }
        newcharset(l, &buff);
        1
    }
}

unsafe extern "C-unwind" fn lp_range(l: *mut lua_State) -> c_int {
    unsafe {
        let top = lua_gettop(l);
        let mut buff = [0 as byte; CHARSETSIZE];
        clearset(&mut buff);
        for arg in 1..=top {
            let mut len: usize = 0;
            let r = luaL_checklstring(l, arg, &mut len);
            luaL_argcheck(l, len == 2, arg, c"range must have two characters".as_ptr());
            let start = (*r as byte) as usize;
            let end = (*r.add(1) as byte) as usize;
            for c in start..=end {
                setchar(&mut buff, c);
            }
        }
        newcharset(l, &buff);
        1
    }
}

fn codeutftree(l: *mut lua_State, t: *mut TTree, cpu: lua_Unsigned, arg: c_int) {
    unsafe {
        let cp = cpu as c_int;
        let (len, fb) = if cp <= 0x7f {
            (1, cp)
        } else if cp <= 0x7ff {
            (2, 0xC0 | (cp >> 6))
        } else if cp <= 0xffff {
            (3, 0xE0 | (cp >> 12))
        } else {
            luaL_argcheck(l, cpu <= 0x10ffff, arg, c"invalid code point".as_ptr());
            (4, 0xF0 | (cp >> 18))
        };
        (*t).u.n = cp;
        (*t).cap = len as byte;
        (*t).key = fb as c_ushort;
    }
}

unsafe extern "C-unwind" fn lp_utfr(l: *mut lua_State) -> c_int {
    unsafe {
        let from = luaL_checkinteger(l, 1) as lua_Unsigned;
        let to = luaL_checkinteger(l, 2) as lua_Unsigned;
        luaL_argcheck(l, from <= to, 2, c"empty range".as_ptr());
        if to <= 0x7f {
            let mut buff = [0 as byte; CHARSETSIZE];
            clearset(&mut buff);
            for f in from..=to {
                setchar(&mut buff, f);
            }
            newcharset(l, &buff);
        } else {
            let tree = newtree(l, 2);
            (*tree).tag = TUTFR;
            codeutftree(l, tree, from, 1);
            (*sib1(tree)).tag = TXInfo;
            codeutftree(l, sib1(tree), to, 2);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_behind(l: *mut lua_State) -> c_int {
    unsafe {
        let tree1 = getpatt(l, 1, None);
        let n = fixedlen(tree1);
        luaL_argcheck(l, n >= 0, 1, c"pattern may not have fixed length".as_ptr());
        luaL_argcheck(
            l,
            hascaptures(tree1) == 0,
            1,
            c"pattern have captures".as_ptr(),
        );
        luaL_argcheck(
            l,
            n <= MAXBEHIND,
            1,
            c"pattern too long to look behind".as_ptr(),
        );
        let tree = newroot1sib(l, TBehind);
        (*tree).u.n = n;
        1
    }
}

unsafe extern "C-unwind" fn lp_V(l: *mut lua_State) -> c_int {
    unsafe {
        let tree = newleaf(l, TOpenCall);
        luaL_argcheck(
            l,
            !lua_isnoneornil(l, 1),
            1,
            c"non-nil value expected".as_ptr(),
        );
        (*tree).key = addtonewktable(l, 0, 1) as c_ushort;
        1
    }
}

fn capture_aux(l: *mut lua_State, cap: byte, labelidx: c_int) -> c_int {
    unsafe {
        let tree = newroot1sib(l, TCapture);
        (*tree).cap = cap;
        (*tree).key = if labelidx == 0 {
            0
        } else {
            addtonewktable(l, 1, labelidx) as c_ushort
        };
        1
    }
}

fn auxemptycap(tree: *mut TTree, cap: byte) -> *mut TTree {
    unsafe {
        (*tree).tag = TCapture;
        (*tree).cap = cap;
        (*sib1(tree)).tag = TTrue;
        tree
    }
}

fn newemptycap(l: *mut lua_State, cap: byte, key: c_int) -> *mut TTree {
    unsafe {
        let tree = auxemptycap(newtree(l, 2), cap);
        (*tree).key = key as c_ushort;
        tree
    }
}

fn newemptycapkey(l: *mut lua_State, cap: byte, idx: c_int) -> *mut TTree {
    unsafe {
        let tree = auxemptycap(newtree(l, 2), cap);
        (*tree).key = addtonewktable(l, 0, idx) as c_ushort;
        tree
    }
}

unsafe extern "C-unwind" fn lp_divcapture(l: *mut lua_State) -> c_int {
    unsafe {
        match lua_type(l, 2) {
            LUA_TFUNCTION => capture_aux(l, Cfunction, 2),
            LUA_TTABLE => capture_aux(l, Cquery, 2),
            LUA_TSTRING => capture_aux(l, Cstring, 2),
            LUA_TNUMBER => {
                let n = lua_tointeger(l, 2) as c_int;
                let tree = newroot1sib(l, TCapture);
                luaL_argcheck(
                    l,
                    0 <= n && n <= c_int::from(c_short::MAX),
                    1,
                    c"invalid number".as_ptr(),
                );
                (*tree).cap = Cnum;
                (*tree).key = n as c_ushort;
                1
            }
            _ => luaL_error(
                l,
                c"unexpected %s as 2nd operand to LPeg '/'".as_ptr(),
                luaL_typename(l, 2),
            ),
        }
    }
}

unsafe extern "C-unwind" fn lp_acccapture(l: *mut lua_State) -> c_int {
    capture_aux(l, Cacc, 2)
}

unsafe extern "C-unwind" fn lp_substcapture(l: *mut lua_State) -> c_int {
    capture_aux(l, Csubst, 0)
}

unsafe extern "C-unwind" fn lp_tablecapture(l: *mut lua_State) -> c_int {
    capture_aux(l, Ctable, 0)
}

unsafe extern "C-unwind" fn lp_groupcapture(l: *mut lua_State) -> c_int {
    if lua_isnoneornil(l, 2) {
            capture_aux(l, Cgroup, 0)
        } else {
            capture_aux(l, Cgroup, 2)
        }
}

unsafe extern "C-unwind" fn lp_foldcapture(l: *mut lua_State) -> c_int {
    unsafe {
        luaL_checktype(l, 2, LUA_TFUNCTION);
        capture_aux(l, Cfold, 2)
    }
}

unsafe extern "C-unwind" fn lp_simplecapture(l: *mut lua_State) -> c_int {
    capture_aux(l, Csimple, 0)
}

unsafe extern "C-unwind" fn lp_poscapture(l: *mut lua_State) -> c_int {
    newemptycap(l, Cposition, 0);
        1
}

unsafe extern "C-unwind" fn lp_argcapture(l: *mut lua_State) -> c_int {
    unsafe {
        let n = luaL_checkinteger(l, 1) as c_int;
        luaL_argcheck(
            l,
            0 < n && n <= c_int::from(c_short::MAX),
            1,
            c"invalid argument index".as_ptr(),
        );
        newemptycap(l, Carg, n);
        1
    }
}

unsafe extern "C-unwind" fn lp_backref(l: *mut lua_State) -> c_int {
    unsafe {
        luaL_checkany(l, 1);
        newemptycapkey(l, Cbackref, 1);
        1
    }
}

unsafe extern "C-unwind" fn lp_constcapture(l: *mut lua_State) -> c_int {
    unsafe {
        let n = lua_gettop(l);
        if n == 0 {
            newleaf(l, TTrue);
        } else if n == 1 {
            newemptycapkey(l, Cconst, 1);
        } else {
            let mut tree = newtree(l, 1 + 3 * (n - 1) + 2);
            newktable(l, n);
            (*tree).tag = TCapture;
            (*tree).cap = Cgroup;
            (*tree).key = 0;
            tree = sib1(tree);
            for i in 1..n {
                (*tree).tag = TSeq;
                (*tree).u.ps = 3;
                auxemptycap(sib1(tree), Cconst);
                (*sib1(tree)).key = addtoktable(l, i) as c_ushort;
                tree = sib2(tree);
            }
            auxemptycap(tree, Cconst);
            (*tree).key = addtoktable(l, n) as c_ushort;
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_matchtime(l: *mut lua_State) -> c_int {
    unsafe {
        luaL_checktype(l, 2, LUA_TFUNCTION);
        let tree = newroot1sib(l, TRunTime);
        (*tree).key = addtonewktable(l, 1, 2) as c_ushort;
        1
    }
}

fn getfirstrule(l: *mut lua_State, arg: c_int, postab: c_int) {
    unsafe {
        lua_rawgeti(l, arg, 1);
        if lua_isstring(l, -1) != 0 {
            lua_pushvalue(l, -1);
            lua_gettable(l, arg);
        } else {
            lua_pushinteger(l, 1);
            lua_insert(l, -2);
        }
        if !testpattern(l, -1) {
            if lua_isnil(l, -1) {
                luaL_error(l, c"grammar has no initial rule".as_ptr());
            } else {
                luaL_error(
                    l,
                    c"initial rule '%s' is not a pattern".as_ptr(),
                    lua_tostring(l, -2),
                );
            }
        }
        lua_pushvalue(l, -2);
        lua_pushinteger(l, 1);
        lua_settable(l, postab);
    }
}

fn collectrules(l: *mut lua_State, arg: c_int, totalsize: &mut c_int) -> c_int {
    unsafe {
        let mut n: c_int = 1;
        let postab = lua_gettop(l) + 1;
        lua_newtable(l);
        getfirstrule(l, arg, postab);
        let mut size = 3 + getsize(l, postab + 2);
        lua_pushnil(l);
        while lua_next(l, arg) != 0 {
            if lua_tonumber(l, -2) == 1.0 || lua_equal(l, -2, postab + 1) != 0 {
                lua_pop(l, 1);
                continue;
            }
            if !testpattern(l, -1) {
                luaL_error(l, c"rule '%s' is not a pattern".as_ptr(), val2str(l, -2));
            }
            luaL_checkstack(l, LUA_MINSTACK, c"grammar has too many rules".as_ptr());
            lua_pushvalue(l, -2);
            lua_pushinteger(l, size as lua_Integer);
            lua_settable(l, postab);
            size += 2 + getsize(l, -1);
            lua_pushvalue(l, -2);
            n += 1;
        }
        *totalsize = size + 1;
        n
    }
}

fn buildgrammar(l: *mut lua_State, grammar: *mut TTree, frule: c_int, n: c_int) {
    unsafe {
        let mut nd = sib1(grammar);
        for i in 0..n {
            let ridx = frule + 2 * i + 1;
            let mut rulesize: c_int = 0;
            let rn = gettree(l, ridx, Some(&mut rulesize));
            let pr = sib1(nd);
            (*nd).tag = TRule;
            (*nd).key = 0;
            (*pr).tag = TXInfo;
            (*pr).u.n = i;
            (*nd).u.ps = rulesize + 2;
            ptr::copy_nonoverlapping(rn, sib1(pr), rulesize as usize);
            mergektable(l, ridx, sib1(nd));
            nd = sib2(nd);
        }
        (*nd).tag = TTrue;
    }
}

fn checkloops(mut tree: *mut TTree) -> bool {
    unsafe {
        loop {
            if (*tree).tag == TRep && nullable(sib1(tree)) {
                return true;
            } else if (*tree).tag == TGrammar {
                return false;
            } else {
                match NUMSIBLINGS[(*tree).tag as usize] {
                    1 => {
                        tree = sib1(tree);
                    }
                    2 => {
                        if checkloops(sib1(tree)) {
                            return true;
                        }
                        tree = sib2(tree);
                    }
                    _ => {
                        debug_assert_eq!(NUMSIBLINGS[(*tree).tag as usize], 0);
                        return false;
                    }
                }
            }
        }
    }
}

fn verifyerror(l: *mut lua_State, passed: &[c_ushort; MAXRULES], npassed: c_int) -> c_int {
    unsafe {
        let mut i = npassed - 1;
        while i >= 0 {
            let mut j = i - 1;
            while j >= 0 {
                if passed[i as usize] == passed[j as usize] {
                    lua_rawgeti(l, -1, c_int::from(passed[i as usize]));
                    return luaL_error(
                        l,
                        c"rule '%s' may be left recursive".as_ptr(),
                        val2str(l, -1),
                    );
                }
                j -= 1;
            }
            i -= 1;
        }
        luaL_error(l, c"too many left calls in grammar".as_ptr())
    }
}

fn verifyrule(
    l: *mut lua_State,
    mut tree: *mut TTree,
    passed: &mut [c_ushort; MAXRULES],
    mut npassed: c_int,
    mut nb: c_int,
) -> c_int {
    unsafe {
        loop {
            match (*tree).tag {
                TChar | TSet | TAny | TFalse | TUTFR => return nb,
                TTrue | TBehind => return 1,
                TNot | TAnd | TRep => {
                    tree = sib1(tree);
                    nb = 1;
                }
                TCapture | TRunTime | TXInfo => {
                    tree = sib1(tree);
                }
                TCall => {
                    tree = sib2(tree);
                }
                TSeq => {
                    if verifyrule(l, sib1(tree), passed, npassed, 0) == 0 {
                        return nb;
                    }
                    tree = sib2(tree);
                }
                TChoice => {
                    nb = verifyrule(l, sib1(tree), passed, npassed, nb);
                    tree = sib2(tree);
                }
                TRule => {
                    if (npassed as usize) >= MAXRULES {
                        return verifyerror(l, passed, npassed);
                    }
                    passed[npassed as usize] = (*tree).key;
                    npassed += 1;
                    tree = sib1(tree);
                }
                TGrammar => return c_int::from(nullable(tree)),
                _ => {
                    debug_assert!(false);
                    return 0;
                }
            }
        }
    }
}

fn verifygrammar(l: *mut lua_State, grammar: *mut TTree) {
    unsafe {
        let mut passed = [0 as c_ushort; MAXRULES];
        let mut rule = sib1(grammar);
        while (*rule).tag == TRule {
            if (*rule).key != 0 {
                verifyrule(l, sib1(rule), &mut passed, 0, 0);
            }
            rule = sib2(rule);
        }
        debug_assert_eq!((*rule).tag, TTrue);

        rule = sib1(grammar);
        while (*rule).tag == TRule {
            if (*rule).key != 0 && checkloops(sib1(rule)) {
                lua_rawgeti(l, -1, c_int::from((*rule).key));
                luaL_error(l, c"empty loop in rule '%s'".as_ptr(), val2str(l, -1));
            }
            rule = sib2(rule);
        }
        debug_assert_eq!((*rule).tag, TTrue);
    }
}

fn initialrulename(l: *mut lua_State, grammar: *mut TTree, frule: c_int) {
    unsafe {
        if (*sib1(grammar)).key == 0 {
            let n = (lua_objlen(l, -1) + 1) as c_int;
            lua_pushvalue(l, frule);
            lua_rawseti(l, -2, n);
            (*sib1(grammar)).key = n as c_ushort;
        }
    }
}

fn newgrammar(l: *mut lua_State, arg: c_int) -> *mut TTree {
    unsafe {
        let mut treesize: c_int = 0;
        let frule = lua_gettop(l) + 2;
        let n = collectrules(l, arg, &mut treesize);
        let g = newtree(l, treesize);
        luaL_argcheck(
            l,
            (n as usize) <= MAXRULES,
            arg,
            c"grammar has too many rules".as_ptr(),
        );
        (*g).tag = TGrammar;
        (*g).u.n = n;
        lua_newtable(l);
        lua_setfenv(l, -2);
        buildgrammar(l, g, frule, n);
        lua_getfenv(l, -1);
        finalfix(l, frule - 1, g, sib1(g));
        initialrulename(l, g, frule);
        verifygrammar(l, g);
        lua_pop(l, 1);
        lua_insert(l, -(n * 2 + 2));
        lua_pop(l, n * 2 + 1);
        g
    }
}

fn prepcompile(l: *mut lua_State, p: &mut Pattern, idx: c_int) -> *mut Instruction {
    unsafe {
        lua_getfenv(l, idx);
        finalfix(l, 0, ptr::null_mut(), pattern_tree_ptr(p));
        lua_pop(l, 1);
        compile(l, p, getsize(l, idx) as uint)
    }
}

unsafe extern "C-unwind" fn lp_printtree(l: *mut lua_State) -> c_int {
    unsafe {
        let tree = getpatt(l, 1, None);
        let c = lua_toboolean(l, 2);
        if c != 0 {
            lua_getfenv(l, 1);
            finalfix(l, 0, ptr::null_mut(), tree);
            lua_pop(l, 1);
        }
        luaL_error(l, c"function only implemented in debug mode".as_ptr())
    }
}

unsafe extern "C-unwind" fn lp_printcode(l: *mut lua_State) -> c_int {
    unsafe {
        let _ = getpattern(l, 1);
        luaL_error(l, c"function only implemented in debug mode".as_ptr())
    }
}

fn initposition(l: *mut lua_State, len: usize) -> usize {
    unsafe {
        let ii = luaL_optinteger(l, 3, 1);
        if ii > 0 {
            let u = ii as usize;
            if u <= len { u - 1 } else { len }
        } else {
            let u = (-ii) as usize;
            len.saturating_sub(u)
        }
    }
}

unsafe extern "C-unwind" fn lp_match(l: *mut lua_State) -> c_int {
    unsafe {
        let mut capture: [Capture; INITCAPSIZE] = [Capture::new(); INITCAPSIZE];
        let mut len: usize = 0;
        getpatt(l, 1, None);
        let p = getpattern(l, 1);
        let code = if !p.code.is_empty() {
            p.code.as_mut_ptr()
        } else {
            prepcompile(l, p, 1)
        };
        let s = luaL_checklstring(l, SUBJIDX, &mut len);
        let i = initposition(l, len);
        let ptop = lua_gettop(l);
        luaL_argcheck(
            l,
            len < (MAXINDT as usize),
            SUBJIDX,
            c"subject too long".as_ptr(),
        );
        lua_pushnil(l);
        lua_pushlightuserdata(l, capture.as_mut_ptr().cast());
        lua_getfenv(l, 1);
        let r = r#match(l, s, s.add(i), s.add(len), code, capture.as_mut_ptr(), ptop);
        if r.is_null() {
            lua_pushnil(l);
            return 1;
        }
        getcaptures(l, s, r, ptop)
    }
}

unsafe extern "C-unwind" fn lp_setmax(l: *mut lua_State) -> c_int {
    unsafe {
        let lim = luaL_checkinteger(l, 1);
        luaL_argcheck(l, 0 < lim && lim <= MAXLIM, 1, c"out of range".as_ptr());
        lua_settop(l, 1);
        lua_setfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX.as_ptr());
        0
    }
}

unsafe extern "C-unwind" fn lp_type(l: *mut lua_State) -> c_int {
    unsafe {
        if testpattern(l, 1) {
            lua_pushlstring(l, c"pattern".as_ptr(), 7);
        } else {
            lua_pushnil(l);
        }
        1
    }
}

pub unsafe extern "C-unwind" fn lp_gc(l: *mut lua_State) -> c_int {
    let p = getpattern(l, 1);
    p.code = Vec::new();
    0
}

fn createcat(l: *mut lua_State, catname: *const c_char, catf: fn(u8) -> bool) {
    unsafe {
        let mut buff = [0 as byte; CHARSETSIZE];
        clearset(&mut buff);
        for c in 0..=u8::MAX {
            if catf(c) {
                setchar(&mut buff, usize::from(c));
            }
        }
        newcharset(l, &buff);
        lua_setfield(l, -2, catname);
    }
}

unsafe extern "C-unwind" fn lp_locale(l: *mut lua_State) -> c_int {
    unsafe {
        if lua_isnoneornil(l, 1) {
            lua_settop(l, 0);
            lua_createtable(l, 0, 12);
        } else {
            luaL_checktype(l, 1, LUA_TTABLE);
            lua_settop(l, 1);
        }
        createcat(l, c"alnum".as_ptr(), |b| b.is_ascii_alphanumeric());
        createcat(l, c"alpha".as_ptr(), |b| b.is_ascii_alphabetic());
        createcat(l, c"cntrl".as_ptr(), |b| b.is_ascii_control());
        createcat(l, c"digit".as_ptr(), |b| b.is_ascii_digit());
        createcat(l, c"graph".as_ptr(), |b| b.is_ascii_graphic());
        createcat(l, c"lower".as_ptr(), |b| b.is_ascii_lowercase());
        createcat(l, c"print".as_ptr(), |b| matches!(b, 0x20..=0x7e));
        createcat(l, c"punct".as_ptr(), |b| b.is_ascii_punctuation());
        createcat(l, c"space".as_ptr(), |b| matches!(b, b' ' | b'\t'..=b'\r'));
        createcat(l, c"upper".as_ptr(), |b| b.is_ascii_uppercase());
        createcat(l, c"xdigit".as_ptr(), |b| b.is_ascii_hexdigit());
        1
    }
}

const PATTREG: [luaL_Reg; 24] = [
    luaL_Reg {
        name: c"ptree".as_ptr(),
        func: Some(lp_printtree),
    },
    luaL_Reg {
        name: c"pcode".as_ptr(),
        func: Some(lp_printcode),
    },
    luaL_Reg {
        name: c"match".as_ptr(),
        func: Some(lp_match),
    },
    luaL_Reg {
        name: c"B".as_ptr(),
        func: Some(lp_behind),
    },
    luaL_Reg {
        name: c"V".as_ptr(),
        func: Some(lp_V),
    },
    luaL_Reg {
        name: c"C".as_ptr(),
        func: Some(lp_simplecapture),
    },
    luaL_Reg {
        name: c"Cc".as_ptr(),
        func: Some(lp_constcapture),
    },
    luaL_Reg {
        name: c"Cmt".as_ptr(),
        func: Some(lp_matchtime),
    },
    luaL_Reg {
        name: c"Cb".as_ptr(),
        func: Some(lp_backref),
    },
    luaL_Reg {
        name: c"Carg".as_ptr(),
        func: Some(lp_argcapture),
    },
    luaL_Reg {
        name: c"Cp".as_ptr(),
        func: Some(lp_poscapture),
    },
    luaL_Reg {
        name: c"Cs".as_ptr(),
        func: Some(lp_substcapture),
    },
    luaL_Reg {
        name: c"Ct".as_ptr(),
        func: Some(lp_tablecapture),
    },
    luaL_Reg {
        name: c"Cf".as_ptr(),
        func: Some(lp_foldcapture),
    },
    luaL_Reg {
        name: c"Cg".as_ptr(),
        func: Some(lp_groupcapture),
    },
    luaL_Reg {
        name: c"P".as_ptr(),
        func: Some(lp_P),
    },
    luaL_Reg {
        name: c"S".as_ptr(),
        func: Some(lp_set),
    },
    luaL_Reg {
        name: c"R".as_ptr(),
        func: Some(lp_range),
    },
    luaL_Reg {
        name: c"utfR".as_ptr(),
        func: Some(lp_utfr),
    },
    luaL_Reg {
        name: c"locale".as_ptr(),
        func: Some(lp_locale),
    },
    luaL_Reg {
        name: c"version".as_ptr(),
        func: None,
    },
    luaL_Reg {
        name: c"setmaxstack".as_ptr(),
        func: Some(lp_setmax),
    },
    luaL_Reg {
        name: c"type".as_ptr(),
        func: Some(lp_type),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

const METAREG: [luaL_Reg; 10] = [
    luaL_Reg {
        name: c"__mul".as_ptr(),
        func: Some(lp_seq),
    },
    luaL_Reg {
        name: c"__add".as_ptr(),
        func: Some(lp_choice),
    },
    luaL_Reg {
        name: c"__pow".as_ptr(),
        func: Some(lp_star),
    },
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(lp_gc),
    },
    luaL_Reg {
        name: c"__len".as_ptr(),
        func: Some(lp_and),
    },
    luaL_Reg {
        name: c"__div".as_ptr(),
        func: Some(lp_divcapture),
    },
    luaL_Reg {
        name: c"__mod".as_ptr(),
        func: Some(lp_acccapture),
    },
    luaL_Reg {
        name: c"__unm".as_ptr(),
        func: Some(lp_not),
    },
    luaL_Reg {
        name: c"__sub".as_ptr(),
        func: Some(lp_sub),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

pub fn luaopen_lpeg(l: *mut lua_State) -> c_int {
    const LPEG_VERSION: &[u8] = b"LPeg 1.1.0";
    unsafe {
        luaL_newmetatable(l, PATTERN_T.as_ptr());
        lua_pushnumber(l, lua_Number::from(MAXBACK));
        lua_setfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX.as_ptr());
        luaL_register(l, ptr::null(), METAREG.as_ptr());
        luaL_register(l, c"lpeg".as_ptr(), PATTREG.as_ptr());
        lua_pushvalue(l, -1);
        lua_setfield(l, -3, c"__index".as_ptr());
        lua_pushlstring(
            l,
            LPEG_VERSION.as_ptr().cast::<c_char>(),
            LPEG_VERSION.len(),
        );
        lua_setfield(l, -2, c"version".as_ptr());
        1
    }
}
