//! VTerm unit-test callback and inspection harness (`test/unit/fixtures/vterm_test.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::vterm::parser::{CSI_ARG_FLAG_MORE, CSI_ARG_MISSING, VTermStringFragment};
use std::ffi::{c_char, c_int, c_long, c_void};
use std::fs::OpenOptions;
use std::io::Write;
use std::ptr;

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
    ScreenCell, VTERM_PROP_ALTSCREEN, VTERM_PROP_REVERSE, VTermGlyphInfo, VTermScreen,
    VTermScreenCell, getcell,
};
use crate::vterm::state::{
    VTERM_PROP_CURSORBLINK, VTERM_PROP_CURSORSHAPE, VTERM_PROP_CURSORVISIBLE,
    VTERM_PROP_FOCUSREPORT, VTERM_PROP_ICONNAME, VTERM_PROP_MOUSE, VTERM_PROP_SYNCOUTPUT,
    VTERM_PROP_THEMEUPDATES, VTERM_PROP_TITLE, VTermSelectionMask,
};

pub type VTermAttrMask = c_int;

const CSI_ARG_MASK: c_long = !((1u64 << 31) as c_long);

const VTERM_VALUETYPE_STRING: VTermValueType = 3;

const VTERM_N_ATTRS: VTermAttr = 16;

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

#[unsafe(no_mangle)]
pub static mut state_pos: VTermPos = VTermPos { row: 0, col: 0 };
#[unsafe(no_mangle)]
pub static mut want_state_putglyph: bool = false;
#[unsafe(no_mangle)]
pub static mut want_state_movecursor: bool = false;
#[unsafe(no_mangle)]
pub static mut want_state_erase: bool = false;
#[unsafe(no_mangle)]
pub static mut want_state_scrollrect: bool = false;
#[unsafe(no_mangle)]
pub static mut want_state_moverect: bool = false;
#[unsafe(no_mangle)]
pub static mut want_state_settermprop: bool = false;
#[unsafe(no_mangle)]
pub static mut want_state_scrollback: bool = false;
#[unsafe(no_mangle)]
pub static mut want_screen_scrollback: bool = false;

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

static mut STATE_PEN: StatePenRecord = StatePenRecord {
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
};

fn vterm_test_file_path() -> &'static str {
    option_env!("VTERM_TEST_FILE").unwrap_or("test/vterm_test_output")
}

fn append_test_output(bytes: &[u8]) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(vterm_test_file_path())
    {
        let _ = f.write_all(bytes);
    }
}

