//! Functions for managing registers (`src/nvim/register.c`).

use crate::api::private::helpers::{api_clear_string, cbuf_to_string};
use crate::api::private::validate::NvimString;
use crate::autocmd::{
    EVENT_RECORDINGENTER, EVENT_RECORDINGLEAVE, EVENT_TEXTPUTPOST, EVENT_TEXTPUTPRE,
    EVENT_TEXTYANKPOST, HLF_8, apply_autocmds, has_event,
};
use crate::buffer::{
    K_UIMESSAGES, MAXLNUM, VAR_FIXED, buf_is_empty, buflist_findnr, buflist_findpat,
    buflist_name_nr, getaltfname,
};
use crate::buffer_updates::buf_updates_send_changes;
use crate::change::{K_OPT_VE_FLAG_ONEMORE, SIN_NOMARK, changed_bytes, changed_lines, del_chars};
use crate::charset::{NL, NUL, ascii_iswhite, getdigits_int, skipwhite, transchar, vim_strsize};
use crate::clipboard::{adjust_clipboard_name, get_clipboard, set_clipboard};
use crate::cmdexpand::CTRL_P;
use crate::cursor::{
    Colnr, CursorPos, K_OPT_VE_FLAG_ALL, Linenr, coladvance_force, gchar_cursor,
    get_cursor_line_len, get_cursor_line_ptr, get_cursor_pos_len, get_cursor_pos_ptr, getviscol,
    getvpos,
};
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};
use crate::drawscreen::{showmode, update_screen};
use crate::errors::*;
use crate::eval::funcs::RE_SEARCH;
use crate::eval::typval::{
    TvDict, TvList, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_str, tv_dict_add_str_len,
    tv_dict_set_keys_readonly, tv_list_alloc, tv_list_append_string,
};
use crate::eval::{ESC, eval_to_string, get_v_event, restore_v_event};
use crate::ex_cmds::{BL_FIX, BL_WHITE};
use crate::ex_cmds2::check_fname;
use crate::ex_docmd::{CAR, CMOD_LOCKMARKS, Exarg};
use crate::ex_getln::{
    CTRL_A, CTRL_F_CONST, CTRL_L_CONST, CTRL_R_CONST, CTRL_W_CONST, cmdline_paste_str, getcmdline,
};
use crate::extmark::{Bcount, K_EXTMARK_NOOP, K_EXTMARK_UNDO, extmark_splice, extmark_splice_cols};
use crate::file_search::{FNAME_EXP, FNAME_HYP, FNAME_MESS, file_name_at_cursor};
use crate::fold::has_folding;
use crate::globals::*;
use crate::indent::{get_indent, preprocs_left, set_indent, tabstop_padding};
use crate::input::{
    beep_flush, get_recorded, ins_typebuf, redo_append_char, stuff_readbuff, stuff_readbuff_len,
    stuffchar_readbuff, stuffescaped,
};
use crate::insert::{beginline, get_last_insert, get_last_insert_save, oneright, stuff_inserted};
use crate::insexpand::{ins_compl_delete, ins_compl_preinsert_effect};
use crate::keycodes::{
    K_COMMAND, K_SPECIAL, KE_COMMAND, KS_EXTRA, TAB, vim_strsave_escape_ks, vim_unescape_ks,
};
use crate::mapping::{REMAP_NONE, REMAP_YES};
use crate::mark::mark_adjust;
use crate::mbyte::{
    mb_charlen, mb_string2cells_len, mb_tolower, ptr2cells, utf_cluster_info, utf_head_off,
    utf_ptr2_str_char_info, utfc_ptr2len,
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
use crate::regexp::{CTRL_V_CONST, MAXCOL, NUMBUFLEN};
use crate::search::{BACKWARD, Direction, FORWARD, last_search_pat, set_last_search_pat};
use crate::state::{REPLACE_FLAG, SaveVEvent};
use crate::strings::{
    gettext, ngettext, vim_snprintf, vim_snprintf_safelen, vim_strchr, vim_strsave_escaped_ext,
};
use crate::terminal::terminal_paste;
use crate::textformat::Oparg;
use crate::ui::ui_has;
use crate::undo::{u_save, u_save_cursor};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionType {
    MtUnknown = -1,
    MtCharWise = 0,
    MtLineWise = 1,
    MtBlockWise = 2,
}

#[derive(Clone, Debug)]
pub struct Yankreg {
    pub y_array: Vec<NvimString>,
    pub y_allocated: bool,
    pub y_type: MotionType,
    pub y_width: c_int,
    pub timestamp: u64,
    pub additional_data: Option<crate::search::AdditionalData>,
}

impl Drop for Yankreg {
    fn drop(&mut self) {
        for item in self.y_array.iter_mut().rev() {
            api_clear_string(item);
        }
    }
}

const CTRL_U_CONST: u8 = 21;

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

pub const K_GREG_NO_EXPR: c_int = 1;
pub const K_GREG_EXPR_SRC: c_int = 2;

pub const YREG_PASTE: c_int = 0;
pub const YREG_YANK: c_int = 1;
pub const YREG_PUT: c_int = 2;

const K_CPO_REGAPPEND: c_int = b'>' as c_int;

const E_REGISTER_CHAR_CANNOT_CONTAIN_MULTIPLE_LINES: &CStr =
    c"E883: Register '%c' cannot contain multiple lines";

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BlockDef {
    pub startspaces: c_int,
    pub endspaces: c_int,
    pub textlen: c_int,
    pub textstart: *mut c_char,
    pub textcol: Colnr,
    pub start_vcol: Colnr,
    pub end_vcol: Colnr,
    pub is_short: c_int,
    pub is_max: c_int,
    pub is_one_char: c_int,
    pub pre_whitesp: c_int,
    pub pre_whitesp_c: c_int,
    pub end_char_vcols: Colnr,
    pub start_char_vcols: Colnr,
}

impl BlockDef {
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
            is_max: 0,
            is_one_char: 0,
            pre_whitesp: 0,
            pre_whitesp_c: 0,
            end_char_vcols: 0,
            start_char_vcols: 0,
        }
    }
}

pub const EMPTY_YANKREG: Yankreg = Yankreg {
    y_array: Vec::new(),
    y_allocated: false,
    y_type: MotionType::MtCharWise,
    y_width: 0,
    timestamp: 0,
    additional_data: None,
};

struct RegisterModuleState {
    expr_line: Option<std::ffi::CString>,
    get_expr_line_nested: c_int,
    execreg_lastc: c_int,
    y_regs: [Yankreg; NUM_REGISTERS],
    y_previous: *mut Yankreg,
    empty_reg: Yankreg,
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
pub fn reg_empty(reg: &Yankreg) -> bool {
    {
        !reg.y_allocated
            || reg.y_array.is_empty()
            || (reg.y_array.len() == 1
                && reg.y_type == MotionType::MtCharWise
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
            reg_st()
                .y_previous
                .offset_from((&raw const reg_st().y_regs).cast::<Yankreg>()) as c_int
        }
    }
}

pub fn get_y_register(reg: c_int) -> &'static mut Yankreg {
    &mut reg_st().y_regs[reg as usize]
}

pub fn get_y_previous() -> *mut Yankreg {
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
        || (!writing && crate::strings::vim_haschr(c"/#.%:=", regname))
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
    regs: &[Yankreg],
    name: &mut c_char,
    reg: &mut *const Yankreg,
    is_unnamed: &mut bool,
) -> Option<usize> {
    *name = NUL;
    let mut idx = iter.unwrap_or(0);
    while idx < NUM_SAVED_REGISTERS && reg_empty(&regs[idx]) {
        idx += 1;
    }
    if idx >= NUM_SAVED_REGISTERS || reg_empty(&regs[idx]) {
        return None;
    }
    *name = get_register_name(idx as c_int) as u8 as c_char;
    let iter_reg = &raw const regs[idx];
    *reg = iter_reg;
    *is_unnamed = ptr::eq(iter_reg, reg_st().y_previous);
    idx += 1;
    while idx < NUM_SAVED_REGISTERS {
        if !reg_empty(&regs[idx]) {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

pub fn op_global_reg_iter(
    iter: Option<usize>,
    name: &mut c_char,
    reg: &mut *const Yankreg,
    is_unnamed: &mut bool,
) -> Option<usize> {
    op_reg_iter(iter, &reg_st().y_regs, name, reg, is_unnamed)
}

pub fn reg_max_ts(clipboard: bool) -> Timestamp {
    unsafe {
        let mut max_ts: Timestamp = 0;
        let mut iter: Option<usize> = None;
        loop {
            let mut name: c_char = NUL;
            let mut reg: *const Yankreg = ptr::null();
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
            max_ts = max_ts.max(get_y_register(op_reg_index(c_int::from(b'*'))).timestamp);
            max_ts = max_ts.max(get_y_register(op_reg_index(c_int::from(b'+'))).timestamp);
        }
        max_ts
    }
}

pub fn op_reg_set(name: c_char, reg: Yankreg, is_unnamed: bool) -> bool {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return false;
        }
        let target = (&raw mut reg_st().y_regs).cast::<Yankreg>().add(i as usize);
        *target = reg;

        if is_unnamed {
            reg_st().y_previous = target;
        }
        true
    }
}

pub fn op_reg_get(name: c_char) -> *const Yankreg {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return ptr::null();
        }
        (&raw const reg_st().y_regs)
            .cast::<Yankreg>()
            .add(i as usize)
    }
}

