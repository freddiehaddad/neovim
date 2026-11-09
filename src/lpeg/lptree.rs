//! LPeg pattern trees and Lua API (`src/lpeg/lptree.c`).
#![forbid(unsafe_code)]

use std::ffi::CStr;

use crate::lpeg::lpcap::getcaptures;
use crate::lpeg::lpcode::{compile, fixedlen, hascaptures, nofail, nullable};
use crate::lpeg::lpcset::{charsettype, tocharset};
use crate::lpeg::lpvm::r#match;
use crate::lpeg::{
    CACC, CARG, CBACKREF, CCONST, CFOLD, CFUNCTION, CGROUP, CHARSETSIZE, CNUM, CPOSITION, CQUERY,
    CSIMPLE, CSTRING, CSUBST, CTABLE, Capture, Charset, CharsetInfo, IANY, ICHAR, IFAIL,
    INITCAPSIZE, ISET, LpegByte, LpegUint, MAXBACK, MAXBEHIND, MAXINDT, MAXRULES, MAXSTACKIDX,
    NUMSIBLINGS, PATTERN_T, Pattern, SUBJIDX, TAND, TANY, TBEHIND, TCALL, TCAPTURE, TCHAR, TCHOICE,
    TFALSE, TGRAMMAR, TNOT, TOPEN_CALL, TREP, TRULE, TRUN_TIME, TSEQ, TSET, TTRUE, TTag, TTree,
    TUTFR, TXINFO, clearset, lua_Integer, lua_Number, lua_State, lua_Unsigned, setchar, sib1_idx,
    sib2_idx,
};

use crate::lua::converter::{
    LUA_MINSTACK, LUA_REGISTRYINDEX, LUA_TBOOLEAN, LUA_TFUNCTION, LUA_TNUMBER, LUA_TSTRING,
    LUA_TTABLE, lua_isnil, lua_isnoneornil, lua_istable, lua_newtable, lua_pop, lua_setfield,
    lua_tostring, luaL_argcheck, luaL_checkstack, luaL_getmetatable, luaL_newmetatable,
    luaL_typename,
};
use crate::lua::ffi::{
    lua_createtable_safe, lua_equal_safe, lua_getfenv_safe, lua_getmetatable_safe,
    lua_gettable_safe, lua_insert_safe, lua_isstring_safe, lua_newuserdata_typed, lua_next_safe,
    lua_objlen_safe, lua_push_bytes, lua_pushinteger_safe, lua_pushnil_safe, lua_pushnumber_safe,
    lua_pushvalue_safe, lua_rawequal_safe, lua_rawgeti_safe, lua_rawseti_safe, lua_replace_safe,
    lua_setfenv_safe, lua_setmetatable_safe, lua_settable_safe, lua_settop_safe,
    lua_toboolean_safe, lua_tointeger_safe, lua_tolstring_bytes, lua_tonumber_safe,
    lua_touserdata_nonnull, lua_type_safe, lua_val2str_cstr, luaL_checkany_safe, luaL_checkbytes,
    luaL_checkinteger_safe, luaL_checktype_safe, luaL_checkudata_typed, luaL_error_cstr,
    luaL_error_cstr_arg, luaL_error_opt_cstr_arg, luaL_optinteger_safe, luaL_register_safe,
    nlua_gettop,
};

const MAXLIM: lua_Integer = (i32::MAX / 100) as lua_Integer;

fn val2str(l: *mut lua_State, idx: i32) -> &'static CStr {
    lua_val2str_cstr(l, idx)
}

fn fixonecall(l: *mut lua_State, postable: i32, tree: &mut [TTree], g_idx: usize, t_idx: usize) {
    let key = i32::from(tree[t_idx].key);
    lua_rawgeti_safe(l, -1, key);
    lua_gettable_safe(l, postable);
    let n = lua_tonumber_safe(l, -1) as i32;
    lua_pop(l, 1);
    if n == 0 {
        lua_rawgeti_safe(l, -1, key);
        luaL_error_cstr_arg(l, c"rule '%s' undefined in given grammar", val2str(l, -1));
    }
    tree[t_idx].tag = TCALL;
    tree[t_idx].u_n = n - (t_idx as i32 - g_idx as i32);
    let s2 = sib2_idx(tree, t_idx);
    debug_assert_eq!(tree[s2].tag, TRULE);
    tree[s2].key = tree[t_idx].key;
}

fn correctassociativity(tree: &mut [TTree], idx: usize) {
    let t1 = sib1_idx(idx);
    debug_assert!(tree[idx].tag == TCHOICE || tree[idx].tag == TSEQ);
    while tree[t1].tag == tree[idx].tag {
        let n1size = tree[idx].u_n - 1;
        let n11size = tree[t1].u_n - 1;
        let n12size = n1size - n11size - 1;
        let copy_len = n11size as usize;
        tree.copy_within((t1 + 1)..(t1 + 1 + copy_len), idx + 1);
        tree[idx].u_n = n11size + 1;
        let s2 = sib2_idx(tree, idx);
        tree[s2].tag = tree[idx].tag;
        tree[s2].u_n = n12size + 1;
    }
}

