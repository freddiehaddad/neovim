//! Global Neovim API functions (`src/nvim/api/vim.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::add_c;
#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
#[allow(unused_imports)]
use crate::api::private::helpers::static_cstr_as_string;
#[allow(unused_imports)]
use crate::api::private::helpers::{
    api_metadata, api_set_sctx, api_typename, arena_array, arena_dict, arena_string,
    arena_take_arraybuilder, copy_array, copy_dict, copy_object, copy_string, cstr_as_string,
    dict_get_value, dict_set_var, find_buffer_by_handle, find_tab_by_handle, find_window_by_handle,
    get_default_stl_hl, parse_hl_msg, set_mark, string_to_array, try_enter, try_leave,
};
#[allow(unused_imports)]
use crate::api::private::helpers::{
    array_obj, boolean_obj, buffer_obj, cstr_as_obj, dict_obj, integer_obj, nil_obj, string_obj,
    tabpage_obj, window_obj,
};
#[allow(unused_imports)]
use crate::autocmd::may_trigger_vim_suspend_resume;
#[allow(unused_imports)]
use crate::buffer::buf_get_changedtick;
#[allow(unused_imports)]
use crate::buffer::{buf_close_terminal, buflist_new, buflist_nr2name, do_buffer};
#[allow(unused_imports)]
use crate::buffer::{bufref_valid, read_buffer_into, set_bufref};
#[allow(unused_imports)]
use crate::channel::channel_event;
#[allow(unused_imports)]
use crate::cursor::get_cursor_rel_lnum;
#[allow(unused_imports)]
use crate::drawline::use_cursor_line_highlight;
#[allow(unused_imports)]
use crate::drawscreen::win_update_cursorline;
#[allow(unused_imports)]
use crate::eval::typval::tv_dict_find;
#[allow(unused_imports)]
use crate::ex_docmd::do_chdir;
#[allow(unused_imports)]
use crate::ex_docmd::exec_normal;
#[allow(unused_imports)]
use crate::ex_eval::aborting;
#[allow(unused_imports)]
use crate::fold::fold_info;
use crate::globals::*;
#[allow(unused_imports)]
use crate::grid::{get_win_by_grid_handle, win_grid_alloc};
#[allow(unused_imports)]
use crate::highlight::win_check_ns_hl;
#[allow(unused_imports)]
use crate::input::ins_typebuf;
#[allow(unused_imports)]
use crate::insexpand::get_cot_flags;
#[allow(unused_imports)]
use crate::lua::executor::{api_free_luaref, nlua_get_global_ref_count, nlua_is_deferred_safe};
#[allow(unused_imports)]
use crate::lua::executor::{nlua_call_ref, nlua_exec};
#[allow(unused_imports)]
use crate::mapping::{keymap_array, modify_keymap};
#[allow(unused_imports)]
use crate::message::msg_multihl;
#[allow(unused_imports)]
use crate::message::{
    hl_msg_free, msg_ext_no_fast, msg_id_exists, verbose_enter, verbose_leave, verbose_stop,
};
#[allow(unused_imports)]
use crate::r#move::{changed_window_setting, win_col_off};
#[allow(unused_imports)]
use crate::normal::reset_VIsual_and_resel;
#[allow(unused_imports)]
use crate::option::buf_copy_options;
#[allow(unused_imports)]
use crate::option::set_option_direct_for;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::optionstr::check_stl_option;
#[allow(unused_imports)]
use crate::os::input::input_enqueue;
#[allow(unused_imports)]
use crate::register::{finish_yankreg_from_object, prepare_yankreg_from_object};
#[allow(unused_imports)]
use crate::runtime::do_in_runtimepath;
#[allow(unused_imports)]
use crate::runtime::{do_source, get_lib_dir, runtime_get_named, runtime_inspect, script_autoload};
#[allow(unused_imports)]
use crate::ui::{ui_array, ui_call__set_restart_on_crash_exit, ui_call_screenshot, ui_flush};
#[allow(unused_imports)]
use crate::window::{global_stl_height, goto_tabpage_tp, goto_tabpage_win, win_find_tabpage};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::buffer::{
    KeyDict_keymap, KeyDict_keymap_del, api_buf_ensure_loaded, nvim_buf_del_keymap,
};
use crate::api::deprecated::{KeyDict_empty, buffer_del_line, buffer_get_line, buffer_set_line};
use crate::api::extmark::NS;
use crate::api::options::CharBuf;
use crate::api::private::converter::vim_to_object;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectType, api_err_exp, api_err_invalid,
    api_err_required,
};
use crate::api::tabpage::{Buffer, ScreenGrid, Tabpage, Window, pos_T, win_T};
use crate::api::win_config::bufref_T;
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFNEW, apply_autocmds, block_autocmds, unblock_autocmds,
};
use crate::channel::{
    InternalState, TerminalOptions, channel_all_info, channel_alloc, channel_decref,
    channel_incref, channel_info, channel_send,
};
use crate::clipboard::yankreg_T;
use crate::context::{
    Context, CtxStateFlags, ctx_free, ctx_from_dict, ctx_load, ctx_save, ctx_to_dict,
};
use crate::cursor::linenr_T;
use crate::decoration::decor_redraw_signs;
use crate::drawscreen::{
    redraw_all_later, redraw_buf_later, redraw_buf_range_later, redraw_later, setcursor_mayforce,
    update_screen,
};
use crate::eval::deprecated::buf_T;
use crate::eval::vars::{get_globvar_dict, get_vimvar_dict, set_vim_var_nr};
use crate::grid::{schar_T, schar_cache_clear, schar_get};
use crate::highlight::{
    KeyDict_highlight, OptionalKeys, dict2hlattrs, highlight_use_hlstate, hl_check_ns,
    hl_get_attr_by_id, hl_inspect, hl_ns_get_attrs, ns_hl_def,
};
use crate::highlight_group::{
    KeyDict_get_highlight, color_name_table, name_to_color, ns_get_hl_defs, syn_check_group,
    syn_id2name,
};
use crate::input::paste_store;
use crate::input_cmdatom::StringBuilder;
use crate::keycodes::{
    KE_LEFTDRAG, KE_LEFTMOUSE, KE_LEFTRELEASE, KE_MIDDLEMOUSE, KE_MOUSEDOWN, KE_MOUSELEFT,
    KE_MOUSEMOVE, KE_MOUSERIGHT, KE_MOUSEUP, KE_RIGHTMOUSE, KE_X1MOUSE, KE_X2MOUSE, REPTERM_DO_LT,
    REPTERM_FROM_PART, REPTERM_NO_SPECIAL, name_to_mod_mask, replace_termcodes,
    vim_strsave_escape_ks,
};
use crate::lua::converter::{Boolean, Float, LuaRef};
use crate::map::HlAttrs;
use crate::map_glyph_cache::NvimString;
use crate::mapping::MAPTYPE_MAP;
use crate::mark::mark_get_global;
use crate::math::{FAIL, xpopcount};
use crate::mbyte::{mb_string2cells, utfc_ptr2len, utfc_ptr2schar};
use crate::memfile::memfile_T;
use crate::memline::ml_open;
use crate::memory::{Arena, arena_alloc, arena_strdup, memchrsub, strequal, xfree};
use crate::r#move::{update_topline, validate_cursor};
use crate::msgpack_rpc::channel::{
    Channel, ChannelStreamType, channel_prefix, channel_stream_ptr, channel_tail, find_channel,
    rpc_set_client_info,
};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::msgpack_rpc::unpacker::unpack;
use crate::os::input::{input_blocking, input_enqueue_mouse, input_enqueue_raw};
use crate::os::proc::os_proc_children;
#[cfg(windows)]
use crate::os::proc::os_proc_info;
use crate::popupmenu::{pum_ext_select_item, pum_set_info};
use crate::register::{PUT_CURSEND, do_put};
use crate::state::{MODE_MAX_LENGTH, get_mode};
use crate::statusline::{
    SignTextAttrs, build_stl_str_hl, draw_tabline, fillchar_status, foldinfo_T, hlf_T, statuscol_T,
    stl_hlrec_t, win_redr_status, win_redr_winbar,
};
use crate::terminal::{
    terminal_alloc, terminal_buf, terminal_check_size, terminal_destroy, terminal_open,
    terminal_running, terminal_set_streamed_paste,
};

const NUL: c_char = 0;
const NL: c_char = b'\n' as c_char;
const MAXPATHL: usize = 4096;
const MAX_SCHAR_SIZE: usize = 32;
const ERR_BUFLEN: usize = 256;
const SIGN_SHOW_MAX: usize = 9;

const FORWARD: c_int = 1;
const BACKWARD: c_int = -1;

const REMAP_YES: c_int = 0;
const REMAP_NONE: c_int = -1;

const DIP_ALL: c_int = 0x01;
const DIP_DIRFILE: c_int = 0x200;
const DOSO_NONE: c_int = 0;

const kCdScopeGlobal: c_int = 3;

const DOBUF_GOTO: c_int = 0;
const DOBUF_FIRST: c_int = 1;

const BLN_LISTED: c_int = 2;
const BLN_NEW: c_int = 8;
const BLN_NOOPT: c_int = 16;

const BCO_ENTER: c_int = 1;
const BCO_NOHELP: c_int = 4;

const OPT_LOCAL: c_int = 2;
const kOptScopeBuf: c_int = 2;
const kOptBufhidden: c_int = 27;
const kOptBuftype: c_int = 29;
const kOptCotFlagPopup: c_uint = 0x100;

const LUA_NOREF: LuaRef = -2;
const kChannelStreamInternal: ChannelStreamType = 4;

const kRetObject: c_int = 0;
const kRetNilBool: c_int = 1;

const kNone: c_int = -1;
const kFalse: c_int = 0;
const kTrue: c_int = 1;

const kCtxRegs: CtxStateFlags = 1;
const kCtxJumps: CtxStateFlags = 2;
const kCtxBufs: CtxStateFlags = 4;
const kCtxGVars: CtxStateFlags = 8;
const kCtxSFuncs: CtxStateFlags = 16;
const kCtxFuncs: CtxStateFlags = 32;
const kCtxAll: CtxStateFlags = kCtxRegs | kCtxJumps | kCtxBufs | kCtxGVars | kCtxSFuncs | kCtxFuncs;

const INTERNAL_CALL_MASK: u64 = 1u64 << 63;

const EVENT_CHANINFO: c_int = 23;

