//! Decoration provider management and invocation (`src/nvim/decoration_provider.c`).

use crate::api::extmark::describe_ns;
use crate::api::private::helpers::{
    api_clear_error, api_free_array, api_free_object, api_object_to_bool, buffer_obj, error_init,
    error_set, integer_obj, window_obj,
};
use crate::api::private::validate::{Array, ObjectType};
use crate::buffer::FileBuf;
use crate::cursor::Linenr;
use crate::decoration::{NS, decor_check_to_be_deleted};
use crate::globals::*;
use crate::highlight::hl_check_ns;
use crate::log::{LOGLVL_ERR, logmsg};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{K_RET_MULTI, K_RET_NIL_BOOL, api_free_luaref, nlua_call_ref};
use crate::message::msg_schedule_semsg_multiline;
use crate::r#move::validate_botline_win;
use crate::msgpack_rpc::packer::Integer;
use crate::window::WinInfoInfo;
use std::ffi::c_int;
use std::ptr;

pub(crate) const CB_MAX_ERROR: u8 = 3;
pub(crate) const K_DECOR_PROVIDER_ACTIVE: c_int = 1;
const K_DECOR_PROVIDER_WIN_DISABLED: c_int = 2;
const K_DECOR_PROVIDER_REDRAW_DISABLED: c_int = 3;
const K_DECOR_PROVIDER_DISABLED: c_int = 4;

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

struct DecorProviderState {
    decor_providers: Vec<DecorProvider>,
}

impl DecorProviderState {
    const fn new() -> Self {
        Self {
            decor_providers: Vec::new(),
        }
    }
}

static DECOR_PROV_STATE: crate::globals::GlobalState<DecorProviderState> =
    crate::globals::GlobalState::new(DecorProviderState::new());

#[inline]
fn decor_prov_st() -> &'static mut DecorProviderState {
    unsafe { &mut *DECOR_PROV_STATE.as_ptr() }
}

#[inline]
fn providers_mut() -> &'static mut Vec<DecorProvider> {
    &mut decor_prov_st().decor_providers
}

#[inline]
const fn decor_provider_init(ns_id: NS) -> DecorProvider {
    DecorProvider {
        ns_id,
        state: K_DECOR_PROVIDER_DISABLED,
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
fn clear_ref(r: &mut LuaRef) {
    if *r != LUA_NOREF {
        api_free_luaref(*r);
        *r = LUA_NOREF;
    }
}

fn decor_provider_error(
    provider: &mut DecorProvider,
    name: impl crate::strings::IntoCCharPtr,
    msg: impl crate::strings::IntoCCharPtr,
) {
    let name = name.into_c_char_ptr();
    let msg = msg.into_c_char_ptr();
    let ns = describe_ns(provider.ns_id, c"(UNKNOWN PLUGIN)");
    logmsg!(
        LOGLVL_ERR,
        ptr::null(),
        c"decor_provider_error",
        31,
        true,
        c"Error in decoration provider \"%s\" (ns=%s):\n%s",
        name,
        ns,
        msg,
    );
    msg_schedule_semsg_multiline!(c"Decoration provider \"%s\" (ns=%s):\n%s", name, ns, msg,);
}

/// Note we pass in a provider index as this function may cause `decor_prov_st().decor_providers` to be
/// reallocated so we need to be careful with `DecorProvider` pointers (`decor_provider_invoke`).
fn decor_provider_invoke(
    provider_idx: c_int,
    name: &std::ffi::CStr,
    ref_: LuaRef,
    args: Array,
    default_true: bool,
    res: Option<&mut Array>,
) -> bool {
    {
        let mut err = error_init();

        g_st().textlock += 1;
        let ret = nlua_call_ref(
            ref_,
            name.as_ptr(),
            args,
            if res.is_some() {
                K_RET_MULTI
            } else {
                K_RET_NIL_BOOL
            },
            None,
            Some(&mut err),
        );
        g_st().textlock -= 1;

        let provider = &raw mut providers_mut()[provider_idx as usize];
        let provider = unsafe { &mut *provider };
        if !error_set(&err) {
            provider.error_count = 0;
            if let Some(res) = res {
                debug_assert!(matches!(ret.obj_type, ObjectType::ObjectTypeArray));
                *res = ret.data.array();
                return true;
            } else if api_object_to_bool(ret, c"provider %s retval", default_true, &mut err) {
                return true;
            }
        }

        if error_set(&err) && provider.error_count < CB_MAX_ERROR {
            decor_provider_error(&mut *provider, name.as_ptr(), err.msg_ptr());
            provider.error_count += 1;

            if provider.error_count >= CB_MAX_ERROR {
                provider.state = K_DECOR_PROVIDER_DISABLED;
            }
        }

        api_clear_error(&mut err);
        api_free_object(ret);
        false
    }
}

pub fn decor_providers_invoke_spell(
    wp: &mut WinInfoInfo,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
) {
    {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != K_DECOR_PROVIDER_DISABLED && p.spell_nav != LUA_NOREF {
                let spell_nav = p.spell_nav;
                let mut args_buf = [
                    integer_obj(wp.handle as Integer),
                    integer_obj(wp.buf_mut().b_fnum as Integer),
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
                decor_provider_invoke(i as c_int, c"spell", spell_nav, args, true, None);
            }
            i += 1;
        }
    }
}

/// Return whether a provider placed any marks in the callback (`decor_providers_invoke_conceal_line`).
pub fn decor_providers_invoke_conceal_line(wp: &mut WinInfoInfo, row: c_int) -> bool {
    {
        let keys = wp.buf_mut().b_marktree.n_keys;
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != K_DECOR_PROVIDER_DISABLED && p.conceal_line != LUA_NOREF {
                let conceal_line = p.conceal_line;
                let mut args_buf = [
                    integer_obj(wp.handle as Integer),
                    integer_obj(wp.buf_mut().b_fnum as Integer),
                    integer_obj(row as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 3,
                    capacity: 0,
                };
                decor_provider_invoke(i as c_int, c"conceal_line", conceal_line, args, true, None);
            }
            i += 1;
        }
        wp.buf_mut().b_marktree.n_keys > keys
    }
}

/// For each provider invoke the 'start' callback (`decor_providers_start`).
pub fn decor_providers_start() {
    {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != K_DECOR_PROVIDER_DISABLED && p.redraw_start != LUA_NOREF {
                let redraw_start = p.redraw_start;
                let mut args_buf = [integer_obj((display_tick_val() as c_int) as Integer)];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 1,
                    capacity: 0,
                };
                let active =
                    decor_provider_invoke(i as c_int, c"start", redraw_start, args, true, None);
                providers_mut()[i].state = if active {
                    K_DECOR_PROVIDER_ACTIVE
                } else {
                    K_DECOR_PROVIDER_REDRAW_DISABLED
                };
            } else if p.state != K_DECOR_PROVIDER_DISABLED {
                providers_mut()[i].state = K_DECOR_PROVIDER_ACTIVE;
            }
            i += 1;
        }
    }
}

