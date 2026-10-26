//! Decoration provider management and invocation (`src/nvim/decoration_provider.c`).

use crate::api::extmark::describe_ns;
use crate::api::private::helpers::{
    api_clear_error, api_free_array, api_free_object, api_object_to_bool, buffer_obj, error_init,
    error_set, integer_obj, window_obj,
};
use crate::api::private::validate::{Array, ObjectType};
use crate::buffer::buf_T;
use crate::cursor::linenr_T;
use crate::decoration::{NS, decor_check_to_be_deleted};
use crate::globals::*;
use crate::highlight::hl_check_ns;
use crate::log::{LOGLVL_ERR, logmsg};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{api_free_luaref, kRetMulti, kRetNilBool, nlua_call_ref};
use crate::message::msg_schedule_semsg_multiline;
use crate::r#move::validate_botline_win;
use crate::msgpack_rpc::packer::Integer;
use crate::window::win_T;
use std::ffi::{c_char, c_int};
use std::ptr;

pub(crate) const CB_MAX_ERROR: u8 = 3;
pub(crate) const kDecorProviderActive: c_int = 1;
const kDecorProviderWinDisabled: c_int = 2;
const kDecorProviderRedrawDisabled: c_int = 3;
const kDecorProviderDisabled: c_int = 4;

#[repr(C)]
pub struct DecorProvider {
    pub ns_id: NS,
    pub state: c_int,
    pub win_skip_row: c_int,
    pub win_skip_col: c_int,
    pub redraw_start: LuaRef,
    pub redraw_buf: LuaRef,
    pub redraw_win: LuaRef,
    pub redraw_line: LuaRef,
    pub redraw_range: LuaRef,
    pub redraw_end: LuaRef,
    pub hl_def: LuaRef,
    pub spell_nav: LuaRef,
    pub conceal_line: LuaRef,
    pub hl_valid: c_int,
    pub hl_cached: bool,
    pub error_count: u8,
}

static mut decor_providers: Vec<DecorProvider> = Vec::new();

#[inline]
unsafe fn providers_mut() -> &'static mut Vec<DecorProvider> {
    let ptr = &raw mut decor_providers;
    unsafe { &mut *ptr }
}

#[inline]
const fn decor_provider_init(ns_id: NS) -> DecorProvider {
    DecorProvider {
        ns_id,
        state: kDecorProviderDisabled,
        win_skip_row: 0,
        win_skip_col: 0,
        redraw_start: LUA_NOREF,
        redraw_buf: LUA_NOREF,
        redraw_win: LUA_NOREF,
        redraw_line: LUA_NOREF,
        redraw_range: LUA_NOREF,
        redraw_end: LUA_NOREF,
        hl_def: LUA_NOREF,
        spell_nav: LUA_NOREF,
        conceal_line: -1,
        hl_valid: 0,
        hl_cached: false,
        error_count: 0,
    }
}

#[inline]
unsafe fn clear_ref(r: *mut LuaRef) {
    unsafe {
        if *r != LUA_NOREF {
            api_free_luaref(*r);
            *r = LUA_NOREF;
        }
    }
}

unsafe fn decor_provider_error(
    provider: *mut DecorProvider,
    name: *const c_char,
    msg: *const c_char,
) {
    unsafe {
        let ns = describe_ns((*provider).ns_id, c"(UNKNOWN PLUGIN)".as_ptr());
        logmsg!(
            LOGLVL_ERR,
            ptr::null(),
            c"decor_provider_error".as_ptr(),
            31,
            true,
            c"Error in decoration provider \"%s\" (ns=%s):\n%s".as_ptr(),
            name,
            ns,
            msg,
        );
        msg_schedule_semsg_multiline!(
            c"Decoration provider \"%s\" (ns=%s):\n%s".as_ptr(),
            name,
            ns,
            msg,
        );
    }
}

