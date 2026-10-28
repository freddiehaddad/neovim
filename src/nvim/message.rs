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
use crate::dialog::{dlg_st, prompt_for_input};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, redraw_all_later, redraw_later, set_must_redraw,
};
use crate::errors::*;
use crate::eval::callback_call;
use crate::eval::typval::{TypVal, TypvalVval, VarType, tv_clear};
use crate::eval::vars::{
    VV_ERRMSG, VV_SCROLLSTART, VV_STATUSMSG, VV_WARNINGMSG, get_vim_var_str, set_vim_var_string,
    var_redir_str,
};
use crate::event::r#loop::loop_schedule_deferred;
use crate::event::multiqueue::{Event, multiqueue_process_events};
use crate::ex_docmd::{Exarg, do_sleep};
use crate::ex_eval::cause_errthrow;
use crate::extmark::{K_FALSE, K_TRUE, TriState};
use crate::fileio::{check_timestamps, fprintf, fputc, fputs, printf};
use crate::globals::*;
use crate::grid::{
    Sattr, Schar, grid_adjust, grid_alloc, grid_assign_handle, grid_clear, grid_clear_line,
    grid_del_lines, grid_free, grid_ins_lines, grid_line_cursor_goto, grid_line_flush,
    grid_line_flush_if_valid_row, grid_line_mirror, grid_line_puts, grid_line_start,
    schar_from_ascii, schar_get,
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
    K_X2MOUSE, KS_MODIFIER, KeyChord, MOD_MASK_SHIFT, NL, TAB, get_special_key,
};
use crate::log::{LOGLVL_DBG, LOGLVL_INF, log_get_stderr, logmsg};
use crate::main_init::input_unanswerable;
use crate::mbyte::{
    UTF8LEN_TAB, mb_string2cells_len, mb_unescape, ptr2cells, utf_char2bytes, utf_char2cells,
    utf_head_off, utf_ptr2cells, utf_ptr2char, utf_ptr2len, utfc_ptr2len, utfc_ptr2len_len,
};
use crate::memory::{Arena, arena_alloc, xstrlcpy, xstrnlen};
use crate::mouse::{MOUSE_SETPOS, jump_to_mouse, setmouse};
use crate::msgpack_rpc::packer::Integer;
use crate::option::shortmess;
use crate::option_vars::*;
use crate::optionstr::{OptKeyDict_mopt, Optset, opt_keyset};
use crate::os::env::home_replace_save;
use crate::os::fs::os_fopen;
use crate::os::input::os_breakcheck;
use crate::os::time::os_delay;
use crate::regexp::vim_regexec;
use crate::register::write_reg_contents;
use crate::runtime::{ESTACK_NONE, estack_sfile, rt_st, sourcing_lnum, sourcing_name};
use crate::state::{MODE_ASKMORE, MODE_CMDLINE, MODE_EXTERNCMD, MODE_HITRETURN, MODE_SETWSIZE};
use crate::strings::{gettext, ngettext, vim_snprintf, vim_snprintf_safelen};
use crate::ui::{
    ui_active, ui_call_grid_destroy, ui_call_grid_resize, ui_call_grid_scroll,
    ui_call_msg_history_show, ui_call_msg_set_pos, ui_call_msg_show, ui_call_msg_showmode,
    ui_cursor_goto, ui_flush, ui_grid_cursor_goto, ui_has, ui_line, ui_refresh, vim_beep,
};
use crate::ui_compositor::{ui_comp_put_grid, ui_comp_remove_grid};
use std::ffi::{CStr, c_char};
use std::ptr;

pub type MsgID = Object;

const BELL: i32 = 0x07;

pub(crate) const K_ZINDEX_MESSAGES: i32 = 200;

pub(crate) const HLF_M: i32 = 10;
pub(crate) const HLF_MSG: i32 = 63;

const K_OPT_BO_FLAG_MESS: u32 = 0x1000;
const K_OPT_BO_FLAG_SHELL: u32 = 0x10000;
const K_OPT_RDB_FLAG_NOTHROTTLE: u32 = 0x02;
const K_SHM_TRUNC: i32 = b't' as i32;
const K_SHM_TRUNCALL: i32 = b'T' as i32;

pub(crate) const K_OPT_MESSAGESOPT: i32 = 190;
const KEYSET_OPTIDX_MOPT_WAIT: u32 = 1;
const KEYSET_OPTIDX_MOPT_HISTORY: u32 = 3;

const PROGRESS_TARGET_CMD: i32 = 0x01;

const VERBOSE_KIND: &CStr = c"verbose";

pub(crate) const NIL_OBJ: Object = Object {
    obj_type: ObjectType::ObjectTypeNil,
    data: ObjectData::Integer(0),
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
struct Msgchunk {
    sb_eol: c_char,
    sb_msg_col: i32,
    sb_hl_id: i32,
    sb_text: Vec<c_char>,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SbClear {
    SbClearNone = 0,
    SbClearAll = 1,
    SbClearCmdlineBusy = 2,
    SbClearCmdlineDone = 3,
}

struct MessageState {
    confirm_msg_used: i32,
    msg_hist: std::collections::VecDeque<MessageHistoryEntry>,
    msg_hist_temp_idx: Option<usize>,
    msg_hist_len: i32,
    msg_hist_max: i32,
    msg_hit_enter: bool,
    msg_wait: i32,
    progress_msg_target: i32,
    verbose_fd: Option<crate::os::fs::CFile>,
    verbose_did_open: bool,
    keep_msg_more: bool,
    msg_ext_kind: *const c_char,
    msg_ext_trigger: *const c_char,
    msg_ext_id: MsgID,
    msg_ext_chunks: Array,
    msg_ext_last_chunk: Vec<c_char>,
    msg_ext_last_attr: Sattr,
    msg_ext_last_hl_id: i32,
    msg_ext_history: bool,
    msg_ext_append: bool,
    msg_grid_pos_at_flush: i32,
    msg_special_case_for_lua_print: bool,
    msg_id_next: i64,
    is_multihl: i32,
    msg_keep_entered: i32,
    last_sourcing_lnum: i32,
    last_sourcing_name: Option<std::ffi::CString>,
    msg_source_recursive: bool,
    do_clear_hist_temp: bool,
    redir_col: i32,
    str2_special_buf: [c_char; 7],
    msgchunks: Vec<Msgchunk>,
    do_clear_sb_text: SbClear,
    do_more_prompt_entered: bool,
    showmode_clear: bool,
    pre_verbose_kind: *const c_char,
}

impl MessageState {
    const fn new() -> Self {
        Self {
            confirm_msg_used: 0,
            msg_hist: std::collections::VecDeque::new(),
            msg_hist_temp_idx: None,
            msg_hist_len: 0,
            msg_hist_max: 500,
            msg_hit_enter: true,
            msg_wait: 0,
            progress_msg_target: PROGRESS_TARGET_CMD,
            verbose_fd: None,
            verbose_did_open: false,
            keep_msg_more: false,
            msg_ext_kind: ptr::null(),
            msg_ext_trigger: ptr::null(),
            msg_ext_id: Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer(1),
            },
            msg_ext_chunks: Array::new(),
            msg_ext_last_chunk: Vec::new(),
            msg_ext_last_attr: -1,
            msg_ext_last_hl_id: 0,
            msg_ext_history: false,
            msg_ext_append: false,
            msg_grid_pos_at_flush: 0,
            msg_special_case_for_lua_print: false,
            msg_id_next: 1,
            is_multihl: 0,
            msg_keep_entered: 0,
            last_sourcing_lnum: 0,
            last_sourcing_name: None,
            msg_source_recursive: false,
            do_clear_hist_temp: true,
            redir_col: 0,
            str2_special_buf: [0; 7],
            msgchunks: Vec::new(),
            do_clear_sb_text: SbClear::SbClearNone,
            do_more_prompt_entered: false,
            showmode_clear: false,
            pre_verbose_kind: ptr::null(),
        }
    }
}

static MSG_STATE: GlobalState<MessageState> = GlobalState::new(MessageState::new());

#[inline]
fn msg_st() -> &'static mut MessageState {
    unsafe { &mut *MSG_STATE.as_ptr() }
}

#[macro_export]
macro_rules! smsg {
    ($hl_id:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let hl = $hl_id;
        let fmt_ptr: *const std::ffi::c_char =
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt);
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg(
                (&raw const $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
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
        let fmt_ptr: *const std::ffi::c_char =
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt);
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg_keep(
                (&raw const $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
                hl,
                true,
                false,
            )
        }
    }};
}
pub use smsg_keep;

#[inline(never)]
pub fn semsg_fmt(fmt: impl crate::strings::IntoCCharPtr, args: &[crate::strings::FmtVal]) -> bool {
    let fmt = fmt.into_c_char_ptr();
    if emsg_not_now() {
        return true;
    }
    let mut errbuf = [0 as c_char; 1025];
    crate::strings::vim_snprintf_fmt(errbuf.as_mut_ptr(), errbuf.len(), fmt, args);
    emsg(errbuf.as_ptr())
}

#[inline(never)]
pub fn semsg_multiline_fmt(
    kind: impl crate::strings::IntoCCharPtr,
    fmt: impl crate::strings::IntoCCharPtr,
    args: &[crate::strings::FmtVal],
) -> bool {
    let kind = kind.into_c_char_ptr();
    let fmt = fmt.into_c_char_ptr();
    if emsg_not_now() {
        return true;
    }
    let mut errbuf = [0 as c_char; 8192];
    crate::strings::vim_snprintf_fmt(errbuf.as_mut_ptr(), errbuf.len(), fmt, args);
    emsg_multiline(errbuf.as_ptr(), kind, 6, true)
}

#[macro_export]
macro_rules! semsg {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        $crate::message::semsg_fmt(
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt),
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
    }};
}
pub use semsg;

#[macro_export]
macro_rules! semsg_multiline {
    ($kind:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        $crate::message::semsg_multiline_fmt(
            $crate::strings::IntoCCharPtr::into_c_char_ptr($kind),
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt),
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
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
        let fmt_ptr: *const std::ffi::c_char =
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt);
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg_schedule_semsg_cstr(
                (&raw const $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
            );
        }
    }};
}
pub use msg_schedule_semsg;

#[macro_export]
macro_rules! msg_schedule_semsg_multiline {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        let fmt_ptr: *const std::ffi::c_char =
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt);
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::msg_schedule_semsg_multiline_cstr(
                (&raw const $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
            );
        }
    }};
}
pub use msg_schedule_semsg_multiline;

#[macro_export]
macro_rules! swmsg {
    ($hl:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let hl_val: bool = $hl;
        let fmt_ptr: *const std::ffi::c_char =
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt);
        {
            $crate::strings::vim_snprintf!(
                (&raw mut $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
                1025,
                fmt_ptr
                $(, $arg)*
            );
            $crate::message::give_warning(
                (&raw const $crate::globals::g_st().iobuff).cast::<std::ffi::c_char>(),
                hl_val,
                true,
            );
        }
    }};
}
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::win_config::K_UIMULTIGRID;
use crate::buffer::K_UIMESSAGES;
use crate::change::{HLF_W, MB_MAXCHAR};
use crate::charset::NUL;
use crate::cmdexpand::MB_MAXBYTES;
use crate::drawline::{HLF_0, HLF_AT};
use crate::eval::typval::{K_CALLBACK_NONE_CONST, VAR_UNLOCKED};
use crate::eval::userfunc::MSG_BUF_LEN;
use crate::ex_cmds::{CTRL_C_CONST, HLF_R};
use crate::ex_getln::{CTRL_B_CONST, CTRL_F_CONST, K_EVENT};
pub use swmsg;

use crate::keycodes::{k_second, k_third};

#[inline]
fn have_sourcing_info() -> bool {
    !rt_st().exestack.is_empty()
}

/// Returns true if the given integer message-id was previously generated.
pub fn msg_id_exists(id: i64) -> bool {
    id > 0 && id < msg_st().msg_id_next
}

fn ui_ext_msg_set_pos(row: i32, scrolled: bool) {
    {
        let mut buf = [0 as c_char; MAX_SCHAR_SIZE];
        let size = schar_get(buf.as_mut_ptr(), curwin_mut().w_p_fcs_chars.msgsep);
        ui_call_msg_set_pos(
            g_st().msg_grid.handle as Integer,
            row as Integer,
            scrolled,
            NvimString {
                data: buf.as_mut_ptr(),
                size,
            },
            g_st().msg_grid.zindex as Integer,
            g_st().msg_grid.comp_index as Integer,
        );
        g_st().msg_grid.pending_comp_index_update = false;
    }
}

pub fn msg_grid_set_pos(row: i32, scrolled: bool) {
    if !g_st().msg_grid.throttled {
        ui_ext_msg_set_pos(row, scrolled);
        msg_st().msg_grid_pos_at_flush = row;
    }
    g_st().msg_grid_pos = row;
    if crate::grid::grid_allocated(&g_st().msg_grid) {
        g_st().msg_grid_adj.row_offset = -row;
    }
}

pub fn msg_use_grid() -> bool {
    crate::grid::grid_allocated(&g_st().default_grid) && !ui_has(K_UIMESSAGES)
}

pub fn msg_grid_validate() {
    grid_assign_handle(&mut g_st().msg_grid);
    let should_alloc = msg_use_grid();
    let max_rows = g_st().rows_global - opt_vars().p_ch as i32;
    let msg_grid_empty = !crate::grid::grid_allocated(&g_st().msg_grid);
    if should_alloc
        && (g_st().msg_grid.rows != g_st().rows_global
            || g_st().msg_grid.cols != g_st().columns_global
            || msg_grid_empty)
    {
        grid_alloc(
            &mut g_st().msg_grid,
            g_st().rows_global,
            g_st().columns_global,
            false,
            true,
        );
        g_st().msg_grid.zindex = K_ZINDEX_MESSAGES;

        g_st().msg_grid.dirty_col = vec![0; g_st().rows_global as usize];

        // Tricky: allow resize while pager or ex mode is active
        let pos = if (g_st().state_global & MODE_ASKMORE) != 0 {
            0
        } else {
            (max_rows - g_st().msg_scrolled).max(0)
        };
        g_st().msg_grid.throttled = false; // don't throttle in 'cmdheight' area
        msg_grid_set_pos(pos, g_st().msg_scrolled != 0);
        ui_comp_put_grid(
            &mut g_st().msg_grid,
            pos,
            0,
            g_st().msg_grid.rows,
            g_st().msg_grid.cols,
            false,
            true,
        );
        ui_call_grid_resize(
            g_st().msg_grid.handle as Integer,
            g_st().msg_grid.cols as Integer,
            g_st().msg_grid.rows as Integer,
        );

        g_st().msg_scrolled_at_flush = g_st().msg_scrolled;
        g_st().msg_grid.mouse_enabled = false;
        g_st().msg_grid_adj.target = &raw mut g_st().msg_grid;
    } else if !should_alloc && !msg_grid_empty {
        ui_comp_remove_grid(&mut g_st().msg_grid);
        grid_free(&mut g_st().msg_grid);
        g_st().msg_grid.dirty_col = Vec::new();
        ui_call_grid_destroy(g_st().msg_grid.handle as Integer);
        g_st().msg_grid.throttled = false;
        g_st().msg_grid_adj.row_offset = 0;
        g_st().msg_grid_adj.target = &raw mut g_st().default_grid;
        g_st().redraw_cmdline = true;
    } else if !msg_grid_empty && g_st().msg_scrolled == 0 && g_st().msg_grid_pos != max_rows {
        let diff = g_st().msg_grid_pos - max_rows;
        msg_grid_set_pos(max_rows, false);
        if diff > 0 {
            grid_clear(
                &raw mut g_st().msg_grid_adj,
                g_st().rows_global - diff,
                g_st().rows_global,
                0,
                g_st().columns_global,
                hl_attr(HLF_MSG),
            );
        }
    }

    if crate::grid::grid_allocated(&g_st().msg_grid)
        && g_st().msg_scrolled == 0
        && g_st().cmdline_row < g_st().msg_grid_pos
    {
        g_st().cmdline_row = g_st().msg_grid_pos;
    }
}

/// Like msg() but keep it silent when 'verbosefile' is set.
pub fn verb_msg(s: impl crate::strings::IntoCCharPtr) -> i32 {
    let s = s.into_c_char_ptr();
    verbose_enter();
    let n = i32::from(msg_keep(s, 0, false, false));
    verbose_leave();
    n
}

