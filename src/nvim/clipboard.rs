//! Clipboard register handling (`src/nvim/clipboard.c`).

use crate::api::private::helpers::{api_clear_string, cstr_to_string};
use crate::eval::typval::{
    VarType, TvList, tv_list_alloc, tv_list_append_list, tv_list_append_string,
};
use crate::eval::{eval_call_provider, eval_has_provider};
use crate::mcursor::mc_replaying;
use crate::message::{emsg, msg, redirecting};
use crate::option_vars::*;
use crate::register::{
    EMPTY_YANKREG, MotionType, PLUS_REGISTER, STAR_REGISTER, get_y_previous, get_y_register,
    update_yankreg_width, Yankreg,
};
use std::ffi::{CStr, c_char, c_int, c_uint};

const K_OPT_CB_FLAG_UNNAMED: c_uint = 0x01;
const K_OPT_CB_FLAG_UNNAMEDPLUS: c_uint = 0x02;

const CTRL_V: u8 = 0x16;

struct ClipboardModuleState {
    BATCH_CHANGE_COUNT: i32,
    CLIPBOARD_DELAY_UPDATE: bool,
    CLIPBOARD_NEEDS_UPDATE: bool,
    CLIPBOARD_DIDWARN: bool,
}

impl ClipboardModuleState {
    const fn new() -> Self {
        Self {
            BATCH_CHANGE_COUNT: 0,
            CLIPBOARD_DELAY_UPDATE: false,
            CLIPBOARD_NEEDS_UPDATE: false,
            CLIPBOARD_DIDWARN: false,
        }
    }
}

static CLIP_STATE: crate::globals::GlobalState<ClipboardModuleState> = crate::globals::GlobalState::new(ClipboardModuleState::new());

#[inline]
fn clip_st() -> &'static mut ClipboardModuleState {
    unsafe { &mut *CLIP_STATE.as_ptr() }
}

const MSG_NO_CLIP: &CStr = c"clipboard: No provider. Try \":checkhealth\" or \":h clipboard\".";

