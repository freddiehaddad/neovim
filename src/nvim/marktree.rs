//! B-tree data structure for storing marks at `(row, col)` positions and updating them
//! to arbitrary text changes (`src/nvim/marktree.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt::Write as _;

use crate::garray::{ga_concat_len, ga_init, garray_T};
use crate::kvec::KVec;
use crate::map::{
    MTDamage, MTDamagePair, Map, map_del_uint64_t_ptr_t, map_put_ref_ptr_t_ptr_t,
    map_put_ref_uint64_t_MTDamagePair, map_put_ref_uint64_t_ptr_t, mh_get_ptr_t, mh_get_uint64_t,
    ptr_t,
};
use crate::map_glyph_cache::{MH_TOMBSTONE, NvimString};
use crate::memory::{xcalloc, xfree, xmalloc, xmemdup, xrealloc};

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
    pub vt: *mut c_void,
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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Intersection {
    pub size: usize,
    pub capacity: usize,
    pub items: *mut u64,
    pub init_array: [u64; 4],
}

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
    pub s: [mtnode_inner_s; 0],
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

pub type PMapUint64 = Map<u64, ptr_t>;
pub type PMapPtr = Map<ptr_t, ptr_t>;
pub type MTDamageMap = Map<u64, MTDamagePair>;

#[repr(C)]
pub struct MarkTree {
    pub root: *mut MTNode,
    pub meta_root: [u32; kMTMetaCount],
    pub n_keys: usize,
    pub n_nodes: usize,
    pub id2node: [PMapUint64; 1],
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
    unsafe { (*(*x).s.as_mut_ptr()).i_ptr.as_mut_ptr() }
}

#[inline]
unsafe fn node_meta(x: *mut MTNode) -> *mut [u32; kMTMetaCount] {
    unsafe { (*(*x).s.as_mut_ptr()).i_meta.as_mut_ptr() }
}

#[inline]
unsafe fn rawkey(itr: *mut MarkTreeIter) -> *mut MTKey {
    unsafe { (*(*itr).x).key.as_mut_ptr().add((*itr).i as usize) }
}

#[inline]
unsafe fn kvi_init(v: *mut Intersection) {
    unsafe {
        (*v).capacity = 4;
        (*v).size = 0;
        (*v).items = (*v).init_array.as_mut_ptr();
    }
}

#[inline]
unsafe fn kvi_resize(v: *mut Intersection, s: usize) {
    unsafe {
        let new_cap = s.max(4);
        (*v).capacity = new_cap;
        let init_ptr = (*v).init_array.as_mut_ptr();
        if new_cap == 4 {
            if (*v).items != init_ptr {
                std::ptr::copy_nonoverlapping((*v).items, init_ptr, (*v).size);
                xfree((*v).items.cast());
                (*v).items = init_ptr;
            }
        } else if (*v).items == init_ptr {
            let buf = xmalloc(new_cap * std::mem::size_of::<u64>()).cast::<u64>();
            std::ptr::copy_nonoverlapping(init_ptr, buf, (*v).size);
            (*v).items = buf;
        } else {
            (*v).items =
                xrealloc((*v).items.cast(), new_cap * std::mem::size_of::<u64>()).cast::<u64>();
        }
    }
}

#[inline]
unsafe fn kvi_ensure_more_space(v: *mut Intersection, len: usize) {
    unsafe {
        let needed = (*v).size + len;
        if (*v).capacity < needed {
            let mut cap = needed - 1;
            cap |= cap >> 1;
            cap |= cap >> 2;
            cap |= cap >> 4;
            cap |= cap >> 8;
            cap |= cap >> 16;
            cap += 1;
            kvi_resize(v, cap);
        }
    }
}

#[inline]
unsafe fn kvi_pushp(v: *mut Intersection) -> *mut u64 {
    unsafe {
        if (*v).size == (*v).capacity {
            let new_cap = (*v).capacity << 1;
            kvi_resize(v, new_cap);
        }
        let idx = (*v).size;
        (*v).size = idx + 1;
        (*v).items.add(idx)
    }
}

#[inline]
unsafe fn kvi_push(v: *mut Intersection, x: u64) {
    unsafe {
        *kvi_pushp(v) = x;
    }
}

