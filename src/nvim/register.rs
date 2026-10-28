//! Functions for managing registers (`src/nvim/register.c`).

use crate::api::private::helpers::{api_clear_string, cbuf_to_string};
use crate::api::private::validate::NvimString;
use crate::autocmd::{
    EVENT_RECORDINGENTER, EVENT_RECORDINGLEAVE, EVENT_TEXTPUTPOST, EVENT_TEXTPUTPRE,
    EVENT_TEXTYANKPOST, HLF_8, apply_autocmds, has_event,
};
use crate::buffer::{
    MAXLNUM, VAR_FIXED, buf_is_empty, buflist_findnr, buflist_findpat, buflist_name_nr,
    getaltfname, kUIMessages,
};
use crate::buffer_updates::buf_updates_send_changes;
use crate::change::{SIN_NOMARK, changed_bytes, changed_lines, del_chars, kOptVeFlagOnemore};
use crate::charset::{NL, NUL, ascii_iswhite, getdigits_int, skipwhite, transchar, vim_strsize};
use crate::clipboard::{adjust_clipboard_name, get_clipboard, set_clipboard};
use crate::cmdexpand::Ctrl_P;
use crate::cursor::{
    coladvance_force, colnr_T, gchar_cursor, get_cursor_line_len, get_cursor_line_ptr,
    get_cursor_pos_len, get_cursor_pos_ptr, getviscol, getvpos, kOptVeFlagAll, linenr_T, pos_T,
};
use crate::digraph::{kBoolVarFalse, kBoolVarTrue};
use crate::drawscreen::{showmode, update_screen};
use crate::errors::*;
use crate::eval::funcs::RE_SEARCH;
use crate::eval::typval::{
    dict_T, list_T, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_str, tv_dict_add_str_len,
    tv_dict_set_keys_readonly, tv_list_alloc,
    tv_list_append_string,
};
use crate::eval::{ESC, eval_to_string, get_v_event, restore_v_event};
use crate::ex_cmds::{BL_FIX, BL_WHITE};
use crate::ex_cmds2::check_fname;
use crate::ex_docmd::{CAR, CMOD_LOCKMARKS, exarg_T};
use crate::ex_getln::{Ctrl_A, Ctrl_F, Ctrl_L, Ctrl_R, Ctrl_W, cmdline_paste_str, getcmdline};
use crate::extmark::{bcount_t, extmark_splice, extmark_splice_cols, kExtmarkNOOP, kExtmarkUndo};
use crate::file_search::{FNAME_EXP, FNAME_HYP, FNAME_MESS, file_name_at_cursor};
use crate::fold::hasFolding;
use crate::globals::*;
use crate::indent::{get_indent, preprocs_left, set_indent, tabstop_padding};
use crate::input::{
    beep_flush, get_recorded, ins_typebuf, redo_append_char, stuffReadbuff, stuffReadbuffLen,
    stuffcharReadbuff, stuffescaped,
};
use crate::insert::{beginline, get_last_insert, get_last_insert_save, oneright, stuff_inserted};
use crate::insexpand::{ins_compl_delete, ins_compl_preinsert_effect};
use crate::keycodes::{
    K_COMMAND, K_SPECIAL, KE_COMMAND, KS_EXTRA, TAB, vim_strsave_escape_ks, vim_unescape_ks,
};
use crate::mapping::{REMAP_NONE, REMAP_YES};
use crate::mark::mark_adjust;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_charlen, mb_string2cells_len, mb_tolower, ptr2cells, utf_ClusterInfo, utf_head_off,
    utf_ptr2StrCharInfo, utfc_ptr2len,
};
use crate::mcursor::mc_ins_cascading;
use crate::memline::{decl, ml_append, ml_get, ml_get_buf, ml_get_len, ml_replace};
use crate::message::{
    emsg, emsg_invreg, message_filtered, msg, msg_ext_set_kind, msg_outtrans_len, msg_putchar,
    msg_puts, msg_puts_hl, msg_puts_title, msgmore, semsg, smsg,
};
use crate::r#move::{changed_cline_bef_curs, invalidate_botline_win, update_topline};
use crate::normal::{FIND_IDENT, FIND_STRING, exec_stuffed, find_ident_under_cursor};
use crate::ops::{adjust_cursor_eol, block_prep, charwise_block_prep, get_op_char};
use crate::option::get_ve_flags;
use crate::option_vars::*;
use crate::os::input::os_breakcheck;
use crate::os::time::{Timestamp, os_realtime};
use crate::plines::{CSType, CharsizeArg, getvcol, init_charsize_arg, win_charsize};
use crate::regexp::{Ctrl_V, MAXCOL, NUMBUFLEN};
use crate::search::{BACKWARD, Direction, FORWARD, last_search_pat, set_last_search_pat};
use crate::state::{REPLACE_FLAG, save_v_event_T};
use crate::strings::{
    gettext, ngettext, vim_snprintf, vim_snprintf_safelen, vim_strchr,
    vim_strsave_escaped_ext,
};
use crate::terminal::terminal_paste;
use crate::textformat::oparg_T;
use crate::ui::ui_has;
use crate::undo::{u_save, u_save_cursor};
use std::ffi::{CStr, c_char, c_int, c_ulong};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionType {
    kMTUnknown = -1,
    kMTCharWise = 0,
    kMTLineWise = 1,
    kMTBlockWise = 2,
}

#[derive(Clone, Debug)]
pub struct yankreg_T {
    pub y_array: Vec<NvimString>,
    pub y_allocated: bool,
    pub y_type: MotionType,
    pub y_width: c_int,
    pub timestamp: u64,
    pub additional_data: Option<crate::search::AdditionalData>,
}

impl Drop for yankreg_T {
    fn drop(&mut self) {
        for item in self.y_array.iter_mut().rev() {
                api_clear_string(item);
            }
    }
}

const Ctrl_U: u8 = 21;

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

pub const YREG_PASTE: c_int = 0;
pub const YREG_YANK: c_int = 1;
pub const YREG_PUT: c_int = 2;

const kCpoRegappend: c_int = b'>' as c_int;

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

impl block_def {
    pub const fn new() -> Self {
        Self {
            startspaces: 0,
            endspaces: 0,
            textlen: 0,
            textstart: ptr::null_mut(),
            textcol: 0,
            start_vcol: 0,
            end_vcol: 0,
            is_short: 0,
            is_MAX: 0,
            is_oneChar: 0,
            pre_whitesp: 0,
            pre_whitesp_c: 0,
            end_char_vcols: 0,
            start_char_vcols: 0,
        }
    }
}

pub const EMPTY_YANKREG: yankreg_T = yankreg_T {
    y_array: Vec::new(),
    y_allocated: false,
    y_type: MotionType::kMTCharWise,
    y_width: 0,
    timestamp: 0,
    additional_data: None,
};

struct RegisterModuleState {
    expr_line: Option<std::ffi::CString>,
    get_expr_line_nested: c_int,
    execreg_lastc: c_int,
    y_regs: [yankreg_T; NUM_REGISTERS],
    y_previous: *mut yankreg_T,
    empty_reg: yankreg_T,
    do_record_regname: c_int,
    textyankpost_recursive: bool,
    put_autocmd_recursive: bool,
}

impl RegisterModuleState {
    const fn new() -> Self {
        Self {
            expr_line: None,
            get_expr_line_nested: 0,
            execreg_lastc: 0,
            y_regs: [const { EMPTY_YANKREG }; NUM_REGISTERS],
            y_previous: ptr::null_mut(),
            empty_reg: EMPTY_YANKREG,
            do_record_regname: 0,
            textyankpost_recursive: false,
            put_autocmd_recursive: false,
        }
    }
}

static REG_STATE: GlobalState<RegisterModuleState> = GlobalState::new(RegisterModuleState::new());

#[inline]
fn reg_st() -> &'static mut RegisterModuleState {
    unsafe { &mut *REG_STATE.as_ptr() }
}

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
pub fn reg_empty(reg: *const yankreg_T) -> bool {
    unsafe {
        let reg = &*reg;
        !reg.y_allocated
            || reg.y_array.is_empty()
            || (reg.y_array.len() == 1
                && reg.y_type == MotionType::kMTCharWise
                && reg.y_array[0].size == 0)
    }
}

use crate::api::private::helpers::cstr_to_string;

#[inline]
fn copy_string_global(s: NvimString) -> NvimString {
    if s.data.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        cbuf_to_string(s.data, s.size)
    }
}

pub fn get_unname_register() -> c_int {
    unsafe {
        if reg_st().y_previous.is_null() {
            -1
        } else {
            reg_st().y_previous.offset_from((&raw const reg_st().y_regs).cast::<yankreg_T>()) as c_int
        }
    }
}

pub fn get_y_register(reg: c_int) -> *mut yankreg_T {
    unsafe { (&raw mut reg_st().y_regs).cast::<yankreg_T>().add(reg as usize) }
}

pub fn get_y_previous() -> *mut yankreg_T {
    reg_st().y_previous
}

pub fn get_expr_register() -> c_int {
    let Some(new_line) = getcmdline(c_int::from(b'='), 0, 0, true) else {
            return 0;
        };
        if !new_line.as_bytes().is_empty() {
            set_expr_line(Some(new_line));
        }
        c_int::from(b'=')
}

pub fn set_expr_line(new_line: Option<std::ffi::CString>) {
    reg_st().expr_line = new_line;
}

pub fn get_expr_line() -> Option<std::ffi::CString> {
    let expr_copy = reg_st().expr_line.clone()?;
        if reg_st().get_expr_line_nested >= 10 {
            return Some(expr_copy);
        }

        reg_st().get_expr_line_nested += 1;
        let rv = eval_to_string(expr_copy.as_ptr().cast_mut(), true, false);
        reg_st().get_expr_line_nested -= 1;
        rv
}

pub fn get_expr_line_src() -> Option<std::ffi::CString> {
    reg_st().expr_line.clone()
}

