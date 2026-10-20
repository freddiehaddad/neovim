//! VTerm pen and SGR color/attribute handling (`src/nvim/vterm/pen.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use std::ffi::{c_int, c_long, c_void};

use crate::vterm::keyboard::{VTermColor, VTermPen, VTermStateFull};
use crate::vterm::mouse::VTermPos;
use crate::vterm::parser::VTermStringFragment;

pub type VTermAttr = c_int;
pub type VTermValueType = c_int;
pub type VTermProp = c_int;

const VTERM_COLOR_RGB: u8 = 0x00;
const VTERM_COLOR_INDEXED: u8 = 0x01;
const VTERM_COLOR_TYPE_MASK: u8 = 0x01;
const VTERM_COLOR_DEFAULT_FG: u8 = 0x02;
const VTERM_COLOR_DEFAULT_BG: u8 = 0x04;
const VTERM_COLOR_DEFAULT_MASK: u8 = 0x06;

const VTERM_VALUETYPE_BOOL: VTermValueType = 1;
const VTERM_VALUETYPE_INT: VTermValueType = 2;
const VTERM_VALUETYPE_STRING: VTermValueType = 3;
const VTERM_VALUETYPE_COLOR: VTermValueType = 4;

const VTERM_ATTR_BOLD: VTermAttr = 1;
const VTERM_ATTR_UNDERLINE: VTermAttr = 2;
const VTERM_ATTR_ITALIC: VTermAttr = 3;
const VTERM_ATTR_BLINK: VTermAttr = 4;
const VTERM_ATTR_REVERSE: VTermAttr = 5;
const VTERM_ATTR_CONCEAL: VTermAttr = 6;
const VTERM_ATTR_STRIKE: VTermAttr = 7;
const VTERM_ATTR_FONT: VTermAttr = 8;
const VTERM_ATTR_FOREGROUND: VTermAttr = 9;
const VTERM_ATTR_BACKGROUND: VTermAttr = 10;
const VTERM_ATTR_SMALL: VTermAttr = 11;
const VTERM_ATTR_BASELINE: VTermAttr = 12;
const VTERM_ATTR_URI: VTermAttr = 13;
const VTERM_ATTR_DIM: VTermAttr = 14;
const VTERM_ATTR_OVERLINE: VTermAttr = 15;

const VTERM_UNDERLINE_OFF: u32 = 0;
const VTERM_UNDERLINE_SINGLE: u32 = 1;
const VTERM_UNDERLINE_DOUBLE: u32 = 2;
const VTERM_UNDERLINE_CURLY: u32 = 3;

const VTERM_BASELINE_NORMAL: u32 = 0;
const VTERM_BASELINE_RAISE: u32 = 1;
const VTERM_BASELINE_LOWER: u32 = 2;

