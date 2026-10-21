//! Core editor state loop and mode queries (`src/nvim/state.c`).

use crate::autocmd::{EVENT_MODECHANGED, EVENT_SAFESTATE, apply_autocmds, has_event};
use crate::drawscreen::{kOptVeFlagBlock, setcursor, update_screen};
use crate::eval::typval::{tv_dict_add_str, tv_dict_set_keys_readonly};
use crate::eval::{get_v_event, restore_v_event};
use crate::event::r#loop::main_loop_events;
use crate::event::multiqueue::{Event, multiqueue_empty, multiqueue_get};
use crate::ex_getln::{K_EVENT, cmdline_overstrike, get_cmdline_info};
use crate::globals::*;
use crate::hashtab::hashtab_T;
use crate::input::{
    check_end_reg_executing, may_sync_undo, safe_vgetc, stuff_empty, using_script, vpeekc,
};
use crate::insexpand::{ctrl_x_mode_not_defined_yet, ins_compl_active};
use crate::keycodes::get_special_key_name;
use crate::log::{LOGLVL_DBG, g_min_log_level, logmsg};
use crate::option::get_ve_flags;
use crate::os::input::{input_available, input_get, os_breakcheck};
use crate::strings::{strcmp, strcpy, vim_snprintf};
use crate::ui::ui_flush;
use crate::window::win_T;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
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

pub type state_check_callback = Option<unsafe extern "C" fn(state: *mut VimState) -> c_int>;
pub type state_execute_callback =
    Option<unsafe extern "C" fn(state: *mut VimState, key: c_int) -> c_int>;

#[repr(C)]
pub struct VimState {
    pub check: state_check_callback,
    pub execute: state_execute_callback,
}

use crate::cursor::kOptVeFlagAll;
use crate::extmark::kNone;
use crate::regexp::Ctrl_V;

#[repr(C)]
pub struct save_v_event_T {
    pub sve_did_save: bool,
    pub sve_hashtab: hashtab_T,
}

