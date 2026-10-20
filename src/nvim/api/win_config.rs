//! Window configuration API functions (`src/nvim/api/win_config.c`).

#![allow(
    static_mut_refs,
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::autocmd::{OptionalKeys, TryState};
use crate::api::deprecated::{VirtText, VirtTextChunk};
use crate::api::extmark::{parse_virt_text, virt_text_to_array};
use crate::api::options::CtxSwitch;
use crate::api::private::helpers::{
    add_c, api_clear_error, api_free_array, api_free_object, api_set_error, api_typename,
    arena_array, array_obj, cbuf_to_string, cstr_as_string, error_set, find_buffer_by_handle,
    find_window_by_handle, integer_obj, object_to_hl_id, string_obj, try_enter, try_leave,
};
use crate::api::private::validate::{
    Array, Error, ErrorType, Object, ObjectData, ObjectType, api_err_conflict, api_err_exp,
    api_err_invalid, api_err_required,
};
use crate::api::tabpage::{Buffer, WinConfig, Window, frame_T, lpos_T, tabpage_T, win_T};
use crate::autocmd::{apply_autocmds_win, block_autocmds, unblock_autocmds};
use crate::buffer::{bufref_valid, set_bufref};
use crate::charset::transstr;
use crate::context::{ctx_restore, ctx_switch, is_ctx_win};
use crate::drawscreen::{redraw_later, set_must_redraw};
use crate::errors::*;
use crate::ex_docmd::expr_map_locked;
use crate::globals::*;
use crate::highlight_group::{syn_check_group, syn_id2name};
use crate::lua::converter::Boolean;
use crate::map_glyph_cache::NvimString;
use crate::mbyte::{mb_string2cells, mb_string2cells_len};
use crate::memory::{Arena, strequal};
use crate::r#move::changed_window_setting;
use crate::msgpack_rpc::packer::Integer;
use crate::option::didset_window_options;
use crate::option_vars::*;
use crate::ui::ui_has;
use crate::ui_compositor::ui_comp_remove_grid;
use crate::window::{
    check_split_disallowed_err, clear_float_config, goto_tabpage_win, last_status,
    lastwin_nofloating, merge_win_config, one_window, win_append, win_comp_pos, win_find_tabpage,
    win_goto, win_locked, win_remove, win_set_buf, win_setheight_win, win_setwidth_win,
    win_split_ins, win_valid, win_valid_any_tab, window_layout_locked_err, winframe_find_altwin,
    winframe_remove, winframe_restore,
};
use crate::winfloat::{
    win_config_float, win_float_find_altwin, win_new_float, win_set_minimal_style,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type FloatAnchor = c_int;
pub type FloatRelative = c_int;
pub type WinSplit = c_int;
pub type WinStyle = c_int;
pub type AlignTextPos = c_int;
pub type BorderTextType = c_int;
pub use crate::api::autocmd::event_T;
pub use crate::api::command::cmdidx_T;
pub type UIExtension = c_int;

const NUL: c_char = 0;
const MAX_SCHAR_SIZE: usize = 32;
const EVENT_WINNEW: event_T = 146;
const CMD_SIZE: cmdidx_T = 564;
const kUIMultigrid: UIExtension = 6;
const kZIndexFloatDefault: c_int = 50;

const FR_COL: c_char = 2;

const WSP_VERT: c_int = 0x02;
const WSP_HOR: c_int = 0x04;
const WSP_TOP: c_int = 0x08;
const WSP_BOT: c_int = 0x10;
const WSP_BELOW: c_int = 0x40;
const WSP_ABOVE: c_int = 0x80;
const WSP_NOENTER: c_int = 0x200;

const UPD_NOT_VALID: c_int = 40;

const kCtxNoDisplay: c_int = 4;
const kCtxNoEvents: c_int = 8;

const kFloatAnchorEast: FloatAnchor = 1;
const kFloatAnchorSouth: FloatAnchor = 2;

const kFloatRelativeEditor: FloatRelative = 0;
const kFloatRelativeWindow: FloatRelative = 1;
const kFloatRelativeCursor: FloatRelative = 2;
const kFloatRelativeMouse: FloatRelative = 3;
const kFloatRelativeTabline: FloatRelative = 4;
const kFloatRelativeLaststatus: FloatRelative = 5;

const kWinSplitLeft: WinSplit = 0;
const kWinSplitRight: WinSplit = 1;
const kWinSplitAbove: WinSplit = 2;
const kWinSplitBelow: WinSplit = 3;

const kWinStyleUnused: WinStyle = 0;
const kWinStyleMinimal: WinStyle = 1;

const kAlignLeft: AlignTextPos = 0;
const kAlignCenter: AlignTextPos = 1;
const kAlignRight: AlignTextPos = 2;

const kBorderTextTitle: BorderTextType = 0;
const kBorderTextFooter: BorderTextType = 1;

const KEYSET_OPTIDX_win_config__col: u64 = 1;
const KEYSET_OPTIDX_win_config__row: u64 = 2;
const KEYSET_OPTIDX_win_config__win: u64 = 3;
const KEYSET_OPTIDX_win_config__hide: u64 = 4;
const KEYSET_OPTIDX_win_config__width: u64 = 5;
const KEYSET_OPTIDX_win_config__split: u64 = 6;
const KEYSET_OPTIDX_win_config__title: u64 = 7;
const KEYSET_OPTIDX_win_config__mouse: u64 = 8;
const KEYSET_OPTIDX_win_config__fixed: u64 = 9;
const KEYSET_OPTIDX_win_config__style: u64 = 10;
const KEYSET_OPTIDX_win_config__anchor: u64 = 11;
const KEYSET_OPTIDX_win_config__bufpos: u64 = 12;
const KEYSET_OPTIDX_win_config__height: u64 = 13;
const KEYSET_OPTIDX_win_config__zindex: u64 = 14;
const KEYSET_OPTIDX_win_config__footer: u64 = 15;
const KEYSET_OPTIDX_win_config__border: u64 = 16;
const KEYSET_OPTIDX_win_config__external: u64 = 17;
const KEYSET_OPTIDX_win_config__relative: u64 = 18;
const KEYSET_OPTIDX_win_config__vertical: u64 = 19;
const KEYSET_OPTIDX_win_config__focusable: u64 = 20;
const KEYSET_OPTIDX_win_config__noautocmd: u64 = 21;
const KEYSET_OPTIDX_win_config__title_pos: u64 = 22;
const KEYSET_OPTIDX_win_config__footer_pos: u64 = 23;
const KEYSET_OPTIDX_win_config___cmdline_offset: u64 = 24;

pub use crate::api::options::bufref_T;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_win_config {
    pub is_set__win_config_: OptionalKeys,
    pub external: Boolean,
    pub fixed: Boolean,
    pub focusable: Boolean,
    pub footer: Object,
    pub footer_pos: NvimString,
    pub hide: Boolean,
    pub height: Integer,
    pub mouse: Boolean,
    pub relative: NvimString,
    pub row: f64,
    pub style: NvimString,
    pub noautocmd: Boolean,
    pub vertical: Boolean,
    pub win: Window,
    pub width: Integer,
    pub zindex: Integer,
    pub anchor: NvimString,
    pub border: Object,
    pub bufpos: Array,
    pub col: f64,
    pub split: NvimString,
    pub title: Object,
    pub title_pos: NvimString,
    pub _cmdline_offset: Integer,
}

const WIN_CONFIG_INIT: WinConfig = WinConfig {
    window: 0,
    bufpos: lpos_T { lnum: -1, col: 0 },
    height: 0,
    width: 0,
    row: 0.0,
    col: 0.0,
    anchor: 0,
    relative: 0,
    external: false,
    focusable: true,
    mouse: true,
    split: 0,
    zindex: kZIndexFloatDefault,
    style: kWinStyleUnused,
    border: false,
    shadow: false,
    border_chars: [[0; MAX_SCHAR_SIZE]; 8],
    border_hl_ids: [0; 8],
    border_attr: [0; 8],
    title: false,
    title_pos: 0,
    title_chunks: VirtText::new(),
    title_width: 0,
    footer: false,
    footer_pos: 0,
    footer_chunks: VirtText::new(),
    footer_width: 0,
    noautocmd: false,
    fixed: false,
    hide: false,
    _cmdline_offset: c_int::MAX,
};

const NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
unsafe fn cstrn_as_string(s: *const c_char, maxsize: usize) -> NvimString {
    if s.is_null() {
        return NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
    }
    let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), maxsize) };
    let len = slice.iter().position(|&b| b == 0).unwrap_or(maxsize);
    NvimString {
        data: s as *mut c_char,
        size: len,
    }
}

