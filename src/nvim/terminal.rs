//! VT220/xterm-like terminal emulator (`src/nvim/terminal.c`).

use crate::option::{K_OPT_BUFTYPE, OPT_LOCAL, OptInt, SB_MAX, set_option_value};
use crate::vterm::keyboard::VTERM_KEY_KP_0;
use crate::vterm::state::VTERM_SELECTION_CLIPBOARD;
use crate::vterm::state::VTERM_SELECTION_PRIMARY;

use crate::api::private::helpers::{
    api_clear_error, api_free_object, dict_get_value, dict_set_var, integer_obj, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::api::tabpage::Buffer;
use crate::autocmd::{
    AUGROUP_ALL, EVENT_TERMCLOSE, EVENT_TERMENTER, EVENT_TERMLEAVE, EVENT_TERMOPEN,
    EVENT_TERMREQUEST, EVENT_TEXTCHANGEDT, apply_autocmds, apply_autocmds_group, block_autocmds,
    has_event, is_autocmd_blocked, unblock_autocmds,
};
use crate::buffer::{DOBUF_FIRST, DOBUF_WIPE, FileBuf, MAXLNUM, buf_get_changedtick, do_buffer};
use crate::change::{appended_lines_buf, changed_lines, deleted_lines_buf};
use crate::channel::{
    TerminalOptions, channel_term_close, channel_term_read_pause, channel_term_resize,
    channel_term_resume, channel_term_write,
};
use crate::charset::{NUL, is_special};
use crate::context::{
    CtxSwitch, ctx_restore, ctx_switch, handle_get_buffer, handle_get_window, is_ctx_win,
};
use crate::cursor::{CursorPos, Linenr, coladvance};
use crate::cursor_shape::{
    CursorShape, SHAPE_CURSOR, SHAPE_IDX_TERM, parse_shape_opt, shape_table_mut,
};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, redraw_buf_line_later, redraw_later,
    redraw_statuslines, setcursor, show_cursor_info_later, showmode, status_redraw_buf, unshowmode,
    update_screen,
};
use crate::eval::typval::{
    tv_dict_add_nr, tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_list,
    tv_list_append_string,
};
use crate::eval::vars::{VV_TERMREQUEST, get_globvar_dict, set_vim_var_string};
use crate::eval::{K_LIST_LEN_MAY_KNOW_CONST, eval_call_provider, get_v_event, restore_v_event};
use crate::event::r#loop::loop_queues;
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_free, multiqueue_move_events, multiqueue_new,
    multiqueue_new_child, multiqueue_process_events, multiqueue_put_event,
};
use crate::event::time::{
    TimeWatcher, TimeWatcherData, time_watcher_close, time_watcher_init, time_watcher_start,
    time_watcher_stop,
};
use crate::ex_docmd::do_cmdline;
use crate::globals::*;
use crate::grid::schar_get_adv;
use crate::highlight::HlAttrs;
use crate::highlight::{
    HL_BG_INDEXED, HL_FG_INDEXED, hl_add_url, hl_combine_attr, hl_get_term_attr,
};
use crate::highlight_group::name_to_color;
use crate::input::{K_PASTE_START, map_execute_lua, merge_modifiers, paste_repeat, requeue_key};
use crate::input_cmdatom::{Cmdarg, atom_term_enter};
use crate::keycodes::{
    ESC, K_BS, K_COMMAND, K_DEL, K_DOWN, K_END, K_F1, K_F2, K_F3, K_F4, K_F5, K_F6, K_F7, K_F8,
    K_F9, K_F10, K_F11, K_F12, K_F13, K_F14, K_F15, K_F16, K_F17, K_F18, K_F19, K_F20, K_F21,
    K_F22, K_F23, K_F24, K_F25, K_F26, K_F27, K_F28, K_F29, K_F30, K_F31, K_F32, K_F33, K_F34,
    K_F35, K_F36, K_F37, K_F38, K_F39, K_F40, K_F41, K_F42, K_F43, K_F44, K_F45, K_F46, K_F47,
    K_F48, K_F49, K_F50, K_F51, K_F52, K_F53, K_F54, K_F55, K_F56, K_F57, K_F58, K_F59, K_F60,
    K_F61, K_F62, K_F63, K_HOME, K_IGNORE, K_INS, K_K0, K_K1, K_K2, K_K3, K_K4, K_K5, K_K6, K_K7,
    K_K8, K_K9, K_KDEL, K_KDIVIDE, K_KDOWN, K_KEND, K_KENTER, K_KHOME, K_KINS, K_KLEFT, K_KMINUS,
    K_KMULTIPLY, K_KORIGIN, K_KPAGEDOWN, K_KPAGEUP, K_KPLUS, K_KPOINT, K_KRIGHT, K_KUP, K_LEFT,
    K_LEFTDRAG, K_LEFTMOUSE, K_LEFTRELEASE, K_MIDDLEDRAG, K_MIDDLEMOUSE, K_MIDDLERELEASE,
    K_MOUSEDOWN, K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP, K_NOP, K_PAGEDOWN, K_PAGEUP,
    K_RIGHT, K_RIGHTDRAG, K_RIGHTMOUSE, K_RIGHTRELEASE, K_TAB, K_UP, K_X1DRAG, K_X1MOUSE,
    K_X1RELEASE, K_X2DRAG, K_X2MOUSE, K_X2RELEASE, K_ZERO, MOD_MASK_ALT, MOD_MASK_CTRL,
    MOD_MASK_SHIFT, TAB, unsimplify_key,
};
use crate::mark::{K_MARK_ADJUST_TERM, mark_adjust_buf};
use crate::mbyte::{mb_check_adjust_col, utf_ptr2char, utf_ptr2len};
use crate::memline::{ml_append_buf, ml_delete_buf, ml_replace_buf};
use crate::mouse::{
    MSCR_DOWN, MSCR_LEFT, MSCR_RIGHT, MSCR_UP, do_mousescroll, mouse_find_win_inner,
};
use crate::r#move::{curs_columns, set_topline, validate_cursor, win_col_off};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::option_vars::*;
use crate::optionstr::{alloc_string_option, free_string_option};
use crate::state::{
    EditorState, MODE_TERMINAL, SaveVEvent, may_trigger_modechanged, state_enter,
    state_handle_k_event,
};
use crate::strings::vim_snprintf;
use crate::textformat::Oparg;
use crate::ui::{
    ui_busy_start, ui_busy_stop, ui_cursor_shape, ui_flush, ui_mode_info_set, vim_beep,
};
use crate::vterm::keyboard::{
    VTERM_KEY_BACKSPACE, VTERM_KEY_ENTER, VTERM_KEY_FUNCTION_0, VTERM_KEY_NONE, VTERM_KEY_TAB,
    VTERM_MOD_ALT, VTERM_MOD_CTRL, VTERM_MOD_NONE, VTERM_MOD_SHIFT, VTermColor, VTermKey,
    VTermModifier, VTermStateFull, vterm_keyboard_end_paste, vterm_keyboard_key,
    vterm_keyboard_start_paste, vterm_keyboard_unichar,
};
use crate::vterm::mouse::{VTerm, VTermPos, vterm_mouse_button, vterm_mouse_move};
use crate::vterm::parser::{
    VTERM_TERMINATOR_BEL, VTermStringFragment, VTermTerminator, vterm_input_write,
};
use crate::vterm::pen::{
    VTERM_ATTR_URI, VTERM_COLOR_DEFAULT_BG, VTERM_COLOR_DEFAULT_FG, VTERM_COLOR_INDEXED,
    VTERM_UNDERLINE_CURLY, VTERM_UNDERLINE_DOUBLE, VTERM_UNDERLINE_SINGLE, VTERM_VALUETYPE_INT,
    VTermProp, VTermRect, VTermValue, vterm_state_convert_color_to_rgb,
    vterm_state_set_palette_color, vterm_state_set_penattr,
};
use crate::vterm::screen::{
    VTERM_DAMAGE_SCROLL, VTERM_PROP_ALTSCREEN, VTermScreenCallbacks, VTermScreenCell,
    VTermScreenCellAttrs, vterm_obtain_screen, vterm_screen_enable_altscreen,
    vterm_screen_enable_reflow, vterm_screen_flush_damage, vterm_screen_get_cell,
    vterm_screen_reset, vterm_screen_set_callbacks, vterm_screen_set_damage_merge,
    vterm_screen_set_unrecognised_fallbacks,
};
use crate::vterm::state::{
    VTERM_PROP_CURSORBLINK, VTERM_PROP_CURSORSHAPE, VTERM_PROP_CURSORSHAPE_BAR_LEFT,
    VTERM_PROP_CURSORSHAPE_BLOCK, VTERM_PROP_CURSORSHAPE_UNDERLINE, VTERM_PROP_CURSORVISIBLE,
    VTERM_PROP_MOUSE, VTERM_PROP_SYNCOUTPUT, VTERM_PROP_THEMEUPDATES, VTERM_PROP_TITLE,
    VTermSelectionCallbacks, VTermSelectionMask, VTermStateFallbacks, vterm_obtain_state,
    vterm_state_focus_in, vterm_state_focus_out, vterm_state_set_selection_callbacks,
    vterm_state_set_termprop,
};
use crate::vterm::vterm::{
    vterm_get_size, vterm_new, vterm_output_set_callback, vterm_set_size, vterm_set_utf8,
};
use crate::window::{Winopt, may_trigger_win_scrolled_resized, win_valid};
use std::ffi::c_char;
use std::ptr;

use crate::arglist::ML_EMPTY;
use crate::cmdexpand::CTRL_N;
use crate::drawline::{HL_OVERLINE, HL_STRIKETHROUGH, K_OPT_CULOPT_FLAG_NUMBER};
use crate::eval::funcs::{
    HL_BLINK, HL_BOLD, HL_CONCEALED, HL_DIM, HL_INVERSE, HL_ITALIC, HL_UNDERCURL, HL_UNDERDOUBLE,
    HL_UNDERLINE,
};
use crate::ex_cmds::CTRL_C_CONST;
use crate::ex_getln::{CTRL_BSL, CTRL_O, K_EVENT, K_LUA};
use crate::extmark::K_EXTMARK_UNDO;
use crate::search::FORWARD;

const REFRESH_DELAY: u64 = 10;
const TEXTBUF_SIZE: usize = 0x1fff;
const SELECTIONBUF_SIZE: usize = 0x0400;

const K_OPT_BO_FLAG_TERM: u32 = 0x40000;

const K_OPT_TPF_FLAG_BS: u32 = 0x01;
const K_OPT_TPF_FLAG_HT: u32 = 0x02;
const K_OPT_TPF_FLAG_FF: u32 = 0x04;
const K_OPT_TPF_FLAG_ESC: u32 = 0x08;
const K_OPT_TPF_FLAG_DEL: u32 = 0x10;
const K_OPT_TPF_FLAG_C0: u32 = 0x20;
const K_OPT_TPF_FLAG_C1: u32 = 0x40;

const VTERM_UNDERLINE_OFF: u32 = 0;