/// Displays the string 's' on the status line
/// When terminal not initialized (yet) printf("%s", ..) is used.
///
/// @return  true if wait_return() not called
pub fn msg<S: crate::strings::IntoCCharPtr>(s: S, hl_id: i32) -> bool {
    msg_keep(s.into_c_char_ptr(), hl_id, false, false)
}

/// Similar to msg_outtrans_len, but support newlines and tabs.
pub fn msg_multiline(
    str_: NvimString,
    hl_id: i32,
    check_int: bool,
    hist: bool,
    need_clear: &mut bool,
) {
    unsafe {
        let mut s = str_.data.cast_const();
        let mut chunk = s;
        while (crate::memory::ptr_offset_from(s, str_.data) as usize) < str_.size {
            if check_int && g_st().got_int {
                return;
            }
            if *s == NL as c_char
                || *s == TAB as c_char
                || *s == CAR as c_char
                || *s == BELL as c_char
            {
                // Print all chars before the delimiter
                msg_outtrans_len(
                    chunk,
                    crate::memory::ptr_offset_from(s, chunk) as i32,
                    hl_id,
                    hist,
                );

                if *s != TAB as c_char && *need_clear {
                    msg_clr_eos();
                    *need_clear = false;
                }
                if *s == BELL as c_char {
                    vim_beep(K_OPT_BO_FLAG_SHELL);
                } else {
                    msg_putchar_hl((crate::memory::cstr_first_byte(s)) as i32, hl_id);
                }
                chunk = s.wrapping_add(1);
            }
            s = s.wrapping_add(1);
        }

        // Print the remainder or emit empty event if entire message is empty.
        if !crate::memory::cstr_is_empty(chunk) || chunk == str_.data.cast_const() {
            msg_outtrans_len(
                chunk,
                (str_.size - (crate::memory::ptr_offset_from(chunk, str_.data) as usize)) as i32,
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
fn format_progress_message(
    hl_msg: &HlMessage,
    mut msg_data: Option<&mut MessageData>,
    updated_msg: &mut HlMessage,
) -> bool {
    {
        // progress messages are special. displayed as "title: percent% msg"
        if msg_data.as_deref_mut().unwrap().title.size != 0 {
            // this block draws the "title:" before the progress-message
            let mut hl_id = 0;
            if msg_data.as_deref_mut().unwrap().status.data.is_null() {
                hl_id = 0;
            } else if crate::memory::cstr_as_opt_bytes(msg_data.as_deref_mut().unwrap().status.data)
                == Some(b"success")
            {
                hl_id = syn_check_group(c"OkMsg", 5);
            } else if crate::memory::cstr_as_opt_bytes(msg_data.as_deref_mut().unwrap().status.data)
                == Some(b"failed")
            {
                hl_id = syn_check_group(c"ErrorMsg", 8);
            } else if crate::memory::cstr_as_opt_bytes(msg_data.as_deref_mut().unwrap().status.data)
                == Some(b"running")
            {
                hl_id = syn_check_group(c"MoreMsg", 7);
            } else if crate::memory::cstr_as_opt_bytes(msg_data.as_deref_mut().unwrap().status.data)
                == Some(b"cancel")
            {
                hl_id = syn_check_group(c"WarningMsg", 10);
            }
            updated_msg.push(HlMessageChunk {
                text: copy_string(msg_data.as_deref_mut().unwrap().title, None),
                hl_id,
            });
            updated_msg.push(HlMessageChunk {
                text: cstr_to_string(c": "),
                hl_id: 0,
            });
        }
        if msg_data.as_deref_mut().unwrap().percent >= 0 {
            let mut percent_buf = [0 as c_char; 10];
            vim_snprintf!(
                percent_buf.as_mut_ptr(),
                percent_buf.len(),
                c"%3ld%% ",
                msg_data.unwrap().percent,
            );
            let percent = cstr_to_string(percent_buf.as_ptr());
            let hl_id = syn_check_group(c"WarningMsg", 10);
            updated_msg.push(HlMessageChunk {
                text: percent,
                hl_id,
            });
        }

        if !updated_msg.is_empty() {
            for chunk in hl_msg {
                updated_msg.push(HlMessageChunk {
                    text: copy_string(chunk.text, None),
                    hl_id: chunk.hl_id,
                });
            }
            return true;
        }
        false
    }
}

/// Print message chunks, each with their own highlight ID.
pub fn msg_multihl(
    mut id: MsgID,
    hl_msg: &mut HlMessage,
    kind: impl crate::strings::IntoCCharPtr,
    history: bool,
    err: bool,
    mut msg_data: Option<&mut MessageData>,
    needs_msg_clear: &mut bool,
    raw_for_lua_print: bool,
) -> MsgID {
    let kind = kind.into_c_char_ptr();
    if id.obj_type == ObjectType::ObjectTypeNil {
        let next_id = msg_st().msg_id_next;
        msg_st().msg_id_next += 1;
        id = Object {
            obj_type: ObjectType::ObjectTypeInteger,
            data: ObjectData::Integer(next_id),
        };
    } else if id.obj_type == ObjectType::ObjectTypeInteger && !msg_id_exists(id.data.integer()) {
        std::process::abort();
    }

    let mut updated_msg = HlMessage::new();
    let mut hl_msg_updated = false;
    // don't display progress message in cmd when target doesn't have cmd
    if crate::memory::cstr_as_opt_bytes(kind) == Some(b"progress") {
        do_autocmd_progress(id, hl_msg, msg_data.as_deref_mut());
        if (msg_st().progress_msg_target & PROGRESS_TARGET_CMD) == 0 {
            *needs_msg_clear = true;
            return id;
        }
        if msg_data.is_some() && format_progress_message(hl_msg, msg_data, &mut updated_msg) {
            *needs_msg_clear = true;
            hl_msg_updated = true;
        }
    }

    let active_msg: &HlMessage = if hl_msg_updated { &updated_msg } else { hl_msg };

    g_st().no_wait_return += 1;
    msg_start();
    msg_clr_eos();
    let mut need_clear = false;
    if !kind.is_null() {
        msg_ext_set_kind(kind);
    }
    if raw_for_lua_print {
        msg_st().msg_special_case_for_lua_print = true;
        assert!(active_msg.len() == 1);
        let chunk = &active_msg[0];
        printf!(c"%.*s\n", chunk.text.size as i32, chunk.text.data,);
    }

    g_st().msg_ext_skip_flush = true;
    msg_st().msg_ext_id = id;

    for chunk in active_msg {
        msg_st().is_multihl += 1;
        if err {
            emsg_multiline(chunk.text.data, kind, chunk.hl_id, true);
        } else {
            msg_multiline(chunk.text, chunk.hl_id, true, false, &mut need_clear);
        }
        assert!(!ui_has(K_UIMESSAGES) || kind.is_null() || msg_st().msg_ext_kind == kind);
    }

    let has_chunks = !active_msg.is_empty();
    if history && has_chunks {
        if hl_msg_updated {
            msg_hist_add_multihl(std::mem::take(&mut updated_msg), false);
        } else {
            msg_hist_add_multihl(std::mem::take(hl_msg), false);
        }
    }

    g_st().msg_ext_skip_flush = false;
    msg_st().is_multihl = 0;
    g_st().no_wait_return -= 1;
    msg_end();

    // Release the id: it belongs to this message, and a String id only borrows the caller's storage
    // (often a stack buffer). Redundant unless nothing was emitted (e.g. 'msg_silent').
    msg_st().msg_ext_id = Object {
        obj_type: ObjectType::ObjectTypeInteger,
        data: ObjectData::Integer(msg_st().msg_id_next),
    };
    id
}

/// @param keep set keep_msg if it doesn't scroll
pub fn msg_keep(
    s: impl crate::strings::IntoCCharPtr,
    hl_id: i32,
    keep: bool,
    multiline: bool,
) -> bool {
    let mut s = s.into_c_char_ptr();
    {
        if keep && multiline {
            std::process::abort();
        }

        // Skip messages not match ":filter pattern".
        // Don't filter when there is an error.
        if !g_st().emsg_on_display && message_filtered(s) {
            return true;
        }

        if hl_id == 0 {
            set_vim_var_string(VV_STATUSMSG, s, -1);
        }

        // It is possible that displaying a messages causes a problem (e.g.,
        // when redrawing the window), which causes another message, etc..    To
        // break this loop, limit the recursiveness to 3 levels.
        if msg_st().msg_keep_entered >= 3 {
            return true;
        }
        msg_st().msg_keep_entered += 1;

        // Add message to history unless it's a multihl, repeated kept or truncated message.
        let keep_msg_ptr = unsafe { (&raw const g_st().keep_msg).as_ref() }
            .unwrap()
            .as_ref()
            .map_or(ptr::null(), |k| k.as_ptr());
        if msg_st().is_multihl == 0
            && (s != keep_msg_ptr
                || (crate::memory::cstr_first_byte(s) != b'<'
                    && msg_st().msg_hist.back().is_some_and(|last| {
                        crate::memory::cstr_as_bytes(s)
                            != crate::memory::cstr_as_bytes(last.msg.as_slice()[0].text.data)
                    })))
        {
            msg_hist_add(s, -1, hl_id);
        }

        if msg_st().is_multihl == 0 {
            msg_start();
        }
        // Truncate the message if needed.
        let buf = msg_strtrunc(s, 0);
        if let Some(ref buf) = buf {
            s = buf.as_ptr();
        }

        let mut need_clear = true;
        if multiline {
            msg_multiline(cstr_as_string(s), hl_id, false, false, &mut need_clear);
        } else {
            msg_outtrans(s, hl_id, false);
        }
        if need_clear {
            msg_clr_eos();
        }
        let mut retval = true;
        if msg_st().is_multihl == 0 {
            retval = msg_end();
        }

        if keep
            && retval
            && vim_strsize(s)
                < (g_st().rows_global - g_st().cmdline_row - 1) * g_st().columns_global
                    + g_st().sc_col
        {
            set_keep_msg(s, 0);
        }

        g_st().need_fileinfo = false;

        drop(buf);
        msg_st().msg_keep_entered -= 1;
        retval
    }
}

/// Truncate a string such that it can be printed without causing a scroll.
///
/// @param force  always truncate
pub fn msg_strtrunc(s: impl crate::strings::IntoCCharPtr, force: i32) -> Option<std::ffi::CString> {
    let s = s.into_c_char_ptr();
    {
        // May truncate message to avoid a hit-return prompt
        if (g_st().msg_scroll == 0
            && !g_st().need_wait_return
            && shortmess(K_SHM_TRUNCALL)
            && g_st().msg_silent == 0
            && !ui_has(K_UIMESSAGES))
            || force != 0
        {
            let len = vim_strsize(s);
            let room = if g_st().msg_scrolled != 0 {
                // Use all the columns.
                (g_st().rows_global - g_st().msg_row) * g_st().columns_global - 1
            } else {
                // Use up to 'showcmd' column.
                let last_row = if ui_has(K_UIMESSAGES) {
                    g_st().columns_global
                } else {
                    g_st().sc_col - 1
                };
                (g_st().rows_global - g_st().msg_row - 1) * g_st().columns_global + last_row
            };
            if len > room && room > 0 {
                // may have up to 18 bytes per cell (6 per char, up to two
                // composing chars)
                let buflen = (room + 2) * 18;
                let mut buf: Vec<c_char> = vec![0; buflen as usize];
                trunc_string(s, buf.as_mut_ptr(), room, buflen);
                return Some(crate::memory::cstr_to_cstring(buf.as_ptr()));
            }
        }
        None
    }
}

/// Truncate a string "s" to "buf" with cell width "room".
/// "s" and "buf" may be equal.
pub fn trunc_string(
    s: impl crate::strings::IntoCCharPtr,
    buf: *mut c_char,
    room_in: i32,
    buflen: i32,
) {
    let s = s.into_c_char_ptr();
    unsafe {
        let mut room = room_in - 3; // "..." takes 3 chars
        let mut len: i32 = 0;
        let mut e: i32 = 0;

        if crate::memory::cstr_is_empty(s) {
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
            if crate::memory::cstr_byte_at(s, e as usize) == 0 {
                // text fits without truncating!
                *buf.wrapping_add(e as usize) = NUL;
                return;
            }
            let mut n = ptr2cells(s.wrapping_add(e as usize));
            if len + n > half {
                break;
            }
            len += n;
            *buf.wrapping_add(e as usize) = *s.wrapping_add(e as usize);
            n = utfc_ptr2len(s.wrapping_add(e as usize));
            while {
                n -= 1;
                n > 0
            } {
                e += 1;
                if e == buflen {
                    break;
                }
                *buf.wrapping_add(e as usize) = *s.wrapping_add(e as usize);
            }
            e += 1;
        }

        // Last part: End of the string.
        let mut i = crate::memory::cstr_len(s) as i32;
        half = i;
        loop {
            half = half - utf_head_off(s, s.wrapping_add((half - 1) as usize)) - 1;
            let n = ptr2cells(s.wrapping_add(half as usize));
            if len + n > room || half == 0 {
                break;
            }
            len += n;
            i = half;
        }

        if i <= e + 3 {
            // text fits without truncating
            if !ptr::eq(s, buf) {
                len = crate::memory::cstr_len(s) as i32;
                if len >= buflen {
                    len = buflen - 1;
                }
                len = len - e + 1;
                if len < 1 {
                    *buf.wrapping_add((e - 1) as usize) = NUL;
                } else {
                    ptr::copy(
                        s.wrapping_add(e as usize),
                        buf.wrapping_add(e as usize),
                        len as usize,
                    );
                }
            }
        } else if e + 3 < buflen {
            // set the middle and copy the last part
            ptr::copy(c"...".as_ptr(), buf.wrapping_add(e as usize), 3);
            len = crate::memory::cstr_as_bytes(s)[i as usize..].len() as i32 + 1;
            if len >= buflen - e - 3 {
                len = buflen - e - 3 - 1;
            }
            ptr::copy(
                s.wrapping_add(i as usize),
                buf.wrapping_add((e + 3) as usize),
                len as usize,
            );
            *buf.wrapping_add((e + 3 + len - 1) as usize) = NUL;
        } else {
            // can't fit in the "...", just truncate it
            *buf.wrapping_add((buflen - 1) as usize) = NUL;
        }
    }
}

/// Reset the last used sourcing name/lnum.  Makes sure it is displayed again
/// for the next error message;
pub fn reset_last_sourcing() {
    msg_st().last_sourcing_name = None;
    msg_st().last_sourcing_lnum = 0;
}

/// @return  true if "SOURCING_NAME" differs from "last_sourcing_name".
fn other_sourcing_name() -> bool {
    {
        let sname = sourcing_name();
        if have_sourcing_info() && !sname.is_null() {
            if let Some(ref last) = msg_st().last_sourcing_name {
                return crate::memory::cstr_as_bytes(sname) != last.as_bytes();
            }
            return true;
        }
        false
    }
}

/// Get the message about the source, as used for an error message
fn get_emsg_source() -> Option<std::ffi::CString> {
    {
        let mut sname = sourcing_name();
        if have_sourcing_info() && !sname.is_null() && other_sourcing_name() {
            let tofree = estack_sfile(ESTACK_NONE);
            if let Some(ref tf) = tofree {
                sname = tf.as_ptr().cast_mut();
            }

            let p = gettext(c"Error in %s:");
            let buf_len = crate::memory::cstr_len(sname) + crate::memory::cstr_len(p) + 1;
            let mut buf_vec: Vec<c_char> = vec![0; buf_len];
            vim_snprintf!(buf_vec.as_mut_ptr(), buf_len, p, sname);
            return Some(crate::memory::cstr_to_cstring(buf_vec.as_ptr()));
        }
        None
    }
}

/// Get the message about the source lnum, as used for an error message.
fn get_emsg_lnum() -> Option<std::ffi::CString> {
    {
        let sname = sourcing_name();
        let slnum = sourcing_lnum();
        if !sname.is_null()
            && (other_sourcing_name() || slnum != msg_st().last_sourcing_lnum)
            && slnum != 0
        {
            let p = gettext(c"line %4d:");
            let buf_len = 20 + crate::memory::cstr_len(p);
            let mut buf_vec: Vec<c_char> = vec![0; buf_len];
            vim_snprintf!(buf_vec.as_mut_ptr(), buf_len, p, slnum);
            return Some(crate::memory::cstr_to_cstring(buf_vec.as_ptr()));
        }
        None
    }
}

/// Display name and line number for the source of an error.
/// Remember the file name and line number, so that for the next error the info
/// is only displayed if it changed.
pub fn msg_source(hl_id: i32) {
    {
        if msg_st().msg_source_recursive {
            return;
        }
        msg_st().msg_source_recursive = true;

        g_st().no_wait_return += 1;
        if let Some(p) = get_emsg_source() {
            g_st().msg_scroll = 1;
            msg(p.as_ptr(), hl_id);
        }
        if let Some(p) = get_emsg_lnum() {
            msg(p.as_ptr(), HLF_N);
            msg_st().last_sourcing_lnum = sourcing_lnum();
        }

        let sname = sourcing_name();
        if sname.is_null() || other_sourcing_name() {
            msg_st().last_sourcing_name = None;
            if !sname.is_null() {
                msg_st().last_sourcing_name = Some(crate::memory::cstr_to_cstring(sname));
                if redirecting() == 0 {
                    msg_putchar_hl(NL, hl_id);
                }
            }
        }
        g_st().no_wait_return -= 1;

        msg_st().msg_source_recursive = false;
    }
}

/// @return  true if not giving error messages right now:
///            If "emsg_off" is set: no error messages at the moment.
///            If "msg" is in 'debug': do error message but without side effects.
///            If "emsg_skip" is set: never do error messages.
pub fn emsg_not_now() -> bool {
    (g_st().emsg_off > 0
        && !crate::strings::vim_haschr(opt_vars().p_debug, b'm' as i32)
        && !crate::strings::vim_haschr(opt_vars().p_debug, b't' as i32))
        || g_st().emsg_skip > 0
}

pub fn emsg_multiline(
    s: impl crate::strings::IntoCCharPtr,
    kind: impl crate::strings::IntoCCharPtr,
    hl_id: i32,
    multiline: bool,
) -> bool {
    let s = s.into_c_char_ptr();
    let kind = kind.into_c_char_ptr();
    {
        let mut ignore = false;

        // Skip this if not giving error messages at the moment.
        if emsg_not_now() {
            return true;
        }

        g_st().called_emsg += 1;

        // If "emsg_severe" is true: When an error exception is to be thrown,
        // prefer this message over previous messages for the same command.
        let severe = g_st().emsg_severe;
        g_st().emsg_severe = false;

        if g_st().emsg_off == 0 || crate::strings::vim_haschr(opt_vars().p_debug, b't' as i32) {
            if cause_errthrow(s, multiline, msg_st().is_multihl > 1, severe, &mut ignore) {
                if !ignore {
                    g_st().did_emsg += 1;
                }
                return true;
            }

            if g_st().in_assert_fails
                && unsafe { (&raw const g_st().emsg_assert_fails_msg).as_ref() }
                    .unwrap()
                    .is_none()
            {
                g_st().emsg_assert_fails_msg = Some(crate::memory::cstr_to_cstring(s));
                g_st().emsg_assert_fails_lnum = sourcing_lnum() as i64;
                let sname = sourcing_name();
                g_st().emsg_assert_fails_context = Some(if sname.is_null() {
                    std::ffi::CString::default()
                } else {
                    crate::memory::cstr_to_cstring(sname)
                });
            }

            // set "v:errmsg", also when using ":silent! cmd"
            set_vim_var_string(VV_ERRMSG, s, -1);

            let sname = sourcing_name();
            let slnum = sourcing_lnum();

            // When using ":silent! cmd" ignore error messages.
            // But do write it to the redirection file.
            if g_st().emsg_silent != 0 {
                if !g_st().emsg_noredir {
                    msg_start();
                    if let Some(p) = get_emsg_source() {
                        let p_len = p.as_bytes().len();
                        redir_write(p.as_ptr(), p_len as isize);
                        redir_write(c"\n", 1);
                    }
                    if let Some(p) = get_emsg_lnum() {
                        let p_len = p.as_bytes().len();
                        redir_write(p.as_ptr(), p_len as isize);
                        redir_write(c"\n", 1);
                    }
                    redir_write(s, crate::memory::cstr_len(s) as isize);
                }

                // Log (silent) errors as debug messages.
                if !sname.is_null() && slnum != 0 {
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"emsg_multiline",
                        line!() as i32,
                        true,
                        c"(:silent) %s (%s (line %d))",
                        s,
                        sname,
                        slnum,
                    );
                } else {
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"emsg_multiline",
                        line!() as i32,
                        true,
                        c"(:silent) %s",
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
                    c"emsg_multiline",
                    line!() as i32,
                    true,
                    c"%s (%s (line %d))",
                    s,
                    sname,
                    slnum,
                );
            } else {
                logmsg!(
                    LOGLVL_INF,
                    ptr::null(),
                    c"emsg_multiline",
                    line!() as i32,
                    true,
                    c"%s",
                    s,
                );
            }

            g_st().ex_exitval = 1;

            // Reset msg_silent, an error causes messages to be switched back on.
            g_st().msg_silent = 0;
            g_st().cmd_silent = false;

            if g_st().global_busy != 0 {
                // break :global command
                g_st().global_busy += 1;
            }

            if opt_vars().p_eb != 0 {
                beep_flush(); // also includes flush_buffers()
            } else {
                flush_buffers(FLUSH_MINIMAL); // flush internal buffers
            }
            g_st().did_emsg += 1; // flag for DoOneCmd()
        }

        g_st().emsg_on_display = true; // remember there is an error message
        if g_st().msg_scrolled != 0 {
            g_st().need_wait_return = true;
        }
        msg_ext_set_kind(kind);

        // Display name and line number for the source of the error.
        g_st().msg_scroll = 1;
        let save_msg_skip_flush = g_st().msg_ext_skip_flush;
        g_st().msg_ext_skip_flush = true;
        msg_source(hl_id);

        // Display the error message itself.
        g_st().msg_nowait = false; // Wait for this msg.
        let rv = msg_keep(s, hl_id, false, multiline);
        g_st().msg_ext_skip_flush = save_msg_skip_flush;
        rv
    }
}

/// emsg() - display an error message
pub fn emsg<S: crate::strings::IntoCCharPtr>(s: S) -> bool {
    emsg_multiline(s.into_c_char_ptr(), c"emsg", HLF_E, false)
}

pub fn emsg_invreg(name: i32) {
    semsg!(
        gettext(c"E354: Invalid register name: '%s'"),
        transchar_buf(None, name),
    );
}

/// Same as emsg(...), but abort on error when ABORT_ON_INTERNAL_ERROR is
/// defined. It is used for internal errors only, so that they can be
/// detected when fuzzing vim.
pub fn iemsg<S: crate::strings::IntoCCharPtr>(s: S) {
    if emsg_not_now() {
        return;
    }
    emsg(s);
}

/// Give an "Internal error" message.
pub fn internal_error(where_: &std::ffi::CStr) {
    siemsg!(gettext(E_INTERN2), where_.as_ptr());
}

pub fn msg_semsg_event(s: std::ffi::CString) {
    emsg(s.as_ptr());
}

pub fn msg_schedule_semsg_cstr(s: impl crate::strings::IntoCCharPtr) {
    let s = s.into_c_char_ptr();
    {
        let dup = crate::memory::cstr_to_cstring(s);
        loop_schedule_deferred(&mut g_st().main_loop, Event::MsgSemsg(dup));
    }
}

pub fn msg_semsg_multiline_event(s: std::ffi::CString) {
    emsg_multiline(s.as_ptr(), c"emsg", HLF_E, true);
}

pub fn msg_schedule_semsg_multiline_cstr(s: impl crate::strings::IntoCCharPtr) {
    let s = s.into_c_char_ptr();
    {
        let dup = crate::memory::cstr_to_cstring(s);
        loop_schedule_deferred(&mut g_st().main_loop, Event::MsgSemsgMultiline(dup));
    }
}

/// Like msg(), but truncate to a single line if opt_vars().p_shm contains 't', or when
/// "force" is true.  This truncates in another way as for normal messages.
/// Careful: The string may be changed by msg_may_trunc()!
///
/// @return  a pointer to the printed message, if wait_return() not called.
pub fn msg_trunc(s: *mut c_char, force: bool, hl_id: i32) -> *mut c_char {
    // Add message to history before truncating.
    msg_hist_add(s, -1, hl_id);

    let ts = msg_may_trunc(force, s);

    g_st().msg_hist_off = true;
    let n = msg(ts, hl_id);
    g_st().msg_hist_off = false;

    if n { ts } else { ptr::null_mut() }
}

/// Check if message "s" should be truncated at the start (for filenames).
///
/// @return  a pointer to where the truncated message starts.
///
/// @note: May change the message by replacing a character with '<'.
pub fn msg_may_trunc(force: bool, mut s: *mut c_char) -> *mut c_char {
    unsafe {
        if ui_has(K_UIMESSAGES) {
            return s;
        }

        let room = (g_st().rows_global - g_st().cmdline_row - 1) * g_st().columns_global
            + g_st().sc_col
            - 1;
        if room > 0
            && (force || shortmess(K_SHM_TRUNC))
            && (crate::memory::cstr_len(s) as i32) - room > 0
        {
            let mut size = vim_strsize(s);

            // There may be room anyway when there are multibyte chars.
            if size <= room {
                return s;
            }
            let mut n = 0usize;
            while size >= room {
                size -= utf_ptr2cells(s.wrapping_add(n));
                n += utfc_ptr2len(s.wrapping_add(n)) as usize;
            }
            n -= 1;
            s = s.wrapping_add(n);
            *s = b'<' as c_char;
        }
        s
    }
}

/// Shows a progress-message.  A non-"running" `status` ends the message and announces the
/// progress task as "done".
pub fn msg_progress(
    s: *mut c_char,
    id: impl crate::strings::IntoCCharPtr,
    status: impl crate::strings::IntoCCharPtr,
    percent: i32,
    hl_id: i32,
    hist: bool,
    trunc: bool,
    err: bool,
) -> *mut c_char {
    let status = status.into_c_char_ptr().cast_mut();
    let id = id.into_c_char_ptr().cast_mut();
    let mut data = MessageData {
        source: cstr_as_string(c"nvim"),
        percent: Integer::from(percent),
        title: NvimString {
            data: ptr::null_mut(),
            size: 0,
        },
        status: cstr_as_string(status),
        data: Dict::new(),
    };
    msg_with_id(s, id, c"progress", Some(&mut data), hl_id, hist, trunc, err)
}

/// Shows message `s` with `id`: replaces the message with the same `id`, if any.
pub fn msg_with_id(
    mut s: *mut c_char,
    id: impl crate::strings::IntoCCharPtr,
    kind: &std::ffi::CStr,
    data: Option<&mut MessageData>,
    hl_id: i32,
    hist: bool,
    trunc: bool,
    err: bool,
) -> *mut c_char {
    let id = id.into_c_char_ptr();
    let mut clear = false;
    let mut chunks = HlMessage::new();
    if !s.is_null() {
        if hist && (!trunc || ui_has(K_UIMESSAGES)) {
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
        obj_type: ObjectType::ObjectTypeString,
        data: ObjectData::String(cstr_as_string(id)),
    };
    msg_multihl(
        id_obj,
        &mut chunks,
        kind.as_ptr(),
        false,
        err,
        data,
        &mut clear,
        false,
    );
    ui_flush();
    s
}

/// Add the message at the end of the history
///
/// @param[in]  len  Length of s or -1.
fn msg_hist_add(s: impl crate::strings::IntoCCharPtr, len: i32, hl_id: i32) {
    let mut s = s.into_c_char_ptr();
    unsafe {
        let mut size = if len < 0 {
            crate::memory::cstr_len(s)
        } else {
            len as usize
        };
        // Remove leading and trailing newlines.
        while size > 0 && *s == NL as c_char {
            size -= 1;
            s = s.wrapping_add(1);
        }
        while size > 0 && *s.wrapping_add(size - 1) == NL as c_char {
            size -= 1;
        }
        if size == 0 {
            return;
        }
        let text = crate::api::private::helpers::cbuf_to_string(s, size);

        let msg = vec![HlMessageChunk { text, hl_id }];
        msg_hist_add_multihl(msg, false);
    }
}

pub fn do_autocmd_progress(msg_id: MsgID, msg: &HlMessage, mut msg_data: Option<&mut MessageData>) {
    {
        if !has_event(EVENT_PROGRESS) {
            return;
        }

        let mut messages_vec: Vec<Object> = msg
            .iter()
            .map(|chunk| Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(chunk.text),
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
                obj_type: ObjectType::ObjectTypeArray,
                data: ObjectData::Array(messages),
            },
        );
        if msg_data.is_some() {
            if msg_data.as_deref_mut().unwrap().percent >= 0 {
                put_c(
                    c"percent",
                    Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(msg_data.as_deref_mut().unwrap().percent),
                    },
                );
            }
            put_c(
                c"source",
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(msg_data.as_deref_mut().unwrap().source),
                },
            );
            put_c(
                c"status",
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(msg_data.as_deref_mut().unwrap().status),
                },
            );
            put_c(
                c"title",
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(msg_data.as_deref_mut().unwrap().title),
                },
            );
            put_c(
                c"data",
                Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(msg_data.as_deref_mut().unwrap().data),
                },
            );
        }

        let mut dict_obj = Object {
            obj_type: ObjectType::ObjectTypeDict,
            data: ObjectData::Dict(Dict {
                items: data_items.as_mut_ptr(),
                size: data_size,
                capacity: 7,
            }),
        };
        let fname = if msg_data.is_some() && msg_data.as_deref_mut().unwrap().source.size > 0 {
            msg_data.unwrap().source.data
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
            Some(crate::globals::curwin_mut()),
            None,
            &raw mut dict_obj,
            false,
        );
    }
}

