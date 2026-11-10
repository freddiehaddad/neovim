//! Quickfix and location list commands and functions (`src/nvim/quickfix.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{boolean_obj, cstr_as_obj, cstr_as_string, cstrn_as_string};
use crate::arglist::{ML_EMPTY, get_arglist_exp};
use crate::autocmd::{
    EVENT_BUFREADPOST, EVENT_BUFWINENTER, EVENT_FILETYPE, EVENT_QUICKFIXCMDPOST,
    EVENT_QUICKFIXCMDPRE, OPT_NOWIN, apply_autocmds, au_event_disable, au_event_restore,
    block_autocmds, unblock_autocmds,
};
use crate::buffer::{
    BCO_ENTER, BCO_NOHELP, BF_DUMMY, BF_NEW, BLN_DUMMY, BLN_NOOPT, Bufref, DOBUF_UNLOAD,
    DOBUF_WIPE, FileBuf, GETF_SETMARK, GETF_SWITCH, K_EXTMARK_NO_UNDO, MAXLNUM, VAR_FIXED, bt_help,
    bt_normal, bt_quickfix, buf_valid, buflist_findname_exp, buflist_findnr, buflist_findnr_opt,
    buflist_getfile, buflist_new, bufref_valid, close_buffer, do_modelines, no_write_message,
    set_bufref, setfname, wipe_buffer,
};
use crate::change::{TAB, changed_lines};
use crate::charset::{ascii_iswhite, skipdigits_slice, skipwhite_slice, vim_isprintc};
use crate::cmdexpand::Cmdidx;
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
    tv_dict_alloc, tv_dict_alloc_lock, tv_dict_alloc_ret, tv_dict_find, tv_dict_find_opt,
    tv_dict_get_bool, tv_dict_get_number, tv_dict_get_string, tv_dict_get_tv,
    tv_dict_item_alloc_len, tv_dict_item_free, tv_dict_unref, tv_get_number_chk, tv_get_string_chk,
    tv_initial_value, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_first,
    tv_list_iter, tv_list_len, tv_list_ref, tv_list_unref,
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
use crate::ex_cmds2::{autowrite_all, can_abandon};
use crate::ex_docmd::{CMOD_HIDE, Exarg, do_cmdline_cmd, ex_cd, is_loclist_cmd};
use crate::ex_eval::{ExCleanup, aborting, enter_cleanup, leave_cleanup};
use crate::ex_getln::get_list_range;
use crate::extmark::extmark_splice;
use crate::fileio::{
    EINTR, READ_DUMMY, READ_NEW, readfile, shorten_buf_fname, shorten_fnames, vim_fgets,
    vim_tempname,
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
    copy_option_part, set_option_direct, set_option_value_give_err, shortmess,
    skip_to_option_part_slice,
};
use crate::os::fs::{
    CFile, FileInfo, MAXPATHL, os_dirname, os_errno, os_fileinfo_link, os_isdir, os_open_stdin_fd,
    os_path_exists, os_remove, os_set_errno,
};
use crate::regexp::{
    RE_MAGIC, RE_STRING, Regmatch, Regmmatch, Regprog, vim_regcomp, vim_regexec, vim_regexec_multi,
};
use crate::window::{
    TabPageInfo, WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HELP, WSP_NEWLOC, WSP_QUICKFIX, WSP_TOP,
    WSP_VERT, WinInfoInfo, check_can_set_curbuf_forceit, check_lnums, goto_tabpage_win,
    reset_binding, tabline_height, win_close, win_enter, win_goto, win_is_focusable, win_setheight,
    win_setwidth, win_split, win_valid,
};

use crate::mbyte::{Vimconv, convert_setup, remove_bom, string_convert};
use crate::memline::{
    check_need_swap, ml_append_buf, ml_delete, ml_get_buf, ml_get_buf_len, ml_open,
};
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
use crate::optionstr::OptIndex;
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
use crate::strings::{format_to_buf, format_to_buf_safelen, gettext};
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum QfStatus {
    Fail,
    Ok,
    EndOfInput,
    NoMem,
    IgnoreLine,
    MultiScan,
    Abort,
}

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
    pub qf_viscol: bool,
    pub qf_cleared: bool,
    pub qf_type: u8,
    pub qf_user_data: TypVal,
    pub qf_valid: bool,
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
            qf_viscol: false,
            qf_cleared: false,
            qf_type: 0,
            qf_user_data: tv_initial_value(),
            qf_valid: false,
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
    pub fn cur_entry(&mut self) -> Option<&mut Qfline> {
        let idx = self.cur_idx();
        self.qf_items.get_mut(idx)
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

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct QfInfoId(Option<std::num::NonZeroU32>);

impl QfInfoId {
    pub const NULL: Self = Self(None);
    pub const QF_STACK: Self = Self(std::num::NonZeroU32::new(u32::MAX));

    #[inline(always)]
    pub const fn from_slot(idx: usize) -> Self {
        Self(std::num::NonZeroU32::new((idx as u32) + 1))
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(self) -> Option<&'static QfInfo> {
        let raw = self.0?.get();
        if raw == u32::MAX {
            Some(&qf_st().borrow().ql_info_actual)
        } else {
            qf_st()
                .borrow()
                .llist_arena
                .get((raw - 1) as usize)
                .and_then(|s| s.as_deref())
        }
    }

    #[inline(always)]
    pub fn opt_mut(self) -> Option<&'static mut QfInfo> {
        let raw = self.0?.get();
        if raw == u32::MAX {
            Some(&mut qf_st().borrow_mut().ql_info_actual)
        } else {
            qf_st()
                .borrow_mut()
                .llist_arena
                .get_mut((raw - 1) as usize)
                .and_then(|s| s.as_deref_mut())
        }
    }

    #[inline(always)]
    pub fn borrow(self) -> &'static QfInfo {
        self.opt().expect("null or freed QfInfoId")
    }

    #[inline(always)]
    pub fn borrow_mut(self) -> &'static mut QfInfo {
        self.opt_mut().expect("null or freed QfInfoId")
    }
}

impl From<&QfInfo> for QfInfoId {
    #[inline(always)]
    fn from(qi: &QfInfo) -> Self {
        qi.qf_id_handle
    }
}

impl From<&mut QfInfo> for QfInfoId {
    #[inline(always)]
    fn from(qi: &mut QfInfo) -> Self {
        qi.qf_id_handle
    }
}

impl PartialEq<QfInfo> for QfInfoId {
    #[inline(always)]
    fn eq(&self, other: &QfInfo) -> bool {
        *self == other.qf_id_handle
    }
}

impl PartialEq<&QfInfo> for QfInfoId {
    #[inline(always)]
    fn eq(&self, other: &&QfInfo) -> bool {
        *self == other.qf_id_handle
    }
}

impl PartialEq<&mut QfInfo> for QfInfoId {
    #[inline(always)]
    fn eq(&self, other: &&mut QfInfo) -> bool {
        *self == other.qf_id_handle
    }
}

impl std::ops::Deref for QfInfoId {
    type Target = QfInfo;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl std::ops::DerefMut for QfInfoId {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.borrow_mut()
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
    pub qf_id_handle: QfInfoId,
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
    Str(std::ffi::CString, usize),
    List(crate::eval::typval::ListitemId),
    Buf {
        buf: crate::globals::GraphPtr<FileBuf>,
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
    fn linebuf_slice(&self) -> &[u8] {
        let len = self.linelen;
        if self.use_growbuf {
            let cap = self.growbuf.len();
            &self.growbuf[..(len + 1).min(cap)]
        } else {
            let n = (len + 1).min(IOSIZE);
            &g_st().borrow_mut().iobuff_bytes()[..n]
        }
    }

    #[inline]
    fn linebuf_slice_mut(&mut self) -> &mut [u8] {
        let len = self.linelen;
        if self.use_growbuf {
            let cap = self.growbuf.len();
            &mut self.growbuf[..(len + 1).min(cap)]
        } else {
            let n = (len + 1).min(IOSIZE);
            &mut g_st().borrow_mut().iobuff_bytes_mut()[..n]
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
    qf_delq: Vec<QfInfoId>,
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
    llist_arena: Vec<Option<Box<QfInfo>>>,
    free_llists: Vec<usize>,
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
                qf_id_handle: QfInfoId::QF_STACK,
            },
            last_qf_id: 0,
            qf_last_bufname: None,
            qf_last_bufref: Bufref {
                br_buf: crate::globals::BufPtr::NULL,
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
            llist_arena: Vec::new(),
            free_llists: Vec::new(),
        }
    }
}

static QF_STATE: GlobalState<QuickfixState> = GlobalState::new(QuickfixState::new());

#[inline]
fn qf_st() -> crate::globals::GraphPtr<QuickfixState> {
    QF_STATE.handle()
}

#[inline]
fn ql_info_mut() -> &'static mut QfInfo {
    &mut qf_st().borrow_mut().ql_info_actual
}

#[inline]
fn is_qf_window(wp: &WinInfoInfo) -> bool {
    bt_quickfix(wp.buf_opt()) && wp.w_llist_ref.is_null()
}

#[inline]
fn is_ll_window(wp: &WinInfoInfo) -> bool {
    bt_quickfix(wp.buf_opt()) && !wp.w_llist_ref.is_null()
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
fn get_loc_list_opt(wp: &WinInfoInfo) -> Option<&'static mut QfInfo> {
    let p = if is_ll_window(wp) {
        wp.w_llist_ref
    } else {
        wp.w_llist
    };
    p.opt_mut()
}

#[inline]
fn tab_firstwin(tp: &TabPageInfo) -> &'static mut WinInfoInfo {
    if g_st().curtab == tp {
        g_st().firstwin.borrow_mut()
    } else {
        tp.tp_firstwin.borrow_mut()
    }
}

fn qfga_get() -> &'static mut Vec<u8> {
    if qf_st().qfga.capacity() == 0 {
        qf_st().qfga.reserve(256);
    }
    qf_st().qfga.clear();
    &mut qf_st().borrow_mut().qfga
}

fn qfga_clear() {
    if qf_st().qfga.capacity() > 1000 {
        qf_st().qfga = Vec::new();
    } else {
        qf_st().qfga.clear();
    }
}

#[inline]
fn qfga_concat(gap: &mut Vec<u8>, bytes: &[u8]) {
    let bytes = crate::memory::bytes_until_nul(bytes);
    if !bytes.is_empty() {
        gap.extend_from_slice(bytes);
    }
}

fn qf_init_process_nextline(
    qfl: &mut QfList,
    fmt_first: &[Efm],
    state: &mut Qfstate,
    fields: &mut Qffields,
) -> QfStatus {
    let mut status = qf_get_nextline(state);
    if status != QfStatus::Ok {
        return status;
    }

    let line_slice = &state.linebuf_slice()[..state.linelen];
    status = qf_parse_line(qfl, line_slice, fmt_first, fields);
    if status != QfStatus::Ok {
        return status;
    }

    let namebuf_cstr = crate::memory::bytes_as_cstr(&fields.namebuf);
    let currfile_owned: Option<std::ffi::CString>;
    let fname = if !namebuf_cstr.to_bytes().is_empty() || !qfl.qf_dir_stack.is_empty() {
        Some(namebuf_cstr)
    } else if !qfl.qf_file_stack.is_empty() && fields.valid {
        currfile_owned = qfl.currfile_ptr().map(CStr::to_owned);
        currfile_owned.as_deref()
    } else {
        None
    };

    let dir_owned = qfl.directory_ptr().map(CStr::to_owned);
    qf_add_entry(
        qfl,
        dir_owned.as_deref(),
        fname,
        Some(crate::memory::bytes_as_cstr(&fields.module)),
        fields.bnr,
        crate::memory::bytes_as_cstr(&fields.errmsg),
        fields.lnum,
        fields.end_lnum,
        fields.col,
        fields.end_col,
        fields.use_viscol,
        Some(crate::memory::bytes_as_cstr(&fields.pattern)),
        fields.enr,
        fields.type_,
        None,
        fields.valid,
    )
}

pub fn qf_init(
    wp: Option<&mut WinInfoInfo>,
    efile: Option<&CStr>,
    errorformat: &CStr,
    newlist: i32,
    qf_title: Option<&CStr>,
    enc: Option<&CStr>,
) -> i32 {
    let qi = wp.map_or_else(ql_info_mut, ll_get_or_alloc_list);
    let qf_curlist = qi.qf_curlist;

    qf_init_ext(
        qi,
        qf_curlist,
        efile,
        curbuf_opt_mut(),
        None,
        errorformat,
        newlist != 0,
        0,
        0,
        qf_title,
        enc,
    )
}

fn efmpat_to_regpat(
    efmpat: &[u8],
    regpat: &mut Vec<u8>,
    efminfo: &mut Efm,
    idx: usize,
    mut round: i32,
) -> bool {
    let first_b = efmpat.first().copied().unwrap_or(0);
    if efminfo.addr[idx] != 0 {
        semsg!(
            gettext(c"E372: Too many %%%c in format string"),
            i32::from(first_b),
        );
        return false;
    }
    if (idx != 0
        && idx < FMT_PATTERN_R
        && crate::strings::vim_haschr(c"DXOPQ", i32::from(efminfo.prefix)))
        || (idx == FMT_PATTERN_R && !crate::strings::vim_haschr(c"OPQ", i32::from(efminfo.prefix)))
    {
        semsg!(
            gettext(c"E373: Unexpected %%%c in format string"),
            i32::from(first_b),
        );
        return false;
    }
    round += 1;
    efminfo.addr[idx] = round as u8;
    regpat.extend_from_slice(b"\\(");
    #[cfg(windows)]
    if first_b == b'f' {
        regpat.extend_from_slice(b"\\%(\\a:\\)\\=");
    }
    let second_b = efmpat.get(1).copied().unwrap_or(0);
    if first_b == b'f' && second_b != 0 {
        if second_b != b'\\' && second_b != b'%' {
            regpat.extend_from_slice(b".\\{-1,}");
        } else {
            regpat.extend_from_slice(b"\\f\\+");
        }
    } else {
        regpat.extend_from_slice(FMT_PAT[idx].pattern.to_bytes());
    }
    regpat.extend_from_slice(b"\\)");
    true
}

fn scanf_fmt_to_regpat(efm: &[u8], i: &mut usize, len: usize, regpat: &mut Vec<u8>) -> bool {
    let first_b = efm.get(*i).copied().unwrap_or(0);
    if first_b == b'[' || first_b == b'\\' {
        regpat.push(first_b);
        if first_b == b'[' {
            if efm.get(*i + 1).copied() == Some(b'^') {
                *i += 1;
                regpat.push(efm[*i]);
            }
            if *i < len {
                *i += 1;
                regpat.push(efm[*i]);
                while *i < len {
                    *i += 1;
                    let c = efm.get(*i).copied().unwrap_or(0);
                    regpat.push(c);
                    if c == b']' {
                        break;
                    }
                }
                if *i == len {
                    emsg(gettext(c"E374: Missing ] in format string"));
                    return false;
                }
            }
        } else if *i < len {
            *i += 1;
            regpat.push(efm[*i]);
        }
        regpat.extend_from_slice(b"\\+");
        true
    } else {
        semsg!(
            gettext(c"E375: Unsupported %%%c in format string"),
            i32::from(first_b),
        );
        false
    }
}

fn efm_analyze_prefix(efm: &[u8], i: &mut usize, efminfo: &mut Efm) -> bool {
    let mut b = efm.get(*i).copied().unwrap_or(0);
    if crate::strings::vim_haschr(c"+-", i32::from(b)) {
        efminfo.flags = b;
        *i += 1;
        b = efm.get(*i).copied().unwrap_or(0);
    }
    if crate::strings::vim_haschr(c"DXAEWINCZGOPQ", i32::from(b)) {
        efminfo.prefix = b;
        true
    } else {
        semsg!(
            gettext(c"E376: Invalid %%%c in format string prefix"),
            i32::from(b),
        );
        false
    }
}

fn efm_to_regpat(efm_slice: &[u8], fmt_ptr: &mut Efm, regpat: &mut Vec<u8>) -> bool {
    let len = efm_slice.len();
    regpat.clear();
    regpat.push(b'^');
    let mut round: i32 = 0;
    let mut i: usize = 0;
    while i < len {
        if efm_slice[i] == b'%' {
            i += 1;
            let b = efm_slice.get(i).copied().unwrap_or(0);
            if let Some(idx) = FMT_PAT[..FMT_PATTERNS].iter().position(|p| p.convchar == b) {
                if !efmpat_to_regpat(&efm_slice[i..], regpat, fmt_ptr, idx, round) {
                    return false;
                }
                round += 1;
            } else if b == b'*' {
                i += 1;
                if !scanf_fmt_to_regpat(efm_slice, &mut i, len, regpat) {
                    return false;
                }
            } else if crate::strings::vim_haschr(c"%\\.^$~[", i32::from(b)) {
                regpat.push(b);
            } else if b == b'#' {
                regpat.push(b'*');
            } else if b == b'>' {
                fmt_ptr.conthere = 1;
            } else if i == 1 {
                if !efm_analyze_prefix(efm_slice, &mut i, fmt_ptr) {
                    return false;
                }
            } else {
                semsg!(
                    gettext(c"E377: Invalid %%%c in format string"),
                    i32::from(b),
                );
                return false;
            }
        } else {
            if efm_slice[i] == b'\\' && i + 1 < len {
                i += 1;
            } else if crate::strings::vim_haschr(c".*^$~[", i32::from(efm_slice[i])) {
                regpat.push(b'\\');
            }
            if efm_slice.get(i).copied().unwrap_or(0) != 0 {
                regpat.push(efm_slice[i]);
            }
        }
        i += 1;
    }
    regpat.push(b'$');
    regpat.push(0);
    true
}

fn efm_option_part_len(bytes: &[u8]) -> usize {
    let bytes = crate::memory::bytes_until_nul(bytes);
    let mut len: usize = 0;
    while len < bytes.len() && bytes[len] != b',' {
        if bytes[len] == b'\\' && len + 1 < bytes.len() {
            len += 1;
        }
        len += 1;
    }
    len
}

fn parse_efm_option(efm: &[u8]) -> Vec<Efm> {
    let mut rem = crate::memory::bytes_until_nul(efm);
    let mut fmt_list: Vec<Efm> = Vec::new();
    let mut fmtstr = Vec::new();

    let mut errored = false;
    while !rem.is_empty() {
        let mut fmt_item = Efm {
            prog: None,
            addr: [0; FMT_PATTERNS],
            prefix: 0,
            flags: 0,
            conthere: 0,
        };

        let len = efm_option_part_len(rem);

        if !efm_to_regpat(&rem[..len], &mut fmt_item, &mut fmtstr) {
            errored = true;
            break;
        }
        fmt_item.prog = crate::regexp::vim_regcomp_box(
            crate::memory::bytes_as_cstr(&fmtstr),
            RE_MAGIC + RE_STRING,
        );
        if fmt_item.prog.is_none() {
            errored = true;
            break;
        }
        fmt_list.push(fmt_item);
        rem = skip_to_option_part_slice(&rem[len..]);
    }

    if errored {
        fmt_list.clear();
        qf_st().fmt_start_idx = None;
    } else if fmt_list.is_empty() {
        emsg(gettext(c"E378: 'errorformat' contains no pattern"));
    }

    fmt_list
}

fn qf_grow_linebuf(state: &mut Qfstate, newsz: usize) {
    state.linelen = if newsz > LINE_MAXLEN {
        LINE_MAXLEN - 1
    } else {
        newsz
    };
    if state.growbuf.len() < state.linelen + 1 {
        state.growbuf.resize(state.linelen + 1, 0);
    }
    state.use_growbuf = true;
}

fn qf_get_next_str_line(state: &mut Qfstate) -> QfStatus {
    let QfInputSource::Str(ref s, ref mut off) = state.source else {
        return QfStatus::Fail;
    };
    let p_bytes = &s.to_bytes()[*off..];
    if p_bytes.is_empty() {
        return QfStatus::EndOfInput;
    }

    let len = p_bytes
        .iter()
        .position(|&b| b == b'\n')
        .map_or(p_bytes.len(), |idx| idx + 1);
    let line_bytes = p_bytes[..len].to_vec();
    *off += len;

    if len > IOSIZE - 2 {
        qf_grow_linebuf(state, len);
    } else {
        state.use_growbuf = false;
        state.linelen = len;
    }
    let copy_len = state.linelen;
    let dst = state.linebuf_slice_mut();
    dst[..copy_len].copy_from_slice(&line_bytes[..copy_len]);
    dst[copy_len] = 0;

    QfStatus::Ok
}

fn qf_get_next_list_line(state: &mut Qfstate, mut p_li: Option<&Listitem>) -> QfStatus {
    while let Some(li) = p_li {
        if li.li_tv.v_type == VarType::VarString && !li.li_tv.vval.v_string_is_null() {
            break;
        }
        p_li = li.li_next.opt();
    }

    let Some(li) = p_li else {
        state.source = QfInputSource::List(crate::eval::typval::ListitemId::NULL);
        return QfStatus::EndOfInput;
    };

    let s = li.li_tv.vval.v_string_bytes();
    let len = s.len();
    if len > IOSIZE - 2 {
        qf_grow_linebuf(state, len);
    } else {
        state.use_growbuf = false;
        state.linelen = len;
    }

    crate::memory::copy_bytes_to_c_buf(state.linebuf_slice_mut(), s);

    state.source = QfInputSource::List(li.li_next);

    QfStatus::Ok
}