const VTERM_KEY_ESCAPE: VTermKey = 4;
const VTERM_KEY_UP: VTermKey = 5;
const VTERM_KEY_DOWN: VTermKey = 6;
const VTERM_KEY_LEFT: VTermKey = 7;
const VTERM_KEY_RIGHT: VTermKey = 8;
const VTERM_KEY_INS: VTermKey = 9;
const VTERM_KEY_DEL: VTermKey = 10;
const VTERM_KEY_HOME: VTermKey = 11;
const VTERM_KEY_END: VTermKey = 12;
const VTERM_KEY_PAGEUP: VTermKey = 13;
const VTERM_KEY_PAGEDOWN: VTermKey = 14;
const VTERM_KEY_KP_1: VTermKey = 513;
const VTERM_KEY_KP_2: VTermKey = 514;
const VTERM_KEY_KP_3: VTermKey = 515;
const VTERM_KEY_KP_4: VTermKey = 516;
const VTERM_KEY_KP_5: VTermKey = 517;
const VTERM_KEY_KP_6: VTermKey = 518;
const VTERM_KEY_KP_7: VTermKey = 519;
const VTERM_KEY_KP_8: VTermKey = 520;
const VTERM_KEY_KP_9: VTermKey = 521;
const VTERM_KEY_KP_MULT: VTermKey = 522;
const VTERM_KEY_KP_PLUS: VTermKey = 523;
const VTERM_KEY_KP_MINUS: VTermKey = 525;
const VTERM_KEY_KP_PERIOD: VTermKey = 526;
const VTERM_KEY_KP_DIVIDE: VTermKey = 527;
const VTERM_KEY_KP_ENTER: VTermKey = 528;

const CTRL_AT: i32 = 0;
const CTRL_M: i32 = 13;

const TERM_ATTRS_MAX: i32 = 1024;

#[repr(C)]
struct TerminalState {
    term: *mut Terminal,
    save_rd: i32,
    close: bool,
    got_bsl: bool,
    got_bsl_o: bool,
    cursor_visible: bool,
    save_curwin_handle: ObjHandle,
    save_w_p_cul: bool,
    save_w_p_culopt: Option<std::ffi::CString>,
    save_w_p_culopt_flags: u8,
    save_w_p_cuc: i32,
    save_w_p_so: OptInt,
    save_w_p_siso: OptInt,
}

impl TerminalState {
    #[inline]
    fn term_mut(&self) -> &'static mut Terminal {
        unsafe { &mut *self.term }
    }
}

impl TerminalState {
    const fn new() -> Self {
        Self {
            term: ptr::null_mut(),
            save_rd: 0,
            close: false,
            got_bsl: false,
            got_bsl_o: false,
            cursor_visible: false,
            save_curwin_handle: 0,
            save_w_p_cul: false,
            save_w_p_culopt: None,
            save_w_p_culopt_flags: 0,
            save_w_p_cuc: 0,
            save_w_p_so: 0,
            save_w_p_siso: 0,
        }
    }
}

impl EditorState for TerminalState {
    fn check(&mut self) -> i32 {
        terminal_check(self)
    }

    fn execute(&mut self, key: i32) -> i32 {
        terminal_execute(self, key)
    }
}

impl Default for TerminalState {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Default)]
struct TermCursor {
    row: i32,
    col: i32,
    shape: i32,
    visible: bool,
    blink: bool,
}

impl TermCursor {
    const fn new() -> Self {
        Self {
            row: 0,
            col: 0,
            shape: 0,
            visible: false,
            blink: false,
        }
    }
}

#[repr(C)]
struct TermPending {
    resize: bool,
    cursor: bool,
    send: Option<std::rc::Rc<std::cell::RefCell<Vec<c_char>>>>,
    events: *mut MultiQueue,
}

impl TermPending {
    const fn new() -> Self {
        Self {
            resize: false,
            cursor: false,
            send: None,
            events: ptr::null_mut(),
        }
    }
}

impl Default for TermPending {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct Terminal {
    opts: TerminalOptions,
    vt: VTerm,
    textbuf: [c_char; TEXTBUF_SIZE],
    sb_buffer: Vec<Vec<VTermScreenCell>>,
    sb_allocated: bool,
    sb_size: usize,
    sb_pending: i32,
    sb_deleted: usize,
    old_sb_deleted: usize,
    old_height: i32,
    title: Vec<c_char>,
    buf_handle: ObjHandle,
    in_altscreen: bool,
    suspended: bool,
    closed: bool,
    destroy: bool,
    forward_mouse: bool,
    invalid_start: i32,
    invalid_end: i32,
    cursor: TermCursor,
    pending: TermPending,
    streamed_paste: bool,
    theme_updates: bool,
    synchronized_output: bool,
    sync_flush_pending: bool,
    color_set: [bool; 16],
    selection: Vec<c_char>,
    termrequest_buffer: Vec<c_char>,
    termrequest_terminator: VTermTerminator,
    refcount: usize,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        multiqueue_free(self.pending.events);
        self.pending.events = ptr::null_mut();
    }
}

struct TerminalModuleState {
    refresh_timer: TimeWatcher,
    refresh_pending: bool,
    invalidated_terminals: Vec<*mut Terminal>,
}

impl TerminalModuleState {
    const fn new() -> Self {
        Self {
            refresh_timer: TimeWatcher::new(),
            refresh_pending: false,
            invalidated_terminals: Vec::new(),
        }
    }
}

static TERM_MOD_STATE: crate::globals::GlobalState<TerminalModuleState> =
    crate::globals::GlobalState::new(TerminalModuleState::new());

#[inline]
fn term_st() -> &'static mut TerminalModuleState {
    unsafe { &mut *TERM_MOD_STATE.as_ptr() }
}

static VTERM_SCREEN_CALLBACKS: VTermScreenCallbacks = VTermScreenCallbacks {
    damage: Some(term_damage),
    moverect: Some(term_moverect),
    movecursor: Some(term_movecursor),
    settermprop: Some(term_settermprop),
    bell: Some(term_bell),
    resize: None,
    theme: Some(term_theme),
    sb_pushline: Some(term_sb_push),
    sb_popline: Some(term_sb_pop),
    sb_clear: Some(term_sb_clear),
};

static VTERM_SELECTION_CALLBACKS: VTermSelectionCallbacks = VTermSelectionCallbacks {
    set: Some(term_selection_set),
    query: None,
};

static VTERM_FALLBACKS: VTermStateFallbacks = VTermStateFallbacks {
    control: None,
    csi: None,
    osc: Some(on_osc),
    dcs: Some(on_dcs),
    apc: Some(on_apc),
    pm: None,
    sos: None,
};

#[inline]
fn invalidate_terminal_add(term: &mut Terminal) {
    let term_ptr = &raw mut *term;
    if !term_st().invalidated_terminals.contains(&term_ptr) {
        term_st().invalidated_terminals.push(term_ptr);
    }
}

#[inline]
const fn vterm_key_function(n: i32) -> VTermKey {
    VTERM_KEY_FUNCTION_0 + n
}

#[inline]
fn refresh_timer_ptr() -> &'static mut TimeWatcher {
    &mut term_st().refresh_timer
}

#[inline]
const fn cstr_slice_as_string(bytes: &'static [u8]) -> NvimString {
    NvimString {
        data: bytes.as_ptr().cast::<c_char>().cast_mut(),
        size: bytes.len() - 1,
    }
}

pub struct TermRequestEvent {
    pub buf_handle: ObjHandle,
    pub sequence: Vec<c_char>,
    pub pending_send: std::rc::Rc<std::cell::RefCell<Vec<c_char>>>,
    pub row: i32,
    pub col: i32,
    pub sb_deleted: usize,
    pub terminator: VTermTerminator,
}

pub fn emit_termrequest(mut req: Box<TermRequestEvent>) {
    unsafe {
        let buf_handle = req.buf_handle;

        let buf = handle_get_buffer(buf_handle);
        if buf.is_null() || (*buf).terminal.is_null() {
            return;
        }
        let term = &mut *(*buf).terminal;

        if term.sb_pending > 0 {
            multiqueue_put_event(&mut *term.pending.events, Event::TermRequest(req));
            return;
        }

        let sequence_length = req.sequence.len();
        let sequence = if sequence_length == 0 {
            c"".as_ptr().cast_mut()
        } else {
            req.sequence.as_mut_ptr()
        };
        let row = req.row;
        let col = req.col;
        let sb_deleted = req.sb_deleted;
        let terminator = req.terminator;

        set_vim_var_string(VV_TERMREQUEST, sequence, sequence_length as isize);

        let mut cursor_items = [
            integer_obj(i64::from(row) - (term.sb_deleted.wrapping_sub(sb_deleted) as i64)),
            integer_obj(i64::from(col)),
        ];
        let cursor = Array {
            items: cursor_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };

        let termrequest = NvimString {
            data: sequence,
            size: sequence_length,
        };
        let term_obj = if terminator == VTERM_TERMINATOR_BEL {
            string_obj(cstr_slice_as_string(b"\x07\0"))
        } else {
            string_obj(cstr_slice_as_string(b"\x1b\\\0"))
        };
        let mut data_items = [
            KeyValuePair {
                key: cstr_slice_as_string(b"sequence\0"),
                value: string_obj(termrequest),
            },
            KeyValuePair {
                key: cstr_slice_as_string(b"cursor\0"),
                value: Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(cursor),
                },
            },
            KeyValuePair {
                key: cstr_slice_as_string(b"terminator\0"),
                value: term_obj,
            },
        ];
        let mut data_obj = Object {
            obj_type: ObjectType::ObjectTypeDict,
            data: ObjectData::Dict(Dict {
                items: data_items.as_mut_ptr(),
                size: 3,
                capacity: 3,
            }),
        };

        term.refcount += 1;
        apply_autocmds_group(
            EVENT_TERMREQUEST,
            ptr::null_mut(),
            ptr::null_mut(),
            true,
            AUGROUP_ALL,
            buf,
            Some(crate::globals::curwin_mut()),
            None,
            &mut data_obj,
            false,
        );
        term.refcount -= 1;
        let pending_send = req.pending_send.clone();
        drop(req);

        let term_pending_send = term.pending.send.take();
        {
            let pending_buf = pending_send.borrow();
            if !pending_buf.is_empty() {
                terminal_send(term, pending_buf.as_ptr(), pending_buf.len());
            }
        }
        if let Some(ref cur_send) = term_pending_send
            && !std::rc::Rc::ptr_eq(cur_send, &pending_send)
        {
            term.pending.send = term_pending_send;
        }

        if term.buf_handle == 0 && term.refcount == 0 {
            term.destroy = true;
            channel_term_close(&mut *term.opts.data);
        }
    }
}

fn schedule_termrequest(term: &mut Terminal) {
    let pending_send = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    term.pending.send = Some(pending_send.clone());

    let line = row_to_linenr(term, term.cursor.row);
    let req = Box::new(TermRequestEvent {
        buf_handle: term.buf_handle,
        sequence: term.termrequest_buffer.clone(),
        pending_send,
        row: line,
        col: term.cursor.col,
        sb_deleted: term.sb_deleted,
        terminator: term.termrequest_terminator,
    });

    multiqueue_put_event(
        unsafe { &mut *loop_queues(&mut g_st().main_loop).events },
        Event::TermRequest(req),
    );
}

fn parse_osc8(str_: impl crate::strings::IntoCCharPtr, attr: &mut i32) -> i32 {
    let str_ = str_.into_c_char_ptr();
    {
        let mut i: usize = 0;
        while crate::memory::cstr_byte_at(str_, i) != 0 {
            if crate::memory::cstr_byte_at(str_, i) == b';' {
                break;
            }
            i += 1;
        }

        if crate::memory::cstr_byte_at(str_, i) != b';' {
            return 0;
        }

        i += 1;

        if crate::memory::cstr_byte_at(str_, i) == 0 {
            *attr = 0;
            return 1;
        }

        *attr = hl_add_url(0, str_.wrapping_add(i));
        1
    }
}