const UPD_VALID: c_int = 10;
const UPD_NOT_VALID: c_int = 40;
const UPD_CLEAR: c_int = 50;

const VV_LNUM: c_int = 9;
const VV_RELNUM: c_int = 102;
const VV_VIRTNUM: c_int = 103;

const HLF_N: c_int = 12;
const HLF_LNA: c_int = 13;
const HLF_LNB: c_int = 14;
const HLF_CLN: c_int = 15;
const HLF_CLS: c_int = 16;
const HLF_SC: c_int = 35;

const STL_FOLDCOL: c_int = b'C' as c_int;
const STL_SIGNCOL: c_int = b's' as c_int;

const KEYSET_OPTIDX_highlight__update: u32 = 14;
const KEYSET_OPTIDX_highlight__url: u32 = 5;
const KEYSET_OPTIDX_get_ns__winid: u32 = 1;
const KEYSET_OPTIDX_echo_opts__percent: u32 = 8;
const KEYSET_OPTIDX_open_term__on_input: u32 = 1;
const KEYSET_OPTIDX_open_term__force_crlf: u32 = 2;
const KEYSET_OPTIDX_context__types: u32 = 1;
const KEYSET_OPTIDX_chan_set__detach: u32 = 1;
const KEYSET_OPTIDX_eval_statusline__fillchar: u32 = 2;
const KEYSET_OPTIDX_eval_statusline__maxwidth: u32 = 3;
const KEYSET_OPTIDX_eval_statusline__use_statuscol_lnum: u32 = 7;
const KEYSET_OPTIDX_complete_set__info: u32 = 1;
const KEYSET_OPTIDX_redraw__buf: u32 = 1;
const KEYSET_OPTIDX_redraw__win: u32 = 2;
const KEYSET_OPTIDX_redraw__flush: u32 = 3;
const KEYSET_OPTIDX_redraw__range: u32 = 4;
const KEYSET_OPTIDX_redraw__valid: u32 = 5;

const INIT_POS: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};

const INIT_FMARK: crate::eval::deprecated::fmark_T = crate::eval::deprecated::fmark_T {
    mark: INIT_POS,
    fnum: 0,
    timestamp: 0,
    view: crate::eval::deprecated::fmarkv_T {
        topline_offset: 0x7fffffff,
        skipcol: 0,
    },
    additional_data: ptr::null_mut(),
};

const CONTEXT_INIT: Context = Context {
    pos: INIT_POS,
    mark: 0,
    curswant: -1,
    visual_mode_eval: 0,
    visual: crate::eval::deprecated::visualinfo_T {
        vi_start: INIT_POS,
        vi_end: INIT_POS,
        vi_mode: 0,
        vi_curswant: 0,
    },
    op_start: INIT_POS,
    op_end: INIT_POS,
    last_change: INIT_FMARK,
    last_insert: INIT_FMARK,
    pos_marks: [0; 6],
    buf: 0,
    regs: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    jumps: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    bufs: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    gvars: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    funcs: Array::new(),
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_get_ns {
    pub is_set__get_ns_: OptionalKeys,
    pub winid: Window,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_runtime {
    pub is_lua: Boolean,
    pub do_source: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_echo_opts {
    pub is_set__echo_opts_: OptionalKeys,
    pub err: Boolean,
    pub verbose: Boolean,
    pub _truncate: Boolean,
    pub kind: NvimString,
    pub id: Object,
    pub title: NvimString,
    pub status: NvimString,
    pub percent: Integer,
    pub source: NvimString,
    pub data: Dict,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_open_term {
    pub is_set__open_term_: OptionalKeys,
    pub on_input: LuaRef,
    pub force_crlf: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_context {
    pub is_set__context_: OptionalKeys,
    pub types: Array,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_chan_set {
    pub is_set__chan_set_: OptionalKeys,
    pub detach: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_eval_statusline {
    pub is_set__eval_statusline_: OptionalKeys,
    pub winid: Window,
    pub maxwidth: Integer,
    pub fillchar: NvimString,
    pub highlights: Boolean,
    pub use_winbar: Boolean,
    pub use_tabline: Boolean,
    pub use_statuscol_lnum: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_complete_set {
    pub is_set__complete_set_: OptionalKeys,
    pub info: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_redraw {
    pub is_set__redraw_: OptionalKeys,
    pub flush: Boolean,
    pub cursor: Boolean,
    pub valid: Boolean,
    pub statuscolumn: Boolean,
    pub statusline: Boolean,
    pub tabline: Boolean,
    pub winbar: Boolean,
    pub range: Array,
    pub win: Window,
    pub buf: Buffer,
}

pub use crate::api::private::helpers::TryState;

pub use crate::mapping::typebuf_T;

pub use crate::api::private::helpers::HlMessageChunk;

pub type HlMessage = crate::kvec::KVec<HlMessageChunk>;
pub use crate::api::private::helpers::ArrayBuilder;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MessageData {
    pub source: NvimString,
    pub percent: Integer,
    pub title: NvimString,
    pub status: NvimString,
    pub data: Dict,
}

#[repr(C)]
struct RuntimeCookie {
    rv: ArrayBuilder,
    arena: *mut Arena,
}

static mut PASTE_CANCELLED: bool = false;

#[inline]
unsafe fn validate_int(cond: bool, name: *const c_char, val: i64, err: *mut Error) -> bool {
    if !cond {
        unsafe {
            api_err_invalid(err, name, ptr::null(), val, false);
        }
        false
    } else {
        true
    }
}

#[inline]
unsafe fn validate_s(cond: bool, name: *const c_char, val: *const c_char, err: *mut Error) -> bool {
    if !cond {
        unsafe {
            api_err_invalid(err, name, val, 0, true);
        }
        false
    } else {
        true
    }
}

#[inline]
unsafe fn validate_r(cond: bool, name: *const c_char, err: *mut Error) -> bool {
    if !cond {
        unsafe {
            api_err_required(err, name);
        }
        false
    } else {
        true
    }
}

#[inline]
unsafe fn validate_range(cond: bool, name: *const c_char, err: *mut Error) -> bool {
    if !cond {
        unsafe {
            api_err_invalid(err, name, c"out of range".as_ptr(), 0, false);
        }
        false
    } else {
        true
    }
}

#[inline]
unsafe fn validate_t(
    name: *const c_char,
    expected_t: ObjectType,
    actual_t: ObjectType,
    err: *mut Error,
) -> bool {
    if expected_t != actual_t {
        unsafe {
            api_err_exp(err, name, api_typename(expected_t), api_typename(actual_t));
        }
        false
    } else {
        true
    }
}

#[inline]
const fn error_set(e: &Error) -> bool {
    !matches!(e.err_type, ErrorType::kErrorTypeNone)
}

#[inline]
const fn is_internal_call(channel_id: u64) -> bool {
    (channel_id & INTERNAL_CALL_MASK) != 0
}

#[inline]
unsafe fn cstr_to_arena_obj(arena: *mut Arena, s: *const c_char) -> Object {
    unsafe { string_obj(arena_string(arena, cstr_as_string(s))) }
}

#[inline]
unsafe fn put_c(dict: &mut Dict, key: *const c_char, val: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: cstr_as_string(key),
            value: val,
        };
        dict.size += 1;
    }
}

/// Gets a highlight group by name (`nvim_get_hl_id_by_name`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_hl_id_by_name(name: NvimString, err: *mut Error) -> Integer {
    unsafe {
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        let ret = syn_check_group(name.data, name.size);
        try_leave(&tstate, err);
        Integer::from(ret)
    }
}

/// Gets all or specific highlight groups in a namespace (`nvim_get_hl`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_hl(
    ns_id: Integer,
    opts: *mut KeyDict_get_highlight,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe { ns_get_hl_defs(ns_id as NS, opts, arena, err) }
}

/// Sets a highlight group (`nvim_set_hl`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_hl(
    channel_id: u64,
    ns_id: Integer,
    name: NvimString,
    val: *mut KeyDict_highlight,
    err: *mut Error,
) {
    unsafe {
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        let hl_id = syn_check_group(name.data, name.size);
        try_leave(&tstate, err);
        if error_set(&*err) || hl_id == 0 {
            return;
        }
        let mut link_id: c_int = -1;

        if ((*val).is_set__highlight_ & (1u64 << KEYSET_OPTIDX_highlight__url)) != 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Invalid key: 'url'".as_ptr(),
            );
            return;
        }

        let update = ((*val).is_set__highlight_ & (1u64 << KEYSET_OPTIDX_highlight__update)) != 0
            && (*val).update;
        let mut base: *mut HlAttrs = ptr::null_mut();
        let mut base_attrs = std::mem::zeroed::<HlAttrs>();
        if update && hl_ns_get_attrs(ns_id as c_int, hl_id, ptr::null_mut(), &mut base_attrs) {
            base = &mut base_attrs;
        }

        let attrs = dict2hlattrs(val, true, &mut link_id, base, err);
        if !error_set(&*err) {
            let save_current_sctx = api_set_sctx(channel_id);
            ns_hl_def(ns_id as NS, hl_id, attrs, link_id, val);
            current_sctx = save_current_sctx;
        }
    }
}

/// Gets the active highlight namespace (`nvim_get_hl_ns`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_hl_ns(opts: *mut KeyDict_get_ns, err: *mut Error) -> Integer {
    unsafe {
        if ((*opts).is_set__get_ns_ & (1u64 << KEYSET_OPTIDX_get_ns__winid)) != 0 {
            let win = find_window_by_handle((*opts).winid, err);
            if win.is_null() {
                return 0;
            }
            Integer::from((*win).w_ns_hl)
        } else {
            Integer::from(ns_hl_global)
        }
    }
}

/// Set active namespace for highlights (`nvim_set_hl_ns`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_hl_ns(ns_id: Integer, err: *mut Error) {
    unsafe {
        if !validate_int(ns_id >= 0, c"namespace".as_ptr(), ns_id, err) {
            return;
        }
        ns_hl_global = ns_id as NS;
        hl_check_ns();
        redraw_all_later(UPD_NOT_VALID);
    }
}

/// Set active namespace for highlights while redrawing (`nvim_set_hl_ns_fast`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_hl_ns_fast(ns_id: Integer, _err: *mut Error) {
    unsafe {
        ns_hl_fast = ns_id as NS;
        hl_check_ns();
    }
}

