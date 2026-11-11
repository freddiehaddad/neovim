//! User prompts and console dialogs (`yes`/`no`, number, `confirm()`) (`src/nvim/dialog.c`).
#![forbid(unsafe_code)]

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

struct DialogModuleState {
    confirm_msg: std::cell::RefCell<Option<std::ffi::CString>>,
    confirm_buttons: std::cell::RefCell<Vec<u8>>,
}

impl DialogModuleState {
    const fn new() -> Self {
        Self {
            confirm_msg: std::cell::RefCell::new(None),
            confirm_buttons: std::cell::RefCell::new(Vec::new()),
        }
    }
}

std::thread_local! {
    static DLG_STATE: DialogModuleState = const { DialogModuleState::new() };
}

pub(crate) fn has_confirm_msg() -> bool {
    DLG_STATE.with(|st| st.confirm_msg.borrow().is_some())
}

pub(crate) fn clone_confirm_msg() -> Option<std::ffi::CString> {
    DLG_STATE.with(|st| st.confirm_msg.borrow().clone())
}

pub(crate) fn clone_confirm_buttons() -> Vec<u8> {
    DLG_STATE.with(|st| st.confirm_buttons.borrow().clone())
}

fn copy_char(from: &[u8], to: &mut [u8], lowercase: bool) -> usize {
    if from.is_empty() {
        return 0;
    }
    if lowercase {
        let c = mb_tolower(crate::mbyte::utf_slice2char(from));
        utf_char2bytes(c, to) as usize
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

    let msg_cstr = if ui_has(K_UI_MESSAGES) {
        message.to_owned()
    } else {
        let mut v = Vec::with_capacity(msg_bytes.len() + 2);
        v.push(b'\n');
        v.extend_from_slice(msg_bytes);
        v.push(b'\n');
        std::ffi::CString::new(v).unwrap_or_default()
    };
    DLG_STATE.with(|st| {
        *st.confirm_msg.borrow_mut() = Some(msg_cstr);
        *st.confirm_buttons.borrow_mut() = vec![0u8; button_len];
    });

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

    DLG_STATE.with(|st| {
        let mut confirm_buttons = st.confirm_buttons.borrow_mut();
        while pos < btn_bytes.len() {
            let b = btn_bytes[pos];
            if b == DLG_BUTTON_SEP {
                confirm_buttons[msg_pos] = b',';
                confirm_buttons[msg_pos + 1] = b' ';
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
                    confirm_buttons[msg_pos] = DLG_HOTKEY_CHAR;
                    msg_pos += 1;
                } else if pos < btn_bytes.len() {
                    confirm_buttons[msg_pos] = if default_button_idx == 1 { b'[' } else { b'(' };
                    msg_pos += 1;
                    let clen = copy_char(&btn_bytes[pos..], &mut confirm_buttons[msg_pos..], false);
                    msg_pos += clen;
                    confirm_buttons[msg_pos] = if default_button_idx == 1 { b']' } else { b')' };
                    msg_pos += 1;

                    let hlen = copy_char(&btn_bytes[pos..], &mut hotkeys[hk_pos..], true);
                    hotkeys[hk_pos + hlen] = 0;
                }
            } else {
                let clen = copy_char(&btn_bytes[pos..], &mut confirm_buttons[msg_pos..], false);
                msg_pos += clen;
            }

            if pos >= btn_bytes.len() {
                break;
            }
            pos += crate::mbyte::utf_slice2len(&btn_bytes[pos..]).max(1);
        }

        confirm_buttons[msg_pos] = b':';
        confirm_buttons[msg_pos + 1] = b' ';
        confirm_buttons[msg_pos + 2] = 0;
    });
}

fn msg_show_console_dialog(message: &CStr, buttons: &CStr, dfltbutton: i32) -> Vec<u8> {
    let mut has_hotkey = [false; HAS_HOTKEY_LEN];
    let mut hotk = console_dialog_alloc(message, buttons, &mut has_hotkey);
    copy_confirm_hotkeys(buttons, dfltbutton, &has_hotkey, &mut hotk);
    display_confirm_msg();
    hotk
}

/// Ask for a `'y'` or `'n'` reply from the user (`ask_yesno`).
pub fn ask_yesno(str_cstr: &CStr) -> i32 {
    let save_state = g_st().state_global.get();
    g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));

    let s_bytes = str_cstr.to_bytes();
    let suffix = b" (y/n)?\0";
    let mut prompt = Vec::with_capacity(s_bytes.len() + suffix.len());
    prompt.extend_from_slice(s_bytes);
    prompt.extend_from_slice(suffix);
    let prompt_cstr = CStr::from_bytes_until_nul(&prompt).unwrap_or(c"");

    let mut r: i32 = b' ' as i32;
    while r != (b'y' as i32) && r != (b'n' as i32) {
        r = prompt_for_input(Some(prompt_cstr), HLF_R, true, None);
        if r == CTRL_C || r == ESC {
            r = b'n' as i32;
            if !ui_has(K_UI_MESSAGES) {
                msg_putchar(r);
            }
        }
    }

    g_st().need_wait_return.set(g_st().msg_scrolled.get() != 0);
    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    g_st().state_global.set(save_state);
    setmouse();
    r
}

