//! Functions for displaying messages on the command line (`src/nvim/message.c`).

use crate::api::private::helpers::{
    HLF_E, HlMessageChunk, api_free_array, copy_string, cstr_as_string, cstr_to_string,
};
use crate::api::private::validate::{
    Array, Dict, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::api::vim::{HLF_N, HlMessage, MessageData};
use crate::autocmd::{AUGROUP_ALL, EVENT_PROGRESS, HLF_8, HLF_T, apply_autocmds_group, has_event};
use crate::charset::{
    ascii_iswhite, ascii2cells, char2cells, is_special, skipwhite, to_special, transchar_buf,
    transchar_byte_buf, vim_isprintc, vim_strsize,
};
use crate::dialog::{confirm_buttons, confirm_msg, prompt_for_input};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, redraw_all_later, redraw_later, set_must_redraw,
};
use crate::errors::*;
use crate::eval::callback_call;
use crate::eval::typval::{TypvalVval, VarType, tv_clear, typval_T};
use crate::eval::vars::{
    VV_ERRMSG, VV_SCROLLSTART, VV_STATUSMSG, VV_WARNINGMSG, get_vim_var_str, set_vim_var_string,
    var_redir_str,
};
use crate::event::r#loop::loop_schedule_deferred;
use crate::event::multiqueue::{Event, multiqueue_process_events};
use crate::ex_docmd::{do_sleep, exarg_T};
use crate::ex_eval::cause_errthrow;
use crate::extmark::{TriState, kFalse, kTrue};
use crate::fileio::{FILE, check_timestamps, fclose, fprintf, fputc, fputs, printf};
use crate::globals::*;
use crate::grid::{
    grid_adjust, grid_alloc, grid_assign_handle, grid_clear, grid_clear_line, grid_del_lines,
    grid_free, grid_ins_lines, grid_line_cursor_goto, grid_line_flush,
    grid_line_flush_if_valid_row, grid_line_mirror, grid_line_puts, grid_line_start, sattr_T,
    schar_T, schar_from_ascii, schar_get,
};
use crate::highlight::{hl_attr, hl_combine_attr};
use crate::highlight_group::{highlight_changed, syn_check_group, syn_id2attr};
use crate::indent::tabstop_padding;
use crate::input::{
    FLUSH_MINIMAL, beep_flush, char_avail, flush_buffers, get_keystroke, requeue_key, safe_vgetc,
    stuff_empty, typeahead_noflush,
};
use crate::keycodes::{
    BS, CAR, ESC, K_BS, K_DOWN, K_IGNORE, K_LEFTDRAG, K_LEFTMOUSE, K_LEFTRELEASE, K_MIDDLEDRAG,
    K_MIDDLEMOUSE, K_MIDDLERELEASE, K_MOUSEDOWN, K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP,
    K_PAGEDOWN, K_PAGEUP, K_RIGHTDRAG, K_RIGHTMOUSE, K_RIGHTRELEASE, K_SPECIAL, K_UP, K_X1MOUSE,
    K_X2MOUSE, KS_MODIFIER, MOD_MASK_SHIFT, NL, TAB, get_special_key, keychord,
};
use crate::log::{LOGLVL_DBG, LOGLVL_INF, log_get_stderr, logmsg};
use crate::main_init::input_unanswerable;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_string2cells_len, mb_unescape, ptr2cells, utf_char2bytes, utf_char2cells, utf_head_off,
    utf_ptr2cells, utf_ptr2char, utf_ptr2len, utf8len_tab, utfc_ptr2len, utfc_ptr2len_len,
};
use crate::memory::{
    Arena, arena_alloc, cstr_take, strequal, xfree, xmemdupz, xmemrchr, xstrlcat, xstrlcpy,
    xstrnlen,
};
use crate::mouse::{MOUSE_SETPOS, jump_to_mouse, setmouse};
use crate::msgpack_rpc::packer::Integer;
use crate::option::shortmess;
use crate::option_vars::*;
use crate::optionstr::{OptKeyDict_mopt, opt_keyset, optset_T};
use crate::os::env::home_replace_save;
use crate::os::fs::os_fopen;
use crate::os::input::os_breakcheck;
use crate::os::time::os_delay;
use crate::regexp::vim_regexec;
use crate::register::write_reg_contents;
use crate::runtime::{ESTACK_NONE, estack_sfile, exestack, sourcing_lnum, sourcing_name};
use crate::state::{MODE_ASKMORE, MODE_CMDLINE, MODE_EXTERNCMD, MODE_HITRETURN, MODE_SETWSIZE};
use crate::strings::{gettext, ngettext, vim_snprintf, vim_snprintf_safelen, vim_strchr};
use crate::ui::{
    ui_active, ui_call_grid_destroy, ui_call_grid_resize, ui_call_grid_scroll,
    ui_call_msg_history_show, ui_call_msg_set_pos, ui_call_msg_show, ui_call_msg_showmode,
    ui_cursor_goto, ui_flush, ui_grid_cursor_goto, ui_has, ui_line, ui_refresh, vim_beep,
};
use crate::ui_compositor::{ui_comp_put_grid, ui_comp_remove_grid};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong};
use std::ptr;

pub type MsgID = Object;

const BELL: c_int = 0x07;

pub(crate) const kZIndexMessages: c_int = 200;

pub(crate) const HLF_M: c_int = 10;
pub(crate) const HLF_MSG: c_int = 63;

const kOptBoFlagMess: c_uint = 0x1000;
const kOptBoFlagShell: c_uint = 0x10000;
const kOptRdbFlagNothrottle: c_uint = 0x02;
const kShmTrunc: c_int = b't' as c_int;
const kShmTruncall: c_int = b'T' as c_int;

pub(crate) const kOptMessagesopt: c_int = 190;
const KEYSET_OPTIDX_mopt__wait: u32 = 1;
const KEYSET_OPTIDX_mopt__history: u32 = 3;

const PROGRESS_TARGET_CMD: c_int = 0x01;

const VERBOSE_KIND: &CStr = c"verbose";

pub(crate) const NIL_OBJ: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

/// Message history for `:messages`
pub struct MessageHistoryEntry {
    pub msg: HlMessage,
    pub kind: Option<std::ffi::CString>,
    pub temp: bool,
    pub append: bool,
}

// To be able to scroll back at the "more" and "hit-enter" prompts we need to
// store the displayed text and remember where screen lines start.
struct msgchunk_T {
    sb_eol: c_char,
    sb_msg_col: c_int,
    sb_hl_id: c_int,
    sb_text: Vec<c_char>,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum sb_clear_T {
    SB_CLEAR_NONE = 0,
    SB_CLEAR_ALL = 1,
    SB_CLEAR_CMDLINE_BUSY = 2,
    SB_CLEAR_CMDLINE_DONE = 3,
}

static mut confirm_msg_used: c_int = 0;

static mut msg_hist: std::collections::VecDeque<MessageHistoryEntry> =
    std::collections::VecDeque::new();
static mut msg_hist_temp_idx: Option<usize> = None;
static mut msg_hist_len: c_int = 0;
static mut msg_hist_max: c_int = 500;

// 'messagesopt' items. Default: "hit-enter,history:500,progress:c".
static mut msg_hit_enter: bool = true;
static mut msg_wait: c_int = 0;
static mut progress_msg_target: c_int = PROGRESS_TARGET_CMD;

static mut verbose_fd: *mut FILE = ptr::null_mut();
static mut verbose_did_open: bool = false;

static mut keep_msg_more: bool = false;

// Extended msg state, currently used for external UIs with ext_messages
static mut msg_ext_kind: *const c_char = ptr::null();
static mut msg_ext_trigger: *const c_char = ptr::null();
static mut msg_ext_id: MsgID = Object {
    obj_type: ObjectType::kObjectTypeInteger,
    data: ObjectData { integer: 1 },
};
static mut msg_ext_chunks: Array = Array::new();
static mut msg_ext_last_chunk: Vec<c_char> = Vec::new();
static mut msg_ext_last_attr: sattr_T = -1;
static mut msg_ext_last_hl_id: c_int = 0;

static mut msg_ext_history: bool = false;
static mut msg_ext_append: bool = false;

static mut msg_grid_pos_at_flush: c_int = 0;

/// Set to true to avoid msg_use_printf, including for a delayed newline
static mut msg_special_case_for_lua_print: bool = false;

static mut msg_id_next: i64 = 1;

// Ensure a single msg_show event, msg_list and history entry for entire multihl message.
static mut is_multihl: c_int = 0;

static mut MSG_KEEP_ENTERED: c_int = 0;

// Remember the last sourcing name/lnum used in an error message, so that it
// isn't printed each time when it didn't change.
static mut last_sourcing_lnum: c_int = 0;
static mut last_sourcing_name: Option<std::ffi::CString> = None;

static mut MSG_SOURCE_RECURSIVE: bool = false;

static mut do_clear_hist_temp: bool = true;

static mut redir_col: c_int = 0;

static mut STR2SPECIAL_BUF: [c_char; 7] = [0; 7];

static mut msgchunks: Vec<msgchunk_T> = Vec::new();
static mut do_clear_sb_text: sb_clear_T = sb_clear_T::SB_CLEAR_NONE;

static mut DO_MORE_PROMPT_ENTERED: bool = false;
static mut SHOWMODE_CLEAR: bool = false;

static mut pre_verbose_kind: *const c_char = ptr::null();

#[macro_export]
macro_rules! smsg {
    ($hl_id:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let hl = $hl_id;
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg(
                (&raw const $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                hl,
            )
        }
    }};
}
pub use smsg;

#[macro_export]
macro_rules! smsg_keep {
    ($hl_id:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let hl = $hl_id;
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg_keep(
                (&raw const $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                hl,
                true,
                false,
            )
        }
    }};
}
pub use smsg_keep;

#[macro_export]
macro_rules! semsg {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        {
            if $crate::message::emsg_not_now() {
                let _ = (fmt_ptr $(, $crate::strings::IntoFmtVal::into_fmt_val($arg))*);
                true
            } else {
                let mut errbuf = [0 as std::ffi::c_char; 1025];
                $crate::strings::vim_snprintf!(
                    (&raw mut errbuf).cast::<std::ffi::c_char>(),
                    errbuf.len(),
                    fmt_ptr
                    $(, $arg)*
                );
                $crate::message::emsg(errbuf.as_ptr())
            }
        }
    }};
}
pub use semsg;

#[macro_export]
macro_rules! semsg_multiline {
    ($kind:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let kind_ptr: *const std::ffi::c_char = $kind;
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        {
            if $crate::message::emsg_not_now() {
                let _ = (kind_ptr, fmt_ptr $(, $crate::strings::IntoFmtVal::into_fmt_val($arg))*);
                true
            } else {
                let mut errbuf = [0 as std::ffi::c_char; 8192];
                $crate::strings::vim_snprintf!(
                    (&raw mut errbuf).cast::<std::ffi::c_char>(),
                    errbuf.len(),
                    fmt_ptr
                    $(, $arg)*
                );
                $crate::message::emsg_multiline(errbuf.as_ptr(), kind_ptr, 6, true)
            }
        }
    }};
}
pub use semsg_multiline;

#[macro_export]
macro_rules! siemsg {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        let _ = $crate::message::semsg!($fmt $(, $arg)*);
    }};
}
pub use siemsg;

#[macro_export]
macro_rules! msg_schedule_semsg {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg_schedule_semsg_cstr(
                (&raw const $crate::globals::IObuff).cast::<std::ffi::c_char>(),
            );
        }
    }};
}
pub use msg_schedule_semsg;

#[macro_export]
macro_rules! msg_schedule_semsg_multiline {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg_schedule_semsg_multiline_cstr(
                (&raw const $crate::globals::IObuff).cast::<std::ffi::c_char>(),
            );
        }
    }};
}
pub use msg_schedule_semsg_multiline;

#[macro_export]
macro_rules! swmsg {
    ($hl:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let hl_val: bool = $hl;
        let fmt_ptr: *const std::ffi::c_char = $fmt;
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::give_warning(
                (&raw const $crate::globals::IObuff).cast::<std::ffi::c_char>(),
                hl_val,
                true,
            );
        }
    }};
}
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::win_config::kUIMultigrid;
use crate::buffer::kUIMessages;
use crate::change::{HLF_W, MB_MAXCHAR};
use crate::charset::NUL;
use crate::cmdexpand::MB_MAXBYTES;
use crate::drawline::{HLF_0, HLF_AT};
use crate::eval::typval::{VAR_UNLOCKED, kCallbackNone};
use crate::eval::userfunc::MSG_BUF_LEN;
use crate::ex_cmds::{Ctrl_C, HLF_R};
use crate::ex_getln::{Ctrl_B, Ctrl_F, K_EVENT};
pub use swmsg;

use crate::keycodes::{k_second, k_third};

#[inline]
unsafe fn have_sourcing_info() -> bool {
    unsafe {
        let es = &raw const exestack;
        !(*es).is_empty()
    }
}

/// Returns true if the given integer message-id was previously generated.
pub unsafe fn msg_id_exists(id: i64) -> bool {
    unsafe { id > 0 && id < msg_id_next }
}

unsafe fn ui_ext_msg_set_pos(row: c_int, scrolled: bool) {
    unsafe {
        let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
        let size = schar_get(buf.as_mut_ptr(), (*curwin).w_p_fcs_chars.msgsep);
        ui_call_msg_set_pos(
            msg_grid.handle as Integer,
            row as Integer,
            scrolled,
            NvimString {
                data: buf.as_mut_ptr(),
                size,
            },
            msg_grid.zindex as Integer,
            msg_grid.comp_index as Integer,
        );
        msg_grid.pending_comp_index_update = false;
    }
}

pub unsafe fn msg_grid_set_pos(row: c_int, scrolled: bool) {
    unsafe {
        if !msg_grid.throttled {
            ui_ext_msg_set_pos(row, scrolled);
            msg_grid_pos_at_flush = row;
        }
        msg_grid_pos = row;
        if crate::grid::grid_allocated(&raw const msg_grid) {
            msg_grid_adj.row_offset = -row;
        }
    }
}

pub unsafe fn msg_use_grid() -> bool {
    unsafe { crate::grid::grid_allocated(&raw const default_grid) && !ui_has(kUIMessages) }
}

pub unsafe fn msg_grid_validate() {
    unsafe {
        grid_assign_handle(&raw mut msg_grid);
        let should_alloc = msg_use_grid();
        let max_rows = Rows - p_ch as c_int;
        let msg_grid_empty = !crate::grid::grid_allocated(&raw const msg_grid);
        if should_alloc && (msg_grid.rows != Rows || msg_grid.cols != Columns || msg_grid_empty) {
            grid_alloc(&raw mut msg_grid, Rows, Columns, false, true);
            msg_grid.zindex = kZIndexMessages;

            msg_grid.dirty_col = vec![0; Rows as usize];

            // Tricky: allow resize while pager or ex mode is active
            let pos = if (State & MODE_ASKMORE) != 0 {
                0
            } else {
                (max_rows - msg_scrolled).max(0)
            };
            msg_grid.throttled = false; // don't throttle in 'cmdheight' area
            msg_grid_set_pos(pos, msg_scrolled != 0);
            ui_comp_put_grid(
                &raw mut msg_grid,
                pos,
                0,
                msg_grid.rows,
                msg_grid.cols,
                false,
                true,
            );
            ui_call_grid_resize(
                msg_grid.handle as Integer,
                msg_grid.cols as Integer,
                msg_grid.rows as Integer,
            );

            msg_scrolled_at_flush = msg_scrolled;
            msg_grid.mouse_enabled = false;
            msg_grid_adj.target = &raw mut msg_grid;
        } else if !should_alloc && !msg_grid_empty {
            ui_comp_remove_grid(&raw mut msg_grid);
            grid_free(&raw mut msg_grid);
            msg_grid.dirty_col = Vec::new();
            ui_call_grid_destroy(msg_grid.handle as Integer);
            msg_grid.throttled = false;
            msg_grid_adj.row_offset = 0;
            msg_grid_adj.target = &raw mut default_grid;
            redraw_cmdline = true;
        } else if !msg_grid_empty && msg_scrolled == 0 && msg_grid_pos != max_rows {
            let diff = msg_grid_pos - max_rows;
            msg_grid_set_pos(max_rows, false);
            if diff > 0 {
                grid_clear(
                    &raw mut msg_grid_adj,
                    Rows - diff,
                    Rows,
                    0,
                    Columns,
                    hl_attr(HLF_MSG),
                );
            }
        }

        if crate::grid::grid_allocated(&raw const msg_grid)
            && msg_scrolled == 0
            && cmdline_row < msg_grid_pos
        {
            cmdline_row = msg_grid_pos;
        }
    }
}

