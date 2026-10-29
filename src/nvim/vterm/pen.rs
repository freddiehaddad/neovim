//! VTerm pen and SGR color/attribute handling (`src/nvim/vterm/pen.c`).

use crate::vterm::keyboard::{VTermColor, VTermPen, VTermStateFull};
use crate::vterm::mouse::VTermPos;
use crate::vterm::parser::VTermStringFragment;
use crate::vterm::screen::{VTermGlyphInfo, VTermLineInfo, VTermScreen, VTermStateFields};

pub type VTermAttr = i32;
pub type VTermValueType = i32;
pub type VTermProp = i32;

const VTERM_COLOR_RGB: u8 = 0x00;
pub(crate) const VTERM_COLOR_INDEXED: u8 = 0x01;
const VTERM_COLOR_TYPE_MASK: u8 = 0x01;
pub(crate) const VTERM_COLOR_DEFAULT_FG: u8 = 0x02;
pub(crate) const VTERM_COLOR_DEFAULT_BG: u8 = 0x04;
const VTERM_COLOR_DEFAULT_MASK: u8 = 0x06;

pub(crate) const VTERM_VALUETYPE_BOOL: VTermValueType = 1;
pub(crate) const VTERM_VALUETYPE_INT: VTermValueType = 2;
pub(crate) const VTERM_VALUETYPE_COLOR: VTermValueType = 4;

pub(crate) const VTERM_ATTR_BOLD: VTermAttr = 1;
pub(crate) const VTERM_ATTR_UNDERLINE: VTermAttr = 2;
pub(crate) const VTERM_ATTR_ITALIC: VTermAttr = 3;
pub(crate) const VTERM_ATTR_BLINK: VTermAttr = 4;
pub(crate) const VTERM_ATTR_REVERSE: VTermAttr = 5;
pub(crate) const VTERM_ATTR_CONCEAL: VTermAttr = 6;
pub(crate) const VTERM_ATTR_STRIKE: VTermAttr = 7;
pub(crate) const VTERM_ATTR_FONT: VTermAttr = 8;
pub(crate) const VTERM_ATTR_FOREGROUND: VTermAttr = 9;
pub(crate) const VTERM_ATTR_BACKGROUND: VTermAttr = 10;
pub(crate) const VTERM_ATTR_SMALL: VTermAttr = 11;
pub(crate) const VTERM_ATTR_BASELINE: VTermAttr = 12;
pub(crate) const VTERM_ATTR_URI: VTermAttr = 13;
pub(crate) const VTERM_ATTR_DIM: VTermAttr = 14;
pub(crate) const VTERM_ATTR_OVERLINE: VTermAttr = 15;

pub(crate) const VTERM_UNDERLINE_SINGLE: u32 = 1;
pub(crate) const VTERM_UNDERLINE_DOUBLE: u32 = 2;
pub(crate) const VTERM_UNDERLINE_CURLY: u32 = 3;

const VTERM_BASELINE_NORMAL: u32 = 0;
const VTERM_BASELINE_RAISE: u32 = 1;
const VTERM_BASELINE_LOWER: u32 = 2;

pub(crate) const CSI_ARG_FLAG_MORE_U32: u32 = 1u32 << 31;
pub(crate) const CSI_ARG_MASK_U32: u32 = !(1u32 << 31);

#[derive(Clone, Copy)]
pub enum VTermValue {
    Boolean(i32),
    Number(i32),
    String(VTermStringFragment),
    Color(VTermColor),
}

impl VTermValue {
    #[inline]
    pub const fn boolean(self) -> i32 {
        match self {
            Self::Boolean(v) | Self::Number(v) => v,
            _ => 0,
        }
    }

    #[inline]
    pub const fn number(self) -> i32 {
        match self {
            Self::Number(v) | Self::Boolean(v) => v,
            _ => 0,
        }
    }

