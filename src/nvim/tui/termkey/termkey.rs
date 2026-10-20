//! Core libtermkey state machine and key formatting (`src/nvim/tui/termkey/termkey.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::mbyte::{utf_char2bytes, utf_char2len};
use crate::memory::{xfree, xmalloc, xrealloc};
use crate::tui::termkey::driver_csi::{
    TERMKEY_MOUSE_UNKNOWN, TermKeyCsi, TermKeyMouseEvent, free_driver_csi, new_driver_csi,
    peekkey_csi, termkey_interpret_modereport, termkey_interpret_mouse,
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
    TERMKEY_TYPE_UNICODE, TERMKEY_TYPE_UNKNOWN_CSI, TermKey, TermKeyDriver, TermKeyDriverNode,
    TermKeyKey, TermKeyMethods, TermKeyResult, TermKeySym, TermKeyTerminfoGetstrHook,
    TerminfoEntry, free_driver_ti, keyinfo, new_driver_ti, peekkey_ti, start_driver_ti,
    stop_driver_ti,
};
use std::ffi::{CStr, c_char, c_int, c_uchar, c_void};

pub const TERMKEY_FLAG_NOINTERPRET: c_int = 1 << 0;
pub const TERMKEY_FLAG_RAW: c_int = 1 << 2;
pub const TERMKEY_FLAG_UTF8: c_int = 1 << 3;
pub const TERMKEY_FLAG_NOTERMIOS: c_int = 1 << 4;
pub const TERMKEY_FLAG_SPACESYMBOL: c_int = 1 << 5;
pub const TERMKEY_FLAG_CTRLC: c_int = 1 << 6;
pub const TERMKEY_FLAG_EINTR: c_int = 1 << 7;
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

static mut TERMKEY_DRIVER_TI: TermKeyDriver = TermKeyDriver {
    name: c"terminfo".as_ptr(),
    new_driver: Some(new_driver_ti),
    free_driver: Some(free_driver_ti),
    start_driver: Some(start_driver_ti),
    stop_driver: Some(stop_driver_ti),
    peekkey: Some(peekkey_ti),
};

static mut TERMKEY_DRIVER_CSI: TermKeyDriver = TermKeyDriver {
    name: c"CSI".as_ptr(),
    new_driver: Some(new_driver_csi),
    free_driver: Some(free_driver_csi),
    start_driver: None,
    stop_driver: None,
    peekkey: Some(peekkey_csi),
};

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_interpret_string(
    tk: *mut TermKey,
    key: *const TermKeyKey,
    strp: *mut *const c_char,
) -> TermKeyResult {
    unsafe {
        let mut p = (*tk).drivers;
        while !p.is_null() {
            if std::ptr::eq((*p).driver, std::ptr::addr_of_mut!(TERMKEY_DRIVER_CSI)) {
                break;
            }
            p = (*p).next;
        }

        if p.is_null() {
            return TERMKEY_RES_NONE;
        }

        if (*key).r#type != TERMKEY_TYPE_DCS
            && (*key).r#type != TERMKEY_TYPE_OSC
            && (*key).r#type != TERMKEY_TYPE_APC
        {
            return TERMKEY_RES_NONE;
        }

        let csi = (*p).info.cast::<TermKeyCsi>();
        if (*csi).saved_string_id != (*key).code.number {
            return TERMKEY_RES_NONE;
        }

        *strp = (*csi).saved_string;
        TERMKEY_RES_KEY
    }
}