fn qf_get_next_buf_line(
    state: &mut Qfstate,
    buf: &mut FileBuf,
    buflnum: Linenr,
    lnumlast: Linenr,
) -> QfStatus {
    if buflnum > lnumlast {
        return QfStatus::EndOfInput;
    }
    let buf_ptr = crate::globals::GraphPtr::from(&mut *buf);
    let p_buf = ml_get_buf(buf, buflnum).to_bytes();
    let len = ml_get_buf_len(buf, buflnum) as usize;
    state.source = QfInputSource::Buf {
        buf: buf_ptr,
        buflnum: buflnum + 1,
        lnumlast,
    };

    if len > IOSIZE - 2 {
        qf_grow_linebuf(state, len);
    } else {
        state.use_growbuf = false;
        state.linelen = len;
    }
    crate::memory::copy_bytes_to_c_buf(state.linebuf_slice_mut(), p_buf);
    QfStatus::Ok
}

fn qf_get_next_file_line(state: &mut Qfstate) -> QfStatus {
    let QfInputSource::File(ref fd) = state.source else {
        return QfStatus::Fail;
    };
    loop {
        os_set_errno(0);
        if !fd.gets(g_st().iobuff_bytes_mut()) {
            if os_errno() == EINTR {
                continue;
            }
            return QfStatus::EndOfInput;
        }
        break;
    }

    let mut discard = false;
    state.linelen = g_st().iobuff_cstr().to_bytes().len();
    if state.linelen == IOSIZE - 1 && g_st().iobuff[state.linelen - 1] != b'\n' {
        if state.growbuf.len() < 2 * (IOSIZE - 1) {
            state.growbuf.resize(2 * (IOSIZE - 1), 0);
        }

        state.growbuf[..IOSIZE - 1].copy_from_slice(&g_st().iobuff_bytes()[..IOSIZE - 1]);
        let mut growbuflen = state.linelen;
        state.growbuf[growbuflen] = 0;

        loop {
            os_set_errno(0);
            let growbufsiz = state.growbuf.len();
            if !fd.gets(&mut state.growbuf[growbuflen..growbufsiz]) {
                if os_errno() == EINTR {
                    continue;
                }
                break;
            }
            state.linelen = crate::memory::bytes_as_cstr(&state.growbuf[growbuflen..])
                .to_bytes()
                .len();
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
                if !fd.gets(g_st().iobuff_bytes_mut()) {
                    if os_errno() == EINTR {
                        continue;
                    }
                    break;
                }
                if g_st().iobuff_cstr().to_bytes().len() < IOSIZE - 1
                    || g_st().iobuff[IOSIZE - 2] == b'\n'
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

    let cur_slice = state.linebuf_slice();
    if state.vc.vc_type != CONV_NONE && !crate::memory::bytes_until_nul(cur_slice).is_ascii() {
        let mut linelen = state.linelen;
        if let Some(line) = string_convert(
            &state.vc,
            &cur_slice[..linelen.min(cur_slice.len())],
            Some(&mut linelen),
        ) {
            state.linelen = linelen;
            if state.linelen < IOSIZE {
                state.use_growbuf = false;
                let n = state.linelen + 1;
                crate::memory::copy_bytes_to_c_buf(&mut g_st().iobuff_bytes_mut()[..n], &line);
            } else {
                let cap = (state.linelen + 1).min(LINE_MAXLEN);
                if state.growbuf.len() < cap {
                    state.growbuf.resize(cap, 0);
                }
                crate::memory::copy_bytes_to_c_buf(&mut state.growbuf[..cap], &line);
                state.use_growbuf = true;
            }
        }
    }
    QfStatus::Ok
}

fn qf_get_nextline(state: &mut Qfstate) -> QfStatus {
    let status = match state.source {
        QfInputSource::None => QfStatus::Fail,
        QfInputSource::File(_) => qf_get_next_file_line(state),
        QfInputSource::Str(..) => qf_get_next_str_line(state),
        QfInputSource::List(p_li) => qf_get_next_list_line(state, p_li.opt()),
        QfInputSource::Buf {
            buf,
            buflnum,
            lnumlast,
        } => qf_get_next_buf_line(state, buf.borrow_mut(), buflnum, lnumlast),
    };

    if status != QfStatus::Ok {
        return status;
    }

    let linelen = state.linelen;
    let linebuf = state.linebuf_slice_mut();
    if linelen > 0 && linebuf.get(linelen - 1) == Some(&b'\n') {
        linebuf[linelen - 1] = 0;
        #[cfg(windows)]
        if linelen > 1 && linebuf.get(linelen - 2) == Some(&b'\r') {
            linebuf[linelen - 2] = 0;
        }
    }

    remove_bom(linebuf);

    QfStatus::Ok
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
    crate::globals::GraphPtr::of(qi.qf_lists.as_mut_ptr().wrapping_add(idx as usize)).borrow_mut()
}

fn qf_parse_line(
    qfl: &mut QfList,
    mut line: &[u8],
    fmt_first: &[Efm],
    fields: &mut Qffields,
) -> QfStatus {
    let fmt_len = fmt_first.len();
    loop {
        let start_idx = qf_st().fmt_start_idx;
        qf_st().fmt_start_idx = None;
        let mut fmt_idx = start_idx.unwrap_or(0);
        let mut fmt_opt: Option<&Efm> = None;

        let mut idx: i32 = 0;
        let mut tail_off: Option<usize> = None;
        let mut status;

        fields.valid = true;
        while fmt_idx < fmt_len {
            let cur_fmt = &fmt_first[fmt_idx];
            idx = i32::from(cur_fmt.prefix);
            status = qf_parse_get_fields(
                line,
                cur_fmt,
                fields,
                i32::from(qfl.qf_multiline),
                i32::from(qfl.qf_multiscan),
                &mut tail_off,
            );
            if status == QfStatus::NoMem {
                return status;
            }
            if status == QfStatus::Ok {
                fmt_opt = Some(cur_fmt);
                break;
            }
            fmt_idx += 1;
        }
        qfl.qf_multiscan = false;

        if fmt_opt.is_none() || idx == i32::from(b'D') || idx == i32::from(b'X') {
            if fmt_opt.is_some() {
                status = qf_parse_dir_pfx(idx, fields, qfl);
                if status != QfStatus::Ok {
                    return status;
                }
            }
            status = qf_parse_line_nomatch(line, fields);
            if status != QfStatus::Ok {
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
                if status != QfStatus::Ok {
                    return status;
                }
            } else if crate::strings::vim_haschr(c"OPQ", idx) {
                let tail_slice = tail_off.and_then(|off| line.get(off..));
                status = qf_parse_file_pfx(idx, fields, qfl, tail_slice);
                if status == QfStatus::MultiScan {
                    let s = crate::charset::skipwhite_slice(tail_slice.unwrap_or(&[]));
                    let new_linelen = crate::memory::bytes_until_nul(s).len();
                    if new_linelen >= line.len() {
                        return QfStatus::IgnoreLine;
                    }
                    line = &s[..new_linelen];
                    continue;
                }
            }
            if fmt_ptr.flags == b'-' {
                if qfl.qf_multiline {
                    qfl.qf_multiignore = true;
                }
                return QfStatus::IgnoreLine;
            }
        }

        return QfStatus::Ok;
    }
}

fn qf_setup_state(
    pstate: &mut Qfstate,
    enc: Option<&CStr>,
    efile: Option<&CStr>,
    tv: Option<&mut TypVal>,
    buf: Option<&mut FileBuf>,
    lnumfirst: Linenr,
    lnumlast: Linenr,
) -> bool {
    pstate.vc.vc_type = CONV_NONE;
    if let Some(enc_cstr) = enc.filter(|s| !s.to_bytes().is_empty()) {
        convert_setup(
            &mut pstate.vc,
            Some(enc_cstr),
            Some(opt_vars().p_enc.as_cstr()),
        );
    }

    if let Some(efile) = efile {
        let fd_opt = if efile.to_bytes() == b"-" {
            CFile::from_fd(os_open_stdin_fd(), c"r")
        } else {
            CFile::open(efile, c"r")
        };
        let Some(fd) = fd_opt else {
            semsg!(gettext(E_OPENERRF), efile);
            return false;
        };
        pstate.source = QfInputSource::File(fd);
    } else if let Some(tv) = tv {
        if tv.v_type == VarType::VarString {
            pstate.source = QfInputSource::Str(tv.vval.v_string_owned(), 0);
        } else if tv.v_type == VarType::VarList {
            pstate.source =
                QfInputSource::List(tv_list_first(tv.vval.v_list_opt().as_deref()).into());
        }
    } else {
        pstate.source = QfInputSource::Buf {
            buf: buf.into(),
            buflnum: lnumfirst,
            lnumlast,
        };
    }

    true
}

fn qf_init_ext(
    qi: &mut QfInfo,
    mut qf_idx: i32,
    efile: Option<&CStr>,
    mut buf: Option<&mut FileBuf>,
    mut tv: Option<&mut TypVal>,
    errorformat: &CStr,
    newlist: bool,
    lnumfirst: Linenr,
    lnumlast: Linenr,
    qf_title: Option<&CStr>,
    enc: Option<&CStr>,
) -> i32 {
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
        buf.as_deref_mut(),
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

    let efm = if ptr::eq(errorformat.as_ptr(), opt_vars().p_efm.as_ptr())
        && tv.is_none()
        && let Some(ref b) = buf
        && !b.b_p_efm.is_empty()
    {
        b.b_p_efm.as_cstr()
    } else {
        errorformat
    };

    let efm_changed = qf_st()
        .last_efm_cache
        .as_ref()
        .is_none_or(|c| c.as_bytes() != efm.to_bytes());
    if efm_changed {
        qf_st().last_efm_cache = None;
        qf_st().fmt_cache.clear();
        qf_st().fmt_start_idx = None;

        qf_st().fmt_cache = parse_efm_option(efm.to_bytes());
        if !qf_st().fmt_cache.is_empty() {
            qf_st().last_efm_cache = Some(efm.to_owned());
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
            if status == QfStatus::EndOfInput {
                break;
            }
            if status == QfStatus::Fail {
                ok = false;
                break;
            }
            line_breakcheck();
        }
    }

    if ok {
        let file_ok = match state.source {
            QfInputSource::File(ref fd) => !fd.has_error(),
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

fn qf_store_title(qfl: &mut QfList, title: Option<&CStr>) {
    qfl.qf_title = title.map(CStr::to_owned);
}

fn qf_cmdtitle(cmd: &CStr) -> std::ffi::CString {
    let mut buf = [0u8; IOSIZE];
    let len = format_to_buf_safelen!(&mut buf, c":%s", cmd);
    crate::memory::bytes_to_cstring(&buf[..len])
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

fn qf_new_list(qi: &mut QfInfo, qf_title: Option<&CStr>) {
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

fn qf_parse_fmt_f(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields, prefix: i32) -> QfStatus {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QfStatus::Fail;
    }

    let match_cstr = std::ffi::CString::new(rmp.span_bytes(midx)).unwrap_or_default();
    expand_env(match_cstr.as_c_str(), &mut fields.namebuf[..CMDBUFFSIZE]);

    if crate::strings::vim_haschr(c"OPQ", prefix)
        && !os_path_exists(crate::memory::bytes_as_cstr(&fields.namebuf))
    {
        return QfStatus::Fail;
    }

    QfStatus::Ok
}

fn qf_parse_fmt_b(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    let bnr = crate::charset::parse_i64_prefix(rmp.startp[midx].as_bytes()) as i32;
    if buflist_findnr(bnr).is_none() {
        return QfStatus::Fail;
    }
    fields.bnr = bnr;
    QfStatus::Ok
}

fn qf_parse_fmt_n(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.enr = crate::charset::parse_i64_prefix(rmp.startp[midx].as_bytes()) as i32;
    QfStatus::Ok
}

fn qf_parse_fmt_l(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.lnum = crate::charset::parse_i64_prefix(rmp.startp[midx].as_bytes()) as Linenr;
    QfStatus::Ok
}

fn qf_parse_fmt_e(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.end_lnum = crate::charset::parse_i64_prefix(rmp.startp[midx].as_bytes()) as Linenr;
    QfStatus::Ok
}

fn qf_parse_fmt_c(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.col = crate::charset::parse_i64_prefix(rmp.startp[midx].as_bytes()) as i32;
    QfStatus::Ok
}

fn qf_parse_fmt_k(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.end_col = crate::charset::parse_i64_prefix(rmp.startp[midx].as_bytes()) as i32;
    QfStatus::Ok
}

fn qf_parse_fmt_t(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.type_ = rmp.startp[midx].first_byte();
    QfStatus::Ok
}

fn copy_nonerror_line(line: &[u8], fields: &mut Qffields) -> QfStatus {
    let linelen = line.len();
    if linelen >= fields.errmsg.len() {
        fields.errmsg.resize(linelen + 1, 0);
    }
    crate::memory::copy_bytes_to_c_buf(&mut fields.errmsg[..linelen + 1], line);
    QfStatus::Ok
}

fn qf_parse_fmt_m(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QfStatus::Fail;
    }
    let span = rmp.span_bytes(midx);
    let len = span.len();
    if len >= fields.errmsg.len() {
        fields.errmsg.resize(len + 1, 0);
    }
    crate::memory::copy_bytes_to_c_buf(&mut fields.errmsg[..len + 1], span);
    QfStatus::Ok
}

fn qf_parse_fmt_r(
    rmp: &mut Regmatch,
    midx: usize,
    line_base: usize,
    tail_off: &mut Option<usize>,
) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    *tail_off = Some(rmp.startp[midx].addr().saturating_sub(line_base));
    QfStatus::Ok
}

fn qf_parse_fmt_p(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.col = 0;
    for &b in rmp.span_bytes(midx) {
        fields.col += 1;
        if b == TAB {
            fields.col += 7;
            fields.col -= fields.col % 8;
        }
    }
    fields.col += 1;
    fields.use_viscol = true;
    QfStatus::Ok
}

fn qf_parse_fmt_v(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() {
        return QfStatus::Fail;
    }
    fields.col = crate::charset::parse_i64_prefix(rmp.startp[midx].as_bytes()) as i32;
    fields.use_viscol = true;
    QfStatus::Ok
}

fn qf_parse_fmt_s(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QfStatus::Fail;
    }
    let span = rmp.span_bytes(midx);
    let len = span.len().min(CMDBUFFSIZE - 5);
    fields.pattern[..3].copy_from_slice(b"^\\V");
    fields.pattern[3..3 + len].copy_from_slice(&span[..len]);
    fields.pattern[len + 3..len + 6].copy_from_slice(b"\\$\0");
    QfStatus::Ok
}

fn qf_parse_fmt_o(rmp: &mut Regmatch, midx: usize, fields: &mut Qffields) -> QfStatus {
    if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
        return QfStatus::Fail;
    }
    let span = rmp.span_bytes(midx);
    let len = span.len();
    let dsize = (crate::memory::bytes_until_nul(&fields.module).len() + len + 1).min(CMDBUFFSIZE);
    crate::memory::append_bytes_to_c_buf(&mut fields.module[..dsize], span);
    QfStatus::Ok
}

fn qf_parse_match(
    line: &[u8],
    line_base: usize,
    fmt_ptr: &Efm,
    regmatch: &mut Regmatch,
    fields: &mut Qffields,
    qf_multiline: i32,
    qf_multiscan: i32,
    tail_off: &mut Option<usize>,
) -> QfStatus {
    let idx = fmt_ptr.prefix;

    if (idx == b'C' || idx == b'Z') && qf_multiline == 0 {
        return QfStatus::Fail;
    }
    if crate::strings::vim_haschr(c"EWIN", i32::from(idx)) {
        fields.type_ = idx;
    } else {
        fields.type_ = 0;
    }

    for i in 0..FMT_PATTERNS {
        let mut status = QfStatus::Ok;
        let midx = fmt_ptr.addr[i] as isize;
        if i == 0 && midx > 0 {
            status = qf_parse_fmt_f(regmatch, midx as usize, fields, i32::from(idx));
        } else if i == FMT_PATTERN_M {
            if fmt_ptr.flags == b'+' && qf_multiscan == 0 {
                status = copy_nonerror_line(line, fields);
            } else if midx > 0 {
                status = qf_parse_fmt_m(regmatch, midx as usize, fields);
            }
        } else if i == FMT_PATTERN_R && midx > 0 {
            status = qf_parse_fmt_r(regmatch, midx as usize, line_base, tail_off);
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
                _ => QfStatus::Ok,
            };
        }

        if status != QfStatus::Ok {
            return status;
        }
    }

    QfStatus::Ok
}

fn qf_parse_get_fields(
    line: &[u8],
    fmt_ptr: &Efm,
    fields: &mut Qffields,
    qf_multiline: i32,
    qf_multiscan: i32,
    tail_off: &mut Option<usize>,
) -> QfStatus {
    if qf_multiscan != 0 && !crate::strings::vim_haschr(c"OPQ", i32::from(fmt_ptr.prefix)) {
        return QfStatus::Fail;
    }

    fields.namebuf[0] = 0;
    fields.bnr = 0;
    fields.module[0] = 0;
    fields.pattern[0] = 0;
    if qf_multiscan == 0 {
        fields.errmsg[0] = 0;
    }
    fields.lnum = 0;
    fields.end_lnum = 0;
    fields.col = 0;
    fields.end_col = 0;
    fields.use_viscol = false;
    fields.enr = -1;
    fields.type_ = 0;
    *tail_off = None;

    let mut regmatch: Regmatch = Regmatch::new();
    regmatch.rm_ic = true;
    regmatch.borrow_prog(fmt_ptr.prog.as_deref());
    let line_cstr = crate::memory::bytes_to_cstring(crate::memory::bytes_until_nul(line));
    let line_base = line_cstr.as_ptr() as usize;
    let r = vim_regexec(&mut regmatch, &line_cstr, 0);
    let mut status = QfStatus::Fail;
    if r {
        status = qf_parse_match(
            line,
            line_base,
            fmt_ptr,
            &mut regmatch,
            fields,
            qf_multiline,
            qf_multiscan,
            tail_off,
        );
    }

    status
}

fn qf_parse_dir_pfx(idx: i32, fields: &mut Qffields, qfl: &mut QfList) -> QfStatus {
    if idx == i32::from(b'D') {
        let namebuf_cstr = crate::memory::bytes_as_cstr(&fields.namebuf);
        if namebuf_cstr.to_bytes().is_empty() {
            emsg(gettext(c"E379: Missing or empty directory name"));
            return QfStatus::Fail;
        }
        qf_push_dir(namebuf_cstr, &mut qfl.qf_dir_stack, false);
    } else if idx == i32::from(b'X') {
        qf_pop_dir(&mut qfl.qf_dir_stack);
    }

    QfStatus::Ok
}

fn qf_parse_file_pfx(
    idx: i32,
    fields: &mut Qffields,
    qfl: &mut QfList,
    tail: Option<&[u8]>,
) -> QfStatus {
    fields.valid = false;
    let namebuf_cstr = crate::memory::bytes_as_cstr(&fields.namebuf);
    if namebuf_cstr.to_bytes().is_empty() || os_path_exists(namebuf_cstr) {
        if !namebuf_cstr.to_bytes().is_empty() && idx == i32::from(b'P') {
            qf_push_dir(namebuf_cstr, &mut qfl.qf_file_stack, true);
        } else if idx == i32::from(b'Q') {
            qf_pop_dir(&mut qfl.qf_file_stack);
        }
        fields.namebuf[0] = 0;
        if tail.is_some_and(|t| !crate::memory::bytes_until_nul(t).is_empty()) {
            qfl.qf_multiscan = true;
            return QfStatus::MultiScan;
        }
    }

    QfStatus::Ok
}

fn qf_parse_line_nomatch(line: &[u8], fields: &mut Qffields) -> QfStatus {
    fields.namebuf[0] = 0;
    fields.lnum = 0;
    fields.valid = false;

    copy_nonerror_line(line, fields)
}

fn qf_parse_multiline_pfx(idx: i32, qfl: &mut QfList, fields: &mut Qffields) -> QfStatus {
    if !qfl.qf_multiignore {
        let Some(qfprev_ref) = qfl.qf_items.last_mut() else {
            return QfStatus::Fail;
        };
        let qfprev = crate::globals::GraphPtr::<Qfline>::from_ref(qfprev_ref);
        let qfprev = qfprev.borrow_mut();
        let errmsg_bytes = crate::memory::bytes_until_nul(&fields.errmsg);
        if !errmsg_bytes.is_empty() {
            let mut bytes = qfprev.qf_text.as_bytes().to_vec();
            bytes.push(b'\n');
            bytes.extend_from_slice(errmsg_bytes);
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
            qfprev.qf_viscol = fields.use_viscol;
        }
        if qfprev.qf_end_col == 0 {
            qfprev.qf_end_col = fields.end_col;
        }
        if qfprev.qf_fnum == 0 {
            let namebuf_cstr = crate::memory::bytes_as_cstr(&fields.namebuf);
            let currfile_owned: Option<std::ffi::CString>;
            let fname = if !namebuf_cstr.to_bytes().is_empty() || !qfl.qf_dir_stack.is_empty() {
                Some(namebuf_cstr)
            } else if !qfl.qf_file_stack.is_empty() && fields.valid {
                currfile_owned = qfl.currfile_ptr().map(CStr::to_owned);
                currfile_owned.as_deref()
            } else {
                None
            };
            let dir_owned = qfl.directory_ptr().map(CStr::to_owned);
            qfprev.qf_fnum = qf_get_fnum(qfl, dir_owned.as_deref(), fname);
        }
    }
    if idx == i32::from(b'Z') {
        qfl.qf_multiline = false;
        qfl.qf_multiignore = false;
    }
    line_breakcheck();

    QfStatus::IgnoreLine
}

fn locstack_queue_delreq(qi: &mut QfInfo) {
    qf_st().qf_delq.push(qi.qf_id_handle);
}

pub fn qf_stack_get_bufnr() -> i32 {
    qf_st().ql_info_actual.qf_bufnr
}

fn wipe_qf_buffer(qi: &mut QfInfo) {
    if qi.qf_bufnr == INVALID_QFBUFNR {
        return;
    }

    if let Some(qfbuf) = buflist_findnr_opt(qi.qf_bufnr)
        && qfbuf.b_nwindows == 0
    {
        let mut buf_was_null = false;
        if curwin_mut().w_buffer.is_null() {
            curwin_mut().w_buffer = g_st().curbuf;
            buf_was_null = true;
        }

        close_buffer(None, Some(qfbuf), DOBUF_WIPE, false, false, false);
        qi.qf_bufnr = INVALID_QFBUFNR;
        if buf_was_null {
            curwin_mut().w_buffer = crate::globals::BufPtr::NULL;
        }
    }
}

fn qf_free_list_stack_items(qi: &mut QfInfo) {
    for i in 0..qi.qf_listcount {
        qf_free(qf_get_list(&mut *qi, i));
    }
}

fn ll_free_all(qi: Option<&mut QfInfo>) {
    let Some(qi) = qi else {
        return;
    };

    if qf_st().quickfix_busy > 0 {
        locstack_queue_delreq(qi);
        return;
    }

    qi.qf_refcount -= 1;

    if qi.qf_refcount < 1 {
        let id = qi.qf_id_handle;
        wipe_qf_buffer(qi);
        if let Some(nz) = id.0
            && nz.get() != u32::MAX
        {
            let idx = (nz.get() - 1) as usize;
            let st = qf_st().borrow_mut();
            let dropped = st.llist_arena[idx].take();
            st.free_llists.push(idx);
            drop(dropped);
        }
    }
}

pub fn qf_free_all(wp: Option<&mut WinInfoInfo>) {
    if let Some(wp) = wp {
        let llist = std::mem::replace(&mut wp.w_llist, QfInfoId::NULL);
        ll_free_all(llist.opt_mut());
        let llist_ref = std::mem::replace(&mut wp.w_llist_ref, QfInfoId::NULL);
        ll_free_all(llist_ref.opt_mut());
    } else {
        qf_free_list_stack_items(ql_info_mut());
    }
}

fn incr_quickfix_busy() {
    qf_st().quickfix_busy += 1;
}

fn decr_quickfix_busy() {
    qf_st().quickfix_busy -= 1;
    if qf_st().quickfix_busy == 0 {
        while let Some(qi) = qf_st().qf_delq.pop() {
            ll_free_all(qi.opt_mut());
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
    dir: Option<&CStr>,
    fname: Option<&CStr>,
    module: Option<&CStr>,
    bufnum: i32,
    mesg: &CStr,
    lnum: Linenr,
    end_lnum: Linenr,
    col: i32,
    end_col: i32,
    vis_col: bool,
    pattern: Option<&CStr>,
    nr: i32,
    mut type_: u8,
    user_data: Option<&TypVal>,
    valid: bool,
) -> QfStatus {
    let mut qfp = Qfline::new();

    let buf = if bufnum != 0 {
        let b = buflist_findnr_opt(bufnum);
        qfp.qf_fnum = bufnum;
        if let Some(ref mut b) = b.map(|r| &mut *r) {
            b.b_has_qf_entry |= if is_qf_list(qfl) {
                BUF_HAS_QF_ENTRY
            } else {
                BUF_HAS_LL_ENTRY
            };
        }
        buflist_findnr_opt(bufnum)
    } else {
        qfp.qf_fnum = qf_get_fnum(qfl, dir, fname);
        buflist_findnr_opt(qfp.qf_fnum)
    };
    let fullname = fname.and_then(|f| fix_fname(Some(f)));
    qfp.qf_fname = None;
    if let Some(buf) = buf
        && buf.b_ffname.is_some()
        && let Some(ref full) = fullname
        && !path_equal(full, buf.ffname_cstr(), K_PATH_CMP_LITERAL)
        && let Some(p) = path_try_shorten_fname(Some(full))
    {
        qfp.qf_fname = Some(p.to_owned());
    }
    drop(fullname);
    qfp.qf_text = mesg.to_owned();
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
    qfp.qf_pattern = pattern
        .filter(|s| !s.to_bytes().is_empty())
        .map(CStr::to_owned);
    qfp.qf_module = module
        .filter(|s| !s.to_bytes().is_empty())
        .map(CStr::to_owned);
    qfp.qf_nr = nr;
    if type_ != 1 && !vim_isprintc(i32::from(type_)) {
        type_ = 0;
    }
    qfp.qf_type = type_;
    qfp.qf_valid = valid;
    qfp.qf_cleared = false;

    if qf_list_empty(Some(&*qfl)) {
        qfl.qf_index = 0;
    }
    qfl.qf_items.push(qfp);
    if qfl.qf_index == 0 && valid {
        qfl.qf_index = qfl.count();
    }

    QfStatus::Ok
}

pub fn qf_resize_stack(n: i32) {
    qf_resize_stack_base(ql_info_mut(), n);
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
    if let Some(wp) = qf_find_win_with_loclist(llw.w_llist_ref.borrow()) {
        wp.w_onebuf_opt.wo_lhi = llw.w_onebuf_opt.wo_lhi;
    }
}

fn qf_sync_win_to_llw(pwp: &mut WinInfoInfo) {
    let llw = pwp.w_llist;
    if !llw.is_null() {
        for wp in win_iter_mut(tab_firstwin(curtab_mut())) {
            if wp.w_llist_ref == llw && bt_quickfix(wp.buf_opt()) {
                wp.w_onebuf_opt.wo_lhi = pwp.w_onebuf_opt.wo_lhi;
                return;
            }
        }
    }
}

fn qf_alloc_stack(qfltype: Qfltype, n: i32) -> &'static mut QfInfo {
    let qi = if qfltype == QFLT_QUICKFIX {
        ql_info_mut()
    } else {
        let st = qf_st().borrow_mut();
        let idx = if let Some(idx) = st.free_llists.pop() {
            let id = QfInfoId::from_slot(idx);
            st.llist_arena[idx] = Some(Box::new(QfInfo {
                qf_refcount: 1,
                qf_listcount: 0,
                qf_curlist: 0,
                qf_maxcount: 0,
                qf_lists: Vec::new(),
                qfl_type: 0,
                qf_bufnr: 0,
                qf_id_handle: id,
            }));
            idx
        } else {
            let idx = st.llist_arena.len();
            let id = QfInfoId::from_slot(idx);
            st.llist_arena.push(Some(Box::new(QfInfo {
                qf_refcount: 1,
                qf_listcount: 0,
                qf_curlist: 0,
                qf_maxcount: 0,
                qf_lists: Vec::new(),
                qfl_type: 0,
                qf_bufnr: 0,
                qf_id_handle: id,
            })));
            idx
        };
        st.llist_arena[idx].as_deref_mut().unwrap()
    };
    qi.qfl_type = qfltype;
    qi.qf_bufnr = INVALID_QFBUFNR;
    qi.qf_lists.clear();
    qi.qf_lists.resize_with(n as usize, QfList::default);
    qi.qf_maxcount = n;

    qi
}

fn ll_get_or_alloc_list(wp: &mut WinInfoInfo) -> &'static mut QfInfo {
    if is_ll_window(wp) {
        return wp.w_llist_ref.borrow_mut();
    }

    let llist_ref = std::mem::replace(&mut wp.w_llist_ref, QfInfoId::NULL);
    ll_free_all(llist_ref.opt_mut());

    if wp.w_llist.is_null() {
        wp.w_llist = qf_alloc_stack(QFLT_LOCATION, wp.w_onebuf_opt.wo_lhi as i32).qf_id_handle;
    }

    wp.w_llist.borrow_mut()
}

fn qf_cmd_get_stack_opt(eap: &mut Exarg, print_emsg: bool) -> Option<&'static mut QfInfo> {
    if is_loclist_cmd(eap.cmdidx) {
        let Some(qi) = get_loc_list_opt(curwin_mut()) else {
            if print_emsg {
                emsg(gettext(E_LOCLIST));
            }
            return None;
        };
        return Some(qi);
    }

    Some(ql_info_mut())
}