fn msg_hist_add_multihl(msg: HlMessage, temp: bool) {
    {
        if msg_st().do_clear_hist_temp {
            msg_hist_clear_temp();
            msg_st().do_clear_hist_temp = false;
        }

        if g_st().msg_hist_off || g_st().msg_silent != 0 {
            return;
        }

        let temp_idx = msg_st().msg_hist_temp_idx;
        if temp_idx.is_none() {
            msg_st().msg_hist_temp_idx = Some(msg_st().msg_hist.len());
        }
        msg_st().msg_hist.push_back(MessageHistoryEntry {
            msg,
            kind: (!msg_st().msg_ext_kind.is_null())
                .then(|| crate::memory::cstr_to_cstring(msg_st().msg_ext_kind)),
            temp,
            append: msg_st().msg_ext_append,
        });

        msg_st().msg_hist_len += i32::from(!temp);
        msg_st().msg_ext_history = true;

        msg_hist_clear(msg_st().msg_hist_max);
    }
}

/// Delete oldest messages from the history until there are "keep" messages.
pub fn msg_hist_clear(keep: i32) {
    {
        while msg_st().msg_hist_len > keep || (keep == 0 && !msg_st().msg_hist.is_empty()) {
            if let Some(entry) = msg_st().msg_hist.pop_front() {
                msg_st().msg_hist_len -= i32::from(!entry.temp);
                if let Some(idx) = msg_st().msg_hist_temp_idx {
                    msg_st().msg_hist_temp_idx = Some(idx.saturating_sub(1));
                }
            }
        }
        if msg_st().msg_hist.is_empty() {
            msg_st().msg_hist_temp_idx = None;
        }
    }
}

