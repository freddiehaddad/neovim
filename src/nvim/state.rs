//! Core editor state loop and mode queries (`src/nvim/state.c`).

use crate::autocmd::{EVENT_MODECHANGED, EVENT_SAFESTATE, apply_autocmds, has_event};
use crate::drawscreen::{kOptVeFlagBlock, setcursor, update_screen};
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
use crate::log::{LOGLVL_DBG, log_st, logmsg};
use crate::option::get_ve_flags;
use crate::os::input::{input_available, input_get, os_breakcheck};
use crate::strings::{vim_snprintf};
use crate::ui::ui_flush;
use crate::window::WinInfoInfo;
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

pub const MODE_MAX_LENGTH: usize = 4;

pub const MODE_NORMAL: c_int = 0x01;
pub const MODE_VISUAL: c_int = 0x02;
pub const MODE_OP_PENDING: c_int = 0x04;
pub const MODE_CMDLINE: c_int = 0x08;
pub const MODE_INSERT: c_int = 0x10;
pub const MODE_LANGMAP: c_int = 0x20;
pub const MODE_SELECT: c_int = 0x40;
pub const MODE_TERMINAL: c_int = 0x80;
pub const MAP_ALL_MODES: c_int = 0xff;
pub const REPLACE_FLAG: c_int = 0x100;
pub const MODE_REPLACE: c_int = REPLACE_FLAG | MODE_INSERT;
pub const VREPLACE_FLAG: c_int = 0x200;
pub const MODE_VREPLACE: c_int = REPLACE_FLAG | VREPLACE_FLAG | MODE_INSERT;
pub const MODE_LREPLACE: c_int = REPLACE_FLAG | MODE_LANGMAP;
pub const MODE_NORMAL_BUSY: c_int = 0x1000 | MODE_NORMAL;
pub const MODE_HITRETURN: c_int = 0x2000 | MODE_NORMAL;
pub const MODE_ASKMORE: c_int = 0x3000;
pub const MODE_SETWSIZE: c_int = 0x4000;
pub const MODE_EXTERNCMD: c_int = 0x5000;
pub const MODE_SHOWMATCH: c_int = 0x6000 | MODE_INSERT;

const kOptVeFlagInsert: c_uint = 0x06;

pub trait EditorState {
    fn check(&mut self) -> c_int;
    fn execute(&mut self, key: c_int) -> c_int;
}

use crate::cursor::kOptVeFlagAll;
use crate::extmark::kNone;
use crate::regexp::Ctrl_V;

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

struct StateModuleState {
    was_safe: bool,
}

impl StateModuleState {
    const fn new() -> Self {
        Self {
            was_safe: false,
        }
    }
}

static STATE_MOD_STATE: crate::globals::GlobalState<StateModuleState> = crate::globals::GlobalState::new(StateModuleState::new());

#[inline]
fn state_mod_st() -> &'static mut StateModuleState {
    unsafe { &mut *STATE_MOD_STATE.as_ptr() }
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
                    let events = main_loop_events();
                    if vpeekc() != 0 || g_st().typebuf.tb_len > 0 {
                        break safe_vgetc();
                    } else if !multiqueue_empty(unsafe { &*events }) {
                        ui_flush();
                        break K_EVENT;
                    } else {
                        if g_st().must_redraw != 0 && !g_st().need_wait_return && (g_st().State & MODE_CMDLINE) == 0 {
                            update_screen();
                            setcursor();
                        }
                        ui_flush();
                        input_get(ptr::null_mut(), 0, -1, g_st().typebuf.tb_change_cnt, events);
                        if input_available() == 0 && !multiqueue_empty(unsafe { &*main_loop_events() }) {
                            break K_EVENT;
                        }
                    }
                };

                if key == K_EVENT {
                    check_end_reg_executing(true);
                    may_sync_undo();
                }

                if log_st().g_min_log_level <= LOGLVL_DBG {
                    let keyname = if key == K_EVENT {
                        c"K_EVENT".as_ptr()
                    } else {
                        get_special_key_name(key, g_st().mod_mask).cast_const()
                    };
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"state_enter",
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
    unsafe {
        loop {
            let events = main_loop_events();
            multiqueue_get(&mut *events).dispatch();

            if multiqueue_empty( &*main_loop_events()) {
                return;
            }

            os_breakcheck();
            if input_available() != 0 || g_st().got_int {
                return;
            }
        }
    }
}

