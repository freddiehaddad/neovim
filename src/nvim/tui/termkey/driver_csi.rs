//! CSI / SS3 / DCS / OSC / APC driver for libtermkey (`src/nvim/tui/termkey/driver-csi.c`).

use std::ffi::{c_char, c_int, c_uint};

use crate::tui::termkey::driver_ti::{
    TERMKEY_EVENT_PRESS, TERMKEY_EVENT_RELEASE, TERMKEY_EVENT_REPEAT, TERMKEY_EVENT_UNKNOWN,
    TERMKEY_KEYMOD_ALT, TERMKEY_KEYMOD_SHIFT, TERMKEY_RES_AGAIN, TERMKEY_RES_EOF,
    TERMKEY_RES_ERROR, TERMKEY_RES_KEY, TERMKEY_RES_NONE, TERMKEY_SYM_BEGIN, TERMKEY_SYM_DELETE,
    TERMKEY_SYM_DOWN, TERMKEY_SYM_END, TERMKEY_SYM_FIND, TERMKEY_SYM_HOME, TERMKEY_SYM_INSERT,
    TERMKEY_SYM_KP0, TERMKEY_SYM_KP1, TERMKEY_SYM_KP2, TERMKEY_SYM_KP3, TERMKEY_SYM_KP4,
    TERMKEY_SYM_KP5, TERMKEY_SYM_KP6, TERMKEY_SYM_KP7, TERMKEY_SYM_KP8, TERMKEY_SYM_KP9,
    TERMKEY_SYM_KPCOMMA, TERMKEY_SYM_KPDIV, TERMKEY_SYM_KPENTER, TERMKEY_SYM_KPEQUALS,
    TERMKEY_SYM_KPMINUS, TERMKEY_SYM_KPMULT, TERMKEY_SYM_KPPERIOD, TERMKEY_SYM_KPPLUS,
    TERMKEY_SYM_LEFT, TERMKEY_SYM_PAGEDOWN, TERMKEY_SYM_PAGEUP, TERMKEY_SYM_RIGHT,
    TERMKEY_SYM_SELECT, TERMKEY_SYM_TAB, TERMKEY_SYM_UNKNOWN, TERMKEY_SYM_UP, TERMKEY_TYPE_APC,
    TERMKEY_TYPE_DCS, TERMKEY_TYPE_FUNCTION, TERMKEY_TYPE_KEYSYM, TERMKEY_TYPE_MODEREPORT,
    TERMKEY_TYPE_MOUSE, TERMKEY_TYPE_OSC, TERMKEY_TYPE_POSITION, TERMKEY_TYPE_UNICODE,
    TERMKEY_TYPE_UNKNOWN_CSI, TermKey, TermKeyEvent, TermKeyKey, TermKeyResult, TermKeySym,
    TermKeyType, TiKeyInfo,
};
use crate::tui::termkey::termkey::{emit_codepoint, peekkey_mouse};

pub type TermKeyMouseEvent = c_int;
pub const TERMKEY_MOUSE_UNKNOWN: TermKeyMouseEvent = 0;
pub const TERMKEY_MOUSE_PRESS: TermKeyMouseEvent = 1;
pub const TERMKEY_MOUSE_DRAG: TermKeyMouseEvent = 2;
pub const TERMKEY_MOUSE_RELEASE: TermKeyMouseEvent = 3;

pub const TERMKEY_FLAG_CONVERTKP: c_int = 1 << 1;

#[derive(Clone, Copy)]
pub struct TermKeyCsiParam {
    pub present: bool,
    pub param: c_int,
    pub subparams: [c_int; 4],
    pub nsubparams: usize,
}