/// Sends input-keys to Nvim (`nvim_feedkeys`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_feedkeys(keys: NvimString, mode: NvimString, escape_ks: Boolean) {
    unsafe {
        let mut remap = true;
        let mut insert = false;
        let mut typed = false;
        let mut execute = false;
        let mut dangerous = false;
        let mut lowlevel = false;

        for i in 0..mode.size {
            match *mode.data.add(i) as u8 {
                b'n' => remap = false,
                b'm' => remap = true,
                b't' => typed = true,
                b'i' => insert = true,
                b'x' => execute = true,
                b'!' => dangerous = true,
                b'L' => lowlevel = true,
                _ => {}
            }
        }

        if keys.size == 0 && !execute {
            return;
        }

        let keys_esc = if escape_ks {
            vim_strsave_escape_ks(keys.data)
        } else {
            keys.data
        };
        if lowlevel {
            input_enqueue_raw(keys_esc, CStr::from_ptr(keys_esc).to_bytes().len());
        } else {
            ins_typebuf(
                keys_esc,
                if remap { REMAP_YES } else { REMAP_NONE },
                if insert { 0 } else { typebuf.tb_len },
                !typed,
                false,
            );
            if vgetc_busy != 0 {
                typebuf_was_filled = true;
            }
        }

        if escape_ks {
            xfree(keys_esc.cast::<c_void>());
        }

        if execute {
            let save_msg_scroll = msg_scroll;
            msg_scroll = 0;
            if !dangerous {
                ex_normal_busy += 1;
            }
            exec_normal(true, lowlevel);
            if !dangerous {
                ex_normal_busy -= 1;
            }
            msg_scroll |= save_msg_scroll;
        }
    }
}

/// Queues raw user-input (`nvim_input`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_input(channel_id: u64, keys: NvimString) -> Integer {
    unsafe {
        may_trigger_vim_suspend_resume(false);
        input_enqueue(channel_id, keys) as Integer
    }
}

/// Send mouse event from GUI (`nvim_input_mouse`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_input_mouse(
    button: NvimString,
    action: NvimString,
    modifier: NvimString,
    grid: Integer,
    row: Integer,
    col: Integer,
    err: *mut Error,
) {
    unsafe {
        may_trigger_vim_suspend_resume(false);

        let valid = 'check: {
            if button.data.is_null() || action.data.is_null() {
                break 'check None;
            }

            let mut code: c_int = if strequal(button.data, c"left".as_ptr()) {
                c_int::from(KE_LEFTMOUSE)
            } else if strequal(button.data, c"middle".as_ptr()) {
                c_int::from(KE_MIDDLEMOUSE)
            } else if strequal(button.data, c"right".as_ptr()) {
                c_int::from(KE_RIGHTMOUSE)
            } else if strequal(button.data, c"wheel".as_ptr()) {
                c_int::from(KE_MOUSEDOWN)
            } else if strequal(button.data, c"x1".as_ptr()) {
                c_int::from(KE_X1MOUSE)
            } else if strequal(button.data, c"x2".as_ptr()) {
                c_int::from(KE_X2MOUSE)
            } else if strequal(button.data, c"move".as_ptr()) {
                c_int::from(KE_MOUSEMOVE)
            } else {
                break 'check None;
            };

            if code == c_int::from(KE_MOUSEDOWN) {
                if strequal(action.data, c"down".as_ptr()) {
                    code = c_int::from(KE_MOUSEUP);
                } else if strequal(action.data, c"up".as_ptr()) {
                    // code = KE_MOUSEDOWN
                } else if strequal(action.data, c"left".as_ptr()) {
                    code = c_int::from(KE_MOUSERIGHT);
                } else if strequal(action.data, c"right".as_ptr()) {
                    code = c_int::from(KE_MOUSELEFT);
                } else {
                    break 'check None;
                }
            } else if code != c_int::from(KE_MOUSEMOVE) {
                if strequal(action.data, c"press".as_ptr()) {
                    // pass
                } else if strequal(action.data, c"drag".as_ptr()) {
                    code += c_int::from(KE_LEFTDRAG) - c_int::from(KE_LEFTMOUSE);
                } else if strequal(action.data, c"release".as_ptr()) {
                    code += c_int::from(KE_LEFTRELEASE) - c_int::from(KE_LEFTMOUSE);
                } else {
                    break 'check None;
                }
            }

            Some(code)
        };

        let Some(code) = valid else {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"invalid button or action".as_ptr(),
            );
            return;
        };

        let mut modmask: c_int = 0;
        for i in 0..modifier.size {
            let byte = *modifier.data.add(i);
            if byte == (b'-' as c_char) {
                continue;
            }
            let m = name_to_mod_mask(c_int::from(byte as u8));
            if m == 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid modifier: %c".as_ptr(),
                    c_int::from(byte),
                );
                return;
            }
            modmask |= m;
        }

        input_enqueue_mouse(
            code,
            modmask as u8,
            grid as c_int,
            row as c_int,
            col as c_int,
        );
    }
}

/// Converts terminal codes and keycodes in a key sequence (`nvim_replace_termcodes`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_replace_termcodes(
    str_: NvimString,
    from_part: Boolean,
    do_lt: Boolean,
    special: Boolean,
) -> NvimString {
    if str_.size == 0 {
        return NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
    }

    let mut flags: c_int = 0;
    if from_part {
        flags |= REPTERM_FROM_PART;
    }
    if do_lt {
        flags |= REPTERM_DO_LT;
    }
    if !special {
        flags |= REPTERM_NO_SPECIAL;
    }

    unsafe {
        let mut ptr_out: *mut c_char = ptr::null_mut();
        replace_termcodes(
            str_.data,
            str_.size,
            &mut ptr_out,
            0,
            flags,
            ptr::null_mut(),
            p_cpo,
        );
        cstr_as_string(ptr_out)
    }
}

/// Executes Lua code (`nvim_exec_lua`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_exec_lua(
    code: NvimString,
    args: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { nlua_exec(code, ptr::null(), args, kRetObject, arena, err) }
}

/// Executes Lua code during api-fast contexts (`nvim__exec_lua_fast`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__exec_lua_fast(
    code: NvimString,
    args: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { nvim_exec_lua(code, args, arena, err) }
}

/// Calculates the number of display cells occupied by `text` (`nvim_strwidth`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_strwidth(text: NvimString, err: *mut Error) -> Integer {
    unsafe {
        if !validate_s(
            text.size <= (c_int::MAX as usize),
            c"text length".as_ptr(),
            c"(too long)".as_ptr(),
            err,
        ) {
            return 0;
        }
        mb_string2cells(text.data) as Integer
    }
}

/// Gets the paths contained in runtime-search-path (`nvim_list_runtime_paths`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_list_runtime_paths(arena: *mut Arena, err: *mut Error) -> Array {
    unsafe {
        nvim_get_runtime_file(
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            },
            true,
            arena,
            err,
        )
    }
}

/// Inspect runtime (`nvim__runtime_inspect`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__runtime_inspect(arena: *mut Arena) -> Array {
    unsafe { runtime_inspect(arena) }
}

unsafe extern "C" fn find_runtime_cb(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    c: *mut c_void,
) -> bool {
    unsafe {
        let cookie = &mut *c.cast::<RuntimeCookie>();
        for i in 0..(num_fnames as usize) {
            cookie
                .rv
                .push(cstr_to_arena_obj(cookie.arena, *fnames.add(i)));
            if !all {
                return true;
            }
        }
        num_fnames > 0
    }
}

/// Finds files in runtime directories (`nvim_get_runtime_file`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_runtime_file(
    name: NvimString,
    all: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut cookie = std::mem::MaybeUninit::<RuntimeCookie>::zeroed();
        let cookie_ptr = cookie.as_mut_ptr();
        (*cookie_ptr).rv.init();
        (*cookie_ptr).arena = arena;

        let flags = DIP_DIRFILE | if all { DIP_ALL } else { 0 };
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        do_in_runtimepath(
            if name.size != 0 {
                name.data
            } else {
                c"".as_ptr().cast_mut()
            },
            flags,
            Some(find_runtime_cb),
            cookie_ptr.cast::<c_void>(),
        );
        try_leave(&tstate, err);

        arena_take_arraybuilder(arena, &mut (*cookie_ptr).rv)
    }
}

/// Get lib dir (`nvim__get_lib_dir`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__get_lib_dir() -> NvimString {
    unsafe { cstr_as_string(get_lib_dir()) }
}

/// Find files in runtime directories (`nvim__get_runtime`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__get_runtime(
    pat: Array,
    all: Boolean,
    opts: *mut KeyDict_runtime,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        if (*opts).do_source && !nlua_is_deferred_safe() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'do_source' used in fast callback".as_ptr(),
            );
        }
        if error_set(&*err) {
            return Array::new();
        }

        let res = runtime_get_named((*opts).is_lua, pat, all, arena);
        if (*opts).do_source {
            for i in 0..res.size {
                let name = (*res.items.add(i)).data.string;
                do_source(name.data, false, DOSO_NONE, ptr::null_mut());
            }
        }
        res
    }
}

/// Changes the global working directory (`nvim_set_current_dir`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_current_dir(dir: NvimString, err: *mut Error) {
    unsafe {
        if !validate_s(
            dir.size < MAXPATHL,
            c"directory name".as_ptr(),
            c"(too long)".as_ptr(),
            err,
        ) {
            return;
        }
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        do_chdir(dir.data, kCdScopeGlobal);
        try_leave(&tstate, err);
    }
}

/// Gets the current line (`nvim_get_current_line`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_current_line(arena: *mut Arena, err: *mut Error) -> NvimString {
    unsafe {
        buffer_get_line(
            (*curbuf).b_fnum,
            Integer::from((*curwin).w_cursor.lnum - 1),
            arena,
            err,
        )
    }
}

/// Sets the text on the current line (`nvim_set_current_line`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_current_line(
    line: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        buffer_set_line(
            (*curbuf).b_fnum,
            Integer::from((*curwin).w_cursor.lnum - 1),
            line,
            arena,
            err,
        );
    }
}

/// Deletes the current line (`nvim_del_current_line`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_del_current_line(arena: *mut Arena, err: *mut Error) {
    unsafe {
        buffer_del_line(
            (*curbuf).b_fnum,
            Integer::from((*curwin).w_cursor.lnum - 1),
            arena,
            err,
        );
    }
}

