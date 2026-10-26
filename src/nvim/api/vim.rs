//! Global Neovim API functions (`src/nvim/api/vim.c`).

use crate::api::buffer::{
    INTERNAL_CALL_MASK, KeyDict_keymap, KeyDict_keymap_del, api_buf_ensure_loaded,
    nvim_buf_del_keymap,
};
use crate::api::deprecated::{KeyDict_empty, buffer_del_line, buffer_get_line, buffer_set_line};
use crate::api::private::converter::vim_to_object;
use crate::api::private::helpers::{
    ArrayBuilder, HlMessageChunk, TryState, add_c, api_metadata, api_set_error, api_set_sctx,
    api_typename, arena_array, arena_dict, arena_take_arraybuilder, array_obj, boolean_obj,
    buffer_obj, copy_array, copy_dict, copy_object, copy_string, cstr_as_obj, cstr_as_string,
    cstr_to_arena_obj, dict_get_value, dict_obj, dict_set_var, error_set, find_buffer_by_handle,
    find_tab_by_handle, find_window_by_handle, get_default_stl_hl, integer_obj, nil_obj,
    parse_hl_msg, put_c, set_mark, static_cstr_as_string, string_obj, string_to_array, tabpage_obj,
    try_enter, try_leave, window_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType, api_err_exp,
    api_err_invalid, api_err_required,
};
use crate::api::tabpage::{Buffer, Tabpage, Window};
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFNEW, EVENT_CHANINFO, apply_autocmds, block_autocmds,
    may_trigger_vim_suspend_resume, unblock_autocmds,
};
use crate::buffer::{
    BCO_ENTER, BCO_NOHELP, BLN_LISTED, BLN_NEW, BLN_NOOPT, DOBUF_FIRST, DOBUF_GOTO, buf_T,
    buf_close_terminal, buf_get_changedtick, buflist_new, buflist_nr2name, bufref_T, bufref_valid,
    do_buffer, read_buffer_into, set_bufref,
};
use crate::channel::{
    Channel, TerminalOptions, channel_all_info, channel_alloc, channel_decref, channel_event,
    channel_incref, channel_info, channel_send,
};
use crate::charset::{NL, NUL};
use crate::context::{
    CONTEXT_INIT, CtxStateFlags, ctx_free, ctx_from_dict, ctx_load, ctx_save, ctx_to_dict,
    kCtxBufs, kCtxFuncs, kCtxGVars, kCtxJumps, kCtxRegs, kCtxSFuncs,
};
use crate::cursor::{get_cursor_rel_lnum, linenr_T};
use crate::decoration::{NS, SignTextAttrs, decor_redraw_signs};
use crate::drawline::use_cursor_line_highlight;
use crate::drawscreen::{
    UPD_CLEAR, UPD_NOT_VALID, UPD_VALID, redraw_all_later, redraw_buf_later,
    redraw_buf_range_later, redraw_later, setcursor_mayforce, update_screen, win_update_cursorline,
};
use crate::eval::typval::tv_dict_find;
use crate::eval::vars::{
    VV_LNUM, VV_RELNUM, VV_VIRTNUM, get_globvar_dict, get_vimvar_dict, set_vim_var_nr,
};
use crate::ex_docmd::{do_chdir, exec_normal};
use crate::ex_eval::aborting;
use crate::ex_getln::hlf_T;
use crate::fold::fold_info;
use crate::globals::*;
use crate::grid::{
    ScreenGrid, get_win_by_grid_handle, schar_T, schar_cache_clear, schar_get, win_grid_alloc,
};
use crate::highlight::HlAttrs;
use crate::highlight::{
    KeyDict_highlight, OptionalKeys, dict2hlattrs, highlight_use_hlstate, hl_check_ns,
    hl_get_attr_by_id, hl_inspect, hl_ns_get_attrs, ns_hl_def, win_check_ns_hl,
};
use crate::highlight_group::{
    KeyDict_get_highlight, color_name_table, name_to_color, ns_get_hl_defs, syn_check_group,
    syn_id2name,
};
use crate::input::{ins_typebuf, paste_store};
use crate::insexpand::get_cot_flags;
use crate::keycodes::{
    KE_LEFTDRAG, KE_LEFTMOUSE, KE_LEFTRELEASE, KE_MIDDLEMOUSE, KE_MOUSEDOWN, KE_MOUSELEFT,
    KE_MOUSEMOVE, KE_MOUSERIGHT, KE_MOUSEUP, KE_RIGHTMOUSE, KE_X1MOUSE, KE_X2MOUSE, REPTERM_DO_LT,
    REPTERM_FROM_PART, REPTERM_NO_SPECIAL, name_to_mod_mask, replace_termcodes,
    vim_strsave_escape_ks,
};
use crate::lua::converter::{Boolean, Float, LUA_NOREF, LuaRef};
use crate::lua::executor::{
    api_free_luaref, kRetNilBool, kRetObject, nlua_call_ref, nlua_exec, nlua_get_global_ref_count,
    nlua_is_deferred_safe,
};
use crate::mapping::{MAPTYPE_MAP, REMAP_NONE, REMAP_YES, keymap_array, modify_keymap};
use crate::mark::mark_get_global;
use crate::math::{FAIL, xpopcount};
use crate::mbyte::{mb_string2cells, utfc_ptr2len, utfc_ptr2schar};
use crate::memfile::memfile_T;
use crate::memline::ml_open;
use crate::memory::{Arena, arena_alloc, arena_strdup, memchrsub, strequal, xfree};
use crate::message::{
    msg_ext_no_fast, msg_id_exists, msg_multihl, verbose_enter, verbose_leave, verbose_stop,
};
use crate::r#move::{changed_window_setting, update_topline, validate_cursor, win_col_off};
use crate::msgpack_rpc::channel::{ChannelStreamType, find_channel, rpc_set_client_info};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::msgpack_rpc::unpacker::unpack;
use crate::normal::reset_VIsual_and_resel;
use crate::option::{
    OPT_LOCAL, OptTarget, buf_copy_options, kOptBufhidden, kOptBuftype, set_option_direct_for,
};
use crate::option_vars::*;
use crate::optionstr::{CharBuf, check_stl_option};
use crate::os::input::{input_blocking, input_enqueue, input_enqueue_mouse, input_enqueue_raw};
use crate::os::proc::os_proc_children;
use crate::popupmenu::{pum_ext_select_item, pum_set_info};
use crate::register::{
    PUT_CURSEND, do_put, finish_yankreg_from_object, prepare_yankreg_from_object,
};
use crate::runtime::{
    DIP_ALL, DIP_DIRFILE, DOSO_NONE, RuntimeCallback, do_in_runtimepath, do_source, get_lib_dir,
    runtime_get_named, runtime_inspect, script_autoload,
};
use crate::search::{BACKWARD, FORWARD};
use crate::state::{MODE_MAX_LENGTH, get_mode};
use crate::statusline::{
    build_stl_str_hl, draw_tabline, fillchar_status, foldinfo_T, statuscol_T, stl_hlrec_t,
    win_redr_status, win_redr_winbar,
};
use crate::terminal::{
    terminal_alloc, terminal_buf, terminal_check_size, terminal_destroy, terminal_open,
    terminal_running, terminal_set_streamed_paste,
};
use crate::ui::{ui_array, ui_call__set_restart_on_crash_exit, ui_call_screenshot, ui_flush};
use crate::window::{
    global_stl_height, goto_tabpage_tp, goto_tabpage_win, win_T, win_find_tabpage,
};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

#[cfg(windows)]
use crate::os::proc::os_proc_info;

use crate::extmark::{kFalse, kNone, kTrue};
use crate::os::fs::MAXPATHL;

pub(crate) const ERR_BUFLEN: usize = 256;
pub(crate) const SIGN_SHOW_MAX: usize = 9;

pub(crate) const kCdScopeGlobal: c_int = 3;

pub(crate) const kOptCotFlagPopup: c_uint = 0x100;

pub(crate) const kChannelStreamInternal: ChannelStreamType = 4;

