//! Terminfo driver for libtermkey (`src/nvim/tui/termkey/driver-ti.c`).

use std::ffi::{CStr, c_char, c_int};

use crate::tui::termkey::termkey::peekkey_mouse;

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
#[derive(Clone)]
pub struct TerminfoEntry {
    pub back_color_erase: bool,
    pub Tc: bool,
    pub RGB: bool,
    pub Su: bool,
    pub max_colors: c_int,
    pub lines: c_int,
    pub columns: c_int,
    pub defs: [Option<std::borrow::Cow<'static, CStr>>; kTermCount],
    pub keys: [[Option<std::borrow::Cow<'static, CStr>>; 2]; kTermKeyCount],
    pub f_keys: [Option<std::borrow::Cow<'static, CStr>>; kTerminfoFuncKeyMax],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermKeyKeyCode {
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

impl TermKeyKey {
    pub const fn new() -> Self {
        Self {
            r#type: 0,
            code: TermKeyKeyCode {
                codepoint: 0,
                number: 0,
                sym: 0,
                mouse: [0; 4],
            },
            modifiers: 0,
            event: 0,
            utf8: [0; 7],
        }
    }
}

impl Default for TermKeyKey {
    fn default() -> Self {
        Self::new()
    }
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
    unsafe fn(
        name: *const c_char,
        value: *const c_char,
        data: *mut crate::tui::input::TermInput,
    ) -> *const c_char,
>;

pub struct TermKey {
    pub flags: c_int,
    pub canonflags: c_int,
    pub buffer: Vec<u8>,
    pub buffstart: usize,
    pub buffcount: usize,
    pub hightide: usize,
    pub ti_getstr_hook: TermKeyTerminfoGetstrHook,
    pub ti_getstr_hook_data: *mut crate::tui::input::TermInput,
    pub is_closed: c_char,
    pub is_started: c_char,
    pub keynames: Vec<Option<&'static CStr>>,
    pub c0: [keyinfo; 32],
    pub ti: TermKeyTI,
    pub csi: crate::tui::termkey::driver_csi::TermKeyCsi,
}

enum TrieNode {
    Key(keyinfo),
    Arr {
        min: u8,
        max: u8,
        children: Vec<Option<Box<TrieNode>>>,
    },
}

impl TrieNode {
    fn new_arr(min: u8, max: u8) -> Self {
        let count = if max >= min {
            (usize::from(max) - usize::from(min)) + 1
        } else {
            0
        };
        let mut children = Vec::with_capacity(count);
        children.resize_with(count, || None);
        Self::Arr { min, max, children }
    }

    fn lookup_next(&self, b: u8) -> Option<&TrieNode> {
        match self {
            Self::Key(_) => std::process::abort(),
            Self::Arr { min, max, children } => {
                if b < *min || b > *max {
                    None
                } else {
                    children[usize::from(b - *min)].as_deref()
                }
            }
        }
    }

    fn compress(self) -> Self {
        match self {
            Self::Key(k) => Self::Key(k),
            Self::Arr { mut children, .. } => {
                let Some(min) = children.iter().position(Option::is_some) else {
                    return Self::new_arr(1, 0);
                };
                let max = children.iter().rposition(Option::is_some).unwrap();
                let compressed = children
                    .drain(min..=max)
                    .map(|child| child.map(|n| Box::new((*n).compress())))
                    .collect();
                Self::Arr {
                    min: min as u8,
                    max: max as u8,
                    children: compressed,
                }
            }
        }
    }
}

pub struct TermKeyTI {
    pub ti: *mut TerminfoEntry,
    root: Option<Box<TrieNode>>,
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

fn insert_seq(root: &mut TrieNode, seq: &[u8], info: keyinfo) {
    let mut cur = root;
    for (idx, &b) in seq.iter().enumerate() {
        let is_last = idx + 1 == seq.len();
        let TrieNode::Arr { min, max, children } = cur else {
            std::process::abort();
        };
        if b < *min || b > *max {
            std::process::abort();
        }
        let slot = &mut children[usize::from(b - *min)];
        if is_last {
            if slot.is_none() {
                *slot = Some(Box::new(TrieNode::Key(info)));
            }
            return;
        }
        if slot.is_none() {
            *slot = Some(Box::new(TrieNode::new_arr(0, 0xff)));
        }
        cur = slot.as_deref_mut().unwrap();
    }
}

unsafe fn try_load_terminfo_key(
    tk: &TermKey,
    root: &mut TrieNode,
    fn_nr: bool,
    key: usize,
    shift: bool,
    name: *const c_char,
    info: keyinfo,
) -> bool {
    unsafe {
        let mut value: *const c_char = std::ptr::null();
        let ti_ptr = tk.ti.ti;

        if !ti_ptr.is_null() {
            if !fn_nr {
                value = (*ti_ptr).keys[key][usize::from(shift)]
                    .as_deref()
                    .map_or(std::ptr::null(), CStr::as_ptr);
            } else {
                debug_assert!(!shift);
                value = (*ti_ptr).f_keys[key]
                    .as_deref()
                    .map_or(std::ptr::null(), CStr::as_ptr);
            }
        }

        if let Some(hook) = tk.ti_getstr_hook {
            value = hook(name, value, tk.ti_getstr_hook_data);
        }

        if value.is_null() || value == (-1isize as *const c_char) || *value == 0 {
            return false;
        }

        let seq = CStr::from_ptr(value).to_bytes();
        insert_seq(root, seq, info);

        true
    }
}

unsafe fn load_terminfo(tk: &mut TermKey) {
    unsafe {
        let mut root = TrieNode::new_arr(0, 0xff);

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
                tk,
                &mut root,
                false,
                entry.ti_key as usize,
                false,
                name_buf.as_ptr(),
                info,
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
                tk,
                &mut root,
                false,
                entry.ti_key as usize,
                true,
                name_buf.as_ptr(),
                shift_info,
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
            if !try_load_terminfo_key(
                tk,
                &mut root,
                true,
                i - 1,
                false,
                name_buf.as_ptr(),
                fn_info,
            ) {
                break;
            }
        }

        tk.ti.root = Some(Box::new(root.compress()));
    }
}

impl TermKeyTI {
    pub fn new(term: *mut TerminfoEntry) -> Self {
        Self {
            ti: term,
            root: None,
        }
    }
}

pub unsafe fn start_driver_ti(tk: &mut TermKey) -> c_int {
    unsafe {
        if tk.ti.root.is_none() {
            load_terminfo(tk);
        }
        1
    }
}

pub unsafe fn peekkey_ti(
    tk: &mut TermKey,
    key: *mut TermKeyKey,
    force: c_int,
    nbytep: *mut usize,
) -> TermKeyResult {
    unsafe {
        if tk.buffcount == 0 {
            return if tk.is_closed != 0 {
                TERMKEY_RES_EOF
            } else {
                TERMKEY_RES_NONE
            };
        }

        let Some(mut p) = tk.ti.root.as_deref() else {
            return TERMKEY_RES_NONE;
        };

        let mut pos = 0usize;
        while pos < tk.buffcount {
            let b = tk.buffer[tk.buffstart + pos];
            let Some(next) = p.lookup_next(b) else {
                return TERMKEY_RES_NONE;
            };
            p = next;
            pos += 1;

            let TrieNode::Key(k) = *p else {
                continue;
            };

            if k.r#type == TERMKEY_TYPE_MOUSE {
                tk.buffstart += pos;
                tk.buffcount -= pos;

                let mouse_result = peekkey_mouse(tk, key, nbytep);

                tk.buffstart -= pos;
                tk.buffcount += pos;

                if mouse_result == TERMKEY_RES_KEY {
                    *nbytep += pos;
                }

                return mouse_result;
            }

            (*key).r#type = k.r#type;
            (*key).code.sym = k.sym;
            (*key).code.number = k.sym;
            (*key).modifiers = k.modifier_set;
            *nbytep = pos;
            return TERMKEY_RES_KEY;
        }

        if force == 0 {
            return TERMKEY_RES_AGAIN;
        }

        TERMKEY_RES_NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::termkey::termkey::termkey_new_abstract;

    #[test]
    fn trie_insert_compress_and_lookup() {
        let mut root = TrieNode::new_arr(0, 0xff);
        insert_seq(
            &mut root,
            b"\x1bOP",
            keyinfo {
                r#type: TERMKEY_TYPE_FUNCTION,
                sym: 1,
                modifier_mask: 0,
                modifier_set: 0,
            },
        );
        let root = root.compress();
        let n1 = root.lookup_next(0x1b).unwrap();
        let n2 = n1.lookup_next(b'O').unwrap();
        let n3 = n2.lookup_next(b'P').unwrap();
        assert!(matches!(n3, TrieNode::Key(k) if k.sym == 1));
        assert!(n2.lookup_next(b'Q').is_none());

        unsafe {
            let mut tk = termkey_new_abstract(std::ptr::null_mut(), 0).unwrap();
            assert_eq!(start_driver_ti(&mut tk), 1);
        }
    }
}