/// Gets a global (g:) variable (`nvim_get_var`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_var(
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut di = tv_dict_find(get_globvar_dict(), name.data, name.size as isize);
        if di.is_null() {
            let found = script_autoload(name.data, name.size, false) && !aborting();
            if !found {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Key not found: %s".as_ptr(),
                    name.data,
                );
                return nil_obj();
            }
            di = tv_dict_find(get_globvar_dict(), name.data, name.size as isize);
        }
        if di.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Key not found: %s".as_ptr(),
                name.data,
            );
            return nil_obj();
        }
        vim_to_object(&mut (*di).di_tv, arena, true)
    }
}

/// Sets a global (g:) variable (`nvim_set_var`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_var(name: NvimString, value: Object, err: *mut Error) {
    unsafe {
        dict_set_var(
            get_globvar_dict(),
            name,
            value,
            false,
            false,
            ptr::null_mut(),
            err,
        );
    }
}

/// Removes a global (g:) variable (`nvim_del_var`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_del_var(name: NvimString, err: *mut Error) {
    unsafe {
        dict_set_var(
            get_globvar_dict(),
            name,
            nil_obj(),
            true,
            false,
            ptr::null_mut(),
            err,
        );
    }
}

/// Gets a v: variable (`nvim_get_vvar`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_vvar(
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { dict_get_value(get_vimvar_dict(), name, arena, err) }
}

/// Sets a v: variable (`nvim_set_vvar`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_vvar(name: NvimString, value: Object, err: *mut Error) {
    unsafe {
        dict_set_var(
            get_vimvar_dict(),
            name,
            value,
            false,
            false,
            ptr::null_mut(),
            err,
        );
    }
}

/// Prints a message given by a list of `[text, hl_group]` chunks (`nvim_echo`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_echo(
    chunks: Array,
    history: Boolean,
    opts: *mut KeyDict_echo_opts,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut id = integer_obj(-1);
        let hl_msg = parse_hl_msg(chunks, (*opts).err, err);
        if error_set(&*err) {
            hl_msg_free(hl_msg);
            return id;
        }

        let mut kind = (*opts).kind.data.cast_const();
        if (*opts).verbose {
            verbose_enter();
        } else if kind.is_null() {
            kind = if (*opts).err {
                c"echoerr".as_ptr()
            } else if history {
                c"echomsg".as_ptr()
            } else {
                c"echo".as_ptr()
            };
        }

        let is_progress = strequal(kind, c"progress".as_ptr());
        let mut needs_clear = !history;
        let has_percent =
            ((*opts).is_set__echo_opts_ & (1u64 << KEYSET_OPTIDX_echo_opts__percent)) != 0;

        let valid = 'validate: {
            if !is_progress
                && ((*opts).status.size != 0
                    || (*opts).title.size != 0
                    || has_percent
                    || (*opts).data.size != 0
                    || (*opts).source.size != 0)
            {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Conflict: title/source/status/percent/data not allowed with kind='%s'"
                        .as_ptr(),
                    kind,
                );
                break 'validate false;
            }

            if is_progress
                && !(strequal((*opts).status.data, c"success".as_ptr())
                    || strequal((*opts).status.data, c"failed".as_ptr())
                    || strequal((*opts).status.data, c"running".as_ptr())
                    || strequal((*opts).status.data, c"cancel".as_ptr()))
            {
                api_err_exp(
                    err,
                    c"status".as_ptr(),
                    c"success|failed|running|cancel".as_ptr(),
                    (*opts).status.data,
                );
                break 'validate false;
            }

            if !validate_range(
                !is_progress || !has_percent || ((*opts).percent >= 0 && (*opts).percent <= 100),
                c"percent".as_ptr(),
                err,
            ) {
                break 'validate false;
            }

            if !validate_r(
                !is_progress || (*opts).source.size != 0,
                c"opts.source".as_ptr(),
                err,
            ) {
                break 'validate false;
            }

            if !validate_s(
                !is_progress || !strequal((*opts).source.data, c"nvim".as_ptr()),
                c"source".as_ptr(),
                (*opts).source.data,
                err,
            ) {
                break 'validate false;
            }

            if !validate_int(
                (*opts).id.obj_type != ObjectType::kObjectTypeInteger
                    || msg_id_exists((*opts).id.data.integer),
                c"id".as_ptr(),
                (*opts).id.data.integer,
                err,
            ) {
                break 'validate false;
            }

            true
        };

        if !valid {
            hl_msg_free(hl_msg);
            return id;
        }

        let mut msg_data = MessageData {
            source: (*opts).source,
            percent: if has_percent { (*opts).percent } else { -1 },
            title: (*opts).title,
            status: (*opts).status,
            data: (*opts).data,
        };

        let save_nwr = need_wait_return;
        let save_lines_left = lines_left;
        let save_msg_didany = msg_didany;
        if (*opts)._truncate {
            no_wait_return += 1;
            lines_left = 0;
            msg_didany = true;
            msg_no_more = true;
        }
        msg_ext_no_fast();
        id = msg_multihl(
            (*opts).id,
            hl_msg,
            kind,
            history,
            (*opts).err,
            &mut msg_data,
            &mut needs_clear,
            false,
        );
        if (*opts)._truncate {
            msg_no_more = false;
            msg_didany = save_msg_didany;
            lines_left = save_lines_left;
            no_wait_return -= 1;
            need_wait_return = save_nwr;
        }

        if (*opts).verbose {
            verbose_leave();
            verbose_stop();
        }

        if !needs_clear {
            return id;
        }

        hl_msg_free(hl_msg);
        id
    }
}

/// Gets the current list of buffers (`nvim_list_bufs`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_list_bufs(arena: *mut Arena) -> Array {
    unsafe {
        let mut n = 0usize;
        let mut b = firstbuf;
        while !b.is_null() {
            n += 1;
            b = (*b).b_next;
        }

        let mut rv = arena_array(arena, n);
        b = firstbuf;
        while !b.is_null() {
            add_c(&mut rv, buffer_obj((*b).b_fnum));
            b = (*b).b_next;
        }
        rv
    }
}

/// Gets the current buffer (`nvim_get_current_buf`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_current_buf() -> Buffer {
    unsafe { (*curbuf).b_fnum }
}

/// Sets the current window's buffer to `buf` (`nvim_set_current_buf`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_current_buf(buf: Buffer, err: *mut Error) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        do_buffer(DOBUF_GOTO, DOBUF_FIRST, FORWARD, (*b).b_fnum, 0);
        try_leave(&tstate, err);
    }
}

/// Gets the current list of all window-IDs in all tabpages (`nvim_list_wins`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_list_wins(arena: *mut Arena) -> Array {
    unsafe {
        let mut n = 0usize;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                n += 1;
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        let mut rv = arena_array(arena, n);
        tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                add_c(&mut rv, window_obj((*wp).handle));
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        rv
    }
}

/// Gets the current window (`nvim_get_current_win`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_current_win() -> Window {
    unsafe { (*curwin).handle }
}

/// Navigates to the given window (`nvim_set_current_win`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_current_win(win: Window, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        if (*w).w_buffer != curbuf {
            reset_VIsual_and_resel();
        }
        goto_tabpage_win(win_find_tabpage(w), w);
        try_leave(&tstate, err);
    }
}

/// Creates a new, empty, unnamed buffer (`nvim_create_buf`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_create_buf(
    listed: Boolean,
    scratch: Boolean,
    err: *mut Error,
) -> Buffer {
    unsafe {
        let mut ret: Buffer = 0;
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        'create: {
            block_autocmds();
            let buf = buflist_new(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                BLN_NOOPT | BLN_NEW | if listed { BLN_LISTED } else { 0 },
            );
            if buf.is_null() {
                unblock_autocmds();
                break 'create;
            }
            if ml_open(buf) == FAIL {
                unblock_autocmds();
                break 'create;
            }
            let tick = buf_get_changedtick(buf);
            (*buf).b_last_changedtick = tick;
            (*buf).b_last_changedtick_i = tick;
            (*buf).b_last_changedtick_pum = tick;

            buf_copy_options(buf, BCO_ENTER | BCO_NOHELP);

            if scratch {
                set_option_direct_for(
                    kOptBufhidden,
                    string_obj(static_cstr_as_string(c"hide")),
                    OPT_LOCAL,
                    0,
                    kOptScopeBuf,
                    buf.cast::<c_void>(),
                );
                set_option_direct_for(
                    kOptBuftype,
                    string_obj(static_cstr_as_string(c"nofile")),
                    OPT_LOCAL,
                    0,
                    kOptScopeBuf,
                    buf.cast::<c_void>(),
                );
                assert!((*(*buf).b_ml.ml_mfp.cast::<memfile_T>()).mf_fd < 0);
                (*buf).b_p_swf = 0;
                (*buf).b_p_ml = 0;
            }

            unblock_autocmds();

            let mut bufref = std::mem::zeroed::<bufref_T>();
            set_bufref(&mut bufref, buf);
            if apply_autocmds(EVENT_BUFNEW, ptr::null_mut(), ptr::null_mut(), false, buf)
                && !bufref_valid(&mut bufref)
            {
                break 'create;
            }
            if listed
                && apply_autocmds(EVENT_BUFADD, ptr::null_mut(), ptr::null_mut(), false, buf)
                && !bufref_valid(&mut bufref)
            {
                break 'create;
            }

            ret = (*buf).b_fnum;
        }
        try_leave(&tstate, err);

        if ret == 0 && !error_set(&*err) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to create buffer".as_ptr(),
            );
        }
        ret
    }
}

unsafe extern "C" fn term_read_pause(_pause: bool, _data: *mut c_void) {}

