//! Core libtermkey state machine and key formatting (`src/nvim/tui/termkey/termkey.c`).

use crate::mbyte::{utf_char2bytes, utf_char2len};
use crate::tui::termkey::driver_csi::{
    TERMKEY_MOUSE_UNKNOWN, TermKeyCsi, TermKeyMouseEvent, peekkey_csi,
    termkey_interpret_modereport, termkey_interpret_mouse,
};
use crate::tui::termkey::driver_ti::{
    TERMKEY_EVENT_PRESS, TERMKEY_EVENT_RELEASE, TERMKEY_EVENT_REPEAT, TERMKEY_KEYMOD_ALT,
    TERMKEY_KEYMOD_CTRL, TERMKEY_KEYMOD_SHIFT, TERMKEY_RES_AGAIN, TERMKEY_RES_EOF,
    TERMKEY_RES_ERROR, TERMKEY_RES_KEY, TERMKEY_RES_NONE, TERMKEY_SYM_BACKSPACE, TERMKEY_SYM_BEGIN,
    TERMKEY_SYM_CANCEL, TERMKEY_SYM_CLEAR, TERMKEY_SYM_CLOSE, TERMKEY_SYM_COMMAND,
    TERMKEY_SYM_COPY, TERMKEY_SYM_DEL, TERMKEY_SYM_DELETE, TERMKEY_SYM_DOWN, TERMKEY_SYM_END,
    TERMKEY_SYM_ENTER, TERMKEY_SYM_ESCAPE, TERMKEY_SYM_EXIT, TERMKEY_SYM_FIND, TERMKEY_SYM_HELP,
    TERMKEY_SYM_HOME, TERMKEY_SYM_INSERT, TERMKEY_SYM_KP0, TERMKEY_SYM_KP1, TERMKEY_SYM_KP2,
    TERMKEY_SYM_KP3, TERMKEY_SYM_KP4, TERMKEY_SYM_KP5, TERMKEY_SYM_KP6, TERMKEY_SYM_KP7,
    TERMKEY_SYM_KP8, TERMKEY_SYM_KP9, TERMKEY_SYM_KPCOMMA, TERMKEY_SYM_KPDIV, TERMKEY_SYM_KPENTER,
    TERMKEY_SYM_KPEQUALS, TERMKEY_SYM_KPMINUS, TERMKEY_SYM_KPMULT, TERMKEY_SYM_KPPERIOD,
    TERMKEY_SYM_KPPLUS, TERMKEY_SYM_LEFT, TERMKEY_SYM_MARK, TERMKEY_SYM_MESSAGE, TERMKEY_SYM_MOVE,
    TERMKEY_SYM_NONE, TERMKEY_SYM_OPEN, TERMKEY_SYM_OPTIONS, TERMKEY_SYM_PAGEDOWN,
    TERMKEY_SYM_PAGEUP, TERMKEY_SYM_PRINT, TERMKEY_SYM_REDO, TERMKEY_SYM_REFERENCE,
    TERMKEY_SYM_REFRESH, TERMKEY_SYM_REPLACE, TERMKEY_SYM_RESTART, TERMKEY_SYM_RESUME,
    TERMKEY_SYM_RIGHT, TERMKEY_SYM_SAVE, TERMKEY_SYM_SELECT, TERMKEY_SYM_SPACE,
    TERMKEY_SYM_SUSPEND, TERMKEY_SYM_TAB, TERMKEY_SYM_UNDO, TERMKEY_SYM_UNKNOWN, TERMKEY_SYM_UP,
    TERMKEY_TYPE_APC, TERMKEY_TYPE_DCS, TERMKEY_TYPE_FUNCTION, TERMKEY_TYPE_KEYSYM,
    TERMKEY_TYPE_MODEREPORT, TERMKEY_TYPE_MOUSE, TERMKEY_TYPE_OSC, TERMKEY_TYPE_POSITION,
    TERMKEY_TYPE_UNICODE, TERMKEY_TYPE_UNKNOWN_CSI, TermKey, TermKeyKey, TermKeyResult, TermKeySym,
    TermKeyTI, TerminfoEntry, keyinfo, peekkey_ti, start_driver_ti,
};
use std::ffi::{CStr, c_char, c_int};

pub const TERMKEY_FLAG_NOINTERPRET: c_int = 1 << 0;
pub const TERMKEY_FLAG_UTF8: c_int = 1 << 3;
pub const TERMKEY_FLAG_SPACESYMBOL: c_int = 1 << 5;
pub const TERMKEY_FLAG_NOSTART: c_int = 1 << 8;
pub const TERMKEY_FLAG_KEEPC0: c_int = 1 << 9;

pub const TERMKEY_CANON_SPACESYMBOL: c_int = 1 << 0;
pub const TERMKEY_CANON_DELBS: c_int = 1 << 1;

pub type TermKeyFormat = c_int;
pub const TERMKEY_FORMAT_LONGMOD: TermKeyFormat = 1 << 0;
pub const TERMKEY_FORMAT_CARETCTRL: TermKeyFormat = 1 << 1;
pub const TERMKEY_FORMAT_ALTISMETA: TermKeyFormat = 1 << 2;
pub const TERMKEY_FORMAT_WRAPBRACKET: TermKeyFormat = 1 << 3;
pub const TERMKEY_FORMAT_SPACEMOD: TermKeyFormat = 1 << 4;
pub const TERMKEY_FORMAT_LOWERMOD: TermKeyFormat = 1 << 5;
pub const TERMKEY_FORMAT_LOWERSPACE: TermKeyFormat = 1 << 6;
pub const TERMKEY_FORMAT_MOUSE_POS: TermKeyFormat = 1 << 8;

const UNICODE_INVALID: c_int = 0xFFFD;

struct KeyNameInit {
    sym: TermKeySym,
    name: &'static CStr,
}

