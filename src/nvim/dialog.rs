//! User prompts and console dialogs (`yes`/`no`, number, `confirm()`) (`src/nvim/dialog.c`).

use crate::cmdexpand::EXPAND_NOTHING;
use crate::eval::typval::Callback;
use crate::ex_cmds::HLF_R;
use crate::ex_getln::getcmdline_prompt;
use crate::globals::*;
use crate::input::ins_typebuf;
use crate::keycodes::{CAR, ESC};
use crate::main_init::input_unanswerable;
use crate::mapping::REMAP_YES;
use crate::mbyte::{mb_tolower, utf_char2bytes, utf_ptr2char, utf_ptr2len, utfc_ptr2len};

use crate::message::{HLF_M, display_confirm_msg, msg_end_prompt, msg_putchar, set_keep_msg};
use crate::mouse::setmouse;
use crate::os::input::input_available;
use crate::ui::{ui_active, ui_call_busy_stop, ui_flush, ui_has, ui_mouse_has};
use std::ffi::{c_char, c_int};

const DLG_BUTTON_SEP: u8 = b'\n';
const DLG_HOTKEY_CHAR: u8 = b'&';

const CTRL_C: c_int = 0x03;

const K_UI_MESSAGES: c_int = 4;
const K_MOUSE_COMMAND: c_int = b'c' as c_int;

pub(crate) const VIM_YES: c_int = 2;
pub(crate) const VIM_NO: c_int = 3;
const VIM_CANCEL: c_int = 4;
pub(crate) const VIM_ALL: c_int = 5;
pub(crate) const VIM_DISCARDALL: c_int = 6;

const HAS_HOTKEY_LEN: usize = 30;
const HOTK_LEN: usize = 4;

pub(crate) struct DialogModuleState {
    pub(crate) confirm_msg: Option<std::ffi::CString>,
    pub(crate) confirm_buttons: Vec<c_char>,
}

impl DialogModuleState {
    const fn new() -> Self {
        Self {
            confirm_msg: None,
            confirm_buttons: Vec::new(),
        }
    }
}

static DLG_STATE: GlobalState<DialogModuleState> = GlobalState::new(DialogModuleState::new());

#[inline]
pub(crate) fn dlg_st() -> &'static mut DialogModuleState {
    unsafe { &mut *DLG_STATE.as_ptr() }
}

fn copy_char(from: *const c_char, to: *mut c_char, lowercase: bool) -> usize {
    unsafe {
        if lowercase {
            let c = mb_tolower(utf_ptr2char(from));
            utf_char2bytes(c, to) as usize
        } else {
            let len = utfc_ptr2len(from) as usize;
            std::ptr::copy(from.cast::<u8>(), to.cast::<u8>(), len);
            len
        }
    }
}

fn console_dialog_alloc(
    message: *const c_char,
    buttons: *const c_char,
    has_hotkey: &mut [bool; HAS_HOTKEY_LEN],
) -> Vec<c_char> {
    unsafe {
        let mut lenhotkey = HOTK_LEN;
        has_hotkey[0] = false;

        let mut button_len = 0usize;
        let mut idx = 0usize;
        let mut r = buttons;
        while *r != 0 {
            let b = crate::memory::cstr_first_byte(r);
            if b == DLG_BUTTON_SEP {
                button_len += 3;
                lenhotkey += HOTK_LEN;
                if idx < HAS_HOTKEY_LEN - 1 {
                    idx += 1;
                    has_hotkey[idx] = false;
                }
            } else if b == DLG_HOTKEY_CHAR {
                r = r.add(1);
                button_len += 1;
                if idx < HAS_HOTKEY_LEN - 1 {
                    has_hotkey[idx] = true;
                }
            }
            if *r == 0 {
                break;
            }
            r = r.add(utf_ptr2len(r) as usize);
        }

        let msg_bytes = crate::memory::cstr_as_bytes(message);
        button_len += crate::memory::cstr_len(buttons) + 3;
        lenhotkey += 1;

        if !has_hotkey[0] {
            button_len += 2;
        }

        dlg_st().confirm_msg = Some(if ui_has(K_UI_MESSAGES) {
            crate::memory::cstr_to_cstring(message)
        } else {
            let mut v = Vec::with_capacity(msg_bytes.len() + 2);
            v.push(b'\n');
            v.extend_from_slice(msg_bytes);
            v.push(b'\n');
            std::ffi::CString::new(v).unwrap_or_default()
        });

        dlg_st().confirm_buttons = vec![0 as c_char; button_len];

        vec![0 as c_char; lenhotkey]
    }
}