unsafe fn strpncmp_camel(
    strp: *mut *const c_char,
    strcamelp: *mut *const c_char,
    mut n: usize,
) -> c_int {
    unsafe {
        let mut s = *strp;
        let mut sc = *strcamelp;
        let mut prev_lower = false;

        while (*s != 0 || *sc != 0) && n != 0 {
            let sc_u8 = *sc as u8;
            let b = sc_u8.to_ascii_lowercase() as c_char;
            if sc_u8.is_ascii_uppercase() && prev_lower {
                if *s != b' ' as c_char {
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

unsafe fn termkey_alloc() -> *mut TermKey {
    unsafe {
        let tk = xmalloc(std::mem::size_of::<TermKey>()).cast::<TermKey>();
        (*tk).fd = -1;
        (*tk).flags = 0;
        (*tk).canonflags = 0;
        (*tk).buffer = std::ptr::null_mut();
        (*tk).buffstart = 0;
        (*tk).buffcount = 0;
        (*tk).buffsize = 256;
        (*tk).hightide = 0;
        (*tk).ti_getstr_hook = None;
        (*tk).ti_getstr_hook_data = std::ptr::null_mut();
        (*tk).waittime = 50;
        (*tk).is_closed = 0;
        (*tk).is_started = 0;
        (*tk).nkeynames = 64;
        (*tk).keynames = std::ptr::null_mut();
        for i in 0..32 {
            (*tk).c0[i] = keyinfo {
                r#type: 0,
                sym: TERMKEY_SYM_NONE,
                modifier_mask: 0,
                modifier_set: 0,
            };
        }
        (*tk).drivers = std::ptr::null_mut();
        (*tk).method = TermKeyMethods {
            emit_codepoint: Some(emit_codepoint),
            peekkey_simple: Some(peekkey_simple),
            peekkey_mouse: Some(peekkey_mouse),
        };
        tk
    }
}

unsafe fn termkey_init(tk: *mut TermKey, term: *mut TerminfoEntry) -> c_int {
    unsafe {
        (*tk).buffer = xmalloc((*tk).buffsize).cast::<c_uchar>();
        (*tk).keynames = xmalloc(std::mem::size_of::<*const c_char>() * ((*tk).nkeynames as usize))
            .cast::<*const c_char>();

        for i in 0..(*tk).nkeynames as usize {
            *(*tk).keynames.add(i) = std::ptr::null();
        }

        for entry in KEYNAMES {
            if termkey_register_keyname(tk, entry.sym, entry.name.as_ptr()) == -1 {
                xfree((*tk).keynames.cast());
                xfree((*tk).buffer.cast());
                return 0;
            }
        }

        register_c0(tk, TERMKEY_SYM_TAB, 0x09, std::ptr::null());
        register_c0(tk, TERMKEY_SYM_ENTER, 0x0d, std::ptr::null());
        register_c0(tk, TERMKEY_SYM_ESCAPE, 0x1b, std::ptr::null());

        let mut tail: *mut TermKeyDriverNode = std::ptr::null_mut();
        let drivers_list = [
            std::ptr::addr_of_mut!(TERMKEY_DRIVER_TI),
            std::ptr::addr_of_mut!(TERMKEY_DRIVER_CSI),
        ];

        for &drv in &drivers_list {
            let Some(new_fn) = (*drv).new_driver else {
                continue;
            };
            let info = new_fn(tk, term);
            if info.is_null() {
                continue;
            }

            let thisdrv =
                xmalloc(std::mem::size_of::<TermKeyDriverNode>()).cast::<TermKeyDriverNode>();
            (*thisdrv).driver = drv;
            (*thisdrv).info = info;
            (*thisdrv).next = std::ptr::null_mut();

            if tail.is_null() {
                (*tk).drivers = thisdrv;
            } else {
                (*tail).next = thisdrv;
            }
            tail = thisdrv;
        }

        if (*tk).drivers.is_null() {
            xfree((*tk).keynames.cast());
            xfree((*tk).buffer.cast());
            return 0;
        }

        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_new_abstract(
    term: *mut TerminfoEntry,
    flags: c_int,
) -> *mut TermKey {
    unsafe {
        let tk = termkey_alloc();
        if tk.is_null() {
            return std::ptr::null_mut();
        }

        (*tk).fd = -1;
        termkey_set_flags(tk, flags);

        if termkey_init(tk, term) == 0 {
            xfree(tk.cast());
            return std::ptr::null_mut();
        }

        if (flags & TERMKEY_FLAG_NOSTART) == 0 && termkey_start(tk) == 0 {
            xfree(tk.cast());
            return std::ptr::null_mut();
        }

        tk
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_free(tk: *mut TermKey) {
    unsafe {
        xfree((*tk).buffer.cast());
        (*tk).buffer = std::ptr::null_mut();
        xfree((*tk).keynames.cast());
        (*tk).keynames = std::ptr::null_mut();

        let mut p = (*tk).drivers;
        while !p.is_null() {
            if let Some(free_fn) = (*(*p).driver).free_driver {
                free_fn((*p).info);
            }
            let next = (*p).next;
            xfree(p.cast());
            p = next;
        }

        xfree(tk.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_destroy(tk: *mut TermKey) {
    unsafe {
        if (*tk).is_started != 0 {
            termkey_stop(tk);
        }
        termkey_free(tk);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_hook_terminfo_getstr(
    tk: *mut TermKey,
    hookfn: TermKeyTerminfoGetstrHook,
    data: *mut c_void,
) {
    unsafe {
        (*tk).ti_getstr_hook = hookfn;
        (*tk).ti_getstr_hook_data = data;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_start(tk: *mut TermKey) -> c_int {
    unsafe {
        if (*tk).is_started != 0 {
            return 1;
        }

        let mut p = (*tk).drivers;
        while !p.is_null() {
            if let Some(start_fn) = (*(*p).driver).start_driver
                && start_fn(tk, (*p).info) == 0
            {
                return 0;
            }
            p = (*p).next;
        }

        (*tk).is_started = 1;
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_stop(tk: *mut TermKey) -> c_int {
    unsafe {
        if (*tk).is_started == 0 {
            return 1;
        }

        let mut p = (*tk).drivers;
        while !p.is_null() {
            if let Some(stop_fn) = (*(*p).driver).stop_driver {
                stop_fn(tk, (*p).info);
            }
            p = (*p).next;
        }

        (*tk).is_started = 0;
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_set_flags(tk: *mut TermKey, newflags: c_int) {
    unsafe {
        (*tk).flags = newflags;
        if ((*tk).flags & TERMKEY_FLAG_SPACESYMBOL) != 0 {
            (*tk).canonflags |= TERMKEY_CANON_SPACESYMBOL;
        } else {
            (*tk).canonflags &= !TERMKEY_CANON_SPACESYMBOL;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_get_canonflags(tk: *mut TermKey) -> c_int {
    unsafe { (*tk).canonflags }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_set_canonflags(tk: *mut TermKey, flags: c_int) {
    unsafe {
        (*tk).canonflags = flags;
        if ((*tk).canonflags & TERMKEY_CANON_SPACESYMBOL) != 0 {
            (*tk).flags |= TERMKEY_FLAG_SPACESYMBOL;
        } else {
            (*tk).flags &= !TERMKEY_FLAG_SPACESYMBOL;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_get_buffer_size(tk: *mut TermKey) -> usize {
    unsafe { (*tk).buffsize }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_set_buffer_size(tk: *mut TermKey, size: usize) -> c_int {
    unsafe {
        let buffer = xrealloc((*tk).buffer.cast(), size).cast::<c_uchar>();
        (*tk).buffer = buffer;
        (*tk).buffsize = size;
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_get_buffer_remaining(tk: *mut TermKey) -> usize {
    unsafe { (*tk).buffsize - (*tk).buffcount }
}

#[inline]
unsafe fn eat_bytes(tk: *mut TermKey, count: usize) {
    unsafe {
        if count >= (*tk).buffcount {
            (*tk).buffstart = 0;
            (*tk).buffcount = 0;
        } else {
            (*tk).buffstart += count;
            (*tk).buffcount -= count;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fill_utf8(codepoint: c_int, str_out: *mut c_char) -> c_int {
    unsafe {
        let nbytes = utf_char2bytes(codepoint, str_out);
        *str_out.add(nbytes as usize) = 0;
        nbytes
    }
}

unsafe fn parse_utf8(
    bytes: *const c_uchar,
    len: usize,
    cp: *mut c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        let b0 = *bytes;
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

        for b in 1..nbytes {
            if b >= len {
                return TERMKEY_RES_AGAIN;
            }
            let cb = *bytes.add(b);
            if !(0x80..0xc0).contains(&cb) {
                *cp = UNICODE_INVALID;
                *nbytep = b;
                return TERMKEY_RES_KEY;
            }
            *cp = (*cp << 6) | c_int::from(cb & 0x3f);
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
}

unsafe extern "C" fn emit_codepoint(tk: *mut TermKey, codepoint: c_int, key: *mut TermKeyKey) {
    unsafe {
        if codepoint == 0 {
            (*key).r#type = TERMKEY_TYPE_KEYSYM;
            (*key).code.sym = TERMKEY_SYM_SPACE;
            (*key).modifiers = TERMKEY_KEYMOD_CTRL;
        } else if codepoint < 0x20 && ((*tk).flags & TERMKEY_FLAG_KEEPC0) == 0 {
            (*key).code.codepoint = 0;
            (*key).modifiers = 0;

            let idx = codepoint as usize;
            if ((*tk).flags & TERMKEY_FLAG_NOINTERPRET) == 0
                && (*tk).c0[idx].sym != TERMKEY_SYM_UNKNOWN
            {
                (*key).code.sym = (*tk).c0[idx].sym;
                (*key).modifiers |= (*tk).c0[idx].modifier_set;
            }

            if (*key).code.sym == 0 {
                (*key).r#type = TERMKEY_TYPE_UNICODE;
                if (b'A' as c_int..=b'Z' as c_int).contains(&(codepoint + 0x40)) {
                    (*key).code.codepoint = codepoint + 0x60;
                } else {
                    (*key).code.codepoint = codepoint + 0x40;
                }
                (*key).modifiers = TERMKEY_KEYMOD_CTRL;
            } else {
                (*key).r#type = TERMKEY_TYPE_KEYSYM;
            }
        } else if codepoint == 0x7f && ((*tk).flags & TERMKEY_FLAG_NOINTERPRET) == 0 {
            (*key).r#type = TERMKEY_TYPE_KEYSYM;
            (*key).code.sym = TERMKEY_SYM_DEL;
            (*key).modifiers = 0;
        } else if codepoint > 0 && codepoint < 0x80 {
            (*key).r#type = TERMKEY_TYPE_UNICODE;
            (*key).code.codepoint = codepoint;
            (*key).modifiers = 0;
        } else if (0x80..0xa0).contains(&codepoint) {
            (*key).r#type = TERMKEY_TYPE_UNICODE;
            (*key).code.codepoint = codepoint - 0x40;
            (*key).modifiers = TERMKEY_KEYMOD_CTRL | TERMKEY_KEYMOD_ALT;
        } else {
            (*key).r#type = TERMKEY_TYPE_UNICODE;
            (*key).code.codepoint = codepoint;
            (*key).modifiers = 0;
        }

        termkey_canonicalise(tk, key);

        if (*key).r#type == TERMKEY_TYPE_UNICODE {
            fill_utf8((*key).code.codepoint, (*key).utf8.as_mut_ptr());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_canonicalise(tk: *mut TermKey, key: *mut TermKeyKey) {
    unsafe {
        let flags = (*tk).canonflags;

        if (flags & TERMKEY_CANON_SPACESYMBOL) != 0 {
            if (*key).r#type == TERMKEY_TYPE_UNICODE && (*key).code.codepoint == 0x20 {
                (*key).r#type = TERMKEY_TYPE_KEYSYM;
                (*key).code.sym = TERMKEY_SYM_SPACE;
            }
        } else if (*key).r#type == TERMKEY_TYPE_KEYSYM && (*key).code.sym == TERMKEY_SYM_SPACE {
            (*key).r#type = TERMKEY_TYPE_UNICODE;
            (*key).code.codepoint = 0x20;
            fill_utf8((*key).code.codepoint, (*key).utf8.as_mut_ptr());
        }

        if (flags & TERMKEY_CANON_DELBS) != 0
            && (*key).r#type == TERMKEY_TYPE_KEYSYM
            && (*key).code.sym == TERMKEY_SYM_DEL
        {
            (*key).code.sym = TERMKEY_SYM_BACKSPACE;
        }
    }
}

unsafe fn peekkey(
    tk: *mut TermKey,
    key: *mut TermKeyKey,
    force: c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        let mut again = false;

        if (*tk).is_started == 0 {
            return TERMKEY_RES_ERROR;
        }

        (*key).event = TERMKEY_EVENT_PRESS;

        if (*tk).hightide != 0 {
            (*tk).buffstart += (*tk).hightide;
            (*tk).buffcount -= (*tk).hightide;
            (*tk).hightide = 0;
        }

        let mut p = (*tk).drivers;
        while !p.is_null() {
            let ret =
                ((*(*p).driver).peekkey.unwrap_unchecked())(tk, (*p).info, key, force, nbytep);
            match ret {
                TERMKEY_RES_KEY => {
                    let halfsize = (*tk).buffsize / 2;
                    if (*tk).buffstart > halfsize {
                        std::ptr::copy((*tk).buffer.add(halfsize), (*tk).buffer, halfsize);
                        (*tk).buffstart -= halfsize;
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
            p = (*p).next;
        }

        if again {
            return TERMKEY_RES_AGAIN;
        }

        peekkey_simple(tk, key, force, nbytep)
    }
}

unsafe extern "C" fn peekkey_simple(
    tk: *mut TermKey,
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

        let b0 = *(*tk).buffer.add((*tk).buffstart);

        if b0 == 0x1b {
            if (*tk).buffcount == 1 {
                if force == 0 {
                    return TERMKEY_RES_AGAIN;
                }
                ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, c_int::from(b0), key);
                *nbytep = 1;
                return TERMKEY_RES_KEY;
            }

            (*tk).buffstart += 1;
            (*tk).buffcount -= 1;

            let metakey_result = peekkey(tk, key, force, nbytep);

            (*tk).buffstart -= 1;
            (*tk).buffcount += 1;

            if metakey_result == TERMKEY_RES_KEY {
                if (*key).event == TERMKEY_EVENT_REPEAT
                    || (*key).event == TERMKEY_EVENT_RELEASE
                    || (*key).r#type == TERMKEY_TYPE_DCS
                    || (*key).r#type == TERMKEY_TYPE_OSC
                    || (*key).r#type == TERMKEY_TYPE_APC
                {
                    (*key).event = TERMKEY_EVENT_PRESS;
                    ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, c_int::from(b0), key);
                    *nbytep = 1;
                    return TERMKEY_RES_KEY;
                }
                (*key).modifiers |= TERMKEY_KEYMOD_ALT;
                *nbytep += 1;
            }

            metakey_result
        } else if b0 < 0xa0 {
            ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, c_int::from(b0), key);
            *nbytep = 1;
            TERMKEY_RES_KEY
        } else if ((*tk).flags & TERMKEY_FLAG_UTF8) != 0 {
            let mut codepoint: c_int = 0;
            let mut res = parse_utf8(
                (*tk).buffer.add((*tk).buffstart),
                (*tk).buffcount,
                &mut codepoint,
                nbytep,
            );

            if res == TERMKEY_RES_AGAIN && force != 0 {
                codepoint = UNICODE_INVALID;
                *nbytep = (*tk).buffcount;
                res = TERMKEY_RES_KEY;
            }

            (*key).r#type = TERMKEY_TYPE_UNICODE;
            (*key).modifiers = 0;
            ((*tk).method.emit_codepoint.unwrap_unchecked())(tk, codepoint, key);
            res
        } else {
            (*key).r#type = TERMKEY_TYPE_UNICODE;
            (*key).code.codepoint = c_int::from(b0);
            (*key).modifiers = 0;
            (*key).utf8[0] = b0 as c_char;
            (*key).utf8[1] = 0;
            *nbytep = 1;
            TERMKEY_RES_KEY
        }
    }
}

unsafe extern "C" fn peekkey_mouse(
    tk: *mut TermKey,
    key: *mut TermKeyKey,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        if (*tk).buffcount < 3 {
            return TERMKEY_RES_AGAIN;
        }

        let buf = (*tk).buffer.add((*tk).buffstart);
        (*key).r#type = TERMKEY_TYPE_MOUSE;
        (*key).code.mouse[0] = (*buf).wrapping_sub(0x20) as c_char;
        (*key).code.mouse[1] = (*buf.add(1)).wrapping_sub(0x20) as c_char;
        (*key).code.mouse[2] = (*buf.add(2)).wrapping_sub(0x20) as c_char;
        (*key).code.mouse[3] = 0;

        (*key).modifiers = c_int::from(((*key).code.mouse[0] as u8 & 0x1c) >> 2);
        (*key).code.mouse[0] = ((*key).code.mouse[0] as u8 & !0x1c) as c_char;

        *nbytep = 3;
        TERMKEY_RES_KEY
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_getkey(tk: *mut TermKey, key: *mut TermKeyKey) -> TermKeyResult {
    unsafe {
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
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_getkey_force(
    tk: *mut TermKey,
    key: *mut TermKeyKey,
) -> TermKeyResult {
    unsafe {
        let mut nbytes: usize = 0;
        let ret = peekkey(tk, key, 1, &mut nbytes);

        if ret == TERMKEY_RES_KEY {
            eat_bytes(tk, nbytes);
        }

        ret
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_push_bytes(
    tk: *mut TermKey,
    bytes: *const c_char,
    mut len: usize,
) -> usize {
    unsafe {
        if (*tk).buffstart != 0 {
            std::ptr::copy(
                (*tk).buffer.add((*tk).buffstart),
                (*tk).buffer,
                (*tk).buffcount,
            );
            (*tk).buffstart = 0;
        }

        if (*tk).buffcount >= (*tk).buffsize {
            return usize::MAX;
        }

        if len > (*tk).buffsize - (*tk).buffcount {
            len = (*tk).buffsize - (*tk).buffcount;
        }

        std::ptr::copy_nonoverlapping(
            bytes.cast::<c_uchar>(),
            (*tk).buffer.add((*tk).buffcount),
            len,
        );
        (*tk).buffcount += len;

        len
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_register_keyname(
    tk: *mut TermKey,
    mut sym: TermKeySym,
    name: *const c_char,
) -> TermKeySym {
    unsafe {
        if sym == 0 {
            sym = (*tk).nkeynames;
        }

        if sym >= (*tk).nkeynames {
            let new_keynames = xrealloc(
                (*tk).keynames.cast(),
                std::mem::size_of::<*const c_char>() * ((sym as usize) + 1),
            )
            .cast::<*const c_char>();
            (*tk).keynames = new_keynames;

            for i in (*tk).nkeynames..sym {
                *(*tk).keynames.add(i as usize) = std::ptr::null();
            }

            (*tk).nkeynames = sym + 1;
        }

        *(*tk).keynames.add(sym as usize) = name;
        sym
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_get_keyname(tk: *mut TermKey, sym: TermKeySym) -> *const c_char {
    if sym == TERMKEY_SYM_UNKNOWN {
        return c"UNKNOWN".as_ptr();
    }
    unsafe {
        if sym >= 0 && sym < (*tk).nkeynames {
            *(*tk).keynames.add(sym as usize)
        } else {
            c"UNKNOWN".as_ptr()
        }
    }
}

unsafe fn termkey_lookup_keyname_format(
    tk: *mut TermKey,
    str_in: *const c_char,
    sym: *mut TermKeySym,
    format: TermKeyFormat,
) -> *const c_char {
    unsafe {
        *sym = 0;
        while *sym < (*tk).nkeynames {
            let mut thiskey = *(*tk).keynames.add(*sym as usize);
            if !thiskey.is_null() {
                let len = CStr::from_ptr(thiskey).to_bytes().len();
                if (format & TERMKEY_FORMAT_LOWERSPACE) != 0 {
                    let mut thisstr = str_in;
                    if strpncmp_camel(&mut thisstr, &mut thiskey, len) == 0 {
                        return thisstr;
                    }
                } else {
                    let key_bytes = std::slice::from_raw_parts(thiskey.cast::<u8>(), len);
                    let mut matched = true;
                    for (i, &kb) in key_bytes.iter().enumerate() {
                        if *str_in.add(i) as u8 != kb {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_lookup_keyname(
    tk: *mut TermKey,
    str_in: *const c_char,
    sym: *mut TermKeySym,
) -> *const c_char {
    unsafe { termkey_lookup_keyname_format(tk, str_in, sym, 0) }
}

unsafe fn register_c0(
    tk: *mut TermKey,
    sym: TermKeySym,
    ctrl: u8,
    name: *const c_char,
) -> TermKeySym {
    unsafe { register_c0_full(tk, sym, 0, 0, ctrl, name) }
}

unsafe fn register_c0_full(
    tk: *mut TermKey,
    mut sym: TermKeySym,
    modifier_set: c_int,
    modifier_mask: c_int,
    ctrl: u8,
    name: *const c_char,
) -> TermKeySym {
    if ctrl >= 0x20 {
        return -1;
    }
    unsafe {
        if !name.is_null() {
            sym = termkey_register_keyname(tk, sym, name);
        }
        let idx = usize::from(ctrl);
        (*tk).c0[idx].sym = sym;
        (*tk).c0[idx].modifier_set = modifier_set;
        (*tk).c0[idx].modifier_mask = modifier_mask;
        sym
    }
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

unsafe fn append_str(buffer: *mut c_char, len: usize, pos: &mut usize, s: &[u8]) -> bool {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn termkey_strfkey(
    tk: *mut TermKey,
    buffer: *mut c_char,
    len: usize,
    key: *mut TermKeyKey,
    format: TermKeyFormat,
) -> usize {
    unsafe {
        let mut pos = 0usize;
        let mod_idx = usize::from((format & TERMKEY_FORMAT_LONGMOD) != 0)
            + usize::from((format & TERMKEY_FORMAT_ALTISMETA) != 0) * 2
            + usize::from((format & TERMKEY_FORMAT_LOWERMOD) != 0) * 4;
        let mods = &MODNAMES[mod_idx];

        let wrapbracket = (format & TERMKEY_FORMAT_WRAPBRACKET) != 0
            && ((*key).r#type != TERMKEY_TYPE_UNICODE || (*key).modifiers != 0);

        let sep = if (format & TERMKEY_FORMAT_SPACEMOD) != 0 {
            b' '
        } else {
            b'-'
        };

        if (format & TERMKEY_FORMAT_CARETCTRL) != 0
            && (*key).r#type == TERMKEY_TYPE_UNICODE
            && (*key).modifiers == TERMKEY_KEYMOD_CTRL
        {
            let codepoint = (*key).code.codepoint;
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

        if ((*key).modifiers & TERMKEY_KEYMOD_ALT) != 0 {
            let mut s = mods.alt.as_bytes().to_vec();
            s.push(sep);
            if !append_str(buffer, len, &mut pos, &s) {
                return pos;
            }
        }

        if ((*key).modifiers & TERMKEY_KEYMOD_CTRL) != 0 {
            let mut s = mods.ctrl.as_bytes().to_vec();
            s.push(sep);
            if !append_str(buffer, len, &mut pos, &s) {
                return pos;
            }
        }

        if ((*key).modifiers & TERMKEY_KEYMOD_SHIFT) != 0 {
            let mut s = mods.shift.as_bytes().to_vec();
            s.push(sep);
            if !append_str(buffer, len, &mut pos, &s) {
                return pos;
            }
        }

        let ok = match (*key).r#type {
            TERMKEY_TYPE_UNICODE => {
                if (*key).utf8[0] == 0 {
                    fill_utf8((*key).code.codepoint, (*key).utf8.as_mut_ptr());
                }
                let u_bytes = CStr::from_ptr((*key).utf8.as_ptr()).to_bytes();
                append_str(buffer, len, &mut pos, u_bytes)
            }
            TERMKEY_TYPE_KEYSYM => {
                let name = termkey_get_keyname(tk, (*key).code.sym);
                let name_bytes = if name.is_null() {
                    &[][..]
                } else {
                    CStr::from_ptr(name).to_bytes()
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
                let s = format!("{prefix}{}", (*key).code.number);
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
                let s = format!("CSI {}", ((*key).code.number & 0xff) as u8 as char);
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

    #[test]
    fn termkey_lifecycle_push_getkey_and_strfkey() {
        unsafe {
            let tk = termkey_new_abstract(std::ptr::null_mut(), 0);
            assert!(!tk.is_null());
            assert_eq!(termkey_get_buffer_size(tk), 256);

            let input = c"\x1b[A";
            assert_eq!(termkey_push_bytes(tk, input.as_ptr(), 3), 3);

            let mut key: TermKeyKey = std::mem::zeroed();
            assert_eq!(termkey_getkey(tk, &mut key), TERMKEY_RES_KEY);
            assert_eq!(key.r#type, TERMKEY_TYPE_KEYSYM);
            assert_eq!(key.code.sym, TERMKEY_SYM_UP);

            let mut buf = [0 as c_char; 16];
            let len = termkey_strfkey(
                tk,
                buf.as_mut_ptr(),
                buf.len(),
                &mut key,
                TERMKEY_FORMAT_WRAPBRACKET,
            );
            assert_eq!(len, 4);
            assert_eq!(CStr::from_ptr(buf.as_ptr()), c"<Up>");

            termkey_destroy(tk);
        }
    }
}
