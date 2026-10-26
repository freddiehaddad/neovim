//! VTerm keyboard event encoding (`src/nvim/vterm/keyboard.c`).

use crate::input::NUL;
use std::ffi::{c_char, c_int};

use crate::terminal::Terminal;
use crate::tui::termkey::termkey::fill_utf8;
pub use crate::vterm::encoding::VTermEncodingInstance;
use crate::vterm::mouse::{VTerm, VTermPos};
use crate::vterm::screen::VTermScreen;
use crate::vterm::vterm::{
    vterm_push_output_bytes, vterm_push_output_bytes_slice, vterm_push_output_ctrl_bytes,
};

pub type VTermModifier = c_int;
pub type VTermKey = c_int;

const C1_SS3: u8 = 0x8f;
pub(crate) const C1_CSI: u8 = 0x9b;

pub(crate) const BUFIDX_PRIMARY: usize = 0;
pub(crate) const BUFIDX_ALTSCREEN: usize = 1;

pub(crate) const VTERM_MOD_NONE: c_int = 0x00;
pub(crate) const VTERM_MOD_SHIFT: c_int = 0x01;
pub(crate) const VTERM_MOD_ALT: c_int = 0x02;
pub(crate) const VTERM_MOD_CTRL: c_int = 0x04;

pub(crate) const VTERM_KEY_NONE: c_int = 0;
pub(crate) const VTERM_KEY_ENTER: c_int = 1;
pub(crate) const VTERM_KEY_TAB: c_int = 2;
pub(crate) const VTERM_KEY_BACKSPACE: c_int = 3;
pub(crate) const VTERM_KEY_FUNCTION_0: c_int = 256;
const VTERM_KEY_FUNCTION_MAX: c_int = VTERM_KEY_FUNCTION_0 + 255;
pub(crate) const VTERM_KEY_KP_0: c_int = VTERM_KEY_FUNCTION_MAX + 1;