fn on_osc(command: i32, frag: VTermStringFragment, term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    if frag.str_.is_null() || frag.is_empty() {
        return 0;
    }

    if command != 8 && !has_event(EVENT_TERMREQUEST) {
        return 1;
    }

    if frag.initial() {
        term.termrequest_buffer.clear();
        let mut prefix = [0 as c_char; 32];
        let n = vim_snprintf!(prefix.as_mut_ptr(), prefix.len(), c"\x1b]%d;", command,);
        if n > 0 {
            term.termrequest_buffer
                .extend_from_slice(&prefix[..n as usize]);
        }
    }
    if !frag.is_empty() {
        term.termrequest_buffer
            .extend_from_slice(unsafe { std::slice::from_raw_parts(frag.str_, frag.len()) });
    }
    if frag.final_() {
        term.termrequest_terminator = frag.terminator;
        if has_event(EVENT_TERMREQUEST) {
            schedule_termrequest(term);
        }
        if command == 8 {
            term.termrequest_buffer.push(NUL);
            let off = b"\x1b]8;".len();
            let mut attr: i32 = 0;
            if parse_osc8(
                term.termrequest_buffer.as_ptr().wrapping_add(off),
                &mut attr,
            ) != 0
            {
                let state = vterm_obtain_state(&mut term.vt);
                let mut value = VTermValue::Number(attr);
                vterm_state_set_penattr(state, VTERM_ATTR_URI, VTERM_VALUETYPE_INT, &mut value);
            }
        }
    }
    1
}

fn on_dcs(
    command: *const c_char,
    commandlen: usize,
    frag: VTermStringFragment,
    term: *mut Terminal,
) -> i32 {
    let term = unsafe { &mut *term };
    if command.is_null() || frag.str_.is_null() {
        return 0;
    }
    if !has_event(EVENT_TERMREQUEST) {
        return 1;
    }

    if frag.initial() {
        term.termrequest_buffer.clear();
        term.termrequest_buffer
            .extend_from_slice(&[b'\x1b' as c_char, b'P' as c_char]);
        if commandlen > 0 {
            term.termrequest_buffer
                .extend_from_slice(unsafe { std::slice::from_raw_parts(command, commandlen) });
        }
    }
    if !frag.is_empty() {
        term.termrequest_buffer
            .extend_from_slice(unsafe { std::slice::from_raw_parts(frag.str_, frag.len()) });
    }
    if frag.final_() {
        term.termrequest_terminator = frag.terminator;
        schedule_termrequest(term);
    }
    1
}

fn on_apc(frag: VTermStringFragment, term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    if frag.str_.is_null() || frag.is_empty() {
        return 0;
    }

    if !has_event(EVENT_TERMREQUEST) {
        return 1;
    }

    if frag.initial() {
        term.termrequest_buffer.clear();
        term.termrequest_buffer
            .extend_from_slice(&[b'\x1b' as c_char, b'_' as c_char]);
    }
    term.termrequest_buffer
        .extend_from_slice(unsafe { std::slice::from_raw_parts(frag.str_, frag.len()) });
    if frag.final_() {
        term.termrequest_terminator = frag.terminator;
        schedule_termrequest(term);
    }
    1
}

pub fn terminal_init() {
    let timer = refresh_timer_ptr();
    time_watcher_init(
        &mut g_st().main_loop,
        &mut *timer,
        TimeWatcherData::TerminalRefresh,
    );
    timer.events = multiqueue_new_child(loop_queues(&mut g_st().main_loop).events);
}

pub fn terminal_teardown() {
    let timer = refresh_timer_ptr();
    time_watcher_stop(&mut *timer);
    multiqueue_free(timer.events);
    time_watcher_close(&mut *timer);
    term_st().invalidated_terminals = Vec::new();
}

fn term_output_callback(s: &[u8], term: *mut Terminal) {
    unsafe {
        let term = &mut *term;
        terminal_send(term, s.as_ptr().cast::<c_char>(), s.len());
    }
}

fn term_may_alloc_scrollback(term: &mut Terminal, mut buf: *mut FileBuf) -> bool {
    if term.sb_allocated {
        return true;
    }
    if buf.is_null() {
        buf = handle_get_buffer(term.buf_handle);
        if buf.is_null() {
            return false;
        }
    }

    if unsafe { (*buf).b_p_scbk } < 1 {
        unsafe {
            (*buf).b_p_scbk = SB_MAX;
        }
    }
    term.sb_size = unsafe { (*buf).b_p_scbk } as usize;
    term.sb_buffer.reserve(term.sb_size);
    term.sb_allocated = true;
    true
}

pub fn terminal_alloc(buf: &mut FileBuf, mut opts: TerminalOptions) -> &'static mut Terminal {
    if opts.width == 0 {
        opts.width = 80;
    }
    if opts.height == 0 {
        opts.height = 24;
    }

    let term = Box::into_raw(Box::new(Terminal {
        opts,
        vt: vterm_new(i32::from(opts.height), i32::from(opts.width)),
        textbuf: [0; TEXTBUF_SIZE],
        sb_buffer: Vec::new(),
        sb_allocated: false,
        sb_size: 0,
        sb_pending: 0,
        sb_deleted: 0,
        old_sb_deleted: 0,
        old_height: 0,
        title: Vec::new(),
        buf_handle: buf.b_fnum,
        in_altscreen: false,
        suspended: false,
        closed: false,
        destroy: false,
        forward_mouse: false,
        invalid_start: 0,
        invalid_end: 0,
        cursor: TermCursor::new(),
        pending: TermPending::new(),
        streamed_paste: false,
        theme_updates: false,
        synchronized_output: false,
        sync_flush_pending: false,
        color_set: [false; 16],
        selection: Vec::new(),
        termrequest_buffer: Vec::new(),
        termrequest_terminator: 0,
        refcount: 0,
    }));
    buf.terminal = term;
    let term_ref = unsafe { &mut *term };
    vterm_set_utf8(&mut term_ref.vt, 1);

    let state = vterm_obtain_state(&mut term_ref.vt);
    let vts = vterm_obtain_screen(&mut term_ref.vt);
    vterm_screen_enable_altscreen(vts, 1);
    vterm_screen_enable_reflow(vts, true);
    vterm_screen_set_callbacks(vts, Some(&VTERM_SCREEN_CALLBACKS), term);
    vterm_screen_set_unrecognised_fallbacks(vts, Some(&VTERM_FALLBACKS), term);
    vterm_screen_set_damage_merge(vts, VTERM_DAMAGE_SCROLL);
    vterm_screen_reset(vts, 1);
    vterm_output_set_callback(&mut term_ref.vt, Some(term_output_callback), term);

    vterm_state_set_selection_callbacks(
        &mut *state,
        Some(&VTERM_SELECTION_CALLBACKS),
        term,
        SELECTIONBUF_SIZE,
    );

    let shape_num = match (*shape_table_mut())[SHAPE_IDX_TERM].shape {
        CursorShape::ShapeBlock => VTERM_PROP_CURSORSHAPE_BLOCK,
        CursorShape::ShapeHor => VTERM_PROP_CURSORSHAPE_UNDERLINE,
        CursorShape::ShapeVer => VTERM_PROP_CURSORSHAPE_BAR_LEFT,
    };
    let mut cursor_shape = VTermValue::Number(shape_num);
    vterm_state_set_termprop(state, VTERM_PROP_CURSORSHAPE, &mut cursor_shape);

    let mut cursor_blink = VTermValue::Boolean(i32::from(
        (*shape_table_mut())[SHAPE_IDX_TERM].blinkon != 0
            && (*shape_table_mut())[SHAPE_IDX_TERM].blinkoff != 0,
    ));
    vterm_state_set_termprop(state, VTERM_PROP_CURSORBLINK, &mut cursor_blink);

    term_ref.invalid_start = 0;
    term_ref.invalid_end = i32::from(opts.height);

    term_ref.pending.events = multiqueue_new(ptr::null_mut());

    if (buf.b_ml.ml_flags & ML_EMPTY) == 0 {
        let line_count = buf.b_ml.ml_line_count;
        loop {
            if (buf.b_ml.ml_flags & ML_EMPTY) != 0 {
                break;
            }
            ml_delete_buf(buf, 1, false);
        }
        deleted_lines_buf(buf, 1, line_count);
    }
    term_ref.old_height = 1;

    term_ref
}

pub fn terminal_open(termpp: *mut *mut Terminal, buf: &mut FileBuf) {
    unsafe {
        debug_assert!(!(*termpp).is_null());
        let term = &mut **termpp;

        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), &raw mut *buf, 0);

        if term.sb_allocated {
            refresh_scrollback(term, buf);
        } else {
            debug_assert!(term.invalid_start >= 0);
        }
        refresh_screen(term, &mut *buf);
        buf.b_locked += 1;
        set_option_value(
            K_OPT_BUFTYPE,
            string_obj(cstr_slice_as_string(b"terminal\0")),
            OPT_LOCAL,
            true,
            None,
        );
        buf.b_locked -= 1;

        if !buf.b_ffname.is_null() {
            buf_set_term_title(
                &raw mut *buf,
                buf.b_ffname,
                crate::memory::cstr_len(buf.b_ffname),
            );
        }
        curwin_mut().w_onebuf_opt.wo_scb = 0;
        curwin_mut().w_onebuf_opt.wo_crb = 0;
        curwin_mut().w_cursor = CursorPos {
            lnum: 1,
            col: 0,
            coladd: 0,
        };

        apply_autocmds(
            EVENT_TERMOPEN,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            &raw mut *buf,
        );

        ctx_restore(&mut aco);

        if (*termpp).is_null() || term.buf_handle == 0 {
            return;
        }

        if !term_may_alloc_scrollback(term, &raw mut *buf) {
            std::process::abort();
        }

        let state = vterm_obtain_state(&mut term.vt);
        for i in 0..16 {
            let mut var = [0 as c_char; 64];
            vim_snprintf!(var.as_mut_ptr(), var.len(), c"terminal_color_%d", i as i32,);
            let name = get_config_string(buf, var.as_mut_ptr());
            if !name.is_null() {
                let mut dummy: i32 = 0;
                let color_val = name_to_color(name, &mut dummy);
                if color_val != -1 {
                    let color = VTermColor {
                        bytes: [
                            0,
                            ((color_val >> 16) & 0xFF) as u8,
                            ((color_val >> 8) & 0xFF) as u8,
                            (color_val & 0xFF) as u8,
                        ],
                    };
                    vterm_state_set_palette_color(state, i as i32, &color);
                    term.color_set[i] = true;
                }
            }
        }
    }
}