const kCtxAll: CtxStateFlags = kCtxRegs | kCtxJumps | kCtxBufs | kCtxGVars | kCtxSFuncs | kCtxFuncs;

pub(crate) const HLF_N: c_int = 12;
pub(crate) const HLF_LNA: c_int = 13;
pub(crate) const HLF_LNB: c_int = 14;
pub(crate) const HLF_CLN: c_int = 15;
pub(crate) const HLF_CLS: c_int = 16;
pub(crate) const HLF_SC: c_int = 35;

use crate::highlight::{KEYSET_OPTIDX_highlight__update, KEYSET_OPTIDX_highlight__url};
use crate::statusline::{STL_FOLDCOL, STL_SIGNCOL};

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

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_get_ns {
    pub is_set__get_ns_: OptionalKeys,
    pub winid: Window,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_runtime {
    pub is_lua: Boolean,
    pub do_source: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
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
#[derive(Clone, Copy, Default)]
pub struct KeyDict_open_term {
    pub is_set__open_term_: OptionalKeys,
    pub on_input: LuaRef,
    pub force_crlf: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_context {
    pub is_set__context_: OptionalKeys,
    pub types: Array,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_chan_set {
    pub is_set__chan_set_: OptionalKeys,
    pub detach: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
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
#[derive(Clone, Copy, Default)]
pub struct KeyDict_complete_set {
    pub is_set__complete_set_: OptionalKeys,
    pub info: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
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

pub type HlMessage = Vec<HlMessageChunk>;
use crate::api::ui::MAX_SCHAR_SIZE;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MessageData {
    pub source: NvimString,
    pub percent: Integer,
    pub title: NvimString,
    pub status: NvimString,
    pub data: Dict,
}

pub struct RuntimeCookie {
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
pub const fn is_internal_call(channel_id: u64) -> bool {
    (channel_id & INTERNAL_CALL_MASK) != 0
}

/// Gets a highlight group by name
///
/// similar to |hlID()|, but allocates a new ID if not present.
/// @param[out] err Error details, if any
// DLLEXPORT Integer nvim_get_hl_id_by_name(String name, Error *err) FUNC_API_SINCE(7);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_hl_id_by_name(name: NvimString, err: *mut Error) -> Integer {
    unsafe {
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let ret = syn_check_group(name.data, name.size);
        try_leave(&tstate, err);
        Integer::from(ret)
    }
}

/// Gets all or specific highlight groups in a namespace.
///
/// @note When the `link` attribute is defined in the highlight definition
///       map, other attributes will not be taking effect (see |:hi-link|).
///
/// @param ns_id Get highlight groups for namespace ns_id |nvim_get_namespaces()|.
///              Use 0 to get global highlight groups |:highlight|.
/// @param opts  Options dict:
///                 - create: (boolean, default true) When highlight group doesn't exist create it.
///                 - id: (integer) Get a highlight definition by id.
///                 - link: (boolean, default true) Show linked group name instead of effective definition |:hi-link|.
///                 - name: (string) Get a highlight definition by name.
///
/// @param[out] err Error details, if any.
/// @return Highlight groups as a map from group name to a highlight definition map as in |nvim_set_hl()|,
///                   or only a single highlight definition map if requested by name or id.
// DLLEXPORT DictAs(get_hl_info) nvim_get_hl(Integer ns_id, Dict(get_highlight) *opts, Arena *arena,
//                                           Error *err) FUNC_API_SINCE(11);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_hl(
    ns_id: Integer,
    opts: *mut KeyDict_get_highlight,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe { ns_get_hl_defs(ns_id as NS, opts, arena, err) }
}

/// Sets a highlight group. By default, replaces the entire definition (e.g. `nvim_set_hl(0, 'Visual', {})`
/// will clear the "Visual" group), unless `update` is specified.
///
/// @note The fg and bg keys also accept the string values `"fg"` or `"bg"`
///       which act as aliases to the corresponding foreground and background
///       values of the Normal group. If the Normal group has not been defined,
///       using these values results in an error.
///
/// @note If `link` is used in combination with other attributes; only the
///       `link` will take effect (see |:hi-link|).
///
/// @param ns_id Namespace id for this highlight |nvim_create_namespace()|.
///              Use 0 to set a highlight group globally |:highlight|.
///              Highlights from non-global namespaces are not active by default, use
///              |nvim_set_hl_ns()| or |nvim_win_set_hl_ns()| to activate them.
/// @param name  Highlight group name, e.g. "ErrorMsg"
/// @param val   Highlight definition map, accepts the following keys:
///                - altfont: boolean
///                - bg: color name or "#RRGGBB", see note.
///                - bg_indexed: boolean. If true, `bg` is an RGB approximation of `ctermbg`
///                  (a palette index). UIs rendering cterm natively may prefer `ctermbg`.
///                - blend: integer between 0 and 100
///                - blink: boolean
///                - bold: boolean
///                - conceal: boolean Concealment at the UI level (terminal SGR), unrelated to |:syn-conceal|.
///                - cterm: cterm attribute map, like |highlight-args|. If not set, cterm attributes
///                  will match those from the attribute map documented above.
///                - ctermbg: Sets background of cterm color |ctermbg|
///                - ctermfg: Sets foreground of cterm color |ctermfg|
///                - default: boolean Don't override existing definition |:hi-default|
///                - dim: boolean
///                - fg: Color name or "#RRGGBB", see note.
///                - fg_indexed: boolean. Same as `bg_indexed`, for `fg` and `ctermfg`.
///                - font: GUI font name (string). Sets |highlight-font|. Use "NONE" to clear.
///                - force: (boolean, default: false) Update the highlight group even if it already exists.
///                - italic: boolean
///                - link: Name of highlight group to link to. |:hi-link|
///                - link_global: Like "link", but always resolved in the global namespace (ns=0).
///                - nocombine: boolean
///                - overline: boolean
///                - reverse: boolean
///                - sp: Color name or "#RRGGBB"
///                - standout: boolean
///                - strikethrough: boolean
///                - undercurl: boolean
///                - underdashed: boolean
///                - underdotted: boolean
///                - underdouble: boolean
///                - underline: boolean
///                - update: (boolean, default: false) Update specified attributes only, leave others unchanged.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_hl(uint64_t channel_id, Integer ns_id, String name, Dict(highlight) *val,
//                            Error *err) FUNC_API_SINCE(7);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_set_hl(
    channel_id: u64,
    ns_id: Integer,
    name: NvimString,
    val: *mut KeyDict_highlight,
    err: *mut Error,
) {
    unsafe {
        let mut tstate = TryState::new();
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
        let mut base_attrs = HlAttrs::default();
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

/// Gets the active highlight namespace.
///
/// @param opts Optional parameters
///           - winid: (number) |window-ID| for retrieving a window's highlight
///             namespace. A value of -1 is returned when |nvim_win_set_hl_ns()|
///             has not been called for the window (or was called with a namespace
///             of -1).
/// @param[out] err Error details, if any
/// @return Namespace id, or -1
// DLLEXPORT Integer nvim_get_hl_ns(Dict(get_ns) *opts, Error *err) FUNC_API_SINCE(12);
#[unsafe(no_mangle)]
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

/// Set active namespace for highlights defined with |nvim_set_hl()|. This can be set for
/// a single window, see |nvim_win_set_hl_ns()|.
///
/// @param ns_id the namespace to use
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_hl_ns(Integer ns_id, Error *err) FUNC_API_SINCE(10);
#[unsafe(no_mangle)]
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

/// Set active namespace for highlights defined with |nvim_set_hl()| while redrawing.
///
/// This function is meant to be called while redrawing, primarily from
/// |nvim_set_decoration_provider()| on_win and on_line callbacks, which
/// are allowed to change the namespace during a redraw cycle.
///
/// @param ns_id the namespace to activate
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_hl_ns_fast(Integer ns_id, Error *err) FUNC_API_SINCE(10) FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_set_hl_ns_fast(ns_id: Integer, _err: *mut Error) {
    unsafe {
        ns_hl_fast = ns_id as NS;
        hl_check_ns();
    }
}

/// Sends input-keys to Nvim, subject to various quirks controlled by `mode` flags. This is
/// a blocking call, unlike |nvim_input()|.
///
/// On execution error: does not fail, but updates v:errmsg.
///
/// To input keycodes like [<C-o>], pass the result of |nvim_replace_termcodes()|:
///
/// ```vim
/// :let key = nvim_replace_termcodes('<C-o>', v:true, v:false, v:true)
/// :call nvim_feedkeys(key, 'n', v:false)
/// ```
///
/// @param keys         Keys to send as input.
/// @param mode         Behavior flags, see |feedkeys()|.
/// @param escape_ks    If true, escape K_SPECIAL bytes in `keys`. Should be false if you used
///                     |nvim_replace_termcodes()|, else true.
/// @see feedkeys()
/// @see vim_strsave_escape_ks
// DLLEXPORT void nvim_feedkeys(String keys, String mode, Boolean escape_ks) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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
            xfree(keys_esc.cast());
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

/// Queues raw user-input. Unlike |nvim_feedkeys()|, this uses a low-level input buffer and the call
/// is non-blocking (input is processed asynchronously by the eventloop).
///
/// To input blocks of text, |nvim_paste()| is much faster and should be preferred.
///
/// On execution error: does not fail, but updates v:errmsg.
///
/// @note |keycodes| like [<CR>] are translated, so "<" is special.
///       To input a literal "<", send [<LT>].
///
/// @note For mouse events use |nvim_input_mouse()|. The pseudokey form
///       `<LeftMouse><col,row>` is deprecated since |api-level| 6.
///
/// @param keys to be typed
/// @return Number of bytes actually written (can be fewer than
///         requested if the buffer becomes full).
// DLLEXPORT Integer nvim_input(uint64_t channel_id, String keys) FUNC_API_SINCE(1) FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_input(channel_id: u64, keys: NvimString) -> Integer {
    unsafe {
        may_trigger_vim_suspend_resume(false);
        input_enqueue(channel_id, keys) as Integer
    }
}

/// Send mouse event from GUI.
///
/// Non-blocking: does not wait on any result, but queues the event to be
/// processed soon by the event loop.
///
/// @note Currently this doesn't support "scripting" multiple mouse events
///       by calling it multiple times in a loop: the intermediate mouse
///       positions will be ignored. It should be used to implement real-time
///       mouse input in a GUI. The deprecated pseudokey form
///       (`<LeftMouse><col,row>`) of |nvim_input()| has the same limitation.
///
/// @param button Mouse button: one of "left", "right", "middle", "wheel", "move",
///               "x1", "x2".
/// @param action For ordinary buttons, one of "press", "drag", "release".
///               For the wheel, one of "up", "down", "left", "right". Ignored for "move".
/// @param modifier String of modifiers each represented by a single char.
///                 The same specifiers are used as for a key press, except
///                 that the "-" separator is optional, so "C-A-", "c-a"
///                 and "CA" can all be used to specify Ctrl+Alt+click.
/// @param grid Grid number (used by |ui-multigrid| client), or 0 to let Nvim decide positioning of
///             windows. For more information, see [dev-ui-multigrid]
/// @param row Mouse row-position (zero-based, like redraw events)
/// @param col Mouse column-position (zero-based, like redraw events)
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_input_mouse(String button, String action, String modifier, Integer grid,
//                                 Integer row, Integer col, Error *err) FUNC_API_SINCE(6)
//   FUNC_API_FAST;
#[unsafe(no_mangle)]
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

/// Converts terminal codes and |keycodes| ([<CR>], [<Esc>], …) in a key sequence, to the internal
/// representation. See also Lua |vim.keycode()|.
///
/// @see replace_termcodes
/// @see cpoptions
///
/// @param str        String to be converted.
/// @param from_part  Legacy Vim parameter. Usually true.
/// @param do_lt      Also translate [<lt>]. Ignored if `special` is false.
/// @param special    Replace |keycodes|, e.g. [<CR>] becomes a "\r" char.
// DLLEXPORT String nvim_replace_termcodes(String str, Boolean from_part, Boolean do_lt,
//                                         Boolean special) FUNC_API_SINCE(1) FUNC_API_FAST
//   FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
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

/// Executes Lua code. Arguments are available as `...` inside the chunk. The chunk can return
/// a value.
///
/// Only statements are executed. To evaluate an expression, prefix it with "return": `return
/// my_function(...)`
///
/// Example:
/// ```lua
/// local peer = vim.fn.jobstart({ vim.v.progpath, '--clean', '--embed' }, { rpc=true })
/// vim.print(vim.rpcrequest(peer, 'nvim_exec_lua', [[
///       local a, b = ...
///       return ('result: %s'):format(a + b)
///     ]],
///     { 1, 3 }
///   )
/// )
/// ```
///
/// @param code       Lua code to execute.
/// @param args       Arguments to the Lua code.
/// @param[out] err   Lua error raised while parsing or executing the Lua code.
///
/// @return           Value returned by the Lua code (if any), or NIL.
// DLLEXPORT Object nvim_exec_lua(String code, Array args, Arena *arena,
//                                Error *err) FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_exec_lua(
    code: NvimString,
    args: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { nlua_exec(code, ptr::null(), args, kRetObject, arena, err) }
}

/// Like |nvim_exec_lua()|, but can be called during |api-fast| contexts.
///
/// Execute Lua code. Parameters (if any) are available as `...` inside the
/// chunk. The chunk can return a value.
///
/// Only statements are executed. To evaluate an expression, prefix it
/// with `return`: return my_function(...)
///
/// @param code       Lua code to execute
/// @param args       Arguments to the code
/// @param[out] err   Details of an error encountered while parsing
///                   or executing the Lua code.
///
/// @return           Return value of Lua code if present or NIL.
// DLLEXPORT Object nvim__exec_lua_fast(String code, Array args, Arena *arena,
//                                      Error *err) FUNC_API_SINCE(14) FUNC_API_REMOTE_ONLY
//   FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__exec_lua_fast(
    code: NvimString,
    args: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { nvim_exec_lua(code, args, arena, err) }
}

/// Calculates the number of display cells occupied by `text`.
/// Control characters including [<Tab>] count as one cell.
///
/// @param text       Some text
/// @param[out] err   Error details, if any
/// @return Number of cells
// DLLEXPORT Integer nvim_strwidth(String text, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Gets the paths contained in |runtime-search-path|.
///
/// @return List of paths
// DLLEXPORT ArrayOf(String) nvim_list_runtime_paths(Arena *arena, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// @nodoc
// DLLEXPORT Array nvim__runtime_inspect(Arena *arena);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__runtime_inspect(arena: *mut Arena) -> Array {
    unsafe { runtime_inspect(arena) }
}

pub unsafe fn find_runtime_cb(fnames: &[*mut c_char], all: bool, c: *mut RuntimeCookie) -> bool {
    unsafe {
        let cookie = &mut *c;
        for &fname in fnames {
            cookie.rv.push(cstr_to_arena_obj(cookie.arena, fname));
            if !all {
                return true;
            }
        }
        !fnames.is_empty()
    }
}

/// Finds files in runtime directories, in 'runtimepath' order.
///
/// "name" can contain wildcards. For example
/// `nvim_get_runtime_file("colors/*.{vim,lua}", true)` will return all color
/// scheme files. Always use forward slashes (/) in the search pattern for
/// subdirectories regardless of platform.
///
/// It is not an error to not find any files. An empty array is returned then.
///
/// @param name pattern of files to search for
/// @param all whether to return all matches or only the first
/// @return list of absolute paths to the found files
// DLLEXPORT ArrayOf(String) nvim_get_runtime_file(String name, Boolean all, Arena *arena,
//                                                 Error *err) FUNC_API_SINCE(7) FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_runtime_file(
    name: NvimString,
    all: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut cookie = RuntimeCookie {
            rv: ArrayBuilder::new(),
            arena,
        };

        let flags = DIP_DIRFILE | if all { DIP_ALL } else { 0 };
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        do_in_runtimepath(
            if name.size != 0 {
                name.data
            } else {
                c"".as_ptr().cast_mut()
            },
            flags,
            RuntimeCallback::FindRuntime(&raw mut cookie),
        );
        try_leave(&tstate, err);

        arena_take_arraybuilder(arena, &raw mut cookie.rv)
    }
}

/// @nodoc
// DLLEXPORT String nvim__get_lib_dir(void) FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__get_lib_dir() -> NvimString {
    unsafe { cstr_as_string(get_lib_dir()) }
}