fn copy_confirm_hotkeys(
    buttons: *const c_char,
    mut default_button_idx: c_int,
    has_hotkey: &[bool; HAS_HOTKEY_LEN],
    mut hotkeys_ptr: *mut c_char,
) {
    unsafe {
        let first_len = copy_char(buttons, hotkeys_ptr, true);
        *hotkeys_ptr.add(first_len) = 0;

        let mut first_hotkey = !has_hotkey[0];
        let mut msgp = dlg_st().confirm_buttons.as_mut_ptr();
        let mut idx = 0usize;
        let mut r = buttons;

        while *r != 0 {
            let b = crate::memory::cstr_first_byte(r);
            if b == DLG_BUTTON_SEP {
                *msgp = b',' as c_char;
                *msgp.add(1) = b' ' as c_char;
                msgp = msgp.add(2);

                hotkeys_ptr = hotkeys_ptr.add(crate::memory::cstr_len(hotkeys_ptr));
                let nlen = copy_char(r.add(1), hotkeys_ptr, true);
                *hotkeys_ptr.add(nlen) = 0;

                if default_button_idx != 0 {
                    default_button_idx -= 1;
                }

                if idx < HAS_HOTKEY_LEN - 1 {
                    idx += 1;
                    if !has_hotkey[idx] {
                        first_hotkey = true;
                    }
                }
            } else if b == DLG_HOTKEY_CHAR || first_hotkey {
                if b == DLG_HOTKEY_CHAR {
                    r = r.add(1);
                }
                first_hotkey = false;
                if crate::memory::cstr_first_byte(r) == DLG_HOTKEY_CHAR {
                    *msgp = *r;
                    msgp = msgp.add(1);
                } else {
                    *msgp = if default_button_idx == 1 {
                        b'[' as c_char
                    } else {
                        b'(' as c_char
                    };
                    msgp = msgp.add(1);
                    let clen = copy_char(r, msgp, false);
                    msgp = msgp.add(clen);
                    *msgp = if default_button_idx == 1 {
                        b']' as c_char
                    } else {
                        b')' as c_char
                    };
                    msgp = msgp.add(1);

                    let hlen = copy_char(r, hotkeys_ptr, true);
                    *hotkeys_ptr.add(hlen) = 0;
                }
            } else {
                let clen = copy_char(r, msgp, false);
                msgp = msgp.add(clen);
            }

            if *r == 0 {
                break;
            }
            r = r.add(utf_ptr2len(r) as usize);
        }

        *msgp = b':' as c_char;
        *msgp.add(1) = b' ' as c_char;
        *msgp.add(2) = 0;
    }
}

fn msg_show_console_dialog(
    message: impl crate::strings::IntoCCharPtr,
    buttons: impl crate::strings::IntoCCharPtr,
    dfltbutton: c_int,
) -> Vec<c_char> { let message = message.into_c_char_ptr(); let buttons = buttons.into_c_char_ptr();
    let mut has_hotkey = [false; HAS_HOTKEY_LEN];
        let mut hotk = console_dialog_alloc(message, buttons, &mut has_hotkey);
        copy_confirm_hotkeys(buttons, dfltbutton, &has_hotkey, hotk.as_mut_ptr());
        display_confirm_msg();
        hotk
}