const MODE_KEYPAD_BIT: u32 = 1 << 0;
const MODE_CURSOR_BIT: u32 = 1 << 1;
const MODE_NEWLINE_BIT: u32 = 1 << 4;
const MODE_ALT_SCREEN_BIT: u32 = 1 << 9;
const MODE_BRACKETPASTE_BIT: u32 = 1 << 13;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermColor {
    pub bytes: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermPen {
    pub fg: VTermColor,
    pub bg: VTermColor,
    pub uri: c_int,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermStateSaved {
    pub pos: VTermPos,
    pub pen: VTermPen,
    pub mode: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermStateTmp {
    pub words: [u32; 3],
}

#[repr(C)]
pub struct VTermStateSelection {
    pub callbacks: *const crate::vterm::state::VTermSelectionCallbacks,
    pub user: *mut Terminal,
    pub buffer: Vec<c_char>,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermKeyEncodingStack {
    pub items: [u8; 16],
    pub size: u8,
}

#[repr(C)]
pub struct VTermStateFull {
    pub vt: *mut VTerm,
    pub callbacks: *const crate::vterm::pen::VTermStateCallbacks,
    pub cbdata: *mut VTermScreen,
    pub fallbacks: *const crate::vterm::state::VTermStateFallbacks,
    pub fbdata: *mut Terminal,
    pub rows: c_int,
    pub cols: c_int,
    pub pos: VTermPos,
    pub at_phantom: c_int,
    pub scrollregion_top: c_int,
    pub scrollregion_bottom: c_int,
    pub scrollregion_left: c_int,
    pub scrollregion_right: c_int,
    pub tabstops: Vec<u8>,
    pub lineinfos: [Vec<crate::vterm::screen::VTermLineInfo>; 2],
    pub mouse_col: c_int,
    pub mouse_row: c_int,
    pub mouse_buttons: c_int,
    pub mouse_flags: c_int,
    pub mouse_protocol: c_int,
    pub grapheme_buf: [c_char; 32],
    pub grapheme_len: usize,
    pub grapheme_last: u32,
    pub grapheme_state: i32,
    pub combine_width: c_int,
    pub combine_pos: VTermPos,
    pub mode: u32,
    pub encoding: [VTermEncodingInstance; 4],
    pub encoding_utf8: VTermEncodingInstance,
    pub gl_set: c_int,
    pub gr_set: c_int,
    pub gsingle_set: c_int,
    pub pen: VTermPen,
    pub default_fg: VTermColor,
    pub default_bg: VTermColor,
    pub colors: [VTermColor; 16],
    pub bold_is_highbright: c_int,
    pub protected_cell: u32,
    pub saved: VTermStateSaved,
    pub tmp: VTermStateTmp,
    pub selection: VTermStateSelection,
    pub key_encoding_stacks: [VTermKeyEncodingStack; 2],
}

impl VTermStateFull {
    #[inline]
    pub fn lineinfo(&self) -> &[crate::vterm::screen::VTermLineInfo] {
        let idx = if (self.mode & MODE_ALT_SCREEN_BIT) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };
        &self.lineinfos[idx]
    }

    #[inline]
    pub fn lineinfo_mut(&mut self) -> &mut [crate::vterm::screen::VTermLineInfo] {
        let idx = if (self.mode & MODE_ALT_SCREEN_BIT) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };
        &mut self.lineinfos[idx]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeycodeType {
    None,
    Literal,
    Tab,
    Enter,
    Ss3,
    CsiCursor,
    CsiNum,
    Keypad,
}

#[derive(Clone, Copy)]
struct KeycodeEntry {
    r#type: KeycodeType,
    literal: c_int,
    csinum: c_int,
}

const KEYCODES: [KeycodeEntry; 15] = [
    KeycodeEntry {
        r#type: KeycodeType::None,
        literal: NUL,
        csinum: 0,
    }, // NONE
    KeycodeEntry {
        r#type: KeycodeType::Enter,
        literal: b'\r' as c_int,
        csinum: 0,
    }, // ENTER
    KeycodeEntry {
        r#type: KeycodeType::Tab,
        literal: b'\t' as c_int,
        csinum: 0,
    }, // TAB
    KeycodeEntry {
        r#type: KeycodeType::Literal,
        literal: 0x7f,
        csinum: 0,
    }, // BACKSPACE
    KeycodeEntry {
        r#type: KeycodeType::Literal,
        literal: 0x1b,
        csinum: 0,
    }, // ESCAPE
    KeycodeEntry {
        r#type: KeycodeType::CsiCursor,
        literal: b'A' as c_int,
        csinum: 0,
    }, // UP
    KeycodeEntry {
        r#type: KeycodeType::CsiCursor,
        literal: b'B' as c_int,
        csinum: 0,
    }, // DOWN
    KeycodeEntry {
        r#type: KeycodeType::CsiCursor,
        literal: b'D' as c_int,
        csinum: 0,
    }, // LEFT
    KeycodeEntry {
        r#type: KeycodeType::CsiCursor,
        literal: b'C' as c_int,
        csinum: 0,
    }, // RIGHT
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 2,
    }, // INS
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 3,
    }, // DEL
    KeycodeEntry {
        r#type: KeycodeType::CsiCursor,
        literal: b'H' as c_int,
        csinum: 0,
    }, // HOME
    KeycodeEntry {
        r#type: KeycodeType::CsiCursor,
        literal: b'F' as c_int,
        csinum: 0,
    }, // END
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 5,
    }, // PAGEUP
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 6,
    }, // PAGEDOWN
];

const KEYCODES_FN: [KeycodeEntry; 13] = [
    KeycodeEntry {
        r#type: KeycodeType::None,
        literal: NUL,
        csinum: 0,
    }, // F0
    KeycodeEntry {
        r#type: KeycodeType::Ss3,
        literal: b'P' as c_int,
        csinum: 0,
    }, // F1
    KeycodeEntry {
        r#type: KeycodeType::Ss3,
        literal: b'Q' as c_int,
        csinum: 0,
    }, // F2
    KeycodeEntry {
        r#type: KeycodeType::Ss3,
        literal: b'R' as c_int,
        csinum: 0,
    }, // F3
    KeycodeEntry {
        r#type: KeycodeType::Ss3,
        literal: b'S' as c_int,
        csinum: 0,
    }, // F4
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 15,
    }, // F5
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 17,
    }, // F6
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 18,
    }, // F7
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 19,
    }, // F8
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 20,
    }, // F9
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 21,
    }, // F10
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 23,
    }, // F11
    KeycodeEntry {
        r#type: KeycodeType::CsiNum,
        literal: b'~' as c_int,
        csinum: 24,
    }, // F12
];

const KEYCODES_KP: [KeycodeEntry; 18] = [
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'0' as c_int,
        csinum: b'p' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'1' as c_int,
        csinum: b'q' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'2' as c_int,
        csinum: b'r' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'3' as c_int,
        csinum: b's' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'4' as c_int,
        csinum: b't' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'5' as c_int,
        csinum: b'u' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'6' as c_int,
        csinum: b'v' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'7' as c_int,
        csinum: b'w' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'8' as c_int,
        csinum: b'x' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'9' as c_int,
        csinum: b'y' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'*' as c_int,
        csinum: b'j' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'+' as c_int,
        csinum: b'k' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b',' as c_int,
        csinum: b'l' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'-' as c_int,
        csinum: b'm' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'.' as c_int,
        csinum: b'n' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'/' as c_int,
        csinum: b'o' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'\n' as c_int,
        csinum: b'M' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: b'=' as c_int,
        csinum: b'X' as c_int,
    },
];

