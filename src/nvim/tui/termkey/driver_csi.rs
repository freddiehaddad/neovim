//! CSI / SS3 / DCS / OSC / APC driver for libtermkey (`src/nvim/tui/termkey/driver-csi.c`).

use std::ffi::{c_char, c_int, c_uchar, c_uint, c_void};

use crate::memory::{xfree, xmalloc};
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
    TermKeyType, TerminfoEntry, keyinfo,
};

pub type TermKeyMouseEvent = c_int;
pub const TERMKEY_MOUSE_UNKNOWN: TermKeyMouseEvent = 0;
pub const TERMKEY_MOUSE_PRESS: TermKeyMouseEvent = 1;
pub const TERMKEY_MOUSE_DRAG: TermKeyMouseEvent = 2;
pub const TERMKEY_MOUSE_RELEASE: TermKeyMouseEvent = 3;

pub const TERMKEY_FLAG_CONVERTKP: c_int = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermKeyCsiParam {
    pub param: *const c_uchar,
    pub length: usize,
}

#[repr(C)]
pub struct TermKeyCsi {
    pub tk: *mut TermKey,
    pub saved_string_id: c_int,
    pub saved_string: *mut c_char,
}

type CsiHandler = Option<
    unsafe fn(
        tk: *mut TermKey,
        key: *mut TermKeyKey,
        cmd: c_int,
        params: *mut TermKeyCsiParam,
        nparams: c_int,
    ) -> TermKeyResult,
>;

const NCSIFUNCS: usize = 35;

static mut KEYINFO_INITIALISED: c_int = 0;
static mut SS3S: [keyinfo; 64] = [keyinfo {
    r#type: 0,
    sym: TERMKEY_SYM_UNKNOWN,
    modifier_mask: 0,
    modifier_set: 0,
}; 64];
static mut SS3_KPALTS: [c_char; 64] = [0; 64];
static mut CSI_HANDLERS: [CsiHandler; 64] = [None; 64];
static mut CSI_SS3S: [keyinfo; 64] = [keyinfo {
    r#type: 0,
    sym: TERMKEY_SYM_UNKNOWN,
    modifier_mask: 0,
    modifier_set: 0,
}; 64];
static mut CSIFUNCS: [keyinfo; NCSIFUNCS] = [keyinfo {
    r#type: 0,
    sym: TERMKEY_SYM_UNKNOWN,
    modifier_mask: 0,
    modifier_set: 0,
}; NCSIFUNCS];

#[inline]
pub unsafe fn termkey_key_get_linecol(key: *const TermKeyKey, line: *mut c_int, col: *mut c_int) {
    unsafe {
        if !col.is_null() {
            *col = c_int::from((*key).code.mouse[1] as u8)
                | (c_int::from((*key).code.mouse[3] as u8 & 0x0f) << 8);
        }
        if !line.is_null() {
            *line = c_int::from((*key).code.mouse[2] as u8)
                | (c_int::from((*key).code.mouse[3] as u8 & 0x70) << 4);
        }
    }
}