fn qf_cmd_get_or_alloc_stack(
    eap: &Exarg,
    pwinp: &mut Option<&'static mut WinInfoInfo>,
) -> &'static mut QfInfo {
    if is_loclist_cmd(eap.cmdidx) {
        *pwinp = Some(curwin_mut());
        return ll_get_or_alloc_list(curwin_mut());
    }
    ql_info_mut()
}

fn copy_loclist_entries(from_qfl: &QfList, to_qfl: &mut QfList) -> bool {
    for from_qfp in &from_qfl.qf_items {
        if g_st().got_int {
            break;
        }
        if qf_add_entry(
            to_qfl,
            None,
            None,
            from_qfp.qf_module.as_deref(),
            0,
            &from_qfp.qf_text,
            from_qfp.qf_lnum,
            from_qfp.qf_end_lnum,
            from_qfp.qf_col,
            from_qfp.qf_end_col,
            from_qfp.qf_viscol,
            from_qfp.qf_pattern.as_deref(),
            from_qfp.qf_nr,
            0,
            Some(&from_qfp.qf_user_data),
            from_qfp.qf_valid,
        ) == QfStatus::Fail
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
    let Some(qi) = get_loc_list_opt(&*from) else {
        return;
    };

    let to_ll = qf_alloc_stack(QFLT_LOCATION, from.w_onebuf_opt.wo_lhi as i32);
    to.w_llist = to_ll.qf_id_handle;
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

fn qf_get_fnum(qfl: &mut QfList, directory: Option<&CStr>, fname: Option<&CStr>) -> i32 {
    let Some(fname) = fname.filter(|s| !s.to_bytes().is_empty()) else {
        return 0;
    };

    let fname_str = cstr_as_string(Some(fname));
    #[cfg_attr(not(windows), expect(unused_mut))]
    let mut bufname_cstr = if let Some(dir) = directory
        && !vim_isAbsName(fname.to_bytes())
    {
        let mut joined = concat_fnames(cstr_as_string(Some(dir)), fname_str, true);
        if !os_path_exists(&joined) {
            let dir2 = qf_guess_filepath(qfl, fname);
            joined = if let Some(dir2) = dir2 {
                concat_fnames(cstr_as_string(Some(dir2)), fname_str, true)
            } else {
                fname.to_owned()
            };
        }
        joined
    } else {
        fname.to_owned()
    };
    #[cfg(windows)]
    {
        let mut bytes = bufname_cstr.into_bytes_with_nul();
        path_to_slash(&mut bytes);
        bufname_cstr = std::ffi::CString::from_vec_with_nul(bytes).unwrap_or_default();
    }

    let same_as_last = qf_st()
        .qf_last_bufname
        .as_ref()
        .is_some_and(|last| bufname_cstr.as_bytes() == last.to_bytes());
    let buf = if same_as_last && bufref_valid(Some(&qf_st().qf_last_bufref)) {
        qf_st().qf_last_bufref.br_buf
    } else {
        let b = crate::globals::GraphPtr::from(buflist_new(
            Some(bufname_cstr.as_c_str()),
            None,
            0,
            BLN_NOOPT,
        ));
        qf_st().qf_last_bufname = Some(bufname_cstr);
        set_bufref(&mut qf_st().qf_last_bufref, b.opt_mut());
        b
    };
    let Some(buf) = buf.opt_mut() else {
        return 0;
    };
    buf.b_has_qf_entry = if is_qf_list(qfl) {
        BUF_HAS_QF_ENTRY
    } else {
        BUF_HAS_LL_ENTRY
    };
    buf.b_fnum
}

fn qf_push_dir(dirbuf: &CStr, stack: &mut Vec<std::ffi::CString>, is_file_stack: bool) {
    if vim_isAbsName(dirbuf.to_bytes()) || stack.is_empty() || is_file_stack {
        stack.push(dirbuf.to_owned());
        return;
    }

    let mut found_idx: Option<usize> = None;
    let mut new_dirname: Option<std::ffi::CString> = None;
    for i in (0..stack.len()).rev() {
        let dirname_cstr = concat_fnames(
            cstr_as_string(Some(stack[i].as_c_str())),
            cstrn_as_string(dirbuf.to_bytes()),
            true,
        );
        if os_isdir(&dirname_cstr) {
            found_idx = Some(i);
            new_dirname = Some(dirname_cstr);
            break;
        }
    }

    let truncate_to = found_idx.map_or(0, |i| i + 1);
    stack.truncate(truncate_to);

    let cstr = new_dirname.unwrap_or_else(|| dirbuf.to_owned());
    stack.push(cstr);
}

fn qf_pop_dir(stack: &mut Vec<std::ffi::CString>) {
    stack.pop();
}

fn qf_guess_filepath<'a>(qfl: &'a mut QfList, filename: &CStr) -> Option<&'a CStr> {
    let stack = &mut qfl.qf_dir_stack;
    if stack.is_empty() {
        return None;
    }

    let top_idx = stack.len() - 1;
    let mut found_idx: Option<usize> = None;
    let filename_str = cstr_as_string(Some(filename));
    for i in (0..top_idx).rev() {
        let fullname_cstr = concat_fnames(
            cstr_as_string(Some(stack[i].as_c_str())),
            filename_str,
            true,
        );
        if os_path_exists(&fullname_cstr) {
            found_idx = Some(i);
            break;
        }
    }

    let drain_start = found_idx.map_or(0, |i| i + 1);
    stack.drain(drain_start..top_idx);

    found_idx.map(|_| stack[stack.len() - 2].as_c_str())
}

fn qflist_valid(wp: Option<&WinInfoInfo>, qf_id: u32) -> bool {
    let qi = if let Some(wp) = wp {
        if !win_valid(Some(wp)) {
            return false;
        }
        let Some(ll) = get_loc_list_opt(wp) else {
            return false;
        };
        ll
    } else {
        ql_info_mut()
    };

    qi.qf_lists[..qi.qf_listcount as usize]
        .iter()
        .any(|qfl| qfl.qf_id == qf_id)
}

fn is_qf_entry_present(qfl: &QfList, qf_index: i32) -> bool {
    !g_st().got_int && qf_index >= 1 && (qf_index as usize) <= qfl.qf_items.len()
}

fn get_next_valid_entry<'a>(
    qfl: &'a mut QfList,
    qf_index: &mut i32,
    dir: i32,
) -> Option<&'a mut Qfline> {
    let mut idx = *qf_index;
    if idx < 1 || (idx as usize) > qfl.qf_items.len() {
        return None;
    }
    let old_qf_fnum = qfl.qf_items[(idx - 1) as usize].qf_fnum;

    loop {
        if (idx as usize) >= qfl.qf_items.len() {
            return None;
        }
        idx += 1;
        let qfp = &qfl.qf_items[(idx - 1) as usize];
        if !((!qfl.qf_nonevalid && !qfp.qf_valid)
            || (dir == FORWARD_FILE && qfp.qf_fnum == old_qf_fnum))
        {
            break;
        }
    }

    *qf_index = idx;
    Some(&mut qfl.qf_items[(idx - 1) as usize])
}

fn get_prev_valid_entry<'a>(
    qfl: &'a mut QfList,
    qf_index: &mut i32,
    dir: i32,
) -> Option<&'a mut Qfline> {
    let mut idx = *qf_index;
    if idx < 1 || (idx as usize) > qfl.qf_items.len() {
        return None;
    }
    let old_qf_fnum = qfl.qf_items[(idx - 1) as usize].qf_fnum;

    loop {
        if idx <= 1 {
            return None;
        }
        idx -= 1;
        let qfp = &qfl.qf_items[(idx - 1) as usize];
        if !((!qfl.qf_nonevalid && !qfp.qf_valid)
            || (dir == BACKWARD_FILE && qfp.qf_fnum == old_qf_fnum))
        {
            break;
        }
    }

    *qf_index = idx;
    Some(&mut qfl.qf_items[(idx - 1) as usize])
}

fn get_nth_valid_entry<'a>(
    qfl: &'a mut QfList,
    mut errornr: i32,
    dir: i32,
    new_qfidx: &mut i32,
) -> Option<&'a mut Qfline> {
    let mut qf_ptr = crate::globals::GraphPtr::from(qfl.cur_entry());
    let mut qf_idx = qfl.qf_index;
    let mut err: Option<&CStr> = Some(E_NO_MORE_ITEMS);

    while errornr > 0 {
        errornr -= 1;
        let prev_qf_ptr = qf_ptr;
        let prev_index = qf_idx;

        if dir == FORWARD || dir == FORWARD_FILE {
            qf_ptr = get_next_valid_entry(qfl, &mut qf_idx, dir).into();
        } else {
            qf_ptr = get_prev_valid_entry(qfl, &mut qf_idx, dir).into();
        }

        if qf_ptr.is_null() {
            qf_ptr = prev_qf_ptr;
            qf_idx = prev_index;
            if let Some(err) = err {
                emsg(gettext(err));
                return None;
            }
            break;
        }

        err = None;
    }

    *new_qfidx = qf_idx;
    qf_ptr.opt_mut()
}

fn get_nth_entry<'a>(
    qfl: &'a mut QfList,
    errornr: i32,
    new_qfidx: &mut i32,
) -> Option<&'a mut Qfline> {
    if qfl.qf_items.is_empty() {
        *new_qfidx = qfl.qf_index;
        return None;
    }
    let qf_idx = errornr.clamp(1, qfl.count());
    *new_qfidx = qf_idx;
    Some(&mut qfl.qf_items[(qf_idx - 1) as usize])
}

fn qf_get_entry<'a>(
    qfl: &'a mut QfList,
    errornr: i32,
    dir: i32,
    new_qfidx: &mut i32,
) -> Option<&'a mut Qfline> {
    if dir != 0 {
        return get_nth_valid_entry(qfl, errornr, dir, new_qfidx);
    } else if errornr != 0 {
        return get_nth_entry(qfl, errornr, new_qfidx);
    }

    *new_qfidx = qfl.qf_index;
    qfl.cur_entry()
}

fn qf_find_help_win() -> Option<&'static mut WinInfoInfo> {
    for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if bt_help(wp.buf_opt()) && win_is_focusable(wp) {
            return Some(&mut *wp);
        }
    }
    None
}

fn win_set_loclist(wp: &mut WinInfoInfo, qi: &mut QfInfo) {
    wp.w_llist = qi.qf_id_handle;
    qi.qf_refcount += 1;
}

fn jump_to_help_window(qi: &mut QfInfo, newwin: bool, opened_window: &mut bool) -> bool {
    let wp = if g_st().cmdmod.cmod_tab != 0 || newwin {
        None
    } else {
        qf_find_help_win()
    };

    if let Some(wp_ref) = wp
        && wp_ref.buf_mut().b_nwindows > 0
    {
        win_enter(wp_ref, true);
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

fn qf_find_win_with_loclist(ll: &QfInfo) -> Option<&'static mut WinInfoInfo> {
    for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if wp.w_llist == ll && !bt_quickfix(wp.buf_opt()) {
            return Some(&mut *wp);
        }
    }
    None
}

fn qf_find_win_with_normal_buf() -> Option<&'static mut WinInfoInfo> {
    for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if bt_normal(wp.buf_opt()) {
            return Some(&mut *wp);
        }
    }
    None
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
    opt_vars().p_swb = OptString::new();
    opt_vars().swb_flags = 0;
    reset_binding(curwin_mut());
    if let Some(ll_ref) = ll_ref {
        win_set_loclist(curwin_mut(), ll_ref);
    }
    true
}

fn qf_goto_win_with_ll_file(
    use_win: Option<&mut WinInfoInfo>,
    qf_fnum: i32,
    ll_ref: Option<&mut QfInfo>,
) {
    let mut win = crate::globals::GraphPtr::from(use_win);

    if win.is_null() {
        for win2 in win_iter_ptr(tab_firstwin(curtab_mut())) {
            if (*win2).buf_mut().b_fnum == qf_fnum {
                win = win2.into();
                break;
            }
        }
        if win.is_null() {
            win = curwin_ptr();
            loop {
                if bt_normal(win.w_buffer.opt()) {
                    break;
                }
                if win.w_prev.is_null() {
                    win = g_st().lastwin;
                } else {
                    win = win.w_prev;
                }
                if win == curwin_ptr() {
                    break;
                }
            }
        }
    }
    win_goto(win.borrow_mut());

    if win.w_llist.is_null()
        && let Some(ll_ref) = ll_ref
    {
        win_set_loclist(win.borrow_mut(), ll_ref);
    }
}