const KEYCODES_KP_CSIU: [KeycodeEntry; 18] = [
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57399,
        csinum: b'p' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57400,
        csinum: b'q' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57401,
        csinum: b'r' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57402,
        csinum: b's' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57403,
        csinum: b't' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57404,
        csinum: b'u' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57405,
        csinum: b'v' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57406,
        csinum: b'w' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57407,
        csinum: b'x' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57408,
        csinum: b'y' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57411,
        csinum: b'j' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57413,
        csinum: b'k' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57416,
        csinum: b'l' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57412,
        csinum: b'm' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57409,
        csinum: b'n' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57410,
        csinum: b'o' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57414,
        csinum: b'M' as c_int,
    },
    KeycodeEntry {
        r#type: KeycodeType::Keypad,
        literal: 57415,
        csinum: b'X' as c_int,
    },
];

#[inline]
unsafe fn vterm_state_get_key_encoding_disambiguate(state: *const VTermStateFull) -> bool {
    unsafe {
        let screen = if ((*state).mode & MODE_ALT_SCREEN_BIT) != 0 {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };
        let stack = &(*state).key_encoding_stacks[screen];
        debug_assert!(stack.size > 0);
        (stack.items[(stack.size - 1) as usize] & 0x01) != 0
    }
}

pub unsafe fn vterm_keyboard_unichar(vt: *mut VTerm, mut c: u32, mut mod_: VTermModifier) {
    unsafe {
        let passthru = if c == u32::from(b' ') {
            mod_ == VTERM_MOD_NONE
        } else {
            (mod_ & !VTERM_MOD_SHIFT) == 0
        };

        if passthru {
            let mut str_buf = [0 as c_char; 6];
            let seqlen = fill_utf8(c as c_int, str_buf.as_mut_ptr());
            vterm_push_output_bytes(vt, str_buf.as_ptr(), seqlen as usize);
            return;
        }

        let state: *const VTermStateFull = (*vt).state_ptr();
        if vterm_state_get_key_encoding_disambiguate(state) {
            if (u32::from(b'A')..=u32::from(b'Z')).contains(&c) {
                c += u32::from(b'a' - b'A');
                mod_ |= VTERM_MOD_SHIFT;
            }

            let s = format!("{c};{}u", mod_ + 1);
            vterm_push_output_ctrl_bytes(vt, C1_CSI, s.as_bytes());
            return;
        }

        if (mod_ & VTERM_MOD_CTRL) != 0 {
            match c {
                0x32 | 0x20 => {
                    // '2' or ' '
                    c = 0x00;
                }
                0x33..=0x37 => {
                    // '3' ..= '7'
                    c = 0x1b + c - u32::from(b'3');
                }
                0x38 => {
                    // '8'
                    c = 0x7f;
                }
                0x2f => {
                    // '/'
                    c = 0x1f;
                }
                _ => {
                    if (u32::from(b'@')..=0x7f).contains(&c) {
                        c &= 0x1f;
                    }
                }
            }
        }

        if (mod_ & VTERM_MOD_ALT) != 0 {
            vterm_push_output_bytes_slice(vt, &[0x1b, c as u8]);
        } else {
            vterm_push_output_bytes_slice(vt, &[c as u8]);
        }
    }
}

unsafe fn emit_literal(
    vt: *mut VTerm,
    key: VTermKey,
    mod_: VTermModifier,
    mut disambiguate: bool,
    literal: c_int,
) {
    unsafe {
        if disambiguate && matches!(key, VTERM_KEY_TAB | VTERM_KEY_ENTER | VTERM_KEY_BACKSPACE) {
            disambiguate = mod_ != VTERM_MOD_NONE;
        }

        if disambiguate {
            let s = format!("{literal};{}u", mod_ + 1);
            vterm_push_output_ctrl_bytes(vt, C1_CSI, s.as_bytes());
        } else if (mod_ & VTERM_MOD_ALT) != 0 {
            vterm_push_output_bytes_slice(vt, &[0x1b, literal as u8]);
        } else {
            vterm_push_output_bytes_slice(vt, &[literal as u8]);
        }
    }
}

unsafe fn emit_csi(vt: *mut VTerm, mod_: VTermModifier, literal: c_int) {
    unsafe {
        if mod_ == 0 {
            vterm_push_output_ctrl_bytes(vt, C1_CSI, &[literal as u8]);
        } else {
            let s = format!("1;{}{}", mod_ + 1, (literal as u8) as char);
            vterm_push_output_ctrl_bytes(vt, C1_CSI, s.as_bytes());
        }
    }
}

