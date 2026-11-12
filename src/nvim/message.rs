//! Functions for displaying messages on the command line (`src/nvim/message.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    HLF_E, HlMessageChunk, api_free_array, copy_string, cstr_as_string, cstr_to_string,
};
use crate::api::private::validate::{
    Array, Dict, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::api::vim::{HLF_N, HlMessage, MessageData};
use crate::autocmd::{AUGROUP_ALL, EVENT_PROGRESS, HLF_8, HLF_T, apply_autocmds_group, has_event};
use crate::charset::{
    ascii_iswhite, ascii2cells, char2cells, is_special, to_special, transchar_buf,
    transchar_byte_buf, vim_isprintc, vim_strsize,
};
use crate::dialog::{clone_confirm_buttons, clone_confirm_msg, has_confirm_msg, prompt_for_input};
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
use crate::fileio::{check_timestamps, fprintf, printf};
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
    K_X2MOUSE, KS_MODIFIER, KeyChord, MOD_MASK_SHIFT, NL, SpecialKeyBuf, TAB, get_special_key,
};
use crate::log::{LOGLVL_DBG, LOGLVL_INF, log_get_stderr, logmsg};
use crate::main_init::input_unanswerable;
use crate::mbyte::{
    UTF8LEN_TAB, mb_string2cells_len, mb_unescape, utf_char2bytes, utf_char2cells, utf_ptr2cells,
    utf_ptr2char, utf_ptr2len, utf_slice2char, utfc_ptr2len, utfc_ptr2len_len,
};
use crate::mouse::{MOUSE_SETPOS, jump_to_mouse, setmouse};
use crate::msgpack_rpc::packer::Integer;
use crate::option::shortmess;
use crate::option_vars::*;
use crate::optionstr::{OptKeyDict_mopt, Optset, opt_keyset};
use crate::os::env::home_replace_save;
use crate::os::fs::CFile;
use crate::os::input::os_breakcheck;
use crate::os::time::os_delay;
use crate::regexp::vim_regexec;
use crate::register::write_reg_contents;
use crate::runtime::{ESTACK_NONE, estack_sfile, sourcing_lnum, sourcing_name};
use crate::state::{MODE_ASKMORE, MODE_CMDLINE, MODE_EXTERNCMD, MODE_HITRETURN, MODE_SETWSIZE};
use crate::strings::{format_to_buf, format_to_buf_safelen, gettext, ngettext};
use crate::ui::{
    ui_active, ui_call_grid_destroy, ui_call_grid_resize, ui_call_grid_scroll,
    ui_call_msg_history_show, ui_call_msg_set_pos, ui_call_msg_show, ui_call_msg_showmode,
    ui_cursor_goto, ui_flush, ui_grid_cursor_goto, ui_has, ui_line, ui_refresh, vim_beep,
};
use crate::ui_compositor::{ui_comp_put_grid, ui_comp_remove_grid};
use std::ffi::CStr;
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
    sb_eol: bool,
    sb_msg_col: i32,
    sb_hl_id: i32,
    sb_text: Vec<u8>,
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
    confirm_msg_used: std::cell::Cell<i32>,
    msg_hist_temp_idx: std::cell::Cell<Option<usize>>,
    msg_hist_len: std::cell::Cell<i32>,
    msg_hist_max: std::cell::Cell<i32>,
    msg_hit_enter: std::cell::Cell<bool>,
    msg_wait: std::cell::Cell<i32>,
    progress_msg_target: std::cell::Cell<i32>,
    verbose_did_open: std::cell::Cell<bool>,
    keep_msg_more: std::cell::Cell<bool>,
    msg_ext_id: crate::api::private::validate::CloneCell<MsgID>,
    msg_ext_chunks: crate::api::private::validate::CloneCell<Array>,
    msg_ext_last_attr: std::cell::Cell<Sattr>,
    msg_ext_last_hl_id: std::cell::Cell<i32>,
    msg_ext_history: std::cell::Cell<bool>,
    msg_ext_append: std::cell::Cell<bool>,
    msg_grid_pos_at_flush: std::cell::Cell<i32>,
    msg_special_case_for_lua_print: std::cell::Cell<bool>,
    msg_id_next: std::cell::Cell<i64>,
    is_multihl: std::cell::Cell<i32>,
    msg_keep_entered: std::cell::Cell<i32>,
    last_sourcing_lnum: std::cell::Cell<i32>,
    msg_source_recursive: std::cell::Cell<bool>,
    do_clear_hist_temp: std::cell::Cell<bool>,
    redir_col: std::cell::Cell<i32>,
    do_clear_sb_text: std::cell::Cell<SbClear>,
    do_more_prompt_entered: std::cell::Cell<bool>,
    showmode_clear: std::cell::Cell<bool>,
}

impl MessageState {
    const fn new() -> Self {
        Self {
            confirm_msg_used: std::cell::Cell::new(0),
            msg_hist_temp_idx: std::cell::Cell::new(None),
            msg_hist_len: std::cell::Cell::new(0),
            msg_hist_max: std::cell::Cell::new(500),
            msg_hit_enter: std::cell::Cell::new(true),
            msg_wait: std::cell::Cell::new(0),
            progress_msg_target: std::cell::Cell::new(PROGRESS_TARGET_CMD),
            verbose_did_open: std::cell::Cell::new(false),
            keep_msg_more: std::cell::Cell::new(false),
            msg_ext_id: crate::api::private::validate::CloneCell::new(Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer(1),
            }),
            msg_ext_chunks: crate::api::private::validate::CloneCell::new(Array::new()),
            msg_ext_last_attr: std::cell::Cell::new(-1),
            msg_ext_last_hl_id: std::cell::Cell::new(0),
            msg_ext_history: std::cell::Cell::new(false),
            msg_ext_append: std::cell::Cell::new(false),
            msg_grid_pos_at_flush: std::cell::Cell::new(0),
            msg_special_case_for_lua_print: std::cell::Cell::new(false),
            msg_id_next: std::cell::Cell::new(1),
            is_multihl: std::cell::Cell::new(0),
            msg_keep_entered: std::cell::Cell::new(0),
            last_sourcing_lnum: std::cell::Cell::new(0),
            msg_source_recursive: std::cell::Cell::new(false),
            do_clear_hist_temp: std::cell::Cell::new(true),
            redir_col: std::cell::Cell::new(0),
            do_clear_sb_text: std::cell::Cell::new(SbClear::SbClearNone),
            do_more_prompt_entered: std::cell::Cell::new(false),
            showmode_clear: std::cell::Cell::new(false),
        }
    }
}

struct MessageExtState {
    msg_hist: std::cell::RefCell<std::collections::VecDeque<MessageHistoryEntry>>,
    verbose_fd: std::cell::RefCell<Option<crate::os::fs::CFile>>,
    msg_ext_kind: std::cell::RefCell<Option<std::ffi::CString>>,
    msg_ext_trigger: std::cell::RefCell<Option<std::ffi::CString>>,
    msg_ext_last_chunk: std::cell::RefCell<Vec<u8>>,
    last_sourcing_name: std::cell::RefCell<Option<std::ffi::CString>>,
    msgchunks: std::cell::RefCell<Vec<Msgchunk>>,
    pre_verbose_kind: std::cell::RefCell<Option<std::ffi::CString>>,
}

impl MessageExtState {
    const fn new() -> Self {
        Self {
            msg_hist: std::cell::RefCell::new(std::collections::VecDeque::new()),
            verbose_fd: std::cell::RefCell::new(None),
            msg_ext_kind: std::cell::RefCell::new(None),
            msg_ext_trigger: std::cell::RefCell::new(None),
            msg_ext_last_chunk: std::cell::RefCell::new(Vec::new()),
            last_sourcing_name: std::cell::RefCell::new(None),
            msgchunks: std::cell::RefCell::new(Vec::new()),
            pre_verbose_kind: std::cell::RefCell::new(None),
        }
    }
}

thread_local! {
    static MSG_STATE: MessageState = const { MessageState::new() };
    static MSG_EXT: MessageExtState = const { MessageExtState::new() };
}

#[inline]
fn with_msg_ext<R>(f: impl FnOnce(&MessageExtState) -> R) -> R {
    MSG_EXT.with(f)
}