    #[inline]
    pub const fn string(self) -> VTermStringFragment {
        match self {
            Self::String(s) => s,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn color(self) -> VTermColor {
        match self {
            Self::Color(c) => c,
            _ => unreachable!(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermRect {
    pub start_row: i32,
    pub end_row: i32,
    pub start_col: i32,
    pub end_col: i32,
}

pub type VTermMoveCursorFn<T = VTermScreen> =
    fn(pos: VTermPos, oldpos: VTermPos, visible: i32, user: *mut T) -> i32;
pub type VTermScrollRectFn =
    fn(rect: VTermRect, downward: i32, rightward: i32, user: *mut VTermScreen) -> i32;
pub type VTermStateResizeFn =
    fn(rows: i32, cols: i32, fields: &mut VTermStateFields, user: *mut VTermScreen) -> i32;
pub type VTermSetLineInfoFn =
    fn(row: i32, newinfo: &VTermLineInfo, oldinfo: &VTermLineInfo, user: *mut VTermScreen) -> i32;

#[repr(C)]
pub struct VTermStateCallbacks {
    pub putglyph:
        Option<fn(info: &mut VTermGlyphInfo, pos: VTermPos, user: *mut VTermScreen) -> i32>,
    pub movecursor: Option<VTermMoveCursorFn>,
    pub scrollrect: Option<VTermScrollRectFn>,
    pub moverect: Option<fn(dest: VTermRect, src: VTermRect, user: *mut VTermScreen) -> i32>,
    pub erase: Option<fn(rect: VTermRect, selective: i32, user: *mut VTermScreen) -> i32>,
    pub initpen: Option<fn(user: *mut VTermScreen) -> i32>,
    pub setpenattr:
        Option<fn(attr: VTermAttr, val: &mut VTermValue, user: *mut VTermScreen) -> i32>,
    pub settermprop:
        Option<fn(prop: VTermProp, val: &mut VTermValue, user: *mut VTermScreen) -> i32>,
    pub bell: Option<fn(user: *mut VTermScreen) -> i32>,
    pub resize: Option<VTermStateResizeFn>,
    pub theme: Option<fn(dark: &mut bool, user: *mut VTermScreen) -> i32>,
    pub setlineinfo: Option<VTermSetLineInfoFn>,
    pub sb_clear: Option<fn(user: *mut VTermScreen) -> i32>,
}

#[derive(Clone, Copy)]
struct VTermRGB {
    red: u8,
    green: u8,
    blue: u8,
}

const ANSI_COLORS: [VTermRGB; 16] = [
    VTermRGB {
        red: 0,
        green: 0,
        blue: 0,
    },
    VTermRGB {
        red: 224,
        green: 0,
        blue: 0,
    },
    VTermRGB {
        red: 0,
        green: 224,
        blue: 0,
    },
    VTermRGB {
        red: 224,
        green: 224,
        blue: 0,
    },
    VTermRGB {
        red: 0,
        green: 0,
        blue: 224,
    },
    VTermRGB {
        red: 224,
        green: 0,
        blue: 224,
    },
    VTermRGB {
        red: 0,
        green: 224,
        blue: 224,
    },
    VTermRGB {
        red: 224,
        green: 224,
        blue: 224,
    },
    VTermRGB {
        red: 128,
        green: 128,
        blue: 128,
    },
    VTermRGB {
        red: 255,
        green: 64,
        blue: 64,
    },
    VTermRGB {
        red: 64,
        green: 255,
        blue: 64,
    },
    VTermRGB {
        red: 255,
        green: 255,
        blue: 64,
    },
    VTermRGB {
        red: 64,
        green: 64,
        blue: 255,
    },
    VTermRGB {
        red: 255,
        green: 64,
        blue: 255,
    },
    VTermRGB {
        red: 64,
        green: 255,
        blue: 255,
    },
    VTermRGB {
        red: 255,
        green: 255,
        blue: 255,
    },
];

const RAMP6: [u8; 6] = [0x00, 0x33, 0x66, 0x99, 0xCC, 0xFF];

const RAMP24: [u8; 24] = [
    0x00, 0x0B, 0x16, 0x21, 0x2C, 0x37, 0x42, 0x4D, 0x58, 0x63, 0x6E, 0x79, 0x85, 0x90, 0x9B, 0xA6,
    0xB1, 0xBC, 0xC7, 0xD2, 0xDD, 0xE8, 0xF3, 0xFF,
];

impl VTermColor {
    #[inline]
    pub const fn type_byte(&self) -> u8 {
        self.bytes[0]
    }
    #[inline]
    pub const fn set_type_byte(&mut self, t: u8) {
        self.bytes[0] = t;
    }
    #[inline]
    pub const fn red(&self) -> u8 {
        self.bytes[1]
    }
    #[inline]
    pub const fn green(&self) -> u8 {
        self.bytes[2]
    }
    #[inline]
    pub const fn blue(&self) -> u8 {
        self.bytes[3]
    }
    #[inline]
    pub const fn idx(&self) -> u8 {
        self.bytes[1]
    }
    #[inline]
    pub const fn is_indexed(&self) -> bool {
        (self.bytes[0] & VTERM_COLOR_TYPE_MASK) == VTERM_COLOR_INDEXED
    }
    #[inline]
    pub const fn is_rgb(&self) -> bool {
        (self.bytes[0] & VTERM_COLOR_TYPE_MASK) == VTERM_COLOR_RGB
    }
    #[inline]
    pub const fn is_default_fg(&self) -> bool {
        (self.bytes[0] & VTERM_COLOR_DEFAULT_FG) != 0
    }
    #[inline]
    pub const fn is_default_bg(&self) -> bool {
        (self.bytes[0] & VTERM_COLOR_DEFAULT_BG) != 0
    }
    #[inline]
    pub const fn set_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.bytes = [VTERM_COLOR_RGB, r, g, b];
    }
    #[inline]
    pub const fn set_indexed(&mut self, idx: u8) {
        self.bytes[0] = VTERM_COLOR_INDEXED;
        self.bytes[1] = idx;
    }
}

impl VTermPen {
    #[inline]
    pub const fn bold(&self) -> u32 {
        self.flags & 0x01
    }
    #[inline]
    pub const fn set_bold(&mut self, val: u32) {
        self.flags = (self.flags & !0x01) | (val & 0x01);
    }
    #[inline]
    pub const fn underline(&self) -> u32 {
        (self.flags >> 1) & 0x03
    }
    #[inline]
    pub const fn set_underline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 1)) | ((val & 0x03) << 1);
    }
    #[inline]
    pub const fn italic(&self) -> u32 {
        (self.flags >> 3) & 0x01
    }
    #[inline]
    pub const fn set_italic(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 3)) | ((val & 0x01) << 3);
    }
    #[inline]
    pub const fn blink(&self) -> u32 {
        (self.flags >> 4) & 0x01
    }
    #[inline]
    pub const fn set_blink(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 4)) | ((val & 0x01) << 4);
    }
    #[inline]
    pub const fn reverse(&self) -> u32 {
        (self.flags >> 5) & 0x01
    }
    #[inline]
    pub const fn set_reverse(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 5)) | ((val & 0x01) << 5);
    }
    #[inline]
    pub const fn conceal(&self) -> u32 {
        (self.flags >> 6) & 0x01
    }
    #[inline]
    pub const fn set_conceal(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 6)) | ((val & 0x01) << 6);
    }
    #[inline]
    pub const fn strike(&self) -> u32 {
        (self.flags >> 7) & 0x01
    }
    #[inline]
    pub const fn set_strike(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 7)) | ((val & 0x01) << 7);
    }
    #[inline]
    pub const fn font(&self) -> u32 {
        (self.flags >> 8) & 0x0f
    }
    #[inline]
    pub const fn set_font(&mut self, val: u32) {
        self.flags = (self.flags & !(0x0f << 8)) | ((val & 0x0f) << 8);
    }
    #[inline]
    pub const fn small(&self) -> u32 {
        (self.flags >> 12) & 0x01
    }
    #[inline]
    pub const fn set_small(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 12)) | ((val & 0x01) << 12);
    }
    #[inline]
    pub const fn baseline(&self) -> u32 {
        (self.flags >> 13) & 0x03
    }
    #[inline]
    pub const fn set_baseline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 13)) | ((val & 0x03) << 13);
    }
    #[inline]
    pub const fn dim(&self) -> u32 {
        (self.flags >> 15) & 0x01
    }
    #[inline]
    pub const fn set_dim(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 15)) | ((val & 0x01) << 15);
    }
    #[inline]
    pub const fn overline(&self) -> u32 {
        (self.flags >> 16) & 0x01
    }
    #[inline]
    pub const fn set_overline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 16)) | ((val & 0x01) << 16);
    }
}

