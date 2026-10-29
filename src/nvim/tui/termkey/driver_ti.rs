//! Terminfo driver for libtermkey (`src/nvim/tui/termkey/driver-ti.c`).

use std::ffi::CStr;

use crate::tui::termkey::termkey::peekkey_mouse;

pub type TermKeyResult = i32;
pub const TERMKEY_RES_NONE: TermKeyResult = 0;
pub const TERMKEY_RES_KEY: TermKeyResult = 1;
pub const TERMKEY_RES_EOF: TermKeyResult = 2;
pub const TERMKEY_RES_AGAIN: TermKeyResult = 3;
pub const TERMKEY_RES_ERROR: TermKeyResult = 4;

pub type TermKeySym = i32;
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

pub type TermKeyType = i32;
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

pub type TermKeyEvent = i32;
pub const TERMKEY_EVENT_UNKNOWN: TermKeyEvent = 0;
pub const TERMKEY_EVENT_PRESS: TermKeyEvent = 1;
pub const TERMKEY_EVENT_REPEAT: TermKeyEvent = 2;
pub const TERMKEY_EVENT_RELEASE: TermKeyEvent = 3;

pub const TERMKEY_KEYMOD_SHIFT: i32 = 1 << 0;
pub const TERMKEY_KEYMOD_ALT: i32 = 1 << 1;
pub const TERMKEY_KEYMOD_CTRL: i32 = 1 << 2;

pub const K_TERM_COUNT: usize = 49;

pub type TerminfoKey = i32;
pub const K_TERM_KEY_BACKSPACE: TerminfoKey = 0;
pub const K_TERM_KEY_BEG: TerminfoKey = 1;
pub const K_TERM_KEY_BTAB: TerminfoKey = 2;
pub const K_TERM_KEY_CLEAR: TerminfoKey = 3;
pub const K_TERM_KEY_DC: TerminfoKey = 4;
pub const K_TERM_KEY_END: TerminfoKey = 5;
pub const K_TERM_KEY_FIND: TerminfoKey = 6;
pub const K_TERM_KEY_HOME: TerminfoKey = 7;
pub const K_TERM_KEY_IC: TerminfoKey = 8;
pub const K_TERM_KEY_NPAGE: TerminfoKey = 9;
pub const K_TERM_KEY_PPAGE: TerminfoKey = 10;
pub const K_TERM_KEY_SELECT: TerminfoKey = 11;
pub const K_TERM_KEY_SUSPEND: TerminfoKey = 12;
pub const K_TERM_KEY_UNDO: TerminfoKey = 13;
pub const K_TERM_KEY_LEFT: TerminfoKey = 14;
pub const K_TERM_KEY_RIGHT: TerminfoKey = 15;
pub const K_TERM_KEY_COUNT: usize = 16;
pub const K_TERMINFO_FUNC_KEY_MAX: usize = 63;

