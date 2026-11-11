//! The input engine "bytes layer" (`src/nvim/input.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::api_clear_error;
use crate::api::private::validate::{Array, Error, ErrorType, NvimString};
use crate::api::vim::{is_internal_call, nvim_paste};
use crate::charset::{ascii_iswhite, is_special, to_special};
use crate::cursor::{Colnr, Linenr, get_cursor_line_ptr};
use crate::drawscreen::{SHOWCMD_COLS, setcursor, showmode, unshowmode, update_screen};
use crate::errors::*;
use crate::eval::garbage_collect;
use crate::eval::typval::{
    EvalFuncData, TypVal, VarType, Varnumber, tv_check_for_opt_dict_arg, tv_dict_get_bool,
    tv_dict_get_string, tv_dict_has_key, tv_get_bool_chk,
};
use crate::eval::vars::{
    VV_MOUSE_COL, VV_MOUSE_LNUM, VV_MOUSE_WIN, VV_MOUSE_WINID, set_vim_var_nr,
};
use crate::event::r#loop::{main_loop_events, uv_strerror};
use crate::event::multiqueue::{MultiQueue, multiqueue_empty};
use crate::ex_cmds::{CTRL_C_CONST, check_secure};
use crate::ex_docmd::update_topline_cursor;
use crate::ex_getln::{
    CTRL_O, K_LUA, K_OPT_BO_FLAG_ERROR, get_cmdline_info, putcmdline, redrawcmd, redrawcmdline,
    unputcmdline,
};
use crate::extmark::{K_FALSE, K_NONE, TriState};
use crate::globals::*;
use crate::input_cmdatom::{
    CmdSpec, atom_cmdline_set, atom_composite_abort, atom_map_start, atom_payload_end,
    atom_payload_start, atom_redo_cancel, atom_redo_prepped, atom_typed_add, atom_typed_del,
};
use crate::insert::{edit_putchar, edit_unputchar};
use crate::insexpand::{
    compl_status_local, ctrl_x_mode_eval, ctrl_x_mode_not_default, vim_is_ctrl_x_key,
};
use crate::keycodes::{
    CAR, DEL, ESC, K_C_END, K_C_HOME, K_COMMAND, K_DOWN, K_END, K_HOME, K_IGNORE, K_K0, K_K1, K_K2,
    K_K3, K_K4, K_K5, K_K6, K_K7, K_K8, K_K9, K_KCOMMA, K_KDIVIDE, K_KDOWN, K_KENTER, K_KEQUAL,
    K_KLEFT, K_KMINUS, K_KMULTIPLY, K_KPLUS, K_KPOINT, K_KRIGHT, K_KUP, K_LEFT, K_LEFTMOUSE,
    K_MOUSEMOVE, K_RIGHT, K_S_END, K_S_HOME, K_SNR, K_SPECIAL, K_UP, K_XDOWN, K_XEND, K_XHOME,
    K_XLEFT, K_XRIGHT, K_XUP, K_ZEND, K_ZERO, K_ZHOME, KE_COMPLETE_DELAY, KE_FILLER, KE_IGNORE,
    KE_PLUG, KE_SNR, KS_EXTRA, KS_MODIFIER, MOD_MASK_ALT, MOD_MASK_CTRL, MOD_MASK_SHIFT, NL, TAB,
    special_to_buf, termcap2key,
};
use crate::lua::executor::{K_RET_NIL_BOOL, nlua_call_ref, nlua_exec_on_key};
use crate::mapping::{
    MAXMAPLEN, MatchedMap, REMAP_NONE, REMAP_YES, Typebuf, eval_map_expr, get_buf_maphash_list,
    langmap_adjust_mb, map_luaid_get, with_maphash_list,
};
use crate::mbyte::{
    mb_cptr2char_adv, mb_unescape, utf_byte2len, utf_char2bytes, utf_cluster_info,
    utf_ptr2_str_char_info, utf_ptr2cells, utf_ptr2char, utf_slice2char,
};
use crate::mcursor::mc_replaying;
use crate::memline::ml_sync_all;
use crate::memory::{Arena, arena_finish, arena_mem_free, bytes_to_cstring};
use crate::message::{emsg, semsg, semsg_multiline};
use crate::mouse::{is_mouse_key, mouse_comp_pos, mouse_find_win_inner};
use crate::r#move::{validate_cursor, win_col_off};
use crate::normal::{add_to_showcmd, normal_cmd, pop_showcmd, push_showcmd};
use crate::option_vars::*;
use crate::os::env::expand_env;
use crate::os::fileio::{
    FileDescriptor, K_FILE_NON_BLOCKING, K_FILE_READ_ONLY, file_close, file_open, file_open_stdin,
    file_read,
};
use crate::os::input::{input_available, input_get, line_breakcheck, os_breakcheck};
use crate::plines::{
    CSType, CharsizeArg, ClusterInfo, StrCharInfo, init_charsize_arg, win_charsize,
};
use crate::state::{
    MODE_ASKMORE, MODE_CMDLINE, MODE_HITRETURN, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL,
    MODE_SELECT, MODE_TERMINAL, MODE_VISUAL, get_real_state, state_handle_k_event,
    state_no_longer_safe,
};
use crate::strings::{format_to_buf, gettext};
use crate::textformat::Oparg;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_cursor_goto, ui_flush, vim_beep};
use crate::undo::u_sync;
use std::ffi::CStr;
use std::ptr;

use crate::option::OptInt;
pub type FlushBuffers = i32;

pub(crate) const NUL: i32 = 0;

pub(crate) const FLUSH_MINIMAL: FlushBuffers = 0;
const FLUSH_INPUT: FlushBuffers = 2;

const NSCRIPT: usize = 15;
const TYPELEN_INIT: usize = 5 * (MAXMAPLEN + 3);
const DUM_LEN: usize = MAXMAPLEN * 3 + 3;

const RM_YES: u8 = 0;
const RM_NONE: u8 = 1;
const RM_SCRIPT: u8 = 2;
const RM_ABBR: u8 = 4;

pub(crate) const REMAP_SCRIPT: i32 = -2;
const REMAP_SKIP: i32 = -3;

const KEYLEN_PART_KEY: i32 = -1;
const KEYLEN_PART_MAP: i32 = -2;

const KS_VER_SCROLLBAR: i32 = 249;
const KS_HOR_SCROLLBAR: i32 = 248;
const K_VER_SCROLLBAR: i32 = termcap2key(KS_VER_SCROLLBAR, KE_FILLER as i32);
const K_HOR_SCROLLBAR: i32 = termcap2key(KS_HOR_SCROLLBAR, KE_FILLER as i32);
pub(crate) const K_PASTE_START: i32 = termcap2key(b'P' as i32, b'S' as i32);
const K_PASTE_END: i32 = termcap2key(b'P' as i32, b'E' as i32);
pub(crate) const K_COMPLETE_DELAY: i32 = termcap2key(KS_EXTRA as i32, KE_COMPLETE_DELAY as i32);

const K_SELECT_STRING: &[u8; 4] = b"\x80\xf5X\0";
const ESC_STR: &CStr = c"\x1b";
const NL_STR: &CStr = c"\n";

pub(crate) const K_INS_NONE: i32 = 0;

const E_RECURSIVE_MAPPING: &CStr = c"E223: Recursive mapping";
const E_CMD_MAPPING_MUST_END_WITH_CR: &CStr = c"E1255: <Cmd> mapping must end with <CR>";
const E_CMD_MAPPING_MUST_END_WITH_CR_BEFORE_SECOND_CMD: &CStr =
    c"E1136: <Cmd> mapping must end with <CR> before second <Cmd>";
const E_LUA_MAPPING_GONE: &CStr = c"E5117: Lua mapping no longer exists";

/// Readahead ("stuff") buffer.
#[repr(C)]
#[derive(Clone)]
pub struct StuffBuf {
    pub keys: Vec<u8>,
    pub read: usize,
    pub insert: usize,
}

impl StuffBuf {
    pub const fn new() -> Self {
        Self {
            keys: Vec::new(),
            read: 0,
            insert: 0,
        }
    }
}

/// Dot-repeat state: the pending change and the previous one.
#[repr(C)]
#[derive(Clone)]
pub struct RedoState {
    pub cur: CmdSpec,
    pub old: CmdSpec,
}

impl RedoState {
    pub const fn new() -> Self {
        Self {
            cur: CmdSpec::new(),
            old: CmdSpec::new(),
        }
    }
}

impl Default for RedoState {
    fn default() -> Self {
        Self::new()
    }
}

/// Char put back by `vungetc()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UngotKey {
    pub c: i32,
    pub mod_mask: i32,
    pub mouse_grid: i32,
    pub mouse_row: i32,
    pub mouse_col: i32,
    pub stuffed: bool,
}

/// Struct to hold the saved typeahead for `save_typeahead()`.
#[repr(C)]
#[derive(Clone)]
pub struct Tasave {
    pub save_typebuf: Typebuf,
    pub ungot: UngotKey,
    pub save_readbuf1: StuffBuf,
    pub save_readbuf2: StuffBuf,
}

