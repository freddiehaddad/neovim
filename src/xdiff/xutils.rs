//! XDiff utility functions (`src/xdiff/xutils.c`).

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::mem::size_of;
use std::ptr;

use crate::memory::{xfree, xmalloc};
use crate::xdiff::xdiffi::xdl_do_diff;
use crate::xdiff::xprepare::xdl_free_env;
use crate::xdiff::{
    XDF_IGNORE_CR_AT_EOL, XDF_IGNORE_WHITESPACE, XDF_IGNORE_WHITESPACE_AT_EOL,
    XDF_IGNORE_WHITESPACE_CHANGE, XDF_WHITESPACE_FLAGS, chanode_t, chastore_t, mmbuffer_t,
    mmfile_t, xdemitcb_t, xdfenv_t, xdl_isspace, xpparam_t,
};

unsafe extern "C" {
    fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void;
    fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int;
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn xdl_bogosqrt(mut n: c_long) -> c_long {
    let mut i: c_long = 1;
    while n > 0 {
        i <<= 1;
        n >>= 2;
    }
    i
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_emit_diffrec(
    rec: *const c_char,
    size: c_long,
    pre: *const c_char,
    psize: c_long,
    ecb: *mut xdemitcb_t,
) -> c_int {
    let mut i: c_int = 2;
    let mut mb = [mmbuffer_t {
        ptr: ptr::null_mut(),
        size: 0,
    }; 3];

    mb[0].ptr = pre.cast_mut();
    mb[0].size = psize as c_int;
    mb[1].ptr = rec.cast_mut();
    mb[1].size = size as c_int;
    if size > 0 && unsafe { *rec.offset((size - 1) as isize) } != b'\n' as c_char {
        const NO_NL: &[u8] = b"\n\\ No newline at end of file\n\0";
        mb[2].ptr = NO_NL.as_ptr().cast::<c_char>().cast_mut();
        mb[2].size = (NO_NL.len() - 1) as c_int;
        i += 1;
    }
    if let Some(out_line) = unsafe { (*ecb).out_line }
        && unsafe { out_line((*ecb).priv_data, mb.as_mut_ptr(), i) } < 0
    {
        return -1;
    }

    0
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_mmfile_first(mmf: *mut mmfile_t, size: *mut c_long) -> *mut c_void {
    unsafe {
        *size = (*mmf).size as c_long;
        (*mmf).ptr.cast::<c_void>()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_mmfile_size(mmf: *mut mmfile_t) -> c_long {
    unsafe { (*mmf).size as c_long }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_cha_init(
    cha: *mut chastore_t,
    isize: c_long,
    icount: c_long,
) -> c_int {
    unsafe {
        (*cha).head = ptr::null_mut();
        (*cha).tail = ptr::null_mut();
        (*cha).isize = isize;
        (*cha).nsize = icount * isize;
        (*cha).ancur = ptr::null_mut();
        (*cha).sncur = ptr::null_mut();
        (*cha).scurr = 0;
    }
    0
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_cha_free(cha: *mut chastore_t) {
    let mut cur = unsafe { (*cha).head };
    while !cur.is_null() {
        let tmp = cur;
        cur = unsafe { (*cur).next };
        unsafe { xfree(tmp.cast::<c_void>()) };
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_cha_alloc(cha: *mut chastore_t) -> *mut c_void {
    let mut ancur = unsafe { (*cha).ancur };
    if ancur.is_null() || unsafe { (*ancur).icurr == (*cha).nsize } {
        ancur =
            unsafe { xmalloc(size_of::<chanode_t>() + (*cha).nsize as usize) }.cast::<chanode_t>();
        if ancur.is_null() {
            return ptr::null_mut();
        }
        unsafe {
            (*ancur).icurr = 0;
            (*ancur).next = ptr::null_mut();
            if !(*cha).tail.is_null() {
                (*(*cha).tail).next = ancur;
            }
            if (*cha).head.is_null() {
                (*cha).head = ancur;
            }
            (*cha).tail = ancur;
            (*cha).ancur = ancur;
        }
    }

    let data = unsafe {
        ancur
            .cast::<c_char>()
            .add(size_of::<chanode_t>())
            .offset((*ancur).icurr as isize)
            .cast::<c_void>()
    };
    unsafe {
        (*ancur).icurr += (*cha).isize;
    }
    data
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_guess_lines(mf: *mut mmfile_t, sample: c_long) -> c_long {
    let mut nl: c_long = 0;
    let mut size: c_long = 0;
    let mut tsize: c_long = 0;

    let data = unsafe { xdl_mmfile_first(mf, &mut size) }.cast::<c_char>();
    let mut cur = data;
    if !cur.is_null() {
        let top = unsafe { data.offset(size as isize) };
        while nl < sample && cur < top {
            nl += 1;
            let found = unsafe {
                memchr(
                    cur.cast::<c_void>(),
                    c_int::from(b'\n'),
                    top.offset_from(cur) as usize,
                )
            }
            .cast::<c_char>();
            if found.is_null() {
                cur = top;
            } else {
                cur = unsafe { found.add(1) };
            }
        }
        tsize += unsafe { cur.offset_from(data) } as c_long;
    }

    if nl != 0 && tsize != 0 {
        nl = unsafe { xdl_mmfile_size(mf) } / (tsize / nl);
    }

    nl + 1
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_blankline(line: *const c_char, size: c_long, flags: c_long) -> c_int {
    if ((flags as c_ulong) & XDF_WHITESPACE_FLAGS) == 0 {
        return c_int::from(size <= 1);
    }

    let mut i: c_long = 0;
    while i < size && xdl_isspace(unsafe { *line.offset(i as isize) }) {
        i += 1;
    }

    c_int::from(i == size)
}

unsafe fn ends_with_optional_cr(l: *const c_char, mut s: c_long, i: c_long) -> bool {
    let complete = s != 0 && unsafe { *l.offset((s - 1) as isize) } == b'\n' as c_char;

    if complete {
        s -= 1;
    }
    if s == i {
        return true;
    }
    if complete && s == i + 1 && unsafe { *l.offset(i as isize) } == b'\r' as c_char {
        return true;
    }
    false
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_recmatch(
    l1: *const c_char,
    s1: c_long,
    l2: *const c_char,
    s2: c_long,
    flags: c_long,
) -> c_int {
    let uflags = flags as c_ulong;

    if s1 == s2 && unsafe { memcmp(l1.cast::<c_void>(), l2.cast::<c_void>(), s1 as usize) } == 0 {
        return 1;
    }
    if (uflags & XDF_WHITESPACE_FLAGS) == 0 {
        return 0;
    }

    let mut i1: c_long = 0;
    let mut i2: c_long = 0;

    if (uflags & XDF_IGNORE_WHITESPACE) != 0 {
        while i1 < s1 && xdl_isspace(unsafe { *l1.offset(i1 as isize) }) {
            i1 += 1;
        }
        while i2 < s2 && xdl_isspace(unsafe { *l2.offset(i2 as isize) }) {
            i2 += 1;
        }
        while i1 < s1 && i2 < s2 {
            let c1 = unsafe { *l1.offset(i1 as isize) };
            let c2 = unsafe { *l2.offset(i2 as isize) };
            i1 += 1;
            i2 += 1;
            if c1 != c2 {
                return 0;
            }
            while i1 < s1 && xdl_isspace(unsafe { *l1.offset(i1 as isize) }) {
                i1 += 1;
            }
            while i2 < s2 && xdl_isspace(unsafe { *l2.offset(i2 as isize) }) {
                i2 += 1;
            }
        }
    } else if (uflags & XDF_IGNORE_WHITESPACE_CHANGE) != 0 {
        while i1 < s1 && i2 < s2 {
            if xdl_isspace(unsafe { *l1.offset(i1 as isize) })
                && xdl_isspace(unsafe { *l2.offset(i2 as isize) })
            {
                while i1 < s1 && xdl_isspace(unsafe { *l1.offset(i1 as isize) }) {
                    i1 += 1;
                }
                while i2 < s2 && xdl_isspace(unsafe { *l2.offset(i2 as isize) }) {
                    i2 += 1;
                }
                continue;
            }
            let c1 = unsafe { *l1.offset(i1 as isize) };
            let c2 = unsafe { *l2.offset(i2 as isize) };
            i1 += 1;
            i2 += 1;
            if c1 != c2 {
                return 0;
            }
        }
    } else if (uflags & XDF_IGNORE_WHITESPACE_AT_EOL) != 0 {
        while i1 < s1 && i2 < s2 && unsafe { *l1.offset(i1 as isize) == *l2.offset(i2 as isize) } {
            i1 += 1;
            i2 += 1;
        }
    } else if (uflags & XDF_IGNORE_CR_AT_EOL) != 0 {
        while i1 < s1 && i2 < s2 && unsafe { *l1.offset(i1 as isize) == *l2.offset(i2 as isize) } {
            i1 += 1;
            i2 += 1;
        }
        return c_int::from(unsafe {
            ends_with_optional_cr(l1, s1, i1) && ends_with_optional_cr(l2, s2, i2)
        });
    }

    if i1 < s1 {
        while i1 < s1 && xdl_isspace(unsafe { *l1.offset(i1 as isize) }) {
            i1 += 1;
        }
        if s1 != i1 {
            return 0;
        }
    }
    if i2 < s2 {
        while i2 < s2 && xdl_isspace(unsafe { *l2.offset(i2 as isize) }) {
            i2 += 1;
        }
        return c_int::from(s2 == i2);
    }
    1
}

unsafe fn xdl_hash_record_with_whitespace(
    data: *mut *const c_char,
    top: *const c_char,
    flags: c_long,
) -> c_ulong {
    let mut ha: c_ulong = 5381;
    let mut ptr = unsafe { *data };
    let uflags = flags as c_ulong;
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
                ha ^= b' ' as c_ulong;
            } else if (uflags & XDF_IGNORE_WHITESPACE_AT_EOL) != 0 && !at_eol {
                let end = unsafe { ptr.add(1) };
                while ptr2 != end {
                    ha = ha.wrapping_add(ha << 5);
                    ha ^= unsafe { *ptr2 } as c_ulong;
                    ptr2 = unsafe { ptr2.add(1) };
                }
            }
            ptr = unsafe { ptr.add(1) };
            continue;
        }
        ha = ha.wrapping_add(ha << 5);
        ha ^= unsafe { *ptr } as c_ulong;
        ptr = unsafe { ptr.add(1) };
    }
    unsafe {
        *data = if ptr < top { ptr.add(1) } else { ptr };
    }

    ha
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_hash_record(
    data: *mut *const c_char,
    top: *const c_char,
    flags: c_long,
) -> c_ulong {
    if ((flags as c_ulong) & XDF_WHITESPACE_FLAGS) != 0 {
        return unsafe { xdl_hash_record_with_whitespace(data, top, flags) };
    }

    let mut ha: c_ulong = 5381;
    let mut ptr = unsafe { *data };

    while ptr < top && unsafe { *ptr } != b'\n' as c_char {
        ha = ha.wrapping_add(ha << 5);
        ha ^= unsafe { *ptr } as c_ulong;
        ptr = unsafe { ptr.add(1) };
    }
    unsafe {
        *data = if ptr < top { ptr.add(1) } else { ptr };
    }

    ha
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn xdl_hashbits(size: c_uint) -> c_uint {
    let mut val: c_uint = 1;
    let mut bits: c_uint = 0;
    let max_bits = c_uint::BITS;

    while val < size && bits < max_bits {
        val <<= 1;
        bits += 1;
    }
    if bits != 0 { bits } else { 1 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_num_out(out: *mut c_char, mut val: c_long) -> c_int {
    let mut buf = [0 as c_char; 32];
    let mut ptr = buf.len() - 1;
    buf[ptr] = 0;

    if val < 0 {
        ptr -= 1;
        buf[ptr] = b'-' as c_char;
        val = val.wrapping_neg();
    }
    while val != 0 && ptr > 0 {
        ptr -= 1;
        buf[ptr] = b"0123456789"[(val % 10) as usize] as c_char;
        val /= 10;
    }
    let mut str_ptr = out;
    if buf[ptr] != 0 {
        while buf[ptr] != 0 {
            unsafe {
                *str_ptr = buf[ptr];
                str_ptr = str_ptr.add(1);
            }
            ptr += 1;
        }
    } else {
        unsafe {
            *str_ptr = b'0' as c_char;
            str_ptr = str_ptr.add(1);
        }
    }
    unsafe {
        *str_ptr = 0;
        str_ptr.offset_from(out) as c_int
    }
}

unsafe fn xdl_format_hunk_hdr(
    s1: c_long,
    c1: c_long,
    s2: c_long,
    c2: c_long,
    func: *const c_char,
    mut funclen: c_long,
    ecb: *mut xdemitcb_t,
) -> c_int {
    let mut nb: isize = 0;
    let mut buf = [0 as c_char; 128];
    let buf_ptr = buf.as_mut_ptr();

    unsafe {
        ptr::copy_nonoverlapping(b"@@ -".as_ptr().cast::<c_char>(), buf_ptr, 4);
        nb += 4;

        nb += xdl_num_out(buf_ptr.offset(nb), if c1 != 0 { s1 } else { s1 - 1 }) as isize;

        if c1 != 1 {
            *buf_ptr.offset(nb) = b',' as c_char;
            nb += 1;
            nb += xdl_num_out(buf_ptr.offset(nb), c1) as isize;
        }

        ptr::copy_nonoverlapping(b" +".as_ptr().cast::<c_char>(), buf_ptr.offset(nb), 2);
        nb += 2;

        nb += xdl_num_out(buf_ptr.offset(nb), if c2 != 0 { s2 } else { s2 - 1 }) as isize;

        if c2 != 1 {
            *buf_ptr.offset(nb) = b',' as c_char;
            nb += 1;
            nb += xdl_num_out(buf_ptr.offset(nb), c2) as isize;
        }

        ptr::copy_nonoverlapping(b" @@".as_ptr().cast::<c_char>(), buf_ptr.offset(nb), 3);
        nb += 3;

        if !func.is_null() && funclen != 0 {
            *buf_ptr.offset(nb) = b' ' as c_char;
            nb += 1;
            let max_len = (buf.len() as c_long) - (nb as c_long) - 1;
            if funclen > max_len {
                funclen = max_len;
            }
            if funclen > 0 {
                ptr::copy_nonoverlapping(func, buf_ptr.offset(nb), funclen as usize);
                nb += funclen as isize;
            }
        }
        *buf_ptr.offset(nb) = b'\n' as c_char;
        nb += 1;

        let mut mb = mmbuffer_t {
            ptr: buf_ptr,
            size: nb as c_int,
        };
        if let Some(out_line) = (*ecb).out_line
            && out_line((*ecb).priv_data, &mut mb, 1) < 0
        {
            return -1;
        }
    }
    0
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_emit_hunk_hdr(
    s1: c_long,
    c1: c_long,
    s2: c_long,
    c2: c_long,
    func: *const c_char,
    funclen: c_long,
    ecb: *mut xdemitcb_t,
) -> c_int {
    if let Some(out_hunk) = unsafe { (*ecb).out_hunk } {
        if unsafe {
            out_hunk(
                (*ecb).priv_data,
                if c1 != 0 { s1 } else { s1 - 1 },
                c1,
                if c2 != 0 { s2 } else { s2 - 1 },
                c2,
                func,
                funclen,
            )
        } < 0
        {
            return -1;
        }
        0
    } else {
        unsafe { xdl_format_hunk_hdr(s1, c1, s2, c2, func, funclen, ecb) }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_fall_back_diff(
    diff_env: *mut xdfenv_t,
    xpp: *const xpparam_t,
    line1: c_int,
    count1: c_int,
    line2: c_int,
    count2: c_int,
) -> c_int {
    unsafe {
        let mut subfile1 = mmfile_t {
            ptr: (**(*diff_env).xdf1.recs.offset((line1 - 1) as isize))
                .ptr
                .cast_mut(),
            size: 0,
        };
        let end_rec1 = *(*diff_env).xdf1.recs.offset((line1 + count1 - 2) as isize);
        subfile1.size = (*end_rec1)
            .ptr
            .offset((*end_rec1).size as isize)
            .offset_from(subfile1.ptr) as c_int;

        let mut subfile2 = mmfile_t {
            ptr: (**(*diff_env).xdf2.recs.offset((line2 - 1) as isize))
                .ptr
                .cast_mut(),
            size: 0,
        };
        let end_rec2 = *(*diff_env).xdf2.recs.offset((line2 + count2 - 2) as isize);
        subfile2.size = (*end_rec2)
            .ptr
            .offset((*end_rec2).size as isize)
            .offset_from(subfile2.ptr) as c_int;

        let mut env: xdfenv_t = std::mem::zeroed();
        if xdl_do_diff(&mut subfile1, &mut subfile2, xpp, &mut env) < 0 {
            return -1;
        }

        ptr::copy_nonoverlapping(
            env.xdf1.rchg,
            (*diff_env).xdf1.rchg.offset((line1 - 1) as isize),
            count1 as usize,
        );
        ptr::copy_nonoverlapping(
            env.xdf2.rchg,
            (*diff_env).xdf2.rchg.offset((line2 - 1) as isize),
            count2 as usize,
        );

        xdl_free_env(&mut env);
    }

    0
}
