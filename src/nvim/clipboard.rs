//! Clipboard register handling (`src/nvim/clipboard.c`).
#![forbid(unsafe_code)]

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
    EMPTY_YANKREG, MotionType, PLUS_REGISTER, RegIdx, STAR_REGISTER, get_y_previous,
    get_y_register, update_yankreg_width,
};

use std::ffi::CStr;

const K_OPT_CB_FLAG_UNNAMED: u32 = 0x01;
const K_OPT_CB_FLAG_UNNAMEDPLUS: u32 = 0x02;

const CTRL_V: u8 = 0x16;

struct ClipboardModuleState {
    batch_change_count: std::cell::Cell<i32>,
    clipboard_delay_update: std::cell::Cell<bool>,
    clipboard_needs_update: std::cell::Cell<bool>,
    clipboard_didwarn: std::cell::Cell<bool>,
}

impl ClipboardModuleState {
    const fn new() -> Self {
        Self {
            batch_change_count: std::cell::Cell::new(0),
            clipboard_delay_update: std::cell::Cell::new(false),
            clipboard_needs_update: std::cell::Cell::new(false),
            clipboard_didwarn: std::cell::Cell::new(false),
        }
    }
}

std::thread_local! {
    static CLIP_STATE: ClipboardModuleState = const { ClipboardModuleState::new() };
}

const MSG_NO_CLIP: &CStr = c"clipboard: No provider. Try \":checkhealth\" or \":h clipboard\".";

