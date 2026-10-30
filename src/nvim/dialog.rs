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
use crate::mbyte::{mb_tolower, utf_char2bytes};

use crate::message::{HLF_M, display_confirm_msg, msg_end_prompt, msg_putchar, set_keep_msg};
use crate::mouse::setmouse;
use crate::os::input::input_available;
use crate::ui::{ui_active, ui_call_busy_stop, ui_flush, ui_has, ui_mouse_has};
use std::ffi::CStr;

const DLG_BUTTON_SEP: u8 = b'\n';
const DLG_HOTKEY_CHAR: u8 = b'&';

const CTRL_C: i32 = 0x03;

const K_UI_MESSAGES: i32 = 4;
const K_MOUSE_COMMAND: i32 = b'c' as i32;

pub(crate) const VIM_YES: i32 = 2;
pub(crate) const VIM_NO: i32 = 3;
const VIM_CANCEL: i32 = 4;
pub(crate) const VIM_ALL: i32 = 5;
pub(crate) const VIM_DISCARDALL: i32 = 6;

const HAS_HOTKEY_LEN: usize = 30;
const HOTK_LEN: usize = 4;

pub(crate) struct DialogModuleState {
    pub(crate) confirm_msg: Option<std::ffi::CString>,
    pub(crate) confirm_buttons: Vec<u8>,
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

fn copy_char(from: &[u8], to: &mut [u8], lowercase: bool) -> usize {
    if from.is_empty() {
        return 0;
    }
    if lowercase {
        let c = mb_tolower(crate::mbyte::utf_slice2char(from));
        utf_char2bytes(c, to.as_mut_ptr().cast()) as usize
    } else {
        let len = crate::mbyte::utfc_slice2len(from);
        to[..len].copy_from_slice(&from[..len]);
        len
    }
}

fn console_dialog_alloc(
    message: &CStr,
    buttons: &CStr,
    has_hotkey: &mut [bool; HAS_HOTKEY_LEN],
) -> Vec<u8> {
    let mut lenhotkey = HOTK_LEN;
    has_hotkey[0] = false;

    let mut button_len = 0usize;
    let mut idx = 0usize;
    let btn_bytes = buttons.to_bytes();
    let mut pos = 0usize;
    while pos < btn_bytes.len() {
        let b = btn_bytes[pos];
        if b == DLG_BUTTON_SEP {
            button_len += 3;
            lenhotkey += HOTK_LEN;
            if idx < HAS_HOTKEY_LEN - 1 {
                idx += 1;
                has_hotkey[idx] = false;
            }
        } else if b == DLG_HOTKEY_CHAR {
            pos += 1;
            button_len += 1;
            if idx < HAS_HOTKEY_LEN - 1 {
                has_hotkey[idx] = true;
            }
        }
        if pos >= btn_bytes.len() {
            break;
        }
        pos += crate::mbyte::utf_slice2len(&btn_bytes[pos..]).max(1);
    }

    let msg_bytes = message.to_bytes();
    button_len += btn_bytes.len() + 3;
    lenhotkey += 1;

    if !has_hotkey[0] {
        button_len += 2;
    }

    dlg_st().confirm_msg = Some(if ui_has(K_UI_MESSAGES) {
        message.to_owned()
    } else {
        let mut v = Vec::with_capacity(msg_bytes.len() + 2);
        v.push(b'\n');
        v.extend_from_slice(msg_bytes);
        v.push(b'\n');
        std::ffi::CString::new(v).unwrap_or_default()
    });

    dlg_st().confirm_buttons = vec![0u8; button_len];

    vec![0u8; lenhotkey]
}

fn copy_confirm_hotkeys(
    buttons: &CStr,
    mut default_button_idx: i32,
    has_hotkey: &[bool; HAS_HOTKEY_LEN],
    hotkeys: &mut [u8],
) {
    let btn_bytes = buttons.to_bytes();
    let mut hk_pos = 0usize;
    let first_len = copy_char(btn_bytes, &mut hotkeys[hk_pos..], true);
    hotkeys[hk_pos + first_len] = 0;

    let mut first_hotkey = !has_hotkey[0];
    let mut msg_pos = 0usize;
    let mut idx = 0usize;
    let mut pos = 0usize;

    while pos < btn_bytes.len() {
        let b = btn_bytes[pos];
        if b == DLG_BUTTON_SEP {
            dlg_st().confirm_buttons[msg_pos] = b',';
            dlg_st().confirm_buttons[msg_pos + 1] = b' ';
            msg_pos += 2;

            hk_pos += hotkeys[hk_pos..].iter().position(|&x| x == 0).unwrap_or(0);
            let nlen = copy_char(
                btn_bytes.get(pos + 1..).unwrap_or(&[]),
                &mut hotkeys[hk_pos..],
                true,
            );
            hotkeys[hk_pos + nlen] = 0;

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
                pos += 1;
            }
            first_hotkey = false;
            if btn_bytes.get(pos) == Some(&DLG_HOTKEY_CHAR) {
                dlg_st().confirm_buttons[msg_pos] = DLG_HOTKEY_CHAR;
                msg_pos += 1;
            } else if pos < btn_bytes.len() {
                dlg_st().confirm_buttons[msg_pos] =
                    if default_button_idx == 1 { b'[' } else { b'(' };
                msg_pos += 1;
                let clen = copy_char(
                    &btn_bytes[pos..],
                    &mut dlg_st().confirm_buttons[msg_pos..],
                    false,
                );
                msg_pos += clen;
                dlg_st().confirm_buttons[msg_pos] =
                    if default_button_idx == 1 { b']' } else { b')' };
                msg_pos += 1;

                let hlen = copy_char(&btn_bytes[pos..], &mut hotkeys[hk_pos..], true);
                hotkeys[hk_pos + hlen] = 0;
            }
        } else {
            let clen = copy_char(
                &btn_bytes[pos..],
                &mut dlg_st().confirm_buttons[msg_pos..],
                false,
            );
            msg_pos += clen;
        }

        if pos >= btn_bytes.len() {
            break;
        }
        pos += crate::mbyte::utf_slice2len(&btn_bytes[pos..]).max(1);
    }