const KEYNAMES: &[KeyNameInit] = &[
    KeyNameInit {
        sym: TERMKEY_SYM_NONE,
        name: c"NONE",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_BACKSPACE,
        name: c"Backspace",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_TAB,
        name: c"Tab",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_ENTER,
        name: c"Enter",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_ESCAPE,
        name: c"Escape",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_SPACE,
        name: c"Space",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_DEL,
        name: c"DEL",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_UP,
        name: c"Up",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_DOWN,
        name: c"Down",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_LEFT,
        name: c"Left",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_RIGHT,
        name: c"Right",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_BEGIN,
        name: c"Begin",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_FIND,
        name: c"Find",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_INSERT,
        name: c"Insert",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_DELETE,
        name: c"Delete",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_SELECT,
        name: c"Select",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_PAGEUP,
        name: c"PageUp",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_PAGEDOWN,
        name: c"PageDown",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_HOME,
        name: c"Home",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_END,
        name: c"End",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_CANCEL,
        name: c"Cancel",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_CLEAR,
        name: c"Clear",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_CLOSE,
        name: c"Close",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_COMMAND,
        name: c"Command",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_COPY,
        name: c"Copy",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_EXIT,
        name: c"Exit",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_HELP,
        name: c"Help",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_MARK,
        name: c"Mark",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_MESSAGE,
        name: c"Message",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_MOVE,
        name: c"Move",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_OPEN,
        name: c"Open",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_OPTIONS,
        name: c"Options",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_PRINT,
        name: c"Print",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_REDO,
        name: c"Redo",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_REFERENCE,
        name: c"Reference",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_REFRESH,
        name: c"Refresh",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_REPLACE,
        name: c"Replace",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_RESTART,
        name: c"Restart",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_RESUME,
        name: c"Resume",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_SAVE,
        name: c"Save",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_SUSPEND,
        name: c"Suspend",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_UNDO,
        name: c"Undo",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP0,
        name: c"KP0",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP1,
        name: c"KP1",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP2,
        name: c"KP2",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP3,
        name: c"KP3",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP4,
        name: c"KP4",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP5,
        name: c"KP5",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP6,
        name: c"KP6",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP7,
        name: c"KP7",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP8,
        name: c"KP8",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KP9,
        name: c"KP9",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPENTER,
        name: c"KPEnter",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPPLUS,
        name: c"KPPlus",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPMINUS,
        name: c"KPMinus",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPMULT,
        name: c"KPMult",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPDIV,
        name: c"KPDiv",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPCOMMA,
        name: c"KPComma",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPPERIOD,
        name: c"KPPeriod",
    },
    KeyNameInit {
        sym: TERMKEY_SYM_KPEQUALS,
        name: c"KPEquals",
    },
];

const EVNAMES: [&str; 4] = ["Unknown", "Press", "Drag", "Release"];

pub fn termkey_interpret_string(
    tk: &TermKey,
    key: &TermKeyKey,
    strp: *mut *const c_char,
) -> TermKeyResult {
    unsafe {
        if key.r#type != TERMKEY_TYPE_DCS
            && key.r#type != TERMKEY_TYPE_OSC
            && key.r#type != TERMKEY_TYPE_APC
        {
            return TERMKEY_RES_NONE;
        }

        if tk.csi.saved_string_id != key.code.number {
            return TERMKEY_RES_NONE;
        }

        *strp = tk.csi.saved_string.as_ptr();
        TERMKEY_RES_KEY
    }
}

#[cfg(test)]
fn strpncmp_camel(
    strp: *mut *const c_char,
    strcamelp: *mut *const c_char,
    mut n: usize,
) -> c_int {
    unsafe {
        let mut s = *strp;
        let mut sc = *strcamelp;
        let mut prev_lower = false;

        while (*s != 0 || *sc != 0) && n != 0 {
            let sc_u8 = crate::memory::cstr_first_byte(sc);
            let b = sc_u8.to_ascii_lowercase() as c_char;
            if sc_u8.is_ascii_uppercase() && prev_lower {
                if crate::memory::cstr_first_byte(s) != b' ' {
                    break;
                }
                s = s.add(1);
                if *s != b {
                    break;
                }
            } else if *s != b {
                break;
            }

            prev_lower = sc_u8.is_ascii_lowercase();
            s = s.add(1);
            sc = sc.add(1);
            n -= 1;
        }

        *strp = s;
        *strcamelp = sc;
        c_int::from(*s) - c_int::from(*sc)
    }
}

pub fn termkey_new_abstract(term: Option<&TerminfoEntry>, flags: c_int) -> Option<TermKey> {
    let mut tk = TermKey {
            flags: 0,
            canonflags: 0,
            buffer: vec![0u8; 256],
            buffstart: 0,
            buffcount: 0,
            hightide: 0,
            is_closed: 0,
            is_started: 0,
            keynames: vec![None; 64],
            c0: [keyinfo {
                r#type: 0,
                sym: TERMKEY_SYM_NONE,
                modifier_mask: 0,
                modifier_set: 0,
            }; 32],
            ti: TermKeyTI::new(),
            csi: TermKeyCsi::default(),
        };

        termkey_set_flags(&mut tk, flags);

        for entry in KEYNAMES {
            termkey_register_keyname(&mut tk, entry.sym, Some(entry.name));
        }

        register_c0(&mut tk, TERMKEY_SYM_TAB, 0x09, None);
        register_c0(&mut tk, TERMKEY_SYM_ENTER, 0x0d, None);
        register_c0(&mut tk, TERMKEY_SYM_ESCAPE, 0x1b, None);

        if (flags & TERMKEY_FLAG_NOSTART) == 0
            && termkey_start(&mut tk, term, None) == 0
        {
            return None;
        }

        Some(tk)
}

pub fn termkey_start(
    tk: &mut TermKey,
    term: Option<&TerminfoEntry>,
    input: Option<&mut crate::tui::input::TermInput>,
) -> c_int {
    if tk.is_started != 0 {
            return 1;
        }

        if start_driver_ti(tk, term, input) == 0 {
            return 0;
        }

        tk.is_started = 1;
        1
}

#[cfg(test)]
pub fn termkey_stop(tk: &mut TermKey) -> c_int {
    if tk.is_started == 0 {
        return 1;
    }

    tk.is_started = 0;
    1
}

pub fn termkey_set_flags(tk: &mut TermKey, newflags: c_int) {
    tk.flags = newflags;
    if (tk.flags & TERMKEY_FLAG_SPACESYMBOL) != 0 {
        tk.canonflags |= TERMKEY_CANON_SPACESYMBOL;
    } else {
        tk.canonflags &= !TERMKEY_CANON_SPACESYMBOL;
    }
}

pub const fn termkey_get_canonflags(tk: &TermKey) -> c_int {
    tk.canonflags
}