unsafe extern "C" fn term_write(buf: *const c_char, size: usize, data: *mut c_void) {
    unsafe {
        let chan = data.cast::<Channel>();
        let internal = channel_stream_ptr(chan).cast::<InternalState>();
        let cb = (*internal).cb;
        if cb == LUA_NOREF {
            return;
        }
        let prefix = channel_prefix(chan);
        let tail = channel_tail(chan);
        let mut args_items = [
            integer_obj((*prefix).id as Integer),
            buffer_obj(terminal_buf((*tail).term.cast())),
            string_obj(NvimString {
                data: buf.cast_mut(),
                size,
            }),
        ];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        textlock += 1;
        nlua_call_ref(
            cb,
            c"input".as_ptr(),
            args,
            kRetNilBool,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        textlock -= 1;
    }
}

unsafe extern "C" fn term_resize(_width: u16, _height: u16, _data: *mut c_void) {}

unsafe extern "C" fn term_resume(_data: *mut c_void) {}

unsafe extern "C" fn term_close(data: *mut c_void) {
    unsafe {
        let chan = data.cast::<Channel>();
        let internal = channel_stream_ptr(chan).cast::<InternalState>();
        let tail = channel_tail(chan);
        terminal_destroy((&raw mut (*tail).term).cast());
        api_free_luaref((*internal).cb);
        (*internal).cb = LUA_NOREF;
        channel_decref(chan);
    }
}

/// Open a terminal instance in a buffer (`nvim_open_term`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_open_term(
    buf: Buffer,
    opts: *mut KeyDict_open_term,
    err: *mut Error,
) -> Integer {
    unsafe {
        let b = api_buf_ensure_loaded(buf, err);
        if b.is_null() {
            return 0;
        }

        let mut may_read_buffer = true;
        if !(*b).terminal.is_null() {
            if terminal_running((*b).terminal.cast()) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Terminal already connected to buffer %d".as_ptr(),
                    (*b).b_fnum,
                );
                return 0;
            }
            buf_close_terminal(b);
            may_read_buffer = false;
        }

        let mut cb = LUA_NOREF;
        if ((*opts).is_set__open_term_ & (1u64 << KEYSET_OPTIDX_open_term__on_input)) != 0 {
            cb = (*opts).on_input;
            (*opts).on_input = LUA_NOREF;
        }

        let force_crlf =
            if ((*opts).is_set__open_term_ & (1u64 << KEYSET_OPTIDX_open_term__force_crlf)) != 0 {
                (*opts).force_crlf
            } else {
                true
            };

        let chan = channel_alloc(kChannelStreamInternal);
        let internal = channel_stream_ptr(chan).cast::<InternalState>();
        let prefix = channel_prefix(chan);
        let tail = channel_tail(chan);
        (*internal).cb = cb;
        (*internal).closed = false;
        let topts = TerminalOptions {
            data: chan.cast::<c_void>(),
            width: ((*curwin).w_view_width - win_col_off(curwin)).max(0) as u16,
            height: (*curwin).w_view_height as u16,
            read_pause_cb: Some(term_read_pause),
            write_cb: Some(term_write),
            resize_cb: Some(term_resize),
            resume_cb: Some(term_resume),
            close_cb: Some(term_close),
            force_crlf,
        };

        let mut contents = StringBuilder::new();
        if may_read_buffer {
            read_buffer_into(b, 1, (*b).b_ml.ml_line_count, &mut contents);
        }

        channel_incref(chan);
        (*tail).term = terminal_alloc(b, topts);
        terminal_open((&raw mut (*tail).term).cast(), b);
        if !(*tail).term.is_null() {
            terminal_check_size((*tail).term.cast());
        }
        channel_decref(chan);

        if contents.size > 0 {
            let mut error: *const c_char = ptr::null();
            channel_send(
                (*prefix).id,
                contents.items,
                contents.size,
                true,
                &mut error,
            );
            if !error.is_null() {
                api_set_error!(err, ErrorType::kErrorTypeValidation, c"%s".as_ptr(), error);
            }
        }

        (*prefix).id as Integer
    }
}

/// Sends raw data to channel `chan` (`nvim_chan_send`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_chan_send(chan: Integer, data: NvimString, err: *mut Error) {
    if data.size == 0 {
        return;
    }
    unsafe {
        let mut error: *const c_char = ptr::null();
        channel_send(chan as u64, data.data, data.size, false, &mut error);
        if !error.is_null() {
            api_set_error!(err, ErrorType::kErrorTypeValidation, c"%s".as_ptr(), error);
        }
    }
}

/// Gets the current list of tab-IDs (`nvim_list_tabpages`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_list_tabpages(arena: *mut Arena) -> Array {
    unsafe {
        let mut n = 0usize;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            n += 1;
            tp = (*tp).tp_next;
        }

        let mut rv = arena_array(arena, n);
        tp = first_tabpage;
        while !tp.is_null() {
            add_c(&mut rv, tabpage_obj((*tp).handle));
            tp = (*tp).tp_next;
        }
        rv
    }
}

/// Gets the current tabpage (`nvim_get_current_tabpage`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_current_tabpage() -> Tabpage {
    unsafe { (*curtab).handle }
}

/// Sets the current tabpage (`nvim_set_current_tabpage`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_current_tabpage(tabpage: Tabpage, err: *mut Error) {
    unsafe {
        let tp = find_tab_by_handle(tabpage, err);
        if tp.is_null() {
            return;
        }
        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        goto_tabpage_tp(tp, true, true);
        try_leave(&tstate, err);
    }
}

/// Pastes at cursor (`nvim_paste`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_paste(
    channel_id: u64,
    data: NvimString,
    crlf: Boolean,
    phase: Integer,
    arena: *mut Arena,
    err: *mut Error,
) -> Boolean {
    unsafe {
        if !validate_int((-1..=3).contains(&phase), c"phase".as_ptr(), phase, err) {
            return false;
        }
        if phase == -1 || phase == 1 {
            PASTE_CANCELLED = false;
            if !(*curbuf).terminal.is_null() {
                terminal_set_streamed_paste((*curbuf).terminal.cast(), true);
            }
        } else if PASTE_CANCELLED {
            let retval = !PASTE_CANCELLED;
            if phase == 3 {
                PASTE_CANCELLED = false;
            }
            return retval;
        }

        let lines = string_to_array(data, crlf, arena);
        let mut args_items = [array_obj(lines), integer_obj(phase)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        let rv = nlua_exec(
            static_cstr_as_string(c"return vim.paste(...)"),
            ptr::null(),
            args,
            kRetNilBool,
            arena,
            err,
        );
        if error_set(&*err) || (rv.obj_type == ObjectType::kObjectTypeBoolean && !rv.data.boolean) {
            PASTE_CANCELLED = true;
        }
        if (phase == -1 || phase == 3 || PASTE_CANCELLED) && !(*curbuf).terminal.is_null() {
            terminal_set_streamed_paste((*curbuf).terminal.cast(), false);
        }
        let null_str = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        if !PASTE_CANCELLED && (phase == -1 || phase == 1) {
            paste_store(channel_id, kFalse, null_str, crlf);
        }
        if !PASTE_CANCELLED {
            paste_store(channel_id, kNone, data, crlf);
        }
        if phase == 3 || phase == (if PASTE_CANCELLED { 2 } else { -1 }) {
            paste_store(channel_id, kTrue, null_str, crlf);
        }

        let retval = !PASTE_CANCELLED;
        if phase == -1 || phase == 3 {
            PASTE_CANCELLED = false;
        }
        retval
    }
}

/// Puts text at cursor (`nvim_put`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_put(
    lines: Array,
    reg_type: NvimString,
    after: Boolean,
    follow: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        let mut reg = std::mem::zeroed::<yankreg_T>();
        if !validate_s(
            prepare_yankreg_from_object(&mut reg, reg_type, lines.size),
            c"type".as_ptr(),
            reg_type.data,
            err,
        ) {
            return;
        }
        if lines.size == 0 {
            return;
        }

        reg.y_array = arena_alloc(arena, lines.size * std::mem::size_of::<NvimString>(), true)
            .cast::<NvimString>();
        reg.y_size = lines.size;
        for i in 0..lines.size {
            let item = *lines.items.add(i);
            if !validate_t(
                c"line".as_ptr(),
                ObjectType::kObjectTypeString,
                item.obj_type,
                err,
            ) {
                return;
            }
            let line = item.data.string;
            let copied = copy_string(line, arena);
            *reg.y_array.add(i) = copied;
            memchrsub(copied.data.cast::<c_void>(), NUL, NL, line.size);
        }

        finish_yankreg_from_object(&mut reg, false);

        let mut tstate = std::mem::zeroed::<TryState>();
        try_enter(&mut tstate);
        let visual_was_active = Visual.active;
        msg_silent += 1;
        do_put(
            b'_' as c_int,
            &mut reg,
            if after { FORWARD } else { BACKWARD },
            1,
            if follow { PUT_CURSEND } else { 0 },
        );
        msg_silent -= 1;
        Visual.active = visual_was_active;
        try_leave(&tstate, err);
    }
}

/// Returns the 24-bit RGB value of a color name or hex string (`nvim_get_color_by_name`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_color_by_name(name: NvimString) -> Integer {
    unsafe {
        let mut dummy: c_int = 0;
        Integer::from(name_to_color(name.data, &mut dummy))
    }
}

/// Returns a map of color names and RGB values (`nvim_get_color_map`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_color_map(arena: *mut Arena) -> Dict {
    unsafe {
        let mut colors = arena_dict(arena, color_name_table.len());
        let mut i = 0usize;
        while !color_name_table[i].name.is_null() {
            put_c(
                &mut colors,
                color_name_table[i].name,
                integer_obj(Integer::from(color_name_table[i].color)),
            );
            i += 1;
        }
        colors
    }
}

/// Gets a map of the current editor state (`nvim_get_context`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_context(
    opts: *mut KeyDict_context,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let types = if ((*opts).is_set__context_ & (1u64 << KEYSET_OPTIDX_context__types)) != 0 {
            (*opts).types
        } else {
            Array::new()
        };

        let mut int_types: CtxStateFlags = if types.size > 0 { 0 } else { kCtxAll };
        if types.size > 0 {
            for i in 0..types.size {
                let item = *types.items.add(i);
                if item.obj_type == ObjectType::kObjectTypeString {
                    let s = item.data.string.data;
                    if strequal(s, c"regs".as_ptr()) {
                        int_types |= kCtxRegs;
                    } else if strequal(s, c"jumps".as_ptr()) {
                        int_types |= kCtxJumps;
                    } else if strequal(s, c"bufs".as_ptr()) {
                        int_types |= kCtxBufs;
                    } else if strequal(s, c"gvars".as_ptr()) {
                        int_types |= kCtxGVars;
                    } else if strequal(s, c"sfuncs".as_ptr()) {
                        int_types |= kCtxSFuncs;
                    } else if strequal(s, c"funcs".as_ptr()) {
                        int_types |= kCtxFuncs;
                    } else {
                        let _ = validate_s(false, c"type".as_ptr(), s, err);
                        return Dict::new();
                    }
                }
            }
        }

        let mut ctx = CONTEXT_INIT;
        ctx_save(&mut ctx, int_types);
        let dict = ctx_to_dict(&mut ctx, arena);
        ctx_free(&mut ctx);
        dict
    }
}