pub fn terminal_close(termpp: *mut *mut Terminal, status: i32) {
    unsafe {
        let term = &mut **termpp;
        if term.destroy {
            return;
        }

        let mut only_destroy = false;
        let buf = handle_get_buffer(term.buf_handle);

        if term.closed {
            only_destroy = true;
        } else {
            if !g_st().exiting {
                let mut height: i32 = 0;
                let mut width: i32 = 0;
                vterm_get_size(&term.vt, &mut height, &mut width);
                if status != -1 && term.cursor.row == height - 1 {
                    fetch_row(term, term.cursor.row, width);
                    if term.textbuf[0] != NUL {
                        terminal_receive(term, c"\r\n", 2);
                    }
                }
                block_autocmds();
                refresh_terminal(term);
                unblock_autocmds();
            }
            term.closed = true;
        }

        let mut pos = if !buf.is_null() {
            (*buf).b_ml.ml_line_count - 1
        } else {
            0
        };
        if status == -1 || g_st().exiting {
            term.buf_handle = 0;
            if !buf.is_null() {
                (*buf).terminal = ptr::null_mut();
            }
            if term.refcount == 0 {
                term.destroy = true;
                channel_term_close(&mut *term.opts.data);
            }
        } else if !only_destroy {
            for wp in for_all_windows_mut() {
                if wp.w_buffer == buf {
                    wp.w_redr_status = true;
                }
            }
            pos = row_to_linenr(term, term.cursor.row).min(pos);
        }

        if only_destroy {
            return;
        }

        if status >= 0 {
            g_st().did_check_timestamps = false;
            g_st().need_check_timestamps = true;
        }

        if !buf.is_null() && !is_autocmd_blocked() {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let dict = get_v_event(&mut save_v_event);
            tv_dict_add_nr(&mut *dict, c"status", i64::from(status));
            tv_dict_set_keys_readonly(&mut *dict);

            let mut data_items = [KeyValuePair {
                key: cstr_slice_as_string(b"pos\0"),
                value: integer_obj(i64::from(pos)),
            }];
            let mut data_obj = Object {
                obj_type: ObjectType::ObjectTypeDict,
                data: ObjectData::Dict(Dict {
                    items: data_items.as_mut_ptr(),
                    size: 1,
                    capacity: 1,
                }),
            };

            apply_autocmds_group(
                EVENT_TERMCLOSE,
                ptr::null_mut(),
                ptr::null_mut(),
                status >= 0,
                AUGROUP_ALL,
                buf,
                Some(crate::globals::curwin_mut()),
                None,
                &mut data_obj,
                false,
            );

            restore_v_event(&mut *dict, &mut save_v_event);
        }
    }
}

pub fn terminal_state_change_event(buf_handle: ObjHandle) {
    unsafe {
        let buf = handle_get_buffer(buf_handle);
        if !buf.is_null() && !(*buf).terminal.is_null() {
            redraw_buf_line_later(&*(buf), (*buf).b_ml.ml_line_count, false);
        }
    }
}

pub fn terminal_set_state(term: &mut Terminal, suspended: bool) {
    if term.suspended != suspended {
        multiqueue_put_event(
            unsafe { &mut *refresh_timer_ptr().events },
            Event::TermStateChange(term.buf_handle),
        );
    }
    term.suspended = suspended;
}

pub fn terminal_check_size(term: &mut Terminal) {
    if term.closed {
        return;
    }

    let mut curwidth: i32 = 0;
    let mut curheight: i32 = 0;
    vterm_get_size(&term.vt, &mut curheight, &mut curwidth);
    let mut width: u16 = 0;
    let mut height: u16 = 0;

    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&*tp) {
            if !is_ctx_win(wp) && !wp.w_buffer.is_null() && (*wp).buf_mut().terminal == term {
                let win_width = (wp.w_view_width - win_col_off(wp)).max(0) as u16;
                width = width.max(win_width);
                height = height.max(wp.w_view_height as u16);
            }
        }
    }

    if (curheight == i32::from(height) && curwidth == i32::from(width)) || height == 0 || width == 0
    {
        return;
    }

    vterm_set_size(&mut term.vt, i32::from(height), i32::from(width));
    vterm_screen_flush_damage(unsafe { &mut *term.vt.screen_mut_ptr() });
    term.pending.resize = true;
    invalidate_terminal(term, -1, -1);
}

fn set_terminal_winopts(s: &mut TerminalState) {
    debug_assert!(s.save_curwin_handle == 0);

    s.save_curwin_handle = curwin_mut().handle;
    s.save_w_p_cul = curwin_mut().w_onebuf_opt.wo_cul != 0;
    s.save_w_p_culopt = None;
    s.save_w_p_culopt_flags = curwin_mut().w_p_culopt_flags;
    s.save_w_p_cuc = curwin_mut().w_onebuf_opt.wo_cuc;
    s.save_w_p_so = curwin_mut().w_onebuf_opt.wo_so;
    s.save_w_p_siso = curwin_mut().w_onebuf_opt.wo_siso;

    if curwin_mut().w_onebuf_opt.wo_cul != 0
        && (curwin_mut().w_p_culopt_flags & K_OPT_CULOPT_FLAG_NUMBER) != 0
    {
        if !(crate::memory::cstr_as_opt_bytes(curwin_mut().w_onebuf_opt.wo_culopt)
            == Some(b"number"))
        {
            s.save_w_p_culopt = Some(crate::memory::cstr_to_cstring(
                curwin_mut().w_onebuf_opt.wo_culopt,
            ));
            free_string_option(curwin_mut().w_onebuf_opt.wo_culopt);
            curwin_mut().w_onebuf_opt.wo_culopt = alloc_string_option(c"number");
        }
        curwin_mut().w_p_culopt_flags = K_OPT_CULOPT_FLAG_NUMBER;
    } else {
        curwin_mut().w_onebuf_opt.wo_cul = 0;
    }
    curwin_mut().w_onebuf_opt.wo_cuc = 0;
    curwin_mut().w_onebuf_opt.wo_so = 0;
    curwin_mut().w_onebuf_opt.wo_siso = 0;

    if curwin_mut().w_onebuf_opt.wo_cuc != s.save_w_p_cuc {
        redraw_later(Some(crate::globals::curwin_mut()), UPD_SOME_VALID);
    } else if (curwin_mut().w_onebuf_opt.wo_cul != 0) != s.save_w_p_cul
        || (curwin_mut().w_onebuf_opt.wo_cul != 0
            && curwin_mut().w_p_culopt_flags != s.save_w_p_culopt_flags)
    {
        redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
    }
}

fn unset_terminal_winopts(s: &mut TerminalState) {
    unsafe {
        debug_assert!(s.save_curwin_handle != 0);

        let wp = handle_get_window(s.save_curwin_handle);
        if !wp.is_null() {
            let mut winopts: *mut Winopt = ptr::null_mut();
            if (*wp).buf_mut().b_fnum != s.term_mut().buf_handle {
                let buf = handle_get_buffer(s.term_mut().buf_handle);
                if !buf.is_null() {
                    for wip in &mut (*buf).b_wininfo {
                        if wip.wi_win == wp && wip.wi_optset {
                            winopts = &raw mut wip.wi_opt;
                            break;
                        }
                    }
                }
            } else {
                winopts = &raw mut (*wp).w_onebuf_opt;
                if win_valid(wp) {
                    if s.save_w_p_cuc != (*wp).w_onebuf_opt.wo_cuc {
                        redraw_later(wp.as_mut(), UPD_SOME_VALID);
                    } else if s.save_w_p_cul != ((*wp).w_onebuf_opt.wo_cul != 0)
                        || (s.save_w_p_cul && s.save_w_p_culopt_flags != (*wp).w_p_culopt_flags)
                    {
                        redraw_later(wp.as_mut(), UPD_VALID);
                    }
                }
                (*wp).w_p_culopt_flags = s.save_w_p_culopt_flags;
            }

            if !winopts.is_null() {
                let winopts = { &mut *winopts };
                let winopts = { &mut *winopts };
                if let Some(saved) = s.save_w_p_culopt.take() {
                    free_string_option(winopts.wo_culopt);
                    winopts.wo_culopt = alloc_string_option(saved.as_ptr());
                }
                winopts.wo_cul = i32::from(s.save_w_p_cul);
                winopts.wo_cuc = s.save_w_p_cuc;
                winopts.wo_so = s.save_w_p_so;
                winopts.wo_siso = s.save_w_p_siso;
            }
        }

        s.save_w_p_culopt = None;
        s.save_curwin_handle = 0;
    }
}

