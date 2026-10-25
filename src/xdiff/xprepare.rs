//! XDiff environment preparation (`src/xdiff/xprepare.c`).

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong};
use std::pin::Pin;
use std::ptr;

use crate::xdiff::xutils::{
    xdl_bogosqrt, xdl_guess_lines, xdl_hash_record, xdl_hashbits, xdl_mmfile_first, xdl_recmatch,
};
use crate::xdiff::{
    XDF_HISTOGRAM_DIFF, XDF_PATIENCE_DIFF, mmfile_t, xdf_diff_alg, xdfenv_t, xdfile_t,
    xdl_hashlong, xpparam_t, xrecord_t,
};

const XDL_KPDIS_RUN: c_long = 4;
const XDL_MAX_EQLIMIT: c_long = 1024;
const XDL_SIMSCAN_WINDOW: c_long = 100;
const XDL_GUESS_NLINES1: c_long = 256;
const XDL_GUESS_NLINES2: c_long = 20;

#[repr(C)]
#[derive(Copy, Clone)]
struct xdlclass_t {
    next: *mut xdlclass_t,
    ha: c_ulong,
    line: *const c_char,
    size: c_long,
    idx: c_long,
    len1: c_long,
    len2: c_long,
}

struct xdlclassifier_t {
    hbits: c_uint,
    rchash: Vec<*mut xdlclass_t>,
    rcrecs: Vec<Pin<Box<xdlclass_t>>>,
    flags: c_long,
}

fn xdl_init_classifier(cf: &mut xdlclassifier_t, size: c_long, flags: c_long) -> c_int {
    cf.flags = flags;
    cf.hbits = xdl_hashbits(size as c_uint);
    let hsize = 1usize << cf.hbits;
    cf.rchash = vec![ptr::null_mut(); hsize];
    cf.rcrecs = Vec::with_capacity(size as usize);
    0
}

unsafe fn xdl_classify_record(
    pass: c_uint,
    cf: &mut xdlclassifier_t,
    rhash: *mut *mut xrecord_t,
    hbits: c_uint,
    rec: *mut xrecord_t,
) -> c_int {
    unsafe {
        let line = (*rec).ptr;
        let hi = xdl_hashlong((*rec).ha, cf.hbits) as usize;
        let mut rcrec = cf.rchash[hi];
        while !rcrec.is_null() {
            if (*rcrec).ha == (*rec).ha
                && xdl_recmatch(
                    (*rcrec).line,
                    (*rcrec).size,
                    (*rec).ptr,
                    (*rec).size,
                    cf.flags,
                ) != 0
            {
                break;
            }
            rcrec = (*rcrec).next;
        }

        if rcrec.is_null() {
            let idx = cf.rcrecs.len() as c_long;
            let mut boxed = Box::pin(xdlclass_t {
                next: cf.rchash[hi],
                ha: (*rec).ha,
                line,
                size: (*rec).size,
                idx,
                len1: 0,
                len2: 0,
            });
            rcrec = &raw mut *boxed;
            cf.rcrecs.push(boxed);
            cf.rchash[hi] = rcrec;
        }

        if pass == 1 {
            (*rcrec).len1 += 1;
        } else {
            (*rcrec).len2 += 1;
        }

        (*rec).ha = (*rcrec).idx as c_ulong;

        let rhi = xdl_hashlong((*rec).ha, hbits) as isize;
        (*rec).next = *rhash.offset(rhi);
        *rhash.offset(rhi) = rec;
    }
    0
}

