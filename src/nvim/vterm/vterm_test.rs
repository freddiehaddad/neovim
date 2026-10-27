//! VTerm unit-test callback and inspection harness (`test/unit/fixtures/vterm_test.c`).

use crate::terminal::Terminal;
use crate::vterm::parser::{CSI_ARG_FLAG_MORE, CSI_ARG_MISSING, VTermStringFragment};
use std::ffi::{c_char, c_int, c_long};
use std::io::Write;
use std::ptr;
use std::sync::Mutex;

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::grid::{schar_T, schar_get};
use crate::mbyte::utf_ptr2StrCharInfo;
use crate::vterm::keyboard::{VTermColor, VTermStateFull};
use crate::vterm::mouse::VTermPos;
use crate::vterm::pen::{
    VTERM_ATTR_BACKGROUND, VTERM_ATTR_BASELINE, VTERM_ATTR_BLINK, VTERM_ATTR_BOLD,
    VTERM_ATTR_CONCEAL, VTERM_ATTR_DIM, VTERM_ATTR_FONT, VTERM_ATTR_FOREGROUND, VTERM_ATTR_ITALIC,
    VTERM_ATTR_OVERLINE, VTERM_ATTR_REVERSE, VTERM_ATTR_SMALL, VTERM_ATTR_STRIKE,
    VTERM_ATTR_UNDERLINE, VTERM_ATTR_URI, VTERM_VALUETYPE_BOOL, VTERM_VALUETYPE_COLOR,
    VTERM_VALUETYPE_INT, VTermAttr, VTermProp, VTermRect, VTermValue, VTermValueType,
};
use crate::vterm::screen::{
    ScreenCell, VTERM_PROP_ALTSCREEN, VTERM_PROP_REVERSE, VTermGlyphInfo, VTermLineInfo,
    VTermScreen, VTermScreenCell, getcell,
};
use crate::vterm::state::{
    VTERM_PROP_CURSORBLINK, VTERM_PROP_CURSORSHAPE, VTERM_PROP_CURSORVISIBLE,
    VTERM_PROP_FOCUSREPORT, VTERM_PROP_ICONNAME, VTERM_PROP_MOUSE, VTERM_PROP_SYNCOUTPUT,
    VTERM_PROP_THEMEUPDATES, VTERM_PROP_TITLE, VTermSelectionMask,
};

pub type VTermAttrMask = c_int;

const CSI_ARG_MASK: c_long = !((1u64 << 31) as c_long);

const VTERM_VALUETYPE_STRING: VTermValueType = 3;

pub(crate) const VTERM_N_ATTRS: VTermAttr = 16;

const VTERM_ATTR_BOLD_MASK: VTermAttrMask = 1 << 0;
const VTERM_ATTR_UNDERLINE_MASK: VTermAttrMask = 1 << 1;
const VTERM_ATTR_ITALIC_MASK: VTermAttrMask = 1 << 2;
const VTERM_ATTR_BLINK_MASK: VTermAttrMask = 1 << 3;
const VTERM_ATTR_REVERSE_MASK: VTermAttrMask = 1 << 4;
const VTERM_ATTR_CONCEAL_MASK: VTermAttrMask = 1 << 5;
const VTERM_ATTR_STRIKE_MASK: VTermAttrMask = 1 << 6;
const VTERM_ATTR_FONT_MASK: VTermAttrMask = 1 << 7;
const VTERM_ATTR_FOREGROUND_MASK: VTermAttrMask = 1 << 8;
const VTERM_ATTR_BACKGROUND_MASK: VTermAttrMask = 1 << 9;
const VTERM_ATTR_SMALL_MASK: VTermAttrMask = 1 << 10;
const VTERM_ATTR_BASELINE_MASK: VTermAttrMask = 1 << 11;
const VTERM_ATTR_URI_MASK: VTermAttrMask = 1 << 12;
const VTERM_ATTR_DIM_MASK: VTermAttrMask = 1 << 13;
const VTERM_ATTR_OVERLINE_MASK: VTermAttrMask = 1 << 14;

#[repr(C)]
struct StatePenRecord {
    bold: c_int,
    underline: c_int,
    italic: c_int,
    blink: c_int,
    reverse: c_int,
    conceal: c_int,
    strike: c_int,
    font: c_int,
    small: c_int,
    baseline: c_int,
    dim: c_int,
    overline: c_int,
    foreground: VTermColor,
    background: VTermColor,
}

struct VTermTestState {
    state_pos: VTermPos,
    want_state_putglyph: bool,
    want_state_movecursor: bool,
    want_state_erase: bool,
    want_state_scrollrect: bool,
    want_state_moverect: bool,
    want_state_settermprop: bool,
    want_state_scrollback: bool,
    want_screen_scrollback: bool,
    STATE_PEN: StatePenRecord,
    TEST_VT: Option<Box<std::cell::UnsafeCell<crate::vterm::mouse::VTerm>>>,
}

impl VTermTestState {
    const fn new() -> Self {
        Self {
            state_pos: VTermPos { row: 0, col: 0 },
            want_state_putglyph: false,
            want_state_movecursor: false,
            want_state_erase: false,
            want_state_scrollrect: false,
            want_state_moverect: false,
            want_state_settermprop: false,
            want_state_scrollback: false,
            want_screen_scrollback: false,
            STATE_PEN: StatePenRecord {
                bold: 0,
                underline: 0,
                italic: 0,
                blink: 0,
                reverse: 0,
                conceal: 0,
                strike: 0,
                font: 0,
                small: 0,
                baseline: 0,
                dim: 0,
                overline: 0,
                foreground: VTermColor { bytes: [0; 4] },
                background: VTermColor { bytes: [0; 4] },
            },
            TEST_VT: None,
        }
    }
}

static VT_TEST_STATE: crate::globals::GlobalState<VTermTestState> =
    crate::globals::GlobalState::new(VTermTestState::new());

#[inline]
unsafe fn vtt_st() -> &'static mut VTermTestState {
    unsafe { &mut *VT_TEST_STATE.as_ptr() }
}

static TEST_OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn append_test_output(bytes: &[u8]) {
    TEST_OUTPUT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend_from_slice(bytes);
}

fn take_test_output() -> Vec<u8> {
    let mut g = TEST_OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *g)
}