pub fn termkey_set_canonflags(tk: &mut TermKey, flags: c_int) {
    tk.canonflags = flags;
    if (tk.canonflags & TERMKEY_CANON_SPACESYMBOL) != 0 {
        tk.flags |= TERMKEY_FLAG_SPACESYMBOL;
    } else {
        tk.flags &= !TERMKEY_FLAG_SPACESYMBOL;
    }
}

pub fn termkey_get_buffer_size(tk: &TermKey) -> usize {
    tk.buffer.len()
}

pub fn termkey_set_buffer_size(tk: &mut TermKey, size: usize) -> c_int {
    tk.buffer.resize(size, 0);
    1
}

pub fn termkey_get_buffer_remaining(tk: &TermKey) -> usize {
    tk.buffer.len() - tk.buffcount
}

#[inline]
fn eat_bytes(tk: &mut TermKey, count: usize) {
    if count >= tk.buffcount {
        tk.buffstart = 0;
        tk.buffcount = 0;
    } else {
        tk.buffstart += count;
        tk.buffcount -= count;
    }
}

pub fn fill_utf8(codepoint: c_int, str_out: *mut c_char) -> c_int {
    unsafe {
        let nbytes = utf_char2bytes(codepoint, str_out);
        *str_out.add(nbytes as usize) = 0;
        nbytes
    }
}

fn parse_utf8(bytes: &[u8], cp: &mut c_int, nbytep: &mut usize) -> TermKeyResult {
    let b0 = bytes[0];
    let nbytes: usize;

    if b0 < 0x80 {
        *cp = c_int::from(b0);
        *nbytep = 1;
        return TERMKEY_RES_KEY;
    } else if b0 < 0xc0 {
        *cp = UNICODE_INVALID;
        *nbytep = 1;
        return TERMKEY_RES_KEY;
    } else if b0 < 0xe0 {
        nbytes = 2;
        *cp = c_int::from(b0 & 0x1f);
    } else if b0 < 0xf0 {
        nbytes = 3;
        *cp = c_int::from(b0 & 0x0f);
    } else if b0 < 0xf8 {
        nbytes = 4;
        *cp = c_int::from(b0 & 0x07);
    } else if b0 < 0xfc {
        nbytes = 5;
        *cp = c_int::from(b0 & 0x03);
    } else if b0 < 0xfe {
        nbytes = 6;
        *cp = c_int::from(b0 & 0x01);
    } else {
        *cp = UNICODE_INVALID;
        *nbytep = 1;
        return TERMKEY_RES_KEY;
    }

    for (b, &cb) in bytes.iter().enumerate().take(nbytes).skip(1) {
        if !(0x80..0xc0).contains(&cb) {
            *cp = UNICODE_INVALID;
            *nbytep = b;
            return TERMKEY_RES_KEY;
        }
        *cp = (*cp << 6) | c_int::from(cb & 0x3f);
    }
    if bytes.len() < nbytes {
        return TERMKEY_RES_AGAIN;
    }

    if (nbytes as c_int) > utf_char2len(*cp) {
        *cp = UNICODE_INVALID;
    }

    if (0xD800..=0xDFFF).contains(&*cp) || *cp == 0xFFFE || *cp == 0xFFFF {
        *cp = UNICODE_INVALID;
    }

    *nbytep = nbytes;
    TERMKEY_RES_KEY
}

pub(crate) fn emit_codepoint(tk: &TermKey, codepoint: c_int, key: &mut TermKeyKey) {
    if codepoint == 0 {
            key.r#type = TERMKEY_TYPE_KEYSYM;
            key.code.sym = TERMKEY_SYM_SPACE;
            key.modifiers = TERMKEY_KEYMOD_CTRL;
        } else if codepoint < 0x20 && (tk.flags & TERMKEY_FLAG_KEEPC0) == 0 {
            key.code.codepoint = 0;
            key.modifiers = 0;

            let idx = codepoint as usize;
            if (tk.flags & TERMKEY_FLAG_NOINTERPRET) == 0 && tk.c0[idx].sym != TERMKEY_SYM_UNKNOWN {
                key.code.sym = tk.c0[idx].sym;
                key.modifiers |= tk.c0[idx].modifier_set;
            }

            if key.code.sym == 0 {
                key.r#type = TERMKEY_TYPE_UNICODE;
                if (b'A' as c_int..=b'Z' as c_int).contains(&(codepoint + 0x40)) {
                    key.code.codepoint = codepoint + 0x60;
                } else {
                    key.code.codepoint = codepoint + 0x40;
                }
                key.modifiers = TERMKEY_KEYMOD_CTRL;
            } else {
                key.r#type = TERMKEY_TYPE_KEYSYM;
            }
        } else if codepoint == 0x7f && (tk.flags & TERMKEY_FLAG_NOINTERPRET) == 0 {
            key.r#type = TERMKEY_TYPE_KEYSYM;
            key.code.sym = TERMKEY_SYM_DEL;
            key.modifiers = 0;
        } else if codepoint > 0 && codepoint < 0x80 {
            key.r#type = TERMKEY_TYPE_UNICODE;
            key.code.codepoint = codepoint;
            key.modifiers = 0;
        } else if (0x80..0xa0).contains(&codepoint) {
            key.r#type = TERMKEY_TYPE_UNICODE;
            key.code.codepoint = codepoint - 0x40;
            key.modifiers = TERMKEY_KEYMOD_CTRL | TERMKEY_KEYMOD_ALT;
        } else {
            key.r#type = TERMKEY_TYPE_UNICODE;
            key.code.codepoint = codepoint;
            key.modifiers = 0;
        }

        termkey_canonicalise(tk, key);

        if key.r#type == TERMKEY_TYPE_UNICODE {
            fill_utf8(key.code.codepoint, key.utf8.as_mut_ptr());
        }
}

