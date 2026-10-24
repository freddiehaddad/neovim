//! B-tree data structure for storing marks at `(row, col)` positions and updating them
//! to arbitrary text changes (`src/nvim/marktree.c`).

use std::ffi::{CStr, c_char, c_int};
use std::fmt::Write as _;

use crate::api::private::validate::NvimString;
use crate::memory::xmemdupz;

use crate::cursor::colnr_T;

pub const MT_MAX_DEPTH: usize = 20;
pub const MT_BRANCH_FACTOR: usize = 10;
pub const MT_LOG2_BRANCH: c_int = 5;
const T: c_int = MT_BRANCH_FACTOR as c_int;
const T_USIZE: usize = MT_BRANCH_FACTOR;

pub const kMTMetaInline: usize = 0;
pub const kMTMetaLines: usize = 1;
pub const kMTMetaSignHL: usize = 2;
pub const kMTMetaSignText: usize = 3;
pub const kMTMetaConcealLines: usize = 4;
pub const kMTMetaCount: usize = 5;

pub type MetaFilter = *const u32;

pub const MT_FLAG_REAL: u16 = 1 << 0;
pub const MT_FLAG_END: u16 = 1 << 1;
pub const MT_FLAG_PAIRED: u16 = 1 << 2;
pub const MT_FLAG_ORPHANED: u16 = 1 << 3;
pub const MT_FLAG_NO_UNDO: u16 = 1 << 4;
pub const MT_FLAG_INVALIDATE: u16 = 1 << 5;
pub const MT_FLAG_INVALID: u16 = 1 << 6;
pub const MT_FLAG_DECOR_EXT: u16 = 1 << 7;
pub const MT_FLAG_DECOR_HL: u16 = 1 << 8;
pub const MT_FLAG_DECOR_SIGNTEXT: u16 = 1 << 9;
pub const MT_FLAG_DECOR_SIGNHL: u16 = 1 << 10;
pub const MT_FLAG_DECOR_VIRT_LINES: u16 = 1 << 11;
pub const MT_FLAG_DECOR_VIRT_TEXT_INLINE: u16 = 1 << 12;
pub const MT_FLAG_DECOR_CONCEAL_LINES: u16 = 1 << 13;
pub const MT_FLAG_RIGHT_GRAVITY: u16 = 1 << 14;
pub const MT_FLAG_LAST: u16 = 1 << 15;

pub const MT_FLAG_DECOR_MASK: u16 = MT_FLAG_DECOR_EXT
    | MT_FLAG_DECOR_HL
    | MT_FLAG_DECOR_SIGNTEXT
    | MT_FLAG_DECOR_SIGNHL
    | MT_FLAG_DECOR_VIRT_LINES
    | MT_FLAG_DECOR_VIRT_TEXT_INLINE;

pub const MT_FLAG_EXTERNAL_MASK: u16 = MT_FLAG_DECOR_MASK
    | MT_FLAG_NO_UNDO
    | MT_FLAG_INVALIDATE
    | MT_FLAG_INVALID
    | MT_FLAG_DECOR_CONCEAL_LINES;

pub const MARKTREE_END_FLAG: u64 = 1;
pub const DECOR_PRIORITY_BASE: u16 = 0x1000;