const CSI_ARG_FLAG_MORE_U32: u32 = 1u32 << 31;
const CSI_ARG_MASK_U32: u32 = !(1u32 << 31);
const CSI_ARG_MISSING_I64: i64 = (1i64 << 31) - 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub union VTermValue {
    pub boolean: c_int,
    pub number: c_int,
    pub string: VTermStringFragment,
    pub color: VTermColor,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermRect {
    pub start_row: c_int,
    pub end_row: c_int,
    pub start_col: c_int,
    pub end_col: c_int,
}

#[repr(C)]
pub struct VTermStateCallbacks {
    pub putglyph:
        Option<unsafe extern "C" fn(info: *mut c_void, pos: VTermPos, user: *mut c_void) -> c_int>,
    pub movecursor: Option<
        unsafe extern "C" fn(
            pos: VTermPos,
            oldpos: VTermPos,
            visible: c_int,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub scrollrect: Option<
        unsafe extern "C" fn(
            rect: VTermRect,
            downward: c_int,
            rightward: c_int,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub moverect:
        Option<unsafe extern "C" fn(dest: VTermRect, src: VTermRect, user: *mut c_void) -> c_int>,
    pub erase:
        Option<unsafe extern "C" fn(rect: VTermRect, selective: c_int, user: *mut c_void) -> c_int>,
    pub initpen: Option<unsafe extern "C" fn(user: *mut c_void) -> c_int>,
    pub setpenattr: Option<
        unsafe extern "C" fn(attr: VTermAttr, val: *mut VTermValue, user: *mut c_void) -> c_int,
    >,
    pub settermprop: Option<
        unsafe extern "C" fn(prop: VTermProp, val: *mut VTermValue, user: *mut c_void) -> c_int,
    >,
    pub bell: Option<unsafe extern "C" fn(user: *mut c_void) -> c_int>,
    pub resize: Option<
        unsafe extern "C" fn(
            rows: c_int,
            cols: c_int,
            fields: *mut c_void,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub theme: Option<unsafe extern "C" fn(dark: *mut bool, user: *mut c_void) -> c_int>,
    pub setlineinfo: Option<
        unsafe extern "C" fn(
            row: c_int,
            newinfo: *const c_void,
            oldinfo: *const c_void,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub sb_clear: Option<unsafe extern "C" fn(user: *mut c_void) -> c_int>,
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

#[inline]
fn csi_arg_has_more(a: c_long) -> bool {
    (a as u32 & CSI_ARG_FLAG_MORE_U32) != 0
}

#[inline]
fn csi_arg(a: c_long) -> i64 {
    i64::from((a as u32) & CSI_ARG_MASK_U32)
}

#[inline]
fn csi_arg_is_missing(a: c_long) -> bool {
    csi_arg(a) == CSI_ARG_MISSING_I64
}

use crate::vterm::vterm::vterm_get_attr_type;

fn lookup_default_colour_ansi(idx: isize, col: &mut VTermColor) {
    if (0..16).contains(&idx) {
        let c = ANSI_COLORS[idx as usize];
        col.set_rgb(c.red, c.green, c.blue);
    }
}

unsafe fn lookup_colour_ansi(
    state: *const VTermStateFull,
    index: isize,
    col: *mut VTermColor,
) -> bool {
    if (0..16).contains(&index) {
        unsafe {
            *col = (*state).colors[index as usize];
        }
        return true;
    }
    false
}

unsafe fn lookup_colour_palette(
    state: *const VTermStateFull,
    mut index: isize,
    col: *mut VTermColor,
) -> bool {
    if (0..16).contains(&index) {
        unsafe { lookup_colour_ansi(state, index, col) }
    } else if (16..232).contains(&index) {
        index -= 16;
        let idx = index as usize;
        unsafe {
            (*col).set_rgb(RAMP6[(idx / 36) % 6], RAMP6[(idx / 6) % 6], RAMP6[idx % 6]);
        }
        true
    } else if (232..256).contains(&index) {
        index -= 232;
        let v = RAMP24[index as usize];
        unsafe {
            (*col).set_rgb(v, v, v);
        }
        true
    } else {
        false
    }
}

unsafe fn lookup_colour(
    _state: *const VTermStateFull,
    palette: c_int,
    args: *const c_long,
    argcount: c_int,
    col: *mut VTermColor,
) -> c_int {
    unsafe {
        match palette {
            2 => {
                let mut grouplen: c_int = 0;
                while grouplen < argcount && csi_arg_has_more(*args.add(grouplen as usize)) {
                    grouplen += 1;
                }
                grouplen = if grouplen < argcount {
                    grouplen + 1
                } else {
                    argcount
                };
                let skip = if grouplen > 3 { 1 } else { 0 };

                if argcount - skip < 3 {
                    return argcount;
                }

                (*col).set_rgb(
                    csi_arg(*args.add(skip as usize)) as u8,
                    csi_arg(*args.add((skip + 1) as usize)) as u8,
                    csi_arg(*args.add((skip + 2) as usize)) as u8,
                );

                skip + 3
            }
            5 => {
                if argcount == 0 || csi_arg_is_missing(*args) {
                    return c_int::from(argcount != 0);
                }

                (*col).set_indexed((*args) as u8);

                c_int::from(argcount != 0)
            }
            _ => 0,
        }
    }
}

unsafe fn setpenattr(
    state: *mut VTermStateFull,
    attr: VTermAttr,
    _type: VTermValueType,
    val: *mut VTermValue,
) {
    unsafe {
        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).setpenattr
        {
            cb(attr, val, (*state).cbdata);
        }
    }
}

unsafe fn setpenattr_bool(state: *mut VTermStateFull, attr: VTermAttr, boolean: c_int) {
    let mut val = VTermValue { boolean };
    unsafe {
        setpenattr(state, attr, VTERM_VALUETYPE_BOOL, &raw mut val);
    }
}

unsafe fn setpenattr_int(state: *mut VTermStateFull, attr: VTermAttr, number: c_int) {
    let mut val = VTermValue { number };
    unsafe {
        setpenattr(state, attr, VTERM_VALUETYPE_INT, &raw mut val);
    }
}

unsafe fn setpenattr_col(state: *mut VTermStateFull, attr: VTermAttr, color: VTermColor) {
    let mut val = VTermValue { color };
    unsafe {
        setpenattr(state, attr, VTERM_VALUETYPE_COLOR, &raw mut val);
    }
}

unsafe fn set_pen_col_ansi(state: *mut VTermStateFull, attr: VTermAttr, col: i64) {
    unsafe {
        let colp = if attr == VTERM_ATTR_BACKGROUND {
            &raw mut (*state).pen.bg
        } else {
            &raw mut (*state).pen.fg
        };
        (*colp).set_indexed(col as u8);
        setpenattr_col(state, attr, *colp);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_newpen(state: *mut VTermStateFull) {
    unsafe {
        (*state).default_fg.set_rgb(240, 240, 240);
        (*state).default_bg.set_rgb(0, 0, 0);
        let dfg = (*state).default_fg;
        let dbg = (*state).default_bg;
        vterm_state_set_default_colors(state, &raw const dfg, &raw const dbg);

        for col in 0..16 {
            lookup_default_colour_ansi(col as isize, &mut (*state).colors[col]);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_resetpen(state: *mut VTermStateFull) {
    unsafe {
        (*state).pen.set_bold(0);
        setpenattr_bool(state, VTERM_ATTR_BOLD, 0);
        (*state).pen.set_underline(0);
        setpenattr_int(state, VTERM_ATTR_UNDERLINE, 0);
        (*state).pen.set_italic(0);
        setpenattr_bool(state, VTERM_ATTR_ITALIC, 0);
        (*state).pen.set_blink(0);
        setpenattr_bool(state, VTERM_ATTR_BLINK, 0);
        (*state).pen.set_reverse(0);
        setpenattr_bool(state, VTERM_ATTR_REVERSE, 0);
        (*state).pen.set_conceal(0);
        setpenattr_bool(state, VTERM_ATTR_CONCEAL, 0);
        (*state).pen.set_strike(0);
        setpenattr_bool(state, VTERM_ATTR_STRIKE, 0);
        (*state).pen.set_font(0);
        setpenattr_int(state, VTERM_ATTR_FONT, 0);
        (*state).pen.set_small(0);
        setpenattr_bool(state, VTERM_ATTR_SMALL, 0);
        (*state).pen.set_baseline(0);
        setpenattr_int(state, VTERM_ATTR_BASELINE, 0);
        (*state).pen.set_dim(0);
        setpenattr_bool(state, VTERM_ATTR_DIM, 0);
        (*state).pen.set_overline(0);
        setpenattr_bool(state, VTERM_ATTR_OVERLINE, 0);

        (*state).pen.fg = (*state).default_fg;
        setpenattr_col(state, VTERM_ATTR_FOREGROUND, (*state).default_fg);
        (*state).pen.bg = (*state).default_bg;
        setpenattr_col(state, VTERM_ATTR_BACKGROUND, (*state).default_bg);

        (*state).pen.uri = 0;
        setpenattr_int(state, VTERM_ATTR_URI, 0);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_savepen(state: *mut VTermStateFull, save: c_int) {
    unsafe {
        if save != 0 {
            (*state).saved.pen = (*state).pen;
        } else {
            (*state).pen = (*state).saved.pen;

            setpenattr_bool(state, VTERM_ATTR_BOLD, (*state).pen.bold() as c_int);
            setpenattr_int(
                state,
                VTERM_ATTR_UNDERLINE,
                (*state).pen.underline() as c_int,
            );
            setpenattr_bool(state, VTERM_ATTR_ITALIC, (*state).pen.italic() as c_int);
            setpenattr_bool(state, VTERM_ATTR_BLINK, (*state).pen.blink() as c_int);
            setpenattr_bool(state, VTERM_ATTR_REVERSE, (*state).pen.reverse() as c_int);
            setpenattr_bool(state, VTERM_ATTR_CONCEAL, (*state).pen.conceal() as c_int);
            setpenattr_bool(state, VTERM_ATTR_STRIKE, (*state).pen.strike() as c_int);
            setpenattr_int(state, VTERM_ATTR_FONT, (*state).pen.font() as c_int);
            setpenattr_bool(state, VTERM_ATTR_SMALL, (*state).pen.small() as c_int);
            setpenattr_int(state, VTERM_ATTR_BASELINE, (*state).pen.baseline() as c_int);
            setpenattr_bool(state, VTERM_ATTR_DIM, (*state).pen.dim() as c_int);
            setpenattr_bool(state, VTERM_ATTR_OVERLINE, (*state).pen.overline() as c_int);

            setpenattr_col(state, VTERM_ATTR_FOREGROUND, (*state).pen.fg);
            setpenattr_col(state, VTERM_ATTR_BACKGROUND, (*state).pen.bg);

            setpenattr_int(state, VTERM_ATTR_URI, (*state).pen.uri);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_set_default_colors(
    state: *mut VTermStateFull,
    default_fg: *const VTermColor,
    default_bg: *const VTermColor,
) {
    unsafe {
        if !default_fg.is_null() {
            (*state).default_fg = *default_fg;
            let t = ((*state).default_fg.type_byte() & !VTERM_COLOR_DEFAULT_MASK)
                | VTERM_COLOR_DEFAULT_FG;
            (*state).default_fg.set_type_byte(t);
        }

        if !default_bg.is_null() {
            (*state).default_bg = *default_bg;
            let t = ((*state).default_bg.type_byte() & !VTERM_COLOR_DEFAULT_MASK)
                | VTERM_COLOR_DEFAULT_BG;
            (*state).default_bg.set_type_byte(t);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_set_palette_color(
    state: *mut VTermStateFull,
    index: c_int,
    col: *const VTermColor,
) {
    if (0..16).contains(&index) {
        unsafe {
            (*state).colors[index as usize] = *col;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_convert_color_to_rgb(
    state: *const VTermStateFull,
    col: *mut VTermColor,
) {
    unsafe {
        if (*col).is_indexed() {
            lookup_colour_palette(state, (*col).idx() as isize, col);
        }
        let t = (*col).type_byte() & VTERM_COLOR_TYPE_MASK;
        (*col).set_type_byte(t);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_setpen(
    state: *mut VTermStateFull,
    args: *const c_long,
    argcount: c_int,
) {
    unsafe {
        let mut argi: c_int = 0;

        while argi < argcount {
            let arg = csi_arg(*args.add(argi as usize));
            match arg {
                CSI_ARG_MISSING_I64 | 0 => {
                    vterm_state_resetpen(state);
                }
                1 => {
                    let fg = (*state).pen.fg;
                    (*state).pen.set_bold(1);
                    setpenattr_bool(state, VTERM_ATTR_BOLD, 1);
                    if !fg.is_default_fg()
                        && fg.is_indexed()
                        && fg.idx() < 8
                        && (*state).bold_is_highbright != 0
                    {
                        let idx =
                            i64::from(fg.idx()) + if (*state).pen.bold() != 0 { 8 } else { 0 };
                        set_pen_col_ansi(state, VTERM_ATTR_FOREGROUND, idx);
                    }
                }
                2 => {
                    (*state).pen.set_dim(1);
                    setpenattr_bool(state, VTERM_ATTR_DIM, 1);
                }
                3 => {
                    (*state).pen.set_italic(1);
                    setpenattr_bool(state, VTERM_ATTR_ITALIC, 1);
                }
                4 => {
                    (*state).pen.set_underline(VTERM_UNDERLINE_SINGLE);
                    if csi_arg_has_more(*args.add(argi as usize)) {
                        argi += 1;
                        match csi_arg(*args.add(argi as usize)) {
                            0 => (*state).pen.set_underline(0),
                            1 => (*state).pen.set_underline(VTERM_UNDERLINE_SINGLE),
                            2 => (*state).pen.set_underline(VTERM_UNDERLINE_DOUBLE),
                            3 => (*state).pen.set_underline(VTERM_UNDERLINE_CURLY),
                            _ => {}
                        }
                    }
                    setpenattr_int(
                        state,
                        VTERM_ATTR_UNDERLINE,
                        (*state).pen.underline() as c_int,
                    );
                }
                5 => {
                    (*state).pen.set_blink(1);
                    setpenattr_bool(state, VTERM_ATTR_BLINK, 1);
                }
                7 => {
                    (*state).pen.set_reverse(1);
                    setpenattr_bool(state, VTERM_ATTR_REVERSE, 1);
                }
                8 => {
                    (*state).pen.set_conceal(1);
                    setpenattr_bool(state, VTERM_ATTR_CONCEAL, 1);
                }
                9 => {
                    (*state).pen.set_strike(1);
                    setpenattr_bool(state, VTERM_ATTR_STRIKE, 1);
                }
                10..=19 => {
                    (*state).pen.set_font((arg - 10) as u32);
                    setpenattr_int(state, VTERM_ATTR_FONT, (*state).pen.font() as c_int);
                }
                21 => {
                    (*state).pen.set_underline(VTERM_UNDERLINE_DOUBLE);
                    setpenattr_int(
                        state,
                        VTERM_ATTR_UNDERLINE,
                        (*state).pen.underline() as c_int,
                    );
                }
                22 => {
                    (*state).pen.set_bold(0);
                    setpenattr_bool(state, VTERM_ATTR_BOLD, 0);
                    (*state).pen.set_dim(0);
                    setpenattr_bool(state, VTERM_ATTR_DIM, 0);
                }
                23 => {
                    (*state).pen.set_italic(0);
                    setpenattr_bool(state, VTERM_ATTR_ITALIC, 0);
                }
                24 => {
                    (*state).pen.set_underline(0);
                    setpenattr_int(state, VTERM_ATTR_UNDERLINE, 0);
                }
                25 => {
                    (*state).pen.set_blink(0);
                    setpenattr_bool(state, VTERM_ATTR_BLINK, 0);
                }
                27 => {
                    (*state).pen.set_reverse(0);
                    setpenattr_bool(state, VTERM_ATTR_REVERSE, 0);
                }
                28 => {
                    (*state).pen.set_conceal(0);
                    setpenattr_bool(state, VTERM_ATTR_CONCEAL, 0);
                }
                29 => {
                    (*state).pen.set_strike(0);
                    setpenattr_bool(state, VTERM_ATTR_STRIKE, 0);
                }
                30..=37 => {
                    let mut value = arg - 30;
                    if (*state).pen.bold() != 0 && (*state).bold_is_highbright != 0 {
                        value += 8;
                    }
                    set_pen_col_ansi(state, VTERM_ATTR_FOREGROUND, value);
                }
                38 => {
                    if argcount - argi < 1 {
                        return;
                    }
                    argi += 1 + lookup_colour(
                        state,
                        csi_arg(*args.add((argi + 1) as usize)) as c_int,
                        args.add((argi + 2) as usize),
                        argcount - argi - 2,
                        &raw mut (*state).pen.fg,
                    );
                    setpenattr_col(state, VTERM_ATTR_FOREGROUND, (*state).pen.fg);
                }
                39 => {
                    (*state).pen.fg = (*state).default_fg;
                    setpenattr_col(state, VTERM_ATTR_FOREGROUND, (*state).pen.fg);
                }
                40..=47 => {
                    let value = arg - 40;
                    set_pen_col_ansi(state, VTERM_ATTR_BACKGROUND, value);
                }
                48 => {
                    if argcount - argi < 1 {
                        return;
                    }
                    argi += 1 + lookup_colour(
                        state,
                        csi_arg(*args.add((argi + 1) as usize)) as c_int,
                        args.add((argi + 2) as usize),
                        argcount - argi - 2,
                        &raw mut (*state).pen.bg,
                    );
                    setpenattr_col(state, VTERM_ATTR_BACKGROUND, (*state).pen.bg);
                }
                49 => {
                    (*state).pen.bg = (*state).default_bg;
                    setpenattr_col(state, VTERM_ATTR_BACKGROUND, (*state).pen.bg);
                }
                53 => {
                    (*state).pen.set_overline(1);
                    setpenattr_bool(state, VTERM_ATTR_OVERLINE, 1);
                }
                55 => {
                    (*state).pen.set_overline(0);
                    setpenattr_bool(state, VTERM_ATTR_OVERLINE, 0);
                }
                73..=75 => {
                    (*state).pen.set_small(u32::from(arg != 75));
                    (*state).pen.set_baseline(if arg == 73 {
                        VTERM_BASELINE_RAISE
                    } else if arg == 74 {
                        VTERM_BASELINE_LOWER
                    } else {
                        VTERM_BASELINE_NORMAL
                    });
                    setpenattr_bool(state, VTERM_ATTR_SMALL, (*state).pen.small() as c_int);
                    setpenattr_int(state, VTERM_ATTR_BASELINE, (*state).pen.baseline() as c_int);
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

            loop {
                let cur = *args.add(argi as usize);
                argi += 1;
                if !csi_arg_has_more(cur) {
                    break;
                }
            }
        }
    }
}

unsafe fn vterm_state_getpen_color(
    col: &VTermColor,
    mut argi: c_int,
    args: *mut c_long,
    fg: bool,
) -> c_int {
    if (fg && col.is_default_fg()) || (!fg && col.is_default_bg()) {
        return argi;
    }

    let more = CSI_ARG_FLAG_MORE_U32 as c_long;
    unsafe {
        if col.is_indexed() {
            let idx = col.idx();
            if idx < 8 {
                *args.add(argi as usize) = c_long::from(idx) + if fg { 30 } else { 40 };
                argi += 1;
            } else if idx < 16 {
                *args.add(argi as usize) = c_long::from(idx - 8) + if fg { 90 } else { 100 };
                argi += 1;
            } else {
                *args.add(argi as usize) = more | if fg { 38 } else { 48 };
                argi += 1;
                *args.add(argi as usize) = more | 5;
                argi += 1;
                *args.add(argi as usize) = c_long::from(idx);
                argi += 1;
            }
        } else if col.is_rgb() {
            *args.add(argi as usize) = more | if fg { 38 } else { 48 };
            argi += 1;
            *args.add(argi as usize) = more | 2;
            argi += 1;
            *args.add(argi as usize) = more | c_long::from(col.red());
            argi += 1;
            *args.add(argi as usize) = more | c_long::from(col.green());
            argi += 1;
            *args.add(argi as usize) = c_long::from(col.blue());
            argi += 1;
        }
    }
    argi
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_getpen(
    state: *mut VTermStateFull,
    args: *mut c_long,
    _argcount: c_int,
) -> c_int {
    unsafe {
        let mut argi: c_int = 0;
        let more = CSI_ARG_FLAG_MORE_U32 as c_long;

        if (*state).pen.bold() != 0 {
            *args.add(argi as usize) = 1;
            argi += 1;
        }
        if (*state).pen.dim() != 0 {
            *args.add(argi as usize) = 2;
            argi += 1;
        }
        if (*state).pen.italic() != 0 {
            *args.add(argi as usize) = 3;
            argi += 1;
        }
        if (*state).pen.underline() == VTERM_UNDERLINE_SINGLE {
            *args.add(argi as usize) = 4;
            argi += 1;
        }
        if (*state).pen.underline() == VTERM_UNDERLINE_CURLY {
            *args.add(argi as usize) = 4 | more;
            argi += 1;
            *args.add(argi as usize) = 3;
            argi += 1;
        }
        if (*state).pen.blink() != 0 {
            *args.add(argi as usize) = 5;
            argi += 1;
        }
        if (*state).pen.reverse() != 0 {
            *args.add(argi as usize) = 7;
            argi += 1;
        }
        if (*state).pen.conceal() != 0 {
            *args.add(argi as usize) = 8;
            argi += 1;
        }
        if (*state).pen.strike() != 0 {
            *args.add(argi as usize) = 9;
            argi += 1;
        }
        if (*state).pen.font() != 0 {
            *args.add(argi as usize) = (10 + (*state).pen.font()) as c_long;
            argi += 1;
        }
        if (*state).pen.underline() == VTERM_UNDERLINE_DOUBLE {
            *args.add(argi as usize) = 21;
            argi += 1;
        }

        argi = vterm_state_getpen_color(&(*state).pen.fg, argi, args, true);
        argi = vterm_state_getpen_color(&(*state).pen.bg, argi, args, false);

        if (*state).pen.overline() != 0 {
            *args.add(argi as usize) = 53;
            argi += 1;
        }

        if (*state).pen.small() != 0 {
            if (*state).pen.baseline() == VTERM_BASELINE_RAISE {
                *args.add(argi as usize) = 73;
                argi += 1;
            } else if (*state).pen.baseline() == VTERM_BASELINE_LOWER {
                *args.add(argi as usize) = 74;
                argi += 1;
            }
        }

        argi
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_state_set_penattr(
    state: *mut VTermStateFull,
    attr: VTermAttr,
    r#type: VTermValueType,
    val: *mut VTermValue,
) -> c_int {
    if val.is_null() {
        return 0;
    }
    unsafe {
        if r#type != vterm_get_attr_type(attr) {
            return 0;
        }

        match attr {
            VTERM_ATTR_BOLD => (*state).pen.set_bold((*val).boolean as u32),
            VTERM_ATTR_UNDERLINE => (*state).pen.set_underline((*val).number as u32),
            VTERM_ATTR_ITALIC => (*state).pen.set_italic((*val).boolean as u32),
            VTERM_ATTR_BLINK => (*state).pen.set_blink((*val).boolean as u32),
            VTERM_ATTR_REVERSE => (*state).pen.set_reverse((*val).boolean as u32),
            VTERM_ATTR_CONCEAL => (*state).pen.set_conceal((*val).boolean as u32),
            VTERM_ATTR_STRIKE => (*state).pen.set_strike((*val).boolean as u32),
            VTERM_ATTR_FONT => (*state).pen.set_font((*val).number as u32),
            VTERM_ATTR_FOREGROUND => (*state).pen.fg = (*val).color,
            VTERM_ATTR_BACKGROUND => (*state).pen.bg = (*val).color,
            VTERM_ATTR_SMALL => (*state).pen.set_small((*val).boolean as u32),
            VTERM_ATTR_BASELINE => (*state).pen.set_baseline((*val).number as u32),
            VTERM_ATTR_URI => (*state).pen.uri = (*val).number,
            VTERM_ATTR_DIM => (*state).pen.set_dim((*val).boolean as u32),
            VTERM_ATTR_OVERLINE => (*state).pen.set_overline((*val).boolean as u32),
            _ => return 0,
        }

        let callbacks = (*state).callbacks.cast::<VTermStateCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).setpenattr
        {
            cb(attr, val, (*state).cbdata);
        }

        1
    }
}