/// Determine if register `*name` should be used as a clipboard (`adjust_clipboard_name`).
pub fn adjust_clipboard_name(
    name: &mut c_int,
    quiet: bool,
    writing: bool,
) -> *mut Yankreg {
    let reg_char = *name;
        let flags = opt_vars().cb_flags;
        let explicit_cb_reg = reg_char == b'*' as c_int || reg_char == b'+' as c_int;
        let implicit_cb_reg =
            reg_char == 0 && (flags & (K_OPT_CB_FLAG_UNNAMED | K_OPT_CB_FLAG_UNNAMEDPLUS)) != 0;
        if !explicit_cb_reg && !implicit_cb_reg {
            return std::ptr::null_mut();
        }

        if !eval_has_provider(c"clipboard", false) {
            if clip_st().BATCH_CHANGE_COUNT <= 1
                && !quiet
                && (!clip_st().CLIPBOARD_DIDWARN || (explicit_cb_reg && redirecting() == 0))
            {
                clip_st().CLIPBOARD_DIDWARN = true;
                msg(MSG_NO_CLIP, 0);
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
                clip_st().CLIPBOARD_NEEDS_UPDATE = false;
            }
            target
        } else {
            if writing && clip_st().CLIPBOARD_DELAY_UPDATE {
                clip_st().CLIPBOARD_NEEDS_UPDATE = true;
                return std::ptr::null_mut();
            } else if !writing && clip_st().CLIPBOARD_NEEDS_UPDATE {
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

/// Read clipboard contents into `*target` (`get_clipboard`).
pub fn get_clipboard(mut name: c_int, target: *mut *mut Yankreg, quiet: bool) -> bool {
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
        tv_list_append_string( &mut *args, &regname, 1);

        let result = eval_call_provider(
            c"clipboard",
            c"get",
            &mut *args,
            false,
        );

        let fail_cleanup =
            |reg: &mut Yankreg, errmsg: bool, target: *mut *mut Yankreg| -> bool {
                *reg = EMPTY_YANKREG;
                if errmsg {
                    emsg(c"clipboard: provider returned invalid data");
                }
                *target = reg as *mut Yankreg;
                false
            };

        if result.v_type != VarType::VAR_LIST {
            if result.v_type == VarType::VAR_NUMBER && result.vval.v_number() == 0 {
                errmsg = false;
            }
            return fail_cleanup(reg, errmsg, target);
        }

        let res = result.vval.v_list();
        let res_len = if res.is_null() { 0 } else { (*res).lv_len };
        let lines: *mut TvList;

        if res_len == 2
            && !(*res).lv_first.is_null()
            && (*(*res).lv_first).li_tv.v_type == VarType::VAR_LIST
        {
            lines = (*(*res).lv_first).li_tv.vval.v_list();
            let last_li = (*res).lv_last;
            if last_li.is_null() || (*last_li).li_tv.v_type != VarType::VAR_STRING {
                return fail_cleanup(reg, errmsg, target);
            }
            let regtype = (*last_li).li_tv.vval.v_string();
            if regtype.is_null() || crate::memory::cstr_len(regtype) > 1 {
                return fail_cleanup(reg, errmsg, target);
            }
            reg.y_type = match crate::memory::cstr_first_byte(regtype) {
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
        reg.additional_data = None;
        reg.timestamp = 0;

        for li in crate::eval::typval::list_iter_mut(if lines.is_null() {
            std::ptr::null_mut()
        } else {
            (*lines).lv_first
        }) {
            if li.li_tv.v_type != VarType::VAR_STRING {
                return fail_cleanup(reg, errmsg, target);
            }
            let s = li.li_tv.vval.v_string();
            reg.y_array
                .push(cstr_to_string(if s.is_null() { c"".as_ptr() } else { s }));
        }

        if reg.y_array.last().is_some_and(|last| last.size == 0) {
            if reg.y_type != MotionType::kMTCharWise {
                if let Some(mut popped) = reg.y_array.pop() {
                    api_clear_string(&mut popped);
                }
                if reg.y_type == MotionType::kMTUnknown {
                    reg.y_type = MotionType::kMTLineWise;
                }
            }
        } else if reg.y_type == MotionType::kMTUnknown {
            reg.y_type = MotionType::kMTCharWise;
        }

        update_yankreg_width(&mut *reg_ptr);
        *target = reg_ptr;
        true
    }
}

/// Write `reg` to the clipboard provider (`set_clipboard`).
pub fn set_clipboard(mut name: c_int, reg: &Yankreg) {
    unsafe {
        if mc_replaying() || adjust_clipboard_name(&mut name, false, true).is_null() {
            return;
        }

                let extra = usize::from(reg.y_type != MotionType::kMTCharWise);
        let lines = tv_list_alloc((reg.y_array.len() + extra) as isize);

        for &item in &reg.y_array {
            tv_list_append_string( &mut *lines, item.data, item.size as isize);
        }

        let regtype: c_char = match reg.y_type {
            MotionType::kMTLineWise => {
                tv_list_append_string( &mut *lines, std::ptr::null(), 0);
                b'V' as c_char
            }
            MotionType::kMTCharWise => b'v' as c_char,
            MotionType::kMTBlockWise => {
                tv_list_append_string( &mut *lines, std::ptr::null(), 0);
                b'b' as c_char
            }
            MotionType::kMTUnknown => std::process::abort(),
        };

        let args = tv_list_alloc(3);
        tv_list_append_list( &mut *args, lines);
        tv_list_append_string( &mut *args, &regtype, 1);
        let name_char = name as c_char;
        tv_list_append_string( &mut *args, &name_char, 1);

        let _ = eval_call_provider(
            c"clipboard",
            c"set",
            &mut *args,
            true,
        );
    }
}

/// Defer clipboard updates during batch operations (`start_batch_changes`).
pub fn start_batch_changes() {
    clip_st().BATCH_CHANGE_COUNT += 1;
        if clip_st().BATCH_CHANGE_COUNT > 1 {
            return;
        }
        clip_st().CLIPBOARD_DELAY_UPDATE = true;
}

/// End batch operation and flush deferred clipboard update if needed (`end_batch_changes`).
pub fn end_batch_changes() {
    clip_st().BATCH_CHANGE_COUNT -= 1;
        if clip_st().BATCH_CHANGE_COUNT > 0 {
            return;
        }
        clip_st().CLIPBOARD_DELAY_UPDATE = false;
        if clip_st().CLIPBOARD_NEEDS_UPDATE {
            clip_st().CLIPBOARD_NEEDS_UPDATE = false;
            set_clipboard(0, unsafe { &*get_y_previous() });
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_changes_nesting_toggles_delay_flag() {
        start_batch_changes();
            assert!(clip_st().CLIPBOARD_DELAY_UPDATE);
            start_batch_changes();
            end_batch_changes();
            assert!(clip_st().CLIPBOARD_DELAY_UPDATE);
            end_batch_changes();
            assert!(!clip_st().CLIPBOARD_DELAY_UPDATE);
    }
}