fn qf_goto_win_with_qfl_file(qf_fnum: i32) {
    let mut win = curwin_ptr();
    let mut altwin = crate::globals::GraphPtr::<WinInfoInfo>::NULL;
    loop {
        if win.buf_mut().b_fnum == qf_fnum {
            break;
        }
        if win.w_prev.is_null() {
            win = g_st().lastwin;
        } else {
            win = win.w_prev;
        }

        if is_qf_window(win.borrow()) {
            if (opt_vars().swb_flags & K_OPT_SWB_FLAG_USELAST) != 0
                && win_valid(g_st().prevwin.opt())
                && !crate::globals::prevwin_mut().w_onebuf_opt.wo_wfb
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
            && !win.w_onebuf_opt.wo_pvw
            && !win.w_onebuf_opt.wo_wfb
            && bt_normal(win.w_buffer.opt())
        {
            altwin = crate::globals::GraphPtr::<WinInfoInfo>::of(win);
        }
    }

    win_goto(win.borrow_mut());
}

fn qf_jump_to_usable_window(qf_fnum: i32, newwin: bool, opened_window: &mut bool) -> bool {
    let mut usable_wp: Option<&mut WinInfoInfo> = None;
    let mut usable_win = false;

    let ll_ref = if newwin {
        QfInfoId::NULL
    } else {
        curwin_mut().w_llist_ref
    };
    if !ll_ref.is_null() {
        usable_wp = qf_find_win_with_loclist(ll_ref.borrow());
        if usable_wp.is_some() {
            usable_win = true;
        }
    }

    if !usable_win && qf_find_win_with_normal_buf().is_some() {
        usable_win = true;
    }

    if !usable_win && (opt_vars().swb_flags & K_OPT_SWB_FLAG_USETAB) != 0 {
        usable_win = qf_goto_tabwin_with_file(qf_fnum);
    }

    if ((g_st().firstwin == g_st().lastwin) && bt_quickfix(Some(crate::globals::curbuf_mut())))
        || !usable_win
        || newwin
    {
        if !qf_open_new_file_win(ll_ref.opt_mut()) {
            return false;
        }
        *opened_window = true;
    } else if !curwin_mut().w_llist_ref.is_null() {
        qf_goto_win_with_ll_file(usable_wp, qf_fnum, ll_ref.opt_mut());
    } else {
        qf_goto_win_with_qfl_file(qf_fnum);
    }

    true
}

fn qf_jump_edit_buffer(
    qi_ref: &mut QfInfo,
    qf_index: i32,
    forceit: i32,
    prev_winid: i32,
    opened_window: &mut bool,
) -> QfStatus {
    let qi = QfInfoId::from(qi_ref);
    let qfl = crate::globals::GraphPtr::<QfList>::from_ref(qf_get_curlist(qi.borrow_mut()));
    let old_changetick = qfl.qf_changedtick;
    let old_qf_curlist = qi.qf_curlist;
    let qfl_type = qfl.qfl_type;
    let mut retval = QfStatus::Ok;
    let save_qfid = qfl.qf_id;
    let (qf_type, fnum) = {
        let qfp = &&(qfl.borrow().qf_items)[(qf_index - 1) as usize];
        (qfp.qf_type, qfp.qf_fnum)
    };

    if qf_type == 1 {
        if !can_abandon(curbuf_mut(), forceit != 0) {
            no_write_message();
            return QfStatus::Fail;
        }
        retval = if do_ecmd(
            fnum,
            None,
            None,
            None,
            1,
            ECMD_HIDE + ECMD_SET_HELP,
            (prev_winid == curwin_mut().handle).then(curwin_mut),
        ) {
            QfStatus::Ok
        } else {
            QfStatus::Fail
        };
    } else {
        if forceit == 0 && curwin_mut().w_onebuf_opt.wo_wfb && curbuf_mut().b_fnum != fnum {
            if qi.qfl_type == QFLT_LOCATION {
                emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
                return QfStatus::Fail;
            }

            if win_valid(g_st().prevwin.opt())
                && !crate::globals::prevwin_mut().w_onebuf_opt.wo_wfb
                && !bt_quickfix(crate::globals::prevwin_mut().buf_opt())
            {
                win_goto(crate::globals::prevwin_mut());
            }
            if curwin_mut().w_onebuf_opt.wo_wfb {
                if win_split(0, 0) {
                    *opened_window = true;
                }
                if curwin_mut().w_onebuf_opt.wo_wfb {
                    emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
                    retval = QfStatus::Fail;
                }
            }
        }

        if retval == QfStatus::Ok {
            retval = if buflist_getfile(fnum, 1, GETF_SETMARK | GETF_SWITCH, forceit) {
                QfStatus::Ok
            } else {
                QfStatus::Fail
            };
        }
    }

    if qfl_type == QFLT_LOCATION {
        let wp = win_id2wp(prev_winid);
        if wp.is_none() && curwin_mut().w_llist != qi {
            emsg(gettext(c"E924: Current window was closed"));
            *opened_window = false;
            return QfStatus::Abort;
        }
    }

    if qfl_type == QFLT_QUICKFIX && !qflist_valid(None, save_qfid) {
        emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        return QfStatus::Abort;
    }

    if old_qf_curlist != qi.qf_curlist
        || old_changetick != qfl.qf_changedtick
        || !is_qf_entry_present(qfl.borrow(), qf_index)
    {
        if qfl_type == QFLT_QUICKFIX {
            emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        } else {
            emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
        }
        return QfStatus::Abort;
    }

    retval
}
fn qf_screen_col_to_idx(vcol: Colnr) -> i32 {
    let line = get_cursor_line_ptr().to_bytes();
    let mut off = 0usize;
    let mut col: Colnr = 0;

    while off < line.len() && col < vcol {
        if line[off] == TAB {
            col += 8 - (col % 8);
        } else {
            col += crate::mbyte::slice2cells(&line[off..]);
        }
        if col > vcol {
            break;
        }
        off += crate::mbyte::utfc_slice2len(&line[off..]);
    }
    off as i32
}

fn qf_jump_goto_line(qf_lnum: Linenr, qf_col: i32, qf_viscol: bool, qf_pattern: Option<&CStr>) {
    if let Some(qf_pattern) = qf_pattern {
        let save_cursor = curwin_mut().w_cursor;
        curwin_mut().w_cursor.lnum = 0;
        if do_search(
            None,
            b'/' as i32,
            b'/' as i32,
            Some(qf_pattern),
            qf_pattern.to_bytes().len(),
            1,
            SEARCH_KEEP,
            opt_vars().p_magic,
            None,
        ) == 0
        {
            curwin_mut().w_cursor = save_cursor;
        }
    } else {
        let mut i = qf_lnum;
        if i > 0 {
            i = i.min(curbuf_mut().b_ml.ml_line_count);
            curwin_mut().w_cursor.lnum = i;
        }
        if qf_col > 0 {
            curwin_mut().w_cursor.coladd = 0;
            if qf_viscol {
                curwin_mut().w_cursor.col = qf_screen_col_to_idx(qf_col - 1);
            } else {
                curwin_mut().w_cursor.col = qf_col - 1;
            }
            curwin_mut().w_set_curswant = true;
            check_cursor(curwin_mut());
        } else {
            beginline(BL_WHITE | BL_FIX);
        }
    }
}

fn qf_jump_print_msg(
    qi: &mut QfInfo,
    qf_index: i32,
    qf_ptr: &mut Qfline,
    same_curbuf: bool,
    old_lnum: Linenr,
) {
    let gap = qfga_get();

    if g_st().msg_scrolled == 0 {
        update_topline(curwin_mut());
        if g_st().must_redraw != 0 {
            update_screen();
        }
    }
    let deleted_str = if qf_ptr.qf_cleared {
        gettext(c" (line deleted)")
    } else {
        c""
    };
    let iobufflen = format_to_buf_safelen!(
        &mut g_st().iobuff,
        gettext(c"(%d of %d)%s%s: "),
        qf_index,
        qf_get_curlist(qi).count(),
        deleted_str,
        qf_types(i32::from(qf_ptr.qf_type), qf_ptr.qf_nr).as_c_str(),
    );
    qfga_concat(gap, &g_st().iobuff_bytes()[..iobufflen]);
    qf_fmt_text(
        gap,
        crate::charset::skipwhite_slice(qf_ptr.qf_text.as_bytes()),
    );
    gap.push(0);

    let i = g_st().msg_scroll;
    if same_curbuf && curwin_mut().w_cursor.lnum == old_lnum {
        g_st().msg_scroll = 1;
    } else if (g_st().msg_scrolled == 0 || (opt_vars().p_ch == 0 && g_st().msg_scrolled == 1))
        && shortmess(K_SHM_OVERALL)
    {
        g_st().msg_scroll = 0;
    }
    msg_ext_set_kind(c"quickfix");
    msg_keep(crate::memory::bytes_as_cstr(gap), 0, true, false);
    g_st().msg_scroll = i;

    qfga_clear();
}

fn qf_jump_open_window(
    qi_ref: &mut QfInfo,
    qf_index: i32,
    newwin: bool,
    opened_window: &mut bool,
) -> Option<QfStatus> {
    let qi = crate::globals::GraphPtr::from(qi_ref);
    let qfl = crate::globals::GraphPtr::<QfList>::from_ref(qf_get_curlist(qi.borrow_mut()));
    let old_changetick = qfl.qf_changedtick;
    let old_qf_curlist = qi.qf_curlist;
    let qfl_type = qfl.qfl_type;
    let qf_type = qfl.qf_items[(qf_index - 1) as usize].qf_type;

    if qf_type == 1
        && (!bt_help(curwin_mut().buf_opt()) || g_st().cmdmod.cmod_tab != 0)
        && !jump_to_help_window(qi.borrow_mut(), newwin, opened_window)
    {
        return Some(QfStatus::Fail);
    }
    if old_qf_curlist != qi.qf_curlist
        || old_changetick != qfl.qf_changedtick
        || !is_qf_entry_present(qfl.borrow(), qf_index)
    {
        if qfl_type == QFLT_QUICKFIX {
            emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        } else {
            emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
        }
        return Some(QfStatus::Abort);
    }

    let qf_fnum = qfl.qf_items[(qf_index - 1) as usize].qf_fnum;
    if bt_quickfix(Some(crate::globals::curbuf_mut())) && !*opened_window {
        if qf_fnum == 0 {
            return None;
        }

        if !qf_jump_to_usable_window(qf_fnum, newwin, opened_window) {
            return Some(QfStatus::Fail);
        }
    }
    if old_qf_curlist != qi.qf_curlist
        || old_changetick != qfl.qf_changedtick
        || !is_qf_entry_present(qfl.borrow(), qf_index)
    {
        if qfl_type == QFLT_QUICKFIX {
            emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
        } else {
            emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
        }
        return Some(QfStatus::Abort);
    }

    Some(QfStatus::Ok)
}

fn qf_jump_to_buffer(
    qi_ref: &mut QfInfo,
    qf_index: i32,
    forceit: i32,
    prev_winid: i32,
    opened_window: &mut bool,
    openfold: i32,
    print_message: bool,
) -> QfStatus {
    let qi = crate::globals::GraphPtr::from(qi_ref);
    let old_curbuf = g_st().curbuf;
    let old_lnum = curwin_mut().w_cursor.lnum;
    let retval;
    let qfl = crate::globals::GraphPtr::<QfList>::from_ref(qf_get_curlist(qi.borrow_mut()));

    if qfl.qf_items[(qf_index - 1) as usize].qf_fnum != 0 {
        retval = qf_jump_edit_buffer(
            qi.borrow_mut(),
            qf_index,
            forceit,
            prev_winid,
            opened_window,
        );
        if retval != QfStatus::Ok {
            return retval;
        }
    } else {
        retval = QfStatus::Ok;
    }

    if g_st().curbuf == old_curbuf {
        setpcmark();
    }

    let qfp = &mut (qfl.borrow_mut().qf_items)[(qf_index - 1) as usize];
    qf_jump_goto_line(
        qfp.qf_lnum,
        qfp.qf_col,
        qfp.qf_viscol,
        qfp.qf_pattern.as_deref(),
    );

    if (opt_vars().fdo_flags & K_OPT_FDO_FLAG_QUICKFIX) != 0 && openfold != 0 {
        fold_open_cursor();
    }
    if print_message {
        qf_jump_print_msg(
            qi.borrow_mut(),
            qf_index,
            qfp,
            g_st().curbuf == old_curbuf,
            old_lnum,
        );
    }

    retval
}

pub fn qf_jump(qi: Option<&mut QfInfo>, dir: i32, errornr: i32, forceit: bool) {
    qf_jump_newwin(qi, dir, errornr, forceit, false);
}

fn qf_jump_newwin(
    qi_opt: Option<&mut QfInfo>,
    dir: i32,
    errornr: i32,
    forceit: bool,
    newwin: bool,
) {
    let old_swb = opt_vars().p_swb.clone();
    let old_swb_flags = opt_vars().swb_flags;
    let old_key_typed = g_st().key_typed;

    let mut qi = qi_opt.map_or_else(
        || crate::globals::GraphPtr::from(ql_info_mut()),
        crate::globals::GraphPtr::from,
    );

    if qf_stack_empty(qi.opt()) || qf_list_empty(Some(&*(qf_get_curlist(qi.borrow_mut())))) {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    incr_quickfix_busy();

    let mut qfl = crate::globals::GraphPtr::<QfList>::from_ref(qf_get_curlist(qi.borrow_mut()));

    let mut qf_index = qfl.qf_index;
    let old_qf_index = qf_index;

    let qf_ptr = qf_get_entry(qfl.borrow_mut(), errornr, dir, &mut qf_index);
    if qf_ptr.is_some() {
        qfl.qf_index = qf_index;

        let print_message = !qf_win_pos_update(qi.borrow_mut(), old_qf_index);
        let prev_winid = curwin_mut().handle;
        let mut opened_window = false;
        let retval = qf_jump_open_window(qi.borrow_mut(), qf_index, newwin, &mut opened_window);
        if retval == Some(QfStatus::Fail) {
            qf_index = old_qf_index;
        } else if retval == Some(QfStatus::Abort) {
            qi = crate::globals::GraphPtr::NULL;
        } else if retval.is_some() {
            let retval = qf_jump_to_buffer(
                qi.borrow_mut(),
                qf_index,
                i32::from(forceit),
                prev_winid,
                &mut opened_window,
                i32::from(old_key_typed),
                print_message,
            );
            if retval == QfStatus::Abort {
                qi = crate::globals::GraphPtr::NULL;
            }
            if retval != QfStatus::Ok {
                if opened_window {
                    win_close(curwin_mut(), true, false);
                }
                if !qi.is_null() && qfl.qf_items[(qf_index - 1) as usize].qf_fnum != 0 {
                    qf_index = old_qf_index;
                }
            }
        }
    } else {
        qf_index = old_qf_index;
    }

    if !qi.is_null() {
        qfl.qf_index = qf_index;
    }
    if opt_vars().p_swb != old_swb && opt_vars().p_swb.is_null() {
        opt_vars().p_swb = old_swb;
        opt_vars().swb_flags = old_swb_flags;
    }
    decr_quickfix_busy();
}

fn qf_list_entry(qfp: &mut Qfline, qf_idx: i32, cursel: bool) {
    let mut fname: Option<&CStr> = None;
    if let Some(ref module) = qfp.qf_module
        && !module.as_bytes().is_empty()
    {
        format_to_buf!(&mut g_st().iobuff, c"%2d %s", qf_idx, module);
    } else {
        if qfp.qf_fnum != 0
            && let Some(buf) = buflist_findnr_opt(qfp.qf_fnum)
        {
            fname = qfp.qf_fname.as_deref().or_else(|| buf.fname_opt());
            if qfp.qf_type == 1
                && let Some(f) = fname
            {
                fname = Some(path_tail(f));
            }
        }
        if let Some(f) = fname {
            format_to_buf!(&mut g_st().iobuff, c"%2d %s", qf_idx, f);
        } else {
            format_to_buf!(&mut g_st().iobuff, c"%2d", qf_idx);
        }
    }

    let mut filter_entry = true;
    if let Some(ref module) = qfp.qf_module
        && !module.as_bytes().is_empty()
    {
        filter_entry &= message_filtered(module);
    }
    if filter_entry && let Some(f) = fname {
        filter_entry &= message_filtered(f);
    }
    if filter_entry && let Some(ref pat) = qfp.qf_pattern {
        filter_entry &= message_filtered(pat);
    }
    if filter_entry {
        filter_entry &= message_filtered(&qfp.qf_text);
    }
    if filter_entry {
        return;
    }

    if g_st().msg_col > 0 {
        msg_putchar(b'\n' as i32);
    }
    msg_outtrans(
        crate::memory::bytes_as_cstr(g_st().iobuff_bytes()),
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
    qfga_concat(gap, qf_types(i32::from(qfp.qf_type), qfp.qf_nr).as_bytes());
    gap.push(0);
    if !crate::memory::bytes_until_nul(gap).is_empty() {
        msg_puts_hl(
            crate::memory::bytes_as_cstr(gap),
            qf_st().qf_line_hl_id,
            false,
        );
    }
    msg_puts_hl(c":", qf_st().qf_sep_hl_id, false);
    if let Some(ref pat) = qfp.qf_pattern {
        gap = qfga_get();
        qf_fmt_text(gap, pat.as_bytes());
        gap.push(0);
        msg_puts(crate::memory::bytes_as_cstr(gap));
        msg_puts_hl(c":", qf_st().qf_sep_hl_id, false);
    }
    msg_puts(c" ");

    gap = qfga_get();
    let text = if fname.is_some() || qfp.qf_lnum != 0 {
        crate::charset::skipwhite_slice(qfp.qf_text.as_bytes())
    } else {
        qfp.qf_text.as_bytes()
    };
    qf_fmt_text(gap, text);
    gap.push(0);
    msg_prt_line(gap, false);
}

pub fn qf_list(eap: &mut Exarg) {
    let arg = eap.arg.clone();
    let mut all = eap.forceit;
    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

    if qf_stack_empty(Some(qi)) || qf_list_empty(Some(&*(qf_get_curlist(qi)))) {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    let arg_bytes = (arg).as_bytes();
    let mut rem = arg_bytes;
    let plus = if let Some(after_plus) = rem.strip_prefix(b"+") {
        rem = after_plus;
        true
    } else {
        false
    };
    let mut idx1: i32 = 1;
    let mut idx2: i32 = -1;
    if !get_list_range(&mut rem, &mut idx1, &mut idx2) || !rem.is_empty() {
        semsg!(
            gettext(E_TRAILING_ARG),
            arg.wrapping_add(arg_bytes.len() - rem.len())
        );
        return;
    }
    let qfl = qf_get_curlist(qi);
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

    shorten_fnames(false);

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
        if (qfp.qf_valid || all) && idx1 <= i && i <= idx2 {
            qf_list_entry(qfp, i, i == qfl.qf_index);
        }
        os_breakcheck();
    }
    qfga_clear();
}

fn qf_fmt_text(gap: &mut Vec<u8>, text: &[u8]) {
    let mut rem = crate::memory::bytes_until_nul(text);
    while let Some((&b, rest)) = rem.split_first() {
        if b == b'\n' {
            gap.push(b' ');
            rem = rest;
            while let Some((&nb, nrest)) = rem.split_first() {
                if !ascii_iswhite(i32::from(nb)) && nb != b'\n' {
                    break;
                }
                rem = nrest;
            }
        } else {
            gap.push(b);
            rem = rest;
        }
    }
}

fn qf_range_text(gap: &mut Vec<u8>, qfp: &Qfline) {
    let mut buf_size = format_to_buf_safelen!(&mut g_st().iobuff, c"%d", qfp.qf_lnum);

    if qfp.qf_end_lnum > 0 && qfp.qf_lnum != qfp.qf_end_lnum {
        buf_size += format_to_buf_safelen!(&mut g_st().iobuff[buf_size..], c"-%d", qfp.qf_end_lnum);
    }
    if qfp.qf_col > 0 {
        buf_size += format_to_buf_safelen!(&mut g_st().iobuff[buf_size..], c" col %d", qfp.qf_col);
        if qfp.qf_end_col > 0 && qfp.qf_col != qfp.qf_end_col {
            buf_size +=
                format_to_buf_safelen!(&mut g_st().iobuff[buf_size..], c"-%d", qfp.qf_end_col);
        }
    }
    qfga_concat(gap, &g_st().iobuff_bytes()[..buf_size]);
}

fn qf_msg(qi: &mut QfInfo, which: i32, lead: &CStr) {
    let qfl = &(&qi.qf_lists)[which as usize];
    let title = qfl.qf_title.as_ref();
    let count = qfl.count();
    let mut buf = [0u8; IOSIZE];

    format_to_buf!(
        &mut buf,
        gettext(c"%serror list %d of %d; %d errors "),
        lead,
        which + 1,
        qi.qf_listcount,
        count,
    );

    if let Some(title) = title {
        let len = crate::memory::bytes_until_nul(&buf).len();
        if len < 34 {
            buf[len..34].fill(b' ');
            buf[34] = 0;
        }
        crate::memory::append_bytes_to_c_buf(&mut buf, title.as_bytes());
    }
    trunc_string(None, &mut buf, g_st().columns_global - 1);
    msg(crate::memory::bytes_as_cstr(&buf), 0);
}

pub fn qf_age(eap: &mut Exarg) {
    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

    let mut count = if eap.addr_count != 0 { eap.line2 } else { 1 };
    while count > 0 {
        count -= 1;
        if eap.cmdidx == Cmdidx::CmdColder || eap.cmdidx == Cmdidx::CmdLolder {
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
    let qi = qf_cmd_get_stack_opt(eap, false);

    if eap.addr_count > 0 {
        let Some(qi) = qi else {
            emsg(gettext(E_LOCLIST));
            return;
        };

        if eap.line2 > 0 && eap.line2 <= qi.qf_listcount {
            let curlist = eap.line2 - 1;
            qi.qf_curlist = curlist;
            qf_msg(qi, curlist, c"");
            qf_update_buffer(qi, 0);
        } else {
            emsg(gettext(E_INVRANGE));
        }
        return;
    }

    if qf_stack_empty(qi.as_deref()) {
        msg(gettext(c"No entries"), 0);
    } else if let Some(qi) = qi {
        for i in 0..qi.qf_listcount {
            let lead = if i == qi.qf_curlist { c"> " } else { c"  " };
            qf_msg(&mut *qi, i, lead);
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
    let buf_has_flag = if wp.is_none() {
        BUF_HAS_QF_ENTRY
    } else {
        BUF_HAS_LL_ENTRY
    };

    if (buf.b_has_qf_entry & buf_has_flag) == 0 {
        return false;
    }
    let qi = if let Some(wp) = wp {
        let Some(ll) = wp.w_llist.opt_mut() else {
            return false;
        };
        ll
    } else {
        ql_info_mut()
    };

    let mut found_one = false;
    for idx in 0..qi.qf_listcount {
        let qfl = qf_get_list(qi, idx);
        for qfp in &mut qfl.qf_items {
            if g_st().got_int {
                break;
            }
            if qfp.qf_fnum == buf.b_fnum {
                found_one = true;
                if !qfp.qf_cleared {
                    if qfp.qf_lnum >= line1 && qfp.qf_lnum <= line2 {
                        if amount == MAXLNUM {
                            qfp.qf_cleared = true;
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
    let len = format_to_buf_safelen!(&mut buf, c"%s %3d", p, nr);
    crate::memory::bytes_to_cstring(&buf[..len])
}

pub fn qf_view_result(split: bool) {
    let qi = if is_ll_window(curwin_mut()) {
        let Some(ll) = get_loc_list_opt(curwin_mut()) else {
            emsg(gettext(E_NO_ERRORS));
            return;
        };
        ll
    } else {
        ql_info_mut()
    };

    if qf_list_empty(Some(&*(qf_get_curlist(qi)))) {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    if split {
        qf_jump_newwin(Some(qi), 0, curwin_mut().w_cursor.lnum, false, true);
        do_cmdline_cmd(c"clearjumps");
        return;
    }

    do_cmdline_cmd(if is_ll_window(curwin_mut()) {
        c".ll"
    } else {
        c".cc"
    });
}

pub fn ex_cwindow(eap: &mut Exarg) {
    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

    let qfl = qf_get_curlist(qi);
    let win = qf_find_win(qi);

    if qf_stack_empty(Some(qi)) || qfl.qf_nonevalid || qf_list_empty(Some(&*qfl)) {
        if win.is_some() {
            ex_cclose(eap);
        }
    } else if win.is_none() {
        ex_copen(eap);
    }
}

pub fn ex_cclose(eap: &mut Exarg) {
    let Some(qi) = qf_cmd_get_stack_opt(eap, false) else {
        return;
    };

    if let Some(win) = qf_find_win(qi) {
        win_close(win, false, false);
    }
}

fn qf_goto_cwindow(qi: &QfInfo, resize: bool, sz: i32, vertsplit: bool) -> bool {
    let Some(win) = qf_find_win(qi) else {
        return false;
    };

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
    set_option_value_give_err(K_OPT_BUFTYPE, cstr_as_obj(Some(c"quickfix")), OPT_LOCAL);
    set_option_value_give_err(K_OPT_BUFHIDDEN, cstr_as_obj(Some(c"hide")), OPT_LOCAL);
    reset_binding(curwin_mut());
    curwin_mut().w_onebuf_opt.wo_diff = false;
    set_option_value_give_err(K_OPT_FOLDMETHOD, cstr_as_obj(Some(c"manual")), OPT_LOCAL);
}

fn qf_open_new_cwindow(qi: &mut QfInfo, height: i32) -> bool {
    let mut oldwin = curwin_ptr();
    let prevtab = g_st().curtab;
    let mut flags: i32 = 0;

    let qf_buf_fnum = qf_find_buf(qi).map(|b| b.b_fnum);
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
        curwin_mut().w_llist_ref = qi.qf_id_handle;
        qi.qf_refcount += 1;
    }

    if oldwin != curwin_ptr() {
        oldwin = crate::globals::WinPtr::NULL;
    }
    if let Some(fnum) = qf_buf_fnum {
        if !do_ecmd(
            fnum,
            None,
            None,
            None,
            ECMD_ONE,
            ECMD_HIDE + ECMD_OLDBUF + ECMD_NOWINENTER,
            oldwin.opt_mut(),
        ) {
            return false;
        }
    } else {
        if !do_ecmd(
            0,
            None,
            None,
            None,
            ECMD_ONE,
            ECMD_HIDE + ECMD_NOWINENTER,
            oldwin.opt_mut(),
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
    curwin_mut().w_onebuf_opt.wo_wfh = true;
    if win_valid(win.opt()) {
        g_st().prevwin = win;
    }
    true
}

fn qf_set_title_var(qfl: &mut QfList) {
    if let Some(ref title) = qfl.qf_title {
        set_internal_string_var(c"w:quickfix_title", title);
    }
}

pub fn ex_copen(eap: &mut Exarg) {
    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

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
            qi,
            eap.addr_count != 0,
            height,
            (g_st().cmdmod.cmod_split & WSP_VERT) != 0,
        );
    }
    if !status && !qf_open_new_cwindow(qi, height) {
        decr_quickfix_busy();
        return;
    }

    let qfl = qf_get_curlist(qi);
    qf_set_title_var(qfl);
    let lnum = qfl.qf_index;

    qf_fill_buffer(Some(&mut *qfl), curbuf_mut(), 0, curwin_mut().handle);

    decr_quickfix_busy();

    curwin_mut().w_cursor.lnum = lnum;
    curwin_mut().w_cursor.col = 0;
    check_cursor(curwin_mut());
    update_topline(curwin_mut());
}

fn qf_win_goto(win: &mut WinInfoInfo, lnum: Linenr) {
    let old_curwin = curwin_ptr();

    set_curwin(&mut *win);
    g_st().curbuf = win.w_buffer;
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
    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

    if let Some(win_ref) = qf_find_win(qi) {
        let last_lnum = win_ref.buf_mut().b_ml.ml_line_count;
        if win_ref.w_cursor.lnum != last_lnum {
            qf_win_goto(win_ref, last_lnum);
        }
    }
}

pub fn qf_current_entry(wp: &mut WinInfoInfo) -> Linenr {
    let qi = if is_ll_window(&*wp) {
        wp.w_llist_ref.borrow_mut()
    } else {
        ql_info_mut()
    };

    qf_get_curlist(qi).qf_index
}

fn qf_win_pos_update(qi: &mut QfInfo, old_qf_index: i32) -> bool {
    let qf_index = qf_get_curlist(qi).qf_index;

    let win = qf_find_win(qi);
    let found = win.is_some();
    if let Some(win_ref) = win
        && qf_index <= win_ref.buf_mut().b_ml.ml_line_count
        && old_qf_index != qf_index
    {
        win_ref.w_redraw_top = old_qf_index.min(qf_index);
        win_ref.w_redraw_bot = old_qf_index.max(qf_index);
        qf_win_goto(win_ref, qf_index);
    }
    found
}

fn is_qf_win(win: &WinInfoInfo, qi: &QfInfo) -> bool {
    if buf_valid(win.buf_opt())
        && bt_quickfix(win.buf_opt())
        && ((is_qf_stack(qi) && win.w_llist_ref.is_null())
            || (is_ll_stack(qi) && win.w_llist_ref == qi))
    {
        return true;
    }
    false
}

fn qf_find_win(qi: &QfInfo) -> Option<&'static mut WinInfoInfo> {
    for win in win_iter_ptr(tab_firstwin(curtab_mut())) {
        if is_qf_win(&*win, qi) {
            return Some(&mut *win);
        }
    }
    None
}

fn qf_find_buf(qi: &mut QfInfo) -> Option<&'static mut FileBuf> {
    if qi.qf_bufnr != INVALID_QFBUFNR {
        if let Some(qfbuf) = buflist_findnr_opt(qi.qf_bufnr) {
            return Some(qfbuf);
        }
        qi.qf_bufnr = INVALID_QFBUFNR;
    }

    for tp in for_all_tabpages_mut() {
        for win in win_iter_mut(tab_firstwin(tp)) {
            if is_qf_win(win, &*qi) {
                return win.w_buffer.opt_mut();
            }
        }
    }

    None
}

fn qf_update_win_titlevar(qi: &mut QfInfo) {
    let qfl = qf_get_curlist(&mut *qi);
    let save_curwin = curwin_ptr();

    for tp in for_all_tabpages_mut() {
        for win in win_iter_ptr(tab_firstwin(tp)) {
            if is_qf_win(&*win, &*qi) {
                set_curwin(&mut *win);
                qf_set_title_var(qfl);
            }
        }
    }
    set_curwin(save_curwin);
}

fn qf_update_buffer(qi: &mut QfInfo, old_count: usize) {
    let Some(buf) = qf_find_buf(qi) else {
        return;
    };

    let old_line_count = buf.b_ml.ml_line_count;
    let old_endcol = ml_get_buf_len(&mut *buf, old_line_count);
    let old_bytecount = get_region_bytecount(&mut *buf, 1, old_line_count, 0, old_endcol);
    let mut qf_winid: i32 = 0;

    if is_ll_stack(&*qi) {
        let win = if curwin_mut().w_llist == qi {
            Some(curwin_mut())
        } else {
            qf_find_win_with_loclist(qi).or_else(|| qf_find_win(&*qi))
        };
        let Some(win) = win else {
            return;
        };
        qf_winid = win.handle;
    }

    incr_quickfix_busy();

    let mut aco: CtxSwitch = CtxSwitch::new();
    if old_count == 0 {
        ctx_switch(
            &mut aco,
            None,
            None,
            Some(crate::globals::GraphPtr::from(&*buf).borrow_mut()),
            0,
        );
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

    if let Some(win) = qf_find_win(&*qi)
        && old_line_count < win.w_botline
    {
        redraw_buf_later(&*buf, UPD_NOT_VALID);
    }

    decr_quickfix_busy();
}

fn qf_buf_add_line(
    _qfl: &QfList,
    buf: &mut FileBuf,
    lnum: Linenr,
    qfp: &Qfline,
    dirname: &mut [u8],
    qftf_str: Option<&CStr>,
    first_bufline: bool,
) -> bool {
    let gap = qfga_get();

    if let Some(qftf_str) = qftf_str.filter(|s| !s.to_bytes().is_empty()) {
        qfga_concat(gap, qftf_str.to_bytes());
    } else {
        if let Some(ref module) = qfp.qf_module {
            qfga_concat(gap, module.as_bytes());
        } else if qfp.qf_fnum != 0
            && let Some(errbuf) = buflist_findnr_opt(qfp.qf_fnum)
            && errbuf.fname_opt().is_some()
        {
            if qfp.qf_type == 1 {
                qfga_concat(gap, path_tail(errbuf.fname_cstr()).to_bytes());
            } else {
                if first_bufline
                    && (errbuf.b_sfname.is_none() || path_is_absolute(errbuf.sfname_bytes()))
                {
                    if dirname[0] == 0 {
                        os_dirname(dirname);
                    }
                    shorten_buf_fname(errbuf, crate::memory::bytes_as_cstr(dirname), 0);
                }
                qfga_concat(
                    gap,
                    qfp.qf_fname
                        .as_ref()
                        .map_or_else(|| errbuf.fname_bytes(), |s| s.as_bytes()),
                );
            }
        }

        gap.push(b'|');

        if qfp.qf_lnum > 0 {
            qf_range_text(gap, qfp);
            qfga_concat(gap, qf_types(i32::from(qfp.qf_type), qfp.qf_nr).as_bytes());
        } else if let Some(ref pat) = qfp.qf_pattern {
            qf_fmt_text(gap, pat.as_bytes());
        }
        gap.push(b'|');
        gap.push(b' ');

        let text_arg = if gap.len() > 3 {
            crate::charset::skipwhite_slice(qfp.qf_text.as_bytes())
        } else {
            qfp.qf_text.as_bytes()
        };
        qf_fmt_text(gap, text_arg);
    }

    gap.push(0);
    let text_len = gap.len().saturating_sub(1);
    if !ml_append_buf(&mut *buf, lnum, &gap[..text_len], gap.len() as Colnr, false) {
        return false;
    }

    true
}

fn call_qftf_func(
    qfl: &mut QfList,
    qf_winid: i32,
    start_idx: i32,
    end_idx: i32,
) -> Option<&'static mut TvList> {
    let mut qftf_list: Option<&'static mut TvList> = None;

    if qf_st().qftf_recursive {
        return None;
    }
    qf_st().qftf_recursive = true;

    let is_qf = is_qf_list(qfl);
    let qf_id = qfl.qf_id;
    let cb = if qfl.qf_qftf_cb.cb_type() != K_CALLBACK_NONE_CONST {
        &mut qfl.qf_qftf_cb
    } else {
        &mut opt_vars().p_qftf
    };
    if cb.cb_type() != K_CALLBACK_NONE_CONST {
        let mut args: [TypVal; 1] = [TypVal::new(); 1];
        let mut rettv: TypVal = TypVal::new();

        let dict = tv_dict_alloc_lock(VAR_FIXED);
        tv_dict_add_nr(dict, c"quickfix", Varnumber::from(is_qf));
        tv_dict_add_nr(dict, c"winid", Varnumber::from(qf_winid));
        tv_dict_add_nr(dict, c"id", Varnumber::from(qf_id));
        tv_dict_add_nr(dict, c"start_idx", Varnumber::from(start_idx));
        tv_dict_add_nr(dict, c"end_idx", Varnumber::from(end_idx));
        dict.dv_refcount += 1;
        args[0].v_type = VarType::VarDict;
        args[0].vval = crate::eval::typval::TypvalVval::Dict(dict.dv_id);

        g_st().textlock += 1;
        if callback_call(cb, 1, &mut args, &mut rettv) {
            if rettv.v_type == VarType::VarList {
                tv_list_ref(rettv.vval.v_list_opt());
                qftf_list = rettv.vval.v_list().opt_mut();
            }
            tv_clear(&mut rettv);
        }
        g_st().textlock -= 1;
        tv_dict_unref(Some(dict));
    }

    qf_st().qftf_recursive = false;
    qftf_list
}

fn qf_fill_buffer(qfl: Option<&mut QfList>, buf: &mut FileBuf, old_count: usize, qf_winid: i32) {
    let old_key_typed = g_st().key_typed;

    if old_count == 0 {
        if g_st().curbuf != buf {
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
        let qftf_list_ptr = qftf_list
            .as_deref()
            .map_or(ptr::null_mut(), |l| ptr::from_ref(l).cast_mut());
        let mut qftf_iter = tv_list_iter(qftf_list.as_deref());

        let mut prev_bufnr: i32 = -1;
        let mut invalid_val = false;

        while (lnum as usize) < qfl.qf_items.len() {
            let mut qftf_str: Option<&CStr> = None;

            if let Some(qftf_li) = qftf_iter.next()
                && !invalid_val
            {
                if let Some(s) = tv_get_string_chk(&qftf_li.li_tv) {
                    qftf_str = Some(s);
                } else {
                    invalid_val = true;
                }
            }

            let qfp_fnum = qfl.qf_items[lnum as usize].qf_fnum;
            if !qf_buf_add_line(
                qfl,
                &mut *buf,
                lnum,
                &qfl.qf_items[lnum as usize],
                &mut dirname,
                qftf_str,
                prev_bufnr != qfp_fnum,
            ) {
                break;
            }
            prev_bufnr = qfp_fnum;
            lnum += 1;
        }
        if old_count == 0 {
            ml_delete(lnum + 1);
        }

        tv_list_unref(crate::globals::GraphPtr::of(qftf_list_ptr).opt_mut());
        qfga_clear();
    }

    check_lnums(true);

    if old_count == 0 {
        curbuf_mut().b_ro_locked += 1;
        set_option_value_give_err(K_OPT_FILETYPE, cstr_as_obj(Some(c"qf")), OPT_LOCAL);
        curbuf_mut().b_p_ma = false;

        curbuf_mut().b_keep_filetype = true;
        apply_autocmds(
            EVENT_BUFREADPOST,
            Some(c"quickfix"),
            None,
            false,
            curbuf_opt_mut(),
        );
        apply_autocmds(
            EVENT_BUFWINENTER,
            Some(c"quickfix"),
            None,
            false,
            curbuf_opt_mut(),
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

fn qf_jump_first(qi: &mut QfInfo, save_qfid: u32, forceit: bool) {
    if !qf_restore_list(qi, save_qfid) {
        return;
    }

    if !check_can_set_curbuf_forceit(forceit) {
        return;
    }

    if !qf_list_empty(Some(&*(qf_get_curlist(qi)))) {
        qf_jump(Some(qi), 0, 0, forceit);
    }
}

pub fn grep_internal(cmdidx: Cmdidx) -> i32 {
    let ov = opt_vars();
    let gp = if curbuf_mut().b_p_gp.is_empty() {
        ov.p_gp.as_bytes()
    } else {
        curbuf_mut().b_p_gp.as_bytes()
    };
    i32::from(
        (cmdidx == Cmdidx::CmdGrep
            || cmdidx == Cmdidx::CmdLgrep
            || cmdidx == Cmdidx::CmdGrepadd
            || cmdidx == Cmdidx::CmdLgrepadd)
            && b"internal" == gp,
    )
}

fn make_get_auname(cmdidx: Cmdidx) -> Option<&'static CStr> {
    match cmdidx {
        Cmdidx::CmdMake => Some(c"make"),
        Cmdidx::CmdLmake => Some(c"lmake"),
        Cmdidx::CmdGrep => Some(c"grep"),
        Cmdidx::CmdLgrep => Some(c"lgrep"),
        Cmdidx::CmdGrepadd => Some(c"grepadd"),
        Cmdidx::CmdLgrepadd => Some(c"lgrepadd"),
        _ => None,
    }
}

fn make_get_fullcmd(makecmd: &CStr, fname: &CStr) -> std::ffi::CString {
    let mut len = opt_vars().p_shq.as_bytes().len() * 2 + makecmd.to_bytes().len() + 1;
    if !opt_vars().p_sp.is_empty() {
        len += opt_vars().p_sp.as_bytes().len() + fname.to_bytes().len() + 3;
    }
    let mut cmd_vec: Vec<u8> = vec![0; len];
    format_to_buf!(
        &mut cmd_vec,
        c"%s%s%s",
        &opt_vars().p_shq,
        makecmd,
        &opt_vars().p_shq
    );

    if !opt_vars().p_sp.is_empty() {
        append_redir(&mut cmd_vec, opt_vars().p_sp.as_cstr(), fname);
    }

    if g_st().msg_col == 0 {
        g_st().msg_didout = false;
    }
    msg_start();
    msg_puts(c":!");
    msg_outtrans(crate::memory::bytes_as_cstr(&cmd_vec), 0, false);

    crate::memory::cstring_until_nul(&cmd_vec)
}

pub fn ex_make(eap: &mut Exarg) {
    let enc = if !curbuf_mut().b_p_menc.is_empty() {
        curbuf_mut().b_p_menc.to_owned()
    } else {
        opt_vars().p_menc.to_owned()
    };

    if grep_internal(eap.cmdidx) != 0 {
        ex_vimgrep(eap);
        return;
    }

    let au_name = make_get_auname(eap.cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            Some(au_name),
            curbuf_mut().fname_opt(),
            true,
            curbuf_opt_mut(),
        )
        && aborting()
    {
        return;
    }

    let mut wp = crate::globals::GraphPtr::<WinInfoInfo>::NULL;
    if is_loclist_cmd(eap.cmdidx) {
        wp = crate::globals::GraphPtr::<WinInfoInfo>::of(curwin_ptr());
    }

    autowrite_all();
    let Some(fname) = get_mef_name() else {
        return;
    };
    os_remove(&fname);

    let cmd = make_get_fullcmd(eap.arg.as_cstr(), &fname);

    do_shell(Some(cmd.as_c_str()), 0);

    incr_quickfix_busy();

    let gefm_owned;
    let errorformat: &CStr = if eap.cmdidx != Cmdidx::CmdMake && eap.cmdidx != Cmdidx::CmdLmake {
        if !curbuf_mut().b_p_gefm.is_empty() {
            gefm_owned = curbuf_mut().b_p_gefm.to_owned();
            &gefm_owned
        } else {
            opt_vars().borrow().p_gefm.as_cstr()
        }
    } else {
        opt_vars().borrow().p_efm.as_cstr()
    };

    let newlist = eap.cmdidx != Cmdidx::CmdGrepadd && eap.cmdidx != Cmdidx::CmdLgrepadd;

    let res = qf_init(
        wp.opt_mut(),
        Some(&fname),
        errorformat,
        i32::from(newlist),
        Some(&qf_cmdtitle(eap.cmdline_cstr())),
        Some(&enc),
    );

    let qi = if !wp.is_null() {
        get_loc_list_opt(wp.borrow())
    } else {
        Some(ql_info_mut())
    };
    if let Some(qi) = qi {
        if res >= 0 {
            qf_list_changed(qf_get_curlist(qi));
        }
        let save_qfid = qf_get_curlist(qi).qf_id;
        let qi_ptr = ptr::from_mut(qi);
        if let Some(au_name) = au_name {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                Some(au_name),
                curbuf_mut().fname_opt(),
                true,
                curbuf_opt_mut(),
            );
        }
        if res > 0 && !eap.forceit && qflist_valid(wp.opt(), save_qfid) {
            qf_jump_first(
                crate::globals::GraphPtr::of(qi_ptr).borrow_mut(),
                save_qfid,
                false,
            );
        }
    }

    decr_quickfix_busy();
    os_remove(&fname);
}

fn get_mef_name() -> Option<std::ffi::CString> {
    let p_mef = opt_vars().p_mef.to_owned();
    let mef_bytes = p_mef.as_bytes();
    if mef_bytes.is_empty() {
        let name = vim_tempname();
        if name.is_none() {
            emsg(gettext(E_NOTMP));
        }
        return name;
    }

    let Some(off_idx) = mef_bytes.windows(2).position(|w| w == b"##") else {
        return Some(opt_vars().p_mef.to_owned());
    };

    loop {
        if qf_st().mef_start == -1 {
            qf_st().mef_start = os_get_pid() as i32;
        } else {
            qf_st().mef_off += 19;
        }
        let mut name_vec: Vec<u8> = Vec::with_capacity(mef_bytes.len() + 30);
        name_vec.extend_from_slice(&mef_bytes[..off_idx]);
        name_vec.extend_from_slice(format!("{}{}", qf_st().mef_start, qf_st().mef_off).as_bytes());
        name_vec.extend_from_slice(&mef_bytes[off_idx + 2..]);
        name_vec.push(0);
        let name_cstr = crate::memory::bytes_as_cstr(&name_vec);
        let mut file_info = FileInfo::new();
        let file_or_link_found = os_fileinfo_link(name_cstr, &mut file_info);
        if !file_or_link_found {
            return Some(name_cstr.to_owned());
        }
    }
}

pub fn qf_get_size(eap: &mut Exarg) -> usize {
    let Some(qi) = qf_cmd_get_stack_opt(eap, false) else {
        return 0;
    };
    qf_get_curlist(qi).qf_items.len()
}

pub fn qf_get_valid_size(eap: &mut Exarg) -> usize {
    let Some(qi) = qf_cmd_get_stack_opt(eap, false) else {
        return 0;
    };

    let mut prev_fnum: i32 = 0;
    let mut sz: usize = 0;
    let qfl = qf_get_curlist(qi);
    for qfp in &qfl.qf_items {
        if g_st().got_int {
            break;
        }
        if qfp.qf_valid {
            if eap.cmdidx == Cmdidx::CmdCdo || eap.cmdidx == Cmdidx::CmdLdo {
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
    let Some(qi) = qf_cmd_get_stack_opt(eap, false) else {
        return 0;
    };

    assert!(qf_get_curlist(qi).qf_index >= 0);
    qf_get_curlist(qi).qf_index as usize
}

pub fn qf_get_cur_valid_idx(eap: &mut Exarg) -> i32 {
    let Some(qi) = qf_cmd_get_stack_opt(eap, false) else {
        return 1;
    };

    let qfl = qf_get_curlist(qi);
    if !qf_list_has_valid_entries(qfl) {
        return 1;
    }

    let mut prev_fnum: i32 = 0;
    let mut eidx: i32 = 0;
    assert!(qfl.qf_index >= 0);
    let limit = (qfl.qf_index as usize).min(qfl.qf_items.len());
    for qfp in &(&qfl.qf_items)[..limit] {
        if qfp.qf_valid {
            if eap.cmdidx == Cmdidx::CmdCfdo || eap.cmdidx == Cmdidx::CmdLfdo {
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
        if qfp.qf_valid {
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
    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

    let mut errornr: i32 = if eap.addr_count > 0 {
        eap.line2
    } else {
        match eap.cmdidx {
            Cmdidx::CmdCc | Cmdidx::CmdLl => 0,
            Cmdidx::CmdCrewind | Cmdidx::CmdLrewind | Cmdidx::CmdCfirst | Cmdidx::CmdLfirst => 1,
            _ => 32767,
        }
    };

    if eap.cmdidx == Cmdidx::CmdCdo
        || eap.cmdidx == Cmdidx::CmdLdo
        || eap.cmdidx == Cmdidx::CmdCfdo
        || eap.cmdidx == Cmdidx::CmdLfdo
    {
        let n = if eap.addr_count > 0 {
            assert!(eap.line1 >= 0);
            eap.line1 as usize
        } else {
            1
        };
        let valid_entry = qf_get_nth_valid_entry(
            qf_get_curlist(qi),
            n,
            eap.cmdidx == Cmdidx::CmdCfdo || eap.cmdidx == Cmdidx::CmdLfdo,
        );
        assert!(valid_entry <= i32::MAX as usize);
        errornr = valid_entry as i32;
    }

    qf_jump(Some(qi), 0, errornr, eap.forceit);
}

pub fn ex_cnext(eap: &mut Exarg) {
    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

    let errornr = if eap.addr_count > 0
        && eap.cmdidx != Cmdidx::CmdCdo
        && eap.cmdidx != Cmdidx::CmdLdo
        && eap.cmdidx != Cmdidx::CmdCfdo
        && eap.cmdidx != Cmdidx::CmdLfdo
    {
        eap.line2
    } else {
        1
    };

    let dir = match eap.cmdidx {
        Cmdidx::CmdCprevious | Cmdidx::CmdLprevious | Cmdidx::CmdCnext | Cmdidx::CmdLnext => {
            BACKWARD
        }
        Cmdidx::CmdCnfileVariant | Cmdidx::CmdLnfileVariant | Cmdidx::CmdCfdo | Cmdidx::CmdLfdo => {
            FORWARD_FILE
        }
        Cmdidx::CmdCpfile | Cmdidx::CmdLpfile | Cmdidx::CmdCnfile | Cmdidx::CmdLnfile => {
            BACKWARD_FILE
        }
        _ => FORWARD,
    };

    qf_jump(Some(qi), dir, errornr, eap.forceit);
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

    let buf_has_flag = if eap.cmdidx == Cmdidx::CmdCabove
        || eap.cmdidx == Cmdidx::CmdCbelow
        || eap.cmdidx == Cmdidx::CmdCbefore
        || eap.cmdidx == Cmdidx::CmdCafter
    {
        BUF_HAS_QF_ENTRY
    } else {
        BUF_HAS_LL_ENTRY
    };

    if (curbuf_mut().b_has_qf_entry & buf_has_flag) == 0 {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    let Some(qi) = qf_cmd_get_stack_opt(eap, true) else {
        return;
    };

    let qfl = qf_get_curlist(qi);
    if !qf_list_has_valid_entries(qfl) {
        emsg(gettext(E_NO_ERRORS));
        return;
    }

    let dir = if eap.cmdidx == Cmdidx::CmdCbelow
        || eap.cmdidx == Cmdidx::CmdLbelow
        || eap.cmdidx == Cmdidx::CmdCafter
        || eap.cmdidx == Cmdidx::CmdLafter
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
        eap.cmdidx == Cmdidx::CmdCbelow
            || eap.cmdidx == Cmdidx::CmdLbelow
            || eap.cmdidx == Cmdidx::CmdCabove
            || eap.cmdidx == Cmdidx::CmdLabove,
    );

    if errornr > 0 {
        qf_jump(Some(qi), 0, errornr, false);
    } else {
        emsg(gettext(E_NO_MORE_ITEMS));
    }
}

fn cfile_get_auname(cmdidx: Cmdidx) -> Option<&'static CStr> {
    match cmdidx {
        Cmdidx::CmdCfile => Some(c"cfile"),
        Cmdidx::CmdCgetfile => Some(c"cgetfile"),
        Cmdidx::CmdCaddfile => Some(c"caddfile"),
        Cmdidx::CmdLfile => Some(c"lfile"),
        Cmdidx::CmdLgetfile => Some(c"lgetfile"),
        Cmdidx::CmdLaddfile => Some(c"laddfile"),
        _ => None,
    }
}

pub fn ex_cfile(eap: &mut Exarg) {
    let mut wp = crate::globals::GraphPtr::<WinInfoInfo>::NULL;

    let au_name = cfile_get_auname(eap.cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            Some(au_name),
            None,
            false,
            curbuf_opt_mut(),
        )
        && aborting()
    {
        return;
    }
    if !eap.arg.is_empty() {
        set_option_direct(K_OPT_ERRORFILE, cstr_as_obj(eap.arg.as_opt_cstr()), 0, 0);
    }

    let enc = if !curbuf_mut().b_p_menc.is_empty() {
        curbuf_mut().b_p_menc.to_owned()
    } else {
        opt_vars().p_menc.to_owned()
    };

    if is_loclist_cmd(eap.cmdidx) {
        wp = crate::globals::GraphPtr::<WinInfoInfo>::of(curwin_ptr());
    }

    incr_quickfix_busy();

    let p_ef = opt_vars().p_ef.to_owned();
    let res = qf_init(
        wp.opt_mut(),
        Some(&p_ef),
        opt_vars().borrow().p_efm.as_cstr(),
        i32::from(eap.cmdidx != Cmdidx::CmdCaddfile && eap.cmdidx != Cmdidx::CmdLaddfile),
        Some(&qf_cmdtitle(eap.cmdline_cstr())),
        Some(&enc),
    );
    let qi = if !wp.is_null() {
        let Some(ll) = get_loc_list_opt(wp.borrow()) else {
            decr_quickfix_busy();
            return;
        };
        ll
    } else {
        ql_info_mut()
    };
    if res >= 0 {
        qf_list_changed(qf_get_curlist(qi));
    }
    let save_qfid = qf_get_curlist(qi).qf_id;
    let qi_ptr = ptr::from_mut(qi);
    if let Some(au_name) = au_name {
        apply_autocmds(
            EVENT_QUICKFIXCMDPOST,
            Some(au_name),
            None,
            false,
            curbuf_opt_mut(),
        );
    }
    if res > 0
        && (eap.cmdidx == Cmdidx::CmdCfile || eap.cmdidx == Cmdidx::CmdLfile)
        && qflist_valid(wp.opt(), save_qfid)
    {
        qf_jump_first(
            crate::globals::GraphPtr::of(qi_ptr).borrow_mut(),
            save_qfid,
            eap.forceit,
        );
    }

    decr_quickfix_busy();
}

fn vgr_get_auname(cmdidx: Cmdidx) -> Option<&'static CStr> {
    match cmdidx {
        Cmdidx::CmdVimgrep => Some(c"vimgrep"),
        Cmdidx::CmdLvimgrep => Some(c"lvimgrep"),
        Cmdidx::CmdVimgrepadd => Some(c"vimgrepadd"),
        Cmdidx::CmdLvimgrepadd => Some(c"lvimgrepadd"),
        Cmdidx::CmdGrep => Some(c"grep"),
        Cmdidx::CmdLgrep => Some(c"lgrep"),
        Cmdidx::CmdGrepadd => Some(c"grepadd"),
        Cmdidx::CmdLgrepadd => Some(c"lgrepadd"),
        _ => None,
    }
}

fn vgr_init_regmatch(regmatch: &mut Regmmatch, s: Option<&CStr>) {
    regmatch.free_prog();

    if let Some(s) = s.filter(|s| !s.to_bytes().is_empty()) {
        regmatch.set_prog(vim_regcomp(s, RE_MAGIC));
    } else {
        let Some(last_pat) = last_search_pat() else {
            emsg(gettext(E_NOPREVRE));
            return;
        };
        regmatch.set_prog(vim_regcomp(&last_pat, RE_MAGIC));
    }

    regmatch.rmm_ic = opt_vars().p_ic;
    regmatch.rmm_maxcol = 0;
}

fn vgr_display_fname(fname: &CStr) {
    msg_start();
    match msg_strtrunc(fname, 1) {
        Some(p) => msg_outtrans(&p, 0, false),
        None => msg_outtrans(fname, 0, false),
    };
    msg_clr_eos();
    g_st().msg_didout = false;
    g_st().msg_nowait = true;
    g_st().msg_col = 0;
    ui_flush();
}

fn vgr_load_dummy_buf(
    fname: &CStr,
    dirname_start: &CStr,
    dirname_now: &mut [u8],
) -> Option<&'static mut FileBuf> {
    let save_ei = au_event_disable(c",Filetype");

    let save_mls = opt_vars().p_mls;
    opt_vars().p_mls = 0;

    let buf = load_dummy_buffer(fname, dirname_start, dirname_now);

    opt_vars().p_mls = save_mls;
    au_event_restore(Some(save_ei));

    buf
}

fn vgr_qflist_valid(
    wp: Option<&WinInfoInfo>,
    qi: &mut QfInfo,
    qfid: u32,
    title: Option<&CStr>,
) -> bool {
    if !qflist_valid(wp, qfid) {
        if wp.is_some() {
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
    fname: Option<&CStr>,
    buf: &mut FileBuf,
    spat: Option<&CStr>,
    regmatch: &mut Regmmatch,
    tomatch: &mut i32,
    duplicate_name: i32,
    flags: i32,
) -> bool {
    let mut found_match = false;
    let spat_bytes = spat.map_or(&[][..], CStr::to_bytes);
    let pat_len = spat_bytes.len().min(FUZZY_MATCH_MAX_LEN);

    let mut lnum: Linenr = 1;
    while lnum <= buf.b_ml.ml_line_count && *tomatch > 0 {
        let mut col: Colnr = 0;
        if (flags & VGR_FUZZY) == 0 {
            while vim_regexec_multi(
                regmatch,
                Some(curwin_mut()),
                &mut *buf,
                lnum,
                col,
                None,
                None,
            ) > 0
            {
                let mesg_cstr = ml_get_buf(buf, regmatch.startpos[0].lnum + lnum);
                if qf_add_entry(
                    qfl,
                    None,
                    fname,
                    None,
                    if duplicate_name != 0 { 0 } else { buf.b_fnum },
                    mesg_cstr,
                    regmatch.startpos[0].lnum + lnum,
                    regmatch.endpos[0].lnum + lnum,
                    regmatch.startpos[0].col + 1,
                    regmatch.endpos[0].col + 1,
                    false,
                    None,
                    0,
                    0,
                    None,
                    true,
                ) == QfStatus::Fail
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
            let str_cstr = ml_get_buf(buf, lnum);
            let str_bytes = str_cstr.to_bytes();
            let linelen = ml_get_buf_len(buf, lnum);
            let mut score: i32 = 0;
            let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];

            while fuzzy_match(
                &str_bytes[(col as usize).min(str_bytes.len())..],
                spat_bytes,
                false,
                &mut score,
                &mut matches,
            ) {
                if qf_add_entry(
                    qfl,
                    None,
                    fname,
                    None,
                    if duplicate_name != 0 { 0 } else { buf.b_fnum },
                    str_cstr,
                    lnum,
                    0,
                    matches[0] as Colnr + col + 1,
                    0,
                    false,
                    None,
                    0,
                    0,
                    None,
                    true,
                ) == QfStatus::Fail
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

fn vgr_jump_to_match(
    qi: &mut QfInfo,
    forceit: bool,
    redraw_for_dummy: &mut bool,
    first_match_buf: Option<&FileBuf>,
    target_dir: Option<&CStr>,
) {
    let buf = g_st().curbuf;
    qf_jump(Some(qi), 0, 0, forceit);
    if buf != g_st().curbuf {
        *redraw_for_dummy = false;
    }

    if first_match_buf.is_some_and(|b| g_st().curbuf == b)
        && let Some(target_dir) = target_dir
    {
        let mut ea: Exarg = Exarg::new();
        ea.arg = crate::strings::OwnedSlice::from_cstr(target_dir);
        ea.cmdidx = Cmdidx::CmdLcd;
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

    args.regmatch.free_prog();
    args.qf_title = Some(qf_cmdtitle(eap.cmdline_cstr()));
    args.tomatch = if eap.addr_count > 0 {
        eap.line2
    } else {
        MAXLNUM
    };

    let Some(p_cstr) = skip_vimgrep_pat(
        eap.arg.as_cstr(),
        Some(&mut args.spat),
        Some(&mut args.flags),
    ) else {
        emsg(gettext(E_INVALPAT));
        return false;
    };
    let p_rem = crate::charset::skipwhite_slice(p_cstr.to_bytes());

    vgr_init_regmatch(&mut args.regmatch, args.spat.as_deref());
    if args.regmatch.regprog.is_null() {
        return false;
    }

    if p_rem.is_empty() {
        emsg(gettext(c"E683: File name missing or invalid pattern"));
        return false;
    }

    if !get_arglist_exp(p_rem, &mut args.fnames, true) || args.fnames.is_empty() {
        emsg(gettext(E_NOMATCH));
        return false;
    }

    true
}

fn vgr_process_files(
    wp_in: Option<&WinInfoInfo>,
    qi_in: &mut QfInfo,
    cmd_args: &mut VgrArgs,
    redraw_for_dummy: &mut bool,
    first_match_buf: &mut Option<&'static mut FileBuf>,
    target_dir: &mut Option<std::ffi::CString>,
) -> bool {
    let wp = crate::globals::GraphPtr::from(wp_in);
    let qi = crate::globals::GraphPtr::from(qi_in);
    let mut status = false;
    let mut save_qfid = qf_get_curlist(qi.borrow_mut()).qf_id;
    let mut duplicate_name = false;

    let mut dirname_start = vec![0u8; MAXPATHL];
    let mut dirname_now = vec![0u8; MAXPATHL];

    os_dirname(&mut dirname_start);
    let dirname_start_cstr = crate::memory::bytes_as_cstr(&dirname_start);

    let mut seconds: time_t = 0;
    let mut fi: usize = 0;
    let mut aborted = false;
    while fi < cmd_args.fnames.len() && !g_st().got_int && cmd_args.tomatch > 0 {
        let fname_exp_cstr = cmd_args.fnames.as_slice()[fi].as_c_str();
        let fname_exp_opt = Some(fname_exp_cstr);
        let fname = path_try_shorten_fname(fname_exp_opt);
        let fname_cstr = fname.unwrap_or(c"");
        let now = os_time() as time_t;
        if now > seconds {
            seconds = now;
            vgr_display_fname(fname_cstr);
        }

        let mut buf = crate::globals::GraphPtr::from(buflist_findname_exp(fname_exp_opt));
        let using_dummy: bool;
        if buf.is_null() || buf.b_ml.ml_mfp.is_none() {
            duplicate_name = !buf.is_null();
            using_dummy = true;
            *redraw_for_dummy = true;
            buf = vgr_load_dummy_buf(fname_cstr, dirname_start_cstr, &mut dirname_now).into();
        } else {
            using_dummy = false;
        }

        if !vgr_qflist_valid(
            wp.opt(),
            qi.borrow_mut(),
            save_qfid,
            cmd_args.qf_title.as_deref(),
        ) {
            aborted = true;
            break;
        }

        save_qfid = qf_get_curlist(qi.borrow_mut()).qf_id;

        if buf.is_null() {
            if !g_st().got_int {
                smsg!(0, gettext(c"Cannot open file \"%s\""), fname_cstr);
            }
        } else {
            let found_match = vgr_match_buflines(
                qf_get_curlist(qi.borrow_mut()),
                fname,
                buf.borrow_mut(),
                cmd_args.spat.as_deref(),
                &mut cmd_args.regmatch,
                &mut cmd_args.tomatch,
                i32::from(duplicate_name),
                cmd_args.flags,
            );

            if using_dummy {
                if found_match && first_match_buf.is_none() {
                    *first_match_buf = Some(buf.borrow_mut());
                }
                let is_first_match = first_match_buf.as_deref().is_some_and(|first| buf == first);
                if duplicate_name {
                    wipe_dummy_buffer(buf.borrow_mut(), Some(dirname_start_cstr));
                    buf = crate::globals::BufPtr::NULL;
                } else if (g_st().cmdmod.cmod_flags & CMOD_HIDE) == 0
                    || matches!(buf.b_p_bh.first_byte(), b'u' | b'w' | b'd')
                {
                    if !found_match {
                        wipe_dummy_buffer(buf.borrow_mut(), Some(dirname_start_cstr));
                        buf = crate::globals::BufPtr::NULL;
                    } else if !is_first_match
                        || (cmd_args.flags & VGR_NOJUMP) != 0
                        || existing_swapfile(buf.borrow())
                    {
                        unload_dummy_buffer(buf.borrow_mut(), dirname_start_cstr);
                        buf.b_flags &= !BF_DUMMY;
                        buf = crate::globals::BufPtr::NULL;
                    }
                }

                if !buf.is_null() {
                    let buf = buf.borrow_mut();
                    buf.b_flags &= !BF_DUMMY;

                    if is_first_match
                        && target_dir.is_none()
                        && (dirname_start_cstr.to_bytes()
                            != crate::memory::bytes_until_nul(&dirname_now))
                    {
                        *target_dir = Some(crate::memory::cstring_until_nul(&dirname_now));
                    }

                    let mut aco: CtxSwitch = CtxSwitch::new();
                    let b_p_ft = buf.b_p_ft.to_owned();
                    let b_fname = buf.fname_opt().map(CStr::to_owned);
                    ctx_switch(&mut aco, None, None, Some(&mut *buf), 0);
                    apply_autocmds(
                        EVENT_FILETYPE,
                        Some(&b_p_ft),
                        b_fname.as_deref(),
                        true,
                        Some(&mut *buf),
                    );
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

pub fn ex_vimgrep(eap: &mut Exarg) {
    if !check_can_set_curbuf_forceit(eap.forceit) {
        return;
    }

    let au_name = vgr_get_auname(eap.cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            Some(au_name),
            curbuf_mut().fname_opt(),
            true,
            curbuf_opt_mut(),
        )
        && aborting()
    {
        return;
    }

    let mut wp: Option<&'static mut WinInfoInfo> = None;
    let qi = qf_cmd_get_or_alloc_stack(eap, &mut wp);
    let wp_ptr = crate::globals::GraphPtr::from(wp.as_deref());
    let mut target_dir: Option<std::ffi::CString> = None;
    let mut args = VgrArgs::new();
    if vgr_process_args(eap, &mut args) {
        if (eap.cmdidx != Cmdidx::CmdGrepadd
            && eap.cmdidx != Cmdidx::CmdLgrepadd
            && eap.cmdidx != Cmdidx::CmdVimgrepadd
            && eap.cmdidx != Cmdidx::CmdLvimgrepadd)
            || qf_stack_empty(Some(qi))
        {
            qf_new_list(qi, args.qf_title.as_deref());
        }

        incr_quickfix_busy();

        let mut redraw_for_dummy = false;
        let mut first_match_buf: Option<&'static mut FileBuf> = None;
        let status = vgr_process_files(
            wp_ptr.opt(),
            qi,
            &mut args,
            &mut redraw_for_dummy,
            &mut first_match_buf,
            &mut target_dir,
        );

        if status {
            let qfl = qf_get_curlist(qi);
            qfl.qf_nonevalid = false;
            qfl.qf_index = 1;
            qf_list_changed(qfl);

            qf_update_buffer(qi, 0);

            let save_qfid = qf_get_curlist(qi).qf_id;
            let qi_ptr = ptr::from_mut(qi);

            if let Some(au_name) = au_name {
                apply_autocmds(
                    EVENT_QUICKFIXCMDPOST,
                    Some(au_name),
                    curbuf_mut().fname_opt(),
                    true,
                    curbuf_opt_mut(),
                );
            }

            if qflist_valid(wp_ptr.opt(), save_qfid)
                && qf_restore_list(crate::globals::GraphPtr::of(qi_ptr).borrow_mut(), save_qfid)
            {
                if !qf_list_empty(Some(
                    &*(qf_get_curlist(crate::globals::GraphPtr::of(qi_ptr).borrow_mut())),
                )) {
                    if (args.flags & VGR_NOJUMP) == 0 {
                        vgr_jump_to_match(
                            crate::globals::GraphPtr::of(qi_ptr).borrow_mut(),
                            eap.forceit,
                            &mut redraw_for_dummy,
                            first_match_buf.as_deref(),
                            target_dir.as_deref(),
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
    args.regmatch.free_prog();
}

fn restore_start_dir(dirname_start: &CStr) {
    let mut dirname_now = vec![0u8; MAXPATHL];

    os_dirname(&mut dirname_now);
    if dirname_start.to_bytes() != crate::memory::bytes_until_nul(&dirname_now) {
        let mut ea: Exarg = Exarg::new();
        ea.arg = crate::strings::OwnedSlice::from_cstr(dirname_start);
        ea.cmdidx = if curwin_mut().w_localdir.is_none() {
            Cmdidx::CmdCd
        } else {
            Cmdidx::CmdLcd
        };
        ex_cd(&mut ea);
    }
}

fn load_dummy_buffer(
    fname: &CStr,
    dirname_start: &CStr,
    resulting_dir: &mut [u8],
) -> Option<&'static mut FileBuf> {
    let mut newbuf = crate::globals::GraphPtr::from(buflist_new(None, None, 1, BLN_DUMMY));
    if newbuf.is_null() {
        return None;
    }

    let mut failed = true;
    let mut newbufref: Bufref = Bufref::new();
    set_bufref(&mut newbufref, newbuf.opt_mut());

    buf_copy_options(newbuf.borrow_mut(), BCO_ENTER | BCO_NOHELP);

    if ml_open(newbuf.borrow_mut()) {
        newbuf.b_locked += 1;

        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, None, None, newbuf.opt_mut(), 0);

        setfname(curbuf_mut(), Some(fname), None, false);
        check_need_swap(true);
        curbuf_mut().b_flags &= !BF_DUMMY;

        let mut newbuf_to_wipe: Bufref = Bufref::new();
        let readfile_result = readfile(
            Some(fname),
            None,
            0,
            0,
            MAXLNUM,
            None,
            READ_NEW | READ_DUMMY,
            false,
        );

        newbuf.b_locked -= 1;

        if readfile_result == Some(true) && !g_st().got_int && (curbuf_mut().b_flags & BF_NEW) == 0
        {
            failed = false;
            if g_st().curbuf != newbuf {
                set_bufref(&mut newbuf_to_wipe, newbuf.opt_mut());
                newbuf = g_st().curbuf;
            }
        }

        ctx_restore(&mut aco);

        if !newbuf_to_wipe.br_buf.is_null() && bufref_valid(Some(&newbuf_to_wipe)) {
            block_autocmds();
            wipe_dummy_buffer(newbuf_to_wipe.br_buf.borrow_mut(), None);
            unblock_autocmds();
        }

        newbuf.b_flags |= BF_DUMMY;
    }

    os_dirname(resulting_dir);
    restore_start_dir(dirname_start);

    if !bufref_valid(Some(&newbufref)) {
        return None;
    }
    if failed {
        wipe_dummy_buffer(newbuf.borrow_mut(), Some(dirname_start));
        return None;
    }
    Some(newbuf.borrow_mut())
}

fn wipe_dummy_buffer(buf: &mut FileBuf, dirname_start: Option<&CStr>) {
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
        wipe_buffer(buf, true);
        leave_cleanup(&mut cs);

        if let Some(dirname_start) = dirname_start {
            restore_start_dir(dirname_start);
        }
        return;
    }

    buf.b_flags &= !BF_DUMMY;
}

fn unload_dummy_buffer(buf: &mut FileBuf, dirname_start: &CStr) {
    let buf_ptr = ptr::from_mut(buf);
    if g_st().curbuf == buf_ptr {
        return;
    }

    close_buffer(None, Some(buf), DOBUF_UNLOAD, false, true, false);
    restore_start_dir(dirname_start);
}

fn get_qfline_items(qfp: &mut Qfline, list: &mut TvList) -> bool {
    let mut bufnum = qfp.qf_fnum;
    if bufnum != 0 && buflist_findnr(bufnum).is_none() {
        bufnum = 0;
    }

    let dict = tv_dict_alloc();
    tv_list_append_dict(&mut *list, Some(dict));

    let buf = [qfp.qf_type, 0];
    let buflen: usize = if buf[0] == 0 { 0 } else { 1 };
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
            Some(qfp.qf_module.as_deref().unwrap_or(c"")),
        )
        || !tv_dict_add_str(
            &mut *dict,
            c"pattern",
            Some(qfp.qf_pattern.as_deref().unwrap_or(c"")),
        )
        || !tv_dict_add_str(&mut *dict, c"text", Some(qfp.qf_text.as_c_str()))
        || !tv_dict_add_str_len(&mut *dict, c"type", Some(&buf[..buflen]))
        || (qfp.qf_user_data.v_type != VarType::VarUnknown
            && !tv_dict_add_tv(&mut *dict, b"user_data", &mut qfp.qf_user_data))
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
    let qi = if let Some(qi) = qi_arg {
        qi
    } else if let Some(wp) = wp {
        let Some(ll) = get_loc_list_opt(wp) else {
            return false;
        };
        ll
    } else {
        ql_info_mut()
    };

    if eidx < 0 {
        return true;
    }

    if qf_idx == INVALID_QFIDX {
        qf_idx = qi.qf_curlist;
    }

    if qf_idx >= qi.qf_listcount {
        return false;
    }

    let qfl = qf_get_list(qi, qf_idx);
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

    let mut errorformat = opt_vars().borrow().p_efm.as_cstr();
    if let Some(efm_di) = tv_dict_find_opt(Some(what), c"efm", 3) {
        if efm_di.di_tv.v_type != VarType::VarString || efm_di.di_tv.vval.v_string_is_null() {
            return false;
        }
        errorformat = efm_di.di_tv.vval.v_string_cstr_or_empty();
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
        qf_id_handle: QfInfoId::NULL,
    };

    if qf_init_ext(
        &mut qi,
        0,
        None,
        None,
        Some(&mut di.di_tv),
        errorformat,
        true,
        0,
        0,
        None,
        None,
    ) > 0
    {
        get_errorlist(Some(&mut qi), None, 0, 0, &mut *l);
    }

    tv_dict_add_list(retdict, c"items", Some(l));
    true
}

fn qf_winid(qi: Option<&QfInfo>) -> i32 {
    if let Some(qi) = qi
        && let Some(win) = qf_find_win(qi)
    {
        return win.handle;
    }
    0
}

fn qf_getprop_qfbufnr(qi: Option<&QfInfo>, retdict: &mut TvDict) -> bool {
    let mut bufnum: i32 = 0;
    if let Some(qi) = qi
        && buflist_findnr(qi.qf_bufnr).is_some()
    {
        bufnum = qi.qf_bufnr;
    }
    tv_dict_add_nr(retdict, c"qfbufnr", Varnumber::from(bufnum))
}

fn qf_getprop_keys2flags(what: &TvDict, loclist: bool) -> i32 {
    let mut flags = QF_GETLIST_NONE;

    if tv_dict_find(Some(what), c"all", 3).is_some() {
        flags |= QF_GETLIST_ALL;
        if !loclist {
            flags &= !QF_GETLIST_FILEWINID;
        }
    }
    if tv_dict_find(Some(what), c"title", 5).is_some() {
        flags |= QF_GETLIST_TITLE;
    }
    if tv_dict_find(Some(what), c"nr", 2).is_some() {
        flags |= QF_GETLIST_NR;
    }
    if tv_dict_find(Some(what), c"winid", 5).is_some() {
        flags |= QF_GETLIST_WINID;
    }
    if tv_dict_find(Some(what), c"context", 7).is_some() {
        flags |= QF_GETLIST_CONTEXT;
    }
    if tv_dict_find(Some(what), c"id", 2).is_some() {
        flags |= QF_GETLIST_ID;
    }
    if tv_dict_find(Some(what), c"items", 5).is_some() {
        flags |= QF_GETLIST_ITEMS;
    }
    if tv_dict_find(Some(what), c"idx", 3).is_some() {
        flags |= QF_GETLIST_IDX;
    }
    if tv_dict_find(Some(what), c"size", 4).is_some() {
        flags |= QF_GETLIST_SIZE;
    }
    if tv_dict_find(Some(what), c"changedtick", 11).is_some() {
        flags |= QF_GETLIST_TICK;
    }
    if loclist && tv_dict_find(Some(what), c"filewinid", 9).is_some() {
        flags |= QF_GETLIST_FILEWINID;
    }
    if tv_dict_find(Some(what), c"qfbufnr", 7).is_some() {
        flags |= QF_GETLIST_QFBUFNR;
    }
    if tv_dict_find(Some(what), c"quickfixtextfunc", 16).is_some() {
        flags |= QF_GETLIST_QFTF;
    }

    flags
}

fn qf_getprop_qfidx(qi: &mut QfInfo, what: &mut TvDict) -> i32 {
    let mut qf_idx = qi.qf_curlist;
    if let Some(di) = tv_dict_find_opt(Some(what), c"nr", 2) {
        if di.di_tv.v_type == VarType::VarNumber {
            if di.di_tv.vval.v_number() != 0 {
                qf_idx = di.di_tv.vval.v_number() as i32 - 1;
                if qf_idx < 0 || qf_idx >= qi.qf_listcount {
                    qf_idx = INVALID_QFIDX;
                }
            }
        } else if di.di_tv.v_type == VarType::VarString
            && (di.di_tv.vval.v_string_opt_bytes() == Some(b"$"))
        {
            qf_idx = qi.qf_listcount - 1;
        } else {
            qf_idx = INVALID_QFIDX;
        }
    }

    if let Some(di) = tv_dict_find_opt(Some(what), c"id", 2) {
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

fn qf_getprop_defaults(
    qi: Option<&QfInfo>,
    flags: i32,
    locstack: bool,
    retdict: &mut TvDict,
) -> bool {
    let mut status = true;

    if (flags & QF_GETLIST_TITLE) != 0 {
        status = tv_dict_add_str_len(retdict, c"title", Some(b""));
    }
    if status && (flags & QF_GETLIST_ITEMS) != 0 {
        let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
        status = tv_dict_add_list(retdict, c"items", Some(l));
    }
    if status && (flags & QF_GETLIST_NR) != 0 {
        status = tv_dict_add_nr(retdict, c"nr", 0);
    }
    if status && (flags & QF_GETLIST_WINID) != 0 {
        status = tv_dict_add_nr(retdict, c"winid", Varnumber::from(qf_winid(qi)));
    }
    if status && (flags & QF_GETLIST_CONTEXT) != 0 {
        status = tv_dict_add_str_len(retdict, c"context", Some(b""));
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
        status = qf_getprop_qfbufnr(qi, &mut *retdict);
    }
    if status && (flags & QF_GETLIST_QFTF) != 0 {
        status = tv_dict_add_str_len(retdict, c"quickfixtextfunc", Some(b""));
    }

    status
}

fn qf_getprop_title(qfl: &mut QfList, retdict: &mut TvDict) -> bool {
    tv_dict_add_str(retdict, c"title", qfl.qf_title.as_deref())
}

fn qf_getprop_filewinid(wp: Option<&WinInfoInfo>, qi: &QfInfo, retdict: &mut TvDict) -> bool {
    let mut winid: ObjHandle = 0;

    if let Some(wp) = wp
        && is_ll_window(wp)
        && let Some(ll_wp) = qf_find_win_with_loclist(qi)
    {
        winid = ll_wp.handle;
    }

    tv_dict_add_nr(retdict, c"filewinid", Varnumber::from(winid))
}

fn qf_getprop_items(qi: &mut QfInfo, qf_idx: i32, eidx: i32, retdict: &mut TvDict) -> bool {
    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    get_errorlist(Some(&mut *qi), None, qf_idx, eidx, &mut *l);
    tv_dict_add_list(retdict, c"items", Some(l));
    true
}

fn qf_getprop_ctx(qfl: &mut QfList, retdict: &mut TvDict) -> bool {
    if let Some(ref ctx) = qfl.qf_ctx {
        let di = tv_dict_item_alloc_len(b"context");
        tv_copy(ctx, &mut di.di_tv);
        let status = tv_dict_add(retdict, di);
        if !status {
            tv_dict_item_free(di);
        }
        status
    } else {
        tv_dict_add_str_len(retdict, c"context", Some(b""))
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
        let status = tv_dict_add_tv(retdict, b"quickfixtextfunc", &mut tv);
        tv_clear(&mut tv);
        status
    } else {
        tv_dict_add_str_len(retdict, c"quickfixtextfunc", Some(b""))
    }
}

fn qf_get_properties(wp: Option<&WinInfoInfo>, what: &mut TvDict, retdict: &mut TvDict) -> bool {
    let mut status = true;
    let mut qf_idx = INVALID_QFIDX;

    if let Some(di) = tv_dict_find_opt(Some(what), c"lines", 5) {
        return qf_get_list_from_lines(what, di, retdict);
    }

    let mut qi = if let Some(wp) = wp {
        get_loc_list_opt(wp)
    } else {
        Some(ql_info_mut())
    };

    let flags = qf_getprop_keys2flags(what, wp.is_some());

    if !qf_stack_empty(qi.as_deref())
        && let Some(ref mut q) = qi
    {
        qf_idx = qf_getprop_qfidx(q, what);
    }

    if qf_stack_empty(qi.as_deref()) || qf_idx == INVALID_QFIDX {
        return qf_getprop_defaults(qi.as_deref(), flags, wp.is_some(), retdict);
    }
    let qi = qi.unwrap();

    let mut eidx: i32 = 0;

    if let Some(di) = tv_dict_find_opt(Some(what), c"idx", 3) {
        if di.di_tv.v_type != VarType::VarNumber {
            return false;
        }
        eidx = di.di_tv.vval.v_number() as i32;
    }

    if (flags & QF_GETLIST_TITLE) != 0 {
        status = qf_getprop_title(qf_get_list(qi, qf_idx), retdict);
    }
    if status && (flags & QF_GETLIST_NR) != 0 {
        status = tv_dict_add_nr(retdict, c"nr", Varnumber::from(qf_idx + 1));
    }
    if status && (flags & QF_GETLIST_WINID) != 0 {
        status = tv_dict_add_nr(retdict, c"winid", Varnumber::from(qf_winid(Some(qi))));
    }
    if status && (flags & QF_GETLIST_ITEMS) != 0 {
        status = qf_getprop_items(qi, qf_idx, eidx, retdict);
    }
    if status && (flags & QF_GETLIST_CONTEXT) != 0 {
        status = qf_getprop_ctx(qf_get_list(qi, qf_idx), retdict);
    }
    if status && (flags & QF_GETLIST_ID) != 0 {
        let qf_id = qf_get_list(qi, qf_idx).qf_id;
        status = tv_dict_add_nr(retdict, c"id", Varnumber::from(qf_id));
    }
    if status && (flags & QF_GETLIST_IDX) != 0 {
        status = qf_getprop_idx(qf_get_list(qi, qf_idx), eidx, retdict);
    }
    if status && (flags & QF_GETLIST_SIZE) != 0 {
        let count = qf_get_list(qi, qf_idx).count();
        status = tv_dict_add_nr(retdict, c"size", Varnumber::from(count));
    }
    if status && (flags & QF_GETLIST_TICK) != 0 {
        let tick = qf_get_list(qi, qf_idx).qf_changedtick;
        status = tv_dict_add_nr(retdict, c"changedtick", Varnumber::from(tick));
    }
    if status && wp.is_some() && (flags & QF_GETLIST_FILEWINID) != 0 {
        status = qf_getprop_filewinid(wp, qi, retdict);
    }
    if status && (flags & QF_GETLIST_QFBUFNR) != 0 {
        status = qf_getprop_qfbufnr(Some(qi), retdict);
    }
    if status && (flags & QF_GETLIST_QFTF) != 0 {
        status = qf_getprop_qftf(qf_get_list(qi, qf_idx), retdict);
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
) -> QfStatus {
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
    let vcol = tv_dict_get_number(Some(d), c"vcol") != 0;
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

    if bufnum != 0 && buflist_findnr(bufnum).is_none() {
        if !qf_st().did_bufnr_emsg {
            qf_st().did_bufnr_emsg = true;
            semsg!(gettext(E_BUFFER_NR_NOT_FOUND), bufnum);
        }
        valid = false;
        bufnum = 0;
    }

    if tv_dict_find(Some(d), c"valid", -1).is_some() {
        valid = tv_dict_get_bool(Some(d), c"valid", 0) != 0;
    }

    let status = qf_add_entry(
        qfl,
        None,
        filename.as_deref(),
        module.as_deref(),
        bufnum,
        &text,
        lnum,
        end_lnum,
        col,
        end_col,
        vcol,
        pattern.as_deref(),
        nr,
        type_
            .and_then(|s| s.to_bytes().first().copied())
            .unwrap_or(0),
        Some(&user_data),
        valid,
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
    title: Option<&CStr>,
    action: i32,
) -> QfStatus {
    let mut qfl = qf_get_list(&mut *qi, qf_idx);
    let mut retval = QfStatus::Ok;
    let mut valid_entry = false;

    let mut old_count: usize = 0;

    let mut prev_fnum: i32 = 0;
    let mut prev_lnum: i32 = 0;
    let mut prev_col: i32 = 0;
    if let Some(cur_entry) = qfl.cur_entry() {
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

    for (i, li) in tv_list_iter(list).enumerate() {
        if li.li_tv.v_type == VarType::VarDict
            && let Some(d) = li.li_tv.vval.v_dict_opt()
        {
            retval = qf_add_entry_from_dict(qfl, d, i == 0, &mut valid_entry);
            if retval == QfStatus::Fail {
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

fn qf_setprop_get_qfidx(qi: &QfInfo, what: &TvDict, action: i32, newlist: &mut bool) -> i32 {
    let mut qf_idx = qi.qf_curlist;

    if let Some(di) = tv_dict_find_opt(Some(what), c"nr", 2) {
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
            && (di.di_tv.vval.v_string_opt_bytes() == Some(b"$"))
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

    if !*newlist && let Some(di) = tv_dict_find_opt(Some(what), c"id", 2) {
        if di.di_tv.v_type != VarType::VarNumber {
            return INVALID_QFIDX;
        }
        return qf_id2nr(qi, di.di_tv.vval.v_number() as u32);
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
        di.di_tv.vval.v_list_opt().as_deref(),
        title_save.as_deref(),
        if action == b' ' as i32 {
            b'a' as i32
        } else {
            action
        },
    ) == QfStatus::Ok
}

fn qf_setprop_items_from_lines(
    qi: &mut QfInfo,
    qf_idx: i32,
    what: &TvDict,
    di: &mut Dictitem,
    action: i32,
) -> bool {
    let efm_owned: std::ffi::CString;
    let mut errorformat = opt_vars().borrow().p_efm.as_cstr();
    let mut retval = false;

    if let Some(efm_di) = tv_dict_find_opt(Some(what), c"efm", 3) {
        if efm_di.di_tv.v_type != VarType::VarString || efm_di.di_tv.vval.v_string_is_null() {
            return false;
        }
        efm_owned = efm_di.di_tv.vval.v_string_owned();
        errorformat = efm_owned.as_c_str();
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
        None,
        None,
        Some(&mut di.di_tv),
        errorformat,
        false,
        0,
        0,
        None,
        None,
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
        && !di.di_tv.vval.v_string_is_null()
        && (di.di_tv.vval.v_string_bytes() == b"$")
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
    if get_nth_entry(qfl, newidx, &mut newidx).is_none() {
        return false;
    }
    qfl.qf_index = newidx;
    let qfl_id = qfl.qf_id;

    if (&qi.qf_lists)[qi.qf_curlist as usize].qf_id == qfl_id {
        qf_win_pos_update(qi, old_qfidx);
    }
    true
}

fn qf_set_properties(qi: &mut QfInfo, what: &TvDict, action: i32, title: Option<&CStr>) -> bool {
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
    if let Some(di) = tv_dict_find_opt(Some(what), c"title", 5) {
        retval = qf_setprop_title(qi, qf_idx, what, di);
    }
    if let Some(di) = tv_dict_find_opt(Some(what), c"items", 5) {
        retval = qf_setprop_items(qi, qf_idx, di, action);
    }
    if let Some(di) = tv_dict_find_opt(Some(what), c"lines", 5) {
        retval = qf_setprop_items_from_lines(qi, qf_idx, what, di, action);
    }
    if let Some(di) = tv_dict_find_opt(Some(what), c"context", 7) {
        retval = qf_setprop_context(qf_get_list(&mut *qi, qf_idx), di);
    }
    if let Some(di) = tv_dict_find_opt(Some(what), c"idx", 3) {
        let qfl = qf_get_list(qi, qf_idx);
        retval = qf_setprop_curidx(qi, qfl, di);
    }
    if let Some(di) = tv_dict_find_opt(Some(what), c"quickfixtextfunc", 16) {
        retval = qf_setprop_qftf(qf_get_list(&mut *qi, qf_idx), di);
    }

    if newlist || retval {
        qf_list_changed(qf_get_list(&mut *qi, qf_idx));
    }
    if newlist {
        qf_update_buffer(qi, 0);
    }

    retval
}

fn qf_free_stack(wp_in: Option<&mut WinInfoInfo>, qi_in: &mut QfInfo) {
    let mut wp = crate::globals::GraphPtr::from(wp_in);
    let mut qi = crate::globals::GraphPtr::from(qi_in);
    let mut qfwin = crate::globals::GraphPtr::from(qf_find_win(qi.borrow()));

    if !qfwin.is_null() {
        if qi.qf_curlist < qi.qf_listcount {
            qf_free(qf_get_curlist(qi.borrow_mut()));
        }

        qf_update_buffer(qi.borrow_mut(), 0);
    }

    if !wp.is_null()
        && is_ll_window(wp.borrow())
        && let Some(llwin) = qf_find_win_with_loclist(qi.borrow())
    {
        wp = llwin.into();
    }

    qf_free_all(wp.opt_mut());

    if wp.is_null() {
        qi.qf_curlist = 0;
        qi.qf_listcount = 0;
    } else if !qfwin.is_null() {
        let new_ll = qf_alloc_stack(QFLT_LOCATION, wp.w_onebuf_opt.wo_lhi as i32);

        new_ll.qf_bufnr = qfwin.buf_mut().b_fnum;

        let old_llist_ref = std::mem::replace(&mut qfwin.w_llist_ref, QfInfoId::NULL);
        ll_free_all(old_llist_ref.opt_mut());

        qfwin.w_llist_ref = new_ll.qf_id_handle;

        if wp != qfwin {
            win_set_loclist(wp.borrow_mut(), &mut *new_ll);
        }
    }
}

pub fn set_errorlist(
    mut wp: Option<&mut WinInfoInfo>,
    list: Option<&TvList>,
    action: i32,
    title: Option<&CStr>,
    what: Option<&TvDict>,
) -> bool {
    let qi_id = if let Some(ref mut wp_ref) = wp {
        ll_get_or_alloc_list(wp_ref).qf_id_handle
    } else {
        QfInfoId::QF_STACK
    };

    if action == b'f' as i32 {
        qf_free_stack(wp, qi_id.borrow_mut());
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

    let qi_mut = qi_id.borrow_mut();
    let retval = if let Some(what) = what {
        qf_set_properties(qi_mut, what, action, title)
    } else {
        let curlist = qi_mut.qf_curlist;
        let r = qf_add_entries(qi_mut, curlist, list, title, action);
        if r == QfStatus::Ok {
            qf_list_changed(qf_get_curlist(qi_mut));
        }
        r == QfStatus::Ok
    };

    decr_quickfix_busy();

    retval
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
                    abort = abort || set_ref_in_item(user_data, copy_id, None, None);
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
            abort = set_ref_in_item(ctx, copy_id, None, None);
        }

        let cb = &mut qfl.qf_qftf_cb;
        abort = abort || set_ref_in_callback(cb, copy_id, None, None);
        i += 1;
    }
    abort
}

pub fn set_ref_in_quickfix(copy_id: i32) -> bool {
    if mark_quickfix_ctx(ql_info_mut(), copy_id)
        || mark_quickfix_user_data(ql_info_mut(), copy_id)
        || set_ref_in_callback(&mut opt_vars().p_qftf, copy_id, None, None)
    {
        return true;
    }

    for tp in for_all_tabpages_mut() {
        for win in win_iter_mut(tab_firstwin(tp)) {
            if let Some(ll) = win.w_llist.opt_mut()
                && (mark_quickfix_ctx(ll, copy_id) || mark_quickfix_user_data(ll, copy_id))
            {
                return true;
            }

            if is_ll_window(win)
                && let Some(ll_ref) = win.w_llist_ref.opt_mut()
                && ll_ref.qf_refcount == 1
                && (mark_quickfix_ctx(ll_ref, copy_id) || mark_quickfix_user_data(ll_ref, copy_id))
            {
                return true;
            }
        }
    }

    false
}

fn cbuffer_get_auname(cmdidx: Cmdidx) -> Option<&'static CStr> {
    match cmdidx {
        Cmdidx::CmdCbuffer => Some(c"cbuffer"),
        Cmdidx::CmdCgetbuffer => Some(c"cgetbuffer"),
        Cmdidx::CmdCaddbuffer => Some(c"caddbuffer"),
        Cmdidx::CmdLbuffer => Some(c"lbuffer"),
        Cmdidx::CmdLgetbuffer => Some(c"lgetbuffer"),
        Cmdidx::CmdLaddbuffer => Some(c"laddbuffer"),
        _ => None,
    }
}

fn cbuffer_process_args(
    eap: &mut Exarg,
    line1: &mut Linenr,
    line2: &mut Linenr,
) -> Option<&'static mut FileBuf> {
    let mut buf = crate::globals::GraphPtr::<FileBuf>::NULL;

    if eap.arg.is_empty() {
        buf = crate::globals::GraphPtr::<FileBuf>::of(g_st().curbuf);
    } else if skipwhite_slice(skipdigits_slice(eap.arg.as_bytes())).is_empty() {
        buf = crate::globals::GraphPtr::from(buflist_findnr(crate::charset::parse_i64_prefix(
            eap.arg.as_bytes(),
        ) as i32));
    }

    let Some(buf_ref) = buf.opt_mut() else {
        emsg(gettext(E_INVARG));
        return None;
    };

    if buf_ref.b_ml.ml_mfp.is_none() {
        emsg(gettext(E_BUFFER_IS_NOT_LOADED));
        return None;
    }

    if eap.addr_count == 0 {
        eap.line1 = 1;
        eap.line2 = buf_ref.b_ml.ml_line_count;
    }

    if eap.line1 < 1
        || eap.line1 > buf_ref.b_ml.ml_line_count
        || eap.line2 < 1
        || eap.line2 > buf_ref.b_ml.ml_line_count
    {
        emsg(gettext(E_INVRANGE));
        return None;
    }

    *line1 = eap.line1;
    *line2 = eap.line2;

    Some(buf_ref)
}

pub fn ex_cbuffer(eap: &mut Exarg) {
    let au_name = cbuffer_get_auname(eap.cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            Some(au_name),
            curbuf_mut().fname_opt(),
            true,
            curbuf_opt_mut(),
        )
        && aborting()
    {
        return;
    }

    let mut wp: Option<&'static mut WinInfoInfo> = None;
    let qi = qf_cmd_get_or_alloc_stack(eap, &mut wp);
    let wp_ptr = crate::globals::GraphPtr::from(wp.as_deref());

    let mut line1: Linenr = 0;
    let mut line2: Linenr = 0;
    let Some(buf) = cbuffer_process_args(eap, &mut line1, &mut line2) else {
        return;
    };

    let qf_title_cstr = qf_cmdtitle(eap.cmdline_cstr());
    let mut qf_title: &CStr = &qf_title_cstr;
    if let Some(buf_sfname) = buf.sfname_opt() {
        format_to_buf!(&mut g_st().iobuff, c"%s (%s)", qf_title, buf_sfname);
        qf_title = g_st().iobuff_cstr();
    }

    incr_quickfix_busy();

    let curlist = qi.qf_curlist;
    let mut res = qf_init_ext(
        qi,
        curlist,
        None,
        Some(buf),
        None,
        opt_vars().p_efm.as_cstr(),
        eap.cmdidx != Cmdidx::CmdCaddbuffer && eap.cmdidx != Cmdidx::CmdLaddbuffer,
        eap.line1,
        eap.line2,
        Some(qf_title),
        None,
    );
    if qf_stack_empty(Some(qi)) {
        decr_quickfix_busy();
        return;
    }
    if res >= 0 {
        qf_list_changed(qf_get_curlist(qi));
    }
    let save_qfid = qf_get_curlist(qi).qf_id;
    let qi_ptr = ptr::from_mut(qi);
    if let Some(au_name) = au_name {
        let curbuf_old = g_st().curbuf;
        apply_autocmds(
            EVENT_QUICKFIXCMDPOST,
            Some(au_name),
            curbuf_mut().fname_opt(),
            true,
            curbuf_opt_mut(),
        );
        if g_st().curbuf != curbuf_old {
            res = 0;
        }
    }
    if res > 0
        && (eap.cmdidx == Cmdidx::CmdCbuffer || eap.cmdidx == Cmdidx::CmdLbuffer)
        && qflist_valid(wp_ptr.opt(), save_qfid)
    {
        qf_jump_first(
            crate::globals::GraphPtr::of(qi_ptr).borrow_mut(),
            save_qfid,
            eap.forceit,
        );
    }

    decr_quickfix_busy();
}

fn cexpr_get_auname(cmdidx: Cmdidx) -> Option<&'static CStr> {
    match cmdidx {
        Cmdidx::CmdCexpr => Some(c"cexpr"),
        Cmdidx::CmdCgetexpr => Some(c"cgetexpr"),
        Cmdidx::CmdCaddexpr => Some(c"caddexpr"),
        Cmdidx::CmdLexpr => Some(c"lexpr"),
        Cmdidx::CmdLgetexpr => Some(c"lgetexpr"),
        Cmdidx::CmdLaddexpr => Some(c"laddexpr"),
        _ => None,
    }
}

fn trigger_cexpr_autocmd(cmdidx: Cmdidx) -> bool {
    let au_name = cexpr_get_auname(cmdidx);
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            Some(au_name),
            curbuf_mut().fname_opt(),
            true,
            curbuf_opt_mut(),
        )
        && aborting()
    {
        return false;
    }
    true
}

pub fn cexpr_core(eap: &Exarg, tv: &mut TypVal) -> bool {
    let mut wp: Option<&'static mut WinInfoInfo> = None;
    let qi = qf_cmd_get_or_alloc_stack(eap, &mut wp);
    let wp_ptr = crate::globals::GraphPtr::from(wp.as_deref());

    if (tv.v_type == VarType::VarString && !tv.vval.v_string_is_null())
        || tv.v_type == VarType::VarList
    {
        let au_name = cexpr_get_auname(eap.cmdidx);
        incr_quickfix_busy();
        let curlist = qi.qf_curlist;
        let res = qf_init_ext(
            qi,
            curlist,
            None,
            None,
            Some(&mut *tv),
            opt_vars().p_efm.as_cstr(),
            eap.cmdidx != Cmdidx::CmdCaddexpr && eap.cmdidx != Cmdidx::CmdLaddexpr,
            0,
            0,
            Some(&qf_cmdtitle(eap.cmdline_cstr())),
            None,
        );
        if qf_stack_empty(Some(qi)) {
            decr_quickfix_busy();
            return false;
        }
        if res >= 0 {
            qf_list_changed(qf_get_curlist(qi));
        }
        let save_qfid = qf_get_curlist(qi).qf_id;
        let qi_ptr = ptr::from_mut(qi);
        if let Some(au_name) = au_name {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                Some(au_name),
                curbuf_mut().fname_opt(),
                true,
                curbuf_opt_mut(),
            );
        }
        if res > 0
            && (eap.cmdidx == Cmdidx::CmdCexpr || eap.cmdidx == Cmdidx::CmdLexpr)
            && qflist_valid(wp_ptr.opt(), save_qfid)
        {
            qf_jump_first(
                crate::globals::GraphPtr::of(qi_ptr).borrow_mut(),
                save_qfid,
                eap.forceit,
            );
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

    let arg = eap.arg.to_cstring();
    let Some(mut tv) = eval_expr(&arg, Some(&mut *eap)) else {
        return;
    };

    let _ = cexpr_core(eap, &mut tv);
    tv_clear(&mut tv);
}

fn hgr_get_ll(new_ll: &mut bool) -> &'static mut QfInfo {
    let wp = if bt_help(curwin_mut().buf_opt()) {
        Some(curwin_mut())
    } else {
        qf_find_help_win()
    };

    if let Some(wp_ref) = wp
        && let Some(ll) = wp_ref.w_llist.opt_mut()
    {
        return ll;
    }
    *new_ll = true;
    qf_alloc_stack(QFLT_LOCATION, 1)
}

fn hgr_search_file(qfl: &mut QfList, fname: &CStr, p_regmatch: &mut Regmatch) {
    let Some(fd) = CFile::open(fname, c"r") else {
        return;
    };

    let mut lnum: Linenr = 1;
    while !vim_fgets(g_st().iobuff_bytes_mut(), &fd) && !g_st().got_int {
        if vim_regexec(p_regmatch, g_st().iobuff_cstr(), 0) {
            let mut l = g_st().iobuff_cstr().to_bytes().len();
            while l > 0 && g_st().iobuff[l - 1] <= b' ' {
                l -= 1;
                g_st().iobuff[l] = 0;
            }
            let line_cstr = g_st().iobuff_cstr();

            if qf_add_entry(
                qfl,
                None,
                Some(fname),
                None,
                0,
                line_cstr,
                lnum,
                0,
                (p_regmatch.start_offset(0, line_cstr) as i32) + 1,
                (p_regmatch.end_offset(0, line_cstr) as i32) + 1,
                false,
                None,
                0,
                1,
                None,
                true,
            ) == QfStatus::Fail
            {
                g_st().got_int = true;
                break;
            }
        }
        lnum += 1;
        line_breakcheck();
    }
}

fn hgr_search_files_in_dir(
    qfl: &mut QfList,
    dirname: &mut [u8],
    p_regmatch: &mut Regmatch,
    lang: Option<&CStr>,
) {
    let mut fnames: Vec<std::ffi::CString> = Vec::new();

    add_pathsep(dirname);
    let dir_len = crate::memory::bytes_until_nul(dirname).len();
    let pat_suffix = b"doc/*.\\(txt\\|??x\\)\0";
    dirname[dir_len..dir_len + pat_suffix.len()].copy_from_slice(pat_suffix);
    if gen_expand_wildcards_vec(
        &[crate::memory::bytes_as_cstr(dirname)],
        &mut fnames,
        EW_FILE | EW_SILENT,
    ) && !fnames.is_empty()
    {
        for f_cstr in &fnames {
            if g_st().got_int {
                break;
            }
            let f_bytes = f_cstr.as_bytes();
            let flen = f_bytes.len();
            if let Some(lang_cstr) = lang
                && flen >= 3
            {
                let lang_b = lang_cstr.to_bytes();
                let lang_2 = &lang_b[..2.min(lang_b.len())];
                let ext = &f_bytes[flen - 3..];
                if !lang_2.eq_ignore_ascii_case(&ext[..2])
                    && !(lang_2.eq_ignore_ascii_case(b"en") && ext.eq_ignore_ascii_case(b"txt"))
                {
                    continue;
                }
            }

            hgr_search_file(qfl, f_cstr, p_regmatch);
        }
    }
}

fn hgr_search_in_rtp(qfl: &mut QfList, p_regmatch: &mut Regmatch, lang: Option<&CStr>) {
    let p_rtp = opt_vars().p_rtp.to_owned();
    let mut p = p_rtp.as_bytes();
    while !p.is_empty() && !g_st().got_int {
        copy_option_part(&mut p, g_st().name_buff_bytes_mut(), c",");
        hgr_search_files_in_dir(qfl, g_st().name_buff_bytes_mut(), p_regmatch, lang);
    }
}

pub fn ex_helpgrep(eap: &mut Exarg) {
    let au_name: Option<&'static CStr> = match eap.cmdidx {
        Cmdidx::CmdHelpgrep => Some(c"helpgrep"),
        Cmdidx::CmdLhelpgrep => Some(c"lhelpgrep"),
        _ => None,
    };
    if let Some(au_name) = au_name
        && apply_autocmds(
            EVENT_QUICKFIXCMDPRE,
            Some(au_name),
            curbuf_mut().fname_opt(),
            true,
            curbuf_opt_mut(),
        )
        && aborting()
    {
        return;
    }

    let mut updated = false;
    let save_cpo = std::mem::take(&mut opt_vars().p_cpo);

    let mut new_qi = false;
    let qi = if is_loclist_cmd(eap.cmdidx) {
        hgr_get_ll(&mut new_qi)
    } else {
        ql_info_mut()
    };

    incr_quickfix_busy();

    let lang_owned = if eap.arg.is_null() {
        None
    } else {
        check_help_lang(eap.arg_cstr_buf_mut()).map(CStr::to_owned)
    };
    let lang = lang_owned.as_deref();
    let mut regmatch: Regmatch = Regmatch::new();
    regmatch.set_prog(vim_regcomp(eap.arg.as_cstr(), RE_MAGIC + RE_STRING));
    regmatch.rm_ic = false;
    if !regmatch.regprog.is_null() {
        qf_new_list(qi, Some(&qf_cmdtitle(eap.cmdline_cstr())));
        let qfl = qf_get_curlist(qi);

        hgr_search_in_rtp(qfl, &mut regmatch, lang);

        regmatch.free_prog();

        qfl.qf_nonevalid = false;
        qfl.qf_index = 1;
        qf_list_changed(qfl);
        updated = true;
    }

    if opt_vars().p_cpo.is_null() {
        opt_vars().p_cpo = save_cpo;
    } else if opt_vars().p_cpo.is_empty() {
        set_option_value_give_err(K_OPT_CPOPTIONS, cstr_as_obj(Some(save_cpo.as_cstr())), 0);
    }

    if updated {
        qf_update_buffer(qi, 0);
    }

    let qi_ptr = qi.qf_id_handle;
    if let Some(au_name) = au_name {
        apply_autocmds(
            EVENT_QUICKFIXCMDPOST,
            Some(au_name),
            curbuf_mut().fname_opt(),
            true,
            curbuf_opt_mut(),
        );
        if !new_qi
            && is_ll_stack(qi_ptr.borrow())
            && qf_find_win_with_loclist(qi_ptr.borrow()).is_none()
        {
            decr_quickfix_busy();
            return;
        }
    }

    if !qf_list_empty(Some(&*(qf_get_curlist(qi_ptr.borrow_mut())))) {
        qf_jump(qi_ptr.opt_mut(), 0, 0, false);
    } else {
        semsg!(gettext(E_NOMATCH2), eap.arg);
    }

    decr_quickfix_busy();

    if eap.cmdidx == Cmdidx::CmdLhelpgrep {
        if !bt_help(curwin_mut().buf_opt()) || curwin_mut().w_llist == qi_ptr {
            if new_qi {
                ll_free_all(qi_ptr.opt_mut());
            }
        } else if curwin_mut().w_llist.is_null() && new_qi {
            curwin_mut().w_llist = qi_ptr;
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
            if what_arg.v_type == VarType::VarDict
                && let Some(d) = what_arg.vval.v_dict_opt()
            {
                qf_get_properties(wp, d, rettv.vval.v_dict_ref());
            } else {
                emsg(gettext(E_DICTREQ));
            }
        }
    }
}

pub fn f_getloclist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let wp = find_win_by_nr_or_id(&argvars[0]);
    get_qf_loc_list(false, wp.as_deref(), &mut argvars[1], rettv);
}

pub fn f_getqflist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_qf_loc_list(true, None, &mut argvars[0], rettv);
}

fn set_qf_ll_list(wp: Option<&mut WinInfoInfo>, args: &mut [TypVal], rettv: &mut TypVal) {
    let mut title: Option<&CStr> = None;
    let mut action: u8 = b' ';
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    let mut what: Option<&TvDict> = None;

    if args[0].v_type != VarType::VarList {
        emsg(gettext(E_LISTREQ));
        return;
    } else if qf_st().set_qf_ll_recursive != 0 {
        emsg(gettext(E_AU_RECURSIVE));
        return;
    }

    if args[1].v_type != VarType::VarUnknown {
        if args[1].v_type != VarType::VarString {
            emsg(gettext(E_STRING_REQUIRED));
            return;
        }
        let Some(act) = tv_get_string_chk(&args[1]) else {
            return;
        };
        let act_bytes = act.to_bytes();
        if matches!(act_bytes, b"a" | b"r" | b"u" | b" " | b"f") {
            action = act_bytes[0];
        } else {
            semsg!(gettext(E_INVACT), act);
            return;
        }

        if args[2].v_type != VarType::VarUnknown {
            if args[2].v_type == VarType::VarString {
                let Some(t) = tv_get_string_chk(&args[2]) else {
                    return;
                };
                title = Some(t);
            } else if args[2].v_type == VarType::VarDict
                && let Some(d) = args[2].vval.v_dict_opt()
            {
                what = Some(d);
            } else {
                emsg(gettext(E_DICTREQ));
                return;
            }
        }
    }

    if title.is_none() {
        title = Some(if wp.is_some() {
            c":setloclist()"
        } else {
            c":setqflist()"
        });
    }

    qf_st().set_qf_ll_recursive += 1;
    let l = args[0].vval.v_list_opt().map(|r| &*r);
    if set_errorlist(wp, l, i32::from(action), title, what) {
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
    }
    qf_st().set_qf_ll_recursive -= 1;
}

pub fn f_setloclist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if let Some(win) = find_win_by_nr_or_id(&argvars[0]) {
        set_qf_ll_list(Some(win), &mut argvars[1..], rettv);
    }
}

pub fn f_setqflist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    set_qf_ll_list(None, argvars, rettv);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efm_option_part_len_handles_escapes_and_commas() {
        assert_eq!(efm_option_part_len(b"%f:%l:%m,%f:%l"), 8);
        assert_eq!(efm_option_part_len(b"%f\\,bar:%l,%m"), 10);
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
