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
use crate::charset::{NUL, ascii_iswhite, getdigits_int, transchar, vim_strsize};
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
    mb_charlen, mb_string2cells_len, mb_tolower, ptr2cells, utf_cluster_info,
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
    format_to_buf, format_to_buf_safelen, gettext, ngettext, vim_strchr, vim_strsave_escaped_ext,
};
use crate::terminal::terminal_paste;
use crate::textformat::Oparg;
use crate::ui::ui_has;
use crate::undo::{u_save, u_save_cursor};
use std::ffi::CStr;
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
    pub y_width: i32,
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

pub const PUT_FIXINDENT: i32 = 1;
pub const PUT_CURSEND: i32 = 2;
pub const PUT_CURSLINE: i32 = 4;
pub const PUT_LINE: i32 = 8;
pub const PUT_LINE_SPLIT: i32 = 16;
pub const PUT_LINE_FORWARD: i32 = 32;
pub const PUT_BLOCK_INNER: i32 = 64;

pub const DELETION_REGISTER: i32 = 36;
pub const NUM_SAVED_REGISTERS: usize = 37;
pub const STAR_REGISTER: i32 = 37;
pub const PLUS_REGISTER: i32 = 38;
pub const NUM_REGISTERS: usize = 39;

pub const K_GREG_NO_EXPR: i32 = 1;
pub const K_GREG_EXPR_SRC: i32 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum YankRegMode {
    Paste,
    Yank,
    Put,
}

const K_CPO_REGAPPEND: i32 = b'>' as i32;

const E_REGISTER_CHAR_CANNOT_CONTAIN_MULTIPLE_LINES: &CStr =
    c"E883: Register '%c' cannot contain multiple lines";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BlockDef {
    pub startspaces: i32,
    pub endspaces: i32,
    pub textlen: i32,
    pub textstart: &'static std::ffi::CStr,
    pub textcol: Colnr,
    pub start_vcol: Colnr,
    pub end_vcol: Colnr,
    pub is_short: bool,
    pub is_max: bool,
    pub is_one_char: bool,
    pub pre_whitesp: i32,
    pub pre_whitesp_c: i32,
    pub end_char_vcols: Colnr,
    pub start_char_vcols: Colnr,
}

impl Default for BlockDef {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockDef {
    pub const fn new() -> Self {
        Self {
            startspaces: 0,
            endspaces: 0,
            textlen: 0,
            textstart: c"",
            textcol: 0,
            start_vcol: 0,
            end_vcol: 0,
            is_short: false,
            is_max: false,
            is_one_char: false,
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
    get_expr_line_nested: i32,
    execreg_lastc: i32,
    y_regs: [Yankreg; NUM_REGISTERS],
    y_previous: crate::globals::GraphPtr<Yankreg>,
    empty_reg: Yankreg,
    do_record_regname: i32,
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
            y_previous: crate::globals::GraphPtr::NULL,
            empty_reg: EMPTY_YANKREG,
            do_record_regname: 0,
            textyankpost_recursive: false,
            put_autocmd_recursive: false,
        }
    }
}

static REG_STATE: GlobalState<RegisterModuleState> = GlobalState::new(RegisterModuleState::new());

#[inline]
fn reg_st() -> crate::globals::GraphPtr<RegisterModuleState> {
    REG_STATE.handle()
}

#[inline]
pub const fn is_literal_register(regname: i32) -> bool {
    regname == b'*' as i32
        || regname == b'+' as i32
        || (regname >= 0 && regname <= 0x7f && (regname as u8).is_ascii_alphanumeric())
}

#[inline]
pub const fn op_reg_index(regname: i32) -> i32 {
    if regname >= b'0' as i32 && regname <= b'9' as i32 {
        regname - b'0' as i32
    } else if regname >= b'a' as i32 && regname <= b'z' as i32 {
        regname - b'a' as i32 + 10
    } else if regname >= b'A' as i32 && regname <= b'Z' as i32 {
        regname - b'A' as i32 + 10
    } else if regname == b'-' as i32 {
        DELETION_REGISTER
    } else if regname == b'*' as i32 {
        STAR_REGISTER
    } else if regname == b'+' as i32 {
        PLUS_REGISTER
    } else {
        -1
    }
}

#[inline]
pub const fn is_append_register(regname: i32) -> bool {
    regname >= b'A' as i32 && regname <= b'Z' as i32
}

#[inline]
pub const fn get_register_name(num: i32) -> i32 {
    if num == -1 {
        b'"' as i32
    } else if num < 10 {
        num + b'0' as i32
    } else if num == DELETION_REGISTER {
        b'-' as i32
    } else if num == STAR_REGISTER {
        b'*' as i32
    } else if num == PLUS_REGISTER {
        b'+' as i32
    } else {
        num + b'a' as i32 - 10
    }
}

#[inline]
pub fn reg_empty(reg: &Yankreg) -> bool {
    !reg.y_allocated
        || reg.y_array.is_empty()
        || (reg.y_array.len() == 1
            && reg.y_type == MotionType::MtCharWise
            && reg.y_array[0].size == 0)
}

use crate::api::private::helpers::cstr_to_string;

#[inline]
fn copy_string_global(s: NvimString) -> NvimString {
    if s.data.is_null() {
        NvimString::new()
    } else {
        cbuf_to_string(s.as_bytes())
    }
}

pub fn get_unname_register() -> i32 {
    if reg_st().y_previous.is_null() {
        -1
    } else {
        ((reg_st().y_previous.as_ptr() as usize - reg_st().y_regs.as_ptr() as usize)
            / std::mem::size_of::<Yankreg>()) as i32
    }
}

pub fn get_y_register(reg: i32) -> &'static mut Yankreg {
    &mut reg_st().borrow_mut().y_regs[reg as usize]
}

pub fn get_y_previous() -> &'static mut Yankreg {
    reg_st().y_previous.borrow_mut()
}

pub fn get_expr_register() -> i32 {
    let Some(new_line) = getcmdline(i32::from(b'='), 0, 0, true) else {
        return 0;
    };
    if !new_line.as_bytes().is_empty() {
        set_expr_line(Some(new_line));
    }
    i32::from(b'=')
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
    let rv = eval_to_string(&expr_copy, true, false);
    reg_st().get_expr_line_nested -= 1;
    rv
}

pub fn get_expr_line_src() -> Option<std::ffi::CString> {
    reg_st().expr_line.clone()
}

pub fn valid_yank_reg(regname: i32, writing: bool) -> bool {
    (regname > 0 && regname <= 0x7f && (regname as u8).is_ascii_alphanumeric())
        || (!writing && crate::strings::vim_haschr(c"/#.%:=", regname))
        || regname == i32::from(b'"')
        || regname == i32::from(b'-')
        || regname == i32::from(b'_')
        || regname == i32::from(b'*')
        || regname == i32::from(b'+')
}

pub fn get_default_register_name() -> i32 {
    let mut name: i32 = 0;
    adjust_clipboard_name(&mut name, true, false);
    name
}

pub fn op_reg_iter<'a>(
    iter: Option<usize>,
    regs: &'a [Yankreg],
    name: &mut u8,
    reg: &mut Option<&'a Yankreg>,
    is_unnamed: &mut bool,
) -> Option<usize> {
    *name = 0;
    let mut idx = iter.unwrap_or(0);
    while idx < NUM_SAVED_REGISTERS && reg_empty(&regs[idx]) {
        idx += 1;
    }
    if idx >= NUM_SAVED_REGISTERS || reg_empty(&regs[idx]) {
        return None;
    }
    *name = get_register_name(idx as i32) as u8;
    let iter_reg = &regs[idx];
    *reg = Some(iter_reg);
    *is_unnamed = reg_st().y_previous == iter_reg;
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
    name: &mut u8,
    reg: &mut Option<&'static Yankreg>,
    is_unnamed: &mut bool,
) -> Option<usize> {
    op_reg_iter(iter, &reg_st().borrow_mut().y_regs, name, reg, is_unnamed)
}

pub fn reg_max_ts(clipboard: bool) -> Timestamp {
    let mut max_ts: Timestamp = 0;
    let mut iter: Option<usize> = None;
    loop {
        let mut name: u8 = 0;
        let mut reg: Option<&Yankreg> = None;
        let mut is_unnamed = false;
        iter = op_global_reg_iter(iter, &mut name, &mut reg, &mut is_unnamed);
        if name == 0 {
            break;
        }
        if let Some(r) = reg {
            max_ts = max_ts.max(r.timestamp);
        }
        if iter.is_none() {
            break;
        }
    }
    if clipboard {
        max_ts = max_ts.max(get_y_register(op_reg_index(i32::from(b'*'))).timestamp);
        max_ts = max_ts.max(get_y_register(op_reg_index(i32::from(b'+'))).timestamp);
    }
    max_ts
}

pub fn op_reg_set(name: u8, reg: Yankreg, is_unnamed: bool) -> bool {
    let i = op_reg_index(i32::from(name));
    if i == -1 {
        return false;
    }
    let idx = i as usize;
    reg_st().y_regs[idx] = reg;

    if is_unnamed {
        reg_st().y_previous = crate::globals::GraphPtr::from(&mut reg_st().y_regs[idx]);
    }
    true
}

pub fn op_reg_get(name: u8) -> Option<&'static Yankreg> {
    let i = op_reg_index(i32::from(name));
    if i == -1 {
        return None;
    }
    Some(&reg_st().borrow_mut().y_regs[i as usize])
}

