//! Clipboard register handling (`src/nvim/clipboard.c`).

use crate::api::private::validate::NvimString;
use crate::eval::typval::{
    VarType, list_T, tv_list_alloc, tv_list_append_list, tv_list_append_string,
};
use crate::eval::{eval_call_provider, eval_has_provider};
use crate::mcursor::mc_replaying;
use crate::memory::{xfree, xstrdup};
use crate::message::{emsg, msg, redirecting};
use crate::option_vars::*;
use crate::register::{
    EMPTY_YANKREG, MotionType, PLUS_REGISTER, STAR_REGISTER, get_y_previous, get_y_register,
    update_yankreg_width, yankreg_T,
};
use std::ffi::{CStr, c_char, c_int, c_uint};

const K_OPT_CB_FLAG_UNNAMED: c_uint = 0x01;
const K_OPT_CB_FLAG_UNNAMEDPLUS: c_uint = 0x02;

const CTRL_V: u8 = 0x16;

static mut BATCH_CHANGE_COUNT: i32 = 0;
static mut CLIPBOARD_DELAY_UPDATE: bool = false;
static mut CLIPBOARD_NEEDS_UPDATE: bool = false;
static mut CLIPBOARD_DIDWARN: bool = false;

const MSG_NO_CLIP: &CStr = c"clipboard: No provider. Try \":checkhealth\" or \":h clipboard\".";

#[inline]
unsafe fn cstr_to_nvim_string(s: *const c_char) -> NvimString {
    let ptr = if s.is_null() { c"".as_ptr() } else { s };
    let len = unsafe { CStr::from_ptr(ptr).to_bytes().len() };
    let data = unsafe { xstrdup(ptr) };
    NvimString { data, size: len }
}

/// Determine if register `*name` should be used as a clipboard (`adjust_clipboard_name`).
pub unsafe fn adjust_clipboard_name(
    name: *mut c_int,
    quiet: bool,
    writing: bool,
) -> *mut yankreg_T {
    unsafe {
        let reg_char = *name;
        let flags = cb_flags;
        let explicit_cb_reg = reg_char == b'*' as c_int || reg_char == b'+' as c_int;
        let implicit_cb_reg =
            reg_char == 0 && (flags & (K_OPT_CB_FLAG_UNNAMED | K_OPT_CB_FLAG_UNNAMEDPLUS)) != 0;
        if !explicit_cb_reg && !implicit_cb_reg {
            return std::ptr::null_mut();
        }

        if !eval_has_provider(c"clipboard".as_ptr(), false) {
            if BATCH_CHANGE_COUNT <= 1
                && !quiet
                && (!CLIPBOARD_DIDWARN || (explicit_cb_reg && redirecting() == 0))
            {
                CLIPBOARD_DIDWARN = true;
                msg(MSG_NO_CLIP.as_ptr(), 0);
            }
            return std::ptr::null_mut();
        }

        if explicit_cb_reg {
            let target = get_y_register(if reg_char == b'*' as c_int {
                STAR_REGISTER
            } else {
                PLUS_REGISTER
            });
            let match_flag = if reg_char == b'*' as c_int {
                K_OPT_CB_FLAG_UNNAMED
            } else {
                K_OPT_CB_FLAG_UNNAMEDPLUS
            };
            if writing && (flags & match_flag) != 0 {
                CLIPBOARD_NEEDS_UPDATE = false;
            }
            target
        } else {
            if writing && CLIPBOARD_DELAY_UPDATE {
                CLIPBOARD_NEEDS_UPDATE = true;
                return std::ptr::null_mut();
            } else if !writing && CLIPBOARD_NEEDS_UPDATE {
                return std::ptr::null_mut();
            }

            if (flags & K_OPT_CB_FLAG_UNNAMEDPLUS) != 0 {
                *name = if (flags & K_OPT_CB_FLAG_UNNAMED) != 0 && writing {
                    b'"' as c_int
                } else {
                    b'+' as c_int
                };
                get_y_register(PLUS_REGISTER)
            } else {
                *name = b'*' as c_int;
                get_y_register(STAR_REGISTER)
            }
        }
    }
}

