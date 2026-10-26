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
use crate::memory::{Arena, arena_finish, arena_mem_free, xfree, xmalloc, xmemdupz};
use crate::message::{emsg, internal_error, semsg, semsg_multiline};
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
use crate::strings::{atoi, gettext, strcmp, strlen, strncmp, vim_snprintf, vim_strchr};
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
            save_typebuf: typebuf_T {
                tb_buf: ptr::null_mut(),
                tb_noremap: ptr::null_mut(),
                tb_buflen: 0,
                tb_off: 0,
                tb_len: 0,
                tb_maplen: 0,
                tb_silent: 0,
                tb_no_abbr_cnt: 0,
                tb_change_cnt: 0,
            },
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

static mut curscript: c_int = -1;
static mut scriptin: [FileDescriptor; NSCRIPT] = [const {
    FileDescriptor {
        fd: 0,
        buffer: ptr::null_mut(),
        read_pos: ptr::null_mut(),
        write_pos: ptr::null_mut(),
        wr: false,
        eof: false,
        non_blocking: false,
        bytes_read: 0,
    }
}; NSCRIPT];
static mut redobuff: RedoState = RedoState::new();
static mut recordbuff: Vec<c_char> = Vec::new();
static mut readbuf1: StuffBuf = StuffBuf::new();
static mut readbuf2: StuffBuf = StuffBuf::new();
static mut ungot: UngotKey = UngotKey {
    c: -1,
    mod_mask: 0,
    mouse_grid: 0,
    mouse_row: 0,
    mouse_col: 0,
    stuffed: false,
};
static mut on_key_buf: Vec<c_char> = Vec::new();
static mut on_key_ignore_len: usize = 0;
static mut on_key_blocked: c_int = 0;
static mut typeahead_char: c_int = 0;
static mut block_redo: bool = false;
static mut KeyNoremap: c_int = 0;
static mut typebuf_init: [u8; TYPELEN_INIT] = [0; TYPELEN_INIT];
static mut noremapbuf_init: [u8; TYPELEN_INIT] = [0; TYPELEN_INIT];
static mut last_recorded_len: usize = 0;
static mut saved_typebuf: [typebuf_T; NSCRIPT] = [typebuf_T {
    tb_buf: ptr::null_mut(),
    tb_noremap: ptr::null_mut(),
    tb_buflen: 0,
    tb_off: 0,
    tb_len: 0,
    tb_maplen: 0,
    tb_silent: 0,
    tb_no_abbr_cnt: 0,
    tb_change_cnt: 0,
}; NSCRIPT];
static mut no_reduce_keys: c_int = 0;

static mut GOTCHARS_STATE: gotchars_state_T = gotchars_state_T::new();
static mut UPDATESCRIPT_COUNT: c_int = 0;
static mut SHOWCMD_STATE: gotchars_state_T = gotchars_state_T::new();
static mut LAST_VGETC_RECORDED_LEN: usize = 0;
static mut VGETORPEEK_TC: c_int = 0;

use crate::api::buffer::LUA_INTERNAL_CALL;
use crate::cmdexpand::{Ctrl_N, Ctrl_P, MB_MAXBYTES};
use crate::keycodes::{k_second, k_third};
use crate::lua::converter::LUA_NOREF;
use crate::math::{FAIL, OK};
use crate::regexp::Ctrl_V;

#[inline]
pub unsafe fn mb_byte2len_check(b: c_int) -> c_int {
    if !(0..=255).contains(&b) {
        1
    } else {
        unsafe { utf_byte2len(b) }
    }
}

#[inline]
pub(crate) unsafe fn langmap_adjust(c: &mut c_int, condition: bool) {
    unsafe {
        if *p_langmap != 0
            && condition
            && (p_lrm != 0
                || if vgetc_busy != 0 {
                    typebuf_maplen() == 0
                } else {
                    KeyTyped
                })
            && KeyStuffed == 0
            && *c >= 0
        {
            if *c < 256 {
                *c = langmap_mapchar[*c as usize] as c_int;
            } else {
                *c = langmap_adjust_mb(*c);
            }
        }
    }
}

/// Gets the contents of the record buffer as a string (typeahead encoding) and clears the buffer.
pub unsafe fn get_recorded() -> *mut c_char {
    unsafe {
        let rb = &raw mut recordbuff;
        let mut len = (*rb).len();
        let p = xmemdupz((*rb).as_ptr().cast(), len).cast::<c_char>();
        *rb = Vec::new();

        if len >= last_recorded_len {
            len -= last_recorded_len;
            *p.add(len) = NUL as c_char;
        }

        if len > 0 && restart_edit != 0 && *p.add(len - 1) == Ctrl_O as c_char {
            *p.add(len - 1) = NUL as c_char;
        }

        p
    }
}

unsafe fn redo_prefix(spec: *const CmdSpec, buf: &mut Vec<c_char>, replay: bool) {
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
                c"%d".as_ptr(),
                (*spec).count as c_int,
            );
            if n > 0 {
                buf.extend_from_slice(&numbuf[..n as usize]);
            }
        }
    }
}

