//! User prompts and console dialogs (`yes`/`no`, number, `confirm()`) (`src/nvim/dialog.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc
)]

use std::ffi::{CStr, c_char, c_int, c_void};

use crate::eval::deprecated::Callback;
use crate::memory::{xfree, xmalloc, xstrdup};

const DLG_BUTTON_SEP: u8 = b'\n';
const DLG_HOTKEY_CHAR: u8 = b'&';

const CTRL_C: c_int = 0x03;
const CAR: c_int = 0x0d;
const ESC: c_int = 0x1b;

const HLF_M: c_int = 10;
const HLF_R: c_int = 18;

const K_UI_MESSAGES: c_int = 4;
const K_MOUSE_COMMAND: c_int = b'c' as c_int;
const EXPAND_NOTHING: c_int = 0;
const REMAP_YES: c_int = 0;

const VIM_YES: c_int = 2;
const VIM_NO: c_int = 3;
const VIM_CANCEL: c_int = 4;
const VIM_ALL: c_int = 5;
const VIM_DISCARDALL: c_int = 6;

const HAS_HOTKEY_LEN: usize = 30;
const HOTK_LEN: usize = 4;

#[unsafe(no_mangle)]
pub static mut confirm_msg: *mut c_char = std::ptr::null_mut();

#[unsafe(no_mangle)]
pub static mut confirm_buttons: *mut c_char = std::ptr::null_mut();

#[cfg(not(test))]
unsafe extern "C" {
    static mut State: c_int;
    static mut no_wait_return: c_int;
    static mut need_wait_return: bool;
    static mut msg_scrolled: c_int;
    static mut keep_msg: *mut c_char;
    static mut keep_msg_hl_id: c_int;
    static mut cmdline_row: c_int;
    static mut msg_row: c_int;
    static mut no_mapping: c_int;
    static mut allow_keys: c_int;
    static mut silent_mode: bool;
    static mut msg_silent: c_int;
    static mut msg_didout: bool;
    static mut msg_didany: bool;

    fn ui_has(ext: c_int) -> bool;
    fn ui_mouse_has(mode: c_int) -> bool;
    fn ui_call_busy_stop();
    fn ui_flush();
    fn ui_active() -> usize;
    fn input_available() -> bool;
    fn setmouse();
    fn msg_putchar(c: c_int);
    fn set_keep_msg(s: *const c_char, hl_id: c_int);
    fn msg_end_prompt();
    fn display_confirm_msg();

    fn getcmdline_prompt(
        firstc: c_int,
        prompt: *const c_char,
        hl_id: c_int,
        xp_context: c_int,
        xp_arg: *const c_char,
        highlight_callback: Callback,
        one_key: bool,
        mouse_used: *mut bool,
    ) -> *mut c_char;

    fn ins_typebuf(
        str_ptr: *const c_char,
        noremap: c_int,
        offset: c_int,
        nottyped: bool,
        silent: bool,
    ) -> c_int;

    fn mb_tolower(a: c_int) -> c_int;
    fn utf_ptr2char(p: *const c_char) -> c_int;
    fn utfc_ptr2len(p: *const c_char) -> c_int;
    fn utf_ptr2len(p: *const c_char) -> c_int;
    fn utf_char2bytes(c: c_int, buf: *mut c_char) -> c_int;
}