static mut was_safe: bool = false;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_enter(s: *mut VimState) {
    unsafe {
        loop {
            let check_result = if let Some(check_fn) = (*s).check {
                check_fn(s)
            } else {
                1
            };

            if check_result == 0 {
                break;
            } else if check_result == -1 {
                continue;
            }

            'getkey: loop {
                let key = loop {
                    let events = main_loop_events();
                    if vpeekc() != 0 || typebuf.tb_len > 0 {
                        break safe_vgetc();
                    } else if !multiqueue_empty(events) {
                        ui_flush();
                        break K_EVENT;
                    } else {
                        if must_redraw != 0 && !need_wait_return && (State & MODE_CMDLINE) == 0 {
                            update_screen();
                            setcursor();
                        }
                        ui_flush();
                        input_get(ptr::null_mut(), 0, -1, typebuf.tb_change_cnt, events);
                        if input_available() == 0 && !multiqueue_empty(main_loop_events()) {
                            break K_EVENT;
                        }
                    }
                };

                if key == K_EVENT {
                    check_end_reg_executing(true);
                    may_sync_undo();
                }

                if g_min_log_level <= LOGLVL_DBG {
                    let keyname = if key == K_EVENT {
                        c"K_EVENT".as_ptr()
                    } else {
                        get_special_key_name(key, mod_mask).cast_const()
                    };
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"state_enter".as_ptr(),
                        97,
                        true,
                        c"input: %s".as_ptr(),
                        keyname,
                    );
                }

                let execute_fn = (*s).execute.unwrap_unchecked();
                let execute_result = execute_fn(s, key);
                if execute_result == 0 {
                    return;
                } else if execute_result == -1 {
                    continue 'getkey;
                }
                break 'getkey;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_handle_k_event() {
    unsafe {
        loop {
            let events = main_loop_events();
            let mut event: Event = multiqueue_get(events);
            if let Some(handler) = event.handler {
                handler(event.argv.as_mut_ptr().cast::<*mut c_void>());
            }

            if multiqueue_empty(main_loop_events()) {
                return;
            }

            os_breakcheck();
            if input_available() != 0 || got_int {
                return;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn virtual_active(wp: *mut win_T) -> bool {
    unsafe {
        if (State & MODE_TERMINAL) != 0 {
            return true;
        }

        let cur_ve_flags = get_ve_flags(wp);

        if cur_ve_flags == kOptVeFlagAll
            || ((cur_ve_flags & kOptVeFlagInsert) != 0 && (State & MODE_INSERT) != 0)
        {
            return true;
        }

        if virtual_op != kNone {
            return virtual_op != 0;
        }
        (cur_ve_flags & kOptVeFlagBlock) != 0 && Visual.active && Visual.mode == Ctrl_V
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_real_state() -> c_int {
    unsafe {
        if (State & MODE_NORMAL) != 0 {
            if Visual.active {
                if Visual.select {
                    return MODE_SELECT;
                }
                return MODE_VISUAL;
            } else if finish_op {
                return MODE_OP_PENDING;
            }
        }
        State
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_mode(buf: *mut c_char) {
    unsafe {
        let mut i: usize = 0;

        if State == MODE_HITRETURN
            || State == MODE_ASKMORE
            || State == MODE_SETWSIZE
            || ((State & MODE_CMDLINE) != 0 && (*get_cmdline_info()).one_key)
        {
            *buf.add(i) = b'r' as c_char;
            i += 1;
            if State == MODE_ASKMORE {
                *buf.add(i) = b'm' as c_char;
                i += 1;
            } else if (State & MODE_CMDLINE) != 0 {
                *buf.add(i) = b'?' as c_char;
                i += 1;
            }
        } else if State == MODE_EXTERNCMD {
            *buf.add(i) = b'!' as c_char;
            i += 1;
        } else if (State & MODE_INSERT) != 0 {
            if (State & VREPLACE_FLAG) != 0 {
                *buf.add(i) = b'R' as c_char;
                i += 1;
                *buf.add(i) = b'v' as c_char;
                i += 1;
            } else if (State & REPLACE_FLAG) != 0 {
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
        } else if (State & MODE_CMDLINE) != 0 || exmode_active {
            *buf.add(i) = b'c' as c_char;
            i += 1;
            if exmode_active {
                *buf.add(i) = b'v' as c_char;
                i += 1;
            }
            if (State & MODE_CMDLINE) != 0 && cmdline_overstrike() {
                *buf.add(i) = b'r' as c_char;
                i += 1;
            }
        } else if (State & MODE_TERMINAL) != 0 {
            *buf.add(i) = b't' as c_char;
            i += 1;
        } else if Visual.active {
            if Visual.select {
                *buf.add(i) = (Visual.mode + (b's' as c_int) - (b'v' as c_int)) as c_char;
                i += 1;
            } else {
                *buf.add(i) = Visual.mode as c_char;
                i += 1;
                if Visual.restart_select != 0 {
                    *buf.add(i) = b's' as c_char;
                    i += 1;
                }
            }
        } else {
            *buf.add(i) = b'n' as c_char;
            i += 1;
            if finish_op {
                *buf.add(i) = b'o' as c_char;
                i += 1;
                *buf.add(i) = motion_force as c_char;
                i += 1;
            } else if !(*curbuf).terminal.is_null() {
                *buf.add(i) = b't' as c_char;
                i += 1;
                if restart_edit == b'I' as c_int {
                    *buf.add(i) = b'T' as c_char;
                    i += 1;
                }
            } else if restart_edit == b'I' as c_int
                || restart_edit == b'R' as c_int
                || restart_edit == b'V' as c_int
            {
                *buf.add(i) = b'i' as c_char;
                i += 1;
                *buf.add(i) = restart_edit as c_char;
                i += 1;
            }
        }

        *buf.add(i) = 0;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn may_trigger_modechanged() {
    unsafe {
        if !has_event(EVENT_MODECHANGED) || got_int {
            return;
        }

        let mut curr_mode: [c_char; MODE_MAX_LENGTH] = [0; MODE_MAX_LENGTH];
        let mut pattern_buf: [c_char; 2 * MODE_MAX_LENGTH] = [0; 2 * MODE_MAX_LENGTH];

        get_mode(curr_mode.as_mut_ptr());
        if strcmp(curr_mode.as_ptr(), ptr::addr_of!(last_mode).cast()) == 0 {
            return;
        }

        let mut save_v_event: save_v_event_T = std::mem::zeroed();
        let v_event = get_v_event(&mut save_v_event);
        tv_dict_add_str(v_event, c"new_mode".as_ptr(), 8, curr_mode.as_ptr());
        tv_dict_add_str(
            v_event,
            c"old_mode".as_ptr(),
            8,
            ptr::addr_of!(last_mode).cast(),
        );
        tv_dict_set_keys_readonly(v_event);

        vim_snprintf!(
            pattern_buf.as_mut_ptr(),
            pattern_buf.len(),
            c"%s:%s".as_ptr(),
            ptr::addr_of!(last_mode).cast::<c_char>(),
            curr_mode.as_ptr(),
        );

        apply_autocmds(
            EVENT_MODECHANGED,
            pattern_buf.as_mut_ptr(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        strcpy(ptr::addr_of_mut!(last_mode).cast(), curr_mode.as_ptr());

        restore_v_event(v_event, &mut save_v_event);
    }
}

#[inline]
unsafe fn is_safe_now() -> bool {
    unsafe {
        stuff_empty()
            && typebuf.tb_len == 0
            && using_script() == 0
            && global_busy == 0
            && !debug_mode
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn may_trigger_safestate(safe: bool) {
    unsafe {
        let is_safe = safe && is_safe_now();

        if was_safe != is_safe && g_min_log_level <= LOGLVL_DBG {
            let msg: &CStr = if is_safe {
                c"SafeState: Start triggering"
            } else {
                c"SafeState: Stop triggering"
            };
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"may_trigger_safestate".as_ptr(),
                307,
                true,
                c"%s".as_ptr(),
                msg.as_ptr(),
            );
        }
        if is_safe {
            apply_autocmds(
                EVENT_SAFESTATE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }
        was_safe = is_safe;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn state_no_longer_safe(reason: *const c_char) {
    unsafe {
        if was_safe && !reason.is_null() && g_min_log_level <= LOGLVL_DBG {
            logmsg!(
                LOGLVL_DBG,
                ptr::null(),
                c"state_no_longer_safe".as_ptr(),
                320,
                true,
                c"SafeState reset: %s".as_ptr(),
                reason,
            );
        }
        was_safe = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_was_safe_state() -> bool {
    unsafe { was_safe }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_state_reset_clears_was_safe() {
        unsafe {
            state_no_longer_safe(ptr::null());
            assert!(!get_was_safe_state());
        }
    }
}