fn finalfix(
    l: *mut lua_State,
    postable: i32,
    tree: &mut [TTree],
    g_opt: Option<usize>,
    mut t_idx: usize,
) {
    loop {
        match tree[t_idx].tag {
            TGRAMMAR => return,
            TOPEN_CALL => {
                if let Some(g_idx) = g_opt {
                    fixonecall(l, postable, tree, g_idx, t_idx);
                } else {
                    let key = i32::from(tree[t_idx].key);
                    lua_rawgeti_safe(l, -1, key);
                    luaL_error_cstr_arg(l, c"rule '%s' used outside a grammar", val2str(l, -1));
                }
            }
            TSEQ | TCHOICE => {
                correctassociativity(tree, t_idx);
            }
            _ => {}
        }
        match NUMSIBLINGS[tree[t_idx].tag as usize] {
            1 => {
                t_idx = sib1_idx(t_idx);
            }
            2 => {
                finalfix(l, postable, tree, g_opt, sib1_idx(t_idx));
                t_idx = sib2_idx(tree, t_idx);
            }
            _ => {
                debug_assert_eq!(NUMSIBLINGS[tree[t_idx].tag as usize], 0);
                return;
            }
        }
    }
}

fn newktable(l: *mut lua_State, n: i32) {
    lua_createtable_safe(l, n, 0);
    lua_setfenv_safe(l, -2);
}

fn addtoktable(l: *mut lua_State, idx: i32) -> i32 {
    if lua_isnil(l, idx) {
        0
    } else {
        lua_getfenv_safe(l, -1);
        let mut n = lua_objlen_safe(l, -1) as i32;
        if n >= i32::from(u16::MAX) {
            luaL_error_cstr(l, c"too many Lua values in pattern");
        }
        lua_pushvalue_safe(l, idx);
        n += 1;
        lua_rawseti_safe(l, -2, n);
        lua_pop(l, 1);
        n
    }
}

fn ktablelen(l: *mut lua_State, idx: i32) -> i32 {
    if !lua_istable(l, idx) {
        0
    } else {
        lua_objlen_safe(l, idx) as i32
    }
}

fn concattable(l: *mut lua_State, idx1: i32, idx2: i32) -> i32 {
    let n1 = ktablelen(l, idx1);
    let n2 = ktablelen(l, idx2);
    if n1 + n2 > i32::from(u16::MAX) {
        luaL_error_cstr(l, c"too many Lua values in pattern");
    }
    if n1 == 0 {
        return 0;
    }
    for i in 1..=n1 {
        lua_rawgeti_safe(l, idx1, i);
        lua_rawseti_safe(l, idx2 - 1, n2 + i);
    }
    n2
}

fn correctkeys(tree: &mut [TTree], mut idx: usize, n: i32) {
    if n == 0 {
        return;
    }
    loop {
        match tree[idx].tag {
            TOPEN_CALL | TCALL | TRUN_TIME | TRULE => {
                if tree[idx].key > 0 {
                    tree[idx].key = tree[idx].key.wrapping_add(n as u16);
                }
            }
            TCAPTURE if tree[idx].key > 0 && tree[idx].cap != CARG && tree[idx].cap != CNUM => {
                tree[idx].key = tree[idx].key.wrapping_add(n as u16);
            }
            _ => {}
        }
        match NUMSIBLINGS[tree[idx].tag as usize] {
            1 => {
                idx = sib1_idx(idx);
            }
            2 => {
                correctkeys(tree, sib1_idx(idx), n);
                idx = sib2_idx(tree, idx);
            }
            _ => {
                debug_assert_eq!(NUMSIBLINGS[tree[idx].tag as usize], 0);
                return;
            }
        }
    }
}

fn joinktables(l: *mut lua_State, p1: i32, tree: &mut [TTree], t2_idx: usize, p2: i32) {
    lua_getfenv_safe(l, p1);
    lua_getfenv_safe(l, p2);
    let n1 = ktablelen(l, -2);
    let n2 = ktablelen(l, -1);
    if n1 == 0 && n2 == 0 {
        lua_pop(l, 2);
    } else if n2 == 0 || lua_equal_safe(l, -2, -1) {
        lua_pop(l, 1);
        lua_setfenv_safe(l, -2);
    } else if n1 == 0 {
        lua_setfenv_safe(l, -3);
        lua_pop(l, 1);
    } else {
        lua_createtable_safe(l, n1 + n2, 0);
        concattable(l, -3, -1);
        concattable(l, -2, -1);
        lua_setfenv_safe(l, -4);
        lua_pop(l, 2);
        correctkeys(tree, t2_idx, n1);
    }
}

fn copyktable(l: *mut lua_State, idx: i32) {
    lua_getfenv_safe(l, idx);
    lua_setfenv_safe(l, -2);
}

fn mergektable(l: *mut lua_State, idx: i32, tree_opt: Option<(&mut [TTree], usize)>) {
    lua_getfenv_safe(l, -1);
    lua_getfenv_safe(l, idx);
    let n = concattable(l, -1, -2);
    lua_pop(l, 2);
    if let Some((tree, stree_idx)) = tree_opt {
        correctkeys(tree, stree_idx, n);
    }
}

fn addtonewktable(l: *mut lua_State, p: i32, idx: i32) -> i32 {
    newktable(l, 1);
    if p != 0 {
        mergektable(l, p, None);
    }
    addtoktable(l, idx)
}

