//! The input engine "bytes layer" (`src/nvim/input.c`).

use crate::api::private::helpers::api_clear_error;
use crate::api::private::validate::{Array, Error, ErrorType, NvimString};
use crate::api::vim::{is_internal_call, nvim_paste};
use crate::charset::{ascii_iswhite, is_special, skipwhite, to_special};
use crate::cursor::{colnr_T, get_cursor_line_ptr, linenr_T};
use crate::drawscreen::{SHOWCMD_COLS, setcursor, showmode, unshowmode, update_screen};
use crate::errors::*;
use crate::eval::garbage_collect;
use crate::eval::typval::{
    EvalFuncData, VarType, tv_check_for_opt_dict_arg, tv_dict_get_bool, tv_dict_get_string,
    tv_dict_has_key, tv_get_bool_chk, typval_T, varnumber_T,
};
use crate::eval::vars::{
    VV_MOUSE_COL, VV_MOUSE_LNUM, VV_MOUSE_WIN, VV_MOUSE_WINID, set_vim_var_nr,
};
use crate::event::r#loop::{main_loop_events, uv_strerror};
use crate::event::multiqueue::{MultiQueue, multiqueue_empty};
use crate::ex_cmds::{Ctrl_C, check_secure};
use crate::ex_docmd::update_topline_cursor;
use crate::ex_getln::{
    Ctrl_O, K_LUA, get_cmdline_info, kOptBoFlagError, putcmdline, redrawcmd, redrawcmdline,
    unputcmdline,
};
use crate::extmark::{TriState, kFalse, kNone};
use crate::fileio::fputc;
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
use crate::lua::executor::{kRetNilBool, nlua_call_ref, nlua_exec_on_key};
use crate::mapping::{
    MAXMAPLEN, REMAP_NONE, REMAP_YES, eval_map_expr, get_buf_maphash_list, get_maphash_list,
    langmap_adjust_mb, map_luaid_get, mapblock_T, typebuf_T,
};
use crate::mbyte::{
    mb_cptr2char_adv, mb_unescape, utf_ClusterInfo, utf_byte2len, utf_char2bytes, utf_head_off,
    utf_ptr2StrCharInfo, utf_ptr2cells, utf_ptr2char,
};
use crate::mcursor::mc_replaying;
use crate::memline::ml_sync_all;
use crate::memory::{Arena, arena_finish, arena_mem_free, cstr_from_bytes};
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
use crate::os::fs::MAXPATHL;
use crate::os::input::{input_available, input_get, line_breakcheck, os_breakcheck};
use crate::plines::{
    CSType, CharsizeArg, ClusterInfo, StrCharInfo, init_charsize_arg, win_charsize,
};
use crate::state::{
    MODE_ASKMORE, MODE_CMDLINE, MODE_HITRETURN, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL,
    MODE_SELECT, MODE_TERMINAL, MODE_VISUAL, get_real_state, state_handle_k_event,
    state_no_longer_safe,
};
use crate::strings::{gettext, vim_snprintf, vim_strchr};
use crate::textformat::oparg_T;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_cursor_goto, ui_flush, vim_beep};
use crate::undo::u_sync;
use std::ffi::{CStr, c_char, c_int, c_long, c_uint};
use std::ptr;

use crate::option::OptInt;
pub type flush_buffers_T = c_int;

pub(crate) const NUL: c_int = 0;

pub(crate) const FLUSH_MINIMAL: flush_buffers_T = 0;
const FLUSH_INPUT: flush_buffers_T = 2;

const NSCRIPT: usize = 15;
const TYPELEN_INIT: usize = 5 * (MAXMAPLEN + 3);
const DUM_LEN: usize = MAXMAPLEN * 3 + 3;

const RM_YES: u8 = 0;
const RM_NONE: u8 = 1;
const RM_SCRIPT: u8 = 2;
const RM_ABBR: u8 = 4;

pub(crate) const REMAP_SCRIPT: c_int = -2;
const REMAP_SKIP: c_int = -3;

const KEYLEN_PART_KEY: c_int = -1;
const KEYLEN_PART_MAP: c_int = -2;

const KS_VER_SCROLLBAR: c_int = 249;
const KS_HOR_SCROLLBAR: c_int = 248;
const K_VER_SCROLLBAR: c_int = termcap2key(KS_VER_SCROLLBAR, KE_FILLER as c_int);
const K_HOR_SCROLLBAR: c_int = termcap2key(KS_HOR_SCROLLBAR, KE_FILLER as c_int);
pub(crate) const K_PASTE_START: c_int = termcap2key(b'P' as c_int, b'S' as c_int);
const K_PASTE_END: c_int = termcap2key(b'P' as c_int, b'E' as c_int);
pub(crate) const K_COMPLETE_DELAY: c_int =
    termcap2key(KS_EXTRA as c_int, KE_COMPLETE_DELAY as c_int);

const K_SELECT_STRING: &[u8; 4] = b"\x80\xf5X\0";
const ESC_STR: &CStr = c"\x1b";
const NL_STR: &CStr = c"\n";

pub(crate) const kInsNone: c_int = 0;

const E_RECURSIVE_MAPPING: &CStr = c"E223: Recursive mapping";
const E_CMD_MAPPING_MUST_END_WITH_CR: &CStr = c"E1255: <Cmd> mapping must end with <CR>";
const E_CMD_MAPPING_MUST_END_WITH_CR_BEFORE_SECOND_CMD: &CStr =
    c"E1136: <Cmd> mapping must end with <CR> before second <Cmd>";
const E_LUA_MAPPING_GONE: &CStr = c"E5117: Lua mapping no longer exists";

/// Readahead ("stuff") buffer.
#[repr(C)]
#[derive(Clone)]
pub struct StuffBuf {
    pub keys: Vec<c_char>,
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
    pub c: c_int,
    pub mod_mask: c_int,
    pub mouse_grid: c_int,
    pub mouse_row: c_int,
    pub mouse_col: c_int,
    pub stuffed: bool,
}

/// Struct to hold the saved typeahead for `save_typeahead()`.
#[repr(C)]
#[derive(Clone)]
pub struct tasave_T {
    pub save_typebuf: typebuf_T,
    pub ungot: UngotKey,
    pub save_readbuf1: StuffBuf,
    pub save_readbuf2: StuffBuf,
}

