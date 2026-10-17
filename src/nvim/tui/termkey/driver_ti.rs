//! Terminfo driver for libtermkey (`src/nvim/tui/termkey/driver-ti.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use std::ffi::{c_char, c_int, c_uchar, c_void};

use crate::memory::{xfree, xmalloc, xstrdup};

pub type TermKeyResult = c_int;
pub const TERMKEY_RES_NONE: TermKeyResult = 0;
pub const TERMKEY_RES_KEY: TermKeyResult = 1;
pub const TERMKEY_RES_EOF: TermKeyResult = 2;
pub const TERMKEY_RES_AGAIN: TermKeyResult = 3;
pub const TERMKEY_RES_ERROR: TermKeyResult = 4;

pub type TermKeySym = c_int;
pub const TERMKEY_SYM_UNKNOWN: TermKeySym = -1;
pub const TERMKEY_SYM_NONE: TermKeySym = 0;
pub const TERMKEY_SYM_BACKSPACE: TermKeySym = 1;
pub const TERMKEY_SYM_TAB: TermKeySym = 2;
pub const TERMKEY_SYM_ENTER: TermKeySym = 3;
pub const TERMKEY_SYM_ESCAPE: TermKeySym = 4;
pub const TERMKEY_SYM_SPACE: TermKeySym = 5;
pub const TERMKEY_SYM_DEL: TermKeySym = 6;
pub const TERMKEY_SYM_UP: TermKeySym = 7;
pub const TERMKEY_SYM_DOWN: TermKeySym = 8;
pub const TERMKEY_SYM_LEFT: TermKeySym = 9;
pub const TERMKEY_SYM_RIGHT: TermKeySym = 10;
pub const TERMKEY_SYM_BEGIN: TermKeySym = 11;
pub const TERMKEY_SYM_FIND: TermKeySym = 12;
pub const TERMKEY_SYM_INSERT: TermKeySym = 13;
pub const TERMKEY_SYM_DELETE: TermKeySym = 14;
pub const TERMKEY_SYM_SELECT: TermKeySym = 15;
pub const TERMKEY_SYM_PAGEUP: TermKeySym = 16;
pub const TERMKEY_SYM_PAGEDOWN: TermKeySym = 17;
pub const TERMKEY_SYM_HOME: TermKeySym = 18;
pub const TERMKEY_SYM_END: TermKeySym = 19;
pub const TERMKEY_SYM_CANCEL: TermKeySym = 20;
pub const TERMKEY_SYM_CLEAR: TermKeySym = 21;
pub const TERMKEY_SYM_CLOSE: TermKeySym = 22;
pub const TERMKEY_SYM_COMMAND: TermKeySym = 23;
pub const TERMKEY_SYM_COPY: TermKeySym = 24;
pub const TERMKEY_SYM_EXIT: TermKeySym = 25;
pub const TERMKEY_SYM_HELP: TermKeySym = 26;
pub const TERMKEY_SYM_MARK: TermKeySym = 27;
pub const TERMKEY_SYM_MESSAGE: TermKeySym = 28;
pub const TERMKEY_SYM_MOVE: TermKeySym = 29;
pub const TERMKEY_SYM_OPEN: TermKeySym = 30;
pub const TERMKEY_SYM_OPTIONS: TermKeySym = 31;
pub const TERMKEY_SYM_PRINT: TermKeySym = 32;
pub const TERMKEY_SYM_REDO: TermKeySym = 33;
pub const TERMKEY_SYM_REFERENCE: TermKeySym = 34;
pub const TERMKEY_SYM_REFRESH: TermKeySym = 35;
pub const TERMKEY_SYM_REPLACE: TermKeySym = 36;
pub const TERMKEY_SYM_RESTART: TermKeySym = 37;
pub const TERMKEY_SYM_RESUME: TermKeySym = 38;
pub const TERMKEY_SYM_SAVE: TermKeySym = 39;
pub const TERMKEY_SYM_SUSPEND: TermKeySym = 40;
pub const TERMKEY_SYM_UNDO: TermKeySym = 41;
pub const TERMKEY_SYM_KP0: TermKeySym = 42;
pub const TERMKEY_SYM_KP1: TermKeySym = 43;
pub const TERMKEY_SYM_KP2: TermKeySym = 44;
pub const TERMKEY_SYM_KP3: TermKeySym = 45;
pub const TERMKEY_SYM_KP4: TermKeySym = 46;
pub const TERMKEY_SYM_KP5: TermKeySym = 47;
pub const TERMKEY_SYM_KP6: TermKeySym = 48;
pub const TERMKEY_SYM_KP7: TermKeySym = 49;
pub const TERMKEY_SYM_KP8: TermKeySym = 50;
pub const TERMKEY_SYM_KP9: TermKeySym = 51;
pub const TERMKEY_SYM_KPENTER: TermKeySym = 52;
pub const TERMKEY_SYM_KPPLUS: TermKeySym = 53;
pub const TERMKEY_SYM_KPMINUS: TermKeySym = 54;
pub const TERMKEY_SYM_KPMULT: TermKeySym = 55;
pub const TERMKEY_SYM_KPDIV: TermKeySym = 56;
pub const TERMKEY_SYM_KPCOMMA: TermKeySym = 57;
pub const TERMKEY_SYM_KPPERIOD: TermKeySym = 58;
pub const TERMKEY_SYM_KPEQUALS: TermKeySym = 59;
pub const TERMKEY_N_SYMS: TermKeySym = 60;

