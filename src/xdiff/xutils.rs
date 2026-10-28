//! XDiff utility functions (`src/xdiff/xutils.c`).

use std::ffi::c_char;
use std::ptr;

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

pub fn xdl_emit_diffrec(
    rec: *const c_char,
    size: i64,
    pre: *const c_char,
    psize: i64,
    ecb: &mut Xdemitcb,
) -> i32 {
    let mut i: usize = 2;
    let mut mb = [Mmbuffer {
        ptr: ptr::null_mut(),
        size: 0,
    }; 3];

    mb[0].ptr = pre.cast_mut();
    mb[0].size = psize as i32;
    mb[1].ptr = rec.cast_mut();
    mb[1].size = size as i32;
    if size > 0 && unsafe { *rec.offset((size - 1) as isize) } != b'\n' as c_char {
        const NO_NL: &[u8] = b"\n\\ No newline at end of file\n\0";
        mb[2].ptr = NO_NL.as_ptr().cast::<c_char>().cast_mut();
        mb[2].size = (NO_NL.len() - 1) as i32;
        i += 1;
    }
    if ecb.out_line(&mb[..i]) < 0 {
        return -1;
    }

    0
}

pub fn xdl_mmfile_first(mmf: &Mmfile, size: &mut i64) -> *mut c_char {
    *size = mmf.size as i64;
    mmf.ptr
}

pub fn xdl_mmfile_size(mmf: &Mmfile) -> i64 {
    mmf.size as i64
}

pub fn xdl_guess_lines(mf: &Mmfile, sample: i64) -> i64 {
    let mut nl: i64 = 0;
    let mut size: i64 = 0;
    let mut tsize: i64 = 0;

    let data = xdl_mmfile_first(mf, &mut size);
    let mut cur = data;
    if !cur.is_null() {
        let top = unsafe { data.offset(size as isize) };
        while nl < sample && cur < top {
            nl += 1;
            let rem = unsafe {
                std::slice::from_raw_parts(cur.cast::<u8>(), top.offset_from(cur) as usize)
            };
            if let Some(pos) = rem.iter().position(|&b| b == b'\n') {
                cur = unsafe { cur.add(pos + 1) };
            } else {
                cur = top;
            }
        }
        tsize += unsafe { cur.offset_from(data) } as i64;
    }

    if nl != 0 && tsize != 0 {
        nl = xdl_mmfile_size(mf) / (tsize / nl);
    }

    nl + 1
}

pub fn xdl_blankline_slice(line: &[u8], flags: u64) -> i32 {
    if (flags & XDF_WHITESPACE_FLAGS) == 0 {
        return i32::from(line.len() <= 1);
    }

    let mut i = 0usize;
    while i < line.len() && xdl_isspace(line[i] as c_char) {
        i += 1;
    }

    i32::from(i == line.len())
}