fn testpattern(l: *mut lua_State, idx: i32) -> bool {
    if lua_touserdata_nonnull(l, idx) && lua_getmetatable_safe(l, idx) {
        luaL_getmetatable(l, PATTERN_T);
        if lua_rawequal_safe(l, -1, -2) {
            lua_pop(l, 2);
            return true;
        }
        lua_pop(l, 2);
    }
    false
}

fn getpattern(l: *mut lua_State, idx: i32) -> &'static mut Pattern {
    luaL_checkudata_typed(l, idx, PATTERN_T)
}

fn getsize(l: *mut lua_State, idx: i32) -> i32 {
    getpattern(l, idx).tree.len() as i32
}

fn gettree(l: *mut lua_State, idx: i32, len: Option<&mut i32>) -> &'static mut [TTree] {
    let p = getpattern(l, idx);
    if let Some(len) = len {
        *len = p.tree.len() as i32;
    }
    &mut p.tree
}

fn newtree(l: *mut lua_State, len: i32) -> &'static mut [TTree] {
    let p = lua_newuserdata_typed(
        l,
        Pattern {
            code: Vec::new(),
            tree: vec![TTree::new(); len as usize],
        },
    );
    luaL_getmetatable(l, PATTERN_T);
    lua_pushvalue_safe(l, -1);
    lua_setfenv_safe(l, -3);
    lua_setmetatable_safe(l, -2);
    &mut p.tree
}

fn newleaf(l: *mut lua_State, tag: TTag) -> &'static mut [TTree] {
    let tree = newtree(l, 1);
    tree[0].tag = tag;
    tree
}

fn newcharset(l: *mut lua_State, cs: &[LpegByte; CHARSETSIZE]) -> &'static mut [TTree] {
    let mut info = CharsetInfo::new();
    let op = charsettype(cs, &mut info);
    match op {
        IFAIL => newleaf(l, TFALSE),
        IANY => newleaf(l, TANY),
        ICHAR => {
            let tree = newleaf(l, TCHAR);
            tree[0].u_n = info.offset;
            tree
        }
        _ => {
            debug_assert_eq!(op, ISET);
            let tree = newtree(l, 1);
            tree[0].tag = TSET;
            tree[0].set_offset = info.offset as LpegByte;
            tree[0].set_size = info.size as LpegByte;
            tree[0].set_deflt = info.deflt as LpegByte;
            let sz = info.size as usize;
            let off = info.offset as usize;
            tree[0].set_bitmap[..sz].copy_from_slice(&cs[off..off + sz]);
            tree
        }
    }
}

fn seqaux(tree: &mut [TTree], idx: usize, sib: &[TTree]) -> usize {
    let sibsize = sib.len();
    tree[idx].tag = TSEQ;
    tree[idx].u_n = (sibsize + 1) as i32;
    tree[idx + 1..idx + 1 + sibsize].copy_from_slice(sib);
    sib2_idx(tree, idx)
}

fn fillseq(tree: &mut [TTree], mut idx: usize, tag: TTag, n: i32, s: Option<&[u8]>) {
    for i in 0..(n - 1) {
        tree[idx].tag = TSEQ;
        tree[idx].u_n = 2;
        let s1 = sib1_idx(idx);
        tree[s1].tag = tag;
        tree[s1].u_n = s.map_or(0, |b| i32::from(b[i as usize]));
        idx = sib2_idx(tree, idx);
    }
    tree[idx].tag = tag;
    tree[idx].u_n = s.map_or(0, |b| i32::from(b[(n - 1) as usize]));
}

fn numtree(l: *mut lua_State, mut n: i32) -> &'static mut [TTree] {
    if n == 0 {
        newleaf(l, TTRUE)
    } else {
        let (tree, nd_idx) = if n > 0 {
            (newtree(l, 2 * n - 1), 0)
        } else {
            n = -n;
            let t = newtree(l, 2 * n);
            t[0].tag = TNOT;
            (t, 1)
        };
        fillseq(tree, nd_idx, TANY, n, None);
        tree
    }
}

fn getpatt(l: *mut lua_State, idx: i32, len: Option<&mut i32>) -> &'static mut [TTree] {
    let tree = match lua_type_safe(l, idx) {
        LUA_TSTRING => {
            let s_bytes = lua_tolstring_bytes(l, idx).to_vec();
            let slen = s_bytes.len();
            if slen == 0 {
                newleaf(l, TTRUE)
            } else {
                let t = newtree(l, (2 * (slen - 1) + 1) as i32);
                fillseq(t, 0, TCHAR, slen as i32, Some(&s_bytes));
                t
            }
        }
        LUA_TNUMBER => {
            let n = lua_tointeger_safe(l, idx) as i32;
            numtree(l, n)
        }
        LUA_TBOOLEAN => {
            if lua_toboolean_safe(l, idx) {
                newleaf(l, TTRUE)
            } else {
                newleaf(l, TFALSE)
            }
        }
        LUA_TTABLE => newgrammar(l, idx),
        LUA_TFUNCTION => {
            let t = newtree(l, 2);
            t[0].tag = TRUN_TIME;
            t[0].key = addtonewktable(l, 0, idx) as u16;
            t[1].tag = TTRUE;
            t
        }
        _ => return gettree(l, idx, len),
    };
    lua_replace_safe(l, idx);
    if let Some(len) = len {
        *len = tree.len() as i32;
    }
    tree
}