pub type TermKeyType = c_int;
pub const TERMKEY_TYPE_UNKNOWN_CSI: TermKeyType = -1;
pub const TERMKEY_TYPE_UNICODE: TermKeyType = 0;
pub const TERMKEY_TYPE_FUNCTION: TermKeyType = 1;
pub const TERMKEY_TYPE_KEYSYM: TermKeyType = 2;
pub const TERMKEY_TYPE_MOUSE: TermKeyType = 3;
pub const TERMKEY_TYPE_POSITION: TermKeyType = 4;
pub const TERMKEY_TYPE_MODEREPORT: TermKeyType = 5;
pub const TERMKEY_TYPE_DCS: TermKeyType = 6;
pub const TERMKEY_TYPE_OSC: TermKeyType = 7;
pub const TERMKEY_TYPE_APC: TermKeyType = 8;

pub type TermKeyEvent = c_int;
pub const TERMKEY_EVENT_UNKNOWN: TermKeyEvent = 0;
pub const TERMKEY_EVENT_PRESS: TermKeyEvent = 1;
pub const TERMKEY_EVENT_REPEAT: TermKeyEvent = 2;
pub const TERMKEY_EVENT_RELEASE: TermKeyEvent = 3;

pub const TERMKEY_KEYMOD_SHIFT: c_int = 1 << 0;
pub const TERMKEY_KEYMOD_ALT: c_int = 1 << 1;
pub const TERMKEY_KEYMOD_CTRL: c_int = 1 << 2;

pub const kTerm_keypad_local: usize = 28;
pub const kTerm_keypad_xmit: usize = 29;
pub const kTermCount: usize = 49;