#[inline]
unsafe fn cstr_to_obj(s: *const c_char) -> Object {
    unsafe {
        let len = CStr::from_ptr(s).to_bytes().len();
        string_obj(cbuf_to_string(s, len))
    }
}

#[inline]
unsafe fn strequal_str(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == b.as_bytes() }
}

#[inline]
unsafe fn striequal_str(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe {
        CStr::from_ptr(a)
            .to_bytes()
            .eq_ignore_ascii_case(b.as_bytes())
    }
}

unsafe fn try_wrap<F: FnOnce()>(err: *mut Error, f: F) {
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        f();
        try_leave(&tstate, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_open_win(
    buf: Buffer,
    enter: Boolean,
    config: *mut KeyDict_win_config,
    err: *mut Error,
) -> Window {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return 0;
        }

        let mask = (*config).is_set__win_config_;
        let is_split = has_key(mask, KEYSET_OPTIDX_win_config__split)
            || has_key(mask, KEYSET_OPTIDX_win_config__vertical);
        let mut fconfig = WIN_CONFIG_INIT;
        if !parse_win_config(ptr::null_mut(), config, &mut fconfig, false, !is_split, err) {
            return 0;
        }

        let mut rv: Window = 0;
        if fconfig.noautocmd {
            block_autocmds();
        }

        'cleanup: {
            let mut wp: *mut win_T = ptr::null_mut();
            let mut tp = curtab;
            assert!(!curwin.is_null());
            let mut parent = if (*config).win == 0 {
                curwin
            } else {
                ptr::null_mut()
            };
            if (*config).win > 0 {
                parent = find_window_by_handle(fconfig.window, err);
                if parent.is_null() {
                    break 'cleanup;
                } else if is_split && (*parent).w_floating {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Cannot split a floating window".as_ptr(),
                    );
                    break 'cleanup;
                }
                tp = win_find_tabpage(parent);
            }
            if is_split {
                if !check_split_disallowed_err(if !parent.is_null() { parent } else { curwin }, err)
                {
                    break 'cleanup;
                }

                if has_key(mask, KEYSET_OPTIDX_win_config__vertical)
                    && !has_key(mask, KEYSET_OPTIDX_win_config__split)
                {
                    if (*config).vertical {
                        fconfig.split = if p_spr != 0 {
                            kWinSplitRight
                        } else {
                            kWinSplitLeft
                        };
                    } else {
                        fconfig.split = if p_sb != 0 {
                            kWinSplitBelow
                        } else {
                            kWinSplitAbove
                        };
                    }
                }
                let flags = win_split_flags(fconfig.split, parent.is_null()) | WSP_NOENTER;
                let size = if (flags & WSP_VERT) != 0 {
                    fconfig.width
                } else {
                    fconfig.height
                };

                try_wrap(err, || {
                    if parent.is_null() || parent == curwin {
                        wp = win_split_ins(size, flags, ptr::null_mut(), 0, ptr::null_mut());
                    } else {
                        let mut switchwin: CtxSwitch = std::mem::zeroed();
                        let result = ctx_switch(
                            &mut switchwin,
                            parent,
                            tp,
                            ptr::null_mut(),
                            kCtxNoEvents | kCtxNoDisplay,
                        );
                        assert!(result);
                        wp = win_split_ins(size, flags, ptr::null_mut(), 0, ptr::null_mut());
                        ctx_restore(&mut switchwin);
                    }
                });
                if !wp.is_null() {
                    (*wp).w_config = fconfig;
                    if size > 0 {
                        if (flags & WSP_VERT) != 0 && (*wp).w_width != size {
                            win_setwidth_win(size, wp, true);
                        } else if (flags & WSP_VERT) == 0 && (*wp).w_height != size {
                            win_setheight_win(size, wp, true);
                        }
                    }
                }
            } else {
                if (*(*curwin).w_buffer).b_locked_split != 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"E1159: Cannot open a float when closing the buffer".as_ptr(),
                    );
                    break 'cleanup;
                }
                wp = win_new_float(ptr::null_mut(), false, fconfig, err);
            }
            if wp.is_null() {
                if !error_set(err) {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to create window".as_ptr(),
                    );
                }
                break 'cleanup;
            }

            if fconfig._cmdline_offset < c_int::MAX {
                cmdline_win = wp;
            }

            let mut bufref = bufref_T {
                br_buf: ptr::null_mut(),
                br_fnum: 0,
                br_buf_free_count: 0,
            };
            set_bufref(&mut bufref, b);
            if !fconfig.noautocmd {
                let mut switchwin: CtxSwitch = std::mem::zeroed();
                let result = ctx_switch(&mut switchwin, wp, tp, ptr::null_mut(), kCtxNoDisplay);
                assert!(result);
                if apply_autocmds_win(
                    EVENT_WINNEW,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    curbuf,
                    wp,
                ) {
                    tp = win_find_tabpage(wp);
                }
                ctx_restore(&mut switchwin);
            }
            if !tp.is_null() && enter {
                goto_tabpage_win(tp, wp);
                tp = win_find_tabpage(wp);
            }
            if !tp.is_null() && bufref_valid(&mut bufref) && b != (*wp).w_buffer {
                let au_no_enter_leave = curwin != wp && !fconfig.noautocmd;
                if au_no_enter_leave {
                    autocmd_no_enter += 1;
                    autocmd_no_leave += 1;
                }
                win_set_buf(wp, b, err);
                if !fconfig.noautocmd {
                    tp = win_find_tabpage(wp);
                }
                if au_no_enter_leave {
                    autocmd_no_enter -= 1;
                    autocmd_no_leave -= 1;
                }
            }
            if tp.is_null() {
                api_clear_error(err);
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Window was closed immediately".as_ptr(),
                );
                break 'cleanup;
            }

            if fconfig.style == kWinStyleMinimal {
                win_set_minimal_style(wp);
                didset_window_options(wp, true);
                changed_window_setting(wp);
            }
            rv = (*wp).handle;
        }

        if fconfig.noautocmd {
            unblock_autocmds();
        }
        rv
    }
}