impl Tasave {
    pub const fn new() -> Self {
        Self {
            save_typebuf: Typebuf::ZERO,
            ungot: UngotKey {
                c: 0,
                mod_mask: 0,
                mouse_grid: 0,
                mouse_row: 0,
                mouse_col: 0,
                stuffed: false,
            },
            save_readbuf1: StuffBuf::new(),
            save_readbuf2: StuffBuf::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct GotcharsState {
    buf: [u8; MB_MAXBYTES * 3 + 4],
    prev_c: i32,
    buflen: usize,
    pending_special: u32,
    pending_mbyte: u32,
}

impl GotcharsState {
    const fn new() -> Self {
        Self {
            buf: [0; MB_MAXBYTES * 3 + 4],
            prev_c: 0,
            buflen: 0,
            pending_special: 0,
            pending_mbyte: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum MapResult {
    MapResultFail = 0,
    MapResultGet = 1,
    MapResultRetry = 2,
    MapResultNomatch = 3,
}

struct InputModuleState {
    curscript: std::cell::Cell<i32>,
    ungot: std::cell::Cell<UngotKey>,
    on_key_ignore_len: std::cell::Cell<usize>,
    on_key_blocked: std::cell::Cell<i32>,
    typeahead_char: std::cell::Cell<i32>,
    block_redo: std::cell::Cell<bool>,
    key_noremap: std::cell::Cell<i32>,
    last_recorded_len: std::cell::Cell<usize>,
    no_reduce_keys: std::cell::Cell<i32>,
    gotchars_state: std::cell::Cell<GotcharsState>,
    updatescript_count: std::cell::Cell<i32>,
    showcmd_state: std::cell::Cell<GotcharsState>,
    last_vgetc_recorded_len: std::cell::Cell<usize>,
    vgetorpeek_tc: std::cell::Cell<i32>,
}

impl InputModuleState {
    const fn new() -> Self {
        Self {
            curscript: std::cell::Cell::new(-1),
            ungot: std::cell::Cell::new(UngotKey {
                c: -1,
                mod_mask: 0,
                mouse_grid: 0,
                mouse_row: 0,
                mouse_col: 0,
                stuffed: false,
            }),
            on_key_ignore_len: std::cell::Cell::new(0),
            on_key_blocked: std::cell::Cell::new(0),
            typeahead_char: std::cell::Cell::new(0),
            block_redo: std::cell::Cell::new(false),
            key_noremap: std::cell::Cell::new(0),
            last_recorded_len: std::cell::Cell::new(0),
            no_reduce_keys: std::cell::Cell::new(0),
            gotchars_state: std::cell::Cell::new(GotcharsState::new()),
            updatescript_count: std::cell::Cell::new(0),
            showcmd_state: std::cell::Cell::new(GotcharsState::new()),
            last_vgetc_recorded_len: std::cell::Cell::new(0),
            vgetorpeek_tc: std::cell::Cell::new(0),
        }
    }
}

struct InputExtState {
    scriptin: std::cell::RefCell<[FileDescriptor; NSCRIPT]>,
    redobuff: std::cell::RefCell<RedoState>,
    recordbuff: std::cell::RefCell<Vec<u8>>,
    readbuf1: std::cell::RefCell<StuffBuf>,
    readbuf2: std::cell::RefCell<StuffBuf>,
    on_key_buf: std::cell::RefCell<Vec<u8>>,
    saved_typebuf: std::cell::RefCell<[Typebuf; NSCRIPT]>,
}

impl InputExtState {
    const fn new() -> Self {
        Self {
            scriptin: std::cell::RefCell::new([const { FileDescriptor::new() }; NSCRIPT]),
            redobuff: std::cell::RefCell::new(RedoState::new()),
            recordbuff: std::cell::RefCell::new(Vec::new()),
            readbuf1: std::cell::RefCell::new(StuffBuf::new()),
            readbuf2: std::cell::RefCell::new(StuffBuf::new()),
            on_key_buf: std::cell::RefCell::new(Vec::new()),
            saved_typebuf: std::cell::RefCell::new([const { Typebuf::ZERO }; NSCRIPT]),
        }
    }
}

thread_local! {
    static INP_STATE: InputModuleState = const { InputModuleState::new() };
    static INP_EXT: InputExtState = const { InputExtState::new() };
}

#[inline]
fn with_inp_ext<R>(f: impl FnOnce(&InputExtState) -> R) -> R {
    INP_EXT.with(f)
}

use crate::api::buffer::LUA_INTERNAL_CALL;
use crate::cmdexpand::{CTRL_N, CTRL_P, MB_MAXBYTES};
use crate::keycodes::{k_second, k_third};
use crate::lua::converter::LUA_NOREF;
use crate::regexp::CTRL_V_CONST;

#[inline]
pub fn mb_byte2len_check(b: i32) -> i32 {
    if !(0..=255).contains(&b) {
        1
    } else {
        utf_byte2len(b)
    }
}

#[inline]
pub(crate) fn langmap_adjust(c: &mut i32, condition: bool) {
    if !opt_vars().p_langmap.is_empty()
        && condition
        && (opt_vars().p_lrm
            || if g_st().vgetc_busy != 0 {
                typebuf_maplen() == 0
            } else {
                g_st().key_typed
            })
        && g_st().key_stuffed == 0
        && *c >= 0
    {
        if *c < 256 {
            *c = g_st().langmap_mapchar[*c as usize] as i32;
        } else {
            *c = langmap_adjust_mb(*c);
        }
    }
}

/// Gets the contents of the record buffer as a string (typeahead encoding) and clears the buffer.
pub fn get_recorded() -> std::ffi::CString {
    let mut bytes: Vec<u8> = with_inp_ext(|ext| std::mem::take(&mut *ext.recordbuff.borrow_mut()));
    let mut len = bytes.len();

    if len >= INP_STATE.with(|s| s.last_recorded_len.get()) {
        len -= INP_STATE.with(|s| s.last_recorded_len.get());
        bytes.truncate(len);
    }

    if len > 0 && g_st().restart_edit != 0 && bytes[len - 1] == CTRL_O as u8 {
        bytes.pop();
    }

    crate::memory::bytes_to_cstring(&bytes)
}

fn redo_prefix(spec: &CmdSpec, buf: &mut Vec<u8>, replay: bool) {
    if spec.regname != 0 {
        sb_add_char(buf, b'"' as i32);
        sb_add_char(buf, spec.regname);
        if replay && spec.regname == b'=' as i32 {
            sb_add_char(buf, CAR);
        }
    }
    if spec.count != 0 {
        let mut numbuf = [0u8; 32];
        let n = format_to_buf!(&mut numbuf, c"%d", spec.count as i32);
        if n > 0 {
            buf.extend_from_slice(&numbuf[..n as usize]);
        }
    }
}

fn redo_chars(spec: &CmdSpec, buf: &mut Vec<u8>, arg_meta: bool) {
    if spec.op != NUL {
        sb_add_char(buf, spec.op);
    }
    if spec.op_extra != NUL {
        sb_add_char(buf, spec.op_extra);
    }
    if spec.motion_force != NUL {
        sb_add_char(buf, spec.motion_force);
    }
    if spec.cmd != NUL {
        sb_add_char(buf, spec.cmd);
    }
    if spec.cmd2 != NUL {
        sb_add_char(buf, spec.cmd2);
    }
    if spec.cmdarg != NUL && !arg_meta {
        sb_add_char(buf, spec.cmdarg);
    }
}

fn format_redo_keys(spec: &CmdSpec) -> Option<std::ffi::CString> {
    let mut buf: Vec<u8> = Vec::new();

    redo_prefix(spec, &mut buf, false);

    if !spec.body.is_empty() {
        buf.extend_from_slice(&spec.body);
    } else {
        redo_chars(spec, &mut buf, false);
    }
    if buf.is_empty() {
        None
    } else {
        Some(crate::memory::bytes_to_cstring(&buf))
    }
}

/// Gets `spec` (or current pending change, if `spec` is `None`), composed into a "redo" keyseq.
pub fn redo_keys(spec: Option<&CmdSpec>) -> Option<std::ffi::CString> {
    if let Some(spec) = spec {
        format_redo_keys(spec)
    } else {
        with_inp_ext(|ext| format_redo_keys(&ext.redobuff.borrow().cur))
    }
}

/// Gets the pending change's `CmdSpec` (without `body`).
pub fn redo_spec() -> CmdSpec {
    with_inp_ext(|ext| ext.redobuff.borrow().cur.without_body())
}

fn add_buff(buf: &mut StuffBuf, slice: &[u8]) {
    if slice.is_empty() {
        return;
    }
    let insert = buf.insert;
    buf.keys.splice(insert..insert, slice.iter().copied());
    buf.insert += slice.len();
}

fn add_num_buff(buf: &mut StuffBuf, n: i32) {
    let mut number = [0u8; 32];
    let numberlen = format_to_buf!(&mut number, c"%d", n) as usize;
    add_buff(&mut *buf, &number[..numberlen]);
}

fn key_byte_encode(c: i32, temp: &mut [u8; 4]) -> isize {
    if is_special(c) || c == K_SPECIAL as i32 || c == NUL {
        temp[0] = K_SPECIAL;
        temp[1] = k_second(c);
        temp[2] = k_third(c);
        temp[3] = 0;
        3
    } else {
        temp[0] = c as u8;
        temp[1] = 0;
        1
    }
}

fn key_char_encode(c: i32, buf: &mut [u8]) -> usize {
    let mut bytes = [0u8; MB_MAXBYTES + 1];
    let len = if is_special(c) {
        1
    } else {
        utf_char2bytes(c, &mut bytes)
    };
    let mut off: usize = 0;
    for &b in bytes.iter().take(len as usize) {
        let mut tmp = [0u8; 4];
        let n = key_byte_encode(if is_special(c) { c } else { b as i32 }, &mut tmp) as usize;
        buf[off..off + n + 1].copy_from_slice(&tmp[..n + 1]);
        off += n;
    }
    off
}

fn add_char_buff(buf: &mut StuffBuf, c: i32) {
    let mut temp = [0u8; MB_MAXBYTES * 3 + 1];
    let len = key_char_encode(c, &mut temp);
    add_buff(buf, &temp[..len]);
}

fn sb_add_byte(sb: &mut Vec<u8>, c: i32) {
    let mut temp = [0u8; 4];
    let templen = key_byte_encode(c, &mut temp) as usize;
    sb.extend_from_slice(&temp[..templen]);
}

/// Dual of `add_char_buff()`, for a byte buffer.
pub fn sb_add_char(sb: &mut Vec<u8>, c: i32) {
    let mut temp = [0u8; MB_MAXBYTES * 3 + 1];
    let len = key_char_encode(c, &mut temp);
    sb.extend_from_slice(&temp[..len]);
}

fn read_readbuffers(advance: bool) -> i32 {
    with_inp_ext(|ext| {
        let mut c = read_readbuf(&mut ext.readbuf1.borrow_mut(), advance);
        if c == NUL {
            c = read_readbuf(&mut ext.readbuf2.borrow_mut(), advance);
        }
        c
    })
}

fn read_readbuf(buf: &mut StuffBuf, advance: bool) -> i32 {
    if buf.read == buf.keys.len() {
        return NUL;
    }

    let c = i32::from(buf.keys.as_slice()[buf.read]);

    if advance {
        buf.read += 1;
        if buf.read == buf.keys.len() {
            buf.keys.clear();
            buf.read = 0;
            buf.insert = 0;
        } else {
            buf.insert = buf.insert.max(buf.read);
        }
    }
    c
}

fn start_stuff() {
    with_inp_ext(|ext| {
        let mut rb1 = ext.readbuf1.borrow_mut();
        rb1.insert = rb1.read;
        let mut rb2 = ext.readbuf2.borrow_mut();
        rb2.insert = rb2.read;
    });
}

/// Returns true if the readahead ("stuff") buffer is empty.
pub fn stuff_empty() -> bool {
    with_inp_ext(|ext| {
        ext.readbuf1.borrow().keys.is_empty() && ext.readbuf2.borrow().keys.is_empty()
    })
}

/// Returns true if `INP_STATE.with(|s| s.readbuf1.get())` is empty.
pub fn readbuf1_empty() -> bool {
    with_inp_ext(|ext| ext.readbuf1.borrow().keys.is_empty())
}

/// Set a typeahead character that won't be flushed.
pub fn typeahead_noflush(c: i32) {
    INP_STATE.with(|s| s.typeahead_char.set(c));
}

/// Remove the contents of the stuff buffer and the mapped characters in the typeahead buffer.
pub fn flush_buffers(flush_typeahead: FlushBuffers) {
    init_typebuf();

    if g_st().typebuf.tb_maplen > 0 {
        atom_composite_abort();
    }

    if with_inp_ext(|ext| !ext.readbuf2.borrow().keys.is_empty()) && g_st().visual_global.active {
        g_st().visual_global.need_end = true;
    }
    with_inp_ext(|ext| {
        *ext.readbuf1.borrow_mut() = StuffBuf::new();
        *ext.readbuf2.borrow_mut() = StuffBuf::new();
    });

    if flush_typeahead == FLUSH_MINIMAL {
        if g_st().typebuf.tb_off + g_st().typebuf.tb_maplen >= g_st().typebuf.tb_buflen {
            g_st().typebuf.tb_off = MAXMAPLEN as i32;
            g_st().typebuf.tb_len = 0;
        } else {
            g_st().typebuf.tb_off += g_st().typebuf.tb_maplen;
            g_st().typebuf.tb_len -= g_st().typebuf.tb_maplen;
        }
        if g_st().typebuf.tb_len == 0 {
            g_st().typebuf_was_filled = false;
        }
    } else {
        if flush_typeahead == FLUSH_INPUT {
            init_typebuf();
            while inchar(0, g_st().typebuf.tb_buflen - 1, 10) != 0 {}
        }
        g_st().typebuf.tb_off = MAXMAPLEN as i32;
        g_st().typebuf.tb_len = 0;
        g_st().typebuf_was_filled = false;
    }
    g_st().typebuf.tb_maplen = 0;
    g_st().typebuf.tb_silent = 0;
    g_st().cmd_silent = false;
    g_st().typebuf.tb_no_abbr_cnt = 0;
    g_st().typebuf.tb_change_cnt = g_st().typebuf.tb_change_cnt.wrapping_add(1);
    if g_st().typebuf.tb_change_cnt == 0 {
        g_st().typebuf.tb_change_cnt = 1;
    }
}

/// Hides keys from `vim.on_key()` callbacks.
pub fn block_on_key() {
    INP_STATE.with(|s| s.on_key_blocked.set(s.on_key_blocked.get() + (1)));
}

pub fn unblock_on_key() {
    INP_STATE.with(|s| s.on_key_blocked.set(s.on_key_blocked.get() - (1)));
}

/// Flush map and typeahead buffers and give a warning for an error.
pub fn beep_flush() {
    if g_st().emsg_silent == 0 && !mc_replaying() {
        flush_buffers(FLUSH_MINIMAL);
    }
    vim_beep(K_OPT_BO_FLAG_ERROR);
}

/// Starts capturing a new change.
pub fn redo_new(spec: CmdSpec) {
    assert!(spec.body.is_empty());
    if INP_STATE.with(|s| s.block_redo.get()) {
        return;
    }

    with_inp_ext(|ext| {
        let mut rb = ext.redobuff.borrow_mut();
        rb.old = std::mem::replace(&mut rb.cur, spec);
    });
}

/// Frees both redo buffers.
pub fn redo_free_all() {
    with_inp_ext(|ext| *ext.redobuff.borrow_mut() = RedoState::new());
}

/// Prepare for redo of any command.
pub fn prep_redo(as_atom: bool, arg_meta: bool, spec: CmdSpec) {
    if as_atom {
        atom_redo_prepped();
    }
    redo_new(spec.without_body());
    if INP_STATE.with(|s| s.block_redo.get()) {
        return;
    }
    with_inp_ext(|ext| redo_chars(&spec, &mut ext.redobuff.borrow_mut().cur.body, arg_meta));
}

/// Prepare for redo of a Visual-mode command.
pub fn prep_redo_visual(keys: &[u8], spec: CmdSpec) {
    let mut stored = spec.without_body();
    stored.regname = 0;
    stored.count = 0;
    atom_redo_prepped();
    redo_new(stored);
    if INP_STATE.with(|s| s.block_redo.get()) {
        return;
    }
    with_inp_ext(|ext| {
        let mut rb = ext.redobuff.borrow_mut();
        if !keys.is_empty() {
            rb.cur.body.extend_from_slice(keys);
        }
        redo_prefix(&spec, &mut rb.cur.body, false);
        redo_chars(&spec, &mut rb.cur.body, false);
    });
}

/// Discard the contents of the redo buffer and restore the previous redo buffer.
pub fn redo_cancel() {
    if INP_STATE.with(|s| s.block_redo.get()) {
        return;
    }

    atom_redo_cancel();
    with_inp_ext(|ext| {
        let mut rb = ext.redobuff.borrow_mut();
        rb.cur = std::mem::take(&mut rb.old);
        *ext.readbuf1.borrow_mut() = StuffBuf::new();
        *ext.readbuf2.borrow_mut() = StuffBuf::new();
    });
}

/// Saves the redo state to `save_redo` and detaches it.
pub fn save_redobuff(save_redo: &mut RedoState) {
    with_inp_ext(|ext| {
        let mut rb = ext.redobuff.borrow_mut();
        save_redo.cur = rb.cur.clone();
        save_redo.old = std::mem::take(&mut rb.old);
    });
}

/// Restores the redo state from `save_redo`.
pub fn restore_redobuff(save_redo: &mut RedoState) {
    with_inp_ext(|ext| *ext.redobuff.borrow_mut() = std::mem::take(&mut *save_redo));
}

/// Append `s` to the redo buffer.
pub fn redo_append_str(s: &[u8]) {
    if !INP_STATE.with(|s| s.block_redo.get()) && !s.is_empty() {
        with_inp_ext(|ext| ext.redobuff.borrow_mut().cur.body.extend_from_slice(s));
    }
}

/// Appends `str_bytes` to `buf` literally, escaping special characters with CTRL-V.
pub fn sb_add_lit(buf: &mut Vec<u8>, str_bytes: &[u8]) {
    let mut idx = 0usize;
    let total = str_bytes.len();
    while idx < total {
        let start = idx;
        while idx < total && (b' '..DEL as u8).contains(&str_bytes[idx]) {
            idx += 1;
        }

        if idx == total && idx > 0 && matches!(str_bytes[idx - 1], b'0' | b'^') {
            idx -= 1;
        }
        if idx > start {
            buf.extend_from_slice(&str_bytes[start..idx]);
        }

        if idx >= total {
            break;
        }

        let rem_bytes = &str_bytes[idx..];
        let mut rem = rem_bytes;
        let c = mb_cptr2char_adv(&mut rem);
        idx += (rem_bytes.len() - rem.len()).max(1);
        if c < b' ' as i32 || c == DEL || (idx >= total && (c == b'0' as i32 || c == b'^' as i32)) {
            sb_add_char(buf, CTRL_V_CONST);
        }

        if idx >= total && c == b'0' as i32 {
            buf.extend_from_slice(b"048");
        } else {
            sb_add_char(buf, c);
        }
    }
}

/// Append to redo literally; no-op if `INP_STATE.with(|s| s.block_redo.get())` is set.
pub fn redo_append_lit(s: &[u8]) {
    if INP_STATE.with(|s| s.block_redo.get()) {
        return;
    }
    with_inp_ext(|ext| sb_add_lit(&mut ext.redobuff.borrow_mut().cur.body, s));
}

/// Appends `s` to `buf`, leaving 3-byte special key codes unmodified and escaping other K_SPECIAL bytes.
pub fn sb_add_spec(buf: &mut Vec<u8>, rem: &[u8]) {
    let mut rem = crate::memory::bytes_until_nul(rem);
    while !rem.is_empty() {
        if rem.len() >= 3 && rem[0] == K_SPECIAL && rem[1] != 0 && rem[2] != 0 {
            buf.extend_from_slice(&rem[..3]);
            rem = &rem[3..];
        } else {
            let c = mb_cptr2char_adv(&mut rem);
            sb_add_char(buf, c);
        }
    }
}

/// Append `s` to the redo buffer.
pub fn redo_append_spec(s: &[u8]) {
    if INP_STATE.with(|s| s.block_redo.get()) {
        return;
    }
    with_inp_ext(|ext| sb_add_spec(&mut ext.redobuff.borrow_mut().cur.body, s));
}

/// Appends character `c` to the redo buffer, translated to typeahead encoding.
pub fn redo_append_char(c: i32) {
    if !INP_STATE.with(|s| s.block_redo.get()) {
        with_inp_ext(|ext| sb_add_char(&mut ext.redobuff.borrow_mut().cur.body, c));
    }
}

/// Appends string `s` (must be typeahead encoding) to the stuff buffer.
pub fn stuff_readbuff(s: &CStr) {
    with_inp_ext(|ext| add_buff(&mut ext.readbuf1.borrow_mut(), s.to_bytes()));
}

/// Appends string `s` (must be typeahead encoding) to the redo stuff buffer.
pub fn stuff_redo_readbuff(s: &std::ffi::CStr) {
    with_inp_ext(|ext| add_buff(&mut ext.readbuf2.borrow_mut(), s.to_bytes()));
}

pub fn stuff_readbuff_len(s: &[u8]) {
    if g_st().add_last_insert == 1 && !s.is_empty() {
        g_st().last_insert_ga.extend_from_slice(s);
    }
    with_inp_ext(|ext| add_buff(&mut ext.readbuf1.borrow_mut(), s));
}

/// Stuff `s` into the stuff buffer, leaving special key codes unmodified.
pub fn stuff_readbuff_special(s: &CStr) {
    let mut rem = s.to_bytes();
    while !rem.is_empty() {
        if rem.len() >= 3 && rem[0] == K_SPECIAL && rem[1] != 0 && rem[2] != 0 {
            stuff_readbuff_len(&rem[..3]);
            rem = &rem[3..];
        } else {
            let mut c = mb_cptr2char_adv(&mut rem);
            if c == CAR || c == NL || c == ESC {
                c = b' ' as i32;
            }
            stuffchar_readbuff(c);
        }
    }
}

/// Appends character `c` to the stuff buffer, translated to typeahead encoding.
pub fn stuffchar_readbuff(c: i32) {
    with_inp_ext(|ext| add_char_buff(&mut ext.readbuf1.borrow_mut(), c));
}

/// Append a number to the stuff buffer.
pub fn stuffnum_readbuff(n: i32) {
    with_inp_ext(|ext| add_num_buff(&mut ext.readbuf1.borrow_mut(), n));
}

/// Stuff a string into the typeahead buffer.
pub fn stuffescaped(rem: &[u8], literally: bool) {
    let mut rem = crate::memory::bytes_until_nul(rem);
    while !rem.is_empty() {
        let span_len = rem
            .iter()
            .take_while(|&&b| (b' '..DEL as u8).contains(&b) || (b == K_SPECIAL && !literally))
            .count();
        if span_len > 0 {
            stuff_readbuff_len(&rem[..span_len]);
            rem = &rem[span_len..];
        }

        if !rem.is_empty() {
            let c = mb_cptr2char_adv(&mut rem);
            if literally && ((c < b' ' as i32 && c != TAB) || c == DEL) {
                stuffchar_readbuff(CTRL_V_CONST);
            }
            stuffchar_readbuff(c);
        }
    }
}

/// Dot-repeat `"."` command.
pub fn start_redo(count: i32, old_redo: bool) -> bool {
    let Some((mut spec, body)) = with_inp_ext(|ext| {
        let mut rb = ext.redobuff.borrow_mut();
        let rd = if old_redo { &mut rb.old } else { &mut rb.cur };
        if rd.body.is_empty() && rd.regname == 0 && rd.count == 0 {
            return None;
        }
        let mut spec = rd.without_body();
        if spec.regname >= b'1' as i32 && spec.regname < b'9' as i32 {
            spec.regname += 1;
            rd.regname = spec.regname;
        }
        Some((spec, rd.body.clone()))
    }) else {
        return false;
    };

    if count != 0 {
        spec.count = i64::from(count);
    }
    let mut prefix: Vec<u8> = Vec::new();
    redo_prefix(&spec, &mut prefix, true);
    with_inp_ext(|ext| add_buff(&mut ext.readbuf2.borrow_mut(), &prefix));
    drop(prefix);
    if spec.regname == b'=' as i32 {
        g_st().cmd_silent = true;
    }

    with_inp_ext(|ext| add_buff(&mut ext.readbuf2.borrow_mut(), &body));
    true
}

/// Repeats the last insert (`R`, `o`, `O`, `a`, `A`, `i` or `I` command).
pub fn start_redo_ins() -> bool {
    if with_inp_ext(|ext| ext.redobuff.borrow().cur.body.is_empty()) {
        return false;
    }
    start_stuff();

    let (add_nl, rem) = with_inp_ext(|ext| {
        let body = &ext.redobuff.borrow().cur.body;
        let len = body.len();
        let mut pos = 0usize;
        let mut add_nl = false;
        while pos < len {
            let b = body[pos];
            if b == K_SPECIAL && len - pos >= 3 {
                pos += 3;
                continue;
            }
            if crate::strings::vim_haschr(c"AaIiRrOo", i32::from(b)) {
                if b == b'O' || b == b'o' {
                    add_nl = true;
                }
                pos += 1;
                break;
            }
            pos += 1;
        }
        (add_nl, body[pos..].to_vec())
    });
    if add_nl {
        with_inp_ext(|ext| add_buff(&mut ext.readbuf2.borrow_mut(), NL_STR.to_bytes()));
    }

    with_inp_ext(|ext| add_buff(&mut ext.readbuf2.borrow_mut(), &rem));
    INP_STATE.with(|s| s.block_redo.set(true));
    true
}

pub fn stop_redo_ins() {
    INP_STATE.with(|s| s.block_redo.set(false));
}

fn init_typebuf() {
    if !g_st().typebuf.tb_buf.is_empty() {
        return;
    }

    g_st().typebuf.tb_buf = vec![0u8; TYPELEN_INIT];
    g_st().typebuf.tb_noremap = vec![0u8; TYPELEN_INIT];
    g_st().typebuf.tb_buflen = TYPELEN_INIT as i32;
    g_st().typebuf.tb_len = 0;
    g_st().typebuf.tb_off = (MAXMAPLEN + 4) as i32;
    g_st().typebuf.tb_change_cnt = 1;
}

/// Returns true when keys cannot be remapped.
pub fn noremap_keys() -> bool {
    (INP_STATE.with(|s| s.key_noremap.get()) & ((RM_NONE | RM_SCRIPT) as i32)) != 0
}

/// Insert a string in position `offset` in the typeahead buffer.
pub fn ins_typebuf(str_: &CStr, noremap: i32, offset: i32, nottyped: bool, silent: bool) -> bool {
    init_typebuf();
    g_st().typebuf.tb_change_cnt = g_st().typebuf.tb_change_cnt.wrapping_add(1);
    if g_st().typebuf.tb_change_cnt == 0 {
        g_st().typebuf.tb_change_cnt = 1;
    }
    state_no_longer_safe(Some(c"ins_typebuf()"));

    let str_slice = str_.to_bytes();
    let add_usize = str_slice.len();
    let addlen = add_usize as i32;

    if offset == 0 && addlen <= g_st().typebuf.tb_off {
        g_st().typebuf.tb_off -= addlen;
        let off = g_st().typebuf.tb_off as usize;
        g_st().typebuf.tb_buf[off..off + add_usize].copy_from_slice(str_slice);
    } else if g_st().typebuf.tb_len == 0
        && g_st().typebuf.tb_buflen >= addlen + (3 * (MAXMAPLEN + 4)) as i32
    {
        g_st().typebuf.tb_off =
            (g_st().typebuf.tb_buflen - addlen - (3 * (MAXMAPLEN + 4)) as i32) / 2;
        let off = g_st().typebuf.tb_off as usize;
        g_st().typebuf.tb_buf[off..off + add_usize].copy_from_slice(str_slice);
    } else {
        let newoff = (MAXMAPLEN + 4) as i32;
        let extra = addlen + newoff + (4 * (MAXMAPLEN + 4)) as i32;
        if g_st().typebuf.tb_len > i32::MAX - extra {
            emsg(gettext(E_COMMAND_TOO_COMPLEX));
            setcursor();
            return false;
        }
        let newlen = g_st().typebuf.tb_len + extra;
        let mut s1_vec = vec![0u8; newlen as usize];
        let mut s2_vec = vec![0u8; newlen as usize];
        g_st().typebuf.tb_buflen = newlen;

        let tb_off = g_st().typebuf.tb_off as usize;
        let off_usize = offset as usize;
        let newoff_usize = newoff as usize;
        let bytes = (g_st().typebuf.tb_len - offset + 1) as usize;
        assert!(bytes > 0);
        let tail_noremap = (g_st().typebuf.tb_len - offset) as usize;

        s1_vec[newoff_usize..newoff_usize + off_usize]
            .copy_from_slice(&g_st().typebuf.tb_buf[tb_off..tb_off + off_usize]);
        s1_vec[newoff_usize + off_usize..newoff_usize + off_usize + add_usize]
            .copy_from_slice(str_slice);
        s1_vec[newoff_usize + off_usize + add_usize..newoff_usize + off_usize + add_usize + bytes]
            .copy_from_slice(
                &g_st().typebuf.tb_buf[tb_off + off_usize..tb_off + off_usize + bytes],
            );
        g_st().typebuf.tb_buf = s1_vec;

        s2_vec[newoff_usize..newoff_usize + off_usize]
            .copy_from_slice(&g_st().typebuf.tb_noremap[tb_off..tb_off + off_usize]);
        s2_vec[newoff_usize + off_usize + add_usize
            ..newoff_usize + off_usize + add_usize + tail_noremap]
            .copy_from_slice(
                &g_st().typebuf.tb_noremap[tb_off + off_usize..tb_off + off_usize + tail_noremap],
            );
        g_st().typebuf.tb_noremap = s2_vec;

        g_st().typebuf.tb_off = newoff;
    }
    g_st().typebuf.tb_len += addlen;

    let val = if noremap == REMAP_SCRIPT {
        RM_SCRIPT
    } else if noremap == REMAP_SKIP {
        RM_ABBR
    } else {
        RM_NONE
    };

    let mut nrm = if noremap == REMAP_SKIP {
        1
    } else if noremap < 0 {
        addlen
    } else {
        noremap
    };
    let start_idx = (g_st().typebuf.tb_off + offset) as usize;
    for i in 0..add_usize {
        nrm -= 1;
        g_st().typebuf.tb_noremap[start_idx + i] = if nrm >= 0 { val } else { RM_YES };
    }

    if nottyped || g_st().typebuf.tb_maplen > offset {
        g_st().typebuf.tb_maplen += addlen;
    }
    if silent || g_st().typebuf.tb_silent > offset {
        g_st().typebuf.tb_silent += addlen;
        g_st().cmd_silent = true;
    }
    if g_st().typebuf.tb_no_abbr_cnt != 0 && offset == 0 {
        g_st().typebuf.tb_no_abbr_cnt += addlen;
    }

    true
}

/// Re-queues key `c`: puts a key obtained by `vgetc()` back into typeahead.
pub fn requeue_key(c: i32, modifiers: i32, extra_consumed: i32, recorded: bool) {
    let mut buf = [0u8; MB_MAXBYTES * 3 + 4];
    let len = special_to_buf(c, modifiers, true, &mut buf) as usize;
    assert!(len < buf.len());
    buf[len] = 0;
    ins_typebuf(
        CStr::from_bytes_until_nul(&buf).unwrap_or_default(),
        INP_STATE.with(|s| s.key_noremap.get()),
        0,
        !g_st().key_typed,
        g_st().cmd_silent,
    );
    ungetchars(len as i32 + extra_consumed, recorded);
}

/// Return true if the typeahead buffer was changed.
pub fn typebuf_changed(tb_change_cnt: i32) -> bool {
    tb_change_cnt != 0
        && (g_st().typebuf.tb_change_cnt != tb_change_cnt || g_st().typebuf_was_filled)
}

/// Return true if there are no characters in the typeahead buffer that have not been typed.
pub fn typebuf_typed() -> i32 {
    i32::from(g_st().typebuf.tb_maplen == 0)
}

/// Get the number of characters that are mapped (or not typed).
pub fn typebuf_maplen() -> i32 {
    g_st().typebuf.tb_maplen
}

/// Remove `len` characters from `typebuf.tb_buf[typebuf.tb_off + offset]`.
pub fn del_typebuf(len: i32, offset: i32) {
    if len == 0 {
        return;
    }

    g_st().typebuf.tb_len -= len;

    if offset == 0
        && g_st().typebuf.tb_buflen - (g_st().typebuf.tb_off + len) >= (3 * MAXMAPLEN + 3) as i32
    {
        g_st().typebuf.tb_off += len;
    } else {
        let i = (g_st().typebuf.tb_off + offset) as usize;
        let off_usize = offset as usize;
        if g_st().typebuf.tb_off > MAXMAPLEN as i32 {
            let tb_off = g_st().typebuf.tb_off as usize;
            g_st()
                .typebuf
                .tb_buf
                .copy_within(tb_off..tb_off + off_usize, MAXMAPLEN);
            g_st()
                .typebuf
                .tb_noremap
                .copy_within(tb_off..tb_off + off_usize, MAXMAPLEN);
            g_st().typebuf.tb_off = MAXMAPLEN as i32;
        }
        let bytes = (g_st().typebuf.tb_len - offset + 1) as usize;
        assert!(bytes > 0);
        let src_start = i + len as usize;
        let dst_start = (g_st().typebuf.tb_off + offset) as usize;
        g_st()
            .typebuf
            .tb_buf
            .copy_within(src_start..src_start + bytes, dst_start);
        let noremap_bytes = (g_st().typebuf.tb_len - offset) as usize;
        g_st()
            .typebuf
            .tb_noremap
            .copy_within(src_start..src_start + noremap_bytes, dst_start);
    }

    if g_st().typebuf.tb_maplen > offset {
        if g_st().typebuf.tb_maplen < offset + len {
            g_st().typebuf.tb_maplen = offset;
        } else {
            g_st().typebuf.tb_maplen -= len;
        }
    }
    if g_st().typebuf.tb_silent > offset {
        if g_st().typebuf.tb_silent < offset + len {
            g_st().typebuf.tb_silent = offset;
        } else {
            g_st().typebuf.tb_silent -= len;
        }
    }
    if g_st().typebuf.tb_no_abbr_cnt > offset {
        if g_st().typebuf.tb_no_abbr_cnt < offset + len {
            g_st().typebuf.tb_no_abbr_cnt = offset;
        } else {
            g_st().typebuf.tb_no_abbr_cnt -= len;
        }
    }

    g_st().typebuf_was_filled = false;
    g_st().typebuf.tb_change_cnt = g_st().typebuf.tb_change_cnt.wrapping_add(1);
    if g_st().typebuf.tb_change_cnt == 0 {
        g_st().typebuf.tb_change_cnt = 1;
    }
}

fn gotchars_add_byte(state: &mut GotcharsState, byte: u8) -> bool {
    state.buf[state.buflen] = byte;
    state.buflen += 1;
    let mut c = byte as i32;
    let mut retval = false;
    let in_special = state.pending_special > 0;
    let in_mbyte = state.pending_mbyte > 0;

    'ret_false: {
        if in_special {
            state.pending_special -= 1;
        } else if c == K_SPECIAL as i32 {
            state.pending_special = 2;
        }

        if state.pending_special > 0 {
            break 'ret_false;
        }

        if in_mbyte {
            state.pending_mbyte -= 1;
        } else {
            if in_special {
                if state.prev_c == KS_MODIFIER as i32 {
                    break 'ret_false;
                }
                c = to_special(state.prev_c, c);
                if c == K_COMPLETE_DELAY {
                    state.buflen = 0;
                }
            }
            state.pending_mbyte = (mb_byte2len_check(c) - 1) as u32;
        }

        if state.pending_mbyte > 0 {
            break 'ret_false;
        }

        retval = true;
    }
    state.prev_c = c;
    retval
}

fn gotchars(chars: &[u8]) {
    for &b in chars {
        if !INP_STATE.with(|s| {
            let mut gs = s.gotchars_state.get();
            let ok = gotchars_add_byte(&mut gs, b);
            s.gotchars_state.set(gs);
            ok
        }) {
            continue;
        }

        let buflen = INP_STATE.with(|s| s.gotchars_state.get()).buflen;
        for i in 0..buflen {
            updatescript(INP_STATE.with(|s| s.gotchars_state.get()).buf[i] as i32);
        }

        if buflen > INP_STATE.with(|s| s.on_key_ignore_len.get()) {
            let ignore_len = INP_STATE.with(|s| s.on_key_ignore_len.get());
            let gotchars_buf = INP_STATE.with(|s| s.gotchars_state.get()).buf;
            let chunk = &gotchars_buf[ignore_len..buflen];
            with_inp_ext(|ext| ext.on_key_buf.borrow_mut().extend_from_slice(chunk));
            INP_STATE.with(|s| s.on_key_ignore_len.set(0));
        } else {
            INP_STATE.with(|s| {
                s.on_key_ignore_len
                    .set(s.on_key_ignore_len.get() - (buflen))
            });
        }

        if g_st().reg_recording != 0 {
            let gotchars_buf = INP_STATE.with(|s| s.gotchars_state.get()).buf;
            let chunk = &gotchars_buf[..buflen];
            with_inp_ext(|ext| ext.recordbuff.borrow_mut().extend_from_slice(chunk));
            let rec_len = INP_STATE
                .with(|s| s.last_recorded_len.get())
                .wrapping_add(buflen);
            INP_STATE.with(|s| s.last_recorded_len.set(rec_len));
        }

        let gotchars_buf = INP_STATE.with(|s| s.gotchars_state.get()).buf;
        atom_typed_add(&gotchars_buf[..buflen]);

        INP_STATE.with(|s| {
            let mut gs = s.gotchars_state.get();
            gs.buflen = 0;
            s.gotchars_state.set(gs);
        });
    }

    may_sync_undo();
    g_st().debug_did_msg = false;
    g_st().maptick += 1;
}

/// Record an `<Ignore>` key.
pub fn gotchars_ignore() {
    let nop_buf: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_IGNORE];
    INP_STATE.with(|s| s.on_key_ignore_len.set(s.on_key_ignore_len.get() + (3)));
    gotchars(&nop_buf);
}

fn ungetchars(len: i32, recorded: bool) {
    if !g_st().key_typed {
        return;
    }
    let len_u = len as usize;
    if recorded
        && g_st().reg_recording != 0
        && with_inp_ext(|ext| ext.recordbuff.borrow().len() >= len_u)
    {
        with_inp_ext(|ext| {
            let mut rb = ext.recordbuff.borrow_mut();
            let new_len = rb.len() - len_u;
            rb.truncate(new_len);
        });
        let rec_len = INP_STATE
            .with(|s| s.last_recorded_len.get())
            .wrapping_sub(len_u);
        INP_STATE.with(|s| s.last_recorded_len.set(rec_len));
    }
    atom_typed_del(len_u);
    let trim = with_inp_ext(|ext| {
        let mut okb = ext.on_key_buf.borrow_mut();
        let trim = len_u.min(okb.len());
        let new_len = okb.len() - trim;
        okb.truncate(new_len);
        trim
    });
    INP_STATE.with(|s| {
        s.on_key_ignore_len
            .set(s.on_key_ignore_len.get() + (len_u - trim))
    });
}

/// Sync undo when typed characters are obtained from the typeahead buffer.
pub fn may_sync_undo() {
    if ((g_st().state_global & (MODE_INSERT | MODE_CMDLINE)) == 0
        || g_st().ins_global.moved != K_INS_NONE)
        && INP_STATE.with(|s| s.curscript.get()) < 0
    {
        u_sync(false);
    }
}

fn alloc_typebuf() {
    g_st().typebuf.tb_buf = vec![0u8; TYPELEN_INIT];
    g_st().typebuf.tb_noremap = vec![0u8; TYPELEN_INIT];
    g_st().typebuf.tb_buflen = TYPELEN_INIT as i32;
    g_st().typebuf.tb_off = (MAXMAPLEN + 4) as i32;
    g_st().typebuf.tb_len = 0;
    g_st().typebuf.tb_maplen = 0;
    g_st().typebuf.tb_silent = 0;
    g_st().typebuf.tb_no_abbr_cnt = 0;
    g_st().typebuf.tb_change_cnt = g_st().typebuf.tb_change_cnt.wrapping_add(1);
    if g_st().typebuf.tb_change_cnt == 0 {
        g_st().typebuf.tb_change_cnt = 1;
    }
    g_st().typebuf_was_filled = false;
}

fn free_typebuf() {
    g_st().typebuf.tb_buf = Vec::new();
    g_st().typebuf.tb_noremap = Vec::new();
}

fn save_typebuf() {
    let cur_idx = INP_STATE.with(|s| s.curscript.get());
    assert!(cur_idx >= 0);
    init_typebuf();
    let prev_cnt = g_st().typebuf.tb_change_cnt;
    with_inp_ext(|ext| {
        ext.saved_typebuf.borrow_mut()[cur_idx as usize] =
            std::mem::replace(&mut g_st().typebuf, Typebuf::ZERO);
    });
    g_st().typebuf.tb_change_cnt = prev_cnt;
    alloc_typebuf();
}

fn can_get_ungot() -> bool {
    let ungot = INP_STATE.with(|s| s.ungot.get());
    ungot.c != -1 && (ungot.stuffed || stuff_empty())
}

/// Save all three kinds of typeahead, so that the user must type at a prompt.
pub fn save_typeahead(tp: &mut Tasave) {
    let prev_cnt = g_st().typebuf.tb_change_cnt;
    tp.save_typebuf = std::mem::replace(&mut g_st().typebuf, Typebuf::ZERO);
    g_st().typebuf.tb_change_cnt = prev_cnt;
    alloc_typebuf();
    tp.ungot = INP_STATE.with(|s| s.ungot.get());
    INP_STATE.with(|s| {
        let mut u = s.ungot.get();
        u.c = -1;
        s.ungot.set(u);
    });

    with_inp_ext(|ext| {
        tp.save_readbuf1 = std::mem::replace(&mut *ext.readbuf1.borrow_mut(), StuffBuf::new());
        tp.save_readbuf2 = std::mem::replace(&mut *ext.readbuf2.borrow_mut(), StuffBuf::new());
    });
}

/// Restore the typeahead to what it was before calling `save_typeahead()`.
pub fn restore_typeahead(tp: &mut Tasave) {
    free_typebuf();
    g_st().typebuf = std::mem::replace(&mut tp.save_typebuf, Typebuf::ZERO);
    INP_STATE.with(|s| s.ungot.set(tp.ungot));

    with_inp_ext(|ext| {
        *ext.readbuf1.borrow_mut() = std::mem::replace(&mut tp.save_readbuf1, StuffBuf::new());
        *ext.readbuf2.borrow_mut() = std::mem::replace(&mut tp.save_readbuf2, StuffBuf::new());
    });
}

/// Open a new script file for the `":source!"` command.
pub fn openscript(name: &CStr, directly: bool) {
    {
        if INP_STATE.with(|s| s.curscript.get()) + 1 == NSCRIPT as i32 {
            emsg(gettext(E_NESTING));
            return;
        }

        if check_secure() {
            return;
        }

        if g_st().ignore_script {
            return;
        }

        INP_STATE.with(|s| s.curscript.set(s.curscript.get() + (1)));
        expand_env(name, g_st().name_buff_bytes_mut());
        let cur_idx = INP_STATE.with(|s| s.curscript.get()) as usize;
        let error = with_inp_ext(|ext| {
            file_open(
                &mut ext.scriptin.borrow_mut()[cur_idx],
                Some(g_st().name_buff_cstr()),
                K_FILE_READ_ONLY,
                0,
            )
        });
        if error != 0 {
            semsg!(gettext(E_NOTOPEN_2), name, uv_strerror(error));
            INP_STATE.with(|s| s.curscript.set(s.curscript.get() - (1)));
            return;
        }
        save_typebuf();

        if directly {
            let mut oa = Oparg::new();
            let save_state = g_st().state_global;
            let save_restart_edit = g_st().restart_edit;
            let save_finish_op = g_st().finish_op;
            let save_msg_scroll = g_st().msg_scroll;

            g_st().state_global = MODE_NORMAL;
            g_st().msg_scroll = 0;
            g_st().restart_edit = 0;
            g_st().finish_op = false;

            let oldcurscript = INP_STATE.with(|s| s.curscript.get());
            loop {
                update_topline_cursor();
                normal_cmd(&mut oa, false);
                vpeekc();
                if INP_STATE.with(|s| s.curscript.get()) < oldcurscript {
                    break;
                }
            }

            g_st().state_global = save_state;
            g_st().msg_scroll = save_msg_scroll;
            g_st().restart_edit = save_restart_edit;
            g_st().finish_op = save_finish_op;
        }
    }
}

fn closescript() {
    let cur_idx = INP_STATE.with(|s| s.curscript.get());
    assert!(cur_idx >= 0);
    free_typebuf();
    with_inp_ext(|ext| {
        g_st().typebuf = std::mem::replace(
            &mut ext.saved_typebuf.borrow_mut()[cur_idx as usize],
            Typebuf::ZERO,
        );
        file_close(&mut ext.scriptin.borrow_mut()[cur_idx as usize], false);
    });
    INP_STATE.with(|s| s.curscript.set(s.curscript.get() - (1)));
}

pub fn close_all_scripts() {
    while INP_STATE.with(|s| s.curscript.get()) >= 0 {
        closescript();
    }
}

pub fn open_scriptin(scriptin_name: &CStr) -> bool {
    assert!(INP_STATE.with(|s| s.curscript.get()) == -1);
    INP_STATE.with(|s| s.curscript.set(s.curscript.get() + (1)));

    let error = with_inp_ext(|ext| {
        let mut scriptin = ext.scriptin.borrow_mut();
        if scriptin_name.to_bytes() == b"-" {
            file_open_stdin(&mut scriptin[0])
        } else {
            file_open(
                &mut scriptin[0],
                Some(scriptin_name),
                K_FILE_READ_ONLY | K_FILE_NON_BLOCKING,
                0,
            )
        }
    });
    if error != 0 {
        let mut errbuf = [0u8; 1024];
        let n = format_to_buf!(
            &mut errbuf,
            gettext(c"Cannot open for reading: \"%s\": %s\n"),
            scriptin_name,
            uv_strerror(error),
        );
        if n > 0 {
            let slice = &errbuf[..(n as usize).min(errbuf.len() - 1)];
            let _ = std::io::Write::write_all(&mut std::io::stderr(), slice);
        }
        INP_STATE.with(|s| s.curscript.set(s.curscript.get() - (1)));
        return false;
    }
    save_typebuf();

    true
}

/// Return true when reading keys from a script file.
pub fn using_script() -> i32 {
    i32::from(INP_STATE.with(|s| s.curscript.get()) >= 0)
}

/// Called just before doing a blocking wait.
pub fn before_blocking() {
    updatescript(0);
    if g_st().may_garbage_collect {
        garbage_collect(false);
    }
}

fn updatescript(c: i32) {
    if c != 0
        && let Some(ref so) = g_st().scriptout
    {
        so.putc(c);
    }
    let idle = c == 0;
    if idle
        || (opt_vars().p_uc > 0 && {
            INP_STATE.with(|s| s.updatescript_count.set(s.updatescript_count.get() + (1)));
            (INP_STATE.with(|s| s.updatescript_count.get()) as OptInt) >= opt_vars().p_uc
        })
    {
        ml_sync_all(i32::from(idle), 1, opt_vars().p_fs || idle);
        INP_STATE.with(|s| s.updatescript_count.set(0));
    }
}

/// Merge `modifiers` into `c_arg`.
pub fn merge_modifiers(c_arg: i32, modifiers: &mut i32) -> i32 {
    let mut c = c_arg;

    if (*modifiers & (MOD_MASK_CTRL as i32)) != 0 {
        if c >= b'@' as i32 && c <= 0x7f {
            c &= 0x1f;
            if c == NUL {
                c = K_ZERO;
            }
        } else if c == b'6' as i32 {
            c = 0x1e;
        }
        if c != c_arg {
            *modifiers &= !(MOD_MASK_CTRL as i32);
        }
    }
    c
}

fn add_byte_to_showcmd(byte: u8) {
    if !opt_vars().p_sc || g_st().msg_silent != 0 {
        return;
    }

    if !INP_STATE.with(|s| {
        let mut ss = s.showcmd_state.get();
        let ok = gotchars_add_byte(&mut ss, byte);
        s.showcmd_state.set(ss);
        ok
    }) {
        return;
    }

    let buflen = INP_STATE.with(|s| s.showcmd_state.get()).buflen;
    INP_STATE.with(|s| {
        let mut ss = s.showcmd_state.get();
        ss.buf[buflen] = NUL as u8;
        ss.buflen = 0;
        s.showcmd_state.set(ss);
    });

    let mut modifiers: i32 = 0;
    let mut c: i32 = NUL;

    let buf_copy = INP_STATE.with(|s| s.showcmd_state.get()).buf;
    let mut rem = &buf_copy[..buflen];
    if rem.len() >= 3 && rem[0] == K_SPECIAL && rem[1] == KS_MODIFIER && rem[2] != 0 {
        modifiers = i32::from(rem[2]);
        rem = &rem[3..];
    }

    if !rem.is_empty() {
        if let Some(mb_cstr) = mb_unescape(&mut rem) {
            c = utf_slice2char(mb_cstr.to_bytes());
        } else {
            c = i32::from(rem[0]);
            rem = &rem[1..];
        }
        if c <= 0x7f {
            let mut modifiers_after = modifiers;
            let mod_c = merge_modifiers(c, &mut modifiers_after);
            if modifiers_after == 0 {
                modifiers = 0;
                c = mod_c;
            }
        }
    }

    if modifiers != 0 {
        add_to_showcmd(K_SPECIAL as i32);
        add_to_showcmd(KS_MODIFIER as i32);
        add_to_showcmd(modifiers);
    }
    if c != NUL {
        add_to_showcmd(c);
    }
    for &b in rem {
        add_to_showcmd(i32::from(b));
    }
}

/// Get the next input character.
pub fn vgetc() -> i32 {
    let mut c: i32;
    let mut buf = [0u8; MB_MAXBYTES + 1];

    if g_st().may_garbage_collect && g_st().want_garbage_collect {
        garbage_collect(false);
    }

    if can_get_ungot() {
        c = INP_STATE.with(|s| s.ungot.get()).c;
        INP_STATE.with(|s| {
            let mut u = s.ungot.get();
            u.c = -1;
            s.ungot.set(u);
        });
        g_st().mod_mask = INP_STATE.with(|s| s.ungot.get()).mod_mask;
        g_st().mouse_grid = INP_STATE.with(|s| s.ungot.get()).mouse_grid;
        g_st().mouse_row = INP_STATE.with(|s| s.ungot.get()).mouse_row;
        g_st().mouse_col = INP_STATE.with(|s| s.ungot.get()).mouse_col;
    } else {
        g_st().mod_mask = 0;
        g_st().vgetc_mod_mask = 0;
        g_st().vgetc_char = 0;

        let rec_len = INP_STATE.with(|st| {
            st.last_recorded_len
                .get()
                .wrapping_sub(st.last_vgetc_recorded_len.get())
        });
        INP_STATE.with(|s| s.last_recorded_len.set(rec_len));

        loop {
            let mut did_inc = false;
            if g_st().mod_mask != 0 {
                g_st().no_mapping += 1;
                g_st().allow_keys += 1;
                did_inc = true;
            }
            c = vgetorpeek(true);
            if did_inc {
                g_st().no_mapping -= 1;
                g_st().allow_keys -= 1;
            }

            if c == K_SPECIAL as i32 {
                let save_allow_keys = g_st().allow_keys;
                g_st().no_mapping += 1;
                g_st().allow_keys = 0;
                let c2 = vgetorpeek(true);
                c = vgetorpeek(true);
                g_st().no_mapping -= 1;
                g_st().allow_keys = save_allow_keys;
                if c2 == KS_MODIFIER as i32 {
                    g_st().mod_mask = c;
                    continue;
                }
                c = to_special(c2, c);
            }

            let n = mb_byte2len_check(c);
            if n > 1 {
                g_st().no_mapping += 1;
                buf[0] = c as u8;
                for slot in buf.iter_mut().take(n as usize).skip(1) {
                    *slot = vgetorpeek(true) as u8;
                    if *slot == K_SPECIAL {
                        vgetorpeek(true);
                        vgetorpeek(true);
                    }
                }
                g_st().no_mapping -= 1;
                c = utf_ptr2char(&buf);
            }

            if g_st().no_mapping == 0
                && g_st().key_typed
                && g_st().mod_mask == MOD_MASK_ALT as i32
                && (g_st().state_global & MODE_TERMINAL) == 0
                && !is_mouse_key(c)
            {
                g_st().mod_mask = 0;
                requeue_key(c, 0, 3, true);
                ins_typebuf(
                    ESC_STR,
                    INP_STATE.with(|s| s.key_noremap.get()),
                    0,
                    !g_st().key_typed,
                    g_st().cmd_silent,
                );
                continue;
            }

            if g_st().vgetc_char == 0 {
                g_st().vgetc_mod_mask = g_st().mod_mask;
                g_st().vgetc_char = c;
            }

            match c {
                K_KPLUS => c = b'+' as i32,
                K_KMINUS => c = b'-' as i32,
                K_KDIVIDE => c = b'/' as i32,
                K_KMULTIPLY => c = b'*' as i32,
                K_KENTER => c = CAR,
                K_KPOINT => c = b'.' as i32,
                K_KCOMMA => c = b',' as i32,
                K_KEQUAL => c = b'=' as i32,
                K_K0 => c = b'0' as i32,
                K_K1 => c = b'1' as i32,
                K_K2 => c = b'2' as i32,
                K_K3 => c = b'3' as i32,
                K_K4 => c = b'4' as i32,
                K_K5 => c = b'5' as i32,
                K_K6 => c = b'6' as i32,
                K_K7 => c = b'7' as i32,
                K_K8 => c = b'8' as i32,
                K_K9 => c = b'9' as i32,
                K_XHOME | K_ZHOME => {
                    if g_st().mod_mask == MOD_MASK_SHIFT as i32 {
                        c = K_S_HOME;
                        g_st().mod_mask = 0;
                    } else if g_st().mod_mask == MOD_MASK_CTRL as i32 {
                        c = K_C_HOME;
                        g_st().mod_mask = 0;
                    } else {
                        c = K_HOME;
                    }
                }
                K_XEND | K_ZEND => {
                    if g_st().mod_mask == MOD_MASK_SHIFT as i32 {
                        c = K_S_END;
                        g_st().mod_mask = 0;
                    } else if g_st().mod_mask == MOD_MASK_CTRL as i32 {
                        c = K_C_END;
                        g_st().mod_mask = 0;
                    } else {
                        c = K_END;
                    }
                }
                K_KUP | K_XUP => c = K_UP,
                K_KDOWN | K_XDOWN => c = K_DOWN,
                K_KLEFT | K_XLEFT => c = K_LEFT,
                K_KRIGHT | K_XRIGHT => c = K_RIGHT,
                _ => {}
            }

            break;
        }

        let rec_len = INP_STATE.with(|s| s.last_recorded_len.get());
        INP_STATE.with(|s| s.last_vgetc_recorded_len.set(rec_len));
    }

    g_st().may_garbage_collect = false;

    let mut on_key_buf = with_inp_ext(|ext| {
        let mut b = std::mem::take(&mut *ext.on_key_buf.borrow_mut());
        b.push(0);
        b
    });
    if INP_STATE.with(|s| s.on_key_blocked.get()) == 0 && nlua_exec_on_key(c, &mut on_key_buf) {
        if c == K_COMMAND {
            let _ = getcmdkeycmd(NUL, 0, false);
        } else if c == K_LUA {
            map_execute_lua(false, true);
        } else if c == K_PASTE_START {
            paste_repeat(0);
        }
        c = K_IGNORE;
    }

    if c != K_IGNORE {
        state_no_longer_safe(Some(c"key typed"));
    }

    c
}

/// Like `vgetc()`, but never return a NUL when called recursively.
pub fn safe_vgetc() -> i32 {
    let mut c = vgetc();
    if c == NUL {
        c = get_keystroke(None);
    }
    c
}

/// Like `safe_vgetc()`, but loop to handle `K_IGNORE`.
pub fn plain_vgetc() -> i32 {
    loop {
        let c = safe_vgetc();
        if c != K_IGNORE && c != K_VER_SCROLLBAR && c != K_HOR_SCROLLBAR && c != K_MOUSEMOVE {
            return c;
        }
    }
}

/// Check if a character is available, such that `vgetc()` will not block.
pub fn vpeekc() -> i32 {
    if can_get_ungot() {
        return INP_STATE.with(|s| s.ungot.get()).c;
    }
    vgetorpeek(false)
}

/// Check if any character is available, also half an escape sequence.
pub fn vpeekc_any() -> i32 {
    let mut c = vpeekc();
    if c == NUL && g_st().typebuf.tb_len > 0 {
        c = ESC;
    }
    c
}

/// Call `vpeekc()` without causing anything to be mapped.
pub fn char_avail() -> bool {
    if test_disable_char_avail.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    g_st().no_mapping += 1;
    let retval = vpeekc();
    g_st().no_mapping -= 1;
    retval != NUL
}

fn getchar_common(argvars: &mut [TypVal], rettv: &mut TypVal, mut allow_number: bool) {
    let mut n: Varnumber;
    let called_emsg_start = g_st().called_emsg;
    let mut error = false;
    let mut simplify = true;
    let mut cursor_flag: u8 = 0;

    if argvars[0].v_type != VarType::VarUnknown && !tv_check_for_opt_dict_arg(argvars, 1) {
        return;
    }

    if argvars[0].v_type != VarType::VarUnknown && argvars[1].v_type == VarType::VarDict {
        let d = argvars[1].vval.v_dict_opt();

        if allow_number {
            allow_number = tv_dict_get_bool(d.as_deref(), c"number", 1) != 0;
        } else if tv_dict_has_key(d.as_deref(), c"number") {
            semsg!(gettext(E_INVARG2), c"number");
        }

        simplify = tv_dict_get_bool(d.as_deref(), c"simplify", 1) != 0;

        if let Some(cursor_str) = tv_dict_get_string(d.as_deref(), c"cursor") {
            if cursor_str != c"hide" && cursor_str != c"keep" && cursor_str != c"msg" {
                semsg!(gettext(E_INVARG_NVAL), c"cursor", cursor_str,);
            } else {
                cursor_flag = cursor_str.to_bytes()[0];
            }
        }
    }

    if g_st().called_emsg != called_emsg_start {
        return;
    }

    if cursor_flag == b'h' {
        ui_busy_start();
    }

    g_st().no_mapping += 1;
    g_st().allow_keys += 1;
    block_on_key();
    if !simplify {
        INP_STATE.with(|s| s.no_reduce_keys.set(s.no_reduce_keys.get() + (1)));
    }
    atom_payload_start();
    loop {
        if cursor_flag == b'm' || (cursor_flag == 0 && g_st().msg_col > 0) {
            ui_cursor_goto(g_st().msg_row, g_st().msg_col);
        }

        if argvars[0].v_type == VarType::VarUnknown
            || (argvars[0].v_type == VarType::VarNumber && argvars[0].vval.v_number() == -1)
        {
            if !char_avail() {
                ui_flush();
                let events = main_loop_events();
                input_get(None, -1, g_st().typebuf.tb_change_cnt, Some(events));
                if input_available() == 0 && !multiqueue_empty(main_loop_events()) {
                    state_handle_k_event();
                    continue;
                }
            }
            n = Varnumber::from(safe_vgetc());
        } else if tv_get_bool_chk(&argvars[0], Some(&mut error)) != 0 {
            n = Varnumber::from(vpeekc_any());
        } else if error || vpeekc_any() == NUL {
            n = 0;
        } else {
            n = Varnumber::from(safe_vgetc());
        }

        let ni = n as i32;
        if ni == K_IGNORE || ni == K_MOUSEMOVE || ni == K_VER_SCROLLBAR || ni == K_HOR_SCROLLBAR {
            continue;
        }
        break;
    }
    atom_payload_end();
    g_st().no_mapping -= 1;
    g_st().allow_keys -= 1;
    unblock_on_key();
    if !simplify {
        INP_STATE.with(|s| s.no_reduce_keys.set(s.no_reduce_keys.get() - (1)));
    }

    if cursor_flag == b'h' {
        ui_busy_stop();
    }

    set_vim_var_nr(VV_MOUSE_WIN, 0);
    set_vim_var_nr(VV_MOUSE_WINID, 0);
    set_vim_var_nr(VV_MOUSE_LNUM, 0);
    set_vim_var_nr(VV_MOUSE_COL, 0);

    let ni = n as i32;
    if n != 0 && (!allow_number || is_special(ni) || g_st().mod_mask != 0) {
        let mut temp = [0u8; 10];
        let mut i: usize = 0;

        if g_st().mod_mask != 0 {
            temp[i] = K_SPECIAL;
            temp[i + 1] = KS_MODIFIER;
            temp[i + 2] = g_st().mod_mask as u8;
            i += 3;
        }
        if is_special(ni) {
            temp[i] = K_SPECIAL;
            temp[i + 1] = k_second(ni);
            temp[i + 2] = k_third(ni);
            i += 3;
        } else {
            i += utf_char2bytes(ni, &mut temp[i..]) as usize;
        }
        assert!(i < 10);
        crate::eval::typval::tv_set_bytes(&mut *rettv, &temp[..i]);

        if is_mouse_key(ni) {
            let mut row = g_st().mouse_row;
            let mut col = g_st().mouse_col;
            let mut grid = g_st().mouse_grid;
            let mut lnum: Linenr = 0;

            if row >= 0 && col >= 0 {
                let mut winnr: i32 = 1;
                let Some(win) = mouse_find_win_inner(&mut grid, &mut row, &mut col) else {
                    return;
                };
                let win_ptr = ptr::from_mut(win);

                mouse_comp_pos(win, &mut row, &mut col, &mut lnum);

                let mut wp = g_st().firstwin;
                while wp != win_ptr {
                    winnr += 1;

                    wp = wp.w_next;
                }
                set_vim_var_nr(VV_MOUSE_WIN, Varnumber::from(winnr));

                set_vim_var_nr(VV_MOUSE_WINID, Varnumber::from(wp.handle));

                set_vim_var_nr(VV_MOUSE_LNUM, Varnumber::from(lnum));
                set_vim_var_nr(VV_MOUSE_COL, Varnumber::from(col + 1));
            }
        }
    } else if !allow_number {
        rettv.v_type = VarType::VarString;
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Number(n);
    }
}

/// `"getchar()"` function.
pub fn f_getchar(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getchar_common(argvars, rettv, true);
}

/// `"getcharstr()"` function.
pub fn f_getcharstr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getchar_common(argvars, rettv, false);
}

/// `"getcharmod()"` function.
pub fn f_getcharmod(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(g_st().mod_mask));
}

fn put_string_in_typebuf(offset: i32, slen: i32, string: &mut [u8], new_slen: i32) -> bool {
    let extra = new_slen - slen;
    string[new_slen as usize] = NUL as u8;
    if extra < 0 {
        del_typebuf(-extra, offset);
    } else if extra > 0
        && !ins_typebuf(
            CStr::from_bytes_until_nul(&string[slen as usize..]).unwrap_or_default(),
            REMAP_YES,
            offset,
            false,
            false,
        )
    {
        return false;
    }
    let dst = (g_st().typebuf.tb_off + offset) as usize;
    let new_usize = new_slen as usize;
    g_st().typebuf.tb_buf[dst..dst + new_usize].copy_from_slice(&string[..new_usize]);
    true
}

fn at_ins_compl_key() -> bool {
    let off = g_st().typebuf.tb_off as usize;
    let tb_buf = &g_st().typebuf.tb_buf[off..];
    let mut c = i32::from(tb_buf[0]);

    if g_st().typebuf.tb_len > 3
        && c == K_SPECIAL as i32
        && tb_buf[1] == KS_MODIFIER
        && (tb_buf[2] & (MOD_MASK_CTRL as u8)) != 0
    {
        c = i32::from(tb_buf[3] & 0x1f);
    }
    !ctrl_x_mode_eval()
        && ((ctrl_x_mode_not_default() && vim_is_ctrl_x_key(c))
            || (compl_status_local() && (c == CTRL_N || c == CTRL_P)))
}

fn check_simplify_modifier(max_offset: i32) -> i32 {
    if (g_st().state_global & MODE_TERMINAL) != 0 || INP_STATE.with(|s| s.no_reduce_keys.get()) > 0
    {
        return 0;
    }

    for offset in 0..max_offset {
        if offset + 3 >= g_st().typebuf.tb_len {
            break;
        }
        let tp_idx = (g_st().typebuf.tb_off + offset) as usize;
        if g_st().typebuf.tb_buf[tp_idx] == K_SPECIAL
            && g_st().typebuf.tb_buf[tp_idx + 1] == KS_MODIFIER
        {
            let orig_mod = i32::from(g_st().typebuf.tb_buf[tp_idx + 2]);
            let mut modifier = orig_mod;
            let c = i32::from(g_st().typebuf.tb_buf[tp_idx + 3]);
            let new_c = merge_modifiers(c, &mut modifier);

            if new_c != c {
                if offset == 0 {
                    g_st().vgetc_char = c;
                    g_st().vgetc_mod_mask = orig_mod;
                }
                let mut new_string = [0u8; MB_MAXBYTES + 1];
                let len = if is_special(new_c) {
                    new_string[0] = K_SPECIAL;
                    new_string[1] = k_second(new_c);
                    new_string[2] = k_third(new_c);
                    3
                } else {
                    utf_char2bytes(new_c, &mut new_string)
                };
                if modifier == 0 {
                    if !put_string_in_typebuf(offset, 4, &mut new_string, len) {
                        return -1;
                    }
                } else {
                    g_st().typebuf.tb_buf[tp_idx + 2] = modifier as u8;
                    if !put_string_in_typebuf(offset + 3, 1, &mut new_string, len) {
                        return -1;
                    }
                }
                return len;
            }
        }
    }
    0
}

fn char_iter(itp: &mut &[u8], nomap: i32) -> i32 {
    let mut it = *itp;
    let mut c = i32::from(it.first().copied().unwrap_or(0));
    if c == NUL {
        return 0;
    }

    if nomap == 0 {
        let mblen = utf_byte2len(c);
        if mblen > 1 {
            let mut buf = [0u8; MB_MAXBYTES + 1];
            buf[0] = c as u8;
            let mut i: i32 = 1;

            loop {
                it = &it[1.min(it.len())..];
                buf[i as usize] = it.first().copied().unwrap_or(0);
                if buf[i as usize] == K_SPECIAL {
                    it = &it[1.min(it.len())..];
                    if it.first().copied().unwrap_or(0) == 0 {
                        break;
                    }
                    it = &it[1.min(it.len())..];
                    if it.first().copied().unwrap_or(0) == 0 {
                        break;
                    }
                }
                if buf[i as usize] == NUL as u8 {
                    break;
                }
                i += 1;
                if i >= mblen {
                    break;
                }
            }

            if i != mblen {
                *itp = it;
                return 0;
            }

            c = utf_ptr2char(&buf);
            if c == buf[0] as i32 {
                it = *itp;
            }
        }
    }

    *itp = &it[1.min(it.len())..];
    c
}

fn handle_mapping(keylenp: &mut i32, timedout: &bool, mapdepth: &mut i32, advance: bool) -> i32 {
    let mut mp: Option<MatchedMap> = None;
    let mut mp_match: Option<MatchedMap> = None;
    let mut mp_match_len: i32 = 0;
    let mut tb_match_len: i32 = 0;
    let mut max_mlen: i32 = 0;
    let mut keylen = *keylenp;
    let local_state = get_real_state();
    let mut is_plug_map = false;

    let tb_off = g_st().typebuf.tb_off as usize;
    let tb_len_u = g_st().typebuf.tb_len.max(0) as usize;
    if g_st().typebuf.tb_len >= 3
        && g_st().typebuf.tb_buf[tb_off..tb_off + 3] == [K_SPECIAL, KS_EXTRA, KE_PLUG]
    {
        is_plug_map = true;
    }

    let mut tb_b1 = i32::from(g_st().typebuf.tb_buf[tb_off]);
    if g_st().no_mapping == 0
        && (g_st().no_zero_mapping == 0 || tb_b1 != b'0' as i32)
        && (g_st().typebuf.tb_maplen == 0
            || is_plug_map
            || (g_st().typebuf.tb_noremap[tb_off] & (RM_NONE | RM_ABBR)) == 0)
        && !(opt_vars().p_paste && (g_st().state_global & (MODE_INSERT | MODE_CMDLINE)) != 0)
        && !(g_st().state_global == MODE_HITRETURN && (tb_b1 == CAR || tb_b1 == b' ' as i32))
        && g_st().state_global != MODE_ASKMORE
        && !at_ins_compl_key()
    {
        let nolmaplen: i32;
        let mut tb_c1 = tb_b1;
        let tb_full_slice = &g_st().typebuf.tb_buf[tb_off..tb_off + tb_len_u];
        let mut tb_iter: &[u8] = tb_full_slice;

        if tb_b1 == K_SPECIAL as i32 {
            nolmaplen = 2;
            tb_iter = &tb_iter[1.min(tb_iter.len())..];
        } else {
            tb_c1 = char_iter(&mut tb_iter, 0);
            if tb_c1 == 0 {
                if *timedout {
                    *keylenp = 0;
                    return MapResult::MapResultGet as i32;
                }
                *keylenp = KEYLEN_PART_KEY;
                return MapResult::MapResultNomatch as i32;
            }

            let orig_tb_c1 = tb_c1;
            langmap_adjust(
                &mut tb_c1,
                (g_st().state_global & (MODE_CMDLINE | MODE_INSERT)) == 0
                    && get_real_state() != MODE_SELECT,
            );
            if orig_tb_c1 != tb_c1 {
                let mut cbuf = [0u8; 6];
                utf_char2bytes(tb_c1, &mut cbuf);
                tb_b1 = i32::from(cbuf[0]);
            }
            nolmaplen = 0;
        }

        mp = None;
        with_maphash_list(local_state, tb_b1, |global_list| {
            for cand in get_buf_maphash_list(local_state, tb_b1)
                .iter()
                .chain(global_list)
            {
                let m_keys_slice = cand.m_keys.as_bytes();
                let mut key_iter: &[u8] = m_keys_slice;

                if char_iter(&mut key_iter, 0) == tb_c1
                    && (cand.m_mode & local_state) != 0
                    && ((cand.m_mode & MODE_LANGMAP) == 0 || g_st().typebuf.tb_maplen == 0)
                {
                    let mut nomap = nolmaplen;
                    let mut modifiers: i32 = 0;
                    let mut tmp_tb_iter = tb_iter;
                    let mut mlen = (m_keys_slice.len() - key_iter.len()) as i32;
                    let mut tb_mlen = (tb_full_slice.len() - tmp_tb_iter.len()) as i32;

                    loop {
                        let mut c2 = char_iter(&mut tmp_tb_iter, nomap);
                        if c2 == 0 {
                            break;
                        }
                        let tb_nomap = nomap;
                        if nomap > 0 {
                            if nomap == 2 && c2 == KS_MODIFIER as i32 {
                                modifiers = 1;
                            } else if nomap == 1 && modifiers == 1 {
                                modifiers = c2;
                            }
                            nomap -= 1;
                        } else {
                            if c2 == K_SPECIAL as i32 {
                                nomap = 2;
                            } else if merge_modifiers(c2, &mut modifiers) == c2 {
                                langmap_adjust(&mut c2, true);
                            }
                            modifiers = 0;
                        }

                        let key_char = char_iter(&mut key_iter, tb_nomap);
                        if key_char != c2 {
                            break;
                        }

                        mlen = (m_keys_slice.len() - key_iter.len()) as i32;
                        tb_mlen = (tb_full_slice.len() - tmp_tb_iter.len()) as i32;
                    }

                    keylen = cand.m_keylen;
                    if mlen == keylen
                        || (mlen == g_st().typebuf.tb_len && g_st().typebuf.tb_len < keylen)
                    {
                        let noremap_slice =
                            &g_st().typebuf.tb_noremap[tb_off..tb_off + mlen as usize];
                        let m_keys_bytes = cand.m_keys.as_bytes();
                        if noremap_slice[0] == RM_SCRIPT
                            && !m_keys_bytes.starts_with(&[K_SPECIAL, KS_EXTRA, KE_SNR])
                        {
                            continue;
                        }

                        if !is_plug_map
                            && noremap_slice
                                .iter()
                                .any(|&b| (b & (RM_NONE | RM_ABBR)) != 0)
                        {
                            continue;
                        }

                        if keylen > g_st().typebuf.tb_len {
                            if !*timedout && !mp_match.as_ref().is_some_and(|m| m.m_nowait != 0) {
                                keylen = KEYLEN_PART_MAP;
                                mp = Some(cand.to_matched());
                                break;
                            }
                        } else if keylen > mp_match_len
                            || (keylen == mp_match_len
                                && mp_match
                                    .as_ref()
                                    .is_some_and(|m| (m.m_mode & MODE_LANGMAP) == 0)
                                && (cand.m_mode & MODE_LANGMAP) != 0)
                        {
                            mp_match = Some(cand.to_matched());
                            mp_match_len = keylen;
                            tb_match_len = tb_mlen;
                        }
                    } else {
                        max_mlen = max_mlen.max(mlen);
                    }
                }
            }
        });

        if keylen != KEYLEN_PART_MAP && mp_match.is_some() {
            mp = mp_match;
            keylen = tb_match_len;
        }
    }

    if (mp.is_none() || max_mlen > mp_match_len) && keylen != KEYLEN_PART_MAP {
        if g_st().no_mapping == 0 || g_st().allow_keys != 0 {
            if tb_b1 == K_SPECIAL as i32
                && (g_st().typebuf.tb_len < 2
                    || (g_st().typebuf.tb_buf[tb_off + 1] == KS_MODIFIER
                        && g_st().typebuf.tb_len < 4))
            {
                keylen = KEYLEN_PART_KEY;
            } else {
                keylen = check_simplify_modifier(max_mlen + 1);
                if keylen < 0 {
                    return MapResult::MapResultFail as i32;
                }
            }
        } else {
            keylen = 0;
        }
        if keylen == 0 && mp.is_none() {
            *keylenp = keylen;
            return MapResult::MapResultGet as i32;
        }
        if keylen > 0 {
            *keylenp = keylen;
            return MapResult::MapResultRetry as i32;
        }
        if keylen < 0 {
            assert_eq!(keylen, KEYLEN_PART_KEY);
        } else {
            assert!(mp.is_some());
            keylen = tb_match_len;
        }
    }

    if keylen >= 0 && keylen <= g_st().typebuf.tb_len {
        let mp = mp.as_ref().unwrap();
        if keylen > g_st().typebuf.tb_maplen && (mp.m_mode & MODE_LANGMAP) == 0 {
            let start = (g_st().typebuf.tb_off + g_st().typebuf.tb_maplen) as usize;
            let count = (keylen - g_st().typebuf.tb_maplen) as usize;
            let chunk = g_st().typebuf.tb_buf[start..start + count].to_vec();
            gotchars(&chunk);
            atom_map_start(&mp.m_keys.as_bytes()[..mp.m_keylen as usize], !advance);
        }

        g_st().cmd_silent = g_st().typebuf.tb_silent > 0;
        del_typebuf(keylen, 0);

        *mapdepth += 1;
        if (*mapdepth as OptInt) >= opt_vars().p_mmd {
            emsg(gettext(E_RECURSIVE_MAPPING));
            if (g_st().state_global & MODE_CMDLINE) != 0 {
                redrawcmdline();
            } else {
                setcursor();
            }
            flush_buffers(FLUSH_MINIMAL);
            *mapdepth = 0;
            *keylenp = keylen;
            return MapResult::MapResultFail as i32;
        }

        if g_st().visual_global.active
            && g_st().visual_global.select
            && (mp.m_mode & MODE_VISUAL) != 0
        {
            g_st().visual_global.select = false;
            ins_typebuf(
                CStr::from_bytes_with_nul(K_SELECT_STRING).unwrap(),
                REMAP_NONE,
                0,
                true,
                false,
            );
        }

        let save_m_expr = mp.m_expr != 0;
        let save_m_noremap = mp.m_noremap;
        let save_m_silent = mp.m_silent != 0;
        let save_m_mode = mp.m_mode;
        let save_m_keys = mp.m_keys.clone();
        let save_alt_m_keys = mp.m_alt_keys.clone();
        let save_alt_m_keylen = save_alt_m_keys
            .as_ref()
            .map_or(0, |k| k.as_bytes().len() as i32);

        let owned_map_str: Option<std::ffi::CString> = if save_m_expr {
            let save_vgetc_busy = g_st().vgetc_busy;
            let save_may_garbage_collect = g_st().may_garbage_collect;
            let prev_did_emsg = g_st().did_emsg;

            g_st().vgetc_busy = 0;
            g_st().may_garbage_collect = false;

            let mut res = eval_map_expr(mp, NUL);

            if res.as_ref().is_none_or(|s| s.to_bytes().is_empty()) {
                if prev_did_emsg != g_st().did_emsg {
                    res = Some(std::ffi::CString::new([K_SPECIAL, KS_EXTRA, KE_IGNORE]).unwrap());
                    if (g_st().state_global & MODE_CMDLINE) != 0 {
                        g_st().msg_didout = true;
                        g_st().msg_row = g_st().msg_row.max(g_st().cmdline_row);
                        redrawcmd();
                    }
                } else if (g_st().state_global & (MODE_NORMAL | MODE_INSERT)) != 0 {
                    setcursor();
                }
            }

            g_st().vgetc_busy = save_vgetc_busy;
            g_st().may_garbage_collect = save_may_garbage_collect;
            res
        } else {
            Some(mp.m_str.clone())
        };

        let i = if let Some(ref map_cstr) = owned_map_str {
            let map_b = map_cstr.as_bytes();
            if keylen > g_st().typebuf.tb_maplen && (save_m_mode & MODE_LANGMAP) != 0 {
                gotchars(map_b);
            }

            let k_b = save_m_keys.as_bytes();
            let k_len = keylen as usize;
            let noremap = if save_m_noremap != REMAP_YES {
                save_m_noremap
            } else if map_b[..k_len.min(map_b.len())] == k_b[..k_len.min(k_b.len())]
                || save_alt_m_keys.as_ref().is_some_and(|alt| {
                    let alt_b = alt.as_bytes();
                    let ak_len = save_alt_m_keylen as usize;
                    map_b[..ak_len.min(map_b.len())] == alt_b[..ak_len.min(alt_b.len())]
                })
            {
                REMAP_SKIP
            } else {
                REMAP_YES
            };

            ins_typebuf(
                map_cstr.as_c_str(),
                noremap,
                0,
                true,
                g_st().cmd_silent || save_m_silent,
            )
        } else {
            false
        };
        drop(owned_map_str);
        drop(save_m_keys);
        drop(save_alt_m_keys);
        *keylenp = keylen;
        if !i {
            return MapResult::MapResultFail as i32;
        }
        return MapResult::MapResultRetry as i32;
    }

    *keylenp = keylen;
    MapResult::MapResultNomatch as i32
}

/// Unget one character (can only be done once!).
pub fn vungetc(c: i32) {
    INP_STATE.with(|s| {
        s.ungot.set(UngotKey {
            c,
            mod_mask: g_st().mod_mask,
            mouse_grid: g_st().mouse_grid,
            mouse_row: g_st().mouse_row,
            mouse_col: g_st().mouse_col,
            stuffed: g_st().key_stuffed != 0,
        })
    });
}

/// When peeking and not getting a character, `reg_executing` cannot be cleared yet.
pub fn check_end_reg_executing(advance: bool) {
    if g_st().reg_executing != 0
        && (g_st().typebuf.tb_maplen == 0 || g_st().pending_end_reg_executing)
    {
        if advance {
            g_st().reg_executing = 0;
            g_st().pending_end_reg_executing = false;
        } else {
            g_st().pending_end_reg_executing = true;
        }
    }
}

fn vgetorpeek(advance: bool) -> i32 {
    let mut c: i32;
    let mut timedout = false;
    let mut mapdepth: i32 = 0;
    let mut mode_deleted = false;

    if g_st().vgetc_busy > 0 && g_st().ex_normal_busy == 0 {
        return NUL;
    }

    g_st().vgetc_busy += 1;

    if advance {
        g_st().key_stuffed = 0;
        g_st().typebuf_was_empty = false;
    }

    init_typebuf();
    start_stuff();
    check_end_reg_executing(advance);
    loop {
        if INP_STATE.with(|s| s.typeahead_char.get()) != 0 {
            c = INP_STATE.with(|s| s.typeahead_char.get());
            if advance {
                INP_STATE.with(|s| s.typeahead_char.set(0));
            }
        } else {
            c = read_readbuffers(advance);
        }
        if c != NUL && !g_st().got_int {
            if advance {
                g_st().key_stuffed = 1;
            }
            if g_st().typebuf.tb_no_abbr_cnt == 0 {
                g_st().typebuf.tb_no_abbr_cnt = 1;
            }
        } else {
            loop {
                check_end_reg_executing(advance);
                if g_st().typebuf.tb_maplen != 0 {
                    line_breakcheck();
                } else {
                    if ((g_st().mapped_ctrl_c | curbuf_mut().b_mapped_ctrl_c) & get_real_state())
                        != 0
                    {
                        g_st().ctrl_c_interrupts = false;
                    }
                    os_breakcheck();
                    g_st().ctrl_c_interrupts = true;
                }
                let mut keylen: i32 = 0;
                if g_st().got_int {
                    c = inchar(0, g_st().typebuf.tb_buflen - 1, 0);
                    if (c != 0 || g_st().typebuf.tb_maplen != 0)
                        && (g_st().state_global & (MODE_INSERT | MODE_CMDLINE)) != 0
                    {
                        c = ESC;
                    } else {
                        c = CTRL_C_CONST;
                    }
                    flush_buffers(FLUSH_INPUT);

                    if advance {
                        g_st().typebuf.tb_buf[0] = c as u8;
                        gotchars(&[c as u8]);
                    }
                    g_st().cmd_silent = false;

                    break;
                } else if g_st().typebuf.tb_len > 0 {
                    let result = handle_mapping(&mut keylen, &timedout, &mut mapdepth, advance);

                    if result == MapResult::MapResultRetry as i32 {
                        continue;
                    }

                    if result == MapResult::MapResultFail as i32 {
                        atom_composite_abort();
                        c = -1;
                        break;
                    }

                    if result == MapResult::MapResultGet as i32 {
                        let tb_off = g_st().typebuf.tb_off as usize;
                        c = i32::from(g_st().typebuf.tb_buf[tb_off]);
                        if advance {
                            g_st().cmd_silent = g_st().typebuf.tb_silent > 0;
                            if g_st().typebuf.tb_maplen > 0 {
                                g_st().key_typed = false;
                            } else {
                                g_st().key_typed = true;
                                gotchars(&[c as u8]);
                            }
                            INP_STATE.with(|s| {
                                s.key_noremap
                                    .set(i32::from(g_st().typebuf.tb_noremap[tb_off]))
                            });
                            del_typebuf(1, 0);
                        }
                        break;
                    }
                }

                c = 0;
                let mut new_wcol = curwin_mut().w_wcol;
                let mut new_wrow = curwin_mut().w_wrow;
                if advance
                    && g_st().typebuf.tb_len == 1
                    && g_st().typebuf.tb_buf[g_st().typebuf.tb_off as usize] == ESC as u8
                    && g_st().no_mapping == 0
                    && g_st().ex_normal_busy == 0
                    && g_st().typebuf.tb_maplen == 0
                    && (g_st().state_global & MODE_INSERT) != 0
                    && (opt_vars().p_timeout
                        || (keylen == KEYLEN_PART_KEY && opt_vars().p_ttimeout))
                    && {
                        c = inchar(
                            (g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize,
                            3,
                            25,
                        );
                        c == 0
                    }
                {
                    if g_st().mode_displayed {
                        unshowmode(true);
                        mode_deleted = true;
                    }
                    validate_cursor(curwin_mut());
                    let old_wcol = curwin_mut().w_wcol;
                    let old_wrow = curwin_mut().w_wrow;

                    if curwin_mut().w_cursor.col != 0 {
                        let mut col: Colnr = 0;
                        if curwin_mut().w_wcol > 0 {
                            if g_st().ins_global.did_ai
                                && crate::charset::skipwhite_slice(
                                    crate::cursor::get_cursor_pos_slice(),
                                )
                                .is_empty()
                            {
                                curwin_mut().w_wcol = 0;
                                let ptr_cstr = get_cursor_line_ptr();
                                let end_off = curwin_mut().w_cursor.col as usize;

                                let mut csarg = CharsizeArg::new();
                                let cstype: CSType = init_charsize_arg(
                                    &mut csarg,
                                    curwin_mut(),
                                    curwin_mut().w_cursor.lnum,
                                    ptr_cstr,
                                );
                                let mut ci: StrCharInfo = utf_ptr2_str_char_info(ptr_cstr);
                                let mut vcol: i32 = 0;
                                while ci.offset_from(ptr_cstr) < end_off {
                                    if !ascii_iswhite(ci.chr.value) {
                                        curwin_mut().w_wcol = vcol;
                                    }
                                    let cli: ClusterInfo = utf_cluster_info(ci);
                                    vcol += win_charsize(
                                        cstype,
                                        vcol,
                                        ci.cstr(),
                                        ci.chr.value,
                                        &mut csarg,
                                        cli.cells,
                                    )
                                    .width;
                                    ci = cli.next;
                                }

                                curwin_mut().w_wrow = curwin_mut().w_cline_row
                                    + curwin_mut().w_wcol / curwin_mut().w_view_width;
                                curwin_mut().w_wcol %= curwin_mut().w_view_width;
                                curwin_mut().w_wcol += win_col_off(curwin_mut());
                                col = 0;
                            } else {
                                curwin_mut().w_wcol -= 1;
                                col = curwin_mut().w_cursor.col - 1;
                            }
                        } else if curwin_mut().w_onebuf_opt.wo_wrap && curwin_mut().w_wrow != 0 {
                            curwin_mut().w_wrow -= 1;
                            curwin_mut().w_wcol = curwin_mut().w_view_width - 1;
                            col = curwin_mut().w_cursor.col - 1;
                        }
                        if col > 0 && curwin_mut().w_wcol > 0 {
                            let line_slice = crate::cursor::get_cursor_line_slice();
                            col -= crate::mbyte::utf_head_off_slice(line_slice, col as usize);
                            if utf_ptr2cells(&line_slice[(col as usize).min(line_slice.len())..])
                                > 1
                            {
                                curwin_mut().w_wcol -= 1;
                            }
                        }
                    }
                    setcursor();
                    ui_flush();
                    new_wcol = curwin_mut().w_wcol;
                    new_wrow = curwin_mut().w_wrow;
                    curwin_mut().w_wcol = old_wcol;
                    curwin_mut().w_wrow = old_wrow;
                }
                if c < 0 {
                    continue;
                }

                for n in 1..=c {
                    g_st().typebuf.tb_noremap[(g_st().typebuf.tb_off + n) as usize] = RM_YES;
                }
                g_st().typebuf.tb_len += c;

                if g_st().typebuf.tb_len >= g_st().typebuf.tb_maplen + MAXMAPLEN as i32 {
                    timedout = true;
                    continue;
                }

                if g_st().ex_normal_busy > 0 {
                    if g_st().typebuf.tb_len > 0 {
                        timedout = true;
                        continue;
                    }

                    c = if (g_st().state_global & MODE_CMDLINE) != 0
                        || (g_st().cmdwin_type > 0
                            && INP_STATE.with(|s| s.vgetorpeek_tc.get()) == ESC)
                    {
                        CTRL_C_CONST
                    } else {
                        ESC
                    };
                    INP_STATE.with(|s| s.vgetorpeek_tc.set(c));

                    if advance {
                        g_st().typebuf_was_empty = true;
                    }
                    g_st().typebuf.tb_no_abbr_cnt = 0;
                    break;
                }

                if ((g_st().state_global & MODE_INSERT) != 0 || opt_vars().p_lz)
                    && (g_st().state_global & MODE_CMDLINE) == 0
                    && advance
                    && g_st().must_redraw != 0
                    && !g_st().need_wait_return
                {
                    update_screen();
                    setcursor();
                }

                let mut showcmd_idx: i32 = 0;
                let mut showing_partial = false;
                if g_st().typebuf.tb_len > 0 && advance {
                    if ((g_st().state_global & (MODE_NORMAL | MODE_INSERT)) != 0
                        || g_st().state_global == MODE_LANGMAP)
                        && g_st().state_global != MODE_HITRETURN
                    {
                        let last_idx = (g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1) as usize;
                        if (g_st().state_global & MODE_INSERT) != 0
                            && utf_ptr2cells(&g_st().typebuf.tb_buf[last_idx..]) == 1
                        {
                            edit_putchar(i32::from(g_st().typebuf.tb_buf[last_idx]), false);
                            setcursor();
                            showing_partial = true;
                        }
                        let old_wcol = curwin_mut().w_wcol;
                        let old_wrow = curwin_mut().w_wrow;
                        curwin_mut().w_wcol = new_wcol;
                        curwin_mut().w_wrow = new_wrow;
                        push_showcmd();
                        if g_st().typebuf.tb_len > SHOWCMD_COLS {
                            showcmd_idx = g_st().typebuf.tb_len - SHOWCMD_COLS;
                        }
                        while showcmd_idx < g_st().typebuf.tb_len {
                            let idx = (g_st().typebuf.tb_off + showcmd_idx) as usize;
                            add_byte_to_showcmd(g_st().typebuf.tb_buf[idx]);
                            showcmd_idx += 1;
                        }
                        curwin_mut().w_wcol = old_wcol;
                        curwin_mut().w_wrow = old_wrow;
                    }

                    if (g_st().state_global & MODE_CMDLINE) != 0
                        && !get_cmdline_info().cmdbuff().is_null()
                        && g_st().cmdline_star == 0
                    {
                        let last_idx = (g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1) as usize;
                        let first_b = g_st().typebuf.tb_buf.get(last_idx).copied().unwrap_or(0);
                        let cells = utf_ptr2cells(crate::memory::bytes_until_nul(
                            &g_st().typebuf.tb_buf[last_idx..],
                        ));
                        if cells == 1 && first_b < 128 {
                            putcmdline(first_b, false);
                            showing_partial = true;
                        }
                    }
                }

                if g_st().typebuf.tb_len == 0 {
                    timedout = false;
                }

                let mut wait_time: i64 = 0;
                if advance {
                    if g_st().typebuf.tb_len == 0
                        || !(opt_vars().p_timeout
                            || (opt_vars().p_ttimeout && keylen == KEYLEN_PART_KEY))
                    {
                        wait_time = -1;
                    } else if keylen == KEYLEN_PART_KEY && opt_vars().p_ttm >= 0 {
                        wait_time = opt_vars().p_ttm;
                    } else {
                        wait_time = opt_vars().p_tm;
                    }
                }

                let wait_tb_len = g_st().typebuf.tb_len;
                c = inchar(
                    (g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize,
                    g_st().typebuf.tb_buflen - g_st().typebuf.tb_off - g_st().typebuf.tb_len - 1,
                    wait_time,
                );

                if showcmd_idx != 0 {
                    pop_showcmd();
                }
                if showing_partial {
                    if (g_st().state_global & MODE_INSERT) != 0 {
                        edit_unputchar();
                    }
                    if (g_st().state_global & MODE_CMDLINE) != 0
                        && !get_cmdline_info().cmdbuff().is_null()
                    {
                        unputcmdline();
                    } else {
                        setcursor();
                    }
                }

                if c < 0 {
                    continue;
                }
                if c == NUL {
                    if !advance {
                        break;
                    }
                    if wait_tb_len > 0 {
                        timedout = true;
                        continue;
                    }
                } else {
                    while g_st().typebuf.tb_buf
                        [(g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize]
                        != NUL as u8
                    {
                        let idx = (g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize;
                        g_st().typebuf.tb_noremap[idx] = RM_YES;
                        g_st().typebuf.tb_len += 1;
                    }
                }
            }
        }

        if !(c < 0 || (advance && c == NUL)) {
            break;
        }
    }

    if advance
        && opt_vars().p_smd
        && g_st().msg_silent == 0
        && (g_st().state_global & MODE_INSERT) != 0
    {
        if c == ESC && !mode_deleted && g_st().no_mapping == 0 && g_st().mode_displayed {
            if g_st().typebuf.tb_len != 0 && !g_st().key_typed {
                g_st().redraw_cmdline = true;
            } else {
                unshowmode(false);
            }
        } else if c != ESC && mode_deleted {
            if g_st().typebuf.tb_len != 0 && !g_st().key_typed {
                g_st().redraw_cmdline = true;
            } else {
                showmode();
            }
        }
    }

    if timedout && c == ESC {
        gotchars_ignore();
    }

    g_st().vgetc_busy -= 1;

    c
}

fn inchar(tb_idx: usize, maxlen: i32, wait_time: i64) -> i32 {
    let mut len: i32 = 0;
    let mut retesc: i32 = 0;
    let tb_change_cnt = g_st().typebuf.tb_change_cnt;

    if wait_time == -1 || wait_time > 100 {
        ui_flush();
    }

    if g_st().state_global != MODE_HITRETURN {
        g_st().did_outofmem_msg = false;
        g_st().did_swapwrite_msg = false;
    }

    let mut read_size: isize = -1;
    while INP_STATE.with(|s| s.curscript.get()) >= 0 && read_size <= 0 && !g_st().ignore_script {
        let mut script_char = [0u8; 1];
        let cur_idx = INP_STATE.with(|s| s.curscript.get()) as usize;
        if g_st().got_int || {
            read_size = with_inp_ext(|ext| {
                file_read(&mut ext.scriptin.borrow_mut()[cur_idx], &mut script_char)
            });
            read_size != 1
        } {
            closescript();
            if g_st().got_int {
                retesc = 1;
            } else {
                return -1;
            }
        } else {
            g_st().typebuf.tb_buf[tb_idx] = script_char[0];

            len = 1;
        }
    }

    if read_size <= 0 {
        if g_st().got_int {
            let mut dum = [0u8; DUM_LEN + 1];
            loop {
                len = input_get(Some(&mut dum[..DUM_LEN]), 0, 0, None);
                if len == 0 || (len == 1 && dum[0] == CTRL_C_CONST as u8) {
                    break;
                }
            }
            return retesc;
        }

        if wait_time == -1 || wait_time > 10 {
            ui_flush();
        }

        let max_read = (maxlen / 3).max(0) as usize;
        len = input_get(
            (max_read > 0)
                .then(|| &mut g_st().borrow_mut().typebuf.tb_buf[tb_idx..tb_idx + max_read]),
            wait_time as i32,
            tb_change_cnt,
            None,
        );
    }

    if typebuf_changed(tb_change_cnt) {
        return 0;
    }

    if len > 0 {
        g_st().typebuf.tb_change_cnt = g_st().typebuf.tb_change_cnt.wrapping_add(1);
        if g_st().typebuf.tb_change_cnt == 0 {
            g_st().typebuf.tb_change_cnt = 1;
        }
    }

    fix_input_buffer(&mut g_st().typebuf.tb_buf[tb_idx..], len)
}

/// Fix typed characters for use by `vgetc()`.
pub fn fix_input_buffer(buf: &mut [u8], mut len: i32) -> i32 {
    if using_script() == 0 {
        buf[len as usize] = NUL as u8;

        return len;
    }

    let mut idx = 0usize;
    let mut i = len;
    while {
        i -= 1;
        i >= 0
    } {
        if buf[idx] == NUL as u8 || (buf[idx] == K_SPECIAL && (i < 2 || buf[idx + 1] != KS_EXTRA)) {
            buf.copy_within(idx + 1..idx + 1 + i as usize, idx + 3);
            let orig = i32::from(buf[idx]);
            buf[idx + 2] = k_third(orig);
            buf[idx + 1] = k_second(orig);
            buf[idx] = K_SPECIAL;
            idx += 2;
            len += 2;
        }
        idx += 1;
    }

    buf[idx] = NUL as u8;

    len
}

/// Get a key stroke directly from the user.
pub fn get_keystroke(events: Option<&MultiQueue>) -> i32 {
    let mut buf: Vec<u8> = vec![0; 150];
    let mut buflen: i32 = 150;
    let mut len: i32 = 0;
    let mut n: i32;
    let save_mapped_ctrl_c = g_st().mapped_ctrl_c;

    g_st().mod_mask = 0;
    g_st().mapped_ctrl_c = 0;
    loop {
        ui_flush();
        let mut maxlen = (buflen - 6 - len) / 3;
        if maxlen < 10 {
            buflen += 100;
            buf.resize(buflen as usize, 0);
            maxlen = (buflen - 6 - len) / 3;
        }

        n = input_get(
            (maxlen > 0).then(|| &mut buf[len as usize..len as usize + maxlen as usize]),
            if len == 0 { -1 } else { 100 },
            0,
            events,
        );
        if n > 0 {
            n = fix_input_buffer(&mut buf[len as usize..], n);
            len += n;
        }

        if n > 0 {
            len = n;
        }
        if len == 0 {
            continue;
        }

        n = buf[0] as i32;
        if n == K_SPECIAL as i32 {
            n = to_special(buf[1] as i32, buf[2] as i32);
            if buf[1] == KS_MODIFIER || n == K_IGNORE || (is_mouse_key(n) && n != K_LEFTMOUSE) {
                if buf[1] == KS_MODIFIER {
                    g_st().mod_mask = buf[2] as i32;
                }
                len -= 3;
                if len > 0 {
                    buf.copy_within(3..(3 + len as usize), 0);
                }
                continue;
            }
            break;
        }
        if utf_byte2len(n) > len {
            continue;
        }
        let term_idx = if len >= buflen { buflen - 1 } else { len } as usize;
        buf[term_idx] = NUL as u8;
        n = utf_ptr2char(buf.as_slice());
        break;
    }

    g_st().mapped_ctrl_c = save_mapped_ctrl_c;
    merge_modifiers(n, &mut g_st().mod_mask)
}

/// Function passed to `do_cmdline()` to get the command after a `<Cmd>` key from typeahead.
pub fn getcmdkeycmd(promptc: i32, _indent: i32, _do_concat: bool) -> Option<std::ffi::CString> {
    let mut line: Vec<u8> = Vec::new();
    let mut c1: i32 = -1;
    let mut cmod: i32 = 0;
    let mut aborted = false;

    g_st().no_mapping += 1;

    g_st().got_int = false;
    while c1 != NUL && !aborted {
        if vgetorpeek(false) == NUL {
            emsg(gettext(E_CMD_MAPPING_MUST_END_WITH_CR));
            aborted = true;
            break;
        }

        c1 = vgetorpeek(true);

        if c1 == K_SPECIAL as i32 {
            c1 = vgetorpeek(true);
            let c2 = vgetorpeek(true);
            if c1 == KS_MODIFIER as i32 {
                cmod = c2;
                continue;
            }
            c1 = to_special(c1, c2);
        }

        if g_st().got_int {
            aborted = true;
        } else if c1 == b'\r' as i32 || c1 == b'\n' as i32 {
            c1 = NUL;
        } else if c1 == ESC {
            aborted = true;
        } else if c1 == K_COMMAND {
            emsg(gettext(E_CMD_MAPPING_MUST_END_WITH_CR_BEFORE_SECOND_CMD));
            aborted = true;
        } else if c1 == K_SNR {
            line.extend_from_slice(b"<SNR>");
        } else {
            if cmod != 0 {
                line.extend_from_slice(&[K_SPECIAL, KS_MODIFIER, cmod as u8]);
            }
            if is_special(c1) {
                line.extend_from_slice(&[K_SPECIAL, k_second(c1), k_third(c1)]);
            } else {
                line.push(c1 as u8);
            }
        }

        cmod = 0;
    }

    g_st().no_mapping -= 1;

    if aborted {
        None
    } else {
        let out = bytes_to_cstring(&line);
        if promptc == b':' as i32 {
            atom_cmdline_set(K_COMMAND, out.as_bytes());
        }
        Some(out)
    }
}

/// Handle a Lua mapping: get its id from typeahead and execute its callback.
pub fn map_execute_lua(may_repeat: bool, discard: bool) -> bool {
    let mut line: Vec<u8> = Vec::new();
    let mut c1: i32 = -1;
    let mut aborted = false;

    g_st().no_mapping += 1;

    g_st().got_int = false;
    while c1 != NUL && !aborted {
        c1 = vgetorpeek(true);
        if g_st().got_int {
            aborted = true;
        } else if c1 == b'\r' as i32 || c1 == b'\n' as i32 {
            c1 = NUL;
        } else {
            line.push(c1 as u8);
        }
    }

    g_st().no_mapping -= 1;

    if aborted || discard {
        return !aborted;
    }

    line.push(0);
    let id = crate::charset::parse_i64_prefix(&line) as i32;
    drop(line);
    let ref_ = map_luaid_get(id);
    if ref_ == LUA_NOREF {
        emsg(gettext(E_LUA_MAPPING_GONE));
        return false;
    }
    if may_repeat {
        g_st().repeat_luamap = id;
    }

    let mut err = Error::new();
    let args = Array::new();
    nlua_call_ref(ref_, None, args, K_RET_NIL_BOOL, None, Some(&mut err));
    if err.err_type != ErrorType::ErrorTypeNone {
        semsg_multiline!(c"emsg", c"E5108: %s", err.msg_cstr());
        api_clear_error(&mut err);
    }

    true
}

/// Wraps pasted text stream with `K_PASTE_START` and `K_PASTE_END`.
pub fn paste_store(channel_id: u64, state: TriState, str_: NvimString, crlf: bool) {
    if (g_st().state_global & MODE_CMDLINE) != 0 {
        return;
    }

    let need_redo = !INP_STATE.with(|s| s.block_redo.get());
    let need_record = g_st().reg_recording != 0 && !is_internal_call(channel_id);

    if !need_redo && !need_record {
        return;
    }

    if state != K_NONE {
        let c = if state == K_FALSE {
            K_PASTE_START
        } else {
            K_PASTE_END
        };
        if need_redo {
            if state == K_FALSE && (g_st().state_global & MODE_INSERT) == 0 {
                redo_new(CmdSpec::default());
            }
            with_inp_ext(|ext| sb_add_char(&mut ext.redobuff.borrow_mut().cur.body, c));
        }
        if need_record {
            with_inp_ext(|ext| sb_add_char(&mut ext.recordbuff.borrow_mut(), c));
        }
        return;
    }

    let str_bytes = str_.as_bytes();
    let mut pos = 0usize;

    while pos < str_bytes.len() {
        let start = pos;
        while pos < str_bytes.len()
            && str_bytes[pos] != K_SPECIAL
            && str_bytes[pos] != 0
            && str_bytes[pos] != NL as u8
            && !(crlf && str_bytes[pos] == CAR as u8)
        {
            pos += 1;
        }

        if pos > start {
            let slice = &str_bytes[start..pos];
            if need_redo {
                with_inp_ext(|ext| {
                    ext.redobuff.borrow_mut().cur.body.extend_from_slice(slice);
                });
            }
            if need_record {
                with_inp_ext(|ext| ext.recordbuff.borrow_mut().extend_from_slice(slice));
            }
        }

        if pos < str_bytes.len() {
            let mut c = str_bytes[pos] as i32;
            pos += 1;
            if crlf && c == CAR {
                if pos < str_bytes.len() && str_bytes[pos] == NL as u8 {
                    pos += 1;
                }
                c = NL;
            }
            if need_redo {
                with_inp_ext(|ext| sb_add_byte(&mut ext.redobuff.borrow_mut().cur.body, c));
            }
            if need_record {
                with_inp_ext(|ext| sb_add_byte(&mut ext.recordbuff.borrow_mut(), c));
            }
        }
    }
}

/// Gets a paste stored by `paste_store()` from typeahead and repeats it.
pub fn paste_repeat(count: i32) {
    let mut buf: Vec<u8> = Vec::new();
    let mut aborted = false;

    g_st().no_mapping += 1;

    g_st().got_int = false;
    while !aborted {
        let mut c1 = vgetorpeek(true) as u8;
        if c1 == K_SPECIAL {
            c1 = vgetorpeek(true) as u8;
            let c2 = vgetorpeek(true) as u8;
            let c = to_special(c1 as i32, c2 as i32);
            if c == K_PASTE_END {
                break;
            } else if c == K_ZERO {
                buf.push(NUL as u8);
            } else if c == K_SPECIAL as i32 {
                buf.push(K_SPECIAL);
            } else {
                buf.extend_from_slice(&[K_SPECIAL, c1, c2]);
            }
        } else {
            buf.push(c1);
        }
        aborted = g_st().got_int;
    }

    g_st().no_mapping -= 1;

    let str_ = NvimString::from_bytes(&buf);
    let mut arena = Arena::new();
    let mut err = Error::new();
    let mut i = 0;
    while !aborted && i < count {
        nvim_paste(LUA_INTERNAL_CALL, str_, false, -1, &mut arena, &mut err);
        aborted = err.err_type != ErrorType::ErrorTypeNone;
        i += 1;
    }
    api_clear_error(&mut err);
    arena_mem_free(arena_finish(&mut arena));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_byte_encode_and_merge_modifiers_work() {
        let mut tmp = [0u8; 4];
        assert_eq!(key_byte_encode(b'a' as i32, &mut tmp), 1);
        assert_eq!(tmp[0], b'a');
        assert_eq!(key_byte_encode(NUL, &mut tmp), 3);
        assert_eq!(tmp[0], K_SPECIAL);

        let mut mods = MOD_MASK_CTRL as i32;
        let merged = merge_modifiers(b'c' as i32, &mut mods);
        assert_eq!(merged, CTRL_C_CONST);
        assert_eq!(mods, 0);
    }
}