/// Like msg() but keep it silent when 'verbosefile' is set.
pub unsafe fn verb_msg(s: *const c_char) -> c_int {
    unsafe {
        verbose_enter();
        let n = c_int::from(msg_keep(s, 0, false, false));
        verbose_leave();
        n
    }
}

/// Displays the string 's' on the status line
/// When terminal not initialized (yet) printf("%s", ..) is used.
///
/// @return  true if wait_return() not called
pub unsafe fn msg(s: *const c_char, hl_id: c_int) -> bool {
    unsafe { msg_keep(s, hl_id, false, false) }
}

/// Similar to msg_outtrans_len, but support newlines and tabs.
pub unsafe fn msg_multiline(
    str_: NvimString,
    hl_id: c_int,
    check_int: bool,
    hist: bool,
    need_clear: *mut bool,
) {
    unsafe {
        let mut s = str_.data.cast_const();
        let mut chunk = s;
        while (s.offset_from(str_.data) as usize) < str_.size {
            if check_int && got_int {
                return;
            }
            if *s == NL as c_char
                || *s == TAB as c_char
                || *s == CAR as c_char
                || *s == BELL as c_char
            {
                // Print all chars before the delimiter
                msg_outtrans_len(chunk, s.offset_from(chunk) as c_int, hl_id, hist);

                if *s != TAB as c_char && *need_clear {
                    msg_clr_eos();
                    *need_clear = false;
                }
                if *s == BELL as c_char {
                    vim_beep(kOptBoFlagShell);
                } else {
                    msg_putchar_hl((*s as u8) as c_int, hl_id);
                }
                chunk = s.add(1);
            }
            s = s.add(1);
        }

        // Print the remainder or emit empty event if entire message is empty.
        if *chunk != NUL || chunk == str_.data.cast_const() {
            msg_outtrans_len(
                chunk,
                (str_.size - (chunk.offset_from(str_.data) as usize)) as c_int,
                hl_id,
                hist,
            );
        }
    }
}

/// Format a progress message, adding title and percent if given.
///
/// @param hl_msg Message chunks
/// @param msg_data Additional data for progress messages
unsafe fn format_progress_message(
    hl_msg: &HlMessage,
    msg_data: *mut MessageData,
    updated_msg: &mut HlMessage,
) -> bool {
    unsafe {
        // progress messages are special. displayed as "title: percent% msg"
        if (*msg_data).title.size != 0 {
            // this block draws the "title:" before the progress-message
            let mut hl_id = 0;
            if (*msg_data).status.data.is_null() {
                hl_id = 0;
            } else if strequal((*msg_data).status.data, c"success".as_ptr()) {
                hl_id = syn_check_group(c"OkMsg".as_ptr(), 5);
            } else if strequal((*msg_data).status.data, c"failed".as_ptr()) {
                hl_id = syn_check_group(c"ErrorMsg".as_ptr(), 8);
            } else if strequal((*msg_data).status.data, c"running".as_ptr()) {
                hl_id = syn_check_group(c"MoreMsg".as_ptr(), 7);
            } else if strequal((*msg_data).status.data, c"cancel".as_ptr()) {
                hl_id = syn_check_group(c"WarningMsg".as_ptr(), 10);
            }
            updated_msg.push(HlMessageChunk {
                text: copy_string((*msg_data).title, ptr::null_mut()),
                hl_id,
            });
            updated_msg.push(HlMessageChunk {
                text: cstr_to_string(c": ".as_ptr()),
                hl_id: 0,
            });
        }
        if (*msg_data).percent >= 0 {
            let mut percent_buf = [0 as c_char; 10];
            vim_snprintf!(
                percent_buf.as_mut_ptr(),
                percent_buf.len(),
                c"%3ld%% ".as_ptr(),
                (*msg_data).percent as c_long,
            );
            let percent = cstr_to_string(percent_buf.as_ptr());
            let hl_id = syn_check_group(c"WarningMsg".as_ptr(), 10);
            updated_msg.push(HlMessageChunk {
                text: percent,
                hl_id,
            });
        }

        if !updated_msg.is_empty() {
            for chunk in hl_msg {
                updated_msg.push(HlMessageChunk {
                    text: copy_string(chunk.text, ptr::null_mut()),
                    hl_id: chunk.hl_id,
                });
            }
            return true;
        }
        false
    }
}

/// Print message chunks, each with their own highlight ID.
pub unsafe fn msg_multihl(
    mut id: MsgID,
    hl_msg: &mut HlMessage,
    kind: *const c_char,
    history: bool,
    err: bool,
    msg_data: *mut MessageData,
    needs_msg_clear: *mut bool,
    raw_for_lua_print: bool,
) -> MsgID {
    unsafe {
        if id.obj_type == ObjectType::kObjectTypeNil {
            let next_id = msg_id_next;
            msg_id_next += 1;
            id = Object {
                obj_type: ObjectType::kObjectTypeInteger,
                data: ObjectData { integer: next_id },
            };
        } else if id.obj_type == ObjectType::kObjectTypeInteger && !msg_id_exists(id.data.integer) {
            std::process::abort();
        }

        let mut updated_msg = HlMessage::new();
        let mut hl_msg_updated = false;
        // don't display progress message in cmd when target doesn't have cmd
        if strequal(kind, c"progress".as_ptr()) {
            do_autocmd_progress(id, hl_msg, msg_data);
            if (progress_msg_target & PROGRESS_TARGET_CMD) == 0 {
                *needs_msg_clear = true;
                return id;
            }
            if !msg_data.is_null() && format_progress_message(hl_msg, msg_data, &mut updated_msg) {
                *needs_msg_clear = true;
                hl_msg_updated = true;
            }
        }

        let active_msg: &HlMessage = if hl_msg_updated { &updated_msg } else { hl_msg };

        no_wait_return += 1;
        msg_start();
        msg_clr_eos();
        let mut need_clear = false;
        if !kind.is_null() {
            msg_ext_set_kind(kind);
        }
        if raw_for_lua_print {
            msg_special_case_for_lua_print = true;
            assert!(active_msg.len() == 1);
            let chunk = &active_msg[0];
            printf!(
                c"%.*s\n".as_ptr(),
                chunk.text.size as c_int,
                chunk.text.data,
            );
        }

        msg_ext_skip_flush = true;
        msg_ext_id = id;

        for chunk in active_msg {
            is_multihl += 1;
            if err {
                emsg_multiline(chunk.text.data, kind, chunk.hl_id, true);
            } else {
                msg_multiline(chunk.text, chunk.hl_id, true, false, &raw mut need_clear);
            }
            assert!(!ui_has(kUIMessages) || kind.is_null() || msg_ext_kind == kind);
        }

        let has_chunks = !active_msg.is_empty();
        if history && has_chunks {
            if hl_msg_updated {
                msg_hist_add_multihl(std::mem::take(&mut updated_msg), false);
            } else {
                msg_hist_add_multihl(std::mem::take(hl_msg), false);
            }
        }

        msg_ext_skip_flush = false;
        is_multihl = 0;
        no_wait_return -= 1;
        msg_end();

        // Release the id: it belongs to this message, and a String id only borrows the caller's storage
        // (often a stack buffer). Redundant unless nothing was emitted (e.g. 'msg_silent').
        msg_ext_id = Object {
            obj_type: ObjectType::kObjectTypeInteger,
            data: ObjectData {
                integer: msg_id_next,
            },
        };
        id
    }
}

/// @param keep set keep_msg if it doesn't scroll
pub unsafe fn msg_keep(mut s: *const c_char, hl_id: c_int, keep: bool, multiline: bool) -> bool {
    unsafe {
        if keep && multiline {
            std::process::abort();
        }

        // Skip messages not match ":filter pattern".
        // Don't filter when there is an error.
        if !emsg_on_display && message_filtered(s) {
            return true;
        }

        if hl_id == 0 {
            set_vim_var_string(VV_STATUSMSG, s, -1);
        }

        // It is possible that displaying a messages causes a problem (e.g.,
        // when redrawing the window), which causes another message, etc..    To
        // break this loop, limit the recursiveness to 3 levels.
        if MSG_KEEP_ENTERED >= 3 {
            return true;
        }
        MSG_KEEP_ENTERED += 1;

        // Add message to history unless it's a multihl, repeated kept or truncated message.
        let hist = &raw const msg_hist;
        let keep_msg_ptr = (&raw const keep_msg)
            .as_ref()
            .unwrap()
            .as_ref()
            .map_or(ptr::null(), |k| k.as_ptr());
        if is_multihl == 0
            && (s != keep_msg_ptr
                || (*s != b'<' as c_char
                    && (*hist).back().is_some_and(|last| {
                        CStr::from_ptr(s) != CStr::from_ptr(last.msg.as_slice()[0].text.data)
                    })))
        {
            msg_hist_add(s, -1, hl_id);
        }

        if is_multihl == 0 {
            msg_start();
        }
        // Truncate the message if needed.
        let buf = msg_strtrunc(s, 0);
        if let Some(ref buf) = buf {
            s = buf.as_ptr();
        }

        let mut need_clear = true;
        if multiline {
            msg_multiline(cstr_as_string(s), hl_id, false, false, &raw mut need_clear);
        } else {
            msg_outtrans(s, hl_id, false);
        }
        if need_clear {
            msg_clr_eos();
        }
        let mut retval = true;
        if is_multihl == 0 {
            retval = msg_end();
        }

        if keep && retval && vim_strsize(s) < (Rows - cmdline_row - 1) * Columns + sc_col {
            set_keep_msg(s, 0);
        }

        need_fileinfo = false;

        drop(buf);
        MSG_KEEP_ENTERED -= 1;
        retval
    }
}

/// Truncate a string such that it can be printed without causing a scroll.
///
/// @param force  always truncate
pub unsafe fn msg_strtrunc(s: *const c_char, force: c_int) -> Option<std::ffi::CString> {
    unsafe {
        // May truncate message to avoid a hit-return prompt
        if (msg_scroll == 0
            && !need_wait_return
            && shortmess(kShmTruncall)
            && msg_silent == 0
            && !ui_has(kUIMessages))
            || force != 0
        {
            let len = vim_strsize(s);
            let room = if msg_scrolled != 0 {
                // Use all the columns.
                (Rows - msg_row) * Columns - 1
            } else {
                // Use up to 'showcmd' column.
                let last_row = if ui_has(kUIMessages) {
                    Columns
                } else {
                    sc_col - 1
                };
                (Rows - msg_row - 1) * Columns + last_row
            };
            if len > room && room > 0 {
                // may have up to 18 bytes per cell (6 per char, up to two
                // composing chars)
                let buflen = (room + 2) * 18;
                let mut buf: Vec<c_char> = vec![0; buflen as usize];
                trunc_string(s, buf.as_mut_ptr(), room, buflen);
                return Some(CStr::from_ptr(buf.as_ptr()).to_owned());
            }
        }
        None
    }
}

/// Truncate a string "s" to "buf" with cell width "room".
/// "s" and "buf" may be equal.
pub unsafe fn trunc_string(s: *const c_char, buf: *mut c_char, room_in: c_int, buflen: c_int) {
    unsafe {
        let mut room = room_in - 3; // "..." takes 3 chars
        let mut len: c_int = 0;
        let mut e: c_int = 0;

        if *s == NUL {
            if buflen > 0 {
                *buf = NUL;
            }
            return;
        }

        if room_in < 3 {
            room = 0;
        }
        let mut half = room / 2;

        // First part: Start of the string.
        while len < half && e < buflen {
            if *s.add(e as usize) == NUL {
                // text fits without truncating!
                *buf.add(e as usize) = NUL;
                return;
            }
            let mut n = ptr2cells(s.add(e as usize));
            if len + n > half {
                break;
            }
            len += n;
            *buf.add(e as usize) = *s.add(e as usize);
            n = utfc_ptr2len(s.add(e as usize));
            while {
                n -= 1;
                n > 0
            } {
                e += 1;
                if e == buflen {
                    break;
                }
                *buf.add(e as usize) = *s.add(e as usize);
            }
            e += 1;
        }

        // Last part: End of the string.
        let mut i = CStr::from_ptr(s).to_bytes().len() as c_int;
        half = i;
        loop {
            half = half - utf_head_off(s, s.add((half - 1) as usize)) - 1;
            let n = ptr2cells(s.add(half as usize));
            if len + n > room || half == 0 {
                break;
            }
            len += n;
            i = half;
        }

        if i <= e + 3 {
            // text fits without truncating
            if !ptr::eq(s, buf) {
                len = CStr::from_ptr(s).to_bytes().len() as c_int;
                if len >= buflen {
                    len = buflen - 1;
                }
                len = len - e + 1;
                if len < 1 {
                    *buf.add((e - 1) as usize) = NUL;
                } else {
                    ptr::copy(s.add(e as usize), buf.add(e as usize), len as usize);
                }
            }
        } else if e + 3 < buflen {
            // set the middle and copy the last part
            ptr::copy(c"...".as_ptr(), buf.add(e as usize), 3);
            len = CStr::from_ptr(s.add(i as usize)).to_bytes().len() as c_int + 1;
            if len >= buflen - e - 3 {
                len = buflen - e - 3 - 1;
            }
            ptr::copy(s.add(i as usize), buf.add((e + 3) as usize), len as usize);
            *buf.add((e + 3 + len - 1) as usize) = NUL;
        } else {
            // can't fit in the "...", just truncate it
            *buf.add((buflen - 1) as usize) = NUL;
        }
    }
}

/// Reset the last used sourcing name/lnum.  Makes sure it is displayed again
/// for the next error message;
pub unsafe fn reset_last_sourcing() {
    unsafe {
        last_sourcing_name = None;
        last_sourcing_lnum = 0;
    }
}

/// @return  true if "SOURCING_NAME" differs from "last_sourcing_name".
unsafe fn other_sourcing_name() -> bool {
    unsafe {
        let sname = sourcing_name();
        if have_sourcing_info() && !sname.is_null() {
            if let Some(ref last) = *(&raw const last_sourcing_name).as_ref().unwrap() {
                return CStr::from_ptr(sname) != last.as_c_str();
            }
            return true;
        }
        false
    }
}

/// Get the message about the source, as used for an error message
unsafe fn get_emsg_source() -> Option<std::ffi::CString> {
    unsafe {
        let mut sname = sourcing_name();
        if have_sourcing_info() && !sname.is_null() && other_sourcing_name() {
            let tofree = cstr_take(estack_sfile(ESTACK_NONE));
            if let Some(ref tf) = tofree {
                sname = tf.as_ptr().cast_mut();
            }

            let p = gettext(c"Error in %s:".as_ptr());
            let buf_len =
                CStr::from_ptr(sname).to_bytes().len() + CStr::from_ptr(p).to_bytes().len() + 1;
            let mut buf_vec: Vec<c_char> = vec![0; buf_len];
            vim_snprintf!(buf_vec.as_mut_ptr(), buf_len, p, sname);
            return Some(CStr::from_ptr(buf_vec.as_ptr()).to_owned());
        }
        None
    }
}