/// Note we pass in a provider index as this function may cause `decor_providers` to be
/// reallocated so we need to be careful with `DecorProvider` pointers (`decor_provider_invoke`).
unsafe fn decor_provider_invoke(
    provider_idx: c_int,
    name: *const c_char,
    ref_: LuaRef,
    args: Array,
    default_true: bool,
    res: *mut Array,
) -> bool {
    unsafe {
        let mut err = error_init();

        textlock += 1;
        let ret = nlua_call_ref(
            ref_,
            name,
            args,
            if !res.is_null() {
                kRetMulti
            } else {
                kRetNilBool
            },
            ptr::null_mut(),
            &raw mut err,
        );
        textlock -= 1;

        let provider = &raw mut providers_mut()[provider_idx as usize];
        if !error_set(&err) {
            (*provider).error_count = 0;
            if !res.is_null() {
                debug_assert!(matches!(ret.obj_type, ObjectType::kObjectTypeArray));
                *res = ret.data.array;
                return true;
            } else if api_object_to_bool(
                ret,
                c"provider %s retval".as_ptr(),
                default_true,
                &raw mut err,
            ) {
                return true;
            }
        }

        if error_set(&err) && (*provider).error_count < CB_MAX_ERROR {
            decor_provider_error(provider, name, err.msg_ptr());
            (*provider).error_count += 1;

            if (*provider).error_count >= CB_MAX_ERROR {
                (*provider).state = kDecorProviderDisabled;
            }
        }

        api_clear_error(&raw mut err);
        api_free_object(ret);
        false
    }
}

pub unsafe fn decor_providers_invoke_spell(
    wp: *mut win_T,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
) {
    unsafe {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != kDecorProviderDisabled && p.spell_nav != LUA_NOREF {
                let spell_nav = p.spell_nav;
                let mut args_buf = [
                    integer_obj((*wp).handle as Integer),
                    integer_obj((*(*wp).w_buffer).b_fnum as Integer),
                    integer_obj(start_row as Integer),
                    integer_obj(start_col as Integer),
                    integer_obj(end_row as Integer),
                    integer_obj(end_col as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 6,
                    capacity: 0,
                };
                decor_provider_invoke(
                    i as c_int,
                    c"spell".as_ptr(),
                    spell_nav,
                    args,
                    true,
                    ptr::null_mut(),
                );
            }
            i += 1;
        }
    }
}

/// Return whether a provider placed any marks in the callback (`decor_providers_invoke_conceal_line`).
pub unsafe fn decor_providers_invoke_conceal_line(wp: *mut win_T, row: c_int) -> bool {
    unsafe {
        let keys = (*(*wp).w_buffer).b_marktree.n_keys;
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != kDecorProviderDisabled && p.conceal_line != LUA_NOREF {
                let conceal_line = p.conceal_line;
                let mut args_buf = [
                    integer_obj((*wp).handle as Integer),
                    integer_obj((*(*wp).w_buffer).b_fnum as Integer),
                    integer_obj(row as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 3,
                    capacity: 0,
                };
                decor_provider_invoke(
                    i as c_int,
                    c"conceal_line".as_ptr(),
                    conceal_line,
                    args,
                    true,
                    ptr::null_mut(),
                );
            }
            i += 1;
        }
        (*(*wp).w_buffer).b_marktree.n_keys > keys
    }
}

/// For each provider invoke the 'start' callback (`decor_providers_start`).
pub unsafe fn decor_providers_start() {
    unsafe {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != kDecorProviderDisabled && p.redraw_start != LUA_NOREF {
                let redraw_start = p.redraw_start;
                let mut args_buf = [integer_obj((display_tick as c_int) as Integer)];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 1,
                    capacity: 0,
                };
                let active = decor_provider_invoke(
                    i as c_int,
                    c"start".as_ptr(),
                    redraw_start,
                    args,
                    true,
                    ptr::null_mut(),
                );
                providers_mut()[i].state = if active {
                    kDecorProviderActive
                } else {
                    kDecorProviderRedrawDisabled
                };
            } else if p.state != kDecorProviderDisabled {
                providers_mut()[i].state = kDecorProviderActive;
            }
            i += 1;
        }
    }
}