/// Find files in runtime directories
///
/// @param pat pattern of files to search for
/// @param all whether to return all matches or only the first
/// @param opts is_lua: only search Lua subdirs
/// @return list of absolute paths to the found files
// DLLEXPORT ArrayOf(String) nvim__get_runtime(ArrayOf(String) pat, Boolean all, Dict(runtime) *opts,
//                                             Arena *arena, Error *err) FUNC_API_SINCE(8)
//   FUNC_API_FAST;
#[unsafe(no_mangle)]
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

/// Changes the global working directory.
///
/// @param dir      Directory path
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_current_dir(String dir, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        do_chdir(dir.data, kCdScopeGlobal);
        try_leave(&tstate, err);
    }
}

/// Gets the current line.
///
/// @param[out] err Error details, if any
/// @return Current line string
// DLLEXPORT String nvim_get_current_line(Arena *arena, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Sets the text on the current line.
///
/// @param line     Line contents
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_current_line(String line, Arena *arena,
//                                      Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
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

/// Deletes the current line.
///
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_del_current_line(Arena *arena, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
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

/// Gets a global (g:) variable.
///
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_get_var(String name, Arena *arena, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Sets a global (g:) variable.
///
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_var(String name, Object value, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Removes a global (g:) variable.
///
/// @param name     Variable name
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_del_var(String name, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Gets a v: variable.
///
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return         Variable value
// DLLEXPORT Object nvim_get_vvar(String name, Arena *arena, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_vvar(
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { dict_get_value(get_vimvar_dict(), name, arena, err) }
}

/// Sets a v: variable, if it is not readonly.
///
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_vvar(String name, Object value, Error *err) FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
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

/// Prints a message given by a list of `[text, hl_group]` "chunks". Emits a |Progress| event if
/// `kind='progress'`.
///
/// Returns a message-id, which can be given in later calls to update an existing message. The
/// message-id is an autogenerated integer, or a user-defined string. The id "address space" is
/// global, so plugins specifying a string id should use qualified names such as "my.msg.id" to
/// avoid unintentional conflicts.
///
/// Example:
/// ```lua
/// vim.api.nvim_echo({ { 'chunk1-line1\nchunk1-line2\n' }, { 'chunk2-line1' } }, true)
/// ```
///
/// @param chunks List of `[text, hl_group]` pairs, where each is a `text` string highlighted by
///               the (optional) name or ID `hl_group`.
/// @param history  if true, add to |message-history|.
/// @param opts  Optional parameters.
///          - data (`table?`) Dict of arbitrary data, available in |Progress| |event-data|.
///          - err (`boolean?`)  Treat the message like `:echoerr`. Sets `hl_group` to |hl-ErrorMsg| by default.
///          - id (`integer|string?`) Message-id returned by a previous `nvim_echo` call, or
///            a user-defined id (string). If existing message has this id, it will be updated
///            instead of creating a new message.
///          - kind (`string?`) Decides the |ui-messages| kind in the emitted message. Set "progress"
///            to emit a |progress-message|.
///          - percent (`integer?`) |progress-message| percentage, or nil to signal "unknown progress".
///          - source (`string?`) |progress-message| source.
///          - status (`string?`) |progress-message| status:
///            - "success": Process completed successfully.
///            - "running": Process is ongoing.
///            - "failed": Process failed.
///            - "cancel": Process should be cancelled. Progress owner must handle the |Progress|
///              event to perform the cancellation.
///          - title (`string?`) Message title. Only for |progress-message| currently.
///          - verbose (`boolean?`) Message is controlled by the 'verbose' option. `nvim -V3log` will write the
///            message to the "log" file instead of standard output.
/// @return Message-id, or -1 if message wasn't shown.
// DLLEXPORT Union(Integer, String) nvim_echo(ArrayOf(Tuple(String, *HLGroupID)) chunks,
//                                            Boolean history, Dict(echo_opts) *opts,
//                                            Error *err) FUNC_API_SINCE(7);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_echo(
    chunks: Array,
    history: Boolean,
    opts: *mut KeyDict_echo_opts,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut id = integer_obj(-1);
        let mut hl_msg = parse_hl_msg(chunks, (*opts).err, err);
        if error_set(&*err) {
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
            &mut hl_msg,
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

        id
    }
}

/// Gets the current list of buffers.
///
/// Includes unlisted (unloaded/deleted) buffers, like `:ls!`.
/// Use |nvim_buf_is_loaded()| to check if a buffer is loaded.
///
/// @return List of buffer ids
// DLLEXPORT ArrayOf(Buffer) nvim_list_bufs(Arena *arena) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Gets the current buffer.
///
/// @return Buffer id
// DLLEXPORT Buffer nvim_get_current_buf(void) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_current_buf() -> Buffer {
    unsafe { (*curbuf).b_fnum }
}

/// Sets the current window's buffer to `buf`.
///
/// @param buf   Buffer id
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_current_buf(Buffer buf, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_set_current_buf(buf: Buffer, err: *mut Error) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        do_buffer(DOBUF_GOTO, DOBUF_FIRST, FORWARD, (*b).b_fnum, 0);
        try_leave(&tstate, err);
    }
}

/// Gets the current list of all |window-ID|s in all tabpages.
///
/// @return List of |window-ID|s
// DLLEXPORT ArrayOf(Window) nvim_list_wins(Arena *arena) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Gets the current window.
///
/// @return |window-ID|
// DLLEXPORT Window nvim_get_current_win(void) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_current_win() -> Window {
    unsafe { (*curwin).handle }
}