pub fn op_reg_set_previous(name: c_char) -> bool {
    unsafe {
        let i = op_reg_index(c_int::from(name as u8));
        if i == -1 {
            return false;
        }
        reg_st().y_previous = (&raw mut reg_st().y_regs).cast::<Yankreg>().add(i as usize);
        true
    }
}

pub fn update_yankreg_width(reg: &mut Yankreg) {
    {
        if reg.y_type == MotionType::MtBlockWise {
            let mut maxlen: usize = 0;
            for &item in &reg.y_array {
                let rowlen = mb_string2cells_len(item.data, item.size);
                maxlen = maxlen.max(rowlen);
            }
            assert!(maxlen <= c_int::MAX as usize);
            reg.y_width = reg.y_width.max((maxlen as Colnr) - 1);
        }
    }
}

pub fn get_yank_register(regname: c_int, mode: c_int) -> &'static mut Yankreg {
    unsafe {
        let mut reg: *mut Yankreg = ptr::null_mut();

        if (mode == YREG_PASTE || mode == YREG_PUT) && get_clipboard(regname, &raw mut reg, false) {
            return &mut *reg;
        } else if mode == YREG_PUT && (regname == c_int::from(b'*') || regname == c_int::from(b'+'))
        {
            return &mut reg_st().empty_reg;
        } else if mode != YREG_YANK
            && (regname == 0
                || regname == c_int::from(b'"')
                || regname == c_int::from(b'*')
                || regname == c_int::from(b'+'))
            && !reg_st().y_previous.is_null()
        {
            return &mut *reg_st().y_previous;
        }

        let mut i = op_reg_index(regname);
        if i == -1 {
            i = 0;
        }
        reg = (&raw mut reg_st().y_regs).cast::<Yankreg>().add(i as usize);

        if mode == YREG_YANK {
            reg_st().y_previous = reg;
        }
        &mut *reg
    }
}

pub fn yank_register_mline(regname: c_int, reg: *mut *mut Yankreg) -> bool {
    unsafe {
        *reg = ptr::null_mut();
        if regname != 0 && !valid_yank_reg(regname, false) {
            return false;
        }
        if regname == c_int::from(b'_') {
            return false;
        }
        *reg = get_yank_register(regname, YREG_PASTE);
        (*(*reg)).y_type == MotionType::MtLineWise
    }
}

pub fn copy_register(name: c_int) -> Yankreg {
    {
        let reg = &*get_yank_register(name, YREG_PASTE);
        let y_array = reg.y_array.iter().map(|&s| copy_string_global(s)).collect();
        Yankreg {
            y_array,
            y_allocated: reg.y_allocated,
            y_type: reg.y_type,
            y_width: reg.y_width,
            timestamp: reg.timestamp,
            additional_data: None,
        }
    }
}

fn stuff_yank(regname: c_int, p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    unsafe {
        if regname != 0 && !valid_yank_reg(regname, true) {
            return false;
        }
        if regname == c_int::from(b'_') {
            return true;
        }

        let plen = crate::memory::cstr_len(p);
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
            reg.y_type = MotionType::MtCharWise;
        }
        reg.timestamp = os_realtime() as u64;
        true
    }
}

