//! Quickfix and location list commands and functions (`src/nvim/quickfix.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::command::{CMOD_HIDE, cmdidx_T, exarg_T};
use crate::api::options::{
    BF_DUMMY, CtxSwitch, IOSIZE, kOptBufhidden, kOptBuftype, kOptFiletype, kOptSwapfile,
};
use crate::api::private::helpers::{
    boolean_obj, cbuf_as_string, copy_string, cstr_as_obj, cstr_as_string,
};
use crate::api::tabpage::{tabpage_T, win_T};
use crate::arglist::{ML_EMPTY, get_arglist_exp};
use crate::autocmd::{
    EVENT_BUFREADPOST, EVENT_BUFWINENTER, EVENT_FILETYPE, EVENT_QUICKFIXCMDPOST,
    EVENT_QUICKFIXCMDPRE, OPT_NOWIN, apply_autocmds, au_event_disable, au_event_restore,
    block_autocmds, unblock_autocmds,
};
use crate::buffer::{
    BF_NEW, BLN_DUMMY, BLN_NOOPT, DOBUF_UNLOAD, DOBUF_WIPE, GETF_SETMARK, GETF_SWITCH, MAXLNUM,
    NOTDONE, VAR_FIXED, bt_help, bt_normal, bt_quickfix, buf_valid, buflist_findname_exp,
    buflist_findnr, buflist_getfile, buflist_new, bufref_T, bufref_valid, close_buffer,
    do_modelines, kExtmarkNoUndo, no_write_message, set_bufref, setfname, wipe_buffer,
};
use crate::change::{TAB, changed_lines};
use crate::charset::{ascii_iswhite, skipdigits, skipwhite, vim_isprintc};
use crate::clipboard::{VarType, list_T, listitem_T, typval_T};
use crate::context::{ctx_restore, ctx_switch};
use crate::cursor::{check_cursor, colnr_T, get_cursor_line_ptr, linenr_T};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, redraw_buf_later, redraw_curbuf_later, redraw_later, update_screen,
};
use crate::errors::*;
use crate::eval::buffer::varnumber_T;
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::{Callback, EvalFuncData, buf_T, pos_T};
use crate::eval::fs::FileInfo;
use crate::eval::typval::{
    callback_copy, callback_free, callback_put, tv_clear, tv_copy, tv_dict_add, tv_dict_add_list,
    tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len, tv_dict_add_tv, tv_dict_alloc,
    tv_dict_alloc_lock, tv_dict_alloc_ret, tv_dict_find, tv_dict_get_bool, tv_dict_get_number,
    tv_dict_get_string, tv_dict_get_tv, tv_dict_item_alloc_len, tv_dict_item_free, tv_dict_unref,
    tv_free, tv_get_number_chk, tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_dict, tv_list_first, tv_list_len, tv_list_ref, tv_list_unref,
};
use crate::eval::vars::{OptIndex, set_internal_string_var};
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
use crate::ex_docmd::{do_cmdline_cmd, ex_cd, is_loclist_cmd};
use crate::ex_eval::{aborting, cleanup_T, enter_cleanup, leave_cleanup};
use crate::ex_getln::get_list_range;
use crate::extmark::extmark_splice;
#[cfg(windows)]
use crate::fileio::_fdopen;
#[cfg(not(windows))]
use crate::fileio::fdopen;
use crate::fileio::{
    EINTR, READ_DUMMY, READ_NEW, fclose, ferror, fgets, readfile, shorten_buf_fname,
    shorten_fnames, vim_fgets, vim_tempname,
};
use crate::fold::{foldOpenCursor, foldUpdateAll};
use crate::fuzzy::{FUZZY_MATCH_MAX_LEN, fuzzy_match};
use crate::garray::{ga_append, ga_clear, ga_concat, ga_concat_len, ga_init, garray_T};
use crate::globals::*;
use crate::help::check_help_lang;
use crate::highlight_group::syn_name2id;
use crate::insert::beginline;
use crate::lua::stdlib::regmatch_T;
use crate::map_glyph_cache::NvimString;
use crate::mark::{BUF_HAS_LL_ENTRY, BUF_HAS_QF_ENTRY, setpcmark};
use crate::r#match::{regmmatch_T, regprog_T};
use crate::math::{FAIL, OK};
use crate::mbyte::{convert_setup, ptr2cells, remove_bom, string_convert, utfc_ptr2len, vimconv_T};
use crate::memfile::memfile_T;
use crate::memline::{
    check_need_swap, ml_append_buf, ml_delete, ml_get_buf, ml_get_buf_len, ml_open,
};
use crate::memory::{
    strequal, xcalloc, xfree, xmalloc, xmallocz, xmemdupz, xrealloc, xstrdup, xstrlcat, xstrlcpy,
};
use crate::message::{
    emsg, internal_error, message_filtered, msg, msg_clr_eos, msg_ext_set_kind, msg_keep,
    msg_outtrans, msg_prt_line, msg_putchar, msg_puts, msg_puts_hl, msg_start, msg_strtrunc, semsg,
    smsg, trunc_string,
};
use crate::r#move::update_topline;
use crate::msgpack_rpc::packer::handle_T;
use crate::normal::reset_VIsual_and_resel;
use crate::ops::get_region_bytecount;
use crate::option::{
    buf_copy_options, copy_option_part, set_option_direct, set_option_value_give_err, shortmess,
    skip_to_option_part,
};
use crate::option_vars::*;
use crate::optionstr::free_string_option;
use crate::os::env::{expand_env, os_get_pid};
use crate::os::fs::os_errno as get_errno;
use crate::os::fs::os_set_errno as set_errno;
use crate::os::fs::{
    os_dirname, os_fileinfo_link, os_fopen, os_isdir, os_open_stdin_fd, os_path_exists, os_remove,
};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::os::time::c_time as time;
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{
    EW_FILE, EW_SILENT, FreeWild, add_pathsep, concat_fnames, fix_fname, gen_expand_wildcards,
    kPathCmpLiteral, path_equal, path_is_absolute, path_tail, path_try_shorten_fname,
    vim_isAbsName,
};
use crate::regexp::{
    RE_MAGIC, RE_STRING, vim_regcomp, vim_regexec, vim_regexec_multi, vim_regfree,
};
use crate::search::{do_search, last_search_pat};
use crate::strings::{
    atoi, atol, gettext, has_non_ascii, strcmp as c_strcmp, strlen as c_strlen, vim_snprintf,
    vim_snprintf_safelen, vim_strchr,
};
use crate::ui::ui_flush;
use crate::undo::u_clearallandblockfree;
use crate::window::reset_binding;
use crate::window::{
    WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HELP, WSP_NEWLOC, WSP_QUICKFIX, WSP_TOP, WSP_VERT,
    check_can_set_curbuf_forceit, check_lnums, goto_tabpage_win, tabline_height, win_close,
    win_enter, win_goto, win_setheight, win_setwidth, win_split, win_valid,
};
use std::ffi::{CStr, c_char, c_int, c_longlong, c_uint, c_void};
use std::ptr;

pub use crate::api::autocmd::event_T;
use crate::api::autocmd::kCallbackNone;
pub use crate::api::buffer::bcount_t;
use crate::api::buffer::{FORWARD, NL, NUL};
use crate::api::deprecated::OPT_LOCAL;
use crate::api::vim::{BACKWARD, BCO_ENTER, BCO_NOHELP, HLF_N, MAXPATHL};
use crate::cmdexpand::HLF_D;
use crate::diff::kOptFoldmethod;
use crate::drawline::HLF_QFL;
use crate::eval::funcs::{SEARCH_KEEP, kOptCpoptions};
pub use crate::eval::vars::scid_T;
pub use crate::extmark::ExtmarkOp;
pub use crate::fileio::FILE;
pub use crate::os::time::time_t;
pub use crate::search::Direction;
use crate::viml::parser::parser::CONV_NONE;

#[cfg(windows)]
const CAR: c_char = 0x0d;

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

const CMD_cNext: cmdidx_T = 46;
const CMD_cNfile: cmdidx_T = 47;
const CMD_cabove: cmdidx_T = 50;
const CMD_caddbuffer: cmdidx_T = 51;
const CMD_caddexpr: cmdidx_T = 52;
const CMD_caddfile: cmdidx_T = 53;
const CMD_cafter: cmdidx_T = 54;
const CMD_cbuffer: cmdidx_T = 57;
const CMD_cbefore: cmdidx_T = 58;
const CMD_cbelow: cmdidx_T = 59;
const CMD_cc: cmdidx_T = 61;
const CMD_cd: cmdidx_T = 63;
const CMD_cexpr: cmdidx_T = 66;
const CMD_cfile: cmdidx_T = 67;
const CMD_cfirst: cmdidx_T = 69;
const CMD_cgetfile: cmdidx_T = 70;
const CMD_cgetbuffer: cmdidx_T = 71;
const CMD_cgetexpr: cmdidx_T = 72;
const CMD_cnfile: cmdidx_T = 88;
const CMD_colder: cmdidx_T = 93;
const CMD_cprevious: cmdidx_T = 103;
const CMD_cpfile: cmdidx_T = 104;
const CMD_crewind: cmdidx_T = 106;
const CMD_grep: cmdidx_T = 175;
const CMD_grepadd: cmdidx_T = 176;
const CMD_helpgrep: cmdidx_T = 181;
const CMD_lNext: cmdidx_T = 214;
const CMD_lNfile: cmdidx_T = 215;
const CMD_labove: cmdidx_T = 217;
const CMD_laddexpr: cmdidx_T = 219;
const CMD_laddbuffer: cmdidx_T = 220;
const CMD_laddfile: cmdidx_T = 221;
const CMD_lafter: cmdidx_T = 222;
const CMD_lbuffer: cmdidx_T = 224;
const CMD_lbelow: cmdidx_T = 226;
const CMD_lcd: cmdidx_T = 228;
const CMD_lexpr: cmdidx_T = 235;
const CMD_lfile: cmdidx_T = 236;
const CMD_lfirst: cmdidx_T = 238;
const CMD_lgetfile: cmdidx_T = 239;
const CMD_lgetbuffer: cmdidx_T = 240;
const CMD_lgetexpr: cmdidx_T = 241;
const CMD_lgrep: cmdidx_T = 242;
const CMD_lgrepadd: cmdidx_T = 243;
const CMD_lhelpgrep: cmdidx_T = 244;
const CMD_ll: cmdidx_T = 246;
const CMD_lmake: cmdidx_T = 251;
const CMD_lnfile: cmdidx_T = 255;
const CMD_lolder: cmdidx_T = 261;
const CMD_lprevious: cmdidx_T = 263;
const CMD_lpfile: cmdidx_T = 264;
const CMD_lrewind: cmdidx_T = 265;
const CMD_lvimgrep: cmdidx_T = 271;
const CMD_lvimgrepadd: cmdidx_T = 272;
const CMD_make: cmdidx_T = 278;
const CMD_vimgrep: cmdidx_T = 517;
const CMD_vimgrepadd: cmdidx_T = 518;

const E_NO_MORE_ITEMS: &CStr = c"E553: No more items";
const E_CURRENT_QUICKFIX_LIST_WAS_CHANGED: &CStr = c"E925: Current quickfix list was changed";
const E_CURRENT_LOCATION_LIST_WAS_CHANGED: &CStr = c"E926: Current location list was changed";
const E_INVACT: &CStr = c"E927: Invalid action: '%s'";

#[repr(C)]
pub struct dir_stack_T {
    pub next: *mut dir_stack_T,
    pub dirname: *mut c_char,
}

#[repr(C)]
pub struct qfline_T {
    pub qf_next: *mut qfline_T,
    pub qf_prev: *mut qfline_T,
    pub qf_lnum: linenr_T,
    pub qf_end_lnum: linenr_T,
    pub qf_fnum: c_int,
    pub qf_col: c_int,
    pub qf_end_col: c_int,
    pub qf_nr: c_int,
    pub qf_module: *mut c_char,
    pub qf_fname: *mut c_char,
    pub qf_pattern: *mut c_char,
    pub qf_text: *mut c_char,
    pub qf_viscol: c_char,
    pub qf_cleared: c_char,
    pub qf_type: c_char,
    pub qf_user_data: typval_T,
    pub qf_valid: c_char,
}

pub type qfltype_T = c_int;
pub const QFLT_QUICKFIX: qfltype_T = 0;
pub const QFLT_LOCATION: qfltype_T = 1;
pub const QFLT_INTERNAL: qfltype_T = 2;

#[repr(C)]
pub struct qf_list_T {
    pub qf_id: c_uint,
    pub qfl_type: qfltype_T,
    pub qf_start: *mut qfline_T,
    pub qf_last: *mut qfline_T,
    pub qf_ptr: *mut qfline_T,
    pub qf_count: c_int,
    pub qf_index: c_int,
    pub qf_nonevalid: bool,
    pub qf_has_user_data: bool,
    pub qf_title: *mut c_char,
    pub qf_ctx: *mut typval_T,
    pub qf_qftf_cb: Callback,
    pub qf_dir_stack: *mut dir_stack_T,
    pub qf_directory: *mut c_char,
    pub qf_file_stack: *mut dir_stack_T,
    pub qf_currfile: *mut c_char,
    pub qf_multiline: bool,
    pub qf_multiignore: bool,
    pub qf_multiscan: bool,
    pub qf_changedtick: c_int,
}

#[repr(C)]
pub struct qf_info_T {
    pub qf_refcount: c_int,
    pub qf_listcount: c_int,
    pub qf_curlist: c_int,
    pub qf_maxcount: c_int,
    pub qf_lists: *mut qf_list_T,
    pub qfl_type: qfltype_T,
    pub qf_bufnr: c_int,
}

#[repr(C)]
struct efm_T {
    prog: *mut regprog_T,
    next: *mut efm_T,
    addr: [c_char; FMT_PATTERNS],
    prefix: c_char,
    flags: c_char,
    conthere: c_int,
}

#[repr(C)]
struct qf_delq_T {
    next: *mut qf_delq_T,
    qi: *mut qf_info_T,
}

#[repr(C)]
struct qfstate_T {
    linebuf: *mut c_char,
    linelen: usize,
    growbuf: *mut c_char,
    growbufsiz: usize,
    fd: *mut FILE,
    tv: *mut typval_T,
    p_str: *mut c_char,
    p_list: *mut list_T,
    p_li: *mut listitem_T,
    buf: *mut buf_T,
    buflnum: linenr_T,
    lnumlast: linenr_T,
    vc: vimconv_T,
}

#[repr(C)]
struct qffields_T {
    namebuf: *mut c_char,
    bnr: c_int,
    module: *mut c_char,
    errmsg: *mut c_char,
    errmsglen: usize,
    lnum: linenr_T,
    end_lnum: linenr_T,
    col: c_int,
    end_col: c_int,
    use_viscol: bool,
    pattern: *mut c_char,
    enr: c_int,
    type_: c_char,
    user_data: *mut typval_T,
    valid: bool,
}

#[repr(C)]
struct vgr_args_T {
    tomatch: c_int,
    spat: *mut c_char,
    flags: c_int,
    fnames: *mut *mut c_char,
    fcount: c_int,
    regmatch: regmmatch_T,
    qf_title: *mut c_char,
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

static mut ql_info_actual: qf_info_T = qf_info_T {
    qf_refcount: 0,
    qf_listcount: 0,
    qf_curlist: 0,
    qf_maxcount: 0,
    qf_lists: ptr::null_mut(),
    qfl_type: QFLT_QUICKFIX,
    qf_bufnr: 0,
};
static mut ql_info: *mut qf_info_T = ptr::null_mut();
static mut last_qf_id: c_uint = 0;

static mut qf_last_bufname: *mut c_char = ptr::null_mut();
static mut qf_last_bufref: bufref_T = bufref_T {
    br_buf: ptr::null_mut(),
    br_fnum: 0,
    br_buf_free_count: 0,
};

static mut qfga: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};
static mut qfga_initialized: bool = false;

static mut quickfix_busy: c_int = 0;
static mut qf_delq_head: *mut qf_delq_T = ptr::null_mut();
static mut fmt_start: *mut efm_T = ptr::null_mut();

static mut fmt_first_cache: *mut efm_T = ptr::null_mut();
static mut last_efm_cache: *mut c_char = ptr::null_mut();
static mut qftitle_str: [c_char; IOSIZE] = [0; IOSIZE];
static mut qfFile_hl_id: c_int = 0;
static mut qfSep_hl_id: c_int = 0;
static mut qfLine_hl_id: c_int = 0;
static mut qf_types_cc: [c_char; 3] = [0; 3];
static mut qf_types_buf: [c_char; 20] = [0; 20];
static mut qftf_recursive: bool = false;
static mut mef_start: c_int = -1;
static mut mef_off: c_int = 0;
static mut did_bufnr_emsg: bool = false;
static mut set_qf_ll_recursive: c_int = 0;

#[inline]
unsafe fn c_fdopen(fd: c_int, mode: *const c_char) -> *mut FILE {
    #[cfg(windows)]
    unsafe {
        _fdopen(fd, mode)
    }
    #[cfg(not(windows))]
    unsafe {
        fdopen(fd, mode)
    }
}