unsafe fn win_split_dir(win: *mut win_T) -> WinSplit {
    unsafe {
        if (*win).w_frame.is_null() || (*(*win).w_frame).fr_parent.is_null() {
            return kWinSplitLeft;
        }

        let layout = (*(*(*win).w_frame).fr_parent).fr_layout;
        if layout == FR_COL {
            if !(*(*win).w_frame).fr_next.is_null() {
                kWinSplitAbove
            } else {
                kWinSplitBelow
            }
        } else if !(*(*win).w_frame).fr_next.is_null() {
            kWinSplitLeft
        } else {
            kWinSplitRight
        }
    }
}

const fn win_split_flags(split: WinSplit, toplevel: bool) -> c_int {
    let mut flags = 0;
    if split == kWinSplitAbove || split == kWinSplitBelow {
        flags |= WSP_HOR;
    } else {
        flags |= WSP_VERT;
    }
    if split == kWinSplitAbove || split == kWinSplitLeft {
        flags |= if toplevel { WSP_TOP } else { WSP_ABOVE };
    } else {
        flags |= if toplevel { WSP_BOT } else { WSP_BELOW };
    }
    flags
}

unsafe fn win_can_move_tp(wp: *mut win_T, tp: *mut tabpage_T, err: *mut Error) -> bool {
    unsafe {
        if one_window(wp, if tp == curtab { ptr::null_mut() } else { tp }) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Cannot move last non-floating window".as_ptr(),
            );
            return false;
        }
        if win_locked(wp) != 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Cannot move window to another tabpage whilst in use".as_ptr(),
            );
            return false;
        }
        if window_layout_locked_err(crate::cmdexpand::cmdidx_T::CMD_SIZE, err) {
            return false;
        }
        if textlock != 0 || expr_map_locked() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"%s".as_ptr(),
                e_textlock.as_ptr(),
            );
            return false;
        }
        if is_ctx_win(wp) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Cannot move autocmd window to another tabpage".as_ptr(),
            );
            return false;
        }
        true
    }
}

unsafe fn win_find_altwin(win: *mut win_T, tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        let tp_arg = if tp == curtab { ptr::null_mut() } else { tp };
        if (*win).w_floating {
            win_float_find_altwin(win, tp_arg)
        } else {
            let mut dir: c_int = 0;
            winframe_find_altwin(win, &mut dir, tp_arg, ptr::null_mut())
        }
    }
}

unsafe fn win_config_split(
    win: *mut win_T,
    config: *const KeyDict_win_config,
    fconfig: *mut WinConfig,
    err: *mut Error,
) -> bool {
    unsafe {
        let was_split = !(*win).w_floating;
        let mask = (*config).is_set__win_config_;
        let has_split = has_key(mask, KEYSET_OPTIDX_win_config__split);
        let has_vertical = has_key(mask, KEYSET_OPTIDX_win_config__vertical);
        let old_split = win_split_dir(win);
        if has_vertical && !has_split {
            if (*config).vertical {
                (*fconfig).split = if old_split == kWinSplitRight || p_spr != 0 {
                    kWinSplitRight
                } else {
                    kWinSplitLeft
                };
            } else {
                (*fconfig).split = if old_split == kWinSplitBelow || p_sb != 0 {
                    kWinSplitBelow
                } else {
                    kWinSplitAbove
                };
            }
        }

        'resize: {
            if (!has_vertical && !has_split)
                || (was_split
                    && !has_key(mask, KEYSET_OPTIDX_win_config__win)
                    && old_split == (*fconfig).split)
            {
                break 'resize;
            }

            let mut parent: *mut win_T = ptr::null_mut();
            let mut parent_tp: *mut tabpage_T = ptr::null_mut();
            if (*config).win == 0 {
                parent = curwin;
                parent_tp = curtab;
            } else if (*config).win > 0 {
                parent = find_window_by_handle((*fconfig).window, err);
                if parent.is_null() {
                    return false;
                }
                parent_tp = win_find_tabpage(parent);
            }

            let mut win_tp = win_find_tabpage(win);
            if !parent.is_null() {
                if (*parent).w_floating {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Cannot split a floating window".as_ptr(),
                    );
                    return false;
                }
                if win_tp != parent_tp && !win_can_move_tp(win, win_tp, err) {
                    return false;
                }
            }

            if !check_split_disallowed_err(win, err) {
                return false;
            }

            let mut to_split_ok = false;
            let curwin_moving_tp = win == curwin && !parent.is_null() && win_tp != parent_tp;

            'restore_curwin: {
                if curwin_moving_tp {
                    let altwin = win_find_altwin(win, win_tp);
                    assert!(!altwin.is_null());
                    win_goto(altwin);

                    if curwin == win {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Failed to switch away from window %d".as_ptr(),
                            (*win).handle,
                        );
                        return false;
                    }
                    win_tp = win_find_tabpage(win);
                    if win_tp.is_null() || !win_valid_any_tab(parent) {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Windows to split were closed".as_ptr(),
                        );
                        break 'restore_curwin;
                    }
                    if was_split == (*win).w_floating || (*parent).w_floating {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Floating state of windows to split changed".as_ptr(),
                        );
                        break 'restore_curwin;
                    }
                }

                let mut dir: c_int = 0;
                let mut unflat_altfr: *mut frame_T = ptr::null_mut();
                let altwin: *mut win_T;

                if was_split {
                    if (*(*win).w_frame).fr_parent.is_null() {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Cannot move last non-floating window".as_ptr(),
                        );
                        break 'restore_curwin;
                    } else if !parent.is_null() && (*parent).handle == (*win).handle {
                        let mut n_frames = 0;
                        let mut fr = (*(*(*win).w_frame).fr_parent).fr_child;
                        while !fr.is_null() {
                            n_frames += 1;
                            fr = (*fr).fr_next;
                        }

                        let mut neighbor: *mut win_T = ptr::null_mut();

                        if n_frames > 2 {
                            let frame = (*(*win).w_frame).fr_parent;
                            if !(*frame).fr_parent.is_null() {
                                neighbor = if (*fconfig).split == kWinSplitAbove
                                    || (*fconfig).split == kWinSplitLeft
                                {
                                    (*win).w_next
                                } else {
                                    (*win).w_prev
                                };
                            }
                            altwin = winframe_remove(
                                win,
                                &mut dir,
                                if win_tp == curtab {
                                    ptr::null_mut()
                                } else {
                                    win_tp
                                },
                                &mut unflat_altfr,
                            );
                        } else if n_frames == 2 {
                            altwin = winframe_remove(
                                win,
                                &mut dir,
                                if win_tp == curtab {
                                    ptr::null_mut()
                                } else {
                                    win_tp
                                },
                                &mut unflat_altfr,
                            );
                            neighbor = altwin;
                        } else {
                            api_set_error!(
                                err,
                                ErrorType::kErrorTypeException,
                                c"Cannot split window into itself".as_ptr(),
                            );
                            break 'restore_curwin;
                        }
                        parent = neighbor;
                    } else {
                        altwin = winframe_remove(
                            win,
                            &mut dir,
                            if win_tp == curtab {
                                ptr::null_mut()
                            } else {
                                win_tp
                            },
                            &mut unflat_altfr,
                        );
                    }
                } else {
                    altwin = win_float_find_altwin(
                        win,
                        if win_tp == curtab {
                            ptr::null_mut()
                        } else {
                            win_tp
                        },
                    );
                }

                win_remove(
                    win,
                    if win_tp == curtab {
                        ptr::null_mut()
                    } else {
                        win_tp
                    },
                );
                if win_tp == curtab {
                    last_status(false);
                    win_comp_pos();
                }

                let flags = win_split_flags((*fconfig).split, parent.is_null()) | WSP_NOENTER;
                parent_tp = if !parent.is_null() {
                    win_find_tabpage(parent)
                } else {
                    curtab
                };

                try_wrap(err, || {
                    let need_switch = !parent.is_null() && parent != curwin;
                    let mut switchwin: CtxSwitch = std::mem::zeroed();
                    if need_switch {
                        let result = ctx_switch(
                            &mut switchwin,
                            parent,
                            parent_tp,
                            ptr::null_mut(),
                            kCtxNoEvents | kCtxNoDisplay,
                        );
                        assert!(result);
                    }
                    to_split_ok = !win_split_ins(0, flags, win, 0, unflat_altfr).is_null();
                    if !to_split_ok {
                        win_append(
                            (*win).w_prev,
                            win,
                            if win_tp == curtab {
                                ptr::null_mut()
                            } else {
                                win_tp
                            },
                        );
                    }
                    if need_switch {
                        ctx_restore(&mut switchwin);
                    }
                });
                if !to_split_ok {
                    if was_split {
                        winframe_restore(win, dir, unflat_altfr);
                    }
                    if !error_set(err) {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Failed to move window %d into split".as_ptr(),
                            (*win).handle,
                        );
                    }
                    break 'restore_curwin;
                }

                if win_tp != parent_tp && (*win_tp).tp_curwin == win {
                    (*win_tp).tp_curwin = altwin;
                }
                break 'resize;
            }

            if curwin_moving_tp && win_valid(win) {
                win_goto(win);
            }
            return false;
        }

        if has_key(mask, KEYSET_OPTIDX_win_config__width) {
            win_setwidth_win((*fconfig).width, win, true);
        }
        if has_key(mask, KEYSET_OPTIDX_win_config__height) {
            win_setheight_win((*fconfig).height, win, true);
        }

        if !was_split {
            clear_float_config(fconfig, false);
        }
        merge_win_config(&mut (*win).w_config, *fconfig);
        true
    }
}

