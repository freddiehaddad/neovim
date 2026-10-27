//! Core diff algorithm (`src/xdiff/xdiffi.c`).

use std::ffi::{c_char, c_int, c_long, c_ulong};

use crate::xdiff::xemit::{xdl_emit_diff, xdl_get_hunk};
use crate::xdiff::xhistogram::xdl_do_histogram_diff;
use crate::xdiff::xpatience::xdl_do_patience_diff;
use crate::xdiff::xprepare::xdl_prepare_env;
use crate::xdiff::xutils::{xdl_blankline, xdl_bogosqrt, xdl_recmatch};
use crate::xdiff::{
    XDF_HISTOGRAM_DIFF, XDF_IGNORE_BLANK_LINES, XDF_INDENT_HEURISTIC, XDF_NEED_MINIMAL,
    XDF_PATIENCE_DIFF, diffdata_t, emit_func_t, mmfile_t, xdalgoenv_t, xdchange_t, xdemitcb_t,
    xdemitconf_t, xdf_diff_alg, xdfenv_t, xdfile_t, xdl_isspace, xpparam_t, xrecord_t,
};

const XDL_MAX_COST_MIN: c_long = 256;
const XDL_HEUR_MIN_COST: c_long = 256;
const XDL_LINE_MAX: c_long = c_long::MAX;
const XDL_SNAKE_CNT: c_long = 20;
const XDL_K_HEUR: c_long = 4;

const MAX_INDENT: c_int = 200;
const MAX_BLANKS: c_int = 20;
const START_OF_FILE_PENALTY: c_int = 1;
const END_OF_FILE_PENALTY: c_int = 21;
const TOTAL_BLANK_WEIGHT: c_int = -30;
const POST_BLANK_WEIGHT: c_int = 6;
const RELATIVE_INDENT_PENALTY: c_int = -4;
const RELATIVE_INDENT_WITH_BLANK_PENALTY: c_int = 10;
const RELATIVE_OUTDENT_PENALTY: c_int = 24;
const RELATIVE_OUTDENT_WITH_BLANK_PENALTY: c_int = 17;
const RELATIVE_DEDENT_PENALTY: c_int = 23;
const RELATIVE_DEDENT_WITH_BLANK_PENALTY: c_int = 17;
const INDENT_WEIGHT: c_int = 60;
const INDENT_HEURISTIC_MAX_SLIDING: c_long = 100;

#[repr(C)]
#[derive(Copy, Clone)]
struct xdpsplit_t {
    i1: c_long,
    i2: c_long,
    min_lo: c_int,
    min_hi: c_int,
}