use crate::vterm::state::{CSI_ARG_MISSING, csi_arg, csi_arg_has_more, csi_arg_is_missing};
use crate::vterm::vterm::vterm_get_attr_type;

fn lookup_default_colour_ansi(idx: isize, col: &mut VTermColor) {
    if (0..16).contains(&idx) {
        let c = ANSI_COLORS[idx as usize];
        col.set_rgb(c.red, c.green, c.blue);
    }
}

fn lookup_colour_ansi(state: &VTermStateFull, index: isize, col: &mut VTermColor) -> bool {
    if (0..16).contains(&index) {
        *col = state.colors[index as usize];
        return true;
    }
    false
}

fn lookup_colour_palette(state: &VTermStateFull, mut index: isize, col: &mut VTermColor) -> bool {
    if (0..16).contains(&index) {
        lookup_colour_ansi(state, index, col)
    } else if (16..232).contains(&index) {
        index -= 16;
        let idx = index as usize;
        col.set_rgb(RAMP6[(idx / 36) % 6], RAMP6[(idx / 6) % 6], RAMP6[idx % 6]);
        true
    } else if (232..256).contains(&index) {
        index -= 232;
        let v = RAMP24[index as usize];
        col.set_rgb(v, v, v);
        true
    } else {
        false
    }
}