/// Get the message about the source lnum, as used for an error message.
unsafe fn get_emsg_lnum() -> Option<std::ffi::CString> {
    unsafe {
        let sname = sourcing_name();
        let slnum = sourcing_lnum();
        if !sname.is_null() && (other_sourcing_name() || slnum != last_sourcing_lnum) && slnum != 0
        {
            let p = gettext(c"line %4d:".as_ptr());
            let buf_len = 20 + CStr::from_ptr(p).to_bytes().len();
            let mut buf_vec: Vec<c_char> = vec![0; buf_len];
            vim_snprintf!(buf_vec.as_mut_ptr(), buf_len, p, slnum);
            return Some(CStr::from_ptr(buf_vec.as_ptr()).to_owned());
        }
        None
    }
}

/// Display name and line number for the source of an error.
/// Remember the file name and line number, so that for the next error the info
/// is only displayed if it changed.
pub unsafe fn msg_source(hl_id: c_int) {
    unsafe {
        if MSG_SOURCE_RECURSIVE {
            return;
        }
        MSG_SOURCE_RECURSIVE = true;

        no_wait_return += 1;
        if let Some(p) = get_emsg_source() {
            msg_scroll = 1;
            msg(p.as_ptr(), hl_id);
        }
        if let Some(p) = get_emsg_lnum() {
            msg(p.as_ptr(), HLF_N);
            last_sourcing_lnum = sourcing_lnum();
        }

        let sname = sourcing_name();
        if sname.is_null() || other_sourcing_name() {
            last_sourcing_name = None;
            if !sname.is_null() {
                last_sourcing_name = Some(CStr::from_ptr(sname).to_owned());
                if redirecting() == 0 {
                    msg_putchar_hl(NL, hl_id);
                }
            }
        }
        no_wait_return -= 1;

        MSG_SOURCE_RECURSIVE = false;
    }
}

/// @return  true if not giving error messages right now:
///            If "emsg_off" is set: no error messages at the moment.
///            If "msg" is in 'debug': do error message but without side effects.
///            If "emsg_skip" is set: never do error messages.
pub unsafe fn emsg_not_now() -> bool {
    unsafe {
        (emsg_off > 0
            && vim_strchr(p_debug, b'm' as c_int).is_null()
            && vim_strchr(p_debug, b't' as c_int).is_null())
            || emsg_skip > 0
    }
}

pub unsafe fn emsg_multiline(
    s: *const c_char,
    kind: *const c_char,
    hl_id: c_int,
    multiline: bool,
) -> bool {
    unsafe {
        let mut ignore = false;

        // Skip this if not giving error messages at the moment.
        if emsg_not_now() {
            return true;
        }

        called_emsg += 1;

        // If "emsg_severe" is true: When an error exception is to be thrown,
        // prefer this message over previous messages for the same command.
        let severe = emsg_severe;
        emsg_severe = false;

        if emsg_off == 0 || !vim_strchr(p_debug, b't' as c_int).is_null() {
            if cause_errthrow(s, multiline, is_multihl > 1, severe, &raw mut ignore) {
                if !ignore {
                    did_emsg += 1;
                }
                return true;
            }

            if in_assert_fails
                && (&raw const emsg_assert_fails_msg)
                    .as_ref()
                    .unwrap()
                    .is_none()
            {
                emsg_assert_fails_msg = Some(CStr::from_ptr(s).to_owned());
                emsg_assert_fails_lnum = sourcing_lnum() as c_long;
                let sname = sourcing_name();
                emsg_assert_fails_context = Some(if sname.is_null() {
                    std::ffi::CString::default()
                } else {
                    CStr::from_ptr(sname).to_owned()
                });
            }

            // set "v:errmsg", also when using ":silent! cmd"
            set_vim_var_string(VV_ERRMSG, s, -1);

            let sname = sourcing_name();
            let slnum = sourcing_lnum();

            // When using ":silent! cmd" ignore error messages.
            // But do write it to the redirection file.
            if emsg_silent != 0 {
                if !emsg_noredir {
                    msg_start();
                    if let Some(p) = get_emsg_source() {
                        let p_len = p.as_bytes().len();
                        redir_write(p.as_ptr(), p_len as isize);
                        redir_write(c"\n".as_ptr(), 1);
                    }
                    if let Some(p) = get_emsg_lnum() {
                        let p_len = p.as_bytes().len();
                        redir_write(p.as_ptr(), p_len as isize);
                        redir_write(c"\n".as_ptr(), 1);
                    }
                    redir_write(s, CStr::from_ptr(s).to_bytes().len() as isize);
                }

                // Log (silent) errors as debug messages.
                if !sname.is_null() && slnum != 0 {
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"emsg_multiline".as_ptr(),
                        line!() as c_int,
                        true,
                        c"(:silent) %s (%s (line %d))".as_ptr(),
                        s,
                        sname,
                        slnum,
                    );
                } else {
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"emsg_multiline".as_ptr(),
                        line!() as c_int,
                        true,
                        c"(:silent) %s".as_ptr(),
                        s,
                    );
                }

                return true;
            }

            // Log editor errors as INFO.
            if !sname.is_null() && slnum != 0 {
                logmsg!(
                    LOGLVL_INF,
                    ptr::null(),
                    c"emsg_multiline".as_ptr(),
                    line!() as c_int,
                    true,
                    c"%s (%s (line %d))".as_ptr(),
                    s,
                    sname,
                    slnum,
                );
            } else {
                logmsg!(
                    LOGLVL_INF,
                    ptr::null(),
                    c"emsg_multiline".as_ptr(),
                    line!() as c_int,
                    true,
                    c"%s".as_ptr(),
                    s,
                );
            }

            ex_exitval = 1;

            // Reset msg_silent, an error causes messages to be switched back on.
            msg_silent = 0;
            cmd_silent = false;

            if global_busy != 0 {
                // break :global command
                global_busy += 1;
            }

            if p_eb != 0 {
                beep_flush(); // also includes flush_buffers()
            } else {
                flush_buffers(FLUSH_MINIMAL); // flush internal buffers
            }
            did_emsg += 1; // flag for DoOneCmd()
        }

        emsg_on_display = true; // remember there is an error message
        if msg_scrolled != 0 {
            need_wait_return = true;
        }
        msg_ext_set_kind(kind);

        // Display name and line number for the source of the error.
        msg_scroll = 1;
        let save_msg_skip_flush = msg_ext_skip_flush;
        msg_ext_skip_flush = true;
        msg_source(hl_id);

        // Display the error message itself.
        msg_nowait = false; // Wait for this msg.
        let rv = msg_keep(s, hl_id, false, multiline);
        msg_ext_skip_flush = save_msg_skip_flush;
        rv
    }
}

/// emsg() - display an error message
pub unsafe fn emsg(s: *const c_char) -> bool {
    unsafe { emsg_multiline(s, c"emsg".as_ptr(), HLF_E, false) }
}

pub unsafe fn emsg_invreg(name: c_int) {
    unsafe {
        semsg!(
            gettext(c"E354: Invalid register name: '%s'".as_ptr()),
            transchar_buf(ptr::null(), name),
        );
    }
}

/// Same as emsg(...), but abort on error when ABORT_ON_INTERNAL_ERROR is
/// defined. It is used for internal errors only, so that they can be
/// detected when fuzzing vim.
pub unsafe fn iemsg(s: *const c_char) {
    unsafe {
        if emsg_not_now() {
            return;
        }
        emsg(s);
    }
}

/// Give an "Internal error" message.
pub unsafe fn internal_error(where_: *const c_char) {
    unsafe {
        siemsg!(gettext(e_intern2.as_ptr()), where_);
    }
}

pub unsafe fn msg_semsg_event(s: std::ffi::CString) {
    unsafe {
        emsg(s.as_ptr());
    }
}

pub unsafe fn msg_schedule_semsg_cstr(s: *const c_char) {
    unsafe {
        let dup = CStr::from_ptr(s).to_owned();
        loop_schedule_deferred((&raw mut main_loop).cast(), Event::MsgSemsg(dup));
    }
}

pub unsafe fn msg_semsg_multiline_event(s: std::ffi::CString) {
    unsafe {
        emsg_multiline(s.as_ptr(), c"emsg".as_ptr(), HLF_E, true);
    }
}

pub unsafe fn msg_schedule_semsg_multiline_cstr(s: *const c_char) {
    unsafe {
        let dup = CStr::from_ptr(s).to_owned();
        loop_schedule_deferred((&raw mut main_loop).cast(), Event::MsgSemsgMultiline(dup));
    }
}

/// Like msg(), but truncate to a single line if p_shm contains 't', or when
/// "force" is true.  This truncates in another way as for normal messages.
/// Careful: The string may be changed by msg_may_trunc()!
///
/// @return  a pointer to the printed message, if wait_return() not called.
pub unsafe fn msg_trunc(s: *mut c_char, force: bool, hl_id: c_int) -> *mut c_char {
    unsafe {
        // Add message to history before truncating.
        msg_hist_add(s, -1, hl_id);

        let ts = msg_may_trunc(force, s);

        msg_hist_off = true;
        let n = msg(ts, hl_id);
        msg_hist_off = false;

        if n { ts } else { ptr::null_mut() }
    }
}

/// Check if message "s" should be truncated at the start (for filenames).
///
/// @return  a pointer to where the truncated message starts.
///
/// @note: May change the message by replacing a character with '<'.
pub unsafe fn msg_may_trunc(force: bool, mut s: *mut c_char) -> *mut c_char {
    unsafe {
        if ui_has(kUIMessages) {
            return s;
        }

        let room = (Rows - cmdline_row - 1) * Columns + sc_col - 1;
        if room > 0
            && (force || shortmess(kShmTrunc))
            && (CStr::from_ptr(s).to_bytes().len() as c_int) - room > 0
        {
            let mut size = vim_strsize(s);

            // There may be room anyway when there are multibyte chars.
            if size <= room {
                return s;
            }
            let mut n = 0usize;
            while size >= room {
                size -= utf_ptr2cells(s.add(n));
                n += utfc_ptr2len(s.add(n)) as usize;
            }
            n -= 1;
            s = s.add(n);
            *s = b'<' as c_char;
        }
        s
    }
}

/// Shows a progress-message.  A non-"running" `status` ends the message and announces the
/// progress task as "done".
pub unsafe fn msg_progress(
    s: *mut c_char,
    id: *mut c_char,
    status: *mut c_char,
    percent: c_int,
    hl_id: c_int,
    hist: bool,
    trunc: bool,
    err: bool,
) -> *mut c_char {
    unsafe {
        let mut data = MessageData {
            source: cstr_as_string(c"nvim".as_ptr()),
            percent: Integer::from(percent),
            title: NvimString {
                data: ptr::null_mut(),
                size: 0,
            },
            status: cstr_as_string(status),
            data: Dict::new(),
        };
        msg_with_id(
            s,
            id,
            c"progress".as_ptr(),
            &raw mut data,
            hl_id,
            hist,
            trunc,
            err,
        )
    }
}

/// Shows message `s` with `id`: replaces the message with the same `id`, if any.
pub unsafe fn msg_with_id(
    mut s: *mut c_char,
    id: *mut c_char,
    kind: *const c_char,
    data: *mut MessageData,
    hl_id: c_int,
    hist: bool,
    trunc: bool,
    err: bool,
) -> *mut c_char {
    unsafe {
        let mut clear = false;
        let mut chunks = HlMessage::new();
        if !s.is_null() {
            if hist && (!trunc || ui_has(kUIMessages)) {
                msg_hist_add(s, -1, 0);
            }
            if trunc {
                s = msg_may_trunc(false, s);
            }
            chunks.push(HlMessageChunk {
                text: cstr_to_string(s),
                hl_id,
            });
        }
        msg_ext_no_fast();
        let id_obj = Object {
            obj_type: ObjectType::kObjectTypeString,
            data: ObjectData {
                string: cstr_as_string(id),
            },
        };
        msg_multihl(
            id_obj,
            &mut chunks,
            kind,
            false,
            err,
            data,
            &raw mut clear,
            false,
        );
        ui_flush();
        s
    }
}

/// Add the message at the end of the history
///
/// @param[in]  len  Length of s or -1.
unsafe fn msg_hist_add(mut s: *const c_char, len: c_int, hl_id: c_int) {
    unsafe {
        let mut size = if len < 0 {
            CStr::from_ptr(s).to_bytes().len()
        } else {
            len as usize
        };
        // Remove leading and trailing newlines.
        while size > 0 && *s == NL as c_char {
            size -= 1;
            s = s.add(1);
        }
        while size > 0 && *s.add(size - 1) == NL as c_char {
            size -= 1;
        }
        if size == 0 {
            return;
        }
        let text = NvimString {
            data: xmemdupz(s.cast(), size).cast(),
            size,
        };

        let msg = vec![HlMessageChunk { text, hl_id }];
        msg_hist_add_multihl(msg, false);
    }
}

pub unsafe fn do_autocmd_progress(msg_id: MsgID, msg: &HlMessage, msg_data: *mut MessageData) {
    unsafe {
        if !has_event(EVENT_PROGRESS) {
            return;
        }

        let mut messages_vec: Vec<Object> = msg
            .iter()
            .map(|chunk| Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData { string: chunk.text },
            })
            .collect();
        let messages = Array {
            items: messages_vec.as_mut_ptr(),
            size: messages_vec.len(),
            capacity: messages_vec.len(),
        };

        let empty_pair = KeyValuePair {
            key: NvimString {
                data: ptr::null_mut(),
                size: 0,
            },
            value: NIL_OBJ,
        };
        let mut data_items = [empty_pair; 7];
        let mut data_size = 0usize;
        let mut put_c = |key: &'static CStr, value: Object| {
            data_items[data_size] = KeyValuePair {
                key: NvimString {
                    data: key.as_ptr().cast_mut(),
                    size: key.to_bytes().len(),
                },
                value,
            };
            data_size += 1;
        };

        put_c(c"id", msg_id);
        put_c(
            c"text",
            Object {
                obj_type: ObjectType::kObjectTypeArray,
                data: ObjectData { array: messages },
            },
        );
        if !msg_data.is_null() {
            if (*msg_data).percent >= 0 {
                put_c(
                    c"percent",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: (*msg_data).percent,
                        },
                    },
                );
            }
            put_c(
                c"source",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: (*msg_data).source,
                    },
                },
            );
            put_c(
                c"status",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: (*msg_data).status,
                    },
                },
            );
            put_c(
                c"title",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: (*msg_data).title,
                    },
                },
            );
            put_c(
                c"data",
                Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData {
                        dict: (*msg_data).data,
                    },
                },
            );
        }

        let mut dict_obj = Object {
            obj_type: ObjectType::kObjectTypeDict,
            data: ObjectData {
                dict: Dict {
                    items: data_items.as_mut_ptr(),
                    size: data_size,
                    capacity: 7,
                },
            },
        };
        let fname = if !msg_data.is_null() && (*msg_data).source.size > 0 {
            (*msg_data).source.data
        } else {
            c"".as_ptr().cast_mut()
        };
        apply_autocmds_group(
            EVENT_PROGRESS,
            fname,
            ptr::null_mut(),
            true,
            AUGROUP_ALL,
            ptr::null_mut(),
            curwin,
            ptr::null_mut(),
            &raw mut dict_obj,
            false,
        );
    }
}

unsafe fn msg_hist_add_multihl(msg: HlMessage, temp: bool) {
    unsafe {
        if do_clear_hist_temp {
            msg_hist_clear_temp();
            do_clear_hist_temp = false;
        }

        if msg_hist_off || msg_silent != 0 {
            return;
        }

        let hist = &raw mut msg_hist;
        let temp_idx = msg_hist_temp_idx;
        if temp_idx.is_none() {
            msg_hist_temp_idx = Some((*hist).len());
        }
        (*hist).push_back(MessageHistoryEntry {
            msg,
            kind: (!msg_ext_kind.is_null()).then(|| CStr::from_ptr(msg_ext_kind).to_owned()),
            temp,
            append: msg_ext_append,
        });

        msg_hist_len += c_int::from(!temp);
        msg_ext_history = true;

        msg_hist_clear(msg_hist_max);
    }
}