unsafe fn xdl_split(
    ha1: *const c_ulong,
    off1: c_long,
    lim1: c_long,
    ha2: *const c_ulong,
    off2: c_long,
    lim2: c_long,
    kvdf: *mut c_long,
    kvdb: *mut c_long,
    need_min: c_int,
    spl: *mut xdpsplit_t,
    xenv: *mut xdalgoenv_t,
) -> c_long {
    let dmin = off1 - lim2;
    let dmax = lim1 - off2;
    let fmid = off1 - off2;
    let bmid = lim1 - lim2;
    let odd = ((fmid - bmid) & 1) != 0;
    let mut fmin = fmid;
    let mut fmax = fmid;
    let mut bmin = bmid;
    let mut bmax = bmid;

    unsafe {
        *kvdf.offset(fmid as isize) = off1;
        *kvdb.offset(bmid as isize) = lim1;

        let mut ec: c_long = 1;
        loop {
            let mut got_snake = false;

            if fmin > dmin {
                fmin -= 1;
                *kvdf.offset((fmin - 1) as isize) = -1;
            } else {
                fmin += 1;
            }
            if fmax < dmax {
                fmax += 1;
                *kvdf.offset((fmax + 1) as isize) = -1;
            } else {
                fmax -= 1;
            }

            let mut d = fmax;
            while d >= fmin {
                let mut i1 = if *kvdf.offset((d - 1) as isize) >= *kvdf.offset((d + 1) as isize) {
                    *kvdf.offset((d - 1) as isize) + 1
                } else {
                    *kvdf.offset((d + 1) as isize)
                };
                let prev1 = i1;
                let mut i2 = i1 - d;
                while i1 < lim1 && i2 < lim2 && *ha1.offset(i1 as isize) == *ha2.offset(i2 as isize)
                {
                    i1 += 1;
                    i2 += 1;
                }
                if i1 - prev1 > (*xenv).snake_cnt {
                    got_snake = true;
                }
                *kvdf.offset(d as isize) = i1;
                if odd && bmin <= d && d <= bmax && *kvdb.offset(d as isize) <= i1 {
                    (*spl).i1 = i1;
                    (*spl).i2 = i2;
                    (*spl).min_lo = 1;
                    (*spl).min_hi = 1;
                    return ec;
                }
                d -= 2;
            }

            if bmin > dmin {
                bmin -= 1;
                *kvdb.offset((bmin - 1) as isize) = XDL_LINE_MAX;
            } else {
                bmin += 1;
            }
            if bmax < dmax {
                bmax += 1;
                *kvdb.offset((bmax + 1) as isize) = XDL_LINE_MAX;
            } else {
                bmax -= 1;
            }

            d = bmax;
            while d >= bmin {
                let mut i1 = if *kvdb.offset((d - 1) as isize) < *kvdb.offset((d + 1) as isize) {
                    *kvdb.offset((d - 1) as isize)
                } else {
                    *kvdb.offset((d + 1) as isize) - 1
                };
                let prev1 = i1;
                let mut i2 = i1 - d;
                while i1 > off1
                    && i2 > off2
                    && *ha1.offset((i1 - 1) as isize) == *ha2.offset((i2 - 1) as isize)
                {
                    i1 -= 1;
                    i2 -= 1;
                }
                if prev1 - i1 > (*xenv).snake_cnt {
                    got_snake = true;
                }
                *kvdb.offset(d as isize) = i1;
                if !odd && fmin <= d && d <= fmax && i1 <= *kvdf.offset(d as isize) {
                    (*spl).i1 = i1;
                    (*spl).i2 = i2;
                    (*spl).min_lo = 1;
                    (*spl).min_hi = 1;
                    return ec;
                }
                d -= 2;
            }

            if need_min != 0 {
                ec += 1;
                continue;
            }

            if got_snake && ec > (*xenv).heur_min {
                let mut best: c_long = 0;
                d = fmax;
                while d >= fmin {
                    let dd = if d > fmid { d - fmid } else { fmid - d };
                    let i1 = *kvdf.offset(d as isize);
                    let i2 = i1 - d;
                    let v = (i1 - off1) + (i2 - off2) - dd;

                    if v > XDL_K_HEUR * ec
                        && v > best
                        && off1 + (*xenv).snake_cnt <= i1
                        && i1 < lim1
                        && off2 + (*xenv).snake_cnt <= i2
                        && i2 < lim2
                    {
                        let mut k: c_long = 1;
                        while *ha1.offset((i1 - k) as isize) == *ha2.offset((i2 - k) as isize) {
                            if k == (*xenv).snake_cnt {
                                best = v;
                                (*spl).i1 = i1;
                                (*spl).i2 = i2;
                                break;
                            }
                            k += 1;
                        }
                    }
                    d -= 2;
                }
                if best > 0 {
                    (*spl).min_lo = 1;
                    (*spl).min_hi = 0;
                    return ec;
                }

                best = 0;
                d = bmax;
                while d >= bmin {
                    let dd = if d > bmid { d - bmid } else { bmid - d };
                    let i1 = *kvdb.offset(d as isize);
                    let i2 = i1 - d;
                    let v = (lim1 - i1) + (lim2 - i2) - dd;

                    if v > XDL_K_HEUR * ec
                        && v > best
                        && off1 < i1
                        && i1 <= lim1 - (*xenv).snake_cnt
                        && off2 < i2
                        && i2 <= lim2 - (*xenv).snake_cnt
                    {
                        let mut k: c_long = 0;
                        while *ha1.offset((i1 + k) as isize) == *ha2.offset((i2 + k) as isize) {
                            if k == (*xenv).snake_cnt - 1 {
                                best = v;
                                (*spl).i1 = i1;
                                (*spl).i2 = i2;
                                break;
                            }
                            k += 1;
                        }
                    }
                    d -= 2;
                }
                if best > 0 {
                    (*spl).min_lo = 0;
                    (*spl).min_hi = 1;
                    return ec;
                }
            }

            if ec >= (*xenv).mxcost {
                let mut fbest: c_long = -1;
                let mut fbest1: c_long = -1;
                d = fmax;
                while d >= fmin {
                    let mut i1 = (*kvdf.offset(d as isize)).min(lim1);
                    let mut i2 = i1 - d;
                    if lim2 < i2 {
                        i1 = lim2 + d;
                        i2 = lim2;
                    }
                    if fbest < i1 + i2 {
                        fbest = i1 + i2;
                        fbest1 = i1;
                    }
                    d -= 2;
                }

                let mut bbest: c_long = XDL_LINE_MAX;
                let mut bbest1: c_long = XDL_LINE_MAX;
                d = bmax;
                while d >= bmin {
                    let mut i1 = off1.max(*kvdb.offset(d as isize));
                    let mut i2 = i1 - d;
                    if i2 < off2 {
                        i1 = off2 + d;
                        i2 = off2;
                    }
                    if i1 + i2 < bbest {
                        bbest = i1 + i2;
                        bbest1 = i1;
                    }
                    d -= 2;
                }

                if (lim1 + lim2) - bbest < fbest - (off1 + off2) {
                    (*spl).i1 = fbest1;
                    (*spl).i2 = fbest - fbest1;
                    (*spl).min_lo = 1;
                    (*spl).min_hi = 0;
                } else {
                    (*spl).i1 = bbest1;
                    (*spl).i2 = bbest - bbest1;
                    (*spl).min_lo = 0;
                    (*spl).min_hi = 1;
                }
                return ec;
            }

            ec += 1;
        }
    }
}

