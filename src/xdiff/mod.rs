//! File differential library (`src/xdiff/`).

pub mod xdiffi;
pub mod xemit;
pub mod xhistogram;
pub mod xpatience;
pub mod xprepare;
pub mod xutils;

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong};

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

pub const XDL_EMIT_NO_HUNK_HDR: c_ulong = 1 << 1;

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
#[derive(Copy, Clone, Default)]
pub struct xpparam_t {
    pub flags: c_ulong,
}

#[derive(Copy, Clone, Default)]
pub enum xdemitcb_t {
    #[default]
    None,
    Unified(*mut Vec<u8>),
    Diff(*mut crate::diff::diffout_T),
    OnHunk(*mut crate::lua::xdiff::HunkPriv),
    Locations(*mut crate::lua::xdiff::HunkPriv),
}

impl xdemitcb_t {
    #[inline]
    pub const fn has_hunk_func(self) -> bool {
        matches!(self, Self::Diff(_) | Self::OnHunk(_) | Self::Locations(_))
    }

    #[inline]
    pub unsafe fn out_line(self, mb: *mut mmbuffer_t, nbuf: c_int) -> c_int {
        match self {
            Self::Unified(out) => unsafe { crate::lua::xdiff::write_string(out, mb, nbuf) },
            _ => 0,
        }
    }

    #[inline]
    pub unsafe fn call_hunk(
        self,
        start_a: c_int,
        count_a: c_int,
        start_b: c_int,
        count_b: c_int,
    ) -> c_int {
        match self {
            Self::Diff(dout) => unsafe {
                crate::diff::xdiff_out(start_a, count_a, start_b, count_b, dout)
            },
            Self::OnHunk(priv_data) => unsafe {
                crate::lua::xdiff::call_on_hunk_cb(start_a, count_a, start_b, count_b, priv_data)
            },
            Self::Locations(priv_data) => unsafe {
                crate::lua::xdiff::hunk_locations_cb(start_a, count_a, start_b, count_b, priv_data)
            },
            Self::None | Self::Unified(_) => 0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct xdemitconf_t {
    pub ctxlen: c_long,
    pub interhunkctxlen: c_long,
    pub flags: c_ulong,
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
pub struct xdfile_t {
    pub rcha: Vec<std::pin::Pin<Box<xrecord_t>>>,
    pub nrec: c_long,
    pub hbits: c_uint,
    pub rhash_vec: Vec<*mut xrecord_t>,
    pub rhash: *mut *mut xrecord_t,
    pub dstart: c_long,
    pub dend: c_long,
    pub recs_vec: Vec<*mut xrecord_t>,
    pub recs: *mut *mut xrecord_t,
    pub rchg_vec: Vec<c_char>,
    pub rchg: *mut c_char,
    pub rindex_vec: Vec<c_long>,
    pub rindex: *mut c_long,
    pub nreff: c_long,
    pub ha_vec: Vec<c_ulong>,
    pub ha: *mut c_ulong,
}

impl xdfile_t {
    pub const fn new() -> Self {
        Self {
            rcha: Vec::new(),
            nrec: 0,
            hbits: 0,
            rhash_vec: Vec::new(),
            rhash: std::ptr::null_mut(),
            dstart: 0,
            dend: 0,
            recs_vec: Vec::new(),
            recs: std::ptr::null_mut(),
            rchg_vec: Vec::new(),
            rchg: std::ptr::null_mut(),
            rindex_vec: Vec::new(),
            rindex: std::ptr::null_mut(),
            nreff: 0,
            ha_vec: Vec::new(),
            ha: std::ptr::null_mut(),
        }
    }
}

impl Default for xdfile_t {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Default)]
pub struct xdfenv_t {
    pub xdf1: xdfile_t,
    pub xdf2: xdfile_t,
}

impl xdfenv_t {
    pub const fn new() -> Self {
        Self {
            xdf1: xdfile_t::new(),
            xdf2: xdfile_t::new(),
        }
    }
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
    matches!(c as u8, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xdiff::xutils::{
        xdl_blankline_slice, xdl_bogosqrt, xdl_hashbits, xdl_recmatch, xdl_recmatch_slice,
    };

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
        assert_eq!(xdl_blankline_slice(b"  \t\n", XDF_IGNORE_WHITESPACE), 1);
        assert_eq!(xdl_blankline_slice(b"  \t\n", 0), 0);

        let a = b"hello  world\n";
        let b = b"hello world\n";
        assert_eq!(xdl_recmatch_slice(a, b, 0), 0);
        assert_eq!(xdl_recmatch_slice(a, b, XDF_IGNORE_WHITESPACE_CHANGE), 1);
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
