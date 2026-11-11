//! Core editor state loop and mode queries (`src/nvim/state.c`).
#![forbid(unsafe_code)]

use crate::autocmd::{EVENT_MODECHANGED, EVENT_SAFESTATE, apply_autocmds, has_event};
use crate::drawscreen::{K_OPT_VE_FLAG_BLOCK, setcursor, update_screen};
use crate::eval::typval::{tv_dict_add_str, tv_dict_set_keys_readonly};
use crate::eval::{get_v_event, restore_v_event};
use crate::event::r#loop::main_loop_events;
use crate::event::multiqueue::{multiqueue_empty, multiqueue_get};
use crate::ex_getln::{K_EVENT, cmdline_overstrike, get_cmdline_info};
use crate::globals::*;
use crate::hashtab::Hashtab;
use crate::input::{
    check_end_reg_executing, may_sync_undo, safe_vgetc, stuff_empty, using_script, vpeekc,
};
use crate::insexpand::{ctrl_x_mode_not_defined_yet, ins_compl_active};
use crate::keycodes::get_special_key_name;
use crate::log::{LOGLVL_DBG, logmsg, min_log_level};
use crate::option::get_ve_flags;
use crate::os::input::{input_available, input_get, os_breakcheck};
use crate::strings::format_to_buf;
use crate::ui::ui_flush;
use crate::window::WinInfoInfo;
use std::ffi::CStr;

pub const MODE_MAX_LENGTH: usize = 4;

pub const MODE_NORMAL: i32 = 0x01;
pub const MODE_VISUAL: i32 = 0x02;
pub const MODE_OP_PENDING: i32 = 0x04;
pub const MODE_CMDLINE: i32 = 0x08;
pub const MODE_INSERT: i32 = 0x10;
pub const MODE_LANGMAP: i32 = 0x20;
pub const MODE_SELECT: i32 = 0x40;
pub const MODE_TERMINAL: i32 = 0x80;
pub const MAP_ALL_MODES: i32 = 0xff;
pub const REPLACE_FLAG: i32 = 0x100;
pub const MODE_REPLACE: i32 = REPLACE_FLAG | MODE_INSERT;
pub const VREPLACE_FLAG: i32 = 0x200;
pub const MODE_VREPLACE: i32 = REPLACE_FLAG | VREPLACE_FLAG | MODE_INSERT;
pub const MODE_LREPLACE: i32 = REPLACE_FLAG | MODE_LANGMAP;
pub const MODE_NORMAL_BUSY: i32 = 0x1000 | MODE_NORMAL;
pub const MODE_HITRETURN: i32 = 0x2000 | MODE_NORMAL;
pub const MODE_ASKMORE: i32 = 0x3000;
pub const MODE_SETWSIZE: i32 = 0x4000;
pub const MODE_EXTERNCMD: i32 = 0x5000;
pub const MODE_SHOWMATCH: i32 = 0x6000 | MODE_INSERT;

const K_OPT_VE_FLAG_INSERT: u32 = 0x06;

pub trait EditorState {
    fn check(&mut self) -> i32;
    fn execute(&mut self, key: i32) -> i32;
}

use crate::cursor::K_OPT_VE_FLAG_ALL;
use crate::extmark::K_NONE;
use crate::regexp::CTRL_V_CONST;

#[repr(C)]
pub struct SaveVEvent {
    pub sve_did_save: bool,
    pub sve_hashtab: Hashtab,
}

impl SaveVEvent {
    #[inline]
    pub const fn new() -> Self {
        Self {
            sve_did_save: false,
            sve_hashtab: Hashtab::new(),
        }
    }
}