pub unsafe fn xdl_recs_cmp(
    dd1: *mut diffdata_t,
    mut off1: c_long,
    mut lim1: c_long,
    dd2: *mut diffdata_t,
    mut off2: c_long,
    mut lim2: c_long,
    kvdf: *mut c_long,
    kvdb: *mut c_long,
    need_min: c_int,
    xenv: *mut xdalgoenv_t,
) -> c_int {
    unsafe {
        let ha1 = (*dd1).ha;
        let ha2 = (*dd2).ha;

        while off1 < lim1 && off2 < lim2 && *ha1.offset(off1 as isize) == *ha2.offset(off2 as isize)
        {
            off1 += 1;
            off2 += 1;
        }
        while off1 < lim1
            && off2 < lim2
            && *ha1.offset((lim1 - 1) as isize) == *ha2.offset((lim2 - 1) as isize)
        {
            lim1 -= 1;
            lim2 -= 1;
        }

        if off1 == lim1 {
            let rchg2 = (*dd2).rchg;
            let rindex2 = (*dd2).rindex;
            while off2 < lim2 {
                *rchg2.offset(*rindex2.offset(off2 as isize) as isize) = 1;
                off2 += 1;
            }
        } else if off2 == lim2 {
            let rchg1 = (*dd1).rchg;
            let rindex1 = (*dd1).rindex;
            while off1 < lim1 {
                *rchg1.offset(*rindex1.offset(off1 as isize) as isize) = 1;
                off1 += 1;
            }
        } else {
            let mut spl = xdpsplit_t {
                i1: 0,
                i2: 0,
                min_lo: 0,
                min_hi: 0,
            };

            if xdl_split(
                ha1, off1, lim1, ha2, off2, lim2, kvdf, kvdb, need_min, &mut spl, xenv,
            ) < 0
            {
                return -1;
            }

            if xdl_recs_cmp(
                dd1, off1, spl.i1, dd2, off2, spl.i2, kvdf, kvdb, spl.min_lo, xenv,
            ) < 0
                || xdl_recs_cmp(
                    dd1, spl.i1, lim1, dd2, spl.i2, lim2, kvdf, kvdb, spl.min_hi, xenv,
                ) < 0
            {
                return -1;
            }
        }
    }
    0
}