pub fn msg_hist_clear_temp() {
    {
        let temp_idx = msg_st().msg_hist_temp_idx;
        msg_st().msg_hist_temp_idx = None;
        if let Some(start_idx) = temp_idx {
            let mut i = start_idx;
            while i < msg_st().msg_hist.len() {
                if (&msg_st().msg_hist)[i].temp {
                    msg_st().msg_hist.remove(i);
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// Check the 'messagesopt' fields after schema validation.
pub fn validate_messagesopt(args: &mut Optset) -> *const c_char {
    {
        let v = opt_keyset(
            args.os_newval.data.string().data,
            K_OPT_MESSAGESOPT,
            ptr::null_mut(),
        )
        .cast::<OptKeyDict_mopt>();
        let v = unsafe { &mut *v };

        let has_wait = (v.is_set_mopt & (1 << KEYSET_OPTIDX_MOPT_WAIT)) != 0;
        let has_history = (v.is_set_mopt & (1 << KEYSET_OPTIDX_MOPT_HISTORY)) != 0;

        // Either "wait" or "hit-enter" is required; "history" always.
        if (!v.hit_enter && !has_wait) || !has_history {
            return E_INVARG.as_ptr();
        }
        // "history" and "wait" must be <= 10000; "maxheight" is a percentage.
        if v.history > 10000 || v.wait > 10000 || v.maxheight > 100 {
            return E_INVARG.as_ptr();
        }
        ptr::null()
    }
}

/// Process the updated 'messagesopt' option value.
pub fn did_set_messagesopt(_args: &mut Optset) -> *const c_char {
    {
        let v = opt_keyset(opt_vars().p_mopt, K_OPT_MESSAGESOPT, ptr::null_mut())
            .cast::<OptKeyDict_mopt>();
        let v = unsafe { &mut *v };

        msg_st().msg_hit_enter = v.hit_enter;
        msg_st().msg_wait = v.wait as i32;
        msg_st().progress_msg_target =
            if crate::memory::cstr_as_opt_bytes(v.progress.as_ptr()) == Some(b"c") {
                PROGRESS_TARGET_CMD
            } else {
                0
            };
        msg_st().msg_hist_max = v.history as i32;
        msg_hist_clear(msg_st().msg_hist_max);
        ptr::null()
    }
}

/// :messages command implementation
pub fn ex_messages(eap: &mut Exarg) {
    {
        if crate::memory::cstr_as_bytes(eap.arg) == b"clear" {
            msg_hist_clear(if eap.addr_count != 0 { eap.line2 } else { 0 });
            return;
        }

        if !crate::memory::cstr_is_empty(eap.arg) {
            emsg(gettext(E_INVARG));
            return;
        }

        let mut entries = Array::new();
        let mut idx = if eap.skip != 0 {
            msg_st()
                .msg_hist_temp_idx
                .unwrap_or(msg_st().msg_hist.len())
        } else {
            0
        };
        let mut skip = if eap.addr_count != 0 {
            msg_st().msg_hist_len - eap.line2
        } else {
            0
        };
        while idx < msg_st().msg_hist.len() {
            let p = &mut (&mut msg_st().msg_hist)[idx];
            idx += 1;
            // Skip over count or temporary "g<" messages.
            let prev_skip = skip;
            skip -= 1;
            if (p.temp && eap.skip == 0) || prev_skip > 0 {
                continue;
            }
            if ui_has(K_UIMESSAGES) && g_st().msg_silent == 0 {
                let mut entry = Array::new();
                entry.push(Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_to_string(
                        p.kind.as_ref().map_or(ptr::null(), |k| k.as_ptr()),
                    )),
                });
                let mut content = Array::new();
                for chunk in &p.msg {
                    let mut content_entry = Array::new();
                    content_entry.push(Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(if chunk.hl_id != 0 {
                            syn_id2attr(chunk.hl_id) as Integer
                        } else {
                            0
                        }),
                    });
                    content_entry.push(Object {
                        obj_type: ObjectType::ObjectTypeString,
                        data: ObjectData::String(copy_string(chunk.text, None)),
                    });
                    content_entry.push(Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(chunk.hl_id as Integer),
                    });
                    content.push(Object {
                        obj_type: ObjectType::ObjectTypeArray,
                        data: ObjectData::Array(content_entry),
                    });
                }
                entry.push(Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(content),
                });
                entry.push(Object {
                    obj_type: ObjectType::ObjectTypeBoolean,
                    data: ObjectData::Boolean(p.append),
                });
                entries.push(Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(entry),
                });
            }
            if redirecting() != 0 || !ui_has(K_UIMESSAGES) {
                g_st().msg_silent += i32::from(ui_has(K_UIMESSAGES));
                let mut needs_clear = false;
                msg_multihl(
                    NIL_OBJ,
                    &mut p.msg,
                    p.kind.as_ref().map_or(ptr::null(), |k| k.as_ptr()),
                    false,
                    false,
                    None,
                    &mut needs_clear,
                    false,
                );
                g_st().msg_silent -= i32::from(ui_has(K_UIMESSAGES));
            }
        }
        if entries.size > 0 {
            ui_call_msg_history_show(entries, eap.skip != 0);
            api_free_array(entries);
        }
    }
}

/// Call this after prompting the user.  This will avoid a hit-return message
/// and a delay.
pub fn msg_end_prompt() {
    g_st().need_wait_return = false;
    g_st().emsg_on_display = false;
    g_st().cmdline_row = g_st().msg_row;
    g_st().msg_col = 0;
    msg_clr_eos();
    g_st().lines_left = -1;
}

/// Wait for the user to hit a key (normally Enter)
///
/// @param redraw  if true, redraw the entire screen UPD_NOT_VALID
///                if false, do a normal redraw
///                if -1, don't redraw at all
pub fn wait_return(redraw: i32) {
    {
        let mut c: i32;
        let mut had_got_int: bool;

        if redraw == 1 {
            redraw_all_later(UPD_NOT_VALID);
        }

        if ui_has(K_UIMESSAGES) {
            prompt_for_input(c"Press any key to continue", HLF_M, true, None);
            return;
        }

        // If using ":silent cmd", don't wait for a return.  Also don't set
        // need_wait_return to do it later.
        if g_st().msg_silent != 0 {
            return;
        }

        if (g_st().headless_mode && ui_active() == 0) || input_unanswerable() {
            return;
        }

        // When inside vgetc(), we can't wait for a typed character at all.
        if g_st().vgetc_busy > 0 {
            return;
        }
        g_st().need_wait_return = true;
        if g_st().no_wait_return != 0 {
            g_st().cmdline_row = g_st().msg_row;
            return;
        }

        g_st().redir_off = true; // don't redirect this message
        let old_state = g_st().state_global;
        if g_st().quit_more {
            c = CAR; // just pretend CR was hit
            g_st().quit_more = false;
            g_st().got_int = false;
        } else if !stuff_empty() {
            c = CAR;
        } else {
            g_st().state_global = MODE_HITRETURN;
            setmouse();
            g_st().cmdline_row = g_st().msg_row;
            if g_st().need_check_timestamps {
                check_timestamps(0);
            }

            // if cmdheight=0, we need to scroll in the first line of msg_grid upon the screen
            if opt_vars().p_ch == 0 && !ui_has(K_UIMESSAGES) && g_st().msg_scrolled == 0 {
                msg_grid_validate();
                msg_scroll_up(false, true);
                g_st().msg_scrolled += 1;
                g_st().cmdline_row = g_st().rows_global - 1;
            }

            if msg_st().msg_hit_enter {
                hit_return_msg(true);

                loop {
                    had_got_int = g_st().got_int;

                    g_st().no_mapping += 1;
                    g_st().allow_keys += 1;

                    let save_reg_recording = g_st().reg_recording;
                    let save_scriptout = g_st().scriptout.take();
                    g_st().reg_recording = 0;
                    c = safe_vgetc();
                    if had_got_int && g_st().global_busy == 0 {
                        g_st().got_int = false;
                    }
                    g_st().no_mapping -= 1;
                    g_st().allow_keys -= 1;
                    g_st().reg_recording = save_reg_recording;
                    g_st().scriptout = save_scriptout;

                    if g_st().key_typed && opt_vars().p_more != 0 {
                        if c == b'b' as i32
                            || c == CTRL_B_CONST
                            || c == b'k' as i32
                            || c == b'u' as i32
                            || c == b'g' as i32
                            || c == K_UP
                            || c == K_PAGEUP
                        {
                            if g_st().msg_scrolled > g_st().rows_global {
                                do_more_prompt(c);
                            } else {
                                g_st().msg_didout = false;
                                c = K_IGNORE;
                                g_st().msg_col = 0;
                            }
                            if g_st().quit_more {
                                c = CAR;
                                g_st().quit_more = false;
                                g_st().got_int = false;
                            } else if c != K_IGNORE {
                                c = K_IGNORE;
                                hit_return_msg(false);
                            }
                        } else if g_st().msg_scrolled > g_st().rows_global - 2
                            && (c == b'j' as i32
                                || c == b'd' as i32
                                || c == b'f' as i32
                                || c == CTRL_F_CONST
                                || c == K_DOWN
                                || c == K_PAGEDOWN)
                        {
                            c = K_IGNORE;
                        }
                    }

                    if !((had_got_int && c == CTRL_C_CONST)
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
                } else if !g_st().key_typed
                    || (!crate::strings::vim_haschr(c"\r\n ", c)
                        && c != CTRL_C_CONST
                        && c != b'q' as i32)
                {
                    requeue_key(g_st().vgetc_char, g_st().vgetc_mod_mask, 0, false);
                    g_st().do_redraw = true;
                }
            } else {
                c = CAR;
                do_sleep(i64::from(msg_st().msg_wait), true);
            }
        }
        g_st().redir_off = false;

        if c == b':' as i32 || c == b'?' as i32 || c == b'/' as i32 {
            g_st().cmdline_row = g_st().msg_row;
            g_st().skip_redraw = true;
            g_st().do_redraw = false;
        }

        let tmp_state = g_st().state_global;
        g_st().state_global = old_state;
        setmouse();
        msg_check();
        g_st().need_wait_return = false;
        g_st().did_wait_return = true;
        g_st().emsg_on_display = false;
        g_st().lines_left = -1;
        reset_last_sourcing();
        if g_st().keep_msg.as_ref().is_some_and(|km| {
            vim_strsize(km.as_ptr())
                >= (g_st().rows_global - g_st().cmdline_row - 1) * g_st().columns_global
                    + g_st().sc_col
        }) {
            g_st().keep_msg = None;
        }

        if tmp_state == MODE_SETWSIZE {
            ui_refresh();
        } else if !g_st().skip_redraw && (redraw == 1 || (g_st().msg_scrolled != 0 && redraw != -1))
        {
            redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
        }
    }
}

/// Write the hit-return prompt.
fn hit_return_msg(newline_sb: bool) {
    let save_p_more = opt_vars().p_more;

    if !newline_sb {
        opt_vars().p_more = 0;
    }
    if g_st().msg_didout {
        msg_putchar(NL);
    }
    opt_vars().p_more = 0;
    if g_st().got_int {
        msg_puts(gettext(c"Interrupt: "));
    }

    msg_puts_hl(
        gettext(c"Press ENTER or type command to continue"),
        HLF_R,
        false,
    );
    if msg_use_printf() == 0 {
        msg_clr_eos();
    }
    opt_vars().p_more = save_p_more;
}

/// Set "keep_msg" to "s".  Free the old value and check for NULL pointer.
pub fn set_keep_msg(s: impl crate::strings::IntoCCharPtr, hl_id: i32) {
    let s = s.into_c_char_ptr();
    {
        if ui_has(K_UIMESSAGES) {
            return;
        }

        if !s.is_null() && g_st().msg_silent == 0 {
            g_st().keep_msg = Some(crate::memory::cstr_to_cstring(s));
        } else {
            g_st().keep_msg = None;
        }
        msg_st().keep_msg_more = false;
        g_st().keep_msg_hl_id = hl_id;
    }
}

/// Return true if printing messages should currently be done.
pub fn messaging() -> bool {
    !(opt_vars().p_lz != 0 && char_avail() && !g_st().key_typed)
        && (opt_vars().p_ch > 0 || ui_has(K_UIMESSAGES))
}

pub fn msgmore(n: i32) {
    {
        if g_st().global_busy != 0 || !messaging() {
            return;
        }

        if g_st().keep_msg.is_some() && !msg_st().keep_msg_more {
            return;
        }

        let pn = n.abs();

        if i64::from(pn) > opt_vars().p_report {
            let msg_ptr = (&raw mut g_st().msg_buf).cast::<c_char>();
            if n > 0 {
                vim_snprintf!(
                    msg_ptr,
                    MSG_BUF_LEN,
                    ngettext(c"%d more line", c"%d more lines", pn as u64,),
                    pn,
                );
            } else {
                vim_snprintf!(
                    msg_ptr,
                    MSG_BUF_LEN,
                    ngettext(c"%d line less", c"%d fewer lines", pn as u64,),
                    pn,
                );
            }
            if g_st().got_int {
                {
                    let _dst: *mut c_char = msg_ptr;
                    let _dsize: usize = MSG_BUF_LEN;
                    let _dlen = crate::memory::cstr_len(_dst);
                    crate::memory::xstrlcpy(
                        _dst.wrapping_add(_dlen),
                        gettext(c" (Interrupted)"),
                        _dsize - _dlen,
                    );
                };
            }
            if msg(msg_ptr, 0) {
                set_keep_msg(msg_ptr, 0);
                msg_st().keep_msg_more = true;
            }
        }
    }
}

pub fn msg_ext_set_kind<S: crate::strings::IntoCCharPtr>(msg_kind: S) {
    let msg_kind = msg_kind.into_c_char_ptr();
    {
        msg_ext_ui_flush();
        msg_st().msg_ext_kind = msg_kind;
        msg_st().redir_col = if msg_st().msg_ext_append {
            msg_st().redir_col
        } else {
            0
        };

        if !msg_kind.is_null() && crate::memory::cstr_as_bytes(msg_kind) == b"list_cmd" {
            msg_ext_no_fast();
        }
    }
}

pub fn msg_ext_set_append(append: bool) {
    msg_ext_ui_flush();
    msg_st().msg_ext_append = append;
}

pub fn msg_ext_set_trigger(trigger: &std::ffi::CStr) {
    msg_ext_ui_flush();
    msg_st().msg_ext_trigger = trigger.as_ptr();
}

// Should be executed at all callsites emitting non-internal messages.
pub fn msg_ext_no_fast() {
    msg_ext_ui_flush();
    g_st().msg_ext_fast = false;
}

/// Prepare for outputting characters in the command line.
pub fn msg_start() {
    let mut did_return = false;

    g_st().msg_row = g_st().msg_row.max(g_st().cmdline_row);

    if g_st().msg_silent == 0 {
        g_st().keep_msg = None;
        g_st().need_fileinfo = false;
    }

    if g_st().need_highlight_changed {
        highlight_changed();
    }

    if g_st().need_clr_eos || (opt_vars().p_ch == 0 && g_st().redrawing_cmdline) {
        g_st().need_clr_eos = false;
        msg_clr_eos();
    }

    // if cmdheight=0, we need to scroll in the first line of msg_grid upon the screen
    if opt_vars().p_ch == 0 && !ui_has(K_UIMESSAGES) && g_st().msg_scrolled == 0 {
        msg_grid_validate();
        msg_scroll_up(false, true);
        g_st().msg_scrolled += 1;
        g_st().cmdline_row = g_st().rows_global - 1;
    }

    if g_st().msg_scroll == 0 && g_st().full_screen {
        g_st().msg_row = g_st().cmdline_row;
        g_st().msg_col = 0;
    } else if (g_st().msg_didout || opt_vars().p_ch == 0) && !ui_has(K_UIMESSAGES) {
        if ((opt_vars().p_ch == 0 && !g_st().msg_didout) || msg_st().msg_special_case_for_lua_print)
            && msg_use_printf() != 0
        {
            msg_puts_display(c"\n", 1, 0, 0);
        } else {
            msg_putchar(NL);
        }
        did_return = true;
        g_st().cmdline_row = g_st().msg_row;
    }
    msg_st().msg_special_case_for_lua_print = false;
    if !g_st().msg_didany || g_st().lines_left < 0 {
        msg_starthere();
    }
    if g_st().msg_silent == 0 {
        g_st().msg_didout = false;
    }

    if ui_has(K_UIMESSAGES) {
        msg_ext_ui_flush();
    }

    if !did_return {
        redir_write(c"\n", 1);
    }
}

/// Note that the current msg position is where messages start.
pub fn msg_starthere() {
    g_st().lines_left = g_st().cmdline_row;
    g_st().msg_didany = false;
}

pub fn msg_putchar(c: i32) {
    msg_putchar_hl(c, 0);
}

pub fn msg_putchar_hl(c: i32, hl_id: i32) {
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

pub fn msg_outnum(n: i32) {
    let mut buf = [0 as c_char; 20];
    vim_snprintf!(buf.as_mut_ptr(), buf.len(), c"%d", n);
    msg_puts(buf.as_ptr());
}

pub fn msg_home_replace(fname: impl crate::strings::IntoCCharPtr) {
    let fname = fname.into_c_char_ptr();
    msg_home_replace_hl(fname, 0);
}

fn msg_home_replace_hl(fname: impl crate::strings::IntoCCharPtr, hl_id: i32) {
    let fname = fname.into_c_char_ptr();
    let name = home_replace_save(None, fname);
    msg_outtrans(name.as_ptr(), hl_id, false);
}

/// Output "len" characters in "str" (including NULs) with translation
/// if "len" is -1, output up to a NUL character. Use highlight "hl_id".
///
/// @return  the number of characters it takes on the screen.
pub fn msg_outtrans(str_: impl crate::strings::IntoCCharPtr, hl_id: i32, hist: bool) -> i32 {
    let str_ = str_.into_c_char_ptr();
    msg_outtrans_len(str_, crate::memory::cstr_len(str_) as i32, hl_id, hist)
}

pub fn msg_outtrans_len(
    msgstr: impl crate::strings::IntoCCharPtr,
    mut len: i32,
    hl_id: i32,
    hist: bool,
) -> i32 {
    let msgstr = msgstr.into_c_char_ptr();
    {
        let mut retval = 0;
        let mut str_ = msgstr;
        let mut plain_start = msgstr;
        let save_got_int = g_st().got_int;

        // Only quit when got_int was set in here.
        g_st().got_int = false;

        if hist {
            msg_hist_add(str_, len, hl_id);
        }

        // When drawing over the command line no need to clear it later or remove
        // the mode message.
        if g_st().msg_silent == 0
            && len > 0
            && g_st().msg_row >= g_st().cmdline_row
            && g_st().msg_col == 0
        {
            g_st().clear_cmdline = false;
            g_st().mode_displayed = false;
        }

        // Go over the string.  Special characters are translated and printed.
        // Normal characters are printed several at a time.
        while {
            len -= 1;
            len >= 0 && !g_st().got_int
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
                        msg_puts_len(
                            plain_start,
                            crate::memory::ptr_offset_from(str_, plain_start),
                            hl_id,
                            hist,
                        );
                    }
                    plain_start = str_.wrapping_add(mb_l as usize);
                    msg_puts_hl(
                        transchar_buf(None, c),
                        if hl_id == 0 { HLF_8 } else { hl_id },
                        false,
                    );
                    retval += char2cells(c);
                }
                len -= mb_l - 1;
                str_ = str_.wrapping_add(mb_l as usize);
            } else {
                let s = transchar_byte_buf(None, i32::from(crate::memory::cstr_first_byte(str_)));
                if crate::memory::cstr_byte_at(s, 1) != 0 {
                    if str_ > plain_start {
                        msg_puts_len(
                            plain_start,
                            crate::memory::ptr_offset_from(str_, plain_start),
                            hl_id,
                            hist,
                        );
                    }
                    plain_start = str_.wrapping_add(1);
                    msg_puts_hl(s, if hl_id == 0 { HLF_8 } else { hl_id }, false);
                    retval += crate::memory::cstr_len(s) as i32;
                } else {
                    retval += 1;
                }
                str_ = str_.wrapping_add(1);
            }
        }

        if (str_ > plain_start || plain_start == msgstr) && !g_st().got_int {
            msg_puts_len(
                plain_start,
                crate::memory::ptr_offset_from(str_, plain_start),
                hl_id,
                hist,
            );
        }

        g_st().got_int |= save_got_int;

        retval
    }
}