pub fn op_reg_set_previous(name: u8) -> bool {
    let i = op_reg_index(i32::from(name));
    if i == -1 {
        return false;
    }
    reg_st().y_previous = crate::globals::GraphPtr::from(&mut reg_st().y_regs[i as usize]);
    true
}

pub fn update_yankreg_width(reg: &mut Yankreg) {
    if reg.y_type == MotionType::MtBlockWise {
        let mut maxlen: usize = 0;
        for &item in &reg.y_array {
            let rowlen = mb_string2cells_len(item.as_bytes());
            maxlen = maxlen.max(rowlen);
        }
        assert!(maxlen <= i32::MAX as usize);
        reg.y_width = reg.y_width.max((maxlen as Colnr) - 1);
    }
}

pub fn get_yank_register(regname: i32, mode: YankRegMode) -> &'static mut Yankreg {
    let mut reg: Option<&'static mut Yankreg> = None;

    if (mode == YankRegMode::Paste || mode == YankRegMode::Put)
        && get_clipboard(regname, &mut reg, false)
    {
        return reg.unwrap();
    } else if mode == YankRegMode::Put && (regname == i32::from(b'*') || regname == i32::from(b'+'))
    {
        return &mut reg_st().borrow_mut().empty_reg;
    } else if mode != YankRegMode::Yank
        && (regname == 0
            || regname == i32::from(b'"')
            || regname == i32::from(b'*')
            || regname == i32::from(b'+'))
        && !reg_st().y_previous.is_null()
    {
        return reg_st().y_previous.borrow_mut();
    }

    let mut i = op_reg_index(regname);
    if i == -1 {
        i = 0;
    }
    let idx = i as usize;
    if mode == YankRegMode::Yank {
        reg_st().y_previous = crate::globals::GraphPtr::from(&mut reg_st().y_regs[idx]);
    }
    &mut reg_st().borrow_mut().y_regs[idx]
}

pub fn yank_register_mline(regname: i32, reg: &mut Option<&'static mut Yankreg>) -> bool {
    *reg = None;
    if regname != 0 && !valid_yank_reg(regname, false) {
        return false;
    }
    if regname == i32::from(b'_') {
        return false;
    }
    let r = get_yank_register(regname, YankRegMode::Paste);
    let is_mline = r.y_type == MotionType::MtLineWise;
    *reg = Some(r);
    is_mline
}

