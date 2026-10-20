//! Decoration provider management and invocation (`src/nvim/decoration_provider.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::extmark::{DecorProvider, NS, describe_ns, linenr_T};
use crate::api::private::helpers::{
    api_clear_error, api_free_array, api_free_object, api_object_to_bool, buffer_obj, error_init,
    integer_obj, window_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, ObjectType};
use crate::api::tabpage::win_T;
use crate::decoration::decor_check_to_be_deleted;
use crate::eval::deprecated::buf_T;
use crate::globals::*;
use crate::highlight::hl_check_ns;
use crate::kvec::KVec;
use crate::log::logmsg;
use crate::lua::converter::LuaRef;
use crate::lua::executor::{api_free_luaref, nlua_call_ref};
use crate::memory::{xfree, xrealloc};
use crate::message::msg_schedule_semsg_multiline;
use crate::r#move::validate_botline_win;
use crate::msgpack_rpc::packer::Integer;
use std::ffi::{c_char, c_int};
use std::ptr;

const LUA_NOREF: LuaRef = -2;
const CB_MAX_ERROR: u8 = 3;
const LOGLVL_ERR: c_int = 4;

const kRetNilBool: c_int = 1;
const kRetMulti: c_int = 3;

const kDecorProviderActive: c_int = 1;
const kDecorProviderWinDisabled: c_int = 2;
const kDecorProviderRedrawDisabled: c_int = 3;
const kDecorProviderDisabled: c_int = 4;

static mut decor_providers: KVec<DecorProvider> = KVec {
    size: 0,
    capacity: 0,
    items: ptr::null_mut(),
};

#[inline]
const fn error_set(err: &Error) -> bool {
    !matches!(err.err_type, ErrorType::kErrorTypeNone)
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

        let provider = decor_providers.items.add(provider_idx as usize);
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
            decor_provider_error(provider, name, err.msg);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_invoke_spell(
    wp: *mut win_T,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
) {
    unsafe {
        let mut i = 0usize;
        while i < decor_providers.size {
            let p = decor_providers.items.add(i);
            if (*p).state != kDecorProviderDisabled && (*p).spell_nav != LUA_NOREF {
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
                    (*p).spell_nav,
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_invoke_conceal_line(wp: *mut win_T, row: c_int) -> bool {
    unsafe {
        let keys = (*(*wp).w_buffer).b_marktree[0].n_keys;
        let mut i = 0usize;
        while i < decor_providers.size {
            let p = decor_providers.items.add(i);
            if (*p).state != kDecorProviderDisabled && (*p).conceal_line != LUA_NOREF {
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
                    (*p).conceal_line,
                    args,
                    true,
                    ptr::null_mut(),
                );
            }
            i += 1;
        }
        (*(*wp).w_buffer).b_marktree[0].n_keys > keys
    }
}

/// For each provider invoke the 'start' callback (`decor_providers_start`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_start() {
    unsafe {
        let mut i = 0usize;
        while i < decor_providers.size {
            let p = decor_providers.items.add(i);
            if (*p).state != kDecorProviderDisabled && (*p).redraw_start != LUA_NOREF {
                let mut args_buf = [integer_obj((display_tick as c_int) as Integer)];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 1,
                    capacity: 0,
                };
                let active = decor_provider_invoke(
                    i as c_int,
                    c"start".as_ptr(),
                    (*p).redraw_start,
                    args,
                    true,
                    ptr::null_mut(),
                );
                (*decor_providers.items.add(i)).state = if active {
                    kDecorProviderActive
                } else {
                    kDecorProviderRedrawDisabled
                };
            } else if (*p).state != kDecorProviderDisabled {
                (*decor_providers.items.add(i)).state = kDecorProviderActive;
            }
            i += 1;
        }
    }
}

/// For each provider run 'win' (`decor_providers_invoke_win`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_invoke_win(wp: *mut win_T) {
    unsafe {
        debug_assert!(
            decor_state.current_end == 0
                && decor_state.future_begin == decor_state.ranges_i.size as c_int
        );

        if decor_providers.size > 0 {
            validate_botline_win(wp);
        }
        let botline: linenr_T = (*wp).w_botline.min((*(*wp).w_buffer).b_ml.ml_line_count);

        let mut i = 0usize;
        while i < decor_providers.size {
            let p = decor_providers.items.add(i);
            if (*p).state == kDecorProviderWinDisabled {
                (*p).state = kDecorProviderActive;
            }

            (*p).win_skip_row = 0;
            (*p).win_skip_col = 0;

            if (*p).state == kDecorProviderActive && (*p).redraw_win != LUA_NOREF {
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
                    (*p).redraw_win,
                    args,
                    true,
                    ptr::null_mut(),
                ) {
                    (*decor_providers.items.add(i)).state = kDecorProviderWinDisabled;
                }
            }
            i += 1;
        }
    }
}