fn lookup_colour(
    _state: &VTermStateFull,
    palette: i32,
    args: &[i64],
    col: &mut VTermColor,
) -> usize {
    let argcount = args.len();
    match palette {
        2 => {
            let mut grouplen: usize = 0;
            while grouplen < argcount && csi_arg_has_more(args[grouplen]) {
                grouplen += 1;
            }
            grouplen = if grouplen < argcount {
                grouplen + 1
            } else {
                argcount
            };
            let skip = if grouplen > 3 { 1 } else { 0 };

            if argcount.saturating_sub(skip) < 3 {
                return argcount;
            }

            col.set_rgb(
                csi_arg(args[skip]) as u8,
                csi_arg(args[skip + 1]) as u8,
                csi_arg(args[skip + 2]) as u8,
            );

            skip + 3
        }
        5 => {
            if argcount == 0 || csi_arg_is_missing(args[0]) {
                return usize::from(argcount != 0);
            }

            col.set_indexed(args[0] as u8);

            usize::from(argcount != 0)
        }
        _ => 0,
    }
}

fn setpenattr(
    state: &mut VTermStateFull,
    attr: VTermAttr,
    _type: VTermValueType,
    val: &mut VTermValue,
) {
    if let Some(callbacks) = state.callbacks
        && let Some(cb) = callbacks.setpenattr
    {
        cb(attr, val, state.screen_ptr());
    }
}

fn setpenattr_bool(state: &mut VTermStateFull, attr: VTermAttr, boolean: i32) {
    let mut val = VTermValue::Boolean(boolean);
    setpenattr(state, attr, VTERM_VALUETYPE_BOOL, &mut val);
}

fn setpenattr_int(state: &mut VTermStateFull, attr: VTermAttr, number: i32) {
    let mut val = VTermValue::Number(number);
    setpenattr(state, attr, VTERM_VALUETYPE_INT, &mut val);
}

fn setpenattr_col(state: &mut VTermStateFull, attr: VTermAttr, color: VTermColor) {
    let mut val = VTermValue::Color(color);
    setpenattr(state, attr, VTERM_VALUETYPE_COLOR, &mut val);
}

fn set_pen_col_ansi(state: &mut VTermStateFull, attr: VTermAttr, col: i32) {
    let colp = if attr == VTERM_ATTR_BACKGROUND {
        &mut state.pen.bg
    } else {
        &mut state.pen.fg
    };
    colp.set_indexed(col as u8);
    let col_val = *colp;
    setpenattr_col(state, attr, col_val);
}

pub fn vterm_state_newpen(state: &mut VTermStateFull) {
    state.default_fg.set_rgb(240, 240, 240);
    state.default_bg.set_rgb(0, 0, 0);
    let dfg = state.default_fg;
    let dbg = state.default_bg;
    vterm_state_set_default_colors(state, Some(&dfg), Some(&dbg));

    for col in 0..16 {
        lookup_default_colour_ansi(col as isize, &mut state.colors[col]);
    }
}