unsafe fn xdl_prepare_ctx(
    pass: c_uint,
    mf: *mut mmfile_t,
    narec: c_long,
    xpp: *const xpparam_t,
    cf: &mut xdlclassifier_t,
    xdf: *mut xdfile_t,
) -> c_int {
    unsafe {
        let xdf = &mut *xdf;
        xdf.rcha = Vec::with_capacity(narec.max(0) as usize);
        xdf.recs_vec = Vec::with_capacity(narec.max(0) as usize);

        let hbits: c_uint;
        if xdf_diff_alg((*xpp).flags) == XDF_HISTOGRAM_DIFF {
            hbits = 0;
            xdf.rhash_vec = Vec::new();
            xdf.rhash = ptr::null_mut();
        } else {
            hbits = xdl_hashbits(narec as c_uint);
            let hsize: usize = 1usize << hbits;
            xdf.rhash_vec = vec![ptr::null_mut(); hsize];
            xdf.rhash = xdf.rhash_vec.as_mut_ptr();
        }

        let mut nrec: c_long = 0;
        let mut bsize: c_long = 0;
        let blk = xdl_mmfile_first(mf, &mut bsize).cast_const();
        let mut cur = blk;
        if !cur.is_null() {
            let top = blk.offset(bsize as isize);
            while cur < top {
                let prev = cur;
                let hav = xdl_hash_record(&mut cur, top, (*xpp).flags as c_long);
                let mut boxed = Box::pin(xrecord_t {
                    next: ptr::null_mut(),
                    ptr: prev,
                    size: cur.offset_from(prev) as c_long,
                    ha: hav,
                });
                let crec: *mut xrecord_t = &raw mut *boxed;
                xdf.rcha.push(boxed);
                xdf.recs_vec.push(crec);
                nrec += 1;

                if xdf_diff_alg((*xpp).flags) != XDF_HISTOGRAM_DIFF
                    && xdl_classify_record(pass, cf, xdf.rhash, hbits, crec) < 0
                {
                    *xdf = xdfile_t::new();
                    return -1;
                }
            }
        }

        xdf.rchg_vec = vec![0; (nrec + 2) as usize];
        xdf.rindex_vec = vec![0; (nrec + 1) as usize];
        xdf.ha_vec = vec![0; (nrec + 1) as usize];

        xdf.nrec = nrec;
        xdf.recs = xdf.recs_vec.as_mut_ptr();
        xdf.hbits = hbits;
        xdf.rchg = xdf.rchg_vec.as_mut_ptr().add(1);
        xdf.rindex = xdf.rindex_vec.as_mut_ptr();
        xdf.nreff = 0;
        xdf.ha = xdf.ha_vec.as_mut_ptr();
        xdf.dstart = 0;
        xdf.dend = nrec - 1;
    }

    0
}

pub unsafe fn xdl_prepare_env(
    mf1: *mut mmfile_t,
    mf2: *mut mmfile_t,
    xpp: *const xpparam_t,
    xe: *mut xdfenv_t,
) -> c_int {
    unsafe {
        let mut cf = xdlclassifier_t {
            hbits: 0,
            rchash: Vec::new(),
            rcrecs: Vec::new(),
            flags: 0,
        };

        let sample = if xdf_diff_alg((*xpp).flags) == XDF_HISTOGRAM_DIFF {
            XDL_GUESS_NLINES2
        } else {
            XDL_GUESS_NLINES1
        };

        let enl1 = xdl_guess_lines(mf1, sample) + 1;
        let enl2 = xdl_guess_lines(mf2, sample) + 1;

        if xdf_diff_alg((*xpp).flags) != XDF_HISTOGRAM_DIFF
            && xdl_init_classifier(&mut cf, enl1 + enl2 + 1, (*xpp).flags as c_long) < 0
        {
            return -1;
        }

        if xdl_prepare_ctx(1, mf1, enl1, xpp, &mut cf, &mut (*xe).xdf1) < 0 {
            return -1;
        }
        if xdl_prepare_ctx(2, mf2, enl2, xpp, &mut cf, &mut (*xe).xdf2) < 0 {
            (*xe).xdf1 = xdfile_t::new();
            return -1;
        }

        if xdf_diff_alg((*xpp).flags) != XDF_PATIENCE_DIFF
            && xdf_diff_alg((*xpp).flags) != XDF_HISTOGRAM_DIFF
            && xdl_optimize_ctxs(&cf, &mut (*xe).xdf1, &mut (*xe).xdf2) < 0
        {
            *xe = xdfenv_t::new();
            return -1;
        }
    }

    0
}

unsafe fn xdl_clean_mmatch(dis: *const c_char, i: c_long, mut s: c_long, mut e: c_long) -> c_int {
    if i - s > XDL_SIMSCAN_WINDOW {
        s = i - XDL_SIMSCAN_WINDOW;
    }
    if e - i > XDL_SIMSCAN_WINDOW {
        e = i + XDL_SIMSCAN_WINDOW;
    }

    let mut r: c_long = 1;
    let mut rdis0: c_long = 0;
    let mut rpdis0: c_long = 1;
    while (i - r) >= s {
        let d = unsafe { *dis.offset((i - r) as isize) };
        if d == 0 {
            rdis0 += 1;
        } else if d == 2 {
            rpdis0 += 1;
        } else {
            break;
        }
        r += 1;
    }
    if rdis0 == 0 {
        return 0;
    }

    r = 1;
    let mut rdis1: c_long = 0;
    let mut rpdis1: c_long = 1;
    while (i + r) <= e {
        let d = unsafe { *dis.offset((i + r) as isize) };
        if d == 0 {
            rdis1 += 1;
        } else if d == 2 {
            rpdis1 += 1;
        } else {
            break;
        }
        r += 1;
    }
    if rdis1 == 0 {
        return 0;
    }
    rdis1 += rdis0;
    rpdis1 += rpdis0;

    c_int::from(rpdis1 * XDL_KPDIS_RUN < (rpdis1 + rdis1))
}

