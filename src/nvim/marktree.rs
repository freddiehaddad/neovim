//! B-tree data structure for storing marks at `(row, col)` positions and updating them
//! to arbitrary text changes (`src/nvim/marktree.c`).

use std::fmt::Write as _;

use crate::api::private::helpers::cbuf_to_string;
use crate::api::private::validate::NvimString;

use crate::cursor::Colnr;

pub const MT_MAX_DEPTH: usize = 20;
pub const MT_BRANCH_FACTOR: usize = 10;
pub const MT_LOG2_BRANCH: i32 = 5;
const T: i32 = MT_BRANCH_FACTOR as i32;
const T_USIZE: usize = MT_BRANCH_FACTOR;

pub const K_MTMETA_INLINE: usize = 0;
pub const K_MTMETA_LINES: usize = 1;
pub const K_MTMETA_SIGN_HL: usize = 2;
pub const K_MTMETA_SIGN_TEXT: usize = 3;
pub const K_MTMETA_CONCEAL_LINES: usize = 4;
pub const K_MTMETA_COUNT: usize = 5;

pub type MetaFilter = [u32; K_MTMETA_COUNT];

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

const META_MAP: [u32; K_MTMETA_COUNT] = [
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
    pub hl_id: i32,
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

#[derive(Clone, Copy)]
pub enum DecorInlineData {
    Hl(DecorHighlightInline),
    Ext(DecorExt),
}

impl DecorInlineData {
    #[inline]
    pub const fn hl(self) -> DecorHighlightInline {
        match self {
            Self::Hl(hl) => hl,
            Self::Ext(_) => DECOR_HIGHLIGHT_INLINE_INIT,
        }
    }

    #[inline]
    pub fn hl_mut(&mut self) -> &mut DecorHighlightInline {
        match self {
            Self::Hl(hl) => hl,
            Self::Ext(_) => unreachable!(),
        }
    }

    #[inline]
    pub const fn ext(self) -> DecorExt {
        match self {
            Self::Ext(ext) => ext,
            Self::Hl(_) => DecorExt {
                sh_idx: u32::MAX,
                vt: std::ptr::null_mut(),
            },
        }
    }
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
    decor_data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MTPair {
    pub start: MTKey,
    pub end_pos: MTPos,
    pub end_right_gravity: bool,
}

impl MTPair {
    #[inline]
    pub const fn new() -> Self {
        Self {
            start: MTKey {
                pos: MTPos { row: 0, col: 0 },
                ns: 0,
                id: 0,
                flags: 0,
                decor_data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
            },
            end_pos: MTPos { row: 0, col: 0 },
            end_right_gravity: false,
        }
    }
}

impl Default for MTPair {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

pub type Intersection = Vec<u64>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MtnodeInner {
    pub i_ptr: [Option<usize>; 2 * MT_BRANCH_FACTOR],
    pub i_meta: [[u32; K_MTMETA_COUNT]; 2 * MT_BRANCH_FACTOR],
}

#[repr(C)]
pub struct MTNode {
    pub n: i32,
    pub level: i16,
    pub p_idx: i16,
    pub intersect: Intersection,
    pub parent: Option<usize>,
    pub key: [MTKey; 2 * MT_BRANCH_FACTOR - 1],
    pub inner: Option<Box<MtnodeInner>>,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MarkTreeIterState {
    pub oldcol: i32,
    pub i: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MarkTreeIter {
    pub pos: MTPos,
    pub lvl: i32,
    pub x: Option<usize>,
    pub i: i32,
    pub s: [MarkTreeIterState; MT_MAX_DEPTH],
    pub intersect_idx: usize,
    pub intersect_pos: MTPos,
    pub intersect_pos_x: MTPos,
}

impl MarkTreeIter {
    #[inline]
    pub const fn new() -> Self {
        Self {
            pos: MTPos { row: 0, col: 0 },
            lvl: 0,
            x: None,
            i: 0,
            s: [MarkTreeIterState { oldcol: 0, i: 0 }; MT_MAX_DEPTH],
            intersect_idx: 0,
            intersect_pos: MTPos { row: 0, col: 0 },
            intersect_pos_x: MTPos { row: 0, col: 0 },
        }
    }
}

impl Default for MarkTreeIter {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MTDamage {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub old_i: i32,
    pub new_i: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MTDamagePair {
    pub start: MTDamage,
    pub end: MTDamage,
}

pub type PMapUint64 = std::collections::BTreeMap<u64, usize>;
#[cfg(test)]
pub type PMapPtr = std::collections::BTreeMap<usize, Vec<u64>>;
pub type MTDamageMap = std::collections::BTreeMap<u64, MTDamagePair>;

#[repr(C)]
pub struct MarkTree {
    pub root: Option<usize>,
    pub meta_root: [u32; K_MTMETA_COUNT],
    pub n_keys: usize,
    pub n_nodes: usize,
    pub id2node: PMapUint64,
    pub nodes: Vec<MTNode>,
    pub free_nodes: Vec<usize>,
}

impl MarkTree {
    pub const fn new() -> Self {
        Self {
            root: None,
            meta_root: [0; K_MTMETA_COUNT],
            n_keys: 0,
            n_nodes: 0,
            id2node: std::collections::BTreeMap::new(),
            nodes: Vec::new(),
            free_nodes: Vec::new(),
        }
    }

    #[inline]
    pub fn key_mut(&mut self, itr: &MarkTreeIter) -> &mut MTKey {
        &mut self.nodes[itr.x.unwrap()].key[itr.i as usize]
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
fn key_cmp(a: MTKey, b: MTKey) -> i32 {
    let cmp_row = i32::from(a.pos.row > b.pos.row) - i32::from(a.pos.row < b.pos.row);
    if cmp_row != 0 {
        return cmp_row;
    }
    let cmp_col = i32::from(a.pos.col > b.pos.col) - i32::from(a.pos.col < b.pos.col);
    if cmp_col != 0 {
        return cmp_col;
    }
    const CMP_MASK: u16 = MT_FLAG_RIGHT_GRAVITY | MT_FLAG_END | MT_FLAG_REAL | MT_FLAG_LAST;
    let af = a.flags & CMP_MASK;
    let bf = b.flags & CMP_MASK;
    i32::from(af > bf) - i32::from(af < bf)
}

#[inline]
fn node_inner(b: &MarkTree, x: usize) -> &MtnodeInner {
    b.nodes[x]
        .inner
        .as_deref()
        .expect("node_inner called on leaf node")
}

#[inline]
fn node_inner_mut(b: &mut MarkTree, x: usize) -> &mut MtnodeInner {
    b.nodes[x]
        .inner
        .as_deref_mut()
        .expect("node_inner_mut called on leaf node")
}

#[inline]
fn node_ptr(b: &MarkTree, x: usize) -> &[Option<usize>; 2 * MT_BRANCH_FACTOR] {
    &node_inner(b, x).i_ptr
}

#[inline]
fn node_meta(b: &MarkTree, x: usize) -> &[[u32; K_MTMETA_COUNT]; 2 * MT_BRANCH_FACTOR] {
    &node_inner(b, x).i_meta
}

#[inline]
fn node_meta_mut(b: &mut MarkTree, x: usize) -> &mut [[u32; K_MTMETA_COUNT]; 2 * MT_BRANCH_FACTOR] {
    &mut node_inner_mut(b, x).i_meta
}

#[inline]
fn rawkey<'a>(b: &'a MarkTree, itr: &MarkTreeIter) -> &'a MTKey {
    &b.nodes[itr.x.unwrap()].key[itr.i as usize]
}

#[inline]
fn rawkey_mut<'a>(b: &'a mut MarkTree, itr: &MarkTreeIter) -> &'a mut MTKey {
    &mut b.nodes[itr.x.unwrap()].key[itr.i as usize]
}

#[inline]
fn marktree_getp_aux(b: &MarkTree, x: usize, k: MTKey, match_out: Option<&mut bool>) -> i32 {
    let mut dummy_match = false;
    let m = match_out.unwrap_or(&mut dummy_match);

    let xn = b.nodes[x].n;
    if xn == 0 {
        *m = false;
        return -1;
    }
    let mut begin: i32 = 0;
    let mut end: i32 = xn;
    while begin < end {
        let mid = (begin + end) >> 1;
        if key_cmp(b.nodes[x].key[mid as usize], k) < 0 {
            begin = mid + 1;
        } else {
            end = mid;
        }
    }
    if begin == xn {
        *m = false;
        return xn - 1;
    }
    *m = key_cmp(k, b.nodes[x].key[begin as usize]) == 0;
    if !*m {
        begin -= 1;
    }
    begin
}

#[inline]
fn refkey(b: &mut MarkTree, x: usize, i: i32) {
    let key_id = mt_lookup_key(b.nodes[x].key[i as usize]);
    b.id2node.insert(key_id, x);
}

#[inline]
fn id2node(b: &MarkTree, id: u64) -> Option<usize> {
    b.id2node.get(&id).copied()
}

fn split_node(b: &mut MarkTree, x: usize, i: i32, next: MTKey) {
    let y = node_ptr(b, x)[i as usize].unwrap();
    let y_level = b.nodes[y].level;
    let z = marktree_alloc_node(b, y_level != 0);
    b.nodes[z].level = y_level;
    b.nodes[z].n = T - 1;

    let last_start = if mt_end(next) {
        mt_lookup_id(next.ns, next.id, false)
    } else {
        MARKTREE_END_FLAG
    };

    let y_intersect = b.nodes[y].intersect.clone();
    b.nodes[z].intersect = y_intersect;

    if y_level == 0 {
        let pi = pseudo_index(b, y, 0);
        for j in 0..T_USIZE {
            let k = b.nodes[y].key[j];
            let pi_end = pseudo_index_for_id(b, mt_lookup_id(k.ns, k.id, true), true);
            if mt_start(k) && pi_end > pi && mt_lookup_key(k) != last_start {
                intersect_node(b, z, mt_lookup_id(k.ns, k.id, false));
            }
        }

        for j in (T_USIZE - 1)..((T_USIZE * 2) - 1) {
            let k = b.nodes[y].key[j];
            let pi_start = pseudo_index_for_id(b, mt_lookup_id(k.ns, k.id, false), true);
            if mt_end(k) && pi_start > 0 && pi_start < pi {
                intersect_node(b, y, mt_lookup_id(k.ns, k.id, false));
            }
        }
    }

    let keys_copy: [MTKey; T_USIZE - 1] = b.nodes[y].key[T_USIZE..(2 * T_USIZE - 1)]
        .try_into()
        .unwrap();
    b.nodes[z].key[..(T_USIZE - 1)].copy_from_slice(&keys_copy);
    for j in 0..(T - 1) {
        refkey(b, z, j);
    }
    if y_level != 0 {
        let y_ptrs: [Option<usize>; T_USIZE] = node_inner(b, y).i_ptr[T_USIZE..(2 * T_USIZE)]
            .try_into()
            .unwrap();
        let y_metas: [[u32; K_MTMETA_COUNT]; T_USIZE] = node_inner(b, y).i_meta
            [T_USIZE..(2 * T_USIZE)]
            .try_into()
            .unwrap();
        node_inner_mut(b, z).i_ptr[..T_USIZE].copy_from_slice(&y_ptrs);
        node_inner_mut(b, z).i_meta[..T_USIZE].copy_from_slice(&y_metas);
        for (j, &child) in y_ptrs.iter().enumerate() {
            let child = child.unwrap();
            b.nodes[child].parent = Some(z);
            b.nodes[child].p_idx = j as i16;
        }
    }
    b.nodes[y].n = T - 1;
    let i_usize = i as usize;
    let xn = b.nodes[x].n;
    let count_after = (xn - i) as usize;
    let mut z_meta = [0u32; K_MTMETA_COUNT];
    meta_describe_node(b, &mut z_meta, z);
    {
        let x_inner = node_inner_mut(b, x);
        x_inner
            .i_ptr
            .copy_within((i_usize + 1)..(i_usize + 1 + count_after), i_usize + 2);
        x_inner
            .i_meta
            .copy_within((i_usize + 1)..(i_usize + 1 + count_after), i_usize + 2);
        x_inner.i_ptr[i_usize + 1] = Some(z);
        x_inner.i_meta[i_usize + 1] = z_meta;
    }
    b.nodes[z].parent = Some(x);
    for j in (i_usize + 1)..(xn as usize + 2) {
        let child = node_inner(b, x).i_ptr[j].unwrap();
        b.nodes[child].p_idx = j as i16;
    }
    b.nodes[x]
        .key
        .copy_within(i_usize..(i_usize + count_after), i_usize + 1);

    // move key to internal layer:
    b.nodes[x].key[i_usize] = b.nodes[y].key[T_USIZE - 1];
    refkey(b, x, i);
    b.nodes[x].n += 1;

    let meta_inc = meta_describe_key(b.nodes[x].key[i_usize]);
    {
        let x_inner = node_inner_mut(b, x);
        for (m, &inc) in meta_inc.iter().enumerate() {
            x_inner.i_meta[i_usize][m] -= x_inner.i_meta[i_usize + 1][m] + inc;
        }
    }

    let x_key_pos = b.nodes[x].key[i_usize].pos;
    for j in 0..(T_USIZE - 1) {
        relative(x_key_pos, &mut b.nodes[z].key[j].pos);
    }
    if i > 0 {
        let prev_pos = b.nodes[x].key[i_usize - 1].pos;
        unrelative(prev_pos, &mut b.nodes[x].key[i_usize].pos);
    }

    if y_level != 0 {
        bubble_up(b, y);
        bubble_up(b, z);
    }
}

fn marktree_putp_aux(b: &mut MarkTree, x: usize, mut k: MTKey, meta_inc: &[u32; K_MTMETA_COUNT]) {
    let mut i = marktree_getp_aux(b, x, k, None) + 1;
    if b.nodes[x].level == 0 {
        let xn = b.nodes[x].n;
        if i != xn {
            b.nodes[x]
                .key
                .copy_within(i as usize..xn as usize, i as usize + 1);
        }
        b.nodes[x].key[i as usize] = k;
        refkey(b, x, i);
        b.nodes[x].n += 1;
    } else {
        let child0 = node_ptr(b, x)[i as usize].unwrap();
        if b.nodes[child0].n == 2 * T - 1 {
            split_node(b, x, i, k);
            if key_cmp(k, b.nodes[x].key[i as usize]) > 0 {
                i += 1;
            }
        }
        if i > 0 {
            relative(b.nodes[x].key[i as usize - 1].pos, &mut k.pos);
        }
        let child = node_ptr(b, x)[i as usize].unwrap();
        marktree_putp_aux(b, child, k, meta_inc);
        let x_meta = node_meta_mut(b, x);
        for m in 0..K_MTMETA_COUNT {
            x_meta[i as usize][m] += meta_inc[m];
        }
    }
}

pub fn marktree_put(b: &mut MarkTree, mut key: MTKey, end_row: i32, end_col: i32, end_right: bool) {
    debug_assert_eq!(
        key.flags & !(MT_FLAG_EXTERNAL_MASK | MT_FLAG_RIGHT_GRAVITY),
        0
    );
    if end_row >= 0 {
        key.flags |= MT_FLAG_PAIRED;
    }

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
        let mut itr = MarkTreeIter::new();
        let mut end_itr = MarkTreeIter::new();
        marktree_lookup(b, mt_lookup_key(key), Some(&mut itr));
        marktree_lookup(b, mt_lookup_key(end_key), Some(&mut end_itr));

        marktree_intersect_pair(b, mt_lookup_key(key), &mut itr, &end_itr, false);
    }
}

fn intersection_has(x: &Intersection, id: u64) -> bool {
    x.binary_search(&id).is_ok()
}

fn intersect_node(b: &mut MarkTree, x: usize, id: u64) {
    debug_assert_eq!(id & MARKTREE_END_FLAG, 0);
    let pos = b.nodes[x].intersect.partition_point(|&item| item < id);
    b.nodes[x].intersect.insert(pos, id);
}

fn unintersect_node(b: &mut MarkTree, x: usize, id: u64, strict: bool) {
    debug_assert_eq!(id & MARKTREE_END_FLAG, 0);
    match b.nodes[x].intersect.binary_search(&id) {
        Ok(i) => {
            b.nodes[x].intersect.remove(i);
        }
        Err(_) => {
            if strict {
                debug_assert!(false);
            }
        }
    }
}

pub fn marktree_intersect_pair(
    b: &mut MarkTree,
    id: u64,
    itr: &mut MarkTreeIter,
    end_itr: &MarkTreeIter,
    delete: bool,
) {
    let maxlvl = itr.lvl.min(end_itr.lvl);
    let iat = |it: &MarkTreeIter, l: i32, q: i32| -> i32 {
        if l == it.lvl {
            it.i + q
        } else {
            it.s[l as usize].i
        }
    };
    let mut lvl = 0;
    while lvl < maxlvl {
        let si = itr.s[lvl as usize].i;
        let ei = end_itr.s[lvl as usize].i;
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

    while let Some(itr_x) = itr.x {
        let mut skip = false;
        if itr.x == end_itr.x {
            if b.nodes[itr_x].level == 0 || itr.i >= end_itr.i {
                break;
            } else {
                skip = true;
            }
        } else if itr.lvl > lvl || iat(itr, lvl, 1) < iat(end_itr, lvl, 1) {
            skip = true;
        } else {
            lvl += 1;
        }

        if skip && b.nodes[itr_x].level != 0 {
            let x = node_ptr(b, itr_x)[(itr.i + 1) as usize].unwrap();
            if delete {
                unintersect_node(b, x, id, true);
            } else {
                intersect_node(b, x, id);
            }
        }
        marktree_itr_next_skip(b, itr, skip, true, None, None);
    }
}

fn marktree_alloc_node(b: &mut MarkTree, internal: bool) -> usize {
    let inner = if internal {
        Some(Box::new(MtnodeInner {
            i_ptr: [None; 2 * MT_BRANCH_FACTOR],
            i_meta: [[0; K_MTMETA_COUNT]; 2 * MT_BRANCH_FACTOR],
        }))
    } else {
        None
    };
    let node = MTNode {
        n: 0,
        level: 0,
        p_idx: 0,
        intersect: Vec::new(),
        parent: None,
        key: [MT_INVALID_KEY; 2 * MT_BRANCH_FACTOR - 1],
        inner,
    };
    b.n_nodes += 1;
    if let Some(idx) = b.free_nodes.pop() {
        b.nodes[idx] = node;
        idx
    } else {
        let idx = b.nodes.len();
        b.nodes.push(node);
        idx
    }
}

#[inline]
fn meta_describe_key_inc(meta_inc: &mut [u32; K_MTMETA_COUNT], key: MTKey) {
    if !mt_end(key) && !mt_invalid(key) {
        meta_inc[K_MTMETA_INLINE] += u32::from((key.flags & MT_FLAG_DECOR_VIRT_TEXT_INLINE) != 0);
        meta_inc[K_MTMETA_LINES] += u32::from((key.flags & MT_FLAG_DECOR_VIRT_LINES) != 0);
        meta_inc[K_MTMETA_SIGN_HL] += u32::from((key.flags & MT_FLAG_DECOR_SIGNHL) != 0);
        meta_inc[K_MTMETA_SIGN_TEXT] += u32::from((key.flags & MT_FLAG_DECOR_SIGNTEXT) != 0);
        meta_inc[K_MTMETA_CONCEAL_LINES] +=
            u32::from((key.flags & MT_FLAG_DECOR_CONCEAL_LINES) != 0);
    }
}

#[inline]
fn meta_describe_key(k: MTKey) -> [u32; K_MTMETA_COUNT] {
    let mut meta_inc = [0u32; K_MTMETA_COUNT];
    meta_describe_key_inc(&mut meta_inc, k);
    meta_inc
}

fn meta_describe_node(b: &MarkTree, meta_node: &mut [u32; K_MTMETA_COUNT], x: usize) {
    meta_node.fill(0);
    for i in 0..b.nodes[x].n as usize {
        meta_describe_key_inc(meta_node, b.nodes[x].key[i]);
    }
    if b.nodes[x].level != 0 {
        let x_meta = node_meta(b, x);
        for child_meta in &x_meta[..(b.nodes[x].n as usize + 1)] {
            for m in 0..K_MTMETA_COUNT {
                meta_node[m] += child_meta[m];
            }
        }
    }
}

#[inline]
fn meta_has(meta_count: &[u32; K_MTMETA_COUNT], meta_filter: &MetaFilter) -> bool {
    let mut count = 0u32;
    for m in 0..K_MTMETA_COUNT {
        count += meta_count[m] & meta_filter[m];
    }
    count > 0
}

pub fn marktree_put_key(b: &mut MarkTree, mut k: MTKey) {
    k.flags |= MT_FLAG_REAL;
    if b.root.is_none() {
        b.root = Some(marktree_alloc_node(b, true));
    }
    let mut r = b.root.unwrap();
    if b.nodes[r].n == 2 * T - 1 {
        let s = marktree_alloc_node(b, true);
        b.root = Some(s);
        b.nodes[s].level = b.nodes[r].level + 1;
        b.nodes[s].n = 0;
        let meta_root = b.meta_root;
        {
            let s_inner = node_inner_mut(b, s);
            s_inner.i_ptr[0] = Some(r);
            s_inner.i_meta[0] = meta_root;
        }
        b.nodes[r].parent = Some(s);
        b.nodes[r].p_idx = 0;
        split_node(b, s, 0, k);
        r = s;
    }

    let meta_inc = meta_describe_key(k);
    marktree_putp_aux(b, r, k, &meta_inc);
    for (m, &inc) in meta_inc.iter().enumerate() {
        b.meta_root[m] += inc;
    }
    b.n_keys += 1;
}

pub fn marktree_del_itr(b: &mut MarkTree, itr: &mut MarkTreeIter, rev: bool) -> u64 {
    let mut adjustment = 0;

    let cur = itr.x.unwrap();
    let curi = itr.i;
    let id = mt_lookup_key(b.nodes[cur].key[curi as usize]);

    let raw = *rawkey(b, itr);
    let mut other = 0u64;
    if mt_paired(raw) && (raw.flags & MT_FLAG_ORPHANED) == 0 {
        other = mt_lookup_key_side(raw, !mt_end(raw));

        let mut other_itr = MarkTreeIter::new();
        marktree_lookup(b, other, Some(&mut other_itr));
        rawkey_mut(b, &other_itr).flags |= MT_FLAG_ORPHANED;
        if mt_start(raw) {
            let mut this_itr = *itr;
            marktree_intersect_pair(b, id, &mut this_itr, &other_itr, true);
        } else {
            marktree_intersect_pair(b, other, &mut other_itr, itr, true);
        }
    }

    // 2.
    if b.nodes[itr.x.unwrap()].level != 0 {
        if rev {
            std::process::abort();
        } else {
            marktree_itr_prev(b, itr);
            adjustment = -1;
        }
    }

    // 3.
    let mut x = itr.x.unwrap();
    debug_assert_eq!(b.nodes[x].level, 0);
    let mut intkey = b.nodes[x].key[itr.i as usize];

    let mut meta_inc = meta_describe_key(intkey);
    let xn = b.nodes[x].n;
    if xn > itr.i + 1 {
        b.nodes[x]
            .key
            .copy_within((itr.i as usize + 1)..xn as usize, itr.i as usize);
    }
    b.nodes[x].n -= 1;

    b.n_keys -= 1;
    b.id2node.remove(&id);

    // 4.
    if adjustment == -1 {
        let mut ilvl = itr.lvl - 1;
        let mut lnode = x;
        let mut start_id = 0u64;
        let mut did_bubble = false;
        if mt_end(intkey) {
            start_id = mt_lookup_key_side(intkey, false);
        }
        loop {
            let p = b.nodes[lnode].parent.unwrap();
            if ilvl < 0 {
                std::process::abort();
            }
            let i = itr.s[ilvl as usize].i;
            debug_assert_eq!(node_ptr(b, p)[i as usize], Some(lnode));
            if i > 0 {
                unrelative(b.nodes[p].key[i as usize - 1].pos, &mut intkey.pos);
            }

            let c0 = node_ptr(b, p)[0].unwrap();
            if p != cur && start_id != 0 && intersection_has(&b.nodes[c0].intersect, start_id) {
                let last = i32::from(lnode != x);
                let pn = b.nodes[p].n;
                for c_idx in 0..(pn + last) as usize {
                    let child = node_ptr(b, p)[c_idx].unwrap();
                    unintersect_node(b, child, start_id, true);
                }
                intersect_node(b, p, start_id);
                did_bubble = true;
            }

            let lnode_p_idx = b.nodes[lnode].p_idx as usize;
            let p_meta = node_meta_mut(b, p);
            for (m, &inc) in meta_inc.iter().enumerate() {
                p_meta[lnode_p_idx][m] -= inc;
            }

            lnode = p;
            ilvl -= 1;
            if lnode == cur {
                break;
            }
        }

        let mut deleted = b.nodes[cur].key[curi as usize];
        meta_inc = meta_describe_key(deleted);
        b.nodes[cur].key[curi as usize] = intkey;
        refkey(b, cur, curi);
        if mt_end(b.nodes[cur].key[curi as usize]) && !did_bubble {
            let pi = pseudo_index(b, x, 0);
            let pi_start = pseudo_index_for_id(b, start_id, true);
            if pi_start > 0 && pi_start < pi {
                intersect_node(b, x, start_id);
            }
        }

        relative(intkey.pos, &mut deleted.pos);
        let mut y = node_ptr(b, cur)[curi as usize + 1];
        if deleted.pos.row != 0 || deleted.pos.col != 0 {
            while let Some(y_idx) = y {
                for k in 0..b.nodes[y_idx].n as usize {
                    unrelative(deleted.pos, &mut b.nodes[y_idx].key[k].pos);
                }
                y = if b.nodes[y_idx].level != 0 {
                    node_ptr(b, y_idx)[0]
                } else {
                    None
                };
            }
        }
        itr.i -= 1;
    }

    let mut lnode = cur;
    while let Some(parent) = b.nodes[lnode].parent {
        let lnode_p_idx = b.nodes[lnode].p_idx as usize;
        let p_meta = &mut node_meta_mut(b, parent)[lnode_p_idx];
        for (m, &inc) in meta_inc.iter().enumerate() {
            p_meta[m] -= inc;
        }
        lnode = parent;
    }
    for (m, &inc) in meta_inc.iter().enumerate() {
        debug_assert!(b.meta_root[m] >= inc);
        b.meta_root[m] -= inc;
    }

    // 5.
    let mut itr_dirty = false;
    let mut rlvl = itr.lvl - 1;
    let mut lasti_is_itr_i = true;
    let mut lasti_lvl: usize = 0;
    let mut ppos = itr.pos;
    while Some(x) != b.root {
        debug_assert!(rlvl >= 0);
        let p = b.nodes[x].parent.unwrap();
        if b.nodes[x].n >= T - 1 {
            break;
        }
        let pi = itr.s[rlvl as usize].i;
        debug_assert_eq!(node_ptr(b, p)[pi as usize], Some(x));
        if pi > 0 {
            ppos.row -= b.nodes[p].key[pi as usize - 1].pos.row;
            ppos.col = itr.s[rlvl as usize].oldcol;
        }

        let left_n = if pi > 0 {
            b.nodes[node_ptr(b, p)[pi as usize - 1].unwrap()].n
        } else {
            0
        };
        let right_n = if pi < b.nodes[p].n {
            b.nodes[node_ptr(b, p)[pi as usize + 1].unwrap()].n
        } else {
            0
        };

        if pi > 0 && left_n > T - 1 {
            if lasti_is_itr_i {
                itr.i += 1;
            } else {
                itr.s[lasti_lvl].i += 1;
            }
            itr_dirty = true;
            pivot_right(b, ppos, p, pi - 1);
            break;
        } else if pi < b.nodes[p].n && right_n > T - 1 {
            pivot_left(b, ppos, p, pi);
            break;
        } else if pi > 0 {
            debug_assert_eq!(left_n, T - 1);
            if lasti_is_itr_i {
                itr.i += T;
            } else {
                itr.s[lasti_lvl].i += T;
            }
            x = merge_node(b, p, pi - 1);
            if lasti_is_itr_i {
                itr.x = Some(x);
            }
            itr.s[rlvl as usize].i -= 1;
            itr_dirty = true;
        } else {
            debug_assert!(pi < b.nodes[p].n && right_n == T - 1);
            merge_node(b, p, pi);
        }
        lasti_is_itr_i = false;
        lasti_lvl = rlvl as usize;
        rlvl -= 1;
        x = p;
    }

    // 6.
    let root_idx = b.root.unwrap();
    if b.nodes[root_idx].n == 0 {
        if itr.lvl > 0 {
            itr.s.copy_within(1..itr.lvl as usize, 0);
            itr.lvl -= 1;
        }
        if b.nodes[root_idx].level != 0 {
            let oldroot = root_idx;
            let new_root = node_ptr(b, oldroot)[0].unwrap();
            b.root = Some(new_root);
            let oldroot_meta = node_meta(b, oldroot)[0];
            for (m, &meta_val) in oldroot_meta.iter().enumerate() {
                debug_assert_eq!(b.meta_root[m], meta_val);
            }

            b.nodes[new_root].parent = None;
            marktree_free_node(b, oldroot);
        } else {
            itr.x = None;
        }
    }

    if itr.x.is_some() && itr_dirty {
        marktree_itr_fix_pos(b, itr);
    }

    if adjustment == -1 {
        marktree_itr_next(b, itr);
        marktree_itr_next(b, itr);
    } else if let Some(itr_x) = itr.x
        && itr.i >= b.nodes[itr_x].n
    {
        debug_assert_eq!(b.nodes[itr_x].level, 0);
        marktree_itr_next(b, itr);
    }

    other
}

pub fn marktree_revise_meta(b: &mut MarkTree, itr: &MarkTreeIter, old_key: MTKey) {
    let meta_old = meta_describe_key(old_key);
    let meta_new = meta_describe_key(*rawkey(b, itr));

    if meta_old == meta_new {
        return;
    }

    let mut lnode = itr.x.unwrap();
    while let Some(parent) = b.nodes[lnode].parent {
        let p_idx = b.nodes[lnode].p_idx as usize;
        let meta_p = &mut node_meta_mut(b, parent)[p_idx];
        for m in 0..K_MTMETA_COUNT {
            meta_p[m] = meta_p[m]
                .wrapping_add(meta_new[m])
                .wrapping_sub(meta_old[m]);
        }
        lnode = parent;
    }

    for m in 0..K_MTMETA_COUNT {
        b.meta_root[m] = b.meta_root[m]
            .wrapping_add(meta_new[m])
            .wrapping_sub(meta_old[m]);
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
pub fn intersect_mov_test(
    x: &[u64],
    y: &[u64],
    win: &[u64],
    wout: &mut [u64],
    nwout: &mut usize,
    dout: &mut [u64],
    ndout: &mut usize,
) -> bool {
    let xi = x.to_vec();
    let mut yi = y.to_vec();
    let mut w = win.to_vec();
    let mut d = Vec::new();

    intersect_mov(&xi, &mut yi, &mut w, &mut d);

    if w.len() > *nwout || d.len() > *ndout {
        return false;
    }

    wout[..w.len()].copy_from_slice(&w);
    *nwout = w.len();

    dout[..d.len()].copy_from_slice(&d);
    *ndout = d.len();

    true
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

fn bubble_up(b: &mut MarkTree, x: usize) {
    let mut xi = Vec::new();
    let xn = b.nodes[x].n as usize;
    let c0 = node_ptr(b, x)[0].unwrap();
    let cn = node_ptr(b, x)[xn].unwrap();
    intersect_common(&mut xi, &b.nodes[c0].intersect, &b.nodes[cn].intersect);
    if !xi.is_empty() {
        for idx in 0..(xn + 1) {
            let child = node_ptr(b, x)[idx].unwrap();
            intersect_sub(&mut b.nodes[child].intersect, &xi);
        }
        intersect_add(&mut b.nodes[x].intersect, &xi);
    }
}

fn merge_node(b: &mut MarkTree, p: usize, i: i32) -> usize {
    let i_usize = i as usize;
    let x = node_ptr(b, p)[i_usize].unwrap();
    let y = node_ptr(b, p)[i_usize + 1].unwrap();
    let mut mi = Vec::new();

    let mut x_intersect = std::mem::take(&mut b.nodes[x].intersect);
    let mut y_intersect = std::mem::take(&mut b.nodes[y].intersect);
    intersect_merge(&mut mi, &mut x_intersect, &mut y_intersect);
    b.nodes[x].intersect = x_intersect;
    b.nodes[y].intersect = y_intersect;

    let xn = b.nodes[x].n as usize;
    let yn = b.nodes[y].n as usize;
    b.nodes[x].key[xn] = b.nodes[p].key[i_usize];
    refkey(b, x, xn as i32);
    if i > 0 {
        let prev_pos = b.nodes[p].key[i_usize - 1].pos;
        relative(prev_pos, &mut b.nodes[x].key[xn].pos);
    }

    let meta_inc = meta_describe_key(b.nodes[x].key[xn]);

    let y_keys: Vec<MTKey> = b.nodes[y].key[..yn].to_vec();
    b.nodes[x].key[(xn + 1)..(xn + 1 + yn)].copy_from_slice(&y_keys);
    let mid_pos = b.nodes[x].key[xn].pos;
    for k in 0..(yn as i32) {
        refkey(b, x, (xn as i32) + 1 + k);
        unrelative(mid_pos, &mut b.nodes[x].key[xn + 1 + k as usize].pos);
    }
    if b.nodes[x].level != 0 {
        let y_ptrs: Vec<Option<usize>> = node_inner(b, y).i_ptr[..(yn + 1)].to_vec();
        let y_metas: Vec<[u32; K_MTMETA_COUNT]> = node_inner(b, y).i_meta[..(yn + 1)].to_vec();
        node_inner_mut(b, x).i_ptr[(xn + 1)..(xn + 2 + yn)].copy_from_slice(&y_ptrs);
        node_inner_mut(b, x).i_meta[(xn + 1)..(xn + 2 + yn)].copy_from_slice(&y_metas);
        let x_inter = b.nodes[x].intersect.clone();
        for c_idx in 0..(xn + 1) {
            let child = node_ptr(b, x)[c_idx].unwrap();
            for &id in &x_inter {
                intersect_node(b, child, id);
            }
        }
        let y_inter = b.nodes[y].intersect.clone();
        for ky in 0..(yn + 1) {
            let k = xn + ky + 1;
            let child = node_ptr(b, x)[k].unwrap();
            b.nodes[child].parent = Some(x);
            b.nodes[child].p_idx = k as i16;
            for &id in &y_inter {
                intersect_node(b, child, id);
            }
        }
    }
    b.nodes[x].n += (yn as i32) + 1;
    {
        let p_inner = node_inner_mut(b, p);
        for (m, &inc) in meta_inc.iter().enumerate() {
            p_inner.i_meta[i_usize][m] += p_inner.i_meta[i_usize + 1][m] + inc;
        }
    }

    let pn = b.nodes[p].n;
    let rem = (pn - i - 1) as usize;
    b.nodes[p]
        .key
        .copy_within((i_usize + 1)..(i_usize + 1 + rem), i_usize);
    {
        let p_inner = node_inner_mut(b, p);
        p_inner
            .i_ptr
            .copy_within((i_usize + 2)..(i_usize + 2 + rem), i_usize + 1);
        p_inner
            .i_meta
            .copy_within((i_usize + 2)..(i_usize + 2 + rem), i_usize + 1);
    }
    for j in (i_usize + 1)..(pn as usize) {
        let child = node_ptr(b, p)[j].unwrap();
        b.nodes[child].p_idx = j as i16;
    }
    b.nodes[p].n -= 1;
    marktree_free_node(b, y);

    b.nodes[x].intersect = mi;

    x
}

fn pivot_right(b: &mut MarkTree, _p_pos: MTPos, p: usize, i: i32) {
    let i_usize = i as usize;
    let x = node_ptr(b, p)[i_usize].unwrap();
    let y = node_ptr(b, p)[i_usize + 1].unwrap();
    let yn = b.nodes[y].n as usize;
    b.nodes[y].key.copy_within(0..yn, 1);
    if b.nodes[y].level != 0 {
        {
            let y_inner = node_inner_mut(b, y);
            y_inner.i_ptr.copy_within(0..(yn + 1), 1);
            y_inner.i_meta.copy_within(0..(yn + 1), 1);
        }
        for j in 1..(yn + 2) {
            let child = node_ptr(b, y)[j].unwrap();
            b.nodes[child].p_idx = j as i16;
        }
    }

    b.nodes[y].key[0] = b.nodes[p].key[i_usize];
    refkey(b, y, 0);
    let xn_prev = b.nodes[x].n as usize - 1;
    b.nodes[p].key[i_usize] = b.nodes[x].key[xn_prev];
    refkey(b, p, i);

    let meta_inc_y = meta_describe_key(b.nodes[y].key[0]);
    let meta_inc_x = meta_describe_key(b.nodes[p].key[i_usize]);

    {
        let p_inner = node_inner_mut(b, p);
        for m in 0..K_MTMETA_COUNT {
            p_inner.i_meta[i_usize + 1][m] += meta_inc_y[m];
            p_inner.i_meta[i_usize][m] -= meta_inc_x[m];
        }
    }

    if b.nodes[x].level != 0 {
        let xn = b.nodes[x].n as usize;
        let moved_ptr = node_inner(b, x).i_ptr[xn];
        let moved_meta = node_inner(b, x).i_meta[xn];
        {
            let y_inner = node_inner_mut(b, y);
            y_inner.i_ptr[0] = moved_ptr;
            y_inner.i_meta[0] = moved_meta;
        }
        {
            let p_inner = node_inner_mut(b, p);
            for (m, &mm) in moved_meta.iter().enumerate() {
                p_inner.i_meta[i_usize + 1][m] += mm;
                p_inner.i_meta[i_usize][m] -= mm;
            }
        }
        let child0 = moved_ptr.unwrap();
        b.nodes[child0].parent = Some(y);
        b.nodes[child0].p_idx = 0;
    }
    b.nodes[x].n -= 1;
    b.nodes[y].n += 1;
    if i > 0 {
        let prev_pos = b.nodes[p].key[i_usize - 1].pos;
        unrelative(prev_pos, &mut b.nodes[p].key[i_usize].pos);
    }
    let p_key_pos = b.nodes[p].key[i_usize].pos;
    relative(p_key_pos, &mut b.nodes[y].key[0].pos);
    let y0_pos = b.nodes[y].key[0].pos;
    for k in 1..b.nodes[y].n as usize {
        unrelative(y0_pos, &mut b.nodes[y].key[k].pos);
    }

    if b.nodes[x].level != 0 {
        let mut d = Vec::new();
        let y0 = node_ptr(b, y)[0].unwrap();
        let x_inter = b.nodes[x].intersect.clone();
        let mut y_inter = std::mem::take(&mut b.nodes[y].intersect);
        let mut y0_inter = std::mem::take(&mut b.nodes[y0].intersect);
        intersect_mov(&x_inter, &mut y_inter, &mut y0_inter, &mut d);
        b.nodes[y].intersect = y_inter;
        b.nodes[y0].intersect = y0_inter;
        if !d.is_empty() {
            let yn_new = b.nodes[y].n as usize;
            for c_idx in 1..(yn_new + 1) {
                let child = node_ptr(b, y)[c_idx].unwrap();
                intersect_add(&mut b.nodes[child].intersect, &d);
            }
        }

        bubble_up(b, x);
    } else {
        if mt_end(b.nodes[p].key[i_usize]) {
            let pi = pseudo_index(b, x, 0);
            let start_id = mt_lookup_key_side(b.nodes[p].key[i_usize], false);
            let pi_start = pseudo_index_for_id(b, start_id, true);
            if pi_start > 0 && pi_start < pi {
                intersect_node(b, x, start_id);
            }
        }

        if mt_start(b.nodes[y].key[0]) {
            let id = mt_lookup_key(b.nodes[y].key[0]);
            unintersect_node(b, y, id, false);
        }
    }
}

fn pivot_left(b: &mut MarkTree, _p_pos: MTPos, p: usize, i: i32) {
    let i_usize = i as usize;
    let x = node_ptr(b, p)[i_usize].unwrap();
    let y = node_ptr(b, p)[i_usize + 1].unwrap();

    let y0_pos = b.nodes[y].key[0].pos;
    for k in 1..b.nodes[y].n as usize {
        relative(y0_pos, &mut b.nodes[y].key[k].pos);
    }
    let p_key_pos = b.nodes[p].key[i_usize].pos;
    unrelative(p_key_pos, &mut b.nodes[y].key[0].pos);
    if i > 0 {
        let prev_pos = b.nodes[p].key[i_usize - 1].pos;
        relative(prev_pos, &mut b.nodes[p].key[i_usize].pos);
    }

    let xn = b.nodes[x].n as usize;
    let yn = b.nodes[y].n as usize;
    b.nodes[x].key[xn] = b.nodes[p].key[i_usize];
    refkey(b, x, xn as i32);
    b.nodes[p].key[i_usize] = b.nodes[y].key[0];
    refkey(b, p, i);

    let meta_inc_x = meta_describe_key(b.nodes[x].key[xn]);
    let meta_inc_y = meta_describe_key(b.nodes[p].key[i_usize]);
    {
        let p_inner = node_inner_mut(b, p);
        for m in 0..K_MTMETA_COUNT {
            p_inner.i_meta[i_usize][m] += meta_inc_x[m];
            p_inner.i_meta[i_usize + 1][m] -= meta_inc_y[m];
        }
    }

    if b.nodes[x].level != 0 {
        let moved_ptr = node_inner(b, y).i_ptr[0];
        let moved_meta = node_inner(b, y).i_meta[0];
        {
            let x_inner = node_inner_mut(b, x);
            x_inner.i_ptr[xn + 1] = moved_ptr;
            x_inner.i_meta[xn + 1] = moved_meta;
        }
        {
            let p_inner = node_inner_mut(b, p);
            for (m, &mm) in moved_meta.iter().enumerate() {
                p_inner.i_meta[i_usize + 1][m] -= mm;
                p_inner.i_meta[i_usize][m] += mm;
            }
        }
        let child_moved = moved_ptr.unwrap();
        b.nodes[child_moved].parent = Some(x);
        b.nodes[child_moved].p_idx = (xn + 1) as i16;
    }
    b.nodes[y].key.copy_within(1..yn, 0);
    if b.nodes[y].level != 0 {
        {
            let y_inner = node_inner_mut(b, y);
            y_inner.i_ptr.copy_within(1..(yn + 1), 0);
            y_inner.i_meta.copy_within(1..(yn + 1), 0);
        }
        for j in 0..yn {
            let child = node_ptr(b, y)[j].unwrap();
            b.nodes[child].p_idx = j as i16;
        }
    }
    b.nodes[x].n += 1;
    b.nodes[y].n -= 1;

    if b.nodes[x].level != 0 {
        let mut d = Vec::new();
        let xn_new = b.nodes[x].n as usize;
        let xlast = node_ptr(b, x)[xn_new].unwrap();
        let y_inter = b.nodes[y].intersect.clone();
        let mut x_inter = std::mem::take(&mut b.nodes[x].intersect);
        let mut xlast_inter = std::mem::take(&mut b.nodes[xlast].intersect);
        intersect_mov(&y_inter, &mut x_inter, &mut xlast_inter, &mut d);
        b.nodes[x].intersect = x_inter;
        b.nodes[xlast].intersect = xlast_inter;
        if !d.is_empty() {
            for c_idx in 0..xn_new {
                let child = node_ptr(b, x)[c_idx].unwrap();
                intersect_add(&mut b.nodes[child].intersect, &d);
            }
        }

        bubble_up(b, y);
    } else {
        if mt_start(b.nodes[p].key[i_usize]) {
            let pi = pseudo_index(b, y, 0);
            let end_id = mt_lookup_key_side(b.nodes[p].key[i_usize], true);
            let pi_end = pseudo_index_for_id(b, end_id, true);
            if pi_end > pi {
                let start_id = mt_lookup_key(b.nodes[p].key[i_usize]);
                intersect_node(b, y, start_id);
            }
        }

        let xlast_key = b.nodes[x].key[b.nodes[x].n as usize - 1];
        if mt_end(xlast_key) {
            unintersect_node(b, x, mt_lookup_key_side(xlast_key, false), false);
        }
    }
}

pub fn marktree_clear(b: &mut MarkTree) {
    b.root = None;
    b.id2node.clear();
    b.nodes.clear();
    b.free_nodes.clear();
    b.n_keys = 0;
    b.n_nodes = 0;
    b.meta_root = [0; K_MTMETA_COUNT];
}

fn marktree_free_node(b: &mut MarkTree, x: usize) {
    b.nodes[x].intersect.clear();
    b.nodes[x].inner = None;
    b.nodes[x].parent = None;
    b.free_nodes.push(x);
    b.n_nodes -= 1;
}

pub fn marktree_move(b: &mut MarkTree, itr: &mut MarkTreeIter, row: i32, col: i32) {
    let mut key = *rawkey(b, itr);
    let x = itr.x.unwrap();
    if b.nodes[x].level == 0 {
        let mut internal = false;
        let mut newpos = MTPos { row, col };
        if b.nodes[x].parent.is_some() {
            if pos_less(itr.pos, newpos) {
                relative(itr.pos, &mut newpos);
                if pos_less(newpos, b.nodes[x].key[b.nodes[x].n as usize - 1].pos) {
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
            let mut new_i = marktree_getp_aux(b, x, key, Some(&mut matched));
            if !matched {
                new_i += 1;
            }
            if new_i == itr.i {
                b.nodes[x].key[itr.i as usize].pos = newpos;
            } else if new_i < itr.i {
                b.nodes[x]
                    .key
                    .copy_within(new_i as usize..itr.i as usize, new_i as usize + 1);
                b.nodes[x].key[new_i as usize] = key;
            } else {
                b.nodes[x]
                    .key
                    .copy_within((itr.i as usize + 1)..new_i as usize, itr.i as usize);
                b.nodes[x].key[new_i as usize - 1] = key;
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
    itr.x = None;
}

pub fn marktree_restore_pair(b: &mut MarkTree, key: MTKey) {
    let mut itr = MarkTreeIter::new();
    let mut end_itr = MarkTreeIter::new();
    marktree_lookup(b, mt_lookup_key_side(key, false), Some(&mut itr));
    marktree_lookup(b, mt_lookup_key_side(key, true), Some(&mut end_itr));
    if itr.x.is_none() || end_itr.x.is_none() {
        return;
    }
    rawkey_mut(b, &itr).flags &= !MT_FLAG_ORPHANED;
    rawkey_mut(b, &end_itr).flags &= !MT_FLAG_ORPHANED;

    marktree_intersect_pair(b, mt_lookup_key_side(key, false), &mut itr, &end_itr, false);
}

pub fn marktree_itr_get(b: &MarkTree, row: i32, col: i32, itr: &mut MarkTreeIter) -> bool {
    marktree_itr_get_ext(b, MTPos { row, col }, itr, false, false, None, None)
}

pub fn marktree_itr_get_ext(
    b: &MarkTree,
    p: MTPos,
    itr: &mut MarkTreeIter,
    last: bool,
    gravity: bool,
    mut oldbase: Option<&mut [MTPos; MT_MAX_DEPTH]>,
    meta_filter: Option<&MetaFilter>,
) -> bool {
    if b.n_keys == 0 {
        itr.x = None;
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
        decor_data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
    };
    itr.pos = MTPos { row: 0, col: 0 };
    itr.x = b.root;
    itr.lvl = 0;
    if let Some(ref mut ob) = oldbase {
        ob[itr.lvl as usize] = itr.pos;
    }
    loop {
        let itr_x = itr.x.unwrap();
        itr.i = marktree_getp_aux(b, itr_x, k, None) + 1;

        if b.nodes[itr_x].level == 0 {
            break;
        }
        if let Some(mf) = meta_filter
            && !meta_has(&node_meta(b, itr_x)[itr.i as usize], mf)
        {
            break;
        }

        let lvl = itr.lvl as usize;
        itr.s[lvl].i = itr.i;
        itr.s[lvl].oldcol = itr.pos.col;

        if itr.i > 0 {
            let prev_pos = b.nodes[itr_x].key[itr.i as usize - 1].pos;
            compose(&mut itr.pos, prev_pos);
            relative(prev_pos, &mut k.pos);
        }
        itr.x = node_ptr(b, itr_x)[itr.i as usize];
        itr.lvl += 1;
        if let Some(ref mut ob) = oldbase {
            ob[itr.lvl as usize] = itr.pos;
        }
    }

    if last {
        marktree_itr_prev(b, itr)
    } else if itr.i >= b.nodes[itr.x.unwrap()].n {
        marktree_itr_next_skip(b, itr, true, false, None, None)
    } else {
        true
    }
}

#[cfg(test)]
pub fn marktree_itr_first(b: &MarkTree, itr: &mut MarkTreeIter) -> bool {
    if b.n_keys == 0 {
        itr.x = None;
        return false;
    }

    itr.x = b.root;
    itr.i = 0;
    itr.lvl = 0;
    itr.pos = MTPos { row: 0, col: 0 };
    while b.nodes[itr.x.unwrap()].level > 0 {
        let lvl = itr.lvl as usize;
        itr.s[lvl].i = 0;
        itr.s[lvl].oldcol = 0;
        itr.lvl += 1;
        itr.x = node_ptr(b, itr.x.unwrap())[0];
    }
    true
}

pub fn marktree_itr_next(b: &MarkTree, itr: &mut MarkTreeIter) -> bool {
    marktree_itr_next_skip(b, itr, false, false, None, None)
}

fn marktree_itr_next_skip(
    b: &MarkTree,
    itr: &mut MarkTreeIter,
    mut skip: bool,
    preload: bool,
    mut oldbase: Option<&mut [MTPos; MT_MAX_DEPTH]>,
    meta_filter: Option<&MetaFilter>,
) -> bool {
    let Some(mut itr_x) = itr.x else {
        return false;
    };
    itr.i += 1;
    if let Some(mf) = meta_filter
        && b.nodes[itr_x].level > 0
        && !meta_has(&node_meta(b, itr_x)[itr.i as usize], mf)
    {
        skip = true;
    }
    if b.nodes[itr_x].level == 0 || skip {
        if preload && b.nodes[itr_x].level == 0 && skip {
            itr.i = b.nodes[itr_x].n;
        } else if itr.i < b.nodes[itr_x].n {
            return true;
        }
        while itr.i >= b.nodes[itr_x].n {
            let Some(parent) = b.nodes[itr_x].parent else {
                itr.x = None;
                return false;
            };
            itr.x = Some(parent);
            itr_x = parent;
            itr.lvl -= 1;
            let lvl = itr.lvl as usize;
            itr.i = itr.s[lvl].i;
            if itr.i > 0 {
                itr.pos.row -= b.nodes[itr_x].key[itr.i as usize - 1].pos.row;
                itr.pos.col = itr.s[lvl].oldcol;
            }
        }
    } else {
        while b.nodes[itr_x].level > 0 {
            let lvl = itr.lvl as usize;
            if itr.i > 0 {
                itr.s[lvl].oldcol = itr.pos.col;
                compose(&mut itr.pos, b.nodes[itr_x].key[itr.i as usize - 1].pos);
            }
            if let Some(ref mut ob) = oldbase
                && itr.i == 0
            {
                ob[lvl + 1] = ob[lvl];
            }
            itr.s[lvl].i = itr.i;
            let child = node_ptr(b, itr_x)[itr.i as usize].unwrap();
            debug_assert_eq!(b.nodes[child].parent, Some(itr_x));
            itr.lvl += 1;
            itr.x = Some(child);
            itr_x = child;
            if preload && b.nodes[itr_x].level != 0 {
                itr.i = -1;
                break;
            }
            itr.i = 0;
            if let Some(mf) = meta_filter
                && b.nodes[itr_x].level != 0
                && !meta_has(&node_meta(b, itr_x)[0], mf)
            {
                break;
            }
        }
    }
    true
}

pub fn marktree_itr_get_filter(
    b: &MarkTree,
    row: i32,
    col: i32,
    stop_row: i32,
    stop_col: i32,
    meta_filter: &MetaFilter,
    itr: &mut MarkTreeIter,
) -> bool {
    if !meta_has(&b.meta_root, meta_filter) {
        return false;
    }
    if !marktree_itr_get_ext(
        b,
        MTPos { row, col },
        itr,
        false,
        false,
        None,
        Some(meta_filter),
    ) {
        return false;
    }
    marktree_itr_check_filter(b, itr, stop_row, stop_col, meta_filter)
}

pub fn marktree_itr_step_out_filter(
    b: &MarkTree,
    itr: &mut MarkTreeIter,
    meta_filter: &MetaFilter,
) -> bool {
    if !meta_has(&b.meta_root, meta_filter) {
        itr.x = None;
        return false;
    }

    while let Some(itr_x) = itr.x
        && let Some(parent) = b.nodes[itr_x].parent
    {
        let p_idx = b.nodes[itr_x].p_idx as usize;
        if meta_has(&node_meta(b, parent)[p_idx], meta_filter) {
            return true;
        }

        itr.i = b.nodes[itr_x].n;
        marktree_itr_next_skip(b, itr, true, false, None, None);
    }

    itr.x.is_some()
}

pub fn marktree_itr_next_filter(
    b: &MarkTree,
    itr: &mut MarkTreeIter,
    stop_row: i32,
    stop_col: i32,
    meta_filter: &MetaFilter,
) -> bool {
    if !marktree_itr_next_skip(b, itr, false, false, None, Some(meta_filter)) {
        return false;
    }
    marktree_itr_check_filter(b, itr, stop_row, stop_col, meta_filter)
}

fn marktree_itr_check_filter(
    b: &MarkTree,
    itr: &mut MarkTreeIter,
    stop_row: i32,
    stop_col: i32,
    meta_filter: &MetaFilter,
) -> bool {
    let stop_pos = MTPos {
        row: stop_row,
        col: stop_col,
    };

    let mut key_filter = 0u32;
    for (m, &mm) in META_MAP.iter().enumerate() {
        key_filter |= mm & meta_filter[m];
    }

    loop {
        if pos_leq(stop_pos, marktree_itr_pos(b, itr)) {
            itr.x = None;
            return false;
        }

        let k = *rawkey(b, itr);
        if !mt_end(k) && (u32::from(k.flags) & key_filter) != 0 {
            return true;
        }

        if !marktree_itr_next_skip(b, itr, false, false, None, Some(meta_filter)) {
            return false;
        }
    }
}

pub fn marktree_itr_prev(b: &MarkTree, itr: &mut MarkTreeIter) -> bool {
    let Some(mut itr_x) = itr.x else {
        return false;
    };
    if b.nodes[itr_x].level == 0 {
        itr.i -= 1;
        if itr.i >= 0 {
            return true;
        }
        while itr.i < 0 {
            let Some(parent) = b.nodes[itr_x].parent else {
                itr.x = None;
                return false;
            };
            itr.x = Some(parent);
            itr_x = parent;
            itr.lvl -= 1;
            let lvl = itr.lvl as usize;
            itr.i = itr.s[lvl].i - 1;
            if itr.i >= 0 {
                itr.pos.row -= b.nodes[itr_x].key[itr.i as usize].pos.row;
                itr.pos.col = itr.s[lvl].oldcol;
            }
        }
    } else {
        while b.nodes[itr_x].level > 0 {
            let lvl = itr.lvl as usize;
            if itr.i > 0 {
                itr.s[lvl].oldcol = itr.pos.col;
                compose(&mut itr.pos, b.nodes[itr_x].key[itr.i as usize - 1].pos);
            }
            itr.s[lvl].i = itr.i;
            let child = node_ptr(b, itr_x)[itr.i as usize].unwrap();
            debug_assert_eq!(b.nodes[child].parent, Some(itr_x));
            itr.x = Some(child);
            itr_x = child;
            itr.i = b.nodes[itr_x].n;
            itr.lvl += 1;
        }
        itr.i -= 1;
    }
    true
}

pub fn marktree_itr_pos(b: &MarkTree, itr: &MarkTreeIter) -> MTPos {
    let mut pos = rawkey(b, itr).pos;
    unrelative(itr.pos, &mut pos);
    pos
}

pub fn marktree_itr_current(b: &MarkTree, itr: &MarkTreeIter) -> MTKey {
    if itr.x.is_some() {
        let mut key = *rawkey(b, itr);
        key.pos = marktree_itr_pos(b, itr);
        key
    } else {
        MT_INVALID_KEY
    }
}

#[inline]
fn itr_eq(itr1: &MarkTreeIter, itr2: &MarkTreeIter) -> bool {
    itr1.x == itr2.x && itr1.i == itr2.i
}

pub fn marktree_itr_get_overlap(b: &MarkTree, row: i32, col: i32, itr: &mut MarkTreeIter) -> bool {
    if b.n_keys == 0 {
        itr.x = None;
        return false;
    }

    itr.x = b.root;
    itr.i = -1;
    itr.lvl = 0;
    itr.pos = MTPos { row: 0, col: 0 };
    itr.intersect_pos = MTPos { row, col };
    itr.intersect_pos_x = MTPos { row, col };
    itr.intersect_idx = 0;
    true
}

pub fn marktree_itr_step_overlap(b: &MarkTree, itr: &mut MarkTreeIter, pair: &mut MTPair) -> bool {
    while itr.i == -1 {
        let itr_x = itr.x.unwrap();
        if itr.intersect_idx < b.nodes[itr_x].intersect.len() {
            let id = b.nodes[itr_x].intersect.as_slice()[itr.intersect_idx];
            itr.intersect_idx += 1;
            *pair = mtpair_from(
                marktree_lookup(b, id, None),
                marktree_lookup(b, id | MARKTREE_END_FLAG, None),
            );
            return true;
        }

        if b.nodes[itr_x].level == 0 {
            itr.i = 0;
            itr.s[itr.lvl as usize].i = 0;
            break;
        }

        let k = MTKey {
            pos: itr.intersect_pos_x,
            ns: 0,
            id: 0,
            flags: 0,
            decor_data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
        };
        itr.i = marktree_getp_aux(b, itr_x, k, None) + 1;

        let lvl = itr.lvl as usize;
        itr.s[lvl].i = itr.i;
        itr.s[lvl].oldcol = itr.pos.col;

        if itr.i > 0 {
            let prev_pos = b.nodes[itr_x].key[itr.i as usize - 1].pos;
            compose(&mut itr.pos, prev_pos);
            relative(prev_pos, &mut itr.intersect_pos_x);
        }
        itr.x = node_ptr(b, itr_x)[itr.i as usize];
        itr.lvl += 1;
        itr.i = -1;
        itr.intersect_idx = 0;
    }

    let itr_x = itr.x.unwrap();
    while itr.i < b.nodes[itr_x].n && pos_less(rawkey(b, itr).pos, itr.intersect_pos_x) {
        let mut k = b.nodes[itr_x].key[itr.i as usize];
        itr.i += 1;
        itr.s[itr.lvl as usize].i = itr.i;
        if mt_start(k) {
            let end = marktree_lookup(b, mt_lookup_id(k.ns, k.id, true), None);
            if pos_less(end.pos, itr.intersect_pos) {
                continue;
            }

            unrelative(itr.pos, &mut k.pos);
            *pair = mtpair_from(k, end);
            return true;
        }
    }

    while itr.i < b.nodes[itr_x].n {
        let mut k = b.nodes[itr_x].key[itr.i as usize];
        itr.i += 1;
        if mt_end(k) {
            let id = mt_lookup_id(k.ns, k.id, false);
            if id2node(b, id) == itr.x {
                continue;
            }
            unrelative(itr.pos, &mut k.pos);
            let start = marktree_lookup(b, id, None);
            if pos_leq(itr.intersect_pos, start.pos) {
                continue;
            }
            *pair = mtpair_from(start, k);
            return true;
        }
    }

    itr.i = itr.s[itr.lvl as usize].i;
    debug_assert!(itr.i >= 0);
    if itr.i >= b.nodes[itr_x].n {
        marktree_itr_next(b, itr);
    }

    false
}

fn check_damage(b: &MarkTree, damage: &mut MTDamageMap, itr1: &MarkTreeIter, itr2: &MarkTreeIter) {
    let k1 = *rawkey(b, itr1);
    let start_id = mt_lookup_key_side(k1, false);
    let p = damage.entry(start_id).or_default();
    let me: &mut MTDamage = if mt_end(k1) { &mut p.end } else { &mut p.start };
    debug_assert!(me.new.is_none());
    *me = MTDamage {
        old: itr1.x,
        new: itr2.x,
        old_i: itr1.i,
        new_i: itr2.i,
    };
}

fn swap_keys(
    b: &mut MarkTree,
    itr1: &mut MarkTreeIter,
    itr2: &mut MarkTreeIter,
    damage: &mut MTDamageMap,
) {
    let x1_idx = itr1.x.unwrap();
    let x2_idx = itr2.x.unwrap();
    if b.nodes[x1_idx].level != 0 || x1_idx != x2_idx {
        if mt_paired(*rawkey(b, itr1)) {
            check_damage(b, damage, itr1, itr2);
        }
        if mt_paired(*rawkey(b, itr2)) {
            check_damage(b, damage, itr2, itr1);
        }
    }

    if x1_idx != x2_idx {
        let meta_inc_1 = meta_describe_key(*rawkey(b, itr1));
        let meta_inc_2 = meta_describe_key(*rawkey(b, itr2));

        if meta_inc_1 != meta_inc_2 {
            let mut x1 = x1_idx;
            let mut x2 = x2_idx;
            while x1 != x2 {
                if b.nodes[x1].level <= b.nodes[x2].level {
                    let p1 = b.nodes[x1].parent.unwrap();
                    let p1_idx = b.nodes[x1].p_idx as usize;
                    let meta_node = &mut node_meta_mut(b, p1)[p1_idx];
                    for m in 0..K_MTMETA_COUNT {
                        meta_node[m] = meta_node[m]
                            .wrapping_add(meta_inc_2[m])
                            .wrapping_sub(meta_inc_1[m]);
                    }
                    x1 = p1;
                }
                if b.nodes[x2].level < b.nodes[x1].level {
                    let p2 = b.nodes[x2].parent.unwrap();
                    let p2_idx = b.nodes[x2].p_idx as usize;
                    let meta_node = &mut node_meta_mut(b, p2)[p2_idx];
                    for m in 0..K_MTMETA_COUNT {
                        meta_node[m] = meta_node[m]
                            .wrapping_add(meta_inc_1[m])
                            .wrapping_sub(meta_inc_2[m]);
                    }
                    x2 = p2;
                }
            }
        }
    }

    let key1 = *rawkey(b, itr1);
    let key2 = *rawkey(b, itr2);
    *rawkey_mut(b, itr1) = key2;
    rawkey_mut(b, itr1).pos = key1.pos;
    *rawkey_mut(b, itr2) = key1;
    rawkey_mut(b, itr2).pos = key2.pos;
    refkey(b, x1_idx, itr1.i);
    refkey(b, x2_idx, itr2.i);
}

pub fn marktree_splice(
    b: &mut MarkTree,
    start_line: i32,
    start_col: i32,
    old_extent_line: i32,
    old_extent_col: i32,
    new_extent_line: i32,
    new_extent_col: i32,
) -> bool {
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
    let mut itr = MarkTreeIter::new();
    let mut enditr = MarkTreeIter::new();

    let mut oldbase = [MTPos { row: 0, col: 0 }; MT_MAX_DEPTH];

    marktree_itr_get_ext(b, start, &mut itr, false, true, Some(&mut oldbase), None);
    if itr.x.is_none() {
        return false;
    }
    let delta = MTPos {
        row: new_extent.row - old_extent.row,
        col: new_extent.col - old_extent.col,
    };

    if may_delete {
        let ipos = marktree_itr_pos(b, &itr);
        if !pos_leq(old_extent, ipos)
            || (old_extent.row == ipos.row
                && old_extent.col == ipos.col
                && !mt_right(*rawkey(b, &itr)))
        {
            marktree_itr_get_ext(b, old_extent, &mut enditr, true, true, None, None);
            debug_assert!(enditr.x.is_some());
        } else {
            may_delete = false;
        }
    }

    let mut past_right = false;
    let mut moved = false;
    let mut damage: MTDamageMap = MTDamageMap::new();

    if may_delete {
        'outer1: while itr.x.is_some() && !past_right {
            let mut loc_start = start;
            let mut loc_old = old_extent;
            relative(itr.pos, &mut loc_start);
            relative(oldbase[itr.lvl as usize], &mut loc_old);

            loop {
                if !pos_leq(rawkey(b, &itr).pos, loc_old) {
                    break 'outer1;
                }

                if mt_right(*rawkey(b, &itr)) {
                    while !itr_eq(&itr, &enditr) && mt_right(*rawkey(b, &enditr)) {
                        marktree_itr_prev(b, &mut enditr);
                    }
                    if !mt_right(*rawkey(b, &enditr)) {
                        swap_keys(b, &mut itr, &mut enditr, &mut damage);
                    } else {
                        break 'outer1;
                    }
                }

                if itr_eq(&itr, &enditr) {
                    past_right = true;
                }

                moved = true;
                let itr_x = itr.x.unwrap();
                if b.nodes[itr_x].level != 0 {
                    let lvl = itr.lvl as usize;
                    oldbase[lvl + 1] = rawkey(b, &itr).pos;
                    let base_lvl = oldbase[lvl];
                    unrelative(base_lvl, &mut oldbase[lvl + 1]);
                    rawkey_mut(b, &itr).pos = loc_start;
                    marktree_itr_next_skip(b, &mut itr, false, false, Some(&mut oldbase), None);
                    break;
                } else {
                    rawkey_mut(b, &itr).pos = loc_start;
                    if itr.i < b.nodes[itr_x].n - 1 {
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
        'outer2: while itr.x.is_some() {
            let mut loc_new = new_extent;
            relative(itr.pos, &mut loc_new);
            let mut limit = old_extent;
            relative(oldbase[itr.lvl as usize], &mut limit);

            loop {
                if pos_leq(limit, rawkey(b, &itr).pos) {
                    break 'outer2;
                }

                let oldpos = rawkey(b, &itr).pos;
                rawkey_mut(b, &itr).pos = loc_new;
                moved = true;
                let itr_x = itr.x.unwrap();
                if b.nodes[itr_x].level != 0 {
                    let lvl = itr.lvl as usize;
                    oldbase[lvl + 1] = oldpos;
                    let base_lvl = oldbase[lvl];
                    unrelative(base_lvl, &mut oldbase[lvl + 1]);

                    marktree_itr_next_skip(b, &mut itr, false, false, Some(&mut oldbase), None);
                    break;
                } else if itr.i < b.nodes[itr_x].n - 1 {
                    itr.i += 1;
                } else {
                    marktree_itr_next(b, &mut itr);
                    break;
                }
            }
        }
    }

    while itr.x.is_some() {
        unrelative(oldbase[itr.lvl as usize], &mut rawkey_mut(b, &itr).pos);
        let realrow = rawkey(b, &itr).pos.row;
        debug_assert!(realrow >= old_extent.row);
        let mut done = false;
        if realrow == old_extent.row {
            if delta.col != 0 {
                rawkey_mut(b, &itr).pos.col += delta.col;
            }
        } else if same_line {
            done = true;
        }
        if delta.row != 0 {
            rawkey_mut(b, &itr).pos.row += delta.row;
            moved = true;
        }
        let itr_pos = itr.pos;
        relative(itr_pos, &mut rawkey_mut(b, &itr).pos);
        if done {
            break;
        }
        marktree_itr_next_skip(b, &mut itr, true, false, None, None);
    }

    for (start_id, d) in damage {
        if let (Some(s_old), Some(e_old)) = (d.start.old, d.end.old) {
            marktree_itr_set_node(b, Some(&mut itr), s_old, d.start.old_i);
            marktree_itr_set_node(b, Some(&mut enditr), e_old, d.end.old_i);
            marktree_intersect_pair(b, start_id, &mut itr, &enditr, true);
            marktree_itr_set_node(b, Some(&mut itr), d.start.new.unwrap(), d.start.new_i);
            marktree_itr_set_node(b, Some(&mut enditr), d.end.new.unwrap(), d.end.new_i);
            marktree_intersect_pair(b, start_id, &mut itr, &enditr, false);
        } else if let Some(s_old) = d.start.old {
            let mut endpos = MarkTreeIter::new();
            marktree_lookup(b, start_id | MARKTREE_END_FLAG, Some(&mut endpos));
            if endpos.x.is_some() {
                marktree_itr_set_node(b, Some(&mut itr), s_old, d.start.old_i);
                enditr = endpos;
                marktree_intersect_pair(b, start_id, &mut itr, &enditr, true);
                marktree_itr_set_node(b, Some(&mut itr), d.start.new.unwrap(), d.start.new_i);
                enditr = endpos;
                marktree_intersect_pair(b, start_id, &mut itr, &enditr, false);
            }
        } else if let Some(e_old) = d.end.old {
            let mut startpos = MarkTreeIter::new();
            marktree_lookup(b, start_id, Some(&mut startpos));
            if startpos.x.is_some() {
                itr = startpos;
                marktree_itr_set_node(b, Some(&mut enditr), e_old, d.end.old_i);
                marktree_intersect_pair(b, start_id, &mut itr, &enditr, true);
                itr = startpos;
                marktree_itr_set_node(b, Some(&mut enditr), d.end.new.unwrap(), d.end.new_i);
                marktree_intersect_pair(b, start_id, &mut itr, &enditr, false);
            }
        }
    }

    moved
}

pub fn marktree_move_region(
    b: &mut MarkTree,
    start_row: i32,
    start_col: Colnr,
    extent_row: i32,
    extent_col: Colnr,
    new_row: i32,
    new_col: Colnr,
) {
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
    let mut itr = MarkTreeIter::new();
    marktree_itr_get_ext(b, start, &mut itr, false, true, None, None);
    let mut saved: Vec<MTKey> = Vec::new();
    while itr.x.is_some() {
        let mut k = marktree_itr_current(b, &itr);
        if !pos_leq(k.pos, end) || (k.pos.row == end.row && k.pos.col == end.col && mt_right(k)) {
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

pub fn marktree_lookup_ns(
    b: &MarkTree,
    ns: u32,
    id: u32,
    end: bool,
    itr: Option<&mut MarkTreeIter>,
) -> MTKey {
    marktree_lookup(b, mt_lookup_id(ns, id, end), itr)
}

fn pseudo_index(b: &MarkTree, x: usize, mut i: i32) -> u64 {
    let mut off = MT_LOG2_BRANCH * i32::from(b.nodes[x].level);
    let mut index = 0u64;

    let mut cur = Some(x);
    while let Some(cur_idx) = cur {
        index |= ((i + 1) as u64) << off;
        off += MT_LOG2_BRANCH;
        i = i32::from(b.nodes[cur_idx].p_idx);
        cur = b.nodes[cur_idx].parent;
    }

    index
}

fn pseudo_index_for_id(b: &MarkTree, id: u64, sloppy: bool) -> u64 {
    let Some(n) = id2node(b, id) else {
        return 0;
    };

    let mut i = 0;
    if b.nodes[n].level != 0 || !sloppy {
        while i < b.nodes[n].n {
            if mt_lookup_key(b.nodes[n].key[i as usize]) == id {
                break;
            }
            i += 1;
        }
        debug_assert!(i < b.nodes[n].n);
        if b.nodes[n].level != 0 {
            i += 1;
        }
    }

    pseudo_index(b, n, i)
}

pub fn marktree_lookup(b: &MarkTree, id: u64, itr: Option<&mut MarkTreeIter>) -> MTKey {
    let Some(n) = id2node(b, id) else {
        if let Some(itr) = itr {
            itr.x = None;
        }
        return MT_INVALID_KEY;
    };
    for i in 0..b.nodes[n].n {
        if mt_lookup_key(b.nodes[n].key[i as usize]) == id {
            return marktree_itr_set_node(b, itr, n, i);
        }
    }

    std::process::abort();
}

pub fn marktree_itr_set_node(
    b: &MarkTree,
    mut itr: Option<&mut MarkTreeIter>,
    mut n: usize,
    mut i: i32,
) -> MTKey {
    let root_level = b.nodes[b.root.unwrap()].level;
    let mut key = b.nodes[n].key[i as usize];
    if let Some(ref mut itr) = itr {
        itr.i = i;
        itr.x = Some(n);
        itr.lvl = i32::from(root_level - b.nodes[n].level);
    }
    while let Some(p) = b.nodes[n].parent {
        i = i32::from(b.nodes[n].p_idx);
        debug_assert_eq!(node_ptr(b, p)[i as usize], Some(n));

        if let Some(ref mut itr) = itr {
            itr.s[(root_level - b.nodes[p].level) as usize].i = i;
        }
        if i > 0 {
            unrelative(b.nodes[p].key[i as usize - 1].pos, &mut key.pos);
        }
        n = p;
    }
    if let Some(itr) = itr {
        marktree_itr_fix_pos(b, itr);
    }
    key
}

pub fn marktree_get_altpos(b: &MarkTree, mark: MTKey, itr: Option<&mut MarkTreeIter>) -> MTPos {
    marktree_get_alt(b, mark, itr).pos
}

pub fn marktree_get_alt(b: &MarkTree, mark: MTKey, itr: Option<&mut MarkTreeIter>) -> MTKey {
    if mt_paired(mark) {
        marktree_lookup_ns(b, mark.ns, mark.id, !mt_end(mark), itr)
    } else {
        mark
    }
}

fn marktree_itr_fix_pos(b: &MarkTree, itr: &mut MarkTreeIter) {
    itr.pos = MTPos { row: 0, col: 0 };
    let mut x = b.root.unwrap();
    for lvl in 0..itr.lvl as usize {
        itr.s[lvl].oldcol = itr.pos.col;
        let i = itr.s[lvl].i;
        if i > 0 {
            compose(&mut itr.pos, b.nodes[x].key[i as usize - 1].pos);
        }
        debug_assert!(b.nodes[x].level != 0);
        x = node_ptr(b, x)[i as usize].unwrap();
    }
    debug_assert_eq!(Some(x), itr.x);
}

#[cfg(test)]
pub fn marktree_put_test(
    b: &mut MarkTree,
    ns: u32,
    id: u32,
    row: i32,
    col: i32,
    right_gravity: bool,
    end_row: i32,
    end_col: i32,
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
        decor_data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
    };
    marktree_put(b, key, end_row, end_col, end_right);
}

#[cfg(test)]
pub fn marktree_del_pair_test(b: &mut MarkTree, ns: u32, id: u32) {
    let mut itr = MarkTreeIter::new();
    marktree_lookup_ns(b, ns, id, false, Some(&mut itr));

    let other = marktree_del_itr(b, &mut itr, false);
    assert!(other != 0);
    marktree_lookup(b, other, Some(&mut itr));
    marktree_del_itr(b, &mut itr, false);
}

#[cfg(test)]
pub fn marktree_check(b: &MarkTree) {
    let Some(root) = b.root else {
        assert_eq!(b.n_keys, 0);
        assert_eq!(b.n_nodes, 0);
        assert_eq!(b.id2node.len(), 0);
        return;
    };

    let mut dummy = MTPos { row: 0, col: 0 };
    let mut last_right = false;

    let nkeys = marktree_check_node(b, root, &mut dummy, &mut last_right, &b.meta_root);
    assert_eq!(b.n_keys, nkeys);
    assert_eq!(b.n_keys, b.id2node.len());
}

#[cfg(test)]
pub fn marktree_check_node(
    b: &MarkTree,
    x: usize,
    last: &mut MTPos,
    last_right: &mut bool,
    meta_node_ref: &[u32; K_MTMETA_COUNT],
) -> usize {
    assert!(b.nodes[x].n < 2 * T);
    assert!(b.nodes[x].n >= if Some(x) != b.root { T - 1 } else { 0 });
    let mut n_keys = b.nodes[x].n as usize;

    for i in 0..b.nodes[x].n as usize {
        if b.nodes[x].level != 0 {
            let child = node_ptr(b, x)[i].unwrap();
            let child_meta = node_meta(b, x)[i];
            n_keys += marktree_check_node(b, child, last, last_right, &child_meta);
        } else {
            *last = MTPos { row: 0, col: 0 };
        }
        if i > 0 {
            unrelative(b.nodes[x].key[i - 1].pos, last);
        }
        assert!(pos_leq(*last, b.nodes[x].key[i].pos));
        if last.row == b.nodes[x].key[i].pos.row && last.col == b.nodes[x].key[i].pos.col {
            assert!(!*last_right || mt_right(b.nodes[x].key[i]));
        }
        *last_right = mt_right(b.nodes[x].key[i]);
        assert!(b.nodes[x].key[i].pos.col >= 0);
        assert_eq!(id2node(b, mt_lookup_key(b.nodes[x].key[i])), Some(x));
    }

    if b.nodes[x].level != 0 {
        let xn = b.nodes[x].n as usize;
        let child = node_ptr(b, x)[xn].unwrap();
        let child_meta = node_meta(b, x)[xn];
        n_keys += marktree_check_node(b, child, last, last_right, &child_meta);
        unrelative(b.nodes[x].key[xn - 1].pos, last);

        let x_ptr = node_ptr(b, x);
        for i in 0..(xn + 1) {
            let child = x_ptr[i].unwrap();
            assert_eq!(b.nodes[child].parent, Some(x));
            assert_eq!(b.nodes[child].p_idx as usize, i);
            assert_eq!(b.nodes[child].level, b.nodes[x].level - 1);
            for prev in &x_ptr[..i] {
                assert_ne!(Some(child), *prev);
            }
        }
    } else if b.nodes[x].n > 0 {
        *last = b.nodes[x].key[b.nodes[x].n as usize - 1].pos;
    }

    let mut meta_node = [0u32; K_MTMETA_COUNT];
    meta_describe_node(b, &mut meta_node, x);
    assert_eq!(&meta_node, meta_node_ref);

    n_keys
}

#[cfg(test)]
pub fn marktree_check_intersections(b: &mut MarkTree) -> bool {
    let Some(root) = b.root else {
        return true;
    };
    let mut checked: PMapPtr = PMapPtr::new();

    // 1. move x->intersect to checked[x] and reinit x->intersect
    mt_recurse_nodes(b, root, &mut checked);

    // 2. iterate over all marks. for each START mark of a pair,
    // intersect the nodes between the pair
    let mut itr = MarkTreeIter::new();
    marktree_itr_first(b, &mut itr);
    loop {
        let mark = marktree_itr_current(b, &itr);
        if mark.pos.row < 0 {
            break;
        }

        if mt_start(mark) {
            let mut start_itr = itr;
            let mut end_itr = MarkTreeIter::new();
            let end_id = mt_lookup_id(mark.ns, mark.id, true);
            let k = marktree_lookup(b, end_id, Some(&mut end_itr));
            if k.pos.row >= 0 {
                marktree_intersect_pair(b, mt_lookup_key(mark), &mut start_itr, &end_itr, false);
            }
        }

        marktree_itr_next(b, &mut itr);
    }

    // 3. for each node check if the recreated intersection
    // matches the old checked[x] intersection.
    mt_recurse_nodes_compare(b, root, &checked)
}

#[cfg(test)]
pub fn mt_recurse_nodes(b: &mut MarkTree, x: usize, checked: &mut PMapPtr) {
    if !b.nodes[x].intersect.is_empty() {
        checked.insert(x, std::mem::take(&mut b.nodes[x].intersect));
    }

    if b.nodes[x].level != 0 {
        let xn = b.nodes[x].n as usize;
        for c_idx in 0..(xn + 1) {
            let child = node_ptr(b, x)[c_idx].unwrap();
            mt_recurse_nodes(b, child, checked);
        }
    }
}

#[cfg(test)]
pub fn mt_recurse_nodes_compare(b: &MarkTree, x: usize, checked: &PMapPtr) -> bool {
    let expected = checked.get(&x).map_or(&[][..], |v| v.as_slice());
    if b.nodes[x].intersect.as_slice() != expected {
        return false;
    }

    if b.nodes[x].level != 0 {
        let x_ptr = node_ptr(b, x);
        for &child in &x_ptr[..(b.nodes[x].n as usize + 1)] {
            if !mt_recurse_nodes_compare(b, child.unwrap(), checked) {
                return false;
            }
        }
    }

    true
}

pub fn mt_inspect(b: &MarkTree, keys: bool, dot: bool) -> NvimString {
    let mut out = String::new();
    let p = MTPos { row: 0, col: 0 };
    if let Some(root) = b.root {
        if dot {
            out.push_str("digraph D {\n\n");
            mt_inspect_dotfile_node(b, &mut out, root, p, None);
            out.push_str("\n}");
        } else {
            mt_inspect_node(b, &mut out, keys, root, p);
        }
    }
    cbuf_to_string(out.as_ptr().cast(), out.len())
}

#[inline]
const fn mt_dbg_id(id: u64) -> u64 {
    (id >> 1) & 0xffffffff
}

fn mt_inspect_node(b: &MarkTree, out: &mut String, keys: bool, n: usize, off: MTPos) {
    out.push('[');
    if keys && !b.nodes[n].intersect.is_empty() {
        for (i, &id) in b.nodes[n].intersect.iter().enumerate() {
            out.push_str(if i == 0 { "{" } else { ";" });
            let _ = write!(out, "{}", mt_dbg_id(id));
        }
        out.push_str("},");
    }
    if b.nodes[n].level != 0 {
        mt_inspect_node(b, out, keys, node_ptr(b, n)[0].unwrap(), off);
    }
    for i in 0..b.nodes[n].n as usize {
        let mut p = b.nodes[n].key[i].pos;
        unrelative(off, &mut p);
        let _ = write!(out, "{}/{}", p.row, p.col);
        if keys {
            let key = b.nodes[n].key[i];
            out.push(':');
            if mt_start(key) {
                out.push('<');
            }
            let _ = write!(out, "{}", key.id);
            if mt_end(key) {
                out.push('>');
            }
        }
        if b.nodes[n].level != 0 {
            mt_inspect_node(b, out, keys, node_ptr(b, n)[i + 1].unwrap(), p);
        } else {
            out.push(',');
        }
    }
    out.push(']');
}

fn mt_inspect_dotfile_node(
    b: &MarkTree,
    out: &mut String,
    n: usize,
    off: MTPos,
    parent: Option<&str>,
) {
    let name_str = if let Some(p_str) = parent {
        format!(
            "{}_{}{}",
            p_str,
            (b'a' + b.nodes[n].level as u8) as char,
            b.nodes[n].p_idx
        )
    } else {
        "MTNode".to_string()
    };

    let _ = writeln!(out, "  {name_str}[shape=plaintext, label=<");
    out.push_str("    <table border='0' cellborder='1' cellspacing='0'>\n");
    if !b.nodes[n].intersect.is_empty() {
        out.push_str("    <tr><td>");
        for (i, &id) in b.nodes[n].intersect.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{}", mt_dbg_id(id));
        }
        out.push_str("</td></tr>\n");
    }

    out.push_str("    <tr><td>");
    for i in 0..b.nodes[n].n as usize {
        let k = b.nodes[n].key[i];
        if i > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{}", k.id);
        if mt_paired(k) {
            out.push_str(if mt_end(k) { "e" } else { "s" });
        }
    }
    out.push_str("</td></tr>\n    </table>\n>];\n");
    if let Some(p_str) = parent {
        let _ = writeln!(out, "  {p_str} -> {name_str}");
    }
    if b.nodes[n].level != 0 {
        mt_inspect_dotfile_node(b, out, node_ptr(b, n)[0].unwrap(), off, Some(&name_str));
    }
    for i in 0..b.nodes[n].n as usize {
        let mut p = b.nodes[n].key[i].pos;
        unrelative(off, &mut p);
        if b.nodes[n].level != 0 {
            mt_inspect_dotfile_node(b, out, node_ptr(b, n)[i + 1].unwrap(), p, Some(&name_str));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marktree_put_splice_lookup_and_check() {
        let mut tree = MarkTree::new();
        for i in 1..=30u32 {
            marktree_put_test(
                &mut tree,
                1,
                i,
                (i / 3) as i32,
                (i % 5) as i32,
                i % 2 == 0,
                (i / 3 + 1) as i32,
                (i % 5 + 2) as i32,
                true,
                i % 3 == 0,
            );
        }
        marktree_check(&tree);
        assert!(marktree_check_intersections(&mut tree));

        marktree_splice(&mut tree, 2, 1, 1, 2, 3, 4);
        marktree_check(&tree);
        assert!(marktree_check_intersections(&mut tree));

        marktree_del_pair_test(&mut tree, 1, 5);
        marktree_check(&tree);
        assert!(marktree_check_intersections(&mut tree));

        marktree_clear(&mut tree);
        marktree_check(&tree);

        let mut id = 0u32;
        for i in 1..=50 {
            for j in 1..=50 {
                id += 1;
                marktree_put_test(&mut tree, 10, id, j, i, (i % 2) > 0, -1, -1, false, false);
            }
        }
        marktree_check(&tree);
        marktree_splice(&mut tree, 2, 2, 0, 5, 1, 2);
        marktree_check(&tree);
        marktree_splice(&mut tree, 30, 2, 15, 5, 1, 2);
        marktree_check(&tree);
        marktree_clear(&mut tree);
        marktree_check(&tree);

        // intersect_mov_test
        let x = [1u64, 3, 4, 6, 7, 9];
        let y = [2u64, 3, 5, 6, 8, 9];
        let w = [0u64, 2, 5, 8, 10];
        let mut wout = [0u64; 17];
        let mut dout = [0u64; 17];
        let mut nwout = wout.len();
        let mut ndout = dout.len();
        assert!(intersect_mov_test(
            &x, &y, &w, &mut wout, &mut nwout, &mut dout, &mut ndout,
        ));
        assert_eq!(&wout[..nwout], &[0, 1, 4, 7, 10]);
        assert_eq!(ndout, 0);

        // Edge case splicing overlapping ranges (#37867)
        let mut nid = 0u32;
        let mut put_range = |t: &mut MarkTree, r1: i32, c1: i32, g1: bool, r2: i32, c2: i32| {
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
    }
}