/// Ask the user for input through a cmdline prompt (`prompt_for_input`).
pub fn prompt_for_input(
    prompt: Option<&CStr>,
    hl_id: i32,
    one_key: bool,
    mouse_used: Option<&mut bool>,
) -> i32 {
    let mut ret = if one_key { ESC } else { 0 };
    let kmsg = g_st().keep_msg.get().clone();

    let prompt_cstr = if let Some(prompt_cstr) = prompt {
        prompt_cstr
    } else if mouse_used.is_some() && ui_mouse_has(K_MOUSE_COMMAND) {
        c"Type number and <Enter> or click with the mouse (q or empty cancels): "
    } else {
        c"Type number and <Enter> (q or empty cancels): "
    };

    g_st().cmdline_row.set(g_st().msg_row.get());
    ui_call_busy_stop();
    ui_flush();

    g_st().no_mapping.set(g_st().no_mapping.get() + (1));
    g_st().allow_keys.set(g_st().allow_keys.get() + (1));
    let resp = getcmdline_prompt(
        -1,
        Some(prompt_cstr),
        hl_id,
        EXPAND_NOTHING,
        None,
        Callback::NONE,
        one_key,
        mouse_used,
    );
    g_st().allow_keys.set(g_st().allow_keys.get() - (1));
    g_st().no_mapping.set(g_st().no_mapping.get() - (1));

    if let Some(ref resp_cstr) = resp {
        if one_key {
            ret = i32::from(resp_cstr.to_bytes().first().copied().unwrap_or(0));
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
        set_keep_msg(Some(kmsg_cstr.as_c_str()), g_st().keep_msg_hl_id.get());
    }

    ret
}

/// Generic console dialog handler (`do_dialog`).
pub fn do_dialog(
    _dlg_type: i32,
    _title: Option<&CStr>,
    message: &CStr,
    buttons: &CStr,
    dfltbutton: i32,
    _textfield: Option<&CStr>,
    ex_cmd: i32,
) -> i32 {
    if g_st().silent_mode.get() {
        return dfltbutton;
    }

    let save_msg_silent = g_st().msg_silent.get();
    let old_state = g_st().state_global.get();
    g_st().msg_silent.set(0);
    g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));

    let hotkeys_vec = msg_show_console_dialog(message, buttons, dfltbutton);
    let retval: i32;

    loop {
        if (ui_active() == 0 || input_unanswerable()) && input_available() == 0 {
            retval = dfltbutton;
            break;
        }

        let btn_vec = clone_confirm_buttons();
        let btn_cstr = CStr::from_bytes_until_nul(&btn_vec).unwrap_or(c"");
        let mut c = prompt_for_input(Some(btn_cstr), HLF_M, true, None);
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
                    g_st().msg_didout.set(false);
                    g_st().msg_didany.set(false);
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
                g_st().msg_didout.set(false);
                g_st().msg_didany.set(false);
            }
        }
    }

    drop(hotkeys_vec);
    DLG_STATE.with(|st| *st.confirm_msg.borrow_mut() = None);

    g_st().msg_silent.set(save_msg_silent);
    g_st().state_global.set(old_state);
    setmouse();
    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    msg_end_prompt();

    retval
}

/// Yes/No dialog (`vim_dialog_yesno`).
pub fn vim_dialog_yesno(dlg_type: i32, title: Option<&CStr>, message: &CStr, dflt: i32) -> i32 {
    if do_dialog(
        dlg_type,
        Some(title.unwrap_or(c"Question")),
        message,
        c"&Yes\n&No",
        dflt,
        None,
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
    title: Option<&CStr>,
    message: &CStr,
    dflt: i32,
) -> i32 {
    match do_dialog(
        dlg_type,
        Some(title.unwrap_or(c"Question")),
        message,
        c"&Yes\n&No\n&Cancel",
        dflt,
        None,
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
    title: Option<&CStr>,
    message: &CStr,
    dflt: i32,
) -> i32 {
    match do_dialog(
        dlg_type,
        Some(title.unwrap_or(c"Question")),
        message,
        c"&Yes\n&No\nSave &All\n&Discard All\n&Cancel",
        dflt,
        None,
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
        g_st().silent_mode.set(true);
        assert_eq!(vim_dialog_yesno(0, None, c"Save?", 1), VIM_YES);
        assert_eq!(vim_dialog_yesno(0, None, c"Save?", 2), VIM_NO);
    }
}