#[inline]
unsafe fn kvi_copy(v1: *mut Intersection, v0: *const Intersection) {
    unsafe {
        if (*v1).capacity < (*v0).size {
            kvi_resize(v1, (*v0).size);
        }
        (*v1).size = (*v0).size;
        std::ptr::copy_nonoverlapping((*v0).items, (*v1).items, (*v0).size);
    }
}

#[inline]
unsafe fn kvi_destroy(v: *mut Intersection) {
    unsafe {
        let init_ptr = (*v).init_array.as_mut_ptr();
        if (*v).items != init_ptr {
            xfree((*v).items.cast());
            (*v).items = std::ptr::null_mut();
        }
    }
}

#[inline]
unsafe fn map_destroy_generic<K, V>(map: *mut Map<K, V>) {
    unsafe {
        xfree((*map).set.keys.cast());
        xfree((*map).set.h.hash.cast());
        xfree((*map).values.cast());
        *map = std::mem::zeroed();
    }
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
        let slot = map_put_ref_uint64_t_ptr_t(
            (*b).id2node.as_mut_ptr(),
            key_id,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        *slot = x.cast();
    }
}

#[inline]
unsafe fn id2node(b: *mut MarkTree, id: u64) -> *mut MTNode {
    unsafe {
        let map = (*b).id2node.as_mut_ptr();
        let k = mh_get_uint64_t(&mut (*map).set, id);
        if k == MH_TOMBSTONE {
            std::ptr::null_mut()
        } else {
            (*(*map).values.add(k as usize)).cast::<MTNode>()
        }
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

        kvi_copy(&mut (*z).intersect, &(*y).intersect);

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_put(
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

unsafe fn intersection_has(x: *const Intersection, id: u64) -> bool {
    unsafe {
        for i in 0..(*x).size {
            let item = *(*x).items.add(i);
            if item == id {
                return true;
            } else if item >= id {
                return false;
            }
        }
        false
    }
}

unsafe fn intersect_item_pos(x: *mut MTNode, id: u64) -> usize {
    unsafe {
        let mut lo = 0usize;
        let mut hi = (*x).intersect.size;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if *(*x).intersect.items.add(mid) < id {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

unsafe fn intersect_node(_b: *mut MarkTree, x: *mut MTNode, id: u64) {
    debug_assert_eq!(id & MARKTREE_END_FLAG, 0);
    unsafe {
        let sz = (*x).intersect.size;
        if sz == 0 || *(*x).intersect.items.add(sz - 1) < id {
            kvi_push(&mut (*x).intersect, id);
            return;
        }
        let i = intersect_item_pos(x, id);
        kvi_pushp(&mut (*x).intersect);
        let new_sz = (*x).intersect.size;
        std::ptr::copy(
            (*x).intersect.items.add(i),
            (*x).intersect.items.add(i + 1),
            new_sz - i - 1,
        );
        *(*x).intersect.items.add(i) = id;
    }
}

unsafe fn unintersect_node(_b: *mut MarkTree, x: *mut MTNode, id: u64, strict: bool) {
    debug_assert_eq!(id & MARKTREE_END_FLAG, 0);
    unsafe {
        let i = intersect_item_pos(x, id);
        let sz = (*x).intersect.size;
        let seen = i < sz && *(*x).intersect.items.add(i) == id;
        if strict {
            debug_assert!(seen);
        }

        if seen {
            if i < sz - 1 {
                std::ptr::copy(
                    (*x).intersect.items.add(i + 1),
                    (*x).intersect.items.add(i),
                    sz - i - 1,
                );
            }
            (*x).intersect.size -= 1;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_intersect_pair(
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
                let x = *node_ptr((*itr).x).add((*itr).i as usize + 1);
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
        let ilen = std::mem::size_of::<MTNode>() + std::mem::size_of::<mtnode_inner_s>();
        let size = if internal {
            ilen
        } else {
            std::mem::size_of::<MTNode>()
        };
        let x = xcalloc(1, size).cast::<MTNode>();
        kvi_init(&mut (*x).intersect);
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_put_key(b: *mut MarkTree, mut k: MTKey) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_del_itr(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    rev: bool,
) -> u64 {
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
        map_del_uint64_t_ptr_t((*b).id2node.as_mut_ptr(), id, std::ptr::null_mut());

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_revise_meta(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    old_key: MTKey,
) {
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

unsafe fn intersect_merge(m: *mut Intersection, x: *mut Intersection, y: *mut Intersection) {
    unsafe {
        let mut xi = 0usize;
        let mut yi = 0usize;
        let mut xn = 0usize;
        let mut yn = 0usize;
        while xi < (*x).size && yi < (*y).size {
            let xv = *(*x).items.add(xi);
            let yv = *(*y).items.add(yi);
            if xv == yv {
                kvi_push(m, xv);
                xi += 1;
                yi += 1;
            } else if xv < yv {
                *(*x).items.add(xn) = xv;
                xn += 1;
                xi += 1;
            } else {
                *(*y).items.add(yn) = yv;
                yn += 1;
                yi += 1;
            }
        }

        if xi < (*x).size {
            let rem = (*x).size - xi;
            std::ptr::copy((*x).items.add(xi), (*x).items.add(xn), rem);
            xn += rem;
        }
        if yi < (*y).size {
            let rem = (*y).size - yi;
            std::ptr::copy((*y).items.add(yi), (*y).items.add(yn), rem);
            yn += rem;
        }

        (*x).size = xn;
        (*y).size = yn;
    }
}

unsafe fn intersect_mov(
    x: *mut Intersection,
    y: *mut Intersection,
    w: *mut Intersection,
    d: *mut Intersection,
) {
    unsafe {
        let mut wi = 0usize;
        let mut yi = 0usize;
        let mut wn = 0usize;
        let mut yn = 0usize;
        let mut xi = 0usize;
        while wi < (*w).size || xi < (*x).size {
            if wi < (*w).size && (xi >= (*x).size || *(*x).items.add(xi) >= *(*w).items.add(wi)) {
                let wv = *(*w).items.add(wi);
                if xi < (*x).size && *(*x).items.add(xi) == wv {
                    xi += 1;
                }
                while yi < (*y).size && *(*y).items.add(yi) < wv {
                    kvi_push(d, *(*y).items.add(yi));
                    yi += 1;
                }
                if yi < (*y).size && *(*y).items.add(yi) == wv {
                    *(*y).items.add(yn) = *(*y).items.add(yi);
                    yn += 1;
                    yi += 1;
                    wi += 1;
                } else {
                    *(*w).items.add(wn) = wv;
                    wn += 1;
                    wi += 1;
                }
            } else {
                let xv = *(*x).items.add(xi);
                while yi < (*y).size && *(*y).items.add(yi) < xv {
                    kvi_push(d, *(*y).items.add(yi));
                    yi += 1;
                }
                if yi < (*y).size && *(*y).items.add(yi) == xv {
                    *(*y).items.add(yn) = *(*y).items.add(yi);
                    yn += 1;
                    yi += 1;
                    xi += 1;
                } else {
                    if wi == wn {
                        let n = (*w).size - wn;
                        kvi_pushp(w);
                        if n > 0 {
                            std::ptr::copy((*w).items.add(wn), (*w).items.add(wn + 1), n);
                        }
                        *(*w).items.add(wi) = xv;
                        wn += 1;
                        wi += 1;
                    } else {
                        debug_assert!(wn < wi);
                        *(*w).items.add(wn) = xv;
                        wn += 1;
                    }
                    xi += 1;
                }
            }
        }
        if yi < (*y).size {
            let n = (*y).size - yi;
            kvi_ensure_more_space(d, n);
            std::ptr::copy_nonoverlapping((*y).items.add(yi), (*d).items.add((*d).size), n);
            (*d).size += n;
        }
        (*w).size = wn;
        (*y).size = yn;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn intersect_mov_test(
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
        let mut xi = Intersection {
            size: nx,
            capacity: 0,
            items: x.cast_mut(),
            init_array: [0; 4],
        };
        let mut yi = Intersection {
            size: ny,
            capacity: 0,
            items: y.cast_mut(),
            init_array: [0; 4],
        };

        let mut w = Intersection {
            size: 0,
            capacity: 4,
            items: std::ptr::null_mut(),
            init_array: [0; 4],
        };
        kvi_init(&mut w);
        for i in 0..nwin {
            kvi_push(&mut w, *win.add(i));
        }
        let mut d = Intersection {
            size: 0,
            capacity: 4,
            items: std::ptr::null_mut(),
            init_array: [0; 4],
        };
        kvi_init(&mut d);

        intersect_mov(&mut xi, &mut yi, &mut w, &mut d);

        if w.size > *nwout || d.size > *ndout {
            kvi_destroy(&mut w);
            kvi_destroy(&mut d);
            return false;
        }

        std::ptr::copy_nonoverlapping(w.items, wout, w.size);
        *nwout = w.size;

        std::ptr::copy_nonoverlapping(d.items, dout, d.size);
        *ndout = d.size;

        kvi_destroy(&mut w);
        kvi_destroy(&mut d);
        true
    }
}

unsafe fn intersect_common(i: *mut Intersection, x: *const Intersection, y: *const Intersection) {
    unsafe {
        let mut xi = 0usize;
        let mut yi = 0usize;
        while xi < (*x).size && yi < (*y).size {
            let xv = *(*x).items.add(xi);
            let yv = *(*y).items.add(yi);
            if xv == yv {
                kvi_push(i, xv);
                xi += 1;
                yi += 1;
            } else if xv < yv {
                xi += 1;
            } else {
                yi += 1;
            }
        }
    }
}

unsafe fn intersect_add(x: *mut Intersection, y: *const Intersection) {
    unsafe {
        let mut xi = 0usize;
        let mut yi = 0usize;
        while xi < (*x).size && yi < (*y).size {
            let xv = *(*x).items.add(xi);
            let yv = *(*y).items.add(yi);
            if xv == yv {
                xi += 1;
                yi += 1;
            } else if yv < xv {
                let n = (*x).size - xi;
                kvi_pushp(x);
                std::ptr::copy((*x).items.add(xi), (*x).items.add(xi + 1), n);
                *(*x).items.add(xi) = yv;
                xi += 1;
                yi += 1;
            } else {
                xi += 1;
            }
        }
        if yi < (*y).size {
            let n = (*y).size - yi;
            kvi_ensure_more_space(x, n);
            std::ptr::copy_nonoverlapping((*y).items.add(yi), (*x).items.add((*x).size), n);
            (*x).size += n;
        }
    }
}

unsafe fn intersect_sub(x: *mut Intersection, y: *const Intersection) {
    unsafe {
        let mut xi = 0usize;
        let mut yi = 0usize;
        let mut xn = 0usize;
        while xi < (*x).size && yi < (*y).size {
            let xv = *(*x).items.add(xi);
            let yv = *(*y).items.add(yi);
            if xv == yv {
                xi += 1;
                yi += 1;
            } else if xv < yv {
                *(*x).items.add(xn) = xv;
                xn += 1;
                xi += 1;
            } else {
                yi += 1;
            }
        }
        if xi < (*x).size {
            let n = (*x).size - xi;
            if xn < xi {
                std::ptr::copy((*x).items.add(xi), (*x).items.add(xn), n);
            }
            xn += n;
        }
        (*x).size = xn;
    }
}

unsafe fn bubble_up(x: *mut MTNode) {
    unsafe {
        let mut xi = Intersection {
            size: 0,
            capacity: 4,
            items: std::ptr::null_mut(),
            init_array: [0; 4],
        };
        kvi_init(&mut xi);
        let x_ptr = node_ptr(x);
        intersect_common(
            &mut xi,
            &(**x_ptr).intersect,
            &(**x_ptr.add((*x).n as usize)).intersect,
        );
        if xi.size != 0 {
            for i in 0..((*x).n as usize + 1) {
                intersect_sub(&mut (**x_ptr.add(i)).intersect, &xi);
            }
            intersect_add(&mut (*x).intersect, &xi);
        }
        kvi_destroy(&mut xi);
    }
}

unsafe fn merge_node(b: *mut MarkTree, p: *mut MTNode, i: c_int) -> *mut MTNode {
    unsafe {
        let i_usize = i as usize;
        let p_ptr = node_ptr(p);
        let p_meta = node_meta(p);
        let x = *p_ptr.add(i_usize);
        let y = *p_ptr.add(i_usize + 1);
        let mut mi = Intersection {
            size: 0,
            capacity: 4,
            items: std::ptr::null_mut(),
            init_array: [0; 4],
        };
        kvi_init(&mut mi);

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
                for idx in 0..(*x).intersect.size {
                    intersect_node(b, *x_ptr.add(k), *(*x).intersect.items.add(idx));
                }
            }
            for ky in 0..(yn + 1) {
                let k = xn + ky + 1;
                let child = *x_ptr.add(k);
                (*child).parent = x;
                (*child).p_idx = k as i16;
                for idx in 0..(*y).intersect.size {
                    intersect_node(b, child, *(*y).intersect.items.add(idx));
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

        kvi_destroy(&mut (*x).intersect);
        kvi_move(&mut (*x).intersect, &mut mi);

        x
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kvi_move(dest: *mut Intersection, src: *mut Intersection) {
    unsafe {
        (*dest).size = (*src).size;
        (*dest).capacity = (*src).capacity;
        if (*src).items == (*src).init_array.as_mut_ptr() {
            std::ptr::copy_nonoverlapping(
                (*src).init_array.as_ptr(),
                (*dest).init_array.as_mut_ptr(),
                (*src).size,
            );
            (*dest).items = (*dest).init_array.as_mut_ptr();
        } else {
            (*dest).items = (*src).items;
        }
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
            let mut d = Intersection {
                size: 0,
                capacity: 4,
                items: std::ptr::null_mut(),
                init_array: [0; 4],
            };
            kvi_init(&mut d);
            let y_ptr = node_ptr(y);
            intersect_mov(
                &mut (*x).intersect,
                &mut (*y).intersect,
                &mut (**y_ptr).intersect,
                &mut d,
            );
            if d.size != 0 {
                for yi in 1..((*y).n as usize + 1) {
                    intersect_add(&mut (**y_ptr.add(yi)).intersect, &d);
                }
            }
            kvi_destroy(&mut d);

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
            let mut d = Intersection {
                size: 0,
                capacity: 4,
                items: std::ptr::null_mut(),
                init_array: [0; 4],
            };
            kvi_init(&mut d);
            let x_ptr = node_ptr(x);
            intersect_mov(
                &mut (*y).intersect,
                &mut (*x).intersect,
                &mut (**x_ptr.add((*x).n as usize)).intersect,
                &mut d,
            );
            if d.size != 0 {
                for xi in 0..(*x).n as usize {
                    intersect_add(&mut (**x_ptr.add(xi)).intersect, &d);
                }
            }
            kvi_destroy(&mut d);

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_clear(b: *mut MarkTree) {
    unsafe {
        if !(*b).root.is_null() {
            marktree_free_subtree(b, (*b).root);
            (*b).root = std::ptr::null_mut();
        }
        map_destroy_generic((*b).id2node.as_mut_ptr());
        (*b).n_keys = 0;
        (*b).meta_root = [0; kMTMetaCount];
        debug_assert_eq!((*b).n_nodes, 0);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_free_subtree(b: *mut MarkTree, x: *mut MTNode) {
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
        kvi_destroy(&mut (*x).intersect);
        xfree(x.cast());
        (*b).n_nodes -= 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_move(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    row: c_int,
    col: c_int,
) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_restore_pair(b: *mut MarkTree, key: MTKey) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_get(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_get_ext(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_first(b: *mut MarkTree, itr: *mut MarkTreeIter) -> bool {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_last(b: *mut MarkTree, itr: *mut MarkTreeIter) -> c_int {
    unsafe {
        if (*b).n_keys == 0 {
            (*itr).x = std::ptr::null_mut();
            return 0;
        }
        (*itr).pos = MTPos { row: 0, col: 0 };
        (*itr).x = (*b).root;
        (*itr).lvl = 0;
        loop {
            (*itr).i = (*(*itr).x).n;

            if (*(*itr).x).level == 0 {
                break;
            }

            let lvl = (*itr).lvl as usize;
            (*itr).s[lvl].i = (*itr).i;
            (*itr).s[lvl].oldcol = (*itr).pos.col;

            debug_assert!((*itr).i > 0);
            compose(&mut (*itr).pos, (*(*itr).x).key[(*itr).i as usize - 1].pos);

            (*itr).x = *node_ptr((*itr).x).add((*itr).i as usize);
            (*itr).lvl += 1;
        }
        (*itr).i -= 1;
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_next(b: *mut MarkTree, itr: *mut MarkTreeIter) -> bool {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_get_filter(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_step_out_filter(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_next_filter(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_prev(_b: *mut MarkTree, itr: *mut MarkTreeIter) -> bool {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_node_done(itr: *mut MarkTreeIter) -> bool {
    unsafe { (*itr).x.is_null() || (*itr).i == (*(*itr).x).n - 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_pos(itr: *mut MarkTreeIter) -> MTPos {
    unsafe {
        let mut pos = (*rawkey(itr)).pos;
        unrelative((*itr).pos, &mut pos);
        pos
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_current(itr: *mut MarkTreeIter) -> MTKey {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_get_overlap(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_step_overlap(
    b: *mut MarkTree,
    itr: *mut MarkTreeIter,
    pair: *mut MTPair,
) -> bool {
    unsafe {
        while (*itr).i == -1 {
            if (*itr).intersect_idx < (*(*itr).x).intersect.size {
                let id = *(*(*itr).x).intersect.items.add((*itr).intersect_idx);
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
    damage: *mut MTDamageMap,
    itr1: *mut MarkTreeIter,
    itr2: *mut MarkTreeIter,
) {
    unsafe {
        let start_id = mt_lookup_key_side(*rawkey(itr1), false);
        let p = map_put_ref_uint64_t_MTDamagePair(
            damage,
            start_id,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        let me: *mut MTDamage = if mt_end(*rawkey(itr1)) {
            &mut (*p).end
        } else {
            &mut (*p).start
        };
        debug_assert!((*me).new.is_null());
        *me = MTDamage {
            old: (*itr1).x.cast(),
            new: (*itr2).x.cast(),
            old_i: (*itr1).i,
            new_i: (*itr2).i,
        };
    }
}

unsafe fn swap_keys(
    b: *mut MarkTree,
    itr1: *mut MarkTreeIter,
    itr2: *mut MarkTreeIter,
    damage: *mut MTDamageMap,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_splice(
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
        let mut damage: MTDamageMap = std::mem::zeroed();

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

        for idx in 0..damage.set.h.n_keys as usize {
            let start_id = *damage.set.keys.add(idx);
            let d = *damage.values.add(idx);
            if !d.start.old.is_null() && !d.end.old.is_null() {
                marktree_itr_set_node(b, &mut itr, d.start.old.cast(), d.start.old_i);
                marktree_itr_set_node(b, &mut enditr, d.end.old.cast(), d.end.old_i);
                marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, true);
                marktree_itr_set_node(b, &mut itr, d.start.new.cast(), d.start.new_i);
                marktree_itr_set_node(b, &mut enditr, d.end.new.cast(), d.end.new_i);
                marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, false);
            } else if !d.start.old.is_null() {
                let mut endpos = MarkTreeIter::default();
                marktree_lookup(b, start_id | MARKTREE_END_FLAG, &mut endpos);
                if !endpos.x.is_null() {
                    marktree_itr_set_node(b, &mut itr, d.start.old.cast(), d.start.old_i);
                    enditr = endpos;
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, true);
                    marktree_itr_set_node(b, &mut itr, d.start.new.cast(), d.start.new_i);
                    enditr = endpos;
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, false);
                }
            } else if !d.end.old.is_null() {
                let mut startpos = MarkTreeIter::default();
                marktree_lookup(b, start_id, &mut startpos);
                if !startpos.x.is_null() {
                    itr = startpos;
                    marktree_itr_set_node(b, &mut enditr, d.end.old.cast(), d.end.old_i);
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, true);
                    itr = startpos;
                    marktree_itr_set_node(b, &mut enditr, d.end.new.cast(), d.end.new_i);
                    marktree_intersect_pair(b, start_id, &mut itr, &mut enditr, false);
                }
            }
        }
        map_destroy_generic(&mut damage);

        moved
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_move_region(
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
        let mut saved: KVec<MTKey> = KVec::new();
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

        for i in 0..saved.size {
            let mut item = *saved.items.add(i);
            unrelative(new, &mut item.pos);
            marktree_put_key(b, item);
            if mt_paired(item) {
                marktree_restore_pair(b, item);
            }
        }
        saved.destroy();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_lookup_ns(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_lookup(
    b: *mut MarkTree,
    id: u64,
    itr: *mut MarkTreeIter,
) -> MTKey {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_itr_set_node(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_get_altpos(
    b: *mut MarkTree,
    mark: MTKey,
    itr: *mut MarkTreeIter,
) -> MTPos {
    unsafe { marktree_get_alt(b, mark, itr).pos }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_get_alt(
    b: *mut MarkTree,
    mark: MTKey,
    itr: *mut MarkTreeIter,
) -> MTKey {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_put_test(
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

#[unsafe(no_mangle)]
pub extern "C" fn mt_right_test(key: MTKey) -> bool {
    mt_right(key)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_del_pair_test(b: *mut MarkTree, ns: u32, id: u32) {
    unsafe {
        let mut itr = MarkTreeIter::default();
        marktree_lookup_ns(b, ns, id, false, &mut itr);

        let other = marktree_del_itr(b, &mut itr, false);
        assert!(other != 0);
        marktree_lookup(b, other, &mut itr);
        marktree_del_itr(b, &mut itr, false);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_check(b: *mut MarkTree) {
    unsafe {
        if (*b).root.is_null() {
            assert_eq!((*b).n_keys, 0);
            assert_eq!((*b).n_nodes, 0);
            assert_eq!((*b).id2node[0].set.h.size, 0);
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
        assert_eq!((*b).n_keys, (*b).id2node[0].set.h.size as usize);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_check_node(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn marktree_check_intersections(b: *mut MarkTree) -> bool {
    unsafe {
        if (*b).root.is_null() {
            return true;
        }
        let mut checked: PMapPtr = std::mem::zeroed();

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
        let status = mt_recurse_nodes_compare((*b).root, &mut checked);

        for idx in 0..checked.set.h.n_keys as usize {
            let val = *checked.values.add(idx);
            xfree(val);
        }
        map_destroy_generic(&mut checked);

        status
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mt_recurse_nodes(x: *mut MTNode, checked: *mut PMapPtr) {
    unsafe {
        if (*x).intersect.size != 0 {
            kvi_push(&mut (*x).intersect, u64::MAX);
            let val: *mut u64 = if (*x).intersect.items == (*x).intersect.init_array.as_mut_ptr() {
                xmemdup(
                    (*x).intersect.items.cast(),
                    (*x).intersect.size * std::mem::size_of::<u64>(),
                )
                .cast::<u64>()
            } else {
                (*x).intersect.items
            };
            let slot = map_put_ref_ptr_t_ptr_t(
                checked,
                x.cast(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            *slot = val.cast();
            kvi_init(&mut (*x).intersect);
        }

        if (*x).level != 0 {
            let x_ptr = node_ptr(x);
            for i in 0..((*x).n as usize + 1) {
                mt_recurse_nodes(*x_ptr.add(i), checked);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mt_recurse_nodes_compare(x: *mut MTNode, checked: *mut PMapPtr) -> bool {
    unsafe {
        let k = mh_get_ptr_t(&mut (*checked).set, x.cast());
        let ref_ptr: *const u64 = if k == MH_TOMBSTONE {
            std::ptr::null()
        } else {
            (*(*checked).values.add(k as usize)).cast::<u64>()
        };
        if !ref_ptr.is_null() {
            let mut i = 0usize;
            loop {
                let rv = *ref_ptr.add(i);
                if rv == u64::MAX {
                    if i != (*x).intersect.size {
                        return false;
                    }
                    break;
                } else if (*x).intersect.size <= i || rv != *(*x).intersect.items.add(i) {
                    return false;
                }
                i += 1;
            }
        } else if (*x).intersect.size != 0 {
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

#[inline]
unsafe fn ga_put_str(ga: *mut garray_T, s: &str) {
    unsafe {
        ga_concat_len(ga, s.as_ptr().cast(), s.len());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mt_inspect(b: *mut MarkTree, keys: bool, dot: bool) -> NvimString {
    unsafe {
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: std::ptr::null_mut(),
        };
        ga_init(&mut ga, std::mem::size_of::<c_char>() as c_int, 80);
        let p = MTPos { row: 0, col: 0 };
        if !(*b).root.is_null() {
            if dot {
                ga_put_str(&mut ga, "digraph D {\n\n");
                mt_inspect_dotfile_node(b, &mut ga, (*b).root, p, std::ptr::null());
                ga_put_str(&mut ga, "\n}");
            } else {
                mt_inspect_node(b, &mut ga, keys, (*b).root, p);
            }
        }
        NvimString {
            data: ga.ga_data.cast::<c_char>(),
            size: ga.ga_len as usize,
        }
    }
}

#[inline]
const fn mt_dbg_id(id: u64) -> u64 {
    (id >> 1) & 0xffffffff
}

unsafe fn mt_inspect_node(
    _b: *mut MarkTree,
    ga: *mut garray_T,
    keys: bool,
    n: *mut MTNode,
    off: MTPos,
) {
    unsafe {
        let mut tmp = String::new();
        ga_put_str(ga, "[");
        if keys && (*n).intersect.size != 0 {
            for i in 0..(*n).intersect.size {
                ga_put_str(ga, if i == 0 { "{" } else { ";" });
                tmp.clear();
                let _ = write!(tmp, "{}", mt_dbg_id(*(*n).intersect.items.add(i)));
                ga_put_str(ga, &tmp);
            }
            ga_put_str(ga, "},");
        }
        if (*n).level != 0 {
            mt_inspect_node(_b, ga, keys, *node_ptr(n), off);
        }
        for i in 0..(*n).n as usize {
            let mut p = (*n).key[i].pos;
            unrelative(off, &mut p);
            tmp.clear();
            let _ = write!(tmp, "{}/{}", p.row, p.col);
            ga_put_str(ga, &tmp);
            if keys {
                let key = (*n).key[i];
                ga_put_str(ga, ":");
                if mt_start(key) {
                    ga_put_str(ga, "<");
                }
                tmp.clear();
                let _ = write!(tmp, "{}", key.id);
                ga_put_str(ga, &tmp);
                if mt_end(key) {
                    ga_put_str(ga, ">");
                }
            }
            if (*n).level != 0 {
                mt_inspect_node(_b, ga, keys, *node_ptr(n).add(i + 1), p);
            } else {
                ga_put_str(ga, ",");
            }
        }
        ga_put_str(ga, "]");
    }
}

unsafe fn mt_inspect_dotfile_node(
    _b: *mut MarkTree,
    ga: *mut garray_T,
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

        let mut tmp = String::new();
        let _ = writeln!(tmp, "  {name_str}[shape=plaintext, label=<");
        ga_put_str(ga, &tmp);
        ga_put_str(
            ga,
            "    <table border='0' cellborder='1' cellspacing='0'>\n",
        );
        if (*n).intersect.size != 0 {
            ga_put_str(ga, "    <tr><td>");
            for i in 0..(*n).intersect.size {
                if i > 0 {
                    ga_put_str(ga, ", ");
                }
                tmp.clear();
                let _ = write!(tmp, "{}", mt_dbg_id(*(*n).intersect.items.add(i)));
                ga_put_str(ga, &tmp);
            }
            ga_put_str(ga, "</td></tr>\n");
        }

        ga_put_str(ga, "    <tr><td>");
        for i in 0..(*n).n as usize {
            let k = (*n).key[i];
            if i > 0 {
                ga_put_str(ga, ", ");
            }
            tmp.clear();
            let _ = write!(tmp, "{}", k.id);
            ga_put_str(ga, &tmp);
            if mt_paired(k) {
                ga_put_str(ga, if mt_end(k) { "e" } else { "s" });
            }
        }
        ga_put_str(ga, "</td></tr>\n");
        ga_put_str(ga, "    </table>\n");
        ga_put_str(ga, ">];\n");
        if !parent.is_null() {
            let p_str = CStr::from_ptr(parent).to_string_lossy();
            tmp.clear();
            let _ = writeln!(tmp, "  {p_str} -> {name_str}");
            ga_put_str(ga, &tmp);
        }
        if (*n).level != 0 {
            mt_inspect_dotfile_node(_b, ga, *node_ptr(n), off, namebuf.as_ptr());
        }
        for i in 0..(*n).n as usize {
            let mut p = (*n).key[i].pos;
            unrelative(off, &mut p);
            if (*n).level != 0 {
                mt_inspect_dotfile_node(_b, ga, *node_ptr(n).add(i + 1), p, namebuf.as_ptr());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marktree_put_splice_lookup_and_check() {
        let mut tree: MarkTree = unsafe { std::mem::zeroed() };
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
        }
    }
}