pub fn valid_yank_reg(regname: c_int, writing: bool) -> bool {
    (regname > 0 && regname <= 0x7f && (regname as u8).is_ascii_alphanumeric())
            || (!writing && !vim_strchr(c"/#.%:=".as_ptr(), regname).is_null())
            || regname == c_int::from(b'"')
            || regname == c_int::from(b'-')
            || regname == c_int::from(b'_')
            || regname == c_int::from(b'*')
            || regname == c_int::from(b'+')
}

pub fn get_default_register_name() -> c_int {
    let mut name: c_int = 0;
        adjust_clipboard_name(&mut name, true, false);
        name
}

pub fn op_reg_iter(
    iter: Option<usize>,
    regs: &[yankreg_T],
    name: &mut c_char,
    reg: &mut *const yankreg_T,
    is_unnamed: &mut bool,
) -> Option<usize> {
    *name = NUL;
        let mut idx = iter.unwrap_or(0);
        while idx < NUM_SAVED_REGISTERS && reg_empty(&raw const regs[idx]) {
            idx += 1;
        }
        if idx >= NUM_SAVED_REGISTERS || reg_empty(&raw const regs[idx]) {
            return None;
        }
        *name = get_register_name(idx as c_int) as u8 as c_char;
        let iter_reg = &raw const regs[idx];
        *reg = iter_reg;
        *is_unnamed = ptr::eq(iter_reg, reg_st().y_previous);
        idx += 1;
        while idx < NUM_SAVED_REGISTERS {
            if !reg_empty(&raw const regs[idx]) {
                return Some(idx);
            }
            idx += 1;
        }
        None
}

pub fn op_global_reg_iter(
    iter: Option<usize>,
    name: &mut c_char,
    reg: &mut *const yankreg_T,
    is_unnamed: &mut bool,
) -> Option<usize> {
    op_reg_iter(
            iter,
            &reg_st().y_regs,
            name,
            reg,
            is_unnamed,
        )
}

pub fn reg_max_ts(clipboard: bool) -> Timestamp {
    unsafe {
        let mut max_ts: Timestamp = 0;
        let mut iter: Option<usize> = None;
        loop {
            let mut name: c_char = NUL;
            let mut reg: *const yankreg_T = ptr::null();
            let mut is_unnamed = false;
            iter = op_global_reg_iter(iter, &mut name, &mut reg, &mut is_unnamed);
            if name == NUL {
                break;
            }
            max_ts = max_ts.max((*reg).timestamp);
            if iter.is_none() {
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

pub fn op_reg_set(name: c_char, reg: yankreg_T, is_unnamed: bool) -> bool {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return false;
        }
        let target = (&raw mut reg_st().y_regs).cast::<yankreg_T>().add(i as usize);
        *target = reg;

        if is_unnamed {
            reg_st().y_previous = target;
        }
        true
    }
}

pub fn op_reg_get(name: c_char) -> *const yankreg_T {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return ptr::null();
        }
        (&raw const reg_st().y_regs).cast::<yankreg_T>().add(i as usize)
    }
}

pub fn op_reg_set_previous(name: c_char) -> bool {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return false;
        }
        reg_st().y_previous = (&raw mut reg_st().y_regs).cast::<yankreg_T>().add(i as usize);
        true
    }
}

pub fn update_yankreg_width(reg: *mut yankreg_T) {
    unsafe {
        let reg = &mut *reg;
        if reg.y_type == MotionType::kMTBlockWise {
            let mut maxlen: usize = 0;
            for &item in &reg.y_array {
                let rowlen = mb_string2cells_len(item.data, item.size);
                maxlen = maxlen.max(rowlen);
            }
            assert!(maxlen <= c_int::MAX as usize);
            reg.y_width = reg.y_width.max((maxlen as colnr_T) - 1);
        }
    }
}

pub fn get_yank_register(regname: c_int, mode: c_int) -> *mut yankreg_T {
    unsafe {
        let mut reg: *mut yankreg_T = ptr::null_mut();

        if (mode == YREG_PASTE || mode == YREG_PUT) && get_clipboard(regname, &raw mut reg, false) {
            return reg;
        } else if mode == YREG_PUT && (regname == c_int::from(b'*') || regname == c_int::from(b'+'))
        {
            return &raw mut reg_st().empty_reg;
        } else if mode != YREG_YANK
            && (regname == 0
                || regname == c_int::from(b'"')
                || regname == c_int::from(b'*')
                || regname == c_int::from(b'+'))
            && !reg_st().y_previous.is_null()
        {
            return reg_st().y_previous;
        }

        let mut i = op_reg_index(regname);
        if i == -1 {
            i = 0;
        }
        reg = (&raw mut reg_st().y_regs).cast::<yankreg_T>().add(i as usize);

        if mode == YREG_YANK {
            reg_st().y_previous = reg;
        }
        reg
    }
}