pub unsafe fn xdl_do_diff(
    mf1: *mut mmfile_t,
    mf2: *mut mmfile_t,
    xpp: *const xpparam_t,
    xe: *mut xdfenv_t,
) -> c_int {
    unsafe {
        if xdf_diff_alg((*xpp).flags) == XDF_PATIENCE_DIFF {
            return xdl_do_patience_diff(mf1, mf2, xpp, xe);
        }

        if xdf_diff_alg((*xpp).flags) == XDF_HISTOGRAM_DIFF {
            return xdl_do_histogram_diff(mf1, mf2, xpp, xe);
        }

        if xdl_prepare_env(mf1, mf2, xpp, xe) < 0 {
            return -1;
        }

        let ndiags = (*xe).xdf1.nreff + (*xe).xdf2.nreff + 3;
        let mut kvd_vec = vec![0 as c_long; (2 * ndiags + 2) as usize];
        let kvd = kvd_vec.as_mut_ptr();
        let mut kvdf = kvd;
        let mut kvdb = kvdf.offset(ndiags as isize);
        kvdf = kvdf.offset(((*xe).xdf2.nreff + 1) as isize);
        kvdb = kvdb.offset(((*xe).xdf2.nreff + 1) as isize);

        let mut xenv = xdalgoenv_t {
            mxcost: xdl_bogosqrt(ndiags).max(XDL_MAX_COST_MIN),
            snake_cnt: XDL_SNAKE_CNT,
            heur_min: XDL_HEUR_MIN_COST,
        };

        let mut dd1 = diffdata_t {
            nrec: (*xe).xdf1.nreff,
            ha: (*xe).xdf1.ha.as_ptr(),
            rchg: (*xe).xdf1.rchg,
            rindex: (*xe).xdf1.rindex.as_mut_ptr(),
        };
        let mut dd2 = diffdata_t {
            nrec: (*xe).xdf2.nreff,
            ha: (*xe).xdf2.ha.as_ptr(),
            rchg: (*xe).xdf2.rchg,
            rindex: (*xe).xdf2.rindex.as_mut_ptr(),
        };

        if xdl_recs_cmp(
            &mut dd1,
            0,
            dd1.nrec,
            &mut dd2,
            0,
            dd2.nrec,
            kvdf,
            kvdb,
            c_int::from(((*xpp).flags & XDF_NEED_MINIMAL) != 0),
            &mut xenv,
        ) < 0
        {
            *xe = xdfenv_t::new();
            return -1;
        }
    }
    0
}

fn recs_match(rec1: &xrecord_t, rec2: &xrecord_t, flags: c_long) -> bool {
    unsafe {
        rec1.ha == rec2.ha && xdl_recmatch(rec1.ptr, rec1.size, rec2.ptr, rec2.size, flags) != 0
    }
}

fn xget_indent(rec: &xrecord_t) -> c_int {
    let mut ret: c_int = 0;
    unsafe {
        for i in 0..rec.size {
            let c = *rec.ptr.offset(i as isize);
            if !xdl_isspace(c) {
                return ret;
            } else if c == b' ' as c_char {
                ret += 1;
            } else if c == b'\t' as c_char {
                ret += 8 - ret % 8;
            }

            if ret >= MAX_INDENT {
                return MAX_INDENT;
            }
        }
    }
    -1
}

#[derive(Copy, Clone, Default)]
struct SplitMeasurement {
    end_of_file: c_int,
    indent: c_int,
    pre_blank: c_int,
    pre_indent: c_int,
    post_blank: c_int,
    post_indent: c_int,
}

#[derive(Copy, Clone, Default)]
struct SplitScore {
    effective_indent: c_int,
    penalty: c_int,
}

unsafe fn measure_split(xdf: *const xdfile_t, split: c_long, m: &mut SplitMeasurement) {
    unsafe {
        let recs = &(*xdf).recs;
        if split >= (*xdf).nrec {
            m.end_of_file = 1;
            m.indent = -1;
        } else {
            m.end_of_file = 0;
            m.indent = xget_indent(&recs[split as usize]);
        }

        m.pre_blank = 0;
        m.pre_indent = -1;
        let mut i = split - 1;
        while i >= 0 {
            m.pre_indent = xget_indent(&recs[i as usize]);
            if m.pre_indent != -1 {
                break;
            }
            m.pre_blank += 1;
            if m.pre_blank == MAX_BLANKS {
                m.pre_indent = 0;
                break;
            }
            i -= 1;
        }

        m.post_blank = 0;
        m.post_indent = -1;
        i = split + 1;
        while i < (*xdf).nrec {
            m.post_indent = xget_indent(&recs[i as usize]);
            if m.post_indent != -1 {
                break;
            }
            m.post_blank += 1;
            if m.post_blank == MAX_BLANKS {
                m.post_indent = 0;
                break;
            }
            i += 1;
        }
    }
}