unsafe fn emit_ss3(vt: *mut VTerm, mod_: VTermModifier, literal: c_int) {
    unsafe {
        if mod_ == 0 {
            vterm_push_output_ctrl_bytes(vt, C1_SS3, &[literal as u8]);
        } else {
            emit_csi(vt, mod_, literal);
        }
    }
}

pub unsafe fn vterm_keyboard_key(vt: *mut VTerm, key: VTermKey, mod_: VTermModifier) {
    if key == VTERM_KEY_NONE {
        return;
    }
    unsafe {
        let state: *const VTermStateFull = (*vt).state_ptr();
        let disambiguate = vterm_state_get_key_encoding_disambiguate(state);

        let mut k = if key < VTERM_KEY_FUNCTION_0 {
            if key < 0 || (key as usize) >= KEYCODES.len() {
                return;
            }
            KEYCODES[key as usize]
        } else if (VTERM_KEY_FUNCTION_0..=VTERM_KEY_FUNCTION_MAX).contains(&key) {
            let idx = (key - VTERM_KEY_FUNCTION_0) as usize;
            if idx >= KEYCODES_FN.len() {
                return;
            }
            KEYCODES_FN[idx]
        } else if key >= VTERM_KEY_KP_0 {
            let idx = (key - VTERM_KEY_KP_0) as usize;
            if idx >= KEYCODES_KP.len() {
                return;
            }
            if disambiguate {
                KEYCODES_KP_CSIU[idx]
            } else {
                KEYCODES_KP[idx]
            }
        } else {
            return;
        };

        match k.r#type {
            KeycodeType::None => {}
            KeycodeType::Tab => {
                if disambiguate {
                    emit_literal(vt, key, mod_, disambiguate, k.literal);
                } else if mod_ == VTERM_MOD_SHIFT {
                    vterm_push_output_ctrl_bytes(vt, C1_CSI, b"Z");
                } else if (mod_ & VTERM_MOD_SHIFT) != 0 {
                    let s = format!("1;{}Z", mod_ + 1);
                    vterm_push_output_ctrl_bytes(vt, C1_CSI, s.as_bytes());
                } else {
                    emit_literal(vt, key, mod_, disambiguate, k.literal);
                }
            }
            KeycodeType::Enter => {
                if ((*state).mode & MODE_NEWLINE_BIT) != 0 {
                    vterm_push_output_bytes_slice(vt, b"\r\n");
                } else {
                    emit_literal(vt, key, mod_, disambiguate, k.literal);
                }
            }
            KeycodeType::Literal => {
                emit_literal(vt, key, mod_, disambiguate, k.literal);
            }
            KeycodeType::Ss3 => {
                emit_ss3(vt, mod_, k.literal);
            }
            KeycodeType::CsiNum => {
                if mod_ == 0 {
                    let s = format!("{}{}", k.csinum, (k.literal as u8) as char);
                    vterm_push_output_ctrl_bytes(vt, C1_CSI, s.as_bytes());
                } else {
                    let s = format!("{};{}{}", k.csinum, mod_ + 1, (k.literal as u8) as char);
                    vterm_push_output_ctrl_bytes(vt, C1_CSI, s.as_bytes());
                }
            }
            KeycodeType::CsiCursor => {
                if ((*state).mode & MODE_CURSOR_BIT) != 0 {
                    emit_ss3(vt, mod_, k.literal);
                } else {
                    emit_csi(vt, mod_, k.literal);
                }
            }
            KeycodeType::Keypad => {
                if ((*state).mode & MODE_KEYPAD_BIT) != 0 {
                    k.literal = k.csinum;
                    emit_ss3(vt, mod_, k.literal);
                } else {
                    emit_literal(vt, key, mod_, disambiguate, k.literal);
                }
            }
        }
    }
}

pub unsafe fn vterm_keyboard_start_paste(vt: *mut VTerm) {
    unsafe {
        let state: *const VTermStateFull = (*vt).state_ptr();
        if ((*state).mode & MODE_BRACKETPASTE_BIT) != 0 {
            vterm_push_output_ctrl_bytes(vt, C1_CSI, b"200~");
        }
    }
}

pub unsafe fn vterm_keyboard_end_paste(vt: *mut VTerm) {
    unsafe {
        let state: *const VTermStateFull = (*vt).state_ptr();
        if ((*state).mode & MODE_BRACKETPASTE_BIT) != 0 {
            vterm_push_output_ctrl_bytes(vt, C1_CSI, b"201~");
        }
    }
}
