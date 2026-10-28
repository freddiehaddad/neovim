//! LPeg pattern trees and Lua API (`src/lpeg/lptree.c`).

use std::ffi::c_char;
use std::mem::{offset_of, size_of};
use std::ptr;

use crate::lpeg::lpcap::getcaptures;
use crate::lpeg::lpcode::{compile, fixedlen, hascaptures, nofail, nullable};
use crate::lpeg::lpcset::{charsettype, tocharset};
use crate::lpeg::lpvm::r#match;
use crate::lpeg::{
    CACC, CARG, CBACKREF, CCONST, CFOLD, CFUNCTION, CGROUP, CHARSETSIZE, CNUM, CPOSITION, CQUERY,
    CSIMPLE, CSTRING, CSUBST, CTABLE, Capture, Charset, CharsetInfo, IANY, ICHAR, IFAIL,
    INITCAPSIZE, ISET, Instruction, LpegByte, LpegUint, MAXBACK, MAXBEHIND, MAXINDT, MAXRULES,
    MAXSTACKIDX, NUMSIBLINGS, PATTERN_T, Pattern, SUBJIDX, TAND, TANY, TBEHIND, TCALL, TCAPTURE,
    TCHAR, TCHOICE, TFALSE, TGRAMMAR, TNOT, TOPEN_CALL, TREP, TRULE, TRUN_TIME, TSEQ, TSET, TTRUE,
    TTag, TTree, TUTFR, TXINFO, bytes2slots, clearset, lua_Integer, lua_Number, lua_State,
    lua_Unsigned, luaL_Reg, setchar, sib1, sib2, treebuffer, ttree_bitmap_offset,
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

const MAXLIM: lua_Integer = (i32::MAX / 100) as lua_Integer;

#[inline]
fn pattern_tree_ptr(p: &mut Pattern) -> &'static mut TTree {
    unsafe { &mut *(&raw mut p.tree).cast::<TTree>() }
}

fn val2str(l: *mut lua_State, idx: i32) -> *const c_char {
    unsafe {
        let k = lua_tostring(l, idx);
        if !k.is_null() {
            lua_pushfstring(l, c"%s".as_ptr(), k)
        } else {
            lua_pushfstring(l, c"(a %s)".as_ptr(), luaL_typename(l, idx))
        }
    }
}

fn fixonecall(l: *mut lua_State, postable: i32, g: *mut TTree, t: &mut TTree) {
    unsafe {
        lua_rawgeti(l, -1, i32::from(t.key));
        lua_gettable(l, postable);
        let n = lua_tonumber(l, -1) as i32;
        lua_pop(l, 1);
        if n == 0 {
            lua_rawgeti(l, -1, i32::from(t.key));
            luaL_error(
                l,
                c"rule '%s' undefined in given grammar".as_ptr(),
                val2str(l, -1),
            );
        }
        t.tag = TCALL;
        t.u.ps = n - ((t as *mut TTree).offset_from(g) as i32);
        debug_assert_eq!((*sib2(t)).tag, TRULE);
        (*sib2(t)).key = t.key;
    }
}

fn correctassociativity(tree: &mut TTree) {
    unsafe {
        let t1 = sib1(&raw mut *tree);
        debug_assert!(tree.tag == TCHOICE || tree.tag == TSEQ);
        while (*t1).tag == tree.tag {
            let n1size = tree.u.ps - 1;
            let n11size = (*t1).u.ps - 1;
            let n12size = n1size - n11size - 1;
            ptr::copy(sib1(t1), sib1(&raw mut *tree), n11size as usize);
            tree.u.ps = n11size + 1;
            (*sib2(&raw mut *tree)).tag = tree.tag;
            (*sib2(&raw mut *tree)).u.ps = n12size + 1;
        }
    }
}