unsafe fn win_config_float_tp(
    win: *mut win_T,
    config: *const KeyDict_win_config,
    fconfig: *const WinConfig,
    err: *mut Error,
) -> bool {
    unsafe {
        let mut win_tp = win_find_tabpage(win);
        let mut parent = win;
        let mut parent_tp = win_tp;
        if has_key((*config).is_set__win_config_, KEYSET_OPTIDX_win_config__win) {
            parent = find_window_by_handle((*fconfig).window, err);
            if parent.is_null() {
                return false;
            }
            parent_tp = win_find_tabpage(parent);
        }

        let mut curwin_moving_tp = false;
        let mut altwin: *mut win_T = ptr::null_mut();

        'restore_curwin: {
            if win_tp != parent_tp {
                if !win_can_move_tp(win, win_tp, err) {
                    return false;
                }
                altwin = win_find_altwin(win, win_tp);
                assert!(!altwin.is_null());

                if curwin == win {
                    curwin_moving_tp = true;
                    win_goto(altwin);

                    if curwin == win {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Failed to switch away from window %d".as_ptr(),
                            (*win).handle,
                        );
                        return false;
                    }
                    win_tp = win_find_tabpage(win);
                    parent_tp = win_find_tabpage(parent);

                    if win_tp.is_null() || parent_tp.is_null() {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Target windows were closed".as_ptr(),
                        );
                        break 'restore_curwin;
                    }
                    if win_tp != parent_tp && !win_can_move_tp(win, win_tp, err) {
                        break 'restore_curwin;
                    }
                    altwin = win_find_altwin(win, win_tp);
                    assert!(!altwin.is_null());
                }
            }

            if !(*win).w_floating {
                if win_new_float(win, false, *fconfig, err).is_null() {
                    break 'restore_curwin;
                }
                redraw_later(win, UPD_NOT_VALID);
            }

            if win_tp != parent_tp {
                win_remove(
                    win,
                    if win_tp == curtab {
                        ptr::null_mut()
                    } else {
                        win_tp
                    },
                );
                let append_tp = if parent_tp == curtab {
                    ptr::null_mut()
                } else {
                    parent_tp
                };
                win_append(lastwin_nofloating(append_tp), win, append_tp);

                if win_tp != curtab && (*win_tp).tp_curwin == win {
                    (*win_tp).tp_curwin = altwin;
                }

                ui_comp_remove_grid(&mut (*win).w_grid_alloc);

                redraw_later(win, UPD_NOT_VALID);
                set_must_redraw(UPD_NOT_VALID);
            }

            win_config_float(win, *fconfig);
            return true;
        }

        if curwin_moving_tp && win_valid(win) {
            win_goto(win);
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_set_config(
    win: Window,
    config: *mut KeyDict_win_config,
    err: *mut Error,
) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        let was_split = !(*w).w_floating;
        let mask = (*config).is_set__win_config_;
        let has_split = has_key(mask, KEYSET_OPTIDX_win_config__split);
        let has_vertical = has_key(mask, KEYSET_OPTIDX_win_config__vertical);
        let old_style = (*w).w_config.style;
        let mut fconfig = (*w).w_config;

        let to_split = (*config).relative.size == 0
            && !(has_key(mask, KEYSET_OPTIDX_win_config__external) && (*config).external)
            && (has_split || has_vertical || was_split);
        let will_float =
            (!was_split && !to_split) || (*config).relative.size > 0 || (*config).external;

        if !parse_win_config(
            w,
            config,
            &mut fconfig,
            !was_split || to_split,
            will_float,
            err,
        ) {
            return;
        }

        if to_split {
            if !win_config_split(w, config, &mut fconfig, err) {
                return;
            }
        } else if !win_config_float_tp(w, config, &fconfig, err) {
            return;
        }

        if fconfig.style == kWinStyleMinimal && old_style != fconfig.style {
            win_set_minimal_style(w);
            didset_window_options(w, true);
            changed_window_setting(w);
        }
        if fconfig._cmdline_offset < c_int::MAX {
            cmdline_win = w;
        } else if w == cmdline_win && fconfig._cmdline_offset == c_int::MAX {
            cmdline_win = ptr::null_mut();
        }
    }
}