unsafe fn redo_chars(spec: *const CmdSpec, buf: &mut Vec<c_char>, arg_meta: bool) {
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

unsafe fn sb_take_string(buf: &mut Vec<c_char>) -> NvimString {
    unsafe {
        if buf.is_empty() {
            *buf = Vec::new();
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        let len = buf.len();
        let items = xmemdupz(buf.as_ptr().cast(), len).cast();
        *buf = Vec::new();
        NvimString {
            data: items,
            size: len,
        }
    }
}

/// Gets `spec` (or current pending change, if `spec` is NULL), composed into a "redo" keyseq.
pub unsafe fn redo_keys(spec: *const CmdSpec) -> NvimString {
    unsafe {
        let spec = if !spec.is_null() {
            spec
        } else {
            &raw const redobuff.cur
        };

        let mut buf: Vec<c_char> = Vec::new();
        redo_prefix(spec, &mut buf, false);
        if !(*spec).body.is_empty() {
            buf.extend_from_slice(&(*spec).body);
        } else {
            redo_chars(spec, &mut buf, false);
        }
        sb_take_string(&mut buf)
    }
}

/// Gets the pending change's `CmdSpec` (without `body`).
pub unsafe fn redo_spec() -> CmdSpec {
    unsafe {
        let cur = &raw const redobuff.cur;
        (*cur).without_body()
    }
}

unsafe fn add_buff(buf: *mut StuffBuf, s: *const c_char, mut slen: isize) {
    unsafe {
        if slen < 0 {
            slen = strlen(s) as isize;
        }
        if slen == 0 {
            return;
        }
        let slen_u = slen as usize;
        let slice = std::slice::from_raw_parts(s, slen_u);
        let insert = (*buf).insert;
        (*buf).keys.splice(insert..insert, slice.iter().copied());
        (*buf).insert += slen_u;
    }
}

unsafe fn add_num_buff(buf: *mut StuffBuf, n: c_int) {
    unsafe {
        let mut number = [0 as c_char; 32];
        let numberlen = vim_snprintf!(number.as_mut_ptr(), number.len(), c"%d".as_ptr(), n);
        add_buff(buf, number.as_ptr(), numberlen as isize);
    }
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

unsafe fn key_char_encode(c: c_int, buf: *mut c_char) -> usize {
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

unsafe fn add_char_buff(buf: *mut StuffBuf, c: c_int) {
    unsafe {
        let mut temp = [0 as c_char; MB_MAXBYTES * 3 + 1];
        let len = key_char_encode(c, temp.as_mut_ptr());
        add_buff(buf, temp.as_ptr(), len as isize);
    }
}

fn sb_add_byte(sb: &mut Vec<c_char>, c: c_int) {
    let mut temp = [0 as c_char; 4];
    let templen = key_byte_encode(c, &mut temp) as usize;
    sb.extend_from_slice(&temp[..templen]);
}

/// Dual of `add_char_buff()`, for a byte buffer.
pub unsafe fn sb_add_char(sb: &mut Vec<c_char>, c: c_int) {
    unsafe {
        let mut temp = [0 as c_char; MB_MAXBYTES * 3 + 1];
        let len = key_char_encode(c, temp.as_mut_ptr());
        sb.extend_from_slice(&temp[..len]);
    }
}

unsafe fn read_readbuffers(advance: bool) -> c_int {
    unsafe {
        let mut c = read_readbuf(&raw mut readbuf1, advance);
        if c == NUL {
            c = read_readbuf(&raw mut readbuf2, advance);
        }
        c
    }
}

unsafe fn read_readbuf(buf: *mut StuffBuf, advance: bool) -> c_int {
    unsafe {
        if (*buf).read == (*buf).keys.len() {
            return NUL;
        }

        let c = (*buf).keys.as_slice()[(*buf).read] as u8 as c_int;

        if advance {
            (*buf).read += 1;
            if (*buf).read == (*buf).keys.len() {
                (*buf).keys.clear();
                (*buf).read = 0;
                (*buf).insert = 0;
            } else {
                (*buf).insert = (*buf).insert.max((*buf).read);
            }
        }
        c
    }
}

unsafe fn start_stuff() {
    unsafe {
        readbuf1.insert = readbuf1.read;
        readbuf2.insert = readbuf2.read;
    }
}

/// Returns true if the readahead ("stuff") buffer is empty.
pub unsafe fn stuff_empty() -> bool {
    unsafe {
        let k1 = &raw const readbuf1.keys;
        let k2 = &raw const readbuf2.keys;
        (*k1).is_empty() && (*k2).is_empty()
    }
}

/// Returns true if `readbuf1` is empty.
pub unsafe fn readbuf1_empty() -> bool {
    unsafe {
        let k1 = &raw const readbuf1.keys;
        (*k1).is_empty()
    }
}

/// Set a typeahead character that won't be flushed.
pub unsafe fn typeahead_noflush(c: c_int) {
    unsafe {
        typeahead_char = c;
    }
}

/// Remove the contents of the stuff buffer and the mapped characters in the typeahead buffer.
pub unsafe fn flush_buffers(flush_typeahead: flush_buffers_T) {
    unsafe {
        init_typebuf();

        if typebuf.tb_maplen > 0 {
            atom_composite_abort();
        }

        let k2 = &raw const readbuf2.keys;
        if !(*k2).is_empty() && Visual.active {
            Visual.need_end = true;
        }
        readbuf1 = StuffBuf::new();
        readbuf2 = StuffBuf::new();

        if flush_typeahead == FLUSH_MINIMAL {
            if typebuf.tb_off + typebuf.tb_maplen >= typebuf.tb_buflen {
                typebuf.tb_off = MAXMAPLEN as c_int;
                typebuf.tb_len = 0;
            } else {
                typebuf.tb_off += typebuf.tb_maplen;
                typebuf.tb_len -= typebuf.tb_maplen;
            }
            if typebuf.tb_len == 0 {
                typebuf_was_filled = false;
            }
        } else {
            if flush_typeahead == FLUSH_INPUT {
                while inchar(typebuf.tb_buf, typebuf.tb_buflen - 1, 10) != 0 {}
            }
            typebuf.tb_off = MAXMAPLEN as c_int;
            typebuf.tb_len = 0;
            typebuf_was_filled = false;
        }
        typebuf.tb_maplen = 0;
        typebuf.tb_silent = 0;
        cmd_silent = false;
        typebuf.tb_no_abbr_cnt = 0;
        typebuf.tb_change_cnt = typebuf.tb_change_cnt.wrapping_add(1);
        if typebuf.tb_change_cnt == 0 {
            typebuf.tb_change_cnt = 1;
        }
    }
}

/// Hides keys from `vim.on_key()` callbacks.
pub unsafe fn block_on_key() {
    unsafe {
        on_key_blocked += 1;
    }
}

pub unsafe fn unblock_on_key() {
    unsafe {
        on_key_blocked -= 1;
    }
}

/// Flush map and typeahead buffers and give a warning for an error.
pub unsafe fn beep_flush() {
    unsafe {
        if emsg_silent == 0 && !mc_replaying() {
            flush_buffers(FLUSH_MINIMAL);
        }
        vim_beep(kOptBoFlagError);
    }
}

/// Starts capturing a new change.
pub unsafe fn redo_new(spec: CmdSpec) {
    unsafe {
        assert!(spec.body.is_empty());
        if block_redo {
            return;
        }

        let rb = &raw mut redobuff;
        (*rb).old = std::mem::replace(&mut (*rb).cur, spec);
    }
}

/// Frees both redo buffers.
pub unsafe fn redo_free_all() {
    unsafe {
        let rb = &raw mut redobuff;
        *rb = RedoState::new();
    }
}

/// Prepare for redo of any command.
pub unsafe fn prep_redo(as_atom: bool, arg_meta: bool, spec: CmdSpec) {
    unsafe {
        if as_atom {
            atom_redo_prepped();
        }
        redo_new(spec.without_body());
        if block_redo {
            return;
        }
        let body = &raw mut redobuff.cur.body;
        redo_chars(&raw const spec, &mut *body, arg_meta);
    }
}

/// Prepare for redo of a Visual-mode command.
pub unsafe fn prep_redo_visual(keys: *const c_char, len: usize, spec: CmdSpec) {
    unsafe {
        let mut stored = spec.without_body();
        stored.regname = 0;
        stored.count = 0;
        atom_redo_prepped();
        redo_new(stored);
        if block_redo {
            return;
        }
        let body = &raw mut redobuff.cur.body;
        if !keys.is_null() && len > 0 {
            (*body).extend_from_slice(std::slice::from_raw_parts(keys, len));
        }
        redo_prefix(&raw const spec, &mut *body, false);
        redo_chars(&raw const spec, &mut *body, false);
    }
}

/// Discard the contents of the redo buffer and restore the previous redo buffer.
pub unsafe fn redo_cancel() {
    unsafe {
        if block_redo {
            return;
        }

        atom_redo_cancel();
        let rb = &raw mut redobuff;
        (*rb).cur = std::mem::take(&mut (*rb).old);
        readbuf1 = StuffBuf::new();
        readbuf2 = StuffBuf::new();
    }
}

/// Saves the redo state to `save_redo` and detaches it.
pub unsafe fn save_redobuff(save_redo: *mut RedoState) {
    unsafe {
        let rb = &raw mut redobuff;
        (*save_redo).cur = (*rb).cur.clone();
        (*save_redo).old = std::mem::take(&mut (*rb).old);
    }
}

/// Restores the redo state from `save_redo`.
pub unsafe fn restore_redobuff(save_redo: *mut RedoState) {
    unsafe {
        let rb = &raw mut redobuff;
        *rb = std::mem::take(&mut *save_redo);
    }
}

/// Append `len` bytes of `s` (-1: up to the NUL) to the redo buffer.
pub unsafe fn redo_append_str(s: *const c_char, len: isize) {
    unsafe {
        if !block_redo {
            let slen = if len < 0 { strlen(s) } else { len as usize };
            if !s.is_null() && slen > 0 {
                let body = &raw mut redobuff.cur.body;
                (*body).extend_from_slice(std::slice::from_raw_parts(s, slen));
            }
        }
    }
}

/// Appends `str` to `buf` literally, escaping special characters with CTRL-V.
pub unsafe fn sb_add_lit(buf: &mut Vec<c_char>, str_: *const c_char, len: c_int) {
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

/// Append to redo literally; no-op if `block_redo` is set.
pub unsafe fn redo_append_lit(str_: *const c_char, len: c_int) {
    unsafe {
        if block_redo {
            return;
        }
        let body = &raw mut redobuff.cur.body;
        sb_add_lit(&mut *body, str_, len);
    }
}

/// Appends `s` to `buf`, leaving 3-byte special key codes unmodified and escaping other K_SPECIAL bytes.
pub unsafe fn sb_add_spec(buf: &mut Vec<c_char>, mut s: *const c_char) {
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
pub unsafe fn redo_append_spec(s: *const c_char) {
    unsafe {
        if block_redo {
            return;
        }
        let body = &raw mut redobuff.cur.body;
        sb_add_spec(&mut *body, s);
    }
}

/// Appends character `c` to the redo buffer, translated to typeahead encoding.
pub unsafe fn redo_append_char(c: c_int) {
    unsafe {
        if !block_redo {
            let body = &raw mut redobuff.cur.body;
            sb_add_char(&mut *body, c);
        }
    }
}

/// Appends string `s` (must be typeahead encoding) to the stuff buffer.
pub unsafe fn stuffReadbuff(s: *const c_char) {
    unsafe {
        add_buff(&raw mut readbuf1, s, -1);
    }
}

/// Appends string `s` (must be typeahead encoding) to the redo stuff buffer.
pub unsafe fn stuffRedoReadbuff(s: *const c_char) {
    unsafe {
        add_buff(&raw mut readbuf2, s, -1);
    }
}

pub unsafe fn stuffReadbuffLen(s: *const c_char, len: isize) {
    unsafe {
        if add_last_insert == 1 && len > 0 {
            let last_ins = &raw mut last_insert_ga;
            (*last_ins).extend_from_slice(std::slice::from_raw_parts(s, len as usize));
        }
        add_buff(&raw mut readbuf1, s, len);
    }
}

/// Stuff `s` into the stuff buffer, leaving special key codes unmodified.
pub unsafe fn stuffReadbuffSpecial(mut s: *const c_char) {
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
pub unsafe fn stuffcharReadbuff(c: c_int) {
    unsafe {
        add_char_buff(&raw mut readbuf1, c);
    }
}

/// Append a number to the stuff buffer.
pub unsafe fn stuffnumReadbuff(n: c_int) {
    unsafe {
        add_num_buff(&raw mut readbuf1, n);
    }
}

/// Stuff a string into the typeahead buffer.
pub unsafe fn stuffescaped(mut arg: *const c_char, literally: bool) {
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
pub unsafe fn start_redo(count: c_int, old_redo: bool) -> c_int {
    unsafe {
        let rd = if old_redo {
            &raw mut redobuff.old
        } else {
            &raw mut redobuff.cur
        };
        if (*rd).body.is_empty() && (*rd).regname == 0 && (*rd).count == 0 {
            return FAIL;
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
        add_buff(&raw mut readbuf2, prefix.as_ptr(), prefix.len() as isize);
        drop(prefix);
        if spec.regname == b'=' as c_int {
            cmd_silent = true;
        }

        add_buff(
            &raw mut readbuf2,
            (*rd).body.as_ptr(),
            (*rd).body.len() as isize,
        );
        OK
    }
}

/// Repeats the last insert (`R`, `o`, `O`, `a`, `A`, `i` or `I` command).
pub unsafe fn start_redo_ins() -> c_int {
    unsafe {
        let body = &raw const redobuff.cur.body;
        if (*body).is_empty() {
            return FAIL;
        }
        start_stuff();

        let mut p = (*body).as_ptr();
        let end = p.add((*body).len());
        while p < end {
            if *p as u8 == K_SPECIAL && end.offset_from(p) >= 3 {
                p = p.add(3);
                continue;
            }
            if !vim_strchr(c"AaIiRrOo".as_ptr(), *p as u8 as c_int).is_null() {
                if *p == b'O' as c_char || *p == b'o' as c_char {
                    add_buff(&raw mut readbuf2, NL_STR.as_ptr(), -1);
                }
                p = p.add(1);
                break;
            }
            p = p.add(1);
        }

        add_buff(&raw mut readbuf2, p, end.offset_from(p));
        block_redo = true;
        OK
    }
}

pub unsafe fn stop_redo_ins() {
    unsafe {
        block_redo = false;
    }
}

unsafe fn init_typebuf() {
    unsafe {
        if !typebuf.tb_buf.is_null() {
            return;
        }

        typebuf.tb_buf = (&raw mut typebuf_init).cast::<u8>();
        typebuf.tb_noremap = (&raw mut noremapbuf_init).cast::<u8>();
        typebuf.tb_buflen = TYPELEN_INIT as c_int;
        typebuf.tb_len = 0;
        typebuf.tb_off = (MAXMAPLEN + 4) as c_int;
        typebuf.tb_change_cnt = 1;
    }
}

/// Returns true when keys cannot be remapped.
pub unsafe fn noremap_keys() -> bool {
    unsafe { (KeyNoremap & ((RM_NONE | RM_SCRIPT) as c_int)) != 0 }
}

/// Insert a string in position `offset` in the typeahead buffer.
pub unsafe fn ins_typebuf(
    str_: *mut c_char,
    noremap: c_int,
    offset: c_int,
    nottyped: bool,
    silent: bool,
) -> c_int {
    unsafe {
        init_typebuf();
        typebuf.tb_change_cnt = typebuf.tb_change_cnt.wrapping_add(1);
        if typebuf.tb_change_cnt == 0 {
            typebuf.tb_change_cnt = 1;
        }
        state_no_longer_safe(c"ins_typebuf()".as_ptr());

        let addlen = strlen(str_) as c_int;

        if offset == 0 && addlen <= typebuf.tb_off {
            typebuf.tb_off -= addlen;
            ptr::copy(
                str_.cast::<u8>(),
                typebuf.tb_buf.add(typebuf.tb_off as usize),
                addlen as usize,
            );
        } else if typebuf.tb_len == 0
            && typebuf.tb_buflen >= addlen + (3 * (MAXMAPLEN + 4)) as c_int
        {
            typebuf.tb_off = (typebuf.tb_buflen - addlen - (3 * (MAXMAPLEN + 4)) as c_int) / 2;
            ptr::copy(
                str_.cast::<u8>(),
                typebuf.tb_buf.add(typebuf.tb_off as usize),
                addlen as usize,
            );
        } else {
            let newoff = (MAXMAPLEN + 4) as c_int;
            let extra = addlen + newoff + (4 * (MAXMAPLEN + 4)) as c_int;
            if typebuf.tb_len > c_int::MAX - extra {
                emsg(gettext((&raw const e_command_too_complex).cast()));
                setcursor();
                return FAIL;
            }
            let newlen = typebuf.tb_len + extra;
            let s1 = xmalloc(newlen as usize).cast::<u8>();
            let s2 = xmalloc(newlen as usize).cast::<u8>();
            typebuf.tb_buflen = newlen;

            ptr::copy(
                typebuf.tb_buf.add(typebuf.tb_off as usize),
                s1.add(newoff as usize),
                offset as usize,
            );
            ptr::copy(
                str_.cast::<u8>(),
                s1.add((newoff + offset) as usize),
                addlen as usize,
            );
            let bytes = typebuf.tb_len - offset + 1;
            assert!(bytes > 0);
            ptr::copy(
                typebuf.tb_buf.add((typebuf.tb_off + offset) as usize),
                s1.add((newoff + offset + addlen) as usize),
                bytes as usize,
            );
            if typebuf.tb_buf != (&raw mut typebuf_init).cast::<u8>() {
                xfree(typebuf.tb_buf.cast());
            }
            typebuf.tb_buf = s1;

            ptr::copy(
                typebuf.tb_noremap.add(typebuf.tb_off as usize),
                s2.add(newoff as usize),
                offset as usize,
            );
            ptr::copy(
                typebuf.tb_noremap.add((typebuf.tb_off + offset) as usize),
                s2.add((newoff + offset + addlen) as usize),
                (typebuf.tb_len - offset) as usize,
            );
            if typebuf.tb_noremap != (&raw mut noremapbuf_init).cast::<u8>() {
                xfree(typebuf.tb_noremap.cast());
            }
            typebuf.tb_noremap = s2;

            typebuf.tb_off = newoff;
        }
        typebuf.tb_len += addlen;

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
            *typebuf
                .tb_noremap
                .add((typebuf.tb_off + i + offset) as usize) = if nrm >= 0 { val } else { RM_YES };
        }

        if nottyped || typebuf.tb_maplen > offset {
            typebuf.tb_maplen += addlen;
        }
        if silent || typebuf.tb_silent > offset {
            typebuf.tb_silent += addlen;
            cmd_silent = true;
        }
        if typebuf.tb_no_abbr_cnt != 0 && offset == 0 {
            typebuf.tb_no_abbr_cnt += addlen;
        }

        OK
    }
}

/// Re-queues key `c`: puts a key obtained by `vgetc()` back into typeahead.
pub unsafe fn requeue_key(c: c_int, modifiers: c_int, extra_consumed: c_int, recorded: bool) {
    unsafe {
        let mut buf = [0 as c_char; MB_MAXBYTES * 3 + 4];
        let len = special_to_buf(c, modifiers, true, buf.as_mut_ptr()) as usize;
        assert!(len < buf.len());
        buf[len] = NUL as c_char;
        ins_typebuf(buf.as_mut_ptr(), KeyNoremap, 0, !KeyTyped, cmd_silent);
        ungetchars(len as c_int + extra_consumed, recorded);
    }
}

/// Return true if the typeahead buffer was changed.
pub unsafe fn typebuf_changed(tb_change_cnt: c_int) -> bool {
    unsafe { tb_change_cnt != 0 && (typebuf.tb_change_cnt != tb_change_cnt || typebuf_was_filled) }
}

/// Return true if there are no characters in the typeahead buffer that have not been typed.
pub unsafe fn typebuf_typed() -> c_int {
    unsafe { c_int::from(typebuf.tb_maplen == 0) }
}

/// Get the number of characters that are mapped (or not typed).
pub unsafe fn typebuf_maplen() -> c_int {
    unsafe { typebuf.tb_maplen }
}

/// Remove `len` characters from `typebuf.tb_buf[typebuf.tb_off + offset]`.
pub unsafe fn del_typebuf(len: c_int, offset: c_int) {
    unsafe {
        if len == 0 {
            return;
        }

        typebuf.tb_len -= len;

        if offset == 0 && typebuf.tb_buflen - (typebuf.tb_off + len) >= (3 * MAXMAPLEN + 3) as c_int
        {
            typebuf.tb_off += len;
        } else {
            let i = typebuf.tb_off + offset;
            if typebuf.tb_off > MAXMAPLEN as c_int {
                ptr::copy(
                    typebuf.tb_buf.add(typebuf.tb_off as usize),
                    typebuf.tb_buf.add(MAXMAPLEN),
                    offset as usize,
                );
                ptr::copy(
                    typebuf.tb_noremap.add(typebuf.tb_off as usize),
                    typebuf.tb_noremap.add(MAXMAPLEN),
                    offset as usize,
                );
                typebuf.tb_off = MAXMAPLEN as c_int;
            }
            let bytes = typebuf.tb_len - offset + 1;
            assert!(bytes > 0);
            ptr::copy(
                typebuf.tb_buf.add((i + len) as usize),
                typebuf.tb_buf.add((typebuf.tb_off + offset) as usize),
                bytes as usize,
            );
            ptr::copy(
                typebuf.tb_noremap.add((i + len) as usize),
                typebuf.tb_noremap.add((typebuf.tb_off + offset) as usize),
                (typebuf.tb_len - offset) as usize,
            );
        }

        if typebuf.tb_maplen > offset {
            if typebuf.tb_maplen < offset + len {
                typebuf.tb_maplen = offset;
            } else {
                typebuf.tb_maplen -= len;
            }
        }
        if typebuf.tb_silent > offset {
            if typebuf.tb_silent < offset + len {
                typebuf.tb_silent = offset;
            } else {
                typebuf.tb_silent -= len;
            }
        }
        if typebuf.tb_no_abbr_cnt > offset {
            if typebuf.tb_no_abbr_cnt < offset + len {
                typebuf.tb_no_abbr_cnt = offset;
            } else {
                typebuf.tb_no_abbr_cnt -= len;
            }
        }

        typebuf_was_filled = false;
        typebuf.tb_change_cnt = typebuf.tb_change_cnt.wrapping_add(1);
        if typebuf.tb_change_cnt == 0 {
            typebuf.tb_change_cnt = 1;
        }
    }
}

unsafe fn gotchars_add_byte(state: *mut gotchars_state_T, byte: u8) -> bool {
    unsafe {
        (*state).buf[(*state).buflen] = byte;
        (*state).buflen += 1;
        let mut c = byte as c_int;
        let mut retval = false;
        let in_special = (*state).pending_special > 0;
        let in_mbyte = (*state).pending_mbyte > 0;

        'ret_false: {
            if in_special {
                (*state).pending_special -= 1;
            } else if c == K_SPECIAL as c_int {
                (*state).pending_special = 2;
            }

            if (*state).pending_special > 0 {
                break 'ret_false;
            }

            if in_mbyte {
                (*state).pending_mbyte -= 1;
            } else {
                if in_special {
                    if (*state).prev_c == KS_MODIFIER as c_int {
                        break 'ret_false;
                    }
                    c = to_special((*state).prev_c, c);
                    if c == K_COMPLETE_DELAY {
                        (*state).buflen = 0;
                    }
                }
                (*state).pending_mbyte = (mb_byte2len_check(c) - 1) as c_uint;
            }

            if (*state).pending_mbyte > 0 {
                break 'ret_false;
            }

            retval = true;
        }
        (*state).prev_c = c;
        retval
    }
}

unsafe fn gotchars(chars: *const u8, len: usize) {
    unsafe {
        let mut s = chars;
        let mut todo = len;

        while todo > 0 {
            todo -= 1;
            let b = *s;
            s = s.add(1);
            if !gotchars_add_byte(&raw mut GOTCHARS_STATE, b) {
                continue;
            }

            let buflen = GOTCHARS_STATE.buflen;
            for i in 0..buflen {
                updatescript(GOTCHARS_STATE.buf[i] as c_int);
            }

            if buflen > on_key_ignore_len {
                let start_ptr = (&raw const GOTCHARS_STATE.buf)
                    .cast::<c_char>()
                    .add(on_key_ignore_len);
                let okb = &raw mut on_key_buf;
                (*okb).extend_from_slice(std::slice::from_raw_parts(
                    start_ptr,
                    buflen - on_key_ignore_len,
                ));
                on_key_ignore_len = 0;
            } else {
                on_key_ignore_len -= buflen;
            }

            if reg_recording != 0 {
                let rb = &raw mut recordbuff;
                (*rb).extend_from_slice(std::slice::from_raw_parts(
                    (&raw const GOTCHARS_STATE.buf).cast::<c_char>(),
                    buflen,
                ));
                last_recorded_len += buflen;
            }

            atom_typed_add((&raw const GOTCHARS_STATE.buf).cast::<u8>(), buflen);

            GOTCHARS_STATE.buflen = 0;
        }

        may_sync_undo();
        debug_did_msg = false;
        maptick += 1;
    }
}

/// Record an `<Ignore>` key.
pub unsafe fn gotchars_ignore() {
    unsafe {
        let nop_buf: [u8; 3] = [K_SPECIAL, KS_EXTRA, KE_IGNORE];
        on_key_ignore_len += 3;
        gotchars(nop_buf.as_ptr(), 3);
    }
}

unsafe fn ungetchars(len: c_int, recorded: bool) {
    unsafe {
        if !KeyTyped {
            return;
        }
        let len_u = len as usize;
        let rb = &raw mut recordbuff;
        if recorded && reg_recording != 0 && (*rb).len() >= len_u {
            (*rb).truncate((*rb).len() - len_u);
            last_recorded_len -= len_u;
        }
        atom_typed_del(len_u);
        let okb = &raw mut on_key_buf;
        let trim = len_u.min((*okb).len());
        (*okb).truncate((*okb).len() - trim);
        on_key_ignore_len += len_u - trim;
    }
}

/// Sync undo when typed characters are obtained from the typeahead buffer.
pub unsafe fn may_sync_undo() {
    unsafe {
        if ((State & (MODE_INSERT | MODE_CMDLINE)) == 0 || Ins.moved != kInsNone) && curscript < 0 {
            u_sync(false);
        }
    }
}

unsafe fn alloc_typebuf() {
    unsafe {
        typebuf.tb_buf = xmalloc(TYPELEN_INIT).cast::<u8>();
        typebuf.tb_noremap = xmalloc(TYPELEN_INIT).cast::<u8>();
        typebuf.tb_buflen = TYPELEN_INIT as c_int;
        typebuf.tb_off = (MAXMAPLEN + 4) as c_int;
        typebuf.tb_len = 0;
        typebuf.tb_maplen = 0;
        typebuf.tb_silent = 0;
        typebuf.tb_no_abbr_cnt = 0;
        typebuf.tb_change_cnt = typebuf.tb_change_cnt.wrapping_add(1);
        if typebuf.tb_change_cnt == 0 {
            typebuf.tb_change_cnt = 1;
        }
        typebuf_was_filled = false;
    }
}

unsafe fn free_typebuf() {
    unsafe {
        if typebuf.tb_buf == (&raw mut typebuf_init).cast::<u8>() {
            internal_error(c"Free typebuf 1".as_ptr());
        } else {
            xfree(typebuf.tb_buf.cast());
            typebuf.tb_buf = ptr::null_mut();
        }
        if typebuf.tb_noremap == (&raw mut noremapbuf_init).cast::<u8>() {
            internal_error(c"Free typebuf 2".as_ptr());
        } else {
            xfree(typebuf.tb_noremap.cast());
            typebuf.tb_noremap = ptr::null_mut();
        }
    }
}

unsafe fn save_typebuf() {
    unsafe {
        assert!(curscript >= 0);
        init_typebuf();
        *(&raw mut saved_typebuf)
            .cast::<typebuf_T>()
            .add(curscript as usize) = typebuf;
        alloc_typebuf();
    }
}

unsafe fn can_get_ungot() -> bool {
    unsafe { ungot.c != -1 && (ungot.stuffed || stuff_empty()) }
}

/// Save all three kinds of typeahead, so that the user must type at a prompt.
pub unsafe fn save_typeahead(tp: *mut tasave_T) {
    unsafe {
        (*tp).save_typebuf = typebuf;
        alloc_typebuf();
        (*tp).ungot = ungot;
        ungot.c = -1;

        let rb1 = &raw mut readbuf1;
        let rb2 = &raw mut readbuf2;
        (*tp).save_readbuf1 = std::mem::replace(&mut *rb1, StuffBuf::new());
        (*tp).save_readbuf2 = std::mem::replace(&mut *rb2, StuffBuf::new());
    }
}

/// Restore the typeahead to what it was before calling `save_typeahead()`.
pub unsafe fn restore_typeahead(tp: *mut tasave_T) {
    unsafe {
        free_typebuf();
        typebuf = (*tp).save_typebuf;
        ungot = (*tp).ungot;

        let rb1 = &raw mut readbuf1;
        let rb2 = &raw mut readbuf2;
        *rb1 = std::mem::replace(&mut (*tp).save_readbuf1, StuffBuf::new());
        *rb2 = std::mem::replace(&mut (*tp).save_readbuf2, StuffBuf::new());
    }
}

/// Open a new script file for the `":source!"` command.
pub unsafe fn openscript(name: *mut c_char, directly: bool) {
    unsafe {
        if curscript + 1 == NSCRIPT as c_int {
            emsg(gettext((&raw const e_nesting).cast()));
            return;
        }

        if check_secure() {
            return;
        }

        if ignore_script {
            return;
        }

        curscript += 1;
        let namebuff_ptr = (&raw mut NameBuff).cast::<c_char>();
        expand_env(name, namebuff_ptr, MAXPATHL as c_int);
        let scriptin_ptr = (&raw mut scriptin)
            .cast::<FileDescriptor>()
            .add(curscript as usize);
        let error = file_open(scriptin_ptr, namebuff_ptr, K_FILE_READ_ONLY, 0);
        if error != 0 {
            semsg!(
                gettext((&raw const e_notopen_2).cast()),
                name,
                uv_strerror(error),
            );
            curscript -= 1;
            return;
        }
        save_typebuf();

        if directly {
            let mut oa = oparg_T::new();
            let save_State = State;
            let save_restart_edit = restart_edit;
            let save_finish_op = finish_op;
            let save_msg_scroll = msg_scroll;

            State = MODE_NORMAL;
            msg_scroll = 0;
            restart_edit = 0;
            finish_op = false;

            let oldcurscript = curscript;
            loop {
                update_topline_cursor();
                normal_cmd(&raw mut oa, false);
                vpeekc();
                if curscript < oldcurscript {
                    break;
                }
            }

            State = save_State;
            msg_scroll = save_msg_scroll;
            restart_edit = save_restart_edit;
            finish_op = save_finish_op;
        }
    }
}

unsafe fn closescript() {
    unsafe {
        assert!(curscript >= 0);
        free_typebuf();
        typebuf = *(&raw const saved_typebuf)
            .cast::<typebuf_T>()
            .add(curscript as usize);

        let scriptin_ptr = (&raw mut scriptin)
            .cast::<FileDescriptor>()
            .add(curscript as usize);
        file_close(scriptin_ptr, false);
        curscript -= 1;
    }
}

pub unsafe fn close_all_scripts() {
    unsafe {
        while curscript >= 0 {
            closescript();
        }
    }
}

pub unsafe fn open_scriptin(scriptin_name: *mut c_char) -> bool {
    unsafe {
        assert!(curscript == -1);
        curscript += 1;

        let scriptin0 = (&raw mut scriptin).cast::<FileDescriptor>();
        let error = if strcmp(scriptin_name, c"-".as_ptr()) == 0 {
            file_open_stdin(scriptin0)
        } else {
            file_open(
                scriptin0,
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
                gettext(c"Cannot open for reading: \"%s\": %s\n".as_ptr()),
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
            curscript -= 1;
            return false;
        }
        save_typebuf();

        true
    }
}

/// Return true when reading keys from a script file.
pub unsafe fn using_script() -> c_int {
    unsafe { c_int::from(curscript >= 0) }
}

/// Called just before doing a blocking wait.
pub unsafe fn before_blocking() {
    unsafe {
        updatescript(0);
        if may_garbage_collect {
            garbage_collect(false);
        }
    }
}

unsafe fn updatescript(c: c_int) {
    unsafe {
        if c != 0 && !scriptout.is_null() {
            fputc(c, scriptout);
        }
        let idle = c == 0;
        if idle
            || (p_uc > 0 && {
                UPDATESCRIPT_COUNT += 1;
                (UPDATESCRIPT_COUNT as OptInt) >= p_uc
            })
        {
            ml_sync_all(c_int::from(idle), 1, p_fs != 0 || idle);
            UPDATESCRIPT_COUNT = 0;
        }
    }
}

/// Merge `modifiers` into `c_arg`.
pub unsafe fn merge_modifiers(c_arg: c_int, modifiers: *mut c_int) -> c_int {
    unsafe {
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
}

unsafe fn add_byte_to_showcmd(byte: u8) {
    unsafe {
        if p_sc == 0 || msg_silent != 0 {
            return;
        }

        if !gotchars_add_byte(&raw mut SHOWCMD_STATE, byte) {
            return;
        }

        let buflen = SHOWCMD_STATE.buflen;
        SHOWCMD_STATE.buf[buflen] = NUL as u8;
        SHOWCMD_STATE.buflen = 0;

        let mut modifiers: c_int = 0;
        let mut c: c_int = NUL;

        let mut ptr = (&raw const SHOWCMD_STATE.buf).cast::<u8>();
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
                let mod_c = merge_modifiers(c, &raw mut modifiers_after);
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
pub unsafe fn vgetc() -> c_int {
    unsafe {
        let mut c: c_int;
        let mut buf = [0u8; MB_MAXBYTES + 1];

        if may_garbage_collect && want_garbage_collect {
            garbage_collect(false);
        }

        if can_get_ungot() {
            c = ungot.c;
            ungot.c = -1;
            mod_mask = ungot.mod_mask;
            mouse_grid = ungot.mouse_grid;
            mouse_row = ungot.mouse_row;
            mouse_col = ungot.mouse_col;
        } else {
            mod_mask = 0;
            vgetc_mod_mask = 0;
            vgetc_char = 0;

            last_recorded_len -= LAST_VGETC_RECORDED_LEN;

            loop {
                let mut did_inc = false;
                if mod_mask != 0 {
                    no_mapping += 1;
                    allow_keys += 1;
                    did_inc = true;
                }
                c = vgetorpeek(true);
                if did_inc {
                    no_mapping -= 1;
                    allow_keys -= 1;
                }

                if c == K_SPECIAL as c_int {
                    let save_allow_keys = allow_keys;
                    no_mapping += 1;
                    allow_keys = 0;
                    let c2 = vgetorpeek(true);
                    c = vgetorpeek(true);
                    no_mapping -= 1;
                    allow_keys = save_allow_keys;
                    if c2 == KS_MODIFIER as c_int {
                        mod_mask = c;
                        continue;
                    }
                    c = to_special(c2, c);
                }

                let n = mb_byte2len_check(c);
                if n > 1 {
                    no_mapping += 1;
                    buf[0] = c as u8;
                    for slot in buf.iter_mut().take(n as usize).skip(1) {
                        *slot = vgetorpeek(true) as u8;
                        if *slot == K_SPECIAL {
                            vgetorpeek(true);
                            vgetorpeek(true);
                        }
                    }
                    no_mapping -= 1;
                    c = utf_ptr2char(buf.as_ptr().cast());
                }

                if no_mapping == 0
                    && KeyTyped
                    && mod_mask == MOD_MASK_ALT as c_int
                    && (State & MODE_TERMINAL) == 0
                    && !is_mouse_key(c)
                {
                    mod_mask = 0;
                    requeue_key(c, 0, 3, true);
                    ins_typebuf(
                        ESC_STR.as_ptr().cast_mut(),
                        KeyNoremap,
                        0,
                        !KeyTyped,
                        cmd_silent,
                    );
                    continue;
                }

                if vgetc_char == 0 {
                    vgetc_mod_mask = mod_mask;
                    vgetc_char = c;
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
                        if mod_mask == MOD_MASK_SHIFT as c_int {
                            c = K_S_HOME;
                            mod_mask = 0;
                        } else if mod_mask == MOD_MASK_CTRL as c_int {
                            c = K_C_HOME;
                            mod_mask = 0;
                        } else {
                            c = K_HOME;
                        }
                    }
                    K_XEND | K_ZEND => {
                        if mod_mask == MOD_MASK_SHIFT as c_int {
                            c = K_S_END;
                            mod_mask = 0;
                        } else if mod_mask == MOD_MASK_CTRL as c_int {
                            c = K_C_END;
                            mod_mask = 0;
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

            LAST_VGETC_RECORDED_LEN = last_recorded_len;
        }

        may_garbage_collect = false;

        let okb = &raw mut on_key_buf;
        (*okb).push(NUL as c_char);
        if on_key_blocked == 0 && nlua_exec_on_key(c, (*okb).as_mut_ptr()) {
            if c == K_COMMAND {
                xfree(getcmdkeycmd(NUL, 0, false).cast());
            } else if c == K_LUA {
                map_execute_lua(false, true);
            } else if c == K_PASTE_START {
                paste_repeat(0);
            }
            c = K_IGNORE;
        }
        *okb = Vec::new();

        if c != K_IGNORE {
            state_no_longer_safe(c"key typed".as_ptr());
        }

        c
    }
}

/// Like `vgetc()`, but never return a NUL when called recursively.
pub unsafe fn safe_vgetc() -> c_int {
    unsafe {
        let mut c = vgetc();
        if c == NUL {
            c = get_keystroke(ptr::null_mut());
        }
        c
    }
}

/// Like `safe_vgetc()`, but loop to handle `K_IGNORE`.
pub unsafe fn plain_vgetc() -> c_int {
    unsafe {
        loop {
            let c = safe_vgetc();
            if c != K_IGNORE && c != K_VER_SCROLLBAR && c != K_HOR_SCROLLBAR && c != K_MOUSEMOVE {
                return c;
            }
        }
    }
}

/// Check if a character is available, such that `vgetc()` will not block.
pub unsafe fn vpeekc() -> c_int {
    unsafe {
        if can_get_ungot() {
            return ungot.c;
        }
        vgetorpeek(false)
    }
}

/// Check if any character is available, also half an escape sequence.
pub unsafe fn vpeekc_any() -> c_int {
    unsafe {
        let mut c = vpeekc();
        if c == NUL && typebuf.tb_len > 0 {
            c = ESC;
        }
        c
    }
}

/// Call `vpeekc()` without causing anything to be mapped.
pub unsafe fn char_avail() -> bool {
    unsafe {
        if test_disable_char_avail {
            return false;
        }
        no_mapping += 1;
        let retval = vpeekc();
        no_mapping -= 1;
        retval != NUL
    }
}

unsafe fn getchar_common(argvars: *mut typval_T, rettv: *mut typval_T, mut allow_number: bool) {
    unsafe {
        let mut n: varnumber_T;
        let called_emsg_start = called_emsg;
        let mut error = false;
        let mut simplify = true;
        let mut cursor_flag: c_char = NUL as c_char;

        if (*argvars).v_type != VarType::VAR_UNKNOWN
            && tv_check_for_opt_dict_arg(argvars, 1) == FAIL
        {
            return;
        }

        let arg1 = argvars.add(1);
        if (*argvars).v_type != VarType::VAR_UNKNOWN && (*arg1).v_type == VarType::VAR_DICT {
            let d = (*arg1).vval.v_dict;

            if allow_number {
                allow_number = tv_dict_get_bool(d, c"number".as_ptr(), 1) != 0;
            } else if tv_dict_has_key(d, c"number".as_ptr()) {
                semsg!(gettext((&raw const e_invarg2).cast()), c"number".as_ptr());
            }

            simplify = tv_dict_get_bool(d, c"simplify".as_ptr(), 1) != 0;

            let cursor_str = tv_dict_get_string(d, c"cursor".as_ptr(), false);
            if !cursor_str.is_null() {
                if strcmp(cursor_str, c"hide".as_ptr()) != 0
                    && strcmp(cursor_str, c"keep".as_ptr()) != 0
                    && strcmp(cursor_str, c"msg".as_ptr()) != 0
                {
                    semsg!(
                        gettext((&raw const e_invargNval).cast()),
                        c"cursor".as_ptr(),
                        cursor_str,
                    );
                } else {
                    cursor_flag = *cursor_str;
                }
            }
        }

        if called_emsg != called_emsg_start {
            return;
        }

        if cursor_flag == b'h' as c_char {
            ui_busy_start();
        }

        no_mapping += 1;
        allow_keys += 1;
        block_on_key();
        if !simplify {
            no_reduce_keys += 1;
        }
        atom_payload_start();
        loop {
            if cursor_flag == b'm' as c_char || (cursor_flag == NUL as c_char && msg_col > 0) {
                ui_cursor_goto(msg_row, msg_col);
            }

            if (*argvars).v_type == VarType::VAR_UNKNOWN
                || ((*argvars).v_type == VarType::VAR_NUMBER && (*argvars).vval.v_number == -1)
            {
                if !char_avail() {
                    ui_flush();
                    let events = main_loop_events();
                    input_get(ptr::null_mut(), 0, -1, typebuf.tb_change_cnt, events);
                    if input_available() == 0 && !multiqueue_empty(events) {
                        state_handle_k_event();
                        continue;
                    }
                }
                n = varnumber_T::from(safe_vgetc());
            } else if tv_get_bool_chk(argvars, &raw mut error) != 0 {
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
        no_mapping -= 1;
        allow_keys -= 1;
        unblock_on_key();
        if !simplify {
            no_reduce_keys -= 1;
        }

        if cursor_flag == b'h' as c_char {
            ui_busy_stop();
        }

        set_vim_var_nr(VV_MOUSE_WIN, 0);
        set_vim_var_nr(VV_MOUSE_WINID, 0);
        set_vim_var_nr(VV_MOUSE_LNUM, 0);
        set_vim_var_nr(VV_MOUSE_COL, 0);

        let ni = n as c_int;
        if n != 0 && (!allow_number || is_special(ni) || mod_mask != 0) {
            let mut temp = [0 as c_char; 10];
            let mut i: usize = 0;

            if mod_mask != 0 {
                temp[i] = K_SPECIAL as c_char;
                temp[i + 1] = KS_MODIFIER as c_char;
                temp[i + 2] = mod_mask as c_char;
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
            (*rettv).v_type = VarType::VAR_STRING;
            (*rettv).vval.v_string = xmemdupz(temp.as_ptr().cast(), i).cast();

            if is_mouse_key(ni) {
                let mut row = mouse_row;
                let mut col = mouse_col;
                let mut grid = mouse_grid;
                let mut lnum: linenr_T = 0;

                if row >= 0 && col >= 0 {
                    let mut winnr: c_int = 1;
                    let win = mouse_find_win_inner(&raw mut grid, &raw mut row, &raw mut col);
                    if win.is_null() {
                        return;
                    }
                    mouse_comp_pos(win, &raw mut row, &raw mut col, &raw mut lnum);
                    let mut wp = firstwin;
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
            (*rettv).v_type = VarType::VAR_STRING;
        } else {
            (*rettv).vval.v_number = n;
        }
    }
}

/// `"getchar()"` function.
pub unsafe fn f_getchar(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        getchar_common(argvars, rettv, true);
    }
}

/// `"getcharstr()"` function.
pub unsafe fn f_getcharstr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        getchar_common(argvars, rettv, false);
    }
}

/// `"getcharmod()"` function.
pub unsafe fn f_getcharmod(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(mod_mask);
    }
}

unsafe fn put_string_in_typebuf(
    offset: c_int,
    slen: c_int,
    string: *mut u8,
    new_slen: c_int,
) -> c_int {
    unsafe {
        let extra = new_slen - slen;
        *string.add(new_slen as usize) = NUL as u8;
        if extra < 0 {
            del_typebuf(-extra, offset);
        } else if extra > 0
            && ins_typebuf(
                string.add(slen as usize).cast::<c_char>(),
                REMAP_YES,
                offset,
                false,
                false,
            ) == FAIL
        {
            return FAIL;
        }
        ptr::copy(
            string,
            typebuf.tb_buf.add((typebuf.tb_off + offset) as usize),
            new_slen as usize,
        );
        OK
    }
}

unsafe fn at_ins_compl_key() -> bool {
    unsafe {
        let p = typebuf.tb_buf.add(typebuf.tb_off as usize);
        let mut c = *p as c_int;

        if typebuf.tb_len > 3
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

unsafe fn check_simplify_modifier(max_offset: c_int) -> c_int {
    unsafe {
        if (State & MODE_TERMINAL) != 0 || no_reduce_keys > 0 {
            return 0;
        }

        for offset in 0..max_offset {
            if offset + 3 >= typebuf.tb_len {
                break;
            }
            let tp = typebuf.tb_buf.add((typebuf.tb_off + offset) as usize);
            if *tp == K_SPECIAL && *tp.add(1) == KS_MODIFIER {
                let mut modifier = *tp.add(2) as c_int;
                let c = *tp.add(3) as c_int;
                let new_c = merge_modifiers(c, &raw mut modifier);

                if new_c != c {
                    if offset == 0 {
                        vgetc_char = c;
                        vgetc_mod_mask = *tp.add(2) as c_int;
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
                        if put_string_in_typebuf(offset, 4, new_string.as_mut_ptr(), len) == FAIL {
                            return -1;
                        }
                    } else {
                        *tp.add(2) = modifier as u8;
                        if put_string_in_typebuf(offset + 3, 1, new_string.as_mut_ptr(), len)
                            == FAIL
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

unsafe fn char_iter(itp: *mut *const u8, nomap: c_int) -> c_int {
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

unsafe fn handle_mapping(
    keylenp: *mut c_int,
    timedout: *const bool,
    mapdepth: *mut c_int,
    advance: bool,
) -> c_int {
    unsafe {
        let mut mp: *mut mapblock_T = ptr::null_mut();
        let mut mp_match: *mut mapblock_T = ptr::null_mut();
        let mut mp_match_len: c_int = 0;
        let mut tb_match_len: c_int = 0;
        let mut max_mlen: c_int = 0;
        let mut keylen = *keylenp;
        let local_State = get_real_state();
        let mut is_plug_map = false;

        let tb_start = typebuf.tb_buf.add(typebuf.tb_off as usize);
        if typebuf.tb_len >= 3
            && *tb_start == K_SPECIAL
            && *tb_start.add(1) == KS_EXTRA
            && *tb_start.add(2) == KE_PLUG
        {
            is_plug_map = true;
        }

        let mut tb_b1 = *tb_start as c_int;
        if no_mapping == 0
            && (no_zero_mapping == 0 || tb_b1 != b'0' as c_int)
            && (typebuf.tb_maplen == 0
                || is_plug_map
                || (*typebuf.tb_noremap.add(typebuf.tb_off as usize) & (RM_NONE | RM_ABBR)) == 0)
            && !(p_paste != 0 && (State & (MODE_INSERT | MODE_CMDLINE)) != 0)
            && !(State == MODE_HITRETURN && (tb_b1 == CAR || tb_b1 == b' ' as c_int))
            && State != MODE_ASKMORE
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
                    (State & (MODE_CMDLINE | MODE_INSERT)) == 0 && get_real_state() != MODE_SELECT,
                );
                if orig_tb_c1 != tb_c1 {
                    let mut cbuf = [0 as c_char; 6];
                    utf_char2bytes(tb_c1, cbuf.as_mut_ptr());
                    tb_b1 = cbuf[0] as u8 as c_int;
                }
                nolmaplen = 0;
            }

            mp = ptr::null_mut();
            for cand in get_buf_maphash_list(local_State, tb_b1)
                .iter()
                .chain(get_maphash_list(local_State, tb_b1))
            {
                let cand: *mut mapblock_T = (&raw const **cand).cast_mut();
                let mut key_iter: *const u8 = (*cand).m_keys.as_ptr().cast::<u8>();

                if char_iter(&raw mut key_iter, 0) == tb_c1
                    && ((*cand).m_mode & local_State) != 0
                    && (((*cand).m_mode & MODE_LANGMAP) == 0 || typebuf.tb_maplen == 0)
                {
                    let mut nomap = nolmaplen;
                    let mut modifiers: c_int = 0;
                    let mut tmp_tb_iter = tb_iter;
                    let mut mlen =
                        key_iter.offset_from((*cand).m_keys.as_ptr().cast::<u8>()) as c_int;
                    let mut tb_mlen =
                        (tmp_tb_iter.offset_from(typebuf.tb_buf) as c_int) - typebuf.tb_off;

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
                            } else if merge_modifiers(c2, &raw mut modifiers) == c2 {
                                langmap_adjust(&mut c2, true);
                            }
                            modifiers = 0;
                        }

                        let key_char = char_iter(&raw mut key_iter, tb_nomap);
                        if key_char != c2 {
                            break;
                        }

                        mlen = key_iter.offset_from((*cand).m_keys.as_ptr().cast::<u8>()) as c_int;
                        tb_mlen =
                            (tmp_tb_iter.offset_from(typebuf.tb_buf) as c_int) - typebuf.tb_off;
                    }

                    keylen = (*cand).m_keylen;
                    if mlen == keylen || (mlen == typebuf.tb_len && typebuf.tb_len < keylen) {
                        let mut s = typebuf.tb_noremap.add(typebuf.tb_off as usize);
                        let m_keys_u8 = (*cand).m_keys.as_ptr().cast::<u8>();
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

                        if keylen > typebuf.tb_len {
                            if !*timedout && !(!mp_match.is_null() && (*mp_match).m_nowait != 0) {
                                keylen = KEYLEN_PART_MAP;
                                mp = cand;
                                break;
                            }
                        } else if keylen > mp_match_len
                            || (keylen == mp_match_len
                                && !mp_match.is_null()
                                && ((*mp_match).m_mode & MODE_LANGMAP) == 0
                                && ((*cand).m_mode & MODE_LANGMAP) != 0)
                        {
                            mp_match = cand;
                            mp_match_len = keylen;
                            tb_match_len = tb_mlen;
                        }
                    } else {
                        max_mlen = max_mlen.max(mlen);
                    }
                }
            }

            if keylen != KEYLEN_PART_MAP && !mp_match.is_null() {
                mp = mp_match;
                keylen = tb_match_len;
            }
        }

        if (mp.is_null() || max_mlen > mp_match_len) && keylen != KEYLEN_PART_MAP {
            if no_mapping == 0 || allow_keys != 0 {
                if tb_b1 == K_SPECIAL as c_int
                    && (typebuf.tb_len < 2
                        || (*typebuf.tb_buf.add((typebuf.tb_off + 1) as usize) == KS_MODIFIER
                            && typebuf.tb_len < 4))
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
            if keylen == 0 && mp.is_null() {
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
                assert!(!mp.is_null());
                keylen = tb_match_len;
            }
        }

        if keylen >= 0 && keylen <= typebuf.tb_len {
            let mut map_str: *mut c_char;

            if keylen > typebuf.tb_maplen && ((*mp).m_mode & MODE_LANGMAP) == 0 {
                gotchars(
                    typebuf
                        .tb_buf
                        .add((typebuf.tb_off + typebuf.tb_maplen) as usize),
                    (keylen - typebuf.tb_maplen) as usize,
                );
                atom_map_start((*mp).m_keys.as_ptr(), (*mp).m_keylen as usize, !advance);
            }

            cmd_silent = typebuf.tb_silent > 0;
            del_typebuf(keylen, 0);

            *mapdepth += 1;
            if (*mapdepth as OptInt) >= p_mmd {
                emsg(gettext(E_RECURSIVE_MAPPING.as_ptr()));
                if (State & MODE_CMDLINE) != 0 {
                    redrawcmdline();
                } else {
                    setcursor();
                }
                flush_buffers(FLUSH_MINIMAL);
                *mapdepth = 0;
                *keylenp = keylen;
                return map_result_T::map_result_fail as c_int;
            }

            if Visual.active && Visual.select && ((*mp).m_mode & MODE_VISUAL) != 0 {
                Visual.select = false;
                ins_typebuf(
                    K_SELECT_STRING.as_ptr().cast::<c_char>().cast_mut(),
                    REMAP_NONE,
                    0,
                    true,
                    false,
                );
            }

            let save_m_expr = (*mp).m_expr != 0;
            let save_m_noremap = (*mp).m_noremap;
            let save_m_silent = (*mp).m_silent != 0;
            let mut save_m_keys: Option<std::ffi::CString> = None;
            let mut save_alt_m_keys: Option<std::ffi::CString> = None;
            let save_alt_m_keylen = if !(*mp).m_alt.is_null() {
                (*(*mp).m_alt).m_keylen
            } else {
                0
            };

            if save_m_expr {
                let save_vgetc_busy = vgetc_busy;
                let save_may_garbage_collect = may_garbage_collect;
                let prev_did_emsg = did_emsg;

                vgetc_busy = 0;
                may_garbage_collect = false;

                save_m_keys = Some((*mp).m_keys.clone());
                save_alt_m_keys = if !(*mp).m_alt.is_null() {
                    Some((*(*mp).m_alt).m_keys.clone())
                } else {
                    None
                };
                map_str = eval_map_expr(mp, NUL);

                if map_str.is_null() || *map_str == NUL as c_char {
                    if prev_did_emsg != did_emsg {
                        xfree(map_str.cast());
                        let buf: [u8; 4] = [K_SPECIAL, KS_EXTRA, KE_IGNORE, 0];
                        map_str = xmemdupz(buf.as_ptr().cast(), 3).cast();
                        if (State & MODE_CMDLINE) != 0 {
                            msg_didout = true;
                            msg_row = msg_row.max(cmdline_row);
                            redrawcmd();
                        }
                    } else if (State & (MODE_NORMAL | MODE_INSERT)) != 0 {
                        setcursor();
                    }
                }

                vgetc_busy = save_vgetc_busy;
                may_garbage_collect = save_may_garbage_collect;
            } else {
                map_str = (*mp).m_str.as_ptr().cast_mut();
            }

            let i = if map_str.is_null() {
                FAIL
            } else {
                if keylen > typebuf.tb_maplen && ((*mp).m_mode & MODE_LANGMAP) != 0 {
                    gotchars(map_str.cast::<u8>(), strlen(map_str));
                }

                let noremap = if save_m_noremap != REMAP_YES {
                    save_m_noremap
                } else if if save_m_expr {
                    strncmp(
                        map_str,
                        save_m_keys.as_ref().unwrap().as_ptr(),
                        keylen as usize,
                    ) == 0
                        || save_alt_m_keys.as_ref().is_some_and(|alt| {
                            strncmp(map_str, alt.as_ptr(), save_alt_m_keylen as usize) == 0
                        })
                } else {
                    strncmp(map_str, (*mp).m_keys.as_ptr(), keylen as usize) == 0
                        || (!(*mp).m_alt.is_null()
                            && strncmp(
                                map_str,
                                (*(*mp).m_alt).m_keys.as_ptr(),
                                (*(*mp).m_alt).m_keylen as usize,
                            ) == 0)
                } {
                    REMAP_SKIP
                } else {
                    REMAP_YES
                };

                let res = ins_typebuf(map_str, noremap, 0, true, cmd_silent || save_m_silent);
                if save_m_expr {
                    xfree(map_str.cast());
                }
                res
            };
            drop(save_m_keys);
            drop(save_alt_m_keys);
            *keylenp = keylen;
            if i == FAIL {
                return map_result_T::map_result_fail as c_int;
            }
            return map_result_T::map_result_retry as c_int;
        }

        *keylenp = keylen;
        map_result_T::map_result_nomatch as c_int
    }
}

/// Unget one character (can only be done once!).
pub unsafe fn vungetc(c: c_int) {
    unsafe {
        ungot = UngotKey {
            c,
            mod_mask,
            mouse_grid,
            mouse_row,
            mouse_col,
            stuffed: KeyStuffed != 0,
        };
    }
}

/// When peeking and not getting a character, `reg_executing` cannot be cleared yet.
pub unsafe fn check_end_reg_executing(advance: bool) {
    unsafe {
        if reg_executing != 0 && (typebuf.tb_maplen == 0 || pending_end_reg_executing) {
            if advance {
                reg_executing = 0;
                pending_end_reg_executing = false;
            } else {
                pending_end_reg_executing = true;
            }
        }
    }
}

unsafe fn vgetorpeek(advance: bool) -> c_int {
    unsafe {
        let mut c: c_int;
        let mut timedout = false;
        let mut mapdepth: c_int = 0;
        let mut mode_deleted = false;

        if vgetc_busy > 0 && ex_normal_busy == 0 {
            return NUL;
        }

        vgetc_busy += 1;

        if advance {
            KeyStuffed = 0;
            typebuf_was_empty = false;
        }

        init_typebuf();
        start_stuff();
        check_end_reg_executing(advance);
        loop {
            if typeahead_char != 0 {
                c = typeahead_char;
                if advance {
                    typeahead_char = 0;
                }
            } else {
                c = read_readbuffers(advance);
            }
            if c != NUL && !got_int {
                if advance {
                    KeyStuffed = 1;
                }
                if typebuf.tb_no_abbr_cnt == 0 {
                    typebuf.tb_no_abbr_cnt = 1;
                }
            } else {
                loop {
                    check_end_reg_executing(advance);
                    if typebuf.tb_maplen != 0 {
                        line_breakcheck();
                    } else {
                        if ((mapped_ctrl_c | (*curbuf).b_mapped_ctrl_c) & get_real_state()) != 0 {
                            ctrl_c_interrupts = false;
                        }
                        os_breakcheck();
                        ctrl_c_interrupts = true;
                    }
                    let mut keylen: c_int = 0;
                    if got_int {
                        c = inchar(typebuf.tb_buf, typebuf.tb_buflen - 1, 0);
                        if (c != 0 || typebuf.tb_maplen != 0)
                            && (State & (MODE_INSERT | MODE_CMDLINE)) != 0
                        {
                            c = ESC;
                        } else {
                            c = Ctrl_C;
                        }
                        flush_buffers(FLUSH_INPUT);

                        if advance {
                            *typebuf.tb_buf = c as u8;
                            gotchars(typebuf.tb_buf, 1);
                        }
                        cmd_silent = false;

                        break;
                    } else if typebuf.tb_len > 0 {
                        let result = handle_mapping(
                            &raw mut keylen,
                            &raw const timedout,
                            &raw mut mapdepth,
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
                            c = *typebuf.tb_buf.add(typebuf.tb_off as usize) as c_int;
                            if advance {
                                cmd_silent = typebuf.tb_silent > 0;
                                if typebuf.tb_maplen > 0 {
                                    KeyTyped = false;
                                } else {
                                    KeyTyped = true;
                                    gotchars(typebuf.tb_buf.add(typebuf.tb_off as usize), 1);
                                }
                                KeyNoremap =
                                    *typebuf.tb_noremap.add(typebuf.tb_off as usize) as c_int;
                                del_typebuf(1, 0);
                            }
                            break;
                        }
                    }

                    c = 0;
                    let mut new_wcol = (*curwin).w_wcol;
                    let mut new_wrow = (*curwin).w_wrow;
                    if advance
                        && typebuf.tb_len == 1
                        && *typebuf.tb_buf.add(typebuf.tb_off as usize) == ESC as u8
                        && no_mapping == 0
                        && ex_normal_busy == 0
                        && typebuf.tb_maplen == 0
                        && (State & MODE_INSERT) != 0
                        && (p_timeout != 0 || (keylen == KEYLEN_PART_KEY && p_ttimeout != 0))
                        && {
                            c = inchar(
                                typebuf
                                    .tb_buf
                                    .add((typebuf.tb_off + typebuf.tb_len) as usize),
                                3,
                                25,
                            );
                            c == 0
                        }
                    {
                        if mode_displayed {
                            unshowmode(true);
                            mode_deleted = true;
                        }
                        validate_cursor(curwin);
                        let old_wcol = (*curwin).w_wcol;
                        let old_wrow = (*curwin).w_wrow;

                        if (*curwin).w_cursor.col != 0 {
                            let mut col: colnr_T = 0;
                            if (*curwin).w_wcol > 0 {
                                if Ins.did_ai
                                    && *skipwhite(
                                        get_cursor_line_ptr().add((*curwin).w_cursor.col as usize),
                                    ) == NUL as c_char
                                {
                                    (*curwin).w_wcol = 0;
                                    let ptr = get_cursor_line_ptr();
                                    let endptr = ptr.add((*curwin).w_cursor.col as usize);

                                    let mut csarg = CharsizeArg::new();
                                    let cstype: CSType = init_charsize_arg(
                                        &raw mut csarg,
                                        curwin,
                                        (*curwin).w_cursor.lnum,
                                        ptr,
                                    );
                                    let mut ci: StrCharInfo = utf_ptr2StrCharInfo(ptr);
                                    let mut vcol: c_int = 0;
                                    while ci.ptr < endptr.cast_const() {
                                        if !ascii_iswhite(ci.chr.value) {
                                            (*curwin).w_wcol = vcol;
                                        }
                                        let cli: ClusterInfo = utf_ClusterInfo(ci);
                                        vcol += win_charsize(
                                            cstype,
                                            vcol,
                                            ci.ptr.cast_mut(),
                                            ci.chr.value,
                                            &raw mut csarg,
                                            cli.cells,
                                        )
                                        .width;
                                        ci = cli.next;
                                    }

                                    (*curwin).w_wrow = (*curwin).w_cline_row
                                        + (*curwin).w_wcol / (*curwin).w_view_width;
                                    (*curwin).w_wcol %= (*curwin).w_view_width;
                                    (*curwin).w_wcol += win_col_off(curwin);
                                    col = 0;
                                } else {
                                    (*curwin).w_wcol -= 1;
                                    col = (*curwin).w_cursor.col - 1;
                                }
                            } else if (*curwin).w_onebuf_opt.wo_wrap != 0 && (*curwin).w_wrow != 0 {
                                (*curwin).w_wrow -= 1;
                                (*curwin).w_wcol = (*curwin).w_view_width - 1;
                                col = (*curwin).w_cursor.col - 1;
                            }
                            if col > 0 && (*curwin).w_wcol > 0 {
                                let ptr = get_cursor_line_ptr();
                                col -= utf_head_off(ptr, ptr.add(col as usize));
                                if utf_ptr2cells(ptr.add(col as usize)) > 1 {
                                    (*curwin).w_wcol -= 1;
                                }
                            }
                        }
                        setcursor();
                        ui_flush();
                        new_wcol = (*curwin).w_wcol;
                        new_wrow = (*curwin).w_wrow;
                        (*curwin).w_wcol = old_wcol;
                        (*curwin).w_wrow = old_wrow;
                    }
                    if c < 0 {
                        continue;
                    }

                    for n in 1..=c {
                        *typebuf.tb_noremap.add((typebuf.tb_off + n) as usize) = RM_YES;
                    }
                    typebuf.tb_len += c;

                    if typebuf.tb_len >= typebuf.tb_maplen + MAXMAPLEN as c_int {
                        timedout = true;
                        continue;
                    }

                    if ex_normal_busy > 0 {
                        if typebuf.tb_len > 0 {
                            timedout = true;
                            continue;
                        }

                        c = if (State & MODE_CMDLINE) != 0
                            || (cmdwin_type > 0 && VGETORPEEK_TC == ESC)
                        {
                            Ctrl_C
                        } else {
                            ESC
                        };
                        VGETORPEEK_TC = c;

                        if advance {
                            typebuf_was_empty = true;
                        }
                        typebuf.tb_no_abbr_cnt = 0;
                        break;
                    }

                    if ((State & MODE_INSERT) != 0 || p_lz != 0)
                        && (State & MODE_CMDLINE) == 0
                        && advance
                        && must_redraw != 0
                        && !need_wait_return
                    {
                        update_screen();
                        setcursor();
                    }

                    let mut showcmd_idx: c_int = 0;
                    let mut showing_partial = false;
                    if typebuf.tb_len > 0 && advance {
                        if ((State & (MODE_NORMAL | MODE_INSERT)) != 0 || State == MODE_LANGMAP)
                            && State != MODE_HITRETURN
                        {
                            if (State & MODE_INSERT) != 0
                                && utf_ptr2cells(
                                    typebuf
                                        .tb_buf
                                        .add((typebuf.tb_off + typebuf.tb_len - 1) as usize)
                                        .cast::<c_char>(),
                                ) == 1
                            {
                                edit_putchar(
                                    *typebuf
                                        .tb_buf
                                        .add((typebuf.tb_off + typebuf.tb_len - 1) as usize)
                                        as c_int,
                                    false,
                                );
                                setcursor();
                                showing_partial = true;
                            }
                            let old_wcol = (*curwin).w_wcol;
                            let old_wrow = (*curwin).w_wrow;
                            (*curwin).w_wcol = new_wcol;
                            (*curwin).w_wrow = new_wrow;
                            push_showcmd();
                            if typebuf.tb_len > SHOWCMD_COLS {
                                showcmd_idx = typebuf.tb_len - SHOWCMD_COLS;
                            }
                            while showcmd_idx < typebuf.tb_len {
                                add_byte_to_showcmd(
                                    *typebuf.tb_buf.add((typebuf.tb_off + showcmd_idx) as usize),
                                );
                                showcmd_idx += 1;
                            }
                            (*curwin).w_wcol = old_wcol;
                            (*curwin).w_wrow = old_wrow;
                        }

                        if (State & MODE_CMDLINE) != 0
                            && !(*get_cmdline_info()).cmdbuff.is_null()
                            && cmdline_star == 0
                        {
                            let p = typebuf
                                .tb_buf
                                .add((typebuf.tb_off + typebuf.tb_len - 1) as usize)
                                .cast::<c_char>();
                            if utf_ptr2cells(p) == 1 && (*p as u8) < 128 {
                                putcmdline(*p, false);
                                showing_partial = true;
                            }
                        }
                    }

                    if typebuf.tb_len == 0 {
                        timedout = false;
                    }

                    let mut wait_time: c_long = 0;
                    if advance {
                        if typebuf.tb_len == 0
                            || !(p_timeout != 0 || (p_ttimeout != 0 && keylen == KEYLEN_PART_KEY))
                        {
                            wait_time = -1;
                        } else if keylen == KEYLEN_PART_KEY && p_ttm >= 0 {
                            wait_time = p_ttm as c_long;
                        } else {
                            wait_time = p_tm as c_long;
                        }
                    }

                    let wait_tb_len = typebuf.tb_len;
                    c = inchar(
                        typebuf
                            .tb_buf
                            .add((typebuf.tb_off + typebuf.tb_len) as usize),
                        typebuf.tb_buflen - typebuf.tb_off - typebuf.tb_len - 1,
                        wait_time,
                    );

                    if showcmd_idx != 0 {
                        pop_showcmd();
                    }
                    if showing_partial {
                        if (State & MODE_INSERT) != 0 {
                            edit_unputchar();
                        }
                        if (State & MODE_CMDLINE) != 0 && !(*get_cmdline_info()).cmdbuff.is_null() {
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
                        while *typebuf
                            .tb_buf
                            .add((typebuf.tb_off + typebuf.tb_len) as usize)
                            != NUL as u8
                        {
                            *typebuf
                                .tb_noremap
                                .add((typebuf.tb_off + typebuf.tb_len) as usize) = RM_YES;
                            typebuf.tb_len += 1;
                        }
                    }
                }
            }

            if !(c < 0 || (advance && c == NUL)) {
                break;
            }
        }

        if advance && p_smd != 0 && msg_silent == 0 && (State & MODE_INSERT) != 0 {
            if c == ESC && !mode_deleted && no_mapping == 0 && mode_displayed {
                if typebuf.tb_len != 0 && !KeyTyped {
                    redraw_cmdline = true;
                } else {
                    unshowmode(false);
                }
            } else if c != ESC && mode_deleted {
                if typebuf.tb_len != 0 && !KeyTyped {
                    redraw_cmdline = true;
                } else {
                    showmode();
                }
            }
        }

        if timedout && c == ESC {
            gotchars_ignore();
        }

        vgetc_busy -= 1;

        c
    }
}

unsafe fn inchar(buf: *mut u8, maxlen: c_int, wait_time: c_long) -> c_int {
    unsafe {
        let mut len: c_int = 0;
        let mut retesc: c_int = 0;
        let tb_change_cnt = typebuf.tb_change_cnt;

        if wait_time == -1 || wait_time > 100 {
            ui_flush();
        }

        if State != MODE_HITRETURN {
            did_outofmem_msg = false;
            did_swapwrite_msg = false;
        }

        let mut read_size: isize = -1;
        while curscript >= 0 && read_size <= 0 && !ignore_script {
            let mut script_char: c_char = 0;
            let scriptin_ptr = (&raw mut scriptin)
                .cast::<FileDescriptor>()
                .add(curscript as usize);
            if got_int || {
                read_size = file_read(scriptin_ptr, &raw mut script_char, 1);
                read_size != 1
            } {
                closescript();
                if got_int {
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
            if got_int {
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
            typebuf.tb_change_cnt = typebuf.tb_change_cnt.wrapping_add(1);
            if typebuf.tb_change_cnt == 0 {
                typebuf.tb_change_cnt = 1;
            }
        }

        fix_input_buffer(buf, len)
    }
}

/// Fix typed characters for use by `vgetc()`.
pub unsafe fn fix_input_buffer(buf: *mut u8, mut len: c_int) -> c_int {
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
pub unsafe fn get_keystroke(events: *mut MultiQueue) -> c_int {
    unsafe {
        let mut buf: Vec<u8> = vec![0; 150];
        let mut buflen: c_int = 150;
        let mut len: c_int = 0;
        let mut n: c_int;
        let save_mapped_ctrl_c = mapped_ctrl_c;

        mod_mask = 0;
        mapped_ctrl_c = 0;
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
                        mod_mask = buf[2] as c_int;
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

        mapped_ctrl_c = save_mapped_ctrl_c;
        merge_modifiers(n, &raw mut mod_mask)
    }
}

/// Function passed to `do_cmdline()` to get the command after a `<Cmd>` key from typeahead.
pub unsafe fn getcmdkeycmd(promptc: c_int, _indent: c_int, _do_concat: bool) -> *mut c_char {
    unsafe {
        let mut line: Vec<u8> = Vec::new();
        let mut c1: c_int = -1;
        let mut cmod: c_int = 0;
        let mut aborted = false;

        no_mapping += 1;

        got_int = false;
        while c1 != NUL && !aborted {
            if vgetorpeek(false) == NUL {
                emsg(gettext(E_CMD_MAPPING_MUST_END_WITH_CR.as_ptr()));
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

            if got_int {
                aborted = true;
            } else if c1 == b'\r' as c_int || c1 == b'\n' as c_int {
                c1 = NUL;
            } else if c1 == ESC {
                aborted = true;
            } else if c1 == K_COMMAND {
                emsg(gettext(
                    E_CMD_MAPPING_MUST_END_WITH_CR_BEFORE_SECOND_CMD.as_ptr(),
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

        no_mapping -= 1;

        if aborted {
            ptr::null_mut()
        } else {
            let out = xmemdupz(line.as_ptr().cast(), line.len()).cast::<c_char>();
            if promptc == b':' as c_int {
                atom_cmdline_set(K_COMMAND, out, line.len());
            }
            out
        }
    }
}

/// Handle a Lua mapping: get its id from typeahead and execute its callback.
pub unsafe fn map_execute_lua(may_repeat: bool, discard: bool) -> bool {
    unsafe {
        let mut line: Vec<c_char> = Vec::new();
        let mut c1: c_int = -1;
        let mut aborted = false;

        no_mapping += 1;

        got_int = false;
        while c1 != NUL && !aborted {
            c1 = vgetorpeek(true);
            if got_int {
                aborted = true;
            } else if c1 == b'\r' as c_int || c1 == b'\n' as c_int {
                c1 = NUL;
            } else {
                line.push(c1 as c_char);
            }
        }

        no_mapping -= 1;

        if aborted || discard {
            return !aborted;
        }

        line.push(NUL as c_char);
        let id = atoi(line.as_ptr());
        drop(line);
        let ref_ = map_luaid_get(id);
        if ref_ == LUA_NOREF {
            emsg(gettext(E_LUA_MAPPING_GONE.as_ptr()));
            return false;
        }
        if may_repeat {
            repeat_luamap = id;
        }

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
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
            ptr::null_mut(),
            &raw mut err,
        );
        if err.err_type != ErrorType::kErrorTypeNone {
            semsg_multiline!(c"emsg".as_ptr(), c"E5108: %s".as_ptr(), err.msg);
            api_clear_error(&raw mut err);
        }

        true
    }
}

/// Wraps pasted text stream with `K_PASTE_START` and `K_PASTE_END`.
pub unsafe fn paste_store(channel_id: u64, state: TriState, str_: NvimString, crlf: bool) {
    unsafe {
        if (State & MODE_CMDLINE) != 0 {
            return;
        }

        let need_redo = !block_redo;
        let need_record = reg_recording != 0 && !is_internal_call(channel_id);

        if !need_redo && !need_record {
            return;
        }

        let rbody = &raw mut redobuff.cur.body;
        let recbuf = &raw mut recordbuff;

        if state != kNone {
            let c = if state == kFalse {
                K_PASTE_START
            } else {
                K_PASTE_END
            };
            if need_redo {
                if state == kFalse && (State & MODE_INSERT) == 0 {
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
pub unsafe fn paste_repeat(count: c_int) {
    unsafe {
        let mut buf: Vec<u8> = Vec::new();
        let mut aborted = false;

        no_mapping += 1;

        got_int = false;
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
            aborted = got_int;
        }

        no_mapping -= 1;

        let str_ = NvimString {
            data: buf.as_mut_ptr().cast::<c_char>(),
            size: buf.len(),
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut i = 0;
        while !aborted && i < count {
            nvim_paste(
                LUA_INTERNAL_CALL,
                str_,
                false,
                -1,
                &raw mut arena,
                &raw mut err,
            );
            aborted = err.err_type != ErrorType::kErrorTypeNone;
            i += 1;
        }
        api_clear_error(&raw mut err);
        arena_mem_free(arena_finish(&raw mut arena));
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
        let merged = unsafe { merge_modifiers(b'c' as c_int, &raw mut mods) };
        assert_eq!(merged, Ctrl_C);
        assert_eq!(mods, 0);
    }
}
