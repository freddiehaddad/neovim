//! Core VTerm instance management and output buffering (`src/nvim/vterm/vterm.c`).

use std::ffi::{c_char, c_int};
use std::ptr;

use crate::terminal::Terminal;
use crate::vterm::mouse::VTerm;
use crate::vterm::parser::NORMAL;
use crate::vterm::pen::{
    VTERM_ATTR_BACKGROUND, VTERM_ATTR_BASELINE, VTERM_ATTR_BLINK, VTERM_ATTR_BOLD,
    VTERM_ATTR_CONCEAL, VTERM_ATTR_DIM, VTERM_ATTR_FONT, VTERM_ATTR_FOREGROUND, VTERM_ATTR_ITALIC,
    VTERM_ATTR_OVERLINE, VTERM_ATTR_REVERSE, VTERM_ATTR_SMALL, VTERM_ATTR_STRIKE,
    VTERM_ATTR_UNDERLINE, VTERM_ATTR_URI, VTERM_VALUETYPE_BOOL, VTERM_VALUETYPE_COLOR,
    VTERM_VALUETYPE_INT, VTermAttr, VTermRect, VTermValueType,
};
use crate::vterm::screen::VTermScreen;
use crate::vterm::state::{VT_MODE_CTRL8BIT, VT_MODE_UTF8};

pub type VTermOutputCallback = Option<unsafe fn(s: *const c_char, len: usize, user: *mut Terminal)>;

/// Create a new `VTerm` instance of size `rows` x `cols` (`vterm_new`).
pub fn vterm_new(rows: c_int, cols: c_int) -> VTerm {
    let mut vt = VTerm::new();
    vt.rows = rows;
    vt.cols = cols;
    vt.parser.state = NORMAL;
    vt.parser.callbacks = None;
    vt.parser.cbdata = ptr::null_mut();
    vt.parser.emit_nul = false;
    vt.outfunc = None;
    vt.outdata = ptr::null_mut();
    vt
}

pub unsafe fn vterm_get_size(vt: *const VTerm, rowsp: *mut c_int, colsp: *mut c_int) {
    unsafe {
        if !rowsp.is_null() {
            *rowsp = (*vt).rows;
        }
        if !colsp.is_null() {
            *colsp = (*vt).cols;
        }
    }
}

pub unsafe fn vterm_set_size(vt: *mut VTerm, rows: c_int, cols: c_int) {
    if rows < 1 || cols < 1 {
        return;
    }

    unsafe {
        (*vt).rows = rows;
        (*vt).cols = cols;

        if let Some(callbacks) = (*vt).parser.callbacks
            && let Some(resize_cb) = callbacks.resize
        {
            let _ = resize_cb(rows, cols, (*vt).parser.cbdata);
        }
    }
}

pub unsafe fn vterm_set_utf8(vt: *mut VTerm, is_utf8: c_int) {
    unsafe {
        if is_utf8 != 0 {
            (*vt).mode |= VT_MODE_UTF8;
        } else {
            (*vt).mode &= !VT_MODE_UTF8;
        }
    }
}

pub unsafe fn vterm_output_set_callback(
    vt: *mut VTerm,
    func: VTermOutputCallback,
    user: *mut Terminal,
) {
    unsafe {
        (*vt).outfunc = func;
        (*vt).outdata = user;
    }
}

pub unsafe fn vterm_push_output_bytes(vt: *mut VTerm, bytes: *const c_char, len: usize) {
    unsafe {
        if let Some(func) = (*vt).outfunc {
            func(bytes, len, (*vt).outdata);
        }
    }
}

pub unsafe fn vterm_push_output_bytes_slice(vt: *mut VTerm, payload: &[u8]) {
    unsafe {
        vterm_push_output_bytes(vt, payload.as_ptr().cast::<c_char>(), payload.len());
    }
}

pub unsafe fn vterm_push_output_ctrl_bytes(vt: *mut VTerm, ctrl: u8, payload: &[u8]) {
    unsafe {
        let ctrl8bit = ((*vt).mode & VT_MODE_CTRL8BIT) != 0;
        let mut buf = Vec::with_capacity(2 + payload.len());
        if ctrl >= 0x80 && !ctrl8bit {
            buf.push(0x1b);
            buf.push(ctrl - 0x40);
        } else {
            buf.push(ctrl);
        }
        buf.extend_from_slice(payload);
        vterm_push_output_bytes_slice(vt, &buf);
    }
}