pub fn vterm_state_resetpen(state: &mut VTermStateFull) {
    state.pen.set_bold(0);
    setpenattr_bool(state, VTERM_ATTR_BOLD, 0);
    state.pen.set_underline(0);
    setpenattr_int(state, VTERM_ATTR_UNDERLINE, 0);
    state.pen.set_italic(0);
    setpenattr_bool(state, VTERM_ATTR_ITALIC, 0);
    state.pen.set_blink(0);
    setpenattr_bool(state, VTERM_ATTR_BLINK, 0);
    state.pen.set_reverse(0);
    setpenattr_bool(state, VTERM_ATTR_REVERSE, 0);
    state.pen.set_conceal(0);
    setpenattr_bool(state, VTERM_ATTR_CONCEAL, 0);
    state.pen.set_strike(0);
    setpenattr_bool(state, VTERM_ATTR_STRIKE, 0);
    state.pen.set_font(0);
    setpenattr_int(state, VTERM_ATTR_FONT, 0);
    state.pen.set_small(0);
    setpenattr_bool(state, VTERM_ATTR_SMALL, 0);
    state.pen.set_baseline(0);
    setpenattr_int(state, VTERM_ATTR_BASELINE, 0);
    state.pen.set_dim(0);
    setpenattr_bool(state, VTERM_ATTR_DIM, 0);
    state.pen.set_overline(0);
    setpenattr_bool(state, VTERM_ATTR_OVERLINE, 0);

    state.pen.fg = state.default_fg;
    setpenattr_col(state, VTERM_ATTR_FOREGROUND, state.default_fg);
    state.pen.bg = state.default_bg;
    setpenattr_col(state, VTERM_ATTR_BACKGROUND, state.default_bg);

    state.pen.uri = 0;
    setpenattr_int(state, VTERM_ATTR_URI, 0);
}

pub fn vterm_state_savepen(state: &mut VTermStateFull, save: i32) {
    if save != 0 {
        state.saved.pen = state.pen;
    } else {
        state.pen = state.saved.pen;

        setpenattr_bool(state, VTERM_ATTR_BOLD, state.pen.bold() as i32);
        setpenattr_int(state, VTERM_ATTR_UNDERLINE, state.pen.underline() as i32);
        setpenattr_bool(state, VTERM_ATTR_ITALIC, state.pen.italic() as i32);
        setpenattr_bool(state, VTERM_ATTR_BLINK, state.pen.blink() as i32);
        setpenattr_bool(state, VTERM_ATTR_REVERSE, state.pen.reverse() as i32);
        setpenattr_bool(state, VTERM_ATTR_CONCEAL, state.pen.conceal() as i32);
        setpenattr_bool(state, VTERM_ATTR_STRIKE, state.pen.strike() as i32);
        setpenattr_int(state, VTERM_ATTR_FONT, state.pen.font() as i32);
        setpenattr_bool(state, VTERM_ATTR_SMALL, state.pen.small() as i32);
        setpenattr_int(state, VTERM_ATTR_BASELINE, state.pen.baseline() as i32);
        setpenattr_bool(state, VTERM_ATTR_DIM, state.pen.dim() as i32);
        setpenattr_bool(state, VTERM_ATTR_OVERLINE, state.pen.overline() as i32);

        setpenattr_col(state, VTERM_ATTR_FOREGROUND, state.pen.fg);
        setpenattr_col(state, VTERM_ATTR_BACKGROUND, state.pen.bg);

        setpenattr_int(state, VTERM_ATTR_URI, state.pen.uri);
    }
}

pub fn vterm_state_set_default_colors(
    state: &mut VTermStateFull,
    default_fg: Option<&VTermColor>,
    default_bg: Option<&VTermColor>,
) {
    if let Some(default_fg) = default_fg {
        state.default_fg = *default_fg;
        let t = (state.default_fg.type_byte() & !VTERM_COLOR_DEFAULT_MASK) | VTERM_COLOR_DEFAULT_FG;
        state.default_fg.set_type_byte(t);
    }

    if let Some(default_bg) = default_bg {
        state.default_bg = *default_bg;
        let t = (state.default_bg.type_byte() & !VTERM_COLOR_DEFAULT_MASK) | VTERM_COLOR_DEFAULT_BG;
        state.default_bg.set_type_byte(t);
    }
}