pub fn copy_register(name: i32) -> Yankreg {
    let reg = &*get_yank_register(name, YankRegMode::Paste);
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

fn stuff_yank(regname: i32, p: &[u8]) -> bool {
    if regname != 0 && !valid_yank_reg(regname, true) {
        return false;
    }
    if regname == i32::from(b'_') {
        return true;
    }

    let p_bytes = crate::memory::bytes_until_nul(p);
    let plen = p_bytes.len();
    let reg = &mut *get_yank_register(regname, YankRegMode::Yank);
    if is_append_register(regname) && reg.y_allocated && !reg.y_array.is_empty() {
        let pp = reg.y_array.last_mut().unwrap();
        let tmplen = pp.size + plen;
        let mut tmp = vec![0u8; tmplen + 1];
        tmp[..pp.size].copy_from_slice(pp.as_bytes());
        tmp[pp.size..tmplen].copy_from_slice(p_bytes);
        api_clear_string(pp);
        *pp = cbuf_to_string(&tmp[..tmplen]);
    } else {
        *reg = EMPTY_YANKREG;
        reg.additional_data = None;
        reg.y_array = vec![cbuf_to_string(p_bytes)];
        reg.y_allocated = true;
        reg.y_type = MotionType::MtCharWise;
    }
    reg.timestamp = os_realtime() as u64;
    true
}

pub fn do_record(c: i32) -> bool {
    let retval: bool;

    if g_st().reg_recording == 0 {
        if c < 0 || (!(c <= 0x7f && (c as u8).is_ascii_alphanumeric()) && c != i32::from(b'"')) {
            retval = false;
        } else {
            g_st().reg_recording = c;
            showmode();
            reg_st().do_record_regname = c;
            retval = true;
            apply_autocmds(EVENT_RECORDINGENTER, None, None, false, curbuf_opt_mut());
        }
    } else {
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let dict = get_v_event(&mut save_v_event);

        let mut p_vec = get_recorded().into_bytes_with_nul();
        vim_unescape_ks(&mut p_vec);
        tv_dict_add_str(
            &mut *dict,
            c"regcontents",
            Some(CStr::from_bytes_until_nul(&p_vec).unwrap_or_default()),
        );

        let mut buf = [0u8; NUMBUFLEN + 2];
        buf[0] = reg_st().do_record_regname as u8;
        buf[1] = 0;
        tv_dict_add_str(
            &mut *dict,
            c"regname",
            Some(CStr::from_bytes_until_nul(&buf).unwrap_or_default()),
        );
        tv_dict_set_keys_readonly(&mut *dict);

        apply_autocmds(EVENT_RECORDINGLEAVE, None, None, false, curbuf_opt_mut());
        restore_v_event(&mut *dict, &mut save_v_event);
        g_st().reg_recorded = g_st().reg_recording;
        g_st().reg_recording = 0;
        if opt_vars().p_ch == 0 || ui_has(K_UIMESSAGES) {
            showmode();
        } else {
            msg(c"", 0);
        }
        let old_y_previous = reg_st().y_previous;
        retval = stuff_yank(reg_st().do_record_regname, &p_vec);
        reg_st().y_previous = old_y_previous;
    }
    retval
}

fn put_in_typebuf(s: &CStr, esc: bool, colon: bool, is_silent: bool) -> bool {
    let mut retval = true;

    put_reedit_in_typebuf(is_silent);
    if colon {
        retval = ins_typebuf(c"\n", REMAP_NONE, 0, true, is_silent);
    }
    if retval {
        let owned_p = if esc {
            Some(vim_strsave_escape_ks(s.to_bytes()))
        } else {
            None
        };
        let p_cstr = owned_p.as_deref().unwrap_or(s);
        retval = ins_typebuf(
            p_cstr,
            if esc { REMAP_NONE } else { REMAP_YES },
            0,
            true,
            is_silent,
        );
    }
    if colon && retval {
        retval = ins_typebuf(c":", REMAP_NONE, 0, true, is_silent);
    }
    retval
}

fn put_reedit_in_typebuf(silent: bool) {
    if g_st().restart_edit == 0 {
        return;
    }

    let mut buf: [u8; 11] = [
        K_SPECIAL, KS_EXTRA, KE_COMMAND, b's', b't', b'a', b'r', b't', b'i', CAR, 0,
    ];
    if g_st().restart_edit == i32::from(b'R') {
        buf[8] = b'r';
    } else if g_st().restart_edit == i32::from(b'V') {
        buf[8] = b'g';
    } else if g_st().restart_edit == i32::from(b'A') {
        buf[8] = b'!';
    }
    if ins_typebuf(
        CStr::from_bytes_until_nul(&buf).unwrap_or_default(),
        REMAP_NONE,
        0,
        true,
        silent,
    ) {
        g_st().restart_edit = 0;
    }
}

fn execreg_line_continuation(lines: &[NvimString], idx: &mut usize) -> Vec<u8> {
    let mut cmd_start = *idx;
    assert!(cmd_start > 0);
    let cmd_end = cmd_start;

    let mut buf: Vec<u8> = Vec::with_capacity(400);

    while cmd_start > 0 {
        cmd_start -= 1;
        let p_bytes = crate::charset::skipwhite_slice(lines[cmd_start].as_bytes());
        if p_bytes.first() != Some(&b'\\') && !p_bytes.starts_with(b"\"\\ ") {
            break;
        }
    }

    let first_line = lines[cmd_start];
    if !first_line.data.is_null() && first_line.size > 0 {
        buf.extend_from_slice(first_line.as_bytes());
    }
    for item in &lines[(cmd_start + 1)..=cmd_end] {
        let p_bytes = crate::charset::skipwhite_slice(item.as_bytes());
        if let Some(rest) = p_bytes.strip_prefix(b"\\") {
            buf.extend_from_slice(rest);
        }
    }
    buf.push(0);

    *idx = cmd_start;
    buf
}

pub fn do_execreg(mut regname: i32, colon: bool, addcr: bool, silent: bool) -> bool {
    let mut retval = true;

    if regname == i32::from(b'@') {
        if reg_st().execreg_lastc == 0 {
            emsg(gettext(c"E748: No previously used register"));
            return false;
        }
        regname = reg_st().execreg_lastc;
    }
    if regname == i32::from(b'%') || regname == i32::from(b'#') || !valid_yank_reg(regname, false) {
        emsg_invreg(regname);
        return false;
    }
    reg_st().execreg_lastc = regname;

    if regname == i32::from(b'_') {
        return true;
    }

    if regname == i32::from(b':') {
        let Some(last_cmd) = g_st().borrow_mut().last_cmdline.as_ref() else {
            emsg(gettext(E_NOLASTCMD));
            return false;
        };
        g_st().new_last_cmdline = None;
        let p = vim_strsave_escaped_ext(
            last_cmd.as_bytes(),
            b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f",
            CTRL_V_CONST as u8,
            false,
        );
        if g_st().visual_global.active && p.as_bytes().starts_with(b"'<,'>") {
            retval = put_in_typebuf(
                crate::strings::ByteSlice::from_ptr(p.as_ptr().wrapping_add(5)).as_cstr(),
                true,
                true,
                silent,
            );
        } else {
            retval = put_in_typebuf(&p, true, true, silent);
        }
    } else if regname == i32::from(b'=') {
        let Some(p) = get_expr_line() else {
            return false;
        };
        retval = put_in_typebuf(&p, true, colon, silent);
    } else if regname == i32::from(b'.') {
        let Some(p) = get_last_insert_save() else {
            emsg(gettext(E_NOINSTEXT));
            return false;
        };
        retval = put_in_typebuf(&p, false, colon, silent);
    } else {
        let reg = &*get_yank_register(regname, YankRegMode::Paste);
        if !reg.y_allocated {
            return false;
        }

        let remap = if colon { REMAP_NONE } else { REMAP_YES };
        let is_silent = silent;

        put_reedit_in_typebuf(silent);
        let mut i = reg.y_array.len();
        while i > 0 {
            i -= 1;
            if (reg.y_type == MotionType::MtLineWise || i < reg.y_array.len() - 1 || addcr)
                && !ins_typebuf(c"\n", remap, 0, true, is_silent)
            {
                return false;
            }

            let mut str_ = reg.y_array[i].data;
            let cont_vec: Vec<u8>;
            if colon && i > 0 {
                let p = str_.skipwhite();
                let p_bytes = p.as_bytes();
                if p_bytes.first() == Some(&b'\\') || p_bytes.starts_with(b"\"\\ ") {
                    cont_vec = execreg_line_continuation(&reg.y_array, &mut i);
                    str_ = crate::strings::ByteSlice::from_bytes(&cont_vec);
                }
            }
            let escaped = vim_strsave_escape_ks(str_.as_bytes());
            retval = ins_typebuf(&escaped, remap, 0, true, is_silent);
            drop(escaped);
            if !retval {
                return false;
            }
            if colon && !ins_typebuf(c":", remap, 0, true, is_silent) {
                return false;
            }
        }
        g_st().reg_executing = if regname == 0 {
            i32::from(b'"')
        } else {
            regname
        };
        g_st().pending_end_reg_executing = false;
    }
    retval
}

pub fn insert_reg(regname: i32, reg: Option<&Yankreg>, literally_arg: bool) -> bool {
    let mut retval = true;
    let literally = literally_arg || is_literal_register(regname);

    os_breakcheck();
    if g_st().got_int {
        return false;
    }

    if regname != 0 && !valid_yank_reg(regname, false) {
        return false;
    }

    if regname == i32::from(b'.') {
        retval = stuff_inserted(0, 1, 1);
    } else if let Some(spec) = get_spec_reg(regname, true) {
        let Some(arg) = spec else {
            return false;
        };
        stuffescaped(crate::memory::bytes_until_nul(&arg), literally);
    } else {
        let reg_ref: &Yankreg = if let Some(r) = reg {
            r
        } else {
            get_yank_register(regname, YankRegMode::Paste)
        };
        if !reg_ref.y_allocated {
            retval = false;
        } else {
            let put = reg_ref.y_type == MotionType::MtCharWise
                && (regname == i32::from(b'-')
                    || (mc_ins_cascading()
                        && reg_ref.y_array.len() == 1
                        && (((0..=0x7f).contains(&regname)
                            && (regname as u8).is_ascii_alphanumeric())
                            || regname == i32::from(b'"'))));
            for i in 0..reg_ref.y_array.len() {
                if put {
                    let mut dir = BACKWARD;
                    if (g_st().state_global & REPLACE_FLAG) != 0 {
                        if !u_save_cursor() {
                            return false;
                        }
                        del_chars(mb_charlen(reg_ref.y_array[0].as_bytes()), 1);
                        let curpos = curwin_mut().w_cursor;
                        if !oneright() {
                            dir = FORWARD;
                        }
                        curwin_mut().w_cursor = curpos;
                    }

                    redo_append_char(CTRL_R_CONST);
                    redo_append_char(regname);
                    do_put(regname, None, dir, 1, PUT_CURSEND);
                } else {
                    stuffescaped(reg_ref.y_array[i].as_bytes(), literally);
                    if reg_ref.y_type == MotionType::MtLineWise || i < reg_ref.y_array.len() - 1 {
                        stuffchar_readbuff(i32::from(b'\n'));
                    }
                }
            }
        }
    }

    retval
}

pub fn get_spec_reg(regname: i32, errmsg: bool) -> Option<Option<Vec<u8>>> {
    let cstr_to_vec =
        |cstr: Option<std::ffi::CString>| cstr.map(std::ffi::CString::into_bytes_with_nul);
    let ptr_to_vec = |p: *const std::ffi::c_char| {
        (!p.is_null()).then(|| {
            crate::strings::ByteSlice::from_ptr(p)
                .as_cstr()
                .to_bytes_with_nul()
                .to_vec()
        })
    };
    match regname {
        x if x == i32::from(b'%') => {
            if errmsg {
                check_fname();
            }
            Some(
                curbuf_mut()
                    .fname_opt()
                    .map(|s| s.to_bytes_with_nul().to_vec()),
            )
        }
        x if x == i32::from(b'#') => {
            Some(getaltfname(errmsg).map(|s| s.to_bytes_with_nul().to_vec()))
        }
        x if x == i32::from(b'=') => Some(cstr_to_vec(get_expr_line())),
        x if x == i32::from(b':') => {
            let last_cmd = g_st()
                .last_cmdline
                .as_ref()
                .map(|s| s.as_bytes_with_nul().to_vec());
            if last_cmd.is_none() && errmsg {
                emsg(gettext(E_NOLASTCMD));
            }
            Some(last_cmd)
        }
        x if x == i32::from(b'/') => {
            if last_search_pat().is_none() && errmsg {
                emsg(gettext(E_NOPREVRE));
            }
            Some(last_search_pat().map(|s| s.to_bytes_with_nul().to_vec()))
        }
        x if x == i32::from(b'.') => {
            let v = cstr_to_vec(get_last_insert_save());
            if v.is_none() && errmsg {
                emsg(gettext(E_NOINSTEXT));
            }
            Some(v)
        }
        CTRL_F_CONST | CTRL_P => {
            if !errmsg {
                return None;
            }
            Some(cstr_to_vec(file_name_at_cursor(
                FNAME_MESS | FNAME_HYP | if regname == CTRL_P { FNAME_EXP } else { 0 },
                1,
                None,
            )))
        }
        CTRL_W_CONST | CTRL_A => {
            if !errmsg {
                return None;
            }
            let ident = find_ident_under_cursor(
                if regname == CTRL_W_CONST {
                    FIND_IDENT | FIND_STRING
                } else {
                    FIND_STRING
                },
                None,
            );
            if !ident.is_empty() {
                Some(cstr_to_vec(Some(crate::memory::bytes_to_cstring(ident))))
            } else {
                Some(None)
            }
        }
        CTRL_L_CONST => {
            if !errmsg {
                return None;
            }
            Some(ptr_to_vec(
                (ml_get_buf(curwin_mut().buf_mut(), curwin_mut().w_cursor.lnum)).as_ptr(),
            ))
        }
        x if x == i32::from(b'_') => Some(Some(vec![0])),
        _ => None,
    }
}

pub fn cmdline_paste_reg(regname: i32, literally_arg: bool, remcr: bool) -> bool {
    let literally = literally_arg || is_literal_register(regname);

    let reg = &*get_yank_register(regname, YankRegMode::Paste);
    if !reg.y_allocated {
        return false;
    }

    for i in 0..reg.y_array.len() {
        cmdline_paste_str(reg.y_array[i].as_bytes(), literally);

        if i < reg.y_array.len() - 1 && !remcr {
            cmdline_paste_str(b"\r", literally);
        }

        os_breakcheck();
        if g_st().got_int {
            return false;
        }
    }
    true
}

pub fn shift_delete_registers(y_append: bool) {
    let regs = &mut reg_st().y_regs;
    regs[9] = EMPTY_YANKREG;
    let mut n = 9;
    while n > 1 {
        regs[n] = std::mem::replace(&mut regs[n - 1], EMPTY_YANKREG);
        n -= 1;
    }
    if !y_append {
        reg_st().y_previous = crate::globals::GraphPtr::from(&mut regs[1]);
    }
}

pub fn clear_registers() {
    reg_st().y_regs.fill_with(|| EMPTY_YANKREG);
}

fn yank_copy_line(
    reg: &mut Yankreg,
    bd: &mut BlockDef,
    y_idx: usize,
    exclude_trailing_space: bool,
) {
    if exclude_trailing_space {
        bd.endspaces = 0;
    }
    let size = bd.startspaces + bd.endspaces + bd.textlen;
    assert!(size >= 0);
    let start_usize = bd.startspaces as usize;
    let text_usize = bd.textlen as usize;
    let end_usize = bd.endspaces as usize;
    let mut buf_vec = vec![0u8; (size as usize) + 1];
    let text_ptr = crate::strings::ByteSlice::from_cstr(bd.textstart);
    buf_vec[..start_usize].fill(b' ');
    buf_vec[start_usize..start_usize + text_usize].copy_from_slice(text_ptr.slice(text_usize));
    buf_vec[start_usize + text_usize..start_usize + text_usize + end_usize].fill(b' ');
    let mut out_size = start_usize + text_usize + end_usize;
    if exclude_trailing_space {
        let mut s = bd.textlen + bd.endspaces;
        while s > 0 && ascii_iswhite(text_ptr.wrapping_add((s - 1) as usize).first_byte()) {
            s = s
                - crate::mbyte::utf_head_off_ptr(text_ptr, text_ptr.wrapping_add((s - 1) as usize))
                - 1;
            out_size -= 1;
        }
    }
    buf_vec[out_size] = 0;
    reg.y_array[y_idx] = cbuf_to_string(crate::strings::AsByteSlice::slice_len(
        buf_vec.as_ptr(),
        out_size,
    ));
}

pub fn op_yank_reg(oap: &mut Oparg, message: bool, reg_in: &mut Yankreg, append: bool) {
    let mut reg = crate::globals::GraphPtr::<Yankreg>::of(reg_in).as_ptr();
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
    if append && crate::globals::GraphPtr::of(reg).borrow().y_allocated {
        reg = &raw mut newreg;
    } else {
        *crate::globals::GraphPtr::of(reg).borrow_mut() = EMPTY_YANKREG;
    }

    if orig_motion == MotionType::MtCharWise
        && oap.start.col == 0
        && !oap.inclusive
        && (!oap.is_visual || opt_vars().p_sel.first_byte() == b'o')
        && oap.end.col == 0
        && yanklines > 1
    {
        yank_type = MotionType::MtLineWise;
        yankendlnum -= 1;
        yanklines -= 1;
    }

    let reg_mut = crate::globals::GraphPtr::of(reg).borrow_mut();
    reg_mut.y_type = yank_type;
    reg_mut.y_width = 0;
    reg_mut.y_array = vec![NvimString::new(); yanklines];
    reg_mut.y_allocated = true;
    reg_mut.additional_data = None;
    reg_mut.timestamp = os_realtime() as u64;

    let mut y_idx: usize = 0;
    let mut lnum = oap.start.lnum;

    if yank_type == MotionType::MtBlockWise {
        reg_mut.y_width = oap.end_vcol - oap.start_vcol;

        if curwin_mut().w_curswant == MAXCOL && reg_mut.y_width > 0 {
            reg_mut.y_width -= 1;
        }
    }

    while lnum <= yankendlnum {
        let reg_mut = crate::globals::GraphPtr::of(reg).borrow_mut();
        match reg_mut.y_type {
            MotionType::MtBlockWise => {
                block_prep(oap, &mut bd, lnum, false);

                yank_copy_line(reg_mut, &mut bd, y_idx, oap.excl_tr_ws);
            }
            MotionType::MtLineWise => {
                let line_slice = crate::memline::ml_get_slice(lnum);

                reg_mut.y_array[y_idx] = cbuf_to_string(crate::strings::AsByteSlice::slice_len(
                    line_slice.as_ptr(),
                    line_slice.len(),
                ));
            }
            MotionType::MtCharWise => {
                charwise_block_prep(oap.start, oap.end, &mut bd, lnum, oap.inclusive);
                let tmp = bd.textstart.to_bytes().len() as i32;
                if tmp < bd.textlen {
                    bd.textlen = tmp;
                }

                yank_copy_line(reg_mut, &mut bd, y_idx, false);
            }
            MotionType::MtUnknown => std::process::abort(),
        }
        lnum += 1;
        y_idx += 1;
    }

    if !ptr::eq(curr, reg) {
        let curr_reg = crate::globals::GraphPtr::of(curr).borrow_mut();
        let new_reg = crate::globals::GraphPtr::of(reg).borrow_mut();
        if yank_type == MotionType::MtLineWise {
            curr_reg.y_type = MotionType::MtLineWise;
        }

        if curr_reg.y_type == MotionType::MtCharWise
            && !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_REGAPPEND)
            && !curr_reg.y_array.is_empty()
            && !new_reg.y_array.is_empty()
        {
            let last_old = *curr_reg.y_array.last().unwrap();
            let first_new = new_reg.y_array[0];
            let mut pnew = vec![0u8; last_old.size + first_new.size + 1];

            crate::globals::GraphPtr::copy_to(
                last_old.data.cast::<u8>(),
                pnew.as_mut_ptr(),
                last_old.size,
            );

            crate::globals::GraphPtr::copy_to(
                first_new.data.cast::<u8>(),
                pnew.as_mut_ptr().wrapping_add(last_old.size),
                first_new.size + 1,
            );

            let last_slot = curr_reg.y_array.last_mut().unwrap();
            api_clear_string(last_slot);
            *last_slot = cbuf_to_string(crate::strings::AsByteSlice::slice_len(
                pnew.as_ptr(),
                last_old.size + first_new.size,
            ));
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
            let mut namebuf = [0u8; 100];
            if oap.regname == 0 {
                namebuf[0] = 0;
            } else {
                format_to_buf!(&mut namebuf, gettext(c" into \"%c"), oap.regname);
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

            let name_cstr = CStr::from_bytes_until_nul(&namebuf).unwrap_or_default();
            if yank_type == MotionType::MtBlockWise {
                smsg!(
                    0,
                    ngettext(BLOCK_SINGLE, BLOCK_PLURAL, yanklines as u64,),
                    yanklines as i64,
                    name_cstr,
                );
            } else {
                smsg!(
                    0,
                    ngettext(LINE_SINGLE, LINE_PLURAL, yanklines as u64,),
                    yanklines as i64,
                    name_cstr,
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

pub fn format_reg_type(reg_type: MotionType, reg_width: Colnr, buf: &mut [u8]) -> usize {
    assert!(buf.len() > 1);
    match reg_type {
        MotionType::MtLineWise => {
            buf[0] = b'V';
            buf[1] = 0;
            1
        }
        MotionType::MtCharWise => {
            buf[0] = b'v';
            buf[1] = 0;
            1
        }
        MotionType::MtBlockWise => format_to_buf_safelen!(buf, c"\x16%d", reg_width + 1),
        MotionType::MtUnknown => {
            buf[0] = 0;
            0
        }
    }
}

fn add_regtype_to_dict(reg: Option<&Yankreg>, dict: &mut TvDict, buf: &mut [u8]) {
    let len = format_reg_type(
        if let Some(reg) = reg {
            reg.y_type
        } else {
            MotionType::MtCharWise
        },
        if let Some(reg) = reg { reg.y_width } else { 0 },
        buf,
    );
    tv_dict_add_str_len(dict, c"regtype", Some(&buf[..len]));
}

pub fn do_autocmd_textyankpost(oap: &mut Oparg, reg: &mut Yankreg) {
    if reg_st().textyankpost_recursive || !has_event(EVENT_TEXTYANKPOST) {
        return;
    }

    reg_st().textyankpost_recursive = true;

    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let dict = get_v_event(&mut save_v_event);

    let list = tv_list_alloc(reg.y_array.len() as isize);
    for &item in &reg.y_array {
        crate::eval::typval::tv_list_append_bytes(&mut *list, item.as_bytes());
    }
    list.lv_lock = VAR_FIXED;
    tv_dict_add_list(&mut *dict, c"regcontents", Some(list));

    let mut buf = [0u8; NUMBUFLEN + 2];
    add_regtype_to_dict(Some(reg), &mut *dict, &mut buf);

    buf[0] = oap.regname as u8;
    buf[1] = 0;
    tv_dict_add_str(
        &mut *dict,
        c"regname",
        Some(CStr::from_bytes_until_nul(&buf).unwrap_or_default()),
    );

    tv_dict_add_bool(
        &mut *dict,
        c"inclusive",
        if oap.inclusive {
            K_BOOL_VAR_TRUE_CONST
        } else {
            K_BOOL_VAR_FALSE_CONST
        },
    );

    buf[0] = get_op_char(oap.op_type) as u8;
    buf[1] = 0;
    tv_dict_add_str(
        &mut *dict,
        c"operator",
        Some(CStr::from_bytes_until_nul(&buf).unwrap_or_default()),
    );

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
    apply_autocmds(EVENT_TEXTYANKPOST, None, None, false, curbuf_opt_mut());
    g_st().textlock -= 1;
    restore_v_event(&mut *dict, &mut save_v_event);

    reg_st().textyankpost_recursive = false;
}

fn put_do_autocmd(
    regname: i32,
    reg: Option<&Yankreg>,
    insert: Option<&NvimString>,
    post: bool,
    dir: Direction,
) {
    if reg_st().put_autocmd_recursive || (regname == i32::from(b'_') && reg.is_none()) {
        return;
    }

    if regname != i32::from(b'.') && insert.is_none() && reg.is_none() {
        return;
    }

    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let v_event = get_v_event(&mut save_v_event);

    let list = tv_list_alloc(reg.map_or(1, |reg| reg.y_array.len() as isize));

    if regname == i32::from(b'.') {
        if !g_st().last_insert_ga.is_empty() {
            crate::eval::typval::tv_list_append_bytes(&mut *list, &g_st().last_insert_ga);
        }
    } else if let Some(insert) = insert {
        crate::eval::typval::tv_list_append_bytes(&mut *list, insert.as_bytes());
    } else {
        for &item in &(reg.unwrap()).y_array {
            crate::eval::typval::tv_list_append_bytes(&mut *list, item.as_bytes());
        }
    }

    list.lv_lock = VAR_FIXED;
    tv_dict_add_list(&mut *v_event, c"regcontents", Some(list));

    let mut buf = [0u8; NUMBUFLEN + 2];
    buf[0] = regname as u8;
    buf[1] = 0;
    let buflen = if buf[0] == 0 { 0 } else { 1 };
    tv_dict_add_str_len(&mut *v_event, c"regname", Some(&buf[..buflen]));

    buf[0] = if dir == BACKWARD { b'P' } else { b'p' };
    buf[1] = 0;
    tv_dict_add_str_len(&mut *v_event, c"operator", Some(&buf[..1]));

    add_regtype_to_dict(reg, &mut *v_event, &mut buf);

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
        apply_autocmds(EVENT_TEXTPUTPOST, None, None, false, curbuf_opt_mut());
    } else {
        apply_autocmds(EVENT_TEXTPUTPRE, None, None, false, curbuf_opt_mut());
    }
    g_st().textlock -= 1;
    reg_st().put_autocmd_recursive = false;

    restore_v_event(&mut *v_event, &mut save_v_event);
}

pub fn op_yank(oap: &mut Oparg, message: bool) -> bool {
    if oap.regname != 0 && !valid_yank_reg(oap.regname, true) {
        beep_flush();
        return false;
    }
    if oap.regname == i32::from(b'_') {
        return true;
    }

    let reg = get_yank_register(oap.regname, YankRegMode::Yank);
    op_yank_reg(oap, message, reg, is_append_register(oap.regname));
    set_clipboard(oap.regname, &*reg);
    do_autocmd_textyankpost(oap, &mut *reg);
    true
}

pub fn do_put(
    regname: i32,
    reg_in: Option<&mut Yankreg>,
    mut dir: i32,
    mut count: i32,
    flags: i32,
) {
    let mut reg = crate::globals::GraphPtr::<Yankreg>::of(reg_in);
    let mut totlen: usize = 0;
    let mut lnum: Linenr;
    let mut y_type: MotionType;
    let y_size: usize;
    let mut y_width: i32 = 0;
    let mut vcol: Colnr = 0;
    let y_array;
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

    if regname == i32::from(b'.') && reg.is_null() {
        let non_linewise_vis =
            g_st().visual_global.active && g_st().visual_global.mode != i32::from(b'V');

        let command_start_char = if non_linewise_vis {
            i32::from(b'c')
        } else if (flags & PUT_LINE) != 0 {
            i32::from(b'i')
        } else if dir == FORWARD {
            i32::from(b'a')
        } else {
            i32::from(b'i')
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
                stuff_readbuff_len(b"put _");
            } else {
                stuff_readbuff_len(b"put! _");
            }
            stuffchar_readbuff(i32::from(CAR));
            g_st().add_last_insert = save_add_last_insert;
        }

        if (flags & PUT_LINE) != 0 {
            stuffchar_readbuff(command_start_char);
            while count > 0 {
                stuff_inserted(0, 1, i32::from(count != 1));
                if count != 1 {
                    let s: [u8; 3] = [b'\n', b' ', CTRL_U_CONST];
                    stuff_readbuff_len(&s);
                }
                count -= 1;
            }
        } else {
            stuff_inserted(command_start_char, count, 0);
        }

        if has_event(EVENT_TEXTPUTPRE) {
            put_do_autocmd(i32::from(b'.'), None, None, false, dir);
        }
        if has_event(EVENT_TEXTPUTPOST) {
            put_do_autocmd(i32::from(b'.'), None, None, true, dir);
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
                let cursor_pos = crate::cursor::get_cursor_pos_slice();
                let one_past_line = cursor_pos.is_empty();
                let mut eol = false;
                if !one_past_line {
                    let clen = crate::mbyte::utfc_slice2len(cursor_pos);
                    eol = cursor_pos[clen..].is_empty();
                }

                let ve_allows =
                    cur_ve_flags == K_OPT_VE_FLAG_ALL || cur_ve_flags == K_OPT_VE_FLAG_ONEMORE;
                let eof =
                    curbuf_mut().b_ml.ml_line_count == curwin_mut().w_cursor.lnum && one_past_line;
                if ve_allows || !(eol || eof) {
                    stuffchar_readbuff(i32::from(b'l'));
                }
            }
        } else if (flags & PUT_LINE) != 0 {
            stuff_readbuff(c"g'[");
        }

        if command_start_char == i32::from(b'a')
            && !u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1)
        {
            return;
        }

        exec_stuffed(None);
        return;
    }

    let mut insert_string = NvimString::new();
    let mut insert_string_owned: Option<Vec<u8>> = None;
    if reg.is_null()
        && let Some(spec) = get_spec_reg(regname, true)
    {
        let Some(spec_vec) = spec else {
            return;
        };
        insert_string.data = crate::strings::ByteSlice::from_bytes(&spec_vec);
        insert_string_owned = Some(spec_vec);
    }

    if curbuf_mut().terminal.is_null()
        && !u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1)
    {
        return;
    }

    if !insert_string.data.is_null() {
        insert_string.size = insert_string.cstr_bytes().len();
        y_type = MotionType::MtCharWise;
        if regname == i32::from(b'=') {
            let mut ptr_ = insert_string.data.as_ptr();
            let mut ptrlen = insert_string.size;
            while !ptr_.is_null() {
                let mut tmp = vim_strchr(
                    crate::strings::ByteSlice::from_ptr(ptr_).as_cstr(),
                    i32::from(b'\n'),
                )
                .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut());
                if tmp.is_null() {
                    expr_y_array.push(NvimString {
                        data: crate::strings::ByteSlice::from_ptr(ptr_),
                        size: ptrlen,
                    });
                } else {
                    *crate::globals::GraphPtr::of(tmp).borrow_mut() = NUL as _;

                    let item_size = (tmp as usize) - (ptr_ as usize);
                    expr_y_array.push(NvimString {
                        data: crate::strings::ByteSlice::from_ptr(ptr_),
                        size: item_size,
                    });
                    ptrlen -= item_size + 1;
                    tmp = tmp.wrapping_add(1);
                    if crate::strings::AsByteSlice::is_empty_val(tmp) {
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
                reg = crate::globals::GraphPtr::<Yankreg>::of(get_yank_register(
                    regname,
                    YankRegMode::Paste,
                ));
            }

            put_do_autocmd(regname, reg.opt(), None, false, dir);

            reg = crate::globals::GraphPtr::<Yankreg>::of(save_reg);
        }
        if reg.is_null() {
            reg = crate::globals::GraphPtr::<Yankreg>::of(get_yank_register(
                regname,
                YankRegMode::Paste,
            ));
        }

        let reg_ref = reg.borrow_mut();
        y_type = reg_ref.y_type;
        y_width = reg_ref.y_width;
        y_size = reg_ref.y_array.len();
        y_allocated = reg_ref.y_allocated;
        y_array = reg_ref.y_array.as_mut_ptr();
    }

    'end: {
        if !curbuf_mut().terminal.is_null() {
            let y_slice = if y_size == 0 {
                &[][..]
            } else {
                crate::globals::GraphPtr::raw_slice(y_array, y_size)
            };
            terminal_paste(count, y_slice);
            break 'end;
        }

        let mut split_pos: Colnr = 0;
        if y_type == MotionType::MtLineWise {
            if (flags & PUT_LINE_SPLIT) != 0 {
                if !u_save_cursor() {
                    break 'end;
                }
                let curline = get_cursor_line_ptr().as_ptr().cast_mut();
                let mut p = get_cursor_pos_ptr().as_ptr().cast_mut();
                let p_orig = p;
                let plen = get_cursor_pos_len() as usize;
                if dir == FORWARD && plen > 0 {
                    p = p.wrapping_add(utfc_ptr2len(p) as usize);
                }
                split_pos = ((p as usize) - (curline as usize)) as Colnr;

                let tail_len = plen - ((p as usize) - (p_orig as usize));
                let mut tail_vec: Vec<u8> = Vec::with_capacity(tail_len + 1);

                tail_vec.extend_from_slice(crate::globals::GraphPtr::raw_slice(
                    p.cast::<u8>(),
                    tail_len,
                ));

                tail_vec.push(0);
                ml_append(
                    curwin_mut().w_cursor.lnum,
                    crate::memory::bytes_until_nul(&tail_vec),
                    0,
                    false,
                );

                crate::memline::ml_replace_len(
                    curwin_mut().w_cursor.lnum,
                    crate::strings::AsByteSlice::slice_len(
                        get_cursor_line_ptr().as_ptr().cast_mut(),
                        split_pos as usize,
                    ),
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
                    c"\""
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
                has_folding(curwin_mut(), lnum, Some(&mut lnum), None);
            } else {
                has_folding(curwin_mut(), lnum, None, Some(&mut lnum));
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
                coladvance_force(getviscol() + i32::from(dir == FORWARD));
            }
        }

        lnum = curwin_mut().w_cursor.lnum;
        let mut col = curwin_mut().w_cursor.col;

        if y_type == MotionType::MtBlockWise {
            let mut incr: i32 = 0;
            let mut bd = BlockDef::default();
            let c = gchar_cursor();
            let mut endcol2: Colnr = 0;

            if dir == FORWARD && c != 0 {
                let mut cursor = curwin_mut().w_cursor;
                if cur_ve_flags == K_OPT_VE_FLAG_ALL {
                    getvcol(
                        curwin_mut(),
                        &mut cursor,
                        Some(&mut col),
                        None,
                        Some(&mut endcol2),
                        0,
                    );
                } else {
                    getvcol(curwin_mut(), &mut cursor, None, None, Some(&mut col), 0);
                }
                curwin_mut().w_cursor = cursor;
                curwin_mut().w_cursor.col +=
                    crate::mbyte::utfc_slice2len(crate::cursor::get_cursor_pos_slice()) as Colnr;
                col += 1;
            } else {
                let mut cursor = curwin_mut().w_cursor;
                getvcol(
                    curwin_mut(),
                    &mut cursor,
                    Some(&mut col),
                    None,
                    Some(&mut endcol2),
                    0,
                );
                curwin_mut().w_cursor = cursor;
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
                let mut spaces: i32 = 0;
                let mut lines_appended: i32 = 0;

                bd.startspaces = 0;
                bd.endspaces = 0;
                let mut delcount: i32 = 0;

                if curwin_mut().w_cursor.lnum > curbuf_mut().b_ml.ml_line_count {
                    if !ml_append(curbuf_mut().b_ml.ml_line_count, b"", 1, false) {
                        break;
                    }
                    nr_lines += 1;
                    lines_appended = 1;
                }
                let oldp = get_cursor_line_ptr().as_ptr().cast_mut();
                let oldlen = get_cursor_line_len();

                let mut csarg = CharsizeArg::new();
                let mut cstype: CSType = init_charsize_arg(
                    &mut csarg,
                    curwin_mut(),
                    curwin_mut().w_cursor.lnum,
                    crate::strings::ByteSlice::from_ptr(oldp).as_cstr(),
                );
                let oldp_cstr = crate::strings::ByteSlice::from_ptr(oldp).as_cstr();
                let mut ci = utf_ptr2_str_char_info(oldp_cstr);
                vcol = 0;
                while vcol < col && !ci.ptr.is_empty() {
                    let cli = utf_cluster_info(ci);
                    incr =
                        win_charsize(cstype, vcol, ci.cstr(), ci.chr.value, &mut csarg, cli.cells)
                            .width;
                    vcol += incr;
                    ci = cli.next;
                }
                let mut ptr_ = oldp.wrapping_add(ci.offset_from(oldp_cstr));
                bd.textcol = ci.offset_from(oldp_cstr) as Colnr;

                let shortline = (vcol < col)
                    || (vcol == col && crate::strings::AsByteSlice::is_empty_val(ptr_));

                if vcol < col {
                    bd.startspaces = col - vcol;
                } else if vcol > col {
                    bd.endspaces = vcol - col;
                    bd.startspaces = incr - bd.endspaces;
                    bd.textcol -= 1;
                    delcount = 1;
                    bd.textcol -= crate::mbyte::utf_head_off_ptr(
                        oldp,
                        oldp.wrapping_add(bd.textcol as usize),
                    );
                    if *crate::globals::GraphPtr::of(oldp.wrapping_add(bd.textcol as usize))
                        .borrow() as u8
                        != TAB as u8
                    {
                        delcount = 0;
                        bd.endspaces = 0;
                    }
                }

                let yanklen = crate::globals::GraphPtr::of(y_array.wrapping_add(i))
                    .borrow()
                    .size as i32;

                if (flags & PUT_BLOCK_INNER) == 0 {
                    spaces = y_width + 1;

                    let y_data = (crate::globals::GraphPtr::of(y_array.wrapping_add(i))
                        .borrow()
                        .data)
                        .as_cstr();
                    cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, y_data);

                    ci = utf_ptr2_str_char_info(y_data);

                    while !ci.ptr.is_empty() {
                        let cli = utf_cluster_info(ci);
                        spaces -=
                            win_charsize(cstype, 0, ci.cstr(), ci.chr.value, &mut csarg, cli.cells)
                                .width;
                        ci = cli.next;
                    }
                    spaces = spaces.max(0);
                }

                if yanklen + spaces != 0
                    && count > ((i32::MAX - (bd.startspaces + bd.endspaces)) / (yanklen + spaces))
                {
                    emsg(gettext(E_RESULTING_TEXT_TOO_LONG));
                    break;
                }

                totlen = (count as usize) * ((yanklen + spaces) as usize)
                    + (bd.startspaces as usize)
                    + (bd.endspaces as usize);
                let mut newp_vec = vec![0u8; totlen + (oldlen as usize) + 1];
                let newp = newp_vec.as_mut_ptr();

                ptr_ = newp.cast();

                crate::globals::GraphPtr::copy_to(oldp, ptr_, bd.textcol as usize);

                ptr_ = ptr_.wrapping_add(bd.textcol as usize);

                crate::globals::GraphPtr::write_bytes(
                    ptr_.cast::<u8>(),
                    b' ',
                    bd.startspaces as usize,
                );

                ptr_ = ptr_.wrapping_add(bd.startspaces as usize);

                for j in 0..count {
                    crate::globals::GraphPtr::copy_to(
                        crate::globals::GraphPtr::of(y_array.wrapping_add(i))
                            .borrow()
                            .data
                            .as_ptr(),
                        ptr_,
                        yanklen as usize,
                    );

                    ptr_ = ptr_.wrapping_add(yanklen as usize);

                    if (j < count - 1 || !shortline) && spaces > 0 {
                        crate::globals::GraphPtr::write_bytes(
                            ptr_.cast::<u8>(),
                            b' ',
                            spaces as usize,
                        );

                        ptr_ = ptr_.wrapping_add(spaces as usize);
                    } else {
                        totlen -= spaces as usize;
                    }
                }

                crate::globals::GraphPtr::write_bytes(
                    ptr_.cast::<u8>(),
                    b' ',
                    bd.endspaces as usize,
                );

                ptr_ = ptr_.wrapping_add(bd.endspaces as usize);

                let columns = oldlen - bd.textcol - delcount + 1;
                assert!(columns >= 0);

                crate::globals::GraphPtr::copy_to(
                    oldp.wrapping_add((bd.textcol + delcount) as usize),
                    ptr_,
                    columns as usize,
                );

                ml_replace(
                    curwin_mut().w_cursor.lnum,
                    crate::strings::ByteSlice::from_ptr(newp).as_bytes(),
                    true,
                );
                drop(newp_vec);
                extmark_splice_cols(
                    curbuf_mut(),
                    curwin_mut().w_cursor.lnum - 1,
                    bd.textcol,
                    delcount,
                    (totlen as i32) + lines_appended,
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
            let yanklen = crate::globals::GraphPtr::of(y_array).borrow().size as i32;

            if y_type == MotionType::MtCharWise {
                if dir == FORWARD && gchar_cursor() != 0 {
                    let bytelen = crate::mbyte::utfc_slice2len(crate::cursor::get_cursor_pos_slice())
                        as Colnr;
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
                let mut first_byte_off: i32 = 0;

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
                } else if count > i32::MAX / yanklen {
                    emsg(gettext(E_RESULTING_TEXT_TOO_LONG));
                } else {
                    totlen = (count as usize) * (yanklen as usize);
                    loop {
                        let oldp = ml_get(lnum).as_ptr().cast_mut();
                        let oldlen = ml_get_len(lnum);
                        if lnum > start_lnum {
                            let mut pos = CursorPos {
                                lnum,
                                col: 0,
                                coladd: 0,
                            };
                            if getvpos(curwin_mut(), &mut pos, vcol) {
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
                        let mut newp_vec = vec![0u8; totlen + (oldlen as usize) + 1];
                        let newp = newp_vec.as_mut_ptr();

                        crate::globals::GraphPtr::copy_to(oldp.cast::<u8>(), newp, col as usize);

                        let mut ptr_ = newp.wrapping_add(col as usize);
                        for _ in 0..(count as usize) {
                            crate::globals::GraphPtr::copy_to(
                                crate::globals::GraphPtr::of(y_array)
                                    .borrow()
                                    .data
                                    .cast::<u8>(),
                                ptr_,
                                yanklen as usize,
                            );

                            ptr_ = ptr_.wrapping_add(yanklen as usize);
                        }

                        crate::globals::GraphPtr::copy_to(
                            oldp.wrapping_add(col as usize).cast::<u8>(),
                            ptr_,
                            ((oldlen - col) as usize) + 1,
                        );

                        ml_replace(
                            lnum,
                            crate::strings::ByteSlice::from_ptr(newp).as_bytes(),
                            true,
                        );

                        first_byte_off = crate::mbyte::utf_head_off_ptr(newp, ptr_.wrapping_sub(1));
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
                            totlen as i32,
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
                let mut orig_indent: i32 = 0;
                let mut indent_diff: i32 = 0;
                let mut first_indent = true;
                let mut lendiff: i32 = 0;

                if (flags & PUT_FIXINDENT) != 0 {
                    orig_indent = get_indent();
                }

                'error: {
                    for cnt in 1..=count {
                        let mut i: usize = 0;
                        if y_type == MotionType::MtCharWise {
                            lnum = new_cursor.lnum;
                            let line_slice = crate::memline::ml_get_slice(lnum);
                            let last_y =
                                crate::globals::GraphPtr::of(y_array.wrapping_add(y_size - 1))
                                    .borrow()
                                    .as_bytes();
                            let mut append_vec: Vec<u8> = Vec::with_capacity(
                                last_y.len() + line_slice.len() - col as usize + 1,
                            );
                            append_vec.extend_from_slice(last_y);
                            append_vec.extend_from_slice(&line_slice[col as usize..]);
                            append_vec.push(0);
                            ml_append(lnum, crate::memory::bytes_until_nul(&append_vec), 0, false);
                            new_lnum += 1;
                            drop(append_vec);

                            let old_slice = crate::memline::ml_get_slice(lnum);
                            let first_y = crate::globals::GraphPtr::of(y_array).borrow().as_bytes();
                            let mut newp_vec =
                                Vec::with_capacity((col as usize) + (yanklen as usize) + 1);
                            newp_vec.extend_from_slice(&old_slice[..col as usize]);
                            newp_vec.extend_from_slice(&first_y[..yanklen as usize]);
                            newp_vec.push(0);
                            ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);
                            drop(newp_vec);

                            curwin_mut().w_cursor.lnum = lnum;
                            i = 1;
                        }

                        while i < y_size {
                            if y_type != MotionType::MtCharWise || i < y_size - 1 {
                                if !ml_append(
                                    lnum,
                                    crate::globals::GraphPtr::of(y_array.wrapping_add(i))
                                        .borrow_mut()
                                        .cstr_bytes(),
                                    0,
                                    false,
                                ) {
                                    break 'error;
                                }
                                new_lnum += 1;
                            }
                            lnum += 1;
                            nr_lines += 1;
                            if (flags & PUT_FIXINDENT) != 0 {
                                let old_pos = curwin_mut().w_cursor;
                                curwin_mut().w_cursor.lnum = lnum;
                                let ptr_slice = crate::memline::ml_get_slice(lnum);
                                if cnt == count && i == y_size - 1 {
                                    lendiff = ptr_slice.len() as Colnr;
                                }
                                let indent = if (ptr_slice.first() == Some(&b'#')
                                    && preprocs_left())
                                    || ptr_slice.is_empty()
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
                        let mut lastsize: i32 = 0;
                        if y_type == MotionType::MtCharWise
                            || (y_type == MotionType::MtLineWise && (flags & PUT_LINE_SPLIT) != 0)
                        {
                            for k in 0..(y_size - 1) {
                                totsize += (crate::globals::GraphPtr::of(y_array.wrapping_add(k))
                                    .borrow()
                                    .size as Bcount)
                                    + 1;
                            }

                            lastsize =
                                crate::globals::GraphPtr::of(y_array.wrapping_add(y_size - 1))
                                    .borrow()
                                    .size as i32;

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
                                (y_size as i32) - 1,
                                lastsize,
                                totsize,
                                K_EXTMARK_UNDO,
                            );
                        } else if y_type == MotionType::MtLineWise && (flags & PUT_LINE_SPLIT) != 0
                        {
                            extmark_splice(
                                curbuf_mut(),
                                new_cursor.lnum - 1,
                                split_pos,
                                0,
                                0,
                                0,
                                (y_size as i32) + 1,
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

                let kind = if y_type == MotionType::MtLineWise && (flags & PUT_LINE_SPLIT) == 0 {
                    K_EXTMARK_UNDO
                } else {
                    K_EXTMARK_NOOP
                };
                mark_adjust(
                    curbuf_mut().b_op_start.lnum + Linenr::from(y_type == MotionType::MtCharWise),
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
                let last_item =
                    *crate::globals::GraphPtr::of(y_array.wrapping_add(y_size - 1)).borrow();
                col = ((last_item.size as Colnr) - lendiff).max(0);
                if col > 1 {
                    curbuf_mut().b_op_end.col = col - 1;
                    if last_item.size > 0 {
                        curbuf_mut().b_op_end.col -= crate::mbyte::utf_head_off_ptr(
                            last_item.data,
                            last_item.data.wrapping_add(last_item.size - 1),
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
            put_do_autocmd(regname, reg.opt(), None, true, dir);
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

fn dis_msg(p_opt: Option<&CStr>, skip_esc: bool) {
    let Some(p_cstr) = p_opt else {
        return;
    };
    let mut p = crate::strings::ByteSlice::from_cstr(p_cstr);
    let mut n = g_st().columns_global - 6;
    while !p.is_empty() && !(p.first_byte() == ESC && skip_esc && p.len() == 1) {
        n -= ptr2cells(p);
        if n < 0 {
            break;
        }
        let l = utfc_ptr2len(p);
        if l > 1 {
            msg_outtrans_len(p.slice(l as usize), l, 0, false);
            p = p.wrapping_add(l as usize);
        } else {
            msg_outtrans_len(p.slice(1), 1, 0, false);
            p = p.wrapping_add(1);
        }
    }
    os_breakcheck();
}

pub fn ex_display(eap: &mut Exarg) {
    let mut arg = eap.arg;
    if !arg.is_null() && arg.is_empty() {
        arg = crate::strings::BYTE_SLICE_NULL;
    }
    let hl_id = HLF_8;

    msg_ext_set_kind(c"list_cmd");
    g_st().msg_ext_skip_flush = true;
    msg_puts_title(gettext(c"\nType Name Content"));

    let regs = (&raw mut reg_st().y_regs).cast::<Yankreg>();
    let mut i: i32 = -1;
    while i < (NUM_REGISTERS as i32) && !g_st().got_int {
        let name = get_register_name(i);
        if !arg.is_null() && !crate::strings::vim_haschr(arg.as_cstr(), name) {
            i += 1;
            continue;
        }

        let type_ch = match get_reg_type(name, None) {
            MotionType::MtLineWise => i32::from(b'l'),
            MotionType::MtCharWise => i32::from(b'c'),
            _ => i32::from(b'b'),
        };

        let mut yb = if i == -1 {
            if !reg_st().y_previous.is_null() {
                reg_st().y_previous.as_ptr()
            } else {
                regs
            }
        } else {
            regs.wrapping_add(i as usize)
        };

        let mut cb_target: Option<&'static mut Yankreg> = None;
        get_clipboard(name, &mut cb_target, true);
        if let Some(cb_reg) = cb_target {
            yb = cb_reg;
        }

        if name == mb_tolower(g_st().redir_reg)
            || (g_st().redir_reg == i32::from(b'"') && reg_st().y_previous == yb)
        {
            i += 1;
            continue;
        }

        let yb_ref = crate::globals::GraphPtr::of(yb).borrow();
        if yb_ref.y_allocated {
            let mut do_show = false;
            for item in &yb_ref.y_array {
                if !message_filtered(item.as_cstr()) {
                    do_show = true;
                    break;
                }
            }

            if do_show || yb_ref.y_array.is_empty() {
                msg_putchar(i32::from(b'\n'));
                msg_puts(c"  ");
                msg_putchar(type_ch);
                msg_puts(c"  ");
                msg_putchar(i32::from(b'"'));
                msg_putchar(name);
                msg_puts(c"   ");

                let mut n = g_st().columns_global - 11;
                let mut j: usize = 0;
                while j < yb_ref.y_array.len() && n > 1 {
                    if j != 0 {
                        msg_puts_hl(c"^J", hl_id, false);
                        n -= 2;
                    }
                    let mut p = yb_ref.y_array[j].data;
                    while !p.is_empty() {
                        n -= ptr2cells(p);
                        if n < 0 {
                            break;
                        }
                        let clen = utfc_ptr2len(p);
                        msg_outtrans_len(p.slice(clen.max(0) as usize), clen, 0, false);
                        p = p.wrapping_add(clen as usize);
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
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'.')))
        && !g_st().got_int
        && !message_filtered(p.as_cstr())
    {
        msg_puts(c"\n  c  \".   ");
        dis_msg(p.as_opt_cstr(), true);
    }

    if let Some(last_cmd) = g_st().last_cmdline.as_ref()
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b':')))
        && !g_st().got_int
        && !message_filtered(last_cmd)
    {
        msg_puts(c"\n  c  \":   ");
        dis_msg(Some(last_cmd), false);
    }

    if curbuf_mut().fname_opt().is_some()
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'%')))
        && !g_st().got_int
        && !message_filtered(curbuf_mut().fname_cstr())
    {
        msg_puts(c"\n  c  \"%   ");
        dis_msg(curbuf_mut().fname_opt(), false);
    }

    if (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'#')))
        && !g_st().got_int
    {
        let mut fname = None;
        let mut dummy: Linenr = 0;
        if buflist_name_nr(0, &mut fname, &mut dummy) && !message_filtered(fname.unwrap_or(c"")) {
            msg_puts(c"\n  c  \"#   ");
            dis_msg(fname, false);
        }
    }

    if last_search_pat().is_some()
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'/')))
        && !g_st().got_int
        && !message_filtered(last_search_pat().unwrap_or(c""))
    {
        msg_puts(c"\n  c  \"/   ");
        dis_msg(last_search_pat(), false);
    }

    if let Some(el) = &reg_st().expr_line
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'=')))
        && !g_st().got_int
        && !message_filtered(el)
    {
        msg_puts(c"\n  c  \"=   ");
        dis_msg(Some(el), false);
    }
    g_st().msg_ext_skip_flush = false;
}

pub fn get_reg_type(regname: i32, reg_width: Option<&mut Colnr>) -> MotionType {
    if regname == i32::from(b'%')
        || regname == i32::from(b'#')
        || regname == i32::from(b'=')
        || regname == i32::from(b':')
        || regname == i32::from(b'/')
        || regname == i32::from(b'.')
        || regname == CTRL_F_CONST
        || regname == CTRL_P
        || regname == CTRL_W_CONST
        || regname == CTRL_A
        || regname == i32::from(b'_')
    {
        return MotionType::MtCharWise;
    }

    if regname != 0 && !valid_yank_reg(regname, false) {
        return MotionType::MtUnknown;
    }

    let reg = &*get_yank_register(regname, YankRegMode::Paste);
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

fn get_reg_wrap_one_line_list(s: Option<std::ffi::CString>) -> Option<&'static mut TvList> {
    let s = s?;
    let list = tv_list_alloc(1);
    tv_list_append_string(&mut *list, Some(s.as_c_str()), -1);
    Some(list)
}

pub fn get_reg_contents_str(mut regname: i32, flags: i32) -> Option<std::ffi::CString> {
    if regname == i32::from(b'=') {
        if (flags & K_GREG_NO_EXPR) != 0 {
            return None;
        }
        if (flags & K_GREG_EXPR_SRC) != 0 {
            return get_expr_line_src();
        }
        return get_expr_line();
    }

    if regname == i32::from(b'@') {
        regname = i32::from(b'"');
    }

    if regname != 0 && !valid_yank_reg(regname, false) {
        return None;
    }

    if let Some(spec) = get_spec_reg(regname, false) {
        let retval = spec?;
        return Some(crate::memory::bytes_as_cstr(&retval).to_owned());
    }

    let reg = &*get_yank_register(regname, YankRegMode::Put);
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
            out.extend_from_slice(item.as_bytes());
        }
        if reg.y_type == MotionType::MtLineWise || i < reg.y_array.len() - 1 {
            out.push(b'\n');
        }
    }

    Some(std::ffi::CString::new(out).unwrap_or_default())
}

pub fn get_reg_contents_list(mut regname: i32, flags: i32) -> Option<&'static mut TvList> {
    if regname == i32::from(b'=') {
        if (flags & K_GREG_NO_EXPR) != 0 {
            return None;
        }
        if (flags & K_GREG_EXPR_SRC) != 0 {
            return get_reg_wrap_one_line_list(get_expr_line_src());
        }
        return get_reg_wrap_one_line_list(get_expr_line());
    }

    if regname == i32::from(b'@') {
        regname = i32::from(b'"');
    }

    if regname != 0 && !valid_yank_reg(regname, false) {
        return None;
    }

    if let Some(spec) = get_spec_reg(regname, false) {
        let retval = spec?;
        return get_reg_wrap_one_line_list(Some(crate::memory::bytes_as_cstr(&retval).to_owned()));
    }

    let reg = &*get_yank_register(regname, YankRegMode::Put);
    if !reg.y_allocated {
        return None;
    }

    let list = tv_list_alloc(reg.y_array.len() as isize);
    for &item in &reg.y_array {
        crate::eval::typval::tv_list_append_bytes(&mut *list, item.as_bytes());
    }
    Some(list)
}

fn init_write_reg(name: i32, must_append: bool) -> Option<&'static mut Yankreg> {
    if !valid_yank_reg(name, true) {
        emsg_invreg(name);
        return None;
    }

    let reg = get_yank_register(name, YankRegMode::Yank);
    if !is_append_register(name) && !must_append {
        *reg = EMPTY_YANKREG;
    }
    Some(reg)
}

fn str_to_reg(
    y_ptr: &mut Yankreg,
    mut yank_type: MotionType,
    str_bytes: &[u8],
    blocklen: Colnr,
    str_list: Option<&[impl AsRef<CStr>]>,
) {
    if !y_ptr.y_allocated {
        y_ptr.y_array.clear();
    }

    if yank_type == MotionType::MtUnknown {
        yank_type =
            if str_list.is_some() || str_bytes.last().is_some_and(|&b| b == b'\n' || b == CAR) {
                MotionType::MtLineWise
            } else {
                MotionType::MtCharWise
            };
    }

    let mut newlines: usize;
    let mut extraline: usize = 0;
    let mut append = false;

    if let Some(list) = str_list {
        newlines = list.len();
    } else {
        newlines = str_bytes.iter().filter(|&&b| b == b'\n').count();
        if yank_type == MotionType::MtCharWise || str_bytes.last() != Some(&b'\n') {
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

    if let Some(list) = str_list {
        for item in list {
            let s = item.as_ref();
            y_ptr.y_array.push(cstr_to_string(Some(s)));
            if yank_type == MotionType::MtBlockWise {
                let charlen = vim_strsize(s) as usize;
                maxlen = maxlen.max(charlen);
            }
        }
    } else {
        let len = str_bytes.len();
        let limit = len + extraline;
        let mut start = 0;
        while start < limit {
            let rem = len.saturating_sub(start);
            let slice = &str_bytes[start.min(len)..];
            let line_len = slice.iter().position(|&b| b == b'\n').unwrap_or(rem);
            if yank_type == MotionType::MtBlockWise {
                let charlen = mb_string2cells_len(&slice[..line_len]);
                maxlen = maxlen.max(charlen);
            }

            let extra = if append {
                y_ptr.y_array.last().unwrap().size
            } else {
                0
            };
            let s_len = extra + line_len;
            let mut s_vec = vec![0u8; s_len + 1];
            if extra > 0 {
                let last_data = y_ptr.y_array.last().unwrap().data;
                s_vec[..extra].copy_from_slice(last_data.slice(extra));
            }
            if line_len > 0 {
                s_vec[extra..s_len].copy_from_slice(&slice[..line_len]);
            }
            for b in &mut s_vec[..s_len] {
                if *b == 0 {
                    *b = b'\n';
                }
            }
            let new_str = cbuf_to_string(&s_vec[..s_len]);

            if append {
                let last = y_ptr.y_array.last_mut().unwrap();
                api_clear_string(last);
                *last = new_str;
                append = false;
            } else {
                y_ptr.y_array.push(new_str);
            }

            start += line_len + 1;
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

pub fn write_reg_contents(name: i32, str_: &[u8], len: isize, must_append: bool) {
    write_reg_contents_ex(name, str_, len, must_append, MotionType::MtUnknown, 0);
}

pub fn write_reg_contents_lst(
    name: i32,
    strings: &[impl AsRef<CStr>],
    must_append: bool,
    yank_type: MotionType,
    block_len: Colnr,
) {
    if name == i32::from(b'/') || name == i32::from(b'=') || name == i32::from(b'#') {
        let s = if strings.is_empty() {
            c""
        } else if strings.len() > 1 {
            semsg!(gettext(E_REGISTER_CHAR_CANNOT_CONTAIN_MULTIPLE_LINES), name,);
            return;
        } else {
            strings[0].as_ref()
        };
        write_reg_contents_ex(name, s.to_bytes(), -1, must_append, yank_type, block_len);
        return;
    }

    if name == i32::from(b'_') {
        return;
    }

    let old_y_previous = reg_st().y_previous;
    let Some(reg) = init_write_reg(name, must_append) else {
        return;
    };

    str_to_reg(reg, yank_type, &[], block_len, Some(strings));
    set_clipboard(name, reg);
    if name != i32::from(b'"') {
        reg_st().y_previous = old_y_previous;
    }
}

pub fn write_reg_contents_ex(
    name: i32,
    str_in: &[u8],
    len: isize,
    must_append: bool,
    yank_type: MotionType,
    block_len: Colnr,
) {
    let str_slice = if len < 0 {
        crate::memory::bytes_until_nul(str_in)
    } else {
        &str_in[..(len as usize).min(str_in.len())]
    };

    if name == i32::from(b'/') {
        let pat_cstr = crate::memory::bytes_to_cstring(crate::memory::bytes_until_nul(str_slice));
        set_last_search_pat(Some(&pat_cstr), RE_SEARCH, 1, true);
        return;
    }

    if name == i32::from(b'#') {
        if str_slice.is_empty() {
            curwin_mut().w_alt_fnum = 0;
            return;
        }

        let buf = if str_slice.first().is_some_and(u8::is_ascii_digit) {
            let num = crate::charset::parse_i64_prefix(str_slice) as i32;
            let b = buflist_findnr(num);
            if b.is_none() {
                semsg!(gettext(E_NOBUFNR), i64::from(num));
            }
            b
        } else {
            buflist_findnr(buflist_findpat(str_slice, true, false, false))
        };
        let Some(buf_ref) = buf else {
            return;
        };
        curwin_mut().w_alt_fnum = buf_ref.b_fnum;
        return;
    }

    if name == i32::from(b'=') {
        let mut out: Vec<u8> = Vec::new();
        if must_append && let Some(el) = &reg_st().expr_line {
            out.extend_from_slice(el.as_bytes());
        }
        let end = str_slice
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(str_slice.len());
        out.extend_from_slice(&str_slice[..end]);
        reg_st().expr_line = Some(std::ffi::CString::new(out).unwrap_or_default());
        return;
    }

    if name == i32::from(b'_') {
        return;
    }

    let old_y_previous = reg_st().y_previous;
    let Some(reg) = init_write_reg(name, must_append) else {
        return;
    };
    str_to_reg(reg, yank_type, str_slice, block_len, None::<&[&CStr]>);
    set_clipboard(name, reg);
    if name != i32::from(b'"') {
        reg_st().y_previous = old_y_previous;
    }
}

pub fn prepare_yankreg_from_object(reg: &mut Yankreg, regtype: NvimString, _lines: usize) -> bool {
    let type_ch = if !regtype.data.is_null() {
        regtype.first_byte()
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
        let reg_bytes = regtype.as_bytes();
        if !reg_bytes.get(1).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        let mut rem = &reg_bytes[1..];
        reg.y_width = getdigits_int(&mut rem, false, 1) - 1;
        if !rem.is_empty() {
            return false;
        }
    }

    reg.additional_data = None;
    reg.timestamp = 0;
    true
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
        assert_eq!(op_reg_index(i32::from(b'0')), 0);
        assert_eq!(op_reg_index(i32::from(b'9')), 9);
        assert_eq!(op_reg_index(i32::from(b'a')), 10);
        assert_eq!(op_reg_index(i32::from(b'z')), 35);
        assert_eq!(op_reg_index(i32::from(b'A')), 10);
        assert_eq!(op_reg_index(i32::from(b'Z')), 35);
        assert_eq!(op_reg_index(i32::from(b'-')), DELETION_REGISTER);
        assert_eq!(op_reg_index(i32::from(b'*')), STAR_REGISTER);
        assert_eq!(op_reg_index(i32::from(b'+')), PLUS_REGISTER);
        assert_eq!(op_reg_index(i32::from(b'%')), -1);

        assert_eq!(get_register_name(-1), i32::from(b'"'));
        for idx in 0..(NUM_REGISTERS as i32) {
            let name = get_register_name(idx);
            assert_eq!(op_reg_index(name), idx);
        }
    }
}