pub fn terminal_enter() -> bool {
    unsafe {
        let buf = g_st().curbuf;
        debug_assert!(!(*buf).terminal.is_null());
        let mut s = TerminalState::new();
        s.term = (*buf).terminal;
        s.cursor_visible = true;
        g_st().ins_global.stop_insert_mode = false;

        terminal_check_size(s.term_mut());

        atom_term_enter();
        let save_state = g_st().state_global;
        s.save_rd = g_st().redrawing_disabled;
        g_st().state_global = MODE_TERMINAL;
        g_st().mapped_ctrl_c |= MODE_TERMINAL;
        g_st().redrawing_disabled = 0;

        set_terminal_winopts(&mut s);

        s.term_mut().pending.cursor = true;
        adjust_topline_cursor(s.term_mut(), &mut *buf, 0);
        showmode();
        ui_cursor_shape();

        terminal_focus(s.term_mut(), true);
        curbuf_mut().b_last_changedtick_i = buf_get_changedtick(curbuf_mut());

        s.term_mut().refcount += 1;
        apply_autocmds(
            EVENT_TERMENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        may_trigger_modechanged();
        s.term_mut().refcount -= 1;
        if s.term_mut().buf_handle == 0 {
            s.close = true;
        }

        state_enter(&mut s);

        if !s.got_bsl_o {
            g_st().restart_edit = 0;
        }
        g_st().state_global = save_state;
        g_st().redrawing_disabled = s.save_rd;
        if !s.cursor_visible {
            ui_busy_stop();
        }

        let _ = parse_shape_opt(SHAPE_CURSOR);

        unset_terminal_winopts(&mut s);

        terminal_focus(s.term_mut(), false);
        curbuf_mut().b_last_changedtick = buf_get_changedtick(curbuf_mut());

        if curbuf_mut().terminal == s.term && !s.close {
            terminal_check_cursor();
        }
        if g_st().restart_edit != 0 {
            showmode();
        } else {
            unshowmode(true);
        }
        ui_cursor_shape();

        if s.close {
            s.term_mut().refcount += 1;
        }
        apply_autocmds(
            EVENT_TERMLEAVE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        if s.close {
            s.term_mut().refcount -= 1;
            let buf_handle = s.term_mut().buf_handle;
            s.term_mut().destroy = true;
            channel_term_close(&mut *s.term_mut().opts.data);
            if buf_handle != 0 {
                do_buffer(DOBUF_WIPE, DOBUF_FIRST, FORWARD, buf_handle, 1);
            }
        }

        s.got_bsl_o
    }
}

fn terminal_check_cursor() {
    let term = curbuf_mut().terminal_mut();
    curwin_mut().w_cursor.lnum = curbuf_mut()
        .b_ml
        .ml_line_count
        .min(row_to_linenr(term, term.cursor.row));
    let topline = (curbuf_mut().b_ml.ml_line_count - curwin_mut().w_view_height + 1).max(1);
    if topline != curwin_mut().w_topline {
        set_topline(curwin_mut(), topline);
    }
    if term.suspended && (g_st().state_global & MODE_TERMINAL) != 0 {
        curwin_mut().w_cursor = CursorPos {
            lnum: curbuf_mut().b_ml.ml_line_count,
            col: 0,
            coladd: 0,
        };
    } else {
        let off = if (g_st().state_global & MODE_TERMINAL) != 0 {
            0
        } else if curwin_mut().w_onebuf_opt.wo_rl != 0 {
            1
        } else {
            -1
        };
        coladvance(curwin_mut(), (term.cursor.col + off).max(0));
    }
}

fn terminal_check_focus(s: &mut TerminalState) -> bool {
    if curbuf_mut().terminal.is_null() {
        return false;
    }

    if s.save_curwin_handle != curwin_mut().handle {
        unset_terminal_winopts(s);
        set_terminal_winopts(s);
    }
    let cur_term = curbuf_mut().terminal_mut();
    if !std::ptr::eq(s.term, cur_term) {
        terminal_focus(s.term_mut(), false);
        if s.close {
            s.term_mut().destroy = true;
            channel_term_close(unsafe { &mut *s.term_mut().opts.data });
            s.close = false;
        }

        s.term = &raw mut *cur_term;
        s.term_mut().pending.cursor = true;
        invalidate_terminal(s.term_mut(), -1, -1);
        terminal_focus(s.term_mut(), true);
    }
    true
}

fn terminal_check(s: &mut TerminalState) -> i32 {
    debug_assert!(!s.close || (s.term_mut().buf_handle == 0 && s.term != curbuf_mut().terminal));

    if g_st().ins_global.stop_insert_mode || !terminal_check_focus(&mut *s) {
        return 0;
    }

    terminal_check_refresh();

    terminal_check_cursor();
    validate_cursor(curwin_mut());

    s.term_mut().refcount += 1;
    if has_event(EVENT_TEXTCHANGEDT)
        && curbuf_mut().b_last_changedtick_i != buf_get_changedtick(curbuf_mut())
    {
        apply_autocmds(
            EVENT_TEXTCHANGEDT,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        curbuf_mut().b_last_changedtick_i = buf_get_changedtick(curbuf_mut());
    }
    may_trigger_win_scrolled_resized();
    s.term_mut().refcount -= 1;
    if s.term_mut().buf_handle == 0 {
        s.close = true;
    }

    if !terminal_check_focus(&mut *s) {
        return 0;
    }
    terminal_check_cursor();
    validate_cursor(curwin_mut());

    show_cursor_info_later(false);
    if g_st().must_redraw != 0 {
        update_screen();
    } else {
        redraw_statuslines();
        if g_st().clear_cmdline || g_st().redraw_cmdline || g_st().redraw_mode {
            showmode();
        }
    }

    setcursor();
    refresh_cursor(s.term_mut(), &mut s.cursor_visible);
    ui_flush();
    1
}

fn terminal_execute(s: &mut TerminalState, key: i32) -> i32 {
    let mut tmp_mod_mask = g_st().mod_mask;
    let mod_key = merge_modifiers(key, &mut tmp_mod_mask);

    match mod_key {
        K_LEFTMOUSE | K_LEFTDRAG | K_LEFTRELEASE | K_MIDDLEMOUSE | K_MIDDLEDRAG
        | K_MIDDLERELEASE | K_RIGHTMOUSE | K_RIGHTDRAG | K_RIGHTRELEASE | K_X1MOUSE | K_X1DRAG
        | K_X1RELEASE | K_X2MOUSE | K_X2DRAG | K_X2RELEASE | K_MOUSEDOWN | K_MOUSEUP
        | K_MOUSELEFT | K_MOUSERIGHT | K_MOUSEMOVE => {
            if send_mouse_event(s.term_mut(), key) {
                return 0;
            }
        }
        K_PASTE_START => {
            paste_repeat(1);
        }
        K_EVENT => {
            s.term_mut().refcount += 1;
            state_handle_k_event();
            s.term_mut().refcount -= 1;
            if s.term_mut().buf_handle == 0 {
                s.close = true;
            }
        }
        K_COMMAND => {
            do_cmdline(ptr::null_mut(), crate::cmdexpand::LineGetter::CmdKeyCmd, 0);
        }
        K_LUA => {
            map_execute_lua(false, false);
        }
        K_IGNORE | K_NOP => {}
        _ => {
            if mod_key == CTRL_N && s.got_bsl {
                return 0;
            }
            if mod_key == CTRL_O && s.got_bsl {
                s.got_bsl_o = true;
                g_st().restart_edit = b'I' as i32;
                return 0;
            }
            if mod_key == CTRL_C_CONST {
                g_st().got_int = false;
            }
            if mod_key == CTRL_BSL && !s.got_bsl {
                s.got_bsl = true;
                return 1;
            }
            if s.term_mut().suspended {
                channel_term_resume(unsafe { &mut *s.term_mut().opts.data });
                terminal_set_state(s.term_mut(), false);
                return 1;
            }
            if s.term_mut().closed {
                s.close = true;
                return 0;
            }

            s.got_bsl = false;
            terminal_send_key(s.term_mut(), key);
        }
    }

    1
}

pub fn terminal_destroy(termpp: *mut *mut Terminal) {
    let term = unsafe { &mut **termpp };
    let buf = handle_get_buffer(term.buf_handle);
    if !buf.is_null() {
        term.buf_handle = 0;
        unsafe {
            (*buf).terminal = ptr::null_mut();
        }
    }

    if term.refcount == 0 {
        if let Some(pos) = term_st()
            .invalidated_terminals
            .iter()
            .position(|&t| std::ptr::eq(t, term))
        {
            block_autocmds();
            refresh_terminal(term);
            unblock_autocmds();
            term_st().invalidated_terminals.swap_remove(pos);
        }
        drop(unsafe { Box::from_raw(&raw mut *term) });
        unsafe {
            *termpp = ptr::null_mut();
        }
    }
}

fn terminal_send(term: &mut Terminal, data: impl crate::strings::IntoCCharPtr, size: usize) {
    let data = data.into_c_char_ptr();
    {
        if term.closed {
            return;
        }
        if let Some(ref send_buf) = term.pending.send {
            if size > 0 {
                send_buf
                    .borrow_mut()
                    .extend_from_slice(unsafe { std::slice::from_raw_parts(data, size) });
            }
            return;
        }
        channel_term_write(data, size, unsafe { &mut *term.opts.data });
    }
}

fn is_filter_char(c: i32) -> bool {
    let flag = match c {
        0x08 => K_OPT_TPF_FLAG_BS,
        0x09 => K_OPT_TPF_FLAG_HT,
        0x0A | 0x0D => 0,
        0x0C => K_OPT_TPF_FLAG_FF,
        0x1b => K_OPT_TPF_FLAG_ESC,
        0x7F => K_OPT_TPF_FLAG_DEL,
        _ => {
            if c < b' ' as i32 {
                K_OPT_TPF_FLAG_C0
            } else if (0x80..=0x9F).contains(&c) {
                K_OPT_TPF_FLAG_C1
            } else {
                0
            }
        }
    };
    (opt_vars().tpf_flags & flag) != 0
}

pub fn terminal_set_streamed_paste(term: &mut Terminal, streamed: bool) {
    if term.streamed_paste != streamed {
        let cur_term = curbuf_mut().terminal_mut();
        if streamed {
            vterm_keyboard_start_paste(&mut cur_term.vt);
        } else {
            vterm_keyboard_end_paste(&mut cur_term.vt);
        }
    }
    term.streamed_paste = streamed;
}

pub fn terminal_paste(count: i32, y_array: *mut NvimString, y_size: usize) {
    if y_size == 0 {
        return;
    }
    let cur_term = curbuf_mut().terminal_mut();
    if !cur_term.streamed_paste {
        vterm_keyboard_start_paste(&mut cur_term.vt);
    }
    let mut buff: Vec<c_char> = Vec::with_capacity(unsafe { (*y_array).size });
    for _ in 0..count {
        for j in 0..y_size {
            if j > 0 {
                let term_now = curbuf_mut().terminal_mut();
                #[cfg(windows)]
                terminal_send(term_now, c"\r\n", 2);
                #[cfg(not(windows))]
                terminal_send(term_now, c"\n", 1);
            }
            let item = unsafe { *y_array.wrapping_add(j) };
            buff.clear();
            buff.reserve(item.size);
            let mut src = item.data;
            while !crate::memory::cstr_is_empty(src) {
                let len = utf_ptr2len(src) as usize;
                let c = utf_ptr2char(src);
                if !is_filter_char(c) {
                    buff.extend_from_slice(unsafe { std::slice::from_raw_parts(src, len) });
                }
                src = src.wrapping_add(len);
            }
            let term_now = curbuf_mut().terminal_mut();
            terminal_send(term_now, buff.as_ptr(), buff.len());
        }
    }
    let term_end = curbuf_mut().terminal_mut();
    if !term_end.streamed_paste {
        vterm_keyboard_end_paste(&mut term_end.vt);
    }
}

fn terminal_send_key(term: &mut Terminal, mut c: i32) {
    let mut mod_: VTermModifier = VTERM_MOD_NONE;
    if c == K_ZERO {
        c = CTRL_AT;
    }

    let key = convert_key(&mut c, g_st().mod_mask, &mut mod_);
    if key != VTERM_KEY_NONE {
        vterm_keyboard_key(&mut term.vt, key, mod_);
    } else if !is_special(c) {
        vterm_keyboard_unichar(&mut term.vt, c as u32, mod_);
    }
}

pub fn on_sync_flush(buf_handle: ObjHandle) {
    unsafe {
        if g_st().exiting {
            return;
        }
        let buf = handle_get_buffer(buf_handle);
        if buf.is_null() || (*buf).terminal.is_null() {
            return;
        }
        block_autocmds();
        refresh_terminal(&mut *(*buf).terminal);
        unblock_autocmds();
    }
}

pub fn terminal_receive(term: &mut Terminal, data: impl crate::strings::IntoCCharPtr, len: usize) {
    let data = data.into_c_char_ptr();
    {
        if data.is_null() {
            return;
        }

        if term.opts.force_crlf {
            let mut crlf_data: Vec<c_char> = Vec::with_capacity(len);
            for i in 0..len {
                let b = unsafe { *data.wrapping_add(i) };
                if b == b'\n' as c_char
                    && (i == 0 || crate::memory::cstr_byte_at(data, i - 1) != b'\r')
                {
                    crlf_data.push(b'\r' as c_char);
                }
                crlf_data.push(b);
            }
            vterm_input_write(&mut term.vt, crlf_data.as_ptr(), crlf_data.len());
        } else {
            vterm_input_write(&mut term.vt, data, len);
        }
        vterm_screen_flush_damage(unsafe { &mut *term.vt.screen_mut_ptr() });

        if term.sync_flush_pending {
            term.sync_flush_pending = false;
            let mut height: i32 = 0;
            let mut width: i32 = 0;
            vterm_get_size(&term.vt, &mut height, &mut width);
            term.invalid_start = 0;
            term.invalid_end = height;
            multiqueue_put_event(
                unsafe { &mut *loop_queues(&mut g_st().main_loop).events },
                Event::TermSyncFlush(term.buf_handle),
            );
        }
    }
}

fn get_rgb(state: &VTermStateFull, mut color: VTermColor) -> i32 {
    vterm_state_convert_color_to_rgb(state, &mut color);
    let r = i32::from(color.bytes[1]);
    let g = i32::from(color.bytes[2]);
    let b = i32::from(color.bytes[3]);
    (r << 16) | (g << 8) | b
}

const fn get_underline_hl_flag(attrs: VTermScreenCellAttrs) -> i32 {
    match attrs.underline() {
        VTERM_UNDERLINE_OFF => 0,
        VTERM_UNDERLINE_SINGLE => HL_UNDERLINE,
        VTERM_UNDERLINE_DOUBLE => HL_UNDERDOUBLE,
        VTERM_UNDERLINE_CURLY => HL_UNDERCURL,
        _ => HL_UNDERLINE,
    }
}

pub fn terminal_get_line_attributes(term: &mut Terminal, linenr: i32, term_attrs: *mut i32) {
    let mut height: i32 = 0;
    let mut width: i32 = 0;
    vterm_get_size(&term.vt, &mut height, &mut width);
    let state = vterm_obtain_state(&mut term.vt);
    debug_assert!(linenr != 0);
    let row = linenr_to_row(term, linenr);
    if row >= height {
        return;
    }

    width = width.min(TERM_ATTRS_MAX);
    for col in 0..width {
        let mut cell = VTermScreenCell::default();
        let color_valid = fetch_cell(term, row, col, &mut cell);
        let fg_type = cell.fg.bytes[0];
        let bg_type = cell.bg.bytes[0];
        let fg_default = !color_valid || (fg_type & VTERM_COLOR_DEFAULT_FG) != 0;
        let bg_default = !color_valid || (bg_type & VTERM_COLOR_DEFAULT_BG) != 0;

        let vt_fg = if fg_default {
            -1
        } else {
            get_rgb(state, cell.fg)
        };
        let vt_bg = if bg_default {
            -1
        } else {
            get_rgb(state, cell.bg)
        };

        let fg_indexed = (fg_type & VTERM_COLOR_INDEXED) != 0;
        let bg_indexed = (bg_type & VTERM_COLOR_INDEXED) != 0;

        let vt_fg_idx: i16 = if !fg_default && fg_indexed {
            i16::from(cell.fg.bytes[1]) + 1
        } else {
            0
        };
        let vt_bg_idx: i16 = if !bg_default && bg_indexed {
            i16::from(cell.bg.bytes[1]) + 1
        } else {
            0
        };

        let fg_set = vt_fg_idx != 0 && vt_fg_idx <= 16 && term.color_set[(vt_fg_idx - 1) as usize];
        let bg_set = vt_bg_idx != 0 && vt_bg_idx <= 16 && term.color_set[(vt_bg_idx - 1) as usize];

        let hl_attrs = (if cell.attrs.bold() != 0 { HL_BOLD } else { 0 })
            | (if cell.attrs.dim() != 0 { HL_DIM } else { 0 })
            | (if cell.attrs.blink() != 0 { HL_BLINK } else { 0 })
            | (if cell.attrs.conceal() != 0 {
                HL_CONCEALED
            } else {
                0
            })
            | (if cell.attrs.overline() != 0 {
                HL_OVERLINE
            } else {
                0
            })
            | (if cell.attrs.italic() != 0 {
                HL_ITALIC
            } else {
                0
            })
            | (if cell.attrs.reverse() != 0 {
                HL_INVERSE
            } else {
                0
            })
            | get_underline_hl_flag(cell.attrs)
            | (if cell.attrs.strike() != 0 {
                HL_STRIKETHROUGH
            } else {
                0
            })
            | (if fg_indexed && !fg_set {
                HL_FG_INDEXED
            } else {
                0
            })
            | (if bg_indexed && !bg_set {
                HL_BG_INDEXED
            } else {
                0
            });

        let mut attr_id: i32 = 0;
        if hl_attrs != 0 || !fg_default || !bg_default {
            let mut attrs = HlAttrs {
                cterm_ae_attr: hl_attrs,
                cterm_fg_color: vt_fg_idx,
                cterm_bg_color: vt_bg_idx,
                rgb_ae_attr: hl_attrs,
                rgb_fg_color: vt_fg,
                rgb_bg_color: vt_bg,
                rgb_sp_color: -1,
                hl_blend: -1,
                font: 0,
                url: -1,
            };
            attr_id = hl_get_term_attr(&mut attrs);
        }

        if cell.uri > 0 {
            attr_id = hl_combine_attr(attr_id, cell.uri);
        }

        unsafe {
            *term_attrs.wrapping_add(col as usize) = attr_id;
        }
    }
}

pub fn terminal_buf(term: &Terminal) -> Buffer {
    term.buf_handle
}

pub fn terminal_running(term: &Terminal) -> bool {
    !term.closed
}

pub fn terminal_suspended(term: &Terminal) -> bool {
    term.suspended
}

pub fn terminal_notify_theme(term: &mut Terminal, dark: bool) {
    if !term.theme_updates {
        return;
    }
    let seq: &[u8; 9] = if dark {
        b"\x1b[?997;1n"
    } else {
        b"\x1b[?997;2n"
    };
    terminal_send(term, seq.as_ptr().cast::<c_char>(), seq.len());
}

fn terminal_focus(term: &mut Terminal, focus: bool) {
    let state = vterm_obtain_state(&mut term.vt);
    if focus {
        vterm_state_focus_in(state);
    } else {
        vterm_state_focus_out(state);
    }
}

fn term_damage(rect: VTermRect, term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    invalidate_terminal(term, rect.start_row, rect.end_row);
    1
}

fn term_moverect(dest: VTermRect, src: VTermRect, term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    invalidate_terminal(
        term,
        dest.start_row.min(src.start_row),
        dest.end_row.max(src.end_row),
    );
    1
}

fn term_movecursor(
    new_pos: VTermPos,
    _old_pos: VTermPos,
    _visible: i32,
    term: *mut Terminal,
) -> i32 {
    let term = unsafe { &mut *term };
    term.cursor.row = new_pos.row;
    term.cursor.col = new_pos.col;
    invalidate_terminal(term, -1, -1);
    1
}

fn buf_set_term_title(buf: *mut FileBuf, title: impl crate::strings::IntoCCharPtr, len: usize) {
    let title = title.into_c_char_ptr();
    unsafe {
        if buf.is_null() {
            return;
        }

        let mut err = Error::new();
        (*buf).b_locked += 1;
        dict_set_var(
            &mut *(*buf).b_vars(),
            cstr_slice_as_string(b"term_title\0"),
            string_obj(NvimString {
                data: title.cast_mut(),
                size: len,
            }),
            false,
            false,
            None,
            &mut err,
        );
        (*buf).b_locked -= 1;
        api_clear_error(&mut err);
        status_redraw_buf(buf);
    }
}

fn term_settermprop(prop: VTermProp, val: &mut VTermValue, term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    match prop {
        VTERM_PROP_ALTSCREEN => {
            term.in_altscreen = val.boolean() != 0;
        }
        VTERM_PROP_CURSORVISIBLE => {
            term.cursor.visible = val.boolean() != 0;
            invalidate_terminal(term, -1, -1);
        }
        VTERM_PROP_TITLE => {
            let buf = handle_get_buffer(term.buf_handle);
            let frag = val.string();
            let flen = frag.len();

            if frag.initial() && frag.final_() {
                buf_set_term_title(buf, frag.str_, flen);
                return 1;
            }

            if frag.initial() {
                term.title.clear();
                term.title.reserve(flen.max(1024));
            }

            term.title
                .extend_from_slice(unsafe { std::slice::from_raw_parts(frag.str_, flen) });

            if frag.final_() {
                buf_set_term_title(buf, term.title.as_ptr(), term.title.len());
                term.title = Vec::new();
            }
        }
        VTERM_PROP_MOUSE => {
            term.forward_mouse = val.number() != 0;
        }
        VTERM_PROP_CURSORBLINK => {
            term.cursor.blink = val.boolean() != 0;
            term.pending.cursor = true;
            invalidate_terminal(term, -1, -1);
        }
        VTERM_PROP_CURSORSHAPE => {
            term.cursor.shape = val.number();
            term.pending.cursor = true;
            invalidate_terminal(term, -1, -1);
        }
        VTERM_PROP_THEMEUPDATES => {
            term.theme_updates = val.boolean() != 0;
        }
        VTERM_PROP_SYNCOUTPUT => {
            term.synchronized_output = val.boolean() != 0;
            if val.boolean() == 0 {
                term.sync_flush_pending = true;
            }
        }
        _ => return 0,
    }

    1
}

fn term_bell(_data: *mut Terminal) -> i32 {
    vim_beep(K_OPT_BO_FLAG_TERM);
    1
}

fn term_theme(dark: &mut bool, _data: *mut Terminal) -> i32 {
    {
        *dark = crate::memory::cstr_first_byte(opt_vars().p_bg) == b'd';
    }
    1
}

fn term_sb_push(cells: &[VTermScreenCell], term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    if !term_may_alloc_scrollback(term, ptr::null_mut()) {
        return 0;
    }
    debug_assert!(term.sb_size > 0);

    let c = cells.len();
    let mut sbrow = if term.sb_buffer.len() == term.sb_size {
        term.sb_deleted += 1;
        term.sb_buffer.pop().unwrap_or_default()
    } else {
        Vec::with_capacity(c)
    };

    sbrow.clear();
    sbrow.extend_from_slice(cells);
    term.sb_buffer.insert(0, sbrow);

    if term.sb_pending < term.sb_size as i32 {
        term.sb_pending += 1;
    }

    if !term.synchronized_output {
        invalidate_terminal_add(term);
    }

    1
}

fn term_sb_pop(cells: &mut [VTermScreenCell], term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    if term.sb_buffer.is_empty() {
        return 0;
    }

    if term.sb_pending > 0 {
        term.sb_pending -= 1;
    } else {
        term.old_height += 1;
    }

    let sbrow = term.sb_buffer.remove(0);

    let cols_usize = cells.len();
    let cols_to_copy = cols_usize.min(sbrow.len());

    cells[..cols_to_copy].copy_from_slice(&sbrow[..cols_to_copy]);
    for cell in &mut cells[cols_to_copy..cols_usize] {
        cell.schar = 0;
        cell.width = 1;
    }

    if !term.synchronized_output {
        invalidate_terminal_add(term);
    }

    1
}

fn term_sb_clear(term: *mut Terminal) -> i32 {
    let term = unsafe { &mut *term };
    if term.in_altscreen || term.sb_size == 0 || term.sb_buffer.is_empty() {
        return 1;
    }

    let deleted_len = term.sb_buffer.len();
    term.sb_buffer.clear();

    term.sb_deleted += deleted_len;
    term.sb_pending = 0;
    invalidate_terminal(term, -1, -1);

    1
}

pub fn term_clipboard_set(mask: VTermSelectionMask, data: std::ffi::CString) {
    let regname: c_char = match mask as u16 {
        VTERM_SELECTION_CLIPBOARD => b'+' as c_char,
        VTERM_SELECTION_PRIMARY => b'*' as c_char,
        _ => b'+' as c_char,
    };

    let lines = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    let mut start = data.as_ptr();
    loop {
        let start_bytes = crate::memory::cstr_as_bytes(start);
        if let Some(pos) = start_bytes.iter().position(|&b| b == b'\n') {
            tv_list_append_string(&mut *lines, start, pos as isize);
            start = start.wrapping_add(pos + 1);
        } else {
            break;
        }
    }
    tv_list_append_string(&mut *lines, start, -1);
    drop(data);

    let args = tv_list_alloc(3);
    tv_list_append_list(&mut *args, lines);

    let regtype: c_char = b'v' as c_char;
    tv_list_append_string(&mut *args, &regtype, 1);
    tv_list_append_string(&mut *args, &regname, 1);
    eval_call_provider(c"clipboard", c"set", &mut *args, true);
}

fn term_selection_set(
    mask: VTermSelectionMask,
    frag: VTermStringFragment,
    term: *mut Terminal,
) -> i32 {
    let term = unsafe { &mut *term };
    if frag.initial() {
        term.selection.clear();
    }

    if !frag.is_empty() {
        term.selection
            .extend_from_slice(unsafe { std::slice::from_raw_parts(frag.str_, frag.len()) });
    }

    if frag.final_() {
        let bytes = crate::memory::cstr_slice(term.selection.as_ptr(), term.selection.len());
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let data = std::ffi::CString::new(&bytes[..end]).unwrap();
        multiqueue_put_event(
            unsafe { &mut *loop_queues(&mut g_st().main_loop).events },
            Event::TermClipboardSet { mask, data },
        );
    }

    1
}

fn convert_modifiers(key: &mut i32, modifiers: i32, statep: &mut VTermModifier) {
    if (modifiers & i32::from(MOD_MASK_SHIFT)) != 0 {
        *statep |= VTERM_MOD_SHIFT;
    }
    if (modifiers & i32::from(MOD_MASK_CTRL)) != 0 {
        *statep |= VTERM_MOD_CTRL;
        if (modifiers & i32::from(MOD_MASK_SHIFT)) == 0
            && *key >= b'A' as i32
            && *key <= b'Z' as i32
        {
            *key += (b'a' - b'A') as i32;
        }
    }
    if (modifiers & i32::from(MOD_MASK_ALT)) != 0 {
        *statep |= VTERM_MOD_ALT;
    }
}

fn convert_key(key: &mut i32, mut modifiers: i32, statep: &mut VTermModifier) -> VTermKey {
    *key = unsimplify_key(*key, &mut modifiers);
    convert_modifiers(key, modifiers, statep);

    match *key {
        K_BS => VTERM_KEY_BACKSPACE,
        K_TAB | TAB => VTERM_KEY_TAB,
        CTRL_M => VTERM_KEY_ENTER,
        ESC => VTERM_KEY_ESCAPE,
        K_UP => VTERM_KEY_UP,
        K_DOWN => VTERM_KEY_DOWN,
        K_LEFT => VTERM_KEY_LEFT,
        K_RIGHT => VTERM_KEY_RIGHT,
        K_INS => VTERM_KEY_INS,
        K_DEL => VTERM_KEY_DEL,
        K_HOME => VTERM_KEY_HOME,
        K_END => VTERM_KEY_END,
        K_PAGEUP => VTERM_KEY_PAGEUP,
        K_PAGEDOWN => VTERM_KEY_PAGEDOWN,
        K_K0 | K_KINS => VTERM_KEY_KP_0,
        K_K1 | K_KEND => VTERM_KEY_KP_1,
        K_K2 | K_KDOWN => VTERM_KEY_KP_2,
        K_K3 | K_KPAGEDOWN => VTERM_KEY_KP_3,
        K_K4 | K_KLEFT => VTERM_KEY_KP_4,
        K_K5 | K_KORIGIN => VTERM_KEY_KP_5,
        K_K6 | K_KRIGHT => VTERM_KEY_KP_6,
        K_K7 | K_KHOME => VTERM_KEY_KP_7,
        K_K8 | K_KUP => VTERM_KEY_KP_8,
        K_K9 | K_KPAGEUP => VTERM_KEY_KP_9,
        K_KDEL | K_KPOINT => VTERM_KEY_KP_PERIOD,
        K_KENTER => VTERM_KEY_KP_ENTER,
        K_KPLUS => VTERM_KEY_KP_PLUS,
        K_KMINUS => VTERM_KEY_KP_MINUS,
        K_KMULTIPLY => VTERM_KEY_KP_MULT,
        K_KDIVIDE => VTERM_KEY_KP_DIVIDE,
        K_F1 => vterm_key_function(1),
        K_F2 => vterm_key_function(2),
        K_F3 => vterm_key_function(3),
        K_F4 => vterm_key_function(4),
        K_F5 => vterm_key_function(5),
        K_F6 => vterm_key_function(6),
        K_F7 => vterm_key_function(7),
        K_F8 => vterm_key_function(8),
        K_F9 => vterm_key_function(9),
        K_F10 => vterm_key_function(10),
        K_F11 => vterm_key_function(11),
        K_F12 => vterm_key_function(12),
        K_F13 => vterm_key_function(13),
        K_F14 => vterm_key_function(14),
        K_F15 => vterm_key_function(15),
        K_F16 => vterm_key_function(16),
        K_F17 => vterm_key_function(17),
        K_F18 => vterm_key_function(18),
        K_F19 => vterm_key_function(19),
        K_F20 => vterm_key_function(20),
        K_F21 => vterm_key_function(21),
        K_F22 => vterm_key_function(22),
        K_F23 => vterm_key_function(23),
        K_F24 => vterm_key_function(24),
        K_F25 => vterm_key_function(25),
        K_F26 => vterm_key_function(26),
        K_F27 => vterm_key_function(27),
        K_F28 => vterm_key_function(28),
        K_F29 => vterm_key_function(29),
        K_F30 => vterm_key_function(30),
        K_F31 => vterm_key_function(31),
        K_F32 => vterm_key_function(32),
        K_F33 => vterm_key_function(33),
        K_F34 => vterm_key_function(34),
        K_F35 => vterm_key_function(35),
        K_F36 => vterm_key_function(36),
        K_F37 => vterm_key_function(37),
        K_F38 => vterm_key_function(38),
        K_F39 => vterm_key_function(39),
        K_F40 => vterm_key_function(40),
        K_F41 => vterm_key_function(41),
        K_F42 => vterm_key_function(42),
        K_F43 => vterm_key_function(43),
        K_F44 => vterm_key_function(44),
        K_F45 => vterm_key_function(45),
        K_F46 => vterm_key_function(46),
        K_F47 => vterm_key_function(47),
        K_F48 => vterm_key_function(48),
        K_F49 => vterm_key_function(49),
        K_F50 => vterm_key_function(50),
        K_F51 => vterm_key_function(51),
        K_F52 => vterm_key_function(52),
        K_F53 => vterm_key_function(53),
        K_F54 => vterm_key_function(54),
        K_F55 => vterm_key_function(55),
        K_F56 => vterm_key_function(56),
        K_F57 => vterm_key_function(57),
        K_F58 => vterm_key_function(58),
        K_F59 => vterm_key_function(59),
        K_F60 => vterm_key_function(60),
        K_F61 => vterm_key_function(61),
        K_F62 => vterm_key_function(62),
        K_F63 => vterm_key_function(63),
        _ => VTERM_KEY_NONE,
    }
}

fn mouse_action(
    term: &mut Terminal,
    button: i32,
    row: i32,
    col: i32,
    pressed: bool,
    mod_: VTermModifier,
) {
    vterm_mouse_move(&mut term.vt, row, col, mod_);
    if button != 0 {
        vterm_mouse_button(&mut term.vt, button, pressed, mod_);
    }
}

fn send_mouse_event(term: &mut Terminal, mut c: i32) -> bool {
    unsafe {
        let mut row = g_st().mouse_row;
        let mut col = g_st().mouse_col;
        let mut grid = g_st().mouse_grid;
        let mouse_win = mouse_find_win_inner(&mut grid, &mut row, &mut col);
        if !mouse_win.is_null() {
            let mouse_win = { &mut *mouse_win };
            let offset: i32;
            if !term.suspended
                && !term.closed
                && term.forward_mouse
                && (*mouse_win).buf_mut().terminal == term
                && row >= 0
                && (grid > 1 || row + mouse_win.w_winbar_height < mouse_win.w_height)
                && {
                    offset = win_col_off(mouse_win);
                    col >= offset
                }
                && (grid > 1 || col < mouse_win.w_width)
            {
                let (button, pressed) = match c {
                    K_LEFTDRAG | K_LEFTMOUSE => (1, true),
                    K_LEFTRELEASE => (1, false),
                    K_MIDDLEDRAG | K_MIDDLEMOUSE => (2, true),
                    K_MIDDLERELEASE => (2, false),
                    K_RIGHTDRAG | K_RIGHTMOUSE => (3, true),
                    K_RIGHTRELEASE => (3, false),
                    K_X1DRAG | K_X1MOUSE => (8, true),
                    K_X1RELEASE => (8, false),
                    K_X2DRAG | K_X2MOUSE => (9, true),
                    K_X2RELEASE => (9, false),
                    K_MOUSEDOWN => (4, true),
                    K_MOUSEUP => (5, true),
                    K_MOUSELEFT => (7, true),
                    K_MOUSERIGHT => (6, true),
                    K_MOUSEMOVE => (0, false),
                    _ => return false,
                };

                let mut mod_: VTermModifier = VTERM_MOD_NONE;
                convert_modifiers(&mut c, g_st().mod_mask, &mut mod_);
                mouse_action(term, button, row, col - offset, pressed, mod_);
                return false;
            }

            if c == K_MOUSEUP || c == K_MOUSEDOWN || c == K_MOUSELEFT || c == K_MOUSERIGHT {
                let save_curwin = curwin_ptr();
                set_curwin(mouse_win);
                g_st().curbuf = curwin_mut().w_buffer;

                let mut cap = Cmdarg::new();
                let mut oa = Oparg::new();
                cap.oap = &mut oa;
                cap.cmdchar = c;
                cap.arg = match c {
                    K_MOUSEUP => MSCR_UP,
                    K_MOUSEDOWN => MSCR_DOWN,
                    K_MOUSELEFT => MSCR_LEFT,
                    K_MOUSERIGHT => MSCR_RIGHT,
                    _ => std::process::abort(),
                };

                do_mousescroll(&mut cap);

                curwin_mut().w_redr_status = true;
                set_curwin(save_curwin);
                g_st().curbuf = curwin_mut().w_buffer;
                redraw_later(Some(&mut *mouse_win), UPD_NOT_VALID);
                invalidate_terminal(term, -1, -1);
                return std::ptr::eq(mouse_win, curwin_ptr());
            }
        }

        if (c == K_LEFTRELEASE && !mouse_win.is_null() && (*mouse_win).buf_mut().terminal == term)
            || c == K_MOUSEMOVE
        {
            return false;
        }

        requeue_key(g_st().vgetc_char, g_st().vgetc_mod_mask, 0, true);
        true
    }
}

fn fetch_row(term: &mut Terminal, row: i32, end_col: i32) {
    let mut col: i32 = 0;
    let mut line_len: usize = 0;
    let textbuf_ptr = (&raw mut term.textbuf).cast::<c_char>();
    let mut ptr = textbuf_ptr;

    while col < end_col {
        let mut cell = VTermScreenCell::default();
        fetch_cell(term, row, col, &mut cell);
        if cell.schar != 0 {
            schar_get_adv(&mut ptr, cell.schar);
            line_len = crate::memory::ptr_offset_from(ptr, textbuf_ptr) as usize;
        } else {
            unsafe {
                *ptr = b' ' as c_char;
            }
            ptr = ptr.wrapping_add(1);
        }
        col += i32::from(cell.width);
    }

    unsafe {
        *textbuf_ptr.wrapping_add(line_len) = NUL;
    }
}

fn fetch_cell(term: &mut Terminal, row: i32, col: i32, cell: &mut VTermScreenCell) -> bool {
    if row < 0 {
        let sbrow = &term.sb_buffer.as_slice()[(-row - 1) as usize];
        if (col as usize) < sbrow.len() {
            *cell = sbrow[col as usize];
        } else {
            *cell = VTermScreenCell {
                schar: 0,
                width: 1,
                ..Default::default()
            };
            return false;
        }
    } else {
        vterm_screen_get_cell(
            unsafe { &*term.vt.screen_mut_ptr() },
            VTermPos { row, col },
            cell,
        );
    }
    true
}

fn invalidate_terminal(term: &mut Terminal, start_row: i32, end_row: i32) {
    if start_row != -1 && end_row != -1 {
        term.invalid_start = term.invalid_start.min(start_row);
        term.invalid_end = term.invalid_end.max(end_row);
    }

    if term.synchronized_output {
        return;
    }

    invalidate_terminal_add(term);
    if !term_st().refresh_pending {
        time_watcher_start(&mut *refresh_timer_ptr(), REFRESH_DELAY, 0);
        term_st().refresh_pending = true;
    }
}

pub fn terminal_check_refresh() {
    unsafe {
        multiqueue_process_events(&mut *refresh_timer_ptr().events);
    }
}

fn refresh_terminal(term: &mut Terminal) {
    let buf = handle_get_buffer(term.buf_handle);
    if buf.is_null() {
        return;
    }
    let buf = { unsafe { &mut *buf } };
    let ml_before = buf.b_ml.ml_line_count;

    let resized = refresh_size(term);
    refresh_scrollback(term, &mut *buf);
    refresh_screen(term, &mut *buf);

    let ml_added = buf.b_ml.ml_line_count - ml_before;
    adjust_topline_cursor(term, &mut *buf, ml_added);

    if resized {
        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_ptr(&*tp) {
                if wp.w_buffer == buf && wp.w_leftcol != 0 {
                    wp.w_leftcol = 0;
                    curs_columns(&mut *wp, 1);
                }
            }
        }
    }

    multiqueue_move_events(
        unsafe { &mut *loop_queues(&mut g_st().main_loop).events },
        unsafe { &mut *term.pending.events },
    );
}

fn refresh_cursor(term: &mut Terminal, cursor_visible: &mut bool) {
    if !is_focused(term) {
        return;
    }
    if term.cursor.visible != *cursor_visible {
        *cursor_visible = term.cursor.visible;
        if *cursor_visible {
            ui_busy_stop();
        } else {
            ui_busy_start();
        }
    }

    if !term.pending.cursor {
        return;
    }
    term.pending.cursor = false;

    if term.cursor.blink {
        (*shape_table_mut())[SHAPE_IDX_TERM].blinkon = 500;
        (*shape_table_mut())[SHAPE_IDX_TERM].blinkoff = 500;
    } else {
        (*shape_table_mut())[SHAPE_IDX_TERM].blinkon = 0;
        (*shape_table_mut())[SHAPE_IDX_TERM].blinkoff = 0;
    }

    match term.cursor.shape {
        VTERM_PROP_CURSORSHAPE_BLOCK => {
            (*shape_table_mut())[SHAPE_IDX_TERM].shape = CursorShape::ShapeBlock;
        }
        VTERM_PROP_CURSORSHAPE_UNDERLINE => {
            (*shape_table_mut())[SHAPE_IDX_TERM].shape = CursorShape::ShapeHor;
            (*shape_table_mut())[SHAPE_IDX_TERM].percentage = 20;
        }
        VTERM_PROP_CURSORSHAPE_BAR_LEFT => {
            (*shape_table_mut())[SHAPE_IDX_TERM].shape = CursorShape::ShapeVer;
            (*shape_table_mut())[SHAPE_IDX_TERM].percentage = 25;
        }
        _ => {}
    }

    ui_mode_info_set();
}

pub(crate) fn refresh_timer_cb() {
    term_st().refresh_pending = false;
    if g_st().exiting {
        return;
    }

    block_autocmds();
    let to_refresh = std::mem::take(&mut term_st().invalidated_terminals);

    for term_ptr in to_refresh {
        let term = unsafe { &mut *term_ptr };
        if !term.synchronized_output {
            refresh_terminal(term);
        }
    }

    unblock_autocmds();
}

fn refresh_size(term: &mut Terminal) -> bool {
    if !term.pending.resize || term.closed {
        return false;
    }

    term.pending.resize = false;
    let mut width: i32 = 0;
    let mut height: i32 = 0;
    vterm_get_size(&term.vt, &mut height, &mut width);
    term.invalid_start = 0;
    term.invalid_end = height;
    channel_term_resize(width as u16, height as u16, unsafe { &mut *term.opts.data });
    true
}

pub fn on_scrollback_option_changed(term: &mut Terminal) {
    if term.sb_allocated {
        refresh_terminal(term);
    }
}

fn adjust_scrollback(term: &mut Terminal, buf: &mut FileBuf) {
    if buf.b_p_scbk < 1 {
        buf.b_p_scbk = SB_MAX;
    }
    let scbk = buf.b_p_scbk as usize;
    debug_assert!(term.sb_buffer.len() < usize::MAX);
    if term.sb_pending > 0 {
        std::process::abort();
    }

    if scbk < term.sb_buffer.len() {
        let diff = term.sb_buffer.len() - scbk;
        for _ in 0..diff {
            ml_delete_buf(buf, 1, false);
            term.sb_buffer.pop();
        }
        mark_adjust_buf(
            &mut *buf,
            1,
            diff as Linenr,
            MAXLNUM,
            -(diff as Linenr),
            true,
            K_MARK_ADJUST_TERM,
            K_EXTMARK_UNDO,
        );
        deleted_lines_buf(buf, 1, diff as Linenr);
    }

    term.sb_size = scbk;
}

fn refresh_scrollback(term: &mut Terminal, buf: &mut FileBuf) {
    channel_term_read_pause(true, unsafe { &mut *term.opts.data });

    let mut deleted = (term.sb_deleted - term.old_sb_deleted) as Linenr;
    deleted = deleted.min(buf.b_ml.ml_line_count);
    mark_adjust_buf(
        &mut *buf,
        1,
        deleted,
        MAXLNUM,
        -deleted,
        true,
        K_MARK_ADJUST_TERM,
        K_EXTMARK_UNDO,
    );
    term.old_sb_deleted = term.sb_deleted;

    let mut old_height = term.old_height;
    let mut width: i32 = 0;
    let mut height: i32 = 0;
    vterm_get_size(&term.vt, &mut height, &mut width);

    while deleted > 0 && buf.b_ml.ml_line_count > old_height {
        ml_delete_buf(buf, 1, false);
        deleted_lines_buf(buf, 1, 1);
        deleted -= 1;
    }

    old_height = old_height.min(buf.b_ml.ml_line_count);
    let textbuf_ptr = (&raw mut term.textbuf).cast::<c_char>();
    while term.sb_pending > 0 {
        fetch_row(term, -term.sb_pending, width);
        let buf_index = buf.b_ml.ml_line_count - old_height;
        ml_append_buf(buf, buf_index, textbuf_ptr, 0, false);
        appended_lines_buf(buf, buf_index, 1);
        term.sb_pending -= 1;
    }

    let max_line_count = term.sb_buffer.len() as i32 + height;
    loop {
        if buf.b_ml.ml_line_count <= max_line_count {
            break;
        }
        ml_delete_buf(buf, buf.b_ml.ml_line_count, false);
        deleted_lines_buf(buf, buf.b_ml.ml_line_count, 1);
    }

    adjust_scrollback(term, buf);

    channel_term_read_pause(false, unsafe { &mut *term.opts.data });
}

fn refresh_screen(term: &mut Terminal, buf: &mut FileBuf) {
    let mut changed: i32 = 0;
    let mut added: i32 = 0;
    let mut height: i32 = 0;
    let mut width: i32 = 0;
    vterm_get_size(&term.vt, &mut height, &mut width);
    term.invalid_end = term.invalid_end.min(height);

    if term.invalid_start >= term.invalid_end {
        term.invalid_start = i32::MAX;
        term.invalid_end = -1;
        return;
    }

    let textbuf_ptr = (&raw mut term.textbuf).cast::<c_char>();
    let mut r = term.invalid_start;
    let mut linenr = row_to_linenr(term, r);
    while r < term.invalid_end {
        fetch_row(term, r, width);

        if linenr <= buf.b_ml.ml_line_count {
            ml_replace_buf(buf, linenr, textbuf_ptr, true, false);
            changed += 1;
        } else {
            ml_append_buf(buf, linenr - 1, textbuf_ptr, 0, false);
            added += 1;
        }
        r += 1;
        linenr += 1;
    }
    term.old_height = height;

    let change_start = row_to_linenr(term, term.invalid_start);
    let change_end = change_start + changed;
    term.invalid_start = i32::MAX;
    term.invalid_end = -1;
    changed_lines(buf, change_start, 0, change_end, added, true);
}

fn adjust_topline_cursor(term: &mut Terminal, buf: &mut FileBuf, added: i32) {
    let ml_end = buf.b_ml.ml_line_count;

    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&*tp) {
            if wp.w_buffer == buf {
                if std::ptr::eq(wp, curwin_ptr()) && is_focused(term) {
                    terminal_check_cursor();
                    continue;
                }

                let following = ml_end == wp.w_cursor.lnum + added;
                if following {
                    wp.w_cursor.lnum = ml_end;
                    set_topline(wp, (wp.w_cursor.lnum - wp.w_view_height + 1).max(1));
                } else {
                    wp.w_cursor.lnum = wp.w_cursor.lnum.min(ml_end);
                }
                mb_check_adjust_col(&mut *wp);
            }
        }
    }

    if ml_end == buf.b_last_cursor.mark.lnum + added {
        buf.b_last_cursor.mark.lnum = ml_end;
    }

    for wip in &mut buf.b_wininfo {
        if ml_end == wip.wi_mark.mark.lnum + added {
            wip.wi_mark.mark.lnum = ml_end;
        }
    }
}