pub fn vterm_state_set_palette_color(state: &mut VTermStateFull, index: i32, col: &VTermColor) {
    if (0..16).contains(&index) {
        state.colors[index as usize] = *col;
    }
}

pub fn vterm_state_convert_color_to_rgb(state: &VTermStateFull, col: &mut VTermColor) {
    if col.is_indexed() {
        lookup_colour_palette(state, col.idx() as isize, col);
    }
    let t = col.type_byte() & VTERM_COLOR_TYPE_MASK;
    col.set_type_byte(t);
}

pub fn vterm_state_setpen(state: &mut VTermStateFull, args: &[i64]) {
    let argcount = args.len();
    let mut argi: usize = 0;

    while argi < argcount {
        let arg = csi_arg(args[argi]);
        match arg {
            CSI_ARG_MISSING | 0 => {
                vterm_state_resetpen(state);
            }
            1 => {
                let fg = state.pen.fg;
                state.pen.set_bold(1);
                setpenattr_bool(state, VTERM_ATTR_BOLD, 1);
                if !fg.is_default_fg()
                    && fg.is_indexed()
                    && fg.idx() < 8
                    && state.bold_is_highbright != 0
                {
                    let idx = i32::from(fg.idx()) + if state.pen.bold() != 0 { 8 } else { 0 };
                    set_pen_col_ansi(state, VTERM_ATTR_FOREGROUND, idx);
                }
            }
            2 => {
                state.pen.set_dim(1);
                setpenattr_bool(state, VTERM_ATTR_DIM, 1);
            }
            3 => {
                state.pen.set_italic(1);
                setpenattr_bool(state, VTERM_ATTR_ITALIC, 1);
            }
            4 => {
                state.pen.set_underline(VTERM_UNDERLINE_SINGLE);
                if csi_arg_has_more(args[argi]) && argi + 1 < argcount {
                    argi += 1;
                    match csi_arg(args[argi]) {
                        0 => state.pen.set_underline(0),
                        1 => state.pen.set_underline(VTERM_UNDERLINE_SINGLE),
                        2 => state.pen.set_underline(VTERM_UNDERLINE_DOUBLE),
                        3 => state.pen.set_underline(VTERM_UNDERLINE_CURLY),
                        _ => {}
                    }
                }
                setpenattr_int(state, VTERM_ATTR_UNDERLINE, state.pen.underline() as i32);
            }
            5 => {
                state.pen.set_blink(1);
                setpenattr_bool(state, VTERM_ATTR_BLINK, 1);
            }
            7 => {
                state.pen.set_reverse(1);
                setpenattr_bool(state, VTERM_ATTR_REVERSE, 1);
            }
            8 => {
                state.pen.set_conceal(1);
                setpenattr_bool(state, VTERM_ATTR_CONCEAL, 1);
            }
            9 => {
                state.pen.set_strike(1);
                setpenattr_bool(state, VTERM_ATTR_STRIKE, 1);
            }
            10..=19 => {
                state.pen.set_font((arg - 10) as u32);
                setpenattr_int(state, VTERM_ATTR_FONT, state.pen.font() as i32);
            }
            21 => {
                state.pen.set_underline(VTERM_UNDERLINE_DOUBLE);
                setpenattr_int(state, VTERM_ATTR_UNDERLINE, state.pen.underline() as i32);
            }
            22 => {
                state.pen.set_bold(0);
                setpenattr_bool(state, VTERM_ATTR_BOLD, 0);
                state.pen.set_dim(0);
                setpenattr_bool(state, VTERM_ATTR_DIM, 0);
            }
            23 => {
                state.pen.set_italic(0);
                setpenattr_bool(state, VTERM_ATTR_ITALIC, 0);
            }
            24 => {
                state.pen.set_underline(0);
                setpenattr_int(state, VTERM_ATTR_UNDERLINE, 0);
            }
            25 => {
                state.pen.set_blink(0);
                setpenattr_bool(state, VTERM_ATTR_BLINK, 0);
            }
            27 => {
                state.pen.set_reverse(0);
                setpenattr_bool(state, VTERM_ATTR_REVERSE, 0);
            }
            28 => {
                state.pen.set_conceal(0);
                setpenattr_bool(state, VTERM_ATTR_CONCEAL, 0);
            }
            29 => {
                state.pen.set_strike(0);
                setpenattr_bool(state, VTERM_ATTR_STRIKE, 0);
            }
            30..=37 => {
                let mut value = arg - 30;
                if state.pen.bold() != 0 && state.bold_is_highbright != 0 {
                    value += 8;
                }
                set_pen_col_ansi(state, VTERM_ATTR_FOREGROUND, value);
            }
            38 => {
                if argi + 1 >= argcount {
                    return;
                }
                let mut fg = state.pen.fg;
                argi += 1 + lookup_colour(
                    state,
                    csi_arg(args[argi + 1]),
                    &args[(argi + 2).min(argcount)..],
                    &mut fg,
                );
                state.pen.fg = fg;
                setpenattr_col(state, VTERM_ATTR_FOREGROUND, fg);
            }
            39 => {
                state.pen.fg = state.default_fg;
                setpenattr_col(state, VTERM_ATTR_FOREGROUND, state.pen.fg);
            }
            40..=47 => {
                let value = arg - 40;
                set_pen_col_ansi(state, VTERM_ATTR_BACKGROUND, value);
            }
            48 => {
                if argi + 1 >= argcount {
                    return;
                }
                let mut bg = state.pen.bg;
                argi += 1 + lookup_colour(
                    state,
                    csi_arg(args[argi + 1]),
                    &args[(argi + 2).min(argcount)..],
                    &mut bg,
                );
                state.pen.bg = bg;
                setpenattr_col(state, VTERM_ATTR_BACKGROUND, bg);
            }
            49 => {
                state.pen.bg = state.default_bg;
                setpenattr_col(state, VTERM_ATTR_BACKGROUND, state.pen.bg);
            }
            53 => {
                state.pen.set_overline(1);
                setpenattr_bool(state, VTERM_ATTR_OVERLINE, 1);
            }
            55 => {
                state.pen.set_overline(0);
                setpenattr_bool(state, VTERM_ATTR_OVERLINE, 0);
            }
            73..=75 => {
                state.pen.set_small(u32::from(arg != 75));
                state.pen.set_baseline(if arg == 73 {
                    VTERM_BASELINE_RAISE
                } else if arg == 74 {
                    VTERM_BASELINE_LOWER
                } else {
                    VTERM_BASELINE_NORMAL
                });
                setpenattr_bool(state, VTERM_ATTR_SMALL, state.pen.small() as i32);
                setpenattr_int(state, VTERM_ATTR_BASELINE, state.pen.baseline() as i32);
            }
            90..=97 => {
                let value = arg - 90 + 8;
                set_pen_col_ansi(state, VTERM_ATTR_FOREGROUND, value);
            }
            100..=107 => {
                let value = arg - 100 + 8;
                set_pen_col_ansi(state, VTERM_ATTR_BACKGROUND, value);
            }
            _ => {}
        }

        while argi < argcount {
            let cur = args[argi];
            argi += 1;
            if !csi_arg_has_more(cur) {
                break;
            }
        }
    }
}