pub fn msg_make(arg: impl crate::strings::IntoCCharPtr) {
    let mut arg = arg.into_c_char_ptr();
    {
        const STR: &[u8; 6] = b"eeffoc";
        const RS: &[u8; 15] = b"Plon#dqg#vxjduB";

        arg = skipwhite(arg);
        let mut i: isize = 5;
        while !crate::memory::cstr_is_empty(arg) && i >= 0 {
            if crate::memory::cstr_first_byte(arg) != STR[i as usize] {
                break;
            }
            arg = arg.wrapping_add(1);
            i -= 1;
        }
        if i < 0 {
            msg_putchar(NL);
            for &b in RS {
                msg_putchar(i32::from(b - 3));
            }
        }
    }
}

/// Output the string 'str' up to a NUL character.
/// Return the number of characters it takes on the screen.
pub fn msg_outtrans_special(
    strstart: impl crate::strings::IntoCCharPtr,
    from: bool,
    maxlen: i32,
) -> i32 {
    let strstart = strstart.into_c_char_ptr();
    {
        if strstart.is_null() {
            return 0;
        }
        let mut str_ = strstart;
        let mut retval = 0;
        let hl_id = HLF_8;

        while !crate::memory::cstr_is_empty(str_) {
            let mut text = if (str_ == strstart || crate::memory::cstr_byte_at(str_, 1) == 0)
                && crate::memory::cstr_first_byte(str_) == b' '
            {
                str_ = str_.wrapping_add(1);
                c"<Space>".as_ptr()
            } else {
                str2special(&raw mut str_, from, K_FALSE, None)
            };
            if !crate::memory::cstr_is_empty(text) && crate::memory::cstr_byte_at(text, 1) == 0 {
                text = transchar_byte_buf(None, i32::from(crate::memory::cstr_first_byte(text)));
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
pub fn str2special_save(
    str_: impl crate::strings::IntoCCharPtr,
    replace_spaces: bool,
    replace_others: TriState,
) -> std::ffi::CString {
    let str_ = str_.into_c_char_ptr();
    {
        let mut buf: Vec<u8> = Vec::with_capacity(40);
        let mut p = str_;
        while !crate::memory::cstr_is_empty(p) {
            let s = str2special(&raw mut p, replace_spaces, replace_others, None);
            if !s.is_null() {
                buf.extend_from_slice(crate::memory::cstr_as_bytes(s));
            }
        }
        crate::memory::cstr_from_bytes(&buf)
    }
}

/// Convert string, replacing key codes with printables
pub fn str2special_arena(
    str_: impl crate::strings::IntoCCharPtr,
    replace_spaces: bool,
    replace_others: TriState,
    arena: &mut Arena,
) -> *mut c_char {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        let mut p = str_;
        let mut len = 0usize;
        while !crate::memory::cstr_is_empty(p) {
            let s = str2special(&raw mut p, replace_spaces, replace_others, None);
            len += crate::memory::cstr_len(s);
        }

        let buf = arena_alloc(arena, len + 1, false).cast::<c_char>();
        let mut pos = 0usize;
        p = str_;
        while !crate::memory::cstr_is_empty(p) {
            let s = str2special(&raw mut p, replace_spaces, replace_others, None);
            let s_len = crate::memory::cstr_len(s);
            ptr::copy_nonoverlapping(s, buf.wrapping_add(pos), s_len);
            pos += s_len;
        }
        *buf.wrapping_add(pos) = NUL;
        buf
    }
}

/// Convert character, replacing key with printable representation.
pub fn str2special(
    sp: *mut *const c_char,
    replace_spaces: bool,
    replace_others: TriState,
    mut data: Option<&mut KeyChord>,
) -> *const c_char {
    unsafe {
        let p = mb_unescape(sp);
        if !p.is_null() {
            if data.is_some() {
                data.as_deref_mut().unwrap().mods = 0;
                data.as_deref_mut().unwrap().key = NvimString {
                    data: p.cast_mut(),
                    size: crate::memory::cstr_len(p),
                };
                data.as_deref_mut().unwrap().key_alt = NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                };
            }
            return p;
        }

        let mut str_ = *sp;
        let mut c = (crate::memory::cstr_first_byte(str_)) as i32;
        let mut modifiers = 0;
        let mut special = false;
        if c == K_SPECIAL as i32
            && crate::memory::cstr_byte_at(str_, 1) != 0
            && crate::memory::cstr_byte_at(str_, 2) != 0
        {
            if crate::memory::cstr_byte_at(str_, 1) == KS_MODIFIER {
                modifiers = (crate::memory::cstr_byte_at(str_, 2)) as i32;
                str_ = str_.wrapping_add(3);
                c = (crate::memory::cstr_first_byte(str_)) as i32;
            }
            if c == K_SPECIAL as i32
                && crate::memory::cstr_byte_at(str_, 1) != 0
                && crate::memory::cstr_byte_at(str_, 2) != 0
            {
                c = to_special(
                    crate::memory::cstr_byte_at(str_, 1) as i32,
                    crate::memory::cstr_byte_at(str_, 2) as i32,
                );
                str_ = str_.wrapping_add(2);
            }
            if is_special(c) || modifiers != 0 {
                special = true;
            }
        }

        if !is_special(c) && UTF8LEN_TAB[c as usize] > 1 {
            *sp = str_;
            let p = mb_unescape(sp);
            if !p.is_null() {
                c = utf_ptr2char(p);
            } else {
                *sp = str_.wrapping_add(1);
            }
        } else {
            *sp = str_.wrapping_add(usize::from(!crate::memory::cstr_is_empty(str_)));
        }

        if special
            || c < b' ' as i32
            || (replace_spaces && c == b' ' as i32)
            || (replace_others != K_FALSE && c == b'<' as i32)
            || (replace_others == K_TRUE && (c == b'|' as i32 || c == b'\\' as i32))
        {
            return get_special_key(
                c,
                modifiers,
                (data.is_some()).then(|| data.as_deref_mut().unwrap()),
            );
        }

        let buf_ptr = (&raw mut msg_st().str2_special_buf).cast::<c_char>();
        if data.is_some() {
            data.as_deref_mut().unwrap().mods = 0;
            if (b'A' as i32..=b'Z' as i32).contains(&c) {
                data.as_deref_mut().unwrap().key_mem = (c + 32) as c_char;
                data.as_deref_mut().unwrap().key = NvimString {
                    data: &raw mut data.as_deref_mut().unwrap().key_mem,
                    size: 1,
                };
                data.as_deref_mut().unwrap().mods |= i32::from(MOD_MASK_SHIFT);
                data.as_deref_mut().unwrap().key_alt = NvimString {
                    data: buf_ptr,
                    size: 1,
                };
            } else {
                data.as_deref_mut().unwrap().key = NvimString {
                    data: buf_ptr,
                    size: 1,
                };
                data.unwrap().key_alt = NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                };
            }
        }

        *buf_ptr = c as c_char;
        *buf_ptr.wrapping_add(1) = NUL;
        buf_ptr
    }
}

/// print line for :print or :list command
pub fn msg_prt_line(s: impl crate::strings::IntoCCharPtr, mut list: bool) {
    let mut s = s.into_c_char_ptr();
    unsafe {
        let mut sc: Schar;
        let mut col = 0;
        let mut n_extra = 0;
        let mut sc_extra: Schar = 0;
        let mut sc_final: Schar = 0;
        let mut p_extra: *const c_char = ptr::null();
        let mut hl_id = 0;
        let mut lead: *const c_char = ptr::null();
        let mut in_multispace: bool;
        let mut multispace_pos = 0usize;
        let mut trail: *const c_char = ptr::null();

        if curwin_mut().w_onebuf_opt.wo_list != 0 {
            list = true;
        }

        if list {
            // find start of trailing whitespace
            if curwin_mut().w_p_lcs_chars.trail != 0 {
                trail = s.wrapping_add(crate::memory::cstr_len(s));
                while trail > s && ascii_iswhite(*trail.wrapping_sub(1) as i32) {
                    trail = trail.wrapping_sub(1);
                }
            }
            // find end of leading whitespace
            if curwin_mut().w_p_lcs_chars.lead != 0
                || !curwin_mut().w_p_lcs_chars.leadmultispace.is_empty()
                || curwin_mut().w_p_lcs_chars.leadtab1 != 0
            {
                lead = s;
                while ascii_iswhite(crate::memory::cstr_first_char(lead)) {
                    lead = lead.wrapping_add(1);
                }
                if crate::memory::cstr_is_empty(lead) {
                    lead = ptr::null();
                }
            }
        }

        // output a space for an empty line, otherwise the line will be overwritten
        if crate::memory::cstr_is_empty(s) && !(list && curwin_mut().w_p_lcs_chars.eol != 0) {
            msg_putchar(b' ' as i32);
        }

        while !g_st().got_int {
            if n_extra > 0 {
                n_extra -= 1;
                if n_extra == 0 && sc_final != 0 {
                    sc = sc_final;
                } else if sc_extra != 0 {
                    sc = sc_extra;
                } else {
                    assert!(!p_extra.is_null());
                    sc = schar_from_ascii(crate::memory::cstr_first_byte(p_extra));
                    p_extra = p_extra.wrapping_add(1);
                }
            } else {
                let l = utfc_ptr2len(s);
                if l > 1 {
                    col += utf_ptr2cells(s);
                    let mut buf = [0 as c_char; MB_MAXBYTES + 1];
                    if l as usize >= MB_MAXBYTES {
                        xstrlcpy(buf.as_mut_ptr(), c"?", buf.len());
                    } else if curwin_mut().w_p_lcs_chars.nbsp != 0
                        && list
                        && (utf_ptr2char(s) == 160 || utf_ptr2char(s) == 0x202f)
                    {
                        schar_get(buf.as_mut_ptr(), curwin_mut().w_p_lcs_chars.nbsp);
                    } else {
                        ptr::copy(s, buf.as_mut_ptr(), l as usize);
                        buf[l as usize] = NUL;
                    }
                    msg_puts(buf.as_ptr());
                    s = s.wrapping_add(l as usize);
                    continue;
                } else {
                    hl_id = 0;
                    let c = (crate::memory::cstr_first_byte(s)) as i32;
                    s = s.wrapping_add(1);
                    if c >= 0x80 {
                        col += utf_char2cells(c);
                        msg_putchar(c);
                        continue;
                    }
                    sc_extra = 0;
                    sc_final = 0;
                    if list {
                        in_multispace = c == b' ' as i32
                            && (crate::memory::cstr_first_byte(s) == b' '
                                || (col > 0 && *s.wrapping_sub(2) == b' ' as c_char));
                        if !in_multispace {
                            multispace_pos = 0;
                        }
                    } else {
                        in_multispace = false;
                    }
                    if c == TAB && (!list || curwin_mut().w_p_lcs_chars.tab1 != 0) {
                        n_extra =
                            tabstop_padding(col, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array)
                                - 1;
                        if !list {
                            sc = schar_from_ascii(b' ');
                            sc_extra = schar_from_ascii(b' ');
                        } else {
                            let mut lcs_tab1 = curwin_mut().w_p_lcs_chars.tab1;
                            let mut lcs_tab2 = curwin_mut().w_p_lcs_chars.tab2;
                            let mut lcs_tab3 = curwin_mut().w_p_lcs_chars.tab3;
                            if !lead.is_null()
                                && s <= lead
                                && curwin_mut().w_p_lcs_chars.leadtab1 != 0
                            {
                                lcs_tab1 = curwin_mut().w_p_lcs_chars.leadtab1;
                                lcs_tab2 = curwin_mut().w_p_lcs_chars.leadtab2;
                                lcs_tab3 = curwin_mut().w_p_lcs_chars.leadtab3;
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
                    } else if c == NUL as i32 && list && curwin_mut().w_p_lcs_chars.eol != 0 {
                        p_extra = c"".as_ptr();
                        n_extra = 1;
                        sc = curwin_mut().w_p_lcs_chars.eol;
                        hl_id = HLF_AT;
                        s = s.wrapping_sub(1);
                    } else if c != NUL as i32 && {
                        let n = ascii2cells(c);
                        if n > 1 {
                            n_extra = n - 1;
                            p_extra = transchar_byte_buf(None, i32::from(c as u8));
                            sc = schar_from_ascii(crate::memory::cstr_first_byte(p_extra));
                            p_extra = p_extra.wrapping_add(1);
                            hl_id = HLF_0;
                            true
                        } else {
                            sc = 0;
                            false
                        }
                    } {
                        // handled in condition
                    } else if c == b' ' as i32 {
                        if !lead.is_null()
                            && s <= lead
                            && in_multispace
                            && !curwin_mut().w_p_lcs_chars.leadmultispace.is_empty()
                        {
                            sc = curwin_mut().w_p_lcs_chars.leadmultispace.as_slice()
                                [multispace_pos];
                            multispace_pos += 1;
                            if multispace_pos >= curwin_mut().w_p_lcs_chars.leadmultispace.len() {
                                multispace_pos = 0;
                            }
                            hl_id = HLF_0;
                        } else if !lead.is_null()
                            && s <= lead
                            && curwin_mut().w_p_lcs_chars.lead != 0
                        {
                            sc = curwin_mut().w_p_lcs_chars.lead;
                            hl_id = HLF_0;
                        } else if !trail.is_null() && s > trail {
                            sc = curwin_mut().w_p_lcs_chars.trail;
                            hl_id = HLF_0;
                        } else if in_multispace && !curwin_mut().w_p_lcs_chars.multispace.is_empty()
                        {
                            sc = curwin_mut().w_p_lcs_chars.multispace.as_slice()[multispace_pos];
                            multispace_pos += 1;
                            if multispace_pos >= curwin_mut().w_p_lcs_chars.multispace.len() {
                                multispace_pos = 0;
                            }
                            hl_id = HLF_0;
                        } else if list && curwin_mut().w_p_lcs_chars.space != 0 {
                            sc = curwin_mut().w_p_lcs_chars.space;
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
pub fn msg_puts<S: crate::strings::IntoCCharPtr>(s: S) {
    msg_puts_hl(s.into_c_char_ptr(), 0, false);
}

pub fn msg_puts_title(s: impl crate::strings::IntoCCharPtr) {
    let mut s = s.into_c_char_ptr();
    unsafe {
        if ui_has(K_UIMESSAGES) && *s == NL as c_char {
            s = s.wrapping_add(1);
        }
        msg_puts_hl(s, HLF_T, false);
    }
}

/// Show a message in such a way that it always fits in the line.  Cut out a
/// part in the middle and replace it with "..." when necessary.
/// Does not handle multi-byte characters!
pub fn msg_outtrans_long(longstr: impl crate::strings::IntoCCharPtr, hl_id: i32) {
    let longstr = longstr.into_c_char_ptr();
    {
        let len = crate::memory::cstr_len(longstr) as i32;
        let mut slen = len;
        let room = g_st().columns_global - g_st().msg_col;
        if !ui_has(K_UIMESSAGES) && len > room && room >= 20 {
            slen = (room - 3) / 2;
            msg_outtrans_len(longstr, slen, hl_id, false);
            msg_puts_hl(c"...", HLF_8, false);
        }
        msg_outtrans_len(
            longstr.wrapping_add((len - slen) as usize),
            slen,
            hl_id,
            false,
        );
    }
}

/// Basic function for writing a message with highlight id.
pub fn msg_puts_hl<S: crate::strings::IntoCCharPtr>(s: S, hl_id: i32, hist: bool) {
    msg_puts_len(s.into_c_char_ptr(), -1, hl_id, hist);
}

/// Write a message with highlight id.
pub fn msg_puts_len(str_: impl crate::strings::IntoCCharPtr, len: isize, hl_id: i32, hist: bool) {
    let str_ = str_.into_c_char_ptr();
    {
        assert!(len < 0 || !crate::memory::cstr_slice(str_, len as usize).contains(&0));
        // If redirection is on, also write to the redirection file.
        redir_write(str_, len);

        // Don't print anything when using ":silent cmd" or empty message.
        if g_st().msg_silent != 0 || crate::memory::cstr_is_empty(str_) {
            if crate::memory::cstr_is_empty(str_) && ui_has(K_UIMESSAGES) {
                msg_ext_no_fast();
                ui_call_msg_show(
                    cstr_as_string(c"empty"),
                    Array::new(),
                    false,
                    false,
                    false,
                    Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(-1),
                    },
                    NvimString {
                        data: ptr::null_mut(),
                        size: 0,
                    },
                );
                g_st().cmdline_was_last_drawn = false;
            }
            return;
        }

        if hist {
            msg_hist_add(str_, len as i32, hl_id);
        }

        let overflow =
            !ui_has(K_UIMESSAGES) && g_st().msg_scrolled > if opt_vars().p_ch == 0 { 1 } else { 0 };

        if overflow && !g_st().msg_scrolled_ign && crate::memory::cstr_as_bytes(str_) != b"\r" {
            g_st().need_wait_return = true;
        }
        g_st().msg_didany = true;

        if msg_use_printf() != 0 && !msg_st().msg_special_case_for_lua_print {
            let saved_msg_col = g_st().msg_col;
            msg_puts_printf(str_, len);
            if g_st().headless_mode {
                g_st().msg_col = saved_msg_col;
            }
        }
        if msg_use_printf() == 0
            || (g_st().headless_mode && crate::grid::grid_allocated(&g_st().default_grid))
        {
            msg_puts_display(str_, len as i32, hl_id, 0);
        }

        g_st().need_fileinfo = false;
    }
}

fn msg_ext_emit_chunk() {
    {
        if msg_st().msg_ext_last_attr == -1 {
            return;
        }
        let mut chunk = Array::new();
        chunk.push(Object {
            obj_type: ObjectType::ObjectTypeInteger,
            data: ObjectData::Integer(msg_st().msg_ext_last_attr as Integer),
        });
        msg_st().msg_ext_last_attr = -1;
        let text = crate::api::private::helpers::cbuf_to_string(
            msg_st().msg_ext_last_chunk.as_ptr().cast(),
            msg_st().msg_ext_last_chunk.len(),
        );
        msg_st().msg_ext_last_chunk = Vec::new();
        chunk.push(Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(text),
        });
        chunk.push(Object {
            obj_type: ObjectType::ObjectTypeInteger,
            data: ObjectData::Integer(msg_st().msg_ext_last_hl_id as Integer),
        });
        msg_st().msg_ext_chunks.push(Object {
            obj_type: ObjectType::ObjectTypeArray,
            data: ObjectData::Array(chunk),
        });
    }
}

/// The display part of msg_puts_len().
/// May be called recursively to display scroll-back text.
fn msg_puts_display(
    str_: impl crate::strings::IntoCCharPtr,
    maxlen: i32,
    hl_id: i32,
    recurse: i32,
) {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        let mut s = str_;
        let mut sb_str = str_;
        let mut sb_col = g_st().msg_col;
        let attr = if hl_id != 0 { syn_id2attr(hl_id) } else { 0 };

        g_st().did_wait_return = false;

        if ui_has(K_UIMESSAGES) {
            if attr != msg_st().msg_ext_last_attr {
                msg_ext_emit_chunk();
                msg_st().msg_ext_last_attr = attr;
                msg_st().msg_ext_last_hl_id = hl_id;
            }
            let len = if maxlen < 0 {
                crate::memory::cstr_len(str_)
            } else {
                xstrnlen(str_, maxlen as usize)
            };
            if len > 0 {
                msg_st()
                    .msg_ext_last_chunk
                    .extend_from_slice(std::slice::from_raw_parts(str_, len));
            }

            let lastline = {
                let _s = (str_).cast::<u8>();
                let _sz = len;
                if _sz == 0 {
                    std::ptr::null_mut::<std::ffi::c_void>()
                } else {
                    std::slice::from_raw_parts(_s, _sz)
                        .iter()
                        .rposition(|&_b| _b == b'\n')
                        .map_or(std::ptr::null_mut::<std::ffi::c_void>(), |_i| {
                            _s.wrapping_add(_i).cast_mut().cast::<std::ffi::c_void>()
                        })
                }
            }
            .cast::<c_char>();
            let p = if !lastline.is_null() {
                lastline.wrapping_add(1)
            } else {
                str_
            };
            let col =
                mb_string2cells_len(p, len - (crate::memory::ptr_offset_from(p, str_) as usize))
                    as i32;
            g_st().msg_col = (if !lastline.is_null() {
                0
            } else {
                g_st().msg_col
            }) + col;

            return;
        }

        let print_attr = hl_combine_attr(hl_attr(HLF_MSG), attr);
        msg_grid_validate();

        g_st().cmdline_was_last_drawn = g_st().redrawing_cmdline;

        let mut msg_row_pending = -1;

        loop {
            if g_st().msg_col >= g_st().columns_global {
                if opt_vars().p_more != 0 && recurse == 0 {
                    store_sb_text(&raw mut sb_str, s, hl_id, &mut sb_col, 1);
                }
                if g_st().msg_no_more && g_st().lines_left == 0 {
                    break;
                }

                g_st().msg_col = 0;
                g_st().msg_row += 1;
                g_st().msg_didout = false;
            }

            if g_st().msg_row >= g_st().rows_global {
                g_st().msg_row = g_st().rows_global - 1;

                if g_st().msg_no_more && g_st().lines_left == 0 {
                    break;
                }

                if recurse == 0 {
                    if msg_row_pending >= 0 {
                        msg_line_flush();
                        msg_row_pending = -1;
                    }

                    msg_scroll_up(true, false);

                    inc_msg_scrolled();
                    g_st().need_wait_return = true;
                    g_st().redraw_cmdline = true;
                    if g_st().cmdline_row > 0 {
                        g_st().cmdline_row -= 1;
                    }

                    if g_st().lines_left > 0 {
                        g_st().lines_left -= 1;
                    }

                    if opt_vars().p_more != 0
                        && g_st().lines_left == 0
                        && g_st().state_global != MODE_HITRETURN
                        && !g_st().msg_no_more
                    {
                        if do_more_prompt(NUL as i32) {
                            s = dlg_st().confirm_buttons.as_ptr();
                        }
                        if g_st().quit_more {
                            return;
                        }
                    }
                }
            }

            if !((maxlen < 0 || (crate::memory::ptr_offset_from(s, str_) as i32) < maxlen)
                && !crate::memory::cstr_is_empty(s))
            {
                break;
            }

            if g_st().msg_row != msg_row_pending
                && (crate::memory::cstr_first_byte(s) >= 0x20 || *s == TAB as c_char)
            {
                if msg_row_pending >= 0 {
                    msg_line_flush();
                }
                grid_line_start(&raw mut g_st().msg_grid_adj, g_st().msg_row);
                msg_row_pending = g_st().msg_row;
            }

            if crate::memory::cstr_first_byte(s) >= 0x20 {
                let mut cw = utf_ptr2cells(s);
                let l = if maxlen >= 0 {
                    utfc_ptr2len_len(
                        s,
                        crate::memory::ptr_offset_from(str_.wrapping_add(maxlen as usize), s)
                            as i32,
                    )
                } else {
                    utfc_ptr2len(s)
                };

                if cw > 1 && g_st().msg_col == g_st().columns_global - 1 {
                    grid_line_puts(g_st().msg_col, c">", 1, hl_attr(HLF_AT));
                    cw = 1;
                } else {
                    grid_line_puts(g_st().msg_col, s, l, print_attr);
                    s = s.wrapping_add(l as usize);
                }
                g_st().msg_didout = true;
                g_st().msg_col += cw;
            } else {
                let c = *s;
                s = s.wrapping_add(1);
                if c == NL as c_char {
                    g_st().msg_didout = false;
                    g_st().msg_col = 0;
                    g_st().msg_row += 1;
                    if opt_vars().p_more != 0 && recurse == 0 {
                        store_sb_text(&raw mut sb_str, s, hl_id, &mut sb_col, 1);
                    }
                } else if c == CAR as c_char {
                    g_st().msg_col = 0;
                } else if c == BS as c_char {
                    if g_st().msg_col != 0 {
                        g_st().msg_col -= 1;
                    }
                } else if c == TAB as c_char {
                    loop {
                        grid_line_puts(g_st().msg_col, c" ", 1, print_attr);
                        g_st().msg_col += 1;

                        if g_st().msg_col == g_st().columns_global || (g_st().msg_col & 7) == 0 {
                            break;
                        }
                    }
                } else if c == BELL as c_char {
                    vim_beep(K_OPT_BO_FLAG_SHELL);
                }
            }
        }

        if msg_row_pending >= 0 {
            msg_line_flush();
        }
        msg_cursor_goto(g_st().msg_row, g_st().msg_col);

        if opt_vars().p_more != 0 && recurse == 0 {
            store_sb_text(&raw mut sb_str, s, hl_id, &mut sb_col, 0);
        }

        msg_check();
    }
}

pub fn msg_line_flush() {
    if g_st().cmdmsg_rl {
        grid_line_mirror(g_st().msg_grid.cols);
    }
    grid_line_flush_if_valid_row();
}

pub fn msg_cursor_goto(mut row: i32, mut col: i32) {
    unsafe {
        if g_st().cmdmsg_rl {
            col = g_st().columns_global - 1 - col;
        }
        let grid = grid_adjust(&mut g_st().msg_grid_adj, &mut row, &mut col);
        ui_grid_cursor_goto((*grid).handle, row, col);
    }
}

/// @return  true when ":filter pattern" was used and "msg" does not match
///          "pattern".
pub fn message_filtered(msg: impl crate::strings::IntoCCharPtr) -> bool {
    let msg = msg.into_c_char_ptr();
    if g_st().cmdmod.cmod_filter_regmatch.regprog.is_null() {
        return false;
    }

    let match_ = vim_regexec(&mut g_st().cmdmod.cmod_filter_regmatch, msg, 0);
    if g_st().cmdmod.cmod_filter_force {
        match_
    } else {
        !match_
    }
}

/// including horizontal separator
pub fn msg_scrollsize() -> i32 {
    g_st().msg_scrolled
        + opt_vars().p_ch as i32
        + i32::from(opt_vars().p_ch > 0 || g_st().msg_scrolled > 1)
}

pub fn msg_do_throttle() -> bool {
    msg_use_grid() && (opt_vars().rdb_flags & K_OPT_RDB_FLAG_NOTHROTTLE) == 0
}

/// Scroll the screen up one line for displaying the next message line.
pub fn msg_scroll_up(may_throttle: bool, zerocmd: bool) {
    if may_throttle && msg_do_throttle() {
        g_st().msg_grid.throttled = true;
    }
    g_st().msg_did_scroll = true;
    if g_st().msg_grid_pos > 0 {
        msg_grid_set_pos(g_st().msg_grid_pos - 1, !zerocmd);

        if zerocmd && crate::grid::grid_allocated(&g_st().msg_grid) {
            grid_clear_line(
                &mut g_st().msg_grid,
                g_st().msg_grid.line_offset[0],
                g_st().msg_grid.cols,
                false,
            );
        }
    } else {
        grid_del_lines(
            &mut g_st().msg_grid,
            0,
            1,
            g_st().msg_grid.rows,
            0,
            g_st().msg_grid.cols,
        );
        for i in 0..(g_st().msg_grid.rows - 1) as usize {
            g_st().msg_grid.dirty_col[i] = g_st().msg_grid.dirty_col[i + 1];
        }
        g_st().msg_grid.dirty_col[(g_st().msg_grid.rows - 1) as usize] = 0;
    }

    grid_clear(
        &raw mut g_st().msg_grid_adj,
        g_st().rows_global - 1,
        g_st().rows_global,
        0,
        g_st().columns_global,
        hl_attr(HLF_MSG),
    );
}

/// Send throttled message output to UI clients
pub fn msg_scroll_flush() {
    if g_st().msg_grid.throttled {
        g_st().msg_grid.throttled = false;
        let pos_delta = msg_st().msg_grid_pos_at_flush - g_st().msg_grid_pos;
        assert!(pos_delta >= 0);
        let delta = (g_st().msg_scrolled - g_st().msg_scrolled_at_flush).min(g_st().msg_grid.rows);

        if pos_delta > 0 {
            ui_ext_msg_set_pos(g_st().msg_grid_pos, true);
        }

        let to_scroll = delta - pos_delta - g_st().msg_grid_scroll_discount;
        assert!(to_scroll >= 0);

        if to_scroll > 0 && g_st().msg_grid_pos == 0 {
            ui_call_grid_scroll(
                g_st().msg_grid.handle as Integer,
                0,
                g_st().rows_global as Integer,
                0,
                g_st().columns_global as Integer,
                to_scroll as Integer,
                0,
            );
        }

        for i in (g_st().rows_global - delta.max(1)).max(0)..g_st().rows_global {
            let row = i - g_st().msg_grid_pos;
            assert!(row >= 0);
            ui_line(
                &mut g_st().msg_grid,
                row,
                false,
                0,
                g_st().msg_grid.dirty_col[row as usize],
                g_st().msg_grid.cols,
                hl_attr(HLF_MSG),
                false,
            );
            g_st().msg_grid.dirty_col[row as usize] = 0;
        }
    }
    g_st().msg_scrolled_at_flush = g_st().msg_scrolled;
    g_st().msg_grid_scroll_discount = 0;
    msg_st().msg_grid_pos_at_flush = g_st().msg_grid_pos;
}

pub fn msg_reset_scroll() {
    if ui_has(K_UIMESSAGES) {
        return;
    }
    g_st().msg_grid.throttled = false;
    msg_grid_set_pos(g_st().rows_global - opt_vars().p_ch as i32, false);
    g_st().clear_cmdline = true;
    if crate::grid::grid_allocated(&g_st().msg_grid) {
        let limit = msg_scrollsize().min(g_st().msg_grid.rows);
        for i in 0..limit {
            grid_clear_line(
                &mut g_st().msg_grid,
                g_st().msg_grid.line_offset[i as usize],
                g_st().msg_grid.cols,
                false,
            );
        }
    }
    g_st().msg_scrolled = 0;
    g_st().msg_scrolled_at_flush = 0;
    g_st().msg_grid_scroll_discount = 0;
}

pub fn msg_ui_refresh() {
    if ui_has(K_UIMULTIGRID) && crate::grid::grid_allocated(&g_st().msg_grid) {
        ui_call_grid_resize(
            g_st().msg_grid.handle as Integer,
            g_st().msg_grid.cols as Integer,
            g_st().msg_grid.rows as Integer,
        );
        ui_ext_msg_set_pos(g_st().msg_grid_pos, g_st().msg_scrolled != 0);
    }
}

pub fn msg_ui_flush() {
    if ui_has(K_UIMULTIGRID)
        && crate::grid::grid_allocated(&g_st().msg_grid)
        && g_st().msg_grid.pending_comp_index_update
    {
        ui_ext_msg_set_pos(g_st().msg_grid_pos, g_st().msg_scrolled != 0);
    }
}

/// Increment "msg_scrolled".
fn inc_msg_scrolled() {
    {
        if crate::memory::cstr_is_empty(get_vim_var_str(VV_SCROLLSTART)) {
            let sname = sourcing_name();
            let mut tofree_vec: Vec<c_char>;

            let p = if sname.is_null() {
                cstr_as_string(gettext(c"Unknown"))
            } else {
                let tofreesize = crate::memory::cstr_len(sname) + 40;
                tofree_vec = vec![0; tofreesize];
                let tofree = tofree_vec.as_mut_ptr();
                let size = vim_snprintf_safelen!(
                    tofree,
                    tofreesize,
                    gettext(c"%s line %lld"),
                    sname,
                    i64::from(sourcing_lnum()),
                );
                NvimString { data: tofree, size }
            };
            set_vim_var_string(VV_SCROLLSTART, p.data, p.size as isize);
        }
        g_st().msg_scrolled += 1;
        set_must_redraw(UPD_VALID);
    }
}

/// Store part of a printed message for displaying when scrolling back.
fn store_sb_text(
    sb_str: *mut *const c_char,
    s: impl crate::strings::IntoCCharPtr,
    hl_id: i32,
    sb_col: &mut i32,
    finish: i32,
) {
    let s = s.into_c_char_ptr();
    unsafe {
        if msg_st().do_clear_sb_text == SbClear::SbClearAll
            || msg_st().do_clear_sb_text == SbClear::SbClearCmdlineDone
        {
            clear_sb_text(msg_st().do_clear_sb_text == SbClear::SbClearAll);
            msg_sb_eol();
            if msg_st().do_clear_sb_text == SbClear::SbClearCmdlineDone
                && s > *sb_str
                && **sb_str == NL as c_char
            {
                *sb_str = (*sb_str).wrapping_add(1);
            }
            msg_st().do_clear_sb_text = SbClear::SbClearNone;
        }

        if s > *sb_str {
            let text_len = crate::memory::ptr_offset_from(s, *sb_str) as usize;
            let mut text = Vec::with_capacity(text_len + 1);
            text.extend_from_slice(std::slice::from_raw_parts(*sb_str, text_len));
            text.push(NUL);
            msg_st().msgchunks.push(Msgchunk {
                sb_eol: finish as c_char,
                sb_msg_col: *sb_col,
                sb_hl_id: hl_id,
                sb_text: text,
            });
        } else if finish != 0
            && let Some(last) = msg_st().msgchunks.last_mut()
        {
            last.sb_eol = 1;
        }

        *sb_str = s;
        *sb_col = 0;
    }
}

/// Finished showing messages, clear the scroll-back text on the next message.
pub fn may_clear_sb_text() {
    msg_ext_ui_flush();
    msg_st().do_clear_sb_text = SbClear::SbClearAll;
    msg_st().do_clear_hist_temp = true;
}

/// Starting to edit the command line: do not clear messages now.
pub fn sb_text_start_cmdline() {
    if msg_st().do_clear_sb_text == SbClear::SbClearCmdlineBusy {
        sb_text_restart_cmdline();
    } else {
        msg_sb_eol();
        msg_st().do_clear_sb_text = SbClear::SbClearCmdlineBusy;
    }
}

/// Redrawing the command line: clear the last unfinished line.
pub fn sb_text_restart_cmdline() {
    {
        msg_st().do_clear_sb_text = SbClear::SbClearCmdlineBusy;

        let Some(last) = msg_st().msgchunks.last() else {
            return;
        };
        if last.sb_eol != 0 {
            return;
        }

        if let Some(start_idx) = msg_sb_start(Some(msg_st().msgchunks.len() - 1)) {
            msg_st().msgchunks.truncate(start_idx);
        }
    }
}

/// Ending to edit the command line: clear old lines but the last one later.
pub fn sb_text_end_cmdline() {
    msg_st().do_clear_sb_text = SbClear::SbClearCmdlineDone;
}

/// Clear any text remembered for scrolling back.
pub fn clear_sb_text(all: bool) {
    {
        if all {
            msg_st().msgchunks.clear();
        } else if !msg_st().msgchunks.is_empty()
            && let Some(start_idx) = msg_sb_start(Some(msg_st().msgchunks.len() - 1))
        {
            msg_st().msgchunks.drain(0..start_idx);
        }
    }
}

/// "g<" command.
pub fn show_sb_text() {
    {
        if ui_has(K_UIMESSAGES) {
            let mut ea: Exarg = Exarg::new();
            ea.arg = c"".as_ptr().cast_mut();
            ea.skip = 1;
            ex_messages(&mut ea);
            return;
        }
        let mp = msg_sb_start(msg_st().msgchunks.len().checked_sub(1));
        if mp.is_none_or(|idx| idx == 0) {
            vim_beep(K_OPT_BO_FLAG_MESS);
        } else {
            do_more_prompt(b'G' as i32);
            wait_return(0);
        }
    }
}

/// Move to the start of screen line in already displayed text.
fn msg_sb_start(mps: Option<usize>) -> Option<usize> {
    {
        let mut idx = mps?;
        while idx > 0 && msg_st().msgchunks.as_slice()[idx - 1].sb_eol == 0 {
            idx -= 1;
        }
        Some(idx)
    }
}

/// Mark the last message chunk as finishing the line.
pub fn msg_sb_eol() {
    {
        if let Some(last) = msg_st().msgchunks.last_mut() {
            last.sb_eol = 1;
        }
    }
}

/// Display a screen line from previously displayed text at row "row".
fn disp_sb_line(row: i32, smp: usize) -> Option<usize> {
    {
        let mut idx = smp;
        loop {
            g_st().msg_row = row;
            let chunk = &mut msg_st().msgchunks.as_mut_slice()[idx];
            g_st().msg_col = chunk.sb_msg_col;
            let p = chunk.sb_text.as_mut_ptr();
            let hl_id = chunk.sb_hl_id;
            let eol = chunk.sb_eol;
            msg_puts_display(p, -1, hl_id, 1);
            if eol != 0 || idx + 1 >= msg_st().msgchunks.len() {
                break;
            }
            idx += 1;
        }
        if idx + 1 < msg_st().msgchunks.len() {
            Some(idx + 1)
        } else {
            None
        }
    }
}

/// @return  true when messages should be printed to stdout/stderr
pub fn msg_use_printf() -> i32 {
    i32::from(!g_st().embedded_mode && ui_active() == 0 && !ui_has(K_UIMESSAGES))
}

/// Print a message when there is no valid screen.
fn msg_puts_printf(str_: impl crate::strings::IntoCCharPtr, maxlen: isize) {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        let mut s = str_;
        let mut buf = [0 as c_char; 7];

        if crate::eval::typval::Callback::raw_cb_type(&g_st().on_print) != K_CALLBACK_NONE_CONST {
            let mut argv = [TypVal {
                v_type: VarType::VarString,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::String(str_.cast_mut()),
            }];
            let mut rettv = TypVal {
                v_type: VarType::VarUnknown,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };
            callback_call(&mut g_st().on_print, 1, argv.as_mut_ptr(), &mut rettv);
            tv_clear(&mut rettv);
            return;
        }

        while (maxlen < 0 || crate::memory::ptr_offset_from(s, str_) < maxlen)
            && !crate::memory::cstr_is_empty(s)
        {
            let len = utf_ptr2len(s) as usize;
            if !(g_st().silent_mode && opt_vars().p_verbose == 0) {
                let mut p = buf.as_mut_ptr();
                if *s == NL as c_char
                    && !g_st().info_message
                    && !g_st().silent_mode
                    && !g_st().headless_mode
                {
                    *p = CAR as c_char;
                    p = p.wrapping_add(1);
                }
                ptr::copy_nonoverlapping(s, p, len);
                *p.wrapping_add(len) = NUL;
                if g_st().info_message {
                    printf!(c"%s", buf.as_ptr());
                } else {
                    fprintf!(log_get_stderr(), c"%s", buf.as_ptr());
                }
            }

            let cw = utf_char2cells(utf_ptr2char(s));
            if *s == CAR as c_char || *s == NL as c_char {
                g_st().msg_col = 0;
                g_st().msg_didout = false;
            } else {
                g_st().msg_col += cw;
                g_st().msg_didout = true;
            }
            s = s.wrapping_add(len);
        }
    }
}

/// Show the more-prompt and handle the user response.
fn do_more_prompt(typed_char: i32) -> bool {
    {
        let mut used_typed_char = typed_char;
        let old_state = g_st().state_global;
        let mut retval = false;
        let mut to_redraw = false;
        let mut mp_last: Option<usize> = None;

        let no_need_more = (g_st().headless_mode && !g_st().embedded_mode && ui_active() == 0)
            || input_unanswerable();

        if no_need_more
            || msg_st().do_more_prompt_entered
            || (g_st().state_global == MODE_HITRETURN && typed_char == 0)
        {
            return false;
        }
        msg_st().do_more_prompt_entered = true;

        if typed_char == b'G' as i32 {
            mp_last = msg_sb_start(msg_st().msgchunks.len().checked_sub(1));
            let mut i = 0;
            while i < g_st().rows_global - 2
                && let Some(idx) = mp_last
                && idx > 0
            {
                mp_last = msg_sb_start(Some(idx - 1));
                i += 1;
            }
        }

        g_st().state_global = MODE_ASKMORE;
        setmouse();
        if typed_char == NUL as i32 {
            msg_moremsg(false);
        }
        loop {
            let c = if used_typed_char != NUL as i32 {
                let ch = used_typed_char;
                used_typed_char = NUL as i32;
                ch
            } else {
                get_keystroke(g_st().resize_events)
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
                    toscroll = -(g_st().rows_global / 2);
                }
                0x64 /* 'd' */ => {
                    toscroll = g_st().rows_global / 2;
                }
                0x62 /* 'b' */ | CTRL_B_CONST | K_PAGEUP => {
                    toscroll = -(g_st().rows_global - 1);
                }
                0x20 /* ' ' */ | 0x66 /* 'f' */ | CTRL_F_CONST | K_PAGEDOWN | K_LEFTMOUSE => {
                    toscroll = g_st().rows_global - 1;
                }
                0x67 /* 'g' */ => {
                    toscroll = -999999;
                }
                0x47 /* 'G' */ => {
                    toscroll = 999999;
                    g_st().lines_left = 999999;
                }
                0x3a /* ':' */ => {
                    if msg_st().confirm_msg_used == 0 {
                        typeahead_noflush(b':' as i32);
                        g_st().cmdline_row = g_st().rows_global - 1;
                        g_st().skip_redraw = true;
                        g_st().need_wait_return = false;
                    }
                    if msg_st().confirm_msg_used != 0 {
                        retval = true;
                    } else {
                        g_st().got_int = true;
                        g_st().quit_more = true;
                    }
                    g_st().lines_left = g_st().rows_global - 1;
                }
                0x71 /* 'q' */ | CTRL_C_CONST | ESC => {
                    if msg_st().confirm_msg_used != 0 {
                        retval = true;
                    } else {
                        g_st().got_int = true;
                        g_st().quit_more = true;
                    }
                    g_st().lines_left = g_st().rows_global - 1;
                }
                K_EVENT => {
                    multiqueue_process_events( unsafe { &mut *g_st().resize_events });
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
                        msg_sb_start(msg_st().msgchunks.len().checked_sub(1))
                    };

                    let mut i = 0;
                    while i < g_st().rows_global - 2
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
                                mp_last = msg_sb_start(msg_st().msgchunks.len().checked_sub(1));
                            }
                            i -= 1;
                        }

                        if toscroll == -1 && !to_redraw {
                            grid_ins_lines(
                                &mut g_st().msg_grid,
                                0,
                                1,
                                g_st().rows_global,
                                0,
                                g_st().columns_global,
                            );
                            grid_clear(
                                &raw mut g_st().msg_grid_adj,
                                0,
                                1,
                                0,
                                g_st().columns_global,
                                hl_attr(HLF_MSG),
                            );
                            if let Some(cur_idx) = mp {
                                disp_sb_line(0, cur_idx);
                            }
                        } else {
                            grid_clear(
                                &raw mut g_st().msg_grid_adj,
                                0,
                                g_st().rows_global,
                                0,
                                g_st().columns_global,
                                hl_attr(HLF_MSG),
                            );
                            let mut i = 0;
                            while let Some(cur_idx) = mp
                                && i < g_st().rows_global - 1
                            {
                                mp = disp_sb_line(i, cur_idx);
                                g_st().msg_scrolled += 1;
                                i += 1;
                            }
                            to_redraw = false;
                        }
                        toscroll = 0;
                    }
                } else {
                    if g_st().cmdline_row >= g_st().rows_global && !ui_has(K_UIMESSAGES) {
                        msg_scroll_up(true, false);
                        g_st().msg_scrolled += 1;
                    }
                    while toscroll > 0
                        && let Some(last_idx) = mp_last
                    {
                        if msg_do_throttle() && !g_st().msg_grid.throttled {
                            g_st().msg_scrolled_at_flush -= 1;
                            g_st().msg_grid_scroll_discount += 1;
                        }
                        msg_scroll_up(true, false);
                        inc_msg_scrolled();
                        grid_clear(
                            &raw mut g_st().msg_grid_adj,
                            g_st().rows_global - 2,
                            g_st().rows_global - 1,
                            0,
                            g_st().columns_global,
                            hl_attr(HLF_MSG),
                        );
                        mp_last = disp_sb_line(g_st().rows_global - 2, last_idx);
                        toscroll -= 1;
                    }
                }

                if toscroll <= 0 {
                    grid_clear(
                        &raw mut g_st().msg_grid_adj,
                        g_st().rows_global - 1,
                        g_st().rows_global,
                        0,
                        g_st().columns_global,
                        hl_attr(HLF_MSG),
                    );
                    msg_moremsg(false);
                    continue;
                }

                g_st().lines_left = toscroll;
            }

            break;
        }

        // clear the --more-- message
        grid_clear(
            &raw mut g_st().msg_grid_adj,
            g_st().rows_global - 1,
            g_st().rows_global,
            0,
            g_st().columns_global,
            hl_attr(HLF_MSG),
        );
        g_st().redraw_cmdline = true;
        g_st().clear_cmdline = false;
        g_st().mode_displayed = false;

        g_st().state_global = old_state;
        setmouse();
        if g_st().quit_more {
            g_st().msg_row = g_st().rows_global - 1;
            g_st().msg_col = 0;
        }

        msg_st().do_more_prompt_entered = false;
        retval
    }
}

fn msg_moremsg(full: bool) {
    let attr = hl_combine_attr(hl_attr(HLF_MSG), hl_attr(HLF_M));
    grid_line_start(&raw mut g_st().msg_grid_adj, g_st().rows_global - 1);
    let mut len = grid_line_puts(0, gettext(c"-- More --"), -1, attr);
    if full {
        len += grid_line_puts(
            len,
            gettext(c" SPACE/d/j: screen/page/line down, b/u/k: up, q: quit "),
            -1,
            attr,
        );
    }
    grid_line_cursor_goto(len);
    grid_line_flush();
}

/// Repeat the message for the current mode: MODE_ASKMORE, MODE_EXTERNCMD,
/// or confirm() prompt.
pub fn repeat_message() {
    if ui_has(K_UIMESSAGES) {
        return;
    }

    if g_st().state_global == MODE_ASKMORE {
        msg_moremsg(true);
        g_st().msg_row = g_st().rows_global - 1;
    } else if (g_st().state_global & MODE_CMDLINE) != 0 && dlg_st().confirm_msg.is_some() {
        display_confirm_msg();
        g_st().msg_row = g_st().rows_global - 1;
    } else if g_st().state_global == MODE_EXTERNCMD {
        ui_cursor_goto(g_st().msg_row, g_st().msg_col);
    } else if g_st().state_global == MODE_HITRETURN || g_st().state_global == MODE_SETWSIZE {
        if g_st().msg_row == g_st().rows_global - 1 {
            g_st().msg_didout = false;
            g_st().msg_col = 0;
            msg_clr_eos();
        }
        hit_return_msg(false);
        g_st().msg_row = g_st().rows_global - 1;
    }
}

/// Clear from current message position to end of screen.
/// Skip this when ":silent" was used, no need to clear for redirection.
pub fn msg_clr_eos() {
    if g_st().msg_silent == 0 {
        msg_clr_eos_force();
    }
}

/// Clear from current message position to end of screen.
pub fn msg_clr_eos_force() {
    if ui_has(K_UIMESSAGES) {
        return;
    }
    let msg_startcol = if g_st().cmdmsg_rl { 0 } else { g_st().msg_col };
    let msg_endcol = if g_st().cmdmsg_rl {
        g_st().columns_global - g_st().msg_col
    } else {
        g_st().columns_global
    };

    if crate::grid::grid_allocated(&g_st().msg_grid) && g_st().msg_row < g_st().msg_grid_pos {
        msg_grid_validate();
        if g_st().msg_row < g_st().msg_grid_pos {
            g_st().msg_row = g_st().msg_grid_pos;
        }
    }

    grid_clear(
        &raw mut g_st().msg_grid_adj,
        g_st().msg_row,
        g_st().msg_row + 1,
        msg_startcol,
        msg_endcol,
        hl_attr(HLF_MSG),
    );
    grid_clear(
        &raw mut g_st().msg_grid_adj,
        g_st().msg_row + 1,
        g_st().rows_global,
        0,
        g_st().columns_global,
        hl_attr(HLF_MSG),
    );

    g_st().redraw_cmdline = true;
    if g_st().msg_row < g_st().rows_global - 1 || g_st().msg_col == 0 {
        g_st().clear_cmdline = false;
        g_st().mode_displayed = false;
        g_st().cmdline_was_last_drawn = false;
    }
}

/// Clear the command line.
pub fn msg_clr_cmdline() {
    g_st().msg_row = g_st().cmdline_row;
    g_st().msg_col = 0;
    msg_clr_eos_force();
}

/// end putting a message on the screen
/// call wait_return() if the message does not fit in the available space
///
/// @return  true if wait_return() not called.
pub fn msg_end() -> bool {
    if !g_st().exiting && g_st().need_wait_return && (g_st().state_global & MODE_CMDLINE) == 0 {
        wait_return(0);
        return false;
    }

    msg_ext_ui_flush();
    true
}

/// Clear "msg_ext_chunks" before flushing so that ui_flush() does not re-emit
/// the same message recursively.
fn msg_ext_init_chunks() -> Array {
    {
        let tofree = std::mem::take(&mut msg_st().msg_ext_chunks);
        g_st().msg_col = 0;
        tofree
    }
}

pub fn msg_ext_ui_flush() {
    unsafe {
        if !ui_has(K_UIMESSAGES) {
            msg_st().msg_ext_kind = ptr::null();
            return;
        } else if g_st().msg_ext_skip_flush {
            return;
        }

        msg_ext_emit_chunk();
        if msg_st().msg_ext_chunks.size > 0 {
            let tofree = msg_ext_init_chunks();

            ui_call_msg_show(
                cstr_as_string(msg_st().msg_ext_kind),
                tofree,
                g_st().msg_ext_overwrite,
                msg_st().msg_ext_history,
                msg_st().msg_ext_append,
                msg_st().msg_ext_id,
                cstr_as_string(msg_st().msg_ext_trigger),
            );
            if msg_st().msg_ext_history {
                api_free_array(tofree);
            } else {
                // Add to history as temporary message for "g<".
                let mut msg = HlMessage::new();
                for i in 0..tofree.size {
                    let chunk = tofree[i].data.array().items;
                    let text = std::mem::replace(
                        &mut (*chunk.wrapping_add(1)).data.string(),
                        NvimString {
                            data: ptr::null_mut(),
                            size: 0,
                        },
                    );
                    msg.push(HlMessageChunk {
                        text,
                        hl_id: (*chunk.wrapping_add(2)).data.integer() as i32,
                    });
                }
                api_free_array(tofree);
                msg_hist_add_multihl(msg, true);
            }
            g_st().msg_ext_overwrite = false;
            msg_st().msg_ext_history = false;
            msg_st().msg_ext_append = false;
            g_st().msg_ext_fast = true;
            msg_st().msg_ext_kind = ptr::null();
            if msg_st().msg_ext_id.obj_type == ObjectType::ObjectTypeInteger
                && msg_st().msg_ext_id.data.integer() == msg_st().msg_id_next
            {
                msg_st().msg_id_next += 1;
            }
            msg_st().msg_ext_id = Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer(msg_st().msg_id_next),
            };
        }
    }
}