#[inline]
pub unsafe fn termkey_key_set_linecol(key: *mut TermKeyKey, mut line: c_int, mut col: c_int) {
    if line > 0xfff {
        line = 0xfff;
    }
    if col > 0x7ff {
        col = 0x7ff;
    }
    unsafe {
        (*key).code.mouse[1] = (line & 0x0ff) as u8 as c_char;
        (*key).code.mouse[2] = (col & 0x0ff) as u8 as c_char;
        (*key).code.mouse[3] = (((line & 0xf00) >> 8) | ((col & 0x300) >> 4)) as u8 as c_char;
    }
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

unsafe fn handle_csi_ss3_full(
    _tk: *mut TermKey,
    key: *mut TermKeyKey,
    cmd: c_int,
    params: *mut TermKeyCsiParam,
    nparams: c_int,
) -> TermKeyResult {
    unsafe {
        let mut result = TERMKEY_RES_KEY;

        if !(0x40..0x80).contains(&cmd) {
            return TERMKEY_RES_NONE;
        }

        if nparams > 1 && !(*params.add(1)).param.is_null() {
            let mut arg: c_int = 0;
            let mut subparam: c_int = 0;
            let mut nsubparams: usize = 1;
            result = termkey_interpret_csi_param(
                *params.add(1),
                &mut arg,
                &mut subparam,
                &mut nsubparams,
            );
            if result != TERMKEY_RES_KEY {
                return result;
            }

            if nsubparams > 0 {
                (*key).event = parse_key_event(subparam);
                if (*key).event == TERMKEY_EVENT_UNKNOWN {
                    return TERMKEY_RES_NONE;
                }
            }

            (*key).modifiers = arg - 1;
        } else {
            (*key).modifiers = 0;
        }

        let idx = (cmd - 0x40) as usize;
        (*key).r#type = CSI_SS3S[idx].r#type;
        (*key).code.sym = CSI_SS3S[idx].sym;
        (*key).modifiers &= !CSI_SS3S[idx].modifier_mask;
        (*key).modifiers |= CSI_SS3S[idx].modifier_set;

        if (*key).code.sym == TERMKEY_SYM_UNKNOWN {
            result = TERMKEY_RES_NONE;
        }

        result
    }
}

unsafe fn register_csi_ss3_full(
    key_type: TermKeyType,
    sym: TermKeySym,
    modifier_set: c_int,
    modifier_mask: c_int,
    cmd: u8,
) {
    if !(0x40..0x80).contains(&cmd) {
        return;
    }
    let idx = usize::from(cmd - 0x40);
    unsafe {
        CSI_SS3S[idx] = keyinfo {
            r#type: key_type,
            sym,
            modifier_mask,
            modifier_set,
        };
        CSI_HANDLERS[idx] = Some(handle_csi_ss3_full);
    }
}

unsafe fn register_csi_ss3(key_type: TermKeyType, sym: TermKeySym, cmd: u8) {
    unsafe {
        register_csi_ss3_full(key_type, sym, 0, 0, cmd);
    }
}

unsafe fn register_ss3kpalt(key_type: TermKeyType, sym: TermKeySym, cmd: u8, kpalt: c_char) {
    if !(0x40..0x80).contains(&cmd) {
        return;
    }
    let idx = usize::from(cmd - 0x40);
    unsafe {
        SS3S[idx] = keyinfo {
            r#type: key_type,
            sym,
            modifier_mask: 0,
            modifier_set: 0,
        };
        SS3_KPALTS[idx] = kpalt;
    }
}

unsafe fn handle_csifunc(
    tk: *mut TermKey,
    key: *mut TermKeyKey,
    _cmd: c_int,
    params: *mut TermKeyCsiParam,
    nparams: c_int,
) -> TermKeyResult {
    if nparams == 0 {
        return TERMKEY_RES_NONE;
    }

    unsafe {
        let mut result: TermKeyResult;
        let mut args = [0 as c_int; 3];

        if nparams > 1 && !(*params.add(1)).param.is_null() {
            let mut subparam: c_int = 0;
            let mut nsubparams: usize = 1;
            result = termkey_interpret_csi_param(
                *params.add(1),
                &mut args[1],
                &mut subparam,
                &mut nsubparams,
            );
            if result != TERMKEY_RES_KEY {
                return result;
            }

            if nsubparams > 0 {
                (*key).event = parse_key_event(subparam);
                if (*key).event == TERMKEY_EVENT_UNKNOWN {
                    return TERMKEY_RES_NONE;
                }
            }

            (*key).modifiers = args[1] - 1;
        } else {
            (*key).modifiers = 0;
        }

        (*key).r#type = TERMKEY_TYPE_KEYSYM;

        result = termkey_interpret_csi_param(
            *params,
            &mut args[0],
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if result != TERMKEY_RES_KEY {
            return result;
        }

        if args[0] == 27 && nparams > 2 && !(*params.add(2)).param.is_null() {
            result = termkey_interpret_csi_param(
                *params.add(2),
                &mut args[2],
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if result != TERMKEY_RES_KEY {
                return result;
            }

            let m = (*key).modifiers;
            ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, args[2], key);
            (*key).modifiers |= m;
        } else if args[0] >= 0 && (args[0] as usize) < NCSIFUNCS {
            let idx = args[0] as usize;
            (*key).r#type = CSIFUNCS[idx].r#type;
            (*key).code.sym = CSIFUNCS[idx].sym;
            (*key).modifiers &= !CSIFUNCS[idx].modifier_mask;
            (*key).modifiers |= CSIFUNCS[idx].modifier_set;
        } else {
            (*key).code.sym = TERMKEY_SYM_UNKNOWN;
        }

        if (*key).code.sym == TERMKEY_SYM_UNKNOWN {
            result = TERMKEY_RES_NONE;
        }

        result
    }
}

unsafe fn register_csifunc(key_type: TermKeyType, sym: TermKeySym, number: usize) {
    if number >= NCSIFUNCS {
        return;
    }
    unsafe {
        CSIFUNCS[number] = keyinfo {
            r#type: key_type,
            sym,
            modifier_mask: 0,
            modifier_set: 0,
        };
        CSI_HANDLERS[usize::from(b'~' - 0x40)] = Some(handle_csifunc);
    }
}

unsafe fn handle_csi_u(
    tk: *mut TermKey,
    key: *mut TermKeyKey,
    cmd: c_int,
    params: *mut TermKeyCsiParam,
    nparams: c_int,
) -> TermKeyResult {
    unsafe {
        if cmd != b'u' as c_int {
            return TERMKEY_RES_NONE;
        }
        let mut args = [0 as c_int; 2];
        if nparams > 1 && !(*params.add(1)).param.is_null() {
            let mut subparam: c_int = 0;
            let mut nsubparams: usize = 1;
            if termkey_interpret_csi_param(
                *params.add(1),
                &mut args[1],
                &mut subparam,
                &mut nsubparams,
            ) != TERMKEY_RES_KEY
            {
                return TERMKEY_RES_ERROR;
            }

            if nsubparams > 0 {
                (*key).event = parse_key_event(subparam);
                if (*key).event == TERMKEY_EVENT_UNKNOWN {
                    return TERMKEY_RES_NONE;
                }
            }

            (*key).modifiers = args[1] - 1;
        } else {
            (*key).modifiers = 0;
        }

        if termkey_interpret_csi_param(
            *params,
            &mut args[0],
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ) != TERMKEY_RES_KEY
        {
            return TERMKEY_RES_ERROR;
        }

        let m = (*key).modifiers;
        (*key).r#type = TERMKEY_TYPE_KEYSYM;
        ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, args[0], key);
        (*key).modifiers |= m;

        TERMKEY_RES_KEY
    }
}

unsafe fn handle_csi_m(
    _tk: *mut TermKey,
    key: *mut TermKeyKey,
    mut cmd: c_int,
    params: *mut TermKeyCsiParam,
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

    unsafe {
        let mut args = [0 as c_int; 3];
        for (i, arg) in args.iter_mut().enumerate() {
            if termkey_interpret_csi_param(
                *params.add(i),
                arg,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != TERMKEY_RES_KEY
            {
                return TERMKEY_RES_ERROR;
            }
        }

        if initial == 0 {
            (*key).r#type = TERMKEY_TYPE_MOUSE;
            (*key).code.mouse[0] = args[0] as u8 as c_char;
            (*key).modifiers = c_int::from(((*key).code.mouse[0] as u8 & 0x1c) >> 2);
            (*key).code.mouse[0] = ((*key).code.mouse[0] as u8 & !0x1c) as c_char;
            termkey_key_set_linecol(key, args[1], args[2]);
            return TERMKEY_RES_KEY;
        }

        if initial == b'<' as c_int {
            (*key).r#type = TERMKEY_TYPE_MOUSE;
            (*key).code.mouse[0] = args[0] as u8 as c_char;
            (*key).modifiers = c_int::from(((*key).code.mouse[0] as u8 & 0x1c) >> 2);
            (*key).code.mouse[0] = ((*key).code.mouse[0] as u8 & !0x1c) as c_char;
            termkey_key_set_linecol(key, args[1], args[2]);
            if cmd == b'm' as c_int {
                (*key).code.mouse[3] = ((*key).code.mouse[3] as u8 | 0x80) as c_char;
            }
            return TERMKEY_RES_KEY;
        }

        TERMKEY_RES_NONE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_interpret_mouse(
    _tk: *mut TermKey,
    key: *const TermKeyKey,
    event: *mut TermKeyMouseEvent,
    button: *mut c_int,
    line: *mut c_int,
    col: *mut c_int,
) -> TermKeyResult {
    unsafe {
        if (*key).r#type != TERMKEY_TYPE_MOUSE {
            return TERMKEY_RES_NONE;
        }

        if !button.is_null() {
            *button = 0;
        }

        termkey_key_get_linecol(key, line, col);

        if event.is_null() {
            return TERMKEY_RES_KEY;
        }

        let mut btn: c_int = 0;
        let mut code = c_int::from((*key).code.mouse[0] as u8);
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

        if !button.is_null() {
            *button = btn;
        }

        if ((*key).code.mouse[3] as u8 & 0x80) != 0 {
            *event = TERMKEY_MOUSE_RELEASE;
        }

        TERMKEY_RES_KEY
    }
}

unsafe fn handle_csi_R(
    tk: *mut TermKey,
    key: *mut TermKeyKey,
    cmd: c_int,
    params: *mut TermKeyCsiParam,
    nparams: c_int,
) -> TermKeyResult {
    unsafe {
        if cmd == (b'R' as c_int | ((b'?' as c_int) << 8)) {
            if nparams < 2 {
                return TERMKEY_RES_NONE;
            }
            let mut args = [0 as c_int; 2];
            if termkey_interpret_csi_param(
                *params,
                &mut args[0],
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != TERMKEY_RES_KEY
            {
                return TERMKEY_RES_ERROR;
            }
            if termkey_interpret_csi_param(
                *params.add(1),
                &mut args[1],
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != TERMKEY_RES_KEY
            {
                return TERMKEY_RES_ERROR;
            }
            (*key).r#type = TERMKEY_TYPE_POSITION;
            termkey_key_set_linecol(key, args[1], args[0]);
            TERMKEY_RES_KEY
        } else {
            handle_csi_ss3_full(tk, key, cmd, params, nparams)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_interpret_position(
    _tk: *mut TermKey,
    key: *const TermKeyKey,
    line: *mut c_int,
    col: *mut c_int,
) -> TermKeyResult {
    unsafe {
        if (*key).r#type != TERMKEY_TYPE_POSITION {
            return TERMKEY_RES_NONE;
        }
        termkey_key_get_linecol(key, line, col);
        TERMKEY_RES_KEY
    }
}

unsafe fn handle_csi_y(
    _tk: *mut TermKey,
    key: *mut TermKeyKey,
    cmd: c_int,
    params: *mut TermKeyCsiParam,
    nparams: c_int,
) -> TermKeyResult {
    const CMD_ANSI: c_int = (b'y' as c_int) | ((b'$' as c_int) << 16);
    const CMD_DEC: c_int = CMD_ANSI | ((b'?' as c_int) << 8);
    unsafe {
        if cmd == CMD_ANSI || cmd == CMD_DEC {
            if nparams < 2 {
                return TERMKEY_RES_NONE;
            }
            let mut args = [0 as c_int; 2];
            if termkey_interpret_csi_param(
                *params,
                &mut args[0],
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != TERMKEY_RES_KEY
            {
                return TERMKEY_RES_ERROR;
            }
            if termkey_interpret_csi_param(
                *params.add(1),
                &mut args[1],
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != TERMKEY_RES_KEY
            {
                return TERMKEY_RES_ERROR;
            }
            (*key).r#type = TERMKEY_TYPE_MODEREPORT;
            (*key).code.mouse[0] = ((cmd >> 8) & 0xff) as u8 as c_char;
            (*key).code.mouse[1] = ((args[0] >> 8) & 0xff) as u8 as c_char;
            (*key).code.mouse[2] = (args[0] & 0xff) as u8 as c_char;
            (*key).code.mouse[3] = (args[1] & 0xff) as u8 as c_char;
            TERMKEY_RES_KEY
        } else {
            TERMKEY_RES_NONE
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_interpret_modereport(
    _tk: *mut TermKey,
    key: *const TermKeyKey,
    initial: *mut c_int,
    mode: *mut c_int,
    value: *mut c_int,
) -> TermKeyResult {
    unsafe {
        if (*key).r#type != TERMKEY_TYPE_MODEREPORT {
            return TERMKEY_RES_NONE;
        }
        if !initial.is_null() {
            *initial = c_int::from((*key).code.mouse[0] as u8);
        }
        if !mode.is_null() {
            *mode = (c_int::from((*key).code.mouse[1] as u8) << 8)
                | c_int::from((*key).code.mouse[2] as u8);
        }
        if !value.is_null() {
            *value = c_int::from((*key).code.mouse[3] as u8);
        }
        TERMKEY_RES_KEY
    }
}

unsafe fn parse_csi(
    tk: *mut TermKey,
    introlen: usize,
    csi_len: *mut usize,
    params: *mut TermKeyCsiParam,
    nargs: *mut usize,
    commandp: *mut c_uint,
) -> TermKeyResult {
    unsafe {
        let mut csi_end = introlen;
        let buf = (*tk).buffer.add((*tk).buffstart);

        while csi_end < (*tk).buffcount {
            let b = *buf.add(csi_end);
            if (0x40..0x80).contains(&b) {
                break;
            }
            csi_end += 1;
        }

        if csi_end >= (*tk).buffcount {
            return TERMKEY_RES_AGAIN;
        }

        let cmd = *buf.add(csi_end);
        *commandp = c_uint::from(cmd);

        let mut present = false;
        let mut argi = 0usize;
        let mut p = introlen;

        if (*buf.add(p) >= b'<') && (*buf.add(p) <= b'?') {
            *commandp |= c_uint::from(*buf.add(p)) << 8;
            p += 1;
        }

        while p < csi_end {
            let c = *buf.add(p);

            if (b'0'..b';').contains(&c) {
                if !present {
                    (*params.add(argi)).param = buf.add(p);
                    present = true;
                }
            } else if c == b';' {
                if !present {
                    (*params.add(argi)).param = std::ptr::null();
                    (*params.add(argi)).length = 0;
                } else {
                    (*params.add(argi)).length =
                        buf.add(p) as usize - (*params.add(argi)).param as usize;
                }
                present = false;
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

        if present {
            (*params.add(argi)).length = buf.add(p) as usize - (*params.add(argi)).param as usize;
            argi += 1;
        }

        *nargs = argi;
        *csi_len = csi_end + 1;

        TERMKEY_RES_KEY
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_interpret_csi(
    tk: *mut TermKey,
    key: *const TermKeyKey,
    params: *mut TermKeyCsiParam,
    nparams: *mut usize,
    cmd: *mut c_uint,
) -> TermKeyResult {
    unsafe {
        let mut dummy: usize = 0;
        if (*tk).hightide == 0 {
            return TERMKEY_RES_NONE;
        }
        if (*key).r#type != TERMKEY_TYPE_UNKNOWN_CSI {
            return TERMKEY_RES_NONE;
        }
        parse_csi(tk, 0, &mut dummy, params, nparams, cmd)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_interpret_csi_param(
    param: TermKeyCsiParam,
    paramp: *mut c_int,
    subparams: *mut c_int,
    nsubparams: *mut usize,
) -> TermKeyResult {
    if paramp.is_null() {
        return TERMKEY_RES_ERROR;
    }

    unsafe {
        if param.param.is_null() {
            *paramp = -1;
            if !nsubparams.is_null() {
                *nsubparams = 0;
            }
            return TERMKEY_RES_KEY;
        }

        let mut arg: c_int = 0;
        let capacity = if !nsubparams.is_null() {
            *nsubparams
        } else {
            0
        };
        let mut length = 0usize;
        let mut i = 0usize;
        while i < param.length && length <= capacity {
            let c = *param.param.add(i);
            if c == b':' {
                if length == 0 {
                    *paramp = arg;
                } else if !subparams.is_null() {
                    *subparams.add(length - 1) = arg;
                }
                arg = 0;
                length += 1;
                i += 1;
                continue;
            }
            debug_assert!(c.is_ascii_digit());
            arg = (10 * arg) + c_int::from(c - b'0');
            i += 1;
        }

        if length == 0 {
            *paramp = arg;
        } else if !subparams.is_null() {
            *subparams.add(length - 1) = arg;
        }

        if !nsubparams.is_null() {
            *nsubparams = length;
        }

        TERMKEY_RES_KEY
    }
}

unsafe fn register_keys() -> c_int {
    unsafe {
        for i in 0..64 {
            CSI_SS3S[i].sym = TERMKEY_SYM_UNKNOWN;
            SS3S[i].sym = TERMKEY_SYM_UNKNOWN;
            SS3_KPALTS[i] = 0;
        }
        for i in 0..NCSIFUNCS {
            (*(&raw mut CSIFUNCS).cast::<keyinfo>().add(i)).sym = TERMKEY_SYM_UNKNOWN;
        }

        register_csi_ss3(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_UP, b'A');
        register_csi_ss3(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_DOWN, b'B');
        register_csi_ss3(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_RIGHT, b'C');
        register_csi_ss3(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_LEFT, b'D');
        register_csi_ss3(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_BEGIN, b'E');
        register_csi_ss3(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_END, b'F');
        register_csi_ss3(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_HOME, b'H');
        register_csi_ss3(TERMKEY_TYPE_FUNCTION, 1, b'P');
        register_csi_ss3(TERMKEY_TYPE_FUNCTION, 2, b'Q');
        register_csi_ss3(TERMKEY_TYPE_FUNCTION, 3, b'R');
        register_csi_ss3(TERMKEY_TYPE_FUNCTION, 4, b'S');

        register_csi_ss3_full(
            TERMKEY_TYPE_KEYSYM,
            TERMKEY_SYM_TAB,
            TERMKEY_KEYMOD_SHIFT,
            TERMKEY_KEYMOD_SHIFT,
            b'Z',
        );

        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPENTER, b'M', 0);
        register_ss3kpalt(
            TERMKEY_TYPE_KEYSYM,
            TERMKEY_SYM_KPEQUALS,
            b'X',
            b'=' as c_char,
        );
        register_ss3kpalt(
            TERMKEY_TYPE_KEYSYM,
            TERMKEY_SYM_KPMULT,
            b'j',
            b'*' as c_char,
        );
        register_ss3kpalt(
            TERMKEY_TYPE_KEYSYM,
            TERMKEY_SYM_KPPLUS,
            b'k',
            b'+' as c_char,
        );
        register_ss3kpalt(
            TERMKEY_TYPE_KEYSYM,
            TERMKEY_SYM_KPCOMMA,
            b'l',
            b',' as c_char,
        );
        register_ss3kpalt(
            TERMKEY_TYPE_KEYSYM,
            TERMKEY_SYM_KPMINUS,
            b'm',
            b'-' as c_char,
        );
        register_ss3kpalt(
            TERMKEY_TYPE_KEYSYM,
            TERMKEY_SYM_KPPERIOD,
            b'n',
            b'.' as c_char,
        );
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KPDIV, b'o', b'/' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP0, b'p', b'0' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP1, b'q', b'1' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP2, b'r', b'2' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP3, b's', b'3' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP4, b't', b'4' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP5, b'u', b'5' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP6, b'v', b'6' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP7, b'w', b'7' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP8, b'x', b'8' as c_char);
        register_ss3kpalt(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_KP9, b'y', b'9' as c_char);

        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_FIND, 1);
        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_INSERT, 2);
        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_DELETE, 3);
        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_SELECT, 4);
        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_PAGEUP, 5);
        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_PAGEDOWN, 6);
        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_HOME, 7);
        register_csifunc(TERMKEY_TYPE_KEYSYM, TERMKEY_SYM_END, 8);

        register_csifunc(TERMKEY_TYPE_FUNCTION, 1, 11);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 2, 12);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 3, 13);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 4, 14);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 5, 15);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 6, 17);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 7, 18);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 8, 19);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 9, 20);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 10, 21);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 11, 23);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 12, 24);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 13, 25);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 14, 26);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 15, 28);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 16, 29);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 17, 31);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 18, 32);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 19, 33);
        register_csifunc(TERMKEY_TYPE_FUNCTION, 20, 34);

        CSI_HANDLERS[usize::from(b'u' - 0x40)] = Some(handle_csi_u);
        CSI_HANDLERS[usize::from(b'M' - 0x40)] = Some(handle_csi_m);
        CSI_HANDLERS[usize::from(b'm' - 0x40)] = Some(handle_csi_m);
        CSI_HANDLERS[usize::from(b'R' - 0x40)] = Some(handle_csi_R);
        CSI_HANDLERS[usize::from(b'y' - 0x40)] = Some(handle_csi_y);

        KEYINFO_INITIALISED = 1;
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn new_driver_csi(
    tk: *mut TermKey,
    _term: *mut TerminfoEntry,
) -> *mut c_void {
    unsafe {
        if KEYINFO_INITIALISED == 0 && register_keys() == 0 {
            return std::ptr::null_mut();
        }

        let csi = xmalloc(std::mem::size_of::<TermKeyCsi>()).cast::<TermKeyCsi>();
        (*csi).tk = tk;
        (*csi).saved_string_id = 0;
        (*csi).saved_string = std::ptr::null_mut();
        csi.cast::<c_void>()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_driver_csi(info: *mut c_void) {
    unsafe {
        let csi = info.cast::<TermKeyCsi>();
        if !(*csi).saved_string.is_null() {
            xfree((*csi).saved_string.cast());
        }
        xfree(csi.cast());
    }
}

unsafe fn peekkey_csi_csi(
    tk: *mut TermKey,
    _csi: *mut TermKeyCsi,
    introlen: usize,
    key: *mut TermKeyKey,
    force: c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        let mut csi_len: usize = 0;
        let mut nparams: usize = 16;
        let mut params = [TermKeyCsiParam {
            param: std::ptr::null(),
            length: 0,
        }; 16];
        let mut cmd: c_uint = 0;

        let ret = parse_csi(
            tk,
            introlen,
            &mut csi_len,
            params.as_mut_ptr(),
            &mut nparams,
            &mut cmd,
        );

        if ret == TERMKEY_RES_AGAIN {
            if force == 0 {
                return TERMKEY_RES_AGAIN;
            }
            ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, b'[' as c_int, key);
            (*key).modifiers |= TERMKEY_KEYMOD_ALT;
            *nbytep = introlen;
            return TERMKEY_RES_KEY;
        }

        if cmd == b'M' as c_uint && nparams < 3 {
            (*tk).buffstart += csi_len;
            (*tk).buffcount -= csi_len;

            let mouse_result = ((*tk).method.peekkey_mouse.unwrap_unchecked())(tk, key, nbytep);

            (*tk).buffstart -= csi_len;
            (*tk).buffcount += csi_len;

            if mouse_result == TERMKEY_RES_KEY {
                *nbytep += csi_len;
            }

            return mouse_result;
        }

        let mut result = TERMKEY_RES_NONE;
        if let Some(handler) = CSI_HANDLERS[((cmd & 0xff) - 0x40) as usize] {
            result = handler(tk, key, cmd as c_int, params.as_mut_ptr(), nparams as c_int);
        }

        if result == TERMKEY_RES_NONE {
            (*key).r#type = TERMKEY_TYPE_UNKNOWN_CSI;
            (*key).code.number = cmd as c_int;
            (*key).modifiers = 0;

            (*tk).hightide = csi_len - introlen;
            *nbytep = introlen;
            return TERMKEY_RES_KEY;
        }

        *nbytep = csi_len;
        result
    }
}