/// Delete oldest messages from the history until there are "keep" messages.
pub unsafe fn msg_hist_clear(keep: c_int) {
    unsafe {
        let hist = &raw mut msg_hist;
        while msg_hist_len > keep || (keep == 0 && !(*hist).is_empty()) {
            if let Some(entry) = (*hist).pop_front() {
                msg_hist_len -= c_int::from(!entry.temp);
                if let Some(idx) = msg_hist_temp_idx {
                    msg_hist_temp_idx = Some(idx.saturating_sub(1));
                }
            }
        }
        if (*hist).is_empty() {
            msg_hist_temp_idx = None;
        }
    }
}

pub unsafe fn msg_hist_clear_temp() {
    unsafe {
        let temp_idx = msg_hist_temp_idx;
        msg_hist_temp_idx = None;
        if let Some(start_idx) = temp_idx {
            let hist = &raw mut msg_hist;
            let mut i = start_idx;
            while i < (*hist).len() {
                if (&(*hist))[i].temp {
                    (*hist).remove(i);
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// Check the 'messagesopt' fields after schema validation.
pub unsafe fn validate_messagesopt(args: *mut optset_T) -> *const c_char {
    unsafe {
        let v = opt_keyset(
            (*args).os_newval.data.string.data,
            kOptMessagesopt,
            ptr::null_mut(),
        )
        .cast::<OptKeyDict_mopt>();

        let has_wait = ((*v).is_set__mopt_ & (1 << KEYSET_OPTIDX_mopt__wait)) != 0;
        let has_history = ((*v).is_set__mopt_ & (1 << KEYSET_OPTIDX_mopt__history)) != 0;

        // Either "wait" or "hit-enter" is required; "history" always.
        if (!(*v).hit_enter && !has_wait) || !has_history {
            return e_invarg.as_ptr();
        }
        // "history" and "wait" must be <= 10000; "maxheight" is a percentage.
        if (*v).history > 10000 || (*v).wait > 10000 || (*v).maxheight > 100 {
            return e_invarg.as_ptr();
        }
        ptr::null()
    }
}

/// Process the updated 'messagesopt' option value.
pub unsafe fn did_set_messagesopt(_args: *mut optset_T) -> *const c_char {
    unsafe {
        let v = opt_keyset(p_mopt, kOptMessagesopt, ptr::null_mut()).cast::<OptKeyDict_mopt>();

        msg_hit_enter = (*v).hit_enter;
        msg_wait = (*v).wait as c_int;
        progress_msg_target = if strequal((*v).progress.as_ptr(), c"c".as_ptr()) {
            PROGRESS_TARGET_CMD
        } else {
            0
        };
        msg_hist_max = (*v).history as c_int;
        msg_hist_clear(msg_hist_max);
        ptr::null()
    }
}

/// :messages command implementation
pub unsafe fn ex_messages(eap: *mut exarg_T) {
    unsafe {
        if CStr::from_ptr((*eap).arg) == c"clear" {
            msg_hist_clear(if (*eap).addr_count != 0 {
                (*eap).line2 as c_int
            } else {
                0
            });
            return;
        }

        if *(*eap).arg != NUL {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut entries = Array::new();
        let hist = &raw mut msg_hist;
        let mut idx = if (*eap).skip != 0 {
            msg_hist_temp_idx.unwrap_or((*hist).len())
        } else {
            0
        };
        let mut skip = if (*eap).addr_count != 0 {
            msg_hist_len - (*eap).line2 as c_int
        } else {
            0
        };
        while idx < (*hist).len() {
            let p: *mut MessageHistoryEntry = &raw mut (&mut (*hist))[idx];
            idx += 1;
            // Skip over count or temporary "g<" messages.
            let prev_skip = skip;
            skip -= 1;
            if ((*p).temp && (*eap).skip == 0) || prev_skip > 0 {
                continue;
            }
            if ui_has(kUIMessages) && msg_silent == 0 {
                let mut entry = Array::new();
                entry.push(Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_to_string(
                            (*p).kind.as_ref().map_or(ptr::null(), |k| k.as_ptr()),
                        ),
                    },
                });
                let mut content = Array::new();
                for chunk in &(*p).msg {
                    let mut content_entry = Array::new();
                    content_entry.push(Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: if chunk.hl_id != 0 {
                                syn_id2attr(chunk.hl_id) as Integer
                            } else {
                                0
                            },
                        },
                    });
                    content_entry.push(Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: copy_string(chunk.text, ptr::null_mut()),
                        },
                    });
                    content_entry.push(Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: chunk.hl_id as Integer,
                        },
                    });
                    content.push(Object {
                        obj_type: ObjectType::kObjectTypeArray,
                        data: ObjectData {
                            array: content_entry,
                        },
                    });
                }
                entry.push(Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData { array: content },
                });
                entry.push(Object {
                    obj_type: ObjectType::kObjectTypeBoolean,
                    data: ObjectData {
                        boolean: (*p).append,
                    },
                });
                entries.push(Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData { array: entry },
                });
            }
            if redirecting() != 0 || !ui_has(kUIMessages) {
                msg_silent += c_int::from(ui_has(kUIMessages));
                let mut needs_clear = false;
                msg_multihl(
                    NIL_OBJ,
                    &mut (*p).msg,
                    (*p).kind.as_ref().map_or(ptr::null(), |k| k.as_ptr()),
                    false,
                    false,
                    ptr::null_mut(),
                    &raw mut needs_clear,
                    false,
                );
                msg_silent -= c_int::from(ui_has(kUIMessages));
            }
        }
        if entries.size > 0 {
            ui_call_msg_history_show(entries, (*eap).skip != 0);
            api_free_array(entries);
        }
    }
}

/// Call this after prompting the user.  This will avoid a hit-return message
/// and a delay.
pub unsafe fn msg_end_prompt() {
    unsafe {
        need_wait_return = false;
        emsg_on_display = false;
        cmdline_row = msg_row;
        msg_col = 0;
        msg_clr_eos();
        lines_left = -1;
    }
}

/// Wait for the user to hit a key (normally Enter)
///
/// @param redraw  if true, redraw the entire screen UPD_NOT_VALID
///                if false, do a normal redraw
///                if -1, don't redraw at all
pub unsafe fn wait_return(redraw: c_int) {
    unsafe {
        let mut c: c_int;
        let mut had_got_int: bool;

        if redraw == 1 {
            redraw_all_later(UPD_NOT_VALID);
        }

        if ui_has(kUIMessages) {
            prompt_for_input(
                c"Press any key to continue".as_ptr().cast_mut(),
                HLF_M,
                true,
                ptr::null_mut(),
            );
            return;
        }

        // If using ":silent cmd", don't wait for a return.  Also don't set
        // need_wait_return to do it later.
        if msg_silent != 0 {
            return;
        }

        if (headless_mode && ui_active() == 0) || input_unanswerable() {
            return;
        }

        // When inside vgetc(), we can't wait for a typed character at all.
        if vgetc_busy > 0 {
            return;
        }
        need_wait_return = true;
        if no_wait_return != 0 {
            cmdline_row = msg_row;
            return;
        }

        redir_off = true; // don't redirect this message
        let oldState = State;
        if quit_more {
            c = CAR; // just pretend CR was hit
            quit_more = false;
            got_int = false;
        } else if !stuff_empty() {
            c = CAR;
        } else {
            State = MODE_HITRETURN;
            setmouse();
            cmdline_row = msg_row;
            if need_check_timestamps {
                check_timestamps(0);
            }

            // if cmdheight=0, we need to scroll in the first line of msg_grid upon the screen
            if p_ch == 0 && !ui_has(kUIMessages) && msg_scrolled == 0 {
                msg_grid_validate();
                msg_scroll_up(false, true);
                msg_scrolled += 1;
                cmdline_row = Rows - 1;
            }

            if msg_hit_enter {
                hit_return_msg(true);

                loop {
                    had_got_int = got_int;

                    no_mapping += 1;
                    allow_keys += 1;

                    let save_reg_recording = reg_recording;
                    let save_scriptout = scriptout;
                    reg_recording = 0;
                    scriptout = ptr::null_mut();
                    c = safe_vgetc();
                    if had_got_int && global_busy == 0 {
                        got_int = false;
                    }
                    no_mapping -= 1;
                    allow_keys -= 1;
                    reg_recording = save_reg_recording;
                    scriptout = save_scriptout;

                    if KeyTyped && p_more != 0 {
                        if c == b'b' as c_int
                            || c == Ctrl_B
                            || c == b'k' as c_int
                            || c == b'u' as c_int
                            || c == b'g' as c_int
                            || c == K_UP
                            || c == K_PAGEUP
                        {
                            if msg_scrolled > Rows {
                                do_more_prompt(c);
                            } else {
                                msg_didout = false;
                                c = K_IGNORE;
                                msg_col = 0;
                            }
                            if quit_more {
                                c = CAR;
                                quit_more = false;
                                got_int = false;
                            } else if c != K_IGNORE {
                                c = K_IGNORE;
                                hit_return_msg(false);
                            }
                        } else if msg_scrolled > Rows - 2
                            && (c == b'j' as c_int
                                || c == b'd' as c_int
                                || c == b'f' as c_int
                                || c == Ctrl_F
                                || c == K_DOWN
                                || c == K_PAGEDOWN)
                        {
                            c = K_IGNORE;
                        }
                    }

                    if !((had_got_int && c == Ctrl_C)
                        || c == K_IGNORE
                        || c == K_LEFTDRAG
                        || c == K_LEFTRELEASE
                        || c == K_MIDDLEDRAG
                        || c == K_MIDDLERELEASE
                        || c == K_RIGHTDRAG
                        || c == K_RIGHTRELEASE
                        || c == K_MOUSELEFT
                        || c == K_MOUSERIGHT
                        || c == K_MOUSEDOWN
                        || c == K_MOUSEUP
                        || c == K_MOUSEMOVE)
                    {
                        break;
                    }
                }
                os_breakcheck();

                // Avoid that the mouse-up event causes visual mode to start.
                if c == K_LEFTMOUSE
                    || c == K_MIDDLEMOUSE
                    || c == K_RIGHTMOUSE
                    || c == K_X1MOUSE
                    || c == K_X2MOUSE
                {
                    jump_to_mouse(MOUSE_SETPOS, ptr::null_mut(), 0);
                } else if !KeyTyped
                    || (vim_strchr(c"\r\n ".as_ptr(), c).is_null()
                        && c != Ctrl_C
                        && c != b'q' as c_int)
                {
                    requeue_key(vgetc_char, vgetc_mod_mask, 0, false);
                    do_redraw = true;
                }
            } else {
                c = CAR;
                do_sleep(i64::from(msg_wait), true);
            }
        }
        redir_off = false;

        if c == b':' as c_int || c == b'?' as c_int || c == b'/' as c_int {
            cmdline_row = msg_row;
            skip_redraw = true;
            do_redraw = false;
        }

        let tmpState = State;
        State = oldState;
        setmouse();
        msg_check();
        need_wait_return = false;
        did_wait_return = true;
        emsg_on_display = false;
        lines_left = -1;
        reset_last_sourcing();
        if (&raw const keep_msg).as_ref().unwrap().as_ref().is_some_and(
            |km| vim_strsize(km.as_ptr()) >= (Rows - cmdline_row - 1) * Columns + sc_col,
        ) {
            keep_msg = None;
        }

        if tmpState == MODE_SETWSIZE {
            ui_refresh();
        } else if !skip_redraw && (redraw == 1 || (msg_scrolled != 0 && redraw != -1)) {
            redraw_later(curwin, UPD_VALID);
        }
    }
}

/// Write the hit-return prompt.
unsafe fn hit_return_msg(newline_sb: bool) {
    unsafe {
        let save_p_more = p_more;

        if !newline_sb {
            p_more = 0;
        }
        if msg_didout {
            msg_putchar(NL);
        }
        p_more = 0;
        if got_int {
            msg_puts(gettext(c"Interrupt: ".as_ptr()));
        }

        msg_puts_hl(
            gettext(c"Press ENTER or type command to continue".as_ptr()),
            HLF_R,
            false,
        );
        if msg_use_printf() == 0 {
            msg_clr_eos();
        }
        p_more = save_p_more;
    }
}

/// Set "keep_msg" to "s".  Free the old value and check for NULL pointer.
pub unsafe fn set_keep_msg(s: *const c_char, hl_id: c_int) {
    unsafe {
        if ui_has(kUIMessages) {
            return;
        }

        if !s.is_null() && msg_silent == 0 {
            keep_msg = Some(CStr::from_ptr(s).to_owned());
        } else {
            keep_msg = None;
        }
        keep_msg_more = false;
        keep_msg_hl_id = hl_id;
    }
}

/// Return true if printing messages should currently be done.
pub unsafe fn messaging() -> bool {
    unsafe { !(p_lz != 0 && char_avail() && !KeyTyped) && (p_ch > 0 || ui_has(kUIMessages)) }
}

pub unsafe fn msgmore(n: c_int) {
    unsafe {
        if global_busy != 0 || !messaging() {
            return;
        }

        if (&raw const keep_msg).as_ref().unwrap().is_some() && !keep_msg_more {
            return;
        }

        let pn = n.abs();

        if i64::from(pn) > p_report {
            let msg_ptr = (&raw mut msg_buf).cast::<c_char>();
            if n > 0 {
                vim_snprintf!(
                    msg_ptr,
                    MSG_BUF_LEN,
                    ngettext(
                        c"%d more line".as_ptr(),
                        c"%d more lines".as_ptr(),
                        pn as c_ulong,
                    ),
                    pn,
                );
            } else {
                vim_snprintf!(
                    msg_ptr,
                    MSG_BUF_LEN,
                    ngettext(
                        c"%d line less".as_ptr(),
                        c"%d fewer lines".as_ptr(),
                        pn as c_ulong,
                    ),
                    pn,
                );
            }
            if got_int {
                xstrlcat(msg_ptr, gettext(c" (Interrupted)".as_ptr()), MSG_BUF_LEN);
            }
            if msg(msg_ptr, 0) {
                set_keep_msg(msg_ptr, 0);
                keep_msg_more = true;
            }
        }
    }
}

pub unsafe fn msg_ext_set_kind(msg_kind: *const c_char) {
    unsafe {
        msg_ext_ui_flush();
        msg_ext_kind = msg_kind;
        redir_col = if msg_ext_append { redir_col } else { 0 };

        if !msg_kind.is_null() && CStr::from_ptr(msg_kind) == c"list_cmd" {
            msg_ext_no_fast();
        }
    }
}

pub unsafe fn msg_ext_set_append(append: bool) {
    unsafe {
        msg_ext_ui_flush();
        msg_ext_append = append;
    }
}

pub unsafe fn msg_ext_set_trigger(trigger: *const c_char) {
    unsafe {
        msg_ext_ui_flush();
        msg_ext_trigger = trigger;
    }
}

// Should be executed at all callsites emitting non-internal messages.
pub unsafe fn msg_ext_no_fast() {
    unsafe {
        msg_ext_ui_flush();
        msg_ext_fast = false;
    }
}

/// Prepare for outputting characters in the command line.
pub unsafe fn msg_start() {
    unsafe {
        let mut did_return = false;

        msg_row = msg_row.max(cmdline_row);

        if msg_silent == 0 {
            keep_msg = None;
            need_fileinfo = false;
        }

        if need_highlight_changed {
            highlight_changed();
        }

        if need_clr_eos || (p_ch == 0 && redrawing_cmdline) {
            need_clr_eos = false;
            msg_clr_eos();
        }

        // if cmdheight=0, we need to scroll in the first line of msg_grid upon the screen
        if p_ch == 0 && !ui_has(kUIMessages) && msg_scrolled == 0 {
            msg_grid_validate();
            msg_scroll_up(false, true);
            msg_scrolled += 1;
            cmdline_row = Rows - 1;
        }

        if msg_scroll == 0 && full_screen {
            msg_row = cmdline_row;
            msg_col = 0;
        } else if (msg_didout || p_ch == 0) && !ui_has(kUIMessages) {
            if ((p_ch == 0 && !msg_didout) || msg_special_case_for_lua_print)
                && msg_use_printf() != 0
            {
                msg_puts_display(c"\n".as_ptr(), 1, 0, 0);
            } else {
                msg_putchar(NL);
            }
            did_return = true;
            cmdline_row = msg_row;
        }
        msg_special_case_for_lua_print = false;
        if !msg_didany || lines_left < 0 {
            msg_starthere();
        }
        if msg_silent == 0 {
            msg_didout = false;
        }

        if ui_has(kUIMessages) {
            msg_ext_ui_flush();
        }

        if !did_return {
            redir_write(c"\n".as_ptr(), 1);
        }
    }
}