fn score_add_split(m: &SplitMeasurement, s: &mut SplitScore) {
    if m.pre_indent == -1 && m.pre_blank == 0 {
        s.penalty += START_OF_FILE_PENALTY;
    }

    if m.end_of_file != 0 {
        s.penalty += END_OF_FILE_PENALTY;
    }

    let post_blank = if m.indent == -1 { 1 + m.post_blank } else { 0 };
    let total_blank = m.pre_blank + post_blank;

    s.penalty += TOTAL_BLANK_WEIGHT * total_blank;
    s.penalty += POST_BLANK_WEIGHT * post_blank;

    let indent = if m.indent != -1 {
        m.indent
    } else {
        m.post_indent
    };
    let any_blanks = total_blank != 0;

    s.effective_indent += indent;

    if indent == -1 || m.pre_indent == -1 || indent == m.pre_indent {
        // No additional adjustments needed.
    } else if indent > m.pre_indent {
        s.penalty += if any_blanks {
            RELATIVE_INDENT_WITH_BLANK_PENALTY
        } else {
            RELATIVE_INDENT_PENALTY
        };
    } else if m.post_indent != -1 && m.post_indent > indent {
        s.penalty += if any_blanks {
            RELATIVE_OUTDENT_WITH_BLANK_PENALTY
        } else {
            RELATIVE_OUTDENT_PENALTY
        };
    } else {
        s.penalty += if any_blanks {
            RELATIVE_DEDENT_WITH_BLANK_PENALTY
        } else {
            RELATIVE_DEDENT_PENALTY
        };
    }
}

fn score_cmp(s1: &SplitScore, s2: &SplitScore) -> c_int {
    let cmp_indents = c_int::from(s1.effective_indent > s2.effective_indent)
        - c_int::from(s1.effective_indent < s2.effective_indent);
    INDENT_WEIGHT * cmp_indents + (s1.penalty - s2.penalty)
}

#[derive(Copy, Clone, Default)]
struct XdlGroup {
    start: c_long,
    end: c_long,
}

unsafe fn group_init(xdf: *mut xdfile_t, g: &mut XdlGroup) {
    g.start = 0;
    g.end = 0;
    unsafe {
        while *(*xdf).rchg.offset(g.end as isize) != 0 {
            g.end += 1;
        }
    }
}

#[inline]
unsafe fn group_next(xdf: *mut xdfile_t, g: &mut XdlGroup) -> c_int {
    unsafe {
        if g.end == (*xdf).nrec {
            return -1;
        }
        g.start = g.end + 1;
        g.end = g.start;
        while *(*xdf).rchg.offset(g.end as isize) != 0 {
            g.end += 1;
        }
    }
    0
}

#[inline]
unsafe fn group_previous(xdf: *mut xdfile_t, g: &mut XdlGroup) -> c_int {
    if g.start == 0 {
        return -1;
    }
    g.end = g.start - 1;
    g.start = g.end;
    unsafe {
        while *(*xdf).rchg.offset((g.start - 1) as isize) != 0 {
            g.start -= 1;
        }
    }
    0
}

unsafe fn group_slide_down(xdf: *mut xdfile_t, g: &mut XdlGroup, flags: c_long) -> c_int {
    unsafe {
        let recs = &(*xdf).recs;
        if g.end < (*xdf).nrec && recs_match(&recs[g.start as usize], &recs[g.end as usize], flags)
        {
            *(*xdf).rchg.offset(g.start as isize) = 0;
            g.start += 1;
            *(*xdf).rchg.offset(g.end as isize) = 1;
            g.end += 1;

            while *(*xdf).rchg.offset(g.end as isize) != 0 {
                g.end += 1;
            }
            0
        } else {
            -1
        }
    }
}

