//! Functions for managing registers (`src/nvim/register.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_assignments,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::command::exarg_T;
use crate::api::private::helpers::{cbuf_as_string, cbuf_to_string};
use crate::autocmd::{
    EVENT_RECORDINGENTER, EVENT_RECORDINGLEAVE, EVENT_TEXTPUTPOST, EVENT_TEXTPUTPRE,
    EVENT_TEXTYANKPOST, apply_autocmds, has_event,
};
use crate::buffer::{buf_is_empty, buflist_findnr, buflist_findpat, buflist_name_nr, getaltfname};
use crate::buffer_updates::buf_updates_send_changes;
use crate::change::{changed_bytes, changed_lines, del_chars};
use crate::charset::{ascii_iswhite, getdigits_int, skipwhite, transchar, vim_strsize};
use crate::clipboard::{
    MotionType, adjust_clipboard_name, get_clipboard, set_clipboard, yankreg_T,
};
use crate::cursor::{
    coladvance_force, colnr_T, gchar_cursor, get_cursor_line_len, get_cursor_line_ptr,
    get_cursor_pos_len, get_cursor_pos_ptr, getviscol, getvpos, linenr_T,
};
use crate::drawscreen::{showmode, update_screen};
use crate::errors::*;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::pos_T;
use crate::eval::typval::{
    BoolVarValue, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_str, tv_dict_add_str_len,
    tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_allocated_string,
    tv_list_append_string,
};
use crate::eval::{eval_to_string, get_v_event, restore_v_event};
use crate::ex_cmds2::check_fname;
use crate::ex_getln::{cmdline_paste_str, getcmdline};
use crate::extmark::{ExtmarkOp, bcount_t, extmark_splice, extmark_splice_cols};
use crate::file_search::{FNAME_EXP, FNAME_HYP, FNAME_MESS, file_name_at_cursor};
use crate::fold::hasFolding;
use crate::garray::{ga_append, ga_clear, ga_concat_len, ga_init, ga_set_growsize, garray_T};
use crate::globals::*;
use crate::grid::UIExtension;
use crate::indent::{get_indent, preprocs_left, set_indent, tabstop_padding};
use crate::input::{
    beep_flush, get_recorded, ins_typebuf, redo_append_char, stuffReadbuff, stuffReadbuffLen,
    stuffcharReadbuff, stuffescaped,
};
use crate::insert::{beginline, get_last_insert, get_last_insert_save, oneright, stuff_inserted};
use crate::insexpand::{ins_compl_delete, ins_compl_preinsert_effect};
use crate::keycodes::{
    K_SPECIAL, KE_COMMAND, KS_EXTRA, termcap2key, vim_strsave_escape_ks, vim_unescape_ks,
};
use crate::map_glyph_cache::NvimString;
use crate::mark::mark_adjust;
use crate::mbyte::{
    mb_charlen, mb_string2cells_len, mb_tolower, ptr2cells, utf_head_off, utfc_ptr2len,
};
use crate::mcursor::mc_ins_cascading;
use crate::memline::{decl, ml_append, ml_get, ml_get_buf, ml_get_len, ml_replace};
use crate::memory::{
    memchrsub, memcnt, xcalloc, xfree, xmalloc, xmallocz, xmemdupz, xrealloc, xstrdup,
};
use crate::message::{
    emsg, emsg_invreg, message_filtered, msg, msg_ext_set_kind, msg_outtrans_len, msg_putchar,
    msg_puts, msg_puts_hl, msg_puts_title, msgmore, semsg, smsg,
};
use crate::r#move::{changed_cline_bef_curs, invalidate_botline_win, update_topline};
use crate::normal::{exec_stuffed, find_ident_under_cursor};
use crate::ops::{adjust_cursor_eol, block_prep, charwise_block_prep, get_op_char};
use crate::option::get_ve_flags;
use crate::option_vars::*;
use crate::os::input::os_breakcheck;
use crate::os::time::{Timestamp, os_realtime};
use crate::plines::{
    CSType, CharsizeArg, getvcol, init_charsize_arg, utf_ClusterInfo, utf_ptr2StrCharInfo,
    win_charsize,
};
use crate::search::{Direction, last_search_pat, set_last_search_pat};
use crate::state::{REPLACE_FLAG, save_v_event_T};
use crate::strings::{
    atoi, gettext, ngettext, strncmp, vim_snprintf, vim_snprintf_safelen, vim_strchr,
    vim_strsave_escaped_ext,
};
use crate::terminal::terminal_paste;
use crate::textformat::oparg_T;
use crate::ui::ui_has;
use crate::undo::{u_save, u_save_cursor};
use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr;

const NUL: c_char = 0;
const TAB: c_int = b'\t' as c_int;
const NL: c_char = b'\n' as c_char;
const CAR: c_char = b'\r' as c_char;
const ESC: c_char = 0x1b;
const Ctrl_A: c_int = 1;
const Ctrl_F: c_int = 6;
const Ctrl_L: c_int = 12;
const Ctrl_P: c_int = 16;
const Ctrl_R: c_int = 18;
const Ctrl_U: u8 = 21;
const Ctrl_V: c_int = 22;
const Ctrl_W: c_int = 23;

const FAIL: c_int = 0;
const OK: c_int = 1;
const FORWARD: Direction = 1;
const BACKWARD: Direction = -1;
const MAXLNUM: linenr_T = 0x7fffffff;
const MAXCOL: colnr_T = 0x7fffffff;
const NUMBUFLEN: usize = 65;

pub const PUT_FIXINDENT: c_int = 1;
pub const PUT_CURSEND: c_int = 2;
pub const PUT_CURSLINE: c_int = 4;
pub const PUT_LINE: c_int = 8;
pub const PUT_LINE_SPLIT: c_int = 16;
pub const PUT_LINE_FORWARD: c_int = 32;
pub const PUT_BLOCK_INNER: c_int = 64;

pub const DELETION_REGISTER: c_int = 36;
pub const NUM_SAVED_REGISTERS: usize = 37;
pub const STAR_REGISTER: c_int = 37;
pub const PLUS_REGISTER: c_int = 38;
pub const NUM_REGISTERS: usize = 39;

pub const kGRegNoExpr: c_int = 1;
pub const kGRegExprSrc: c_int = 2;
pub const kGRegList: c_int = 4;

pub const YREG_PASTE: c_int = 0;
pub const YREG_YANK: c_int = 1;
pub const YREG_PUT: c_int = 2;

const REMAP_YES: c_int = 0;
const REMAP_NONE: c_int = -1;

const K_COMMAND: c_int = termcap2key(KS_EXTRA as c_int, KE_COMMAND as c_int);

const FIND_IDENT: c_int = 1;
const FIND_STRING: c_int = 2;

const VAR_FIXED: c_int = 2;
const kBoolVarFalse: BoolVarValue = 0;
const kBoolVarTrue: BoolVarValue = 1;

const CMOD_LOCKMARKS: c_int = 0x0800;
const SIN_NOMARK: c_int = 8;
const BL_WHITE: c_int = 1;
const BL_FIX: c_int = 4;
const RE_SEARCH: c_int = 0;

const HLF_8: c_int = 1;

const kUIMessages: UIExtension = 4;
const kCpoRegappend: c_int = b'>' as c_int;
const kOptVeFlagAll: c_uint = 0x04;
const kOptVeFlagOnemore: c_uint = 0x08;
const kExtmarkNOOP: ExtmarkOp = 0;
const kExtmarkUndo: ExtmarkOp = 1;

const E_REGISTER_CHAR_CANNOT_CONTAIN_MULTIPLE_LINES: &CStr =
    c"E883: Register '%c' cannot contain multiple lines";

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct block_def {
    pub startspaces: c_int,
    pub endspaces: c_int,
    pub textlen: c_int,
    pub textstart: *mut c_char,
    pub textcol: colnr_T,
    pub start_vcol: colnr_T,
    pub end_vcol: colnr_T,
    pub is_short: c_int,
    pub is_MAX: c_int,
    pub is_oneChar: c_int,
    pub pre_whitesp: c_int,
    pub pre_whitesp_c: c_int,
    pub end_char_vcols: colnr_T,
    pub start_char_vcols: colnr_T,
}

const EMPTY_YANKREG: yankreg_T = yankreg_T {
    y_array: ptr::null_mut(),
    y_size: 0,
    y_type: MotionType::kMTCharWise,
    y_width: 0,
    timestamp: 0,
    additional_data: ptr::null_mut(),
};

static mut expr_line: *mut c_char = ptr::null_mut();
static mut get_expr_line_nested: c_int = 0;
static mut execreg_lastc: c_int = 0;
static mut y_regs: [yankreg_T; NUM_REGISTERS] = [EMPTY_YANKREG; NUM_REGISTERS];
static mut y_previous: *mut yankreg_T = ptr::null_mut();
static mut empty_reg: yankreg_T = EMPTY_YANKREG;
static mut do_record_regname: c_int = 0;
static mut textyankpost_recursive: bool = false;
static mut put_autocmd_recursive: bool = false;

#[inline]
pub const fn is_literal_register(regname: c_int) -> bool {
    regname == b'*' as c_int
        || regname == b'+' as c_int
        || (regname >= 0 && regname <= 0x7f && (regname as u8).is_ascii_alphanumeric())
}

#[inline]
pub const fn op_reg_index(regname: c_int) -> c_int {
    if regname >= b'0' as c_int && regname <= b'9' as c_int {
        regname - b'0' as c_int
    } else if regname >= b'a' as c_int && regname <= b'z' as c_int {
        regname - b'a' as c_int + 10
    } else if regname >= b'A' as c_int && regname <= b'Z' as c_int {
        regname - b'A' as c_int + 10
    } else if regname == b'-' as c_int {
        DELETION_REGISTER
    } else if regname == b'*' as c_int {
        STAR_REGISTER
    } else if regname == b'+' as c_int {
        PLUS_REGISTER
    } else {
        -1
    }
}

#[inline]
pub const fn is_append_register(regname: c_int) -> bool {
    regname >= b'A' as c_int && regname <= b'Z' as c_int
}

#[inline]
pub const fn get_register_name(num: c_int) -> c_int {
    if num == -1 {
        b'"' as c_int
    } else if num < 10 {
        num + b'0' as c_int
    } else if num == DELETION_REGISTER {
        b'-' as c_int
    } else if num == STAR_REGISTER {
        b'*' as c_int
    } else if num == PLUS_REGISTER {
        b'+' as c_int
    } else {
        num + b'a' as c_int - 10
    }
}

#[inline]
pub unsafe fn reg_empty(reg: *const yankreg_T) -> bool {
    unsafe {
        (*reg).y_array.is_null()
            || (*reg).y_size == 0
            || ((*reg).y_size == 1
                && (*reg).y_type == MotionType::kMTCharWise
                && (*(*reg).y_array).size == 0)
    }
}

#[inline]
unsafe fn cstr_to_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        let len = unsafe { CStr::from_ptr(s).to_bytes().len() };
        unsafe { cbuf_to_string(s, len) }
    }
}

#[inline]
unsafe fn copy_string_global(s: NvimString) -> NvimString {
    if s.data.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        unsafe { cbuf_to_string(s.data, s.size) }
    }
}