/// Note that the current msg position is where messages start.
pub unsafe fn msg_starthere() {
    unsafe {
        lines_left = cmdline_row;
        msg_didany = false;
    }
}

pub unsafe fn msg_putchar(c: c_int) {
    unsafe {
        msg_putchar_hl(c, 0);
    }
}

pub unsafe fn msg_putchar_hl(c: c_int, hl_id: c_int) {
    unsafe {
        let mut buf = [0 as c_char; MB_MAXCHAR + 1];

        if is_special(c) {
            buf[0] = K_SPECIAL as c_char;
            buf[1] = k_second(c) as c_char;
            buf[2] = k_third(c) as c_char;
            buf[3] = NUL;
        } else {
            let len = utf_char2bytes(c, buf.as_mut_ptr()) as usize;
            buf[len] = NUL;
        }
        msg_puts_hl(buf.as_ptr(), hl_id, false);
    }
}

pub unsafe fn msg_outnum(n: c_int) {
    unsafe {
        let mut buf = [0 as c_char; 20];
        vim_snprintf!(buf.as_mut_ptr(), buf.len(), c"%d".as_ptr(), n);
        msg_puts(buf.as_ptr());
    }
}

pub unsafe fn msg_home_replace(fname: *const c_char) {
    unsafe {
        msg_home_replace_hl(fname, 0);
    }
}

unsafe fn msg_home_replace_hl(fname: *const c_char, hl_id: c_int) {
    unsafe {
        let name = cstr_take(home_replace_save(ptr::null_mut(), fname));
        msg_outtrans(
            name.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            hl_id,
            false,
        );
    }
}

/// Output "len" characters in "str" (including NULs) with translation
/// if "len" is -1, output up to a NUL character. Use highlight "hl_id".
///
/// @return  the number of characters it takes on the screen.
pub unsafe fn msg_outtrans(str_: *const c_char, hl_id: c_int, hist: bool) -> c_int {
    unsafe {
        msg_outtrans_len(
            str_,
            CStr::from_ptr(str_).to_bytes().len() as c_int,
            hl_id,
            hist,
        )
    }
}

pub unsafe fn msg_outtrans_len(
    msgstr: *const c_char,
    mut len: c_int,
    hl_id: c_int,
    hist: bool,
) -> c_int {
    unsafe {
        let mut retval = 0;
        let mut str_ = msgstr;
        let mut plain_start = msgstr;
        let save_got_int = got_int;

        // Only quit when got_int was set in here.
        got_int = false;

        if hist {
            msg_hist_add(str_, len, hl_id);
        }

        // When drawing over the command line no need to clear it later or remove
        // the mode message.
        if msg_silent == 0 && len > 0 && msg_row >= cmdline_row && msg_col == 0 {
            clear_cmdline = false;
            mode_displayed = false;
        }

        // Go over the string.  Special characters are translated and printed.
        // Normal characters are printed several at a time.
        while {
            len -= 1;
            len >= 0 && !got_int
        } {
            // Don't include composing chars after the end.
            let mb_l = utfc_ptr2len_len(str_, len + 1);
            if mb_l > 1 {
                let c = utf_ptr2char(str_);
                if vim_isprintc(c) {
                    // Printable multi-byte char: count the cells.
                    retval += utf_ptr2cells(str_);
                } else {
                    // Unprintable multi-byte char: print the printable chars so
                    // far and the translation of the unprintable char.
                    if str_ > plain_start {
                        msg_puts_len(plain_start, str_.offset_from(plain_start), hl_id, hist);
                    }
                    plain_start = str_.add(mb_l as usize);
                    msg_puts_hl(
                        transchar_buf(ptr::null(), c),
                        if hl_id == 0 { HLF_8 } else { hl_id },
                        false,
                    );
                    retval += char2cells(c);
                }
                len -= mb_l - 1;
                str_ = str_.add(mb_l as usize);
            } else {
                let s = transchar_byte_buf(ptr::null(), c_int::from(*str_ as u8));
                if *s.add(1) != NUL {
                    if str_ > plain_start {
                        msg_puts_len(plain_start, str_.offset_from(plain_start), hl_id, hist);
                    }
                    plain_start = str_.add(1);
                    msg_puts_hl(s, if hl_id == 0 { HLF_8 } else { hl_id }, false);
                    retval += CStr::from_ptr(s).to_bytes().len() as c_int;
                } else {
                    retval += 1;
                }
                str_ = str_.add(1);
            }
        }

        if (str_ > plain_start || plain_start == msgstr) && !got_int {
            msg_puts_len(plain_start, str_.offset_from(plain_start), hl_id, hist);
        }

        got_int |= save_got_int;

        retval
    }
}

pub unsafe fn msg_make(mut arg: *const c_char) {
    unsafe {
        const STR: &[u8; 6] = b"eeffoc";
        const RS: &[u8; 15] = b"Plon#dqg#vxjduB";

        arg = skipwhite(arg);
        let mut i: isize = 5;
        while *arg != NUL && i >= 0 {
            if *arg as u8 != STR[i as usize] {
                break;
            }
            arg = arg.add(1);
            i -= 1;
        }
        if i < 0 {
            msg_putchar(NL);
            for &b in RS {
                msg_putchar(c_int::from(b - 3));
            }
        }
    }
}

/// Output the string 'str' up to a NUL character.
/// Return the number of characters it takes on the screen.
pub unsafe fn msg_outtrans_special(strstart: *const c_char, from: bool, maxlen: c_int) -> c_int {
    unsafe {
        if strstart.is_null() {
            return 0;
        }
        let mut str_ = strstart;
        let mut retval = 0;
        let hl_id = HLF_8;

        while *str_ != NUL {
            let mut text = if (str_ == strstart || *str_.add(1) == NUL) && *str_ == b' ' as c_char {
                str_ = str_.add(1);
                c"<Space>".as_ptr()
            } else {
                str2special(&raw mut str_, from, kFalse, ptr::null_mut())
            };
            if *text != NUL && *text.add(1) == NUL {
                text = transchar_byte_buf(ptr::null(), c_int::from(*text as u8));
            }
            let len = vim_strsize(text);
            if maxlen > 0 && retval + len >= maxlen {
                break;
            }
            // Highlight special keys
            msg_puts_hl(
                text,
                if len > 1 && utfc_ptr2len(text) <= 1 {
                    hl_id
                } else {
                    0
                },
                false,
            );
            retval += len;
        }
        retval
    }
}

/// Convert string, replacing key codes with printables
pub unsafe fn str2special_save(
    str_: *const c_char,
    replace_spaces: bool,
    replace_others: TriState,
) -> *mut c_char {
    unsafe {
        let mut buf: Vec<u8> = Vec::with_capacity(40);
        let mut p = str_;
        while *p != NUL {
            let s = str2special(&raw mut p, replace_spaces, replace_others, ptr::null_mut());
            if !s.is_null() {
                buf.extend_from_slice(CStr::from_ptr(s).to_bytes());
            }
        }
        xmemdupz(buf.as_ptr().cast(), buf.len()).cast()
    }
}

/// Convert string, replacing key codes with printables
pub unsafe fn str2special_arena(
    str_: *const c_char,
    replace_spaces: bool,
    replace_others: TriState,
    arena: *mut Arena,
) -> *mut c_char {
    unsafe {
        let mut p = str_;
        let mut len = 0usize;
        while *p != NUL {
            let s = str2special(&raw mut p, replace_spaces, replace_others, ptr::null_mut());
            len += CStr::from_ptr(s).to_bytes().len();
        }

        let buf = arena_alloc(arena, len + 1, false).cast::<c_char>();
        let mut pos = 0usize;
        p = str_;
        while *p != NUL {
            let s = str2special(&raw mut p, replace_spaces, replace_others, ptr::null_mut());
            let s_len = CStr::from_ptr(s).to_bytes().len();
            ptr::copy_nonoverlapping(s, buf.add(pos), s_len);
            pos += s_len;
        }
        *buf.add(pos) = NUL;
        buf
    }
}

/// Convert character, replacing key with printable representation.
pub unsafe fn str2special(
    sp: *mut *const c_char,
    replace_spaces: bool,
    replace_others: TriState,
    data: *mut keychord,
) -> *const c_char {
    unsafe {
        let p = mb_unescape(sp);
        if !p.is_null() {
            if !data.is_null() {
                (*data).mods = 0;
                (*data).key = NvimString {
                    data: p.cast_mut(),
                    size: CStr::from_ptr(p).to_bytes().len(),
                };
                (*data).key_alt = NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                };
            }
            return p;
        }

        let mut str_ = *sp;
        let mut c = (*str_ as u8) as c_int;
        let mut modifiers = 0;
        let mut special = false;
        if c == K_SPECIAL as c_int && *str_.add(1) != NUL && *str_.add(2) != NUL {
            if *str_.add(1) as u8 == KS_MODIFIER {
                modifiers = (*str_.add(2) as u8) as c_int;
                str_ = str_.add(3);
                c = (*str_ as u8) as c_int;
            }
            if c == K_SPECIAL as c_int && *str_.add(1) != NUL && *str_.add(2) != NUL {
                c = to_special(*str_.add(1) as u8 as c_int, *str_.add(2) as u8 as c_int);
                str_ = str_.add(2);
            }
            if is_special(c) || modifiers != 0 {
                special = true;
            }
        }

        if !is_special(c) && utf8len_tab[c as usize] > 1 {
            *sp = str_;
            let p = mb_unescape(sp);
            if !p.is_null() {
                c = utf_ptr2char(p);
            } else {
                *sp = str_.add(1);
            }
        } else {
            *sp = str_.add(usize::from(*str_ != NUL));
        }

        if special
            || c < b' ' as c_int
            || (replace_spaces && c == b' ' as c_int)
            || (replace_others != kFalse && c == b'<' as c_int)
            || (replace_others == kTrue && (c == b'|' as c_int || c == b'\\' as c_int))
        {
            return get_special_key(c, modifiers, data);
        }

        let buf_ptr = (&raw mut STR2SPECIAL_BUF).cast::<c_char>();
        if !data.is_null() {
            (*data).mods = 0;
            if (b'A' as c_int..=b'Z' as c_int).contains(&c) {
                (*data).key_mem = (c + 32) as c_char;
                (*data).key = NvimString {
                    data: &raw mut (*data).key_mem,
                    size: 1,
                };
                (*data).mods |= c_int::from(MOD_MASK_SHIFT);
                (*data).key_alt = NvimString {
                    data: buf_ptr,
                    size: 1,
                };
            } else {
                (*data).key = NvimString {
                    data: buf_ptr,
                    size: 1,
                };
                (*data).key_alt = NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                };
            }
        }

        *buf_ptr = c as c_char;
        *buf_ptr.add(1) = NUL;
        buf_ptr
    }
}

/// print line for :print or :list command
pub unsafe fn msg_prt_line(mut s: *const c_char, mut list: bool) {
    unsafe {
        let mut sc: schar_T;
        let mut col = 0;
        let mut n_extra = 0;
        let mut sc_extra: schar_T = 0;
        let mut sc_final: schar_T = 0;
        let mut p_extra: *const c_char = ptr::null();
        let mut hl_id = 0;
        let mut lead: *const c_char = ptr::null();
        let mut in_multispace: bool;
        let mut multispace_pos = 0usize;
        let mut trail: *const c_char = ptr::null();

        if (*curwin).w_onebuf_opt.wo_list != 0 {
            list = true;
        }

        if list {
            // find start of trailing whitespace
            if (*curwin).w_p_lcs_chars.trail != 0 {
                trail = s.add(CStr::from_ptr(s).to_bytes().len());
                while trail > s && ascii_iswhite(*trail.sub(1) as c_int) {
                    trail = trail.sub(1);
                }
            }
            // find end of leading whitespace
            if (*curwin).w_p_lcs_chars.lead != 0
                || !(*curwin).w_p_lcs_chars.leadmultispace.is_empty()
                || (*curwin).w_p_lcs_chars.leadtab1 != 0
            {
                lead = s;
                while ascii_iswhite(*lead as c_int) {
                    lead = lead.add(1);
                }
                if *lead == NUL {
                    lead = ptr::null();
                }
            }
        }

        // output a space for an empty line, otherwise the line will be overwritten
        if *s == NUL && !(list && (*curwin).w_p_lcs_chars.eol != 0) {
            msg_putchar(b' ' as c_int);
        }

        while !got_int {
            if n_extra > 0 {
                n_extra -= 1;
                if n_extra == 0 && sc_final != 0 {
                    sc = sc_final;
                } else if sc_extra != 0 {
                    sc = sc_extra;
                } else {
                    assert!(!p_extra.is_null());
                    sc = schar_from_ascii(*p_extra as u8);
                    p_extra = p_extra.add(1);
                }
            } else {
                let l = utfc_ptr2len(s);
                if l > 1 {
                    col += utf_ptr2cells(s);
                    let mut buf = [0 as c_char; MB_MAXBYTES + 1];
                    if l as usize >= MB_MAXBYTES {
                        xstrlcpy(buf.as_mut_ptr(), c"?".as_ptr(), buf.len());
                    } else if (*curwin).w_p_lcs_chars.nbsp != 0
                        && list
                        && (utf_ptr2char(s) == 160 || utf_ptr2char(s) == 0x202f)
                    {
                        schar_get(buf.as_mut_ptr(), (*curwin).w_p_lcs_chars.nbsp);
                    } else {
                        ptr::copy(s, buf.as_mut_ptr(), l as usize);
                        buf[l as usize] = NUL;
                    }
                    msg_puts(buf.as_ptr());
                    s = s.add(l as usize);
                    continue;
                } else {
                    hl_id = 0;
                    let c = (*s as u8) as c_int;
                    s = s.add(1);
                    if c >= 0x80 {
                        col += utf_char2cells(c);
                        msg_putchar(c);
                        continue;
                    }
                    sc_extra = 0;
                    sc_final = 0;
                    if list {
                        in_multispace = c == b' ' as c_int
                            && (*s == b' ' as c_char || (col > 0 && *s.sub(2) == b' ' as c_char));
                        if !in_multispace {
                            multispace_pos = 0;
                        }
                    } else {
                        in_multispace = false;
                    }
                    if c == TAB && (!list || (*curwin).w_p_lcs_chars.tab1 != 0) {
                        n_extra =
                            tabstop_padding(col, (*curbuf).b_p_ts, &(*curbuf).b_p_vts_array) - 1;
                        if !list {
                            sc = schar_from_ascii(b' ');
                            sc_extra = schar_from_ascii(b' ');
                        } else {
                            let mut lcs_tab1 = (*curwin).w_p_lcs_chars.tab1;
                            let mut lcs_tab2 = (*curwin).w_p_lcs_chars.tab2;
                            let mut lcs_tab3 = (*curwin).w_p_lcs_chars.tab3;
                            if !lead.is_null() && s <= lead && (*curwin).w_p_lcs_chars.leadtab1 != 0
                            {
                                lcs_tab1 = (*curwin).w_p_lcs_chars.leadtab1;
                                lcs_tab2 = (*curwin).w_p_lcs_chars.leadtab2;
                                lcs_tab3 = (*curwin).w_p_lcs_chars.leadtab3;
                            }
                            sc = if n_extra == 0 && lcs_tab3 != 0 {
                                lcs_tab3
                            } else {
                                lcs_tab1
                            };
                            sc_extra = lcs_tab2;
                            sc_final = lcs_tab3;
                            hl_id = HLF_0;
                        }
                    } else if c == NUL as c_int && list && (*curwin).w_p_lcs_chars.eol != 0 {
                        p_extra = c"".as_ptr();
                        n_extra = 1;
                        sc = (*curwin).w_p_lcs_chars.eol;
                        hl_id = HLF_AT;
                        s = s.sub(1);
                    } else if c != NUL as c_int && {
                        let n = ascii2cells(c);
                        if n > 1 {
                            n_extra = n - 1;
                            p_extra = transchar_byte_buf(ptr::null(), c_int::from(c as u8));
                            sc = schar_from_ascii(*p_extra as u8);
                            p_extra = p_extra.add(1);
                            hl_id = HLF_0;
                            true
                        } else {
                            sc = 0;
                            false
                        }
                    } {
                        // handled in condition
                    } else if c == b' ' as c_int {
                        if !lead.is_null()
                            && s <= lead
                            && in_multispace
                            && !(*curwin).w_p_lcs_chars.leadmultispace.is_empty()
                        {
                            sc = (*curwin).w_p_lcs_chars.leadmultispace.as_slice()[multispace_pos];
                            multispace_pos += 1;
                            if multispace_pos >= (*curwin).w_p_lcs_chars.leadmultispace.len() {
                                multispace_pos = 0;
                            }
                            hl_id = HLF_0;
                        } else if !lead.is_null() && s <= lead && (*curwin).w_p_lcs_chars.lead != 0
                        {
                            sc = (*curwin).w_p_lcs_chars.lead;
                            hl_id = HLF_0;
                        } else if !trail.is_null() && s > trail {
                            sc = (*curwin).w_p_lcs_chars.trail;
                            hl_id = HLF_0;
                        } else if in_multispace && !(*curwin).w_p_lcs_chars.multispace.is_empty() {
                            sc = (*curwin).w_p_lcs_chars.multispace.as_slice()[multispace_pos];
                            multispace_pos += 1;
                            if multispace_pos >= (*curwin).w_p_lcs_chars.multispace.len() {
                                multispace_pos = 0;
                            }
                            hl_id = HLF_0;
                        } else if list && (*curwin).w_p_lcs_chars.space != 0 {
                            sc = (*curwin).w_p_lcs_chars.space;
                            hl_id = HLF_0;
                        } else {
                            sc = schar_from_ascii(b' ');
                        }
                    } else {
                        sc = schar_from_ascii(c as u8);
                    }
                }
            }

            if sc == 0 {
                break;
            }

            let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
            schar_get(buf.as_mut_ptr(), sc);
            msg_puts_hl(buf.as_ptr(), hl_id, false);
            col += 1;
        }
        msg_clr_eos();
    }
}

