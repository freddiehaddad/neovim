//! Quickfix and location list commands and functions (`src/nvim/quickfix.c`).

use crate::api::private::helpers::{boolean_obj, cbuf_as_string, cstr_as_obj, cstr_as_string};
use crate::arglist::{ML_EMPTY, get_arglist_exp};
use crate::autocmd::{
    EVENT_BUFREADPOST, EVENT_BUFWINENTER, EVENT_FILETYPE, EVENT_QUICKFIXCMDPOST,
    EVENT_QUICKFIXCMDPRE, OPT_NOWIN, apply_autocmds, au_event_disable, au_event_restore,
    block_autocmds, unblock_autocmds,
};
use crate::buffer::{
    BCO_ENTER, BCO_NOHELP, BF_DUMMY, BF_NEW, BLN_DUMMY, BLN_NOOPT, Bufref, DOBUF_UNLOAD,
    DOBUF_WIPE, FileBuf, GETF_SETMARK, GETF_SWITCH, K_EXTMARK_NO_UNDO, MAXLNUM, NOTDONE, VAR_FIXED,
    bt_help, bt_normal, bt_quickfix, buf_valid, buflist_findname_exp, buflist_findnr,
    buflist_getfile, buflist_new, bufref_valid, close_buffer, do_modelines, no_write_message,
    set_bufref, setfname, wipe_buffer,
};
use crate::change::{TAB, changed_lines};
use crate::charset::{NL, NUL, ascii_iswhite, skipdigits, skipwhite, vim_isprintc};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor, get_cursor_line_ptr};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, redraw_buf_later, redraw_curbuf_later, redraw_later, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    Callback, Dictitem, EvalFuncData, K_CALLBACK_NONE_CONST, Listitem, TvDict, TvList, TypVal,
    VarType, Varnumber, callback_copy, callback_free, callback_put, tv_clear, tv_copy, tv_dict_add,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len, tv_dict_add_tv,
    tv_dict_alloc, tv_dict_alloc_lock, tv_dict_alloc_ret, tv_dict_find, tv_dict_get_bool,
    tv_dict_get_number, tv_dict_get_string, tv_dict_get_tv, tv_dict_item_alloc_len,
    tv_dict_item_free, tv_dict_unref, tv_get_number_chk, tv_get_string_chk, tv_initial_value,
    tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_first, tv_list_len, tv_list_ref,
    tv_list_unref,
};
use crate::eval::vars::set_internal_string_var;
use crate::eval::window::{find_win_by_nr_or_id, win_id2wp};
use crate::eval::{
    K_LIST_LEN_MAY_KNOW_CONST, callback_call, callback_from_typval, eval_expr, set_ref_in_callback,
    set_ref_in_item,
};
use crate::ex_cmds::{
    BL_FIX, BL_WHITE, ECMD_HIDE, ECMD_NOWINENTER, ECMD_OLDBUF, ECMD_ONE, ECMD_SET_HELP,
    K_SHM_OVERALL, VGR_FUZZY, VGR_GLOBAL, VGR_NOJUMP, append_redir, check_secure, do_ecmd,
    do_shell, skip_vimgrep_pat,
};
use crate::ex_cmds2::{CMD_CDO, CMD_CFDO, CMD_LDO, CMD_LFDO, autowrite_all, can_abandon};
use crate::ex_docmd::{CMOD_HIDE, Exarg, do_cmdline_cmd, ex_cd, is_loclist_cmd};
use crate::ex_eval::{ExCleanup, aborting, enter_cleanup, leave_cleanup};
use crate::ex_getln::get_list_range;
use crate::extmark::extmark_splice;
use crate::fileio::{
    EINTR, FILE, READ_DUMMY, READ_NEW, fclose, fdopen, ferror, fgets, readfile, shorten_buf_fname,
    shorten_fnames, vim_fgets, vim_tempname,
};
use crate::fold::{fold_open_cursor, fold_update_all};
use crate::fuzzy::{FUZZY_MATCH_MAX_LEN, fuzzy_match};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::help::check_help_lang;
use crate::highlight_group::syn_name2id;
use crate::insert::beginline;
use crate::mark::{BUF_HAS_LL_ENTRY, BUF_HAS_QF_ENTRY, setpcmark};
use crate::option::{
    K_OPT_BUFHIDDEN, K_OPT_BUFTYPE, K_OPT_FILETYPE, K_OPT_SWAPFILE, OPT_LOCAL, buf_copy_options,
    copy_option_part, set_option_direct, set_option_value_give_err, shortmess, skip_to_option_part,
};
use crate::os::fs::{
    FileInfo, MAXPATHL, os_dirname, os_errno, os_fileinfo_link, os_fopen, os_isdir,
    os_open_stdin_fd, os_path_exists, os_remove, os_set_errno,
};
use crate::regexp::{
    RE_MAGIC, RE_STRING, Regmatch, Regmmatch, Regprog, vim_regcomp, vim_regexec, vim_regexec_multi,
    vim_regfree,
};
use crate::window::{
    TabPageInfo, WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HELP, WSP_NEWLOC, WSP_QUICKFIX, WSP_TOP,
    WSP_VERT, WinInfoInfo, check_can_set_curbuf_forceit, check_lnums, goto_tabpage_win,
    reset_binding, tabline_height, win_close, win_enter, win_goto, win_setheight, win_setwidth,
    win_split, win_valid,
};

use crate::math::{FAIL, OK};
use crate::mbyte::{Vimconv, convert_setup, ptr2cells, remove_bom, string_convert, utfc_ptr2len};
use crate::memline::{
    check_need_swap, ml_append_buf, ml_delete, ml_get_buf, ml_get_buf_len, ml_open,
};
use crate::memory::xstrlcpy;
use crate::message::{
    emsg, internal_error, message_filtered, msg, msg_clr_eos, msg_ext_set_kind, msg_keep,
    msg_outtrans, msg_prt_line, msg_putchar, msg_puts, msg_puts_hl, msg_start, msg_strtrunc, semsg,
    smsg, trunc_string,
};
use crate::r#move::update_topline;
use crate::msgpack_rpc::packer::ObjHandle;
use crate::normal::reset_visual_and_resel;
use crate::ops::get_region_bytecount;
use crate::option_vars::*;
use crate::optionstr::{OptIndex, free_string_option};
use crate::os::env::{expand_env, os_get_pid};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::os::time::{os_time, time_t};
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{
    EW_FILE, EW_SILENT, K_PATH_CMP_LITERAL, add_pathsep, concat_fnames, fix_fname,
    gen_expand_wildcards_vec, path_equal, path_is_absolute, path_tail, path_try_shorten_fname,
    vim_isAbsName,
};
use crate::search::{BACKWARD, Direction, FORWARD, do_search, last_search_pat};
use crate::strings::{gettext, vim_snprintf, vim_snprintf_safelen, vim_strchr};
use crate::ui::ui_flush;
use crate::undo::u_clearallandblockfree;
use std::ffi::CStr;
use std::ptr;

use crate::api::vim::HLF_N;
use crate::cmdexpand::HLF_D;
use crate::diff::K_OPT_FOLDMETHOD;
use crate::drawline::HLF_QFL;
use crate::eval::funcs::{K_OPT_CPOPTIONS, SEARCH_KEEP};
use crate::viml::parser::parser::CONV_NONE;

const INVALID_QFIDX: i32 = -1;
const INVALID_QFBUFNR: i32 = 0;
const FMT_PATTERNS: usize = 14;
const FMT_PATTERN_M: usize = 8;
const FMT_PATTERN_R: usize = 9;
const QF_WINHEIGHT: i32 = 10;
const LINE_MAXLEN: usize = 4096;

const QF_FAIL: i32 = 0;
const QF_OK: i32 = 1;
const QF_END_OF_INPUT: i32 = 2;
const QF_NOMEM: i32 = 3;
const QF_IGNORE_LINE: i32 = 4;
const QF_MULTISCAN: i32 = 5;
const QF_ABORT: i32 = 6;

const QF_GETLIST_NONE: i32 = 0x0;
const QF_GETLIST_TITLE: i32 = 0x1;
const QF_GETLIST_ITEMS: i32 = 0x2;
const QF_GETLIST_NR: i32 = 0x4;
const QF_GETLIST_WINID: i32 = 0x8;
const QF_GETLIST_CONTEXT: i32 = 0x10;
const QF_GETLIST_ID: i32 = 0x20;
const QF_GETLIST_IDX: i32 = 0x40;
const QF_GETLIST_SIZE: i32 = 0x80;
const QF_GETLIST_TICK: i32 = 0x100;
const QF_GETLIST_FILEWINID: i32 = 0x200;
const QF_GETLIST_QFBUFNR: i32 = 0x400;
const QF_GETLIST_QFTF: i32 = 0x800;
const QF_GETLIST_ALL: i32 = 0xFFF;

pub(crate) const CMDBUFFSIZE: usize = 1024;

const FORWARD_FILE: Direction = 3;
const BACKWARD_FILE: Direction = -3;

pub(crate) const K_OPT_SWB_FLAG_USETAB: u32 = 0x02;
const K_OPT_SWB_FLAG_USELAST: u32 = 0x20;
const K_OPT_FDO_FLAG_QUICKFIX: u32 = 0x20;

pub(crate) const K_OPT_ERRORFILE: OptIndex = 86;

const CMD_CNEXT: i32 = 46;
const CMD_CNFILE: i32 = 47;
const CMD_CABOVE: i32 = 50;
const CMD_CADDBUFFER: i32 = 51;
const CMD_CADDEXPR: i32 = 52;
const CMD_CADDFILE: i32 = 53;
const CMD_CAFTER: i32 = 54;
const CMD_CBUFFER: i32 = 57;
const CMD_CBEFORE: i32 = 58;
const CMD_CBELOW: i32 = 59;
const CMD_CC: i32 = 61;
const CMD_CD: i32 = 63;
const CMD_CEXPR: i32 = 66;
const CMD_CFILE: i32 = 67;
const CMD_CFIRST: i32 = 69;
const CMD_CGETFILE: i32 = 70;
const CMD_CGETBUFFER: i32 = 71;
const CMD_CGETEXPR: i32 = 72;
const CMD_CNFILE_VARIANT: i32 = 88;
const CMD_COLDER: i32 = 93;
const CMD_CPREVIOUS: i32 = 103;
const CMD_CPFILE: i32 = 104;
const CMD_CREWIND: i32 = 106;
const CMD_GREP: i32 = 175;
const CMD_GREPADD: i32 = 176;
const CMD_HELPGREP: i32 = 181;
const CMD_LNEXT: i32 = 214;
const CMD_LNFILE: i32 = 215;
const CMD_LABOVE: i32 = 217;
const CMD_LADDEXPR: i32 = 219;
const CMD_LADDBUFFER: i32 = 220;
const CMD_LADDFILE: i32 = 221;
const CMD_LAFTER: i32 = 222;
const CMD_LBUFFER: i32 = 224;
const CMD_LBELOW: i32 = 226;
const CMD_LCD: i32 = 228;
const CMD_LEXPR: i32 = 235;
const CMD_LFILE: i32 = 236;
const CMD_LFIRST: i32 = 238;
const CMD_LGETFILE: i32 = 239;
const CMD_LGETBUFFER: i32 = 240;
const CMD_LGETEXPR: i32 = 241;
const CMD_LGREP: i32 = 242;
const CMD_LGREPADD: i32 = 243;
const CMD_LHELPGREP: i32 = 244;
const CMD_LL: i32 = 246;
const CMD_LMAKE: i32 = 251;
const CMD_LNFILE_VARIANT: i32 = 255;
const CMD_LOLDER: i32 = 261;
const CMD_LPREVIOUS: i32 = 263;
const CMD_LPFILE: i32 = 264;
const CMD_LREWIND: i32 = 265;
const CMD_LVIMGREP: i32 = 271;
const CMD_LVIMGREPADD: i32 = 272;
const CMD_MAKE: i32 = 278;
const CMD_VIMGREP: i32 = 517;
const CMD_VIMGREPADD: i32 = 518;

const E_NO_MORE_ITEMS: &CStr = c"E553: No more items";
const E_CURRENT_QUICKFIX_LIST_WAS_CHANGED: &CStr = c"E925: Current quickfix list was changed";
const E_CURRENT_LOCATION_LIST_WAS_CHANGED: &CStr = c"E926: Current location list was changed";
const E_INVACT: &CStr = c"E927: Invalid action: '%s'";

#[repr(C)]
pub struct Qfline {
    pub qf_lnum: Linenr,
    pub qf_end_lnum: Linenr,
    pub qf_fnum: i32,
    pub qf_col: i32,
    pub qf_end_col: i32,
    pub qf_nr: i32,
    pub qf_module: Option<std::ffi::CString>,
    pub qf_fname: Option<std::ffi::CString>,
    pub qf_pattern: Option<std::ffi::CString>,
    pub qf_text: std::ffi::CString,
    pub qf_viscol: u8,
    pub qf_cleared: u8,
    pub qf_type: u8,
    pub qf_user_data: TypVal,
    pub qf_valid: u8,
}

impl Qfline {
    pub fn new() -> Self {
        Self {
            qf_lnum: 0,
            qf_end_lnum: 0,
            qf_fnum: 0,
            qf_col: 0,
            qf_end_col: 0,
            qf_nr: 0,
            qf_module: None,
            qf_fname: None,
            qf_pattern: None,
            qf_text: std::ffi::CString::default(),
            qf_viscol: 0,
            qf_cleared: 0,
            qf_type: 0,
            qf_user_data: tv_initial_value(),
            qf_valid: 0,
        }
    }
}

impl Drop for Qfline {
    fn drop(&mut self) {
        tv_clear(&mut self.qf_user_data);
    }
}

pub type Qfltype = i32;
pub const QFLT_QUICKFIX: Qfltype = 0;
pub const QFLT_LOCATION: Qfltype = 1;
pub const QFLT_INTERNAL: Qfltype = 2;

#[repr(C)]
pub struct QfList {
    pub qf_id: u32,
    pub qfl_type: Qfltype,
    pub qf_items: Vec<Qfline>,
    pub qf_index: i32,
    pub qf_nonevalid: bool,
    pub qf_has_user_data: bool,
    pub qf_title: Option<std::ffi::CString>,
    pub qf_ctx: Option<TypVal>,
    pub qf_qftf_cb: Callback,
    pub qf_dir_stack: Vec<std::ffi::CString>,
    pub qf_file_stack: Vec<std::ffi::CString>,
    pub qf_multiline: bool,
    pub qf_multiignore: bool,
    pub qf_multiscan: bool,
    pub qf_changedtick: i32,
}

impl QfList {
    #[inline]
    pub fn directory_ptr(&self) -> Option<&CStr> {
        self.qf_dir_stack.last().map(std::ffi::CString::as_c_str)
    }

    #[inline]
    pub fn currfile_ptr(&self) -> Option<&CStr> {
        self.qf_file_stack.last().map(std::ffi::CString::as_c_str)
    }

    #[inline]
    pub fn count(&self) -> i32 {
        self.qf_items.len() as i32
    }

    #[inline]
    pub fn cur_idx(&self) -> usize {
        if self.qf_index > 0 {
            (self.qf_index - 1) as usize
        } else {
            0
        }
    }

    #[inline]
    pub fn cur_entry(&mut self) -> *mut Qfline {
        let idx = self.cur_idx();
        if idx < self.qf_items.len() {
            &raw mut self.qf_items[idx]
        } else {
            ptr::null_mut()
        }
    }
}

impl Default for QfList {
    fn default() -> Self {
        Self {
            qf_id: 0,
            qfl_type: 0,
            qf_items: Vec::new(),
            qf_index: 0,
            qf_nonevalid: false,
            qf_has_user_data: false,
            qf_title: None,
            qf_ctx: None,
            qf_qftf_cb: Callback::new(),
            qf_dir_stack: Vec::new(),
            qf_file_stack: Vec::new(),
            qf_multiline: false,
            qf_multiignore: false,
            qf_multiscan: false,
            qf_changedtick: 0,
        }
    }
}

impl Drop for QfList {
    fn drop(&mut self) {
        qf_free(self);
    }
}

#[repr(C)]
pub struct QfInfo {
    pub qf_refcount: i32,
    pub qf_listcount: i32,
    pub qf_curlist: i32,
    pub qf_maxcount: i32,
    pub qf_lists: Vec<QfList>,
    pub qfl_type: Qfltype,
    pub qf_bufnr: i32,
}

#[repr(C)]
struct Efm {
    prog: Option<Box<Regprog>>,
    addr: [u8; FMT_PATTERNS],
    prefix: u8,
    flags: u8,
    conthere: i32,
}

enum QfInputSource {
    None,
    File(crate::os::fs::CFile),
    Str(*mut u8),
    List(*mut Listitem),
    Buf {
        buf: *mut FileBuf,
        buflnum: Linenr,
        lnumlast: Linenr,
    },
}

struct Qfstate {
    use_growbuf: bool,
    linelen: usize,
    growbuf: Vec<u8>,
    source: QfInputSource,
    vc: Vimconv,
}

impl Qfstate {
    #[inline]
    fn linebuf_ptr(&mut self) -> *mut u8 {
        if self.use_growbuf {
            self.growbuf.as_mut_ptr()
        } else {
            g_st().iobuff.as_mut_ptr().cast()
        }
    }
}

impl Default for Qfstate {
    fn default() -> Self {
        Self {
            use_growbuf: false,
            linelen: 0,
            growbuf: Vec::new(),
            source: QfInputSource::None,
            vc: Vimconv::new(),
        }
    }
}

#[repr(C)]
struct Qffields {
    namebuf: Vec<u8>,
    bnr: i32,
    module: Vec<u8>,
    errmsg: Vec<u8>,
    lnum: Linenr,
    end_lnum: Linenr,
    col: i32,
    end_col: i32,
    use_viscol: bool,
    pattern: Vec<u8>,
    enr: i32,
    type_: u8,
    valid: bool,
}

impl Default for Qffields {
    fn default() -> Self {
        Self {
            namebuf: vec![0; CMDBUFFSIZE + 1],
            bnr: 0,
            module: vec![0; CMDBUFFSIZE + 1],
            errmsg: vec![0; CMDBUFFSIZE + 1],
            lnum: 0,
            end_lnum: 0,
            col: 0,
            end_col: 0,
            use_viscol: false,
            pattern: vec![0; CMDBUFFSIZE + 1],
            enr: 0,
            type_: 0,
            valid: false,
        }
    }
}

#[repr(C)]
struct VgrArgs {
    tomatch: i32,
    spat: Option<std::ffi::CString>,
    flags: i32,
    fnames: Vec<std::ffi::CString>,
    regmatch: Regmmatch,
    qf_title: Option<std::ffi::CString>,
}

impl VgrArgs {
    const fn new() -> Self {
        Self {
            tomatch: 0,
            spat: None,
            flags: 0,
            fnames: Vec::new(),
            regmatch: Regmmatch::new(),
            qf_title: None,
        }
    }

    #[inline]
    fn spat_ptr(&self) -> Option<&CStr> {
        self.spat.as_deref()
    }
}

impl Default for VgrArgs {
    fn default() -> Self {
        Self::new()
    }
}

struct FmtPattern {
    convchar: u8,
    pattern: &'static CStr,
}

const FMT_PAT: [FmtPattern; FMT_PATTERNS] = [
    FmtPattern {
        convchar: b'f',
        pattern: c".\\+",
    },
    FmtPattern {
        convchar: b'b',
        pattern: c"\\d\\+",
    },
    FmtPattern {
        convchar: b'n',
        pattern: c"\\d\\+",
    },
    FmtPattern {
        convchar: b'l',
        pattern: c"\\d\\+",
    },
    FmtPattern {
        convchar: b'e',
        pattern: c"\\d\\+",
    },
    FmtPattern {
        convchar: b'c',
        pattern: c"\\d\\+",
    },
    FmtPattern {
        convchar: b'k',
        pattern: c"\\d\\+",
    },
    FmtPattern {
        convchar: b't',
        pattern: c".",
    },
    FmtPattern {
        convchar: b'm',
        pattern: c".\\+",
    },
    FmtPattern {
        convchar: b'r',
        pattern: c".*",
    },
    FmtPattern {
        convchar: b'p',
        pattern: c"[-\t .]*",
    },
    FmtPattern {
        convchar: b'v',
        pattern: c"\\d\\+",
    },
    FmtPattern {
        convchar: b's',
        pattern: c".\\+",
    },
    FmtPattern {
        convchar: b'o',
        pattern: c".\\+",
    },
];

struct QuickfixState {
    ql_info_actual: QfInfo,
    last_qf_id: u32,
    qf_last_bufname: Option<std::ffi::CString>,
    qf_last_bufref: Bufref,
    qfga: Vec<u8>,
    quickfix_busy: i32,
    qf_delq: Vec<*mut QfInfo>,
    fmt_start_idx: Option<usize>,
    fmt_cache: Vec<Efm>,
    last_efm_cache: Option<std::ffi::CString>,
    qf_file_hl_id: i32,
    qf_sep_hl_id: i32,
    qf_line_hl_id: i32,
    qftf_recursive: bool,
    mef_start: i32,
    mef_off: i32,
    did_bufnr_emsg: bool,
    set_qf_ll_recursive: i32,
}

impl QuickfixState {
    const fn new() -> Self {
        Self {
            ql_info_actual: QfInfo {
                qf_refcount: 0,
                qf_listcount: 0,
                qf_curlist: 0,
                qf_maxcount: 0,
                qf_lists: Vec::new(),
                qfl_type: QFLT_QUICKFIX,
                qf_bufnr: INVALID_QFBUFNR,
            },
            last_qf_id: 0,
            qf_last_bufname: None,
            qf_last_bufref: Bufref {
                br_buf: ptr::null_mut(),
                br_fnum: 0,
                br_buf_free_count: 0,
            },
            qfga: Vec::new(),
            quickfix_busy: 0,
            qf_delq: Vec::new(),
            fmt_start_idx: None,
            fmt_cache: Vec::new(),
            last_efm_cache: None,
            qf_file_hl_id: 0,
            qf_sep_hl_id: 0,
            qf_line_hl_id: 0,
            qftf_recursive: false,
            mef_start: -1,
            mef_off: 0,
            did_bufnr_emsg: false,
            set_qf_ll_recursive: 0,
        }
    }
}

static QF_STATE: GlobalState<QuickfixState> = GlobalState::new(QuickfixState::new());

#[inline]
fn qf_st() -> &'static mut QuickfixState {
    unsafe { &mut *QF_STATE.as_ptr() }
}

#[inline]
fn ql_info_ptr() -> *mut QfInfo {
    &raw mut qf_st().ql_info_actual
}

#[inline]
fn c_strcat<T>(dst: *mut T, src: impl crate::strings::IntoCCharPtr) {
    let dst: *mut u8 = dst.cast();
    let src: *const u8 = src.into_c_char_ptr().cast();
    let dlen = crate::memory::cstr_len(dst.cast());
    let slen = crate::memory::cstr_len(src.cast());
    unsafe { ptr::copy_nonoverlapping(src, dst.wrapping_add(dlen), slen + 1) };
}

#[inline]
fn is_qf_window(wp: &WinInfoInfo) -> bool {
    bt_quickfix(unsafe { wp.w_buffer.as_ref() }) && wp.w_llist_ref.is_null()
}

#[inline]
fn is_ll_window(wp: &WinInfoInfo) -> bool {
    bt_quickfix(unsafe { wp.w_buffer.as_ref() }) && !wp.w_llist_ref.is_null()
}

#[inline]
fn is_qf_stack(qi: &QfInfo) -> bool {
    qi.qfl_type == QFLT_QUICKFIX
}

#[inline]
fn is_ll_stack(qi: &QfInfo) -> bool {
    qi.qfl_type == QFLT_LOCATION
}

#[inline]
fn is_qf_list(qfl: &QfList) -> bool {
    qfl.qfl_type == QFLT_QUICKFIX
}

#[inline]
fn get_loc_list(wp: &WinInfoInfo) -> *mut QfInfo {
    if is_ll_window(wp) {
        wp.w_llist_ref
    } else {
        wp.w_llist
    }
}

#[inline]
fn tab_firstwin(tp: &TabPageInfo) -> &'static mut WinInfoInfo {
    unsafe {
        if ptr::eq(tp, g_st().curtab) {
            &mut *g_st().firstwin
        } else {
            &mut *tp.tp_firstwin
        }
    }
}

fn qfga_get() -> &'static mut Vec<u8> {
    if qf_st().qfga.capacity() == 0 {
        qf_st().qfga.reserve(256);
    }
    qf_st().qfga.clear();
    &mut qf_st().qfga
}

fn qfga_clear() {
    if qf_st().qfga.capacity() > 1000 {
        qf_st().qfga = Vec::new();
    } else {
        qf_st().qfga.clear();
    }
}

#[inline]
fn qfga_concat(gap: &mut Vec<u8>, s: impl crate::strings::IntoCCharPtr) {
    let s: *const u8 = s.into_c_char_ptr().cast();
    if !s.is_null() {
        let len = crate::memory::cstr_len(s.cast());
        if len > 0 {
            gap.extend_from_slice(unsafe { std::slice::from_raw_parts(s, len) });
        }
    }
}

#[inline]
fn qfga_concat_len(gap: &mut Vec<u8>, s: impl crate::strings::IntoCCharPtr, len: usize) {
    let s: *const u8 = s.into_c_char_ptr().cast();
    if !s.is_null() && len > 0 {
        gap.extend_from_slice(unsafe { std::slice::from_raw_parts(s, len) });
    }
}

fn qf_init_process_nextline(
    qfl: &mut QfList,
    fmt_first: &[Efm],
    state: &mut Qfstate,
    fields: &mut Qffields,
) -> i32 {
    let mut status = qf_get_nextline(state);
    if status != QF_OK {
        return status;
    }

    status = qf_parse_line(
        qfl,
        state.linebuf_ptr().cast(),
        state.linelen,
        fmt_first,
        fields,
    );
    if status != QF_OK {
        return status;
    }

    let fname = if !crate::memory::cstr_is_empty(fields.namebuf.as_ptr().cast())
        || !qfl.qf_dir_stack.is_empty()
    {
        fields.namebuf.as_ptr().cast()
    } else if !qfl.qf_file_stack.is_empty() && fields.valid {
        qfl.currfile_ptr().map_or(ptr::null(), CStr::as_ptr)
    } else {
        ptr::null()
    };

    let dir_ptr = qfl.directory_ptr().map_or(ptr::null(), CStr::as_ptr);
    qf_add_entry(
        qfl,
        dir_ptr,
        fname,
        fields.module.as_ptr().cast(),
        fields.bnr,
        fields.errmsg.as_ptr().cast(),
        fields.lnum,
        fields.end_lnum,
        fields.col,
        fields.end_col,
        u8::from(fields.use_viscol),
        fields.pattern.as_ptr().cast(),
        fields.enr,
        fields.type_,
        None,
        u8::from(fields.valid),
    )
}

pub fn qf_init(
    wp: Option<&mut WinInfoInfo>,
    efile: impl crate::strings::IntoCCharPtr,
    errorformat: impl crate::strings::IntoCCharPtr,
    newlist: i32,
    qf_title: impl crate::strings::IntoCCharPtr,
    enc: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let errorformat = errorformat.into_c_char_ptr();
    let enc = enc.into_c_char_ptr();
    let efile = efile.into_c_char_ptr();
    let qf_title = qf_title.into_c_char_ptr();
    {
        let qi = wp.map_or_else(ql_info_ptr, |w| ll_get_or_alloc_list(w));

        qf_init_ext(
            unsafe { &mut *qi },
            unsafe { (*qi).qf_curlist },
            efile,
            g_st().curbuf,
            None,
            errorformat,
            newlist != 0,
            0,
            0,
            qf_title,
            enc,
        )
    }
}

fn efmpat_to_regpat<T>(
    efmpat: impl crate::strings::IntoCCharPtr,
    regpat: *mut T,
    efminfo: &mut Efm,
    idx: usize,
    mut round: i32,
) -> *mut T {
    let efmpat = efmpat.into_c_char_ptr();
    let mut regpat: *mut u8 = regpat.cast();
    {
        if efminfo.addr[idx] != 0 {
            semsg!(
                gettext(c"E372: Too many %%%c in format string"),
                crate::memory::cstr_first_char(efmpat),
            );
            return ptr::null_mut();
        }
        if (idx != 0
            && idx < FMT_PATTERN_R
            && crate::strings::vim_haschr(c"DXOPQ", i32::from(efminfo.prefix)))
            || (idx == FMT_PATTERN_R
                && !crate::strings::vim_haschr(c"OPQ", i32::from(efminfo.prefix)))
        {
            semsg!(
                gettext(c"E373: Unexpected %%%c in format string"),
                crate::memory::cstr_first_char(efmpat),
            );
            return ptr::null_mut();
        }
        round += 1;
        efminfo.addr[idx] = round as u8;
        unsafe {
            *regpat = b'\\';
        }
        regpat = regpat.wrapping_add(1);
        unsafe {
            *regpat = b'(';
        }
        regpat = regpat.wrapping_add(1);
        #[cfg(windows)]
        if crate::memory::cstr_first_byte(efmpat) == b'f' {
            crate::memory::cstr_cpy(regpat.cast(), c"\\%(\\a:\\)\\=");
            regpat = regpat.wrapping_add(10);
        }
        if crate::memory::cstr_first_byte(efmpat) == b'f'
            && crate::memory::cstr_byte_at(efmpat, 1) != 0
        {
            if crate::memory::cstr_byte_at(efmpat, 1) != b'\\'
                && crate::memory::cstr_byte_at(efmpat, 1) != b'%'
            {
                crate::memory::cstr_cpy(regpat.cast(), c".\\{-1,}");
                regpat = regpat.wrapping_add(7);
            } else {
                crate::memory::cstr_cpy(regpat.cast(), c"\\f\\+");
                regpat = regpat.wrapping_add(4);
            }
        } else {
            let mut srcptr: *const u8 = FMT_PAT[idx].pattern.as_ptr().cast();
            while {
                unsafe {
                    *regpat = *srcptr;
                }
                srcptr = srcptr.wrapping_add(1);
                !crate::memory::cstr_is_empty(regpat.cast())
            } {
                regpat = regpat.wrapping_add(1);
            }
        }
        unsafe {
            *regpat = b'\\';
        }
        regpat = regpat.wrapping_add(1);
        unsafe {
            *regpat = b')';
        }
        regpat = regpat.wrapping_add(1);

        regpat.cast()
    }
}