pub fn yank_register_mline(regname: c_int, reg: *mut *mut yankreg_T) -> bool {
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

pub fn copy_register(name: c_int) -> yankreg_T {
    unsafe {
        let reg = &*get_yank_register(name, YREG_PASTE);
        let y_array = reg.y_array.iter().map(|&s| copy_string_global(s)).collect();
        yankreg_T {
            y_array,
            y_allocated: reg.y_allocated,
            y_type: reg.y_type,
            y_width: reg.y_width,
            timestamp: reg.timestamp,
            additional_data: None,
        }
    }
}

fn stuff_yank(regname: c_int, p: *const c_char) -> c_int {
    unsafe {
        if regname != 0 && !valid_yank_reg(regname, true) {
            return FAIL;
        }
        if regname == c_int::from(b'_') {
            return OK;
        }

        let plen = CStr::from_ptr(p).to_bytes().len();
        let reg = &mut *get_yank_register(regname, YREG_YANK);
        if is_append_register(regname) && reg.y_allocated && !reg.y_array.is_empty() {
            let pp = reg.y_array.last_mut().unwrap();
            let tmplen = pp.size + plen;
            let mut tmp = vec![0 as c_char; tmplen + 1];
            ptr::copy_nonoverlapping(pp.data, tmp.as_mut_ptr(), pp.size);
            ptr::copy_nonoverlapping(p, tmp.as_mut_ptr().add(pp.size), plen);
            api_clear_string(pp);
            *pp = cbuf_to_string(tmp.as_ptr(), tmplen);
        } else {
            *reg = EMPTY_YANKREG;
            reg.additional_data = None;
            reg.y_array = vec![cbuf_to_string(p, plen)];
            reg.y_allocated = true;
            reg.y_type = MotionType::kMTCharWise;
        }
        reg.timestamp = os_realtime() as u64;
        OK
    }
}

pub fn do_record(c: c_int) -> c_int {
    unsafe {
        let retval: c_int;

        if g_st().reg_recording == 0 {
            if c < 0
                || (!(c <= 0x7f && (c as u8).is_ascii_alphanumeric()) && c != c_int::from(b'"'))
            {
                retval = FAIL;
            } else {
                g_st().reg_recording = c;
                showmode();
                reg_st().do_record_regname = c;
                retval = OK;
                apply_autocmds(
                    EVENT_RECORDINGENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
            }
        } else {
            let mut save_v_event: save_v_event_T = save_v_event_T::new();
            let dict = get_v_event(&mut save_v_event);

            let mut p_vec = get_recorded().into_bytes_with_nul();
            let p = p_vec.as_mut_ptr().cast::<c_char>();
            vim_unescape_ks(p);
            tv_dict_add_str(dict, c"regcontents".as_ptr(), 11, p);

            let mut buf = [0 as c_char; NUMBUFLEN + 2];
            buf[0] = reg_st().do_record_regname as u8 as c_char;
            buf[1] = NUL;
            tv_dict_add_str(dict, c"regname".as_ptr(), 7, buf.as_ptr());
            tv_dict_set_keys_readonly(&mut *dict);

            apply_autocmds(
                EVENT_RECORDINGLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            restore_v_event(&mut *dict, &mut save_v_event);
            g_st().reg_recorded = g_st().reg_recording;
            g_st().reg_recording = 0;
            if opt_vars().p_ch == 0 || ui_has(kUIMessages) {
                showmode();
            } else {
                msg(c"".as_ptr(), 0);
            }
            let old_y_previous = reg_st().y_previous;
            retval = stuff_yank(reg_st().do_record_regname, p);
            reg_st().y_previous = old_y_previous;
        }
        retval
    }
}

fn put_in_typebuf(s: *mut c_char, esc: bool, colon: bool, silent: c_int) -> c_int {
    let mut retval = OK;
        let is_silent = silent != 0;

        put_reedit_in_typebuf(silent);
        if colon {
            retval = ins_typebuf(c"\n".as_ptr().cast_mut(), REMAP_NONE, 0, true, is_silent);
        }
        if retval == OK {
            let owned_p = if esc {
                Some(vim_strsave_escape_ks(s))
            } else {
                None
            };
            let p = owned_p.as_ref().map_or(s, |cs| cs.as_ptr().cast_mut());
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
        }
        if colon && retval == OK {
            retval = ins_typebuf(c":".as_ptr().cast_mut(), REMAP_NONE, 0, true, is_silent);
        }
        retval
}

fn put_reedit_in_typebuf(silent: c_int) {
    if g_st().restart_edit == 0 {
            return;
        }

        let mut buf: [u8; 11] = [
            K_SPECIAL, KS_EXTRA, KE_COMMAND, b's', b't', b'a', b'r', b't', b'i', CAR as u8, 0,
        ];
        if g_st().restart_edit == c_int::from(b'R') {
            buf[8] = b'r';
        } else if g_st().restart_edit == c_int::from(b'V') {
            buf[8] = b'g';
        } else if g_st().restart_edit == c_int::from(b'A') {
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
            g_st().restart_edit = 0;
        }
}

fn execreg_line_continuation(lines: *mut NvimString, idx: &mut usize) -> Vec<c_char> {
    unsafe {
        let mut cmd_start = *idx;
        assert!(cmd_start > 0);
        let cmd_end = cmd_start;

        let mut buf: Vec<c_char> = Vec::with_capacity(400);

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
        if !(*tmp).data.is_null() && (*tmp).size > 0 {
            buf.extend_from_slice(std::slice::from_raw_parts((*tmp).data, (*tmp).size));
        }
        for j in (cmd_start + 1)..=cmd_end {
            tmp = lines.add(j);
            let mut p = skipwhite((*tmp).data);
            if *p == b'\\' as c_char {
                p = p.add(1);
                let rem = ((*tmp).data as usize + (*tmp).size) - (p as usize);
                if rem > 0 {
                    buf.extend_from_slice(std::slice::from_raw_parts(p, rem));
                }
            }
        }
        buf.push(0);

        *idx = cmd_start;
        buf
    }
}

pub fn do_execreg(mut regname: c_int, colon: c_int, addcr: c_int, silent: c_int) -> c_int {
    unsafe {
        let mut retval = OK;

        if regname == c_int::from(b'@') {
            if reg_st().execreg_lastc == 0 {
                emsg(gettext(c"E748: No previously used register".as_ptr()));
                return FAIL;
            }
            regname = reg_st().execreg_lastc;
        }
        if regname == c_int::from(b'%')
            || regname == c_int::from(b'#')
            || !valid_yank_reg(regname, false)
        {
            emsg_invreg(regname);
            return FAIL;
        }
        reg_st().execreg_lastc = regname;

        if regname == c_int::from(b'_') {
            return OK;
        }

        if regname == c_int::from(b':') {
            let last_cmd_ptr = &raw const g_st().last_cmdline;
            let Some(last_cmd) = (*last_cmd_ptr).as_ref() else {
                emsg(gettext(e_nolastcmd.as_ptr()));
                return FAIL;
            };
            g_st().new_last_cmdline = None;
            let p = vim_strsave_escaped_ext(
                std::ffi::CStr::from_ptr(last_cmd.as_ptr()),
                c"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f",
                Ctrl_V as u8 as c_char,
                false,
            );
            let p_ptr = p.as_ptr().cast_mut();
            if g_st().Visual.active && (crate::memory::cstr_prefix_bytes(p_ptr, 5) == &b"'<,'>"[..5]) {
                retval = put_in_typebuf(p_ptr.add(5), true, true, silent);
            } else {
                retval = put_in_typebuf(p_ptr, true, true, silent);
            }
        } else if regname == c_int::from(b'=') {
            let Some(p) = get_expr_line() else {
                return FAIL;
            };
            retval = put_in_typebuf(p.as_ptr().cast_mut(), true, colon != 0, silent);
        } else if regname == c_int::from(b'.') {
            let Some(p) = get_last_insert_save() else {
                emsg(gettext(e_noinstext.as_ptr()));
                return FAIL;
            };
            retval = put_in_typebuf(p.as_ptr().cast_mut(), false, colon != 0, silent);
        } else {
            let reg = &*get_yank_register(regname, YREG_PASTE);
            if !reg.y_allocated {
                return FAIL;
            }

            let remap = if colon != 0 { REMAP_NONE } else { REMAP_YES };
            let is_silent = silent != 0;

            put_reedit_in_typebuf(silent);
            let mut i = reg.y_array.len();
            while i > 0 {
                i -= 1;
                if (reg.y_type == MotionType::kMTLineWise
                    || i < reg.y_array.len() - 1
                    || addcr != 0)
                    && ins_typebuf(c"\n".as_ptr().cast_mut(), remap, 0, true, is_silent) == FAIL
                {
                    return FAIL;
                }

                let mut str_ = reg.y_array[i].data;
                let cont_vec: Vec<c_char>;
                if colon != 0 && i > 0 {
                    let p = skipwhite(str_);
                    if *p == b'\\' as c_char
                        || (*p == b'"' as c_char
                            && *p.add(1) == b'\\' as c_char
                            && *p.add(2) == b' ' as c_char)
                    {
                        cont_vec =
                            execreg_line_continuation(reg.y_array.as_ptr().cast_mut(), &mut i);
                        str_ = cont_vec.as_ptr().cast_mut();
                    }
                }
                let escaped = vim_strsave_escape_ks(str_);
                retval = ins_typebuf(escaped.as_ptr().cast_mut(), remap, 0, true, is_silent);
                drop(escaped);
                if retval == FAIL {
                    return FAIL;
                }
                if colon != 0
                    && ins_typebuf(c":".as_ptr().cast_mut(), remap, 0, true, is_silent) == FAIL
                {
                    return FAIL;
                }
            }
            g_st().reg_executing = if regname == 0 {
                c_int::from(b'"')
            } else {
                regname
            };
            g_st().pending_end_reg_executing = false;
        }
        retval
    }
}

pub fn insert_reg(regname: c_int, mut reg: *mut yankreg_T, literally_arg: bool) -> c_int {
    unsafe {
        let mut retval = OK;
        let mut owned: Option<Vec<c_char>> = None;
        let literally = literally_arg || is_literal_register(regname);

        os_breakcheck();
        if g_st().got_int {
            return FAIL;
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return FAIL;
        }

        let mut arg: *mut c_char = ptr::null_mut();
        if regname == c_int::from(b'.') {
            retval = stuff_inserted(0, 1, 1);
        } else if get_spec_reg(regname, &raw mut arg, &mut owned, true) {
            if arg.is_null() {
                return FAIL;
            }
            stuffescaped(arg, literally);
        } else {
            if reg.is_null() {
                reg = get_yank_register(regname, YREG_PASTE);
            }
            let reg_ref = &*reg;
            if !reg_ref.y_allocated {
                retval = FAIL;
            } else {
                let put = reg_ref.y_type == MotionType::kMTCharWise
                    && (regname == c_int::from(b'-')
                        || (mc_ins_cascading()
                            && reg_ref.y_array.len() == 1
                            && (((0..=0x7f).contains(&regname)
                                && (regname as u8).is_ascii_alphanumeric())
                                || regname == c_int::from(b'"'))));
                for i in 0..reg_ref.y_array.len() {
                    if put {
                        let mut dir = BACKWARD;
                        if (g_st().State & REPLACE_FLAG) != 0 {
                            if u_save_cursor() == FAIL {
                                return FAIL;
                            }
                            del_chars(mb_charlen(reg_ref.y_array[0].data), 1);
                            let curpos = (**curwin.as_ptr()).w_cursor;
                            if oneright() == FAIL {
                                dir = FORWARD;
                            }
                            (**curwin.as_ptr()).w_cursor = curpos;
                        }

                        redo_append_char(Ctrl_R);
                        redo_append_char(regname);
                        do_put(regname, ptr::null_mut(), dir, 1, PUT_CURSEND);
                    } else {
                        stuffescaped(reg_ref.y_array[i].data, literally);
                        if reg_ref.y_type == MotionType::kMTLineWise
                            || i < reg_ref.y_array.len() - 1
                        {
                            stuffcharReadbuff(c_int::from(b'\n'));
                        }
                    }
                }
            }
        }

        retval
    }
}

pub fn get_spec_reg(
    regname: c_int,
    argp: *mut *mut c_char,
    owned: &mut Option<Vec<c_char>>,
    errmsg: bool,
) -> bool {
    unsafe {
        *argp = ptr::null_mut();
        *owned = None;
        let set_owned = |argp: *mut *mut c_char,
                         owned: &mut Option<Vec<c_char>>,
                         cstr: Option<std::ffi::CString>| {
            if let Some(s) = cstr {
                let mut v: Vec<c_char> = s
                    .into_bytes_with_nul()
                    .into_iter()
                    .map(|b| b as c_char)
                    .collect();
                *argp = v.as_mut_ptr();
                *owned = Some(v);
            }
        };
        match regname {
            x if x == c_int::from(b'%') => {
                if errmsg {
                    check_fname();
                }
                *argp = (*g_st().curbuf).b_fname;
                true
            }
            x if x == c_int::from(b'#') => {
                *argp = getaltfname(errmsg);
                true
            }
            x if x == c_int::from(b'=') => {
                set_owned(argp, owned, get_expr_line());
                true
            }
            x if x == c_int::from(b':') => {
                let last_cmd_ref_ptr = &raw const g_st().last_cmdline;
                let last_cmd_ptr = (*last_cmd_ref_ptr)
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                if last_cmd_ptr.is_null() && errmsg {
                    emsg(gettext(e_nolastcmd.as_ptr()));
                }
                *argp = last_cmd_ptr;
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
                set_owned(argp, owned, get_last_insert_save());
                if (*argp).is_null() && errmsg {
                    emsg(gettext(e_noinstext.as_ptr()));
                }
                true
            }
            Ctrl_F | Ctrl_P => {
                if !errmsg {
                    return false;
                }
                set_owned(
                    argp,
                    owned,
                    file_name_at_cursor(
                        FNAME_MESS | FNAME_HYP | if regname == Ctrl_P { FNAME_EXP } else { 0 },
                        1,
                        None,
                    ),
                );
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
                    None,
                );
                if cnt != 0 {
                    let mut v: Vec<c_char> = Vec::with_capacity(cnt + 1);
                    v.extend_from_slice(std::slice::from_raw_parts(*argp, cnt));
                    v.push(0);
                    *argp = v.as_mut_ptr();
                    *owned = Some(v);
                } else {
                    *argp = ptr::null_mut();
                }
                true
            }
            Ctrl_L => {
                if !errmsg {
                    return false;
                }
                *argp = ml_get_buf((**curwin.as_ptr()).w_buffer, (**curwin.as_ptr()).w_cursor.lnum);
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

pub fn cmdline_paste_reg(regname: c_int, literally_arg: bool, remcr: bool) -> bool {
    unsafe {
        let literally = literally_arg || is_literal_register(regname);

        let reg = &*get_yank_register(regname, YREG_PASTE);
        if !reg.y_allocated {
            return false;
        }

        for i in 0..reg.y_array.len() {
            cmdline_paste_str(reg.y_array[i].data, literally);

            if i < reg.y_array.len() - 1 && !remcr {
                cmdline_paste_str(c"\r".as_ptr(), literally);
            }

            os_breakcheck();
            if g_st().got_int {
                return false;
            }
        }
        true
    }
}

pub fn shift_delete_registers(y_append: bool) {
    unsafe {
        let regs = (&raw mut reg_st().y_regs).cast::<yankreg_T>();
        *regs.add(9) = EMPTY_YANKREG;
        let mut n = 9;
        while n > 1 {
            *regs.add(n) = std::mem::replace(&mut *regs.add(n - 1), EMPTY_YANKREG);
            n -= 1;
        }
        if !y_append {
            reg_st().y_previous = regs.add(1);
        }
    }
}

pub fn clear_registers() {
    unsafe {
        let regs = (&raw mut reg_st().y_regs).cast::<yankreg_T>();
        for i in 0..NUM_REGISTERS {
            *regs.add(i) = EMPTY_YANKREG;
        }
    }
}

fn yank_copy_line(
    reg: *mut yankreg_T,
    bd: &mut block_def,
    y_idx: usize,
    exclude_trailing_space: bool,
) {
    unsafe {
        if exclude_trailing_space {
            bd.endspaces = 0;
        }
        let size = bd.startspaces + bd.endspaces + bd.textlen;
        assert!(size >= 0);
        let mut buf_vec = vec![0 as c_char; (size as usize) + 1];
        let mut pnew = buf_vec.as_mut_ptr();
        ptr::write_bytes(pnew.cast::<u8>(), b' ', bd.startspaces as usize);
        pnew = pnew.add(bd.startspaces as usize);
        ptr::copy(bd.textstart, pnew, bd.textlen as usize);
        pnew = pnew.add(bd.textlen as usize);
        ptr::write_bytes(pnew.cast::<u8>(), b' ', bd.endspaces as usize);
        pnew = pnew.add(bd.endspaces as usize);
        if exclude_trailing_space {
            let mut s = bd.textlen + bd.endspaces;
            while s > 0 && ascii_iswhite(*bd.textstart.add((s - 1) as usize) as u8) {
                s = s - utf_head_off(bd.textstart, bd.textstart.add((s - 1) as usize)) - 1;
                pnew = pnew.sub(1);
            }
        }
        *pnew = NUL;
        let out_size = (pnew as usize) - (buf_vec.as_ptr() as usize);
        (&mut *reg).y_array[y_idx] = cbuf_to_string(buf_vec.as_ptr(), out_size);
    }
}

pub fn op_yank_reg(oap: &mut oparg_T, message: bool, mut reg: *mut yankreg_T, append: bool) {
    unsafe {
        let mut newreg = EMPTY_YANKREG;
        let orig_motion = match oap.motion_type {
            0 => MotionType::kMTCharWise,
            1 => MotionType::kMTLineWise,
            2 => MotionType::kMTBlockWise,
            _ => MotionType::kMTUnknown,
        };
        let mut yank_type = orig_motion;
        let mut yanklines = oap.line_count as usize;
        let mut yankendlnum = oap.end.lnum;
        let mut bd = block_def::default();

        let curr = reg;
        if append && (*reg).y_allocated {
            reg = &raw mut newreg;
        } else {
            *reg = EMPTY_YANKREG;
        }

        if orig_motion == MotionType::kMTCharWise
            && oap.start.col == 0
            && !oap.inclusive
            && (!oap.is_VIsual || *opt_vars().p_sel == b'o' as c_char)
            && oap.end.col == 0
            && yanklines > 1
        {
            yank_type = MotionType::kMTLineWise;
            yankendlnum -= 1;
            yanklines -= 1;
        }

        (*reg).y_type = yank_type;
        (*reg).y_width = 0;
        (*reg).y_array = vec![
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
            yanklines
        ];
        (*reg).y_allocated = true;
        (*reg).additional_data = None;
        (*reg).timestamp = os_realtime() as u64;

        let mut y_idx: usize = 0;
        let mut lnum = oap.start.lnum;

        if yank_type == MotionType::kMTBlockWise {
            (*reg).y_width = oap.end_vcol - oap.start_vcol;
            if (**curwin.as_ptr()).w_curswant == MAXCOL && (*reg).y_width > 0 {
                (*reg).y_width -= 1;
            }
        }

        while lnum <= yankendlnum {
            match (*reg).y_type {
                MotionType::kMTBlockWise => {
                    block_prep(oap, &mut bd, lnum, false);
                    yank_copy_line(reg, &mut bd, y_idx, oap.excl_tr_ws);
                }
                MotionType::kMTLineWise => {
                    (&mut *reg).y_array[y_idx] =
                        cbuf_to_string(ml_get(lnum), ml_get_len(lnum) as usize);
                }
                MotionType::kMTCharWise => {
                    charwise_block_prep(
                        oap.start,
                        oap.end,
                        &mut bd,
                        lnum,
                        oap.inclusive,
                    );
                    let tmp = CStr::from_ptr(bd.textstart).to_bytes().len() as c_int;
                    if tmp < bd.textlen {
                        bd.textlen = tmp;
                    }
                    yank_copy_line(reg, &mut bd, y_idx, false);
                }
                MotionType::kMTUnknown => std::process::abort(),
            }
            lnum += 1;
            y_idx += 1;
        }

        if !ptr::eq(curr, reg) {
            let curr_reg = &mut *curr;
            let new_reg = &mut *reg;
            if yank_type == MotionType::kMTLineWise {
                curr_reg.y_type = MotionType::kMTLineWise;
            }

            if curr_reg.y_type == MotionType::kMTCharWise
                && vim_strchr(opt_vars().p_cpo, kCpoRegappend).is_null()
                && !curr_reg.y_array.is_empty()
                && !new_reg.y_array.is_empty()
            {
                let last_old = *curr_reg.y_array.last().unwrap();
                let first_new = new_reg.y_array[0];
                let mut pnew = vec![0 as c_char; last_old.size + first_new.size + 1];
                ptr::copy_nonoverlapping(last_old.data, pnew.as_mut_ptr(), last_old.size);
                ptr::copy_nonoverlapping(
                    first_new.data,
                    pnew.as_mut_ptr().add(last_old.size),
                    first_new.size + 1,
                );
                let last_slot = curr_reg.y_array.last_mut().unwrap();
                api_clear_string(last_slot);
                *last_slot = cbuf_to_string(pnew.as_ptr(), last_old.size + first_new.size);
                api_clear_string(&mut new_reg.y_array[0]);
                y_idx = 1;
            } else {
                y_idx = 0;
            }
            while y_idx < new_reg.y_array.len() {
                curr_reg.y_array.push(new_reg.y_array[y_idx]);
                y_idx += 1;
            }
            new_reg.y_array.clear();
        }

        if message {
            if yank_type == MotionType::kMTCharWise && yanklines == 1 {
                yanklines = 0;
            }
            if yanklines > opt_vars().p_report as usize {
                let mut namebuf = [0 as c_char; 100];
                if oap.regname == 0 {
                    namebuf[0] = NUL;
                } else {
                    vim_snprintf!(
                        namebuf.as_mut_ptr(),
                        namebuf.len(),
                        gettext(c" into \"%c".as_ptr()),
                        oap.regname,
                    );
                }

                update_topline(*curwin.as_ptr());
                if g_st().must_redraw != 0 {
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

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*g_st().curbuf).b_op_start = oap.start;
            (*g_st().curbuf).b_op_end = oap.end;
            if yank_type == MotionType::kMTLineWise {
                (*g_st().curbuf).b_op_start.col = 0;
                (*g_st().curbuf).b_op_end.col = MAXCOL;
            }
            if yank_type == MotionType::kMTCharWise && !oap.inclusive {
                decl(&mut (*g_st().curbuf).b_op_end);
            }
        }
    }
}

pub fn format_reg_type(
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

fn add_regtype_to_dict(
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

pub fn do_autocmd_textyankpost(oap: &mut oparg_T, reg: &mut yankreg_T) {
    unsafe {
        if reg_st().textyankpost_recursive || !has_event(EVENT_TEXTYANKPOST) {
            return;
        }

        reg_st().textyankpost_recursive = true;

        let mut save_v_event: save_v_event_T = save_v_event_T::new();
        let dict = get_v_event(&mut save_v_event);

        let reg_ref = &*reg;
        let list = tv_list_alloc(reg_ref.y_array.len() as isize);
        for &item in &reg_ref.y_array {
            tv_list_append_string(list, item.data, item.size as isize);
        }
        (*list).lv_lock = VAR_FIXED;
        tv_dict_add_list(dict, c"regcontents".as_ptr(), 11, list);

        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        add_regtype_to_dict(&raw mut *reg, dict, buf.as_mut_ptr(), buf.len());

        buf[0] = oap.regname as u8 as c_char;
        buf[1] = NUL;
        tv_dict_add_str(dict, c"regname".as_ptr(), 7, buf.as_ptr());

        tv_dict_add_bool(
            dict,
            c"inclusive".as_ptr(),
            9,
            if oap.inclusive {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );

        buf[0] = get_op_char(oap.op_type) as u8 as c_char;
        buf[1] = NUL;
        tv_dict_add_str(dict, c"operator".as_ptr(), 8, buf.as_ptr());

        tv_dict_add_bool(
            dict,
            c"visual".as_ptr(),
            6,
            if oap.is_VIsual {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );

        tv_dict_set_keys_readonly(&mut *dict);
        g_st().textlock += 1;
        apply_autocmds(
            EVENT_TEXTYANKPOST,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        g_st().textlock -= 1;
        restore_v_event(&mut *dict, &mut save_v_event);

        reg_st().textyankpost_recursive = false;
    }
}

fn put_do_autocmd(
    regname: c_int,
    reg: *mut yankreg_T,
    insert: Option<&NvimString>,
    post: bool,
    dir: Direction,
) {
    unsafe {
        if reg_st().put_autocmd_recursive || (regname == c_int::from(b'_') && reg.is_null()) {
            return;
        }

        if regname != c_int::from(b'.') && insert.is_none() && reg.is_null() {
            return;
        }

        let mut save_v_event: save_v_event_T = save_v_event_T::new();
        let v_event = get_v_event(&mut save_v_event);

        let list = tv_list_alloc(if !reg.is_null() {
            (*reg).y_array.len() as isize
        } else {
            1
        });

        if regname == c_int::from(b'.') {
            let last_ins = &raw const g_st().last_insert_ga;
            if !(*last_ins).is_empty() {
                tv_list_append_string(list, (*last_ins).as_ptr(), (*last_ins).len() as isize);
            }
        } else if let Some(insert) = insert {
            tv_list_append_string(list, insert.data, insert.size as isize);
        } else {
            for &item in &(&*reg).y_array {
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
            if g_st().Visual.active {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );

        tv_dict_set_keys_readonly(&mut *v_event);

        reg_st().put_autocmd_recursive = true;
        g_st().textlock += 1;
        if post {
            apply_autocmds(
                EVENT_TEXTPUTPOST,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        } else {
            apply_autocmds(
                EVENT_TEXTPUTPRE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
        g_st().textlock -= 1;
        reg_st().put_autocmd_recursive = false;

        restore_v_event(&mut *v_event, &mut save_v_event);
    }
}

pub fn op_yank(oap: &mut oparg_T, message: bool) -> bool {
    unsafe {
        if oap.regname != 0 && !valid_yank_reg(oap.regname, true) {
            beep_flush();
            return false;
        }
        if oap.regname == c_int::from(b'_') {
            return true;
        }

        let reg = get_yank_register(oap.regname, YREG_YANK);
        op_yank_reg(oap, message, reg, is_append_register(oap.regname));
        set_clipboard(oap.regname, reg);
        do_autocmd_textyankpost(oap, &mut *reg);
        true
    }
}

pub fn do_put(
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
        let y_size: usize;
        let mut y_width: c_int = 0;
        let mut vcol: colnr_T = 0;
        let y_array: *mut NvimString;
        let mut expr_y_array: Vec<NvimString> = Vec::new();
        let mut y_allocated = true;
        let mut nr_lines: linenr_T = 0;
        let orig_start = (*g_st().curbuf).b_op_start;
        let orig_end = (*g_st().curbuf).b_op_end;
        let cur_ve_flags = get_ve_flags(*curwin.as_ptr());

        if ins_compl_preinsert_effect() {
            ins_compl_delete(false);
        }

        (*g_st().curbuf).b_op_start = (**curwin.as_ptr()).w_cursor;
        (*g_st().curbuf).b_op_end = (**curwin.as_ptr()).w_cursor;

        if regname == c_int::from(b'.') && reg.is_null() {
            let non_linewise_vis = g_st().Visual.active && g_st().Visual.mode != c_int::from(b'V');

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
                g_st().add_last_insert += 1;
            }

            if (flags & PUT_LINE) != 0 {
                let save_add_last_insert = g_st().add_last_insert;
                g_st().add_last_insert = 0;
                stuffcharReadbuff(K_COMMAND);
                if dir == FORWARD {
                    stuffReadbuffLen(c"put _".as_ptr(), 5);
                } else {
                    stuffReadbuffLen(c"put! _".as_ptr(), 6);
                }
                stuffcharReadbuff(c_int::from(CAR as u8));
                g_st().add_last_insert = save_add_last_insert;
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
                put_do_autocmd(c_int::from(b'.'), ptr::null_mut(), None, false, dir);
            }
            if has_event(EVENT_TEXTPUTPOST) {
                put_do_autocmd(c_int::from(b'.'), ptr::null_mut(), None, true, dir);
            }

            if has_textput_events {
                g_st().add_last_insert -= 1;
                if g_st().add_last_insert == 0 {
                    let last_ins = &raw mut g_st().last_insert_ga;
                    *last_ins = Vec::new();
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
                        (*g_st().curbuf).b_ml.ml_line_count == (**curwin.as_ptr()).w_cursor.lnum && one_past_line;
                    if ve_allows || !(eol || eof) {
                        stuffcharReadbuff(c_int::from(b'l'));
                    }
                }
            } else if (flags & PUT_LINE) != 0 {
                stuffReadbuff(c"g'[".as_ptr());
            }

            if command_start_char == c_int::from(b'a')
                && u_save((**curwin.as_ptr()).w_cursor.lnum, (**curwin.as_ptr()).w_cursor.lnum + 1) == FAIL
            {
                return;
            }

            exec_stuffed(None);
            return;
        }

        let mut insert_string = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        let mut insert_string_owned: Option<Vec<c_char>> = None;
        if reg.is_null()
            && get_spec_reg(
                regname,
                &raw mut insert_string.data,
                &mut insert_string_owned,
                true,
            )
            && insert_string.data.is_null()
        {
            return;
        }

        if (*g_st().curbuf).terminal.is_null()
            && u_save((**curwin.as_ptr()).w_cursor.lnum, (**curwin.as_ptr()).w_cursor.lnum + 1) == FAIL
        {
            return;
        }

        if !insert_string.data.is_null() {
            insert_string.size = CStr::from_ptr(insert_string.data).to_bytes().len();
            y_type = MotionType::kMTCharWise;
            if regname == c_int::from(b'=') {
                let mut ptr_ = insert_string.data;
                let mut ptrlen = insert_string.size;
                while !ptr_.is_null() {
                    let mut tmp = vim_strchr(ptr_, c_int::from(b'\n'));
                    if tmp.is_null() {
                        expr_y_array.push(NvimString {
                            data: ptr_,
                            size: ptrlen,
                        });
                    } else {
                        *tmp = NUL;
                        let item_size = (tmp as usize) - (ptr_ as usize);
                        expr_y_array.push(NvimString {
                            data: ptr_,
                            size: item_size,
                        });
                        ptrlen -= item_size + 1;
                        tmp = tmp.add(1);
                        if *tmp == NUL {
                            y_type = MotionType::kMTLineWise;
                            break;
                        }
                    }
                    ptr_ = tmp;
                }
                y_size = expr_y_array.len();
                y_array = expr_y_array.as_mut_ptr();
            } else {
                y_size = 1;
                y_array = &raw mut insert_string;
            }
            if has_event(EVENT_TEXTPUTPRE) {
                put_do_autocmd(
                    regname,
                    ptr::null_mut(),
                    Some(&insert_string),
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
                put_do_autocmd(regname, reg, None, false, dir);
                reg = save_reg;
            }
            if reg.is_null() {
                reg = get_yank_register(regname, YREG_PASTE);
            }

            let reg_ref = &mut *reg;
            y_type = reg_ref.y_type;
            y_width = reg_ref.y_width;
            y_size = reg_ref.y_array.len();
            y_allocated = reg_ref.y_allocated;
            y_array = reg_ref.y_array.as_mut_ptr();
        }

        'end: {
            if !(*g_st().curbuf).terminal.is_null() {
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

                    let tail_len = plen - ((p as usize) - (p_orig as usize));
                    let mut tail_vec: Vec<c_char> = Vec::with_capacity(tail_len + 1);
                    tail_vec.extend_from_slice(std::slice::from_raw_parts(p, tail_len));
                    tail_vec.push(0);
                    ml_append((**curwin.as_ptr()).w_cursor.lnum, tail_vec.as_mut_ptr(), 0, false);

                    crate::memline::ml_replace_len(
                        (**curwin.as_ptr()).w_cursor.lnum,
                        get_cursor_line_ptr(),
                        split_pos as usize,
                        true,
                    );
                    nr_lines += 1;
                    dir = FORWARD;

                    buf_updates_send_changes(g_st().curbuf, (**curwin.as_ptr()).w_cursor.lnum, 1, 1);
                }
                if (flags & PUT_LINE_FORWARD) != 0 {
                    (**curwin.as_ptr()).w_cursor = (*g_st().curbuf).b_visual.vi_end;
                    dir = FORWARD;
                }
                (*g_st().curbuf).b_op_start = (**curwin.as_ptr()).w_cursor;
                (*g_st().curbuf).b_op_end = (**curwin.as_ptr()).w_cursor;
            }

            if (flags & PUT_LINE) != 0 {
                y_type = MotionType::kMTLineWise;
            }

            if y_size == 0 || !y_allocated {
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
                lnum = (**curwin.as_ptr()).w_cursor.lnum + (y_size as linenr_T) + 1;
                lnum = lnum.min((*g_st().curbuf).b_ml.ml_line_count + 1);
                if u_save((**curwin.as_ptr()).w_cursor.lnum - 1, lnum) == FAIL {
                    break 'end;
                }
            } else if y_type == MotionType::kMTLineWise {
                lnum = (**curwin.as_ptr()).w_cursor.lnum;
                if dir == BACKWARD {
                    hasFolding(*curwin.as_ptr(), lnum, Some(&mut lnum), ptr::null_mut());
                } else {
                    hasFolding(*curwin.as_ptr(), lnum, None, &raw mut lnum);
                }
                if dir == FORWARD {
                    lnum += 1;
                }
                let save_res = if buf_is_empty(&mut *g_st().curbuf) {
                    u_save(0, 2)
                } else {
                    u_save(lnum - 1, lnum)
                };
                if save_res == FAIL {
                    break 'end;
                }
                if dir == FORWARD {
                    (**curwin.as_ptr()).w_cursor.lnum = lnum - 1;
                } else {
                    (**curwin.as_ptr()).w_cursor.lnum = lnum;
                }
                (*g_st().curbuf).b_op_start = (**curwin.as_ptr()).w_cursor;
            } else if u_save_cursor() == FAIL {
                break 'end;
            }

            if cur_ve_flags == kOptVeFlagAll && y_type == MotionType::kMTCharWise {
                if gchar_cursor() == TAB {
                    let viscol = getviscol();
                    let ts = (*g_st().curbuf).b_p_ts;
                    let cond = if dir == FORWARD {
                        tabstop_padding(viscol, ts, &(*g_st().curbuf).b_p_vts_array) != 1
                    } else {
                        (**curwin.as_ptr()).w_cursor.coladd > 0
                    };
                    if cond {
                        coladvance_force(viscol);
                    } else {
                        (**curwin.as_ptr()).w_cursor.coladd = 0;
                    }
                } else if (**curwin.as_ptr()).w_cursor.coladd > 0 || gchar_cursor() == 0 {
                    coladvance_force(getviscol() + c_int::from(dir == FORWARD));
                }
            }

            lnum = (**curwin.as_ptr()).w_cursor.lnum;
            let mut col = (**curwin.as_ptr()).w_cursor.col;

            if y_type == MotionType::kMTBlockWise {
                let mut incr: c_int = 0;
                let mut bd = block_def::default();
                let c = gchar_cursor();
                let mut endcol2: colnr_T = 0;

                if dir == FORWARD && c != 0 {
                    if cur_ve_flags == kOptVeFlagAll {
                        getvcol(
                            &mut **curwin.as_ptr(),
                            &mut (**curwin.as_ptr()).w_cursor,
                            Some(&mut col),
                            None,
                            Some(&mut endcol2),
                            0,
                        );
                    } else {
                        getvcol(
                            &mut **curwin.as_ptr(),
                            &mut (**curwin.as_ptr()).w_cursor,
                            None,
                            None,
                            Some(&mut col),
                            0,
                        );
                    }
                    (**curwin.as_ptr()).w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
                    col += 1;
                } else {
                    getvcol(
                        &mut **curwin.as_ptr(),
                        &mut (**curwin.as_ptr()).w_cursor,
                        Some(&mut col),
                        None,
                        Some(&mut endcol2),
                        0,
                    );
                }

                col += (**curwin.as_ptr()).w_cursor.coladd;
                if cur_ve_flags == kOptVeFlagAll
                    && ((**curwin.as_ptr()).w_cursor.coladd > 0 || endcol2 == (**curwin.as_ptr()).w_cursor.col)
                {
                    if dir == FORWARD && c == 0 {
                        col += 1;
                    }
                    if dir != FORWARD && c != 0 && (**curwin.as_ptr()).w_cursor.coladd > 0 {
                        (**curwin.as_ptr()).w_cursor.col += 1;
                    }
                    if c == TAB {
                        if dir == BACKWARD && (**curwin.as_ptr()).w_cursor.col != 0 {
                            (**curwin.as_ptr()).w_cursor.col -= 1;
                        }
                        if dir == FORWARD && col - 1 == endcol2 {
                            (**curwin.as_ptr()).w_cursor.col += 1;
                        }
                    }
                }
                (**curwin.as_ptr()).w_cursor.coladd = 0;
                bd.textcol = 0;

                for i in 0..y_size {
                    let mut spaces: c_int = 0;
                    let mut lines_appended: c_int = 0;

                    bd.startspaces = 0;
                    bd.endspaces = 0;
                    let mut delcount: c_int = 0;

                    if (**curwin.as_ptr()).w_cursor.lnum > (*g_st().curbuf).b_ml.ml_line_count {
                        if ml_append(
                            (*g_st().curbuf).b_ml.ml_line_count,
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

                    let mut csarg = CharsizeArg::new();
                    let mut cstype: CSType =
                        init_charsize_arg(&mut csarg, *curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum, oldp);
                    let mut ci = utf_ptr2StrCharInfo(oldp);
                    vcol = 0;
                    while vcol < col && *ci.ptr != NUL {
                        let cli = utf_ClusterInfo(ci);
                        incr = win_charsize(
                            cstype,
                            vcol,
                            ci.ptr,
                            ci.chr.value,
                            &mut csarg,
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
                            init_charsize_arg(&mut csarg, *curwin.as_ptr(), 0, (*y_array.add(i)).data);
                        ci = utf_ptr2StrCharInfo((*y_array.add(i)).data);
                        while *ci.ptr != NUL {
                            let cli = utf_ClusterInfo(ci);
                            spaces -= win_charsize(
                                cstype,
                                0,
                                ci.ptr,
                                ci.chr.value,
                                &mut csarg,
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
                    let mut newp_vec = vec![0 as c_char; totlen + (oldlen as usize) + 1];
                    let newp = newp_vec.as_mut_ptr();

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
                    ml_replace((**curwin.as_ptr()).w_cursor.lnum, newp, true);
                    drop(newp_vec);
                    extmark_splice_cols(
                        g_st().curbuf,
                        (**curwin.as_ptr()).w_cursor.lnum - 1,
                        bd.textcol,
                        delcount,
                        (totlen as c_int) + lines_appended,
                        kExtmarkUndo,
                    );

                    (**curwin.as_ptr()).w_cursor.lnum += 1;
                    if i == 0 {
                        (**curwin.as_ptr()).w_cursor.col += bd.startspaces;
                    }
                }

                changed_lines(
                    g_st().curbuf,
                    lnum,
                    0,
                    (*g_st().curbuf).b_op_start.lnum + (y_size as linenr_T) - nr_lines,
                    nr_lines,
                    true,
                );

                (*g_st().curbuf).b_op_start = (**curwin.as_ptr()).w_cursor;
                (*g_st().curbuf).b_op_start.lnum = lnum;

                (*g_st().curbuf).b_op_end.lnum = (**curwin.as_ptr()).w_cursor.lnum - 1;
                (*g_st().curbuf).b_op_end.col = (bd.textcol + (totlen as colnr_T) - 1).max(0);
                (*g_st().curbuf).b_op_end.coladd = 0;
                if (flags & PUT_CURSEND) != 0 {
                    (**curwin.as_ptr()).w_cursor = (*g_st().curbuf).b_op_end;
                    (**curwin.as_ptr()).w_cursor.col += 1;

                    let len = get_cursor_line_len();
                    (**curwin.as_ptr()).w_cursor.col = (**curwin.as_ptr()).w_cursor.col.min(len);
                } else {
                    (**curwin.as_ptr()).w_cursor.lnum = lnum;
                }
            } else {
                let yanklen = (*y_array).size as c_int;

                if y_type == MotionType::kMTCharWise {
                    if dir == FORWARD && gchar_cursor() != 0 {
                        let bytelen = utfc_ptr2len(get_cursor_pos_ptr());
                        col += bytelen;
                        if yanklen != 0 {
                            (**curwin.as_ptr()).w_cursor.col += bytelen;
                            (*g_st().curbuf).b_op_end.col += bytelen;
                        }
                    }
                    (*g_st().curbuf).b_op_start = (**curwin.as_ptr()).w_cursor;
                } else if dir == BACKWARD {
                    lnum -= 1;
                }
                let new_cursor = (**curwin.as_ptr()).w_cursor;

                if y_type == MotionType::kMTCharWise && y_size == 1 {
                    let mut end_lnum: linenr_T = 0;
                    let start_lnum = lnum;
                    let mut first_byte_off: c_int = 0;

                    if g_st().Visual.active {
                        end_lnum = (*g_st().curbuf)
                            .b_visual
                            .vi_end
                            .lnum
                            .max((*g_st().curbuf).b_visual.vi_start.lnum);
                        if end_lnum > start_lnum {
                            let mut pos = pos_T {
                                lnum,
                                col,
                                coladd: 0,
                            };
                            getvcol(
                                &mut **curwin.as_ptr(),
                                &mut pos,
                                None,
                                Some(&mut vcol),
                                None,
                                0,
                            );
                        }
                    }

                    if count == 0 || yanklen == 0 {
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
                                if getvpos(*curwin.as_ptr(), &mut pos, vcol) == OK {
                                    col = pos.col;
                                } else {
                                    col = MAXCOL;
                                }
                            }
                            if g_st().Visual.active && col > oldlen {
                                lnum += 1;
                                if !(g_st().Visual.active && lnum <= end_lnum) {
                                    break;
                                }
                                continue;
                            }
                            let mut newp_vec = vec![0 as c_char; totlen + (oldlen as usize) + 1];
                            let newp = newp_vec.as_mut_ptr();
                            ptr::copy(oldp, newp, col as usize);
                            let mut ptr_ = newp.add(col as usize);
                            for _ in 0..(count as usize) {
                                ptr::copy((*y_array).data, ptr_, yanklen as usize);
                                ptr_ = ptr_.add(yanklen as usize);
                            }
                            ptr::copy(oldp.add(col as usize), ptr_, ((oldlen - col) as usize) + 1);
                            ml_replace(lnum, newp, true);

                            first_byte_off = utf_head_off(newp, ptr_.sub(1));
                            drop(newp_vec);

                            if lnum == (**curwin.as_ptr()).w_cursor.lnum {
                                changed_cline_bef_curs(&mut **curwin.as_ptr());
                                invalidate_botline_win(&mut **curwin.as_ptr());
                                (**curwin.as_ptr()).w_cursor.col += (totlen - 1) as colnr_T;
                            }
                            changed_bytes(lnum, col);
                            extmark_splice_cols(
                                g_st().curbuf,
                                lnum - 1,
                                col,
                                0,
                                totlen as c_int,
                                kExtmarkUndo,
                            );
                            if g_st().Visual.active {
                                lnum += 1;
                            }
                            if !(g_st().Visual.active && lnum <= end_lnum) {
                                break;
                            }
                        }
                    }

                    (*g_st().curbuf).b_op_end = (**curwin.as_ptr()).w_cursor;
                    (*g_st().curbuf).b_op_end.col -= first_byte_off;

                    if totlen != 0 && (g_st().restart_edit != 0 || (flags & PUT_CURSEND) != 0) {
                        (**curwin.as_ptr()).w_cursor.col += 1;
                    } else {
                        (**curwin.as_ptr()).w_cursor.col -= first_byte_off;
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
                                let mut append_vec: Vec<c_char> = vec![0; ptrlen + totlen + 1];
                                ptr::copy_nonoverlapping(
                                    (*y_array.add(y_size - 1)).data,
                                    append_vec.as_mut_ptr(),
                                    totlen,
                                );
                                ptr::copy_nonoverlapping(
                                    ptr_,
                                    append_vec.as_mut_ptr().add(totlen),
                                    ptrlen + 1,
                                );
                                ml_append(lnum, append_vec.as_mut_ptr(), 0, false);
                                new_lnum += 1;
                                drop(append_vec);

                                let oldp = ml_get(lnum);
                                let mut newp_vec =
                                    vec![0 as c_char; (col as usize) + (yanklen as usize) + 1];
                                let newp = newp_vec.as_mut_ptr();
                                ptr::copy(oldp, newp, col as usize);
                                ptr::copy(
                                    (*y_array).data,
                                    newp.add(col as usize),
                                    (yanklen as usize) + 1,
                                );
                                ml_replace(lnum, newp, true);
                                drop(newp_vec);

                                (**curwin.as_ptr()).w_cursor.lnum = lnum;
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
                                    let old_pos = (**curwin.as_ptr()).w_cursor;
                                    (**curwin.as_ptr()).w_cursor.lnum = lnum;
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
                                    (**curwin.as_ptr()).w_cursor = old_pos;
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
                                    g_st().curbuf,
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
                                    g_st().curbuf,
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
                        (*g_st().curbuf).b_op_start.col = 0;
                        if dir == FORWARD {
                            (*g_st().curbuf).b_op_start.lnum += 1;
                        }
                    }

                    let kind = if y_type == MotionType::kMTLineWise && (flags & PUT_LINE_SPLIT) == 0
                    {
                        kExtmarkUndo
                    } else {
                        kExtmarkNOOP
                    };
                    mark_adjust(
                        (*g_st().curbuf).b_op_start.lnum
                            + linenr_T::from(y_type == MotionType::kMTCharWise),
                        MAXLNUM,
                        nr_lines,
                        0,
                        kind,
                    );

                    if y_type == MotionType::kMTCharWise {
                        changed_lines(
                            g_st().curbuf,
                            (**curwin.as_ptr()).w_cursor.lnum,
                            col,
                            (**curwin.as_ptr()).w_cursor.lnum + 1,
                            nr_lines,
                            true,
                        );
                    } else {
                        changed_lines(
                            g_st().curbuf,
                            (*g_st().curbuf).b_op_start.lnum,
                            0,
                            (*g_st().curbuf).b_op_start.lnum,
                            nr_lines,
                            true,
                        );
                    }

                    (*g_st().curbuf).b_op_end.lnum = new_lnum;
                    let last_item = *y_array.add(y_size - 1);
                    col = ((last_item.size as colnr_T) - lendiff).max(0);
                    if col > 1 {
                        (*g_st().curbuf).b_op_end.col = col - 1;
                        if last_item.size > 0 {
                            (*g_st().curbuf).b_op_end.col -= utf_head_off(
                                last_item.data,
                                last_item.data.add(last_item.size - 1),
                            );
                        }
                    } else {
                        (*g_st().curbuf).b_op_end.col = 0;
                    }

                    if (flags & PUT_CURSLINE) != 0 {
                        (**curwin.as_ptr()).w_cursor.lnum = lnum;
                        beginline(BL_WHITE | BL_FIX);
                    } else if (flags & PUT_CURSEND) != 0 {
                        if y_type == MotionType::kMTLineWise {
                            if lnum >= (*g_st().curbuf).b_ml.ml_line_count {
                                (**curwin.as_ptr()).w_cursor.lnum = (*g_st().curbuf).b_ml.ml_line_count;
                            } else {
                                (**curwin.as_ptr()).w_cursor.lnum = lnum + 1;
                            }
                            (**curwin.as_ptr()).w_cursor.col = 0;
                        } else {
                            (**curwin.as_ptr()).w_cursor.lnum = new_lnum;
                            (**curwin.as_ptr()).w_cursor.col = col;
                            (*g_st().curbuf).b_op_end = (**curwin.as_ptr()).w_cursor;
                            if col > 1 {
                                (*g_st().curbuf).b_op_end.col = col - 1;
                            }
                        }
                    } else if y_type == MotionType::kMTLineWise {
                        (**curwin.as_ptr()).w_cursor.col = 0;
                        if dir == FORWARD {
                            (**curwin.as_ptr()).w_cursor.lnum += 1;
                        }
                        beginline(BL_WHITE | BL_FIX);
                    } else {
                        (**curwin.as_ptr()).w_cursor = new_cursor;
                    }
                }
            }

            msgmore(nr_lines);
            (**curwin.as_ptr()).w_set_curswant = true;

            let len = get_cursor_line_len();
            if (**curwin.as_ptr()).w_cursor.col > len {
                if cur_ve_flags == kOptVeFlagAll {
                    (**curwin.as_ptr()).w_cursor.coladd = (**curwin.as_ptr()).w_cursor.col - len;
                }
                (**curwin.as_ptr()).w_cursor.col = len;
            }
        }

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            (*g_st().curbuf).b_op_start = orig_start;
            (*g_st().curbuf).b_op_end = orig_end;
        }

        if has_event(EVENT_TEXTPUTPOST) {
            if insert_string.data.is_null() {
                put_do_autocmd(regname, reg, None, true, dir);
            } else {
                put_do_autocmd(
                    regname,
                    ptr::null_mut(),
                    Some(&insert_string),
                    true,
                    dir,
                );
            }
        }

        drop(insert_string_owned);

        if (*g_st().curbuf).terminal.is_null() {
            g_st().Visual.active = false;
        }

        adjust_cursor_eol();
    }
}

fn dis_msg(mut p: *const c_char, skip_esc: bool) {
    unsafe {
        let mut n = g_st().Columns - 6;
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

pub fn ex_display(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;
        if !arg.is_null() && *arg == NUL {
            arg = ptr::null_mut();
        }
        let hl_id = HLF_8;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        g_st().msg_ext_skip_flush = true;
        msg_puts_title(gettext(c"\nType Name Content".as_ptr()));

        let regs = (&raw mut reg_st().y_regs).cast::<yankreg_T>();
        let mut i: c_int = -1;
        while i < (NUM_REGISTERS as c_int) && !g_st().got_int {
            let name = get_register_name(i);
            if !arg.is_null() && vim_strchr(arg, name).is_null() {
                i += 1;
                continue;
            }

            let type_ch = match get_reg_type(name, None) {
                MotionType::kMTLineWise => c_int::from(b'l'),
                MotionType::kMTCharWise => c_int::from(b'c'),
                _ => c_int::from(b'b'),
            };

            let mut yb = if i == -1 {
                if !reg_st().y_previous.is_null() {
                    reg_st().y_previous
                } else {
                    regs
                }
            } else {
                regs.add(i as usize)
            };

            get_clipboard(name, &raw mut yb, true);

            if name == mb_tolower(g_st().redir_reg)
                || (g_st().redir_reg == c_int::from(b'"') && ptr::eq(yb, reg_st().y_previous))
            {
                i += 1;
                continue;
            }

            let yb_ref = &*yb;
            if yb_ref.y_allocated {
                let mut do_show = false;
                for item in &yb_ref.y_array {
                    if !message_filtered(item.data) {
                        do_show = true;
                        break;
                    }
                }

                if do_show || yb_ref.y_array.is_empty() {
                    msg_putchar(c_int::from(b'\n'));
                    msg_puts(c"  ".as_ptr());
                    msg_putchar(type_ch);
                    msg_puts(c"  ".as_ptr());
                    msg_putchar(c_int::from(b'"'));
                    msg_putchar(name);
                    msg_puts(c"   ".as_ptr());

                    let mut n = g_st().Columns - 11;
                    let mut j: usize = 0;
                    while j < yb_ref.y_array.len() && n > 1 {
                        if j != 0 {
                            msg_puts_hl(c"^J".as_ptr(), hl_id, false);
                            n -= 2;
                        }
                        let mut p: *const c_char = yb_ref.y_array[j].data;
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
                    if n > 1 && yb_ref.y_type == MotionType::kMTLineWise {
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
            && !g_st().got_int
            && !message_filtered(p)
        {
            msg_puts(c"\n  c  \".   ".as_ptr());
            dis_msg(p, true);
        }

        let last_cmd_ptr = &raw const g_st().last_cmdline;
        if let Some(last_cmd) = (*last_cmd_ptr).as_ref()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b':')).is_null())
            && !g_st().got_int
            && !message_filtered(last_cmd.as_ptr())
        {
            msg_puts(c"\n  c  \":   ".as_ptr());
            dis_msg(last_cmd.as_ptr(), false);
        }

        if !(*g_st().curbuf).b_fname.is_null()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b'%')).is_null())
            && !g_st().got_int
            && !message_filtered((*g_st().curbuf).b_fname)
        {
            msg_puts(c"\n  c  \"%   ".as_ptr());
            dis_msg((*g_st().curbuf).b_fname, false);
        }

        if (arg.is_null() || !vim_strchr(arg, c_int::from(b'#')).is_null()) && !g_st().got_int {
            let mut fname: *mut c_char = ptr::null_mut();
            let mut dummy: linenr_T = 0;
            if buflist_name_nr(0, &raw mut fname, &mut dummy) != FAIL
                && !message_filtered(fname)
            {
                msg_puts(c"\n  c  \"#   ".as_ptr());
                dis_msg(fname, false);
            }
        }

        if !last_search_pat().is_null()
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b'/')).is_null())
            && !g_st().got_int
            && !message_filtered(last_search_pat())
        {
            msg_puts(c"\n  c  \"/   ".as_ptr());
            dis_msg(last_search_pat(), false);
        }

        if let Some(el) = &reg_st().expr_line
            && (arg.is_null() || !vim_strchr(arg, c_int::from(b'=')).is_null())
            && !g_st().got_int
            && !message_filtered(el.as_ptr())
        {
            msg_puts(c"\n  c  \"=   ".as_ptr());
            dis_msg(el.as_ptr(), false);
        }
        g_st().msg_ext_skip_flush = false;
    }
}

pub fn get_reg_type(regname: c_int, reg_width: Option<&mut colnr_T>) -> MotionType {
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

        let reg = &*get_yank_register(regname, YREG_PASTE);
        if reg.y_allocated {
            if reg.y_type == MotionType::kMTBlockWise
                && let Some(reg_width) = reg_width
            {
                *reg_width = reg.y_width;
            }
            return reg.y_type;
        }
        MotionType::kMTUnknown
    }
}

fn get_reg_wrap_one_line_list(s: Option<std::ffi::CString>) -> *mut list_T {
    unsafe {
        let Some(s) = s else {
            return ptr::null_mut();
        };
        let list = tv_list_alloc(1);
        tv_list_append_string(list, s.as_ptr(), -1);
        list
    }
}

pub fn get_reg_contents_str(mut regname: c_int, flags: c_int) -> Option<std::ffi::CString> {
    unsafe {
        if regname == c_int::from(b'=') {
            if (flags & kGRegNoExpr) != 0 {
                return None;
            }
            if (flags & kGRegExprSrc) != 0 {
                return get_expr_line_src();
            }
            return get_expr_line();
        }

        if regname == c_int::from(b'@') {
            regname = c_int::from(b'"');
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return None;
        }

        let mut retval: *mut c_char = ptr::null_mut();
        let mut owned: Option<Vec<c_char>> = None;
        if get_spec_reg(regname, &raw mut retval, &mut owned, false) {
            if retval.is_null() {
                return None;
            }
            return Some(CStr::from_ptr(retval).to_owned());
        }

        let reg = &*get_yank_register(regname, YREG_PUT);
        if !reg.y_allocated {
            return None;
        }

        let mut len: usize = 0;
        for (i, &item) in reg.y_array.iter().enumerate() {
            len += item.size;
            if reg.y_type == MotionType::kMTLineWise || i < reg.y_array.len() - 1 {
                len += 1;
            }
        }

        let mut out: Vec<u8> = Vec::with_capacity(len);
        for (i, &item) in reg.y_array.iter().enumerate() {
            if item.size > 0 {
                out.extend_from_slice(std::slice::from_raw_parts(item.data.cast::<u8>(), item.size));
            }
            if reg.y_type == MotionType::kMTLineWise || i < reg.y_array.len() - 1 {
                out.push(b'\n');
            }
        }

        Some(std::ffi::CString::new(out).unwrap_or_default())
    }
}

pub fn get_reg_contents_list(mut regname: c_int, flags: c_int) -> *mut list_T {
    unsafe {
        if regname == c_int::from(b'=') {
            if (flags & kGRegNoExpr) != 0 {
                return ptr::null_mut();
            }
            if (flags & kGRegExprSrc) != 0 {
                return get_reg_wrap_one_line_list(get_expr_line_src());
            }
            return get_reg_wrap_one_line_list(get_expr_line());
        }

        if regname == c_int::from(b'@') {
            regname = c_int::from(b'"');
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return ptr::null_mut();
        }

        let mut retval: *mut c_char = ptr::null_mut();
        let mut owned: Option<Vec<c_char>> = None;
        if get_spec_reg(regname, &raw mut retval, &mut owned, false) {
            if retval.is_null() {
                return ptr::null_mut();
            }
            return get_reg_wrap_one_line_list(Some(CStr::from_ptr(retval).to_owned()));
        }

        let reg = &*get_yank_register(regname, YREG_PUT);
        if !reg.y_allocated {
            return ptr::null_mut();
        }

        let list = tv_list_alloc(reg.y_array.len() as isize);
        for &item in &reg.y_array {
            tv_list_append_string(list, item.data, item.size as isize);
        }
        list
    }
}

fn init_write_reg(
    name: c_int,
    old_y_previous: *mut *mut yankreg_T,
    must_append: bool,
) -> *mut yankreg_T {
    unsafe {
        if !valid_yank_reg(name, true) {
            emsg_invreg(name);
            return ptr::null_mut();
        }

        *old_y_previous = reg_st().y_previous;

        let reg = get_yank_register(name, YREG_YANK);
        if !is_append_register(name) && !must_append {
            *reg = EMPTY_YANKREG;
        }
        reg
    }
}

fn str_to_reg(
    y_ptr: &mut yankreg_T,
    mut yank_type: MotionType,
    str_: *const c_char,
    len: usize,
    blocklen: colnr_T,
    str_list: bool,
) {
    unsafe {
        let y_reg = &mut *y_ptr;
        if !y_reg.y_allocated {
            y_reg.y_array.clear();
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
            newlines = { let _d = (str_).cast::<u8>(); let _sz = len; if _sz == 0 { 0 } else { std::slice::from_raw_parts(_d, _sz).iter().filter(|&&_b| _b == (b'\n' as c_char) as u8).count() } };
            if yank_type == MotionType::kMTCharWise
                || len == 0
                || *str_.add(len - 1) != b'\n' as c_char
            {
                extraline = 1;
                newlines += 1;
            }
            if !y_reg.y_array.is_empty() && y_reg.y_type == MotionType::kMTCharWise {
                append = true;
                newlines -= 1;
            }
        }

        if y_reg.y_array.len() + newlines == 0 {
            y_reg.y_array.clear();
            y_reg.y_allocated = false;
            return;
        }

        y_reg.y_array.reserve(newlines);
        y_reg.y_allocated = true;

        let mut maxlen: usize = 0;

        if str_list {
            let mut ss = str_.cast::<*mut c_char>();
            while !(*ss).is_null() {
                y_reg.y_array.push(cstr_to_string(*ss));
                if yank_type == MotionType::kMTBlockWise {
                    let charlen = vim_strsize(*ss) as usize;
                    maxlen = maxlen.max(charlen);
                }
                ss = ss.add(1);
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
                    y_reg.y_array.last().unwrap().size
                } else {
                    0
                };
                let s_len = extra + line_len;
                let mut s_vec = vec![0 as c_char; s_len + 1];
                if extra > 0 {
                    ptr::copy_nonoverlapping(
                        y_reg.y_array.last().unwrap().data,
                        s_vec.as_mut_ptr(),
                        extra,
                    );
                }
                if line_len > 0 {
                    ptr::copy_nonoverlapping(start, s_vec.as_mut_ptr().add(extra), line_len);
                }
                { let _d = (s_vec.as_mut_ptr()).cast::<u8>(); let _sz = s_len; if _sz > 0 { let _c = (NUL) as u8; let _x = (b'\n' as c_char) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
                let new_str = cbuf_to_string(s_vec.as_ptr(), s_len);

                if append {
                    let last = y_reg.y_array.last_mut().unwrap();
                    api_clear_string(last);
                    *last = new_str;
                    append = false;
                } else {
                    y_reg.y_array.push(new_str);
                }

                start = start.add(line_len + 1);
            }
        }
        y_reg.y_type = yank_type;
        y_reg.additional_data = None;
        y_reg.timestamp = os_realtime() as u64;
        if yank_type == MotionType::kMTBlockWise {
            y_reg.y_width = if blocklen == -1 {
                (maxlen as colnr_T) - 1
            } else {
                blocklen
            };
        } else {
            y_reg.y_width = 0;
        }
    }
}

fn finish_write_reg(name: c_int, reg: *mut yankreg_T, old_y_previous: *mut yankreg_T) {
    set_clipboard(name, reg);
        if name != c_int::from(b'"') {
            reg_st().y_previous = old_y_previous;
        }
}

pub fn write_reg_contents(name: c_int, str_: *const c_char, len: isize, must_append: c_int) {
    write_reg_contents_ex(name, str_, len, must_append != 0, MotionType::kMTUnknown, 0);
}

pub fn write_reg_contents_lst(
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

        str_to_reg(&mut *reg, yank_type, strings.cast::<c_char>(), 0, block_len, true);
        finish_write_reg(name, reg, old_y_previous);
    }
}

pub fn write_reg_contents_ex(
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
                (**curwin.as_ptr()).w_alt_fnum = 0;
                return;
            }

            let buf = if (*str_ as u8).is_ascii_digit() {
                let num = crate::charset::parse_i64_prefix(std::ffi::CStr::from_ptr(str_).to_bytes()) as c_int;
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
            (**curwin.as_ptr()).w_alt_fnum = (*buf).b_fnum;
            return;
        }

        if name == c_int::from(b'=') {
            let mut out: Vec<u8> = Vec::new();
            if must_append
                && let Some(el) = &reg_st().expr_line
            {
                out.extend_from_slice(el.as_bytes());
            }
            let slice = std::slice::from_raw_parts(str_.cast::<u8>(), len as usize);
            let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
            out.extend_from_slice(&slice[..end]);
            reg_st().expr_line = Some(std::ffi::CString::new(out).unwrap_or_default());
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
        str_to_reg(&mut *reg, yank_type, str_, len as usize, block_len, false);
        finish_write_reg(name, reg, old_y_previous);
    }
}

pub fn prepare_yankreg_from_object(
    reg: &mut yankreg_T,
    regtype: NvimString,
    _lines: usize,
) -> bool {
    unsafe {
        let type_ch = if !regtype.data.is_null() {
            *regtype.data as u8
        } else {
            0
        };

        reg.y_type = match type_ch {
            0 => MotionType::kMTUnknown,
            b'v' | b'c' => MotionType::kMTCharWise,
            b'V' | b'l' => MotionType::kMTLineWise,
            b'b' | 0x16 => MotionType::kMTBlockWise,
            _ => return false,
        };

        reg.y_width = 0;
        if regtype.size > 1 {
            if reg.y_type != MotionType::kMTBlockWise {
                return false;
            }
            if !(*regtype.data.add(1) as u8).is_ascii_digit() {
                return false;
            }
            let mut p = regtype.data.add(1);
            reg.y_width = getdigits_int(&raw mut p, false, 1) - 1;
            if regtype.size > (p as usize) - (regtype.data as usize) {
                return false;
            }
        }

        reg.additional_data = None;
        reg.timestamp = 0;
        true
    }
}

pub fn finish_yankreg_from_object(reg: &mut yankreg_T, clipboard_adjust: bool) {
    let reg_ref = &mut *reg;
        if reg_ref.y_array.last().is_some_and(|last| last.size == 0) {
            if reg_ref.y_type != MotionType::kMTCharWise {
                if reg_ref.y_type == MotionType::kMTUnknown || clipboard_adjust {
                    reg_ref.y_array.pop();
                }
                if reg_ref.y_type == MotionType::kMTUnknown {
                    reg_ref.y_type = MotionType::kMTLineWise;
                }
            }
        } else if reg_ref.y_type == MotionType::kMTUnknown {
            reg_ref.y_type = MotionType::kMTCharWise;
        }

        update_yankreg_width(&raw mut *reg);
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
