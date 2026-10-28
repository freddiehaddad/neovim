//! Diff hunk emission (`src/xdiff/xemit.c`).

use std::ffi::{c_char, c_int, c_long, c_ulong};
use std::ptr;

use crate::xdiff::xutils::{xdl_emit_diffrec, xdl_emit_hunk_hdr};
use crate::xdiff::{XDL_EMIT_NO_HUNK_HDR, Xdchange, Xdemitcb, Xdemitconf, Xdfenv, Xdfile};

fn xdl_get_rec(xdf: &Xdfile, ri: c_long, rec: &mut *const c_char) -> c_long {
    let r = &xdf.recs[ri as usize];
    *rec = r.ptr;
    r.size
}

fn xdl_emit_record(xdf: &Xdfile, ri: c_long, pre: &std::ffi::CStr, ecb: &mut Xdemitcb) -> c_int {
    let psize = { pre.to_bytes().len() as c_long };
    let mut rec: *const c_char = ptr::null();
    let size = xdl_get_rec(xdf, ri, &mut rec);
    if xdl_emit_diffrec(rec, size, pre.as_ptr(), psize, ecb) < 0 {
        return -1;
    }
    0
}

pub fn xdl_get_hunk(xscr: &[Xdchange], idx: &mut usize, xecfg: &Xdemitconf) -> Option<usize> {
    let max_common = 2 * xecfg.ctxlen + xecfg.interhunkctxlen;
    let max_ignorable = xecfg.ctxlen;
    let mut ignored: c_ulong = 0;

    let mut xchp = *idx;
    while xchp < xscr.len() && xscr[xchp].ignore != 0 {
        let xch = xchp + 1;
        if xch >= xscr.len() || xscr[xch].i1 - (xscr[xchp].i1 + xscr[xchp].chg1) >= max_ignorable {
            *idx = xch;
        }
        xchp = xch;
    }

    if *idx >= xscr.len() {
        return None;
    }

    let mut lxch = *idx;
    xchp = *idx;
    let mut xch = xchp + 1;
    while xch < xscr.len() {
        let distance = xscr[xch].i1 - (xscr[xchp].i1 + xscr[xchp].chg1);
        if distance > max_common {
            break;
        }

        if distance < max_ignorable && (xscr[xch].ignore == 0 || lxch == xchp) {
            lxch = xch;
            ignored = 0;
        } else if distance < max_ignorable && xscr[xch].ignore != 0 {
            ignored += xscr[xch].chg2 as c_ulong;
        } else if lxch != xchp
            && xscr[xch].i1 + (ignored as c_long) - (xscr[lxch].i1 + xscr[lxch].chg1) > max_common
        {
            break;
        } else if xscr[xch].ignore == 0 {
            lxch = xch;
            ignored = 0;
        } else {
            ignored += xscr[xch].chg2 as c_ulong;
        }

        xchp = xch;
        xch += 1;
    }

    Some(lxch)
}

pub fn xdl_emit_diff(
    xe: &mut Xdfenv,
    xscr: &[Xdchange],
    ecb: &mut Xdemitcb,
    xecfg: &Xdemitconf,
) -> c_int {
    let mut idx = 0;
    while let Some(end_idx) = xdl_get_hunk(xscr, &mut idx, xecfg) {
        let mut cur = idx;
        let xche = &xscr[end_idx];

        let mut s1 = (xscr[cur].i1 - xecfg.ctxlen).max(0);
        let mut s2 = (xscr[cur].i2 - xecfg.ctxlen).max(0);

        let mut lctx = xecfg.ctxlen;
        lctx = lctx.min(xe.xdf1.nrec - (xche.i1 + xche.chg1));
        lctx = lctx.min(xe.xdf2.nrec - (xche.i2 + xche.chg2));

        let e1 = xche.i1 + xche.chg1 + lctx;
        let e2 = xche.i2 + xche.chg2 + lctx;

        if (xecfg.flags & XDL_EMIT_NO_HUNK_HDR) == 0
            && xdl_emit_hunk_hdr(s1 + 1, e1 - s1, s2 + 1, e2 - s2, ecb) < 0
        {
            return -1;
        }

        while s2 < xscr[cur].i2 {
            if xdl_emit_record(&xe.xdf2, s2, c" ", ecb) < 0 {
                return -1;
            }
            s2 += 1;
        }

        s1 = xscr[cur].i1;
        s2 = xscr[cur].i2;
        loop {
            let xch = &xscr[cur];
            while s1 < xch.i1 && s2 < xch.i2 {
                if xdl_emit_record(&xe.xdf2, s2, c" ", ecb) < 0 {
                    return -1;
                }
                s1 += 1;
                s2 += 1;
            }

            s1 = xch.i1;
            while s1 < xch.i1 + xch.chg1 {
                if xdl_emit_record(&xe.xdf1, s1, c"-", ecb) < 0 {
                    return -1;
                }
                s1 += 1;
            }

            s2 = xch.i2;
            while s2 < xch.i2 + xch.chg2 {
                if xdl_emit_record(&xe.xdf2, s2, c"+", ecb) < 0 {
                    return -1;
                }
                s2 += 1;
            }

            if cur == end_idx {
                break;
            }
            s1 = xch.i1 + xch.chg1;
            s2 = xch.i2 + xch.chg2;
            cur += 1;
        }

        s2 = xche.i2 + xche.chg2;
        while s2 < e2 {
            if xdl_emit_record(&xe.xdf2, s2, c" ", ecb) < 0 {
                return -1;
            }
            s2 += 1;
        }

        idx = end_idx + 1;
    }

    0
}
