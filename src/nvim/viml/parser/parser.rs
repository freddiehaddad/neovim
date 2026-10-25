//! VimL parser input reader and state lifecycle (`src/nvim/viml/parser/parser.c`).

use crate::mbyte::{string_convert, vimconv_T};
use crate::memory::xfree;
use std::ffi::{c_char, c_int, c_void};

pub const CONV_NONE: c_int = 0;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ParserLine {
    pub data: *const c_char,
    pub size: usize,
    pub allocated: bool,
}

pub type ParserLineGetter =
    Option<unsafe fn(cookie: *mut *const ParserLine, ret_pline: *mut ParserLine)>;

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
    pub get_line: ParserLineGetter,
    pub cookie: *mut *const ParserLine,
    pub lines: Vec<ParserLine>,
    pub conv: vimconv_T,
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

/// Simple line getter reading from an array of `ParserLine` (`parser_simple_get_line`).
///
/// # Safety
/// `cookie` must point to a `*const ParserLine` cursor, and `ret_pline` must be writable.
pub unsafe fn parser_simple_get_line(
    plines_p: *mut *const ParserLine,
    ret_pline: *mut ParserLine,
) {
    unsafe {
        *ret_pline = **plines_p;
        *plines_p = (*plines_p).add(1);
    }
}

unsafe fn viml_preader_get_line(preader: &mut ParserInputReader, ret_pline: &mut ParserLine) {
    let mut pline = ParserLine {
        data: std::ptr::null(),
        size: 0,
        allocated: false,
    };
    if let Some(get_line) = preader.get_line {
        unsafe {
            get_line(preader.cookie, &mut pline);
        }
    }
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
                xfree(pline.data.cast_mut().cast::<c_void>());
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

/// Free all heap memory allocated by `pstate` (`viml_parser_destroy`).
///
/// # Safety
/// `pstate` must be a valid writable pointer to an initialized `ParserState`.
pub unsafe fn viml_parser_destroy(pstate: *mut ParserState) {
    let pstate = unsafe { &mut *pstate };
    for pline in pstate.reader.lines.drain(..) {
        if pline.allocated {
            unsafe {
                xfree(pline.data.cast_mut().cast::<c_void>());
            }
        }
    }
    pstate.reader.lines = Vec::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_get_line_advances_cursor() {
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
        let mut cursor = lines.as_ptr();
        let mut out = ParserLine {
            data: std::ptr::null(),
            size: 0,
            allocated: false,
        };
        unsafe {
            parser_simple_get_line(&mut cursor, &mut out);
            assert_eq!(out.size, 9);
            parser_simple_get_line(&mut cursor, &mut out);
            assert!(out.data.is_null());
        }
    }
}