const META_MAP: [u32; kMTMetaCount] = [
    MT_FLAG_DECOR_VIRT_TEXT_INLINE as u32,
    MT_FLAG_DECOR_VIRT_LINES as u32,
    MT_FLAG_DECOR_SIGNHL as u32,
    MT_FLAG_DECOR_SIGNTEXT as u32,
    MT_FLAG_DECOR_CONCEAL_LINES as u32,
];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MTPos {
    pub row: i32,
    pub col: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorHighlightInline {
    pub flags: u16,
    pub priority: u16,
    pub hl_id: c_int,
    pub conceal_char: u32,
}

pub const DECOR_HIGHLIGHT_INLINE_INIT: DecorHighlightInline = DecorHighlightInline {
    flags: 0,
    priority: DECOR_PRIORITY_BASE,
    hl_id: 0,
    conceal_char: 0,
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorExt {
    pub sh_idx: u32,
    pub vt: *mut crate::decoration::DecorVirtText,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union DecorInlineData {
    pub hl: DecorHighlightInline,
    pub ext: DecorExt,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MTKey {
    pub pos: MTPos,
    pub ns: u32,
    pub id: u32,
    pub flags: u16,
    pub decor_data: DecorInlineData,
}

pub const MT_INVALID_KEY: MTKey = MTKey {
    pos: MTPos { row: -1, col: -1 },
    ns: 0,
    id: 0,
    flags: 0,
    decor_data: DecorInlineData {
        hl: DECOR_HIGHLIGHT_INLINE_INIT,
    },
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MTPair {
    pub start: MTKey,
    pub end_pos: MTPos,
    pub end_right_gravity: bool,
}

pub type Intersection = Vec<u64>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct mtnode_inner_s {
    pub i_ptr: [*mut MTNode; 2 * MT_BRANCH_FACTOR],
    pub i_meta: [[u32; kMTMetaCount]; 2 * MT_BRANCH_FACTOR],
}

#[repr(C)]
pub struct MTNode {
    pub n: i32,
    pub level: i16,
    pub p_idx: i16,
    pub intersect: Intersection,
    pub parent: *mut MTNode,
    pub key: [MTKey; 2 * MT_BRANCH_FACTOR - 1],
    pub inner: Option<Box<mtnode_inner_s>>,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MarkTreeIterState {
    pub oldcol: c_int,
    pub i: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MarkTreeIter {
    pub pos: MTPos,
    pub lvl: c_int,
    pub x: *mut MTNode,
    pub i: c_int,
    pub s: [MarkTreeIterState; MT_MAX_DEPTH],
    pub intersect_idx: usize,
    pub intersect_pos: MTPos,
    pub intersect_pos_x: MTPos,
}

impl Default for MarkTreeIter {
    fn default() -> Self {
        Self {
            pos: MTPos { row: 0, col: 0 },
            lvl: 0,
            x: std::ptr::null_mut(),
            i: 0,
            s: [MarkTreeIterState::default(); MT_MAX_DEPTH],
            intersect_idx: 0,
            intersect_pos: MTPos { row: 0, col: 0 },
            intersect_pos_x: MTPos { row: 0, col: 0 },
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MTDamage {
    pub old: *mut MTNode,
    pub new: *mut MTNode,
    pub old_i: c_int,
    pub new_i: c_int,
}

impl Default for MTDamage {
    fn default() -> Self {
        Self {
            old: std::ptr::null_mut(),
            new: std::ptr::null_mut(),
            old_i: 0,
            new_i: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MTDamagePair {
    pub start: MTDamage,
    pub end: MTDamage,
}

pub type PMapUint64 = std::collections::BTreeMap<u64, *mut MTNode>;
#[cfg(test)]
pub type PMapPtr = std::collections::BTreeMap<*mut MTNode, Vec<u64>>;
pub type MTDamageMap = std::collections::BTreeMap<u64, MTDamagePair>;

#[repr(C)]
pub struct MarkTree {
    pub root: *mut MTNode,
    pub meta_root: [u32; kMTMetaCount],
    pub n_keys: usize,
    pub n_nodes: usize,
    pub id2node: PMapUint64,
}

impl MarkTree {
    pub const fn new() -> Self {
        Self {
            root: std::ptr::null_mut(),
            meta_root: [0; kMTMetaCount],
            n_keys: 0,
            n_nodes: 0,
            id2node: std::collections::BTreeMap::new(),
        }
    }
}

#[inline]
pub const fn mt_lookup_id(ns: u32, id: u32, enda: bool) -> u64 {
    ((ns as u64) << 33) | ((id as u64) << 1) | if enda { MARKTREE_END_FLAG } else { 0 }
}

#[inline]
const fn mt_lookup_key_side(key: MTKey, end: bool) -> u64 {
    mt_lookup_id(key.ns, key.id, end)
}

#[inline]
pub const fn mt_lookup_key(key: MTKey) -> u64 {
    mt_lookup_id(key.ns, key.id, (key.flags & MT_FLAG_END) != 0)
}

#[inline]
pub const fn mt_paired(key: MTKey) -> bool {
    (key.flags & MT_FLAG_PAIRED) != 0
}

#[inline]
pub const fn mt_end(key: MTKey) -> bool {
    (key.flags & MT_FLAG_END) != 0
}

#[inline]
pub const fn mt_start(key: MTKey) -> bool {
    mt_paired(key) && !mt_end(key)
}

#[inline]
pub const fn mt_right(key: MTKey) -> bool {
    (key.flags & MT_FLAG_RIGHT_GRAVITY) != 0
}

#[inline]
pub const fn mt_invalid(key: MTKey) -> bool {
    (key.flags & MT_FLAG_INVALID) != 0
}

#[inline]
pub const fn mt_no_undo(key: MTKey) -> bool {
    (key.flags & MT_FLAG_NO_UNDO) != 0
}

#[inline]
pub const fn mt_invalidate(key: MTKey) -> bool {
    (key.flags & MT_FLAG_INVALIDATE) != 0
}

#[inline]
pub const fn mt_decor_any(key: MTKey) -> bool {
    (key.flags & MT_FLAG_DECOR_MASK) != 0
}

#[inline]
pub const fn mt_decor_sign(key: MTKey) -> bool {
    (key.flags & (MT_FLAG_DECOR_SIGNTEXT | MT_FLAG_DECOR_SIGNHL)) != 0
}

#[inline]
pub const fn mt_decor(key: MTKey) -> crate::decoration::DecorInline {
    crate::decoration::DecorInline {
        ext: (key.flags & MT_FLAG_DECOR_EXT) != 0,
        data: key.decor_data,
    }
}

#[inline]
pub const fn mt_flags(
    right_gravity: bool,
    no_undo: bool,
    invalidate: bool,
    decor_ext: bool,
) -> u16 {
    (if right_gravity {
        MT_FLAG_RIGHT_GRAVITY
    } else {
        0
    }) | (if no_undo { MT_FLAG_NO_UNDO } else { 0 })
        | (if invalidate { MT_FLAG_INVALIDATE } else { 0 })
        | (if decor_ext { MT_FLAG_DECOR_EXT } else { 0 })
}

#[inline]
pub const fn mtpair_from(start: MTKey, end: MTKey) -> MTPair {
    MTPair {
        start,
        end_pos: end.pos,
        end_right_gravity: mt_right(end),
    }
}

#[inline]
const fn pos_leq(a: MTPos, b: MTPos) -> bool {
    a.row < b.row || (a.row == b.row && a.col <= b.col)
}

#[inline]
const fn pos_less(a: MTPos, b: MTPos) -> bool {
    !pos_leq(b, a)
}

#[inline]
fn relative(base: MTPos, val: &mut MTPos) {
    debug_assert!(pos_leq(base, *val));
    if val.row == base.row {
        val.row = 0;
        val.col -= base.col;
    } else {
        val.row -= base.row;
    }
}

#[inline]
fn unrelative(base: MTPos, val: &mut MTPos) {
    if val.row == 0 {
        val.row = base.row;
        val.col += base.col;
    } else {
        val.row += base.row;
    }
}

#[inline]
fn compose(base: &mut MTPos, val: MTPos) {
    if val.row == 0 {
        base.col += val.col;
    } else {
        base.row += val.row;
        base.col = val.col;
    }
}

#[inline]
fn key_cmp(a: MTKey, b: MTKey) -> c_int {
    let cmp_row = c_int::from(a.pos.row > b.pos.row) - c_int::from(a.pos.row < b.pos.row);
    if cmp_row != 0 {
        return cmp_row;
    }
    let cmp_col = c_int::from(a.pos.col > b.pos.col) - c_int::from(a.pos.col < b.pos.col);
    if cmp_col != 0 {
        return cmp_col;
    }
    const CMP_MASK: u16 = MT_FLAG_RIGHT_GRAVITY | MT_FLAG_END | MT_FLAG_REAL | MT_FLAG_LAST;
    let af = a.flags & CMP_MASK;
    let bf = b.flags & CMP_MASK;
    c_int::from(af > bf) - c_int::from(af < bf)
}

#[inline]
unsafe fn node_ptr(x: *mut MTNode) -> *mut *mut MTNode {
    unsafe {
        (*x).inner
            .as_deref_mut()
            .expect("node_ptr called on leaf node")
            .i_ptr
            .as_mut_ptr()
    }
}

#[inline]
unsafe fn node_meta(x: *mut MTNode) -> *mut [u32; kMTMetaCount] {
    unsafe {
        (*x).inner
            .as_deref_mut()
            .expect("node_meta called on leaf node")
            .i_meta
            .as_mut_ptr()
    }
}

#[inline]
unsafe fn rawkey(itr: *mut MarkTreeIter) -> *mut MTKey {
    unsafe { (*(*itr).x).key.as_mut_ptr().add((*itr).i as usize) }
}

#[inline]
unsafe fn marktree_getp_aux(x: *const MTNode, k: MTKey, match_out: Option<&mut bool>) -> c_int {
    let mut dummy_match = false;
    let m = match_out.unwrap_or(&mut dummy_match);

    let xn = unsafe { (*x).n };
    if xn == 0 {
        *m = false;
        return -1;
    }
    let mut begin: c_int = 0;
    let mut end: c_int = xn;
    while begin < end {
        let mid = (begin + end) >> 1;
        if key_cmp(unsafe { (*x).key[mid as usize] }, k) < 0 {
            begin = mid + 1;
        } else {
            end = mid;
        }
    }
    if begin == xn {
        *m = false;
        return xn - 1;
    }
    *m = key_cmp(k, unsafe { (*x).key[begin as usize] }) == 0;
    if !*m {
        begin -= 1;
    }
    begin
}

#[inline]
unsafe fn refkey(b: *mut MarkTree, x: *mut MTNode, i: c_int) {
    unsafe {
        let key_id = mt_lookup_key((*x).key[i as usize]);
        (*b).id2node.insert(key_id, x);
    }
}

#[inline]
unsafe fn id2node(b: *mut MarkTree, id: u64) -> *mut MTNode {
    unsafe {
        (*b).id2node
            .get(&id)
            .copied()
            .unwrap_or(std::ptr::null_mut())
    }
}

unsafe fn split_node(b: *mut MarkTree, x: *mut MTNode, i: c_int, next: MTKey) {
    unsafe {
        let x_ptr = node_ptr(x);
        let x_meta = node_meta(x);
        let y = *x_ptr.add(i as usize);
        let z = marktree_alloc_node(b, (*y).level != 0);
        (*z).level = (*y).level;
        (*z).n = T - 1;

        let last_start = if mt_end(next) {
            mt_lookup_id(next.ns, next.id, false)
        } else {
            MARKTREE_END_FLAG
        };

        (*z).intersect.clone_from(&(*y).intersect);

        if (*y).level == 0 {
            let pi = pseudo_index(y, 0);
            for j in 0..T_USIZE {
                let k = (*y).key[j];
                let pi_end = pseudo_index_for_id(b, mt_lookup_id(k.ns, k.id, true), true);
                if mt_start(k) && pi_end > pi && mt_lookup_key(k) != last_start {
                    intersect_node(b, z, mt_lookup_id(k.ns, k.id, false));
                }
            }

            for j in (T_USIZE - 1)..((T_USIZE * 2) - 1) {
                let k = (*y).key[j];
                let pi_start = pseudo_index_for_id(b, mt_lookup_id(k.ns, k.id, false), true);
                if mt_end(k) && pi_start > 0 && pi_start < pi {
                    intersect_node(b, y, mt_lookup_id(k.ns, k.id, false));
                }
            }
        }

        std::ptr::copy_nonoverlapping(
            (*y).key.as_ptr().add(T_USIZE),
            (*z).key.as_mut_ptr(),
            T_USIZE - 1,
        );
        for j in 0..(T - 1) {
            refkey(b, z, j);
        }
        if (*y).level != 0 {
            let y_ptr = node_ptr(y);
            let y_meta = node_meta(y);
            let z_ptr = node_ptr(z);
            let z_meta = node_meta(z);
            std::ptr::copy_nonoverlapping(y_ptr.add(T_USIZE), z_ptr, T_USIZE);
            std::ptr::copy_nonoverlapping(y_meta.add(T_USIZE), z_meta, T_USIZE);
            for j in 0..T_USIZE {
                let child = *z_ptr.add(j);
                (*child).parent = z;
                (*child).p_idx = j as i16;
            }
        }
        (*y).n = T - 1;
        let i_usize = i as usize;
        let count_after = ((*x).n - i) as usize;
        std::ptr::copy(x_ptr.add(i_usize + 1), x_ptr.add(i_usize + 2), count_after);
        std::ptr::copy(
            x_meta.add(i_usize + 1),
            x_meta.add(i_usize + 2),
            count_after,
        );
        *x_ptr.add(i_usize + 1) = z;
        meta_describe_node((*x_meta.add(i_usize + 1)).as_mut_ptr(), z);
        (*z).parent = x;
        for j in (i + 1)..((*x).n + 2) {
            (**x_ptr.add(j as usize)).p_idx = j as i16;
        }
        std::ptr::copy(
            (*x).key.as_ptr().add(i_usize),
            (*x).key.as_mut_ptr().add(i_usize + 1),
            count_after,
        );

        // move key to internal layer:
        (*x).key[i_usize] = (*y).key[T_USIZE - 1];
        refkey(b, x, i);
        (*x).n += 1;

        let mut meta_inc = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc.as_mut_ptr(), (*x).key[i_usize]);
        for (m, &inc) in meta_inc.iter().enumerate() {
            (*x_meta.add(i_usize))[m] -= (*x_meta.add(i_usize + 1))[m] + inc;
        }

        let x_key_pos = (*x).key[i_usize].pos;
        for j in 0..(T_USIZE - 1) {
            relative(x_key_pos, &mut (*z).key[j].pos);
        }
        if i > 0 {
            let prev_pos = (*x).key[i_usize - 1].pos;
            unrelative(prev_pos, &mut (*x).key[i_usize].pos);
        }

        if (*y).level != 0 {
            bubble_up(y);
            bubble_up(z);
        }
    }
}

unsafe fn marktree_putp_aux(b: *mut MarkTree, x: *mut MTNode, mut k: MTKey, meta_inc: *const u32) {
    unsafe {
        let mut i = marktree_getp_aux(x, k, None) + 1;
        if (*x).level == 0 {
            if i != (*x).n {
                std::ptr::copy(
                    (*x).key.as_ptr().add(i as usize),
                    (*x).key.as_mut_ptr().add(i as usize + 1),
                    ((*x).n - i) as usize,
                );
            }
            (*x).key[i as usize] = k;
            refkey(b, x, i);
            (*x).n += 1;
        } else {
            let x_ptr = node_ptr(x);
            if (**x_ptr.add(i as usize)).n == 2 * T - 1 {
                split_node(b, x, i, k);
                if key_cmp(k, (*x).key[i as usize]) > 0 {
                    i += 1;
                }
            }
            if i > 0 {
                relative((*x).key[i as usize - 1].pos, &mut k.pos);
            }
            marktree_putp_aux(b, *x_ptr.add(i as usize), k, meta_inc);
            let x_meta = node_meta(x);
            for m in 0..kMTMetaCount {
                (*x_meta.add(i as usize))[m] += *meta_inc.add(m);
            }
        }
    }
}

pub unsafe fn marktree_put(
    b: *mut MarkTree,
    mut key: MTKey,
    end_row: c_int,
    end_col: c_int,
    end_right: bool,
) {
    debug_assert_eq!(
        key.flags & !(MT_FLAG_EXTERNAL_MASK | MT_FLAG_RIGHT_GRAVITY),
        0
    );
    if end_row >= 0 {
        key.flags |= MT_FLAG_PAIRED;
    }

    unsafe {
        marktree_put_key(b, key);

        if end_row >= 0 {
            let mut end_key = key;
            end_key.flags = (key.flags & !MT_FLAG_RIGHT_GRAVITY)
                | MT_FLAG_END
                | if end_right { MT_FLAG_RIGHT_GRAVITY } else { 0 };
            end_key.pos = MTPos {
                row: end_row,
                col: end_col,
            };
            marktree_put_key(b, end_key);
            let mut itr = MarkTreeIter::default();
            let mut end_itr = MarkTreeIter::default();
            marktree_lookup(b, mt_lookup_key(key), &mut itr);
            marktree_lookup(b, mt_lookup_key(end_key), &mut end_itr);

            marktree_intersect_pair(b, mt_lookup_key(key), &mut itr, &mut end_itr, false);
        }
    }
}

fn intersection_has(x: &Intersection, id: u64) -> bool {
    x.binary_search(&id).is_ok()
}

unsafe fn intersect_node(_b: *mut MarkTree, x: *mut MTNode, id: u64) {
    debug_assert_eq!(id & MARKTREE_END_FLAG, 0);
    unsafe {
        let pos = (*x).intersect.partition_point(|&item| item < id);
        (*x).intersect.insert(pos, id);
    }
}

unsafe fn unintersect_node(_b: *mut MarkTree, x: *mut MTNode, id: u64, strict: bool) {
    debug_assert_eq!(id & MARKTREE_END_FLAG, 0);
    unsafe {
        match (*x).intersect.binary_search(&id) {
            Ok(i) => {
                (*x).intersect.remove(i);
            }
            Err(_) => {
                if strict {
                    debug_assert!(false);
                }
            }
        }
    }
}

pub unsafe fn marktree_intersect_pair(
    b: *mut MarkTree,
    id: u64,
    itr: *mut MarkTreeIter,
    end_itr: *mut MarkTreeIter,
    delete: bool,
) {
    unsafe {
        let maxlvl = (*itr).lvl.min((*end_itr).lvl);
        let iat = |it: *const MarkTreeIter, l: c_int, q: c_int| -> c_int {
            if l == (*it).lvl {
                (*it).i + q
            } else {
                (*it).s[l as usize].i
            }
        };
        let mut lvl = 0;
        while lvl < maxlvl {
            let si = (*itr).s[lvl as usize].i;
            let ei = (*end_itr).s[lvl as usize].i;
            if si > ei {
                return;
            } else if si < ei {
                break;
            }
            lvl += 1;
        }
        if lvl == maxlvl && iat(itr, lvl, 1) > iat(end_itr, lvl, 0) {
            return;
        }

        while !(*itr).x.is_null() {
            let mut skip = false;
            if (*itr).x == (*end_itr).x {
                if (*(*itr).x).level == 0 || (*itr).i >= (*end_itr).i {
                    break;
                } else {
                    skip = true;
                }
            } else if (*itr).lvl > lvl || iat(itr, lvl, 1) < iat(end_itr, lvl, 1) {
                skip = true;
            } else {
                lvl += 1;
            }

            if skip && (*(*itr).x).level != 0 {
                let x = *node_ptr((*itr).x).add(((*itr).i + 1) as usize);
                if delete {
                    unintersect_node(b, x, id, true);
                } else {
                    intersect_node(b, x, id);
                }
            }
            marktree_itr_next_skip(b, itr, skip, true, std::ptr::null_mut(), std::ptr::null());
        }
    }
}

unsafe fn marktree_alloc_node(b: *mut MarkTree, internal: bool) -> *mut MTNode {
    unsafe {
        let inner = if internal {
            Some(Box::new(mtnode_inner_s {
                i_ptr: [std::ptr::null_mut(); 2 * MT_BRANCH_FACTOR],
                i_meta: [[0; kMTMetaCount]; 2 * MT_BRANCH_FACTOR],
            }))
        } else {
            None
        };
        let x = Box::into_raw(Box::new(MTNode {
            n: 0,
            level: 0,
            p_idx: 0,
            intersect: Vec::new(),
            parent: std::ptr::null_mut(),
            key: [MT_INVALID_KEY; 2 * MT_BRANCH_FACTOR - 1],
            inner,
        }));
        (*b).n_nodes += 1;
        x
    }
}

#[inline]
unsafe fn meta_describe_key_inc(meta_inc: *mut u32, k: *const MTKey) {
    unsafe {
        let key = *k;
        if !mt_end(key) && !mt_invalid(key) {
            *meta_inc.add(kMTMetaInline) +=
                u32::from((key.flags & MT_FLAG_DECOR_VIRT_TEXT_INLINE) != 0);
            *meta_inc.add(kMTMetaLines) += u32::from((key.flags & MT_FLAG_DECOR_VIRT_LINES) != 0);
            *meta_inc.add(kMTMetaSignHL) += u32::from((key.flags & MT_FLAG_DECOR_SIGNHL) != 0);
            *meta_inc.add(kMTMetaSignText) += u32::from((key.flags & MT_FLAG_DECOR_SIGNTEXT) != 0);
            *meta_inc.add(kMTMetaConcealLines) +=
                u32::from((key.flags & MT_FLAG_DECOR_CONCEAL_LINES) != 0);
        }
    }
}

#[inline]
unsafe fn meta_describe_key(meta_inc: *mut u32, k: MTKey) {
    unsafe {
        std::ptr::write_bytes(meta_inc, 0, kMTMetaCount);
        meta_describe_key_inc(meta_inc, &k);
    }
}

unsafe fn meta_describe_node(meta_node: *mut u32, x: *mut MTNode) {
    unsafe {
        std::ptr::write_bytes(meta_node, 0, kMTMetaCount);
        for i in 0..(*x).n as usize {
            meta_describe_key_inc(meta_node, &(*x).key[i]);
        }
        if (*x).level != 0 {
            let x_meta = node_meta(x);
            for i in 0..((*x).n as usize + 1) {
                for m in 0..kMTMetaCount {
                    *meta_node.add(m) += (*x_meta.add(i))[m];
                }
            }
        }
    }
}

#[inline]
unsafe fn meta_has(meta_count: *const u32, meta_filter: MetaFilter) -> bool {
    unsafe {
        let mut count = 0u32;
        for m in 0..kMTMetaCount {
            count += *meta_count.add(m) & *meta_filter.add(m);
        }
        count > 0
    }
}

pub unsafe fn marktree_put_key(b: *mut MarkTree, mut k: MTKey) {
    unsafe {
        k.flags |= MT_FLAG_REAL;
        if (*b).root.is_null() {
            (*b).root = marktree_alloc_node(b, true);
        }
        let mut r = (*b).root;
        if (*r).n == 2 * T - 1 {
            let s = marktree_alloc_node(b, true);
            (*b).root = s;
            (*s).level = (*r).level + 1;
            (*s).n = 0;
            *node_ptr(s) = r;
            let s_meta0 = &mut *node_meta(s);
            s_meta0[..kMTMetaCount].copy_from_slice(&(*b).meta_root);
            (*r).parent = s;
            (*r).p_idx = 0;
            split_node(b, s, 0, k);
            r = s;
        }

        let mut meta_inc = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc.as_mut_ptr(), k);
        marktree_putp_aux(b, r, k, meta_inc.as_ptr());
        for (m, &inc) in meta_inc.iter().enumerate() {
            (*b).meta_root[m] += inc;
        }
        (*b).n_keys += 1;
    }
}

pub unsafe fn marktree_del_itr(b: *mut MarkTree, itr: *mut MarkTreeIter, rev: bool) -> u64 {
    unsafe {
        let mut adjustment = 0;

        let cur = (*itr).x;
        let curi = (*itr).i;
        let id = mt_lookup_key((*cur).key[curi as usize]);

        let raw = *rawkey(itr);
        let mut other = 0u64;
        if mt_paired(raw) && (raw.flags & MT_FLAG_ORPHANED) == 0 {
            other = mt_lookup_key_side(raw, !mt_end(raw));

            let mut other_itr = MarkTreeIter::default();
            marktree_lookup(b, other, &mut other_itr);
            (*rawkey(&mut other_itr)).flags |= MT_FLAG_ORPHANED;
            if mt_start(raw) {
                let mut this_itr = *itr;
                marktree_intersect_pair(b, id, &mut this_itr, &mut other_itr, true);
            } else {
                marktree_intersect_pair(b, other, &mut other_itr, itr, true);
            }
        }

        // 2.
        if (*(*itr).x).level != 0 {
            if rev {
                std::process::abort();
            } else {
                marktree_itr_prev(b, itr);
                adjustment = -1;
            }
        }

        // 3.
        let mut x = (*itr).x;
        debug_assert_eq!((*x).level, 0);
        let mut intkey = (*x).key[(*itr).i as usize];

        let mut meta_inc = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc.as_mut_ptr(), intkey);
        if (*x).n > (*itr).i + 1 {
            std::ptr::copy(
                (*x).key.as_ptr().add((*itr).i as usize + 1),
                (*x).key.as_mut_ptr().add((*itr).i as usize),
                ((*x).n - (*itr).i - 1) as usize,
            );
        }
        (*x).n -= 1;

        (*b).n_keys -= 1;
        (*b).id2node.remove(&id);

        // 4.
        if adjustment == -1 {
            let mut ilvl = (*itr).lvl - 1;
            let mut lnode = x;
            let mut start_id = 0u64;
            let mut did_bubble = false;
            if mt_end(intkey) {
                start_id = mt_lookup_key_side(intkey, false);
            }
            loop {
                let p = (*lnode).parent;
                if ilvl < 0 {
                    std::process::abort();
                }
                let i = (*itr).s[ilvl as usize].i;
                let p_ptr = node_ptr(p);
                debug_assert_eq!(*p_ptr.add(i as usize), lnode);
                if i > 0 {
                    unrelative((*p).key[i as usize - 1].pos, &mut intkey.pos);
                }

                if p != cur && start_id != 0 && intersection_has(&(**p_ptr).intersect, start_id) {
                    let last = c_int::from(lnode != x);
                    for k in 0..((*p).n + last) {
                        unintersect_node(b, *p_ptr.add(k as usize), start_id, true);
                    }
                    intersect_node(b, p, start_id);
                    did_bubble = true;
                }

                let p_meta = node_meta(p);
                for (m, &inc) in meta_inc.iter().enumerate() {
                    (*p_meta.add((*lnode).p_idx as usize))[m] -= inc;
                }

                lnode = p;
                ilvl -= 1;
                if lnode == cur {
                    break;
                }
            }

            let mut deleted = (*cur).key[curi as usize];
            meta_describe_key(meta_inc.as_mut_ptr(), deleted);
            (*cur).key[curi as usize] = intkey;
            refkey(b, cur, curi);
            if mt_end((*cur).key[curi as usize]) && !did_bubble {
                let pi = pseudo_index(x, 0);
                let pi_start = pseudo_index_for_id(b, start_id, true);
                if pi_start > 0 && pi_start < pi {
                    intersect_node(b, x, start_id);
                }
            }

            relative(intkey.pos, &mut deleted.pos);
            let mut y = *node_ptr(cur).add(curi as usize + 1);
            if deleted.pos.row != 0 || deleted.pos.col != 0 {
                while !y.is_null() {
                    for k in 0..(*y).n as usize {
                        unrelative(deleted.pos, &mut (*y).key[k].pos);
                    }
                    y = if (*y).level != 0 {
                        *node_ptr(y)
                    } else {
                        std::ptr::null_mut()
                    };
                }
            }
            (*itr).i -= 1;
        }

        let mut lnode = cur;
        while !(*lnode).parent.is_null() {
            let p_meta = node_meta((*lnode).parent).add((*lnode).p_idx as usize);
            for (m, &inc) in meta_inc.iter().enumerate() {
                (*p_meta)[m] -= inc;
            }
            lnode = (*lnode).parent;
        }
        for (m, &inc) in meta_inc.iter().enumerate() {
            debug_assert!((*b).meta_root[m] >= inc);
            (*b).meta_root[m] -= inc;
        }

        // 5.
        let mut itr_dirty = false;
        let mut rlvl = (*itr).lvl - 1;
        let mut lasti_is_itr_i = true;
        let mut lasti_lvl: usize = 0;
        let mut ppos = (*itr).pos;
        while x != (*b).root {
            debug_assert!(rlvl >= 0);
            let p = (*x).parent;
            if (*x).n >= T - 1 {
                break;
            }
            let pi = (*itr).s[rlvl as usize].i;
            let p_ptr = node_ptr(p);
            debug_assert_eq!(*p_ptr.add(pi as usize), x);
            if pi > 0 {
                ppos.row -= (*p).key[pi as usize - 1].pos.row;
                ppos.col = (*itr).s[rlvl as usize].oldcol;
            }

            if pi > 0 && (**p_ptr.add(pi as usize - 1)).n > T - 1 {
                if lasti_is_itr_i {
                    (*itr).i += 1;
                } else {
                    (*itr).s[lasti_lvl].i += 1;
                }
                itr_dirty = true;
                pivot_right(b, ppos, p, pi - 1);
                break;
            } else if pi < (*p).n && (**p_ptr.add(pi as usize + 1)).n > T - 1 {
                pivot_left(b, ppos, p, pi);
                break;
            } else if pi > 0 {
                debug_assert_eq!((**p_ptr.add(pi as usize - 1)).n, T - 1);
                if lasti_is_itr_i {
                    (*itr).i += T;
                } else {
                    (*itr).s[lasti_lvl].i += T;
                }
                x = merge_node(b, p, pi - 1);
                if lasti_is_itr_i {
                    (*itr).x = x;
                }
                (*itr).s[rlvl as usize].i -= 1;
                itr_dirty = true;
            } else {
                debug_assert!(pi < (*p).n && (**p_ptr.add(pi as usize + 1)).n == T - 1);
                merge_node(b, p, pi);
            }
            lasti_is_itr_i = false;
            lasti_lvl = rlvl as usize;
            rlvl -= 1;
            x = p;
        }

        // 6.
        if (*(*b).root).n == 0 {
            if (*itr).lvl > 0 {
                std::ptr::copy(
                    (*itr).s.as_ptr().add(1),
                    (*itr).s.as_mut_ptr(),
                    ((*itr).lvl - 1) as usize,
                );
                (*itr).lvl -= 1;
            }
            if (*(*b).root).level != 0 {
                let oldroot = (*b).root;
                (*b).root = *node_ptr(oldroot);
                let oldroot_meta = *node_meta(oldroot);
                for (m, &meta_val) in oldroot_meta.iter().enumerate() {
                    debug_assert_eq!((*b).meta_root[m], meta_val);
                }

                (*(*b).root).parent = std::ptr::null_mut();
                marktree_free_node(b, oldroot);
            } else {
                (*itr).x = std::ptr::null_mut();
            }
        }

        if !(*itr).x.is_null() && itr_dirty {
            marktree_itr_fix_pos(b, itr);
        }

        if adjustment == -1 {
            marktree_itr_next(b, itr);
            marktree_itr_next(b, itr);
        } else if !(*itr).x.is_null() && (*itr).i >= (*(*itr).x).n {
            debug_assert_eq!((*(*itr).x).level, 0);
            marktree_itr_next(b, itr);
        }

        other
    }
}

pub unsafe fn marktree_revise_meta(b: *mut MarkTree, itr: *mut MarkTreeIter, old_key: MTKey) {
    unsafe {
        let mut meta_old = [0u32; kMTMetaCount];
        let mut meta_new = [0u32; kMTMetaCount];
        meta_describe_key(meta_old.as_mut_ptr(), old_key);
        meta_describe_key(meta_new.as_mut_ptr(), *rawkey(itr));

        if meta_old == meta_new {
            return;
        }

        let mut lnode = (*itr).x;
        while !(*lnode).parent.is_null() {
            let meta_p = node_meta((*lnode).parent).add((*lnode).p_idx as usize);
            for m in 0..kMTMetaCount {
                (*meta_p)[m] = (*meta_p)[m]
                    .wrapping_add(meta_new[m])
                    .wrapping_sub(meta_old[m]);
            }
            lnode = (*lnode).parent;
        }

        for m in 0..kMTMetaCount {
            (*b).meta_root[m] = (*b).meta_root[m]
                .wrapping_add(meta_new[m])
                .wrapping_sub(meta_old[m]);
        }
    }
}

fn intersect_merge(m: &mut Intersection, x: &mut Intersection, y: &mut Intersection) {
    let mut xi = 0usize;
    let mut yi = 0usize;
    let mut xn = 0usize;
    let mut yn = 0usize;
    while xi < x.len() && yi < y.len() {
        let xv = x[xi];
        let yv = y[yi];
        if xv == yv {
            m.push(xv);
            xi += 1;
            yi += 1;
        } else if xv < yv {
            x[xn] = xv;
            xn += 1;
            xi += 1;
        } else {
            y[yn] = yv;
            yn += 1;
            yi += 1;
        }
    }

    if xi < x.len() {
        let rem = x.len() - xi;
        x.copy_within(xi.., xn);
        xn += rem;
    }
    if yi < y.len() {
        let rem = y.len() - yi;
        y.copy_within(yi.., yn);
        yn += rem;
    }

    x.truncate(xn);
    y.truncate(yn);
}

fn intersect_mov(
    x: &Intersection,
    y: &mut Intersection,
    w: &mut Intersection,
    d: &mut Intersection,
) {
    let mut wi = 0usize;
    let mut yi = 0usize;
    let mut wn = 0usize;
    let mut yn = 0usize;
    let mut xi = 0usize;
    while wi < w.len() || xi < x.len() {
        if wi < w.len() && (xi >= x.len() || x[xi] >= w[wi]) {
            let wv = w[wi];
            if xi < x.len() && x[xi] == wv {
                xi += 1;
            }
            while yi < y.len() && y[yi] < wv {
                d.push(y[yi]);
                yi += 1;
            }
            if yi < y.len() && y[yi] == wv {
                y[yn] = y[yi];
                yn += 1;
                yi += 1;
                wi += 1;
            } else {
                w[wn] = wv;
                wn += 1;
                wi += 1;
            }
        } else {
            let xv = x[xi];
            while yi < y.len() && y[yi] < xv {
                d.push(y[yi]);
                yi += 1;
            }
            if yi < y.len() && y[yi] == xv {
                y[yn] = y[yi];
                yn += 1;
                yi += 1;
                xi += 1;
            } else {
                if wi == wn {
                    w.insert(wn, xv);
                    wn += 1;
                    wi += 1;
                } else {
                    debug_assert!(wn < wi);
                    w[wn] = xv;
                    wn += 1;
                }
                xi += 1;
            }
        }
    }
    if yi < y.len() {
        d.extend_from_slice(&y[yi..]);
    }
    w.truncate(wn);
    y.truncate(yn);
}

#[cfg(test)]
pub unsafe fn intersect_mov_test(
    x: *const u64,
    nx: usize,
    y: *const u64,
    ny: usize,
    win: *const u64,
    nwin: usize,
    wout: *mut u64,
    nwout: *mut usize,
    dout: *mut u64,
    ndout: *mut usize,
) -> bool {
    unsafe {
        let xi = if nx == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(x, nx).to_vec()
        };
        let mut yi = if ny == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(y, ny).to_vec()
        };
        let mut w = if nwin == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(win, nwin).to_vec()
        };
        let mut d = Vec::new();

        intersect_mov(&xi, &mut yi, &mut w, &mut d);

        if w.len() > *nwout || d.len() > *ndout {
            return false;
        }

        std::ptr::copy_nonoverlapping(w.as_ptr(), wout, w.len());
        *nwout = w.len();

        std::ptr::copy_nonoverlapping(d.as_ptr(), dout, d.len());
        *ndout = d.len();

        true
    }
}

fn intersect_common(i: &mut Intersection, x: &Intersection, y: &Intersection) {
    let mut xi = 0usize;
    let mut yi = 0usize;
    while xi < x.len() && yi < y.len() {
        let xv = x[xi];
        let yv = y[yi];
        if xv == yv {
            i.push(xv);
            xi += 1;
            yi += 1;
        } else if xv < yv {
            xi += 1;
        } else {
            yi += 1;
        }
    }
}

fn intersect_add(x: &mut Intersection, y: &Intersection) {
    let mut xi = 0usize;
    let mut yi = 0usize;
    while xi < x.len() && yi < y.len() {
        let xv = x[xi];
        let yv = y[yi];
        if xv == yv {
            xi += 1;
            yi += 1;
        } else if yv < xv {
            x.insert(xi, yv);
            xi += 1;
            yi += 1;
        } else {
            xi += 1;
        }
    }
    if yi < y.len() {
        x.extend_from_slice(&y[yi..]);
    }
}

fn intersect_sub(x: &mut Intersection, y: &Intersection) {
    let mut xi = 0usize;
    let mut yi = 0usize;
    let mut xn = 0usize;
    while xi < x.len() && yi < y.len() {
        let xv = x[xi];
        let yv = y[yi];
        if xv == yv {
            xi += 1;
            yi += 1;
        } else if xv < yv {
            x[xn] = xv;
            xn += 1;
            xi += 1;
        } else {
            yi += 1;
        }
    }
    if xi < x.len() {
        let n = x.len() - xi;
        if xn < xi {
            x.copy_within(xi.., xn);
        }
        xn += n;
    }
    x.truncate(xn);
}

unsafe fn bubble_up(x: *mut MTNode) {
    unsafe {
        let mut xi = Vec::new();
        let x_ptr = node_ptr(x);
        intersect_common(
            &mut xi,
            &(**x_ptr).intersect,
            &(**x_ptr.add((*x).n as usize)).intersect,
        );
        if !xi.is_empty() {
            for i in 0..((*x).n as usize + 1) {
                intersect_sub(&mut (**x_ptr.add(i)).intersect, &xi);
            }
            intersect_add(&mut (*x).intersect, &xi);
        }
    }
}

unsafe fn merge_node(b: *mut MarkTree, p: *mut MTNode, i: c_int) -> *mut MTNode {
    unsafe {
        let i_usize = i as usize;
        let p_ptr = node_ptr(p);
        let p_meta = node_meta(p);
        let x = *p_ptr.add(i_usize);
        let y = *p_ptr.add(i_usize + 1);
        let mut mi = Vec::new();

        intersect_merge(&mut mi, &mut (*x).intersect, &mut (*y).intersect);

        let xn = (*x).n as usize;
        let yn = (*y).n as usize;
        (*x).key[xn] = (*p).key[i_usize];
        refkey(b, x, (*x).n);
        if i > 0 {
            relative((*p).key[i_usize - 1].pos, &mut (*x).key[xn].pos);
        }

        let mut meta_inc = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc.as_mut_ptr(), (*x).key[xn]);

        std::ptr::copy((*y).key.as_ptr(), (*x).key.as_mut_ptr().add(xn + 1), yn);
        let mid_pos = (*x).key[xn].pos;
        for k in 0..(*y).n {
            refkey(b, x, (*x).n + 1 + k);
            unrelative(mid_pos, &mut (*x).key[xn + 1 + k as usize].pos);
        }
        if (*x).level != 0 {
            let x_ptr = node_ptr(x);
            let x_meta = node_meta(x);
            let y_ptr = node_ptr(y);
            let y_meta = node_meta(y);
            std::ptr::copy(y_ptr, x_ptr.add(xn + 1), yn + 1);
            std::ptr::copy(y_meta, x_meta.add(xn + 1), yn + 1);
            for k in 0..(xn + 1) {
                for &id in &(*x).intersect {
                    intersect_node(b, *x_ptr.add(k), id);
                }
            }
            for ky in 0..(yn + 1) {
                let k = xn + ky + 1;
                let child = *x_ptr.add(k);
                (*child).parent = x;
                (*child).p_idx = k as i16;
                for &id in &(*y).intersect {
                    intersect_node(b, child, id);
                }
            }
        }
        (*x).n += (*y).n + 1;
        for (m, &inc) in meta_inc.iter().enumerate() {
            (*p_meta.add(i_usize))[m] += (*p_meta.add(i_usize + 1))[m] + inc;
        }

        let rem = ((*p).n - i - 1) as usize;
        std::ptr::copy(
            (*p).key.as_ptr().add(i_usize + 1),
            (*p).key.as_mut_ptr().add(i_usize),
            rem,
        );
        std::ptr::copy(p_ptr.add(i_usize + 2), p_ptr.add(i_usize + 1), rem);
        std::ptr::copy(p_meta.add(i_usize + 2), p_meta.add(i_usize + 1), rem);
        for j in (i + 1)..(*p).n {
            (**p_ptr.add(j as usize)).p_idx = j as i16;
        }
        (*p).n -= 1;
        marktree_free_node(b, y);

        (*x).intersect = mi;

        x
    }
}

unsafe fn pivot_right(b: *mut MarkTree, _p_pos: MTPos, p: *mut MTNode, i: c_int) {
    unsafe {
        let i_usize = i as usize;
        let p_ptr = node_ptr(p);
        let p_meta = node_meta(p);
        let x = *p_ptr.add(i_usize);
        let y = *p_ptr.add(i_usize + 1);
        let yn = (*y).n as usize;
        std::ptr::copy((*y).key.as_ptr(), (*y).key.as_mut_ptr().add(1), yn);
        if (*y).level != 0 {
            let y_ptr = node_ptr(y);
            let y_meta = node_meta(y);
            std::ptr::copy(y_ptr, y_ptr.add(1), yn + 1);
            std::ptr::copy(y_meta, y_meta.add(1), yn + 1);
            for j in 1..((*y).n + 2) {
                (**y_ptr.add(j as usize)).p_idx = j as i16;
            }
        }

        (*y).key[0] = (*p).key[i_usize];
        refkey(b, y, 0);
        (*p).key[i_usize] = (*x).key[(*x).n as usize - 1];
        refkey(b, p, i);

        let mut meta_inc_y = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc_y.as_mut_ptr(), (*y).key[0]);
        let mut meta_inc_x = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc_x.as_mut_ptr(), (*p).key[i_usize]);

        for m in 0..kMTMetaCount {
            (*p_meta.add(i_usize + 1))[m] += meta_inc_y[m];
            (*p_meta.add(i_usize))[m] -= meta_inc_x[m];
        }

        if (*x).level != 0 {
            let x_ptr = node_ptr(x);
            let x_meta = node_meta(x);
            let y_ptr = node_ptr(y);
            let y_meta = node_meta(y);
            *y_ptr = *x_ptr.add((*x).n as usize);
            *y_meta = *x_meta.add((*x).n as usize);
            for m in 0..kMTMetaCount {
                (*p_meta.add(i_usize + 1))[m] += (*y_meta)[m];
                (*p_meta.add(i_usize))[m] -= (*y_meta)[m];
            }
            (**y_ptr).parent = y;
            (**y_ptr).p_idx = 0;
        }
        (*x).n -= 1;
        (*y).n += 1;
        if i > 0 {
            let prev_pos = (*p).key[i_usize - 1].pos;
            unrelative(prev_pos, &mut (*p).key[i_usize].pos);
        }
        let p_key_pos = (*p).key[i_usize].pos;
        relative(p_key_pos, &mut (*y).key[0].pos);
        let y0_pos = (*y).key[0].pos;
        for k in 1..(*y).n as usize {
            unrelative(y0_pos, &mut (*y).key[k].pos);
        }

        if (*x).level != 0 {
            let mut d = Vec::new();
            let y_ptr = node_ptr(y);
            intersect_mov(
                &(*x).intersect,
                &mut (*y).intersect,
                &mut (**y_ptr).intersect,
                &mut d,
            );
            if !d.is_empty() {
                for yi in 1..((*y).n as usize + 1) {
                    intersect_add(&mut (**y_ptr.add(yi)).intersect, &d);
                }
            }

            bubble_up(x);
        } else {
            if mt_end((*p).key[i_usize]) {
                let pi = pseudo_index(x, 0);
                let start_id = mt_lookup_key_side((*p).key[i_usize], false);
                let pi_start = pseudo_index_for_id(b, start_id, true);
                if pi_start > 0 && pi_start < pi {
                    intersect_node(b, x, start_id);
                }
            }

            if mt_start((*y).key[0]) {
                unintersect_node(b, y, mt_lookup_key((*y).key[0]), false);
            }
        }
    }
}

unsafe fn pivot_left(b: *mut MarkTree, _p_pos: MTPos, p: *mut MTNode, i: c_int) {
    unsafe {
        let i_usize = i as usize;
        let p_ptr = node_ptr(p);
        let p_meta = node_meta(p);
        let x = *p_ptr.add(i_usize);
        let y = *p_ptr.add(i_usize + 1);

        let y0_pos = (*y).key[0].pos;
        for k in 1..(*y).n as usize {
            relative(y0_pos, &mut (*y).key[k].pos);
        }
        unrelative((*p).key[i_usize].pos, &mut (*y).key[0].pos);
        if i > 0 {
            let prev_pos = (*p).key[i_usize - 1].pos;
            relative(prev_pos, &mut (*p).key[i_usize].pos);
        }

        let xn = (*x).n as usize;
        let yn = (*y).n as usize;
        (*x).key[xn] = (*p).key[i_usize];
        refkey(b, x, (*x).n);
        (*p).key[i_usize] = (*y).key[0];
        refkey(b, p, i);

        let mut meta_inc_x = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc_x.as_mut_ptr(), (*x).key[xn]);
        let mut meta_inc_y = [0u32; kMTMetaCount];
        meta_describe_key(meta_inc_y.as_mut_ptr(), (*p).key[i_usize]);
        for m in 0..kMTMetaCount {
            (*p_meta.add(i_usize))[m] += meta_inc_x[m];
            (*p_meta.add(i_usize + 1))[m] -= meta_inc_y[m];
        }

        if (*x).level != 0 {
            let x_ptr = node_ptr(x);
            let x_meta = node_meta(x);
            let y_ptr = node_ptr(y);
            let y_meta = node_meta(y);
            *x_ptr.add(xn + 1) = *y_ptr;
            *x_meta.add(xn + 1) = *y_meta;
            for m in 0..kMTMetaCount {
                (*p_meta.add(i_usize + 1))[m] -= (*y_meta)[m];
                (*p_meta.add(i_usize))[m] += (*y_meta)[m];
            }
            (**x_ptr.add(xn + 1)).parent = x;
            (**x_ptr.add(xn + 1)).p_idx = (xn + 1) as i16;
        }
        std::ptr::copy((*y).key.as_ptr().add(1), (*y).key.as_mut_ptr(), yn - 1);
        if (*y).level != 0 {
            let y_ptr = node_ptr(y);
            let y_meta = node_meta(y);
            std::ptr::copy(y_ptr.add(1), y_ptr, yn);
            std::ptr::copy(y_meta.add(1), y_meta, yn);
            for j in 0..yn {
                (**y_ptr.add(j)).p_idx = j as i16;
            }
        }
        (*x).n += 1;
        (*y).n -= 1;

        if (*x).level != 0 {
            let mut d = Vec::new();
            let x_ptr = node_ptr(x);
            intersect_mov(
                &(*y).intersect,
                &mut (*x).intersect,
                &mut (**x_ptr.add((*x).n as usize)).intersect,
                &mut d,
            );
            if !d.is_empty() {
                for xi in 0..(*x).n as usize {
                    intersect_add(&mut (**x_ptr.add(xi)).intersect, &d);
                }
            }

            bubble_up(y);
        } else {
            if mt_start((*p).key[i_usize]) {
                let pi = pseudo_index(y, 0);
                let end_id = mt_lookup_key_side((*p).key[i_usize], true);
                let pi_end = pseudo_index_for_id(b, end_id, true);
                if pi_end > pi {
                    intersect_node(b, y, mt_lookup_key((*p).key[i_usize]));
                }
            }

            if mt_end((*x).key[(*x).n as usize - 1]) {
                unintersect_node(
                    b,
                    x,
                    mt_lookup_key_side((*x).key[(*x).n as usize - 1], false),
                    false,
                );
            }
        }
    }
}

pub unsafe fn marktree_clear(b: *mut MarkTree) {
    unsafe {
        if !(*b).root.is_null() {
            marktree_free_subtree(b, (*b).root);
            (*b).root = std::ptr::null_mut();
        }
        (*b).id2node.clear();
        (*b).n_keys = 0;
        (*b).meta_root = [0; kMTMetaCount];
        debug_assert_eq!((*b).n_nodes, 0);
    }
}

pub unsafe fn marktree_free_subtree(b: *mut MarkTree, x: *mut MTNode) {
    unsafe {
        if (*x).level != 0 {
            let x_ptr = node_ptr(x);
            for i in 0..((*x).n as usize + 1) {
                marktree_free_subtree(b, *x_ptr.add(i));
            }
        }
        marktree_free_node(b, x);
    }
}

unsafe fn marktree_free_node(b: *mut MarkTree, x: *mut MTNode) {
    unsafe {
        drop(Box::from_raw(x));
        (*b).n_nodes -= 1;
    }
}

pub unsafe fn marktree_move(b: *mut MarkTree, itr: *mut MarkTreeIter, row: c_int, col: c_int) {
    unsafe {
        let mut key = *rawkey(itr);
        let x = (*itr).x;
        if (*x).level == 0 {
            let mut internal = false;
            let mut newpos = MTPos { row, col };
            if !(*x).parent.is_null() {
                if pos_less((*itr).pos, newpos) {
                    relative((*itr).pos, &mut newpos);
                    if pos_less(newpos, (*x).key[(*x).n as usize - 1].pos) {
                        internal = true;
                    }
                }
            } else {
                internal = true;
            }

            if internal {
                if key.pos.row == newpos.row && key.pos.col == newpos.col {
                    return;
                }
                key.pos = newpos;
                let mut matched = false;
                let mut new_i = marktree_getp_aux(x, key, Some(&mut matched));
                if !matched {
                    new_i += 1;
                }
                if new_i == (*itr).i {
                    (*x).key[(*itr).i as usize].pos = newpos;
                } else if new_i < (*itr).i {
                    std::ptr::copy(
                        (*x).key.as_ptr().add(new_i as usize),
                        (*x).key.as_mut_ptr().add(new_i as usize + 1),
                        ((*itr).i - new_i) as usize,
                    );
                    (*x).key[new_i as usize] = key;
                } else {
                    std::ptr::copy(
                        (*x).key.as_ptr().add((*itr).i as usize + 1),
                        (*x).key.as_mut_ptr().add((*itr).i as usize),
                        (new_i - (*itr).i - 1) as usize,
                    );
                    (*x).key[new_i as usize - 1] = key;
                }
                return;
            }
        }
        let other = marktree_del_itr(b, itr, false);
        key.pos = MTPos { row, col };

        marktree_put_key(b, key);

        if other != 0 {
            marktree_restore_pair(b, key);
        }
        (*itr).x = std::ptr::null_mut();
    }
}

pub unsafe fn marktree_restore_pair(b: *mut MarkTree, key: MTKey) {
    unsafe {
        let mut itr = MarkTreeIter::default();
        let mut end_itr = MarkTreeIter::default();
        marktree_lookup(b, mt_lookup_key_side(key, false), &mut itr);
        marktree_lookup(b, mt_lookup_key_side(key, true), &mut end_itr);
        if itr.x.is_null() || end_itr.x.is_null() {
            return;
        }
        (*rawkey(&mut itr)).flags &= !MT_FLAG_ORPHANED;
        (*rawkey(&mut end_itr)).flags &= !MT_FLAG_ORPHANED;

        marktree_intersect_pair(
            b,
            mt_lookup_key_side(key, false),
            &mut itr,
            &mut end_itr,
            false,
        );
    }
}

pub unsafe fn marktree_itr_get(
    b: *mut MarkTree,
    row: i32,
    col: c_int,
    itr: *mut MarkTreeIter,
) -> bool {
    unsafe {
        marktree_itr_get_ext(
            b,
            MTPos { row, col },
            itr,
            false,
            false,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    }
}

pub unsafe fn marktree_itr_get_ext(
    b: *mut MarkTree,
    p: MTPos,
    itr: *mut MarkTreeIter,
    last: bool,
    gravity: bool,
    oldbase: *mut MTPos,
    meta_filter: MetaFilter,
) -> bool {
    unsafe {
        if (*b).n_keys == 0 {
            (*itr).x = std::ptr::null_mut();
            return false;
        }

        let mut k = MTKey {
            pos: p,
            ns: 0,
            id: 0,
            flags: if gravity {
                MT_FLAG_RIGHT_GRAVITY
            } else if last {
                MT_FLAG_LAST
            } else {
                0
            },
            decor_data: DecorInlineData {
                hl: DECOR_HIGHLIGHT_INLINE_INIT,
            },
        };
        (*itr).pos = MTPos { row: 0, col: 0 };
        (*itr).x = (*b).root;
        (*itr).lvl = 0;
        if !oldbase.is_null() {
            *oldbase.add((*itr).lvl as usize) = (*itr).pos;
        }
        loop {
            (*itr).i = marktree_getp_aux((*itr).x, k, None) + 1;

            if (*(*itr).x).level == 0 {
                break;
            }
            if !meta_filter.is_null() {
                let x_meta = node_meta((*itr).x);
                if !meta_has((*x_meta.add((*itr).i as usize)).as_ptr(), meta_filter) {
                    break;
                }
            }

            let lvl = (*itr).lvl as usize;
            (*itr).s[lvl].i = (*itr).i;
            (*itr).s[lvl].oldcol = (*itr).pos.col;

            if (*itr).i > 0 {
                let prev_pos = (*(*itr).x).key[(*itr).i as usize - 1].pos;
                compose(&mut (*itr).pos, prev_pos);
                relative(prev_pos, &mut k.pos);
            }
            (*itr).x = *node_ptr((*itr).x).add((*itr).i as usize);
            (*itr).lvl += 1;
            if !oldbase.is_null() {
                *oldbase.add((*itr).lvl as usize) = (*itr).pos;
            }
        }

        if last {
            marktree_itr_prev(b, itr)
        } else if (*itr).i >= (*(*itr).x).n {
            marktree_itr_next_skip(b, itr, true, false, std::ptr::null_mut(), std::ptr::null())
        } else {
            true
        }
    }
}

#[cfg(test)]
pub unsafe fn marktree_itr_first(b: *mut MarkTree, itr: *mut MarkTreeIter) -> bool {
    unsafe {
        if (*b).n_keys == 0 {
            (*itr).x = std::ptr::null_mut();
            return false;
        }

        (*itr).x = (*b).root;
        (*itr).i = 0;
        (*itr).lvl = 0;
        (*itr).pos = MTPos { row: 0, col: 0 };
        while (*(*itr).x).level > 0 {
            let lvl = (*itr).lvl as usize;
            (*itr).s[lvl].i = 0;
            (*itr).s[lvl].oldcol = 0;
            (*itr).lvl += 1;
            (*itr).x = *node_ptr((*itr).x);
        }
        true
    }
}

pub unsafe fn marktree_itr_next(b: *mut MarkTree, itr: *mut MarkTreeIter) -> bool {
    unsafe { marktree_itr_next_skip(b, itr, false, false, std::ptr::null_mut(), std::ptr::null()) }
}

unsafe fn marktree_itr_next_skip(
    _b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    mut skip: bool,
    preload: bool,
    oldbase: *mut MTPos,
    meta_filter: MetaFilter,
) -> bool {
    unsafe {
        if (*itr).x.is_null() {
            return false;
        }
        (*itr).i += 1;
        if !meta_filter.is_null() && (*(*itr).x).level > 0 {
            let x_meta = node_meta((*itr).x);
            if !meta_has((*x_meta.add((*itr).i as usize)).as_ptr(), meta_filter) {
                skip = true;
            }
        }
        if (*(*itr).x).level == 0 || skip {
            if preload && (*(*itr).x).level == 0 && skip {
                (*itr).i = (*(*itr).x).n;
            } else if (*itr).i < (*(*itr).x).n {
                return true;
            }
            while (*itr).i >= (*(*itr).x).n {
                (*itr).x = (*(*itr).x).parent;
                if (*itr).x.is_null() {
                    return false;
                }
                (*itr).lvl -= 1;
                let lvl = (*itr).lvl as usize;
                (*itr).i = (*itr).s[lvl].i;
                if (*itr).i > 0 {
                    (*itr).pos.row -= (*(*itr).x).key[(*itr).i as usize - 1].pos.row;
                    (*itr).pos.col = (*itr).s[lvl].oldcol;
                }
            }
        } else {
            while (*(*itr).x).level > 0 {
                let lvl = (*itr).lvl as usize;
                if (*itr).i > 0 {
                    (*itr).s[lvl].oldcol = (*itr).pos.col;
                    compose(&mut (*itr).pos, (*(*itr).x).key[(*itr).i as usize - 1].pos);
                }
                if !oldbase.is_null() && (*itr).i == 0 {
                    *oldbase.add(lvl + 1) = *oldbase.add(lvl);
                }
                (*itr).s[lvl].i = (*itr).i;
                let child = *node_ptr((*itr).x).add((*itr).i as usize);
                debug_assert_eq!((*child).parent, (*itr).x);
                (*itr).lvl += 1;
                (*itr).x = child;
                if preload && (*(*itr).x).level != 0 {
                    (*itr).i = -1;
                    break;
                }
                (*itr).i = 0;
                if !meta_filter.is_null() && (*(*itr).x).level != 0 {
                    let x_meta = node_meta((*itr).x);
                    if !meta_has((*x_meta).as_ptr(), meta_filter) {
                        break;
                    }
                }
            }
        }
        true
    }
}

pub unsafe fn marktree_itr_get_filter(
    b: *mut MarkTree,
    row: i32,
    col: c_int,
    stop_row: c_int,
    stop_col: c_int,
    meta_filter: MetaFilter,
    itr: *mut MarkTreeIter,
) -> bool {
    unsafe {
        if !meta_has((*b).meta_root.as_ptr(), meta_filter) {
            return false;
        }
        if !marktree_itr_get_ext(
            b,
            MTPos { row, col },
            itr,
            false,
            false,
            std::ptr::null_mut(),
            meta_filter,
        ) {
            return false;
        }
        marktree_itr_check_filter(b, itr, stop_row, stop_col, meta_filter)
    }
}

pub unsafe fn marktree_itr_step_out_filter(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    meta_filter: MetaFilter,
) -> bool {
    unsafe {
        if !meta_has((*b).meta_root.as_ptr(), meta_filter) {
            (*itr).x = std::ptr::null_mut();
            return false;
        }

        while !(*itr).x.is_null() && !(*(*itr).x).parent.is_null() {
            let parent = (*(*itr).x).parent;
            let p_idx = (*(*itr).x).p_idx as usize;
            if meta_has((*node_meta(parent).add(p_idx)).as_ptr(), meta_filter) {
                return true;
            }

            (*itr).i = (*(*itr).x).n;
            marktree_itr_next_skip(b, itr, true, false, std::ptr::null_mut(), std::ptr::null());
        }

        !(*itr).x.is_null()
    }
}

pub unsafe fn marktree_itr_next_filter(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    stop_row: c_int,
    stop_col: c_int,
    meta_filter: MetaFilter,
) -> bool {
    unsafe {
        if !marktree_itr_next_skip(b, itr, false, false, std::ptr::null_mut(), meta_filter) {
            return false;
        }
        marktree_itr_check_filter(b, itr, stop_row, stop_col, meta_filter)
    }
}

unsafe fn marktree_itr_check_filter(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    stop_row: c_int,
    stop_col: c_int,
    meta_filter: MetaFilter,
) -> bool {
    unsafe {
        let stop_pos = MTPos {
            row: stop_row,
            col: stop_col,
        };

        let mut key_filter = 0u32;
        for (m, &mm) in META_MAP.iter().enumerate() {
            key_filter |= mm & *meta_filter.add(m);
        }

        loop {
            if pos_leq(stop_pos, marktree_itr_pos(itr)) {
                (*itr).x = std::ptr::null_mut();
                return false;
            }

            let k = *rawkey(itr);
            if !mt_end(k) && (u32::from(k.flags) & key_filter) != 0 {
                return true;
            }

            if !marktree_itr_next_skip(b, itr, false, false, std::ptr::null_mut(), meta_filter) {
                return false;
            }
        }
    }
}

pub unsafe fn marktree_itr_prev(_b: *mut MarkTree, itr: *mut MarkTreeIter) -> bool {
    unsafe {
        if (*itr).x.is_null() {
            return false;
        }
        if (*(*itr).x).level == 0 {
            (*itr).i -= 1;
            if (*itr).i >= 0 {
                return true;
            }
            while (*itr).i < 0 {
                (*itr).x = (*(*itr).x).parent;
                if (*itr).x.is_null() {
                    return false;
                }
                (*itr).lvl -= 1;
                let lvl = (*itr).lvl as usize;
                (*itr).i = (*itr).s[lvl].i - 1;
                if (*itr).i >= 0 {
                    (*itr).pos.row -= (*(*itr).x).key[(*itr).i as usize].pos.row;
                    (*itr).pos.col = (*itr).s[lvl].oldcol;
                }
            }
        } else {
            while (*(*itr).x).level > 0 {
                let lvl = (*itr).lvl as usize;
                if (*itr).i > 0 {
                    (*itr).s[lvl].oldcol = (*itr).pos.col;
                    compose(&mut (*itr).pos, (*(*itr).x).key[(*itr).i as usize - 1].pos);
                }
                (*itr).s[lvl].i = (*itr).i;
                let child = *node_ptr((*itr).x).add((*itr).i as usize);
                debug_assert_eq!((*child).parent, (*itr).x);
                (*itr).x = child;
                (*itr).i = (*(*itr).x).n;
                (*itr).lvl += 1;
            }
            (*itr).i -= 1;
        }
        true
    }
}

pub unsafe fn marktree_itr_pos(itr: *mut MarkTreeIter) -> MTPos {
    unsafe {
        let mut pos = (*rawkey(itr)).pos;
        unrelative((*itr).pos, &mut pos);
        pos
    }
}

pub unsafe fn marktree_itr_current(itr: *mut MarkTreeIter) -> MTKey {
    unsafe {
        if !(*itr).x.is_null() {
            let mut key = *rawkey(itr);
            key.pos = marktree_itr_pos(itr);
            key
        } else {
            MT_INVALID_KEY
        }
    }
}

#[inline]
unsafe fn itr_eq(itr1: *mut MarkTreeIter, itr2: *mut MarkTreeIter) -> bool {
    unsafe { std::ptr::eq(rawkey(itr1), rawkey(itr2)) }
}

pub unsafe fn marktree_itr_get_overlap(
    b: *mut MarkTree,
    row: c_int,
    col: c_int,
    itr: *mut MarkTreeIter,
) -> bool {
    unsafe {
        if (*b).n_keys == 0 {
            (*itr).x = std::ptr::null_mut();
            return false;
        }

        (*itr).x = (*b).root;
        (*itr).i = -1;
        (*itr).lvl = 0;
        (*itr).pos = MTPos { row: 0, col: 0 };
        (*itr).intersect_pos = MTPos { row, col };
        (*itr).intersect_pos_x = MTPos { row, col };
        (*itr).intersect_idx = 0;
        true
    }
}

pub unsafe fn marktree_itr_step_overlap(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    pair: *mut MTPair,
) -> bool {
    unsafe {
        while (*itr).i == -1 {
            if (*itr).intersect_idx < (*(*itr).x).intersect.len() {
                let id = (*(*itr).x).intersect.as_slice()[(*itr).intersect_idx];
                (*itr).intersect_idx += 1;
                *pair = mtpair_from(
                    marktree_lookup(b, id, std::ptr::null_mut()),
                    marktree_lookup(b, id | MARKTREE_END_FLAG, std::ptr::null_mut()),
                );
                return true;
            }

            if (*(*itr).x).level == 0 {
                (*itr).i = 0;
                (*itr).s[(*itr).lvl as usize].i = 0;
                break;
            }

            let k = MTKey {
                pos: (*itr).intersect_pos_x,
                ns: 0,
                id: 0,
                flags: 0,
                decor_data: DecorInlineData {
                    hl: DECOR_HIGHLIGHT_INLINE_INIT,
                },
            };
            (*itr).i = marktree_getp_aux((*itr).x, k, None) + 1;

            let lvl = (*itr).lvl as usize;
            (*itr).s[lvl].i = (*itr).i;
            (*itr).s[lvl].oldcol = (*itr).pos.col;

            if (*itr).i > 0 {
                let prev_pos = (*(*itr).x).key[(*itr).i as usize - 1].pos;
                compose(&mut (*itr).pos, prev_pos);
                relative(prev_pos, &mut (*itr).intersect_pos_x);
            }
            (*itr).x = *node_ptr((*itr).x).add((*itr).i as usize);
            (*itr).lvl += 1;
            (*itr).i = -1;
            (*itr).intersect_idx = 0;
        }

        while (*itr).i < (*(*itr).x).n && pos_less((*rawkey(itr)).pos, (*itr).intersect_pos_x) {
            let mut k = (*(*itr).x).key[(*itr).i as usize];
            (*itr).i += 1;
            (*itr).s[(*itr).lvl as usize].i = (*itr).i;
            if mt_start(k) {
                let end = marktree_lookup(b, mt_lookup_id(k.ns, k.id, true), std::ptr::null_mut());
                if pos_less(end.pos, (*itr).intersect_pos) {
                    continue;
                }

                unrelative((*itr).pos, &mut k.pos);
                *pair = mtpair_from(k, end);
                return true;
            }
        }

        while (*itr).i < (*(*itr).x).n {
            let mut k = (*(*itr).x).key[(*itr).i as usize];
            (*itr).i += 1;
            if mt_end(k) {
                let id = mt_lookup_id(k.ns, k.id, false);
                if id2node(b, id) == (*itr).x {
                    continue;
                }
                unrelative((*itr).pos, &mut k.pos);
                let start = marktree_lookup(b, id, std::ptr::null_mut());
                if pos_leq((*itr).intersect_pos, start.pos) {
                    continue;
                }
                *pair = mtpair_from(start, k);
                return true;
            }
        }

        (*itr).i = (*itr).s[(*itr).lvl as usize].i;
        debug_assert!((*itr).i >= 0);
        if (*itr).i >= (*(*itr).x).n {
            marktree_itr_next(b, itr);
        }

        false
    }
}

unsafe fn check_damage(
    _b: *mut MarkTree,
    damage: &mut MTDamageMap,
    itr1: *mut MarkTreeIter,
    itr2: *mut MarkTreeIter,
) {
    unsafe {
        let start_id = mt_lookup_key_side(*rawkey(itr1), false);
        let p = damage.entry(start_id).or_default();
        let me: &mut MTDamage = if mt_end(*rawkey(itr1)) {
            &mut p.end
        } else {
            &mut p.start
        };
        debug_assert!(me.new.is_null());
        *me = MTDamage {
            old: (*itr1).x,
            new: (*itr2).x,
            old_i: (*itr1).i,
            new_i: (*itr2).i,
        };
    }
}

unsafe fn swap_keys(
    b: *mut MarkTree,
    itr1: *mut MarkTreeIter,
    itr2: *mut MarkTreeIter,
    damage: &mut MTDamageMap,
) {
    unsafe {
        if (*(*itr1).x).level != 0 || (*itr1).x != (*itr2).x {
            if mt_paired(*rawkey(itr1)) {
                check_damage(b, damage, itr1, itr2);
            }
            if mt_paired(*rawkey(itr2)) {
                check_damage(b, damage, itr2, itr1);
            }
        }

        if (*itr1).x != (*itr2).x {
            let mut meta_inc_1 = [0u32; kMTMetaCount];
            meta_describe_key(meta_inc_1.as_mut_ptr(), *rawkey(itr1));
            let mut meta_inc_2 = [0u32; kMTMetaCount];
            meta_describe_key(meta_inc_2.as_mut_ptr(), *rawkey(itr2));

            if meta_inc_1 != meta_inc_2 {
                let mut x1 = (*itr1).x;
                let mut x2 = (*itr2).x;
                while x1 != x2 {
                    if (*x1).level <= (*x2).level {
                        let meta_node = node_meta((*x1).parent).add((*x1).p_idx as usize);
                        for m in 0..kMTMetaCount {
                            (*meta_node)[m] = (*meta_node)[m]
                                .wrapping_add(meta_inc_2[m])
                                .wrapping_sub(meta_inc_1[m]);
                        }
                        x1 = (*x1).parent;
                    }
                    if (*x2).level < (*x1).level {
                        let meta_node = node_meta((*x2).parent).add((*x2).p_idx as usize);
                        for m in 0..kMTMetaCount {
                            (*meta_node)[m] = (*meta_node)[m]
                                .wrapping_add(meta_inc_1[m])
                                .wrapping_sub(meta_inc_2[m]);
                        }
                        x2 = (*x2).parent;
                    }
                }
            }
        }

        let key1 = *rawkey(itr1);
        let key2 = *rawkey(itr2);
        *rawkey(itr1) = key2;
        (*rawkey(itr1)).pos = key1.pos;
        *rawkey(itr2) = key1;
        (*rawkey(itr2)).pos = key2.pos;
        refkey(b, (*itr1).x, (*itr1).i);
        refkey(b, (*itr2).x, (*itr2).i);
    }
}

pub unsafe fn marktree_splice(
    b: *mut MarkTree,
    start_line: i32,
    start_col: c_int,
    old_extent_line: c_int,
    old_extent_col: c_int,
    new_extent_line: c_int,
    new_extent_col: c_int,
) -> bool {
    unsafe {
        let start = MTPos {
            row: start_line,
            col: start_col,
        };
        let mut old_extent = MTPos {
            row: old_extent_line,
            col: old_extent_col,
        };
        let mut new_extent = MTPos {
            row: new_extent_line,
            col: new_extent_col,
        };

        let mut may_delete = old_extent.row != 0 || old_extent.col != 0;
        let same_line = old_extent.row == 0 && new_extent.row == 0;
        unrelative(start, &mut old_extent);
        unrelative(start, &mut new_extent);
        let mut itr = MarkTreeIter::default();
        let mut enditr = MarkTreeIter::default();

        let mut oldbase = [MTPos { row: 0, col: 0 }; MT_MAX_DEPTH];

        marktree_itr_get_ext(
            b,
            start,
            &mut itr,
            false,
            true,
            oldbase.as_mut_ptr(),
            std::ptr::null(),
        );
        if itr.x.is_null() {
            return false;
        }
        let delta = MTPos {
            row: new_extent.row - old_extent.row,
            col: new_extent.col - old_extent.col,
        };

        if may_delete {
            let ipos = marktree_itr_pos(&mut itr);
            if !pos_leq(old_extent, ipos)
                || (old_extent.row == ipos.row
                    && old_extent.col == ipos.col
                    && !mt_right(*rawkey(&mut itr)))
            {
                marktree_itr_get_ext(
                    b,
                    old_extent,
                    &mut enditr,
                    true,
                    true,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                debug_assert!(!enditr.x.is_null());
            } else {
                may_delete = false;
            }
        }

        let mut past_right = false;
        let mut moved = false;
        let mut damage: MTDamageMap = MTDamageMap::new();

        if may_delete {
            'outer1: while !itr.x.is_null() && !past_right {
                let mut loc_start = start;
                let mut loc_old = old_extent;
                relative(itr.pos, &mut loc_start);
                relative(oldbase[itr.lvl as usize], &mut loc_old);

                loop {
                    if !pos_leq((*rawkey(&mut itr)).pos, loc_old) {
                        break 'outer1;
                    }

                    if mt_right(*rawkey(&mut itr)) {
                        while !itr_eq(&mut itr, &mut enditr) && mt_right(*rawkey(&mut enditr)) {
                            marktree_itr_prev(b, &mut enditr);
                        }
                        if !mt_right(*rawkey(&mut enditr)) {
                            swap_keys(b, &mut itr, &mut enditr, &mut damage);
                        } else {
                            break 'outer1;
                        }
                    }

                    if itr_eq(&mut itr, &mut enditr) {
                        past_right = true;
                    }

                    moved = true;
                    if (*itr.x).level != 0 {
                        let lvl = itr.lvl as usize;
                        oldbase[lvl + 1] = (*rawkey(&mut itr)).pos;
                        let base_lvl = oldbase[lvl];
                        unrelative(base_lvl, &mut oldbase[lvl + 1]);
                        (*rawkey(&mut itr)).pos = loc_start;
                        marktree_itr_next_skip(
                            b,
                            &mut itr,
                            false,
                            false,
                            oldbase.as_mut_ptr(),
                            std::ptr::null(),
                        );
                        break;
                    } else {
                        (*rawkey(&mut itr)).pos = loc_start;
                        if itr.i < (*itr.x).n - 1 {
                            itr.i += 1;
                            if !past_right {
                                continue;
                            }
                        } else {
                            marktree_itr_next(b, &mut itr);
                        }
                        break;
                    }
                }
            }
            'outer2: while !itr.x.is_null() {
                let mut loc_new = new_extent;
                relative(itr.pos, &mut loc_new);
                let mut limit = old_extent;
                relative(oldbase[itr.lvl as usize], &mut limit);

                loop {
                    if pos_leq(limit, (*rawkey(&mut itr)).pos) {
                        break 'outer2;
                    }

                    let oldpos = (*rawkey(&mut itr)).pos;
                    (*rawkey(&mut itr)).pos = loc_new;
                    moved = true;
                    if (*itr.x).level != 0 {
                        let lvl = itr.lvl as usize;
                        oldbase[lvl + 1] = oldpos;
                        let base_lvl = oldbase[lvl];
                        unrelative(base_lvl, &mut oldbase[lvl + 1]);

                        marktree_itr_next_skip(
                            b,
                            &mut itr,
                            false,
                            false,
                            oldbase.as_mut_ptr(),
                            std::ptr::null(),
                        );
                        break;
                    } else if itr.i < (*itr.x).n - 1 {
                        itr.i += 1;
                    } else {
                        marktree_itr_next(b, &mut itr);
                        break;
                    }
                }
            }
        }

        while !itr.x.is_null() {
            unrelative(oldbase[itr.lvl as usize], &mut (*rawkey(&mut itr)).pos);
            let realrow = (*rawkey(&mut itr)).pos.row;
            debug_assert!(realrow >= old_extent.row);
            let mut done = false;
            if realrow == old_extent.row {
                if delta.col != 0 {
                    (*rawkey(&mut itr)).pos.col += delta.col;
                }
            } else if same_line {
                done = true;
            }
            if delta.row != 0 {
                (*rawkey(&mut itr)).pos.row += delta.row;
                moved = true;
            }
            relative(itr.pos, &mut (*rawkey(&mut itr)).pos);
            if done {
                break;
            }
            marktree_itr_next_skip(
                b,
                &mut itr,
                true,
                false,
                std::ptr::null_mut(),
                std::ptr::null(),
            );
        }

        for (start_id, d) in damage {
            if !d.start.old.is_null() && !d.end.old.is_null() {
                marktree_itr_set_node(b, &mut itr, d.start.old, d.start.old_i);
                marktree_itr_set_node(b, &mut enditr, d.end.old, d.end.old_i);
                marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, true);
                marktree_itr_set_node(b, &mut itr, d.start.new, d.start.new_i);
                marktree_itr_set_node(b, &mut enditr, d.end.new, d.end.new_i);
                marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, false);
            } else if !d.start.old.is_null() {
                let mut endpos = MarkTreeIter::default();
                marktree_lookup(b, start_id | MARKTREE_END_FLAG, &mut endpos);
                if !endpos.x.is_null() {
                    marktree_itr_set_node(b, &mut itr, d.start.old, d.start.old_i);
                    enditr = endpos;
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, true);
                    marktree_itr_set_node(b, &mut itr, d.start.new, d.start.new_i);
                    enditr = endpos;
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, false);
                }
            } else if !d.end.old.is_null() {
                let mut startpos = MarkTreeIter::default();
                marktree_lookup(b, start_id, &mut startpos);
                if !startpos.x.is_null() {
                    itr = startpos;
                    marktree_itr_set_node(b, &mut enditr, d.end.old, d.end.old_i);
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, true);
                    itr = startpos;
                    marktree_itr_set_node(b, &mut enditr, d.end.new, d.end.new_i);
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, false);
                }
            }
        }

        moved
    }
}