/// Navigates to the given window (and tabpage, implicitly).
///
/// @param win |window-ID| to focus
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_current_win(Window win, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_set_current_win(win: Window, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        if (*w).w_buffer != curbuf {
            reset_VIsual_and_resel();
        }
        goto_tabpage_win(win_find_tabpage(w), w);
        try_leave(&tstate, err);
    }
}

/// Creates a new, empty, unnamed buffer.
///
/// @param listed Sets 'buflisted'
/// @param scratch Creates a "throwaway" |scratch-buffer| for temporary work
///                (always 'nomodified'). Also sets 'nomodeline' on the buffer.
/// @param[out] err Error details, if any
/// @return Buffer id, or 0 on error
// DLLEXPORT Buffer nvim_create_buf(Boolean listed, Boolean scratch, Error *err) FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_create_buf(
    listed: Boolean,
    scratch: Boolean,
    err: *mut Error,
) -> Buffer {
    unsafe {
        let mut ret: Buffer = 0;
        let mut tstate = TryState::new();
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
                    OptTarget::Buf(buf),
                );
                set_option_direct_for(
                    kOptBuftype,
                    string_obj(static_cstr_as_string(c"nofile")),
                    OPT_LOCAL,
                    0,
                    OptTarget::Buf(buf),
                );
                assert!((*(*buf).b_ml.ml_mfp.cast::<memfile_T>()).mf_fd < 0);
                (*buf).b_p_swf = 0;
                (*buf).b_p_ml = 0;
            }

            unblock_autocmds();

            let mut bufref = bufref_T::new();
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

unsafe fn term_read_pause(_pause: bool, _data: *mut Channel) {}

