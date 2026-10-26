//! VTerm mouse event encoding (`src/nvim/vterm/mouse.c`).

use std::ffi::{c_char, c_int, c_long};

use crate::math::xctz;
use crate::terminal::Terminal;
use crate::tui::termkey::termkey::fill_utf8;
use crate::vterm::keyboard::{C1_CSI, VTermModifier, VTermStateFull};
use crate::vterm::vterm::{VTermOutputCallback, vterm_push_output_ctrl_bytes};

const CSI_ARGS_MAX: usize = 32;
pub(crate) const CSI_LEADER_MAX: usize = 16;
pub(crate) const INTERMED_MAX: usize = 16;

pub(crate) const MOUSE_WANT_DRAG: c_int = 0x02;
pub(crate) const MOUSE_WANT_MOVE: c_int = 0x04;

pub(crate) const MOUSE_X10: c_int = 0;
pub(crate) const MOUSE_UTF8: c_int = 1;
pub(crate) const MOUSE_SGR: c_int = 2;
pub(crate) const MOUSE_RXVT: c_int = 3;

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

impl VTermParserCsi {
    pub const fn new() -> Self {
        Self {
            leaderlen: 0,
            leader: [0; CSI_LEADER_MAX],
            argi: 0,
            args: [0; CSI_ARGS_MAX],
        }
    }
}

impl Default for VTermParserCsi {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermParserOsc {
    pub command: c_int,
}

impl VTermParserOsc {
    pub const fn new() -> Self {
        Self { command: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParserDcs {
    pub commandlen: c_int,
    pub command: [c_char; CSI_LEADER_MAX],
}

impl VTermParserDcs {
    pub const fn new() -> Self {
        Self {
            commandlen: 0,
            command: [0; CSI_LEADER_MAX],
        }
    }
}

impl Default for VTermParserDcs {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParser {
    pub state: c_int,
    pub in_esc: u8,
    pub intermedlen: c_int,
    pub intermed: [c_char; INTERMED_MAX],
    pub csi: VTermParserCsi,
    pub osc: VTermParserOsc,
    pub dcs: VTermParserDcs,
    pub callbacks: *const crate::vterm::parser::VTermParserCallbacks,
    pub cbdata: *mut VTermStateFull,
    pub string_initial: bool,
    pub emit_nul: bool,
}

impl VTermParser {
    pub const fn new() -> Self {
        Self {
            state: 0,
            in_esc: 0,
            intermedlen: 0,
            intermed: [0; INTERMED_MAX],
            csi: VTermParserCsi::new(),
            osc: VTermParserOsc::new(),
            dcs: VTermParserDcs::new(),
            callbacks: std::ptr::null(),
            cbdata: std::ptr::null_mut(),
            string_initial: false,
            emit_nul: false,
        }
    }
}

impl Default for VTermParser {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct VTerm {
    pub rows: c_int,
    pub cols: c_int,
    pub mode: u32,
    pub parser: VTermParser,
    pub outfunc: VTermOutputCallback,
    pub outdata: *mut Terminal,
    pub screen: Option<Box<crate::vterm::screen::VTermScreen>>,
    pub state: Option<Box<VTermStateFull>>,
}

impl VTerm {
    pub const fn new() -> Self {
        Self {
            rows: 0,
            cols: 0,
            mode: 0,
            parser: VTermParser::new(),
            outfunc: None,
            outdata: std::ptr::null_mut(),
            screen: None,
            state: None,
        }
    }

    #[inline]
    pub fn state_mut_ptr(&mut self) -> *mut VTermStateFull {
        self.state
            .as_deref_mut()
            .map_or(std::ptr::null_mut(), |s| &raw mut *s)
    }

    #[inline]
    pub fn state_ptr(&self) -> *const VTermStateFull {
        self.state
            .as_deref()
            .map_or(std::ptr::null(), |s| &raw const *s)
    }

    #[inline]
    pub fn screen_mut_ptr(&mut self) -> *mut crate::vterm::screen::VTermScreen {
        self.screen
            .as_deref_mut()
            .map_or(std::ptr::null_mut(), |s| &raw mut *s)
    }
}

impl Default for VTerm {
    fn default() -> Self {
        Self::new()
    }
}

unsafe fn output_mouse(
    state: *mut VTermStateFull,
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
                let payload = [
                    b'M',
                    ((code | modifiers) + 0x20) as u8,
                    (col + 0x21) as u8,
                    (row + 0x21) as u8,
                ];
                vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, &payload);
            }
            MOUSE_UTF8 => {
                let mut utf8 = [0u8; 19];
                utf8[0] = b'M';
                let mut len: usize = 1;

                if pressed == 0 {
                    code = 3;
                }

                len += fill_utf8(
                    (code | modifiers) + 0x20,
                    utf8.as_mut_ptr().add(len).cast::<c_char>(),
                ) as usize;
                len += fill_utf8(col + 0x21, utf8.as_mut_ptr().add(len).cast::<c_char>()) as usize;
                len += fill_utf8(row + 0x21, utf8.as_mut_ptr().add(len).cast::<c_char>()) as usize;

                vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, &utf8[..len]);
            }
            MOUSE_SGR => {
                let ch = if pressed != 0 { 'M' } else { 'm' };
                let s = format!("<{};{};{}{ch}", code | modifiers, col + 1, row + 1);
                vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s.as_bytes());
            }
            MOUSE_RXVT => {
                if pressed == 0 {
                    code = 3;
                }

                let s = format!("{};{};{}M", code | modifiers, col + 1, row + 1);
                vterm_push_output_ctrl_bytes((*state).vt, C1_CSI, s.as_bytes());
            }
            _ => {}
        }
    }
}

pub unsafe fn vterm_mouse_move(vt: *mut VTerm, row: c_int, col: c_int, mod_: VTermModifier) {
    unsafe {
        let state = (*vt).state_mut_ptr();

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

pub unsafe fn vterm_mouse_button(
    vt: *mut VTerm,
    button: c_int,
    pressed: bool,
    mod_: VTermModifier,
) {
    unsafe {
        let state = (*vt).state_mut_ptr();

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