impl TermKeyCsiParam {
    pub const fn new() -> Self {
        Self {
            present: false,
            param: 0,
            subparams: [0; 4],
            nsubparams: 0,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut arg: c_int = 0;
        let mut param: c_int = 0;
        let mut subparams = [0 as c_int; 4];
        let mut length = 0usize;
        for &c in bytes {
            if c == b':' {
                if length == 0 {
                    param = arg;
                } else if length - 1 < subparams.len() {
                    subparams[length - 1] = arg;
                }
                arg = 0;
                length += 1;
                continue;
            }
            debug_assert!(c.is_ascii_digit());
            arg = (10 * arg) + c_int::from(c - b'0');
        }
        if length == 0 {
            param = arg;
        } else if length - 1 < subparams.len() {
            subparams[length - 1] = arg;
        }
        Self {
            present: true,
            param,
            subparams,
            nsubparams: length,
        }
    }
}

impl Default for TermKeyCsiParam {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
pub struct TermKeyCsi {
    pub saved_string_id: c_int,
    pub saved_string: Vec<c_char>,
}

const NCSIFUNCS: usize = 35;

const EMPTY_KEYINFO: TiKeyInfo = TiKeyInfo {
    r#type: 0,
    sym: TERMKEY_SYM_UNKNOWN,
    modifier_mask: 0,
    modifier_set: 0,
};

const fn ki(r#type: TermKeyType, sym: TermKeySym) -> TiKeyInfo {
    TiKeyInfo {
        r#type,
        sym,
        modifier_mask: 0,
        modifier_set: 0,
    }
}

const CSI_SS3S: [TiKeyInfo; 64] = {
    let mut tbl = [EMPTY_KEYINFO; 64];
    tbl[(b'A' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_UP);
    tbl[(b'B' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_DOWN);
    tbl[(b'C' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_RIGHT);
    tbl[(b'D' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_LEFT);
    tbl[(b'E' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_BEGIN);
    tbl[(b'F' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_END);
    tbl[(b'H' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_HOME);
    tbl[(b'P' - 0x40) as usize] = ki(TERMKEY_TYPE_FUNCTION, 1);
    tbl[(b'Q' - 0x40) as usize] = ki(TERMKEY_TYPE_FUNCTION, 2);
    tbl[(b'R' - 0x40) as usize] = ki(TERMKEY_TYPE_FUNCTION, 3);
    tbl[(b'S' - 0x40) as usize] = ki(TERMKEY_TYPE_FUNCTION, 4);
    tbl[(b'Z' - 0x40) as usize] = TiKeyInfo {
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_TAB,
        modifier_mask: TERMKEY_KEYMOD_SHIFT,
        modifier_set: TERMKEY_KEYMOD_SHIFT,
    };
    tbl
};

const SS3S: [TiKeyInfo; 64] = {
    let mut tbl = [EMPTY_KEYINFO; 64];
    tbl[(b'M' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPENTER);
    tbl[(b'X' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPEQUALS);
    tbl[(b'j' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPMULT);
    tbl[(b'k' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPPLUS);
    tbl[(b'l' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPCOMMA);
    tbl[(b'm' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPMINUS);
    tbl[(b'n' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPPERIOD);
    tbl[(b'o' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPDIV);
    tbl[(b'p' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP0);
    tbl[(b'q' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP1);
    tbl[(b'r' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP2);
    tbl[(b's' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP3);
    tbl[(b't' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP4);
    tbl[(b'u' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP5);
    tbl[(b'v' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP6);
    tbl[(b'w' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP7);
    tbl[(b'x' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP8);
    tbl[(b'y' - 0x40) as usize] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP9);
    tbl
};

const SS3_KPALTS: [c_char; 64] = {
    let mut tbl = [0 as c_char; 64];
    tbl[(b'X' - 0x40) as usize] = b'=' as c_char;
    tbl[(b'j' - 0x40) as usize] = b'*' as c_char;
    tbl[(b'k' - 0x40) as usize] = b'+' as c_char;
    tbl[(b'l' - 0x40) as usize] = b',' as c_char;
    tbl[(b'm' - 0x40) as usize] = b'-' as c_char;
    tbl[(b'n' - 0x40) as usize] = b'.' as c_char;
    tbl[(b'o' - 0x40) as usize] = b'/' as c_char;
    tbl[(b'p' - 0x40) as usize] = b'0' as c_char;
    tbl[(b'q' - 0x40) as usize] = b'1' as c_char;
    tbl[(b'r' - 0x40) as usize] = b'2' as c_char;
    tbl[(b's' - 0x40) as usize] = b'3' as c_char;
    tbl[(b't' - 0x40) as usize] = b'4' as c_char;
    tbl[(b'u' - 0x40) as usize] = b'5' as c_char;
    tbl[(b'v' - 0x40) as usize] = b'6' as c_char;
    tbl[(b'w' - 0x40) as usize] = b'7' as c_char;
    tbl[(b'x' - 0x40) as usize] = b'8' as c_char;
    tbl[(b'y' - 0x40) as usize] = b'9' as c_char;
    tbl
};

const CSIFUNCS: [TiKeyInfo; NCSIFUNCS] = {
    let mut tbl = [EMPTY_KEYINFO; NCSIFUNCS];
    tbl[1] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_FIND);
    tbl[2] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_INSERT);
    tbl[3] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_DELETE);
    tbl[4] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_SELECT);
    tbl[5] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_PAGEUP);
    tbl[6] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_PAGEDOWN);
    tbl[7] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_HOME);
    tbl[8] = ki(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_END);
    tbl[11] = ki(TERMKEY_TYPE_FUNCTION, 1);
    tbl[12] = ki(TERMKEY_TYPE_FUNCTION, 2);
    tbl[13] = ki(TERMKEY_TYPE_FUNCTION, 3);
    tbl[14] = ki(TERMKEY_TYPE_FUNCTION, 4);
    tbl[15] = ki(TERMKEY_TYPE_FUNCTION, 5);
    tbl[17] = ki(TERMKEY_TYPE_FUNCTION, 6);
    tbl[18] = ki(TERMKEY_TYPE_FUNCTION, 7);
    tbl[19] = ki(TERMKEY_TYPE_FUNCTION, 8);
    tbl[20] = ki(TERMKEY_TYPE_FUNCTION, 9);
    tbl[21] = ki(TERMKEY_TYPE_FUNCTION, 10);
    tbl[23] = ki(TERMKEY_TYPE_FUNCTION, 11);
    tbl[24] = ki(TERMKEY_TYPE_FUNCTION, 12);
    tbl[25] = ki(TERMKEY_TYPE_FUNCTION, 13);
    tbl[26] = ki(TERMKEY_TYPE_FUNCTION, 14);
    tbl[28] = ki(TERMKEY_TYPE_FUNCTION, 15);
    tbl[29] = ki(TERMKEY_TYPE_FUNCTION, 16);
    tbl[31] = ki(TERMKEY_TYPE_FUNCTION, 17);
    tbl[32] = ki(TERMKEY_TYPE_FUNCTION, 18);
    tbl[33] = ki(TERMKEY_TYPE_FUNCTION, 19);
    tbl[34] = ki(TERMKEY_TYPE_FUNCTION, 20);
    tbl
};

#[inline]
pub fn termkey_key_get_linecol(key: &TermKeyKey, line: &mut c_int, col: &mut c_int) {
    *col = c_int::from(key.code.mouse[1] as u8)
        | (c_int::from(key.code.mouse[3] as u8 & 0x0f) << 8);
    *line = c_int::from(key.code.mouse[2] as u8)
        | (c_int::from(key.code.mouse[3] as u8 & 0x70) << 4);
}

#[inline]
pub fn termkey_key_set_linecol(key: &mut TermKeyKey, mut line: c_int, mut col: c_int) {
    if line > 0xfff {
        line = 0xfff;
    }
    if col > 0x7ff {
        col = 0x7ff;
    }
    key.code.mouse[1] = (line & 0x0ff) as u8 as c_char;
        key.code.mouse[2] = (col & 0x0ff) as u8 as c_char;
        key.code.mouse[3] = (((line & 0xf00) >> 8) | ((col & 0x300) >> 4)) as u8 as c_char;
}

#[inline]
fn parse_key_event(n: c_int) -> TermKeyEvent {
    match n {
        1 => TERMKEY_EVENT_PRESS,
        2 => TERMKEY_EVENT_REPEAT,
        3 => TERMKEY_EVENT_RELEASE,
        _ => TERMKEY_EVENT_UNKNOWN,
    }
}

fn handle_csi_ss3_full(
    _tk: &mut TermKey,
    key: &mut TermKeyKey,
    cmd: c_int,
    params: &[TermKeyCsiParam],
    nparams: c_int,
) -> TermKeyResult {
    let mut result = TERMKEY_RES_KEY;

        if !(0x40..0x80).contains(&cmd) {
            return TERMKEY_RES_NONE;
        }

        if nparams > 1 && params[1].present {
            let mut arg: c_int = 0;
            let mut subparam: c_int = 0;
            let mut nsubparams: usize = 1;
            result = termkey_interpret_csi_param(
                params[1],
                &mut arg,
                Some(std::slice::from_mut(&mut subparam)),
                Some(&mut nsubparams),
            );
            if result != TERMKEY_RES_KEY {
                return result;
            }

            if nsubparams > 0 {
                key.event = parse_key_event(subparam);
                if key.event == TERMKEY_EVENT_UNKNOWN {
                    return TERMKEY_RES_NONE;
                }
            }

            key.modifiers = arg - 1;
        } else {
            key.modifiers = 0;
        }

        let idx = (cmd - 0x40) as usize;
        key.r#type = CSI_SS3S[idx].r#type;
        key.code.sym = CSI_SS3S[idx].sym;
        key.code.number = CSI_SS3S[idx].sym;
        key.modifiers &= !CSI_SS3S[idx].modifier_mask;
        key.modifiers |= CSI_SS3S[idx].modifier_set;

        if key.code.sym == TERMKEY_SYM_UNKNOWN {
            result = TERMKEY_RES_NONE;
        }

        result
}

fn handle_csifunc(
    tk: &mut TermKey,
    key: &mut TermKeyKey,
    _cmd: c_int,
    params: &[TermKeyCsiParam],
    nparams: c_int,
) -> TermKeyResult {
    if nparams == 0 {
        return TERMKEY_RES_NONE;
    }

    let mut result: TermKeyResult;
        let mut args = [0 as c_int; 3];

        if nparams > 1 && params[1].present {
            let mut subparam: c_int = 0;
            let mut nsubparams: usize = 1;
            result = termkey_interpret_csi_param(
                params[1],
                &mut args[1],
                Some(std::slice::from_mut(&mut subparam)),
                Some(&mut nsubparams),
            );
            if result != TERMKEY_RES_KEY {
                return result;
            }

            if nsubparams > 0 {
                key.event = parse_key_event(subparam);
                if key.event == TERMKEY_EVENT_UNKNOWN {
                    return TERMKEY_RES_NONE;
                }
            }

            key.modifiers = args[1] - 1;
        } else {
            key.modifiers = 0;
        }

        key.r#type = TERMKEY_TYPE_KEYSYM;

        result = termkey_interpret_csi_param(params[0], &mut args[0], None, None);
        if result != TERMKEY_RES_KEY {
            return result;
        }

        if args[0] == 27 && nparams > 2 && params[2].present {
            result = termkey_interpret_csi_param(params[2], &mut args[2], None, None);
            if result != TERMKEY_RES_KEY {
                return result;
            }

            let m = key.modifiers;
            emit_codepoint(tk, args[2], key);
            key.modifiers |= m;
        } else if args[0] >= 0 && (args[0] as usize) < NCSIFUNCS {
            let idx = args[0] as usize;
            key.r#type = CSIFUNCS[idx].r#type;
            key.code.sym = CSIFUNCS[idx].sym;
            key.code.number = CSIFUNCS[idx].sym;
            key.modifiers &= !CSIFUNCS[idx].modifier_mask;
            key.modifiers |= CSIFUNCS[idx].modifier_set;
        } else {
            key.code.sym = TERMKEY_SYM_UNKNOWN;
        }

        if key.code.sym == TERMKEY_SYM_UNKNOWN {
            result = TERMKEY_RES_NONE;
        }

        result
}

fn handle_csi_u(
    tk: &mut TermKey,
    key: &mut TermKeyKey,
    cmd: c_int,
    params: &[TermKeyCsiParam],
    nparams: c_int,
) -> TermKeyResult {
    if cmd != b'u' as c_int {
            return TERMKEY_RES_NONE;
        }
        let mut args = [0 as c_int; 2];
        if nparams > 1 && params[1].present {
            let mut subparam: c_int = 0;
            let mut nsubparams: usize = 1;
            if termkey_interpret_csi_param(
                params[1],
                &mut args[1],
                Some(std::slice::from_mut(&mut subparam)),
                Some(&mut nsubparams),
            ) != TERMKEY_RES_KEY
            {
                return TERMKEY_RES_ERROR;
            }

            if nsubparams > 0 {
                key.event = parse_key_event(subparam);
                if key.event == TERMKEY_EVENT_UNKNOWN {
                    return TERMKEY_RES_NONE;
                }
            }

            key.modifiers = args[1] - 1;
        } else {
            key.modifiers = 0;
        }

        if termkey_interpret_csi_param(params[0], &mut args[0], None, None) != TERMKEY_RES_KEY {
            return TERMKEY_RES_ERROR;
        }

        let m = key.modifiers;
        key.r#type = TERMKEY_TYPE_KEYSYM;
        emit_codepoint(tk, args[0], key);
        key.modifiers |= m;

        TERMKEY_RES_KEY
}

fn handle_csi_m(
    _tk: &mut TermKey,
    key: &mut TermKeyKey,
    mut cmd: c_int,
    params: &[TermKeyCsiParam],
    nparams: c_int,
) -> TermKeyResult {
    let initial = cmd >> 8;
    cmd &= 0xff;

    if cmd != b'M' as c_int && cmd != b'm' as c_int {
        return TERMKEY_RES_NONE;
    }

    if nparams < 3 {
        return TERMKEY_RES_NONE;
    }

    let mut args = [0 as c_int; 3];
        for (i, arg) in args.iter_mut().enumerate() {
            if termkey_interpret_csi_param(params[i], arg, None, None) != TERMKEY_RES_KEY {
                return TERMKEY_RES_ERROR;
            }
        }

        if initial == 0 {
            key.r#type = TERMKEY_TYPE_MOUSE;
            key.code.mouse[0] = args[0] as u8 as c_char;
            key.modifiers = c_int::from((key.code.mouse[0] as u8 & 0x1c) >> 2);
            key.code.mouse[0] = (key.code.mouse[0] as u8 & !0x1c) as c_char;
            termkey_key_set_linecol(key, args[1], args[2]);
            return TERMKEY_RES_KEY;
        }

        if initial == b'<' as c_int {
            key.r#type = TERMKEY_TYPE_MOUSE;
            key.code.mouse[0] = args[0] as u8 as c_char;
            key.modifiers = c_int::from((key.code.mouse[0] as u8 & 0x1c) >> 2);
            key.code.mouse[0] = (key.code.mouse[0] as u8 & !0x1c) as c_char;
            termkey_key_set_linecol(key, args[1], args[2]);
            if cmd == b'm' as c_int {
                key.code.mouse[3] = (key.code.mouse[3] as u8 | 0x80) as c_char;
            }
            return TERMKEY_RES_KEY;
        }

        TERMKEY_RES_NONE
}

pub fn termkey_interpret_mouse(
    _tk: &TermKey,
    key: &TermKeyKey,
    event: &mut TermKeyMouseEvent,
    button: &mut c_int,
    line: &mut c_int,
    col: &mut c_int,
) -> TermKeyResult {
    {
        if key.r#type != TERMKEY_TYPE_MOUSE {
            return TERMKEY_RES_NONE;
        }

        {
            *button = 0;
        }

        termkey_key_get_linecol(key, line, col);

        let mut btn: c_int = 0;
        let mut code = c_int::from(key.code.mouse[0] as u8);
        let drag = (code & 0x20) != 0;
        code &= !0x3c;

        match code {
            0..=2 => {
                *event = if drag {
                    TERMKEY_MOUSE_DRAG
                } else {
                    TERMKEY_MOUSE_PRESS
                };
                btn = code + 1;
            }
            3 => {
                *event = TERMKEY_MOUSE_RELEASE;
            }
            64..=67 => {
                *event = if drag {
                    TERMKEY_MOUSE_DRAG
                } else {
                    TERMKEY_MOUSE_PRESS
                };
                btn = code + 4 - 64;
            }
            128 | 129 => {
                *event = if drag {
                    TERMKEY_MOUSE_DRAG
                } else {
                    TERMKEY_MOUSE_PRESS
                };
                btn = code + 8 - 128;
            }
            _ => {
                *event = TERMKEY_MOUSE_UNKNOWN;
            }
        }

        {
            *button = btn;
        }

        if (key.code.mouse[3] as u8 & 0x80) != 0 {
            *event = TERMKEY_MOUSE_RELEASE;
        }

        TERMKEY_RES_KEY
    }
}

fn handle_csi_R(
    tk: &mut TermKey,
    key: &mut TermKeyKey,
    cmd: c_int,
    params: &[TermKeyCsiParam],
    nparams: c_int,
) -> TermKeyResult {
    if cmd == (b'R' as c_int | ((b'?' as c_int) << 8)) {
            if nparams < 2 {
                return TERMKEY_RES_NONE;
            }
            let mut args = [0 as c_int; 2];
            if termkey_interpret_csi_param(params[0], &mut args[0], None, None) != TERMKEY_RES_KEY {
                return TERMKEY_RES_ERROR;
            }
            if termkey_interpret_csi_param(params[1], &mut args[1], None, None) != TERMKEY_RES_KEY {
                return TERMKEY_RES_ERROR;
            }
            key.r#type = TERMKEY_TYPE_POSITION;
            termkey_key_set_linecol(key, args[1], args[0]);
            TERMKEY_RES_KEY
        } else {
            handle_csi_ss3_full(tk, key, cmd, params, nparams)
        }
}

#[cfg(test)]
pub fn termkey_interpret_position(
    _tk: &TermKey,
    key: &TermKeyKey,
    line: &mut c_int,
    col: &mut c_int,
) -> TermKeyResult {
    if key.r#type != TERMKEY_TYPE_POSITION {
        return TERMKEY_RES_NONE;
    }
    termkey_key_get_linecol(key, line, col);
    TERMKEY_RES_KEY
}

fn handle_csi_y(
    _tk: &mut TermKey,
    key: &mut TermKeyKey,
    cmd: c_int,
    params: &[TermKeyCsiParam],
    nparams: c_int,
) -> TermKeyResult {
    const CMD_ANSI: c_int = (b'y' as c_int) | ((b'$' as c_int) << 16);
    const CMD_DEC: c_int = CMD_ANSI | ((b'?' as c_int) << 8);
    if cmd == CMD_ANSI || cmd == CMD_DEC {
            if nparams < 2 {
                return TERMKEY_RES_NONE;
            }
            let mut args = [0 as c_int; 2];
            if termkey_interpret_csi_param(params[0], &mut args[0], None, None) != TERMKEY_RES_KEY {
                return TERMKEY_RES_ERROR;
            }
            if termkey_interpret_csi_param(params[1], &mut args[1], None, None) != TERMKEY_RES_KEY {
                return TERMKEY_RES_ERROR;
            }
            key.r#type = TERMKEY_TYPE_MODEREPORT;
            key.code.mouse[0] = ((cmd >> 8) & 0xff) as u8 as c_char;
            key.code.mouse[1] = ((args[0] >> 8) & 0xff) as u8 as c_char;
            key.code.mouse[2] = (args[0] & 0xff) as u8 as c_char;
            key.code.mouse[3] = (args[1] & 0xff) as u8 as c_char;
            TERMKEY_RES_KEY
        } else {
            TERMKEY_RES_NONE
        }
}

pub fn termkey_interpret_modereport(
    _tk: &TermKey,
    key: &TermKeyKey,
    initial: &mut c_int,
    mode: &mut c_int,
    value: &mut c_int,
) -> TermKeyResult {
    {
        if key.r#type != TERMKEY_TYPE_MODEREPORT {
            return TERMKEY_RES_NONE;
        }
        {
            *initial = c_int::from(key.code.mouse[0] as u8);
        }
        {
            *mode = (c_int::from(key.code.mouse[1] as u8) << 8)
                | c_int::from(key.code.mouse[2] as u8);
        }
        {
            *value = c_int::from(key.code.mouse[3] as u8);
        }
        TERMKEY_RES_KEY
    }
}

fn parse_csi(
    tk: &TermKey,
    introlen: usize,
    csi_len: &mut usize,
    params: &mut [TermKeyCsiParam; 16],
    nargs: &mut usize,
    commandp: &mut c_uint,
) -> TermKeyResult {
    let mut csi_end = introlen;
    let slice = &tk.buffer[tk.buffstart..tk.buffstart + tk.buffcount];

    while csi_end < tk.buffcount {
        let b = slice[csi_end];
        if (0x40..0x80).contains(&b) {
            break;
        }
        csi_end += 1;
    }

    if csi_end >= tk.buffcount {
        return TERMKEY_RES_AGAIN;
    }

    let cmd = slice[csi_end];
    *commandp = c_uint::from(cmd);

    let mut param_start: Option<usize> = None;
    let mut argi = 0usize;
    let mut p = introlen;

    if (slice[p] >= b'<') && (slice[p] <= b'?') {
        *commandp |= c_uint::from(slice[p]) << 8;
        p += 1;
    }

    while p < csi_end {
        let c = slice[p];

        if (b'0'..b';').contains(&c) {
            if param_start.is_none() {
                param_start = Some(p);
            }
        } else if c == b';' {
            params[argi] = match param_start.take() {
                Some(start) => TermKeyCsiParam::from_bytes(&slice[start..p]),
                None => TermKeyCsiParam::new(),
            };
            argi += 1;
            if argi >= 16 {
                break;
            }
        } else if (0x20..=0x2f).contains(&c) {
            *commandp |= c_uint::from(c) << 16;
            break;
        }

        p += 1;
    }

    if let Some(start) = param_start {
        params[argi] = TermKeyCsiParam::from_bytes(&slice[start..p]);
        argi += 1;
    }

    *nargs = argi;
    *csi_len = csi_end + 1;

    TERMKEY_RES_KEY
}

pub fn termkey_interpret_csi(
    tk: &TermKey,
    key: &TermKeyKey,
    params: &mut [TermKeyCsiParam; 16],
    nparams: &mut usize,
    cmd: &mut c_uint,
) -> TermKeyResult {
    let mut dummy: usize = 0;
    if tk.hightide == 0 {
        return TERMKEY_RES_NONE;
    }
    if key.r#type != TERMKEY_TYPE_UNKNOWN_CSI {
        return TERMKEY_RES_NONE;
    }
    parse_csi(tk, 0, &mut dummy, params, nparams, cmd)
}

pub fn termkey_interpret_csi_param(
    param: TermKeyCsiParam,
    paramp: &mut c_int,
    subparams: Option<&mut [c_int]>,
    nsubparams: Option<&mut usize>,
) -> TermKeyResult {
    if !param.present {
        *paramp = -1;
        if let Some(nsub) = nsubparams {
            *nsub = 0;
        }
        return TERMKEY_RES_KEY;
    }

    *paramp = param.param;
    let capacity = nsubparams.as_deref().copied().unwrap_or(0);
    let count = param.nsubparams.min(capacity).min(param.subparams.len());
    if let Some(sub) = subparams {
        for (dst, &src) in sub.iter_mut().take(count).zip(&param.subparams[..count]) {
            *dst = src;
        }
    }
    if let Some(nsub) = nsubparams {
        *nsub = count;
    }

    TERMKEY_RES_KEY
}

fn peekkey_csi_csi(
    tk: &mut TermKey,
    introlen: usize,
    key: &mut TermKeyKey,
    force: c_int,
    nbytep: &mut usize,
) -> TermKeyResult {
    {
        let mut csi_len: usize = 0;
        let mut nparams: usize = 16;
        let mut params = [TermKeyCsiParam::new(); 16];
        let mut cmd: c_uint = 0;

        let ret = parse_csi(
            tk,
            introlen,
            &mut csi_len,
            &mut params,
            &mut nparams,
            &mut cmd,
        );

        if ret == TERMKEY_RES_AGAIN {
            if force == 0 {
                return TERMKEY_RES_AGAIN;
            }
            emit_codepoint(tk, b'[' as c_int, key);
            key.modifiers |= TERMKEY_KEYMOD_ALT;
            *nbytep = introlen;
            return TERMKEY_RES_KEY;
        }

        if cmd == b'M' as c_uint && nparams < 3 {
            tk.buffstart += csi_len;
            tk.buffcount -= csi_len;

            let mouse_result = peekkey_mouse(tk, key, nbytep);

            tk.buffstart -= csi_len;
            tk.buffcount += csi_len;

            if mouse_result == TERMKEY_RES_KEY {
                *nbytep += csi_len;
            }

            return mouse_result;
        }

        let cmd_i32 = cmd as c_int;
        let np_i32 = nparams as c_int;
        let result = match (cmd & 0xff) as u8 {
            b'u' => handle_csi_u(tk, key, cmd_i32, &params, np_i32),
            b'M' | b'm' => handle_csi_m(tk, key, cmd_i32, &params, np_i32),
            b'R' => handle_csi_R(tk, key, cmd_i32, &params, np_i32),
            b'y' => handle_csi_y(tk, key, cmd_i32, &params, np_i32),
            b'~' => handle_csifunc(tk, key, cmd_i32, &params, np_i32),
            b'A'..=b'F' | b'H' | b'P'..=b'S' | b'Z' => {
                handle_csi_ss3_full(tk, key, cmd_i32, &params, np_i32)
            }
            _ => TERMKEY_RES_NONE,
        };

        if result == TERMKEY_RES_NONE {
            key.r#type = TERMKEY_TYPE_UNKNOWN_CSI;
            key.code.number = cmd as c_int;
            key.modifiers = 0;

            tk.hightide = csi_len - introlen;
            *nbytep = introlen;
            return TERMKEY_RES_KEY;
        }

        *nbytep = csi_len;
        result
    }
}

fn peekkey_ss3(
    tk: &mut TermKey,
    introlen: usize,
    key: &mut TermKeyKey,
    force: c_int,
    nbytep: &mut usize,
) -> TermKeyResult {
    {
        if tk.buffcount < introlen + 1 {
            if force == 0 {
                return TERMKEY_RES_AGAIN;
            }
            emit_codepoint(tk, b'O' as c_int, key);
            key.modifiers |= TERMKEY_KEYMOD_ALT;
            *nbytep = tk.buffcount;
            return TERMKEY_RES_KEY;
        }

        let cmd = tk.buffer[tk.buffstart + introlen];
        if !(0x40..0x80).contains(&cmd) {
            return TERMKEY_RES_NONE;
        }

        let idx = usize::from(cmd - 0x40);
        key.r#type = CSI_SS3S[idx].r#type;
        key.code.sym = CSI_SS3S[idx].sym;
        key.code.number = CSI_SS3S[idx].sym;
        key.modifiers = CSI_SS3S[idx].modifier_set;

        if key.code.sym == TERMKEY_SYM_UNKNOWN {
            if (tk.flags & TERMKEY_FLAG_CONVERTKP) != 0 && SS3_KPALTS[idx] != 0 {
                key.r#type = TERMKEY_TYPE_UNICODE;
                key.code.codepoint = c_int::from(SS3_KPALTS[idx] as u8);
                key.modifiers = 0;
                key.utf8[0] = SS3_KPALTS[idx];
                key.utf8[1] = 0;
            } else {
                key.r#type = SS3S[idx].r#type;
                key.code.sym = SS3S[idx].sym;
                key.code.number = SS3S[idx].sym;
                key.modifiers = SS3S[idx].modifier_set;
            }
        }

        if key.code.sym == TERMKEY_SYM_UNKNOWN {
            return TERMKEY_RES_NONE;
        }

        *nbytep = introlen + 1;
        TERMKEY_RES_KEY
    }
}

fn peekkey_ctrlstring(
    tk: &mut TermKey,
    introlen: usize,
    key: &mut TermKeyKey,
    _force: c_int,
    nbytep: &mut usize,
) -> TermKeyResult {
    {
        let mut str_end = introlen;
        let slice = &tk.buffer[tk.buffstart..tk.buffstart + tk.buffcount];

        while str_end < tk.buffcount {
            let b = slice[str_end];
            if b == 0x07 || b == 0x9c {
                break;
            }
            if b == 0x1b && (str_end + 1) < tk.buffcount && slice[str_end + 1] == 0x5c {
                break;
            }
            str_end += 1;
        }

        if str_end >= tk.buffcount {
            return TERMKEY_RES_AGAIN;
        }

        *nbytep = str_end + 1;
        if slice[str_end] == 0x1b {
            *nbytep += 1;
        }

        let len = str_end - introlen;
        let ctrl_type = slice[introlen - 1] & 0x1f;
        let saved_string = &mut tk.csi.saved_string;
        saved_string.clear();
        saved_string.reserve(len + 1);
        tk.csi.saved_string_id += 1;

        let mut copied = 0usize;
        while copied < len && tk.buffer[tk.buffstart + introlen + copied] != 0 {
            saved_string.push(tk.buffer[tk.buffstart + introlen + copied] as c_char);
            copied += 1;
        }
        while copied <= len {
            saved_string.push(0);
            copied += 1;
        }

        match ctrl_type {
            0x10 => {
                key.r#type = TERMKEY_TYPE_DCS;
            }
            0x1d => {
                key.r#type = TERMKEY_TYPE_OSC;
            }
            0x1f => {
                key.r#type = TERMKEY_TYPE_APC;
            }
            _ => {
                std::process::abort();
            }
        }

        key.code.number = tk.csi.saved_string_id;
        key.modifiers = 0;

        TERMKEY_RES_KEY
    }
}

pub fn peekkey_csi(
    tk: &mut TermKey,
    key: &mut TermKeyKey,
    force: c_int,
    nbytep: &mut usize,
) -> TermKeyResult {
    if tk.buffcount == 0 {
            return if tk.is_closed != 0 {
                TERMKEY_RES_EOF
            } else {
                TERMKEY_RES_NONE
            };
        }

        let b0 = tk.buffer[tk.buffstart];

        match b0 {
            0x1b => {
                if tk.buffcount < 2 {
                    return TERMKEY_RES_NONE;
                }
                match tk.buffer[tk.buffstart + 1] {
                    0x4f => peekkey_ss3(tk, 2, key, force, &mut *nbytep),
                    0x50 | 0x5d | 0x5f => peekkey_ctrlstring(tk, 2, key, force, &mut *nbytep),
                    0x5b => peekkey_csi_csi(tk, 2, key, force, &mut *nbytep),
                    _ => TERMKEY_RES_NONE,
                }
            }
            0x8f => peekkey_ss3(tk, 1, key, force, &mut *nbytep),
            0x90 | 0x9d => peekkey_ctrlstring(tk, 1, key, force, &mut *nbytep),
            0x9b => peekkey_csi_csi(tk, 1, key, force, &mut *nbytep),
            _ => TERMKEY_RES_NONE,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpret_csi_param_parses_params_and_subparams() {
        let param = TermKeyCsiParam::from_bytes(b"12:3:4");
        let mut p: c_int = 0;
        let mut sub = [0 as c_int; 2];
        let mut nsub: usize = 2;
        assert_eq!(
            termkey_interpret_csi_param(param, &mut p, Some(&mut sub), Some(&mut nsub)),
            TERMKEY_RES_KEY
        );
            assert_eq!(p, 12);
            assert_eq!(nsub, 2);
            assert_eq!(sub, [3, 4]);
    }
}