    dlg_st().confirm_buttons[msg_pos] = b':';
    dlg_st().confirm_buttons[msg_pos + 1] = b' ';
    dlg_st().confirm_buttons[msg_pos + 2] = 0;
}

fn msg_show_console_dialog(message: &CStr, buttons: &CStr, dfltbutton: i32) -> Vec<u8> {
    let mut has_hotkey = [false; HAS_HOTKEY_LEN];
    let mut hotk = console_dialog_alloc(message, buttons, &mut has_hotkey);
    copy_confirm_hotkeys(buttons, dfltbutton, &has_hotkey, &mut hotk);
    display_confirm_msg();
    hotk
}

/// Ask for a `'y'` or `'n'` reply from the user (`ask_yesno`).
pub fn ask_yesno(str_ptr: impl crate::strings::IntoCCharPtr) -> i32 {
    let str_ptr = str_ptr.into_c_char_ptr();
    {
        let save_state = g_st().state_global;
        g_st().no_wait_return += 1;

        let s_bytes = crate::memory::cstr_as_bytes(str_ptr);
        let suffix = b" (y/n)?\0";
        let mut prompt = Vec::with_capacity(s_bytes.len() + suffix.len());
        prompt.extend_from_slice(s_bytes);
        prompt.extend_from_slice(suffix);

        let mut r: i32 = b' ' as i32;
        while r != (b'y' as i32) && r != (b'n' as i32) {
            r = prompt_for_input(prompt.as_ptr().cast(), HLF_R, true, None);
            if r == CTRL_C || r == ESC {
                r = b'n' as i32;
                if !ui_has(K_UI_MESSAGES) {
                    msg_putchar(r);
                }
            }
        }

        g_st().need_wait_return = g_st().msg_scrolled != 0;
        g_st().no_wait_return -= 1;
        g_st().state_global = save_state;
        setmouse();
        r
    }
}