#[inline]
fn row_to_linenr(term: &mut Terminal, row: i32) -> i32 {
    if row != i32::MAX {
        row + term.sb_buffer.len() as i32 + 1
    } else {
        i32::MAX
    }
}

#[inline]
fn linenr_to_row(term: &mut Terminal, linenr: i32) -> i32 {
    linenr - term.sb_buffer.len() as i32 - 1
}

#[inline]
fn is_focused(term: &mut Terminal) -> bool {
    (g_st().state_global & MODE_TERMINAL) != 0 && curbuf_mut().terminal == term
}

fn get_config_string(buf: &mut FileBuf, key: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let key = key.into_c_char_ptr().cast_mut();
    {
        let mut err = Error::new();
        let key_str = NvimString {
            data: key,
            size: crate::memory::cstr_len(key),
        };
        let mut obj = dict_get_value(Some(&*buf.b_vars()), key_str, None, &mut err);
        api_clear_error(&mut err);
        if obj.obj_type == ObjectType::ObjectTypeNil {
            obj = dict_get_value(Some(&*get_globvar_dict()), key_str, None, &mut err);
            api_clear_error(&mut err);
        }
        if obj.obj_type == ObjectType::ObjectTypeString {
            return obj.data.string().data;
        }
        api_free_object(obj);
        ptr::null_mut()
    }
}