#[cfg(not(test))]
unsafe fn copy_char(from: *const c_char, to: *mut c_char, lowercase: bool) -> usize {
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

#[cfg(not(test))]
unsafe fn console_dialog_alloc(
    message: *const c_char,
    buttons: *const c_char,
    has_hotkey: &mut [bool; HAS_HOTKEY_LEN],
) -> *mut c_char {
    unsafe {
        let mut lenhotkey = HOTK_LEN;
        has_hotkey[0] = false;

        let mut button_len = 0usize;
        let mut idx = 0usize;
        let mut r = buttons;
        while *r != 0 {
            let b = *r as u8;
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

        let msg_bytes = CStr::from_ptr(message).to_bytes();
        let msg_len = msg_bytes.len() + 3;
        button_len += CStr::from_ptr(buttons).to_bytes().len() + 3;
        lenhotkey += 1;

        if !has_hotkey[0] {
            button_len += 2;
        }

        confirm_msg = xmalloc(msg_len).cast::<c_char>();
        let dst = confirm_msg.cast::<u8>();
        if ui_has(K_UI_MESSAGES) {
            std::ptr::copy_nonoverlapping(msg_bytes.as_ptr(), dst, msg_bytes.len());
            *dst.add(msg_bytes.len()) = 0;
        } else {
            *dst = b'\n';
            std::ptr::copy_nonoverlapping(msg_bytes.as_ptr(), dst.add(1), msg_bytes.len());
            *dst.add(1 + msg_bytes.len()) = b'\n';
            *dst.add(2 + msg_bytes.len()) = 0;
        }

        xfree(confirm_buttons.cast::<c_void>());
        confirm_buttons = xmalloc(button_len).cast::<c_char>();

        xmalloc(lenhotkey).cast::<c_char>()
    }
}

#[cfg(not(test))]
unsafe fn copy_confirm_hotkeys(
    buttons: *const c_char,
    mut default_button_idx: c_int,
    has_hotkey: &[bool; HAS_HOTKEY_LEN],
    mut hotkeys_ptr: *mut c_char,
) {
    unsafe {
        let first_len = copy_char(buttons, hotkeys_ptr, true);
        *hotkeys_ptr.add(first_len) = 0;

        let mut first_hotkey = !has_hotkey[0];
        let mut msgp = confirm_buttons;
        let mut idx = 0usize;
        let mut r = buttons;

        while *r != 0 {
            let b = *r as u8;
            if b == DLG_BUTTON_SEP {
                *msgp = b',' as c_char;
                *msgp.add(1) = b' ' as c_char;
                msgp = msgp.add(2);

                hotkeys_ptr = hotkeys_ptr.add(CStr::from_ptr(hotkeys_ptr).to_bytes().len());
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
                if *r as u8 == DLG_HOTKEY_CHAR {
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

#[cfg(not(test))]
unsafe fn msg_show_console_dialog(
    message: *const c_char,
    buttons: *const c_char,
    dfltbutton: c_int,
) -> *mut c_char {
    unsafe {
        let mut has_hotkey = [false; HAS_HOTKEY_LEN];
        let hotk = console_dialog_alloc(message, buttons, &mut has_hotkey);
        copy_confirm_hotkeys(buttons, dfltbutton, &has_hotkey, hotk);
        display_confirm_msg();
        hotk
    }
}

/// Ask for a `'y'` or `'n'` reply from the user (`ask_yesno`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ask_yesno(str_ptr: *const c_char) -> c_int {
    #[cfg(test)]
    {
        let _ = str_ptr;
        b'n' as c_int
    }
    #[cfg(not(test))]
    unsafe {
        let save_state = State;
        no_wait_return += 1;

        let s_bytes = CStr::from_ptr(str_ptr).to_bytes();
        let suffix = b" (y/n)?\0";
        let prompt = xmalloc(s_bytes.len() + suffix.len()).cast::<c_char>();
        std::ptr::copy_nonoverlapping(s_bytes.as_ptr(), prompt.cast::<u8>(), s_bytes.len());
        std::ptr::copy_nonoverlapping(
            suffix.as_ptr(),
            prompt.cast::<u8>().add(s_bytes.len()),
            suffix.len(),
        );

        let mut r: c_int = b' ' as c_int;
        while r != (b'y' as c_int) && r != (b'n' as c_int) {
            r = prompt_for_input(prompt, HLF_R, true, std::ptr::null_mut());
            if r == CTRL_C || r == ESC {
                r = b'n' as c_int;
                if !ui_has(K_UI_MESSAGES) {
                    msg_putchar(r);
                }
            }
        }

        need_wait_return = msg_scrolled != 0;
        no_wait_return -= 1;
        State = save_state;
        setmouse();
        xfree(prompt.cast::<c_void>());
        r
    }
}

/// Ask the user for input through a cmdline prompt (`prompt_for_input`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn prompt_for_input(
    prompt: *mut c_char,
    hl_id: c_int,
    one_key: bool,
    mouse_used: *mut bool,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (prompt, hl_id, one_key, mouse_used);
        0
    }
    #[cfg(not(test))]
    unsafe {
        let mut ret = if one_key { ESC } else { 0 };
        let kmsg = if !keep_msg.is_null() {
            xstrdup(keep_msg)
        } else {
            std::ptr::null_mut()
        };

        let prompt_ptr: *const c_char = if prompt.is_null() {
            if !mouse_used.is_null() && ui_mouse_has(K_MOUSE_COMMAND) {
                c"Type number and <Enter> or click with the mouse (q or empty cancels): ".as_ptr()
            } else {
                c"Type number and <Enter> (q or empty cancels): ".as_ptr()
            }
        } else {
            prompt
        };

        cmdline_row = msg_row;
        ui_call_busy_stop();
        ui_flush();

        no_mapping += 1;
        allow_keys += 1;
        let resp = getcmdline_prompt(
            -1,
            prompt_ptr,
            hl_id,
            EXPAND_NOTHING,
            std::ptr::null(),
            Callback {
                data: std::ptr::null_mut(),
                cb_type: 0,
            },
            one_key,
            mouse_used,
        );
        allow_keys -= 1;
        no_mapping -= 1;

        if !resp.is_null() {
            if one_key {
                ret = *resp as c_int;
            } else {
                let s = CStr::from_ptr(resp).to_string_lossy();
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
            xfree(resp.cast::<c_void>());
        }

        if !kmsg.is_null() {
            set_keep_msg(kmsg, keep_msg_hl_id);
            xfree(kmsg.cast::<c_void>());
        }

        ret
    }
}

/// Generic console dialog handler (`do_dialog`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_dialog(
    _dlg_type: c_int,
    _title: *const c_char,
    message: *const c_char,
    buttons: *const c_char,
    dfltbutton: c_int,
    _textfield: *const c_char,
    ex_cmd: c_int,
) -> c_int {
    #[cfg(test)]
    {
        let _ = (message, buttons, ex_cmd);
        dfltbutton
    }
    #[cfg(not(test))]
    unsafe {
        if silent_mode {
            return dfltbutton;
        }

        let save_msg_silent = msg_silent;
        let old_state = State;
        msg_silent = 0;
        no_wait_return += 1;

        let hotkeys = msg_show_console_dialog(message, buttons, dfltbutton);
        let retval: c_int;

        loop {
            if ui_active() == 0 && !input_available() {
                retval = dfltbutton;
                break;
            }

            let mut c = prompt_for_input(confirm_buttons, HLF_M, true, std::ptr::null_mut());
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
                        msg_didout = false;
                        msg_didany = false;
                        continue;
                    }
                    if c == (b':' as c_int) && ex_cmd != 0 {
                        retval = dfltbutton;
                        ins_typebuf(c":".as_ptr(), REMAP_YES, 0, false, false);
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
                    msg_didout = false;
                    msg_didany = false;
                }
            }
        }

        xfree(hotkeys.cast::<c_void>());
        xfree(confirm_msg.cast::<c_void>());
        confirm_msg = std::ptr::null_mut();

        msg_silent = save_msg_silent;
        State = old_state;
        setmouse();
        no_wait_return -= 1;
        msg_end_prompt();

        retval
    }
}

/// Yes/No dialog (`vim_dialog_yesno`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_dialog_yesno(
    dlg_type: c_int,
    title: *mut c_char,
    message: *mut c_char,
    dflt: c_int,
) -> c_int {
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title.cast_const()
    };
    if unsafe {
        do_dialog(
            dlg_type,
            t,
            message,
            c"&Yes\n&No".as_ptr(),
            dflt,
            std::ptr::null(),
            0,
        )
    } == 1
    {
        VIM_YES
    } else {
        VIM_NO
    }
}