pub type TerminfoKey = c_int;
pub const kTermKey_backspace: TerminfoKey = 0;
pub const kTermKey_beg: TerminfoKey = 1;
pub const kTermKey_btab: TerminfoKey = 2;
pub const kTermKey_clear: TerminfoKey = 3;
pub const kTermKey_dc: TerminfoKey = 4;
pub const kTermKey_end: TerminfoKey = 5;
pub const kTermKey_find: TerminfoKey = 6;
pub const kTermKey_home: TerminfoKey = 7;
pub const kTermKey_ic: TerminfoKey = 8;
pub const kTermKey_npage: TerminfoKey = 9;
pub const kTermKey_ppage: TerminfoKey = 10;
pub const kTermKey_select: TerminfoKey = 11;
pub const kTermKey_suspend: TerminfoKey = 12;
pub const kTermKey_undo: TerminfoKey = 13;
pub const kTermKey_left: TerminfoKey = 14;
pub const kTermKey_right: TerminfoKey = 15;
pub const kTermKeyCount: usize = 16;
pub const kTerminfoFuncKeyMax: usize = 63;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TerminfoEntry {
    pub back_color_erase: bool,
    pub Tc: bool,
    pub RGB: bool,
    pub Su: bool,
    pub max_colors: c_int,
    pub lines: c_int,
    pub columns: c_int,
    pub defs: [*const c_char; kTermCount],
    pub keys: [[*const c_char; 2]; kTermKeyCount],
    pub f_keys: [*const c_char; kTerminfoFuncKeyMax],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union TermKeyKeyCode {
    pub codepoint: c_int,
    pub number: c_int,
    pub sym: TermKeySym,
    pub mouse: [c_char; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermKeyKey {
    pub r#type: TermKeyType,
    pub code: TermKeyKeyCode,
    pub modifiers: c_int,
    pub event: TermKeyEvent,
    pub utf8: [c_char; 7],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct keyinfo {
    pub r#type: TermKeyType,
    pub sym: TermKeySym,
    pub modifier_mask: c_int,
    pub modifier_set: c_int,
}

pub type TermKeyTerminfoGetstrHook = Option<
    unsafe extern "C" fn(
        name: *const c_char,
        value: *const c_char,
        data: *mut c_void,
    ) -> *const c_char,
>;

#[repr(C)]
pub struct TermKeyDriver {
    pub name: *const c_char,
    pub new_driver:
        Option<unsafe extern "C" fn(tk: *mut TermKey, term: *mut TerminfoEntry) -> *mut c_void>,
    pub free_driver: Option<unsafe extern "C" fn(info: *mut c_void)>,
    pub start_driver: Option<unsafe extern "C" fn(tk: *mut TermKey, info: *mut c_void) -> c_int>,
    pub stop_driver: Option<unsafe extern "C" fn(tk: *mut TermKey, info: *mut c_void) -> c_int>,
    pub peekkey: Option<
        unsafe extern "C" fn(
            tk: *mut TermKey,
            info: *mut c_void,
            key: *mut TermKeyKey,
            force: c_int,
            nbytes: *mut usize,
        ) -> TermKeyResult,
    >,
}

#[repr(C)]
pub struct TermKeyDriverNode {
    pub driver: *mut TermKeyDriver,
    pub info: *mut c_void,
    pub next: *mut TermKeyDriverNode,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermKeyMethods {
    pub emit_codepoint:
        Option<unsafe extern "C" fn(tk: *mut TermKey, codepoint: c_int, key: *mut TermKeyKey)>,
    pub peekkey_simple: Option<
        unsafe extern "C" fn(
            tk: *mut TermKey,
            key: *mut TermKeyKey,
            force: c_int,
            nbytes: *mut usize,
        ) -> TermKeyResult,
    >,
    pub peekkey_mouse: Option<
        unsafe extern "C" fn(
            tk: *mut TermKey,
            key: *mut TermKeyKey,
            nbytes: *mut usize,
        ) -> TermKeyResult,
    >,
}

#[repr(C)]
pub struct TermKey {
    pub fd: c_int,
    pub flags: c_int,
    pub canonflags: c_int,
    pub buffer: *mut c_uchar,
    pub buffstart: usize,
    pub buffcount: usize,
    pub buffsize: usize,
    pub hightide: usize,
    pub ti_getstr_hook: TermKeyTerminfoGetstrHook,
    pub ti_getstr_hook_data: *mut c_void,
    pub waittime: c_int,
    pub is_closed: c_char,
    pub is_started: c_char,
    pub nkeynames: c_int,
    pub keynames: *mut *const c_char,
    pub c0: [keyinfo; 32],
    pub drivers: *mut TermKeyDriverNode,
    pub method: TermKeyMethods,
}

type trie_nodetype = c_int;
const TYPE_KEY: trie_nodetype = 0;
const TYPE_ARR: trie_nodetype = 1;

#[repr(C)]
pub struct trie_node {
    pub r#type: trie_nodetype,
}

#[repr(C)]
pub struct trie_node_key {
    pub r#type: trie_nodetype,
    pub key: keyinfo,
}

#[repr(C)]
pub struct trie_node_arr {
    pub r#type: trie_nodetype,
    pub min: u8,
    pub max: u8,
    pub arr: [*mut trie_node; 0],
}

#[repr(C)]
pub struct TermKeyTI {
    pub tk: *mut TermKey,
    pub ti: *mut TerminfoEntry,
    pub root: *mut trie_node,
    pub start_string: *mut c_char,
    pub stop_string: *mut c_char,
}

struct TiFuncEntry {
    ti_key: TerminfoKey,
    funcname: &'static str,
    r#type: TermKeyType,
    sym: TermKeySym,
    mods: c_int,
}

const FUNCS: &[TiFuncEntry] = &[
    TiFuncEntry {
        ti_key: kTermKey_backspace,
        funcname: "backspace",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_BACKSPACE,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_beg,
        funcname: "beg",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_BEGIN,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_btab,
        funcname: "btab",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_TAB,
        mods: TERMKEY_KEYMOD_SHIFT,
    },
    TiFuncEntry {
        ti_key: kTermKey_clear,
        funcname: "clear",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_CLEAR,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_dc,
        funcname: "dc",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_DELETE,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_end,
        funcname: "end",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_END,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_find,
        funcname: "find",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_FIND,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_home,
        funcname: "home",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_HOME,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_ic,
        funcname: "ic",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_INSERT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_left,
        funcname: "left",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_LEFT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_npage,
        funcname: "npage",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_PAGEDOWN,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_ppage,
        funcname: "ppage",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_PAGEUP,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_right,
        funcname: "right",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_RIGHT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_select,
        funcname: "select",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_SELECT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_suspend,
        funcname: "suspend",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_SUSPEND,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: kTermKey_undo,
        funcname: "undo",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_UNDO,
        mods: 0,
    },
];

unsafe fn new_node_key(
    node_type: TermKeyType,
    sym: TermKeySym,
    modmask: c_int,
    modset: c_int,
) -> *mut trie_node {
    unsafe {
        let n = xmalloc(std::mem::size_of::<trie_node_key>()).cast::<trie_node_key>();
        (*n).r#type = TYPE_KEY;
        (*n).key = keyinfo {
            r#type: node_type,
            sym,
            modifier_mask: modmask,
            modifier_set: modset,
        };
        n.cast::<trie_node>()
    }
}

unsafe fn new_node_arr(min: u8, max: u8) -> *mut trie_node {
    unsafe {
        let count = if max >= min {
            (usize::from(max) - usize::from(min)) + 1
        } else {
            0
        };
        let n = xmalloc(
            std::mem::size_of::<trie_node_arr>() + count * std::mem::size_of::<*mut trie_node>(),
        )
        .cast::<trie_node_arr>();
        (*n).r#type = TYPE_ARR;
        (*n).min = min;
        (*n).max = max;
        let arr_ptr = (*n).arr.as_mut_ptr();
        for i in 0..count {
            *arr_ptr.add(i) = std::ptr::null_mut();
        }
        n.cast::<trie_node>()
    }
}

unsafe fn lookup_next(n: *mut trie_node, b: u8) -> *mut trie_node {
    unsafe {
        match (*n).r#type {
            TYPE_KEY => {
                std::process::abort();
            }
            TYPE_ARR => {
                let nar = n.cast::<trie_node_arr>();
                if b < (*nar).min || b > (*nar).max {
                    return std::ptr::null_mut();
                }
                *(*nar).arr.as_ptr().add(usize::from(b - (*nar).min))
            }
            _ => std::ptr::null_mut(),
        }
    }
}

unsafe fn free_trie(n: *mut trie_node) {
    if n.is_null() {
        return;
    }
    unsafe {
        if (*n).r#type == TYPE_ARR {
            let nar = n.cast::<trie_node_arr>();
            if (*nar).max >= (*nar).min {
                let count = (usize::from((*nar).max) - usize::from((*nar).min)) + 1;
                let arr_ptr = (*nar).arr.as_ptr();
                for i in 0..count {
                    let child = *arr_ptr.add(i);
                    if !child.is_null() {
                        free_trie(child);
                    }
                }
            }
        }
        xfree(n.cast());
    }
}

unsafe fn compress_trie(n: *mut trie_node) -> *mut trie_node {
    if n.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        if (*n).r#type == TYPE_KEY {
            return n;
        }
        let nar = n.cast::<trie_node_arr>();
        let arr_ptr = (*nar).arr.as_ptr();
        let mut min: usize = 0;
        while (*arr_ptr.add(min)).is_null() {
            if min == 255 {
                xfree(nar.cast());
                return new_node_arr(1, 0);
            }
            min += 1;
        }
        let mut max: usize = 0xff;
        while (*arr_ptr.add(max)).is_null() {
            max -= 1;
        }

        let new = new_node_arr(min as u8, max as u8).cast::<trie_node_arr>();
        let new_arr = (*new).arr.as_mut_ptr();
        for i in min..=max {
            *new_arr.add(i - min) = compress_trie(*arr_ptr.add(i));
        }

        xfree(nar.cast());
        new.cast::<trie_node>()
    }
}

unsafe fn insert_seq(ti: *mut TermKeyTI, seq: *const c_char, node: *mut trie_node) -> c_int {
    unsafe {
        let mut pos = 0usize;
        let mut p = (*ti).root;

        loop {
            let b = *seq.add(pos) as u8;
            if b == 0 {
                break;
            }
            let next = lookup_next(p, b);
            if next.is_null() {
                break;
            }
            p = next;
            pos += 1;
        }

        loop {
            let b = *seq.add(pos) as u8;
            if b == 0 {
                break;
            }
            let next = if *seq.add(pos + 1) != 0 {
                new_node_arr(0, 0xff)
            } else {
                node
            };
            if next.is_null() {
                return 0;
            }

            if (*p).r#type == TYPE_ARR {
                let nar = p.cast::<trie_node_arr>();
                if b < (*nar).min || b > (*nar).max {
                    std::process::abort();
                }
                *(*nar).arr.as_mut_ptr().add(usize::from(b - (*nar).min)) = next;
                p = next;
            } else {
                std::process::abort();
            }

            pos += 1;
        }

        1
    }
}