/// Ask for a `'y'` or `'n'` reply from the user (`ask_yesno`).
pub fn ask_yesno(str_ptr: impl crate::strings::IntoCCharPtr) -> c_int { let str_ptr = str_ptr.into_c_char_ptr();
     {
        let save_state = g_st().State;
        g_st().no_wait_return += 1;

        let s_bytes = crate::memory::cstr_as_bytes(str_ptr);
        let suffix = b" (y/n)?\0";
        let mut prompt = Vec::with_capacity(s_bytes.len() + suffix.len());
        prompt.extend_from_slice(s_bytes);
        prompt.extend_from_slice(suffix);

        let mut r: c_int = b' ' as c_int;
        while r != (b'y' as c_int) && r != (b'n' as c_int) {
            r = prompt_for_input(
                prompt.as_mut_ptr().cast::<c_char>(),
                HLF_R,
                true,
                None,
            );
            if r == CTRL_C || r == ESC {
                r = b'n' as c_int;
                if !ui_has(K_UI_MESSAGES) {
                    msg_putchar(r);
                }
            }
        }

        g_st().need_wait_return = g_st().msg_scrolled != 0;
        g_st().no_wait_return -= 1;
        g_st().State = save_state;
        setmouse();
        r
    }
}

/// Ask the user for input through a cmdline prompt (`prompt_for_input`).
pub fn prompt_for_input(
    prompt: impl crate::strings::IntoCCharPtr,
    hl_id: c_int,
    one_key: bool,mouse_used: Option<&mut bool>,
) -> c_int {
    let prompt = prompt.into_c_char_ptr().cast_mut();
    unsafe {
        let mut ret = if one_key { ESC } else { 0 };
        let kmsg = g_st().keep_msg.clone();

        let prompt_ptr: *const c_char = if prompt.is_null() {
            if mouse_used.is_some() && ui_mouse_has(K_MOUSE_COMMAND) {
                c"Type number and <Enter> or click with the mouse (q or empty cancels): ".as_ptr()
            } else {
                c"Type number and <Enter> (q or empty cancels): ".as_ptr()
            }
        } else {
            prompt
        };

        g_st().cmdline_row = g_st().msg_row;
        ui_call_busy_stop();
        ui_flush();

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        let resp = getcmdline_prompt(
            -1,
            prompt_ptr,
            hl_id,
            EXPAND_NOTHING,
            std::ptr::null(),
            Callback::NONE,
            one_key,
            mouse_used.map_or(std::ptr::null_mut(), |p| &raw mut *p),
        );
        g_st().allow_keys -= 1;
        g_st().no_mapping -= 1;

        if let Some(ref resp_cstr) = resp {
            if one_key {
                ret = *resp_cstr.as_ptr() as c_int;
            } else {
                let s = resp_cstr.to_string_lossy();
                let trimmed = s.trim_start();
                let mut sign: c_int = 1;
                let digits = if let Some(rest) = trimmed.strip_prefix('-') {
                    sign = -1;
                    rest
                } else if let Some(rest) = trimmed.strip_prefix('+') {
                    rest
                } else {
                    trimmed
                };
                let mut val: c_int = 0;
                for b in digits.bytes() {
                    if b.is_ascii_digit() {
                        val = val.wrapping_mul(10).wrapping_add((b - b'0') as c_int);
                    } else {
                        break;
                    }
                }
                ret = val.wrapping_mul(sign);
            }
        }

        if let Some(ref kmsg_cstr) = kmsg {
            set_keep_msg(kmsg_cstr.as_ptr(), g_st().keep_msg_hl_id);
        }

        ret
    }
}