fn newroot1sib(l: *mut lua_State, tag: TTag) -> &'static mut [TTree] {
    let mut s1: i32 = 0;
    let tree1 = getpatt(l, 1, Some(&mut s1));
    let tree = newtree(l, 1 + s1);
    tree[0].tag = tag;
    tree[1..1 + (s1 as usize)].copy_from_slice(tree1);
    copyktable(l, 1);
    tree
}

fn newroot2sib(l: *mut lua_State, tag: TTag) -> &'static mut [TTree] {
    let mut s1: i32 = 0;
    let mut s2: i32 = 0;
    let tree1 = getpatt(l, 1, Some(&mut s1));
    let tree2 = getpatt(l, 2, Some(&mut s2));
    let tree = newtree(l, 1 + s1 + s2);
    tree[0].tag = tag;
    tree[0].u_n = 1 + s1;
    let u1 = s1 as usize;
    let u2 = s2 as usize;
    tree[1..1 + u1].copy_from_slice(tree1);
    tree[1 + u1..1 + u1 + u2].copy_from_slice(tree2);
    let s2_idx = sib2_idx(tree, 0);
    joinktables(l, 1, tree, s2_idx, 2);
    tree
}

extern "C-unwind" fn lp_p(l: *mut lua_State) -> i32 {
    luaL_checkany_safe(l, 1);
    getpatt(l, 1, None);
    lua_settop_safe(l, 1);
    1
}

extern "C-unwind" fn lp_seq(l: *mut lua_State) -> i32 {
    let tree1 = getpatt(l, 1, None);
    let tree2 = getpatt(l, 2, None);
    if tree1[0].tag == TFALSE || tree2[0].tag == TTRUE {
        lua_pushvalue_safe(l, 1);
    } else if tree1[0].tag == TTRUE {
        lua_pushvalue_safe(l, 2);
    } else {
        newroot2sib(l, TSEQ);
    }
    1
}

extern "C-unwind" fn lp_choice(l: *mut lua_State) -> i32 {
    let mut st1 = Charset::new();
    let mut st2 = Charset::new();
    let t1 = getpatt(l, 1, None);
    let t2 = getpatt(l, 2, None);
    if tocharset(&t1[0], &mut st1) != 0 && tocharset(&t2[0], &mut st2) != 0 {
        for i in 0..CHARSETSIZE {
            st1.cs[i] |= st2.cs[i];
        }
        newcharset(l, &st1.cs);
    } else if nofail(t1, 0) || t2[0].tag == TFALSE {
        lua_pushvalue_safe(l, 1);
    } else if t1[0].tag == TFALSE {
        lua_pushvalue_safe(l, 2);
    } else {
        newroot2sib(l, TCHOICE);
    }
    1
}

extern "C-unwind" fn lp_star(l: *mut lua_State) -> i32 {
    let mut size1: i32 = 0;
    let mut n = luaL_checkinteger_safe(l, 2) as i32;
    let tree1 = getpatt(l, 1, Some(&mut size1));
    let u1 = size1 as usize;
    if n >= 0 {
        let tree = newtree(l, (n + 1) * (size1 + 1));
        if nullable(tree1, 0) {
            luaL_error_cstr(l, c"loop body may accept empty string");
        }
        let mut idx = 0usize;
        while n > 0 {
            n -= 1;
            idx = seqaux(tree, idx, tree1);
        }
        tree[idx].tag = TREP;
        tree[idx + 1..idx + 1 + u1].copy_from_slice(tree1);
    } else {
        n = -n;
        let tree = newtree(l, n * (size1 + 3) - 1);
        let mut idx = 0usize;
        while n > 1 {
            tree[idx].tag = TCHOICE;
            tree[idx].u_n = n * (size1 + 3) - 2;
            let s2 = sib2_idx(tree, idx);
            tree[s2].tag = TTRUE;
            idx = sib1_idx(idx);
            idx = seqaux(tree, idx, tree1);
            n -= 1;
        }
        tree[idx].tag = TCHOICE;
        tree[idx].u_n = size1 + 1;
        let s2 = sib2_idx(tree, idx);
        tree[s2].tag = TTRUE;
        tree[idx + 1..idx + 1 + u1].copy_from_slice(tree1);
    }
    copyktable(l, 1);
    1
}

extern "C-unwind" fn lp_and(l: *mut lua_State) -> i32 {
    newroot1sib(l, TAND);
    1
}

extern "C-unwind" fn lp_not(l: *mut lua_State) -> i32 {
    newroot1sib(l, TNOT);
    1
}

extern "C-unwind" fn lp_sub(l: *mut lua_State) -> i32 {
    let mut st1 = Charset::new();
    let mut st2 = Charset::new();
    let mut s1: i32 = 0;
    let mut s2: i32 = 0;
    let t1 = getpatt(l, 1, Some(&mut s1));
    let t2 = getpatt(l, 2, Some(&mut s2));
    if tocharset(&t1[0], &mut st1) != 0 && tocharset(&t2[0], &mut st2) != 0 {
        for i in 0..CHARSETSIZE {
            st1.cs[i] &= !st2.cs[i];
        }
        newcharset(l, &st1.cs);
    } else {
        let tree = newtree(l, 2 + s1 + s2);
        tree[0].tag = TSEQ;
        tree[0].u_n = 2 + s2;
        tree[1].tag = TNOT;
        let u1 = s1 as usize;
        let u2 = s2 as usize;
        tree[2..2 + u2].copy_from_slice(t2);
        let s2_idx = sib2_idx(tree, 0);
        tree[s2_idx..s2_idx + u1].copy_from_slice(t1);
        joinktables(l, 1, tree, 1, 2);
    }
    1
}