pub fn termkey_canonicalise(tk: &TermKey, key: &mut TermKeyKey) {
    let flags = tk.canonflags;

        if (flags & TERMKEY_CANON_SPACESYMBOL) != 0 {
            if key.r#type == TERMKEY_TYPE_UNICODE && key.code.codepoint == 0x20 {
                key.r#type = TERMKEY_TYPE_KEYSYM;
                key.code.sym = TERMKEY_SYM_SPACE;
            }
        } else if key.r#type == TERMKEY_TYPE_KEYSYM && key.code.sym == TERMKEY_SYM_SPACE {
            key.r#type = TERMKEY_TYPE_UNICODE;
            key.code.codepoint = 0x20;
            fill_utf8(key.code.codepoint, key.utf8.as_mut_ptr());
        }

        if (flags & TERMKEY_CANON_DELBS) != 0
            && key.r#type == TERMKEY_TYPE_KEYSYM
            && key.code.sym == TERMKEY_SYM_DEL
        {
            key.code.sym = TERMKEY_SYM_BACKSPACE;
        }
}

fn peekkey(
    tk: &mut TermKey,
    key: &mut TermKeyKey,
    force: c_int,
    nbytep: &mut usize,
) -> TermKeyResult {
    let mut again = false;

        if tk.is_started == 0 {
            return TERMKEY_RES_ERROR;
        }

        key.event = TERMKEY_EVENT_PRESS;

        if tk.hightide != 0 {
            tk.buffstart += tk.hightide;
            tk.buffcount -= tk.hightide;
            tk.hightide = 0;
        }

        for peek_fn in [peekkey_ti, peekkey_csi] {
            let ret = peek_fn(tk, key, force, nbytep);
            match ret {
                TERMKEY_RES_KEY => {
                    let halfsize = tk.buffer.len() / 2;
                    if tk.buffstart > halfsize {
                        tk.buffer.copy_within(halfsize..(halfsize * 2), 0);
                        tk.buffstart -= halfsize;
                    }
                    return ret;
                }
                TERMKEY_RES_EOF | TERMKEY_RES_ERROR => {
                    return ret;
                }
                TERMKEY_RES_AGAIN if force == 0 => {
                    again = true;
                }
                _ => {}
            }
        }

        if again {
            return TERMKEY_RES_AGAIN;
        }

        peekkey_simple(tk, key, force, nbytep)
}

fn peekkey_simple(
    tk: &mut TermKey,
    key: &mut TermKeyKey,
    force: c_int,
    nbytep: &mut usize,
) -> TermKeyResult {
    {
        if tk.buffcount == 0 {
            return if tk.is_closed != 0 {
                TERMKEY_RES_EOF
            } else {
                TERMKEY_RES_NONE
            };
        }

        let b0 = tk.buffer[tk.buffstart];

        if b0 == 0x1b {
            if tk.buffcount == 1 {
                if force == 0 {
                    return TERMKEY_RES_AGAIN;
                }
                emit_codepoint(tk, c_int::from(b0), key);
                *nbytep = 1;
                return TERMKEY_RES_KEY;
            }

            tk.buffstart += 1;
            tk.buffcount -= 1;

            let metakey_result = peekkey(tk, key, force, nbytep);

            tk.buffstart -= 1;
            tk.buffcount += 1;

            if metakey_result == TERMKEY_RES_KEY {
                if key.event == TERMKEY_EVENT_REPEAT
                    || key.event == TERMKEY_EVENT_RELEASE
                    || key.r#type == TERMKEY_TYPE_DCS
                    || key.r#type == TERMKEY_TYPE_OSC
                    || key.r#type == TERMKEY_TYPE_APC
                {
                    key.event = TERMKEY_EVENT_PRESS;
                    emit_codepoint(tk, c_int::from(b0), key);
                    *nbytep = 1;
                    return TERMKEY_RES_KEY;
                }
                key.modifiers |= TERMKEY_KEYMOD_ALT;
                *nbytep += 1;
            }

            metakey_result
        } else if b0 < 0xa0 {
            emit_codepoint(tk, c_int::from(b0), key);
            *nbytep = 1;
            TERMKEY_RES_KEY
        } else if (tk.flags & TERMKEY_FLAG_UTF8) != 0 {
            let mut codepoint: c_int = 0;
            let start = tk.buffstart;
            let count = tk.buffcount;
            let slice = &tk.buffer[start..start + count];
            let mut res = parse_utf8(slice, &mut codepoint, &mut *nbytep);

            if res == TERMKEY_RES_AGAIN && force != 0 {
                codepoint = UNICODE_INVALID;
                *nbytep = count;
                res = TERMKEY_RES_KEY;
            }

            key.r#type = TERMKEY_TYPE_UNICODE;
            key.modifiers = 0;
            emit_codepoint(tk, codepoint, key);
            res
        } else {
            key.r#type = TERMKEY_TYPE_UNICODE;
            key.code.codepoint = c_int::from(b0);
            key.modifiers = 0;
            key.utf8[0] = b0 as c_char;
            key.utf8[1] = 0;
            *nbytep = 1;
            TERMKEY_RES_KEY
        }
    }
}

pub(crate) fn peekkey_mouse(
    tk: &TermKey,
    key: &mut TermKeyKey,
    nbytep: &mut usize,
) -> TermKeyResult {
    {
        if tk.buffcount < 3 {
            return TERMKEY_RES_AGAIN;
        }

        let start = tk.buffstart;
        let b0 = tk.buffer[start];
        let b1 = tk.buffer[start + 1];
        let b2 = tk.buffer[start + 2];
        key.r#type = TERMKEY_TYPE_MOUSE;
        key.code.mouse[0] = b0.wrapping_sub(0x20) as c_char;
        key.code.mouse[1] = b1.wrapping_sub(0x20) as c_char;
        key.code.mouse[2] = b2.wrapping_sub(0x20) as c_char;
        key.code.mouse[3] = 0;

        key.modifiers = c_int::from((key.code.mouse[0] as u8 & 0x1c) >> 2);
        key.code.mouse[0] = (key.code.mouse[0] as u8 & !0x1c) as c_char;

        *nbytep = 3;
        TERMKEY_RES_KEY
    }
}

pub fn termkey_getkey(tk: &mut TermKey, key: &mut TermKeyKey) -> TermKeyResult {
    let mut nbytes: usize = 0;
        let ret = peekkey(tk, key, 0, &mut nbytes);

        if ret == TERMKEY_RES_KEY {
            eat_bytes(tk, nbytes);
        }

        if ret == TERMKEY_RES_AGAIN {
            let _ = peekkey(tk, key, 1, &mut nbytes);
        }

        ret
}