#[macro_export]
macro_rules! smsg {
    ($hl_id:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let hl = $hl_id;
        {
            match (
                $crate::strings::IntoFmtVal::into_fmt_val(&$fmt),
                [$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
            ) {
                (ref fmt_val, ref args_val) => {
                    $crate::strings::format_to_buf_fmt(
                        (&mut *$crate::globals::g_st().iobuff_bytes_mut()),
                        fmt_val.as_cstr(),
                        args_val,
                    );
                }
            }
            $crate::message::msg(
                &$crate::globals::g_st().iobuff_cstr(),
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
        {
            match (
                $crate::strings::IntoFmtVal::into_fmt_val(&$fmt),
                [$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
            ) {
                (ref fmt_val, ref args_val) => {
                    $crate::strings::format_to_buf_fmt(
                        (&mut *$crate::globals::g_st().iobuff_bytes_mut()),
                        fmt_val.as_cstr(),
                        args_val,
                    );
                }
            }
            $crate::message::msg_keep(
                &$crate::globals::g_st().iobuff_cstr(),
                hl,
                true,
                false,
            )
        }
    }};
}
pub use smsg_keep;

#[inline(never)]
pub fn semsg_fmt(fmt: &std::ffi::CStr, args: &[crate::strings::FmtVal]) -> bool {
    if emsg_not_now() {
        return true;
    }
    let mut errbuf = [0u8; 1025];
    crate::strings::format_to_buf_fmt(&mut errbuf, Some(fmt), args);
    emsg(crate::memory::bytes_as_cstr(&errbuf))
}

#[inline(never)]
pub fn semsg_multiline_fmt(
    kind: &std::ffi::CStr,
    fmt: &std::ffi::CStr,
    args: &[crate::strings::FmtVal],
) -> bool {
    if emsg_not_now() {
        return true;
    }
    let mut errbuf = [0u8; 8192];
    crate::strings::format_to_buf_fmt(&mut errbuf, Some(fmt), args);
    emsg_multiline(crate::memory::bytes_as_cstr(&errbuf), kind, 6, true)
}

#[macro_export]
macro_rules! semsg {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        $crate::message::semsg_fmt(
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
        )
    }};
}
pub use semsg;

#[macro_export]
macro_rules! semsg_multiline {
    ($kind:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        $crate::message::semsg_multiline_fmt(
            $kind,
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
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
        {
            match (
                $crate::strings::IntoFmtVal::into_fmt_val(&$fmt),
                [$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
            ) {
                (ref fmt_val, ref args_val) => {
                    $crate::strings::format_to_buf_fmt(
                        (&mut *$crate::globals::g_st().iobuff_bytes_mut()),
                        fmt_val.as_cstr(),
                        args_val,
                    );
                }
            }
            $crate::message::msg_schedule_semsg_cstr(
                &$crate::globals::g_st().iobuff_cstr(),
            );
        }
    }};
}
pub use msg_schedule_semsg;

#[macro_export]
macro_rules! msg_schedule_semsg_multiline {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {{
        {
            match (
                $crate::strings::IntoFmtVal::into_fmt_val(&$fmt),
                [$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
            ) {
                (ref fmt_val, ref args_val) => {
                    $crate::strings::format_to_buf_fmt(
                        (&mut *$crate::globals::g_st().iobuff_bytes_mut()),
                        fmt_val.as_cstr(),
                        args_val,
                    );
                }
            }
            $crate::message::msg_schedule_semsg_multiline_cstr(
                &$crate::globals::g_st().iobuff_cstr(),
            );
        }
    }};
}
pub use msg_schedule_semsg_multiline;

#[macro_export]
macro_rules! swmsg {
    ($hl:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {{
        let hl_val: bool = $hl;
        {
            match (
                $crate::strings::IntoFmtVal::into_fmt_val(&$fmt),
                [$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
            ) {
                (ref fmt_val, ref args_val) => {
                    $crate::strings::format_to_buf_fmt(
                        (&mut *$crate::globals::g_st().iobuff_bytes_mut()),
                        fmt_val.as_cstr(),
                        args_val,
                    );
                }
            }
            $crate::message::give_warning(
                &$crate::globals::g_st().iobuff_cstr(),
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
use crate::ex_cmds::{CTRL_C_CONST, HLF_R};
use crate::ex_getln::{CTRL_B_CONST, CTRL_F_CONST, K_EVENT};
pub use swmsg;

use crate::keycodes::{k_second, k_third};

#[inline]
fn have_sourcing_info() -> bool {
    !crate::runtime::estack_is_empty()
}

/// Returns true if the given integer message-id was previously generated.
pub fn msg_id_exists(id: i64) -> bool {
    id > 0 && id < MSG_STATE.with(|s| s.msg_id_next.get())
}

fn ui_ext_msg_set_pos(row: i32, scrolled: bool) {
    let mut buf = [0u8; MAX_SCHAR_SIZE];
    let size = schar_get(&mut buf, curwin_mut().w_p_fcs_chars.get().msgsep);
    ui_call_msg_set_pos(
        g_st().msg_grid.handle.get() as Integer,
        row as Integer,
        scrolled,
        NvimString::from_bytes_len(&buf, size),
        g_st().msg_grid.zindex.get() as Integer,
        g_st().msg_grid.comp_index.get() as Integer,
    );
    g_st().msg_grid.pending_comp_index_update.set(false);
}

pub fn msg_grid_set_pos(row: i32, scrolled: bool) {
    if !g_st().msg_grid.throttled.get() {
        ui_ext_msg_set_pos(row, scrolled);
        MSG_STATE.with(|s| s.msg_grid_pos_at_flush.set(row));
    }
    g_st().msg_grid_pos.set(row);
    if crate::grid::grid_allocated(&g_st().msg_grid) {
        g_st().msg_grid_adj.row_offset.set(-row);
    }
}

pub fn msg_use_grid() -> bool {
    crate::grid::grid_allocated(&g_st().default_grid) && !ui_has(K_UIMESSAGES)
}

pub fn msg_grid_validate() {
    grid_assign_handle(&g_st().msg_grid);
    let should_alloc = msg_use_grid();
    let max_rows = g_st().rows_global.get() - opt_vars().p_ch.get() as i32;
    let msg_grid_empty = !crate::grid::grid_allocated(&g_st().msg_grid);
    if should_alloc
        && (g_st().msg_grid.rows.get() != g_st().rows_global.get()
            || g_st().msg_grid.cols.get() != g_st().columns_global.get()
            || msg_grid_empty)
    {
        grid_alloc(
            &g_st().msg_grid,
            g_st().rows_global.get(),
            g_st().columns_global.get(),
            false,
            true,
        );
        g_st().msg_grid.zindex.set(K_ZINDEX_MESSAGES);

        *g_st().msg_grid.dirty_col.borrow_mut() = vec![0; g_st().rows_global.get() as usize];

        // Tricky: allow resize while pager or ex mode is active
        let pos = if (g_st().state_global.get() & MODE_ASKMORE) != 0 {
            0
        } else {
            (max_rows - g_st().msg_scrolled.get()).max(0)
        };
        g_st().msg_grid.throttled.set(false); // don't throttle in 'cmdheight' area
        msg_grid_set_pos(pos, g_st().msg_scrolled.get() != 0);
        ui_comp_put_grid(
            &g_st().msg_grid,
            pos,
            0,
            g_st().msg_grid.rows.get(),
            g_st().msg_grid.cols.get(),
            false,
            true,
        );
        ui_call_grid_resize(
            g_st().msg_grid.handle.get() as Integer,
            g_st().msg_grid.cols.get() as Integer,
            g_st().msg_grid.rows.get() as Integer,
        );

        g_st().msg_scrolled_at_flush.set(g_st().msg_scrolled.get());
        g_st().msg_grid.mouse_enabled.set(false);
        g_st().msg_grid_adj.target.set(crate::grid::GridTarget::Msg);
    } else if !should_alloc && !msg_grid_empty {
        ui_comp_remove_grid(&g_st().msg_grid);
        grid_free(&g_st().msg_grid);
        *g_st().msg_grid.dirty_col.borrow_mut() = Vec::new();
        ui_call_grid_destroy(g_st().msg_grid.handle.get() as Integer);
        g_st().msg_grid.throttled.set(false);
        g_st().msg_grid_adj.row_offset.set(0);
        g_st()
            .msg_grid_adj
            .target
            .set(crate::grid::GridTarget::Default);
        g_st().redraw_cmdline.set(true);
    } else if !msg_grid_empty
        && g_st().msg_scrolled.get() == 0
        && g_st().msg_grid_pos.get() != max_rows
    {
        let diff = g_st().msg_grid_pos.get() - max_rows;
        msg_grid_set_pos(max_rows, false);
        if diff > 0 {
            grid_clear(
                &g_st().msg_grid_adj,
                g_st().rows_global.get() - diff,
                g_st().rows_global.get(),
                0,
                g_st().columns_global.get(),
                hl_attr(HLF_MSG),
            );
        }
    }

    if crate::grid::grid_allocated(&g_st().msg_grid)
        && g_st().msg_scrolled.get() == 0
        && g_st().cmdline_row.get() < g_st().msg_grid_pos.get()
    {
        g_st().cmdline_row.set(g_st().msg_grid_pos.get());
    }
}

/// Like msg() but keep it silent when 'verbosefile' is set.
pub fn verb_msg(s: &std::ffi::CStr) -> i32 {
    verbose_enter();
    let n = i32::from(msg_keep(s, 0, false, false));
    verbose_leave();
    n
}

/// Displays the string 's' on the status line
/// When terminal not initialized (yet) printf("%s", ..) is used.
///
/// @return  true if wait_return() not called
pub fn msg(s: &std::ffi::CStr, hl_id: i32) -> bool {
    msg_keep(s, hl_id, false, false)
}

/// Similar to msg_outtrans_len, but support newlines and tabs.
pub fn msg_multiline(
    str_: NvimString,
    hl_id: i32,
    check_int: bool,
    hist: bool,
    need_clear: &mut bool,
) {
    let bytes = str_.as_bytes();
    let mut chunk_start = 0usize;
    for (idx, &b) in bytes.iter().enumerate() {
        if check_int && g_st().got_int.get() {
            return;
        }
        if b == b'\n' || i32::from(b) == TAB || b == b'\r' || b == BELL as u8 {
            // Print all chars before the delimiter
            let chunk_len = idx - chunk_start;
            msg_outtrans_len(&bytes[chunk_start..], chunk_len as i32, hl_id, hist);

            if i32::from(b) != TAB && *need_clear {
                msg_clr_eos();
                *need_clear = false;
            }
            if b == BELL as u8 {
                vim_beep(K_OPT_BO_FLAG_SHELL);
            } else {
                msg_putchar_hl(i32::from(b), hl_id);
            }
            chunk_start = idx + 1;
        }
    }

    // Print the remainder or emit empty event if entire message is empty.
    let rem = bytes.len() - chunk_start;
    if rem > 0 || chunk_start == 0 {
        msg_outtrans_len(&bytes[chunk_start..], rem as i32, hl_id, hist);
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
    // progress messages are special. displayed as "title: percent% msg"
    if msg_data.as_deref_mut().unwrap().title.size != 0 {
        // this block draws the "title:" before the progress-message
        let mut hl_id = 0;
        if msg_data.as_deref_mut().unwrap().status.data.is_null() {
            hl_id = 0;
        } else if msg_data.as_deref_mut().unwrap().status.opt_bytes() == Some(b"success") {
            hl_id = syn_check_group(b"OkMsg");
        } else if msg_data.as_deref_mut().unwrap().status.opt_bytes() == Some(b"failed") {
            hl_id = syn_check_group(b"ErrorMsg");
        } else if msg_data.as_deref_mut().unwrap().status.opt_bytes() == Some(b"running") {
            hl_id = syn_check_group(b"MoreMsg");
        } else if msg_data.as_deref_mut().unwrap().status.opt_bytes() == Some(b"cancel") {
            hl_id = syn_check_group(b"WarningMsg");
        }
        updated_msg.push(HlMessageChunk {
            text: copy_string(msg_data.as_deref_mut().unwrap().title.clone(), None),
            hl_id,
        });
        updated_msg.push(HlMessageChunk {
            text: cstr_to_string(Some(c": ")),
            hl_id: 0,
        });
    }
    if msg_data.as_deref_mut().unwrap().percent >= 0 {
        let mut percent_buf = [0u8; 10];
        format_to_buf!(&mut percent_buf, c"%3ld%% ", msg_data.unwrap().percent);
        let percent = cstr_to_string(Some(crate::memory::bytes_as_cstr(&percent_buf)));
        let hl_id = syn_check_group(b"WarningMsg");
        updated_msg.push(HlMessageChunk {
            text: percent,
            hl_id,
        });
    }

    if !updated_msg.is_empty() {
        for chunk in hl_msg {
            updated_msg.push(HlMessageChunk {
                text: copy_string(chunk.text.clone(), None),
                hl_id: chunk.hl_id,
            });
        }
        return true;
    }
    false
}

/// Print message chunks, each with their own highlight ID.
pub fn msg_multihl(
    mut id: MsgID,
    hl_msg: &mut HlMessage,
    kind: Option<&std::ffi::CStr>,
    history: bool,
    err: bool,
    mut msg_data: Option<&mut MessageData>,
    needs_msg_clear: &mut bool,
    raw_for_lua_print: bool,
) -> MsgID {
    if id.obj_type == ObjectType::ObjectTypeNil {
        let next_id = MSG_STATE.with(|s| s.msg_id_next.get());
        MSG_STATE.with(|s| s.msg_id_next.set(s.msg_id_next.get() + (1)));
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
    if kind.is_some_and(|k| k.to_bytes() == b"progress") {
        do_autocmd_progress(id.clone(), hl_msg, msg_data.as_deref_mut());
        if (MSG_STATE.with(|s| s.progress_msg_target.get()) & PROGRESS_TARGET_CMD) == 0 {
            *needs_msg_clear = true;
            return id;
        }
        if msg_data.is_some() && format_progress_message(hl_msg, msg_data, &mut updated_msg) {
            *needs_msg_clear = true;
            hl_msg_updated = true;
        }
    }

    let active_msg: &HlMessage = if hl_msg_updated { &updated_msg } else { hl_msg };

    g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
    msg_start();
    msg_clr_eos();
    let mut need_clear = false;
    if let Some(kind_cstr) = kind {
        msg_ext_set_kind(kind_cstr);
    }
    if raw_for_lua_print {
        MSG_STATE.with(|s| s.msg_special_case_for_lua_print.set(true));
        assert!(active_msg.len() == 1);
        let chunk = &active_msg[0];
        printf!(c"%.*s\n", chunk.text.size as i32, chunk.text,);
    }

    g_st().msg_ext_skip_flush.set(true);
    MSG_STATE.with(|s| s.msg_ext_id.set(id.clone()));

    for chunk in active_msg {
        MSG_STATE.with(|s| s.is_multihl.set(s.is_multihl.get() + (1)));
        if err {
            emsg_multiline(chunk.text.as_cstr(), kind.unwrap_or(c""), chunk.hl_id, true);
        } else {
            msg_multiline(
                chunk.text.clone(),
                chunk.hl_id,
                true,
                false,
                &mut need_clear,
            );
        }
        assert!(
            !ui_has(K_UIMESSAGES)
                || kind.is_none_or(|k| {
                    with_msg_ext(|ext| ext.msg_ext_kind.borrow().as_deref() == Some(k))
                })
        );
    }

    let has_chunks = !active_msg.is_empty();
    if history && has_chunks {
        if hl_msg_updated {
            msg_hist_add_multihl(std::mem::take(&mut updated_msg), false);
        } else {
            msg_hist_add_multihl(std::mem::take(hl_msg), false);
        }
    }

    g_st().msg_ext_skip_flush.set(false);
    MSG_STATE.with(|s| s.is_multihl.set(0));
    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    msg_end();

    // Release the id: it belongs to this message, and a String id only borrows the caller's storage
    // (often a stack buffer). Redundant unless nothing was emitted (e.g. 'msg_silent').
    let next_id = MSG_STATE.with(|s| s.msg_id_next.get());
    MSG_STATE.with(|s| {
        s.msg_ext_id.set(Object {
            obj_type: ObjectType::ObjectTypeInteger,
            data: ObjectData::Integer(next_id),
        })
    });
    id
}

/// @param keep set keep_msg if it doesn't scroll
pub fn msg_keep(s_in: &std::ffi::CStr, hl_id: i32, keep: bool, multiline: bool) -> bool {
    let mut s = s_in;
    {
        if keep && multiline {
            std::process::abort();
        }

        // Skip messages not match ":filter pattern".
        // Don't filter when there is an error.
        if !g_st().emsg_on_display.get() && message_filtered(s) {
            return true;
        }

        if hl_id == 0 {
            set_vim_var_string(VV_STATUSMSG, Some(s), -1);
        }

        // It is possible that displaying a messages causes a problem (e.g.,
        // when redrawing the window), which causes another message, etc..    To
        // break this loop, limit the recursiveness to 3 levels.
        if MSG_STATE.with(|s| s.msg_keep_entered.get()) >= 3 {
            return true;
        }
        MSG_STATE.with(|s| s.msg_keep_entered.set(s.msg_keep_entered.get() + (1)));

        // Add message to history unless it's a multihl, repeated kept or truncated message.
        let keep_msg_ptr = g_st()
            .keep_msg
            .get()
            .as_ref()
            .map_or(ptr::null(), |k| k.as_ptr());
        if MSG_STATE.with(|s| s.is_multihl.get()) == 0
            && (s.as_ptr() != keep_msg_ptr
                || (s.to_bytes().first().copied().unwrap_or(0) != b'<'
                    && with_msg_ext(|ext| {
                        ext.msg_hist.borrow().back().is_some_and(|last| {
                            s.to_bytes() != last.msg.as_slice()[0].text.cstr_bytes()
                        })
                    })))
        {
            msg_hist_add(s.to_bytes(), -1, hl_id);
        }

        if MSG_STATE.with(|s| s.is_multihl.get()) == 0 {
            msg_start();
        }
        // Truncate the message if needed.
        let buf = msg_strtrunc(s, 0);
        if let Some(ref buf) = buf {
            s = buf.as_c_str();
        }

        let mut need_clear = true;
        if multiline {
            msg_multiline(
                cstr_as_string(Some(s)),
                hl_id,
                false,
                false,
                &mut need_clear,
            );
        } else {
            msg_outtrans(s, hl_id, false);
        }
        if need_clear {
            msg_clr_eos();
        }
        let mut retval = true;
        if MSG_STATE.with(|s| s.is_multihl.get()) == 0 {
            retval = msg_end();
        }

        if keep
            && retval
            && vim_strsize(s)
                < (g_st().rows_global.get() - g_st().cmdline_row.get() - 1)
                    * g_st().columns_global.get()
                    + g_st().sc_col.get()
        {
            set_keep_msg(Some(s), 0);
        }

        g_st().need_fileinfo.set(false);

        drop(buf);
        MSG_STATE.with(|s| s.msg_keep_entered.set(s.msg_keep_entered.get() - (1)));
        retval
    }
}

/// Truncate a string such that it can be printed without causing a scroll.
///
/// @param force  always truncate
pub fn msg_strtrunc(s: &std::ffi::CStr, force: i32) -> Option<std::ffi::CString> {
    {
        // May truncate message to avoid a hit-return prompt
        if (g_st().msg_scroll.get() == 0
            && !g_st().need_wait_return.get()
            && shortmess(K_SHM_TRUNCALL)
            && g_st().msg_silent.get() == 0
            && !ui_has(K_UIMESSAGES))
            || force != 0
        {
            let len = vim_strsize(s);
            let room = if g_st().msg_scrolled.get() != 0 {
                // Use all the columns.
                (g_st().rows_global.get() - g_st().msg_row.get()) * g_st().columns_global.get() - 1
            } else {
                // Use up to 'showcmd' column.
                let last_row = if ui_has(K_UIMESSAGES) {
                    g_st().columns_global.get()
                } else {
                    g_st().sc_col.get() - 1
                };
                (g_st().rows_global.get() - g_st().msg_row.get() - 1) * g_st().columns_global.get()
                    + last_row
            };
            if len > room && room > 0 {
                // may have up to 18 bytes per cell (6 per char, up to two
                // composing chars)
                let buflen = (room + 2) * 18;
                let mut buf: Vec<u8> = vec![0; buflen as usize];
                trunc_string(Some(s.to_bytes()), &mut buf, room);
                return Some(crate::memory::cstring_until_nul(&buf));
            }
        }
        None
    }
}

/// Truncate a string `s` (or `out` in place when `s` is `None`) to `out` with cell width `room_in`.
pub fn trunc_string(s: Option<&[u8]>, out: &mut [u8], room_in: i32) {
    if out.is_empty() {
        return;
    }
    let buflen = out.len() as i32;
    let room = (room_in - 3).max(0); // "..." takes 3 chars

    let (s_len, e, i, fits_early) = {
        let s_bytes: &[u8] = match s {
            Some(b) => crate::memory::bytes_until_nul(b),
            None => crate::memory::bytes_until_nul(out),
        };
        if s_bytes.is_empty() {
            (0, 0, 0, true)
        } else {
            let s_len = s_bytes.len() as i32;
            let mut len: i32 = 0;
            let mut e: i32 = 0;
            let mut half = room / 2;
            let mut fits_early = false;

            while len < half && e < buflen {
                if (e as usize) >= s_bytes.len() {
                    fits_early = true;
                    break;
                }
                let rem = &s_bytes[e as usize..];
                let n = crate::mbyte::slice2cells(rem);
                if len + n > half {
                    break;
                }
                len += n;
                let mut step = crate::mbyte::utfc_slice2len(rem) as i32;
                while {
                    step -= 1;
                    step > 0
                } {
                    e += 1;
                    if e == buflen {
                        break;
                    }
                }
                e += 1;
            }

            if fits_early {
                (s_len, e, 0, true)
            } else {
                let mut i = s_len;
                half = i;
                loop {
                    half =
                        half - crate::mbyte::utf_head_off_slice(s_bytes, (half - 1) as usize) - 1;
                    let n = crate::mbyte::slice2cells(&s_bytes[half as usize..]);
                    if len + n > room || half == 0 {
                        break;
                    }
                    len += n;
                    i = half;
                }
                (s_len, e, i, false)
            }
        }
    };

    if s_len == 0 {
        out[0] = 0;
        return;
    }
    if fits_early {
        if let Some(b) = s {
            let s_bytes = crate::memory::bytes_until_nul(b);
            let copy_len = s_bytes.len().min(out.len() - 1);
            out[..copy_len].copy_from_slice(&s_bytes[..copy_len]);
            out[copy_len] = 0;
        }
        return;
    }

    if let Some(b) = s {
        let s_bytes = crate::memory::bytes_until_nul(b);
        let copy_e = (e as usize).min(out.len()).min(s_bytes.len());
        if copy_e > 0 {
            out[..copy_e].copy_from_slice(&s_bytes[..copy_e]);
        }
    }

    if i <= e + 3 {
        // text fits without truncating
        if let Some(b) = s {
            let s_bytes = crate::memory::bytes_until_nul(b);
            let len = s_len.min(buflen - 1) - e + 1;
            if len < 1 {
                out[(e - 1).min(buflen - 1) as usize] = 0;
            } else {
                let ue = e as usize;
                let ulen = (len as usize).min(out.len() - ue);
                let src_len = (ulen - 1).min(s_bytes.len().saturating_sub(ue));
                out[ue..ue + src_len].copy_from_slice(&s_bytes[ue..ue + src_len]);
                out[ue + src_len] = 0;
            }
        }
    } else if e + 3 < buflen {
        // set the middle and copy the last part
        let ue = e as usize;
        out[ue..ue + 3].copy_from_slice(b"...");
        let len = (s_len - i + 1).min(buflen - e - 3 - 1);
        let ulen = len as usize;
        let ui = i as usize;
        if let Some(b) = s {
            let s_bytes = crate::memory::bytes_until_nul(b);
            let copy_tail = (ulen - 1).min(s_bytes.len().saturating_sub(ui));
            out[ue + 3..ue + 3 + copy_tail].copy_from_slice(&s_bytes[ui..ui + copy_tail]);
        } else {
            out.copy_within(ui..ui + ulen, ue + 3);
        }
        out[ue + 3 + ulen - 1] = 0;
    } else {
        // can't fit in the "...", just truncate it
        out[(buflen - 1) as usize] = 0;
    }
}

/// Reset the last used sourcing name/lnum.  Makes sure it is displayed again
/// for the next error message;
pub fn reset_last_sourcing() {
    with_msg_ext(|ext| *ext.last_sourcing_name.borrow_mut() = None);
    MSG_STATE.with(|s| s.last_sourcing_lnum.set(0));
}

/// @return  true if "SOURCING_NAME" differs from "last_sourcing_name".
fn other_sourcing_name() -> bool {
    if have_sourcing_info()
        && let Some(sname) = sourcing_name()
    {
        if let Some(last) = with_msg_ext(|ext| ext.last_sourcing_name.borrow().clone()) {
            return sname.to_bytes() != last.as_bytes();
        }
        return true;
    }
    false
}

/// Get the message about the source, as used for an error message
fn get_emsg_source() -> Option<std::ffi::CString> {
    if have_sourcing_info()
        && let Some(mut sname) = sourcing_name()
        && other_sourcing_name()
    {
        let tofree = estack_sfile(ESTACK_NONE);
        if let Some(tf) = tofree {
            sname = tf;
        }

        let p = gettext(c"Error in %s:");
        let buf_len = sname.to_bytes().len() + p.to_bytes().len() + 1;
        let mut buf_vec: Vec<u8> = vec![0; buf_len];
        format_to_buf!(&mut buf_vec[..], p, &sname);
        return Some(crate::memory::cstring_until_nul(&buf_vec));
    }
    None
}

/// Get the message about the source lnum, as used for an error message.
fn get_emsg_lnum() -> Option<std::ffi::CString> {
    let sname = sourcing_name();
    let slnum = sourcing_lnum();
    if sname.is_some()
        && (other_sourcing_name() || slnum != MSG_STATE.with(|s| s.last_sourcing_lnum.get()))
        && slnum != 0
    {
        let p = gettext(c"line %4d:");
        let buf_len = 20 + p.to_bytes().len();
        let mut buf_vec: Vec<u8> = vec![0; buf_len];
        format_to_buf!(&mut buf_vec, p, slnum);
        return Some(crate::memory::cstring_until_nul(&buf_vec));
    }
    None
}

/// Display name and line number for the source of an error.
/// Remember the file name and line number, so that for the next error the info
/// is only displayed if it changed.
pub fn msg_source(hl_id: i32) {
    if MSG_STATE.with(|s| s.msg_source_recursive.get()) {
        return;
    }
    MSG_STATE.with(|s| s.msg_source_recursive.set(true));

    g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
    if let Some(p) = get_emsg_source() {
        g_st().msg_scroll.set(1);
        msg(p.as_c_str(), hl_id);
    }
    if let Some(p) = get_emsg_lnum() {
        msg(p.as_c_str(), HLF_N);
        MSG_STATE.with(|s| s.last_sourcing_lnum.set(sourcing_lnum()));
    }

    let sname = sourcing_name();
    if sname.is_none() || other_sourcing_name() {
        with_msg_ext(|ext| *ext.last_sourcing_name.borrow_mut() = None);
        if let Some(sname) = sname {
            with_msg_ext(|ext| *ext.last_sourcing_name.borrow_mut() = Some(sname.to_owned()));
            if redirecting() == 0 {
                msg_putchar_hl(NL, hl_id);
            }
        }
    }
    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));

    MSG_STATE.with(|s| s.msg_source_recursive.set(false));
}

/// @return  true if not giving error messages right now:
///            If "emsg_off" is set: no error messages at the moment.
///            If "msg" is in 'debug': do error message but without side effects.
///            If "emsg_skip" is set: never do error messages.
pub fn emsg_not_now() -> bool {
    (g_st().emsg_off.get() > 0
        && !crate::strings::vim_haschr(opt_vars().p_debug.as_cstr(), b'm' as i32)
        && !crate::strings::vim_haschr(opt_vars().p_debug.as_cstr(), b't' as i32))
        || g_st().emsg_skip.get() > 0
}

pub fn emsg_multiline(
    s: &std::ffi::CStr,
    kind: &std::ffi::CStr,
    hl_id: i32,
    multiline: bool,
) -> bool {
    {
        let mut ignore = false;

        // Skip this if not giving error messages at the moment.
        if emsg_not_now() {
            return true;
        }

        g_st().called_emsg.set(g_st().called_emsg.get() + (1));

        // If "emsg_severe" is true: When an error exception is to be thrown,
        // prefer this message over previous messages for the same command.
        let severe = g_st().emsg_severe.get();
        g_st().emsg_severe.set(false);

        if g_st().emsg_off.get() == 0
            || crate::strings::vim_haschr(opt_vars().p_debug.as_cstr(), b't' as i32)
        {
            if cause_errthrow(
                s,
                multiline,
                MSG_STATE.with(|s| s.is_multihl.get()) > 1,
                severe,
                &mut ignore,
            ) {
                if !ignore {
                    g_st().did_emsg.set(g_st().did_emsg.get() + (1));
                }
                return true;
            }

            if g_st().in_assert_fails.get() && g_st().emsg_assert_fails_msg.get().is_none() {
                g_st().emsg_assert_fails_msg.set(Some(s.to_owned()));
                g_st().emsg_assert_fails_lnum.set(sourcing_lnum() as i64);
                let sname = sourcing_name();
                g_st()
                    .emsg_assert_fails_context
                    .set(Some(sname.unwrap_or_default().to_owned()));
            }

            // set "v:errmsg", also when using ":silent! cmd"
            set_vim_var_string(VV_ERRMSG, Some(s), -1);

            let sname = sourcing_name();
            let slnum = sourcing_lnum();

            // When using ":silent! cmd" ignore error messages.
            // But do write it to the redirection file.
            if g_st().emsg_silent.get() != 0 {
                if !g_st().emsg_noredir.get() {
                    msg_start();
                    if let Some(p) = get_emsg_source() {
                        let p_bytes = p.as_bytes();
                        redir_write(p_bytes, p_bytes.len() as isize);
                        redir_write(b"\n", 1);
                    }
                    if let Some(p) = get_emsg_lnum() {
                        let p_bytes = p.as_bytes();
                        redir_write(p_bytes, p_bytes.len() as isize);
                        redir_write(b"\n", 1);
                    }
                    redir_write(s.to_bytes(), s.to_bytes().len() as isize);
                }

                // Log (silent) errors as debug messages.
                if let Some(sname) = sname
                    && slnum != 0
                {
                    logmsg!(
                        LOGLVL_DBG,
                        None,
                        Some(c"emsg_multiline"),
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
                        None,
                        Some(c"emsg_multiline"),
                        line!() as i32,
                        true,
                        c"(:silent) %s",
                        s,
                    );
                }

                return true;
            }

            // Log editor errors as INFO.
            if let Some(sname) = sname
                && slnum != 0
            {
                logmsg!(
                    LOGLVL_INF,
                    None,
                    Some(c"emsg_multiline"),
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
                    None,
                    Some(c"emsg_multiline"),
                    line!() as i32,
                    true,
                    c"%s",
                    s,
                );
            }

            g_st().ex_exitval.set(1);

            // Reset msg_silent, an error causes messages to be switched back on.
            g_st().msg_silent.set(0);
            g_st().cmd_silent.set(false);

            if g_st().global_busy.get() != 0 {
                // break :global command
                g_st().global_busy.set(g_st().global_busy.get() + (1));
            }

            if opt_vars().p_eb.get() {
                beep_flush(); // also includes flush_buffers()
            } else {
                flush_buffers(FLUSH_MINIMAL); // flush internal buffers
            }
            g_st().did_emsg.set(g_st().did_emsg.get() + (1)); // flag for DoOneCmd()
        }

        g_st().emsg_on_display.set(true); // remember there is an error message
        if g_st().msg_scrolled.get() != 0 {
            g_st().need_wait_return.set(true);
        }
        msg_ext_set_kind(kind);

        // Display name and line number for the source of the error.
        g_st().msg_scroll.set(1);
        let save_msg_skip_flush = g_st().msg_ext_skip_flush.get();
        g_st().msg_ext_skip_flush.set(true);
        msg_source(hl_id);

        // Display the error message itself.
        g_st().msg_nowait.set(false); // Wait for this msg.
        let rv = msg_keep(s, hl_id, false, multiline);
        g_st().msg_ext_skip_flush.set(save_msg_skip_flush);
        rv
    }
}

/// emsg() - display an error message
pub fn emsg(s: &std::ffi::CStr) -> bool {
    emsg_multiline(s, c"emsg", HLF_E, false)
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
pub fn iemsg(s: &std::ffi::CStr) {
    if emsg_not_now() {
        return;
    }
    emsg(s);
}

/// Give an "Internal error" message.
pub fn internal_error(where_: &std::ffi::CStr) {
    siemsg!(gettext(E_INTERN2), where_);
}

pub fn msg_semsg_event(s: std::ffi::CString) {
    emsg(s.as_c_str());
}

pub fn msg_schedule_semsg_cstr(s: &std::ffi::CStr) {
    loop_schedule_deferred(
        crate::event::r#loop::main_loop_ptr(),
        Event::MsgSemsg(s.to_owned()),
    );
}

pub fn msg_semsg_multiline_event(s: std::ffi::CString) {
    emsg_multiline(s.as_c_str(), c"emsg", HLF_E, true);
}

pub fn msg_schedule_semsg_multiline_cstr(s: &std::ffi::CStr) {
    loop_schedule_deferred(
        crate::event::r#loop::main_loop_ptr(),
        Event::MsgSemsgMultiline(s.to_owned()),
    );
}

/// Like msg(), but truncate to a single line if opt_vars().p_shm contains 't', or when
/// "force" is true.  This truncates in another way as for normal messages.
/// Careful: The string may be changed by msg_may_trunc()!
///
/// @return  a pointer to the printed message, if wait_return() not called.
pub fn msg_trunc(s: &mut [u8], force: bool, hl_id: i32) -> Option<&std::ffi::CStr> {
    // Add message to history before truncating.
    msg_hist_add(crate::memory::bytes_until_nul(s), -1, hl_id);

    let ts = msg_may_trunc(force, s);

    g_st().msg_hist_off.set(true);
    let n = msg(ts, hl_id);
    g_st().msg_hist_off.set(false);

    n.then_some(ts)
}

/// Check if message "s" should be truncated at the start (for filenames).
///
/// @return  a pointer to where the truncated message starts.
///
/// @note: May change the message by replacing a character with '<'.
pub fn msg_may_trunc(force: bool, s: &mut [u8]) -> &std::ffi::CStr {
    if ui_has(K_UIMESSAGES) {
        return crate::memory::bytes_as_cstr(s);
    }

    let slen = crate::memory::bytes_until_nul(s).len() as i32;
    let room = (g_st().rows_global.get() - g_st().cmdline_row.get() - 1)
        * g_st().columns_global.get()
        + g_st().sc_col.get()
        - 1;
    let mut off = 0usize;
    if room > 0 && (force || shortmess(K_SHM_TRUNC)) && slen - room > 0 {
        let mut size = vim_strsize(crate::memory::bytes_as_cstr(s));

        // There may be room anyway when there are multibyte chars.
        if size <= room {
            return crate::memory::bytes_as_cstr(s);
        }
        let mut n = 0usize;
        while size >= room {
            size -= utf_ptr2cells(&s[n..]);
            n += utfc_ptr2len(&s[n..]) as usize;
        }
        n -= 1;
        s[n] = b'<';
        off = n;
    }
    crate::memory::bytes_as_cstr(&s[off..])
}

/// Shows a progress-message.  A non-"running" `status` ends the message and announces the
/// progress task as "done".
pub fn msg_progress<'a>(
    s: Option<&'a mut [u8]>,
    id: &std::ffi::CStr,
    status: &std::ffi::CStr,
    percent: i32,
    hl_id: i32,
    hist: bool,
    trunc: bool,
    err: bool,
) -> Option<&'a std::ffi::CStr> {
    let mut data = MessageData {
        source: cstr_as_string(Some(c"nvim")),
        percent: Integer::from(percent),
        title: NvimString::new(),
        status: cstr_as_string(Some(status)),
        data: Dict::new(),
    };
    msg_with_id(s, id, c"progress", Some(&mut data), hl_id, hist, trunc, err)
}

/// Shows message `s` with `id`: replaces the message with the same `id`, if any.
pub fn msg_with_id<'a>(
    s: Option<&'a mut [u8]>,
    id: &std::ffi::CStr,
    kind: &std::ffi::CStr,
    data: Option<&mut MessageData>,
    hl_id: i32,
    hist: bool,
    trunc: bool,
    err: bool,
) -> Option<&'a std::ffi::CStr> {
    let mut clear = false;
    let mut chunks = HlMessage::new();
    let mut out_cstr: Option<&'a std::ffi::CStr> = None;
    if let Some(s_buf) = s {
        if hist && (!trunc || ui_has(K_UIMESSAGES)) {
            msg_hist_add(crate::memory::bytes_until_nul(s_buf), -1, 0);
        }
        let s_cstr = if trunc {
            msg_may_trunc(false, s_buf)
        } else {
            crate::memory::bytes_as_cstr(s_buf)
        };
        chunks.push(HlMessageChunk {
            text: cstr_to_string(Some(s_cstr)),
            hl_id,
        });
        out_cstr = Some(s_cstr);
    }
    msg_ext_no_fast();
    let id_obj = Object {
        obj_type: ObjectType::ObjectTypeString,
        data: ObjectData::String(cstr_as_string(Some(id))),
    };
    msg_multihl(
        id_obj,
        &mut chunks,
        Some(kind),
        false,
        err,
        data,
        &mut clear,
        false,
    );
    ui_flush();
    out_cstr
}

/// Add the message at the end of the history
///
/// @param[in]  len  Length of s or -1.
fn msg_hist_add(s: &[u8], len: i32, hl_id: i32) {
    let mut slice = if len < 0 {
        crate::memory::bytes_until_nul(s)
    } else {
        &s[..(len as usize).min(s.len())]
    };
    while let Some((&b'\n', rest)) = slice.split_first() {
        slice = rest;
    }
    while let Some((&b'\n', rest)) = slice.split_last() {
        slice = rest;
    }
    if slice.is_empty() {
        return;
    }
    let text = crate::api::private::helpers::cbuf_to_string(slice);
    let msg = vec![HlMessageChunk { text, hl_id }];
    msg_hist_add_multihl(msg, false);
}

pub fn do_autocmd_progress(msg_id: MsgID, msg: &HlMessage, mut msg_data: Option<&mut MessageData>) {
    if !has_event(EVENT_PROGRESS) {
        return;
    }

    let mut messages_vec: Vec<Object> = msg
        .iter()
        .map(|chunk| Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(chunk.text.clone()),
        })
        .collect();
    let messages = Array::from_slice(&mut messages_vec);

    let mut data_items = [const {
        KeyValuePair {
            key: NvimString::new(),
            value: NIL_OBJ,
        }
    }; 7];
    let mut data_size = 0usize;
    let mut put_c = |key: &'static CStr, value: Object| {
        data_items[data_size] = KeyValuePair {
            key: NvimString::from_cstr(key),
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
                data: ObjectData::String(msg_data.as_deref_mut().unwrap().source.clone()),
            },
        );
        put_c(
            c"status",
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(msg_data.as_deref_mut().unwrap().status.clone()),
            },
        );
        put_c(
            c"title",
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(msg_data.as_deref_mut().unwrap().title.clone()),
            },
        );
        put_c(
            c"data",
            Object {
                obj_type: ObjectType::ObjectTypeDict,
                data: ObjectData::Dict(msg_data.as_deref_mut().unwrap().data.clone()),
            },
        );
    }

    let dict_obj = Object {
        obj_type: ObjectType::ObjectTypeDict,
        data: ObjectData::Dict(Dict::from_slice(&mut data_items[..data_size])),
    };
    let fname = if let Some(ref d) = msg_data
        && d.source.size > 0
    {
        d.source.as_cstr()
    } else {
        c""
    };
    apply_autocmds_group(
        EVENT_PROGRESS,
        Some(fname),
        None,
        true,
        AUGROUP_ALL,
        None,
        Some(&crate::globals::curwin_mut()),
        None,
        Some(&dict_obj),
        false,
    );
}

fn msg_hist_add_multihl(msg: HlMessage, temp: bool) {
    if MSG_STATE.with(|s| s.do_clear_hist_temp.get()) {
        msg_hist_clear_temp();
        MSG_STATE.with(|s| s.do_clear_hist_temp.set(false));
    }

    if g_st().msg_hist_off.get() || g_st().msg_silent.get() != 0 {
        return;
    }

    let temp_idx = MSG_STATE.with(|s| s.msg_hist_temp_idx.get());
    if temp_idx.is_none() {
        let len = with_msg_ext(|ext| ext.msg_hist.borrow().len());
        MSG_STATE.with(|s| s.msg_hist_temp_idx.set(Some(len)));
    }
    let append = MSG_STATE.with(|s| s.msg_ext_append.get());
    with_msg_ext(|ext| {
        let kind = ext.msg_ext_kind.borrow().clone();
        ext.msg_hist.borrow_mut().push_back(MessageHistoryEntry {
            msg,
            kind,
            temp,
            append,
        });
    });

    MSG_STATE.with(|s| {
        s.msg_hist_len
            .set(s.msg_hist_len.get() + (i32::from(!temp)))
    });
    MSG_STATE.with(|s| s.msg_ext_history.set(true));

    msg_hist_clear(MSG_STATE.with(|s| s.msg_hist_max.get()));
}

/// Delete oldest messages from the history until there are "keep" messages.
pub fn msg_hist_clear(keep: i32) {
    while MSG_STATE.with(|s| s.msg_hist_len.get()) > keep
        || (keep == 0 && with_msg_ext(|ext| !ext.msg_hist.borrow().is_empty()))
    {
        if let Some(entry) = with_msg_ext(|ext| ext.msg_hist.borrow_mut().pop_front()) {
            MSG_STATE.with(|s| {
                s.msg_hist_len
                    .set(s.msg_hist_len.get() - (i32::from(!entry.temp)))
            });
            if let Some(idx) = MSG_STATE.with(|s| s.msg_hist_temp_idx.get()) {
                MSG_STATE.with(|s| s.msg_hist_temp_idx.set(Some(idx.saturating_sub(1))));
            }
        }
    }
    if with_msg_ext(|ext| ext.msg_hist.borrow().is_empty()) {
        MSG_STATE.with(|s| s.msg_hist_temp_idx.set(None));
    }
}

pub fn msg_hist_clear_temp() {
    let temp_idx = MSG_STATE.with(|s| s.msg_hist_temp_idx.get());
    MSG_STATE.with(|s| s.msg_hist_temp_idx.set(None));
    if let Some(start_idx) = temp_idx {
        with_msg_ext(|ext| {
            let mut hist = ext.msg_hist.borrow_mut();
            let mut i = start_idx;
            while i < hist.len() {
                if hist[i].temp {
                    hist.remove(i);
                } else {
                    i += 1;
                }
            }
        });
    }
}

/// Check the 'messagesopt' fields after schema validation.
pub fn validate_messagesopt(args: &mut Optset) -> Option<&'static CStr> {
    let v: OptKeyDict_mopt =
        opt_keyset(args.os_newval.data.string().cstr_bytes(), K_OPT_MESSAGESOPT);

    let has_wait = (v.is_set_mopt & (1 << KEYSET_OPTIDX_MOPT_WAIT)) != 0;
    let has_history = (v.is_set_mopt & (1 << KEYSET_OPTIDX_MOPT_HISTORY)) != 0;

    // Either "wait" or "hit-enter" is required; "history" always.
    if (!v.hit_enter && !has_wait) || !has_history {
        return Some(E_INVARG);
    }
    // "history" and "wait" must be <= 10000; "maxheight" is a percentage.
    if v.history > 10000 || v.wait > 10000 || v.maxheight > 100 {
        return Some(E_INVARG);
    }
    None
}

/// Process the updated 'messagesopt' option value.
pub fn did_set_messagesopt(_args: &mut Optset) -> Option<&'static CStr> {
    let v: OptKeyDict_mopt = opt_keyset(opt_vars().p_mopt.as_bytes(), K_OPT_MESSAGESOPT);

    MSG_STATE.with(|s| s.msg_hit_enter.set(v.hit_enter));
    MSG_STATE.with(|s| s.msg_wait.set(v.wait as i32));
    MSG_STATE.with(|s| {
        s.progress_msg_target
            .set(if crate::memory::bytes_until_nul(&v.progress) == b"c" {
                PROGRESS_TARGET_CMD
            } else {
                0
            })
    });
    MSG_STATE.with(|s| s.msg_hist_max.set(v.history as i32));
    msg_hist_clear(MSG_STATE.with(|s| s.msg_hist_max.get()));
    None
}