pub unsafe fn vterm_push_output_str_bytes(vt: *mut VTerm, ctrl: u8, term: bool, payload: &[u8]) {
    unsafe {
        let ctrl8bit = ((*vt).mode & VT_MODE_CTRL8BIT) != 0;
        let mut buf = Vec::with_capacity(4 + payload.len());
        if ctrl != 0 {
            if ctrl >= 0x80 && !ctrl8bit {
                buf.push(0x1b);
                buf.push(ctrl - 0x40);
            } else {
                buf.push(ctrl);
            }
        }
        buf.extend_from_slice(payload);
        if term {
            let st: &[u8] = if ctrl8bit { b"\x9c" } else { b"\x1b\\" };
            buf.extend_from_slice(st);
        }
        vterm_push_output_bytes_slice(vt, &buf);
    }
}

pub fn vterm_get_attr_type(attr: VTermAttr) -> VTermValueType {
    match attr {
        VTERM_ATTR_BOLD => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_UNDERLINE => VTERM_VALUETYPE_INT,
        VTERM_ATTR_ITALIC => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_BLINK => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_REVERSE => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_CONCEAL => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_STRIKE => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_FONT => VTERM_VALUETYPE_INT,
        VTERM_ATTR_FOREGROUND => VTERM_VALUETYPE_COLOR,
        VTERM_ATTR_BACKGROUND => VTERM_VALUETYPE_COLOR,
        VTERM_ATTR_SMALL => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_BASELINE => VTERM_VALUETYPE_INT,
        VTERM_ATTR_URI => VTERM_VALUETYPE_INT,
        VTERM_ATTR_DIM => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_OVERLINE => VTERM_VALUETYPE_BOOL,
        _ => 0,
    }
}

pub unsafe fn vterm_scroll_rect(
    mut rect: VTermRect,
    downward: c_int,
    rightward: c_int,
    moverect: Option<unsafe fn(src: VTermRect, dest: VTermRect, user: *mut VTermScreen) -> c_int>,
    eraserect: Option<
        unsafe fn(rect: VTermRect, selective: c_int, user: *mut VTermScreen) -> c_int,
    >,
    user: *mut VTermScreen,
) {
    unsafe {
        if downward.abs() >= rect.end_row - rect.start_row
            || rightward.abs() >= rect.end_col - rect.start_col
        {
            if let Some(erase_cb) = eraserect {
                let _ = erase_cb(rect, 0, user);
            }
            return;
        }

        let (src_start_col, src_end_col, dest_start_col, dest_end_col) = if rightward >= 0 {
            (
                rect.start_col + rightward,
                rect.end_col,
                rect.start_col,
                rect.end_col - rightward,
            )
        } else {
            let leftward = -rightward;
            (
                rect.start_col,
                rect.end_col - leftward,
                rect.start_col + leftward,
                rect.end_col,
            )
        };

        let (src_start_row, src_end_row, dest_start_row, dest_end_row) = if downward >= 0 {
            (
                rect.start_row + downward,
                rect.end_row,
                rect.start_row,
                rect.end_row - downward,
            )
        } else {
            let upward = -downward;
            (
                rect.start_row,
                rect.end_row - upward,
                rect.start_row + upward,
                rect.end_row,
            )
        };

        let src = VTermRect {
            start_row: src_start_row,
            end_row: src_end_row,
            start_col: src_start_col,
            end_col: src_end_col,
        };
        let dest = VTermRect {
            start_row: dest_start_row,
            end_row: dest_end_row,
            start_col: dest_start_col,
            end_col: dest_end_col,
        };

        if let Some(move_cb) = moverect {
            let _ = move_cb(dest, src, user);
        }

        if downward > 0 {
            rect.start_row = rect.end_row - downward;
        } else if downward < 0 {
            rect.end_row = rect.start_row - downward;
        }

        if rightward > 0 {
            rect.start_col = rect.end_col - rightward;
        } else if rightward < 0 {
            rect.end_col = rect.start_col - rightward;
        }

        if let Some(erase_cb) = eraserect {
            let _ = erase_cb(rect, 0, user);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vterm_get_attr_type_maps_attributes() {
        use crate::vterm::vterm_test::VTERM_N_ATTRS;
        assert_eq!(vterm_get_attr_type(VTERM_ATTR_BOLD), VTERM_VALUETYPE_BOOL);
        assert_eq!(
            vterm_get_attr_type(VTERM_ATTR_UNDERLINE),
            VTERM_VALUETYPE_INT
        );
        assert_eq!(
            vterm_get_attr_type(VTERM_ATTR_FOREGROUND),
            VTERM_VALUETYPE_COLOR
        );
        assert_eq!(vterm_get_attr_type(VTERM_N_ATTRS), 0);
    }
}