/// For each provider run 'win' (`decor_providers_invoke_win`).
pub unsafe fn decor_providers_invoke_win(wp: *mut win_T) {
    unsafe {
        let ds = &raw const decor_state;
        debug_assert!(
            (*ds).current_end == 0 && (*ds).future_begin == (*ds).ranges_i.len() as c_int
        );

        if !providers_mut().is_empty() {
            validate_botline_win(wp);
        }
        let botline: linenr_T = (*wp).w_botline.min((*(*wp).w_buffer).b_ml.ml_line_count);

        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &mut providers_mut()[i];
            if p.state == kDecorProviderWinDisabled {
                p.state = kDecorProviderActive;
            }

            p.win_skip_row = 0;
            p.win_skip_col = 0;

            if p.state == kDecorProviderActive && p.redraw_win != LUA_NOREF {
                let redraw_win = p.redraw_win;
                let mut args_buf = [
                    window_obj((*wp).handle),
                    buffer_obj((*(*wp).w_buffer).b_fnum),
                    integer_obj(((*wp).w_topline - 1) as Integer),
                    integer_obj((botline - 1) as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 4,
                    capacity: 0,
                };
                if !decor_provider_invoke(
                    i as c_int,
                    c"win".as_ptr(),
                    redraw_win,
                    args,
                    true,
                    ptr::null_mut(),
                ) {
                    providers_mut()[i].state = kDecorProviderWinDisabled;
                }
            }
            i += 1;
        }
    }
}

/// For each provider invoke the 'line' callback for a given window row (`decor_providers_invoke_line`).
pub unsafe fn decor_providers_invoke_line(wp: *mut win_T, row: c_int) {
    unsafe {
        decor_state.running_decor_provider = true;
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state == kDecorProviderActive && p.redraw_line != LUA_NOREF {
                let redraw_line = p.redraw_line;
                let mut args_buf = [
                    window_obj((*wp).handle),
                    buffer_obj((*(*wp).w_buffer).b_fnum),
                    integer_obj(row as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 3,
                    capacity: 0,
                };
                if !decor_provider_invoke(
                    i as c_int,
                    c"line".as_ptr(),
                    redraw_line,
                    args,
                    true,
                    ptr::null_mut(),
                ) {
                    providers_mut()[i].state = kDecorProviderWinDisabled;
                }

                hl_check_ns();
            }
            i += 1;
        }
        decor_state.running_decor_provider = false;
    }
}

pub unsafe fn decor_providers_invoke_range(
    wp: *mut win_T,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
) {
    unsafe {
        decor_state.running_decor_provider = true;
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state == kDecorProviderActive && p.redraw_range != LUA_NOREF {
                if p.win_skip_row > end_row
                    || (p.win_skip_row == end_row && p.win_skip_col >= end_col)
                {
                    i += 1;
                    continue;
                }

                let redraw_range = p.redraw_range;
                let mut args_buf = [
                    window_obj((*wp).handle),
                    buffer_obj((*(*wp).w_buffer).b_fnum),
                    integer_obj(start_row as Integer),
                    integer_obj(start_col as Integer),
                    integer_obj(end_row as Integer),
                    integer_obj(end_col as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 6,
                    capacity: 0,
                };
                let mut res = Array::new();
                let status = decor_provider_invoke(
                    i as c_int,
                    c"range".as_ptr(),
                    redraw_range,
                    args,
                    true,
                    &raw mut res,
                );
                let p = &mut providers_mut()[i];

                if !status {
                    p.state = kDecorProviderWinDisabled;
                } else if res.size >= 1 {
                    let first = *res.items;
                    if matches!(first.obj_type, ObjectType::kObjectTypeBoolean) {
                        if !first.data.boolean {
                            p.state = kDecorProviderWinDisabled;
                        }
                    } else if matches!(first.obj_type, ObjectType::kObjectTypeInteger) {
                        let row = first.data.integer;
                        let mut col: Integer = 0;
                        if res.size >= 2 {
                            let second = *res.items.add(1);
                            if matches!(second.obj_type, ObjectType::kObjectTypeInteger) {
                                col = second.data.integer;
                            }
                        }
                        p.win_skip_row = row as c_int;
                        p.win_skip_col = col as c_int;
                    }
                }

                api_free_array(res);

                hl_check_ns();
            }
            i += 1;
        }
        decor_state.running_decor_provider = false;
    }
}