/// Sets the current editor state from the given map (`nvim_load_context`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_load_context(dict: Dict, err: *mut Error) -> Object {
    unsafe {
        let mut ctx = CONTEXT_INIT;
        let save_did_emsg = did_emsg;
        did_emsg = 0;

        ctx_from_dict(dict, &mut ctx, err);
        if !error_set(&*err) {
            ctx_load(&mut ctx, kCtxAll, 0);
        }

        ctx_free(&mut ctx);
        did_emsg = save_did_emsg;
        nil_obj()
    }
}

/// Gets the current mode (`nvim_get_mode`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_mode(arena: *mut Arena) -> Dict {
    unsafe {
        let mut rv = arena_dict(arena, 2);
        let modestr = arena_alloc(arena, MODE_MAX_LENGTH, false).cast::<c_char>();
        get_mode(modestr);
        let blocked = input_blocking();

        put_c(&mut rv, c"mode".as_ptr(), cstr_as_obj(modestr));
        put_c(&mut rv, c"blocking".as_ptr(), boolean_obj(blocked));
        rv
    }
}

/// Gets a list of global mapping definitions (`nvim_get_keymap`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_keymap(mode: NvimString, arena: *mut Arena) -> Array {
    unsafe { keymap_array(mode, ptr::null_mut(), arena) }
}

/// Sets a global mapping (`nvim_set_keymap`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_keymap(
    channel_id: u64,
    mode: NvimString,
    lhs: NvimString,
    rhs: NvimString,
    opts: *mut KeyDict_keymap,
    err: *mut Error,
) {
    unsafe {
        modify_keymap(channel_id, -1, MAPTYPE_MAP, mode, lhs, rhs, opts, err);
    }
}

/// Unmaps a global mapping (`nvim_del_keymap`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_del_keymap(
    channel_id: u64,
    mode: NvimString,
    lhs: NvimString,
    opts: *mut KeyDict_keymap_del,
    err: *mut Error,
) {
    unsafe {
        nvim_buf_del_keymap(channel_id, -1, mode, lhs, opts, err);
    }
}

/// Returns a 2-tuple of channel id and api-metadata (`nvim_get_api_info`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_api_info(channel_id: u64, arena: *mut Arena) -> Array {
    unsafe {
        let mut rv = arena_array(arena, 2);
        assert!(channel_id <= (i64::MAX as u64));
        add_c(&mut rv, integer_obj(channel_id as i64));
        add_c(&mut rv, api_metadata());
        rv
    }
}

/// Self-identifies the client (`nvim_set_client_info`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_client_info(
    channel_id: u64,
    name: NvimString,
    mut version: Dict,
    client_type: NvimString,
    methods: Dict,
    attributes: Dict,
    arena: *mut Arena,
    _err: *mut Error,
) {
    unsafe {
        let mut info_items = [std::mem::zeroed::<KeyValuePair>(); 5];
        let mut info = Dict {
            items: info_items.as_mut_ptr(),
            size: 0,
            capacity: 5,
        };
        put_c(&mut info, c"name".as_ptr(), string_obj(name));

        let mut has_major = false;
        for i in 0..version.size {
            if strequal((*version.items.add(i)).key.data, c"major".as_ptr()) {
                has_major = true;
                break;
            }
        }
        if !has_major {
            let mut v = arena_dict(arena, version.size + 1);
            if version.size != 0 {
                ptr::copy_nonoverlapping(version.items, v.items, version.size);
                v.size = version.size;
            }
            put_c(&mut v, c"major".as_ptr(), integer_obj(0));
            version = v;
        }
        put_c(&mut info, c"version".as_ptr(), dict_obj(version));
        put_c(&mut info, c"type".as_ptr(), string_obj(client_type));
        put_c(&mut info, c"methods".as_ptr(), dict_obj(methods));
        put_c(&mut info, c"attributes".as_ptr(), dict_obj(attributes));

        rpc_set_client_info(channel_id, copy_dict(info, ptr::null_mut()));
    }
}

/// Sets channel properties (`nvim_chan_set`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_chan_set(
    channel_id: u64,
    mut chan: Integer,
    config: *mut KeyDict_chan_set,
    err: *mut Error,
) {
    unsafe {
        if chan == 0 && !is_internal_call(channel_id) {
            chan = channel_id as Integer;
        }
        let channel = find_channel(chan as u64);
        if !validate_int(!channel.is_null(), c"chan".as_ptr(), chan, err) {
            return;
        }
        if ((*config).is_set__chan_set_ & (1u64 << KEYSET_OPTIDX_chan_set__detach)) != 0 {
            (*channel_tail(channel)).detach = (*config).detach;
            channel_event(channel, EVENT_CHANINFO);
        }
    }
}

/// Records the cmdwin scratchbuf and type (`nvim__cmdwin_set`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__cmdwin_set(type_str: NvimString, buf: Buffer, err: *mut Error) {
    unsafe {
        if type_str.size == 0 || buf == 0 {
            cmdwin_type = 0;
            cmdwin_buf = ptr::null_mut();
            return;
        }
        let b = find_buffer_by_handle(buf, err);
        if error_set(&*err) {
            return;
        }
        cmdwin_type = c_int::from(*type_str.data as u8);
        cmdwin_buf = b;
    }
}

/// Gets information about a channel (`nvim_get_chan_info`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_chan_info(
    channel_id: u64,
    mut chan: Integer,
    arena: *mut Arena,
    _err: *mut Error,
) -> Dict {
    if chan < 0 {
        return Dict::new();
    }
    unsafe {
        if chan == 0 && !is_internal_call(channel_id) {
            assert!(channel_id <= (i64::MAX as u64));
            chan = channel_id as Integer;
        }
        channel_info(chan as u64, arena)
    }
}

/// Get information about all open channels (`nvim_list_chans`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_list_chans(arena: *mut Arena) -> Array {
    unsafe { channel_all_info(arena) }
}

/// Returns object given as argument (`nvim__id`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__id(obj: Object, arena: *mut Arena) -> Object {
    unsafe { copy_object(obj, arena) }
}

/// Returns array given as argument (`nvim__id_array`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__id_array(arr: Array, arena: *mut Arena) -> Array {
    unsafe { copy_array(arr, arena) }
}

/// Returns dict given as argument (`nvim__id_dict`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__id_dict(dct: Dict, arena: *mut Arena) -> Dict {
    unsafe { copy_dict(dct, arena) }
}

/// Returns floating-point value given as argument (`nvim__id_float`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn nvim__id_float(flt: Float) -> Float {
    flt
}

/// Gets internal stats (`nvim__stats`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__stats(arena: *mut Arena) -> Dict {
    unsafe {
        let mut rv = arena_dict(arena, 6);
        put_c(&mut rv, c"fsync".as_ptr(), integer_obj(g_stats.fsync));
        put_c(
            &mut rv,
            c"log_skip".as_ptr(),
            integer_obj(Integer::from(g_stats.log_skip)),
        );
        put_c(
            &mut rv,
            c"lua_refcount".as_ptr(),
            integer_obj(Integer::from(nlua_get_global_ref_count())),
        );
        put_c(&mut rv, c"redraw".as_ptr(), integer_obj(g_stats.redraw));
        put_c(
            &mut rv,
            c"arena_alloc_count".as_ptr(),
            integer_obj(arena_alloc_count as Integer),
        );
        put_c(
            &mut rv,
            c"ts_query_parse_count".as_ptr(),
            integer_obj(tslua_query_parse_count as Integer),
        );
        rv
    }
}

/// Gets a list of dictionaries representing attached UIs (`nvim_list_uis`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_list_uis(arena: *mut Arena) -> Array {
    unsafe { ui_array(arena) }
}

/// Gets the immediate children of process `pid` (`nvim_get_proc_children`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_proc_children(
    pid: Integer,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rvobj = Array::new();
        let mut proc_list: *mut c_int = ptr::null_mut();

        if !validate_int(
            pid > 0 && pid <= Integer::from(c_int::MAX),
            c"pid".as_ptr(),
            pid,
            err,
        ) {
            return rvobj;
        }

        let mut proc_count: usize = 0;
        let rv = os_proc_children(pid as c_int, &mut proc_list, &mut proc_count);
        if rv == 2 {
            let mut a_items = [integer_obj(pid)];
            let a = Array {
                items: a_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            let o = nlua_exec(
                static_cstr_as_string(c"return vim._os_proc_children(...)"),
                ptr::null(),
                a,
                kRetObject,
                arena,
                err,
            );
            if o.obj_type == ObjectType::kObjectTypeArray {
                rvobj = o.data.array;
            } else if !error_set(&*err) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to get process children. pid=%lld error=%d".as_ptr(),
                    pid,
                    rv,
                );
            }
        } else {
            rvobj = arena_array(arena, proc_count);
            for i in 0..proc_count {
                add_c(&mut rvobj, integer_obj(Integer::from(*proc_list.add(i))));
            }
        }

        xfree(proc_list.cast::<c_void>());
        rvobj
    }
}

/// Gets info describing process `pid` (`nvim_get_proc`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_proc(pid: Integer, arena: *mut Arena, err: *mut Error) -> Object {
    unsafe {
        #[allow(unused_assignments)]
        let mut rvobj = nil_obj();
        if !validate_int(
            pid > 0 && pid <= Integer::from(c_int::MAX),
            c"pid".as_ptr(),
            pid,
            err,
        ) {
            return nil_obj();
        }

        #[cfg(windows)]
        {
            let _ = err;
            rvobj = dict_obj(os_proc_info(pid as c_int, arena));
            if rvobj.data.dict.size == 0 {
                return nil_obj();
            }
        }
        #[cfg(not(windows))]
        {
            let mut a_items = [integer_obj(pid)];
            let a = Array {
                items: a_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            let o = nlua_exec(
                static_cstr_as_string(c"return vim._os_proc_info(...)"),
                ptr::null(),
                a,
                kRetObject,
                arena,
                err,
            );
            if o.obj_type == ObjectType::kObjectTypeArray && o.data.array.size == 0 {
                return nil_obj();
            } else if o.obj_type == ObjectType::kObjectTypeDict {
                rvobj = o;
            } else if !error_set(&*err) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to get process info. pid=%lld".as_ptr(),
                    pid,
                );
            }
        }
        rvobj
    }
}