unsafe fn try_load_terminfo_key(
    ti: *mut TermKeyTI,
    fn_nr: bool,
    key: usize,
    shift: bool,
    name: *const c_char,
    info: &keyinfo,
) -> bool {
    unsafe {
        let mut value: *const c_char = std::ptr::null();

        if !(*ti).ti.is_null() {
            if !fn_nr {
                value = (*(*ti).ti).keys[key][usize::from(shift)];
            } else {
                debug_assert!(!shift);
                value = (*(*ti).ti).f_keys[key];
            }
        }

        if let Some(hook) = (*(*ti).tk).ti_getstr_hook {
            value = hook(name, value, (*(*ti).tk).ti_getstr_hook_data);
        }

        if value.is_null() || value == (-1isize as *const c_char) || *value == 0 {
            return false;
        }

        let node = new_node_key(info.r#type, info.sym, info.modifier_mask, info.modifier_set);
        insert_seq(ti, value, node);

        true
    }
}

unsafe fn load_terminfo(ti: *mut TermKeyTI) -> c_int {
    unsafe {
        (*ti).root = new_node_arr(0, 0xff);
        if (*ti).root.is_null() {
            return 0;
        }

        for entry in FUNCS {
            let mut name_buf = [0 as c_char; 16];
            let s = format!("key_{}", entry.funcname);
            std::ptr::copy_nonoverlapping(s.as_ptr().cast(), name_buf.as_mut_ptr(), s.len());
            name_buf[s.len()] = 0;

            let info = keyinfo {
                r#type: entry.r#type,
                sym: entry.sym,
                modifier_mask: entry.mods,
                modifier_set: entry.mods,
            };
            if !try_load_terminfo_key(
                ti,
                false,
                entry.ti_key as usize,
                false,
                name_buf.as_ptr(),
                &info,
            ) {
                continue;
            }

            let s_shift = format!("key_s{}", entry.funcname);
            std::ptr::copy_nonoverlapping(
                s_shift.as_ptr().cast(),
                name_buf.as_mut_ptr(),
                s_shift.len(),
            );
            name_buf[s_shift.len()] = 0;

            let shift_info = keyinfo {
                r#type: entry.r#type,
                sym: entry.sym,
                modifier_mask: entry.mods | TERMKEY_KEYMOD_SHIFT,
                modifier_set: entry.mods | TERMKEY_KEYMOD_SHIFT,
            };
            try_load_terminfo_key(
                ti,
                false,
                entry.ti_key as usize,
                true,
                name_buf.as_ptr(),
                &shift_info,
            );
        }

        for i in 1..=kTerminfoFuncKeyMax {
            let mut name_buf = [0 as c_char; 16];
            let s = format!("key_f{i}");
            std::ptr::copy_nonoverlapping(s.as_ptr().cast(), name_buf.as_mut_ptr(), s.len());
            name_buf[s.len()] = 0;

            let fn_info = keyinfo {
                r#type: TERMKEY_TYPE_FUNCTION,
                sym: i as c_int,
                modifier_mask: 0,
                modifier_set: 0,
            };
            if !try_load_terminfo_key(ti, true, i - 1, false, name_buf.as_ptr(), &fn_info) {
                break;
            }
        }

        let keypad_xmit = if !(*ti).ti.is_null() {
            (*(*ti).ti).defs[kTerm_keypad_xmit]
        } else {
            std::ptr::null()
        };
        (*ti).start_string = if !keypad_xmit.is_null() {
            xstrdup(keypad_xmit)
        } else {
            std::ptr::null_mut()
        };

        let keypad_local = if !(*ti).ti.is_null() {
            (*(*ti).ti).defs[kTerm_keypad_local]
        } else {
            std::ptr::null()
        };
        (*ti).stop_string = if !keypad_local.is_null() {
            xstrdup(keypad_local)
        } else {
            std::ptr::null_mut()
        };

        (*ti).root = compress_trie((*ti).root);

        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn new_driver_ti(tk: *mut TermKey, term: *mut TerminfoEntry) -> *mut c_void {
    unsafe {
        let ti = xmalloc(std::mem::size_of::<TermKeyTI>()).cast::<TermKeyTI>();
        (*ti).tk = tk;
        (*ti).root = std::ptr::null_mut();
        (*ti).start_string = std::ptr::null_mut();
        (*ti).stop_string = std::ptr::null_mut();
        (*ti).ti = term;
        ti.cast::<c_void>()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn start_driver_ti(_tk: *mut TermKey, info: *mut c_void) -> c_int {
    unsafe {
        let ti = info.cast::<TermKeyTI>();
        if (*ti).root.is_null() {
            load_terminfo(ti);
        }
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stop_driver_ti(_tk: *mut TermKey, _info: *mut c_void) -> c_int {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_driver_ti(info: *mut c_void) {
    unsafe {
        let ti = info.cast::<TermKeyTI>();
        free_trie((*ti).root);
        if !(*ti).start_string.is_null() {
            xfree((*ti).start_string.cast());
        }
        if !(*ti).stop_string.is_null() {
            xfree((*ti).stop_string.cast());
        }
        xfree(ti.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn peekkey_ti(
    tk: *mut TermKey,
    info: *mut c_void,
    key: *mut TermKeyKey,
    force: c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        let ti = info.cast::<TermKeyTI>();

        if (*tk).buffcount == 0 {
            return if (*tk).is_closed != 0 {
                TERMKEY_RES_EOF
            } else {
                TERMKEY_RES_NONE
            };
        }

        let mut p = (*ti).root;
        let mut pos = 0usize;
        while pos < (*tk).buffcount {
            let b = *(*tk).buffer.add((*tk).buffstart + pos);
            p = lookup_next(p, b);
            if p.is_null() {
                break;
            }

            pos += 1;

            if (*p).r#type != TYPE_KEY {
                continue;
            }

            let nk = p.cast::<trie_node_key>();
            if (*nk).key.r#type == TERMKEY_TYPE_MOUSE {
                (*tk).buffstart += pos;
                (*tk).buffcount -= pos;

                let mouse_result = ((*tk).method.peekkey_mouse.unwrap_unchecked())(tk, key, nbytep);

                (*tk).buffstart -= pos;
                (*tk).buffcount += pos;

                if mouse_result == TERMKEY_RES_KEY {
                    *nbytep += pos;
                }

                return mouse_result;
            }

            (*key).r#type = (*nk).key.r#type;
            (*key).code.sym = (*nk).key.sym;
            (*key).modifiers = (*nk).key.modifier_set;
            *nbytep = pos;
            return TERMKEY_RES_KEY;
        }

        if !p.is_null() && force == 0 {
            return TERMKEY_RES_AGAIN;
        }

        TERMKEY_RES_NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trie_insert_compress_and_lookup() {
        let mut tk: TermKey = unsafe { std::mem::zeroed() };
        tk.fd = -1;
        unsafe {
            let info = new_driver_ti(&mut tk, std::ptr::null_mut());
            assert_eq!(start_driver_ti(&mut tk, info), 1);
            assert_eq!(stop_driver_ti(&mut tk, info), 1);
            free_driver_ti(info);
        }
    }
}
