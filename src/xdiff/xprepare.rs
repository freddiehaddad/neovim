//! XDiff environment preparation (`src/xdiff/xprepare.c`).

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

use crate::memory::{xfree, xmalloc, xrealloc};
use crate::xdiff::xutils::{
    xdl_bogosqrt, xdl_cha_alloc, xdl_cha_free, xdl_cha_init, xdl_guess_lines, xdl_hash_record,
    xdl_hashbits, xdl_mmfile_first, xdl_recmatch,
};
use crate::xdiff::{
    XDF_HISTOGRAM_DIFF, XDF_PATIENCE_DIFF, chastore_t, mmfile_t, xdf_diff_alg, xdfenv_t, xdfile_t,
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

#[repr(C)]
#[derive(Copy, Clone)]
struct xdlclassifier_t {
    hbits: c_uint,
    hsize: c_long,
    rchash: *mut *mut xdlclass_t,
    ncha: chastore_t,
    rcrecs: *mut *mut xdlclass_t,
    alloc: c_long,
    count: c_long,
    flags: c_long,
}

unsafe fn xdl_init_classifier(cf: *mut xdlclassifier_t, size: c_long, flags: c_long) -> c_int {
    unsafe {
        (*cf).flags = flags;
        (*cf).hbits = xdl_hashbits(size as c_uint);
        (*cf).hsize = 1 << (*cf).hbits;

        if xdl_cha_init(
            &mut (*cf).ncha,
            size_of::<xdlclass_t>() as c_long,
            size / 4 + 1,
        ) < 0
        {
            return -1;
        }
        (*cf).rchash = xmalloc(((*cf).hsize as usize) * size_of::<*mut xdlclass_t>())
            .cast::<*mut xdlclass_t>();
        if (*cf).rchash.is_null() {
            xdl_cha_free(&mut (*cf).ncha);
            return -1;
        }
        ptr::write_bytes((*cf).rchash, 0, (*cf).hsize as usize);

        (*cf).alloc = size;
        (*cf).rcrecs = xmalloc(((*cf).alloc as usize) * size_of::<*mut xdlclass_t>())
            .cast::<*mut xdlclass_t>();
        if (*cf).rcrecs.is_null() {
            xfree((*cf).rchash.cast::<c_void>());
            xdl_cha_free(&mut (*cf).ncha);
            return -1;
        }

        (*cf).count = 0;
    }
    0
}

unsafe fn xdl_free_classifier(cf: *mut xdlclassifier_t) {
    unsafe {
        xfree((*cf).rcrecs.cast::<c_void>());
        xfree((*cf).rchash.cast::<c_void>());
        xdl_cha_free(&mut (*cf).ncha);
    }
}

unsafe fn xdl_classify_record(
    pass: c_uint,
    cf: *mut xdlclassifier_t,
    rhash: *mut *mut xrecord_t,
    hbits: c_uint,
    rec: *mut xrecord_t,
) -> c_int {
    unsafe {
        let line = (*rec).ptr;
        let hi = xdl_hashlong((*rec).ha, (*cf).hbits) as isize;
        let mut rcrec = *(*cf).rchash.offset(hi);
        while !rcrec.is_null() {
            if (*rcrec).ha == (*rec).ha
                && xdl_recmatch(
                    (*rcrec).line,
                    (*rcrec).size,
                    (*rec).ptr,
                    (*rec).size,
                    (*cf).flags,
                ) != 0
            {
                break;
            }
            rcrec = (*rcrec).next;
        }

        if rcrec.is_null() {
            rcrec = xdl_cha_alloc(&mut (*cf).ncha).cast::<xdlclass_t>();
            if rcrec.is_null() {
                return -1;
            }
            (*rcrec).idx = (*cf).count;
            (*cf).count += 1;
            if (*cf).count > (*cf).alloc {
                (*cf).alloc *= 2;
                let rcrecs = xrealloc(
                    (*cf).rcrecs.cast::<c_void>(),
                    ((*cf).alloc as usize) * size_of::<*mut xdlclass_t>(),
                )
                .cast::<*mut xdlclass_t>();
                if rcrecs.is_null() {
                    return -1;
                }
                (*cf).rcrecs = rcrecs;
            }
            *(*cf).rcrecs.offset((*rcrec).idx as isize) = rcrec;
            (*rcrec).line = line;
            (*rcrec).size = (*rec).size;
            (*rcrec).ha = (*rec).ha;
            (*rcrec).len1 = 0;
            (*rcrec).len2 = 0;
            (*rcrec).next = *(*cf).rchash.offset(hi);
            *(*cf).rchash.offset(hi) = rcrec;
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
    mut narec: c_long,
    xpp: *const xpparam_t,
    cf: *mut xdlclassifier_t,
    xdf: *mut xdfile_t,
) -> c_int {
    let mut ha: *mut c_ulong = ptr::null_mut();
    let mut rindex: *mut c_long = ptr::null_mut();
    let mut rchg: *mut c_char = ptr::null_mut();
    let mut rhash: *mut *mut xrecord_t = ptr::null_mut();
    let mut recs: *mut *mut xrecord_t = ptr::null_mut();

    let abort = |ha: *mut c_ulong,
                 rindex: *mut c_long,
                 rchg: *mut c_char,
                 rhash: *mut *mut xrecord_t,
                 recs: *mut *mut xrecord_t,
                 xdf: *mut xdfile_t| unsafe {
        xfree(ha.cast::<c_void>());
        xfree(rindex.cast::<c_void>());
        xfree(rchg.cast::<c_void>());
        xfree(rhash.cast::<c_void>());
        xfree(recs.cast::<c_void>());
        xdl_cha_free(&mut (*xdf).rcha);
        -1
    };

    unsafe {
        if xdl_cha_init(
            &mut (*xdf).rcha,
            size_of::<xrecord_t>() as c_long,
            narec / 4 + 1,
        ) < 0
        {
            return abort(ha, rindex, rchg, rhash, recs, xdf);
        }
        recs = xmalloc((narec as usize) * size_of::<*mut xrecord_t>()).cast::<*mut xrecord_t>();
        if recs.is_null() {
            return abort(ha, rindex, rchg, rhash, recs, xdf);
        }

        let hbits: c_uint;
        if xdf_diff_alg((*xpp).flags) == XDF_HISTOGRAM_DIFF {
            hbits = 0;
        } else {
            hbits = xdl_hashbits(narec as c_uint);
            let hsize: usize = 1usize << hbits;
            rhash = xmalloc(hsize * size_of::<*mut xrecord_t>()).cast::<*mut xrecord_t>();
            if rhash.is_null() {
                return abort(ha, rindex, rchg, rhash, recs, xdf);
            }
            ptr::write_bytes(rhash, 0, hsize);
        }

        let mut nrec: c_long = 0;
        let mut bsize: c_long = 0;
        let blk = xdl_mmfile_first(mf, &mut bsize)
            .cast::<c_char>()
            .cast_const();
        let mut cur = blk;
        if !cur.is_null() {
            let top = blk.offset(bsize as isize);
            while cur < top {
                let prev = cur;
                let hav = xdl_hash_record(&mut cur, top, (*xpp).flags as c_long);
                if nrec >= narec {
                    narec *= 2;
                    let rrecs = xrealloc(
                        recs.cast::<c_void>(),
                        (narec as usize) * size_of::<*mut xrecord_t>(),
                    )
                    .cast::<*mut xrecord_t>();
                    if rrecs.is_null() {
                        return abort(ha, rindex, rchg, rhash, recs, xdf);
                    }
                    recs = rrecs;
                }
                let crec = xdl_cha_alloc(&mut (*xdf).rcha).cast::<xrecord_t>();
                if crec.is_null() {
                    return abort(ha, rindex, rchg, rhash, recs, xdf);
                }
                (*crec).ptr = prev;
                (*crec).size = cur.offset_from(prev) as c_long;
                (*crec).ha = hav;
                *recs.offset(nrec as isize) = crec;
                nrec += 1;

                if xdf_diff_alg((*xpp).flags) != XDF_HISTOGRAM_DIFF
                    && xdl_classify_record(pass, cf, rhash, hbits, crec) < 0
                {
                    return abort(ha, rindex, rchg, rhash, recs, xdf);
                }
            }
        }

        rchg = xmalloc(((nrec + 2) as usize) * size_of::<c_char>()).cast::<c_char>();
        if rchg.is_null() {
            return abort(ha, rindex, rchg, rhash, recs, xdf);
        }
        ptr::write_bytes(rchg, 0, (nrec + 2) as usize);

        rindex = xmalloc(((nrec + 1) as usize) * size_of::<c_long>()).cast::<c_long>();
        if rindex.is_null() {
            return abort(ha, rindex, rchg, rhash, recs, xdf);
        }
        ha = xmalloc(((nrec + 1) as usize) * size_of::<c_ulong>()).cast::<c_ulong>();
        if ha.is_null() {
            return abort(ha, rindex, rchg, rhash, recs, xdf);
        }

        (*xdf).nrec = nrec;
        (*xdf).recs = recs;
        (*xdf).hbits = hbits;
        (*xdf).rhash = rhash;
        (*xdf).rchg = rchg.add(1);
        (*xdf).rindex = rindex;
        (*xdf).nreff = 0;
        (*xdf).ha = ha;
        (*xdf).dstart = 0;
        (*xdf).dend = nrec - 1;
    }

    0
}

unsafe fn xdl_free_ctx(xdf: *mut xdfile_t) {
    unsafe {
        xfree((*xdf).rhash.cast::<c_void>());
        xfree((*xdf).rindex.cast::<c_void>());
        xfree((*xdf).rchg.sub(1).cast::<c_void>());
        xfree((*xdf).ha.cast::<c_void>());
        xfree((*xdf).recs.cast::<c_void>());
        xdl_cha_free(&mut (*xdf).rcha);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn xdl_prepare_env(
    mf1: *mut mmfile_t,
    mf2: *mut mmfile_t,
    xpp: *const xpparam_t,
    xe: *mut xdfenv_t,
) -> c_int {
    unsafe {
        let mut cf: xdlclassifier_t = zeroed();

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
            xdl_free_classifier(&mut cf);
            return -1;
        }
        if xdl_prepare_ctx(2, mf2, enl2, xpp, &mut cf, &mut (*xe).xdf2) < 0 {
            xdl_free_ctx(&mut (*xe).xdf1);
            xdl_free_classifier(&mut cf);
            return -1;
        }

        if xdf_diff_alg((*xpp).flags) != XDF_PATIENCE_DIFF
            && xdf_diff_alg((*xpp).flags) != XDF_HISTOGRAM_DIFF
            && xdl_optimize_ctxs(&mut cf, &mut (*xe).xdf1, &mut (*xe).xdf2) < 0
        {
            xdl_free_ctx(&mut (*xe).xdf2);
            xdl_free_ctx(&mut (*xe).xdf1);
            xdl_free_classifier(&mut cf);
            return -1;
        }

        if xdf_diff_alg((*xpp).flags) != XDF_HISTOGRAM_DIFF {
            xdl_free_classifier(&mut cf);
        }
    }

    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn xdl_free_env(xe: *mut xdfenv_t) {
    unsafe {
        xdl_free_ctx(&mut (*xe).xdf2);
        xdl_free_ctx(&mut (*xe).xdf1);
    }
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
    cf: *mut xdlclassifier_t,
    xdf1: *mut xdfile_t,
    xdf2: *mut xdfile_t,
) -> c_int {
    unsafe {
        let total = ((*xdf1).nrec + (*xdf2).nrec + 2) as usize;
        let dis = xmalloc(total).cast::<c_char>();
        if dis.is_null() {
            return -1;
        }
        ptr::write_bytes(dis, 0, total);
        let dis1 = dis;
        let dis2 = dis1.offset(((*xdf1).nrec + 1) as isize);

        let mut mlim = xdl_bogosqrt((*xdf1).nrec);
        if mlim > XDL_MAX_EQLIMIT {
            mlim = XDL_MAX_EQLIMIT;
        }
        let mut i = (*xdf1).dstart;
        let mut recs = (*xdf1).recs.offset((*xdf1).dstart as isize);
        while i <= (*xdf1).dend {
            let rcrec = *(*cf).rcrecs.offset((**recs).ha as isize);
            let nm = if !rcrec.is_null() { (*rcrec).len2 } else { 0 };
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
            let rcrec = *(*cf).rcrecs.offset((**recs).ha as isize);
            let nm = if !rcrec.is_null() { (*rcrec).len1 } else { 0 };
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

        xfree(dis.cast::<c_void>());
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
    cf: *mut xdlclassifier_t,
    xdf1: *mut xdfile_t,
    xdf2: *mut xdfile_t,
) -> c_int {
    if unsafe { xdl_trim_ends(xdf1, xdf2) < 0 || xdl_cleanup_records(cf, xdf1, xdf2) < 0 } {
        return -1;
    }
    0
}