fn finalfix(l: *mut lua_State, postable: i32, g: *mut TTree, mut t: *mut TTree) {
    unsafe {
        loop {
            match (*t).tag {
                TGRAMMAR => return,
                TOPEN_CALL => {
                    if !g.is_null() {
                        fixonecall(l, postable, g, &mut *(t));
                    } else {
                        lua_rawgeti(l, -1, i32::from((*t).key));
                        luaL_error(
                            l,
                            c"rule '%s' used outside a grammar".as_ptr(),
                            val2str(l, -1),
                        );
                    }
                }
                TSEQ | TCHOICE => {
                    correctassociativity(&mut *t);
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

fn newktable(l: *mut lua_State, n: i32) {
    unsafe {
        lua_createtable(l, n, 0);
        lua_setfenv(l, -2);
    }
}

fn addtoktable(l: *mut lua_State, idx: i32) -> i32 {
    unsafe {
        if lua_isnil(l, idx) {
            0
        } else {
            lua_getfenv(l, -1);
            let mut n = lua_objlen(l, -1) as i32;
            if n >= i32::from(u16::MAX) {
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

fn ktablelen(l: *mut lua_State, idx: i32) -> i32 {
    unsafe {
        if !lua_istable(l, idx) {
            0
        } else {
            lua_objlen(l, idx) as i32
        }
    }
}

fn concattable(l: *mut lua_State, idx1: i32, idx2: i32) -> i32 {
    unsafe {
        let n1 = ktablelen(l, idx1);
        let n2 = ktablelen(l, idx2);
        if n1 + n2 > i32::from(u16::MAX) {
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

fn correctkeys(mut tree: *mut TTree, n: i32) {
    if n == 0 {
        return;
    }
    unsafe {
        loop {
            match (*tree).tag {
                TOPEN_CALL | TCALL | TRUN_TIME | TRULE => {
                    if (*tree).key > 0 {
                        (*tree).key = (*tree).key.wrapping_add(n as u16);
                    }
                }
                TCAPTURE if (*tree).key > 0 && (*tree).cap != CARG && (*tree).cap != CNUM => {
                    (*tree).key = (*tree).key.wrapping_add(n as u16);
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

fn joinktables(l: *mut lua_State, p1: i32, t2: *mut TTree, p2: i32) {
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

fn copyktable(l: *mut lua_State, idx: i32) {
    unsafe {
        lua_getfenv(l, idx);
        lua_setfenv(l, -2);
    }
}

fn mergektable(l: *mut lua_State, idx: i32, stree: *mut TTree) {
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

fn addtonewktable(l: *mut lua_State, p: i32, idx: i32) -> i32 {
    newktable(l, 1);
    if p != 0 {
        mergektable(l, p, ptr::null_mut());
    }
    addtoktable(l, idx)
}

fn testpattern(l: *mut lua_State, idx: i32) -> bool {
    unsafe {
        if !lua_touserdata(l, idx).is_null() && lua_getmetatable(l, idx) != 0 {
            luaL_getmetatable(l, PATTERN_T);
            if lua_rawequal(l, -1, -2) != 0 {
                lua_pop(l, 2);
                return true;
            }
            lua_pop(l, 2);
        }
        false
    }
}

fn getpattern(l: *mut lua_State, idx: i32) -> &'static mut Pattern {
    unsafe { &mut *luaL_checkudata(l, idx, PATTERN_T).cast::<Pattern>() }
}

fn getsize(l: *mut lua_State, idx: i32) -> i32 {
    unsafe { ((lua_objlen(l, idx) - offset_of!(Pattern, tree)) / size_of::<TTree>()) as i32 }
}

fn gettree(l: *mut lua_State, idx: i32, len: Option<&mut i32>) -> &'static mut TTree {
    let p = getpattern(l, idx);
    if let Some(len) = len {
        *len = getsize(l, idx);
    }
    pattern_tree_ptr(p)
}

fn newtree(l: *mut lua_State, len: i32) -> &'static mut TTree {
    unsafe {
        let size = offset_of!(Pattern, tree) + (len as usize) * size_of::<TTree>();
        let p = lua_newuserdata(l, size).cast::<Pattern>();
        luaL_getmetatable(l, PATTERN_T);
        lua_pushvalue(l, -1);
        lua_setfenv(l, -3);
        lua_setmetatable(l, -2);
        (&raw mut (*p).code).write(Vec::new());
        pattern_tree_ptr(&mut *p)
    }
}

fn newleaf(l: *mut lua_State, tag: TTag) -> &'static mut TTree {
    let tree = newtree(l, 1);
    tree.tag = tag;
    tree
}

fn newcharset(l: *mut lua_State, cs: &[LpegByte; CHARSETSIZE]) -> &'static mut TTree {
    unsafe {
        let mut info = CharsetInfo::new();
        let op = charsettype(cs, &mut info);
        match op {
            IFAIL => newleaf(l, TFALSE),
            IANY => newleaf(l, TANY),
            ICHAR => {
                let tree = newleaf(l, TCHAR);
                let tree = { &mut *tree };
                tree.u.n = info.offset;
                tree
            }
            _ => {
                debug_assert_eq!(op, ISET);
                let bsize = ttree_bitmap_offset() + (info.size as usize);
                let tree = newtree(l, bytes2slots(bsize) as i32);
                let tree = { &mut *tree };
                tree.tag = TSET;
                tree.u.set.offset = info.offset as LpegByte;
                tree.u.set.size = info.size as LpegByte;
                tree.u.set.deflt = info.deflt as LpegByte;
                let tb = treebuffer(tree);
                for i in 0..(info.size as usize) {
                    *tb.wrapping_add(i) = cs[(info.offset as usize) + i];
                }
                tree
            }
        }
    }
}

fn seqaux(tree: &mut TTree, sib: *const TTree, sibsize: i32) -> &'static mut TTree {
    unsafe {
        tree.tag = TSEQ;
        tree.u.ps = sibsize + 1;
        ptr::copy_nonoverlapping(sib, sib1(&raw mut *tree), sibsize as usize);
        &mut *sib2(&raw mut *tree)
    }
}

fn fillseq(mut tree: *mut TTree, tag: TTag, n: i32, s: *const c_char) {
    unsafe {
        for i in 0..(n - 1) {
            (*tree).tag = TSEQ;
            (*tree).u.ps = 2;
            (*sib1(tree)).tag = tag;
            (*sib1(tree)).u.n = if !s.is_null() {
                i32::from(*s.wrapping_add(i as usize) as LpegByte)
            } else {
                0
            };
            tree = sib2(tree);
        }
        (*tree).tag = tag;
        (*tree).u.n = if !s.is_null() {
            i32::from(*s.wrapping_add((n - 1) as usize) as LpegByte)
        } else {
            0
        };
    }
}

fn numtree(l: *mut lua_State, mut n: i32) -> &'static mut TTree {
    unsafe {
        if n == 0 {
            newleaf(l, TTRUE)
        } else {
            let tree: *mut TTree;
            let nd: *mut TTree = if n > 0 {
                tree = newtree(l, 2 * n - 1);
                tree
            } else {
                n = -n;
                tree = newtree(l, 2 * n);
                (*tree).tag = TNOT;
                sib1(tree)
            };
            fillseq(nd, TANY, n, ptr::null());
            &mut *tree
        }
    }
}

fn getpatt(l: *mut lua_State, idx: i32, len: Option<&mut i32>) -> &'static mut TTree {
    unsafe {
        let tree = match lua_type(l, idx) {
            LUA_TSTRING => {
                let mut slen: usize = 0;
                let s = lua_tolstring(l, idx, &mut slen);
                if slen == 0 {
                    newleaf(l, TTRUE)
                } else {
                    let t = newtree(l, (2 * (slen - 1) + 1) as i32);
                    fillseq(t, TCHAR, slen as i32, s);
                    t
                }
            }
            LUA_TNUMBER => {
                let n = lua_tointeger(l, idx) as i32;
                numtree(l, n)
            }
            LUA_TBOOLEAN => {
                if lua_toboolean(l, idx) != 0 {
                    newleaf(l, TTRUE)
                } else {
                    newleaf(l, TFALSE)
                }
            }
            LUA_TTABLE => newgrammar(l, idx),
            LUA_TFUNCTION => {
                let t = newtree(l, 2);
                t.tag = TRUN_TIME;
                t.key = addtonewktable(l, 0, idx) as u16;
                (*sib1(t)).tag = TTRUE;
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

fn newroot1sib(l: *mut lua_State, tag: TTag) -> &'static mut TTree {
    unsafe {
        let mut s1: i32 = 0;
        let tree1 = getpatt(l, 1, Some(&mut s1));
        let tree = newtree(l, 1 + s1);
        tree.tag = tag;
        ptr::copy_nonoverlapping(tree1, sib1(tree), s1 as usize);
        copyktable(l, 1);
        tree
    }
}

fn newroot2sib(l: *mut lua_State, tag: TTag) -> &'static mut TTree {
    unsafe {
        let mut s1: i32 = 0;
        let mut s2: i32 = 0;
        let tree1 = getpatt(l, 1, Some(&mut s1));
        let tree2 = getpatt(l, 2, Some(&mut s2));
        let tree = newtree(l, 1 + s1 + s2);
        tree.tag = tag;
        tree.u.ps = 1 + s1;
        ptr::copy_nonoverlapping(tree1, sib1(tree), s1 as usize);
        ptr::copy_nonoverlapping(tree2, sib2(tree), s2 as usize);
        joinktables(l, 1, sib2(tree), 2);
        tree
    }
}

unsafe extern "C-unwind" fn lp_p(l: *mut lua_State) -> i32 {
    unsafe {
        luaL_checkany(l, 1);
        getpatt(l, 1, None);
        lua_settop(l, 1);
        1
    }
}

unsafe extern "C-unwind" fn lp_seq(l: *mut lua_State) -> i32 {
    unsafe {
        let tree1 = getpatt(l, 1, None);
        let tree2 = getpatt(l, 2, None);
        if tree1.tag == TFALSE || tree2.tag == TTRUE {
            lua_pushvalue(l, 1);
        } else if tree1.tag == TTRUE {
            lua_pushvalue(l, 2);
        } else {
            newroot2sib(l, TSEQ);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_choice(l: *mut lua_State) -> i32 {
    unsafe {
        let mut st1 = Charset::new();
        let mut st2 = Charset::new();
        let t1 = getpatt(l, 1, None);
        let t2 = getpatt(l, 2, None);
        if tocharset(&mut *(t1), &mut st1) != 0 && tocharset(&mut *(t2), &mut st2) != 0 {
            for i in 0..CHARSETSIZE {
                st1.cs[i] |= st2.cs[i];
            }
            newcharset(l, &st1.cs);
        } else if nofail(t1) || t2.tag == TFALSE {
            lua_pushvalue(l, 1);
        } else if t1.tag == TFALSE {
            lua_pushvalue(l, 2);
        } else {
            newroot2sib(l, TCHOICE);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_star(l: *mut lua_State) -> i32 {
    unsafe {
        let mut size1: i32 = 0;
        let mut n = luaL_checkinteger(l, 2) as i32;
        let tree1 = getpatt(l, 1, Some(&mut size1));
        if n >= 0 {
            let mut tree = newtree(l, (n + 1) * (size1 + 1));
            if nullable(tree1) {
                luaL_error(l, c"loop body may accept empty string".as_ptr());
            }
            while n > 0 {
                n -= 1;
                tree = seqaux(&mut *tree, tree1, size1);
            }
            tree.tag = TREP;
            ptr::copy_nonoverlapping(tree1, sib1(tree), size1 as usize);
        } else {
            n = -n;
            let mut tree: *mut TTree = newtree(l, n * (size1 + 3) - 1);
            while n > 1 {
                (*tree).tag = TCHOICE;
                (*tree).u.ps = n * (size1 + 3) - 2;
                (*sib2(tree)).tag = TTRUE;
                tree = sib1(tree);
                tree = seqaux(&mut *tree, tree1, size1);
                n -= 1;
            }
            (*tree).tag = TCHOICE;
            (*tree).u.ps = size1 + 1;
            (*sib2(tree)).tag = TTRUE;
            ptr::copy_nonoverlapping(tree1, sib1(tree), size1 as usize);
        }
        copyktable(l, 1);
        1
    }
}

unsafe extern "C-unwind" fn lp_and(l: *mut lua_State) -> i32 {
    newroot1sib(l, TAND);
    1
}

unsafe extern "C-unwind" fn lp_not(l: *mut lua_State) -> i32 {
    newroot1sib(l, TNOT);
    1
}

unsafe extern "C-unwind" fn lp_sub(l: *mut lua_State) -> i32 {
    unsafe {
        let mut st1 = Charset::new();
        let mut st2 = Charset::new();
        let mut s1: i32 = 0;
        let mut s2: i32 = 0;
        let t1 = getpatt(l, 1, Some(&mut s1));
        let t2 = getpatt(l, 2, Some(&mut s2));
        if tocharset(&mut *(t1), &mut st1) != 0 && tocharset(&mut *(t2), &mut st2) != 0 {
            for i in 0..CHARSETSIZE {
                st1.cs[i] &= !st2.cs[i];
            }
            newcharset(l, &st1.cs);
        } else {
            let tree = newtree(l, 2 + s1 + s2);
            tree.tag = TSEQ;
            tree.u.ps = 2 + s2;
            (*sib1(tree)).tag = TNOT;
            ptr::copy_nonoverlapping(t2, sib1(sib1(tree)), s2 as usize);
            ptr::copy_nonoverlapping(t1, sib2(tree), s1 as usize);
            joinktables(l, 1, sib1(tree), 2);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_set(l: *mut lua_State) -> i32 {
    unsafe {
        let mut len: usize = 0;
        let mut s = luaL_checklstring(l, 1, &mut len);
        let mut buff = [0 as LpegByte; CHARSETSIZE];
        clearset(&mut buff);
        while len > 0 {
            len -= 1;
            setchar(&mut buff, (*s as LpegByte) as usize);
            s = s.wrapping_add(1);
        }
        newcharset(l, &buff);
        1
    }
}

unsafe extern "C-unwind" fn lp_range(l: *mut lua_State) -> i32 {
    unsafe {
        let top = lua_gettop(l);
        let mut buff = [0 as LpegByte; CHARSETSIZE];
        clearset(&mut buff);
        for arg in 1..=top {
            let mut len: usize = 0;
            let r = luaL_checklstring(l, arg, &mut len);
            luaL_argcheck(l, len == 2, arg, c"range must have two characters");
            let start = (*r as LpegByte) as usize;
            let end = (*r.wrapping_add(1) as LpegByte) as usize;
            for c in start..=end {
                setchar(&mut buff, c);
            }
        }
        newcharset(l, &buff);
        1
    }
}

fn codeutftree(l: *mut lua_State, t: &mut TTree, cpu: lua_Unsigned, arg: i32) {
    let cp = cpu as i32;
    let (len, fb) = if cp <= 0x7f {
        (1, cp)
    } else if cp <= 0x7ff {
        (2, 0xC0 | (cp >> 6))
    } else if cp <= 0xffff {
        (3, 0xE0 | (cp >> 12))
    } else {
        luaL_argcheck(l, cpu <= 0x10ffff, arg, c"invalid code point");
        (4, 0xF0 | (cp >> 18))
    };
    t.u.n = cp;
    t.cap = len as LpegByte;
    t.key = fb as u16;
}

unsafe extern "C-unwind" fn lp_utfr(l: *mut lua_State) -> i32 {
    unsafe {
        let from = luaL_checkinteger(l, 1) as lua_Unsigned;
        let to = luaL_checkinteger(l, 2) as lua_Unsigned;
        luaL_argcheck(l, from <= to, 2, c"empty range");
        if to <= 0x7f {
            let mut buff = [0 as LpegByte; CHARSETSIZE];
            clearset(&mut buff);
            for f in from..=to {
                setchar(&mut buff, f);
            }
            newcharset(l, &buff);
        } else {
            let tree = newtree(l, 2);
            tree.tag = TUTFR;
            codeutftree(l, &mut *(tree), from, 1);
            (*sib1(tree)).tag = TXINFO;
            codeutftree(l, &mut *(sib1(tree)), to, 2);
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_behind(l: *mut lua_State) -> i32 {
    let tree1 = getpatt(l, 1, None);
    let n = fixedlen(tree1);
    luaL_argcheck(l, n >= 0, 1, c"pattern may not have fixed length");
    luaL_argcheck(l, hascaptures(tree1) == 0, 1, c"pattern have captures");
    luaL_argcheck(l, n <= MAXBEHIND, 1, c"pattern too long to look behind");
    let tree = newroot1sib(l, TBEHIND);
    tree.u.n = n;
    1
}

unsafe extern "C-unwind" fn lp_v(l: *mut lua_State) -> i32 {
    let tree = newleaf(l, TOPEN_CALL);
    luaL_argcheck(l, !lua_isnoneornil(l, 1), 1, c"non-nil value expected");
    tree.key = addtonewktable(l, 0, 1) as u16;
    1
}

fn capture_aux(l: *mut lua_State, cap: LpegByte, labelidx: i32) -> i32 {
    let tree = newroot1sib(l, TCAPTURE);
    tree.cap = cap;
    tree.key = if labelidx == 0 {
        0
    } else {
        addtonewktable(l, 1, labelidx) as u16
    };
    1
}

fn auxemptycap(tree: *mut TTree, cap: LpegByte) -> &'static mut TTree {
    unsafe {
        (*tree).tag = TCAPTURE;
        (*tree).cap = cap;
        (*sib1(tree)).tag = TTRUE;
        &mut *tree
    }
}

fn newemptycap(l: *mut lua_State, cap: LpegByte, key: i32) -> &'static mut TTree {
    let tree = auxemptycap(newtree(l, 2), cap);
    tree.key = key as u16;
    tree
}

fn newemptycapkey(l: *mut lua_State, cap: LpegByte, idx: i32) -> &'static mut TTree {
    let tree = auxemptycap(newtree(l, 2), cap);
    tree.key = addtonewktable(l, 0, idx) as u16;
    tree
}

unsafe extern "C-unwind" fn lp_divcapture(l: *mut lua_State) -> i32 {
    unsafe {
        match lua_type(l, 2) {
            LUA_TFUNCTION => capture_aux(l, CFUNCTION, 2),
            LUA_TTABLE => capture_aux(l, CQUERY, 2),
            LUA_TSTRING => capture_aux(l, CSTRING, 2),
            LUA_TNUMBER => {
                let n = lua_tointeger(l, 2) as i32;
                let tree = newroot1sib(l, TCAPTURE);
                luaL_argcheck(l, 0 <= n && n <= i32::from(i16::MAX), 1, c"invalid number");
                tree.cap = CNUM;
                tree.key = n as u16;
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

unsafe extern "C-unwind" fn lp_acccapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CACC, 2)
}

unsafe extern "C-unwind" fn lp_substcapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CSUBST, 0)
}

unsafe extern "C-unwind" fn lp_tablecapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CTABLE, 0)
}

unsafe extern "C-unwind" fn lp_groupcapture(l: *mut lua_State) -> i32 {
    if lua_isnoneornil(l, 2) {
        capture_aux(l, CGROUP, 0)
    } else {
        capture_aux(l, CGROUP, 2)
    }
}

unsafe extern "C-unwind" fn lp_foldcapture(l: *mut lua_State) -> i32 {
    unsafe {
        luaL_checktype(l, 2, LUA_TFUNCTION);
        capture_aux(l, CFOLD, 2)
    }
}

unsafe extern "C-unwind" fn lp_simplecapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CSIMPLE, 0)
}

unsafe extern "C-unwind" fn lp_poscapture(l: *mut lua_State) -> i32 {
    newemptycap(l, CPOSITION, 0);
    1
}

unsafe extern "C-unwind" fn lp_argcapture(l: *mut lua_State) -> i32 {
    unsafe {
        let n = luaL_checkinteger(l, 1) as i32;
        luaL_argcheck(
            l,
            0 < n && n <= i32::from(i16::MAX),
            1,
            c"invalid argument index",
        );
        newemptycap(l, CARG, n);
        1
    }
}

unsafe extern "C-unwind" fn lp_backref(l: *mut lua_State) -> i32 {
    unsafe {
        luaL_checkany(l, 1);
        newemptycapkey(l, CBACKREF, 1);
        1
    }
}

unsafe extern "C-unwind" fn lp_constcapture(l: *mut lua_State) -> i32 {
    unsafe {
        let n = lua_gettop(l);
        if n == 0 {
            newleaf(l, TTRUE);
        } else if n == 1 {
            newemptycapkey(l, CCONST, 1);
        } else {
            let mut tree: *mut TTree = newtree(l, 1 + 3 * (n - 1) + 2);
            newktable(l, n);
            (*tree).tag = TCAPTURE;
            (*tree).cap = CGROUP;
            (*tree).key = 0;
            tree = sib1(tree);
            for i in 1..n {
                (*tree).tag = TSEQ;
                (*tree).u.ps = 3;
                auxemptycap(sib1(tree), CCONST);
                (*sib1(tree)).key = addtoktable(l, i) as u16;
                tree = sib2(tree);
            }
            auxemptycap(tree, CCONST);
            (*tree).key = addtoktable(l, n) as u16;
        }
        1
    }
}

unsafe extern "C-unwind" fn lp_matchtime(l: *mut lua_State) -> i32 {
    unsafe {
        luaL_checktype(l, 2, LUA_TFUNCTION);
        let tree = newroot1sib(l, TRUN_TIME);
        tree.key = addtonewktable(l, 1, 2) as u16;
        1
    }
}

fn getfirstrule(l: *mut lua_State, arg: i32, postab: i32) {
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

fn collectrules(l: *mut lua_State, arg: i32, totalsize: &mut i32) -> i32 {
    unsafe {
        let mut n: i32 = 1;
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
            luaL_checkstack(l, LUA_MINSTACK, c"grammar has too many rules");
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

fn buildgrammar(l: *mut lua_State, grammar: *mut TTree, frule: i32, n: i32) {
    unsafe {
        let mut nd = sib1(grammar);
        for i in 0..n {
            let ridx = frule + 2 * i + 1;
            let mut rulesize: i32 = 0;
            let rn = gettree(l, ridx, Some(&mut rulesize));
            let pr = sib1(nd);
            (*nd).tag = TRULE;
            (*nd).key = 0;
            (*pr).tag = TXINFO;
            (*pr).u.n = i;
            (*nd).u.ps = rulesize + 2;
            ptr::copy_nonoverlapping(rn, sib1(pr), rulesize as usize);
            mergektable(l, ridx, sib1(nd));
            nd = sib2(nd);
        }
        (*nd).tag = TTRUE;
    }
}

fn checkloops(mut tree: *mut TTree) -> bool {
    unsafe {
        loop {
            if (*tree).tag == TREP && nullable(sib1(tree)) {
                return true;
            } else if (*tree).tag == TGRAMMAR {
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

fn verifyerror(l: *mut lua_State, passed: &[u16; MAXRULES], npassed: i32) -> i32 {
    unsafe {
        let mut i = npassed - 1;
        while i >= 0 {
            let mut j = i - 1;
            while j >= 0 {
                if passed[i as usize] == passed[j as usize] {
                    lua_rawgeti(l, -1, i32::from(passed[i as usize]));
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
    passed: &mut [u16; MAXRULES],
    mut npassed: i32,
    mut nb: i32,
) -> i32 {
    unsafe {
        loop {
            match (*tree).tag {
                TCHAR | TSET | TANY | TFALSE | TUTFR => return nb,
                TTRUE | TBEHIND => return 1,
                TNOT | TAND | TREP => {
                    tree = sib1(tree);
                    nb = 1;
                }
                TCAPTURE | TRUN_TIME | TXINFO => {
                    tree = sib1(tree);
                }
                TCALL => {
                    tree = sib2(tree);
                }
                TSEQ => {
                    if verifyrule(l, sib1(tree), passed, npassed, 0) == 0 {
                        return nb;
                    }
                    tree = sib2(tree);
                }
                TCHOICE => {
                    nb = verifyrule(l, sib1(tree), passed, npassed, nb);
                    tree = sib2(tree);
                }
                TRULE => {
                    if (npassed as usize) >= MAXRULES {
                        return verifyerror(l, passed, npassed);
                    }
                    passed[npassed as usize] = (*tree).key;
                    npassed += 1;
                    tree = sib1(tree);
                }
                TGRAMMAR => return i32::from(nullable(tree)),
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
        let mut passed = [0_u16; MAXRULES];
        let mut rule = sib1(grammar);
        while (*rule).tag == TRULE {
            if (*rule).key != 0 {
                verifyrule(l, sib1(rule), &mut passed, 0, 0);
            }
            rule = sib2(rule);
        }
        debug_assert_eq!((*rule).tag, TTRUE);

        rule = sib1(grammar);
        while (*rule).tag == TRULE {
            if (*rule).key != 0 && checkloops(sib1(rule)) {
                lua_rawgeti(l, -1, i32::from((*rule).key));
                luaL_error(l, c"empty loop in rule '%s'".as_ptr(), val2str(l, -1));
            }
            rule = sib2(rule);
        }
        debug_assert_eq!((*rule).tag, TTRUE);
    }
}

fn initialrulename(l: *mut lua_State, grammar: *mut TTree, frule: i32) {
    unsafe {
        if (*sib1(grammar)).key == 0 {
            let n = (lua_objlen(l, -1) + 1) as i32;
            lua_pushvalue(l, frule);
            lua_rawseti(l, -2, n);
            (*sib1(grammar)).key = n as u16;
        }
    }
}

fn newgrammar(l: *mut lua_State, arg: i32) -> &'static mut TTree {
    unsafe {
        let mut treesize: i32 = 0;
        let frule = lua_gettop(l) + 2;
        let n = collectrules(l, arg, &mut treesize);
        let g = newtree(l, treesize);
        luaL_argcheck(
            l,
            (n as usize) <= MAXRULES,
            arg,
            c"grammar has too many rules",
        );
        g.tag = TGRAMMAR;
        g.u.n = n;
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

fn prepcompile(l: *mut lua_State, p: &mut Pattern, idx: i32) -> *mut Instruction {
    unsafe {
        lua_getfenv(l, idx);
        finalfix(l, 0, ptr::null_mut(), pattern_tree_ptr(p));
        lua_pop(l, 1);
        compile(l, p, getsize(l, idx) as LpegUint)
    }
}

unsafe extern "C-unwind" fn lp_printtree(l: *mut lua_State) -> i32 {
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

unsafe extern "C-unwind" fn lp_printcode(l: *mut lua_State) -> i32 {
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

unsafe extern "C-unwind" fn lp_match(l: *mut lua_State) -> i32 {
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
        luaL_argcheck(l, len < (MAXINDT as usize), SUBJIDX, c"subject too long");
        lua_pushnil(l);
        lua_pushlightuserdata(l, capture.as_mut_ptr().cast());
        lua_getfenv(l, 1);
        let r = r#match(
            l,
            s,
            s.wrapping_add(i),
            s.wrapping_add(len),
            code,
            capture.as_mut_ptr(),
            ptop,
        );
        if r.is_null() {
            lua_pushnil(l);
            return 1;
        }
        getcaptures(l, s, r, ptop)
    }
}

unsafe extern "C-unwind" fn lp_setmax(l: *mut lua_State) -> i32 {
    unsafe {
        let lim = luaL_checkinteger(l, 1);
        luaL_argcheck(l, 0 < lim && lim <= MAXLIM, 1, c"out of range");
        lua_settop(l, 1);
        lua_setfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX);
        0
    }
}

unsafe extern "C-unwind" fn lp_type(l: *mut lua_State) -> i32 {
    unsafe {
        if testpattern(l, 1) {
            lua_pushlstring(l, c"pattern".as_ptr(), 7);
        } else {
            lua_pushnil(l);
        }
        1
    }
}

pub unsafe extern "C-unwind" fn lp_gc(l: *mut lua_State) -> i32 {
    let p = getpattern(l, 1);
    p.code = Vec::new();
    0
}

fn createcat(l: *mut lua_State, catname: &std::ffi::CStr, catf: fn(u8) -> bool) {
    let mut buff = [0 as LpegByte; CHARSETSIZE];
    clearset(&mut buff);
    for c in 0..=u8::MAX {
        if catf(c) {
            setchar(&mut buff, usize::from(c));
        }
    }
    newcharset(l, &buff);
    lua_setfield(l, -2, catname.as_ptr());
}

unsafe extern "C-unwind" fn lp_locale(l: *mut lua_State) -> i32 {
    unsafe {
        if lua_isnoneornil(l, 1) {
            lua_settop(l, 0);
            lua_createtable(l, 0, 12);
        } else {
            luaL_checktype(l, 1, LUA_TTABLE);
            lua_settop(l, 1);
        }
        createcat(l, c"alnum", |b| b.is_ascii_alphanumeric());
        createcat(l, c"alpha", |b| b.is_ascii_alphabetic());
        createcat(l, c"cntrl", |b| b.is_ascii_control());
        createcat(l, c"digit", |b| b.is_ascii_digit());
        createcat(l, c"graph", |b| b.is_ascii_graphic());
        createcat(l, c"lower", |b| b.is_ascii_lowercase());
        createcat(l, c"print", |b| matches!(b, 0x20..=0x7e));
        createcat(l, c"punct", |b| b.is_ascii_punctuation());
        createcat(l, c"space", |b| matches!(b, b' ' | b'\t'..=b'\r'));
        createcat(l, c"upper", |b| b.is_ascii_uppercase());
        createcat(l, c"xdigit", |b| b.is_ascii_hexdigit());
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
        func: Some(lp_v),
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
        func: Some(lp_p),
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

pub fn luaopen_lpeg(l: *mut lua_State) -> i32 {
    const LPEG_VERSION: &[u8] = b"LPeg 1.1.0";
    unsafe {
        luaL_newmetatable(l, PATTERN_T);
        lua_pushnumber(l, lua_Number::from(MAXBACK));
        lua_setfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX);
        luaL_register(l, ptr::null(), METAREG.as_ptr());
        luaL_register(l, c"lpeg", PATTREG.as_ptr());
        lua_pushvalue(l, -1);
        lua_setfield(l, -3, c"__index");
        lua_pushlstring(
            l,
            LPEG_VERSION.as_ptr().cast::<c_char>(),
            LPEG_VERSION.len(),
        );
        lua_setfield(l, -2, c"version");
        1
    }
}