pub fn termkey_getkey_force(tk: &mut TermKey, key: &mut TermKeyKey) -> TermKeyResult {
    let mut nbytes: usize = 0;
        let ret = peekkey(tk, key, 1, &mut nbytes);

        if ret == TERMKEY_RES_KEY {
            eat_bytes(tk, nbytes);
        }

        ret
}

pub fn termkey_push_bytes(tk: &mut TermKey, bytes: impl crate::strings::IntoCCharPtr, mut len: usize) -> usize { let bytes = bytes.into_c_char_ptr();
     {
        if tk.buffstart != 0 {
            let start = tk.buffstart;
            let count = tk.buffcount;
            tk.buffer.copy_within(start..start + count, 0);
            tk.buffstart = 0;
        }

        let buffsize = tk.buffer.len();
        if tk.buffcount >= buffsize {
            return usize::MAX;
        }

        if len > buffsize - tk.buffcount {
            len = buffsize - tk.buffcount;
        }

        if len > 0 {
            let src = crate::memory::cstr_slice(bytes, len);
            tk.buffer[tk.buffcount..tk.buffcount + len].copy_from_slice(src);
            tk.buffcount += len;
        }

        len
    }
}

pub fn termkey_register_keyname(
    tk: &mut TermKey,
    mut sym: TermKeySym,
    name: Option<&'static CStr>,
) -> TermKeySym {
    if sym == 0 {
        sym = tk.keynames.len() as c_int;
    }

    let idx = sym as usize;
    if idx >= tk.keynames.len() {
        tk.keynames.resize(idx + 1, None);
    }

    tk.keynames[idx] = name;
    sym
}

pub fn termkey_get_keyname(tk: &TermKey, sym: TermKeySym) -> *const c_char {
    if sym == TERMKEY_SYM_UNKNOWN {
        return c"UNKNOWN".as_ptr();
    }
    if sym >= 0 && (sym as usize) < tk.keynames.len() {
        tk.keynames[sym as usize].map_or(std::ptr::null(), CStr::as_ptr)
    } else {
        c"UNKNOWN".as_ptr()
    }
}

#[cfg(test)]
fn termkey_lookup_keyname_format(
    tk: &TermKey,
    str_in: impl crate::strings::IntoCCharPtr,
    sym: &mut TermKeySym,
    format: TermKeyFormat,
) -> *const c_char { let str_in = str_in.into_c_char_ptr();
    unsafe {
        let keynames = &tk.keynames;
        *sym = 0;
        while (*sym as usize) < keynames.len() {
            if let Some(key_cstr) = keynames[*sym as usize] {
                let mut thiskey = key_cstr.as_ptr();
                let key_bytes = key_cstr.to_bytes();
                let len = key_bytes.len();
                if (format & TERMKEY_FORMAT_LOWERSPACE) != 0 {
                    let mut thisstr = str_in;
                    if strpncmp_camel(&mut thisstr, &mut thiskey, len) == 0 {
                        return thisstr;
                    }
                } else {
                    let mut matched = true;
                    for (i, &kb) in key_bytes.iter().enumerate() {
                        if crate::memory::cstr_byte_at(str_in, i) != kb {
                            matched = false;
                            break;
                        }
                    }
                    if matched {
                        return str_in.add(len);
                    }
                }
            }
            *sym += 1;
        }
        std::ptr::null()
    }
}

#[cfg(test)]
pub fn termkey_lookup_keyname(
    tk: &TermKey,
    str_in: &std::ffi::CStr,
    sym: &mut TermKeySym,
) -> *const c_char {
    termkey_lookup_keyname_format(tk, str_in.as_ptr(), sym, 0)
}

fn register_c0(
    tk: &mut TermKey,
    sym: TermKeySym,
    ctrl: u8,
    name: Option<&'static CStr>,
) -> TermKeySym {
    register_c0_full(tk, sym, 0, 0, ctrl, name)
}

fn register_c0_full(
    tk: &mut TermKey,
    mut sym: TermKeySym,
    modifier_set: c_int,
    modifier_mask: c_int,
    ctrl: u8,
    name: Option<&'static CStr>,
) -> TermKeySym {
    if ctrl >= 0x20 {
        return -1;
    }
    if name.is_some() {
        sym = termkey_register_keyname(tk, sym, name);
    }
    let idx = usize::from(ctrl);
    tk.c0[idx].sym = sym;
    tk.c0[idx].modifier_set = modifier_set;
    tk.c0[idx].modifier_mask = modifier_mask;
    sym
}

struct ModNames {
    shift: &'static str,
    alt: &'static str,
    ctrl: &'static str,
}

const MODNAMES: [ModNames; 8] = [
    ModNames {
        shift: "S",
        alt: "A",
        ctrl: "C",
    },
    ModNames {
        shift: "Shift",
        alt: "Alt",
        ctrl: "Ctrl",
    },
    ModNames {
        shift: "S",
        alt: "M",
        ctrl: "C",
    },
    ModNames {
        shift: "Shift",
        alt: "Meta",
        ctrl: "Ctrl",
    },
    ModNames {
        shift: "s",
        alt: "a",
        ctrl: "c",
    },
    ModNames {
        shift: "shift",
        alt: "alt",
        ctrl: "ctrl",
    },
    ModNames {
        shift: "s",
        alt: "m",
        ctrl: "c",
    },
    ModNames {
        shift: "shift",
        alt: "meta",
        ctrl: "ctrl",
    },
];

fn append_str(buffer: *mut c_char, len: usize, pos: &mut usize, s: &[u8]) -> bool {
    if s.is_empty() {
        return false;
    }
    if *pos + 1 < len {
        let avail = len - 1 - *pos;
        let to_copy = s.len().min(avail);
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr().cast(), buffer.add(*pos), to_copy);
            *buffer.add(*pos + to_copy) = 0;
        }
    } else if len > 0 && *pos == 0 {
        unsafe {
            *buffer = 0;
        }
    }
    *pos += s.len();
    true
}

fn camel_to_spaces(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() + 4);
    let mut prev_lower = false;
    for &b in src {
        if b.is_ascii_uppercase() && prev_lower {
            out.push(b' ');
        }
        prev_lower = b.is_ascii_lowercase();
        out.push(b.to_ascii_lowercase());
    }
    out
}