#[inline]
unsafe fn strnicmp_asc(mut s1: *const c_char, mut s2: *const c_char, mut n: usize) -> c_int {
    unsafe {
        while n > 0 {
            let c1 = (*s1 as u8).to_ascii_lowercase();
            let c2 = (*s2 as u8).to_ascii_lowercase();
            if c1 != c2 {
                return c_int::from(c1) - c_int::from(c2);
            }
            if c1 == 0 {
                return 0;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            n -= 1;
        }
        0
    }
}

#[inline]
unsafe fn c_strcpy(dst: *mut c_char, src: *const c_char) {
    unsafe {
        let len = c_strlen(src);
        ptr::copy_nonoverlapping(src, dst, len + 1);
    }
}

#[inline]
unsafe fn c_strcat(dst: *mut c_char, src: *const c_char) {
    unsafe {
        let dlen = c_strlen(dst);
        let slen = c_strlen(src);
        ptr::copy_nonoverlapping(src, dst.add(dlen), slen + 1);
    }
}

#[inline]
unsafe fn is_qf_window(wp: *const win_T) -> bool {
    unsafe { bt_quickfix((*wp).w_buffer) && (*wp).w_llist_ref.is_null() }
}

#[inline]
unsafe fn is_ll_window(wp: *const win_T) -> bool {
    unsafe { bt_quickfix((*wp).w_buffer) && !(*wp).w_llist_ref.is_null() }
}

#[inline]
unsafe fn is_qf_stack(qi: *const qf_info_T) -> bool {
    unsafe { (*qi).qfl_type == QFLT_QUICKFIX }
}

#[inline]
unsafe fn is_ll_stack(qi: *const qf_info_T) -> bool {
    unsafe { (*qi).qfl_type == QFLT_LOCATION }
}

#[inline]
unsafe fn is_qf_list(qfl: *const qf_list_T) -> bool {
    unsafe { (*qfl).qfl_type == QFLT_QUICKFIX }
}

#[inline]
unsafe fn get_loc_list(wp: *const win_T) -> *mut qf_info_T {
    unsafe {
        if is_ll_window(wp) {
            (*wp).w_llist_ref.cast::<qf_info_T>()
        } else {
            (*wp).w_llist.cast::<qf_info_T>()
        }
    }
}

#[inline]
unsafe fn tab_firstwin(tp: *const tabpage_T) -> *mut win_T {
    unsafe {
        if tp == curtab.cast_const() {
            firstwin
        } else {
            (*tp).tp_firstwin
        }
    }
}

unsafe fn qfga_get() -> *mut garray_T {
    unsafe {
        if !qfga_initialized {
            qfga_initialized = true;
            ga_init(&raw mut qfga, 1, 256);
        }
        qfga.ga_len = 0;
        &raw mut qfga
    }
}

unsafe fn qfga_clear() {
    unsafe {
        if qfga.ga_maxlen > 1000 {
            ga_clear(&raw mut qfga);
        } else {
            qfga.ga_len = 0;
        }
    }
}

unsafe fn qf_init_process_nextline(
    qfl: *mut qf_list_T,
    fmt_first: *mut efm_T,
    state: *mut qfstate_T,
    fields: *mut qffields_T,
) -> c_int {
    unsafe {
        let mut status = qf_get_nextline(state);
        if status != QF_OK {
            return status;
        }

        status = qf_parse_line(qfl, (*state).linebuf, (*state).linelen, fmt_first, fields);
        if status != QF_OK {
            return status;
        }

        let fname = if *(*fields).namebuf != NUL || !(*qfl).qf_directory.is_null() {
            (*fields).namebuf
        } else if !(*qfl).qf_currfile.is_null() && (*fields).valid {
            (*qfl).qf_currfile
        } else {
            ptr::null_mut()
        };

        qf_add_entry(
            qfl,
            (*qfl).qf_directory,
            fname,
            (*fields).module,
            (*fields).bnr,
            (*fields).errmsg,
            (*fields).lnum,
            (*fields).end_lnum,
            (*fields).col,
            (*fields).end_col,
            c_char::from((*fields).use_viscol),
            (*fields).pattern,
            (*fields).enr,
            (*fields).type_,
            (*fields).user_data,
            c_char::from((*fields).valid),
        )
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_init(
    wp: *mut win_T,
    efile: *const c_char,
    errorformat: *mut c_char,
    newlist: c_int,
    qf_title: *const c_char,
    enc: *mut c_char,
) -> c_int {
    unsafe {
        let qi = if wp.is_null() {
            ql_info
        } else {
            ll_get_or_alloc_list(wp)
        };
        assert!(!qi.is_null());

        qf_init_ext(
            qi,
            (*qi).qf_curlist,
            efile,
            curbuf,
            ptr::null_mut(),
            errorformat,
            newlist != 0,
            0,
            0,
            qf_title,
            enc,
        )
    }
}

unsafe fn efmpat_to_regpat(
    efmpat: *const c_char,
    mut regpat: *mut c_char,
    efminfo: *mut efm_T,
    idx: usize,
    mut round: c_int,
) -> *mut c_char {
    unsafe {
        if (*efminfo).addr[idx] != 0 {
            semsg!(
                gettext(c"E372: Too many %%%c in format string".as_ptr()),
                c_int::from(*efmpat),
            );
            return ptr::null_mut();
        }
        if (idx != 0
            && idx < FMT_PATTERN_R
            && !vim_strchr(c"DXOPQ".as_ptr(), c_int::from((*efminfo).prefix as u8)).is_null())
            || (idx == FMT_PATTERN_R
                && vim_strchr(c"OPQ".as_ptr(), c_int::from((*efminfo).prefix as u8)).is_null())
        {
            semsg!(
                gettext(c"E373: Unexpected %%%c in format string".as_ptr()),
                c_int::from(*efmpat),
            );
            return ptr::null_mut();
        }
        round += 1;
        (*efminfo).addr[idx] = round as c_char;
        *regpat = b'\\' as c_char;
        regpat = regpat.add(1);
        *regpat = b'(' as c_char;
        regpat = regpat.add(1);
        #[cfg(windows)]
        if *efmpat == b'f' as c_char {
            c_strcpy(regpat, c"\\%(\\a:\\)\\=".as_ptr());
            regpat = regpat.add(10);
        }
        if *efmpat == b'f' as c_char && *efmpat.add(1) != NUL {
            if *efmpat.add(1) != b'\\' as c_char && *efmpat.add(1) != b'%' as c_char {
                c_strcpy(regpat, c".\\{-1,}".as_ptr());
                regpat = regpat.add(7);
            } else {
                c_strcpy(regpat, c"\\f\\+".as_ptr());
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

unsafe fn scanf_fmt_to_regpat(
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
                        emsg(gettext(c"E374: Missing ] in format string".as_ptr()));
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
                gettext(c"E375: Unsupported %%%c in format string".as_ptr()),
                c_int::from(*efmp),
            );
            return ptr::null_mut();
        }

        *pefmp = efmp;
        regpat
    }
}

unsafe fn efm_analyze_prefix(mut efmp: *const c_char, efminfo: *mut efm_T) -> *const c_char {
    unsafe {
        if !vim_strchr(c"+-".as_ptr(), c_int::from(*efmp as u8)).is_null() {
            (*efminfo).flags = *efmp;
            efmp = efmp.add(1);
        }
        if !vim_strchr(c"DXAEWINCZGOPQ".as_ptr(), c_int::from(*efmp as u8)).is_null() {
            (*efminfo).prefix = *efmp;
        } else {
            semsg!(
                gettext(c"E376: Invalid %%%c in format string prefix".as_ptr()),
                c_int::from(*efmp),
            );
            return ptr::null();
        }
        efmp
    }
}

unsafe fn efm_to_regpat(
    efm: *const c_char,
    len: c_int,
    fmt_ptr: *mut efm_T,
    regpat: *mut c_char,
) -> c_int {
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
                        return FAIL;
                    }
                    round += 1;
                } else if *efmp == b'*' as c_char {
                    efmp = efmp.add(1);
                    ptr = scanf_fmt_to_regpat(&raw mut efmp, efm, len, ptr);
                    if ptr.is_null() {
                        return FAIL;
                    }
                } else if !vim_strchr(c"%\\.^$~[".as_ptr(), c_int::from(*efmp as u8)).is_null() {
                    *ptr = *efmp;
                    ptr = ptr.add(1);
                } else if *efmp == b'#' as c_char {
                    *ptr = b'*' as c_char;
                    ptr = ptr.add(1);
                } else if *efmp == b'>' as c_char {
                    (*fmt_ptr).conthere = 1;
                } else if efmp == efm.add(1) {
                    efmp = efm_analyze_prefix(efmp, fmt_ptr);
                    if efmp.is_null() {
                        return FAIL;
                    }
                } else {
                    semsg!(
                        gettext(c"E377: Invalid %%%c in format string".as_ptr()),
                        c_int::from(*efmp),
                    );
                    return FAIL;
                }
            } else {
                if *efmp == b'\\' as c_char && efmp.add(1) < end {
                    efmp = efmp.add(1);
                } else if !vim_strchr(c".*^$~[".as_ptr(), c_int::from(*efmp as u8)).is_null() {
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

        OK
    }
}

unsafe fn free_efm_list(efm_first: *mut *mut efm_T) {
    unsafe {
        let mut efm_ptr = *efm_first;
        while !efm_ptr.is_null() {
            *efm_first = (*efm_ptr).next;
            vim_regfree((*efm_ptr).prog);
            xfree(efm_ptr.cast());
            efm_ptr = *efm_first;
        }
        fmt_start = ptr::null_mut();
    }
}

unsafe fn efm_regpat_bufsz(efm: *const c_char) -> usize {
    unsafe {
        let mut sz = (FMT_PATTERNS * 3) + (c_strlen(efm) << 2);
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

unsafe fn efm_option_part_len(efm: *const c_char) -> c_int {
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

unsafe fn parse_efm_option(mut efm: *mut c_char) -> *mut efm_T {
    unsafe {
        let mut fmt_first: *mut efm_T = ptr::null_mut();
        let mut fmt_last: *mut efm_T = ptr::null_mut();

        let sz = efm_regpat_bufsz(efm);
        let fmtstr = xmalloc(sz).cast::<c_char>();

        let mut errored = false;
        while *efm != NUL {
            let fmt_ptr = xcalloc(1, size_of::<efm_T>()).cast::<efm_T>();
            if fmt_first.is_null() {
                fmt_first = fmt_ptr;
            } else {
                (*fmt_last).next = fmt_ptr;
            }
            fmt_last = fmt_ptr;

            let len = efm_option_part_len(efm);

            if efm_to_regpat(efm, len, fmt_ptr, fmtstr) == FAIL {
                errored = true;
                break;
            }
            (*fmt_ptr).prog = vim_regcomp(fmtstr, RE_MAGIC + RE_STRING);
            if (*fmt_ptr).prog.is_null() {
                errored = true;
                break;
            }
            efm = skip_to_option_part(efm.add(len as usize));
        }

        if errored {
            free_efm_list(&raw mut fmt_first);
        } else if fmt_first.is_null() {
            emsg(gettext(c"E378: 'errorformat' contains no pattern".as_ptr()));
        }

        xfree(fmtstr.cast());
        fmt_first
    }
}

unsafe fn qf_grow_linebuf(state: *mut qfstate_T, newsz: usize) -> *mut c_char {
    unsafe {
        (*state).linelen = if newsz > LINE_MAXLEN {
            LINE_MAXLEN - 1
        } else {
            newsz
        };
        if (*state).growbuf.is_null() {
            (*state).growbuf = xmalloc((*state).linelen + 1).cast::<c_char>();
            (*state).growbufsiz = (*state).linelen;
        } else if (*state).linelen > (*state).growbufsiz {
            (*state).growbuf =
                xrealloc((*state).growbuf.cast(), (*state).linelen + 1).cast::<c_char>();
            (*state).growbufsiz = (*state).linelen;
        }
        (*state).growbuf
    }
}

unsafe fn qf_get_next_str_line(state: *mut qfstate_T) -> c_int {
    unsafe {
        let mut p_str = (*state).p_str;
        if *p_str == NUL {
            return QF_END_OF_INPUT;
        }

        let p = vim_strchr(p_str, b'\n' as c_int);
        let len = if !p.is_null() {
            p.offset_from(p_str) as usize + 1
        } else {
            c_strlen(p_str)
        };

        if len > IOSIZE - 2 {
            (*state).linebuf = qf_grow_linebuf(state, len);
        } else {
            (*state).linebuf = (&raw mut IObuff).cast::<c_char>();
            (*state).linelen = len;
        }
        ptr::copy_nonoverlapping(p_str, (*state).linebuf, (*state).linelen);
        *(*state).linebuf.add((*state).linelen) = NUL;

        p_str = p_str.add(len);
        (*state).p_str = p_str;

        QF_OK
    }
}

unsafe fn qf_get_next_list_line(state: *mut qfstate_T) -> c_int {
    unsafe {
        let mut p_li = (*state).p_li;
        while !p_li.is_null()
            && ((*p_li).li_tv.v_type != VarType::VAR_STRING
                || (*p_li).li_tv.vval.v_string.is_null())
        {
            p_li = (*p_li).li_next;
        }

        if p_li.is_null() {
            (*state).p_li = ptr::null_mut();
            return QF_END_OF_INPUT;
        }

        let s = (*p_li).li_tv.vval.v_string;
        let len = c_strlen(s);
        if len > IOSIZE - 2 {
            (*state).linebuf = qf_grow_linebuf(state, len);
        } else {
            (*state).linebuf = (&raw mut IObuff).cast::<c_char>();
            (*state).linelen = len;
        }

        xstrlcpy((*state).linebuf, s, (*state).linelen + 1);
        (*state).p_li = (*p_li).li_next;
        QF_OK
    }
}

unsafe fn qf_get_next_buf_line(state: *mut qfstate_T) -> c_int {
    unsafe {
        if (*state).buflnum > (*state).lnumlast {
            return QF_END_OF_INPUT;
        }
        let p_buf = ml_get_buf((*state).buf, (*state).buflnum);
        let len = ml_get_buf_len((*state).buf, (*state).buflnum) as usize;
        (*state).buflnum += 1;

        if len > IOSIZE - 2 {
            (*state).linebuf = qf_grow_linebuf(state, len);
        } else {
            (*state).linebuf = (&raw mut IObuff).cast::<c_char>();
            (*state).linelen = len;
        }
        xstrlcpy((*state).linebuf, p_buf, (*state).linelen + 1);
        QF_OK
    }
}

unsafe fn qf_get_next_file_line(state: *mut qfstate_T) -> c_int {
    unsafe {
        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
        loop {
            set_errno(0);
            if fgets(iobuff_ptr, IOSIZE as c_int, (*state).fd).is_null() {
                if get_errno() == EINTR {
                    continue;
                }
                return QF_END_OF_INPUT;
            }
            break;
        }

        let mut discard = false;
        (*state).linelen = c_strlen(iobuff_ptr);
        if (*state).linelen == IOSIZE - 1 && *iobuff_ptr.add((*state).linelen - 1) != NL {
            if (*state).growbuf.is_null() {
                (*state).growbufsiz = 2 * (IOSIZE - 1);
                (*state).growbuf = xmalloc((*state).growbufsiz).cast::<c_char>();
            }

            ptr::copy_nonoverlapping(iobuff_ptr, (*state).growbuf, IOSIZE - 1);
            let mut growbuflen = (*state).linelen;
            *(*state).growbuf.add(growbuflen) = NUL;

            loop {
                set_errno(0);
                if fgets(
                    (*state).growbuf.add(growbuflen),
                    ((*state).growbufsiz - growbuflen) as c_int,
                    (*state).fd,
                )
                .is_null()
                {
                    if get_errno() == EINTR {
                        continue;
                    }
                    break;
                }
                (*state).linelen = c_strlen((*state).growbuf.add(growbuflen));
                growbuflen += (*state).linelen;
                if *(*state).growbuf.add(growbuflen - 1) == NL {
                    break;
                }
                if (*state).growbufsiz == LINE_MAXLEN {
                    discard = true;
                    break;
                }

                (*state).growbufsiz = (2 * (*state).growbufsiz).min(LINE_MAXLEN);
                (*state).growbuf =
                    xrealloc((*state).growbuf.cast(), (*state).growbufsiz).cast::<c_char>();
            }

            if discard {
                loop {
                    set_errno(0);
                    if fgets(iobuff_ptr, IOSIZE as c_int, (*state).fd).is_null() {
                        if get_errno() == EINTR {
                            continue;
                        }
                        break;
                    }
                    if c_strlen(iobuff_ptr) < IOSIZE - 1 || *iobuff_ptr.add(IOSIZE - 2) == NL {
                        break;
                    }
                }
            }

            (*state).linebuf = (*state).growbuf;
            (*state).linelen = growbuflen;
        } else {
            (*state).linebuf = iobuff_ptr;
        }

        if (*state).vc.vc_type != CONV_NONE && has_non_ascii((*state).linebuf) {
            let line = string_convert(
                &raw mut (*state).vc,
                (*state).linebuf,
                &raw mut (*state).linelen,
            );
            if !line.is_null() {
                if (*state).linelen < IOSIZE {
                    xstrlcpy((*state).linebuf, line, (*state).linelen + 1);
                    xfree(line.cast());
                } else {
                    xfree((*state).growbuf.cast());
                    (*state).linebuf = line;
                    (*state).growbuf = line;
                    (*state).growbufsiz = (*state).linelen.min(LINE_MAXLEN);
                }
            }
        }
        QF_OK
    }
}

unsafe fn qf_get_nextline(state: *mut qfstate_T) -> c_int {
    unsafe {
        let mut status = QF_FAIL;

        if (*state).fd.is_null() {
            if !(*state).tv.is_null() {
                if (*(*state).tv).v_type == VarType::VAR_STRING {
                    status = qf_get_next_str_line(state);
                } else if (*(*state).tv).v_type == VarType::VAR_LIST {
                    status = qf_get_next_list_line(state);
                }
            } else {
                status = qf_get_next_buf_line(state);
            }
        } else {
            status = qf_get_next_file_line(state);
        }

        if status != QF_OK {
            return status;
        }

        if (*state).linelen > 0 && *(*state).linebuf.add((*state).linelen - 1) == NL {
            *(*state).linebuf.add((*state).linelen - 1) = NUL;
            #[cfg(windows)]
            if (*state).linelen > 1 && *(*state).linebuf.add((*state).linelen - 2) == CAR {
                *(*state).linebuf.add((*state).linelen - 2) = NUL;
            }
        }

        remove_bom((*state).linebuf);

        QF_OK
    }
}

#[inline]
unsafe fn qf_stack_empty(qi: *const qf_info_T) -> bool {
    qi.is_null() || unsafe { (*qi).qf_listcount <= 0 }
}

#[inline]
unsafe fn qf_list_empty(qfl: *const qf_list_T) -> bool {
    qfl.is_null() || unsafe { (*qfl).qf_count <= 0 }
}

#[inline]
unsafe fn qf_list_has_valid_entries(qfl: *const qf_list_T) -> bool {
    unsafe { !qf_list_empty(qfl) && !(*qfl).qf_nonevalid }
}

#[inline]
unsafe fn qf_get_list(qi: *mut qf_info_T, idx: c_int) -> *mut qf_list_T {
    unsafe { (*qi).qf_lists.add(idx as usize) }
}

unsafe fn qf_parse_line(
    qfl: *mut qf_list_T,
    mut linebuf: *mut c_char,
    mut linelen: usize,
    fmt_first: *mut efm_T,
    fields: *mut qffields_T,
) -> c_int {
    unsafe {
        loop {
            let mut fmt_ptr = if fmt_start.is_null() {
                fmt_first
            } else {
                let p = fmt_start;
                fmt_start = ptr::null_mut();
                p
            };

            let mut idx: c_int = 0;
            let mut tail: *mut c_char = ptr::null_mut();
            let mut status;

            (*fields).valid = true;
            while !fmt_ptr.is_null() {
                idx = c_int::from((*fmt_ptr).prefix as u8);
                status = qf_parse_get_fields(
                    linebuf,
                    linelen,
                    fmt_ptr,
                    fields,
                    c_int::from((*qfl).qf_multiline),
                    c_int::from((*qfl).qf_multiscan),
                    &raw mut tail,
                );
                if status == QF_NOMEM {
                    return status;
                }
                if status == QF_OK {
                    break;
                }
                fmt_ptr = (*fmt_ptr).next;
            }
            (*qfl).qf_multiscan = false;

            if fmt_ptr.is_null() || idx == c_int::from(b'D') || idx == c_int::from(b'X') {
                if !fmt_ptr.is_null() {
                    status = qf_parse_dir_pfx(idx, fields, qfl);
                    if status != QF_OK {
                        return status;
                    }
                }
                status = qf_parse_line_nomatch(linebuf, linelen, fields);
                if status != QF_OK {
                    return status;
                }
                if fmt_ptr.is_null() {
                    (*qfl).qf_multiline = false;
                    (*qfl).qf_multiignore = false;
                }
            } else {
                if (*fmt_ptr).conthere != 0 {
                    fmt_start = fmt_ptr;
                }

                if !vim_strchr(c"AEWIN".as_ptr(), idx).is_null() {
                    (*qfl).qf_multiline = true;
                    (*qfl).qf_multiignore = false;
                } else if !vim_strchr(c"CZ".as_ptr(), idx).is_null() {
                    status = qf_parse_multiline_pfx(idx, qfl, fields);
                    if status != QF_OK {
                        return status;
                    }
                } else if !vim_strchr(c"OPQ".as_ptr(), idx).is_null() {
                    status = qf_parse_file_pfx(idx, fields, qfl, tail);
                    if status == QF_MULTISCAN {
                        let s = skipwhite(tail);
                        let new_linelen = c_strlen(s);
                        if new_linelen >= linelen {
                            return QF_IGNORE_LINE;
                        }
                        linebuf = s;
                        linelen = new_linelen;
                        continue;
                    }
                }
                if (*fmt_ptr).flags == b'-' as c_char {
                    if (*qfl).qf_multiline {
                        (*qfl).qf_multiignore = true;
                    }
                    return QF_IGNORE_LINE;
                }
            }

            return QF_OK;
        }
    }
}

unsafe fn qf_alloc_fields(pfields: *mut qffields_T) {
    unsafe {
        (*pfields).namebuf = xmalloc(CMDBUFFSIZE + 1).cast::<c_char>();
        (*pfields).module = xmalloc(CMDBUFFSIZE + 1).cast::<c_char>();
        (*pfields).errmsglen = CMDBUFFSIZE + 1;
        (*pfields).errmsg = xmalloc((*pfields).errmsglen).cast::<c_char>();
        (*pfields).pattern = xmalloc(CMDBUFFSIZE + 1).cast::<c_char>();
    }
}

unsafe fn qf_free_fields(pfields: *mut qffields_T) {
    unsafe {
        xfree((*pfields).namebuf.cast());
        xfree((*pfields).module.cast());
        xfree((*pfields).errmsg.cast());
        xfree((*pfields).pattern.cast());
    }
}

unsafe fn qf_setup_state(
    pstate: *mut qfstate_T,
    enc: *mut c_char,
    efile: *const c_char,
    tv: *mut typval_T,
    buf: *mut buf_T,
    lnumfirst: linenr_T,
    lnumlast: linenr_T,
) -> c_int {
    unsafe {
        (*pstate).vc.vc_type = CONV_NONE;
        if !enc.is_null() && *enc != NUL {
            convert_setup(&raw mut (*pstate).vc, enc, p_enc);
        }

        if !efile.is_null() {
            (*pstate).fd = if strequal(efile, c"-".as_ptr()) {
                c_fdopen(os_open_stdin_fd(), c"r".as_ptr())
            } else {
                os_fopen(efile, c"r".as_ptr()).cast::<FILE>()
            };
            if (*pstate).fd.is_null() {
                semsg!(gettext((&raw const e_openerrf).cast()), efile);
                return FAIL;
            }
        }

        if !tv.is_null() {
            if (*tv).v_type == VarType::VAR_STRING {
                (*pstate).p_str = (*tv).vval.v_string;
            } else if (*tv).v_type == VarType::VAR_LIST {
                (*pstate).p_li = tv_list_first((*tv).vval.v_list);
            }
            (*pstate).tv = tv;
        }
        (*pstate).buf = buf;
        (*pstate).buflnum = lnumfirst;
        (*pstate).lnumlast = lnumlast;

        OK
    }
}

unsafe fn qf_cleanup_state(pstate: *mut qfstate_T) {
    unsafe {
        if !(*pstate).fd.is_null() {
            fclose((*pstate).fd);
        }
        xfree((*pstate).growbuf.cast());
        if (*pstate).vc.vc_type != CONV_NONE {
            convert_setup(&raw mut (*pstate).vc, ptr::null_mut(), ptr::null_mut());
        }
    }
}

unsafe fn qf_init_ext(
    qi: *mut qf_info_T,
    mut qf_idx: c_int,
    efile: *const c_char,
    buf: *mut buf_T,
    tv: *mut typval_T,
    errorformat: *mut c_char,
    newlist: bool,
    lnumfirst: linenr_T,
    lnumlast: linenr_T,
    qf_title: *const c_char,
    enc: *mut c_char,
) -> c_int {
    unsafe {
        let mut state: qfstate_T = std::mem::zeroed();
        let mut fields: qffields_T = std::mem::zeroed();
        let mut old_last: *mut qfline_T = ptr::null_mut();
        let mut retval: c_int = -1;

        xfree(qf_last_bufname.cast());
        qf_last_bufname = ptr::null_mut();

        qf_alloc_fields(&raw mut fields);
        if qf_setup_state(&raw mut state, enc, efile, tv, buf, lnumfirst, lnumlast) == FAIL {
            if qf_idx == (*qi).qf_curlist {
                qf_update_buffer(qi, old_last);
            }
            qf_cleanup_state(&raw mut state);
            qf_free_fields(&raw mut fields);
            return retval;
        }

        let qfl: *mut qf_list_T;
        let mut adding = false;
        if newlist || qf_idx == (*qi).qf_listcount {
            qf_new_list(qi, qf_title);
            qf_idx = (*qi).qf_curlist;
            qfl = qf_get_list(qi, qf_idx);
        } else {
            adding = true;
            qfl = qf_get_list(qi, qf_idx);
            if !qf_list_empty(qfl) {
                old_last = (*qfl).qf_last;
            }
        }

        let efm =
            if errorformat == p_efm && tv.is_null() && !buf.is_null() && *(*buf).b_p_efm != NUL {
                (*buf).b_p_efm
            } else {
                errorformat
            };

        if last_efm_cache.is_null() || c_strcmp(last_efm_cache, efm) != 0 {
            xfree(last_efm_cache.cast());
            last_efm_cache = ptr::null_mut();
            free_efm_list(&raw mut fmt_first_cache);

            fmt_first_cache = parse_efm_option(efm);
            if !fmt_first_cache.is_null() {
                last_efm_cache = xstrdup(efm);
            }
        }

        let mut ok = !fmt_first_cache.is_null();
        if ok {
            got_int = false;
            while !got_int {
                let status =
                    qf_init_process_nextline(qfl, fmt_first_cache, &raw mut state, &raw mut fields);
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
            if state.fd.is_null() || ferror(state.fd) == 0 {
                if (*qfl).qf_index == 0 {
                    (*qfl).qf_ptr = (*qfl).qf_start;
                    (*qfl).qf_index = 1;
                    (*qfl).qf_nonevalid = true;
                } else {
                    (*qfl).qf_nonevalid = false;
                    if (*qfl).qf_ptr.is_null() {
                        (*qfl).qf_ptr = (*qfl).qf_start;
                    }
                }
                retval = (*qfl).qf_count;
            } else {
                emsg(gettext((&raw const e_readerrf).cast()));
                ok = false;
            }
        }

        if !ok && !adding {
            qf_free(qfl);
            (*qi).qf_listcount -= 1;
            if (*qi).qf_curlist > 0 {
                (*qi).qf_curlist -= 1;
            }
        }

        if qf_idx == (*qi).qf_curlist {
            qf_update_buffer(qi, old_last);
        }
        qf_cleanup_state(&raw mut state);
        qf_free_fields(&raw mut fields);

        retval
    }
}

unsafe fn qf_store_title(qfl: *mut qf_list_T, title: *const c_char) {
    unsafe {
        xfree((*qfl).qf_title.cast());
        (*qfl).qf_title = ptr::null_mut();

        if title.is_null() {
            return;
        }

        let len = c_strlen(title) + 1;
        let p = xmallocz(len).cast::<c_char>();
        (*qfl).qf_title = p;
        xstrlcpy(p, title, len + 1);
    }
}

unsafe fn qf_cmdtitle(cmd: *const c_char) -> *mut c_char {
    unsafe {
        let buf = (&raw mut qftitle_str).cast::<c_char>();
        vim_snprintf!(buf, IOSIZE, c":%s".as_ptr(), cmd);
        buf
    }
}

#[inline]
unsafe fn qf_get_curlist(qi: *mut qf_info_T) -> *mut qf_list_T {
    unsafe { qf_get_list(qi, (*qi).qf_curlist) }
}

unsafe fn qf_pop_stack(qi: *mut qf_info_T, adjust: bool) {
    unsafe {
        qf_free((*qi).qf_lists);
        for i in 1..(*qi).qf_listcount {
            ptr::copy_nonoverlapping(
                (*qi).qf_lists.add(i as usize),
                (*qi).qf_lists.add((i - 1) as usize),
                1,
            );
        }

        ptr::write_bytes((*qi).qf_lists.add(((*qi).qf_listcount - 1) as usize), 0, 1);

        if adjust {
            (*qi).qf_listcount -= 1;
            if (*qi).qf_curlist == 0 {
                (*qi).qf_curlist = (*qi).qf_listcount - 1;
            } else {
                (*qi).qf_curlist -= 1;
            }
        }
    }
}

unsafe fn qf_new_list(qi: *mut qf_info_T, qf_title: *const c_char) {
    unsafe {
        while (*qi).qf_listcount > (*qi).qf_curlist + 1 {
            (*qi).qf_listcount -= 1;
            qf_free((*qi).qf_lists.add((*qi).qf_listcount as usize));
        }

        if (*qi).qf_listcount == (*qi).qf_maxcount {
            qf_pop_stack(qi, false);
            (*qi).qf_curlist = (*qi).qf_listcount - 1;
        } else {
            (*qi).qf_curlist = (*qi).qf_listcount;
            (*qi).qf_listcount += 1;
        }

        let qfl = qf_get_curlist(qi);
        ptr::write_bytes(qfl, 0, 1);
        qf_store_title(qfl, qf_title);
        (*qfl).qfl_type = (*qi).qfl_type;
        last_qf_id += 1;
        (*qfl).qf_id = last_qf_id;
        (*qfl).qf_has_user_data = false;
    }
}

unsafe fn qf_parse_fmt_f(
    rmp: *mut regmatch_T,
    midx: usize,
    fields: *mut qffields_T,
    prefix: c_int,
) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() || (*rmp).endp[midx].is_null() {
            return QF_FAIL;
        }

        let c = *(*rmp).endp[midx];
        *(*rmp).endp[midx] = NUL;
        expand_env((*rmp).startp[midx], (*fields).namebuf, CMDBUFFSIZE as c_int);
        *(*rmp).endp[midx] = c;

        if !vim_strchr(c"OPQ".as_ptr(), prefix).is_null() && !os_path_exists((*fields).namebuf) {
            return QF_FAIL;
        }

        QF_OK
    }
}

unsafe fn qf_parse_fmt_b(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        let bnr = atol((*rmp).startp[midx]) as c_int;
        if buflist_findnr(bnr).is_null() {
            return QF_FAIL;
        }
        (*fields).bnr = bnr;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_n(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).enr = atol((*rmp).startp[midx]) as c_int;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_l(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).lnum = atol((*rmp).startp[midx]) as linenr_T;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_e(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).end_lnum = atol((*rmp).startp[midx]) as linenr_T;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_c(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).col = atol((*rmp).startp[midx]) as c_int;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_k(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).end_col = atol((*rmp).startp[midx]) as c_int;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_t(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).type_ = *(*rmp).startp[midx];
        QF_OK
    }
}

unsafe fn copy_nonerror_line(
    linebuf: *const c_char,
    linelen: usize,
    fields: *mut qffields_T,
) -> c_int {
    unsafe {
        if linelen >= (*fields).errmsglen {
            (*fields).errmsg = xrealloc((*fields).errmsg.cast(), linelen + 1).cast::<c_char>();
            (*fields).errmsglen = linelen + 1;
        }
        xstrlcpy((*fields).errmsg, linebuf, linelen + 1);
        QF_OK
    }
}

unsafe fn qf_parse_fmt_m(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() || (*rmp).endp[midx].is_null() {
            return QF_FAIL;
        }
        let len = (*rmp).endp[midx].offset_from((*rmp).startp[midx]) as usize;
        if len >= (*fields).errmsglen {
            (*fields).errmsg = xrealloc((*fields).errmsg.cast(), len + 1).cast::<c_char>();
            (*fields).errmsglen = len + 1;
        }
        xstrlcpy((*fields).errmsg, (*rmp).startp[midx], len + 1);
        QF_OK
    }
}

unsafe fn qf_parse_fmt_r(rmp: *mut regmatch_T, midx: usize, tail: *mut *mut c_char) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        *tail = (*rmp).startp[midx];
        QF_OK
    }
}

unsafe fn qf_parse_fmt_p(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() || (*rmp).endp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).col = 0;
        let mut match_ptr = (*rmp).startp[midx];
        while match_ptr != (*rmp).endp[midx] {
            (*fields).col += 1;
            if *match_ptr == TAB {
                (*fields).col += 7;
                (*fields).col -= (*fields).col % 8;
            }
            match_ptr = match_ptr.add(1);
        }
        (*fields).col += 1;
        (*fields).use_viscol = true;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_v(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() {
            return QF_FAIL;
        }
        (*fields).col = atol((*rmp).startp[midx]) as c_int;
        (*fields).use_viscol = true;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_s(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() || (*rmp).endp[midx].is_null() {
            return QF_FAIL;
        }
        let mut len = (*rmp).endp[midx].offset_from((*rmp).startp[midx]) as usize;
        len = len.min(CMDBUFFSIZE - 5);
        c_strcpy((*fields).pattern, c"^\\V".as_ptr());
        xstrlcat((*fields).pattern, (*rmp).startp[midx], len + 4);
        *(*fields).pattern.add(len + 3) = b'\\' as c_char;
        *(*fields).pattern.add(len + 4) = b'$' as c_char;
        *(*fields).pattern.add(len + 5) = NUL;
        QF_OK
    }
}

unsafe fn qf_parse_fmt_o(rmp: *mut regmatch_T, midx: usize, fields: *mut qffields_T) -> c_int {
    unsafe {
        if (*rmp).startp[midx].is_null() || (*rmp).endp[midx].is_null() {
            return QF_FAIL;
        }
        let len = (*rmp).endp[midx].offset_from((*rmp).startp[midx]) as usize;
        let dsize = (c_strlen((*fields).module) + len + 1).min(CMDBUFFSIZE);
        xstrlcat((*fields).module, (*rmp).startp[midx], dsize);
        QF_OK
    }
}