unsafe fn xdl_cleanup_records(
    cf: &xdlclassifier_t,
    xdf1: *mut xdfile_t,
    xdf2: *mut xdfile_t,
) -> c_int {
    unsafe {
        let total = ((*xdf1).nrec + (*xdf2).nrec + 2) as usize;
        let mut dis_vec = vec![0 as c_char; total];
        let dis1 = dis_vec.as_mut_ptr();
        let dis2 = dis1.offset(((*xdf1).nrec + 1) as isize);

        let mut mlim = xdl_bogosqrt((*xdf1).nrec);
        if mlim > XDL_MAX_EQLIMIT {
            mlim = XDL_MAX_EQLIMIT;
        }
        let mut i = (*xdf1).dstart;
        let mut recs = (*xdf1).recs.offset((*xdf1).dstart as isize);
        while i <= (*xdf1).dend {
            let nm = cf.rcrecs[(**recs).ha as usize].len2;
            *dis1.offset(i as isize) = if nm == 0 {
                0
            } else if nm >= mlim {
                2
            } else {
                1
            };
            i += 1;
            recs = recs.add(1);
        }

        mlim = xdl_bogosqrt((*xdf2).nrec);
        if mlim > XDL_MAX_EQLIMIT {
            mlim = XDL_MAX_EQLIMIT;
        }
        i = (*xdf2).dstart;
        recs = (*xdf2).recs.offset((*xdf2).dstart as isize);
        while i <= (*xdf2).dend {
            let nm = cf.rcrecs[(**recs).ha as usize].len1;
            *dis2.offset(i as isize) = if nm == 0 {
                0
            } else if nm >= mlim {
                2
            } else {
                1
            };
            i += 1;
            recs = recs.add(1);
        }

        let mut nreff: c_long = 0;
        i = (*xdf1).dstart;
        recs = (*xdf1).recs.offset((*xdf1).dstart as isize);
        while i <= (*xdf1).dend {
            let d = *dis1.offset(i as isize);
            if d == 1 || (d == 2 && xdl_clean_mmatch(dis1, i, (*xdf1).dstart, (*xdf1).dend) == 0) {
                *(*xdf1).rindex.offset(nreff as isize) = i;
                *(*xdf1).ha.offset(nreff as isize) = (**recs).ha;
                nreff += 1;
            } else {
                *(*xdf1).rchg.offset(i as isize) = 1;
            }
            i += 1;
            recs = recs.add(1);
        }
        (*xdf1).nreff = nreff;

        nreff = 0;
        i = (*xdf2).dstart;
        recs = (*xdf2).recs.offset((*xdf2).dstart as isize);
        while i <= (*xdf2).dend {
            let d = *dis2.offset(i as isize);
            if d == 1 || (d == 2 && xdl_clean_mmatch(dis2, i, (*xdf2).dstart, (*xdf2).dend) == 0) {
                *(*xdf2).rindex.offset(nreff as isize) = i;
                *(*xdf2).ha.offset(nreff as isize) = (**recs).ha;
                nreff += 1;
            } else {
                *(*xdf2).rchg.offset(i as isize) = 1;
            }
            i += 1;
            recs = recs.add(1);
        }
        (*xdf2).nreff = nreff;
    }
    0
}

unsafe fn xdl_trim_ends(xdf1: *mut xdfile_t, xdf2: *mut xdfile_t) -> c_int {
    unsafe {
        let mut recs1 = (*xdf1).recs;
        let mut recs2 = (*xdf2).recs;
        let mut i: c_long = 0;
        let mut lim = (*xdf1).nrec.min((*xdf2).nrec);
        while i < lim {
            if (**recs1).ha != (**recs2).ha {
                break;
            }
            i += 1;
            recs1 = recs1.add(1);
            recs2 = recs2.add(1);
        }

        (*xdf1).dstart = i;
        (*xdf2).dstart = i;

        recs1 = (*xdf1).recs.offset(((*xdf1).nrec - 1) as isize);
        recs2 = (*xdf2).recs.offset(((*xdf2).nrec - 1) as isize);
        lim -= i;
        i = 0;
        while i < lim {
            if (**recs1).ha != (**recs2).ha {
                break;
            }
            i += 1;
            recs1 = recs1.sub(1);
            recs2 = recs2.sub(1);
        }

        (*xdf1).dend = (*xdf1).nrec - i - 1;
        (*xdf2).dend = (*xdf2).nrec - i - 1;
    }
    0
}

unsafe fn xdl_optimize_ctxs(
    cf: &xdlclassifier_t,
    xdf1: *mut xdfile_t,
    xdf2: *mut xdfile_t,
) -> c_int {
    if unsafe { xdl_trim_ends(xdf1, xdf2) < 0 || xdl_cleanup_records(cf, xdf1, xdf2) < 0 } {
        return -1;
    }
    0
}
