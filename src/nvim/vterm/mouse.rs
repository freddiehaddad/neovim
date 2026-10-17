//! VTerm mouse event encoding (`src/nvim/vterm/mouse.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

use std::ffi::{c_char, c_int, c_long, c_void};

use crate::math::xctz;
use crate::tui::termkey::termkey::fill_utf8;

pub type VTermModifier = c_int;

const C1_CSI: u8 = 0x9b;
const CSI_ARGS_MAX: usize = 32;
const CSI_LEADER_MAX: usize = 16;
const INTERMED_MAX: usize = 16;

const MOUSE_WANT_CLICK: c_int = 0x01;
const MOUSE_WANT_DRAG: c_int = 0x02;
const MOUSE_WANT_MOVE: c_int = 0x04;

const MOUSE_X10: c_int = 0;
const MOUSE_UTF8: c_int = 1;
const MOUSE_SGR: c_int = 2;
const MOUSE_RXVT: c_int = 3;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermPos {
    pub row: c_int,
    pub col: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParserCsi {
    pub leaderlen: c_int,
    pub leader: [c_char; CSI_LEADER_MAX],
    pub argi: c_int,
    pub args: [c_long; CSI_ARGS_MAX],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParserOsc {
    pub command: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParserDcs {
    pub commandlen: c_int,
    pub command: [c_char; CSI_LEADER_MAX],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union VTermParserV {
    pub csi: VTermParserCsi,
    pub osc: VTermParserOsc,
    pub dcs: VTermParserDcs,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParser {
    pub state: c_int,
    pub in_esc: u8,
    pub intermedlen: c_int,
    pub intermed: [c_char; INTERMED_MAX],
    pub v: VTermParserV,
    pub callbacks: *const c_void,
    pub cbdata: *mut c_void,
    pub string_initial: bool,
    pub emit_nul: bool,
}

#[repr(C)]
pub struct VTerm {
    pub allocator: *const c_void,
    pub allocdata: *mut c_void,
    pub rows: c_int,
    pub cols: c_int,
    pub mode: u32,
    pub parser: VTermParser,
    pub outfunc: *mut c_void,
    pub outdata: *mut c_void,
    pub outbuffer: *mut c_char,
    pub outbuffer_len: usize,
    pub outbuffer_cur: usize,
    pub tmpbuffer: *mut c_char,
    pub tmpbuffer_len: usize,
    pub state: *mut VTermState,
    pub screen: *mut c_void,
}

#[repr(C)]
pub struct VTermState {
    pub vt: *mut VTerm,
    pub callbacks: *const c_void,
    pub cbdata: *mut c_void,
    pub fallbacks: *const c_void,
    pub fbdata: *mut c_void,
    pub rows: c_int,
    pub cols: c_int,
    pub pos: VTermPos,
    pub at_phantom: c_int,
    pub scrollregion_top: c_int,
    pub scrollregion_bottom: c_int,
    pub scrollregion_left: c_int,
    pub scrollregion_right: c_int,
    pub tabstops: *mut u8,
    pub lineinfos: [*mut c_void; 2],
    pub lineinfo: *mut c_void,
    pub mouse_col: c_int,
    pub mouse_row: c_int,
    pub mouse_buttons: c_int,
    pub mouse_flags: c_int,
    pub mouse_protocol: c_int,
}

unsafe extern "C" {
    fn vterm_push_output_sprintf_ctrl(vt: *mut VTerm, ctrl: u8, fmt: *const c_char, ...);
}

unsafe fn output_mouse(
    state: *mut VTermState,
    mut code: c_int,
    pressed: c_int,
    mut modifiers: c_int,
    mut col: c_int,
    mut row: c_int,
) {
    unsafe {
        modifiers <<= 2;

        match (*state).mouse_protocol {
            MOUSE_X10 => {
                if col + 0x21 > 0xff {
                    col = 0xff - 0x21;
                }
                if row + 0x21 > 0xff {
                    row = 0xff - 0x21;
                }

                if pressed == 0 {
                    code = 3;
                }

                if (code & 0x80) != 0 {
                    return;
                }
                vterm_push_output_sprintf_ctrl(
                    (*state).vt,
                    C1_CSI,
                    c"M%c%c%c".as_ptr(),
                    (code | modifiers) + 0x20,
                    col + 0x21,
                    row + 0x21,
                );
            }
            MOUSE_UTF8 => {
                let mut utf8 = [0 as c_char; 18];
                let mut len: usize = 0;

                if pressed == 0 {
                    code = 3;
                }

                len += fill_utf8((code | modifiers) + 0x20, utf8.as_mut_ptr().add(len)) as usize;
                len += fill_utf8(col + 0x21, utf8.as_mut_ptr().add(len)) as usize;
                len += fill_utf8(row + 0x21, utf8.as_mut_ptr().add(len)) as usize;
                utf8[len] = 0;

                vterm_push_output_sprintf_ctrl((*state).vt, C1_CSI, c"M%s".as_ptr(), utf8.as_ptr());
            }
            MOUSE_SGR => {
                vterm_push_output_sprintf_ctrl(
                    (*state).vt,
                    C1_CSI,
                    c"<%d;%d;%d%c".as_ptr(),
                    code | modifiers,
                    col + 1,
                    row + 1,
                    if pressed != 0 {
                        b'M' as c_int
                    } else {
                        b'm' as c_int
                    },
                );
            }
            MOUSE_RXVT => {
                if pressed == 0 {
                    code = 3;
                }

                vterm_push_output_sprintf_ctrl(
                    (*state).vt,
                    C1_CSI,
                    c"%d;%d;%dM".as_ptr(),
                    code | modifiers,
                    col + 1,
                    row + 1,
                );
            }
            _ => {}
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_mouse_move(
    vt: *mut VTerm,
    row: c_int,
    col: c_int,
    mod_: VTermModifier,
) {
    unsafe {
        let state = (*vt).state;

        if col == (*state).mouse_col && row == (*state).mouse_row {
            return;
        }

        (*state).mouse_col = col;
        (*state).mouse_row = row;

        if (((*state).mouse_flags & MOUSE_WANT_DRAG) != 0 && (*state).mouse_buttons != 0)
            || ((*state).mouse_flags & MOUSE_WANT_MOVE) != 0
        {
            if (*state).mouse_buttons != 0 {
                let button = xctz((*state).mouse_buttons as u64) + 1;
                if button < 4 {
                    output_mouse(state, button - 1 + 0x20, 1, mod_, col, row);
                } else if (8..12).contains(&button) {
                    output_mouse(state, button - 8 + 0x80 + 0x20, 1, mod_, col, row);
                }
            } else {
                output_mouse(state, 3 + 0x20, 1, mod_, col, row);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_mouse_button(
    vt: *mut VTerm,
    button: c_int,
    pressed: bool,
    mod_: VTermModifier,
) {
    unsafe {
        let state = (*vt).state;

        let old_buttons = (*state).mouse_buttons;

        if (1..=3).contains(&button) || (8..=11).contains(&button) {
            if pressed {
                (*state).mouse_buttons |= 1 << (button - 1);
            } else {
                (*state).mouse_buttons &= !(1 << (button - 1));
            }
        }

        if (*state).mouse_buttons == old_buttons && !(4..=7).contains(&button) {
            return;
        }

        if (*state).mouse_flags == 0 {
            return;
        }

        if button < 4 {
            output_mouse(
                state,
                button - 1,
                c_int::from(pressed),
                mod_,
                (*state).mouse_col,
                (*state).mouse_row,
            );
        } else if button < 8 {
            output_mouse(
                state,
                button - 4 + 0x40,
                c_int::from(pressed),
                mod_,
                (*state).mouse_col,
                (*state).mouse_row,
            );
        } else if button < 12 {
            output_mouse(
                state,
                button - 8 + 0x80,
                c_int::from(pressed),
                mod_,
                (*state).mouse_col,
                (*state).mouse_row,
            );
        }
    }
}
