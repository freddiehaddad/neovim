//! Decoration provider management and invocation (`src/nvim/decoration_provider.c`).
#![forbid(unsafe_code)]

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

pub(crate) const CB_MAX_ERROR: u8 = 3;
pub(crate) const K_DECOR_PROVIDER_ACTIVE: i32 = 1;
const K_DECOR_PROVIDER_WIN_DISABLED: i32 = 2;
const K_DECOR_PROVIDER_REDRAW_DISABLED: i32 = 3;
const K_DECOR_PROVIDER_DISABLED: i32 = 4;

#[repr(C)]
pub struct DecorProvider {
    pub ns_id: NS,
    pub state: i32,
    pub win_skip_row: i32,
    pub win_skip_col: i32,
    pub redraw_start: LuaRef,
    pub redraw_buf: LuaRef,
    pub redraw_win: LuaRef,
    pub redraw_line: LuaRef,
    pub redraw_range: LuaRef,
    pub redraw_end: LuaRef,
    pub hl_def: LuaRef,
    pub spell_nav: LuaRef,
    pub conceal_line: LuaRef,
    pub conceal: LuaRef,
    pub hl_valid: i32,
    pub hl_cached: bool,
    pub error_count: u8,
}

std::thread_local! {
    static DECOR_PROVIDERS: std::cell::RefCell<Vec<DecorProvider>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

#[inline]
fn with_providers_mut<R>(f: impl FnOnce(&mut Vec<DecorProvider>) -> R) -> R {
    DECOR_PROVIDERS.with(|p| f(&mut p.borrow_mut()))
}

#[inline]
fn providers_len() -> usize {
    DECOR_PROVIDERS.with(|p| p.borrow().len())
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
        conceal_line: LUA_NOREF,
        conceal: LUA_NOREF,
        hl_valid: -1,
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
    name: &std::ffi::CStr,
    msg: Option<&std::ffi::CStr>,
) {
    let ns = describe_ns(provider.ns_id, c"(UNKNOWN PLUGIN)");
    logmsg!(
        LOGLVL_ERR,
        None,
        Some(c"decor_provider_error"),
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
    provider_idx: i32,
    name: &std::ffi::CStr,
    ref_: LuaRef,
    args: Array,
    default_true: bool,
    res: Option<&mut Array>,
) -> bool {
    let mut err = error_init();

    g_st().textlock += 1;
    let ret = nlua_call_ref(
        ref_,
        Some(name),
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

    let ret_ok = with_providers_mut(|providers| {
        let provider = &mut providers[provider_idx as usize];
        if !error_set(&err) {
            provider.error_count = 0;
            if let Some(res) = res {
                debug_assert!(matches!(ret.obj_type, ObjectType::ObjectTypeArray));
                *res = ret.data.array();
                return true;
            } else if api_object_to_bool(ret.clone(), c"provider %s retval", default_true, &mut err)
            {
                return true;
            }
        }

        if error_set(&err) && provider.error_count < CB_MAX_ERROR {
            decor_provider_error(&mut *provider, name, err.msg.as_deref());
            provider.error_count += 1;

            if provider.error_count >= CB_MAX_ERROR {
                provider.state = K_DECOR_PROVIDER_DISABLED;
            }
        }
        false
    });
    if ret_ok {
        return true;
    }

    api_clear_error(&mut err);
    api_free_object(ret);
    false
}

pub fn decor_providers_invoke_spell(
    wp: &mut WinInfoInfo,
    start_row: i32,
    start_col: i32,
    end_row: i32,
    end_col: i32,
) {
    let mut i = 0usize;
    while i < providers_len() {
        let spell_nav = with_providers_mut(|providers| {
            let p = &providers[i];
            if p.state != K_DECOR_PROVIDER_DISABLED && p.spell_nav != LUA_NOREF {
                Some(p.spell_nav)
            } else {
                None
            }
        });
        if let Some(spell_nav) = spell_nav {
            let mut args_buf = [
                integer_obj(wp.handle as Integer),
                integer_obj(wp.buf_mut().b_fnum as Integer),
                integer_obj(start_row as Integer),
                integer_obj(start_col as Integer),
                integer_obj(end_row as Integer),
                integer_obj(end_col as Integer),
            ];
            let args = Array::from_slice(&mut args_buf[..6]);
            decor_provider_invoke(i as i32, c"spell", spell_nav, args, true, None);
        }
        i += 1;
    }
}

/// Invokes the `_on_conceal_line` (if `lines`) or `_on_conceal` callback for buffer row `row` (`decor_providers_invoke_conceal`).
pub fn decor_providers_invoke_conceal(wp: &mut WinInfoInfo, row: i32, lines: bool) {
    if !lines
        && (row < 0
            || wp.w_onebuf_opt.wo_cole.get() < 1
            || wp.buf_mut().b_conceal_providers == 0
            || (!wp.w_onebuf_opt.wo_wrap.get()
                && !crate::decoration::decor_row_has_conceal(wp, row)))
    {
        return;
    }
    let running = g_st().decor_state.running_decor_provider;
    g_st().decor_state.running_decor_provider = running || g_st().updating_screen;
    let mut i = 0usize;
    while i < providers_len() {
        let ref_opt = with_providers_mut(|providers| {
            let p = &providers[i];
            let ref_ = if lines { p.conceal_line } else { p.conceal };
            if p.state != K_DECOR_PROVIDER_DISABLED && ref_ != LUA_NOREF {
                Some(ref_)
            } else {
                None
            }
        });
        if let Some(ref_) = ref_opt {
            let mut args_buf = [
                integer_obj(wp.handle as Integer),
                integer_obj(wp.buf_mut().b_fnum as Integer),
                integer_obj(row as Integer),
            ];
            let args = Array::from_slice(&mut args_buf[..3]);
            decor_provider_invoke(
                i as i32,
                if lines { c"conceal_line" } else { c"conceal" },
                ref_,
                args,
                true,
                None,
            );
        }
        i += 1;
    }
    g_st().decor_state.running_decor_provider = running;
}

/// For each provider invoke the 'start' callback (`decor_providers_start`).
pub fn decor_providers_start() {
    let mut i = 0usize;
    while i < providers_len() {
        let redraw_start = with_providers_mut(|providers| {
            let p = &mut providers[i];
            if p.state != K_DECOR_PROVIDER_DISABLED && p.redraw_start != LUA_NOREF {
                Some(p.redraw_start)
            } else {
                if p.state != K_DECOR_PROVIDER_DISABLED {
                    p.state = K_DECOR_PROVIDER_ACTIVE;
                }
                None
            }
        });
        if let Some(redraw_start) = redraw_start {
            let mut args_buf = [integer_obj((display_tick_val() as i32) as Integer)];
            let args = Array::from_slice(&mut args_buf[..1]);
            let active = decor_provider_invoke(i as i32, c"start", redraw_start, args, true, None);
            with_providers_mut(|providers| {
                providers[i].state = if active {
                    K_DECOR_PROVIDER_ACTIVE
                } else {
                    K_DECOR_PROVIDER_REDRAW_DISABLED
                };
            });
        }
        i += 1;
    }
}

/// For each provider run 'win' (`decor_providers_invoke_win`).
pub fn decor_providers_invoke_win(wp: &mut WinInfoInfo) {
    debug_assert!(
        g_st().decor_state.current_end == 0
            && g_st().decor_state.future_begin == g_st().decor_state.ranges_i.len() as i32
    );

    if providers_len() > 0 {
        validate_botline_win(&mut *wp);
    }
    let botline: Linenr = wp.w_botline.min(wp.buf_mut().b_ml.ml_line_count);

    let mut i = 0usize;
    while i < providers_len() {
        let redraw_win = with_providers_mut(|providers| {
            let p = &mut providers[i];
            if p.state == K_DECOR_PROVIDER_WIN_DISABLED {
                p.state = K_DECOR_PROVIDER_ACTIVE;
            }
            p.win_skip_row = 0;
            p.win_skip_col = 0;
            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_win != LUA_NOREF {
                Some(p.redraw_win)
            } else {
                None
            }
        });
        if let Some(redraw_win) = redraw_win {
            let mut args_buf = [
                window_obj(wp.handle),
                buffer_obj(wp.buf_mut().b_fnum),
                integer_obj((wp.w_topline - 1) as Integer),
                integer_obj((botline - 1) as Integer),
            ];
            let args = Array::from_slice(&mut args_buf[..4]);
            if !decor_provider_invoke(i as i32, c"win", redraw_win, args, true, None) {
                with_providers_mut(|providers| {
                    providers[i].state = K_DECOR_PROVIDER_WIN_DISABLED;
                });
            }
        }
        i += 1;
    }
}

/// For each provider invoke the 'line' callback for a given window row (`decor_providers_invoke_line`).
pub fn decor_providers_invoke_line(wp: &mut WinInfoInfo, row: i32) {
    g_st().decor_state.running_decor_provider = true;
    let mut i = 0usize;
    while i < providers_len() {
        let redraw_line = with_providers_mut(|providers| {
            let p = &providers[i];
            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_line != LUA_NOREF {
                Some(p.redraw_line)
            } else {
                None
            }
        });
        if let Some(redraw_line) = redraw_line {
            let mut args_buf = [
                window_obj(wp.handle),
                buffer_obj(wp.buf_mut().b_fnum),
                integer_obj(row as Integer),
            ];
            let args = Array::from_slice(&mut args_buf[..3]);
            if !decor_provider_invoke(i as i32, c"line", redraw_line, args, true, None) {
                with_providers_mut(|providers| {
                    providers[i].state = K_DECOR_PROVIDER_WIN_DISABLED;
                });
            }

            hl_check_ns();
        }
        i += 1;
    }
    g_st().decor_state.running_decor_provider = false;
}

pub fn decor_providers_invoke_range(
    wp: &mut WinInfoInfo,
    start_row: i32,
    start_col: i32,
    end_row: i32,
    end_col: i32,
) {
    g_st().decor_state.running_decor_provider = true;
    let mut i = 0usize;
    while i < providers_len() {
        let redraw_range = with_providers_mut(|providers| {
            let p = &providers[i];
            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_range != LUA_NOREF {
                if p.win_skip_row > end_row
                    || (p.win_skip_row == end_row && p.win_skip_col >= end_col)
                {
                    None
                } else {
                    Some(p.redraw_range)
                }
            } else {
                None
            }
        });
        if let Some(redraw_range) = redraw_range {
            let mut args_buf = [
                window_obj(wp.handle),
                buffer_obj(wp.buf_mut().b_fnum),
                integer_obj(start_row as Integer),
                integer_obj(start_col as Integer),
                integer_obj(end_row as Integer),
                integer_obj(end_col as Integer),
            ];
            let args = Array::from_slice(&mut args_buf[..6]);
            let mut res = Array::new();
            let status =
                decor_provider_invoke(i as i32, c"range", redraw_range, args, true, Some(&mut res));
            with_providers_mut(|providers| {
                let p = &mut providers[i];
                if !status {
                    p.state = K_DECOR_PROVIDER_WIN_DISABLED;
                } else if res.size >= 1 {
                    let first = &res[0];
                    if matches!(first.obj_type, ObjectType::ObjectTypeBoolean) {
                        if !first.data.boolean() {
                            p.state = K_DECOR_PROVIDER_WIN_DISABLED;
                        }
                    } else if matches!(first.obj_type, ObjectType::ObjectTypeInteger) {
                        let row = first.data.integer();
                        let mut col: Integer = 0;
                        if res.size >= 2 {
                            let second = &res[1];
                            if matches!(second.obj_type, ObjectType::ObjectTypeInteger) {
                                col = second.data.integer();
                            }
                        }
                        p.win_skip_row = row as i32;
                        p.win_skip_col = col as i32;
                    }
                }
            });

            api_free_array(res);

            hl_check_ns();
        }
        i += 1;
    }
    g_st().decor_state.running_decor_provider = false;
}

/// For each provider invoke the 'buf' callback for a given buffer (`decor_providers_invoke_buf`).
pub fn decor_providers_invoke_buf(buf: &mut FileBuf) {
    let mut i = 0usize;
    while i < providers_len() {
        let redraw_buf = with_providers_mut(|providers| {
            let p = &providers[i];
            if p.state == K_DECOR_PROVIDER_ACTIVE && p.redraw_buf != LUA_NOREF {
                Some(p.redraw_buf)
            } else {
                None
            }
        });
        if let Some(redraw_buf) = redraw_buf {
            let mut args_buf = [
                buffer_obj(buf.b_fnum),
                integer_obj(display_tick_val() as Integer),
            ];
            let args = Array::from_slice(&mut args_buf[..2]);
            decor_provider_invoke(i as i32, c"buf", redraw_buf, args, true, None);
        }
        i += 1;
    }
}

/// For each provider invoke the 'end' callback (`decor_providers_invoke_end`).
pub fn decor_providers_invoke_end() {
    let mut i = 0usize;
    while i < providers_len() {
        let redraw_end = with_providers_mut(|providers| {
            let p = &providers[i];
            if p.state != K_DECOR_PROVIDER_DISABLED && p.redraw_end != LUA_NOREF {
                Some(p.redraw_end)
            } else {
                None
            }
        });
        if let Some(redraw_end) = redraw_end {
            let mut args_buf = [integer_obj((display_tick_val() as i32) as Integer)];
            let args = Array::from_slice(&mut args_buf[..1]);
            decor_provider_invoke(i as i32, c"end", redraw_end, args, true, None);
        }
        i += 1;
    }
    decor_check_to_be_deleted();
}

/// Mark all cached state of per-namespace highlights as invalid (`decor_provider_invalidate_hl`).
pub fn decor_provider_invalidate_hl() {
    with_providers_mut(|providers| {
        for p in providers.iter_mut() {
            p.hl_cached = false;
        }
    });

    if g_st().ns_hl_active != 0 {
        g_st().ns_hl_active = -1;
        hl_check_ns();
    }
}

pub fn with_decor_provider_mut<R>(
    ns_id: NS,
    force: bool,
    f: impl FnOnce(&mut DecorProvider) -> R,
) -> Option<R> {
    debug_assert!(ns_id > 0);
    with_providers_mut(|p_vec| {
        if let Some(idx) = p_vec.iter().position(|p| p.ns_id == ns_id) {
            return Some(f(&mut p_vec[idx]));
        }

        if !force {
            return None;
        }

        p_vec.push(decor_provider_init(ns_id));
        p_vec.last_mut().map(f)
    })
}

pub fn decor_provider_clear(mut p: Option<&mut DecorProvider>) {
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
    clear_ref(&mut p.as_deref_mut().unwrap().conceal);
    p.unwrap().state = K_DECOR_PROVIDER_DISABLED;
}

impl Drop for DecorProvider {
    fn drop(&mut self) {
        decor_provider_clear(Some(&mut *self));
    }
}

pub fn decor_free_all_mem() {
    with_providers_mut(|providers| *providers = Vec::new());
}