/// Selects an item in the completion popup menu (`nvim_select_popupmenu_item`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_select_popupmenu_item(
    item: Integer,
    mut insert: Boolean,
    finish: Boolean,
    _opts: *mut KeyDict_empty,
    _err: *mut Error,
) {
    if finish {
        insert = true;
    }
    unsafe {
        pum_ext_select_item(item as c_int, insert, finish);
    }
}

/// Inspect screen grid cell (`nvim__inspect_cell`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__inspect_cell(
    grid: Integer,
    row: Integer,
    col: Integer,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut ret = Array::new();
        let mut g: *mut ScreenGrid = &raw mut default_grid;
        if grid == Integer::from(pum_grid.handle) {
            g = &raw mut pum_grid;
        } else if grid > 1 {
            let wp = get_win_by_grid_handle(grid as handle_T);
            if !validate_int(
                !wp.is_null() && !(*wp).w_grid_alloc.chars.is_null(),
                c"grid handle".as_ptr(),
                grid,
                err,
            ) {
                return ret;
            }
            g = &raw mut (*wp).w_grid_alloc;
        }

        if row < 0 || row >= Integer::from((*g).rows) || col < 0 || col >= Integer::from((*g).cols)
        {
            return ret;
        }
        ret = arena_array(arena, 3);
        let off = *(*g).line_offset.add(row as usize) + (col as usize);
        let sc_buf = arena_alloc(arena, MAX_SCHAR_SIZE, false).cast::<c_char>();
        schar_get(sc_buf, *(*g).chars.add(off));
        add_c(&mut ret, cstr_as_obj(sc_buf));
        let attr = *(*g).attrs.add(off);
        add_c(
            &mut ret,
            dict_obj(hl_get_attr_by_id(Integer::from(attr), true, arena, err)),
        );
        if !highlight_use_hlstate() {
            add_c(&mut ret, array_obj(hl_inspect(attr, arena)));
        }
        ret
    }
}

/// Take screenshot (`nvim__screenshot`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__screenshot(path: NvimString) {
    unsafe {
        ui_call_screenshot(path);
    }
}

/// Invalidate glyph cache (`nvim__invalidate_glyph_cache`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__invalidate_glyph_cache() {
    unsafe {
        schar_cache_clear();
        must_redraw = UPD_CLEAR;
    }
}

/// Unpack msgpack string (`nvim__unpack`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__unpack(
    str_: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { unpack(str_.data, str_.size, arena, err) }
}

/// Deletes an uppercase/file named mark (`nvim_del_mark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_del_mark(name: NvimString, err: *mut Error) -> Boolean {
    unsafe {
        if !validate_s(
            name.size == 1,
            c"mark name (must be a single char)".as_ptr(),
            name.data,
            err,
        ) {
            return false;
        }
        let ch = *name.data as u8;
        if !validate_s(
            ch.is_ascii_uppercase() || ch.is_ascii_digit(),
            c"mark name (must be file/uppercase)".as_ptr(),
            name.data,
            err,
        ) {
            return false;
        }
        set_mark(ptr::null_mut(), name, 0, 0, err)
    }
}

/// Returns a `(row, col, buffer, buffername)` tuple representing the position of the mark (`nvim_get_mark`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_mark(
    name: NvimString,
    _opts: *mut KeyDict_empty,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        if !validate_s(
            name.size == 1,
            c"mark name (must be a single char)".as_ptr(),
            name.data,
            err,
        ) {
            return rv;
        }
        let ch = *name.data as u8;
        if !validate_s(
            ch.is_ascii_uppercase() || ch.is_ascii_digit(),
            c"mark name (must be file/uppercase)".as_ptr(),
            name.data,
            err,
        ) {
            return rv;
        }

        let mark = mark_get_global(false, c_int::from(*name.data));
        let pos = (*mark).fmark.mark;
        let mut allocated = false;
        let mut bufnr: c_int;
        let mut filename: *mut c_char;

        if (*mark).fmark.fnum != 0 {
            bufnr = (*mark).fmark.fnum;
            filename = buflist_nr2name(bufnr, 1, 1);
            allocated = true;
        } else {
            filename = (*mark).fname;
            bufnr = 0;
        }

        let exists = !filename.is_null();
        let row: Integer;
        let col: Integer;

        if !exists || pos.lnum <= 0 {
            if allocated {
                xfree(filename.cast::<c_void>());
                allocated = false;
            }
            filename = c"".as_ptr().cast_mut();
            bufnr = 0;
            row = 0;
            col = 0;
        } else {
            row = Integer::from(pos.lnum);
            col = Integer::from(pos.col);
        }

        rv = arena_array(arena, 4);
        add_c(&mut rv, integer_obj(row));
        add_c(&mut rv, integer_obj(col));
        add_c(&mut rv, integer_obj(Integer::from(bufnr)));
        add_c(&mut rv, cstr_to_arena_obj(arena, filename));

        if allocated {
            xfree(filename.cast::<c_void>());
        }

        rv
    }
}

/// Evaluates statusline string (`nvim_eval_statusline`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_eval_statusline(
    str_: NvimString,
    opts: *mut KeyDict_eval_statusline,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut result = Dict::new();
        let mut fillchar: schar_T = 0;
        let mut statuscol_lnum: c_int = 0;

        if str_.size < 2 || *str_.data != (b'%' as c_char) || *str_.data.add(1) != (b'!' as c_char)
        {
            let mut raw_errbuf = [0 as c_char; ERR_BUFLEN];
            let errbuf = CharBuf {
                data: raw_errbuf.as_mut_ptr(),
                size: ERR_BUFLEN,
            };
            let errmsg = check_stl_option(str_.data, &errbuf);
            if !errmsg.is_null() {
                api_set_error!(err, ErrorType::kErrorTypeValidation, c"%s".as_ptr(), errmsg);
                return result;
            }
        }

        let window = (*opts).winid;

        if ((*opts).is_set__eval_statusline_ & (1u64 << KEYSET_OPTIDX_eval_statusline__fillchar))
            != 0
        {
            if *(*opts).fillchar.data == 0
                || (utfc_ptr2len((*opts).fillchar.data) as usize) != (*opts).fillchar.size
            {
                api_err_exp(
                    err,
                    c"fillchar".as_ptr(),
                    c"single character".as_ptr(),
                    ptr::null(),
                );
                return result;
            }
            let mut c: c_int = 0;
            fillchar = utfc_ptr2schar((*opts).fillchar.data, &mut c);
        }

        let mut use_bools = c_int::from((*opts).use_winbar) + c_int::from((*opts).use_tabline);

        let wp = if (*opts).use_tabline {
            curwin
        } else {
            find_window_by_handle(window, err)
        };
        if wp.is_null() {
            return result;
        }

        if ((*opts).is_set__eval_statusline_
            & (1u64 << KEYSET_OPTIDX_eval_statusline__use_statuscol_lnum))
            != 0
        {
            statuscol_lnum = (*opts).use_statuscol_lnum as c_int;
            if !validate_range(
                statuscol_lnum > 0 && statuscol_lnum <= (*(*wp).w_buffer).b_ml.ml_line_count,
                c"use_statuscol_lnum".as_ptr(),
                err,
            ) {
                return result;
            }
            use_bools += 1;
        }
        if use_bools > 1 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Can only use one of 'use_winbar', 'use_tabline' and 'use_statuscol_lnum'"
                    .as_ptr(),
            );
            return result;
        }

        let mut stc_hl_id: c_int = 0;
        let mut scl_hl_id: c_int = 0;
        let mut statuscol = std::mem::zeroed::<statuscol_T>();
        let mut sattrs = [crate::decoration::SignTextAttrs {
            text: [0; 2],
            hl_id: 0,
        }; SIGN_SHOW_MAX];

        if statuscol_lnum != 0 {
            let mut line_id: c_int = 0;
            let mut cul_id: c_int = 0;
            let mut num_id: c_int = 0;
            let lnum = statuscol_lnum as linenr_T;
            let mut cursorline_fi = std::mem::zeroed::<foldinfo_T>();
            decor_redraw_signs(
                wp,
                (*wp).w_buffer,
                lnum - 1,
                sattrs.as_mut_ptr(),
                &mut line_id,
                &mut cul_id,
                &mut num_id,
            );

            statuscol.sattrs = sattrs.as_mut_ptr().cast::<SignTextAttrs>();
            statuscol.foldinfo = fold_info(wp, lnum);
            win_update_cursorline(wp, &mut cursorline_fi);
            let use_cul = use_cursor_line_highlight(wp, lnum);
            statuscol.sign_cul_id = if use_cul { cul_id } else { 0 };
            scl_hl_id = if use_cul { HLF_CLS } else { HLF_SC };

            if num_id != 0 {
                stc_hl_id = num_id;
            } else if use_cul {
                stc_hl_id = HLF_CLN;
            } else if (*wp).w_onebuf_opt.wo_rnu != 0 {
                stc_hl_id = if lnum < (*wp).w_cursor.lnum {
                    HLF_LNA
                } else {
                    HLF_LNB
                };
            } else {
                stc_hl_id = HLF_N;
            }

            set_vim_var_nr(VV_LNUM, i64::from(lnum));
            let rel = get_cursor_rel_lnum(wp, lnum);
            #[allow(clippy::unnecessary_cast)]
            set_vim_var_nr(VV_RELNUM, (rel as i64).abs());
            set_vim_var_nr(VV_VIRTNUM, 0);
        } else if fillchar == 0 && !(*opts).use_tabline {
            if (*opts).use_winbar {
                fillchar = (*wp).w_p_fcs_chars.wbr;
            } else {
                let mut group: hlf_T = 0;
                fillchar = fillchar_status(&mut group, wp);
            }
        }

        let maxwidth = if ((*opts).is_set__eval_statusline_
            & (1u64 << KEYSET_OPTIDX_eval_statusline__maxwidth))
            != 0
        {
            (*opts).maxwidth as c_int
        } else if statuscol_lnum != 0 {
            win_col_off(wp)
        } else if (*opts).use_tabline || (!(*opts).use_winbar && global_stl_height() > 0) {
            Columns
        } else {
            (*wp).w_width
        };

        result = arena_dict(arena, 3);
        let buf_data = arena_alloc(arena, MAXPATHL, false).cast::<c_char>();
        let buf = CharBuf {
            data: buf_data,
            size: MAXPATHL,
        };
        let mut hltab: *mut stl_hlrec_t = ptr::null_mut();
        let mut hltab_len: usize = 0;

        let p_crb_save = (*wp).w_onebuf_opt.wo_crb;
        (*wp).w_onebuf_opt.wo_crb = 0;

        let width = build_stl_str_hl(
            wp,
            buf,
            str_.data,
            -1,
            0,
            fillchar,
            maxwidth,
            if (*opts).highlights {
                &mut hltab
            } else {
                ptr::null_mut()
            },
            &mut hltab_len,
            ptr::null_mut(),
            if statuscol_lnum != 0 {
                &mut statuscol
            } else {
                ptr::null_mut()
            },
        );

        put_c(
            &mut result,
            c"width".as_ptr(),
            integer_obj(Integer::from(width)),
        );

        (*wp).w_onebuf_opt.wo_crb = p_crb_save;

        if (*opts).highlights {
            let mut hl_values = arena_array(arena, hltab_len + 1);
            let dfltname = get_default_stl_hl(
                if (*opts).use_tabline {
                    ptr::null_mut()
                } else {
                    wp
                },
                (*opts).use_winbar,
                stc_hl_id,
            );
            if (*hltab).start.is_null() || (*hltab).start.offset_from(buf_data) != 0 {
                let mut hl_info = arena_dict(arena, 3);
                put_c(&mut hl_info, c"start".as_ptr(), integer_obj(0));
                put_c(&mut hl_info, c"group".as_ptr(), cstr_as_obj(dfltname));
                let mut groups = arena_array(arena, 1);
                add_c(&mut groups, cstr_as_obj(dfltname));
                put_c(&mut hl_info, c"groups".as_ptr(), array_obj(groups));
                add_c(&mut hl_values, dict_obj(hl_info));
            }

            let mut sp = hltab;
            while !(*sp).start.is_null() {
                let user_str: String;
                let grpname: *const c_char = if (*sp).userhl == 0 {
                    get_default_stl_hl(
                        if (*opts).use_tabline {
                            ptr::null_mut()
                        } else {
                            wp
                        },
                        (*opts).use_winbar,
                        stc_hl_id,
                    )
                } else if (*sp).userhl < 0 {
                    syn_id2name(-(*sp).userhl)
                } else {
                    user_str = format!("User{}\0", (*sp).userhl);
                    arena_strdup(arena, user_str.as_ptr().cast::<c_char>())
                };

                let combine = if (*sp).item == STL_SIGNCOL {
                    syn_id2name(scl_hl_id)
                } else if (*sp).item == STL_FOLDCOL {
                    grpname
                } else {
                    dfltname
                };
                let mut hl_info = arena_dict(arena, 3);
                put_c(
                    &mut hl_info,
                    c"start".as_ptr(),
                    integer_obj((*sp).start.offset_from(buf_data) as Integer),
                );
                put_c(&mut hl_info, c"group".as_ptr(), cstr_as_obj(grpname));
                let diff_group = combine != grpname;
                let mut groups = arena_array(arena, 1 + usize::from(diff_group));
                if diff_group {
                    add_c(&mut groups, cstr_as_obj(combine));
                }
                add_c(&mut groups, cstr_as_obj(grpname));
                put_c(&mut hl_info, c"groups".as_ptr(), array_obj(groups));
                add_c(&mut hl_values, dict_obj(hl_info));

                sp = sp.add(1);
            }
            put_c(&mut result, c"highlights".as_ptr(), array_obj(hl_values));
        }
        put_c(&mut result, c"str".as_ptr(), cstr_as_obj(buf_data));

        result
    }
}

