//! XDiff utility functions (`src/xdiff/xutils.c`).

use crate::xdiff::xdiffi::xdl_do_diff;
use crate::xdiff::{
    Mmbuffer, Mmfile, XDF_IGNORE_CR_AT_EOL, XDF_IGNORE_WHITESPACE, XDF_IGNORE_WHITESPACE_AT_EOL,
    XDF_IGNORE_WHITESPACE_CHANGE, XDF_WHITESPACE_FLAGS, Xdemitcb, Xdfenv, Xpparam, xdl_isspace,
};

pub fn xdl_bogosqrt(mut n: i64) -> i64 {
    let mut i: i64 = 1;
    while n > 0 {
        i <<= 1;
        n >>= 2;
    }
    i
}

pub fn xdl_emit_diffrec(rec: &[u8], pre: &[u8], ecb: &mut Xdemitcb<'_, '_>) -> i32 {
    let mut i: usize = 2;
    let mut mb = [Mmbuffer::default(); 3];

    mb[0].slice = pre;
    mb[1].slice = rec;
    if !rec.is_empty() && rec[rec.len() - 1] != b'\n' {
        mb[2].slice = b"\n\\ No newline at end of file\n";
        i += 1;
    }
    if ecb.out_line(&mb[..i]) < 0 {
        return -1;
    }

    0
}

pub fn xdl_guess_lines(mf: &Mmfile<'_>, sample: i64) -> i64 {
    let mut nl: i64 = 0;
    let data = mf.slice;
    let mut cur = 0usize;
    while nl < sample && cur < data.len() {
        nl += 1;
        if let Some(pos) = data[cur..].iter().position(|&b| b == b'\n') {
            cur += pos + 1;
        } else {
            cur = data.len();
        }
    }
    let tsize = cur as i64;

    if nl != 0 && tsize != 0 {
        nl = (data.len() as i64) / (tsize / nl);
    }

    nl + 1
}

pub fn xdl_blankline_slice(line: &[u8], flags: u64) -> i32 {
    if (flags & XDF_WHITESPACE_FLAGS) == 0 {
        return i32::from(line.len() <= 1);
    }

    let mut i = 0usize;
    while i < line.len() && xdl_isspace(line[i]) {
        i += 1;
    }

    i32::from(i == line.len())
}

fn ends_with_optional_cr_slice(l: &[u8], i: usize) -> bool {
    let mut s = l.len();
    let complete = s != 0 && l[s - 1] == b'\n';

    if complete {
        s -= 1;
    }
    if s == i {
        return true;
    }
    if complete && s == i + 1 && l[i] == b'\r' {
        return true;
    }
    false
}

pub fn xdl_recmatch_slice(b1: &[u8], b2: &[u8], uflags: u64) -> i32 {
    if b1 == b2 {
        return 1;
    }
    if (uflags & XDF_WHITESPACE_FLAGS) == 0 {
        return 0;
    }

    let s1 = b1.len();
    let s2 = b2.len();
    let mut i1 = 0usize;
    let mut i2 = 0usize;

    if (uflags & XDF_IGNORE_WHITESPACE) != 0 {
        while i1 < s1 && xdl_isspace(b1[i1]) {
            i1 += 1;
        }
        while i2 < s2 && xdl_isspace(b2[i2]) {
            i2 += 1;
        }
        while i1 < s1 && i2 < s2 {
            let c1 = b1[i1];
            let c2 = b2[i2];
            i1 += 1;
            i2 += 1;
            if c1 != c2 {
                return 0;
            }
            while i1 < s1 && xdl_isspace(b1[i1]) {
                i1 += 1;
            }
            while i2 < s2 && xdl_isspace(b2[i2]) {
                i2 += 1;
            }
        }
    } else if (uflags & XDF_IGNORE_WHITESPACE_CHANGE) != 0 {
        while i1 < s1 && i2 < s2 {
            if xdl_isspace(b1[i1]) && xdl_isspace(b2[i2]) {
                while i1 < s1 && xdl_isspace(b1[i1]) {
                    i1 += 1;
                }
                while i2 < s2 && xdl_isspace(b2[i2]) {
                    i2 += 1;
                }
                continue;
            }
            let c1 = b1[i1];
            let c2 = b2[i2];
            i1 += 1;
            i2 += 1;
            if c1 != c2 {
                return 0;
            }
        }
    } else if (uflags & XDF_IGNORE_WHITESPACE_AT_EOL) != 0 {
        while i1 < s1 && i2 < s2 && b1[i1] == b2[i2] {
            i1 += 1;
            i2 += 1;
        }
    } else if (uflags & XDF_IGNORE_CR_AT_EOL) != 0 {
        while i1 < s1 && i2 < s2 && b1[i1] == b2[i2] {
            i1 += 1;
            i2 += 1;
        }
        return i32::from(
            ends_with_optional_cr_slice(b1, i1) && ends_with_optional_cr_slice(b2, i2),
        );
    }

    if i1 < s1 {
        while i1 < s1 && xdl_isspace(b1[i1]) {
            i1 += 1;
        }
        if s1 != i1 {
            return 0;
        }
    }
    if i2 < s2 {
        while i2 < s2 && xdl_isspace(b2[i2]) {
            i2 += 1;
        }
        return i32::from(s2 == i2);
    }
    1
}

