//! VimL parser input reader and state lifecycle (`src/nvim/viml/parser/parser.c`).

use crate::mbyte::{string_convert, vimconv_T};
use crate::memory::xfree;
use std::ffi::{c_char, c_int};

pub const CONV_NONE: c_int = 0;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ParserLine {
    pub data: *const c_char,
    pub size: usize,
    pub allocated: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ParserPosition {
    pub line: usize,
    pub col: usize,
}

impl ParserPosition {
    pub const fn new() -> Self {
        Self { line: 0, col: 0 }
    }
}

pub struct ParserInputReader {
    pub cursor: *const ParserLine,
    pub lines: Vec<ParserLine>,
    pub conv: vimconv_T,
}

impl ParserInputReader {
    pub const fn new(cursor: *const ParserLine) -> Self {
        Self {
            cursor,
            lines: Vec::new(),
            conv: vimconv_T {
                vc_type: CONV_NONE,
                vc_factor: 1,
                vc_fd: std::ptr::null_mut(),
                vc_fail: false,
            },
        }
    }
}

impl Drop for ParserInputReader {
    fn drop(&mut self) {
        for pline in self.lines.drain(..) {
            if pline.allocated {
                unsafe {
                    xfree(pline.data.cast_mut().cast());
                }
            }
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ParserHighlightChunk {
    pub start: ParserPosition,
    pub end_col: usize,
    pub group: *const c_char,
}

pub type ParserHighlight = Vec<ParserHighlightChunk>;

pub struct ParserState {
    pub reader: ParserInputReader,
    pub pos: ParserPosition,
    pub colors: *mut ParserHighlight,
}

unsafe fn viml_preader_get_line(preader: &mut ParserInputReader, ret_pline: &mut ParserLine) {
    let mut pline = if !preader.cursor.is_null() {
        unsafe {
            let p = *preader.cursor;
            preader.cursor = preader.cursor.add(1);
            p
        }
    } else {
        ParserLine {
            data: std::ptr::null(),
            size: 0,
            allocated: false,
        }
    };
    if preader.conv.vc_type != CONV_NONE && pline.size != 0 {
        let mut cpline = ParserLine {
            data: std::ptr::null(),
            size: pline.size,
            allocated: true,
        };
        cpline.data =
            unsafe { string_convert(&preader.conv, pline.data.cast_mut(), &mut cpline.size) };
        if pline.allocated {
            unsafe {
                xfree(pline.data.cast_mut().cast());
            }
        }
        pline = cpline;
    }
    preader.lines.push(pline);
    *ret_pline = pline;
}

/// Get the currently parsed line shifted to `pstate->pos.col` (`viml_parser_get_remaining_line`).
///
/// # Safety
/// `pstate` and `ret_pline` must be valid writable pointers.
pub unsafe fn viml_parser_get_remaining_line(
    pstate: *mut ParserState,
    ret_pline: *mut ParserLine,
) -> bool {
    let pstate = unsafe { &mut *pstate };
    let ret_pline = unsafe { &mut *ret_pline };

    let num_lines = pstate.reader.lines.len();
    if pstate.pos.line == num_lines {
        unsafe {
            viml_preader_get_line(&mut pstate.reader, ret_pline);
        }
    } else {
        *ret_pline = *pstate.reader.lines.last().unwrap();
    }
    assert_eq!(pstate.pos.line, pstate.reader.lines.len() - 1);
    if !ret_pline.data.is_null() {
        ret_pline.data = unsafe { ret_pline.data.add(pstate.pos.col) };
        ret_pline.size -= pstate.pos.col;
    }
    !ret_pline.data.is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preader_get_line_advances_cursor() {
        let lines = [
            ParserLine {
                data: c"let x = 1".as_ptr(),
                size: 9,
                allocated: false,
            },
            ParserLine {
                data: std::ptr::null(),
                size: 0,
                allocated: false,
            },
        ];
        let mut reader = ParserInputReader::new(lines.as_ptr());
        let mut out = ParserLine {
            data: std::ptr::null(),
            size: 0,
            allocated: false,
        };
        unsafe {
            viml_preader_get_line(&mut reader, &mut out);
            assert_eq!(out.size, 9);
            viml_preader_get_line(&mut reader, &mut out);
            assert!(out.data.is_null());
        }
    }
}
