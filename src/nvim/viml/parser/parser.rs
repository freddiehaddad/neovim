//! VimL parser input reader and state lifecycle (`src/nvim/viml/parser/parser.c`).

use crate::mbyte::{Vimconv, string_convert_slice};

pub const CONV_NONE: i32 = 0;

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct ParserLine {
    pub data: Option<Vec<u8>>,
    pub offset: usize,
}

impl ParserLine {
    pub const NONE: Self = Self {
        data: None,
        offset: 0,
    };

    #[inline]
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            data: Some(data.to_vec()),
            offset: 0,
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self.data {
            Some(ref d) => d.get(self.offset..).unwrap_or(&[]),
            None => &[],
        }
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.as_bytes().len()
    }
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

pub struct ParserInputReader<'a> {
    pub cursor: &'a [ParserLine],
    pub lines: Vec<ParserLine>,
    pub owned_lines: Vec<Vec<u8>>,
    pub conv: Vimconv,
}

impl<'a> ParserInputReader<'a> {
    pub const fn new(cursor: &'a [ParserLine]) -> Self {
        Self {
            cursor,
            lines: Vec::new(),
            owned_lines: Vec::new(),
            conv: Vimconv {
                vc_type: CONV_NONE,
                vc_factor: 1,
                vc_fd: None,
                vc_fail: false,
            },
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ParserHighlightChunk {
    pub start: ParserPosition,
    pub end_col: usize,
    pub group: &'static std::ffi::CStr,
}

pub type ParserHighlight = Vec<ParserHighlightChunk>;

pub struct ParserState<'a> {
    pub reader: ParserInputReader<'a>,
    pub pos: ParserPosition,
    pub colors: Option<ParserHighlight>,
}

fn viml_preader_get_line(preader: &mut ParserInputReader<'_>, ret_pline: &mut ParserLine) {
    let mut pline = if let Some((first, rest)) = preader.cursor.split_first() {
        preader.cursor = rest;
        first.clone()
    } else {
        ParserLine::NONE
    };
    if preader.conv.vc_type != CONV_NONE && pline.size() != 0 {
        let mut csize = pline.size();
        let mut cpline = ParserLine::NONE;
        if let Some(conv_bytes) = string_convert_slice(&preader.conv, pline.as_bytes(), &mut csize)
        {
            cpline = ParserLine::from_slice(&conv_bytes[..csize.min(conv_bytes.len())]);
            preader.owned_lines.push(conv_bytes);
        }
        pline = cpline;
    }
    preader.lines.push(pline.clone());
    *ret_pline = pline;
}

/// Get the currently parsed line shifted to `pstate.pos.col` (`viml_parser_get_remaining_line`).
pub fn viml_parser_get_remaining_line(
    pstate: &mut ParserState<'_>,
    ret_pline: &mut ParserLine,
) -> bool {
    let num_lines = pstate.reader.lines.len();
    if pstate.pos.line == num_lines {
        viml_preader_get_line(&mut pstate.reader, ret_pline);
    } else {
        *ret_pline = pstate.reader.lines.last().unwrap().clone();
    }
    assert_eq!(pstate.pos.line, pstate.reader.lines.len() - 1);
    if ret_pline.data.is_some() {
        ret_pline.offset += pstate.pos.col;
    }
    ret_pline.data.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preader_get_line_advances_cursor() {
        let lines = [ParserLine::from_slice(b"let x = 1"), ParserLine::NONE];
        let mut reader = ParserInputReader::new(&lines);
        let mut out = ParserLine::NONE;
        viml_preader_get_line(&mut reader, &mut out);
        assert_eq!(out.size(), 9);
        viml_preader_get_line(&mut reader, &mut out);
        assert!(out.data.is_none());
    }
}
