//! The input engine "bytes layer" (`src/nvim/input.c`).

use crate::api::private::helpers::api_clear_error;
use crate::api::private::validate::{Array, Error, ErrorType, NvimString};
use crate::api::vim::{is_internal_call, nvim_paste};
use crate::charset::{ascii_iswhite, is_special, skipwhite, to_special};
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
use crate::lua::executor::{K_RET_NIL_BOOL, nlua_call_ref, nlua_exec_on_key};
use crate::mapping::{
    MAXMAPLEN, Mapblock, REMAP_NONE, REMAP_YES, Typebuf, eval_map_expr, get_buf_maphash_list,
    get_maphash_list, langmap_adjust_mb, map_luaid_get,
};
use crate::mbyte::{
    mb_cptr2char_adv, mb_unescape, utf_byte2len, utf_char2bytes, utf_cluster_info, utf_head_off,
    utf_ptr2_str_char_info, utf_ptr2cells, utf_ptr2char,
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
use crate::strings::{gettext, vim_snprintf};
use crate::textformat::Oparg;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_cursor_goto, ui_flush, vim_beep};
use crate::undo::u_sync;
use std::ffi::{CStr, c_char};
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
    curscript: i32,
    scriptin: [FileDescriptor; NSCRIPT],
    redobuff: RedoState,
    recordbuff: Vec<c_char>,
    readbuf1: StuffBuf,
    readbuf2: StuffBuf,
    ungot: UngotKey,
    on_key_buf: Vec<c_char>,
    on_key_ignore_len: usize,
    on_key_blocked: i32,
    typeahead_char: i32,
    block_redo: bool,
    key_noremap: i32,
    last_recorded_len: usize,
    saved_typebuf: [Typebuf; NSCRIPT],
    no_reduce_keys: i32,
    gotchars_state: GotcharsState,
    updatescript_count: i32,
    showcmd_state: GotcharsState,
    last_vgetc_recorded_len: usize,
    vgetorpeek_tc: i32,
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
            key_noremap: 0,
            last_recorded_len: 0,
            saved_typebuf: [const { Typebuf::ZERO }; NSCRIPT],
            no_reduce_keys: 0,
            gotchars_state: GotcharsState::new(),
            updatescript_count: 0,
            showcmd_state: GotcharsState::new(),
            last_vgetc_recorded_len: 0,
            vgetorpeek_tc: 0,
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
    g_st().typebuf.tb_buf.as_mut_ptr()
}