fn xdl_hash_record_with_whitespace(blk: &[u8], cur: &mut usize, uflags: u64) -> u64 {
    let mut ha: u64 = 5381;
    let mut idx = *cur;
    let top = blk.len();
    let cr_at_eol_only = (uflags & XDF_WHITESPACE_FLAGS) == XDF_IGNORE_CR_AT_EOL;

    while idx < top && blk[idx] != b'\n' {
        if cr_at_eol_only {
            if blk[idx] == b'\r' && idx + 1 < top && blk[idx + 1] == b'\n' {
                idx += 1;
                continue;
            }
        } else if xdl_isspace(blk[idx]) {
            let mut idx2 = idx;
            while idx + 1 < top && xdl_isspace(blk[idx + 1]) && blk[idx + 1] != b'\n' {
                idx += 1;
            }
            let at_eol = top <= idx + 1 || blk[idx + 1] == b'\n';
            if (uflags & XDF_IGNORE_WHITESPACE) != 0 {
                // already handled
            } else if (uflags & XDF_IGNORE_WHITESPACE_CHANGE) != 0 && !at_eol {
                ha = ha.wrapping_add(ha << 5);
                ha ^= b' ' as u64;
            } else if (uflags & XDF_IGNORE_WHITESPACE_AT_EOL) != 0 && !at_eol {
                let end = idx + 1;
                while idx2 != end {
                    ha = ha.wrapping_add(ha << 5);
                    ha ^= (blk[idx2] as i8) as u64;
                    idx2 += 1;
                }
            }
            idx += 1;
            continue;
        }
        ha = ha.wrapping_add(ha << 5);
        ha ^= (blk[idx] as i8) as u64;
        idx += 1;
    }
    *cur = if idx < top { idx + 1 } else { idx };

    ha
}

pub fn xdl_hash_record(blk: &[u8], cur: &mut usize, uflags: u64) -> u64 {
    if (uflags & XDF_WHITESPACE_FLAGS) != 0 {
        return xdl_hash_record_with_whitespace(blk, cur, uflags);
    }

    let mut ha: u64 = 5381;
    let mut idx = *cur;
    let top = blk.len();

    while idx < top && blk[idx] != b'\n' {
        ha = ha.wrapping_add(ha << 5);
        ha ^= (blk[idx] as i8) as u64;
        idx += 1;
    }
    *cur = if idx < top { idx + 1 } else { idx };

    ha
}

pub fn xdl_hashbits(size: u32) -> u32 {
    let mut val: u32 = 1;
    let mut bits: u32 = 0;
    let max_bits = u32::BITS;

    while val < size && bits < max_bits {
        val <<= 1;
        bits += 1;
    }
    if bits != 0 { bits } else { 1 }
}

fn xdl_format_hunk_hdr(s1: i64, c1: i64, s2: i64, c2: i64, ecb: &mut Xdemitcb<'_, '_>) -> i32 {
    use std::io::Write as _;
    let mut buf = [0u8; 128];
    let mut cursor = std::io::Cursor::new(&mut buf[..]);
    let _ = write!(cursor, "@@ -{}", if c1 != 0 { s1 } else { s1 - 1 });
    if c1 != 1 {
        let _ = write!(cursor, ",{c1}");
    }
    let _ = write!(cursor, " +{}", if c2 != 0 { s2 } else { s2 - 1 });
    if c2 != 1 {
        let _ = write!(cursor, ",{c2}");
    }
    let _ = writeln!(cursor, " @@");
    let nb = cursor.position() as usize;

    let mb = [Mmbuffer { slice: &buf[..nb] }];
    if ecb.out_line(&mb) < 0 {
        return -1;
    }
    0
}

pub fn xdl_emit_hunk_hdr(s1: i64, c1: i64, s2: i64, c2: i64, ecb: &mut Xdemitcb<'_, '_>) -> i32 {
    xdl_format_hunk_hdr(s1, c1, s2, c2, ecb)
}

pub fn xdl_fall_back_diff(
    diff_env: &mut Xdfenv<'_>,
    xpp: &Xpparam,
    line1: i32,
    count1: i32,
    line2: i32,
    count2: i32,
) -> i32 {
    let start1 = diff_env.xdf1.recs[(line1 - 1) as usize].offset;
    let end_rec1 = &diff_env.xdf1.recs[(line1 + count1 - 2) as usize];
    let end1 = end_rec1.offset + end_rec1.slice.len();
    let mut subfile1 = Mmfile {
        slice: &diff_env.xdf1.file_slice[start1..end1],
    };

    let start2 = diff_env.xdf2.recs[(line2 - 1) as usize].offset;
    let end_rec2 = &diff_env.xdf2.recs[(line2 + count2 - 2) as usize];
    let end2 = end_rec2.offset + end_rec2.slice.len();
    let mut subfile2 = Mmfile {
        slice: &diff_env.xdf2.file_slice[start2..end2],
    };

    let mut env = Xdfenv::new();
    if xdl_do_diff(&mut subfile1, &mut subfile2, xpp, &mut env) < 0 {
        return -1;
    }

    let l1 = line1 as usize;
    let c1 = count1 as usize;
    diff_env.xdf1.rchg_vec[l1..l1 + c1].copy_from_slice(&env.xdf1.rchg_vec[1..1 + c1]);

    let l2 = line2 as usize;
    let c2 = count2 as usize;
    diff_env.xdf2.rchg_vec[l2..l2 + c2].copy_from_slice(&env.xdf2.rchg_vec[1..1 + c2]);

    0
}
