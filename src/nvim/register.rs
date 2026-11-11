#![forbid(unsafe_code)]

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
use crate::charset::{ascii_iswhite, getdigits_int, transchar, vim_strsize};
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
    TvDict, tv_dict_add_bool, tv_dict_add_list, tv_dict_add_str, tv_dict_add_str_len,
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
    format_to_buf, format_to_buf_safelen, gettext, ngettext, vim_strsave_escaped_ext,
};
use crate::terminal::terminal_paste;
use crate::textformat::Oparg;
use crate::ui::ui_has;
use crate::undo::{u_save, u_save_cursor};
use std::ffi::CStr;

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

struct RegisterCopyState {
    get_expr_line_nested: std::cell::Cell<i32>,
    execreg_lastc: std::cell::Cell<i32>,
    y_previous: std::cell::Cell<Option<usize>>,
    do_record_regname: std::cell::Cell<i32>,
    textyankpost_recursive: std::cell::Cell<bool>,
    put_autocmd_recursive: std::cell::Cell<bool>,
}

impl RegisterCopyState {
    const fn new() -> Self {
        Self {
            get_expr_line_nested: std::cell::Cell::new(0),
            execreg_lastc: std::cell::Cell::new(0),
            y_previous: std::cell::Cell::new(None),
            do_record_regname: std::cell::Cell::new(0),
            textyankpost_recursive: std::cell::Cell::new(false),
            put_autocmd_recursive: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static REG_STATE: RegisterCopyState = const { RegisterCopyState::new() };
    static EXPR_LINE: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RegIdx(pub usize);

pub const EMPTY_REG_IDX: RegIdx = RegIdx(NUM_REGISTERS);

#[derive(Clone, Copy)]
pub enum PutReg<'a> {
    Idx(RegIdx),
    Ref(&'a Yankreg),
}

impl PutReg<'_> {
    #[inline]
    pub fn with<R>(self, f: impl FnOnce(&Yankreg) -> R) -> R {
        match self {
            Self::Idx(idx) => idx.with(f),
            Self::Ref(r) => f(r),
        }
    }
}

thread_local! {
    static Y_REGS: std::cell::RefCell<[Yankreg; NUM_REGISTERS]> = const {
        std::cell::RefCell::new([const { EMPTY_YANKREG }; NUM_REGISTERS])
    };
    static EMPTY_REG: std::cell::RefCell<Yankreg> = const {
        std::cell::RefCell::new(EMPTY_YANKREG)
    };
}

impl RegIdx {
    #[inline]
    pub fn with<R>(self, f: impl FnOnce(&Yankreg) -> R) -> R {
        if self.0 == NUM_REGISTERS {
            EMPTY_REG.with(|cell| f(&cell.borrow()))
        } else {
            Y_REGS.with(|cell| f(&cell.borrow()[self.0]))
        }
    }

    #[inline]
    pub fn with_mut<R>(self, f: impl FnOnce(&mut Yankreg) -> R) -> R {
        if self.0 == NUM_REGISTERS {
            EMPTY_REG.with(|cell| f(&mut cell.borrow_mut()))
        } else {
            Y_REGS.with(|cell| f(&mut cell.borrow_mut()[self.0]))
        }
    }
}

pub fn get_unname_register() -> i32 {
    REG_STATE
        .with(|s| s.y_previous.get())
        .map_or(-1, |idx| idx as i32)
}

#[inline]
pub fn get_y_register(reg: i32) -> RegIdx {
    RegIdx(reg as usize)
}

#[inline]
pub fn get_y_previous() -> RegIdx {
    let idx = REG_STATE.with(|s| s.y_previous.get()).unwrap_or(0);
    RegIdx(idx)
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
    EXPR_LINE.with(|cell| *cell.borrow_mut() = new_line);
}

pub fn get_expr_line() -> Option<std::ffi::CString> {
    let expr_copy = EXPR_LINE.with(|cell| cell.borrow().clone())?;
    if REG_STATE.with(|s| s.get_expr_line_nested.get()) >= 10 {
        return Some(expr_copy);
    }

    REG_STATE.with(|s| {
        s.get_expr_line_nested
            .set(s.get_expr_line_nested.get() + (1))
    });
    let rv = eval_to_string(&expr_copy, true, false);
    REG_STATE.with(|s| {
        s.get_expr_line_nested
            .set(s.get_expr_line_nested.get() - (1))
    });
    rv
}

pub fn get_expr_line_src() -> Option<std::ffi::CString> {
    EXPR_LINE.with(|cell| cell.borrow().clone())
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
    *is_unnamed = REG_STATE
        .with(|s| s.y_previous.get())
        .is_some_and(|prev_idx| prev_idx == idx);
    idx += 1;
    while idx < NUM_SAVED_REGISTERS {
        if !reg_empty(&regs[idx]) {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

pub fn with_global_reg_iter(
    iter: Option<usize>,
    mut f: impl FnMut(u8, &Yankreg, bool),
) -> Option<usize> {
    Y_REGS.with(|cell| {
        let regs = cell.borrow();
        let mut name: u8 = 0;
        let mut reg_opt: Option<&Yankreg> = None;
        let mut is_unnamed = false;
        let next = op_reg_iter(iter, &*regs, &mut name, &mut reg_opt, &mut is_unnamed);
        if name != 0
            && let Some(reg) = reg_opt
        {
            f(name, reg, is_unnamed);
        }
        next
    })
}

pub fn reg_max_ts(clipboard: bool) -> Timestamp {
    let mut max_ts: Timestamp = 0;
    let mut iter: Option<usize> = None;
    loop {
        let mut got_one = false;
        iter = with_global_reg_iter(iter, |_, r, _| {
            got_one = true;
            max_ts = max_ts.max(r.timestamp);
        });
        if !got_one || iter.is_none() {
            break;
        }
    }
    if clipboard {
        max_ts = max_ts.max(get_y_register(op_reg_index(i32::from(b'*'))).with(|r| r.timestamp));
        max_ts = max_ts.max(get_y_register(op_reg_index(i32::from(b'+'))).with(|r| r.timestamp));
    }
    max_ts
}

pub fn op_reg_set(name: u8, reg: Yankreg, is_unnamed: bool) -> bool {
    let i = op_reg_index(i32::from(name));
    if i == -1 {
        return false;
    }
    let idx = i as usize;
    Y_REGS.with(|cell| cell.borrow_mut()[idx] = reg);

    if is_unnamed {
        REG_STATE.with(|s| s.y_previous.set(Some(idx)));
    }
    true
}

pub fn op_reg_get(name: u8) -> Option<RegIdx> {
    let i = op_reg_index(i32::from(name));
    if i == -1 {
        return None;
    }
    Some(RegIdx(i as usize))
}

pub fn op_reg_set_previous(name: u8) -> bool {
    let i = op_reg_index(i32::from(name));
    if i == -1 {
        return false;
    }
    REG_STATE.with(|s| s.y_previous.set(Some(i as usize)));
    true
}

pub fn update_yankreg_width(reg: &mut Yankreg) {
    if reg.y_type == MotionType::MtBlockWise {
        let mut maxlen: usize = 0;
        for item in &reg.y_array {
            let rowlen = mb_string2cells_len(item.as_bytes());
            maxlen = maxlen.max(rowlen);
        }
        assert!(maxlen <= i32::MAX as usize);
        reg.y_width = reg.y_width.max((maxlen as Colnr) - 1);
    }
}

pub fn get_yank_register(regname: i32, mode: YankRegMode) -> RegIdx {
    let mut reg: Option<RegIdx> = None;

    if (mode == YankRegMode::Paste || mode == YankRegMode::Put)
        && get_clipboard(regname, &mut reg, false)
    {
        return reg.unwrap();
    } else if mode == YankRegMode::Put && (regname == i32::from(b'*') || regname == i32::from(b'+'))
    {
        return EMPTY_REG_IDX;
    } else if mode != YankRegMode::Yank
        && (regname == 0
            || regname == i32::from(b'"')
            || regname == i32::from(b'*')
            || regname == i32::from(b'+'))
        && let Some(prev_idx) = REG_STATE.with(|s| s.y_previous.get())
    {
        return RegIdx(prev_idx);
    }

    let mut i = op_reg_index(regname);
    if i == -1 {
        i = 0;
    }
    let idx = i as usize;
    if mode == YankRegMode::Yank {
        REG_STATE.with(|s| s.y_previous.set(Some(idx)));
    }
    RegIdx(idx)
}

pub fn yank_register_mline(regname: i32, reg: &mut Option<RegIdx>) -> bool {
    *reg = None;
    if regname != 0 && !valid_yank_reg(regname, false) {
        return false;
    }
    if regname == i32::from(b'_') {
        return false;
    }
    let r = get_yank_register(regname, YankRegMode::Paste);
    let is_mline = r.with(|reg| reg.y_type == MotionType::MtLineWise);
    *reg = Some(r);
    is_mline
}

pub fn copy_register(name: i32) -> Yankreg {
    get_yank_register(name, YankRegMode::Paste).with(|reg| {
        let y_array = reg
            .y_array
            .iter()
            .map(|s| copy_string_global(s.clone()))
            .collect();
        Yankreg {
            y_array,
            y_allocated: reg.y_allocated,
            y_type: reg.y_type,
            y_width: reg.y_width,
            timestamp: reg.timestamp,
            additional_data: None,
        }
    })
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
    let reg_idx = get_yank_register(regname, YankRegMode::Yank);
    reg_idx.with_mut(|reg| {
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
    });
    true
}

pub fn do_record(c: i32) -> bool {
    let retval: bool;

    if g_st().reg_recording.get() == 0 {
        if c < 0 || (!(c <= 0x7f && (c as u8).is_ascii_alphanumeric()) && c != i32::from(b'"')) {
            retval = false;
        } else {
            g_st().reg_recording.set(c);
            showmode();
            REG_STATE.with(|s| s.do_record_regname.set(c));
            retval = true;
            apply_autocmds(EVENT_RECORDINGENTER, None, None, false, curbuf_opt_mut());
        }
    } else {
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let dict = get_v_event(&mut save_v_event);

        let mut p_vec = get_recorded().into_bytes_with_nul();
        vim_unescape_ks(&mut p_vec);
        tv_dict_add_str(
            &dict,
            c"regcontents",
            Some(CStr::from_bytes_until_nul(&p_vec).unwrap_or_default()),
        );

        let mut buf = [0u8; NUMBUFLEN + 2];
        buf[0] = REG_STATE.with(|s| s.do_record_regname.get()) as u8;
        buf[1] = 0;
        tv_dict_add_str(
            &dict,
            c"regname",
            Some(CStr::from_bytes_until_nul(&buf).unwrap_or_default()),
        );
        tv_dict_set_keys_readonly(&dict);

        apply_autocmds(EVENT_RECORDINGLEAVE, None, None, false, curbuf_opt_mut());
        restore_v_event(&dict, &mut save_v_event);
        g_st().reg_recorded.set(g_st().reg_recording.get());
        g_st().reg_recording.set(0);
        if opt_vars().p_ch.get() == 0 || ui_has(K_UIMESSAGES) {
            showmode();
        } else {
            msg(c"", 0);
        }
        let old_y_previous = REG_STATE.with(|s| s.y_previous.get());
        retval = stuff_yank(REG_STATE.with(|s| s.do_record_regname.get()), &p_vec);
        REG_STATE.with(|s| s.y_previous.set(old_y_previous));
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
    if g_st().restart_edit.get() == 0 {
        return;
    }

    let mut buf: [u8; 11] = [
        K_SPECIAL, KS_EXTRA, KE_COMMAND, b's', b't', b'a', b'r', b't', b'i', CAR, 0,
    ];
    if g_st().restart_edit.get() == i32::from(b'R') {
        buf[8] = b'r';
    } else if g_st().restart_edit.get() == i32::from(b'V') {
        buf[8] = b'g';
    } else if g_st().restart_edit.get() == i32::from(b'A') {
        buf[8] = b'!';
    }
    if ins_typebuf(
        CStr::from_bytes_until_nul(&buf).unwrap_or_default(),
        REMAP_NONE,
        0,
        true,
        silent,
    ) {
        g_st().restart_edit.set(0);
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

    let first_line = &lines[cmd_start];
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
        if REG_STATE.with(|s| s.execreg_lastc.get()) == 0 {
            emsg(gettext(c"E748: No previously used register"));
            return false;
        }
        regname = REG_STATE.with(|s| s.execreg_lastc.get());
    }
    if regname == i32::from(b'%') || regname == i32::from(b'#') || !valid_yank_reg(regname, false) {
        emsg_invreg(regname);
        return false;
    }
    REG_STATE.with(|s| s.execreg_lastc.set(regname));

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
                crate::memory::CStrSubslice::advance(p.as_c_str(), 5),
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
        let (y_allocated, y_type, y_array) = get_yank_register(regname, YankRegMode::Paste)
            .with(|reg| (reg.y_allocated, reg.y_type, reg.y_array.clone()));
        if !y_allocated {
            return false;
        }

        let remap = if colon { REMAP_NONE } else { REMAP_YES };
        let is_silent = silent;

        put_reedit_in_typebuf(silent);
        let mut i = y_array.len();
        while i > 0 {
            i -= 1;
            if (y_type == MotionType::MtLineWise || i < y_array.len() - 1 || addcr)
                && !ins_typebuf(c"\n", remap, 0, true, is_silent)
            {
                return false;
            }

            let mut str_bytes = y_array[i].cstr_bytes();
            let cont_vec: Vec<u8>;
            if colon && i > 0 {
                let p_bytes = crate::charset::skipwhite_slice(str_bytes);
                if p_bytes.first() == Some(&b'\\') || p_bytes.starts_with(b"\"\\ ") {
                    cont_vec = execreg_line_continuation(&y_array, &mut i);
                    str_bytes = crate::memory::bytes_until_nul(&cont_vec);
                }
            }
            let escaped = vim_strsave_escape_ks(str_bytes);
            retval = ins_typebuf(&escaped, remap, 0, true, is_silent);
            drop(escaped);
            if !retval {
                return false;
            }
            if colon && !ins_typebuf(c":", remap, 0, true, is_silent) {
                return false;
            }
        }
        g_st().reg_executing.set(if regname == 0 {
            i32::from(b'"')
        } else {
            regname
        });
        g_st().pending_end_reg_executing.set(false);
    }
    retval
}

pub fn insert_reg(regname: i32, reg: Option<RegIdx>, literally_arg: bool) -> bool {
    let mut retval = true;
    let literally = literally_arg || is_literal_register(regname);

    os_breakcheck();
    if g_st().got_int.get() {
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
        let reg_idx = reg.unwrap_or_else(|| get_yank_register(regname, YankRegMode::Paste));
        let (y_allocated, y_type, y_array) =
            reg_idx.with(|r| (r.y_allocated, r.y_type, r.y_array.clone()));
        if !y_allocated {
            retval = false;
        } else {
            let put = y_type == MotionType::MtCharWise
                && (regname == i32::from(b'-')
                    || (mc_ins_cascading()
                        && y_array.len() == 1
                        && (((0..=0x7f).contains(&regname)
                            && (regname as u8).is_ascii_alphanumeric())
                            || regname == i32::from(b'"'))));
            for i in 0..y_array.len() {
                if put {
                    let mut dir = BACKWARD;
                    if (g_st().state_global.get() & REPLACE_FLAG) != 0 {
                        if !u_save_cursor() {
                            return false;
                        }
                        del_chars(mb_charlen(y_array[0].as_bytes()), 1);
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
                    stuffescaped(y_array[i].as_bytes(), literally);
                    if y_type == MotionType::MtLineWise || i < y_array.len() - 1 {
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
            Some(getaltfname(errmsg).map(std::ffi::CString::into_bytes_with_nul))
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
            let last_pat = last_search_pat();
            if last_pat.is_none() && errmsg {
                emsg(gettext(E_NOPREVRE));
            }
            Some(last_pat.map(std::ffi::CString::into_bytes_with_nul))
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
            Some(Some(
                ml_get_buf(curwin_mut().buf_mut(), curwin_mut().w_cursor.lnum)
                    .to_bytes_with_nul()
                    .to_vec(),
            ))
        }
        x if x == i32::from(b'_') => Some(Some(vec![0])),
        _ => None,
    }
}

pub fn cmdline_paste_reg(regname: i32, literally_arg: bool, remcr: bool) -> bool {
    let literally = literally_arg || is_literal_register(regname);

    let (y_allocated, y_array) = get_yank_register(regname, YankRegMode::Paste)
        .with(|reg| (reg.y_allocated, reg.y_array.clone()));
    if !y_allocated {
        return false;
    }

    for i in 0..y_array.len() {
        cmdline_paste_str(y_array[i].as_bytes(), literally);

        if i < y_array.len() - 1 && !remcr {
            cmdline_paste_str(b"\r", literally);
        }

        os_breakcheck();
        if g_st().got_int.get() {
            return false;
        }
    }
    true
}

pub fn shift_delete_registers(y_append: bool) {
    Y_REGS.with(|cell| {
        let mut regs = cell.borrow_mut();
        regs[9] = EMPTY_YANKREG;
        let mut n = 9;
        while n > 1 {
            regs[n] = std::mem::replace(&mut regs[n - 1], EMPTY_YANKREG);
            n -= 1;
        }
    });
    if !y_append {
        REG_STATE.with(|s| s.y_previous.set(Some(1)));
    }
}

pub fn clear_registers() {
    Y_REGS.with(|cell| cell.borrow_mut().fill_with(|| EMPTY_YANKREG));
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
    let text_bytes = bd.textstart.to_bytes_with_nul();
    buf_vec[..start_usize].fill(b' ');
    buf_vec[start_usize..start_usize + text_usize]
        .copy_from_slice(&text_bytes[..text_usize.min(text_bytes.len())]);
    buf_vec[start_usize + text_usize..start_usize + text_usize + end_usize].fill(b' ');
    let mut out_size = start_usize + text_usize + end_usize;
    if exclude_trailing_space {
        let mut s = bd.textlen + bd.endspaces;
        while s > 0
            && text_bytes
                .get((s - 1) as usize)
                .copied()
                .is_some_and(ascii_iswhite)
        {
            s = s - crate::mbyte::utf_head_off_slice(text_bytes, (s - 1) as usize) - 1;
            out_size -= 1;
        }
    }
    buf_vec[out_size] = 0;
    reg.y_array[y_idx] = cbuf_to_string(&buf_vec[..out_size]);
}

pub fn op_yank_reg(oap: &mut Oparg, message: bool, reg_idx: RegIdx, append: bool) {
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

    let is_append = append && reg_idx.with(|r| r.y_allocated);
    if !is_append {
        reg_idx.with_mut(|r| *r = EMPTY_YANKREG);
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

    newreg.y_type = yank_type;
    newreg.y_width = 0;
    newreg.y_array = vec![NvimString::new(); yanklines];
    newreg.y_allocated = true;
    newreg.additional_data = None;
    newreg.timestamp = os_realtime() as u64;

    let mut y_idx: usize = 0;
    let mut lnum = oap.start.lnum;

    if yank_type == MotionType::MtBlockWise {
        newreg.y_width = oap.end_vcol - oap.start_vcol;

        if curwin_mut().w_curswant.get() == MAXCOL && newreg.y_width > 0 {
            newreg.y_width -= 1;
        }
    }

    while lnum <= yankendlnum {
        match newreg.y_type {
            MotionType::MtBlockWise => {
                block_prep(oap, &mut bd, lnum, false);

                yank_copy_line(&mut newreg, &mut bd, y_idx, oap.excl_tr_ws);
            }
            MotionType::MtLineWise => {
                let line_slice = crate::memline::ml_get_slice(lnum);

                newreg.y_array[y_idx] = cbuf_to_string(line_slice);
            }
            MotionType::MtCharWise => {
                charwise_block_prep(oap.start, oap.end, &mut bd, lnum, oap.inclusive);
                let tmp = bd.textstart.to_bytes().len() as i32;
                if tmp < bd.textlen {
                    bd.textlen = tmp;
                }

                yank_copy_line(&mut newreg, &mut bd, y_idx, false);
            }
            MotionType::MtUnknown => std::process::abort(),
        }
        lnum += 1;
        y_idx += 1;
    }

    let cpo_regappend = crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_REGAPPEND);
    reg_idx.with_mut(|curr_reg| {
        if is_append {
            if yank_type == MotionType::MtLineWise {
                curr_reg.y_type = MotionType::MtLineWise;
            }

            if curr_reg.y_type == MotionType::MtCharWise
                && !cpo_regappend
                && !curr_reg.y_array.is_empty()
                && !newreg.y_array.is_empty()
            {
                let last_old = curr_reg.y_array.last().unwrap().clone();
                let first_new = newreg.y_array[0].clone();
                let mut pnew = Vec::with_capacity(last_old.size + first_new.size);
                pnew.extend_from_slice(last_old.as_bytes());
                pnew.extend_from_slice(first_new.as_bytes());

                let last_slot = curr_reg.y_array.last_mut().unwrap();
                api_clear_string(last_slot);
                *last_slot = cbuf_to_string(&pnew);
                api_clear_string(&mut newreg.y_array[0]);
                y_idx = 1;
            } else {
                y_idx = 0;
            }
            while y_idx < newreg.y_array.len() {
                curr_reg.y_array.push(newreg.y_array[y_idx].clone());
                y_idx += 1;
            }
            newreg.y_array.clear();
        } else {
            *curr_reg = newreg;
        }
    });

    if message {
        if yank_type == MotionType::MtCharWise && yanklines == 1 {
            yanklines = 0;
        }
        if yanklines > opt_vars().p_report.get() as usize {
            let mut namebuf = [0u8; 100];
            if oap.regname == 0 {
                namebuf[0] = 0;
            } else {
                format_to_buf!(&mut namebuf, gettext(c" into \"%c"), oap.regname);
            }

            update_topline(curwin_mut());
            if g_st().must_redraw.get() != 0 {
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

fn add_regtype_to_dict(reg: Option<PutReg<'_>>, dict: &TvDict, buf: &mut [u8]) {
    let (y_type, y_width) = reg.map_or((MotionType::MtCharWise, 0), |r| {
        r.with(|reg| (reg.y_type, reg.y_width))
    });
    let len = format_reg_type(y_type, y_width, buf);
    tv_dict_add_str_len(dict, c"regtype", Some(&buf[..len]));
}

pub fn do_autocmd_textyankpost(oap: &mut Oparg, reg: RegIdx) {
    if REG_STATE.with(|s| s.textyankpost_recursive.get()) || !has_event(EVENT_TEXTYANKPOST) {
        return;
    }

    REG_STATE.with(|s| s.textyankpost_recursive.set(true));

    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let dict = get_v_event(&mut save_v_event);

    let list = reg.with(|r| {
        let list = tv_list_alloc(r.y_array.len() as isize);
        for item in &r.y_array {
            crate::eval::typval::tv_list_append_bytes(&list, item.as_bytes());
        }
        list
    });
    list.lv_lock.set(VAR_FIXED);
    tv_dict_add_list(&dict, c"regcontents", Some(&list));

    let mut buf = [0u8; NUMBUFLEN + 2];
    add_regtype_to_dict(Some(PutReg::Idx(reg)), &dict, &mut buf);

    buf[0] = oap.regname as u8;
    buf[1] = 0;
    tv_dict_add_str(
        &dict,
        c"regname",
        Some(CStr::from_bytes_until_nul(&buf).unwrap_or_default()),
    );

    tv_dict_add_bool(
        &dict,
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
        &dict,
        c"operator",
        Some(CStr::from_bytes_until_nul(&buf).unwrap_or_default()),
    );

    tv_dict_add_bool(
        &dict,
        c"visual",
        if oap.is_visual {
            K_BOOL_VAR_TRUE_CONST
        } else {
            K_BOOL_VAR_FALSE_CONST
        },
    );

    tv_dict_set_keys_readonly(&dict);
    g_st().textlock.set(g_st().textlock.get() + (1));
    apply_autocmds(EVENT_TEXTYANKPOST, None, None, false, curbuf_opt_mut());
    g_st().textlock.set(g_st().textlock.get() - (1));
    restore_v_event(&dict, &mut save_v_event);

    REG_STATE.with(|s| s.textyankpost_recursive.set(false));
}

fn put_do_autocmd(
    regname: i32,
    reg: Option<PutReg<'_>>,
    insert: Option<&NvimString>,
    post: bool,
    dir: Direction,
) {
    if REG_STATE.with(|s| s.put_autocmd_recursive.get())
        || (regname == i32::from(b'_') && reg.is_none())
    {
        return;
    }

    if regname != i32::from(b'.') && insert.is_none() && reg.is_none() {
        return;
    }

    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let v_event = get_v_event(&mut save_v_event);

    let list = tv_list_alloc(reg.map_or(1, |r| r.with(|reg| reg.y_array.len() as isize)));

    if regname == i32::from(b'.') {
        if !g_st().last_insert_ga.is_empty() {
            crate::eval::typval::tv_list_append_bytes(&list, &g_st().last_insert_ga);
        }
    } else if let Some(insert) = insert {
        crate::eval::typval::tv_list_append_bytes(&list, insert.as_bytes());
    } else if let Some(r) = reg {
        r.with(|reg| {
            for item in &reg.y_array {
                crate::eval::typval::tv_list_append_bytes(&list, item.as_bytes());
            }
        });
    }

    list.lv_lock.set(VAR_FIXED);
    tv_dict_add_list(&v_event, c"regcontents", Some(&list));

    let mut buf = [0u8; NUMBUFLEN + 2];
    buf[0] = regname as u8;
    buf[1] = 0;
    let buflen = if buf[0] == 0 { 0 } else { 1 };
    tv_dict_add_str_len(&v_event, c"regname", Some(&buf[..buflen]));

    buf[0] = if dir == BACKWARD { b'P' } else { b'p' };
    buf[1] = 0;
    tv_dict_add_str_len(&v_event, c"operator", Some(&buf[..1]));

    add_regtype_to_dict(reg, &v_event, &mut buf);

    tv_dict_add_bool(
        &v_event,
        c"visual",
        if g_st().visual_global.active {
            K_BOOL_VAR_TRUE_CONST
        } else {
            K_BOOL_VAR_FALSE_CONST
        },
    );

    tv_dict_set_keys_readonly(&v_event);

    REG_STATE.with(|s| s.put_autocmd_recursive.set(true));
    g_st().textlock.set(g_st().textlock.get() + (1));
    if post {
        apply_autocmds(EVENT_TEXTPUTPOST, None, None, false, curbuf_opt_mut());
    } else {
        apply_autocmds(EVENT_TEXTPUTPRE, None, None, false, curbuf_opt_mut());
    }
    g_st().textlock.set(g_st().textlock.get() - (1));
    REG_STATE.with(|s| s.put_autocmd_recursive.set(false));

    restore_v_event(&v_event, &mut save_v_event);
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
    set_clipboard(oap.regname, reg);
    do_autocmd_textyankpost(oap, reg);
    true
}

pub fn do_put(regname: i32, mut reg: Option<PutReg<'_>>, mut dir: i32, mut count: i32, flags: i32) {
    let mut totlen: usize = 0;
    let mut lnum: Linenr;
    let mut y_type: MotionType;
    let y_size: usize;
    let mut y_width: i32 = 0;
    let mut vcol: Colnr = 0;
    let y_array: &[NvimString];
    let mut expr_y_array: Vec<NvimString> = Vec::new();
    let reg_y_array: Vec<NvimString>;
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

    if regname == i32::from(b'.') && reg.is_none() {
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
            g_st()
                .add_last_insert
                .set(g_st().add_last_insert.get() + (1));
        }

        if (flags & PUT_LINE) != 0 {
            let save_add_last_insert = g_st().add_last_insert.get();
            g_st().add_last_insert.set(0);
            stuffchar_readbuff(K_COMMAND);
            if dir == FORWARD {
                stuff_readbuff_len(b"put _");
            } else {
                stuff_readbuff_len(b"put! _");
            }
            stuffchar_readbuff(i32::from(CAR));
            g_st().add_last_insert.set(save_add_last_insert);
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
            g_st()
                .add_last_insert
                .set(g_st().add_last_insert.get() - (1));
            if g_st().add_last_insert.get() == 0 {
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
    if reg.is_none()
        && let Some(spec) = get_spec_reg(regname, true)
    {
        let Some(spec_vec) = spec else {
            return;
        };
        insert_string_owned = Some(spec_vec);
    }

    if curbuf_mut().terminal.get().is_null()
        && !u_save(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.lnum + 1)
    {
        return;
    }

    if let Some(ref mut spec_buf) = insert_string_owned {
        let spec_len = crate::memory::bytes_until_nul(spec_buf).len();
        y_type = MotionType::MtCharWise;
        if regname == i32::from(b'=') {
            let mut start = 0usize;
            loop {
                if let Some(rel_nl) = spec_buf[start..spec_len].iter().position(|&b| b == b'\n') {
                    let nl_idx = start + rel_nl;
                    spec_buf[nl_idx] = 0;
                    expr_y_array.push(NvimString::from_bytes_len(
                        &spec_buf[start..=nl_idx],
                        rel_nl,
                    ));
                    start = nl_idx + 1;
                    if start == spec_len {
                        y_type = MotionType::MtLineWise;
                        break;
                    }
                } else {
                    let rem_len = spec_len - start;
                    expr_y_array.push(NvimString::from_bytes_len(
                        &spec_buf[start..=spec_len],
                        rem_len,
                    ));
                    break;
                }
            }
            y_size = expr_y_array.len();
            y_array = &expr_y_array;
            insert_string = NvimString::from_bytes_len(spec_buf, spec_len);
        } else {
            insert_string = NvimString::from_bytes_len(spec_buf, spec_len);
            y_size = 1;
            y_array = std::slice::from_ref(&insert_string);
        }
        if has_event(EVENT_TEXTPUTPRE) {
            put_do_autocmd(regname, None, Some(&insert_string), false, dir);
        }
    } else {
        if has_event(EVENT_TEXTPUTPRE) {
            let save_reg = reg;
            if reg.is_none() {
                reg = Some(PutReg::Idx(get_yank_register(regname, YankRegMode::Paste)));
            }

            put_do_autocmd(regname, reg, None, false, dir);

            reg = save_reg;
        }
        let reg_val =
            reg.unwrap_or_else(|| PutReg::Idx(get_yank_register(regname, YankRegMode::Paste)));
        reg = Some(reg_val);

        (y_type, y_width, y_size, y_allocated, reg_y_array) = reg_val.with(|r| {
            (
                r.y_type,
                r.y_width,
                r.y_array.len(),
                r.y_allocated,
                r.y_array.clone(),
            )
        });
        y_array = &reg_y_array;
    }

    'end: {
        if !curbuf_mut().terminal.get().is_null() {
            terminal_paste(count, y_array);
            break 'end;
        }

        let mut split_pos: Colnr = 0;
        if y_type == MotionType::MtLineWise {
            if (flags & PUT_LINE_SPLIT) != 0 {
                if !u_save_cursor() {
                    break 'end;
                }
                let p_cstr = get_cursor_pos_ptr();
                let p_slice = p_cstr.to_bytes();
                let plen = get_cursor_pos_len() as usize;
                let adv = if dir == FORWARD && plen > 0 {
                    utfc_ptr2len(p_cstr) as usize
                } else {
                    0
                };
                split_pos = curwin_mut().w_cursor.col + (adv as Colnr);

                let tail_start = adv.min(p_slice.len());
                let tail_end = plen.min(p_slice.len());
                let tail_vec = p_slice[tail_start..tail_end].to_vec();
                ml_append(curwin_mut().w_cursor.lnum, &tail_vec, 0, false);

                let line_slice = crate::cursor::get_cursor_line_slice();
                let split_u = (split_pos as usize).min(line_slice.len());
                crate::memline::ml_replace_len(
                    curwin_mut().w_cursor.lnum,
                    &line_slice[..split_u],
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
            let tc;
            semsg!(
                gettext(c"E353: Nothing in register %s"),
                if regname == 0 {
                    c"\""
                } else {
                    tc = transchar(regname);
                    &tc
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
                let ts = curbuf_mut().b_p_ts.get();
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

            for (i, y_item) in y_array.iter().enumerate() {
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
                let oldp_cstr = get_cursor_line_ptr();
                let oldp_bytes = oldp_cstr.to_bytes();
                let oldp_with_nul = oldp_cstr.to_bytes_with_nul();
                let oldlen = get_cursor_line_len();

                let mut csarg = CharsizeArg::new();
                let mut cstype: CSType = init_charsize_arg(
                    &mut csarg,
                    curwin_mut(),
                    curwin_mut().w_cursor.lnum,
                    oldp_cstr,
                );
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
                let text_off = ci.offset_from(oldp_cstr);
                bd.textcol = text_off as Colnr;

                let shortline = (vcol < col)
                    || (vcol == col && oldp_with_nul.get(text_off).copied().unwrap_or(0) == 0);

                if vcol < col {
                    bd.startspaces = col - vcol;
                } else if vcol > col {
                    bd.endspaces = vcol - col;
                    bd.startspaces = incr - bd.endspaces;
                    bd.textcol -= 1;
                    delcount = 1;
                    bd.textcol -= crate::mbyte::utf_head_off_slice(oldp_bytes, bd.textcol as usize);
                    if oldp_bytes.get(bd.textcol as usize).copied().unwrap_or(0) != TAB as u8 {
                        delcount = 0;
                        bd.endspaces = 0;
                    }
                }

                let yanklen = y_item.size as i32;

                if (flags & PUT_BLOCK_INNER) == 0 {
                    spaces = y_width + 1;

                    let y_data = y_item.as_cstr();
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
                let mut ptr_idx = bd.textcol as usize;

                newp_vec[..ptr_idx].copy_from_slice(&oldp_with_nul[..ptr_idx]);

                let start_sp = bd.startspaces as usize;
                newp_vec[ptr_idx..ptr_idx + start_sp].fill(b' ');
                ptr_idx += start_sp;

                let y_bytes = y_item.as_bytes();
                let y_len = yanklen as usize;
                for j in 0..count {
                    newp_vec[ptr_idx..ptr_idx + y_len].copy_from_slice(&y_bytes[..y_len]);
                    ptr_idx += y_len;

                    if (j < count - 1 || !shortline) && spaces > 0 {
                        let sp = spaces as usize;
                        newp_vec[ptr_idx..ptr_idx + sp].fill(b' ');
                        ptr_idx += sp;
                    } else {
                        totlen -= spaces as usize;
                    }
                }

                let end_sp = bd.endspaces as usize;
                newp_vec[ptr_idx..ptr_idx + end_sp].fill(b' ');
                ptr_idx += end_sp;

                let columns = oldlen - bd.textcol - delcount + 1;
                assert!(columns >= 0);

                let tail_start = (bd.textcol + delcount) as usize;
                let cols_u = columns as usize;
                newp_vec[ptr_idx..ptr_idx + cols_u]
                    .copy_from_slice(&oldp_with_nul[tail_start..tail_start + cols_u]);

                ml_replace(
                    curwin_mut().w_cursor.lnum,
                    crate::memory::bytes_until_nul(&newp_vec),
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
            let yanklen = y_array[0].size as i32;

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
                    let y_bytes = y_array[0].as_bytes();
                    let y_len = yanklen as usize;
                    loop {
                        let oldp_with_nul = ml_get(lnum).to_bytes_with_nul();
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
                        let col_u = col as usize;
                        newp_vec[..col_u].copy_from_slice(&oldp_with_nul[..col_u]);

                        let mut ptr_idx = col_u;
                        for _ in 0..(count as usize) {
                            newp_vec[ptr_idx..ptr_idx + y_len].copy_from_slice(&y_bytes[..y_len]);
                            ptr_idx += y_len;
                        }

                        let rem_len = ((oldlen - col) as usize) + 1;
                        newp_vec[ptr_idx..ptr_idx + rem_len]
                            .copy_from_slice(&oldp_with_nul[col_u..col_u + rem_len]);

                        ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);

                        let prev_idx = ptr_idx.saturating_sub(1);
                        first_byte_off = crate::mbyte::utf_head_off_slice(&newp_vec, prev_idx);
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

                if totlen != 0 && (g_st().restart_edit.get() != 0 || (flags & PUT_CURSEND) != 0) {
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
                            let last_y = y_array[y_size - 1].as_bytes();
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
                            let first_y = y_array[0].as_bytes();
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
                                if !ml_append(lnum, y_array[i].cstr_bytes(), 0, false) {
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
                            for item in &y_array[..(y_size - 1)] {
                                totsize += (item.size as Bcount) + 1;
                            }

                            lastsize = y_array[y_size - 1].size as i32;

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
                let last_item = &y_array[y_size - 1];
                col = ((last_item.size as Colnr) - lendiff).max(0);
                if col > 1 {
                    curbuf_mut().b_op_end.col = col - 1;
                    if last_item.size > 0 {
                        curbuf_mut().b_op_end.col -=
                            crate::mbyte::utf_head_off(last_item.as_bytes(), last_item.size - 1);
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
        curwin_mut().w_set_curswant.set(true);

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
            put_do_autocmd(regname, reg, None, true, dir);
        } else {
            put_do_autocmd(regname, None, Some(&insert_string), true, dir);
        }
    }

    drop(insert_string_owned);

    if curbuf_mut().terminal.get().is_null() {
        g_st().visual_global.active = false;
    }

    adjust_cursor_eol();
}

fn dis_msg(p_opt: Option<&CStr>, skip_esc: bool) {
    let Some(p_cstr) = p_opt else {
        return;
    };
    let mut p = p_cstr.to_bytes();
    let mut n = g_st().columns_global.get() - 6;
    while !p.is_empty() && !(p[0] == ESC && skip_esc && p.len() == 1) {
        n -= ptr2cells(p);
        if n < 0 {
            break;
        }
        let l = utfc_ptr2len(p);
        if l > 1 {
            let l_u = (l as usize).min(p.len());
            msg_outtrans_len(&p[..l_u], l, 0, false);
            p = &p[l_u..];
        } else {
            msg_outtrans_len(&p[..1], 1, 0, false);
            p = &p[1..];
        }
    }
    os_breakcheck();
}

pub fn ex_display(eap: &mut Exarg) {
    let mut arg = eap.arg.clone();
    if !arg.is_null() && arg.is_empty() {
        arg = crate::strings::OwnedSlice::NULL;
    }
    let hl_id = HLF_8;

    msg_ext_set_kind(c"list_cmd");
    g_st().msg_ext_skip_flush.set(true);
    msg_puts_title(gettext(c"\nType Name Content"));

    let mut i: i32 = -1;
    while i < (NUM_REGISTERS as i32) && !g_st().got_int.get() {
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

        let yb_idx = if i == -1 {
            REG_STATE.with(|s| s.y_previous.get()).unwrap_or(0)
        } else {
            i as usize
        };

        let mut cb_target: Option<RegIdx> = None;
        get_clipboard(name, &mut cb_target, true);
        let yb_reg = cb_target.unwrap_or(RegIdx(yb_idx));

        if name == mb_tolower(g_st().redir_reg.get())
            || (g_st().redir_reg.get() == i32::from(b'"')
                && REG_STATE
                    .with(|s| s.y_previous.get())
                    .is_some_and(|prev_idx| yb_reg == RegIdx(prev_idx)))
        {
            i += 1;
            continue;
        }

        let (y_allocated, y_type, y_array) =
            yb_reg.with(|r| (r.y_allocated, r.y_type, r.y_array.clone()));
        if y_allocated {
            let mut do_show = false;
            for item in &y_array {
                if !message_filtered(item.as_cstr()) {
                    do_show = true;
                    break;
                }
            }

            if do_show || y_array.is_empty() {
                msg_putchar(i32::from(b'\n'));
                msg_puts(c"  ");
                msg_putchar(type_ch);
                msg_puts(c"  ");
                msg_putchar(i32::from(b'"'));
                msg_putchar(name);
                msg_puts(c"   ");

                let mut n = g_st().columns_global.get() - 11;
                let mut j: usize = 0;
                while j < y_array.len() && n > 1 {
                    if j != 0 {
                        msg_puts_hl(c"^J", hl_id, false);
                        n -= 2;
                    }
                    let mut p = y_array[j].cstr_bytes();
                    while !p.is_empty() {
                        n -= ptr2cells(p);
                        if n < 0 {
                            break;
                        }
                        let clen = utfc_ptr2len(p);
                        let clen_u = (clen.max(0) as usize).min(p.len());
                        msg_outtrans_len(&p[..clen_u], clen, 0, false);
                        p = &p[clen_u..];
                    }
                    j += 1;
                }
                if n > 1 && y_type == MotionType::MtLineWise {
                    msg_puts_hl(c"^J", hl_id, false);
                }
            }
            os_breakcheck();
        }

        i += 1;
    }

    if let Some(insert) = get_last_insert()
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'.')))
        && !g_st().got_int.get()
        && !message_filtered(&insert)
    {
        msg_puts(c"\n  c  \".   ");
        dis_msg(Some(&insert), true);
    }

    if let Some(last_cmd) = g_st().last_cmdline.as_ref()
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b':')))
        && !g_st().got_int.get()
        && !message_filtered(last_cmd)
    {
        msg_puts(c"\n  c  \":   ");
        dis_msg(Some(last_cmd), false);
    }

    if curbuf_mut().fname_opt().is_some()
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'%')))
        && !g_st().got_int.get()
        && !message_filtered(curbuf_mut().fname_cstr())
    {
        msg_puts(c"\n  c  \"%   ");
        dis_msg(curbuf_mut().fname_opt(), false);
    }

    if (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'#')))
        && !g_st().got_int.get()
    {
        let mut fname = None;
        let mut dummy: Linenr = 0;
        if buflist_name_nr(0, &mut fname, &mut dummy)
            && !message_filtered(fname.as_deref().unwrap_or(c""))
        {
            msg_puts(c"\n  c  \"#   ");
            dis_msg(fname.as_deref(), false);
        }
    }

    if let Some(last_pat) = last_search_pat()
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'/')))
        && !g_st().got_int.get()
        && !message_filtered(&last_pat)
    {
        msg_puts(c"\n  c  \"/   ");
        dis_msg(Some(&last_pat), false);
    }

    if let Some(el) = EXPR_LINE.with(|cell| cell.borrow().clone())
        && (arg.is_null() || crate::strings::vim_haschr(arg.as_cstr(), i32::from(b'=')))
        && !g_st().got_int.get()
        && !message_filtered(&el)
    {
        msg_puts(c"\n  c  \"=   ");
        dis_msg(Some(&el), false);
    }
    g_st().msg_ext_skip_flush.set(false);
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

    get_yank_register(regname, YankRegMode::Paste).with(|reg| {
        if reg.y_allocated {
            if reg.y_type == MotionType::MtBlockWise
                && let Some(reg_width) = reg_width
            {
                *reg_width = reg.y_width;
            }
            return reg.y_type;
        }
        MotionType::MtUnknown
    })
}

fn get_reg_wrap_one_line_list(s: Option<std::ffi::CString>) -> Option<crate::eval::typval::ListId> {
    let s = s?;
    let list = tv_list_alloc(1);
    tv_list_append_string(&list, Some(s.as_c_str()), -1);
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

    get_yank_register(regname, YankRegMode::Put).with(|reg| {
        if !reg.y_allocated {
            return None;
        }

        let mut len: usize = 0;
        for (i, item) in reg.y_array.iter().enumerate() {
            len += item.size;
            if reg.y_type == MotionType::MtLineWise || i < reg.y_array.len() - 1 {
                len += 1;
            }
        }

        let mut out: Vec<u8> = Vec::with_capacity(len);
        for (i, item) in reg.y_array.iter().enumerate() {
            if item.size > 0 {
                out.extend_from_slice(item.as_bytes());
            }
            if reg.y_type == MotionType::MtLineWise || i < reg.y_array.len() - 1 {
                out.push(b'\n');
            }
        }

        Some(std::ffi::CString::new(out).unwrap_or_default())
    })
}

pub fn get_reg_contents_list(mut regname: i32, flags: i32) -> Option<crate::eval::typval::ListId> {
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

    get_yank_register(regname, YankRegMode::Put).with(|reg| {
        if !reg.y_allocated {
            return None;
        }

        let list = tv_list_alloc(reg.y_array.len() as isize);
        for item in &reg.y_array {
            crate::eval::typval::tv_list_append_bytes(&list, item.as_bytes());
        }
        Some(list)
    })
}

fn init_write_reg(name: i32, must_append: bool) -> Option<RegIdx> {
    if !valid_yank_reg(name, true) {
        emsg_invreg(name);
        return None;
    }

    let reg = get_yank_register(name, YankRegMode::Yank);
    if !is_append_register(name) && !must_append {
        reg.with_mut(|r| *r = EMPTY_YANKREG);
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
                let last_item = y_ptr.y_array.last().unwrap();
                s_vec[..extra].copy_from_slice(last_item.slice(extra));
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

    let old_y_previous = REG_STATE.with(|s| s.y_previous.get());
    let Some(reg) = init_write_reg(name, must_append) else {
        return;
    };

    reg.with_mut(|r| str_to_reg(r, yank_type, &[], block_len, Some(strings)));
    set_clipboard(name, reg);
    if name != i32::from(b'"') {
        REG_STATE.with(|s| s.y_previous.set(old_y_previous));
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
            curwin_mut().w_alt_fnum.set(0);
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
        curwin_mut().w_alt_fnum.set(buf_ref.b_fnum.get());
        return;
    }

    if name == i32::from(b'=') {
        let mut out: Vec<u8> = Vec::new();
        if must_append {
            EXPR_LINE.with(|cell| {
                if let Some(el) = cell.borrow().as_ref() {
                    out.extend_from_slice(el.as_bytes());
                }
            });
        }
        let end = str_slice
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(str_slice.len());
        out.extend_from_slice(&str_slice[..end]);
        EXPR_LINE.with(|cell| {
            *cell.borrow_mut() = Some(std::ffi::CString::new(out).unwrap_or_default());
        });
        return;
    }

    if name == i32::from(b'_') {
        return;
    }

    let old_y_previous = REG_STATE.with(|s| s.y_previous.get());
    let Some(reg) = init_write_reg(name, must_append) else {
        return;
    };
    reg.with_mut(|r| str_to_reg(r, yank_type, str_slice, block_len, None::<&[&CStr]>));
    set_clipboard(name, reg);
    if name != i32::from(b'"') {
        REG_STATE.with(|s| s.y_previous.set(old_y_previous));
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