unsafe fn term_write(buf: *const c_char, size: usize, chan: *mut Channel) {
    unsafe {
        let internal = &raw mut (*chan).internal;
        let cb = (*internal).cb;
        if cb == LUA_NOREF {
            return;
        }
        let prefix = chan;
        let tail = chan;
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

unsafe fn term_resize(_width: u16, _height: u16, _data: *mut Channel) {}

unsafe fn term_resume(_data: *mut Channel) {}

unsafe fn term_close(chan: *mut Channel) {
    unsafe {
        let internal = &raw mut (*chan).internal;
        let tail = chan;
        terminal_destroy((&raw mut (*tail).term).cast());
        api_free_luaref((*internal).cb);
        (*internal).cb = LUA_NOREF;
        channel_decref(chan);
    }
}

/// Open a terminal instance in a buffer
///
/// By default (and currently the only option) the terminal will not be
/// connected to an external process. Instead, input sent on the channel
/// will be echoed directly by the terminal. This is useful to display
/// ANSI terminal sequences returned as part of an RPC message, or similar.
///
/// Note: |:ls| reports the buffer as "R" (running) until the channel is closed. |chanclose()|
///
/// Note: To initialize the terminal size, display the buffer in a window first. E.g. for a floating display,
/// 1. Create an empty buffer using |nvim_create_buf()|.
/// 2. Display it with |nvim_open_win()|.
/// 3. Call nvim_open_term().
/// 4. Then calling |nvim_chan_send()| will process sequences in a virtual terminal with the
///    intended size (defined by the window width/height).
///
/// Example: this `TermHl` command can be used to display and highlight raw ANSI termcodes, so you
/// can use Nvim as a "scrollback pager" (for terminals like kitty): [ansi-colorize]()
/// [terminal-scrollback-pager]()
///
/// ```lua
/// vim.api.nvim_create_user_command('TermHl', function()
///   vim.api.nvim_open_term(0)
/// end, { desc = 'Highlights ANSI termcodes in curbuf' })
/// ```
///
/// @param buf Buffer which displays the PTY output. The initial buffer contents (if any) will be
///            written to the PTY.
/// @param opts   Optional parameters.
///          - force_crlf: (boolean, default: true) Convert "\n" to "\r\n".
///          - on_input: (`fun("input", chan: integer, buf: integer, data: string)`) Function invoked
///            when the terminal emits bytes, see above. |textlock| applies. May call
///            |nvim_chan_send()| directly.
/// @param[out] err Error details, if any
/// @return Channel id, or 0 on error
// DLLEXPORT Integer nvim_open_term(Buffer buf, Dict(open_term) *opts,
//                                  Error *err) FUNC_API_SINCE(7) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
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
        let internal = &raw mut (*chan).internal;
        let prefix = chan;
        let tail = chan;
        (*internal).cb = cb;
        (*internal).closed = false;
        let topts = TerminalOptions {
            data: chan,
            width: ((*curwin).w_view_width - win_col_off(curwin)).max(0) as u16,
            height: (*curwin).w_view_height as u16,
            read_pause_cb: Some(term_read_pause),
            write_cb: Some(term_write),
            resize_cb: Some(term_resize),
            resume_cb: Some(term_resume),
            close_cb: Some(term_close),
            force_crlf,
        };

        let mut contents: Vec<c_char> = Vec::new();
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

        if !contents.is_empty() {
            let mut error: *const c_char = ptr::null();
            channel_send(
                (*prefix).id,
                contents.as_mut_ptr(),
                contents.len(),
                false,
                &mut error,
            );
            if !error.is_null() {
                api_set_error!(err, ErrorType::kErrorTypeValidation, c"%s".as_ptr(), error);
            }
        }

        (*prefix).id as Integer
    }
}

/// Sends raw data to channel `chan`. |channel-bytes|
/// - For a job, it writes it to the stdin of the process.
/// - For the stdio channel |channel-stdio|, it writes to Nvim's stdout.
/// - For an internal terminal instance (|nvim_open_term()|) it writes directly to terminal output.
///
/// This function writes raw data, not RPC messages. Use |vim.rpcrequest()| and |vim.rpcnotify()| if
/// the channel expects RPC messages (i.e. it was created with `rpc=true`).
///
/// To write data to the |TUI| host terminal, see |nvim_ui_send()|.
///
/// @param chan Channel id
/// @param data Data to write. 8-bit clean: may contain NUL bytes.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_chan_send(Integer chan, String data,
//                               Error *err) FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY FUNC_API_LUA_ONLY;
#[unsafe(no_mangle)]
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

/// Gets the current list of |tab-ID|s.
///
/// @return List of |tab-ID|s
// DLLEXPORT ArrayOf(Tabpage) nvim_list_tabpages(Arena *arena) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Gets the current tabpage.
///
/// @return |tab-ID|
// DLLEXPORT Tabpage nvim_get_current_tabpage(void) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_current_tabpage() -> Tabpage {
    unsafe { (*curtab).handle }
}

/// Sets the current tabpage.
///
/// @param tabpage  |tab-ID| to focus
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_current_tabpage(Tabpage tabpage,
//                                         Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_set_current_tabpage(tabpage: Tabpage, err: *mut Error) {
    unsafe {
        let tp = find_tab_by_handle(tabpage, err);
        if tp.is_null() {
            return;
        }
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        goto_tabpage_tp(tp, true, true);
        try_leave(&tstate, err);
    }
}

/// Pastes at cursor (in any mode), and sets "redo" so dot (|.|) will repeat the input. UIs call
/// this to implement "paste", but it's also intended for use by scripts to input large,
/// dot-repeatable blocks of text (as opposed to |nvim_input()| which is subject to mappings/events
/// and is thus much slower).
///
/// Invokes the |vim.paste()| handler, which handles each mode appropriately.
///
/// Errors ('nomodifiable', `vim.paste()` failure, …) are reflected in `err` but do not affect the
/// return value (which is strictly decided by `vim.paste()`).  On error or cancel, subsequent calls
/// are ignored ("drained") until the next paste is initiated (phase 1 or -1).
///
/// Useful in mappings and scripts to insert multiline text. Example:
///
/// ```lua
/// vim.keymap.set('n', 'x', function()
///   vim.api.nvim_paste([[
///     line1
///     line2
///     line3
///   ]], false, -1)
/// end, { buf = 0 })
/// ```
///
/// @param data  Multiline input. Lines break at LF ("\n"). May be binary (containing NUL bytes).
/// @param crlf  Also break lines at CR and CRLF.
/// @param phase  -1: paste in a single call (i.e. without streaming).
///               To "stream" a paste, call `nvim_paste` sequentially with
///               these `phase` values:
///                 - 1: starts the paste (exactly once)
///                 - 2: continues the paste (zero or more times)
///                 - 3: ends the paste (exactly once)
/// @param[out] err Error details, if any
/// @return
///     - true: Client may continue pasting.
///     - false: Client should cancel the paste.
// DLLEXPORT Boolean nvim_paste(uint64_t channel_id, String data, Boolean crlf, Integer phase,
//                              Arena *arena, Error *err) FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
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

/// Puts text at cursor, in any mode. For dot-repeatable input, use |nvim_paste()|.
///
/// Compare |:put| and |p| which are always linewise.
///
/// @param lines  |readfile()|-style list of lines. |channel-lines|
/// @param type  Edit behavior: any |getregtype()| result, or:
///              - "b" |blockwise-visual| mode (may include width, e.g. "b3")
///              - "c" |charwise| mode
///              - "l" |linewise| mode
///              - ""  guess by contents, see |setreg()|
/// @param after  If true insert after cursor (like |p|), or before (like |P|).
/// @param follow  If true place cursor at end of inserted text.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_put(ArrayOf(String) lines, String type, Boolean after, Boolean follow,
//                         Arena *arena, Error *err) FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_put(
    lines: Array,
    reg_type: NvimString,
    after: Boolean,
    follow: Boolean,
    _arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        let mut reg = crate::register::EMPTY_YANKREG;
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

        reg.y_array = Vec::with_capacity(lines.size);
        reg.y_allocated = true;
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
            let copied = copy_string(line, ptr::null_mut());
            reg.y_array.push(copied);
            memchrsub(copied.data.cast(), NUL, NL, line.size);
        }

        finish_yankreg_from_object(&mut reg, false);

        let mut tstate = TryState::new();
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