fn scanf_fmt_to_regpat<T, U>(
    pefmp: *mut *const T,
    efm: impl crate::strings::IntoCCharPtr,
    len: i32,
    regpat: *mut U,
) -> *mut U {
    let efm: *const u8 = efm.into_c_char_ptr().cast();
    let mut regpat: *mut u8 = regpat.cast();
    unsafe {
        let mut efmp: *const u8 = (*pefmp).cast();
        let end = efm.wrapping_add(len as usize);

        if crate::memory::cstr_first_byte(efmp.cast()) == b'['
            || crate::memory::cstr_first_byte(efmp.cast()) == b'\\'
        {
            *regpat = *efmp;
            let ch = *regpat;
            regpat = regpat.wrapping_add(1);
            if ch == b'[' {
                if crate::memory::cstr_byte_at(efmp.cast(), 1) == b'^' {
                    efmp = efmp.wrapping_add(1);
                    *regpat = *efmp;
                    regpat = regpat.wrapping_add(1);
                }
                if efmp < end {
                    efmp = efmp.wrapping_add(1);
                    *regpat = *efmp;
                    regpat = regpat.wrapping_add(1);
                    while efmp < end && {
                        efmp = efmp.wrapping_add(1);
                        *regpat = *efmp;
                        let c = *regpat;
                        regpat = regpat.wrapping_add(1);
                        c != b']'
                    } {}
                    if efmp == end {
                        emsg(gettext(c"E374: Missing ] in format string"));
                        return ptr::null_mut();
                    }
                }
            } else if efmp < end {
                efmp = efmp.wrapping_add(1);
                *regpat = *efmp;
                regpat = regpat.wrapping_add(1);
            }
            *regpat = b'\\';
            regpat = regpat.wrapping_add(1);
            *regpat = b'+';
            regpat = regpat.wrapping_add(1);
        } else {
            semsg!(
                gettext(c"E375: Unsupported %%%c in format string"),
                crate::memory::cstr_first_char(efmp.cast()),
            );
            return ptr::null_mut();
        }

        *pefmp = efmp.cast();
        regpat.cast()
    }
}

fn efm_analyze_prefix<T>(efmp: *const T, efminfo: &mut Efm) -> *const T {
    let mut efmp: *const u8 = efmp.cast();
    {
        if crate::strings::vim_haschr(
            c"+-",
            i32::from(crate::memory::cstr_first_byte(efmp.cast())),
        ) {
            efminfo.flags = unsafe { *efmp };
            efmp = efmp.wrapping_add(1);
        }
        if crate::strings::vim_haschr(
            c"DXAEWINCZGOPQ",
            i32::from(crate::memory::cstr_first_byte(efmp.cast())),
        ) {
            efminfo.prefix = unsafe { *efmp };
        } else {
            semsg!(
                gettext(c"E376: Invalid %%%c in format string prefix"),
                crate::memory::cstr_first_char(efmp.cast()),
            );
            return ptr::null();
        }
        efmp.cast()
    }
}

fn efm_to_regpat(
    efm: impl crate::strings::IntoCCharPtr,
    len: i32,
    fmt_ptr: &mut Efm,
    regpat: impl crate::strings::IntoCCharPtr,
) -> bool {
    let regpat: *mut u8 = regpat.into_c_char_ptr().cast_mut().cast();
    let efm: *const u8 = efm.into_c_char_ptr().cast();
    {
        let mut ptr = regpat;
        unsafe {
            *ptr = b'^';
        }
        ptr = ptr.wrapping_add(1);
        let mut round: i32 = 0;
        let end = efm.wrapping_add(len as usize);
        let mut efmp = efm;
        while efmp < end {
            if crate::memory::cstr_first_byte(efmp.cast()) == b'%' {
                efmp = efmp.wrapping_add(1);
                let mut idx: usize = 0;
                while idx < FMT_PATTERNS {
                    if FMT_PAT[idx].convchar == unsafe { *efmp } {
                        break;
                    }
                    idx += 1;
                }
                if idx < FMT_PATTERNS {
                    ptr = efmpat_to_regpat(efmp.cast(), ptr, fmt_ptr, idx, round);
                    if ptr.is_null() {
                        return false;
                    }
                    round += 1;
                } else if crate::memory::cstr_first_byte(efmp.cast()) == b'*' {
                    efmp = efmp.wrapping_add(1);
                    ptr = scanf_fmt_to_regpat(&raw mut efmp, efm.cast(), len, ptr);
                    if ptr.is_null() {
                        return false;
                    }
                } else if crate::strings::vim_haschr(
                    c"%\\.^$~[",
                    i32::from(crate::memory::cstr_first_byte(efmp.cast())),
                ) {
                    unsafe {
                        *ptr = *efmp;
                    }
                    ptr = ptr.wrapping_add(1);
                } else if crate::memory::cstr_first_byte(efmp.cast()) == b'#' {
                    unsafe {
                        *ptr = b'*';
                    }
                    ptr = ptr.wrapping_add(1);
                } else if crate::memory::cstr_first_byte(efmp.cast()) == b'>' {
                    fmt_ptr.conthere = 1;
                } else if efmp == efm.wrapping_add(1) {
                    efmp = efm_analyze_prefix(efmp, fmt_ptr);
                    if efmp.is_null() {
                        return false;
                    }
                } else {
                    semsg!(
                        gettext(c"E377: Invalid %%%c in format string"),
                        crate::memory::cstr_first_char(efmp.cast()),
                    );
                    return false;
                }
            } else {
                if crate::memory::cstr_first_byte(efmp.cast()) == b'\\' && efmp.wrapping_add(1) < end
                {
                    efmp = efmp.wrapping_add(1);
                } else if crate::strings::vim_haschr(
                    c".*^$~[",
                    i32::from(crate::memory::cstr_first_byte(efmp.cast())),
                ) {
                    unsafe {
                        *ptr = b'\\';
                    }
                    ptr = ptr.wrapping_add(1);
                }
                if !crate::memory::cstr_is_empty(efmp.cast()) {
                    unsafe {
                        *ptr = *efmp;
                    }
                    ptr = ptr.wrapping_add(1);
                }
            }
            efmp = efmp.wrapping_add(1);
        }
        unsafe {
            *ptr = b'$';
        }
        ptr = ptr.wrapping_add(1);
        unsafe {
            *ptr = 0;
        }

        true
    }
}

fn efm_regpat_bufsz(efm: impl crate::strings::IntoCCharPtr) -> usize {
    let efm = efm.into_c_char_ptr();
    {
        let mut sz = (FMT_PATTERNS * 3) + (crate::memory::cstr_len(efm) << 2);
        for item in &FMT_PAT {
            sz += item.pattern.to_bytes().len();
        }
        #[cfg(windows)]
        {
            sz += 12;
        }
        #[cfg(not(windows))]
        {
            sz += 2;
        }
        sz
    }
}

fn efm_option_part_len(efm: impl crate::strings::IntoCCharPtr) -> i32 {
    let efm = efm.into_c_char_ptr();
    {
        let mut len: usize = 0;
        while crate::memory::cstr_byte_at(efm, len) != 0
            && crate::memory::cstr_byte_at(efm, len) != b','
        {
            if crate::memory::cstr_byte_at(efm, len) == b'\\'
                && crate::memory::cstr_byte_at(efm, len + 1) != 0
            {
                len += 1;
            }
            len += 1;
        }
        len as i32
    }
}

fn parse_efm_option(efm: impl crate::strings::IntoCCharPtr) -> Vec<Efm> {
    let mut efm = efm.into_c_char_ptr();
    {
        let mut fmt_list: Vec<Efm> = Vec::new();

        let sz = efm_regpat_bufsz(efm);
        let mut fmtstr = vec![0u8; sz];

        let mut errored = false;
        while !crate::memory::cstr_is_empty(efm) {
            let mut fmt_item = Efm {
                prog: None,
                addr: [0; FMT_PATTERNS],
                prefix: 0,
                flags: 0,
                conthere: 0,
            };

            let len = efm_option_part_len(efm);

            if !efm_to_regpat(efm, len, &mut fmt_item, fmtstr.as_mut_ptr().cast()) {
                errored = true;
                break;
            }
            fmt_item.prog =
                crate::regexp::vim_regcomp_box(fmtstr.as_ptr().cast(), RE_MAGIC + RE_STRING);
            if fmt_item.prog.is_none() {
                errored = true;
                break;
            }
            fmt_list.push(fmt_item);
            efm = skip_to_option_part(efm.wrapping_add(len as usize));
        }

        if errored {
            fmt_list.clear();
            qf_st().fmt_start_idx = None;
        } else if fmt_list.is_empty() {
            emsg(gettext(c"E378: 'errorformat' contains no pattern"));
        }

        fmt_list
    }
}

fn qf_grow_linebuf(state: &mut Qfstate, newsz: usize) -> *mut u8 {
    state.linelen = if newsz > LINE_MAXLEN {
        LINE_MAXLEN - 1
    } else {
        newsz
    };
    if state.growbuf.len() < state.linelen + 1 {
        state.growbuf.resize(state.linelen + 1, 0);
    }
    state.use_growbuf = true;
    state.growbuf.as_mut_ptr()
}

fn qf_get_next_str_line(state: &mut Qfstate, p_str: impl crate::strings::IntoCCharPtr) -> i32 {
    let mut p_str = p_str.into_c_char_ptr().cast_mut();
    {
        if crate::memory::cstr_is_empty(p_str) {
            return QF_END_OF_INPUT;
        }

        let p = vim_strchr(p_str, b'\n' as i32);
        let len = if !p.is_null() {
            crate::memory::ptr_offset_from(p, p_str) as usize + 1
        } else {
            crate::memory::cstr_len(p_str)
        };

        let linebuf = if len > IOSIZE - 2 {
            qf_grow_linebuf(state, len)
        } else {
            state.use_growbuf = false;
            state.linelen = len;
            g_st().iobuff.as_mut_ptr().cast()
        };
        unsafe { ptr::copy_nonoverlapping(p_str.cast(), linebuf, state.linelen) };
        unsafe {
            *linebuf.wrapping_add(state.linelen) = 0;
        }

        p_str = p_str.wrapping_add(len);
        state.source = QfInputSource::Str(p_str.cast());

        QF_OK
    }
}

fn qf_get_next_list_line(state: &mut Qfstate, mut p_li: *mut Listitem) -> i32 {
    unsafe {
        while !p_li.is_null()
            && ((*p_li).li_tv.v_type != VarType::VarString
                || (*p_li).li_tv.vval.v_string().is_null())
        {
            p_li = (*p_li).li_next;
        }

        if p_li.is_null() {
            state.source = QfInputSource::List(ptr::null_mut());
            return QF_END_OF_INPUT;
        }

        let s = (*p_li).li_tv.vval.v_string();
        let len = crate::memory::cstr_len(s);
        let linebuf = if len > IOSIZE - 2 {
            qf_grow_linebuf(state, len)
        } else {
            state.use_growbuf = false;
            state.linelen = len;
            g_st().iobuff.as_mut_ptr().cast()
        };

        xstrlcpy(linebuf.cast(), s, state.linelen + 1);
        state.source = QfInputSource::List((*p_li).li_next);
        QF_OK
    }
}

fn qf_get_next_buf_line(
    state: &mut Qfstate,
    buf: *mut FileBuf,
    buflnum: Linenr,
    lnumlast: Linenr,
) -> i32 {
    if buflnum > lnumlast {
        return QF_END_OF_INPUT;
    }
    let p_buf = ml_get_buf(unsafe { &mut *buf }, buflnum);
    let len = ml_get_buf_len(unsafe { &mut *buf }, buflnum) as usize;
    state.source = QfInputSource::Buf {
        buf,
        buflnum: buflnum + 1,
        lnumlast,
    };

    let linebuf = if len > IOSIZE - 2 {
        qf_grow_linebuf(state, len)
    } else {
        state.use_growbuf = false;
        state.linelen = len;
        g_st().iobuff.as_mut_ptr().cast()
    };
    xstrlcpy(linebuf.cast(), p_buf, state.linelen + 1);
    QF_OK
}

fn qf_get_next_file_line(state: &mut Qfstate, fd: *mut FILE) -> i32 {
    let iobuff_ptr = g_st().iobuff.as_mut_ptr();
    loop {
        os_set_errno(0);
        if unsafe { fgets(iobuff_ptr, IOSIZE as i32, fd) }.is_null() {
            if os_errno() == EINTR {
                continue;
            }
            return QF_END_OF_INPUT;
        }
        break;
    }

    let mut discard = false;
    state.linelen = crate::memory::cstr_len(iobuff_ptr);
    if state.linelen == IOSIZE - 1 && unsafe { *iobuff_ptr.wrapping_add(state.linelen - 1) } != NL {
        if state.growbuf.len() < 2 * (IOSIZE - 1) {
            state.growbuf.resize(2 * (IOSIZE - 1), 0);
        }

        unsafe {
            ptr::copy_nonoverlapping(iobuff_ptr.cast(), state.growbuf.as_mut_ptr(), IOSIZE - 1)
        };
        let mut growbuflen = state.linelen;
        state.growbuf[growbuflen] = 0;

        loop {
            os_set_errno(0);
            let growbufsiz = state.growbuf.len();
            if unsafe {
                fgets(
                    state.growbuf.as_mut_ptr().wrapping_add(growbuflen).cast(),
                    (growbufsiz - growbuflen) as i32,
                    fd,
                )
            }
            .is_null()
            {
                if os_errno() == EINTR {
                    continue;
                }
                break;
            }
            state.linelen =
                crate::memory::cstr_len(state.growbuf.as_ptr().wrapping_add(growbuflen).cast());
            growbuflen += state.linelen;
            if state.growbuf[growbuflen - 1] == b'\n' {
                break;
            }
            if growbufsiz == LINE_MAXLEN {
                discard = true;
                break;
            }

            let new_siz = (2 * growbufsiz).min(LINE_MAXLEN);
            state.growbuf.resize(new_siz, 0);
        }

        if discard {
            loop {
                os_set_errno(0);
                if unsafe { fgets(iobuff_ptr, IOSIZE as i32, fd) }.is_null() {
                    if os_errno() == EINTR {
                        continue;
                    }
                    break;
                }
                if crate::memory::cstr_len(iobuff_ptr) < IOSIZE - 1
                    || unsafe { *iobuff_ptr.wrapping_add(IOSIZE - 2) } == NL
                {
                    break;
                }
            }
        }

        state.use_growbuf = true;
        state.linelen = growbuflen;
    } else {
        state.use_growbuf = false;
    }

    let cur_linebuf = state.linebuf_ptr();
    if state.vc.vc_type != CONV_NONE
        && !cur_linebuf.is_null()
        && !crate::memory::cstr_as_bytes(cur_linebuf.cast()).is_ascii()
        && let Some(line) = string_convert(&state.vc, cur_linebuf.cast(), Some(&mut state.linelen))
    {
        if state.linelen < IOSIZE {
            xstrlcpy(cur_linebuf.cast(), line.as_ptr().cast(), state.linelen + 1);
        } else {
            let cap = (state.linelen + 1).min(LINE_MAXLEN);
            if state.growbuf.len() < cap {
                state.growbuf.resize(cap, 0);
            }
            xstrlcpy(state.growbuf.as_mut_ptr().cast(), line.as_ptr().cast(), cap);
            state.use_growbuf = true;
        }
    }
    QF_OK
}

fn qf_get_nextline(state: &mut Qfstate) -> i32 {
    let status = match state.source {
        QfInputSource::None => QF_FAIL,
        QfInputSource::File(ref fd) => qf_get_next_file_line(state, fd.as_ptr()),
        QfInputSource::Str(p_str) => qf_get_next_str_line(state, p_str.cast()),
        QfInputSource::List(p_li) => qf_get_next_list_line(state, p_li),
        QfInputSource::Buf {
            buf,
            buflnum,
            lnumlast,
        } => qf_get_next_buf_line(state, buf, buflnum, lnumlast),
    };

    if status != QF_OK {
        return status;
    }

    let linebuf = state.linebuf_ptr();
    if state.linelen > 0 && unsafe { *linebuf.wrapping_add(state.linelen - 1) } == b'\n' {
        unsafe {
            *linebuf.wrapping_add(state.linelen - 1) = 0;
        }
        #[cfg(windows)]
        if state.linelen > 1 && unsafe { *linebuf.wrapping_add(state.linelen - 2) } == b'\r' {
            unsafe {
                *linebuf.wrapping_add(state.linelen - 2) = 0;
            }
        }
    }

    remove_bom(linebuf.cast());

    QF_OK
}

#[inline]
fn qf_stack_empty(qi: Option<&QfInfo>) -> bool {
    qi.is_none() || { qi.unwrap().qf_listcount <= 0 }
}

#[inline]
fn qf_list_empty(qfl: Option<&QfList>) -> bool {
    qfl.is_none() || { qfl.unwrap().qf_items.is_empty() }
}

#[inline]
fn qf_list_has_valid_entries(qfl: &QfList) -> bool {
    !qf_list_empty(Some(qfl)) && !qfl.qf_nonevalid
}

#[inline]
fn qf_get_list(qi: &mut QfInfo, idx: i32) -> &'static mut QfList {
    unsafe { &mut *qi.qf_lists.as_mut_ptr().wrapping_add(idx as usize) }
}

fn qf_parse_line(
    qfl: &mut QfList,
    linebuf: impl crate::strings::IntoCCharPtr,
    mut linelen: usize,
    fmt_first: &[Efm],
    fields: &mut Qffields,
) -> i32 {
    let mut linebuf = linebuf.into_c_char_ptr();
    {
        let fmt_len = fmt_first.len();
        loop {
            let start_idx = qf_st().fmt_start_idx;
            qf_st().fmt_start_idx = None;
            let mut fmt_idx = start_idx.unwrap_or(0);
            let mut fmt_opt: Option<&Efm> = None;

            let mut idx: i32 = 0;
            let mut tail = ptr::null_mut();
            let mut status;

            fields.valid = true;
            while fmt_idx < fmt_len {
                let cur_fmt = &fmt_first[fmt_idx];
                idx = i32::from(cur_fmt.prefix);
                status = qf_parse_get_fields(
                    linebuf,
                    linelen,
                    cur_fmt,
                    fields,
                    i32::from(qfl.qf_multiline),
                    i32::from(qfl.qf_multiscan),
                    &raw mut tail,
                );
                if status == QF_NOMEM {
                    return status;
                }
                if status == QF_OK {
                    fmt_opt = Some(cur_fmt);
                    break;
                }
                fmt_idx += 1;
            }
            qfl.qf_multiscan = false;

            if fmt_opt.is_none() || idx == i32::from(b'D') || idx == i32::from(b'X') {
                if fmt_opt.is_some() {
                    status = qf_parse_dir_pfx(idx, fields, qfl);
                    if status != QF_OK {
                        return status;
                    }
                }
                status = qf_parse_line_nomatch(linebuf, linelen, fields);
                if status != QF_OK {
                    return status;
                }

                if fmt_opt.is_none() {
                    qfl.qf_multiline = false;
                    qfl.qf_multiignore = false;
                }
            } else if let Some(fmt_ptr) = fmt_opt {
                if fmt_ptr.conthere != 0 {
                    qf_st().fmt_start_idx = Some(fmt_idx);
                }

                if crate::strings::vim_haschr(c"AEWIN", idx) {
                    qfl.qf_multiline = true;
                    qfl.qf_multiignore = false;
                } else if crate::strings::vim_haschr(c"CZ", idx) {
                    status = qf_parse_multiline_pfx(idx, qfl, fields);
                    if status != QF_OK {
                        return status;
                    }
                } else if crate::strings::vim_haschr(c"OPQ", idx) {
                    status = qf_parse_file_pfx(idx, fields, qfl, tail);
                    if status == QF_MULTISCAN {
                        let s = skipwhite(tail);
                        let new_linelen = crate::memory::cstr_len(s);
                        if new_linelen >= linelen {
                            return QF_IGNORE_LINE;
                        }
                        linebuf = s;
                        linelen = new_linelen;
                        continue;
                    }
                }
                if fmt_ptr.flags == b'-' {
                    if qfl.qf_multiline {
                        qfl.qf_multiignore = true;
                    }
                    return QF_IGNORE_LINE;
                }
            }

            return QF_OK;
        }
    }
}

fn qf_setup_state(
    pstate: &mut Qfstate,
    enc: impl crate::strings::IntoCCharPtr,
    efile: impl crate::strings::IntoCCharPtr,
    tv: Option<&mut TypVal>,
    buf: *mut FileBuf,
    lnumfirst: Linenr,
    lnumlast: Linenr,
) -> bool {
    let enc = enc.into_c_char_ptr();
    let efile = efile.into_c_char_ptr();
    {
        pstate.vc.vc_type = CONV_NONE;
        if !crate::memory::cstr_is_empty(enc) {
            convert_setup(&mut pstate.vc, enc, opt_vars().p_enc);
        }

        if !efile.is_null() {
            let raw_fd = if crate::memory::cstr_as_opt_bytes(efile) == Some(b"-") {
                unsafe { fdopen(os_open_stdin_fd(), c"r".as_ptr()) }
            } else {
                os_fopen(efile, c"r").cast::<FILE>()
            };
            let Some(fd) = crate::os::fs::CFile::from_raw(raw_fd) else {
                semsg!(gettext(E_OPENERRF), efile);
                return false;
            };
            pstate.source = QfInputSource::File(fd);
        } else if let Some(tv) = tv {
            if tv.v_type == VarType::VarString {
                pstate.source = QfInputSource::Str(tv.vval.v_string().cast());
            } else if tv.v_type == VarType::VarList {
                pstate.source =
                    QfInputSource::List(tv_list_first(unsafe { tv.vval.v_list().as_ref() }));
            }
        } else {
            pstate.source = QfInputSource::Buf {
                buf,
                buflnum: lnumfirst,
                lnumlast,
            };
        }

        true
    }
}

fn qf_init_ext(
    qi: &mut QfInfo,
    mut qf_idx: i32,
    efile: impl crate::strings::IntoCCharPtr,
    buf: *mut FileBuf,
    mut tv: Option<&mut TypVal>,
    errorformat: impl crate::strings::IntoCCharPtr,
    newlist: bool,
    lnumfirst: Linenr,
    lnumlast: Linenr,
    qf_title: impl crate::strings::IntoCCharPtr,
    enc: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let errorformat = errorformat.into_c_char_ptr();
    let enc = enc.into_c_char_ptr();
    let efile = efile.into_c_char_ptr();
    let qf_title = qf_title.into_c_char_ptr();
    unsafe {
        let mut state = Qfstate::default();
        let mut fields = Qffields::default();
        let mut old_count: usize = 0;
        let mut retval: i32 = -1;

        qf_st().qf_last_bufname = None;

        if !qf_setup_state(
            &mut state,
            enc,
            efile,
            tv.as_deref_mut(),
            buf,
            lnumfirst,
            lnumlast,
        ) {
            if qf_idx == qi.qf_curlist {
                qf_update_buffer(qi, old_count);
            }
            return retval;
        }

        let qfl: &mut QfList;
        let mut adding = false;
        if newlist || qf_idx == qi.qf_listcount {
            qf_new_list(qi, qf_title);
            qf_idx = qi.qf_curlist;
            qfl = qf_get_list(&mut *qi, qf_idx);
        } else {
            adding = true;
            qfl = qf_get_list(&mut *qi, qf_idx);
            if !qf_list_empty(Some(&*qfl)) {
                old_count = qfl.qf_items.len();
            }
        }

        let efm = if errorformat == opt_vars().p_efm
            && tv.is_none()
            && !buf.is_null()
            && !crate::memory::cstr_is_empty((*buf).b_p_efm)
        {
            (*buf).b_p_efm
        } else {
            errorformat
        };

        let efm_changed = (&raw const qf_st().last_efm_cache)
            .as_ref()
            .unwrap()
            .as_ref()
            .is_none_or(|c| {
                crate::memory::cstr_as_bytes(c.as_ptr()) != crate::memory::cstr_as_bytes(efm)
            });
        if efm_changed {
            qf_st().last_efm_cache = None;
            qf_st().fmt_cache.clear();
            qf_st().fmt_start_idx = None;

            qf_st().fmt_cache = parse_efm_option(efm);
            if !qf_st().fmt_cache.is_empty() {
                qf_st().last_efm_cache = Some(crate::memory::cstr_to_cstring(efm));
            }
        }

        let mut ok = !qf_st().fmt_cache.is_empty();
        if ok {
            g_st().got_int = false;
            while !g_st().got_int {
                let status = qf_init_process_nextline(
                    qfl,
                    qf_st().fmt_cache.as_slice(),
                    &mut state,
                    &mut fields,
                );
                if status == QF_END_OF_INPUT {
                    break;
                }
                if status == QF_FAIL {
                    ok = false;
                    break;
                }
                line_breakcheck();
            }
        }

        if ok {
            let file_ok = match state.source {
                QfInputSource::File(ref fd) => ferror(fd.as_ptr()) == 0,
                _ => true,
            };
            if file_ok {
                if qfl.qf_index == 0 {
                    qfl.qf_index = 1;
                    qfl.qf_nonevalid = true;
                } else {
                    qfl.qf_nonevalid = false;
                }
                retval = qfl.count();
            } else {
                emsg(gettext(E_READERRF));
                ok = false;
            }
        }

        if !ok && !adding {
            qf_free(qfl);
            qi.qf_listcount -= 1;
            if qi.qf_curlist > 0 {
                qi.qf_curlist -= 1;
            }
        }

        if qf_idx == qi.qf_curlist {
            qf_update_buffer(qi, old_count);
        }

        retval
    }
}

fn qf_store_title(qfl: &mut QfList, title: impl crate::strings::IntoCCharPtr) {
    let title = title.into_c_char_ptr();
    {
        qfl.qf_title = if title.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(title))
        };
    }
}

fn qf_cmdtitle(cmd: impl crate::strings::IntoCCharPtr) -> std::ffi::CString {
    let cmd = cmd.into_c_char_ptr();
    let mut buf = [0u8; IOSIZE];
    let len = vim_snprintf_safelen!(buf.as_mut_ptr().cast(), IOSIZE, c":%s", cmd);
    crate::memory::cstr_from_bytes(&buf[..len])
}

#[inline]
fn qf_get_curlist(qi: &mut QfInfo) -> &'static mut QfList {
    qf_get_list(qi, qi.qf_curlist)
}

fn qf_pop_stack(qi: &mut QfInfo, adjust: bool) {
    let cnt = qi.qf_listcount as usize;
    let lists = &mut qi.qf_lists.as_mut_slice()[..cnt];
    lists.rotate_left(1);
    lists[cnt - 1] = QfList::default();

    if adjust {
        qi.qf_listcount -= 1;
        if qi.qf_curlist == 0 {
            qi.qf_curlist = qi.qf_listcount - 1;
        } else {
            qi.qf_curlist -= 1;
        }
    }
}

fn qf_new_list(qi: &mut QfInfo, qf_title: impl crate::strings::IntoCCharPtr) {
    let qf_title = qf_title.into_c_char_ptr();
    while qi.qf_listcount > qi.qf_curlist + 1 {
        qi.qf_listcount -= 1;
        let lc = qi.qf_listcount;
        qf_free(qf_get_list(&mut *qi, lc));
    }

    if qi.qf_listcount == qi.qf_maxcount {
        qf_pop_stack(qi, false);
        qi.qf_curlist = qi.qf_listcount - 1;
    } else {
        qi.qf_curlist = qi.qf_listcount;
        qi.qf_listcount += 1;
    }

    let qfl = qf_get_curlist(&mut *qi);
    *qfl = QfList::default();
    qf_store_title(qfl, qf_title);
    qfl.qfl_type = qi.qfl_type;
    qf_st().last_qf_id += 1;
    qfl.qf_id = qf_st().last_qf_id;
    qfl.qf_has_user_data = false;
}

fn qf_parse_fmt_f(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields, prefix: i32) -> i32 {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QF_FAIL;
    }

    let c = unsafe { *rmp.endp[midx] };
    unsafe {
        *rmp.endp[midx] = NUL;
    }
    expand_env(
        rmp.startp[midx],
        fields.namebuf.as_mut_ptr(),
        CMDBUFFSIZE as i32,
    );
    unsafe {
        *rmp.endp[midx] = c;
    }

    if crate::strings::vim_haschr(c"OPQ", prefix)
        && !os_path_exists(fields.namebuf.as_ptr().cast())
    {
        return QF_FAIL;
    }

    QF_OK
}

fn qf_parse_fmt_b(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    let bnr =
        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as i32;
    if buflist_findnr(bnr).is_null() {
        return QF_FAIL;
    }
    fields.bnr = bnr;
    QF_OK
}

fn qf_parse_fmt_n(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    fields.enr =
        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as i32;
    QF_OK
}

fn qf_parse_fmt_l(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    fields.lnum =
        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as Linenr;
    QF_OK
}

fn qf_parse_fmt_e(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    fields.end_lnum =
        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as Linenr;
    QF_OK
}

fn qf_parse_fmt_c(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    fields.col =
        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as i32;
    QF_OK
}

fn qf_parse_fmt_k(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    fields.end_col =
        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as i32;
    QF_OK
}

fn qf_parse_fmt_t(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    fields.type_ = unsafe { *rmp.startp[midx].cast::<u8>() };
    QF_OK
}

fn copy_nonerror_line(
    linebuf: impl crate::strings::IntoCCharPtr,
    linelen: usize,
    fields: &mut Qffields,
) -> i32 {
    let linebuf = linebuf.into_c_char_ptr();
    if linelen >= fields.errmsg.len() {
        fields.errmsg.resize(linelen + 1, 0);
    }
    xstrlcpy(fields.errmsg.as_mut_ptr().cast(), linebuf, linelen + 1);
    QF_OK
}

fn qf_parse_fmt_m(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QF_FAIL;
    }
    let len = crate::memory::ptr_offset_from(rmp.endp[midx], rmp.startp[midx]) as usize;
    if len >= fields.errmsg.len() {
        fields.errmsg.resize(len + 1, 0);
    }
    xstrlcpy(fields.errmsg.as_mut_ptr().cast(), rmp.startp[midx], len + 1);
    QF_OK
}

fn qf_parse_fmt_r<T>(rmp: &mut Regmatch, midx: usize, tail: *mut *mut T) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    unsafe {
        *tail = rmp.startp[midx].cast();
    }
    QF_OK
}

fn qf_parse_fmt_p(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QF_FAIL;
    }
    fields.col = 0;
    let mut match_ptr = rmp.startp[midx];
    while match_ptr != rmp.endp[midx] {
        fields.col += 1;
        if crate::memory::cstr_first_byte(match_ptr) == TAB {
            fields.col += 7;
            fields.col -= fields.col % 8;
        }
        match_ptr = match_ptr.wrapping_add(1);
    }
    fields.col += 1;
    fields.use_viscol = true;
    QF_OK
}

fn qf_parse_fmt_v(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() {
        return QF_FAIL;
    }
    fields.col =
        crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as i32;
    fields.use_viscol = true;
    QF_OK
}