extern "C-unwind" fn lp_set(l: *mut lua_State) -> i32 {
    let mut buff = [0 as LpegByte; CHARSETSIZE];
    clearset(&mut buff);
    let bytes = luaL_checkbytes(l, 1);
    for &b in bytes {
        setchar(&mut buff, usize::from(b));
    }
    newcharset(l, &buff);
    1
}

extern "C-unwind" fn lp_range(l: *mut lua_State) -> i32 {
    let mut buff = [0 as LpegByte; CHARSETSIZE];
    clearset(&mut buff);
    let top = nlua_gettop(l);
    for arg in 1..=top {
        let r_bytes = luaL_checkbytes(l, arg);
        luaL_argcheck(
            l,
            r_bytes.len() == 2,
            arg,
            c"range must have two characters",
        );
        let start = usize::from(r_bytes[0]);
        let end = usize::from(r_bytes[1]);
        for c in start..=end {
            setchar(&mut buff, c);
        }
    }
    newcharset(l, &buff);
    1
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
    t.u_n = cp;
    t.cap = len as LpegByte;
    t.key = fb as u16;
}

extern "C-unwind" fn lp_utfr(l: *mut lua_State) -> i32 {
    let from = luaL_checkinteger_safe(l, 1) as lua_Unsigned;
    let to = luaL_checkinteger_safe(l, 2) as lua_Unsigned;
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
        tree[0].tag = TUTFR;
        codeutftree(l, &mut tree[0], from, 1);
        tree[1].tag = TXINFO;
        codeutftree(l, &mut tree[1], to, 2);
    }
    1
}

extern "C-unwind" fn lp_behind(l: *mut lua_State) -> i32 {
    let tree1 = getpatt(l, 1, None);
    let n = fixedlen(tree1, 0);
    luaL_argcheck(l, n >= 0, 1, c"pattern may not have fixed length");
    luaL_argcheck(l, hascaptures(tree1, 0) == 0, 1, c"pattern have captures");
    luaL_argcheck(l, n <= MAXBEHIND, 1, c"pattern too long to look behind");
    let tree = newroot1sib(l, TBEHIND);
    tree[0].u_n = n;
    1
}

extern "C-unwind" fn lp_v(l: *mut lua_State) -> i32 {
    let tree = newleaf(l, TOPEN_CALL);
    luaL_argcheck(l, !lua_isnoneornil(l, 1), 1, c"non-nil value expected");
    tree[0].key = addtonewktable(l, 0, 1) as u16;
    1
}

fn capture_aux(l: *mut lua_State, cap: LpegByte, labelidx: i32) -> i32 {
    let tree = newroot1sib(l, TCAPTURE);
    tree[0].cap = cap;
    tree[0].key = if labelidx == 0 {
        0
    } else {
        addtonewktable(l, 1, labelidx) as u16
    };
    1
}

fn auxemptycap(tree: &mut [TTree], idx: usize, cap: LpegByte) {
    tree[idx].tag = TCAPTURE;
    tree[idx].cap = cap;
    tree[sib1_idx(idx)].tag = TTRUE;
}

fn newemptycap(l: *mut lua_State, cap: LpegByte, key: i32) -> &'static mut [TTree] {
    let tree = newtree(l, 2);
    auxemptycap(tree, 0, cap);
    tree[0].key = key as u16;
    tree
}

fn newemptycapkey(l: *mut lua_State, cap: LpegByte, idx: i32) -> &'static mut [TTree] {
    let tree = newtree(l, 2);
    auxemptycap(tree, 0, cap);
    tree[0].key = addtonewktable(l, 0, idx) as u16;
    tree
}

extern "C-unwind" fn lp_divcapture(l: *mut lua_State) -> i32 {
    match lua_type_safe(l, 2) {
        LUA_TFUNCTION => capture_aux(l, CFUNCTION, 2),
        LUA_TTABLE => capture_aux(l, CQUERY, 2),
        LUA_TSTRING => capture_aux(l, CSTRING, 2),
        LUA_TNUMBER => {
            let n = lua_tointeger_safe(l, 2) as i32;
            let tree = newroot1sib(l, TCAPTURE);
            luaL_argcheck(l, 0 <= n && n <= i32::from(i16::MAX), 1, c"invalid number");
            tree[0].cap = CNUM;
            tree[0].key = n as u16;
            1
        }
        _ => luaL_error_cstr_arg(
            l,
            c"unexpected %s as 2nd operand to LPeg '/'",
            luaL_typename(l, 2),
        ),
    }
}

extern "C-unwind" fn lp_acccapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CACC, 2)
}

extern "C-unwind" fn lp_substcapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CSUBST, 0)
}

extern "C-unwind" fn lp_tablecapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CTABLE, 0)
}