/// Returns the 24-bit RGB value of a |nvim_get_color_map()| color name or
/// "#rrggbb" hexadecimal string.
///
/// Example:
///
/// ```vim
/// :echo nvim_get_color_by_name("Pink")
/// :echo nvim_get_color_by_name("#cbcbcb")
/// ```
///
/// @param name Color name or "#rrggbb" string
/// @return 24-bit RGB value, or -1 for invalid argument.
// DLLEXPORT Integer nvim_get_color_by_name(String name) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_color_by_name(name: NvimString) -> Integer {
    unsafe {
        let mut dummy: c_int = 0;
        Integer::from(name_to_color(name.data, &mut dummy))
    }
}

/// Returns a map of color names and RGB values.
///
/// Keys are color names (e.g. "Aqua") and values are 24-bit RGB color values
/// (e.g. 65535).
///
/// @return Map of color names and RGB values.
// DLLEXPORT DictOf(Integer) nvim_get_color_map(Arena *arena) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
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

/// Gets a map (msgpack encoding) of the current editor state:
///
/// - "bufs"    [buffer-list]
/// - "funcs"   Vimscript global and [script-local] functions
/// - "gvars"   [global-variable]s
/// - "jumps"   [jumplist]
/// - "regs"    [registers]
/// - "sfuncs"  Vimscript [script-local] functions
///
/// @param opts  Optional parameters.
///               - types:  List of context types (see above) to gather, or empty for "all".
/// @param[out]  err  Error details, if any
///
/// @return Editor state as a msgpack-encoded map.
// DLLEXPORT Dict nvim_get_context(Dict(context) *opts, Arena *arena, Error *err) FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
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

/// Sets the current editor state from the given map.
///
/// @param dict  Msgpack-encoded editor state, in the form returned by [nvim_get_context()].
// DLLEXPORT Object nvim_load_context(Dict dict, Error *err) FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
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

/// Gets the current mode. |mode()|
/// "blocking" is true if Nvim is waiting for input.
///
/// @returns Dict { "mode": String, "blocking": Boolean }
// DLLEXPORT DictAs(get_mode) nvim_get_mode(Arena *arena) FUNC_API_SINCE(2) FUNC_API_FAST;
#[unsafe(no_mangle)]
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

/// Gets a list of global (non-buffer-local) |mapping| definitions.
///
/// @param  mode       Mode short-name ("n", "i", "v", ...)
/// @returns Array of |maparg()|-like dictionaries describing mappings.
///          The "buf" key is always zero.
// DLLEXPORT ArrayOf(DictAs(get_keymap)) nvim_get_keymap(String mode, Arena *arena) FUNC_API_SINCE(3);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_keymap(mode: NvimString, arena: *mut Arena) -> Array {
    unsafe { keymap_array(mode, ptr::null_mut(), arena) }
}

/// Sets a global |mapping| for the given mode.
///
/// To set a buffer-local mapping, use |nvim_buf_set_keymap()|.
///
/// Unlike |:map|, leading/trailing whitespace is accepted as part of the {lhs} or {rhs}.
/// Empty {rhs} is [<Nop>]. |keycodes| are replaced as usual.
///
/// Example:
///
/// ```vim
/// call nvim_set_keymap('n', ' <NL>', '', {'nowait': v:true})
/// ```
///
/// is equivalent to:
///
/// ```vim
/// nmap <nowait> <Space><NL> <Nop>
/// ```
///
/// @param channel_id Channel id (implicit dispatcher arg)
/// @param  mode  Mode short-name (map command prefix: "n", "i", "v", "x", …)
///               or "!" for |:map!|, or empty string for |:map|.
///               "ia", "ca" or "!a" for abbreviation in Insert mode, Cmdline mode, or both, respectively
/// @param  lhs   Left-hand-side |{lhs}| of the mapping.
/// @param  rhs   Right-hand-side |{rhs}| of the mapping.
/// @param  opts  Optional parameters map: Accepts all |:map-arguments| as keys except [<buffer>],
///               values are booleans (default false). Also:
///               - "callback" Lua function called in place of {rhs}.
///               - "desc" human-readable description.
///               - "noremap" disables |recursive_mapping|, like |:noremap|
///               - "replace_keycodes" (boolean) When "expr" is true, replace keycodes in the
///                 resulting string (see |nvim_replace_termcodes()|). Returning nil from the Lua
///                 "callback" is equivalent to returning an empty string.
/// @param[out]   err   Error details, if any.
// DLLEXPORT void nvim_set_keymap(uint64_t channel_id, String mode, String lhs, String rhs,
//                                Dict(keymap) *opts, Error *err) FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
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

/// Unmaps a global |mapping| for the given mode.
///
/// To unmap a buffer-local mapping, use |nvim_buf_del_keymap()|.
///
/// @param  mode  Mode short-name ("n", "i", "v", ...)
/// @param  lhs   Left-hand-side |{lhs}| of the mapping.
/// @param  opts  Optional parameters.
///               - lhs: When true, only match {lhs}, not {rhs}.
///
/// @see |nvim_set_keymap()|
// DLLEXPORT void nvim_del_keymap(uint64_t channel_id, String mode, String lhs, Dict(keymap_del) *opts,
//                                Error *err) FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
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

/// Returns a 2-tuple (Array), where item 0 is the current channel id and item
/// 1 is the |api-metadata| map (Dict).
///
/// @returns 2-tuple `[{channel-id}, {api-metadata}]`
// DLLEXPORT ArrayOf(Object, 2) nvim_get_api_info(uint64_t channel_id,
//                                                Arena *arena) FUNC_API_SINCE(1) FUNC_API_FAST
//   FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_api_info(channel_id: u64, arena: *mut Arena) -> Array {
    unsafe {
        let mut rv = arena_array(arena, 2);
        assert!(channel_id <= (i64::MAX as u64));
        add_c(&mut rv, integer_obj(channel_id as i64));
        add_c(&mut rv, api_metadata());
        rv
    }
}

/// Self-identifies the client, and sets optional flags on the channel. Defines the `client` object
/// returned by |nvim_get_chan_info()|.
///
/// Clients should call this just after connecting, to provide hints for debugging and
/// orchestration. (Note: Something is better than nothing! Fields are optional, but at least set
/// `name`.)
///
/// Can be called more than once; caller should merge old info if appropriate. Example: a library
/// first identifies the channel, then a plugin using that library later identifies itself.
///
/// @param channel_id Channel id (implicit dispatcher arg)
/// @param name Client short-name. Sets the `client.name` field of |nvim_get_chan_info()|.
/// @param version  Dict describing the version, with these
///     (optional) keys:
///     - "commit" hash or similar identifier of commit
///     - "major" major version (defaults to 0 if not set, for no release yet)
///     - "minor" minor version
///     - "patch" patch number
///     - "prerelease" string describing a prerelease, like "dev" or "beta1"
/// @param type Must be one of the following values. Client libraries should
///     default to "remote" unless overridden by the user.
///     - "embedder" application using Nvim as a component (for example,
///                  IDE/editor implementing a vim mode).
///     - "host" plugin host, typically started by nvim
///     - "msgpack-rpc" remote client connected to Nvim via fully MessagePack-RPC
///                     compliant protocol.
///     - "plugin" single plugin, started by nvim
///     - "remote" remote client connected "Nvim flavored" MessagePack-RPC (responses
///                must be in reverse order of requests). |msgpack-rpc|
///     - "ui" gui frontend
/// @param methods Builtin methods in the client. For a host, this does not
///     include plugin methods which will be discovered later.
///     The key should be the method name, the values are dicts with
///     these (optional) keys (more keys may be added in future
///     versions of Nvim, thus unknown keys are ignored. Clients
///     must only use keys defined in this or later versions of
///     Nvim):
///     - "async"  if true, send as a notification. If false or unspecified,
///                use a blocking request
///     - "nargs" Number of arguments. Could be a single integer or an array
///                of two integers, minimum and maximum inclusive.
///
/// @param attributes Arbitrary string:string map of informal client properties.
///     Suggested keys:
///     - "license": License description ("Apache 2", "GPLv3", "MIT", …)
///     - "logo":    URI or path to image, preferably small logo or icon.
///                  .png or .svg format is preferred.
///     - "pid":     Process id.
///     - "website": Client homepage URL (e.g. GitHub repository)
///
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_client_info(uint64_t channel_id, String name, Dict version, String type,
//                                     Dict methods, Dict attributes, Arena *arena,
//                                     Error *err) FUNC_API_SINCE(4) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
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
        let mut info_items = [KeyValuePair::default(); 5];
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

/// Sets channel properties.
///
/// @param chan Channel id, or 0 for current (RPC) channel.
/// @param config Properties:
///     - detach: (boolean, default: true) Closing the channel does not self-exit Nvim. Nvim
///       self-exits when the last `detach=false` |RPC| channel closes. See also |:detach!|.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_chan_set(uint64_t channel_id, Integer chan, Dict(chan_set) *config,
//                              Error *err) FUNC_API_SINCE(15) FUNC_API_FAST;
#[unsafe(no_mangle)]
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
            (*channel).detach = (*config).detach;
            channel_event(channel, EVENT_CHANINFO);
        }
    }
}