impl tasave_T {
    pub const fn new() -> Self {
        Self {
            save_typebuf: typebuf_T::ZERO,
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
struct gotchars_state_T {
    buf: [u8; MB_MAXBYTES * 3 + 4],
    prev_c: c_int,
    buflen: usize,
    pending_special: c_uint,
    pending_mbyte: c_uint,
}

impl gotchars_state_T {
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
enum map_result_T {
    map_result_fail = 0,
    map_result_get = 1,
    map_result_retry = 2,
    map_result_nomatch = 3,
}

struct InputModuleState {
    curscript: c_int,
    scriptin: [FileDescriptor; NSCRIPT],
    redobuff: RedoState,
    recordbuff: Vec<c_char>,
    readbuf1: StuffBuf,
    readbuf2: StuffBuf,
    ungot: UngotKey,
    on_key_buf: Vec<c_char>,
    on_key_ignore_len: usize,
    on_key_blocked: c_int,
    typeahead_char: c_int,
    block_redo: bool,
    KeyNoremap: c_int,
    last_recorded_len: usize,
    saved_typebuf: [typebuf_T; NSCRIPT],
    no_reduce_keys: c_int,
    GOTCHARS_STATE: gotchars_state_T,
    UPDATESCRIPT_COUNT: c_int,
    SHOWCMD_STATE: gotchars_state_T,
    LAST_VGETC_RECORDED_LEN: usize,
    VGETORPEEK_TC: c_int,
}

impl InputModuleState {
    const fn new() -> Self {
        Self {
            curscript: -1,
            scriptin: [const { FileDescriptor::new() }; NSCRIPT],
            redobuff: RedoState::new(),
            recordbuff: Vec::new(),
            readbuf1: StuffBuf::new(),
            readbuf2: StuffBuf::new(),
            ungot: UngotKey {
                c: -1,
                mod_mask: 0,
                mouse_grid: 0,
                mouse_row: 0,
                mouse_col: 0,
                stuffed: false,
            },
            on_key_buf: Vec::new(),
            on_key_ignore_len: 0,
            on_key_blocked: 0,
            typeahead_char: 0,
            block_redo: false,
            KeyNoremap: 0,
            last_recorded_len: 0,
            saved_typebuf: [const { typebuf_T::ZERO }; NSCRIPT],
            no_reduce_keys: 0,
            GOTCHARS_STATE: gotchars_state_T::new(),
            UPDATESCRIPT_COUNT: 0,
            SHOWCMD_STATE: gotchars_state_T::new(),
            LAST_VGETC_RECORDED_LEN: 0,
            VGETORPEEK_TC: 0,
        }
    }
}

static INP_STATE: GlobalState<InputModuleState> = GlobalState::new(InputModuleState::new());

#[inline]
fn inp_st() -> &'static mut InputModuleState {
    unsafe { &mut *INP_STATE.as_ptr() }
}

#[inline]
fn tb_buf_ptr() -> *mut u8 {
    unsafe {
        let p = &raw mut g_st().typebuf.tb_buf;
        (*p).as_mut_ptr()
    }
}

#[inline]
fn tb_noremap_ptr() -> *mut u8 {
    unsafe {
        let p = &raw mut g_st().typebuf.tb_noremap;
        (*p).as_mut_ptr()
    }
}

use crate::api::buffer::LUA_INTERNAL_CALL;
use crate::cmdexpand::{Ctrl_N, Ctrl_P, MB_MAXBYTES};
use crate::keycodes::{k_second, k_third};
use crate::lua::converter::LUA_NOREF;
use crate::regexp::Ctrl_V;

#[inline]
pub fn mb_byte2len_check(b: c_int) -> c_int {
    if !(0..=255).contains(&b) {
        1
    } else {
        utf_byte2len(b)
    }
}

#[inline]
pub(crate) fn langmap_adjust(c: &mut c_int, condition: bool) {
    unsafe {
        if *opt_vars().p_langmap != 0
            && condition
            && (opt_vars().p_lrm != 0
                || if g_st().vgetc_busy != 0 {
                    typebuf_maplen() == 0
                } else {
                    g_st().KeyTyped
                })
            && g_st().KeyStuffed == 0
            && *c >= 0
        {
            if *c < 256 {
                *c = g_st().langmap_mapchar[*c as usize] as c_int;
            } else {
                *c = langmap_adjust_mb(*c);
            }
        }
    }
}

/// Gets the contents of the record buffer as a string (typeahead encoding) and clears the buffer.
pub fn get_recorded() -> std::ffi::CString {
    unsafe {
        let rb = &raw mut inp_st().recordbuff;
        let mut bytes: Vec<u8> = std::mem::take(&mut *rb)
            .into_iter()
            .map(|b| b as u8)
            .collect();
        let mut len = bytes.len();

        if len >= inp_st().last_recorded_len {
            len -= inp_st().last_recorded_len;
            bytes.truncate(len);
        }

        if len > 0 && g_st().restart_edit != 0 && bytes[len - 1] == Ctrl_O as u8 {
            bytes.pop();
        }

        crate::memory::cstr_from_bytes(&bytes)
    }
}

fn redo_prefix(spec: *const CmdSpec, buf: &mut Vec<c_char>, replay: bool) {
    unsafe {
        if (*spec).regname != 0 {
            sb_add_char(buf, b'"' as c_int);
            sb_add_char(buf, (*spec).regname);
            if replay && (*spec).regname == b'=' as c_int {
                sb_add_char(buf, CAR);
            }
        }
        if (*spec).count != 0 {
            let mut numbuf = [0 as c_char; 32];
            let n = vim_snprintf!(
                numbuf.as_mut_ptr(),
                numbuf.len(),
                c"%d",
                (*spec).count as c_int,
            );
            if n > 0 {
                buf.extend_from_slice(&numbuf[..n as usize]);
            }
        }
    }
}

fn redo_chars(spec: *const CmdSpec, buf: &mut Vec<c_char>, arg_meta: bool) {
    unsafe {
        if (*spec).op != NUL {
            sb_add_char(buf, (*spec).op);
        }
        if (*spec).op_extra != NUL {
            sb_add_char(buf, (*spec).op_extra);
        }
        if (*spec).motion_force != NUL {
            sb_add_char(buf, (*spec).motion_force);
        }
        if (*spec).cmd != NUL {
            sb_add_char(buf, (*spec).cmd);
        }
        if (*spec).cmd2 != NUL {
            sb_add_char(buf, (*spec).cmd2);
        }
        if (*spec).cmdarg != NUL && !arg_meta {
            sb_add_char(buf, (*spec).cmdarg);
        }
    }
}

/// Gets `spec` (or current pending change, if `spec` is NULL), composed into a "redo" keyseq.
pub fn redo_keys(spec: *const CmdSpec) -> Option<std::ffi::CString> {
    unsafe {
        let spec = if !spec.is_null() {
            spec
        } else {
            &raw const inp_st().redobuff.cur
        };

        let mut buf: Vec<c_char> = Vec::new();
        redo_prefix(spec, &mut buf, false);
        if !(*spec).body.is_empty() {
            buf.extend_from_slice(&(*spec).body);
        } else {
            redo_chars(spec, &mut buf, false);
        }
        if buf.is_empty() {
            None
        } else {
            let bytes = std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), buf.len());
            Some(crate::memory::cstr_from_bytes(bytes))
        }
    }
}

/// Gets the pending change's `CmdSpec` (without `body`).
pub fn redo_spec() -> CmdSpec {
    unsafe {
        let cur = &raw const inp_st().redobuff.cur;
        (*cur).without_body()
    }
}

fn add_buff(buf: &mut StuffBuf, s: impl crate::strings::IntoCCharPtr, mut slen: isize) {
    let s = s.into_c_char_ptr();
    unsafe {
        if slen < 0 {
            slen = crate::memory::cstr_len(s) as isize;
        }
        if slen == 0 {
            return;
        }
        let slen_u = slen as usize;
        let slice = std::slice::from_raw_parts(s, slen_u);
        let insert = buf.insert;
        buf.keys.splice(insert..insert, slice.iter().copied());
        buf.insert += slen_u;
    }
}

fn add_num_buff(buf: &mut StuffBuf, n: c_int) {
    let mut number = [0 as c_char; 32];
        let numberlen = vim_snprintf!(number.as_mut_ptr(), number.len(), c"%d", n);
        add_buff(&mut *buf, number.as_ptr(), numberlen as isize);
}

fn key_byte_encode(c: c_int, temp: &mut [c_char; 4]) -> isize {
    if is_special(c) || c == K_SPECIAL as c_int || c == NUL {
        temp[0] = K_SPECIAL as c_char;
        temp[1] = k_second(c) as c_char;
        temp[2] = k_third(c) as c_char;
        temp[3] = NUL as c_char;
        3
    } else {
        temp[0] = c as c_char;
        temp[1] = NUL as c_char;
        1
    }
}

fn key_char_encode(c: c_int, buf: *mut c_char) -> usize {
    unsafe {
        let mut bytes = [0u8; MB_MAXBYTES + 1];
        let len = if is_special(c) {
            1
        } else {
            utf_char2bytes(c, bytes.as_mut_ptr().cast())
        };
        let mut off: usize = 0;
        for &b in bytes.iter().take(len as usize) {
            let mut tmp = [0 as c_char; 4];
            let n = key_byte_encode(if is_special(c) { c } else { b as c_int }, &mut tmp) as usize;
            ptr::copy_nonoverlapping(tmp.as_ptr(), buf.add(off), n + 1);
            off += n;
        }
        off
    }
}

fn add_char_buff(buf: &mut StuffBuf, c: c_int) {
    let mut temp = [0 as c_char; MB_MAXBYTES * 3 + 1];
    let len = key_char_encode(c, temp.as_mut_ptr());
    add_buff(buf, temp.as_ptr(), len as isize);
}

fn sb_add_byte(sb: &mut Vec<c_char>, c: c_int) {
    let mut temp = [0 as c_char; 4];
    let templen = key_byte_encode(c, &mut temp) as usize;
    sb.extend_from_slice(&temp[..templen]);
}

/// Dual of `add_char_buff()`, for a byte buffer.
pub fn sb_add_char(sb: &mut Vec<c_char>, c: c_int) {
    let mut temp = [0 as c_char; MB_MAXBYTES * 3 + 1];
        let len = key_char_encode(c, temp.as_mut_ptr());
        sb.extend_from_slice(&temp[..len]);
}

fn read_readbuffers(advance: bool) -> c_int {
    let mut c = read_readbuf(&mut inp_st().readbuf1, advance);
        if c == NUL {
            c = read_readbuf(&mut inp_st().readbuf2, advance);
        }
        c
}

fn read_readbuf(buf: &mut StuffBuf, advance: bool) -> c_int {
    if buf.read == buf.keys.len() {
            return NUL;
        }

        let c = buf.keys.as_slice()[buf.read] as u8 as c_int;

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
    inp_st().readbuf1.insert = inp_st().readbuf1.read;
        inp_st().readbuf2.insert = inp_st().readbuf2.read;
}

/// Returns true if the readahead ("stuff") buffer is empty.
pub fn stuff_empty() -> bool {
    unsafe {
        let k1 = &raw const inp_st().readbuf1.keys;
        let k2 = &raw const inp_st().readbuf2.keys;
        (*k1).is_empty() && (*k2).is_empty()
    }
}

/// Returns true if `inp_st().readbuf1` is empty.
pub fn readbuf1_empty() -> bool {
    unsafe {
        let k1 = &raw const inp_st().readbuf1.keys;
        (*k1).is_empty()
    }
}

/// Set a typeahead character that won't be flushed.
pub fn typeahead_noflush(c: c_int) {
    inp_st().typeahead_char = c;
}

/// Remove the contents of the stuff buffer and the mapped characters in the typeahead buffer.
pub fn flush_buffers(flush_typeahead: flush_buffers_T) {
    unsafe {
        init_typebuf();

        if g_st().typebuf.tb_maplen > 0 {
            atom_composite_abort();
        }

        let k2 = &raw const inp_st().readbuf2.keys;
        if !(*k2).is_empty() && g_st().Visual.active {
            g_st().Visual.need_end = true;
        }
        inp_st().readbuf1 = StuffBuf::new();
        inp_st().readbuf2 = StuffBuf::new();

        if flush_typeahead == FLUSH_MINIMAL {
            if g_st().typebuf.tb_off + g_st().typebuf.tb_maplen >= g_st().typebuf.tb_buflen {
                g_st().typebuf.tb_off = MAXMAPLEN as c_int;
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
                while inchar(tb_buf_ptr(), g_st().typebuf.tb_buflen - 1, 10) != 0 {}
            }
            g_st().typebuf.tb_off = MAXMAPLEN as c_int;
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
}

/// Hides keys from `vim.on_key()` callbacks.
pub fn block_on_key() {
    inp_st().on_key_blocked += 1;
}

pub fn unblock_on_key() {
    inp_st().on_key_blocked -= 1;
}

/// Flush map and typeahead buffers and give a warning for an error.
pub fn beep_flush() {
    if g_st().emsg_silent == 0 && !mc_replaying() {
            flush_buffers(FLUSH_MINIMAL);
        }
        vim_beep(kOptBoFlagError);
}

/// Starts capturing a new change.
pub fn redo_new(spec: CmdSpec) {
    unsafe {
        assert!(spec.body.is_empty());
        if inp_st().block_redo {
            return;
        }

        let rb = &raw mut inp_st().redobuff;
        (*rb).old = std::mem::replace(&mut (*rb).cur, spec);
    }
}

/// Frees both redo buffers.
pub fn redo_free_all() {
    unsafe {
        let rb = &raw mut inp_st().redobuff;
        *rb = RedoState::new();
    }
}

/// Prepare for redo of any command.
pub fn prep_redo(as_atom: bool, arg_meta: bool, spec: CmdSpec) {
    unsafe {
        if as_atom {
            atom_redo_prepped();
        }
        redo_new(spec.without_body());
        if inp_st().block_redo {
            return;
        }
        let body = &raw mut inp_st().redobuff.cur.body;
        redo_chars(&raw const spec, &mut *body, arg_meta);
    }
}

/// Prepare for redo of a Visual-mode command.
pub fn prep_redo_visual(keys: *const c_char, len: usize, spec: CmdSpec) {
    unsafe {
        let mut stored = spec.without_body();
        stored.regname = 0;
        stored.count = 0;
        atom_redo_prepped();
        redo_new(stored);
        if inp_st().block_redo {
            return;
        }
        let body = &raw mut inp_st().redobuff.cur.body;
        if !keys.is_null() && len > 0 {
            (*body).extend_from_slice(std::slice::from_raw_parts(keys, len));
        }
        redo_prefix(&raw const spec, &mut *body, false);
        redo_chars(&raw const spec, &mut *body, false);
    }
}

/// Discard the contents of the redo buffer and restore the previous redo buffer.
pub fn redo_cancel() {
    unsafe {
        if inp_st().block_redo {
            return;
        }

        atom_redo_cancel();
        let rb = &raw mut inp_st().redobuff;
        (*rb).cur = std::mem::take(&mut (*rb).old);
        inp_st().readbuf1 = StuffBuf::new();
        inp_st().readbuf2 = StuffBuf::new();
    }
}

/// Saves the redo state to `save_redo` and detaches it.
pub fn save_redobuff(save_redo: &mut RedoState) {
    unsafe {
        let rb = &raw mut inp_st().redobuff;
        save_redo.cur = (*rb).cur.clone();
        save_redo.old = std::mem::take(&mut (*rb).old);
    }
}

/// Restores the redo state from `save_redo`.
pub fn restore_redobuff(save_redo: &mut RedoState) {
    unsafe {
        let rb = &raw mut inp_st().redobuff;
        *rb = std::mem::take(&mut *save_redo);
    }
}

/// Append `len` bytes of `s` (-1: up to the NUL) to the redo buffer.
pub fn redo_append_str<S: crate::strings::IntoCCharPtr>(s: S, len: isize) {
    let s = s.into_c_char_ptr();
    unsafe {
        if !inp_st().block_redo {
            let slen = if len < 0 { crate::memory::cstr_len(s) } else { len as usize };
            if !s.is_null() && slen > 0 {
                let body = &raw mut inp_st().redobuff.cur.body;
                (*body).extend_from_slice(std::slice::from_raw_parts(s, slen));
            }
        }
    }
}

/// Appends `str` to `buf` literally, escaping special characters with CTRL-V.
pub fn sb_add_lit(buf: &mut Vec<c_char>, str_: *const c_char, len: c_int) {
    unsafe {
        let mut s = str_;
        while if len < 0 {
            *s != NUL as c_char
        } else {
            s.offset_from(str_) < len as isize
        } {
            let start = s;
            while *s >= b' ' as c_char
                && (*s as u8) < DEL as u8
                && (len < 0 || s.offset_from(str_) < len as isize)
            {
                s = s.add(1);
            }

            if *s == NUL as c_char && (*s.sub(1) == b'0' as c_char || *s.sub(1) == b'^' as c_char) {
                s = s.sub(1);
            }
            if s > start {
                let n = s.offset_from(start) as usize;
                buf.extend_from_slice(std::slice::from_raw_parts(start, n));
            }

            if *s == NUL as c_char || (len >= 0 && s.offset_from(str_) >= len as isize) {
                break;
            }

            let c = mb_cptr2char_adv(&raw mut s);
            if c < b' ' as c_int
                || c == DEL
                || (*s == NUL as c_char && (c == b'0' as c_int || c == b'^' as c_int))
            {
                sb_add_char(buf, Ctrl_V);
            }

            if *s == NUL as c_char && c == b'0' as c_int {
                buf.extend_from_slice(std::slice::from_raw_parts(c"048".as_ptr(), 3));
            } else {
                sb_add_char(buf, c);
            }
        }
    }
}

/// Append to redo literally; no-op if `inp_st().block_redo` is set.
pub fn redo_append_lit(str_: *const c_char, len: c_int) {
    unsafe {
        if inp_st().block_redo {
            return;
        }
        let body = &raw mut inp_st().redobuff.cur.body;
        sb_add_lit(&mut *body, str_, len);
    }
}

/// Appends `s` to `buf`, leaving 3-byte special key codes unmodified and escaping other K_SPECIAL bytes.
pub fn sb_add_spec(buf: &mut Vec<c_char>, mut s: *const c_char) {
    unsafe {
        while *s != NUL as c_char {
            if *s as u8 == K_SPECIAL && *s.add(1) != NUL as c_char && *s.add(2) != NUL as c_char {
                buf.extend_from_slice(std::slice::from_raw_parts(s, 3));
                s = s.add(3);
            } else {
                let c = mb_cptr2char_adv(&raw mut s);
                sb_add_char(buf, c);
            }
        }
    }
}

/// Append `s` to the redo buffer.
pub fn redo_append_spec(s: *const c_char) {
    unsafe {
        if inp_st().block_redo {
            return;
        }
        let body = &raw mut inp_st().redobuff.cur.body;
        sb_add_spec(&mut *body, s);
    }
}

/// Appends character `c` to the redo buffer, translated to typeahead encoding.
pub fn redo_append_char(c: c_int) {
    unsafe {
        if !inp_st().block_redo {
            let body = &raw mut inp_st().redobuff.cur.body;
            sb_add_char(&mut *body, c);
        }
    }
}

/// Appends string `s` (must be typeahead encoding) to the stuff buffer.
pub fn stuffReadbuff(s: impl crate::strings::IntoCCharPtr) {
    let s = s.into_c_char_ptr();
    add_buff(&mut inp_st().readbuf1, s, -1);
}

/// Appends string `s` (must be typeahead encoding) to the redo stuff buffer.
pub fn stuffRedoReadbuff(s: &std::ffi::CStr) {
    add_buff(&mut inp_st().readbuf2, s.as_ptr(), -1);
}

pub fn stuffReadbuffLen(s: impl crate::strings::IntoCCharPtr, len: isize) {
    let s = s.into_c_char_ptr();
    unsafe {
        if g_st().add_last_insert == 1 && len > 0 {
            let last_ins = &raw mut g_st().last_insert_ga;
            (*last_ins).extend_from_slice(std::slice::from_raw_parts(s, len as usize));
        }
        add_buff(&mut inp_st().readbuf1, s, len);
    }
}

/// Stuff `s` into the stuff buffer, leaving special key codes unmodified.
pub fn stuffReadbuffSpecial(mut s: *const c_char) {
    unsafe {
        while *s != NUL as c_char {
            if *s as u8 == K_SPECIAL && *s.add(1) != NUL as c_char && *s.add(2) != NUL as c_char {
                stuffReadbuffLen(s, 3);
                s = s.add(3);
            } else {
                let mut c = mb_cptr2char_adv(&raw mut s);
                if c == CAR || c == NL || c == ESC {
                    c = b' ' as c_int;
                }
                stuffcharReadbuff(c);
            }
        }
    }
}

/// Appends character `c` to the stuff buffer, translated to typeahead encoding.
pub fn stuffcharReadbuff(c: c_int) {
    add_char_buff(&mut inp_st().readbuf1, c);
}

/// Append a number to the stuff buffer.
pub fn stuffnumReadbuff(n: c_int) {
    add_num_buff(&mut inp_st().readbuf1, n);
}

/// Stuff a string into the typeahead buffer.
pub fn stuffescaped(mut arg: *const c_char, literally: bool) {
    unsafe {
        while *arg != NUL as c_char {
            let start = arg;
            while (*arg >= b' ' as c_char && (*arg as u8) < DEL as u8)
                || (*arg as u8 == K_SPECIAL && !literally)
            {
                arg = arg.add(1);
            }
            if arg > start {
                stuffReadbuffLen(start, arg.offset_from(start));
            }

            if *arg != NUL as c_char {
                let c = mb_cptr2char_adv(&raw mut arg);
                if literally && ((c < b' ' as c_int && c != TAB) || c == DEL) {
                    stuffcharReadbuff(Ctrl_V);
                }
                stuffcharReadbuff(c);
            }
        }
    }
}

/// Dot-repeat `"."` command.
pub fn start_redo(count: c_int, old_redo: bool) -> bool {
    unsafe {
        let rd = if old_redo {
            &raw mut inp_st().redobuff.old
        } else {
            &raw mut inp_st().redobuff.cur
        };
        if (*rd).body.is_empty() && (*rd).regname == 0 && (*rd).count == 0 {
            return false;
        }

        let mut spec = (*rd).without_body();
        if spec.regname >= b'1' as c_int && spec.regname < b'9' as c_int {
            spec.regname += 1;
        }
        if count != 0 {
            spec.count = c_long::from(count);
        }
        let mut prefix: Vec<c_char> = Vec::new();
        redo_prefix(&raw const spec, &mut prefix, true);
        add_buff(&mut inp_st().readbuf2, prefix.as_ptr(), prefix.len() as isize);
        drop(prefix);
        if spec.regname == b'=' as c_int {
            g_st().cmd_silent = true;
        }

        add_buff(
            &mut inp_st().readbuf2,
            (*rd).body.as_ptr(),
            (*rd).body.len() as isize,
        );
        true
    }
}

/// Repeats the last insert (`R`, `o`, `O`, `a`, `A`, `i` or `I` command).
pub fn start_redo_ins() -> bool {
    unsafe {
        let body = &raw const inp_st().redobuff.cur.body;
        if (*body).is_empty() {
            return false;
        }
        start_stuff();

        let mut p = (*body).as_ptr();
        let end = p.add((*body).len());
        while p < end {
            if *p as u8 == K_SPECIAL && end.offset_from(p) >= 3 {
                p = p.add(3);
                continue;
            }
            if !vim_strchr(c"AaIiRrOo", *p as u8 as c_int).is_null() {
                if *p == b'O' as c_char || *p == b'o' as c_char {
                    add_buff(&mut inp_st().readbuf2, NL_STR, -1);
                }
                p = p.add(1);
                break;
            }
            p = p.add(1);
        }

        add_buff(&mut inp_st().readbuf2, p, end.offset_from(p));
        inp_st().block_redo = true;
        true
    }
}

pub fn stop_redo_ins() {
    inp_st().block_redo = false;
}

fn init_typebuf() {
    unsafe {
        let p = &raw const g_st().typebuf.tb_buf;
        if !(*p).is_empty() {
            return;
        }

        g_st().typebuf.tb_buf = vec![0u8; TYPELEN_INIT];
        g_st().typebuf.tb_noremap = vec![0u8; TYPELEN_INIT];
        g_st().typebuf.tb_buflen = TYPELEN_INIT as c_int;
        g_st().typebuf.tb_len = 0;
        g_st().typebuf.tb_off = (MAXMAPLEN + 4) as c_int;
        g_st().typebuf.tb_change_cnt = 1;
    }
}

/// Returns true when keys cannot be remapped.
pub fn noremap_keys() -> bool {
    (inp_st().KeyNoremap & ((RM_NONE | RM_SCRIPT) as c_int)) != 0
}

/// Insert a string in position `offset` in the typeahead buffer.
pub fn ins_typebuf(
    str_: impl crate::strings::IntoCCharPtr,
    noremap: c_int,
    offset: c_int,
    nottyped: bool,
    silent: bool,
) -> bool {
    let str_ = str_.into_c_char_ptr().cast_mut();
    unsafe {
        init_typebuf();
        g_st().typebuf.tb_change_cnt = g_st().typebuf.tb_change_cnt.wrapping_add(1);
        if g_st().typebuf.tb_change_cnt == 0 {
            g_st().typebuf.tb_change_cnt = 1;
        }
        state_no_longer_safe(c"ins_typebuf()");

        let addlen = crate::memory::cstr_len(str_) as c_int;

        if offset == 0 && addlen <= g_st().typebuf.tb_off {
            g_st().typebuf.tb_off -= addlen;
            ptr::copy(
                str_.cast::<u8>(),
                tb_buf_ptr().add(g_st().typebuf.tb_off as usize),
                addlen as usize,
            );
        } else if g_st().typebuf.tb_len == 0
            && g_st().typebuf.tb_buflen >= addlen + (3 * (MAXMAPLEN + 4)) as c_int
        {
            g_st().typebuf.tb_off = (g_st().typebuf.tb_buflen - addlen - (3 * (MAXMAPLEN + 4)) as c_int) / 2;
            ptr::copy(
                str_.cast::<u8>(),
                tb_buf_ptr().add(g_st().typebuf.tb_off as usize),
                addlen as usize,
            );
        } else {
            let newoff = (MAXMAPLEN + 4) as c_int;
            let extra = addlen + newoff + (4 * (MAXMAPLEN + 4)) as c_int;
            if g_st().typebuf.tb_len > c_int::MAX - extra {
                emsg(gettext(e_command_too_complex));
                setcursor();
                return false;
            }
            let newlen = g_st().typebuf.tb_len + extra;
            let mut s1_vec = vec![0u8; newlen as usize];
            let mut s2_vec = vec![0u8; newlen as usize];
            let s1 = s1_vec.as_mut_ptr();
            let s2 = s2_vec.as_mut_ptr();
            g_st().typebuf.tb_buflen = newlen;

            ptr::copy(
                tb_buf_ptr().add(g_st().typebuf.tb_off as usize),
                s1.add(newoff as usize),
                offset as usize,
            );
            ptr::copy(
                str_.cast::<u8>(),
                s1.add((newoff + offset) as usize),
                addlen as usize,
            );
            let bytes = g_st().typebuf.tb_len - offset + 1;
            assert!(bytes > 0);
            ptr::copy(
                tb_buf_ptr().add((g_st().typebuf.tb_off + offset) as usize),
                s1.add((newoff + offset + addlen) as usize),
                bytes as usize,
            );
            g_st().typebuf.tb_buf = s1_vec;

            ptr::copy(
                tb_noremap_ptr().add(g_st().typebuf.tb_off as usize),
                s2.add(newoff as usize),
                offset as usize,
            );
            ptr::copy(
                tb_noremap_ptr().add((g_st().typebuf.tb_off + offset) as usize),
                s2.add((newoff + offset + addlen) as usize),
                (g_st().typebuf.tb_len - offset) as usize,
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
        for i in 0..addlen {
            nrm -= 1;
            *tb_noremap_ptr().add((g_st().typebuf.tb_off + i + offset) as usize) =
                if nrm >= 0 { val } else { RM_YES };
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
}

/// Re-queues key `c`: puts a key obtained by `vgetc()` back into typeahead.
pub fn requeue_key(c: c_int, modifiers: c_int, extra_consumed: c_int, recorded: bool) {
    let mut buf = [0 as c_char; MB_MAXBYTES * 3 + 4];
        let len = special_to_buf(c, modifiers, true, buf.as_mut_ptr()) as usize;
        assert!(len < buf.len());
        buf[len] = NUL as c_char;
        ins_typebuf(buf.as_mut_ptr(), inp_st().KeyNoremap, 0, !g_st().KeyTyped, g_st().cmd_silent);
        ungetchars(len as c_int + extra_consumed, recorded);
}

/// Return true if the typeahead buffer was changed.
pub fn typebuf_changed(tb_change_cnt: c_int) -> bool {
    tb_change_cnt != 0 && (g_st().typebuf.tb_change_cnt != tb_change_cnt || g_st().typebuf_was_filled)
}

/// Return true if there are no characters in the typeahead buffer that have not been typed.
pub fn typebuf_typed() -> c_int {
    c_int::from(g_st().typebuf.tb_maplen == 0)
}

/// Get the number of characters that are mapped (or not typed).
pub fn typebuf_maplen() -> c_int {
    g_st().typebuf.tb_maplen
}

/// Remove `len` characters from `typebuf.tb_buf[typebuf.tb_off + offset]`.
pub fn del_typebuf(len: c_int, offset: c_int) {
    unsafe {
        if len == 0 {
            return;
        }

        g_st().typebuf.tb_len -= len;

        if offset == 0 && g_st().typebuf.tb_buflen - (g_st().typebuf.tb_off + len) >= (3 * MAXMAPLEN + 3) as c_int
        {
            g_st().typebuf.tb_off += len;
        } else {
            let i = g_st().typebuf.tb_off + offset;
            if g_st().typebuf.tb_off > MAXMAPLEN as c_int {
                ptr::copy(
                    tb_buf_ptr().add(g_st().typebuf.tb_off as usize),
                    tb_buf_ptr().add(MAXMAPLEN),
                    offset as usize,
                );
                ptr::copy(
                    tb_noremap_ptr().add(g_st().typebuf.tb_off as usize),
                    tb_noremap_ptr().add(MAXMAPLEN),
                    offset as usize,
                );
                g_st().typebuf.tb_off = MAXMAPLEN as c_int;
            }
            let bytes = g_st().typebuf.tb_len - offset + 1;
            assert!(bytes > 0);
            ptr::copy(
                tb_buf_ptr().add((i + len) as usize),
                tb_buf_ptr().add((g_st().typebuf.tb_off + offset) as usize),
                bytes as usize,
            );
            ptr::copy(
                tb_noremap_ptr().add((i + len) as usize),
                tb_noremap_ptr().add((g_st().typebuf.tb_off + offset) as usize),
                (g_st().typebuf.tb_len - offset) as usize,
            );
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
}

fn gotchars_add_byte(state: &mut gotchars_state_T, byte: u8) -> bool {
    state.buf[state.buflen] = byte;
        state.buflen += 1;
        let mut c = byte as c_int;
        let mut retval = false;
        let in_special = state.pending_special > 0;
        let in_mbyte = state.pending_mbyte > 0;

        'ret_false: {
            if in_special {
                state.pending_special -= 1;
            } else if c == K_SPECIAL as c_int {
                state.pending_special = 2;
            }

            if state.pending_special > 0 {
                break 'ret_false;
            }

            if in_mbyte {
                state.pending_mbyte -= 1;
            } else {
                if in_special {
                    if state.prev_c == KS_MODIFIER as c_int {
                        break 'ret_false;
                    }
                    c = to_special(state.prev_c, c);
                    if c == K_COMPLETE_DELAY {
                        state.buflen = 0;
                    }
                }
                state.pending_mbyte = (mb_byte2len_check(c) - 1) as c_uint;
            }

            if state.pending_mbyte > 0 {
                break 'ret_false;
            }

            retval = true;
        }
        state.prev_c = c;
        retval
}

fn gotchars(chars: *const u8, len: usize) {
    unsafe {
        let mut s = chars;
        let mut todo = len;

        while todo > 0 {
            todo -= 1;
            let b = *s;
            s = s.add(1);
            if !gotchars_add_byte(&mut inp_st().GOTCHARS_STATE, b) {
                continue;
            }

            let buflen = inp_st().GOTCHARS_STATE.buflen;
            for i in 0..buflen {
                updatescript(inp_st().GOTCHARS_STATE.buf[i] as c_int);
            }

            if buflen > inp_st().on_key_ignore_len {
                let start_ptr = (&raw const inp_st().GOTCHARS_STATE.buf)
                    .cast::<c_char>()
                    .add(inp_st().on_key_ignore_len);
                let okb = &raw mut inp_st().on_key_buf;
                (*okb).extend_from_slice(std::slice::from_raw_parts(
                    start_ptr,
                    buflen - inp_st().on_key_ignore_len,
                ));
                inp_st().on_key_ignore_len = 0;
            } else {
                inp_st().on_key_ignore_len -= buflen;
            }

            if g_st().reg_recording != 0 {
                let rb = &raw mut inp_st().recordbuff;
                (*rb).extend_from_slice(std::slice::from_raw_parts(
                    (&raw const inp_st().GOTCHARS_STATE.buf).cast::<c_char>(),
                    buflen,
                ));
                inp_st().last_recorded_len = inp_st().last_recorded_len.wrapping_add(buflen);
            }

            atom_typed_add((&raw const inp_st().GOTCHARS_STATE.buf).cast::<u8>(), buflen);

            inp_st().GOTCHARS_STATE.buflen = 0;
        }

        may_sync_undo();
        g_st().debug_did_msg = false;
        g_st().maptick += 1;
    }
}

/// Record an `<Ignore>` key.
pub fn gotchars_ignore() {
    let nop_buf: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_IGNORE];
        inp_st().on_key_ignore_len += 3;
        gotchars(nop_buf.as_ptr(), 3);
}

fn ungetchars(len: c_int, recorded: bool) {
    unsafe {
        if !g_st().KeyTyped {
            return;
        }
        let len_u = len as usize;
        let rb = &raw mut inp_st().recordbuff;
        if recorded && g_st().reg_recording != 0 && (*rb).len() >= len_u {
            (*rb).truncate((*rb).len() - len_u);
            inp_st().last_recorded_len = inp_st().last_recorded_len.wrapping_sub(len_u);
        }
        atom_typed_del(len_u);
        let okb = &raw mut inp_st().on_key_buf;
        let trim = len_u.min((*okb).len());
        (*okb).truncate((*okb).len() - trim);
        inp_st().on_key_ignore_len += len_u - trim;
    }
}

/// Sync undo when typed characters are obtained from the typeahead buffer.
pub fn may_sync_undo() {
    if ((g_st().State & (MODE_INSERT | MODE_CMDLINE)) == 0 || g_st().Ins.moved != kInsNone) && inp_st().curscript < 0 {
            u_sync(false);
        }
}

fn alloc_typebuf() {
    g_st().typebuf.tb_buf = vec![0u8; TYPELEN_INIT];
        g_st().typebuf.tb_noremap = vec![0u8; TYPELEN_INIT];
        g_st().typebuf.tb_buflen = TYPELEN_INIT as c_int;
        g_st().typebuf.tb_off = (MAXMAPLEN + 4) as c_int;
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
    assert!(inp_st().curscript >= 0);
        init_typebuf();
        let prev_cnt = g_st().typebuf.tb_change_cnt;
        inp_st().saved_typebuf[inp_st().curscript as usize] =
            std::mem::replace(&mut g_st().typebuf, typebuf_T::ZERO);
        g_st().typebuf.tb_change_cnt = prev_cnt;
        alloc_typebuf();
}

fn can_get_ungot() -> bool {
    inp_st().ungot.c != -1 && (inp_st().ungot.stuffed || stuff_empty())
}

/// Save all three kinds of typeahead, so that the user must type at a prompt.
pub fn save_typeahead(tp: &mut tasave_T) {
    unsafe {
        let prev_cnt = g_st().typebuf.tb_change_cnt;
        tp.save_typebuf =
            std::mem::replace(&mut g_st().typebuf, typebuf_T::ZERO);
        g_st().typebuf.tb_change_cnt = prev_cnt;
        alloc_typebuf();
        tp.ungot = inp_st().ungot;
        inp_st().ungot.c = -1;

        let rb1 = &raw mut inp_st().readbuf1;
        let rb2 = &raw mut inp_st().readbuf2;
        tp.save_readbuf1 = std::mem::replace(&mut *rb1, StuffBuf::new());
        tp.save_readbuf2 = std::mem::replace(&mut *rb2, StuffBuf::new());
    }
}

/// Restore the typeahead to what it was before calling `save_typeahead()`.
pub fn restore_typeahead(tp: &mut tasave_T) {
    unsafe {
        free_typebuf();
        g_st().typebuf = std::mem::replace(&mut tp.save_typebuf, typebuf_T::ZERO);
        inp_st().ungot = tp.ungot;

        let rb1 = &raw mut inp_st().readbuf1;
        let rb2 = &raw mut inp_st().readbuf2;
        *rb1 = std::mem::replace(&mut tp.save_readbuf1, StuffBuf::new());
        *rb2 = std::mem::replace(&mut tp.save_readbuf2, StuffBuf::new());
    }
}

/// Open a new script file for the `":source!"` command.
pub fn openscript(name: *mut c_char, directly: bool) {
    unsafe {
        if inp_st().curscript + 1 == NSCRIPT as c_int {
            emsg(gettext(e_nesting));
            return;
        }

        if check_secure() {
            return;
        }

        if g_st().ignore_script {
            return;
        }

        inp_st().curscript += 1;
        let namebuff_ptr = (&raw mut g_st().NameBuff).cast::<c_char>();
        expand_env(name, namebuff_ptr, MAXPATHL as c_int);
        let scriptin_ptr = (&raw mut inp_st().scriptin)
            .cast::<FileDescriptor>()
            .add(inp_st().curscript as usize);
        let error = file_open(&mut *scriptin_ptr, namebuff_ptr, K_FILE_READ_ONLY, 0);
        if error != 0 {
            semsg!(
                gettext(e_notopen_2),
                name,
                uv_strerror(error),
            );
            inp_st().curscript -= 1;
            return;
        }
        save_typebuf();

        if directly {
            let mut oa = oparg_T::new();
            let save_State = g_st().State;
            let save_restart_edit = g_st().restart_edit;
            let save_finish_op = g_st().finish_op;
            let save_msg_scroll = g_st().msg_scroll;

            g_st().State = MODE_NORMAL;
            g_st().msg_scroll = 0;
            g_st().restart_edit = 0;
            g_st().finish_op = false;

            let oldcurscript = inp_st().curscript;
            loop {
                update_topline_cursor();
                normal_cmd(&mut oa, false);
                vpeekc();
                if inp_st().curscript < oldcurscript {
                    break;
                }
            }

            g_st().State = save_State;
            g_st().msg_scroll = save_msg_scroll;
            g_st().restart_edit = save_restart_edit;
            g_st().finish_op = save_finish_op;
        }
    }
}

fn closescript() {
    unsafe {
        assert!(inp_st().curscript >= 0);
        free_typebuf();
        g_st().typebuf = std::mem::replace(
            &mut inp_st().saved_typebuf[inp_st().curscript as usize],
            typebuf_T::ZERO,
        );

        let scriptin_ptr = (&raw mut inp_st().scriptin)
            .cast::<FileDescriptor>()
            .add(inp_st().curscript as usize);
        file_close(&mut *scriptin_ptr, false);
        inp_st().curscript -= 1;
    }
}

pub fn close_all_scripts() {
    while inp_st().curscript >= 0 {
            closescript();
        }
}

pub fn open_scriptin(scriptin_name: *mut c_char) -> bool {
    unsafe {
        assert!(inp_st().curscript == -1);
        inp_st().curscript += 1;

        let scriptin0 = (&raw mut inp_st().scriptin).cast::<FileDescriptor>();
        let error = if crate::memory::cstr_as_bytes(scriptin_name) == b"-" {
            file_open_stdin(&mut *scriptin0)
        } else {
            file_open(
                &mut *scriptin0,
                scriptin_name,
                K_FILE_READ_ONLY | K_FILE_NON_BLOCKING,
                0,
            )
        };
        if error != 0 {
            let mut errbuf = [0 as c_char; 1024];
            let n = vim_snprintf!(
                errbuf.as_mut_ptr(),
                errbuf.len(),
                gettext(c"Cannot open for reading: \"%s\": %s\n"),
                scriptin_name,
                uv_strerror(error),
            );
            if n > 0 {
                let slice = std::slice::from_raw_parts(
                    errbuf.as_ptr().cast::<u8>(),
                    (n as usize).min(errbuf.len() - 1),
                );
                let _ = std::io::Write::write_all(&mut std::io::stderr(), slice);
            }
            inp_st().curscript -= 1;
            return false;
        }
        save_typebuf();

        true
    }
}

/// Return true when reading keys from a script file.
pub fn using_script() -> c_int {
    c_int::from(inp_st().curscript >= 0)
}

/// Called just before doing a blocking wait.
pub fn before_blocking() {
    updatescript(0);
        if g_st().may_garbage_collect {
            garbage_collect(false);
        }
}

fn updatescript(c: c_int) {
    unsafe {
        if c != 0
            && let Some(ref so) = g_st().scriptout
        {
            fputc(c, so.as_ptr());
        }
        let idle = c == 0;
        if idle
            || (opt_vars().p_uc > 0 && {
                inp_st().UPDATESCRIPT_COUNT += 1;
                (inp_st().UPDATESCRIPT_COUNT as OptInt) >= opt_vars().p_uc
            })
        {
            ml_sync_all(c_int::from(idle), 1, opt_vars().p_fs != 0 || idle);
            inp_st().UPDATESCRIPT_COUNT = 0;
        }
    }
}

/// Merge `modifiers` into `c_arg`.
pub fn merge_modifiers(c_arg: c_int, modifiers: &mut c_int) -> c_int {
    let mut c = c_arg;

        if (*modifiers & (MOD_MASK_CTRL as c_int)) != 0 {
            if c >= b'@' as c_int && c <= 0x7f {
                c &= 0x1f;
                if c == NUL {
                    c = K_ZERO;
                }
            } else if c == b'6' as c_int {
                c = 0x1e;
            }
            if c != c_arg {
                *modifiers &= !(MOD_MASK_CTRL as c_int);
            }
        }
        c
}

fn add_byte_to_showcmd(byte: u8) {
    unsafe {
        if opt_vars().p_sc == 0 || g_st().msg_silent != 0 {
            return;
        }

        if !gotchars_add_byte(&mut inp_st().SHOWCMD_STATE, byte) {
            return;
        }

        let buflen = inp_st().SHOWCMD_STATE.buflen;
        inp_st().SHOWCMD_STATE.buf[buflen] = NUL as u8;
        inp_st().SHOWCMD_STATE.buflen = 0;

        let mut modifiers: c_int = 0;
        let mut c: c_int = NUL;

        let mut ptr = (&raw const inp_st().SHOWCMD_STATE.buf).cast::<u8>();
        if *ptr == K_SPECIAL && *ptr.add(1) == KS_MODIFIER && *ptr.add(2) != NUL as u8 {
            modifiers = *ptr.add(2) as c_int;
            ptr = ptr.add(3);
        }

        if *ptr != NUL as u8 {
            let mut cptr = ptr.cast::<c_char>();
            let mb_ptr = mb_unescape(&raw mut cptr);
            ptr = cptr.cast::<u8>();
            if !mb_ptr.is_null() {
                c = utf_ptr2char(mb_ptr);
            } else {
                c = *ptr as c_int;
                ptr = ptr.add(1);
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
            add_to_showcmd(K_SPECIAL as c_int);
            add_to_showcmd(KS_MODIFIER as c_int);
            add_to_showcmd(modifiers);
        }
        if c != NUL {
            add_to_showcmd(c);
        }
        while *ptr != NUL as u8 {
            add_to_showcmd(*ptr as c_int);
            ptr = ptr.add(1);
        }
    }
}

/// Get the next input character.
pub fn vgetc() -> c_int {
    unsafe {
        let mut c: c_int;
        let mut buf = [0u8; MB_MAXBYTES + 1];

        if g_st().may_garbage_collect && g_st().want_garbage_collect {
            garbage_collect(false);
        }

        if can_get_ungot() {
            c = inp_st().ungot.c;
            inp_st().ungot.c = -1;
            g_st().mod_mask = inp_st().ungot.mod_mask;
            g_st().mouse_grid = inp_st().ungot.mouse_grid;
            g_st().mouse_row = inp_st().ungot.mouse_row;
            g_st().mouse_col = inp_st().ungot.mouse_col;
        } else {
            g_st().mod_mask = 0;
            g_st().vgetc_mod_mask = 0;
            g_st().vgetc_char = 0;

            inp_st().last_recorded_len = inp_st().last_recorded_len.wrapping_sub(inp_st().LAST_VGETC_RECORDED_LEN);

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

                if c == K_SPECIAL as c_int {
                    let save_allow_keys = g_st().allow_keys;
                    g_st().no_mapping += 1;
                    g_st().allow_keys = 0;
                    let c2 = vgetorpeek(true);
                    c = vgetorpeek(true);
                    g_st().no_mapping -= 1;
                    g_st().allow_keys = save_allow_keys;
                    if c2 == KS_MODIFIER as c_int {
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
                    c = utf_ptr2char(buf.as_ptr().cast());
                }

                if g_st().no_mapping == 0
                    && g_st().KeyTyped
                    && g_st().mod_mask == MOD_MASK_ALT as c_int
                    && (g_st().State & MODE_TERMINAL) == 0
                    && !is_mouse_key(c)
                {
                    g_st().mod_mask = 0;
                    requeue_key(c, 0, 3, true);
                    ins_typebuf(
                        ESC_STR,
                        inp_st().KeyNoremap,
                        0,
                        !g_st().KeyTyped,
                        g_st().cmd_silent,
                    );
                    continue;
                }

                if g_st().vgetc_char == 0 {
                    g_st().vgetc_mod_mask = g_st().mod_mask;
                    g_st().vgetc_char = c;
                }

                match c {
                    K_KPLUS => c = b'+' as c_int,
                    K_KMINUS => c = b'-' as c_int,
                    K_KDIVIDE => c = b'/' as c_int,
                    K_KMULTIPLY => c = b'*' as c_int,
                    K_KENTER => c = CAR,
                    K_KPOINT => c = b'.' as c_int,
                    K_KCOMMA => c = b',' as c_int,
                    K_KEQUAL => c = b'=' as c_int,
                    K_K0 => c = b'0' as c_int,
                    K_K1 => c = b'1' as c_int,
                    K_K2 => c = b'2' as c_int,
                    K_K3 => c = b'3' as c_int,
                    K_K4 => c = b'4' as c_int,
                    K_K5 => c = b'5' as c_int,
                    K_K6 => c = b'6' as c_int,
                    K_K7 => c = b'7' as c_int,
                    K_K8 => c = b'8' as c_int,
                    K_K9 => c = b'9' as c_int,
                    K_XHOME | K_ZHOME => {
                        if g_st().mod_mask == MOD_MASK_SHIFT as c_int {
                            c = K_S_HOME;
                            g_st().mod_mask = 0;
                        } else if g_st().mod_mask == MOD_MASK_CTRL as c_int {
                            c = K_C_HOME;
                            g_st().mod_mask = 0;
                        } else {
                            c = K_HOME;
                        }
                    }
                    K_XEND | K_ZEND => {
                        if g_st().mod_mask == MOD_MASK_SHIFT as c_int {
                            c = K_S_END;
                            g_st().mod_mask = 0;
                        } else if g_st().mod_mask == MOD_MASK_CTRL as c_int {
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

            inp_st().LAST_VGETC_RECORDED_LEN = inp_st().last_recorded_len;
        }

        g_st().may_garbage_collect = false;

        let okb = &raw mut inp_st().on_key_buf;
        (*okb).push(NUL as c_char);
        if inp_st().on_key_blocked == 0 && nlua_exec_on_key(c, (*okb).as_mut_ptr()) {
            if c == K_COMMAND {
                let _ = getcmdkeycmd(NUL, 0, false);
            } else if c == K_LUA {
                map_execute_lua(false, true);
            } else if c == K_PASTE_START {
                paste_repeat(0);
            }
            c = K_IGNORE;
        }
        *okb = Vec::new();

        if c != K_IGNORE {
            state_no_longer_safe(c"key typed");
        }

        c
    }
}

/// Like `vgetc()`, but never return a NUL when called recursively.
pub fn safe_vgetc() -> c_int {
    let mut c = vgetc();
        if c == NUL {
            c = get_keystroke(ptr::null_mut());
        }
        c
}

/// Like `safe_vgetc()`, but loop to handle `K_IGNORE`.
pub fn plain_vgetc() -> c_int {
    loop {
            let c = safe_vgetc();
            if c != K_IGNORE && c != K_VER_SCROLLBAR && c != K_HOR_SCROLLBAR && c != K_MOUSEMOVE {
                return c;
            }
        }
}

/// Check if a character is available, such that `vgetc()` will not block.
pub fn vpeekc() -> c_int {
    if can_get_ungot() {
            return inp_st().ungot.c;
        }
        vgetorpeek(false)
}

/// Check if any character is available, also half an escape sequence.
pub fn vpeekc_any() -> c_int {
    let mut c = vpeekc();
        if c == NUL && g_st().typebuf.tb_len > 0 {
            c = ESC;
        }
        c
}

/// Call `vpeekc()` without causing anything to be mapped.
pub fn char_avail() -> bool {
    unsafe {
        if *test_disable_char_avail.as_ptr() {
            return false;
        }
        g_st().no_mapping += 1;
        let retval = vpeekc();
        g_st().no_mapping -= 1;
        retval != NUL
    }
}

fn getchar_common(argvars: &mut [typval_T], rettv: &mut typval_T, mut allow_number: bool) {
    unsafe {
        let mut n: varnumber_T;
        let called_emsg_start = g_st().called_emsg;
        let mut error = false;
        let mut simplify = true;
        let mut cursor_flag: c_char = NUL as c_char;

        if argvars[0].v_type != VarType::VAR_UNKNOWN
            && !tv_check_for_opt_dict_arg(argvars, 1)
        {
            return;
        }

        let arg1 = (&raw mut argvars[1]);
        if argvars[0].v_type != VarType::VAR_UNKNOWN && (*arg1).v_type == VarType::VAR_DICT {
            let d = (*arg1).vval.v_dict();

            if allow_number {
                allow_number = tv_dict_get_bool(d, c"number", 1) != 0;
            } else if tv_dict_has_key(d, c"number") {
                semsg!(gettext(e_invarg2), c"number");
            }

            simplify = tv_dict_get_bool(d, c"simplify", 1) != 0;

            let cursor_str = tv_dict_get_string(d, c"cursor");
            if !cursor_str.is_null() {
                if crate::memory::cstr_as_bytes(cursor_str) != b"hide"
                    && crate::memory::cstr_as_bytes(cursor_str) != b"keep"
                    && crate::memory::cstr_as_bytes(cursor_str) != b"msg"
                {
                    semsg!(
                        gettext(e_invargNval),
                        c"cursor",
                        cursor_str,
                    );
                } else {
                    cursor_flag = *cursor_str;
                }
            }
        }

        if g_st().called_emsg != called_emsg_start {
            return;
        }

        if cursor_flag == b'h' as c_char {
            ui_busy_start();
        }

        g_st().no_mapping += 1;
        g_st().allow_keys += 1;
        block_on_key();
        if !simplify {
            inp_st().no_reduce_keys += 1;
        }
        atom_payload_start();
        loop {
            if cursor_flag == b'm' as c_char || (cursor_flag == NUL as c_char && g_st().msg_col > 0) {
                ui_cursor_goto(g_st().msg_row, g_st().msg_col);
            }

            if argvars[0].v_type == VarType::VAR_UNKNOWN
                || (argvars[0].v_type == VarType::VAR_NUMBER && argvars[0].vval.v_number() == -1)
            {
                if !char_avail() {
                    ui_flush();
                    let events = main_loop_events();
                    input_get(ptr::null_mut(), 0, -1, g_st().typebuf.tb_change_cnt, events);
                    if input_available() == 0 && !multiqueue_empty(events) {
                        state_handle_k_event();
                        continue;
                    }
                }
                n = varnumber_T::from(safe_vgetc());
            } else if tv_get_bool_chk(&argvars[0], Some(&mut error)) != 0 {
                n = varnumber_T::from(vpeekc_any());
            } else if error || vpeekc_any() == NUL {
                n = 0;
            } else {
                n = varnumber_T::from(safe_vgetc());
            }

            let ni = n as c_int;
            if ni == K_IGNORE || ni == K_MOUSEMOVE || ni == K_VER_SCROLLBAR || ni == K_HOR_SCROLLBAR
            {
                continue;
            }
            break;
        }
        atom_payload_end();
        g_st().no_mapping -= 1;
        g_st().allow_keys -= 1;
        unblock_on_key();
        if !simplify {
            inp_st().no_reduce_keys -= 1;
        }

        if cursor_flag == b'h' as c_char {
            ui_busy_stop();
        }

        set_vim_var_nr(VV_MOUSE_WIN, 0);
        set_vim_var_nr(VV_MOUSE_WINID, 0);
        set_vim_var_nr(VV_MOUSE_LNUM, 0);
        set_vim_var_nr(VV_MOUSE_COL, 0);

        let ni = n as c_int;
        if n != 0 && (!allow_number || is_special(ni) || g_st().mod_mask != 0) {
            let mut temp = [0 as c_char; 10];
            let mut i: usize = 0;

            if g_st().mod_mask != 0 {
                temp[i] = K_SPECIAL as c_char;
                temp[i + 1] = KS_MODIFIER as c_char;
                temp[i + 2] = g_st().mod_mask as c_char;
                i += 3;
            }
            if is_special(ni) {
                temp[i] = K_SPECIAL as c_char;
                temp[i + 1] = k_second(ni) as c_char;
                temp[i + 2] = k_third(ni) as c_char;
                i += 3;
            } else {
                i += utf_char2bytes(ni, temp.as_mut_ptr().add(i)) as usize;
            }
            assert!(i < 10);
            temp[i] = NUL as c_char;
            crate::eval::typval::tv_set_string_len(&mut *rettv, temp.as_ptr(), i);

            if is_mouse_key(ni) {
                let mut row = g_st().mouse_row;
                let mut col = g_st().mouse_col;
                let mut grid = g_st().mouse_grid;
                let mut lnum: linenr_T = 0;

                if row >= 0 && col >= 0 {
                    let mut winnr: c_int = 1;
                    let win = mouse_find_win_inner(&mut grid, &mut row, &mut col);
                    if win.is_null() {
                        return;
                    }
                    mouse_comp_pos(&mut *win, &mut row, &mut col, &mut lnum);
                    let mut wp = g_st().firstwin;
                    while wp != win {
                        winnr += 1;
                        wp = (*wp).w_next;
                    }
                    set_vim_var_nr(VV_MOUSE_WIN, varnumber_T::from(winnr));
                    set_vim_var_nr(VV_MOUSE_WINID, varnumber_T::from((*wp).handle));
                    set_vim_var_nr(VV_MOUSE_LNUM, varnumber_T::from(lnum));
                    set_vim_var_nr(VV_MOUSE_COL, varnumber_T::from(col + 1));
                }
            }
        } else if !allow_number {
            rettv.v_type = VarType::VAR_STRING;
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(n);
        }
    }
}

/// `"getchar()"` function.
pub fn f_getchar(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    getchar_common(argvars, rettv, true);
}

/// `"getcharstr()"` function.
pub fn f_getcharstr(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    getchar_common(argvars, rettv, false);
}

/// `"getcharmod()"` function.
pub fn f_getcharmod(_argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(g_st().mod_mask));
}

fn put_string_in_typebuf(
    offset: c_int,
    slen: c_int,
    string: *mut u8,
    new_slen: c_int,
) -> bool {
    unsafe {
        let extra = new_slen - slen;
        *string.add(new_slen as usize) = NUL as u8;
        if extra < 0 {
            del_typebuf(-extra, offset);
        } else if extra > 0
            && !ins_typebuf(
                string.add(slen as usize).cast::<c_char>(),
                REMAP_YES,
                offset,
                false,
                false,
            )
        {
            return false;
        }
        ptr::copy(
            string,
            tb_buf_ptr().add((g_st().typebuf.tb_off + offset) as usize),
            new_slen as usize,
        );
        true
    }
}

fn at_ins_compl_key() -> bool {
    unsafe {
        let p = tb_buf_ptr().add(g_st().typebuf.tb_off as usize);
        let mut c = *p as c_int;

        if g_st().typebuf.tb_len > 3
            && c == K_SPECIAL as c_int
            && *p.add(1) == KS_MODIFIER
            && (*p.add(2) & (MOD_MASK_CTRL as u8)) != 0
        {
            c = (*p.add(3) & 0x1f) as c_int;
        }
        !ctrl_x_mode_eval()
            && ((ctrl_x_mode_not_default() && vim_is_ctrl_x_key(c))
                || (compl_status_local() && (c == Ctrl_N || c == Ctrl_P)))
    }
}

fn check_simplify_modifier(max_offset: c_int) -> c_int {
    unsafe {
        if (g_st().State & MODE_TERMINAL) != 0 || inp_st().no_reduce_keys > 0 {
            return 0;
        }

        for offset in 0..max_offset {
            if offset + 3 >= g_st().typebuf.tb_len {
                break;
            }
            let tp = tb_buf_ptr().add((g_st().typebuf.tb_off + offset) as usize);
            if *tp == K_SPECIAL && *tp.add(1) == KS_MODIFIER {
                let mut modifier = *tp.add(2) as c_int;
                let c = *tp.add(3) as c_int;
                let new_c = merge_modifiers(c, &mut modifier);

                if new_c != c {
                    if offset == 0 {
                        g_st().vgetc_char = c;
                        g_st().vgetc_mod_mask = *tp.add(2) as c_int;
                    }
                    let mut new_string = [0u8; MB_MAXBYTES + 1];
                    let len = if is_special(new_c) {
                        new_string[0] = K_SPECIAL;
                        new_string[1] = k_second(new_c);
                        new_string[2] = k_third(new_c);
                        3
                    } else {
                        utf_char2bytes(new_c, new_string.as_mut_ptr().cast())
                    };
                    if modifier == 0 {
                        if !put_string_in_typebuf(offset, 4, new_string.as_mut_ptr(), len) {
                            return -1;
                        }
                    } else {
                        *tp.add(2) = modifier as u8;
                        if !put_string_in_typebuf(offset + 3, 1, new_string.as_mut_ptr(), len)
                        {
                            return -1;
                        }
                    }
                    return len;
                }
            }
        }
        0
    }
}

fn char_iter(itp: *mut *const u8, nomap: c_int) -> c_int {
    unsafe {
        let mut it = *itp;
        let mut c = *it as c_int;
        if c == NUL {
            return 0;
        }

        if nomap == 0 {
            let mblen = utf_byte2len(c);
            if mblen > 1 {
                let mut buf = [0u8; MB_MAXBYTES + 1];
                buf[0] = c as u8;
                let mut i: c_int = 1;

                loop {
                    it = it.add(1);
                    buf[i as usize] = *it;
                    if buf[i as usize] == K_SPECIAL {
                        it = it.add(1);
                        if *it == NUL as u8 {
                            break;
                        }
                        it = it.add(1);
                        if *it == NUL as u8 {
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

                c = utf_ptr2char(buf.as_ptr().cast());
                if c == buf[0] as c_int {
                    it = *itp;
                }
            }
        }

        it = it.add(1);
        *itp = it;
        c
    }
}

fn handle_mapping(
    keylenp: &mut c_int,
    timedout: &bool,
    mapdepth: &mut c_int,
    advance: bool,
) -> c_int {
    unsafe {
        let mut mp: Option<&mapblock_T> = None;
        let mut mp_match: Option<&mapblock_T> = None;
        let mut mp_match_len: c_int = 0;
        let mut tb_match_len: c_int = 0;
        let mut max_mlen: c_int = 0;
        let mut keylen = *keylenp;
        let local_State = get_real_state();
        let mut is_plug_map = false;

        let tb_start = tb_buf_ptr().add(g_st().typebuf.tb_off as usize);
        if g_st().typebuf.tb_len >= 3
            && *tb_start == K_SPECIAL
            && *tb_start.add(1) == KS_EXTRA
            && *tb_start.add(2) == KE_PLUG
        {
            is_plug_map = true;
        }

        let mut tb_b1 = *tb_start as c_int;
        if g_st().no_mapping == 0
            && (g_st().no_zero_mapping == 0 || tb_b1 != b'0' as c_int)
            && (g_st().typebuf.tb_maplen == 0
                || is_plug_map
                || (*tb_noremap_ptr().add(g_st().typebuf.tb_off as usize) & (RM_NONE | RM_ABBR)) == 0)
            && !(opt_vars().p_paste != 0 && (g_st().State & (MODE_INSERT | MODE_CMDLINE)) != 0)
            && !(g_st().State == MODE_HITRETURN && (tb_b1 == CAR || tb_b1 == b' ' as c_int))
            && g_st().State != MODE_ASKMORE
            && !at_ins_compl_key()
        {
            let nolmaplen: c_int;
            let mut tb_c1 = tb_b1;
            let mut tb_iter: *const u8 = tb_start.cast_const();

            if tb_b1 == K_SPECIAL as c_int {
                nolmaplen = 2;
                tb_iter = tb_iter.add(1);
            } else {
                tb_c1 = char_iter(&raw mut tb_iter, 0);
                if tb_c1 == 0 {
                    if *timedout {
                        *keylenp = 0;
                        return map_result_T::map_result_get as c_int;
                    }
                    *keylenp = KEYLEN_PART_KEY;
                    return map_result_T::map_result_nomatch as c_int;
                }

                let orig_tb_c1 = tb_c1;
                langmap_adjust(
                    &mut tb_c1,
                    (g_st().State & (MODE_CMDLINE | MODE_INSERT)) == 0 && get_real_state() != MODE_SELECT,
                );
                if orig_tb_c1 != tb_c1 {
                    let mut cbuf = [0 as c_char; 6];
                    utf_char2bytes(tb_c1, cbuf.as_mut_ptr());
                    tb_b1 = cbuf[0] as u8 as c_int;
                }
                nolmaplen = 0;
            }

            mp = None;
            for cand in get_buf_maphash_list(local_State, tb_b1)
                .iter()
                .chain(get_maphash_list(local_State, tb_b1))
            {
                let mut key_iter: *const u8 = cand.m_keys.as_ptr().cast::<u8>();

                if char_iter(&raw mut key_iter, 0) == tb_c1
                    && (cand.m_mode & local_State) != 0
                    && ((cand.m_mode & MODE_LANGMAP) == 0 || g_st().typebuf.tb_maplen == 0)
                {
                    let mut nomap = nolmaplen;
                    let mut modifiers: c_int = 0;
                    let mut tmp_tb_iter = tb_iter;
                    let mut mlen =
                        key_iter.offset_from(cand.m_keys.as_ptr().cast::<u8>()) as c_int;
                    let mut tb_mlen =
                        (tmp_tb_iter.offset_from(tb_buf_ptr()) as c_int) - g_st().typebuf.tb_off;

                    loop {
                        let mut c2 = char_iter(&raw mut tmp_tb_iter, nomap);
                        if c2 == 0 {
                            break;
                        }
                        let tb_nomap = nomap;
                        if nomap > 0 {
                            if nomap == 2 && c2 == KS_MODIFIER as c_int {
                                modifiers = 1;
                            } else if nomap == 1 && modifiers == 1 {
                                modifiers = c2;
                            }
                            nomap -= 1;
                        } else {
                            if c2 == K_SPECIAL as c_int {
                                nomap = 2;
                            } else if merge_modifiers(c2, &mut modifiers) == c2 {
                                langmap_adjust(&mut c2, true);
                            }
                            modifiers = 0;
                        }

                        let key_char = char_iter(&raw mut key_iter, tb_nomap);
                        if key_char != c2 {
                            break;
                        }

                        mlen = key_iter.offset_from(cand.m_keys.as_ptr().cast::<u8>()) as c_int;
                        tb_mlen =
                            (tmp_tb_iter.offset_from(tb_buf_ptr()) as c_int) - g_st().typebuf.tb_off;
                    }

                    keylen = cand.m_keylen;
                    if mlen == keylen || (mlen == g_st().typebuf.tb_len && g_st().typebuf.tb_len < keylen) {
                        let mut s = tb_noremap_ptr().add(g_st().typebuf.tb_off as usize);
                        let m_keys_u8 = cand.m_keys.as_ptr().cast::<u8>();
                        if *s == RM_SCRIPT
                            && (*m_keys_u8 != K_SPECIAL
                                || *m_keys_u8.add(1) != KS_EXTRA
                                || *m_keys_u8.add(2) != KE_SNR)
                        {
                            continue;
                        }

                        let mut n = mlen;
                        while {
                            n -= 1;
                            n >= 0
                        } {
                            if (*s & (RM_NONE | RM_ABBR)) != 0 {
                                break;
                            }
                            s = s.add(1);
                        }
                        if !is_plug_map && n >= 0 {
                            continue;
                        }

                        if keylen > g_st().typebuf.tb_len {
                            if !*timedout && !mp_match.is_some_and(|m| m.m_nowait != 0) {
                                keylen = KEYLEN_PART_MAP;
                                mp = Some(cand);
                                break;
                            }
                        } else if keylen > mp_match_len
                            || (keylen == mp_match_len
                                && mp_match.is_some_and(|m| (m.m_mode & MODE_LANGMAP) == 0)
                                && (cand.m_mode & MODE_LANGMAP) != 0)
                        {
                            mp_match = Some(cand);
                            mp_match_len = keylen;
                            tb_match_len = tb_mlen;
                        }
                    } else {
                        max_mlen = max_mlen.max(mlen);
                    }
                }
            }

            if keylen != KEYLEN_PART_MAP && mp_match.is_some() {
                mp = mp_match;
                keylen = tb_match_len;
            }
        }

        if (mp.is_none() || max_mlen > mp_match_len) && keylen != KEYLEN_PART_MAP {
            if g_st().no_mapping == 0 || g_st().allow_keys != 0 {
                if tb_b1 == K_SPECIAL as c_int
                    && (g_st().typebuf.tb_len < 2
                        || (*tb_buf_ptr().add((g_st().typebuf.tb_off + 1) as usize) == KS_MODIFIER
                            && g_st().typebuf.tb_len < 4))
                {
                    keylen = KEYLEN_PART_KEY;
                } else {
                    keylen = check_simplify_modifier(max_mlen + 1);
                    if keylen < 0 {
                        return map_result_T::map_result_fail as c_int;
                    }
                }
            } else {
                keylen = 0;
            }
            if keylen == 0 && mp.is_none() {
                *keylenp = keylen;
                return map_result_T::map_result_get as c_int;
            }
            if keylen > 0 {
                *keylenp = keylen;
                return map_result_T::map_result_retry as c_int;
            }
            if keylen < 0 {
                assert_eq!(keylen, KEYLEN_PART_KEY);
            } else {
                assert!(mp.is_some());
                keylen = tb_match_len;
            }
        }

        if keylen >= 0 && keylen <= g_st().typebuf.tb_len {
            let mp = mp.unwrap();
            if keylen > g_st().typebuf.tb_maplen && (mp.m_mode & MODE_LANGMAP) == 0 {
                gotchars(
                    tb_buf_ptr().add((g_st().typebuf.tb_off + g_st().typebuf.tb_maplen) as usize),
                    (keylen - g_st().typebuf.tb_maplen) as usize,
                );
                atom_map_start(mp.m_keys.as_ptr(), mp.m_keylen as usize, !advance);
            }

            g_st().cmd_silent = g_st().typebuf.tb_silent > 0;
            del_typebuf(keylen, 0);

            *mapdepth += 1;
            if (*mapdepth as OptInt) >= opt_vars().p_mmd {
                emsg(gettext(E_RECURSIVE_MAPPING));
                if (g_st().State & MODE_CMDLINE) != 0 {
                    redrawcmdline();
                } else {
                    setcursor();
                }
                flush_buffers(FLUSH_MINIMAL);
                *mapdepth = 0;
                *keylenp = keylen;
                return map_result_T::map_result_fail as c_int;
            }

            if g_st().Visual.active && g_st().Visual.select && (mp.m_mode & MODE_VISUAL) != 0 {
                g_st().Visual.select = false;
                ins_typebuf(
                    K_SELECT_STRING.as_ptr().cast::<c_char>().cast_mut(),
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
            let save_alt_m_keys = mp.alt_keys();
            let save_alt_m_keylen = save_alt_m_keys
                .as_ref()
                .map_or(0, |k| k.as_bytes().len() as c_int);

            let owned_map_str: Option<std::ffi::CString> = if save_m_expr {
                let save_vgetc_busy = g_st().vgetc_busy;
                let save_may_garbage_collect = g_st().may_garbage_collect;
                let prev_did_emsg = g_st().did_emsg;

                g_st().vgetc_busy = 0;
                g_st().may_garbage_collect = false;

                let mut res = eval_map_expr(mp, NUL);

                if res.as_ref().is_none_or(|s| s.to_bytes().is_empty()) {
                    if prev_did_emsg != g_st().did_emsg {
                        res = Some(
                            std::ffi::CString::new([K_SPECIAL, KS_EXTRA, KE_IGNORE]).unwrap(),
                        );
                        if (g_st().State & MODE_CMDLINE) != 0 {
                            g_st().msg_didout = true;
                            g_st().msg_row = g_st().msg_row.max(g_st().cmdline_row);
                            redrawcmd();
                        }
                    } else if (g_st().State & (MODE_NORMAL | MODE_INSERT)) != 0 {
                        setcursor();
                    }
                }

                g_st().vgetc_busy = save_vgetc_busy;
                g_st().may_garbage_collect = save_may_garbage_collect;
                res
            } else {
                Some(mp.m_str.clone())
            };
            let map_str: *mut c_char = owned_map_str
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());

            let i = if map_str.is_null() {
                false
            } else {
                if keylen > g_st().typebuf.tb_maplen && (save_m_mode & MODE_LANGMAP) != 0 {
                    gotchars(map_str.cast::<u8>(), crate::memory::cstr_len(map_str));
                }

                let noremap = if save_m_noremap != REMAP_YES {
                    save_m_noremap
                } else if (crate::memory::cstr_prefix_bytes(map_str, keylen as usize) == crate::memory::cstr_prefix_bytes(save_m_keys.as_ptr(), keylen as usize))
                    || save_alt_m_keys.as_ref().is_some_and(|alt| {
                        crate::memory::cstr_prefix_bytes(map_str, save_alt_m_keylen as usize) == crate::memory::cstr_prefix_bytes(alt.as_ptr(), save_alt_m_keylen as usize)
                    })
                {
                    REMAP_SKIP
                } else {
                    REMAP_YES
                };

                ins_typebuf(map_str, noremap, 0, true, g_st().cmd_silent || save_m_silent)
            };
            drop(owned_map_str);
            drop(save_m_keys);
            drop(save_alt_m_keys);
            *keylenp = keylen;
            if !i {
                return map_result_T::map_result_fail as c_int;
            }
            return map_result_T::map_result_retry as c_int;
        }

        *keylenp = keylen;
        map_result_T::map_result_nomatch as c_int
    }
}

/// Unget one character (can only be done once!).
pub fn vungetc(c: c_int) {
    inp_st().ungot = UngotKey {
            c,
            mod_mask: g_st().mod_mask,
            mouse_grid: g_st().mouse_grid,
            mouse_row: g_st().mouse_row,
            mouse_col: g_st().mouse_col,
            stuffed: g_st().KeyStuffed != 0,
        };
}

/// When peeking and not getting a character, `reg_executing` cannot be cleared yet.
pub fn check_end_reg_executing(advance: bool) {
    if g_st().reg_executing != 0 && (g_st().typebuf.tb_maplen == 0 || g_st().pending_end_reg_executing) {
            if advance {
                g_st().reg_executing = 0;
                g_st().pending_end_reg_executing = false;
            } else {
                g_st().pending_end_reg_executing = true;
            }
        }
}

fn vgetorpeek(advance: bool) -> c_int {
    unsafe {
        let mut c: c_int;
        let mut timedout = false;
        let mut mapdepth: c_int = 0;
        let mut mode_deleted = false;

        if g_st().vgetc_busy > 0 && g_st().ex_normal_busy == 0 {
            return NUL;
        }

        g_st().vgetc_busy += 1;

        if advance {
            g_st().KeyStuffed = 0;
            g_st().typebuf_was_empty = false;
        }

        init_typebuf();
        start_stuff();
        check_end_reg_executing(advance);
        loop {
            if inp_st().typeahead_char != 0 {
                c = inp_st().typeahead_char;
                if advance {
                    inp_st().typeahead_char = 0;
                }
            } else {
                c = read_readbuffers(advance);
            }
            if c != NUL && !g_st().got_int {
                if advance {
                    g_st().KeyStuffed = 1;
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
                        if ((g_st().mapped_ctrl_c | curbuf_mut().b_mapped_ctrl_c) & get_real_state()) != 0 {
                            g_st().ctrl_c_interrupts = false;
                        }
                        os_breakcheck();
                        g_st().ctrl_c_interrupts = true;
                    }
                    let mut keylen: c_int = 0;
                    if g_st().got_int {
                        c = inchar(tb_buf_ptr(), g_st().typebuf.tb_buflen - 1, 0);
                        if (c != 0 || g_st().typebuf.tb_maplen != 0)
                            && (g_st().State & (MODE_INSERT | MODE_CMDLINE)) != 0
                        {
                            c = ESC;
                        } else {
                            c = Ctrl_C;
                        }
                        flush_buffers(FLUSH_INPUT);

                        if advance {
                            *tb_buf_ptr() = c as u8;
                            gotchars(tb_buf_ptr(), 1);
                        }
                        g_st().cmd_silent = false;

                        break;
                    } else if g_st().typebuf.tb_len > 0 {
                        let result = handle_mapping(
                            &mut keylen,
                            &timedout,
                            &mut mapdepth,
                            advance,
                        );

                        if result == map_result_T::map_result_retry as c_int {
                            continue;
                        }

                        if result == map_result_T::map_result_fail as c_int {
                            atom_composite_abort();
                            c = -1;
                            break;
                        }

                        if result == map_result_T::map_result_get as c_int {
                            c = *tb_buf_ptr().add(g_st().typebuf.tb_off as usize) as c_int;
                            if advance {
                                g_st().cmd_silent = g_st().typebuf.tb_silent > 0;
                                if g_st().typebuf.tb_maplen > 0 {
                                    g_st().KeyTyped = false;
                                } else {
                                    g_st().KeyTyped = true;
                                    gotchars(tb_buf_ptr().add(g_st().typebuf.tb_off as usize), 1);
                                }
                                inp_st().KeyNoremap =
                                    *tb_noremap_ptr().add(g_st().typebuf.tb_off as usize) as c_int;
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
                        && *tb_buf_ptr().add(g_st().typebuf.tb_off as usize) == ESC as u8
                        && g_st().no_mapping == 0
                        && g_st().ex_normal_busy == 0
                        && g_st().typebuf.tb_maplen == 0
                        && (g_st().State & MODE_INSERT) != 0
                        && (opt_vars().p_timeout != 0 || (keylen == KEYLEN_PART_KEY && opt_vars().p_ttimeout != 0))
                        && {
                            c = inchar(
                                tb_buf_ptr().add((g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize),
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
                        validate_cursor(curwin_ptr());
                        let old_wcol = curwin_mut().w_wcol;
                        let old_wrow = curwin_mut().w_wrow;

                        if curwin_mut().w_cursor.col != 0 {
                            let mut col: colnr_T = 0;
                            if curwin_mut().w_wcol > 0 {
                                if g_st().Ins.did_ai
                                    && *skipwhite(
                                        get_cursor_line_ptr().add(curwin_mut().w_cursor.col as usize),
                                    ) == NUL as c_char
                                {
                                    curwin_mut().w_wcol = 0;
                                    let ptr = get_cursor_line_ptr();
                                    let endptr = ptr.add(curwin_mut().w_cursor.col as usize);

                                    let mut csarg = CharsizeArg::new();
                                    let cstype: CSType = init_charsize_arg(
                                        &mut csarg,
                                        curwin_ptr(),
                                        curwin_mut().w_cursor.lnum,
                                        ptr,
                                    );
                                    let mut ci: StrCharInfo = utf_ptr2StrCharInfo(ptr);
                                    let mut vcol: c_int = 0;
                                    while ci.ptr < endptr.cast_const() {
                                        if !ascii_iswhite(ci.chr.value) {
                                            curwin_mut().w_wcol = vcol;
                                        }
                                        let cli: ClusterInfo = utf_ClusterInfo(ci);
                                        vcol += win_charsize(
                                            cstype,
                                            vcol,
                                            ci.ptr.cast_mut(),
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
                                    curwin_mut().w_wcol += win_col_off(curwin_ptr());
                                    col = 0;
                                } else {
                                    curwin_mut().w_wcol -= 1;
                                    col = curwin_mut().w_cursor.col - 1;
                                }
                            } else if curwin_mut().w_onebuf_opt.wo_wrap != 0 && curwin_mut().w_wrow != 0 {
                                curwin_mut().w_wrow -= 1;
                                curwin_mut().w_wcol = curwin_mut().w_view_width - 1;
                                col = curwin_mut().w_cursor.col - 1;
                            }
                            if col > 0 && curwin_mut().w_wcol > 0 {
                                let ptr = get_cursor_line_ptr();
                                col -= utf_head_off(ptr, ptr.add(col as usize));
                                if utf_ptr2cells(ptr.add(col as usize)) > 1 {
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
                        *tb_noremap_ptr().add((g_st().typebuf.tb_off + n) as usize) = RM_YES;
                    }
                    g_st().typebuf.tb_len += c;

                    if g_st().typebuf.tb_len >= g_st().typebuf.tb_maplen + MAXMAPLEN as c_int {
                        timedout = true;
                        continue;
                    }

                    if g_st().ex_normal_busy > 0 {
                        if g_st().typebuf.tb_len > 0 {
                            timedout = true;
                            continue;
                        }

                        c = if (g_st().State & MODE_CMDLINE) != 0
                            || (g_st().cmdwin_type > 0 && inp_st().VGETORPEEK_TC == ESC)
                        {
                            Ctrl_C
                        } else {
                            ESC
                        };
                        inp_st().VGETORPEEK_TC = c;

                        if advance {
                            g_st().typebuf_was_empty = true;
                        }
                        g_st().typebuf.tb_no_abbr_cnt = 0;
                        break;
                    }

                    if ((g_st().State & MODE_INSERT) != 0 || opt_vars().p_lz != 0)
                        && (g_st().State & MODE_CMDLINE) == 0
                        && advance
                        && g_st().must_redraw != 0
                        && !g_st().need_wait_return
                    {
                        update_screen();
                        setcursor();
                    }

                    let mut showcmd_idx: c_int = 0;
                    let mut showing_partial = false;
                    if g_st().typebuf.tb_len > 0 && advance {
                        if ((g_st().State & (MODE_NORMAL | MODE_INSERT)) != 0 || g_st().State == MODE_LANGMAP)
                            && g_st().State != MODE_HITRETURN
                        {
                            if (g_st().State & MODE_INSERT) != 0
                                && utf_ptr2cells(
                                    tb_buf_ptr()
                                        .add((g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1) as usize)
                                        .cast::<c_char>(),
                                ) == 1
                            {
                                edit_putchar(
                                    *tb_buf_ptr()
                                        .add((g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1) as usize)
                                        as c_int,
                                    false,
                                );
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
                                add_byte_to_showcmd(
                                    *tb_buf_ptr().add((g_st().typebuf.tb_off + showcmd_idx) as usize),
                                );
                                showcmd_idx += 1;
                            }
                            curwin_mut().w_wcol = old_wcol;
                            curwin_mut().w_wrow = old_wrow;
                        }

                        if (g_st().State & MODE_CMDLINE) != 0
                            && !(*get_cmdline_info()).cmdbuff.is_null()
                            && g_st().cmdline_star == 0
                        {
                            let p = tb_buf_ptr()
                                .add((g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1) as usize)
                                .cast::<c_char>();
                            if utf_ptr2cells(p) == 1 && (*p as u8) < 128 {
                                putcmdline(*p, false);
                                showing_partial = true;
                            }
                        }
                    }

                    if g_st().typebuf.tb_len == 0 {
                        timedout = false;
                    }

                    let mut wait_time: c_long = 0;
                    if advance {
                        if g_st().typebuf.tb_len == 0
                            || !(opt_vars().p_timeout != 0 || (opt_vars().p_ttimeout != 0 && keylen == KEYLEN_PART_KEY))
                        {
                            wait_time = -1;
                        } else if keylen == KEYLEN_PART_KEY && opt_vars().p_ttm >= 0 {
                            wait_time = opt_vars().p_ttm as c_long;
                        } else {
                            wait_time = opt_vars().p_tm as c_long;
                        }
                    }

                    let wait_tb_len = g_st().typebuf.tb_len;
                    c = inchar(
                        tb_buf_ptr().add((g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize),
                        g_st().typebuf.tb_buflen - g_st().typebuf.tb_off - g_st().typebuf.tb_len - 1,
                        wait_time,
                    );

                    if showcmd_idx != 0 {
                        pop_showcmd();
                    }
                    if showing_partial {
                        if (g_st().State & MODE_INSERT) != 0 {
                            edit_unputchar();
                        }
                        if (g_st().State & MODE_CMDLINE) != 0 && !(*get_cmdline_info()).cmdbuff.is_null() {
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
                        while *tb_buf_ptr().add((g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize)
                            != NUL as u8
                        {
                            *tb_noremap_ptr().add((g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize) =
                                RM_YES;
                            g_st().typebuf.tb_len += 1;
                        }
                    }
                }
            }

            if !(c < 0 || (advance && c == NUL)) {
                break;
            }
        }

        if advance && opt_vars().p_smd != 0 && g_st().msg_silent == 0 && (g_st().State & MODE_INSERT) != 0 {
            if c == ESC && !mode_deleted && g_st().no_mapping == 0 && g_st().mode_displayed {
                if g_st().typebuf.tb_len != 0 && !g_st().KeyTyped {
                    g_st().redraw_cmdline = true;
                } else {
                    unshowmode(false);
                }
            } else if c != ESC && mode_deleted {
                if g_st().typebuf.tb_len != 0 && !g_st().KeyTyped {
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
}

fn inchar(buf: *mut u8, maxlen: c_int, wait_time: c_long) -> c_int {
    unsafe {
        let mut len: c_int = 0;
        let mut retesc: c_int = 0;
        let tb_change_cnt = g_st().typebuf.tb_change_cnt;

        if wait_time == -1 || wait_time > 100 {
            ui_flush();
        }

        if g_st().State != MODE_HITRETURN {
            g_st().did_outofmem_msg = false;
            g_st().did_swapwrite_msg = false;
        }

        let mut read_size: isize = -1;
        while inp_st().curscript >= 0 && read_size <= 0 && !g_st().ignore_script {
            let mut script_char: c_char = 0;
            let scriptin_ptr = (&raw mut inp_st().scriptin)
                .cast::<FileDescriptor>()
                .add(inp_st().curscript as usize);
            if g_st().got_int || {
                read_size = file_read(scriptin_ptr, &raw mut script_char, 1);
                read_size != 1
            } {
                closescript();
                if g_st().got_int {
                    retesc = 1;
                } else {
                    return -1;
                }
            } else {
                *buf = script_char as u8;
                len = 1;
            }
        }

        if read_size <= 0 {
            if g_st().got_int {
                let mut dum = [0u8; DUM_LEN + 1];
                loop {
                    len = input_get(dum.as_mut_ptr(), DUM_LEN as c_int, 0, 0, ptr::null_mut());
                    if len == 0 || (len == 1 && dum[0] == Ctrl_C as u8) {
                        break;
                    }
                }
                return retesc;
            }

            if wait_time == -1 || wait_time > 10 {
                ui_flush();
            }

            len = input_get(
                buf,
                maxlen / 3,
                wait_time as c_int,
                tb_change_cnt,
                ptr::null_mut(),
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

        fix_input_buffer(buf, len)
    }
}

/// Fix typed characters for use by `vgetc()`.
pub fn fix_input_buffer(buf: *mut u8, mut len: c_int) -> c_int {
    unsafe {
        if using_script() == 0 {
            *buf.add(len as usize) = NUL as u8;
            return len;
        }

        let mut p = buf;
        let mut i = len;
        while {
            i -= 1;
            i >= 0
        } {
            if *p == NUL as u8 || (*p == K_SPECIAL && (i < 2 || *p.add(1) != KS_EXTRA)) {
                ptr::copy(p.add(1), p.add(3), i as usize);
                *p.add(2) = k_third(*p as c_int);
                *p.add(1) = k_second(*p as c_int);
                *p = K_SPECIAL;
                p = p.add(2);
                len += 2;
            }
            p = p.add(1);
        }
        *p = NUL as u8;
        len
    }
}

/// Get a key stroke directly from the user.
pub fn get_keystroke(events: *mut MultiQueue) -> c_int {
    unsafe {
        let mut buf: Vec<u8> = vec![0; 150];
        let mut buflen: c_int = 150;
        let mut len: c_int = 0;
        let mut n: c_int;
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
                buf.as_mut_ptr().add(len as usize),
                maxlen,
                if len == 0 { -1 } else { 100 },
                0,
                events,
            );
            if n > 0 {
                n = fix_input_buffer(buf.as_mut_ptr().add(len as usize), n);
                len += n;
            }

            if n > 0 {
                len = n;
            }
            if len == 0 {
                continue;
            }

            n = buf[0] as c_int;
            if n == K_SPECIAL as c_int {
                n = to_special(buf[1] as c_int, buf[2] as c_int);
                if buf[1] == KS_MODIFIER || n == K_IGNORE || (is_mouse_key(n) && n != K_LEFTMOUSE) {
                    if buf[1] == KS_MODIFIER {
                        g_st().mod_mask = buf[2] as c_int;
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
            n = utf_ptr2char(buf.as_ptr().cast());
            break;
        }

        g_st().mapped_ctrl_c = save_mapped_ctrl_c;
        merge_modifiers(n, &mut g_st().mod_mask)
    }
}

/// Function passed to `do_cmdline()` to get the command after a `<Cmd>` key from typeahead.
pub fn getcmdkeycmd(
    promptc: c_int,
    _indent: c_int,
    _do_concat: bool,
) -> Option<std::ffi::CString> {
    let mut line: Vec<u8> = Vec::new();
        let mut c1: c_int = -1;
        let mut cmod: c_int = 0;
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

            if c1 == K_SPECIAL as c_int {
                c1 = vgetorpeek(true);
                let c2 = vgetorpeek(true);
                if c1 == KS_MODIFIER as c_int {
                    cmod = c2;
                    continue;
                }
                c1 = to_special(c1, c2);
            }

            if g_st().got_int {
                aborted = true;
            } else if c1 == b'\r' as c_int || c1 == b'\n' as c_int {
                c1 = NUL;
            } else if c1 == ESC {
                aborted = true;
            } else if c1 == K_COMMAND {
                emsg(gettext(
                    E_CMD_MAPPING_MUST_END_WITH_CR_BEFORE_SECOND_CMD,
                ));
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
            let out = cstr_from_bytes(&line);
            if promptc == b':' as c_int {
                atom_cmdline_set(K_COMMAND, out.as_ptr(), out.as_bytes().len());
            }
            Some(out)
        }
}

/// Handle a Lua mapping: get its id from typeahead and execute its callback.
pub fn map_execute_lua(may_repeat: bool, discard: bool) -> bool {
     {
        let mut line: Vec<c_char> = Vec::new();
        let mut c1: c_int = -1;
        let mut aborted = false;

        g_st().no_mapping += 1;

        g_st().got_int = false;
        while c1 != NUL && !aborted {
            c1 = vgetorpeek(true);
            if g_st().got_int {
                aborted = true;
            } else if c1 == b'\r' as c_int || c1 == b'\n' as c_int {
                c1 = NUL;
            } else {
                line.push(c1 as c_char);
            }
        }

        g_st().no_mapping -= 1;

        if aborted || discard {
            return !aborted;
        }

        line.push(NUL as c_char);
        let id = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(line.as_ptr())) as c_int;
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
        let args = Array {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        nlua_call_ref(
            ref_,
            ptr::null(),
            args,
            kRetNilBool,
            None,
            Some(&mut err),
        );
        if err.err_type != ErrorType::kErrorTypeNone {
            semsg_multiline!(c"emsg", c"E5108: %s", err.msg_ptr());
            api_clear_error(&mut err);
        }

        true
    }
}

/// Wraps pasted text stream with `K_PASTE_START` and `K_PASTE_END`.
pub fn paste_store(channel_id: u64, state: TriState, str_: NvimString, crlf: bool) {
    unsafe {
        if (g_st().State & MODE_CMDLINE) != 0 {
            return;
        }

        let need_redo = !inp_st().block_redo;
        let need_record = g_st().reg_recording != 0 && !is_internal_call(channel_id);

        if !need_redo && !need_record {
            return;
        }

        let rbody = &raw mut inp_st().redobuff.cur.body;
        let recbuf = &raw mut inp_st().recordbuff;

        if state != kNone {
            let c = if state == kFalse {
                K_PASTE_START
            } else {
                K_PASTE_END
            };
            if need_redo {
                if state == kFalse && (g_st().State & MODE_INSERT) == 0 {
                    redo_new(CmdSpec::default());
                }
                sb_add_char(&mut *rbody, c);
            }
            if need_record {
                sb_add_char(&mut *recbuf, c);
            }
            return;
        }

        let mut s = str_.data.cast_const();
        let str_end = s.add(str_.size);

        while s < str_end {
            let start = s;
            while s < str_end
                && *s as u8 != K_SPECIAL
                && *s != NUL as c_char
                && *s != NL as c_char
                && !(crlf && *s == CAR as c_char)
            {
                s = s.add(1);
            }

            if s > start {
                let n = s.offset_from(start) as usize;
                let slice = std::slice::from_raw_parts(start, n);
                if need_redo {
                    (*rbody).extend_from_slice(slice);
                }
                if need_record {
                    (*recbuf).extend_from_slice(slice);
                }
            }

            if s < str_end {
                let mut c = *s as u8 as c_int;
                s = s.add(1);
                if crlf && c == CAR {
                    if s < str_end && *s == NL as c_char {
                        s = s.add(1);
                    }
                    c = NL;
                }
                if need_redo {
                    sb_add_byte(&mut *rbody, c);
                }
                if need_record {
                    sb_add_byte(&mut *recbuf, c);
                }
            }
        }
    }
}

/// Gets a paste stored by `paste_store()` from typeahead and repeats it.
pub fn paste_repeat(count: c_int) {
    {
        let mut buf: Vec<u8> = Vec::new();
        let mut aborted = false;

        g_st().no_mapping += 1;

        g_st().got_int = false;
        while !aborted {
            let mut c1 = vgetorpeek(true) as u8;
            if c1 == K_SPECIAL {
                c1 = vgetorpeek(true) as u8;
                let c2 = vgetorpeek(true) as u8;
                let c = to_special(c1 as c_int, c2 as c_int);
                if c == K_PASTE_END {
                    break;
                } else if c == K_ZERO {
                    buf.push(NUL as u8);
                } else if c == K_SPECIAL as c_int {
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

        let str_ = NvimString {
            data: buf.as_mut_ptr().cast::<c_char>(),
            size: buf.len(),
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let mut err = Error::new();
        let mut i = 0;
        while !aborted && i < count {
            nvim_paste(
                LUA_INTERNAL_CALL,
                str_,
                false,
                -1,
                &mut arena,
                &mut err,
            );
            aborted = err.err_type != ErrorType::kErrorTypeNone;
            i += 1;
        }
        api_clear_error(&mut err);
        arena_mem_free(arena_finish(&mut arena));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_byte_encode_and_merge_modifiers_work() {
        let mut tmp = [0 as c_char; 4];
        assert_eq!(key_byte_encode(b'a' as c_int, &mut tmp), 1);
        assert_eq!(tmp[0], b'a' as c_char);
        assert_eq!(key_byte_encode(NUL, &mut tmp), 3);
        assert_eq!(tmp[0] as u8, K_SPECIAL);

        let mut mods = MOD_MASK_CTRL as c_int;
        let merged = merge_modifiers(b'c' as c_int, &mut mods);
        assert_eq!(merged, Ctrl_C);
        assert_eq!(mods, 0);
    }
}