pub fn msg_ext_flush_showmode() {
    if ui_has(K_UIMESSAGES) && (msg_st().msg_ext_last_attr != -1 || msg_st().showmode_clear) {
        msg_st().showmode_clear = msg_st().msg_ext_last_attr != -1;
        msg_ext_emit_chunk();
        let tofree = msg_ext_init_chunks();
        ui_call_msg_showmode(tofree);
        api_free_array(tofree);
    }
}

/// If the written message runs into the shown command or ruler, we have to
/// wait for hit-return and redraw the window later.
pub fn msg_check() {
    if ui_has(K_UIMESSAGES) {
        return;
    }
    if g_st().msg_row == g_st().rows_global - 1 && g_st().msg_col >= g_st().sc_col {
        g_st().need_wait_return = true;
        g_st().redraw_cmdline = true;
    }
}

/// May write a string to the redirection file.
fn redir_write(str_: impl crate::strings::IntoCCharPtr, maxlen: isize) {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        let mut s = str_;

        if maxlen == 0 {
            return;
        }

        if g_st().redir_off {
            return;
        }

        if !crate::memory::cstr_is_empty(opt_vars().p_vfile) && msg_st().verbose_fd.is_none() {
            verbose_open();
        }

        if redirecting() != 0 {
            if *s != NL as c_char && *s != CAR as c_char {
                while msg_st().redir_col < g_st().msg_col {
                    if !g_st().capture_ga.is_null() {
                        (*g_st().capture_ga).push(b' ' as c_char);
                    }
                    if g_st().redir_reg != 0 {
                        write_reg_contents(g_st().redir_reg, c" ", 1, 1);
                    } else if g_st().redir_vname {
                        var_redir_str(c" ", -1);
                    } else if let Some(ref rf) = g_st().redir_fd {
                        fputs(c" ", rf.as_ptr());
                    }
                    if let Some(ref vf) = msg_st().verbose_fd {
                        fputs(c" ", vf.as_ptr());
                    }
                    msg_st().redir_col += 1;
                }
            }

            let len = if maxlen == -1 {
                crate::memory::cstr_len(s)
            } else {
                maxlen as usize
            };
            if !g_st().capture_ga.is_null() && len > 0 {
                (*g_st().capture_ga).extend_from_slice(std::slice::from_raw_parts(str_, len));
            }
            if g_st().redir_reg != 0 {
                write_reg_contents(g_st().redir_reg, s, len as isize, 1);
            }
            if g_st().redir_vname {
                var_redir_str(s, maxlen as i32);
            }

            while (maxlen < 0 || (crate::memory::ptr_offset_from(s, str_) as i32) < maxlen as i32)
                && !crate::memory::cstr_is_empty(s)
            {
                if g_st().redir_reg == 0
                    && !g_st().redir_vname
                    && g_st().capture_ga.is_null()
                    && let Some(ref rf) = g_st().redir_fd
                {
                    fputc(crate::memory::cstr_first_char(s), rf.as_ptr());
                }
                if let Some(ref vf) = msg_st().verbose_fd {
                    fputc(crate::memory::cstr_first_char(s), vf.as_ptr());
                }
                if *s == CAR as c_char || *s == NL as c_char {
                    msg_st().redir_col = 0;
                } else if *s == TAB as c_char {
                    msg_st().redir_col += 8 - msg_st().redir_col % 8;
                } else {
                    msg_st().redir_col += 1;
                }
                s = s.wrapping_add(1);
            }

            if g_st().msg_silent != 0 {
                g_st().msg_col = msg_st().redir_col;
            }
        }
    }
}