/// Records the cmdwin scratchbuf and type, or clears both when type="" / buf=0. Internal use only.
///
/// @param type  ':', '/', '?' (first char only); empty to clear.
/// @param buf   cmdwin buffer id, or 0 to clear.
/// @param[out] err Error details, if any.
// DLLEXPORT void nvim__cmdwin_set(String type, Buffer buf, Error *err) FUNC_API_SINCE(14);
#[unsafe(no_mangle)]
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

/// Gets information about a channel.
///
/// The docs for |nvim_list_uis()| show an example.
///
/// @param chan Channel id, or 0 for current (RPC) channel
/// @returns Channel info dict with these keys:
///    - "argv"     (optional) Job arguments list.
///    - "buf"      (optional) Buffer connected to |terminal| instance.
///    - "buffer"   (optional) Deprecated alias for `buf`.
///    - "client"   (optional) Info about the peer (client on the other end of the channel), as set
///                 by |nvim_set_client_info()|.
///    - "detach"   (optional) Closing the |RPC| channel does not exit Nvim. |nvim_chan_set()|
///    - "exitcode" (optional) Exit code of the |terminal| process.
///    - "id"       Channel id.
///    - "internal" (optional) In-process channel (stream=socket).
///    - "mode"     How data received on the channel is interpreted.
///         - "bytes"      Send and receive raw bytes.
///         - "terminal"   |terminal| instance interprets ASCII sequences.
///         - "rpc"        |RPC| communication on the channel is active.
///    - "pty"      (optional) Name of pseudoterminal. On a POSIX system this is a device path like
///                 "/dev/pts/1". If unknown, the key will still be present if a pty is used (e.g.
///                 for conpty on Windows).
///    - "stream"   Stream underlying the channel.
///         - "stdio"      stdin and stdout of this Nvim instance
///         - "stderr"     stderr of this Nvim instance
///         - "socket"     TCP/IP socket or named pipe
///         - "job"        Job with communication over its stdio.
// DLLEXPORT Dict nvim_get_chan_info(uint64_t channel_id, Integer chan, Arena *arena,
//                                   Error *err) FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
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

/// Get information about all open channels.
///
/// @returns Array of Dictionaries, each describing a channel with
///          the format specified at |nvim_get_chan_info()|.
// DLLEXPORT ArrayOf(Dict) nvim_list_chans(Arena *arena) FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_list_chans(arena: *mut Arena) -> Array {
    unsafe { channel_all_info(arena) }
}

/// Returns object given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  obj  Object to return.
///
/// @return its argument.
// DLLEXPORT Object nvim__id(Object obj, Arena *arena);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__id(obj: Object, arena: *mut Arena) -> Object {
    unsafe { copy_object(obj, arena) }
}

/// Returns array given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  arr  Array to return.
///
/// @return its argument.
// DLLEXPORT Array nvim__id_array(Array arr, Arena *arena);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__id_array(arr: Array, arena: *mut Arena) -> Array {
    unsafe { copy_array(arr, arena) }
}

/// Returns dict given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  dct  Dict to return.
///
/// @return its argument.
// DLLEXPORT Dict nvim__id_dict(Dict dct, Arena *arena);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__id_dict(dct: Dict, arena: *mut Arena) -> Dict {
    unsafe { copy_dict(dct, arena) }
}

/// Returns floating-point value given as argument.
///
/// This API function is used for testing. One should not rely on its presence
/// in plugins.
///
/// @param[in]  flt  Value to return.
///
/// @return its argument.
// DLLEXPORT Float nvim__id_float(Float flt);
#[unsafe(no_mangle)]
pub extern "C" fn nvim__id_float(flt: Float) -> Float {
    flt
}

/// Gets internal stats.
///
/// @return Map of various internal stats.
// DLLEXPORT Dict nvim__stats(Arena *arena);
#[unsafe(no_mangle)]
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

/// Gets a list of dictionaries representing attached UIs.
///
/// Example: The Nvim builtin |TUI| sets its channel info as described in |startup-tui|. In
/// particular, it sets `client.name` to "nvim-tui". So you can check if the TUI is running by
/// inspecting the client name of each UI:
///
/// ```lua
/// vim.print(vim.api.nvim_get_chan_info(vim.api.nvim_list_uis()[1].chan).client.name)
/// ```
///
/// @return Array of UI dictionaries, each with these keys:
///   - "height"  Requested height of the UI
///   - "width"   Requested width of the UI
///   - "rgb"     true if the UI uses RGB colors (false implies |cterm-colors|)
///   - "ext_..." Requested UI extensions, see |ui-option|
///   - "chan"    |channel-id| of remote UI
// DLLEXPORT ArrayOf(Dict) nvim_list_uis(Arena *arena) FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_list_uis(arena: *mut Arena) -> Array {
    unsafe { ui_array(arena) }
}

/// Gets the immediate children of process `pid`.
///
/// @return Array of child process ids, empty if process not found.
// DLLEXPORT Array nvim_get_proc_children(Integer pid, Arena *arena, Error *err) FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_proc_children(
    pid: Integer,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rvobj = Array::new();

        if !validate_int(
            pid > 0 && pid <= Integer::from(c_int::MAX),
            c"pid".as_ptr(),
            pid,
            err,
        ) {
            return rvobj;
        }

        match os_proc_children(pid as c_int) {
            Err(rv) => {
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
            }
            Ok(children) => {
                rvobj = arena_array(arena, children.len());
                for child_pid in children {
                    add_c(&mut rvobj, integer_obj(Integer::from(child_pid)));
                }
            }
        }

        rvobj
    }
}

/// Gets info describing process `pid`.
///
/// @return Map of process properties, or NIL if process not found.
// DLLEXPORT Object nvim_get_proc(Integer pid, Arena *arena, Error *err) FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_proc(pid: Integer, arena: *mut Arena, err: *mut Error) -> Object {
    unsafe {
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
            let rvobj = dict_obj(os_proc_info(pid as c_int, arena));
            if rvobj.data.dict.size == 0 {
                return nil_obj();
            }
            rvobj
        }
        #[cfg(not(windows))]
        {
            let mut rvobj = nil_obj();
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
            rvobj
        }
    }
}