/// Yes/No/Cancel dialog (`vim_dialog_yesnocancel`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_dialog_yesnocancel(
    dlg_type: c_int,
    title: *mut c_char,
    message: *mut c_char,
    dflt: c_int,
) -> c_int {
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title.cast_const()
    };
    match unsafe {
        do_dialog(
            dlg_type,
            t,
            message,
            c"&Yes\n&No\n&Cancel".as_ptr(),
            dflt,
            std::ptr::null(),
            0,
        )
    } {
        1 => VIM_YES,
        2 => VIM_NO,
        _ => VIM_CANCEL,
    }
}

/// Yes/No/Save All/Discard All/Cancel dialog (`vim_dialog_yesnoallcancel`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_dialog_yesnoallcancel(
    dlg_type: c_int,
    title: *mut c_char,
    message: *mut c_char,
    dflt: c_int,
) -> c_int {
    let t = if title.is_null() {
        c"Question".as_ptr()
    } else {
        title.cast_const()
    };
    match unsafe {
        do_dialog(
            dlg_type,
            t,
            message,
            c"&Yes\n&No\nSave &All\n&Discard All\n&Cancel".as_ptr(),
            dflt,
            std::ptr::null(),
            0,
        )
    } {
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
        unsafe {
            assert_eq!(
                vim_dialog_yesno(0, std::ptr::null_mut(), c"Save?".as_ptr().cast_mut(), 1),
                VIM_YES
            );
            assert_eq!(
                vim_dialog_yesno(0, std::ptr::null_mut(), c"Save?".as_ptr().cast_mut(), 2),
                VIM_NO
            );
        }
    }
}