/// Sets info for the completion item at the given index (`nvim__complete_set`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__complete_set(
    index: Integer,
    opts: *mut KeyDict_complete_set,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut rv = arena_dict(arena, 2);
        if (get_cot_flags() & kOptCotFlagPopup) == 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"completeopt option does not include popup".as_ptr(),
            );
            return rv;
        }
        if ((*opts).is_set__complete_set_ & (1u64 << KEYSET_OPTIDX_complete_set__info)) != 0 {
            let wp = pum_set_info(index as c_int, (*opts).info.data);
            if !wp.is_null() {
                put_c(&mut rv, c"winid".as_ptr(), window_obj((*wp).handle));
                put_c(
                    &mut rv,
                    c"bufnr".as_ptr(),
                    buffer_obj((*(*wp).w_buffer).b_fnum),
                );
            }
        }
        rv
    }
}

unsafe fn redraw_status(wp: *mut win_T, opts: *mut KeyDict_redraw, flush: *mut bool) {
    unsafe {
        if (*opts).statuscolumn && *(*wp).w_onebuf_opt.wo_stc != NUL {
            (*wp).w_nrwidth_line_count = 0;
            changed_window_setting(wp);
        }

        let old_row_offset = (*wp).w_grid.row_offset;
        win_grid_alloc(wp);

        if (*wp).w_lines_valid == 0 || (*wp).w_grid.row_offset != old_row_offset {
            *flush = true;
        }

        if *flush && ((*opts).statusline || (*opts).winbar) {
            (*wp).w_redr_status = true;
        } else if (*opts).statusline || (*opts).winbar {
            win_check_ns_hl(wp);
            if (*opts).winbar {
                win_redr_winbar(wp);
            }
            if (*opts).statusline {
                win_redr_status(wp);
            }
            win_check_ns_hl(ptr::null_mut());
        }
    }
}

/// Instruct Nvim to redraw various components (`nvim__redraw`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__redraw(opts: *mut KeyDict_redraw, err: *mut Error) {
    unsafe {
        let mut win: *mut win_T = ptr::null_mut();
        let mut buf: *mut buf_T = ptr::null_mut();

        if ((*opts).is_set__redraw_ & (1u64 << KEYSET_OPTIDX_redraw__win)) != 0 {
            win = find_window_by_handle((*opts).win, err);
            if error_set(&*err) {
                return;
            }
        }

        if ((*opts).is_set__redraw_ & (1u64 << KEYSET_OPTIDX_redraw__buf)) != 0 {
            if !win.is_null() {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"cannot use both 'buf' and 'win'".as_ptr(),
                );
                return;
            }
            buf = find_buffer_by_handle((*opts).buf, err);
            if error_set(&*err) {
                return;
            }
        }

        let count = u32::from(!win.is_null()) + u32::from(!buf.is_null());
        if (xpopcount((*opts).is_set__redraw_) as u32) <= count {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"at least one action required".as_ptr(),
            );
            return;
        }

        let has_valid = ((*opts).is_set__redraw_ & (1u64 << KEYSET_OPTIDX_redraw__valid)) != 0;
        let has_range = ((*opts).is_set__redraw_ & (1u64 << KEYSET_OPTIDX_redraw__range)) != 0;
        let has_flush = ((*opts).is_set__redraw_ & (1u64 << KEYSET_OPTIDX_redraw__flush)) != 0;

        if has_valid {
            let rtype = if (*opts).valid {
                UPD_VALID
            } else {
                UPD_NOT_VALID
            };
            if !win.is_null() {
                redraw_later(win, rtype);
            } else if !buf.is_null() {
                redraw_buf_later(buf, rtype);
            } else {
                redraw_all_later(rtype);
            }
        }

        if has_range {
            let r = (*opts).range;
            let ok = r.size == 2
                && (*r.items).obj_type == ObjectType::kObjectTypeInteger
                && (*r.items.add(1)).obj_type == ObjectType::kObjectTypeInteger
                && (*r.items).data.integer >= 0
                && (*r.items.add(1)).data.integer >= -1;
            if !ok {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"Invalid 'range': Expected 2-tuple of Integers".as_ptr(),
                );
                return;
            }
            let begin_raw = (*r.items).data.integer;
            let end_raw = (*r.items.add(1)).data.integer;

            let rbuf = if !win.is_null() {
                (*win).w_buffer
            } else if !buf.is_null() {
                buf
            } else {
                curbuf
            };
            let line_count = i64::from((*rbuf).b_ml.ml_line_count);
            let begin = begin_raw.min(line_count) as c_int;
            let end = if end_raw == -1 {
                line_count as c_int
            } else {
                end_raw.max(i64::from(begin)).min(line_count) as c_int
            };

            if begin < end {
                redraw_buf_range_later(rbuf, 1 + begin, end);
            }
        }

        if has_valid || has_range {
            (*opts).flush = if has_flush { (*opts).flush } else { true };
        }

        let mut flush_ui = (*opts).flush;
        if (*opts).tabline {
            if redraw_tabline && (*firstwin).w_lines_valid == 0 {
                (*opts).flush = true;
            } else {
                draw_tabline();
            }
            flush_ui = true;
        }

        let save_lz = p_lz;
        let save_rd = RedrawingDisabled;
        RedrawingDisabled = 0;
        p_lz = 0;
        if (*opts).statuscolumn || (*opts).statusline || (*opts).winbar {
            if win.is_null() {
                let mut wp = firstwin;
                while !wp.is_null() {
                    if buf.is_null() || (*wp).w_buffer == buf {
                        redraw_status(wp, opts, &raw mut (*opts).flush);
                    }
                    wp = (*wp).w_next;
                }
            } else {
                redraw_status(win, opts, &raw mut (*opts).flush);
            }
            flush_ui = true;
        }

        let cwin = if !win.is_null() { win } else { curwin };
        if (*opts).cursor && ((*cwin).w_grid.target.is_null() || !(*(*cwin).w_grid.target).valid) {
            (*opts).flush = true;
        }

        if (*opts).flush && !cmdpreview {
            validate_cursor(curwin);
            update_topline(curwin);
            update_screen();
        }

        if (*opts).cursor {
            setcursor_mayforce(cwin, true);
            flush_ui = true;
        }

        if flush_ui {
            ui_flush();
        }

        RedrawingDisabled = save_rd;
        p_lz = save_lz;
    }
}

/// Set restart on crash (`nvim__set_restart_on_crash`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__set_restart_on_crash(progpath: NvimString, argv: Array) {
    unsafe {
        ui_call__set_restart_on_crash_exit(progpath, argv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_call_and_id_float_work() {
        assert!(is_internal_call(INTERNAL_CALL_MASK));
        assert!(is_internal_call(INTERNAL_CALL_MASK + 1));
        assert!(!is_internal_call(1));
        assert_eq!(nvim__id_float(3.25), 3.25);
    }
}