unsafe fn config_put_bordertext(
    config: &mut KeyDict_win_config,
    fconfig: &WinConfig,
    bordertext_type: BorderTextType,
    arena: *mut Arena,
) {
    unsafe {
        let (vt, align) = match bordertext_type {
            kBorderTextTitle => (fconfig.title_chunks, fconfig.title_pos),
            _ => (fconfig.footer_chunks, fconfig.footer_pos),
        };

        let bordertext = virt_text_to_array(vt, true, arena);

        let pos = match align {
            kAlignLeft => c"left".as_ptr(),
            kAlignCenter => c"center".as_ptr(),
            _ => c"right".as_ptr(),
        };

        if bordertext_type == kBorderTextTitle {
            config.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__title;
            config.title = array_obj(bordertext);
            config.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__title_pos;
            config.title_pos = cstr_as_string(pos);
        } else {
            config.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__footer;
            config.footer = array_obj(bordertext);
            config.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__footer_pos;
            config.footer_pos = cstr_as_string(pos);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_get_config(
    win: Window,
    arena: *mut Arena,
    err: *mut Error,
) -> KeyDict_win_config {
    unsafe {
        const FLOAT_RELATIVE_STR: [&CStr; 6] = [
            c"editor",
            c"win",
            c"cursor",
            c"mouse",
            c"tabline",
            c"laststatus",
        ];
        const WIN_SPLIT_STR: [&CStr; 4] = [c"left", c"right", c"above", c"below"];
        const WIN_STYLE_STR: [&CStr; 2] = [c"", c"minimal"];

        let mut rv: KeyDict_win_config = std::mem::zeroed();

        let wp = find_window_by_handle(win, err);
        if wp.is_null() {
            return rv;
        }

        let config = &(*wp).w_config;

        rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__focusable;
        rv.focusable = config.focusable;
        rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__external;
        rv.external = config.external;
        rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__hide;
        rv.hide = config.hide;
        rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__mouse;
        rv.mouse = config.mouse;
        rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__style;
        rv.style = cstr_as_string(WIN_STYLE_STR[config.style as usize].as_ptr());

        if (*wp).w_floating {
            rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__width;
            rv.width = i64::from(config.width);
            rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__height;
            rv.height = i64::from(config.height);
            if !config.external {
                if config.relative == kFloatRelativeWindow {
                    rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__win;
                    rv.win = config.window;
                    if config.bufpos.lnum >= 0 {
                        let mut pos = arena_array(arena, 2);
                        add_c(&mut pos, integer_obj(i64::from(config.bufpos.lnum)));
                        add_c(&mut pos, integer_obj(i64::from(config.bufpos.col)));
                        rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__bufpos;
                        rv.bufpos = pos;
                    }
                }
                rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__anchor;
                rv.anchor = cstr_as_string(float_anchor_str[config.anchor as usize]);
                rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__row;
                rv.row = config.row;
                rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__col;
                rv.col = config.col;
                rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__zindex;
                rv.zindex = i64::from(config.zindex);
            }
            if config.border {
                let mut border = arena_array(arena, 8);
                for i in 0..8 {
                    let s = cstrn_as_string(config.border_chars[i].as_ptr(), MAX_SCHAR_SIZE);
                    let hi_id = config.border_hl_ids[i];
                    let hi_name = syn_id2name(hi_id);
                    if *hi_name != 0 {
                        let mut tuple = arena_array(arena, 2);
                        add_c(&mut tuple, string_obj(s));
                        add_c(&mut tuple, string_obj(cstr_as_string(hi_name)));
                        add_c(&mut border, array_obj(tuple));
                    } else {
                        add_c(&mut border, string_obj(s));
                    }
                }
                rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__border;
                rv.border = array_obj(border);
                if config.title {
                    config_put_bordertext(&mut rv, config, kBorderTextTitle, arena);
                }
                if config.footer {
                    config_put_bordertext(&mut rv, config, kBorderTextFooter, arena);
                }
            } else {
                rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__border;
                rv.border = string_obj(cstr_as_string(c"none".as_ptr()));
            }
        } else if !config.external {
            rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__width;
            rv.width = i64::from((*wp).w_width);
            rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__height;
            rv.height = i64::from((*wp).w_height);
            let split = win_split_dir(wp);
            rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__split;
            rv.split = cstr_as_string(WIN_SPLIT_STR[split as usize].as_ptr());
        }

        let rel = if (*wp).w_floating && !config.external {
            FLOAT_RELATIVE_STR[config.relative as usize].as_ptr()
        } else {
            c"".as_ptr()
        };
        rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config__relative;
        rv.relative = cstr_as_string(rel);
        if config._cmdline_offset < c_int::MAX {
            rv.is_set__win_config_ |= 1u64 << KEYSET_OPTIDX_win_config___cmdline_offset;
            rv._cmdline_offset = i64::from(config._cmdline_offset);
        }

        rv
    }
}

unsafe fn parse_float_anchor(anchor: NvimString, out: *mut FloatAnchor) -> bool {
    unsafe {
        if anchor.size == 0 {
            *out = 0;
        }
        let str_ptr = anchor.data;
        if striequal_str(str_ptr, "NW") {
            *out = 0;
        } else if striequal_str(str_ptr, "NE") {
            *out = kFloatAnchorEast;
        } else if striequal_str(str_ptr, "SW") {
            *out = kFloatAnchorSouth;
        } else if striequal_str(str_ptr, "SE") {
            *out = kFloatAnchorSouth | kFloatAnchorEast;
        } else {
            return false;
        }
        true
    }
}

unsafe fn parse_float_relative(relative: NvimString, out: *mut FloatRelative) -> bool {
    unsafe {
        let str_ptr = relative.data;
        if striequal_str(str_ptr, "editor") {
            *out = kFloatRelativeEditor;
        } else if striequal_str(str_ptr, "win") {
            *out = kFloatRelativeWindow;
        } else if striequal_str(str_ptr, "cursor") {
            *out = kFloatRelativeCursor;
        } else if striequal_str(str_ptr, "mouse") {
            *out = kFloatRelativeMouse;
        } else if striequal_str(str_ptr, "tabline") {
            *out = kFloatRelativeTabline;
        } else if striequal_str(str_ptr, "laststatus") {
            *out = kFloatRelativeLaststatus;
        } else {
            return false;
        }
        true
    }
}

unsafe fn parse_config_split(split: NvimString, out: *mut WinSplit) -> bool {
    unsafe {
        let str_ptr = split.data;
        if striequal_str(str_ptr, "left") {
            *out = kWinSplitLeft;
        } else if striequal_str(str_ptr, "right") {
            *out = kWinSplitRight;
        } else if striequal_str(str_ptr, "above") {
            *out = kWinSplitAbove;
        } else if striequal_str(str_ptr, "below") {
            *out = kWinSplitBelow;
        } else {
            return false;
        }
        true
    }
}

unsafe fn parse_float_bufpos(bufpos: Array, out: *mut lpos_T) -> bool {
    unsafe {
        if bufpos.size != 2
            || (*bufpos.items.add(0)).obj_type != ObjectType::kObjectTypeInteger
            || (*bufpos.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
        {
            return false;
        }
        (*out).lnum = (*bufpos.items.add(0)).data.integer as i32;
        (*out).col = (*bufpos.items.add(1)).data.integer as i32;
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn parse_bordertext(
    bordertext: Object,
    bordertext_type: BorderTextType,
    fconfig: *mut WinConfig,
    err: *mut Error,
) {
    unsafe {
        if bordertext.obj_type != ObjectType::kObjectTypeString
            && bordertext.obj_type != ObjectType::kObjectTypeArray
        {
            api_err_exp(
                err,
                c"title/footer".as_ptr(),
                c"String or Array".as_ptr(),
                api_typename(bordertext.obj_type),
            );
            return;
        }

        if bordertext.obj_type == ObjectType::kObjectTypeArray && bordertext.data.array.size == 0 {
            api_err_exp(
                err,
                c"title/footer".as_ptr(),
                c"non-empty Array".as_ptr(),
                ptr::null(),
            );
            return;
        }

        let is_title = bordertext_type == kBorderTextTitle;
        let is_present = if is_title {
            &mut (*fconfig).title
        } else {
            &mut (*fconfig).footer
        };
        let chunks = if is_title {
            &mut (*fconfig).title_chunks
        } else {
            &mut (*fconfig).footer_chunks
        };
        let width = if is_title {
            &mut (*fconfig).title_width
        } else {
            &mut (*fconfig).footer_width
        };

        if bordertext.obj_type == ObjectType::kObjectTypeString {
            if bordertext.data.string.size == 0 {
                *is_present = false;
                return;
            }
            let text = transstr(bordertext.data.string.data, true);
            chunks.init();
            chunks.push(VirtTextChunk { text, hl_id: -1 });
            *width = mb_string2cells(text) as c_int;
        } else {
            *chunks = parse_virt_text(bordertext.data.array, err, width, true);
            if error_set(err) {
                return;
            }
        }

        *is_present = true;
    }
}

unsafe fn parse_bordertext_pos(
    wp: *mut win_T,
    bordertext_pos: NvimString,
    bordertext_type: BorderTextType,
    fconfig: *mut WinConfig,
    err: *mut Error,
) -> bool {
    unsafe {
        let align = if bordertext_type == kBorderTextTitle {
            &mut (*fconfig).title_pos
        } else {
            &mut (*fconfig).footer_pos
        };

        if bordertext_pos.size == 0 {
            if wp.is_null() {
                *align = kAlignLeft;
            }
            return true;
        }

        let pos = bordertext_pos.data;
        if strequal_str(pos, "left") {
            *align = kAlignLeft;
        } else if strequal_str(pos, "center") {
            *align = kAlignCenter;
        } else if strequal_str(pos, "right") {
            *align = kAlignRight;
        } else {
            api_err_invalid(
                err,
                if bordertext_type == kBorderTextTitle {
                    c"title_pos".as_ptr()
                } else {
                    c"footer_pos".as_ptr()
                },
                pos,
                0,
                true,
            );
            return false;
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn parse_border_style(
    style: Object,
    fconfig: *mut WinConfig,
    err: *mut Error,
) {
    unsafe {
        const DEFAULTS_CHARS: [[&str; 8]; 6] = [
            ["╔", "═", "╗", "║", "╝", "═", "╚", "║"],
            ["┌", "─", "┐", "│", "┘", "─", "└", "│"],
            ["", "", " ", " ", " ", " ", " ", ""],
            ["╭", "─", "╮", "│", "╯", "─", "╰", "│"],
            [" ", " ", " ", " ", " ", " ", " ", " "],
            ["┏", "━", "┓", "┃", "┛", "━", "┗", "┃"],
        ];
        const DEFAULTS_SHADOW: [bool; 6] = [false, false, true, false, false, false];

        let chars = &mut (*fconfig).border_chars;
        let hl_ids = &mut (*fconfig).border_hl_ids;

        (*fconfig).border = true;

        if style.obj_type == ObjectType::kObjectTypeArray {
            let arr = style.data.array;
            let mut size = arr.size;
            if size == 0 || size > 8 || (size & (size - 1)) != 0 {
                api_err_exp(
                    err,
                    c"border".as_ptr(),
                    c"1, 2, 4, or 8 chars".as_ptr(),
                    ptr::null(),
                );
                return;
            }
            for i in 0..size {
                let iytem = *arr.items.add(i);
                let string: NvimString;
                let mut hl_id: c_int = 0;
                if iytem.obj_type == ObjectType::kObjectTypeArray {
                    let iarr = iytem.data.array;
                    if iarr.size == 0 || iarr.size > 2 {
                        api_err_exp(
                            err,
                            c"border".as_ptr(),
                            c"1 or 2-item Array".as_ptr(),
                            ptr::null(),
                        );
                        return;
                    }
                    let first = *iarr.items.add(0);
                    if first.obj_type != ObjectType::kObjectTypeString {
                        api_err_exp(
                            err,
                            c"border".as_ptr(),
                            c"Array of Strings".as_ptr(),
                            ptr::null(),
                        );
                        return;
                    }
                    string = first.data.string;
                    if iarr.size == 2 {
                        hl_id = object_to_hl_id(
                            *iarr.items.add(1),
                            c"border char highlight".as_ptr(),
                            err,
                        );
                        if error_set(err) {
                            return;
                        }
                    }
                } else if iytem.obj_type == ObjectType::kObjectTypeString {
                    string = iytem.data.string;
                } else {
                    api_err_exp(
                        err,
                        c"border".as_ptr(),
                        c"String or Array".as_ptr(),
                        api_typename(iytem.obj_type),
                    );
                    return;
                }
                if string.size != 0 && mb_string2cells_len(string.data, string.size) > 1 {
                    api_err_exp(
                        err,
                        c"border".as_ptr(),
                        c"only one-cell chars".as_ptr(),
                        ptr::null(),
                    );
                    return;
                }
                let len = string.size.min(MAX_SCHAR_SIZE - 1);
                if len > 0 {
                    ptr::copy_nonoverlapping(string.data, chars[i].as_mut_ptr(), len);
                }
                chars[i][len] = NUL;
                hl_ids[i] = hl_id;
            }
            while size < 8 {
                for j in 0..size {
                    chars[size + j] = chars[j];
                    hl_ids[size + j] = hl_ids[j];
                }
                size <<= 1;
            }
            if (chars[7][0] != 0 && chars[1][0] != 0 && chars[0][0] == 0)
                || (chars[1][0] != 0 && chars[3][0] != 0 && chars[2][0] == 0)
                || (chars[3][0] != 0 && chars[5][0] != 0 && chars[4][0] == 0)
                || (chars[5][0] != 0 && chars[7][0] != 0 && chars[6][0] == 0)
            {
                api_err_exp(
                    err,
                    c"border".as_ptr(),
                    c"corner char between edge chars".as_ptr(),
                    ptr::null(),
                );
            }
        } else if style.obj_type == ObjectType::kObjectTypeString {
            let str_val = style.data.string;
            if str_val.size == 0 || strequal_str(str_val.data, "none") {
                (*fconfig).border = false;
                (*fconfig).title = false;
                (*fconfig).footer = false;
                return;
            }
            for i in 0..6 {
                let def_name = opt_winborder_values[i + 1];
                if strequal(str_val.data, def_name) {
                    *chars = [[0; MAX_SCHAR_SIZE]; 8];
                    for (slot, piece) in chars.iter_mut().zip(DEFAULTS_CHARS[i].iter()) {
                        let bytes = piece.as_bytes();
                        ptr::copy_nonoverlapping(
                            bytes.as_ptr().cast::<c_char>(),
                            slot.as_mut_ptr(),
                            bytes.len(),
                        );
                    }
                    *hl_ids = [0; 8];
                    if DEFAULTS_SHADOW[i] {
                        let hl_blend = syn_check_group(c"FloatShadow".as_ptr(), 11);
                        let hl_through = syn_check_group(c"FloatShadowThrough".as_ptr(), 18);
                        hl_ids[2] = hl_through;
                        hl_ids[3] = hl_blend;
                        hl_ids[4] = hl_blend;
                        hl_ids[5] = hl_blend;
                        hl_ids[6] = hl_through;
                    }
                    return;
                }
            }
            api_err_invalid(err, c"border".as_ptr(), str_val.data, 0, true);
        }
    }
}

unsafe fn generate_api_error(wp: *mut win_T, attribute: *const c_char, err: *mut Error) {
    unsafe {
        if !wp.is_null() && (*wp).w_floating {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Required: 'relative' when reconfiguring floating window %d".as_ptr(),
                (*wp).handle,
            );
        } else {
            api_err_conflict(err, attribute, c"non-float window".as_ptr());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn parse_winborder(
    fconfig: *mut WinConfig,
    border_opt: *mut c_char,
    err: *mut Error,
) -> bool {
    unsafe {
        if fconfig.is_null() {
            return false;
        }
        let opt_bytes = CStr::from_ptr(border_opt).to_bytes();
        let style = if opt_bytes.contains(&b',') {
            let mut border_chars = Array::new();
            for part in opt_bytes.split(|&b| b == b',') {
                if border_chars.size >= 8 {
                    api_free_array(border_chars);
                    return false;
                }
                border_chars.push(string_obj(cbuf_to_string(
                    part.as_ptr().cast::<c_char>(),
                    part.len(),
                )));
            }
            if border_chars.size != 8 {
                api_free_array(border_chars);
                return false;
            }
            array_obj(border_chars)
        } else {
            cstr_to_obj(border_opt)
        };

        parse_border_style(style, fconfig, err);
        api_free_object(style);
        !error_set(err)
    }
}

unsafe fn parse_win_config(
    wp: *mut win_T,
    config: *mut KeyDict_win_config,
    fconfig: *mut WinConfig,
    reconf: bool,
    will_float: bool,
    err: *mut Error,
) -> bool {
    unsafe {
        let mut has_relative = false;
        let mut relative_is_win = false;
        let mut is_split = false;
        let mask = (*config).is_set__win_config_;

        'fail: {
            if (*config).relative.size > 0 {
                if !parse_float_relative((*config).relative, &mut (*fconfig).relative) {
                    api_err_invalid(err, c"relative".as_ptr(), (*config).relative.data, 0, true);
                    break 'fail;
                }

                if !(has_key(mask, KEYSET_OPTIDX_win_config__row)
                    && has_key(mask, KEYSET_OPTIDX_win_config__col))
                    && !has_key(mask, KEYSET_OPTIDX_win_config__bufpos)
                {
                    api_err_required(err, c"'relative' requires 'row'/'col' or 'bufpos'".as_ptr());
                    break 'fail;
                }

                has_relative = true;
                (*fconfig).external = false;
                if (*fconfig).relative == kFloatRelativeWindow {
                    relative_is_win = true;
                    (*fconfig).bufpos.lnum = -1;
                }
            } else if !(*config).external {
                if has_key(mask, KEYSET_OPTIDX_win_config__vertical)
                    || has_key(mask, KEYSET_OPTIDX_win_config__split)
                {
                    is_split = true;
                    (*fconfig).external = false;
                } else if wp.is_null() {
                    api_err_required(
                        err,
                        c"'relative' or 'external' when creating a float".as_ptr(),
                    );
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__vertical) && !is_split {
                api_err_conflict(err, c"vertical".as_ptr(), c"floating windows".as_ptr());
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__split) && !is_split {
                api_err_conflict(err, c"split".as_ptr(), c"floating windows".as_ptr());
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__split) {
                if !is_split {
                    api_err_conflict(err, c"split".as_ptr(), c"floating windows".as_ptr());
                    break 'fail;
                }
                if !parse_config_split((*config).split, &mut (*fconfig).split) {
                    api_err_invalid(err, c"split".as_ptr(), (*config).split.data, 0, true);
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__anchor)
                && !parse_float_anchor((*config).anchor, &mut (*fconfig).anchor)
            {
                api_err_invalid(err, c"anchor".as_ptr(), (*config).anchor.data, 0, true);
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__row) {
                if !has_relative || is_split {
                    generate_api_error(wp, c"row".as_ptr(), err);
                    break 'fail;
                }
                (*fconfig).row = (*config).row;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__col) {
                if !has_relative || is_split {
                    generate_api_error(wp, c"col".as_ptr(), err);
                    break 'fail;
                }
                (*fconfig).col = (*config).col;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__bufpos) {
                if !has_relative || is_split {
                    generate_api_error(wp, c"bufpos".as_ptr(), err);
                    break 'fail;
                } else {
                    if !parse_float_bufpos((*config).bufpos, &mut (*fconfig).bufpos) {
                        api_err_exp(
                            err,
                            c"bufpos".as_ptr(),
                            c"[row, col] array".as_ptr(),
                            ptr::null(),
                        );
                        break 'fail;
                    }

                    if !has_key(mask, KEYSET_OPTIDX_win_config__row) {
                        (*fconfig).row = if ((*fconfig).anchor & kFloatAnchorSouth) != 0 {
                            0.0
                        } else {
                            1.0
                        };
                    }
                    if !has_key(mask, KEYSET_OPTIDX_win_config__col) {
                        (*fconfig).col = 0.0;
                    }
                }
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__width) {
                if (*config).width <= 0 {
                    api_err_exp(
                        err,
                        c"width".as_ptr(),
                        c"positive Integer".as_ptr(),
                        ptr::null(),
                    );
                    break 'fail;
                }
                (*fconfig).width = (*config).width as c_int;
            } else if !reconf && !is_split {
                api_err_required(err, c"width".as_ptr());
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__height) {
                if (*config).height <= 0 {
                    api_err_exp(
                        err,
                        c"height".as_ptr(),
                        c"positive Integer".as_ptr(),
                        ptr::null(),
                    );
                    break 'fail;
                }
                (*fconfig).height = (*config).height as c_int;
            } else if !reconf && !is_split {
                api_err_required(err, c"height".as_ptr());
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__external) {
                (*fconfig).external = (*config).external;
                if has_relative && (*fconfig).external {
                    api_err_conflict(err, c"relative".as_ptr(), c"external".as_ptr());
                    break 'fail;
                }
                if (*fconfig).external && !ui_has(kUIMultigrid) {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"UI doesn't support external windows".as_ptr(),
                    );
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__win) && (*fconfig).external {
                api_err_conflict(err, c"win".as_ptr(), c"external window".as_ptr());
                break 'fail;
            }

            if relative_is_win
                || (has_key(mask, KEYSET_OPTIDX_win_config__win)
                    && !is_split
                    && !wp.is_null()
                    && (*wp).w_floating
                    && (*fconfig).relative == kFloatRelativeWindow)
            {
                let target_win = find_window_by_handle((*config).win, err);
                if target_win.is_null() {
                    break 'fail;
                }
                if target_win == wp {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"floating window cannot be relative to itself".as_ptr(),
                    );
                    break 'fail;
                }
                (*fconfig).window = (*target_win).handle;
            } else {
                if has_key(mask, KEYSET_OPTIDX_win_config__win) {
                    if !is_split && !has_relative && (wp.is_null() || !(*wp).w_floating) {
                        api_err_required(
                            err,
                            c"non-float with 'win' requires 'split' or 'vertical'".as_ptr(),
                        );
                        break 'fail;
                    }
                    (*fconfig).window = (*config).win;
                }
                if (*fconfig).window == 0 {
                    (*fconfig).window = (*curwin).handle;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__focusable) {
                (*fconfig).focusable = (*config).focusable;
                (*fconfig).mouse = (*config).focusable;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__mouse) {
                (*fconfig).mouse = (*config).mouse;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__zindex) {
                if !will_float {
                    api_err_conflict(err, c"zindex".as_ptr(), c"non-float window".as_ptr());
                    break 'fail;
                }
                if (*config).zindex <= 0 {
                    api_err_exp(
                        err,
                        c"zindex".as_ptr(),
                        c"positive Integer".as_ptr(),
                        ptr::null(),
                    );
                    break 'fail;
                }
                (*fconfig).zindex = (*config).zindex as c_int;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__title) {
                if !will_float {
                    api_err_conflict(err, c"title".as_ptr(), c"non-float window".as_ptr());
                    break 'fail;
                }

                parse_bordertext((*config).title, kBorderTextTitle, fconfig, err);
                if error_set(err) {
                    break 'fail;
                }

                if !parse_bordertext_pos(wp, (*config).title_pos, kBorderTextTitle, fconfig, err) {
                    break 'fail;
                }
            } else if has_key(mask, KEYSET_OPTIDX_win_config__title_pos) {
                api_err_required(err, c"'title' requires 'title_pos'".as_ptr());
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__footer) {
                if !will_float {
                    api_err_conflict(err, c"footer".as_ptr(), c"non-float window".as_ptr());
                    break 'fail;
                }

                parse_bordertext((*config).footer, kBorderTextFooter, fconfig, err);
                if error_set(err) {
                    break 'fail;
                }

                if !parse_bordertext_pos(wp, (*config).footer_pos, kBorderTextFooter, fconfig, err)
                {
                    break 'fail;
                }
            } else if has_key(mask, KEYSET_OPTIDX_win_config__footer_pos) {
                api_err_required(err, c"'footer' requires 'footer_pos'".as_ptr());
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__border) {
                if !will_float {
                    api_err_conflict(err, c"border".as_ptr(), c"non-float window".as_ptr());
                    break 'fail;
                }
                let border_style = (*config).border;
                if border_style.obj_type != ObjectType::kObjectTypeNil {
                    parse_border_style(border_style, fconfig, err);
                    if error_set(err) {
                        break 'fail;
                    }
                }
            } else if *p_winborder != NUL
                && (wp.is_null() || !(*wp).w_floating)
                && !parse_winborder(fconfig, p_winborder, err)
            {
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__style) {
                if *(*config).style.data == NUL {
                    (*fconfig).style = kWinStyleUnused;
                } else if striequal_str((*config).style.data, "minimal") {
                    (*fconfig).style = kWinStyleMinimal;
                } else {
                    api_err_invalid(err, c"style".as_ptr(), (*config).style.data, 0, true);
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__noautocmd) {
                if !wp.is_null() && (*config).noautocmd != (*fconfig).noautocmd {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"'noautocmd' cannot be changed on existing window".as_ptr(),
                    );
                    break 'fail;
                }
                (*fconfig).noautocmd = (*config).noautocmd;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__fixed) {
                (*fconfig).fixed = (*config).fixed;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config__hide) {
                if !will_float && (*config).hide {
                    api_err_conflict(err, c"hide".as_ptr(), c"non-float window".as_ptr());
                    break 'fail;
                }
                if (*fconfig).hide && !(*config).hide {
                    redraw_later(wp, UPD_NOT_VALID);
                }
                (*fconfig).hide = (*config).hide;
            }

            if has_key(mask, KEYSET_OPTIDX_win_config___cmdline_offset) {
                (*fconfig)._cmdline_offset = (*config)._cmdline_offset as c_int;
            }

            return true;
        }

        merge_win_config(
            fconfig,
            if !wp.is_null() {
                (*wp).w_config
            } else {
                WIN_CONFIG_INIT
            },
        );
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_split_flags_computes_expected_bits() {
        assert_eq!(win_split_flags(kWinSplitAbove, false), WSP_HOR | WSP_ABOVE);
        assert_eq!(win_split_flags(kWinSplitBelow, true), WSP_HOR | WSP_BOT);
        assert_eq!(win_split_flags(kWinSplitLeft, true), WSP_VERT | WSP_TOP);
        assert_eq!(win_split_flags(kWinSplitRight, false), WSP_VERT | WSP_BELOW);
    }
}