unsafe fn peekkey_ss3(
    tk: *mut TermKey,
    _csi: *mut TermKeyCsi,
    introlen: usize,
    key: *mut TermKeyKey,
    force: c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        if (*tk).buffcount < introlen + 1 {
            if force == 0 {
                return TERMKEY_RES_AGAIN;
            }
            ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, b'O' as c_int, key);
            (*key).modifiers |= TERMKEY_KEYMOD_ALT;
            *nbytep = (*tk).buffcount;
            return TERMKEY_RES_KEY;
        }

        let cmd = *(*tk).buffer.add((*tk).buffstart + introlen);
        if !(0x40..0x80).contains(&cmd) {
            return TERMKEY_RES_NONE;
        }

        let idx = usize::from(cmd - 0x40);
        (*key).r#type = CSI_SS3S[idx].r#type;
        (*key).code.sym = CSI_SS3S[idx].sym;
        (*key).modifiers = CSI_SS3S[idx].modifier_set;

        if (*key).code.sym == TERMKEY_SYM_UNKNOWN {
            if ((*tk).flags & TERMKEY_FLAG_CONVERTKP) != 0 && SS3_KPALTS[idx] != 0 {
                (*key).r#type = TERMKEY_TYPE_UNICODE;
                (*key).code.codepoint = c_int::from(SS3_KPALTS[idx] as u8);
                (*key).modifiers = 0;
                (*key).utf8[0] = SS3_KPALTS[idx];
                (*key).utf8[1] = 0;
            } else {
                (*key).r#type = SS3S[idx].r#type;
                (*key).code.sym = SS3S[idx].sym;
                (*key).modifiers = SS3S[idx].modifier_set;
            }
        }

        if (*key).code.sym == TERMKEY_SYM_UNKNOWN {
            return TERMKEY_RES_NONE;
        }

        *nbytep = introlen + 1;
        TERMKEY_RES_KEY
    }
}

