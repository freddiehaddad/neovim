//! File differential library (`src/xdiff/`).
#![forbid(unsafe_code)]

pub mod xdiffi;
pub mod xemit;
pub mod xhistogram;
pub mod xpatience;
pub mod xprepare;
pub mod xutils;

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

#[derive(Copy, Clone, Debug, Default)]
pub struct Mmfile<'a> {
    pub slice: &'a [u8],
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Mmbuffer<'a> {
    pub slice: &'a [u8],
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct Xpparam {
    pub flags: u64,
}

#[derive(Copy, Clone, Default)]
pub enum Xdemitcb<'a> {
    #[default]
    None,
    Unified(crate::globals::GraphPtr<Vec<u8>>),
    Diff(crate::globals::GraphPtr<crate::diff::Diffout>),
    OnHunk(crate::globals::GraphPtr<crate::lua::xdiff::HunkPriv<'a>>),
    Locations(crate::globals::GraphPtr<crate::lua::xdiff::HunkPriv<'a>>),
}

impl<'a> Xdemitcb<'a> {
    #[inline]
    pub const fn has_hunk_func(self) -> bool {
        matches!(self, Self::Diff(_) | Self::OnHunk(_) | Self::Locations(_))
    }

    #[inline]
    pub fn out_line(self, mb: &[Mmbuffer<'_>]) -> i32 {
        match self {
            Self::Unified(out) => crate::lua::xdiff::write_string(out.borrow_mut(), mb),
            _ => 0,
        }
    }

    #[inline]
    pub fn call_hunk(self, start_a: i32, count_a: i32, start_b: i32, count_b: i32) -> i32 {
        match self {
            Self::Diff(dout) => {
                crate::diff::xdiff_out(start_a, count_a, start_b, count_b, dout.borrow_mut())
            }
            Self::OnHunk(priv_data) => crate::lua::xdiff::call_on_hunk_cb(
                start_a,
                count_a,
                start_b,
                count_b,
                priv_data.borrow_mut(),
            ),
            Self::Locations(priv_data) => crate::lua::xdiff::hunk_locations_cb(
                start_a,
                count_a,
                start_b,
                count_b,
                priv_data.borrow_mut(),
            ),
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

#[derive(Copy, Clone, Default)]
pub struct Xrecord<'a> {
    pub slice: &'a [u8],
    pub offset: usize,
    pub ha: u64,
}

#[derive(Default)]
pub struct Xdfile<'a> {
    pub file_slice: &'a [u8],
    pub nrec: i64,
    pub dstart: i64,
    pub dend: i64,
    pub recs: Vec<Xrecord<'a>>,
    pub rchg_vec: Vec<u8>,
    pub rindex: Vec<i64>,
    pub nreff: i64,
    pub ha: Vec<u64>,
}

impl<'a> Xdfile<'a> {
    pub const fn new() -> Self {
        Self {
            file_slice: &[],
            nrec: 0,
            dstart: 0,
            dend: 0,
            recs: Vec::new(),
            rchg_vec: Vec::new(),
            rindex: Vec::new(),
            nreff: 0,
            ha: Vec::new(),
        }
    }

    #[inline]
    pub fn rchg(&self, idx: i64) -> u8 {
        self.rchg_vec[(idx + 1) as usize]
    }

    #[inline]
    pub fn set_rchg(&mut self, idx: i64, val: u8) {
        self.rchg_vec[(idx + 1) as usize] = val;
    }
}

#[derive(Default)]
pub struct Xdfenv<'a> {
    pub xdf1: Xdfile<'a>,
    pub xdf2: Xdfile<'a>,
}

impl<'a> Xdfenv<'a> {
    pub const fn new() -> Self {
        Self {
            xdf1: Xdfile::new(),
            xdf2: Xdfile::new(),
        }
    }
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

pub type EmitFunc<'a> =
    fn(xe: &mut Xdfenv<'a>, xscr: &[Xdchange], ecb: &mut Xdemitcb<'_>, xecfg: &Xdemitconf) -> i32;

#[inline]
pub fn xdl_hashlong(v: u64, b: u32) -> u64 {
    v.wrapping_add(v >> b) & (1_u64 << b).wrapping_sub(1)
}

#[inline]
pub const fn xdl_isspace(c: u8) -> bool {
    matches!(c, b' ' | 0x09..=0x0d)
}
