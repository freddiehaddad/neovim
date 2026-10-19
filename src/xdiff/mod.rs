//! File differential library (`src/xdiff/`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

pub mod xdiffi;
pub mod xemit;
pub mod xhistogram;
pub mod xpatience;
pub mod xprepare;
pub mod xutils;

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

pub const XDF_NEED_MINIMAL: c_ulong = 1 << 0;

pub const XDF_IGNORE_WHITESPACE: c_ulong = 1 << 1;
pub const XDF_IGNORE_WHITESPACE_CHANGE: c_ulong = 1 << 2;
pub const XDF_IGNORE_WHITESPACE_AT_EOL: c_ulong = 1 << 3;
pub const XDF_IGNORE_CR_AT_EOL: c_ulong = 1 << 4;
pub const XDF_WHITESPACE_FLAGS: c_ulong = XDF_IGNORE_WHITESPACE
    | XDF_IGNORE_WHITESPACE_CHANGE
    | XDF_IGNORE_WHITESPACE_AT_EOL
    | XDF_IGNORE_CR_AT_EOL;

pub const XDF_IGNORE_BLANK_LINES: c_ulong = 1 << 7;

pub const XDF_PATIENCE_DIFF: c_ulong = 1 << 14;
pub const XDF_HISTOGRAM_DIFF: c_ulong = 1 << 15;
pub const XDF_DIFF_ALGORITHM_MASK: c_ulong = XDF_PATIENCE_DIFF | XDF_HISTOGRAM_DIFF;

#[inline]
pub const fn xdf_diff_alg(flags: c_ulong) -> c_ulong {
    flags & XDF_DIFF_ALGORITHM_MASK
}

pub const XDF_INDENT_HEURISTIC: c_ulong = 1 << 23;

pub const XDL_EMIT_FUNCNAMES: c_ulong = 1 << 0;
pub const XDL_EMIT_NO_HUNK_HDR: c_ulong = 1 << 1;
pub const XDL_EMIT_FUNCCONTEXT: c_ulong = 1 << 2;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct mmfile_t {
    pub ptr: *mut c_char,
    pub size: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mmbuffer_t {
    pub ptr: *mut c_char,
    pub size: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xpparam_t {
    pub flags: c_ulong,
    pub anchors: *mut *mut c_char,
    pub anchors_nr: usize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xdemitcb_t {
    pub priv_data: *mut c_void,
    pub out_hunk: Option<
        unsafe extern "C" fn(
            priv_data: *mut c_void,
            old_begin: c_long,
            old_nr: c_long,
            new_begin: c_long,
            new_nr: c_long,
            func: *const c_char,
            funclen: c_long,
        ) -> c_int,
    >,
    pub out_line: Option<
        unsafe extern "C" fn(priv_data: *mut c_void, mb: *mut mmbuffer_t, nbuf: c_int) -> c_int,
    >,
}

pub type find_func_t = Option<
    unsafe extern "C" fn(
        line: *const c_char,
        line_len: c_long,
        buffer: *mut c_char,
        buffer_size: c_long,
        priv_data: *mut c_void,
    ) -> c_long,
>;

pub type xdl_emit_hunk_consume_func_t = Option<
    unsafe extern "C" fn(
        start_a: c_int,
        count_a: c_int,
        start_b: c_int,
        count_b: c_int,
        cb_data: *mut c_void,
    ) -> c_int,
>;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xdemitconf_t {
    pub ctxlen: c_long,
    pub interhunkctxlen: c_long,
    pub flags: c_ulong,
    pub find_func: find_func_t,
    pub find_func_priv: *mut c_void,
    pub hunk_func: xdl_emit_hunk_consume_func_t,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct chanode_t {
    pub next: *mut chanode_t,
    pub icurr: c_long,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct chastore_t {
    pub head: *mut chanode_t,
    pub tail: *mut chanode_t,
    pub isize: c_long,
    pub nsize: c_long,
    pub ancur: *mut chanode_t,
    pub sncur: *mut chanode_t,
    pub scurr: c_long,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xrecord_t {
    pub next: *mut xrecord_t,
    pub ptr: *const c_char,
    pub size: c_long,
    pub ha: c_ulong,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xdfile_t {
    pub rcha: chastore_t,
    pub nrec: c_long,
    pub hbits: c_uint,
    pub rhash: *mut *mut xrecord_t,
    pub dstart: c_long,
    pub dend: c_long,
    pub recs: *mut *mut xrecord_t,
    pub rchg: *mut c_char,
    pub rindex: *mut c_long,
    pub nreff: c_long,
    pub ha: *mut c_ulong,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xdfenv_t {
    pub xdf1: xdfile_t,
    pub xdf2: xdfile_t,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct diffdata_t {
    pub nrec: c_long,
    pub ha: *const c_ulong,
    pub rindex: *mut c_long,
    pub rchg: *mut c_char,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xdalgoenv_t {
    pub mxcost: c_long,
    pub snake_cnt: c_long,
    pub heur_min: c_long,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct xdchange_t {
    pub next: *mut xdchange_t,
    pub i1: c_long,
    pub i2: c_long,
    pub chg1: c_long,
    pub chg2: c_long,
    pub ignore: c_int,
}

pub type emit_func_t = unsafe fn(
    xe: *mut xdfenv_t,
    xscr: *mut xdchange_t,
    ecb: *mut xdemitcb_t,
    xecfg: *const xdemitconf_t,
) -> c_int;

#[inline]
pub fn xdl_hashlong(v: c_ulong, b: c_uint) -> c_ulong {
    v.wrapping_add(v >> b) & ((1 as c_ulong) << b).wrapping_sub(1)
}

#[inline]
pub fn xdl_isspace(c: c_char) -> bool {
    unsafe extern "C" {
        fn isspace(c: c_int) -> c_int;
    }
    unsafe { isspace(c_int::from(c as u8)) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xdiff::xutils::{xdl_bogosqrt, xdl_hashbits, xdl_num_out, xdl_recmatch};

    #[test]
    fn test_xdl_bogosqrt_and_hashbits() {
        assert_eq!(xdl_bogosqrt(0), 1);
        assert_eq!(xdl_bogosqrt(1), 2);
        assert_eq!(xdl_bogosqrt(16), 8);
        assert_eq!(xdl_hashbits(0), 1);
        assert_eq!(xdl_hashbits(1), 1);
        assert_eq!(xdl_hashbits(8), 3);
    }

    #[test]
    fn test_xdl_num_out_and_recmatch() {
        let mut buf = [0 as c_char; 32];
        let len = unsafe { xdl_num_out(buf.as_mut_ptr(), 12345) };
        assert_eq!(len, 5);
        let s = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len as usize) };
        assert_eq!(s, b"12345");

        let a = b"hello  world\n";
        let b = b"hello world\n";
        assert_eq!(
            unsafe {
                xdl_recmatch(
                    a.as_ptr().cast::<c_char>(),
                    a.len() as c_long,
                    b.as_ptr().cast::<c_char>(),
                    b.len() as c_long,
                    0,
                )
            },
            0
        );
        assert_eq!(
            unsafe {
                xdl_recmatch(
                    a.as_ptr().cast::<c_char>(),
                    a.len() as c_long,
                    b.as_ptr().cast::<c_char>(),
                    b.len() as c_long,
                    XDF_IGNORE_WHITESPACE_CHANGE as c_long,
                )
            },
            1
        );
    }
}