impl Default for SaveVEvent {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

std::thread_local! {
    static WAS_SAFE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn state_enter<S: EditorState>(s: &mut S) {
    loop {
        let check_result = s.check();

        if check_result == 0 {
            break;
        } else if check_result == -1 {
            continue;
        }

        'getkey: loop {
            let key = loop {
                if vpeekc() != 0 || g_st().typebuf.tb_len > 0 {
                    break safe_vgetc();
                } else if !multiqueue_empty(main_loop_events()) {
                    ui_flush();
                    break K_EVENT;
                } else {
                    if g_st().must_redraw != 0
                        && !g_st().need_wait_return
                        && (g_st().state_global & MODE_CMDLINE) == 0
                    {
                        update_screen();
                        setcursor();
                    }
                    ui_flush();
                    input_get(
                        None,
                        -1,
                        g_st().typebuf.tb_change_cnt,
                        Some(main_loop_events()),
                    );
                    if input_available() == 0 && !multiqueue_empty(main_loop_events()) {
                        break K_EVENT;
                    }
                }
            };

            if key == K_EVENT {
                check_end_reg_executing(true);
                may_sync_undo();
            }

            if min_log_level() <= LOGLVL_DBG {
                let special_name;
                let keyname: &CStr = if key == K_EVENT {
                    c"K_EVENT"
                } else {
                    special_name = get_special_key_name(key, g_st().mod_mask);
                    &special_name
                };
                logmsg!(
                    LOGLVL_DBG,
                    None,
                    Some(c"state_enter"),
                    97,
                    true,
                    c"input: %s",
                    keyname,
                );
            }

            let execute_result = s.execute(key);
            if execute_result == 0 {
                return;
            } else if execute_result == -1 {
                continue 'getkey;
            }
            break 'getkey;
        }
    }
}

pub fn state_handle_k_event() {
    loop {
        multiqueue_get(main_loop_events()).dispatch();

        if multiqueue_empty(main_loop_events()) {
            return;
        }

        os_breakcheck();
        if input_available() != 0 || g_st().got_int {
            return;
        }
    }
}

pub fn virtual_active(wp: &mut WinInfoInfo) -> bool {
    if (g_st().state_global & MODE_TERMINAL) != 0 {
        return true;
    }

    let cur_ve_flags = get_ve_flags(wp);

    if cur_ve_flags == K_OPT_VE_FLAG_ALL
        || ((cur_ve_flags & K_OPT_VE_FLAG_INSERT) != 0 && (g_st().state_global & MODE_INSERT) != 0)
    {
        return true;
    }

    if g_st().virtual_op != K_NONE {
        return g_st().virtual_op != 0;
    }
    (cur_ve_flags & K_OPT_VE_FLAG_BLOCK) != 0
        && g_st().visual_global.active
        && g_st().visual_global.mode == CTRL_V_CONST
}

pub fn get_real_state() -> i32 {
    if (g_st().state_global & MODE_NORMAL) != 0 {
        if g_st().visual_global.active {
            if g_st().visual_global.select {
                return MODE_SELECT;
            }
            return MODE_VISUAL;
        } else if g_st().finish_op {
            return MODE_OP_PENDING;
        }
    }
    g_st().state_global
}

pub fn get_mode(buf: &mut [u8; MODE_MAX_LENGTH]) -> usize {
    *buf = [0; MODE_MAX_LENGTH];
    let mut i: usize = 0;

    if g_st().state_global == MODE_HITRETURN
        || g_st().state_global == MODE_ASKMORE
        || g_st().state_global == MODE_SETWSIZE
        || ((g_st().state_global & MODE_CMDLINE) != 0 && get_cmdline_info().one_key.get())
    {
        buf[i] = b'r';
        i += 1;
        if g_st().state_global == MODE_ASKMORE {
            buf[i] = b'm';
            i += 1;
        } else if (g_st().state_global & MODE_CMDLINE) != 0 {
            buf[i] = b'?';
            i += 1;
        }
    } else if g_st().state_global == MODE_EXTERNCMD {
        buf[i] = b'!';
        i += 1;
    } else if (g_st().state_global & MODE_INSERT) != 0 {
        if (g_st().state_global & VREPLACE_FLAG) != 0 {
            buf[i] = b'R';
            i += 1;
            buf[i] = b'v';
            i += 1;
        } else if (g_st().state_global & REPLACE_FLAG) != 0 {
            buf[i] = b'R';
            i += 1;
        } else {
            buf[i] = b'i';
            i += 1;
        }
        if ins_compl_active() {
            buf[i] = b'c';
            i += 1;
        } else if ctrl_x_mode_not_defined_yet() {
            buf[i] = b'x';
            i += 1;
        }
    } else if (g_st().state_global & MODE_CMDLINE) != 0 || g_st().exmode_active {
        buf[i] = b'c';
        i += 1;
        if g_st().exmode_active {
            buf[i] = b'v';
            i += 1;
        }
        if (g_st().state_global & MODE_CMDLINE) != 0 && cmdline_overstrike() {
            buf[i] = b'r';
            i += 1;
        }
    } else if (g_st().state_global & MODE_TERMINAL) != 0 {
        buf[i] = b't';
        i += 1;
    } else if g_st().visual_global.active {
        if g_st().visual_global.select {
            buf[i] = (g_st().visual_global.mode + (b's' as i32) - (b'v' as i32)) as u8;
            i += 1;
        } else {
            buf[i] = g_st().visual_global.mode as u8;
            i += 1;
            if g_st().visual_global.restart_select != 0 {
                buf[i] = b's';
                i += 1;
            }
        }
    } else {
        buf[i] = b'n';
        i += 1;
        if g_st().finish_op {
            buf[i] = b'o';
            i += 1;
            if g_st().motion_force != 0 {
                buf[i] = g_st().motion_force as u8;
                i += 1;
            }
        } else if !curbuf_mut().terminal.is_null() {
            buf[i] = b't';
            i += 1;
            if g_st().restart_edit == b'I' as i32 {
                buf[i] = b'T';
                i += 1;
            }
        } else if g_st().restart_edit == b'I' as i32
            || g_st().restart_edit == b'R' as i32
            || g_st().restart_edit == b'V' as i32
        {
            buf[i] = b'i';
            i += 1;
            buf[i] = g_st().restart_edit as u8;
            i += 1;
        }
    }

    buf[i] = 0;
    i
}

pub fn may_trigger_modechanged() {
    if !has_event(EVENT_MODECHANGED) || g_st().got_int {
        return;
    }

    let mut curr_mode: [u8; MODE_MAX_LENGTH] = [0; MODE_MAX_LENGTH];
    let mut pattern_buf: [u8; 2 * MODE_MAX_LENGTH] = [0; 2 * MODE_MAX_LENGTH];

    get_mode(&mut curr_mode);
    if curr_mode == g_st().last_mode {
        return;
    }

    let curr_cstr = CStr::from_bytes_until_nul(&curr_mode).unwrap_or(c"");
    let old_mode = g_st().last_mode;
    let old_cstr = CStr::from_bytes_until_nul(&old_mode).unwrap_or(c"");

    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let v_event = get_v_event(&mut save_v_event);
    tv_dict_add_str(&v_event, c"new_mode", Some(curr_cstr));
    tv_dict_add_str(&v_event, c"old_mode", Some(old_cstr));
    tv_dict_set_keys_readonly(&v_event);

    format_to_buf!(&mut pattern_buf[..], c"%s:%s", old_cstr, curr_cstr,);

    apply_autocmds(
        EVENT_MODECHANGED,
        Some(crate::memory::bytes_as_cstr(&pattern_buf)),
        None,
        false,
        curbuf_opt_mut(),
    );
    g_st().last_mode = curr_mode;

    restore_v_event(&v_event, &mut save_v_event);
}

#[inline]
fn is_safe_now() -> bool {
    stuff_empty()
        && g_st().typebuf.tb_len == 0
        && using_script() == 0
        && g_st().global_busy == 0
        && !g_st().debug_mode
}

pub fn may_trigger_safestate(safe: bool) {
    let is_safe = safe && is_safe_now();

    if WAS_SAFE.get() != is_safe && min_log_level() <= LOGLVL_DBG {
        let msg: &CStr = if is_safe {
            c"SafeState: Start triggering"
        } else {
            c"SafeState: Stop triggering"
        };
        logmsg!(
            LOGLVL_DBG,
            None,
            Some(c"may_trigger_safestate"),
            307,
            true,
            c"%s",
            msg,
        );
    }
    if is_safe {
        apply_autocmds(EVENT_SAFESTATE, None, None, false, curbuf_opt_mut());
    }
    WAS_SAFE.set(is_safe);
}

pub fn state_no_longer_safe(reason: Option<&CStr>) {
    if WAS_SAFE.get()
        && let Some(reason) = reason
        && min_log_level() <= LOGLVL_DBG
    {
        logmsg!(
            LOGLVL_DBG,
            None,
            Some(c"state_no_longer_safe"),
            320,
            true,
            c"SafeState reset: %s",
            reason,
        );
    }
    WAS_SAFE.set(false);
}

pub fn get_was_safe_state() -> bool {
    WAS_SAFE.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_state_reset_clears_was_safe() {
        state_no_longer_safe(None);
        assert!(!get_was_safe_state());
    }
}
