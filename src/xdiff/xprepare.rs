//! XDiff environment preparation (`src/xdiff/xprepare.c`).

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong};

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
    next: Option<usize>,
    ha: c_ulong,
    line: *const c_char,
    size: c_long,
    idx: c_long,
    len1: c_long,
    len2: c_long,
}

struct xdlclassifier_t {
    hbits: c_uint,
    rchash: Vec<Option<usize>>,
    rcrecs: Vec<xdlclass_t>,
    flags: c_long,
}

fn xdl_init_classifier(cf: &mut xdlclassifier_t, size: c_long, flags: c_long) -> c_int {
    cf.flags = flags;
    cf.hbits = xdl_hashbits(size as c_uint);
    let hsize = 1usize << cf.hbits;
    cf.rchash = vec![None; hsize];
    cf.rcrecs = Vec::with_capacity(size as usize);
    0
}

fn xdl_classify_record(
    pass: c_uint,
    cf: &mut xdlclassifier_t,
    rec: &mut xrecord_t,
) -> c_int {
    let line = rec.ptr;
        let hi = xdl_hashlong(rec.ha, cf.hbits) as usize;
        let mut rcrec_idx = cf.rchash[hi];
        while let Some(idx) = rcrec_idx {
            let rcrec = &cf.rcrecs[idx];
            if rcrec.ha == rec.ha
                && xdl_recmatch(rcrec.line, rcrec.size, rec.ptr, rec.size, cf.flags) != 0
            {
                break;
            }
            rcrec_idx = rcrec.next;
        }

        let idx = if let Some(idx) = rcrec_idx {
            idx
        } else {
            let new_idx = cf.rcrecs.len();
            cf.rcrecs.push(xdlclass_t {
                next: cf.rchash[hi],
                ha: rec.ha,
                line,
                size: rec.size,
                idx: new_idx as c_long,
                len1: 0,
                len2: 0,
            });
            cf.rchash[hi] = Some(new_idx);
            new_idx
        };

        let rcrec = &mut cf.rcrecs[idx];
        if pass == 1 {
            rcrec.len1 += 1;
        } else {
            rcrec.len2 += 1;
        }

        rec.ha = rcrec.idx as c_ulong;
    0
}

fn xdl_prepare_ctx(
    pass: c_uint,
    mf: *mut mmfile_t,
    narec: c_long,
    xpp: *const xpparam_t,
    cf: &mut xdlclassifier_t,
    xdf: *mut xdfile_t,
) -> c_int {
    unsafe {
        let xdf = &mut *xdf;
        xdf.recs = Vec::with_capacity(narec.max(0) as usize);

        let mut nrec: c_long = 0;
        let mut bsize: c_long = 0;
        let blk = xdl_mmfile_first(mf, &mut bsize).cast_const();
        let mut cur = blk;
        if !cur.is_null() {
            let top = blk.offset(bsize as isize);
            while cur < top {
                let prev = cur;
                let hav = xdl_hash_record(&mut cur, top, (*xpp).flags as c_long);
                let mut rec = xrecord_t {
                    ptr: prev,
                    size: cur.offset_from(prev) as c_long,
                    ha: hav,
                };
                nrec += 1;

                if xdf_diff_alg((*xpp).flags) != XDF_HISTOGRAM_DIFF
                    && xdl_classify_record(pass, cf, &mut rec) < 0
                {
                    *xdf = xdfile_t::new();
                    return -1;
                }
                xdf.recs.push(rec);
            }
        }

        xdf.rchg_vec = vec![0; (nrec + 2) as usize];
        xdf.rindex = vec![0; (nrec + 1) as usize];
        xdf.ha = vec![0; (nrec + 1) as usize];

        xdf.nrec = nrec;
        xdf.rchg = xdf.rchg_vec.as_mut_ptr().add(1);
        xdf.nreff = 0;
        xdf.dstart = 0;
        xdf.dend = nrec - 1;
    }

    0
}