/// For each provider run 'win' (`decor_providers_invoke_win`).
pub fn decor_providers_invoke_win(wp: &mut WinInfoInfo) {
    {
        debug_assert!(
            g_st().decor_state.current_end == 0
                && g_st().decor_state.future_begin == g_st().decor_state.ranges_i.len() as c_int
        );

        if !providers_mut().is_empty() {
            validate_botline_win(&mut *wp);
        }
        let botline: Linenr = wp.w_botline.min(wp.buf_mut().b_ml.ml_line_count);

        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &mut providers_mut()[i];
            if p.state == K_DECOR_PROVIDER_WIN_DISABLED {
                p.state = K_DECOR_PROVIDER_ACTIVE;
            }

            p.win_skip_row = 0;
            p.win_skip_col = 0;

            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_win != LUA_NOREF {
                let redraw_win = p.redraw_win;
                let mut args_buf = [
                    window_obj(wp.handle),
                    buffer_obj(wp.buf_mut().b_fnum),
                    integer_obj((wp.w_topline - 1) as Integer),
                    integer_obj((botline - 1) as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 4,
                    capacity: 0,
                };
                if !decor_provider_invoke(i as c_int, c"win", redraw_win, args, true, None) {
                    providers_mut()[i].state = K_DECOR_PROVIDER_WIN_DISABLED;
                }
            }
            i += 1;
        }
    }
}

/// For each provider invoke the 'line' callback for a given window row (`decor_providers_invoke_line`).
pub fn decor_providers_invoke_line(wp: &mut WinInfoInfo, row: c_int) {
    {
        g_st().decor_state.running_decor_provider = true;
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_line != LUA_NOREF {
                let redraw_line = p.redraw_line;
                let mut args_buf = [
                    window_obj(wp.handle),
                    buffer_obj(wp.buf_mut().b_fnum),
                    integer_obj(row as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 3,
                    capacity: 0,
                };
                if !decor_provider_invoke(i as c_int, c"line", redraw_line, args, true, None) {
                    providers_mut()[i].state = K_DECOR_PROVIDER_WIN_DISABLED;
                }

                hl_check_ns();
            }
            i += 1;
        }
        g_st().decor_state.running_decor_provider = false;
    }
}