#[inline]
fn tb_noremap_ptr() -> *mut u8 {
    g_st().typebuf.tb_noremap.as_mut_ptr()
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
    {
        if !crate::memory::cstr_is_empty(opt_vars().p_langmap)
            && condition
            && (opt_vars().p_lrm != 0
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
}

/// Gets the contents of the record buffer as a string (typeahead encoding) and clears the buffer.
pub fn get_recorded() -> std::ffi::CString {
    {
        let mut bytes: Vec<u8> = std::mem::take(&mut inp_st().recordbuff)
            .into_iter()
            .map(|b| b as u8)
            .collect();
        let mut len = bytes.len();

        if len >= inp_st().last_recorded_len {
            len -= inp_st().last_recorded_len;
            bytes.truncate(len);
        }

        if len > 0 && g_st().restart_edit != 0 && bytes[len - 1] == CTRL_O as u8 {
            bytes.pop();
        }

        crate::memory::cstr_from_bytes(&bytes)
    }
}

fn redo_prefix(spec: &CmdSpec, buf: &mut Vec<c_char>, replay: bool) {
    {
        if spec.regname != 0 {
            sb_add_char(buf, b'"' as i32);
            sb_add_char(buf, spec.regname);
            if replay && spec.regname == b'=' as i32 {
                sb_add_char(buf, CAR);
            }
        }
        if spec.count != 0 {
            let mut numbuf = [0 as c_char; 32];
            let n = vim_snprintf!(numbuf.as_mut_ptr(), numbuf.len(), c"%d", spec.count as i32,);
            if n > 0 {
                buf.extend_from_slice(&numbuf[..n as usize]);
            }
        }
    }
}

fn redo_chars(spec: &CmdSpec, buf: &mut Vec<c_char>, arg_meta: bool) {
    {
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
        redo_prefix(&*spec, &mut buf, false);
        if !(*spec).body.is_empty() {
            buf.extend_from_slice(&(*spec).body);
        } else {
            redo_chars(&*spec, &mut buf, false);
        }
        if buf.is_empty() {
            None
        } else {
            let bytes = crate::memory::cstr_slice(buf.as_ptr(), buf.len());
            Some(crate::memory::cstr_from_bytes(bytes))
        }
    }
}

/// Gets the pending change's `CmdSpec` (without `body`).
pub fn redo_spec() -> CmdSpec {
    inp_st().redobuff.cur.without_body()
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

fn add_num_buff(buf: &mut StuffBuf, n: i32) {
    let mut number = [0 as c_char; 32];
    let numberlen = vim_snprintf!(number.as_mut_ptr(), number.len(), c"%d", n);
    add_buff(&mut *buf, number.as_ptr(), numberlen as isize);
}

fn key_byte_encode(c: i32, temp: &mut [c_char; 4]) -> isize {
    if is_special(c) || c == K_SPECIAL as i32 || c == NUL {
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

fn key_char_encode(c: i32, buf: *mut c_char) -> usize {
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
            let n = key_byte_encode(if is_special(c) { c } else { b as i32 }, &mut tmp) as usize;
            ptr::copy_nonoverlapping(tmp.as_ptr(), buf.wrapping_add(off), n + 1);
            off += n;
        }
        off
    }
}

fn add_char_buff(buf: &mut StuffBuf, c: i32) {
    let mut temp = [0 as c_char; MB_MAXBYTES * 3 + 1];
    let len = key_char_encode(c, temp.as_mut_ptr());
    add_buff(buf, temp.as_ptr(), len as isize);
}

fn sb_add_byte(sb: &mut Vec<c_char>, c: i32) {
    let mut temp = [0 as c_char; 4];
    let templen = key_byte_encode(c, &mut temp) as usize;
    sb.extend_from_slice(&temp[..templen]);
}

/// Dual of `add_char_buff()`, for a byte buffer.
pub fn sb_add_char(sb: &mut Vec<c_char>, c: i32) {
    let mut temp = [0 as c_char; MB_MAXBYTES * 3 + 1];
    let len = key_char_encode(c, temp.as_mut_ptr());
    sb.extend_from_slice(&temp[..len]);
}

fn read_readbuffers(advance: bool) -> i32 {
    let mut c = read_readbuf(&mut inp_st().readbuf1, advance);
    if c == NUL {
        c = read_readbuf(&mut inp_st().readbuf2, advance);
    }
    c
}

fn read_readbuf(buf: &mut StuffBuf, advance: bool) -> i32 {
    if buf.read == buf.keys.len() {
        return NUL;
    }

    let c = buf.keys.as_slice()[buf.read] as u8 as i32;

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
    inp_st().readbuf1.keys.is_empty() && inp_st().readbuf2.keys.is_empty()
}

/// Returns true if `inp_st().readbuf1` is empty.
pub fn readbuf1_empty() -> bool {
    inp_st().readbuf1.keys.is_empty()
}

/// Set a typeahead character that won't be flushed.
pub fn typeahead_noflush(c: i32) {
    inp_st().typeahead_char = c;
}

/// Remove the contents of the stuff buffer and the mapped characters in the typeahead buffer.
pub fn flush_buffers(flush_typeahead: FlushBuffers) {
    {
        init_typebuf();

        if g_st().typebuf.tb_maplen > 0 {
            atom_composite_abort();
        }

        if !inp_st().readbuf2.keys.is_empty() && g_st().visual_global.active {
            g_st().visual_global.need_end = true;
        }
        inp_st().readbuf1 = StuffBuf::new();
        inp_st().readbuf2 = StuffBuf::new();

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
                while inchar(tb_buf_ptr(), g_st().typebuf.tb_buflen - 1, 10) != 0 {}
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
    vim_beep(K_OPT_BO_FLAG_ERROR);
}

/// Starts capturing a new change.
pub fn redo_new(spec: CmdSpec) {
    {
        assert!(spec.body.is_empty());
        if inp_st().block_redo {
            return;
        }

        inp_st().redobuff.old = std::mem::replace(&mut inp_st().redobuff.cur, spec);
    }
}

/// Frees both redo buffers.
pub fn redo_free_all() {
    {
        inp_st().redobuff = RedoState::new();
    }
}

/// Prepare for redo of any command.
pub fn prep_redo(as_atom: bool, arg_meta: bool, spec: CmdSpec) {
    {
        if as_atom {
            atom_redo_prepped();
        }
        redo_new(spec.without_body());
        if inp_st().block_redo {
            return;
        }
        redo_chars(&spec, &mut inp_st().redobuff.cur.body, arg_meta);
    }
}

/// Prepare for redo of a Visual-mode command.
pub fn prep_redo_visual(keys: impl crate::strings::IntoCCharPtr, len: usize, spec: CmdSpec) {
    let keys = keys.into_c_char_ptr();
    unsafe {
        let mut stored = spec.without_body();
        stored.regname = 0;
        stored.count = 0;
        atom_redo_prepped();
        redo_new(stored);
        if inp_st().block_redo {
            return;
        }
        if !keys.is_null() && len > 0 {
            inp_st()
                .redobuff
                .cur
                .body
                .extend_from_slice(std::slice::from_raw_parts(keys, len));
        }
        redo_prefix(&spec, &mut inp_st().redobuff.cur.body, false);
        redo_chars(&spec, &mut inp_st().redobuff.cur.body, false);
    }
}

/// Discard the contents of the redo buffer and restore the previous redo buffer.
pub fn redo_cancel() {
    {
        if inp_st().block_redo {
            return;
        }

        atom_redo_cancel();
        inp_st().redobuff.cur = std::mem::take(&mut inp_st().redobuff.old);
        inp_st().readbuf1 = StuffBuf::new();
        inp_st().readbuf2 = StuffBuf::new();
    }
}

/// Saves the redo state to `save_redo` and detaches it.
pub fn save_redobuff(save_redo: &mut RedoState) {
    {
        save_redo.cur = inp_st().redobuff.cur.clone();
        save_redo.old = std::mem::take(&mut inp_st().redobuff.old);
    }
}

/// Restores the redo state from `save_redo`.
pub fn restore_redobuff(save_redo: &mut RedoState) {
    {
        inp_st().redobuff = std::mem::take(&mut *save_redo);
    }
}

/// Append `len` bytes of `s` (-1: up to the NUL) to the redo buffer.
pub fn redo_append_str<S: crate::strings::IntoCCharPtr>(s: S, len: isize) {
    let s = s.into_c_char_ptr();
    unsafe {
        if !inp_st().block_redo {
            let slen = if len < 0 {
                crate::memory::cstr_len(s)
            } else {
                len as usize
            };
            if !s.is_null() && slen > 0 {
                inp_st()
                    .redobuff
                    .cur
                    .body
                    .extend_from_slice(std::slice::from_raw_parts(s, slen));
            }
        }
    }
}

/// Appends `str` to `buf` literally, escaping special characters with CTRL-V.
pub fn sb_add_lit(buf: &mut Vec<c_char>, str_: impl crate::strings::IntoCCharPtr, len: i32) {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        let mut s = str_;
        while if len < 0 {
            !crate::memory::cstr_is_empty(s)
        } else {
            s.offset_from(str_) < len as isize
        } {
            let start = s;
            while *s >= b' ' as c_char
                && (crate::memory::cstr_first_byte(s)) < DEL as u8
                && (len < 0 || s.offset_from(str_) < len as isize)
            {
                s = s.wrapping_add(1);
            }

            if crate::memory::cstr_is_empty(s)
                && (*s.wrapping_sub(1) == b'0' as c_char || *s.wrapping_sub(1) == b'^' as c_char)
            {
                s = s.wrapping_sub(1);
            }
            if s > start {
                let n = s.offset_from(start) as usize;
                buf.extend_from_slice(std::slice::from_raw_parts(start, n));
            }

            if crate::memory::cstr_is_empty(s) || (len >= 0 && s.offset_from(str_) >= len as isize)
            {
                break;
            }

            let c = mb_cptr2char_adv(&raw mut s);
            if c < b' ' as i32
                || c == DEL
                || (crate::memory::cstr_is_empty(s) && (c == b'0' as i32 || c == b'^' as i32))
            {
                sb_add_char(buf, CTRL_V_CONST);
            }

            if crate::memory::cstr_is_empty(s) && c == b'0' as i32 {
                buf.extend_from_slice(std::slice::from_raw_parts(c"048".as_ptr(), 3));
            } else {
                sb_add_char(buf, c);
            }
        }
    }
}

/// Append to redo literally; no-op if `inp_st().block_redo` is set.
pub fn redo_append_lit(str_: impl crate::strings::IntoCCharPtr, len: i32) {
    let str_ = str_.into_c_char_ptr();
    {
        if inp_st().block_redo {
            return;
        }
        sb_add_lit(&mut inp_st().redobuff.cur.body, str_, len);
    }
}

/// Appends `s` to `buf`, leaving 3-byte special key codes unmodified and escaping other K_SPECIAL bytes.
pub fn sb_add_spec(buf: &mut Vec<c_char>, s: impl crate::strings::IntoCCharPtr) {
    let mut s = s.into_c_char_ptr();
    unsafe {
        while !crate::memory::cstr_is_empty(s) {
            if crate::memory::cstr_first_byte(s) == K_SPECIAL
                && crate::memory::cstr_byte_at(s, 1) != 0
                && crate::memory::cstr_byte_at(s, 2) != 0
            {
                buf.extend_from_slice(std::slice::from_raw_parts(s, 3));
                s = s.wrapping_add(3);
            } else {
                let c = mb_cptr2char_adv(&raw mut s);
                sb_add_char(buf, c);
            }
        }
    }
}

/// Append `s` to the redo buffer.
pub fn redo_append_spec(s: impl crate::strings::IntoCCharPtr) {
    let s = s.into_c_char_ptr();
    {
        if inp_st().block_redo {
            return;
        }
        sb_add_spec(&mut inp_st().redobuff.cur.body, s);
    }
}

/// Appends character `c` to the redo buffer, translated to typeahead encoding.
pub fn redo_append_char(c: i32) {
    {
        if !inp_st().block_redo {
            sb_add_char(&mut inp_st().redobuff.cur.body, c);
        }
    }
}

/// Appends string `s` (must be typeahead encoding) to the stuff buffer.
pub fn stuff_readbuff(s: impl crate::strings::IntoCCharPtr) {
    let s = s.into_c_char_ptr();
    add_buff(&mut inp_st().readbuf1, s, -1);
}

/// Appends string `s` (must be typeahead encoding) to the redo stuff buffer.
pub fn stuff_redo_readbuff(s: &std::ffi::CStr) {
    add_buff(&mut inp_st().readbuf2, s.as_ptr(), -1);
}

pub fn stuff_readbuff_len(s: impl crate::strings::IntoCCharPtr, len: isize) {
    let s = s.into_c_char_ptr();
    unsafe {
        if g_st().add_last_insert == 1 && len > 0 {
            g_st()
                .last_insert_ga
                .extend_from_slice(std::slice::from_raw_parts(s, len as usize));
        }
        add_buff(&mut inp_st().readbuf1, s, len);
    }
}

/// Stuff `s` into the stuff buffer, leaving special key codes unmodified.
pub fn stuff_readbuff_special(s: impl crate::strings::IntoCCharPtr) {
    let mut s = s.into_c_char_ptr();
    {
        while !crate::memory::cstr_is_empty(s) {
            if crate::memory::cstr_first_byte(s) == K_SPECIAL
                && crate::memory::cstr_byte_at(s, 1) != 0
                && crate::memory::cstr_byte_at(s, 2) != 0
            {
                stuff_readbuff_len(s, 3);
                s = s.wrapping_add(3);
            } else {
                let mut c = mb_cptr2char_adv(&raw mut s);
                if c == CAR || c == NL || c == ESC {
                    c = b' ' as i32;
                }
                stuffchar_readbuff(c);
            }
        }
    }
}

/// Appends character `c` to the stuff buffer, translated to typeahead encoding.
pub fn stuffchar_readbuff(c: i32) {
    add_char_buff(&mut inp_st().readbuf1, c);
}

/// Append a number to the stuff buffer.
pub fn stuffnum_readbuff(n: i32) {
    add_num_buff(&mut inp_st().readbuf1, n);
}

/// Stuff a string into the typeahead buffer.
pub fn stuffescaped(arg: impl crate::strings::IntoCCharPtr, literally: bool) {
    let mut arg = arg.into_c_char_ptr();
    unsafe {
        while !crate::memory::cstr_is_empty(arg) {
            let start = arg;
            while (*arg >= b' ' as c_char && (crate::memory::cstr_first_byte(arg)) < DEL as u8)
                || (crate::memory::cstr_first_byte(arg) == K_SPECIAL && !literally)
            {
                arg = arg.wrapping_add(1);
            }
            if arg > start {
                stuff_readbuff_len(start, arg.offset_from(start));
            }

            if !crate::memory::cstr_is_empty(arg) {
                let c = mb_cptr2char_adv(&raw mut arg);
                if literally && ((c < b' ' as i32 && c != TAB) || c == DEL) {
                    stuffchar_readbuff(CTRL_V_CONST);
                }
                stuffchar_readbuff(c);
            }
        }
    }
}

/// Dot-repeat `"."` command.
pub fn start_redo(count: i32, old_redo: bool) -> bool {
    {
        let rd = if old_redo {
            &raw mut inp_st().redobuff.old
        } else {
            &raw mut inp_st().redobuff.cur
        };
        let rd = unsafe { &mut *rd };
        if rd.body.is_empty() && rd.regname == 0 && rd.count == 0 {
            return false;
        }

        let mut spec = (*rd).without_body();
        if spec.regname >= b'1' as i32 && spec.regname < b'9' as i32 {
            spec.regname += 1;
        }
        if count != 0 {
            spec.count = i64::from(count);
        }
        let mut prefix: Vec<c_char> = Vec::new();
        redo_prefix(&spec, &mut prefix, true);
        add_buff(
            &mut inp_st().readbuf2,
            prefix.as_ptr(),
            prefix.len() as isize,
        );
        drop(prefix);
        if spec.regname == b'=' as i32 {
            g_st().cmd_silent = true;
        }

        add_buff(
            &mut inp_st().readbuf2,
            rd.body.as_ptr(),
            rd.body.len() as isize,
        );
        true
    }
}

/// Repeats the last insert (`R`, `o`, `O`, `a`, `A`, `i` or `I` command).
pub fn start_redo_ins() -> bool {
    unsafe {
        if inp_st().redobuff.cur.body.is_empty() {
            return false;
        }
        start_stuff();

        let mut p = inp_st().redobuff.cur.body.as_ptr();
        let end = p.wrapping_add(inp_st().redobuff.cur.body.len());
        while p < end {
            if crate::memory::cstr_first_byte(p) == K_SPECIAL && end.offset_from(p) >= 3 {
                p = p.wrapping_add(3);
                continue;
            }
            if crate::strings::vim_haschr(c"AaIiRrOo", crate::memory::cstr_first_byte(p) as i32) {
                if crate::memory::cstr_first_byte(p) == b'O'
                    || crate::memory::cstr_first_byte(p) == b'o'
                {
                    add_buff(&mut inp_st().readbuf2, NL_STR, -1);
                }
                p = p.wrapping_add(1);
                break;
            }
            p = p.wrapping_add(1);
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
    {
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
}

/// Returns true when keys cannot be remapped.
pub fn noremap_keys() -> bool {
    (inp_st().key_noremap & ((RM_NONE | RM_SCRIPT) as i32)) != 0
}

/// Insert a string in position `offset` in the typeahead buffer.
pub fn ins_typebuf(
    str_: impl crate::strings::IntoCCharPtr,
    noremap: i32,
    offset: i32,
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

        let addlen = crate::memory::cstr_len(str_) as i32;

        if offset == 0 && addlen <= g_st().typebuf.tb_off {
            g_st().typebuf.tb_off -= addlen;
            ptr::copy(
                str_.cast::<u8>(),
                tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize),
                addlen as usize,
            );
        } else if g_st().typebuf.tb_len == 0
            && g_st().typebuf.tb_buflen >= addlen + (3 * (MAXMAPLEN + 4)) as i32
        {
            g_st().typebuf.tb_off =
                (g_st().typebuf.tb_buflen - addlen - (3 * (MAXMAPLEN + 4)) as i32) / 2;
            ptr::copy(
                str_.cast::<u8>(),
                tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize),
                addlen as usize,
            );
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
            let s1 = s1_vec.as_mut_ptr();
            let s2 = s2_vec.as_mut_ptr();
            g_st().typebuf.tb_buflen = newlen;

            ptr::copy(
                tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize),
                s1.wrapping_add(newoff as usize),
                offset as usize,
            );
            ptr::copy(
                str_.cast::<u8>(),
                s1.wrapping_add((newoff + offset) as usize),
                addlen as usize,
            );
            let bytes = g_st().typebuf.tb_len - offset + 1;
            assert!(bytes > 0);
            ptr::copy(
                tb_buf_ptr().wrapping_add((g_st().typebuf.tb_off + offset) as usize),
                s1.wrapping_add((newoff + offset + addlen) as usize),
                bytes as usize,
            );
            g_st().typebuf.tb_buf = s1_vec;

            ptr::copy(
                tb_noremap_ptr().wrapping_add(g_st().typebuf.tb_off as usize),
                s2.wrapping_add(newoff as usize),
                offset as usize,
            );
            ptr::copy(
                tb_noremap_ptr().wrapping_add((g_st().typebuf.tb_off + offset) as usize),
                s2.wrapping_add((newoff + offset + addlen) as usize),
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
            *tb_noremap_ptr().wrapping_add((g_st().typebuf.tb_off + i + offset) as usize) =
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
pub fn requeue_key(c: i32, modifiers: i32, extra_consumed: i32, recorded: bool) {
    let mut buf = [0 as c_char; MB_MAXBYTES * 3 + 4];
    let len = special_to_buf(c, modifiers, true, buf.as_mut_ptr()) as usize;
    assert!(len < buf.len());
    buf[len] = NUL as c_char;
    ins_typebuf(
        buf.as_mut_ptr(),
        inp_st().key_noremap,
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
    unsafe {
        if len == 0 {
            return;
        }

        g_st().typebuf.tb_len -= len;

        if offset == 0
            && g_st().typebuf.tb_buflen - (g_st().typebuf.tb_off + len)
                >= (3 * MAXMAPLEN + 3) as i32
        {
            g_st().typebuf.tb_off += len;
        } else {
            let i = g_st().typebuf.tb_off + offset;
            if g_st().typebuf.tb_off > MAXMAPLEN as i32 {
                ptr::copy(
                    tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize),
                    tb_buf_ptr().wrapping_add(MAXMAPLEN),
                    offset as usize,
                );
                ptr::copy(
                    tb_noremap_ptr().wrapping_add(g_st().typebuf.tb_off as usize),
                    tb_noremap_ptr().wrapping_add(MAXMAPLEN),
                    offset as usize,
                );
                g_st().typebuf.tb_off = MAXMAPLEN as i32;
            }
            let bytes = g_st().typebuf.tb_len - offset + 1;
            assert!(bytes > 0);
            ptr::copy(
                tb_buf_ptr().wrapping_add((i + len) as usize),
                tb_buf_ptr().wrapping_add((g_st().typebuf.tb_off + offset) as usize),
                bytes as usize,
            );
            ptr::copy(
                tb_noremap_ptr().wrapping_add((i + len) as usize),
                tb_noremap_ptr().wrapping_add((g_st().typebuf.tb_off + offset) as usize),
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

fn gotchars(chars: *const u8, len: usize) {
    unsafe {
        let mut s = chars;
        let mut todo = len;

        while todo > 0 {
            todo -= 1;
            let b = *s;
            s = s.wrapping_add(1);
            if !gotchars_add_byte(&mut inp_st().gotchars_state, b) {
                continue;
            }

            let buflen = inp_st().gotchars_state.buflen;
            for i in 0..buflen {
                updatescript(inp_st().gotchars_state.buf[i] as i32);
            }

            if buflen > inp_st().on_key_ignore_len {
                let start_ptr = (&raw const inp_st().gotchars_state.buf)
                    .cast::<c_char>()
                    .wrapping_add(inp_st().on_key_ignore_len);
                inp_st()
                    .on_key_buf
                    .extend_from_slice(std::slice::from_raw_parts(
                        start_ptr,
                        buflen - inp_st().on_key_ignore_len,
                    ));
                inp_st().on_key_ignore_len = 0;
            } else {
                inp_st().on_key_ignore_len -= buflen;
            }

            if g_st().reg_recording != 0 {
                inp_st()
                    .recordbuff
                    .extend_from_slice(std::slice::from_raw_parts(
                        (&raw const inp_st().gotchars_state.buf).cast::<c_char>(),
                        buflen,
                    ));
                inp_st().last_recorded_len = inp_st().last_recorded_len.wrapping_add(buflen);
            }

            atom_typed_add(
                (&raw const inp_st().gotchars_state.buf).cast::<u8>(),
                buflen,
            );

            inp_st().gotchars_state.buflen = 0;
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

fn ungetchars(len: i32, recorded: bool) {
    {
        if !g_st().key_typed {
            return;
        }
        let len_u = len as usize;
        if recorded && g_st().reg_recording != 0 && inp_st().recordbuff.len() >= len_u {
            inp_st()
                .recordbuff
                .truncate(inp_st().recordbuff.len() - len_u);
            inp_st().last_recorded_len = inp_st().last_recorded_len.wrapping_sub(len_u);
        }
        atom_typed_del(len_u);
        let trim = len_u.min(inp_st().on_key_buf.len());
        inp_st()
            .on_key_buf
            .truncate(inp_st().on_key_buf.len() - trim);
        inp_st().on_key_ignore_len += len_u - trim;
    }
}

/// Sync undo when typed characters are obtained from the typeahead buffer.
pub fn may_sync_undo() {
    if ((g_st().state_global & (MODE_INSERT | MODE_CMDLINE)) == 0
        || g_st().ins_global.moved != K_INS_NONE)
        && inp_st().curscript < 0
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
    assert!(inp_st().curscript >= 0);
    init_typebuf();
    let prev_cnt = g_st().typebuf.tb_change_cnt;
    inp_st().saved_typebuf[inp_st().curscript as usize] =
        std::mem::replace(&mut g_st().typebuf, Typebuf::ZERO);
    g_st().typebuf.tb_change_cnt = prev_cnt;
    alloc_typebuf();
}

fn can_get_ungot() -> bool {
    inp_st().ungot.c != -1 && (inp_st().ungot.stuffed || stuff_empty())
}

/// Save all three kinds of typeahead, so that the user must type at a prompt.
pub fn save_typeahead(tp: &mut Tasave) {
    {
        let prev_cnt = g_st().typebuf.tb_change_cnt;
        tp.save_typebuf = std::mem::replace(&mut g_st().typebuf, Typebuf::ZERO);
        g_st().typebuf.tb_change_cnt = prev_cnt;
        alloc_typebuf();
        tp.ungot = inp_st().ungot;
        inp_st().ungot.c = -1;

        tp.save_readbuf1 = std::mem::replace(&mut inp_st().readbuf1, StuffBuf::new());
        tp.save_readbuf2 = std::mem::replace(&mut inp_st().readbuf2, StuffBuf::new());
    }
}

/// Restore the typeahead to what it was before calling `save_typeahead()`.
pub fn restore_typeahead(tp: &mut Tasave) {
    {
        free_typebuf();
        g_st().typebuf = std::mem::replace(&mut tp.save_typebuf, Typebuf::ZERO);
        inp_st().ungot = tp.ungot;

        inp_st().readbuf1 = std::mem::replace(&mut tp.save_readbuf1, StuffBuf::new());
        inp_st().readbuf2 = std::mem::replace(&mut tp.save_readbuf2, StuffBuf::new());
    }
}

/// Open a new script file for the `":source!"` command.
pub fn openscript(name: impl crate::strings::IntoCCharPtr, directly: bool) {
    let name = name.into_c_char_ptr();
    unsafe {
        if inp_st().curscript + 1 == NSCRIPT as i32 {
            emsg(gettext(E_NESTING));
            return;
        }

        if check_secure() {
            return;
        }

        if g_st().ignore_script {
            return;
        }

        inp_st().curscript += 1;
        let namebuff_ptr = g_st().name_buff.as_mut_ptr();
        expand_env(name, namebuff_ptr, MAXPATHL as i32);
        let scriptin_ptr = (&raw mut inp_st().scriptin)
            .cast::<FileDescriptor>()
            .wrapping_add(inp_st().curscript as usize);
        let error = file_open(&mut *scriptin_ptr, namebuff_ptr, K_FILE_READ_ONLY, 0);
        if error != 0 {
            semsg!(gettext(E_NOTOPEN_2), name, uv_strerror(error),);
            inp_st().curscript -= 1;
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

            let oldcurscript = inp_st().curscript;
            loop {
                update_topline_cursor();
                normal_cmd(&mut oa, false);
                vpeekc();
                if inp_st().curscript < oldcurscript {
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
    unsafe {
        assert!(inp_st().curscript >= 0);
        free_typebuf();
        g_st().typebuf = std::mem::replace(
            &mut inp_st().saved_typebuf[inp_st().curscript as usize],
            Typebuf::ZERO,
        );

        let scriptin_ptr = (&raw mut inp_st().scriptin)
            .cast::<FileDescriptor>()
            .wrapping_add(inp_st().curscript as usize);
        file_close(&mut *scriptin_ptr, false);
        inp_st().curscript -= 1;
    }
}

pub fn close_all_scripts() {
    while inp_st().curscript >= 0 {
        closescript();
    }
}

pub fn open_scriptin(scriptin_name: impl crate::strings::IntoCCharPtr) -> bool {
    let scriptin_name = scriptin_name.into_c_char_ptr();
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
                let slice =
                    crate::memory::cstr_slice(errbuf.as_ptr(), (n as usize).min(errbuf.len() - 1));
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
pub fn using_script() -> i32 {
    i32::from(inp_st().curscript >= 0)
}

/// Called just before doing a blocking wait.
pub fn before_blocking() {
    updatescript(0);
    if g_st().may_garbage_collect {
        garbage_collect(false);
    }
}

fn updatescript(c: i32) {
    unsafe {
        if c != 0
            && let Some(ref so) = g_st().scriptout
        {
            fputc(c, so.as_ptr());
        }
        let idle = c == 0;
        if idle
            || (opt_vars().p_uc > 0 && {
                inp_st().updatescript_count += 1;
                (inp_st().updatescript_count as OptInt) >= opt_vars().p_uc
            })
        {
            ml_sync_all(i32::from(idle), 1, opt_vars().p_fs != 0 || idle);
            inp_st().updatescript_count = 0;
        }
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
    unsafe {
        if opt_vars().p_sc == 0 || g_st().msg_silent != 0 {
            return;
        }

        if !gotchars_add_byte(&mut inp_st().showcmd_state, byte) {
            return;
        }

        let buflen = inp_st().showcmd_state.buflen;
        inp_st().showcmd_state.buf[buflen] = NUL as u8;
        inp_st().showcmd_state.buflen = 0;

        let mut modifiers: i32 = 0;
        let mut c: i32 = NUL;

        let mut ptr = (&raw const inp_st().showcmd_state.buf).cast::<u8>();
        if *ptr == K_SPECIAL
            && *ptr.wrapping_add(1) == KS_MODIFIER
            && crate::memory::cstr_byte_at(ptr.cast(), 2) != 0_u8
        {
            modifiers = crate::memory::cstr_char_at(ptr.cast(), 2);
            ptr = ptr.wrapping_add(3);
        }

        if *ptr != NUL as u8 {
            let mut cptr = ptr.cast::<c_char>();
            let mb_ptr = mb_unescape(&raw mut cptr);
            ptr = cptr.cast::<u8>();
            if !mb_ptr.is_null() {
                c = utf_ptr2char(mb_ptr);
            } else {
                c = *ptr as i32;
                ptr = ptr.wrapping_add(1);
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
        while *ptr != NUL as u8 {
            add_to_showcmd(*ptr as i32);
            ptr = ptr.wrapping_add(1);
        }
    }
}

/// Get the next input character.
pub fn vgetc() -> i32 {
    {
        let mut c: i32;
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

            inp_st().last_recorded_len = inp_st()
                .last_recorded_len
                .wrapping_sub(inp_st().last_vgetc_recorded_len);

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
                    c = utf_ptr2char(buf.as_ptr().cast());
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
                        inp_st().key_noremap,
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

            inp_st().last_vgetc_recorded_len = inp_st().last_recorded_len;
        }

        g_st().may_garbage_collect = false;

        inp_st().on_key_buf.push(NUL as c_char);
        if inp_st().on_key_blocked == 0 && nlua_exec_on_key(c, inp_st().on_key_buf.as_mut_ptr()) {
            if c == K_COMMAND {
                let _ = getcmdkeycmd(NUL, 0, false);
            } else if c == K_LUA {
                map_execute_lua(false, true);
            } else if c == K_PASTE_START {
                paste_repeat(0);
            }
            c = K_IGNORE;
        }
        inp_st().on_key_buf = Vec::new();

        if c != K_IGNORE {
            state_no_longer_safe(c"key typed");
        }

        c
    }
}

/// Like `vgetc()`, but never return a NUL when called recursively.
pub fn safe_vgetc() -> i32 {
    let mut c = vgetc();
    if c == NUL {
        c = get_keystroke(ptr::null_mut());
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
        return inp_st().ungot.c;
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

fn getchar_common(argvars: &mut [TypVal], rettv: &mut TypVal, mut allow_number: bool) {
    unsafe {
        let mut n: Varnumber;
        let called_emsg_start = g_st().called_emsg;
        let mut error = false;
        let mut simplify = true;
        let mut cursor_flag: c_char = NUL as c_char;

        if argvars[0].v_type != VarType::VarUnknown && !tv_check_for_opt_dict_arg(argvars, 1) {
            return;
        }

        let arg1 = (&raw mut argvars[1]);
        if argvars[0].v_type != VarType::VarUnknown && (*arg1).v_type == VarType::VarDict {
            let d = (*arg1).vval.v_dict();

            if allow_number {
                allow_number = tv_dict_get_bool(d.as_ref(), c"number", 1) != 0;
            } else if tv_dict_has_key(d.as_ref(), c"number") {
                semsg!(gettext(E_INVARG2), c"number");
            }

            simplify = tv_dict_get_bool(d.as_ref(), c"simplify", 1) != 0;

            let cursor_str = tv_dict_get_string(d.as_ref(), c"cursor");
            if !cursor_str.is_null() {
                if crate::memory::cstr_as_bytes(cursor_str) != b"hide"
                    && crate::memory::cstr_as_bytes(cursor_str) != b"keep"
                    && crate::memory::cstr_as_bytes(cursor_str) != b"msg"
                {
                    semsg!(gettext(E_INVARG_NVAL), c"cursor", cursor_str,);
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
            if cursor_flag == b'm' as c_char || (cursor_flag == NUL as c_char && g_st().msg_col > 0)
            {
                ui_cursor_goto(g_st().msg_row, g_st().msg_col);
            }

            if argvars[0].v_type == VarType::VarUnknown
                || (argvars[0].v_type == VarType::VarNumber && argvars[0].vval.v_number() == -1)
            {
                if !char_avail() {
                    ui_flush();
                    let events = main_loop_events();
                    input_get(ptr::null_mut(), 0, -1, g_st().typebuf.tb_change_cnt, events);
                    if input_available() == 0 && !multiqueue_empty(&*events) {
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

        let ni = n as i32;
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
                i += utf_char2bytes(ni, temp.as_mut_ptr().wrapping_add(i)) as usize;
            }
            assert!(i < 10);
            temp[i] = NUL as c_char;
            crate::eval::typval::tv_set_string_len(&mut *rettv, temp.as_ptr(), i);

            if is_mouse_key(ni) {
                let mut row = g_st().mouse_row;
                let mut col = g_st().mouse_col;
                let mut grid = g_st().mouse_grid;
                let mut lnum: Linenr = 0;

                if row >= 0 && col >= 0 {
                    let mut winnr: i32 = 1;
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
                    set_vim_var_nr(VV_MOUSE_WIN, Varnumber::from(winnr));
                    set_vim_var_nr(VV_MOUSE_WINID, Varnumber::from((*wp).handle));
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

fn put_string_in_typebuf(offset: i32, slen: i32, string: *mut u8, new_slen: i32) -> bool {
    unsafe {
        let extra = new_slen - slen;
        *string.wrapping_add(new_slen as usize) = NUL as u8;
        if extra < 0 {
            del_typebuf(-extra, offset);
        } else if extra > 0
            && !ins_typebuf(
                string.wrapping_add(slen as usize).cast::<c_char>(),
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
            tb_buf_ptr().wrapping_add((g_st().typebuf.tb_off + offset) as usize),
            new_slen as usize,
        );
        true
    }
}

fn at_ins_compl_key() -> bool {
    unsafe {
        let p = tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize);
        let mut c = *p as i32;

        if g_st().typebuf.tb_len > 3
            && c == K_SPECIAL as i32
            && *p.wrapping_add(1) == KS_MODIFIER
            && (*p.wrapping_add(2) & (MOD_MASK_CTRL as u8)) != 0
        {
            c = (*p.wrapping_add(3) & 0x1f) as i32;
        }
        !ctrl_x_mode_eval()
            && ((ctrl_x_mode_not_default() && vim_is_ctrl_x_key(c))
                || (compl_status_local() && (c == CTRL_N || c == CTRL_P)))
    }
}

fn check_simplify_modifier(max_offset: i32) -> i32 {
    unsafe {
        if (g_st().state_global & MODE_TERMINAL) != 0 || inp_st().no_reduce_keys > 0 {
            return 0;
        }

        for offset in 0..max_offset {
            if offset + 3 >= g_st().typebuf.tb_len {
                break;
            }
            let tp = tb_buf_ptr().wrapping_add((g_st().typebuf.tb_off + offset) as usize);
            if *tp == K_SPECIAL && *tp.wrapping_add(1) == KS_MODIFIER {
                let mut modifier = crate::memory::cstr_char_at(tp.cast(), 2);
                let c = crate::memory::cstr_char_at(tp.cast(), 3);
                let new_c = merge_modifiers(c, &mut modifier);

                if new_c != c {
                    if offset == 0 {
                        g_st().vgetc_char = c;
                        g_st().vgetc_mod_mask = crate::memory::cstr_char_at(tp.cast(), 2);
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
                        *tp.wrapping_add(2) = modifier as u8;
                        if !put_string_in_typebuf(offset + 3, 1, new_string.as_mut_ptr(), len) {
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

fn char_iter(itp: *mut *const u8, nomap: i32) -> i32 {
    unsafe {
        let mut it = *itp;
        let mut c = *it as i32;
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
                    it = it.wrapping_add(1);
                    buf[i as usize] = *it;
                    if buf[i as usize] == K_SPECIAL {
                        it = it.wrapping_add(1);
                        if *it == NUL as u8 {
                            break;
                        }
                        it = it.wrapping_add(1);
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
                if c == buf[0] as i32 {
                    it = *itp;
                }
            }
        }

        it = it.wrapping_add(1);
        *itp = it;
        c
    }
}

fn handle_mapping(keylenp: &mut i32, timedout: &bool, mapdepth: &mut i32, advance: bool) -> i32 {
    unsafe {
        let mut mp: Option<&Mapblock> = None;
        let mut mp_match: Option<&Mapblock> = None;
        let mut mp_match_len: i32 = 0;
        let mut tb_match_len: i32 = 0;
        let mut max_mlen: i32 = 0;
        let mut keylen = *keylenp;
        let local_state = get_real_state();
        let mut is_plug_map = false;

        let tb_start = tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize);
        if g_st().typebuf.tb_len >= 3
            && *tb_start == K_SPECIAL
            && *tb_start.wrapping_add(1) == KS_EXTRA
            && *tb_start.wrapping_add(2) == KE_PLUG
        {
            is_plug_map = true;
        }

        let mut tb_b1 = *tb_start as i32;
        if g_st().no_mapping == 0
            && (g_st().no_zero_mapping == 0 || tb_b1 != b'0' as i32)
            && (g_st().typebuf.tb_maplen == 0
                || is_plug_map
                || (*tb_noremap_ptr().wrapping_add(g_st().typebuf.tb_off as usize)
                    & (RM_NONE | RM_ABBR))
                    == 0)
            && !(opt_vars().p_paste != 0
                && (g_st().state_global & (MODE_INSERT | MODE_CMDLINE)) != 0)
            && !(g_st().state_global == MODE_HITRETURN && (tb_b1 == CAR || tb_b1 == b' ' as i32))
            && g_st().state_global != MODE_ASKMORE
            && !at_ins_compl_key()
        {
            let nolmaplen: i32;
            let mut tb_c1 = tb_b1;
            let mut tb_iter: *const u8 = tb_start.cast_const();

            if tb_b1 == K_SPECIAL as i32 {
                nolmaplen = 2;
                tb_iter = tb_iter.wrapping_add(1);
            } else {
                tb_c1 = char_iter(&raw mut tb_iter, 0);
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
                    let mut cbuf = [0 as c_char; 6];
                    utf_char2bytes(tb_c1, cbuf.as_mut_ptr());
                    tb_b1 = cbuf[0] as u8 as i32;
                }
                nolmaplen = 0;
            }

            mp = None;
            for cand in get_buf_maphash_list(local_state, tb_b1)
                .iter()
                .chain(get_maphash_list(local_state, tb_b1))
            {
                let mut key_iter: *const u8 = cand.m_keys.as_ptr().cast::<u8>();

                if char_iter(&raw mut key_iter, 0) == tb_c1
                    && (cand.m_mode & local_state) != 0
                    && ((cand.m_mode & MODE_LANGMAP) == 0 || g_st().typebuf.tb_maplen == 0)
                {
                    let mut nomap = nolmaplen;
                    let mut modifiers: i32 = 0;
                    let mut tmp_tb_iter = tb_iter;
                    let mut mlen = key_iter.offset_from(cand.m_keys.as_ptr().cast::<u8>()) as i32;
                    let mut tb_mlen =
                        (tmp_tb_iter.offset_from(tb_buf_ptr()) as i32) - g_st().typebuf.tb_off;

                    loop {
                        let mut c2 = char_iter(&raw mut tmp_tb_iter, nomap);
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

                        let key_char = char_iter(&raw mut key_iter, tb_nomap);
                        if key_char != c2 {
                            break;
                        }

                        mlen = key_iter.offset_from(cand.m_keys.as_ptr().cast::<u8>()) as i32;
                        tb_mlen =
                            (tmp_tb_iter.offset_from(tb_buf_ptr()) as i32) - g_st().typebuf.tb_off;
                    }

                    keylen = cand.m_keylen;
                    if mlen == keylen
                        || (mlen == g_st().typebuf.tb_len && g_st().typebuf.tb_len < keylen)
                    {
                        let mut s = tb_noremap_ptr().wrapping_add(g_st().typebuf.tb_off as usize);
                        let m_keys_u8 = cand.m_keys.as_ptr().cast::<u8>();
                        if *s == RM_SCRIPT
                            && (*m_keys_u8 != K_SPECIAL
                                || *m_keys_u8.wrapping_add(1) != KS_EXTRA
                                || *m_keys_u8.wrapping_add(2) != KE_SNR)
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
                            s = s.wrapping_add(1);
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
                if tb_b1 == K_SPECIAL as i32
                    && (g_st().typebuf.tb_len < 2
                        || (*tb_buf_ptr().wrapping_add((g_st().typebuf.tb_off + 1) as usize)
                            == KS_MODIFIER
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
            let mp = mp.unwrap();
            if keylen > g_st().typebuf.tb_maplen && (mp.m_mode & MODE_LANGMAP) == 0 {
                gotchars(
                    tb_buf_ptr()
                        .wrapping_add((g_st().typebuf.tb_off + g_st().typebuf.tb_maplen) as usize),
                    (keylen - g_st().typebuf.tb_maplen) as usize,
                );
                atom_map_start(mp.m_keys.as_ptr(), mp.m_keylen as usize, !advance);
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
                        res =
                            Some(std::ffi::CString::new([K_SPECIAL, KS_EXTRA, KE_IGNORE]).unwrap());
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
                } else if (crate::memory::cstr_prefix_bytes(map_str, keylen as usize)
                    == crate::memory::cstr_prefix_bytes(save_m_keys.as_ptr(), keylen as usize))
                    || save_alt_m_keys.as_ref().is_some_and(|alt| {
                        crate::memory::cstr_prefix_bytes(map_str, save_alt_m_keylen as usize)
                            == crate::memory::cstr_prefix_bytes(
                                alt.as_ptr(),
                                save_alt_m_keylen as usize,
                            )
                    })
                {
                    REMAP_SKIP
                } else {
                    REMAP_YES
                };

                ins_typebuf(
                    map_str,
                    noremap,
                    0,
                    true,
                    g_st().cmd_silent || save_m_silent,
                )
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
}

/// Unget one character (can only be done once!).
pub fn vungetc(c: i32) {
    inp_st().ungot = UngotKey {
        c,
        mod_mask: g_st().mod_mask,
        mouse_grid: g_st().mouse_grid,
        mouse_row: g_st().mouse_row,
        mouse_col: g_st().mouse_col,
        stuffed: g_st().key_stuffed != 0,
    };
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
    unsafe {
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
                        if ((g_st().mapped_ctrl_c | curbuf_mut().b_mapped_ctrl_c)
                            & get_real_state())
                            != 0
                        {
                            g_st().ctrl_c_interrupts = false;
                        }
                        os_breakcheck();
                        g_st().ctrl_c_interrupts = true;
                    }
                    let mut keylen: i32 = 0;
                    if g_st().got_int {
                        c = inchar(tb_buf_ptr(), g_st().typebuf.tb_buflen - 1, 0);
                        if (c != 0 || g_st().typebuf.tb_maplen != 0)
                            && (g_st().state_global & (MODE_INSERT | MODE_CMDLINE)) != 0
                        {
                            c = ESC;
                        } else {
                            c = CTRL_C_CONST;
                        }
                        flush_buffers(FLUSH_INPUT);

                        if advance {
                            *tb_buf_ptr() = c as u8;
                            gotchars(tb_buf_ptr(), 1);
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
                            c = *tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize) as i32;
                            if advance {
                                g_st().cmd_silent = g_st().typebuf.tb_silent > 0;
                                if g_st().typebuf.tb_maplen > 0 {
                                    g_st().key_typed = false;
                                } else {
                                    g_st().key_typed = true;
                                    gotchars(
                                        tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize),
                                        1,
                                    );
                                }
                                inp_st().key_noremap = *tb_noremap_ptr()
                                    .wrapping_add(g_st().typebuf.tb_off as usize)
                                    as i32;
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
                        && *tb_buf_ptr().wrapping_add(g_st().typebuf.tb_off as usize) == ESC as u8
                        && g_st().no_mapping == 0
                        && g_st().ex_normal_busy == 0
                        && g_st().typebuf.tb_maplen == 0
                        && (g_st().state_global & MODE_INSERT) != 0
                        && (opt_vars().p_timeout != 0
                            || (keylen == KEYLEN_PART_KEY && opt_vars().p_ttimeout != 0))
                        && {
                            c = inchar(
                                tb_buf_ptr().wrapping_add(
                                    (g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize,
                                ),
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
                                    && *skipwhite(
                                        get_cursor_line_ptr()
                                            .wrapping_add(curwin_mut().w_cursor.col as usize),
                                    ) == NUL as c_char
                                {
                                    curwin_mut().w_wcol = 0;
                                    let ptr = get_cursor_line_ptr();
                                    let endptr =
                                        ptr.wrapping_add(curwin_mut().w_cursor.col as usize);

                                    let mut csarg = CharsizeArg::new();
                                    let cstype: CSType = init_charsize_arg(
                                        &mut csarg,
                                        curwin_mut(),
                                        curwin_mut().w_cursor.lnum,
                                        ptr,
                                    );
                                    let mut ci: StrCharInfo = utf_ptr2_str_char_info(ptr);
                                    let mut vcol: i32 = 0;
                                    while ci.ptr < endptr.cast_const() {
                                        if !ascii_iswhite(ci.chr.value) {
                                            curwin_mut().w_wcol = vcol;
                                        }
                                        let cli: ClusterInfo = utf_cluster_info(ci);
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
                            } else if curwin_mut().w_onebuf_opt.wo_wrap != 0
                                && curwin_mut().w_wrow != 0
                            {
                                curwin_mut().w_wrow -= 1;
                                curwin_mut().w_wcol = curwin_mut().w_view_width - 1;
                                col = curwin_mut().w_cursor.col - 1;
                            }
                            if col > 0 && curwin_mut().w_wcol > 0 {
                                let ptr = get_cursor_line_ptr();
                                col -= utf_head_off(ptr, ptr.wrapping_add(col as usize));
                                if utf_ptr2cells(ptr.wrapping_add(col as usize)) > 1 {
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
                        *tb_noremap_ptr().wrapping_add((g_st().typebuf.tb_off + n) as usize) =
                            RM_YES;
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
                            || (g_st().cmdwin_type > 0 && inp_st().vgetorpeek_tc == ESC)
                        {
                            CTRL_C_CONST
                        } else {
                            ESC
                        };
                        inp_st().vgetorpeek_tc = c;

                        if advance {
                            g_st().typebuf_was_empty = true;
                        }
                        g_st().typebuf.tb_no_abbr_cnt = 0;
                        break;
                    }

                    if ((g_st().state_global & MODE_INSERT) != 0 || opt_vars().p_lz != 0)
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
                            if (g_st().state_global & MODE_INSERT) != 0
                                && utf_ptr2cells(
                                    tb_buf_ptr()
                                        .wrapping_add(
                                            (g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1)
                                                as usize,
                                        )
                                        .cast::<c_char>(),
                                ) == 1
                            {
                                edit_putchar(
                                    *tb_buf_ptr().wrapping_add(
                                        (g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1)
                                            as usize,
                                    ) as i32,
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
                                    *tb_buf_ptr().wrapping_add(
                                        (g_st().typebuf.tb_off + showcmd_idx) as usize,
                                    ),
                                );
                                showcmd_idx += 1;
                            }
                            curwin_mut().w_wcol = old_wcol;
                            curwin_mut().w_wrow = old_wrow;
                        }

                        if (g_st().state_global & MODE_CMDLINE) != 0
                            && !get_cmdline_info().cmdbuff.is_null()
                            && g_st().cmdline_star == 0
                        {
                            let p = tb_buf_ptr()
                                .wrapping_add(
                                    (g_st().typebuf.tb_off + g_st().typebuf.tb_len - 1) as usize,
                                )
                                .cast::<c_char>();
                            if utf_ptr2cells(p) == 1 && (crate::memory::cstr_first_byte(p)) < 128 {
                                putcmdline(*p, false);
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
                            || !(opt_vars().p_timeout != 0
                                || (opt_vars().p_ttimeout != 0 && keylen == KEYLEN_PART_KEY))
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
                        tb_buf_ptr()
                            .wrapping_add((g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize),
                        g_st().typebuf.tb_buflen
                            - g_st().typebuf.tb_off
                            - g_st().typebuf.tb_len
                            - 1,
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
                            && !get_cmdline_info().cmdbuff.is_null()
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
                        while *tb_buf_ptr()
                            .wrapping_add((g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize)
                            != NUL as u8
                        {
                            *tb_noremap_ptr().wrapping_add(
                                (g_st().typebuf.tb_off + g_st().typebuf.tb_len) as usize,
                            ) = RM_YES;
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
            && opt_vars().p_smd != 0
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
}

fn inchar(buf: *mut u8, maxlen: i32, wait_time: i64) -> i32 {
    unsafe {
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
        while inp_st().curscript >= 0 && read_size <= 0 && !g_st().ignore_script {
            let mut script_char: c_char = 0;
            let scriptin_ptr = (&raw mut inp_st().scriptin)
                .cast::<FileDescriptor>()
                .wrapping_add(inp_st().curscript as usize);
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
                    len = input_get(dum.as_mut_ptr(), DUM_LEN as i32, 0, 0, ptr::null_mut());
                    if len == 0 || (len == 1 && dum[0] == CTRL_C_CONST as u8) {
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
                wait_time as i32,
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
pub fn fix_input_buffer(buf: *mut u8, mut len: i32) -> i32 {
    unsafe {
        if using_script() == 0 {
            *buf.wrapping_add(len as usize) = NUL as u8;
            return len;
        }

        let mut p = buf;
        let mut i = len;
        while {
            i -= 1;
            i >= 0
        } {
            if *p == NUL as u8 || (*p == K_SPECIAL && (i < 2 || *p.wrapping_add(1) != KS_EXTRA)) {
                ptr::copy(p.wrapping_add(1), p.wrapping_add(3), i as usize);
                *p.wrapping_add(2) = k_third(*p as i32);
                *p.wrapping_add(1) = k_second(*p as i32);
                *p = K_SPECIAL;
                p = p.wrapping_add(2);
                len += 2;
            }
            p = p.wrapping_add(1);
        }
        *p = NUL as u8;
        len
    }
}

/// Get a key stroke directly from the user.
pub fn get_keystroke(events: *mut MultiQueue) -> i32 {
    {
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
                buf.as_mut_ptr().wrapping_add(len as usize),
                maxlen,
                if len == 0 { -1 } else { 100 },
                0,
                events,
            );
            if n > 0 {
                n = fix_input_buffer(buf.as_mut_ptr().wrapping_add(len as usize), n);
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
            n = utf_ptr2char(buf.as_ptr().cast());
            break;
        }

        g_st().mapped_ctrl_c = save_mapped_ctrl_c;
        merge_modifiers(n, &mut g_st().mod_mask)
    }
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
        let out = cstr_from_bytes(&line);
        if promptc == b':' as i32 {
            atom_cmdline_set(K_COMMAND, out.as_ptr(), out.as_bytes().len());
        }
        Some(out)
    }
}

/// Handle a Lua mapping: get its id from typeahead and execute its callback.
pub fn map_execute_lua(may_repeat: bool, discard: bool) -> bool {
    {
        let mut line: Vec<c_char> = Vec::new();
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
                line.push(c1 as c_char);
            }
        }

        g_st().no_mapping -= 1;

        if aborted || discard {
            return !aborted;
        }

        line.push(NUL as c_char);
        let id =
            crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(line.as_ptr())) as i32;
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
            K_RET_NIL_BOOL,
            None,
            Some(&mut err),
        );
        if err.err_type != ErrorType::ErrorTypeNone {
            semsg_multiline!(c"emsg", c"E5108: %s", err.msg_ptr());
            api_clear_error(&mut err);
        }

        true
    }
}

/// Wraps pasted text stream with `K_PASTE_START` and `K_PASTE_END`.
pub fn paste_store(channel_id: u64, state: TriState, str_: NvimString, crlf: bool) {
    unsafe {
        if (g_st().state_global & MODE_CMDLINE) != 0 {
            return;
        }

        let need_redo = !inp_st().block_redo;
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
                sb_add_char(&mut inp_st().redobuff.cur.body, c);
            }
            if need_record {
                sb_add_char(&mut inp_st().recordbuff, c);
            }
            return;
        }

        let mut s = str_.data.cast_const();
        let str_end = s.wrapping_add(str_.size);

        while s < str_end {
            let start = s;
            while s < str_end
                && crate::memory::cstr_first_byte(s) != K_SPECIAL
                && !crate::memory::cstr_is_empty(s)
                && *s != NL as c_char
                && !(crlf && *s == CAR as c_char)
            {
                s = s.wrapping_add(1);
            }

            if s > start {
                let n = s.offset_from(start) as usize;
                let slice = std::slice::from_raw_parts(start, n);
                if need_redo {
                    inp_st().redobuff.cur.body.extend_from_slice(slice);
                }
                if need_record {
                    inp_st().recordbuff.extend_from_slice(slice);
                }
            }

            if s < str_end {
                let mut c = crate::memory::cstr_first_byte(s) as i32;
                s = s.wrapping_add(1);
                if crlf && c == CAR {
                    if s < str_end && *s == NL as c_char {
                        s = s.wrapping_add(1);
                    }
                    c = NL;
                }
                if need_redo {
                    sb_add_byte(&mut inp_st().redobuff.cur.body, c);
                }
                if need_record {
                    sb_add_byte(&mut inp_st().recordbuff, c);
                }
            }
        }
    }
}

/// Gets a paste stored by `paste_store()` from typeahead and repeats it.
pub fn paste_repeat(count: i32) {
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
            nvim_paste(LUA_INTERNAL_CALL, str_, false, -1, &mut arena, &mut err);
            aborted = err.err_type != ErrorType::ErrorTypeNone;
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
        assert_eq!(key_byte_encode(b'a' as i32, &mut tmp), 1);
        assert_eq!(tmp[0], b'a' as c_char);
        assert_eq!(key_byte_encode(NUL, &mut tmp), 3);
        assert_eq!(tmp[0] as u8, K_SPECIAL);

        let mut mods = MOD_MASK_CTRL as i32;
        let merged = merge_modifiers(b'c' as i32, &mut mods);
        assert_eq!(merged, CTRL_C_CONST);
        assert_eq!(mods, 0);
    }
}