unsafe fn group_slide_up(xdf: *mut xdfile_t, g: &mut XdlGroup, flags: c_long) -> c_int {
    unsafe {
        let recs = &(*xdf).recs;
        if g.start > 0
            && recs_match(
                &recs[(g.start - 1) as usize],
                &recs[(g.end - 1) as usize],
                flags,
            )
        {
            g.start -= 1;
            *(*xdf).rchg.offset(g.start as isize) = 1;
            g.end -= 1;
            *(*xdf).rchg.offset(g.end as isize) = 0;

            while *(*xdf).rchg.offset((g.start - 1) as isize) != 0 {
                g.start -= 1;
            }
            0
        } else {
            -1
        }
    }
}

fn xdl_bug(msg: &str) -> ! {
    eprintln!("BUG: {msg}");
    std::process::exit(1);
}

pub unsafe fn xdl_change_compact(xdf: *mut xdfile_t, xdfo: *mut xdfile_t, flags: c_long) -> c_int {
    let mut g = XdlGroup::default();
    let mut go = XdlGroup::default();

    unsafe {
        group_init(xdf, &mut g);
        group_init(xdfo, &mut go);

        loop {
            if g.end != g.start {
                let mut earliest_end: c_long;
                let mut end_matching_other: c_long;
                let mut groupsize: c_long;

                loop {
                    groupsize = g.end - g.start;
                    end_matching_other = -1;

                    while group_slide_up(xdf, &mut g, flags) == 0 {
                        if group_previous(xdfo, &mut go) != 0 {
                            xdl_bug("group sync broken sliding up");
                        }
                    }

                    earliest_end = g.end;
                    if go.end > go.start {
                        end_matching_other = g.end;
                    }

                    loop {
                        if group_slide_down(xdf, &mut g, flags) != 0 {
                            break;
                        }
                        if group_next(xdfo, &mut go) != 0 {
                            xdl_bug("group sync broken sliding down");
                        }
                        if go.end > go.start {
                            end_matching_other = g.end;
                        }
                    }

                    if groupsize == g.end - g.start {
                        break;
                    }
                }

                if g.end == earliest_end {
                    // no shifting was possible
                } else if end_matching_other != -1 {
                    while go.end == go.start {
                        if group_slide_up(xdf, &mut g, flags) != 0 {
                            xdl_bug("match disappeared");
                        }
                        if group_previous(xdfo, &mut go) != 0 {
                            xdl_bug("group sync broken sliding to match");
                        }
                    }
                } else if ((flags as c_ulong) & XDF_INDENT_HEURISTIC) != 0 {
                    let mut shift = earliest_end;
                    if g.end - groupsize - 1 > shift {
                        shift = g.end - groupsize - 1;
                    }
                    if g.end - INDENT_HEURISTIC_MAX_SLIDING > shift {
                        shift = g.end - INDENT_HEURISTIC_MAX_SLIDING;
                    }
                    let mut best_shift: c_long = -1;
                    let mut best_score = SplitScore::default();

                    while shift <= g.end {
                        let mut m = SplitMeasurement::default();
                        let mut score = SplitScore::default();

                        measure_split(xdf, shift, &mut m);
                        score_add_split(&m, &mut score);
                        measure_split(xdf, shift - groupsize, &mut m);
                        score_add_split(&m, &mut score);
                        if best_shift == -1 || score_cmp(&score, &best_score) <= 0 {
                            best_score = score;
                            best_shift = shift;
                        }
                        shift += 1;
                    }

                    while g.end > best_shift {
                        if group_slide_up(xdf, &mut g, flags) != 0 {
                            xdl_bug("best shift unreached");
                        }
                        if group_previous(xdfo, &mut go) != 0 {
                            xdl_bug("group sync broken sliding to blank line");
                        }
                    }
                }
            }

            if group_next(xdf, &mut g) != 0 {
                break;
            }
            if group_next(xdfo, &mut go) != 0 {
                xdl_bug("group sync broken moving to next group");
            }
        }

        if group_next(xdfo, &mut go) == 0 {
            xdl_bug("group sync broken at end of file");
        }
    }

    0
}