pub fn decor_providers_invoke_range(
    wp: &mut WinInfoInfo,
    start_row: c_int,
    start_col: c_int,
    end_row: c_int,
    end_col: c_int,
) {
    unsafe {
        g_st().decor_state.running_decor_provider = true;
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_range != LUA_NOREF {
                if p.win_skip_row > end_row
                    || (p.win_skip_row == end_row && p.win_skip_col >= end_col)
                {
                    i += 1;
                    continue;
                }

                let redraw_range = p.redraw_range;
                let mut args_buf = [
                    window_obj(wp.handle),
                    buffer_obj(wp.buf_mut().b_fnum),
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
                    c"range",
                    redraw_range,
                    args,
                    true,
                    Some(&mut res),
                );
                let p = &mut providers_mut()[i];

                if !status {
                    p.state = K_DECOR_PROVIDER_WIN_DISABLED;
                } else if res.size >= 1 {
                    let first = *res.items;
                    if matches!(first.obj_type, ObjectType::ObjectTypeBoolean) {
                        if !first.data.boolean() {
                            p.state = K_DECOR_PROVIDER_WIN_DISABLED;
                        }
                    } else if matches!(first.obj_type, ObjectType::ObjectTypeInteger) {
                        let row = first.data.integer();
                        let mut col: Integer = 0;
                        if res.size >= 2 {
                            let second = res[1];
                            if matches!(second.obj_type, ObjectType::ObjectTypeInteger) {
                                col = second.data.integer();
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
        g_st().decor_state.running_decor_provider = false;
    }
}

/// For each provider invoke the 'buf' callback for a given buffer (`decor_providers_invoke_buf`).
pub fn decor_providers_invoke_buf(buf: &mut FileBuf) {
    {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_buf != LUA_NOREF {
                let redraw_buf = p.redraw_buf;
                let mut args_buf = [
                    buffer_obj(buf.b_fnum),
                    integer_obj(display_tick_val() as Integer),
                ];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 2,
                    capacity: 0,
                };
                decor_provider_invoke(i as c_int, c"buf", redraw_buf, args, true, None);
            }
            i += 1;
        }
    }
}

/// For each provider invoke the 'end' callback (`decor_providers_invoke_end`).
pub fn decor_providers_invoke_end() {
    {
        let mut i = 0usize;
        while i < providers_mut().len() {
            let p = &providers_mut()[i];
            if p.state != K_DECOR_PROVIDER_DISABLED && p.redraw_end != LUA_NOREF {
                let redraw_end = p.redraw_end;
                let mut args_buf = [integer_obj((display_tick_val() as c_int) as Integer)];
                let args = Array {
                    items: args_buf.as_mut_ptr(),
                    size: 1,
                    capacity: 0,
                };
                decor_provider_invoke(i as c_int, c"end", redraw_end, args, true, None);
            }
            i += 1;
        }
        decor_check_to_be_deleted();
    }
}

/// Mark all cached state of per-namespace highlights as invalid (`decor_provider_invalidate_hl`).
pub fn decor_provider_invalidate_hl() {
    for p in providers_mut().iter_mut() {
        p.hl_cached = false;
    }

    if g_st().ns_hl_active != 0 {
        g_st().ns_hl_active = -1;
        hl_check_ns();
    }
}

pub fn get_decor_provider(ns_id: NS, force: bool) -> *mut DecorProvider {
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

pub fn decor_provider_clear(mut p: Option<&mut DecorProvider>) {
    {
        if p.is_none() {
            return;
        }
        clear_ref(&mut p.as_deref_mut().unwrap().redraw_start);
        clear_ref(&mut p.as_deref_mut().unwrap().redraw_buf);
        clear_ref(&mut p.as_deref_mut().unwrap().redraw_win);
        clear_ref(&mut p.as_deref_mut().unwrap().redraw_line);
        clear_ref(&mut p.as_deref_mut().unwrap().redraw_range);
        clear_ref(&mut p.as_deref_mut().unwrap().redraw_end);
        clear_ref(&mut p.as_deref_mut().unwrap().spell_nav);
        clear_ref(&mut p.as_deref_mut().unwrap().conceal_line);
        p.unwrap().state = K_DECOR_PROVIDER_DISABLED;
    }
}

impl Drop for DecorProvider {
    fn drop(&mut self) {
        decor_provider_clear(Some(&mut *self));
    }
}

pub fn decor_free_all_mem() {
    decor_prov_st().decor_providers = Vec::new();
}