/// Output a string to the screen at position msg_row, msg_col.
/// Update msg_row and msg_col for the next message.
pub unsafe fn msg_puts(s: *const c_char) {
    unsafe {
        msg_puts_hl(s, 0, false);
    }
}

pub unsafe fn msg_puts_title(mut s: *const c_char) {
    unsafe {
        if ui_has(kUIMessages) && *s == NL as c_char {
            s = s.add(1);
        }
        msg_puts_hl(s, HLF_T, false);
    }
}

/// Show a message in such a way that it always fits in the line.  Cut out a
/// part in the middle and replace it with "..." when necessary.
/// Does not handle multi-byte characters!
pub unsafe fn msg_outtrans_long(longstr: *const c_char, hl_id: c_int) {
    unsafe {
        let len = CStr::from_ptr(longstr).to_bytes().len() as c_int;
        let mut slen = len;
        let room = Columns - msg_col;
        if !ui_has(kUIMessages) && len > room && room >= 20 {
            slen = (room - 3) / 2;
            msg_outtrans_len(longstr, slen, hl_id, false);
            msg_puts_hl(c"...".as_ptr(), HLF_8, false);
        }
        msg_outtrans_len(longstr.add((len - slen) as usize), slen, hl_id, false);
    }
}

/// Basic function for writing a message with highlight id.
pub unsafe fn msg_puts_hl(s: *const c_char, hl_id: c_int, hist: bool) {
    unsafe {
        msg_puts_len(s, -1, hl_id, hist);
    }
}

/// Write a message with highlight id.
pub unsafe fn msg_puts_len(str_: *const c_char, len: isize, hl_id: c_int, hist: bool) {
    unsafe {
        assert!(
            len < 0 || !std::slice::from_raw_parts(str_.cast::<u8>(), len as usize).contains(&0)
        );
        // If redirection is on, also write to the redirection file.
        redir_write(str_, len);

        // Don't print anything when using ":silent cmd" or empty message.
        if msg_silent != 0 || *str_ == NUL {
            if *str_ == NUL && ui_has(kUIMessages) {
                msg_ext_no_fast();
                ui_call_msg_show(
                    cstr_as_string(c"empty".as_ptr()),
                    Array::new(),
                    false,
                    false,
                    false,
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData { integer: -1 },
                    },
                    NvimString {
                        data: ptr::null_mut(),
                        size: 0,
                    },
                );
                cmdline_was_last_drawn = false;
            }
            return;
        }

        if hist {
            msg_hist_add(str_, len as c_int, hl_id);
        }

        let overflow = !ui_has(kUIMessages) && msg_scrolled > if p_ch == 0 { 1 } else { 0 };

        if overflow && !msg_scrolled_ign && CStr::from_ptr(str_) != c"\r" {
            need_wait_return = true;
        }
        msg_didany = true;

        if msg_use_printf() != 0 && !msg_special_case_for_lua_print {
            let saved_msg_col = msg_col;
            msg_puts_printf(str_, len);
            if headless_mode {
                msg_col = saved_msg_col;
            }
        }
        if msg_use_printf() == 0
            || (headless_mode && crate::grid::grid_allocated(&raw const default_grid))
        {
            msg_puts_display(str_, len as c_int, hl_id, 0);
        }

        need_fileinfo = false;
    }
}

unsafe fn msg_ext_emit_chunk() {
    unsafe {
        if msg_ext_last_attr == -1 {
            return;
        }
        let mut chunk = Array::new();
        chunk.push(Object {
            obj_type: ObjectType::kObjectTypeInteger,
            data: ObjectData {
                integer: msg_ext_last_attr as Integer,
            },
        });
        msg_ext_last_attr = -1;
        let last_chunk = &raw mut msg_ext_last_chunk;
        let text = NvimString {
            data: xmemdupz((*last_chunk).as_ptr().cast(), (*last_chunk).len()).cast(),
            size: (*last_chunk).len(),
        };
        *last_chunk = Vec::new();
        chunk.push(Object {
            obj_type: ObjectType::kObjectTypeString,
            data: ObjectData { string: text },
        });
        chunk.push(Object {
            obj_type: ObjectType::kObjectTypeInteger,
            data: ObjectData {
                integer: msg_ext_last_hl_id as Integer,
            },
        });
        let chunks = &raw mut msg_ext_chunks;
        (*chunks).push(Object {
            obj_type: ObjectType::kObjectTypeArray,
            data: ObjectData { array: chunk },
        });
    }
}

/// The display part of msg_puts_len().
/// May be called recursively to display scroll-back text.
unsafe fn msg_puts_display(str_: *const c_char, maxlen: c_int, hl_id: c_int, recurse: c_int) {
    unsafe {
        let mut s = str_;
        let mut sb_str = str_;
        let mut sb_col = msg_col;
        let attr = if hl_id != 0 { syn_id2attr(hl_id) } else { 0 };

        did_wait_return = false;

        if ui_has(kUIMessages) {
            if attr != msg_ext_last_attr {
                msg_ext_emit_chunk();
                msg_ext_last_attr = attr;
                msg_ext_last_hl_id = hl_id;
            }
            let len = if maxlen < 0 {
                CStr::from_ptr(str_).to_bytes().len()
            } else {
                xstrnlen(str_, maxlen as usize)
            };
            if len > 0 {
                let last_chunk = &raw mut msg_ext_last_chunk;
                (*last_chunk).extend_from_slice(std::slice::from_raw_parts(str_, len));
            }

            let lastline = xmemrchr(str_.cast(), b'\n', len).cast::<c_char>();
            let p = if !lastline.is_null() {
                lastline.add(1)
            } else {
                str_
            };
            let col = mb_string2cells_len(p, len - (p.offset_from(str_) as usize)) as c_int;
            msg_col = (if !lastline.is_null() { 0 } else { msg_col }) + col;

            return;
        }

        let print_attr = hl_combine_attr(hl_attr(HLF_MSG), attr);
        msg_grid_validate();

        cmdline_was_last_drawn = redrawing_cmdline;

        let mut msg_row_pending = -1;

        loop {
            if msg_col >= Columns {
                if p_more != 0 && recurse == 0 {
                    store_sb_text(&raw mut sb_str, s, hl_id, &raw mut sb_col, 1);
                }
                if msg_no_more && lines_left == 0 {
                    break;
                }

                msg_col = 0;
                msg_row += 1;
                msg_didout = false;
            }

            if msg_row >= Rows {
                msg_row = Rows - 1;

                if msg_no_more && lines_left == 0 {
                    break;
                }

                if recurse == 0 {
                    if msg_row_pending >= 0 {
                        msg_line_flush();
                        msg_row_pending = -1;
                    }

                    msg_scroll_up(true, false);

                    inc_msg_scrolled();
                    need_wait_return = true;
                    redraw_cmdline = true;
                    if cmdline_row > 0 {
                        cmdline_row -= 1;
                    }

                    if lines_left > 0 {
                        lines_left -= 1;
                    }

                    if p_more != 0 && lines_left == 0 && State != MODE_HITRETURN && !msg_no_more {
                        if do_more_prompt(NUL as c_int) {
                            s = (&raw const confirm_buttons).as_ref().unwrap().as_ptr();
                        }
                        if quit_more {
                            return;
                        }
                    }
                }
            }

            if !((maxlen < 0 || (s.offset_from(str_) as c_int) < maxlen) && *s != NUL) {
                break;
            }

            if msg_row != msg_row_pending && (*s as u8 >= 0x20 || *s == TAB as c_char) {
                if msg_row_pending >= 0 {
                    msg_line_flush();
                }
                grid_line_start(&raw mut msg_grid_adj, msg_row);
                msg_row_pending = msg_row;
            }

            if *s as u8 >= 0x20 {
                let mut cw = utf_ptr2cells(s);
                let l = if maxlen >= 0 {
                    utfc_ptr2len_len(s, str_.add(maxlen as usize).offset_from(s) as c_int)
                } else {
                    utfc_ptr2len(s)
                };

                if cw > 1 && msg_col == Columns - 1 {
                    grid_line_puts(msg_col, c">".as_ptr(), 1, hl_attr(HLF_AT));
                    cw = 1;
                } else {
                    grid_line_puts(msg_col, s, l, print_attr);
                    s = s.add(l as usize);
                }
                msg_didout = true;
                msg_col += cw;
            } else {
                let c = *s;
                s = s.add(1);
                if c == NL as c_char {
                    msg_didout = false;
                    msg_col = 0;
                    msg_row += 1;
                    if p_more != 0 && recurse == 0 {
                        store_sb_text(&raw mut sb_str, s, hl_id, &raw mut sb_col, 1);
                    }
                } else if c == CAR as c_char {
                    msg_col = 0;
                } else if c == BS as c_char {
                    if msg_col != 0 {
                        msg_col -= 1;
                    }
                } else if c == TAB as c_char {
                    loop {
                        grid_line_puts(msg_col, c" ".as_ptr(), 1, print_attr);
                        msg_col += 1;

                        if msg_col == Columns || (msg_col & 7) == 0 {
                            break;
                        }
                    }
                } else if c == BELL as c_char {
                    vim_beep(kOptBoFlagShell);
                }
            }
        }

        if msg_row_pending >= 0 {
            msg_line_flush();
        }
        msg_cursor_goto(msg_row, msg_col);

        if p_more != 0 && recurse == 0 {
            store_sb_text(&raw mut sb_str, s, hl_id, &raw mut sb_col, 0);
        }

        msg_check();
    }
}

pub unsafe fn msg_line_flush() {
    unsafe {
        if cmdmsg_rl {
            grid_line_mirror(msg_grid.cols);
        }
        grid_line_flush_if_valid_row();
    }
}

pub unsafe fn msg_cursor_goto(mut row: c_int, mut col: c_int) {
    unsafe {
        if cmdmsg_rl {
            col = Columns - 1 - col;
        }
        let grid = grid_adjust(&raw mut msg_grid_adj, &raw mut row, &raw mut col);
        ui_grid_cursor_goto((*grid).handle, row, col);
    }
}

/// @return  true when ":filter pattern" was used and "msg" does not match
///          "pattern".
pub unsafe fn message_filtered(msg: *const c_char) -> bool {
    unsafe {
        if cmdmod.cmod_filter_regmatch.regprog.is_null() {
            return false;
        }

        let match_ = vim_regexec((&raw mut cmdmod.cmod_filter_regmatch).cast(), msg, 0);
        if cmdmod.cmod_filter_force {
            match_
        } else {
            !match_
        }
    }
}

/// including horizontal separator
pub unsafe fn msg_scrollsize() -> c_int {
    unsafe { msg_scrolled + p_ch as c_int + c_int::from(p_ch > 0 || msg_scrolled > 1) }
}

pub unsafe fn msg_do_throttle() -> bool {
    unsafe { msg_use_grid() && (rdb_flags & kOptRdbFlagNothrottle) == 0 }
}

/// Scroll the screen up one line for displaying the next message line.
pub unsafe fn msg_scroll_up(may_throttle: bool, zerocmd: bool) {
    unsafe {
        if may_throttle && msg_do_throttle() {
            msg_grid.throttled = true;
        }
        msg_did_scroll = true;
        if msg_grid_pos > 0 {
            msg_grid_set_pos(msg_grid_pos - 1, !zerocmd);

            if zerocmd && crate::grid::grid_allocated(&raw const msg_grid) {
                grid_clear_line(
                    &raw mut msg_grid,
                    msg_grid.line_offset[0],
                    msg_grid.cols,
                    false,
                );
            }
        } else {
            grid_del_lines(&raw mut msg_grid, 0, 1, msg_grid.rows, 0, msg_grid.cols);
            for i in 0..(msg_grid.rows - 1) as usize {
                msg_grid.dirty_col[i] = msg_grid.dirty_col[i + 1];
            }
            msg_grid.dirty_col[(msg_grid.rows - 1) as usize] = 0;
        }

        grid_clear(
            &raw mut msg_grid_adj,
            Rows - 1,
            Rows,
            0,
            Columns,
            hl_attr(HLF_MSG),
        );
    }
}

/// Send throttled message output to UI clients
pub unsafe fn msg_scroll_flush() {
    unsafe {
        if msg_grid.throttled {
            msg_grid.throttled = false;
            let pos_delta = msg_grid_pos_at_flush - msg_grid_pos;
            assert!(pos_delta >= 0);
            let delta = (msg_scrolled - msg_scrolled_at_flush).min(msg_grid.rows);

            if pos_delta > 0 {
                ui_ext_msg_set_pos(msg_grid_pos, true);
            }

            let to_scroll = delta - pos_delta - msg_grid_scroll_discount;
            assert!(to_scroll >= 0);

            if to_scroll > 0 && msg_grid_pos == 0 {
                ui_call_grid_scroll(
                    msg_grid.handle as Integer,
                    0,
                    Rows as Integer,
                    0,
                    Columns as Integer,
                    to_scroll as Integer,
                    0,
                );
            }

            for i in (Rows - delta.max(1)).max(0)..Rows {
                let row = i - msg_grid_pos;
                assert!(row >= 0);
                ui_line(
                    &raw mut msg_grid,
                    row,
                    false,
                    0,
                    msg_grid.dirty_col[row as usize],
                    msg_grid.cols,
                    hl_attr(HLF_MSG),
                    false,
                );
                msg_grid.dirty_col[row as usize] = 0;
            }
        }
        msg_scrolled_at_flush = msg_scrolled;
        msg_grid_scroll_discount = 0;
        msg_grid_pos_at_flush = msg_grid_pos;
    }
}