fn qf_parse_fmt_s(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QF_FAIL;
    }
    let mut len = crate::memory::ptr_offset_from(rmp.endp[midx], rmp.startp[midx]) as usize;
    len = len.min(CMDBUFFSIZE - 5);
    let pat_ptr = fields.pattern.as_mut_ptr();
    crate::memory::cstr_cpy(pat_ptr.cast(), c"^\\V");
    {
        let _dst: *mut _ = pat_ptr.cast();
        let _dsize: usize = len + 4;
        let _dlen = crate::memory::cstr_len(_dst);
        crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), rmp.startp[midx], _dsize - _dlen);
    };
    unsafe {
        *pat_ptr.wrapping_add(len + 3) = b'\\';
        *pat_ptr.wrapping_add(len + 4) = b'$';
        *pat_ptr.wrapping_add(len + 5) = 0;
    }
    QF_OK
}

fn qf_parse_fmt_o(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> i32 {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QF_FAIL;
    }
    let len = crate::memory::ptr_offset_from(rmp.endp[midx], rmp.startp[midx]) as usize;
    let mod_ptr: *mut _ = fields.module.as_mut_ptr().cast();
    let dsize = (crate::memory::cstr_len(mod_ptr) + len + 1).min(CMDBUFFSIZE);
    {
        let _dst: *mut _ = mod_ptr;
        let _dsize: usize = dsize;
        let _dlen = crate::memory::cstr_len(_dst);
        crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), rmp.startp[midx], _dsize - _dlen);
    };
    QF_OK
}

fn qf_parse_match<T>(
    linebuf: impl crate::strings::IntoCCharPtr,
    linelen: usize,
    fmt_ptr: &Efm,
    regmatch: &mut Regmatch,
    fields: &mut Qffields,
    qf_multiline: i32,
    qf_multiscan: i32,
    tail: *mut *mut T,
) -> i32 {
    let linebuf = linebuf.into_c_char_ptr();
    let idx = fmt_ptr.prefix;

    if (idx == b'C' || idx == b'Z') && qf_multiline == 0 {
        return QF_FAIL;
    }
    if crate::strings::vim_haschr(c"EWIN", i32::from(idx)) {
        fields.type_ = idx;
    } else {
        fields.type_ = 0;
    }

    for i in 0..FMT_PATTERNS {
        let mut status = QF_OK;
        let midx = fmt_ptr.addr[i] as isize;
        if i == 0 && midx > 0 {
            status = qf_parse_fmt_f(regmatch, midx as usize, fields, i32::from(idx));
        } else if i == FMT_PATTERN_M {
            if fmt_ptr.flags == b'+' && qf_multiscan == 0 {
                status = copy_nonerror_line(linebuf, linelen, fields);
            } else if midx > 0 {
                status = qf_parse_fmt_m(regmatch, midx as usize, fields);
            }
        } else if i == FMT_PATTERN_R && midx > 0 {
            status = qf_parse_fmt_r(regmatch, midx as usize, tail);
        } else if midx > 0 {
            let m = midx as usize;
            status = match i {
                1 => qf_parse_fmt_b(regmatch, m, fields),
                2 => qf_parse_fmt_n(regmatch, m, fields),
                3 => qf_parse_fmt_l(regmatch, m, fields),
                4 => qf_parse_fmt_e(regmatch, m, fields),
                5 => qf_parse_fmt_c(regmatch, m, fields),
                6 => qf_parse_fmt_k(regmatch, m, fields),
                7 => qf_parse_fmt_t(regmatch, m, fields),
                8 => qf_parse_fmt_m(regmatch, m, fields),
                10 => qf_parse_fmt_p(regmatch, m, fields),
                11 => qf_parse_fmt_v(regmatch, m, fields),
                12 => qf_parse_fmt_s(regmatch, m, fields),
                13 => qf_parse_fmt_o(regmatch, m, fields),
                _ => QF_OK,
            };
        }

        if status != QF_OK {
            return status;
        }
    }

    QF_OK
}

fn qf_parse_get_fields<T>(
    linebuf: impl crate::strings::IntoCCharPtr,
    linelen: usize,
    fmt_ptr: &Efm,
    fields: &mut Qffields,
    qf_multiline: i32,
    qf_multiscan: i32,
    tail: *mut *mut T,
) -> i32 {
    let linebuf = linebuf.into_c_char_ptr();
    {
        if qf_multiscan != 0 && !crate::strings::vim_haschr(c"OPQ", i32::from(fmt_ptr.prefix)) {
            return QF_FAIL;
        }

        unsafe {
            *fields.namebuf.as_mut_ptr() = 0;
        }
        fields.bnr = 0;
        unsafe {
            *fields.module.as_mut_ptr() = 0;
        }
        unsafe {
            *fields.pattern.as_mut_ptr() = 0;
        }
        if qf_multiscan == 0 {
            unsafe {
                *fields.errmsg.as_mut_ptr() = 0;
            }
        }
        fields.lnum = 0;
        fields.end_lnum = 0;
        fields.col = 0;
        fields.end_col = 0;
        fields.use_viscol = false;
        fields.enr = -1;
        fields.type_ = 0;
        unsafe {
            *tail = ptr::null_mut();
        }

        let mut regmatch: Regmatch = Regmatch::new();
        regmatch.rm_ic = true;
        regmatch.regprog = crate::regexp::regprog_as_ptr(&fmt_ptr.prog);
        let r = vim_regexec(&mut regmatch, linebuf, 0);
        let mut status = QF_FAIL;
        if r {
            status = qf_parse_match(
                linebuf,
                linelen,
                fmt_ptr,
                &mut regmatch,
                fields,
                qf_multiline,
                qf_multiscan,
                tail,
            );
        }

        status
    }
}

fn qf_parse_dir_pfx(idx: i32, fields: &mut Qffields, qfl: &mut QfList) -> i32 {
    if idx == i32::from(b'D') {
        if crate::memory::cstr_is_empty(fields.namebuf.as_ptr().cast()) {
            emsg(gettext(c"E379: Missing or empty directory name"));
            return QF_FAIL;
        }
        qf_push_dir(fields.namebuf.as_ptr().cast(), &mut qfl.qf_dir_stack, false);
    } else if idx == i32::from(b'X') {
        qf_pop_dir(&mut qfl.qf_dir_stack);
    }

    QF_OK
}

fn qf_parse_file_pfx(
    idx: i32,
    fields: &mut Qffields,
    qfl: &mut QfList,
    tail: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let tail = tail.into_c_char_ptr();
    {
        fields.valid = false;
        if crate::memory::cstr_is_empty(fields.namebuf.as_ptr().cast())
            || os_path_exists(fields.namebuf.as_ptr().cast())
        {
            if !crate::memory::cstr_is_empty(fields.namebuf.as_ptr().cast()) && idx == i32::from(b'P')
            {
                qf_push_dir(fields.namebuf.as_ptr().cast(), &mut qfl.qf_file_stack, true);
            } else if idx == i32::from(b'Q') {
                qf_pop_dir(&mut qfl.qf_file_stack);
            }
            unsafe {
                *fields.namebuf.as_mut_ptr() = 0;
            }
            if !crate::memory::cstr_is_empty(tail) {
                qfl.qf_multiscan = true;
                return QF_MULTISCAN;
            }
        }

        QF_OK
    }
}

fn qf_parse_line_nomatch(
    linebuf: impl crate::strings::IntoCCharPtr,
    linelen: usize,
    fields: &mut Qffields,
) -> i32 {
    let linebuf = linebuf.into_c_char_ptr();
    {
        unsafe {
            *fields.namebuf.as_mut_ptr() = 0;
        }
        fields.lnum = 0;
        fields.valid = false;

        copy_nonerror_line(linebuf, linelen, fields)
    }
}

fn qf_parse_multiline_pfx(idx: i32, qfl: &mut QfList, fields: &mut Qffields) -> i32 {
    if !qfl.qf_multiignore {
        let Some(qfprev) = qfl.qf_items.last_mut() else {
            return QF_FAIL;
        };
        let qfprev: *mut Qfline = qfprev;
        let qfprev = unsafe { &mut *qfprev };
        if !crate::memory::cstr_is_empty(fields.errmsg.as_ptr().cast()) {
            let mut bytes = qfprev.qf_text.as_bytes().to_vec();
            bytes.push(b'\n');
            bytes.extend_from_slice(crate::memory::cstr_as_bytes(fields.errmsg.as_ptr().cast()));
            qfprev.qf_text = std::ffi::CString::new(bytes).unwrap();
        }
        if qfprev.qf_nr == -1 {
            qfprev.qf_nr = fields.enr;
        }
        if vim_isprintc(i32::from(fields.type_)) && qfprev.qf_type == 0 {
            qfprev.qf_type = fields.type_;
        }

        if qfprev.qf_lnum == 0 {
            qfprev.qf_lnum = fields.lnum;
        }
        if qfprev.qf_end_lnum == 0 {
            qfprev.qf_end_lnum = fields.end_lnum;
        }
        if qfprev.qf_col == 0 {
            qfprev.qf_col = fields.col;
            qfprev.qf_viscol = u8::from(fields.use_viscol);
        }
        if qfprev.qf_end_col == 0 {
            qfprev.qf_end_col = fields.end_col;
        }
        if qfprev.qf_fnum == 0 {
            let fname = if !crate::memory::cstr_is_empty(fields.namebuf.as_ptr().cast())
                || !qfl.qf_dir_stack.is_empty()
            {
                fields.namebuf.as_ptr().cast()
            } else if !qfl.qf_file_stack.is_empty() && fields.valid {
                qfl.currfile_ptr().map_or(ptr::null(), CStr::as_ptr)
            } else {
                ptr::null()
            };
            let dir_ptr = qfl.directory_ptr().map_or(ptr::null(), CStr::as_ptr);
            qfprev.qf_fnum = qf_get_fnum(qfl, dir_ptr, fname);
        }
    }
    if idx == i32::from(b'Z') {
        qfl.qf_multiline = false;
        qfl.qf_multiignore = false;
    }
    line_breakcheck();

    QF_IGNORE_LINE
}

fn locstack_queue_delreq(qi: &mut QfInfo) {
    qf_st().qf_delq.push(qi);
}

pub fn qf_stack_get_bufnr() -> i32 {
    qf_st().ql_info_actual.qf_bufnr
}

fn wipe_qf_buffer(qi: &mut QfInfo) {
    if qi.qf_bufnr == INVALID_QFBUFNR {
        return;
    }

    let qfbuf = buflist_findnr(qi.qf_bufnr);
    if !qfbuf.is_null() && unsafe { (*qfbuf).b_nwindows } == 0 {
        let mut buf_was_null = false;
        if curwin_mut().w_buffer.is_null() {
            curwin_mut().w_buffer = g_st().curbuf;
            buf_was_null = true;
        }

        close_buffer(ptr::null_mut(), qfbuf, DOBUF_WIPE, false, false, false);
        qi.qf_bufnr = INVALID_QFBUFNR;
        if buf_was_null {
            curwin_mut().w_buffer = ptr::null_mut();
        }
    }
}

fn qf_free_list_stack_items(qi: &mut QfInfo) {
    for i in 0..qi.qf_listcount {
        qf_free(qf_get_list(&mut *qi, i));
    }
}

fn ll_free_all(pqi: *mut *mut QfInfo) {
    unsafe {
        let qi = *pqi;
        if qi.is_null() {
            return;
        }
        *pqi = ptr::null_mut();

        if qf_st().quickfix_busy > 0 {
            locstack_queue_delreq(&mut *qi);
            return;
        }

        (*qi).qf_refcount -= 1;
        if (*qi).qf_refcount < 1 {
            wipe_qf_buffer(&mut *qi);
            drop(Box::from_raw(qi));
        }
    }
}

pub fn qf_free_all(wp: Option<&mut WinInfoInfo>) {
    let qi = ql_info_ptr();
    if let Some(wp) = wp {
        ll_free_all(&raw mut wp.w_llist);
        ll_free_all(&raw mut wp.w_llist_ref);
    } else if !qi.is_null() {
        qf_free_list_stack_items(unsafe { &mut *qi });
    }
}

fn incr_quickfix_busy() {
    qf_st().quickfix_busy += 1;
}

fn decr_quickfix_busy() {
    qf_st().quickfix_busy -= 1;
    if qf_st().quickfix_busy == 0 {
        while let Some(mut qi) = qf_st().qf_delq.pop() {
            ll_free_all(&raw mut qi);
        }
    }
}

pub fn check_quickfix_busy() {
    if qf_st().quickfix_busy != 0 {
        semsg!(
            c"quickfix_busy not zero on exit: %lld",
            i64::from(qf_st().quickfix_busy),
        );
    }
}

fn qf_add_entry(
    qfl: &mut QfList,
    dir: impl crate::strings::IntoCCharPtr,
    fname: impl crate::strings::IntoCCharPtr,
    module: impl crate::strings::IntoCCharPtr,
    bufnum: i32,
    mesg: impl crate::strings::IntoCCharPtr,
    lnum: Linenr,
    end_lnum: Linenr,
    col: i32,
    end_col: i32,
    vis_col: u8,
    pattern: impl crate::strings::IntoCCharPtr,
    nr: i32,
    mut type_: u8,
    user_data: Option<&TypVal>,
    valid: u8,
) -> i32 {
    let dir = dir.into_c_char_ptr().cast_mut();
    let fname = fname.into_c_char_ptr().cast_mut();
    let module = module.into_c_char_ptr();
    let mesg = mesg.into_c_char_ptr();
    let pattern = pattern.into_c_char_ptr();
    unsafe {
        let buf: *mut FileBuf;
        let mut qfp = Qfline::new();

        if bufnum != 0 {
            buf = buflist_findnr(bufnum);
            qfp.qf_fnum = bufnum;
            if !buf.is_null() {
                (*buf).b_has_qf_entry |= if is_qf_list(qfl) {
                    BUF_HAS_QF_ENTRY
                } else {
                    BUF_HAS_LL_ENTRY
                };
            }
        } else {
            qfp.qf_fnum = qf_get_fnum(qfl, dir, fname);
            buf = buflist_findnr(qfp.qf_fnum);
        }
        let fullname = if !fname.is_null() {
            fix_fname(fname)
        } else {
            None
        };
        qfp.qf_fname = None;
        if !buf.is_null()
            && !(*buf).b_ffname.is_null()
            && let Some(ref full) = fullname
            && !path_equal(full.as_ptr(), (*buf).b_ffname, K_PATH_CMP_LITERAL)
        {
            let p = path_try_shorten_fname(full.as_ptr().cast_mut());
            if !p.is_null() {
                qfp.qf_fname = Some(crate::memory::cstr_to_cstring(p));
            }
        }
        drop(fullname);
        qfp.qf_text = crate::memory::cstr_to_cstring(mesg);
        qfp.qf_lnum = lnum;
        qfp.qf_end_lnum = end_lnum;
        qfp.qf_col = col;
        qfp.qf_end_col = end_col;
        qfp.qf_viscol = vis_col;
        if let Some(user_data) = user_data
            && user_data.v_type != VarType::VarUnknown
        {
            tv_copy(user_data, &mut qfp.qf_user_data);
            qfl.qf_has_user_data = true;
        } else {
            qfp.qf_user_data.v_type = VarType::VarUnknown;
        }
        qfp.qf_pattern = if crate::memory::cstr_is_empty(pattern) {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(pattern))
        };
        qfp.qf_module = if crate::memory::cstr_is_empty(module) {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(module))
        };
        qfp.qf_nr = nr;
        if type_ != 1 && !vim_isprintc(i32::from(type_)) {
            type_ = 0;
        }
        qfp.qf_type = type_;
        qfp.qf_valid = valid;
        qfp.qf_cleared = 0;

        if qf_list_empty(Some(&*qfl)) {
            qfl.qf_index = 0;
        }
        qfl.qf_items.push(qfp);
        if qfl.qf_index == 0 && valid != 0 {
            qfl.qf_index = qfl.count();
        }

        QF_OK
    }
}

pub fn qf_resize_stack(n: i32) {
    unsafe {
        qf_resize_stack_base(&mut *ql_info_ptr(), n);
    }
}

pub fn ll_resize_stack(wp: &mut WinInfoInfo, n: i32) {
    if is_ll_window(wp) {
        qf_sync_llw_to_win(wp);
    } else {
        qf_sync_win_to_llw(wp);
    }

    let qi = ll_get_or_alloc_list(wp);
    qf_resize_stack_base(&mut *qi, n);
}

fn qf_resize_stack_base(qi: &mut QfInfo, n: i32) {
    if n == qi.qf_maxcount {
        return;
    } else if n < qi.qf_maxcount && n < qi.qf_listcount {
        let amount_to_rm = qi.qf_listcount - n;
        for _ in 0..amount_to_rm {
            qf_pop_stack(qi, true);
        }
    }

    qi.qf_lists.resize_with(n as usize, QfList::default);
    qi.qf_maxcount = n;

    qf_update_buffer(qi, 0);
}

pub fn qf_init_stack() {
    let _ = qf_alloc_stack(QFLT_QUICKFIX, opt_vars().p_chi as i32);
}

fn qf_sync_llw_to_win(llw: &mut WinInfoInfo) {
    unsafe {
        let wp = qf_find_win_with_loclist(&*llw.w_llist_ref);
        if !wp.is_null() {
            (*wp).w_onebuf_opt.wo_lhi = llw.w_onebuf_opt.wo_lhi;
        }
    }
}

fn qf_sync_win_to_llw(pwp: &mut WinInfoInfo) {
    let llw = pwp.w_llist;
    if !llw.is_null() {
        for wp in win_iter_mut(tab_firstwin(curtab_mut())) {
            if wp.w_llist_ref == llw && bt_quickfix(unsafe { wp.w_buffer.as_ref() }) {
                wp.w_onebuf_opt.wo_lhi = pwp.w_onebuf_opt.wo_lhi;
                return;
            }
        }
    }
}

fn qf_alloc_stack(qfltype: Qfltype, n: i32) -> &'static mut QfInfo {
    let qi = if qfltype == QFLT_QUICKFIX {
        &raw mut qf_st().ql_info_actual
    } else {
        Box::into_raw(Box::new(QfInfo {
            qf_refcount: 1,
            qf_listcount: 0,
            qf_curlist: 0,
            qf_maxcount: 0,
            qf_lists: Vec::new(),
            qfl_type: 0,
            qf_bufnr: 0,
        }))
    };
    let qi = unsafe { &mut *qi };
    qi.qfl_type = qfltype;
    qi.qf_bufnr = INVALID_QFBUFNR;
    qi.qf_lists.clear();
    qi.qf_lists.resize_with(n as usize, QfList::default);
    qi.qf_maxcount = n;

    &mut *qi
}

fn ll_get_or_alloc_list(wp: &mut WinInfoInfo) -> &'static mut QfInfo {
    if is_ll_window(wp) {
        return unsafe { &mut *wp.w_llist_ref };
    }

    ll_free_all(&raw mut wp.w_llist_ref);

    if wp.w_llist.is_null() {
        wp.w_llist = qf_alloc_stack(QFLT_LOCATION, wp.w_onebuf_opt.wo_lhi as i32);
    }

    unsafe { &mut *wp.w_llist }
}

fn qf_cmd_get_stack(eap: &mut Exarg, print_emsg: bool) -> *mut QfInfo {
    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());

    if is_loclist_cmd(eap.cmdidx) {
        qi = get_loc_list(curwin_mut());
        if qi.is_null() {
            if print_emsg {
                emsg(gettext(E_LOCLIST));
            }
            return ptr::null_mut();
        }
    }

    qi
}

fn qf_cmd_get_or_alloc_stack(eap: &Exarg, pwinp: *mut *mut WinInfoInfo) -> *mut QfInfo {
    let mut qi = ql_info_ptr();
    if is_loclist_cmd(eap.cmdidx) {
        qi = ll_get_or_alloc_list(curwin_mut());
        unsafe {
            *pwinp = curwin_ptr();
        }
    }
    qi
}

fn copy_loclist_entries(from_qfl: &QfList, to_qfl: &mut QfList) -> bool {
    for from_qfp in &from_qfl.qf_items {
        if g_st().got_int {
            break;
        }
        if qf_add_entry(
            to_qfl,
            ptr::null_mut(),
            ptr::null_mut(),
            from_qfp
                .qf_module
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            0,
            from_qfp.qf_text.as_ptr().cast_mut(),
            from_qfp.qf_lnum,
            from_qfp.qf_end_lnum,
            from_qfp.qf_col,
            from_qfp.qf_end_col,
            from_qfp.qf_viscol,
            from_qfp
                .qf_pattern
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            from_qfp.qf_nr,
            0,
            Some(&from_qfp.qf_user_data),
            from_qfp.qf_valid,
        ) == QF_FAIL
        {
            return false;
        }

        let prevp = to_qfl.qf_items.last_mut().unwrap();
        prevp.qf_fnum = from_qfp.qf_fnum;
        prevp.qf_type = from_qfp.qf_type;
    }

    true
}

fn copy_loclist(from_qfl: &QfList, to_qfl: &mut QfList) -> bool {
    to_qfl.qfl_type = from_qfl.qfl_type;
    to_qfl.qf_nonevalid = from_qfl.qf_nonevalid;
    to_qfl.qf_has_user_data = from_qfl.qf_has_user_data;
    to_qfl.qf_items.clear();
    to_qfl.qf_index = 0;
    to_qfl.qf_title = from_qfl.qf_title.clone();
    if let Some(ref from_ctx) = from_qfl.qf_ctx {
        let mut to_ctx = tv_initial_value();
        tv_copy(from_ctx, &mut to_ctx);
        to_qfl.qf_ctx = Some(to_ctx);
    } else {
        to_qfl.qf_ctx = None;
    }
    callback_copy(&mut to_qfl.qf_qftf_cb, &from_qfl.qf_qftf_cb);

    if !from_qfl.qf_items.is_empty() && !copy_loclist_entries(from_qfl, to_qfl) {
        return false;
    }

    to_qfl.qf_index = from_qfl.qf_index;
    qf_st().last_qf_id += 1;
    to_qfl.qf_id = qf_st().last_qf_id;
    to_qfl.qf_changedtick = 0;

    if to_qfl.qf_nonevalid {
        to_qfl.qf_index = 1;
    }

    true
}

pub fn copy_loclist_stack(from: &mut WinInfoInfo, to: &mut WinInfoInfo) {
    let qi = get_loc_list(&*from);
    if qi.is_null() {
        return;
    }
    let qi = unsafe { &mut *qi };

    let to_ll = qf_alloc_stack(QFLT_LOCATION, from.w_onebuf_opt.wo_lhi as i32);
    to.w_llist = to_ll;
    to.w_onebuf_opt.wo_lhi = i64::from(to_ll.qf_maxcount);
    to_ll.qf_listcount = qi.qf_listcount;

    for idx in 0..qi.qf_listcount {
        to_ll.qf_curlist = idx;
        if !copy_loclist(qf_get_list(&mut *qi, idx), qf_get_list(&mut *to_ll, idx)) {
            qf_free_all(Some(&mut *to));
            return;
        }
    }

    to_ll.qf_curlist = qi.qf_curlist;
}

fn qf_get_fnum(
    qfl: &mut QfList,
    directory: impl crate::strings::IntoCCharPtr,
    fname: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let mut directory = directory.into_c_char_ptr().cast_mut();
    let fname = fname.into_c_char_ptr().cast_mut();
    unsafe {
        let mut ptr_cstr: Option<std::ffi::CString> = None;
        if crate::memory::cstr_is_empty(fname) {
            return 0;
        }

        let fname_str = cstr_as_string(fname);
        let bufname_ptr = if !directory.is_null() && !vim_isAbsName(fname) {
            let mut joined = concat_fnames(cstr_as_string(directory), fname_str, true);
            if !os_path_exists(joined.as_ptr()) {
                directory = qf_guess_filepath(qfl, fname)
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                joined = if !directory.is_null() {
                    concat_fnames(cstr_as_string(directory), fname_str, true)
                } else {
                    crate::memory::cstr_to_cstring(fname)
                };
            }
            let p = joined.as_ptr().cast_mut();
            ptr_cstr = Some(joined);
            p
        } else {
            fname
        };
        #[cfg(windows)]
        path_to_slash(bufname_ptr);

        let same_as_last = (&raw const qf_st().qf_last_bufname)
            .as_ref()
            .unwrap()
            .as_ref()
            .is_some_and(|last| {
                crate::memory::cstr_as_bytes(bufname_ptr)
                    == crate::memory::cstr_as_bytes(last.as_ptr())
            });
        let buf = if same_as_last && bufref_valid(Some(&qf_st().qf_last_bufref)) {
            drop(ptr_cstr);
            qf_st().qf_last_bufref.br_buf
        } else {
            let b = buflist_new(bufname_ptr, ptr::null_mut(), 0, BLN_NOOPT);
            qf_st().qf_last_bufname =
                ptr_cstr.or_else(|| Some(crate::memory::cstr_to_cstring(bufname_ptr)));
            set_bufref(&mut qf_st().qf_last_bufref, b);
            b
        };
        if buf.is_null() {
            return 0;
        }
        (*buf).b_has_qf_entry = if is_qf_list(qfl) {
            BUF_HAS_QF_ENTRY
        } else {
            BUF_HAS_LL_ENTRY
        };
        (*buf).b_fnum
    }
}

fn qf_push_dir(
    dirbuf: impl crate::strings::IntoCCharPtr,
    stack: &mut Vec<std::ffi::CString>,
    is_file_stack: bool,
) {
    let dirbuf = dirbuf.into_c_char_ptr().cast_mut();
    {
        if vim_isAbsName(dirbuf) || stack.is_empty() || is_file_stack {
            stack.push(crate::memory::cstr_to_cstring(dirbuf));
            return;
        }

        let dirbuf_len = crate::memory::cstr_len(dirbuf);
        let mut found_idx: Option<usize> = None;
        let mut new_dirname: Option<std::ffi::CString> = None;
        for i in (0..stack.len()).rev() {
            let dirname_cstr = concat_fnames(
                cstr_as_string(stack[i].as_ptr()),
                cbuf_as_string(dirbuf, dirbuf_len),
                true,
            );
            if os_isdir(dirname_cstr.as_ptr()) {
                found_idx = Some(i);
                new_dirname = Some(dirname_cstr);
                break;
            }
        }

        let truncate_to = found_idx.map_or(0, |i| i + 1);
        stack.truncate(truncate_to);

        let cstr = new_dirname.unwrap_or_else(|| {
            let slice = crate::memory::cstr_slice(dirbuf, dirbuf_len);
            std::ffi::CString::new(slice).unwrap()
        });
        stack.push(cstr);
    }
}

fn qf_pop_dir(stack: &mut Vec<std::ffi::CString>) {
    stack.pop();
}

fn qf_guess_filepath(
    qfl: &mut QfList,
    filename: impl crate::strings::IntoCCharPtr,
) -> Option<&CStr> {
    let filename = filename.into_c_char_ptr();
    let stack = &mut qfl.qf_dir_stack;
    if stack.is_empty() {
        return None;
    }

    let top_idx = stack.len() - 1;
    let mut found_idx: Option<usize> = None;
    let filename_str = cstr_as_string(filename);
    for i in (0..top_idx).rev() {
        let fullname_cstr = concat_fnames(cstr_as_string(stack[i].as_ptr()), filename_str, true);
        if os_path_exists(fullname_cstr.as_ptr()) {
            found_idx = Some(i);
            break;
        }
    }

    let drain_start = found_idx.map_or(0, |i| i + 1);
    stack.drain(drain_start..top_idx);

    found_idx.map(|_| stack[stack.len() - 2].as_c_str())
}

fn qflist_valid(wp: *mut WinInfoInfo, qf_id: u32) -> bool {
    let mut qi = ql_info_ptr();
    if !wp.is_null() {
        if !win_valid(wp) {
            return false;
        }
        qi = get_loc_list(unsafe { &*wp });
    }
    if qi.is_null() {
        return false;
    }

    for i in 0..unsafe { (*qi).qf_listcount } {
        if (unsafe { &(*qi).qf_lists })[i as usize].qf_id == qf_id {
            return true;
        }
    }
    false
}

fn is_qf_entry_present(qfl: &QfList, qf_index: i32) -> bool {
    !g_st().got_int && qf_index >= 1 && (qf_index as usize) <= qfl.qf_items.len()
}

fn get_next_valid_entry(qfl: &mut QfList, qf_index: &mut i32, dir: i32) -> *mut Qfline {
    let mut idx = *qf_index;
    if idx < 1 || (idx as usize) > qfl.qf_items.len() {
        return ptr::null_mut();
    }
    let old_qf_fnum = (&qfl.qf_items)[(idx - 1) as usize].qf_fnum;

    loop {
        if (idx as usize) >= qfl.qf_items.len() {
            return ptr::null_mut();
        }
        idx += 1;
        let qfp = &(&qfl.qf_items)[(idx - 1) as usize];
        if !((!qfl.qf_nonevalid && qfp.qf_valid == 0)
            || (dir == FORWARD_FILE && qfp.qf_fnum == old_qf_fnum))
        {
            break;
        }
    }

    *qf_index = idx;
    &raw mut (&mut qfl.qf_items)[(idx - 1) as usize]
}

fn get_prev_valid_entry(qfl: &mut QfList, qf_index: &mut i32, dir: i32) -> *mut Qfline {
    let mut idx = *qf_index;
    if idx < 1 || (idx as usize) > qfl.qf_items.len() {
        return ptr::null_mut();
    }
    let old_qf_fnum = (&qfl.qf_items)[(idx - 1) as usize].qf_fnum;

    loop {
        if idx <= 1 {
            return ptr::null_mut();
        }
        idx -= 1;
        let qfp = &(&qfl.qf_items)[(idx - 1) as usize];
        if !((!qfl.qf_nonevalid && qfp.qf_valid == 0)
            || (dir == BACKWARD_FILE && qfp.qf_fnum == old_qf_fnum))
        {
            break;
        }
    }

    *qf_index = idx;
    &raw mut (&mut qfl.qf_items)[(idx - 1) as usize]
}

fn get_nth_valid_entry(
    qfl: &mut QfList,
    mut errornr: i32,
    dir: i32,
    new_qfidx: &mut i32,
) -> *mut Qfline {
    let mut qf_ptr = qfl.cur_entry();
    let mut qf_idx = qfl.qf_index;
    let mut err = E_NO_MORE_ITEMS.as_ptr();

    while errornr > 0 {
        errornr -= 1;
        let prev_qf_ptr = qf_ptr;
        let prev_index = qf_idx;

        if dir == FORWARD || dir == FORWARD_FILE {
            qf_ptr = get_next_valid_entry(qfl, &mut qf_idx, dir);
        } else {
            qf_ptr = get_prev_valid_entry(qfl, &mut qf_idx, dir);
        }

        if qf_ptr.is_null() {
            qf_ptr = prev_qf_ptr;
            qf_idx = prev_index;
            if !err.is_null() {
                emsg(gettext(err));
                return ptr::null_mut();
            }
            break;
        }

        err = ptr::null();
    }

    *new_qfidx = qf_idx;
    qf_ptr
}