pub unsafe fn marktree_move_region(
    b: *mut MarkTree,
    start_row: c_int,
    start_col: colnr_T,
    extent_row: c_int,
    extent_col: colnr_T,
    new_row: c_int,
    new_col: colnr_T,
) {
    unsafe {
        let start = MTPos {
            row: start_row,
            col: start_col,
        };
        let size = MTPos {
            row: extent_row,
            col: extent_col,
        };
        let mut end = size;
        unrelative(start, &mut end);
        let mut itr = MarkTreeIter::default();
        marktree_itr_get_ext(
            b,
            start,
            &mut itr,
            false,
            true,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        let mut saved: Vec<MTKey> = Vec::new();
        while !itr.x.is_null() {
            let mut k = marktree_itr_current(&mut itr);
            if !pos_leq(k.pos, end) || (k.pos.row == end.row && k.pos.col == end.col && mt_right(k))
            {
                break;
            }
            relative(start, &mut k.pos);
            saved.push(k);
            marktree_del_itr(b, &mut itr, false);
        }

        marktree_splice(b, start.row, start.col, size.row, size.col, 0, 0);
        let new = MTPos {
            row: new_row,
            col: new_col,
        };
        marktree_splice(b, new.row, new.col, 0, 0, size.row, size.col);

        for mut item in saved {
            unrelative(new, &mut item.pos);
            marktree_put_key(b, item);
            if mt_paired(item) {
                marktree_restore_pair(b, item);
            }
        }
    }
}

pub unsafe fn marktree_lookup_ns(
    b: *mut MarkTree,
    ns: u32,
    id: u32,
    end: bool,
    itr: *mut MarkTreeIter,
) -> MTKey {
    unsafe { marktree_lookup(b, mt_lookup_id(ns, id, end), itr) }
}

unsafe fn pseudo_index(mut x: *mut MTNode, mut i: c_int) -> u64 {
    unsafe {
        let mut off = MT_LOG2_BRANCH * c_int::from((*x).level);
        let mut index = 0u64;

        while !x.is_null() {
            index |= ((i + 1) as u64) << off;
            off += MT_LOG2_BRANCH;
            i = c_int::from((*x).p_idx);
            x = (*x).parent;
        }

        index
    }
}

unsafe fn pseudo_index_for_id(b: *mut MarkTree, id: u64, sloppy: bool) -> u64 {
    unsafe {
        let n = id2node(b, id);
        if n.is_null() {
            return 0;
        }

        let mut i = 0;
        if (*n).level != 0 || !sloppy {
            while i < (*n).n {
                if mt_lookup_key((*n).key[i as usize]) == id {
                    break;
                }
                i += 1;
            }
            debug_assert!(i < (*n).n);
            if (*n).level != 0 {
                i += 1;
            }
        }

        pseudo_index(n, i)
    }
}

pub unsafe fn marktree_lookup(b: *mut MarkTree, id: u64, itr: *mut MarkTreeIter) -> MTKey {
    unsafe {
        let n = id2node(b, id);
        if n.is_null() {
            if !itr.is_null() {
                (*itr).x = std::ptr::null_mut();
            }
            return MT_INVALID_KEY;
        }
        for i in 0..(*n).n {
            if mt_lookup_key((*n).key[i as usize]) == id {
                return marktree_itr_set_node(b, itr, n, i);
            }
        }

        std::process::abort();
    }
}

pub unsafe fn marktree_itr_set_node(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    mut n: *mut MTNode,
    mut i: c_int,
) -> MTKey {
    unsafe {
        let mut key = (*n).key[i as usize];
        if !itr.is_null() {
            (*itr).i = i;
            (*itr).x = n;
            (*itr).lvl = c_int::from((*(*b).root).level - (*n).level);
        }
        while !(*n).parent.is_null() {
            let p = (*n).parent;
            i = c_int::from((*n).p_idx);
            debug_assert_eq!(*node_ptr(p).add(i as usize), n);

            if !itr.is_null() {
                (*itr).s[((*(*b).root).level - (*p).level) as usize].i = i;
            }
            if i > 0 {
                unrelative((*p).key[i as usize - 1].pos, &mut key.pos);
            }
            n = p;
        }
        if !itr.is_null() {
            marktree_itr_fix_pos(b, itr);
        }
        key
    }
}

pub unsafe fn marktree_get_altpos(b: *mut MarkTree, mark: MTKey, itr: *mut MarkTreeIter) -> MTPos {
    unsafe { marktree_get_alt(b, mark, itr).pos }
}

pub unsafe fn marktree_get_alt(b: *mut MarkTree, mark: MTKey, itr: *mut MarkTreeIter) -> MTKey {
    if mt_paired(mark) {
        unsafe { marktree_lookup_ns(b, mark.ns, mark.id, !mt_end(mark), itr) }
    } else {
        mark
    }
}

unsafe fn marktree_itr_fix_pos(b: *mut MarkTree, itr: *mut MarkTreeIter) {
    unsafe {
        (*itr).pos = MTPos { row: 0, col: 0 };
        let mut x = (*b).root;
        for lvl in 0..(*itr).lvl as usize {
            (*itr).s[lvl].oldcol = (*itr).pos.col;
            let i = (*itr).s[lvl].i;
            if i > 0 {
                compose(&mut (*itr).pos, (*x).key[i as usize - 1].pos);
            }
            debug_assert!((*x).level != 0);
            x = *node_ptr(x).add(i as usize);
        }
        debug_assert_eq!(x, (*itr).x);
    }
}

#[cfg(test)]
pub unsafe fn marktree_put_test(
    b: *mut MarkTree,
    ns: u32,
    id: u32,
    row: c_int,
    col: c_int,
    right_gravity: bool,
    end_row: c_int,
    end_col: c_int,
    end_right: bool,
    meta_inline: bool,
) {
    let mut flags = mt_flags(right_gravity, false, false, false);
    if meta_inline {
        flags |= MT_FLAG_DECOR_VIRT_TEXT_INLINE;
    }
    let key = MTKey {
        pos: MTPos { row, col },
        ns,
        id,
        flags,
        decor_data: DecorInlineData {
            hl: DECOR_HIGHLIGHT_INLINE_INIT,
        },
    };
    unsafe {
        marktree_put(b, key, end_row, end_col, end_right);
    }
}

#[cfg(test)]
pub unsafe fn marktree_del_pair_test(b: *mut MarkTree, ns: u32, id: u32) {
    unsafe {
        let mut itr = MarkTreeIter::default();
        marktree_lookup_ns(b, ns, id, false, &mut itr);

        let other = marktree_del_itr(b, &mut itr, false);
        assert!(other != 0);
        marktree_lookup(b, other, &mut itr);
        marktree_del_itr(b, &mut itr, false);
    }
}

#[cfg(test)]
pub unsafe fn marktree_check(b: *mut MarkTree) {
    unsafe {
        if (*b).root.is_null() {
            assert_eq!((*b).n_keys, 0);
            assert_eq!((*b).n_nodes, 0);
            assert_eq!((*b).id2node.len(), 0);
            return;
        }

        let mut dummy = MTPos { row: 0, col: 0 };
        let mut last_right = false;

        let nkeys = marktree_check_node(
            b,
            (*b).root,
            &mut dummy,
            &mut last_right,
            (*b).meta_root.as_ptr(),
        );
        assert_eq!((*b).n_keys, nkeys);
        assert_eq!((*b).n_keys, (*b).id2node.len());
    }
}

#[cfg(test)]
pub unsafe fn marktree_check_node(
    b: *mut MarkTree,
    x: *mut MTNode,
    last: *mut MTPos,
    last_right: *mut bool,
    meta_node_ref: *const u32,
) -> usize {
    unsafe {
        assert!((*x).n < 2 * T);
        assert!((*x).n >= if x != (*b).root { T - 1 } else { 0 });
        let mut n_keys = (*x).n as usize;

        for i in 0..(*x).n as usize {
            if (*x).level != 0 {
                n_keys += marktree_check_node(
                    b,
                    *node_ptr(x).add(i),
                    last,
                    last_right,
                    (*node_meta(x).add(i)).as_ptr(),
                );
            } else {
                *last = MTPos { row: 0, col: 0 };
            }
            if i > 0 {
                unrelative((*x).key[i - 1].pos, &mut *last);
            }
            assert!(pos_leq(*last, (*x).key[i].pos));
            if (*last).row == (*x).key[i].pos.row && (*last).col == (*x).key[i].pos.col {
                assert!(!*last_right || mt_right((*x).key[i]));
            }
            *last_right = mt_right((*x).key[i]);
            assert!((*x).key[i].pos.col >= 0);
            assert_eq!(id2node(b, mt_lookup_key((*x).key[i])), x);
        }

        if (*x).level != 0 {
            let xn = (*x).n as usize;
            n_keys += marktree_check_node(
                b,
                *node_ptr(x).add(xn),
                last,
                last_right,
                (*node_meta(x).add(xn)).as_ptr(),
            );
            unrelative((*x).key[xn - 1].pos, &mut *last);

            let x_ptr = node_ptr(x);
            for i in 0..(xn + 1) {
                let child = *x_ptr.add(i);
                assert_eq!((*child).parent, x);
                assert_eq!((*child).p_idx as usize, i);
                assert_eq!((*child).level, (*x).level - 1);
                for j in 0..i {
                    assert_ne!(child, *x_ptr.add(j));
                }
            }
        } else if (*x).n > 0 {
            *last = (*x).key[(*x).n as usize - 1].pos;
        }

        let mut meta_node = [0u32; kMTMetaCount];
        meta_describe_node(meta_node.as_mut_ptr(), x);
        for (m, &val) in meta_node.iter().enumerate() {
            assert_eq!(*meta_node_ref.add(m), val);
        }

        n_keys
    }
}

#[cfg(test)]
pub unsafe fn marktree_check_intersections(b: *mut MarkTree) -> bool {
    unsafe {
        if (*b).root.is_null() {
            return true;
        }
        let mut checked: PMapPtr = PMapPtr::new();

        // 1. move x->intersect to checked[x] and reinit x->intersect
        mt_recurse_nodes((*b).root, &mut checked);

        // 2. iterate over all marks. for each START mark of a pair,
        // intersect the nodes between the pair
        let mut itr = MarkTreeIter::default();
        marktree_itr_first(b, &mut itr);
        loop {
            let mark = marktree_itr_current(&mut itr);
            if mark.pos.row < 0 {
                break;
            }

            if mt_start(mark) {
                let mut start_itr = itr;
                let mut end_itr = MarkTreeIter::default();
                let end_id = mt_lookup_id(mark.ns, mark.id, true);
                let k = marktree_lookup(b, end_id, &mut end_itr);
                if k.pos.row >= 0 {
                    marktree_intersect_pair(
                        b,
                        mt_lookup_key(mark),
                        &mut start_itr,
                        &mut end_itr,
                        false,
                    );
                }
            }

            marktree_itr_next(b, &mut itr);
        }

        // 3. for each node check if the recreated intersection
        // matches the old checked[x] intersection.
        mt_recurse_nodes_compare((*b).root, &checked)
    }
}

#[cfg(test)]
pub unsafe fn mt_recurse_nodes(x: *mut MTNode, checked: &mut PMapPtr) {
    unsafe {
        if !(*x).intersect.is_empty() {
            checked.insert(x, std::mem::take(&mut (*x).intersect));
        }

        if (*x).level != 0 {
            let x_ptr = node_ptr(x);
            for i in 0..((*x).n as usize + 1) {
                mt_recurse_nodes(*x_ptr.add(i), checked);
            }
        }
    }
}

#[cfg(test)]
pub unsafe fn mt_recurse_nodes_compare(x: *mut MTNode, checked: &PMapPtr) -> bool {
    unsafe {
        let expected = checked.get(&x).map_or(&[][..], |v| v.as_slice());
        if (*x).intersect.as_slice() != expected {
            return false;
        }

        if (*x).level != 0 {
            let x_ptr = node_ptr(x);
            for i in 0..((*x).n as usize + 1) {
                if !mt_recurse_nodes_compare(*x_ptr.add(i), checked) {
                    return false;
                }
            }
        }

        true
    }
}

pub unsafe fn mt_inspect(b: *mut MarkTree, keys: bool, dot: bool) -> NvimString {
    unsafe {
        let mut out = String::new();
        let p = MTPos { row: 0, col: 0 };
        if !(*b).root.is_null() {
            if dot {
                out.push_str("digraph D {\n\n");
                mt_inspect_dotfile_node(b, &mut out, (*b).root, p, std::ptr::null());
                out.push_str("\n}");
            } else {
                mt_inspect_node(b, &mut out, keys, (*b).root, p);
            }
        }
        NvimString {
            data: xmemdupz(out.as_ptr().cast(), out.len()).cast::<c_char>(),
            size: out.len(),
        }
    }
}

#[inline]
const fn mt_dbg_id(id: u64) -> u64 {
    (id >> 1) & 0xffffffff
}

unsafe fn mt_inspect_node(
    _b: *mut MarkTree,
    out: &mut String,
    keys: bool,
    n: *mut MTNode,
    off: MTPos,
) {
    unsafe {
        out.push('[');
        if keys && !(*n).intersect.is_empty() {
            for (i, &id) in (*n).intersect.iter().enumerate() {
                out.push_str(if i == 0 { "{" } else { ";" });
                let _ = write!(out, "{}", mt_dbg_id(id));
            }
            out.push_str("},");
        }
        if (*n).level != 0 {
            mt_inspect_node(_b, out, keys, *node_ptr(n), off);
        }
        for i in 0..(*n).n as usize {
            let mut p = (*n).key[i].pos;
            unrelative(off, &mut p);
            let _ = write!(out, "{}/{}", p.row, p.col);
            if keys {
                let key = (*n).key[i];
                out.push(':');
                if mt_start(key) {
                    out.push('<');
                }
                let _ = write!(out, "{}", key.id);
                if mt_end(key) {
                    out.push('>');
                }
            }
            if (*n).level != 0 {
                mt_inspect_node(_b, out, keys, *node_ptr(n).add(i + 1), p);
            } else {
                out.push(',');
            }
        }
        out.push(']');
    }
}

unsafe fn mt_inspect_dotfile_node(
    _b: *mut MarkTree,
    out: &mut String,
    n: *mut MTNode,
    off: MTPos,
    parent: *const c_char,
) {
    unsafe {
        let mut namebuf = [0 as c_char; 64];
        let name_str = if !parent.is_null() {
            let p_str = CStr::from_ptr(parent).to_string_lossy();
            let s = format!(
                "{}_{}{}",
                p_str,
                (b'a' + (*n).level as u8) as char,
                (*n).p_idx
            );
            let bytes = s.as_bytes();
            let copy_len = bytes.len().min(63);
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr().cast::<c_char>(),
                namebuf.as_mut_ptr(),
                copy_len,
            );
            namebuf[copy_len] = 0;
            s
        } else {
            std::ptr::copy_nonoverlapping(c"MTNode".as_ptr(), namebuf.as_mut_ptr(), 7);
            "MTNode".to_string()
        };

        let _ = writeln!(out, "  {name_str}[shape=plaintext, label=<");
        out.push_str("    <table border='0' cellborder='1' cellspacing='0'>\n");
        if !(*n).intersect.is_empty() {
            out.push_str("    <tr><td>");
            for (i, &id) in (*n).intersect.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "{}", mt_dbg_id(id));
            }
            out.push_str("</td></tr>\n");
        }

        out.push_str("    <tr><td>");
        for i in 0..(*n).n as usize {
            let k = (*n).key[i];
            if i > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{}", k.id);
            if mt_paired(k) {
                out.push_str(if mt_end(k) { "e" } else { "s" });
            }
        }
        out.push_str("</td></tr>\n    </table>\n>];\n");
        if !parent.is_null() {
            let p_str = CStr::from_ptr(parent).to_string_lossy();
            let _ = writeln!(out, "  {p_str} -> {name_str}");
        }
        if (*n).level != 0 {
            mt_inspect_dotfile_node(_b, out, *node_ptr(n), off, namebuf.as_ptr());
        }
        for i in 0..(*n).n as usize {
            let mut p = (*n).key[i].pos;
            unrelative(off, &mut p);
            if (*n).level != 0 {
                mt_inspect_dotfile_node(_b, out, *node_ptr(n).add(i + 1), p, namebuf.as_ptr());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marktree_put_splice_lookup_and_check() {
        let mut tree = MarkTree::new();
        unsafe {
            for i in 1..=30u32 {
                marktree_put_test(
                    &mut tree,
                    1,
                    i,
                    (i / 3) as c_int,
                    (i % 5) as c_int,
                    i % 2 == 0,
                    (i / 3 + 1) as c_int,
                    (i % 5 + 2) as c_int,
                    true,
                    i % 3 == 0,
                );
            }
            marktree_check(&mut tree);
            assert!(marktree_check_intersections(&mut tree));

            marktree_splice(&mut tree, 2, 1, 1, 2, 3, 4);
            marktree_check(&mut tree);
            assert!(marktree_check_intersections(&mut tree));

            marktree_del_pair_test(&mut tree, 1, 5);
            marktree_check(&mut tree);
            assert!(marktree_check_intersections(&mut tree));

            marktree_clear(&mut tree);
            marktree_check(&mut tree);

            let mut id = 0u32;
            for i in 1..=50 {
                for j in 1..=50 {
                    id += 1;
                    marktree_put_test(&mut tree, 10, id, j, i, (i % 2) > 0, -1, -1, false, false);
                }
            }
            marktree_check(&mut tree);
            marktree_splice(&mut tree, 2, 2, 0, 5, 1, 2);
            marktree_check(&mut tree);
            marktree_splice(&mut tree, 30, 2, 15, 5, 1, 2);
            marktree_check(&mut tree);
            marktree_clear(&mut tree);
            marktree_check(&mut tree);

            // intersect_mov_test
            let x = [1u64, 3, 4, 6, 7, 9];
            let y = [2u64, 3, 5, 6, 8, 9];
            let w = [0u64, 2, 5, 8, 10];
            let mut wout = [0u64; 17];
            let mut dout = [0u64; 17];
            let mut nwout = wout.len();
            let mut ndout = dout.len();
            assert!(intersect_mov_test(
                x.as_ptr(),
                x.len(),
                y.as_ptr(),
                y.len(),
                w.as_ptr(),
                w.len(),
                wout.as_mut_ptr(),
                &mut nwout,
                dout.as_mut_ptr(),
                &mut ndout,
            ));
            assert_eq!(&wout[..nwout], &[0, 1, 4, 7, 10]);
            assert_eq!(ndout, 0);

            // Edge case splicing overlapping ranges (#37867)
            let mut nid = 0u32;
            let mut put_range =
                |t: &mut MarkTree, r1: c_int, c1: c_int, g1: bool, r2: c_int, c2: c_int| {
                    nid += 1;
                    marktree_put_test(t, 10, nid, r1, c1, g1, r2, c2, false, false);
                };
            put_range(&mut tree, 190, 48, false, -1, -1);
            put_range(&mut tree, 48, 48, true, -1, -1);
            put_range(&mut tree, 190, 48, false, -1, -1);
            put_range(&mut tree, 166, 48, false, 166, 48);
            put_range(&mut tree, 48, 48, true, 48, 48);
            put_range(&mut tree, 48, 48, true, 48, 48);
            put_range(&mut tree, 48, 48, true, 255, 48);
            put_range(&mut tree, 131, 48, false, 48, 48);
            put_range(&mut tree, 131, 48, false, 48, 48);
            put_range(&mut tree, 48, 48, true, 131, 48);
            put_range(&mut tree, 48, 48, false, 216, 48);
            put_range(&mut tree, 172, 48, false, 51, 48);
            put_range(&mut tree, 131, 48, false, 131, 48);
            put_range(&mut tree, 156, 48, false, 131, 48);
            put_range(&mut tree, 135, 48, false, 166, 48);
            put_range(&mut tree, 172, 48, false, 250, 48);
            put_range(&mut tree, 48, 48, false, 143, 48);
            assert!(marktree_check_intersections(&mut tree));
            marktree_splice(&mut tree, 48, 0, 139, 0, 0, 0);
            assert!(marktree_check_intersections(&mut tree));
            marktree_clear(&mut tree);
        }
    }
}