pub fn virtual_active(wp: &mut WinInfoInfo) -> bool {
    if (g_st().State & MODE_TERMINAL) != 0 {
            return true;
        }

        let cur_ve_flags = get_ve_flags( wp);

        if cur_ve_flags == kOptVeFlagAll
            || ((cur_ve_flags & kOptVeFlagInsert) != 0 && (g_st().State & MODE_INSERT) != 0)
        {
            return true;
        }

        if g_st().virtual_op != kNone {
            return g_st().virtual_op != 0;
        }
        (cur_ve_flags & kOptVeFlagBlock) != 0 && g_st().Visual.active && g_st().Visual.mode == Ctrl_V
}

pub fn get_real_state() -> c_int {
    if (g_st().State & MODE_NORMAL) != 0 {
            if g_st().Visual.active {
                if g_st().Visual.select {
                    return MODE_SELECT;
                }
                return MODE_VISUAL;
            } else if g_st().finish_op {
                return MODE_OP_PENDING;
            }
        }
        g_st().State
}

pub fn get_mode(buf: *mut c_char) {
    unsafe {
        let mut i: usize = 0;

        if g_st().State == MODE_HITRETURN
            || g_st().State == MODE_ASKMORE
            || g_st().State == MODE_SETWSIZE
            || ((g_st().State & MODE_CMDLINE) != 0 && (*get_cmdline_info()).one_key)
        {
            *buf.add(i) = b'r' as c_char;
            i += 1;
            if g_st().State == MODE_ASKMORE {
                *buf.add(i) = b'm' as c_char;
                i += 1;
            } else if (g_st().State & MODE_CMDLINE) != 0 {
                *buf.add(i) = b'?' as c_char;
                i += 1;
            }
        } else if g_st().State == MODE_EXTERNCMD {
            *buf.add(i) = b'!' as c_char;
            i += 1;
        } else if (g_st().State & MODE_INSERT) != 0 {
            if (g_st().State & VREPLACE_FLAG) != 0 {
                *buf.add(i) = b'R' as c_char;
                i += 1;
                *buf.add(i) = b'v' as c_char;
                i += 1;
            } else if (g_st().State & REPLACE_FLAG) != 0 {
                *buf.add(i) = b'R' as c_char;
                i += 1;
            } else {
                *buf.add(i) = b'i' as c_char;
                i += 1;
            }
            if ins_compl_active() {
                *buf.add(i) = b'c' as c_char;
                i += 1;
            } else if ctrl_x_mode_not_defined_yet() {
                *buf.add(i) = b'x' as c_char;
                i += 1;
            }
        } else if (g_st().State & MODE_CMDLINE) != 0 || g_st().exmode_active {
            *buf.add(i) = b'c' as c_char;
            i += 1;
            if g_st().exmode_active {
                *buf.add(i) = b'v' as c_char;
                i += 1;
            }
            if (g_st().State & MODE_CMDLINE) != 0 && cmdline_overstrike() {
                *buf.add(i) = b'r' as c_char;
                i += 1;
            }
        } else if (g_st().State & MODE_TERMINAL) != 0 {
            *buf.add(i) = b't' as c_char;
            i += 1;
        } else if g_st().Visual.active {
            if g_st().Visual.select {
                *buf.add(i) = (g_st().Visual.mode + (b's' as c_int) - (b'v' as c_int)) as c_char;
                i += 1;
            } else {
                *buf.add(i) = g_st().Visual.mode as c_char;
                i += 1;
                if g_st().Visual.restart_select != 0 {
                    *buf.add(i) = b's' as c_char;
                    i += 1;
                }
            }
        } else {
            *buf.add(i) = b'n' as c_char;
            i += 1;
            if g_st().finish_op {
                *buf.add(i) = b'o' as c_char;
                i += 1;
                *buf.add(i) = g_st().motion_force as c_char;
                i += 1;
            } else if !curbuf_mut().terminal.is_null() {
                *buf.add(i) = b't' as c_char;
                i += 1;
                if g_st().restart_edit == b'I' as c_int {
                    *buf.add(i) = b'T' as c_char;
                    i += 1;
                }
            } else if g_st().restart_edit == b'I' as c_int
                || g_st().restart_edit == b'R' as c_int
                || g_st().restart_edit == b'V' as c_int
            {
                *buf.add(i) = b'i' as c_char;
                i += 1;
                *buf.add(i) = g_st().restart_edit as c_char;
                i += 1;
            }
        }

        *buf.add(i) = 0;
    }
}