/// Determine if register `*name` should be used as a clipboard (`adjust_clipboard_name`).
pub fn adjust_clipboard_name(name: &mut i32, quiet: bool, writing: bool) -> Option<RegIdx> {
    let reg_char = *name;
    let flags = opt_vars().cb_flags.get();
    let explicit_cb_reg = reg_char == b'*' as i32 || reg_char == b'+' as i32;
    let implicit_cb_reg =
        reg_char == 0 && (flags & (K_OPT_CB_FLAG_UNNAMED | K_OPT_CB_FLAG_UNNAMEDPLUS)) != 0;
    if !explicit_cb_reg && !implicit_cb_reg {
        return None;
    }

    if !eval_has_provider(b"clipboard", false) {
        let should_warn = CLIP_STATE.with(|st| {
            st.batch_change_count.get() <= 1
                && !quiet
                && (!st.clipboard_didwarn.get() || (explicit_cb_reg && redirecting() == 0))
        });
        if should_warn {
            CLIP_STATE.with(|st| st.clipboard_didwarn.set(true));
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
            CLIP_STATE.with(|st| st.clipboard_needs_update.set(false));
        }
        Some(target)
    } else {
        let (delay_update, needs_update) = CLIP_STATE.with(|st| {
            (
                st.clipboard_delay_update.get(),
                st.clipboard_needs_update.get(),
            )
        });
        if writing && delay_update {
            CLIP_STATE.with(|st| st.clipboard_needs_update.set(true));
            return None;
        } else if !writing && needs_update {
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
pub fn get_clipboard(mut name: i32, target: &mut Option<RegIdx>, quiet: bool) -> bool {
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

    reg.with_mut(|r| *r = EMPTY_YANKREG);

    let args = tv_list_alloc(1);
    tv_list_append_bytes(&args, &[name as u8]);

    let result = eval_call_provider(c"clipboard", c"get", &args, false);

    let fail_cleanup = |reg: RegIdx, errmsg: bool, target: &mut Option<RegIdx>| {
        reg.with_mut(|r| *r = EMPTY_YANKREG);
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
    let res_len = res.map_or(0, |l| l.lv_len.get());
    let lines_list: crate::eval::typval::ListId;
    let lines: Option<&TvList>;
    let mut y_type;

    if res_len == 2
        && let Some(first_li) = res.and_then(TvList::first_opt)
        && first_li.tv().v_type == VarType::VarList
    {
        lines_list = first_li.tv().vval.v_list();
        lines = lines_list.opt();

        let Some(last_li) = res.and_then(TvList::last_opt) else {
            return fail_cleanup(reg, errmsg, target);
        };
        let last_tv = last_li.tv();
        if last_tv.v_type != VarType::VarString {
            return fail_cleanup(reg, errmsg, target);
        }
        let Some(regtype) = last_tv.vval.v_string_cstr() else {
            return fail_cleanup(reg, errmsg, target);
        };
        let regtype_bytes = regtype.to_bytes();
        if regtype_bytes.len() > 1 {
            return fail_cleanup(reg, errmsg, target);
        }
        y_type = match regtype_bytes.first().copied().unwrap_or(0) {
            0 => MotionType::MtUnknown,
            b'v' | b'c' => MotionType::MtCharWise,
            b'V' | b'l' => MotionType::MtLineWise,
            b'b' | CTRL_V => MotionType::MtBlockWise,
            _ => return fail_cleanup(reg, errmsg, target),
        };
        drop(last_tv);
    } else {
        lines = res;
        y_type = MotionType::MtUnknown;
    }

    let lines_len = lines.map_or(0, |l| l.lv_len.get().max(0) as usize);
    let mut y_array = Vec::with_capacity(lines_len);

    for li in crate::eval::typval::list_iter_ptr(lines.and_then(|l| l.first_opt())) {
        let li_tv = li.tv();
        if li_tv.v_type != VarType::VarString {
            for mut s in y_array {
                api_clear_string(&mut s);
            }
            return fail_cleanup(reg, errmsg, target);
        }
        y_array.push(cstr_to_string(Some(li_tv.vval.v_string_cstr_or_empty())));
    }

    if y_array.last().is_some_and(|last| last.size == 0) {
        if y_type != MotionType::MtCharWise {
            if let Some(mut popped) = y_array.pop() {
                api_clear_string(&mut popped);
            }
            if y_type == MotionType::MtUnknown {
                y_type = MotionType::MtLineWise;
            }
        }
    } else if y_type == MotionType::MtUnknown {
        y_type = MotionType::MtCharWise;
    }

    reg.with_mut(|r| {
        r.y_type = y_type;
        r.y_array = y_array;
        r.y_allocated = true;
        r.y_width = 0;
        r.additional_data = None;
        r.timestamp = 0;
        update_yankreg_width(r);
    });

    *target = Some(reg);
    true
}

/// Write `reg` to the clipboard provider (`set_clipboard`).
pub fn set_clipboard(mut name: i32, reg_idx: RegIdx) {
    if mc_replaying() || adjust_clipboard_name(&mut name, false, true).is_none() {
        return;
    }

    let (lines, regtype) = reg_idx.with(|reg| {
        let extra = usize::from(reg.y_type != MotionType::MtCharWise);
        let lines = tv_list_alloc((reg.y_array.len() + extra) as isize);

        for item in &reg.y_array {
            tv_list_append_bytes(&lines, item.as_bytes());
        }

        let regtype: u8 = match reg.y_type {
            MotionType::MtLineWise => {
                tv_list_append_string(&lines, None, 0);
                b'V'
            }
            MotionType::MtCharWise => b'v',
            MotionType::MtBlockWise => {
                tv_list_append_string(&lines, None, 0);
                b'b'
            }
            MotionType::MtUnknown => std::process::abort(),
        };
        (lines, regtype)
    });

    let args = tv_list_alloc(3);
    tv_list_append_list(&args, Some(&lines));
    tv_list_append_bytes(&args, &[regtype]);
    tv_list_append_bytes(&args, &[name as u8]);

    let _ = eval_call_provider(c"clipboard", c"set", &args, true);
}

/// Defer clipboard updates during batch operations (`start_batch_changes`).
pub fn start_batch_changes() {
    CLIP_STATE.with(|st| {
        let c = st.batch_change_count.get() + 1;
        st.batch_change_count.set(c);
        if c <= 1 {
            st.clipboard_delay_update.set(true);
        }
    });
}

/// End batch operation and flush deferred clipboard update if needed (`end_batch_changes`).
pub fn end_batch_changes() {
    let flush = CLIP_STATE.with(|st| {
        let c = st.batch_change_count.get() - 1;
        st.batch_change_count.set(c);
        if c > 0 {
            return false;
        }
        st.clipboard_delay_update.set(false);
        if st.clipboard_needs_update.get() {
            st.clipboard_needs_update.set(false);
            true
        } else {
            false
        }
    });
    if flush {
        set_clipboard(0, get_y_previous());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_changes_nesting_toggles_delay_flag() {
        start_batch_changes();
        assert!(CLIP_STATE.with(|st| st.clipboard_delay_update.get()));
        start_batch_changes();
        end_batch_changes();
        assert!(CLIP_STATE.with(|st| st.clipboard_delay_update.get()));
        end_batch_changes();
        assert!(!CLIP_STATE.with(|st| st.clipboard_delay_update.get()));
    }
}