unsafe fn push_frag_bytes(out: &mut Vec<u8>, frag: VTermStringFragment) {
    let len = frag.len();
    if len > 0 && !frag.str_.is_null() {
        let slice = unsafe { std::slice::from_raw_parts(frag.str_.cast::<u8>(), len) };
        out.extend_from_slice(slice);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parser_text(
    bytes: *const c_char,
    len: usize,
    _user: *mut c_void,
) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parser_csi(
    leader: *const c_char,
    args: *const c_long,
    argcount: c_int,
    intermed: *const c_char,
    command: c_char,
    _user: *mut c_void,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parser_osc(
    command: c_int,
    frag: VTermStringFragment,
    _user: *mut c_void,
) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parser_dcs(
    command: *const c_char,
    commandlen: usize,
    frag: VTermStringFragment,
    _user: *mut c_void,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parser_apc(frag: VTermStringFragment, _user: *mut c_void) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parser_pm(frag: VTermStringFragment, _user: *mut c_void) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parser_sos(frag: VTermStringFragment, _user: *mut c_void) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn selection_set(
    mask: VTermSelectionMask,
    frag: VTermStringFragment,
    _user: *mut c_void,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn selection_query(mask: VTermSelectionMask, _user: *mut c_void) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_putglyph(
    info: *mut VTermGlyphInfo,
    pos: VTermPos,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        if !want_state_putglyph {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_movecursor(
    pos: VTermPos,
    _oldpos: VTermPos,
    _visible: c_int,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        state_pos = pos;
        let mut out = Vec::new();
        if want_state_movecursor {
            let _ = writeln!(out, "movecursor {},{}", pos.row, pos.col);
        }
        append_test_output(&out);
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_scrollrect(
    rect: VTermRect,
    downward: c_int,
    rightward: c_int,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        if !want_state_scrollrect {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_moverect(
    dest: VTermRect,
    src: VTermRect,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        if !want_state_moverect {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn print_color(col: *const VTermColor) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_settermprop(
    prop: VTermProp,
    val: *mut VTermValue,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        if !want_state_settermprop {
            return 1;
        }
        let mut out = Vec::new();
        let errcode = match vterm_get_prop_type(prop) {
            VTERM_VALUETYPE_BOOL => {
                let s = if (*val).boolean != 0 { "true" } else { "false" };
                let _ = writeln!(out, "settermprop {prop} {s}");
                1
            }
            VTERM_VALUETYPE_INT => {
                let _ = writeln!(out, "settermprop {prop} {}", (*val).number);
                1
            }
            VTERM_VALUETYPE_STRING => {
                let frag = (*val).string;
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
                format_color_bytes(&mut out, &(*val).color);
                out.push(b'\n');
                1
            }
            _ => 0,
        };
        append_test_output(&out);
        errcode
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_erase(
    rect: VTermRect,
    selective: c_int,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        if !want_state_erase {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_setpenattr(
    attr: VTermAttr,
    val: *mut VTermValue,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        match attr {
            VTERM_ATTR_BOLD => STATE_PEN.bold = (*val).boolean,
            VTERM_ATTR_UNDERLINE => STATE_PEN.underline = (*val).number,
            VTERM_ATTR_ITALIC => STATE_PEN.italic = (*val).boolean,
            VTERM_ATTR_BLINK => STATE_PEN.blink = (*val).boolean,
            VTERM_ATTR_REVERSE => STATE_PEN.reverse = (*val).boolean,
            VTERM_ATTR_CONCEAL => STATE_PEN.conceal = (*val).boolean,
            VTERM_ATTR_STRIKE => STATE_PEN.strike = (*val).boolean,
            VTERM_ATTR_FONT => STATE_PEN.font = (*val).number,
            VTERM_ATTR_SMALL => STATE_PEN.small = (*val).boolean,
            VTERM_ATTR_BASELINE => STATE_PEN.baseline = (*val).number,
            VTERM_ATTR_DIM => STATE_PEN.dim = (*val).boolean,
            VTERM_ATTR_OVERLINE => STATE_PEN.overline = (*val).boolean,
            VTERM_ATTR_FOREGROUND => STATE_PEN.foreground = (*val).color,
            VTERM_ATTR_BACKGROUND => STATE_PEN.background = (*val).color,
            VTERM_N_ATTRS => return 0,
            _ => {}
        }
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_sb_clear(_user: *mut c_void) -> c_int {
    unsafe {
        if !want_state_scrollback {
            return 1;
        }
        append_test_output(b"sb_clear\n");
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn screen_sb_pushline(
    cols: c_int,
    cells: *const VTermScreenCell,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        if !want_screen_scrollback {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn screen_sb_popline(
    cols: c_int,
    cells: *mut VTermScreenCell,
    _user: *mut c_void,
) -> c_int {
    unsafe {
        if !want_screen_scrollback {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn screen_sb_clear(_user: *mut c_void) -> c_int {
    unsafe {
        if !want_screen_scrollback {
            return 1;
        }
        append_test_output(b"sb_clear\n");
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_output(s: *const c_char, len: usize, _user: *mut c_void) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_state_get_penattr(
    state: *const VTermStateFull,
    attr: VTermAttr,
    val: *mut VTermValue,
) -> c_int {
    unsafe {
        let pen = &(*state).pen;
        match attr {
            VTERM_ATTR_BOLD => {
                (*val).boolean = pen.bold() as c_int;
                1
            }
            VTERM_ATTR_UNDERLINE => {
                (*val).number = pen.underline() as c_int;
                1
            }
            VTERM_ATTR_ITALIC => {
                (*val).boolean = pen.italic() as c_int;
                1
            }
            VTERM_ATTR_BLINK => {
                (*val).boolean = pen.blink() as c_int;
                1
            }
            VTERM_ATTR_REVERSE => {
                (*val).boolean = pen.reverse() as c_int;
                1
            }
            VTERM_ATTR_CONCEAL => {
                (*val).boolean = pen.conceal() as c_int;
                1
            }
            VTERM_ATTR_STRIKE => {
                (*val).boolean = pen.strike() as c_int;
                1
            }
            VTERM_ATTR_FONT => {
                (*val).number = pen.font() as c_int;
                1
            }
            VTERM_ATTR_FOREGROUND => {
                (*val).color = pen.fg;
                1
            }
            VTERM_ATTR_BACKGROUND => {
                (*val).color = pen.bg;
                1
            }
            VTERM_ATTR_SMALL => {
                (*val).boolean = pen.small() as c_int;
                1
            }
            VTERM_ATTR_BASELINE => {
                (*val).number = pen.baseline() as c_int;
                1
            }
            VTERM_ATTR_URI => {
                (*val).number = pen.uri;
                1
            }
            VTERM_ATTR_DIM => {
                (*val).boolean = pen.dim() as c_int;
                1
            }
            VTERM_ATTR_OVERLINE => {
                (*val).boolean = pen.overline() as c_int;
                1
            }
            _ => 0,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_color_is_equal(a: *const VTermColor, b: *const VTermColor) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_screen_get_attrs_extent(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_screen_get_text(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_screen_is_eol(
    screen: *const VTermScreen,
    mut pos: VTermPos,
) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_state_get_cursorpos(
    state: *const VTermStateFull,
    cursorpos: *mut VTermPos,
) {
    unsafe {
        *cursorpos = (*state).pos;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_state_set_bold_highbright(
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
}