pub fn may_trigger_modechanged() {
    unsafe {
        if !has_event(EVENT_MODECHANGED) || g_st().got_int {
            return;
        }

        let mut curr_mode: [c_char; MODE_MAX_LENGTH] = [0; MODE_MAX_LENGTH];
        let mut pattern_buf: [c_char; 2 * MODE_MAX_LENGTH] = [0; 2 * MODE_MAX_LENGTH];

        get_mode(curr_mode.as_mut_ptr());
        if crate::memory::cstr_as_bytes(curr_mode.as_ptr()) == crate::memory::cstr_as_bytes(ptr::addr_of!(g_st().last_mode).cast()) {
            return;
        }

        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);
        tv_dict_add_str(&mut *v_event, c"new_mode", curr_mode.as_ptr());
        tv_dict_add_str(
            &mut *v_event,
            c"old_mode",
            ptr::addr_of!(g_st().last_mode).cast(),
        );
        tv_dict_set_keys_readonly(&mut *v_event);

        vim_snprintf!(
            pattern_buf.as_mut_ptr(),
            pattern_buf.len(),
            c"%s:%s",
            ptr::addr_of!(g_st().last_mode).cast::<c_char>(),
            curr_mode.as_ptr(),
        );

        apply_autocmds(
            EVENT_MODECHANGED,
            pattern_buf.as_mut_ptr(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        crate::memory::cstr_cpy(ptr::addr_of_mut!(g_st().last_mode).cast(), curr_mode.as_ptr());

        restore_v_event(&mut *v_event, &mut save_v_event);
    }
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

        if state_mod_st().was_safe != is_safe && log_st().g_min_log_level <= LOGLVL_DBG {
            let msg: &CStr = if is_safe {
                c"SafeState: Start triggering"
            } else {
                c"SafeState: Stop triggering"
            };
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"may_trigger_safestate",
                307,
                true,
                c"%s",
                msg.as_ptr(),
            );
        }
        if is_safe {
            apply_autocmds(
                EVENT_SAFESTATE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
        state_mod_st().was_safe = is_safe;
}

pub fn state_no_longer_safe(reason: impl crate::strings::IntoCCharPtr) {
    let reason = reason.into_c_char_ptr();
    if state_mod_st().was_safe && !reason.is_null() && log_st().g_min_log_level <= LOGLVL_DBG {
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"state_no_longer_safe",
                320,
                true,
                c"SafeState reset: %s",
                reason,
            );
        }
        state_mod_st().was_safe = false;
}

pub fn get_was_safe_state() -> bool {
    state_mod_st().was_safe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_state_reset_clears_was_safe() {
        state_no_longer_safe(ptr::null());
            assert!(!get_was_safe_state());
    }
}