pub unsafe fn xdl_build_script(xe: *mut xdfenv_t, xscr: &mut Vec<xdchange_t>) -> c_int {
    xscr.clear();
    unsafe {
        let rchg1 = (*xe).xdf1.rchg;
        let rchg2 = (*xe).xdf2.rchg;
        let mut i1 = (*xe).xdf1.nrec;
        let mut i2 = (*xe).xdf2.nrec;

        while i1 >= 0 || i2 >= 0 {
            if *rchg1.offset((i1 - 1) as isize) != 0 || *rchg2.offset((i2 - 1) as isize) != 0 {
                let l1 = i1;
                while *rchg1.offset((i1 - 1) as isize) != 0 {
                    i1 -= 1;
                }
                let l2 = i2;
                while *rchg2.offset((i2 - 1) as isize) != 0 {
                    i2 -= 1;
                }

                xscr.push(xdchange_t {
                    i1,
                    i2,
                    chg1: l1 - i1,
                    chg2: l2 - i2,
                    ignore: 0,
                });
            }
            i1 -= 1;
            i2 -= 1;
        }

        xscr.reverse();
    }
    0
}

unsafe fn xdl_call_hunk_func(
    _xe: *mut xdfenv_t,
    xscr: &[xdchange_t],
    ecb: *mut xdemitcb_t,
    xecfg: *const xdemitconf_t,
) -> c_int {
    let mut idx = 0;
    unsafe {
        while let Some(end_idx) = xdl_get_hunk(xscr, &mut idx, xecfg) {
            let xch = &xscr[idx];
            let xche = &xscr[end_idx];
            if (*ecb).call_hunk(
                xch.i1 as c_int,
                (xche.i1 + xche.chg1 - xch.i1) as c_int,
                xch.i2 as c_int,
                (xche.i2 + xche.chg2 - xch.i2) as c_int,
            ) < 0
            {
                return -1;
            }
            idx = end_idx + 1;
        }
    }
    0
}

unsafe fn xdl_mark_ignorable_lines(xscr: &mut [xdchange_t], xe: *mut xdfenv_t, flags: c_long) {
    unsafe {
        let recs1 = &(*xe).xdf1.recs;
        let recs2 = &(*xe).xdf2.recs;
        for xch in xscr {
            let mut ignore: c_int = 1;
            let mut i: c_long = 0;
            while i < xch.chg1 && ignore != 0 {
                let r = &recs1[(xch.i1 + i) as usize];
                ignore = xdl_blankline(r.ptr, r.size, flags);
                i += 1;
            }

            i = 0;
            while i < xch.chg2 && ignore != 0 {
                let r = &recs2[(xch.i2 + i) as usize];
                ignore = xdl_blankline(r.ptr, r.size, flags);
                i += 1;
            }

            xch.ignore = ignore;
        }
    }
}

unsafe fn xdl_emit_diff_wrapper(
    xe: *mut xdfenv_t,
    xscr: &[xdchange_t],
    ecb: *mut xdemitcb_t,
    xecfg: *const xdemitconf_t,
) -> c_int {
    unsafe { xdl_emit_diff(xe, xscr, ecb, xecfg) }
}

pub unsafe fn xdl_diff(
    mf1: *mut mmfile_t,
    mf2: *mut mmfile_t,
    xpp: *const xpparam_t,
    xecfg: *const xdemitconf_t,
    ecb: *mut xdemitcb_t,
) -> c_int {
    unsafe {
        let mut xscr: Vec<xdchange_t> = Vec::new();
        let mut xe = xdfenv_t::new();
        let ef: emit_func_t = if (*ecb).has_hunk_func() {
            xdl_call_hunk_func
        } else {
            xdl_emit_diff_wrapper
        };

        if xdl_do_diff(mf1, mf2, xpp, &mut xe) < 0 {
            return -1;
        }
        if xdl_change_compact(&mut xe.xdf1, &mut xe.xdf2, (*xpp).flags as c_long) < 0
            || xdl_change_compact(&mut xe.xdf2, &mut xe.xdf1, (*xpp).flags as c_long) < 0
            || xdl_build_script(&mut xe, &mut xscr) < 0
        {
            return -1;
        }
        if !xscr.is_empty() {
            if ((*xpp).flags & XDF_IGNORE_BLANK_LINES) != 0 {
                xdl_mark_ignorable_lines(&mut xscr, &mut xe, (*xpp).flags as c_long);
            }

            if ef(&mut xe, &xscr, ecb, xecfg) < 0 {
                return -1;
            }
        }
    }

    0
}