pub fn redirecting() -> i32 {
    {
        i32::from(
            has_redir_fd()
                || !crate::memory::cstr_is_empty(opt_vars().p_vfile)
                || g_st().redir_reg != 0
                || g_st().redir_vname
                || !g_st().capture_ga.is_null(),
        )
    }
}

/// Before giving verbose message.
/// Must always be called paired with verbose_leave()!
pub fn verbose_enter() {
    {
        if !crate::memory::cstr_is_empty(opt_vars().p_vfile) {
            g_st().msg_silent += 1;
        }
        if !g_st().msg_ext_skip_verbose {
            if msg_st().msg_ext_kind != VERBOSE_KIND.as_ptr() {
                msg_st().pre_verbose_kind = msg_st().msg_ext_kind;
            }
            msg_ext_set_kind(VERBOSE_KIND);
        }
        g_st().msg_ext_skip_verbose = false;
    }
}

/// After giving verbose message.
/// Must always be called paired with verbose_enter()!
pub fn verbose_leave() {
    {
        if !crate::memory::cstr_is_empty(opt_vars().p_vfile) {
            g_st().msg_silent -= 1;
            if g_st().msg_silent < 0 {
                g_st().msg_silent = 0;
            }
        }
        if !msg_st().pre_verbose_kind.is_null() {
            msg_ext_set_kind(msg_st().pre_verbose_kind);
            msg_st().pre_verbose_kind = ptr::null();
        }
    }
}