pub fn do_record(c: c_int) -> bool {
    {
        let retval: bool;

        if g_st().reg_recording == 0 {
            if c < 0
                || (!(c <= 0x7f && (c as u8).is_ascii_alphanumeric()) && c != c_int::from(b'"'))
            {
                retval = false;
            } else {
                g_st().reg_recording = c;
                showmode();
                reg_st().do_record_regname = c;
                retval = true;
                apply_autocmds(
                    EVENT_RECORDINGENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
            }
        } else {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let dict = get_v_event(&mut save_v_event);

            let mut p_vec = get_recorded().into_bytes_with_nul();
            let p = p_vec.as_mut_ptr().cast::<c_char>();
            vim_unescape_ks(p);
            tv_dict_add_str(&mut *dict, c"regcontents", p);

            let mut buf = [0 as c_char; NUMBUFLEN + 2];
            buf[0] = reg_st().do_record_regname as u8 as c_char;
            buf[1] = NUL;
            tv_dict_add_str(&mut *dict, c"regname", buf.as_ptr());
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
            if opt_vars().p_ch == 0 || ui_has(K_UIMESSAGES) {
                showmode();
            } else {
                msg(c"", 0);
            }
            let old_y_previous = reg_st().y_previous;
            retval = stuff_yank(reg_st().do_record_regname, p);
            reg_st().y_previous = old_y_previous;
        }
        retval
    }
}

fn put_in_typebuf(
    s: impl crate::strings::IntoCCharPtr,
    esc: bool,
    colon: bool,
    silent: c_int,
) -> bool {
    let s = s.into_c_char_ptr();
    let mut retval = true;
    let is_silent = silent != 0;

    put_reedit_in_typebuf(silent);
    if colon {
        retval = ins_typebuf(c"\n", REMAP_NONE, 0, true, is_silent);
    }
    if retval {
        let owned_p = if esc {
            Some(vim_strsave_escape_ks(s))
        } else {
            None
        };
        let p = owned_p.as_ref().map_or(s, |cs| cs.as_ptr().cast_mut());
        if p.is_null() {
            retval = false;
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
    if colon && retval {
        retval = ins_typebuf(c":", REMAP_NONE, 0, true, is_silent);
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
    ) {
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
            if crate::memory::cstr_first_byte(p) != b'\\'
                && (crate::memory::cstr_first_byte(p) != b'"'
                    || crate::memory::cstr_byte_at(p, 1) != b'\\'
                    || crate::memory::cstr_byte_at(p, 2) != b' ')
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
            if crate::memory::cstr_first_byte(p) == b'\\' {
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

pub fn do_execreg(mut regname: c_int, colon: c_int, addcr: c_int, silent: c_int) -> bool {
    unsafe {
        let mut retval = true;

        if regname == c_int::from(b'@') {
            if reg_st().execreg_lastc == 0 {
                emsg(gettext(c"E748: No previously used register"));
                return false;
            }
            regname = reg_st().execreg_lastc;
        }
        if regname == c_int::from(b'%')
            || regname == c_int::from(b'#')
            || !valid_yank_reg(regname, false)
        {
            emsg_invreg(regname);
            return false;
        }
        reg_st().execreg_lastc = regname;

        if regname == c_int::from(b'_') {
            return true;
        }

        if regname == c_int::from(b':') {
            let Some(last_cmd) = g_st().last_cmdline.as_ref() else {
                emsg(gettext(E_NOLASTCMD));
                return false;
            };
            g_st().new_last_cmdline = None;
            let p = vim_strsave_escaped_ext(
                last_cmd.as_ptr(),
                c"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f",
                CTRL_V_CONST as u8 as c_char,
                false,
            );
            let p_ptr = p.as_ptr().cast_mut();
            if g_st().visual_global.active
                && (crate::memory::cstr_prefix_bytes(p_ptr, 5) == &b"'<,'>"[..5])
            {
                retval = put_in_typebuf(p_ptr.add(5), true, true, silent);
            } else {
                retval = put_in_typebuf(p_ptr, true, true, silent);
            }
        } else if regname == c_int::from(b'=') {
            let Some(p) = get_expr_line() else {
                return false;
            };
            retval = put_in_typebuf(p.as_ptr().cast_mut(), true, colon != 0, silent);
        } else if regname == c_int::from(b'.') {
            let Some(p) = get_last_insert_save() else {
                emsg(gettext(E_NOINSTEXT));
                return false;
            };
            retval = put_in_typebuf(p.as_ptr().cast_mut(), false, colon != 0, silent);
        } else {
            let reg = &*get_yank_register(regname, YREG_PASTE);
            if !reg.y_allocated {
                return false;
            }

            let remap = if colon != 0 { REMAP_NONE } else { REMAP_YES };
            let is_silent = silent != 0;

            put_reedit_in_typebuf(silent);
            let mut i = reg.y_array.len();
            while i > 0 {
                i -= 1;
                if (reg.y_type == MotionType::MtLineWise || i < reg.y_array.len() - 1 || addcr != 0)
                    && !ins_typebuf(c"\n", remap, 0, true, is_silent)
                {
                    return false;
                }

                let mut str_ = reg.y_array[i].data;
                let cont_vec: Vec<c_char>;
                if colon != 0 && i > 0 {
                    let p = skipwhite(str_);
                    if crate::memory::cstr_first_byte(p) == b'\\'
                        || (crate::memory::cstr_first_byte(p) == b'"'
                            && crate::memory::cstr_byte_at(p, 1) == b'\\'
                            && crate::memory::cstr_byte_at(p, 2) == b' ')
                    {
                        cont_vec =
                            execreg_line_continuation(reg.y_array.as_ptr().cast_mut(), &mut i);
                        str_ = cont_vec.as_ptr().cast_mut();
                    }
                }
                let escaped = vim_strsave_escape_ks(str_);
                retval = ins_typebuf(escaped.as_ptr().cast_mut(), remap, 0, true, is_silent);
                drop(escaped);
                if !retval {
                    return false;
                }
                if colon != 0 && !ins_typebuf(c":", remap, 0, true, is_silent) {
                    return false;
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

pub fn insert_reg(regname: c_int, mut reg: *mut Yankreg, literally_arg: bool) -> bool {
    unsafe {
        let mut retval = true;
        let mut owned: Option<Vec<c_char>> = None;
        let literally = literally_arg || is_literal_register(regname);

        os_breakcheck();
        if g_st().got_int {
            return false;
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return false;
        }

        let mut arg: *mut c_char = ptr::null_mut();
        if regname == c_int::from(b'.') {
            retval = stuff_inserted(0, 1, 1);
        } else if get_spec_reg(regname, &raw mut arg, &mut owned, true) {
            if arg.is_null() {
                return false;
            }
            stuffescaped(arg, literally);
        } else {
            if reg.is_null() {
                reg = get_yank_register(regname, YREG_PASTE);
            }
            let reg_ref = &*reg;
            if !reg_ref.y_allocated {
                retval = false;
            } else {
                let put = reg_ref.y_type == MotionType::MtCharWise
                    && (regname == c_int::from(b'-')
                        || (mc_ins_cascading()
                            && reg_ref.y_array.len() == 1
                            && (((0..=0x7f).contains(&regname)
                                && (regname as u8).is_ascii_alphanumeric())
                                || regname == c_int::from(b'"'))));
                for i in 0..reg_ref.y_array.len() {
                    if put {
                        let mut dir = BACKWARD;
                        if (g_st().state_global & REPLACE_FLAG) != 0 {
                            if !u_save_cursor() {
                                return false;
                            }
                            del_chars(mb_charlen(reg_ref.y_array[0].data), 1);
                            let curpos = curwin_mut().w_cursor;
                            if !oneright() {
                                dir = FORWARD;
                            }
                            curwin_mut().w_cursor = curpos;
                        }

                        redo_append_char(CTRL_R_CONST);
                        redo_append_char(regname);
                        do_put(regname, ptr::null_mut(), dir, 1, PUT_CURSEND);
                    } else {
                        stuffescaped(reg_ref.y_array[i].data, literally);
                        if reg_ref.y_type == MotionType::MtLineWise || i < reg_ref.y_array.len() - 1
                        {
                            stuffchar_readbuff(c_int::from(b'\n'));
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
                *argp = curbuf_mut().b_fname;
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
                let last_cmd_ptr = g_st()
                    .last_cmdline
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                if last_cmd_ptr.is_null() && errmsg {
                    emsg(gettext(E_NOLASTCMD));
                }
                *argp = last_cmd_ptr;
                true
            }
            x if x == c_int::from(b'/') => {
                if last_search_pat().is_null() && errmsg {
                    emsg(gettext(E_NOPREVRE));
                }
                *argp = last_search_pat();
                true
            }
            x if x == c_int::from(b'.') => {
                set_owned(argp, owned, get_last_insert_save());
                if (*argp).is_null() && errmsg {
                    emsg(gettext(E_NOINSTEXT));
                }
                true
            }
            CTRL_F_CONST | CTRL_P => {
                if !errmsg {
                    return false;
                }
                set_owned(
                    argp,
                    owned,
                    file_name_at_cursor(
                        FNAME_MESS | FNAME_HYP | if regname == CTRL_P { FNAME_EXP } else { 0 },
                        1,
                        None,
                    ),
                );
                true
            }
            CTRL_W_CONST | CTRL_A => {
                if !errmsg {
                    return false;
                }
                let cnt = find_ident_under_cursor(
                    argp,
                    if regname == CTRL_W_CONST {
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
            CTRL_L_CONST => {
                if !errmsg {
                    return false;
                }
                *argp = ml_get_buf(curwin_mut().buf_mut(), curwin_mut().w_cursor.lnum);
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
    {
        let literally = literally_arg || is_literal_register(regname);

        let reg = &*get_yank_register(regname, YREG_PASTE);
        if !reg.y_allocated {
            return false;
        }

        for i in 0..reg.y_array.len() {
            cmdline_paste_str(reg.y_array[i].data, literally);

            if i < reg.y_array.len() - 1 && !remcr {
                cmdline_paste_str(c"\r", literally);
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
        let regs = (&raw mut reg_st().y_regs).cast::<Yankreg>();
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
        let regs = (&raw mut reg_st().y_regs).cast::<Yankreg>();
        for i in 0..NUM_REGISTERS {
            *regs.add(i) = EMPTY_YANKREG;
        }
    }
}

fn yank_copy_line(
    reg: &mut Yankreg,
    bd: &mut BlockDef,
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
        (reg).y_array[y_idx] = cbuf_to_string(buf_vec.as_ptr(), out_size);
    }
}

pub fn op_yank_reg(oap: &mut Oparg, message: bool, mut reg: *mut Yankreg, append: bool) {
    unsafe {
        let mut newreg = EMPTY_YANKREG;
        let orig_motion = match oap.motion_type {
            0 => MotionType::MtCharWise,
            1 => MotionType::MtLineWise,
            2 => MotionType::MtBlockWise,
            _ => MotionType::MtUnknown,
        };
        let mut yank_type = orig_motion;
        let mut yanklines = oap.line_count as usize;
        let mut yankendlnum = oap.end.lnum;
        let mut bd = BlockDef::default();

        let curr = reg;
        if append && (*reg).y_allocated {
            reg = &raw mut newreg;
        } else {
            *reg = EMPTY_YANKREG;
        }

        if orig_motion == MotionType::MtCharWise
            && oap.start.col == 0
            && !oap.inclusive
            && (!oap.is_visual || crate::memory::cstr_first_byte(opt_vars().p_sel) == b'o')
            && oap.end.col == 0
            && yanklines > 1
        {
            yank_type = MotionType::MtLineWise;
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

        if yank_type == MotionType::MtBlockWise {
            (*reg).y_width = oap.end_vcol - oap.start_vcol;
            if curwin_mut().w_curswant == MAXCOL && (*reg).y_width > 0 {
                (*reg).y_width -= 1;
            }
        }

        while lnum <= yankendlnum {
            match (*reg).y_type {
                MotionType::MtBlockWise => {
                    block_prep(oap, &mut bd, lnum, false);
                    yank_copy_line(&mut *reg, &mut bd, y_idx, oap.excl_tr_ws);
                }
                MotionType::MtLineWise => {
                    (&mut *reg).y_array[y_idx] =
                        cbuf_to_string(ml_get(lnum), ml_get_len(lnum) as usize);
                }
                MotionType::MtCharWise => {
                    charwise_block_prep(oap.start, oap.end, &mut bd, lnum, oap.inclusive);
                    let tmp = crate::memory::cstr_len(bd.textstart) as c_int;
                    if tmp < bd.textlen {
                        bd.textlen = tmp;
                    }
                    yank_copy_line(&mut *reg, &mut bd, y_idx, false);
                }
                MotionType::MtUnknown => std::process::abort(),
            }
            lnum += 1;
            y_idx += 1;
        }

        if !ptr::eq(curr, reg) {
            let curr_reg = &mut *curr;
            let new_reg = &mut *reg;
            if yank_type == MotionType::MtLineWise {
                curr_reg.y_type = MotionType::MtLineWise;
            }

            if curr_reg.y_type == MotionType::MtCharWise
                && !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_REGAPPEND)
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
            if yank_type == MotionType::MtCharWise && yanklines == 1 {
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
                        gettext(c" into \"%c"),
                        oap.regname,
                    );
                }

                update_topline(curwin_mut());
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

                if yank_type == MotionType::MtBlockWise {
                    smsg!(
                        0,
                        ngettext(BLOCK_SINGLE, BLOCK_PLURAL, yanklines as u64,),
                        yanklines as i64,
                        namebuf.as_ptr(),
                    );
                } else {
                    smsg!(
                        0,
                        ngettext(LINE_SINGLE, LINE_PLURAL, yanklines as u64,),
                        yanklines as i64,
                        namebuf.as_ptr(),
                    );
                }
            }
        }

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start = oap.start;
            curbuf_mut().b_op_end = oap.end;
            if yank_type == MotionType::MtLineWise {
                curbuf_mut().b_op_start.col = 0;
                curbuf_mut().b_op_end.col = MAXCOL;
            }
            if yank_type == MotionType::MtCharWise && !oap.inclusive {
                decl(&mut curbuf_mut().b_op_end);
            }
        }
    }
}

pub fn format_reg_type(
    reg_type: MotionType,
    reg_width: Colnr,
    buf: *mut c_char,
    bufsize: usize,
) -> usize {
    unsafe {
        assert!(bufsize > 1);
        match reg_type {
            MotionType::MtLineWise => {
                *buf = b'V' as c_char;
                *buf.add(1) = NUL;
                1
            }
            MotionType::MtCharWise => {
                *buf = b'v' as c_char;
                *buf.add(1) = NUL;
                1
            }
            MotionType::MtBlockWise => {
                vim_snprintf_safelen!(buf, bufsize, c"\x16%d", reg_width + 1)
            }
            MotionType::MtUnknown => {
                *buf = NUL;
                0
            }
        }
    }
}

fn add_regtype_to_dict(reg: Option<&Yankreg>, dict: &mut TvDict, buf: *mut c_char, bufsize: usize) {
    {
        let len = format_reg_type(
            if let Some(reg) = reg {
                reg.y_type
            } else {
                MotionType::MtCharWise
            },
            if let Some(reg) = reg { reg.y_width } else { 0 },
            buf,
            bufsize,
        );
        tv_dict_add_str_len(dict, c"regtype", buf, len as c_int);
    }
}

pub fn do_autocmd_textyankpost(oap: &mut Oparg, reg: &mut Yankreg) {
    {
        if reg_st().textyankpost_recursive || !has_event(EVENT_TEXTYANKPOST) {
            return;
        }

        reg_st().textyankpost_recursive = true;

        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let dict = get_v_event(&mut save_v_event);

        let list = tv_list_alloc(reg.y_array.len() as isize);
        for &item in &reg.y_array {
            tv_list_append_string(&mut *list, item.data, item.size as isize);
        }
        list.lv_lock = VAR_FIXED;
        tv_dict_add_list(&mut *dict, c"regcontents", list);

        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        add_regtype_to_dict(Some(reg), &mut *dict, buf.as_mut_ptr(), buf.len());

        buf[0] = oap.regname as u8 as c_char;
        buf[1] = NUL;
        tv_dict_add_str(&mut *dict, c"regname", buf.as_ptr());

        tv_dict_add_bool(
            &mut *dict,
            c"inclusive",
            if oap.inclusive {
                K_BOOL_VAR_TRUE_CONST
            } else {
                K_BOOL_VAR_FALSE_CONST
            },
        );

        buf[0] = get_op_char(oap.op_type) as u8 as c_char;
        buf[1] = NUL;
        tv_dict_add_str(&mut *dict, c"operator", buf.as_ptr());

        tv_dict_add_bool(
            &mut *dict,
            c"visual",
            if oap.is_visual {
                K_BOOL_VAR_TRUE_CONST
            } else {
                K_BOOL_VAR_FALSE_CONST
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
    reg: Option<&Yankreg>,
    insert: Option<&NvimString>,
    post: bool,
    dir: Direction,
) {
    {
        if reg_st().put_autocmd_recursive || (regname == c_int::from(b'_') && reg.is_none()) {
            return;
        }

        if regname != c_int::from(b'.') && insert.is_none() && reg.is_none() {
            return;
        }

        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);

        let list = tv_list_alloc(reg.map_or(1, |reg| reg.y_array.len() as isize));

        if regname == c_int::from(b'.') {
            if !g_st().last_insert_ga.is_empty() {
                tv_list_append_string(
                    &mut *list,
                    g_st().last_insert_ga.as_ptr(),
                    g_st().last_insert_ga.len() as isize,
                );
            }
        } else if let Some(insert) = insert {
            tv_list_append_string(&mut *list, insert.data, insert.size as isize);
        } else {
            for &item in &(reg.unwrap()).y_array {
                tv_list_append_string(&mut *list, item.data, item.size as isize);
            }
        }

        list.lv_lock = VAR_FIXED;
        tv_dict_add_list(&mut *v_event, c"regcontents", list);

        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        buf[0] = regname as u8 as c_char;
        buf[1] = NUL;
        let buflen = if buf[0] == NUL { 0 } else { 1 };
        tv_dict_add_str_len(&mut *v_event, c"regname", buf.as_ptr(), buflen);

        buf[0] = if dir == BACKWARD { b'P' } else { b'p' } as c_char;
        buf[1] = NUL;
        tv_dict_add_str_len(&mut *v_event, c"operator", buf.as_ptr(), 1);

        add_regtype_to_dict(reg, &mut *v_event, buf.as_mut_ptr(), buf.len());

        tv_dict_add_bool(
            &mut *v_event,
            c"visual",
            if g_st().visual_global.active {
                K_BOOL_VAR_TRUE_CONST
            } else {
                K_BOOL_VAR_FALSE_CONST
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

pub fn op_yank(oap: &mut Oparg, message: bool) -> bool {
    {
        if oap.regname != 0 && !valid_yank_reg(oap.regname, true) {
            beep_flush();
            return false;
        }
        if oap.regname == c_int::from(b'_') {
            return true;
        }

        let reg = get_yank_register(oap.regname, YREG_YANK);
        op_yank_reg(oap, message, reg, is_append_register(oap.regname));
        set_clipboard(oap.regname, &*reg);
        do_autocmd_textyankpost(oap, &mut *reg);
        true
    }
}

pub fn do_put(
    regname: c_int,
    mut reg: *mut Yankreg,
    mut dir: c_int,
    mut count: c_int,
    flags: c_int,
) {
    unsafe {
        let mut totlen: usize = 0;
        let mut lnum: Linenr;
        let mut y_type: MotionType;
        let y_size: usize;
        let mut y_width: c_int = 0;
        let mut vcol: Colnr = 0;
        let y_array: *mut NvimString;
        let mut expr_y_array: Vec<NvimString> = Vec::new();
        let mut y_allocated = true;
        let mut nr_lines: Linenr = 0;
        let orig_start = curbuf_mut().b_op_start;
        let orig_end = curbuf_mut().b_op_end;
        let cur_ve_flags = get_ve_flags(curwin_mut());

        if ins_compl_preinsert_effect() {
            ins_compl_delete(false);
        }

        curbuf_mut().b_op_start = curwin_mut().w_cursor;
        curbuf_mut().b_op_end = curwin_mut().w_cursor;

        if regname == c_int::from(b'.') && reg.is_null() {
            let non_linewise_vis =
                g_st().visual_global.active && g_st().visual_global.mode != c_int::from(b'V');

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
                stuffchar_readbuff(K_COMMAND);
                if dir == FORWARD {
                    stuff_readbuff_len(c"put _", 5);
                } else {
                    stuff_readbuff_len(c"put! _", 6);
                }
                stuffchar_readbuff(c_int::from(CAR as u8));
                g_st().add_last_insert = save_add_last_insert;
            }

            if (flags & PUT_LINE) != 0 {
                stuffchar_readbuff(command_start_char);
                while count > 0 {
                    stuff_inserted(0, 1, c_int::from(count != 1));
                    if count != 1 {
                        let s: [u8; 4] = [b'\n', b' ', CTRL_U_CONST, 0];
                        stuff_readbuff_len(s.as_ptr().cast::<c_char>(), 3);
                    }
                    count -= 1;
                }
            } else {
                stuff_inserted(command_start_char, count, 0);
            }

            if has_event(EVENT_TEXTPUTPRE) {
                put_do_autocmd(c_int::from(b'.'), None, None, false, dir);
            }
            if has_event(EVENT_TEXTPUTPOST) {
                put_do_autocmd(c_int::from(b'.'), None, None, true, dir);
            }

            if has_textput_events {
                g_st().add_last_insert -= 1;
                if g_st().add_last_insert == 0 {
                    g_st().last_insert_ga = Vec::new();
                }
            }

            if (flags & PUT_CURSEND) != 0 {
                if (flags & PUT_LINE) != 0 {
                    stuff_readbuff(c"j0");
                } else {
                    let cursor_pos = get_cursor_pos_ptr();
                    let one_past_line = crate::memory::cstr_is_empty(cursor_pos);
                    let mut eol = false;
                    if !one_past_line {
                        eol = *cursor_pos.add(utfc_ptr2len(cursor_pos) as usize) == NUL;
                    }

                    let ve_allows =
                        cur_ve_flags == K_OPT_VE_FLAG_ALL || cur_ve_flags == K_OPT_VE_FLAG_ONEMORE;
                    let eof = curbuf_mut().b_ml.ml_line_count == curwin_mut().w_cursor.lnum
                        && one_past_line;
                    if ve_allows || !(eol || eof) {
                        stuffchar_readbuff(c_int::from(b'l'));
                    }
                }
            } else if (flags & PUT_LINE) != 0 {
                stuff_readbuff(c"g'[");
            }

            if command_start_char == c_int::from(b'a')
                && !u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1)
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

        if curbuf_mut().terminal.is_null()
            && !u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1)
        {
            return;
        }

        if !insert_string.data.is_null() {
            insert_string.size = crate::memory::cstr_len(insert_string.data);
            y_type = MotionType::MtCharWise;
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
                        if crate::memory::cstr_is_empty(tmp) {
                            y_type = MotionType::MtLineWise;
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
                put_do_autocmd(regname, None, Some(&insert_string), false, dir);
            }
        } else {
            if has_event(EVENT_TEXTPUTPRE) {
                let save_reg = reg;
                if reg.is_null() {
                    reg = get_yank_register(regname, YREG_PASTE);
                }
                put_do_autocmd(regname, reg.as_ref(), None, false, dir);
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
            if !curbuf_mut().terminal.is_null() {
                terminal_paste(count, y_array, y_size);
                break 'end;
            }

            let mut split_pos: Colnr = 0;
            if y_type == MotionType::MtLineWise {
                if (flags & PUT_LINE_SPLIT) != 0 {
                    if !u_save_cursor() {
                        break 'end;
                    }
                    let curline = get_cursor_line_ptr();
                    let mut p = get_cursor_pos_ptr();
                    let p_orig = p;
                    let plen = get_cursor_pos_len() as usize;
                    if dir == FORWARD && !crate::memory::cstr_is_empty(p) {
                        p = p.add(utfc_ptr2len(p) as usize);
                    }
                    split_pos = ((p as usize) - (curline as usize)) as Colnr;

                    let tail_len = plen - ((p as usize) - (p_orig as usize));
                    let mut tail_vec: Vec<c_char> = Vec::with_capacity(tail_len + 1);
                    tail_vec.extend_from_slice(std::slice::from_raw_parts(p, tail_len));
                    tail_vec.push(0);
                    ml_append(curwin_mut().w_cursor.lnum, tail_vec.as_mut_ptr(), 0, false);

                    crate::memline::ml_replace_len(
                        curwin_mut().w_cursor.lnum,
                        get_cursor_line_ptr(),
                        split_pos as usize,
                        true,
                    );
                    nr_lines += 1;
                    dir = FORWARD;

                    buf_updates_send_changes(curbuf_mut(), curwin_mut().w_cursor.lnum, 1, 1);
                }
                if (flags & PUT_LINE_FORWARD) != 0 {
                    curwin_mut().w_cursor = curbuf_mut().b_visual.vi_end;
                    dir = FORWARD;
                }
                curbuf_mut().b_op_start = curwin_mut().w_cursor;
                curbuf_mut().b_op_end = curwin_mut().w_cursor;
            }

            if (flags & PUT_LINE) != 0 {
                y_type = MotionType::MtLineWise;
            }

            if y_size == 0 || !y_allocated {
                semsg!(
                    gettext(c"E353: Nothing in register %s"),
                    if regname == 0 {
                        c"\"".as_ptr()
                    } else {
                        transchar(regname)
                    },
                );
                break 'end;
            }

            if y_type == MotionType::MtBlockWise {
                lnum = curwin_mut().w_cursor.lnum + (y_size as Linenr) + 1;
                lnum = lnum.min(curbuf_mut().b_ml.ml_line_count + 1);
                if !u_save(curwin_mut().w_cursor.lnum - 1, lnum) {
                    break 'end;
                }
            } else if y_type == MotionType::MtLineWise {
                lnum = curwin_mut().w_cursor.lnum;
                if dir == BACKWARD {
                    has_folding(curwin_ptr(), lnum, Some(&mut lnum), ptr::null_mut());
                } else {
                    has_folding(curwin_ptr(), lnum, None, &raw mut lnum);
                }
                if dir == FORWARD {
                    lnum += 1;
                }
                let save_res = if buf_is_empty(curbuf_mut()) {
                    u_save(0, 2)
                } else {
                    u_save(lnum - 1, lnum)
                };
                if !save_res {
                    break 'end;
                }
                if dir == FORWARD {
                    curwin_mut().w_cursor.lnum = lnum - 1;
                } else {
                    curwin_mut().w_cursor.lnum = lnum;
                }
                curbuf_mut().b_op_start = curwin_mut().w_cursor;
            } else if !u_save_cursor() {
                break 'end;
            }

            if cur_ve_flags == K_OPT_VE_FLAG_ALL && y_type == MotionType::MtCharWise {
                if gchar_cursor() == TAB {
                    let viscol = getviscol();
                    let ts = curbuf_mut().b_p_ts;
                    let cond = if dir == FORWARD {
                        tabstop_padding(viscol, ts, &curbuf_mut().b_p_vts_array) != 1
                    } else {
                        curwin_mut().w_cursor.coladd > 0
                    };
                    if cond {
                        coladvance_force(viscol);
                    } else {
                        curwin_mut().w_cursor.coladd = 0;
                    }
                } else if curwin_mut().w_cursor.coladd > 0 || gchar_cursor() == 0 {
                    coladvance_force(getviscol() + c_int::from(dir == FORWARD));
                }
            }

            lnum = curwin_mut().w_cursor.lnum;
            let mut col = curwin_mut().w_cursor.col;

            if y_type == MotionType::MtBlockWise {
                let mut incr: c_int = 0;
                let mut bd = BlockDef::default();
                let c = gchar_cursor();
                let mut endcol2: Colnr = 0;

                if dir == FORWARD && c != 0 {
                    if cur_ve_flags == K_OPT_VE_FLAG_ALL {
                        getvcol(
                            curwin_mut(),
                            &mut curwin_mut().w_cursor,
                            Some(&mut col),
                            None,
                            Some(&mut endcol2),
                            0,
                        );
                    } else {
                        getvcol(
                            curwin_mut(),
                            &mut curwin_mut().w_cursor,
                            None,
                            None,
                            Some(&mut col),
                            0,
                        );
                    }
                    curwin_mut().w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
                    col += 1;
                } else {
                    getvcol(
                        curwin_mut(),
                        &mut curwin_mut().w_cursor,
                        Some(&mut col),
                        None,
                        Some(&mut endcol2),
                        0,
                    );
                }

                col += curwin_mut().w_cursor.coladd;
                if cur_ve_flags == K_OPT_VE_FLAG_ALL
                    && (curwin_mut().w_cursor.coladd > 0 || endcol2 == curwin_mut().w_cursor.col)
                {
                    if dir == FORWARD && c == 0 {
                        col += 1;
                    }
                    if dir != FORWARD && c != 0 && curwin_mut().w_cursor.coladd > 0 {
                        curwin_mut().w_cursor.col += 1;
                    }
                    if c == TAB {
                        if dir == BACKWARD && curwin_mut().w_cursor.col != 0 {
                            curwin_mut().w_cursor.col -= 1;
                        }
                        if dir == FORWARD && col - 1 == endcol2 {
                            curwin_mut().w_cursor.col += 1;
                        }
                    }
                }
                curwin_mut().w_cursor.coladd = 0;
                bd.textcol = 0;

                for i in 0..y_size {
                    let mut spaces: c_int = 0;
                    let mut lines_appended: c_int = 0;

                    bd.startspaces = 0;
                    bd.endspaces = 0;
                    let mut delcount: c_int = 0;

                    if curwin_mut().w_cursor.lnum > curbuf_mut().b_ml.ml_line_count {
                        if !ml_append(curbuf_mut().b_ml.ml_line_count, c"", 1, false) {
                            break;
                        }
                        nr_lines += 1;
                        lines_appended = 1;
                    }
                    let oldp = get_cursor_line_ptr();
                    let oldlen = get_cursor_line_len();

                    let mut csarg = CharsizeArg::new();
                    let mut cstype: CSType = init_charsize_arg(
                        &mut csarg,
                        curwin_mut(),
                        curwin_mut().w_cursor.lnum,
                        oldp,
                    );
                    let mut ci = utf_ptr2_str_char_info(oldp);
                    vcol = 0;
                    while vcol < col && !crate::memory::cstr_is_empty(ci.ptr) {
                        let cli = utf_cluster_info(ci);
                        incr =
                            win_charsize(cstype, vcol, ci.ptr, ci.chr.value, &mut csarg, cli.cells)
                                .width;
                        vcol += incr;
                        ci = cli.next;
                    }
                    let mut ptr_ = ci.ptr.cast_mut();
                    bd.textcol = ((ptr_ as usize) - (oldp as usize)) as Colnr;

                    let shortline =
                        (vcol < col) || (vcol == col && crate::memory::cstr_is_empty(ptr_));

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
                            init_charsize_arg(&mut csarg, curwin_mut(), 0, (*y_array.add(i)).data);
                        ci = utf_ptr2_str_char_info((*y_array.add(i)).data);
                        while !crate::memory::cstr_is_empty(ci.ptr) {
                            let cli = utf_cluster_info(ci);
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
                        emsg(gettext(E_RESULTING_TEXT_TOO_LONG));
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
                    ml_replace(curwin_mut().w_cursor.lnum, newp, true);
                    drop(newp_vec);
                    extmark_splice_cols(
                        curbuf_mut(),
                        curwin_mut().w_cursor.lnum - 1,
                        bd.textcol,
                        delcount,
                        (totlen as c_int) + lines_appended,
                        K_EXTMARK_UNDO,
                    );

                    curwin_mut().w_cursor.lnum += 1;
                    if i == 0 {
                        curwin_mut().w_cursor.col += bd.startspaces;
                    }
                }

                changed_lines(
                    curbuf_mut(),
                    lnum,
                    0,
                    curbuf_mut().b_op_start.lnum + (y_size as Linenr) - nr_lines,
                    nr_lines,
                    true,
                );

                curbuf_mut().b_op_start = curwin_mut().w_cursor;
                curbuf_mut().b_op_start.lnum = lnum;

                curbuf_mut().b_op_end.lnum = curwin_mut().w_cursor.lnum - 1;
                curbuf_mut().b_op_end.col = (bd.textcol + (totlen as Colnr) - 1).max(0);
                curbuf_mut().b_op_end.coladd = 0;
                if (flags & PUT_CURSEND) != 0 {
                    curwin_mut().w_cursor = curbuf_mut().b_op_end;
                    curwin_mut().w_cursor.col += 1;

                    let len = get_cursor_line_len();
                    curwin_mut().w_cursor.col = curwin_mut().w_cursor.col.min(len);
                } else {
                    curwin_mut().w_cursor.lnum = lnum;
                }
            } else {
                let yanklen = (*y_array).size as c_int;

                if y_type == MotionType::MtCharWise {
                    if dir == FORWARD && gchar_cursor() != 0 {
                        let bytelen = utfc_ptr2len(get_cursor_pos_ptr());
                        col += bytelen;
                        if yanklen != 0 {
                            curwin_mut().w_cursor.col += bytelen;
                            curbuf_mut().b_op_end.col += bytelen;
                        }
                    }
                    curbuf_mut().b_op_start = curwin_mut().w_cursor;
                } else if dir == BACKWARD {
                    lnum -= 1;
                }
                let new_cursor = curwin_mut().w_cursor;

                if y_type == MotionType::MtCharWise && y_size == 1 {
                    let mut end_lnum: Linenr = 0;
                    let start_lnum = lnum;
                    let mut first_byte_off: c_int = 0;

                    if g_st().visual_global.active {
                        end_lnum = curbuf_mut()
                            .b_visual
                            .vi_end
                            .lnum
                            .max(curbuf_mut().b_visual.vi_start.lnum);
                        if end_lnum > start_lnum {
                            let mut pos = CursorPos {
                                lnum,
                                col,
                                coladd: 0,
                            };
                            getvcol(curwin_mut(), &mut pos, None, Some(&mut vcol), None, 0);
                        }
                    }

                    if count == 0 || yanklen == 0 {
                    } else if count > c_int::MAX / yanklen {
                        emsg(gettext(E_RESULTING_TEXT_TOO_LONG));
                    } else {
                        totlen = (count as usize) * (yanklen as usize);
                        loop {
                            let oldp = ml_get(lnum);
                            let oldlen = ml_get_len(lnum);
                            if lnum > start_lnum {
                                let mut pos = CursorPos {
                                    lnum,
                                    col: 0,
                                    coladd: 0,
                                };
                                if getvpos(curwin_ptr(), &mut pos, vcol) {
                                    col = pos.col;
                                } else {
                                    col = MAXCOL;
                                }
                            }
                            if g_st().visual_global.active && col > oldlen {
                                lnum += 1;
                                if !(g_st().visual_global.active && lnum <= end_lnum) {
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

                            if lnum == curwin_mut().w_cursor.lnum {
                                changed_cline_bef_curs(curwin_mut());
                                invalidate_botline_win(curwin_mut());
                                curwin_mut().w_cursor.col += (totlen - 1) as Colnr;
                            }
                            changed_bytes(lnum, col);
                            extmark_splice_cols(
                                curbuf_mut(),
                                lnum - 1,
                                col,
                                0,
                                totlen as c_int,
                                K_EXTMARK_UNDO,
                            );
                            if g_st().visual_global.active {
                                lnum += 1;
                            }
                            if !(g_st().visual_global.active && lnum <= end_lnum) {
                                break;
                            }
                        }
                    }

                    curbuf_mut().b_op_end = curwin_mut().w_cursor;
                    curbuf_mut().b_op_end.col -= first_byte_off;

                    if totlen != 0 && (g_st().restart_edit != 0 || (flags & PUT_CURSEND) != 0) {
                        curwin_mut().w_cursor.col += 1;
                    } else {
                        curwin_mut().w_cursor.col -= first_byte_off;
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
                            if y_type == MotionType::MtCharWise {
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

                                curwin_mut().w_cursor.lnum = lnum;
                                i = 1;
                            }

                            while i < y_size {
                                if y_type != MotionType::MtCharWise || i < y_size - 1 {
                                    if !ml_append(lnum, (*y_array.add(i)).data, 0, false) {
                                        break 'error;
                                    }
                                    new_lnum += 1;
                                }
                                lnum += 1;
                                nr_lines += 1;
                                if (flags & PUT_FIXINDENT) != 0 {
                                    let old_pos = curwin_mut().w_cursor;
                                    curwin_mut().w_cursor.lnum = lnum;
                                    let ptr_ = ml_get(lnum);
                                    if cnt == count && i == y_size - 1 {
                                        lendiff = ml_get_len(lnum);
                                    }
                                    let indent = if (crate::memory::cstr_first_byte(ptr_) == b'#'
                                        && preprocs_left())
                                        || crate::memory::cstr_is_empty(ptr_)
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
                                    curwin_mut().w_cursor = old_pos;
                                    if cnt == count && i == y_size - 1 {
                                        lendiff -= ml_get_len(lnum);
                                    }
                                }
                                i += 1;
                            }

                            let mut totsize: Bcount = 0;
                            let mut lastsize: c_int = 0;
                            if y_type == MotionType::MtCharWise
                                || (y_type == MotionType::MtLineWise
                                    && (flags & PUT_LINE_SPLIT) != 0)
                            {
                                for k in 0..(y_size - 1) {
                                    totsize += ((*y_array.add(k)).size as Bcount) + 1;
                                }
                                lastsize = (*y_array.add(y_size - 1)).size as c_int;
                                totsize += lastsize as Bcount;
                            }
                            if y_type == MotionType::MtCharWise {
                                extmark_splice(
                                    curbuf_mut(),
                                    new_cursor.lnum - 1,
                                    col,
                                    0,
                                    0,
                                    0,
                                    (y_size as c_int) - 1,
                                    lastsize,
                                    totsize,
                                    K_EXTMARK_UNDO,
                                );
                            } else if y_type == MotionType::MtLineWise
                                && (flags & PUT_LINE_SPLIT) != 0
                            {
                                extmark_splice(
                                    curbuf_mut(),
                                    new_cursor.lnum - 1,
                                    split_pos,
                                    0,
                                    0,
                                    0,
                                    (y_size as c_int) + 1,
                                    0,
                                    totsize + 2,
                                    K_EXTMARK_UNDO,
                                );
                            }

                            if cnt == 1 {
                                new_lnum = lnum;
                            }
                        }
                    }

                    if y_type == MotionType::MtLineWise {
                        curbuf_mut().b_op_start.col = 0;
                        if dir == FORWARD {
                            curbuf_mut().b_op_start.lnum += 1;
                        }
                    }

                    let kind = if y_type == MotionType::MtLineWise && (flags & PUT_LINE_SPLIT) == 0
                    {
                        K_EXTMARK_UNDO
                    } else {
                        K_EXTMARK_NOOP
                    };
                    mark_adjust(
                        curbuf_mut().b_op_start.lnum
                            + Linenr::from(y_type == MotionType::MtCharWise),
                        MAXLNUM,
                        nr_lines,
                        0,
                        kind,
                    );

                    if y_type == MotionType::MtCharWise {
                        changed_lines(
                            curbuf_mut(),
                            curwin_mut().w_cursor.lnum,
                            col,
                            curwin_mut().w_cursor.lnum + 1,
                            nr_lines,
                            true,
                        );
                    } else {
                        changed_lines(
                            curbuf_mut(),
                            curbuf_mut().b_op_start.lnum,
                            0,
                            curbuf_mut().b_op_start.lnum,
                            nr_lines,
                            true,
                        );
                    }

                    curbuf_mut().b_op_end.lnum = new_lnum;
                    let last_item = *y_array.add(y_size - 1);
                    col = ((last_item.size as Colnr) - lendiff).max(0);
                    if col > 1 {
                        curbuf_mut().b_op_end.col = col - 1;
                        if last_item.size > 0 {
                            curbuf_mut().b_op_end.col -= utf_head_off(
                                last_item.data,
                                last_item.data.add(last_item.size - 1),
                            );
                        }
                    } else {
                        curbuf_mut().b_op_end.col = 0;
                    }

                    if (flags & PUT_CURSLINE) != 0 {
                        curwin_mut().w_cursor.lnum = lnum;
                        beginline(BL_WHITE | BL_FIX);
                    } else if (flags & PUT_CURSEND) != 0 {
                        if y_type == MotionType::MtLineWise {
                            if lnum >= curbuf_mut().b_ml.ml_line_count {
                                curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
                            } else {
                                curwin_mut().w_cursor.lnum = lnum + 1;
                            }
                            curwin_mut().w_cursor.col = 0;
                        } else {
                            curwin_mut().w_cursor.lnum = new_lnum;
                            curwin_mut().w_cursor.col = col;
                            curbuf_mut().b_op_end = curwin_mut().w_cursor;
                            if col > 1 {
                                curbuf_mut().b_op_end.col = col - 1;
                            }
                        }
                    } else if y_type == MotionType::MtLineWise {
                        curwin_mut().w_cursor.col = 0;
                        if dir == FORWARD {
                            curwin_mut().w_cursor.lnum += 1;
                        }
                        beginline(BL_WHITE | BL_FIX);
                    } else {
                        curwin_mut().w_cursor = new_cursor;
                    }
                }
            }

            msgmore(nr_lines);
            curwin_mut().w_set_curswant = true;

            let len = get_cursor_line_len();
            if curwin_mut().w_cursor.col > len {
                if cur_ve_flags == K_OPT_VE_FLAG_ALL {
                    curwin_mut().w_cursor.coladd = curwin_mut().w_cursor.col - len;
                }
                curwin_mut().w_cursor.col = len;
            }
        }

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            curbuf_mut().b_op_start = orig_start;
            curbuf_mut().b_op_end = orig_end;
        }

        if has_event(EVENT_TEXTPUTPOST) {
            if insert_string.data.is_null() {
                put_do_autocmd(regname, reg.as_ref(), None, true, dir);
            } else {
                put_do_autocmd(regname, None, Some(&insert_string), true, dir);
            }
        }

        drop(insert_string_owned);

        if curbuf_mut().terminal.is_null() {
            g_st().visual_global.active = false;
        }

        adjust_cursor_eol();
    }
}

fn dis_msg(p: impl crate::strings::IntoCCharPtr, skip_esc: bool) {
    let mut p = p.into_c_char_ptr();
    unsafe {
        let mut n = g_st().columns_global - 6;
        while !crate::memory::cstr_is_empty(p)
            && !(*p == ESC && skip_esc && crate::memory::cstr_byte_at(p, 1) == 0)
        {
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

pub fn ex_display(eap: &mut Exarg) {
    unsafe {
        let mut arg = eap.arg;
        if !arg.is_null() && crate::memory::cstr_is_empty(arg) {
            arg = ptr::null_mut();
        }
        let hl_id = HLF_8;

        msg_ext_set_kind(c"list_cmd");
        g_st().msg_ext_skip_flush = true;
        msg_puts_title(gettext(c"\nType Name Content"));

        let regs = (&raw mut reg_st().y_regs).cast::<Yankreg>();
        let mut i: c_int = -1;
        while i < (NUM_REGISTERS as c_int) && !g_st().got_int {
            let name = get_register_name(i);
            if !arg.is_null() && !crate::strings::vim_haschr(arg, name) {
                i += 1;
                continue;
            }

            let type_ch = match get_reg_type(name, None) {
                MotionType::MtLineWise => c_int::from(b'l'),
                MotionType::MtCharWise => c_int::from(b'c'),
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
                    msg_puts(c"  ");
                    msg_putchar(type_ch);
                    msg_puts(c"  ");
                    msg_putchar(c_int::from(b'"'));
                    msg_putchar(name);
                    msg_puts(c"   ");

                    let mut n = g_st().columns_global - 11;
                    let mut j: usize = 0;
                    while j < yb_ref.y_array.len() && n > 1 {
                        if j != 0 {
                            msg_puts_hl(c"^J", hl_id, false);
                            n -= 2;
                        }
                        let mut p: *const c_char = yb_ref.y_array[j].data;
                        while !crate::memory::cstr_is_empty(p) {
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
                    if n > 1 && yb_ref.y_type == MotionType::MtLineWise {
                        msg_puts_hl(c"^J", hl_id, false);
                    }
                }
                os_breakcheck();
            }

            i += 1;
        }

        let insert = get_last_insert();
        let p = insert.data;
        if !p.is_null()
            && (arg.is_null() || crate::strings::vim_haschr(arg, c_int::from(b'.')))
            && !g_st().got_int
            && !message_filtered(p)
        {
            msg_puts(c"\n  c  \".   ");
            dis_msg(p, true);
        }

        if let Some(last_cmd) = g_st().last_cmdline.as_ref()
            && (arg.is_null() || crate::strings::vim_haschr(arg, c_int::from(b':')))
            && !g_st().got_int
            && !message_filtered(last_cmd.as_ptr())
        {
            msg_puts(c"\n  c  \":   ");
            dis_msg(last_cmd.as_ptr(), false);
        }

        if !curbuf_mut().b_fname.is_null()
            && (arg.is_null() || crate::strings::vim_haschr(arg, c_int::from(b'%')))
            && !g_st().got_int
            && !message_filtered(curbuf_mut().b_fname)
        {
            msg_puts(c"\n  c  \"%   ");
            dis_msg(curbuf_mut().b_fname, false);
        }

        if (arg.is_null() || crate::strings::vim_haschr(arg, c_int::from(b'#'))) && !g_st().got_int
        {
            let mut fname: *mut c_char = ptr::null_mut();
            let mut dummy: Linenr = 0;
            if buflist_name_nr(0, &raw mut fname, &mut dummy) && !message_filtered(fname) {
                msg_puts(c"\n  c  \"#   ");
                dis_msg(fname, false);
            }
        }

        if !last_search_pat().is_null()
            && (arg.is_null() || crate::strings::vim_haschr(arg, c_int::from(b'/')))
            && !g_st().got_int
            && !message_filtered(last_search_pat())
        {
            msg_puts(c"\n  c  \"/   ");
            dis_msg(last_search_pat(), false);
        }

        if let Some(el) = &reg_st().expr_line
            && (arg.is_null() || crate::strings::vim_haschr(arg, c_int::from(b'=')))
            && !g_st().got_int
            && !message_filtered(el.as_ptr())
        {
            msg_puts(c"\n  c  \"=   ");
            dis_msg(el.as_ptr(), false);
        }
        g_st().msg_ext_skip_flush = false;
    }
}

pub fn get_reg_type(regname: c_int, reg_width: Option<&mut Colnr>) -> MotionType {
    {
        if regname == c_int::from(b'%')
            || regname == c_int::from(b'#')
            || regname == c_int::from(b'=')
            || regname == c_int::from(b':')
            || regname == c_int::from(b'/')
            || regname == c_int::from(b'.')
            || regname == CTRL_F_CONST
            || regname == CTRL_P
            || regname == CTRL_W_CONST
            || regname == CTRL_A
            || regname == c_int::from(b'_')
        {
            return MotionType::MtCharWise;
        }

        if regname != 0 && !valid_yank_reg(regname, false) {
            return MotionType::MtUnknown;
        }

        let reg = &*get_yank_register(regname, YREG_PASTE);
        if reg.y_allocated {
            if reg.y_type == MotionType::MtBlockWise
                && let Some(reg_width) = reg_width
            {
                *reg_width = reg.y_width;
            }
            return reg.y_type;
        }
        MotionType::MtUnknown
    }
}

fn get_reg_wrap_one_line_list(s: Option<std::ffi::CString>) -> *mut TvList {
    {
        let Some(s) = s else {
            return ptr::null_mut();
        };
        let list = tv_list_alloc(1);
        tv_list_append_string(&mut *list, s.as_ptr(), -1);
        list
    }
}

pub fn get_reg_contents_str(mut regname: c_int, flags: c_int) -> Option<std::ffi::CString> {
    {
        if regname == c_int::from(b'=') {
            if (flags & K_GREG_NO_EXPR) != 0 {
                return None;
            }
            if (flags & K_GREG_EXPR_SRC) != 0 {
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
            return Some(crate::memory::cstr_to_cstring(retval));
        }

        let reg = &*get_yank_register(regname, YREG_PUT);
        if !reg.y_allocated {
            return None;
        }

        let mut len: usize = 0;
        for (i, &item) in reg.y_array.iter().enumerate() {
            len += item.size;
            if reg.y_type == MotionType::MtLineWise || i < reg.y_array.len() - 1 {
                len += 1;
            }
        }

        let mut out: Vec<u8> = Vec::with_capacity(len);
        for (i, &item) in reg.y_array.iter().enumerate() {
            if item.size > 0 {
                out.extend_from_slice(crate::memory::cstr_slice(item.data, item.size));
            }
            if reg.y_type == MotionType::MtLineWise || i < reg.y_array.len() - 1 {
                out.push(b'\n');
            }
        }

        Some(std::ffi::CString::new(out).unwrap_or_default())
    }
}

pub fn get_reg_contents_list(mut regname: c_int, flags: c_int) -> *mut TvList {
    {
        if regname == c_int::from(b'=') {
            if (flags & K_GREG_NO_EXPR) != 0 {
                return ptr::null_mut();
            }
            if (flags & K_GREG_EXPR_SRC) != 0 {
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
            return get_reg_wrap_one_line_list(Some(crate::memory::cstr_to_cstring(retval)));
        }

        let reg = &*get_yank_register(regname, YREG_PUT);
        if !reg.y_allocated {
            return ptr::null_mut();
        }

        let list = tv_list_alloc(reg.y_array.len() as isize);
        for &item in &reg.y_array {
            tv_list_append_string(&mut *list, item.data, item.size as isize);
        }
        list
    }
}

fn init_write_reg(
    name: c_int,
    old_y_previous: *mut *mut Yankreg,
    must_append: bool,
) -> *mut Yankreg {
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
    y_ptr: &mut Yankreg,
    mut yank_type: MotionType,
    str_: impl crate::strings::IntoCCharPtr,
    len: usize,
    blocklen: Colnr,
    str_list: bool,
) {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        if !y_ptr.y_allocated {
            y_ptr.y_array.clear();
        }

        if yank_type == MotionType::MtUnknown {
            yank_type = if str_list
                || (len > 0 && (*str_.add(len - 1) == NL || *str_.add(len - 1) == CAR))
            {
                MotionType::MtLineWise
            } else {
                MotionType::MtCharWise
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
            newlines = {
                let _d = (str_).cast::<u8>();
                let _sz = len;
                if _sz == 0 {
                    0
                } else {
                    std::slice::from_raw_parts(_d, _sz)
                        .iter()
                        .filter(|&&_b| _b == (b'\n' as c_char) as u8)
                        .count()
                }
            };
            if yank_type == MotionType::MtCharWise
                || len == 0
                || crate::memory::cstr_byte_at(str_, len - 1) != b'\n'
            {
                extraline = 1;
                newlines += 1;
            }
            if !y_ptr.y_array.is_empty() && y_ptr.y_type == MotionType::MtCharWise {
                append = true;
                newlines -= 1;
            }
        }

        if y_ptr.y_array.len() + newlines == 0 {
            y_ptr.y_array.clear();
            y_ptr.y_allocated = false;
            return;
        }

        y_ptr.y_array.reserve(newlines);
        y_ptr.y_allocated = true;

        let mut maxlen: usize = 0;

        if str_list {
            let mut ss = str_.cast::<*mut c_char>();
            while !(*ss).is_null() {
                y_ptr.y_array.push(cstr_to_string(*ss));
                if yank_type == MotionType::MtBlockWise {
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
                let slice = crate::memory::cstr_slice(start, rem);
                let line_len = slice.iter().position(|&b| b == b'\n').unwrap_or(rem);
                if yank_type == MotionType::MtBlockWise {
                    let charlen = mb_string2cells_len(start, line_len);
                    maxlen = maxlen.max(charlen);
                }

                let extra = if append {
                    y_ptr.y_array.last().unwrap().size
                } else {
                    0
                };
                let s_len = extra + line_len;
                let mut s_vec = vec![0 as c_char; s_len + 1];
                if extra > 0 {
                    ptr::copy_nonoverlapping(
                        y_ptr.y_array.last().unwrap().data,
                        s_vec.as_mut_ptr(),
                        extra,
                    );
                }
                if line_len > 0 {
                    ptr::copy_nonoverlapping(start, s_vec.as_mut_ptr().add(extra), line_len);
                }
                {
                    let _d = (s_vec.as_mut_ptr()).cast::<u8>();
                    let _sz = s_len;
                    if _sz > 0 {
                        let _c = (NUL) as u8;
                        let _x = (b'\n' as c_char) as u8;
                        for _b in std::slice::from_raw_parts_mut(_d, _sz) {
                            if *_b == _c {
                                *_b = _x;
                            }
                        }
                    }
                };
                let new_str = cbuf_to_string(s_vec.as_ptr(), s_len);

                if append {
                    let last = y_ptr.y_array.last_mut().unwrap();
                    api_clear_string(last);
                    *last = new_str;
                    append = false;
                } else {
                    y_ptr.y_array.push(new_str);
                }

                start = start.add(line_len + 1);
            }
        }
        y_ptr.y_type = yank_type;
        y_ptr.additional_data = None;
        y_ptr.timestamp = os_realtime() as u64;
        if yank_type == MotionType::MtBlockWise {
            y_ptr.y_width = if blocklen == -1 {
                (maxlen as Colnr) - 1
            } else {
                blocklen
            };
        } else {
            y_ptr.y_width = 0;
        }
    }
}

fn finish_write_reg(name: c_int, reg: &mut Yankreg, old_y_previous: *mut Yankreg) {
    set_clipboard(name, reg);
    if name != c_int::from(b'"') {
        reg_st().y_previous = old_y_previous;
    }
}

pub fn write_reg_contents(
    name: c_int,
    str_: impl crate::strings::IntoCCharPtr,
    len: isize,
    must_append: c_int,
) {
    let str_ = str_.into_c_char_ptr();
    write_reg_contents_ex(name, str_, len, must_append != 0, MotionType::MtUnknown, 0);
}

pub fn write_reg_contents_lst(
    name: c_int,
    strings: *mut *mut c_char,
    must_append: bool,
    yank_type: MotionType,
    block_len: Colnr,
) {
    unsafe {
        if name == c_int::from(b'/') || name == c_int::from(b'=') || name == c_int::from(b'#') {
            let s = if (*strings).is_null() {
                c"".as_ptr()
            } else if !(*strings.add(1)).is_null() {
                semsg!(gettext(E_REGISTER_CHAR_CANNOT_CONTAIN_MULTIPLE_LINES), name,);
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

        let mut old_y_previous: *mut Yankreg = ptr::null_mut();
        let reg = init_write_reg(name, &raw mut old_y_previous, must_append);
        if reg.is_null() {
            return;
        }

        str_to_reg(
            &mut *reg,
            yank_type,
            strings.cast::<c_char>(),
            0,
            block_len,
            true,
        );
        finish_write_reg(name, &mut *reg, old_y_previous);
    }
}

pub fn write_reg_contents_ex(
    name: c_int,
    str_: impl crate::strings::IntoCCharPtr,
    mut len: isize,
    must_append: bool,
    yank_type: MotionType,
    block_len: Colnr,
) {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        if len < 0 {
            len = crate::memory::cstr_len(str_) as isize;
        }

        if name == c_int::from(b'/') {
            set_last_search_pat(str_, RE_SEARCH, 1, true);
            return;
        }

        if name == c_int::from(b'#') {
            if len == 0 {
                curwin_mut().w_alt_fnum = 0;
                return;
            }

            let buf = if (crate::memory::cstr_first_byte(str_)).is_ascii_digit() {
                let num =
                    crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(str_)) as c_int;
                let b = buflist_findnr(num);
                if b.is_null() {
                    semsg!(gettext(E_NOBUFNR), i64::from(num));
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
            curwin_mut().w_alt_fnum = (*buf).b_fnum;
            return;
        }

        if name == c_int::from(b'=') {
            let mut out: Vec<u8> = Vec::new();
            if must_append && let Some(el) = &reg_st().expr_line {
                out.extend_from_slice(el.as_bytes());
            }
            let slice = crate::memory::cstr_slice(str_, len as usize);
            let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
            out.extend_from_slice(&slice[..end]);
            reg_st().expr_line = Some(std::ffi::CString::new(out).unwrap_or_default());
            return;
        }

        if name == c_int::from(b'_') {
            return;
        }

        let mut old_y_previous: *mut Yankreg = ptr::null_mut();
        let reg = init_write_reg(name, &raw mut old_y_previous, must_append);
        if reg.is_null() {
            return;
        }
        str_to_reg(&mut *reg, yank_type, str_, len as usize, block_len, false);
        finish_write_reg(name, &mut *reg, old_y_previous);
    }
}

pub fn prepare_yankreg_from_object(reg: &mut Yankreg, regtype: NvimString, _lines: usize) -> bool {
    unsafe {
        let type_ch = if !regtype.data.is_null() {
            crate::memory::cstr_first_byte(regtype.data)
        } else {
            0
        };

        reg.y_type = match type_ch {
            0 => MotionType::MtUnknown,
            b'v' | b'c' => MotionType::MtCharWise,
            b'V' | b'l' => MotionType::MtLineWise,
            b'b' | 0x16 => MotionType::MtBlockWise,
            _ => return false,
        };

        reg.y_width = 0;
        if regtype.size > 1 {
            if reg.y_type != MotionType::MtBlockWise {
                return false;
            }
            if !(crate::memory::cstr_byte_at(regtype.data, 1)).is_ascii_digit() {
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

pub fn finish_yankreg_from_object(reg: &mut Yankreg, clipboard_adjust: bool) {
    if reg.y_array.last().is_some_and(|last| last.size == 0) {
        if reg.y_type != MotionType::MtCharWise {
            if reg.y_type == MotionType::MtUnknown || clipboard_adjust {
                reg.y_array.pop();
            }
            if reg.y_type == MotionType::MtUnknown {
                reg.y_type = MotionType::MtLineWise;
            }
        }
    } else if reg.y_type == MotionType::MtUnknown {
        reg.y_type = MotionType::MtCharWise;
    }

    update_yankreg_width(reg);
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
