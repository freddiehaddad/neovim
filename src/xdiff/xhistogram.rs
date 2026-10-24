//! Histogram diff algorithm (`src/xdiff/xhistogram.c`).

use std::ffi::{c_int, c_long, c_uint};
use std::mem::size_of;
use std::ptr;

use crate::xdiff::xprepare::xdl_prepare_env;
use crate::xdiff::xutils::{
    xdl_cha_alloc, xdl_cha_free, xdl_cha_init, xdl_fall_back_diff, xdl_hashbits, xdl_recmatch,
};
use crate::xdiff::{
    XDF_DIFF_ALGORITHM_MASK, chastore_t, mmfile_t, xdfenv_t, xdl_hashlong, xpparam_t, xrecord_t,
};

const MAX_PTR: c_uint = c_int::MAX as c_uint;
const MAX_CNT: c_uint = c_int::MAX as c_uint;

#[repr(C)]
#[derive(Copy, Clone)]
struct Record {
    ptr: c_uint,
    cnt: c_uint,
    next: *mut Record,
}

struct HistIndex {
    records: Vec<*mut Record>,
    line_map: Vec<*mut Record>,
    rcha: chastore_t,
    next_ptrs: Vec<c_uint>,
    table_bits: c_uint,
    max_chain_length: c_uint,
    ptr_shift: c_uint,
    cnt: c_uint,
    has_common: c_uint,
    env: *mut xdfenv_t,
    xpp: *const xpparam_t,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct Region {
    begin1: c_uint,
    end1: c_uint,
    begin2: c_uint,
    end2: c_uint,
}

#[inline]
unsafe fn get_rec(env: *mut xdfenv_t, side: c_int, line: c_uint) -> *mut xrecord_t {
    unsafe {
        if side == 1 {
            *(*env).xdf1.recs.offset((line as isize) - 1)
        } else {
            *(*env).xdf2.recs.offset((line as isize) - 1)
        }
    }
}

#[inline]
unsafe fn cmp_recs(xpp: *const xpparam_t, r1: *mut xrecord_t, r2: *mut xrecord_t) -> bool {
    unsafe {
        (*r1).ha == (*r2).ha
            && xdl_recmatch(
                (*r1).ptr,
                (*r1).size,
                (*r2).ptr,
                (*r2).size,
                (*xpp).flags as c_long,
            ) != 0
    }
}

#[inline]
unsafe fn cmp_index(index: &HistIndex, s1: c_int, l1: c_uint, s2: c_int, l2: c_uint) -> bool {
    unsafe {
        cmp_recs(
            index.xpp,
            get_rec(index.env, s1, l1),
            get_rec(index.env, s2, l2),
        )
    }
}

#[inline]
unsafe fn table_hash(index: &HistIndex, side: c_int, line: c_uint) -> usize {
    unsafe {
        let rec = get_rec(index.env, side, line);
        xdl_hashlong((*rec).ha, index.table_bits) as usize
    }
}

#[inline]
fn line_map_idx(index: &HistIndex, ptr: c_uint) -> usize {
    (ptr - index.ptr_shift) as usize
}

unsafe fn scan_a(index: &mut HistIndex, line1: c_int, count1: c_int) -> c_int {
    let mut ptr = line1 + count1 - 1;
    unsafe {
        while line1 <= ptr {
            let uptr = ptr as c_uint;
            let tbl_idx = table_hash(index, 1, uptr);
            let mut rec = index.records[tbl_idx];

            let mut chain_len: c_uint = 0;
            let mut matched = false;
            while !rec.is_null() {
                if cmp_index(index, 1, (*rec).ptr, 1, uptr) {
                    let lm_idx = line_map_idx(index, uptr);
                    index.next_ptrs[lm_idx] = (*rec).ptr;
                    (*rec).ptr = uptr;
                    (*rec).cnt = MAX_CNT.min((*rec).cnt + 1);
                    index.line_map[lm_idx] = rec;
                    matched = true;
                    break;
                }
                rec = (*rec).next;
                chain_len += 1;
            }

            if !matched {
                if chain_len == index.max_chain_length {
                    return -1;
                }

                rec = xdl_cha_alloc(&mut index.rcha).cast::<Record>();
                if rec.is_null() {
                    return -1;
                }
                (*rec).ptr = uptr;
                (*rec).cnt = 1;
                (*rec).next = index.records[tbl_idx];
                index.records[tbl_idx] = rec;
                let lm_idx = line_map_idx(index, uptr);
                index.line_map[lm_idx] = rec;
            }

            ptr -= 1;
        }
    }
    0
}

unsafe fn try_lcs(
    index: &mut HistIndex,
    lcs: *mut Region,
    b_ptr: c_int,
    line1: c_int,
    count1: c_int,
    line2: c_int,
    count2: c_int,
) -> c_int {
    let mut b_next = (b_ptr + 1) as c_uint;
    let ub_ptr = b_ptr as c_uint;
    let line_end1 = line1 + count1 - 1;
    let line_end2 = line2 + count2 - 1;

    unsafe {
        let mut rec = index.records[table_hash(index, 2, ub_ptr)];
        while !rec.is_null() {
            if (*rec).cnt > index.cnt {
                if index.has_common == 0 {
                    index.has_common = c_uint::from(cmp_index(index, 1, (*rec).ptr, 2, ub_ptr));
                }
                rec = (*rec).next;
                continue;
            }

            let mut as_ = (*rec).ptr;
            if !cmp_index(index, 1, as_, 2, ub_ptr) {
                rec = (*rec).next;
                continue;
            }

            index.has_common = 1;
            loop {
                let mut should_break = false;
                let mut np = index.next_ptrs[line_map_idx(index, as_)];
                let mut bs = ub_ptr;
                let mut ae = as_;
                let mut be = bs;
                let mut rc = (*rec).cnt;

                while line1 < (as_ as c_int)
                    && line2 < (bs as c_int)
                    && cmp_index(index, 1, as_ - 1, 2, bs - 1)
                {
                    as_ -= 1;
                    bs -= 1;
                    if 1 < rc {
                        rc = rc.min((*index.line_map[line_map_idx(index, as_)]).cnt);
                    }
                }
                while (ae as c_int) < line_end1
                    && (be as c_int) < line_end2
                    && cmp_index(index, 1, ae + 1, 2, be + 1)
                {
                    ae += 1;
                    be += 1;
                    if 1 < rc {
                        rc = rc.min((*index.line_map[line_map_idx(index, ae)]).cnt);
                    }
                }

                if b_next <= be {
                    b_next = be + 1;
                }
                if (*lcs).end1 - (*lcs).begin1 < ae - as_ || rc < index.cnt {
                    (*lcs).begin1 = as_;
                    (*lcs).begin2 = bs;
                    (*lcs).end1 = ae;
                    (*lcs).end2 = be;
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

            rec = (*rec).next;
        }
    }

    b_next as c_int
}

unsafe fn fall_back_to_classic_diff(
    xpp: *const xpparam_t,
    env: *mut xdfenv_t,
    line1: c_int,
    count1: c_int,
    line2: c_int,
    count2: c_int,
) -> c_int {
    unsafe {
        let xpparam = xpparam_t {
            flags: (*xpp).flags & !XDF_DIFF_ALGORITHM_MASK,
        };
        xdl_fall_back_diff(env, &xpparam, line1, count1, line2, count2)
    }
}

unsafe fn find_lcs(
    xpp: *const xpparam_t,
    env: *mut xdfenv_t,
    lcs: *mut Region,
    line1: c_int,
    count1: c_int,
    line2: c_int,
    count2: c_int,
) -> c_int {
    let table_bits = xdl_hashbits(count1 as c_uint);
    let records_size = 1usize << table_bits;
    let line_map_size = count1 as usize;
    let mut index = HistIndex {
        records: vec![ptr::null_mut(); records_size],
        line_map: vec![ptr::null_mut(); line_map_size],
        rcha: chastore_t::new(),
        next_ptrs: vec![0; line_map_size],
        table_bits,
        max_chain_length: 64,
        ptr_shift: line1 as c_uint,
        cnt: 65,
        has_common: 0,
        env,
        xpp,
    };

    unsafe {
        if xdl_cha_init(
            &mut index.rcha,
            size_of::<Record>() as c_long,
            (count1 / 4 + 1) as c_long,
        ) < 0
        {
            return -1;
        }

        if scan_a(&mut index, line1, count1) != 0 {
            xdl_cha_free(&mut index.rcha);
            return -1;
        }

        let mut b_ptr = line2;
        let line_end2 = line2 + count2 - 1;
        while b_ptr <= line_end2 {
            b_ptr = try_lcs(&mut index, lcs, b_ptr, line1, count1, line2, count2);
        }

        let ret = if index.has_common != 0 && index.max_chain_length < index.cnt {
            1
        } else {
            0
        };
        xdl_cha_free(&mut index.rcha);
        ret
    }
}

unsafe fn histogram_diff(
    xpp: *const xpparam_t,
    env: *mut xdfenv_t,
    mut line1: c_int,
    mut count1: c_int,
    mut line2: c_int,
    mut count2: c_int,
) -> c_int {
    unsafe {
        loop {
            if count1 <= 0 && count2 <= 0 {
                return 0;
            }

            if ((line1 + count1 - 1) as c_uint) >= MAX_PTR {
                return -1;
            }

            if count1 == 0 {
                while count2 > 0 {
                    count2 -= 1;
                    *(*env).xdf2.rchg.offset((line2 - 1) as isize) = 1;
                    line2 += 1;
                }
                return 0;
            } else if count2 == 0 {
                while count1 > 0 {
                    count1 -= 1;
                    *(*env).xdf1.rchg.offset((line1 - 1) as isize) = 1;
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
                    *(*env).xdf1.rchg.offset((line1 - 1) as isize) = 1;
                    line1 += 1;
                }
                while count2 > 0 {
                    count2 -= 1;
                    *(*env).xdf2.rchg.offset((line2 - 1) as isize) = 1;
                    line2 += 1;
                }
                return 0;
            } else {
                let res = histogram_diff(
                    xpp,
                    env,
                    line1,
                    (lcs.begin1 as c_int) - line1,
                    line2,
                    (lcs.begin2 as c_int) - line2,
                );
                if res != 0 {
                    return res;
                }
                count1 = (line1 + count1 - 1) - (lcs.end1 as c_int);
                line1 = (lcs.end1 as c_int) + 1;
                count2 = (line2 + count2 - 1) - (lcs.end2 as c_int);
                line2 = (lcs.end2 as c_int) + 1;
            }
        }
    }
}

pub unsafe fn xdl_do_histogram_diff(
    file1: *mut mmfile_t,
    file2: *mut mmfile_t,
    xpp: *const xpparam_t,
    env: *mut xdfenv_t,
) -> c_int {
    unsafe {
        if xdl_prepare_env(file1, file2, xpp, env) < 0 {
            return -1;
        }

        histogram_diff(
            xpp,
            env,
            ((*env).xdf1.dstart + 1) as c_int,
            ((*env).xdf1.dend - (*env).xdf1.dstart + 1) as c_int,
            ((*env).xdf2.dstart + 1) as c_int,
            ((*env).xdf2.dend - (*env).xdf2.dstart + 1) as c_int,
        )
    }
}