/// Like verbose_enter() and set msg_scroll when displaying the message.
pub fn verbose_enter_scroll() {
    {
        verbose_enter();
        if crate::memory::cstr_is_empty(opt_vars().p_vfile) {
            g_st().msg_scroll = 1;
        }
    }
}

/// Like verbose_leave() and set cmdline_row when displaying the message.
pub fn verbose_leave_scroll() {
    {
        verbose_leave();
        if crate::memory::cstr_is_empty(opt_vars().p_vfile) {
            g_st().cmdline_row = g_st().msg_row;
        }
    }
}

/// Called when 'verbosefile' is set: stop writing to the file.
pub fn verbose_stop() {
    msg_st().verbose_fd = None;
    msg_st().verbose_did_open = false;
}

/// Open the file 'verbosefile'.
///
/// @return  FAIL or OK.
pub fn verbose_open() -> bool {
    if msg_st().verbose_fd.is_none() && !msg_st().verbose_did_open {
        msg_st().verbose_did_open = true;

        msg_st().verbose_fd = crate::os::fs::CFile::from_raw(os_fopen(opt_vars().p_vfile, c"a"));
        if msg_st().verbose_fd.is_none() {
            semsg!(gettext(E_NOTOPEN), opt_vars().p_vfile);
            return false;
        }
    }
    true
}

/// Give a warning message (for searching).
/// Use 'w' highlighting and may repeat the message after redrawing
pub fn give_warning(message: impl crate::strings::IntoCCharPtr, hl: bool, hist: bool) {
    let message = message.into_c_char_ptr();
    if g_st().msg_silent != 0 {
        return;
    }

    let save_msg_hist_off = g_st().msg_hist_off;
    g_st().msg_hist_off = !hist;

    g_st().no_wait_return += 1;

    set_vim_var_string(VV_WARNINGMSG, message, -1);
    g_st().keep_msg = None;
    if hl {
        g_st().keep_msg_hl_id = HLF_W;
    } else {
        g_st().keep_msg_hl_id = 0;
    }

    if msg_st().msg_ext_kind.is_null() {
        msg_ext_set_kind(c"wmsg");
    }

    if msg(message, g_st().keep_msg_hl_id) && g_st().msg_scrolled == 0 {
        set_keep_msg(message, g_st().keep_msg_hl_id);
    }
    g_st().msg_didout = false;
    g_st().msg_nowait = true;
    g_st().msg_col = 0;

    g_st().no_wait_return -= 1;
    g_st().msg_hist_off = save_msg_hist_off;
}

/// Advance msg cursor to column "col".
pub fn msg_advance(mut col: i32) {
    if g_st().msg_silent != 0 {
        g_st().msg_col = col;
        return;
    }
    col = col.min(g_st().columns_global - 1);
    while g_st().msg_col < col {
        msg_putchar(b' ' as i32);
    }
}

/// Display the ":confirm" message.  Also called from do_dialog() and when screen resized.
pub fn display_confirm_msg() {
    msg_st().confirm_msg_used += 1;
    if let Some(ref msg) = dlg_st().confirm_msg {
        msg_ext_set_kind(c"confirm");
        msg_puts_hl(msg.as_ptr(), HLF_M, false);
    }
    msg_st().confirm_msg_used -= 1;
}

/// Only for legacy UI (`!ui_has(K_UIMESSAGES)`): Pause to display a message for `ms` milliseconds.
pub fn msg_delay(mut ms: u64, ignoreinput: bool) {
    if ui_has(K_UIMESSAGES) {
        return;
    }

    if g_st().nvim_testing {
        ms = 100;
    }

    logmsg!(
        LOGLVL_DBG,
        ptr::null(),
        c"msg_delay",
        line!() as i32,
        true,
        c"%llu ms%s",
        ms,
        if g_st().nvim_testing {
            c" (skipped by NVIM_TEST)".as_ptr()
        } else {
            c"".as_ptr()
        },
    );
    ui_flush();
    os_delay(ms, ignoreinput);
}

/// Check if there should be a delay to allow the user to see a message.
pub fn msg_check_for_delay(check_msg_scroll: bool) {
    if (g_st().emsg_on_display || (check_msg_scroll && g_st().msg_scroll != 0))
        && !g_st().did_wait_return
        && g_st().emsg_silent == 0
        && !g_st().in_assert_fails
        && !ui_has(K_UIMESSAGES)
    {
        msg_delay(1006, true);
        g_st().emsg_on_display = false;
        if check_msg_scroll {
            g_st().msg_scroll = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trunc_string_in_place_and_by_copy() {
        let cases: &[(&str, &str, i32)] = &[
            ("text", "text", 20),
            ("a short text", "a short text", 20),
            ("a text that fits", "a text that fits", 34),
            ("a text tha just fits", "a text tha just fits", 20),
            ("a text that nott fits", "a text t...nott fits", 20),
        ];

        for &(s, expected, room) in cases {
            {
                let mut buf = [0 as c_char; 40];
                let c_s = std::ffi::CString::new(s).unwrap();
                trunc_string(c_s.as_ptr(), buf.as_mut_ptr(), room, 40);
                assert_eq!(
                    crate::memory::cstr_from_ptr(buf.as_ptr()).to_str().unwrap(),
                    expected
                );

                let mut inplace = [0 as c_char; 40];
                unsafe {
                    ptr::copy_nonoverlapping(c_s.as_ptr(), inplace.as_mut_ptr(), s.len() + 1)
                };
                trunc_string(inplace.as_ptr(), inplace.as_mut_ptr(), room, 40);
                assert_eq!(
                    crate::memory::cstr_from_ptr(inplace.as_ptr())
                        .to_str()
                        .unwrap(),
                    expected
                );
            }
        }
    }
}