fn get_nth_entry(qfl: &mut QfList, errornr: i32, new_qfidx: &mut i32) -> *mut Qfline {
    if qfl.qf_items.is_empty() {
        *new_qfidx = qfl.qf_index;
        return ptr::null_mut();
    }
    let qf_idx = errornr.clamp(1, qfl.count());
    *new_qfidx = qf_idx;
    &raw mut (&mut qfl.qf_items)[(qf_idx - 1) as usize]
}

fn qf_get_entry(qfl: &mut QfList, errornr: i32, dir: i32, new_qfidx: &mut i32) -> *mut Qfline {
    let mut qf_ptr = qfl.cur_entry();
    let mut qfidx = qfl.qf_index;

    if dir != 0 {
        qf_ptr = get_nth_valid_entry(qfl, errornr, dir, &mut qfidx);
    } else if errornr != 0 {
        qf_ptr = get_nth_entry(qfl, errornr, &mut qfidx);
    }

    *new_qfidx = qfidx;
    qf_ptr
}

fn qf_find_help_win() -> *mut WinInfoInfo {
    for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if bt_help(unsafe { (wp.w_buffer).as_ref() }) && !wp.w_config.hide && wp.w_config.focusable
        {
            return wp;
        }
    }
    ptr::null_mut()
}

fn win_set_loclist(wp: &mut WinInfoInfo, qi: &mut QfInfo) {
    wp.w_llist = &raw mut *qi;
    qi.qf_refcount += 1;
}

fn jump_to_help_window(qi: &mut QfInfo, newwin: bool, opened_window: &mut bool) -> bool {
    unsafe {
        let wp = if g_st().cmdmod.cmod_tab != 0 || newwin {
            ptr::null_mut()
        } else {
            qf_find_help_win()
        };

        if !wp.is_null() && (*wp).buf_mut().b_nwindows > 0 {
            win_enter(wp, true);
        } else {
            let mut flags = WSP_HELP;
            if g_st().cmdmod.cmod_split == 0
                && curwin_mut().w_width != g_st().columns_global
                && curwin_mut().w_width < 80
            {
                flags |= WSP_TOP;
            }

            if is_ll_stack(qi) && !newwin {
                flags |= WSP_NEWLOC;
            }

            if !win_split(0, flags) {
                return false;
            }

            *opened_window = true;

            if i64::from(curwin_mut().w_height) < opt_vars().p_hh {
                win_setheight(opt_vars().p_hh as i32);
            }

            if is_ll_stack(qi) && !newwin {
                win_set_loclist(curwin_mut(), qi);
            }
        }

        g_st().restart_edit = 0;

        true
    }
}

fn qf_find_win_with_loclist(ll: &QfInfo) -> *mut WinInfoInfo {
    for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if wp.w_llist.cast_const() == ll && !bt_quickfix(unsafe { (wp.w_buffer).as_ref() }) {
            return wp;
        }
    }
    ptr::null_mut()
}

fn qf_find_win_with_normal_buf() -> *mut WinInfoInfo {
    for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if bt_normal(unsafe { (wp.w_buffer).as_ref() }) {
            return wp;
        }
    }
    ptr::null_mut()
}

fn qf_goto_tabwin_with_file(fnum: i32) -> bool {
    for tp in for_all_tabpages_mut() {
        for wp in win_iter_ptr(tab_firstwin(tp)) {
            if (*wp).buf_mut().b_fnum == fnum {
                goto_tabpage_win(tp, wp);
                return true;
            }
        }
    }
    false
}

fn qf_open_new_file_win(ll_ref: Option<&mut QfInfo>) -> bool {
    let mut flags = WSP_ABOVE;
    if ll_ref.is_some() {
        flags |= WSP_NEWLOC;
    }
    if !win_split(0, flags) {
        return false;
    }
    opt_vars().p_swb = EMPTY_STRING_OPTION.as_ptr().cast();
    opt_vars().swb_flags = 0;
    reset_binding(curwin_mut());
    if let Some(ll_ref) = ll_ref {
        win_set_loclist(curwin_mut(), ll_ref);
    }
    true
}

fn qf_goto_win_with_ll_file(use_win: *mut WinInfoInfo, qf_fnum: i32, ll_ref: Option<&mut QfInfo>) {
    unsafe {
        let mut win = use_win;

        if win.is_null() {
            for win2 in win_iter_ptr(tab_firstwin(curtab_mut())) {
                if (*win2).buf_mut().b_fnum == qf_fnum {
                    win = win2;
                    break;
                }
            }
            if win.is_null() {
                win = curwin_ptr();
                loop {
                    if bt_normal(((*win).w_buffer).as_ref()) {
                        break;
                    }
                    if (*win).w_prev.is_null() {
                        win = g_st().lastwin;
                    } else {
                        win = (*win).w_prev;
                    }
                    if win == curwin_ptr() {
                        break;
                    }
                }
            }
        }
        win_goto(win);

        if (*win).w_llist.is_null()
            && let Some(ll_ref) = ll_ref
        {
            win_set_loclist(&mut *win, ll_ref);
        }
    }
}

fn qf_goto_win_with_qfl_file(qf_fnum: i32) {
    unsafe {
        let mut win = curwin_ptr();
        let mut altwin: *mut WinInfoInfo = ptr::null_mut();
        loop {
            if (*win).buf_mut().b_fnum == qf_fnum {
                break;
            }
            if (*win).w_prev.is_null() {
                win = g_st().lastwin;
            } else {
                win = (*win).w_prev;
            }

            if is_qf_window(&*win) {
                if (opt_vars().swb_flags & K_OPT_SWB_FLAG_USELAST) != 0
                    && win_valid(g_st().prevwin)
                    && crate::globals::prevwin_mut().w_onebuf_opt.wo_wfb == 0
                {
                    win = g_st().prevwin;
                } else if !altwin.is_null() {
                    win = altwin;
                } else if !curwin_mut().w_prev.is_null() {
                    win = curwin_mut().w_prev;
                } else {
                    win = curwin_mut().w_next;
                }
                break;
            }

            if altwin.is_null()
                && (*win).w_onebuf_opt.wo_pvw == 0
                && (*win).w_onebuf_opt.wo_wfb == 0
                && bt_normal(((*win).w_buffer).as_ref())
            {
                altwin = win;
            }
        }

        win_goto(win);
    }
}

fn qf_jump_to_usable_window(qf_fnum: i32, newwin: bool, opened_window: &mut bool) -> bool {
    let mut usable_wp: *mut WinInfoInfo = ptr::null_mut();
    let mut usable_win = false;

    let ll_ref = if newwin {
        ptr::null_mut()
    } else {
        curwin_mut().w_llist_ref
    };
    if !ll_ref.is_null() {
        usable_wp = qf_find_win_with_loclist(unsafe { &*ll_ref });
        if !usable_wp.is_null() {
            usable_win = true;
        }
    }

    if !usable_win {
        let win = qf_find_win_with_normal_buf();
        if !win.is_null() {
            usable_win = true;
        }
    }

    if !usable_win && (opt_vars().swb_flags & K_OPT_SWB_FLAG_USETAB) != 0 {
        usable_win = qf_goto_tabwin_with_file(qf_fnum);
    }

    if ((g_st().firstwin == g_st().lastwin) && bt_quickfix(Some(crate::globals::curbuf_mut())))
        || !usable_win
        || newwin
    {
        if !qf_open_new_file_win(unsafe { ll_ref.as_mut() }) {
            return false;
        }
        *opened_window = true;
    } else if !curwin_mut().w_llist_ref.is_null() {
        qf_goto_win_with_ll_file(usable_wp, qf_fnum, unsafe { ll_ref.as_mut() });
    } else {
        qf_goto_win_with_qfl_file(qf_fnum);
    }

    true
}

fn qf_jump_edit_buffer(
    qi: *mut QfInfo,
    qf_index: i32,
    forceit: i32,
    prev_winid: i32,
    opened_window: &mut bool,
) -> i32 {
    let qfl: *mut QfList = qf_get_curlist(unsafe { &mut *qi });
    let old_changetick = unsafe { (*qfl).qf_changedtick };
    let old_qf_curlist = unsafe { (*qi).qf_curlist };
    let qfl_type = unsafe { (*qfl).qfl_type };
    let mut retval = OK;
    let save_qfid = unsafe { (*qfl).qf_id };
    let (qf_type, fnum) = {
        let qfp = &(unsafe { &(*qfl).qf_items })[(qf_index - 1) as usize];
        (qfp.qf_type, qfp.qf_fnum)
    };

    if qf_type == 1 {
        if !can_abandon(curbuf_mut(), forceit != 0) {
            no_write_message();
            return FAIL;
        }
        retval = if do_ecmd(
            fnum,
            ptr::null_mut(),
            ptr::null_mut(),
            None,
            1,
            ECMD_HIDE + ECMD_SET_HELP,
            if prev_winid == curwin_mut().handle {
                curwin_ptr()
            } else {
                ptr::null_mut()
            },
        ) {
            OK
        } else {
            FAIL
        };
    } else {
        if forceit == 0 && curwin_mut().w_onebuf_opt.wo_wfb != 0 && curbuf_mut().b_fnum != fnum {
            if unsafe { (*qi).qfl_type } == QFLT_LOCATION {
                emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
                return FAIL;
            }

            if win_valid(g_st().prevwin)
                && crate::globals::prevwin_mut().w_onebuf_opt.wo_wfb == 0
                && !bt_quickfix(unsafe { (crate::globals::prevwin_mut().w_buffer).as_ref() })
            {
                win_goto(g_st().prevwin);
            }
            if curwin_mut().w_onebuf_opt.wo_wfb != 0 {
                if win_split(0, 0) {
                    *opened_window = true;
                }
                if curwin_mut().w_onebuf_opt.wo_wfb != 0 {
                    emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
                    retval = FAIL;
                }
            }
        }

        if retval == OK {
            retval = if buflist_getfile(fnum, 1, GETF_SETMARK | GETF_SWITCH, forceit) {
                OK
            } else {
                FAIL
            };
        }
    }

    if qfl_type == QFLT_LOCATION {
        let wp = win_id2wp(prev_winid);
        if wp.is_null() && curwin_mut().w_llist != qi {
            emsg(gettext(c"E924: Current window was closed"));
            *opened_window = false;
            return QF_ABORT;
        }
    }

    if qfl_type == QFLT_QUICKFIX && !qflist_valid(ptr::null_mut(), save_qfid) {
        emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        return QF_ABORT;
    }

    if old_qf_curlist != unsafe { (*qi).qf_curlist }
        || old_changetick != unsafe { (*qfl).qf_changedtick }
        || !is_qf_entry_present(unsafe { &*qfl }, qf_index)
    {
        if qfl_type == QFLT_QUICKFIX {
            emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        } else {
            emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
        }
        return QF_ABORT;
    }

    retval
}

fn qf_screen_col_to_idx(vcol: Colnr) -> i32 {
    let line = get_cursor_line_ptr().cast_const();
    let mut p = line;
    let mut col: Colnr = 0;

    while !crate::memory::cstr_is_empty(p) && col < vcol {
        if crate::memory::cstr_first_byte(p) == TAB {
            col += 8 - (col % 8);
        } else {
            col += ptr2cells(p);
        }
        if col > vcol {
            break;
        }
        p = p.wrapping_add(utfc_ptr2len(p) as usize);
    }
    crate::memory::ptr_offset_from(p, line) as i32
}

fn qf_jump_goto_line(
    qf_lnum: Linenr,
    qf_col: i32,
    qf_viscol: u8,
    qf_pattern: impl crate::strings::IntoCCharPtr,
) {
    let qf_pattern = qf_pattern.into_c_char_ptr().cast_mut();
    {
        if qf_pattern.is_null() {
            let mut i = qf_lnum;
            if i > 0 {
                i = i.min(curbuf_mut().b_ml.ml_line_count);
                curwin_mut().w_cursor.lnum = i;
            }
            if qf_col > 0 {
                curwin_mut().w_cursor.coladd = 0;
                if qf_viscol != 0 {
                    curwin_mut().w_cursor.col = qf_screen_col_to_idx(qf_col - 1);
                } else {
                    curwin_mut().w_cursor.col = qf_col - 1;
                }
                curwin_mut().w_set_curswant = true;
                check_cursor(curwin_mut());
            } else {
                beginline(BL_WHITE | BL_FIX);
            }
        } else {
            let save_cursor = curwin_mut().w_cursor;
            curwin_mut().w_cursor.lnum = 0;
            if do_search(
                None,
                b'/' as i32,
                b'/' as i32,
                qf_pattern,
                crate::memory::cstr_len(qf_pattern),
                1,
                SEARCH_KEEP,
                opt_vars().p_magic != 0,
                None,
            ) == 0
            {
                curwin_mut().w_cursor = save_cursor;
            }
        }
    }
}

fn qf_jump_print_msg(
    qi: &mut QfInfo,
    qf_index: i32,
    qf_ptr: &mut Qfline,
    old_curbuf: *mut FileBuf,
    old_lnum: Linenr,
) {
    let gap = qfga_get();

    if g_st().msg_scrolled == 0 {
        update_topline(curwin_mut());
        if g_st().must_redraw != 0 {
            update_screen();
        }
    }
    let iobuff_ptr = g_st().iobuff.as_mut_ptr();
    let deleted_str = if qf_ptr.qf_cleared != 0 {
        gettext(c" (line deleted)").cast_const()
    } else {
        c"".as_ptr()
    };
    let iobufflen = vim_snprintf_safelen!(
        iobuff_ptr,
        IOSIZE,
        gettext(c"(%d of %d)%s%s: "),
        qf_index,
        qf_get_curlist(qi).count(),
        deleted_str,
        qf_types(i32::from(qf_ptr.qf_type), qf_ptr.qf_nr).as_c_str(),
    );
    qfga_concat_len(gap, iobuff_ptr, iobufflen);
    qf_fmt_text(gap, skipwhite(qf_ptr.qf_text.as_ptr()));
    gap.push(0);

    let i = g_st().msg_scroll;
    if std::ptr::eq(g_st().curbuf, old_curbuf) && curwin_mut().w_cursor.lnum == old_lnum {
        g_st().msg_scroll = 1;
    } else if (g_st().msg_scrolled == 0 || (opt_vars().p_ch == 0 && g_st().msg_scrolled == 1))
        && shortmess(K_SHM_OVERALL)
    {
        g_st().msg_scroll = 0;
    }
    msg_ext_set_kind(c"quickfix");
    msg_keep(gap.as_ptr().cast(), 0, true, false);
    g_st().msg_scroll = i;

    qfga_clear();
}

fn qf_jump_open_window(
    qi: *mut QfInfo,
    qf_index: i32,
    newwin: bool,
    opened_window: &mut bool,
) -> i32 {
    let qfl: *mut QfList = qf_get_curlist(unsafe { &mut *qi });
    let old_changetick = unsafe { (*qfl).qf_changedtick };
    let old_qf_curlist = unsafe { (*qi).qf_curlist };
    let qfl_type = unsafe { (*qfl).qfl_type };
    let qf_type = (unsafe { &(*qfl).qf_items })[(qf_index - 1) as usize].qf_type;

    if qf_type == 1
        && (!bt_help(unsafe { curwin_mut().w_buffer.as_ref() }) || g_st().cmdmod.cmod_tab != 0)
        && !jump_to_help_window(unsafe { &mut *qi }, newwin, opened_window)
    {
        return FAIL;
    }
    if old_qf_curlist != unsafe { (*qi).qf_curlist }
        || old_changetick != unsafe { (*qfl).qf_changedtick }
        || !is_qf_entry_present(unsafe { &*qfl }, qf_index)
    {
        if qfl_type == QFLT_QUICKFIX {
            emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        } else {
            emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
        }
        return QF_ABORT;
    }

    let qf_fnum = (unsafe { &(*qfl).qf_items })[(qf_index - 1) as usize].qf_fnum;
    if bt_quickfix(Some(crate::globals::curbuf_mut())) && !*opened_window {
        if qf_fnum == 0 {
            return NOTDONE;
        }

        if !qf_jump_to_usable_window(qf_fnum, newwin, opened_window) {
            return FAIL;
        }
    }
    if old_qf_curlist != unsafe { (*qi).qf_curlist }
        || old_changetick != unsafe { (*qfl).qf_changedtick }
        || !is_qf_entry_present(unsafe { &*qfl }, qf_index)
    {
        if qfl_type == QFLT_QUICKFIX {
            emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        } else {
            emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
        }
        return QF_ABORT;
    }

    OK
}

fn qf_jump_to_buffer(
    qi: *mut QfInfo,
    qf_index: i32,
    forceit: i32,
    prev_winid: i32,
    opened_window: &mut bool,
    openfold: i32,
    print_message: bool,
) -> i32 {
    let old_curbuf = g_st().curbuf;
    let old_lnum = curwin_mut().w_cursor.lnum;
    let retval;
    let qfl: *mut QfList = qf_get_curlist(unsafe { &mut *qi });

    if (unsafe { &(*qfl).qf_items })[(qf_index - 1) as usize].qf_fnum != 0 {
        retval = qf_jump_edit_buffer(qi, qf_index, forceit, prev_winid, opened_window);
        if retval != OK {
            return retval;
        }
    } else {
        retval = OK;
    }

    if g_st().curbuf == old_curbuf {
        setpcmark();
    }

    let qf_ptr = &raw mut (unsafe { &mut (*qfl).qf_items })[(qf_index - 1) as usize];
    unsafe {
        qf_jump_goto_line(
            (*qf_ptr).qf_lnum,
            (*qf_ptr).qf_col,
            (*qf_ptr).qf_viscol,
            (*qf_ptr)
                .qf_pattern
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
        );
    }

    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_QUICKFIX) != 0 && openfold != 0 {
        fold_open_cursor();
    }
    if print_message {
        qf_jump_print_msg(
            unsafe { &mut *qi },
            qf_index,
            unsafe { &mut *qf_ptr },
            old_curbuf,
            old_lnum,
        );
    }

    retval
}

pub fn qf_jump(qi: *mut QfInfo, dir: i32, errornr: i32, forceit: i32) {
    qf_jump_newwin(qi, dir, errornr, forceit, false);
}

fn qf_jump_newwin(mut qi: *mut QfInfo, dir: i32, errornr: i32, forceit: i32, newwin: bool) {
    let old_swb = opt_vars().p_swb;
    let old_swb_flags = opt_vars().swb_flags;
    let old_key_typed = g_st().key_typed;

    if qi.is_null() {
        qi = ql_info_ptr();
    }

    if qf_stack_empty(unsafe { qi.as_ref() })
        || qf_list_empty(Some(&*(qf_get_curlist(unsafe { &mut *qi }))))
    {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    incr_quickfix_busy();

    let qfl: *mut QfList = qf_get_curlist(unsafe { &mut *qi });

    let mut qf_index = unsafe { (*qfl).qf_index };
    let old_qf_index = qf_index;

    let qf_ptr = qf_get_entry(unsafe { &mut *qfl }, errornr, dir, &mut qf_index);
    if !qf_ptr.is_null() {
        unsafe {
            (*qfl).qf_index = qf_index;
        }

        let print_message = !qf_win_pos_update(unsafe { &mut *qi }, old_qf_index);
        let prev_winid = curwin_mut().handle;
        let mut opened_window = false;
        let mut retval = qf_jump_open_window(qi, qf_index, newwin, &mut opened_window);
        if retval == FAIL {
            qf_index = old_qf_index;
        } else if retval == QF_ABORT {
            qi = ptr::null_mut();
        } else if retval != NOTDONE {
            retval = qf_jump_to_buffer(
                qi,
                qf_index,
                forceit,
                prev_winid,
                &mut opened_window,
                i32::from(old_key_typed),
                print_message,
            );
            if retval == QF_ABORT {
                qi = ptr::null_mut();
            } else if retval != OK {
                if opened_window {
                    win_close(curwin_ptr(), true, false);
                }
                if (unsafe { &(*qfl).qf_items })[(qf_index - 1) as usize].qf_fnum != 0 {
                    qf_index = old_qf_index;
                }
            }
        }
    } else {
        qf_index = old_qf_index;
    }

    if !qi.is_null() {
        unsafe {
            (*qfl).qf_index = qf_index;
        }
    }
    if opt_vars().p_swb != old_swb
        && opt_vars().p_swb == EMPTY_STRING_OPTION.as_ptr().cast()
    {
        opt_vars().p_swb = old_swb;
        opt_vars().swb_flags = old_swb_flags;
    }
    decr_quickfix_busy();
}

fn qf_list_entry(qfp: &mut Qfline, qf_idx: i32, cursel: bool) {
    let mut fname = ptr::null_mut();
    let iobuff_ptr = g_st().iobuff.as_mut_ptr();
    if let Some(ref module) = qfp.qf_module
        && !module.as_bytes().is_empty()
    {
        vim_snprintf!(iobuff_ptr, IOSIZE, c"%2d %s", qf_idx, module.as_ptr(),);
    } else {
        if qfp.qf_fnum != 0 {
            let buf = buflist_findnr(qfp.qf_fnum);
            if !buf.is_null() {
                fname = qfp
                    .qf_fname
                    .as_ref()
                    .map_or(unsafe { (*buf).b_fname }, |s| s.as_ptr().cast_mut());
                if qfp.qf_type == 1 {
                    fname = path_tail(fname);
                }
            }
        }
        if fname.is_null() {
            vim_snprintf!(iobuff_ptr, IOSIZE, c"%2d", qf_idx);
        } else {
            vim_snprintf!(iobuff_ptr, IOSIZE, c"%2d %s", qf_idx, fname);
        }
    }

    let mut filter_entry = true;
    if let Some(ref module) = qfp.qf_module
        && !module.as_bytes().is_empty()
    {
        filter_entry &= message_filtered(module.as_ptr());
    }
    if filter_entry && !fname.is_null() {
        filter_entry &= message_filtered(fname);
    }
    if filter_entry && let Some(ref pat) = qfp.qf_pattern {
        filter_entry &= message_filtered(pat.as_ptr());
    }
    if filter_entry {
        filter_entry &= message_filtered(qfp.qf_text.as_ptr());
    }
    if filter_entry {
        return;
    }

    if g_st().msg_col > 0 {
        msg_putchar(b'\n' as i32);
    }
    msg_outtrans(
        iobuff_ptr,
        if cursel {
            HLF_QFL
        } else {
            qf_st().qf_file_hl_id
        },
        false,
    );

    if qfp.qf_lnum != 0 {
        msg_puts_hl(c":", qf_st().qf_sep_hl_id, false);
    }
    let mut gap = qfga_get();
    if qfp.qf_lnum != 0 {
        qf_range_text(gap, qfp);
    }
    qfga_concat(gap, &qf_types(i32::from(qfp.qf_type), qfp.qf_nr));
    gap.push(0);
    if !crate::memory::cstr_is_empty(gap.as_ptr().cast()) {
        msg_puts_hl(gap.as_ptr().cast(), qf_st().qf_line_hl_id, false);
    }
    msg_puts_hl(c":", qf_st().qf_sep_hl_id, false);
    if let Some(ref pat) = qfp.qf_pattern {
        gap = qfga_get();
        qf_fmt_text(gap, pat.as_ptr());
        gap.push(0);
        msg_puts(gap.as_ptr().cast());
        msg_puts_hl(c":", qf_st().qf_sep_hl_id, false);
    }
    msg_puts(c" ");

    gap = qfga_get();
    let text = if !fname.is_null() || qfp.qf_lnum != 0 {
        skipwhite(qfp.qf_text.as_ptr())
    } else {
        qfp.qf_text.as_ptr()
    };
    qf_fmt_text(gap, text);
    gap.push(0);
    msg_prt_line(gap.as_mut_ptr().cast(), false);
}

pub fn qf_list(eap: &mut Exarg) {
    let mut arg = eap.arg;
    let mut all = eap.forceit != 0;
    let qi = qf_cmd_get_stack(eap, true);
    if qi.is_null() {
        return;
    }

    if qf_stack_empty(unsafe { qi.as_ref() })
        || qf_list_empty(Some(&*(qf_get_curlist(unsafe { &mut *qi }))))
    {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    let mut plus = false;
    if crate::memory::cstr_first_byte(arg) == b'+' {
        arg = arg.wrapping_add(1);
        plus = true;
    }
    let mut idx1: i32 = 1;
    let mut idx2: i32 = -1;
    if !get_list_range(&raw mut arg, &mut idx1, &mut idx2) || !crate::memory::cstr_is_empty(arg) {
        semsg!(gettext(E_TRAILING_ARG), arg);
        return;
    }
    let qfl = qf_get_curlist(unsafe { &mut *qi });
    if plus {
        let i = qfl.qf_index;
        idx2 = i + idx1;
        idx1 = i;
    } else {
        let i = qfl.count();
        if idx1 < 0 {
            idx1 = if -idx1 > i { 0 } else { idx1 + i + 1 };
        }
        if idx2 < 0 {
            idx2 = if -idx2 > i { 0 } else { idx2 + i + 1 };
        }
    }

    shorten_fnames(0);

    qf_st().qf_file_hl_id = syn_name2id(c"qfFileName");
    if qf_st().qf_file_hl_id == 0 {
        qf_st().qf_file_hl_id = HLF_D;
    }
    qf_st().qf_sep_hl_id = syn_name2id(c"qfSeparator");
    if qf_st().qf_sep_hl_id == 0 {
        qf_st().qf_sep_hl_id = HLF_D;
    }
    qf_st().qf_line_hl_id = syn_name2id(c"qfLineNr");
    if qf_st().qf_line_hl_id == 0 {
        qf_st().qf_line_hl_id = HLF_N;
    }

    if qfl.qf_nonevalid {
        all = true;
    }
    msg_ext_set_kind(c"list_cmd");
    for (idx, qfp) in qfl.qf_items.iter_mut().enumerate() {
        if g_st().got_int {
            break;
        }
        let i = (idx + 1) as i32;
        if (qfp.qf_valid != 0 || all) && idx1 <= i && i <= idx2 {
            qf_list_entry(qfp, i, i == qfl.qf_index);
        }
        os_breakcheck();
    }
    qfga_clear();
}

fn qf_fmt_text(gap: &mut Vec<u8>, text: impl crate::strings::IntoCCharPtr) {
    let text: *const u8 = text.into_c_char_ptr().cast();
    let mut p = text;
    while !crate::memory::cstr_is_empty(p.cast()) {
        if crate::memory::cstr_first_byte(p.cast()) == b'\n' {
            gap.push(b' ');
            loop {
                p = p.wrapping_add(1);
                if crate::memory::cstr_is_empty(p.cast()) {
                    break;
                }
                if !ascii_iswhite(i32::from(crate::memory::cstr_first_byte(p.cast())))
                    && crate::memory::cstr_first_byte(p.cast()) != b'\n'
                {
                    break;
                }
            }
        } else {
            gap.push(unsafe { *p });
            p = p.wrapping_add(1);
        }
    }
}

fn qf_range_text(gap: &mut Vec<u8>, qfp: &Qfline) {
    let buf_data = g_st().iobuff.as_mut_ptr();
    let mut buf_size = vim_snprintf_safelen!(buf_data, IOSIZE, c"%d", qfp.qf_lnum);

    if qfp.qf_end_lnum > 0 && qfp.qf_lnum != qfp.qf_end_lnum {
        buf_size += vim_snprintf_safelen!(
            buf_data.wrapping_add(buf_size),
            IOSIZE - buf_size,
            c"-%d",
            qfp.qf_end_lnum,
        );
    }
    if qfp.qf_col > 0 {
        buf_size += vim_snprintf_safelen!(
            buf_data.wrapping_add(buf_size),
            IOSIZE - buf_size,
            c" col %d",
            qfp.qf_col,
        );
        if qfp.qf_end_col > 0 && qfp.qf_col != qfp.qf_end_col {
            buf_size += vim_snprintf_safelen!(
                buf_data.wrapping_add(buf_size),
                IOSIZE - buf_size,
                c"-%d",
                qfp.qf_end_col,
            );
        }
    }

    qfga_concat_len(gap, buf_data, buf_size);
}

fn qf_msg(qi: &mut QfInfo, which: i32, lead: impl crate::strings::IntoCCharPtr) {
    let lead = lead.into_c_char_ptr();
    {
        let qfl = &(&qi.qf_lists)[which as usize];
        let title = qfl.qf_title.as_ref();
        let count = qfl.count();
        let mut buf = [0u8; IOSIZE];

        vim_snprintf!(
            buf.as_mut_ptr().cast(),
            IOSIZE,
            gettext(c"%serror list %d of %d; %d errors "),
            lead,
            which + 1,
            qi.qf_listcount,
            count,
        );

        if let Some(title) = title {
            let len = crate::memory::cstr_len(buf.as_ptr().cast());
            if len < 34 {
                unsafe { ptr::write_bytes(buf.as_mut_ptr().wrapping_add(len), b' ', 34 - len) };
                buf[34] = 0;
            }
            {
                let _dst: *mut _ = buf.as_mut_ptr().cast();
                let _dsize: usize = IOSIZE;
                let _dlen = crate::memory::cstr_len(_dst);
                crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), title.as_ptr(), _dsize - _dlen);
            };
        }
        trunc_string(
            buf.as_ptr().cast(),
            buf.as_mut_ptr().cast(),
            g_st().columns_global - 1,
            IOSIZE as i32,
        );
        msg(buf.as_ptr().cast(), 0);
    }
}

pub fn qf_age(eap: &mut Exarg) {
    let qi = qf_cmd_get_stack(eap, true);
    if qi.is_null() {
        return;
    }
    let qi = unsafe { &mut *qi };

    let mut count = if eap.addr_count != 0 { eap.line2 } else { 1 };
    while count > 0 {
        count -= 1;
        if eap.cmdidx == CMD_COLDER || eap.cmdidx == CMD_LOLDER {
            if qi.qf_curlist == 0 {
                emsg(gettext(c"E380: At bottom of quickfix stack"));
                break;
            }
            qi.qf_curlist -= 1;
        } else {
            if qi.qf_curlist >= qi.qf_listcount - 1 {
                emsg(gettext(c"E381: At top of quickfix stack"));
                break;
            }
            qi.qf_curlist += 1;
        }
    }
    qf_msg(qi, qi.qf_curlist, c"");
    qf_update_buffer(&mut *qi, 0);
}

pub fn qf_history(eap: &mut Exarg) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);

        if eap.addr_count > 0 {
            if qi.is_null() {
                emsg(gettext(E_LOCLIST));
                return;
            }

            if eap.line2 > 0 && eap.line2 <= (*qi).qf_listcount {
                (*qi).qf_curlist = eap.line2 - 1;
                qf_msg(&mut *qi, (*qi).qf_curlist, c"");
                qf_update_buffer(&mut *qi, 0);
            } else {
                emsg(gettext(E_INVRANGE));
            }
            return;
        }

        if qf_stack_empty(qi.as_ref()) {
            msg(gettext(c"No entries"), 0);
        } else {
            for i in 0..(*qi).qf_listcount {
                let lead = if i == (*qi).qf_curlist {
                    c"> ".as_ptr()
                } else {
                    c"  ".as_ptr()
                };
                qf_msg(&mut *qi, i, lead);
            }
        }
    }
}

