//! Quickfix and location list commands and functions (`src/nvim/quickfix.c`).

use crate::api::private::helpers::{boolean_obj, cbuf_as_string, cstr_as_obj, cstr_as_string};
use crate::arglist::{ML_EMPTY, get_arglist_exp};
use crate::autocmd::{
    EVENT_BUFREADPOST, EVENT_BUFWINENTER, EVENT_FILETYPE, EVENT_QUICKFIXCMDPOST,
    EVENT_QUICKFIXCMDPRE, OPT_NOWIN, apply_autocmds, au_event_disable, au_event_restore,
    block_autocmds, unblock_autocmds,
};
use crate::buffer::{
    BCO_ENTER, BCO_NOHELP, BF_DUMMY, BF_NEW, BLN_DUMMY, BLN_NOOPT, DOBUF_UNLOAD, DOBUF_WIPE,
    GETF_SETMARK, GETF_SWITCH, MAXLNUM, NOTDONE, VAR_FIXED, bt_help, bt_normal, bt_quickfix, buf_T,
    buf_valid, buflist_findname_exp, buflist_findnr, buflist_getfile, buflist_new, bufref_T,
    bufref_valid, close_buffer, do_modelines, kExtmarkNoUndo, no_write_message, set_bufref,
    setfname, wipe_buffer,
};
use crate::change::{TAB, changed_lines};
use crate::charset::{NL, NUL, ascii_iswhite, skipdigits, skipwhite, vim_isprintc};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{check_cursor, colnr_T, get_cursor_line_ptr, linenr_T, pos_T};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, redraw_buf_later, redraw_curbuf_later, redraw_later, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{
    Callback, EvalFuncData, VarType, callback_copy, callback_free, callback_put, dict_T,
    dictitem_T, kCallbackNone, list_T, listitem_T, tv_clear, tv_copy, tv_dict_add,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len, tv_dict_add_tv,
    tv_dict_alloc, tv_dict_alloc_lock, tv_dict_alloc_ret, tv_dict_find, tv_dict_get_bool,
    tv_dict_get_number, tv_dict_get_string, tv_dict_get_tv, tv_dict_item_alloc_len,
    tv_dict_item_free, tv_dict_unref, tv_get_number_chk, tv_get_string_chk, tv_initial_value,
    tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_first, tv_list_len, tv_list_ref,
    tv_list_unref, typval_T, varnumber_T,
};
use crate::eval::vars::set_internal_string_var;
use crate::eval::window::{find_win_by_nr_or_id, win_id2wp};
use crate::eval::{
    callback_call, callback_from_typval, eval_expr, kListLenMayKnow, set_ref_in_callback,
    set_ref_in_item,
};
use crate::ex_cmds::{
    BL_FIX, BL_WHITE, ECMD_HIDE, ECMD_NOWINENTER, ECMD_OLDBUF, ECMD_ONE, ECMD_SET_HELP, VGR_FUZZY,
    VGR_GLOBAL, VGR_NOJUMP, append_redir, check_secure, do_ecmd, do_shell, kShmOverall,
    skip_vimgrep_pat,
};
use crate::ex_cmds2::{CMD_cdo, CMD_cfdo, CMD_ldo, CMD_lfdo, autowrite_all, can_abandon};
#[cfg(windows)]
use crate::ex_docmd::CAR;
use crate::ex_docmd::{CMOD_HIDE, do_cmdline_cmd, ex_cd, exarg_T, is_loclist_cmd};
use crate::ex_eval::{aborting, cleanup_T, enter_cleanup, leave_cleanup};
use crate::ex_getln::get_list_range;
use crate::extmark::extmark_splice;
use crate::fileio::{
    EINTR, FILE, READ_DUMMY, READ_NEW, fclose, fdopen, ferror, fgets, readfile, shorten_buf_fname,
    shorten_fnames, vim_fgets, vim_tempname,
};
use crate::fold::{foldOpenCursor, foldUpdateAll};
use crate::fuzzy::{FUZZY_MATCH_MAX_LEN, fuzzy_match};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::help::check_help_lang;
use crate::highlight_group::syn_name2id;
use crate::insert::beginline;
use crate::mark::{BUF_HAS_LL_ENTRY, BUF_HAS_QF_ENTRY, setpcmark};
use crate::option::{
    OPT_LOCAL, buf_copy_options, copy_option_part, kOptBufhidden, kOptBuftype, kOptFiletype,
    kOptSwapfile, set_option_direct, set_option_value_give_err, shortmess, skip_to_option_part,
};
use crate::os::fs::{
    FileInfo, MAXPATHL, os_dirname, os_errno, os_fileinfo_link, os_fopen, os_isdir,
    os_open_stdin_fd, os_path_exists, os_remove, os_set_errno,
};
use crate::regexp::{
    RE_MAGIC, RE_STRING, regmatch_T, regmmatch_T, regprog_T, vim_regcomp, vim_regexec,
    vim_regexec_multi, vim_regfree,
};
use crate::window::{
    WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HELP, WSP_NEWLOC, WSP_QUICKFIX, WSP_TOP, WSP_VERT,
    check_can_set_curbuf_forceit, check_lnums, goto_tabpage_win, reset_binding, tabline_height,
    tabpage_T, win_T, win_close, win_enter, win_goto, win_setheight, win_setwidth, win_split,
    win_valid,
};

use crate::math::{FAIL, OK};
use crate::mbyte::{convert_setup, ptr2cells, remove_bom, string_convert, utfc_ptr2len, vimconv_T};
use crate::memline::{
    check_need_swap, ml_append_buf, ml_delete, ml_get_buf, ml_get_buf_len, ml_open,
};
use crate::memory::{xstrlcpy};
use crate::message::{
    emsg, internal_error, message_filtered, msg, msg_clr_eos, msg_ext_set_kind, msg_keep,
    msg_outtrans, msg_prt_line, msg_putchar, msg_puts, msg_puts_hl, msg_start, msg_strtrunc, semsg,
    smsg, trunc_string,
};
use crate::r#move::update_topline;
use crate::msgpack_rpc::packer::handle_T;
use crate::normal::reset_VIsual_and_resel;
use crate::ops::get_region_bytecount;
use crate::option_vars::*;
use crate::optionstr::{OptIndex, free_string_option};
use crate::os::env::{expand_env, os_get_pid};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::os::time::{os_time, time_t};
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{
    EW_FILE, EW_SILENT, add_pathsep, concat_fnames, fix_fname, gen_expand_wildcards_vec,
    kPathCmpLiteral, path_equal, path_is_absolute, path_tail, path_try_shorten_fname,
    vim_isAbsName,
};
use crate::search::{BACKWARD, Direction, FORWARD, do_search, last_search_pat};
use crate::strings::{
    gettext, vim_snprintf, vim_snprintf_safelen,
    vim_strchr,
};
use crate::ui::ui_flush;
use crate::undo::u_clearallandblockfree;
use std::ffi::{CStr, c_char, c_int, c_longlong, c_uint};
use std::ptr;

use crate::api::vim::HLF_N;
use crate::cmdexpand::HLF_D;
use crate::diff::kOptFoldmethod;
use crate::drawline::HLF_QFL;
use crate::eval::funcs::{SEARCH_KEEP, kOptCpoptions};
use crate::viml::parser::parser::CONV_NONE;

const INVALID_QFIDX: c_int = -1;
const INVALID_QFBUFNR: c_int = 0;
const FMT_PATTERNS: usize = 14;
const FMT_PATTERN_M: usize = 8;
const FMT_PATTERN_R: usize = 9;
const QF_WINHEIGHT: c_int = 10;
const LINE_MAXLEN: usize = 4096;

const QF_FAIL: c_int = 0;
const QF_OK: c_int = 1;
const QF_END_OF_INPUT: c_int = 2;
const QF_NOMEM: c_int = 3;
const QF_IGNORE_LINE: c_int = 4;
const QF_MULTISCAN: c_int = 5;
const QF_ABORT: c_int = 6;

const QF_GETLIST_NONE: c_int = 0x0;
const QF_GETLIST_TITLE: c_int = 0x1;
const QF_GETLIST_ITEMS: c_int = 0x2;
const QF_GETLIST_NR: c_int = 0x4;
const QF_GETLIST_WINID: c_int = 0x8;
const QF_GETLIST_CONTEXT: c_int = 0x10;
const QF_GETLIST_ID: c_int = 0x20;
const QF_GETLIST_IDX: c_int = 0x40;
const QF_GETLIST_SIZE: c_int = 0x80;
const QF_GETLIST_TICK: c_int = 0x100;
const QF_GETLIST_FILEWINID: c_int = 0x200;
const QF_GETLIST_QFBUFNR: c_int = 0x400;
const QF_GETLIST_QFTF: c_int = 0x800;
const QF_GETLIST_ALL: c_int = 0xFFF;

pub(crate) const CMDBUFFSIZE: usize = 1024;

const FORWARD_FILE: Direction = 3;
const BACKWARD_FILE: Direction = -3;

pub(crate) const kOptSwbFlagUsetab: c_uint = 0x02;
const kOptSwbFlagUselast: c_uint = 0x20;
const kOptFdoFlagQuickfix: c_uint = 0x20;

pub(crate) const kOptErrorfile: OptIndex = 86;

const CMD_cNext: c_int = 46;
const CMD_cNfile: c_int = 47;
const CMD_cabove: c_int = 50;
const CMD_caddbuffer: c_int = 51;
const CMD_caddexpr: c_int = 52;
const CMD_caddfile: c_int = 53;
const CMD_cafter: c_int = 54;
const CMD_cbuffer: c_int = 57;
const CMD_cbefore: c_int = 58;
const CMD_cbelow: c_int = 59;
const CMD_cc: c_int = 61;
const CMD_cd: c_int = 63;
const CMD_cexpr: c_int = 66;
const CMD_cfile: c_int = 67;
const CMD_cfirst: c_int = 69;
const CMD_cgetfile: c_int = 70;
const CMD_cgetbuffer: c_int = 71;
const CMD_cgetexpr: c_int = 72;
const CMD_cnfile: c_int = 88;
const CMD_colder: c_int = 93;
const CMD_cprevious: c_int = 103;
const CMD_cpfile: c_int = 104;
const CMD_crewind: c_int = 106;
const CMD_grep: c_int = 175;
const CMD_grepadd: c_int = 176;
const CMD_helpgrep: c_int = 181;
const CMD_lNext: c_int = 214;
const CMD_lNfile: c_int = 215;
const CMD_labove: c_int = 217;
const CMD_laddexpr: c_int = 219;
const CMD_laddbuffer: c_int = 220;
const CMD_laddfile: c_int = 221;
const CMD_lafter: c_int = 222;
const CMD_lbuffer: c_int = 224;
const CMD_lbelow: c_int = 226;
const CMD_lcd: c_int = 228;
const CMD_lexpr: c_int = 235;
const CMD_lfile: c_int = 236;
const CMD_lfirst: c_int = 238;
const CMD_lgetfile: c_int = 239;
const CMD_lgetbuffer: c_int = 240;
const CMD_lgetexpr: c_int = 241;
const CMD_lgrep: c_int = 242;
const CMD_lgrepadd: c_int = 243;
const CMD_lhelpgrep: c_int = 244;
const CMD_ll: c_int = 246;
const CMD_lmake: c_int = 251;
const CMD_lnfile: c_int = 255;
const CMD_lolder: c_int = 261;
const CMD_lprevious: c_int = 263;
const CMD_lpfile: c_int = 264;
const CMD_lrewind: c_int = 265;
const CMD_lvimgrep: c_int = 271;
const CMD_lvimgrepadd: c_int = 272;
const CMD_make: c_int = 278;
const CMD_vimgrep: c_int = 517;
const CMD_vimgrepadd: c_int = 518;

const E_NO_MORE_ITEMS: &CStr = c"E553: No more items";
const E_CURRENT_QUICKFIX_LIST_WAS_CHANGED: &CStr = c"E925: Current quickfix list was changed";
const E_CURRENT_LOCATION_LIST_WAS_CHANGED: &CStr = c"E926: Current location list was changed";
const E_INVACT: &CStr = c"E927: Invalid action: '%s'";

#[repr(C)]
pub struct qfline_T {
    pub qf_lnum: linenr_T,
    pub qf_end_lnum: linenr_T,
    pub qf_fnum: c_int,
    pub qf_col: c_int,
    pub qf_end_col: c_int,
    pub qf_nr: c_int,
    pub qf_module: Option<std::ffi::CString>,
    pub qf_fname: Option<std::ffi::CString>,
    pub qf_pattern: Option<std::ffi::CString>,
    pub qf_text: std::ffi::CString,
    pub qf_viscol: c_char,
    pub qf_cleared: c_char,
    pub qf_type: c_char,
    pub qf_user_data: typval_T,
    pub qf_valid: c_char,
}