pub fn xdl_prepare_env(
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

fn xdl_clean_mmatch(dis: *const c_char, i: c_long, mut s: c_long, mut e: c_long) -> c_int {
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

fn xdl_cleanup_records(
    cf: &xdlclassifier_t,
    xdf1: *mut xdfile_t,
    xdf2: *mut xdfile_t,
) -> c_int {
    unsafe {
        let total = ((*xdf1).nrec + (*xdf2).nrec + 2) as usize;
        let mut dis_vec = vec![0 as c_char; total];
        let dis1 = dis_vec.as_mut_ptr();
        let dis2 = dis1.offset(((*xdf1).nrec + 1) as isize);

        let mut mlim = xdl_bogosqrt((*xdf1).nrec).min(XDL_MAX_EQLIMIT);
        for i in (*xdf1).dstart..=(*xdf1).dend {
            let nm = cf.rcrecs[(&(*xdf1).recs)[i as usize].ha as usize].len2;
            *dis1.offset(i as isize) = if nm == 0 {
                0
            } else if nm >= mlim {
                2
            } else {
                1
            };
        }

        mlim = xdl_bogosqrt((*xdf2).nrec).min(XDL_MAX_EQLIMIT);
        for i in (*xdf2).dstart..=(*xdf2).dend {
            let nm = cf.rcrecs[(&(*xdf2).recs)[i as usize].ha as usize].len1;
            *dis2.offset(i as isize) = if nm == 0 {
                0
            } else if nm >= mlim {
                2
            } else {
                1
            };
        }

        let mut nreff: c_long = 0;
        for i in (*xdf1).dstart..=(*xdf1).dend {
            let d = *dis1.offset(i as isize);
            if d == 1 || (d == 2 && xdl_clean_mmatch(dis1, i, (*xdf1).dstart, (*xdf1).dend) == 0) {
                (&mut (*xdf1).rindex)[nreff as usize] = i;
                (&mut (*xdf1).ha)[nreff as usize] = (&(*xdf1).recs)[i as usize].ha;
                nreff += 1;
            } else {
                *(*xdf1).rchg.offset(i as isize) = 1;
            }
        }
        (*xdf1).nreff = nreff;

        nreff = 0;
        for i in (*xdf2).dstart..=(*xdf2).dend {
            let d = *dis2.offset(i as isize);
            if d == 1 || (d == 2 && xdl_clean_mmatch(dis2, i, (*xdf2).dstart, (*xdf2).dend) == 0) {
                (&mut (*xdf2).rindex)[nreff as usize] = i;
                (&mut (*xdf2).ha)[nreff as usize] = (&(*xdf2).recs)[i as usize].ha;
                nreff += 1;
            } else {
                *(*xdf2).rchg.offset(i as isize) = 1;
            }
        }
        (*xdf2).nreff = nreff;
    }
    0
}

fn xdl_trim_ends(xdf1: *mut xdfile_t, xdf2: *mut xdfile_t) -> c_int {
    unsafe {
        let recs1 = &(*xdf1).recs;
        let recs2 = &(*xdf2).recs;
        let mut i: c_long = 0;
        let mut lim = (*xdf1).nrec.min((*xdf2).nrec);
        while i < lim {
            if recs1[i as usize].ha != recs2[i as usize].ha {
                break;
            }
            i += 1;
        }

        (*xdf1).dstart = i;
        (*xdf2).dstart = i;

        lim -= i;
        i = 0;
        while i < lim {
            let idx1 = ((*xdf1).nrec - 1 - i) as usize;
            let idx2 = ((*xdf2).nrec - 1 - i) as usize;
            if recs1[idx1].ha != recs2[idx2].ha {
                break;
            }
            i += 1;
        }

        (*xdf1).dend = (*xdf1).nrec - i - 1;
        (*xdf2).dend = (*xdf2).nrec - i - 1;
    }
    0
}

fn xdl_optimize_ctxs(
    cf: &xdlclassifier_t,
    xdf1: *mut xdfile_t,
    xdf2: *mut xdfile_t,
) -> c_int {
    if xdl_trim_ends(xdf1, xdf2) < 0 || xdl_cleanup_records(cf, xdf1, xdf2) < 0 {
        return -1;
    }
    0
}