/// Ask the user for input through a cmdline prompt (`prompt_for_input`).
pub fn prompt_for_input(
    prompt: impl crate::strings::IntoCCharPtr,
    hl_id: i32,
    one_key: bool,
    mouse_used: Option<&mut bool>,
) -> i32 {
    let prompt = prompt.into_c_char_ptr().cast_mut();
    {
        let mut ret = if one_key { ESC } else { 0 };
        let kmsg = g_st().keep_msg.clone();

        let prompt_ptr = if prompt.is_null() {
            if mouse_used.is_some() && ui_mouse_has(K_MOUSE_COMMAND) {
                c"Type number and <Enter> or click with the mouse (q or empty cancels): ".as_ptr()
            } else {
                c"Type number and <Enter> (q or empty cancels): ".as_ptr()
            }
        } else {
            prompt.cast_const()
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
                ret = crate::memory::cstr_first_char(resp_cstr.as_ptr());
            } else {
                let s = resp_cstr.to_string_lossy();
                let trimmed = s.trim_start();
                let mut sign: i32 = 1;
                let digits = if let Some(rest) = trimmed.strip_prefix('-') {
                    sign = -1;
                    rest
                } else if let Some(rest) = trimmed.strip_prefix('+') {
                    rest
                } else {
                    trimmed
                };
                let mut val: i32 = 0;
                for b in digits.bytes() {
                    if b.is_ascii_digit() {
                        val = val.wrapping_mul(10).wrapping_add((b - b'0') as i32);
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
    _dlg_type: i32,
    _title: impl crate::strings::IntoCCharPtr,
    message: impl crate::strings::IntoCCharPtr,
    buttons: impl crate::strings::IntoCCharPtr,
    dfltbutton: i32,
    _textfield: impl crate::strings::IntoCCharPtr,
    ex_cmd: i32,
) -> i32 {
    let message = crate::memory::cstr_from_ptr(message);
    let buttons = crate::memory::cstr_from_ptr(buttons);
    {
        if g_st().silent_mode {
            return dfltbutton;
        }

        let save_msg_silent = g_st().msg_silent;
        let old_state = g_st().state_global;
        g_st().msg_silent = 0;
        g_st().no_wait_return += 1;

        let hotkeys_vec = msg_show_console_dialog(message, buttons, dfltbutton);
        let retval: i32;

        loop {
            if (ui_active() == 0 || input_unanswerable()) && input_available() == 0 {
                retval = dfltbutton;
                break;
            }

            let btn_cstr = CStr::from_bytes_until_nul(&dlg_st().confirm_buttons).unwrap_or(c"");
            let mut c = prompt_for_input(btn_cstr, HLF_M, true, None);
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
                    if c == (b':' as i32) && ex_cmd != 0 {
                        retval = dfltbutton;
                        ins_typebuf(c":", REMAP_YES, 0, false, false);
                        break;
                    }

                    c = mb_tolower(c);
                    let mut candidate = 1_i32;
                    let mut i = 0usize;
                    while i < hotkeys_vec.len() && hotkeys_vec[i] != 0 {
                        if crate::mbyte::utf_slice2char(&hotkeys_vec[i..]) == c {
                            break;
                        }
                        i += crate::mbyte::utfc_slice2len(&hotkeys_vec[i..]).max(1);
                        candidate += 1;
                    }
                    if i < hotkeys_vec.len() && hotkeys_vec[i] != 0 {
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
        g_st().state_global = old_state;
        setmouse();
        g_st().no_wait_return -= 1;
        msg_end_prompt();

        retval
    }
}

/// Yes/No dialog (`vim_dialog_yesno`).
pub fn vim_dialog_yesno(
    dlg_type: i32,
    title: impl crate::strings::IntoCCharPtr,
    message: impl crate::strings::IntoCCharPtr,
    dflt: i32,
) -> i32 {
    let title = title.into_c_char_ptr();
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title
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
    dlg_type: i32,
    title: impl crate::strings::IntoCCharPtr,
    message: impl crate::strings::IntoCCharPtr,
    dflt: i32,
) -> i32 {
    let title = title.into_c_char_ptr();
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title
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
    dlg_type: i32,
    title: impl crate::strings::IntoCCharPtr,
    message: impl crate::strings::IntoCCharPtr,
    dflt: i32,
) -> i32 {
    let title = title.into_c_char_ptr();
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title
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