pub fn xdl_blankline(line: *const c_char, size: i64, flags: i64) -> i32 {
    let slice = unsafe { std::slice::from_raw_parts(line.cast::<u8>(), size.max(0) as usize) };
    xdl_blankline_slice(slice, flags as u64)
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
        while i1 < s1 && xdl_isspace(b1[i1] as c_char) {
            i1 += 1;
        }
        while i2 < s2 && xdl_isspace(b2[i2] as c_char) {
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
            while i1 < s1 && xdl_isspace(b1[i1] as c_char) {
                i1 += 1;
            }
            while i2 < s2 && xdl_isspace(b2[i2] as c_char) {
                i2 += 1;
            }
        }
    } else if (uflags & XDF_IGNORE_WHITESPACE_CHANGE) != 0 {
        while i1 < s1 && i2 < s2 {
            if xdl_isspace(b1[i1] as c_char) && xdl_isspace(b2[i2] as c_char) {
                while i1 < s1 && xdl_isspace(b1[i1] as c_char) {
                    i1 += 1;
                }
                while i2 < s2 && xdl_isspace(b2[i2] as c_char) {
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
        while i1 < s1 && xdl_isspace(b1[i1] as c_char) {
            i1 += 1;
        }
        if s1 != i1 {
            return 0;
        }
    }
    if i2 < s2 {
        while i2 < s2 && xdl_isspace(b2[i2] as c_char) {
            i2 += 1;
        }
        return i32::from(s2 == i2);
    }
    1
}

pub fn xdl_recmatch(l1: *const c_char, s1: i64, l2: *const c_char, s2: i64, flags: i64) -> i32 {
    let b1 = unsafe { std::slice::from_raw_parts(l1.cast::<u8>(), s1.max(0) as usize) };
    let b2 = unsafe { std::slice::from_raw_parts(l2.cast::<u8>(), s2.max(0) as usize) };
    xdl_recmatch_slice(b1, b2, flags as u64)
}

fn xdl_hash_record_with_whitespace(
    data: &mut *const c_char,
    top: *const c_char,
    flags: i64,
) -> u64 {
    let mut ha: u64 = 5381;
    let mut ptr = *data;
    let uflags = flags as u64;
    let cr_at_eol_only = (uflags & XDF_WHITESPACE_FLAGS) == XDF_IGNORE_CR_AT_EOL;

    while ptr < top && unsafe { *ptr } != b'\n' as c_char {
        if cr_at_eol_only {
            if unsafe { *ptr } == b'\r' as c_char
                && unsafe { ptr.add(1) < top && *ptr.add(1) == b'\n' as c_char }
            {
                ptr = unsafe { ptr.add(1) };
                continue;
            }
        } else if xdl_isspace(unsafe { *ptr }) {
            let mut ptr2 = ptr;
            while unsafe {
                ptr.add(1) < top && xdl_isspace(*ptr.add(1)) && *ptr.add(1) != b'\n' as c_char
            } {
                ptr = unsafe { ptr.add(1) };
            }
            let at_eol = unsafe { top <= ptr.add(1) || *ptr.add(1) == b'\n' as c_char };
            if (uflags & XDF_IGNORE_WHITESPACE) != 0 {
                // already handled
            } else if (uflags & XDF_IGNORE_WHITESPACE_CHANGE) != 0 && !at_eol {
                ha = ha.wrapping_add(ha << 5);
                ha ^= b' ' as u64;
            } else if (uflags & XDF_IGNORE_WHITESPACE_AT_EOL) != 0 && !at_eol {
                let end = unsafe { ptr.add(1) };
                while ptr2 != end {
                    ha = ha.wrapping_add(ha << 5);
                    ha ^= unsafe { *ptr2 } as u64;
                    ptr2 = unsafe { ptr2.add(1) };
                }
            }
            ptr = unsafe { ptr.add(1) };
            continue;
        }
        ha = ha.wrapping_add(ha << 5);
        ha ^= unsafe { *ptr } as u64;
        ptr = unsafe { ptr.add(1) };
    }
    *data = if ptr < top {
        unsafe { ptr.add(1) }
    } else {
        ptr
    };

    ha
}

pub fn xdl_hash_record(data: &mut *const c_char, top: *const c_char, flags: i64) -> u64 {
    if ((flags as u64) & XDF_WHITESPACE_FLAGS) != 0 {
        return xdl_hash_record_with_whitespace(data, top, flags);
    }

    let mut ha: u64 = 5381;
    let mut ptr = *data;

    while ptr < top && unsafe { *ptr } != b'\n' as c_char {
        ha = ha.wrapping_add(ha << 5);
        ha ^= unsafe { *ptr } as u64;
        ptr = unsafe { ptr.add(1) };
    }
    *data = if ptr < top {
        unsafe { ptr.add(1) }
    } else {
        ptr
    };

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

fn xdl_format_hunk_hdr(s1: i64, c1: i64, s2: i64, c2: i64, ecb: &mut Xdemitcb) -> i32 {
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
    let nb = cursor.position() as i32;

    let mb = [Mmbuffer {
        ptr: buf.as_mut_ptr().cast::<c_char>(),
        size: nb,
    }];
    if ecb.out_line(&mb) < 0 {
        return -1;
    }
    0
}

pub fn xdl_emit_hunk_hdr(s1: i64, c1: i64, s2: i64, c2: i64, ecb: &mut Xdemitcb) -> i32 {
    xdl_format_hunk_hdr(s1, c1, s2, c2, ecb)
}

pub fn xdl_fall_back_diff(
    diff_env: &mut Xdfenv,
    xpp: &Xpparam,
    line1: i32,
    count1: i32,
    line2: i32,
    count2: i32,
) -> i32 {
    unsafe {
        let mut subfile1 = Mmfile {
            ptr: diff_env.xdf1.recs[(line1 - 1) as usize].ptr.cast_mut(),
            size: 0,
        };
        let end_rec1 = &diff_env.xdf1.recs[(line1 + count1 - 2) as usize];
        subfile1.size = end_rec1
            .ptr
            .offset(end_rec1.size as isize)
            .offset_from(subfile1.ptr) as i32;

        let mut subfile2 = Mmfile {
            ptr: diff_env.xdf2.recs[(line2 - 1) as usize].ptr.cast_mut(),
            size: 0,
        };
        let end_rec2 = &diff_env.xdf2.recs[(line2 + count2 - 2) as usize];
        subfile2.size = end_rec2
            .ptr
            .offset(end_rec2.size as isize)
            .offset_from(subfile2.ptr) as i32;

        let mut env = Xdfenv::new();
        if xdl_do_diff(&mut subfile1, &mut subfile2, xpp, &mut env) < 0 {
            return -1;
        }

        ptr::copy_nonoverlapping(
            env.xdf1.rchg,
            diff_env.xdf1.rchg.offset((line1 - 1) as isize),
            count1 as usize,
        );
        ptr::copy_nonoverlapping(
            env.xdf2.rchg,
            diff_env.xdf2.rchg.offset((line2 - 1) as isize),
            count2 as usize,
        );
    }

    0
}