/// Generic console dialog handler (`do_dialog`).
pub fn do_dialog(
    _dlg_type: c_int,
    _title: *const c_char,
    message: impl crate::strings::IntoCCharPtr,
    buttons: impl crate::strings::IntoCCharPtr,
    dfltbutton: c_int,
    _textfield: *const c_char,
    ex_cmd: c_int,
) -> c_int { let message = message.into_c_char_ptr();
    let buttons = buttons.into_c_char_ptr();
    unsafe {
        if g_st().silent_mode {
            return dfltbutton;
        }

        let save_msg_silent = g_st().msg_silent;
        let old_state = g_st().State;
        g_st().msg_silent = 0;
        g_st().no_wait_return += 1;

        let hotkeys_vec = msg_show_console_dialog(message, buttons, dfltbutton);
        let hotkeys = hotkeys_vec.as_ptr();
        let retval: c_int;

        loop {
            if (ui_active() == 0 || input_unanswerable()) && input_available() == 0 {
                retval = dfltbutton;
                break;
            }

            let mut c = prompt_for_input(
                dlg_st().confirm_buttons.as_mut_ptr(),
                HLF_M,
                true,
                None,
            );
            match c {
                CAR | 0 => {
                    retval = dfltbutton;
                    break;
                }
                CTRL_C | ESC => {
                    retval = 0;
                    break;
                }
                _ => {
                    if c < 0 {
                        g_st().msg_didout = false;
                        g_st().msg_didany = false;
                        continue;
                    }
                    if c == (b':' as c_int) && ex_cmd != 0 {
                        retval = dfltbutton;
                        ins_typebuf(c":", REMAP_YES, 0, false, false);
                        break;
                    }

                    c = mb_tolower(c);
                    let mut candidate = 1 as c_int;
                    let mut i = 0usize;
                    while *hotkeys.add(i) != 0 {
                        if utf_ptr2char(hotkeys.add(i)) == c {
                            break;
                        }
                        i += utfc_ptr2len(hotkeys.add(i)) as usize;
                        candidate += 1;
                    }
                    if *hotkeys.add(i) != 0 {
                        retval = candidate;
                        break;
                    }
                    g_st().msg_didout = false;
                    g_st().msg_didany = false;
                }
            }
        }

        drop(hotkeys_vec);
        dlg_st().confirm_msg = None;

        g_st().msg_silent = save_msg_silent;
        g_st().State = old_state;
        setmouse();
        g_st().no_wait_return -= 1;
        msg_end_prompt();

        retval
    }
}

/// Yes/No dialog (`vim_dialog_yesno`).
pub fn vim_dialog_yesno(
    dlg_type: c_int,
    title: *mut c_char,
    message: impl crate::strings::IntoCCharPtr,
    dflt: c_int,
) -> c_int {
    let message = message.into_c_char_ptr().cast_mut();
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title.cast_const()
    };
    if do_dialog(
            dlg_type,
            t,
            message,
            c"&Yes\n&No",
            dflt,
            std::ptr::null(),
            0,
        ) == 1
    {
        VIM_YES
    } else {
        VIM_NO
    }
}

/// Yes/No/Cancel dialog (`vim_dialog_yesnocancel`).
pub fn vim_dialog_yesnocancel(
    dlg_type: c_int,
    title: *mut c_char,
    message: impl crate::strings::IntoCCharPtr,
    dflt: c_int,
) -> c_int { let message = message.into_c_char_ptr();
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title.cast_const()
    };
    match do_dialog(
            dlg_type,
            t,
            message,
            c"&Yes\n&No\n&Cancel",
            dflt,
            std::ptr::null(),
            0,
        ) {
        1 => VIM_YES,
        2 => VIM_NO,
        _ => VIM_CANCEL,
    }
}

/// Yes/No/Save All/Discard All/Cancel dialog (`vim_dialog_yesnoallcancel`).
pub fn vim_dialog_yesnoallcancel(
    dlg_type: c_int,
    title: *mut c_char,
    message: impl crate::strings::IntoCCharPtr,
    dflt: c_int,
) -> c_int { let message = message.into_c_char_ptr();
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title.cast_const()
    };
    match do_dialog(
            dlg_type,
            t,
            message,
            c"&Yes\n&No\nSave &All\n&Discard All\n&Cancel",
            dflt,
            std::ptr::null(),
            0,
        ) {
        1 => VIM_YES,
        2 => VIM_NO,
        3 => VIM_ALL,
        4 => VIM_DISCARDALL,
        _ => VIM_CANCEL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_dialog_yesno_uses_default_in_test_mode() {
        g_st().silent_mode = true;
            assert_eq!(
                vim_dialog_yesno(0, std::ptr::null_mut(), c"Save?", 1),
                VIM_YES
            );
            assert_eq!(
                vim_dialog_yesno(0, std::ptr::null_mut(), c"Save?", 2),
                VIM_NO
            );
    }
}