/// For each provider invoke the 'buf' callback for a given buffer (`decor_providers_invoke_buf`).
pub unsafe fn decor_providers_invoke_buf(buf: *mut buf_T) {
    unsafe {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state == kDecorProviderActive && p.redraw_buf != LUA_NOREF {
                let redraw_buf = p.redraw_buf;
                let mut args_buf = [
                    buffer_obj((*buf).b_fnum),
                    integer_obj(display_tick as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 2,
                    capacity: 0,
                };
                decor_provider_invoke(
                    i as c_int,
                    c"buf".as_ptr(),
                    redraw_buf,
                    args,
                    true,
                    ptr::null_mut(),
                );
            }
            i += 1;
        }
    }
}

/// For each provider invoke the 'end' callback (`decor_providers_invoke_end`).
pub unsafe fn decor_providers_invoke_end() {
    unsafe {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != kDecorProviderDisabled && p.redraw_end != LUA_NOREF {
                let redraw_end = p.redraw_end;
                let mut args_buf = [integer_obj((display_tick as c_int) as Integer)];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 1,
                    capacity: 0,
                };
                decor_provider_invoke(
                    i as c_int,
                    c"end".as_ptr(),
                    redraw_end,
                    args,
                    true,
                    ptr::null_mut(),
                );
            }
            i += 1;
        }
        decor_check_to_be_deleted();
    }
}

/// Mark all cached state of per-namespace highlights as invalid (`decor_provider_invalidate_hl`).
pub unsafe fn decor_provider_invalidate_hl() {
    unsafe {
        for p in providers_mut().iter_mut() {
            p.hl_cached = false;
        }

        if ns_hl_active != 0 {
            ns_hl_active = -1;
            hl_check_ns();
        }
    }
}

pub unsafe fn get_decor_provider(ns_id: NS, force: bool) -> *mut DecorProvider {
    unsafe {
        debug_assert!(ns_id > 0);
        let p_vec = providers_mut();
        for p in p_vec.iter_mut() {
            if p.ns_id == ns_id {
                return &raw mut *p;
            }
        }

        if !force {
            return ptr::null_mut();
        }

        p_vec.push(decor_provider_init(ns_id));
        p_vec.last_mut().map_or(ptr::null_mut(), |p| &raw mut *p)
    }
}

pub unsafe fn decor_provider_clear(p: *mut DecorProvider) {
    unsafe {
        if p.is_null() {
            return;
        }
        clear_ref(&raw mut (*p).redraw_start);
        clear_ref(&raw mut (*p).redraw_buf);
        clear_ref(&raw mut (*p).redraw_win);
        clear_ref(&raw mut (*p).redraw_line);
        clear_ref(&raw mut (*p).redraw_range);
        clear_ref(&raw mut (*p).redraw_end);
        clear_ref(&raw mut (*p).spell_nav);
        clear_ref(&raw mut (*p).conceal_line);
        (*p).state = kDecorProviderDisabled;
    }
}

impl Drop for DecorProvider {
    fn drop(&mut self) {
        unsafe {
            decor_provider_clear(self);
        }
    }
}

pub unsafe fn decor_free_all_mem() {
    unsafe {
        decor_providers = Vec::new();
    }
}