extern "C-unwind" fn lp_groupcapture(l: *mut lua_State) -> i32 {
    if lua_isnoneornil(l, 2) {
        capture_aux(l, CGROUP, 0)
    } else {
        capture_aux(l, CGROUP, 2)
    }
}

extern "C-unwind" fn lp_foldcapture(l: *mut lua_State) -> i32 {
    luaL_checktype_safe(l, 2, LUA_TFUNCTION);
    capture_aux(l, CFOLD, 2)
}

extern "C-unwind" fn lp_simplecapture(l: *mut lua_State) -> i32 {
    capture_aux(l, CSIMPLE, 0)
}

extern "C-unwind" fn lp_poscapture(l: *mut lua_State) -> i32 {
    newemptycap(l, CPOSITION, 0);
    1
}

extern "C-unwind" fn lp_argcapture(l: *mut lua_State) -> i32 {
    let n = luaL_checkinteger_safe(l, 1) as i32;
    luaL_argcheck(
        l,
        0 < n && n <= i32::from(i16::MAX),
        1,
        c"invalid argument index",
    );
    newemptycap(l, CARG, n);
    1
}

extern "C-unwind" fn lp_backref(l: *mut lua_State) -> i32 {
    luaL_checkany_safe(l, 1);
    newemptycapkey(l, CBACKREF, 1);
    1
}

extern "C-unwind" fn lp_constcapture(l: *mut lua_State) -> i32 {
    let n = nlua_gettop(l);
    if n == 0 {
        newleaf(l, TTRUE);
    } else if n == 1 {
        newemptycapkey(l, CCONST, 1);
    } else {
        let tree = newtree(l, 1 + 3 * (n - 1) + 2);
        newktable(l, n);
        tree[0].tag = TCAPTURE;
        tree[0].cap = CGROUP;
        tree[0].key = 0;
        let mut idx = 1usize;
        for i in 1..n {
            tree[idx].tag = TSEQ;
            tree[idx].u_n = 3;
            let s1 = sib1_idx(idx);
            auxemptycap(tree, s1, CCONST);
            tree[s1].key = addtoktable(l, i) as u16;
            idx = sib2_idx(tree, idx);
        }
        auxemptycap(tree, idx, CCONST);
        tree[idx].key = addtoktable(l, n) as u16;
    }
    1
}

extern "C-unwind" fn lp_matchtime(l: *mut lua_State) -> i32 {
    luaL_checktype_safe(l, 2, LUA_TFUNCTION);
    let tree = newroot1sib(l, TRUN_TIME);
    tree[0].key = addtonewktable(l, 1, 2) as u16;
    1
}

fn getfirstrule(l: *mut lua_State, arg: i32, postab: i32) {
    lua_rawgeti_safe(l, arg, 1);
    if lua_isstring_safe(l, -1) {
        lua_pushvalue_safe(l, -1);
        lua_gettable_safe(l, arg);
    } else {
        lua_pushinteger_safe(l, 1);
        lua_insert_safe(l, -2);
    }
    if !testpattern(l, -1) {
        if lua_isnil(l, -1) {
            luaL_error_cstr(l, c"grammar has no initial rule");
        } else {
            luaL_error_opt_cstr_arg(
                l,
                c"initial rule '%s' is not a pattern",
                lua_tostring(l, -2),
            );
        }
    }
    lua_pushvalue_safe(l, -2);
    lua_pushinteger_safe(l, 1);
    lua_settable_safe(l, postab);
}

fn collectrules(l: *mut lua_State, arg: i32, totalsize: &mut i32) -> i32 {
    let mut n: i32 = 1;
    let postab = nlua_gettop(l) + 1;
    lua_newtable(l);
    getfirstrule(l, arg, postab);
    let mut size = 3 + getsize(l, postab + 2);
    lua_pushnil_safe(l);
    while lua_next_safe(l, arg) != 0 {
        if lua_tonumber_safe(l, -2) == 1.0 || lua_equal_safe(l, -2, postab + 1) {
            lua_pop(l, 1);
            continue;
        }
        if !testpattern(l, -1) {
            luaL_error_cstr_arg(l, c"rule '%s' is not a pattern", val2str(l, -2));
        }
        luaL_checkstack(l, LUA_MINSTACK, c"grammar has too many rules");
        lua_pushvalue_safe(l, -2);
        lua_pushinteger_safe(l, size as lua_Integer);
        lua_settable_safe(l, postab);
        size += 2 + getsize(l, -1);
        lua_pushvalue_safe(l, -2);
        n += 1;
    }
    *totalsize = size + 1;
    n
}

fn buildgrammar(l: *mut lua_State, grammar: &mut [TTree], frule: i32, n: i32) {
    let mut nd = 1usize;
    for i in 0..n {
        let ridx = frule + 2 * i + 1;
        let mut rulesize: i32 = 0;
        let rn = gettree(l, ridx, Some(&mut rulesize));
        let u_size = rulesize as usize;
        let pr = sib1_idx(nd);
        grammar[nd].tag = TRULE;
        grammar[nd].key = 0;
        grammar[pr].tag = TXINFO;
        grammar[pr].u_n = i;
        grammar[nd].u_n = rulesize + 2;
        let body = sib1_idx(pr);
        grammar[body..body + u_size].copy_from_slice(rn);
        mergektable(l, ridx, Some((grammar, pr)));
        nd = sib2_idx(grammar, nd);
    }
    grammar[nd].tag = TTRUE;
}