#[repr(C)]
#[derive(Clone)]
pub struct TerminfoEntry {
    pub back_color_erase: bool,
    pub tc: bool,
    pub rgb: bool,
    pub su: bool,
    pub max_colors: i32,
    pub lines: i32,
    pub columns: i32,
    pub defs: [Option<std::borrow::Cow<'static, CStr>>; K_TERM_COUNT],
    pub keys: [[Option<std::borrow::Cow<'static, CStr>>; 2]; K_TERM_KEY_COUNT],
    pub f_keys: [Option<std::borrow::Cow<'static, CStr>>; K_TERMINFO_FUNC_KEY_MAX],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermKeyKeyCode {
    pub codepoint: i32,
    pub number: i32,
    pub sym: TermKeySym,
    pub mouse: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermKeyKey {
    pub r#type: TermKeyType,
    pub code: TermKeyKeyCode,
    pub modifiers: i32,
    pub event: TermKeyEvent,
    pub utf8: [u8; 7],
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

    #[inline]
    pub fn utf8_bytes(&self) -> &[u8] {
        let end = self
            .utf8
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.utf8.len());
        &self.utf8[..end]
    }
}

impl Default for TermKeyKey {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TiKeyInfo {
    pub r#type: TermKeyType,
    pub sym: TermKeySym,
    pub modifier_mask: i32,
    pub modifier_set: i32,
}

pub struct TermKey {
    pub flags: i32,
    pub canonflags: i32,
    pub buffer: Vec<u8>,
    pub buffstart: usize,
    pub buffcount: usize,
    pub hightide: usize,
    pub is_closed: u8,
    pub is_started: u8,
    pub keynames: Vec<Option<&'static str>>,
    pub c0: [TiKeyInfo; 32],
    pub ti: TermKeyTI,
    pub csi: crate::tui::termkey::driver_csi::TermKeyCsi,
}

enum TrieNode {
    Key(TiKeyInfo),
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

#[derive(Default)]
pub struct TermKeyTI {
    root: Option<Box<TrieNode>>,
}

struct TiFuncEntry {
    ti_key: TerminfoKey,
    funcname: &'static str,
    r#type: TermKeyType,
    sym: TermKeySym,
    mods: i32,
}

const FUNCS: &[TiFuncEntry] = &[
    TiFuncEntry {
        ti_key: K_TERM_KEY_BACKSPACE,
        funcname: "backspace",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_BACKSPACE,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_BEG,
        funcname: "beg",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_BEGIN,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_BTAB,
        funcname: "btab",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_TAB,
        mods: TERMKEY_KEYMOD_SHIFT,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_CLEAR,
        funcname: "clear",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_CLEAR,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_DC,
        funcname: "dc",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_DELETE,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_END,
        funcname: "end",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_END,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_FIND,
        funcname: "find",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_FIND,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_HOME,
        funcname: "home",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_HOME,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_IC,
        funcname: "ic",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_INSERT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_LEFT,
        funcname: "left",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_LEFT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_NPAGE,
        funcname: "npage",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_PAGEDOWN,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_PPAGE,
        funcname: "ppage",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_PAGEUP,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_RIGHT,
        funcname: "right",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_RIGHT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_SELECT,
        funcname: "select",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_SELECT,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_SUSPEND,
        funcname: "suspend",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_SUSPEND,
        mods: 0,
    },
    TiFuncEntry {
        ti_key: K_TERM_KEY_UNDO,
        funcname: "undo",
        r#type: TERMKEY_TYPE_KEYSYM,
        sym: TERMKEY_SYM_UNDO,
        mods: 0,
    },
];

fn insert_seq(root: &mut TrieNode, seq: &[u8], info: TiKeyInfo) {
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

fn try_load_terminfo_key(
    ti: Option<&TerminfoEntry>,
    input: Option<&mut crate::tui::input::TermInput>,
    root: &mut TrieNode,
    fn_nr: bool,
    key: usize,
    shift: bool,
    name: &str,
    info: TiKeyInfo,
) -> bool {
    let mut value: Option<&CStr> = None;

    if let Some(ti_ref) = ti {
        if !fn_nr {
            value = ti_ref.keys[key][usize::from(shift)].as_deref();
        } else {
            debug_assert!(!shift);
            value = ti_ref.f_keys[key].as_deref();
        }
    }

    let seq = if let Some(input) = input {
        crate::tui::tui::tui_tk_ti_getstr(name, value, input)
    } else {
        value.map(CStr::to_bytes)
    };

    let Some(seq) = seq else {
        return false;
    };
    if seq.is_empty() {
        return false;
    }

    insert_seq(root, seq, info);

    true
}

fn load_terminfo(
    tk: &mut TermKey,
    ti: Option<&TerminfoEntry>,
    mut input: Option<&mut crate::tui::input::TermInput>,
) {
    let mut root = TrieNode::new_arr(0, 0xff);

    for entry in FUNCS {
        let name = format!("key_{}", entry.funcname);

        let info = TiKeyInfo {
            r#type: entry.r#type,
            sym: entry.sym,
            modifier_mask: entry.mods,
            modifier_set: entry.mods,
        };
        if !try_load_terminfo_key(
            ti,
            input.as_deref_mut(),
            &mut root,
            false,
            entry.ti_key as usize,
            false,
            &name,
            info,
        ) {
            continue;
        }

        let name_shift = format!("key_s{}", entry.funcname);

        let shift_info = TiKeyInfo {
            r#type: entry.r#type,
            sym: entry.sym,
            modifier_mask: entry.mods | TERMKEY_KEYMOD_SHIFT,
            modifier_set: entry.mods | TERMKEY_KEYMOD_SHIFT,
        };
        try_load_terminfo_key(
            ti,
            input.as_deref_mut(),
            &mut root,
            false,
            entry.ti_key as usize,
            true,
            &name_shift,
            shift_info,
        );
    }

    for i in 1..=K_TERMINFO_FUNC_KEY_MAX {
        let name = format!("key_f{i}");

        let fn_info = TiKeyInfo {
            r#type: TERMKEY_TYPE_FUNCTION,
            sym: i as i32,
            modifier_mask: 0,
            modifier_set: 0,
        };
        if !try_load_terminfo_key(
            ti,
            input.as_deref_mut(),
            &mut root,
            true,
            i - 1,
            false,
            &name,
            fn_info,
        ) {
            break;
        }
    }

    tk.ti.root = Some(Box::new(root.compress()));
}

impl TermKeyTI {
    pub const fn new() -> Self {
        Self { root: None }
    }
}

pub fn start_driver_ti(
    tk: &mut TermKey,
    ti: Option<&TerminfoEntry>,
    input: Option<&mut crate::tui::input::TermInput>,
) -> i32 {
    if tk.ti.root.is_none() {
        load_terminfo(tk, ti, input);
    }
    1
}

pub fn peekkey_ti(
    tk: &mut TermKey,
    key: &mut TermKeyKey,
    force: i32,
    nbytep: &mut usize,
) -> TermKeyResult {
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

            let mouse_result = peekkey_mouse(tk, key, &mut *nbytep);

            tk.buffstart -= pos;
            tk.buffcount += pos;

            if mouse_result == TERMKEY_RES_KEY {
                *nbytep += pos;
            }

            return mouse_result;
        }

        key.r#type = k.r#type;
        key.code.sym = k.sym;
        key.code.number = k.sym;
        key.modifiers = k.modifier_set;
        *nbytep = pos;
        return TERMKEY_RES_KEY;
    }

    if force == 0 {
        return TERMKEY_RES_AGAIN;
    }

    TERMKEY_RES_NONE
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
            TiKeyInfo {
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

        let mut tk = termkey_new_abstract(None, 0).unwrap();
        assert_eq!(start_driver_ti(&mut tk, None, None), 1);
    }
}