fn qf_free_items(qfl: &mut QfList) {
    qfl.qf_items.clear();
    qfl.qf_index = 0;
    qfl.qf_nonevalid = true;

    qfl.qf_dir_stack.clear();
    qfl.qf_file_stack.clear();
    qfl.qf_multiline = false;
    qfl.qf_multiignore = false;
    qfl.qf_multiscan = false;
}

fn qf_free(qfl: &mut QfList) {
    qf_free_items(qfl);

    qfl.qf_title = None;
    if let Some(ref mut ctx) = qfl.qf_ctx {
        tv_clear(ctx);
    }
    qfl.qf_ctx = None;
    callback_free(&mut qfl.qf_qftf_cb);
    qfl.qf_id = 0;
    qfl.qf_changedtick = 0;
}

pub fn qf_mark_adjust(
    buf: &mut FileBuf,
    wp: Option<&WinInfoInfo>,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) -> bool {
    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());
    let buf_has_flag = if wp.is_none() {
        BUF_HAS_QF_ENTRY
    } else {
        BUF_HAS_LL_ENTRY
    };

    if (buf.b_has_qf_entry & buf_has_flag) == 0 {
        return false;
    }
    if let Some(wp) = wp {
        if wp.w_llist.is_null() {
            return false;
        }
        qi = wp.w_llist;
    }

    let mut found_one = false;
    for idx in 0..unsafe { (*qi).qf_listcount } {
        let qfl = qf_get_list(unsafe { &mut *qi }, idx);
        for qfp in &mut qfl.qf_items {
            if g_st().got_int {
                break;
            }
            if qfp.qf_fnum == buf.b_fnum {
                found_one = true;
                if qfp.qf_cleared == 0 {
                    if qfp.qf_lnum >= line1 && qfp.qf_lnum <= line2 {
                        if amount == MAXLNUM {
                            qfp.qf_cleared = 1;
                        } else {
                            qfp.qf_lnum += amount;
                        }
                    } else if amount_after != 0 && qfp.qf_lnum > line2 {
                        qfp.qf_lnum += amount_after;
                    }
                }
            }
        }
    }

    found_one
}

fn qf_types(c: i32, nr: i32) -> std::ffi::CString {
    let cc: [u8; 3];
    let p: &CStr = if c == b'W' as i32 || c == b'w' as i32 {
        c" warning"
    } else if c == b'I' as i32 || c == b'i' as i32 {
        c" info"
    } else if c == b'N' as i32 || c == b'n' as i32 {
        c" note"
    } else if c == b'E' as i32 || c == b'e' as i32 || (c == 0 && nr > 0) {
        c" error"
    } else if c == 0 || c == 1 {
        c""
    } else {
        cc = [b' ', c as u8, 0];
        CStr::from_bytes_until_nul(&cc).unwrap_or(c"")
    };

    if nr <= 0 {
        return p.to_owned();
    }

    let mut buf = [0u8; 20];
    let len = vim_snprintf_safelen!(buf.as_mut_ptr().cast(), 20, c"%s %3d", p.as_ptr(), nr);
    crate::memory::cstr_from_bytes(&buf[..len])
}

pub fn qf_view_result(split: bool) {
    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());

    if is_ll_window(curwin_mut()) {
        qi = get_loc_list(curwin_mut());
    }

    if qf_list_empty(Some(&*(qf_get_curlist(unsafe { &mut *qi })))) {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    if split {
        qf_jump_newwin(qi, 0, curwin_mut().w_cursor.lnum, 0, true);
        do_cmdline_cmd(c"clearjumps");
        return;
    }

    do_cmdline_cmd(if is_ll_window(curwin_mut()) {
        c".ll".as_ptr()
    } else {
        c".cc".as_ptr()
    });
}

pub fn ex_cwindow(eap: &mut Exarg) {
    let qi = qf_cmd_get_stack(eap, true);
    if qi.is_null() {
        return;
    }

    let qfl = qf_get_curlist(unsafe { &mut *qi });
    let win = qf_find_win(unsafe { &*qi });

    if qf_stack_empty(unsafe { qi.as_ref() }) || qfl.qf_nonevalid || qf_list_empty(Some(&*qfl)) {
        if !win.is_null() {
            ex_cclose(eap);
        }
    } else if win.is_null() {
        ex_copen(eap);
    }
}

pub fn ex_cclose(eap: &mut Exarg) {
    let qi = qf_cmd_get_stack(eap, false);
    if qi.is_null() {
        return;
    }

    let win = unsafe { qf_find_win(&*qi) };
    if !win.is_null() {
        win_close(win, false, false);
    }
}

fn qf_goto_cwindow(qi: &QfInfo, resize: bool, sz: i32, vertsplit: bool) -> bool {
    let win = qf_find_win(qi);
    if win.is_null() {
        return false;
    }
    let win = unsafe { &mut *win };

    win_goto(win);
    if resize {
        if vertsplit {
            if sz != win.w_width {
                win_setwidth(sz);
            }
        } else if sz != win.w_height
            && (win.w_height + win.w_hsep_height + win.w_status_height + tabline_height()
                < g_st().cmdline_row)
        {
            win_setheight(sz);
        }
    }

    true
}

fn qf_set_cwindow_options() {
    set_option_value_give_err(K_OPT_SWAPFILE, boolean_obj(false), OPT_LOCAL);
    set_option_value_give_err(K_OPT_BUFTYPE, cstr_as_obj(c"quickfix"), OPT_LOCAL);
    set_option_value_give_err(K_OPT_BUFHIDDEN, cstr_as_obj(c"hide"), OPT_LOCAL);
    reset_binding(curwin_mut());
    curwin_mut().w_onebuf_opt.wo_diff = 0;
    set_option_value_give_err(K_OPT_FOLDMETHOD, cstr_as_obj(c"manual"), OPT_LOCAL);
}

fn qf_open_new_cwindow(qi: &mut QfInfo, height: i32) -> bool {
    let mut oldwin = curwin_ptr();
    let prevtab = g_st().curtab;
    let mut flags: i32 = 0;

    let qf_buf = qf_find_buf(qi);
    let win = curwin_ptr();

    if g_st().cmdmod.cmod_split == 0 {
        flags = if is_qf_stack(&*qi) {
            WSP_BOT
        } else {
            WSP_BELOW
        };
    }
    flags |= WSP_NEWLOC;

    if is_qf_stack(&*qi) {
        flags |= WSP_QUICKFIX;
    }

    if !win_split(height, flags) {
        return false;
    }
    reset_binding(curwin_mut());

    if is_ll_stack(&*qi) {
        curwin_mut().w_llist_ref = qi;
        qi.qf_refcount += 1;
    }

    if oldwin != curwin_ptr() {
        oldwin = ptr::null_mut();
    }
    if !qf_buf.is_null() {
        if !do_ecmd(
            unsafe { (*qf_buf).b_fnum },
            ptr::null_mut(),
            ptr::null_mut(),
            None,
            ECMD_ONE,
            ECMD_HIDE + ECMD_OLDBUF + ECMD_NOWINENTER,
            oldwin,
        ) {
            return false;
        }
    } else {
        if !do_ecmd(
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            None,
            ECMD_ONE,
            ECMD_HIDE + ECMD_NOWINENTER,
            oldwin,
        ) {
            return false;
        }
        qi.qf_bufnr = curbuf_mut().b_fnum;
    }

    if !bt_quickfix(Some(crate::globals::curbuf_mut())) {
        qf_set_cwindow_options();
    }

    if g_st().curtab == prevtab && curwin_mut().w_width == g_st().columns_global {
        win_setheight(height);
    }
    curwin_mut().w_onebuf_opt.wo_wfh = 1;
    if win_valid(win) {
        g_st().prevwin = win;
    }
    true
}

fn qf_set_title_var(qfl: &mut QfList) {
    if let Some(ref title) = qfl.qf_title {
        set_internal_string_var(c"w:quickfix_title", title.as_ptr().cast_mut());
    }
}

pub fn ex_copen(eap: &mut Exarg) {
    let qi = qf_cmd_get_stack(eap, true);
    if qi.is_null() {
        return;
    }

    incr_quickfix_busy();

    let height = if eap.addr_count != 0 {
        eap.line2
    } else {
        QF_WINHEIGHT
    };
    reset_visual_and_resel();

    let mut status = false;
    if g_st().cmdmod.cmod_tab == 0 {
        status = qf_goto_cwindow(
            unsafe { &*qi },
            eap.addr_count != 0,
            height,
            (g_st().cmdmod.cmod_split & WSP_VERT) != 0,
        );
    }
    if !status && !qf_open_new_cwindow(unsafe { &mut *qi }, height) {
        decr_quickfix_busy();
        return;
    }

    let qfl = qf_get_curlist(unsafe { &mut *qi });
    qf_set_title_var(qfl);
    let lnum = qfl.qf_index;

    qf_fill_buffer(Some(&mut *qfl), curbuf_mut(), 0, curwin_mut().handle);

    decr_quickfix_busy();

    curwin_mut().w_cursor.lnum = lnum;
    curwin_mut().w_cursor.col = 0;
    check_cursor(curwin_mut());
    update_topline(curwin_mut());
}

fn qf_win_goto(win: *mut WinInfoInfo, lnum: Linenr) {
    let old_curwin = curwin_ptr();

    set_curwin(win);
    g_st().curbuf = unsafe { (*win).w_buffer };
    curwin_mut().w_cursor.lnum = lnum;
    curwin_mut().w_cursor.col = 0;
    curwin_mut().w_cursor.coladd = 0;
    curwin_mut().w_curswant = 0;
    update_topline(curwin_mut());
    redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
    curwin_mut().w_redr_status = true;
    set_curwin(old_curwin);
    g_st().curbuf = curwin_mut().w_buffer;
}

pub fn ex_cbottom(eap: &mut Exarg) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let win = qf_find_win(&*qi);
        if !win.is_null() && (*win).w_cursor.lnum != (*win).buf_mut().b_ml.ml_line_count {
            qf_win_goto(win, (*win).buf_mut().b_ml.ml_line_count);
        }
    }
}

pub fn qf_current_entry(wp: &mut WinInfoInfo) -> Linenr {
    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());

    if is_ll_window(&*wp) {
        qi = wp.w_llist_ref;
    }

    qf_get_curlist(unsafe { &mut *qi }).qf_index
}

fn qf_win_pos_update(qi: &mut QfInfo, old_qf_index: i32) -> bool {
    unsafe {
        let qf_index = qf_get_curlist(qi).qf_index;

        let win = qf_find_win(qi);
        if !win.is_null()
            && qf_index <= (*win).buf_mut().b_ml.ml_line_count
            && old_qf_index != qf_index
        {
            (*win).w_redraw_top = old_qf_index.min(qf_index);
            (*win).w_redraw_bot = old_qf_index.max(qf_index);
            qf_win_goto(win, qf_index);
        }
        !win.is_null()
    }
}

fn is_qf_win(win: &WinInfoInfo, qi: &QfInfo) -> bool {
    if buf_valid(win.w_buffer)
        && bt_quickfix(unsafe { win.w_buffer.as_ref() })
        && ((is_qf_stack(qi) && win.w_llist_ref.is_null())
            || (is_ll_stack(qi) && ptr::eq(win.w_llist_ref.cast_const(), qi)))
    {
        return true;
    }
    false
}

fn qf_find_win(qi: &QfInfo) -> *mut WinInfoInfo {
    for win in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if is_qf_win(&*win, qi) {
            return win;
        }
    }
    ptr::null_mut()
}

fn qf_find_buf(qi: &mut QfInfo) -> *mut FileBuf {
    if qi.qf_bufnr != INVALID_QFBUFNR {
        let qfbuf = buflist_findnr(qi.qf_bufnr);
        if !qfbuf.is_null() {
            return qfbuf;
        }
        qi.qf_bufnr = INVALID_QFBUFNR;
    }

    for tp in for_all_tabpages_mut() {
        for win in win_iter_mut(tab_firstwin(tp)) {
            if is_qf_win(win, &*qi) {
                return win.w_buffer;
            }
        }
    }

    ptr::null_mut()
}

fn qf_update_win_titlevar(qi: &mut QfInfo) {
    let qfl = qf_get_curlist(&mut *qi);
    let save_curwin = curwin_ptr();

    for tp in for_all_tabpages_mut() {
        for win in win_iter_ptr(tab_firstwin(tp)) {
            if is_qf_win(&*win, &*qi) {
                set_curwin(win);
                qf_set_title_var(qfl);
            }
        }
    }
    set_curwin(save_curwin);
}

fn qf_update_buffer(qi: &mut QfInfo, old_count: usize) {
    let buf = qf_find_buf(qi);
    if buf.is_null() {
        return;
    }
    let buf = unsafe { &mut *buf };

    let old_line_count = buf.b_ml.ml_line_count;
    let old_endcol = ml_get_buf_len(&mut *buf, old_line_count);
    let old_bytecount = get_region_bytecount(&mut *buf, 1, old_line_count, 0, old_endcol);
    let mut qf_winid: i32 = 0;

    let mut win: *mut WinInfoInfo;
    if is_ll_stack(&*qi) {
        if curwin_mut().w_llist == qi {
            win = curwin_ptr();
        } else {
            win = qf_find_win_with_loclist(qi);
            if win.is_null() {
                win = qf_find_win(&*qi);
            }
            if win.is_null() {
                return;
            }
        }
        qf_winid = unsafe { (*win).handle };
    }

    incr_quickfix_busy();

    let mut aco: CtxSwitch = CtxSwitch::new();
    if old_count == 0 {
        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
    }

    qf_update_win_titlevar(qi);

    qf_fill_buffer(
        Some(&mut *(qf_get_curlist(&mut *qi))),
        &mut *buf,
        old_count,
        qf_winid,
    );

    let new_line_count = buf.b_ml.ml_line_count;
    let new_endcol = ml_get_buf_len(&mut *buf, new_line_count);
    let delta = new_line_count - old_line_count;

    if old_count == 0 {
        let new_byte_count = get_region_bytecount(&mut *buf, 1, new_line_count, 0, new_endcol);
        extmark_splice(
            &mut *buf,
            0,
            0,
            old_line_count - 1,
            0,
            old_bytecount,
            new_line_count - 1,
            new_endcol,
            new_byte_count,
            K_EXTMARK_NO_UNDO,
        );
        changed_lines(
            &mut *buf,
            1,
            0,
            if old_line_count > 0 {
                old_line_count + 1
            } else {
                1
            },
            delta,
            true,
        );
    } else if delta > 0 {
        let start_lnum = old_line_count + 1;
        let new_byte_count =
            get_region_bytecount(&mut *buf, start_lnum, new_line_count, 0, new_endcol);
        extmark_splice(
            &mut *buf,
            old_line_count - 1,
            old_endcol,
            0,
            0,
            0,
            delta,
            new_endcol,
            new_byte_count,
            K_EXTMARK_NO_UNDO,
        );
        changed_lines(&mut *buf, start_lnum, 0, start_lnum, delta, true);
    }
    buf.b_changed = 0;

    if old_count == 0 {
        qf_win_pos_update(&mut *qi, 0);
        ctx_restore(&mut aco);
    }

    win = qf_find_win(&*qi);
    if !win.is_null() && old_line_count < unsafe { (*win).w_botline } {
        redraw_buf_later(buf, UPD_NOT_VALID);
    }

    decr_quickfix_busy();
}

fn qf_buf_add_line(
    _qfl: &QfList,
    buf: &mut FileBuf,
    lnum: Linenr,
    qfp: &Qfline,
    dirname: impl crate::strings::IntoCCharPtr,
    qftf_str: impl crate::strings::IntoCCharPtr,
    first_bufline: bool,
) -> bool {
    let dirname = dirname.into_c_char_ptr().cast_mut();
    let qftf_str = qftf_str.into_c_char_ptr();
    unsafe {
        let gap = qfga_get();

        if !crate::memory::cstr_is_empty(qftf_str) {
            qfga_concat(gap, qftf_str);
        } else {
            if let Some(ref module) = qfp.qf_module {
                qfga_concat(gap, module.as_ptr());
            } else if qfp.qf_fnum != 0 {
                let errbuf = buflist_findnr(qfp.qf_fnum);
                if !errbuf.is_null() && !(*errbuf).b_fname.is_null() {
                    if qfp.qf_type == 1 {
                        qfga_concat(gap, path_tail((*errbuf).b_fname));
                    } else {
                        if first_bufline
                            && ((*errbuf).b_sfname.is_null()
                                || path_is_absolute((*errbuf).b_sfname))
                        {
                            if crate::memory::cstr_is_empty(dirname) {
                                os_dirname(dirname, MAXPATHL);
                            }
                            shorten_buf_fname(&mut *errbuf, dirname, 0);
                        }
                        qfga_concat(
                            gap,
                            qfp.qf_fname
                                .as_ref()
                                .map_or((*errbuf).b_fname.cast_const(), |s| s.as_ptr()),
                        );
                    }
                }
            }

            gap.push(b'|');

            if qfp.qf_lnum > 0 {
                qf_range_text(gap, qfp);
                qfga_concat(gap, &qf_types(i32::from(qfp.qf_type), qfp.qf_nr));
            } else if let Some(ref pat) = qfp.qf_pattern {
                qf_fmt_text(gap, pat.as_ptr());
            }
            gap.push(b'|');
            gap.push(b' ');

            let text_arg = if gap.len() > 3 {
                skipwhite(qfp.qf_text.as_ptr())
            } else {
                qfp.qf_text.as_ptr()
            };
            qf_fmt_text(gap, text_arg);
        }

        gap.push(0);
        if !ml_append_buf(&mut *buf, lnum, gap.as_mut_ptr().cast(), gap.len() as Colnr, false) {
            return false;
        }

        true
    }
}

fn call_qftf_func(qfl: &mut QfList, qf_winid: i32, start_idx: i32, end_idx: i32) -> *mut TvList {
    unsafe {
        let mut cb: *mut Callback = &raw mut opt_vars().p_qftf;
        let mut qftf_list: *mut TvList = ptr::null_mut();

        if qf_st().qftf_recursive {
            return ptr::null_mut();
        }
        qf_st().qftf_recursive = true;

        if qfl.qf_qftf_cb.cb_type() != K_CALLBACK_NONE_CONST {
            cb = &raw mut qfl.qf_qftf_cb;
        }
        if (*cb).cb_type() != K_CALLBACK_NONE_CONST {
            let mut args: [TypVal; 1] = [TypVal::new(); 1];
            let mut rettv: TypVal = TypVal::new();

            let dict = tv_dict_alloc_lock(VAR_FIXED);
            tv_dict_add_nr(&mut *dict, c"quickfix", Varnumber::from(is_qf_list(qfl)));
            tv_dict_add_nr(&mut *dict, c"winid", Varnumber::from(qf_winid));
            tv_dict_add_nr(&mut *dict, c"id", Varnumber::from(qfl.qf_id));
            tv_dict_add_nr(&mut *dict, c"start_idx", Varnumber::from(start_idx));
            tv_dict_add_nr(&mut *dict, c"end_idx", Varnumber::from(end_idx));
            dict.dv_refcount += 1;
            args[0].v_type = VarType::VarDict;
            args[0].vval = crate::eval::typval::TypvalVval::Dict(dict);

            g_st().textlock += 1;
            if callback_call(&mut *cb, 1, args.as_mut_ptr(), &mut rettv) {
                if rettv.v_type == VarType::VarList {
                    qftf_list = rettv.vval.v_list();
                    tv_list_ref(qftf_list.as_mut());
                }
                tv_clear(&mut rettv);
            }
            g_st().textlock -= 1;
            tv_dict_unref(dict);
        }

        qf_st().qftf_recursive = false;
        qftf_list
    }
}

fn qf_fill_buffer(qfl: Option<&mut QfList>, buf: &mut FileBuf, old_count: usize, qf_winid: i32) {
    let old_key_typed = g_st().key_typed;

    if old_count == 0 {
        if !ptr::eq(buf, g_st().curbuf) {
            internal_error(c"qf_fill_buffer()");
            return;
        }

        while (curbuf_mut().b_ml.ml_flags & ML_EMPTY) == 0 {
            if !ml_delete(1) {
                internal_error(c"qf_fill_buffer()");
                return;
            }
        }

        for tp in for_all_tabpages_mut() {
            for wp in win_iter_mut(tab_firstwin(tp)) {
                if wp.w_buffer == g_st().curbuf {
                    wp.w_skipcol = 0;
                }
            }
        }

        u_clearallandblockfree(curbuf_mut());
    }

    if let Some(qfl) = qfl
        && !qfl.qf_items.is_empty()
    {
        let mut dirname = [0u8; MAXPATHL];
        let mut lnum: Linenr = if old_count == 0 {
            0
        } else {
            buf.b_ml.ml_line_count
        };

        let qfl_cnt = qfl.count();
        let qftf_list = call_qftf_func(qfl, qf_winid, lnum + 1, qfl_cnt);
        let mut qftf_li = tv_list_first(unsafe { qftf_list.as_ref() });

        let mut prev_bufnr: i32 = -1;
        let mut invalid_val = false;

        while (lnum as usize) < qfl.qf_items.len() {
            let mut qftf_str = ptr::null_mut();

            if !qftf_li.is_null() && !invalid_val {
                qftf_str = tv_get_string_chk(unsafe { &(*qftf_li).li_tv }).cast_mut();
                if qftf_str.is_null() {
                    invalid_val = true;
                }
            }

            let qfp = &raw mut (&mut qfl.qf_items)[lnum as usize];
            let qfp_fnum = unsafe { (*qfp).qf_fnum };
            if !qf_buf_add_line(
                qfl,
                &mut *buf,
                lnum,
                unsafe { &*qfp },
                dirname.as_mut_ptr().cast(),
                qftf_str,
                prev_bufnr != qfp_fnum,
            ) {
                break;
            }
            prev_bufnr = qfp_fnum;
            lnum += 1;

            if !qftf_li.is_null() {
                qftf_li = unsafe { (*qftf_li).li_next };
            }
        }
        if old_count == 0 {
            ml_delete(lnum + 1);
        }

        tv_list_unref(qftf_list);
        qfga_clear();
    }

    check_lnums(true);

    if old_count == 0 {
        curbuf_mut().b_ro_locked += 1;
        set_option_value_give_err(K_OPT_FILETYPE, cstr_as_obj(c"qf"), OPT_LOCAL);
        curbuf_mut().b_p_ma = 0;

        curbuf_mut().b_keep_filetype = true;
        apply_autocmds(
            EVENT_BUFREADPOST,
            c"quickfix",
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        apply_autocmds(
            EVENT_BUFWINENTER,
            c"quickfix",
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        curbuf_mut().b_keep_filetype = false;
        curbuf_mut().b_ro_locked -= 1;

        redraw_curbuf_later(UPD_NOT_VALID);
    }

    g_st().key_typed = old_key_typed;
}

#[inline]
fn qf_list_changed(qfl: &mut QfList) {
    qfl.qf_changedtick += 1;
}

fn qf_id2nr(qi: &QfInfo, qfid: u32) -> i32 {
    for qf_idx in 0..qi.qf_listcount {
        if (&qi.qf_lists)[qf_idx as usize].qf_id == qfid {
            return qf_idx;
        }
    }
    INVALID_QFIDX
}

fn qf_restore_list(qi: &mut QfInfo, save_qfid: u32) -> bool {
    if qf_get_curlist(&mut *qi).qf_id == save_qfid {
        return true;
    }

    let curlist = qf_id2nr(qi, save_qfid);
    if curlist < 0 {
        return false;
    }
    qi.qf_curlist = curlist;
    true
}

fn qf_jump_first(qi: &mut QfInfo, save_qfid: u32, forceit: i32) {
    if !qf_restore_list(qi, save_qfid) {
        return;
    }

    if !check_can_set_curbuf_forceit(forceit) {
        return;
    }

    if !qf_list_empty(Some(&*(qf_get_curlist(&mut *qi)))) {
        qf_jump(&raw mut *qi, 0, 0, forceit);
    }
}

pub fn grep_internal(cmdidx: i32) -> i32 {
    let gp = if crate::memory::cstr_is_empty(curbuf_mut().b_p_gp) {
        opt_vars().p_gp
    } else {
        curbuf_mut().b_p_gp
    };
    i32::from(
        (cmdidx == CMD_GREP
            || cmdidx == CMD_LGREP
            || cmdidx == CMD_GREPADD
            || cmdidx == CMD_LGREPADD)
            && b"internal" == crate::memory::cstr_as_bytes(gp),
    )
}

fn make_get_auname(cmdidx: i32) -> Option<&'static CStr> {
    match cmdidx {
        CMD_MAKE => Some(c"make"),
        CMD_LMAKE => Some(c"lmake"),
        CMD_GREP => Some(c"grep"),
        CMD_LGREP => Some(c"lgrep"),
        CMD_GREPADD => Some(c"grepadd"),
        CMD_LGREPADD => Some(c"lgrepadd"),
        _ => None,
    }
}

fn make_get_fullcmd(
    makecmd: impl crate::strings::IntoCCharPtr,
    fname: impl crate::strings::IntoCCharPtr,
) -> std::ffi::CString {
    let makecmd = makecmd.into_c_char_ptr();
    let fname = fname.into_c_char_ptr();
    {
        let mut len =
            crate::memory::cstr_len(opt_vars().p_shq) * 2 + crate::memory::cstr_len(makecmd) + 1;
        if !crate::memory::cstr_is_empty(opt_vars().p_sp) {
            len += crate::memory::cstr_len(opt_vars().p_sp) + crate::memory::cstr_len(fname) + 3;
        }
        let mut cmd_vec: Vec<u8> = vec![0; len];
        let cmd: *mut _ = cmd_vec.as_mut_ptr().cast();
        vim_snprintf!(
            cmd,
            len,
            c"%s%s%s",
            opt_vars().p_shq,
            makecmd,
            opt_vars().p_shq
        );

        if !crate::memory::cstr_is_empty(opt_vars().p_sp) {
            append_redir(cmd, len, opt_vars().p_sp, fname);
        }

        if g_st().msg_col == 0 {
            g_st().msg_didout = false;
        }
        msg_start();
        msg_puts(c":!");
        msg_outtrans(cmd, 0, false);

        crate::memory::cstr_to_cstring(cmd)
    }
}

pub fn ex_make(eap: &mut Exarg) {
    let enc = if !crate::memory::cstr_is_empty(curbuf_mut().b_p_menc) {
        curbuf_mut().b_p_menc
    } else {
        opt_vars().p_menc
    };

    if grep_internal(eap.cmdidx) != 0 {
        ex_vimgrep(eap);
        return;
    }

    let au_name = make_get_auname(eap.cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            au_name.as_ptr().cast_mut(),
            curbuf_mut().b_fname,
            true,
            g_st().curbuf,
        )
        && aborting()
    {
        return;
    }

    let mut wp: *mut WinInfoInfo = ptr::null_mut();
    if is_loclist_cmd(eap.cmdidx) {
        wp = curwin_ptr();
    }

    autowrite_all();
    let Some(fname) = get_mef_name() else {
        return;
    };
    let fname_ptr = fname.as_ptr().cast_mut();
    os_remove(fname_ptr);

    let cmd = make_get_fullcmd(eap.arg, fname_ptr);

    do_shell(cmd.as_ptr().cast_mut(), 0);

    incr_quickfix_busy();

    let errorformat = if eap.cmdidx != CMD_MAKE && eap.cmdidx != CMD_LMAKE {
        if !crate::memory::cstr_is_empty(curbuf_mut().b_p_gefm) {
            curbuf_mut().b_p_gefm
        } else {
            opt_vars().p_gefm
        }
    } else {
        opt_vars().p_efm
    };

    let newlist = eap.cmdidx != CMD_GREPADD && eap.cmdidx != CMD_LGREPADD;

    let res = qf_init(
        unsafe { wp.as_mut() },
        fname_ptr,
        errorformat,
        i32::from(newlist),
        &qf_cmdtitle(unsafe { *eap.cmdlinep }),
        enc,
    );

    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());
    if !wp.is_null() {
        qi = get_loc_list(unsafe { &*wp });
    }
    if !qi.is_null() {
        if res >= 0 {
            qf_list_changed(qf_get_curlist(unsafe { &mut *qi }));
        }
        let save_qfid = qf_get_curlist(unsafe { &mut *qi }).qf_id;
        if let Some(au_name) = au_name {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.as_ptr().cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            );
        }
        if res > 0 && eap.forceit == 0 && qflist_valid(wp, save_qfid) {
            qf_jump_first(unsafe { &mut *qi }, save_qfid, 0);
        }
    }

    decr_quickfix_busy();
    os_remove(fname_ptr);
}

fn get_mef_name() -> Option<std::ffi::CString> {
    if crate::memory::cstr_is_empty(opt_vars().p_mef) {
        let name = vim_tempname();
        if name.is_none() {
            emsg(gettext(E_NOTMP));
        }
        return name;
    }

    let mut p = opt_vars().p_mef;
    while !crate::memory::cstr_is_empty(p) {
        if crate::memory::cstr_first_byte(p) == b'#' && crate::memory::cstr_byte_at(p, 1) == b'#' {
            break;
        }
        p = p.wrapping_add(1);
    }

    if crate::memory::cstr_is_empty(p) {
        return Some(crate::memory::cstr_to_cstring(opt_vars().p_mef));
    }

    loop {
        if qf_st().mef_start == -1 {
            qf_st().mef_start = os_get_pid() as i32;
        } else {
            qf_st().mef_off += 19;
        }
        let mef_len = crate::memory::cstr_len(opt_vars().p_mef);
        let mut name_vec: Vec<u8> = vec![0; mef_len + 30];
        let name: *mut _ = name_vec.as_mut_ptr().cast();
        crate::memory::cstr_cpy(name, opt_vars().p_mef);
        let off_idx = crate::memory::ptr_offset_from(p, opt_vars().p_mef) as usize;
        vim_snprintf!(
            name.wrapping_add(off_idx),
            mef_len + 30 - off_idx,
            c"%d%d",
            qf_st().mef_start,
            qf_st().mef_off,
        );
        c_strcat(name, p.wrapping_add(2));
        let mut file_info = FileInfo::new();
        let file_or_link_found = os_fileinfo_link(name, &mut file_info);
        if !file_or_link_found {
            return Some(crate::memory::cstr_to_cstring(name));
        }
    }
}

pub fn qf_get_size(eap: &mut Exarg) -> usize {
    let qi = qf_cmd_get_stack(eap, false);
    if qi.is_null() {
        return 0;
    }
    qf_get_curlist(unsafe { &mut *qi }).qf_items.len()
}