/// :messages command implementation
pub fn ex_messages(eap: &mut Exarg) {
    if eap.arg.as_bytes() == b"clear" {
        msg_hist_clear(if eap.addr_count != 0 { eap.line2 } else { 0 });
        return;
    }

    if !eap.arg.is_empty() {
        emsg(gettext(E_INVARG));
        return;
    }

    let mut entries = Array::new();
    let hist_len = with_msg_ext(|ext| ext.msg_hist.borrow().len());
    let mut idx = if eap.skip {
        MSG_STATE
            .with(|s| s.msg_hist_temp_idx.get())
            .unwrap_or(hist_len)
    } else {
        0
    };
    let mut skip = if eap.addr_count != 0 {
        MSG_STATE.with(|s| s.msg_hist_len.get()) - eap.line2
    } else {
        0
    };
    while idx < with_msg_ext(|ext| ext.msg_hist.borrow().len()) {
        let cur_idx = idx;
        idx += 1;
        let prev_skip = skip;
        skip -= 1;
        let (is_temp, kind, append) = with_msg_ext(|ext| {
            let p = &ext.msg_hist.borrow()[cur_idx];
            (p.temp, p.kind.clone(), p.append)
        });
        if (is_temp && !eap.skip) || prev_skip > 0 {
            continue;
        }
        if ui_has(K_UIMESSAGES) && g_st().msg_silent.get() == 0 {
            let mut entry = Array::new();
            entry.push(Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_to_string(kind.as_deref())),
            });
            let mut content = Array::new();
            with_msg_ext(|ext| {
                let p = &ext.msg_hist.borrow()[cur_idx];
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
                        data: ObjectData::String(copy_string(chunk.text.clone(), None)),
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
            });
            entry.push(Object {
                obj_type: ObjectType::ObjectTypeArray,
                data: ObjectData::Array(content),
            });
            entry.push(Object {
                obj_type: ObjectType::ObjectTypeBoolean,
                data: ObjectData::Boolean(append),
            });
            entries.push(Object {
                obj_type: ObjectType::ObjectTypeArray,
                data: ObjectData::Array(entry),
            });
        }
        if redirecting() != 0 || !ui_has(K_UIMESSAGES) {
            g_st()
                .msg_silent
                .set(g_st().msg_silent.get() + (i32::from(ui_has(K_UIMESSAGES))));
            let mut needs_clear = false;
            let mut msg_vec =
                with_msg_ext(|ext| std::mem::take(&mut ext.msg_hist.borrow_mut()[cur_idx].msg));
            msg_multihl(
                NIL_OBJ,
                &mut msg_vec,
                kind.as_deref(),
                false,
                false,
                None,
                &mut needs_clear,
                false,
            );
            with_msg_ext(|ext| ext.msg_hist.borrow_mut()[cur_idx].msg = msg_vec);
            g_st()
                .msg_silent
                .set(g_st().msg_silent.get() - (i32::from(ui_has(K_UIMESSAGES))));
        }
    }
    if entries.size > 0 {
        ui_call_msg_history_show(entries.clone(), eap.skip);
        api_free_array(entries);
    }
}

