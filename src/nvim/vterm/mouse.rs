//! VTerm mouse event encoding (`src/nvim/vterm/mouse.c`).

use std::ffi::c_char;

use crate::math::xctz;
use crate::terminal::Terminal;
use crate::tui::termkey::termkey::fill_utf8;
use crate::vterm::keyboard::{C1_CSI, VTermModifier, VTermStateFull};
use crate::vterm::vterm::{VTermOutputCallback, vterm_push_output_ctrl_bytes};

const CSI_ARGS_MAX: usize = 32;
pub(crate) const CSI_LEADER_MAX: usize = 16;
pub(crate) const INTERMED_MAX: usize = 16;

pub(crate) const MOUSE_WANT_DRAG: i32 = 0x02;
pub(crate) const MOUSE_WANT_MOVE: i32 = 0x04;

pub(crate) const MOUSE_X10: i32 = 0;
pub(crate) const MOUSE_UTF8: i32 = 1;
pub(crate) const MOUSE_SGR: i32 = 2;
pub(crate) const MOUSE_RXVT: i32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermPos {
    pub row: i32,
    pub col: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParserCsi {
    pub leaderlen: i32,
    pub leader: [c_char; CSI_LEADER_MAX],
    pub argi: i32,
    pub args: [i64; CSI_ARGS_MAX],
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
    pub command: i32,
}

impl VTermParserOsc {
    pub const fn new() -> Self {
        Self { command: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermParserDcs {
    pub commandlen: i32,
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
    pub state: i32,
    pub in_esc: u8,
    pub intermedlen: i32,
    pub intermed: [c_char; INTERMED_MAX],
    pub csi: VTermParserCsi,
    pub osc: VTermParserOsc,
    pub dcs: VTermParserDcs,
    pub callbacks: Option<&'static crate::vterm::parser::VTermParserCallbacks>,
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
            callbacks: None,
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
    pub rows: i32,
    pub cols: i32,
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

fn output_mouse(
    state: &mut VTermStateFull,
    mut code: i32,
    pressed: i32,
    mut modifiers: i32,
    mut col: i32,
    mut row: i32,
) {
    {
        modifiers <<= 2;

        match state.mouse_protocol {
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
                vterm_push_output_ctrl_bytes(state.vt_mut(), C1_CSI, &payload);
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
                    utf8.as_mut_ptr().wrapping_add(len).cast::<c_char>(),
                ) as usize;
                len += fill_utf8(
                    col + 0x21,
                    utf8.as_mut_ptr().wrapping_add(len).cast::<c_char>(),
                ) as usize;
                len += fill_utf8(
                    row + 0x21,
                    utf8.as_mut_ptr().wrapping_add(len).cast::<c_char>(),
                ) as usize;

                vterm_push_output_ctrl_bytes(state.vt_mut(), C1_CSI, &utf8[..len]);
            }
            MOUSE_SGR => {
                let ch = if pressed != 0 { 'M' } else { 'm' };
                let s = format!("<{};{};{}{ch}", code | modifiers, col + 1, row + 1);
                vterm_push_output_ctrl_bytes(state.vt_mut(), C1_CSI, s.as_bytes());
            }
            MOUSE_RXVT => {
                if pressed == 0 {
                    code = 3;
                }

                let s = format!("{};{};{}M", code | modifiers, col + 1, row + 1);
                vterm_push_output_ctrl_bytes(state.vt_mut(), C1_CSI, s.as_bytes());
            }
            _ => {}
        }
    }
}

pub fn vterm_mouse_move(vt: &mut VTerm, row: i32, col: i32, mod_: VTermModifier) {
    unsafe {
        let state = &mut *vt.state_mut_ptr();

        if col == state.mouse_col && row == state.mouse_row {
            return;
        }

        state.mouse_col = col;
        state.mouse_row = row;

        if ((state.mouse_flags & MOUSE_WANT_DRAG) != 0 && state.mouse_buttons != 0)
            || (state.mouse_flags & MOUSE_WANT_MOVE) != 0
        {
            if state.mouse_buttons != 0 {
                let button = xctz(state.mouse_buttons as u64) + 1;
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

pub fn vterm_mouse_button(vt: &mut VTerm, button: i32, pressed: bool, mod_: VTermModifier) {
    unsafe {
        let state = &mut *vt.state_mut_ptr();

        let old_buttons = state.mouse_buttons;

        if (1..=3).contains(&button) || (8..=11).contains(&button) {
            if pressed {
                state.mouse_buttons |= 1 << (button - 1);
            } else {
                state.mouse_buttons &= !(1 << (button - 1));
            }
        }

        if state.mouse_buttons == old_buttons && !(4..=7).contains(&button) {
            return;
        }

        if state.mouse_flags == 0 {
            return;
        }

        if button < 4 {
            output_mouse(
                state,
                button - 1,
                i32::from(pressed),
                mod_,
                state.mouse_col,
                state.mouse_row,
            );
        } else if button < 8 {
            output_mouse(
                state,
                button - 4 + 0x40,
                i32::from(pressed),
                mod_,
                state.mouse_col,
                state.mouse_row,
            );
        } else if button < 12 {
            output_mouse(
                state,
                button - 8 + 0x80,
                i32::from(pressed),
                mod_,
                state.mouse_col,
                state.mouse_row,
            );
        }
    }
}