fn vterm_state_getpen_color(col: &VTermColor, mut argi: i32, args: &mut [i64], fg: bool) -> i32 {
    if (fg && col.is_default_fg()) || (!fg && col.is_default_bg()) {
        return argi;
    }

    let more = CSI_ARG_FLAG_MORE_U32 as i64;
    if col.is_indexed() {
        let idx = col.idx();
        if idx < 8 {
            args[argi as usize] = i64::from(idx) + if fg { 30 } else { 40 };
            argi += 1;
        } else if idx < 16 {
            args[argi as usize] = i64::from(idx - 8) + if fg { 90 } else { 100 };
            argi += 1;
        } else {
            args[argi as usize] = more | if fg { 38 } else { 48 };
            argi += 1;
            args[argi as usize] = more | 5;
            argi += 1;
            args[argi as usize] = i64::from(idx);
            argi += 1;
        }
    } else if col.is_rgb() {
        args[argi as usize] = more | if fg { 38 } else { 48 };
        argi += 1;
        args[argi as usize] = more | 2;
        argi += 1;
        args[argi as usize] = more | i64::from(col.red());
        argi += 1;
        args[argi as usize] = more | i64::from(col.green());
        argi += 1;
        args[argi as usize] = i64::from(col.blue());
        argi += 1;
    }
    argi
}