/// For each provider invoke the 'line' callback for a given window row (`decor_providers_invoke_line`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_invoke_line(wp: *mut win_T, row: c_int) {
    unsafe {
        decor_state.running_decor_provider = true;
        let mut i = 0usize;
        while i < decor_providers.size {
            let p = decor_providers.items.add(i);
            if (*p).state == kDecorProviderActive && (*p).redraw_line != LUA_NOREF {
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
                    (*p).redraw_line,
                    args,
                    true,
                    ptr::null_mut(),
                ) {
                    (*decor_providers.items.add(i)).state = kDecorProviderWinDisabled;
                }

                hl_check_ns();
            }
            i += 1;
        }
        decor_state.running_decor_provider = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_invoke_range(
    wp: *mut win_T,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
) {
    unsafe {
        decor_state.running_decor_provider = true;
        let mut i = 0usize;
        while i < decor_providers.size {
            let mut p = decor_providers.items.add(i);
            if (*p).state == kDecorProviderActive && (*p).redraw_range != LUA_NOREF {
                if (*p).win_skip_row > end_row
                    || ((*p).win_skip_row == end_row && (*p).win_skip_col >= end_col)
                {
                    i += 1;
                    continue;
                }

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
                let mut res = Array::default();
                let status = decor_provider_invoke(
                    i as c_int,
                    c"range".as_ptr(),
                    (*p).redraw_range,
                    args,
                    true,
                    &raw mut res,
                );
                p = decor_providers.items.add(i);

                if !status {
                    (*p).state = kDecorProviderWinDisabled;
                } else if res.size >= 1 {
                    let first = *res.items;
                    if matches!(first.obj_type, ObjectType::kObjectTypeBoolean) {
                        if !first.data.boolean {
                            (*p).state = kDecorProviderWinDisabled;
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
                        (*p).win_skip_row = row as c_int;
                        (*p).win_skip_col = col as c_int;
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_invoke_buf(buf: *mut buf_T) {
    unsafe {
        let mut i = 0usize;
        while i < decor_providers.size {
            let p = decor_providers.items.add(i);
            if (*p).state == kDecorProviderActive && (*p).redraw_buf != LUA_NOREF {
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
                    (*p).redraw_buf,
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_providers_invoke_end() {
    unsafe {
        let mut i = 0usize;
        while i < decor_providers.size {
            let p = decor_providers.items.add(i);
            if (*p).state != kDecorProviderDisabled && (*p).redraw_end != LUA_NOREF {
                let mut args_buf = [integer_obj((display_tick as c_int) as Integer)];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 1,
                    capacity: 0,
                };
                decor_provider_invoke(
                    i as c_int,
                    c"end".as_ptr(),
                    (*p).redraw_end,
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_provider_invalidate_hl() {
    unsafe {
        for i in 0..decor_providers.size {
            (*decor_providers.items.add(i)).hl_cached = false;
        }

        if ns_hl_active != 0 {
            ns_hl_active = -1;
            hl_check_ns();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_decor_provider(ns_id: NS, force: bool) -> *mut DecorProvider {
    unsafe {
        debug_assert!(ns_id > 0);
        let len = decor_providers.size;
        for i in 0..len {
            let p = decor_providers.items.add(i);
            if (*p).ns_id == ns_id {
                return p;
            }
        }

        if !force {
            return ptr::null_mut();
        }

        if decor_providers.capacity < len + 1 {
            let mut cap = len + 1;
            cap = cap.next_power_of_two().max(4);
            decor_providers.capacity = cap;
            decor_providers.items = xrealloc(
                decor_providers.items.cast(),
                cap * size_of::<DecorProvider>(),
            )
            .cast();
        }
        decor_providers.size = len + 1;
        let item = decor_providers.items.add(len);
        ptr::write(item, decor_provider_init(ns_id));
        item
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_provider_clear(p: *mut DecorProvider) {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn decor_free_all_mem() {
    unsafe {
        for i in 0..decor_providers.size {
            decor_provider_clear(decor_providers.items.add(i));
        }
        xfree(decor_providers.items.cast());
        decor_providers = KVec {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
    }
}