fn clear_test_output() {
    TEST_OUTPUT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

unsafe fn push_frag_bytes(out: &mut Vec<u8>, frag: VTermStringFragment) {
    let len = frag.len();
    if len > 0 && !frag.str_.is_null() {
        let slice = unsafe { std::slice::from_raw_parts(frag.str_.cast::<u8>(), len) };
        out.extend_from_slice(slice);
    }
}

pub unsafe fn parser_text(bytes: *const c_char, len: usize, _user: *mut VTermStateFull) -> c_int {
    let mut out = b"text ".to_vec();
    let mut i = 0usize;
    unsafe {
        while i < len {
            let b = *bytes.add(i) as u8;
            if b < 0x20 || b == 0x7f || (0x80..0xa0).contains(&b) {
                break;
            }
            if i > 0 {
                out.push(b',');
            }
            let _ = write!(out, "{b:x}");
            i += 1;
        }
    }
    out.push(b'\n');
    append_test_output(&out);
    i as c_int
}

pub unsafe fn parser_csi<T>(
    leader: *const c_char,
    args: *const c_long,
    argcount: c_int,
    intermed: *const c_char,
    command: c_char,
    _user: *mut T,
) -> c_int {
    let mut out = Vec::new();
    let _ = write!(out, "csi {:02x}", command as u8);
    unsafe {
        if !leader.is_null() && *leader != 0 {
            out.extend_from_slice(b" L=");
            let mut i = 0usize;
            while *leader.add(i) != 0 {
                let _ = write!(out, "{:02x}", *leader.add(i) as u8);
                i += 1;
            }
        }
        for i in 0..(argcount as usize) {
            let sep = if i > 0 { ',' } else { ' ' };
            let a = *args.add(i);
            if a == CSI_ARG_MISSING {
                let _ = write!(out, "{sep}*");
            } else {
                let val = a & CSI_ARG_MASK;
                let more = if (a & CSI_ARG_FLAG_MORE) != 0 {
                    "+"
                } else {
                    ""
                };
                let _ = write!(out, "{sep}{val}{more}");
            }
        }
        if !intermed.is_null() && *intermed != 0 {
            out.extend_from_slice(b" I=");
            let mut i = 0usize;
            while *intermed.add(i) != 0 {
                let _ = write!(out, "{:02x}", *intermed.add(i) as u8);
                i += 1;
            }
        }
    }
    out.push(b'\n');
    append_test_output(&out);
    1
}

pub unsafe fn parser_osc<T>(command: c_int, frag: VTermStringFragment, _user: *mut T) -> c_int {
    let mut out = b"osc ".to_vec();
    if frag.initial() {
        if command == -1 {
            out.push(b'[');
        } else {
            let _ = write!(out, "[{command};");
        }
    }
    unsafe {
        push_frag_bytes(&mut out, frag);
    }
    if frag.final_() {
        out.push(b']');
    }
    out.push(b'\n');
    append_test_output(&out);
    1
}

pub unsafe fn parser_dcs<T>(
    command: *const c_char,
    commandlen: usize,
    frag: VTermStringFragment,
    _user: *mut T,
) -> c_int {
    let mut out = b"dcs ".to_vec();
    if frag.initial() {
        out.push(b'[');
        if commandlen > 0 && !command.is_null() {
            let cmd = unsafe { std::slice::from_raw_parts(command.cast::<u8>(), commandlen) };
            out.extend_from_slice(cmd);
        }
    }
    unsafe {
        push_frag_bytes(&mut out, frag);
    }
    if frag.final_() {
        out.push(b']');
    }
    out.push(b'\n');
    append_test_output(&out);
    1
}

pub unsafe fn parser_apc<T>(frag: VTermStringFragment, _user: *mut T) -> c_int {
    let mut out = b"apc ".to_vec();
    if frag.initial() {
        out.push(b'[');
    }
    unsafe {
        push_frag_bytes(&mut out, frag);
    }
    if frag.final_() {
        out.push(b']');
    }
    out.push(b'\n');
    append_test_output(&out);
    1
}

pub unsafe fn parser_pm<T>(frag: VTermStringFragment, _user: *mut T) -> c_int {
    let mut out = b"pm ".to_vec();
    if frag.initial() {
        out.push(b'[');
    }
    unsafe {
        push_frag_bytes(&mut out, frag);
    }
    if frag.final_() {
        out.push(b']');
    }
    out.push(b'\n');
    append_test_output(&out);
    1
}

pub unsafe fn parser_sos<T>(frag: VTermStringFragment, _user: *mut T) -> c_int {
    let mut out = b"sos ".to_vec();
    if frag.initial() {
        out.push(b'[');
    }
    unsafe {
        push_frag_bytes(&mut out, frag);
    }
    if frag.final_() {
        out.push(b']');
    }
    out.push(b'\n');
    append_test_output(&out);
    1
}

pub unsafe fn selection_set(
    mask: VTermSelectionMask,
    frag: VTermStringFragment,
    _user: *mut Terminal,
) -> c_int {
    let mut out = Vec::new();
    let _ = write!(out, "selection-set mask={mask:04X} ");
    if frag.initial() {
        out.push(b'[');
    }
    unsafe {
        push_frag_bytes(&mut out, frag);
    }
    if frag.final_() {
        out.push(b']');
    }
    out.push(b'\n');
    append_test_output(&out);
    1
}

pub unsafe fn selection_query(mask: VTermSelectionMask, _user: *mut Terminal) -> c_int {
    let mut out = Vec::new();
    let _ = writeln!(out, "selection-query mask={mask:04X}");
    append_test_output(&out);
    1
}

unsafe fn format_schar(out: &mut Vec<u8>, schar: schar_T) {
    unsafe {
        let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(buf.as_mut_ptr(), schar);
        let mut ci = utf_ptr2StrCharInfo(buf.as_ptr());
        let mut did = false;
        while *ci.ptr != 0 {
            if did {
                out.push(b',');
            }
            if ci.chr.len == 1 && ci.chr.value >= 0x80 {
                let _ = write!(out, "??{:x}", ci.chr.value);
            } else {
                let _ = write!(out, "{:x}", ci.chr.value);
            }
            did = true;
            ci = utf_ptr2StrCharInfo(ci.ptr.add(ci.chr.len as usize));
        }
    }
}

pub unsafe fn state_putglyph(
    info: *mut VTermGlyphInfo,
    pos: VTermPos,
    _user: *mut VTermScreen,
) -> c_int {
    unsafe {
        if !vtt_st().want_state_putglyph {
            return 1;
        }
        let mut out = b"putglyph ".to_vec();
        format_schar(&mut out, (*info).schar);
        let _ = write!(out, " {} {},{}", (*info).width, pos.row, pos.col);
        if (*info).protected_cell() != 0 {
            out.extend_from_slice(b" prot");
        }
        if (*info).dwl() != 0 {
            out.extend_from_slice(b" dwl");
        }
        let dhl = (*info).dhl();
        if dhl != 0 {
            let s = match dhl {
                1 => "top",
                2 => "bottom",
                _ => "?",
            };
            let _ = write!(out, " dhl-{s}");
        }
        out.push(b'\n');
        append_test_output(&out);
        1
    }
}

pub unsafe fn state_movecursor<T>(
    pos: VTermPos,
    _oldpos: VTermPos,
    _visible: c_int,
    _user: *mut T,
) -> c_int {
    unsafe {
        vtt_st().state_pos = pos;
        let mut out = Vec::new();
        if vtt_st().want_state_movecursor {
            let _ = writeln!(out, "movecursor {},{}", pos.row, pos.col);
        }
        append_test_output(&out);
        1
    }
}

pub unsafe fn state_scrollrect(
    rect: VTermRect,
    downward: c_int,
    rightward: c_int,
    _user: *mut VTermScreen,
) -> c_int {
    unsafe {
        if !vtt_st().want_state_scrollrect {
            return 0;
        }
        let mut out = Vec::new();
        let _ = writeln!(
            out,
            "scrollrect {}..{},{}..{} => {:+},{:+}",
            rect.start_row, rect.end_row, rect.start_col, rect.end_col, downward, rightward
        );
        append_test_output(&out);
        1
    }
}

pub unsafe fn state_moverect<T>(dest: VTermRect, src: VTermRect, _user: *mut T) -> c_int {
    unsafe {
        if !vtt_st().want_state_moverect {
            return 0;
        }
        let mut out = Vec::new();
        let _ = writeln!(
            out,
            "moverect {}..{},{}..{} -> {}..{},{}..{}",
            src.start_row,
            src.end_row,
            src.start_col,
            src.end_col,
            dest.start_row,
            dest.end_row,
            dest.start_col,
            dest.end_col
        );
        append_test_output(&out);
        1
    }
}

fn format_color_bytes(out: &mut Vec<u8>, col: &VTermColor) {
    if col.is_rgb() {
        let _ = write!(out, "rgb({},{},{}", col.red(), col.green(), col.blue());
    } else if col.is_indexed() {
        let _ = write!(out, "idx({}", col.idx());
    } else {
        let _ = write!(out, "invalid({}", col.type_byte());
    }
    if col.is_default_fg() {
        out.extend_from_slice(b",is_default_fg");
    }
    if col.is_default_bg() {
        out.extend_from_slice(b",is_default_bg");
    }
    out.push(b')');
}

pub unsafe fn print_color(col: *const VTermColor) {
    if col.is_null() {
        return;
    }
    let mut out = Vec::new();
    unsafe {
        format_color_bytes(&mut out, &*col);
    }
    append_test_output(&out);
}

const fn vterm_get_prop_type(prop: VTermProp) -> VTermValueType {
    match prop {
        VTERM_PROP_CURSORVISIBLE
        | VTERM_PROP_CURSORBLINK
        | VTERM_PROP_ALTSCREEN
        | VTERM_PROP_REVERSE
        | VTERM_PROP_FOCUSREPORT
        | VTERM_PROP_THEMEUPDATES
        | VTERM_PROP_SYNCOUTPUT => VTERM_VALUETYPE_BOOL,
        VTERM_PROP_TITLE | VTERM_PROP_ICONNAME => VTERM_VALUETYPE_STRING,
        VTERM_PROP_CURSORSHAPE | VTERM_PROP_MOUSE => VTERM_VALUETYPE_INT,
        _ => 0,
    }
}

pub unsafe fn state_settermprop<T>(prop: VTermProp, val: *mut VTermValue, _user: *mut T) -> c_int {
    unsafe {
        if !vtt_st().want_state_settermprop {
            return 1;
        }
        let mut out = Vec::new();
        let errcode = match vterm_get_prop_type(prop) {
            VTERM_VALUETYPE_BOOL => {
                let s = if (*val).boolean() != 0 {
                    "true"
                } else {
                    "false"
                };
                let _ = writeln!(out, "settermprop {prop} {s}");
                1
            }
            VTERM_VALUETYPE_INT => {
                let _ = writeln!(out, "settermprop {prop} {}", (*val).number());
                1
            }
            VTERM_VALUETYPE_STRING => {
                let frag = (*val).string();
                let _ = write!(
                    out,
                    "settermprop {prop} {}\"",
                    if frag.initial() { "[" } else { "" }
                );
                push_frag_bytes(&mut out, frag);
                let _ = writeln!(out, "\"{}", if frag.final_() { "]" } else { "" });
                0
            }
            VTERM_VALUETYPE_COLOR => {
                let _ = write!(out, "settermprop {prop} ");
                format_color_bytes(&mut out, &(*val).color());
                out.push(b'\n');
                1
            }
            _ => 0,
        };
        append_test_output(&out);
        errcode
    }
}

pub unsafe fn state_erase(rect: VTermRect, selective: c_int, _user: *mut VTermScreen) -> c_int {
    unsafe {
        if !vtt_st().want_state_erase {
            return 1;
        }
        let mut out = Vec::new();
        let sel = if selective != 0 { " selective" } else { "" };
        let _ = writeln!(
            out,
            "erase {}..{},{}..{}{sel}",
            rect.start_row, rect.end_row, rect.start_col, rect.end_col
        );
        append_test_output(&out);
        1
    }
}

pub unsafe fn state_setpenattr(
    attr: VTermAttr,
    val: *mut VTermValue,
    _user: *mut VTermScreen,
) -> c_int {
    unsafe {
        match attr {
            VTERM_ATTR_BOLD => vtt_st().STATE_PEN.bold = (*val).boolean(),
            VTERM_ATTR_UNDERLINE => vtt_st().STATE_PEN.underline = (*val).number(),
            VTERM_ATTR_ITALIC => vtt_st().STATE_PEN.italic = (*val).boolean(),
            VTERM_ATTR_BLINK => vtt_st().STATE_PEN.blink = (*val).boolean(),
            VTERM_ATTR_REVERSE => vtt_st().STATE_PEN.reverse = (*val).boolean(),
            VTERM_ATTR_CONCEAL => vtt_st().STATE_PEN.conceal = (*val).boolean(),
            VTERM_ATTR_STRIKE => vtt_st().STATE_PEN.strike = (*val).boolean(),
            VTERM_ATTR_FONT => vtt_st().STATE_PEN.font = (*val).number(),
            VTERM_ATTR_SMALL => vtt_st().STATE_PEN.small = (*val).boolean(),
            VTERM_ATTR_BASELINE => vtt_st().STATE_PEN.baseline = (*val).number(),
            VTERM_ATTR_DIM => vtt_st().STATE_PEN.dim = (*val).boolean(),
            VTERM_ATTR_OVERLINE => vtt_st().STATE_PEN.overline = (*val).boolean(),
            VTERM_ATTR_FOREGROUND => vtt_st().STATE_PEN.foreground = (*val).color(),
            VTERM_ATTR_BACKGROUND => vtt_st().STATE_PEN.background = (*val).color(),
            VTERM_N_ATTRS => return 0,
            _ => {}
        }
        1
    }
}

pub unsafe fn state_sb_clear(_user: *mut VTermScreen) -> c_int {
    unsafe {
        if !vtt_st().want_state_scrollback {
            return 1;
        }
        append_test_output(b"sb_clear\n");
        0
    }
}

pub unsafe fn screen_sb_pushline(
    cols: c_int,
    cells: *const VTermScreenCell,
    _user: *mut Terminal,
) -> c_int {
    unsafe {
        if !vtt_st().want_screen_scrollback {
            return 1;
        }
        let mut eol = cols as usize;
        while eol > 0 && (*cells.add(eol - 1)).schar == 0 {
            eol -= 1;
        }
        let mut out = Vec::new();
        let _ = write!(out, "sb_pushline {cols} =");
        for c in 0..eol {
            out.push(b' ');
            format_schar(&mut out, (*cells.add(c)).schar);
        }
        out.push(b'\n');
        append_test_output(&out);
        1
    }
}

pub unsafe fn screen_sb_popline(
    cols: c_int,
    cells: *mut VTermScreenCell,
    _user: *mut Terminal,
) -> c_int {
    unsafe {
        if !vtt_st().want_screen_scrollback {
            return 0;
        }
        for col in 0..(cols as usize) {
            let cell = cells.add(col);
            (*cell).schar = if col < 5 {
                (b'A' + col as u8) as schar_T
            } else {
                0
            };
            (*cell).width = 1;
        }
        let mut out = Vec::new();
        let _ = writeln!(out, "sb_popline {cols}");
        append_test_output(&out);
        1
    }
}

pub unsafe fn screen_sb_clear(_user: *mut Terminal) -> c_int {
    unsafe {
        if !vtt_st().want_screen_scrollback {
            return 1;
        }
        append_test_output(b"sb_clear\n");
        0
    }
}

pub unsafe fn term_output(s: *const c_char, len: usize, _user: *mut Terminal) {
    let mut out = b"output ".to_vec();
    unsafe {
        for i in 0..len {
            let b = *s.add(i) as u8;
            let sep = if i + 1 < len { "," } else { "\n" };
            let _ = write!(out, "{b:x}{sep}");
        }
    }
    append_test_output(&out);
}

pub unsafe fn vterm_state_get_penattr(
    state: *const VTermStateFull,
    attr: VTermAttr,
    val: *mut VTermValue,
) -> c_int {
    unsafe {
        let pen = &(*state).pen;
        match attr {
            VTERM_ATTR_BOLD => {
                *val = VTermValue::Boolean(pen.bold() as c_int);
                1
            }
            VTERM_ATTR_UNDERLINE => {
                *val = VTermValue::Number(pen.underline() as c_int);
                1
            }
            VTERM_ATTR_ITALIC => {
                *val = VTermValue::Boolean(pen.italic() as c_int);
                1
            }
            VTERM_ATTR_BLINK => {
                *val = VTermValue::Boolean(pen.blink() as c_int);
                1
            }
            VTERM_ATTR_REVERSE => {
                *val = VTermValue::Boolean(pen.reverse() as c_int);
                1
            }
            VTERM_ATTR_CONCEAL => {
                *val = VTermValue::Boolean(pen.conceal() as c_int);
                1
            }
            VTERM_ATTR_STRIKE => {
                *val = VTermValue::Boolean(pen.strike() as c_int);
                1
            }
            VTERM_ATTR_FONT => {
                *val = VTermValue::Number(pen.font() as c_int);
                1
            }
            VTERM_ATTR_FOREGROUND => {
                *val = VTermValue::Color(pen.fg);
                1
            }
            VTERM_ATTR_BACKGROUND => {
                *val = VTermValue::Color(pen.bg);
                1
            }
            VTERM_ATTR_SMALL => {
                *val = VTermValue::Boolean(pen.small() as c_int);
                1
            }
            VTERM_ATTR_BASELINE => {
                *val = VTermValue::Number(pen.baseline() as c_int);
                1
            }
            VTERM_ATTR_URI => {
                *val = VTermValue::Number(pen.uri);
                1
            }
            VTERM_ATTR_DIM => {
                *val = VTermValue::Boolean(pen.dim() as c_int);
                1
            }
            VTERM_ATTR_OVERLINE => {
                *val = VTermValue::Boolean(pen.overline() as c_int);
                1
            }
            _ => 0,
        }
    }
}

pub unsafe fn vterm_color_is_equal(a: *const VTermColor, b: *const VTermColor) -> c_int {
    unsafe {
        let ca = &*a;
        let cb = &*b;
        if ca.type_byte() != cb.type_byte() {
            return 0;
        }
        if ca.is_indexed() {
            c_int::from(ca.idx() == cb.idx())
        } else if ca.is_rgb() {
            c_int::from(ca.red() == cb.red() && ca.green() == cb.green() && ca.blue() == cb.blue())
        } else {
            0
        }
    }
}

unsafe fn attrs_differ(attrs: VTermAttrMask, a: *const ScreenCell, b: *const ScreenCell) -> bool {
    unsafe {
        let pa = &(*a).pen;
        let pb = &(*b).pen;
        if (attrs & VTERM_ATTR_BOLD_MASK) != 0 && pa.bold() != pb.bold() {
            return true;
        }
        if (attrs & VTERM_ATTR_UNDERLINE_MASK) != 0 && pa.underline() != pb.underline() {
            return true;
        }
        if (attrs & VTERM_ATTR_ITALIC_MASK) != 0 && pa.italic() != pb.italic() {
            return true;
        }
        if (attrs & VTERM_ATTR_BLINK_MASK) != 0 && pa.blink() != pb.blink() {
            return true;
        }
        if (attrs & VTERM_ATTR_REVERSE_MASK) != 0 && pa.reverse() != pb.reverse() {
            return true;
        }
        if (attrs & VTERM_ATTR_CONCEAL_MASK) != 0 && pa.conceal() != pb.conceal() {
            return true;
        }
        if (attrs & VTERM_ATTR_STRIKE_MASK) != 0 && pa.strike() != pb.strike() {
            return true;
        }
        if (attrs & VTERM_ATTR_FONT_MASK) != 0 && pa.font() != pb.font() {
            return true;
        }
        if (attrs & VTERM_ATTR_FOREGROUND_MASK) != 0
            && vterm_color_is_equal(&raw const pa.fg, &raw const pb.fg) == 0
        {
            return true;
        }
        if (attrs & VTERM_ATTR_BACKGROUND_MASK) != 0
            && vterm_color_is_equal(&raw const pa.bg, &raw const pb.bg) == 0
        {
            return true;
        }
        if (attrs & VTERM_ATTR_SMALL_MASK) != 0 && pa.small() != pb.small() {
            return true;
        }
        if (attrs & VTERM_ATTR_BASELINE_MASK) != 0 && pa.baseline() != pb.baseline() {
            return true;
        }
        if (attrs & VTERM_ATTR_URI_MASK) != 0 && pa.uri != pb.uri {
            return true;
        }
        if (attrs & VTERM_ATTR_DIM_MASK) != 0 && pa.dim() != pb.dim() {
            return true;
        }
        if (attrs & VTERM_ATTR_OVERLINE_MASK) != 0 && pa.overline() != pb.overline() {
            return true;
        }
        false
    }
}

pub unsafe fn vterm_screen_get_attrs_extent(
    screen: *const VTermScreen,
    extent: *mut VTermRect,
    pos: VTermPos,
    attrs: VTermAttrMask,
) -> c_int {
    unsafe {
        let target = getcell(screen, pos.row, pos.col);
        (*extent).start_row = pos.row;
        (*extent).end_row = pos.row + 1;
        if (*extent).start_col < 0 {
            (*extent).start_col = 0;
        }
        if (*extent).end_col < 0 {
            (*extent).end_col = (*screen).cols;
        }

        let mut col = pos.col - 1;
        while col >= (*extent).start_col {
            if attrs_differ(attrs, target, getcell(screen, pos.row, col)) {
                break;
            }
            col -= 1;
        }
        (*extent).start_col = col + 1;

        col = pos.col + 1;
        while col < (*extent).end_col {
            if attrs_differ(attrs, target, getcell(screen, pos.row, col)) {
                break;
            }
            col += 1;
        }
        (*extent).end_col = col - 1;
        1
    }
}

pub unsafe fn vterm_screen_get_text(
    screen: *const VTermScreen,
    buffer: *mut c_char,
    len: usize,
    rect: VTermRect,
) -> usize {
    let mut outpos = 0usize;
    let mut padding = 0usize;

    macro_rules! put_bytes {
        ($src:expr, $n:expr) => {{
            let n: usize = $n;
            if !buffer.is_null() && outpos + n <= len {
                ptr::copy_nonoverlapping($src, buffer.add(outpos), n);
            }
            outpos += n;
        }};
    }

    unsafe {
        for row in rect.start_row..rect.end_row {
            for col in rect.start_col..rect.end_col {
                let cell = getcell(screen, row, col);
                if (*cell).schar == 0 {
                    padding += 1;
                } else if (*cell).schar == u32::MAX {
                    // Gap behind a double-width char
                } else {
                    while padding > 0 {
                        put_bytes!(c" ".as_ptr(), 1);
                        padding -= 1;
                    }
                    let mut buf = [0 as c_char; MAX_SCHAR_SIZE + 1];
                    let thislen = schar_get(buf.as_mut_ptr(), (*cell).schar);
                    put_bytes!(buf.as_ptr(), thislen);
                }
            }
            if row < rect.end_row - 1 {
                put_bytes!(c"\n".as_ptr(), 1);
                padding = 0;
            }
        }
    }

    outpos
}

pub unsafe fn vterm_screen_is_eol(screen: *const VTermScreen, mut pos: VTermPos) -> c_int {
    unsafe {
        while pos.col < (*screen).cols {
            let cell = getcell(screen, pos.row, pos.col);
            if (*cell).schar != 0 {
                return 0;
            }
            pos.col += 1;
        }
        1
    }
}

pub unsafe fn vterm_state_get_cursorpos(state: *const VTermStateFull, cursorpos: *mut VTermPos) {
    unsafe {
        *cursorpos = (*state).pos;
    }
}

pub unsafe fn vterm_state_set_bold_highbright(
    state: *mut VTermStateFull,
    bold_is_highbright: c_int,
) {
    unsafe {
        (*state).bold_is_highbright = bold_is_highbright;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbyte::{utf_ptr2char, utf_ptr2len};
    use crate::vterm::encoding::{VTermEncodingType, vterm_lookup_encoding};
    use crate::vterm::keyboard::{
        VTERM_KEY_BACKSPACE, VTERM_KEY_ENTER, VTERM_KEY_FUNCTION_0, VTERM_KEY_KP_0, VTERM_KEY_NONE,
        VTERM_KEY_TAB, VTERM_MOD_ALT, VTERM_MOD_CTRL, VTERM_MOD_SHIFT, VTermEncodingInstance,
        VTermKey, VTermModifier, vterm_keyboard_end_paste, vterm_keyboard_key,
        vterm_keyboard_start_paste, vterm_keyboard_unichar,
    };
    use crate::vterm::mouse::{VTerm, vterm_mouse_button, vterm_mouse_move};
    use crate::vterm::parser::{
        VTermParserCallbacks, vterm_input_write, vterm_parser_set_callbacks,
    };
    use crate::vterm::pen::VTermStateCallbacks;
    use crate::vterm::screen::{
        VTermScreenCallbacks, vterm_obtain_screen, vterm_screen_convert_color_to_rgb,
        vterm_screen_enable_altscreen, vterm_screen_enable_reflow, vterm_screen_get_cell,
        vterm_screen_reset, vterm_screen_set_callbacks,
    };
    use crate::vterm::state::{
        VTermSelectionCallbacks, VTermStateFallbacks, vterm_obtain_state, vterm_state_focus_in,
        vterm_state_focus_out, vterm_state_get_lineinfo, vterm_state_reset,
        vterm_state_set_callbacks, vterm_state_set_selection_callbacks,
        vterm_state_set_unrecognised_fallbacks,
    };
    use crate::vterm::vterm::{
        vterm_new, vterm_output_set_callback, vterm_set_size, vterm_set_utf8,
    };
    use std::ffi::CStr;

    static VTERM_TEST_LOCK: Mutex<()> = Mutex::new(());

    unsafe fn parser_control<T>(control: u8, _user: *mut T) -> c_int {
        let s = format!("control {control:02x}\n");
        append_test_output(s.as_bytes());
        1
    }

    unsafe fn parser_escape(
        bytes: *const c_char,
        _len: usize,
        _user: *mut VTermStateFull,
    ) -> c_int {
        let slice = unsafe { CStr::from_ptr(bytes).to_bytes() };
        let mut out = b"escape ".to_vec();
        out.extend_from_slice(slice);
        out.push(b'\n');
        append_test_output(&out);
        1
    }

    unsafe fn state_setlineinfo(
        _row: c_int,
        _newinfo: *const VTermLineInfo,
        _oldinfo: *const VTermLineInfo,
        _user: *mut VTermScreen,
    ) -> c_int {
        1
    }

    static PARSER_CBS: VTermParserCallbacks = VTermParserCallbacks {
        text: Some(parser_text),
        control: Some(parser_control),
        escape: Some(parser_escape),
        csi: Some(parser_csi),
        osc: Some(parser_osc),
        dcs: Some(parser_dcs),
        apc: Some(parser_apc),
        pm: Some(parser_pm),
        sos: Some(parser_sos),
        resize: None,
    };

    static STATE_CBS: VTermStateCallbacks = VTermStateCallbacks {
        putglyph: Some(state_putglyph),
        movecursor: Some(state_movecursor),
        scrollrect: Some(state_scrollrect),
        moverect: Some(state_moverect),
        erase: Some(state_erase),
        initpen: None,
        setpenattr: Some(state_setpenattr),
        settermprop: Some(state_settermprop),
        bell: None,
        resize: None,
        theme: None,
        setlineinfo: Some(state_setlineinfo),
        sb_clear: Some(state_sb_clear),
    };

    static SELECTION_CBS: VTermSelectionCallbacks = VTermSelectionCallbacks {
        set: Some(selection_set),
        query: Some(selection_query),
    };

    static FALLBACK_CBS: VTermStateFallbacks = VTermStateFallbacks {
        control: Some(parser_control),
        csi: Some(parser_csi),
        osc: Some(parser_osc),
        dcs: Some(parser_dcs),
        apc: Some(parser_apc),
        pm: Some(parser_pm),
        sos: Some(parser_sos),
    };

    static SCREEN_CBS: VTermScreenCallbacks = VTermScreenCallbacks {
        damage: None,
        moverect: Some(state_moverect),
        movecursor: Some(state_movecursor),
        settermprop: Some(state_settermprop),
        bell: None,
        resize: None,
        theme: None,
        sb_pushline: Some(screen_sb_pushline),
        sb_popline: Some(screen_sb_popline),
        sb_clear: Some(screen_sb_clear),
    };

    #[derive(Default)]
    struct StateOpts {
        b: bool,
        e: bool,
        f: bool,
        g: bool,
        m: bool,
        p: bool,
        s: bool,
    }

    #[derive(Default)]
    struct ScreenOpts {
        a: bool,
        b: bool,
        c: bool,
        m: bool,
        p: bool,
        r: bool,
    }

    unsafe fn init() -> *mut VTerm {
        unsafe {
            crate::option_vars::p_emoji = 1;
            vtt_st().state_pos = VTermPos { row: 0, col: 0 };
            vtt_st().want_state_putglyph = false;
            vtt_st().want_state_movecursor = false;
            vtt_st().want_state_erase = false;
            vtt_st().want_state_scrollrect = false;
            vtt_st().want_state_moverect = false;
            vtt_st().want_state_settermprop = false;
            vtt_st().want_state_scrollback = false;
            vtt_st().want_screen_scrollback = false;
            let cell = Box::new(std::cell::UnsafeCell::new(vterm_new(25, 80)));
            let vt = cell.get();
            vtt_st().TEST_VT = Some(cell);
            vterm_output_set_callback(vt, Some(term_output), ptr::null_mut());
            vterm_set_utf8(vt, 1);
            vt
        }
    }

    unsafe fn vterm_free(_vt: *mut VTerm) {
        unsafe {
            vtt_st().TEST_VT = None;
        }
    }

    unsafe fn wantparser(vt: *mut VTerm) {
        unsafe {
            vterm_parser_set_callbacks(vt, Some(&PARSER_CBS), ptr::null_mut());
        }
    }

    unsafe fn wantstate(vt: *mut VTerm, opts: StateOpts) -> *mut VTermStateFull {
        unsafe {
            let state = vterm_obtain_state(vt);
            vterm_state_set_callbacks(state, Some(&STATE_CBS), ptr::null_mut());
            vterm_state_set_selection_callbacks(
                &mut *state,
                Some(&SELECTION_CBS),
                ptr::null_mut(),
                16,
            );
            vterm_state_set_bold_highbright(state, 1);
            vterm_state_reset(state, 1);

            let fb = opts.f.then_some(&FALLBACK_CBS);
            vtt_st().want_state_scrollback = opts.b;
            vtt_st().want_state_erase = opts.e;
            vterm_state_set_unrecognised_fallbacks(state, fb, ptr::null_mut());
            vtt_st().want_state_putglyph = opts.g;
            vtt_st().want_state_moverect = opts.m;
            vtt_st().want_state_settermprop = opts.p;
            vtt_st().want_state_scrollrect = opts.s;
            state
        }
    }

    unsafe fn wantscreen(vt: *mut VTerm, opts: ScreenOpts) -> *mut VTermScreen {
        unsafe {
            let screen = vterm_obtain_screen(vt);
            vterm_screen_set_callbacks(screen, Some(&SCREEN_CBS), ptr::null_mut());
            if opts.a {
                vterm_screen_enable_altscreen(screen, 1);
            }
            vtt_st().want_screen_scrollback = opts.b;
            vtt_st().want_state_movecursor = opts.c;
            vtt_st().want_state_moverect = opts.m;
            vtt_st().want_state_settermprop = opts.p;
            if opts.r {
                vterm_screen_enable_reflow(screen, true);
            }
            screen
        }
    }

    unsafe fn reset(state: *mut VTermStateFull, screen: *mut VTermScreen) {
        unsafe {
            if !state.is_null() {
                vterm_state_reset(state, 1);
                vterm_state_get_cursorpos(state, &raw mut vtt_st().state_pos);
            }
            if !screen.is_null() {
                vterm_screen_reset(screen, 1);
            }
        }
    }

    unsafe fn push(input: &[u8], vt: *mut VTerm) {
        unsafe {
            vterm_input_write(vt, input.as_ptr().cast(), input.len());
        }
    }

    fn expect(expected: &[u8]) {
        let actual = take_test_output();
        let mut exp = expected.to_vec();
        exp.push(b'\n');
        assert_eq!(
            String::from_utf8_lossy(&actual),
            String::from_utf8_lossy(&exp)
        );
    }

    fn expect_output(expected_preformat: &[u8]) {
        let actual = take_test_output();
        let mut exp = String::from("output ");
        for (i, &b) in expected_preformat.iter().enumerate() {
            if i > 0 {
                exp.push(',');
            }
            use std::fmt::Write as _;
            let _ = write!(exp, "{b:x}");
        }
        exp.push('\n');
        assert_eq!(String::from_utf8_lossy(&actual), exp);
    }

    unsafe fn cursor(row: c_int, col: c_int, state: *const VTermStateFull) {
        unsafe {
            let mut pos = VTermPos { row: 0, col: 0 };
            vterm_state_get_cursorpos(state, &raw mut pos);
            assert_eq!((pos.row, pos.col), (row, col));
        }
    }

    unsafe fn lineinfo(
        row: c_int,
        exp_dwl: bool,
        exp_dhl: bool,
        exp_cont: bool,
        state: *const VTermStateFull,
    ) {
        unsafe {
            let info = vterm_state_get_lineinfo(&*state, row);
            assert_eq!(info.doublewidth() == 1, exp_dwl);
            assert_eq!(info.doubleheight() == 1, exp_dhl);
            assert_eq!(info.continuation(), exp_cont);
        }
    }

    unsafe fn pen(attribute: &str, expected: c_int, state: *const VTermStateFull) {
        let attr = match attribute {
            "bold" => VTERM_ATTR_BOLD,
            "underline" => VTERM_ATTR_UNDERLINE,
            "italic" => VTERM_ATTR_ITALIC,
            "blink" => VTERM_ATTR_BLINK,
            "reverse" => VTERM_ATTR_REVERSE,
            "font" => VTERM_ATTR_FONT,
            "dim" => VTERM_ATTR_DIM,
            "overline" => VTERM_ATTR_OVERLINE,
            _ => unreachable!(),
        };
        unsafe {
            let mut val = VTermValue::Boolean(0);
            vterm_state_get_penattr(state, attr, &raw mut val);
            assert_eq!(val.boolean(), expected);
        }
    }

    unsafe fn resize(rows: c_int, cols: c_int, vt: *mut VTerm) {
        unsafe {
            vterm_set_size(vt, rows, cols);
        }
    }

    unsafe fn screen_chars(
        start_row: c_int,
        start_col: c_int,
        end_row: c_int,
        end_col: c_int,
        expected: &[u8],
        screen: *const VTermScreen,
    ) {
        unsafe {
            let rect = VTermRect {
                start_row,
                start_col,
                end_row,
                end_col,
            };
            let len = vterm_screen_get_text(screen, ptr::null_mut(), 0, rect);
            let mut buf = vec![0u8; len];
            vterm_screen_get_text(screen, buf.as_mut_ptr().cast(), len, rect);
            assert_eq!(buf, expected);
        }
    }

    unsafe fn screen_text(
        start_row: c_int,
        start_col: c_int,
        end_row: c_int,
        end_col: c_int,
        expected: &[u8],
        screen: *const VTermScreen,
    ) {
        unsafe {
            let rect = VTermRect {
                start_row,
                start_col,
                end_row,
                end_col,
            };
            let len = vterm_screen_get_text(screen, ptr::null_mut(), 0, rect);
            let mut buf = vec![0u8; len];
            vterm_screen_get_text(screen, buf.as_mut_ptr().cast(), len, rect);
            let mut actual = String::new();
            for (i, &b) in buf.iter().enumerate() {
                if i > 0 {
                    actual.push(',');
                }
                use std::fmt::Write as _;
                let _ = write!(actual, "{b:02x}");
            }
            assert_eq!(actual.as_bytes(), expected);
        }
    }

    unsafe fn screen_row(row: c_int, expected: &[u8], screen: *const VTermScreen, end_col: c_int) {
        unsafe {
            screen_chars(row, 0, row + 1, end_col, expected, screen);
        }
    }

    unsafe fn screen_cell(row: c_int, col: c_int, expected: &[u8], screen: *mut VTermScreen) {
        use std::fmt::Write as _;
        unsafe {
            let pos = VTermPos { row, col };
            let mut cell = VTermScreenCell::default();
            vterm_screen_get_cell(screen, pos, &raw mut cell);

            let mut buf = [0u8; 32];
            schar_get(buf.as_mut_ptr().cast(), cell.schar);

            let mut actual = String::from("{");
            let mut i = 0usize;
            while buf[i] > 0 {
                let p = buf.as_ptr().add(i).cast::<c_char>();
                let ch = utf_ptr2char(p);
                let chlen = utf_ptr2len(p) as usize;
                if i > 0 {
                    actual.push(',');
                }
                let invalid = ch >= 128 && chlen == 1;
                let _ = write!(actual, "{}{:02x}", if invalid { "?" } else { "" }, ch);
                i += chlen;
            }
            let _ = write!(actual, "}} width={} attrs={{", cell.width);
            if cell.attrs.bold() != 0 {
                actual.push('B');
            }
            if cell.attrs.dim() != 0 {
                actual.push('D');
            }
            if cell.attrs.underline() != 0 {
                let _ = write!(actual, "U{}", cell.attrs.underline());
            }
            if cell.attrs.italic() != 0 {
                actual.push('I');
            }
            if cell.attrs.blink() != 0 {
                actual.push('K');
            }
            if cell.attrs.reverse() != 0 {
                actual.push('R');
            }
            if cell.attrs.overline() != 0 {
                actual.push('O');
            }
            if cell.attrs.font() != 0 {
                let _ = write!(actual, "F{}", cell.attrs.font());
            }
            if cell.attrs.small() != 0 {
                actual.push('S');
            }
            if cell.attrs.baseline() != 0 {
                actual.push(if cell.attrs.baseline() == 1 { '^' } else { '_' });
            }
            actual.push_str("} ");
            let dwl = (cell.attrs.flags >> 12) & 0x01;
            let dhl = (cell.attrs.flags >> 13) & 0x03;
            if dwl != 0 {
                actual.push_str("dwl ");
            }
            if dhl != 0 {
                let _ = write!(actual, "dhl-{} ", if dhl == 2 { "bottom" } else { "top" });
            }
            actual.push_str("fg=");
            vterm_screen_convert_color_to_rgb(screen, &raw mut cell.fg);
            print_color(&raw const cell.fg);
            actual.push_str(&String::from_utf8_lossy(&take_test_output()));
            actual.push_str(" bg=");
            vterm_screen_convert_color_to_rgb(screen, &raw mut cell.bg);
            print_color(&raw const cell.bg);
            actual.push_str(&String::from_utf8_lossy(&take_test_output()));

            assert_eq!(actual, String::from_utf8_lossy(expected));
        }
    }

    unsafe fn screen_eol(row: c_int, col: c_int, expected: c_int, screen: *const VTermScreen) {
        unsafe {
            assert_eq!(vterm_screen_is_eol(screen, VTermPos { row, col }), expected);
        }
    }

    unsafe fn screen_attrs_extent(
        row: c_int,
        col: c_int,
        expected: &[u8],
        screen: *const VTermScreen,
    ) {
        unsafe {
            let mut rect = VTermRect {
                start_row: 0,
                end_row: 0,
                start_col: 0,
                end_col: -1,
            };
            vterm_screen_get_attrs_extent(screen, &raw mut rect, VTermPos { row, col }, 1);
            let actual = format!(
                "{},{}-{},{}",
                rect.start_row, rect.start_col, rect.end_row, rect.end_col
            );
            assert_eq!(actual.as_bytes(), expected);
        }
    }

    unsafe fn wantencoding() -> VTermEncodingInstance {
        let enc = vterm_lookup_encoding(VTermEncodingType::ENC_UTF8, b'u' as c_char).unwrap();
        let mut inst = VTermEncodingInstance::default();
        inst.init(enc);
        inst
    }

    unsafe fn encin(input: &[u8], encoding: &mut VTermEncodingInstance) {
        let len = input.len();
        let mut cp = vec![0u32; len.max(1)];
        let mut cpi: c_int = 0;
        let mut pos: usize = 0;
        encoding.decode(&mut cp, &mut cpi, input, &mut pos);
        clear_test_output();
        if cpi > 0 {
            let mut out = String::from("encout ");
            for (i, &val) in cp[..(cpi as usize)].iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                use std::fmt::Write as _;
                let _ = write!(out, "{val:x}");
            }
            out.push('\n');
            append_test_output(out.as_bytes());
        }
    }

    unsafe fn mousemove(row: c_int, col: c_int, vt: *mut VTerm, mod_: VTermModifier) {
        unsafe {
            vterm_mouse_move(vt, row, col, mod_);
        }
    }

    unsafe fn mousebtn(press: u8, button: c_int, vt: *mut VTerm, mod_: VTermModifier) {
        unsafe {
            vterm_mouse_button(vt, button, press == b'd' || press == b'D', mod_);
        }
    }

    unsafe fn inchar(c: u32, vt: *mut VTerm, mod_: VTermModifier) {
        unsafe {
            vterm_keyboard_unichar(vt, c, mod_);
        }
    }

    unsafe fn inkey(input_key: &str, vt: *mut VTerm, mod_: VTermModifier) {
        const VTERM_KEY_ESCAPE: VTermKey = 4;
        const VTERM_KEY_UP: VTermKey = 5;
        const VTERM_KEY_DEL: VTermKey = 10;
        let key = match input_key {
            "up" => VTERM_KEY_UP,
            "tab" => VTERM_KEY_TAB,
            "enter" => VTERM_KEY_ENTER,
            "bs" => VTERM_KEY_BACKSPACE,
            "del" => VTERM_KEY_DEL,
            "esc" => VTERM_KEY_ESCAPE,
            "f1" => VTERM_KEY_FUNCTION_0 + 1,
            "kp0" => VTERM_KEY_KP_0,
            _ => VTERM_KEY_NONE,
        };
        unsafe {
            vterm_keyboard_key(vt, key, mod_);
        }
    }

    #[test]
    fn vterm_color_is_equal_compares_rgb_and_indexed_colors() {
        unsafe {
            let mut c1 = VTermColor { bytes: [0; 4] };
            let mut c2 = VTermColor { bytes: [0; 4] };
            c1.set_rgb(10, 20, 30);
            c2.set_rgb(10, 20, 30);
            assert_eq!(vterm_color_is_equal(&raw const c1, &raw const c2), 1);
            c2.set_rgb(10, 20, 31);
            assert_eq!(vterm_color_is_equal(&raw const c1, &raw const c2), 0);
        }
    }

    #[test]
    fn spec_02parser() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            vterm_set_utf8(vt, 0);
            wantparser(vt);
            push(b"hello", vt);
            expect(b"text 68,65,6c,6c,6f");
            push(b"\x03", vt);
            expect(b"control 03");
            push(b"\x1f", vt);
            expect(b"control 1f");
            push(b"\x83", vt);
            expect(b"control 83");
            push(b"\x99", vt);
            expect(b"control 99");
            push(b"\x1bC", vt);
            expect(b"control 83");
            push(b"\x1bY", vt);
            expect(b"control 99");
            push(b"\xa0\xcc\xfe", vt);
            expect(b"text a0,cc,fe");
            push(b"1\n2", vt);
            expect(b"text 31\ncontrol 0a\ntext 32");
            push(b"\x1b=", vt);
            expect(b"escape =");
            push(b"\x1b(X", vt);
            expect(b"escape (X");
            push(b"\x1b(", vt);
            push(b"Y", vt);
            expect(b"escape (Y");
            push(b"\x1b(\x1b)Z", vt);
            expect(b"escape )Z");
            push(b"\x1b(\x18AB", vt);
            expect(b"text 41,42");
            push(b"\x1b(\nX", vt);
            expect(b"control 0a\nescape (X");
            push(b"\x1b[a", vt);
            expect(b"csi 61 *");
            push(b"\x1b[9b", vt);
            expect(b"csi 62 9");
            push(b"\x1b[3;4c", vt);
            expect(b"csi 63 3,4");
            push(b"\x1b[1:2c", vt);
            expect(b"csi 63 1+,2");
            push(b"\x1b[678d", vt);
            expect(b"csi 64 678");
            push(b"\x1b[007e", vt);
            expect(b"csi 65 7");
            push(b"\x1b[?2;7f", vt);
            expect(b"csi 66 L=3f 2,7");
            push(b"\x1b[>c", vt);
            expect(b"csi 63 L=3e *");
            push(b"\x1b[12 q", vt);
            expect(b"csi 71 12 I=20");
            push(b"A\x1b[8mB", vt);
            expect(b"text 41\ncsi 6d 8\ntext 42");
            push(b"\x1b", vt);
            push(b"[a", vt);
            expect(b"csi 61 *");
            push(b"foo\x1b[", vt);
            expect(b"text 66,6f,6f");
            push(b"4b", vt);
            expect(b"csi 62 4");
            push(b"\x1b[12;", vt);
            push(b"3c", vt);
            expect(b"csi 63 12,3");
            push(b"\x1b[123\x1b9", vt);
            expect(b"escape 9");
            push(b"\x1b[12\x18AB", vt);
            expect(b"text 41,42");
            push(b"\x1b(\nX", vt);
            expect(b"control 0a\nescape (X");
            push(b"\x1b]1;Hello\x07", vt);
            expect(b"osc [1;Hello]");
            push(b"\x1b]1;Hello\x1b\\", vt);
            expect(b"osc [1;Hello]");
            push(b"\x9d1;Hello\x9c", vt);
            expect(b"osc [1;Hello]");
            push(b"\x1b]52;abc", vt);
            expect(b"osc [52;abc");
            push(b"def", vt);
            expect(b"osc def");
            push(b"ghi\x1b\\", vt);
            expect(b"osc ghi]");
            push(b"\x1b]1234\x07", vt);
            expect(b"osc [1234;]");
            push(b"\x1b]1234\x1b\\", vt);
            expect(b"osc [1234;]");
            push(b"\x1b]Something\x1b9", vt);
            expect(b"escape 9");
            push(b"\x1b]12\x18AB", vt);
            expect(b"text 41,42");
            push(b"\x1b]2;\nBye\x07", vt);
            expect(b"osc [2;\ncontrol 0a\nosc Bye]");
            push(b"\x1bPHello\x07", vt);
            expect(b"dcs [Hello]");
            push(b"\x1bPHello\x1b\\", vt);
            expect(b"dcs [Hello]");
            push(b"\x90Hello\x9c", vt);
            expect(b"dcs [Hello]");
            push(b"\x1bPABC\x1b", vt);
            expect(b"dcs [ABC");
            push(b"\\", vt);
            expect(b"dcs ]");
            push(b"\x1bPSomething\x1b9", vt);
            expect(b"escape 9");
            push(b"\x1bP12\x18AB", vt);
            expect(b"text 41,42");
            push(b"\x1bPBy\ne\x07", vt);
            expect(b"dcs [By\ncontrol 0a\ndcs e]");
            push(b"\x1b_Hello\x07", vt);
            expect(b"apc [Hello]");
            push(b"\x1b_Hello\x1b\\", vt);
            expect(b"apc [Hello]");
            push(b"\x9fHello\x9c", vt);
            expect(b"apc [Hello]");
            push(b"\x1b^Hello\x07", vt);
            expect(b"pm [Hello]");
            push(b"\x1b^Hello\x1b\\", vt);
            expect(b"pm [Hello]");
            push(b"\x9eHello\x9c", vt);
            expect(b"pm [Hello]");
            push(b"\x1bXHello\x07", vt);
            expect(b"sos [Hello]");
            push(b"\x1bXHello\x1b\\", vt);
            expect(b"sos [Hello]");
            push(b"\x98Hello\x9c", vt);
            expect(b"sos [Hello]");
            push(b"\x1bXABC\x01DEF\x1b\\", vt);
            expect(b"sos [ABC\x01DEF]");
            push(b"\x1bXABC\x99DEF\x1b\\", vt);
            expect(b"sos [ABC\x99DEF]");
            push(b"\x00", vt);
            push(b"\x1b[12\x003m", vt);
            expect(b"csi 6d 123");
            push(b"\x7f", vt);
            push(b"\x1b[12\x7f3m", vt);
            expect(b"csi 6d 123");
            push(b"AB\x7fC", vt);
            expect(b"text 41,42\ntext 43");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_03encoding_utf8() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let mut encoding = wantencoding();
            encin(b"123", &mut encoding);
            expect(b"encout 31,32,33");
            encin(b"\xc2\x80\xdf\xbf", &mut encoding);
            expect(b"encout 80,7ff");
            encin(b"\xe0\xa0\x80\xef\xbf\xbd", &mut encoding);
            expect(b"encout 800,fffd");
            encin(b"\xf0\x90\x80\x80\xf7\xbf\xbf\xbf", &mut encoding);
            expect(b"encout 10000,1fffff");
            encin(b"\xc2!", &mut encoding);
            expect(b"encout fffd,21");
            encin(b"\xe0!\xe0\xa0!", &mut encoding);
            expect(b"encout fffd,21,fffd,21");
            encin(b"\xf0!\xf0\x90!\xf0\x90\x80!", &mut encoding);
            expect(b"encout fffd,21,fffd,21,fffd,21");
            encin(b"\xc2\xc2\x90", &mut encoding);
            expect(b"encout fffd,90");
            encin(b"\xe0\xc2\x90\xe0\xa0\xc2\x90", &mut encoding);
            expect(b"encout fffd,90,fffd,90");
            encin(
                b"\xf0\xc2\x90\xf0\x90\xc2\x90\xf0\x90\x80\xc2\x90",
                &mut encoding,
            );
            expect(b"encout fffd,90,fffd,90,fffd,90");
            encin(b"\xc0\x80\xc1\xbf", &mut encoding);
            expect(b"encout fffd,fffd");
            encin(b"\xe0\x80\x80\xe0\x9f\xbf", &mut encoding);
            expect(b"encout fffd,fffd");
            encin(b"\xf0\x80\x80\x80\xf0\x8f\xbf\xbf", &mut encoding);
            expect(b"encout fffd,fffd");
            encin(b"\xed\xa0\x80\xed\xbf\xbf", &mut encoding);
            expect(b"encout fffd,fffd");
            encin(b"\xc2", &mut encoding);
            encin(b"\xa0", &mut encoding);
            expect(b"encout a0");
            encin(b"\xe0", &mut encoding);
            encin(b"\xa0\x80", &mut encoding);
            expect(b"encout 800");
            encin(b"\xe0\xa0", &mut encoding);
            encin(b"\x80", &mut encoding);
            expect(b"encout 800");
            encin(b"\xf0", &mut encoding);
            encin(b"\x90\x80\x80", &mut encoding);
            expect(b"encout 10000");
            encin(b"\xf0\x90", &mut encoding);
            encin(b"\x80\x80", &mut encoding);
            expect(b"encout 10000");
            encin(b"\xf0\x90\x80", &mut encoding);
            encin(b"\x80", &mut encoding);
            expect(b"encout 10000");
        }
    }

    #[test]
    fn spec_10state_putglyph() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: true,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            push(b"ABC", vt);
            expect(b"putglyph 41 1 0,0\nputglyph 42 1 0,1\nputglyph 43 1 0,2");
            reset(state, ptr::null_mut());
            push(b"\xc3\x81\xc3\xa9", vt);
            expect(b"putglyph c1 1 0,0\nputglyph e9 1 0,1");
            reset(state, ptr::null_mut());
            push(b"\xc3", vt);
            push(b"\x81", vt);
            expect(b"putglyph c1 1 0,0");
            reset(state, ptr::null_mut());
            push(b"\xef\xbc\x90 ", vt);
            expect(b"putglyph ff10 2 0,0\nputglyph 20 1 0,2");
            reset(state, ptr::null_mut());
            push(b"\xf0\x9f\x98\x80 ", vt);
            expect(b"putglyph 1f600 2 0,0\nputglyph 20 1 0,2");
            reset(state, ptr::null_mut());
            push(b"e\xcc\x81Z", vt);
            expect(b"putglyph 65,301 1 0,0\nputglyph 5a 1 0,1");
            reset(state, ptr::null_mut());
            push(b"e", vt);
            expect(b"putglyph 65 1 0,0");
            push(b"\xcc\x81Z", vt);
            expect(b"putglyph 65,301 1 0,0\nputglyph 5a 1 0,1");
            reset(state, ptr::null_mut());
            push(b"e\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81", vt);
            expect(b"putglyph 65,301,301,301,301,301,301,301,301,301,301,301,301,301,301 1 0,0");
            reset(state, ptr::null_mut());
            push(b"e", vt);
            expect(b"putglyph 65 1 0,0");
            push(b"\xcc\x81", vt);
            expect(b"putglyph 65,301 1 0,0");
            push(b"\xcc\x82", vt);
            expect(b"putglyph 65,301,302 1 0,0");
            reset(state, ptr::null_mut());
            push(b"\x1b[5;80H", vt);
            push(b"e", vt);
            expect(b"putglyph 65 1 4,79");
            push(b"\xcc\x81", vt);
            expect(b"putglyph 65,301 1 4,79");
            push(b"\xcc\x82Z", vt);
            expect(b"putglyph 65,301,302 1 4,79\nputglyph 5a 1 5,0");
            reset(state, ptr::null_mut());
            push(b"\x1b[5;77H", vt);
            push(b"\xf0\x9f\x87\xa6", vt);
            expect(b"putglyph 1f1e6 2 4,76");
            push(b"\xf0\x9f\x87\xa9", vt);
            expect(b"putglyph 1f1e6,1f1e9 2 4,76");
            push(b"\xf0\x9f\x87\xb1", vt);
            expect(b"putglyph 1f1f1 2 4,78");
            push(b"\xf0\x9f\x87\xae", vt);
            expect(b"putglyph 1f1f1,1f1ee 2 4,78");
            push(b"\xf0\x9f\x87\xb2", vt);
            expect(b"putglyph 1f1f2 2 5,0");
            push(b"\xf0\x9f\x87\xa8", vt);
            expect(b"putglyph 1f1f2,1f1e8 2 5,0");
            reset(state, ptr::null_mut());
            push(b"\xf0\x9f\x8f\xb3\xef\xb8\x8f\xe2\x80\x8d\xf0\x9f\x8c\x88\xf0\x9f\x8f\xb3\xef\xb8\x8f\xe2\x80\x8d\xe2\x9a\xa7\xef\xb8\x8f\xf0\x9f\x8f\xb4\xe2\x80\x8d\xe2\x98\xa0\xef\xb8\x8f", vt);
            expect(b"putglyph 1f3f3,fe0f,200d,1f308 2 0,0\nputglyph 1f3f3,fe0f,200d,26a7,fe0f 2 0,2\nputglyph 1f3f4,200d,2620,fe0f 2 0,4");
            reset(state, ptr::null_mut());
            push(b"\xf0\x9f\x8f\xb3", vt);
            expect(b"putglyph 1f3f3 2 0,0");
            push(b"\xef\xb8\x8f", vt);
            expect(b"putglyph 1f3f3,fe0f 2 0,0");
            push(b"\xe2\x80\x8d", vt);
            expect(b"putglyph 1f3f3,fe0f,200d 2 0,0");
            push(b"\xf0\x9f\x8c\x88", vt);
            expect(b"putglyph 1f3f3,fe0f,200d,1f308 2 0,0");
            push(b"\xe2\x9d\xa4", vt);
            expect(b"putglyph 2764 1 0,2");
            push(b"\xef\xb8\x8f", vt);
            expect(b"putglyph 2764,fe0f 2 0,2");
            push(b"\xe2\x9d\xa4\xef\xb8\x8f", vt);
            expect(b"putglyph 2764,fe0f 2 0,4");
            reset(state, ptr::null_mut());
            push(b"A\x1b[1\"qB\x1b[2\"qC", vt);
            expect(b"putglyph 41 1 0,0\nputglyph 42 1 0,1 prot\nputglyph 43 1 0,2");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_11state_movecursor() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            push(b"ABC", vt);
            cursor(0, 3, state);
            push(b"\x08", vt);
            cursor(0, 2, state);
            push(b"\t", vt);
            cursor(0, 8, state);
            push(b"\r", vt);
            cursor(0, 0, state);
            push(b"\n", vt);
            cursor(1, 0, state);
            push(b"\x1b[4;2H", vt);
            cursor(3, 1, state);
            push(b"\x08", vt);
            cursor(3, 0, state);
            push(b"\x08", vt);
            cursor(3, 0, state);
            push(b"\x1b[4;80H", vt);
            cursor(3, 79, state);
            push(b"X", vt);
            cursor(3, 79, state);
            push(b"\x08", vt);
            cursor(3, 78, state);
            push(b"\x1b[1;78H", vt);
            cursor(0, 77, state);
            push(b"\t", vt);
            cursor(0, 79, state);
            push(b"\t", vt);
            cursor(0, 79, state);
            reset(state, ptr::null_mut());
            push(b"ABC\x1bD", vt);
            cursor(1, 3, state);
            push(b"\x1bM", vt);
            cursor(0, 3, state);
            push(b"\x1bE", vt);
            cursor(1, 0, state);
            reset(state, ptr::null_mut());
            push(b"\x1b[B", vt);
            cursor(1, 0, state);
            push(b"\x1b[3B", vt);
            cursor(4, 0, state);
            push(b"\x1b[0B", vt);
            cursor(5, 0, state);
            push(b"\x1b[C", vt);
            cursor(5, 1, state);
            push(b"\x1b[3C", vt);
            cursor(5, 4, state);
            push(b"\x1b[0C", vt);
            cursor(5, 5, state);
            push(b"\x1b[A", vt);
            cursor(4, 5, state);
            push(b"\x1b[3A", vt);
            cursor(1, 5, state);
            push(b"\x1b[0A", vt);
            cursor(0, 5, state);
            push(b"\x1b[D", vt);
            cursor(0, 4, state);
            push(b"\x1b[3D", vt);
            cursor(0, 1, state);
            push(b"\x1b[0D", vt);
            cursor(0, 0, state);
            push(b"   ", vt);
            cursor(0, 3, state);
            push(b"\x1b[E", vt);
            cursor(1, 0, state);
            push(b"   ", vt);
            cursor(1, 3, state);
            push(b"\x1b[2E", vt);
            cursor(3, 0, state);
            push(b"\x1b[0E", vt);
            cursor(4, 0, state);
            push(b"   ", vt);
            cursor(4, 3, state);
            push(b"\x1b[F", vt);
            cursor(3, 0, state);
            push(b"   ", vt);
            cursor(3, 3, state);
            push(b"\x1b[2F", vt);
            cursor(1, 0, state);
            push(b"\x1b[0F", vt);
            cursor(0, 0, state);
            push(b"\n", vt);
            cursor(1, 0, state);
            push(b"\x1b[20G", vt);
            cursor(1, 19, state);
            push(b"\x1b[G", vt);
            cursor(1, 0, state);
            push(b"\x1b[10;5H", vt);
            cursor(9, 4, state);
            push(b"\x1b[8H", vt);
            cursor(7, 0, state);
            push(b"\x1b[H", vt);
            cursor(0, 0, state);
            push(b"\x1b[10;78H", vt);
            cursor(9, 77, state);
            push(b"ABC", vt);
            cursor(9, 79, state);
            push(b"\x1b[10;80H", vt);
            push(b"C", vt);
            cursor(9, 79, state);
            push(b"X", vt);
            cursor(10, 1, state);
            reset(state, ptr::null_mut());
            push(b"\x1b[A", vt);
            cursor(0, 0, state);
            push(b"\x1b[D", vt);
            cursor(0, 0, state);
            push(b"\x1b[25;80H", vt);
            cursor(24, 79, state);
            push(b"\x1b[B", vt);
            cursor(24, 79, state);
            push(b"\x1b[C", vt);
            cursor(24, 79, state);
            push(b"\x1b[E", vt);
            cursor(24, 0, state);
            push(b"\x1b[H", vt);
            cursor(0, 0, state);
            push(b"\x1b[F", vt);
            cursor(0, 0, state);
            push(b"\x1b[999G", vt);
            cursor(0, 79, state);
            push(b"\x1b[99;99H", vt);
            cursor(24, 79, state);
            reset(state, ptr::null_mut());
            push(b"\x1b[5`", vt);
            cursor(0, 4, state);
            push(b"\x1b[3a", vt);
            cursor(0, 7, state);
            push(b"\x1b[3j", vt);
            cursor(0, 4, state);
            push(b"\x1b[3;3f", vt);
            cursor(2, 2, state);
            push(b"\x1b[5d", vt);
            cursor(4, 2, state);
            push(b"\x1b[2e", vt);
            cursor(6, 2, state);
            push(b"\x1b[2k", vt);
            cursor(4, 2, state);
            reset(state, ptr::null_mut());
            push(b"\t", vt);
            cursor(0, 8, state);
            push(b"   ", vt);
            cursor(0, 11, state);
            push(b"\t", vt);
            cursor(0, 16, state);
            push(b"       ", vt);
            cursor(0, 23, state);
            push(b"\t", vt);
            cursor(0, 24, state);
            push(b"        ", vt);
            cursor(0, 32, state);
            push(b"\t", vt);
            cursor(0, 40, state);
            push(b"\x1b[I", vt);
            cursor(0, 48, state);
            push(b"\x1b[2I", vt);
            cursor(0, 64, state);
            push(b"\x1b[Z", vt);
            cursor(0, 56, state);
            push(b"\x1b[2Z", vt);
            cursor(0, 40, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_12state_scroll() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: false,
                    p: false,
                    s: true,
                },
            );
            push(b"\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n", vt);
            cursor(24, 0, state);
            push(b"\n", vt);
            expect(b"scrollrect 0..25,0..80 => +1,+0");
            cursor(24, 0, state);
            reset(state, ptr::null_mut());
            push(b"\x1b[25H", vt);
            push(b"\x1bD", vt);
            expect(b"scrollrect 0..25,0..80 => +1,+0");
            reset(state, ptr::null_mut());
            push(b"\x1bM", vt);
            expect(b"scrollrect 0..25,0..80 => -1,+0");
            reset(state, ptr::null_mut());
            push(b"\x1b[1;10r", vt);
            cursor(0, 0, state);
            push(b"\n\n\n\n\n\n\n\n\n", vt);
            cursor(9, 0, state);
            push(b"\n", vt);
            expect(b"scrollrect 0..10,0..80 => +1,+0");
            cursor(9, 0, state);
            push(b"\x1b[20H", vt);
            cursor(19, 0, state);
            push(b"\n", vt);
            cursor(20, 0, state);
            push(b"\x1b[9;10r", vt);
            push(b"\x1b[10H", vt);
            push(b"\x1bM", vt);
            cursor(8, 0, state);
            push(b"\x1bM", vt);
            expect(b"scrollrect 8..10,0..80 => -1,+0");
            push(b"\x1b[25H", vt);
            cursor(24, 0, state);
            push(b"\n", vt);
            cursor(24, 0, state);
            push(b"\x1b[?69h", vt);
            push(b"\x1b[3;10r\x1b[10;40s", vt);
            push(b"\x1b[10;10H\n", vt);
            expect(b"scrollrect 2..10,9..40 => +1,+0");
            push(b"\x1bD", vt);
            expect(b"scrollrect 2..10,9..40 => +1,+0");
            push(b"\x1b[3;10H\x1bM", vt);
            expect(b"scrollrect 2..10,9..40 => -1,+0");
            push(b"\x1bP$qr\x1b\\", vt);
            expect_output(b"\x1bP1$r3;10r\x1b\\");
            push(b"\x1bP$qs\x1b\\", vt);
            expect_output(b"\x1bP1$r10;40s\x1b\\");
            push(b"\x1b[?69l\x1b[;0s\x1b[?69h", vt);
            reset(state, ptr::null_mut());
            push(b"\x1b[S", vt);
            expect(b"scrollrect 0..25,0..80 => +1,+0");
            cursor(0, 0, state);
            push(b"\x1b[2S", vt);
            expect(b"scrollrect 0..25,0..80 => +2,+0");
            cursor(0, 0, state);
            push(b"\x1b[100S", vt);
            expect(b"scrollrect 0..25,0..80 => +25,+0");
            push(b"\x1b[T", vt);
            expect(b"scrollrect 0..25,0..80 => -1,+0");
            cursor(0, 0, state);
            push(b"\x1b[2T", vt);
            expect(b"scrollrect 0..25,0..80 => -2,+0");
            cursor(0, 0, state);
            push(b"\x1b[100T", vt);
            expect(b"scrollrect 0..25,0..80 => -25,+0");
            push(b"\x1b[5;20r", vt);
            push(b"\x1b[S", vt);
            expect(b"scrollrect 4..20,0..80 => +1,+0");
            push(b"\x1b[T", vt);
            expect(b"scrollrect 4..20,0..80 => -1,+0");
            reset(state, ptr::null_mut());
            push(b"\x1b[?69h", vt);
            push(b"\x1b[3;10r\x1b[10;40s", vt);
            cursor(0, 0, state);
            push(b"\x1b[3;10H", vt);
            cursor(2, 9, state);
            push(b"\x1b[S", vt);
            expect(b"scrollrect 2..10,9..40 => +1,+0");
            push(b"\x1b[?69l", vt);
            push(b"\x1b[S", vt);
            expect(b"scrollrect 2..10,0..80 => +1,+0");
            reset(state, ptr::null_mut());
            push(b"\x1b[100;105r\x1bD", vt);
            push(b"\x1b[5;2r\x1bD", vt);
            reset(state, ptr::null_mut());
            let _ = state;
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: true,
                    f: false,
                    g: false,
                    m: true,
                    p: false,
                    s: false,
                },
            );
            push(b"\x1b[S", vt);
            expect(b"moverect 1..25,0..80 -> 0..24,0..80\nerase 24..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[2S", vt);
            expect(b"moverect 2..25,0..80 -> 0..23,0..80\nerase 23..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[T", vt);
            expect(b"moverect 0..24,0..80 -> 1..25,0..80\nerase 0..1,0..80");
            cursor(0, 0, state);
            push(b"\x1b[2T", vt);
            expect(b"moverect 0..23,0..80 -> 2..25,0..80\nerase 0..2,0..80");
            cursor(0, 0, state);
            push(b"\x1b[5;5H", vt);
            cursor(4, 4, state);
            push(b"\x1b[r", vt);
            cursor(0, 0, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_13state_edit() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: true,
                    e: true,
                    f: false,
                    g: false,
                    m: false,
                    p: false,
                    s: true,
                },
            );
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ACD", vt);
            push(b"\x1b[2D", vt);
            cursor(0, 1, state);
            push(b"\x1b[@", vt);
            expect(b"scrollrect 0..1,1..80 => +0,-1");
            cursor(0, 1, state);
            push(b"B", vt);
            cursor(0, 2, state);
            push(b"\x1b[3@", vt);
            expect(b"scrollrect 0..1,2..80 => +0,-3");
            push(b"\x1b[?69h", vt);
            push(b"\x1b[;50s", vt);
            push(b"\x1b[20G\x1b[@", vt);
            expect(b"scrollrect 0..1,19..50 => +0,-1");
            push(b"\x1b[70G\x1b[@", vt);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ABBC", vt);
            push(b"\x1b[3D", vt);
            cursor(0, 1, state);
            push(b"\x1b[P", vt);
            expect(b"scrollrect 0..1,1..80 => +0,+1");
            cursor(0, 1, state);
            push(b"\x1b[3P", vt);
            expect(b"scrollrect 0..1,1..80 => +0,+3");
            cursor(0, 1, state);
            push(b"\x1b[?69h", vt);
            push(b"\x1b[;50s", vt);
            push(b"\x1b[20G\x1b[P", vt);
            expect(b"scrollrect 0..1,19..50 => +0,+1");
            push(b"\x1b[70G\x1b[P", vt);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ABC", vt);
            push(b"\x1b[2D", vt);
            cursor(0, 1, state);
            push(b"\x1b[X", vt);
            expect(b"erase 0..1,1..2");
            cursor(0, 1, state);
            push(b"\x1b[3X", vt);
            expect(b"erase 0..1,1..4");
            cursor(0, 1, state);
            push(b"\x1b[100X", vt);
            expect(b"erase 0..1,1..80");
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"A\r\nC", vt);
            cursor(1, 1, state);
            push(b"\x1b[L", vt);
            expect(b"scrollrect 1..25,0..80 => -1,+0");
            cursor(1, 1, state);
            push(b"\rB", vt);
            cursor(1, 1, state);
            push(b"\x1b[3L", vt);
            expect(b"scrollrect 1..25,0..80 => -3,+0");
            push(b"\x1b[5;15r", vt);
            push(b"\x1b[5H\x1b[L", vt);
            expect(b"scrollrect 4..15,0..80 => -1,+0");
            push(b"\x1b[20H\x1b[L", vt);
            push(b"\x1b[?69h", vt);
            push(b"\x1b[10;50s", vt);
            push(b"\x1b[5;10H\x1b[L", vt);
            expect(b"scrollrect 4..15,9..50 => -1,+0");
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"A\r\nB\r\nB\r\nC", vt);
            cursor(3, 1, state);
            push(b"\x1b[2H", vt);
            cursor(1, 0, state);
            push(b"\x1b[M", vt);
            expect(b"scrollrect 1..25,0..80 => +1,+0");
            cursor(1, 0, state);
            push(b"\x1b[3M", vt);
            expect(b"scrollrect 1..25,0..80 => +3,+0");
            cursor(1, 0, state);
            push(b"\x1b[5;15r", vt);
            push(b"\x1b[5H\x1b[M", vt);
            expect(b"scrollrect 4..15,0..80 => +1,+0");
            push(b"\x1b[20H\x1b[M", vt);
            push(b"\x1b[?69h", vt);
            push(b"\x1b[10;50s", vt);
            push(b"\x1b[5;10H\x1b[M", vt);
            expect(b"scrollrect 4..15,9..50 => +1,+0");
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            push(b"\x1b[20G\x1b[5'}", vt);
            expect(b"scrollrect 0..25,19..80 => +0,-5");
            push(b"\x1b[?69h", vt);
            push(b"\x1b[4;20r\x1b[20;60s", vt);
            push(b"\x1b[4;20H\x1b[3'}", vt);
            expect(b"scrollrect 3..20,19..60 => +0,-3");
            push(b"\x1b[70G\x1b['}", vt);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            push(b"\x1b[20G\x1b[5'~", vt);
            expect(b"scrollrect 0..25,19..80 => +0,+5");
            push(b"\x1b[?69h", vt);
            push(b"\x1b[4;20r\x1b[20;60s", vt);
            push(b"\x1b[4;20H\x1b[3'~", vt);
            expect(b"scrollrect 3..20,19..60 => +0,+3");
            push(b"\x1b[70G\x1b['~", vt);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ABCDE", vt);
            push(b"\x1b[3D", vt);
            cursor(0, 2, state);
            push(b"\x1b[0K", vt);
            expect(b"erase 0..1,2..80");
            cursor(0, 2, state);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ABCDE", vt);
            push(b"\x1b[3D", vt);
            cursor(0, 2, state);
            push(b"\x1b[1K", vt);
            expect(b"erase 0..1,0..3");
            cursor(0, 2, state);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ABCDE", vt);
            push(b"\x1b[3D", vt);
            cursor(0, 2, state);
            push(b"\x1b[2K", vt);
            expect(b"erase 0..1,0..80");
            cursor(0, 2, state);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[11G", vt);
            cursor(0, 10, state);
            push(b"\x1b[?0K", vt);
            expect(b"erase 0..1,10..80 selective");
            cursor(0, 10, state);
            push(b"\x1b[?1K", vt);
            expect(b"erase 0..1,0..11 selective");
            cursor(0, 10, state);
            push(b"\x1b[?2K", vt);
            expect(b"erase 0..1,0..80 selective");
            cursor(0, 10, state);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[2;2H", vt);
            cursor(1, 1, state);
            push(b"\x1b[0J", vt);
            expect(b"erase 1..2,1..80\nerase 2..25,0..80");
            cursor(1, 1, state);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[2;2H", vt);
            cursor(1, 1, state);
            push(b"\x1b[1J", vt);
            expect(b"erase 0..1,0..80\nerase 1..2,0..2");
            cursor(1, 1, state);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[2;2H", vt);
            cursor(1, 1, state);
            push(b"\x1b[2J", vt);
            expect(b"erase 0..25,0..80");
            cursor(1, 1, state);
            push(b"\x1b[3J", vt);
            expect(b"sb_clear");
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            push(b"\x1b[5;5H", vt);
            cursor(4, 4, state);
            push(b"\x1b[?0J", vt);
            expect(b"erase 4..5,4..80 selective\nerase 5..25,0..80 selective");
            cursor(4, 4, state);
            push(b"\x1b[?1J", vt);
            expect(b"erase 0..4,0..80 selective\nerase 4..5,0..5 selective");
            cursor(4, 4, state);
            push(b"\x1b[?2J", vt);
            expect(b"erase 0..25,0..80 selective");
            cursor(4, 4, state);
            push(b"\x1b[2\"q", vt);
            push(b"\x1bP$q\"q\x1b\\", vt);
            expect_output(b"\x1bP1$r2\"q\x1b\\");
            let _ = state;
            let state = wantstate(
                vt,
                StateOpts {
                    b: true,
                    e: true,
                    f: false,
                    g: false,
                    m: true,
                    p: false,
                    s: false,
                },
            );
            expect(b"erase 0..25,0..80");
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ACD", vt);
            push(b"\x1b[2D", vt);
            cursor(0, 1, state);
            push(b"\x1b[@", vt);
            expect(b"moverect 0..1,1..79 -> 0..1,2..80\nerase 0..1,1..2");
            cursor(0, 1, state);
            push(b"B", vt);
            cursor(0, 2, state);
            push(b"\x1b[3@", vt);
            expect(b"moverect 0..1,2..77 -> 0..1,5..80\nerase 0..1,2..5");
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"ABBC", vt);
            push(b"\x1b[3D", vt);
            cursor(0, 1, state);
            push(b"\x1b[P", vt);
            expect(b"moverect 0..1,2..80 -> 0..1,1..79\nerase 0..1,79..80");
            cursor(0, 1, state);
            push(b"\x1b[3P", vt);
            expect(b"moverect 0..1,4..80 -> 0..1,1..77\nerase 0..1,77..80");
            cursor(0, 1, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_14state_encoding() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            vterm_set_utf8(vt, 0);
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: true,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            push(b"#", vt);
            expect(b"putglyph 23 1 0,0");
            reset(state, ptr::null_mut());
            push(b"\x1b(0", vt);
            push(b"a", vt);
            expect(b"putglyph 2592 1 0,0");
            reset(state, ptr::null_mut());
            push(b"\x1b)0", vt);
            push(b"a", vt);
            expect(b"putglyph 61 1 0,0");
            push(b"\x0e", vt);
            push(b"a", vt);
            expect(b"putglyph 2592 1 0,1");
            push(b"\x0f", vt);
            push(b"a", vt);
            expect(b"putglyph 61 1 0,2");
            push(b"\x1b*0", vt);
            push(b"a", vt);
            expect(b"putglyph 61 1 0,3");
            push(b"\x1bn", vt);
            push(b"a", vt);
            expect(b"putglyph 2592 1 0,4");
            push(b"\x0f", vt);
            push(b"a", vt);
            expect(b"putglyph 61 1 0,5");
            push(b"\x1b+0", vt);
            push(b"a", vt);
            expect(b"putglyph 61 1 0,6");
            push(b"\x1bo", vt);
            push(b"a", vt);
            expect(b"putglyph 2592 1 0,7");
            push(b"\x0f", vt);
            push(b"a", vt);
            expect(b"putglyph 61 1 0,8");
            push(b"a\x8eaa", vt);
            expect(b"putglyph 61 1 0,9\nputglyph 2592 1 0,10\nputglyph 61 1 0,11");
            push(b"a\x8faa", vt);
            expect(b"putglyph 61 1 0,12\nputglyph 2592 1 0,13\nputglyph 61 1 0,14");
            reset(state, ptr::null_mut());
            push(b"\x1b~", vt);
            push(b"\xe1", vt);
            expect(b"putglyph 61 1 0,0");
            push(b"\x1b)0", vt);
            push(b"\xe1", vt);
            expect(b"putglyph 2592 1 0,1");
            reset(state, ptr::null_mut());
            push(b"\x1b}", vt);
            push(b"\xe1", vt);
            expect(b"putglyph 61 1 0,0");
            push(b"\x1b*0", vt);
            push(b"\xe1", vt);
            expect(b"putglyph 2592 1 0,1");
            reset(state, ptr::null_mut());
            push(b"\x1b|", vt);
            push(b"\xe1", vt);
            expect(b"putglyph 61 1 0,0");
            push(b"\x1b+0", vt);
            push(b"\xe1", vt);
            expect(b"putglyph 2592 1 0,1");
            vterm_set_utf8(vt, 1);
            reset(state, ptr::null_mut());
            push(b"\x1b(B", vt);
            push(b"AB\xc4\x88D", vt);
            expect(b"putglyph 41 1 0,0\nputglyph 42 1 0,1\nputglyph 108 1 0,2\nputglyph 44 1 0,3");
            reset(state, ptr::null_mut());
            push(b"AB\xc4", vt);
            expect(b"putglyph 41 1 0,0\nputglyph 42 1 0,1");
            push(b"\x88D", vt);
            expect(b"putglyph 108 1 0,2\nputglyph 44 1 0,3");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_15state_mode() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: true,
                    f: false,
                    g: true,
                    m: true,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"AC\x1b[DB", vt);
            expect(b"putglyph 41 1 0,0\nputglyph 43 1 0,1\nputglyph 42 1 0,1");
            push(b"\x1b[4h", vt);
            push(b"\x1b[G", vt);
            push(b"AC\x1b[DB", vt);
            expect(b"moverect 0..1,0..79 -> 0..1,1..80\nerase 0..1,0..1\nputglyph 41 1 0,0\nmoverect 0..1,1..79 -> 0..1,2..80\nerase 0..1,1..2\nputglyph 43 1 0,1\nmoverect 0..1,1..79 -> 0..1,2..80\nerase 0..1,1..2\nputglyph 42 1 0,1");
            push(b"e", vt);
            expect(b"moverect 0..1,2..79 -> 0..1,3..80\nerase 0..1,2..3\nputglyph 65 1 0,2");
            push(b"\xcc\x81", vt);
            expect(b"putglyph 65,301 1 0,2");
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[5G\n", vt);
            cursor(1, 4, state);
            push(b"\x1b[20h", vt);
            push(b"\x1b[5G\n", vt);
            cursor(2, 0, state);
            reset(state, ptr::null_mut());
            expect(b"erase 0..25,0..80");
            cursor(0, 0, state);
            push(b"\x1b[5;15r", vt);
            push(b"\x1b[H", vt);
            cursor(0, 0, state);
            push(b"\x1b[3;3H", vt);
            cursor(2, 2, state);
            push(b"\x1b[?6h", vt);
            push(b"\x1b[H", vt);
            cursor(4, 0, state);
            push(b"\x1b[3;3H", vt);
            cursor(6, 2, state);
            push(b"\x1b[?6h", vt);
            push(b"\x1b[?6$p", vt);
            expect_output(b"\x1b[?6;1$y");
            push(b"\x1b[?6l", vt);
            push(b"\x1b[?6$p", vt);
            expect_output(b"\x1b[?6;2$y");
            push(b"\x1b[?6h", vt);
            push(b"\x1b[?69h", vt);
            push(b"\x1b[20;60s", vt);
            push(b"\x1b[H", vt);
            cursor(4, 19, state);
            push(b"\x1b[?69l", vt);
            push(b"\x1b[H", vt);
            push(b"\x1b[10A", vt);
            cursor(4, 0, state);
            push(b"\x1b[20B", vt);
            cursor(14, 0, state);
            push(b"\x1b[?6l", vt);
            push(b"\x1b[r\x1b[?6h", vt);
            cursor(0, 0, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_16state_resize() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: true,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            push(b"AB\x1b[79GCDE", vt);
            expect(b"putglyph 41 1 0,0\nputglyph 42 1 0,1\nputglyph 43 1 0,78\nputglyph 44 1 0,79\nputglyph 45 1 1,0");
            reset(state, ptr::null_mut());
            resize(27, 85, vt);
            push(b"AB\x1b[79GCDE", vt);
            expect(b"putglyph 41 1 0,0\nputglyph 42 1 0,1\nputglyph 43 1 0,78\nputglyph 44 1 0,79\nputglyph 45 1 0,80");
            cursor(0, 81, state);
            resize(28, 90, vt);
            cursor(0, 81, state);
            push(b"FGHI", vt);
            expect(
                b"putglyph 46 1 0,81\nputglyph 47 1 0,82\nputglyph 48 1 0,83\nputglyph 49 1 0,84",
            );
            cursor(0, 85, state);
            resize(25, 80, vt);
            cursor(0, 79, state);
            reset(state, ptr::null_mut());
            push(b"\x1b[79GAB", vt);
            expect(b"putglyph 41 1 0,78\nputglyph 42 1 0,79");
            cursor(0, 79, state);
            resize(30, 100, vt);
            cursor(0, 80, state);
            push(b"C", vt);
            expect(b"putglyph 43 1 0,80");
            cursor(0, 81, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_17state_mouse() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: false,
                    p: true,
                    s: false,
                },
            );
            push(b"\x1b[?1000$p", vt);
            expect_output(b"\x1b[?1000;2$y");
            push(b"\x1b[?1002$p", vt);
            expect_output(b"\x1b[?1002;2$y");
            push(b"\x1b[?1003$p", vt);
            expect_output(b"\x1b[?1003;2$y");
            reset(state, ptr::null_mut());
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            push(b"\x1b[?1000h", vt);
            expect(b"settermprop 8 1");
            mousemove(0, 0, vt, 0);
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[M !!");
            mousebtn(b'u', 1, vt, 0);
            expect_output(b"\x1b[M#!!");
            mousebtn(b'd', 1, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[M0!!");
            mousebtn(b'u', 1, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[M3!!");
            mousebtn(b'd', 2, vt, 0);
            expect_output(b"\x1b[M!!!");
            mousebtn(b'u', 2, vt, 0);
            expect_output(b"\x1b[M#!!");
            mousemove(10, 20, vt, 0);
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[M 5+");
            mousebtn(b'u', 1, vt, 0);
            expect_output(b"\x1b[M#5+");
            mousemove(10, 21, vt, 0);
            mousebtn(b'd', 4, vt, 0);
            expect_output(b"\x1b[M`6+");
            mousebtn(b'd', 4, vt, 0);
            expect_output(b"\x1b[M`6+");
            mousebtn(b'd', 5, vt, 0);
            expect_output(b"\x1b[Ma6+");
            mousebtn(b'd', 6, vt, 0);
            expect_output(b"\x1b[Mb6+");
            mousebtn(b'd', 7, vt, 0);
            expect_output(b"\x1b[Mc6+");
            push(b"\x1b[?1000$p", vt);
            expect_output(b"\x1b[?1000;1$y");
            push(b"\x1b[?1002$p", vt);
            expect_output(b"\x1b[?1002;2$y");
            push(b"\x1b[?1003$p", vt);
            expect_output(b"\x1b[?1003;2$y");
            reset(state, ptr::null_mut());
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            push(b"\x1b[?1002h", vt);
            expect(b"settermprop 8 2");
            mousemove(5, 5, vt, 0);
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[M &&");
            mousemove(5, 6, vt, 0);
            expect_output(b"\x1b[M@'&");
            mousemove(6, 6, vt, 0);
            expect_output(b"\x1b[M@''");
            mousemove(6, 6, vt, 0);
            mousebtn(b'u', 1, vt, 0);
            expect_output(b"\x1b[M#''");
            mousemove(6, 7, vt, 0);
            push(b"\x1b[?1000$p", vt);
            expect_output(b"\x1b[?1000;2$y");
            push(b"\x1b[?1002$p", vt);
            expect_output(b"\x1b[?1002;1$y");
            push(b"\x1b[?1003$p", vt);
            expect_output(b"\x1b[?1003;2$y");
            push(b"\x1b[?1003h", vt);
            expect(b"settermprop 8 3");
            mousemove(6, 8, vt, 0);
            expect_output(b"\x1b[MC)'");
            push(b"\x1b[?1000$p", vt);
            expect_output(b"\x1b[?1000;2$y");
            push(b"\x1b[?1002$p", vt);
            expect_output(b"\x1b[?1002;2$y");
            push(b"\x1b[?1003$p", vt);
            expect_output(b"\x1b[?1003;1$y");
            mousemove(300, 300, vt, 0);
            expect_output(b"\x1b[MC\xff\xff");
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[M \xff\xff");
            mousebtn(b'u', 1, vt, 0);
            expect_output(b"\x1b[M#\xff\xff");
            push(b"\x1b[?1005$p", vt);
            expect_output(b"\x1b[?1005;2$y");
            push(b"\x1b[?1006$p", vt);
            expect_output(b"\x1b[?1006;2$y");
            push(b"\x1b[?1015$p", vt);
            expect_output(b"\x1b[?1015;2$y");
            push(b"\x1b[?1005h", vt);
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[M \xc5\x8d\xc5\x8d");
            mousebtn(b'u', 1, vt, 0);
            expect_output(b"\x1b[M#\xc5\x8d\xc5\x8d");
            push(b"\x1b[?1005$p", vt);
            expect_output(b"\x1b[?1005;1$y");
            push(b"\x1b[?1006$p", vt);
            expect_output(b"\x1b[?1006;2$y");
            push(b"\x1b[?1015$p", vt);
            expect_output(b"\x1b[?1015;2$y");
            push(b"\x1b[?1006h", vt);
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[<0;301;301M");
            mousebtn(b'u', 1, vt, 0);
            expect_output(b"\x1b[<0;301;301m");
            mousebtn(b'd', 8, vt, 0);
            expect_output(b"\x1b[<128;301;301M");
            mousebtn(b'u', 8, vt, 0);
            expect_output(b"\x1b[<128;301;301m");
            mousebtn(b'd', 9, vt, 0);
            expect_output(b"\x1b[<129;301;301M");
            mousebtn(b'u', 9, vt, 0);
            expect_output(b"\x1b[<129;301;301m");
            push(b"\x1b[?1005$p", vt);
            expect_output(b"\x1b[?1005;2$y");
            push(b"\x1b[?1006$p", vt);
            expect_output(b"\x1b[?1006;1$y");
            push(b"\x1b[?1015$p", vt);
            expect_output(b"\x1b[?1015;2$y");
            push(b"\x1b[?1015h", vt);
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[0;301;301M");
            mousebtn(b'u', 1, vt, 0);
            expect_output(b"\x1b[3;301;301M");
            mousebtn(b'd', 8, vt, 0);
            expect_output(b"\x1b[128;301;301M");
            mousebtn(b'u', 8, vt, 0);
            expect_output(b"\x1b[3;301;301M");
            mousebtn(b'd', 9, vt, 0);
            expect_output(b"\x1b[129;301;301M");
            mousebtn(b'u', 9, vt, 0);
            expect_output(b"\x1b[3;301;301M");
            push(b"\x1b[?1005$p", vt);
            expect_output(b"\x1b[?1005;2$y");
            push(b"\x1b[?1006$p", vt);
            expect_output(b"\x1b[?1006;2$y");
            push(b"\x1b[?1015$p", vt);
            expect_output(b"\x1b[?1015;1$y");
            reset(state, ptr::null_mut());
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            mousemove(0, 0, vt, 0);
            mousebtn(b'd', 1, vt, 0);
            mousebtn(b'u', 1, vt, 0);
            push(b"\x1b[?1002;1006h", vt);
            expect(b"settermprop 8 2");
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[<0;1;1M");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_18state_termprops() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: false,
                    p: true,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            push(b"\x1b[?25h", vt);
            expect(b"settermprop 1 true");
            push(b"\x1b[?25$p", vt);
            expect_output(b"\x1b[?25;1$y");
            push(b"\x1b[?25l", vt);
            expect(b"settermprop 1 false");
            push(b"\x1b[?25$p", vt);
            expect_output(b"\x1b[?25;2$y");
            push(b"\x1b[?12h", vt);
            expect(b"settermprop 2 true");
            push(b"\x1b[?12$p", vt);
            expect_output(b"\x1b[?12;1$y");
            push(b"\x1b[?12l", vt);
            expect(b"settermprop 2 false");
            push(b"\x1b[?12$p", vt);
            expect_output(b"\x1b[?12;2$y");
            push(b"\x1b[3 q", vt);
            expect(b"settermprop 2 true\nsettermprop 7 2");
            push(b"\x1b]2;Here is my title\x07", vt);
            expect(b"settermprop 4 [\"Here is my title\"]");
            push(b"\x1b]2;Here is", vt);
            expect(b"settermprop 4 [\"Here is\"");
            push(b" another title\x07", vt);
            expect(b"settermprop 4 \" another title\"]");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_20state_wrapping() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: true,
                    m: true,
                    p: false,
                    s: false,
                },
            );
            push(b"\x1b[75G", vt);
            push(b"AAAAA", vt);
            expect(b"putglyph 41 1 0,74\nputglyph 41 1 0,75\nputglyph 41 1 0,76\nputglyph 41 1 0,77\nputglyph 41 1 0,78");
            cursor(0, 79, state);
            push(b"A", vt);
            expect(b"putglyph 41 1 0,79");
            cursor(0, 79, state);
            push(b"B", vt);
            expect(b"putglyph 42 1 1,0");
            cursor(1, 1, state);
            push(b"\x1b[78G", vt);
            push(b"BBBCC", vt);
            expect(b"putglyph 42 1 1,77\nputglyph 42 1 1,78\nputglyph 42 1 1,79\nputglyph 43 1 2,0\nputglyph 43 1 2,1");
            cursor(2, 2, state);
            reset(state, ptr::null_mut());
            push(b"\x1b[?7l", vt);
            push(b"\x1b[75G", vt);
            push(b"DDDDDD", vt);
            expect(b"putglyph 44 1 0,74\nputglyph 44 1 0,75\nputglyph 44 1 0,76\nputglyph 44 1 0,77\nputglyph 44 1 0,78\nputglyph 44 1 0,79");
            cursor(0, 79, state);
            push(b"D", vt);
            expect(b"putglyph 44 1 0,79");
            cursor(0, 79, state);
            push(b"\x1b[?7h", vt);
            push(b"\x1b[25;78HABC", vt);
            expect(b"putglyph 41 1 24,77\nputglyph 42 1 24,78\nputglyph 43 1 24,79");
            cursor(24, 79, state);
            push(b"D", vt);
            expect(b"moverect 1..25,0..80 -> 0..24,0..80\nputglyph 44 1 24,0");
            push(b"\x1b[25;78HABC", vt);
            expect(b"putglyph 41 1 24,77\nputglyph 42 1 24,78\nputglyph 43 1 24,79");
            cursor(24, 79, state);
            push(b"\x1b[25;1HD", vt);
            expect(b"putglyph 44 1 24,0");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_21state_tabstops() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: true,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,8");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,16");
            cursor(0, 17, state);
            push(b"\x1b[5G\x1bH", vt);
            push(b"\x1b[G\tX", vt);
            expect(b"putglyph 58 1 0,4");
            cursor(0, 5, state);
            push(b"\x1b[9G\x1b[g", vt);
            push(b"\x1b[G\tX\tX", vt);
            expect(b"putglyph 58 1 0,4\nputglyph 58 1 0,16");
            cursor(0, 17, state);
            push(b"\x1b[3g\x1b[50G\x1bH\x1b[G", vt);
            cursor(0, 0, state);
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,49");
            cursor(0, 50, state);
            reset(state, ptr::null_mut());
            resize(30, 100, vt);
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,8");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,16");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,24");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,32");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,40");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,48");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,56");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,64");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,72");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,80");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,88");
            push(b"\tX", vt);
            expect(b"putglyph 58 1 0,96");
            cursor(0, 97, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_22state_save() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: false,
                    p: true,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            push(b"\x1b[2;2H", vt);
            cursor(1, 1, state);
            push(b"\x1b[1m", vt);
            pen("bold", 1, state);
            push(b"\x1b[?1048h", vt);
            push(b"\x1b[5;5H", vt);
            cursor(4, 4, state);
            push(b"\x1b[4 q", vt);
            expect(b"settermprop 2 false\nsettermprop 7 2");
            push(b"\x1b[22;4m", vt);
            pen("bold", 0, state);
            pen("underline", 1, state);
            push(b"\x1b[?1048l", vt);
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            cursor(1, 1, state);
            pen("bold", 1, state);
            pen("underline", 0, state);
            push(b"\x1b[2;2H\x1b7", vt);
            cursor(1, 1, state);
            push(b"\x1b[5;5H", vt);
            cursor(4, 4, state);
            push(b"\x1b8", vt);
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            cursor(1, 1, state);
            push(b"\x1b[2;10H\x1b[?1048h\x1b[6;10H\x1b[?1048h", vt);
            push(b"\x1b[H", vt);
            cursor(0, 0, state);
            push(b"\x1b[?1048l", vt);
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            cursor(5, 9, state);
            push(b"\x1b[H", vt);
            cursor(0, 0, state);
            push(b"\x1b[?1048l", vt);
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            cursor(5, 9, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_25state_input() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            push(b"\x1b[>1u", vt);
            inchar(0x41, vt, 0);
            expect_output(b"A");
            inchar(0x61, vt, 0);
            expect_output(b"a");
            inchar(0x41, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[97;6u");
            inchar(0x61, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[97;5u");
            inchar(0x41, vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[97;4u");
            inchar(0x61, vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[97;3u");
            inchar(0x41, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[97;8u");
            inchar(0x61, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[97;7u");
            inchar(0x49, vt, 0);
            expect_output(b"I");
            inchar(0x69, vt, 0);
            expect_output(b"i");
            inchar(0x49, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[105;6u");
            inchar(0x69, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[105;5u");
            inchar(0x49, vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[105;4u");
            inchar(0x69, vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[105;3u");
            inchar(0x49, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[105;8u");
            inchar(0x69, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[105;7u");
            for i in 0..=9u32 {
                let c = 0x30 + i;
                inchar(c, vt, 0);
                expect_output(format!("{i}").as_bytes());
                inchar(c, vt, VTERM_MOD_CTRL);
                expect_output(format!("\x1b[{c};5u").as_bytes());
                inchar(c, vt, VTERM_MOD_CTRL | VTERM_MOD_SHIFT);
                expect_output(format!("\x1b[{c};6u").as_bytes());
                inchar(c, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
                expect_output(format!("\x1b[{c};7u").as_bytes());
                inchar(c, vt, VTERM_MOD_CTRL | VTERM_MOD_SHIFT | VTERM_MOD_ALT);
                expect_output(format!("\x1b[{c};8u").as_bytes());
            }
            inchar(0x20, vt, 0);
            expect_output(b" ");
            inchar(0x20, vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[32;2u");
            inchar(0x20, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[32;5u");
            inchar(0x20, vt, VTERM_MOD_CTRL | VTERM_MOD_SHIFT);
            expect_output(b"\x1b[32;6u");
            inchar(0x20, vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[32;3u");
            inchar(0x20, vt, VTERM_MOD_SHIFT | VTERM_MOD_ALT);
            expect_output(b"\x1b[32;4u");
            inchar(0x20, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[32;7u");
            inchar(0x20, vt, VTERM_MOD_CTRL | VTERM_MOD_SHIFT | VTERM_MOD_ALT);
            expect_output(b"\x1b[32;8u");
            inkey("up", vt, 0);
            expect_output(b"\x1b[A");
            inkey("up", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[1;2A");
            inkey("up", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[1;5A");
            inkey("up", vt, VTERM_MOD_CTRL | VTERM_MOD_SHIFT);
            expect_output(b"\x1b[1;6A");
            inkey("up", vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[1;3A");
            inkey("up", vt, VTERM_MOD_SHIFT | VTERM_MOD_ALT);
            expect_output(b"\x1b[1;4A");
            inkey("up", vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[1;7A");
            inkey("up", vt, VTERM_MOD_CTRL | VTERM_MOD_SHIFT | VTERM_MOD_ALT);
            expect_output(b"\x1b[1;8A");
            push(b"\x1b[?1h", vt);
            inkey("up", vt, 0);
            expect_output(b"\x1bOA");
            inkey("up", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[1;2A");
            inkey("up", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[1;5A");
            inkey("tab", vt, 0);
            expect_output(b"\t");
            inkey("tab", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[9;2u");
            inkey("tab", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[9;5u");
            inkey("tab", vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[9;3u");
            inkey("tab", vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[9;7u");
            inkey("bs", vt, 0);
            expect_output(b"\x7f");
            inkey("bs", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[127;2u");
            inkey("bs", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[127;5u");
            inkey("bs", vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[127;3u");
            inkey("bs", vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[127;7u");
            inkey("del", vt, 0);
            expect_output(b"\x1b[3~");
            inkey("del", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[3;2~");
            inkey("del", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[3;5~");
            inkey("del", vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[3;3~");
            inkey("del", vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[3;7~");
            inkey("esc", vt, 0);
            expect_output(b"\x1b[27;1u");
            inkey("esc", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[27;2u");
            inkey("esc", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[27;5u");
            inkey("esc", vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[27;3u");
            inkey("esc", vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[27;7u");
            inkey("enter", vt, 0);
            expect_output(b"\r");
            inkey("enter", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[13;2u");
            inkey("enter", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[13;5u");
            inkey("enter", vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[13;3u");
            inkey("enter", vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b[13;7u");
            push(b"\x1b[20h", vt);
            inkey("enter", vt, 0);
            expect_output(b"\r\n");
            inkey("f1", vt, 0);
            expect_output(b"\x1bOP");
            inkey("f1", vt, VTERM_MOD_SHIFT);
            expect_output(b"\x1b[1;2P");
            inkey("f1", vt, VTERM_MOD_ALT);
            expect_output(b"\x1b[1;3P");
            inkey("f1", vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b[1;5P");
            inkey("kp0", vt, 0);
            expect_output(b"\x1b[57399;1u");
            push(b"\x1b=", vt);
            inkey("kp0", vt, 0);
            expect_output(b"\x1bOp");
            vterm_keyboard_start_paste(vt);
            vterm_keyboard_end_paste(vt);
            push(b"\x1b[?2004h", vt);
            vterm_keyboard_start_paste(vt);
            expect_output(b"\x1b[200~");
            vterm_keyboard_end_paste(vt);
            expect_output(b"\x1b[201~");
            vterm_state_focus_in(state);
            vterm_state_focus_out(state);
            let _ = state;
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: false,
                    p: true,
                    s: false,
                },
            );
            push(b"\x1b[?1004h", vt);
            expect(b"settermprop 9 true");
            vterm_state_focus_in(state);
            expect_output(b"\x1b[I");
            vterm_state_focus_out(state);
            expect_output(b"\x1b[O");
            push(b"\x1b[?2026h", vt);
            expect(b"settermprop 11 true");
            push(b"\x1b[?2026l", vt);
            expect(b"settermprop 11 false");
            push(b"\x1b[?2026h", vt);
            expect(b"settermprop 11 true");
            push(b"\x1b[?2026$p", vt);
            expect_output(b"\x1b[?2026;1$y");
            push(b"\x1b[?2026l", vt);
            expect(b"settermprop 11 false");
            push(b"\x1b[?2026$p", vt);
            expect_output(b"\x1b[?2026;2$y");
            push(b"\x1b[<u", vt);
            inchar(0x41, vt, 0);
            expect_output(b"A");
            inchar(0x61, vt, 0);
            expect_output(b"a");
            inchar(0x41, vt, VTERM_MOD_CTRL);
            expect_output(b"\x01");
            inchar(0x61, vt, VTERM_MOD_CTRL);
            expect_output(b"\x01");
            inchar(0x41, vt, VTERM_MOD_ALT);
            expect_output(b"\x1bA");
            inchar(0x61, vt, VTERM_MOD_ALT);
            expect_output(b"\x1ba");
            inchar(0x41, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b\x01");
            inchar(0x61, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b\x01");
            inchar(0x49, vt, 0);
            expect_output(b"I");
            inchar(0x69, vt, 0);
            expect_output(b"i");
            inchar(0x49, vt, VTERM_MOD_CTRL);
            expect_output(b"\t");
            inchar(0x69, vt, VTERM_MOD_CTRL);
            expect_output(b"\t");
            inchar(0x49, vt, VTERM_MOD_ALT);
            expect_output(b"\x1bI");
            inchar(0x69, vt, VTERM_MOD_ALT);
            expect_output(b"\x1bi");
            inchar(0x49, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b\t");
            inchar(0x69, vt, VTERM_MOD_CTRL | VTERM_MOD_ALT);
            expect_output(b"\x1b\t");
            inchar(0x30, vt, VTERM_MOD_CTRL);
            expect_output(b"0");
            inchar(0x31, vt, VTERM_MOD_CTRL);
            expect_output(b"1");
            inchar(0x32, vt, VTERM_MOD_CTRL);
            expect_output(b"\x00");
            inchar(0x33, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1b");
            inchar(0x34, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1c");
            inchar(0x35, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1d");
            inchar(0x36, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1e");
            inchar(0x37, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1f");
            inchar(0x38, vt, VTERM_MOD_CTRL);
            expect_output(b"\x7f");
            inchar(0x39, vt, VTERM_MOD_CTRL);
            expect_output(b"9");
            inchar(0x2F, vt, VTERM_MOD_CTRL);
            expect_output(b"\x1f");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_26state_query() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            reset(state, ptr::null_mut());
            push(b"\x1b[c", vt);
            expect_output(b"\x1b[?61;22;52c");
            reset(state, ptr::null_mut());
            push(b"\x1b[>q", vt);
            expect_output(b"\x1bP>|libvterm(0.3)\x1b\\");
            reset(state, ptr::null_mut());
            push(b"\x1b[5n", vt);
            expect_output(b"\x1b[0n");
            push(b"\x1b[6n", vt);
            expect_output(b"\x1b[1;1R");
            push(b"\x1b[10;10H\x1b[6n", vt);
            expect_output(b"\x1b[10;10R");
            push(b"\x1b[?6n", vt);
            expect_output(b"\x1b[?10;10R");
            push(b"\x1b[3 q", vt);
            push(b"\x1bP$q q\x1b\\", vt);
            expect_output(b"\x1bP1$r3 q\x1b\\");
            push(b"\x1b[1;5;7m", vt);
            push(b"\x1bP$qm\x1b\\", vt);
            expect_output(b"\x1bP1$r1;5;7m\x1b\\");
            push(b"\x1b[0;31;42m", vt);
            push(b"\x1bP$qm\x1b\\", vt);
            expect_output(b"\x1bP1$r31;42m\x1b\\");
            push(b"\x1b[0;93;104m", vt);
            push(b"\x1bP$qm\x1b\\", vt);
            expect_output(b"\x1bP1$r93;104m\x1b\\");
            push(b"\x1b[0;38:5:56;48:5:78m", vt);
            push(b"\x1bP$qm\x1b\\", vt);
            expect_output(b"\x1bP1$r38:5:56;48:5:78m\x1b\\");
            push(b"\x1b[0;38:2:24:68:112;48:2:13:57:101m", vt);
            push(b"\x1bP$qm\x1b\\", vt);
            expect_output(b"\x1bP1$r38:2:24:68:112;48:2:13:57:101m\x1b\\");
            push(b"\x1b G", vt);
            push(b"\x1b[5n", vt);
            expect_output(b"\x9b0n");
            push(b"\x1b F", vt);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_27state_reset() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            reset(state, ptr::null_mut());
            push(b"\x1b[5;5H", vt);
            cursor(4, 4, state);
            let _ = state;
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: true,
                    p: false,
                    s: false,
                },
            );
            push(b"\x1bc", vt);
            cursor(0, 0, state);
            let _ = wantstate(vt, StateOpts::default());
            push(b"\x1b[5;10r", vt);
            let _ = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: false,
                    p: false,
                    s: true,
                },
            );
            push(b"\x1bc\x1b[25H\n", vt);
            expect(b"scrollrect 0..25,0..80 => +1,+0");
            let _ = wantstate(vt, StateOpts::default());
            push(b"ABCDE", vt);
            let _ = state;
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: true,
                    f: false,
                    g: false,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            push(b"\x1bc", vt);
            expect(b"erase 0..25,0..80");
            let _ = wantstate(vt, StateOpts::default());
            push(b"\x1b[5G\x1bH\x1b[G\t", vt);
            cursor(0, 4, state);
            push(b"\x1bc\t", vt);
            cursor(0, 8, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_28state_dbl_wh() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: true,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            push(b"\x1b#5", vt);
            push(b"Hello", vt);
            expect(b"putglyph 48 1 0,0\nputglyph 65 1 0,1\nputglyph 6c 1 0,2\nputglyph 6c 1 0,3\nputglyph 6f 1 0,4");
            reset(state, ptr::null_mut());
            push(b"\x1b#6", vt);
            push(b"Hello", vt);
            expect(b"putglyph 48 1 0,0 dwl\nputglyph 65 1 0,1 dwl\nputglyph 6c 1 0,2 dwl\nputglyph 6c 1 0,3 dwl\nputglyph 6f 1 0,4 dwl");
            cursor(0, 5, state);
            push(b"\x1b[40GAB", vt);
            expect(b"putglyph 41 1 0,39 dwl\nputglyph 42 1 1,0");
            cursor(1, 1, state);
            reset(state, ptr::null_mut());
            push(b"\x1b#3", vt);
            push(b"Hello", vt);
            expect(b"putglyph 48 1 0,0 dwl dhl-top\nputglyph 65 1 0,1 dwl dhl-top\nputglyph 6c 1 0,2 dwl dhl-top\nputglyph 6c 1 0,3 dwl dhl-top\nputglyph 6f 1 0,4 dwl dhl-top");
            cursor(0, 5, state);
            push(b"\r\n\x1b#4", vt);
            push(b"Hello", vt);
            expect(b"putglyph 48 1 1,0 dwl dhl-bottom\nputglyph 65 1 1,1 dwl dhl-bottom\nputglyph 6c 1 1,2 dwl dhl-bottom\nputglyph 6c 1 1,3 dwl dhl-bottom\nputglyph 6f 1 1,4 dwl dhl-bottom");
            cursor(1, 5, state);
            reset(state, ptr::null_mut());
            push(b"\x1b[20H\x1b#6ABC", vt);
            expect(b"putglyph 41 1 19,0 dwl\nputglyph 42 1 19,1 dwl\nputglyph 43 1 19,2 dwl");
            push(b"\x1b[25H\n", vt);
            push(b"\x1b[19;4HDE", vt);
            expect(b"putglyph 44 1 18,3 dwl\nputglyph 45 1 18,4 dwl");
            push(b"\x1b[H\x1bM", vt);
            push(b"\x1b[20;6HFG", vt);
            expect(b"putglyph 46 1 19,5 dwl\nputglyph 47 1 19,6 dwl");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_29state_fallback() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: true,
                    g: false,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            push(b"\x03", vt);
            expect(b"control 03");
            push(b"\x1b[?15;2z", vt);
            expect(b"csi 7a L=3f 15,2");
            push(b"\x1b]27;Something\x1b\\", vt);
            expect(b"osc [27;Something]");
            push(b"\x1bPz123\x1b\\", vt);
            expect(b"dcs [z123]");
            push(b"\x1b_z123\x1b\\", vt);
            expect(b"apc [z123]");
            push(b"\x1b^z123\x1b\\", vt);
            expect(b"pm [z123]");
            push(b"\x1bXz123\x1b\\", vt);
            expect(b"sos [z123]");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_30state_pen() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            push(b"\x1b[m", vt);
            pen("bold", 0, state);
            pen("underline", 0, state);
            pen("italic", 0, state);
            pen("blink", 0, state);
            pen("reverse", 0, state);
            pen("font", 0, state);
            push(b"\x1b[1m", vt);
            pen("bold", 1, state);
            push(b"\x1b[22m", vt);
            pen("bold", 0, state);
            push(b"\x1b[1m\x1b[m", vt);
            pen("bold", 0, state);
            push(b"\x1b[4m", vt);
            pen("underline", 1, state);
            push(b"\x1b[21m", vt);
            pen("underline", 2, state);
            push(b"\x1b[24m", vt);
            pen("underline", 0, state);
            push(b"\x1b[4m\x1b[4:0m", vt);
            pen("underline", 0, state);
            push(b"\x1b[4:1m", vt);
            pen("underline", 1, state);
            push(b"\x1b[4:2m", vt);
            pen("underline", 2, state);
            push(b"\x1b[4:3m", vt);
            pen("underline", 3, state);
            push(b"\x1b[4m\x1b[m", vt);
            pen("underline", 0, state);
            push(b"\x1b[3m", vt);
            pen("italic", 1, state);
            push(b"\x1b[23m", vt);
            pen("italic", 0, state);
            push(b"\x1b[3m\x1b[m", vt);
            pen("italic", 0, state);
            push(b"\x1b[5m", vt);
            pen("blink", 1, state);
            push(b"\x1b[25m", vt);
            pen("blink", 0, state);
            push(b"\x1b[5m\x1b[m", vt);
            pen("blink", 0, state);
            push(b"\x1b[7m", vt);
            pen("reverse", 1, state);
            push(b"\x1b[27m", vt);
            pen("reverse", 0, state);
            push(b"\x1b[7m\x1b[m", vt);
            pen("reverse", 0, state);
            push(b"\x1b[11m", vt);
            pen("font", 1, state);
            push(b"\x1b[19m", vt);
            pen("font", 9, state);
            push(b"\x1b[10m", vt);
            pen("font", 0, state);
            push(b"\x1b[11m\x1b[m", vt);
            pen("font", 0, state);
            push(b"\x1b[2m", vt);
            pen("dim", 1, state);
            push(b"\x1b[22m", vt);
            pen("dim", 0, state);
            push(b"\x1b[2m\x1b[m", vt);
            pen("dim", 0, state);
            push(b"\x1b[1;2m", vt);
            pen("bold", 1, state);
            pen("dim", 1, state);
            push(b"\x1b[22m", vt);
            pen("bold", 0, state);
            pen("dim", 0, state);
            push(b"\x1b[53m", vt);
            pen("overline", 1, state);
            push(b"\x1b[55m", vt);
            pen("overline", 0, state);
            push(b"\x1b[53m\x1b[m", vt);
            pen("overline", 0, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_31state_rep() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: true,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            reset(state, ptr::null_mut());
            push(b"a\x1b[b", vt);
            expect(b"putglyph 61 1 0,0\nputglyph 61 1 0,1");
            reset(state, ptr::null_mut());
            push(b"a\x1b[0b", vt);
            expect(b"putglyph 61 1 0,0\nputglyph 61 1 0,1");
            reset(state, ptr::null_mut());
            push(b"a\x1b[2b", vt);
            expect(b"putglyph 61 1 0,0\nputglyph 61 1 0,1\nputglyph 61 1 0,2");
            reset(state, ptr::null_mut());
            push(b"\xc3\xa9\x1b[b", vt);
            expect(b"putglyph e9 1 0,0\nputglyph e9 1 0,1");
            reset(state, ptr::null_mut());
            push(b"\xef\xbc\x90\x1b[b", vt);
            expect(b"putglyph ff10 2 0,0\nputglyph ff10 2 0,2");
            reset(state, ptr::null_mut());
            push(b"e\xcc\x81\x1b[b", vt);
            expect(b"putglyph 65,301 1 0,0\nputglyph 65,301 1 0,1");
            reset(state, ptr::null_mut());
            push(b"a\x1b[1000bb", vt);
            expect(b"putglyph 61 1 0,0\nputglyph 61 1 0,1\nputglyph 61 1 0,2\nputglyph 61 1 0,3\nputglyph 61 1 0,4\nputglyph 61 1 0,5\nputglyph 61 1 0,6\nputglyph 61 1 0,7\nputglyph 61 1 0,8\nputglyph 61 1 0,9\nputglyph 61 1 0,10\nputglyph 61 1 0,11\nputglyph 61 1 0,12\nputglyph 61 1 0,13\nputglyph 61 1 0,14\nputglyph 61 1 0,15\nputglyph 61 1 0,16\nputglyph 61 1 0,17\nputglyph 61 1 0,18\nputglyph 61 1 0,19\nputglyph 61 1 0,20\nputglyph 61 1 0,21\nputglyph 61 1 0,22\nputglyph 61 1 0,23\nputglyph 61 1 0,24\nputglyph 61 1 0,25\nputglyph 61 1 0,26\nputglyph 61 1 0,27\nputglyph 61 1 0,28\nputglyph 61 1 0,29\nputglyph 61 1 0,30\nputglyph 61 1 0,31\nputglyph 61 1 0,32\nputglyph 61 1 0,33\nputglyph 61 1 0,34\nputglyph 61 1 0,35\nputglyph 61 1 0,36\nputglyph 61 1 0,37\nputglyph 61 1 0,38\nputglyph 61 1 0,39\nputglyph 61 1 0,40\nputglyph 61 1 0,41\nputglyph 61 1 0,42\nputglyph 61 1 0,43\nputglyph 61 1 0,44\nputglyph 61 1 0,45\nputglyph 61 1 0,46\nputglyph 61 1 0,47\nputglyph 61 1 0,48\nputglyph 61 1 0,49\nputglyph 61 1 0,50\nputglyph 61 1 0,51\nputglyph 61 1 0,52\nputglyph 61 1 0,53\nputglyph 61 1 0,54\nputglyph 61 1 0,55\nputglyph 61 1 0,56\nputglyph 61 1 0,57\nputglyph 61 1 0,58\nputglyph 61 1 0,59\nputglyph 61 1 0,60\nputglyph 61 1 0,61\nputglyph 61 1 0,62\nputglyph 61 1 0,63\nputglyph 61 1 0,64\nputglyph 61 1 0,65\nputglyph 61 1 0,66\nputglyph 61 1 0,67\nputglyph 61 1 0,68\nputglyph 61 1 0,69\nputglyph 61 1 0,70\nputglyph 61 1 0,71\nputglyph 61 1 0,72\nputglyph 61 1 0,73\nputglyph 61 1 0,74\nputglyph 61 1 0,75\nputglyph 61 1 0,76\nputglyph 61 1 0,77\nputglyph 61 1 0,78\nputglyph 61 1 0,79\nputglyph 62 1 1,0");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_32state_flow() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            reset(state, ptr::null_mut());
            push(b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", vt);
            push(b"\r\n", vt);
            lineinfo(0, false, false, false, state);
            lineinfo(1, false, false, true, state);
            reset(state, ptr::null_mut());
            push(
                b"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
                vt,
            );
            push(b"\r\n", vt);
            push(b"BBBBBBBBBBBBBBBBBBBB", vt);
            push(b"\r\n", vt);
            lineinfo(0, false, false, false, state);
            lineinfo(1, false, false, false, state);
            reset(state, ptr::null_mut());
            push(b"DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD", vt);
            lineinfo(1, false, false, true, state);
            push(b"\x1bM\x1b[79G\x1b[K", vt);
            lineinfo(1, false, false, false, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_40state_selection() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let _ = wantstate(vt, StateOpts::default());
            push(b"\x1b]52;c;SGVsbG8s\x1b\\", vt);
            expect(b"selection-set mask=0001 [Hello,]");
            push(b"\x1b]52;c;SGVsbG8sIHc=\x1b\\", vt);
            expect(b"selection-set mask=0001 [Hello, w]");
            push(b"\x1b]52;c;SGVsbG8sIHdvcmxkCg==\x1b\\", vt);
            expect(b"selection-set mask=0001 [Hello, world\n]");
            push(b"\x1b]52;c;SGVs", vt);
            expect(b"selection-set mask=0001 [Hel");
            push(b"bG8s\x1b\\", vt);
            expect(b"selection-set mask=0001 lo,]");
            push(b"\x1b]52;c;SGVsbG", vt);
            expect(b"selection-set mask=0001 [Hel");
            push(b"8s\x1b\\", vt);
            expect(b"selection-set mask=0001 lo,]");
            push(b"\x1b]52;c;", vt);
            push(b"SGVsbG8s\x1b\\", vt);
            expect(b"selection-set mask=0001 [Hello,]");
            push(b"\x1b]52;c;SGVsbG8s", vt);
            expect(b"selection-set mask=0001 [Hello,");
            push(b"\x1b\\", vt);
            expect(b"selection-set mask=0001 ]");
            push(
                b"\x1b]52;c;LS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0t\x1b\\",
                vt,
            );
            expect(b"selection-set mask=0001 [---------------\nselection-set mask=0001 ---------------]");
            push(b"\x1b]52;c;\x1b\\", vt);
            expect(b"selection-set mask=0001 []");
            push(b"\x1b]52;c;SGVs*SGVsbG8s\x1b\\", vt);
            expect(b"selection-set mask=0001 []");
            push(b"\x1b]52;c;?\x1b\\", vt);
            expect(b"selection-query mask=0001");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_60screen_ascii() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: true,
                    b: false,
                    c: true,
                    m: false,
                    p: false,
                    r: false,
                },
            );
            reset(ptr::null_mut(), screen);
            push(b"ABC", vt);
            expect(b"movecursor 0,3");
            screen_chars(0, 0, 1, 3, b"ABC", screen);
            screen_chars(0, 0, 1, 80, b"ABC", screen);
            screen_text(0, 0, 1, 3, b"41,42,43", screen);
            screen_text(0, 0, 1, 80, b"41,42,43", screen);
            screen_cell(
                0,
                0,
                b"{41} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                0,
                1,
                b"{42} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                0,
                2,
                b"{43} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_row(0, b"ABC", screen, 80);
            screen_eol(0, 0, 0, screen);
            screen_eol(0, 2, 0, screen);
            screen_eol(0, 3, 1, screen);
            push(b"\x1b[H", vt);
            expect(b"movecursor 0,0");
            screen_row(0, b"ABC", screen, 80);
            screen_text(0, 0, 1, 80, b"41,42,43", screen);
            push(b"E", vt);
            expect(b"movecursor 0,1");
            screen_row(0, b"EBC", screen, 80);
            screen_text(0, 0, 1, 80, b"45,42,43", screen);
            let _ = screen;
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: true,
                    b: false,
                    c: false,
                    m: false,
                    p: false,
                    r: false,
                },
            );
            reset(ptr::null_mut(), screen);
            push(b"ABCDE\x1b[H\x1b[K", vt);
            screen_text(0, 0, 1, 80, b"", screen);
            reset(ptr::null_mut(), screen);
            push(b"ABC\x1b[H\x1b[@", vt);
            push(b"1", vt);
            screen_row(0, b"1ABC", screen, 80);
            reset(ptr::null_mut(), screen);
            push(b"ABC\x1b[H\x1b[P", vt);
            screen_chars(0, 0, 1, 1, b"B", screen);
            screen_chars(0, 1, 1, 2, b"C", screen);
            screen_chars(0, 0, 1, 80, b"BC", screen);
            reset(ptr::null_mut(), screen);
            push(b"Hello\x1b[CWorld", vt);
            screen_row(0, b"Hello World", screen, 80);
            screen_text(0, 0, 1, 80, b"48,65,6c,6c,6f,20,57,6f,72,6c,64", screen);
            reset(ptr::null_mut(), screen);
            push(b"Hello\r\nWorld", vt);
            screen_chars(0, 0, 2, 80, b"Hello\nWorld", screen);
            screen_text(0, 0, 2, 80, b"48,65,6c,6c,6f,0a,57,6f,72,6c,64", screen);
            reset(ptr::null_mut(), screen);
            push(b"P", vt);
            screen_row(0, b"P", screen, 80);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_61screen_unicode() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            push(b"\xc3\x81\xc3\xa9", vt);
            screen_row(0, b"\xc3\x81\xc3\xa9", screen, 80);
            screen_text(0, 0, 1, 80, b"c3,81,c3,a9", screen);
            screen_cell(
                0,
                0,
                b"{c1} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"0123\x1b[H", vt);
            push(b"\xef\xbc\x90", vt);
            screen_row(0, b"\xef\xbc\x9023", screen, 80);
            screen_text(0, 0, 1, 80, b"ef,bc,90,32,33", screen);
            screen_cell(
                0,
                0,
                b"{ff10} width=2 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"0123\x1b[H", vt);
            push(b"e\xcc\x81", vt);
            screen_row(0, b"e\xcc\x81123", screen, 80);
            screen_text(0, 0, 1, 80, b"65,cc,81,31,32,33", screen);
            screen_cell(
                0,
                0,
                b"{65,301} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"e\xcc\x81\xcc\x82\xcc\x83\xcc\x84\xcc\x85\xcc\x86\xcc\x87\xcc\x88\xcc\x89\xcc\x8a", vt);
            screen_cell(0, 0, b"{65,301,302,303,304,305,306,307,308,309,30a} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)", screen);
            reset(ptr::null_mut(), screen);
            push(b"e\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81", vt);
            push(b"\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81\xcc\x81", vt);
            screen_cell(0, 0, b"{65,301,301,301,301,301,301,301,301,301,301,301,301,301,301} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)", screen);
            reset(ptr::null_mut(), screen);
            push(b"\x1b[80G\xef\xbc\x90", vt);
            screen_cell(
                0,
                79,
                b"{} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                1,
                0,
                b"{ff10} width=2 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"\xf0\x9f\x8f\xb3\xef\xb8\x8f\xe2\x80\x8d\xf0\x9f\x8c\x88\xf0\x9f\x8f\xb3\xef\xb8\x8f\xe2\x80\x8d\xe2\x9a\xa7\xef\xb8\x8f\xf0\x9f\x8f\xb4\xe2\x80\x8d\xe2\x98\xa0\xef\xb8\x8f", vt);
            screen_cell(
                0,
                0,
                b"{1f3f3,fe0f,200d,1f308} width=2 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                0,
                2,
                b"{1f3f3,fe0f,200d,26a7,fe0f} width=2 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                0,
                4,
                b"{1f3f4,200d,2620,fe0f} width=2 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_63screen_resize() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(state, screen);
            resize(25, 80, vt);
            push(b"AB\r\nCD", vt);
            screen_chars(0, 0, 1, 80, b"AB", screen);
            screen_chars(1, 0, 2, 80, b"CD", screen);
            resize(25, 100, vt);
            screen_chars(0, 0, 1, 100, b"AB", screen);
            screen_chars(1, 0, 2, 100, b"CD", screen);
            reset(state, screen);
            resize(25, 80, vt);
            push(b"AB\x1b[79GCD", vt);
            screen_chars(0, 0, 1, 2, b"AB", screen);
            screen_chars(0, 78, 1, 80, b"CD", screen);
            resize(25, 100, vt);
            screen_chars(0, 0, 1, 2, b"AB", screen);
            screen_chars(0, 78, 1, 80, b"CD", screen);
            push(b"E", vt);
            screen_chars(0, 78, 1, 81, b"CDE", screen);
            reset(state, screen);
            resize(25, 80, vt);
            push(b"Top\x1b[10HLine 10", vt);
            screen_row(0, b"Top", screen, 80);
            screen_row(9, b"Line 10", screen, 80);
            cursor(9, 7, state);
            resize(20, 80, vt);
            screen_row(0, b"Top", screen, 80);
            screen_row(9, b"Line 10", screen, 80);
            cursor(9, 7, state);
            reset(state, screen);
            resize(25, 80, vt);
            push(b"Top\x1b[25HLine 25\x1b[15H", vt);
            screen_row(0, b"Top", screen, 80);
            screen_row(24, b"Line 25", screen, 80);
            cursor(14, 0, state);
            let _ = screen;
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: false,
                    b: true,
                    c: false,
                    m: false,
                    p: false,
                    r: false,
                },
            );
            resize(20, 80, vt);
            expect(b"sb_pushline 80 = 54 6f 70\nsb_pushline 80 =\nsb_pushline 80 =\nsb_pushline 80 =\nsb_pushline 80 =");
            screen_row(19, b"Line 25", screen, 80);
            cursor(9, 0, state);
            reset(state, screen);
            let _ = wantscreen(vt, ScreenOpts::default());
            resize(25, 80, vt);
            let _ = screen;
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: false,
                    b: true,
                    c: false,
                    m: false,
                    p: false,
                    r: false,
                },
            );
            push(b"\x1b[24HLine 24\r\nLine 25\r\n", vt);
            expect(b"sb_pushline 80 =");
            screen_row(23, b"Line 25", screen, 80);
            cursor(24, 0, state);
            resize(24, 80, vt);
            expect(b"sb_pushline 80 =");
            screen_row(22, b"Line 25", screen, 80);
            cursor(23, 0, state);
            reset(state, screen);
            let _ = wantscreen(vt, ScreenOpts::default());
            resize(25, 80, vt);
            let _ = screen;
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: false,
                    b: true,
                    c: false,
                    m: false,
                    p: false,
                    r: false,
                },
            );
            push(b"\x1b[24HLine 24\r\nLine 25\x1b[H", vt);
            cursor(0, 0, state);
            resize(20, 80, vt);
            expect(b"sb_pushline 80 =\nsb_pushline 80 =\nsb_pushline 80 =\nsb_pushline 80 =\nsb_pushline 80 =");
            cursor(0, 0, state);
            reset(state, screen);
            let _ = screen;
            let screen = wantscreen(vt, ScreenOpts::default());
            resize(25, 80, vt);
            push(b"Line 1\x1b[25HBottom\x1b[15H", vt);
            screen_row(0, b"Line 1", screen, 80);
            screen_row(24, b"Bottom", screen, 80);
            cursor(14, 0, state);
            let _ = screen;
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: false,
                    b: true,
                    c: false,
                    m: false,
                    p: false,
                    r: false,
                },
            );
            resize(30, 80, vt);
            expect(b"sb_popline 80\nsb_popline 80\nsb_popline 80\nsb_popline 80\nsb_popline 80");
            screen_row(0, b"ABCDE", screen, 80);
            screen_row(5, b"Line 1", screen, 80);
            screen_row(29, b"Bottom", screen, 80);
            cursor(19, 0, state);
            let _ = screen;
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(state, screen);
            let _ = screen;
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: true,
                    b: false,
                    c: false,
                    m: false,
                    p: false,
                    r: false,
                },
            );
            resize(25, 80, vt);
            push(b"Main screen\x1b[?1049h\x1b[HAlt screen", vt);
            resize(30, 80, vt);
            screen_row(0, b"Alt screen", screen, 80);
            push(b"\x1b[?1049l", vt);
            screen_row(0, b"Main screen", screen, 80);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_64screen_pen() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            push(b"A", vt);
            screen_cell(
                0,
                0,
                b"{41} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[1mB", vt);
            screen_cell(
                0,
                1,
                b"{42} width=1 attrs={B} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[3mC", vt);
            screen_cell(
                0,
                2,
                b"{43} width=1 attrs={BI} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[4mD", vt);
            screen_cell(
                0,
                3,
                b"{44} width=1 attrs={BU1I} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[mE", vt);
            screen_cell(
                0,
                4,
                b"{45} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[11mF\x1b[m", vt);
            screen_cell(
                0,
                5,
                b"{46} width=1 attrs={F1} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[31mG\x1b[m", vt);
            screen_cell(
                0,
                6,
                b"{47} width=1 attrs={} fg=rgb(224,0,0) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[42mH\x1b[m", vt);
            screen_cell(
                0,
                7,
                b"{48} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,224,0)",
                screen,
            );
            push(b"x\x1b[74m0\x1b[73m2\x1b[m", vt);
            screen_cell(
                0,
                8,
                b"{78} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                0,
                9,
                b"{30} width=1 attrs={S_} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                0,
                10,
                b"{32} width=1 attrs={S^} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[2mI\x1b[m", vt);
            screen_cell(
                0,
                11,
                b"{49} width=1 attrs={D} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[53mJ\x1b[m", vt);
            screen_cell(
                0,
                12,
                b"{4a} width=1 attrs={O} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[H\x1b[7;33;44m\x1b[K", vt);
            screen_cell(
                0,
                0,
                b"{} width=1 attrs={} fg=rgb(224,224,0) bg=rgb(0,0,224)",
                screen,
            );
            screen_cell(
                0,
                79,
                b"{} width=1 attrs={} fg=rgb(224,224,0) bg=rgb(0,0,224)",
                screen,
            );
            push(b"R\x1b[?5h", vt);
            screen_cell(
                0,
                0,
                b"{52} width=1 attrs={} fg=rgb(224,224,0) bg=rgb(0,0,224)",
                screen,
            );
            screen_cell(
                1,
                0,
                b"{} width=1 attrs={R} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b[?5$p", vt);
            expect_output(b"\x1b[?5;1$y");
            push(b"\x1b[?5l", vt);
            screen_cell(
                0,
                0,
                b"{52} width=1 attrs={R} fg=rgb(224,224,0) bg=rgb(0,0,224)",
                screen,
            );
            screen_cell(
                1,
                0,
                b"{} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"ABC\x1b[31mDEF\x1b[m", vt);
            screen_cell(
                0,
                0,
                b"{41} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                0,
                3,
                b"{44} width=1 attrs={} fg=rgb(224,0,0) bg=rgb(0,0,0)",
                screen,
            );
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_65screen_protect() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            push(b"A\x1b[1\"qB\x1b[\"qC", vt);
            screen_row(0, b"ABC", screen, 80);
            push(b"\x1b[G\x1b[?J", vt);
            screen_row(0, b" B", screen, 80);
            reset(ptr::null_mut(), screen);
            push(b"A\x1b[1\"qB\x1b[\"qC", vt);
            screen_row(0, b"ABC", screen, 80);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_66screen_extent() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            push(b"AB\x1b[1mCD\x1b[mE", vt);
            screen_attrs_extent(0, 0, b"0,0-1,1", screen);
            screen_attrs_extent(0, 1, b"0,0-1,1", screen);
            screen_attrs_extent(0, 2, b"0,2-1,3", screen);
            screen_attrs_extent(0, 3, b"0,2-1,3", screen);
            screen_attrs_extent(0, 4, b"0,4-1,79", screen);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_67screen_dbl_wh() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            reset(ptr::null_mut(), screen);
            push(b"\x1b#5", vt);
            push(b"abcde", vt);
            screen_cell(
                0,
                0,
                b"{61} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"\x1b#6", vt);
            push(b"abcde", vt);
            screen_cell(
                0,
                0,
                b"{61} width=1 attrs={} dwl fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"\x1b#3", vt);
            push(b"abcde", vt);
            push(b"\r\n\x1b#4", vt);
            push(b"abcde", vt);
            screen_cell(
                0,
                0,
                b"{61} width=1 attrs={} dwl dhl-top fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                1,
                0,
                b"{61} width=1 attrs={} dwl dhl-bottom fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"abcde", vt);
            screen_cell(
                0,
                0,
                b"{61} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            push(b"\x1b#6", vt);
            screen_cell(
                0,
                0,
                b"{61} width=1 attrs={} dwl fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            reset(ptr::null_mut(), screen);
            push(b"\x1b[25H\x1b#6Final\r\n", vt);
            screen_cell(
                23,
                0,
                b"{46} width=1 attrs={} dwl fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            screen_cell(
                24,
                0,
                b"{} width=1 attrs={} fg=rgb(240,240,240) bg=rgb(0,0,0)",
                screen,
            );
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_68screen_termprops() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: false,
                    b: false,
                    c: false,
                    m: false,
                    p: true,
                    r: false,
                },
            );
            reset(ptr::null_mut(), screen);
            expect(b"settermprop 1 true\nsettermprop 2 true\nsettermprop 7 1");
            push(b"\x1b[?25h", vt);
            expect(b"settermprop 1 true");
            push(b"\x1b[?25l", vt);
            expect(b"settermprop 1 false");
            push(b"\x1b]2;Here is my title\x07", vt);
            expect(b"settermprop 4 [\"Here is my title\"]");
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_69screen_pushline() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            resize(5, 10, vt);
            let state = wantstate(vt, StateOpts::default());
            let screen = wantscreen(
                vt,
                ScreenOpts {
                    a: false,
                    b: false,
                    c: false,
                    m: false,
                    p: false,
                    r: true,
                },
            );
            reset(state, screen);
            reset(state, screen);
            push(b"AAAAAAAAAAAA", vt);
            screen_row(0, b"AAAAAAAAAA", screen, (*vt).cols);
            screen_row(1, b"AA", screen, (*vt).cols);
            lineinfo(1, false, false, true, state);
            cursor(1, 2, state);
            resize(5, 15, vt);
            screen_row(0, b"AAAAAAAAAAAA", screen, (*vt).cols);
            lineinfo(1, false, false, false, state);
            cursor(0, 12, state);
            resize(5, 20, vt);
            screen_row(0, b"AAAAAAAAAAAA", screen, (*vt).cols);
            lineinfo(1, false, false, false, state);
            cursor(0, 12, state);
            reset(state, screen);
            resize(5, 10, vt);
            push(b"ABCDEFGHI", vt);
            screen_row(0, b"ABCDEFGHI", screen, (*vt).cols);
            lineinfo(1, false, false, false, state);
            cursor(0, 9, state);
            resize(5, 8, vt);
            screen_row(1, b"I", screen, (*vt).cols);
            lineinfo(1, false, false, true, state);
            cursor(1, 1, state);
            resize(5, 6, vt);
            screen_row(0, b"ABCDEF", screen, (*vt).cols);
            screen_row(1, b"GHI", screen, (*vt).cols);
            lineinfo(1, false, false, true, state);
            cursor(1, 3, state);
            reset(state, screen);
            resize(5, 10, vt);
            push(b"PROMPT GOES HERE\r\n> \r\n\r\nPROMPT GOES HERE\r\n> ", vt);
            screen_row(0, b"> ", screen, (*vt).cols);
            screen_row(2, b"PROMPT GOE", screen, (*vt).cols);
            screen_row(3, b"S HERE", screen, (*vt).cols);
            lineinfo(3, false, false, true, state);
            screen_row(4, b"> ", screen, (*vt).cols);
            cursor(4, 2, state);
            resize(5, 11, vt);
            screen_row(0, b"> ", screen, (*vt).cols);
            screen_row(2, b"PROMPT GOES", screen, (*vt).cols);
            screen_row(3, b" HERE", screen, (*vt).cols);
            lineinfo(3, false, false, true, state);
            screen_row(4, b"> ", screen, (*vt).cols);
            cursor(4, 2, state);
            resize(5, 12, vt);
            screen_row(0, b"> ", screen, (*vt).cols);
            screen_row(2, b"PROMPT GOES ", screen, (*vt).cols);
            screen_row(3, b"HERE", screen, (*vt).cols);
            lineinfo(3, false, false, true, state);
            screen_row(4, b"> ", screen, (*vt).cols);
            cursor(4, 2, state);
            resize(5, 16, vt);
            screen_row(0, b"> ", screen, (*vt).cols);
            lineinfo(3, false, false, false, state);
            screen_row(3, b"> ", screen, (*vt).cols);
            cursor(3, 2, state);
            reset(state, screen);
            resize(5, 5, vt);
            resize(3, 1, vt);
            push(b"\x1b[2;1Habc\r\n\x1b[H", vt);
            resize(1, 1, vt);
            cursor(0, 0, state);
            reset(state, screen);
            resize(4, 4, vt);
            push(b"ABCD\r\nX", vt);
            screen_row(0, b"ABCD", screen, (*vt).cols);
            screen_row(1, b"X", screen, (*vt).cols);
            cursor(1, 1, state);
            resize(4, 5, vt);
            screen_row(0, b"ABCD", screen, (*vt).cols);
            screen_row(1, b"X", screen, (*vt).cols);
            cursor(1, 1, state);
            reset(state, screen);
            resize(4, 4, vt);
            push(b"ABCD\r\nX", vt);
            cursor(1, 1, state);
            resize(3, 4, vt);
            screen_row(0, b"ABCD", screen, (*vt).cols);
            screen_row(1, b"X", screen, (*vt).cols);
            cursor(1, 1, state);
            resize(5, 10, vt);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_90vttest_01_movement_3() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let state = wantstate(vt, StateOpts::default());
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(state, screen);
            push(b"A B C D E F G H I", vt);
            push(b"\r\n", vt);
            push(b"A\x1b[2\x08CB\x1b[2\x08CC\x1b[2\x08CD\x1b[2\x08CE\x1b[2\x08CF\x1b[2\x08CG\x1b[2\x08CH\x1b[2\x08CI", vt);
            push(b"\r\n", vt);
            push(b"A \x1b[\r2CB\x1b[\r4CC\x1b[\r6CD\x1b[\r8CE\x1b[\r10CF\x1b[\r12CG\x1b[\r14CH\x1b[\r16CI", vt);
            push(b"\r\n", vt);
            push(b"A \x1b[1\x0bAB \x1b[1\x0bAC \x1b[1\x0bAD \x1b[1\x0bAE \x1b[1\x0bAF \x1b[1\x0bAG \x1b[1\x0bAH \x1b[1\x0bAI \x1b[1\x0bA", vt);
            for i in 0..=2 {
                screen_row(i, b"A B C D E F G H I", screen, 80);
            }
            screen_row(3, b"A B C D E F G H I ", screen, 80);
            cursor(3, 18, state);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_90vttest_01_movement_4() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            push(b"\x1b[00000000004;000000001HT", vt);
            push(b"\x1b[00000000004;000000002Hh", vt);
            push(b"\x1b[00000000004;000000003Hi", vt);
            push(b"\x1b[00000000004;000000004Hs", vt);
            push(b"\x1b[00000000004;000000005H ", vt);
            push(b"\x1b[00000000004;000000006Hi", vt);
            push(b"\x1b[00000000004;000000007Hs", vt);
            push(b"\x1b[00000000004;000000008H ", vt);
            push(b"\x1b[00000000004;000000009Ha", vt);
            push(b"\x1b[00000000004;0000000010H ", vt);
            push(b"\x1b[00000000004;0000000011Hc", vt);
            push(b"\x1b[00000000004;0000000012Ho", vt);
            push(b"\x1b[00000000004;0000000013Hr", vt);
            push(b"\x1b[00000000004;0000000014Hr", vt);
            push(b"\x1b[00000000004;0000000015He", vt);
            push(b"\x1b[00000000004;0000000016Hc", vt);
            push(b"\x1b[00000000004;0000000017Ht", vt);
            push(b"\x1b[00000000004;0000000018H ", vt);
            push(b"\x1b[00000000004;0000000019Hs", vt);
            push(b"\x1b[00000000004;0000000020He", vt);
            push(b"\x1b[00000000004;0000000021Hn", vt);
            push(b"\x1b[00000000004;0000000022Ht", vt);
            push(b"\x1b[00000000004;0000000023He", vt);
            push(b"\x1b[00000000004;0000000024Hn", vt);
            push(b"\x1b[00000000004;0000000025Hc", vt);
            push(b"\x1b[00000000004;0000000026He", vt);
            screen_row(3, b"This is a correct sentence", screen, 80);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_90vttest_02_screen_3() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            push(b"\x1b[?6h", vt);
            push(b"\x1b[23;24r", vt);
            push(b"\n", vt);
            push(b"Bottom", vt);
            push(b"\x1b[1;1H", vt);
            push(b"Above", vt);
            screen_row(22, b"Above", screen, 80);
            screen_row(23, b"Bottom", screen, 80);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_90vttest_02_screen_4() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let screen = wantscreen(vt, ScreenOpts::default());
            reset(ptr::null_mut(), screen);
            push(b"\x1b[?6l", vt);
            push(b"\x1b[23;24r", vt);
            push(b"\x1b[24;1H", vt);
            push(b"Bottom", vt);
            push(b"\x1b[1;1H", vt);
            push(b"Top", vt);
            screen_row(23, b"Bottom", screen, 80);
            screen_row(0, b"Top", screen, 80);
            vterm_free(vt);
        }
    }

    #[test]
    fn spec_mouse_reporting_should_not_break_by_idempotent_decsm_1002() {
        let _guard = VTERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_test_output();
        unsafe {
            let vt = init();
            let _ = wantstate(
                vt,
                StateOpts {
                    b: false,
                    e: false,
                    f: false,
                    g: false,
                    m: false,
                    p: false,
                    s: false,
                },
            );
            push(b"\x1b[?1002h", vt);
            mousemove(0, 0, vt, 0);
            mousebtn(b'd', 1, vt, 0);
            expect_output(b"\x1b[M !!");
            mousemove(1, 0, vt, 0);
            expect_output(b"\x1b[M@!\"");
            push(b"\x1b[?1002h", vt);
            mousemove(2, 0, vt, 0);
            expect_output(b"\x1b[M@!#");
            vterm_free(vt);
        }
    }
}