pub unsafe fn msg_reset_scroll() {
    unsafe {
        if ui_has(kUIMessages) {
            return;
        }
        msg_grid.throttled = false;
        msg_grid_set_pos(Rows - p_ch as c_int, false);
        clear_cmdline = true;
        if crate::grid::grid_allocated(&raw const msg_grid) {
            let limit = msg_scrollsize().min(msg_grid.rows);
            for i in 0..limit {
                grid_clear_line(
                    &raw mut msg_grid,
                    msg_grid.line_offset[i as usize],
                    msg_grid.cols,
                    false,
                );
            }
        }
        msg_scrolled = 0;
        msg_scrolled_at_flush = 0;
        msg_grid_scroll_discount = 0;
    }
}

pub unsafe fn msg_ui_refresh() {
    unsafe {
        if ui_has(kUIMultigrid) && crate::grid::grid_allocated(&raw const msg_grid) {
            ui_call_grid_resize(
                msg_grid.handle as Integer,
                msg_grid.cols as Integer,
                msg_grid.rows as Integer,
            );
            ui_ext_msg_set_pos(msg_grid_pos, msg_scrolled != 0);
        }
    }
}

pub unsafe fn msg_ui_flush() {
    unsafe {
        if ui_has(kUIMultigrid)
            && crate::grid::grid_allocated(&raw const msg_grid)
            && msg_grid.pending_comp_index_update
        {
            ui_ext_msg_set_pos(msg_grid_pos, msg_scrolled != 0);
        }
    }
}

/// Increment "msg_scrolled".
unsafe fn inc_msg_scrolled() {
    unsafe {
        if *get_vim_var_str(VV_SCROLLSTART) == NUL {
            let sname = sourcing_name();
            let mut tofree_vec: Vec<c_char>;

            let p = if sname.is_null() {
                cstr_as_string(gettext(c"Unknown".as_ptr()))
            } else {
                let tofreesize = CStr::from_ptr(sname).to_bytes().len() + 40;
                tofree_vec = vec![0; tofreesize];
                let tofree = tofree_vec.as_mut_ptr();
                let size = vim_snprintf_safelen!(
                    tofree,
                    tofreesize,
                    gettext(c"%s line %lld".as_ptr()),
                    sname,
                    i64::from(sourcing_lnum()),
                );
                NvimString { data: tofree, size }
            };
            set_vim_var_string(VV_SCROLLSTART, p.data, p.size as isize);
        }
        msg_scrolled += 1;
        set_must_redraw(UPD_VALID);
    }
}

/// Store part of a printed message for displaying when scrolling back.
unsafe fn store_sb_text(
    sb_str: *mut *const c_char,
    s: *const c_char,
    hl_id: c_int,
    sb_col: *mut c_int,
    finish: c_int,
) {
    unsafe {
        if do_clear_sb_text == sb_clear_T::SB_CLEAR_ALL
            || do_clear_sb_text == sb_clear_T::SB_CLEAR_CMDLINE_DONE
        {
            clear_sb_text(do_clear_sb_text == sb_clear_T::SB_CLEAR_ALL);
            msg_sb_eol();
            if do_clear_sb_text == sb_clear_T::SB_CLEAR_CMDLINE_DONE
                && s > *sb_str
                && **sb_str == NL as c_char
            {
                *sb_str = (*sb_str).add(1);
            }
            do_clear_sb_text = sb_clear_T::SB_CLEAR_NONE;
        }

        let chunks = &raw mut msgchunks;
        if s > *sb_str {
            let text_len = s.offset_from(*sb_str) as usize;
            let mut text = Vec::with_capacity(text_len + 1);
            text.extend_from_slice(std::slice::from_raw_parts(*sb_str, text_len));
            text.push(NUL);
            (*chunks).push(msgchunk_T {
                sb_eol: finish as c_char,
                sb_msg_col: *sb_col,
                sb_hl_id: hl_id,
                sb_text: text,
            });
        } else if finish != 0
            && let Some(last) = (*chunks).last_mut()
        {
            last.sb_eol = 1;
        }

        *sb_str = s;
        *sb_col = 0;
    }
}

/// Finished showing messages, clear the scroll-back text on the next message.
pub unsafe fn may_clear_sb_text() {
    unsafe {
        msg_ext_ui_flush();
        do_clear_sb_text = sb_clear_T::SB_CLEAR_ALL;
        do_clear_hist_temp = true;
    }
}

/// Starting to edit the command line: do not clear messages now.
pub unsafe fn sb_text_start_cmdline() {
    unsafe {
        if do_clear_sb_text == sb_clear_T::SB_CLEAR_CMDLINE_BUSY {
            sb_text_restart_cmdline();
        } else {
            msg_sb_eol();
            do_clear_sb_text = sb_clear_T::SB_CLEAR_CMDLINE_BUSY;
        }
    }
}

/// Redrawing the command line: clear the last unfinished line.
pub unsafe fn sb_text_restart_cmdline() {
    unsafe {
        do_clear_sb_text = sb_clear_T::SB_CLEAR_CMDLINE_BUSY;

        let chunks = &raw mut msgchunks;
        let Some(last) = (*chunks).last() else {
            return;
        };
        if last.sb_eol != 0 {
            return;
        }

        if let Some(start_idx) = msg_sb_start(Some((*chunks).len() - 1)) {
            (*chunks).truncate(start_idx);
        }
    }
}

/// Ending to edit the command line: clear old lines but the last one later.
pub unsafe fn sb_text_end_cmdline() {
    unsafe {
        do_clear_sb_text = sb_clear_T::SB_CLEAR_CMDLINE_DONE;
    }
}

/// Clear any text remembered for scrolling back.
pub unsafe fn clear_sb_text(all: bool) {
    unsafe {
        let chunks = &raw mut msgchunks;
        if all {
            (*chunks).clear();
        } else if !(*chunks).is_empty()
            && let Some(start_idx) = msg_sb_start(Some((*chunks).len() - 1))
        {
            (*chunks).drain(0..start_idx);
        }
    }
}

/// "g<" command.
pub unsafe fn show_sb_text() {
    unsafe {
        if ui_has(kUIMessages) {
            let mut ea: exarg_T = exarg_T::new();
            ea.arg = c"".as_ptr().cast_mut();
            ea.skip = 1;
            ex_messages(&raw mut ea);
            return;
        }
        let chunks = &raw const msgchunks;
        let mp = msg_sb_start((*chunks).len().checked_sub(1));
        if mp.is_none_or(|idx| idx == 0) {
            vim_beep(kOptBoFlagMess);
        } else {
            do_more_prompt(b'G' as c_int);
            wait_return(0);
        }
    }
}

/// Move to the start of screen line in already displayed text.
unsafe fn msg_sb_start(mps: Option<usize>) -> Option<usize> {
    unsafe {
        let chunks = &raw const msgchunks;
        let mut idx = mps?;
        while idx > 0 && (*chunks).as_slice()[idx - 1].sb_eol == 0 {
            idx -= 1;
        }
        Some(idx)
    }
}

/// Mark the last message chunk as finishing the line.
pub unsafe fn msg_sb_eol() {
    unsafe {
        let chunks = &raw mut msgchunks;
        if let Some(last) = (*chunks).last_mut() {
            last.sb_eol = 1;
        }
    }
}

/// Display a screen line from previously displayed text at row "row".
unsafe fn disp_sb_line(row: c_int, smp: usize) -> Option<usize> {
    unsafe {
        let chunks = &raw mut msgchunks;
        let mut idx = smp;
        loop {
            msg_row = row;
            let chunk = &mut (*chunks).as_mut_slice()[idx];
            msg_col = chunk.sb_msg_col;
            let p = chunk.sb_text.as_mut_ptr();
            let hl_id = chunk.sb_hl_id;
            let eol = chunk.sb_eol;
            msg_puts_display(p, -1, hl_id, 1);
            if eol != 0 || idx + 1 >= (*chunks).len() {
                break;
            }
            idx += 1;
        }
        if idx + 1 < (*chunks).len() {
            Some(idx + 1)
        } else {
            None
        }
    }
}

/// @return  true when messages should be printed to stdout/stderr
pub unsafe fn msg_use_printf() -> c_int {
    unsafe { c_int::from(!embedded_mode && ui_active() == 0 && !ui_has(kUIMessages)) }
}

/// Print a message when there is no valid screen.
unsafe fn msg_puts_printf(str_: *const c_char, maxlen: isize) {
    unsafe {
        let mut s = str_;
        let mut buf = [0 as c_char; 7];

        if on_print.cb_type() != kCallbackNone {
            let mut argv = [typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: str_.cast_mut(),
                },
            }];
            let mut rettv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };
            callback_call(&raw mut on_print, 1, argv.as_mut_ptr(), &raw mut rettv);
            tv_clear(&raw mut rettv);
            return;
        }

        while (maxlen < 0 || s.offset_from(str_) < maxlen) && *s != NUL {
            let len = utf_ptr2len(s) as usize;
            if !(silent_mode && p_verbose == 0) {
                let mut p = buf.as_mut_ptr();
                if *s == NL as c_char && !info_message && !silent_mode && !headless_mode {
                    *p = CAR as c_char;
                    p = p.add(1);
                }
                ptr::copy_nonoverlapping(s, p, len);
                *p.add(len) = NUL;
                if info_message {
                    printf!(c"%s".as_ptr(), buf.as_ptr());
                } else {
                    fprintf!(log_get_stderr(), c"%s".as_ptr(), buf.as_ptr());
                }
            }

            let cw = utf_char2cells(utf_ptr2char(s));
            if *s == CAR as c_char || *s == NL as c_char {
                msg_col = 0;
                msg_didout = false;
            } else {
                msg_col += cw;
                msg_didout = true;
            }
            s = s.add(len);
        }
    }
}

/// Show the more-prompt and handle the user response.
unsafe fn do_more_prompt(typed_char: c_int) -> bool {
    unsafe {
        let mut used_typed_char = typed_char;
        let oldState = State;
        let mut retval = false;
        let mut to_redraw = false;
        let mut mp_last: Option<usize> = None;

        let no_need_more =
            (headless_mode && !embedded_mode && ui_active() == 0) || input_unanswerable();

        if no_need_more || DO_MORE_PROMPT_ENTERED || (State == MODE_HITRETURN && typed_char == 0) {
            return false;
        }
        DO_MORE_PROMPT_ENTERED = true;

        let chunks = &raw const msgchunks;
        if typed_char == b'G' as c_int {
            mp_last = msg_sb_start((*chunks).len().checked_sub(1));
            let mut i = 0;
            while i < Rows - 2
                && let Some(idx) = mp_last
                && idx > 0
            {
                mp_last = msg_sb_start(Some(idx - 1));
                i += 1;
            }
        }

        State = MODE_ASKMORE;
        setmouse();
        if typed_char == NUL as c_int {
            msg_moremsg(false);
        }
        loop {
            let c = if used_typed_char != NUL as c_int {
                let ch = used_typed_char;
                used_typed_char = NUL as c_int;
                ch
            } else {
                get_keystroke(resize_events)
            };

            let mut toscroll = 0;
            match c {
                BS | K_BS | 0x6b /* 'k' */ | K_UP => {
                    toscroll = -1;
                }
                CAR | NL | 0x6a /* 'j' */ | K_DOWN => {
                    toscroll = 1;
                }
                0x75 /* 'u' */ => {
                    toscroll = -(Rows / 2);
                }
                0x64 /* 'd' */ => {
                    toscroll = Rows / 2;
                }
                0x62 /* 'b' */ | Ctrl_B | K_PAGEUP => {
                    toscroll = -(Rows - 1);
                }
                0x20 /* ' ' */ | 0x66 /* 'f' */ | Ctrl_F | K_PAGEDOWN | K_LEFTMOUSE => {
                    toscroll = Rows - 1;
                }
                0x67 /* 'g' */ => {
                    toscroll = -999999;
                }
                0x47 /* 'G' */ => {
                    toscroll = 999999;
                    lines_left = 999999;
                }
                0x3a /* ':' */ => {
                    if confirm_msg_used == 0 {
                        typeahead_noflush(b':' as c_int);
                        cmdline_row = Rows - 1;
                        skip_redraw = true;
                        need_wait_return = false;
                    }
                    if confirm_msg_used != 0 {
                        retval = true;
                    } else {
                        got_int = true;
                        quit_more = true;
                    }
                    lines_left = Rows - 1;
                }
                0x71 /* 'q' */ | Ctrl_C | ESC => {
                    if confirm_msg_used != 0 {
                        retval = true;
                    } else {
                        got_int = true;
                        quit_more = true;
                    }
                    lines_left = Rows - 1;
                }
                K_EVENT => {
                    multiqueue_process_events(resize_events);
                    to_redraw = true;
                }
                _ => {
                    msg_moremsg(true);
                    continue;
                }
            }

            assert!(toscroll == 0 || !to_redraw);

            if toscroll != 0 || to_redraw {
                if toscroll < 0 || to_redraw {
                    let mut mp = if let Some(last_idx) = mp_last {
                        if last_idx > 0 {
                            msg_sb_start(Some(last_idx - 1))
                        } else {
                            None
                        }
                    } else {
                        msg_sb_start((*chunks).len().checked_sub(1))
                    };

                    let mut i = 0;
                    while i < Rows - 2
                        && let Some(idx) = mp
                        && idx > 0
                    {
                        mp = msg_sb_start(Some(idx - 1));
                        i += 1;
                    }

                    if let Some(mp_idx) = mp
                        && (mp_idx > 0 || to_redraw)
                    {
                        let mut i = 0;
                        while i > toscroll {
                            let Some(cur_idx) = mp else {
                                break;
                            };
                            if cur_idx == 0 {
                                break;
                            }
                            mp = msg_sb_start(Some(cur_idx - 1));
                            if let Some(last_idx) = mp_last {
                                mp_last = msg_sb_start(last_idx.checked_sub(1));
                            } else {
                                mp_last = msg_sb_start((*chunks).len().checked_sub(1));
                            }
                            i -= 1;
                        }

                        if toscroll == -1 && !to_redraw {
                            grid_ins_lines(&raw mut msg_grid, 0, 1, Rows, 0, Columns);
                            grid_clear(&raw mut msg_grid_adj, 0, 1, 0, Columns, hl_attr(HLF_MSG));
                            if let Some(cur_idx) = mp {
                                disp_sb_line(0, cur_idx);
                            }
                        } else {
                            grid_clear(
                                &raw mut msg_grid_adj,
                                0,
                                Rows,
                                0,
                                Columns,
                                hl_attr(HLF_MSG),
                            );
                            let mut i = 0;
                            while let Some(cur_idx) = mp
                                && i < Rows - 1
                            {
                                mp = disp_sb_line(i, cur_idx);
                                msg_scrolled += 1;
                                i += 1;
                            }
                            to_redraw = false;
                        }
                        toscroll = 0;
                    }
                } else {
                    if cmdline_row >= Rows && !ui_has(kUIMessages) {
                        msg_scroll_up(true, false);
                        msg_scrolled += 1;
                    }
                    while toscroll > 0
                        && let Some(last_idx) = mp_last
                    {
                        if msg_do_throttle() && !msg_grid.throttled {
                            msg_scrolled_at_flush -= 1;
                            msg_grid_scroll_discount += 1;
                        }
                        msg_scroll_up(true, false);
                        inc_msg_scrolled();
                        grid_clear(
                            &raw mut msg_grid_adj,
                            Rows - 2,
                            Rows - 1,
                            0,
                            Columns,
                            hl_attr(HLF_MSG),
                        );
                        mp_last = disp_sb_line(Rows - 2, last_idx);
                        toscroll -= 1;
                    }
                }

                if toscroll <= 0 {
                    grid_clear(
                        &raw mut msg_grid_adj,
                        Rows - 1,
                        Rows,
                        0,
                        Columns,
                        hl_attr(HLF_MSG),
                    );
                    msg_moremsg(false);
                    continue;
                }

                lines_left = toscroll;
            }

            break;
        }

        // clear the --more-- message
        grid_clear(
            &raw mut msg_grid_adj,
            Rows - 1,
            Rows,
            0,
            Columns,
            hl_attr(HLF_MSG),
        );
        redraw_cmdline = true;
        clear_cmdline = false;
        mode_displayed = false;

        State = oldState;
        setmouse();
        if quit_more {
            msg_row = Rows - 1;
            msg_col = 0;
        }

        DO_MORE_PROMPT_ENTERED = false;
        retval
    }
}