unsafe fn qf_parse_match(
    linebuf: *mut c_char,
    linelen: usize,
    fmt_ptr: *mut efm_T,
    regmatch: *mut regmatch_T,
    fields: *mut qffields_T,
    qf_multiline: c_int,
    qf_multiscan: c_int,
    tail: *mut *mut c_char,
) -> c_int {
    unsafe {
        let idx = (*fmt_ptr).prefix;

        if (idx == b'C' as c_char || idx == b'Z' as c_char) && qf_multiline == 0 {
            return QF_FAIL;
        }
        if !vim_strchr(c"EWIN".as_ptr(), c_int::from(idx as u8)).is_null() {
            (*fields).type_ = idx;
        } else {
            (*fields).type_ = 0;
        }

        for i in 0..FMT_PATTERNS {
            let mut status = QF_OK;
            let midx = (*fmt_ptr).addr[i] as isize;
            if i == 0 && midx > 0 {
                status = qf_parse_fmt_f(regmatch, midx as usize, fields, c_int::from(idx as u8));
            } else if i == FMT_PATTERN_M {
                if (*fmt_ptr).flags == b'+' as c_char && qf_multiscan == 0 {
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
}

unsafe fn qf_parse_get_fields(
    linebuf: *mut c_char,
    linelen: usize,
    fmt_ptr: *mut efm_T,
    fields: *mut qffields_T,
    qf_multiline: c_int,
    qf_multiscan: c_int,
    tail: *mut *mut c_char,
) -> c_int {
    unsafe {
        if qf_multiscan != 0
            && vim_strchr(c"OPQ".as_ptr(), c_int::from((*fmt_ptr).prefix as u8)).is_null()
        {
            return QF_FAIL;
        }

        *(*fields).namebuf = NUL;
        (*fields).bnr = 0;
        *(*fields).module = NUL;
        *(*fields).pattern = NUL;
        if qf_multiscan == 0 {
            *(*fields).errmsg = NUL;
        }
        (*fields).lnum = 0;
        (*fields).end_lnum = 0;
        (*fields).col = 0;
        (*fields).end_col = 0;
        (*fields).use_viscol = false;
        (*fields).enr = -1;
        (*fields).type_ = 0;
        *tail = ptr::null_mut();

        let mut regmatch: regmatch_T = std::mem::zeroed();
        regmatch.rm_ic = true;
        regmatch.regprog = (*fmt_ptr).prog;
        let r = vim_regexec(&raw mut regmatch, linebuf, 0);
        (*fmt_ptr).prog = regmatch.regprog;
        let mut status = QF_FAIL;
        if r {
            status = qf_parse_match(
                linebuf,
                linelen,
                fmt_ptr,
                &raw mut regmatch,
                fields,
                qf_multiline,
                qf_multiscan,
                tail,
            );
        }

        status
    }
}

unsafe fn qf_parse_dir_pfx(idx: c_int, fields: *mut qffields_T, qfl: *mut qf_list_T) -> c_int {
    unsafe {
        if idx == c_int::from(b'D') {
            if *(*fields).namebuf == NUL {
                emsg(gettext(c"E379: Missing or empty directory name".as_ptr()));
                return QF_FAIL;
            }
            (*qfl).qf_directory =
                qf_push_dir((*fields).namebuf, &raw mut (*qfl).qf_dir_stack, false);
            if (*qfl).qf_directory.is_null() {
                return QF_FAIL;
            }
        } else if idx == c_int::from(b'X') {
            (*qfl).qf_directory = qf_pop_dir(&raw mut (*qfl).qf_dir_stack);
        }

        QF_OK
    }
}

unsafe fn qf_parse_file_pfx(
    idx: c_int,
    fields: *mut qffields_T,
    qfl: *mut qf_list_T,
    tail: *const c_char,
) -> c_int {
    unsafe {
        (*fields).valid = false;
        if *(*fields).namebuf == NUL || os_path_exists((*fields).namebuf) {
            if *(*fields).namebuf != NUL && idx == c_int::from(b'P') {
                (*qfl).qf_currfile =
                    qf_push_dir((*fields).namebuf, &raw mut (*qfl).qf_file_stack, true);
            } else if idx == c_int::from(b'Q') {
                (*qfl).qf_currfile = qf_pop_dir(&raw mut (*qfl).qf_file_stack);
            }
            *(*fields).namebuf = NUL;
            if !tail.is_null() && *tail != NUL {
                (*qfl).qf_multiscan = true;
                return QF_MULTISCAN;
            }
        }

        QF_OK
    }
}

unsafe fn qf_parse_line_nomatch(
    linebuf: *mut c_char,
    linelen: usize,
    fields: *mut qffields_T,
) -> c_int {
    unsafe {
        *(*fields).namebuf = NUL;
        (*fields).lnum = 0;
        (*fields).valid = false;

        copy_nonerror_line(linebuf, linelen, fields)
    }
}

unsafe fn qf_parse_multiline_pfx(
    idx: c_int,
    qfl: *mut qf_list_T,
    fields: *mut qffields_T,
) -> c_int {
    unsafe {
        if !(*qfl).qf_multiignore {
            let qfprev = (*qfl).qf_last;
            if qfprev.is_null() {
                return QF_FAIL;
            }
            if *(*fields).errmsg != NUL {
                let textlen = c_strlen((*qfprev).qf_text);
                let errlen = c_strlen((*fields).errmsg);
                (*qfprev).qf_text =
                    xrealloc((*qfprev).qf_text.cast(), textlen + errlen + 2).cast::<c_char>();
                *(*qfprev).qf_text.add(textlen) = b'\n' as c_char;
                c_strcpy((*qfprev).qf_text.add(textlen + 1), (*fields).errmsg);
            }
            if (*qfprev).qf_nr == -1 {
                (*qfprev).qf_nr = (*fields).enr;
            }
            if vim_isprintc(c_int::from((*fields).type_ as u8)) && (*qfprev).qf_type == 0 {
                (*qfprev).qf_type = (*fields).type_;
            }

            if (*qfprev).qf_lnum == 0 {
                (*qfprev).qf_lnum = (*fields).lnum;
            }
            if (*qfprev).qf_end_lnum == 0 {
                (*qfprev).qf_end_lnum = (*fields).end_lnum;
            }
            if (*qfprev).qf_col == 0 {
                (*qfprev).qf_col = (*fields).col;
                (*qfprev).qf_viscol = c_char::from((*fields).use_viscol);
            }
            if (*qfprev).qf_end_col == 0 {
                (*qfprev).qf_end_col = (*fields).end_col;
            }
            if (*qfprev).qf_fnum == 0 {
                let fname = if *(*fields).namebuf != NUL || !(*qfl).qf_directory.is_null() {
                    (*fields).namebuf
                } else if !(*qfl).qf_currfile.is_null() && (*fields).valid {
                    (*qfl).qf_currfile
                } else {
                    ptr::null_mut()
                };
                (*qfprev).qf_fnum = qf_get_fnum(qfl, (*qfl).qf_directory, fname);
            }
        }
        if idx == c_int::from(b'Z') {
            (*qfl).qf_multiline = false;
            (*qfl).qf_multiignore = false;
        }
        line_breakcheck();

        QF_IGNORE_LINE
    }
}

unsafe fn locstack_queue_delreq(qi: *mut qf_info_T) {
    unsafe {
        let q = xmalloc(size_of::<qf_delq_T>()).cast::<qf_delq_T>();
        (*q).qi = qi;
        (*q).next = qf_delq_head;
        qf_delq_head = q;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_stack_get_bufnr() -> c_int {
    unsafe {
        assert!(!ql_info.is_null());
        (*ql_info).qf_bufnr
    }
}

unsafe fn wipe_qf_buffer(qi: *mut qf_info_T) {
    unsafe {
        if (*qi).qf_bufnr == INVALID_QFBUFNR {
            return;
        }

        let qfbuf = buflist_findnr((*qi).qf_bufnr);
        if !qfbuf.is_null() && (*qfbuf).b_nwindows == 0 {
            let mut buf_was_null = false;
            if (*curwin).w_buffer.is_null() {
                (*curwin).w_buffer = curbuf;
                buf_was_null = true;
            }

            close_buffer(ptr::null_mut(), qfbuf, DOBUF_WIPE, false, false, false);
            (*qi).qf_bufnr = INVALID_QFBUFNR;
            if buf_was_null {
                (*curwin).w_buffer = ptr::null_mut();
            }
        }
    }
}

unsafe fn qf_free_list_stack_items(qi: *mut qf_info_T) {
    unsafe {
        for i in 0..(*qi).qf_listcount {
            qf_free(qf_get_list(qi, i));
        }
    }
}

unsafe fn qf_free_lists(qi: *mut qf_info_T) {
    unsafe {
        qf_free_list_stack_items(qi);
        xfree((*qi).qf_lists.cast());
        xfree(qi.cast());
    }
}

unsafe fn ll_free_all(pqi: *mut *mut qf_info_T) {
    unsafe {
        let qi = *pqi;
        if qi.is_null() {
            return;
        }
        *pqi = ptr::null_mut();

        if quickfix_busy > 0 {
            locstack_queue_delreq(qi);
            return;
        }

        (*qi).qf_refcount -= 1;
        if (*qi).qf_refcount < 1 {
            wipe_qf_buffer(qi);
            qf_free_lists(qi);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_free_all(wp: *mut win_T) {
    unsafe {
        let qi = ql_info;
        if !wp.is_null() {
            ll_free_all((&raw mut (*wp).w_llist).cast::<*mut qf_info_T>());
            ll_free_all((&raw mut (*wp).w_llist_ref).cast::<*mut qf_info_T>());
        } else if !qi.is_null() {
            qf_free_list_stack_items(qi);
        }
    }
}

unsafe fn incr_quickfix_busy() {
    unsafe {
        quickfix_busy += 1;
    }
}

unsafe fn decr_quickfix_busy() {
    unsafe {
        quickfix_busy -= 1;
        if quickfix_busy == 0 {
            while !qf_delq_head.is_null() {
                let q = qf_delq_head;
                qf_delq_head = (*q).next;
                ll_free_all(&raw mut (*q).qi);
                xfree(q.cast());
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_quickfix_busy() {
    unsafe {
        if quickfix_busy != 0 {
            semsg!(
                c"quickfix_busy not zero on exit: %lld".as_ptr(),
                c_longlong::from(quickfix_busy),
            );
        }
    }
}

unsafe fn qf_add_entry(
    qfl: *mut qf_list_T,
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
    user_data: *mut typval_T,
    valid: c_char,
) -> c_int {
    unsafe {
        let buf: *mut buf_T;
        let qfp = xmalloc(size_of::<qfline_T>()).cast::<qfline_T>();
        let mut fullname: *mut c_char = ptr::null_mut();

        if bufnum != 0 {
            buf = buflist_findnr(bufnum);
            (*qfp).qf_fnum = bufnum;
            if !buf.is_null() {
                (*buf).b_has_qf_entry |= if is_qf_list(qfl) {
                    BUF_HAS_QF_ENTRY
                } else {
                    BUF_HAS_LL_ENTRY
                };
            }
        } else {
            (*qfp).qf_fnum = qf_get_fnum(qfl, dir, fname);
            buf = buflist_findnr((*qfp).qf_fnum);
        }
        if !fname.is_null() {
            fullname = fix_fname(fname);
        }
        (*qfp).qf_fname = ptr::null_mut();
        if !buf.is_null()
            && !(*buf).b_ffname.is_null()
            && !fullname.is_null()
            && !path_equal(fullname, (*buf).b_ffname, kPathCmpLiteral)
        {
            let p = path_try_shorten_fname(fullname);
            if !p.is_null() {
                (*qfp).qf_fname = xstrdup(p);
            }
        }
        xfree(fullname.cast());
        (*qfp).qf_text = xstrdup(mesg);
        (*qfp).qf_lnum = lnum;
        (*qfp).qf_end_lnum = end_lnum;
        (*qfp).qf_col = col;
        (*qfp).qf_end_col = end_col;
        (*qfp).qf_viscol = vis_col;
        if user_data.is_null() || (*user_data).v_type == VarType::VAR_UNKNOWN {
            (*qfp).qf_user_data.v_type = VarType::VAR_UNKNOWN;
        } else {
            tv_copy(user_data, &raw mut (*qfp).qf_user_data);
            (*qfl).qf_has_user_data = true;
        }
        if pattern.is_null() || *pattern == NUL {
            (*qfp).qf_pattern = ptr::null_mut();
        } else {
            (*qfp).qf_pattern = xstrdup(pattern);
        }
        if module.is_null() || *module == NUL {
            (*qfp).qf_module = ptr::null_mut();
        } else {
            (*qfp).qf_module = xstrdup(module);
        }
        (*qfp).qf_nr = nr;
        if type_ != 1 && !vim_isprintc(c_int::from(type_ as u8)) {
            type_ = 0;
        }
        (*qfp).qf_type = type_;
        (*qfp).qf_valid = valid;

        let lastp = &raw mut (*qfl).qf_last;
        if qf_list_empty(qfl) {
            (*qfl).qf_start = qfp;
            (*qfl).qf_ptr = qfp;
            (*qfl).qf_index = 0;
            (*qfp).qf_prev = ptr::null_mut();
        } else {
            assert!(!(*lastp).is_null());
            (*qfp).qf_prev = *lastp;
            (**lastp).qf_next = qfp;
        }
        (*qfp).qf_next = ptr::null_mut();
        (*qfp).qf_cleared = 0;
        *lastp = qfp;
        (*qfl).qf_count += 1;
        if (*qfl).qf_index == 0 && (*qfp).qf_valid != 0 {
            (*qfl).qf_index = (*qfl).qf_count;
            (*qfl).qf_ptr = qfp;
        }

        QF_OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_resize_stack(n: c_int) {
    unsafe {
        assert!(!ql_info.is_null());
        qf_resize_stack_base(ql_info, n);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ll_resize_stack(wp: *mut win_T, n: c_int) {
    unsafe {
        if is_ll_window(wp) {
            qf_sync_llw_to_win(wp);
        } else {
            qf_sync_win_to_llw(wp);
        }

        let qi = ll_get_or_alloc_list(wp);
        qf_resize_stack_base(qi, n);
    }
}

unsafe fn qf_resize_stack_base(qi: *mut qf_info_T, n: c_int) {
    unsafe {
        let lsz = size_of::<qf_list_T>();

        if n == (*qi).qf_maxcount {
            return;
        } else if n < (*qi).qf_maxcount && n < (*qi).qf_listcount {
            let amount_to_rm = (*qi).qf_listcount - n;
            for _ in 0..amount_to_rm {
                qf_pop_stack(qi, true);
            }
        }

        let new = xrealloc((*qi).qf_lists.cast(), lsz * (n as usize)).cast::<qf_list_T>();

        if n > (*qi).qf_maxcount {
            ptr::write_bytes(
                new.add((*qi).qf_maxcount as usize),
                0,
                (n - (*qi).qf_maxcount) as usize,
            );
        }

        (*qi).qf_lists = new;
        (*qi).qf_maxcount = n;

        qf_update_buffer(qi, ptr::null_mut());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_init_stack() {
    unsafe {
        ql_info = qf_alloc_stack(QFLT_QUICKFIX, p_chi as c_int);
    }
}

unsafe fn qf_sync_llw_to_win(llw: *mut win_T) {
    unsafe {
        let wp = qf_find_win_with_loclist((*llw).w_llist_ref.cast::<qf_info_T>());
        if !wp.is_null() {
            (*wp).w_onebuf_opt.wo_lhi = (*llw).w_onebuf_opt.wo_lhi;
        }
    }
}

unsafe fn qf_sync_win_to_llw(pwp: *mut win_T) {
    unsafe {
        let llw = (*pwp).w_llist.cast::<qf_info_T>();
        if !llw.is_null() {
            let mut wp = tab_firstwin(curtab);
            while !wp.is_null() {
                if (*wp).w_llist_ref.cast::<qf_info_T>() == llw && bt_quickfix((*wp).w_buffer) {
                    (*wp).w_onebuf_opt.wo_lhi = (*pwp).w_onebuf_opt.wo_lhi;
                    return;
                }
                wp = (*wp).w_next;
            }
        }
    }
}

unsafe fn qf_alloc_stack(qfltype: qfltype_T, n: c_int) -> *mut qf_info_T {
    unsafe {
        let qi = if qfltype == QFLT_QUICKFIX {
            &raw mut ql_info_actual
        } else {
            let q = xcalloc(1, size_of::<qf_info_T>()).cast::<qf_info_T>();
            (*q).qf_refcount += 1;
            q
        };
        (*qi).qfl_type = qfltype;
        (*qi).qf_bufnr = INVALID_QFBUFNR;
        (*qi).qf_lists = qf_alloc_list_stack(n);
        (*qi).qf_maxcount = n;

        qi
    }
}

unsafe fn qf_alloc_list_stack(n: c_int) -> *mut qf_list_T {
    unsafe { xcalloc(n as usize, size_of::<qf_list_T>()).cast::<qf_list_T>() }
}

unsafe fn ll_get_or_alloc_list(wp: *mut win_T) -> *mut qf_info_T {
    unsafe {
        if is_ll_window(wp) {
            return (*wp).w_llist_ref.cast::<qf_info_T>();
        }

        ll_free_all((&raw mut (*wp).w_llist_ref).cast::<*mut qf_info_T>());

        if (*wp).w_llist.is_null() {
            (*wp).w_llist =
                qf_alloc_stack(QFLT_LOCATION, (*wp).w_onebuf_opt.wo_lhi as c_int).cast::<c_void>();
        }

        (*wp).w_llist.cast::<qf_info_T>()
    }
}

unsafe fn qf_cmd_get_stack(eap: *mut exarg_T, print_emsg: bool) -> *mut qf_info_T {
    unsafe {
        let mut qi = ql_info;
        assert!(!qi.is_null());

        if is_loclist_cmd((*eap).cmdidx) {
            qi = get_loc_list(curwin);
            if qi.is_null() {
                if print_emsg {
                    emsg(gettext((&raw const e_loclist).cast()));
                }
                return ptr::null_mut();
            }
        }

        qi
    }
}

unsafe fn qf_cmd_get_or_alloc_stack(eap: *const exarg_T, pwinp: *mut *mut win_T) -> *mut qf_info_T {
    unsafe {
        let mut qi = ql_info;
        if is_loclist_cmd((*eap).cmdidx) {
            qi = ll_get_or_alloc_list(curwin);
            *pwinp = curwin;
        }
        qi
    }
}

unsafe fn copy_loclist_entries(from_qfl: *const qf_list_T, to_qfl: *mut qf_list_T) -> c_int {
    unsafe {
        let mut i: c_int = 1;
        let mut from_qfp = (*from_qfl).qf_start;
        while !got_int && i <= (*from_qfl).qf_count && !from_qfp.is_null() {
            if qf_add_entry(
                to_qfl,
                ptr::null_mut(),
                ptr::null_mut(),
                (*from_qfp).qf_module,
                0,
                (*from_qfp).qf_text,
                (*from_qfp).qf_lnum,
                (*from_qfp).qf_end_lnum,
                (*from_qfp).qf_col,
                (*from_qfp).qf_end_col,
                (*from_qfp).qf_viscol,
                (*from_qfp).qf_pattern,
                (*from_qfp).qf_nr,
                0,
                &raw mut (*from_qfp).qf_user_data,
                (*from_qfp).qf_valid,
            ) == QF_FAIL
            {
                return FAIL;
            }

            let prevp = (*to_qfl).qf_last;
            (*prevp).qf_fnum = (*from_qfp).qf_fnum;
            (*prevp).qf_type = (*from_qfp).qf_type;
            if (*from_qfl).qf_ptr == from_qfp {
                (*to_qfl).qf_ptr = prevp;
            }

            i += 1;
            from_qfp = (*from_qfp).qf_next;
        }

        OK
    }
}

unsafe fn copy_loclist(from_qfl: *mut qf_list_T, to_qfl: *mut qf_list_T) -> c_int {
    unsafe {
        (*to_qfl).qfl_type = (*from_qfl).qfl_type;
        (*to_qfl).qf_nonevalid = (*from_qfl).qf_nonevalid;
        (*to_qfl).qf_has_user_data = (*from_qfl).qf_has_user_data;
        (*to_qfl).qf_count = 0;
        (*to_qfl).qf_index = 0;
        (*to_qfl).qf_start = ptr::null_mut();
        (*to_qfl).qf_last = ptr::null_mut();
        (*to_qfl).qf_ptr = ptr::null_mut();
        if !(*from_qfl).qf_title.is_null() {
            (*to_qfl).qf_title = xstrdup((*from_qfl).qf_title);
        } else {
            (*to_qfl).qf_title = ptr::null_mut();
        }
        if !(*from_qfl).qf_ctx.is_null() {
            (*to_qfl).qf_ctx = xcalloc(1, size_of::<typval_T>()).cast::<typval_T>();
            tv_copy((*from_qfl).qf_ctx, (*to_qfl).qf_ctx);
        } else {
            (*to_qfl).qf_ctx = ptr::null_mut();
        }
        callback_copy(
            &raw mut (*to_qfl).qf_qftf_cb,
            &raw mut (*from_qfl).qf_qftf_cb,
        );

        if (*from_qfl).qf_count != 0 && copy_loclist_entries(from_qfl, to_qfl) == FAIL {
            return FAIL;
        }

        (*to_qfl).qf_index = (*from_qfl).qf_index;
        last_qf_id += 1;
        (*to_qfl).qf_id = last_qf_id;
        (*to_qfl).qf_changedtick = 0;

        if (*to_qfl).qf_nonevalid {
            (*to_qfl).qf_ptr = (*to_qfl).qf_start;
            (*to_qfl).qf_index = 1;
        }

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn copy_loclist_stack(from: *mut win_T, to: *mut win_T) {
    unsafe {
        let qi = get_loc_list(from);
        if qi.is_null() {
            return;
        }

        let to_ll = qf_alloc_stack(QFLT_LOCATION, (*from).w_onebuf_opt.wo_lhi as c_int);
        (*to).w_llist = to_ll.cast::<c_void>();
        (*to).w_onebuf_opt.wo_lhi = i64::from((*to_ll).qf_maxcount);
        (*to_ll).qf_listcount = (*qi).qf_listcount;

        for idx in 0..(*qi).qf_listcount {
            (*to_ll).qf_curlist = idx;
            if copy_loclist(qf_get_list(qi, idx), qf_get_list(to_ll, idx)) == FAIL {
                qf_free_all(to);
                return;
            }
        }

        (*to_ll).qf_curlist = (*qi).qf_curlist;
    }
}

unsafe fn qf_get_fnum(
    qfl: *mut qf_list_T,
    mut directory: *mut c_char,
    fname: *mut c_char,
) -> c_int {
    unsafe {
        let mut ptr_str = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        if fname.is_null() || *fname == NUL {
            return 0;
        }

        let fname_str = cstr_as_string(fname);
        let bufname = if !directory.is_null() && !vim_isAbsName(fname) {
            ptr_str = concat_fnames(cstr_as_string(directory), fname_str, true);
            if !os_path_exists(ptr_str.data) {
                xfree(ptr_str.data.cast());
                directory = qf_guess_filepath(qfl, fname);
                if !directory.is_null() {
                    ptr_str = concat_fnames(cstr_as_string(directory), fname_str, true);
                } else {
                    ptr_str = copy_string(fname_str, ptr::null_mut());
                }
            }
            ptr_str
        } else {
            fname_str
        };
        #[cfg(windows)]
        path_to_slash(bufname.data);

        let buf = if !qf_last_bufname.is_null()
            && c_strcmp(bufname.data, qf_last_bufname) == 0
            && bufref_valid(&raw mut qf_last_bufref)
        {
            xfree(ptr_str.data.cast());
            qf_last_bufref.br_buf
        } else {
            xfree(qf_last_bufname.cast());
            let b = buflist_new(bufname.data, ptr::null_mut(), 0, BLN_NOOPT);
            qf_last_bufname = if bufname.data == ptr_str.data {
                bufname.data
            } else {
                xmemdupz(bufname.data.cast(), bufname.size).cast::<c_char>()
            };
            set_bufref(&raw mut qf_last_bufref, b);
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

unsafe fn qf_push_dir(
    dirbuf: *mut c_char,
    stackptr: *mut *mut dir_stack_T,
    is_file_stack: bool,
) -> *mut c_char {
    unsafe {
        let mut ds_new = xmalloc(size_of::<dir_stack_T>()).cast::<dir_stack_T>();
        (*ds_new).next = *stackptr;
        *stackptr = ds_new;

        if vim_isAbsName(dirbuf) || (**stackptr).next.is_null() || is_file_stack {
            (**stackptr).dirname = xstrdup(dirbuf);
        } else {
            let dirbuf_len = c_strlen(dirbuf);
            ds_new = (**stackptr).next;
            (**stackptr).dirname = ptr::null_mut();
            while !ds_new.is_null() {
                let dirname = concat_fnames(
                    cstr_as_string((*ds_new).dirname),
                    cbuf_as_string(dirbuf, dirbuf_len),
                    true,
                );
                if os_isdir(dirname.data) {
                    xfree((**stackptr).dirname.cast());
                    (**stackptr).dirname = dirname.data;
                    break;
                }
                xfree(dirname.data.cast());
                ds_new = (*ds_new).next;
            }

            while (**stackptr).next != ds_new {
                let ds_ptr = (**stackptr).next;
                (**stackptr).next = (*ds_ptr).next;
                xfree((*ds_ptr).dirname.cast());
                xfree(ds_ptr.cast());
            }

            if ds_new.is_null() {
                xfree((**stackptr).dirname.cast());
                (**stackptr).dirname = xmemdupz(dirbuf.cast(), dirbuf_len).cast::<c_char>();
            }
        }

        if !(**stackptr).dirname.is_null() {
            return (**stackptr).dirname;
        }

        let ds_ptr = *stackptr;
        *stackptr = (**stackptr).next;
        xfree(ds_ptr.cast());

        ptr::null_mut()
    }
}

unsafe fn qf_pop_dir(stackptr: *mut *mut dir_stack_T) -> *mut c_char {
    unsafe {
        if !(*stackptr).is_null() {
            let ds_ptr = *stackptr;
            *stackptr = (*ds_ptr).next;
            xfree((*ds_ptr).dirname.cast());
            xfree(ds_ptr.cast());
        }

        if (*stackptr).is_null() {
            ptr::null_mut()
        } else {
            (**stackptr).dirname
        }
    }
}

unsafe fn qf_clean_dir_stack(stackptr: *mut *mut dir_stack_T) {
    unsafe {
        while !(*stackptr).is_null() {
            let ds_ptr = *stackptr;
            *stackptr = (*ds_ptr).next;
            xfree((*ds_ptr).dirname.cast());
            xfree(ds_ptr.cast());
        }
    }
}

unsafe fn qf_guess_filepath(qfl: *mut qf_list_T, filename: *mut c_char) -> *mut c_char {
    unsafe {
        if (*qfl).qf_dir_stack.is_null() {
            return ptr::null_mut();
        }

        let mut ds_ptr = (*(*qfl).qf_dir_stack).next;
        let mut fullname = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        let filename_str = cstr_as_string(filename);
        while !ds_ptr.is_null() {
            xfree(fullname.data.cast());
            fullname = concat_fnames(cstr_as_string((*ds_ptr).dirname), filename_str, true);
            if os_path_exists(fullname.data) {
                break;
            }
            ds_ptr = (*ds_ptr).next;
        }

        xfree(fullname.data.cast());

        while (*(*qfl).qf_dir_stack).next != ds_ptr {
            let ds_tmp = (*(*qfl).qf_dir_stack).next;
            (*(*qfl).qf_dir_stack).next = (*ds_tmp).next;
            xfree((*ds_tmp).dirname.cast());
            xfree(ds_tmp.cast());
        }

        if ds_ptr.is_null() {
            ptr::null_mut()
        } else {
            (*ds_ptr).dirname
        }
    }
}

unsafe fn qflist_valid(wp: *mut win_T, qf_id: c_uint) -> bool {
    unsafe {
        let mut qi = ql_info;
        if !wp.is_null() {
            if !win_valid(wp) {
                return false;
            }
            qi = get_loc_list(wp);
        }
        if qi.is_null() {
            return false;
        }

        for i in 0..(*qi).qf_listcount {
            if (*(*qi).qf_lists.add(i as usize)).qf_id == qf_id {
                return true;
            }
        }
        false
    }
}

unsafe fn is_qf_entry_present(qfl: *mut qf_list_T, qf_ptr: *mut qfline_T) -> bool {
    unsafe {
        let mut i: c_int = 1;
        let mut qfp = (*qfl).qf_start;
        while !got_int && i <= (*qfl).qf_count && !qfp.is_null() {
            if qfp == qf_ptr {
                break;
            }
            i += 1;
            qfp = (*qfp).qf_next;
        }
        i <= (*qfl).qf_count
    }
}

unsafe fn get_next_valid_entry(
    qfl: *mut qf_list_T,
    mut qf_ptr: *mut qfline_T,
    qf_index: *mut c_int,
    dir: c_int,
) -> *mut qfline_T {
    unsafe {
        let mut idx = *qf_index;
        let old_qf_fnum = (*qf_ptr).qf_fnum;

        loop {
            if idx == (*qfl).qf_count || (*qf_ptr).qf_next.is_null() {
                return ptr::null_mut();
            }
            idx += 1;
            qf_ptr = (*qf_ptr).qf_next;
            if !((!(*qfl).qf_nonevalid && (*qf_ptr).qf_valid == 0)
                || (dir == FORWARD_FILE && (*qf_ptr).qf_fnum == old_qf_fnum))
            {
                break;
            }
        }

        *qf_index = idx;
        qf_ptr
    }
}

unsafe fn get_prev_valid_entry(
    qfl: *mut qf_list_T,
    mut qf_ptr: *mut qfline_T,
    qf_index: *mut c_int,
    dir: c_int,
) -> *mut qfline_T {
    unsafe {
        let mut idx = *qf_index;
        let old_qf_fnum = (*qf_ptr).qf_fnum;

        loop {
            if idx == 1 || (*qf_ptr).qf_prev.is_null() {
                return ptr::null_mut();
            }
            idx -= 1;
            qf_ptr = (*qf_ptr).qf_prev;
            if !((!(*qfl).qf_nonevalid && (*qf_ptr).qf_valid == 0)
                || (dir == BACKWARD_FILE && (*qf_ptr).qf_fnum == old_qf_fnum))
            {
                break;
            }
        }

        *qf_index = idx;
        qf_ptr
    }
}

unsafe fn get_nth_valid_entry(
    qfl: *mut qf_list_T,
    mut errornr: c_int,
    dir: c_int,
    new_qfidx: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        let mut qf_ptr = (*qfl).qf_ptr;
        let mut qf_idx = (*qfl).qf_index;
        let mut err: *const c_char = E_NO_MORE_ITEMS.as_ptr();

        while errornr > 0 {
            errornr -= 1;
            let prev_qf_ptr = qf_ptr;
            let prev_index = qf_idx;

            if dir == FORWARD || dir == FORWARD_FILE {
                qf_ptr = get_next_valid_entry(qfl, qf_ptr, &raw mut qf_idx, dir);
            } else {
                qf_ptr = get_prev_valid_entry(qfl, qf_ptr, &raw mut qf_idx, dir);
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

unsafe fn get_nth_entry(
    qfl: *mut qf_list_T,
    errornr: c_int,
    new_qfidx: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        let mut qf_ptr = (*qfl).qf_ptr;
        let mut qf_idx = (*qfl).qf_index;

        while errornr < qf_idx && qf_idx > 1 && !(*qf_ptr).qf_prev.is_null() {
            qf_idx -= 1;
            qf_ptr = (*qf_ptr).qf_prev;
        }

        while errornr > qf_idx && qf_idx < (*qfl).qf_count && !(*qf_ptr).qf_next.is_null() {
            qf_idx += 1;
            qf_ptr = (*qf_ptr).qf_next;
        }

        *new_qfidx = qf_idx;
        qf_ptr
    }
}

unsafe fn qf_get_entry(
    qfl: *mut qf_list_T,
    errornr: c_int,
    dir: c_int,
    new_qfidx: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        let mut qf_ptr = (*qfl).qf_ptr;
        let mut qfidx = (*qfl).qf_index;

        if dir != 0 {
            qf_ptr = get_nth_valid_entry(qfl, errornr, dir, &raw mut qfidx);
        } else if errornr != 0 {
            qf_ptr = get_nth_entry(qfl, errornr, &raw mut qfidx);
        }

        *new_qfidx = qfidx;
        qf_ptr
    }
}

unsafe fn qf_find_help_win() -> *mut win_T {
    unsafe {
        let mut wp = tab_firstwin(curtab);
        while !wp.is_null() {
            if bt_help((*wp).w_buffer) && !(*wp).w_config.hide && (*wp).w_config.focusable {
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

unsafe fn win_set_loclist(wp: *mut win_T, qi: *mut qf_info_T) {
    unsafe {
        (*wp).w_llist = qi.cast::<c_void>();
        (*qi).qf_refcount += 1;
    }
}

unsafe fn jump_to_help_window(qi: *mut qf_info_T, newwin: bool, opened_window: *mut bool) -> c_int {
    unsafe {
        let wp = if cmdmod.cmod_tab != 0 || newwin {
            ptr::null_mut()
        } else {
            qf_find_help_win()
        };

        if !wp.is_null() && (*(*wp).w_buffer).b_nwindows > 0 {
            win_enter(wp, true);
        } else {
            let mut flags = WSP_HELP;
            if cmdmod.cmod_split == 0 && (*curwin).w_width != Columns && (*curwin).w_width < 80 {
                flags |= WSP_TOP;
            }

            if is_ll_stack(qi) && !newwin {
                flags |= WSP_NEWLOC;
            }

            if win_split(0, flags) == FAIL {
                return FAIL;
            }

            *opened_window = true;

            if i64::from((*curwin).w_height) < p_hh {
                win_setheight(p_hh as c_int);
            }

            if is_ll_stack(qi) && !newwin {
                win_set_loclist(curwin, qi);
            }
        }

        restart_edit = 0;

        OK
    }
}

unsafe fn qf_find_win_with_loclist(ll: *const qf_info_T) -> *mut win_T {
    unsafe {
        let mut wp = tab_firstwin(curtab);
        while !wp.is_null() {
            if (*wp).w_llist.cast::<qf_info_T>().cast_const() == ll && !bt_quickfix((*wp).w_buffer)
            {
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

unsafe fn qf_find_win_with_normal_buf() -> *mut win_T {
    unsafe {
        let mut wp = tab_firstwin(curtab);
        while !wp.is_null() {
            if bt_normal((*wp).w_buffer) {
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

unsafe fn qf_goto_tabwin_with_file(fnum: c_int) -> bool {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = tab_firstwin(tp);
            while !wp.is_null() {
                if (*(*wp).w_buffer).b_fnum == fnum {
                    goto_tabpage_win(tp, wp);
                    return true;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        false
    }
}

unsafe fn qf_open_new_file_win(ll_ref: *mut qf_info_T) -> c_int {
    unsafe {
        let mut flags = WSP_ABOVE;
        if !ll_ref.is_null() {
            flags |= WSP_NEWLOC;
        }
        if win_split(0, flags) == FAIL {
            return FAIL;
        }
        p_swb = (&raw mut empty_string_option).cast::<c_char>();
        swb_flags = 0;
        reset_binding(curwin);
        if !ll_ref.is_null() {
            win_set_loclist(curwin, ll_ref);
        }
        OK
    }
}

unsafe fn qf_goto_win_with_ll_file(use_win: *mut win_T, qf_fnum: c_int, ll_ref: *mut qf_info_T) {
    unsafe {
        let mut win = use_win;

        if win.is_null() {
            let mut win2 = tab_firstwin(curtab);
            while !win2.is_null() {
                if (*(*win2).w_buffer).b_fnum == qf_fnum {
                    win = win2;
                    break;
                }
                win2 = (*win2).w_next;
            }
            if win.is_null() {
                win = curwin;
                loop {
                    if bt_normal((*win).w_buffer) {
                        break;
                    }
                    if (*win).w_prev.is_null() {
                        win = lastwin;
                    } else {
                        win = (*win).w_prev;
                    }
                    if win == curwin {
                        break;
                    }
                }
            }
        }
        win_goto(win);

        if (*win).w_llist.is_null() && !ll_ref.is_null() {
            win_set_loclist(win, ll_ref);
        }
    }
}

unsafe fn qf_goto_win_with_qfl_file(qf_fnum: c_int) {
    unsafe {
        let mut win = curwin;
        let mut altwin: *mut win_T = ptr::null_mut();
        loop {
            if (*(*win).w_buffer).b_fnum == qf_fnum {
                break;
            }
            if (*win).w_prev.is_null() {
                win = lastwin;
            } else {
                win = (*win).w_prev;
            }

            if is_qf_window(win) {
                if (swb_flags & kOptSwbFlagUselast) != 0
                    && win_valid(prevwin)
                    && (*prevwin).w_onebuf_opt.wo_wfb == 0
                {
                    win = prevwin;
                } else if !altwin.is_null() {
                    win = altwin;
                } else if !(*curwin).w_prev.is_null() {
                    win = (*curwin).w_prev;
                } else {
                    win = (*curwin).w_next;
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

unsafe fn qf_jump_to_usable_window(
    qf_fnum: c_int,
    newwin: bool,
    opened_window: *mut bool,
) -> c_int {
    unsafe {
        let mut usable_wp: *mut win_T = ptr::null_mut();
        let mut usable_win = false;

        let ll_ref = if newwin {
            ptr::null_mut()
        } else {
            (*curwin).w_llist_ref.cast::<qf_info_T>()
        };
        if !ll_ref.is_null() {
            usable_wp = qf_find_win_with_loclist(ll_ref);
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

        if !usable_win && (swb_flags & kOptSwbFlagUsetab) != 0 {
            usable_win = qf_goto_tabwin_with_file(qf_fnum);
        }

        if ((firstwin == lastwin) && bt_quickfix(curbuf)) || !usable_win || newwin {
            if qf_open_new_file_win(ll_ref) != OK {
                return FAIL;
            }
            *opened_window = true;
        } else if !(*curwin).w_llist_ref.is_null() {
            qf_goto_win_with_ll_file(usable_wp, qf_fnum, ll_ref);
        } else {
            qf_goto_win_with_qfl_file(qf_fnum);
        }

        OK
    }
}

unsafe fn qf_jump_edit_buffer(
    qi: *mut qf_info_T,
    qf_ptr: *mut qfline_T,
    forceit: c_int,
    prev_winid: c_int,
    opened_window: *mut bool,
) -> c_int {
    unsafe {
        let qfl = qf_get_curlist(qi);
        let old_changetick = (*qfl).qf_changedtick;
        let old_qf_curlist = (*qi).qf_curlist;
        let qfl_type = (*qfl).qfl_type;
        let mut retval = OK;
        let save_qfid = (*qfl).qf_id;

        if (*qf_ptr).qf_type == 1 {
            if !can_abandon(curbuf, forceit != 0) {
                no_write_message();
                return FAIL;
            }
            retval = do_ecmd(
                (*qf_ptr).qf_fnum,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                1,
                ECMD_HIDE + ECMD_SET_HELP,
                if prev_winid == (*curwin).handle {
                    curwin
                } else {
                    ptr::null_mut()
                },
            );
        } else {
            let fnum = (*qf_ptr).qf_fnum;

            if forceit == 0 && (*curwin).w_onebuf_opt.wo_wfb != 0 && (*curbuf).b_fnum != fnum {
                if (*qi).qfl_type == QFLT_LOCATION {
                    emsg(gettext((&raw const e_winfixbuf_cannot_go_to_buffer).cast()));
                    return FAIL;
                }

                if win_valid(prevwin)
                    && (*prevwin).w_onebuf_opt.wo_wfb == 0
                    && !bt_quickfix((*prevwin).w_buffer)
                {
                    win_goto(prevwin);
                }
                if (*curwin).w_onebuf_opt.wo_wfb != 0 {
                    if win_split(0, 0) == OK {
                        *opened_window = true;
                    }
                    if (*curwin).w_onebuf_opt.wo_wfb != 0 {
                        emsg(gettext((&raw const e_winfixbuf_cannot_go_to_buffer).cast()));
                        retval = FAIL;
                    }
                }
            }

            if retval == OK {
                retval = buflist_getfile(fnum, 1, GETF_SETMARK | GETF_SWITCH, forceit);
            }
        }

        if qfl_type == QFLT_LOCATION {
            let wp = win_id2wp(prev_winid);
            if wp.is_null() && (*curwin).w_llist.cast::<qf_info_T>() != qi {
                emsg(gettext(c"E924: Current window was closed".as_ptr()));
                *opened_window = false;
                return QF_ABORT;
            }
        }

        if qfl_type == QFLT_QUICKFIX && !qflist_valid(ptr::null_mut(), save_qfid) {
            emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED.as_ptr()));
            return QF_ABORT;
        }

        if old_qf_curlist != (*qi).qf_curlist
            || old_changetick != (*qfl).qf_changedtick
            || !is_qf_entry_present(qfl, qf_ptr)
        {
            if qfl_type == QFLT_QUICKFIX {
                emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED.as_ptr()));
            } else {
                emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED.as_ptr()));
            }
            return QF_ABORT;
        }

        retval
    }
}

unsafe fn qf_screen_col_to_idx(vcol: colnr_T) -> c_int {
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

unsafe fn qf_jump_goto_line(
    qf_lnum: linenr_T,
    qf_col: c_int,
    qf_viscol: c_char,
    qf_pattern: *mut c_char,
) {
    unsafe {
        if qf_pattern.is_null() {
            let mut i = qf_lnum;
            if i > 0 {
                i = i.min((*curbuf).b_ml.ml_line_count);
                (*curwin).w_cursor.lnum = i;
            }
            if qf_col > 0 {
                (*curwin).w_cursor.coladd = 0;
                if qf_viscol != 0 {
                    (*curwin).w_cursor.col = qf_screen_col_to_idx(qf_col - 1);
                } else {
                    (*curwin).w_cursor.col = qf_col - 1;
                }
                (*curwin).w_set_curswant = true;
                check_cursor(curwin);
            } else {
                beginline(BL_WHITE | BL_FIX);
            }
        } else {
            let save_cursor = (*curwin).w_cursor;
            (*curwin).w_cursor.lnum = 0;
            if do_search(
                ptr::null_mut(),
                b'/' as c_int,
                b'/' as c_int,
                qf_pattern,
                c_strlen(qf_pattern),
                1,
                SEARCH_KEEP,
                p_magic != 0,
                ptr::null_mut(),
            ) == 0
            {
                (*curwin).w_cursor = save_cursor;
            }
        }
    }
}

unsafe fn qf_jump_print_msg(
    qi: *mut qf_info_T,
    qf_index: c_int,
    qf_ptr: *mut qfline_T,
    old_curbuf: *mut buf_T,
    old_lnum: linenr_T,
) {
    unsafe {
        let gap = qfga_get();

        if msg_scrolled == 0 {
            update_topline(curwin);
            if must_redraw != 0 {
                update_screen();
            }
        }
        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
        let deleted_str = if (*qf_ptr).qf_cleared != 0 {
            gettext(c" (line deleted)".as_ptr()).cast_const()
        } else {
            c"".as_ptr()
        };
        let iobufflen = vim_snprintf_safelen!(
            iobuff_ptr,
            IOSIZE,
            gettext(c"(%d of %d)%s%s: ".as_ptr()),
            qf_index,
            (*qf_get_curlist(qi)).qf_count,
            deleted_str,
            qf_types(c_int::from((*qf_ptr).qf_type), (*qf_ptr).qf_nr),
        );
        ga_concat_len(gap, iobuff_ptr, iobufflen);
        qf_fmt_text(gap, skipwhite((*qf_ptr).qf_text));
        ga_append(gap, NUL as u8);

        let i = msg_scroll;
        if curbuf == old_curbuf && (*curwin).w_cursor.lnum == old_lnum {
            msg_scroll = 1;
        } else if (msg_scrolled == 0 || (p_ch == 0 && msg_scrolled == 1)) && shortmess(kShmOverall)
        {
            msg_scroll = 0;
        }
        msg_ext_set_kind(c"quickfix".as_ptr());
        msg_keep((*gap).ga_data.cast::<c_char>(), 0, true, false);
        msg_scroll = i;

        qfga_clear();
    }
}

unsafe fn qf_jump_open_window(
    qi: *mut qf_info_T,
    qf_ptr: *mut qfline_T,
    newwin: bool,
    opened_window: *mut bool,
) -> c_int {
    unsafe {
        let qfl = qf_get_curlist(qi);
        let old_changetick = (*qfl).qf_changedtick;
        let old_qf_curlist = (*qi).qf_curlist;
        let qfl_type = (*qfl).qfl_type;

        if (*qf_ptr).qf_type == 1
            && (!bt_help((*curwin).w_buffer) || cmdmod.cmod_tab != 0)
            && jump_to_help_window(qi, newwin, opened_window) == FAIL
        {
            return FAIL;
        }
        if old_qf_curlist != (*qi).qf_curlist
            || old_changetick != (*qfl).qf_changedtick
            || !is_qf_entry_present(qfl, qf_ptr)
        {
            if qfl_type == QFLT_QUICKFIX {
                emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED.as_ptr()));
            } else {
                emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED.as_ptr()));
            }
            return QF_ABORT;
        }

        if bt_quickfix(curbuf) && !*opened_window {
            if (*qf_ptr).qf_fnum == 0 {
                return NOTDONE;
            }

            if qf_jump_to_usable_window((*qf_ptr).qf_fnum, newwin, opened_window) == FAIL {
                return FAIL;
            }
        }
        if old_qf_curlist != (*qi).qf_curlist
            || old_changetick != (*qfl).qf_changedtick
            || !is_qf_entry_present(qfl, qf_ptr)
        {
            if qfl_type == QFLT_QUICKFIX {
                emsg(gettext(E_CURRENT_QUICKFIX_LIST_WAS_CHANGED.as_ptr()));
            } else {
                emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED.as_ptr()));
            }
            return QF_ABORT;
        }

        OK
    }
}

unsafe fn qf_jump_to_buffer(
    qi: *mut qf_info_T,
    qf_index: c_int,
    qf_ptr: *mut qfline_T,
    forceit: c_int,
    prev_winid: c_int,
    opened_window: *mut bool,
    openfold: c_int,
    print_message: bool,
) -> c_int {
    unsafe {
        let old_curbuf = curbuf;
        let old_lnum = (*curwin).w_cursor.lnum;
        let mut retval = OK;

        if (*qf_ptr).qf_fnum != 0 {
            retval = qf_jump_edit_buffer(qi, qf_ptr, forceit, prev_winid, opened_window);
            if retval != OK {
                return retval;
            }
        }

        if curbuf == old_curbuf {
            setpcmark();
        }

        qf_jump_goto_line(
            (*qf_ptr).qf_lnum,
            (*qf_ptr).qf_col,
            (*qf_ptr).qf_viscol,
            (*qf_ptr).qf_pattern,
        );

        if (fdo_flags & kOptFdoFlagQuickfix) != 0 && openfold != 0 {
            foldOpenCursor();
        }
        if print_message {
            qf_jump_print_msg(qi, qf_index, qf_ptr, old_curbuf, old_lnum);
        }

        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_jump(qi: *mut qf_info_T, dir: c_int, errornr: c_int, forceit: c_int) {
    unsafe {
        qf_jump_newwin(qi, dir, errornr, forceit, false);
    }
}

unsafe fn qf_jump_newwin(
    mut qi: *mut qf_info_T,
    dir: c_int,
    errornr: c_int,
    forceit: c_int,
    newwin: bool,
) {
    unsafe {
        let old_swb = p_swb;
        let old_swb_flags = swb_flags;
        let old_KeyTyped = KeyTyped;

        if qi.is_null() {
            assert!(!ql_info.is_null());
            qi = ql_info;
        }

        if qf_stack_empty(qi) || qf_list_empty(qf_get_curlist(qi)) {
            emsg(gettext((&raw const e_no_errors).cast()));
            return;
        }

        incr_quickfix_busy();

        let qfl = qf_get_curlist(qi);

        let mut qf_ptr = (*qfl).qf_ptr;
        let old_qf_ptr = qf_ptr;
        let mut qf_index = (*qfl).qf_index;
        let old_qf_index = qf_index;

        qf_ptr = qf_get_entry(qfl, errornr, dir, &raw mut qf_index);
        if !qf_ptr.is_null() {
            (*qfl).qf_index = qf_index;
            (*qfl).qf_ptr = qf_ptr;

            let print_message = !qf_win_pos_update(qi, old_qf_index);
            let prev_winid = (*curwin).handle;
            let mut opened_window = false;
            let mut retval = qf_jump_open_window(qi, qf_ptr, newwin, &raw mut opened_window);
            if retval == FAIL {
                qf_ptr = old_qf_ptr;
                qf_index = old_qf_index;
            } else if retval == QF_ABORT {
                qi = ptr::null_mut();
                qf_ptr = ptr::null_mut();
            } else if retval != NOTDONE {
                retval = qf_jump_to_buffer(
                    qi,
                    qf_index,
                    qf_ptr,
                    forceit,
                    prev_winid,
                    &raw mut opened_window,
                    c_int::from(old_KeyTyped),
                    print_message,
                );
                if retval == QF_ABORT {
                    qi = ptr::null_mut();
                    qf_ptr = ptr::null_mut();
                }

                if retval != OK {
                    if opened_window {
                        win_close(curwin, true, false);
                    }
                    if !qf_ptr.is_null() && (*qf_ptr).qf_fnum != 0 {
                        qf_ptr = old_qf_ptr;
                        qf_index = old_qf_index;
                    }
                }
            }
        } else {
            qf_ptr = old_qf_ptr;
            qf_index = old_qf_index;
        }

        if !qi.is_null() {
            (*qfl).qf_ptr = qf_ptr;
            (*qfl).qf_index = qf_index;
        }
        if p_swb != old_swb && p_swb == (&raw mut empty_string_option).cast::<c_char>() {
            p_swb = old_swb;
            swb_flags = old_swb_flags;
        }
        decr_quickfix_busy();
    }
}

unsafe fn qf_list_entry(qfp: *mut qfline_T, qf_idx: c_int, cursel: bool) {
    unsafe {
        let mut fname: *mut c_char = ptr::null_mut();
        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
        if !(*qfp).qf_module.is_null() && *(*qfp).qf_module != NUL {
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                c"%2d %s".as_ptr(),
                qf_idx,
                (*qfp).qf_module,
            );
        } else {
            if (*qfp).qf_fnum != 0 {
                let buf = buflist_findnr((*qfp).qf_fnum);
                if !buf.is_null() {
                    fname = if (*qfp).qf_fname.is_null() {
                        (*buf).b_fname
                    } else {
                        (*qfp).qf_fname
                    };
                    if (*qfp).qf_type == 1 {
                        fname = path_tail(fname);
                    }
                }
            }
            if fname.is_null() {
                vim_snprintf!(iobuff_ptr, IOSIZE, c"%2d".as_ptr(), qf_idx);
            } else {
                vim_snprintf!(iobuff_ptr, IOSIZE, c"%2d %s".as_ptr(), qf_idx, fname);
            }
        }

        let mut filter_entry = true;
        if !(*qfp).qf_module.is_null() && *(*qfp).qf_module != NUL {
            filter_entry &= message_filtered((*qfp).qf_module);
        }
        if filter_entry && !fname.is_null() {
            filter_entry &= message_filtered(fname);
        }
        if filter_entry && !(*qfp).qf_pattern.is_null() {
            filter_entry &= message_filtered((*qfp).qf_pattern);
        }
        if filter_entry {
            filter_entry &= message_filtered((*qfp).qf_text);
        }
        if filter_entry {
            return;
        }

        if msg_col > 0 {
            msg_putchar(b'\n' as c_int);
        }
        msg_outtrans(
            iobuff_ptr,
            if cursel { HLF_QFL } else { qfFile_hl_id },
            false,
        );

        if (*qfp).qf_lnum != 0 {
            msg_puts_hl(c":".as_ptr(), qfSep_hl_id, false);
        }
        let mut gap = qfga_get();
        if (*qfp).qf_lnum != 0 {
            qf_range_text(gap, qfp);
        }
        ga_concat(gap, qf_types(c_int::from((*qfp).qf_type), (*qfp).qf_nr));
        ga_append(gap, NUL as u8);
        if *(*gap).ga_data.cast::<c_char>() != NUL {
            msg_puts_hl((*gap).ga_data.cast::<c_char>(), qfLine_hl_id, false);
        }
        msg_puts_hl(c":".as_ptr(), qfSep_hl_id, false);
        if !(*qfp).qf_pattern.is_null() {
            gap = qfga_get();
            qf_fmt_text(gap, (*qfp).qf_pattern);
            ga_append(gap, NUL as u8);
            msg_puts((*gap).ga_data.cast::<c_char>());
            msg_puts_hl(c":".as_ptr(), qfSep_hl_id, false);
        }
        msg_puts(c" ".as_ptr());

        gap = qfga_get();
        let text = if !fname.is_null() || (*qfp).qf_lnum != 0 {
            skipwhite((*qfp).qf_text)
        } else {
            (*qfp).qf_text
        };
        qf_fmt_text(gap, text);
        ga_append(gap, NUL as u8);
        msg_prt_line((*gap).ga_data.cast::<c_char>(), false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_list(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
        let mut all = (*eap).forceit != 0;
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        if qf_stack_empty(qi) || qf_list_empty(qf_get_curlist(qi)) {
            emsg(gettext((&raw const e_no_errors).cast()));
            return;
        }

        let mut plus = false;
        if *arg == b'+' as c_char {
            arg = arg.add(1);
            plus = true;
        }
        let mut idx1: c_int = 1;
        let mut idx2: c_int = -1;
        if get_list_range(&raw mut arg, &raw mut idx1, &raw mut idx2) == FAIL || *arg != NUL {
            semsg!(gettext((&raw const e_trailing_arg).cast()), arg);
            return;
        }
        let qfl = qf_get_curlist(qi);
        if plus {
            let i = (*qfl).qf_index;
            idx2 = i + idx1;
            idx1 = i;
        } else {
            let i = (*qfl).qf_count;
            if idx1 < 0 {
                idx1 = if -idx1 > i { 0 } else { idx1 + i + 1 };
            }
            if idx2 < 0 {
                idx2 = if -idx2 > i { 0 } else { idx2 + i + 1 };
            }
        }

        shorten_fnames(0);

        qfFile_hl_id = syn_name2id(c"qfFileName".as_ptr());
        if qfFile_hl_id == 0 {
            qfFile_hl_id = HLF_D;
        }
        qfSep_hl_id = syn_name2id(c"qfSeparator".as_ptr());
        if qfSep_hl_id == 0 {
            qfSep_hl_id = HLF_D;
        }
        qfLine_hl_id = syn_name2id(c"qfLineNr".as_ptr());
        if qfLine_hl_id == 0 {
            qfLine_hl_id = HLF_N;
        }

        if (*qfl).qf_nonevalid {
            all = true;
        }
        msg_ext_set_kind(c"list_cmd".as_ptr());
        let mut i: c_int = 1;
        let mut qfp = (*qfl).qf_start;
        while !got_int && i <= (*qfl).qf_count && !qfp.is_null() {
            if ((*qfp).qf_valid != 0 || all) && idx1 <= i && i <= idx2 {
                qf_list_entry(qfp, i, i == (*qfl).qf_index);
            }
            os_breakcheck();
            i += 1;
            qfp = (*qfp).qf_next;
        }
        qfga_clear();
    }
}

unsafe fn qf_fmt_text(gap: *mut garray_T, text: *const c_char) {
    unsafe {
        let mut p = text;
        while *p != NUL {
            if *p == b'\n' as c_char {
                ga_append(gap, b' ');
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
                ga_append(gap, *p as u8);
                p = p.add(1);
            }
        }
    }
}

unsafe fn qf_range_text(gap: *mut garray_T, qfp: *const qfline_T) {
    unsafe {
        let buf_data = (&raw mut IObuff).cast::<c_char>();
        let mut buf_size = vim_snprintf_safelen!(buf_data, IOSIZE, c"%d".as_ptr(), (*qfp).qf_lnum);

        if (*qfp).qf_end_lnum > 0 && (*qfp).qf_lnum != (*qfp).qf_end_lnum {
            buf_size += vim_snprintf_safelen!(
                buf_data.add(buf_size),
                IOSIZE - buf_size,
                c"-%d".as_ptr(),
                (*qfp).qf_end_lnum,
            );
        }
        if (*qfp).qf_col > 0 {
            buf_size += vim_snprintf_safelen!(
                buf_data.add(buf_size),
                IOSIZE - buf_size,
                c" col %d".as_ptr(),
                (*qfp).qf_col,
            );
            if (*qfp).qf_end_col > 0 && (*qfp).qf_col != (*qfp).qf_end_col {
                buf_size += vim_snprintf_safelen!(
                    buf_data.add(buf_size),
                    IOSIZE - buf_size,
                    c"-%d".as_ptr(),
                    (*qfp).qf_end_col,
                );
            }
        }

        ga_concat_len(gap, buf_data, buf_size);
    }
}

unsafe fn qf_msg(qi: *mut qf_info_T, which: c_int, lead: *const c_char) {
    unsafe {
        let title = (*(*qi).qf_lists.add(which as usize)).qf_title;
        let count = (*(*qi).qf_lists.add(which as usize)).qf_count;
        let mut buf = [0 as c_char; IOSIZE];

        vim_snprintf!(
            buf.as_mut_ptr(),
            IOSIZE,
            gettext(c"%serror list %d of %d; %d errors ".as_ptr()),
            lead,
            which + 1,
            (*qi).qf_listcount,
            count,
        );

        if !title.is_null() {
            let len = c_strlen(buf.as_ptr());
            if len < 34 {
                ptr::write_bytes(buf.as_mut_ptr().add(len), b' ', 34 - len);
                buf[34] = NUL;
            }
            xstrlcat(buf.as_mut_ptr(), title, IOSIZE);
        }
        trunc_string(buf.as_ptr(), buf.as_mut_ptr(), Columns - 1, IOSIZE as c_int);
        msg(buf.as_ptr(), 0);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_age(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let mut count = if (*eap).addr_count != 0 {
            (*eap).line2 as c_int
        } else {
            1
        };
        while count > 0 {
            count -= 1;
            if (*eap).cmdidx == CMD_colder || (*eap).cmdidx == CMD_lolder {
                if (*qi).qf_curlist == 0 {
                    emsg(gettext(c"E380: At bottom of quickfix stack".as_ptr()));
                    break;
                }
                (*qi).qf_curlist -= 1;
            } else {
                if (*qi).qf_curlist >= (*qi).qf_listcount - 1 {
                    emsg(gettext(c"E381: At top of quickfix stack".as_ptr()));
                    break;
                }
                (*qi).qf_curlist += 1;
            }
        }
        qf_msg(qi, (*qi).qf_curlist, c"".as_ptr());
        qf_update_buffer(qi, ptr::null_mut());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_history(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);

        if (*eap).addr_count > 0 {
            if qi.is_null() {
                emsg(gettext((&raw const e_loclist).cast()));
                return;
            }

            if (*eap).line2 > 0 && (*eap).line2 <= (*qi).qf_listcount {
                (*qi).qf_curlist = (*eap).line2 - 1;
                qf_msg(qi, (*qi).qf_curlist, c"".as_ptr());
                qf_update_buffer(qi, ptr::null_mut());
            } else {
                emsg(gettext((&raw const e_invrange).cast()));
            }
            return;
        }

        if qf_stack_empty(qi) {
            msg(gettext(c"No entries".as_ptr()), 0);
        } else {
            for i in 0..(*qi).qf_listcount {
                let lead = if i == (*qi).qf_curlist {
                    c"> ".as_ptr()
                } else {
                    c"  ".as_ptr()
                };
                qf_msg(qi, i, lead);
            }
        }
    }
}

unsafe fn qf_free_items(qfl: *mut qf_list_T) {
    unsafe {
        let mut stop = false;

        while (*qfl).qf_count != 0 && !(*qfl).qf_start.is_null() {
            let qfp = (*qfl).qf_start;
            let qfpnext = (*qfp).qf_next;
            if !stop {
                xfree((*qfp).qf_fname.cast());
                xfree((*qfp).qf_module.cast());
                xfree((*qfp).qf_text.cast());
                xfree((*qfp).qf_pattern.cast());
                tv_clear(&raw mut (*qfp).qf_user_data);
                stop = qfp == qfpnext;
                xfree(qfp.cast());
                if stop {
                    (*qfl).qf_count = 1;
                } else {
                    (*qfl).qf_start = qfpnext;
                }
            }
            (*qfl).qf_count -= 1;
        }

        (*qfl).qf_index = 0;
        (*qfl).qf_start = ptr::null_mut();
        (*qfl).qf_last = ptr::null_mut();
        (*qfl).qf_ptr = ptr::null_mut();
        (*qfl).qf_nonevalid = true;

        qf_clean_dir_stack(&raw mut (*qfl).qf_dir_stack);
        (*qfl).qf_directory = ptr::null_mut();
        qf_clean_dir_stack(&raw mut (*qfl).qf_file_stack);
        (*qfl).qf_currfile = ptr::null_mut();
        (*qfl).qf_multiline = false;
        (*qfl).qf_multiignore = false;
        (*qfl).qf_multiscan = false;
    }
}

unsafe fn qf_free(qfl: *mut qf_list_T) {
    unsafe {
        qf_free_items(qfl);

        xfree((*qfl).qf_title.cast());
        (*qfl).qf_title = ptr::null_mut();
        tv_free((*qfl).qf_ctx);
        (*qfl).qf_ctx = ptr::null_mut();
        callback_free(&raw mut (*qfl).qf_qftf_cb);
        (*qfl).qf_id = 0;
        (*qfl).qf_changedtick = 0;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_mark_adjust(
    buf: *mut buf_T,
    wp: *mut win_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) -> bool {
    unsafe {
        let mut qi = ql_info;
        assert!(!qi.is_null());
        let buf_has_flag = if wp.is_null() {
            BUF_HAS_QF_ENTRY
        } else {
            BUF_HAS_LL_ENTRY
        };

        if ((*buf).b_has_qf_entry & buf_has_flag) == 0 {
            return false;
        }
        if !wp.is_null() {
            if (*wp).w_llist.is_null() {
                return false;
            }
            qi = (*wp).w_llist.cast::<qf_info_T>();
        }

        let mut found_one = false;
        for idx in 0..(*qi).qf_listcount {
            let qfl = qf_get_list(qi, idx);
            if !qf_list_empty(qfl) {
                let mut i: c_int = 1;
                let mut qfp = (*qfl).qf_start;
                while !got_int && i <= (*qfl).qf_count && !qfp.is_null() {
                    if (*qfp).qf_fnum == (*buf).b_fnum {
                        found_one = true;
                        if (*qfp).qf_cleared == 0 {
                            if (*qfp).qf_lnum >= line1 && (*qfp).qf_lnum <= line2 {
                                if amount == MAXLNUM {
                                    (*qfp).qf_cleared = 1;
                                } else {
                                    (*qfp).qf_lnum += amount;
                                }
                            } else if amount_after != 0 && (*qfp).qf_lnum > line2 {
                                (*qfp).qf_lnum += amount_after;
                            }
                        }
                    }
                    i += 1;
                    qfp = (*qfp).qf_next;
                }
            }
        }

        found_one
    }
}

unsafe fn qf_types(c: c_int, nr: c_int) -> *const c_char {
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
            let cc = (&raw mut qf_types_cc).cast::<c_char>();
            *cc = b' ' as c_char;
            *cc.add(1) = c as c_char;
            *cc.add(2) = NUL;
            cc
        };

        if nr <= 0 {
            return p;
        }

        let buf = (&raw mut qf_types_buf).cast::<c_char>();
        vim_snprintf!(buf, 20, c"%s %3d".as_ptr(), p, nr);
        buf
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_view_result(split: bool) {
    unsafe {
        let mut qi = ql_info;
        assert!(!qi.is_null());

        if is_ll_window(curwin) {
            qi = get_loc_list(curwin);
        }

        if qf_list_empty(qf_get_curlist(qi)) {
            emsg(gettext((&raw const e_no_errors).cast()));
            return;
        }

        if split {
            qf_jump_newwin(qi, 0, (*curwin).w_cursor.lnum as c_int, 0, true);
            do_cmdline_cmd(c"clearjumps".as_ptr());
            return;
        }

        do_cmdline_cmd(if is_ll_window(curwin) {
            c".ll".as_ptr()
        } else {
            c".cc".as_ptr()
        });
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cwindow(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let qfl = qf_get_curlist(qi);
        let win = qf_find_win(qi);

        if qf_stack_empty(qi) || (*qfl).qf_nonevalid || qf_list_empty(qfl) {
            if !win.is_null() {
                ex_cclose(eap);
            }
        } else if win.is_null() {
            ex_copen(eap);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cclose(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return;
        }

        let win = qf_find_win(qi);
        if !win.is_null() {
            win_close(win, false, false);
        }
    }
}

unsafe fn qf_goto_cwindow(qi: *const qf_info_T, resize: bool, sz: c_int, vertsplit: bool) -> c_int {
    unsafe {
        let win = qf_find_win(qi);
        if win.is_null() {
            return FAIL;
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
                    < cmdline_row)
            {
                win_setheight(sz);
            }
        }

        OK
    }
}

unsafe fn qf_set_cwindow_options() {
    unsafe {
        set_option_value_give_err(kOptSwapfile, boolean_obj(false), OPT_LOCAL);
        set_option_value_give_err(kOptBuftype, cstr_as_obj(c"quickfix".as_ptr()), OPT_LOCAL);
        set_option_value_give_err(kOptBufhidden, cstr_as_obj(c"hide".as_ptr()), OPT_LOCAL);
        reset_binding(curwin);
        (*curwin).w_onebuf_opt.wo_diff = 0;
        set_option_value_give_err(kOptFoldmethod, cstr_as_obj(c"manual".as_ptr()), OPT_LOCAL);
    }
}

unsafe fn qf_open_new_cwindow(qi: *mut qf_info_T, height: c_int) -> c_int {
    unsafe {
        let mut oldwin = curwin;
        let prevtab = curtab;
        let mut flags: c_int = 0;

        let qf_buf = qf_find_buf(qi);
        let win = curwin;

        if cmdmod.cmod_split == 0 {
            flags = if is_qf_stack(qi) { WSP_BOT } else { WSP_BELOW };
        }
        flags |= WSP_NEWLOC;

        if is_qf_stack(qi) {
            flags |= WSP_QUICKFIX;
        }

        if win_split(height, flags) == FAIL {
            return FAIL;
        }
        reset_binding(curwin);

        if is_ll_stack(qi) {
            (*curwin).w_llist_ref = qi.cast::<c_void>();
            (*qi).qf_refcount += 1;
        }

        if oldwin != curwin {
            oldwin = ptr::null_mut();
        }
        if !qf_buf.is_null() {
            if do_ecmd(
                (*qf_buf).b_fnum,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ECMD_ONE,
                ECMD_HIDE + ECMD_OLDBUF + ECMD_NOWINENTER,
                oldwin,
            ) == FAIL
            {
                return FAIL;
            }
        } else {
            if do_ecmd(
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ECMD_ONE,
                ECMD_HIDE + ECMD_NOWINENTER,
                oldwin,
            ) == FAIL
            {
                return FAIL;
            }
            (*qi).qf_bufnr = (*curbuf).b_fnum;
        }

        if !bt_quickfix(curbuf) {
            qf_set_cwindow_options();
        }

        if curtab == prevtab && (*curwin).w_width == Columns {
            win_setheight(height);
        }
        (*curwin).w_onebuf_opt.wo_wfh = 1;
        if win_valid(win) {
            prevwin = win;
        }
        OK
    }
}

unsafe fn qf_set_title_var(qfl: *mut qf_list_T) {
    unsafe {
        if !(*qfl).qf_title.is_null() {
            set_internal_string_var(c"w:quickfix_title".as_ptr(), (*qfl).qf_title);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_copen(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        incr_quickfix_busy();

        let height = if (*eap).addr_count != 0 {
            (*eap).line2 as c_int
        } else {
            QF_WINHEIGHT
        };
        reset_VIsual_and_resel();

        let mut status = FAIL;
        if cmdmod.cmod_tab == 0 {
            status = qf_goto_cwindow(
                qi,
                (*eap).addr_count != 0,
                height,
                (cmdmod.cmod_split & WSP_VERT) != 0,
            );
        }
        if status == FAIL && qf_open_new_cwindow(qi, height) == FAIL {
            decr_quickfix_busy();
            return;
        }

        let qfl = qf_get_curlist(qi);
        qf_set_title_var(qfl);
        let lnum = (*qfl).qf_index;

        qf_fill_buffer(qfl, curbuf, ptr::null_mut(), (*curwin).handle);

        decr_quickfix_busy();

        (*curwin).w_cursor.lnum = lnum;
        (*curwin).w_cursor.col = 0;
        check_cursor(curwin);
        update_topline(curwin);
    }
}

unsafe fn qf_win_goto(win: *mut win_T, lnum: linenr_T) {
    unsafe {
        let old_curwin = curwin;

        curwin = win;
        curbuf = (*win).w_buffer;
        (*curwin).w_cursor.lnum = lnum;
        (*curwin).w_cursor.col = 0;
        (*curwin).w_cursor.coladd = 0;
        (*curwin).w_curswant = 0;
        update_topline(curwin);
        redraw_later(curwin, UPD_VALID);
        (*curwin).w_redr_status = true;
        curwin = old_curwin;
        curbuf = (*curwin).w_buffer;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cbottom(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let win = qf_find_win(qi);
        if !win.is_null() && (*win).w_cursor.lnum != (*(*win).w_buffer).b_ml.ml_line_count {
            qf_win_goto(win, (*(*win).w_buffer).b_ml.ml_line_count);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_current_entry(wp: *mut win_T) -> linenr_T {
    unsafe {
        let mut qi = ql_info;
        assert!(!qi.is_null());

        if is_ll_window(wp) {
            qi = (*wp).w_llist_ref.cast::<qf_info_T>();
        }

        (*qf_get_curlist(qi)).qf_index
    }
}

unsafe fn qf_win_pos_update(qi: *mut qf_info_T, old_qf_index: c_int) -> bool {
    unsafe {
        let qf_index = (*qf_get_curlist(qi)).qf_index;

        let win = qf_find_win(qi);
        if !win.is_null()
            && qf_index <= (*(*win).w_buffer).b_ml.ml_line_count
            && old_qf_index != qf_index
        {
            (*win).w_redraw_top = old_qf_index.min(qf_index);
            (*win).w_redraw_bot = old_qf_index.max(qf_index);
            qf_win_goto(win, qf_index);
        }
        !win.is_null()
    }
}

unsafe fn is_qf_win(win: *const win_T, qi: *const qf_info_T) -> bool {
    unsafe {
        if buf_valid((*win).w_buffer)
            && bt_quickfix((*win).w_buffer)
            && ((is_qf_stack(qi) && (*win).w_llist_ref.is_null())
                || (is_ll_stack(qi) && (*win).w_llist_ref.cast::<qf_info_T>().cast_const() == qi))
        {
            return true;
        }
        false
    }
}

unsafe fn qf_find_win(qi: *const qf_info_T) -> *mut win_T {
    unsafe {
        let mut win = tab_firstwin(curtab);
        while !win.is_null() {
            if is_qf_win(win, qi) {
                return win;
            }
            win = (*win).w_next;
        }
        ptr::null_mut()
    }
}

unsafe fn qf_find_buf(qi: *mut qf_info_T) -> *mut buf_T {
    unsafe {
        if (*qi).qf_bufnr != INVALID_QFBUFNR {
            let qfbuf = buflist_findnr((*qi).qf_bufnr);
            if !qfbuf.is_null() {
                return qfbuf;
            }
            (*qi).qf_bufnr = INVALID_QFBUFNR;
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut win = tab_firstwin(tp);
            while !win.is_null() {
                if is_qf_win(win, qi) {
                    return (*win).w_buffer;
                }
                win = (*win).w_next;
            }
            tp = (*tp).tp_next;
        }

        ptr::null_mut()
    }
}

unsafe fn qf_update_win_titlevar(qi: *mut qf_info_T) {
    unsafe {
        let qfl = qf_get_curlist(qi);
        let save_curwin = curwin;

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut win = tab_firstwin(tp);
            while !win.is_null() {
                if is_qf_win(win, qi) {
                    curwin = win;
                    qf_set_title_var(qfl);
                }
                win = (*win).w_next;
            }
            tp = (*tp).tp_next;
        }
        curwin = save_curwin;
    }
}

unsafe fn qf_update_buffer(qi: *mut qf_info_T, old_last: *mut qfline_T) {
    unsafe {
        let buf = qf_find_buf(qi);
        if buf.is_null() {
            return;
        }

        let old_line_count = (*buf).b_ml.ml_line_count;
        let old_endcol = ml_get_buf_len(buf, old_line_count);
        let old_bytecount = get_region_bytecount(buf, 1, old_line_count, 0, old_endcol);
        let mut qf_winid: c_int = 0;

        let mut win: *mut win_T;
        if is_ll_stack(qi) {
            if (*curwin).w_llist.cast::<qf_info_T>() == qi {
                win = curwin;
            } else {
                win = qf_find_win_with_loclist(qi);
                if win.is_null() {
                    win = qf_find_win(qi);
                }
                if win.is_null() {
                    return;
                }
            }
            qf_winid = (*win).handle;
        }

        incr_quickfix_busy();

        let mut aco: CtxSwitch = std::mem::zeroed();
        if old_last.is_null() {
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
        }

        qf_update_win_titlevar(qi);

        qf_fill_buffer(qf_get_curlist(qi), buf, old_last, qf_winid);

        let new_line_count = (*buf).b_ml.ml_line_count;
        let new_endcol = ml_get_buf_len(buf, new_line_count);
        let delta = new_line_count - old_line_count;

        if old_last.is_null() {
            let new_byte_count = get_region_bytecount(buf, 1, new_line_count, 0, new_endcol);
            extmark_splice(
                buf,
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
                buf,
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
                get_region_bytecount(buf, start_lnum, new_line_count, 0, new_endcol);
            extmark_splice(
                buf,
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
            changed_lines(buf, start_lnum, 0, start_lnum, delta, true);
        }
        (*buf).b_changed = 0;

        if old_last.is_null() {
            qf_win_pos_update(qi, 0);
            ctx_restore(&raw mut aco);
        }

        win = qf_find_win(qi);
        if !win.is_null() && old_line_count < (*win).w_botline {
            redraw_buf_later(buf, UPD_NOT_VALID);
        }

        decr_quickfix_busy();
    }
}

unsafe fn qf_buf_add_line(
    _qfl: *mut qf_list_T,
    buf: *mut buf_T,
    lnum: linenr_T,
    qfp: *const qfline_T,
    dirname: *mut c_char,
    qftf_str: *mut c_char,
    first_bufline: bool,
) -> c_int {
    unsafe {
        let gap = qfga_get();

        if !qftf_str.is_null() && *qftf_str != NUL {
            ga_concat(gap, qftf_str);
        } else {
            if !(*qfp).qf_module.is_null() {
                ga_concat(gap, (*qfp).qf_module);
            } else if (*qfp).qf_fnum != 0 {
                let errbuf = buflist_findnr((*qfp).qf_fnum);
                if !errbuf.is_null() && !(*errbuf).b_fname.is_null() {
                    if (*qfp).qf_type == 1 {
                        ga_concat(gap, path_tail((*errbuf).b_fname));
                    } else {
                        if first_bufline
                            && ((*errbuf).b_sfname.is_null()
                                || path_is_absolute((*errbuf).b_sfname))
                        {
                            if *dirname == NUL {
                                os_dirname(dirname, MAXPATHL);
                            }
                            shorten_buf_fname(errbuf, dirname, 0);
                        }
                        ga_concat(
                            gap,
                            if (*qfp).qf_fname.is_null() {
                                (*errbuf).b_fname
                            } else {
                                (*qfp).qf_fname
                            },
                        );
                    }
                }
            }

            ga_append(gap, b'|');

            if (*qfp).qf_lnum > 0 {
                qf_range_text(gap, qfp);
                ga_concat(gap, qf_types(c_int::from((*qfp).qf_type), (*qfp).qf_nr));
            } else if !(*qfp).qf_pattern.is_null() {
                qf_fmt_text(gap, (*qfp).qf_pattern);
            }
            ga_append(gap, b'|');
            ga_append(gap, b' ');

            qf_fmt_text(
                gap,
                if (*gap).ga_len > 3 {
                    skipwhite((*qfp).qf_text)
                } else {
                    (*qfp).qf_text
                },
            );
        }

        ga_append(gap, NUL as u8);
        if ml_append_buf(
            buf,
            lnum,
            (*gap).ga_data.cast::<c_char>(),
            (*gap).ga_len,
            false,
        ) == FAIL
        {
            return FAIL;
        }

        OK
    }
}

unsafe fn call_qftf_func(
    qfl: *mut qf_list_T,
    qf_winid: c_int,
    start_idx: c_int,
    end_idx: c_int,
) -> *mut list_T {
    unsafe {
        let mut cb: *mut Callback = &raw mut p_qftf;
        let mut qftf_list: *mut list_T = ptr::null_mut();

        if qftf_recursive {
            return ptr::null_mut();
        }
        qftf_recursive = true;

        if (*qfl).qf_qftf_cb.cb_type != kCallbackNone {
            cb = &raw mut (*qfl).qf_qftf_cb;
        }
        if (*cb).cb_type != kCallbackNone {
            let mut args: [typval_T; 1] = std::mem::zeroed();
            let mut rettv: typval_T = std::mem::zeroed();

            let dict = tv_dict_alloc_lock(VAR_FIXED);
            tv_dict_add_nr(
                dict,
                c"quickfix".as_ptr(),
                8,
                varnumber_T::from(is_qf_list(qfl)),
            );
            tv_dict_add_nr(dict, c"winid".as_ptr(), 5, varnumber_T::from(qf_winid));
            tv_dict_add_nr(dict, c"id".as_ptr(), 2, varnumber_T::from((*qfl).qf_id));
            tv_dict_add_nr(dict, c"start_idx".as_ptr(), 9, varnumber_T::from(start_idx));
            tv_dict_add_nr(dict, c"end_idx".as_ptr(), 7, varnumber_T::from(end_idx));
            (*dict).dv_refcount += 1;
            args[0].v_type = VarType::VAR_DICT;
            args[0].vval.v_dict = dict;

            textlock += 1;
            if callback_call(cb, 1, args.as_mut_ptr(), &raw mut rettv) {
                if rettv.v_type == VarType::VAR_LIST {
                    qftf_list = rettv.vval.v_list;
                    tv_list_ref(qftf_list);
                }
                tv_clear(&raw mut rettv);
            }
            textlock -= 1;
            tv_dict_unref(dict);
        }

        qftf_recursive = false;
        qftf_list
    }
}

unsafe fn qf_fill_buffer(
    qfl: *mut qf_list_T,
    buf: *mut buf_T,
    old_last: *mut qfline_T,
    qf_winid: c_int,
) {
    unsafe {
        let old_KeyTyped = KeyTyped;

        if old_last.is_null() {
            if buf != curbuf {
                internal_error(c"qf_fill_buffer()".as_ptr());
                return;
            }

            while ((*curbuf).b_ml.ml_flags & ML_EMPTY) == 0 {
                if ml_delete(1) == FAIL {
                    internal_error(c"qf_fill_buffer()".as_ptr());
                    return;
                }
            }

            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = tab_firstwin(tp);
                while !wp.is_null() {
                    if (*wp).w_buffer == curbuf {
                        (*wp).w_skipcol = 0;
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }

            u_clearallandblockfree(curbuf);
        }

        if !qfl.is_null() && !(*qfl).qf_start.is_null() {
            let mut dirname = [0 as c_char; MAXPATHL];
            let mut lnum: linenr_T;
            let mut qfp: *mut qfline_T;

            if old_last.is_null() {
                qfp = (*qfl).qf_start;
                lnum = 0;
            } else {
                qfp = if !(*old_last).qf_next.is_null() {
                    (*old_last).qf_next
                } else {
                    old_last
                };
                lnum = (*buf).b_ml.ml_line_count;
            }

            let qftf_list = call_qftf_func(qfl, qf_winid, lnum + 1, (*qfl).qf_count);
            let mut qftf_li = tv_list_first(qftf_list);

            let mut prev_bufnr: c_int = -1;
            let mut invalid_val = false;

            while lnum < (*qfl).qf_count {
                let mut qftf_str: *mut c_char = ptr::null_mut();

                if !qftf_li.is_null() && !invalid_val {
                    qftf_str = tv_get_string_chk(&raw const (*qftf_li).li_tv).cast_mut();
                    if qftf_str.is_null() {
                        invalid_val = true;
                    }
                }

                if qf_buf_add_line(
                    qfl,
                    buf,
                    lnum,
                    qfp,
                    dirname.as_mut_ptr(),
                    qftf_str,
                    prev_bufnr != (*qfp).qf_fnum,
                ) == FAIL
                {
                    break;
                }
                prev_bufnr = (*qfp).qf_fnum;
                lnum += 1;
                qfp = (*qfp).qf_next;
                if qfp.is_null() {
                    break;
                }

                if !qftf_li.is_null() {
                    qftf_li = (*qftf_li).li_next;
                }
            }
            if old_last.is_null() {
                ml_delete(lnum + 1);
            }

            tv_list_unref(qftf_list);
            qfga_clear();
        }

        check_lnums(true);

        if old_last.is_null() {
            (*curbuf).b_ro_locked += 1;
            set_option_value_give_err(kOptFiletype, cstr_as_obj(c"qf".as_ptr()), OPT_LOCAL);
            (*curbuf).b_p_ma = 0;

            (*curbuf).b_keep_filetype = true;
            apply_autocmds(
                EVENT_BUFREADPOST,
                c"quickfix".as_ptr().cast_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            apply_autocmds(
                EVENT_BUFWINENTER,
                c"quickfix".as_ptr().cast_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            (*curbuf).b_keep_filetype = false;
            (*curbuf).b_ro_locked -= 1;

            redraw_curbuf_later(UPD_NOT_VALID);
        }

        KeyTyped = old_KeyTyped;
    }
}

#[inline]
unsafe fn qf_list_changed(qfl: *mut qf_list_T) {
    unsafe {
        (*qfl).qf_changedtick += 1;
    }
}

unsafe fn qf_id2nr(qi: *const qf_info_T, qfid: c_uint) -> c_int {
    unsafe {
        for qf_idx in 0..(*qi).qf_listcount {
            if (*(*qi).qf_lists.add(qf_idx as usize)).qf_id == qfid {
                return qf_idx;
            }
        }
        INVALID_QFIDX
    }
}

unsafe fn qf_restore_list(qi: *mut qf_info_T, save_qfid: c_uint) -> c_int {
    unsafe {
        if (*qf_get_curlist(qi)).qf_id == save_qfid {
            return OK;
        }

        let curlist = qf_id2nr(qi, save_qfid);
        if curlist < 0 {
            return FAIL;
        }
        (*qi).qf_curlist = curlist;
        OK
    }
}

unsafe fn qf_jump_first(qi: *mut qf_info_T, save_qfid: c_uint, forceit: c_int) {
    unsafe {
        if qf_restore_list(qi, save_qfid) == FAIL {
            return;
        }

        if !check_can_set_curbuf_forceit(forceit) {
            return;
        }

        if !qf_list_empty(qf_get_curlist(qi)) {
            qf_jump(qi, 0, 0, forceit);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn grep_internal(cmdidx: cmdidx_T) -> c_int {
    unsafe {
        let gp = if *(*curbuf).b_p_gp == NUL {
            p_gp
        } else {
            (*curbuf).b_p_gp
        };
        c_int::from(
            (cmdidx == CMD_grep
                || cmdidx == CMD_lgrep
                || cmdidx == CMD_grepadd
                || cmdidx == CMD_lgrepadd)
                && c_strcmp(c"internal".as_ptr(), gp) == 0,
        )
    }
}

unsafe fn make_get_auname(cmdidx: cmdidx_T) -> *const c_char {
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

unsafe fn make_get_fullcmd(makecmd: *const c_char, fname: *const c_char) -> *mut c_char {
    unsafe {
        let mut len = c_strlen(p_shq) * 2 + c_strlen(makecmd) + 1;
        if *p_sp != NUL {
            len += c_strlen(p_sp) + c_strlen(fname) + 3;
        }
        let cmd = xmalloc(len).cast::<c_char>();
        vim_snprintf!(cmd, len, c"%s%s%s".as_ptr(), p_shq, makecmd, p_shq);

        if *p_sp != NUL {
            append_redir(cmd, len, p_sp, fname);
        }

        if msg_col == 0 {
            msg_didout = false;
        }
        msg_start();
        msg_puts(c":!".as_ptr());
        msg_outtrans(cmd, 0, false);

        cmd
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_make(eap: *mut exarg_T) {
    unsafe {
        let enc = if *(*curbuf).b_p_menc != NUL {
            (*curbuf).b_p_menc
        } else {
            p_menc
        };

        if grep_internal((*eap).cmdidx) != 0 {
            ex_vimgrep(eap);
            return;
        }

        let au_name = make_get_auname((*eap).cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
            )
            && aborting()
        {
            return;
        }

        let mut wp: *mut win_T = ptr::null_mut();
        if is_loclist_cmd((*eap).cmdidx) {
            wp = curwin;
        }

        autowrite_all();
        let fname = get_mef_name();
        if fname.is_null() {
            return;
        }
        os_remove(fname);

        let cmd = make_get_fullcmd((*eap).arg, fname);

        do_shell(cmd, 0);

        incr_quickfix_busy();

        let errorformat = if (*eap).cmdidx != CMD_make && (*eap).cmdidx != CMD_lmake {
            if *(*curbuf).b_p_gefm != NUL {
                (*curbuf).b_p_gefm
            } else {
                p_gefm
            }
        } else {
            p_efm
        };

        let newlist = (*eap).cmdidx != CMD_grepadd && (*eap).cmdidx != CMD_lgrepadd;

        let res = qf_init(
            wp,
            fname,
            errorformat,
            c_int::from(newlist),
            qf_cmdtitle(*(*eap).cmdlinep),
            enc,
        );

        let mut qi = ql_info;
        assert!(!qi.is_null());
        if !wp.is_null() {
            qi = get_loc_list(wp);
        }
        if !qi.is_null() {
            if res >= 0 {
                qf_list_changed(qf_get_curlist(qi));
            }
            let save_qfid = (*qf_get_curlist(qi)).qf_id;
            if !au_name.is_null() {
                apply_autocmds(
                    EVENT_QUICKFIXCMDPOST,
                    au_name.cast_mut(),
                    (*curbuf).b_fname,
                    true,
                    curbuf,
                );
            }
            if res > 0 && (*eap).forceit == 0 && qflist_valid(wp, save_qfid) {
                qf_jump_first(qi, save_qfid, 0);
            }
        }

        decr_quickfix_busy();
        os_remove(fname);
        xfree(fname.cast());
        xfree(cmd.cast());
    }
}

unsafe fn get_mef_name() -> *mut c_char {
    unsafe {
        if *p_mef == NUL {
            let name = vim_tempname();
            if name.is_null() {
                emsg(gettext((&raw const e_notmp).cast()));
            }
            return name;
        }

        let mut p = p_mef;
        while *p != NUL {
            if *p == b'#' as c_char && *p.add(1) == b'#' as c_char {
                break;
            }
            p = p.add(1);
        }

        if *p == NUL {
            return xstrdup(p_mef);
        }

        loop {
            if mef_start == -1 {
                mef_start = os_get_pid() as c_int;
            } else {
                mef_off += 19;
            }
            let mef_len = c_strlen(p_mef);
            let name = xmalloc(mef_len + 30).cast::<c_char>();
            c_strcpy(name, p_mef);
            let off_idx = p.offset_from(p_mef) as usize;
            vim_snprintf!(
                name.add(off_idx),
                mef_len + 30 - off_idx,
                c"%d%d".as_ptr(),
                mef_start,
                mef_off,
            );
            c_strcat(name, p.add(2));
            let mut file_info = FileInfo::default();
            let file_or_link_found = os_fileinfo_link(name, &raw mut file_info);
            if !file_or_link_found {
                return name;
            }
            xfree(name.cast());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_get_size(eap: *mut exarg_T) -> usize {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 0;
        }
        (*qf_get_curlist(qi)).qf_count as usize
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_get_valid_size(eap: *mut exarg_T) -> usize {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 0;
        }

        let mut prev_fnum: c_int = 0;
        let mut sz: usize = 0;
        let qfl = qf_get_curlist(qi);
        assert!((*qfl).qf_count >= 0);
        let mut i: c_int = 1;
        let mut qfp = (*qfl).qf_start;
        while !got_int && i <= (*qfl).qf_count && !qfp.is_null() {
            if (*qfp).qf_valid != 0 {
                if (*eap).cmdidx == CMD_cdo || (*eap).cmdidx == CMD_ldo {
                    sz += 1;
                } else if (*qfp).qf_fnum > 0 && (*qfp).qf_fnum != prev_fnum {
                    sz += 1;
                    prev_fnum = (*qfp).qf_fnum;
                }
            }
            i += 1;
            qfp = (*qfp).qf_next;
        }

        sz
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_get_cur_idx(eap: *mut exarg_T) -> usize {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 0;
        }

        assert!((*qf_get_curlist(qi)).qf_index >= 0);
        (*qf_get_curlist(qi)).qf_index as usize
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn qf_get_cur_valid_idx(eap: *mut exarg_T) -> c_int {
    unsafe {
        let qi = qf_cmd_get_stack(eap, false);
        if qi.is_null() {
            return 1;
        }

        let qfl = qf_get_curlist(qi);
        if !qf_list_has_valid_entries(qfl) {
            return 1;
        }

        let mut prev_fnum: c_int = 0;
        let mut eidx: c_int = 0;
        assert!((*qfl).qf_index >= 0);
        let mut i: usize = 1;
        let mut qfp = (*qfl).qf_start;
        while i <= (*qfl).qf_index as usize && !qfp.is_null() {
            if (*qfp).qf_valid != 0 {
                if (*eap).cmdidx == CMD_cfdo || (*eap).cmdidx == CMD_lfdo {
                    if (*qfp).qf_fnum > 0 && (*qfp).qf_fnum != prev_fnum {
                        eidx += 1;
                        prev_fnum = (*qfp).qf_fnum;
                    }
                } else {
                    eidx += 1;
                }
            }
            i += 1;
            qfp = (*qfp).qf_next;
        }

        if eidx != 0 { eidx } else { 1 }
    }
}

unsafe fn qf_get_nth_valid_entry(qfl: *mut qf_list_T, n: usize, fdo: bool) -> usize {
    unsafe {
        if !qf_list_has_valid_entries(qfl) {
            return 1;
        }

        let mut prev_fnum: c_int = 0;
        let mut eidx: usize = 0;
        assert!((*qfl).qf_count >= 0);
        let mut i: c_int = 1;
        let mut qfp = (*qfl).qf_start;
        while !got_int && i <= (*qfl).qf_count && !qfp.is_null() {
            if (*qfp).qf_valid != 0 {
                if fdo {
                    if (*qfp).qf_fnum > 0 && (*qfp).qf_fnum != prev_fnum {
                        eidx += 1;
                        prev_fnum = (*qfp).qf_fnum;
                    }
                } else {
                    eidx += 1;
                }
            }

            if eidx == n {
                break;
            }
            i += 1;
            qfp = (*qfp).qf_next;
        }

        if i <= (*qfl).qf_count { i as usize } else { 1 }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cc(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let mut errornr: c_int = if (*eap).addr_count > 0 {
            (*eap).line2 as c_int
        } else {
            match (*eap).cmdidx {
                CMD_cc | CMD_ll => 0,
                CMD_crewind | CMD_lrewind | CMD_cfirst | CMD_lfirst => 1,
                _ => 32767,
            }
        };

        if (*eap).cmdidx == CMD_cdo
            || (*eap).cmdidx == CMD_ldo
            || (*eap).cmdidx == CMD_cfdo
            || (*eap).cmdidx == CMD_lfdo
        {
            let n = if (*eap).addr_count > 0 {
                assert!((*eap).line1 >= 0);
                (*eap).line1 as usize
            } else {
                1
            };
            let valid_entry = qf_get_nth_valid_entry(
                qf_get_curlist(qi),
                n,
                (*eap).cmdidx == CMD_cfdo || (*eap).cmdidx == CMD_lfdo,
            );
            assert!(valid_entry <= c_int::MAX as usize);
            errornr = valid_entry as c_int;
        }

        qf_jump(qi, 0, errornr, (*eap).forceit);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cnext(eap: *mut exarg_T) {
    unsafe {
        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let errornr = if (*eap).addr_count > 0
            && (*eap).cmdidx != CMD_cdo
            && (*eap).cmdidx != CMD_ldo
            && (*eap).cmdidx != CMD_cfdo
            && (*eap).cmdidx != CMD_lfdo
        {
            (*eap).line2 as c_int
        } else {
            1
        };

        let dir = match (*eap).cmdidx {
            CMD_cprevious | CMD_lprevious | CMD_cNext | CMD_lNext => BACKWARD,
            CMD_cnfile | CMD_lnfile | CMD_cfdo | CMD_lfdo => FORWARD_FILE,
            CMD_cpfile | CMD_lpfile | CMD_cNfile | CMD_lNfile => BACKWARD_FILE,
            _ => FORWARD,
        };

        qf_jump(qi, dir, errornr, (*eap).forceit);
    }
}

unsafe fn qf_find_first_entry_in_buf(
    qfl: *mut qf_list_T,
    bnr: c_int,
    errornr: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        let mut idx: c_int = 1;
        let mut qfp = (*qfl).qf_start;
        while !got_int && idx <= (*qfl).qf_count && !qfp.is_null() {
            if (*qfp).qf_fnum == bnr {
                break;
            }
            idx += 1;
            qfp = (*qfp).qf_next;
        }

        *errornr = idx;
        qfp
    }
}

unsafe fn qf_find_first_entry_on_line(
    mut entry: *mut qfline_T,
    errornr: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        while !got_int
            && !(*entry).qf_prev.is_null()
            && (*entry).qf_fnum == (*(*entry).qf_prev).qf_fnum
            && (*entry).qf_lnum == (*(*entry).qf_prev).qf_lnum
        {
            entry = (*entry).qf_prev;
            *errornr -= 1;
        }
        entry
    }
}

unsafe fn qf_find_last_entry_on_line(
    mut entry: *mut qfline_T,
    errornr: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        while !got_int
            && !(*entry).qf_next.is_null()
            && (*entry).qf_fnum == (*(*entry).qf_next).qf_fnum
            && (*entry).qf_lnum == (*(*entry).qf_next).qf_lnum
        {
            entry = (*entry).qf_next;
            *errornr += 1;
        }
        entry
    }
}

#[inline]
unsafe fn qf_entry_after_pos(qfp: *const qfline_T, pos: *const pos_T, linewise: bool) -> bool {
    unsafe {
        if linewise {
            (*qfp).qf_lnum > (*pos).lnum
        } else {
            (*qfp).qf_lnum > (*pos).lnum
                || ((*qfp).qf_lnum == (*pos).lnum && (*qfp).qf_col > (*pos).col)
        }
    }
}

#[inline]
unsafe fn qf_entry_before_pos(qfp: *const qfline_T, pos: *const pos_T, linewise: bool) -> bool {
    unsafe {
        if linewise {
            (*qfp).qf_lnum < (*pos).lnum
        } else {
            (*qfp).qf_lnum < (*pos).lnum
                || ((*qfp).qf_lnum == (*pos).lnum && (*qfp).qf_col < (*pos).col)
        }
    }
}

#[inline]
unsafe fn qf_entry_on_or_after_pos(
    qfp: *const qfline_T,
    pos: *const pos_T,
    linewise: bool,
) -> bool {
    unsafe {
        if linewise {
            (*qfp).qf_lnum >= (*pos).lnum
        } else {
            (*qfp).qf_lnum > (*pos).lnum
                || ((*qfp).qf_lnum == (*pos).lnum && (*qfp).qf_col >= (*pos).col)
        }
    }
}

#[inline]
unsafe fn qf_entry_on_or_before_pos(
    qfp: *const qfline_T,
    pos: *const pos_T,
    linewise: bool,
) -> bool {
    unsafe {
        if linewise {
            (*qfp).qf_lnum <= (*pos).lnum
        } else {
            (*qfp).qf_lnum < (*pos).lnum
                || ((*qfp).qf_lnum == (*pos).lnum && (*qfp).qf_col <= (*pos).col)
        }
    }
}

unsafe fn qf_find_entry_after_pos(
    bnr: c_int,
    pos: *const pos_T,
    linewise: bool,
    mut qfp: *mut qfline_T,
    errornr: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        if qf_entry_after_pos(qfp, pos, linewise) {
            return qfp;
        }

        while !(*qfp).qf_next.is_null()
            && (*(*qfp).qf_next).qf_fnum == bnr
            && qf_entry_on_or_before_pos((*qfp).qf_next, pos, linewise)
        {
            qfp = (*qfp).qf_next;
            *errornr += 1;
        }

        if (*qfp).qf_next.is_null() || (*(*qfp).qf_next).qf_fnum != bnr {
            return ptr::null_mut();
        }

        qfp = (*qfp).qf_next;
        *errornr += 1;

        qfp
    }
}

unsafe fn qf_find_entry_before_pos(
    bnr: c_int,
    pos: *const pos_T,
    linewise: bool,
    mut qfp: *mut qfline_T,
    errornr: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        while !(*qfp).qf_next.is_null()
            && (*(*qfp).qf_next).qf_fnum == bnr
            && qf_entry_before_pos((*qfp).qf_next, pos, linewise)
        {
            qfp = (*qfp).qf_next;
            *errornr += 1;
        }

        if qf_entry_on_or_after_pos(qfp, pos, linewise) {
            return ptr::null_mut();
        }

        if linewise {
            qfp = qf_find_first_entry_on_line(qfp, errornr);
        }

        qfp
    }
}

unsafe fn qf_find_closest_entry(
    qfl: *mut qf_list_T,
    bnr: c_int,
    pos: *const pos_T,
    dir: Direction,
    linewise: bool,
    errornr: *mut c_int,
) -> *mut qfline_T {
    unsafe {
        *errornr = 0;

        let mut qfp = qf_find_first_entry_in_buf(qfl, bnr, errornr);
        if qfp.is_null() {
            return ptr::null_mut();
        }

        if dir == FORWARD {
            qfp = qf_find_entry_after_pos(bnr, pos, linewise, qfp, errornr);
        } else {
            qfp = qf_find_entry_before_pos(bnr, pos, linewise, qfp, errornr);
        }

        qfp
    }
}

unsafe fn qf_get_nth_below_entry(
    entry_arg: *mut qfline_T,
    mut n: linenr_T,
    linewise: bool,
    errornr: *mut c_int,
) {
    unsafe {
        let mut entry = entry_arg;

        while n > 0 && !got_int {
            n -= 1;
            let first_errornr = *errornr;

            if linewise {
                entry = qf_find_last_entry_on_line(entry, errornr);
            }

            if (*entry).qf_next.is_null() || (*(*entry).qf_next).qf_fnum != (*entry).qf_fnum {
                if linewise {
                    *errornr = first_errornr;
                }
                break;
            }

            entry = (*entry).qf_next;
            *errornr += 1;
        }
    }
}

unsafe fn qf_get_nth_above_entry(
    mut entry: *mut qfline_T,
    mut n: linenr_T,
    linewise: bool,
    errornr: *mut c_int,
) {
    unsafe {
        while n > 0 && !got_int {
            n -= 1;
            if (*entry).qf_prev.is_null() || (*(*entry).qf_prev).qf_fnum != (*entry).qf_fnum {
                break;
            }

            entry = (*entry).qf_prev;
            *errornr -= 1;

            if linewise {
                entry = qf_find_first_entry_on_line(entry, errornr);
            }
        }
    }
}

unsafe fn qf_find_nth_adj_entry(
    qfl: *mut qf_list_T,
    bnr: c_int,
    pos: *mut pos_T,
    mut n: linenr_T,
    dir: Direction,
    linewise: bool,
) -> c_int {
    unsafe {
        let mut errornr: c_int = 0;

        let adj_entry = qf_find_closest_entry(qfl, bnr, pos, dir, linewise, &raw mut errornr);
        if adj_entry.is_null() {
            return 0;
        }

        n -= 1;
        if n > 0 {
            if dir == FORWARD {
                qf_get_nth_below_entry(adj_entry, n, linewise, &raw mut errornr);
            } else {
                qf_get_nth_above_entry(adj_entry, n, linewise, &raw mut errornr);
            }
        }

        errornr
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cbelow(eap: *mut exarg_T) {
    unsafe {
        if (*eap).addr_count > 0 && (*eap).line2 <= 0 {
            emsg(gettext((&raw const e_invrange).cast()));
            return;
        }

        let buf_has_flag = if (*eap).cmdidx == CMD_cabove
            || (*eap).cmdidx == CMD_cbelow
            || (*eap).cmdidx == CMD_cbefore
            || (*eap).cmdidx == CMD_cafter
        {
            BUF_HAS_QF_ENTRY
        } else {
            BUF_HAS_LL_ENTRY
        };

        if ((*curbuf).b_has_qf_entry & buf_has_flag) == 0 {
            emsg(gettext((&raw const e_no_errors).cast()));
            return;
        }

        let qi = qf_cmd_get_stack(eap, true);
        if qi.is_null() {
            return;
        }

        let qfl = qf_get_curlist(qi);
        if !qf_list_has_valid_entries(qfl) {
            emsg(gettext((&raw const e_no_errors).cast()));
            return;
        }

        let dir = if (*eap).cmdidx == CMD_cbelow
            || (*eap).cmdidx == CMD_lbelow
            || (*eap).cmdidx == CMD_cafter
            || (*eap).cmdidx == CMD_lafter
        {
            FORWARD
        } else {
            BACKWARD
        };

        let mut pos = (*curwin).w_cursor;
        pos.col += 1;
        let errornr = qf_find_nth_adj_entry(
            qfl,
            (*curbuf).b_fnum,
            &raw mut pos,
            if (*eap).addr_count > 0 {
                (*eap).line2
            } else {
                0
            },
            dir,
            (*eap).cmdidx == CMD_cbelow
                || (*eap).cmdidx == CMD_lbelow
                || (*eap).cmdidx == CMD_cabove
                || (*eap).cmdidx == CMD_labove,
        );

        if errornr > 0 {
            qf_jump(qi, 0, errornr, 0);
        } else {
            emsg(gettext(E_NO_MORE_ITEMS.as_ptr()));
        }
    }
}

unsafe fn cfile_get_auname(cmdidx: cmdidx_T) -> *const c_char {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cfile(eap: *mut exarg_T) {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();
        let mut qi = ql_info;
        assert!(!qi.is_null());

        let au_name = cfile_get_auname((*eap).cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            )
            && aborting()
        {
            return;
        }
        if *(*eap).arg != NUL {
            set_option_direct(kOptErrorfile, cstr_as_obj((*eap).arg), 0, 0);
        }

        let enc = if *(*curbuf).b_p_menc != NUL {
            (*curbuf).b_p_menc
        } else {
            p_menc
        };

        if is_loclist_cmd((*eap).cmdidx) {
            wp = curwin;
        }

        incr_quickfix_busy();

        let res = qf_init(
            wp,
            p_ef,
            p_efm,
            c_int::from((*eap).cmdidx != CMD_caddfile && (*eap).cmdidx != CMD_laddfile),
            qf_cmdtitle(*(*eap).cmdlinep),
            enc,
        );
        if !wp.is_null() {
            qi = get_loc_list(wp);
            if qi.is_null() {
                decr_quickfix_busy();
                return;
            }
        }
        if res >= 0 {
            qf_list_changed(qf_get_curlist(qi));
        }
        let save_qfid = (*qf_get_curlist(qi)).qf_id;
        if !au_name.is_null() {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.cast_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }
        if res > 0
            && ((*eap).cmdidx == CMD_cfile || (*eap).cmdidx == CMD_lfile)
            && qflist_valid(wp, save_qfid)
        {
            qf_jump_first(qi, save_qfid, (*eap).forceit);
        }

        decr_quickfix_busy();
    }
}

unsafe fn vgr_get_auname(cmdidx: cmdidx_T) -> *const c_char {
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

unsafe fn vgr_init_regmatch(regmatch: *mut regmmatch_T, s: *mut c_char) {
    unsafe {
        (*regmatch).regprog = ptr::null_mut();

        if s.is_null() || *s == NUL {
            if last_search_pat().is_null() {
                emsg(gettext((&raw const e_noprevre).cast()));
                return;
            }
            (*regmatch).regprog = vim_regcomp(last_search_pat(), RE_MAGIC);
        } else {
            (*regmatch).regprog = vim_regcomp(s, RE_MAGIC);
        }

        (*regmatch).rmm_ic = p_ic;
        (*regmatch).rmm_maxcol = 0;
    }
}

unsafe fn vgr_display_fname(fname: *mut c_char) {
    unsafe {
        msg_start();
        let p = msg_strtrunc(fname, 1);
        if p.is_null() {
            msg_outtrans(fname, 0, false);
        } else {
            msg_outtrans(p, 0, false);
            xfree(p.cast());
        }
        msg_clr_eos();
        msg_didout = false;
        msg_nowait = true;
        msg_col = 0;
        ui_flush();
    }
}

unsafe fn vgr_load_dummy_buf(
    fname: *mut c_char,
    dirname_start: *mut c_char,
    dirname_now: *mut c_char,
) -> *mut buf_T {
    unsafe {
        let save_ei = au_event_disable(c",Filetype".as_ptr().cast_mut());

        let save_mls = p_mls;
        p_mls = 0;

        let buf = load_dummy_buffer(fname, dirname_start, dirname_now);

        p_mls = save_mls;
        au_event_restore(save_ei);

        buf
    }
}

unsafe fn vgr_qflist_valid(
    wp: *mut win_T,
    qi: *mut qf_info_T,
    qfid: c_uint,
    title: *mut c_char,
) -> bool {
    unsafe {
        if !qflist_valid(wp, qfid) {
            if !wp.is_null() {
                emsg(gettext(E_CURRENT_LOCATION_LIST_WAS_CHANGED.as_ptr()));
                return false;
            }
            qf_new_list(qi, title);
            return true;
        }
        if qf_restore_list(qi, qfid) == FAIL {
            return false;
        }

        true
    }
}

unsafe fn vgr_match_buflines(
    qfl: *mut qf_list_T,
    fname: *mut c_char,
    buf: *mut buf_T,
    spat: *mut c_char,
    regmatch: *mut regmmatch_T,
    tomatch: *mut c_int,
    duplicate_name: c_int,
    flags: c_int,
) -> bool {
    unsafe {
        let mut found_match = false;
        let pat_len = c_strlen(spat).min(FUZZY_MATCH_MAX_LEN);

        let mut lnum: linenr_T = 1;
        while lnum <= (*buf).b_ml.ml_line_count && *tomatch > 0 {
            let mut col: colnr_T = 0;
            if (flags & VGR_FUZZY) == 0 {
                while vim_regexec_multi(
                    regmatch,
                    curwin,
                    buf,
                    lnum,
                    col,
                    ptr::null_mut(),
                    ptr::null_mut(),
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
                            (*buf).b_fnum
                        },
                        ml_get_buf(buf, (*regmatch).startpos[0].lnum + lnum),
                        (*regmatch).startpos[0].lnum + lnum,
                        (*regmatch).endpos[0].lnum + lnum,
                        (*regmatch).startpos[0].col + 1,
                        (*regmatch).endpos[0].col + 1,
                        0,
                        ptr::null_mut(),
                        0,
                        0,
                        ptr::null_mut(),
                        1,
                    ) == QF_FAIL
                    {
                        got_int = true;
                        break;
                    }
                    found_match = true;
                    *tomatch -= 1;
                    if *tomatch == 0 {
                        break;
                    }
                    if (flags & VGR_GLOBAL) == 0 || (*regmatch).endpos[0].lnum > 0 {
                        break;
                    }
                    col =
                        (*regmatch).endpos[0].col + colnr_T::from(col == (*regmatch).endpos[0].col);
                    if col > ml_get_buf_len(buf, lnum) {
                        break;
                    }
                }
            } else {
                let str_ = ml_get_buf(buf, lnum);
                let linelen = ml_get_buf_len(buf, lnum);
                let mut score: c_int = 0;
                let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];

                while fuzzy_match(
                    str_.add(col as usize),
                    spat,
                    false,
                    &raw mut score,
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
                            (*buf).b_fnum
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
                        ptr::null_mut(),
                        1,
                    ) == QF_FAIL
                    {
                        got_int = true;
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
            if got_int {
                break;
            }
            lnum += 1;
        }

        found_match
    }
}

unsafe fn vgr_jump_to_match(
    qi: *mut qf_info_T,
    forceit: c_int,
    redraw_for_dummy: *mut bool,
    first_match_buf: *mut buf_T,
    target_dir: *mut c_char,
) {
    unsafe {
        let buf = curbuf;
        qf_jump(qi, 0, 0, forceit);
        if buf != curbuf {
            *redraw_for_dummy = false;
        }

        if curbuf == first_match_buf && !target_dir.is_null() {
            let mut ea: exarg_T = std::mem::zeroed();
            ea.arg = target_dir;
            ea.cmdidx = CMD_lcd;
            ex_cd(&raw mut ea);
        }
    }
}

unsafe fn existing_swapfile(buf: *const buf_T) -> bool {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if !mfp.is_null() && !(*mfp).mf_fname.is_null() {
            let fname = (*mfp).mf_fname;
            let len = c_strlen(fname);
            if len >= 2 {
                return *fname.add(len - 1) != b'p' as c_char
                    || *fname.add(len - 2) != b'w' as c_char;
            }
            return true;
        }
        false
    }
}

unsafe fn vgr_process_args(eap: *mut exarg_T, args: *mut vgr_args_T) -> c_int {
    unsafe {
        ptr::write_bytes(args, 0, 1);

        (*args).regmatch.regprog = ptr::null_mut();
        (*args).qf_title = xstrdup(qf_cmdtitle(*(*eap).cmdlinep));
        (*args).tomatch = if (*eap).addr_count > 0 {
            (*eap).line2
        } else {
            MAXLNUM
        };

        let mut p = skip_vimgrep_pat((*eap).arg, &raw mut (*args).spat, &raw mut (*args).flags);
        if p.is_null() {
            emsg(gettext((&raw const e_invalpat).cast()));
            return FAIL;
        }

        vgr_init_regmatch(&raw mut (*args).regmatch, (*args).spat);
        if (*args).regmatch.regprog.is_null() {
            return FAIL;
        }

        p = skipwhite(p);
        if *p == NUL {
            emsg(gettext(
                c"E683: File name missing or invalid pattern".as_ptr(),
            ));
            return FAIL;
        }

        if get_arglist_exp(p, &raw mut (*args).fcount, &raw mut (*args).fnames, true) == FAIL
            || (*args).fcount == 0
        {
            emsg(gettext((&raw const e_nomatch).cast()));
            return FAIL;
        }

        OK
    }
}

unsafe fn vgr_process_files(
    wp: *mut win_T,
    qi: *mut qf_info_T,
    cmd_args: *mut vgr_args_T,
    redraw_for_dummy: *mut bool,
    first_match_buf: *mut *mut buf_T,
    target_dir: *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut status = FAIL;
        let mut save_qfid = (*qf_get_curlist(qi)).qf_id;
        let mut duplicate_name = false;

        let dirname_start = xmalloc(MAXPATHL).cast::<c_char>();
        let dirname_now = xmalloc(MAXPATHL).cast::<c_char>();

        os_dirname(dirname_start, MAXPATHL);

        let mut seconds: time_t = 0;
        let mut fi: c_int = 0;
        let mut aborted = false;
        while fi < (*cmd_args).fcount && !got_int && (*cmd_args).tomatch > 0 {
            let fname_exp = *(*cmd_args).fnames.add(fi as usize);
            let fname = path_try_shorten_fname(fname_exp);
            let now = time(ptr::null_mut());
            if now > seconds {
                seconds = now;
                vgr_display_fname(fname);
            }

            let mut buf = buflist_findname_exp(fname_exp);
            let using_dummy: bool;
            if buf.is_null() || (*buf).b_ml.ml_mfp.is_null() {
                duplicate_name = !buf.is_null();
                using_dummy = true;
                *redraw_for_dummy = true;
                buf = vgr_load_dummy_buf(fname, dirname_start, dirname_now);
            } else {
                using_dummy = false;
            }

            if !vgr_qflist_valid(wp, qi, save_qfid, (*cmd_args).qf_title) {
                aborted = true;
                break;
            }

            save_qfid = (*qf_get_curlist(qi)).qf_id;

            if buf.is_null() {
                if !got_int {
                    smsg!(0, gettext(c"Cannot open file \"%s\"".as_ptr()), fname);
                }
            } else {
                let found_match = vgr_match_buflines(
                    qf_get_curlist(qi),
                    fname,
                    buf,
                    (*cmd_args).spat,
                    &raw mut (*cmd_args).regmatch,
                    &raw mut (*cmd_args).tomatch,
                    c_int::from(duplicate_name),
                    (*cmd_args).flags,
                );

                if using_dummy {
                    if found_match && (*first_match_buf).is_null() {
                        *first_match_buf = buf;
                    }
                    if duplicate_name {
                        wipe_dummy_buffer(buf, dirname_start);
                        buf = ptr::null_mut();
                    } else if (cmdmod.cmod_flags & CMOD_HIDE) == 0
                        || *(*buf).b_p_bh == b'u' as c_char
                        || *(*buf).b_p_bh == b'w' as c_char
                        || *(*buf).b_p_bh == b'd' as c_char
                    {
                        if !found_match {
                            wipe_dummy_buffer(buf, dirname_start);
                            buf = ptr::null_mut();
                        } else if buf != *first_match_buf
                            || ((*cmd_args).flags & VGR_NOJUMP) != 0
                            || existing_swapfile(buf)
                        {
                            unload_dummy_buffer(buf, dirname_start);
                            (*buf).b_flags &= !BF_DUMMY;
                            buf = ptr::null_mut();
                        }
                    }

                    if !buf.is_null() {
                        (*buf).b_flags &= !BF_DUMMY;

                        if buf == *first_match_buf
                            && (*target_dir).is_null()
                            && c_strcmp(dirname_start, dirname_now) != 0
                        {
                            *target_dir = xstrdup(dirname_now);
                        }

                        let mut aco: CtxSwitch = std::mem::zeroed();
                        ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
                        apply_autocmds(EVENT_FILETYPE, (*buf).b_p_ft, (*buf).b_fname, true, buf);
                        do_modelines(OPT_NOWIN);
                        ctx_restore(&raw mut aco);
                    }
                }
            }
            fi += 1;
        }

        if !aborted {
            status = OK;
        }

        xfree(dirname_now.cast());
        xfree(dirname_start.cast());
        status
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_vimgrep(eap: *mut exarg_T) {
    unsafe {
        if !check_can_set_curbuf_forceit((*eap).forceit) {
            return;
        }

        let au_name = vgr_get_auname((*eap).cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
            )
            && aborting()
        {
            return;
        }

        let mut wp: *mut win_T = ptr::null_mut();
        let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);
        let mut target_dir: *mut c_char = ptr::null_mut();
        let mut args: vgr_args_T = std::mem::zeroed();
        if vgr_process_args(eap, &raw mut args) != FAIL {
            if ((*eap).cmdidx != CMD_grepadd
                && (*eap).cmdidx != CMD_lgrepadd
                && (*eap).cmdidx != CMD_vimgrepadd
                && (*eap).cmdidx != CMD_lvimgrepadd)
                || qf_stack_empty(qi)
            {
                qf_new_list(qi, args.qf_title);
            }

            incr_quickfix_busy();

            let mut redraw_for_dummy = false;
            let mut first_match_buf: *mut buf_T = ptr::null_mut();
            let status = vgr_process_files(
                wp,
                qi,
                &raw mut args,
                &raw mut redraw_for_dummy,
                &raw mut first_match_buf,
                &raw mut target_dir,
            );

            FreeWild(args.fcount, args.fnames);

            if status == OK {
                let qfl = qf_get_curlist(qi);
                (*qfl).qf_nonevalid = false;
                (*qfl).qf_ptr = (*qfl).qf_start;
                (*qfl).qf_index = 1;
                qf_list_changed(qfl);

                qf_update_buffer(qi, ptr::null_mut());

                let save_qfid = (*qf_get_curlist(qi)).qf_id;

                if !au_name.is_null() {
                    apply_autocmds(
                        EVENT_QUICKFIXCMDPOST,
                        au_name.cast_mut(),
                        (*curbuf).b_fname,
                        true,
                        curbuf,
                    );
                }

                if qflist_valid(wp, save_qfid) && qf_restore_list(qi, save_qfid) != FAIL {
                    if !qf_list_empty(qf_get_curlist(qi)) {
                        if (args.flags & VGR_NOJUMP) == 0 {
                            vgr_jump_to_match(
                                qi,
                                (*eap).forceit,
                                &raw mut redraw_for_dummy,
                                first_match_buf,
                                target_dir,
                            );
                        }
                    } else {
                        semsg!(gettext((&raw const e_nomatch2).cast()), args.spat);
                    }

                    decr_quickfix_busy();

                    if redraw_for_dummy {
                        foldUpdateAll(curwin);
                    }
                } else {
                    decr_quickfix_busy();
                }
            } else {
                decr_quickfix_busy();
            }
        }

        xfree(args.qf_title.cast());
        xfree(target_dir.cast());
        vim_regfree(args.regmatch.regprog);
    }
}

unsafe fn restore_start_dir(dirname_start: *mut c_char) {
    unsafe {
        let dirname_now = xmalloc(MAXPATHL).cast::<c_char>();

        os_dirname(dirname_now, MAXPATHL);
        if c_strcmp(dirname_start, dirname_now) != 0 {
            let mut ea: exarg_T = std::mem::zeroed();
            ea.arg = dirname_start;
            ea.cmdidx = if (*curwin).w_localdir.is_null() {
                CMD_cd
            } else {
                CMD_lcd
            };
            ex_cd(&raw mut ea);
        }
        xfree(dirname_now.cast());
    }
}

unsafe fn load_dummy_buffer(
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
        let mut newbufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut newbufref, newbuf);

        buf_copy_options(newbuf, BCO_ENTER | BCO_NOHELP);

        if ml_open(newbuf) == OK {
            (*newbuf).b_locked += 1;
            let mut aco: CtxSwitch = std::mem::zeroed();
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), newbuf, 0);

            setfname(curbuf, fname, ptr::null_mut(), false);
            check_need_swap(true);
            (*curbuf).b_flags &= !BF_DUMMY;

            let mut newbuf_to_wipe: bufref_T = std::mem::zeroed();
            let readfile_result = readfile(
                fname,
                ptr::null_mut(),
                0,
                0,
                MAXLNUM,
                ptr::null_mut(),
                READ_NEW | READ_DUMMY,
                false,
            );
            (*newbuf).b_locked -= 1;
            if readfile_result == OK && !got_int && ((*curbuf).b_flags & BF_NEW) == 0 {
                failed = false;
                if curbuf != newbuf {
                    set_bufref(&raw mut newbuf_to_wipe, newbuf);
                    newbuf = curbuf;
                }
            }

            ctx_restore(&raw mut aco);

            if !newbuf_to_wipe.br_buf.is_null() && bufref_valid(&raw mut newbuf_to_wipe) {
                block_autocmds();
                wipe_dummy_buffer(newbuf_to_wipe.br_buf, ptr::null_mut());
                unblock_autocmds();
            }

            (*newbuf).b_flags |= BF_DUMMY;
        }

        os_dirname(resulting_dir, MAXPATHL);
        restore_start_dir(dirname_start);

        if !bufref_valid(&raw mut newbufref) {
            return ptr::null_mut();
        }
        if failed {
            wipe_dummy_buffer(newbuf, dirname_start);
            return ptr::null_mut();
        }
        newbuf
    }
}

unsafe fn wipe_dummy_buffer(buf: *mut buf_T, dirname_start: *mut c_char) {
    unsafe {
        let mut give_up = false;
        loop {
            if (*buf).b_nwindows <= 0 {
                break;
            }
            let mut did_one = false;

            if !(*firstwin).w_next.is_null() {
                let mut wp = firstwin;
                while !wp.is_null() {
                    if (*wp).w_buffer == buf {
                        if win_close(wp, false, false) == OK {
                            did_one = true;
                        }
                        break;
                    }
                    wp = (*wp).w_next;
                }
            }
            if !did_one {
                give_up = true;
                break;
            }
        }

        if !give_up && curbuf != buf && (*buf).b_nwindows == 0 {
            let mut cs: cleanup_T = std::mem::zeroed();
            enter_cleanup(&raw mut cs);
            wipe_buffer(buf, true);
            leave_cleanup(&raw mut cs);

            if !dirname_start.is_null() {
                restore_start_dir(dirname_start);
            }
            return;
        }

        (*buf).b_flags &= !BF_DUMMY;
    }
}

unsafe fn unload_dummy_buffer(buf: *mut buf_T, dirname_start: *mut c_char) {
    unsafe {
        if curbuf == buf {
            return;
        }

        close_buffer(ptr::null_mut(), buf, DOBUF_UNLOAD, false, true, false);
        restore_start_dir(dirname_start);
    }
}

unsafe fn get_qfline_items(qfp: *mut qfline_T, list: *mut list_T) -> c_int {
    unsafe {
        let mut bufnum = (*qfp).qf_fnum;
        if bufnum != 0 && buflist_findnr(bufnum).is_null() {
            bufnum = 0;
        }

        let dict = tv_dict_alloc();
        tv_list_append_dict(list, dict);

        let buf = [(*qfp).qf_type, NUL];
        let buflen: usize = if buf[0] == NUL { 0 } else { 1 };
        let empty = c"".as_ptr();
        if tv_dict_add_nr(dict, c"bufnr".as_ptr(), 5, varnumber_T::from(bufnum)) == FAIL
            || tv_dict_add_nr(dict, c"lnum".as_ptr(), 4, varnumber_T::from((*qfp).qf_lnum)) == FAIL
            || tv_dict_add_nr(
                dict,
                c"end_lnum".as_ptr(),
                8,
                varnumber_T::from((*qfp).qf_end_lnum),
            ) == FAIL
            || tv_dict_add_nr(dict, c"col".as_ptr(), 3, varnumber_T::from((*qfp).qf_col)) == FAIL
            || tv_dict_add_nr(
                dict,
                c"end_col".as_ptr(),
                7,
                varnumber_T::from((*qfp).qf_end_col),
            ) == FAIL
            || tv_dict_add_nr(
                dict,
                c"vcol".as_ptr(),
                4,
                varnumber_T::from((*qfp).qf_viscol),
            ) == FAIL
            || tv_dict_add_nr(dict, c"nr".as_ptr(), 2, varnumber_T::from((*qfp).qf_nr)) == FAIL
            || tv_dict_add_str(
                dict,
                c"module".as_ptr(),
                6,
                if (*qfp).qf_module.is_null() {
                    empty
                } else {
                    (*qfp).qf_module
                },
            ) == FAIL
            || tv_dict_add_str(
                dict,
                c"pattern".as_ptr(),
                7,
                if (*qfp).qf_pattern.is_null() {
                    empty
                } else {
                    (*qfp).qf_pattern
                },
            ) == FAIL
            || tv_dict_add_str(
                dict,
                c"text".as_ptr(),
                4,
                if (*qfp).qf_text.is_null() {
                    empty
                } else {
                    (*qfp).qf_text
                },
            ) == FAIL
            || tv_dict_add_str_len(dict, c"type".as_ptr(), 4, buf.as_ptr(), buflen as c_int) == FAIL
            || ((*qfp).qf_user_data.v_type != VarType::VAR_UNKNOWN
                && tv_dict_add_tv(dict, c"user_data".as_ptr(), 9, &raw mut (*qfp).qf_user_data)
                    == FAIL)
            || tv_dict_add_nr(
                dict,
                c"valid".as_ptr(),
                5,
                varnumber_T::from((*qfp).qf_valid),
            ) == FAIL
        {
            std::process::abort();
        }

        OK
    }
}

unsafe fn get_errorlist(
    qi_arg: *mut qf_info_T,
    wp: *mut win_T,
    mut qf_idx: c_int,
    eidx: c_int,
    list: *mut list_T,
) -> c_int {
    unsafe {
        let mut qi = qi_arg;
        if qi.is_null() {
            qi = ql_info;
            if !wp.is_null() {
                qi = get_loc_list(wp);
            }
            if qi.is_null() {
                return FAIL;
            }
        }

        if eidx < 0 {
            return OK;
        }

        if qf_idx == INVALID_QFIDX {
            qf_idx = (*qi).qf_curlist;
        }

        if qf_idx >= (*qi).qf_listcount {
            return FAIL;
        }

        let qfl = qf_get_list(qi, qf_idx);
        if qf_list_empty(qfl) {
            return FAIL;
        }

        let mut i: c_int = 1;
        let mut qfp = (*qfl).qf_start;
        while !got_int && i <= (*qfl).qf_count && !qfp.is_null() {
            if eidx > 0 {
                if eidx == i {
                    return get_qfline_items(qfp, list);
                }
            } else if get_qfline_items(qfp, list) == FAIL {
                return FAIL;
            }
            i += 1;
            qfp = (*qfp).qf_next;
        }

        OK
    }
}

unsafe fn qf_get_list_from_lines(
    what: *mut dict_T,
    di: *mut dictitem_T,
    retdict: *mut dict_T,
) -> c_int {
    unsafe {
        if (*di).di_tv.v_type != VarType::VAR_LIST || (*di).di_tv.vval.v_list.is_null() {
            return FAIL;
        }

        let mut errorformat = p_efm;
        let efm_di = tv_dict_find(what, c"efm".as_ptr(), 3);
        if !efm_di.is_null() {
            if (*efm_di).di_tv.v_type != VarType::VAR_STRING
                || (*efm_di).di_tv.vval.v_string.is_null()
            {
                return FAIL;
            }
            errorformat = (*efm_di).di_tv.vval.v_string;
        }

        let l = tv_list_alloc(kListLenMayKnow);
        let qi = qf_alloc_stack(QFLT_INTERNAL, 1);

        if qf_init_ext(
            qi,
            0,
            ptr::null(),
            ptr::null_mut(),
            &raw mut (*di).di_tv,
            errorformat,
            true,
            0,
            0,
            ptr::null(),
            ptr::null_mut(),
        ) > 0
        {
            get_errorlist(qi, ptr::null_mut(), 0, 0, l);
            qf_free((*qi).qf_lists);
        }

        qf_free_lists(qi);

        tv_dict_add_list(retdict, c"items".as_ptr(), 5, l);
        OK
    }
}

unsafe fn qf_winid(qi: *mut qf_info_T) -> c_int {
    unsafe {
        if qi.is_null() {
            return 0;
        }
        let win = qf_find_win(qi);
        if !win.is_null() {
            return (*win).handle;
        }
        0
    }
}

unsafe fn qf_getprop_qfbufnr(qi: *const qf_info_T, retdict: *mut dict_T) -> c_int {
    unsafe {
        let mut bufnum: c_int = 0;
        if !qi.is_null() && !buflist_findnr((*qi).qf_bufnr).is_null() {
            bufnum = (*qi).qf_bufnr;
        }
        tv_dict_add_nr(retdict, c"qfbufnr".as_ptr(), 7, varnumber_T::from(bufnum))
    }
}

unsafe fn qf_getprop_keys2flags(what: *const dict_T, loclist: bool) -> c_int {
    unsafe {
        let mut flags = QF_GETLIST_NONE;

        if !tv_dict_find(what, c"all".as_ptr(), 3).is_null() {
            flags |= QF_GETLIST_ALL;
            if !loclist {
                flags &= !QF_GETLIST_FILEWINID;
            }
        }
        if !tv_dict_find(what, c"title".as_ptr(), 5).is_null() {
            flags |= QF_GETLIST_TITLE;
        }
        if !tv_dict_find(what, c"nr".as_ptr(), 2).is_null() {
            flags |= QF_GETLIST_NR;
        }
        if !tv_dict_find(what, c"winid".as_ptr(), 5).is_null() {
            flags |= QF_GETLIST_WINID;
        }
        if !tv_dict_find(what, c"context".as_ptr(), 7).is_null() {
            flags |= QF_GETLIST_CONTEXT;
        }
        if !tv_dict_find(what, c"id".as_ptr(), 2).is_null() {
            flags |= QF_GETLIST_ID;
        }
        if !tv_dict_find(what, c"items".as_ptr(), 5).is_null() {
            flags |= QF_GETLIST_ITEMS;
        }
        if !tv_dict_find(what, c"idx".as_ptr(), 3).is_null() {
            flags |= QF_GETLIST_IDX;
        }
        if !tv_dict_find(what, c"size".as_ptr(), 4).is_null() {
            flags |= QF_GETLIST_SIZE;
        }
        if !tv_dict_find(what, c"changedtick".as_ptr(), 11).is_null() {
            flags |= QF_GETLIST_TICK;
        }
        if loclist && !tv_dict_find(what, c"filewinid".as_ptr(), 9).is_null() {
            flags |= QF_GETLIST_FILEWINID;
        }
        if !tv_dict_find(what, c"qfbufnr".as_ptr(), 7).is_null() {
            flags |= QF_GETLIST_QFBUFNR;
        }
        if !tv_dict_find(what, c"quickfixtextfunc".as_ptr(), 16).is_null() {
            flags |= QF_GETLIST_QFTF;
        }

        flags
    }
}

unsafe fn qf_getprop_qfidx(qi: *mut qf_info_T, what: *mut dict_T) -> c_int {
    unsafe {
        let mut qf_idx = (*qi).qf_curlist;
        let mut di = tv_dict_find(what, c"nr".as_ptr(), 2);
        if !di.is_null() {
            if (*di).di_tv.v_type == VarType::VAR_NUMBER {
                if (*di).di_tv.vval.v_number != 0 {
                    qf_idx = (*di).di_tv.vval.v_number as c_int - 1;
                    if qf_idx < 0 || qf_idx >= (*qi).qf_listcount {
                        qf_idx = INVALID_QFIDX;
                    }
                }
            } else if (*di).di_tv.v_type == VarType::VAR_STRING
                && strequal((*di).di_tv.vval.v_string, c"$".as_ptr())
            {
                qf_idx = (*qi).qf_listcount - 1;
            } else {
                qf_idx = INVALID_QFIDX;
            }
        }

        di = tv_dict_find(what, c"id".as_ptr(), 2);
        if !di.is_null() {
            if (*di).di_tv.v_type == VarType::VAR_NUMBER {
                if (*di).di_tv.vval.v_number != 0 {
                    qf_idx = qf_id2nr(qi, (*di).di_tv.vval.v_number as c_uint);
                }
            } else {
                qf_idx = INVALID_QFIDX;
            }
        }

        qf_idx
    }
}

unsafe fn qf_getprop_defaults(
    qi: *mut qf_info_T,
    flags: c_int,
    locstack: bool,
    retdict: *mut dict_T,
) -> c_int {
    unsafe {
        let mut status = OK;
        let empty = c"".as_ptr();

        if (flags & QF_GETLIST_TITLE) != 0 {
            status = tv_dict_add_str_len(retdict, c"title".as_ptr(), 5, empty, 0);
        }
        if status == OK && (flags & QF_GETLIST_ITEMS) != 0 {
            let l = tv_list_alloc(kListLenMayKnow);
            status = tv_dict_add_list(retdict, c"items".as_ptr(), 5, l);
        }
        if status == OK && (flags & QF_GETLIST_NR) != 0 {
            status = tv_dict_add_nr(retdict, c"nr".as_ptr(), 2, 0);
        }
        if status == OK && (flags & QF_GETLIST_WINID) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"winid".as_ptr(),
                5,
                varnumber_T::from(qf_winid(qi)),
            );
        }
        if status == OK && (flags & QF_GETLIST_CONTEXT) != 0 {
            status = tv_dict_add_str_len(retdict, c"context".as_ptr(), 7, empty, 0);
        }
        if status == OK && (flags & QF_GETLIST_ID) != 0 {
            status = tv_dict_add_nr(retdict, c"id".as_ptr(), 2, 0);
        }
        if status == OK && (flags & QF_GETLIST_IDX) != 0 {
            status = tv_dict_add_nr(retdict, c"idx".as_ptr(), 3, 0);
        }
        if status == OK && (flags & QF_GETLIST_SIZE) != 0 {
            status = tv_dict_add_nr(retdict, c"size".as_ptr(), 4, 0);
        }
        if status == OK && (flags & QF_GETLIST_TICK) != 0 {
            status = tv_dict_add_nr(retdict, c"changedtick".as_ptr(), 11, 0);
        }
        if status == OK && locstack && (flags & QF_GETLIST_FILEWINID) != 0 {
            status = tv_dict_add_nr(retdict, c"filewinid".as_ptr(), 9, 0);
        }
        if status == OK && (flags & QF_GETLIST_QFBUFNR) != 0 {
            status = qf_getprop_qfbufnr(qi, retdict);
        }
        if status == OK && (flags & QF_GETLIST_QFTF) != 0 {
            status = tv_dict_add_str_len(retdict, c"quickfixtextfunc".as_ptr(), 16, empty, 0);
        }

        status
    }
}

unsafe fn qf_getprop_title(qfl: *mut qf_list_T, retdict: *mut dict_T) -> c_int {
    unsafe { tv_dict_add_str(retdict, c"title".as_ptr(), 5, (*qfl).qf_title) }
}

unsafe fn qf_getprop_filewinid(
    wp: *const win_T,
    qi: *const qf_info_T,
    retdict: *mut dict_T,
) -> c_int {
    unsafe {
        let mut winid: handle_T = 0;

        if !wp.is_null() && is_ll_window(wp) {
            let ll_wp = qf_find_win_with_loclist(qi);
            if !ll_wp.is_null() {
                winid = (*ll_wp).handle;
            }
        }

        tv_dict_add_nr(retdict, c"filewinid".as_ptr(), 9, varnumber_T::from(winid))
    }
}

unsafe fn qf_getprop_items(
    qi: *mut qf_info_T,
    qf_idx: c_int,
    eidx: c_int,
    retdict: *mut dict_T,
) -> c_int {
    unsafe {
        let l = tv_list_alloc(kListLenMayKnow);
        get_errorlist(qi, ptr::null_mut(), qf_idx, eidx, l);
        tv_dict_add_list(retdict, c"items".as_ptr(), 5, l);
        OK
    }
}

unsafe fn qf_getprop_ctx(qfl: *mut qf_list_T, retdict: *mut dict_T) -> c_int {
    unsafe {
        if !(*qfl).qf_ctx.is_null() {
            let di = tv_dict_item_alloc_len(c"context".as_ptr(), 7);
            tv_copy((*qfl).qf_ctx, &raw mut (*di).di_tv);
            let status = tv_dict_add(retdict, di);
            if status == FAIL {
                tv_dict_item_free(di);
            }
            status
        } else {
            tv_dict_add_str_len(retdict, c"context".as_ptr(), 7, c"".as_ptr(), 0)
        }
    }
}

unsafe fn qf_getprop_idx(qfl: *mut qf_list_T, mut eidx: c_int, retdict: *mut dict_T) -> c_int {
    unsafe {
        if eidx == 0 {
            eidx = (*qfl).qf_index;
            if qf_list_empty(qfl) {
                eidx = 0;
            }
        }
        tv_dict_add_nr(retdict, c"idx".as_ptr(), 3, varnumber_T::from(eidx))
    }
}

unsafe fn qf_getprop_qftf(qfl: *mut qf_list_T, retdict: *mut dict_T) -> c_int {
    unsafe {
        if (*qfl).qf_qftf_cb.cb_type != kCallbackNone {
            let mut tv: typval_T = std::mem::zeroed();
            callback_put(&raw mut (*qfl).qf_qftf_cb, &raw mut tv);
            let status = tv_dict_add_tv(retdict, c"quickfixtextfunc".as_ptr(), 16, &raw mut tv);
            tv_clear(&raw mut tv);
            status
        } else {
            tv_dict_add_str_len(retdict, c"quickfixtextfunc".as_ptr(), 16, c"".as_ptr(), 0)
        }
    }
}

unsafe fn qf_get_properties(wp: *mut win_T, what: *mut dict_T, retdict: *mut dict_T) -> c_int {
    unsafe {
        let mut qi = ql_info;
        assert!(!qi.is_null());
        let mut status = OK;
        let mut qf_idx = INVALID_QFIDX;

        let di = tv_dict_find(what, c"lines".as_ptr(), 5);
        if !di.is_null() {
            return qf_get_list_from_lines(what, di, retdict);
        }

        if !wp.is_null() {
            qi = get_loc_list(wp);
        }

        let flags = qf_getprop_keys2flags(what, !wp.is_null());

        if !qf_stack_empty(qi) {
            qf_idx = qf_getprop_qfidx(qi, what);
        }

        if qf_stack_empty(qi) || qf_idx == INVALID_QFIDX {
            return qf_getprop_defaults(qi, flags, !wp.is_null(), retdict);
        }

        let qfl = qf_get_list(qi, qf_idx);
        let mut eidx: c_int = 0;

        let di = tv_dict_find(what, c"idx".as_ptr(), 3);
        if !di.is_null() {
            if (*di).di_tv.v_type != VarType::VAR_NUMBER {
                return FAIL;
            }
            eidx = (*di).di_tv.vval.v_number as c_int;
        }

        if (flags & QF_GETLIST_TITLE) != 0 {
            status = qf_getprop_title(qfl, retdict);
        }
        if status == OK && (flags & QF_GETLIST_NR) != 0 {
            status = tv_dict_add_nr(retdict, c"nr".as_ptr(), 2, varnumber_T::from(qf_idx + 1));
        }
        if status == OK && (flags & QF_GETLIST_WINID) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"winid".as_ptr(),
                5,
                varnumber_T::from(qf_winid(qi)),
            );
        }
        if status == OK && (flags & QF_GETLIST_ITEMS) != 0 {
            status = qf_getprop_items(qi, qf_idx, eidx, retdict);
        }
        if status == OK && (flags & QF_GETLIST_CONTEXT) != 0 {
            status = qf_getprop_ctx(qfl, retdict);
        }
        if status == OK && (flags & QF_GETLIST_ID) != 0 {
            status = tv_dict_add_nr(retdict, c"id".as_ptr(), 2, varnumber_T::from((*qfl).qf_id));
        }
        if status == OK && (flags & QF_GETLIST_IDX) != 0 {
            status = qf_getprop_idx(qfl, eidx, retdict);
        }
        if status == OK && (flags & QF_GETLIST_SIZE) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"size".as_ptr(),
                4,
                varnumber_T::from((*qfl).qf_count),
            );
        }
        if status == OK && (flags & QF_GETLIST_TICK) != 0 {
            status = tv_dict_add_nr(
                retdict,
                c"changedtick".as_ptr(),
                11,
                varnumber_T::from((*qfl).qf_changedtick),
            );
        }
        if status == OK && !wp.is_null() && (flags & QF_GETLIST_FILEWINID) != 0 {
            status = qf_getprop_filewinid(wp, qi, retdict);
        }
        if status == OK && (flags & QF_GETLIST_QFBUFNR) != 0 {
            status = qf_getprop_qfbufnr(qi, retdict);
        }
        if status == OK && (flags & QF_GETLIST_QFTF) != 0 {
            status = qf_getprop_qftf(qfl, retdict);
        }

        status
    }
}

unsafe fn qf_setprop_qftf(qfl: *mut qf_list_T, di: *mut dictitem_T) -> c_int {
    unsafe {
        if check_secure() {
            return FAIL;
        }

        let mut cb: Callback = std::mem::zeroed();
        callback_free(&raw mut (*qfl).qf_qftf_cb);
        if callback_from_typval(&raw mut cb, &raw mut (*di).di_tv) {
            (*qfl).qf_qftf_cb = cb;
        }
        OK
    }
}

unsafe fn qf_add_entry_from_dict(
    qfl: *mut qf_list_T,
    d: *mut dict_T,
    first_entry: bool,
    valid_entry: *mut bool,
) -> c_int {
    unsafe {
        if first_entry {
            did_bufnr_emsg = false;
        }

        let filename = tv_dict_get_string(d, c"filename".as_ptr(), true);
        let module = tv_dict_get_string(d, c"module".as_ptr(), true);
        let mut bufnum = tv_dict_get_number(d, c"bufnr".as_ptr()) as c_int;
        let lnum = tv_dict_get_number(d, c"lnum".as_ptr()) as linenr_T;
        let end_lnum = tv_dict_get_number(d, c"end_lnum".as_ptr()) as linenr_T;
        let col = tv_dict_get_number(d, c"col".as_ptr()) as c_int;
        let end_col = tv_dict_get_number(d, c"end_col".as_ptr()) as c_int;
        let vcol = tv_dict_get_number(d, c"vcol".as_ptr()) as c_char;
        let nr = tv_dict_get_number(d, c"nr".as_ptr()) as c_int;
        let type_ = tv_dict_get_string(d, c"type".as_ptr(), false);
        let pattern = tv_dict_get_string(d, c"pattern".as_ptr(), true);
        let mut text = tv_dict_get_string(d, c"text".as_ptr(), true);
        if text.is_null() {
            text = xcalloc(1, 1).cast::<c_char>();
        }
        let mut user_data: typval_T = std::mem::zeroed();
        user_data.v_type = VarType::VAR_UNKNOWN;
        tv_dict_get_tv(d, c"user_data".as_ptr(), &raw mut user_data);

        let mut valid = true;
        if (filename.is_null() && bufnum == 0) || (lnum == 0 && pattern.is_null()) {
            valid = false;
        }

        if bufnum != 0 && buflist_findnr(bufnum).is_null() {
            if !did_bufnr_emsg {
                did_bufnr_emsg = true;
                semsg!(gettext(c"E92: Buffer %d not found".as_ptr()), bufnum);
            }
            valid = false;
            bufnum = 0;
        }

        if !tv_dict_find(d, c"valid".as_ptr(), -1).is_null() {
            valid = tv_dict_get_bool(d, c"valid".as_ptr(), 0) != 0;
        }

        let status = qf_add_entry(
            qfl,
            ptr::null_mut(),
            filename,
            module,
            bufnum,
            text,
            lnum,
            end_lnum,
            col,
            end_col,
            vcol,
            pattern,
            nr,
            if type_.is_null() { NUL } else { *type_ },
            &raw mut user_data,
            c_char::from(valid),
        );

        xfree(filename.cast());
        xfree(module.cast());
        xfree(pattern.cast());
        xfree(text.cast());
        tv_clear(&raw mut user_data);

        if valid {
            *valid_entry = true;
        }

        status
    }
}

unsafe fn entry_is_closer_to_target(
    entry: *mut qfline_T,
    other_entry: *mut qfline_T,
    target_fnum: c_int,
    target_lnum: c_int,
    target_col: c_int,
) -> bool {
    unsafe {
        if target_fnum == 0 {
            return false;
        }

        let is_target_file = (*entry).qf_fnum != 0 && (*entry).qf_fnum == target_fnum;
        let other_is_target_file =
            (*other_entry).qf_fnum != 0 && (*other_entry).qf_fnum == target_fnum;
        if !is_target_file && other_is_target_file {
            return false;
        } else if is_target_file && !other_is_target_file {
            return true;
        }

        if target_lnum == 0 {
            return false;
        }

        let line_distance = if (*entry).qf_lnum != 0 {
            ((*entry).qf_lnum - target_lnum).abs()
        } else {
            c_int::MAX
        };
        let other_line_distance = if (*other_entry).qf_lnum != 0 {
            ((*other_entry).qf_lnum - target_lnum).abs()
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

        let column_distance = if (*entry).qf_col != 0 {
            ((*entry).qf_col - target_col).abs()
        } else {
            c_int::MAX
        };
        let other_column_distance = if (*other_entry).qf_col != 0 {
            ((*other_entry).qf_col - target_col).abs()
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
}

unsafe fn qf_add_entries(
    qi: *mut qf_info_T,
    mut qf_idx: c_int,
    list: *mut list_T,
    title: *mut c_char,
    action: c_int,
) -> c_int {
    unsafe {
        let mut qfl = qf_get_list(qi, qf_idx);
        let mut old_last: *mut qfline_T = ptr::null_mut();
        let mut retval = OK;
        let mut valid_entry = false;

        let mut prev_fnum: c_int = 0;
        let mut prev_lnum: c_int = 0;
        let mut prev_col: c_int = 0;
        if !(*qfl).qf_ptr.is_null() {
            prev_fnum = (*(*qfl).qf_ptr).qf_fnum;
            prev_lnum = (*(*qfl).qf_ptr).qf_lnum;
            prev_col = (*(*qfl).qf_ptr).qf_col;
        }

        let mut select_first_entry = false;
        let mut select_nearest_entry = false;

        if action == b' ' as c_int || qf_idx == (*qi).qf_listcount {
            select_first_entry = true;
            qf_new_list(qi, title);
            qf_idx = (*qi).qf_curlist;
            qfl = qf_get_list(qi, qf_idx);
        } else if action == b'a' as c_int {
            if qf_list_empty(qfl) {
                select_first_entry = true;
            } else {
                old_last = (*qfl).qf_last;
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

        let mut entry_to_select: *mut qfline_T = ptr::null_mut();
        let mut entry_to_select_index: c_int = 0;

        let first_li = tv_list_first(list);
        let mut li = first_li;
        while !li.is_null() {
            if (*li).li_tv.v_type == VarType::VAR_DICT {
                let d = (*li).li_tv.vval.v_dict;
                if !d.is_null() {
                    retval = qf_add_entry_from_dict(qfl, d, li == first_li, &raw mut valid_entry);
                    if retval == QF_FAIL {
                        break;
                    }

                    let entry = (*qfl).qf_last;
                    if (select_first_entry && entry_to_select.is_null())
                        || (select_nearest_entry
                            && (entry_to_select.is_null()
                                || entry_is_closer_to_target(
                                    entry,
                                    entry_to_select,
                                    prev_fnum,
                                    prev_lnum,
                                    prev_col,
                                )))
                    {
                        entry_to_select = entry;
                        entry_to_select_index = (*qfl).qf_count;
                    }
                }
            }
            li = (*li).li_next;
        }

        if valid_entry {
            (*qfl).qf_nonevalid = false;
        } else if (*qfl).qf_index == 0 {
            (*qfl).qf_nonevalid = true;
        }

        if !entry_to_select.is_null() {
            (*qfl).qf_ptr = entry_to_select;
            (*qfl).qf_index = entry_to_select_index;
        }

        qf_update_buffer(qi, old_last);

        retval
    }
}

unsafe fn qf_setprop_get_qfidx(
    qi: *const qf_info_T,
    what: *const dict_T,
    action: c_int,
    newlist: *mut bool,
) -> c_int {
    unsafe {
        let mut qf_idx = (*qi).qf_curlist;

        let di = tv_dict_find(what, c"nr".as_ptr(), 2);
        if !di.is_null() {
            if (*di).di_tv.v_type == VarType::VAR_NUMBER {
                if (*di).di_tv.vval.v_number != 0 {
                    qf_idx = (*di).di_tv.vval.v_number as c_int - 1;
                }

                if (action == b' ' as c_int || action == b'a' as c_int)
                    && qf_idx == (*qi).qf_listcount
                {
                    *newlist = true;
                    qf_idx = if qf_stack_empty(qi) {
                        0
                    } else {
                        (*qi).qf_listcount - 1
                    };
                } else if qf_idx < 0 || qf_idx >= (*qi).qf_listcount {
                    return INVALID_QFIDX;
                } else if action != b' ' as c_int {
                    *newlist = false;
                }
            } else if (*di).di_tv.v_type == VarType::VAR_STRING
                && strequal((*di).di_tv.vval.v_string, c"$".as_ptr())
            {
                if !qf_stack_empty(qi) {
                    qf_idx = (*qi).qf_listcount - 1;
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
            let di = tv_dict_find(what, c"id".as_ptr(), 2);
            if !di.is_null() {
                if (*di).di_tv.v_type != VarType::VAR_NUMBER {
                    return INVALID_QFIDX;
                }
                return qf_id2nr(qi, (*di).di_tv.vval.v_number as c_uint);
            }
        }

        qf_idx
    }
}

unsafe fn qf_setprop_title(
    qi: *mut qf_info_T,
    qf_idx: c_int,
    what: *const dict_T,
    di: *const dictitem_T,
) -> c_int {
    unsafe {
        let qfl = qf_get_list(qi, qf_idx);
        if (*di).di_tv.v_type != VarType::VAR_STRING {
            return FAIL;
        }

        xfree((*qfl).qf_title.cast());
        (*qfl).qf_title = tv_dict_get_string(what, c"title".as_ptr(), true);
        if qf_idx == (*qi).qf_curlist {
            qf_update_win_titlevar(qi);
        }

        OK
    }
}

unsafe fn qf_setprop_items(
    qi: *mut qf_info_T,
    qf_idx: c_int,
    di: *mut dictitem_T,
    action: c_int,
) -> c_int {
    unsafe {
        if (*di).di_tv.v_type != VarType::VAR_LIST {
            return FAIL;
        }

        let title_save = xstrdup((*(*qi).qf_lists.add(qf_idx as usize)).qf_title);
        let retval = qf_add_entries(
            qi,
            qf_idx,
            (*di).di_tv.vval.v_list,
            title_save,
            if action == b' ' as c_int {
                b'a' as c_int
            } else {
                action
            },
        );
        xfree(title_save.cast());

        retval
    }
}

unsafe fn qf_setprop_items_from_lines(
    qi: *mut qf_info_T,
    qf_idx: c_int,
    what: *const dict_T,
    di: *mut dictitem_T,
    action: c_int,
) -> c_int {
    unsafe {
        let mut errorformat = p_efm;
        let mut retval = FAIL;

        let efm_di = tv_dict_find(what, c"efm".as_ptr(), 3);
        if !efm_di.is_null() {
            if (*efm_di).di_tv.v_type != VarType::VAR_STRING
                || (*efm_di).di_tv.vval.v_string.is_null()
            {
                return FAIL;
            }
            errorformat = (*efm_di).di_tv.vval.v_string;
        }

        if (*di).di_tv.v_type != VarType::VAR_LIST || (*di).di_tv.vval.v_list.is_null() {
            return FAIL;
        }

        if action == b'r' as c_int || action == b'u' as c_int {
            qf_free_items((*qi).qf_lists.add(qf_idx as usize));
        }
        if qf_init_ext(
            qi,
            qf_idx,
            ptr::null(),
            ptr::null_mut(),
            &raw mut (*di).di_tv,
            errorformat,
            false,
            0,
            0,
            ptr::null(),
            ptr::null_mut(),
        ) >= 0
        {
            retval = OK;
        }

        retval
    }
}

unsafe fn qf_setprop_context(qfl: *mut qf_list_T, di: *mut dictitem_T) -> c_int {
    unsafe {
        tv_free((*qfl).qf_ctx);
        let ctx = xcalloc(1, size_of::<typval_T>()).cast::<typval_T>();
        tv_copy(&raw const (*di).di_tv, ctx);
        (*qfl).qf_ctx = ctx;

        OK
    }
}

unsafe fn qf_setprop_curidx(
    qi: *mut qf_info_T,
    qfl: *mut qf_list_T,
    di: *const dictitem_T,
) -> c_int {
    unsafe {
        let mut newidx: c_int;

        if (*di).di_tv.v_type == VarType::VAR_STRING
            && !(*di).di_tv.vval.v_string.is_null()
            && c_strcmp((*di).di_tv.vval.v_string, c"$".as_ptr()) == 0
        {
            newidx = (*qfl).qf_count;
        } else {
            let mut denote = false;
            newidx = tv_get_number_chk(&raw const (*di).di_tv, &raw mut denote) as c_int;
            if denote {
                return FAIL;
            }
        }

        if newidx < 1 {
            return FAIL;
        }
        newidx = newidx.min((*qfl).qf_count);
        let old_qfidx = (*qfl).qf_index;
        let qf_ptr = get_nth_entry(qfl, newidx, &raw mut newidx);
        if qf_ptr.is_null() {
            return FAIL;
        }
        (*qfl).qf_ptr = qf_ptr;
        (*qfl).qf_index = newidx;

        if (*(*qi).qf_lists.add((*qi).qf_curlist as usize)).qf_id == (*qfl).qf_id {
            qf_win_pos_update(qi, old_qfidx);
        }
        OK
    }
}

unsafe fn qf_set_properties(
    qi: *mut qf_info_T,
    what: *const dict_T,
    action: c_int,
    title: *mut c_char,
) -> c_int {
    unsafe {
        let mut newlist = action == b' ' as c_int || qf_stack_empty(qi);
        let mut qf_idx = qf_setprop_get_qfidx(qi, what, action, &raw mut newlist);
        if qf_idx == INVALID_QFIDX {
            return FAIL;
        }

        if newlist {
            (*qi).qf_curlist = qf_idx;
            qf_new_list(qi, title);
            qf_idx = (*qi).qf_curlist;
        }

        let qfl = qf_get_list(qi, qf_idx);
        let mut retval = FAIL;
        let mut di = tv_dict_find(what, c"title".as_ptr(), 5);
        if !di.is_null() {
            retval = qf_setprop_title(qi, qf_idx, what, di);
        }
        di = tv_dict_find(what, c"items".as_ptr(), 5);
        if !di.is_null() {
            retval = qf_setprop_items(qi, qf_idx, di, action);
        }
        di = tv_dict_find(what, c"lines".as_ptr(), 5);
        if !di.is_null() {
            retval = qf_setprop_items_from_lines(qi, qf_idx, what, di, action);
        }
        di = tv_dict_find(what, c"context".as_ptr(), 7);
        if !di.is_null() {
            retval = qf_setprop_context(qfl, di);
        }
        di = tv_dict_find(what, c"idx".as_ptr(), 3);
        if !di.is_null() {
            retval = qf_setprop_curidx(qi, qfl, di);
        }
        di = tv_dict_find(what, c"quickfixtextfunc".as_ptr(), 16);
        if !di.is_null() {
            retval = qf_setprop_qftf(qfl, di);
        }

        if newlist || retval == OK {
            qf_list_changed(qfl);
        }
        if newlist {
            qf_update_buffer(qi, ptr::null_mut());
        }

        retval
    }
}

unsafe fn qf_free_stack(mut wp: *mut win_T, qi: *mut qf_info_T) {
    unsafe {
        let qfwin = qf_find_win(qi);

        if !qfwin.is_null() {
            if (*qi).qf_curlist < (*qi).qf_listcount {
                qf_free(qf_get_curlist(qi));
            }
            qf_update_buffer(qi, ptr::null_mut());
        }

        if !wp.is_null() && is_ll_window(wp) {
            let llwin = qf_find_win_with_loclist(qi);
            if !llwin.is_null() {
                wp = llwin;
            }
        }

        qf_free_all(wp);
        if wp.is_null() {
            (*qi).qf_curlist = 0;
            (*qi).qf_listcount = 0;
        } else if !qfwin.is_null() {
            let new_ll = qf_alloc_stack(QFLT_LOCATION, (*wp).w_onebuf_opt.wo_lhi as c_int);
            (*new_ll).qf_bufnr = (*(*qfwin).w_buffer).b_fnum;

            ll_free_all((&raw mut (*qfwin).w_llist_ref).cast::<*mut qf_info_T>());

            (*qfwin).w_llist_ref = new_ll.cast::<c_void>();
            if wp != qfwin {
                win_set_loclist(wp, new_ll);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_errorlist(
    wp: *mut win_T,
    list: *mut list_T,
    action: c_int,
    title: *mut c_char,
    what: *mut dict_T,
) -> c_int {
    unsafe {
        let qi = if !wp.is_null() {
            ll_get_or_alloc_list(wp)
        } else {
            ql_info
        };
        assert!(!qi.is_null());

        if action == b'f' as c_int {
            qf_free_stack(wp, qi);
            return OK;
        }

        if !list.is_null() && tv_list_len(list) != 0 && !what.is_null() {
            semsg!(
                gettext((&raw const e_invarg2).cast()),
                gettext(c"cannot have both a list and a \"what\" argument".as_ptr()),
            );
            return FAIL;
        }

        incr_quickfix_busy();

        let retval = if !what.is_null() {
            qf_set_properties(qi, what, action, title)
        } else {
            let r = qf_add_entries(qi, (*qi).qf_curlist, list, title, action);
            if r == OK {
                qf_list_changed(qf_get_curlist(qi));
            }
            r
        };

        decr_quickfix_busy();

        retval
    }
}

unsafe fn mark_quickfix_user_data(qi: *mut qf_info_T, copyID: c_int) -> bool {
    unsafe {
        let mut abort = false;
        let mut i: c_int = 0;
        while i < (*qi).qf_maxcount && !abort {
            let qfl = (*qi).qf_lists.add(i as usize);
            if (*qfl).qf_has_user_data {
                let mut j: c_int = 1;
                let mut qfp = (*qfl).qf_start;
                while !got_int && j <= (*qfl).qf_count && !qfp.is_null() {
                    let user_data = &raw mut (*qfp).qf_user_data;
                    if (*user_data).v_type != VarType::VAR_NUMBER
                        && (*user_data).v_type != VarType::VAR_STRING
                        && (*user_data).v_type != VarType::VAR_FLOAT
                    {
                        abort = abort
                            || set_ref_in_item(user_data, copyID, ptr::null_mut(), ptr::null_mut());
                    }
                    j += 1;
                    qfp = (*qfp).qf_next;
                }
            }
            i += 1;
        }
        abort
    }
}

unsafe fn mark_quickfix_ctx(qi: *mut qf_info_T, copyID: c_int) -> bool {
    unsafe {
        let mut abort = false;
        let mut i: c_int = 0;
        while i < (*qi).qf_maxcount && !abort {
            let qfl = (*qi).qf_lists.add(i as usize);
            let ctx = (*qfl).qf_ctx;
            if !ctx.is_null()
                && (*ctx).v_type != VarType::VAR_NUMBER
                && (*ctx).v_type != VarType::VAR_STRING
                && (*ctx).v_type != VarType::VAR_FLOAT
            {
                abort = set_ref_in_item(ctx, copyID, ptr::null_mut(), ptr::null_mut());
            }

            let cb = &raw mut (*qfl).qf_qftf_cb;
            abort = abort || set_ref_in_callback(cb, copyID, ptr::null_mut(), ptr::null_mut());
            i += 1;
        }
        abort
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_ref_in_quickfix(copyID: c_int) -> bool {
    unsafe {
        assert!(!ql_info.is_null());
        if mark_quickfix_ctx(ql_info, copyID)
            || mark_quickfix_user_data(ql_info, copyID)
            || set_ref_in_callback(&raw mut p_qftf, copyID, ptr::null_mut(), ptr::null_mut())
        {
            return true;
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut win = tab_firstwin(tp);
            while !win.is_null() {
                if !(*win).w_llist.is_null() {
                    let ll = (*win).w_llist.cast::<qf_info_T>();
                    if mark_quickfix_ctx(ll, copyID) || mark_quickfix_user_data(ll, copyID) {
                        return true;
                    }
                }

                if is_ll_window(win) {
                    let ll_ref = (*win).w_llist_ref.cast::<qf_info_T>();
                    if (*ll_ref).qf_refcount == 1
                        && (mark_quickfix_ctx(ll_ref, copyID)
                            || mark_quickfix_user_data(ll_ref, copyID))
                    {
                        return true;
                    }
                }
                win = (*win).w_next;
            }
            tp = (*tp).tp_next;
        }

        false
    }
}

unsafe fn cbuffer_get_auname(cmdidx: cmdidx_T) -> *const c_char {
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

unsafe fn cbuffer_process_args(
    eap: *mut exarg_T,
    bufp: *mut *mut buf_T,
    line1: *mut linenr_T,
    line2: *mut linenr_T,
) -> c_int {
    unsafe {
        let mut buf: *mut buf_T = ptr::null_mut();

        if *(*eap).arg == NUL {
            buf = curbuf;
        } else if *skipwhite(skipdigits((*eap).arg)) == NUL {
            buf = buflist_findnr(atoi((*eap).arg));
        }

        if buf.is_null() {
            emsg(gettext((&raw const e_invarg).cast()));
            return FAIL;
        }

        if (*buf).b_ml.ml_mfp.is_null() {
            emsg(gettext((&raw const e_buffer_is_not_loaded).cast()));
            return FAIL;
        }

        if (*eap).addr_count == 0 {
            (*eap).line1 = 1;
            (*eap).line2 = (*buf).b_ml.ml_line_count;
        }

        if (*eap).line1 < 1
            || (*eap).line1 > (*buf).b_ml.ml_line_count
            || (*eap).line2 < 1
            || (*eap).line2 > (*buf).b_ml.ml_line_count
        {
            emsg(gettext((&raw const e_invrange).cast()));
            return FAIL;
        }

        *line1 = (*eap).line1;
        *line2 = (*eap).line2;
        *bufp = buf;

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cbuffer(eap: *mut exarg_T) {
    unsafe {
        let au_name = cbuffer_get_auname((*eap).cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
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
        if cbuffer_process_args(eap, &raw mut buf, &raw mut line1, &raw mut line2) == FAIL {
            return;
        }

        let mut qf_title = qf_cmdtitle(*(*eap).cmdlinep);
        if !(*buf).b_sfname.is_null() {
            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                c"%s (%s)".as_ptr(),
                qf_title,
                (*buf).b_sfname,
            );
            qf_title = iobuff_ptr;
        }

        incr_quickfix_busy();

        let mut res = qf_init_ext(
            qi,
            (*qi).qf_curlist,
            ptr::null(),
            buf,
            ptr::null_mut(),
            p_efm,
            (*eap).cmdidx != CMD_caddbuffer && (*eap).cmdidx != CMD_laddbuffer,
            (*eap).line1,
            (*eap).line2,
            qf_title,
            ptr::null_mut(),
        );
        if qf_stack_empty(qi) {
            decr_quickfix_busy();
            return;
        }
        if res >= 0 {
            qf_list_changed(qf_get_curlist(qi));
        }
        let save_qfid = (*qf_get_curlist(qi)).qf_id;
        if !au_name.is_null() {
            let curbuf_old = curbuf;
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
            );
            if curbuf != curbuf_old {
                res = 0;
            }
        }
        if res > 0
            && ((*eap).cmdidx == CMD_cbuffer || (*eap).cmdidx == CMD_lbuffer)
            && qflist_valid(wp, save_qfid)
        {
            qf_jump_first(qi, save_qfid, (*eap).forceit);
        }

        decr_quickfix_busy();
    }
}

unsafe fn cexpr_get_auname(cmdidx: cmdidx_T) -> *const c_char {
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

unsafe fn trigger_cexpr_autocmd(cmdidx: c_int) -> c_int {
    unsafe {
        let au_name = cexpr_get_auname(cmdidx);
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
            )
            && aborting()
        {
            return FAIL;
        }
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cexpr_core(eap: *const exarg_T, tv: *mut typval_T) -> c_int {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();
        let qi = qf_cmd_get_or_alloc_stack(eap, &raw mut wp);

        if ((*tv).v_type == VarType::VAR_STRING && !(*tv).vval.v_string.is_null())
            || (*tv).v_type == VarType::VAR_LIST
        {
            let au_name = cexpr_get_auname((*eap).cmdidx);
            incr_quickfix_busy();
            let res = qf_init_ext(
                qi,
                (*qi).qf_curlist,
                ptr::null(),
                ptr::null_mut(),
                tv,
                p_efm,
                (*eap).cmdidx != CMD_caddexpr && (*eap).cmdidx != CMD_laddexpr,
                0,
                0,
                qf_cmdtitle(*(*eap).cmdlinep),
                ptr::null_mut(),
            );
            if qf_stack_empty(qi) {
                decr_quickfix_busy();
                return FAIL;
            }
            if res >= 0 {
                qf_list_changed(qf_get_curlist(qi));
            }
            let save_qfid = (*qf_get_curlist(qi)).qf_id;
            if !au_name.is_null() {
                apply_autocmds(
                    EVENT_QUICKFIXCMDPOST,
                    au_name.cast_mut(),
                    (*curbuf).b_fname,
                    true,
                    curbuf,
                );
            }
            if res > 0
                && ((*eap).cmdidx == CMD_cexpr || (*eap).cmdidx == CMD_lexpr)
                && qflist_valid(wp, save_qfid)
            {
                qf_jump_first(qi, save_qfid, (*eap).forceit);
            }
            decr_quickfix_busy();
            return OK;
        } else {
            emsg(gettext(c"E777: String or List expected".as_ptr()));
        }
        FAIL
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_cexpr(eap: *mut exarg_T) {
    unsafe {
        if trigger_cexpr_autocmd((*eap).cmdidx) == FAIL {
            return;
        }

        let tv = eval_expr((*eap).arg, eap.cast());
        if tv.is_null() {
            return;
        }

        let _ = cexpr_core(eap, tv);
        tv_free(tv);
    }
}

unsafe fn hgr_get_ll(new_ll: *mut bool) -> *mut qf_info_T {
    unsafe {
        let wp = if bt_help((*curwin).w_buffer) {
            curwin
        } else {
            qf_find_help_win()
        };

        let mut qi = if wp.is_null() {
            ptr::null_mut()
        } else {
            (*wp).w_llist.cast::<qf_info_T>()
        };
        if qi.is_null() {
            qi = qf_alloc_stack(QFLT_LOCATION, 1);
            *new_ll = true;
        }

        qi
    }
}

unsafe fn hgr_search_file(qfl: *mut qf_list_T, fname: *mut c_char, p_regmatch: *mut regmatch_T) {
    unsafe {
        let fd = os_fopen(fname, c"r".as_ptr()).cast::<FILE>();
        if fd.is_null() {
            return;
        }

        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
        let mut lnum: linenr_T = 1;
        while !vim_fgets(iobuff_ptr, IOSIZE as c_int, fd.cast()) && !got_int {
            let line = iobuff_ptr;

            if vim_regexec(p_regmatch, line, 0) {
                let mut l = c_strlen(line);
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
                    ((*p_regmatch).startp[0].offset_from(line) as c_int) + 1,
                    ((*p_regmatch).endp[0].offset_from(line) as c_int) + 1,
                    0,
                    ptr::null_mut(),
                    0,
                    1,
                    ptr::null_mut(),
                    1,
                ) == QF_FAIL
                {
                    got_int = true;
                    break;
                }
            }
            lnum += 1;
            line_breakcheck();
        }
        fclose(fd);
    }
}

unsafe fn hgr_search_files_in_dir(
    qfl: *mut qf_list_T,
    mut dirname: *mut c_char,
    p_regmatch: *mut regmatch_T,
    lang: *const c_char,
) {
    unsafe {
        let mut fcount: c_int = 0;
        let mut fnames: *mut *mut c_char = ptr::null_mut();

        add_pathsep(dirname);
        c_strcat(dirname, c"doc/*.\\(txt\\|??x\\)".as_ptr());
        if gen_expand_wildcards(
            1,
            &raw mut dirname,
            &raw mut fcount,
            &raw mut fnames,
            EW_FILE | EW_SILENT,
        ) == OK
            && fcount > 0
        {
            let mut fi: c_int = 0;
            while fi < fcount && !got_int {
                let f = *fnames.add(fi as usize);
                let flen = c_strlen(f);
                if !lang.is_null()
                    && flen >= 3
                    && strnicmp_asc(lang, f.add(flen - 3), 2) != 0
                    && !(strnicmp_asc(lang, c"en".as_ptr(), 2) == 0
                        && strnicmp_asc(c"txt".as_ptr(), f.add(flen - 3), 3) == 0)
                {
                    fi += 1;
                    continue;
                }

                hgr_search_file(qfl, f, p_regmatch);
                fi += 1;
            }
            FreeWild(fcount, fnames);
        }
    }
}

unsafe fn hgr_search_in_rtp(qfl: *mut qf_list_T, p_regmatch: *mut regmatch_T, lang: *const c_char) {
    unsafe {
        let mut p = p_rtp;
        let name_buff_ptr = (&raw mut NameBuff).cast::<c_char>();
        while *p != NUL && !got_int {
            copy_option_part(&raw mut p, name_buff_ptr, MAXPATHL, c",".as_ptr());
            hgr_search_files_in_dir(qfl, name_buff_ptr, p_regmatch, lang);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_helpgrep(eap: *mut exarg_T) {
    unsafe {
        let mut qi = ql_info;
        assert!(!qi.is_null());
        let au_name: *const c_char = match (*eap).cmdidx {
            CMD_helpgrep => c"helpgrep".as_ptr(),
            CMD_lhelpgrep => c"lhelpgrep".as_ptr(),
            _ => ptr::null(),
        };
        if !au_name.is_null()
            && apply_autocmds(
                EVENT_QUICKFIXCMDPRE,
                au_name.cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
            )
            && aborting()
        {
            return;
        }

        let mut updated = false;
        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast::<c_char>();

        let mut new_qi = false;
        if is_loclist_cmd((*eap).cmdidx) {
            qi = hgr_get_ll(&raw mut new_qi);
        }

        incr_quickfix_busy();

        let lang = check_help_lang((*eap).arg);
        let mut regmatch: regmatch_T = std::mem::zeroed();
        regmatch.regprog = vim_regcomp((*eap).arg, RE_MAGIC + RE_STRING);
        regmatch.rm_ic = false;
        if !regmatch.regprog.is_null() {
            qf_new_list(qi, qf_cmdtitle(*(*eap).cmdlinep));
            let qfl = qf_get_curlist(qi);

            hgr_search_in_rtp(qfl, &raw mut regmatch, lang);

            vim_regfree(regmatch.regprog);

            (*qfl).qf_nonevalid = false;
            (*qfl).qf_ptr = (*qfl).qf_start;
            (*qfl).qf_index = 1;
            qf_list_changed(qfl);
            updated = true;
        }

        if p_cpo == (&raw mut empty_string_option).cast::<c_char>() {
            p_cpo = save_cpo;
        } else {
            if *p_cpo == NUL {
                set_option_value_give_err(kOptCpoptions, cstr_as_obj(save_cpo), 0);
            }
            free_string_option(save_cpo);
        }

        if updated {
            qf_update_buffer(qi, ptr::null_mut());
        }

        if !au_name.is_null() {
            apply_autocmds(
                EVENT_QUICKFIXCMDPOST,
                au_name.cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
            );
            if !new_qi && is_ll_stack(qi) && qf_find_win_with_loclist(qi).is_null() {
                decr_quickfix_busy();
                return;
            }
        }

        if !qf_list_empty(qf_get_curlist(qi)) {
            qf_jump(qi, 0, 0, 0);
        } else {
            semsg!(gettext((&raw const e_nomatch2).cast()), (*eap).arg);
        }

        decr_quickfix_busy();

        if (*eap).cmdidx == CMD_lhelpgrep {
            if !bt_help((*curwin).w_buffer) || (*curwin).w_llist.cast::<qf_info_T>() == qi {
                if new_qi {
                    ll_free_all(&raw mut qi);
                }
            } else if (*curwin).w_llist.is_null() && new_qi {
                (*curwin).w_llist = qi.cast::<c_void>();
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_quickfix() {
    unsafe {
        qf_free_all(ptr::null_mut());
        let mut tab = first_tabpage;
        while !tab.is_null() {
            let mut win = tab_firstwin(tab);
            while !win.is_null() {
                qf_free_all(win);
                win = (*win).w_next;
            }
            tab = (*tab).tp_next;
        }

        callback_free(&raw mut p_qftf);
        ga_clear(&raw mut qfga);
    }
}

unsafe fn get_qf_loc_list(
    is_qf: bool,
    wp: *mut win_T,
    what_arg: *mut typval_T,
    rettv: *mut typval_T,
) {
    unsafe {
        if (*what_arg).v_type == VarType::VAR_UNKNOWN {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            if is_qf || !wp.is_null() {
                get_errorlist(ptr::null_mut(), wp, -1, 0, (*rettv).vval.v_list);
            }
        } else {
            tv_dict_alloc_ret(rettv);
            if is_qf || !wp.is_null() {
                if (*what_arg).v_type == VarType::VAR_DICT {
                    let d = (*what_arg).vval.v_dict;
                    if !d.is_null() {
                        qf_get_properties(wp, d, (*rettv).vval.v_dict);
                    }
                } else {
                    emsg(gettext((&raw const e_dictreq).cast()));
                }
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getloclist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let wp = find_win_by_nr_or_id(argvars);
        get_qf_loc_list(false, wp, argvars.add(1), rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getqflist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_qf_loc_list(true, ptr::null_mut(), argvars, rettv);
    }
}

unsafe fn set_qf_ll_list(wp: *mut win_T, args: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let mut title: *const c_char = ptr::null();
        let mut action: c_char = b' ' as c_char;
        (*rettv).vval.v_number = -1;
        let mut what: *mut dict_T = ptr::null_mut();

        let list_arg = args;
        if (*list_arg).v_type != VarType::VAR_LIST {
            emsg(gettext((&raw const e_listreq).cast()));
            return;
        } else if set_qf_ll_recursive != 0 {
            emsg(gettext((&raw const e_au_recursive).cast()));
            return;
        }

        let action_arg = args.add(1);
        if (*action_arg).v_type != VarType::VAR_UNKNOWN {
            if (*action_arg).v_type != VarType::VAR_STRING {
                emsg(gettext((&raw const e_string_required).cast()));
                return;
            }
            let act = tv_get_string_chk(action_arg);
            if (*act == b'a' as c_char
                || *act == b'r' as c_char
                || *act == b'u' as c_char
                || *act == b' ' as c_char
                || *act == b'f' as c_char)
                && *act.add(1) == NUL
            {
                action = *act;
            } else {
                semsg!(gettext(E_INVACT.as_ptr()), act);
                return;
            }

            let what_arg = args.add(2);
            if (*what_arg).v_type != VarType::VAR_UNKNOWN {
                if (*what_arg).v_type == VarType::VAR_STRING {
                    title = tv_get_string_chk(what_arg);
                    if title.is_null() {
                        return;
                    }
                } else if (*what_arg).v_type == VarType::VAR_DICT
                    && !(*what_arg).vval.v_dict.is_null()
                {
                    what = (*what_arg).vval.v_dict;
                } else {
                    emsg(gettext((&raw const e_dictreq).cast()));
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

        set_qf_ll_recursive += 1;
        let l = (*list_arg).vval.v_list;
        if set_errorlist(wp, l, c_int::from(action), title.cast_mut(), what) == OK {
            (*rettv).vval.v_number = 0;
        }
        set_qf_ll_recursive -= 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setloclist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;

        let win = find_win_by_nr_or_id(argvars);
        if !win.is_null() {
            set_qf_ll_list(win, argvars.add(1), rettv);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setqflist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        set_qf_ll_list(ptr::null_mut(), argvars, rettv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efm_option_part_len_handles_escapes_and_commas() {
        unsafe {
            assert_eq!(efm_option_part_len(c"%f:%l:%m,%f:%l".as_ptr()), 8);
            assert_eq!(efm_option_part_len(c"%f\\,bar:%l,%m".as_ptr()), 10);
        }
    }

    #[test]
    fn qf_entry_pos_comparisons_respect_linewise_mode() {
        unsafe {
            let mut qfp: qfline_T = std::mem::zeroed();
            qfp.qf_lnum = 10;
            qfp.qf_col = 5;
            let pos = pos_T {
                lnum: 10,
                col: 5,
                coladd: 0,
            };
            assert!(!qf_entry_after_pos(&raw const qfp, &raw const pos, true));
            assert!(!qf_entry_after_pos(&raw const qfp, &raw const pos, false));
            assert!(qf_entry_on_or_after_pos(
                &raw const qfp,
                &raw const pos,
                true
            ));
            assert!(qf_entry_on_or_after_pos(
                &raw const qfp,
                &raw const pos,
                false
            ));
            assert!(qf_entry_on_or_before_pos(
                &raw const qfp,
                &raw const pos,
                true
            ));
            assert!(qf_entry_on_or_before_pos(
                &raw const qfp,
                &raw const pos,
                false
            ));
        }
    }

    #[test]
    fn entry_is_closer_to_target_compares_file_line_and_column() {
        unsafe {
            let mut e1: qfline_T = std::mem::zeroed();
            let mut e2: qfline_T = std::mem::zeroed();
            e1.qf_fnum = 1;
            e1.qf_lnum = 12;
            e1.qf_col = 4;
            e2.qf_fnum = 1;
            e2.qf_lnum = 15;
            e2.qf_col = 2;

            assert!(entry_is_closer_to_target(
                &raw mut e1,
                &raw mut e2,
                1,
                10,
                1
            ));
            assert!(!entry_is_closer_to_target(
                &raw mut e2,
                &raw mut e1,
                1,
                10,
                1
            ));

            e2.qf_lnum = 12;
            assert!(entry_is_closer_to_target(
                &raw mut e2,
                &raw mut e1,
                1,
                12,
                1
            ));
        }
    }
}
