//! Histogram diff algorithm (`src/xdiff/xhistogram.c`).

use crate::xdiff::xprepare::xdl_prepare_env;
use crate::xdiff::xutils::{xdl_fall_back_diff, xdl_hashbits, xdl_recmatch};
use crate::xdiff::{Mmfile, XDF_DIFF_ALGORITHM_MASK, Xdfenv, Xpparam, Xrecord, xdl_hashlong};

const MAX_PTR: u32 = i32::MAX as u32;
const MAX_CNT: u32 = i32::MAX as u32;

#[repr(C)]
#[derive(Copy, Clone)]
struct Record {
    ptr: u32,
    cnt: u32,
    next: Option<usize>,
}

struct HistIndex {
    records: Vec<Option<usize>>,
    line_map: Vec<Option<usize>>,
    rcha: Vec<Record>,
    next_ptrs: Vec<u32>,
    table_bits: u32,
    max_chain_length: u32,
    ptr_shift: u32,
    cnt: u32,
    has_common: u32,
    env: *mut Xdfenv,
    xpp: *const Xpparam,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct Region {
    begin1: u32,
    end1: u32,
    begin2: u32,
    end2: u32,
}

#[inline]
fn get_rec(env: &Xdfenv, side: i32, line: u32) -> Xrecord {
    if side == 1 {
        env.xdf1.recs[(line as usize) - 1]
    } else {
        env.xdf2.recs[(line as usize) - 1]
    }
}

#[inline]
fn cmp_recs(xpp: &Xpparam, r1: &Xrecord, r2: &Xrecord) -> bool {
    r1.ha == r2.ha && xdl_recmatch(r1.ptr, r1.size, r2.ptr, r2.size, xpp.flags as i64) != 0
}

#[inline]
fn cmp_index(index: &HistIndex, s1: i32, l1: u32, s2: i32, l2: u32) -> bool {
    let env = unsafe { &*index.env };
    let xpp = unsafe { &*index.xpp };
    let r1 = get_rec(env, s1, l1);
    let r2 = get_rec(env, s2, l2);
    cmp_recs(xpp, &r1, &r2)
}

#[inline]
fn table_hash(index: &HistIndex, side: i32, line: u32) -> usize {
    let rec = get_rec(unsafe { &*index.env }, side, line);
    xdl_hashlong(rec.ha, index.table_bits) as usize
}

#[inline]
fn line_map_idx(index: &HistIndex, ptr: u32) -> usize {
    (ptr - index.ptr_shift) as usize
}

fn scan_a(index: &mut HistIndex, line1: i32, count1: i32) -> i32 {
    let mut ptr = line1 + count1 - 1;
    while line1 <= ptr {
        let uptr = ptr as u32;
        let tbl_idx = table_hash(index, 1, uptr);
        let mut rec_idx = index.records[tbl_idx];

        let mut chain_len: u32 = 0;
        let mut matched = false;
        while let Some(ri) = rec_idx {
            let r_ptr = index.rcha[ri].ptr;
            if cmp_index(index, 1, r_ptr, 1, uptr) {
                let lm_idx = line_map_idx(index, uptr);
                index.next_ptrs[lm_idx] = r_ptr;
                let rec = &mut index.rcha[ri];
                rec.ptr = uptr;
                rec.cnt = MAX_CNT.min(rec.cnt + 1);
                index.line_map[lm_idx] = Some(ri);
                matched = true;
                break;
            }
            rec_idx = index.rcha[ri].next;
            chain_len += 1;
        }

        if !matched {
            if chain_len == index.max_chain_length {
                return -1;
            }

            let new_ri = index.rcha.len();
            index.rcha.push(Record {
                ptr: uptr,
                cnt: 1,
                next: index.records[tbl_idx],
            });
            index.records[tbl_idx] = Some(new_ri);
            let lm_idx = line_map_idx(index, uptr);
            index.line_map[lm_idx] = Some(new_ri);
        }

        ptr -= 1;
    }
    0
}

fn try_lcs(
    index: &mut HistIndex,
    lcs: &mut Region,
    b_ptr: i32,
    line1: i32,
    count1: i32,
    line2: i32,
    count2: i32,
) -> i32 {
    let mut b_next = (b_ptr + 1) as u32;
    let ub_ptr = b_ptr as u32;
    let line_end1 = line1 + count1 - 1;
    let line_end2 = line2 + count2 - 1;

    let mut rec_idx = index.records[table_hash(index, 2, ub_ptr)];
    while let Some(ri) = rec_idx {
        let rec = index.rcha[ri];
        if rec.cnt > index.cnt {
            if index.has_common == 0 {
                index.has_common = u32::from(cmp_index(index, 1, rec.ptr, 2, ub_ptr));
            }
            rec_idx = rec.next;
            continue;
        }

        let mut as_ = rec.ptr;
        if !cmp_index(index, 1, as_, 2, ub_ptr) {
            rec_idx = rec.next;
            continue;
        }

        index.has_common = 1;
        loop {
            let mut should_break = false;
            let mut np = index.next_ptrs[line_map_idx(index, as_)];
            let mut bs = ub_ptr;
            let mut ae = as_;
            let mut be = bs;
            let mut rc = rec.cnt;

            while line1 < (as_ as i32)
                && line2 < (bs as i32)
                && cmp_index(index, 1, as_ - 1, 2, bs - 1)
            {
                as_ -= 1;
                bs -= 1;
                if 1 < rc {
                    let lmi = index.line_map[line_map_idx(index, as_)].unwrap();
                    rc = rc.min(index.rcha[lmi].cnt);
                }
            }
            while (ae as i32) < line_end1
                && (be as i32) < line_end2
                && cmp_index(index, 1, ae + 1, 2, be + 1)
            {
                ae += 1;
                be += 1;
                if 1 < rc {
                    let lmi = index.line_map[line_map_idx(index, ae)].unwrap();
                    rc = rc.min(index.rcha[lmi].cnt);
                }
            }

            if b_next <= be {
                b_next = be + 1;
            }
            if lcs.end1 - lcs.begin1 < ae - as_ || rc < index.cnt {
                lcs.begin1 = as_;
                lcs.begin2 = bs;
                lcs.end1 = ae;
                lcs.end2 = be;
                index.cnt = rc;
            }

            if np == 0 {
                break;
            }

            while np <= ae {
                np = index.next_ptrs[line_map_idx(index, np)];
                if np == 0 {
                    should_break = true;
                    break;
                }
            }

            if should_break {
                break;
            }

            as_ = np;
        }

        rec_idx = rec.next;
    }

    b_next as i32
}

fn fall_back_to_classic_diff(
    xpp: &Xpparam,
    env: &mut Xdfenv,
    line1: i32,
    count1: i32,
    line2: i32,
    count2: i32,
) -> i32 {
    let xpparam = Xpparam {
        flags: xpp.flags & !XDF_DIFF_ALGORITHM_MASK,
    };
    xdl_fall_back_diff(env, &xpparam, line1, count1, line2, count2)
}

fn find_lcs(
    xpp: &Xpparam,
    env: &mut Xdfenv,
    lcs: &mut Region,
    line1: i32,
    count1: i32,
    line2: i32,
    count2: i32,
) -> i32 {
    let table_bits = xdl_hashbits(count1 as u32);
    let records_size = 1usize << table_bits;
    let line_map_size = count1 as usize;
    let mut index = HistIndex {
        records: vec![None; records_size],
        line_map: vec![None; line_map_size],
        rcha: Vec::with_capacity((count1 / 4 + 1).max(0) as usize),
        next_ptrs: vec![0; line_map_size],
        table_bits,
        max_chain_length: 64,
        ptr_shift: line1 as u32,
        cnt: 65,
        has_common: 0,
        env,
        xpp,
    };

    if scan_a(&mut index, line1, count1) != 0 {
        return -1;
    }

    let mut b_ptr = line2;
    let line_end2 = line2 + count2 - 1;
    while b_ptr <= line_end2 {
        b_ptr = try_lcs(&mut index, lcs, b_ptr, line1, count1, line2, count2);
    }

    if index.has_common != 0 && index.max_chain_length < index.cnt {
        1
    } else {
        0
    }
}

fn histogram_diff(
    xpp: &Xpparam,
    env: &mut Xdfenv,
    mut line1: i32,
    mut count1: i32,
    mut line2: i32,
    mut count2: i32,
) -> i32 {
    unsafe {
        loop {
            if count1 <= 0 && count2 <= 0 {
                return 0;
            }

            if ((line1 + count1 - 1) as u32) >= MAX_PTR {
                return -1;
            }

            if count1 == 0 {
                while count2 > 0 {
                    count2 -= 1;
                    *env.xdf2.rchg.wrapping_offset((line2 - 1) as isize) = 1;
                    line2 += 1;
                }
                return 0;
            } else if count2 == 0 {
                while count1 > 0 {
                    count1 -= 1;
                    *env.xdf1.rchg.wrapping_offset((line1 - 1) as isize) = 1;
                    line1 += 1;
                }
                return 0;
            }

            let mut lcs = Region::default();
            let lcs_found = find_lcs(xpp, env, &mut lcs, line1, count1, line2, count2);
            if lcs_found < 0 {
                return -1;
            } else if lcs_found != 0 {
                return fall_back_to_classic_diff(xpp, env, line1, count1, line2, count2);
            } else if lcs.begin1 == 0 && lcs.begin2 == 0 {
                while count1 > 0 {
                    count1 -= 1;
                    *env.xdf1.rchg.wrapping_offset((line1 - 1) as isize) = 1;
                    line1 += 1;
                }
                while count2 > 0 {
                    count2 -= 1;
                    *env.xdf2.rchg.wrapping_offset((line2 - 1) as isize) = 1;
                    line2 += 1;
                }
                return 0;
            } else {
                let res = histogram_diff(
                    xpp,
                    env,
                    line1,
                    (lcs.begin1 as i32) - line1,
                    line2,
                    (lcs.begin2 as i32) - line2,
                );
                if res != 0 {
                    return res;
                }
                count1 = (line1 + count1 - 1) - (lcs.end1 as i32);
                line1 = (lcs.end1 as i32) + 1;
                count2 = (line2 + count2 - 1) - (lcs.end2 as i32);
                line2 = (lcs.end2 as i32) + 1;
            }
        }
    }
}

pub fn xdl_do_histogram_diff(
    file1: &mut Mmfile,
    file2: &mut Mmfile,
    xpp: &Xpparam,
    env: &mut Xdfenv,
) -> i32 {
    if xdl_prepare_env(file1, file2, xpp, env) < 0 {
        return -1;
    }

    histogram_diff(
        xpp,
        env,
        (env.xdf1.dstart + 1) as i32,
        (env.xdf1.dend - env.xdf1.dstart + 1) as i32,
        (env.xdf2.dstart + 1) as i32,
        (env.xdf2.dend - env.xdf2.dstart + 1) as i32,
    )
}