#[inline]
unsafe fn api_clear_string(s: &mut NvimString) {
    unsafe {
        xfree(s.data.cast());
        s.data = ptr::null_mut();
        s.size = 0;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_unname_register() -> c_int {
    unsafe {
        if y_previous.is_null() {
            -1
        } else {
            y_previous.offset_from((&raw const y_regs).cast::<yankreg_T>()) as c_int
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_y_register(reg: c_int) -> *mut yankreg_T {
    unsafe { (&raw mut y_regs).cast::<yankreg_T>().add(reg as usize) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_y_previous() -> *mut yankreg_T {
    unsafe { y_previous }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_expr_register() -> c_int {
    unsafe {
        let new_line = getcmdline(c_int::from(b'='), 0, 0, true);
        if new_line.is_null() {
            return 0;
        }
        if *new_line == NUL {
            xfree(new_line.cast());
        } else {
            set_expr_line(new_line);
        }
        c_int::from(b'=')
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_expr_line(new_line: *mut c_char) {
    unsafe {
        xfree(expr_line.cast());
        expr_line = new_line;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_expr_line() -> *mut c_char {
    unsafe {
        if expr_line.is_null() {
            return ptr::null_mut();
        }

        let expr_copy = xstrdup(expr_line);
        if get_expr_line_nested >= 10 {
            return expr_copy;
        }

        get_expr_line_nested += 1;
        let rv = eval_to_string(expr_copy, true, false);
        get_expr_line_nested -= 1;
        xfree(expr_copy.cast());
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_expr_line_src() -> *mut c_char {
    unsafe {
        if expr_line.is_null() {
            return ptr::null_mut();
        }
        xstrdup(expr_line)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn valid_yank_reg(regname: c_int, writing: bool) -> bool {
    unsafe {
        (regname > 0 && regname <= 0x7f && (regname as u8).is_ascii_alphanumeric())
            || (!writing && !vim_strchr(c"/#.%:=".as_ptr(), regname).is_null())
            || regname == c_int::from(b'"')
            || regname == c_int::from(b'-')
            || regname == c_int::from(b'_')
            || regname == c_int::from(b'*')
            || regname == c_int::from(b'+')
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_default_register_name() -> c_int {
    unsafe {
        let mut name: c_int = 0;
        adjust_clipboard_name(&raw mut name, true, false);
        name
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_reg_iter(
    iter: *const c_void,
    regs: *const yankreg_T,
    name: *mut c_char,
    reg: *mut yankreg_T,
    is_unnamed: *mut bool,
) -> *const c_void {
    unsafe {
        *name = NUL;
        let mut iter_reg = if iter.is_null() {
            regs
        } else {
            iter.cast::<yankreg_T>()
        };
        while (iter_reg.offset_from(regs) as usize) < NUM_SAVED_REGISTERS && reg_empty(iter_reg) {
            iter_reg = iter_reg.add(1);
        }
        if (iter_reg.offset_from(regs) as usize) == NUM_SAVED_REGISTERS || reg_empty(iter_reg) {
            return ptr::null();
        }
        let iter_off = iter_reg.offset_from(regs) as c_int;
        *name = get_register_name(iter_off) as u8 as c_char;
        *reg = *iter_reg;
        *is_unnamed = ptr::eq(iter_reg, y_previous);
        loop {
            iter_reg = iter_reg.add(1);
            if (iter_reg.offset_from(regs) as usize) >= NUM_SAVED_REGISTERS {
                break;
            }
            if !reg_empty(iter_reg) {
                return iter_reg.cast::<c_void>();
            }
        }
        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_global_reg_iter(
    iter: *const c_void,
    name: *mut c_char,
    reg: *mut yankreg_T,
    is_unnamed: *mut bool,
) -> *const c_void {
    unsafe {
        op_reg_iter(
            iter,
            (&raw const y_regs).cast::<yankreg_T>(),
            name,
            reg,
            is_unnamed,
        )
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn reg_max_ts(clipboard: bool) -> Timestamp {
    unsafe {
        let mut max_ts: Timestamp = 0;
        let mut iter: *const c_void = ptr::null();
        loop {
            let mut name: c_char = NUL;
            let mut reg = EMPTY_YANKREG;
            let mut is_unnamed = false;
            iter = op_global_reg_iter(iter, &raw mut name, &raw mut reg, &raw mut is_unnamed);
            if name == NUL {
                break;
            }
            max_ts = max_ts.max(reg.timestamp);
            if iter.is_null() {
                break;
            }
        }
        if clipboard {
            max_ts = max_ts.max((*get_y_register(op_reg_index(c_int::from(b'*')))).timestamp);
            max_ts = max_ts.max((*get_y_register(op_reg_index(c_int::from(b'+')))).timestamp);
        }
        max_ts
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_reg_amount() -> usize {
    unsafe {
        let mut ret: usize = 0;
        let regs = (&raw const y_regs).cast::<yankreg_T>();
        for i in 0..NUM_SAVED_REGISTERS {
            if !reg_empty(regs.add(i)) {
                ret += 1;
            }
        }
        ret
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_reg_set(name: c_char, reg: yankreg_T, is_unnamed: bool) -> bool {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return false;
        }
        let target = (&raw mut y_regs).cast::<yankreg_T>().add(i as usize);
        free_register(target);
        *target = reg;

        if is_unnamed {
            y_previous = target;
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_reg_get(name: c_char) -> *const yankreg_T {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return ptr::null();
        }
        (&raw const y_regs).cast::<yankreg_T>().add(i as usize)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_reg_set_previous(name: c_char) -> bool {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return false;
        }
        y_previous = (&raw mut y_regs).cast::<yankreg_T>().add(i as usize);
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_yankreg_width(reg: *mut yankreg_T) {
    unsafe {
        if (*reg).y_type == MotionType::kMTBlockWise {
            let mut maxlen: usize = 0;
            for i in 0..(*reg).y_size {
                let item = *(*reg).y_array.add(i);
                let rowlen = mb_string2cells_len(item.data, item.size);
                maxlen = maxlen.max(rowlen);
            }
            assert!(maxlen <= c_int::MAX as usize);
            (*reg).y_width = (*reg).y_width.max((maxlen as colnr_T) - 1);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_yank_register(regname: c_int, mode: c_int) -> *mut yankreg_T {
    unsafe {
        let mut reg: *mut yankreg_T = ptr::null_mut();

        if (mode == YREG_PASTE || mode == YREG_PUT) && get_clipboard(regname, &raw mut reg, false) {
            return reg;
        } else if mode == YREG_PUT && (regname == c_int::from(b'*') || regname == c_int::from(b'+'))
        {
            return &raw mut empty_reg;
        } else if mode != YREG_YANK
            && (regname == 0
                || regname == c_int::from(b'"')
                || regname == c_int::from(b'*')
                || regname == c_int::from(b'+'))
            && !y_previous.is_null()
        {
            return y_previous;
        }

        let mut i = op_reg_index(regname);
        if i == -1 {
            i = 0;
        }
        reg = (&raw mut y_regs).cast::<yankreg_T>().add(i as usize);

        if mode == YREG_YANK {
            y_previous = reg;
        }
        reg
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn yank_register_mline(regname: c_int, reg: *mut *mut yankreg_T) -> bool {
    unsafe {
        *reg = ptr::null_mut();
        if regname != 0 && !valid_yank_reg(regname, false) {
            return false;
        }
        if regname == c_int::from(b'_') {
            return false;
        }
        *reg = get_yank_register(regname, YREG_PASTE);
        (*(*reg)).y_type == MotionType::kMTLineWise
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copy_register(name: c_int) -> *mut yankreg_T {
    unsafe {
        let reg = get_yank_register(name, YREG_PASTE);
        let copy = xmalloc(size_of::<yankreg_T>()).cast::<yankreg_T>();
        *copy = *reg;
        if (*copy).y_size == 0 {
            (*copy).y_array = ptr::null_mut();
        } else {
            (*copy).y_array = xcalloc((*copy).y_size, size_of::<NvimString>()).cast::<NvimString>();
            for i in 0..(*copy).y_size {
                *(*copy).y_array.add(i) = copy_string_global(*(*reg).y_array.add(i));
            }
        }
        copy
    }
}

unsafe fn stuff_yank(regname: c_int, p: *mut c_char) -> c_int {
    unsafe {
        if regname != 0 && !valid_yank_reg(regname, true) {
            xfree(p.cast());
            return FAIL;
        }
        if regname == c_int::from(b'_') {
            xfree(p.cast());
            return OK;
        }

        let plen = CStr::from_ptr(p).to_bytes().len();
        let reg = get_yank_register(regname, YREG_YANK);
        if is_append_register(regname) && !(*reg).y_array.is_null() {
            let pp = (*reg).y_array.add((*reg).y_size - 1);
            let tmplen = (*pp).size + plen;
            let tmp = xmalloc(tmplen + 1).cast::<c_char>();
            ptr::copy_nonoverlapping((*pp).data, tmp, (*pp).size);
            ptr::copy_nonoverlapping(p, tmp.add((*pp).size), plen);
            *tmp.add(tmplen) = NUL;
            xfree(p.cast());
            xfree((*pp).data.cast());
            *pp = cbuf_as_string(tmp, tmplen);
        } else {
            free_register(reg);
            (*reg).additional_data = ptr::null_mut();
            (*reg).y_array = xmalloc(size_of::<NvimString>()).cast::<NvimString>();
            *(*reg).y_array = cbuf_as_string(p, plen);
            (*reg).y_size = 1;
            (*reg).y_type = MotionType::kMTCharWise;
        }
        (*reg).timestamp = os_realtime() as u64;
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_record(c: c_int) -> c_int {
    unsafe {
        let retval: c_int;

        if reg_recording == 0 {
            if c < 0
                || (!(c <= 0x7f && (c as u8).is_ascii_alphanumeric()) && c != c_int::from(b'"'))
            {
                retval = FAIL;
            } else {
                reg_recording = c;
                showmode();
                do_record_regname = c;
                retval = OK;
                apply_autocmds(
                    EVENT_RECORDINGENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    curbuf,
                );
            }
        } else {
            let mut save_v_event: save_v_event_T = std::mem::zeroed();
            let dict = get_v_event(&raw mut save_v_event);

            let p = get_recorded();
            if !p.is_null() {
                vim_unescape_ks(p);
                tv_dict_add_str(dict, c"regcontents".as_ptr(), 11, p);
            }

            let mut buf = [0 as c_char; NUMBUFLEN + 2];
            buf[0] = do_record_regname as u8 as c_char;
            buf[1] = NUL;
            tv_dict_add_str(dict, c"regname".as_ptr(), 7, buf.as_ptr());
            tv_dict_set_keys_readonly(dict);

            apply_autocmds(
                EVENT_RECORDINGLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            restore_v_event(dict, &raw mut save_v_event);
            reg_recorded = reg_recording;
            reg_recording = 0;
            if p_ch == 0 || ui_has(kUIMessages) {
                showmode();
            } else {
                msg(c"".as_ptr(), 0);
            }
            if p.is_null() {
                retval = FAIL;
            } else {
                let old_y_previous = y_previous;
                retval = stuff_yank(do_record_regname, p);
                y_previous = old_y_previous;
            }
        }
        retval
    }
}

unsafe fn put_in_typebuf(s: *mut c_char, esc: bool, colon: bool, silent: c_int) -> c_int {
    unsafe {
        let mut retval = OK;
        let is_silent = silent != 0;

        put_reedit_in_typebuf(silent);
        if colon {
            retval = ins_typebuf(c"\n".as_ptr().cast_mut(), REMAP_NONE, 0, true, is_silent);
        }
        if retval == OK {
            let p = if esc { vim_strsave_escape_ks(s) } else { s };
            if p.is_null() {
                retval = FAIL;
            } else {
                retval = ins_typebuf(
                    p,
                    if esc { REMAP_NONE } else { REMAP_YES },
                    0,
                    true,
                    is_silent,
                );
            }
            if esc {
                xfree(p.cast());
            }
        }
        if colon && retval == OK {
            retval = ins_typebuf(c":".as_ptr().cast_mut(), REMAP_NONE, 0, true, is_silent);
        }
        retval
    }
}

unsafe fn put_reedit_in_typebuf(silent: c_int) {
    unsafe {
        if restart_edit == 0 {
            return;
        }

        let mut buf: [u8; 11] = [
            K_SPECIAL, KS_EXTRA, KE_COMMAND, b's', b't', b'a', b'r', b't', b'i', CAR as u8, 0,
        ];
        if restart_edit == c_int::from(b'R') {
            buf[8] = b'r';
        } else if restart_edit == c_int::from(b'V') {
            buf[8] = b'g';
        } else if restart_edit == c_int::from(b'A') {
            buf[8] = b'!';
        }
        if ins_typebuf(
            buf.as_mut_ptr().cast::<c_char>(),
            REMAP_NONE,
            0,
            true,
            silent != 0,
        ) == OK
        {
            restart_edit = 0;
        }
    }
}

unsafe fn execreg_line_continuation(lines: *mut NvimString, idx: *mut usize) -> *mut c_char {
    unsafe {
        let mut cmd_start = *idx;
        assert!(cmd_start > 0);
        let cmd_end = cmd_start;

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, size_of::<c_char>() as c_int, 400);

        while cmd_start > 0 {
            cmd_start -= 1;
            let p = skipwhite((*lines.add(cmd_start)).data);
            if *p != b'\\' as c_char
                && (*p != b'"' as c_char
                    || *p.add(1) != b'\\' as c_char
                    || *p.add(2) != b' ' as c_char)
            {
                break;
            }
        }

        let mut tmp = lines.add(cmd_start);
        ga_concat_len(&raw mut ga, (*tmp).data, (*tmp).size);
        for j in (cmd_start + 1)..=cmd_end {
            tmp = lines.add(j);
            let mut p = skipwhite((*tmp).data);
            if *p == b'\\' as c_char {
                if ga.ga_len > 400 {
                    ga_set_growsize(&raw mut ga, ga.ga_len.min(8000));
                }
                p = p.add(1);
                let rem = ((*tmp).data as usize + (*tmp).size) - (p as usize);
                ga_concat_len(&raw mut ga, p, rem);
            }
        }
        ga_append(&raw mut ga, NUL as u8);
        let str_ = xmemdupz(ga.ga_data, ga.ga_len as usize).cast::<c_char>();
        ga_clear(&raw mut ga);

        *idx = cmd_start;
        str_
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_execreg(
    mut regname: c_int,
    colon: c_int,
    addcr: c_int,
    silent: c_int,
) -> c_int {
    unsafe {
        let mut retval = OK;

        if regname == c_int::from(b'@') {
            if execreg_lastc == 0 {
                emsg(gettext(c"E748: No previously used register".as_ptr()));
                return FAIL;
            }
            regname = execreg_lastc;
        }
        if regname == c_int::from(b'%')
            || regname == c_int::from(b'#')
            || !valid_yank_reg(regname, false)
        {
            emsg_invreg(regname);
            return FAIL;
        }
        execreg_lastc = regname;

        if regname == c_int::from(b'_') {
            return OK;
        }

        if regname == c_int::from(b':') {
            if last_cmdline.is_null() {
                emsg(gettext(e_nolastcmd.as_ptr()));
                return FAIL;
            }
            xfree(new_last_cmdline.cast());
            new_last_cmdline = ptr::null_mut();
            let p = vim_strsave_escaped_ext(
                last_cmdline,
                c"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f".as_ptr(),
                Ctrl_V as u8 as c_char,
                false,
            );
            if Visual.active && strncmp(p, c"'<,'>".as_ptr(), 5) == 0 {
                retval = put_in_typebuf(p.add(5), true, true, silent);
            } else {
                retval = put_in_typebuf(p, true, true, silent);
            }
            xfree(p.cast());
        } else if regname == c_int::from(b'=') {
            let p = get_expr_line();
            if p.is_null() {
                return FAIL;
            }
            retval = put_in_typebuf(p, true, colon != 0, silent);
            xfree(p.cast());
        } else if regname == c_int::from(b'.') {
            let p = get_last_insert_save();
            if p.is_null() {
                emsg(gettext(e_noinstext.as_ptr()));
                return FAIL;
            }
            retval = put_in_typebuf(p, false, colon != 0, silent);
            xfree(p.cast());
        } else {
            let reg = get_yank_register(regname, YREG_PASTE);
            if (*reg).y_array.is_null() {
                return FAIL;
            }

            let remap = if colon != 0 { REMAP_NONE } else { REMAP_YES };
            let is_silent = silent != 0;

            put_reedit_in_typebuf(silent);
            let mut i = (*reg).y_size;
            while i > 0 {
                i -= 1;
                if ((*reg).y_type == MotionType::kMTLineWise || i < (*reg).y_size - 1 || addcr != 0)
                    && ins_typebuf(c"\n".as_ptr().cast_mut(), remap, 0, true, is_silent) == FAIL
                {
                    return FAIL;
                }

                let mut str_ = (*(*reg).y_array.add(i)).data;
                let mut free_str = false;
                if colon != 0 && i > 0 {
                    let p = skipwhite(str_);
                    if *p == b'\\' as c_char
                        || (*p == b'"' as c_char
                            && *p.add(1) == b'\\' as c_char
                            && *p.add(2) == b' ' as c_char)
                    {
                        str_ = execreg_line_continuation((*reg).y_array, &raw mut i);
                        free_str = true;
                    }
                }
                let escaped = vim_strsave_escape_ks(str_);
                if free_str {
                    xfree(str_.cast());
                }
                retval = ins_typebuf(escaped, remap, 0, true, is_silent);
                xfree(escaped.cast());
                if retval == FAIL {
                    return FAIL;
                }
                if colon != 0
                    && ins_typebuf(c":".as_ptr().cast_mut(), remap, 0, true, is_silent) == FAIL
                {
                    return FAIL;
                }
            }
            reg_executing = if regname == 0 {
                c_int::from(b'"')
            } else {
                regname
            };
            pending_end_reg_executing = false;
        }
        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn insert_reg(
    regname: c_int,
    mut reg: *mut yankreg_T,
    literally_arg: bool,
) -> c_int {
    unsafe {
        let mut retval = OK;
        let mut allocated = false;
        let literally = literally_arg || is_literal_register(regname);

        os_breakcheck();
        if got_int {
            return FAIL;
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return FAIL;
        }

        let mut arg: *mut c_char = ptr::null_mut();
        if regname == c_int::from(b'.') {
            retval = stuff_inserted(0, 1, 1);
        } else if get_spec_reg(regname, &raw mut arg, &raw mut allocated, true) {
            if arg.is_null() {
                return FAIL;
            }
            stuffescaped(arg, literally);
            if allocated {
                xfree(arg.cast());
            }
        } else {
            if reg.is_null() {
                reg = get_yank_register(regname, YREG_PASTE);
            }
            if (*reg).y_array.is_null() {
                retval = FAIL;
            } else {
                let put = (*reg).y_type == MotionType::kMTCharWise
                    && (regname == c_int::from(b'-')
                        || (mc_ins_cascading()
                            && (*reg).y_size == 1
                            && (((0..=0x7f).contains(&regname)
                                && (regname as u8).is_ascii_alphanumeric())
                                || regname == c_int::from(b'"'))));
                for i in 0..(*reg).y_size {
                    if put {
                        let mut dir = BACKWARD;
                        if (State & REPLACE_FLAG) != 0 {
                            if u_save_cursor() == FAIL {
                                return FAIL;
                            }
                            del_chars(mb_charlen((*(*reg).y_array).data), 1);
                            let curpos = (*curwin).w_cursor;
                            if oneright() == FAIL {
                                dir = FORWARD;
                            }
                            (*curwin).w_cursor = curpos;
                        }

                        redo_append_char(Ctrl_R);
                        redo_append_char(regname);
                        do_put(regname, ptr::null_mut(), dir, 1, PUT_CURSEND);
                    } else {
                        stuffescaped((*(*reg).y_array.add(i)).data, literally);
                        if (*reg).y_type == MotionType::kMTLineWise || i < (*reg).y_size - 1 {
                            stuffcharReadbuff(c_int::from(b'\n'));
                        }
                    }
                }
            }
        }

        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_spec_reg(
    regname: c_int,
    argp: *mut *mut c_char,
    allocated: *mut bool,
    errmsg: bool,
) -> bool {
    unsafe {
        *argp = ptr::null_mut();
        *allocated = false;
        match regname {
            x if x == c_int::from(b'%') => {
                if errmsg {
                    check_fname();
                }
                *argp = (*curbuf).b_fname;
                true
            }
            x if x == c_int::from(b'#') => {
                *argp = getaltfname(errmsg);
                true
            }
            x if x == c_int::from(b'=') => {
                *argp = get_expr_line();
                *allocated = true;
                true
            }
            x if x == c_int::from(b':') => {
                if last_cmdline.is_null() && errmsg {
                    emsg(gettext(e_nolastcmd.as_ptr()));
                }
                *argp = last_cmdline;
                true
            }
            x if x == c_int::from(b'/') => {
                if last_search_pat().is_null() && errmsg {
                    emsg(gettext(e_noprevre.as_ptr()));
                }
                *argp = last_search_pat();
                true
            }
            x if x == c_int::from(b'.') => {
                *argp = get_last_insert_save();
                *allocated = true;
                if (*argp).is_null() && errmsg {
                    emsg(gettext(e_noinstext.as_ptr()));
                }
                true
            }
            Ctrl_F | Ctrl_P => {
                if !errmsg {
                    return false;
                }
                *argp = file_name_at_cursor(
                    FNAME_MESS | FNAME_HYP | if regname == Ctrl_P { FNAME_EXP } else { 0 },
                    1,
                    ptr::null_mut(),
                );
                *allocated = true;
                true
            }
            Ctrl_W | Ctrl_A => {
                if !errmsg {
                    return false;
                }
                let cnt = find_ident_under_cursor(
                    argp,
                    if regname == Ctrl_W {
                        FIND_IDENT | FIND_STRING
                    } else {
                        FIND_STRING
                    },
                    ptr::null_mut(),
                );
                *argp = if cnt != 0 {
                    xmemdupz((*argp).cast(), cnt).cast::<c_char>()
                } else {
                    ptr::null_mut()
                };
                *allocated = true;
                true
            }
            Ctrl_L => {
                if !errmsg {
                    return false;
                }
                *argp = ml_get_buf((*curwin).w_buffer, (*curwin).w_cursor.lnum);
                true
            }
            x if x == c_int::from(b'_') => {
                *argp = c"".as_ptr().cast_mut();
                true
            }
            _ => false,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cmdline_paste_reg(
    regname: c_int,
    literally_arg: bool,
    remcr: bool,
) -> bool {
    unsafe {
        let literally = literally_arg || is_literal_register(regname);

        let reg = get_yank_register(regname, YREG_PASTE);
        if (*reg).y_array.is_null() {
            return false;
        }

        for i in 0..(*reg).y_size {
            cmdline_paste_str((*(*reg).y_array.add(i)).data, literally);

            if i < (*reg).y_size - 1 && !remcr {
                cmdline_paste_str(c"\r".as_ptr(), literally);
            }

            os_breakcheck();
            if got_int {
                return false;
            }
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shift_delete_registers(y_append: bool) {
    unsafe {
        let regs = (&raw mut y_regs).cast::<yankreg_T>();
        free_register(regs.add(9));
        let mut n = 9;
        while n > 1 {
            *regs.add(n) = *regs.add(n - 1);
            n -= 1;
        }
        if !y_append {
            y_previous = regs.add(1);
        }
        (*regs.add(1)).y_array = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clear_registers() {
    unsafe {
        let regs = (&raw mut y_regs).cast::<yankreg_T>();
        for i in 0..NUM_REGISTERS {
            free_register(regs.add(i));
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_register(reg: *mut yankreg_T) {
    unsafe {
        xfree((*reg).additional_data);
        (*reg).additional_data = ptr::null_mut();
        if !(*reg).y_array.is_null() {
            let mut i = (*reg).y_size;
            while i > 0 {
                i -= 1;
                api_clear_string(&mut *(*reg).y_array.add(i));
            }
            xfree((*reg).y_array.cast());
            (*reg).y_array = ptr::null_mut();
        }
        *reg = EMPTY_YANKREG;
    }
}

unsafe fn yank_copy_line(
    reg: *mut yankreg_T,
    bd: *mut block_def,
    y_idx: usize,
    exclude_trailing_space: bool,
) {
    unsafe {
        if exclude_trailing_space {
            (*bd).endspaces = 0;
        }
        let size = (*bd).startspaces + (*bd).endspaces + (*bd).textlen;
        assert!(size >= 0);
        let mut pnew = xmallocz(size as usize).cast::<c_char>();
        let dst = (*reg).y_array.add(y_idx);
        (*dst).data = pnew;
        ptr::write_bytes(pnew.cast::<u8>(), b' ', (*bd).startspaces as usize);
        pnew = pnew.add((*bd).startspaces as usize);
        ptr::copy((*bd).textstart, pnew, (*bd).textlen as usize);
        pnew = pnew.add((*bd).textlen as usize);
        ptr::write_bytes(pnew.cast::<u8>(), b' ', (*bd).endspaces as usize);
        pnew = pnew.add((*bd).endspaces as usize);
        if exclude_trailing_space {
            let mut s = (*bd).textlen + (*bd).endspaces;
            while s > 0 && ascii_iswhite(*(*bd).textstart.add((s - 1) as usize) as u8) {
                s = s - utf_head_off((*bd).textstart, (*bd).textstart.add((s - 1) as usize)) - 1;
                pnew = pnew.sub(1);
            }
        }
        *pnew = NUL;
        (*dst).size = (pnew as usize) - ((*dst).data as usize);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_yank_reg(
    oap: *mut oparg_T,
    message: bool,
    mut reg: *mut yankreg_T,
    append: bool,
) {
    unsafe {
        let mut newreg = EMPTY_YANKREG;
        let orig_motion = match (*oap).motion_type {
            0 => MotionType::kMTCharWise,
            1 => MotionType::kMTLineWise,
            2 => MotionType::kMTBlockWise,
            _ => MotionType::kMTUnknown,
        };
        let mut yank_type = orig_motion;
        let mut yanklines = (*oap).line_count as usize;
        let mut yankendlnum = (*oap).end.lnum;
        let mut bd = block_def::default();

        let curr = reg;
        if append && !(*reg).y_array.is_null() {
            reg = &raw mut newreg;
        } else {
            free_register(reg);
        }

        if orig_motion == MotionType::kMTCharWise
            && (*oap).start.col == 0
            && !(*oap).inclusive
            && (!(*oap).is_VIsual || *p_sel == b'o' as c_char)
            && (*oap).end.col == 0
            && yanklines > 1
        {
            yank_type = MotionType::kMTLineWise;
            yankendlnum -= 1;
            yanklines -= 1;
        }

        (*reg).y_size = yanklines;
        (*reg).y_type = yank_type;
        (*reg).y_width = 0;
        (*reg).y_array = xcalloc(yanklines, size_of::<NvimString>()).cast::<NvimString>();
        (*reg).additional_data = ptr::null_mut();
        (*reg).timestamp = os_realtime() as u64;

        let mut y_idx: usize = 0;
        let mut lnum = (*oap).start.lnum;

        if yank_type == MotionType::kMTBlockWise {
            (*reg).y_width = (*oap).end_vcol - (*oap).start_vcol;
            if (*curwin).w_curswant == MAXCOL && (*reg).y_width > 0 {
                (*reg).y_width -= 1;
            }
        }

        while lnum <= yankendlnum {
            match (*reg).y_type {
                MotionType::kMTBlockWise => {
                    block_prep(oap, &raw mut bd, lnum, false);
                    yank_copy_line(reg, &raw mut bd, y_idx, (*oap).excl_tr_ws);
                }
                MotionType::kMTLineWise => {
                    *(*reg).y_array.add(y_idx) =
                        cbuf_to_string(ml_get(lnum), ml_get_len(lnum) as usize);
                }
                MotionType::kMTCharWise => {
                    charwise_block_prep(
                        (*oap).start,
                        (*oap).end,
                        &raw mut bd,
                        lnum,
                        (*oap).inclusive,
                    );
                    let tmp = CStr::from_ptr(bd.textstart).to_bytes().len() as c_int;
                    if tmp < bd.textlen {
                        bd.textlen = tmp;
                    }
                    yank_copy_line(reg, &raw mut bd, y_idx, false);
                }
                MotionType::kMTUnknown => std::process::abort(),
            }
            lnum += 1;
            y_idx += 1;
        }

        if !ptr::eq(curr, reg) {
            let mut j: usize = 0;
            let new_ptr = xmalloc(size_of::<NvimString>() * ((*curr).y_size + (*reg).y_size))
                .cast::<NvimString>();
            while j < (*curr).y_size {
                *new_ptr.add(j) = *(*curr).y_array.add(j);
                j += 1;
            }
            xfree((*curr).y_array.cast());
            (*curr).y_array = new_ptr;

            if yank_type == MotionType::kMTLineWise {
                (*curr).y_type = MotionType::kMTLineWise;
            }

            if (*curr).y_type == MotionType::kMTCharWise
                && vim_strchr(p_cpo, kCpoRegappend).is_null()
            {
                let last_old = *(*curr).y_array.add((*curr).y_size - 1);
                let first_new = *(*reg).y_array;
                let pnew = xmalloc(last_old.size + first_new.size + 1).cast::<c_char>();
                j -= 1;
                ptr::copy_nonoverlapping(last_old.data, pnew, last_old.size);
                ptr::copy_nonoverlapping(
                    first_new.data,
                    pnew.add(last_old.size),
                    first_new.size + 1,
                );
                xfree(last_old.data.cast());
                *(*curr).y_array.add(j) = cbuf_as_string(pnew, last_old.size + first_new.size);
                j += 1;
                api_clear_string(&mut *(*reg).y_array);
                y_idx = 1;
            } else {
                y_idx = 0;
            }
            while y_idx < (*reg).y_size {
                *(*curr).y_array.add(j) = *(*reg).y_array.add(y_idx);
                j += 1;
                y_idx += 1;
            }
            (*curr).y_size = j;
            xfree((*reg).y_array.cast());
        }

        if message {
            if yank_type == MotionType::kMTCharWise && yanklines == 1 {
                yanklines = 0;
            }
            if yanklines > p_report as usize {
                let mut namebuf = [0 as c_char; 100];
                if (*oap).regname == 0 {
                    namebuf[0] = NUL;
                } else {
                    vim_snprintf!(
                        namebuf.as_mut_ptr(),
                        namebuf.len(),
                        gettext(c" into \"%c".as_ptr()),
                        (*oap).regname,
                    );
                }

                update_topline(curwin);
                if must_redraw != 0 {
                    update_screen();
                }
                #[cfg(windows)]
                const BLOCK_SINGLE: &CStr = c"block of %lld line yanked%s";
                #[cfg(windows)]
                const BLOCK_PLURAL: &CStr = c"block of %lld lines yanked%s";
                #[cfg(windows)]
                const LINE_SINGLE: &CStr = c"%lld line yanked%s";
                #[cfg(windows)]
                const LINE_PLURAL: &CStr = c"%lld lines yanked%s";

                #[cfg(not(windows))]
                const BLOCK_SINGLE: &CStr = c"block of %ld line yanked%s";
                #[cfg(not(windows))]
                const BLOCK_PLURAL: &CStr = c"block of %ld lines yanked%s";
                #[cfg(not(windows))]
                const LINE_SINGLE: &CStr = c"%ld line yanked%s";
                #[cfg(not(windows))]
                const LINE_PLURAL: &CStr = c"%ld lines yanked%s";

                if yank_type == MotionType::kMTBlockWise {
                    smsg!(
                        0,
                        ngettext(
                            BLOCK_SINGLE.as_ptr(),
                            BLOCK_PLURAL.as_ptr(),
                            yanklines as c_ulong,
                        ),
                        yanklines as i64,
                        namebuf.as_ptr(),
                    );
                } else {
                    smsg!(
                        0,
                        ngettext(
                            LINE_SINGLE.as_ptr(),
                            LINE_PLURAL.as_ptr(),
                            yanklines as c_ulong,
                        ),
                        yanklines as i64,
                        namebuf.as_ptr(),
                    );
                }
            }
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start = (*oap).start;
            (*curbuf).b_op_end = (*oap).end;
            if yank_type == MotionType::kMTLineWise {
                (*curbuf).b_op_start.col = 0;
                (*curbuf).b_op_end.col = MAXCOL;
            }
            if yank_type == MotionType::kMTCharWise && !(*oap).inclusive {
                decl(&raw mut (*curbuf).b_op_end);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn format_reg_type(
    reg_type: MotionType,
    reg_width: colnr_T,
    buf: *mut c_char,
    bufsize: usize,
) -> usize {
    unsafe {
        assert!(bufsize > 1);
        match reg_type {
            MotionType::kMTLineWise => {
                *buf = b'V' as c_char;
                *buf.add(1) = NUL;
                1
            }
            MotionType::kMTCharWise => {
                *buf = b'v' as c_char;
                *buf.add(1) = NUL;
                1
            }
            MotionType::kMTBlockWise => {
                vim_snprintf_safelen!(buf, bufsize, c"\x16%d".as_ptr(), reg_width + 1)
            }
            MotionType::kMTUnknown => {
                *buf = NUL;
                0
            }
        }
    }
}

unsafe fn add_regtype_to_dict(
    reg: *mut yankreg_T,
    dict: *mut dict_T,
    buf: *mut c_char,
    bufsize: usize,
) {
    unsafe {
        let len = format_reg_type(
            if !reg.is_null() {
                (*reg).y_type
            } else {
                MotionType::kMTCharWise
            },
            if !reg.is_null() { (*reg).y_width } else { 0 },
            buf,
            bufsize,
        );
        tv_dict_add_str_len(dict, c"regtype".as_ptr(), 7, buf, len as c_int);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_autocmd_textyankpost(oap: *mut oparg_T, reg: *mut yankreg_T) {
    unsafe {
        if textyankpost_recursive || !has_event(EVENT_TEXTYANKPOST) {
            return;
        }

        textyankpost_recursive = true;

        let mut save_v_event: save_v_event_T = std::mem::zeroed();
        let dict = get_v_event(&raw mut save_v_event);

        let list = tv_list_alloc((*reg).y_size as isize);
        for i in 0..(*reg).y_size {
            let item = *(*reg).y_array.add(i);
            tv_list_append_string(list, item.data, item.size as isize);
        }
        (*list).lv_lock = VAR_FIXED;
        tv_dict_add_list(dict, c"regcontents".as_ptr(), 11, list);

        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        add_regtype_to_dict(reg, dict, buf.as_mut_ptr(), buf.len());

        buf[0] = (*oap).regname as u8 as c_char;
        buf[1] = NUL;
        tv_dict_add_str(dict, c"regname".as_ptr(), 7, buf.as_ptr());

        tv_dict_add_bool(
            dict,
            c"inclusive".as_ptr(),
            9,
            if (*oap).inclusive {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );

        buf[0] = get_op_char((*oap).op_type) as u8 as c_char;
        buf[1] = NUL;
        tv_dict_add_str(dict, c"operator".as_ptr(), 8, buf.as_ptr());

        tv_dict_add_bool(
            dict,
            c"visual".as_ptr(),
            6,
            if (*oap).is_VIsual {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );

        tv_dict_set_keys_readonly(dict);
        textlock += 1;
        apply_autocmds(
            EVENT_TEXTYANKPOST,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        textlock -= 1;
        restore_v_event(dict, &raw mut save_v_event);

        textyankpost_recursive = false;
    }
}

unsafe fn put_do_autocmd(
    regname: c_int,
    reg: *mut yankreg_T,
    insert: *const NvimString,
    post: bool,
    dir: Direction,
) {
    unsafe {
        if put_autocmd_recursive || (regname == c_int::from(b'_') && reg.is_null()) {
            return;
        }

        if regname != c_int::from(b'.') && insert.is_null() && reg.is_null() {
            return;
        }

        let mut save_v_event: save_v_event_T = std::mem::zeroed();
        let v_event = get_v_event(&raw mut save_v_event);

        let list = tv_list_alloc(if !reg.is_null() {
            (*reg).y_size as isize
        } else {
            1
        });

        if regname == c_int::from(b'.') {
            if !last_insert_ga.ga_data.is_null() {
                tv_list_append_string(
                    list,
                    last_insert_ga.ga_data.cast::<c_char>(),
                    last_insert_ga.ga_len as isize,
                );
            }
        } else if !insert.is_null() {
            tv_list_append_string(list, (*insert).data, (*insert).size as isize);
        } else {
            for n in 0..(*reg).y_size {
                let item = *(*reg).y_array.add(n);
                tv_list_append_string(list, item.data, item.size as isize);
            }
        }

        (*list).lv_lock = VAR_FIXED;
        tv_dict_add_list(v_event, c"regcontents".as_ptr(), 11, list);

        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        buf[0] = regname as u8 as c_char;
        buf[1] = NUL;
        let buflen = if buf[0] == NUL { 0 } else { 1 };
        tv_dict_add_str_len(v_event, c"regname".as_ptr(), 7, buf.as_ptr(), buflen);

        buf[0] = if dir == BACKWARD { b'P' } else { b'p' } as c_char;
        buf[1] = NUL;
        tv_dict_add_str_len(v_event, c"operator".as_ptr(), 8, buf.as_ptr(), 1);

        add_regtype_to_dict(reg, v_event, buf.as_mut_ptr(), buf.len());

        tv_dict_add_bool(
            v_event,
            c"visual".as_ptr(),
            6,
            if Visual.active {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );

        tv_dict_set_keys_readonly(v_event);

        put_autocmd_recursive = true;
        textlock += 1;
        if post {
            apply_autocmds(
                EVENT_TEXTPUTPOST,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        } else {
            apply_autocmds(
                EVENT_TEXTPUTPRE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }
        textlock -= 1;
        put_autocmd_recursive = false;

        restore_v_event(v_event, &raw mut save_v_event);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_yank(oap: *mut oparg_T, message: bool) -> bool {
    unsafe {
        if (*oap).regname != 0 && !valid_yank_reg((*oap).regname, true) {
            beep_flush();
            return false;
        }
        if (*oap).regname == c_int::from(b'_') {
            return true;
        }

        let reg = get_yank_register((*oap).regname, YREG_YANK);
        op_yank_reg(oap, message, reg, is_append_register((*oap).regname));
        set_clipboard((*oap).regname, reg);
        do_autocmd_textyankpost(oap, reg);
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_put(
    regname: c_int,
    mut reg: *mut yankreg_T,
    mut dir: c_int,
    mut count: c_int,
    flags: c_int,
) {
    unsafe {
        let mut totlen: usize = 0;
        let mut lnum: linenr_T;
        let mut y_type: MotionType;
        let mut y_size: usize = 0;
        let mut y_width: c_int = 0;
        let mut vcol: colnr_T = 0;
        let mut y_array: *mut NvimString = ptr::null_mut();
        let mut nr_lines: linenr_T = 0;
        let mut allocated = false;
        let orig_start = (*curbuf).b_op_start;
        let orig_end = (*curbuf).b_op_end;
        let cur_ve_flags = get_ve_flags(curwin);

        if ins_compl_preinsert_effect() {
            ins_compl_delete(false);
        }

        (*curbuf).b_op_start = (*curwin).w_cursor;
        (*curbuf).b_op_end = (*curwin).w_cursor;

        if regname == c_int::from(b'.') && reg.is_null() {
            let non_linewise_vis = Visual.active && Visual.mode != c_int::from(b'V');

            let command_start_char = if non_linewise_vis {
                c_int::from(b'c')
            } else if (flags & PUT_LINE) != 0 {
                c_int::from(b'i')
            } else if dir == FORWARD {
                c_int::from(b'a')
            } else {
                c_int::from(b'i')
            };

            let has_textput_events = has_event(EVENT_TEXTPUTPRE) || has_event(EVENT_TEXTPUTPOST);
            if has_textput_events {
                add_last_insert += 1;
            }

            if (flags & PUT_LINE) != 0 {
                let save_add_last_insert = add_last_insert;
                add_last_insert = 0;
                stuffcharReadbuff(K_COMMAND);
                if dir == FORWARD {
                    stuffReadbuffLen(c"put _".as_ptr(), 5);
                } else {
                    stuffReadbuffLen(c"put! _".as_ptr(), 6);
                }
                stuffcharReadbuff(c_int::from(CAR as u8));
                add_last_insert = save_add_last_insert;
            }

            if (flags & PUT_LINE) != 0 {
                stuffcharReadbuff(command_start_char);
                while count > 0 {
                    stuff_inserted(0, 1, c_int::from(count != 1));
                    if count != 1 {
                        let s: [u8; 4] = [b'\n', b' ', Ctrl_U, 0];
                        stuffReadbuffLen(s.as_ptr().cast::<c_char>(), 3);
                    }
                    count -= 1;
                }
            } else {
                stuff_inserted(command_start_char, count, 0);
            }

            if has_event(EVENT_TEXTPUTPRE) {
                put_do_autocmd(c_int::from(b'.'), ptr::null_mut(), ptr::null(), false, dir);
            }
            if has_event(EVENT_TEXTPUTPOST) {
                put_do_autocmd(c_int::from(b'.'), ptr::null_mut(), ptr::null(), true, dir);
            }

            if has_textput_events {
                add_last_insert -= 1;
                if add_last_insert == 0 {
                    ga_clear(&raw mut last_insert_ga);
                }
            }

            if (flags & PUT_CURSEND) != 0 {
                if (flags & PUT_LINE) != 0 {
                    stuffReadbuff(c"j0".as_ptr());
                } else {
                    let cursor_pos = get_cursor_pos_ptr();
                    let one_past_line = *cursor_pos == NUL;
                    let mut eol = false;
                    if !one_past_line {
                        eol = *cursor_pos.add(utfc_ptr2len(cursor_pos) as usize) == NUL;
                    }

                    let ve_allows =
                        cur_ve_flags == kOptVeFlagAll || cur_ve_flags == kOptVeFlagOnemore;
                    let eof =
                        (*curbuf).b_ml.ml_line_count == (*curwin).w_cursor.lnum && one_past_line;
                    if ve_allows || !(eol || eof) {
                        stuffcharReadbuff(c_int::from(b'l'));
                    }
                }
            } else if (flags & PUT_LINE) != 0 {
                stuffReadbuff(c"g'[".as_ptr());
            }

            if command_start_char == c_int::from(b'a')
                && u_save((*curwin).w_cursor.lnum, (*curwin).w_cursor.lnum + 1) == FAIL
            {
                return;
            }

            exec_stuffed(ptr::null_mut());
            return;
        }

        let mut insert_string = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        if reg.is_null()
            && get_spec_reg(
                regname,
                &raw mut insert_string.data,
                &raw mut allocated,
                true,
            )
            && insert_string.data.is_null()
        {
            return;
        }

        if (*curbuf).terminal.is_null()
            && u_save((*curwin).w_cursor.lnum, (*curwin).w_cursor.lnum + 1) == FAIL
        {
            return;
        }

        if !insert_string.data.is_null() {
            insert_string.size = CStr::from_ptr(insert_string.data).to_bytes().len();
            y_type = MotionType::kMTCharWise;
            if regname == c_int::from(b'=') {
                loop {
                    y_size = 0;
                    let mut ptr_ = insert_string.data;
                    let mut ptrlen = insert_string.size;
                    while !ptr_.is_null() {
                        if !y_array.is_null() {
                            (*y_array.add(y_size)).data = ptr_;
                        }
                        y_size += 1;
                        let mut tmp = vim_strchr(ptr_, c_int::from(b'\n'));
                        if tmp.is_null() {
                            if !y_array.is_null() {
                                (*y_array.add(y_size - 1)).size = ptrlen;
                            }
                        } else {
                            if !y_array.is_null() {
                                *tmp = NUL;
                                (*y_array.add(y_size - 1)).size = (tmp as usize) - (ptr_ as usize);
                                ptrlen -= (*y_array.add(y_size - 1)).size + 1;
                            }
                            tmp = tmp.add(1);
                            if *tmp == NUL {
                                y_type = MotionType::kMTLineWise;
                                break;
                            }
                        }
                        ptr_ = tmp;
                    }
                    if !y_array.is_null() {
                        break;
                    }
                    y_array = xmalloc(y_size * size_of::<NvimString>()).cast::<NvimString>();
                }
            } else {
                y_size = 1;
                y_array = &raw mut insert_string;
            }
            if has_event(EVENT_TEXTPUTPRE) {
                put_do_autocmd(
                    regname,
                    ptr::null_mut(),
                    &raw const insert_string,
                    false,
                    dir,
                );
            }
        } else {
            if has_event(EVENT_TEXTPUTPRE) {
                let save_reg = reg;
                if reg.is_null() {
                    reg = get_yank_register(regname, YREG_PASTE);
                }
                put_do_autocmd(regname, reg, ptr::null(), false, dir);
                reg = save_reg;
            }
            if reg.is_null() {
                reg = get_yank_register(regname, YREG_PASTE);
            }

            y_type = (*reg).y_type;
            y_width = (*reg).y_width;
            y_size = (*reg).y_size;
            y_array = (*reg).y_array;
        }

        'end: {
            if !(*curbuf).terminal.is_null() {
                terminal_paste(count, y_array, y_size);
                break 'end;
            }

            let mut split_pos: colnr_T = 0;
            if y_type == MotionType::kMTLineWise {
                if (flags & PUT_LINE_SPLIT) != 0 {
                    if u_save_cursor() == FAIL {
                        break 'end;
                    }
                    let curline = get_cursor_line_ptr();
                    let mut p = get_cursor_pos_ptr();
                    let p_orig = p;
                    let plen = get_cursor_pos_len() as usize;
                    if dir == FORWARD && *p != NUL {
                        p = p.add(utfc_ptr2len(p) as usize);
                    }
                    split_pos = ((p as usize) - (curline as usize)) as colnr_T;

                    let mut ptr_ = xmemdupz(p.cast(), plen - ((p as usize) - (p_orig as usize)))
                        .cast::<c_char>();
                    ml_append((*curwin).w_cursor.lnum, ptr_, 0, false);
                    xfree(ptr_.cast());

                    ptr_ =
                        xmemdupz(get_cursor_line_ptr().cast(), split_pos as usize).cast::<c_char>();
                    ml_replace((*curwin).w_cursor.lnum, ptr_, false);
                    nr_lines += 1;
                    dir = FORWARD;

                    buf_updates_send_changes(curbuf, (*curwin).w_cursor.lnum, 1, 1);
                }
                if (flags & PUT_LINE_FORWARD) != 0 {
                    (*curwin).w_cursor = (*curbuf).b_visual.vi_end;
                    dir = FORWARD;
                }
                (*curbuf).b_op_start = (*curwin).w_cursor;
                (*curbuf).b_op_end = (*curwin).w_cursor;
            }

            if (flags & PUT_LINE) != 0 {
                y_type = MotionType::kMTLineWise;
            }

            if y_size == 0 || y_array.is_null() {
                semsg!(
                    gettext(c"E353: Nothing in register %s".as_ptr()),
                    if regname == 0 {
                        c"\"".as_ptr()
                    } else {
                        transchar(regname)
                    },
                );
                break 'end;
            }

            if y_type == MotionType::kMTBlockWise {
                lnum = (*curwin).w_cursor.lnum + (y_size as linenr_T) + 1;
                lnum = lnum.min((*curbuf).b_ml.ml_line_count + 1);
                if u_save((*curwin).w_cursor.lnum - 1, lnum) == FAIL {
                    break 'end;
                }
            } else if y_type == MotionType::kMTLineWise {
                lnum = (*curwin).w_cursor.lnum;
                if dir == BACKWARD {
                    hasFolding(curwin, lnum, &raw mut lnum, ptr::null_mut());
                } else {
                    hasFolding(curwin, lnum, ptr::null_mut(), &raw mut lnum);
                }
                if dir == FORWARD {
                    lnum += 1;
                }
                let save_res = if buf_is_empty(curbuf) {
                    u_save(0, 2)
                } else {
                    u_save(lnum - 1, lnum)
                };
                if save_res == FAIL {
                    break 'end;
                }
                if dir == FORWARD {
                    (*curwin).w_cursor.lnum = lnum - 1;
                } else {
                    (*curwin).w_cursor.lnum = lnum;
                }
                (*curbuf).b_op_start = (*curwin).w_cursor;
            } else if u_save_cursor() == FAIL {
                break 'end;
            }

            if cur_ve_flags == kOptVeFlagAll && y_type == MotionType::kMTCharWise {
                if gchar_cursor() == TAB {
                    let viscol = getviscol();
                    let ts = (*curbuf).b_p_ts;
                    let cond = if dir == FORWARD {
                        tabstop_padding(viscol, ts, (*curbuf).b_p_vts_array) != 1
                    } else {
                        (*curwin).w_cursor.coladd > 0
                    };
                    if cond {
                        coladvance_force(viscol);
                    } else {
                        (*curwin).w_cursor.coladd = 0;
                    }
                } else if (*curwin).w_cursor.coladd > 0 || gchar_cursor() == 0 {
                    coladvance_force(getviscol() + c_int::from(dir == FORWARD));
                }
            }

            lnum = (*curwin).w_cursor.lnum;
            let mut col = (*curwin).w_cursor.col;

            if y_type == MotionType::kMTBlockWise {
                let mut incr: c_int = 0;
                let mut bd = block_def::default();
                let c = gchar_cursor();
                let mut endcol2: colnr_T = 0;

                if dir == FORWARD && c != 0 {
                    if cur_ve_flags == kOptVeFlagAll {
                        getvcol(
                            curwin,
                            &raw mut (*curwin).w_cursor,
                            &raw mut col,
                            ptr::null_mut(),
                            &raw mut endcol2,
                            0,
                        );
                    } else {
                        getvcol(
                            curwin,
                            &raw mut (*curwin).w_cursor,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            &raw mut col,
                            0,
                        );
                    }
                    (*curwin).w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
                    col += 1;
                } else {
                    getvcol(
                        curwin,
                        &raw mut (*curwin).w_cursor,
                        &raw mut col,
                        ptr::null_mut(),
                        &raw mut endcol2,
                        0,
                    );
                }

                col += (*curwin).w_cursor.coladd;
                if cur_ve_flags == kOptVeFlagAll
                    && ((*curwin).w_cursor.coladd > 0 || endcol2 == (*curwin).w_cursor.col)
                {
                    if dir == FORWARD && c == 0 {
                        col += 1;
                    }
                    if dir != FORWARD && c != 0 && (*curwin).w_cursor.coladd > 0 {
                        (*curwin).w_cursor.col += 1;
                    }
                    if c == TAB {
                        if dir == BACKWARD && (*curwin).w_cursor.col != 0 {
                            (*curwin).w_cursor.col -= 1;
                        }
                        if dir == FORWARD && col - 1 == endcol2 {
                            (*curwin).w_cursor.col += 1;
                        }
                    }
                }
                (*curwin).w_cursor.coladd = 0;
                bd.textcol = 0;

                for i in 0..y_size {
                    let mut spaces: c_int = 0;
                    let mut lines_appended: c_int = 0;

                    bd.startspaces = 0;
                    bd.endspaces = 0;
                    vcol = 0;
                    let mut delcount: c_int = 0;

                    if (*curwin).w_cursor.lnum > (*curbuf).b_ml.ml_line_count {
                        if ml_append(
                            (*curbuf).b_ml.ml_line_count,
                            c"".as_ptr().cast_mut(),
                            1,
                            false,
                        ) == FAIL
                        {
                            break;
                        }
                        nr_lines += 1;
                        lines_appended = 1;
                    }
                    let oldp = get_cursor_line_ptr();
                    let oldlen = get_cursor_line_len();

                    let mut csarg: CharsizeArg = std::mem::zeroed();
                    let mut cstype: CSType =
                        init_charsize_arg(&raw mut csarg, curwin, (*curwin).w_cursor.lnum, oldp);
                    let mut ci = utf_ptr2StrCharInfo(oldp);
                    vcol = 0;
                    while vcol < col && *ci.ptr != NUL {
                        let cli = utf_ClusterInfo(ci);
                        incr = win_charsize(
                            cstype,
                            vcol,
                            ci.ptr,
                            ci.chr.value,
                            &raw mut csarg,
                            cli.cells,
                        )
                        .width;
                        vcol += incr;
                        ci = cli.next;
                    }
                    let mut ptr_ = ci.ptr.cast_mut();
                    bd.textcol = ((ptr_ as usize) - (oldp as usize)) as colnr_T;

                    let shortline = (vcol < col) || (vcol == col && *ptr_ == NUL);

                    if vcol < col {
                        bd.startspaces = col - vcol;
                    } else if vcol > col {
                        bd.endspaces = vcol - col;
                        bd.startspaces = incr - bd.endspaces;
                        bd.textcol -= 1;
                        delcount = 1;
                        bd.textcol -= utf_head_off(oldp, oldp.add(bd.textcol as usize));
                        if *oldp.add(bd.textcol as usize) != TAB as u8 as c_char {
                            delcount = 0;
                            bd.endspaces = 0;
                        }
                    }

                    let yanklen = (*y_array.add(i)).size as c_int;

                    if (flags & PUT_BLOCK_INNER) == 0 {
                        spaces = y_width + 1;

                        cstype =
                            init_charsize_arg(&raw mut csarg, curwin, 0, (*y_array.add(i)).data);
                        ci = utf_ptr2StrCharInfo((*y_array.add(i)).data);
                        while *ci.ptr != NUL {
                            let cli = utf_ClusterInfo(ci);
                            spaces -= win_charsize(
                                cstype,
                                0,
                                ci.ptr,
                                ci.chr.value,
                                &raw mut csarg,
                                cli.cells,
                            )
                            .width;
                            ci = cli.next;
                        }
                        spaces = spaces.max(0);
                    }

                    if yanklen + spaces != 0
                        && count
                            > ((c_int::MAX - (bd.startspaces + bd.endspaces)) / (yanklen + spaces))
                    {
                        emsg(gettext(e_resulting_text_too_long.as_ptr()));
                        break;
                    }

                    totlen = (count as usize) * ((yanklen + spaces) as usize)
                        + (bd.startspaces as usize)
                        + (bd.endspaces as usize);
                    let newp = xmalloc(totlen + (oldlen as usize) + 1).cast::<c_char>();

                    ptr_ = newp;
                    ptr::copy(oldp, ptr_, bd.textcol as usize);
                    ptr_ = ptr_.add(bd.textcol as usize);

                    ptr::write_bytes(ptr_.cast::<u8>(), b' ', bd.startspaces as usize);
                    ptr_ = ptr_.add(bd.startspaces as usize);

                    for j in 0..count {
                        ptr::copy((*y_array.add(i)).data, ptr_, yanklen as usize);
                        ptr_ = ptr_.add(yanklen as usize);

                        if (j < count - 1 || !shortline) && spaces > 0 {
                            ptr::write_bytes(ptr_.cast::<u8>(), b' ', spaces as usize);
                            ptr_ = ptr_.add(spaces as usize);
                        } else {
                            totlen -= spaces as usize;
                        }
                    }

                    ptr::write_bytes(ptr_.cast::<u8>(), b' ', bd.endspaces as usize);
                    ptr_ = ptr_.add(bd.endspaces as usize);

                    let columns = oldlen - bd.textcol - delcount + 1;
                    assert!(columns >= 0);
                    ptr::copy(
                        oldp.add((bd.textcol + delcount) as usize),
                        ptr_,
                        columns as usize,
                    );
                    ml_replace((*curwin).w_cursor.lnum, newp, false);
                    extmark_splice_cols(
                        curbuf,
                        (*curwin).w_cursor.lnum - 1,
                        bd.textcol,
                        delcount,
                        (totlen as c_int) + lines_appended,
                        kExtmarkUndo,
                    );

                    (*curwin).w_cursor.lnum += 1;
                    if i == 0 {
                        (*curwin).w_cursor.col += bd.startspaces;
                    }
                }

                changed_lines(
                    curbuf,
                    lnum,
                    0,
                    (*curbuf).b_op_start.lnum + (y_size as linenr_T) - nr_lines,
                    nr_lines,
                    true,
                );

                (*curbuf).b_op_start = (*curwin).w_cursor;
                (*curbuf).b_op_start.lnum = lnum;

                (*curbuf).b_op_end.lnum = (*curwin).w_cursor.lnum - 1;
                (*curbuf).b_op_end.col = (bd.textcol + (totlen as colnr_T) - 1).max(0);
                (*curbuf).b_op_end.coladd = 0;
                if (flags & PUT_CURSEND) != 0 {
                    (*curwin).w_cursor = (*curbuf).b_op_end;
                    (*curwin).w_cursor.col += 1;

                    let len = get_cursor_line_len();
                    (*curwin).w_cursor.col = (*curwin).w_cursor.col.min(len);
                } else {
                    (*curwin).w_cursor.lnum = lnum;
                }
            } else {
                let yanklen = (*y_array).size as c_int;

                if y_type == MotionType::kMTCharWise {
                    if dir == FORWARD && gchar_cursor() != 0 {
                        let bytelen = utfc_ptr2len(get_cursor_pos_ptr());
                        col += bytelen;
                        if yanklen != 0 {
                            (*curwin).w_cursor.col += bytelen;
                            (*curbuf).b_op_end.col += bytelen;
                        }
                    }
                    (*curbuf).b_op_start = (*curwin).w_cursor;
                } else if dir == BACKWARD {
                    lnum -= 1;
                }
                let new_cursor = (*curwin).w_cursor;

                if y_type == MotionType::kMTCharWise && y_size == 1 {
                    let mut end_lnum: linenr_T = 0;
                    let start_lnum = lnum;
                    let mut first_byte_off: c_int = 0;

                    if Visual.active {
                        end_lnum = (*curbuf)
                            .b_visual
                            .vi_end
                            .lnum
                            .max((*curbuf).b_visual.vi_start.lnum);
                        if end_lnum > start_lnum {
                            let mut pos = pos_T {
                                lnum,
                                col,
                                coladd: 0,
                            };
                            getvcol(
                                curwin,
                                &raw mut pos,
                                ptr::null_mut(),
                                &raw mut vcol,
                                ptr::null_mut(),
                                0,
                            );
                        }
                    }

                    if count == 0 || yanklen == 0 {
                        if Visual.active {
                            lnum = end_lnum;
                        }
                    } else if count > c_int::MAX / yanklen {
                        emsg(gettext(e_resulting_text_too_long.as_ptr()));
                    } else {
                        totlen = (count as usize) * (yanklen as usize);
                        loop {
                            let oldp = ml_get(lnum);
                            let oldlen = ml_get_len(lnum);
                            if lnum > start_lnum {
                                let mut pos = pos_T {
                                    lnum,
                                    col: 0,
                                    coladd: 0,
                                };
                                if getvpos(curwin, &raw mut pos, vcol) == OK {
                                    col = pos.col;
                                } else {
                                    col = MAXCOL;
                                }
                            }
                            if Visual.active && col > oldlen {
                                lnum += 1;
                                if !(Visual.active && lnum <= end_lnum) {
                                    break;
                                }
                                continue;
                            }
                            let newp = xmalloc(totlen + (oldlen as usize) + 1).cast::<c_char>();
                            ptr::copy(oldp, newp, col as usize);
                            let mut ptr_ = newp.add(col as usize);
                            for _ in 0..(count as usize) {
                                ptr::copy((*y_array).data, ptr_, yanklen as usize);
                                ptr_ = ptr_.add(yanklen as usize);
                            }
                            ptr::copy(oldp.add(col as usize), ptr_, ((oldlen - col) as usize) + 1);
                            ml_replace(lnum, newp, false);

                            first_byte_off = utf_head_off(newp, ptr_.sub(1));

                            if lnum == (*curwin).w_cursor.lnum {
                                changed_cline_bef_curs(curwin);
                                invalidate_botline_win(curwin);
                                (*curwin).w_cursor.col += (totlen - 1) as colnr_T;
                            }
                            changed_bytes(lnum, col);
                            extmark_splice_cols(
                                curbuf,
                                lnum - 1,
                                col,
                                0,
                                totlen as c_int,
                                kExtmarkUndo,
                            );
                            if Visual.active {
                                lnum += 1;
                            }
                            if !(Visual.active && lnum <= end_lnum) {
                                break;
                            }
                        }

                        if Visual.active {
                            lnum -= 1;
                        }
                    }

                    (*curbuf).b_op_end = (*curwin).w_cursor;
                    (*curbuf).b_op_end.col -= first_byte_off;

                    if totlen != 0 && (restart_edit != 0 || (flags & PUT_CURSEND) != 0) {
                        (*curwin).w_cursor.col += 1;
                    } else {
                        (*curwin).w_cursor.col -= first_byte_off;
                    }
                } else {
                    let mut new_lnum = new_cursor.lnum;
                    let mut orig_indent: c_int = 0;
                    let mut indent_diff: c_int = 0;
                    let mut first_indent = true;
                    let mut lendiff: c_int = 0;

                    if (flags & PUT_FIXINDENT) != 0 {
                        orig_indent = get_indent();
                    }

                    'error: {
                        for cnt in 1..=count {
                            let mut i: usize = 0;
                            if y_type == MotionType::kMTCharWise {
                                lnum = new_cursor.lnum;
                                let ptr_ = ml_get(lnum).add(col as usize);
                                let ptrlen = (ml_get_len(lnum) as usize) - (col as usize);
                                totlen = (*y_array.add(y_size - 1)).size;
                                let mut newp = xmalloc(ptrlen + totlen + 1).cast::<c_char>();
                                ptr::copy_nonoverlapping(
                                    (*y_array.add(y_size - 1)).data,
                                    newp,
                                    totlen,
                                );
                                ptr::copy_nonoverlapping(ptr_, newp.add(totlen), ptrlen + 1);
                                ml_append(lnum, newp, 0, false);
                                new_lnum += 1;
                                xfree(newp.cast());

                                let oldp = ml_get(lnum);
                                newp = xmalloc((col as usize) + (yanklen as usize) + 1)
                                    .cast::<c_char>();
                                ptr::copy(oldp, newp, col as usize);
                                ptr::copy(
                                    (*y_array).data,
                                    newp.add(col as usize),
                                    (yanklen as usize) + 1,
                                );
                                ml_replace(lnum, newp, false);

                                (*curwin).w_cursor.lnum = lnum;
                                i = 1;
                            }

                            while i < y_size {
                                if y_type != MotionType::kMTCharWise || i < y_size - 1 {
                                    if ml_append(lnum, (*y_array.add(i)).data, 0, false) == FAIL {
                                        break 'error;
                                    }
                                    new_lnum += 1;
                                }
                                lnum += 1;
                                nr_lines += 1;
                                if (flags & PUT_FIXINDENT) != 0 {
                                    let old_pos = (*curwin).w_cursor;
                                    (*curwin).w_cursor.lnum = lnum;
                                    let ptr_ = ml_get(lnum);
                                    if cnt == count && i == y_size - 1 {
                                        lendiff = ml_get_len(lnum);
                                    }
                                    let indent = if (*ptr_ == b'#' as c_char && preprocs_left())
                                        || *ptr_ == NUL
                                    {
                                        0
                                    } else if first_indent {
                                        indent_diff = orig_indent - get_indent();
                                        first_indent = false;
                                        orig_indent
                                    } else {
                                        (get_indent() + indent_diff).max(0)
                                    };
                                    set_indent(indent, SIN_NOMARK);
                                    (*curwin).w_cursor = old_pos;
                                    if cnt == count && i == y_size - 1 {
                                        lendiff -= ml_get_len(lnum);
                                    }
                                }
                                i += 1;
                            }

                            let mut totsize: bcount_t = 0;
                            let mut lastsize: c_int = 0;
                            if y_type == MotionType::kMTCharWise
                                || (y_type == MotionType::kMTLineWise
                                    && (flags & PUT_LINE_SPLIT) != 0)
                            {
                                for k in 0..(y_size - 1) {
                                    totsize += ((*y_array.add(k)).size as bcount_t) + 1;
                                }
                                lastsize = (*y_array.add(y_size - 1)).size as c_int;
                                totsize += lastsize as bcount_t;
                            }
                            if y_type == MotionType::kMTCharWise {
                                extmark_splice(
                                    curbuf,
                                    new_cursor.lnum - 1,
                                    col,
                                    0,
                                    0,
                                    0,
                                    (y_size as c_int) - 1,
                                    lastsize,
                                    totsize,
                                    kExtmarkUndo,
                                );
                            } else if y_type == MotionType::kMTLineWise
                                && (flags & PUT_LINE_SPLIT) != 0
                            {
                                extmark_splice(
                                    curbuf,
                                    new_cursor.lnum - 1,
                                    split_pos,
                                    0,
                                    0,
                                    0,
                                    (y_size as c_int) + 1,
                                    0,
                                    totsize + 2,
                                    kExtmarkUndo,
                                );
                            }

                            if cnt == 1 {
                                new_lnum = lnum;
                            }
                        }
                    }

                    if y_type == MotionType::kMTLineWise {
                        (*curbuf).b_op_start.col = 0;
                        if dir == FORWARD {
                            (*curbuf).b_op_start.lnum += 1;
                        }
                    }

                    let kind = if y_type == MotionType::kMTLineWise && (flags & PUT_LINE_SPLIT) == 0
                    {
                        kExtmarkUndo
                    } else {
                        kExtmarkNOOP
                    };
                    mark_adjust(
                        (*curbuf).b_op_start.lnum
                            + linenr_T::from(y_type == MotionType::kMTCharWise),
                        MAXLNUM,
                        nr_lines,
                        0,
                        kind,
                    );

                    if y_type == MotionType::kMTCharWise {
                        changed_lines(
                            curbuf,
                            (*curwin).w_cursor.lnum,
                            col,
                            (*curwin).w_cursor.lnum + 1,
                            nr_lines,
                            true,
                        );
                    } else {
                        changed_lines(
                            curbuf,
                            (*curbuf).b_op_start.lnum,
                            0,
                            (*curbuf).b_op_start.lnum,
                            nr_lines,
                            true,
                        );
                    }

                    (*curbuf).b_op_end.lnum = new_lnum;
                    let last_item = *y_array.add(y_size - 1);
                    col = ((last_item.size as colnr_T) - lendiff).max(0);
                    if col > 1 {
                        (*curbuf).b_op_end.col = col - 1;
                        if last_item.size > 0 {
                            (*curbuf).b_op_end.col -= utf_head_off(
                                last_item.data,
                                last_item.data.add(last_item.size - 1),
                            );
                        }
                    } else {
                        (*curbuf).b_op_end.col = 0;
                    }

                    if (flags & PUT_CURSLINE) != 0 {
                        (*curwin).w_cursor.lnum = lnum;
                        beginline(BL_WHITE | BL_FIX);
                    } else if (flags & PUT_CURSEND) != 0 {
                        if y_type == MotionType::kMTLineWise {
                            if lnum >= (*curbuf).b_ml.ml_line_count {
                                (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
                            } else {
                                (*curwin).w_cursor.lnum = lnum + 1;
                            }
                            (*curwin).w_cursor.col = 0;
                        } else {
                            (*curwin).w_cursor.lnum = new_lnum;
                            (*curwin).w_cursor.col = col;
                            (*curbuf).b_op_end = (*curwin).w_cursor;
                            if col > 1 {
                                (*curbuf).b_op_end.col = col - 1;
                            }
                        }
                    } else if y_type == MotionType::kMTLineWise {
                        (*curwin).w_cursor.col = 0;
                        if dir == FORWARD {
                            (*curwin).w_cursor.lnum += 1;
                        }
                        beginline(BL_WHITE | BL_FIX);
                    } else {
                        (*curwin).w_cursor = new_cursor;
                    }
                }
            }

            msgmore(nr_lines);
            (*curwin).w_set_curswant = true;

            let len = get_cursor_line_len();
            if (*curwin).w_cursor.col > len {
                if cur_ve_flags == kOptVeFlagAll {
                    (*curwin).w_cursor.coladd = (*curwin).w_cursor.col - len;
                }
                (*curwin).w_cursor.col = len;
            }
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            (*curbuf).b_op_start = orig_start;
            (*curbuf).b_op_end = orig_end;
        }

        if has_event(EVENT_TEXTPUTPOST) {
            if insert_string.data.is_null() {
                put_do_autocmd(regname, reg, ptr::null(), true, dir);
            } else {
                put_do_autocmd(
                    regname,
                    ptr::null_mut(),
                    &raw const insert_string,
                    true,
                    dir,
                );
            }
        }

        if allocated {
            xfree(insert_string.data.cast());
        }
        if regname == c_int::from(b'=') {
            xfree(y_array.cast());
        }

        if (*curbuf).terminal.is_null() {
            Visual.active = false;
        }

        adjust_cursor_eol();
    }
}

unsafe fn dis_msg(mut p: *const c_char, skip_esc: bool) {
    unsafe {
        let mut n = Columns - 6;
        while *p != NUL && !(*p == ESC && skip_esc && *p.add(1) == NUL) {
            n -= ptr2cells(p);
            if n < 0 {
                break;
            }
            let l = utfc_ptr2len(p);
            if l > 1 {
                msg_outtrans_len(p, l, 0, false);
                p = p.add(l as usize);
            } else {
                msg_outtrans_len(p, 1, 0, false);
                p = p.add(1);
            }
        }
        os_breakcheck();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_display(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
        if !arg.is_null() && *arg == NUL {
            arg = ptr::null_mut();
        }
        let hl_id = HLF_8;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        msg_ext_skip_flush = true;
        msg_puts_title(gettext(c"\nType Name Content".as_ptr()));

        let regs = (&raw mut y_regs).cast::<yankreg_T>();
        let mut i: c_int = -1;
        while i < (NUM_REGISTERS as c_int) && !got_int {
            let name = get_register_name(i);
            if !arg.is_null() && vim_strchr(arg, name).is_null() {
                i += 1;
                continue;
            }

            let type_ch = match get_reg_type(name, ptr::null_mut()) {
                MotionType::kMTLineWise => c_int::from(b'l'),
                MotionType::kMTCharWise => c_int::from(b'c'),
                _ => c_int::from(b'b'),
            };

            let mut yb = if i == -1 {
                if !y_previous.is_null() {
                    y_previous
                } else {
                    regs
                }
            } else {
                regs.add(i as usize)
            };

            get_clipboard(name, &raw mut yb, true);

            if name == mb_tolower(redir_reg)
                || (redir_reg == c_int::from(b'"') && ptr::eq(yb, y_previous))
            {
                i += 1;
                continue;
            }

            if !(*yb).y_array.is_null() {
                let mut do_show = false;
                for j in 0..(*yb).y_size {
                    if !message_filtered((*(*yb).y_array.add(j)).data) {
                        do_show = true;
                        break;
                    }
                }

                if do_show || (*yb).y_size == 0 {
                    msg_putchar(c_int::from(b'\n'));
                    msg_puts(c"  ".as_ptr());
                    msg_putchar(type_ch);
                    msg_puts(c"  ".as_ptr());
                    msg_putchar(c_int::from(b'"'));
                    msg_putchar(name);
                    msg_puts(c"   ".as_ptr());

                    let mut n = Columns - 11;
                    let mut j: usize = 0;
                    while j < (*yb).y_size && n > 1 {
                        if j != 0 {
                            msg_puts_hl(c"^J".as_ptr(), hl_id, false);
                            n -= 2;
                        }
                        let mut p: *const c_char = (*(*yb).y_array.add(j)).data;
                        while *p != NUL {
                            n -= ptr2cells(p);
                            if n < 0 {
                                break;
                            }
                            let clen = utfc_ptr2len(p);
                            msg_outtrans_len(p, clen, 0, false);
                            p = p.add(clen as usize);
                        }
                        j += 1;
                    }
                    if n > 1 && (*yb).y_type == MotionType::kMTLineWise {
                        msg_puts_hl(c"^J".as_ptr(), hl_id, false);
                    }
                }
                os_breakcheck();
            }

            i += 1;
        }

        let insert = get_last_insert();
        let p = insert.data;
        if !p.is_null()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b'.')).is_null())
            && !got_int
            && !message_filtered(p)
        {
            msg_puts(c"\n  c  \".   ".as_ptr());
            dis_msg(p, true);
        }

        if !last_cmdline.is_null()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b':')).is_null())
            && !got_int
            && !message_filtered(last_cmdline)
        {
            msg_puts(c"\n  c  \":   ".as_ptr());
            dis_msg(last_cmdline, false);
        }

        if !(*curbuf).b_fname.is_null()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b'%')).is_null())
            && !got_int
            && !message_filtered((*curbuf).b_fname)
        {
            msg_puts(c"\n  c  \"%   ".as_ptr());
            dis_msg((*curbuf).b_fname, false);
        }

        if (arg.is_null() || !vim_strchr(arg, c_int::from(b'#')).is_null()) && !got_int {
            let mut fname: *mut c_char = ptr::null_mut();
            let mut dummy: linenr_T = 0;
            if buflist_name_nr(0, &raw mut fname, &raw mut dummy) != FAIL
                && !message_filtered(fname)
            {
                msg_puts(c"\n  c  \"#   ".as_ptr());
                dis_msg(fname, false);
            }
        }

        if !last_search_pat().is_null()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b'/')).is_null())
            && !got_int
            && !message_filtered(last_search_pat())
        {
            msg_puts(c"\n  c  \"/   ".as_ptr());
            dis_msg(last_search_pat(), false);
        }

        if !expr_line.is_null()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b'=')).is_null())
            && !got_int
            && !message_filtered(expr_line)
        {
            msg_puts(c"\n  c  \"=   ".as_ptr());
            dis_msg(expr_line, false);
        }
        msg_ext_skip_flush = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_reg_type(regname: c_int, reg_width: *mut colnr_T) -> MotionType {
    unsafe {
        if regname == c_int::from(b'%')
            || regname == c_int::from(b'#')
            || regname == c_int::from(b'=')
            || regname == c_int::from(b':')
            || regname == c_int::from(b'/')
            || regname == c_int::from(b'.')
            || regname == Ctrl_F
            || regname == Ctrl_P
            || regname == Ctrl_W
            || regname == Ctrl_A
            || regname == c_int::from(b'_')
        {
            return MotionType::kMTCharWise;
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return MotionType::kMTUnknown;
        }

        let reg = get_yank_register(regname, YREG_PASTE);
        if !(*reg).y_array.is_null() {
            if !reg_width.is_null() && (*reg).y_type == MotionType::kMTBlockWise {
                *reg_width = (*reg).y_width;
            }
            return (*reg).y_type;
        }
        MotionType::kMTUnknown
    }
}

unsafe fn get_reg_wrap_one_line(s: *mut c_char, flags: c_int) -> *mut c_void {
    unsafe {
        if (flags & kGRegList) == 0 {
            return s.cast::<c_void>();
        }
        let list = tv_list_alloc(1);
        tv_list_append_allocated_string(list, s);
        list.cast::<c_void>()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_reg_contents(mut regname: c_int, flags: c_int) -> *mut c_void {
    unsafe {
        if regname == c_int::from(b'=') {
            if (flags & kGRegNoExpr) != 0 {
                return ptr::null_mut();
            }
            if (flags & kGRegExprSrc) != 0 {
                return get_reg_wrap_one_line(get_expr_line_src(), flags);
            }
            return get_reg_wrap_one_line(get_expr_line(), flags);
        }

        if regname == c_int::from(b'@') {
            regname = c_int::from(b'"');
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return ptr::null_mut();
        }

        let mut retval: *mut c_char = ptr::null_mut();
        let mut allocated = false;
        if get_spec_reg(regname, &raw mut retval, &raw mut allocated, false) {
            if retval.is_null() {
                return ptr::null_mut();
            }
            if allocated {
                return get_reg_wrap_one_line(retval, flags);
            }
            return get_reg_wrap_one_line(xstrdup(retval), flags);
        }

        let reg = get_yank_register(regname, YREG_PUT);
        if (*reg).y_array.is_null() {
            return ptr::null_mut();
        }

        if (flags & kGRegList) != 0 {
            let list = tv_list_alloc((*reg).y_size as isize);
            for i in 0..(*reg).y_size {
                let item = *(*reg).y_array.add(i);
                tv_list_append_string(list, item.data, item.size as isize);
            }
            return list.cast::<c_void>();
        }

        let mut len: usize = 0;
        for i in 0..(*reg).y_size {
            len += (*(*reg).y_array.add(i)).size;
            if (*reg).y_type == MotionType::kMTLineWise || i < (*reg).y_size - 1 {
                len += 1;
            }
        }

        retval = xmalloc(len + 1).cast::<c_char>();

        len = 0;
        for i in 0..(*reg).y_size {
            let item = *(*reg).y_array.add(i);
            ptr::copy_nonoverlapping(item.data, retval.add(len), item.size);
            len += item.size;
            if (*reg).y_type == MotionType::kMTLineWise || i < (*reg).y_size - 1 {
                *retval.add(len) = b'\n' as c_char;
                len += 1;
            }
        }
        *retval.add(len) = NUL;

        retval.cast::<c_void>()
    }
}

unsafe fn init_write_reg(
    name: c_int,
    old_y_previous: *mut *mut yankreg_T,
    must_append: bool,
) -> *mut yankreg_T {
    unsafe {
        if !valid_yank_reg(name, true) {
            emsg_invreg(name);
            return ptr::null_mut();
        }

        *old_y_previous = y_previous;

        let reg = get_yank_register(name, YREG_YANK);
        if !is_append_register(name) && !must_append {
            free_register(reg);
        }
        reg
    }
}

unsafe fn str_to_reg(
    y_ptr: *mut yankreg_T,
    mut yank_type: MotionType,
    str_: *const c_char,
    len: usize,
    blocklen: colnr_T,
    str_list: bool,
) {
    unsafe {
        if (*y_ptr).y_array.is_null() {
            (*y_ptr).y_size = 0;
        }

        if yank_type == MotionType::kMTUnknown {
            yank_type = if str_list
                || (len > 0 && (*str_.add(len - 1) == NL || *str_.add(len - 1) == CAR))
            {
                MotionType::kMTLineWise
            } else {
                MotionType::kMTCharWise
            };
        }

        let mut newlines: usize = 0;
        let mut extraline: usize = 0;
        let mut append = false;

        if str_list {
            let mut ss = str_.cast::<*mut c_char>();
            while !(*ss).is_null() {
                newlines += 1;
                ss = ss.add(1);
            }
        } else {
            newlines = memcnt(str_.cast(), b'\n' as c_char, len);
            if yank_type == MotionType::kMTCharWise
                || len == 0
                || *str_.add(len - 1) != b'\n' as c_char
            {
                extraline = 1;
                newlines += 1;
            }
            if (*y_ptr).y_size > 0 && (*y_ptr).y_type == MotionType::kMTCharWise {
                append = true;
                newlines -= 1;
            }
        }

        if (*y_ptr).y_size + newlines == 0 {
            xfree((*y_ptr).y_array.cast());
            (*y_ptr).y_array = ptr::null_mut();
            return;
        }

        let pp = xrealloc(
            (*y_ptr).y_array.cast(),
            ((*y_ptr).y_size + newlines) * size_of::<NvimString>(),
        )
        .cast::<NvimString>();
        (*y_ptr).y_array = pp;

        let mut lnum = (*y_ptr).y_size;
        let mut maxlen: usize = 0;

        if str_list {
            let mut ss = str_.cast::<*mut c_char>();
            while !(*ss).is_null() {
                *pp.add(lnum) = cstr_to_string(*ss);
                if yank_type == MotionType::kMTBlockWise {
                    let charlen = vim_strsize(*ss) as usize;
                    maxlen = maxlen.max(charlen);
                }
                ss = ss.add(1);
                lnum += 1;
            }
        } else {
            let mut start = str_;
            let end = str_.add(len);
            let limit = end.add(extraline);
            while (start as usize) < (limit as usize) {
                let rem = (end as usize).saturating_sub(start as usize);
                let slice = std::slice::from_raw_parts(start.cast::<u8>(), rem);
                let line_len = slice.iter().position(|&b| b == b'\n').unwrap_or(rem);
                if yank_type == MotionType::kMTBlockWise {
                    let charlen = mb_string2cells_len(start, line_len);
                    maxlen = maxlen.max(charlen);
                }

                let extra = if append {
                    lnum -= 1;
                    (*pp.add(lnum)).size
                } else {
                    0
                };
                let s = xmallocz(line_len + extra).cast::<c_char>();
                if extra > 0 {
                    ptr::copy_nonoverlapping((*pp.add(lnum)).data, s, extra);
                }
                if line_len > 0 {
                    ptr::copy_nonoverlapping(start, s.add(extra), line_len);
                }
                let s_len = extra + line_len;

                if append {
                    xfree((*pp.add(lnum)).data.cast());
                    append = false;
                }
                *pp.add(lnum) = cbuf_as_string(s, s_len);

                memchrsub((*pp.add(lnum)).data.cast(), NUL, b'\n' as c_char, s_len);

                start = start.add(line_len + 1);
                lnum += 1;
            }
        }
        (*y_ptr).y_type = yank_type;
        (*y_ptr).y_size = lnum;
        xfree((*y_ptr).additional_data);
        (*y_ptr).additional_data = ptr::null_mut();
        (*y_ptr).timestamp = os_realtime() as u64;
        if yank_type == MotionType::kMTBlockWise {
            (*y_ptr).y_width = if blocklen == -1 {
                (maxlen as colnr_T) - 1
            } else {
                blocklen
            };
        } else {
            (*y_ptr).y_width = 0;
        }
    }
}

unsafe fn finish_write_reg(name: c_int, reg: *mut yankreg_T, old_y_previous: *mut yankreg_T) {
    unsafe {
        set_clipboard(name, reg);
        if name != c_int::from(b'"') {
            y_previous = old_y_previous;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn write_reg_contents(
    name: c_int,
    str_: *const c_char,
    len: isize,
    must_append: c_int,
) {
    unsafe {
        write_reg_contents_ex(name, str_, len, must_append != 0, MotionType::kMTUnknown, 0);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn write_reg_contents_lst(
    name: c_int,
    strings: *mut *mut c_char,
    must_append: bool,
    yank_type: MotionType,
    block_len: colnr_T,
) {
    unsafe {
        if name == c_int::from(b'/') || name == c_int::from(b'=') || name == c_int::from(b'#') {
            let s = if (*strings).is_null() {
                c"".as_ptr()
            } else if !(*strings.add(1)).is_null() {
                semsg!(
                    gettext(E_REGISTER_CHAR_CANNOT_CONTAIN_MULTIPLE_LINES.as_ptr()),
                    name,
                );
                return;
            } else {
                *strings
            };
            write_reg_contents_ex(name, s, -1, must_append, yank_type, block_len);
            return;
        }

        if name == c_int::from(b'_') {
            return;
        }

        let mut old_y_previous: *mut yankreg_T = ptr::null_mut();
        let reg = init_write_reg(name, &raw mut old_y_previous, must_append);
        if reg.is_null() {
            return;
        }

        str_to_reg(reg, yank_type, strings.cast::<c_char>(), 0, block_len, true);
        finish_write_reg(name, reg, old_y_previous);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn write_reg_contents_ex(
    name: c_int,
    str_: *const c_char,
    mut len: isize,
    must_append: bool,
    yank_type: MotionType,
    block_len: colnr_T,
) {
    unsafe {
        if len < 0 {
            len = CStr::from_ptr(str_).to_bytes().len() as isize;
        }

        if name == c_int::from(b'/') {
            set_last_search_pat(str_, RE_SEARCH, 1, true);
            return;
        }

        if name == c_int::from(b'#') {
            if len == 0 {
                (*curwin).w_alt_fnum = 0;
                return;
            }

            let buf = if (*str_ as u8).is_ascii_digit() {
                let num = atoi(str_);
                let b = buflist_findnr(num);
                if b.is_null() {
                    semsg!(gettext(e_nobufnr.as_ptr()), i64::from(num));
                }
                b
            } else {
                buflist_findnr(buflist_findpat(
                    str_,
                    str_.add(len as usize),
                    true,
                    false,
                    false,
                ))
            };
            if buf.is_null() {
                return;
            }
            (*curwin).w_alt_fnum = (*buf).b_fnum;
            return;
        }

        if name == c_int::from(b'=') {
            let mut offset: usize = 0;
            let mut totlen = len as usize;

            if must_append && !expr_line.is_null() {
                let exprlen = CStr::from_ptr(expr_line).to_bytes().len();
                totlen += exprlen;
                offset = exprlen;
            }

            expr_line = xrealloc(expr_line.cast(), totlen + 1).cast::<c_char>();
            ptr::copy_nonoverlapping(str_, expr_line.add(offset), len as usize);
            *expr_line.add(totlen) = NUL;
            return;
        }

        if name == c_int::from(b'_') {
            return;
        }

        let mut old_y_previous: *mut yankreg_T = ptr::null_mut();
        let reg = init_write_reg(name, &raw mut old_y_previous, must_append);
        if reg.is_null() {
            return;
        }
        str_to_reg(reg, yank_type, str_, len as usize, block_len, false);
        finish_write_reg(name, reg, old_y_previous);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prepare_yankreg_from_object(
    reg: *mut yankreg_T,
    regtype: NvimString,
    _lines: usize,
) -> bool {
    unsafe {
        let type_ch = if !regtype.data.is_null() {
            *regtype.data as u8
        } else {
            0
        };

        (*reg).y_type = match type_ch {
            0 => MotionType::kMTUnknown,
            b'v' | b'c' => MotionType::kMTCharWise,
            b'V' | b'l' => MotionType::kMTLineWise,
            b'b' | 0x16 => MotionType::kMTBlockWise,
            _ => return false,
        };

        (*reg).y_width = 0;
        if regtype.size > 1 {
            if (*reg).y_type != MotionType::kMTBlockWise {
                return false;
            }
            if !(*regtype.data.add(1) as u8).is_ascii_digit() {
                return false;
            }
            let mut p = regtype.data.add(1);
            (*reg).y_width = getdigits_int(&raw mut p, false, 1) - 1;
            if regtype.size > (p as usize) - (regtype.data as usize) {
                return false;
            }
        }

        (*reg).additional_data = ptr::null_mut();
        (*reg).timestamp = 0;
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn finish_yankreg_from_object(reg: *mut yankreg_T, clipboard_adjust: bool) {
    unsafe {
        if (*reg).y_size > 0 && (*(*reg).y_array.add((*reg).y_size - 1)).size == 0 {
            if (*reg).y_type != MotionType::kMTCharWise {
                if (*reg).y_type == MotionType::kMTUnknown || clipboard_adjust {
                    (*reg).y_size -= 1;
                }
                if (*reg).y_type == MotionType::kMTUnknown {
                    (*reg).y_type = MotionType::kMTLineWise;
                }
            }
        } else if (*reg).y_type == MotionType::kMTUnknown {
            (*reg).y_type = MotionType::kMTCharWise;
        }

        update_yankreg_width(reg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_index_and_name_roundtrip() {
        assert_eq!(op_reg_index(c_int::from(b'0')), 0);
        assert_eq!(op_reg_index(c_int::from(b'9')), 9);
        assert_eq!(op_reg_index(c_int::from(b'a')), 10);
        assert_eq!(op_reg_index(c_int::from(b'z')), 35);
        assert_eq!(op_reg_index(c_int::from(b'A')), 10);
        assert_eq!(op_reg_index(c_int::from(b'Z')), 35);
        assert_eq!(op_reg_index(c_int::from(b'-')), DELETION_REGISTER);
        assert_eq!(op_reg_index(c_int::from(b'*')), STAR_REGISTER);
        assert_eq!(op_reg_index(c_int::from(b'+')), PLUS_REGISTER);
        assert_eq!(op_reg_index(c_int::from(b'%')), -1);

        assert_eq!(get_register_name(-1), c_int::from(b'"'));
        for idx in 0..(NUM_REGISTERS as c_int) {
            let name = get_register_name(idx);
            assert_eq!(op_reg_index(name), idx);
        }
    }
}
