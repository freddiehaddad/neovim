//! File differential library (`src/xdiff/`).

pub mod xdiffi;
pub mod xemit;
pub mod xhistogram;
pub mod xpatience;
pub mod xprepare;
pub mod xutils;

use std::ffi::c_char;

pub const XDF_NEED_MINIMAL: u64 = 1 << 0;

pub const XDF_IGNORE_WHITESPACE: u64 = 1 << 1;
pub const XDF_IGNORE_WHITESPACE_CHANGE: u64 = 1 << 2;
pub const XDF_IGNORE_WHITESPACE_AT_EOL: u64 = 1 << 3;
pub const XDF_IGNORE_CR_AT_EOL: u64 = 1 << 4;
pub const XDF_WHITESPACE_FLAGS: u64 = XDF_IGNORE_WHITESPACE
    | XDF_IGNORE_WHITESPACE_CHANGE
    | XDF_IGNORE_WHITESPACE_AT_EOL
    | XDF_IGNORE_CR_AT_EOL;

pub const XDF_IGNORE_BLANK_LINES: u64 = 1 << 7;

pub const XDF_PATIENCE_DIFF: u64 = 1 << 14;
pub const XDF_HISTOGRAM_DIFF: u64 = 1 << 15;
pub const XDF_DIFF_ALGORITHM_MASK: u64 = XDF_PATIENCE_DIFF | XDF_HISTOGRAM_DIFF;

#[inline]
pub const fn xdf_diff_alg(flags: u64) -> u64 {
    flags & XDF_DIFF_ALGORITHM_MASK
}

pub const XDF_INDENT_HEURISTIC: u64 = 1 << 23;

pub const XDL_EMIT_NO_HUNK_HDR: u64 = 1 << 1;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Mmfile {
    pub ptr: *mut c_char,
    pub size: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Mmbuffer {
    pub ptr: *mut c_char,
    pub size: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct Xpparam {
    pub flags: u64,
}

#[derive(Copy, Clone, Default)]
pub enum Xdemitcb {
    #[default]
    None,
    Unified(*mut Vec<u8>),
    Diff(*mut crate::diff::Diffout),
    OnHunk(*mut crate::lua::xdiff::HunkPriv),
    Locations(*mut crate::lua::xdiff::HunkPriv),
}

impl Xdemitcb {
    #[inline]
    pub const fn has_hunk_func(self) -> bool {
        matches!(self, Self::Diff(_) | Self::OnHunk(_) | Self::Locations(_))
    }

    #[inline]
    pub fn out_line(self, mb: &[Mmbuffer]) -> i32 {
        match self {
            Self::Unified(out) => crate::lua::xdiff::write_string(unsafe { &mut *out }, mb),
            _ => 0,
        }
    }

    #[inline]
    pub fn call_hunk(self, start_a: i32, count_a: i32, start_b: i32, count_b: i32) -> i32 {
        match self {
            Self::Diff(dout) => unsafe {
                crate::diff::xdiff_out(start_a, count_a, start_b, count_b, &mut *dout)
            },
            Self::OnHunk(priv_data) => {
                crate::lua::xdiff::call_on_hunk_cb(start_a, count_a, start_b, count_b, unsafe {
                    &mut *priv_data
                })
            }
            Self::Locations(priv_data) => {
                crate::lua::xdiff::hunk_locations_cb(start_a, count_a, start_b, count_b, unsafe {
                    &mut *priv_data
                })
            }
            Self::None | Self::Unified(_) => 0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct Xdemitconf {
    pub ctxlen: i64,
    pub interhunkctxlen: i64,
    pub flags: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Xrecord {
    pub ptr: *const c_char,
    pub size: i64,
    pub ha: u64,
}

#[repr(C)]
pub struct Xdfile {
    pub nrec: i64,
    pub dstart: i64,
    pub dend: i64,
    pub recs: Vec<Xrecord>,
    pub rchg_vec: Vec<c_char>,
    pub rchg: *mut c_char,
    pub rindex: Vec<i64>,
    pub nreff: i64,
    pub ha: Vec<u64>,
}

impl Xdfile {
    pub const fn new() -> Self {
        Self {
            nrec: 0,
            dstart: 0,
            dend: 0,
            recs: Vec::new(),
            rchg_vec: Vec::new(),
            rchg: std::ptr::null_mut(),
            rindex: Vec::new(),
            nreff: 0,
            ha: Vec::new(),
        }
    }
}

impl Default for Xdfile {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Default)]
pub struct Xdfenv {
    pub xdf1: Xdfile,
    pub xdf2: Xdfile,
}

impl Xdfenv {
    pub const fn new() -> Self {
        Self {
            xdf1: Xdfile::new(),
            xdf2: Xdfile::new(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Diffdata {
    pub nrec: i64,
    pub ha: *const u64,
    pub rindex: *mut i64,
    pub rchg: *mut c_char,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Xdalgoenv {
    pub mxcost: i64,
    pub snake_cnt: i64,
    pub heur_min: i64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Xdchange {
    pub i1: i64,
    pub i2: i64,
    pub chg1: i64,
    pub chg2: i64,
    pub ignore: i32,
}

pub type EmitFunc =
    fn(xe: &mut Xdfenv, xscr: &[Xdchange], ecb: &mut Xdemitcb, xecfg: &Xdemitconf) -> i32;

#[inline]
pub fn xdl_hashlong(v: u64, b: u32) -> u64 {
    v.wrapping_add(v >> b) & (1_u64 << b).wrapping_sub(1)
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
            xdl_recmatch(
                a.as_ptr().cast::<c_char>(),
                a.len() as i64,
                b.as_ptr().cast::<c_char>(),
                b.len() as i64,
                XDF_IGNORE_WHITESPACE_CHANGE as i64,
            ),
            1
        );
    }
}