pub fn qf_get_valid_size(eap: &mut Exarg) -> usize {
    let qi = qf_cmd_get_stack(eap, false);
    if qi.is_null() {
        return 0;
    }

    let mut prev_fnum: i32 = 0;
    let mut sz: usize = 0;
    let qfl = qf_get_curlist(unsafe { &mut *qi });
    for qfp in &qfl.qf_items {
        if g_st().got_int {
            break;
        }
        if qfp.qf_valid != 0 {
            if eap.cmdidx == CMD_CDO || eap.cmdidx == CMD_LDO {
                sz += 1;
            } else if qfp.qf_fnum > 0 && qfp.qf_fnum != prev_fnum {
                sz += 1;
                prev_fnum = qfp.qf_fnum;
            }
        }
    }

    sz
}

pub fn qf_get_cur_idx(eap: &mut Exarg) -> usize {
    let qi = qf_cmd_get_stack(eap, false);
    if qi.is_null() {
        return 0;
    }

    assert!(qf_get_curlist(unsafe { &mut *qi }).qf_index >= 0);
    qf_get_curlist(unsafe { &mut *qi }).qf_index as usize
}

pub fn qf_get_cur_valid_idx(eap: &mut Exarg) -> i32 {
    let qi = qf_cmd_get_stack(eap, false);
    if qi.is_null() {
        return 1;
    }

    let qfl = qf_get_curlist(unsafe { &mut *qi });
    if !qf_list_has_valid_entries(qfl) {
        return 1;
    }

    let mut prev_fnum: i32 = 0;
    let mut eidx: i32 = 0;
    assert!(qfl.qf_index >= 0);
    let limit = (qfl.qf_index as usize).min(qfl.qf_items.len());
    for qfp in &(&qfl.qf_items)[..limit] {
        if qfp.qf_valid != 0 {
            if eap.cmdidx == CMD_CFDO || eap.cmdidx == CMD_LFDO {
                if qfp.qf_fnum > 0 && qfp.qf_fnum != prev_fnum {
                    eidx += 1;
                    prev_fnum = qfp.qf_fnum;
                }
            } else {
                eidx += 1;
            }
        }
    }

    if eidx != 0 { eidx } else { 1 }
}

fn qf_get_nth_valid_entry(qfl: &mut QfList, n: usize, fdo: bool) -> usize {
    if !qf_list_has_valid_entries(qfl) {
        return 1;
    }

    let mut prev_fnum: i32 = 0;
    let mut eidx: usize = 0;
    for (idx, qfp) in qfl.qf_items.iter().enumerate() {
        if g_st().got_int {
            break;
        }
        if qfp.qf_valid != 0 {
            if fdo {
                if qfp.qf_fnum > 0 && qfp.qf_fnum != prev_fnum {
                    eidx += 1;
                    prev_fnum = qfp.qf_fnum;
                }
            } else {
                eidx += 1;
            }
        }

        if eidx == n {
            return idx + 1;
        }
    }

    1
}

pub fn ex_cc(eap: &mut Exarg) {
    let qi = qf_cmd_get_stack(eap, true);
    if qi.is_null() {
        return;
    }

    let mut errornr: i32 = if eap.addr_count > 0 {
        eap.line2
    } else {
        match eap.cmdidx {
            CMD_CC | CMD_LL => 0,
            CMD_CREWIND | CMD_LREWIND | CMD_CFIRST | CMD_LFIRST => 1,
            _ => 32767,
        }
    };

    if eap.cmdidx == CMD_CDO
        || eap.cmdidx == CMD_LDO
        || eap.cmdidx == CMD_CFDO
        || eap.cmdidx == CMD_LFDO
    {
        let n = if eap.addr_count > 0 {
            assert!(eap.line1 >= 0);
            eap.line1 as usize
        } else {
            1
        };
        let valid_entry = qf_get_nth_valid_entry(
            qf_get_curlist(unsafe { &mut *qi }),
            n,
            eap.cmdidx == CMD_CFDO || eap.cmdidx == CMD_LFDO,
        );
        assert!(valid_entry <= i32::MAX as usize);
        errornr = valid_entry as i32;
    }

    qf_jump(qi, 0, errornr, eap.forceit);
}

pub fn ex_cnext(eap: &mut Exarg) {
    let qi = qf_cmd_get_stack(eap, true);
    if qi.is_null() {
        return;
    }

    let errornr = if eap.addr_count > 0
        && eap.cmdidx != CMD_CDO
        && eap.cmdidx != CMD_LDO
        && eap.cmdidx != CMD_CFDO
        && eap.cmdidx != CMD_LFDO
    {
        eap.line2
    } else {
        1
    };

    let dir = match eap.cmdidx {
        CMD_CPREVIOUS | CMD_LPREVIOUS | CMD_CNEXT | CMD_LNEXT => BACKWARD,
        CMD_CNFILE_VARIANT | CMD_LNFILE_VARIANT | CMD_CFDO | CMD_LFDO => FORWARD_FILE,
        CMD_CPFILE | CMD_LPFILE | CMD_CNFILE | CMD_LNFILE => BACKWARD_FILE,
        _ => FORWARD,
    };

    qf_jump(qi, dir, errornr, eap.forceit);
}

fn qf_find_first_entry_in_buf(items: &[Qfline], bnr: i32) -> Option<usize> {
    for (idx, qfp) in items.iter().enumerate() {
        if g_st().got_int {
            break;
        }
        if qfp.qf_fnum == bnr {
            return Some(idx);
        }
    }
    None
}

fn qf_find_first_entry_on_line(items: &[Qfline], mut idx: usize) -> usize {
    while !g_st().got_int
        && idx > 0
        && items[idx].qf_fnum == items[idx - 1].qf_fnum
        && items[idx].qf_lnum == items[idx - 1].qf_lnum
    {
        idx -= 1;
    }
    idx
}

fn qf_find_last_entry_on_line(items: &[Qfline], mut idx: usize) -> usize {
    while !g_st().got_int
        && idx + 1 < items.len()
        && items[idx].qf_fnum == items[idx + 1].qf_fnum
        && items[idx].qf_lnum == items[idx + 1].qf_lnum
    {
        idx += 1;
    }
    idx
}

#[inline]
fn qf_entry_after_pos(qfp: &Qfline, pos: &CursorPos, linewise: bool) -> bool {
    if linewise {
        qfp.qf_lnum > pos.lnum
    } else {
        qfp.qf_lnum > pos.lnum || (qfp.qf_lnum == pos.lnum && qfp.qf_col > pos.col)
    }
}

#[inline]
fn qf_entry_before_pos(qfp: &Qfline, pos: &CursorPos, linewise: bool) -> bool {
    if linewise {
        qfp.qf_lnum < pos.lnum
    } else {
        qfp.qf_lnum < pos.lnum || (qfp.qf_lnum == pos.lnum && qfp.qf_col < pos.col)
    }
}

#[inline]
fn qf_entry_on_or_after_pos(qfp: &Qfline, pos: &CursorPos, linewise: bool) -> bool {
    if linewise {
        qfp.qf_lnum >= pos.lnum
    } else {
        qfp.qf_lnum > pos.lnum || (qfp.qf_lnum == pos.lnum && qfp.qf_col >= pos.col)
    }
}

#[inline]
fn qf_entry_on_or_before_pos(qfp: &Qfline, pos: &CursorPos, linewise: bool) -> bool {
    if linewise {
        qfp.qf_lnum <= pos.lnum
    } else {
        qfp.qf_lnum < pos.lnum || (qfp.qf_lnum == pos.lnum && qfp.qf_col <= pos.col)
    }
}

fn qf_find_entry_after_pos(
    items: &[Qfline],
    bnr: i32,
    pos: &CursorPos,
    linewise: bool,
    mut idx: usize,
) -> Option<usize> {
    if qf_entry_after_pos(&items[idx], pos, linewise) {
        return Some(idx);
    }

    while idx + 1 < items.len()
        && items[idx + 1].qf_fnum == bnr
        && qf_entry_on_or_before_pos(&items[idx + 1], pos, linewise)
    {
        idx += 1;
    }

    if idx + 1 >= items.len() || items[idx + 1].qf_fnum != bnr {
        return None;
    }

    Some(idx + 1)
}

fn qf_find_entry_before_pos(
    items: &[Qfline],
    bnr: i32,
    pos: &CursorPos,
    linewise: bool,
    mut idx: usize,
) -> Option<usize> {
    while idx + 1 < items.len()
        && items[idx + 1].qf_fnum == bnr
        && qf_entry_before_pos(&items[idx + 1], pos, linewise)
    {
        idx += 1;
    }

    if qf_entry_on_or_after_pos(&items[idx], pos, linewise) {
        return None;
    }

    if linewise {
        idx = qf_find_first_entry_on_line(items, idx);
    }

    Some(idx)
}

fn qf_find_closest_entry(
    items: &[Qfline],
    bnr: i32,
    pos: &CursorPos,
    dir: Direction,
    linewise: bool,
) -> Option<usize> {
    let idx = qf_find_first_entry_in_buf(items, bnr)?;

    if dir == FORWARD {
        qf_find_entry_after_pos(items, bnr, pos, linewise, idx)
    } else {
        qf_find_entry_before_pos(items, bnr, pos, linewise, idx)
    }
}

fn qf_get_nth_below_entry(
    items: &[Qfline],
    mut idx: usize,
    mut n: Linenr,
    linewise: bool,
) -> usize {
    while n > 0 && !g_st().got_int {
        n -= 1;
        let first_idx = idx;

        if linewise {
            idx = qf_find_last_entry_on_line(items, idx);
        }

        if idx + 1 >= items.len() || items[idx + 1].qf_fnum != items[idx].qf_fnum {
            if linewise {
                idx = first_idx;
            }
            break;
        }

        idx += 1;
    }
    idx
}

fn qf_get_nth_above_entry(
    items: &[Qfline],
    mut idx: usize,
    mut n: Linenr,
    linewise: bool,
) -> usize {
    while n > 0 && !g_st().got_int {
        n -= 1;
        if idx == 0 || items[idx - 1].qf_fnum != items[idx].qf_fnum {
            break;
        }

        idx -= 1;

        if linewise {
            idx = qf_find_first_entry_on_line(items, idx);
        }
    }
    idx
}

fn qf_find_nth_adj_entry(
    qfl: &mut QfList,
    bnr: i32,
    pos: &mut CursorPos,
    mut n: Linenr,
    dir: Direction,
    linewise: bool,
) -> i32 {
    let items = &qfl.qf_items;
    let Some(mut idx) = qf_find_closest_entry(items, bnr, pos, dir, linewise) else {
        return 0;
    };

    n -= 1;
    if n > 0 {
        if dir == FORWARD {
            idx = qf_get_nth_below_entry(items, idx, n, linewise);
        } else {
            idx = qf_get_nth_above_entry(items, idx, n, linewise);
        }
    }

    (idx + 1) as i32
}

pub fn ex_cbelow(eap: &mut Exarg) {
    if eap.addr_count > 0 && eap.line2 <= 0 {
        emsg(gettext(E_INVRANGE));
        return;
    }

    let buf_has_flag = if eap.cmdidx == CMD_CABOVE
        || eap.cmdidx == CMD_CBELOW
        || eap.cmdidx == CMD_CBEFORE
        || eap.cmdidx == CMD_CAFTER
    {
        BUF_HAS_QF_ENTRY
    } else {
        BUF_HAS_LL_ENTRY
    };

    if (curbuf_mut().b_has_qf_entry & buf_has_flag) == 0 {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    let qi = qf_cmd_get_stack(eap, true);
    if qi.is_null() {
        return;
    }

    let qfl = qf_get_curlist(unsafe { &mut *qi });
    if !qf_list_has_valid_entries(qfl) {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    let dir = if eap.cmdidx == CMD_CBELOW
        || eap.cmdidx == CMD_LBELOW
        || eap.cmdidx == CMD_CAFTER
        || eap.cmdidx == CMD_LAFTER
    {
        FORWARD
    } else {
        BACKWARD
    };

    let mut pos = curwin_mut().w_cursor;
    pos.col += 1;
    let errornr = qf_find_nth_adj_entry(
        qfl,
        curbuf_mut().b_fnum,
        &mut pos,
        if eap.addr_count > 0 { eap.line2 } else { 0 },
        dir,
        eap.cmdidx == CMD_CBELOW
            || eap.cmdidx == CMD_LBELOW
            || eap.cmdidx == CMD_CABOVE
            || eap.cmdidx == CMD_LABOVE,
    );

    if errornr > 0 {
        qf_jump(qi, 0, errornr, 0);
    } else {
        emsg(gettext(E_NO_MORE_ITEMS));
    }
}

fn cfile_get_auname(cmdidx: i32) -> Option<&'static CStr> {
    match cmdidx {
        CMD_CFILE => Some(c"cfile"),
        CMD_CGETFILE => Some(c"cgetfile"),
        CMD_CADDFILE => Some(c"caddfile"),
        CMD_LFILE => Some(c"lfile"),
        CMD_LGETFILE => Some(c"lgetfile"),
        CMD_LADDFILE => Some(c"laddfile"),
        _ => None,
    }
}

pub fn ex_cfile(eap: &mut Exarg) {
    let mut wp: *mut WinInfoInfo = ptr::null_mut();
    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());

    let au_name = cfile_get_auname(eap.cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            au_name.as_ptr().cast_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        )
        && aborting()
    {
        return;
    }
    if !crate::memory::cstr_is_empty(eap.arg) {
        set_option_direct(K_OPT_ERRORFILE, cstr_as_obj(eap.arg), 0, 0);
    }

    let enc = if !crate::memory::cstr_is_empty(curbuf_mut().b_p_menc) {
        curbuf_mut().b_p_menc
    } else {
        opt_vars().p_menc
    };

    if is_loclist_cmd(eap.cmdidx) {
        wp = curwin_ptr();
    }

    incr_quickfix_busy();

    let res = qf_init(
        unsafe { wp.as_mut() },
        opt_vars().p_ef,
        opt_vars().p_efm,
        i32::from(eap.cmdidx != CMD_CADDFILE && eap.cmdidx != CMD_LADDFILE),
        &qf_cmdtitle(unsafe { *eap.cmdlinep }),
        enc,
    );
    if !wp.is_null() {
        qi = get_loc_list(unsafe { &*wp });
        if qi.is_null() {
            decr_quickfix_busy();
            return;
        }
    }
    if res >= 0 {
        qf_list_changed(qf_get_curlist(unsafe { &mut *qi }));
    }
    let save_qfid = qf_get_curlist(unsafe { &mut *qi }).qf_id;
    if let Some(au_name) = au_name {
        apply_autocmds(
            EVENT_QUICKFIXCMDPOST,
            au_name.as_ptr().cast_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
    }
    if res > 0
        && (eap.cmdidx == CMD_CFILE || eap.cmdidx == CMD_LFILE)
        && qflist_valid(wp, save_qfid)
    {
        qf_jump_first(unsafe { &mut *qi }, save_qfid, eap.forceit);
    }

    decr_quickfix_busy();
}

fn vgr_get_auname(cmdidx: i32) -> Option<&'static CStr> {
    match cmdidx {
        CMD_VIMGREP => Some(c"vimgrep"),
        CMD_LVIMGREP => Some(c"lvimgrep"),
        CMD_VIMGREPADD => Some(c"vimgrepadd"),
        CMD_LVIMGREPADD => Some(c"lvimgrepadd"),
        CMD_GREP => Some(c"grep"),
        CMD_LGREP => Some(c"lgrep"),
        CMD_GREPADD => Some(c"grepadd"),
        CMD_LGREPADD => Some(c"lgrepadd"),
        _ => None,
    }
}

fn vgr_init_regmatch(regmatch: &mut Regmmatch, s: impl crate::strings::IntoCCharPtr) {
    let s = s.into_c_char_ptr();
    {
        regmatch.regprog = ptr::null_mut();

        if crate::memory::cstr_is_empty(s) {
            if last_search_pat().is_none() {
                emsg(gettext(E_NOPREVRE));
                return;
            }
            regmatch.regprog = vim_regcomp(last_search_pat(), RE_MAGIC);
        } else {
            regmatch.regprog = vim_regcomp(s, RE_MAGIC);
        }

        regmatch.rmm_ic = opt_vars().p_ic;
        regmatch.rmm_maxcol = 0;
    }
}

fn vgr_display_fname(fname: impl crate::strings::IntoCCharPtr) {
    let fname = fname.into_c_char_ptr();
    msg_start();
    match msg_strtrunc(fname, 1) {
        Some(p) => msg_outtrans(p.as_ptr(), 0, false),
        None => msg_outtrans(fname, 0, false),
    };
    msg_clr_eos();
    g_st().msg_didout = false;
    g_st().msg_nowait = true;
    g_st().msg_col = 0;
    ui_flush();
}

fn vgr_load_dummy_buf(
    fname: impl crate::strings::IntoCCharPtr,
    dirname_start: impl crate::strings::IntoCCharPtr,
    dirname_now: impl crate::strings::IntoCCharPtr,
) -> *mut FileBuf {
    let fname = fname.into_c_char_ptr().cast_mut();
    let dirname_start = dirname_start.into_c_char_ptr().cast_mut();
    let dirname_now = dirname_now.into_c_char_ptr().cast_mut();
    let save_ei = au_event_disable(c",Filetype");

    let save_mls = opt_vars().p_mls;
    opt_vars().p_mls = 0;

    let buf = load_dummy_buffer(fname, dirname_start, dirname_now);

    opt_vars().p_mls = save_mls;
    au_event_restore(Some(save_ei));

    buf
}

fn vgr_qflist_valid(
    wp: *mut WinInfoInfo,
    qi: &mut QfInfo,
    qfid: u32,
    title: impl crate::strings::IntoCCharPtr,
) -> bool {
    let title = title.into_c_char_ptr();
    if !qflist_valid(wp, qfid) {
        if !wp.is_null() {
            emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
            return false;
        }
        qf_new_list(qi, title);
        return true;
    }
    if !qf_restore_list(qi, qfid) {
        return false;
    }

    true
}

fn vgr_match_buflines(
    qfl: &mut QfList,
    fname: impl crate::strings::IntoCCharPtr,
    buf: &mut FileBuf,
    spat: impl crate::strings::IntoCCharPtr,
    regmatch: &mut Regmmatch,
    tomatch: &mut i32,
    duplicate_name: i32,
    flags: i32,
) -> bool {
    let fname = fname.into_c_char_ptr().cast_mut();
    let spat = spat.into_c_char_ptr();
    {
        let mut found_match = false;
        let pat_len = crate::memory::cstr_len(spat).min(FUZZY_MATCH_MAX_LEN);

        let mut lnum: Linenr = 1;
        while lnum <= buf.b_ml.ml_line_count && *tomatch > 0 {
            let mut col: Colnr = 0;
            if (flags & VGR_FUZZY) == 0 {
                while vim_regexec_multi(
                    regmatch,
                    curwin_ptr(),
                    &raw mut *buf,
                    lnum,
                    col,
                    None,
                    None,
                ) > 0
                {
                    if qf_add_entry(
                        qfl,
                        ptr::null_mut(),
                        fname,
                        ptr::null_mut(),
                        if duplicate_name != 0 { 0 } else { buf.b_fnum },
                        ml_get_buf(buf, regmatch.startpos[0].lnum + lnum),
                        regmatch.startpos[0].lnum + lnum,
                        regmatch.endpos[0].lnum + lnum,
                        regmatch.startpos[0].col + 1,
                        regmatch.endpos[0].col + 1,
                        0,
                        ptr::null_mut(),
                        0,
                        0,
                        None,
                        1,
                    ) == QF_FAIL
                    {
                        g_st().got_int = true;
                        break;
                    }
                    found_match = true;
                    *tomatch -= 1;
                    if *tomatch == 0 {
                        break;
                    }
                    if (flags & VGR_GLOBAL) == 0 || regmatch.endpos[0].lnum > 0 {
                        break;
                    }
                    col = regmatch.endpos[0].col + Colnr::from(col == regmatch.endpos[0].col);
                    if col > ml_get_buf_len(buf, lnum) {
                        break;
                    }
                }
            } else {
                let str_ = ml_get_buf(buf, lnum);
                let linelen = ml_get_buf_len(buf, lnum);
                let mut score: i32 = 0;
                let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];

                while fuzzy_match(
                    str_.wrapping_add(col as usize),
                    spat,
                    false,
                    &mut score,
                    matches.as_mut_ptr(),
                    FUZZY_MATCH_MAX_LEN as i32,
                ) {
                    if qf_add_entry(
                        qfl,
                        ptr::null_mut(),
                        fname,
                        ptr::null_mut(),
                        if duplicate_name != 0 { 0 } else { buf.b_fnum },
                        str_,
                        lnum,
                        0,
                        matches[0] as Colnr + col + 1,
                        0,
                        0,
                        ptr::null_mut(),
                        0,
                        0,
                        None,
                        1,
                    ) == QF_FAIL
                    {
                        g_st().got_int = true;
                        break;
                    }
                    found_match = true;
                    *tomatch -= 1;
                    if *tomatch == 0 {
                        break;
                    }
                    if (flags & VGR_GLOBAL) == 0 {
                        break;
                    }
                    col = matches[pat_len - 1] as Colnr + col + 1;
                    if col > linelen {
                        break;
                    }
                }
            }
            line_breakcheck();
            if g_st().got_int {
                break;
            }
            lnum += 1;
        }

        found_match
    }
}

fn vgr_jump_to_match(
    qi: &mut QfInfo,
    forceit: i32,
    redraw_for_dummy: &mut bool,
    first_match_buf: *mut FileBuf,
    target_dir: impl crate::strings::IntoCCharPtr,
) {
    let target_dir = target_dir.into_c_char_ptr().cast_mut();
    let buf = g_st().curbuf;
    qf_jump(&raw mut *qi, 0, 0, forceit);
    if buf != g_st().curbuf {
        *redraw_for_dummy = false;
    }

    if g_st().curbuf == first_match_buf && !target_dir.is_null() {
        let mut ea: Exarg = Exarg::new();
        ea.arg = target_dir;
        ea.cmdidx = CMD_LCD;
        ex_cd(&mut ea);
    }
}

fn existing_swapfile(buf: &FileBuf) -> bool {
    if let Some(ref mfp) = buf.b_ml.ml_mfp
        && let Some(ref mf_fname) = mfp.mf_fname
    {
        let bytes = mf_fname.as_bytes();
        if bytes.len() >= 2 {
            return !bytes.ends_with(b"wp");
        }
        return true;
    }
    false
}

fn vgr_process_args(eap: &mut Exarg, args: &mut VgrArgs) -> bool {
    *args = VgrArgs::new();

    args.regmatch.regprog = ptr::null_mut();
    args.qf_title = Some(qf_cmdtitle(unsafe { *eap.cmdlinep }));
    args.tomatch = if eap.addr_count > 0 {
        eap.line2
    } else {
        MAXLNUM
    };

    let mut spat = ptr::null_mut();
    let mut p = skip_vimgrep_pat(eap.arg, Some(&mut spat), Some(&mut args.flags));
    if p.is_null() {
        emsg(gettext(E_INVALPAT));
        return false;
    }
    args.spat = (!spat.is_null()).then(|| crate::memory::cstr_to_cstring(spat));

    vgr_init_regmatch(&mut args.regmatch, args.spat.as_deref());
    if args.regmatch.regprog.is_null() {
        return false;
    }

    p = skipwhite(p);
    if crate::memory::cstr_is_empty(p) {
        emsg(gettext(c"E683: File name missing or invalid pattern"));
        return false;
    }

    if !get_arglist_exp(p, &mut args.fnames, true) || args.fnames.is_empty() {
        emsg(gettext(E_NOMATCH));
        return false;
    }

    true
}

fn vgr_process_files(
    wp: *mut WinInfoInfo,
    qi: &mut QfInfo,
    cmd_args: &mut VgrArgs,
    redraw_for_dummy: &mut bool,
    first_match_buf: *mut *mut FileBuf,
    target_dir: &mut Option<std::ffi::CString>,
) -> bool {
    unsafe {
        let mut status = false;
        let mut save_qfid = qf_get_curlist(&mut *qi).qf_id;
        let mut duplicate_name = false;

        let mut dirname_start = vec![0u8; MAXPATHL];
        let mut dirname_now = vec![0u8; MAXPATHL];

        os_dirname(dirname_start.as_mut_ptr(), MAXPATHL);

        let mut seconds: time_t = 0;
        let mut fi: usize = 0;
        let mut aborted = false;
        while fi < cmd_args.fnames.len() && !g_st().got_int && cmd_args.tomatch > 0 {
            let fname_exp = cmd_args.fnames.as_slice()[fi].as_ptr().cast_mut();
            let fname = path_try_shorten_fname(fname_exp);
            let now = os_time() as time_t;
            if now > seconds {
                seconds = now;
                vgr_display_fname(fname);
            }

            let mut buf = buflist_findname_exp(fname_exp);
            let using_dummy: bool;
            if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() {
                duplicate_name = !buf.is_null();
                using_dummy = true;
                *redraw_for_dummy = true;
                buf = vgr_load_dummy_buf(
                    fname,
                    dirname_start.as_mut_ptr().cast(),
                    dirname_now.as_mut_ptr().cast(),
                );
            } else {
                using_dummy = false;
            }

            if !vgr_qflist_valid(
                wp,
                qi,
                save_qfid,
                cmd_args
                    .qf_title
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            ) {
                aborted = true;
                break;
            }

            save_qfid = qf_get_curlist(&mut *qi).qf_id;

            if buf.is_null() {
                if !g_st().got_int {
                    smsg!(0, gettext(c"Cannot open file \"%s\""), fname);
                }
            } else {
                let found_match = vgr_match_buflines(
                    qf_get_curlist(&mut *qi),
                    fname,
                    &mut *buf,
                    cmd_args.spat.as_deref(),
                    &mut cmd_args.regmatch,
                    &mut cmd_args.tomatch,
                    i32::from(duplicate_name),
                    cmd_args.flags,
                );

                if using_dummy {
                    if found_match && (*first_match_buf).is_null() {
                        *first_match_buf = buf;
                    }
                    if duplicate_name {
                        wipe_dummy_buffer(&mut *buf, dirname_start.as_mut_ptr().cast());
                        buf = ptr::null_mut();
                    } else if (g_st().cmdmod.cmod_flags & CMOD_HIDE) == 0
                        || crate::memory::cstr_first_byte((*buf).b_p_bh) == b'u'
                        || crate::memory::cstr_first_byte((*buf).b_p_bh) == b'w'
                        || crate::memory::cstr_first_byte((*buf).b_p_bh) == b'd'
                    {
                        if !found_match {
                            wipe_dummy_buffer(&mut *buf, dirname_start.as_mut_ptr().cast());
                            buf = ptr::null_mut();
                        } else if buf != *first_match_buf
                            || (cmd_args.flags & VGR_NOJUMP) != 0
                            || existing_swapfile(&*buf)
                        {
                            unload_dummy_buffer(buf, dirname_start.as_mut_ptr().cast());
                            (*buf).b_flags &= !BF_DUMMY;
                            buf = ptr::null_mut();
                        }
                    }

                    if !buf.is_null() {
                        let buf = &mut *buf;
                        buf.b_flags &= !BF_DUMMY;

                        if std::ptr::eq(buf, *first_match_buf)
                            && target_dir.is_none()
                            && (crate::memory::cstr_as_bytes(dirname_start.as_ptr().cast())
                                != crate::memory::cstr_as_bytes(dirname_now.as_ptr().cast()))
                        {
                            *target_dir = Some(crate::memory::cstr_to_cstring(
                                dirname_now.as_ptr().cast(),
                            ));
                        }

                        let mut aco: CtxSwitch = CtxSwitch::new();
                        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
                        apply_autocmds(EVENT_FILETYPE, buf.b_p_ft, buf.b_fname, true, buf);
                        do_modelines(OPT_NOWIN);
                        ctx_restore(&mut aco);
                    }
                }
            }
            fi += 1;
        }

        if !aborted {
            status = true;
        }

        status
    }
}

pub fn ex_vimgrep(eap: &mut Exarg) {
    if !check_can_set_curbuf_forceit(eap.forceit) {
        return;
    }

    let au_name = vgr_get_auname(eap.cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            au_name.as_ptr().cast_mut(),
            curbuf_mut().b_fname,
            true,
            g_st().curbuf,
        )
        && aborting()
    {
        return;
    }

    let mut wp: *mut WinInfoInfo = ptr::null_mut();
    let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);
    let mut target_dir: Option<std::ffi::CString> = None;
    let mut args = VgrArgs::new();
    if vgr_process_args(eap, &mut args) {
        if (eap.cmdidx != CMD_GREPADD
            && eap.cmdidx != CMD_LGREPADD
            && eap.cmdidx != CMD_VIMGREPADD
            && eap.cmdidx != CMD_LVIMGREPADD)
            || qf_stack_empty(unsafe { qi.as_ref() })
        {
            qf_new_list(
                unsafe { &mut *qi },
                args.qf_title.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            );
        }

        incr_quickfix_busy();

        let mut redraw_for_dummy = false;
        let mut first_match_buf: *mut FileBuf = ptr::null_mut();
        let status = vgr_process_files(
            wp,
            unsafe { &mut *qi },
            &mut args,
            &mut redraw_for_dummy,
            &raw mut first_match_buf,
            &mut target_dir,
        );

        if status {
            let qfl = qf_get_curlist(unsafe { &mut *qi });
            qfl.qf_nonevalid = false;
            qfl.qf_index = 1;
            qf_list_changed(qfl);

            qf_update_buffer(unsafe { &mut *qi }, 0);

            let save_qfid = qf_get_curlist(unsafe { &mut *qi }).qf_id;

            if let Some(au_name) = au_name {
                apply_autocmds(
                    EVENT_QUICKFIXCMDPOST,
                    au_name.as_ptr().cast_mut(),
                    curbuf_mut().b_fname,
                    true,
                    g_st().curbuf,
                );
            }

            if qflist_valid(wp, save_qfid) && qf_restore_list(unsafe { &mut *qi }, save_qfid) {
                if !qf_list_empty(Some(&*(qf_get_curlist(unsafe { &mut *qi })))) {
                    if (args.flags & VGR_NOJUMP) == 0 {
                        vgr_jump_to_match(
                            unsafe { &mut *qi },
                            eap.forceit,
                            &mut redraw_for_dummy,
                            first_match_buf,
                            target_dir
                                .as_ref()
                                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
                        );
                    }
                } else {
                    semsg!(gettext(E_NOMATCH2), args.spat_ptr());
                }

                decr_quickfix_busy();

                if redraw_for_dummy {
                    fold_update_all(curwin_mut());
                }
            } else {
                decr_quickfix_busy();
            }
        } else {
            decr_quickfix_busy();
        }
    }

    drop(target_dir);
    vim_regfree(args.regmatch.regprog);
}

