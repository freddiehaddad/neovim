//! Clipboard register handling (`src/nvim/clipboard.c`).

use crate::api::private::helpers::{api_clear_string, cstr_to_string};
use crate::eval::typval::{
    TvList, VarType, tv_list_alloc, tv_list_append_bytes, tv_list_append_list,
    tv_list_append_string,
};
use crate::eval::{eval_call_provider, eval_has_provider};
use crate::mcursor::mc_replaying;
use crate::message::{emsg, msg, redirecting};
use crate::option_vars::*;
use crate::register::{
    EMPTY_YANKREG, MotionType, PLUS_REGISTER, STAR_REGISTER, Yankreg, get_y_previous,
    get_y_register, update_yankreg_width,
};

use std::ffi::CStr;

const K_OPT_CB_FLAG_UNNAMED: u32 = 0x01;
const K_OPT_CB_FLAG_UNNAMEDPLUS: u32 = 0x02;

const CTRL_V: u8 = 0x16;

struct ClipboardModuleState {
    batch_change_count: i32,
    clipboard_delay_update: bool,
    clipboard_needs_update: bool,
    clipboard_didwarn: bool,
}

impl ClipboardModuleState {
    const fn new() -> Self {
        Self {
            batch_change_count: 0,
            clipboard_delay_update: false,
            clipboard_needs_update: false,
            clipboard_didwarn: false,
        }
    }
}

static CLIP_STATE: crate::globals::GlobalState<ClipboardModuleState> =
    crate::globals::GlobalState::new(ClipboardModuleState::new());

#[inline]
fn clip_st() -> crate::globals::GraphPtr<ClipboardModuleState> {
    CLIP_STATE.handle()
}

const MSG_NO_CLIP: &CStr = c"clipboard: No provider. Try \":checkhealth\" or \":h clipboard\".";

/// Determine if register `*name` should be used as a clipboard (`adjust_clipboard_name`).
pub fn adjust_clipboard_name(
    name: &mut i32,
    quiet: bool,
    writing: bool,
) -> Option<&'static mut Yankreg> {
    let reg_char = *name;
    let flags = opt_vars().cb_flags;
    let explicit_cb_reg = reg_char == b'*' as i32 || reg_char == b'+' as i32;
    let implicit_cb_reg =
        reg_char == 0 && (flags & (K_OPT_CB_FLAG_UNNAMED | K_OPT_CB_FLAG_UNNAMEDPLUS)) != 0;
    if !explicit_cb_reg && !implicit_cb_reg {
        return None;
    }

    if !eval_has_provider(b"clipboard", false) {
        if clip_st().batch_change_count <= 1
            && !quiet
            && (!clip_st().clipboard_didwarn || (explicit_cb_reg && redirecting() == 0))
        {
            clip_st().clipboard_didwarn = true;
            msg(MSG_NO_CLIP, 0);
        }
        return None;
    }

    if explicit_cb_reg {
        let target = get_y_register(if reg_char == b'*' as i32 {
            STAR_REGISTER
        } else {
            PLUS_REGISTER
        });
        let match_flag = if reg_char == b'*' as i32 {
            K_OPT_CB_FLAG_UNNAMED
        } else {
            K_OPT_CB_FLAG_UNNAMEDPLUS
        };
        if writing && (flags & match_flag) != 0 {
            clip_st().clipboard_needs_update = false;
        }
        Some(target)
    } else {
        if writing && clip_st().clipboard_delay_update {
            clip_st().clipboard_needs_update = true;
            return None;
        } else if !writing && clip_st().clipboard_needs_update {
            return None;
        }

        if (flags & K_OPT_CB_FLAG_UNNAMEDPLUS) != 0 {
            *name = if (flags & K_OPT_CB_FLAG_UNNAMED) != 0 && writing {
                b'"' as i32
            } else {
                b'+' as i32
            };
            Some(get_y_register(PLUS_REGISTER))
        } else {
            *name = b'*' as i32;
            Some(get_y_register(STAR_REGISTER))
        }
    }
}