pub fn vterm_state_getpen(state: &mut VTermStateFull, args: &mut [i64]) -> i32 {
    let mut argi: i32 = 0;
    let more = CSI_ARG_FLAG_MORE_U32 as i64;

    if state.pen.bold() != 0 {
        args[argi as usize] = 1;
        argi += 1;
    }
    if state.pen.dim() != 0 {
        args[argi as usize] = 2;
        argi += 1;
    }
    if state.pen.italic() != 0 {
        args[argi as usize] = 3;
        argi += 1;
    }
    if state.pen.underline() == VTERM_UNDERLINE_SINGLE {
        args[argi as usize] = 4;
        argi += 1;
    }
    if state.pen.underline() == VTERM_UNDERLINE_CURLY {
        args[argi as usize] = 4 | more;
        argi += 1;
        args[argi as usize] = 3;
        argi += 1;
    }
    if state.pen.blink() != 0 {
        args[argi as usize] = 5;
        argi += 1;
    }
    if state.pen.reverse() != 0 {
        args[argi as usize] = 7;
        argi += 1;
    }
    if state.pen.conceal() != 0 {
        args[argi as usize] = 8;
        argi += 1;
    }
    if state.pen.strike() != 0 {
        args[argi as usize] = 9;
        argi += 1;
    }
    if state.pen.font() != 0 {
        args[argi as usize] = (10 + state.pen.font()) as i64;
        argi += 1;
    }
    if state.pen.underline() == VTERM_UNDERLINE_DOUBLE {
        args[argi as usize] = 21;
        argi += 1;
    }

    argi = vterm_state_getpen_color(&state.pen.fg, argi, args, true);
    argi = vterm_state_getpen_color(&state.pen.bg, argi, args, false);

    if state.pen.overline() != 0 {
        args[argi as usize] = 53;
        argi += 1;
    }

    if state.pen.small() != 0 {
        if state.pen.baseline() == VTERM_BASELINE_RAISE {
            args[argi as usize] = 73;
            argi += 1;
        } else if state.pen.baseline() == VTERM_BASELINE_LOWER {
            args[argi as usize] = 74;
            argi += 1;
        }
    }

    argi
}

pub fn vterm_state_set_penattr(
    state: &mut VTermStateFull,
    attr: VTermAttr,
    r#type: VTermValueType,
    val: &mut VTermValue,
) -> i32 {
    if r#type != vterm_get_attr_type(attr) {
        return 0;
    }

    match attr {
        VTERM_ATTR_BOLD => state.pen.set_bold(val.boolean() as u32),
        VTERM_ATTR_UNDERLINE => state.pen.set_underline(val.number() as u32),
        VTERM_ATTR_ITALIC => state.pen.set_italic(val.boolean() as u32),
        VTERM_ATTR_BLINK => state.pen.set_blink(val.boolean() as u32),
        VTERM_ATTR_REVERSE => state.pen.set_reverse(val.boolean() as u32),
        VTERM_ATTR_CONCEAL => state.pen.set_conceal(val.boolean() as u32),
        VTERM_ATTR_STRIKE => state.pen.set_strike(val.boolean() as u32),
        VTERM_ATTR_FONT => state.pen.set_font(val.number() as u32),
        VTERM_ATTR_FOREGROUND => state.pen.fg = val.color(),
        VTERM_ATTR_BACKGROUND => state.pen.bg = val.color(),
        VTERM_ATTR_SMALL => state.pen.set_small(val.boolean() as u32),
        VTERM_ATTR_BASELINE => state.pen.set_baseline(val.number() as u32),
        VTERM_ATTR_URI => state.pen.uri = val.number(),
        VTERM_ATTR_DIM => state.pen.set_dim(val.boolean() as u32),
        VTERM_ATTR_OVERLINE => state.pen.set_overline(val.boolean() as u32),
        _ => return 0,
    }

    if let Some(callbacks) = state.callbacks
        && let Some(cb) = callbacks.setpenattr
    {
        cb(attr, val, state.screen_ptr());
    }

    1
}