fn restore_start_dir(dirname_start: impl crate::strings::IntoCCharPtr) {
    let dirname_start = dirname_start.into_c_char_ptr().cast_mut();
    {
        let mut dirname_now = vec![0u8; MAXPATHL];

        os_dirname(dirname_now.as_mut_ptr(), MAXPATHL);
        if crate::memory::cstr_as_bytes(dirname_start)
            != crate::memory::cstr_as_bytes(dirname_now.as_ptr().cast())
        {
            let mut ea: Exarg = Exarg::new();
            ea.arg = dirname_start;
            ea.cmdidx = if curwin_mut().w_localdir.is_none() {
                CMD_CD
            } else {
                CMD_LCD
            };
            ex_cd(&mut ea);
        }
    }
}

fn load_dummy_buffer(
    fname: impl crate::strings::IntoCCharPtr,
    dirname_start: impl crate::strings::IntoCCharPtr,
    resulting_dir: impl crate::strings::IntoCCharPtr,
) -> *mut FileBuf {
    let fname = fname.into_c_char_ptr().cast_mut();
    let dirname_start = dirname_start.into_c_char_ptr().cast_mut();
    let resulting_dir = resulting_dir.into_c_char_ptr().cast_mut();
    {
        let mut newbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
        if newbuf.is_null() {
            return ptr::null_mut();
        }

        let mut failed = true;
        let mut newbufref: Bufref = Bufref::new();
        set_bufref(&mut newbufref, newbuf);

        buf_copy_options(unsafe { &mut *newbuf }, BCO_ENTER | BCO_NOHELP);

        if ml_open(unsafe { &mut *newbuf }) {
            unsafe {
                (*newbuf).b_locked += 1;
            }
            let mut aco: CtxSwitch = CtxSwitch::new();
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), newbuf, 0);

            setfname(curbuf_mut(), fname, ptr::null_mut(), false);
            check_need_swap(true);
            curbuf_mut().b_flags &= !BF_DUMMY;

            let mut newbuf_to_wipe: Bufref = Bufref::new();
            let readfile_result = readfile(
                fname,
                ptr::null_mut(),
                0,
                0,
                MAXLNUM,
                None,
                READ_NEW | READ_DUMMY,
                false,
            );
            unsafe {
                (*newbuf).b_locked -= 1;
            }
            if readfile_result == OK && !g_st().got_int && (curbuf_mut().b_flags & BF_NEW) == 0 {
                failed = false;
                if g_st().curbuf != newbuf {
                    set_bufref(&mut newbuf_to_wipe, newbuf);
                    newbuf = g_st().curbuf;
                }
            }

            ctx_restore(&mut aco);

            if !newbuf_to_wipe.br_buf.is_null() && bufref_valid(Some(&newbuf_to_wipe)) {
                block_autocmds();
                wipe_dummy_buffer(unsafe { &mut *newbuf_to_wipe.br_buf }, ptr::null_mut());
                unblock_autocmds();
            }

            unsafe {
                (*newbuf).b_flags |= BF_DUMMY;
            }
        }

        os_dirname(resulting_dir, MAXPATHL);
        restore_start_dir(dirname_start);

        if !bufref_valid(Some(&newbufref)) {
            return ptr::null_mut();
        }
        if failed {
            wipe_dummy_buffer(unsafe { &mut *newbuf }, dirname_start);
            return ptr::null_mut();
        }
        newbuf
    }
}

fn wipe_dummy_buffer(buf: &mut FileBuf, dirname_start: impl crate::strings::IntoCCharPtr) {
    let dirname_start = dirname_start.into_c_char_ptr().cast_mut();
    {
        let mut give_up = false;
        loop {
            if buf.b_nwindows <= 0 {
                break;
            }
            let mut did_one = false;

            if !firstwin_mut().w_next.is_null() {
                for wp in for_all_windows_ptr() {
                    if wp.w_buffer == buf {
                        if win_close(wp, false, false) {
                            did_one = true;
                        }
                        break;
                    }
                }
            }
            if !did_one {
                give_up = true;
                break;
            }
        }

        if !give_up && g_st().curbuf != buf && buf.b_nwindows == 0 {
            let mut cs: ExCleanup = ExCleanup::new();
            enter_cleanup(&mut cs);
            wipe_buffer(&raw mut *buf, true);
            leave_cleanup(&mut cs);

            if !dirname_start.is_null() {
                restore_start_dir(dirname_start);
            }
            return;
        }

        buf.b_flags &= !BF_DUMMY;
    }
}

fn unload_dummy_buffer(buf: *mut FileBuf, dirname_start: impl crate::strings::IntoCCharPtr) {
    let dirname_start = dirname_start.into_c_char_ptr().cast_mut();
    if g_st().curbuf == buf {
        return;
    }

    close_buffer(ptr::null_mut(), buf, DOBUF_UNLOAD, false, true, false);
    restore_start_dir(dirname_start);
}

fn get_qfline_items(qfp: &mut Qfline, list: &mut TvList) -> bool {
    let mut bufnum = qfp.qf_fnum;
    if bufnum != 0 && buflist_findnr(bufnum).is_null() {
        bufnum = 0;
    }

    let dict = tv_dict_alloc();
    tv_list_append_dict(&mut *list, dict);

    let buf = [qfp.qf_type, 0];
    let buflen: usize = if buf[0] == 0 { 0 } else { 1 };
    let empty = c"".as_ptr();
    if !tv_dict_add_nr(&mut *dict, c"bufnr", Varnumber::from(bufnum))
        || !tv_dict_add_nr(&mut *dict, c"lnum", Varnumber::from(qfp.qf_lnum))
        || !tv_dict_add_nr(&mut *dict, c"end_lnum", Varnumber::from(qfp.qf_end_lnum))
        || !tv_dict_add_nr(&mut *dict, c"col", Varnumber::from(qfp.qf_col))
        || !tv_dict_add_nr(&mut *dict, c"end_col", Varnumber::from(qfp.qf_end_col))
        || !tv_dict_add_nr(&mut *dict, c"vcol", Varnumber::from(qfp.qf_viscol))
        || !tv_dict_add_nr(&mut *dict, c"nr", Varnumber::from(qfp.qf_nr))
        || !tv_dict_add_str(
            &mut *dict,
            c"module",
            qfp.qf_module.as_ref().map_or(empty, |s| s.as_ptr()),
        )
        || !tv_dict_add_str(
            &mut *dict,
            c"pattern",
            qfp.qf_pattern.as_ref().map_or(empty, |s| s.as_ptr()),
        )
        || !tv_dict_add_str(&mut *dict, c"text", qfp.qf_text.as_ptr())
        || !tv_dict_add_str_len(&mut *dict, c"type", buf.as_ptr().cast(), buflen as i32)
        || (qfp.qf_user_data.v_type != VarType::VarUnknown
            && !tv_dict_add_tv(&mut *dict, c"user_data", 9, &mut qfp.qf_user_data))
        || !tv_dict_add_nr(&mut *dict, c"valid", Varnumber::from(qfp.qf_valid))
    {
        std::process::abort();
    }

    true
}

fn get_errorlist(
    qi_arg: Option<&mut QfInfo>,
    wp: Option<&WinInfoInfo>,
    mut qf_idx: i32,
    eidx: i32,
    list: &mut TvList,
) -> bool {
    let mut qi = qi_arg.map_or(ptr::null_mut(), |p| &raw mut *p);
    if qi.is_null() {
        qi = ql_info_ptr();
        if let Some(wp) = wp {
            qi = get_loc_list(wp);
        }
        if qi.is_null() {
            return false;
        }
    }

    if eidx < 0 {
        return true;
    }

    if qf_idx == INVALID_QFIDX {
        qf_idx = unsafe { (*qi).qf_curlist };
    }

    if qf_idx >= unsafe { (*qi).qf_listcount } {
        return false;
    }

    let qfl = qf_get_list(unsafe { &mut *qi }, qf_idx);
    if qf_list_empty(Some(&*qfl)) {
        return false;
    }

    for (idx, qfp) in qfl.qf_items.iter_mut().enumerate() {
        if g_st().got_int {
            break;
        }
        let i = (idx + 1) as i32;
        if eidx > 0 {
            if eidx == i {
                return get_qfline_items(qfp, list);
            }
        } else if !get_qfline_items(qfp, list) {
            return false;
        }
    }

    true
}

fn qf_get_list_from_lines(what: &mut TvDict, di: &mut Dictitem, retdict: &mut TvDict) -> bool {
    if di.di_tv.v_type != VarType::VarList || di.di_tv.vval.v_list().is_null() {
        return false;
    }

    let mut errorformat = opt_vars().p_efm;
    let efm_di = tv_dict_find(Some(what), c"efm", 3);
    if !efm_di.is_null() {
        let efm_di = unsafe { &mut *efm_di };
        if efm_di.di_tv.v_type != VarType::VarString || efm_di.di_tv.vval.v_string().is_null() {
            return false;
        }
        errorformat = efm_di.di_tv.vval.v_string();
    }

    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    let mut qi = QfInfo {
        qf_refcount: 1,
        qf_listcount: 0,
        qf_curlist: 0,
        qf_maxcount: 1,
        qf_lists: vec![QfList::default()],
        qfl_type: QFLT_INTERNAL,
        qf_bufnr: INVALID_QFBUFNR,
    };

    if qf_init_ext(
        &mut qi,
        0,
        ptr::null(),
        ptr::null_mut(),
        Some(&mut di.di_tv),
        errorformat,
        true,
        0,
        0,
        ptr::null(),
        ptr::null_mut(),
    ) > 0
    {
        get_errorlist(Some(&mut qi), None, 0, 0, &mut *l);
    }

    tv_dict_add_list(retdict, c"items", l);
    true
}

fn qf_winid(qi: Option<&QfInfo>) -> i32 {
    if qi.is_none() {
        return 0;
    }
    let win = qf_find_win(qi.unwrap());
    if !win.is_null() {
        return unsafe { (*win).handle };
    }
    0
}

fn qf_getprop_qfbufnr(qi: Option<&QfInfo>, retdict: &mut TvDict) -> bool {
    let mut bufnum: i32 = 0;
    if let Some(qi) = qi
        && !buflist_findnr(qi.qf_bufnr).is_null()
    {
        bufnum = qi.qf_bufnr;
    }
    tv_dict_add_nr(retdict, c"qfbufnr", Varnumber::from(bufnum))
}

fn qf_getprop_keys2flags(what: &TvDict, loclist: bool) -> i32 {
    let mut flags = QF_GETLIST_NONE;

    if !tv_dict_find(Some(what), c"all", 3).is_null() {
        flags |= QF_GETLIST_ALL;
        if !loclist {
            flags &= !QF_GETLIST_FILEWINID;
        }
    }
    if !tv_dict_find(Some(what), c"title", 5).is_null() {
        flags |= QF_GETLIST_TITLE;
    }
    if !tv_dict_find(Some(what), c"nr", 2).is_null() {
        flags |= QF_GETLIST_NR;
    }
    if !tv_dict_find(Some(what), c"winid", 5).is_null() {
        flags |= QF_GETLIST_WINID;
    }
    if !tv_dict_find(Some(what), c"context", 7).is_null() {
        flags |= QF_GETLIST_CONTEXT;
    }
    if !tv_dict_find(Some(what), c"id", 2).is_null() {
        flags |= QF_GETLIST_ID;
    }
    if !tv_dict_find(Some(what), c"items", 5).is_null() {
        flags |= QF_GETLIST_ITEMS;
    }
    if !tv_dict_find(Some(what), c"idx", 3).is_null() {
        flags |= QF_GETLIST_IDX;
    }
    if !tv_dict_find(Some(what), c"size", 4).is_null() {
        flags |= QF_GETLIST_SIZE;
    }
    if !tv_dict_find(Some(what), c"changedtick", 11).is_null() {
        flags |= QF_GETLIST_TICK;
    }
    if loclist && !tv_dict_find(Some(what), c"filewinid", 9).is_null() {
        flags |= QF_GETLIST_FILEWINID;
    }
    if !tv_dict_find(Some(what), c"qfbufnr", 7).is_null() {
        flags |= QF_GETLIST_QFBUFNR;
    }
    if !tv_dict_find(Some(what), c"quickfixtextfunc", 16).is_null() {
        flags |= QF_GETLIST_QFTF;
    }

    flags
}

fn qf_getprop_qfidx(qi: &mut QfInfo, what: &mut TvDict) -> i32 {
    let mut qf_idx = qi.qf_curlist;
    let mut di = tv_dict_find(Some(what), c"nr", 2);
    if !di.is_null() {
        let di = unsafe { &mut *di };
        if di.di_tv.v_type == VarType::VarNumber {
            if di.di_tv.vval.v_number() != 0 {
                qf_idx = di.di_tv.vval.v_number() as i32 - 1;
                if qf_idx < 0 || qf_idx >= qi.qf_listcount {
                    qf_idx = INVALID_QFIDX;
                }
            }
        } else if di.di_tv.v_type == VarType::VarString
            && (crate::memory::cstr_as_opt_bytes(di.di_tv.vval.v_string()) == Some(b"$"))
        {
            qf_idx = qi.qf_listcount - 1;
        } else {
            qf_idx = INVALID_QFIDX;
        }
    }

    di = tv_dict_find(Some(what), c"id", 2);
    if !di.is_null() {
        let di = unsafe { &mut *di };
        if di.di_tv.v_type == VarType::VarNumber {
            if di.di_tv.vval.v_number() != 0 {
                qf_idx = qf_id2nr(qi, di.di_tv.vval.v_number() as u32);
            }
        } else {
            qf_idx = INVALID_QFIDX;
        }
    }

    qf_idx
}

fn qf_getprop_defaults(qi: *mut QfInfo, flags: i32, locstack: bool, retdict: &mut TvDict) -> bool {
    let mut status = true;
    let empty = c"".as_ptr();

    if (flags & QF_GETLIST_TITLE) != 0 {
        status = tv_dict_add_str_len(retdict, c"title", empty, 0);
    }
    if status && (flags & QF_GETLIST_ITEMS) != 0 {
        let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
        status = tv_dict_add_list(retdict, c"items", l);
    }
    if status && (flags & QF_GETLIST_NR) != 0 {
        status = tv_dict_add_nr(retdict, c"nr", 0);
    }
    if status && (flags & QF_GETLIST_WINID) != 0 {
        status = tv_dict_add_nr(
            retdict,
            c"winid",
            Varnumber::from(qf_winid(unsafe { (qi).as_ref() })),
        );
    }
    if status && (flags & QF_GETLIST_CONTEXT) != 0 {
        status = tv_dict_add_str_len(retdict, c"context", empty, 0);
    }
    if status && (flags & QF_GETLIST_ID) != 0 {
        status = tv_dict_add_nr(retdict, c"id", 0);
    }
    if status && (flags & QF_GETLIST_IDX) != 0 {
        status = tv_dict_add_nr(retdict, c"idx", 0);
    }
    if status && (flags & QF_GETLIST_SIZE) != 0 {
        status = tv_dict_add_nr(retdict, c"size", 0);
    }
    if status && (flags & QF_GETLIST_TICK) != 0 {
        status = tv_dict_add_nr(retdict, c"changedtick", 0);
    }
    if status && locstack && (flags & QF_GETLIST_FILEWINID) != 0 {
        status = tv_dict_add_nr(retdict, c"filewinid", 0);
    }
    if status && (flags & QF_GETLIST_QFBUFNR) != 0 {
        status = qf_getprop_qfbufnr(unsafe { (qi).as_ref() }, &mut *retdict);
    }
    if status && (flags & QF_GETLIST_QFTF) != 0 {
        status = tv_dict_add_str_len(retdict, c"quickfixtextfunc", empty, 0);
    }

    status
}

fn qf_getprop_title(qfl: &mut QfList, retdict: &mut TvDict) -> bool {
    tv_dict_add_str(
        retdict,
        c"title",
        qfl.qf_title.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
    )
}

fn qf_getprop_filewinid(wp: Option<&WinInfoInfo>, qi: &QfInfo, retdict: &mut TvDict) -> bool {
    let mut winid: ObjHandle = 0;

    if let Some(wp) = wp
        && is_ll_window(wp)
    {
        let ll_wp = qf_find_win_with_loclist(qi);
        if !ll_wp.is_null() {
            winid = unsafe { (*ll_wp).handle };
        }
    }

    tv_dict_add_nr(retdict, c"filewinid", Varnumber::from(winid))
}

fn qf_getprop_items(qi: &mut QfInfo, qf_idx: i32, eidx: i32, retdict: &mut TvDict) -> bool {
    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    get_errorlist(Some(&mut *qi), None, qf_idx, eidx, &mut *l);
    tv_dict_add_list(retdict, c"items", l);
    true
}

fn qf_getprop_ctx(qfl: &mut QfList, retdict: &mut TvDict) -> bool {
    if let Some(ref ctx) = qfl.qf_ctx {
        let di = tv_dict_item_alloc_len(c"context", 7);
        tv_copy(ctx, &mut di.di_tv);
        let status = tv_dict_add(retdict, di);
        if !status {
            tv_dict_item_free(di);
        }
        status
    } else {
        tv_dict_add_str_len(retdict, c"context", c"", 0)
    }
}

fn qf_getprop_idx(qfl: &mut QfList, mut eidx: i32, retdict: &mut TvDict) -> bool {
    if eidx == 0 {
        eidx = qfl.qf_index;
        if qf_list_empty(Some(&*qfl)) {
            eidx = 0;
        }
    }
    tv_dict_add_nr(retdict, c"idx", Varnumber::from(eidx))
}

fn qf_getprop_qftf(qfl: &mut QfList, retdict: &mut TvDict) -> bool {
    if qfl.qf_qftf_cb.cb_type() != K_CALLBACK_NONE_CONST {
        let mut tv: TypVal = TypVal::new();
        callback_put(&mut qfl.qf_qftf_cb, &mut tv);
        let status = tv_dict_add_tv(retdict, c"quickfixtextfunc", 16, &mut tv);
        tv_clear(&mut tv);
        status
    } else {
        tv_dict_add_str_len(retdict, c"quickfixtextfunc", c"", 0)
    }
}

fn qf_get_properties(wp: Option<&WinInfoInfo>, what: &mut TvDict, retdict: &mut TvDict) -> bool {
    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());
    let mut status = true;
    let mut qf_idx = INVALID_QFIDX;

    let di = tv_dict_find(Some(what), c"lines", 5);
    if !di.is_null() {
        return qf_get_list_from_lines(what, unsafe { &mut *di }, retdict);
    }

    if let Some(wp) = wp {
        qi = get_loc_list(wp);
    }

    let flags = qf_getprop_keys2flags(what, wp.is_some());

    if !qf_stack_empty(unsafe { qi.as_ref() }) {
        qf_idx = qf_getprop_qfidx(unsafe { &mut *qi }, what);
    }

    if qf_stack_empty(unsafe { qi.as_ref() }) || qf_idx == INVALID_QFIDX {
        return qf_getprop_defaults(qi, flags, wp.is_some(), retdict);
    }

    let mut eidx: i32 = 0;

    let di = tv_dict_find(Some(what), c"idx", 3);
    if !di.is_null() {
        let di = unsafe { &mut *di };
        if di.di_tv.v_type != VarType::VarNumber {
            return false;
        }
        eidx = di.di_tv.vval.v_number() as i32;
    }

    if (flags & QF_GETLIST_TITLE) != 0 {
        status = qf_getprop_title(qf_get_list(unsafe { &mut *qi }, qf_idx), retdict);
    }
    if status && (flags & QF_GETLIST_NR) != 0 {
        status = tv_dict_add_nr(retdict, c"nr", Varnumber::from(qf_idx + 1));
    }
    if status && (flags & QF_GETLIST_WINID) != 0 {
        status = tv_dict_add_nr(
            retdict,
            c"winid",
            Varnumber::from(qf_winid(unsafe { qi.as_ref() })),
        );
    }
    if status && (flags & QF_GETLIST_ITEMS) != 0 {
        status = qf_getprop_items(unsafe { &mut *qi }, qf_idx, eidx, retdict);
    }
    if status && (flags & QF_GETLIST_CONTEXT) != 0 {
        status = qf_getprop_ctx(qf_get_list(unsafe { &mut *qi }, qf_idx), retdict);
    }
    if status && (flags & QF_GETLIST_ID) != 0 {
        let qf_id = qf_get_list(unsafe { &mut *qi }, qf_idx).qf_id;
        status = tv_dict_add_nr(retdict, c"id", Varnumber::from(qf_id));
    }
    if status && (flags & QF_GETLIST_IDX) != 0 {
        status = qf_getprop_idx(qf_get_list(unsafe { &mut *qi }, qf_idx), eidx, retdict);
    }
    if status && (flags & QF_GETLIST_SIZE) != 0 {
        let count = qf_get_list(unsafe { &mut *qi }, qf_idx).count();
        status = tv_dict_add_nr(retdict, c"size", Varnumber::from(count));
    }
    if status && (flags & QF_GETLIST_TICK) != 0 {
        let tick = qf_get_list(unsafe { &mut *qi }, qf_idx).qf_changedtick;
        status = tv_dict_add_nr(retdict, c"changedtick", Varnumber::from(tick));
    }
    if status && wp.is_some() && (flags & QF_GETLIST_FILEWINID) != 0 {
        status = qf_getprop_filewinid(wp, unsafe { &*qi }, retdict);
    }
    if status && (flags & QF_GETLIST_QFBUFNR) != 0 {
        status = qf_getprop_qfbufnr(unsafe { qi.as_ref() }, retdict);
    }
    if status && (flags & QF_GETLIST_QFTF) != 0 {
        status = qf_getprop_qftf(qf_get_list(unsafe { &mut *qi }, qf_idx), retdict);
    }

    status
}

fn qf_setprop_qftf(qfl: &mut QfList, di: &mut Dictitem) -> bool {
    if check_secure() {
        return false;
    }

    let mut cb: Callback = Callback::new();
    callback_free(&mut qfl.qf_qftf_cb);
    if callback_from_typval(&mut cb, &di.di_tv) {
        qfl.qf_qftf_cb = cb;
    }
    true
}

fn qf_add_entry_from_dict(
    qfl: &mut QfList,
    d: &mut TvDict,
    first_entry: bool,
    valid_entry: &mut bool,
) -> i32 {
    if first_entry {
        qf_st().did_bufnr_emsg = false;
    }

    let filename = crate::eval::typval::tv_dict_get_cstring(Some(d), c"filename");
    let module = crate::eval::typval::tv_dict_get_cstring(Some(d), c"module");
    let mut bufnum = tv_dict_get_number(Some(d), c"bufnr") as i32;
    let lnum = tv_dict_get_number(Some(d), c"lnum") as Linenr;
    let end_lnum = tv_dict_get_number(Some(d), c"end_lnum") as Linenr;
    let col = tv_dict_get_number(Some(d), c"col") as i32;
    let end_col = tv_dict_get_number(Some(d), c"end_col") as i32;
    let vcol = tv_dict_get_number(Some(d), c"vcol") as u8;
    let nr = tv_dict_get_number(Some(d), c"nr") as i32;
    let type_ = tv_dict_get_string(Some(d), c"type");
    let pattern = crate::eval::typval::tv_dict_get_cstring(Some(d), c"pattern");
    let text = crate::eval::typval::tv_dict_get_cstring(Some(d), c"text").unwrap_or_default();
    let mut user_data: TypVal = TypVal::new();
    user_data.v_type = VarType::VarUnknown;
    tv_dict_get_tv(Some(d), c"user_data", &mut user_data);

    let mut valid = true;
    if (filename.is_none() && bufnum == 0) || (lnum == 0 && pattern.is_none()) {
        valid = false;
    }

    if bufnum != 0 && buflist_findnr(bufnum).is_null() {
        if !qf_st().did_bufnr_emsg {
            qf_st().did_bufnr_emsg = true;
            semsg!(gettext(E_BUFFER_NR_NOT_FOUND), bufnum);
        }
        valid = false;
        bufnum = 0;
    }

    if !tv_dict_find(Some(d), c"valid", -1).is_null() {
        valid = tv_dict_get_bool(Some(d), c"valid", 0) != 0;
    }

    let status = qf_add_entry(
        qfl,
        ptr::null_mut(),
        filename
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
        module
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
        bufnum,
        text.as_ptr().cast_mut(),
        lnum,
        end_lnum,
        col,
        end_col,
        vcol,
        pattern
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
        nr,
        if type_.is_null() {
            0
        } else {
            unsafe { *type_.cast::<u8>() }
        },
        Some(&user_data),
        u8::from(valid),
    );

    drop(filename);
    drop(module);
    drop(pattern);
    drop(text);
    tv_clear(&mut user_data);

    if valid {
        *valid_entry = true;
    }

    status
}

fn entry_is_closer_to_target(
    entry: &Qfline,
    other_entry: &Qfline,
    target_fnum: i32,
    target_lnum: i32,
    target_col: i32,
) -> bool {
    if target_fnum == 0 {
        return false;
    }

    let is_target_file = entry.qf_fnum != 0 && entry.qf_fnum == target_fnum;
    let other_is_target_file = other_entry.qf_fnum != 0 && other_entry.qf_fnum == target_fnum;
    if !is_target_file && other_is_target_file {
        return false;
    } else if is_target_file && !other_is_target_file {
        return true;
    }

    if target_lnum == 0 {
        return false;
    }

    let line_distance = if entry.qf_lnum != 0 {
        (entry.qf_lnum - target_lnum).abs()
    } else {
        i32::MAX
    };
    let other_line_distance = if other_entry.qf_lnum != 0 {
        (other_entry.qf_lnum - target_lnum).abs()
    } else {
        i32::MAX
    };
    if line_distance > other_line_distance {
        return false;
    } else if line_distance < other_line_distance {
        return true;
    }

    if target_col == 0 {
        return false;
    }

    let column_distance = if entry.qf_col != 0 {
        (entry.qf_col - target_col).abs()
    } else {
        i32::MAX
    };
    let other_column_distance = if other_entry.qf_col != 0 {
        (other_entry.qf_col - target_col).abs()
    } else {
        i32::MAX
    };
    if column_distance > other_column_distance {
        return false;
    } else if column_distance < other_column_distance {
        return true;
    }

    false
}

fn qf_add_entries(
    qi: &mut QfInfo,
    mut qf_idx: i32,
    list: Option<&TvList>,
    title: impl crate::strings::IntoCCharPtr,
    action: i32,
) -> i32 {
    let title = title.into_c_char_ptr();
    {
        let mut qfl = qf_get_list(&mut *qi, qf_idx);
        let mut retval = OK;
        let mut valid_entry = false;

        let mut old_count: usize = 0;

        let mut prev_fnum: i32 = 0;
        let mut prev_lnum: i32 = 0;
        let mut prev_col: i32 = 0;
        let cur_entry = qfl.cur_entry();
        if !cur_entry.is_null() {
            let cur_entry = unsafe { &mut *cur_entry };
            prev_fnum = cur_entry.qf_fnum;
            prev_lnum = cur_entry.qf_lnum;
            prev_col = cur_entry.qf_col;
        }

        let mut select_first_entry = false;
        let mut select_nearest_entry = false;

        if action == b' ' as i32 || qf_idx == qi.qf_listcount {
            select_first_entry = true;
            qf_new_list(qi, title);
            qf_idx = qi.qf_curlist;
            qfl = qf_get_list(&mut *qi, qf_idx);
        } else if action == b'a' as i32 {
            if qf_list_empty(Some(&*qfl)) {
                select_first_entry = true;
            } else {
                old_count = qfl.qf_items.len();
            }
        } else if action == b'r' as i32 {
            select_first_entry = true;
            qf_free_items(qfl);
            qf_store_title(qfl, title);
        } else if action == b'u' as i32 {
            select_nearest_entry = true;
            qf_free_items(qfl);
            qf_store_title(qfl, title);
        }

        let mut entry_to_select_index: i32 = 0;

        let first_li = tv_list_first(list);
        for li in crate::eval::typval::list_iter_ptr(first_li) {
            if li.li_tv.v_type == VarType::VarDict {
                let d = li.li_tv.vval.v_dict();
                if !d.is_null() {
                    retval = qf_add_entry_from_dict(
                        qfl,
                        unsafe { &mut *d },
                        std::ptr::eq(li, first_li),
                        &mut valid_entry,
                    );
                    if retval == QF_FAIL {
                        break;
                    }

                    let entry = qfl.qf_items.last().unwrap();
                    if (select_first_entry && entry_to_select_index == 0)
                        || (select_nearest_entry
                            && (entry_to_select_index == 0
                                || entry_is_closer_to_target(
                                    entry,
                                    &(&qfl.qf_items)[(entry_to_select_index - 1) as usize],
                                    prev_fnum,
                                    prev_lnum,
                                    prev_col,
                                )))
                    {
                        entry_to_select_index = qfl.count();
                    }
                }
            }
        }

        if valid_entry {
            qfl.qf_nonevalid = false;
        } else if qfl.qf_index == 0 {
            qfl.qf_nonevalid = true;
        }

        if entry_to_select_index > 0 {
            qfl.qf_index = entry_to_select_index;
        }

        qf_update_buffer(qi, old_count);

        retval
    }
}

fn qf_setprop_get_qfidx(qi: &QfInfo, what: &TvDict, action: i32, newlist: &mut bool) -> i32 {
    let mut qf_idx = qi.qf_curlist;

    let di = tv_dict_find(Some(what), c"nr", 2);
    if !di.is_null() {
        let di = unsafe { &mut *di };
        if di.di_tv.v_type == VarType::VarNumber {
            if di.di_tv.vval.v_number() != 0 {
                qf_idx = di.di_tv.vval.v_number() as i32 - 1;
            }

            if (action == b' ' as i32 || action == b'a' as i32) && qf_idx == qi.qf_listcount {
                *newlist = true;
                qf_idx = if qf_stack_empty(Some(qi)) {
                    0
                } else {
                    qi.qf_listcount - 1
                };
            } else if qf_idx < 0 || qf_idx >= qi.qf_listcount {
                return INVALID_QFIDX;
            } else if action != b' ' as i32 {
                *newlist = false;
            }
        } else if di.di_tv.v_type == VarType::VarString
            && (crate::memory::cstr_as_opt_bytes(di.di_tv.vval.v_string()) == Some(b"$"))
        {
            if !qf_stack_empty(Some(qi)) {
                qf_idx = qi.qf_listcount - 1;
            } else if *newlist {
                qf_idx = 0;
            } else {
                return INVALID_QFIDX;
            }
        } else {
            return INVALID_QFIDX;
        }
    }

    if !*newlist {
        let di = tv_dict_find(Some(what), c"id", 2);
        if !di.is_null() {
            let di = unsafe { &mut *di };
            if di.di_tv.v_type != VarType::VarNumber {
                return INVALID_QFIDX;
            }
            return qf_id2nr(qi, di.di_tv.vval.v_number() as u32);
        }
    }

    qf_idx
}