/// Call this after prompting the user.  This will avoid a hit-return message
/// and a delay.
pub fn msg_end_prompt() {
    g_st().need_wait_return.set(false);
    g_st().emsg_on_display.set(false);
    g_st().cmdline_row.set(g_st().msg_row.get());
    g_st().msg_col.set(0);
    msg_clr_eos();
    g_st().lines_left.set(-1);
}

/// Wait for the user to hit a key (normally Enter)
///
/// @param redraw  if true, redraw the entire screen UPD_NOT_VALID
///                if false, do a normal redraw
///                if -1, don't redraw at all
pub fn wait_return(redraw: i32) {
    let mut c: i32;
    let mut had_got_int: bool;

    if redraw == 1 {
        redraw_all_later(UPD_NOT_VALID);
    }

    if ui_has(K_UIMESSAGES) {
        prompt_for_input(Some(c"Press any key to continue"), HLF_M, true, None);
        return;
    }

    // If using ":silent cmd", don't wait for a return.  Also don't set
    // need_wait_return to do it later.
    if g_st().msg_silent.get() != 0 {
        return;
    }

    if (g_st().headless_mode.get() && ui_active() == 0) || input_unanswerable() {
        return;
    }

    // When inside vgetc(), we can't wait for a typed character at all.
    if g_st().vgetc_busy.get() > 0 {
        return;
    }
    g_st().need_wait_return.set(true);
    if g_st().no_wait_return.get() != 0 {
        g_st().cmdline_row.set(g_st().msg_row.get());
        return;
    }

    g_st().redir_off.set(true); // don't redirect this message
    let old_state = g_st().state_global.get();
    if g_st().quit_more.get() {
        c = CAR; // just pretend CR was hit
        g_st().quit_more.set(false);
        g_st().got_int.set(false);
    } else if !stuff_empty() {
        c = CAR;
    } else {
        g_st().state_global.set(MODE_HITRETURN);
        setmouse();
        g_st().cmdline_row.set(g_st().msg_row.get());
        if g_st().need_check_timestamps.get() {
            check_timestamps(0);
        }

        // if cmdheight=0, we need to scroll in the first line of msg_grid upon the screen
        if opt_vars().p_ch.get() == 0 && !ui_has(K_UIMESSAGES) && g_st().msg_scrolled.get() == 0 {
            msg_grid_validate();
            msg_scroll_up(false, true);
            g_st().msg_scrolled.set(g_st().msg_scrolled.get() + (1));
            g_st().cmdline_row.set(g_st().rows_global.get() - 1);
        }

        if MSG_STATE.with(|s| s.msg_hit_enter.get()) {
            hit_return_msg(true);

            loop {
                had_got_int = g_st().got_int.get();

                // Process events while waiting, avoid OOM. #42669
                let prompt_row = g_st().msg_row.get();
                let prompt_col = g_st().msg_col.get();
                if !crate::state::state_wait_key() {
                    if g_st().msg_row.get() != prompt_row || g_st().msg_col.get() != prompt_col {
                        hit_return_msg(false); // Show the prompt again, below the messages.
                    }
                    msg_cursor_goto(g_st().msg_row.get(), g_st().msg_col.get()); // Events may move the cursor.
                    continue;
                }

                g_st().no_mapping.set(g_st().no_mapping.get() + (1));
                g_st().allow_keys.set(g_st().allow_keys.get() + (1));

                let save_reg_recording = g_st().reg_recording.get();
                let save_scriptout = g_st().scriptout.take();
                g_st().reg_recording.set(0);
                c = safe_vgetc();
                if had_got_int && g_st().global_busy.get() == 0 {
                    g_st().got_int.set(false);
                }
                g_st().no_mapping.set(g_st().no_mapping.get() - (1));
                g_st().allow_keys.set(g_st().allow_keys.get() - (1));
                g_st().reg_recording.set(save_reg_recording);
                *g_st().scriptout.borrow_mut() = save_scriptout;

                if g_st().key_typed.get() && opt_vars().p_more.get() {
                    if c == b'b' as i32
                        || c == CTRL_B_CONST
                        || c == b'k' as i32
                        || c == b'u' as i32
                        || c == b'g' as i32
                        || c == K_UP
                        || c == K_PAGEUP
                    {
                        if g_st().msg_scrolled.get() > g_st().rows_global.get() {
                            do_more_prompt(c);
                        } else {
                            g_st().msg_didout.set(false);
                            c = K_IGNORE;
                            g_st().msg_col.set(0);
                        }
                        if g_st().quit_more.get() {
                            c = CAR;
                            g_st().quit_more.set(false);
                            g_st().got_int.set(false);
                        } else if c != K_IGNORE {
                            c = K_IGNORE;
                            hit_return_msg(false);
                        }
                    } else if g_st().msg_scrolled.get() > g_st().rows_global.get() - 2
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
                jump_to_mouse(MOUSE_SETPOS, None, 0);
            } else if !g_st().key_typed.get()
                || (!crate::strings::vim_haschr(c"\r\n ", c)
                    && c != CTRL_C_CONST
                    && c != b'q' as i32)
            {
                requeue_key(
                    g_st().vgetc_char.get(),
                    g_st().vgetc_mod_mask.get(),
                    0,
                    false,
                );
                g_st().do_redraw.set(true);
            }
        } else {
            c = CAR;
            do_sleep(i64::from(MSG_STATE.with(|s| s.msg_wait.get())), true);
        }
    }
    g_st().redir_off.set(false);

    if c == b':' as i32 || c == b'?' as i32 || c == b'/' as i32 {
        g_st().cmdline_row.set(g_st().msg_row.get());
        g_st().skip_redraw.set(true);
        g_st().do_redraw.set(false);
    }

    let tmp_state = g_st().state_global.get();
    g_st().state_global.set(old_state);
    setmouse();
    msg_check();
    g_st().need_wait_return.set(false);
    g_st().did_wait_return.set(true);
    g_st().emsg_on_display.set(false);
    g_st().lines_left.set(-1);
    reset_last_sourcing();
    if g_st().keep_msg.get().as_ref().is_some_and(|km| {
        vim_strsize(km)
            >= (g_st().rows_global.get() - g_st().cmdline_row.get() - 1)
                * g_st().columns_global.get()
                + g_st().sc_col.get()
    }) {
        g_st().keep_msg.set(None);
    }

    if tmp_state == MODE_SETWSIZE {
        ui_refresh();
    } else if !g_st().skip_redraw.get()
        && (redraw == 1 || (g_st().msg_scrolled.get() != 0 && redraw != -1))
    {
        redraw_later(Some(&crate::globals::curwin_mut()), UPD_VALID);
    }
}

/// Write the hit-return prompt.
fn hit_return_msg(newline_sb: bool) {
    let save_p_more = opt_vars().p_more.get();

    if !newline_sb {
        opt_vars().p_more.set(false);
    }
    if g_st().msg_didout.get() {
        msg_putchar(NL);
    }
    opt_vars().p_more.set(false);
    if g_st().got_int.get() {
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
    opt_vars().p_more.set(save_p_more);
}

/// Set "keep_msg" to "s".  Free the old value and check for NULL pointer.
pub fn set_keep_msg(s: Option<&std::ffi::CStr>, hl_id: i32) {
    if ui_has(K_UIMESSAGES) {
        return;
    }

    if let Some(s) = s
        && g_st().msg_silent.get() == 0
    {
        g_st().keep_msg.set(Some(s.to_owned()));
    } else {
        g_st().keep_msg.set(None);
    }
    MSG_STATE.with(|s| s.keep_msg_more.set(false));
    g_st().keep_msg_hl_id.set(hl_id);
}

/// Return true if printing messages should currently be done.
pub fn messaging() -> bool {
    !(opt_vars().p_lz.get() && char_avail() && !g_st().key_typed.get())
        && (opt_vars().p_ch.get() > 0 || ui_has(K_UIMESSAGES))
}

pub fn msgmore(n: i32) {
    if g_st().global_busy.get() != 0 || !messaging() {
        return;
    }

    if g_st().keep_msg.get().is_some() && !MSG_STATE.with(|s| s.keep_msg_more.get()) {
        return;
    }

    let pn = n.abs();

    if i64::from(pn) > opt_vars().p_report.get() {
        if n > 0 {
            format_to_buf!(
                &mut *g_st().msg_buf.borrow_mut(),
                ngettext(c"%d more line", c"%d more lines", pn as u64),
                pn,
            );
        } else {
            format_to_buf!(
                &mut *g_st().msg_buf.borrow_mut(),
                ngettext(c"%d line less", c"%d fewer lines", pn as u64),
                pn,
            );
        }
        if g_st().got_int.get() {
            crate::memory::append_bytes_to_c_buf(
                &mut *g_st().msg_buf_bytes_mut(),
                gettext(c" (Interrupted)").to_bytes(),
            );
        }
        let msg_cstr = g_st().msg_buf_cstr();
        if msg(&msg_cstr, 0) {
            set_keep_msg(Some(&msg_cstr), 0);
            MSG_STATE.with(|s| s.keep_msg_more.set(true));
        }
    }
}

pub fn msg_ext_set_kind(msg_kind: &std::ffi::CStr) {
    msg_ext_ui_flush();
    with_msg_ext(|ext| *ext.msg_ext_kind.borrow_mut() = Some(msg_kind.to_owned()));
    if !MSG_STATE.with(|s| s.msg_ext_append.get()) {
        MSG_STATE.with(|s| s.redir_col.set(0));
    }

    if msg_kind.to_bytes() == b"list_cmd" {
        msg_ext_no_fast();
    }
}

pub fn msg_ext_set_append(append: bool) {
    msg_ext_ui_flush();
    MSG_STATE.with(|s| s.msg_ext_append.set(append));
}

pub fn msg_ext_set_trigger(trigger: &std::ffi::CStr) {
    msg_ext_ui_flush();
    with_msg_ext(|ext| *ext.msg_ext_trigger.borrow_mut() = Some(trigger.to_owned()));
}

// Should be executed at all callsites emitting non-internal messages.
pub fn msg_ext_no_fast() {
    msg_ext_ui_flush();
    g_st().msg_ext_fast.set(false);
}

/// Prepare for outputting characters in the command line.
pub fn msg_start() {
    let mut did_return = false;

    g_st()
        .msg_row
        .set(g_st().msg_row.get().max(g_st().cmdline_row.get()));

    if g_st().msg_silent.get() == 0 {
        g_st().keep_msg.set(None);
        g_st().need_fileinfo.set(false);
    }

    if g_st().need_highlight_changed.get() {
        highlight_changed();
    }

    if g_st().need_clr_eos.get() || (opt_vars().p_ch.get() == 0 && g_st().redrawing_cmdline.get()) {
        g_st().need_clr_eos.set(false);
        msg_clr_eos();
    }

    // if cmdheight=0, we need to scroll in the first line of msg_grid upon the screen
    if opt_vars().p_ch.get() == 0 && !ui_has(K_UIMESSAGES) && g_st().msg_scrolled.get() == 0 {
        msg_grid_validate();
        msg_scroll_up(false, true);
        g_st().msg_scrolled.set(g_st().msg_scrolled.get() + (1));
        g_st().cmdline_row.set(g_st().rows_global.get() - 1);
    }

    if g_st().msg_scroll.get() == 0 && g_st().full_screen.get() {
        g_st().msg_row.set(g_st().cmdline_row.get());
        g_st().msg_col.set(0);
    } else if (g_st().msg_didout.get() || opt_vars().p_ch.get() == 0) && !ui_has(K_UIMESSAGES) {
        if ((opt_vars().p_ch.get() == 0 && !g_st().msg_didout.get())
            || MSG_STATE.with(|s| s.msg_special_case_for_lua_print.get()))
            && msg_use_printf() != 0
        {
            msg_puts_display(b"\n", 1, 0, 0);
        } else {
            msg_putchar(NL);
        }
        did_return = true;
        g_st().cmdline_row.set(g_st().msg_row.get());
    }
    MSG_STATE.with(|s| s.msg_special_case_for_lua_print.set(false));
    if !g_st().msg_didany.get() || g_st().lines_left.get() < 0 {
        msg_starthere();
    }
    if g_st().msg_silent.get() == 0 {
        g_st().msg_didout.set(false);
    }

    if ui_has(K_UIMESSAGES) {
        msg_ext_ui_flush();
    }

    if !did_return {
        redir_write(b"\n", 1);
    }
}

/// Note that the current msg position is where messages start.
pub fn msg_starthere() {
    g_st().lines_left.set(g_st().cmdline_row.get());
    g_st().msg_didany.set(false);
}

pub fn msg_putchar(c: i32) {
    msg_putchar_hl(c, 0);
}

pub fn msg_putchar_hl(c: i32, hl_id: i32) {
    let mut buf = [0u8; MB_MAXCHAR + 1];

    if is_special(c) {
        buf[0] = K_SPECIAL;
        buf[1] = k_second(c);
        buf[2] = k_third(c);
        buf[3] = 0;
    } else {
        let len = utf_char2bytes(c, &mut buf) as usize;
        buf[len] = 0;
    }
    msg_puts_hl(crate::memory::bytes_as_cstr(&buf), hl_id, false);
}

pub fn msg_outnum(n: i32) {
    let mut buf = [0u8; 20];
    format_to_buf!(&mut buf, c"%d", n);
    msg_puts(crate::memory::bytes_as_cstr(&buf));
}

pub fn msg_home_replace(fname: Option<&std::ffi::CStr>) {
    msg_home_replace_hl(fname, 0);
}

fn msg_home_replace_hl(fname: Option<&std::ffi::CStr>, hl_id: i32) {
    let name = home_replace_save(None, fname);
    msg_outtrans(name.as_c_str(), hl_id, false);
}

/// Output "len" characters in "str" (including NULs) with translation
/// if "len" is -1, output up to a NUL character. Use highlight "hl_id".
///
/// @return  the number of characters it takes on the screen.
pub fn msg_outtrans(str_: &std::ffi::CStr, hl_id: i32, hist: bool) -> i32 {
    let bytes = str_.to_bytes();
    msg_outtrans_len(bytes, bytes.len() as i32, hl_id, hist)
}

pub fn msg_outtrans_len(msgstr: &[u8], mut len: i32, hl_id: i32, hist: bool) -> i32 {
    let slice = &msgstr[..(len.max(0) as usize).min(msgstr.len())];
    len = slice.len() as i32;
    let mut retval = 0;
    let mut idx = 0usize;
    let mut plain_start = 0usize;
    let save_got_int = g_st().got_int.get();

    // Only quit when got_int was set in here.
    g_st().got_int.set(false);

    if hist {
        msg_hist_add(slice, len, hl_id);
    }

    // When drawing over the command line no need to clear it later or remove
    // the mode message.
    if g_st().msg_silent.get() == 0
        && len > 0
        && g_st().msg_row.get() >= g_st().cmdline_row.get()
        && g_st().msg_col.get() == 0
    {
        g_st().clear_cmdline.set(false);
        g_st().mode_displayed.set(false);
    }

    // Go over the string.  Special characters are translated and printed.
    // Normal characters are printed several at a time.
    while {
        len -= 1;
        len >= 0 && !g_st().got_int.get()
    } {
        let rem = &slice[idx..];
        // Don't include composing chars after the end.
        let mb_l = utfc_ptr2len_len(rem, len + 1);
        if mb_l > 1 {
            let c = utf_ptr2char(rem);
            if vim_isprintc(c) {
                // Printable multi-byte char: count the cells.
                retval += utf_ptr2cells(rem);
            } else {
                // Unprintable multi-byte char: print the printable chars so
                // far and the translation of the unprintable char.
                if idx > plain_start {
                    msg_puts_len(
                        &slice[plain_start..idx],
                        (idx - plain_start) as isize,
                        hl_id,
                        hist,
                    );
                }
                plain_start = idx + mb_l as usize;
                msg_puts_hl(
                    &transchar_buf(None, c),
                    if hl_id == 0 { HLF_8 } else { hl_id },
                    false,
                );
                retval += char2cells(c);
            }
            len -= mb_l - 1;
            idx += mb_l as usize;
        } else {
            let s = transchar_byte_buf(None, i32::from(slice[idx]));
            if s.to_bytes().len() > 1 {
                if idx > plain_start {
                    msg_puts_len(
                        &slice[plain_start..idx],
                        (idx - plain_start) as isize,
                        hl_id,
                        hist,
                    );
                }
                plain_start = idx + 1;
                msg_puts_hl(&s, if hl_id == 0 { HLF_8 } else { hl_id }, false);
                retval += s.to_bytes().len() as i32;
            } else {
                retval += 1;
            }
            idx += 1;
        }
    }

    if (idx > plain_start || plain_start == 0) && !g_st().got_int.get() {
        msg_puts_len(
            &msgstr[plain_start..],
            (idx - plain_start) as isize,
            hl_id,
            hist,
        );
    }

    g_st().got_int.set(g_st().got_int.get() | (save_got_int));

    retval
}

pub fn msg_make(arg: &std::ffi::CStr) {
    const STR: &[u8; 6] = b"eeffoc";
    const RS: &[u8; 15] = b"Plon#dqg#vxjduB";

    let mut rem = crate::charset::skipwhite_slice(arg.to_bytes());
    let mut i: isize = 5;
    while let Some((&b, rest)) = rem.split_first() {
        if i < 0 || b != STR[i as usize] {
            break;
        }
        rem = rest;
        i -= 1;
    }
    if i < 0 {
        msg_putchar(NL);
        for &b in RS {
            msg_putchar(i32::from(b - 3));
        }
    }
}

/// Output the string 'str' up to a NUL character.
/// Return the number of characters it takes on the screen.
pub fn msg_outtrans_special(strstart: &[u8], from: bool, maxlen: i32) -> i32 {
    let strstart = crate::memory::bytes_until_nul(strstart);
    if strstart.is_empty() {
        return 0;
    }
    let mut rem = strstart;
    let mut retval = 0;
    let hl_id = HLF_8;

    while !rem.is_empty() {
        let mut text = if (rem.len() == strstart.len() || rem.len() == 1) && rem[0] == b' ' {
            rem = &rem[1..];
            SpecialKeyBuf::from_cstr(c"<Space>")
        } else {
            str2special(&mut rem, from, K_FALSE, None)
        };
        if text.to_bytes().len() == 1 {
            text =
                SpecialKeyBuf::from_cstr(&transchar_byte_buf(None, i32::from(text.to_bytes()[0])));
        }
        let len = vim_strsize(&text);
        if maxlen > 0 && retval + len >= maxlen {
            break;
        }
        // Highlight special keys
        msg_puts_hl(
            &text,
            if len > 1 && crate::mbyte::utfc_slice2len(text.to_bytes()) <= 1 {
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

/// Convert string, replacing key codes with printables
pub fn str2special_save(
    str_: &[u8],
    replace_spaces: bool,
    replace_others: TriState,
) -> std::ffi::CString {
    let mut buf: Vec<u8> = Vec::with_capacity(40);
    let mut rem = crate::memory::bytes_until_nul(str_);
    while !rem.is_empty() {
        let s = str2special(&mut rem, replace_spaces, replace_others, None);
        buf.extend_from_slice(s.to_bytes());
    }
    crate::memory::bytes_to_cstring(&buf)
}

/// Convert character, replacing key with printable representation.
pub fn str2special(
    sp: &mut &[u8],
    replace_spaces: bool,
    replace_others: TriState,
    mut data: Option<&mut KeyChord>,
) -> SpecialKeyBuf {
    let str_bytes = crate::memory::bytes_until_nul(sp);
    let mut rem = str_bytes;
    if let Some(p) = mb_unescape(&mut rem) {
        *sp = rem;
        if let Some(data) = data.as_deref_mut() {
            data.mods = 0;
            data.key.clear();
            data.key.extend_from_slice(p.to_bytes());
            data.key_alt.clear();
        }
        return SpecialKeyBuf::from_cstr(&p);
    }

    let mut c = str_bytes.first().copied().unwrap_or(0) as i32;
    let mut modifiers = 0;
    let mut special = false;
    let mut str_rem = str_bytes;
    if c == K_SPECIAL as i32 && str_rem.len() >= 3 {
        if str_rem[1] == KS_MODIFIER {
            modifiers = str_rem[2] as i32;
            str_rem = &str_rem[3..];
            c = str_rem.first().copied().unwrap_or(0) as i32;
        }
        if c == K_SPECIAL as i32 && str_rem.len() >= 3 {
            c = to_special(str_rem[1] as i32, str_rem[2] as i32);
            str_rem = &str_rem[2..];
        }
        if is_special(c) || modifiers != 0 {
            special = true;
        }
    }

    if !is_special(c) && UTF8LEN_TAB[c as usize] > 1 {
        let mut rem = str_rem;
        if let Some(p) = mb_unescape(&mut rem) {
            *sp = rem;
            c = utf_slice2char(p.to_bytes());
        } else {
            *sp = &str_rem[1.min(str_rem.len())..];
        }
    } else {
        *sp = &str_rem[usize::from(!str_rem.is_empty())..];
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

    if let Some(d) = data {
        d.mods = 0;
        if (b'A' as i32..=b'Z' as i32).contains(&c) {
            d.key.clear();
            d.key.push((c + 32) as u8);
            d.mods |= i32::from(MOD_MASK_SHIFT);
            d.key_alt.clear();
            d.key_alt.push(c as u8);
        } else {
            d.key.clear();
            d.key.push(c as u8);
            d.key_alt.clear();
        }
    }

    let mut out = SpecialKeyBuf::new();
    out.0[0] = c as u8;
    out
}

/// print line for :print or :list command
pub fn msg_prt_line(s: &[u8], mut list: bool) {
    let bytes = crate::memory::bytes_until_nul(s);
    let mut sc: Schar;
    let mut col = 0;
    let mut n_extra = 0;
    let mut sc_extra: Schar = 0;
    let mut sc_final: Schar = 0;
    let mut p_extra: &[u8] = &[];
    let mut tc_buf: crate::charset::TransCharBuf;
    let mut hl_id = 0;
    let mut lead_idx: Option<usize> = None;
    let mut in_multispace: bool;
    let mut multispace_pos = 0usize;
    let mut trail_idx: Option<usize> = None;

    if curwin_mut().w_onebuf_opt.wo_list.get() {
        list = true;
    }

    if list {
        // find start of trailing whitespace
        if curwin_mut().w_p_lcs_chars.borrow().trail != 0 {
            let mut t = bytes.len();
            while t > 0 && ascii_iswhite(bytes[t - 1]) {
                t -= 1;
            }
            trail_idx = Some(t);
        }
        // find end of leading whitespace
        if curwin_mut().w_p_lcs_chars.borrow().lead != 0
            || !curwin_mut()
                .w_p_lcs_chars
                .borrow()
                .leadmultispace
                .is_empty()
            || curwin_mut().w_p_lcs_chars.borrow().leadtab1 != 0
        {
            let mut l = 0usize;
            while l < bytes.len() && ascii_iswhite(bytes[l]) {
                l += 1;
            }
            if l < bytes.len() {
                lead_idx = Some(l);
            }
        }
    }

    // output a space for an empty line, otherwise the line will be overwritten
    if bytes.is_empty() && !(list && curwin_mut().w_p_lcs_chars.borrow().eol != 0) {
        msg_putchar(b' ' as i32);
    }

    let mut idx = 0usize;
    while !g_st().got_int.get() {
        if n_extra > 0 {
            n_extra -= 1;
            if n_extra == 0 && sc_final != 0 {
                sc = sc_final;
            } else if sc_extra != 0 {
                sc = sc_extra;
            } else {
                sc = schar_from_ascii(p_extra.first().copied().unwrap_or(0));
                p_extra = &p_extra[1.min(p_extra.len())..];
            }
        } else {
            let rem = &bytes[idx.min(bytes.len())..];
            let l = crate::mbyte::utfc_slice2len(rem);
            if l > 1 {
                col += crate::mbyte::utf_slice2cells(rem);
                let mut buf = [0u8; MB_MAXBYTES + 1];
                if l >= MB_MAXBYTES {
                    crate::memory::copy_bytes_to_c_buf(&mut buf, b"?");
                } else if curwin_mut().w_p_lcs_chars.borrow().nbsp != 0
                    && list
                    && matches!(utf_slice2char(rem), 160 | 0x202f)
                {
                    schar_get(&mut buf, curwin_mut().w_p_lcs_chars.borrow().nbsp);
                } else {
                    buf[..l].copy_from_slice(&rem[..l]);
                    buf[l] = 0;
                }
                msg_puts(crate::memory::bytes_as_cstr(&buf));
                idx += l;
                continue;
            } else {
                hl_id = 0;
                let c = rem.first().copied().unwrap_or(0) as i32;
                idx += 1;
                if c >= 0x80 {
                    col += utf_char2cells(c);
                    msg_putchar(c);
                    continue;
                }
                sc_extra = 0;
                sc_final = 0;
                if list {
                    in_multispace = c == b' ' as i32
                        && (bytes.get(idx) == Some(&b' ')
                            || (col > 0 && idx >= 2 && bytes.get(idx - 2) == Some(&b' ')));
                    if !in_multispace {
                        multispace_pos = 0;
                    }
                } else {
                    in_multispace = false;
                }
                if c == TAB && (!list || curwin_mut().w_p_lcs_chars.borrow().tab1 != 0) {
                    n_extra = tabstop_padding(
                        col,
                        curbuf_mut().b_p_ts.get(),
                        &curbuf_mut().b_p_vts_array.borrow(),
                    ) - 1;
                    if !list {
                        sc = schar_from_ascii(b' ');
                        sc_extra = schar_from_ascii(b' ');
                    } else {
                        let mut lcs_tab1 = curwin_mut().w_p_lcs_chars.borrow().tab1;
                        let mut lcs_tab2 = curwin_mut().w_p_lcs_chars.borrow().tab2;
                        let mut lcs_tab3 = curwin_mut().w_p_lcs_chars.borrow().tab3;
                        if lead_idx.is_some_and(|lead| idx <= lead)
                            && curwin_mut().w_p_lcs_chars.borrow().leadtab1 != 0
                        {
                            lcs_tab1 = curwin_mut().w_p_lcs_chars.borrow().leadtab1;
                            lcs_tab2 = curwin_mut().w_p_lcs_chars.borrow().leadtab2;
                            lcs_tab3 = curwin_mut().w_p_lcs_chars.borrow().leadtab3;
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
                } else if c == NUL as i32 && list && curwin_mut().w_p_lcs_chars.borrow().eol != 0 {
                    p_extra = b"";
                    n_extra = 1;
                    sc = curwin_mut().w_p_lcs_chars.borrow().eol;
                    hl_id = HLF_AT;
                    idx -= 1;
                } else if c != NUL as i32 && {
                    let n = ascii2cells(c);
                    if n > 1 {
                        n_extra = n - 1;
                        tc_buf = transchar_byte_buf(None, i32::from(c as u8));
                        let tc = tc_buf.to_bytes();
                        sc = schar_from_ascii(tc.first().copied().unwrap_or(0));
                        p_extra = &tc[1.min(tc.len())..];
                        hl_id = HLF_0;
                        true
                    } else {
                        sc = 0;
                        false
                    }
                } {
                    // handled in condition
                } else if c == b' ' as i32 {
                    if lead_idx.is_some_and(|lead| idx <= lead)
                        && in_multispace
                        && !curwin_mut()
                            .w_p_lcs_chars
                            .borrow()
                            .leadmultispace
                            .is_empty()
                    {
                        sc = curwin_mut()
                            .w_p_lcs_chars
                            .borrow()
                            .leadmultispace
                            .as_slice()[multispace_pos];
                        multispace_pos += 1;
                        if multispace_pos
                            >= curwin_mut().w_p_lcs_chars.borrow().leadmultispace.len()
                        {
                            multispace_pos = 0;
                        }
                        hl_id = HLF_0;
                    } else if lead_idx.is_some_and(|lead| idx <= lead)
                        && curwin_mut().w_p_lcs_chars.borrow().lead != 0
                    {
                        sc = curwin_mut().w_p_lcs_chars.borrow().lead;
                        hl_id = HLF_0;
                    } else if trail_idx.is_some_and(|trail| idx > trail) {
                        sc = curwin_mut().w_p_lcs_chars.borrow().trail;
                        hl_id = HLF_0;
                    } else if in_multispace
                        && !curwin_mut().w_p_lcs_chars.borrow().multispace.is_empty()
                    {
                        sc = curwin_mut().w_p_lcs_chars.borrow().multispace.as_slice()
                            [multispace_pos];
                        multispace_pos += 1;
                        if multispace_pos >= curwin_mut().w_p_lcs_chars.borrow().multispace.len() {
                            multispace_pos = 0;
                        }
                        hl_id = HLF_0;
                    } else if list && curwin_mut().w_p_lcs_chars.borrow().space != 0 {
                        sc = curwin_mut().w_p_lcs_chars.borrow().space;
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

        let mut buf = [0u8; MAX_SCHAR_SIZE];
        schar_get(&mut buf, sc);
        msg_puts_hl(
            CStr::from_bytes_until_nul(&buf).unwrap_or_default(),
            hl_id,
            false,
        );
        col += 1;
    }
    msg_clr_eos();
}

/// Output a string to the screen at position msg_row, msg_col.
/// Update msg_row and msg_col for the next message.
pub fn msg_puts(s: &std::ffi::CStr) {
    msg_puts_hl(s, 0, false);
}

pub fn msg_puts_title(mut s: &std::ffi::CStr) {
    if ui_has(K_UIMESSAGES) && s.to_bytes().first() == Some(&b'\n') {
        s = crate::memory::CStrSubslice::advance(s, 1);
    }
    msg_puts_hl(s, HLF_T, false);
}

/// Show a message in such a way that it always fits in the line.  Cut out a
/// part in the middle and replace it with "..." when necessary.
/// Does not handle multi-byte characters!
pub fn msg_outtrans_long(longstr: &std::ffi::CStr, hl_id: i32) {
    let bytes = longstr.to_bytes();
    let len = bytes.len() as i32;
    let mut slen = len;
    let room = g_st().columns_global.get() - g_st().msg_col.get();
    if !ui_has(K_UIMESSAGES) && len > room && room >= 20 {
        slen = (room - 3) / 2;
        msg_outtrans_len(bytes, slen, hl_id, false);
        msg_puts_hl(c"...", HLF_8, false);
    }
    msg_outtrans_len(&bytes[(len - slen) as usize..], slen, hl_id, false);
}

/// Basic function for writing a message with highlight id.
pub fn msg_puts_hl(s: &std::ffi::CStr, hl_id: i32, hist: bool) {
    msg_puts_len(s.to_bytes(), -1, hl_id, hist);
}

/// Write a message with highlight id.
pub fn msg_puts_len(str_: &[u8], len: isize, hl_id: i32, hist: bool) {
    let str_non_nul = crate::memory::bytes_until_nul(str_);
    let slice = if len < 0 {
        str_non_nul
    } else {
        &str_[..(len as usize).min(str_.len())]
    };
    assert!(len < 0 || !slice.contains(&0));
    // If redirection is on, also write to the redirection file.
    redir_write(slice, len);

    // Don't print anything when using ":silent cmd" or empty message.
    if g_st().msg_silent.get() != 0 || str_non_nul.is_empty() {
        if str_non_nul.is_empty() && ui_has(K_UIMESSAGES) {
            msg_ext_no_fast();
            ui_call_msg_show(
                cstr_as_string(Some(c"empty")),
                Array::new(),
                false,
                false,
                false,
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(-1),
                },
                NvimString::new(),
            );
            g_st().cmdline_was_last_drawn.set(false);
        }
        return;
    }

    if hist {
        msg_hist_add(slice, len as i32, hl_id);
    }

    let overflow = !ui_has(K_UIMESSAGES)
        && g_st().msg_scrolled.get() > if opt_vars().p_ch.get() == 0 { 1 } else { 0 };

    if overflow && !g_st().msg_scrolled_ign.get() && slice != b"\r" {
        g_st().need_wait_return.set(true);
    }
    g_st().msg_didany.set(true);

    if msg_use_printf() != 0 && !MSG_STATE.with(|s| s.msg_special_case_for_lua_print.get()) {
        let saved_msg_col = g_st().msg_col.get();
        msg_puts_printf(slice, len);
        if g_st().headless_mode.get() {
            g_st().msg_col.set(saved_msg_col);
        }
    }
    if msg_use_printf() == 0
        || (g_st().headless_mode.get() && crate::grid::grid_allocated(&g_st().default_grid))
    {
        msg_puts_display(slice, len as i32, hl_id, 0);
    }

    g_st().need_fileinfo.set(false);
}

fn msg_ext_emit_chunk() {
    if MSG_STATE.with(|s| s.msg_ext_last_attr.get()) == -1 {
        return;
    }
    let mut chunk = Array::new();
    chunk.push(Object {
        obj_type: ObjectType::ObjectTypeInteger,
        data: ObjectData::Integer(MSG_STATE.with(|s| s.msg_ext_last_attr.get()) as Integer),
    });
    MSG_STATE.with(|s| s.msg_ext_last_attr.set(-1));
    let text = with_msg_ext(|ext| {
        let s = crate::api::private::helpers::cbuf_to_string(&ext.msg_ext_last_chunk.borrow());
        *ext.msg_ext_last_chunk.borrow_mut() = Vec::new();
        s
    });
    chunk.push(Object {
        obj_type: ObjectType::ObjectTypeString,
        data: ObjectData::String(text),
    });
    chunk.push(Object {
        obj_type: ObjectType::ObjectTypeInteger,
        data: ObjectData::Integer(MSG_STATE.with(|s| s.msg_ext_last_hl_id.get()) as Integer),
    });
    MSG_STATE.with(|s| {
        let mut chunks = s.msg_ext_chunks.get();
        chunks.push(Object {
            obj_type: ObjectType::ObjectTypeArray,
            data: ObjectData::Array(chunk),
        });
        s.msg_ext_chunks.set(chunks);
    });
}

/// The display part of msg_puts_len().
/// May be called recursively to display scroll-back text.
fn msg_puts_display(str_in: &[u8], maxlen: i32, hl_id: i32, recurse: i32) {
    let str_slice = if maxlen < 0 {
        crate::memory::bytes_until_nul(str_in)
    } else {
        let max_u = (maxlen as usize).min(str_in.len());
        crate::memory::bytes_until_nul(&str_in[..max_u])
    };
    {
        let mut confirm_buttons_buf: Vec<u8>;
        let mut s_slice: &[u8] = str_slice;
        let mut sb_slice: &[u8] = str_slice;
        let mut sb_col = g_st().msg_col.get();
        let attr = if hl_id != 0 { syn_id2attr(hl_id) } else { 0 };

        g_st().did_wait_return.set(false);

        if ui_has(K_UIMESSAGES) {
            if attr != MSG_STATE.with(|s| s.msg_ext_last_attr.get()) {
                msg_ext_emit_chunk();
                MSG_STATE.with(|s| s.msg_ext_last_attr.set(attr));
                MSG_STATE.with(|s| s.msg_ext_last_hl_id.set(hl_id));
            }
            if !str_slice.is_empty() {
                with_msg_ext(|ext| {
                    ext.msg_ext_last_chunk
                        .borrow_mut()
                        .extend_from_slice(str_slice);
                });
            }

            let lastline_idx = str_slice.iter().rposition(|&b| b == b'\n');
            let tail = if let Some(idx) = lastline_idx {
                &str_slice[idx + 1..]
            } else {
                str_slice
            };
            let col = mb_string2cells_len(tail) as i32;
            g_st().msg_col.set(
                (if lastline_idx.is_some() {
                    0
                } else {
                    g_st().msg_col.get()
                }) + col,
            );

            return;
        }

        let print_attr = hl_combine_attr(hl_attr(HLF_MSG), attr);
        msg_grid_validate();

        g_st()
            .cmdline_was_last_drawn
            .set(g_st().redrawing_cmdline.get());

        let mut msg_row_pending = -1;

        loop {
            if g_st().msg_col.get() >= g_st().columns_global.get() {
                if opt_vars().p_more.get() && recurse == 0 {
                    store_sb_text(&mut sb_slice, s_slice, hl_id, &mut sb_col, 1);
                }
                if g_st().msg_no_more.get() && g_st().lines_left.get() == 0 {
                    break;
                }

                g_st().msg_col.set(0);
                g_st().msg_row.set(g_st().msg_row.get() + (1));
                g_st().msg_didout.set(false);
            }

            if g_st().msg_row.get() >= g_st().rows_global.get() {
                g_st().msg_row.set(g_st().rows_global.get() - 1);

                if g_st().msg_no_more.get() && g_st().lines_left.get() == 0 {
                    break;
                }

                if recurse == 0 {
                    if msg_row_pending >= 0 {
                        msg_line_flush();
                        msg_row_pending = -1;
                    }

                    msg_scroll_up(true, false);

                    inc_msg_scrolled();
                    g_st().need_wait_return.set(true);
                    g_st().redraw_cmdline.set(true);
                    if g_st().cmdline_row.get() > 0 {
                        g_st().cmdline_row.set(g_st().cmdline_row.get() - (1));
                    }

                    if g_st().lines_left.get() > 0 {
                        g_st().lines_left.set(g_st().lines_left.get() - (1));
                    }

                    if opt_vars().p_more.get()
                        && g_st().lines_left.get() == 0
                        && g_st().state_global.get() != MODE_HITRETURN
                        && !g_st().msg_no_more.get()
                    {
                        if do_more_prompt(NUL as i32) {
                            confirm_buttons_buf = clone_confirm_buttons();
                            s_slice = crate::memory::bytes_until_nul(&confirm_buttons_buf);
                            sb_slice = s_slice;
                        }
                        if g_st().quit_more.get() {
                            return;
                        }
                    }
                }
            }

            if s_slice.is_empty() {
                break;
            }

            if g_st().msg_row.get() != msg_row_pending
                && (s_slice[0] >= 0x20 || i32::from(s_slice[0]) == TAB)
            {
                if msg_row_pending >= 0 {
                    msg_line_flush();
                }
                grid_line_start(&g_st().msg_grid_adj, g_st().msg_row.get());
                msg_row_pending = g_st().msg_row.get();
            }

            if s_slice[0] >= 0x20 {
                let mut cw = utf_ptr2cells(s_slice);
                let l = utfc_ptr2len_len(s_slice, s_slice.len() as i32);

                if cw > 1 && g_st().msg_col.get() == g_st().columns_global.get() - 1 {
                    grid_line_puts(g_st().msg_col.get(), c">", 1, hl_attr(HLF_AT));
                    cw = 1;
                } else {
                    let step = (l as usize).min(s_slice.len());
                    let mut ch_buf = [0u8; 65];
                    let copy_len = step.min(64);
                    ch_buf[..copy_len].copy_from_slice(&s_slice[..copy_len]);
                    grid_line_puts(
                        g_st().msg_col.get(),
                        crate::memory::bytes_as_cstr(&ch_buf),
                        l,
                        print_attr,
                    );
                    s_slice = &s_slice[step..];
                }
                g_st().msg_didout.set(true);
                g_st().msg_col.set(g_st().msg_col.get() + (cw));
            } else {
                let c = s_slice[0];
                s_slice = &s_slice[1..];
                if c == b'\n' {
                    g_st().msg_didout.set(false);
                    g_st().msg_col.set(0);
                    g_st().msg_row.set(g_st().msg_row.get() + (1));
                    if opt_vars().p_more.get() && recurse == 0 {
                        store_sb_text(&mut sb_slice, s_slice, hl_id, &mut sb_col, 1);
                    }
                } else if c == b'\r' {
                    g_st().msg_col.set(0);
                } else if c == BS as u8 {
                    if g_st().msg_col.get() != 0 {
                        g_st().msg_col.set(g_st().msg_col.get() - (1));
                    }
                } else if i32::from(c) == TAB {
                    loop {
                        grid_line_puts(g_st().msg_col.get(), c" ", 1, print_attr);
                        g_st().msg_col.set(g_st().msg_col.get() + (1));

                        if g_st().msg_col.get() == g_st().columns_global.get()
                            || (g_st().msg_col.get() & 7) == 0
                        {
                            break;
                        }
                    }
                } else if c == BELL as u8 {
                    vim_beep(K_OPT_BO_FLAG_SHELL);
                }
            }
        }

        if msg_row_pending >= 0 {
            msg_line_flush();
        }
        msg_cursor_goto(g_st().msg_row.get(), g_st().msg_col.get());

        if opt_vars().p_more.get() && recurse == 0 {
            store_sb_text(&mut sb_slice, s_slice, hl_id, &mut sb_col, 0);
        }

        msg_check();
    }
}

pub fn msg_line_flush() {
    if g_st().cmdmsg_rl.get() {
        grid_line_mirror(g_st().msg_grid.cols.get());
    }
    grid_line_flush_if_valid_row();
}

pub fn msg_cursor_goto(mut row: i32, mut col: i32) {
    if g_st().cmdmsg_rl.get() {
        col = g_st().columns_global.get() - 1 - col;
    }
    let grid = grid_adjust(&g_st().msg_grid_adj, &mut row, &mut col);
    ui_grid_cursor_goto(grid.handle.get(), row, col);
}

/// @return  true when ":filter pattern" was used and "msg" does not match
///          "pattern".
pub fn message_filtered(msg: &std::ffi::CStr) -> bool {
    if g_st()
        .cmdmod
        .borrow()
        .cmod_filter_regmatch
        .regprog
        .is_null()
    {
        return false;
    }

    let mut rm = std::mem::take(&mut g_st().cmdmod.borrow_mut().cmod_filter_regmatch);
    let match_ = vim_regexec(&mut rm, msg, 0);
    g_st().cmdmod.borrow_mut().cmod_filter_regmatch = rm;
    if g_st().cmdmod.borrow().cmod_filter_force {
        match_
    } else {
        !match_
    }
}

/// including horizontal separator
pub fn msg_scrollsize() -> i32 {
    g_st().msg_scrolled.get()
        + opt_vars().p_ch.get() as i32
        + i32::from(opt_vars().p_ch.get() > 0 || g_st().msg_scrolled.get() > 1)
}

pub fn msg_do_throttle() -> bool {
    msg_use_grid() && (opt_vars().rdb_flags.get() & K_OPT_RDB_FLAG_NOTHROTTLE) == 0
}

/// Scroll the screen up one line for displaying the next message line.
pub fn msg_scroll_up(may_throttle: bool, zerocmd: bool) {
    if may_throttle && msg_do_throttle() {
        g_st().msg_grid.throttled.set(true);
    }
    g_st().msg_did_scroll.set(true);
    if g_st().msg_grid_pos.get() > 0 {
        msg_grid_set_pos(g_st().msg_grid_pos.get() - 1, !zerocmd);

        if zerocmd && crate::grid::grid_allocated(&g_st().msg_grid) {
            grid_clear_line(
                &g_st().msg_grid,
                g_st().msg_grid.line_offset.borrow()[0],
                g_st().msg_grid.cols.get(),
                false,
            );
        }
    } else {
        grid_del_lines(
            &g_st().msg_grid,
            0,
            1,
            g_st().msg_grid.rows.get(),
            0,
            g_st().msg_grid.cols.get(),
        );
        {
            let rows = g_st().msg_grid.rows.get();
            let mut dc = g_st().msg_grid.dirty_col.borrow_mut();
            for i in 0..(rows - 1) as usize {
                dc[i] = dc[i + 1];
            }
            dc[(rows - 1) as usize] = 0;
        }
    }

    grid_clear(
        &g_st().msg_grid_adj,
        g_st().rows_global.get() - 1,
        g_st().rows_global.get(),
        0,
        g_st().columns_global.get(),
        hl_attr(HLF_MSG),
    );
}

/// Send throttled message output to UI clients
pub fn msg_scroll_flush() {
    if g_st().msg_grid.throttled.get() {
        g_st().msg_grid.throttled.set(false);
        let pos_delta =
            MSG_STATE.with(|s| s.msg_grid_pos_at_flush.get()) - g_st().msg_grid_pos.get();
        assert!(pos_delta >= 0);
        let delta = (g_st().msg_scrolled.get() - g_st().msg_scrolled_at_flush.get())
            .min(g_st().msg_grid.rows.get());

        if pos_delta > 0 {
            ui_ext_msg_set_pos(g_st().msg_grid_pos.get(), true);
        }

        let to_scroll = delta - pos_delta - g_st().msg_grid_scroll_discount.get();
        assert!(to_scroll >= 0);

        if to_scroll > 0 && g_st().msg_grid_pos.get() == 0 {
            ui_call_grid_scroll(
                g_st().msg_grid.handle.get() as Integer,
                0,
                g_st().rows_global.get() as Integer,
                0,
                g_st().columns_global.get() as Integer,
                to_scroll as Integer,
                0,
            );
        }

        for i in (g_st().rows_global.get() - delta.max(1)).max(0)..g_st().rows_global.get() {
            let row = i - g_st().msg_grid_pos.get();
            assert!(row >= 0);
            let dirty = g_st().msg_grid.dirty_col.borrow()[row as usize];
            ui_line(
                &g_st().msg_grid,
                row,
                false,
                0,
                dirty,
                g_st().msg_grid.cols.get(),
                hl_attr(HLF_MSG),
                false,
            );
            g_st().msg_grid.dirty_col.borrow_mut()[row as usize] = 0;
        }
    }
    g_st().msg_scrolled_at_flush.set(g_st().msg_scrolled.get());
    g_st().msg_grid_scroll_discount.set(0);
    MSG_STATE.with(|s| s.msg_grid_pos_at_flush.set(g_st().msg_grid_pos.get()));
}

pub fn msg_reset_scroll() {
    if ui_has(K_UIMESSAGES) {
        return;
    }
    g_st().msg_grid.throttled.set(false);
    msg_grid_set_pos(
        g_st().rows_global.get() - opt_vars().p_ch.get() as i32,
        false,
    );
    g_st().clear_cmdline.set(true);
    if crate::grid::grid_allocated(&g_st().msg_grid) {
        let limit = msg_scrollsize().min(g_st().msg_grid.rows.get());
        for i in 0..limit {
            let off = g_st().msg_grid.line_offset.borrow()[i as usize];
            grid_clear_line(&g_st().msg_grid, off, g_st().msg_grid.cols.get(), false);
        }
    }
    g_st().msg_scrolled.set(0);
    g_st().msg_scrolled_at_flush.set(0);
    g_st().msg_grid_scroll_discount.set(0);
}

pub fn msg_ui_refresh() {
    if ui_has(K_UIMULTIGRID) && crate::grid::grid_allocated(&g_st().msg_grid) {
        ui_call_grid_resize(
            g_st().msg_grid.handle.get() as Integer,
            g_st().msg_grid.cols.get() as Integer,
            g_st().msg_grid.rows.get() as Integer,
        );
        ui_ext_msg_set_pos(g_st().msg_grid_pos.get(), g_st().msg_scrolled.get() != 0);
    }
}

pub fn msg_ui_flush() {
    if ui_has(K_UIMULTIGRID)
        && crate::grid::grid_allocated(&g_st().msg_grid)
        && g_st().msg_grid.pending_comp_index_update.get()
    {
        ui_ext_msg_set_pos(g_st().msg_grid_pos.get(), g_st().msg_scrolled.get() != 0);
    }
}

/// Increment "msg_scrolled".
fn inc_msg_scrolled() {
    if get_vim_var_str(VV_SCROLLSTART).is_empty() {
        let sname = sourcing_name();
        let mut tofree_vec: Vec<u8>;

        let p = if let Some(sname) = sname {
            let tofreesize = sname.to_bytes().len() + 40;
            tofree_vec = vec![0; tofreesize];
            let size = format_to_buf_safelen!(
                &mut tofree_vec,
                gettext(c"%s line %lld"),
                &sname,
                i64::from(sourcing_lnum()),
            );
            NvimString::from_bytes_len(&tofree_vec, size)
        } else {
            cstr_as_string(Some(gettext(c"Unknown")))
        };
        set_vim_var_string(VV_SCROLLSTART, p.as_opt_cstr(), p.size as isize);
    }
    g_st().msg_scrolled.set(g_st().msg_scrolled.get() + (1));
    set_must_redraw(UPD_VALID);
}

/// Store part of a printed message for displaying when scrolling back.
fn store_sb_text<'a>(
    sb_str: &mut &'a [u8],
    s: &'a [u8],
    hl_id: i32,
    sb_col: &mut i32,
    finish: i32,
) {
    if MSG_STATE.with(|s| s.do_clear_sb_text.get()) == SbClear::SbClearAll
        || MSG_STATE.with(|s| s.do_clear_sb_text.get()) == SbClear::SbClearCmdlineDone
    {
        clear_sb_text(MSG_STATE.with(|s| s.do_clear_sb_text.get()) == SbClear::SbClearAll);
        msg_sb_eol();
        if MSG_STATE.with(|s| s.do_clear_sb_text.get()) == SbClear::SbClearCmdlineDone
            && sb_str.len() > s.len()
            && sb_str[0] == b'\n'
        {
            *sb_str = &sb_str[1..];
        }
        MSG_STATE.with(|s| s.do_clear_sb_text.set(SbClear::SbClearNone));
    }

    if sb_str.len() > s.len() {
        let text_len = sb_str.len() - s.len();
        let mut text = Vec::with_capacity(text_len + 1);
        text.extend_from_slice(&sb_str[..text_len]);
        text.push(0);
        with_msg_ext(|ext| {
            ext.msgchunks.borrow_mut().push(Msgchunk {
                sb_eol: finish != 0,
                sb_msg_col: *sb_col,
                sb_hl_id: hl_id,
                sb_text: text,
            });
        });
    } else if finish != 0 {
        with_msg_ext(|ext| {
            if let Some(last) = ext.msgchunks.borrow_mut().last_mut() {
                last.sb_eol = true;
            }
        });
    }

    *sb_str = s;
    *sb_col = 0;
}

/// Finished showing messages, clear the scroll-back text on the next message.
pub fn may_clear_sb_text() {
    msg_ext_ui_flush();
    MSG_STATE.with(|s| s.do_clear_sb_text.set(SbClear::SbClearAll));
    MSG_STATE.with(|s| s.do_clear_hist_temp.set(true));
}

/// Starting to edit the command line: do not clear messages now.
pub fn sb_text_start_cmdline() {
    if MSG_STATE.with(|s| s.do_clear_sb_text.get()) == SbClear::SbClearCmdlineBusy {
        sb_text_restart_cmdline();
    } else {
        msg_sb_eol();
        MSG_STATE.with(|s| s.do_clear_sb_text.set(SbClear::SbClearCmdlineBusy));
    }
}

/// Redrawing the command line: clear the last unfinished line.
pub fn sb_text_restart_cmdline() {
    MSG_STATE.with(|s| s.do_clear_sb_text.set(SbClear::SbClearCmdlineBusy));

    let len = with_msg_ext(|ext| {
        let chunks = ext.msgchunks.borrow();
        let last = chunks.last()?;
        if last.sb_eol {
            return None;
        }
        Some(chunks.len())
    });
    let Some(len) = len else {
        return;
    };

    if let Some(start_idx) = msg_sb_start(Some(len - 1)) {
        with_msg_ext(|ext| ext.msgchunks.borrow_mut().truncate(start_idx));
    }
}

/// Ending to edit the command line: clear old lines but the last one later.
pub fn sb_text_end_cmdline() {
    MSG_STATE.with(|s| s.do_clear_sb_text.set(SbClear::SbClearCmdlineDone));
}

/// Clear any text remembered for scrolling back.
pub fn clear_sb_text(all: bool) {
    if all {
        with_msg_ext(|ext| ext.msgchunks.borrow_mut().clear());
    } else {
        let len = with_msg_ext(|ext| ext.msgchunks.borrow().len());
        if len > 0
            && let Some(start_idx) = msg_sb_start(Some(len - 1))
        {
            with_msg_ext(|ext| {
                ext.msgchunks.borrow_mut().drain(0..start_idx);
            });
        }
    }
}

/// "g<" command.
pub fn show_sb_text() {
    if ui_has(K_UIMESSAGES) {
        let mut ea: Exarg = Exarg::new();
        ea.arg = crate::strings::OwnedSlice::from_cstr(c"");
        ea.skip = true;
        ex_messages(&mut ea);
        return;
    }
    let len = with_msg_ext(|ext| ext.msgchunks.borrow().len());
    let mp = msg_sb_start(len.checked_sub(1));
    if mp.is_none_or(|idx| idx == 0) {
        vim_beep(K_OPT_BO_FLAG_MESS);
    } else {
        do_more_prompt(b'G' as i32);
        wait_return(0);
    }
}

/// Move to the start of screen line in already displayed text.
fn msg_sb_start(mps: Option<usize>) -> Option<usize> {
    let mut idx = mps?;
    with_msg_ext(|ext| {
        let chunks = ext.msgchunks.borrow();
        while idx > 0 && !chunks[idx - 1].sb_eol {
            idx -= 1;
        }
    });
    Some(idx)
}

/// Mark the last message chunk as finishing the line.
pub fn msg_sb_eol() {
    with_msg_ext(|ext| {
        if let Some(last) = ext.msgchunks.borrow_mut().last_mut() {
            last.sb_eol = true;
        }
    });
}

/// Display a screen line from previously displayed text at row "row".
fn disp_sb_line(row: i32, smp: usize) -> Option<usize> {
    let mut idx = smp;
    loop {
        g_st().msg_row.set(row);
        let (sb_msg_col, hl_id, eol, text) = with_msg_ext(|ext| {
            let chunk = &ext.msgchunks.borrow()[idx];
            (
                chunk.sb_msg_col,
                chunk.sb_hl_id,
                chunk.sb_eol,
                chunk.sb_text.clone(),
            )
        });
        g_st().msg_col.set(sb_msg_col);
        msg_puts_display(&text, -1, hl_id, 1);
        let len = with_msg_ext(|ext| ext.msgchunks.borrow().len());
        if eol || idx + 1 >= len {
            break;
        }
        idx += 1;
    }
    if idx + 1 < with_msg_ext(|ext| ext.msgchunks.borrow().len()) {
        Some(idx + 1)
    } else {
        None
    }
}

/// @return  true when messages should be printed to stdout/stderr
pub fn msg_use_printf() -> i32 {
    i32::from(!g_st().embedded_mode.get() && ui_active() == 0 && !ui_has(K_UIMESSAGES))
}

/// Print a message when there is no valid screen.
fn msg_puts_printf(str_: &[u8], maxlen: isize) {
    let slice = if maxlen < 0 {
        crate::memory::bytes_until_nul(str_)
    } else {
        let max_u = (maxlen as usize).min(str_.len());
        crate::memory::bytes_until_nul(&str_[..max_u])
    };
    {
        let mut buf = [0u8; 7];

        if crate::eval::typval::Callback::raw_cb_type(&g_st().on_print) != K_CALLBACK_NONE_CONST {
            let cstr = crate::memory::bytes_to_cstring(slice);
            let mut argv = [TypVal {
                v_type: VarType::VarString,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::from_cstr(&cstr),
            }];
            let mut rettv = TypVal {
                v_type: VarType::VarUnknown,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };
            callback_call(&g_st().on_print, 1, &mut argv, &mut rettv);
            tv_clear(&mut rettv);
            return;
        }

        let mut rem = slice;
        while !rem.is_empty() {
            let len = (utf_ptr2len(rem) as usize).max(1).min(rem.len());
            let first = rem[0];
            if !(g_st().silent_mode.get() && opt_vars().p_verbose.get() == 0) {
                let mut pos = 0usize;
                if first == b'\n'
                    && !g_st().info_message.get()
                    && !g_st().silent_mode.get()
                    && !g_st().headless_mode.get()
                {
                    buf[pos] = b'\r';
                    pos += 1;
                }
                buf[pos..pos + len].copy_from_slice(&rem[..len]);
                buf[pos + len] = 0;
                if g_st().info_message.get() {
                    printf!(c"%s", &buf);
                } else {
                    fprintf!(log_get_stderr(), c"%s", &buf);
                }
            }

            let cw = utf_char2cells(utf_slice2char(rem));
            if matches!(first, b'\r' | b'\n') {
                g_st().msg_col.set(0);
                g_st().msg_didout.set(false);
            } else {
                g_st().msg_col.set(g_st().msg_col.get() + (cw));
                g_st().msg_didout.set(true);
            }
            rem = &rem[len..];
        }
    }
}

/// Show the more-prompt and handle the user response.
fn do_more_prompt(typed_char: i32) -> bool {
    let mut used_typed_char = typed_char;
    let old_state = g_st().state_global.get();
    let mut retval = false;
    let mut to_redraw = false;
    let mut mp_last: Option<usize> = None;

    let no_need_more =
        (g_st().headless_mode.get() && !g_st().embedded_mode.get() && ui_active() == 0)
            || input_unanswerable();

    if no_need_more
        || MSG_STATE.with(|s| s.do_more_prompt_entered.get())
        || (g_st().state_global.get() == MODE_HITRETURN && typed_char == 0)
    {
        return false;
    }
    MSG_STATE.with(|s| s.do_more_prompt_entered.set(true));

    if typed_char == b'G' as i32 {
        let len = with_msg_ext(|ext| ext.msgchunks.borrow().len());
        mp_last = msg_sb_start(len.checked_sub(1));
        let mut i = 0;
        while i < g_st().rows_global.get() - 2
            && let Some(idx) = mp_last
            && idx > 0
        {
            mp_last = msg_sb_start(Some(idx - 1));
            i += 1;
        }
    }

    g_st().state_global.set(MODE_ASKMORE);
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
            get_keystroke(g_st().resize_events.get().opt())
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
                    toscroll = -(g_st().rows_global.get() / 2);
                }
                0x64 /* 'd' */ => {
                    toscroll = g_st().rows_global.get() / 2;
                }
                0x62 /* 'b' */ | CTRL_B_CONST | K_PAGEUP => {
                    toscroll = -(g_st().rows_global.get() - 1);
                }
                0x20 /* ' ' */ | 0x66 /* 'f' */ | CTRL_F_CONST | K_PAGEDOWN | K_LEFTMOUSE => {
                    toscroll = g_st().rows_global.get() - 1;
                }
                0x67 /* 'g' */ => {
                    toscroll = -999999;
                }
                0x47 /* 'G' */ => {
                    toscroll = 999999;
                    g_st().lines_left.set(999999);
                }
                0x3a /* ':' */ => {
                    if MSG_STATE.with(|s| s.confirm_msg_used.get()) == 0 {
                        typeahead_noflush(b':' as i32);
                        g_st().cmdline_row.set(g_st().rows_global.get() - 1);
                        g_st().skip_redraw.set(true);
                        g_st().need_wait_return.set(false);
                    }
                    if MSG_STATE.with(|s| s.confirm_msg_used.get()) != 0 {
                        retval = true;
                    } else {
                        g_st().got_int.set(true);
                        g_st().quit_more.set(true);
                    }
                    g_st().lines_left.set(g_st().rows_global.get() - 1);
                }
                0x71 /* 'q' */ | CTRL_C_CONST | ESC => {
                    if MSG_STATE.with(|s| s.confirm_msg_used.get()) != 0 {
                        retval = true;
                    } else {
                        g_st().got_int.set(true);
                        g_st().quit_more.set(true);
                    }
                    g_st().lines_left.set(g_st().rows_global.get() - 1);
                }
                K_EVENT => {
                    multiqueue_process_events(g_st().resize_events.get().borrow_mut());
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
                    let len = with_msg_ext(|ext| ext.msgchunks.borrow().len());
                    msg_sb_start(len.checked_sub(1))
                };

                let mut i = 0;
                while i < g_st().rows_global.get() - 2
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
                            let len = with_msg_ext(|ext| ext.msgchunks.borrow().len());
                            mp_last = msg_sb_start(len.checked_sub(1));
                        }
                        i -= 1;
                    }

                    if toscroll == -1 && !to_redraw {
                        grid_ins_lines(
                            &g_st().msg_grid,
                            0,
                            1,
                            g_st().rows_global.get(),
                            0,
                            g_st().columns_global.get(),
                        );
                        grid_clear(
                            &g_st().msg_grid_adj,
                            0,
                            1,
                            0,
                            g_st().columns_global.get(),
                            hl_attr(HLF_MSG),
                        );
                        if let Some(cur_idx) = mp {
                            disp_sb_line(0, cur_idx);
                        }
                    } else {
                        grid_clear(
                            &g_st().msg_grid_adj,
                            0,
                            g_st().rows_global.get(),
                            0,
                            g_st().columns_global.get(),
                            hl_attr(HLF_MSG),
                        );
                        let mut i = 0;
                        while let Some(cur_idx) = mp
                            && i < g_st().rows_global.get() - 1
                        {
                            mp = disp_sb_line(i, cur_idx);
                            g_st().msg_scrolled.set(g_st().msg_scrolled.get() + (1));
                            i += 1;
                        }
                        to_redraw = false;
                    }
                    toscroll = 0;
                }
            } else {
                if g_st().cmdline_row.get() >= g_st().rows_global.get() && !ui_has(K_UIMESSAGES) {
                    msg_scroll_up(true, false);
                    g_st().msg_scrolled.set(g_st().msg_scrolled.get() + (1));
                }
                while toscroll > 0
                    && let Some(last_idx) = mp_last
                {
                    if msg_do_throttle() && !g_st().msg_grid.throttled.get() {
                        g_st()
                            .msg_scrolled_at_flush
                            .set(g_st().msg_scrolled_at_flush.get() - (1));
                        g_st()
                            .msg_grid_scroll_discount
                            .set(g_st().msg_grid_scroll_discount.get() + (1));
                    }
                    msg_scroll_up(true, false);
                    inc_msg_scrolled();
                    grid_clear(
                        &g_st().msg_grid_adj,
                        g_st().rows_global.get() - 2,
                        g_st().rows_global.get() - 1,
                        0,
                        g_st().columns_global.get(),
                        hl_attr(HLF_MSG),
                    );
                    mp_last = disp_sb_line(g_st().rows_global.get() - 2, last_idx);
                    toscroll -= 1;
                }
            }

            if toscroll <= 0 {
                grid_clear(
                    &g_st().msg_grid_adj,
                    g_st().rows_global.get() - 1,
                    g_st().rows_global.get(),
                    0,
                    g_st().columns_global.get(),
                    hl_attr(HLF_MSG),
                );
                msg_moremsg(false);
                continue;
            }

            g_st().lines_left.set(toscroll);
        }

        break;
    }

    // clear the --more-- message
    grid_clear(
        &g_st().msg_grid_adj,
        g_st().rows_global.get() - 1,
        g_st().rows_global.get(),
        0,
        g_st().columns_global.get(),
        hl_attr(HLF_MSG),
    );
    g_st().redraw_cmdline.set(true);
    g_st().clear_cmdline.set(false);
    g_st().mode_displayed.set(false);

    g_st().state_global.set(old_state);
    setmouse();
    if g_st().quit_more.get() {
        g_st().msg_row.set(g_st().rows_global.get() - 1);
        g_st().msg_col.set(0);
    }

    MSG_STATE.with(|s| s.do_more_prompt_entered.set(false));
    retval
}

fn msg_moremsg(full: bool) {
    let attr = hl_combine_attr(hl_attr(HLF_MSG), hl_attr(HLF_M));
    grid_line_start(&g_st().msg_grid_adj, g_st().rows_global.get() - 1);
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

    if g_st().state_global.get() == MODE_ASKMORE {
        msg_moremsg(true);
        g_st().msg_row.set(g_st().rows_global.get() - 1);
    } else if (g_st().state_global.get() & MODE_CMDLINE) != 0 && has_confirm_msg() {
        display_confirm_msg();
        g_st().msg_row.set(g_st().rows_global.get() - 1);
    } else if g_st().state_global.get() == MODE_EXTERNCMD {
        ui_cursor_goto(g_st().msg_row.get(), g_st().msg_col.get());
    } else if g_st().state_global.get() == MODE_HITRETURN
        || g_st().state_global.get() == MODE_SETWSIZE
    {
        if g_st().msg_row.get() == g_st().rows_global.get() - 1 {
            g_st().msg_didout.set(false);
            g_st().msg_col.set(0);
            msg_clr_eos();
        }
        hit_return_msg(false);
        g_st().msg_row.set(g_st().rows_global.get() - 1);
    }
}

/// Clear from current message position to end of screen.
/// Skip this when ":silent" was used, no need to clear for redirection.
pub fn msg_clr_eos() {
    if g_st().msg_silent.get() == 0 {
        msg_clr_eos_force();
    }
}

/// Clear from current message position to end of screen.
pub fn msg_clr_eos_force() {
    if ui_has(K_UIMESSAGES) {
        return;
    }
    let msg_startcol = if g_st().cmdmsg_rl.get() {
        0
    } else {
        g_st().msg_col.get()
    };
    let msg_endcol = if g_st().cmdmsg_rl.get() {
        g_st().columns_global.get() - g_st().msg_col.get()
    } else {
        g_st().columns_global.get()
    };

    if crate::grid::grid_allocated(&g_st().msg_grid)
        && g_st().msg_row.get() < g_st().msg_grid_pos.get()
    {
        msg_grid_validate();
        if g_st().msg_row.get() < g_st().msg_grid_pos.get() {
            g_st().msg_row.set(g_st().msg_grid_pos.get());
        }
    }

    grid_clear(
        &g_st().msg_grid_adj,
        g_st().msg_row.get(),
        g_st().msg_row.get() + 1,
        msg_startcol,
        msg_endcol,
        hl_attr(HLF_MSG),
    );
    grid_clear(
        &g_st().msg_grid_adj,
        g_st().msg_row.get() + 1,
        g_st().rows_global.get(),
        0,
        g_st().columns_global.get(),
        hl_attr(HLF_MSG),
    );

    g_st().redraw_cmdline.set(true);
    if g_st().msg_row.get() < g_st().rows_global.get() - 1 || g_st().msg_col.get() == 0 {
        g_st().clear_cmdline.set(false);
        g_st().mode_displayed.set(false);
        g_st().cmdline_was_last_drawn.set(false);
    }
}

/// Clear the command line.
pub fn msg_clr_cmdline() {
    g_st().msg_row.set(g_st().cmdline_row.get());
    g_st().msg_col.set(0);
    msg_clr_eos_force();
}

/// end putting a message on the screen
/// call wait_return() if the message does not fit in the available space
///
/// @return  true if wait_return() not called.
pub fn msg_end() -> bool {
    if !g_st().exiting.get()
        && g_st().need_wait_return.get()
        && (g_st().state_global.get() & MODE_CMDLINE) == 0
    {
        wait_return(0);
        return false;
    }

    msg_ext_ui_flush();
    true
}

/// Clear "msg_ext_chunks" before flushing so that ui_flush() does not re-emit
/// the same message recursively.
fn msg_ext_init_chunks() -> Array {
    let tofree = MSG_STATE.with(|s| s.msg_ext_chunks.replace(Array::new()));
    g_st().msg_col.set(0);
    tofree
}

pub fn msg_ext_ui_flush() {
    if !ui_has(K_UIMESSAGES) {
        with_msg_ext(|ext| *ext.msg_ext_kind.borrow_mut() = None);
        return;
    } else if g_st().msg_ext_skip_flush.get() {
        return;
    }

    msg_ext_emit_chunk();
    if MSG_STATE.with(|s| s.msg_ext_chunks.get()).size > 0 {
        let tofree = msg_ext_init_chunks();
        let (kind_cstr, trigger_cstr) = with_msg_ext(|ext| {
            (
                ext.msg_ext_kind.borrow().clone(),
                ext.msg_ext_trigger.borrow().clone(),
            )
        });
        let kind_str = cstr_as_string(kind_cstr.as_deref());
        let trigger_str = cstr_as_string(trigger_cstr.as_deref());

        ui_call_msg_show(
            kind_str,
            tofree.clone(),
            g_st().msg_ext_overwrite.get(),
            MSG_STATE.with(|s| s.msg_ext_history.get()),
            MSG_STATE.with(|s| s.msg_ext_append.get()),
            MSG_STATE.with(|s| s.msg_ext_id.get()),
            trigger_str,
        );
        if MSG_STATE.with(|s| s.msg_ext_history.get()) {
            api_free_array(tofree);
        } else {
            // Add to history as temporary message for "g<".
            let mut msg = HlMessage::new();
            for i in 0..tofree.size {
                let mut chunk = tofree[i].data.array();
                let text = chunk[1].data.string();
                chunk[1].data = ObjectData::String(NvimString::new());

                msg.push(HlMessageChunk {
                    text,
                    hl_id: chunk[2].data.integer() as i32,
                });
            }
            api_free_array(tofree);
            msg_hist_add_multihl(msg, true);
        }
        g_st().msg_ext_overwrite.set(false);
        MSG_STATE.with(|s| s.msg_ext_history.set(false));
        MSG_STATE.with(|s| s.msg_ext_append.set(false));
        g_st().msg_ext_fast.set(true);
        with_msg_ext(|ext| *ext.msg_ext_kind.borrow_mut() = None);
        let next_id = MSG_STATE.with(|st| {
            let ext_id = st.msg_ext_id.get();
            if ext_id.obj_type == ObjectType::ObjectTypeInteger
                && ext_id.data.integer() == st.msg_id_next.get()
            {
                st.msg_id_next.set(st.msg_id_next.get() + 1);
            }
            st.msg_id_next.get()
        });
        MSG_STATE.with(|s| {
            s.msg_ext_id.set(Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer(next_id),
            })
        });
    }
}

pub fn msg_ext_flush_showmode() {
    let has_last_attr = MSG_STATE.with(|s| s.msg_ext_last_attr.get()) != -1;
    if ui_has(K_UIMESSAGES) && (has_last_attr || MSG_STATE.with(|s| s.showmode_clear.get())) {
        MSG_STATE.with(|s| s.showmode_clear.set(has_last_attr));
        msg_ext_emit_chunk();
        let tofree = msg_ext_init_chunks();
        ui_call_msg_showmode(tofree.clone());
        api_free_array(tofree);
    }
}

/// If the written message runs into the shown command or ruler, we have to
/// wait for hit-return and redraw the window later.
pub fn msg_check() {
    if ui_has(K_UIMESSAGES) {
        return;
    }
    if g_st().msg_row.get() == g_st().rows_global.get() - 1
        && g_st().msg_col.get() >= g_st().sc_col.get()
    {
        g_st().need_wait_return.set(true);
        g_st().redraw_cmdline.set(true);
    }
}

/// May write a string to the redirection file.
fn redir_write(str_: &[u8], maxlen: isize) {
    let slice = if maxlen < 0 {
        crate::memory::bytes_until_nul(str_)
    } else {
        let max_u = (maxlen as usize).min(str_.len());
        crate::memory::bytes_until_nul(&str_[..max_u])
    };
    {
        if maxlen == 0 || slice.is_empty() {
            return;
        }

        if g_st().redir_off.get() {
            return;
        }

        if !opt_vars().p_vfile.is_empty() && with_msg_ext(|ext| ext.verbose_fd.borrow().is_none()) {
            verbose_open();
        }

        if redirecting() != 0 {
            if !matches!(slice[0], b'\n' | b'\r') {
                while MSG_STATE.with(|s| s.redir_col.get()) < g_st().msg_col.get() {
                    if let Some(ref mut cap) = *g_st().capture_ga.borrow_mut() {
                        cap.push(b' ');
                    }
                    if g_st().redir_reg.get() != 0 {
                        write_reg_contents(g_st().redir_reg.get(), b" ", 1, true);
                    } else if g_st().redir_vname.get() {
                        var_redir_str(b" ");
                    } else if let Some(ref rf) = *g_st().redir_fd.borrow() {
                        rf.puts(c" ");
                    }
                    with_msg_ext(|ext| {
                        if let Some(ref vf) = *ext.verbose_fd.borrow() {
                            vf.puts(c" ");
                        }
                    });
                    MSG_STATE.with(|s| s.redir_col.set(s.redir_col.get() + (1)));
                }
            }

            let len = slice.len();
            if len > 0
                && let Some(ref mut cap) = *g_st().capture_ga.borrow_mut()
            {
                cap.extend_from_slice(slice);
            }
            if g_st().redir_reg.get() != 0 {
                write_reg_contents(g_st().redir_reg.get(), slice, len as isize, true);
            }
            if g_st().redir_vname.get() {
                var_redir_str(slice);
            }

            for &b in slice {
                if g_st().redir_reg.get() == 0
                    && !g_st().redir_vname.get()
                    && g_st().capture_ga.borrow().is_none()
                    && let Some(ref rf) = *g_st().redir_fd.borrow()
                {
                    rf.putc(i32::from(b));
                }
                with_msg_ext(|ext| {
                    if let Some(ref vf) = *ext.verbose_fd.borrow() {
                        vf.putc(i32::from(b));
                    }
                });
                if matches!(b, b'\r' | b'\n') {
                    MSG_STATE.with(|s| s.redir_col.set(0));
                } else if i32::from(b) == TAB {
                    let rc = MSG_STATE.with(|s| s.redir_col.get());
                    MSG_STATE.with(|s| s.redir_col.set(rc + 8 - rc % 8));
                } else {
                    MSG_STATE.with(|s| s.redir_col.set(s.redir_col.get() + (1)));
                }
            }

            if g_st().msg_silent.get() != 0 {
                g_st().msg_col.set(MSG_STATE.with(|s| s.redir_col.get()));
            }
        }
    }
}

pub fn redirecting() -> i32 {
    i32::from(
        has_redir_fd()
            || !opt_vars().p_vfile.is_empty()
            || g_st().redir_reg.get() != 0
            || g_st().redir_vname.get()
            || g_st().capture_ga.borrow().is_some(),
    )
}

/// Before giving verbose message.
/// Must always be called paired with verbose_leave()!
pub fn verbose_enter() {
    if !opt_vars().p_vfile.is_empty() {
        g_st().msg_silent.set(g_st().msg_silent.get() + (1));
    }
    if !g_st().msg_ext_skip_verbose.get() {
        with_msg_ext(|ext| {
            if ext.msg_ext_kind.borrow().as_deref() != Some(VERBOSE_KIND) {
                *ext.pre_verbose_kind.borrow_mut() = ext.msg_ext_kind.borrow().clone();
            }
        });
        msg_ext_set_kind(VERBOSE_KIND);
    }
    g_st().msg_ext_skip_verbose.set(false);
}

/// After giving verbose message.
/// Must always be called paired with verbose_enter()!
pub fn verbose_leave() {
    if !opt_vars().p_vfile.is_empty() {
        g_st().msg_silent.set(g_st().msg_silent.get() - (1));
        if g_st().msg_silent.get() < 0 {
            g_st().msg_silent.set(0);
        }
    }
    if let Some(prev_kind) = with_msg_ext(|ext| ext.pre_verbose_kind.borrow_mut().take()) {
        msg_ext_set_kind(&prev_kind);
    }
}

/// Like verbose_enter() and set msg_scroll when displaying the message.
pub fn verbose_enter_scroll() {
    verbose_enter();
    if opt_vars().p_vfile.is_empty() {
        g_st().msg_scroll.set(1);
    }
}

/// Like verbose_leave() and set cmdline_row when displaying the message.
pub fn verbose_leave_scroll() {
    verbose_leave();
    if opt_vars().p_vfile.is_empty() {
        g_st().cmdline_row.set(g_st().msg_row.get());
    }
}

/// Called when 'verbosefile' is set: stop writing to the file.
pub fn verbose_stop() {
    with_msg_ext(|ext| *ext.verbose_fd.borrow_mut() = None);
    MSG_STATE.with(|s| s.verbose_did_open.set(false));
}

/// Open the file 'verbosefile'.
///
/// @return  FAIL or OK.
pub fn verbose_open() -> bool {
    if with_msg_ext(|ext| ext.verbose_fd.borrow().is_none())
        && !MSG_STATE.with(|s| s.verbose_did_open.get())
    {
        MSG_STATE.with(|s| s.verbose_did_open.set(true));

        let opened = CFile::open(opt_vars().p_vfile.as_cstr(), c"a");
        let is_none = opened.is_none();
        with_msg_ext(|ext| *ext.verbose_fd.borrow_mut() = opened);
        if is_none {
            semsg!(gettext(E_NOTOPEN), &opt_vars().p_vfile);
            return false;
        }
    }
    true
}

/// Give a warning message (for searching).
/// Use 'w' highlighting and may repeat the message after redrawing
pub fn give_warning(message: &std::ffi::CStr, hl: bool, hist: bool) {
    if g_st().msg_silent.get() != 0 {
        return;
    }

    let save_msg_hist_off = g_st().msg_hist_off.get();
    g_st().msg_hist_off.set(!hist);

    g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));

    set_vim_var_string(VV_WARNINGMSG, Some(message), -1);
    g_st().keep_msg.set(None);
    if hl {
        g_st().keep_msg_hl_id.set(HLF_W);
    } else {
        g_st().keep_msg_hl_id.set(0);
    }

    if with_msg_ext(|ext| ext.msg_ext_kind.borrow().is_none()) {
        msg_ext_set_kind(c"wmsg");
    }

    if msg(message, g_st().keep_msg_hl_id.get()) && g_st().msg_scrolled.get() == 0 {
        set_keep_msg(Some(message), g_st().keep_msg_hl_id.get());
    }
    g_st().msg_didout.set(false);
    g_st().msg_nowait.set(true);
    g_st().msg_col.set(0);

    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    g_st().msg_hist_off.set(save_msg_hist_off);
}

/// Advance msg cursor to column "col".
pub fn msg_advance(mut col: i32) {
    if g_st().msg_silent.get() != 0 {
        g_st().msg_col.set(col);
        return;
    }
    col = col.min(g_st().columns_global.get() - 1);
    while g_st().msg_col.get() < col {
        msg_putchar(b' ' as i32);
    }
}

/// Display the ":confirm" message.  Also called from do_dialog() and when screen resized.
pub fn display_confirm_msg() {
    MSG_STATE.with(|s| s.confirm_msg_used.set(s.confirm_msg_used.get() + (1)));
    if let Some(msg) = clone_confirm_msg() {
        msg_ext_set_kind(c"confirm");
        msg_puts_hl(msg.as_c_str(), HLF_M, false);
    }
    MSG_STATE.with(|s| s.confirm_msg_used.set(s.confirm_msg_used.get() - (1)));
}

/// Only for legacy UI (`!ui_has(K_UIMESSAGES)`): Pause to display a message for `ms` milliseconds.
pub fn msg_delay(mut ms: u64, ignoreinput: bool) {
    if ui_has(K_UIMESSAGES) {
        return;
    }

    if g_st().nvim_testing.get() {
        ms = 100;
    }

    logmsg!(
        LOGLVL_DBG,
        None,
        Some(c"msg_delay"),
        line!() as i32,
        true,
        c"%llu ms%s",
        ms,
        if g_st().nvim_testing.get() {
            c" (skipped by NVIM_TEST)"
        } else {
            c""
        },
    );
    ui_flush();
    os_delay(ms, ignoreinput);
}

/// Check if there should be a delay to allow the user to see a message.
pub fn msg_check_for_delay(check_msg_scroll: bool) {
    if (g_st().emsg_on_display.get() || (check_msg_scroll && g_st().msg_scroll.get() != 0))
        && !g_st().did_wait_return.get()
        && g_st().emsg_silent.get() == 0
        && !g_st().in_assert_fails.get()
        && !ui_has(K_UIMESSAGES)
    {
        msg_delay(1006, true);
        g_st().emsg_on_display.set(false);
        if check_msg_scroll {
            g_st().msg_scroll.set(0);
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
            let mut buf = [0u8; 40];
            let c_s = std::ffi::CString::new(s).unwrap();
            trunc_string(Some(c_s.as_bytes()), &mut buf, room);
            assert_eq!(
                CStr::from_bytes_until_nul(&buf).unwrap().to_str().unwrap(),
                expected
            );

            let mut inplace = [0u8; 40];
            inplace[..s.len() + 1].copy_from_slice(c_s.as_bytes_with_nul());
            trunc_string(None, &mut inplace, room);
            assert_eq!(
                CStr::from_bytes_until_nul(&inplace)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                expected
            );
        }
    }
}
