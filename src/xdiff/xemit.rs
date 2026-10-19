//! Diff hunk emission (`src/xdiff/xemit.c`).

use std::ffi::{c_char, c_int, c_long, c_ulong};
use std::ptr;

use crate::xdiff::xutils::{xdl_emit_diffrec, xdl_emit_hunk_hdr};
use crate::xdiff::{
    XDL_EMIT_NO_HUNK_HDR, xdchange_t, xdemitcb_t, xdemitconf_t, xdfenv_t, xdfile_t,
};

use crate::strings::strlen;

unsafe fn xdl_get_rec(xdf: *mut xdfile_t, ri: c_long, rec: *mut *const c_char) -> c_long {
    unsafe {
        let r = *(*xdf).recs.offset(ri as isize);
        *rec = (*r).ptr;
        (*r).size
    }
}

unsafe fn xdl_emit_record(
    xdf: *mut xdfile_t,
    ri: c_long,
    pre: *const c_char,
    ecb: *mut xdemitcb_t,
) -> c_int {
    unsafe {
        let psize = strlen(pre) as c_long;
        let mut rec: *const c_char = ptr::null();
        let size = xdl_get_rec(xdf, ri, &mut rec);
        if xdl_emit_diffrec(rec, size, pre, psize, ecb) < 0 {
            return -1;
        }
    }
    0
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_get_hunk(
    xscr: *mut *mut xdchange_t,
    xecfg: *const xdemitconf_t,
) -> *mut xdchange_t {
    unsafe {
        let max_common = 2 * (*xecfg).ctxlen + (*xecfg).interhunkctxlen;
        let max_ignorable = (*xecfg).ctxlen;
        let mut ignored: c_ulong = 0;

        let mut xchp = *xscr;
        while !xchp.is_null() && (*xchp).ignore != 0 {
            let xch = (*xchp).next;
            if xch.is_null() || (*xch).i1 - ((*xchp).i1 + (*xchp).chg1) >= max_ignorable {
                *xscr = xch;
            }
            xchp = (*xchp).next;
        }

        if (*xscr).is_null() {
            return ptr::null_mut();
        }

        let mut lxch = *xscr;
        xchp = *xscr;
        let mut xch = (*xchp).next;
        while !xch.is_null() {
            let distance = (*xch).i1 - ((*xchp).i1 + (*xchp).chg1);
            if distance > max_common {
                break;
            }

            if distance < max_ignorable && ((*xch).ignore == 0 || lxch == xchp) {
                lxch = xch;
                ignored = 0;
            } else if distance < max_ignorable && (*xch).ignore != 0 {
                ignored += (*xch).chg2 as c_ulong;
            } else if lxch != xchp
                && (*xch).i1 + (ignored as c_long) - ((*lxch).i1 + (*lxch).chg1) > max_common
            {
                break;
            } else if (*xch).ignore == 0 {
                lxch = xch;
                ignored = 0;
            } else {
                ignored += (*xch).chg2 as c_ulong;
            }

            xchp = xch;
            xch = (*xch).next;
        }

        lxch
    }
}

struct FuncLine {
    len: c_long,
    buf: [c_char; 80],
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_emit_diff(
    xe: *mut xdfenv_t,
    xscr: *mut xdchange_t,
    ecb: *mut xdemitcb_t,
    xecfg: *const xdemitconf_t,
) -> c_int {
    let func_line = FuncLine {
        len: 0,
        buf: [0; 80],
    };

    let mut xch = xscr;
    unsafe {
        while !xch.is_null() {
            let xche = xdl_get_hunk(&mut xch, xecfg);
            if xch.is_null() {
                break;
            }

            let mut s1 = ((*xch).i1 - (*xecfg).ctxlen).max(0);
            let mut s2 = ((*xch).i2 - (*xecfg).ctxlen).max(0);

            let mut lctx = (*xecfg).ctxlen;
            lctx = lctx.min((*xe).xdf1.nrec - ((*xche).i1 + (*xche).chg1));
            lctx = lctx.min((*xe).xdf2.nrec - ((*xche).i2 + (*xche).chg2));

            let e1 = (*xche).i1 + (*xche).chg1 + lctx;
            let e2 = (*xche).i2 + (*xche).chg2 + lctx;

            if ((*xecfg).flags & XDL_EMIT_NO_HUNK_HDR) == 0
                && xdl_emit_hunk_hdr(
                    s1 + 1,
                    e1 - s1,
                    s2 + 1,
                    e2 - s2,
                    func_line.buf.as_ptr(),
                    func_line.len,
                    ecb,
                ) < 0
            {
                return -1;
            }

            while s2 < (*xch).i2 {
                if xdl_emit_record(&mut (*xe).xdf2, s2, c" ".as_ptr(), ecb) < 0 {
                    return -1;
                }
                s2 += 1;
            }

            s1 = (*xch).i1;
            s2 = (*xch).i2;
            loop {
                while s1 < (*xch).i1 && s2 < (*xch).i2 {
                    if xdl_emit_record(&mut (*xe).xdf2, s2, c" ".as_ptr(), ecb) < 0 {
                        return -1;
                    }
                    s1 += 1;
                    s2 += 1;
                }

                s1 = (*xch).i1;
                while s1 < (*xch).i1 + (*xch).chg1 {
                    if xdl_emit_record(&mut (*xe).xdf1, s1, c"-".as_ptr(), ecb) < 0 {
                        return -1;
                    }
                    s1 += 1;
                }

                s2 = (*xch).i2;
                while s2 < (*xch).i2 + (*xch).chg2 {
                    if xdl_emit_record(&mut (*xe).xdf2, s2, c"+".as_ptr(), ecb) < 0 {
                        return -1;
                    }
                    s2 += 1;
                }

                if xch == xche {
                    break;
                }
                s1 = (*xch).i1 + (*xch).chg1;
                s2 = (*xch).i2 + (*xch).chg2;
                xch = (*xch).next;
            }

            s2 = (*xche).i2 + (*xche).chg2;
            while s2 < e2 {
                if xdl_emit_record(&mut (*xe).xdf2, s2, c" ".as_ptr(), ecb) < 0 {
                    return -1;
                }
                s2 += 1;
            }

            xch = (*xche).next;
        }
    }

    0
}