unsafe fn msg_moremsg(full: bool) {
    unsafe {
        let attr = hl_combine_attr(hl_attr(HLF_MSG), hl_attr(HLF_M));
        grid_line_start(&raw mut msg_grid_adj, Rows - 1);
        let mut len = grid_line_puts(0, gettext(c"-- More --".as_ptr()), -1, attr);
        if full {
            len += grid_line_puts(
                len,
                gettext(c" SPACE/d/j: screen/page/line down, b/u/k: up, q: quit ".as_ptr()),
                -1,
                attr,
            );
        }
        grid_line_cursor_goto(len);
        grid_line_flush();
    }
}

/// Repeat the message for the current mode: MODE_ASKMORE, MODE_EXTERNCMD,
/// or confirm() prompt.
pub unsafe fn repeat_message() {
    unsafe {
        if ui_has(kUIMessages) {
            return;
        }

        if State == MODE_ASKMORE {
            msg_moremsg(true);
            msg_row = Rows - 1;
        } else if (State & MODE_CMDLINE) != 0
            && (&raw const confirm_msg).as_ref().unwrap().is_some()
        {
            display_confirm_msg();
            msg_row = Rows - 1;
        } else if State == MODE_EXTERNCMD {
            ui_cursor_goto(msg_row, msg_col);
        } else if State == MODE_HITRETURN || State == MODE_SETWSIZE {
            if msg_row == Rows - 1 {
                msg_didout = false;
                msg_col = 0;
                msg_clr_eos();
            }
            hit_return_msg(false);
            msg_row = Rows - 1;
        }
    }
}

/// Clear from current message position to end of screen.
/// Skip this when ":silent" was used, no need to clear for redirection.
pub unsafe fn msg_clr_eos() {
    unsafe {
        if msg_silent == 0 {
            msg_clr_eos_force();
        }
    }
}

/// Clear from current message position to end of screen.
pub unsafe fn msg_clr_eos_force() {
    unsafe {
        if ui_has(kUIMessages) {
            return;
        }
        let msg_startcol = if cmdmsg_rl { 0 } else { msg_col };
        let msg_endcol = if cmdmsg_rl {
            Columns - msg_col
        } else {
            Columns
        };

        if crate::grid::grid_allocated(&raw const msg_grid) && msg_row < msg_grid_pos {
            msg_grid_validate();
            if msg_row < msg_grid_pos {
                msg_row = msg_grid_pos;
            }
        }

        grid_clear(
            &raw mut msg_grid_adj,
            msg_row,
            msg_row + 1,
            msg_startcol,
            msg_endcol,
            hl_attr(HLF_MSG),
        );
        grid_clear(
            &raw mut msg_grid_adj,
            msg_row + 1,
            Rows,
            0,
            Columns,
            hl_attr(HLF_MSG),
        );

        redraw_cmdline = true;
        if msg_row < Rows - 1 || msg_col == 0 {
            clear_cmdline = false;
            mode_displayed = false;
            cmdline_was_last_drawn = false;
        }
    }
}

/// Clear the command line.
pub unsafe fn msg_clr_cmdline() {
    unsafe {
        msg_row = cmdline_row;
        msg_col = 0;
        msg_clr_eos_force();
    }
}

/// end putting a message on the screen
/// call wait_return() if the message does not fit in the available space
///
/// @return  true if wait_return() not called.
pub unsafe fn msg_end() -> bool {
    unsafe {
        if !exiting && need_wait_return && (State & MODE_CMDLINE) == 0 {
            wait_return(0);
            return false;
        }

        msg_ext_ui_flush();
        true
    }
}

/// Clear "msg_ext_chunks" before flushing so that ui_flush() does not re-emit
/// the same message recursively.
unsafe fn msg_ext_init_chunks() -> Array {
    unsafe {
        let chunks = &raw mut msg_ext_chunks;
        let tofree = std::mem::take(&mut *chunks);
        msg_col = 0;
        tofree
    }
}

pub unsafe fn msg_ext_ui_flush() {
    unsafe {
        if !ui_has(kUIMessages) {
            msg_ext_kind = ptr::null();
            return;
        } else if msg_ext_skip_flush {
            return;
        }

        msg_ext_emit_chunk();
        let chunks = &raw const msg_ext_chunks;
        if (*chunks).size > 0 {
            let tofree = msg_ext_init_chunks();

            ui_call_msg_show(
                cstr_as_string(msg_ext_kind),
                tofree,
                msg_ext_overwrite,
                msg_ext_history,
                msg_ext_append,
                msg_ext_id,
                cstr_as_string(msg_ext_trigger),
            );
            if msg_ext_history {
                api_free_array(tofree);
            } else {
                // Add to history as temporary message for "g<".
                let mut msg = HlMessage::new();
                for i in 0..tofree.size {
                    let chunk = (*tofree.items.add(i)).data.array.items;
                    msg.push(HlMessageChunk {
                        text: (*chunk.add(1)).data.string,
                        hl_id: (*chunk.add(2)).data.integer as c_int,
                    });
                    xfree(chunk.cast());
                }
                xfree(tofree.items.cast());
                msg_hist_add_multihl(msg, true);
            }
            msg_ext_overwrite = false;
            msg_ext_history = false;
            msg_ext_append = false;
            msg_ext_fast = true;
            msg_ext_kind = ptr::null();
            if msg_ext_id.obj_type == ObjectType::kObjectTypeInteger
                && msg_ext_id.data.integer == msg_id_next
            {
                msg_id_next += 1;
            }
            msg_ext_id = Object {
                obj_type: ObjectType::kObjectTypeInteger,
                data: ObjectData {
                    integer: msg_id_next,
                },
            };
        }
    }
}

pub unsafe fn msg_ext_flush_showmode() {
    unsafe {
        if ui_has(kUIMessages) && (msg_ext_last_attr != -1 || SHOWMODE_CLEAR) {
            SHOWMODE_CLEAR = msg_ext_last_attr != -1;
            msg_ext_emit_chunk();
            let tofree = msg_ext_init_chunks();
            ui_call_msg_showmode(tofree);
            api_free_array(tofree);
        }
    }
}

/// If the written message runs into the shown command or ruler, we have to
/// wait for hit-return and redraw the window later.
pub unsafe fn msg_check() {
    unsafe {
        if ui_has(kUIMessages) {
            return;
        }
        if msg_row == Rows - 1 && msg_col >= sc_col {
            need_wait_return = true;
            redraw_cmdline = true;
        }
    }
}

/// May write a string to the redirection file.
unsafe fn redir_write(str_: *const c_char, maxlen: isize) {
    unsafe {
        let mut s = str_;

        if maxlen == 0 {
            return;
        }

        if redir_off {
            return;
        }

        if *p_vfile != NUL && verbose_fd.is_null() {
            verbose_open();
        }

        if redirecting() != 0 {
            if *s != NL as c_char && *s != CAR as c_char {
                while redir_col < msg_col {
                    if !capture_ga.is_null() {
                        (*capture_ga).push(b' ' as c_char);
                    }
                    if redir_reg != 0 {
                        write_reg_contents(redir_reg, c" ".as_ptr(), 1, 1);
                    } else if redir_vname {
                        var_redir_str(c" ".as_ptr(), -1);
                    } else if !redir_fd.is_null() {
                        fputs(c" ".as_ptr(), redir_fd);
                    }
                    if !verbose_fd.is_null() {
                        fputs(c" ".as_ptr(), verbose_fd);
                    }
                    redir_col += 1;
                }
            }

            let len = if maxlen == -1 {
                CStr::from_ptr(s).to_bytes().len()
            } else {
                maxlen as usize
            };
            if !capture_ga.is_null() && len > 0 {
                (*capture_ga).extend_from_slice(std::slice::from_raw_parts(str_, len));
            }
            if redir_reg != 0 {
                write_reg_contents(redir_reg, s, len as isize, 1);
            }
            if redir_vname {
                var_redir_str(s, maxlen as c_int);
            }

            while (maxlen < 0 || (s.offset_from(str_) as c_int) < maxlen as c_int) && *s != NUL {
                if redir_reg == 0 && !redir_vname && capture_ga.is_null() && !redir_fd.is_null() {
                    fputc(c_int::from(*s), redir_fd);
                }
                if !verbose_fd.is_null() {
                    fputc(c_int::from(*s), verbose_fd);
                }
                if *s == CAR as c_char || *s == NL as c_char {
                    redir_col = 0;
                } else if *s == TAB as c_char {
                    redir_col += 8 - redir_col % 8;
                } else {
                    redir_col += 1;
                }
                s = s.add(1);
            }

            if msg_silent != 0 {
                msg_col = redir_col;
            }
        }
    }
}

pub unsafe fn redirecting() -> c_int {
    unsafe {
        c_int::from(
            !redir_fd.is_null()
                || *p_vfile != NUL
                || redir_reg != 0
                || redir_vname
                || !capture_ga.is_null(),
        )
    }
}

/// Before giving verbose message.
/// Must always be called paired with verbose_leave()!
pub unsafe fn verbose_enter() {
    unsafe {
        if *p_vfile != NUL {
            msg_silent += 1;
        }
        if !msg_ext_skip_verbose {
            if msg_ext_kind != VERBOSE_KIND.as_ptr() {
                pre_verbose_kind = msg_ext_kind;
            }
            msg_ext_set_kind(VERBOSE_KIND.as_ptr());
        }
        msg_ext_skip_verbose = false;
    }
}

/// After giving verbose message.
/// Must always be called paired with verbose_enter()!
pub unsafe fn verbose_leave() {
    unsafe {
        if *p_vfile != NUL {
            msg_silent -= 1;
            if msg_silent < 0 {
                msg_silent = 0;
            }
        }
        if !pre_verbose_kind.is_null() {
            msg_ext_set_kind(pre_verbose_kind);
            pre_verbose_kind = ptr::null();
        }
    }
}

/// Like verbose_enter() and set msg_scroll when displaying the message.
pub unsafe fn verbose_enter_scroll() {
    unsafe {
        verbose_enter();
        if *p_vfile == NUL {
            msg_scroll = 1;
        }
    }
}

/// Like verbose_leave() and set cmdline_row when displaying the message.
pub unsafe fn verbose_leave_scroll() {
    unsafe {
        verbose_leave();
        if *p_vfile == NUL {
            cmdline_row = msg_row;
        }
    }
}

/// Called when 'verbosefile' is set: stop writing to the file.
pub unsafe fn verbose_stop() {
    unsafe {
        if !verbose_fd.is_null() {
            fclose(verbose_fd);
            verbose_fd = ptr::null_mut();
        }
        verbose_did_open = false;
    }
}

/// Open the file 'verbosefile'.
///
/// @return  FAIL or OK.
pub unsafe fn verbose_open() -> c_int {
    unsafe {
        if verbose_fd.is_null() && !verbose_did_open {
            verbose_did_open = true;

            verbose_fd = os_fopen(p_vfile, c"a".as_ptr());
            if verbose_fd.is_null() {
                semsg!(gettext(e_notopen.as_ptr()), p_vfile);
                return FAIL;
            }
        }
        OK
    }
}

/// Give a warning message (for searching).
/// Use 'w' highlighting and may repeat the message after redrawing
pub unsafe fn give_warning(message: *const c_char, hl: bool, hist: bool) {
    unsafe {
        if msg_silent != 0 {
            return;
        }

        let save_msg_hist_off = msg_hist_off;
        msg_hist_off = !hist;

        no_wait_return += 1;

        set_vim_var_string(VV_WARNINGMSG, message, -1);
        keep_msg = None;
        if hl {
            keep_msg_hl_id = HLF_W;
        } else {
            keep_msg_hl_id = 0;
        }

        if msg_ext_kind.is_null() {
            msg_ext_set_kind(c"wmsg".as_ptr());
        }

        if msg(message, keep_msg_hl_id) && msg_scrolled == 0 {
            set_keep_msg(message, keep_msg_hl_id);
        }
        msg_didout = false;
        msg_nowait = true;
        msg_col = 0;

        no_wait_return -= 1;
        msg_hist_off = save_msg_hist_off;
    }
}

/// Advance msg cursor to column "col".
pub unsafe fn msg_advance(mut col: c_int) {
    unsafe {
        if msg_silent != 0 {
            msg_col = col;
            return;
        }
        col = col.min(Columns - 1);
        while msg_col < col {
            msg_putchar(b' ' as c_int);
        }
    }
}

/// Display the ":confirm" message.  Also called from do_dialog() and when screen resized.
pub unsafe fn display_confirm_msg() {
    unsafe {
        confirm_msg_used += 1;
        if let Some(ref msg) = *(&raw const confirm_msg).as_ref().unwrap() {
            msg_ext_set_kind(c"confirm".as_ptr());
            msg_puts_hl(msg.as_ptr(), HLF_M, false);
        }
        confirm_msg_used -= 1;
    }
}

/// Only for legacy UI (`!ui_has(kUIMessages)`): Pause to display a message for `ms` milliseconds.
pub unsafe fn msg_delay(mut ms: u64, ignoreinput: bool) {
    unsafe {
        if ui_has(kUIMessages) {
            return;
        }

        if nvim_testing {
            ms = 100;
        }

        logmsg!(
            LOGLVL_DBG,
            ptr::null(),
            c"msg_delay".as_ptr(),
            line!() as c_int,
            true,
            c"%llu ms%s".as_ptr(),
            ms,
            if nvim_testing {
                c" (skipped by NVIM_TEST)".as_ptr()
            } else {
                c"".as_ptr()
            },
        );
        ui_flush();
        os_delay(ms, ignoreinput);
    }
}

/// Check if there should be a delay to allow the user to see a message.
pub unsafe fn msg_check_for_delay(check_msg_scroll: bool) {
    unsafe {
        if (emsg_on_display || (check_msg_scroll && msg_scroll != 0))
            && !did_wait_return
            && emsg_silent == 0
            && !in_assert_fails
            && !ui_has(kUIMessages)
        {
            msg_delay(1006, true);
            emsg_on_display = false;
            if check_msg_scroll {
                msg_scroll = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trunc_string_in_place_and_by_copy() {
        let cases: &[(&str, &str, c_int)] = &[
            ("text", "text", 20),
            ("a short text", "a short text", 20),
            ("a text that fits", "a text that fits", 34),
            ("a text tha just fits", "a text tha just fits", 20),
            ("a text that nott fits", "a text t...nott fits", 20),
        ];

        for &(s, expected, room) in cases {
            unsafe {
                let mut buf = [0 as c_char; 40];
                let c_s = std::ffi::CString::new(s).unwrap();
                trunc_string(c_s.as_ptr(), buf.as_mut_ptr(), room, 40);
                assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), expected);

                let mut inplace = [0 as c_char; 40];
                ptr::copy_nonoverlapping(c_s.as_ptr(), inplace.as_mut_ptr(), s.len() + 1);
                trunc_string(inplace.as_ptr(), inplace.as_mut_ptr(), room, 40);
                assert_eq!(CStr::from_ptr(inplace.as_ptr()).to_str().unwrap(), expected);
            }
        }
    }
}