/// Selects an item in the completion popup menu.
///
/// If neither |ins-completion| nor |cmdline-completion| popup menu is active
/// this API call is silently ignored.
/// Useful for an external UI using |ui-popupmenu| to control the popup menu with the mouse.
/// Can also be used in a mapping; use [<Cmd>] |:map-cmd| or a Lua mapping to ensure the mapping
/// doesn't end completion mode.
///
/// @param item    Index (zero-based) of the item to select. Value of -1 selects nothing
///                and restores the original text.
/// @param insert  For |ins-completion|, whether the selection should be inserted in the buffer.
///                Ignored for |cmdline-completion|.
/// @param finish  Finish the completion and dismiss the popup menu. Implies {insert}.
/// @param opts    Optional parameters. Reserved for future use.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_select_popupmenu_item(Integer item, Boolean insert, Boolean finish,
//                                           Dict(empty) *opts, Error *err) FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
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

/// NB: if your UI doesn't use hlstate, this will not return hlstate first time.
// DLLEXPORT Array nvim__inspect_cell(Integer grid, Integer row, Integer col, Arena *arena,
//                                    Error *err);
#[unsafe(no_mangle)]
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
                !wp.is_null() && !(*wp).w_grid_alloc.chars.is_empty(),
                c"grid handle".as_ptr(),
                grid,
                err,
            ) {
                return ret;
            }
            g = &raw mut (*wp).w_grid_alloc;
        }

        let g = &*g;
        if row < 0 || row >= Integer::from(g.rows) || col < 0 || col >= Integer::from(g.cols) {
            return ret;
        }
        ret = arena_array(arena, 3);
        let off = g.line_offset[row as usize] + (col as usize);
        let sc_buf = arena_alloc(arena, MAX_SCHAR_SIZE, false).cast::<c_char>();
        schar_get(sc_buf, g.chars[off]);
        add_c(&mut ret, cstr_as_obj(sc_buf));
        let attr = g.attrs[off];
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

/// @nodoc
// DLLEXPORT void nvim__screenshot(String path) FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__screenshot(path: NvimString) {
    unsafe {
        ui_call_screenshot(path);
    }
}

/// For testing. The condition in schar_cache_clear_if_full is hard to
/// reach, so this function can be used to force a cache clear in a test.
// DLLEXPORT void nvim__invalidate_glyph_cache(void);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__invalidate_glyph_cache() {
    unsafe {
        schar_cache_clear();
        must_redraw = UPD_CLEAR;
    }
}

/// @nodoc
// DLLEXPORT Object nvim__unpack(String str, Arena *arena, Error *err) FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__unpack(
    str_: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { unpack(str_.data, str_.size, arena, err) }
}

/// Deletes an uppercase/file named mark. See |mark-motions|.
///
/// @note Lowercase name (or other buffer-local mark) is an error.
/// @param name       Mark name
/// @return true if the mark was deleted, else false.
/// @see |nvim_buf_del_mark()|
/// @see |nvim_get_mark()|
// DLLEXPORT Boolean nvim_del_mark(String name, Error *err) FUNC_API_SINCE(8);
#[unsafe(no_mangle)]
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

/// Returns a `(row, col, buffer, buffername)` tuple representing the position
/// of the uppercase/file named mark. "End of line" column position is returned
/// as |v:maxcol| (big number). See |mark-motions|.
///
/// Marks are (1,0)-indexed. |api-indexing|
///
/// @note Lowercase name (or other buffer-local mark) is an error.
/// @param name       Mark name
/// @param opts       Optional parameters. Reserved for future use.
/// @return 4-tuple (row, col, buffer, buffername), (0, 0, 0, '') if the mark is
/// not set.
/// @see |nvim_buf_set_mark()|
/// @see |nvim_del_mark()|
// DLLEXPORT Tuple(Integer, Integer, Buffer, String) nvim_get_mark(String name, Dict(empty) *opts,
//                                                                 Arena *arena,
//                                                                 Error *err) FUNC_API_SINCE(8);
#[unsafe(no_mangle)]
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
                xfree(filename.cast());
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
            xfree(filename.cast());
        }

        rv
    }
}

/// Evaluates statusline string.
///
/// @param str Statusline string (see 'statusline').
/// @param opts Optional parameters.
///           - fillchar: (string) Character to fill blank spaces in the statusline (see
///                                'fillchars'). Treated as single-width even if it isn't.
///           - highlights: (boolean) Return highlight information.
///           - maxwidth: (number) Maximum width of statusline.
///           - use_statuscol_lnum: (number) Evaluate statuscolumn for this line number instead of statusline.
///           - use_tabline: (boolean) Evaluate tabline instead of statusline. When true, {winid}
///                                    is ignored. Mutually exclusive with {use_winbar}.
///           - use_winbar: (boolean) Evaluate winbar instead of statusline.
///           - winid: (number) |window-ID| of the window to use as context for statusline.
///
/// @param[out] err Error details, if any.
/// @return Dict containing statusline information, with these keys:
///       - str: (string) Characters that will be displayed on the statusline.
///       - width: (number) Display width of the statusline.
///       - highlights: Array containing highlight information of the statusline. Only included when
///                     the "highlights" key in {opts} is true. Each element of the array is a
///                     |Dict| with these keys:
///           - start: (number) Byte index (0-based) of first character that uses the highlight.
///           - group: (string) Deprecated. Use `groups` instead.
///           - groups: (array) Names of stacked highlight groups (highest priority last).
// DLLEXPORT DictAs(eval_statusline_ret) nvim_eval_statusline(String str, Dict(eval_statusline) *opts,
//                                                            Arena *arena,
//                                                            Error *err) FUNC_API_SINCE(8)
//   FUNC_API_FAST;
#[unsafe(no_mangle)]
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
        let mut statuscol = statuscol_T::new();
        let mut sattrs = [crate::decoration::SignTextAttrs {
            text: [0; 2],
            hl_id: 0,
        }; SIGN_SHOW_MAX];

        if statuscol_lnum != 0 {
            let mut line_id: c_int = 0;
            let mut cul_id: c_int = 0;
            let mut num_id: c_int = 0;
            let lnum = statuscol_lnum as linenr_T;
            let mut cursorline_fi = foldinfo_T::new();
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

                let combine = if (*sp).item == c_int::from(STL_SIGNCOL as u8) {
                    syn_id2name(scl_hl_id)
                } else if (*sp).item == c_int::from(STL_FOLDCOL as u8) {
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

/// Sets info for the completion item at the given index. If the info text was shown in a window,
/// returns the window and buffer ids, or empty dict if not shown.
///
/// @param index  Completion candidate index
/// @param opts   Optional parameters.
///       - info: (string) info text.
/// @return Dict containing these keys:
///       - winid: (integer) floating window id
///       - bufnr: (integer) buffer id in floating window
// DLLEXPORT DictOf(Integer) nvim__complete_set(Integer index, Dict(complete_set) *opts, Arena *arena,
//                                              Error *err);
#[unsafe(no_mangle)]
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

/// Instruct Nvim to redraw various components.
///
/// @see |:redraw|
///
/// @param opts  Optional parameters.
///               - buf: Target a specific buffer number as described below.
///               - cursor: Immediately update cursor position on the screen in
///                 `win` or the current window.
///               - flush: Update the screen with pending updates.
///               - range: Redraw a range in `buf`, the buffer in `win` or the
///                 current buffer (useful for decoration providers). Expects a
///                 tuple `[first, last]` with the first and last line number
///                 of the range, 0-based end-exclusive |api-indexing|.
///               - statuscolumn: Redraw the 'statuscolumn' in `buf`, `win` or
///                 all windows.
///               - statusline: Redraw the 'statusline' in `buf`, `win` or all
///                 windows.
///               - tabline: Redraw the 'tabline'.
///               - valid: When present mark `win`, `buf`, or all windows for
///                 redraw. When `true`, only redraw changed lines (useful for
///                 decoration providers). When `false`, forcefully redraw.
///               - win: Target a specific |window-ID| as described below.
///               - winbar: Redraw the 'winbar' in `buf`, `win` or all windows.
// DLLEXPORT void nvim__redraw(Dict(redraw) *opts, Error *err) FUNC_API_SINCE(12);
#[unsafe(no_mangle)]
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

// DLLEXPORT void nvim__set_restart_on_crash(String progpath, Array argv);
#[unsafe(no_mangle)]
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