fn checkloops(tree: &mut [TTree], mut idx: usize) -> bool {
    loop {
        if tree[idx].tag == TREP && nullable(tree, sib1_idx(idx)) {
            return true;
        } else if tree[idx].tag == TGRAMMAR {
            return false;
        } else {
            match NUMSIBLINGS[tree[idx].tag as usize] {
                1 => {
                    idx = sib1_idx(idx);
                }
                2 => {
                    if checkloops(tree, sib1_idx(idx)) {
                        return true;
                    }
                    idx = sib2_idx(tree, idx);
                }
                _ => {
                    debug_assert_eq!(NUMSIBLINGS[tree[idx].tag as usize], 0);
                    return false;
                }
            }
        }
    }
}

fn verifyerror(l: *mut lua_State, passed: &[u16; MAXRULES], npassed: i32) -> i32 {
    let mut i = npassed - 1;
    while i >= 0 {
        let mut j = i - 1;
        while j >= 0 {
            if passed[i as usize] == passed[j as usize] {
                lua_rawgeti_safe(l, -1, i32::from(passed[i as usize]));
                return luaL_error_cstr_arg(l, c"rule '%s' may be left recursive", val2str(l, -1));
            }
            j -= 1;
        }
        i -= 1;
    }
    luaL_error_cstr(l, c"too many left calls in grammar")
}

fn verifyrule(
    l: *mut lua_State,
    tree: &mut [TTree],
    mut idx: usize,
    passed: &mut [u16; MAXRULES],
    mut npassed: i32,
    mut nb: i32,
) -> i32 {
    loop {
        match tree[idx].tag {
            TCHAR | TSET | TANY | TFALSE | TUTFR => return nb,
            TTRUE | TBEHIND => return 1,
            TNOT | TAND | TREP => {
                idx = sib1_idx(idx);
                nb = 1;
            }
            TCAPTURE | TRUN_TIME | TXINFO => {
                idx = sib1_idx(idx);
            }
            TCALL => {
                idx = sib2_idx(tree, idx);
            }
            TSEQ => {
                if verifyrule(l, tree, sib1_idx(idx), passed, npassed, 0) == 0 {
                    return nb;
                }
                idx = sib2_idx(tree, idx);
            }
            TCHOICE => {
                nb = verifyrule(l, tree, sib1_idx(idx), passed, npassed, nb);
                idx = sib2_idx(tree, idx);
            }
            TRULE => {
                if (npassed as usize) >= MAXRULES {
                    return verifyerror(l, passed, npassed);
                }
                passed[npassed as usize] = tree[idx].key;
                npassed += 1;
                idx = sib1_idx(idx);
            }
            TGRAMMAR => return i32::from(nullable(tree, idx)),
            _ => {
                debug_assert!(false);
                return 0;
            }
        }
    }
}

fn verifygrammar(l: *mut lua_State, grammar: &mut [TTree]) {
    let mut passed = [0_u16; MAXRULES];
    let mut rule = 1usize;
    while grammar[rule].tag == TRULE {
        if grammar[rule].key != 0 {
            verifyrule(l, grammar, sib1_idx(rule), &mut passed, 0, 0);
        }
        rule = sib2_idx(grammar, rule);
    }
    debug_assert_eq!(grammar[rule].tag, TTRUE);

    rule = 1usize;
    while grammar[rule].tag == TRULE {
        if grammar[rule].key != 0 && checkloops(grammar, sib1_idx(rule)) {
            let key = i32::from(grammar[rule].key);
            lua_rawgeti_safe(l, -1, key);
            luaL_error_cstr_arg(l, c"empty loop in rule '%s'", val2str(l, -1));
        }
        rule = sib2_idx(grammar, rule);
    }
    debug_assert_eq!(grammar[rule].tag, TTRUE);
}

fn initialrulename(l: *mut lua_State, grammar: &mut [TTree], frule: i32) {
    if grammar[1].key == 0 {
        let n = (lua_objlen_safe(l, -1) + 1) as i32;
        lua_pushvalue_safe(l, frule);
        lua_rawseti_safe(l, -2, n);
        grammar[1].key = n as u16;
    }
}

fn newgrammar(l: *mut lua_State, arg: i32) -> &'static mut [TTree] {
    let mut treesize: i32 = 0;
    let frule = nlua_gettop(l) + 2;
    let n = collectrules(l, arg, &mut treesize);
    let g = newtree(l, treesize);
    luaL_argcheck(
        l,
        (n as usize) <= MAXRULES,
        arg,
        c"grammar has too many rules",
    );
    g[0].tag = TGRAMMAR;
    g[0].u_n = n;
    lua_newtable(l);
    lua_setfenv_safe(l, -2);
    buildgrammar(l, g, frule, n);
    lua_getfenv_safe(l, -1);
    finalfix(l, frule - 1, g, Some(0), 1);
    initialrulename(l, g, frule);
    verifygrammar(l, g);
    lua_pop(l, 1);
    lua_insert_safe(l, -(n * 2 + 2));
    lua_pop(l, n * 2 + 1);
    g
}