pub fn termkey_strfkey(
    tk: &TermKey,
    buffer: *mut c_char,
    len: usize,
    key: &mut TermKeyKey,
    format: TermKeyFormat,
) -> usize {
     {
        let mut pos = 0usize;
        let mod_idx = usize::from((format & TERMKEY_FORMAT_LONGMOD) != 0)
            + usize::from((format & TERMKEY_FORMAT_ALTISMETA) != 0) * 2
            + usize::from((format & TERMKEY_FORMAT_LOWERMOD) != 0) * 4;
        let mods = &MODNAMES[mod_idx];

        let wrapbracket = (format & TERMKEY_FORMAT_WRAPBRACKET) != 0
            && (key.r#type != TERMKEY_TYPE_UNICODE || key.modifiers != 0);

        let sep = if (format & TERMKEY_FORMAT_SPACEMOD) != 0 {
            b' '
        } else {
            b'-'
        };

        if (format & TERMKEY_FORMAT_CARETCTRL) != 0
            && key.r#type == TERMKEY_TYPE_UNICODE
            && key.modifiers == TERMKEY_KEYMOD_CTRL
        {
            let codepoint = key.code.codepoint;
            if (b'a' as c_int..=b'z' as c_int).contains(&codepoint) {
                let ch = (codepoint as u8 - 0x20) as char;
                let s = if wrapbracket {
                    format!("<^{ch}>")
                } else {
                    format!("^{ch}")
                };
                let _ = append_str(buffer, len, &mut pos, s.as_bytes());
                return pos;
            } else if (b'@' as c_int..b'A' as c_int).contains(&codepoint)
                || (b'Z' as c_int + 1..=b'_' as c_int).contains(&codepoint)
            {
                let ch = codepoint as u8 as char;
                let s = if wrapbracket {
                    format!("<^{ch}>")
                } else {
                    format!("^{ch}")
                };
                let _ = append_str(buffer, len, &mut pos, s.as_bytes());
                return pos;
            }
        }

        if wrapbracket && !append_str(buffer, len, &mut pos, b"<") {
            return pos;
        }

        if (key.modifiers & TERMKEY_KEYMOD_ALT) != 0 {
            let mut s = mods.alt.as_bytes().to_vec();
            s.push(sep);
            if !append_str(buffer, len, &mut pos, &s) {
                return pos;
            }
        }

        if (key.modifiers & TERMKEY_KEYMOD_CTRL) != 0 {
            let mut s = mods.ctrl.as_bytes().to_vec();
            s.push(sep);
            if !append_str(buffer, len, &mut pos, &s) {
                return pos;
            }
        }

        if (key.modifiers & TERMKEY_KEYMOD_SHIFT) != 0 {
            let mut s = mods.shift.as_bytes().to_vec();
            s.push(sep);
            if !append_str(buffer, len, &mut pos, &s) {
                return pos;
            }
        }

        let ok = match key.r#type {
            TERMKEY_TYPE_UNICODE => {
                if key.utf8[0] == 0 {
                    fill_utf8(key.code.codepoint, key.utf8.as_mut_ptr());
                }
                let u_bytes = crate::memory::cstr_as_bytes(key.utf8.as_ptr());
                append_str(buffer, len, &mut pos, u_bytes)
            }
            TERMKEY_TYPE_KEYSYM => {
                let name = termkey_get_keyname(tk, key.code.sym);
                let name_bytes = if name.is_null() {
                    &[][..]
                } else {
                    crate::memory::cstr_as_bytes(name)
                };
                if (format & TERMKEY_FORMAT_LOWERSPACE) != 0 {
                    let camel = camel_to_spaces(name_bytes);
                    append_str(buffer, len, &mut pos, &camel)
                } else {
                    append_str(buffer, len, &mut pos, name_bytes)
                }
            }
            TERMKEY_TYPE_FUNCTION => {
                let prefix = if (format & TERMKEY_FORMAT_LOWERSPACE) != 0 {
                    'f'
                } else {
                    'F'
                };
                let s = format!("{prefix}{}", key.code.number);
                append_str(buffer, len, &mut pos, s.as_bytes())
            }
            TERMKEY_TYPE_MOUSE => {
                let mut ev: TermKeyMouseEvent = TERMKEY_MOUSE_UNKNOWN;
                let mut button: c_int = 0;
                let mut line: c_int = 0;
                let mut col: c_int = 0;
                termkey_interpret_mouse(tk, key, &mut ev, &mut button, &mut line, &mut col);

                let s = format!("Mouse{}({button})", EVNAMES[ev as usize]);
                if !append_str(buffer, len, &mut pos, s.as_bytes()) {
                    return pos;
                }
                if (format & TERMKEY_FORMAT_MOUSE_POS) != 0 {
                    let pos_str = format!(" @ ({},{})", col as u32, line as u32);
                    append_str(buffer, len, &mut pos, pos_str.as_bytes())
                } else {
                    true
                }
            }
            TERMKEY_TYPE_POSITION => append_str(buffer, len, &mut pos, b"Position"),
            TERMKEY_TYPE_MODEREPORT => {
                let mut initial: c_int = 0;
                let mut mode: c_int = 0;
                let mut value: c_int = 0;
                termkey_interpret_modereport(tk, key, &mut initial, &mut mode, &mut value);
                let s = if initial != 0 {
                    format!("Mode({}{mode}={value})", initial as u8 as char)
                } else {
                    format!("Mode({mode}={value})")
                };
                append_str(buffer, len, &mut pos, s.as_bytes())
            }
            TERMKEY_TYPE_DCS => append_str(buffer, len, &mut pos, b"DCS"),
            TERMKEY_TYPE_OSC => append_str(buffer, len, &mut pos, b"OSC"),
            TERMKEY_TYPE_APC => append_str(buffer, len, &mut pos, b"APC"),
            TERMKEY_TYPE_UNKNOWN_CSI => {
                let s = format!("CSI {}", (key.code.number & 0xff) as u8 as char);
                append_str(buffer, len, &mut pos, s.as_bytes())
            }
            _ => false,
        };

        if !ok {
            return pos;
        }

        if wrapbracket && !append_str(buffer, len, &mut pos, b">") {
            return pos;
        }

        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::termkey::driver_csi::{
        TERMKEY_MOUSE_DRAG, TERMKEY_MOUSE_PRESS, TERMKEY_MOUSE_RELEASE, TermKeyCsiParam,
        termkey_interpret_csi, termkey_interpret_position,
    };

    fn push(tk: &mut TermKey, bytes: &[u8]) -> usize {
        termkey_push_bytes(tk, bytes.as_ptr().cast(), bytes.len())
    }

    #[test]
    fn base_getkey_flags_and_buffer_resize() {
         {
            let mut tk = termkey_new_abstract(None, 0).unwrap();
            let tk = &mut tk;
            assert_eq!(termkey_get_buffer_size(tk), 256);
            assert_eq!(tk.is_started, 1);

            termkey_stop(tk);
            assert_eq!(tk.is_started, 0);
            termkey_start(tk, None, None);
            assert_eq!(tk.is_started, 1);

            let mut key = TermKeyKey::new();
            assert_eq!(termkey_get_buffer_remaining(tk), 256);
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_NONE);

            assert_eq!(push(tk, b"h"), 1);
            assert_eq!(termkey_get_buffer_remaining(tk), 255);
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_UNICODE);
            assert_eq!(key.code.codepoint, i32::from(b'h'));
            assert_eq!(key.modifiers, 0);
            assert_eq!(crate::memory::cstr_as_bytes(key.utf8.as_ptr()), b"h");
            assert_eq!(termkey_get_buffer_remaining(tk), 256);
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_NONE);

            // Ctrl-A
            push(tk, b"\x01");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_UNICODE);
            assert_eq!(key.code.codepoint, i32::from(b'a'));
            assert_eq!(key.modifiers, TERMKEY_KEYMOD_CTRL);

            // Partial SS3 sequence
            assert_eq!(push(tk, b"\x1bO"), 2);
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_AGAIN);
            push(tk, b"C");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_KEYSYM);
            assert_eq!(key.code.sym, TERMKEY_SYM_RIGHT);

            // Ctrl-Escape via CSI u
            push(tk, b"\x1b[27;5u");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_KEYSYM);
            assert_eq!(key.code.sym, TERMKEY_SYM_ESCAPE);
            assert_eq!(key.modifiers, TERMKEY_KEYMOD_CTRL);

            // Space with and without TERMKEY_FLAG_SPACESYMBOL
            push(tk, b" ");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_UNICODE);
            assert_eq!(key.code.codepoint, i32::from(b' '));

            termkey_set_flags(tk, TERMKEY_FLAG_SPACESYMBOL);
            push(tk, b" ");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_KEYSYM);
            assert_eq!(key.code.sym, TERMKEY_SYM_SPACE);

            // Buffer resize preserves buffered bytes
            push(tk, b"h");
            assert_eq!(termkey_set_buffer_size(tk, 512), 1);
            assert_eq!(termkey_get_buffer_size(tk), 512);
            assert_eq!(termkey_get_buffer_remaining(tk), 511);
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
        }
    }

    #[test]
    fn utf8_decoding_invalid_continuations_and_partials() {
        let mut tk = termkey_new_abstract(None, TERMKEY_FLAG_UTF8).unwrap();
            let tk = &mut tk;
            let mut key = TermKeyKey::new();

            for (bytes, expected_cp) in [
                (&b"a"[..], 0x61),
                (&b"\xC2\xA0"[..], 0x00A0),
                (&b"\xDF\xBF"[..], 0x07FF),
                (&b"\xE0\xA0\x80"[..], 0x0800),
                (&b"\xEF\xBF\xBD"[..], 0xFFFD),
                (&b"\xF0\x90\x80\x80"[..], 0x10000),
                (&b"\xF4\x8F\xBF\xBF"[..], 0x10FFFF),
            ] {
                push(tk, bytes);
                assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
                assert_eq!(key.r#type, TERMKEY_TYPE_UNICODE);
                assert_eq!(key.code.codepoint, expected_cp);
            }

            // Invalid continuations yield U+FFFD followed by the trailing ASCII byte
            for bad in [
                &b"\xC2!"[..],
                &b"\xE0!"[..],
                &b"\xE0\xA0!"[..],
                &b"\xF0!"[..],
                &b"\xF0\x90!"[..],
                &b"\xF0\x90\x80!"[..],
            ] {
                push(tk, bad);
                assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
                assert_eq!(key.code.codepoint, 0xFFFD);
                assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
                assert_eq!(key.code.codepoint, i32::from(b'!'));
            }

            // Partial 4-byte UTF-8 sequence
            push(tk, b"\xF0");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_AGAIN);
            push(tk, b"\x90");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_AGAIN);
            push(tk, b"\x80");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_AGAIN);
            push(tk, b"\x80");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.code.codepoint, 0x10000);
    }

    #[test]
    fn keyname_strfkey_and_canonicalise() {
         {
            let mut tk = termkey_new_abstract(None, 0).unwrap();
            let tk = &mut tk;
            let mut sym: TermKeySym = TERMKEY_SYM_UNKNOWN;

            assert!(termkey_lookup_keyname(tk, c"SomeUnknownKey", &mut sym).is_null());
            let endp = termkey_lookup_keyname(tk, c"DownMore", &mut sym);
            assert!(!endp.is_null());
            assert_eq!(crate::memory::cstr_as_bytes(endp), b"More");
            assert_eq!(sym, TERMKEY_SYM_DOWN);
            assert_eq!(
                crate::memory::cstr_as_bytes(termkey_get_keyname(tk, TERMKEY_SYM_SPACE)),
                b"Space"
            );

            let mut buf = [0 as c_char; 32];
            let mut key = TermKeyKey::new();
            key.r#type = TERMKEY_TYPE_UNICODE;
            key.code.codepoint = i32::from(b'b');
            key.modifiers = TERMKEY_KEYMOD_CTRL;

            assert_eq!(
                termkey_strfkey(tk, buf.as_mut_ptr(), buf.len(), &mut key, 0),
                3
            );
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"C-b");

            assert_eq!(
                termkey_strfkey(
                    tk,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut key,
                    TERMKEY_FORMAT_LONGMOD | TERMKEY_FORMAT_SPACEMOD | TERMKEY_FORMAT_LOWERMOD,
                ),
                6
            );
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"ctrl b");

            assert_eq!(
                termkey_strfkey(
                    tk,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut key,
                    TERMKEY_FORMAT_CARETCTRL,
                ),
                2
            );
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"^B");

            key.r#type = TERMKEY_TYPE_KEYSYM;
            key.code.sym = TERMKEY_SYM_PAGEUP;
            key.modifiers = 0;
            assert_eq!(
                termkey_strfkey(
                    tk,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut key,
                    TERMKEY_FORMAT_LOWERSPACE,
                ),
                7
            );
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"page up");

            key.r#type = TERMKEY_TYPE_FUNCTION;
            key.code.number = 5;
            assert_eq!(
                termkey_strfkey(
                    tk,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut key,
                    TERMKEY_FORMAT_WRAPBRACKET,
                ),
                4
            );
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b"<F5>");

            // Canonicalise space symbol <-> unicode space
            let mut k1 = TermKeyKey::new();
            k1.r#type = TERMKEY_TYPE_KEYSYM;
            k1.code.sym = TERMKEY_SYM_SPACE;
            termkey_canonicalise(tk, &mut k1);
            assert_eq!(k1.r#type, TERMKEY_TYPE_UNICODE);
            assert_eq!(k1.code.codepoint, 0x20);

            termkey_set_canonflags(tk, termkey_get_canonflags(tk) | TERMKEY_CANON_SPACESYMBOL);
            termkey_canonicalise(tk, &mut k1);
            assert_eq!(k1.r#type, TERMKEY_TYPE_KEYSYM);
            assert_eq!(k1.code.sym, TERMKEY_SYM_SPACE);
        }
    }

    #[test]
    fn mouse_position_modereport_csi_and_dcs_osc() {
         {
            let mut tk = termkey_new_abstract(None, 0).unwrap();
            let tk = &mut tk;
            let mut key = TermKeyKey::new();
            let mut ev: TermKeyMouseEvent = TERMKEY_MOUSE_UNKNOWN;
            let mut button: c_int = 0;
            let mut line: c_int = 0;
            let mut col: c_int = 0;
            let mut buf = [0 as c_char; 32];

            // Classic X10 mouse press, drag, release
            push(tk, b"\x1b[M !!");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_MOUSE);
            assert_eq!(
                termkey_interpret_mouse(tk, &key, &mut ev, &mut button, &mut line, &mut col),
                TERMKEY_RES_KEY
            );
            assert_eq!((ev, button, line, col), (TERMKEY_MOUSE_PRESS, 1, 1, 1));
            assert_eq!(
                termkey_strfkey(
                    tk,
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut key,
                    TERMKEY_FORMAT_MOUSE_POS,
                ),
                21
            );
            assert_eq!(
                crate::memory::cstr_as_bytes(buf.as_ptr()),
                b"MousePress(1) @ (1,1)"
            );

            push(tk, b"\x1b[M@\"!");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            termkey_interpret_mouse(tk, &key, &mut ev, &mut button, &mut line, &mut col);
            assert_eq!((ev, button, line, col), (TERMKEY_MOUSE_DRAG, 1, 1, 2));

            // SGR wide mouse press and release
            push(tk, b"\x1b[<0;500;300M\x1b[<0;30;30m");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            termkey_interpret_mouse(tk, &key, &mut ev, &mut button, &mut line, &mut col);
            assert_eq!((ev, button, line, col), (TERMKEY_MOUSE_PRESS, 1, 300, 500));
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            termkey_interpret_mouse(tk, &key, &mut ev, &mut button, &mut line, &mut col);
            assert_eq!(ev, TERMKEY_MOUSE_RELEASE);

            // Position report and plain CSI R (<F3>)
            push(tk, b"\x1b[?15;7R\x1b[R");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_POSITION);
            assert_eq!(
                termkey_interpret_position(tk, &key, &mut line, &mut col),
                TERMKEY_RES_KEY
            );
            assert_eq!((line, col), (15, 7));

            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_FUNCTION);
            assert_eq!(key.code.number, 3);

            // Mode report
            let mut initial: c_int = 0;
            let mut mode: c_int = 0;
            let mut value: c_int = 0;
            push(tk, b"\x1b[?1;2$y\x1b[4;1$y");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_MODEREPORT);
            assert_eq!(
                termkey_interpret_modereport(tk, &key, &mut initial, &mut mode, &mut value),
                TERMKEY_RES_KEY
            );
            assert_eq!((initial, mode, value), (b'?' as c_int, 1, 2));

            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            termkey_interpret_modereport(tk, &key, &mut initial, &mut mode, &mut value);
            assert_eq!((initial, mode, value), (0, 4, 1));

            // Unknown CSI
            let mut args: [TermKeyCsiParam; 16] = [TermKeyCsiParam::new(); 16];
            let mut nargs: usize = 16;
            let mut command: std::ffi::c_uint = 0;
            push(tk, b"\x1b[5;25v");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_UNKNOWN_CSI);
            assert_eq!(
                termkey_interpret_csi(tk, &key, &mut args, &mut nargs, &mut command),
                TERMKEY_RES_KEY
            );
            assert_eq!((nargs, command), (2, u32::from(b'v')));

            // 7-bit DCS, 8-bit DCS, 7-bit OSC, and false alarm
            let mut strp: *const c_char = std::ptr::null();
            push(tk, b"\x1bP1$r1 q\x1b\\");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_DCS);
            assert_eq!(
                termkey_interpret_string(tk, &key, &mut strp),
                TERMKEY_RES_KEY
            );
            assert_eq!(crate::memory::cstr_as_bytes(strp), b"1$r1 q");

            push(tk, b"\x901$r2 q\x9c");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_DCS);
            assert_eq!(
                termkey_interpret_string(tk, &key, &mut strp),
                TERMKEY_RES_KEY
            );
            assert_eq!(crate::memory::cstr_as_bytes(strp), b"1$r2 q");

            push(tk, b"\x1b]15;abc\x1b\\");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_OSC);
            assert_eq!(
                termkey_interpret_string(tk, &key, &mut strp),
                TERMKEY_RES_KEY
            );
            assert_eq!(crate::memory::cstr_as_bytes(strp), b"15;abc");

            push(tk, b"\x1bP");
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_AGAIN);
            assert_eq!(termkey_getkey_force(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_UNICODE);
            assert_eq!(key.code.codepoint, i32::from(b'P'));
            assert_eq!(key.modifiers, TERMKEY_KEYMOD_ALT);
        }
    }
}