unsafe fn peekkey_ctrlstring(
    tk: *mut TermKey,
    csi: *mut TermKeyCsi,
    introlen: usize,
    key: *mut TermKeyKey,
    _force: c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        let mut str_end = introlen;
        let buf = (*tk).buffer.add((*tk).buffstart);

        while str_end < (*tk).buffcount {
            let b = *buf.add(str_end);
            if b == 0x07 || b == 0x9c {
                break;
            }
            if b == 0x1b && (str_end + 1) < (*tk).buffcount && *buf.add(str_end + 1) == 0x5c {
                break;
            }
            str_end += 1;
        }

        if str_end >= (*tk).buffcount {
            return TERMKEY_RES_AGAIN;
        }

        *nbytep = str_end + 1;
        if *buf.add(str_end) == 0x1b {
            *nbytep += 1;
        }

        if !(*csi).saved_string.is_null() {
            xfree((*csi).saved_string.cast());
        }

        let len = str_end - introlen;
        (*csi).saved_string_id += 1;
        (*csi).saved_string = xmalloc(len + 1).cast::<c_char>();

        let src = buf.add(introlen).cast::<c_char>();
        let mut copied = 0usize;
        while copied < len && *src.add(copied) != 0 {
            *(*csi).saved_string.add(copied) = *src.add(copied);
            copied += 1;
        }
        while copied <= len {
            *(*csi).saved_string.add(copied) = 0;
            copied += 1;
        }

        let ctrl_type = *buf.add(introlen - 1) & 0x1f;
        match ctrl_type {
            0x10 => {
                (*key).r#type = TERMKEY_TYPE_DCS;
            }
            0x1d => {
                (*key).r#type = TERMKEY_TYPE_OSC;
            }
            0x1f => {
                (*key).r#type = TERMKEY_TYPE_APC;
            }
            _ => {
                std::process::abort();
            }
        }

        (*key).code.number = (*csi).saved_string_id;
        (*key).modifiers = 0;

        TERMKEY_RES_KEY
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn peekkey_csi(
    tk: *mut TermKey,
    info: *mut c_void,
    key: *mut TermKeyKey,
    force: c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        if (*tk).buffcount == 0 {
            return if (*tk).is_closed != 0 {
                TERMKEY_RES_EOF
            } else {
                TERMKEY_RES_NONE
            };
        }

        let csi = info.cast::<TermKeyCsi>();
        let buf = (*tk).buffer.add((*tk).buffstart);

        match *buf {
            0x1b => {
                if (*tk).buffcount < 2 {
                    return TERMKEY_RES_NONE;
                }
                match *buf.add(1) {
                    0x4f => peekkey_ss3(tk, csi, 2, key, force, nbytep),
                    0x50 | 0x5d | 0x5f => peekkey_ctrlstring(tk, csi, 2, key, force, nbytep),
                    0x5b => peekkey_csi_csi(tk, csi, 2, key, force, nbytep),
                    _ => TERMKEY_RES_NONE,
                }
            }
            0x8f => peekkey_ss3(tk, csi, 1, key, force, nbytep),
            0x90 | 0x9d => peekkey_ctrlstring(tk, csi, 1, key, force, nbytep),
            0x9b => peekkey_csi_csi(tk, csi, 1, key, force, nbytep),
            _ => TERMKEY_RES_NONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpret_csi_param_parses_params_and_subparams() {
        let raw = b"12:3:4";
        let param = TermKeyCsiParam {
            param: raw.as_ptr(),
            length: raw.len(),
        };
        let mut p: c_int = 0;
        let mut sub = [0 as c_int; 2];
        let mut nsub: usize = 2;
        unsafe {
            assert_eq!(
                termkey_interpret_csi_param(param, &mut p, sub.as_mut_ptr(), &mut nsub),
                TERMKEY_RES_KEY
            );
            assert_eq!(p, 12);
            assert_eq!(nsub, 2);
            assert_eq!(sub, [3, 4]);
        }
    }
}