fn prepcompile(l: *mut lua_State, p: &mut Pattern, idx: i32) {
    lua_getfenv_safe(l, idx);
    finalfix(l, 0, &mut p.tree, None, 0);
    lua_pop(l, 1);
    let size = p.tree.len() as LpegUint;
    if let Err(err) = compile(p, size) {
        luaL_error_cstr(l, err);
    }
}

extern "C-unwind" fn lp_printtree(l: *mut lua_State) -> i32 {
    let tree = getpatt(l, 1, None);
    if lua_toboolean_safe(l, 2) {
        lua_getfenv_safe(l, 1);
        finalfix(l, 0, tree, None, 0);
        lua_pop(l, 1);
    }
    luaL_error_cstr(l, c"function only implemented in debug mode")
}

extern "C-unwind" fn lp_printcode(l: *mut lua_State) -> i32 {
    let _ = getpattern(l, 1);
    luaL_error_cstr(l, c"function only implemented in debug mode")
}

fn initposition(l: *mut lua_State, len: usize) -> usize {
    let ii = luaL_optinteger_safe(l, 3, 1);
    if ii > 0 {
        let u = ii as usize;
        if u <= len { u - 1 } else { len }
    } else {
        let u = (-ii) as usize;
        len.saturating_sub(u)
    }
}

extern "C-unwind" fn lp_match(l: *mut lua_State) -> i32 {
    let mut capture: [Capture; INITCAPSIZE] = [Capture::new(); INITCAPSIZE];
    getpatt(l, 1, None);
    let p = getpattern(l, 1);
    if p.code.is_empty() {
        prepcompile(l, p, 1);
    }
    let subject = luaL_checkbytes(l, SUBJIDX);
    let len = subject.len();
    let i = initposition(l, len);
    let ptop = nlua_gettop(l);
    luaL_argcheck(l, len < (MAXINDT as usize), SUBJIDX, c"subject too long");
    lua_pushnil_safe(l);
    lua_pushnil_safe(l);
    lua_getfenv_safe(l, 1);
    let Some((r_pos, caps)) = r#match(l, subject, i, &p.code, &mut capture, ptop) else {
        lua_pushnil_safe(l);
        return 1;
    };
    getcaptures(l, subject, r_pos, caps, ptop)
}

extern "C-unwind" fn lp_setmax(l: *mut lua_State) -> i32 {
    let lim = luaL_checkinteger_safe(l, 1);
    luaL_argcheck(l, 0 < lim && lim <= MAXLIM, 1, c"out of range");
    lua_settop_safe(l, 1);
    lua_setfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX);
    0
}

extern "C-unwind" fn lp_type(l: *mut lua_State) -> i32 {
    if testpattern(l, 1) {
        lua_push_bytes(l, b"pattern");
    } else {
        lua_pushnil_safe(l);
    }
    1
}

pub extern "C-unwind" fn lp_gc(l: *mut lua_State) -> i32 {
    let p = getpattern(l, 1);
    p.code = Vec::new();
    p.tree = Vec::new();
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
    lua_setfield(l, -2, catname);
}

extern "C-unwind" fn lp_locale(l: *mut lua_State) -> i32 {
    if lua_isnoneornil(l, 1) {
        lua_settop_safe(l, 0);
        lua_createtable_safe(l, 0, 12);
    } else {
        luaL_checktype_safe(l, 1, LUA_TTABLE);
        lua_settop_safe(l, 1);
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

const PATTREG: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 22] = [
    (c"ptree", lp_printtree),
    (c"pcode", lp_printcode),
    (c"match", lp_match),
    (c"B", lp_behind),
    (c"V", lp_v),
    (c"C", lp_simplecapture),
    (c"Cc", lp_constcapture),
    (c"Cmt", lp_matchtime),
    (c"Cb", lp_backref),
    (c"Carg", lp_argcapture),
    (c"Cp", lp_poscapture),
    (c"Cs", lp_substcapture),
    (c"Ct", lp_tablecapture),
    (c"Cf", lp_foldcapture),
    (c"Cg", lp_groupcapture),
    (c"P", lp_p),
    (c"S", lp_set),
    (c"R", lp_range),
    (c"utfR", lp_utfr),
    (c"locale", lp_locale),
    (c"setmaxstack", lp_setmax),
    (c"type", lp_type),
];

const METAREG: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 9] = [
    (c"__mul", lp_seq),
    (c"__add", lp_choice),
    (c"__pow", lp_star),
    (c"__gc", lp_gc),
    (c"__len", lp_and),
    (c"__div", lp_divcapture),
    (c"__mod", lp_acccapture),
    (c"__unm", lp_not),
    (c"__sub", lp_sub),
];

pub fn luaopen_lpeg(l: *mut lua_State) -> i32 {
    const LPEG_VERSION: &[u8] = b"LPeg 1.1.0";
    luaL_newmetatable(l, PATTERN_T);
    lua_pushnumber_safe(l, lua_Number::from(MAXBACK));
    lua_setfield(l, LUA_REGISTRYINDEX, MAXSTACKIDX);
    luaL_register_safe(l, None, &METAREG);
    luaL_register_safe(l, Some(c"lpeg"), &PATTREG);
    lua_pushvalue_safe(l, -1);
    lua_setfield(l, -3, c"__index");
    lua_push_bytes(l, LPEG_VERSION);
    lua_setfield(l, -2, c"version");
    1
}