/// Read clipboard contents into `*target` (`get_clipboard`).
pub unsafe fn get_clipboard(mut name: c_int, target: *mut *mut yankreg_T, quiet: bool) -> bool {
    unsafe {
        let implicit = name == 0;
        let mut errmsg = true;

        let reg_ptr = adjust_clipboard_name(&mut name, quiet, false);
        if reg_ptr.is_null() {
            return false;
        }
        if mc_replaying() {
            if implicit {
                return false;
            }
            *target = reg_ptr;
            return true;
        }
        *reg_ptr = EMPTY_YANKREG;
        let reg = &mut *reg_ptr;

        let args = tv_list_alloc(1);
        let regname = name as c_char;
        tv_list_append_string(args, &regname, 1);

        let result = eval_call_provider(
            c"clipboard".as_ptr().cast_mut(),
            c"get".as_ptr().cast_mut(),
            args,
            false,
        );

        let fail_cleanup =
            |reg: &mut yankreg_T, errmsg: bool, target: *mut *mut yankreg_T| -> bool {
                for item in &mut reg.y_array {
                    xfree(item.data.cast());
                }
                reg.y_array.clear();
                reg.y_allocated = false;
                reg.additional_data = std::ptr::null_mut();
                reg.timestamp = 0;
                if errmsg {
                    emsg(c"clipboard: provider returned invalid data".as_ptr());
                }
                *target = reg as *mut yankreg_T;
                false
            };

        if result.v_type != VarType::VAR_LIST {
            if result.v_type == VarType::VAR_NUMBER && result.vval.v_number == 0 {
                errmsg = false;
            }
            return fail_cleanup(reg, errmsg, target);
        }

        let res = result.vval.v_list;
        let res_len = if res.is_null() { 0 } else { (*res).lv_len };
        let lines: *mut list_T;

        if res_len == 2
            && !(*res).lv_first.is_null()
            && (*(*res).lv_first).li_tv.v_type == VarType::VAR_LIST
        {
            lines = (*(*res).lv_first).li_tv.vval.v_list;
            let last_li = (*res).lv_last;
            if last_li.is_null() || (*last_li).li_tv.v_type != VarType::VAR_STRING {
                return fail_cleanup(reg, errmsg, target);
            }
            let regtype = (*last_li).li_tv.vval.v_string;
            if regtype.is_null() || CStr::from_ptr(regtype).to_bytes().len() > 1 {
                return fail_cleanup(reg, errmsg, target);
            }
            reg.y_type = match *regtype as u8 {
                0 => MotionType::kMTUnknown,
                b'v' | b'c' => MotionType::kMTCharWise,
                b'V' | b'l' => MotionType::kMTLineWise,
                b'b' | CTRL_V => MotionType::kMTBlockWise,
                _ => return fail_cleanup(reg, errmsg, target),
            };
        } else {
            lines = res;
            reg.y_type = MotionType::kMTUnknown;
        }

        let lines_len = if lines.is_null() {
            0
        } else {
            (*lines).lv_len.max(0) as usize
        };
        reg.y_array = Vec::with_capacity(lines_len);
        reg.y_allocated = true;
        reg.y_width = 0;
        reg.additional_data = std::ptr::null_mut();
        reg.timestamp = 0;

        let mut li = if lines.is_null() {
            std::ptr::null_mut()
        } else {
            (*lines).lv_first
        };
        while !li.is_null() {
            if (*li).li_tv.v_type != VarType::VAR_STRING {
                return fail_cleanup(reg, errmsg, target);
            }
            let s = (*li).li_tv.vval.v_string;
            reg.y_array.push(cstr_to_nvim_string(s));
            li = (*li).li_next;
        }

        if reg.y_array.last().is_some_and(|last| last.size == 0) {
            if reg.y_type != MotionType::kMTCharWise {
                let popped = reg.y_array.pop().unwrap();
                xfree(popped.data.cast());
                if reg.y_type == MotionType::kMTUnknown {
                    reg.y_type = MotionType::kMTLineWise;
                }
            }
        } else if reg.y_type == MotionType::kMTUnknown {
            reg.y_type = MotionType::kMTCharWise;
        }

        update_yankreg_width(reg_ptr);
        *target = reg_ptr;
        true
    }
}

/// Write `reg` to the clipboard provider (`set_clipboard`).
pub unsafe fn set_clipboard(mut name: c_int, reg: *mut yankreg_T) {
    unsafe {
        if mc_replaying() || adjust_clipboard_name(&mut name, false, true).is_null() {
            return;
        }

        let reg = &*reg;
        let extra = usize::from(reg.y_type != MotionType::kMTCharWise);
        let lines = tv_list_alloc((reg.y_array.len() + extra) as isize);

        for &item in &reg.y_array {
            tv_list_append_string(lines, item.data, item.size as isize);
        }

        let regtype: c_char = match reg.y_type {
            MotionType::kMTLineWise => {
                tv_list_append_string(lines, std::ptr::null(), 0);
                b'V' as c_char
            }
            MotionType::kMTCharWise => b'v' as c_char,
            MotionType::kMTBlockWise => {
                tv_list_append_string(lines, std::ptr::null(), 0);
                b'b' as c_char
            }
            MotionType::kMTUnknown => std::process::abort(),
        };

        let args = tv_list_alloc(3);
        tv_list_append_list(args, lines);
        tv_list_append_string(args, &regtype, 1);
        let name_char = name as c_char;
        tv_list_append_string(args, &name_char, 1);

        let _ = eval_call_provider(
            c"clipboard".as_ptr().cast_mut(),
            c"set".as_ptr().cast_mut(),
            args,
            true,
        );
    }
}

/// Defer clipboard updates during batch operations (`start_batch_changes`).
pub fn start_batch_changes() {
    unsafe {
        BATCH_CHANGE_COUNT += 1;
        if BATCH_CHANGE_COUNT > 1 {
            return;
        }
        CLIPBOARD_DELAY_UPDATE = true;
    }
}

/// End batch operation and flush deferred clipboard update if needed (`end_batch_changes`).
pub fn end_batch_changes() {
    unsafe {
        BATCH_CHANGE_COUNT -= 1;
        if BATCH_CHANGE_COUNT > 0 {
            return;
        }
        CLIPBOARD_DELAY_UPDATE = false;
        if CLIPBOARD_NEEDS_UPDATE {
            CLIPBOARD_NEEDS_UPDATE = false;
            set_clipboard(0, get_y_previous());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_changes_nesting_toggles_delay_flag() {
        unsafe {
            start_batch_changes();
            assert!(CLIPBOARD_DELAY_UPDATE);
            start_batch_changes();
            end_batch_changes();
            assert!(CLIPBOARD_DELAY_UPDATE);
            end_batch_changes();
            assert!(!CLIPBOARD_DELAY_UPDATE);
        }
    }
}