/// Read clipboard contents into `*target` (`get_clipboard`).
pub fn get_clipboard(
    mut name: i32,
    target: &mut Option<&'static mut Yankreg>,
    quiet: bool,
) -> bool {
    let implicit = name == 0;
    let mut errmsg = true;

    let Some(reg) = adjust_clipboard_name(&mut name, quiet, false) else {
        return false;
    };
    if mc_replaying() {
        if implicit {
            return false;
        }
        *target = Some(reg);
        return true;
    }

    *reg = EMPTY_YANKREG;

    let args = tv_list_alloc(1);
    tv_list_append_bytes(&mut *args, &[name as u8]);

    let result = eval_call_provider(c"clipboard", c"get", &mut *args, false);

    let fail_cleanup =
        |reg: &'static mut Yankreg, errmsg: bool, target: &mut Option<&'static mut Yankreg>| {
            *reg = EMPTY_YANKREG;
            if errmsg {
                emsg(c"clipboard: provider returned invalid data");
            }
            *target = Some(reg);
            false
        };

    if result.v_type != VarType::VarList {
        if result.v_type == VarType::VarNumber && result.vval.v_number() == 0 {
            errmsg = false;
        }
        return fail_cleanup(reg, errmsg, target);
    }

    let res = result.vval.v_list_opt();
    let res_len = res.as_deref().map_or(0, |l| l.lv_len);
    let lines: Option<&'static mut TvList>;

    if res_len == 2
        && let Some(first_li) = res.as_deref().and_then(TvList::first_opt)
        && first_li.li_tv.v_type == VarType::VarList
    {
        lines = first_li.li_tv.vval.v_list_opt();

        let Some(last_li) = res.as_deref().and_then(TvList::last_opt) else {
            return fail_cleanup(reg, errmsg, target);
        };
        if last_li.li_tv.v_type != VarType::VarString {
            return fail_cleanup(reg, errmsg, target);
        }
        let Some(regtype) = last_li.li_tv.vval.v_string_cstr() else {
            return fail_cleanup(reg, errmsg, target);
        };
        let regtype_bytes = regtype.to_bytes();
        if regtype_bytes.len() > 1 {
            return fail_cleanup(reg, errmsg, target);
        }
        reg.y_type = match regtype_bytes.first().copied().unwrap_or(0) {
            0 => MotionType::MtUnknown,
            b'v' | b'c' => MotionType::MtCharWise,
            b'V' | b'l' => MotionType::MtLineWise,
            b'b' | CTRL_V => MotionType::MtBlockWise,
            _ => return fail_cleanup(reg, errmsg, target),
        };
    } else {
        lines = res;
        reg.y_type = MotionType::MtUnknown;
    }

    let lines_len = lines.as_deref().map_or(0, |l| l.lv_len.max(0) as usize);
    reg.y_array = Vec::with_capacity(lines_len);
    reg.y_allocated = true;
    reg.y_width = 0;
    reg.additional_data = None;
    reg.timestamp = 0;

    for li in crate::eval::typval::list_iter_mut(lines.as_deref().and_then(|l| l.first_opt())) {
        if li.li_tv.v_type != VarType::VarString {
            return fail_cleanup(reg, errmsg, target);
        }
        reg.y_array
            .push(cstr_to_string(Some(li.li_tv.vval.v_string_cstr_or_empty())));
    }

    if reg.y_array.last().is_some_and(|last| last.size == 0) {
        if reg.y_type != MotionType::MtCharWise {
            if let Some(mut popped) = reg.y_array.pop() {
                api_clear_string(&mut popped);
            }
            if reg.y_type == MotionType::MtUnknown {
                reg.y_type = MotionType::MtLineWise;
            }
        }
    } else if reg.y_type == MotionType::MtUnknown {
        reg.y_type = MotionType::MtCharWise;
    }

    update_yankreg_width(reg);

    *target = Some(reg);
    true
}

/// Write `reg` to the clipboard provider (`set_clipboard`).
pub fn set_clipboard(mut name: i32, reg: &Yankreg) {
    if mc_replaying() || adjust_clipboard_name(&mut name, false, true).is_none() {
        return;
    }

    let extra = usize::from(reg.y_type != MotionType::MtCharWise);
    let lines = tv_list_alloc((reg.y_array.len() + extra) as isize);

    for &item in &reg.y_array {
        tv_list_append_bytes(&mut *lines, item.as_bytes());
    }

    let regtype: u8 = match reg.y_type {
        MotionType::MtLineWise => {
            tv_list_append_string(&mut *lines, None, 0);
            b'V'
        }
        MotionType::MtCharWise => b'v',
        MotionType::MtBlockWise => {
            tv_list_append_string(&mut *lines, None, 0);
            b'b'
        }
        MotionType::MtUnknown => std::process::abort(),
    };

    let args = tv_list_alloc(3);
    tv_list_append_list(&mut *args, Some(lines));
    tv_list_append_bytes(&mut *args, &[regtype]);
    tv_list_append_bytes(&mut *args, &[name as u8]);

    let _ = eval_call_provider(c"clipboard", c"set", &mut *args, true);
}

/// Defer clipboard updates during batch operations (`start_batch_changes`).
pub fn start_batch_changes() {
    clip_st().batch_change_count += 1;
    if clip_st().batch_change_count > 1 {
        return;
    }
    clip_st().clipboard_delay_update = true;
}

/// End batch operation and flush deferred clipboard update if needed (`end_batch_changes`).
pub fn end_batch_changes() {
    clip_st().batch_change_count -= 1;
    if clip_st().batch_change_count > 0 {
        return;
    }
    clip_st().clipboard_delay_update = false;
    if clip_st().clipboard_needs_update {
        clip_st().clipboard_needs_update = false;
        set_clipboard(0, get_y_previous());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_changes_nesting_toggles_delay_flag() {
        start_batch_changes();
        assert!(clip_st().clipboard_delay_update);
        start_batch_changes();
        end_batch_changes();
        assert!(clip_st().clipboard_delay_update);
        end_batch_changes();
        assert!(!clip_st().clipboard_delay_update);
    }
}