fn qf_setprop_title(qi: &mut QfInfo, qf_idx: i32, what: &TvDict, di: &Dictitem) -> bool {
    let qfl = qf_get_list(&mut *qi, qf_idx);
    if di.di_tv.v_type != VarType::VarString {
        return false;
    }

    qfl.qf_title = crate::eval::typval::tv_dict_get_cstring(Some(what), c"title");
    if qf_idx == qi.qf_curlist {
        qf_update_win_titlevar(qi);
    }

    true
}

fn qf_setprop_items(qi: &mut QfInfo, qf_idx: i32, di: &mut Dictitem, action: i32) -> bool {
    if di.di_tv.v_type != VarType::VarList {
        return false;
    }

    let title_save = (&qi.qf_lists)[qf_idx as usize].qf_title.clone();
    qf_add_entries(
        qi,
        qf_idx,
        unsafe { di.di_tv.vval.v_list().as_ref() },
        title_save
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
        if action == b' ' as i32 {
            b'a' as i32
        } else {
            action
        },
    ) == QF_OK
}

fn qf_setprop_items_from_lines(
    qi: &mut QfInfo,
    qf_idx: i32,
    what: &TvDict,
    di: &mut Dictitem,
    action: i32,
) -> bool {
    let mut errorformat = opt_vars().p_efm;
    let mut retval = false;

    let efm_di = tv_dict_find(Some(what), c"efm", 3);
    if !efm_di.is_null() {
        let efm_di = unsafe { &mut *efm_di };
        if efm_di.di_tv.v_type != VarType::VarString || efm_di.di_tv.vval.v_string().is_null() {
            return false;
        }
        errorformat = efm_di.di_tv.vval.v_string();
    }

    if di.di_tv.v_type != VarType::VarList || di.di_tv.vval.v_list().is_null() {
        return false;
    }

    if action == b'r' as i32 || action == b'u' as i32 {
        qf_free_items(qf_get_list(&mut *qi, qf_idx));
    }
    if qf_init_ext(
        qi,
        qf_idx,
        ptr::null(),
        ptr::null_mut(),
        Some(&mut di.di_tv),
        errorformat,
        false,
        0,
        0,
        ptr::null(),
        ptr::null_mut(),
    ) >= 0
    {
        retval = true;
    }

    retval
}

fn qf_setprop_context(qfl: &mut QfList, di: &mut Dictitem) -> bool {
    if let Some(ref mut ctx) = qfl.qf_ctx {
        tv_clear(ctx);
    }
    let mut ctx = tv_initial_value();
    tv_copy(&di.di_tv, &mut ctx);
    qfl.qf_ctx = Some(ctx);

    true
}

fn qf_setprop_curidx(qi: &mut QfInfo, qfl: &mut QfList, di: &Dictitem) -> bool {
    let mut newidx: i32;

    if di.di_tv.v_type == VarType::VarString
        && !di.di_tv.vval.v_string().is_null()
        && (crate::memory::cstr_as_bytes(di.di_tv.vval.v_string()) == b"$")
    {
        newidx = qfl.count();
    } else {
        let mut denote = false;
        newidx = tv_get_number_chk(&di.di_tv, Some(&mut denote)) as i32;
        if denote {
            return false;
        }
    }

    if newidx < 1 {
        return false;
    }
    newidx = newidx.min(qfl.count());
    let old_qfidx = qfl.qf_index;
    let qf_ptr = get_nth_entry(qfl, newidx, &mut newidx);
    if qf_ptr.is_null() {
        return false;
    }
    qfl.qf_index = newidx;

    if (&qi.qf_lists)[qi.qf_curlist as usize].qf_id == qfl.qf_id {
        qf_win_pos_update(&mut *qi, old_qfidx);
    }
    true
}

fn qf_set_properties(
    qi: &mut QfInfo,
    what: &TvDict,
    action: i32,
    title: impl crate::strings::IntoCCharPtr,
) -> bool {
    let title = title.into_c_char_ptr();
    {
        let mut newlist = action == b' ' as i32 || qf_stack_empty(Some(&*qi));
        let mut qf_idx = qf_setprop_get_qfidx(qi, what, action, &mut newlist);
        if qf_idx == INVALID_QFIDX {
            return false;
        }

        if newlist {
            qi.qf_curlist = qf_idx;
            qf_new_list(qi, title);
            qf_idx = qi.qf_curlist;
        }

        let mut retval = false;
        let mut di = tv_dict_find(Some(what), c"title", 5);
        if !di.is_null() {
            retval = qf_setprop_title(qi, qf_idx, what, unsafe { &*di });
        }
        di = tv_dict_find(Some(what), c"items", 5);
        if !di.is_null() {
            retval = qf_setprop_items(qi, qf_idx, unsafe { &mut *di }, action);
        }
        di = tv_dict_find(Some(what), c"lines", 5);
        if !di.is_null() {
            retval = qf_setprop_items_from_lines(qi, qf_idx, what, unsafe { &mut *di }, action);
        }
        di = tv_dict_find(Some(what), c"context", 7);
        if !di.is_null() {
            retval = qf_setprop_context(qf_get_list(&mut *qi, qf_idx), unsafe { &mut *di });
        }
        di = tv_dict_find(Some(what), c"idx", 3);
        if !di.is_null() {
            let qfl = qf_get_list(&mut *qi, qf_idx);
            retval = qf_setprop_curidx(qi, qfl, unsafe { &*di });
        }
        di = tv_dict_find(Some(what), c"quickfixtextfunc", 16);
        if !di.is_null() {
            retval = qf_setprop_qftf(qf_get_list(&mut *qi, qf_idx), unsafe { &mut *di });
        }

        if newlist || retval {
            qf_list_changed(qf_get_list(&mut *qi, qf_idx));
        }
        if newlist {
            qf_update_buffer(qi, 0);
        }

        retval
    }
}

fn qf_free_stack(mut wp: *mut WinInfoInfo, qi: *mut QfInfo) {
    unsafe {
        let qfwin = qf_find_win(&*qi);

        if !qfwin.is_null() {
            if (*qi).qf_curlist < (*qi).qf_listcount {
                qf_free(qf_get_curlist(&mut *qi));
            }
            qf_update_buffer(&mut *qi, 0);
        }

        if !wp.is_null() && is_ll_window(&*wp) {
            let llwin = qf_find_win_with_loclist(&*qi);
            if !llwin.is_null() {
                wp = llwin;
            }
        }

        qf_free_all(wp.as_mut());
        if wp.is_null() {
            (*qi).qf_curlist = 0;
            (*qi).qf_listcount = 0;
        } else if !qfwin.is_null() {
            let new_ll = qf_alloc_stack(QFLT_LOCATION, (*wp).w_onebuf_opt.wo_lhi as i32);
            new_ll.qf_bufnr = (*qfwin).buf_mut().b_fnum;

            ll_free_all(&raw mut (*qfwin).w_llist_ref);

            (*qfwin).w_llist_ref = new_ll;
            if wp != qfwin {
                win_set_loclist(&mut *wp, &mut *new_ll);
            }
        }
    }
}

pub fn set_errorlist(
    wp: *mut WinInfoInfo,
    list: Option<&TvList>,
    action: i32,
    title: impl crate::strings::IntoCCharPtr,
    what: Option<&TvDict>,
) -> bool {
    let title = title.into_c_char_ptr();
    {
        let qi: *mut QfInfo = if !wp.is_null() {
            ll_get_or_alloc_list(unsafe { &mut *wp })
        } else {
            ql_info_ptr()
        };
        assert!(!qi.is_null());

        if action == b'f' as i32 {
            qf_free_stack(wp, qi);
            return true;
        }

        if list.is_some() && tv_list_len(list) != 0 && what.is_some() {
            semsg!(
                gettext(E_INVARG2),
                gettext(c"cannot have both a list and a \"what\" argument"),
            );
            return false;
        }

        incr_quickfix_busy();

        let retval = if let Some(what) = what {
            qf_set_properties(unsafe { &mut *qi }, what, action, title)
        } else {
            let r = qf_add_entries(
                unsafe { &mut *qi },
                unsafe { (*qi).qf_curlist },
                list,
                title,
                action,
            );
            if r == QF_OK {
                qf_list_changed(qf_get_curlist(unsafe { &mut *qi }));
            }
            r == QF_OK
        };

        decr_quickfix_busy();

        retval
    }
}

fn mark_quickfix_user_data(qi: &mut QfInfo, copy_id: i32) -> bool {
    let mut abort = false;
    let mut i: i32 = 0;
    while i < qi.qf_maxcount && !abort {
        let qfl = &mut (&mut qi.qf_lists)[i as usize];
        if qfl.qf_has_user_data {
            for qfp in &mut qfl.qf_items {
                if g_st().got_int {
                    break;
                }
                let user_data = &mut qfp.qf_user_data;
                if user_data.v_type != VarType::VarNumber
                    && user_data.v_type != VarType::VarString
                    && user_data.v_type != VarType::VarFloat
                {
                    abort = abort
                        || set_ref_in_item(user_data, copy_id, ptr::null_mut(), ptr::null_mut());
                }
            }
        }
        i += 1;
    }
    abort
}

fn mark_quickfix_ctx(qi: &mut QfInfo, copy_id: i32) -> bool {
    let mut abort = false;
    let mut i: i32 = 0;
    while i < qi.qf_maxcount && !abort {
        let qfl = &mut (&mut qi.qf_lists)[i as usize];
        if let Some(ref mut ctx) = qfl.qf_ctx
            && ctx.v_type != VarType::VarNumber
            && ctx.v_type != VarType::VarString
            && ctx.v_type != VarType::VarFloat
        {
            abort = set_ref_in_item(ctx, copy_id, ptr::null_mut(), ptr::null_mut());
        }

        let cb = &mut qfl.qf_qftf_cb;
        abort = abort || set_ref_in_callback(cb, copy_id, ptr::null_mut(), ptr::null_mut());
        i += 1;
    }
    abort
}

pub fn set_ref_in_quickfix(copy_id: i32) -> bool {
    if mark_quickfix_ctx(unsafe { &mut *ql_info_ptr() }, copy_id)
        || mark_quickfix_user_data(unsafe { &mut *ql_info_ptr() }, copy_id)
        || set_ref_in_callback(
            &mut opt_vars().p_qftf,
            copy_id,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    {
        return true;
    }

    for tp in for_all_tabpages_mut() {
        for win in win_iter_mut(tab_firstwin(tp)) {
            if !win.w_llist.is_null() {
                let ll = win.w_llist;
                if mark_quickfix_ctx(unsafe { &mut *ll }, copy_id)
                    || mark_quickfix_user_data(unsafe { &mut *ll }, copy_id)
                {
                    return true;
                }
            }

            if is_ll_window(win) {
                let ll_ref = win.w_llist_ref;
                if unsafe { (*ll_ref).qf_refcount } == 1
                    && (mark_quickfix_ctx(unsafe { &mut *ll_ref }, copy_id)
                        || mark_quickfix_user_data(unsafe { &mut *ll_ref }, copy_id))
                {
                    return true;
                }
            }
        }
    }

    false
}

fn cbuffer_get_auname(cmdidx: i32) -> Option<&'static CStr> {
    match cmdidx {
        CMD_CBUFFER => Some(c"cbuffer"),
        CMD_CGETBUFFER => Some(c"cgetbuffer"),
        CMD_CADDBUFFER => Some(c"caddbuffer"),
        CMD_LBUFFER => Some(c"lbuffer"),
        CMD_LGETBUFFER => Some(c"lgetbuffer"),
        CMD_LADDBUFFER => Some(c"laddbuffer"),
        _ => None,
    }
}

fn cbuffer_process_args(
    eap: &mut Exarg,
    bufp: *mut *mut FileBuf,
    line1: &mut Linenr,
    line2: &mut Linenr,
) -> bool {
    unsafe {
        let mut buf: *mut FileBuf = ptr::null_mut();

        if crate::memory::cstr_is_empty(eap.arg) {
            buf = g_st().curbuf;
        } else if crate::memory::cstr_is_empty(skipwhite(skipdigits(eap.arg))) {
            buf = buflist_findnr(
                crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(eap.arg)) as i32,
            );
        }

        if buf.is_null() {
            emsg(gettext(E_INVARG));
            return false;
        }

        if (*buf).b_ml.ml_mfp.is_none() {
            emsg(gettext(E_BUFFER_IS_NOT_LOADED));
            return false;
        }

        if eap.addr_count == 0 {
            eap.line1 = 1;
            eap.line2 = (*buf).b_ml.ml_line_count;
        }

        if eap.line1 < 1
            || eap.line1 > (*buf).b_ml.ml_line_count
            || eap.line2 < 1
            || eap.line2 > (*buf).b_ml.ml_line_count
        {
            emsg(gettext(E_INVRANGE));
            return false;
        }

        *line1 = eap.line1;
        *line2 = eap.line2;
        *bufp = buf;

        true
    }
}

pub fn ex_cbuffer(eap: &mut Exarg) {
    unsafe {
        let au_name = cbuffer_get_auname(eap.cmdidx);
        if let Some(au_name) = au_name
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.as_ptr().cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            )
            && aborting()
        {
            return;
        }

        let mut wp: *mut WinInfoInfo = ptr::null_mut();
        let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);

        let mut buf: *mut FileBuf = ptr::null_mut();
        let mut line1: Linenr = 0;
        let mut line2: Linenr = 0;
        if !cbuffer_process_args(eap, &raw mut buf, &mut line1, &mut line2) {
            return;
        }

        let qf_title_cstr = qf_cmdtitle(*eap.cmdlinep);
        let mut qf_title = qf_title_cstr.as_ptr().cast_mut();
        if !(*buf).b_sfname.is_null() {
            let iobuff_ptr = g_st().iobuff.as_mut_ptr();
            vim_snprintf!(iobuff_ptr, IOSIZE, c"%s (%s)", qf_title, (*buf).b_sfname,);
            qf_title = iobuff_ptr;
        }

        incr_quickfix_busy();

        let mut res = qf_init_ext(
            &mut *qi,
            (*qi).qf_curlist,
            ptr::null(),
            buf,
            None,
            opt_vars().p_efm,
            eap.cmdidx != CMD_CADDBUFFER && eap.cmdidx != CMD_LADDBUFFER,
            eap.line1,
            eap.line2,
            qf_title,
            ptr::null_mut(),
        );
        if qf_stack_empty(qi.as_ref()) {
            decr_quickfix_busy();
            return;
        }
        if res >= 0 {
            qf_list_changed(qf_get_curlist(&mut *qi));
        }
        let save_qfid = qf_get_curlist(&mut *qi).qf_id;
        if let Some(au_name) = au_name {
            let curbuf_old = g_st().curbuf;
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.as_ptr().cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            );
            if g_st().curbuf != curbuf_old {
                res = 0;
            }
        }
        if res > 0
            && (eap.cmdidx == CMD_CBUFFER || eap.cmdidx == CMD_LBUFFER)
            && qflist_valid(wp, save_qfid)
        {
            qf_jump_first(&mut *qi, save_qfid, eap.forceit);
        }

        decr_quickfix_busy();
    }
}

fn cexpr_get_auname(cmdidx: i32) -> Option<&'static CStr> {
    match cmdidx {
        CMD_CEXPR => Some(c"cexpr"),
        CMD_CGETEXPR => Some(c"cgetexpr"),
        CMD_CADDEXPR => Some(c"caddexpr"),
        CMD_LEXPR => Some(c"lexpr"),
        CMD_LGETEXPR => Some(c"lgetexpr"),
        CMD_LADDEXPR => Some(c"laddexpr"),
        _ => None,
    }
}

fn trigger_cexpr_autocmd(cmdidx: i32) -> bool {
    let au_name = cexpr_get_auname(cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            au_name.as_ptr().cast_mut(),
            curbuf_mut().b_fname,
            true,
            g_st().curbuf,
        )
        && aborting()
    {
        return false;
    }
    true
}

pub fn cexpr_core(eap: &Exarg, tv: &mut TypVal) -> bool {
    let mut wp: *mut WinInfoInfo = ptr::null_mut();
    let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);

    if (tv.v_type == VarType::VarString && !tv.vval.v_string().is_null())
        || tv.v_type == VarType::VarList
    {
        let au_name = cexpr_get_auname(eap.cmdidx);
        incr_quickfix_busy();
        let res = qf_init_ext(
            unsafe { &mut *qi },
            unsafe { (*qi).qf_curlist },
            ptr::null(),
            ptr::null_mut(),
            Some(&mut *tv),
            opt_vars().p_efm,
            eap.cmdidx != CMD_CADDEXPR && eap.cmdidx != CMD_LADDEXPR,
            0,
            0,
            &qf_cmdtitle(unsafe { *eap.cmdlinep }),
            ptr::null_mut(),
        );
        if qf_stack_empty(unsafe { qi.as_ref() }) {
            decr_quickfix_busy();
            return false;
        }
        if res >= 0 {
            qf_list_changed(qf_get_curlist(unsafe { &mut *qi }));
        }
        let save_qfid = qf_get_curlist(unsafe { &mut *qi }).qf_id;
        if let Some(au_name) = au_name {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.as_ptr().cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            );
        }
        if res > 0
            && (eap.cmdidx == CMD_CEXPR || eap.cmdidx == CMD_LEXPR)
            && qflist_valid(wp, save_qfid)
        {
            qf_jump_first(unsafe { &mut *qi }, save_qfid, eap.forceit);
        }
        decr_quickfix_busy();
        return true;
    } else {
        emsg(gettext(c"E777: String or List expected"));
    }
    false
}

pub fn ex_cexpr(eap: &mut Exarg) {
    if !trigger_cexpr_autocmd(eap.cmdidx) {
        return;
    }

    let Some(mut tv) = eval_expr(eap.arg, Some(&mut *eap)) else {
        return;
    };

    let _ = cexpr_core(eap, &mut tv);
    tv_clear(&mut tv);
}

fn hgr_get_ll(new_ll: &mut bool) -> *mut QfInfo {
    let wp = if bt_help(unsafe { curwin_mut().w_buffer.as_ref() }) {
        curwin_ptr()
    } else {
        qf_find_help_win()
    };

    let mut qi = if wp.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*wp).w_llist }
    };
    if qi.is_null() {
        qi = qf_alloc_stack(QFLT_LOCATION, 1);
        *new_ll = true;
    }

    qi
}

fn hgr_search_file(
    qfl: &mut QfList,
    fname: impl crate::strings::IntoCCharPtr,
    p_regmatch: &mut Regmatch,
) {
    let fname = fname.into_c_char_ptr().cast_mut();
    {
        let fd = os_fopen(fname, c"r").cast::<FILE>();
        if fd.is_null() {
            return;
        }

        let iobuff_ptr = g_st().iobuff.as_mut_ptr();
        let mut lnum: Linenr = 1;
        while !vim_fgets(iobuff_ptr, IOSIZE as i32, fd.cast()) && !g_st().got_int {
            let line = iobuff_ptr;

            if vim_regexec(p_regmatch, line, 0) {
                let mut l = crate::memory::cstr_len(line);
                while l > 0 && (crate::memory::cstr_byte_at(line, l - 1)) <= b' ' {
                    l -= 1;
                    unsafe {
                        *line.wrapping_add(l) = NUL;
                    }
                }

                if qf_add_entry(
                    qfl,
                    ptr::null_mut(),
                    fname,
                    ptr::null_mut(),
                    0,
                    line,
                    lnum,
                    0,
                    (crate::memory::ptr_offset_from(p_regmatch.startp[0], line) as i32) + 1,
                    (crate::memory::ptr_offset_from(p_regmatch.endp[0], line) as i32) + 1,
                    0,
                    ptr::null_mut(),
                    0,
                    1,
                    None,
                    1,
                ) == QF_FAIL
                {
                    g_st().got_int = true;
                    break;
                }
            }
            lnum += 1;
            line_breakcheck();
        }
        unsafe { fclose(fd) };
    }
}

fn hgr_search_files_in_dir(
    qfl: &mut QfList,
    dirname: impl crate::strings::IntoCCharPtr,
    p_regmatch: &mut Regmatch,
    lang: impl crate::strings::IntoCCharPtr,
) {
    let mut dirname = dirname.into_c_char_ptr().cast_mut();
    let lang = lang.into_c_char_ptr();
    {
        let mut fnames: Vec<std::ffi::CString> = Vec::new();

        add_pathsep(dirname);
        c_strcat(dirname, c"doc/*.\\(txt\\|??x\\)");
        if gen_expand_wildcards_vec(1, &raw mut dirname, &mut fnames, EW_FILE | EW_SILENT)
            && !fnames.is_empty()
        {
            for f_cstr in &fnames {
                if g_st().got_int {
                    break;
                }
                let f = f_cstr.as_ptr().cast_mut();
                let flen = f_cstr.as_bytes().len();
                if !lang.is_null()
                    && flen >= 3
                    && !(crate::memory::cstr_prefix_bytes(lang, 2).eq_ignore_ascii_case(
                        crate::memory::cstr_prefix_bytes(f.wrapping_add(flen - 3), 2),
                    ))
                    && !(crate::memory::cstr_prefix_bytes(lang, 2).eq_ignore_ascii_case(b"en")
                        && b"txt".eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(
                            f.wrapping_add(flen - 3),
                            3,
                        )))
                {
                    continue;
                }

                hgr_search_file(qfl, f, p_regmatch);
            }
        }
    }
}

fn hgr_search_in_rtp(
    qfl: &mut QfList,
    p_regmatch: &mut Regmatch,
    lang: impl crate::strings::IntoCCharPtr,
) {
    let lang = lang.into_c_char_ptr();
    {
        let mut p = opt_vars().p_rtp;
        let name_buff_ptr = g_st().name_buff.as_mut_ptr();
        while !crate::memory::cstr_is_empty(p) && !g_st().got_int {
            copy_option_part(&raw mut p, name_buff_ptr, MAXPATHL, c",");
            hgr_search_files_in_dir(qfl, name_buff_ptr, p_regmatch, lang);
        }
    }
}

pub fn ex_helpgrep(eap: &mut Exarg) {
    let mut qi = ql_info_ptr();
    assert!(!qi.is_null());
    let au_name: Option<&'static CStr> = match eap.cmdidx {
        CMD_HELPGREP => Some(c"helpgrep"),
        CMD_LHELPGREP => Some(c"lhelpgrep"),
        _ => None,
    };
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            au_name.as_ptr().cast_mut(),
            curbuf_mut().b_fname,
            true,
            g_st().curbuf,
        )
        && aborting()
    {
        return;
    }

    let mut updated = false;
    let save_cpo = opt_vars().p_cpo;
    opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

    let mut new_qi = false;
    if is_loclist_cmd(eap.cmdidx) {
        qi = hgr_get_ll(&mut new_qi);
    }

    incr_quickfix_busy();

    let lang = if eap.arg.is_null() {
        ptr::null()
    } else {
        let len = crate::memory::cstr_len(eap.arg);
        let slice = unsafe { std::slice::from_raw_parts_mut(eap.arg.cast::<u8>(), len + 1) };
        check_help_lang(slice).map_or(ptr::null(), |s| s.as_ptr())
    };
    let mut regmatch: Regmatch = Regmatch::new();
    regmatch.regprog = vim_regcomp(eap.arg, RE_MAGIC + RE_STRING);
    regmatch.rm_ic = false;
    if !regmatch.regprog.is_null() {
        qf_new_list(unsafe { &mut *qi }, &qf_cmdtitle(unsafe { *eap.cmdlinep }));
        let qfl = qf_get_curlist(unsafe { &mut *qi });

        hgr_search_in_rtp(qfl, &mut regmatch, lang);

        vim_regfree(regmatch.regprog);

        qfl.qf_nonevalid = false;
        qfl.qf_index = 1;
        qf_list_changed(qfl);
        updated = true;
    }

    if opt_vars().p_cpo == EMPTY_STRING_OPTION.as_ptr().cast() {
        opt_vars().p_cpo = save_cpo;
    } else {
        if crate::memory::cstr_is_empty(opt_vars().p_cpo) {
            set_option_value_give_err(K_OPT_CPOPTIONS, cstr_as_obj(save_cpo), 0);
        }
        free_string_option(save_cpo);
    }

    if updated {
        qf_update_buffer(unsafe { &mut *qi }, 0);
    }

    if let Some(au_name) = au_name {
        apply_autocmds(
            EVENT_QUICKFIXCMDPOST,
            au_name.as_ptr().cast_mut(),
            curbuf_mut().b_fname,
            true,
            g_st().curbuf,
        );
        if !new_qi
            && is_ll_stack(unsafe { &*qi })
            && qf_find_win_with_loclist(unsafe { &*qi }).is_null()
        {
            decr_quickfix_busy();
            return;
        }
    }

    if !qf_list_empty(Some(&*(qf_get_curlist(unsafe { &mut *qi })))) {
        qf_jump(qi, 0, 0, 0);
    } else {
        semsg!(gettext(E_NOMATCH2), eap.arg);
    }

    decr_quickfix_busy();

    if eap.cmdidx == CMD_LHELPGREP {
        if !bt_help(unsafe { curwin_mut().w_buffer.as_ref() }) || curwin_mut().w_llist == qi {
            if new_qi {
                ll_free_all(&raw mut qi);
            }
        } else if curwin_mut().w_llist.is_null() && new_qi {
            curwin_mut().w_llist = qi;
        }
    }
}

pub fn free_quickfix() {
    qf_free_all(None);
    for tab in for_all_tabpages_mut() {
        for win in win_iter_ptr(tab_firstwin(tab)) {
            qf_free_all(Some(&mut *win));
        }
    }

    callback_free(&mut opt_vars().p_qftf);
    qf_st().qfga = Vec::new();
}

fn get_qf_loc_list(
    is_qf: bool,
    wp: Option<&WinInfoInfo>,
    what_arg: &mut TypVal,
    rettv: &mut TypVal,
) {
    if what_arg.v_type == VarType::VarUnknown {
        tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_MAY_KNOW_CONST);
        if is_qf || wp.is_some() {
            get_errorlist(None, wp, -1, 0, rettv.vval.v_list_ref());
        }
    } else {
        tv_dict_alloc_ret(rettv);
        if is_qf || wp.is_some() {
            if what_arg.v_type == VarType::VarDict {
                let d = what_arg.vval.v_dict();
                if !d.is_null() {
                    unsafe {
                        qf_get_properties(wp, &mut *d, rettv.vval.v_dict_ref());
                    }
                }
            } else {
                emsg(gettext(E_DICTREQ));
            }
        }
    }
}

pub fn f_getloclist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let wp = find_win_by_nr_or_id(&argvars[0]);
    get_qf_loc_list(false, unsafe { wp.as_ref() }, &mut argvars[1], rettv);
}

pub fn f_getqflist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_qf_loc_list(true, None, &mut argvars[0], rettv);
}

fn set_qf_ll_list(wp: *mut WinInfoInfo, args: &mut [TypVal], rettv: &mut TypVal) {
    let args: *mut TypVal = args.as_mut_ptr();
    unsafe {
        let mut title = ptr::null();
        let mut action: u8 = b' ';
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        let mut what: *mut TvDict = ptr::null_mut();

        let list_arg = args;
        if (*list_arg).v_type != VarType::VarList {
            emsg(gettext(E_LISTREQ));
            return;
        } else if qf_st().set_qf_ll_recursive != 0 {
            emsg(gettext(E_AU_RECURSIVE));
            return;
        }

        let action_arg = args.wrapping_add(1);
        if (*action_arg).v_type != VarType::VarUnknown {
            if (*action_arg).v_type != VarType::VarString {
                emsg(gettext(E_STRING_REQUIRED));
                return;
            }
            let act = tv_get_string_chk(&*action_arg);
            if (crate::memory::cstr_first_byte(act) == b'a'
                || crate::memory::cstr_first_byte(act) == b'r'
                || crate::memory::cstr_first_byte(act) == b'u'
                || crate::memory::cstr_first_byte(act) == b' '
                || crate::memory::cstr_first_byte(act) == b'f')
                && crate::memory::cstr_byte_at(act, 1) == 0
            {
                action = *act.cast::<u8>();
            } else {
                semsg!(gettext(E_INVACT), act);
                return;
            }

            let what_arg = args.wrapping_add(2);
            let what_arg = &mut *what_arg;
            if what_arg.v_type != VarType::VarUnknown {
                if what_arg.v_type == VarType::VarString {
                    title = tv_get_string_chk(&*what_arg);
                    if title.is_null() {
                        return;
                    }
                } else if what_arg.v_type == VarType::VarDict && !what_arg.vval.v_dict().is_null() {
                    what = what_arg.vval.v_dict();
                } else {
                    emsg(gettext(E_DICTREQ));
                    return;
                }
            }
        }

        if title.is_null() {
            title = if !wp.is_null() {
                c":setloclist()".as_ptr()
            } else {
                c":setqflist()".as_ptr()
            };
        }

        qf_st().set_qf_ll_recursive += 1;
        let l = (*list_arg).vval.v_list();
        if set_errorlist(
            wp,
            l.as_ref(),
            i32::from(action),
            title.cast_mut(),
            what.as_ref(),
        ) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
        qf_st().set_qf_ll_recursive -= 1;
    }
}

pub fn f_setloclist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    let win = find_win_by_nr_or_id(&argvars[0]);
    if !win.is_null() {
        set_qf_ll_list(win, &mut argvars[1..], rettv);
    }
}

pub fn f_setqflist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    set_qf_ll_list(ptr::null_mut(), argvars, rettv);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efm_option_part_len_handles_escapes_and_commas() {
        assert_eq!(efm_option_part_len(c"%f:%l:%m,%f:%l"), 8);
        assert_eq!(efm_option_part_len(c"%f\\,bar:%l,%m"), 10);
    }

    #[test]
    fn qf_entry_pos_comparisons_respect_linewise_mode() {
        let mut qfp = Qfline::new();
        qfp.qf_lnum = 10;
        qfp.qf_col = 5;
        let pos = CursorPos {
            lnum: 10,
            col: 5,
            coladd: 0,
        };
        assert!(!qf_entry_after_pos(&qfp, &pos, true));
        assert!(!qf_entry_after_pos(&qfp, &pos, false));
        assert!(qf_entry_on_or_after_pos(&qfp, &pos, true));
        assert!(qf_entry_on_or_after_pos(&qfp, &pos, false));
        assert!(qf_entry_on_or_before_pos(&qfp, &pos, true));
        assert!(qf_entry_on_or_before_pos(&qfp, &pos, false));
    }

    #[test]
    fn entry_is_closer_to_target_compares_file_line_and_column() {
        let mut e1 = Qfline::new();
        let mut e2 = Qfline::new();
        e1.qf_fnum = 1;
        e1.qf_lnum = 12;
        e1.qf_col = 4;
        e2.qf_fnum = 1;
        e2.qf_lnum = 15;
        e2.qf_col = 2;

        assert!(entry_is_closer_to_target(&e1, &e2, 1, 10, 1));
        assert!(!entry_is_closer_to_target(&e2, &e1, 1, 10, 1));

        e2.qf_lnum = 12;
        assert!(entry_is_closer_to_target(&e2, &e1, 1, 12, 1));
    }
}