impl qfline_T {
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

impl Drop for qfline_T {
    fn drop(&mut self) {
        tv_clear(&raw mut self.qf_user_data);
    }
}

pub type qfltype_T = c_int;
pub const QFLT_QUICKFIX: qfltype_T = 0;
pub const QFLT_LOCATION: qfltype_T = 1;
pub const QFLT_INTERNAL: qfltype_T = 2;

#[repr(C)]
pub struct qf_list_T {
    pub qf_id: c_uint,
    pub qfl_type: qfltype_T,
    pub qf_items: Vec<qfline_T>,
    pub qf_index: c_int,
    pub qf_nonevalid: bool,
    pub qf_has_user_data: bool,
    pub qf_title: Option<std::ffi::CString>,
    pub qf_ctx: Option<typval_T>,
    pub qf_qftf_cb: Callback,
    pub qf_dir_stack: Vec<std::ffi::CString>,
    pub qf_file_stack: Vec<std::ffi::CString>,
    pub qf_multiline: bool,
    pub qf_multiignore: bool,
    pub qf_multiscan: bool,
    pub qf_changedtick: c_int,
}

impl qf_list_T {
    #[inline]
    pub fn directory_ptr(&self) -> *mut c_char {
        self.qf_dir_stack
            .last()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
    pub fn currfile_ptr(&self) -> *mut c_char {
        self.qf_file_stack
            .last()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
    pub fn count(&self) -> c_int {
        self.qf_items.len() as c_int
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
    pub fn cur_entry(&mut self) -> *mut qfline_T {
        let idx = self.cur_idx();
        if idx < self.qf_items.len() {
            &raw mut self.qf_items[idx]
        } else {
            ptr::null_mut()
        }
    }
}

impl Default for qf_list_T {
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

impl Drop for qf_list_T {
    fn drop(&mut self) {
        qf_free(self);
    }
}

#[repr(C)]
pub struct qf_info_T {
    pub qf_refcount: c_int,
    pub qf_listcount: c_int,
    pub qf_curlist: c_int,
    pub qf_maxcount: c_int,
    pub qf_lists: Vec<qf_list_T>,
    pub qfl_type: qfltype_T,
    pub qf_bufnr: c_int,
}

#[repr(C)]
struct efm_T {
    prog: Option<Box<regprog_T>>,
    addr: [c_char; FMT_PATTERNS],
    prefix: c_char,
    flags: c_char,
    conthere: c_int,
}

enum QfInputSource {
    None,
    File(crate::os::fs::CFile),
    Str(*mut c_char),
    List(*mut listitem_T),
    Buf {
        buf: *mut buf_T,
        buflnum: linenr_T,
        lnumlast: linenr_T,
    },
}

struct qfstate_T {
    use_growbuf: bool,
    linelen: usize,
    growbuf: Vec<c_char>,
    source: QfInputSource,
    vc: vimconv_T,
}

impl qfstate_T {
    #[inline]
    fn linebuf_ptr(&mut self) -> *mut c_char {
        if self.use_growbuf {
            self.growbuf.as_mut_ptr()
        } else {
            (&raw mut g_st().IObuff).cast::<c_char>()
        }
    }
}

impl Default for qfstate_T {
    fn default() -> Self {
        Self {
            use_growbuf: false,
            linelen: 0,
            growbuf: Vec::new(),
            source: QfInputSource::None,
            vc: vimconv_T::new(),
        }
    }
}

#[repr(C)]
struct qffields_T {
    namebuf: Vec<c_char>,
    bnr: c_int,
    module: Vec<c_char>,
    errmsg: Vec<c_char>,
    lnum: linenr_T,
    end_lnum: linenr_T,
    col: c_int,
    end_col: c_int,
    use_viscol: bool,
    pattern: Vec<c_char>,
    enr: c_int,
    type_: c_char,
    valid: bool,
}

impl Default for qffields_T {
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
struct vgr_args_T {
    tomatch: c_int,
    spat: Option<std::ffi::CString>,
    flags: c_int,
    fnames: Vec<std::ffi::CString>,
    regmatch: regmmatch_T,
    qf_title: Option<std::ffi::CString>,
}

impl vgr_args_T {
    const fn new() -> Self {
        Self {
            tomatch: 0,
            spat: None,
            flags: 0,
            fnames: Vec::new(),
            regmatch: regmmatch_T::new(),
            qf_title: None,
        }
    }

    #[inline]
    fn spat_ptr(&self) -> *mut c_char {
        self.spat
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

impl Default for vgr_args_T {
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
    ql_info_actual: qf_info_T,
    last_qf_id: c_uint,
    qf_last_bufname: Option<std::ffi::CString>,
    qf_last_bufref: bufref_T,
    qfga: Vec<c_char>,
    quickfix_busy: c_int,
    qf_delq: Vec<*mut qf_info_T>,
    fmt_start_idx: Option<usize>,
    fmt_cache: Vec<efm_T>,
    last_efm_cache: Option<std::ffi::CString>,
    qftitle_str: [c_char; IOSIZE],
    qfFile_hl_id: c_int,
    qfSep_hl_id: c_int,
    qfLine_hl_id: c_int,
    qf_types_cc: [c_char; 3],
    qf_types_buf: [c_char; 20],
    qftf_recursive: bool,
    mef_start: c_int,
    mef_off: c_int,
    did_bufnr_emsg: bool,
    set_qf_ll_recursive: c_int,
}

impl QuickfixState {
    const fn new() -> Self {
        Self {
            ql_info_actual: qf_info_T {
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
            qf_last_bufref: bufref_T {
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
            qftitle_str: [0; IOSIZE],
            qfFile_hl_id: 0,
            qfSep_hl_id: 0,
            qfLine_hl_id: 0,
            qf_types_cc: [0; 3],
            qf_types_buf: [0; 20],
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
fn ql_info_ptr() -> *mut qf_info_T {
    unsafe { &raw mut (*QF_STATE.as_ptr()).ql_info_actual }
}


#[inline]
fn c_strcat(dst: *mut c_char, src: impl crate::strings::IntoCCharPtr) {
    let src = src.into_c_char_ptr();
    unsafe {
        let dlen = crate::memory::cstr_len(dst);
        let slen = crate::memory::cstr_len(src);
        ptr::copy_nonoverlapping(src, dst.add(dlen), slen + 1);
    }
}

#[inline]
fn is_qf_window(wp: &win_T) -> bool {
    bt_quickfix(wp.w_buffer) && wp.w_llist_ref.is_null()
}

#[inline]
fn is_ll_window(wp: &win_T) -> bool {
    bt_quickfix(wp.w_buffer) && !wp.w_llist_ref.is_null()
}

#[inline]
fn is_qf_stack(qi: &qf_info_T) -> bool {
    qi.qfl_type == QFLT_QUICKFIX
}

#[inline]
fn is_ll_stack(qi: &qf_info_T) -> bool {
    qi.qfl_type == QFLT_LOCATION
}

#[inline]
fn is_qf_list(qfl: &qf_list_T) -> bool {
    qfl.qfl_type == QFLT_QUICKFIX
}

#[inline]
fn get_loc_list(wp: &win_T) -> *mut qf_info_T {
     {
        if is_ll_window(wp) {
            wp.w_llist_ref
        } else {
            wp.w_llist
        }
    }
}

#[inline]
fn tab_firstwin(tp: &tabpage_T) -> *mut win_T {
    if ptr::eq(tp, g_st().curtab) {
            g_st().firstwin
        } else {
            tp.tp_firstwin
        }
}

fn qfga_get() -> &'static mut Vec<c_char> {
    unsafe {
        let gap = &raw mut qf_st().qfga;
        if (*gap).capacity() == 0 {
            (*gap).reserve(256);
        }
        (*gap).clear();
        &mut *gap
    }
}

fn qfga_clear() {
    unsafe {
        let gap = &raw mut qf_st().qfga;
        if (*gap).capacity() > 1000 {
            *gap = Vec::new();
        } else {
            (*gap).clear();
        }
    }
}

#[inline]
fn qfga_concat(gap: &mut Vec<c_char>, s: *const c_char) {
    unsafe {
        if !s.is_null() {
            let len = crate::memory::cstr_len(s);
            if len > 0 {
                gap.extend_from_slice(std::slice::from_raw_parts(s, len));
            }
        }
    }
}

#[inline]
fn qfga_concat_len(gap: &mut Vec<c_char>, s: *const c_char, len: usize) {
    unsafe {
        if !s.is_null() && len > 0 {
            gap.extend_from_slice(std::slice::from_raw_parts(s, len));
        }
    }
}

fn qf_init_process_nextline(
    qfl: &mut qf_list_T,
    fmt_first: &[efm_T],
    state: &mut qfstate_T,
    fields: &mut qffields_T,
) -> c_int {
    unsafe {
        let mut status = qf_get_nextline(state);
        if status != QF_OK {
            return status;
        }

        status = qf_parse_line(
            qfl,
            state.linebuf_ptr(),
            state.linelen,
            fmt_first,
            fields,
        );
        if status != QF_OK {
            return status;
        }

        let fname = if *fields.namebuf.as_ptr() != NUL || !qfl.qf_dir_stack.is_empty() {
            fields.namebuf.as_mut_ptr()
        } else if !qfl.qf_file_stack.is_empty() && fields.valid {
            qfl.currfile_ptr()
        } else {
            ptr::null_mut()
        };

        qf_add_entry(
            qfl,
            qfl.directory_ptr(),
            fname,
            fields.module.as_mut_ptr(),
            fields.bnr,
            fields.errmsg.as_mut_ptr(),
            fields.lnum,
            fields.end_lnum,
            fields.col,
            fields.end_col,
            c_char::from(fields.use_viscol),
            fields.pattern.as_mut_ptr(),
            fields.enr,
            fields.type_,
            None,
            c_char::from(fields.valid),
        )
    }
}

pub fn qf_init(
    wp: *mut win_T,
    efile: *const c_char,
    errorformat: *mut c_char,
    newlist: c_int,
    qf_title: *const c_char,
    enc: *mut c_char,
) -> c_int {
    unsafe {
        let qi = if wp.is_null() {
            ql_info_ptr()
        } else {
            ll_get_or_alloc_list( &mut *wp)
        };
        assert!(!qi.is_null());

        qf_init_ext(
            &mut *qi,
            (*qi).qf_curlist,
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

fn efmpat_to_regpat(
    efmpat: *const c_char,
    mut regpat: *mut c_char,
    efminfo: &mut efm_T,
    idx: usize,
    mut round: c_int,
) -> *mut c_char {
    unsafe {
        if efminfo.addr[idx] != 0 {
            semsg!(
                gettext(c"E372: Too many %%%c in format string"),
                c_int::from(*efmpat),
            );
            return ptr::null_mut();
        }
        if (idx != 0
            && idx < FMT_PATTERN_R
            && !vim_strchr(c"DXOPQ", c_int::from(efminfo.prefix as u8)).is_null())
            || (idx == FMT_PATTERN_R
                && vim_strchr(c"OPQ", c_int::from(efminfo.prefix as u8)).is_null())
        {
            semsg!(
                gettext(c"E373: Unexpected %%%c in format string"),
                c_int::from(*efmpat),
            );
            return ptr::null_mut();
        }
        round += 1;
        efminfo.addr[idx] = round as c_char;
        *regpat = b'\\' as c_char;
        regpat = regpat.add(1);
        *regpat = b'(' as c_char;
        regpat = regpat.add(1);
        #[cfg(windows)]
        if *efmpat == b'f' as c_char {
            crate::memory::cstr_cpy(regpat, c"\\%(\\a:\\)\\=");
            regpat = regpat.add(10);
        }
        if *efmpat == b'f' as c_char && *efmpat.add(1) != NUL {
            if *efmpat.add(1) != b'\\' as c_char && *efmpat.add(1) != b'%' as c_char {
                crate::memory::cstr_cpy(regpat, c".\\{-1,}");
                regpat = regpat.add(7);
            } else {
                crate::memory::cstr_cpy(regpat, c"\\f\\+");
                regpat = regpat.add(4);
            }
        } else {
            let mut srcptr = FMT_PAT[idx].pattern.as_ptr();
            while {
                *regpat = *srcptr;
                srcptr = srcptr.add(1);
                *regpat != NUL
            } {
                regpat = regpat.add(1);
            }
        }
        *regpat = b'\\' as c_char;
        regpat = regpat.add(1);
        *regpat = b')' as c_char;
        regpat = regpat.add(1);

        regpat
    }
}

fn scanf_fmt_to_regpat(
    pefmp: *mut *const c_char,
    efm: *const c_char,
    len: c_int,
    mut regpat: *mut c_char,
) -> *mut c_char {
    unsafe {
        let mut efmp = *pefmp;
        let end = efm.add(len as usize);

        if *efmp == b'[' as c_char || *efmp == b'\\' as c_char {
            *regpat = *efmp;
            let ch = *regpat;
            regpat = regpat.add(1);
            if ch == b'[' as c_char {
                if *efmp.add(1) == b'^' as c_char {
                    efmp = efmp.add(1);
                    *regpat = *efmp;
                    regpat = regpat.add(1);
                }
                if efmp < end {
                    efmp = efmp.add(1);
                    *regpat = *efmp;
                    regpat = regpat.add(1);
                    while efmp < end && {
                        efmp = efmp.add(1);
                        *regpat = *efmp;
                        let c = *regpat;
                        regpat = regpat.add(1);
                        c != b']' as c_char
                    } {}
                    if efmp == end {
                        emsg(gettext(c"E374: Missing ] in format string"));
                        return ptr::null_mut();
                    }
                }
            } else if efmp < end {
                efmp = efmp.add(1);
                *regpat = *efmp;
                regpat = regpat.add(1);
            }
            *regpat = b'\\' as c_char;
            regpat = regpat.add(1);
            *regpat = b'+' as c_char;
            regpat = regpat.add(1);
        } else {
            semsg!(
                gettext(c"E375: Unsupported %%%c in format string"),
                c_int::from(*efmp),
            );
            return ptr::null_mut();
        }

        *pefmp = efmp;
        regpat
    }
}

fn efm_analyze_prefix(mut efmp: *const c_char, efminfo: &mut efm_T) -> *const c_char {
    unsafe {
        if !vim_strchr(c"+-", c_int::from(*efmp as u8)).is_null() {
            efminfo.flags = *efmp;
            efmp = efmp.add(1);
        }
        if !vim_strchr(c"DXAEWINCZGOPQ", c_int::from(*efmp as u8)).is_null() {
            efminfo.prefix = *efmp;
        } else {
            semsg!(
                gettext(c"E376: Invalid %%%c in format string prefix"),
                c_int::from(*efmp),
            );
            return ptr::null();
        }
        efmp
    }
}

fn efm_to_regpat(
    efm: *const c_char,
    len: c_int,
    fmt_ptr: &mut efm_T,
    regpat: *mut c_char,
) -> bool {
    unsafe {
        let mut ptr = regpat;
        *ptr = b'^' as c_char;
        ptr = ptr.add(1);
        let mut round: c_int = 0;
        let end = efm.add(len as usize);
        let mut efmp = efm;
        while efmp < end {
            if *efmp == b'%' as c_char {
                efmp = efmp.add(1);
                let mut idx: usize = 0;
                while idx < FMT_PATTERNS {
                    if FMT_PAT[idx].convchar as c_char == *efmp {
                        break;
                    }
                    idx += 1;
                }
                if idx < FMT_PATTERNS {
                    ptr = efmpat_to_regpat(efmp, ptr, fmt_ptr, idx, round);
                    if ptr.is_null() {
                        return false;
                    }
                    round += 1;
                } else if *efmp == b'*' as c_char {
                    efmp = efmp.add(1);
                    ptr = scanf_fmt_to_regpat(&raw mut efmp, efm, len, ptr);
                    if ptr.is_null() {
                        return false;
                    }
                } else if !vim_strchr(c"%\\.^$~[", c_int::from(*efmp as u8)).is_null() {
                    *ptr = *efmp;
                    ptr = ptr.add(1);
                } else if *efmp == b'#' as c_char {
                    *ptr = b'*' as c_char;
                    ptr = ptr.add(1);
                } else if *efmp == b'>' as c_char {
                    fmt_ptr.conthere = 1;
                } else if efmp == efm.add(1) {
                    efmp = efm_analyze_prefix(efmp, fmt_ptr);
                    if efmp.is_null() {
                        return false;
                    }
                } else {
                    semsg!(
                        gettext(c"E377: Invalid %%%c in format string"),
                        c_int::from(*efmp),
                    );
                    return false;
                }
            } else {
                if *efmp == b'\\' as c_char && efmp.add(1) < end {
                    efmp = efmp.add(1);
                } else if !vim_strchr(c".*^$~[", c_int::from(*efmp as u8)).is_null() {
                    *ptr = b'\\' as c_char;
                    ptr = ptr.add(1);
                }
                if *efmp != NUL {
                    *ptr = *efmp;
                    ptr = ptr.add(1);
                }
            }
            efmp = efmp.add(1);
        }
        *ptr = b'$' as c_char;
        ptr = ptr.add(1);
        *ptr = NUL;

        true
    }
}

fn efm_regpat_bufsz(efm: *const c_char) -> usize {
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

fn efm_option_part_len(efm: impl crate::strings::IntoCCharPtr) -> c_int {
    let efm = efm.into_c_char_ptr();
    unsafe {
        let mut len: usize = 0;
        while *efm.add(len) != NUL && *efm.add(len) != b',' as c_char {
            if *efm.add(len) == b'\\' as c_char && *efm.add(len + 1) != NUL {
                len += 1;
            }
            len += 1;
        }
        len as c_int
    }
}

fn parse_efm_option(mut efm: *mut c_char) -> Vec<efm_T> {
    unsafe {
        let mut fmt_list: Vec<efm_T> = Vec::new();

        let sz = efm_regpat_bufsz(efm);
        let mut fmtstr = vec![0 as c_char; sz];

        let mut errored = false;
        while *efm != NUL {
            let mut fmt_item = efm_T {
                prog: None,
                addr: [0; FMT_PATTERNS],
                prefix: 0,
                flags: 0,
                conthere: 0,
            };

            let len = efm_option_part_len(efm);

            if !efm_to_regpat(efm, len, &mut fmt_item, fmtstr.as_mut_ptr()) {
                errored = true;
                break;
            }
            fmt_item.prog = crate::regexp::vim_regcomp_box(fmtstr.as_ptr(), RE_MAGIC + RE_STRING);
            if fmt_item.prog.is_none() {
                errored = true;
                break;
            }
            fmt_list.push(fmt_item);
            efm = skip_to_option_part(efm.add(len as usize));
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

fn qf_grow_linebuf(state: &mut qfstate_T, newsz: usize) -> *mut c_char {
    {
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
}

fn qf_get_next_str_line(state: &mut qfstate_T, mut p_str: *mut c_char) -> c_int {
    unsafe {
        if *p_str == NUL {
            return QF_END_OF_INPUT;
        }

        let p = vim_strchr(p_str, b'\n' as c_int);
        let len = if !p.is_null() {
            p.offset_from(p_str) as usize + 1
        } else {
            crate::memory::cstr_len(p_str)
        };

        let linebuf = if len > IOSIZE - 2 {
            qf_grow_linebuf(state, len)
        } else {
            state.use_growbuf = false;
            state.linelen = len;
            (&raw mut g_st().IObuff).cast::<c_char>()
        };
        ptr::copy_nonoverlapping(p_str, linebuf, state.linelen);
        *linebuf.add(state.linelen) = NUL;

        p_str = p_str.add(len);
        state.source = QfInputSource::Str(p_str);

        QF_OK
    }
}

fn qf_get_next_list_line(state: &mut qfstate_T, mut p_li: *mut listitem_T) -> c_int {
    unsafe {
        while !p_li.is_null()
            && ((*p_li).li_tv.v_type != VarType::VAR_STRING
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
            (&raw mut g_st().IObuff).cast::<c_char>()
        };

        xstrlcpy(linebuf, s, state.linelen + 1);
        state.source = QfInputSource::List((*p_li).li_next);
        QF_OK
    }
}

fn qf_get_next_buf_line(
    state: &mut qfstate_T,
    buf: *mut buf_T,
    buflnum: linenr_T,
    lnumlast: linenr_T,
) -> c_int {
    {
        if buflnum > lnumlast {
            return QF_END_OF_INPUT;
        }
        let p_buf = ml_get_buf(buf, buflnum);
        let len = ml_get_buf_len(buf, buflnum) as usize;
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
            (&raw mut g_st().IObuff).cast::<c_char>()
        };
        xstrlcpy(linebuf, p_buf, state.linelen + 1);
        QF_OK
    }
}

fn qf_get_next_file_line(state: &mut qfstate_T, fd: *mut FILE) -> c_int {
    unsafe {
        let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        loop {
            os_set_errno(0);
            if fgets(iobuff_ptr, IOSIZE as c_int, fd).is_null() {
                if os_errno() == EINTR {
                    continue;
                }
                return QF_END_OF_INPUT;
            }
            break;
        }

        let mut discard = false;
        state.linelen = crate::memory::cstr_len(iobuff_ptr);
        if state.linelen == IOSIZE - 1 && *iobuff_ptr.add(state.linelen - 1) != NL {
            if state.growbuf.len() < 2 * (IOSIZE - 1) {
                state.growbuf.resize(2 * (IOSIZE - 1), 0);
            }

            ptr::copy_nonoverlapping(iobuff_ptr, state.growbuf.as_mut_ptr(), IOSIZE - 1);
            let mut growbuflen = state.linelen;
            *state.growbuf.as_mut_ptr().add(growbuflen) = NUL;

            loop {
                os_set_errno(0);
                let growbufsiz = state.growbuf.len();
                if fgets(
                    state.growbuf.as_mut_ptr().add(growbuflen),
                    (growbufsiz - growbuflen) as c_int,
                    fd,
                )
                .is_null()
                {
                    if os_errno() == EINTR {
                        continue;
                    }
                    break;
                }
                state.linelen = crate::memory::cstr_len(state.growbuf.as_ptr().add(growbuflen));
                growbuflen += state.linelen;
                if *state.growbuf.as_ptr().add(growbuflen - 1) == NL {
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
                    if fgets(iobuff_ptr, IOSIZE as c_int, fd).is_null() {
                        if os_errno() == EINTR {
                            continue;
                        }
                        break;
                    }
                    if crate::memory::cstr_len(iobuff_ptr) < IOSIZE - 1 || *iobuff_ptr.add(IOSIZE - 2) == NL {
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
            && !cur_linebuf.is_null() && !crate::memory::cstr_as_bytes(cur_linebuf).is_ascii()
            && let Some(line) =
                string_convert(&state.vc, cur_linebuf, Some(&mut state.linelen))
        {
            if state.linelen < IOSIZE {
                xstrlcpy(cur_linebuf, line.as_ptr().cast(), state.linelen + 1);
            } else {
                let cap = (state.linelen + 1).min(LINE_MAXLEN);
                if state.growbuf.len() < cap {
                    state.growbuf.resize(cap, 0);
                }
                xstrlcpy(state.growbuf.as_mut_ptr(), line.as_ptr().cast(), cap);
                state.use_growbuf = true;
            }
        }
        QF_OK
    }
}

fn qf_get_nextline(state: &mut qfstate_T) -> c_int {
    unsafe {
        let status = match state.source {
            QfInputSource::None => QF_FAIL,
            QfInputSource::File(ref fd) => qf_get_next_file_line(state, fd.as_ptr()),
            QfInputSource::Str(p_str) => qf_get_next_str_line(state, p_str),
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
        if state.linelen > 0 && *linebuf.add(state.linelen - 1) == NL {
            *linebuf.add(state.linelen - 1) = NUL;
            #[cfg(windows)]
            if state.linelen > 1 && *linebuf.add(state.linelen - 2) == CAR {
                *linebuf.add(state.linelen - 2) = NUL;
            }
        }

        remove_bom(linebuf);

        QF_OK
    }
}

#[inline]
fn qf_stack_empty(qi: *const qf_info_T) -> bool {
    qi.is_null() || unsafe { (*qi).qf_listcount <= 0 }
}

#[inline]
fn qf_list_empty(qfl: *const qf_list_T) -> bool {
    qfl.is_null() || unsafe { (*qfl).qf_items.is_empty() }
}

#[inline]
fn qf_list_has_valid_entries(qfl: &qf_list_T) -> bool {
    !qf_list_empty(qfl) && !qfl.qf_nonevalid
}

#[inline]
fn qf_get_list(qi: &mut qf_info_T, idx: c_int) -> &'static mut qf_list_T {
    unsafe { &mut *qi.qf_lists.as_mut_ptr().add(idx as usize) }
}

fn qf_parse_line(
    qfl: &mut qf_list_T,
    mut linebuf: *mut c_char,
    mut linelen: usize,
    fmt_first: &[efm_T],
    fields: &mut qffields_T,
) -> c_int {
     {
        let fmt_len = fmt_first.len();
        loop {
            let start_idx = qf_st().fmt_start_idx;
            qf_st().fmt_start_idx = None;
            let mut fmt_idx = start_idx.unwrap_or(0);
            let mut fmt_opt: Option<&efm_T> = None;

            let mut idx: c_int = 0;
            let mut tail: *mut c_char = ptr::null_mut();
            let mut status;

            fields.valid = true;
            while fmt_idx < fmt_len {
                let cur_fmt = &fmt_first[fmt_idx];
                idx = c_int::from(cur_fmt.prefix as u8);
                status = qf_parse_get_fields(
                    linebuf,
                    linelen,
                    cur_fmt,
                    fields,
                    c_int::from(qfl.qf_multiline),
                    c_int::from(qfl.qf_multiscan),
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

            if fmt_opt.is_none() || idx == c_int::from(b'D') || idx == c_int::from(b'X') {
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

                if !vim_strchr(c"AEWIN", idx).is_null() {
                    qfl.qf_multiline = true;
                    qfl.qf_multiignore = false;
                } else if !vim_strchr(c"CZ", idx).is_null() {
                    status = qf_parse_multiline_pfx(idx, qfl, fields);
                    if status != QF_OK {
                        return status;
                    }
                } else if !vim_strchr(c"OPQ", idx).is_null() {
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
                if fmt_ptr.flags == b'-' as c_char {
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
    pstate: &mut qfstate_T,
    enc: *mut c_char,
    efile: *const c_char,mut tv: Option<&mut typval_T>,
    buf: *mut buf_T,
    lnumfirst: linenr_T,
    lnumlast: linenr_T,
) -> bool {
    unsafe {
        pstate.vc.vc_type = CONV_NONE;
        if !enc.is_null() && *enc != NUL {
            convert_setup(&mut pstate.vc, enc, opt_vars().p_enc);
        }

        if !efile.is_null() {
            let raw_fd = if crate::memory::cstr_as_opt_bytes(efile) == Some(b"-") {
                fdopen(os_open_stdin_fd(), c"r".as_ptr())
            } else {
                os_fopen(efile, c"r").cast::<FILE>()
            };
            let Some(fd) = crate::os::fs::CFile::from_raw(raw_fd) else {
                semsg!(gettext(e_openerrf), efile);
                return false;
            };
            pstate.source = QfInputSource::File(fd);
        } else if tv.is_some() {
            if tv.as_deref_mut().unwrap().v_type == VarType::VAR_STRING {
                pstate.source = QfInputSource::Str(tv.as_deref_mut().unwrap().vval.v_string());
            } else if tv.as_deref_mut().unwrap().v_type == VarType::VAR_LIST {
                pstate.source = QfInputSource::List(tv_list_first(tv.unwrap().vval.v_list()));
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
    qi: &mut qf_info_T,
    mut qf_idx: c_int,
    efile: *const c_char,
    buf: *mut buf_T,mut tv: Option<&mut typval_T>,
    errorformat: *mut c_char,
    newlist: bool,
    lnumfirst: linenr_T,
    lnumlast: linenr_T,
    qf_title: *const c_char,
    enc: *mut c_char,
) -> c_int {
    unsafe {
        let mut state = qfstate_T::default();
        let mut fields = qffields_T::default();
        let mut old_count: usize = 0;
        let mut retval: c_int = -1;

        qf_st().qf_last_bufname = None;

        if !qf_setup_state(&mut state, enc, efile, tv.as_deref_mut(), buf, lnumfirst, lnumlast) {
            if qf_idx == qi.qf_curlist {
                qf_update_buffer(qi, old_count);
            }
            return retval;
        }

        let qfl: &mut qf_list_T;
        let mut adding = false;
        if newlist || qf_idx == qi.qf_listcount {
            qf_new_list(qi, qf_title);
            qf_idx = qi.qf_curlist;
            qfl = qf_get_list(&mut *qi, qf_idx);
        } else {
            adding = true;
            qfl = qf_get_list(&mut *qi, qf_idx);
            if !qf_list_empty(qfl) {
                old_count = qfl.qf_items.len();
            }
        }

        let efm =
            if errorformat == opt_vars().p_efm && tv.is_none() && !buf.is_null() && *(*buf).b_p_efm != NUL {
                (*buf).b_p_efm
            } else {
                errorformat
            };

        let cache_ptr = &raw mut qf_st().fmt_cache;
        let efm_changed = (&raw const qf_st().last_efm_cache)
            .as_ref()
            .unwrap()
            .as_ref()
            .is_none_or(|c| crate::memory::cstr_as_bytes(c.as_ptr()) != crate::memory::cstr_as_bytes(efm));
        if efm_changed {
            qf_st().last_efm_cache = None;
            (*cache_ptr).clear();
            qf_st().fmt_start_idx = None;

            *cache_ptr = parse_efm_option(efm);
            if !(*cache_ptr).is_empty() {
                qf_st().last_efm_cache = Some(crate::memory::cstr_to_cstring(efm));
            }
        }

        let mut ok = !(*cache_ptr).is_empty();
        if ok {
            g_st().got_int = false;
            while !g_st().got_int {
                let status = qf_init_process_nextline(
                    qfl,
                    (*cache_ptr).as_slice(),
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
                emsg(gettext(e_readerrf));
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

fn qf_store_title(qfl: &mut qf_list_T, title: *const c_char) {
     {
        qfl.qf_title = if title.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(title))
        };
    }
}

fn qf_cmdtitle(cmd: *const c_char) -> *mut c_char {
    let buf = (&raw mut qf_st().qftitle_str).cast::<c_char>();
        vim_snprintf!(buf, IOSIZE, c":%s", cmd);
        buf
}

#[inline]
fn qf_get_curlist(qi: &mut qf_info_T) -> &'static mut qf_list_T {
    qf_get_list(qi, qi.qf_curlist)
}

fn qf_pop_stack(qi: &mut qf_info_T, adjust: bool) {
    let cnt = qi.qf_listcount as usize;
        let lists = &mut qi.qf_lists.as_mut_slice()[..cnt];
        lists.rotate_left(1);
        lists[cnt - 1] = qf_list_T::default();

        if adjust {
            qi.qf_listcount -= 1;
            if qi.qf_curlist == 0 {
                qi.qf_curlist = qi.qf_listcount - 1;
            } else {
                qi.qf_curlist -= 1;
            }
        }
}

fn qf_new_list(qi: &mut qf_info_T, qf_title: *const c_char) {
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
        *qfl = qf_list_T::default();
        qf_store_title(qfl, qf_title);
        qfl.qfl_type = qi.qfl_type;
        qf_st().last_qf_id += 1;
        qfl.qf_id = qf_st().last_qf_id;
        qfl.qf_has_user_data = false;
}

fn qf_parse_fmt_f(
    rmp: &mut regmatch_T,
    midx: usize,
    fields: &mut qffields_T,
    prefix: c_int,
) -> c_int {
    unsafe {
        if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
            return QF_FAIL;
        }

        let c = *rmp.endp[midx];
        *rmp.endp[midx] = NUL;
        expand_env(
            rmp.startp[midx],
            fields.namebuf.as_mut_ptr(),
            CMDBUFFSIZE as c_int,
        );
        *rmp.endp[midx] = c;

        if !vim_strchr(c"OPQ", prefix).is_null()
            && !os_path_exists(fields.namebuf.as_ptr())
        {
            return QF_FAIL;
        }

        QF_OK
    }
}

fn qf_parse_fmt_b(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
     {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        let bnr = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as std::ffi::c_long as c_int;
        if buflist_findnr(bnr).is_null() {
            return QF_FAIL;
        }
        fields.bnr = bnr;
        QF_OK
    }
}

fn qf_parse_fmt_n(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
     {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        fields.enr = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as std::ffi::c_long as c_int;
        QF_OK
    }
}

fn qf_parse_fmt_l(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
     {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        fields.lnum = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as std::ffi::c_long as linenr_T;
        QF_OK
    }
}

fn qf_parse_fmt_e(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
     {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        fields.end_lnum = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as std::ffi::c_long as linenr_T;
        QF_OK
    }
}

fn qf_parse_fmt_c(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
     {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        fields.col = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as std::ffi::c_long as c_int;
        QF_OK
    }
}

fn qf_parse_fmt_k(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
     {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        fields.end_col = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as std::ffi::c_long as c_int;
        QF_OK
    }
}

fn qf_parse_fmt_t(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
    unsafe {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        fields.type_ = *rmp.startp[midx];
        QF_OK
    }
}

fn copy_nonerror_line(
    linebuf: *const c_char,
    linelen: usize,
    fields: &mut qffields_T,
) -> c_int {
    if linelen >= fields.errmsg.len() {
            fields.errmsg.resize(linelen + 1, 0);
        }
        xstrlcpy(fields.errmsg.as_mut_ptr(), linebuf, linelen + 1);
        QF_OK
}

fn qf_parse_fmt_m(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
    unsafe {
        if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
            return QF_FAIL;
        }
        let len = rmp.endp[midx].offset_from(rmp.startp[midx]) as usize;
        if len >= fields.errmsg.len() {
            fields.errmsg.resize(len + 1, 0);
        }
        xstrlcpy(fields.errmsg.as_mut_ptr(), rmp.startp[midx], len + 1);
        QF_OK
    }
}

fn qf_parse_fmt_r(rmp: &mut regmatch_T, midx: usize, tail: *mut *mut c_char) -> c_int {
    unsafe {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        *tail = rmp.startp[midx];
        QF_OK
    }
}

fn qf_parse_fmt_p(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
    unsafe {
        if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
            return QF_FAIL;
        }
        fields.col = 0;
        let mut match_ptr = rmp.startp[midx];
        while match_ptr != rmp.endp[midx] {
            fields.col += 1;
            if *match_ptr == TAB {
                fields.col += 7;
                fields.col -= fields.col % 8;
            }
            match_ptr = match_ptr.add(1);
        }
        fields.col += 1;
        fields.use_viscol = true;
        QF_OK
    }
}

fn qf_parse_fmt_v(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
     {
        if rmp.startp[midx].is_null() {
            return QF_FAIL;
        }
        fields.col = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(rmp.startp[midx])) as std::ffi::c_long as c_int;
        fields.use_viscol = true;
        QF_OK
    }
}

fn qf_parse_fmt_s(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
    unsafe {
        if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
            return QF_FAIL;
        }
        let mut len = rmp.endp[midx].offset_from(rmp.startp[midx]) as usize;
        len = len.min(CMDBUFFSIZE - 5);
        let pat_ptr = fields.pattern.as_mut_ptr();
        crate::memory::cstr_cpy(pat_ptr, c"^\\V");
        { let _dst: *mut c_char = pat_ptr; let _dsize: usize = len + 4; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), rmp.startp[midx], _dsize - _dlen); };
        *pat_ptr.add(len + 3) = b'\\' as c_char;
        *pat_ptr.add(len + 4) = b'$' as c_char;
        *pat_ptr.add(len + 5) = NUL;
        QF_OK
    }
}

fn qf_parse_fmt_o(rmp: &mut regmatch_T, midx: usize, fields: &mut qffields_T) -> c_int {
    unsafe {
        if rmp.startp[midx].is_null() || rmp.endp[midx].is_null() {
            return QF_FAIL;
        }
        let len = rmp.endp[midx].offset_from(rmp.startp[midx]) as usize;
        let mod_ptr = fields.module.as_mut_ptr();
        let dsize = (crate::memory::cstr_len(mod_ptr) + len + 1).min(CMDBUFFSIZE);
        { let _dst: *mut c_char = mod_ptr; let _dsize: usize = dsize; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), rmp.startp[midx], _dsize - _dlen); };
        QF_OK
    }
}

fn qf_parse_match(
    linebuf: *mut c_char,
    linelen: usize,
    fmt_ptr: &efm_T,
    regmatch: &mut regmatch_T,
    fields: &mut qffields_T,
    qf_multiline: c_int,
    qf_multiscan: c_int,
    tail: *mut *mut c_char,
) -> c_int {
    let idx = fmt_ptr.prefix;

        if (idx == b'C' as c_char || idx == b'Z' as c_char) && qf_multiline == 0 {
            return QF_FAIL;
        }
        if !vim_strchr(c"EWIN", c_int::from(idx as u8)).is_null() {
            fields.type_ = idx;
        } else {
            fields.type_ = 0;
        }

        for i in 0..FMT_PATTERNS {
            let mut status = QF_OK;
            let midx = fmt_ptr.addr[i] as isize;
            if i == 0 && midx > 0 {
                status = qf_parse_fmt_f(regmatch, midx as usize, fields, c_int::from(idx as u8));
            } else if i == FMT_PATTERN_M {
                if fmt_ptr.flags == b'+' as c_char && qf_multiscan == 0 {
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

fn qf_parse_get_fields(
    linebuf: *mut c_char,
    linelen: usize,
    fmt_ptr: &efm_T,
    fields: &mut qffields_T,
    qf_multiline: c_int,
    qf_multiscan: c_int,
    tail: *mut *mut c_char,
) -> c_int {
    unsafe {
        if qf_multiscan != 0
            && vim_strchr(c"OPQ", c_int::from(fmt_ptr.prefix as u8)).is_null()
        {
            return QF_FAIL;
        }

        *fields.namebuf.as_mut_ptr() = NUL;
        fields.bnr = 0;
        *fields.module.as_mut_ptr() = NUL;
        *fields.pattern.as_mut_ptr() = NUL;
        if qf_multiscan == 0 {
            *fields.errmsg.as_mut_ptr() = NUL;
        }
        fields.lnum = 0;
        fields.end_lnum = 0;
        fields.col = 0;
        fields.end_col = 0;
        fields.use_viscol = false;
        fields.enr = -1;
        fields.type_ = 0;
        *tail = ptr::null_mut();

        let mut regmatch: regmatch_T = regmatch_T::new();
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

fn qf_parse_dir_pfx(idx: c_int, fields: &mut qffields_T, qfl: &mut qf_list_T) -> c_int {
    unsafe {
        if idx == c_int::from(b'D') {
            if *fields.namebuf.as_ptr() == NUL {
                emsg(gettext(c"E379: Missing or empty directory name"));
                return QF_FAIL;
            }
            qf_push_dir(
                fields.namebuf.as_mut_ptr(),
                &mut qfl.qf_dir_stack,
                false,
            );
        } else if idx == c_int::from(b'X') {
            qf_pop_dir(&mut qfl.qf_dir_stack);
        }

        QF_OK
    }
}

fn qf_parse_file_pfx(
    idx: c_int,
    fields: &mut qffields_T,
    qfl: &mut qf_list_T,
    tail: *const c_char,
) -> c_int {
    unsafe {
        fields.valid = false;
        if *fields.namebuf.as_ptr() == NUL || os_path_exists(fields.namebuf.as_ptr()) {
            if *fields.namebuf.as_ptr() != NUL && idx == c_int::from(b'P') {
                qf_push_dir(
                    fields.namebuf.as_mut_ptr(),
                    &mut qfl.qf_file_stack,
                    true,
                );
            } else if idx == c_int::from(b'Q') {
                qf_pop_dir(&mut qfl.qf_file_stack);
            }
            *fields.namebuf.as_mut_ptr() = NUL;
            if !tail.is_null() && *tail != NUL {
                qfl.qf_multiscan = true;
                return QF_MULTISCAN;
            }
        }

        QF_OK
    }
}

fn qf_parse_line_nomatch(
    linebuf: *mut c_char,
    linelen: usize,
    fields: &mut qffields_T,
) -> c_int {
    unsafe {
        *fields.namebuf.as_mut_ptr() = NUL;
        fields.lnum = 0;
        fields.valid = false;

        copy_nonerror_line(linebuf, linelen, fields)
    }
}

fn qf_parse_multiline_pfx(
    idx: c_int,
    qfl: &mut qf_list_T,
    fields: &mut qffields_T,
) -> c_int {
    unsafe {
        if !qfl.qf_multiignore {
            let Some(qfprev) = qfl.qf_items.last_mut() else {
                return QF_FAIL;
            };
            let qfprev: *mut qfline_T = qfprev;
            if *fields.errmsg.as_ptr() != NUL {
                let mut bytes = (*qfprev).qf_text.as_bytes().to_vec();
                bytes.push(b'\n');
                bytes.extend_from_slice(crate::memory::cstr_as_bytes(fields.errmsg.as_ptr()));
                (*qfprev).qf_text = std::ffi::CString::new(bytes).unwrap();
            }
            if (*qfprev).qf_nr == -1 {
                (*qfprev).qf_nr = fields.enr;
            }
            if vim_isprintc(c_int::from(fields.type_ as u8)) && (*qfprev).qf_type == 0 {
                (*qfprev).qf_type = fields.type_;
            }

            if (*qfprev).qf_lnum == 0 {
                (*qfprev).qf_lnum = fields.lnum;
            }
            if (*qfprev).qf_end_lnum == 0 {
                (*qfprev).qf_end_lnum = fields.end_lnum;
            }
            if (*qfprev).qf_col == 0 {
                (*qfprev).qf_col = fields.col;
                (*qfprev).qf_viscol = c_char::from(fields.use_viscol);
            }
            if (*qfprev).qf_end_col == 0 {
                (*qfprev).qf_end_col = fields.end_col;
            }
            if (*qfprev).qf_fnum == 0 {
                let fname =
                    if *fields.namebuf.as_ptr() != NUL || !qfl.qf_dir_stack.is_empty() {
                        fields.namebuf.as_mut_ptr()
                    } else if !qfl.qf_file_stack.is_empty() && fields.valid {
                        qfl.currfile_ptr()
                    } else {
                        ptr::null_mut()
                    };
                (*qfprev).qf_fnum = qf_get_fnum(qfl, qfl.directory_ptr(), fname);
            }
        }
        if idx == c_int::from(b'Z') {
            qfl.qf_multiline = false;
            qfl.qf_multiignore = false;
        }
        line_breakcheck();

        QF_IGNORE_LINE
    }
}

fn locstack_queue_delreq(qi: &mut qf_info_T) {
    unsafe {
        let delq = &raw mut qf_st().qf_delq;
        (*delq).push(qi);
    }
}

pub fn qf_stack_get_bufnr() -> c_int {
    unsafe {
        (*ql_info_ptr()).qf_bufnr
    }
}

fn wipe_qf_buffer(qi: &mut qf_info_T) {
    unsafe {
        if qi.qf_bufnr == INVALID_QFBUFNR {
            return;
        }

        let qfbuf = buflist_findnr(qi.qf_bufnr);
        if !qfbuf.is_null() && (*qfbuf).b_nwindows == 0 {
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
}

fn qf_free_list_stack_items(qi: &mut qf_info_T) {
    for i in 0..qi.qf_listcount {
            qf_free(qf_get_list(&mut *qi, i));
        }
}

fn ll_free_all(pqi: *mut *mut qf_info_T) {
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

pub fn qf_free_all(wp: *mut win_T) {
    unsafe {
        let qi = ql_info_ptr();
        if !wp.is_null() {
            ll_free_all(&raw mut (*wp).w_llist);
            ll_free_all(&raw mut (*wp).w_llist_ref);
        } else if !qi.is_null() {
            qf_free_list_stack_items(&mut *qi);
        }
    }
}

fn incr_quickfix_busy() {
    qf_st().quickfix_busy += 1;
}

fn decr_quickfix_busy() {
    unsafe {
        qf_st().quickfix_busy -= 1;
        if qf_st().quickfix_busy == 0 {
            let delq = &raw mut qf_st().qf_delq;
            while let Some(mut qi) = (*delq).pop() {
                ll_free_all(&raw mut qi);
            }
        }
    }
}

pub fn check_quickfix_busy() {
    if qf_st().quickfix_busy != 0 {
            semsg!(
                c"quickfix_busy not zero on exit: %lld",
                c_longlong::from(qf_st().quickfix_busy),
            );
        }
}

fn qf_add_entry(
    qfl: &mut qf_list_T,
    dir: *mut c_char,
    fname: *mut c_char,
    module: *mut c_char,
    bufnum: c_int,
    mesg: *mut c_char,
    lnum: linenr_T,
    end_lnum: linenr_T,
    col: c_int,
    end_col: c_int,
    vis_col: c_char,
    pattern: *mut c_char,
    nr: c_int,
    mut type_: c_char,
    user_data: Option<&typval_T>,
    valid: c_char,
) -> c_int {
    unsafe {
        let buf: *mut buf_T;
        let mut qfp = qfline_T::new();

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
            && !path_equal(full.as_ptr(), (*buf).b_ffname, kPathCmpLiteral)
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
            && user_data.v_type != VarType::VAR_UNKNOWN
        {
            tv_copy(user_data, &mut qfp.qf_user_data);
            qfl.qf_has_user_data = true;
        } else {
            qfp.qf_user_data.v_type = VarType::VAR_UNKNOWN;
        }
        qfp.qf_pattern = if pattern.is_null() || *pattern == NUL {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(pattern))
        };
        qfp.qf_module = if module.is_null() || *module == NUL {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(module))
        };
        qfp.qf_nr = nr;
        if type_ != 1 && !vim_isprintc(c_int::from(type_ as u8)) {
            type_ = 0;
        }
        qfp.qf_type = type_;
        qfp.qf_valid = valid;
        qfp.qf_cleared = 0;

        if qf_list_empty(qfl) {
            qfl.qf_index = 0;
        }
        qfl.qf_items.push(qfp);
        if qfl.qf_index == 0 && valid != 0 {
            qfl.qf_index = qfl.count();
        }

        QF_OK
    }
}

pub fn qf_resize_stack(n: c_int) {
    unsafe {
        qf_resize_stack_base(&mut *ql_info_ptr(), n);
    }
}

pub fn ll_resize_stack(wp: &mut win_T, n: c_int) {
    unsafe {
        if is_ll_window(wp) {
            qf_sync_llw_to_win(wp);
        } else {
            qf_sync_win_to_llw(wp);
        }

        let qi = ll_get_or_alloc_list( wp);
        qf_resize_stack_base(&mut *qi, n);
    }
}

fn qf_resize_stack_base(qi: &mut qf_info_T, n: c_int) {
    if n == qi.qf_maxcount {
            return;
        } else if n < qi.qf_maxcount && n < qi.qf_listcount {
            let amount_to_rm = qi.qf_listcount - n;
            for _ in 0..amount_to_rm {
                qf_pop_stack(qi, true);
            }
        }

        qi.qf_lists.resize_with(n as usize, qf_list_T::default);
        qi.qf_maxcount = n;

        qf_update_buffer(qi, 0);
}

pub fn qf_init_stack() {
    let _ = qf_alloc_stack(QFLT_QUICKFIX, opt_vars().p_chi as c_int);
}

fn qf_sync_llw_to_win(llw: &mut win_T) {
    unsafe {
        let wp = qf_find_win_with_loclist(&*llw.w_llist_ref);
        if !wp.is_null() {
            (*wp).w_onebuf_opt.wo_lhi = llw.w_onebuf_opt.wo_lhi;
        }
    }
}

fn qf_sync_win_to_llw(pwp: &mut win_T) {
     {
        let llw = pwp.w_llist;
        if !llw.is_null() {
            for wp in win_iter_mut(tab_firstwin(curtab_mut())) {
                if wp.w_llist_ref == llw && bt_quickfix(wp.w_buffer) {
                    wp.w_onebuf_opt.wo_lhi = pwp.w_onebuf_opt.wo_lhi;
                    return;
                }
            }
        }
    }
}

fn qf_alloc_stack(qfltype: qfltype_T, n: c_int) -> *mut qf_info_T {
    unsafe {
        let qi = if qfltype == QFLT_QUICKFIX {
            &raw mut qf_st().ql_info_actual
        } else {
            Box::into_raw(Box::new(qf_info_T {
                qf_refcount: 1,
                qf_listcount: 0,
                qf_curlist: 0,
                qf_maxcount: 0,
                qf_lists: Vec::new(),
                qfl_type: 0,
                qf_bufnr: 0,
            }))
        };
        (*qi).qfl_type = qfltype;
        (*qi).qf_bufnr = INVALID_QFBUFNR;
        (*qi).qf_lists.clear();
        (*qi).qf_lists.resize_with(n as usize, qf_list_T::default);
        (*qi).qf_maxcount = n;

        qi
    }
}

fn ll_get_or_alloc_list(wp: &mut win_T) -> *mut qf_info_T {
     {
        if is_ll_window(wp) {
            return wp.w_llist_ref;
        }

        ll_free_all(&raw mut wp.w_llist_ref);

        if wp.w_llist.is_null() {
            wp.w_llist = qf_alloc_stack(QFLT_LOCATION, wp.w_onebuf_opt.wo_lhi as c_int);
        }

        wp.w_llist
    }
}

fn qf_cmd_get_stack(eap: &mut exarg_T, print_emsg: bool) -> *mut qf_info_T {
     {
        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());

        if is_loclist_cmd(eap.cmdidx) {
            qi = get_loc_list(curwin_mut());
            if qi.is_null() {
                if print_emsg {
                    emsg(gettext(e_loclist));
                }
                return ptr::null_mut();
            }
        }

        qi
    }
}

fn qf_cmd_get_or_alloc_stack(eap: &exarg_T, pwinp: *mut *mut win_T) -> *mut qf_info_T {
    unsafe {
        let mut qi = ql_info_ptr();
        if is_loclist_cmd(eap.cmdidx) {
            qi = ll_get_or_alloc_list(curwin_mut());
            *pwinp = curwin_ptr();
        }
        qi
    }
}

fn copy_loclist_entries(from_qfl: &qf_list_T, to_qfl: &mut qf_list_T) -> bool {
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

fn copy_loclist(from_qfl: &qf_list_T, to_qfl: &mut qf_list_T) -> bool {
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
        callback_copy(
            &mut to_qfl.qf_qftf_cb,
            &from_qfl.qf_qftf_cb,
        );

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

pub fn copy_loclist_stack(from: &mut win_T, to: &mut win_T) {
    unsafe {
        let qi = get_loc_list( &*from);
        if qi.is_null() {
            return;
        }

        let to_ll = qf_alloc_stack(QFLT_LOCATION, from.w_onebuf_opt.wo_lhi as c_int);
        to.w_llist = to_ll;
        to.w_onebuf_opt.wo_lhi = i64::from((*to_ll).qf_maxcount);
        (*to_ll).qf_listcount = (*qi).qf_listcount;

        for idx in 0..(*qi).qf_listcount {
            (*to_ll).qf_curlist = idx;
            if !copy_loclist(qf_get_list(&mut *qi, idx), qf_get_list(&mut *to_ll, idx)) {
                qf_free_all(to);
                return;
            }
        }

        (*to_ll).qf_curlist = (*qi).qf_curlist;
    }
}

fn qf_get_fnum(
    qfl: &mut qf_list_T,
    mut directory: *mut c_char,
    fname: *mut c_char,
) -> c_int {
    unsafe {
        let mut ptr_cstr: Option<std::ffi::CString> = None;
        if fname.is_null() || *fname == NUL {
            return 0;
        }

        let fname_str = cstr_as_string(fname);
        let bufname_ptr = if !directory.is_null() && !vim_isAbsName(fname) {
            let mut joined = concat_fnames(cstr_as_string(directory), fname_str, true);
            if !os_path_exists(joined.as_ptr()) {
                directory = qf_guess_filepath(qfl, fname);
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
            .is_some_and(|last| crate::memory::cstr_as_bytes(bufname_ptr) == crate::memory::cstr_as_bytes(last.as_ptr()));
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
    dirbuf: *mut c_char,
    stack: &mut Vec<std::ffi::CString>,
    is_file_stack: bool,
) {
    unsafe {
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
            let slice = std::slice::from_raw_parts(dirbuf.cast::<u8>(), dirbuf_len);
            std::ffi::CString::new(slice).unwrap()
        });
        stack.push(cstr);
    }
}

fn qf_pop_dir(stack: &mut Vec<std::ffi::CString>) {
    stack.pop();
}

fn qf_guess_filepath(qfl: &mut qf_list_T, filename: *mut c_char) -> *mut c_char {
    let stack = &mut qfl.qf_dir_stack;
        if stack.is_empty() {
            return ptr::null_mut();
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

        found_idx.map_or(ptr::null_mut(), |_| {
            stack[stack.len() - 2].as_ptr().cast_mut()
        })
}

fn qflist_valid(wp: *mut win_T, qf_id: c_uint) -> bool {
    unsafe {
        let mut qi = ql_info_ptr();
        if !wp.is_null() {
            if !win_valid(wp) {
                return false;
            }
            qi = get_loc_list( &*wp);
        }
        if qi.is_null() {
            return false;
        }

        for i in 0..(*qi).qf_listcount {
            if (&(*qi).qf_lists)[i as usize].qf_id == qf_id {
                return true;
            }
        }
        false
    }
}

fn is_qf_entry_present(qfl: &qf_list_T, qf_index: c_int) -> bool {
    !g_st().got_int && qf_index >= 1 && (qf_index as usize) <= qfl.qf_items.len()
}

fn get_next_valid_entry(
    qfl: &mut qf_list_T,
    qf_index: &mut c_int,
    dir: c_int,
) -> *mut qfline_T {
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

fn get_prev_valid_entry(
    qfl: &mut qf_list_T,
    qf_index: &mut c_int,
    dir: c_int,
) -> *mut qfline_T {
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
    qfl: &mut qf_list_T,
    mut errornr: c_int,
    dir: c_int,
    new_qfidx: &mut c_int,
) -> *mut qfline_T {
    {
        let mut qf_ptr = qfl.cur_entry();
        let mut qf_idx = qfl.qf_index;
        let mut err: *const c_char = E_NO_MORE_ITEMS.as_ptr();

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
}

fn get_nth_entry(
    qfl: &mut qf_list_T,
    errornr: c_int,
    new_qfidx: &mut c_int,
) -> *mut qfline_T {
    if qfl.qf_items.is_empty() {
            *new_qfidx = qfl.qf_index;
            return ptr::null_mut();
        }
        let qf_idx = errornr.clamp(1, qfl.count());
        *new_qfidx = qf_idx;
        &raw mut (&mut qfl.qf_items)[(qf_idx - 1) as usize]
}

fn qf_get_entry(
    qfl: &mut qf_list_T,
    errornr: c_int,
    dir: c_int,
    new_qfidx: &mut c_int,
) -> *mut qfline_T {
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

fn qf_find_help_win() -> *mut win_T {
    unsafe {
        for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
            if bt_help((*wp).w_buffer) && !(*wp).w_config.hide && (*wp).w_config.focusable {
                return wp;
            }
        }
        ptr::null_mut()
    }
}

fn win_set_loclist(wp: &mut win_T, qi: &mut qf_info_T) {
     {
        wp.w_llist = &raw mut *qi;
        qi.qf_refcount += 1;
    }
}

fn jump_to_help_window(qi: &mut qf_info_T, newwin: bool, opened_window: &mut bool) -> bool {
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
            if g_st().cmdmod.cmod_split == 0 && curwin_mut().w_width != g_st().Columns && curwin_mut().w_width < 80 {
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
                win_setheight(opt_vars().p_hh as c_int);
            }

            if is_ll_stack(qi) && !newwin {
                win_set_loclist(curwin_mut(), qi);
            }
        }

        g_st().restart_edit = 0;

        true
    }
}

fn qf_find_win_with_loclist(ll: &qf_info_T) -> *mut win_T {
    unsafe {
        for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
            if (*wp).w_llist.cast_const() == ll && !bt_quickfix((*wp).w_buffer)
            {
                return wp;
            }
        }
        ptr::null_mut()
    }
}

fn qf_find_win_with_normal_buf() -> *mut win_T {
    unsafe {
        for wp in win_iter_ptr(tab_firstwin(curtab_mut())) {
            if bt_normal((*wp).w_buffer) {
                return wp;
            }
        }
        ptr::null_mut()
    }
}

fn qf_goto_tabwin_with_file(fnum: c_int) -> bool {
    unsafe {
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
}

fn qf_open_new_file_win(ll_ref: *mut qf_info_T) -> bool {
    unsafe {
        let mut flags = WSP_ABOVE;
        if !ll_ref.is_null() {
            flags |= WSP_NEWLOC;
        }
        if !win_split(0, flags) {
            return false;
        }
        opt_vars().p_swb = empty_string_option.as_ptr().cast::<c_char>();
        opt_vars().swb_flags = 0;
        reset_binding(curwin_mut());
        if !ll_ref.is_null() {
            win_set_loclist(curwin_mut(), &mut *ll_ref);
        }
        true
    }
}

fn qf_goto_win_with_ll_file(use_win: *mut win_T, qf_fnum: c_int, ll_ref: *mut qf_info_T) {
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
                    if bt_normal((*win).w_buffer) {
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

        if (*win).w_llist.is_null() && !ll_ref.is_null() {
            win_set_loclist( &mut *win, &mut *ll_ref);
        }
    }
}

fn qf_goto_win_with_qfl_file(qf_fnum: c_int) {
    unsafe {
        let mut win = curwin_ptr();
        let mut altwin: *mut win_T = ptr::null_mut();
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
                if (opt_vars().swb_flags & kOptSwbFlagUselast) != 0
                    && win_valid(g_st().prevwin)
                    && (*g_st().prevwin).w_onebuf_opt.wo_wfb == 0
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
                && bt_normal((*win).w_buffer)
            {
                altwin = win;
            }
        }

        win_goto(win);
    }
}

fn qf_jump_to_usable_window(
    qf_fnum: c_int,
    newwin: bool,
    opened_window: &mut bool,
) -> bool {
    unsafe {
        let mut usable_wp: *mut win_T = ptr::null_mut();
        let mut usable_win = false;

        let ll_ref = if newwin {
            ptr::null_mut()
        } else {
            curwin_mut().w_llist_ref
        };
        if !ll_ref.is_null() {
            usable_wp = qf_find_win_with_loclist(&*ll_ref);
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

        if !usable_win && (opt_vars().swb_flags & kOptSwbFlagUsetab) != 0 {
            usable_win = qf_goto_tabwin_with_file(qf_fnum);
        }

        if ((g_st().firstwin == g_st().lastwin) && bt_quickfix(g_st().curbuf)) || !usable_win || newwin {
            if !qf_open_new_file_win(ll_ref) {
                return false;
            }
            *opened_window = true;
        } else if !curwin_mut().w_llist_ref.is_null() {
            qf_goto_win_with_ll_file(usable_wp, qf_fnum, ll_ref);
        } else {
            qf_goto_win_with_qfl_file(qf_fnum);
        }

        true
    }
}

fn qf_jump_edit_buffer(
    qi: &mut qf_info_T,
    qf_index: c_int,
    forceit: c_int,
    prev_winid: c_int,
    opened_window: &mut bool,
) -> c_int {
    unsafe {
        let qfl = qf_get_curlist(qi);
        let old_changetick = qfl.qf_changedtick;
        let old_qf_curlist = qi.qf_curlist;
        let qfl_type = qfl.qfl_type;
        let mut retval = OK;
        let save_qfid = qfl.qf_id;
        let (qf_type, fnum) = {
            let qfp = &(&qfl.qf_items)[(qf_index - 1) as usize];
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
                if qi.qfl_type == QFLT_LOCATION {
                    emsg(gettext(e_winfixbuf_cannot_go_to_buffer));
                    return FAIL;
                }

                if win_valid(g_st().prevwin)
                    && (*g_st().prevwin).w_onebuf_opt.wo_wfb == 0
                    && !bt_quickfix((*g_st().prevwin).w_buffer)
                {
                    win_goto(g_st().prevwin);
                }
                if curwin_mut().w_onebuf_opt.wo_wfb != 0 {
                    if win_split(0, 0) {
                        *opened_window = true;
                    }
                    if curwin_mut().w_onebuf_opt.wo_wfb != 0 {
                        emsg(gettext(e_winfixbuf_cannot_go_to_buffer));
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

        if old_qf_curlist != qi.qf_curlist
            || old_changetick != qfl.qf_changedtick
            || !is_qf_entry_present(qfl, qf_index)
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
}

fn qf_screen_col_to_idx(vcol: colnr_T) -> c_int {
    unsafe {
        let line = get_cursor_line_ptr().cast_const();
        let mut p = line;
        let mut col: colnr_T = 0;

        while *p != NUL && col < vcol {
            if *p == TAB {
                col += 8 - (col % 8);
            } else {
                col += ptr2cells(p);
            }
            if col > vcol {
                break;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        p.offset_from(line) as c_int
    }
}

fn qf_jump_goto_line(
    qf_lnum: linenr_T,
    qf_col: c_int,
    qf_viscol: c_char,
    qf_pattern: *mut c_char,
) {
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
                ptr::null_mut(),
                b'/' as c_int,
                b'/' as c_int,
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
    qi: &mut qf_info_T,
    qf_index: c_int,
    qf_ptr: &mut qfline_T,
    old_curbuf: *mut buf_T,
    old_lnum: linenr_T,
) {
     {
        let gap = qfga_get();

        if g_st().msg_scrolled == 0 {
            update_topline(curwin_mut());
            if g_st().must_redraw != 0 {
                update_screen();
            }
        }
        let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
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
            qf_types(c_int::from(qf_ptr.qf_type), qf_ptr.qf_nr),
        );
        qfga_concat_len(gap, iobuff_ptr, iobufflen);
        qf_fmt_text(gap, skipwhite(qf_ptr.qf_text.as_ptr()));
        gap.push(NUL);

        let i = g_st().msg_scroll;
        if g_st().curbuf == old_curbuf && curwin_mut().w_cursor.lnum == old_lnum {
            g_st().msg_scroll = 1;
        } else if (g_st().msg_scrolled == 0 || (opt_vars().p_ch == 0 && g_st().msg_scrolled == 1)) && shortmess(kShmOverall)
        {
            g_st().msg_scroll = 0;
        }
        msg_ext_set_kind(c"quickfix");
        msg_keep(gap.as_ptr(), 0, true, false);
        g_st().msg_scroll = i;

        qfga_clear();
    }
}

fn qf_jump_open_window(
    qi: &mut qf_info_T,
    qf_index: c_int,
    newwin: bool,
    opened_window: &mut bool,
) -> c_int {
     {
        let qfl = qf_get_curlist(qi);
        let old_changetick = qfl.qf_changedtick;
        let old_qf_curlist = qi.qf_curlist;
        let qfl_type = qfl.qfl_type;
        let qf_type = (&qfl.qf_items)[(qf_index - 1) as usize].qf_type;

        if qf_type == 1
            && (!bt_help(curwin_mut().w_buffer) || g_st().cmdmod.cmod_tab != 0)
            && !jump_to_help_window(qi, newwin, opened_window)
        {
            return FAIL;
        }
        if old_qf_curlist != qi.qf_curlist
            || old_changetick != qfl.qf_changedtick
            || !is_qf_entry_present(qfl, qf_index)
        {
            if qfl_type == QFLT_QUICKFIX {
                emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED));
            } else {
                emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED));
            }
            return QF_ABORT;
        }

        let qf_fnum = (&qfl.qf_items)[(qf_index - 1) as usize].qf_fnum;
        if bt_quickfix(g_st().curbuf) && !*opened_window {
            if qf_fnum == 0 {
                return NOTDONE;
            }

            if !qf_jump_to_usable_window(qf_fnum, newwin, opened_window) {
                return FAIL;
            }
        }
        if old_qf_curlist != qi.qf_curlist
            || old_changetick != qfl.qf_changedtick
            || !is_qf_entry_present(qfl, qf_index)
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
}

fn qf_jump_to_buffer(
    qi: &mut qf_info_T,
    qf_index: c_int,
    forceit: c_int,
    prev_winid: c_int,
    opened_window: &mut bool,
    openfold: c_int,
    print_message: bool,
) -> c_int {
    unsafe {
        let old_curbuf = g_st().curbuf;
        let old_lnum = curwin_mut().w_cursor.lnum;
        let mut retval = OK;
        let qfl = qf_get_curlist(qi);

        if (&qfl.qf_items)[(qf_index - 1) as usize].qf_fnum != 0 {
            retval = qf_jump_edit_buffer(qi, qf_index, forceit, prev_winid, opened_window);
            if retval != OK {
                return retval;
            }
        }

        if g_st().curbuf == old_curbuf {
            setpcmark();
        }

        let qf_ptr = &raw mut (&mut qfl.qf_items)[(qf_index - 1) as usize];
        qf_jump_goto_line(
            (*qf_ptr).qf_lnum,
            (*qf_ptr).qf_col,
            (*qf_ptr).qf_viscol,
            (*qf_ptr)
                .qf_pattern
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
        );

        if (opt_vars().fdo_flags & kOptFdoFlagQuickfix) != 0 && openfold != 0 {
            foldOpenCursor();
        }
        if print_message {
            qf_jump_print_msg(qi, qf_index, &mut *qf_ptr, old_curbuf, old_lnum);
        }

        retval
    }
}

pub fn qf_jump(qi: *mut qf_info_T, dir: c_int, errornr: c_int, forceit: c_int) {
    qf_jump_newwin(qi, dir, errornr, forceit, false);
}

fn qf_jump_newwin(
    mut qi: *mut qf_info_T,
    dir: c_int,
    errornr: c_int,
    forceit: c_int,
    newwin: bool,
) {
    unsafe {
        let old_swb = opt_vars().p_swb;
        let old_swb_flags = opt_vars().swb_flags;
        let old_KeyTyped = g_st().KeyTyped;

        if qi.is_null() {
                qi = ql_info_ptr();
        }

        if qf_stack_empty(qi) || qf_list_empty(qf_get_curlist(&mut *qi)) {
            emsg(gettext(e_no_errors));
            return;
        }

        incr_quickfix_busy();

        let qfl = qf_get_curlist(&mut *qi);

        let mut qf_index = qfl.qf_index;
        let old_qf_index = qf_index;

        let qf_ptr = qf_get_entry(qfl, errornr, dir, &mut qf_index);
        if !qf_ptr.is_null() {
            qfl.qf_index = qf_index;

            let print_message = !qf_win_pos_update(&mut *qi, old_qf_index);
            let prev_winid = curwin_mut().handle;
            let mut opened_window = false;
            let mut retval = qf_jump_open_window(&mut *qi, qf_index, newwin, &mut opened_window);
            if retval == FAIL {
                qf_index = old_qf_index;
            } else if retval == QF_ABORT {
                qi = ptr::null_mut();
            } else if retval != NOTDONE {
                retval = qf_jump_to_buffer(
                    &mut *qi,
                    qf_index,
                    forceit,
                    prev_winid,
                    &mut opened_window,
                    c_int::from(old_KeyTyped),
                    print_message,
                );
                if retval == QF_ABORT {
                    qi = ptr::null_mut();
                } else if retval != OK {
                    if opened_window {
                        win_close(curwin_ptr(), true, false);
                    }
                    if (&qfl.qf_items)[(qf_index - 1) as usize].qf_fnum != 0 {
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
        if opt_vars().p_swb != old_swb && opt_vars().p_swb == empty_string_option.as_ptr().cast::<c_char>() {
            opt_vars().p_swb = old_swb;
            opt_vars().swb_flags = old_swb_flags;
        }
        decr_quickfix_busy();
    }
}

fn qf_list_entry(qfp: &mut qfline_T, qf_idx: c_int, cursel: bool) {
    unsafe {
        let mut fname: *mut c_char = ptr::null_mut();
        let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        if let Some(ref module) = qfp.qf_module
            && !module.as_bytes().is_empty()
        {
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                c"%2d %s",
                qf_idx,
                module.as_ptr(),
            );
        } else {
            if qfp.qf_fnum != 0 {
                let buf = buflist_findnr(qfp.qf_fnum);
                if !buf.is_null() {
                    fname = qfp
                        .qf_fname
                        .as_ref()
                        .map_or((*buf).b_fname, |s| s.as_ptr().cast_mut());
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
            msg_putchar(b'\n' as c_int);
        }
        msg_outtrans(
            iobuff_ptr,
            if cursel { HLF_QFL } else { qf_st().qfFile_hl_id },
            false,
        );

        if qfp.qf_lnum != 0 {
            msg_puts_hl(c":", qf_st().qfSep_hl_id, false);
        }
        let mut gap = qfga_get();
        if qfp.qf_lnum != 0 {
            qf_range_text(gap, qfp);
        }
        qfga_concat(gap, qf_types(c_int::from(qfp.qf_type), qfp.qf_nr));
        gap.push(NUL);
        if *gap.as_ptr() != NUL {
            msg_puts_hl(gap.as_ptr(), qf_st().qfLine_hl_id, false);
        }
        msg_puts_hl(c":", qf_st().qfSep_hl_id, false);
        if let Some(ref pat) = qfp.qf_pattern {
            gap = qfga_get();
            qf_fmt_text(gap, pat.as_ptr());
            gap.push(NUL);
            msg_puts(gap.as_ptr());
            msg_puts_hl(c":", qf_st().qfSep_hl_id, false);
        }
        msg_puts(c" ");

        gap = qfga_get();
        let text = if !fname.is_null() || qfp.qf_lnum != 0 {
            skipwhite(qfp.qf_text.as_ptr())
        } else {
            qfp.qf_text.as_ptr()
        };
        qf_fmt_text(gap, text);
        gap.push(NUL);
        msg_prt_line(gap.as_mut_ptr(), false);
    }
}

pub fn qf_list(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;
        let mut all = eap.forceit != 0;
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        if qf_stack_empty(qi) || qf_list_empty(qf_get_curlist(&mut *qi)) {
            emsg(gettext(e_no_errors));
            return;
        }

        let mut plus = false;
        if *arg == b'+' as c_char {
            arg = arg.add(1);
            plus = true;
        }
        let mut idx1: c_int = 1;
        let mut idx2: c_int = -1;
        if !get_list_range(&raw mut arg, &mut idx1, &mut idx2) || *arg != NUL {
            semsg!(gettext(e_trailing_arg), arg);
            return;
        }
        let qfl = qf_get_curlist(&mut *qi);
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

        qf_st().qfFile_hl_id = syn_name2id(c"qfFileName");
        if qf_st().qfFile_hl_id == 0 {
            qf_st().qfFile_hl_id = HLF_D;
        }
        qf_st().qfSep_hl_id = syn_name2id(c"qfSeparator");
        if qf_st().qfSep_hl_id == 0 {
            qf_st().qfSep_hl_id = HLF_D;
        }
        qf_st().qfLine_hl_id = syn_name2id(c"qfLineNr");
        if qf_st().qfLine_hl_id == 0 {
            qf_st().qfLine_hl_id = HLF_N;
        }

        if qfl.qf_nonevalid {
            all = true;
        }
        msg_ext_set_kind(c"list_cmd");
        for (idx, qfp) in qfl.qf_items.iter_mut().enumerate() {
            if g_st().got_int {
                break;
            }
            let i = (idx + 1) as c_int;
            if (qfp.qf_valid != 0 || all) && idx1 <= i && i <= idx2 {
                qf_list_entry(qfp, i, i == qfl.qf_index);
            }
            os_breakcheck();
        }
        qfga_clear();
    }
}

fn qf_fmt_text(gap: &mut Vec<c_char>, text: *const c_char) {
    unsafe {
        let mut p = text;
        while *p != NUL {
            if *p == b'\n' as c_char {
                gap.push(b' ' as c_char);
                loop {
                    p = p.add(1);
                    if *p == NUL {
                        break;
                    }
                    if !ascii_iswhite(c_int::from(*p as u8)) && *p != b'\n' as c_char {
                        break;
                    }
                }
            } else {
                gap.push(*p);
                p = p.add(1);
            }
        }
    }
}

fn qf_range_text(gap: &mut Vec<c_char>, qfp: &qfline_T) {
    unsafe {
        let buf_data = (&raw mut g_st().IObuff).cast::<c_char>();
        let mut buf_size = vim_snprintf_safelen!(buf_data, IOSIZE, c"%d", qfp.qf_lnum);

        if qfp.qf_end_lnum > 0 && qfp.qf_lnum != qfp.qf_end_lnum {
            buf_size += vim_snprintf_safelen!(
                buf_data.add(buf_size),
                IOSIZE - buf_size,
                c"-%d",
                qfp.qf_end_lnum,
            );
        }
        if qfp.qf_col > 0 {
            buf_size += vim_snprintf_safelen!(
                buf_data.add(buf_size),
                IOSIZE - buf_size,
                c" col %d",
                qfp.qf_col,
            );
            if qfp.qf_end_col > 0 && qfp.qf_col != qfp.qf_end_col {
                buf_size += vim_snprintf_safelen!(
                    buf_data.add(buf_size),
                    IOSIZE - buf_size,
                    c"-%d",
                    qfp.qf_end_col,
                );
            }
        }

        qfga_concat_len(gap, buf_data, buf_size);
    }
}

fn qf_msg(qi: &mut qf_info_T, which: c_int, lead: impl crate::strings::IntoCCharPtr) {
    let lead = lead.into_c_char_ptr();
    unsafe {
        let qfl = &(&qi.qf_lists)[which as usize];
        let title = qfl.qf_title.as_ref();
        let count = qfl.count();
        let mut buf = [0 as c_char; IOSIZE];

        vim_snprintf!(
            buf.as_mut_ptr(),
            IOSIZE,
            gettext(c"%serror list %d of %d; %d errors "),
            lead,
            which + 1,
            qi.qf_listcount,
            count,
        );

        if let Some(title) = title {
            let len = crate::memory::cstr_len(buf.as_ptr());
            if len < 34 {
                ptr::write_bytes(buf.as_mut_ptr().add(len), b' ', 34 - len);
                buf[34] = NUL;
            }
            { let _dst: *mut c_char = buf.as_mut_ptr(); let _dsize: usize = IOSIZE; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), title.as_ptr(), _dsize - _dlen); };
        }
        trunc_string(buf.as_ptr(), buf.as_mut_ptr(), g_st().Columns - 1, IOSIZE as c_int);
        msg(buf.as_ptr(), 0);
    }
}

pub fn qf_age(eap: &mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let mut count = if eap.addr_count != 0 {
            eap.line2 as c_int
        } else {
            1
        };
        while count > 0 {
            count -= 1;
            if eap.cmdidx == CMD_colder || eap.cmdidx == CMD_lolder {
                if (*qi).qf_curlist == 0 {
                    emsg(gettext(c"E380: At bottom of quickfix stack"));
                    break;
                }
                (*qi).qf_curlist -= 1;
            } else {
                if (*qi).qf_curlist >= (*qi).qf_listcount - 1 {
                    emsg(gettext(c"E381: At top of quickfix stack"));
                    break;
                }
                (*qi).qf_curlist += 1;
            }
        }
        qf_msg(&mut *qi, (*qi).qf_curlist, c"");
        qf_update_buffer(&mut *qi, 0);
    }
}

pub fn qf_history(eap: &mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);

        if eap.addr_count > 0 {
            if qi.is_null() {
                emsg(gettext(e_loclist));
                return;
            }

            if eap.line2 > 0 && eap.line2 <= (*qi).qf_listcount {
                (*qi).qf_curlist = eap.line2 - 1;
                qf_msg(&mut *qi, (*qi).qf_curlist, c"");
                qf_update_buffer(&mut *qi, 0);
            } else {
                emsg(gettext(e_invrange));
            }
            return;
        }

        if qf_stack_empty(qi) {
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

fn qf_free_items(qfl: &mut qf_list_T) {
    qfl.qf_items.clear();
        qfl.qf_index = 0;
        qfl.qf_nonevalid = true;

        qfl.qf_dir_stack.clear();
        qfl.qf_file_stack.clear();
        qfl.qf_multiline = false;
        qfl.qf_multiignore = false;
        qfl.qf_multiscan = false;
}

fn qf_free(qfl: &mut qf_list_T) {
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
    buf: &mut buf_T,
    wp: *mut win_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) -> bool {
    unsafe {
        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());
        let buf_has_flag = if wp.is_null() {
            BUF_HAS_QF_ENTRY
        } else {
            BUF_HAS_LL_ENTRY
        };

        if (buf.b_has_qf_entry & buf_has_flag) == 0 {
            return false;
        }
        if !wp.is_null() {
            if (*wp).w_llist.is_null() {
                return false;
            }
            qi = (*wp).w_llist;
        }

        let mut found_one = false;
        for idx in 0..(*qi).qf_listcount {
            let qfl = qf_get_list(&mut *qi, idx);
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
}

fn qf_types(c: c_int, nr: c_int) -> *const c_char {
    unsafe {
        let p: *const c_char = if c == b'W' as c_int || c == b'w' as c_int {
            c" warning".as_ptr()
        } else if c == b'I' as c_int || c == b'i' as c_int {
            c" info".as_ptr()
        } else if c == b'N' as c_int || c == b'n' as c_int {
            c" note".as_ptr()
        } else if c == b'E' as c_int || c == b'e' as c_int || (c == 0 && nr > 0) {
            c" error".as_ptr()
        } else if c == 0 || c == 1 {
            c"".as_ptr()
        } else {
            let cc = (&raw mut qf_st().qf_types_cc).cast::<c_char>();
            *cc = b' ' as c_char;
            *cc.add(1) = c as c_char;
            *cc.add(2) = NUL;
            cc
        };

        if nr <= 0 {
            return p;
        }

        let buf = (&raw mut qf_st().qf_types_buf).cast::<c_char>();
        vim_snprintf!(buf, 20, c"%s %3d", p, nr);
        buf
    }
}

pub fn qf_view_result(split: bool) {
    unsafe {
        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());

        if is_ll_window(curwin_mut()) {
            qi = get_loc_list(curwin_mut());
        }

        if qf_list_empty(qf_get_curlist(&mut *qi)) {
            emsg(gettext(e_no_errors));
            return;
        }

        if split {
            qf_jump_newwin(qi, 0, curwin_mut().w_cursor.lnum as c_int, 0, true);
            do_cmdline_cmd(c"clearjumps");
            return;
        }

        do_cmdline_cmd(if is_ll_window(curwin_mut()) {
            c".ll".as_ptr()
        } else {
            c".cc".as_ptr()
        });
    }
}

pub fn ex_cwindow(eap: &mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let qfl = qf_get_curlist(&mut *qi);
        let win = qf_find_win(&*qi);

        if qf_stack_empty(qi) || qfl.qf_nonevalid || qf_list_empty(qfl) {
            if !win.is_null() {
                ex_cclose(eap);
            }
        } else if win.is_null() {
            ex_copen(eap);
        }
    }
}

pub fn ex_cclose(eap: &mut exarg_T) {
    let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return;
        }

        let win = unsafe { qf_find_win(&*qi) };
        if !win.is_null() {
            win_close(win, false, false);
        }
}

fn qf_goto_cwindow(qi: &qf_info_T, resize: bool, sz: c_int, vertsplit: bool) -> bool {
    unsafe {
        let win = qf_find_win(qi);
        if win.is_null() {
            return false;
        }

        win_goto(win);
        if resize {
            if vertsplit {
                if sz != (*win).w_width {
                    win_setwidth(sz);
                }
            } else if sz != (*win).w_height
                && ((*win).w_height
                    + (*win).w_hsep_height
                    + (*win).w_status_height
                    + tabline_height()
                    < g_st().cmdline_row)
            {
                win_setheight(sz);
            }
        }

        true
    }
}

fn qf_set_cwindow_options() {
     {
        set_option_value_give_err(kOptSwapfile, boolean_obj(false), OPT_LOCAL);
        set_option_value_give_err(kOptBuftype, cstr_as_obj(c"quickfix"), OPT_LOCAL);
        set_option_value_give_err(kOptBufhidden, cstr_as_obj(c"hide"), OPT_LOCAL);
        reset_binding(curwin_mut());
        curwin_mut().w_onebuf_opt.wo_diff = 0;
        set_option_value_give_err(kOptFoldmethod, cstr_as_obj(c"manual"), OPT_LOCAL);
    }
}

fn qf_open_new_cwindow(qi: &mut qf_info_T, height: c_int) -> bool {
    unsafe {
        let mut oldwin = curwin_ptr();
        let prevtab = g_st().curtab;
        let mut flags: c_int = 0;

        let qf_buf = qf_find_buf(qi);
        let win = curwin_ptr();

        if g_st().cmdmod.cmod_split == 0 {
            flags = if is_qf_stack(&*qi) { WSP_BOT } else { WSP_BELOW };
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
                (*qf_buf).b_fnum,
                ptr::null_mut(),
                ptr::null_mut(),
                None,
                ECMD_ONE,
                ECMD_HIDE + ECMD_OLDBUF + ECMD_NOWINENTER,
                oldwin,
            )
            {
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
            )
            {
                return false;
            }
            qi.qf_bufnr = curbuf_mut().b_fnum;
        }

        if !bt_quickfix(g_st().curbuf) {
            qf_set_cwindow_options();
        }

        if g_st().curtab == prevtab && curwin_mut().w_width == g_st().Columns {
            win_setheight(height);
        }
        curwin_mut().w_onebuf_opt.wo_wfh = 1;
        if win_valid(win) {
            g_st().prevwin = win;
        }
        true
    }
}

fn qf_set_title_var(qfl: &mut qf_list_T) {
    if let Some(ref title) = qfl.qf_title {
            set_internal_string_var(c"w:quickfix_title", title.as_ptr().cast_mut());
        }
}

pub fn ex_copen(eap: &mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        incr_quickfix_busy();

        let height = if eap.addr_count != 0 {
            eap.line2 as c_int
        } else {
            QF_WINHEIGHT
        };
        reset_VIsual_and_resel();

        let mut status = false;
        if g_st().cmdmod.cmod_tab == 0 {
            status = qf_goto_cwindow(
                &*qi,
                eap.addr_count != 0,
                height,
                (g_st().cmdmod.cmod_split & WSP_VERT) != 0,
            );
        }
        if !status && !qf_open_new_cwindow(&mut *qi, height) {
            decr_quickfix_busy();
            return;
        }

        let qfl = qf_get_curlist(&mut *qi);
        qf_set_title_var(qfl);
        let lnum = qfl.qf_index;

        qf_fill_buffer(qfl, curbuf_mut(), 0, curwin_mut().handle);

        decr_quickfix_busy();

        curwin_mut().w_cursor.lnum = lnum;
        curwin_mut().w_cursor.col = 0;
        check_cursor(curwin_mut());
        update_topline(curwin_mut());
    }
}

fn qf_win_goto(win: *mut win_T, lnum: linenr_T) {
    unsafe {
        let old_curwin = curwin_ptr();

        set_curwin(win);
        g_st().curbuf = (*win).w_buffer;
        curwin_mut().w_cursor.lnum = lnum;
        curwin_mut().w_cursor.col = 0;
        curwin_mut().w_cursor.coladd = 0;
        curwin_mut().w_curswant = 0;
        update_topline(curwin_mut());
        redraw_later(curwin_ptr(), UPD_VALID);
        curwin_mut().w_redr_status = true;
        set_curwin(old_curwin);
        g_st().curbuf = curwin_mut().w_buffer;
    }
}

pub fn ex_cbottom(eap: &mut exarg_T) {
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

pub fn qf_current_entry(wp: &mut win_T) -> linenr_T {
    unsafe {
        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());

        if is_ll_window(&*wp) {
            qi = wp.w_llist_ref;
        }

        qf_get_curlist(&mut *qi).qf_index
    }
}

fn qf_win_pos_update(qi: &mut qf_info_T, old_qf_index: c_int) -> bool {
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

fn is_qf_win(win: &win_T, qi: &qf_info_T) -> bool {
    if buf_valid(win.w_buffer)
        && bt_quickfix(win.w_buffer)
        && ((is_qf_stack(qi) && win.w_llist_ref.is_null())
            || (is_ll_stack(qi) && ptr::eq(win.w_llist_ref.cast_const(), qi)))
    {
        return true;
    }
    false
}

fn qf_find_win(qi: &qf_info_T) -> *mut win_T {
    unsafe {
        for win in win_iter_ptr(tab_firstwin(curtab_mut())) {
            if is_qf_win(&*win, qi) {
                return win;
            }
        }
        ptr::null_mut()
    }
}

fn qf_find_buf(qi: &mut qf_info_T) -> *mut buf_T {
     {
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
}

fn qf_update_win_titlevar(qi: &mut qf_info_T) {
    unsafe {
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
}

fn qf_update_buffer(qi: &mut qf_info_T, old_count: usize) {
    unsafe {
        let buf = qf_find_buf(qi);
        if buf.is_null() {
            return;
        }

        let old_line_count = (*buf).b_ml.ml_line_count;
        let old_endcol = ml_get_buf_len(buf, old_line_count);
        let old_bytecount = get_region_bytecount(&mut *buf, 1, old_line_count, 0, old_endcol);
        let mut qf_winid: c_int = 0;

        let mut win: *mut win_T;
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
            qf_winid = (*win).handle;
        }

        incr_quickfix_busy();

        let mut aco: CtxSwitch = CtxSwitch::new();
        if old_count == 0 {
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
        }

        qf_update_win_titlevar(qi);

        qf_fill_buffer(qf_get_curlist(&mut *qi), &mut *buf, old_count, qf_winid);

        let new_line_count = (*buf).b_ml.ml_line_count;
        let new_endcol = ml_get_buf_len(buf, new_line_count);
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
                kExtmarkNoUndo,
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
                kExtmarkNoUndo,
            );
            changed_lines( &mut *buf, start_lnum, 0, start_lnum, delta, true);
        }
        (*buf).b_changed = 0;

        if old_count == 0 {
            qf_win_pos_update(&mut *qi, 0);
            ctx_restore(&mut aco);
        }

        win = qf_find_win(&*qi);
        if !win.is_null() && old_line_count < (*win).w_botline {
            redraw_buf_later(buf, UPD_NOT_VALID);
        }

        decr_quickfix_busy();
    }
}

fn qf_buf_add_line(
    _qfl: &qf_list_T,buf: &mut buf_T,
    lnum: linenr_T,
    qfp: &qfline_T,
    dirname: *mut c_char,
    qftf_str: *mut c_char,
    first_bufline: bool,
) -> bool {
    unsafe {
        let gap = qfga_get();

        if !qftf_str.is_null() && *qftf_str != NUL {
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
                            if *dirname == NUL {
                                os_dirname(dirname, MAXPATHL);
                            }
                            shorten_buf_fname(&mut *errbuf, dirname, 0);
                        }
                        qfga_concat(
                            gap,
                            qfp
                                .qf_fname
                                .as_ref()
                                .map_or((*errbuf).b_fname.cast_const(), |s| s.as_ptr()),
                        );
                    }
                }
            }

            gap.push(b'|' as c_char);

            if qfp.qf_lnum > 0 {
                qf_range_text(gap, qfp);
                qfga_concat(gap, qf_types(c_int::from(qfp.qf_type), qfp.qf_nr));
            } else if let Some(ref pat) = qfp.qf_pattern {
                qf_fmt_text(gap, pat.as_ptr());
            }
            gap.push(b'|' as c_char);
            gap.push(b' ' as c_char);

            let text_arg = if gap.len() > 3 {
                skipwhite(qfp.qf_text.as_ptr())
            } else {
                qfp.qf_text.as_ptr()
            };
            qf_fmt_text(gap, text_arg);
        }

        gap.push(NUL);
        if !ml_append_buf( &mut *buf, lnum, gap.as_mut_ptr(), gap.len() as colnr_T, false) {
            return false;
        }

        true
    }
}

fn call_qftf_func(
    qfl: &mut qf_list_T,
    qf_winid: c_int,
    start_idx: c_int,
    end_idx: c_int,
) -> *mut list_T {
    unsafe {
        let mut cb: *mut Callback = &raw mut opt_vars().p_qftf;
        let mut qftf_list: *mut list_T = ptr::null_mut();

        if qf_st().qftf_recursive {
            return ptr::null_mut();
        }
        qf_st().qftf_recursive = true;

        if qfl.qf_qftf_cb.cb_type() != kCallbackNone {
            cb = &raw mut qfl.qf_qftf_cb;
        }
        if (*cb).cb_type() != kCallbackNone {
            let mut args: [typval_T; 1] = [typval_T::new(); 1];
            let mut rettv: typval_T = typval_T::new();

            let dict = tv_dict_alloc_lock(VAR_FIXED);
            tv_dict_add_nr(
                &mut *dict,
                c"quickfix",
                varnumber_T::from(is_qf_list(qfl)),
            );
            tv_dict_add_nr(&mut *dict, c"winid", varnumber_T::from(qf_winid));
            tv_dict_add_nr(&mut *dict, c"id", varnumber_T::from(qfl.qf_id));
            tv_dict_add_nr(&mut *dict, c"start_idx", varnumber_T::from(start_idx));
            tv_dict_add_nr(&mut *dict, c"end_idx", varnumber_T::from(end_idx));
            (*dict).dv_refcount += 1;
            args[0].v_type = VarType::VAR_DICT;
            args[0].vval = crate::eval::typval::TypvalVval::Dict(dict);

            g_st().textlock += 1;
            if callback_call( &mut *cb, 1, args.as_mut_ptr(), &mut rettv) {
                if rettv.v_type == VarType::VAR_LIST {
                    qftf_list = rettv.vval.v_list();
                    tv_list_ref(qftf_list);
                }
                tv_clear(&raw mut rettv);
            }
            g_st().textlock -= 1;
            tv_dict_unref(dict);
        }

        qf_st().qftf_recursive = false;
        qftf_list
    }
}

fn qf_fill_buffer(qfl: *mut qf_list_T, buf: &mut buf_T, old_count: usize, qf_winid: c_int) {
    unsafe {
        let old_KeyTyped = g_st().KeyTyped;

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

        if !qfl.is_null() && !(*qfl).qf_items.is_empty() {
            let mut dirname = [0 as c_char; MAXPATHL];
            let mut lnum: linenr_T = if old_count == 0 {
                0
            } else {
                buf.b_ml.ml_line_count
            };

            let qftf_list = call_qftf_func(&mut *qfl, qf_winid, lnum + 1, (*qfl).count());
            let mut qftf_li = tv_list_first(qftf_list);

            let mut prev_bufnr: c_int = -1;
            let mut invalid_val = false;

            while (lnum as usize) < (*qfl).qf_items.len() {
                let mut qftf_str: *mut c_char = ptr::null_mut();

                if !qftf_li.is_null() && !invalid_val {
                    qftf_str = tv_get_string_chk(&(*qftf_li).li_tv).cast_mut();
                    if qftf_str.is_null() {
                        invalid_val = true;
                    }
                }

                let qfp = &raw mut (&mut (*qfl).qf_items)[lnum as usize];
                let qfp_fnum = (*qfp).qf_fnum;
                if !qf_buf_add_line(
                    &*qfl,
                    &mut *buf,
                    lnum,
                    &*qfp,
                    dirname.as_mut_ptr(),
                    qftf_str,
                    prev_bufnr != qfp_fnum,
                )
                {
                    break;
                }
                prev_bufnr = qfp_fnum;
                lnum += 1;

                if !qftf_li.is_null() {
                    qftf_li = (*qftf_li).li_next;
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
            set_option_value_give_err(kOptFiletype, cstr_as_obj(c"qf"), OPT_LOCAL);
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

        g_st().KeyTyped = old_KeyTyped;
    }
}

#[inline]
fn qf_list_changed(qfl: &mut qf_list_T) {
    qfl.qf_changedtick += 1;
}

fn qf_id2nr(qi: &qf_info_T, qfid: c_uint) -> c_int {
    for qf_idx in 0..qi.qf_listcount {
            if (&qi.qf_lists)[qf_idx as usize].qf_id == qfid {
                return qf_idx;
            }
        }
        INVALID_QFIDX
}

fn qf_restore_list(qi: &mut qf_info_T, save_qfid: c_uint) -> bool {
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

fn qf_jump_first(qi: &mut qf_info_T, save_qfid: c_uint, forceit: c_int) {
    if !qf_restore_list(qi, save_qfid) {
            return;
        }

        if !check_can_set_curbuf_forceit(forceit) {
            return;
        }

        if !qf_list_empty(qf_get_curlist(&mut *qi)) {
            qf_jump(&raw mut *qi, 0, 0, forceit);
        }
}

pub fn grep_internal(cmdidx: c_int) -> c_int {
     {
        let gp = if crate::memory::cstr_is_empty(curbuf_mut().b_p_gp) {
            opt_vars().p_gp
        } else {
            curbuf_mut().b_p_gp
        };
        c_int::from(
            (cmdidx == CMD_grep
                || cmdidx == CMD_lgrep
                || cmdidx == CMD_grepadd
                || cmdidx == CMD_lgrepadd)
                && b"internal" == crate::memory::cstr_as_bytes(gp),
        )
    }
}

fn make_get_auname(cmdidx: c_int) -> *const c_char {
    match cmdidx {
        CMD_make => c"make".as_ptr(),
        CMD_lmake => c"lmake".as_ptr(),
        CMD_grep => c"grep".as_ptr(),
        CMD_lgrep => c"lgrep".as_ptr(),
        CMD_grepadd => c"grepadd".as_ptr(),
        CMD_lgrepadd => c"lgrepadd".as_ptr(),
        _ => ptr::null(),
    }
}

fn make_get_fullcmd(makecmd: *const c_char, fname: *const c_char) -> std::ffi::CString {
     {
        let mut len = crate::memory::cstr_len(opt_vars().p_shq) * 2 + crate::memory::cstr_len(makecmd) + 1;
        if !crate::memory::cstr_is_empty(opt_vars().p_sp) {
            len += crate::memory::cstr_len(opt_vars().p_sp) + crate::memory::cstr_len(fname) + 3;
        }
        let mut cmd_vec: Vec<c_char> = vec![0; len];
        let cmd = cmd_vec.as_mut_ptr();
        vim_snprintf!(cmd, len, c"%s%s%s", opt_vars().p_shq, makecmd, opt_vars().p_shq);

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

pub fn ex_make(eap: &mut exarg_T) {
    unsafe {
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
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            )
            && aborting()
        {
            return;
        }

        let mut wp: *mut win_T = ptr::null_mut();
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

        let errorformat = if eap.cmdidx != CMD_make && eap.cmdidx != CMD_lmake {
            if !crate::memory::cstr_is_empty(curbuf_mut().b_p_gefm) {
                curbuf_mut().b_p_gefm
            } else {
                opt_vars().p_gefm
            }
        } else {
            opt_vars().p_efm
        };

        let newlist = eap.cmdidx != CMD_grepadd && eap.cmdidx != CMD_lgrepadd;

        let res = qf_init(
            wp,
            fname_ptr,
            errorformat,
            c_int::from(newlist),
            qf_cmdtitle(*eap.cmdlinep),
            enc,
        );

        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());
        if !wp.is_null() {
            qi = get_loc_list( &*wp);
        }
        if !qi.is_null() {
            if res >= 0 {
                qf_list_changed(qf_get_curlist(&mut *qi));
            }
            let save_qfid = qf_get_curlist(&mut *qi).qf_id;
            if !au_name.is_null() {
                apply_autocmds(
                    EVENT_QUICKFIXCMDPOST,
                    au_name.cast_mut(),
                    curbuf_mut().b_fname,
                    true,
                    g_st().curbuf,
                );
            }
            if res > 0 && eap.forceit == 0 && qflist_valid(wp, save_qfid) {
                qf_jump_first(&mut *qi, save_qfid, 0);
            }
        }

        decr_quickfix_busy();
        os_remove(fname_ptr);
    }
}

fn get_mef_name() -> Option<std::ffi::CString> {
    unsafe {
        if crate::memory::cstr_is_empty(opt_vars().p_mef) {
            let name = vim_tempname();
            if name.is_none() {
                emsg(gettext(e_notmp));
            }
            return name;
        }

        let mut p = opt_vars().p_mef;
        while *p != NUL {
            if *p == b'#' as c_char && *p.add(1) == b'#' as c_char {
                break;
            }
            p = p.add(1);
        }

        if *p == NUL {
            return Some(crate::memory::cstr_to_cstring(opt_vars().p_mef));
        }

        loop {
            if qf_st().mef_start == -1 {
                qf_st().mef_start = os_get_pid() as c_int;
            } else {
                qf_st().mef_off += 19;
            }
            let mef_len = crate::memory::cstr_len(opt_vars().p_mef);
            let mut name_vec: Vec<c_char> = vec![0; mef_len + 30];
            let name = name_vec.as_mut_ptr();
            crate::memory::cstr_cpy(name, opt_vars().p_mef);
            let off_idx = p.offset_from(opt_vars().p_mef) as usize;
            vim_snprintf!(
                name.add(off_idx),
                mef_len + 30 - off_idx,
                c"%d%d",
                qf_st().mef_start,
                qf_st().mef_off,
            );
            c_strcat(name, p.add(2));
            let mut file_info = FileInfo::new();
            let file_or_link_found = os_fileinfo_link(name, &mut file_info);
            if !file_or_link_found {
                return Some(crate::memory::cstr_to_cstring(name));
            }
        }
    }
}

pub fn qf_get_size(eap: &mut exarg_T) -> usize {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 0;
        }
        qf_get_curlist(&mut *qi).qf_items.len()
    }
}

pub fn qf_get_valid_size(eap: &mut exarg_T) -> usize {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 0;
        }

        let mut prev_fnum: c_int = 0;
        let mut sz: usize = 0;
        let qfl = qf_get_curlist(&mut *qi);
        for qfp in &qfl.qf_items {
            if g_st().got_int {
                break;
            }
            if qfp.qf_valid != 0 {
                if eap.cmdidx == CMD_cdo || eap.cmdidx == CMD_ldo {
                    sz += 1;
                } else if qfp.qf_fnum > 0 && qfp.qf_fnum != prev_fnum {
                    sz += 1;
                    prev_fnum = qfp.qf_fnum;
                }
            }
        }

        sz
    }
}

pub fn qf_get_cur_idx(eap: &mut exarg_T) -> usize {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 0;
        }

        assert!(qf_get_curlist(&mut *qi).qf_index >= 0);
        qf_get_curlist(&mut *qi).qf_index as usize
    }
}

pub fn qf_get_cur_valid_idx(eap: &mut exarg_T) -> c_int {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 1;
        }

        let qfl = qf_get_curlist(&mut *qi);
        if !qf_list_has_valid_entries(qfl) {
            return 1;
        }

        let mut prev_fnum: c_int = 0;
        let mut eidx: c_int = 0;
        assert!(qfl.qf_index >= 0);
        let limit = (qfl.qf_index as usize).min(qfl.qf_items.len());
        for qfp in &(&qfl.qf_items)[..limit] {
            if qfp.qf_valid != 0 {
                if eap.cmdidx == CMD_cfdo || eap.cmdidx == CMD_lfdo {
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
}

fn qf_get_nth_valid_entry(qfl: &mut qf_list_T, n: usize, fdo: bool) -> usize {
    if !qf_list_has_valid_entries(qfl) {
            return 1;
        }

        let mut prev_fnum: c_int = 0;
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

pub fn ex_cc(eap: &mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let mut errornr: c_int = if eap.addr_count > 0 {
            eap.line2 as c_int
        } else {
            match eap.cmdidx {
                CMD_cc | CMD_ll => 0,
                CMD_crewind | CMD_lrewind | CMD_cfirst | CMD_lfirst => 1,
                _ => 32767,
            }
        };

        if eap.cmdidx == CMD_cdo
            || eap.cmdidx == CMD_ldo
            || eap.cmdidx == CMD_cfdo
            || eap.cmdidx == CMD_lfdo
        {
            let n = if eap.addr_count > 0 {
                assert!(eap.line1 >= 0);
                eap.line1 as usize
            } else {
                1
            };
            let valid_entry = qf_get_nth_valid_entry(
                qf_get_curlist(&mut *qi),
                n,
                eap.cmdidx == CMD_cfdo || eap.cmdidx == CMD_lfdo,
            );
            assert!(valid_entry <= c_int::MAX as usize);
            errornr = valid_entry as c_int;
        }

        qf_jump(qi, 0, errornr, eap.forceit);
    }
}

pub fn ex_cnext(eap: &mut exarg_T) {
    let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let errornr = if eap.addr_count > 0
            && eap.cmdidx != CMD_cdo
            && eap.cmdidx != CMD_ldo
            && eap.cmdidx != CMD_cfdo
            && eap.cmdidx != CMD_lfdo
        {
            eap.line2 as c_int
        } else {
            1
        };

        let dir = match eap.cmdidx {
            CMD_cprevious | CMD_lprevious | CMD_cNext | CMD_lNext => BACKWARD,
            CMD_cnfile | CMD_lnfile | CMD_cfdo | CMD_lfdo => FORWARD_FILE,
            CMD_cpfile | CMD_lpfile | CMD_cNfile | CMD_lNfile => BACKWARD_FILE,
            _ => FORWARD,
        };

        qf_jump(qi, dir, errornr, eap.forceit);
}

fn qf_find_first_entry_in_buf(items: &[qfline_T], bnr: c_int) -> Option<usize> {
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

fn qf_find_first_entry_on_line(items: &[qfline_T], mut idx: usize) -> usize {
    while !g_st().got_int
            && idx > 0
            && items[idx].qf_fnum == items[idx - 1].qf_fnum
            && items[idx].qf_lnum == items[idx - 1].qf_lnum
        {
            idx -= 1;
        }
        idx
}

fn qf_find_last_entry_on_line(items: &[qfline_T], mut idx: usize) -> usize {
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
fn qf_entry_after_pos(qfp: &qfline_T, pos: &pos_T, linewise: bool) -> bool {
    if linewise {
            qfp.qf_lnum > pos.lnum
        } else {
            qfp.qf_lnum > pos.lnum
                || (qfp.qf_lnum == pos.lnum && qfp.qf_col > pos.col)
        }
}

#[inline]
fn qf_entry_before_pos(qfp: &qfline_T, pos: &pos_T, linewise: bool) -> bool {
    if linewise {
            qfp.qf_lnum < pos.lnum
        } else {
            qfp.qf_lnum < pos.lnum
                || (qfp.qf_lnum == pos.lnum && qfp.qf_col < pos.col)
        }
}

#[inline]
fn qf_entry_on_or_after_pos(
    qfp: &qfline_T,
    pos: &pos_T,
    linewise: bool,
) -> bool {
    if linewise {
            qfp.qf_lnum >= pos.lnum
        } else {
            qfp.qf_lnum > pos.lnum
                || (qfp.qf_lnum == pos.lnum && qfp.qf_col >= pos.col)
        }
}

#[inline]
fn qf_entry_on_or_before_pos(
    qfp: &qfline_T,
    pos: &pos_T,
    linewise: bool,
) -> bool {
    if linewise {
            qfp.qf_lnum <= pos.lnum
        } else {
            qfp.qf_lnum < pos.lnum
                || (qfp.qf_lnum == pos.lnum && qfp.qf_col <= pos.col)
        }
}

fn qf_find_entry_after_pos(
    items: &[qfline_T],
    bnr: c_int,
    pos: &pos_T,
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
    items: &[qfline_T],
    bnr: c_int,
    pos: &pos_T,
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
    items: &[qfline_T],
    bnr: c_int,
    pos: &pos_T,
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
    items: &[qfline_T],
    mut idx: usize,
    mut n: linenr_T,
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
    items: &[qfline_T],
    mut idx: usize,
    mut n: linenr_T,
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
    qfl: &mut qf_list_T,
    bnr: c_int,
    pos: &mut pos_T,
    mut n: linenr_T,
    dir: Direction,
    linewise: bool,
) -> c_int {
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

        (idx + 1) as c_int
}

pub fn ex_cbelow(eap: &mut exarg_T) {
    unsafe {
        if eap.addr_count > 0 && eap.line2 <= 0 {
            emsg(gettext(e_invrange));
            return;
        }

        let buf_has_flag = if eap.cmdidx == CMD_cabove
            || eap.cmdidx == CMD_cbelow
            || eap.cmdidx == CMD_cbefore
            || eap.cmdidx == CMD_cafter
        {
            BUF_HAS_QF_ENTRY
        } else {
            BUF_HAS_LL_ENTRY
        };

        if (curbuf_mut().b_has_qf_entry & buf_has_flag) == 0 {
            emsg(gettext(e_no_errors));
            return;
        }

        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let qfl = qf_get_curlist(&mut *qi);
        if !qf_list_has_valid_entries(qfl) {
            emsg(gettext(e_no_errors));
            return;
        }

        let dir = if eap.cmdidx == CMD_cbelow
            || eap.cmdidx == CMD_lbelow
            || eap.cmdidx == CMD_cafter
            || eap.cmdidx == CMD_lafter
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
            if eap.addr_count > 0 {
                eap.line2
            } else {
                0
            },
            dir,
            eap.cmdidx == CMD_cbelow
                || eap.cmdidx == CMD_lbelow
                || eap.cmdidx == CMD_cabove
                || eap.cmdidx == CMD_labove,
        );

        if errornr > 0 {
            qf_jump(qi, 0, errornr, 0);
        } else {
            emsg(gettext(E_NO_MORE_ITEMS));
        }
    }
}

fn cfile_get_auname(cmdidx: c_int) -> *const c_char {
    match cmdidx {
        CMD_cfile => c"cfile".as_ptr(),
        CMD_cgetfile => c"cgetfile".as_ptr(),
        CMD_caddfile => c"caddfile".as_ptr(),
        CMD_lfile => c"lfile".as_ptr(),
        CMD_lgetfile => c"lgetfile".as_ptr(),
        CMD_laddfile => c"laddfile".as_ptr(),
        _ => ptr::null(),
    }
}

pub fn ex_cfile(eap: &mut exarg_T) {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();
        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());

        let au_name = cfile_get_auname(eap.cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            )
            && aborting()
        {
            return;
        }
        if *eap.arg != NUL {
            set_option_direct(kOptErrorfile, cstr_as_obj(eap.arg), 0, 0);
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
            wp,
            opt_vars().p_ef,
            opt_vars().p_efm,
            c_int::from(eap.cmdidx != CMD_caddfile && eap.cmdidx != CMD_laddfile),
            qf_cmdtitle(*eap.cmdlinep),
            enc,
        );
        if !wp.is_null() {
            qi = get_loc_list( &*wp);
            if qi.is_null() {
                decr_quickfix_busy();
                return;
            }
        }
        if res >= 0 {
            qf_list_changed(qf_get_curlist(&mut *qi));
        }
        let save_qfid = qf_get_curlist(&mut *qi).qf_id;
        if !au_name.is_null() {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.cast_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
        if res > 0
            && (eap.cmdidx == CMD_cfile || eap.cmdidx == CMD_lfile)
            && qflist_valid(wp, save_qfid)
        {
            qf_jump_first(&mut *qi, save_qfid, eap.forceit);
        }

        decr_quickfix_busy();
    }
}

fn vgr_get_auname(cmdidx: c_int) -> *const c_char {
    match cmdidx {
        CMD_vimgrep => c"vimgrep".as_ptr(),
        CMD_lvimgrep => c"lvimgrep".as_ptr(),
        CMD_vimgrepadd => c"vimgrepadd".as_ptr(),
        CMD_lvimgrepadd => c"lvimgrepadd".as_ptr(),
        CMD_grep => c"grep".as_ptr(),
        CMD_lgrep => c"lgrep".as_ptr(),
        CMD_grepadd => c"grepadd".as_ptr(),
        CMD_lgrepadd => c"lgrepadd".as_ptr(),
        _ => ptr::null(),
    }
}

fn vgr_init_regmatch(regmatch: &mut regmmatch_T, s: *mut c_char) {
    unsafe {
        regmatch.regprog = ptr::null_mut();

        if s.is_null() || *s == NUL {
            if last_search_pat().is_null() {
                emsg(gettext(e_noprevre));
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

fn vgr_display_fname(fname: *mut c_char) {
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
    fname: *mut c_char,
    dirname_start: *mut c_char,
    dirname_now: *mut c_char,
) -> *mut buf_T {
    let save_ei = au_event_disable(c",Filetype");

        let save_mls = opt_vars().p_mls;
        opt_vars().p_mls = 0;

        let buf = load_dummy_buffer(fname, dirname_start, dirname_now);

        opt_vars().p_mls = save_mls;
        au_event_restore(Some(save_ei));

        buf
}

fn vgr_qflist_valid(
    wp: *mut win_T,
    qi: &mut qf_info_T,
    qfid: c_uint,
    title: *mut c_char,
) -> bool {
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
    qfl: &mut qf_list_T,
    fname: *mut c_char,
    buf: &mut buf_T,
    spat: *mut c_char,
    regmatch: &mut regmmatch_T,
    tomatch: &mut c_int,
    duplicate_name: c_int,
    flags: c_int,
) -> bool {
    unsafe {
        let mut found_match = false;
        let pat_len = crate::memory::cstr_len(spat).min(FUZZY_MATCH_MAX_LEN);

        let mut lnum: linenr_T = 1;
        while lnum <= buf.b_ml.ml_line_count && *tomatch > 0 {
            let mut col: colnr_T = 0;
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
                        if duplicate_name != 0 {
                            0
                        } else {
                            buf.b_fnum
                        },
                        ml_get_buf(&raw mut *buf, regmatch.startpos[0].lnum + lnum),
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
                    col =
                        regmatch.endpos[0].col + colnr_T::from(col == regmatch.endpos[0].col);
                    if col > ml_get_buf_len(&raw mut *buf, lnum) {
                        break;
                    }
                }
            } else {
                let str_ = ml_get_buf(&raw mut *buf, lnum);
                let linelen = ml_get_buf_len(&raw mut *buf, lnum);
                let mut score: c_int = 0;
                let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];

                while fuzzy_match(
                    str_.add(col as usize),
                    spat,
                    false,
                    &mut score,
                    matches.as_mut_ptr(),
                    FUZZY_MATCH_MAX_LEN as c_int,
                ) {
                    if qf_add_entry(
                        qfl,
                        ptr::null_mut(),
                        fname,
                        ptr::null_mut(),
                        if duplicate_name != 0 {
                            0
                        } else {
                            buf.b_fnum
                        },
                        str_,
                        lnum,
                        0,
                        matches[0] as colnr_T + col + 1,
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
                    col = matches[pat_len - 1] as colnr_T + col + 1;
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
    qi: &mut qf_info_T,
    forceit: c_int,
    redraw_for_dummy: &mut bool,
    first_match_buf: *mut buf_T,
    target_dir: *mut c_char,
) {
    let buf = g_st().curbuf;
        qf_jump(&raw mut *qi, 0, 0, forceit);
        if buf != g_st().curbuf {
            *redraw_for_dummy = false;
        }

        if g_st().curbuf == first_match_buf && !target_dir.is_null() {
            let mut ea: exarg_T = exarg_T::new();
            ea.arg = target_dir;
            ea.cmdidx = CMD_lcd;
            ex_cd(&mut ea);
        }
}

fn existing_swapfile(buf: &buf_T) -> bool {
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

fn vgr_process_args(eap: &mut exarg_T, args: &mut vgr_args_T) -> bool {
    unsafe {
        *args = vgr_args_T::new();

        args.regmatch.regprog = ptr::null_mut();
        args.qf_title = Some(crate::memory::cstr_to_cstring(qf_cmdtitle(*eap.cmdlinep)));
        args.tomatch = if eap.addr_count > 0 {
            eap.line2
        } else {
            MAXLNUM
        };

        let mut spat: *mut c_char = ptr::null_mut();
        let mut p = skip_vimgrep_pat(eap.arg, &raw mut spat, Some(&mut args.flags));
        if p.is_null() {
            emsg(gettext(e_invalpat));
            return false;
        }
        args.spat = (!spat.is_null()).then(|| crate::memory::cstr_to_cstring(spat));

        let spat_ptr = args.spat_ptr();
        vgr_init_regmatch(&mut args.regmatch, spat_ptr);
        if args.regmatch.regprog.is_null() {
            return false;
        }

        p = skipwhite(p);
        if *p == NUL {
            emsg(gettext(
                c"E683: File name missing or invalid pattern",
            ));
            return false;
        }

        if !get_arglist_exp(p, &mut args.fnames, true) || args.fnames.is_empty() {
            emsg(gettext(e_nomatch));
            return false;
        }

        true
    }
}

fn vgr_process_files(
    wp: *mut win_T,
    qi: &mut qf_info_T,
    cmd_args: &mut vgr_args_T,
    redraw_for_dummy: &mut bool,
    first_match_buf: *mut *mut buf_T,
    target_dir: &mut Option<std::ffi::CString>,
) -> bool {
    unsafe {
        let mut status = false;
        let mut save_qfid = qf_get_curlist(&mut *qi).qf_id;
        let mut duplicate_name = false;

        let mut dirname_start = vec![0 as c_char; MAXPATHL];
        let mut dirname_now = vec![0 as c_char; MAXPATHL];

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
                buf =
                    vgr_load_dummy_buf(fname, dirname_start.as_mut_ptr(), dirname_now.as_mut_ptr());
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
                    cmd_args.spat_ptr(),
                    &mut cmd_args.regmatch,
                    &mut cmd_args.tomatch,
                    c_int::from(duplicate_name),
                    cmd_args.flags,
                );

                if using_dummy {
                    if found_match && (*first_match_buf).is_null() {
                        *first_match_buf = buf;
                    }
                    if duplicate_name {
                        wipe_dummy_buffer(&mut *buf, dirname_start.as_mut_ptr());
                        buf = ptr::null_mut();
                    } else if (g_st().cmdmod.cmod_flags & CMOD_HIDE) == 0
                        || *(*buf).b_p_bh == b'u' as c_char
                        || *(*buf).b_p_bh == b'w' as c_char
                        || *(*buf).b_p_bh == b'd' as c_char
                    {
                        if !found_match {
                            wipe_dummy_buffer(&mut *buf, dirname_start.as_mut_ptr());
                            buf = ptr::null_mut();
                        } else if buf != *first_match_buf
                            || (cmd_args.flags & VGR_NOJUMP) != 0
                            || existing_swapfile(&*buf)
                        {
                            unload_dummy_buffer(buf, dirname_start.as_mut_ptr());
                            (*buf).b_flags &= !BF_DUMMY;
                            buf = ptr::null_mut();
                        }
                    }

                    if !buf.is_null() {
                        (*buf).b_flags &= !BF_DUMMY;

                        if buf == *first_match_buf
                            && target_dir.is_none()
                            && (crate::memory::cstr_as_bytes(dirname_start.as_ptr()) != crate::memory::cstr_as_bytes(dirname_now.as_ptr()))
                        {
                            *target_dir = Some(crate::memory::cstr_to_cstring(dirname_now.as_ptr()));
                        }

                        let mut aco: CtxSwitch = CtxSwitch::new();
                        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
                        apply_autocmds(EVENT_FILETYPE, (*buf).b_p_ft, (*buf).b_fname, true, buf);
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

pub fn ex_vimgrep(eap: &mut exarg_T) {
    unsafe {
        if !check_can_set_curbuf_forceit(eap.forceit) {
            return;
        }

        let au_name = vgr_get_auname(eap.cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            )
            && aborting()
        {
            return;
        }

        let mut wp: *mut win_T = ptr::null_mut();
        let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);
        let mut target_dir: Option<std::ffi::CString> = None;
        let mut args = vgr_args_T::new();
        if vgr_process_args(eap, &mut args) {
            if (eap.cmdidx != CMD_grepadd
                && eap.cmdidx != CMD_lgrepadd
                && eap.cmdidx != CMD_vimgrepadd
                && eap.cmdidx != CMD_lvimgrepadd)
                || qf_stack_empty(qi)
            {
                qf_new_list(
                    &mut *qi,
                    args.qf_title.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                );
            }

            incr_quickfix_busy();

            let mut redraw_for_dummy = false;
            let mut first_match_buf: *mut buf_T = ptr::null_mut();
            let status = vgr_process_files(
                wp,
                &mut *qi,
                &mut args,
                &mut redraw_for_dummy,
                &raw mut first_match_buf,
                &mut target_dir,
            );

            if status {
                let qfl = qf_get_curlist(&mut *qi);
                qfl.qf_nonevalid = false;
                qfl.qf_index = 1;
                qf_list_changed(qfl);

                qf_update_buffer(&mut *qi, 0);

                let save_qfid = qf_get_curlist(&mut *qi).qf_id;

                if !au_name.is_null() {
                    apply_autocmds(
                        EVENT_QUICKFIXCMDPOST,
                        au_name.cast_mut(),
                        curbuf_mut().b_fname,
                        true,
                        g_st().curbuf,
                    );
                }

                if qflist_valid(wp, save_qfid) && qf_restore_list(&mut *qi, save_qfid) {
                    if !qf_list_empty(qf_get_curlist(&mut *qi)) {
                        if (args.flags & VGR_NOJUMP) == 0 {
                            vgr_jump_to_match(
                                &mut *qi,
                                eap.forceit,
                                &mut redraw_for_dummy,
                                first_match_buf,
                                target_dir
                                    .as_ref()
                                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
                            );
                        }
                    } else {
                        semsg!(gettext(e_nomatch2), args.spat_ptr());
                    }

                    decr_quickfix_busy();

                    if redraw_for_dummy {
                        foldUpdateAll(curwin_mut());
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
}

fn restore_start_dir(dirname_start: *mut c_char) {
     {
        let mut dirname_now = vec![0 as c_char; MAXPATHL];

        os_dirname(dirname_now.as_mut_ptr(), MAXPATHL);
        if crate::memory::cstr_as_bytes(dirname_start) != crate::memory::cstr_as_bytes(dirname_now.as_ptr()) {
            let mut ea: exarg_T = exarg_T::new();
            ea.arg = dirname_start;
            ea.cmdidx = if curwin_mut().w_localdir.is_none() {
                CMD_cd
            } else {
                CMD_lcd
            };
            ex_cd(&mut ea);
        }
    }
}

fn load_dummy_buffer(
    fname: *mut c_char,
    dirname_start: *mut c_char,
    resulting_dir: *mut c_char,
) -> *mut buf_T {
    unsafe {
        let mut newbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
        if newbuf.is_null() {
            return ptr::null_mut();
        }

        let mut failed = true;
        let mut newbufref: bufref_T = bufref_T::new();
        set_bufref(&mut newbufref, newbuf);

        buf_copy_options( &mut *newbuf, BCO_ENTER | BCO_NOHELP);

        if ml_open(&mut *newbuf) {
            (*newbuf).b_locked += 1;
            let mut aco: CtxSwitch = CtxSwitch::new();
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), newbuf, 0);

            setfname(curbuf_mut(), fname, ptr::null_mut(), false);
            check_need_swap(true);
            curbuf_mut().b_flags &= !BF_DUMMY;

            let mut newbuf_to_wipe: bufref_T = bufref_T::new();
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
            (*newbuf).b_locked -= 1;
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
                wipe_dummy_buffer(&mut *newbuf_to_wipe.br_buf, ptr::null_mut());
                unblock_autocmds();
            }

            (*newbuf).b_flags |= BF_DUMMY;
        }

        os_dirname(resulting_dir, MAXPATHL);
        restore_start_dir(dirname_start);

        if !bufref_valid(Some(&newbufref)) {
            return ptr::null_mut();
        }
        if failed {
            wipe_dummy_buffer(&mut *newbuf, dirname_start);
            return ptr::null_mut();
        }
        newbuf
    }
}

fn wipe_dummy_buffer(buf: &mut buf_T, dirname_start: *mut c_char) {
    unsafe {
        let mut give_up = false;
        loop {
            if buf.b_nwindows <= 0 {
                break;
            }
            let mut did_one = false;

            if !firstwin_mut().w_next.is_null() {
                for wp in for_all_windows_ptr() {
                    if (*wp).w_buffer == buf {
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
            let mut cs: cleanup_T = cleanup_T::new();
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

fn unload_dummy_buffer(buf: *mut buf_T, dirname_start: *mut c_char) {
    if g_st().curbuf == buf {
            return;
        }

        close_buffer(ptr::null_mut(), buf, DOBUF_UNLOAD, false, true, false);
        restore_start_dir(dirname_start);
}

fn get_qfline_items(qfp: &mut qfline_T, list: &mut list_T) -> bool {
    let mut bufnum = qfp.qf_fnum;
        if bufnum != 0 && buflist_findnr(bufnum).is_null() {
            bufnum = 0;
        }

        let dict = tv_dict_alloc();
        tv_list_append_dict( &mut *list, dict);

        let buf = [qfp.qf_type, NUL];
        let buflen: usize = if buf[0] == NUL { 0 } else { 1 };
        let empty = c"".as_ptr();
        if !tv_dict_add_nr(unsafe { &mut *dict }, c"bufnr", varnumber_T::from(bufnum))
            || !tv_dict_add_nr(unsafe { &mut *dict }, c"lnum", varnumber_T::from(qfp.qf_lnum))
            || !tv_dict_add_nr(
                unsafe { &mut *dict },
                c"end_lnum",
                varnumber_T::from(qfp.qf_end_lnum),
            )
            || !tv_dict_add_nr(unsafe { &mut *dict }, c"col", varnumber_T::from(qfp.qf_col))
            || !tv_dict_add_nr(
                unsafe { &mut *dict },
                c"end_col",
                varnumber_T::from(qfp.qf_end_col),
            )
            || !tv_dict_add_nr(
                unsafe { &mut *dict },
                c"vcol",
                varnumber_T::from(qfp.qf_viscol),
            )
            || !tv_dict_add_nr(unsafe { &mut *dict }, c"nr", varnumber_T::from(qfp.qf_nr))
            || !tv_dict_add_str(
                unsafe { &mut *dict },
                c"module",
                qfp.qf_module.as_ref().map_or(empty, |s| s.as_ptr()),
            )
            || !tv_dict_add_str(
                unsafe { &mut *dict },
                c"pattern",
                qfp.qf_pattern.as_ref().map_or(empty, |s| s.as_ptr()),
            )
            || !tv_dict_add_str(unsafe { &mut *dict }, c"text", qfp.qf_text.as_ptr())
            || !tv_dict_add_str_len(unsafe { &mut *dict }, c"type", buf.as_ptr(), buflen as c_int)
            || (qfp.qf_user_data.v_type != VarType::VAR_UNKNOWN
                && !tv_dict_add_tv(unsafe { &mut *dict }, c"user_data", 9, &mut qfp.qf_user_data))
            || !tv_dict_add_nr(
                unsafe { &mut *dict },
                c"valid",
                varnumber_T::from(qfp.qf_valid),
            )
        {
            std::process::abort();
        }

        true
}

fn get_errorlist(qi_arg: Option<&mut qf_info_T>,
    wp: *mut win_T,
    mut qf_idx: c_int,
    eidx: c_int,
    list: &mut list_T,
) -> bool {
    unsafe {
        let mut qi = qi_arg.map_or(ptr::null_mut(), |p| &raw mut *p);
        if qi.is_null() {
            qi = ql_info_ptr();
            if !wp.is_null() {
                qi = get_loc_list( &*wp);
            }
            if qi.is_null() {
                return false;
            }
        }

        if eidx < 0 {
            return true;
        }

        if qf_idx == INVALID_QFIDX {
            qf_idx = (*qi).qf_curlist;
        }

        if qf_idx >= (*qi).qf_listcount {
            return false;
        }

        let qfl = qf_get_list(&mut *qi, qf_idx);
        if qf_list_empty(qfl) {
            return false;
        }

        for (idx, qfp) in qfl.qf_items.iter_mut().enumerate() {
            if g_st().got_int {
                break;
            }
            let i = (idx + 1) as c_int;
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
}

fn qf_get_list_from_lines(
    what: &mut dict_T,
    di: &mut dictitem_T,
    retdict: &mut dict_T,
) -> bool {
    unsafe {
        if di.di_tv.v_type != VarType::VAR_LIST || di.di_tv.vval.v_list().is_null() {
            return false;
        }

        let mut errorformat = opt_vars().p_efm;
        let efm_di = tv_dict_find(what, c"efm", 3);
        if !efm_di.is_null() {
            if (*efm_di).di_tv.v_type != VarType::VAR_STRING
                || (*efm_di).di_tv.vval.v_string().is_null()
            {
                return false;
            }
            errorformat = (*efm_di).di_tv.vval.v_string();
        }

        let l = tv_list_alloc(kListLenMayKnow);
        let mut qi = qf_info_T {
            qf_refcount: 1,
            qf_listcount: 0,
            qf_curlist: 0,
            qf_maxcount: 1,
            qf_lists: vec![qf_list_T::default()],
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
            get_errorlist(Some(&mut qi), ptr::null_mut(), 0, 0,  &mut *l);
        }

        tv_dict_add_list(retdict, c"items", l);
        true
    }
}

fn qf_winid(qi: *mut qf_info_T) -> c_int {
    unsafe {
        if qi.is_null() {
            return 0;
        }
        let win = qf_find_win(&*qi);
        if !win.is_null() {
            return (*win).handle;
        }
        0
    }
}

fn qf_getprop_qfbufnr(qi: *const qf_info_T, retdict: &mut dict_T) -> bool {
    unsafe {
        let mut bufnum: c_int = 0;
        if !qi.is_null() && !buflist_findnr((*qi).qf_bufnr).is_null() {
            bufnum = (*qi).qf_bufnr;
        }
        tv_dict_add_nr(retdict, c"qfbufnr", varnumber_T::from(bufnum))
    }
}

fn qf_getprop_keys2flags(what: &dict_T, loclist: bool) -> c_int {
    let mut flags = QF_GETLIST_NONE;

        if !tv_dict_find(what, c"all", 3).is_null() {
            flags |= QF_GETLIST_ALL;
            if !loclist {
                flags &= !QF_GETLIST_FILEWINID;
            }
        }
        if !tv_dict_find(what, c"title", 5).is_null() {
            flags |= QF_GETLIST_TITLE;
        }
        if !tv_dict_find(what, c"nr", 2).is_null() {
            flags |= QF_GETLIST_NR;
        }
        if !tv_dict_find(what, c"winid", 5).is_null() {
            flags |= QF_GETLIST_WINID;
        }
        if !tv_dict_find(what, c"context", 7).is_null() {
            flags |= QF_GETLIST_CONTEXT;
        }
        if !tv_dict_find(what, c"id", 2).is_null() {
            flags |= QF_GETLIST_ID;
        }
        if !tv_dict_find(what, c"items", 5).is_null() {
            flags |= QF_GETLIST_ITEMS;
        }
        if !tv_dict_find(what, c"idx", 3).is_null() {
            flags |= QF_GETLIST_IDX;
        }
        if !tv_dict_find(what, c"size", 4).is_null() {
            flags |= QF_GETLIST_SIZE;
        }
        if !tv_dict_find(what, c"changedtick", 11).is_null() {
            flags |= QF_GETLIST_TICK;
        }
        if loclist && !tv_dict_find(what, c"filewinid", 9).is_null() {
            flags |= QF_GETLIST_FILEWINID;
        }
        if !tv_dict_find(what, c"qfbufnr", 7).is_null() {
            flags |= QF_GETLIST_QFBUFNR;
        }
        if !tv_dict_find(what, c"quickfixtextfunc", 16).is_null() {
            flags |= QF_GETLIST_QFTF;
        }

        flags
}

fn qf_getprop_qfidx(qi: &mut qf_info_T, what: &mut dict_T) -> c_int {
    unsafe {
        let mut qf_idx = qi.qf_curlist;
        let mut di = tv_dict_find(what, c"nr", 2);
        if !di.is_null() {
            if (*di).di_tv.v_type == VarType::VAR_NUMBER {
                if (*di).di_tv.vval.v_number() != 0 {
                    qf_idx = (*di).di_tv.vval.v_number() as c_int - 1;
                    if qf_idx < 0 || qf_idx >= qi.qf_listcount {
                        qf_idx = INVALID_QFIDX;
                    }
                }
            } else if (*di).di_tv.v_type == VarType::VAR_STRING
                && (crate::memory::cstr_as_opt_bytes((*di).di_tv.vval.v_string()) == Some(b"$"))
            {
                qf_idx = qi.qf_listcount - 1;
            } else {
                qf_idx = INVALID_QFIDX;
            }
        }

        di = tv_dict_find(what, c"id", 2);
        if !di.is_null() {
            if (*di).di_tv.v_type == VarType::VAR_NUMBER {
                if (*di).di_tv.vval.v_number() != 0 {
                    qf_idx = qf_id2nr(qi, (*di).di_tv.vval.v_number() as c_uint);
                }
            } else {
                qf_idx = INVALID_QFIDX;
            }
        }

        qf_idx
    }
}

fn qf_getprop_defaults(
    qi: *mut qf_info_T,
    flags: c_int,
    locstack: bool,
    retdict: &mut dict_T,
) -> bool {
    unsafe {
        let mut status = true;
        let empty = c"".as_ptr();

        if (flags & QF_GETLIST_TITLE) != 0 {
            status = tv_dict_add_str_len(retdict, c"title", empty, 0);
        }
        if status && (flags & QF_GETLIST_ITEMS) != 0 {
            let l = tv_list_alloc(kListLenMayKnow);
            status = tv_dict_add_list(retdict, c"items", l);
        }
        if status && (flags & QF_GETLIST_NR) != 0 {
            status = tv_dict_add_nr(retdict, c"nr", 0);
        }
        if status && (flags & QF_GETLIST_WINID) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"winid",
                varnumber_T::from(qf_winid(&raw mut *qi)),
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
            status = qf_getprop_qfbufnr(&raw mut *qi, &mut *retdict);
        }
        if status && (flags & QF_GETLIST_QFTF) != 0 {
            status = tv_dict_add_str_len(retdict, c"quickfixtextfunc", empty, 0);
        }

        status
    }
}

fn qf_getprop_title(qfl: &mut qf_list_T, retdict: &mut dict_T) -> bool {
    tv_dict_add_str(
            retdict,
            c"title",
            qfl.qf_title.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
        )
}

fn qf_getprop_filewinid(
    wp: *const win_T,
    qi: &qf_info_T,
    retdict: &mut dict_T,
) -> bool {
    unsafe {
        let mut winid: handle_T = 0;

        if !wp.is_null() && is_ll_window(&*wp) {
            let ll_wp = qf_find_win_with_loclist(qi);
            if !ll_wp.is_null() {
                winid = (*ll_wp).handle;
            }
        }

        tv_dict_add_nr(retdict, c"filewinid", varnumber_T::from(winid))
    }
}

fn qf_getprop_items(
    qi: &mut qf_info_T,
    qf_idx: c_int,
    eidx: c_int,
    retdict: &mut dict_T,
) -> bool {
    unsafe {
        let l = tv_list_alloc(kListLenMayKnow);
        get_errorlist(Some(&mut *qi), ptr::null_mut(), qf_idx, eidx,  &mut *l);
        tv_dict_add_list(retdict, c"items", l);
        true
    }
}

fn qf_getprop_ctx(qfl: &mut qf_list_T, retdict: &mut dict_T) -> bool {
    unsafe {
        if let Some(ref ctx) = qfl.qf_ctx {
            let di = tv_dict_item_alloc_len(c"context", 7);
            tv_copy(ctx, &mut (*di).di_tv);
            let status = tv_dict_add(retdict, di);
            if !status {
                tv_dict_item_free(di);
            }
            status
        } else {
            tv_dict_add_str_len(retdict, c"context", c"", 0)
        }
    }
}

fn qf_getprop_idx(qfl: &mut qf_list_T, mut eidx: c_int, retdict: &mut dict_T) -> bool {
    if eidx == 0 {
            eidx = qfl.qf_index;
            if qf_list_empty(qfl) {
                eidx = 0;
            }
        }
        tv_dict_add_nr(retdict, c"idx", varnumber_T::from(eidx))
}

fn qf_getprop_qftf(qfl: &mut qf_list_T, retdict: &mut dict_T) -> bool {
    if qfl.qf_qftf_cb.cb_type() != kCallbackNone {
            let mut tv: typval_T = typval_T::new();
            callback_put(&mut qfl.qf_qftf_cb, &mut tv);
            let status = tv_dict_add_tv(retdict, c"quickfixtextfunc", 16, &mut tv);
            tv_clear(&raw mut tv);
            status
        } else {
            tv_dict_add_str_len(retdict, c"quickfixtextfunc", c"", 0)
        }
}

fn qf_get_properties(wp: *mut win_T, what: &mut dict_T, retdict: &mut dict_T) -> bool {
    unsafe {
        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());
        let mut status = true;
        let mut qf_idx = INVALID_QFIDX;

        let di = tv_dict_find(what, c"lines", 5);
        if !di.is_null() {
            return qf_get_list_from_lines(what,  &mut *di, retdict);
        }

        if !wp.is_null() {
            qi = get_loc_list( &*wp);
        }

        let flags = qf_getprop_keys2flags(what, !wp.is_null());

        if !qf_stack_empty(qi) {
            qf_idx = qf_getprop_qfidx(&mut *qi, what);
        }

        if qf_stack_empty(qi) || qf_idx == INVALID_QFIDX {
            return qf_getprop_defaults(qi, flags, !wp.is_null(), retdict);
        }

        let qfl = qf_get_list(&mut *qi, qf_idx);
        let mut eidx: c_int = 0;

        let di = tv_dict_find(what, c"idx", 3);
        if !di.is_null() {
            if (*di).di_tv.v_type != VarType::VAR_NUMBER {
                return false;
            }
            eidx = (*di).di_tv.vval.v_number() as c_int;
        }

        if (flags & QF_GETLIST_TITLE) != 0 {
            status = qf_getprop_title(qfl, retdict);
        }
        if status && (flags & QF_GETLIST_NR) != 0 {
            status = tv_dict_add_nr(retdict, c"nr", varnumber_T::from(qf_idx + 1));
        }
        if status && (flags & QF_GETLIST_WINID) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"winid",
                varnumber_T::from(qf_winid(qi)),
            );
        }
        if status && (flags & QF_GETLIST_ITEMS) != 0 {
            status = qf_getprop_items(&mut *qi, qf_idx, eidx, retdict);
        }
        if status && (flags & QF_GETLIST_CONTEXT) != 0 {
            status = qf_getprop_ctx(qfl, retdict);
        }
        if status && (flags & QF_GETLIST_ID) != 0 {
            status = tv_dict_add_nr(retdict, c"id", varnumber_T::from(qfl.qf_id));
        }
        if status && (flags & QF_GETLIST_IDX) != 0 {
            status = qf_getprop_idx(qfl, eidx, retdict);
        }
        if status && (flags & QF_GETLIST_SIZE) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"size",
                varnumber_T::from(qfl.count()),
            );
        }
        if status && (flags & QF_GETLIST_TICK) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"changedtick",
                varnumber_T::from(qfl.qf_changedtick),
            );
        }
        if status && !wp.is_null() && (flags & QF_GETLIST_FILEWINID) != 0 {
            status = qf_getprop_filewinid(wp, &*qi, retdict);
        }
        if status && (flags & QF_GETLIST_QFBUFNR) != 0 {
            status = qf_getprop_qfbufnr(qi, retdict);
        }
        if status && (flags & QF_GETLIST_QFTF) != 0 {
            status = qf_getprop_qftf(qfl, retdict);
        }

        status
    }
}

fn qf_setprop_qftf(qfl: &mut qf_list_T, di: &mut dictitem_T) -> bool {
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
    qfl: &mut qf_list_T,
    d: &mut dict_T,
    first_entry: bool,
    valid_entry: &mut bool,
) -> c_int {
    unsafe {
        if first_entry {
            qf_st().did_bufnr_emsg = false;
        }

        let filename = crate::eval::typval::tv_dict_get_cstring(d, c"filename");
        let module = crate::eval::typval::tv_dict_get_cstring(d, c"module");
        let mut bufnum = tv_dict_get_number(d, c"bufnr") as c_int;
        let lnum = tv_dict_get_number(d, c"lnum") as linenr_T;
        let end_lnum = tv_dict_get_number(d, c"end_lnum") as linenr_T;
        let col = tv_dict_get_number(d, c"col") as c_int;
        let end_col = tv_dict_get_number(d, c"end_col") as c_int;
        let vcol = tv_dict_get_number(d, c"vcol") as c_char;
        let nr = tv_dict_get_number(d, c"nr") as c_int;
        let type_ = tv_dict_get_string(d, c"type");
        let pattern = crate::eval::typval::tv_dict_get_cstring(d, c"pattern");
        let text = crate::eval::typval::tv_dict_get_cstring(d, c"text").unwrap_or_default();
        let mut user_data: typval_T = typval_T::new();
        user_data.v_type = VarType::VAR_UNKNOWN;
        tv_dict_get_tv(d, c"user_data", &mut user_data);

        let mut valid = true;
        if (filename.is_none() && bufnum == 0) || (lnum == 0 && pattern.is_none()) {
            valid = false;
        }

        if bufnum != 0 && buflist_findnr(bufnum).is_null() {
            if !qf_st().did_bufnr_emsg {
                qf_st().did_bufnr_emsg = true;
                semsg!(gettext(e_buffer_nr_not_found), bufnum);
            }
            valid = false;
            bufnum = 0;
        }

        if !tv_dict_find(d, c"valid", -1).is_null() {
            valid = tv_dict_get_bool(d, c"valid", 0) != 0;
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
            if type_.is_null() { NUL } else { *type_ },
            Some(&user_data),
            c_char::from(valid),
        );

        drop(filename);
        drop(module);
        drop(pattern);
        drop(text);
        tv_clear(&raw mut user_data);

        if valid {
            *valid_entry = true;
        }

        status
    }
}

fn entry_is_closer_to_target(
    entry: &qfline_T,
    other_entry: &qfline_T,
    target_fnum: c_int,
    target_lnum: c_int,
    target_col: c_int,
) -> bool {
    if target_fnum == 0 {
            return false;
        }

        let is_target_file = entry.qf_fnum != 0 && entry.qf_fnum == target_fnum;
        let other_is_target_file =
            other_entry.qf_fnum != 0 && other_entry.qf_fnum == target_fnum;
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
            c_int::MAX
        };
        let other_line_distance = if other_entry.qf_lnum != 0 {
            (other_entry.qf_lnum - target_lnum).abs()
        } else {
            c_int::MAX
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
            c_int::MAX
        };
        let other_column_distance = if other_entry.qf_col != 0 {
            (other_entry.qf_col - target_col).abs()
        } else {
            c_int::MAX
        };
        if column_distance > other_column_distance {
            return false;
        } else if column_distance < other_column_distance {
            return true;
        }

        false
}

fn qf_add_entries(
    qi: &mut qf_info_T,
    mut qf_idx: c_int,
    list: *mut list_T,
    title: *mut c_char,
    action: c_int,
) -> c_int {
    unsafe {
        let mut qfl = qf_get_list(&mut *qi, qf_idx);
        let mut retval = OK;
        let mut valid_entry = false;

        let mut old_count: usize = 0;

        let mut prev_fnum: c_int = 0;
        let mut prev_lnum: c_int = 0;
        let mut prev_col: c_int = 0;
        let cur_entry = qfl.cur_entry();
        if !cur_entry.is_null() {
            prev_fnum = (*cur_entry).qf_fnum;
            prev_lnum = (*cur_entry).qf_lnum;
            prev_col = (*cur_entry).qf_col;
        }

        let mut select_first_entry = false;
        let mut select_nearest_entry = false;

        if action == b' ' as c_int || qf_idx == qi.qf_listcount {
            select_first_entry = true;
            qf_new_list(qi, title);
            qf_idx = qi.qf_curlist;
            qfl = qf_get_list(&mut *qi, qf_idx);
        } else if action == b'a' as c_int {
            if qf_list_empty(qfl) {
                select_first_entry = true;
            } else {
                old_count = qfl.qf_items.len();
            }
        } else if action == b'r' as c_int {
            select_first_entry = true;
            qf_free_items(qfl);
            qf_store_title(qfl, title);
        } else if action == b'u' as c_int {
            select_nearest_entry = true;
            qf_free_items(qfl);
            qf_store_title(qfl, title);
        }

        let mut entry_to_select_index: c_int = 0;

        let first_li = tv_list_first(list);
        for li in crate::eval::typval::list_iter_ptr(first_li) {
            if (*li).li_tv.v_type == VarType::VAR_DICT {
                let d = (*li).li_tv.vval.v_dict();
                if !d.is_null() {
                    retval = qf_add_entry_from_dict(qfl, &mut *d, li == first_li, &mut valid_entry);
                    if retval == QF_FAIL {
                        break;
                    }

                    let entry: *const qfline_T = qfl.qf_items.last().unwrap();
                    if (select_first_entry && entry_to_select_index == 0)
                        || (select_nearest_entry
                            && (entry_to_select_index == 0
                                || entry_is_closer_to_target(
                                    &*entry,
                                    &(&qfl.qf_items)
                                        [(entry_to_select_index - 1) as usize],
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

fn qf_setprop_get_qfidx(
    qi: &qf_info_T,
    what: &dict_T,
    action: c_int,
    newlist: &mut bool,
) -> c_int {
    unsafe {
        let mut qf_idx = qi.qf_curlist;

        let di = tv_dict_find(what, c"nr", 2);
        if !di.is_null() {
            if (*di).di_tv.v_type == VarType::VAR_NUMBER {
                if (*di).di_tv.vval.v_number() != 0 {
                    qf_idx = (*di).di_tv.vval.v_number() as c_int - 1;
                }

                if (action == b' ' as c_int || action == b'a' as c_int)
                    && qf_idx == qi.qf_listcount
                {
                    *newlist = true;
                    qf_idx = if qf_stack_empty(&raw const *qi) {
                        0
                    } else {
                        qi.qf_listcount - 1
                    };
                } else if qf_idx < 0 || qf_idx >= qi.qf_listcount {
                    return INVALID_QFIDX;
                } else if action != b' ' as c_int {
                    *newlist = false;
                }
            } else if (*di).di_tv.v_type == VarType::VAR_STRING
                && (crate::memory::cstr_as_opt_bytes((*di).di_tv.vval.v_string()) == Some(b"$"))
            {
                if !qf_stack_empty(&raw const *qi) {
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
            let di = tv_dict_find(what, c"id", 2);
            if !di.is_null() {
                if (*di).di_tv.v_type != VarType::VAR_NUMBER {
                    return INVALID_QFIDX;
                }
                return qf_id2nr(qi, (*di).di_tv.vval.v_number() as c_uint);
            }
        }

        qf_idx
    }
}

fn qf_setprop_title(
    qi: &mut qf_info_T,
    qf_idx: c_int,
    what: &dict_T,
    di: &dictitem_T,
) -> bool {
     {
        let qfl = qf_get_list(&mut *qi, qf_idx);
        if di.di_tv.v_type != VarType::VAR_STRING {
            return false;
        }

        qfl.qf_title = crate::eval::typval::tv_dict_get_cstring(what, c"title");
        if qf_idx == qi.qf_curlist {
            qf_update_win_titlevar(qi);
        }

        true
    }
}

fn qf_setprop_items(
    qi: &mut qf_info_T,
    qf_idx: c_int,
    di: &mut dictitem_T,
    action: c_int,
) -> bool {
    if di.di_tv.v_type != VarType::VAR_LIST {
            return false;
        }

        let title_save = (&qi.qf_lists)[qf_idx as usize].qf_title.clone();
        qf_add_entries(
            qi,
            qf_idx,
            di.di_tv.vval.v_list(),
            title_save
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            if action == b' ' as c_int {
                b'a' as c_int
            } else {
                action
            },
        ) == QF_OK
}

fn qf_setprop_items_from_lines(
    qi: &mut qf_info_T,
    qf_idx: c_int,
    what: &dict_T,
    di: &mut dictitem_T,
    action: c_int,
) -> bool {
    unsafe {
        let mut errorformat = opt_vars().p_efm;
        let mut retval = false;

        let efm_di = tv_dict_find(what, c"efm", 3);
        if !efm_di.is_null() {
            if (*efm_di).di_tv.v_type != VarType::VAR_STRING
                || (*efm_di).di_tv.vval.v_string().is_null()
            {
                return false;
            }
            errorformat = (*efm_di).di_tv.vval.v_string();
        }

        if di.di_tv.v_type != VarType::VAR_LIST || di.di_tv.vval.v_list().is_null() {
            return false;
        }

        if action == b'r' as c_int || action == b'u' as c_int {
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
}

fn qf_setprop_context(qfl: &mut qf_list_T, di: &mut dictitem_T) -> bool {
    if let Some(ref mut ctx) = qfl.qf_ctx {
            tv_clear(ctx);
        }
        let mut ctx = tv_initial_value();
        tv_copy(&di.di_tv, &mut ctx);
        qfl.qf_ctx = Some(ctx);

        true
}

fn qf_setprop_curidx(
    qi: &mut qf_info_T,
    qfl: &mut qf_list_T,
    di: &dictitem_T,
) -> bool {
     {
        let mut newidx: c_int;

        if di.di_tv.v_type == VarType::VAR_STRING
            && !di.di_tv.vval.v_string().is_null()
            && (crate::memory::cstr_as_bytes(di.di_tv.vval.v_string()) == b"$")
        {
            newidx = qfl.count();
        } else {
            let mut denote = false;
            newidx = tv_get_number_chk(&di.di_tv, Some(&mut denote)) as c_int;
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
}

fn qf_set_properties(
    qi: &mut qf_info_T,
    what: &dict_T,
    action: c_int,
    title: *mut c_char,
) -> bool {
    unsafe {
        let mut newlist = action == b' ' as c_int || qf_stack_empty(&raw mut *qi);
        let mut qf_idx = qf_setprop_get_qfidx(qi, what, action, &mut newlist);
        if qf_idx == INVALID_QFIDX {
            return false;
        }

        if newlist {
            qi.qf_curlist = qf_idx;
            qf_new_list(qi, title);
            qf_idx = qi.qf_curlist;
        }

        let qfl = qf_get_list(&mut *qi, qf_idx);
        let mut retval = false;
        let mut di = tv_dict_find(what, c"title", 5);
        if !di.is_null() {
            retval = qf_setprop_title(qi, qf_idx, what,  &*di);
        }
        di = tv_dict_find(what, c"items", 5);
        if !di.is_null() {
            retval = qf_setprop_items(qi, qf_idx, &mut *di, action);
        }
        di = tv_dict_find(what, c"lines", 5);
        if !di.is_null() {
            retval = qf_setprop_items_from_lines(qi, qf_idx, what,  &mut *di, action);
        }
        di = tv_dict_find(what, c"context", 7);
        if !di.is_null() {
            retval = qf_setprop_context(qfl, &mut *di);
        }
        di = tv_dict_find(what, c"idx", 3);
        if !di.is_null() {
            retval = qf_setprop_curidx(qi, qfl, &*di);
        }
        di = tv_dict_find(what, c"quickfixtextfunc", 16);
        if !di.is_null() {
            retval = qf_setprop_qftf(qfl, &mut *di);
        }

        if newlist || retval {
            qf_list_changed(qfl);
        }
        if newlist {
            qf_update_buffer(qi, 0);
        }

        retval
    }
}

fn qf_free_stack(mut wp: *mut win_T, qi: &mut qf_info_T) {
    unsafe {
        let qfwin = qf_find_win(&*qi);

        if !qfwin.is_null() {
            if qi.qf_curlist < qi.qf_listcount {
                qf_free(qf_get_curlist(&mut *qi));
            }
            qf_update_buffer(qi, 0);
        }

        if !wp.is_null() && is_ll_window(&*wp) {
            let llwin = qf_find_win_with_loclist(qi);
            if !llwin.is_null() {
                wp = llwin;
            }
        }

        qf_free_all(wp);
        if wp.is_null() {
            qi.qf_curlist = 0;
            qi.qf_listcount = 0;
        } else if !qfwin.is_null() {
            let new_ll = qf_alloc_stack(QFLT_LOCATION, (*wp).w_onebuf_opt.wo_lhi as c_int);
            (*new_ll).qf_bufnr = (*qfwin).buf_mut().b_fnum;

            ll_free_all(&raw mut (*qfwin).w_llist_ref);

            (*qfwin).w_llist_ref = new_ll;
            if wp != qfwin {
                win_set_loclist( &mut *wp, &mut *new_ll);
            }
        }
    }
}

pub fn set_errorlist(
    wp: *mut win_T,
    list: *mut list_T,
    action: c_int,
    title: *mut c_char,
    what: *mut dict_T,
) -> bool {
    unsafe {
        let qi = if !wp.is_null() {
            ll_get_or_alloc_list( &mut *wp)
        } else {
            ql_info_ptr()
        };
        assert!(!qi.is_null());

        if action == b'f' as c_int {
            qf_free_stack(wp, &mut *qi);
            return true;
        }

        if !list.is_null() && tv_list_len(list) != 0 && !what.is_null() {
            semsg!(
                gettext(e_invarg2),
                gettext(c"cannot have both a list and a \"what\" argument"),
            );
            return false;
        }

        incr_quickfix_busy();

        let retval = if !what.is_null() {
            qf_set_properties(&mut *qi, &*what, action, title)
        } else {
            let r = qf_add_entries(&mut *qi, (*qi).qf_curlist, list, title, action);
            if r == QF_OK {
                qf_list_changed(qf_get_curlist(&mut *qi));
            }
            r == QF_OK
        };

        decr_quickfix_busy();

        retval
    }
}

fn mark_quickfix_user_data(qi: &mut qf_info_T, copyID: c_int) -> bool {
    unsafe {
        let mut abort = false;
        let mut i: c_int = 0;
        while i < qi.qf_maxcount && !abort {
            let qfl = &mut (&mut qi.qf_lists)[i as usize];
            if qfl.qf_has_user_data {
                for qfp in &mut qfl.qf_items {
                    if g_st().got_int {
                        break;
                    }
                    let user_data = &raw mut qfp.qf_user_data;
                    if (*user_data).v_type != VarType::VAR_NUMBER
                        && (*user_data).v_type != VarType::VAR_STRING
                        && (*user_data).v_type != VarType::VAR_FLOAT
                    {
                        abort = abort
                            || set_ref_in_item(&mut *user_data, copyID, ptr::null_mut(), ptr::null_mut());
                    }
                }
            }
            i += 1;
        }
        abort
    }
}

fn mark_quickfix_ctx(qi: &mut qf_info_T, copyID: c_int) -> bool {
    unsafe {
        let mut abort = false;
        let mut i: c_int = 0;
        while i < qi.qf_maxcount && !abort {
            let qfl = &mut (&mut qi.qf_lists)[i as usize];
            if let Some(ref mut ctx) = qfl.qf_ctx
                && ctx.v_type != VarType::VAR_NUMBER
                && ctx.v_type != VarType::VAR_STRING
                && ctx.v_type != VarType::VAR_FLOAT
            {
                abort = set_ref_in_item(ctx, copyID, ptr::null_mut(), ptr::null_mut());
            }

            let cb = &raw mut qfl.qf_qftf_cb;
            abort = abort || set_ref_in_callback(&mut *cb, copyID, ptr::null_mut(), ptr::null_mut());
            i += 1;
        }
        abort
    }
}

pub fn set_ref_in_quickfix(copyID: c_int) -> bool {
    unsafe {
        if mark_quickfix_ctx(&mut *ql_info_ptr(), copyID)
            || mark_quickfix_user_data(&mut *ql_info_ptr(), copyID)
            || set_ref_in_callback(&mut opt_vars().p_qftf, copyID, ptr::null_mut(), ptr::null_mut())
        {
            return true;
        }

        for tp in for_all_tabpages_mut() {
            for win in win_iter_mut(tab_firstwin(tp)) {
                if !win.w_llist.is_null() {
                    let ll = win.w_llist;
                    if mark_quickfix_ctx(&mut *ll, copyID) || mark_quickfix_user_data(&mut *ll, copyID) {
                        return true;
                    }
                }

                if is_ll_window(win) {
                    let ll_ref = win.w_llist_ref;
                    if (*ll_ref).qf_refcount == 1
                        && (mark_quickfix_ctx(&mut *ll_ref, copyID)
                            || mark_quickfix_user_data(&mut *ll_ref, copyID))
                    {
                        return true;
                    }
                }
            }
        }

        false
    }
}

fn cbuffer_get_auname(cmdidx: c_int) -> *const c_char {
    match cmdidx {
        CMD_cbuffer => c"cbuffer".as_ptr(),
        CMD_cgetbuffer => c"cgetbuffer".as_ptr(),
        CMD_caddbuffer => c"caddbuffer".as_ptr(),
        CMD_lbuffer => c"lbuffer".as_ptr(),
        CMD_lgetbuffer => c"lgetbuffer".as_ptr(),
        CMD_laddbuffer => c"laddbuffer".as_ptr(),
        _ => ptr::null(),
    }
}

fn cbuffer_process_args(
    eap: &mut exarg_T,
    bufp: *mut *mut buf_T,
    line1: &mut linenr_T,
    line2: &mut linenr_T,
) -> bool {
    unsafe {
        let mut buf: *mut buf_T = ptr::null_mut();

        if *eap.arg == NUL {
            buf = g_st().curbuf;
        } else if *skipwhite(skipdigits(eap.arg)) == NUL {
            buf = buflist_findnr(crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(eap.arg)) as c_int);
        }

        if buf.is_null() {
            emsg(gettext(e_invarg));
            return false;
        }

        if (*buf).b_ml.ml_mfp.is_none() {
            emsg(gettext(e_buffer_is_not_loaded));
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
            emsg(gettext(e_invrange));
            return false;
        }

        *line1 = eap.line1;
        *line2 = eap.line2;
        *bufp = buf;

        true
    }
}

pub fn ex_cbuffer(eap: &mut exarg_T) {
    unsafe {
        let au_name = cbuffer_get_auname(eap.cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            )
            && aborting()
        {
            return;
        }

        let mut wp: *mut win_T = ptr::null_mut();
        let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);

        let mut buf: *mut buf_T = ptr::null_mut();
        let mut line1: linenr_T = 0;
        let mut line2: linenr_T = 0;
        if !cbuffer_process_args(eap, &raw mut buf, &mut line1, &mut line2) {
            return;
        }

        let mut qf_title = qf_cmdtitle(*eap.cmdlinep);
        if !(*buf).b_sfname.is_null() {
            let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                c"%s (%s)",
                qf_title,
                (*buf).b_sfname,
            );
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
            eap.cmdidx != CMD_caddbuffer && eap.cmdidx != CMD_laddbuffer,
            eap.line1,
            eap.line2,
            qf_title,
            ptr::null_mut(),
        );
        if qf_stack_empty(qi) {
            decr_quickfix_busy();
            return;
        }
        if res >= 0 {
            qf_list_changed(qf_get_curlist(&mut *qi));
        }
        let save_qfid = qf_get_curlist(&mut *qi).qf_id;
        if !au_name.is_null() {
            let curbuf_old = g_st().curbuf;
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            );
            if g_st().curbuf != curbuf_old {
                res = 0;
            }
        }
        if res > 0
            && (eap.cmdidx == CMD_cbuffer || eap.cmdidx == CMD_lbuffer)
            && qflist_valid(wp, save_qfid)
        {
            qf_jump_first(&mut *qi, save_qfid, eap.forceit);
        }

        decr_quickfix_busy();
    }
}

fn cexpr_get_auname(cmdidx: c_int) -> *const c_char {
    match cmdidx {
        CMD_cexpr => c"cexpr".as_ptr(),
        CMD_cgetexpr => c"cgetexpr".as_ptr(),
        CMD_caddexpr => c"caddexpr".as_ptr(),
        CMD_lexpr => c"lexpr".as_ptr(),
        CMD_lgetexpr => c"lgetexpr".as_ptr(),
        CMD_laddexpr => c"laddexpr".as_ptr(),
        _ => ptr::null(),
    }
}

fn trigger_cexpr_autocmd(cmdidx: c_int) -> bool {
     {
        let au_name = cexpr_get_auname(cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
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
}

pub fn cexpr_core(eap: &exarg_T, tv: &mut typval_T) -> bool {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();
        let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);

        if (tv.v_type == VarType::VAR_STRING && !tv.vval.v_string().is_null())
            || tv.v_type == VarType::VAR_LIST
        {
            let au_name = cexpr_get_auname(eap.cmdidx);
            incr_quickfix_busy();
            let res = qf_init_ext(
                &mut *qi,
                (*qi).qf_curlist,
                ptr::null(),
                ptr::null_mut(),
                Some(&mut *tv),
                opt_vars().p_efm,
                eap.cmdidx != CMD_caddexpr && eap.cmdidx != CMD_laddexpr,
                0,
                0,
                qf_cmdtitle(*eap.cmdlinep),
                ptr::null_mut(),
            );
            if qf_stack_empty(qi) {
                decr_quickfix_busy();
                return false;
            }
            if res >= 0 {
                qf_list_changed(qf_get_curlist(&mut *qi));
            }
            let save_qfid = qf_get_curlist(&mut *qi).qf_id;
            if !au_name.is_null() {
                apply_autocmds(
                    EVENT_QUICKFIXCMDPOST,
                    au_name.cast_mut(),
                    curbuf_mut().b_fname,
                    true,
                    g_st().curbuf,
                );
            }
            if res > 0
                && (eap.cmdidx == CMD_cexpr || eap.cmdidx == CMD_lexpr)
                && qflist_valid(wp, save_qfid)
            {
                qf_jump_first(&mut *qi, save_qfid, eap.forceit);
            }
            decr_quickfix_busy();
            return true;
        } else {
            emsg(gettext(c"E777: String or List expected"));
        }
        false
    }
}

pub fn ex_cexpr(eap: &mut exarg_T) {
    if !trigger_cexpr_autocmd(eap.cmdidx) {
            return;
        }

        let Some(mut tv) = eval_expr(eap.arg, Some(&mut *eap)) else {
            return;
        };

        let _ = cexpr_core(eap, &mut tv);
        tv_clear(&raw mut tv);
}

fn hgr_get_ll(new_ll: &mut bool) -> *mut qf_info_T {
    unsafe {
        let wp = if bt_help(curwin_mut().w_buffer) {
            curwin_ptr()
        } else {
            qf_find_help_win()
        };

        let mut qi = if wp.is_null() {
            ptr::null_mut()
        } else {
            (*wp).w_llist
        };
        if qi.is_null() {
            qi = qf_alloc_stack(QFLT_LOCATION, 1);
            *new_ll = true;
        }

        qi
    }
}

fn hgr_search_file(qfl: &mut qf_list_T, fname: *mut c_char, p_regmatch: &mut regmatch_T) {
    unsafe {
        let fd = os_fopen(fname, c"r").cast::<FILE>();
        if fd.is_null() {
            return;
        }

        let iobuff_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        let mut lnum: linenr_T = 1;
        while !vim_fgets(iobuff_ptr, IOSIZE as c_int, fd.cast()) && !g_st().got_int {
            let line = iobuff_ptr;

            if vim_regexec(p_regmatch, line, 0) {
                let mut l = crate::memory::cstr_len(line);
                while l > 0 && (*line.add(l - 1) as u8) <= b' ' {
                    l -= 1;
                    *line.add(l) = NUL;
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
                    (p_regmatch.startp[0].offset_from(line) as c_int) + 1,
                    (p_regmatch.endp[0].offset_from(line) as c_int) + 1,
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
        fclose(fd);
    }
}

fn hgr_search_files_in_dir(
    qfl: &mut qf_list_T,
    mut dirname: *mut c_char,
    p_regmatch: &mut regmatch_T,
    lang: *const c_char,
) {
    unsafe {
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
                    && !(crate::memory::cstr_prefix_bytes(lang, 2).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(f.add(flen - 3), 2)))
                    && !(crate::memory::cstr_prefix_bytes(lang, 2).eq_ignore_ascii_case(b"en")
                        && b"txt".eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(f.add(flen - 3), 3)))
                {
                    continue;
                }

                hgr_search_file(qfl, f, p_regmatch);
            }
        }
    }
}

fn hgr_search_in_rtp(qfl: &mut qf_list_T, p_regmatch: &mut regmatch_T, lang: *const c_char) {
    unsafe {
        let mut p = opt_vars().p_rtp;
        let name_buff_ptr = (&raw mut g_st().NameBuff).cast::<c_char>();
        while *p != NUL && !g_st().got_int {
            copy_option_part(&raw mut p, name_buff_ptr, MAXPATHL, c",");
            hgr_search_files_in_dir(qfl, name_buff_ptr, p_regmatch, lang);
        }
    }
}

pub fn ex_helpgrep(eap: &mut exarg_T) {
    unsafe {
        let mut qi = ql_info_ptr();
        assert!(!qi.is_null());
        let au_name: *const c_char = match eap.cmdidx {
            CMD_helpgrep => c"helpgrep".as_ptr(),
            CMD_lhelpgrep => c"lhelpgrep".as_ptr(),
            _ => ptr::null(),
        };
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
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
        opt_vars().p_cpo = empty_string_option.as_ptr().cast::<c_char>();

        let mut new_qi = false;
        if is_loclist_cmd(eap.cmdidx) {
            qi = hgr_get_ll(&mut new_qi);
        }

        incr_quickfix_busy();

        let lang = check_help_lang(eap.arg);
        let mut regmatch: regmatch_T = regmatch_T::new();
        regmatch.regprog = vim_regcomp(eap.arg, RE_MAGIC + RE_STRING);
        regmatch.rm_ic = false;
        if !regmatch.regprog.is_null() {
            qf_new_list(&mut *qi, qf_cmdtitle(*eap.cmdlinep));
            let qfl = qf_get_curlist(&mut *qi);

            hgr_search_in_rtp(qfl, &mut regmatch, lang);

            vim_regfree(regmatch.regprog);

            qfl.qf_nonevalid = false;
            qfl.qf_index = 1;
            qf_list_changed(qfl);
            updated = true;
        }

        if opt_vars().p_cpo == empty_string_option.as_ptr().cast::<c_char>() {
            opt_vars().p_cpo = save_cpo;
        } else {
            if crate::memory::cstr_is_empty(opt_vars().p_cpo) {
                set_option_value_give_err(kOptCpoptions, cstr_as_obj(save_cpo), 0);
            }
            free_string_option(save_cpo);
        }

        if updated {
            qf_update_buffer(&mut *qi, 0);
        }

        if !au_name.is_null() {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.cast_mut(),
                curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            );
            if !new_qi && is_ll_stack(&*qi) && qf_find_win_with_loclist(&*qi).is_null() {
                decr_quickfix_busy();
                return;
            }
        }

        if !qf_list_empty(qf_get_curlist(&mut *qi)) {
            qf_jump(qi, 0, 0, 0);
        } else {
            semsg!(gettext(e_nomatch2), eap.arg);
        }

        decr_quickfix_busy();

        if eap.cmdidx == CMD_lhelpgrep {
            if !bt_help(curwin_mut().w_buffer) || curwin_mut().w_llist == qi {
                if new_qi {
                    ll_free_all(&raw mut qi);
                }
            } else if curwin_mut().w_llist.is_null() && new_qi {
                curwin_mut().w_llist = qi;
            }
        }
    }
}

pub fn free_quickfix() {
    unsafe {
        qf_free_all(ptr::null_mut());
        for tab in for_all_tabpages_mut() {
            for win in win_iter_ptr(tab_firstwin(tab)) {
                qf_free_all(win);
            }
        }

        callback_free(&mut opt_vars().p_qftf);
        let gap = &raw mut qf_st().qfga;
        *gap = Vec::new();
    }
}

fn get_qf_loc_list(
    is_qf: bool,
    wp: *mut win_T,
    what_arg: &mut typval_T,
    rettv: &mut typval_T,
) {
    if what_arg.v_type == VarType::VAR_UNKNOWN {
            tv_list_alloc_ret(&mut *rettv, kListLenMayKnow);
            if is_qf || !wp.is_null() {
                get_errorlist(None, wp, -1, 0, unsafe { &mut *rettv.vval.v_list() });
            }
        } else {
            tv_dict_alloc_ret(rettv);
            if is_qf || !wp.is_null() {
                if what_arg.v_type == VarType::VAR_DICT {
                    let d = what_arg.vval.v_dict();
                    if !d.is_null() {
                        unsafe {
                            qf_get_properties(wp, &mut *d, &mut *rettv.vval.v_dict());
                        }
                    }
                } else {
                    emsg(gettext(e_dictreq));
                }
            }
        }
}

pub fn f_getloclist(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    let wp = find_win_by_nr_or_id(&argvars[0]);
    get_qf_loc_list(false, wp, &mut argvars[1], rettv);
}

pub fn f_getqflist(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    get_qf_loc_list(true, ptr::null_mut(), &mut argvars[0], rettv);
}

fn set_qf_ll_list(wp: *mut win_T,args: &mut [typval_T], rettv: &mut typval_T) {
    let args: *mut typval_T = args.as_mut_ptr();
    unsafe {
        let mut title: *const c_char = ptr::null();
        let mut action: c_char = b' ' as c_char;
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        let mut what: *mut dict_T = ptr::null_mut();

        let list_arg = args;
        if (*list_arg).v_type != VarType::VAR_LIST {
            emsg(gettext(e_listreq));
            return;
        } else if qf_st().set_qf_ll_recursive != 0 {
            emsg(gettext(e_au_recursive));
            return;
        }

        let action_arg = args.add(1);
        if (*action_arg).v_type != VarType::VAR_UNKNOWN {
            if (*action_arg).v_type != VarType::VAR_STRING {
                emsg(gettext(e_string_required));
                return;
            }
            let act = tv_get_string_chk(&*action_arg);
            if (*act == b'a' as c_char
                || *act == b'r' as c_char
                || *act == b'u' as c_char
                || *act == b' ' as c_char
                || *act == b'f' as c_char)
                && *act.add(1) == NUL
            {
                action = *act;
            } else {
                semsg!(gettext(E_INVACT), act);
                return;
            }

            let what_arg = args.add(2);
            if (*what_arg).v_type != VarType::VAR_UNKNOWN {
                if (*what_arg).v_type == VarType::VAR_STRING {
                    title = tv_get_string_chk(&*what_arg);
                    if title.is_null() {
                        return;
                    }
                } else if (*what_arg).v_type == VarType::VAR_DICT
                    && !(*what_arg).vval.v_dict().is_null()
                {
                    what = (*what_arg).vval.v_dict();
                } else {
                    emsg(gettext(e_dictreq));
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
        if set_errorlist(wp, l, c_int::from(action), title.cast_mut(), what) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
        qf_st().set_qf_ll_recursive -= 1;
    }
}

pub fn f_setloclist(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    let win = find_win_by_nr_or_id(&argvars[0]);
    if !win.is_null() {
        set_qf_ll_list(win, &mut argvars[1..], rettv);
    }
}

pub fn f_setqflist(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
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
        let mut qfp = qfline_T::new();
            qfp.qf_lnum = 10;
            qfp.qf_col = 5;
            let pos = pos_T {
                lnum: 10,
                col: 5,
                coladd: 0,
            };
            assert!(!qf_entry_after_pos(&qfp, &pos, true));
            assert!(!qf_entry_after_pos(&qfp, &pos, false));
            assert!(qf_entry_on_or_after_pos(
                &qfp,
                &pos,
                true
            ));
            assert!(qf_entry_on_or_after_pos(
                &qfp,
                &pos,
                false
            ));
            assert!(qf_entry_on_or_before_pos(
                &qfp,
                &pos,
                true
            ));
            assert!(qf_entry_on_or_before_pos(
                &qfp,
                &pos,
                false
            ));
    }

    #[test]
    fn entry_is_closer_to_target_compares_file_line_and_column() {
        let mut e1 = qfline_T::new();
            let mut e2 = qfline_T::new();
            e1.qf_fnum = 1;
            e1.qf_lnum = 12;
            e1.qf_col = 4;
            e2.qf_fnum = 1;
            e2.qf_lnum = 15;
            e2.qf_col = 2;

            assert!(entry_is_closer_to_target(
                &e1,
                &e2,
                1,
                10,
                1
            ));
            assert!(!entry_is_closer_to_target(
                &e2,
                &e1,
                1,
                10,
                1
            ));

            e2.qf_lnum = 12;
            assert!(entry_is_closer_to_target(
                &e2,
                &e1,
                1,
                12,
                1
            ));
    }
}
