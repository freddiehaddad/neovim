//! Functions for command-line Ex commands (`src/nvim/ex_cmds.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::bool_comparison,
    clippy::collapsible_if,
    clippy::enum_variant_names,
    clippy::if_same_then_else,
    clippy::missing_safety_doc,
    clippy::needless_bool_assign,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::useless_conversion
)]

#[allow(unused_imports)]
use crate::buffer::{
    buf_clear_file, buf_ensure_loaded, buf_freeall, buf_hide, buf_name_changed, buflist_altfpos,
    buflist_findfmark, buflist_findnr, buflist_new, close_buffer, do_autochdir, do_modelines,
    fileinfo, get_winopts, handle_swap_exists, maketitle, no_write_message, no_write_message_buf,
    open_buffer, set_buflisted, setaltfname, setfname,
};
#[allow(unused_imports)]
use crate::drawscreen::number_width;
use crate::errors::*;
#[allow(unused_imports)]
use crate::ex_cmds2::{check_fname, dialog_changed};
#[allow(unused_imports)]
use crate::ex_docmd::{before_quit_all, ex_may_print};
#[allow(unused_imports)]
use crate::ex_docmd::{check_nextcmd, do_cmdline, not_exiting};
#[allow(unused_imports)]
use crate::ex_eval::{aborting, should_abort};
#[allow(unused_imports)]
use crate::fileio::{
    buf_check_timestamp, set_file_options, set_forced_fenc, vim_tempname, write_lnum_adjust,
};
#[allow(unused_imports)]
use crate::fold::foldMoveRange;
#[allow(unused_imports)]
use crate::fold::{foldUpdate, foldUpdateAll};
use crate::globals::*;
#[allow(unused_imports)]
use crate::input::redo_append_lit;
#[allow(unused_imports)]
use crate::insert::beginline;
#[allow(unused_imports)]
use crate::lua::executor::nlua_call_excmd;
#[allow(unused_imports)]
use crate::main_init::getout;
#[allow(unused_imports)]
use crate::message::{
    emsg, message_filtered, messaging, msg, msg_check_for_delay, msg_clr_eos, msg_end,
    msg_ext_no_fast, msg_ext_set_kind, msg_multiline, msg_outtrans, msg_prt_line, msg_putchar,
    msg_puts, msg_puts_hl, msg_sb_eol, msg_start, msgmore, set_keep_msg, wait_return,
};
use crate::message::{semsg, smsg};
#[allow(unused_imports)]
use crate::r#move::changed_line_abv_curs;
#[allow(unused_imports)]
use crate::normal::reset_VIsual;
#[allow(unused_imports)]
use crate::option::copy_option_part;
#[allow(unused_imports)]
use crate::option::set_option_direct;
#[allow(unused_imports)]
use crate::option::{buf_copy_options, get_fileformat, shortmess};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::regexp::{
    re_multiline, skip_regexp, skip_regexp_err, skip_regexp_ex, vim_regcomp, vim_regexec,
    vim_regexec_multi, vim_regfree,
};
#[allow(unused_imports)]
use crate::strings::{concat_str, vim_strchr, vim_strsave_escaped, xstrnsave};
#[allow(unused_imports)]
use crate::ui::{ui_cursor_goto, ui_cursor_shape, ui_has};
#[allow(unused_imports)]
use crate::window::update_cwd;
#[allow(unused_imports)]
use crate::window::{
    check_can_set_curbuf_forceit, check_lnums, curwin_init, win_enter, win_split, win_valid,
};
use std::ffi::{CStr, c_char, c_double, c_int, c_long, c_ulong, c_void};
use std::ptr;

use crate::api::command::{exarg_T, regmatch_T};
use crate::api::private::validate::{Object, ObjectData, ObjectType};
use crate::api::tabpage::{lpos_T, pos_T, win_T};
use crate::arglist::{check_arg_idx, do_argfile};
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFENTER, EVENT_BUFFILEPOST, EVENT_BUFFILEPRE, EVENT_BUFLEAVE,
    EVENT_BUFWINENTER, EVENT_SHELLCMDPOST, EVENT_SHELLFILTERPOST, apply_autocmds,
    apply_autocmds_retval, augroup_exists, do_doautocmd,
};
use crate::buffer::{
    BFA_KEEP_UNDO, BLN_CURBUF, BLN_LISTED, BLN_NOCURWIN, DOBUF_UNLOAD, GETFILE_ERROR,
    GETFILE_NOT_WRITTEN, GETFILE_OPEN_OTHER, GETFILE_SAME_FILE, bt_dontwrite, bt_nofilename,
    bufref_T, bufref_valid, fname_expand, set_bufref,
};
use crate::buffer_updates::buf_updates_send_changes;
use crate::bufwrite::buf_write;
use crate::change::{
    appended_lines, appended_lines_mark, changed_bytes, changed_lines, del_lines, deleted_lines,
    deleted_lines_mark,
};
use crate::channel::channel_job_running;
#[cfg(not(unix))]
use crate::charset::rem_backslash;
use crate::charset::{
    getdigits_int, skiptobin, skiptodigit, skiptohex, skiptowhite, skipwhite, transchar,
    transchar_nonprint, vim_isIDc, vim_isprintc, vim_str2nr,
};
use crate::cmdexpand::cmdidx_T;
use crate::cmdhist::{HIST_SEARCH, add_to_history};
use crate::cursor::{
    check_cursor, check_cursor_col, check_cursor_lnum, check_pos, coladvance, colnr_T,
    get_cursor_line_ptr, get_cursor_pos_ptr, linenr_T,
};
use crate::decoration::bufhl_add_hl_pos_offset;
use crate::dialog::vim_dialog_yesno;
use crate::diff::{diff_buf_add, diff_invalidate};
use crate::digraph::{get_digraph_for_char, keymap_init};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, redraw_curbuf_later, redraw_later,
    show_cursor_info_later, update_screen,
};
use crate::eval::buffer::varnumber_T;
use crate::eval::deprecated::{buf_T, synblock_T};
use crate::eval::vars::{get_vim_var_str, set_vim_var_string};
use crate::ex_cmds2::{autowrite, autowrite_all, buf_write_all, check_changed};
use crate::ex_eval::cstack_T;
use crate::ex_getln::{curbuf_locked, gotocmdline, text_locked};
use crate::extmark::{bcount_t, extmark_move_region, extmark_splice};
use crate::fileio::{READ_FILTER, READ_KEEP_UNDO, READ_NOWINENTER};
use crate::fold::hasAnyFolding;
use crate::help::prepare_help_buffer;
use crate::highlight_group::syn_check_group;
use crate::indent::{get_indent, get_indent_lnum, set_indent};
use crate::input::redo_append_str;
use crate::keycodes::{CAR, ESC, NL, TAB};
use crate::kvec::KVec;
use crate::map_glyph_cache::NvimString;
use crate::mark::{mark_adjust, mark_adjust_nofold, set_last_cursor, setpcmark};
use crate::r#match::{regmmatch_T, regprog_T};
use crate::math::{FAIL, OK};
use crate::mbyte::{
    utf_char2bytes, utf_iscomposing_first, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::memline::{
    makeswapname, ml_append, ml_append_buf, ml_clearmarked, ml_delete, ml_delete_flags,
    ml_find_line_or_offset, ml_firstmarked, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len,
    ml_replace, ml_replace_buf, ml_setmarked,
};
use crate::memory::{xcalloc, xfree, xmalloc, xmemdupz, xrealloc, xstrdup, xstrlcat, xstrlcpy};
use crate::mouse::setmouse;
use crate::r#move::{
    changed_window_setting, do_check_cursorbind, invalidate_botline_win, scrolldown_clamp,
    scrollup_clamp, update_topline, validate_cursor,
};
use crate::msgpack_rpc::packer::handle_T;
use crate::ops::do_join;
use crate::optionstr::OptIndex;
#[cfg(unix)]
use crate::os::fs::os_nodetype;
use crate::os::fs::{os_file_is_writable, os_file_mkdir, os_isdir, os_path_exists, os_remove};
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::os::shell::call_shell;
use crate::os::time::{os_time, time_t};
use crate::path::{fix_fname, invocation_path_tail, path_fix_case};
use crate::plines::{linetabsize_col, plines_m_win_fill};
use crate::profile::{profile_passed_limit, profile_setlimit, profile_zero, proftime_T};
use crate::search::{get_search_pat, last_search_pat, save_re_pat, search_regcomp};
use crate::shada::SubReplacementString;
use crate::spell::parse_spelllang;
use crate::state::{MODE_CMDLINE, MODE_INSERT, MODE_LANGMAP};
use crate::terminal::{Terminal, terminal_check_size, terminal_running};
use crate::undo::{
    u_inssub, u_save, u_save_cursor, u_savecommon, u_savedel, u_savesub, u_sync, u_unchanged,
};
use crate::winfloat::{win_float_anchor_preview, win_float_special, win_float_update_preview};

pub type OptInt = i64;
pub type UIExtension = c_int;
pub type float_T = c_double;
pub type ExtmarkOp = c_int;
pub type magic_T = c_int;
pub type LineGetter = Option<unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char>;

const NUL: c_char = 0;

const Ctrl_C: c_int = 3;
const Ctrl_E: c_int = 5;
const Ctrl_Y: c_int = 25;

const MAXCOL: colnr_T = 0x7fffffff;
const MAXLNUM: linenr_T = 0x7fffffff;
const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const MSG_BUF_LEN: usize = 480;
const DIALOG_MSG_SIZE: usize = 1000;

const EOL_MAC: c_int = 2;

const BL_WHITE: c_int = 1;
const BL_SOL: c_int = 2;
const BL_FIX: c_int = 4;

const STR2NR_BIN: c_int = 1 << 0;
const STR2NR_OCT: c_int = 1 << 1;
const STR2NR_HEX: c_int = 1 << 2;
const STR2NR_FORCE: c_int = 1 << 7;

const RE_SEARCH: c_int = 0;
const RE_SUBST: c_int = 1;
const RE_BOTH: c_int = 2;
const RE_LAST: c_int = 2;
const RE_MAGIC: c_int = 1;
const SEARCH_HIS: c_int = 0x20;

const REGSUB_COPY: c_int = 1;
const REGSUB_MAGIC: c_int = 2;
const REGSUB_BACKSLASH: c_int = 4;

const kExtmarkNOOP: ExtmarkOp = 0;
const kExtmarkUndo: ExtmarkOp = 1;

const ML_EMPTY: c_int = 0x01;
const ML_DEL_MESSAGE: c_int = 1;

const CMOD_CONFIRM: c_int = 0x0080;
const CMOD_KEEPALT: c_int = 0x0100;
const CMOD_KEEPMARKS: c_int = 0x0200;
const CMOD_LOCKMARKS: c_int = 0x0800;
const CMOD_KEEPPATTERNS: c_int = 0x1000;

const EXFLAG_LIST: c_int = 0x01;
const EXFLAG_NR: c_int = 0x02;
const EXFLAG_PRINT: c_int = 0x04;

const kShellOptFilter: c_int = 1;
const kShellOptDoOut: c_int = 4;
const kShellOptRead: c_int = 16;
const kShellOptWrite: c_int = 32;

const kUIMessages: UIExtension = 4;

const HLF_N: c_int = 12;
const HLF_R: c_int = 18;

const kShmFileinfo: c_int = b'F' as c_int;
const kShmOverall: c_int = b'O' as c_int;

const kCpoAltwrite: c_int = b'A' as c_int;
const kCpoOvernew: c_int = b'O' as c_int;
const kCpoRemmark: c_int = b'R' as c_int;
const kCpoUndo: c_int = b'u' as c_int;

const NODE_OTHER: c_int = 2;

const VIM_QUESTION: c_int = 4;
const VIM_YES: c_int = 2;

const BF_READERR: c_int = 0x02;
const BF_NEW: c_int = 0x04;
const BF_NOTEDITED: c_int = 0x08;
const BF_NEVERLOADED: c_int = 0x10;
const BF_CHECK_RO: c_int = 0x0800;

const CCGD_AW: c_int = 1;
const CCGD_MULTWIN: c_int = 2;
const CCGD_FORCEIT: c_int = 4;
const CCGD_EXCMD: c_int = 16;

const BCO_ENTER: c_int = 1;
const SEA_DIALOG: c_int = 1;
const SEA_QUIT: c_int = 2;
const OPT_WINONLY: c_int = 0x08;
const DOCMD_VERBOSE: c_int = 0x01;
const DOCMD_NOWAIT: c_int = 0x02;
const KEYMAP_INIT: i16 = 1;
const kCdCauseBuffer: c_int = 2;
const VV_SWAPCOMMAND: c_int = 49;
const B_IMODE_LMAP: OptInt = 1;
const kWinPreview: c_int = 2;
const SID_NONE: c_int = -6;

const kOptFoldcolumn: OptIndex = 102;
const kOptInccommand: OptIndex = 145;
const kOptShortmess: OptIndex = 271;

pub const ECMD_HIDE: c_int = 0x01;
pub const ECMD_SET_HELP: c_int = 0x02;
pub const ECMD_OLDBUF: c_int = 0x04;
pub const ECMD_FORCEIT: c_int = 0x08;
pub const ECMD_ADDBUF: c_int = 0x10;
pub const ECMD_ALTBUF: c_int = 0x20;
pub const ECMD_NOWINENTER: c_int = 0x40;

pub const ECMD_LASTL: linenr_T = 0;
pub const ECMD_LAST: linenr_T = -1;
pub const ECMD_ONE: linenr_T = 1;

pub const VGR_GLOBAL: c_int = 1;
pub const VGR_NOJUMP: c_int = 2;
pub const VGR_FUZZY: c_int = 4;

const CASE_INSENSITIVE_FILENAME: bool = cfg!(any(target_os = "windows", target_os = "macos"));

const E_NON_NUMERIC_ARGUMENT_TO_Z: &CStr = c"E144: Non-numeric argument to :z";

/// Case matching style to use for `:substitute`.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SubIgnoreType {
    kSubHonorOptions = 0,
    kSubIgnoreCase = 1,
    kSubMatchCase = 2,
}

/// Flags kept between calls to `:substitute`.
#[repr(C)]
#[derive(Clone, Copy)]
struct subflags_T {
    do_all: bool,
    do_ask: bool,
    do_count: bool,
    do_error: bool,
    do_print: bool,
    do_list: bool,
    do_number: bool,
    do_ic: SubIgnoreType,
}

/// Partial result of a substitution during `:substitute`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SubResult {
    start: lpos_T,
    end: lpos_T,
    pre_match: linenr_T,
}

/// Collected results of a substitution for showing them in the preview window.
#[repr(C)]
struct PreviewLines {
    subresults: KVec<SubResult>,
    lines_needed: linenr_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SortLine {
    start_col_nr: varnumber_T,
    end_col_nr: varnumber_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SortNum {
    value: varnumber_T,
    is_number: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
union SortUnion {
    line: SortLine,
    num: SortNum,
    value_flt: float_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct sorti_T {
    lnum: linenr_T,
    st_u: SortUnion,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LineData {
    start_col: c_int,
    start: lpos_T,
    end: lpos_T,
    matchcols: c_int,
    matchbytes: bcount_t,
    subcols: c_int,
    subbytes: bcount_t,
    lnum_before: linenr_T,
    lnum_after: linenr_T,
}

static mut sortbuf1: *mut c_char = ptr::null_mut();
static mut sortbuf2: *mut c_char = ptr::null_mut();

static mut sort_lc: bool = false;
static mut sort_ic: bool = false;
static mut sort_nr: c_int = 0;
static mut sort_rx: bool = false;
static mut sort_flt: bool = false;
static mut sort_abort: bool = false;

static mut prevcmd: *mut c_char = ptr::null_mut();
static mut global_need_msg_kind: bool = false;
static mut append_indent: c_int = 0;

static mut old_sub: SubReplacementString = SubReplacementString {
    sub: ptr::null_mut(),
    timestamp: 0,
    additional_data: ptr::null_mut(),
};

static mut global_need_beginline: bool = false;

static mut subflags: subflags_T = subflags_T {
    do_all: false,
    do_ask: false,
    do_count: false,
    do_error: true,
    do_print: false,
    do_list: false,
    do_number: false,
    do_ic: SubIgnoreType::kSubHonorOptions,
};

static mut pre_hl_id: c_int = 0;

unsafe extern "C" {

    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn ngettext(msgid1: *const c_char, msgid2: *const c_char, n: c_ulong) -> *mut c_char;

    fn vim_regsub_multi(
        rmp: *mut regmmatch_T,
        lnum: linenr_T,
        source: *const c_char,
        dest: *mut c_char,
        destlen: colnr_T,
        flags: c_int,
    ) -> c_int;
    fn regtilde(source: *const c_char, magic: c_int, preview: bool) -> *mut c_char;

    fn buflist_findname(ffname: *const c_char) -> *mut buf_T;
    fn otherfile(ffname: *const c_char) -> bool;
    fn prompt_for_input(
        prompt: *const c_char,
        hl_id: c_int,
        one_key: bool,
        mouse_used: *mut bool,
    ) -> c_int;
    fn readfile(
        fname: *const c_char,
        sfname: *const c_char,
        from: linenr_T,
        lines_to_skip: linenr_T,
        lines_to_read: linenr_T,
        eap: *mut exarg_T,
        flags: c_int,
        silent: bool,
    ) -> c_int;

    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn vim_snprintf_add(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn vim_snprintf_safelen(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> usize;
    #[cfg(windows)]
    fn vim_stricmp(s1: *const c_char, s2: *const c_char) -> c_int;
    #[cfg(not(windows))]
    fn strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;

    fn atoi(s: *const c_char) -> c_int;
    fn atol(s: *const c_char) -> c_long;
    fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double;
    fn strcoll(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
    fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    fn strchr(s: *const c_char, c: c_int) -> *const c_char;
    fn strlen(s: *const c_char) -> usize;
    fn snprintf(s: *mut c_char, n: usize, format: *const c_char, ...) -> c_int;
    fn qsort(
        base: *mut c_void,
        nmemb: usize,
        size: usize,
        compar: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int,
    );
    fn time(tloc: *mut time_t) -> time_t;
    fn isalpha(c: c_int) -> c_int;
}

#[inline]
unsafe fn stricmp_c(s1: *const c_char, s2: *const c_char) -> c_int {
    #[cfg(windows)]
    unsafe {
        vim_stricmp(s1, s2)
    }
    #[cfg(not(windows))]
    unsafe {
        strcasecmp(s1, s2)
    }
}

#[inline]
const fn ascii_islower(c: c_int) -> bool {
    (c as u32) >= (b'a' as u32) && (c as u32) <= (b'z' as u32)
}

#[inline]
const fn ascii_isupper(c: c_int) -> bool {
    (c as u32) >= (b'A' as u32) && (c as u32) <= (b'Z' as u32)
}

#[inline]
const fn ascii_isalpha(c: c_int) -> bool {
    ascii_isupper(c) || ascii_islower(c)
}

#[inline]
const fn ascii_isdigit(c: c_int) -> bool {
    c >= (b'0' as c_int) && c <= (b'9' as c_int)
}

#[inline]
const fn ascii_isalnum(c: c_int) -> bool {
    ascii_isalpha(c) || ascii_isdigit(c)
}

#[inline]
const fn ascii_iswhite(c: c_int) -> bool {
    c == (b' ' as c_int) || c == (b'\t' as c_int)
}

#[inline]
const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        NvimString {
            data: s.cast_mut(),
            size: unsafe { strlen(s) },
        }
    }
}

#[inline]
const fn cbuf_as_string(s: *mut c_char, len: usize) -> NvimString {
    NvimString { data: s, size: len }
}

#[inline]
unsafe fn cbuf_to_string(s: *const c_char, len: usize) -> NvimString {
    NvimString {
        data: unsafe { xmemdupz(s.cast::<c_void>(), len).cast::<c_char>() },
        size: len,
    }
}

#[inline]
unsafe fn bufIsChanged(buf: *const buf_T) -> bool {
    unsafe { !bt_dontwrite(buf) && ((*buf).b_changed != 0 || file_ff_differs(buf, true)) }
}

#[inline]
unsafe fn curbufIsChanged() -> bool {
    unsafe { bufIsChanged(curbuf) }
}

unsafe extern "C" {
    fn file_ff_differs(buf: *const buf_T, ignore_empty: bool) -> bool;
}

#[inline]
unsafe fn cstr_as_obj(s: *const c_char) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData {
            string: unsafe { cstr_as_string(s) },
        },
    }
}

#[inline]
unsafe fn api_clear_string(value: &mut NvimString) {
    unsafe {
        xfree(value.data.cast::<c_void>());
    }
    value.data = ptr::null_mut();
    value.size = 0;
}

#[inline]
unsafe fn bt_dontwrite_msg(buf: *const buf_T) -> bool {
    unsafe {
        if bt_dontwrite(buf) {
            emsg(gettext(
                c"E382: Cannot write, 'buftype' option is set".as_ptr(),
            ));
            return true;
        }
        false
    }
}

#[inline]
unsafe fn modifiable(buf: *const buf_T) -> bool {
    unsafe { (*buf).b_p_ma != 0 }
}

/// `":ascii"` and `"ga"` implementation.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_ascii(_eap: *mut exarg_T) {
    unsafe {
        let data = get_cursor_pos_ptr();
        let len = utfc_ptr2len(data) as usize;

        if len == 0 {
            msg(c"NUL".as_ptr(), 0);
            return;
        }

        let mut need_clear = true;
        msg_sb_eol();
        msg_start();

        let mut c = utf_ptr2char(data);
        let mut off: usize = 0;
        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();

        if c < 0x80 {
            if c == NL {
                c = NUL as c_int;
            }
            let cval = if c == CAR && get_fileformat(curbuf) == EOL_MAC {
                NL
            } else {
                c
            };
            let mut buf1: [c_char; 20] = [0; 20];
            if vim_isprintc(c) && (c < b' ' as c_int || c > b'~' as c_int) {
                let mut buf3: [c_char; 7] = [0; 7];
                transchar_nonprint(curbuf, buf3.as_mut_ptr(), c);
                vim_snprintf(buf1.as_mut_ptr(), 20, c"  <%s>".as_ptr(), buf3.as_ptr());
            } else {
                buf1[0] = NUL;
            }
            let buf2: [c_char; 20] = [0; 20];

            let dig = get_digraph_for_char(cval);
            if !dig.is_null() {
                vim_snprintf(
                    iobuff_ptr,
                    IOSIZE,
                    gettext(c"<%s>%s%s  %d,  Hex %02x,  Oct %03o, Digr %s".as_ptr()),
                    transchar(c),
                    buf1.as_ptr(),
                    buf2.as_ptr(),
                    cval,
                    cval,
                    cval,
                    dig,
                );
            } else {
                vim_snprintf(
                    iobuff_ptr,
                    IOSIZE,
                    gettext(c"<%s>%s%s  %d,  Hex %02x,  Octal %03o".as_ptr()),
                    transchar(c),
                    buf1.as_ptr(),
                    buf2.as_ptr(),
                    cval,
                    cval,
                    cval,
                );
            }

            msg_multiline(
                cstr_as_string(iobuff_ptr),
                0,
                true,
                false,
                &raw mut need_clear,
            );

            off += utf_ptr2len(data) as usize;
        }

        while off < len {
            c = utf_ptr2char(data.add(off));

            let mut iobuff_len: usize = 0;
            if off > 0 {
                *iobuff_ptr.add(iobuff_len) = b' ' as c_char;
                iobuff_len += 1;
            }
            *iobuff_ptr.add(iobuff_len) = b'<' as c_char;
            iobuff_len += 1;
            if utf_iscomposing_first(c) {
                *iobuff_ptr.add(iobuff_len) = b' ' as c_char;
                iobuff_len += 1;
            }
            iobuff_len += utf_char2bytes(c, iobuff_ptr.add(iobuff_len)) as usize;

            let dig = get_digraph_for_char(c);
            if !dig.is_null() {
                let fmt = if c < 0x10000 {
                    gettext(c"> %d, Hex %04x, Oct %o, Digr %s".as_ptr())
                } else {
                    gettext(c"> %d, Hex %08x, Oct %o, Digr %s".as_ptr())
                };
                vim_snprintf(
                    iobuff_ptr.add(iobuff_len),
                    IOSIZE - iobuff_len,
                    fmt,
                    c,
                    c,
                    c,
                    dig,
                );
            } else {
                let fmt = if c < 0x10000 {
                    gettext(c"> %d, Hex %04x, Octal %o".as_ptr())
                } else {
                    gettext(c"> %d, Hex %08x, Octal %o".as_ptr())
                };
                vim_snprintf(
                    iobuff_ptr.add(iobuff_len),
                    IOSIZE - iobuff_len,
                    fmt,
                    c,
                    c,
                    c,
                );
            }

            msg_multiline(
                cstr_as_string(iobuff_ptr),
                0,
                true,
                false,
                &raw mut need_clear,
            );

            off += utf_ptr2len(data.add(off)) as usize;
        }

        if need_clear {
            msg_clr_eos();
        }
        msg_end();
    }
}

/// `":left"`, `":center"` and `":right"`: align text.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_align(eap: *mut exarg_T) {
    unsafe {
        let mut indent: c_int = 0;
        let mut new_indent: c_int;

        if (*curwin).w_onebuf_opt.wo_rl != 0 {
            if (*eap).cmdidx == cmdidx_T::CMD_right as c_int {
                (*eap).cmdidx = cmdidx_T::CMD_left as c_int;
            } else if (*eap).cmdidx == cmdidx_T::CMD_left as c_int {
                (*eap).cmdidx = cmdidx_T::CMD_right as c_int;
            }
        }

        let mut width = atoi((*eap).arg);
        let save_curpos = (*curwin).w_cursor;
        if (*eap).cmdidx == cmdidx_T::CMD_left as c_int {
            if width >= 0 {
                indent = width;
            }
        } else {
            if width <= 0 {
                width = (*curbuf).b_p_tw as c_int;
            }
            if width == 0 && (*curbuf).b_p_wm > 0 {
                width = (*curwin).w_view_width - (*curbuf).b_p_wm as c_int;
            }
            if width <= 0 {
                width = 80;
            }
        }

        if u_save((*eap).line1 - 1, (*eap).line2 + 1) == FAIL {
            return;
        }

        (*curwin).w_cursor.lnum = (*eap).line1;
        while (*curwin).w_cursor.lnum <= (*eap).line2 {
            if (*eap).cmdidx == cmdidx_T::CMD_left as c_int {
                new_indent = indent;
            } else {
                let mut has_tab: c_int = 0;
                let len = linelen(if (*eap).cmdidx == cmdidx_T::CMD_right as c_int {
                    &raw mut has_tab
                } else {
                    ptr::null_mut()
                }) - get_indent();

                if len <= 0 {
                    (*curwin).w_cursor.lnum += 1;
                    continue;
                }

                if (*eap).cmdidx == cmdidx_T::CMD_center as c_int {
                    new_indent = (width - len) / 2;
                } else {
                    new_indent = width - len;

                    if has_tab != 0 {
                        while new_indent > 0 {
                            set_indent(new_indent, 0);
                            if linelen(ptr::null_mut()) <= width {
                                loop {
                                    new_indent += 1;
                                    set_indent(new_indent, 0);
                                    if linelen(ptr::null_mut()) > width {
                                        break;
                                    }
                                }
                                new_indent -= 1;
                                break;
                            }
                            new_indent -= 1;
                        }
                    }
                }
            }
            new_indent = new_indent.max(0);
            set_indent(new_indent, 0);
            (*curwin).w_cursor.lnum += 1;
        }
        changed_lines(curbuf, (*eap).line1, 0, (*eap).line2 + 1, 0, true);
        (*curwin).w_cursor = save_curpos;
        beginline(BL_WHITE | BL_FIX);
    }
}

/// @return the length of the current line, excluding trailing white space.
unsafe fn linelen(has_tab: *mut c_int) -> c_int {
    unsafe {
        let line = get_cursor_line_ptr();
        if *line == NUL {
            return 0;
        }
        let first = skipwhite(line);

        let mut last = first.add(strlen(first));
        while last > first && ascii_iswhite(*last.sub(1) as c_int) {
            last = last.sub(1);
        }
        let save = *last;
        *last = NUL;
        let len = linetabsize_col(0, line);
        if !has_tab.is_null() {
            *has_tab = c_int::from(!vim_strchr(first, TAB).is_null());
        }
        *last = save;

        len
    }
}

unsafe extern "C" fn string_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        if sort_lc {
            return strcoll(s1.cast::<c_char>(), s2.cast::<c_char>());
        }
        if sort_ic {
            stricmp_c(s1.cast::<c_char>(), s2.cast::<c_char>())
        } else {
            strcmp(s1.cast::<c_char>(), s2.cast::<c_char>())
        }
    }
}

unsafe extern "C" fn sort_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let l1 = *s1.cast::<sorti_T>();
        let l2 = *s2.cast::<sorti_T>();
        let result: c_int;

        if sort_abort {
            return 0;
        }
        fast_breakcheck();
        if got_int {
            sort_abort = true;
        }

        if sort_nr != 0 {
            if l1.st_u.num.is_number != l2.st_u.num.is_number {
                result = if l1.st_u.num.is_number > l2.st_u.num.is_number {
                    1
                } else {
                    -1
                };
            } else if l1.st_u.num.value == l2.st_u.num.value {
                result = 0;
            } else if l1.st_u.num.value > l2.st_u.num.value {
                result = 1;
            } else {
                result = -1;
            }
        } else if sort_flt {
            if l1.st_u.value_flt == l2.st_u.value_flt {
                result = 0;
            } else if l1.st_u.value_flt > l2.st_u.value_flt {
                result = 1;
            } else {
                result = -1;
            }
        } else {
            let len1 = (l1.st_u.line.end_col_nr - l1.st_u.line.start_col_nr) as usize;
            ptr::copy_nonoverlapping(
                ml_get(l1.lnum).add(l1.st_u.line.start_col_nr as usize),
                sortbuf1,
                len1 + 1,
            );
            *sortbuf1.add(len1) = NUL;

            let len2 = (l2.st_u.line.end_col_nr - l2.st_u.line.start_col_nr) as usize;
            ptr::copy_nonoverlapping(
                ml_get(l2.lnum).add(l2.st_u.line.start_col_nr as usize),
                sortbuf2,
                len2 + 1,
            );
            *sortbuf2.add(len2) = NUL;

            result = string_compare(sortbuf1.cast::<c_void>(), sortbuf2.cast::<c_void>());
        }

        if result == 0 {
            return l1.lnum - l2.lnum;
        }
        result
    }
}

/// `":sort"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_sort(eap: *mut exarg_T) {
    unsafe {
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let mut maxlen: c_int = 0;
        let mut count = ((*eap).line2 - (*eap).line1) as usize + 1;
        let mut i: usize = 0;
        let mut unique = false;
        let mut sort_what: c_int = 0;

        if count <= 1 {
            return;
        }

        if u_save((*eap).line1 - 1, (*eap).line2 + 1) == FAIL {
            return;
        }
        sortbuf1 = ptr::null_mut();
        sortbuf2 = ptr::null_mut();
        regmatch.regprog = ptr::null_mut();
        let nrs = xmalloc(count * size_of::<sorti_T>()).cast::<sorti_T>();

        sort_abort = false;
        sort_ic = false;
        sort_lc = false;
        sort_rx = false;
        sort_nr = 0;
        sort_flt = false;
        let mut format_found: usize = 0;
        let mut change_occurred = false;

        'sortend: {
            let mut p = (*eap).arg;
            while *p != NUL {
                if ascii_iswhite(*p as c_int) {
                    // Skip
                } else if *p == b'i' as c_char {
                    sort_ic = true;
                } else if *p == b'l' as c_char {
                    sort_lc = true;
                } else if *p == b'r' as c_char {
                    sort_rx = true;
                } else if *p == b'n' as c_char {
                    sort_nr = 1;
                    format_found += 1;
                } else if *p == b'f' as c_char {
                    sort_flt = true;
                    format_found += 1;
                } else if *p == b'b' as c_char {
                    sort_what = STR2NR_BIN + STR2NR_FORCE;
                    format_found += 1;
                } else if *p == b'o' as c_char {
                    sort_what = STR2NR_OCT + STR2NR_FORCE;
                    format_found += 1;
                } else if *p == b'x' as c_char {
                    sort_what = STR2NR_HEX + STR2NR_FORCE;
                    format_found += 1;
                } else if *p == b'u' as c_char {
                    unique = true;
                } else if *p == b'"' as c_char {
                    break;
                } else if !check_nextcmd(p).is_null() {
                    (*eap).nextcmd = check_nextcmd(p);
                    break;
                } else if !ascii_isalpha(*p as c_int) && regmatch.regprog.is_null() {
                    let s = skip_regexp_err(p.add(1), (*p as u8) as c_int, 1);
                    if s.is_null() {
                        break 'sortend;
                    }
                    *s = NUL;
                    if s == p.add(1) {
                        if last_search_pat().is_null() {
                            emsg(gettext(e_noprevre.as_ptr()));
                            break 'sortend;
                        }
                        regmatch.regprog = vim_regcomp(last_search_pat(), RE_MAGIC);
                    } else {
                        regmatch.regprog = vim_regcomp(p.add(1), RE_MAGIC);
                    }
                    if regmatch.regprog.is_null() {
                        break 'sortend;
                    }
                    p = s;
                    regmatch.rm_ic = p_ic != 0;
                } else {
                    semsg!(gettext(e_invarg2.as_ptr()), p);
                    break 'sortend;
                }
                p = p.add(1);
            }

            if format_found > 1 {
                emsg(gettext(e_invarg.as_ptr()));
                break 'sortend;
            }

            sort_nr |= sort_what;

            for lnum in (*eap).line1..=(*eap).line2 {
                let mut s = ml_get(lnum);
                let len = ml_get_len(lnum);
                maxlen = maxlen.max(len);

                let mut start_col: colnr_T = 0;
                let mut end_col: colnr_T = len;
                if !regmatch.regprog.is_null() && vim_regexec(&raw mut regmatch, s, 0) {
                    if sort_rx {
                        start_col = regmatch.startp[0].offset_from(s) as colnr_T;
                        end_col = regmatch.endp[0].offset_from(s) as colnr_T;
                    } else {
                        start_col = regmatch.endp[0].offset_from(s) as colnr_T;
                    }
                } else if !regmatch.regprog.is_null() {
                    end_col = 0;
                }

                let idx = (lnum - (*eap).line1) as usize;
                if sort_nr != 0 || sort_flt {
                    let s2 = s.add(end_col as usize);
                    let c = *s2;
                    *s2 = NUL;
                    let p = s.add(start_col as usize);
                    if sort_nr != 0 {
                        if (sort_what & STR2NR_HEX) != 0 {
                            s = skiptohex(p);
                        } else if (sort_what & STR2NR_BIN) != 0 {
                            s = skiptobin(p).cast_mut();
                        } else {
                            s = skiptodigit(p);
                        }
                        if s > p && *s.sub(1) == b'-' as c_char {
                            s = s.sub(1);
                        }
                        if *s == NUL {
                            (*nrs.add(idx)).st_u.num = SortNum {
                                is_number: false,
                                value: 0,
                            };
                        } else {
                            let mut val: varnumber_T = 0;
                            vim_str2nr(
                                s,
                                ptr::null_mut(),
                                ptr::null_mut(),
                                sort_what,
                                &raw mut val,
                                ptr::null_mut(),
                                0,
                                false,
                                ptr::null_mut(),
                            );
                            (*nrs.add(idx)).st_u.num = SortNum {
                                is_number: true,
                                value: val,
                            };
                        }
                    } else {
                        s = skipwhite(p);
                        if *s == b'+' as c_char {
                            s = skipwhite(s.add(1));
                        }

                        if *s == NUL {
                            (*nrs.add(idx)).st_u.value_flt = -c_double::MAX;
                        } else {
                            (*nrs.add(idx)).st_u.value_flt = strtod(s, ptr::null_mut());
                        }
                    }
                    *s2 = c;
                } else {
                    (*nrs.add(idx)).st_u.line = SortLine {
                        start_col_nr: start_col as varnumber_T,
                        end_col_nr: end_col as varnumber_T,
                    };
                }

                (*nrs.add(idx)).lnum = lnum;

                if !regmatch.regprog.is_null() {
                    fast_breakcheck();
                }
                if got_int {
                    break 'sortend;
                }
            }

            sortbuf1 = xmalloc(maxlen as usize + 1).cast::<c_char>();
            sortbuf2 = xmalloc(maxlen as usize + 1).cast::<c_char>();

            qsort(
                nrs.cast::<c_void>(),
                count,
                size_of::<sorti_T>(),
                sort_compare,
            );

            if sort_abort {
                break 'sortend;
            }

            let mut old_count: bcount_t = 0;
            let mut new_count: bcount_t = 0;

            let mut lnum = (*eap).line2;
            while i < count {
                let get_lnum = (*nrs.add(if (*eap).forceit != 0 {
                    count - i - 1
                } else {
                    i
                }))
                .lnum;

                if get_lnum + (count as linenr_T - 1) != lnum {
                    change_occurred = true;
                }

                let s = ml_get(get_lnum);
                let bytelen: colnr_T = ml_get_len(get_lnum) + 1;
                old_count += bytelen as bcount_t;
                if !unique
                    || i == 0
                    || string_compare(s.cast::<c_void>(), sortbuf1.cast::<c_void>()) != 0
                {
                    strcpy(sortbuf1, s);
                    if ml_append(lnum, sortbuf1, 0, false) == FAIL {
                        break;
                    }
                    lnum += 1;
                    new_count += bytelen as bcount_t;
                }
                fast_breakcheck();
                if got_int {
                    break 'sortend;
                }
                i += 1;
            }

            if i == count {
                for _ in 0..count {
                    ml_delete((*eap).line1);
                }
            } else {
                count = 0;
            }

            let deleted = count as linenr_T - (lnum - (*eap).line2);
            if deleted > 0 {
                mark_adjust(
                    (*eap).line2 - deleted,
                    (*eap).line2,
                    MAXLNUM,
                    -deleted,
                    kExtmarkNOOP,
                );
                msgmore(-deleted);
            } else if deleted < 0 {
                mark_adjust((*eap).line2, MAXLNUM, -deleted, 0, kExtmarkNOOP);
            }

            if change_occurred || deleted != 0 {
                extmark_splice(
                    curbuf,
                    (*eap).line1 - 1,
                    0,
                    count as c_int,
                    0,
                    old_count,
                    lnum - (*eap).line2,
                    0,
                    new_count,
                    kExtmarkUndo,
                );
                changed_lines(curbuf, (*eap).line1, 0, (*eap).line2 + 1, -deleted, true);
            }

            (*curwin).w_cursor.lnum = (*eap).line1;
            beginline(BL_WHITE | BL_FIX);
        }

        // sortend:
        xfree(nrs.cast::<c_void>());
        xfree(sortbuf1.cast::<c_void>());
        xfree(sortbuf2.cast::<c_void>());
        vim_regfree(regmatch.regprog.cast::<regprog_T>());
        if got_int {
            emsg(gettext(e_interr.as_ptr()));
        }
    }
}

/// `":uniq"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_uniq(eap: *mut exarg_T) {
    unsafe {
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let mut maxlen: c_int = 0;
        let mut count = (*eap).line2 - (*eap).line1 + 1;
        let mut keep_only_unique = false;
        let keep_only_not_unique = (*eap).forceit != 0;
        let mut deleted: linenr_T = 0;

        if count <= 1 {
            return;
        }

        if u_save((*eap).line1 - 1, (*eap).line2 + 1) == FAIL {
            return;
        }
        sortbuf1 = ptr::null_mut();
        regmatch.regprog = ptr::null_mut();

        sort_abort = false;
        sort_ic = false;
        sort_lc = false;
        sort_rx = false;
        sort_nr = 0;
        sort_flt = false;
        let mut change_occurred = false;

        'uniqend: {
            let mut p = (*eap).arg;
            while *p != NUL {
                if ascii_iswhite(*p as c_int) {
                    // Skip
                } else if *p == b'i' as c_char {
                    sort_ic = true;
                } else if *p == b'l' as c_char {
                    sort_lc = true;
                } else if *p == b'r' as c_char {
                    sort_rx = true;
                } else if *p == b'u' as c_char {
                    if !keep_only_not_unique {
                        keep_only_unique = true;
                    }
                } else if *p == b'"' as c_char {
                    break;
                } else if (*eap).nextcmd.is_null() && !check_nextcmd(p).is_null() {
                    (*eap).nextcmd = check_nextcmd(p);
                    break;
                } else if !ascii_isalpha(*p as c_int) && regmatch.regprog.is_null() {
                    let s = skip_regexp_err(p.add(1), (*p as u8) as c_int, 1);
                    if s.is_null() {
                        break 'uniqend;
                    }
                    *s = NUL;
                    if s == p.add(1) {
                        if last_search_pat().is_null() {
                            emsg(gettext(e_noprevre.as_ptr()));
                            break 'uniqend;
                        }
                        regmatch.regprog = vim_regcomp(last_search_pat(), RE_MAGIC);
                    } else {
                        regmatch.regprog = vim_regcomp(p.add(1), RE_MAGIC);
                    }
                    if regmatch.regprog.is_null() {
                        break 'uniqend;
                    }
                    p = s;
                    regmatch.rm_ic = p_ic != 0;
                } else {
                    semsg!(gettext(e_invarg2.as_ptr()), p);
                    break 'uniqend;
                }
                p = p.add(1);
            }

            for lnum in (*eap).line1..=(*eap).line2 {
                let len = ml_get_len(lnum);
                if maxlen < len {
                    maxlen = len;
                }

                if got_int {
                    break 'uniqend;
                }
            }

            sortbuf1 = xmalloc(maxlen as usize + 1).cast::<c_char>();

            let mut match_continue = false;
            let mut next_is_unmatch = false;
            let mut done_lnum = (*eap).line1 - 1;
            let mut i: linenr_T = 0;
            while i < count {
                let get_lnum = (*eap).line1 + i;

                let s = ml_get(get_lnum);
                let len = ml_get_len(get_lnum);

                let mut start_col: colnr_T = 0;
                let mut end_col: colnr_T = len;
                if !regmatch.regprog.is_null() && vim_regexec(&raw mut regmatch, s, 0) {
                    if sort_rx {
                        start_col = regmatch.startp[0].offset_from(s) as colnr_T;
                        end_col = regmatch.endp[0].offset_from(s) as colnr_T;
                    } else {
                        start_col = regmatch.endp[0].offset_from(s) as colnr_T;
                    }
                } else if !regmatch.regprog.is_null() {
                    end_col = 0;
                }
                let mut save_c: c_char = NUL;
                if end_col > 0 {
                    save_c = *s.add(end_col as usize);
                    *s.add(end_col as usize) = NUL;
                }

                let mut is_match = if i > 0 {
                    string_compare(
                        s.add(start_col as usize).cast::<c_void>(),
                        sortbuf1.cast::<c_void>(),
                    ) == 0
                } else {
                    false
                };
                let mut delete_lnum: linenr_T = 0;
                if next_is_unmatch {
                    is_match = false;
                    next_is_unmatch = false;
                }

                if !keep_only_unique && !keep_only_not_unique {
                    if is_match {
                        delete_lnum = get_lnum;
                    } else {
                        strcpy(sortbuf1, s.add(start_col as usize));
                    }
                } else if keep_only_not_unique {
                    if is_match {
                        done_lnum = get_lnum - 1;
                        delete_lnum = get_lnum;
                        match_continue = true;
                    } else {
                        if i > 0 && !match_continue && get_lnum - 1 > done_lnum {
                            delete_lnum = get_lnum - 1;
                            next_is_unmatch = true;
                        } else if i >= count - 1 {
                            delete_lnum = get_lnum;
                        }
                        match_continue = false;
                        strcpy(sortbuf1, s.add(start_col as usize));
                    }
                } else if is_match {
                    if !match_continue {
                        delete_lnum = get_lnum - 1;
                    } else {
                        delete_lnum = get_lnum;
                    }
                    match_continue = true;
                } else {
                    if i == 0 && match_continue {
                        delete_lnum = get_lnum;
                    }
                    match_continue = false;
                    strcpy(sortbuf1, s.add(start_col as usize));
                }

                if end_col > 0 {
                    *s.add(end_col as usize) = save_c;
                }

                if delete_lnum > 0 {
                    ml_delete(delete_lnum);
                    i -= get_lnum - delete_lnum + 1;
                    count -= 1;
                    deleted += 1;
                    change_occurred = true;
                }

                fast_breakcheck();
                if got_int {
                    break 'uniqend;
                }
                i += 1;
            }

            mark_adjust(
                (*eap).line2 - deleted,
                (*eap).line2,
                MAXLNUM,
                -deleted,
                if change_occurred {
                    kExtmarkUndo
                } else {
                    kExtmarkNOOP
                },
            );
            msgmore(-deleted);

            if change_occurred {
                changed_lines(curbuf, (*eap).line1, 0, (*eap).line2 + 1, -deleted, true);
            }

            (*curwin).w_cursor.lnum = (*eap).line1;
            beginline(BL_WHITE | BL_FIX);
        }

        // uniqend:
        xfree(sortbuf1.cast::<c_void>());
        vim_regfree(regmatch.regprog.cast::<regprog_T>());
        if got_int {
            emsg(gettext(e_interr.as_ptr()));
        }
    }
}

/// `:move` command - move lines `line1`-`line2` to line `dest`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_move(line1: linenr_T, line2: linenr_T, mut dest: linenr_T) -> c_int {
    unsafe {
        if dest >= line1 && dest < line2 {
            emsg(gettext(
                c"E134: Cannot move a range of lines into itself".as_ptr(),
            ));
            return FAIL;
        }

        if dest == line1 - 1 || dest == line2 {
            (*curwin).w_cursor.lnum = if dest >= line1 {
                dest
            } else {
                dest + (line2 - line1) + 1
            };
            return OK;
        }

        let start_byte = ml_find_line_or_offset(curbuf, line1, ptr::null_mut(), true) as bcount_t;
        let end_byte = ml_find_line_or_offset(curbuf, line2 + 1, ptr::null_mut(), true) as bcount_t;
        let extent_byte = end_byte - start_byte;
        let dest_byte = ml_find_line_or_offset(curbuf, dest + 1, ptr::null_mut(), true) as bcount_t;

        let num_lines = line2 - line1 + 1;

        if u_save(dest, dest + 1) == FAIL {
            return FAIL;
        }

        let mut extra: linenr_T = 0;
        for l in line1..=line2 {
            let str = xstrnsave(ml_get(l + extra), ml_get_len(l + extra) as usize);
            ml_append(dest + l - line1, str, 0, false);
            xfree(str.cast::<c_void>());
            if dest < line1 {
                extra += 1;
            }
        }

        let mut last_line = (*curbuf).b_ml.ml_line_count;
        mark_adjust_nofold(line1, line2, last_line - line2, 0, kExtmarkNOOP);

        disable_fold_update += 1;
        changed_lines(
            curbuf,
            last_line - num_lines + 1,
            0,
            last_line + 1,
            num_lines,
            false,
        );
        disable_fold_update -= 1;

        let mut line_off: c_int = 0;
        let mut byte_off: bcount_t = 0;
        if dest >= line2 {
            mark_adjust_nofold(line2 + 1, dest, -num_lines, 0, kExtmarkNOOP);
            let mut tab = first_tabpage;
            while !tab.is_null() {
                let mut win = if tab == curtab {
                    firstwin
                } else {
                    (*tab).tp_firstwin
                };
                while !win.is_null() {
                    if (*win).w_buffer == curbuf {
                        foldMoveRange(win, &raw mut (*win).w_folds, line1, line2, dest);
                    }
                    win = (*win).w_next;
                }
                tab = (*tab).tp_next;
            }
            if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                (*curbuf).b_op_start.lnum = dest - num_lines + 1;
                (*curbuf).b_op_end.lnum = dest;
            }
            line_off = -num_lines;
            byte_off = -extent_byte;
        } else {
            mark_adjust_nofold(dest + 1, line1 - 1, num_lines, 0, kExtmarkNOOP);
            let mut tab = first_tabpage;
            while !tab.is_null() {
                let mut win = if tab == curtab {
                    firstwin
                } else {
                    (*tab).tp_firstwin
                };
                while !win.is_null() {
                    if (*win).w_buffer == curbuf {
                        foldMoveRange(win, &raw mut (*win).w_folds, dest + 1, line1 - 1, line2);
                    }
                    win = (*win).w_next;
                }
                tab = (*tab).tp_next;
            }
            if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                (*curbuf).b_op_start.lnum = dest + 1;
                (*curbuf).b_op_end.lnum = dest + num_lines;
            }
        }
        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_end.col = 0;
            (*curbuf).b_op_start.col = 0;
        }
        mark_adjust_nofold(
            last_line - num_lines + 1,
            last_line,
            -(last_line - dest - extra),
            0,
            kExtmarkNOOP,
        );

        disable_fold_update += 1;
        changed_lines(
            curbuf,
            last_line - num_lines + 1,
            0,
            last_line + 1,
            -extra,
            false,
        );
        disable_fold_update -= 1;

        buf_updates_send_changes(curbuf, dest + 1, num_lines as i64, 0);

        if u_save(line1 + extra - 1, line2 + extra + 1) == FAIL {
            return FAIL;
        }

        for _ in line1..=line2 {
            ml_delete_flags(line1 + extra, ML_DEL_MESSAGE);
        }
        if global_busy == 0 && (num_lines as OptInt) > p_report {
            smsg!(
                0,
                ngettext(
                    c"%lld line moved".as_ptr(),
                    c"%lld lines moved".as_ptr(),
                    num_lines as c_ulong,
                ),
                num_lines as i64,
            );
        }

        extmark_move_region(
            curbuf,
            line1 - 1,
            0,
            start_byte,
            line2 - line1 + 1,
            0,
            extent_byte,
            dest + line_off,
            0,
            dest_byte + byte_off,
            kExtmarkUndo,
        );

        if dest >= line1 {
            (*curwin).w_cursor.lnum = dest;
        } else {
            (*curwin).w_cursor.lnum = dest + (line2 - line1) + 1;
        }

        if line1 < dest {
            dest += num_lines + 1;
            last_line = (*curbuf).b_ml.ml_line_count;
            dest = dest.min(last_line + 1);
            changed_lines(curbuf, line1, 0, dest, 0, false);
        } else {
            changed_lines(curbuf, dest + 1, 0, line1 + num_lines, 0, false);
        }

        buf_updates_send_changes(curbuf, line1 + extra, 0, num_lines as i64);

        OK
    }
}

/// `":copy"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_copy(mut line1: linenr_T, mut line2: linenr_T, n: linenr_T) {
    unsafe {
        let count = line2 - line1 + 1;
        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start.lnum = n + 1;
            (*curbuf).b_op_end.lnum = n + count;
            (*curbuf).b_op_end.col = 0;
            (*curbuf).b_op_start.col = 0;
        }

        if u_save(n, n + 1) == FAIL {
            return;
        }

        (*curwin).w_cursor.lnum = n;
        while line1 <= line2 {
            let p = xstrnsave(ml_get(line1), ml_get_len(line1) as usize);
            ml_append((*curwin).w_cursor.lnum, p, 0, false);
            xfree(p.cast::<c_void>());

            if line1 == n {
                line1 = (*curwin).w_cursor.lnum;
            }
            line1 += 1;
            if (*curwin).w_cursor.lnum < line1 {
                line1 += 1;
            }
            if (*curwin).w_cursor.lnum < line2 {
                line2 += 1;
            }
            (*curwin).w_cursor.lnum += 1;
        }

        appended_lines_mark(n, count);
        if Visual.active {
            check_pos(curbuf, &raw mut Visual.start);
        }

        msgmore(count);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_prev_shellcmd() {
    unsafe {
        xfree(prevcmd.cast::<c_void>());
    }
}

/// Check that `prevcmd` is not NULL.
unsafe fn prevcmd_is_set() -> bool {
    unsafe {
        if prevcmd.is_null() {
            emsg(gettext(e_noprev.as_ptr()));
            return false;
        }
        true
    }
}

/// Handle the `":!cmd"` command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_bang(
    addr_count: c_int,
    eap: *mut exarg_T,
    forceit: bool,
    do_in: bool,
    do_out: bool,
) {
    unsafe {
        let arg = (*eap).arg;
        let line1 = (*eap).line1;
        let line2 = (*eap).line2;
        let mut newcmd: *mut c_char = ptr::null_mut();
        let mut free_newcmd = false;
        let scroll_save = msg_scroll;

        if check_secure() {
            return;
        }

        if addr_count == 0 {
            msg_scroll = 0;
            autowrite_all();
            msg_scroll = scroll_save;
        }

        let mut ins_prevcmd = forceit;

        let mut trailarg = skipwhite(arg);
        loop {
            let mut len = strlen(trailarg) + 1;
            if !newcmd.is_null() {
                len += strlen(newcmd);
            }
            if ins_prevcmd {
                if !prevcmd_is_set() {
                    xfree(newcmd.cast::<c_void>());
                    return;
                }
                len += strlen(prevcmd);
            }
            let t = xmalloc(len).cast::<c_char>();
            *t = NUL;
            if !newcmd.is_null() {
                strcat(t, newcmd);
            }
            if ins_prevcmd {
                strcat(t, prevcmd);
            }
            let mut p = t.add(strlen(t));
            strcat(t, trailarg);
            xfree(newcmd.cast::<c_void>());
            newcmd = t;

            trailarg = ptr::null_mut();
            while *p != NUL {
                if *p == b'!' as c_char {
                    if p > newcmd && *p.sub(1) == b'\\' as c_char {
                        ptr::copy(p, p.sub(1), strlen(p) + 1);
                    } else {
                        trailarg = p;
                        *trailarg = NUL;
                        trailarg = trailarg.add(1);
                        ins_prevcmd = true;
                        break;
                    }
                }
                p = p.add(1);
            }
            if trailarg.is_null() {
                break;
            }
        }

        if strlen(newcmd) > 0 {
            xfree(prevcmd.cast::<c_void>());
            prevcmd = newcmd;
        } else {
            free_newcmd = true;
        }

        'theend: {
            if bangredo {
                if !prevcmd_is_set() {
                    break 'theend;
                }

                let cmd = vim_strsave_escaped(prevcmd, c"%#".as_ptr());

                redo_append_lit(cmd, -1);
                xfree(cmd.cast::<c_void>());
                redo_append_str(c"\n".as_ptr(), 1);
                bangredo = false;
            }
            if *p_shq != NUL {
                if free_newcmd {
                    xfree(newcmd.cast::<c_void>());
                }
                newcmd = xmalloc(strlen(prevcmd) + 2 * strlen(p_shq) + 1).cast::<c_char>();
                strcpy(newcmd, p_shq);
                strcat(newcmd, prevcmd);
                strcat(newcmd, p_shq);
                free_newcmd = true;
            }
            if addr_count == 0 {
                msg_start();
                msg_ext_no_fast();
                msg_ext_set_kind(c"shell_cmd".as_ptr());
                msg_putchar(b':' as c_int);
                msg_putchar(b'!' as c_int);
                msg_outtrans(newcmd, 0, false);
                msg_clr_eos();
                ui_cursor_goto(msg_row, msg_col);

                do_shell(newcmd, 0);
            } else {
                do_filter(line1, line2, eap, newcmd, do_in, do_out);
                apply_autocmds(
                    EVENT_SHELLFILTERPOST,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    curbuf,
                );
            }
        }

        if free_newcmd {
            xfree(newcmd.cast::<c_void>());
        }
    }
}

/// `do_filter`: filter lines through a command given by the user.
unsafe fn do_filter(
    line1: linenr_T,
    line2: linenr_T,
    eap: *mut exarg_T,
    cmd: *mut c_char,
    do_in: bool,
    do_out: bool,
) {
    unsafe {
        let mut itmp: *mut c_char = ptr::null_mut();
        let mut otmp: *mut c_char = ptr::null_mut();
        let old_curbuf = curbuf;
        let mut shell_flags: c_int = 0;
        let orig_start = (*curbuf).b_op_start;
        let orig_end = (*curbuf).b_op_end;
        let stmp = p_stmp;

        if *cmd == NUL {
            return;
        }

        let save_cmod_flags = cmdmod.cmod_flags;
        cmdmod.cmod_flags &= !CMOD_LOCKMARKS;

        let cursor_save = (*curwin).w_cursor;
        let mut linecount = line2 - line1 + 1;
        (*curwin).w_cursor.lnum = line1;
        (*curwin).w_cursor.col = 0;
        changed_line_abv_curs();
        invalidate_botline_win(curwin);

        if do_out {
            shell_flags |= kShellOptDoOut;
        }

        'filterend: {
            if !do_in && do_out && stmp == 0 {
                shell_flags |= kShellOptRead;
                (*curwin).w_cursor.lnum = line2;
            } else if do_in && !do_out && stmp == 0 {
                shell_flags |= kShellOptWrite;
                (*curbuf).b_op_start.lnum = line1;
                (*curbuf).b_op_end.lnum = line2;
            } else if do_in && do_out && stmp == 0 {
                shell_flags |= kShellOptRead | kShellOptWrite;
                (*curbuf).b_op_start.lnum = line1;
                (*curbuf).b_op_end.lnum = line2;
                (*curwin).w_cursor.lnum = line2;
            } else {
                if do_in {
                    itmp = vim_tempname();
                    if itmp.is_null() {
                        emsg(gettext(e_notmp.as_ptr()));
                        break 'filterend;
                    }
                }
                if do_out {
                    otmp = vim_tempname();
                    if otmp.is_null() {
                        emsg(gettext(e_notmp.as_ptr()));
                        break 'filterend;
                    }
                }
            }

            no_wait_return += 1;
            if !itmp.is_null()
                && buf_write(
                    curbuf,
                    itmp,
                    ptr::null_mut(),
                    line1,
                    line2,
                    eap,
                    false,
                    false,
                    false,
                    true,
                ) == FAIL
            {
                if !ui_has(kUIMessages) {
                    msg_putchar(b'\n' as c_int);
                }
                no_wait_return -= 1;
                if !aborting() {
                    semsg!(gettext(c"E482: Can't create file %s".as_ptr()), itmp);
                }
                break 'filterend;
            }
            if curbuf != old_curbuf {
                break 'filterend;
            }

            if !do_out && !ui_has(kUIMessages) {
                msg_putchar(b'\n' as c_int);
            }

            let cmd_buf = make_filter_cmd(cmd, itmp, otmp, do_in);
            ui_cursor_goto(Rows - 1, 0);

            'error: {
                if do_out {
                    if u_save(line2, line2 + 1) == FAIL {
                        xfree(cmd_buf.cast::<c_void>());
                        break 'error;
                    }
                    redraw_curbuf_later(UPD_VALID);
                }
                let mut read_linecount = (*curbuf).b_ml.ml_line_count;

                call_shell(cmd_buf, kShellOptFilter | shell_flags, ptr::null_mut());
                xfree(cmd_buf.cast::<c_void>());

                did_check_timestamps = false;
                need_check_timestamps = true;

                os_breakcheck();
                got_int = false;

                if do_out {
                    if !otmp.is_null() {
                        if readfile(
                            otmp,
                            ptr::null_mut(),
                            line2,
                            0,
                            MAXLNUM,
                            eap,
                            READ_FILTER,
                            false,
                        ) != OK
                        {
                            if !aborting() {
                                msg_putchar(b'\n' as c_int);
                                semsg!(gettext(e_cant_read_file_str.as_ptr()), otmp);
                            }
                            break 'error;
                        }
                        if curbuf != old_curbuf {
                            break 'filterend;
                        }
                    }

                    read_linecount = (*curbuf).b_ml.ml_line_count - read_linecount;

                    if (shell_flags & kShellOptRead) != 0 {
                        (*curbuf).b_op_start.lnum = line2 + 1;
                        (*curbuf).b_op_end.lnum = (*curwin).w_cursor.lnum;
                        appended_lines_mark(line2, read_linecount);
                    }

                    if do_in {
                        if (cmdmod.cmod_flags & CMOD_KEEPMARKS) != 0
                            || vim_strchr(p_cpo, kCpoRemmark).is_null()
                        {
                            if read_linecount >= linecount {
                                mark_adjust(line1, line2, linecount, 0, kExtmarkNOOP);
                            } else {
                                mark_adjust(
                                    line1,
                                    line1 + read_linecount - 1,
                                    linecount,
                                    0,
                                    kExtmarkNOOP,
                                );
                                mark_adjust(
                                    line1 + read_linecount,
                                    line2,
                                    MAXLNUM,
                                    0,
                                    kExtmarkNOOP,
                                );
                            }
                        }

                        (*curwin).w_cursor.lnum = line1;
                        del_lines(linecount, true);
                        if read_linecount == 0 {
                            let l = line1.min((*curbuf).b_ml.ml_line_count);
                            (*curbuf).b_op_start.lnum = l;
                            (*curbuf).b_op_end.lnum = l;
                            (*curbuf).b_op_start.col = 0;
                            (*curbuf).b_op_end.col = 0;
                        } else {
                            (*curbuf).b_op_start.lnum -= linecount;
                            (*curbuf).b_op_end.lnum -= linecount;
                        }
                        write_lnum_adjust(-linecount);
                        foldUpdate(curwin, (*curbuf).b_op_start.lnum, (*curbuf).b_op_end.lnum);
                    } else {
                        linecount = (*curbuf).b_op_end.lnum - (*curbuf).b_op_start.lnum + 1;
                        (*curwin).w_cursor.lnum = (*curbuf).b_op_end.lnum;
                    }

                    beginline(BL_WHITE | BL_FIX);
                    no_wait_return -= 1;

                    if (linecount as OptInt) > p_report {
                        if do_in {
                            let msg_buf_ptr = (&raw mut msg_buf).cast::<c_char>();
                            vim_snprintf(
                                msg_buf_ptr,
                                MSG_BUF_LEN,
                                gettext(c"%lld lines filtered".as_ptr()),
                                linecount as i64,
                            );
                            if msg(msg_buf_ptr, 0) && msg_scroll == 0 {
                                set_keep_msg(msg_buf_ptr, 0);
                            }
                        } else {
                            msgmore(linecount);
                        }
                    }
                    break 'filterend;
                }
            }

            // error:
            (*curwin).w_cursor = cursor_save;
            no_wait_return -= 1;
            if !ui_has(kUIMessages) {
                wait_return(0);
            }
        }

        // filterend:
        cmdmod.cmod_flags = save_cmod_flags;
        if curbuf != old_curbuf {
            no_wait_return -= 1;
            emsg(gettext(
                c"E135: *Filter* Autocommands must not change current buffer".as_ptr(),
            ));
        } else if (cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
            (*curbuf).b_op_start = orig_start;
            (*curbuf).b_op_end = orig_end;
        }

        if !itmp.is_null() {
            os_remove(itmp);
        }
        if !otmp.is_null() {
            os_remove(otmp);
        }
        xfree(itmp.cast::<c_void>());
        xfree(otmp.cast::<c_void>());
    }
}

/// Call a shell to execute a command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_shell(cmd: *mut c_char, flags: c_int) {
    unsafe {
        if check_secure() {
            msg_end();
            return;
        }

        msg_putchar(b'\r' as c_int);
        msg_putchar(b'\n' as c_int);

        if p_warn != 0 && !autocmd_busy && msg_silent == 0 {
            let mut buf = firstbuf;
            while !buf.is_null() {
                if bufIsChanged(buf) {
                    msg_puts(gettext(c"[No write since last change]\n".as_ptr()));
                    break;
                }
                buf = (*buf).b_next;
            }
        }

        ui_cursor_goto(msg_row, msg_col);
        call_shell(cmd, flags, ptr::null_mut());
        if msg_silent == 0 {
            msg_didout = true;
        }
        did_check_timestamps = false;
        need_check_timestamps = true;

        msg_row = Rows - 1;
        msg_col = 0;

        apply_autocmds(
            EVENT_SHELLCMDPOST,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
    }
}

#[cfg(not(unix))]
unsafe fn find_pipe(cmd: *const c_char) -> *mut c_char {
    unsafe {
        let mut inquote = false;
        let mut p = cmd;
        while *p != NUL {
            if !inquote && *p == b'|' as c_char {
                return p.cast_mut();
            }
            if *p == b'"' as c_char {
                inquote = !inquote;
            } else if rem_backslash(p) {
                p = p.add(1);
            }
            p = p.add(1);
        }
        ptr::null_mut()
    }
}

/// Create a shell command from a command string, input redirection file and
/// output redirection file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn make_filter_cmd(
    cmd: *mut c_char,
    itmp: *mut c_char,
    otmp: *mut c_char,
    do_in: bool,
) -> *mut c_char {
    unsafe {
        #[cfg(unix)]
        let is_fish_shell = strncmp(
            invocation_path_tail(p_sh, ptr::null_mut()),
            c"fish".as_ptr(),
            4,
        ) == 0;
        #[cfg(not(unix))]
        let is_fish_shell = false;

        let is_pwsh = strncmp(
            invocation_path_tail(p_sh, ptr::null_mut()),
            c"pwsh".as_ptr(),
            4,
        ) == 0
            || strncmp(
                invocation_path_tail(p_sh, ptr::null_mut()),
                c"powershell".as_ptr(),
                10,
            ) == 0;

        let mut len = strlen(cmd) + 1;

        len += if is_fish_shell {
            b"begin; ; end".len()
        } else if !is_pwsh {
            b"()".len()
        } else {
            0
        };

        if !itmp.is_null() {
            len += if is_pwsh {
                strlen(itmp) + b"& { Get-Content  | &  }".len() + 6
            } else {
                strlen(itmp) + b" {  <  } ".len()
            };
        }

        if do_in && is_pwsh {
            len += b" $input | \0".len();
        }

        if !otmp.is_null() {
            len += strlen(otmp) + strlen(p_srr) + 2;
        }

        let buf = xmalloc(len).cast::<c_char>();

        if is_pwsh {
            if !itmp.is_null() {
                xstrlcpy(buf, c"& { Get-Content ".as_ptr(), len - 1);
                xstrlcat(buf, itmp, len - 1);
                xstrlcat(buf, c" | & ".as_ptr(), len - 1);
                xstrlcat(buf, cmd, len - 1);
                xstrlcat(buf, c" }".as_ptr(), len - 1);
            } else if do_in {
                xstrlcpy(buf, c" $input | ".as_ptr(), len - 1);
                xstrlcat(buf, cmd, len);
            } else {
                xstrlcpy(buf, cmd, len);
            }
        } else {
            #[cfg(unix)]
            {
                if !itmp.is_null() || !otmp.is_null() {
                    let fmt = if is_fish_shell {
                        c"begin; %s; end".as_ptr()
                    } else {
                        c"(%s)".as_ptr()
                    };
                    vim_snprintf(buf, len, fmt, cmd);
                } else {
                    xstrlcpy(buf, cmd, len);
                }

                if !itmp.is_null() {
                    xstrlcat(buf, c" < ".as_ptr(), len - 1);
                    xstrlcat(buf, itmp, len - 1);
                }
            }
            #[cfg(not(unix))]
            {
                xstrlcpy(buf, cmd, len);
                if !itmp.is_null() {
                    if *p_shq == NUL {
                        let p = find_pipe(buf);
                        if !p.is_null() {
                            *p = NUL;
                        }
                    }
                    xstrlcat(buf, c" < ".as_ptr(), len);
                    xstrlcat(buf, itmp, len);
                    if *p_shq == NUL {
                        let p = find_pipe(cmd);
                        if !p.is_null() {
                            xstrlcat(buf, c" ".as_ptr(), len - 1);
                            xstrlcat(buf, p, len - 1);
                        }
                    }
                }
            }
        }
        if !otmp.is_null() {
            append_redir(buf, len, p_srr, otmp);
        }
        buf
    }
}

/// Append output redirection for the given file to the end of the buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn append_redir(
    buf: *mut c_char,
    buflen: usize,
    opt: *const c_char,
    fname: *const c_char,
) {
    unsafe {
        let end = buf.add(strlen(buf));
        let mut p = opt;
        loop {
            p = strchr(p, b'%' as c_int);
            if p.is_null() {
                break;
            }
            if *p.add(1) == b's' as c_char {
                break;
            } else if *p.add(1) == b'%' as c_char {
                p = p.add(1);
            }
            p = p.add(1);
        }
        if !p.is_null() {
            *end = b' ' as c_char;
            vim_snprintf(
                end.add(1),
                (buflen as isize - end.add(1).offset_from(buf)) as usize,
                opt,
                fname,
            );
        } else {
            vim_snprintf(
                end,
                (buflen as isize - end.offset_from(buf)) as usize,
                c" %s %s".as_ptr(),
                opt,
                fname,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn print_line_no_prefix(lnum: linenr_T, use_number: bool, list: bool) {
    unsafe {
        let mut numbuf: [c_char; 30] = [0; 30];

        if (*curwin).w_onebuf_opt.wo_nu != 0 || use_number {
            vim_snprintf(
                numbuf.as_mut_ptr(),
                30,
                c"%*d ".as_ptr(),
                number_width(curwin),
                lnum,
            );
            msg_puts_hl(numbuf.as_ptr(), HLF_N + 1, false);
        }
        msg_prt_line(ml_get(lnum), list);
    }
}

/// Print a text line. Also in silent mode (`"ex -s"`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn print_line(lnum: linenr_T, use_number: bool, list: bool, first: bool) {
    unsafe {
        let save_silent = silent_mode;

        if message_filtered(ml_get(lnum)) {
            return;
        }

        silent_mode = false;
        info_message = true;
        if (global_busy == 0 || global_need_msg_kind) && first {
            msg_start();
            msg_ext_set_kind(c"list_cmd".as_ptr());
            global_need_msg_kind = false;
        } else if !save_silent {
            msg_putchar(b'\n' as c_int);
        }
        print_line_no_prefix(lnum, use_number, list);
        if save_silent {
            msg_putchar(b'\n' as c_int);
            silent_mode = save_silent;
        }
        info_message = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn rename_buffer(new_fname: *mut c_char) -> c_int {
    unsafe {
        let mut buf = curbuf;
        apply_autocmds(
            EVENT_BUFFILEPRE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        if buf != curbuf {
            return FAIL;
        }
        if aborting() {
            return FAIL;
        }
        let fname = (*curbuf).b_ffname;
        let sfname = (*curbuf).b_sfname;
        let xfname = (*curbuf).b_fname;
        (*curbuf).b_ffname = ptr::null_mut();
        (*curbuf).b_sfname = ptr::null_mut();
        if setfname(curbuf, new_fname, ptr::null_mut(), true) == FAIL {
            (*curbuf).b_ffname = fname;
            (*curbuf).b_sfname = sfname;
            return FAIL;
        }
        (*curbuf).b_flags |= BF_NOTEDITED;
        if !xfname.is_null() && *xfname != NUL {
            buf = buflist_new(fname, xfname, (*curwin).w_cursor.lnum, 0);
            if !buf.is_null() && (cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
                (*curwin).w_alt_fnum = (*buf).b_fnum;
            }
        }
        xfree(fname.cast::<c_void>());
        xfree(sfname.cast::<c_void>());
        apply_autocmds(
            EVENT_BUFFILEPOST,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        do_autochdir();
        OK
    }
}

/// `":file[!] [fname]"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_file(eap: *mut exarg_T) {
    unsafe {
        if (*eap).addr_count > 0
            && (*(*eap).arg != NUL || (*eap).line2 > 0 || (*eap).addr_count > 1)
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        if *(*eap).arg != NUL || (*eap).addr_count == 1 {
            if rename_buffer((*eap).arg) == FAIL {
                return;
            }
            redraw_tabline = true;
        }

        if *(*eap).arg == NUL || !shortmess(kShmFileinfo) {
            fileinfo(0, 0, (*eap).forceit != 0);
        }
    }
}

/// `":update"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_update(eap: *mut exarg_T) {
    unsafe {
        if curbufIsChanged()
            || (!bt_nofilename(curbuf)
                && !(*curbuf).b_ffname.is_null()
                && !os_path_exists((*curbuf).b_ffname))
        {
            do_write(eap);
        }
    }
}

/// `":write"` and `":saveas"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_write(eap: *mut exarg_T) {
    unsafe {
        if (*eap).cmdidx == cmdidx_T::CMD_saveas as c_int {
            (*eap).line1 = 1;
            (*eap).line2 = (*curbuf).b_ml.ml_line_count;
        }

        if (*eap).usefilter != 0 {
            do_bang(1, eap, false, true, false);
        } else {
            do_write(eap);
        }
    }
}

#[cfg(unix)]
unsafe fn check_writable(fname: *const c_char) -> c_int {
    unsafe {
        if os_nodetype(fname) == NODE_OTHER {
            semsg!(
                gettext(c"E503: \"%s\" is not a file or writable device".as_ptr()),
                fname,
            );
            return FAIL;
        }
        OK
    }
}

unsafe fn handle_mkdir_p_arg(eap: *mut exarg_T, fname: *mut c_char) -> c_int {
    unsafe {
        if (*eap).mkdir_p != 0 && os_file_mkdir(fname, 0o755) < 0 {
            return FAIL;
        }
        OK
    }
}

/// Write current buffer to file `eap->arg`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_write(eap: *mut exarg_T) -> c_int {
    unsafe {
        let mut fname: *mut c_char = ptr::null_mut();
        let mut retval = FAIL;
        let mut free_fname: *mut c_char = ptr::null_mut();
        let mut alt_buf: *mut buf_T = ptr::null_mut();

        if not_writing() {
            return FAIL;
        }

        let mut ffname = (*eap).arg;
        let other: bool;

        'theend: {
            if *ffname == NUL {
                if (*eap).cmdidx == cmdidx_T::CMD_saveas as c_int {
                    emsg(gettext(e_argreq.as_ptr()));
                    break 'theend;
                }
                other = false;
            } else {
                fname = ffname;
                free_fname = fix_fname(ffname);
                if !free_fname.is_null() {
                    ffname = free_fname;
                }
                other = otherfile(ffname);
            }

            if other {
                if !vim_strchr(p_cpo, kCpoAltwrite).is_null()
                    || (*eap).cmdidx == cmdidx_T::CMD_saveas as c_int
                {
                    alt_buf = setaltfname(ffname, fname, 1);
                } else {
                    alt_buf = buflist_findname(ffname);
                }
                if !alt_buf.is_null() && !(*alt_buf).b_ml.ml_mfp.is_null() {
                    emsg(gettext(e_bufloaded.as_ptr()));
                    break 'theend;
                }
            }

            if !other {
                if bt_dontwrite_msg(curbuf) || check_fname() == FAIL {
                    break 'theend;
                }
                #[cfg(unix)]
                if check_writable((*curbuf).b_ffname) == FAIL {
                    break 'theend;
                }
                if check_readonly(&raw mut (*eap).forceit, curbuf) {
                    break 'theend;
                }
            }

            if !other {
                ffname = (*curbuf).b_ffname;
                fname = (*curbuf).b_fname;
                if ((*eap).line1 != 1 || (*eap).line2 != (*curbuf).b_ml.ml_line_count)
                    && (*eap).forceit == 0
                    && (*eap).append == 0
                    && p_wa == 0
                {
                    if p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                        if vim_dialog_yesno(
                            VIM_QUESTION,
                            ptr::null_mut(),
                            gettext(c"Write partial file?".as_ptr()),
                            2,
                        ) != VIM_YES
                        {
                            break 'theend;
                        }
                        (*eap).forceit = 1;
                    } else {
                        emsg(gettext(c"E140: Use ! to write partial buffer".as_ptr()));
                        break 'theend;
                    }
                }
            }

            if check_overwrite(eap, curbuf, fname, ffname, other) == OK {
                if (*eap).cmdidx == cmdidx_T::CMD_saveas as c_int && !alt_buf.is_null() {
                    let was_curbuf = curbuf;

                    apply_autocmds(
                        EVENT_BUFFILEPRE,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        false,
                        curbuf,
                    );
                    apply_autocmds(
                        EVENT_BUFFILEPRE,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        false,
                        alt_buf,
                    );
                    if curbuf != was_curbuf || aborting() {
                        retval = FAIL;
                        break 'theend;
                    }

                    fname = (*alt_buf).b_fname;
                    (*alt_buf).b_fname = (*curbuf).b_fname;
                    (*curbuf).b_fname = fname;
                    fname = (*alt_buf).b_ffname;
                    (*alt_buf).b_ffname = (*curbuf).b_ffname;
                    (*curbuf).b_ffname = fname;
                    fname = (*alt_buf).b_sfname;
                    (*alt_buf).b_sfname = (*curbuf).b_sfname;
                    (*curbuf).b_sfname = fname;
                    buf_name_changed(curbuf);
                    apply_autocmds(
                        EVENT_BUFFILEPOST,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        false,
                        curbuf,
                    );
                    apply_autocmds(
                        EVENT_BUFFILEPOST,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        false,
                        alt_buf,
                    );
                    if (*alt_buf).b_p_bl == 0 {
                        (*alt_buf).b_p_bl = 1;
                        apply_autocmds(
                            EVENT_BUFADD,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            false,
                            alt_buf,
                        );
                    }
                    if curbuf != was_curbuf || aborting() {
                        retval = FAIL;
                        break 'theend;
                    }

                    if *(*curbuf).b_p_ft == NUL {
                        if augroup_exists(c"filetypedetect".as_ptr()) {
                            do_doautocmd(
                                c"filetypedetect BufRead".as_ptr().cast_mut(),
                                true,
                                ptr::null_mut(),
                            );
                        }
                        do_modelines(0);
                    }

                    fname = (*curbuf).b_sfname;
                }

                if handle_mkdir_p_arg(eap, fname) == FAIL {
                    retval = FAIL;
                    break 'theend;
                }

                let name_was_missing = (*curbuf).b_ffname.is_null();
                retval = buf_write(
                    curbuf,
                    ffname,
                    fname,
                    (*eap).line1,
                    (*eap).line2,
                    eap,
                    (*eap).append != 0,
                    (*eap).forceit != 0,
                    true,
                    false,
                );

                if (*eap).cmdidx == cmdidx_T::CMD_saveas as c_int && retval == OK {
                    (*curbuf).b_p_ro = 0;
                    redraw_tabline = true;
                }

                if (*eap).cmdidx == cmdidx_T::CMD_saveas as c_int || name_was_missing {
                    do_autochdir();
                }
            }
        }

        xfree(free_fname.cast::<c_void>());
        retval
    }
}

/// Check if it is allowed to overwrite a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_overwrite(
    eap: *mut exarg_T,
    buf: *mut buf_T,
    fname: *mut c_char,
    ffname: *mut c_char,
    other: bool,
) -> c_int {
    unsafe {
        if (other
            || (!bt_nofilename(buf)
                && (((*buf).b_flags & BF_NOTEDITED) != 0
                    || (((*buf).b_flags & BF_NEW) != 0
                        && vim_strchr(p_cpo, kCpoOvernew).is_null())
                    || ((*buf).b_flags & BF_READERR) != 0)))
            && p_wa == 0
            && os_path_exists(ffname)
        {
            if (*eap).forceit == 0 && (*eap).append == 0 {
                if os_isdir(ffname) {
                    semsg!(gettext(e_isadir2.as_ptr()), ffname);
                    return FAIL;
                }
                if p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                    let mut buff: [c_char; DIALOG_MSG_SIZE] = [0; DIALOG_MSG_SIZE];
                    snprintf(
                        buff.as_mut_ptr(),
                        DIALOG_MSG_SIZE,
                        gettext(c"Overwrite existing file \"%s\"?".as_ptr()),
                        fname,
                    );
                    if vim_dialog_yesno(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 2)
                        != VIM_YES
                    {
                        return FAIL;
                    }
                    (*eap).forceit = 1;
                } else {
                    emsg(gettext(e_exists.as_ptr()));
                    return FAIL;
                }
            }

            if other && emsg_silent == 0 {
                let dir: *mut c_char;
                if *p_dir == NUL {
                    dir = xmalloc(5).cast::<c_char>();
                    strcpy(dir, c".".as_ptr());
                } else {
                    dir = xmalloc(MAXPATHL).cast::<c_char>();
                    let mut p = p_dir;
                    copy_option_part(&raw mut p, dir, MAXPATHL, c",".as_ptr());
                }
                let swapname = makeswapname(fname, ffname, curbuf, dir);
                xfree(dir.cast::<c_void>());
                if os_path_exists(swapname) {
                    if p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                        let mut buff: [c_char; DIALOG_MSG_SIZE] = [0; DIALOG_MSG_SIZE];
                        vim_snprintf(
                            buff.as_mut_ptr(),
                            DIALOG_MSG_SIZE,
                            gettext(c"Swap file \"%s\" exists, overwrite anyway?".as_ptr()),
                            swapname,
                        );
                        if vim_dialog_yesno(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 2)
                            != VIM_YES
                        {
                            xfree(swapname.cast::<c_void>());
                            return FAIL;
                        }
                        (*eap).forceit = 1;
                    } else {
                        semsg!(
                            gettext(c"E768: Swap file exists: %s (:silent! overrides)".as_ptr()),
                            swapname,
                        );
                        xfree(swapname.cast::<c_void>());
                        return FAIL;
                    }
                }
                xfree(swapname.cast::<c_void>());
            }
        }
        OK
    }
}

/// Handle `":wnext"`, `":wNext"` and `":wprevious"` commands.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_wnext(eap: *mut exarg_T) {
    unsafe {
        let i = if *(*eap).cmd.add(1) == b'n' as c_char {
            (*curwin).w_arg_idx + (*eap).line2 as c_int
        } else {
            (*curwin).w_arg_idx - (*eap).line2 as c_int
        };
        (*eap).line1 = 1;
        (*eap).line2 = (*curbuf).b_ml.ml_line_count;
        if do_write(eap) != FAIL {
            do_argfile(eap, i);
        }
    }
}

/// `":wall"`, `":wqall"` and `":xall"`: Write all changed files (and exit).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_wqall(eap: *mut exarg_T) {
    unsafe {
        let mut error: c_int = 0;
        let save_forceit = (*eap).forceit;
        let save_exiting = exiting;

        if (*eap).cmdidx == cmdidx_T::CMD_xall as c_int
            || (*eap).cmdidx == cmdidx_T::CMD_wqall as c_int
        {
            if before_quit_all(eap) == FAIL {
                return;
            }
            exiting = true;
        }

        let mut buf = firstbuf;
        while !buf.is_null() {
            if exiting
                && (*eap).forceit == 0
                && !(*buf).terminal.is_null()
                && channel_job_running((*buf).b_p_channel as u64)
            {
                no_write_message_buf(buf);
                error += 1;
                buf = (*buf).b_next;
                continue;
            } else if !bufIsChanged(buf) || bt_dontwrite(buf) {
                buf = (*buf).b_next;
                continue;
            }

            if not_writing() {
                error += 1;
                break;
            }
            if (*buf).b_ffname.is_null() {
                semsg!(
                    gettext(c"E141: No file name for buffer %lld".as_ptr()),
                    (*buf).b_fnum as i64,
                );
                error += 1;
            } else if check_readonly(&raw mut (*eap).forceit, buf)
                || check_overwrite(eap, buf, (*buf).b_fname, (*buf).b_ffname, false) == FAIL
            {
                error += 1;
            } else {
                let mut bufref: bufref_T = std::mem::zeroed();
                set_bufref(&raw mut bufref, buf);
                if handle_mkdir_p_arg(eap, (*buf).b_fname) == FAIL
                    || buf_write_all(buf, (*eap).forceit != 0) == FAIL
                {
                    error += 1;
                }
                if !bufref_valid(&raw mut bufref) {
                    buf = firstbuf;
                    (*eap).forceit = save_forceit;
                    if !buf.is_null() {
                        buf = (*buf).b_next;
                    }
                    continue;
                }
            }
            (*eap).forceit = save_forceit;
            buf = (*buf).b_next;
        }
        if exiting {
            if error == 0 {
                getout(0);
            }
            not_exiting(save_exiting);
        }
    }
}

/// Check the `'write'` option.
unsafe fn not_writing() -> bool {
    unsafe {
        if p_write != 0 {
            return false;
        }
        emsg(gettext(
            c"E142: File not written: Writing is disabled by 'write' option".as_ptr(),
        ));
        true
    }
}

/// Check if a buffer is read-only.
unsafe fn check_readonly(forceit: *mut c_int, buf: *mut buf_T) -> bool {
    unsafe {
        if *forceit == 0
            && ((*buf).b_p_ro != 0
                || (os_path_exists((*buf).b_ffname) && os_file_is_writable((*buf).b_ffname) == 0))
        {
            if (p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                && !(*buf).b_fname.is_null()
            {
                let mut buff: [c_char; DIALOG_MSG_SIZE] = [0; DIALOG_MSG_SIZE];
                let fname = if !(*buf).b_fname.is_null() {
                    (*buf).b_fname
                } else {
                    gettext(c"Untitled".as_ptr())
                };
                if (*buf).b_p_ro != 0 {
                    snprintf(
                        buff.as_mut_ptr(),
                        DIALOG_MSG_SIZE,
                        gettext(
                            c"'readonly' option is set for \"%s\".\nDo you wish to write anyway?"
                                .as_ptr(),
                        ),
                        fname,
                    );
                } else {
                    snprintf(
                        buff.as_mut_ptr(),
                        DIALOG_MSG_SIZE,
                        gettext(
                            c"File permissions of \"%s\" are read-only.\nIt may still be possible to write it.\nDo you wish to try?"
                                .as_ptr(),
                        ),
                        fname,
                    );
                }

                if vim_dialog_yesno(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 2) == VIM_YES
                {
                    *forceit = 1;
                    return false;
                }
                return true;
            } else if (*buf).b_p_ro != 0 {
                emsg(gettext(e_readonly.as_ptr()));
            } else {
                semsg!(
                    gettext(c"E505: \"%s\" is read-only (add ! to override)".as_ptr()),
                    (*buf).b_fname,
                );
            }
            return true;
        }

        false
    }
}

/// Try to abandon the current file and edit a new or existing file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getfile(
    fnum: c_int,
    ffname_arg: *mut c_char,
    sfname_arg: *mut c_char,
    setpm: bool,
    lnum: linenr_T,
    forceit: bool,
) -> c_int {
    unsafe {
        if !check_can_set_curbuf_forceit(c_int::from(forceit)) {
            return GETFILE_ERROR;
        }

        let mut ffname = ffname_arg;
        let mut sfname = sfname_arg;
        let other: bool;
        let retval: c_int;
        let mut free_me: *mut c_char = ptr::null_mut();

        if text_locked() {
            return GETFILE_ERROR;
        }
        if curbuf_locked() {
            return GETFILE_ERROR;
        }

        if fnum == 0 {
            fname_expand(curbuf, &raw mut ffname, &raw mut sfname);
            other = otherfile(ffname);
            free_me = ffname;
        } else {
            other = fnum != (*curbuf).b_fnum;
        }

        if other {
            no_wait_return += 1;
        }
        'theend: {
            if other
                && !forceit
                && (*curbuf).b_nwindows == 1
                && !buf_hide(curbuf)
                && curbufIsChanged()
                && autowrite(curbuf, forceit) == FAIL
            {
                if p_confirm != 0 && p_write != 0 {
                    dialog_changed(curbuf, false);
                }
                if curbufIsChanged() {
                    no_wait_return -= 1;
                    no_write_message();
                    retval = GETFILE_NOT_WRITTEN;
                    break 'theend;
                }
            }
            if other {
                no_wait_return -= 1;
            }
            if setpm {
                setpcmark();
            }
            if !other {
                if lnum != 0 {
                    (*curwin).w_cursor.lnum = lnum;
                }
                check_cursor_lnum(curwin);
                beginline(BL_SOL | BL_FIX);
                retval = GETFILE_SAME_FILE;
            } else if do_ecmd(
                fnum,
                ffname,
                sfname,
                ptr::null_mut(),
                lnum,
                (if buf_hide(curbuf) { ECMD_HIDE } else { 0 })
                    + (if forceit { ECMD_FORCEIT } else { 0 }),
                curwin,
            ) == OK
            {
                retval = GETFILE_OPEN_OTHER;
            } else {
                retval = GETFILE_ERROR;
            }
        }

        xfree(free_me.cast::<c_void>());
        retval
    }
}

/// Set `v:swapcommand` for the `SwapExists` autocommands.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_swapcommand(command: *mut c_char, newlnum: linenr_T) -> bool {
    unsafe {
        if (command.is_null() && newlnum <= 0) || *get_vim_var_str(VV_SWAPCOMMAND) != NUL {
            return false;
        }
        let valsize = if !command.is_null() {
            strlen(command) + 3
        } else {
            30
        };
        let data = xmalloc(valsize).cast::<c_char>();
        let size = if !command.is_null() {
            vim_snprintf_safelen(data, valsize, c":%s\r".as_ptr(), command)
        } else {
            vim_snprintf_safelen(data, valsize, c"%lldG".as_ptr(), newlnum as i64)
        };
        set_vim_var_string(VV_SWAPCOMMAND, data, size as isize);
        xfree(data.cast::<c_void>());
        true
    }
}

/// Start editing a new file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_ecmd(
    fnum: c_int,
    mut ffname: *mut c_char,
    mut sfname: *mut c_char,
    eap: *mut exarg_T,
    mut newlnum: linenr_T,
    flags: c_int,
    mut oldwin: *mut win_T,
) -> c_int {
    unsafe {
        let other_file: bool;
        let oldbuf: bool;
        let mut auto_buf = false;
        let mut new_name: *mut c_char = ptr::null_mut();
        let mut did_set_swapcommand = false;
        let mut buf: *mut buf_T;
        let mut bufref: bufref_T = std::mem::zeroed();
        let mut old_curbuf: bufref_T = std::mem::zeroed();
        let mut free_fname: *mut c_char = ptr::null_mut();
        let mut retval = FAIL;
        let mut topline: linenr_T = 0;
        let mut newcol: c_int = -1;
        let mut solcol: c_int = -1;
        let mut command: *mut c_char = ptr::null_mut();
        let mut did_get_winopts = false;
        let mut readfile_flags: c_int = 0;
        let mut did_inc_redrawing_disabled = false;
        let so_ptr: *mut OptInt = if (*curwin).w_onebuf_opt.wo_so >= 0 {
            &raw mut (*curwin).w_onebuf_opt.wo_so
        } else {
            &raw mut p_so
        };

        if !eap.is_null() {
            command = (*eap).do_ecmd_cmd;
        }

        set_bufref(&raw mut old_curbuf, curbuf);

        'theend: {
            if fnum != 0 {
                if fnum == (*curbuf).b_fnum {
                    return OK;
                }
                other_file = true;
            } else {
                if sfname.is_null() {
                    sfname = ffname;
                }
                if CASE_INSENSITIVE_FILENAME && !sfname.is_null() {
                    path_fix_case(sfname);
                }

                if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) != 0
                    && (ffname.is_null() || *ffname == NUL)
                {
                    break 'theend;
                }

                if ffname.is_null() {
                    other_file = true;
                } else if *ffname == NUL && (*curbuf).b_ffname.is_null() {
                    other_file = false;
                } else {
                    if *ffname == NUL {
                        ffname = (*curbuf).b_ffname;
                        sfname = (*curbuf).b_fname;
                    }
                    free_fname = fix_fname(ffname);
                    if !free_fname.is_null() {
                        ffname = free_fname;
                    }
                    other_file = otherfile(ffname);
                }
            }

            if !other_file && !(*curbuf).terminal.is_null() {
                check_arg_idx(curwin);
                maketitle();
                retval = OK;
                break 'theend;
            }

            if ((!other_file && (flags & ECMD_OLDBUF) == 0)
                || ((*curbuf).b_nwindows == 1
                    && (flags & (ECMD_HIDE | ECMD_ADDBUF | ECMD_ALTBUF)) == 0))
                && check_changed(
                    curbuf,
                    (if p_awa != 0 { CCGD_AW } else { 0 })
                        | (if other_file { 0 } else { CCGD_MULTWIN })
                        | (if (flags & ECMD_FORCEIT) != 0 {
                            CCGD_FORCEIT
                        } else {
                            0
                        })
                        | (if eap.is_null() { 0 } else { CCGD_EXCMD }),
                )
            {
                if fnum == 0 && other_file && !ffname.is_null() {
                    setaltfname(ffname, sfname, if newlnum < 0 { 0 } else { newlnum });
                }
                break 'theend;
            }

            reset_VIsual();

            if !oldwin.is_null() && !win_valid(oldwin) {
                oldwin = ptr::null_mut();
            }

            did_set_swapcommand = set_swapcommand(command, newlnum);

            if other_file {
                let prev_alt_fnum = (*curwin).w_alt_fnum;

                if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) == 0 {
                    if (cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
                        (*curwin).w_alt_fnum = (*curbuf).b_fnum;
                    }
                    if !oldwin.is_null() {
                        buflist_altfpos(oldwin);
                    }
                }

                if fnum != 0 {
                    buf = buflist_findnr(fnum);
                } else {
                    if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) != 0 {
                        let mut tlnum: linenr_T = 0;

                        if !command.is_null() {
                            tlnum = atol(command) as linenr_T;
                            if tlnum <= 0 {
                                tlnum = 1;
                            }
                        }
                        let newbuf = buflist_new(ffname, sfname, tlnum, BLN_LISTED | BLN_NOCURWIN);
                        if !newbuf.is_null() && (flags & ECMD_ALTBUF) != 0 {
                            (*curwin).w_alt_fnum = (*newbuf).b_fnum;
                        }
                        break 'theend;
                    }
                    buf = buflist_new(
                        ffname,
                        sfname,
                        0,
                        BLN_CURBUF
                            | if (flags & ECMD_SET_HELP) != 0 {
                                0
                            } else {
                                BLN_LISTED
                            },
                    );
                    if !oldwin.is_null() {
                        oldwin = curwin;
                    }
                    set_bufref(&raw mut old_curbuf, curbuf);
                }
                if buf.is_null() {
                    break 'theend;
                }
                if (*buf).b_locked_split > 0 {
                    emsg(gettext(e_cannot_switch_to_a_closing_buffer.as_ptr()));
                    break 'theend;
                }
                if (*curwin).w_alt_fnum == (*buf).b_fnum && prev_alt_fnum != 0 {
                    (*curwin).w_alt_fnum = prev_alt_fnum;
                }
                if (*buf).b_ml.ml_mfp.is_null() {
                    oldbuf = false;
                } else {
                    oldbuf = true;
                    set_bufref(&raw mut bufref, buf);
                    buf_check_timestamp(buf);
                    if !bufref_valid(&raw mut bufref) || curbuf != old_curbuf.br_buf {
                        break 'theend;
                    }
                    if aborting() {
                        break 'theend;
                    }
                }

                if (oldbuf && newlnum == ECMD_LASTL) || newlnum == ECMD_LAST {
                    let pos = &raw mut (*buflist_findfmark(buf)).mark;
                    newlnum = (*pos).lnum;
                    solcol = (*pos).col;
                }

                if buf != curbuf {
                    if !(*buf).b_fname.is_null() {
                        new_name = xstrdup((*buf).b_fname);
                    }
                    let save_au_new_curbuf = au_new_curbuf;
                    set_bufref(&raw mut au_new_curbuf, buf);
                    apply_autocmds(
                        EVENT_BUFLEAVE,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        false,
                        curbuf,
                    );

                    if !bufref_valid(&raw mut au_new_curbuf) {
                        delbuf_msg(new_name);
                        au_new_curbuf = save_au_new_curbuf;
                        break 'theend;
                    }
                    if aborting() {
                        xfree(new_name.cast::<c_void>());
                        au_new_curbuf = save_au_new_curbuf;
                        break 'theend;
                    }
                    if buf == curbuf {
                        auto_buf = true;
                    } else {
                        let the_curwin = curwin;

                        (*the_curwin).w_locked += 1;
                        (*buf).b_locked += 1;

                        if curbuf == old_curbuf.br_buf {
                            buf_copy_options(buf, BCO_ENTER);
                        }

                        u_sync(false);
                        close_buffer(
                            curwin,
                            curbuf,
                            if (flags & ECMD_HIDE) != 0
                                || (!(*curbuf).terminal.is_null()
                                    && terminal_running((*curbuf).terminal.cast::<Terminal>()))
                            {
                                0
                            } else {
                                DOBUF_UNLOAD
                            },
                            false,
                            false,
                            !oldwin.is_null(),
                        );

                        if win_valid(the_curwin) {
                            (*the_curwin).w_locked -= 1;
                        }
                        (*buf).b_locked -= 1;

                        if aborting() && !(*curwin).w_buffer.is_null() {
                            xfree(new_name.cast::<c_void>());
                            au_new_curbuf = save_au_new_curbuf;
                            break 'theend;
                        }
                        if !bufref_valid(&raw mut au_new_curbuf) {
                            if (*curwin).w_buffer.is_null() {
                                buf = lastbuf;
                            } else {
                                delbuf_msg(new_name);
                                au_new_curbuf = save_au_new_curbuf;
                                break 'theend;
                            }
                        }
                        if buf == curbuf {
                            auto_buf = true;
                        } else {
                            if (*curwin).w_buffer.is_null()
                                || (*curwin).w_s.cast::<synblock_T>()
                                    == &raw mut (*(*curwin).w_buffer).b_s
                            {
                                (*curwin).w_s = (&raw mut (*buf).b_s).cast::<c_void>();
                            }

                            if !(*curwin).w_buffer.is_null() {
                                (*(*curwin).w_buffer).b_nwindows -= 1;
                            }

                            (*curwin).w_buffer = buf;
                            curbuf = buf;
                            (*curbuf).b_nwindows += 1;

                            if !oldbuf && !eap.is_null() {
                                set_file_options(true, eap);
                                set_forced_fenc(eap);
                            }
                        }

                        get_winopts(curbuf);
                        did_get_winopts = true;
                    }
                    xfree(new_name.cast::<c_void>());
                    au_new_curbuf = save_au_new_curbuf;
                }

                (*curwin).w_pcmark.lnum = 1;
                (*curwin).w_pcmark.col = 0;
            } else {
                if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) != 0 || check_fname() == FAIL {
                    break 'theend;
                }
                oldbuf = (flags & ECMD_OLDBUF) != 0;
            }

            RedrawingDisabled += 1;
            did_inc_redrawing_disabled = true;

            buf = curbuf;
            if (flags & ECMD_SET_HELP) != 0 || keep_help_flag {
                prepare_help_buffer();
            } else if !(*curbuf).b_help {
                set_buflisted(1);
            }

            if buf != curbuf {
                break 'theend;
            }
            if aborting() {
                break 'theend;
            }

            (*curbuf).b_did_filetype = false;

            if !other_file && !oldbuf {
                set_last_cursor(curwin);
                if newlnum == ECMD_LAST || newlnum == ECMD_LASTL {
                    newlnum = (*curwin).w_cursor.lnum;
                    solcol = (*curwin).w_cursor.col;
                }
                buf = curbuf;
                new_name = if !(*buf).b_fname.is_null() {
                    xstrdup((*buf).b_fname)
                } else {
                    ptr::null_mut()
                };
                set_bufref(&raw mut bufref, buf);

                if ((*curbuf).b_flags & BF_NEVERLOADED) == 0
                    && (p_ur < 0 || ((*curbuf).b_ml.ml_line_count as OptInt) <= p_ur)
                {
                    u_sync(false);
                    if u_savecommon(curbuf, 0, (*curbuf).b_ml.ml_line_count + 1, 0, true) == FAIL {
                        xfree(new_name.cast::<c_void>());
                        break 'theend;
                    }
                    u_unchanged(curbuf);
                    buf_freeall(curbuf, BFA_KEEP_UNDO);

                    readfile_flags = READ_KEEP_UNDO;
                } else {
                    buf_freeall(curbuf, 0);
                }
                if !bufref_valid(&raw mut bufref) {
                    delbuf_msg(new_name);
                    break 'theend;
                }
                xfree(new_name.cast::<c_void>());

                if buf != curbuf {
                    break 'theend;
                }
                if aborting() {
                    break 'theend;
                }
                buf_clear_file(curbuf);
                (*curbuf).b_op_start.lnum = 0;
                (*curbuf).b_op_end.lnum = 0;
            }

            retval = OK;

            if !other_file {
                (*curbuf).b_flags &= !BF_NOTEDITED;
            }

            check_arg_idx(curwin);

            if !auto_buf {
                curwin_init();

                let mut tp = first_tabpage;
                while !tp.is_null() {
                    let mut win = if tp == curtab {
                        firstwin
                    } else {
                        (*tp).tp_firstwin
                    };
                    while !win.is_null() {
                        if (*win).w_buffer == curbuf {
                            foldUpdateAll(win);
                        }
                        win = (*win).w_next;
                    }
                    tp = (*tp).tp_next;
                }

                do_autochdir();

                let orig_pos = (*curwin).w_cursor;
                topline = (*curwin).w_topline;
                if !oldbuf {
                    swap_exists_action = SEA_DIALOG;
                    (*curbuf).b_flags |= BF_CHECK_RO;

                    if (flags & ECMD_NOWINENTER) != 0 {
                        readfile_flags |= READ_NOWINENTER;
                    }
                    if should_abort(open_buffer(false, eap, readfile_flags)) {
                        retval = FAIL;
                    }

                    if swap_exists_action == SEA_QUIT {
                        retval = FAIL;
                    }
                    handle_swap_exists(&raw mut old_curbuf);
                } else {
                    do_modelines(OPT_WINONLY);

                    apply_autocmds_retval(
                        EVENT_BUFENTER,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        false,
                        curbuf,
                        &raw mut retval,
                    );
                    if (flags & ECMD_NOWINENTER) == 0 {
                        apply_autocmds_retval(
                            EVENT_BUFWINENTER,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            false,
                            curbuf,
                            &raw mut retval,
                        );
                    }
                }
                check_arg_idx(curwin);

                if !equalpos((*curwin).w_cursor, orig_pos) {
                    let text = get_cursor_line_ptr();

                    if (*curwin).w_cursor.lnum != orig_pos.lnum
                        || (*curwin).w_cursor.col != skipwhite(text).offset_from(text) as c_int
                    {
                        newlnum = (*curwin).w_cursor.lnum;
                        newcol = (*curwin).w_cursor.col;
                    }
                }
                if (*curwin).w_topline == topline {
                    topline = 0;
                }

                changed_line_abv_curs();

                maketitle();
                if retval != FAIL {
                    win_float_update_preview(curwin);
                }
            }

            if (*curwin).w_onebuf_opt.wo_diff != 0 {
                diff_buf_add(curbuf);
                diff_invalidate(curbuf);
            }

            if did_get_winopts
                && (*curwin).w_onebuf_opt.wo_spell != 0
                && *(*(*curwin).w_s.cast::<synblock_T>()).b_p_spl != NUL
            {
                parse_spelllang(curwin);
            }

            if command.is_null() {
                if newcol >= 0 {
                    (*curwin).w_cursor.lnum = newlnum;
                    (*curwin).w_cursor.col = newcol;
                    check_cursor(curwin);
                } else if newlnum > 0 {
                    (*curwin).w_cursor.lnum = newlnum;
                    check_cursor_lnum(curwin);
                    if solcol >= 0 && p_sol == 0 {
                        (*curwin).w_cursor.col = solcol;
                        check_cursor_col(curwin);
                        (*curwin).w_cursor.coladd = 0;
                        (*curwin).w_set_curswant = true;
                    } else {
                        beginline(BL_SOL | BL_FIX);
                    }
                } else {
                    beginline(BL_WHITE | BL_FIX);
                }
            }

            check_lnums(false);

            if oldbuf && !auto_buf {
                let msg_scroll_save = msg_scroll;

                if shortmess(kShmOverall) && msg_listdo_overwrite == 0 && !exiting && p_verbose == 0
                {
                    msg_scroll = 0;
                }
                if msg_scroll == 0 {
                    msg_check_for_delay(false);
                }
                msg_start();
                msg_scroll = msg_scroll_save;
                msg_scrolled_ign = true;

                if !shortmess(kShmFileinfo) {
                    fileinfo(0, 1, false);
                }

                msg_scrolled_ign = false;
            }

            (*curbuf).b_last_used = time(ptr::null_mut());

            if !command.is_null() {
                do_cmdline(command, None, ptr::null_mut(), DOCMD_VERBOSE);
            }

            if ((*curbuf).b_kmap_state & KEYMAP_INIT) != 0 {
                keymap_init();
            }

            RedrawingDisabled -= 1;
            did_inc_redrawing_disabled = false;
            if !skip_redraw {
                let n = *so_ptr;
                if topline == 0 && command.is_null() {
                    *so_ptr = 999;
                }
                update_topline(curwin);
                (*curwin).w_scbind_pos = plines_m_win_fill(curwin, 1, (*curwin).w_topline);
                *so_ptr = n;
                redraw_curbuf_later(UPD_NOT_VALID);
            }

            update_cwd(kCdCauseBuffer);

            do_autochdir();
        }

        // theend:
        if bufref_valid(&raw mut old_curbuf) && !(*old_curbuf.br_buf).terminal.is_null() {
            terminal_check_size((*old_curbuf.br_buf).terminal.cast::<Terminal>());
        }
        if (!bufref_valid(&raw mut old_curbuf) || curbuf != old_curbuf.br_buf)
            && !(*curbuf).terminal.is_null()
        {
            terminal_check_size((*curbuf).terminal.cast::<Terminal>());
        }

        if did_inc_redrawing_disabled {
            RedrawingDisabled -= 1;
        }
        if did_set_swapcommand {
            set_vim_var_string(VV_SWAPCOMMAND, ptr::null(), -1);
        }
        xfree(free_fname.cast::<c_void>());
        retval
    }
}

unsafe fn delbuf_msg(name: *mut c_char) {
    unsafe {
        semsg!(
            gettext(c"E143: Autocommands unexpectedly deleted new buffer %s".as_ptr()),
            if name.is_null() { c"".as_ptr() } else { name },
        );
        xfree(name.cast::<c_void>());
        au_new_curbuf.br_buf = ptr::null_mut();
        au_new_curbuf.br_buf_free_count = 0;
    }
}

/// Get the next line of text for `":append"`, `":insert"` and `":change"`.
unsafe fn get_append_line(eap: *mut exarg_T, indent: c_int) -> *mut c_char {
    unsafe {
        if *(*eap).arg == b'|' as c_char {
            let theline = xstrdup((*eap).arg.add(1));
            *(*eap).arg = NUL;
            theline
        } else if (*eap).ea_getline.is_none() {
            if (*eap).nextcmd.is_null() {
                return ptr::null_mut();
            }
            let mut p = vim_strchr((*eap).nextcmd, NL);
            if p.is_null() {
                p = (*eap).nextcmd.add(strlen((*eap).nextcmd));
            }
            let theline = xmemdupz(
                (*eap).nextcmd.cast::<c_void>(),
                p.offset_from((*eap).nextcmd) as usize,
            )
            .cast::<c_char>();
            if *p != NUL {
                p = p.add(1);
            } else {
                p = ptr::null_mut();
            }
            (*eap).nextcmd = p;
            theline
        } else {
            let getline_State = State;
            State = MODE_CMDLINE;
            let ea_getline = (*eap).ea_getline.unwrap();
            let cstack = (*eap).cstack.cast::<cstack_T>();
            let theline = ea_getline(
                if (*cstack).cs_looplevel > 0 {
                    -1
                } else {
                    NUL as c_int
                },
                (*eap).cookie,
                indent,
                true,
            );
            State = getline_State;
            theline
        }
    }
}

/// `":insert"` and `":append"`, also used by `":change"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_append(eap: *mut exarg_T) {
    unsafe {
        let mut did_undo = false;
        let mut lnum = (*eap).line2;
        let mut indent: c_int = 0;
        let mut empty = ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0;

        if (*eap).skip != 0 {
            loop {
                let theline = get_append_line(eap, 0);
                if theline.is_null() {
                    break;
                }
                let end = *theline == b'.' as c_char && *theline.add(1) == NUL;
                xfree(theline.cast::<c_void>());
                if end {
                    break;
                }
            }
            return;
        }

        if (*eap).forceit != 0 {
            (*curbuf).b_p_ai = if (*curbuf).b_p_ai != 0 { 0 } else { 1 };
        }

        if (*eap).cmdidx != cmdidx_T::CMD_change as c_int && (*curbuf).b_p_ai != 0 && lnum > 0 {
            append_indent = get_indent_lnum(lnum);
        }

        if (*eap).cmdidx != cmdidx_T::CMD_append as c_int {
            lnum -= 1;
        }

        if empty && lnum == 1 {
            lnum = 0;
        }

        let save_State = State;
        State = MODE_INSERT;
        if (*curbuf).b_p_iminsert == B_IMODE_LMAP {
            State |= MODE_LANGMAP;
        }

        loop {
            msg_scroll = 1;
            need_wait_return = false;
            if (*curbuf).b_p_ai != 0 {
                if append_indent >= 0 {
                    indent = append_indent;
                    append_indent = -1;
                } else if lnum > 0 {
                    indent = get_indent_lnum(lnum);
                }
            }
            let theline = get_append_line(eap, indent);
            lines_left = Rows - 1;
            if theline.is_null() {
                break;
            }

            let mut vcol: c_int = 0;
            let mut p = theline;
            while indent > vcol {
                if *p == b' ' as c_char {
                    vcol += 1;
                } else if *p == TAB as c_char {
                    vcol += 8 - vcol % 8;
                } else {
                    break;
                }
                p = p.add(1);
            }
            if (*p == b'.' as c_char && *p.add(1) == NUL)
                || (!did_undo && u_save(lnum, lnum + 1 + if empty { 1 } else { 0 }) == FAIL)
            {
                xfree(theline.cast::<c_void>());
                break;
            }

            if *p == NUL {
                *theline = NUL;
            }

            did_undo = true;
            ml_append(lnum, theline, 0, false);
            if empty {
                appended_lines(lnum, 1);
            } else {
                appended_lines_mark(lnum, 1);
            }

            xfree(theline.cast::<c_void>());
            lnum += 1;

            if empty {
                ml_delete(2);
                empty = false;
            }
        }
        State = save_State;
        ui_cursor_shape();

        if (*eap).forceit != 0 {
            (*curbuf).b_p_ai = if (*curbuf).b_p_ai != 0 { 0 } else { 1 };
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start.lnum = if (*eap).line2 < (*curbuf).b_ml.ml_line_count {
                (*eap).line2 + 1
            } else {
                (*curbuf).b_ml.ml_line_count
            };
            if (*eap).cmdidx != cmdidx_T::CMD_append as c_int {
                (*curbuf).b_op_start.lnum -= 1;
            }
            (*curbuf).b_op_end.lnum = if (*eap).line2 < lnum {
                lnum
            } else {
                (*curbuf).b_op_start.lnum
            };
            (*curbuf).b_op_end.col = 0;
            (*curbuf).b_op_start.col = 0;
        }
        (*curwin).w_cursor.lnum = lnum;
        check_cursor_lnum(curwin);
        beginline(BL_SOL | BL_FIX);

        need_wait_return = false;
        redraw_curbuf_later(UPD_NOT_VALID);
    }
}

/// `":change"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_change(eap: *mut exarg_T) {
    unsafe {
        if (*eap).skip != 0 {
            ex_append(eap);
            return;
        }
        if (*eap).line2 >= (*eap).line1 && u_save((*eap).line1 - 1, (*eap).line2 + 1) == FAIL {
            return;
        }

        if if (*eap).forceit != 0 {
            (*curbuf).b_p_ai == 0
        } else {
            (*curbuf).b_p_ai != 0
        } {
            append_indent = get_indent_lnum((*eap).line1);
        }

        let mut lnum = (*eap).line2;
        while lnum >= (*eap).line1 {
            if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
                break;
            }
            ml_delete((*eap).line1);
            lnum -= 1;
        }

        check_cursor_lnum(curwin);
        deleted_lines_mark((*eap).line1, (*eap).line2 - lnum);

        (*eap).line2 = (*eap).line1;
        ex_append(eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_z(eap: *mut exarg_T) {
    unsafe {
        let mut bigness: i64;
        let mut minus: c_int = 0;
        let mut start: linenr_T;
        let mut end: linenr_T;
        let mut curs: linenr_T;
        let lnum = (*eap).line2;

        if (*eap).forceit != 0 {
            bigness = (Rows - 1) as i64;
        } else if firstwin == lastwin {
            bigness = (*curwin).w_onebuf_opt.wo_scr * 2;
        } else {
            bigness = ((*curwin).w_view_height - 3) as i64;
        }
        bigness = bigness.max(1);

        let mut x = (*eap).arg;
        let kind = x;
        if *kind == b'-' as c_char
            || *kind == b'+' as c_char
            || *kind == b'=' as c_char
            || *kind == b'^' as c_char
            || *kind == b'.' as c_char
        {
            x = x.add(1);
        }
        while *x == b'-' as c_char || *x == b'+' as c_char {
            x = x.add(1);
        }

        if *x != 0 {
            if !ascii_isdigit(*x as c_int) {
                emsg(gettext(E_NON_NUMERIC_ARGUMENT_TO_Z.as_ptr()));
                return;
            }
            bigness = atol(x) as i64;

            if bigness > 2 * ((*curbuf).b_ml.ml_line_count as i64) || bigness < 0 {
                bigness = 2 * ((*curbuf).b_ml.ml_line_count as i64);
            }

            p_window = bigness as c_int as OptInt;
            if *kind == b'=' as c_char {
                bigness += 2;
            }
        }

        if *kind == b'-' as c_char || *kind == b'+' as c_char {
            x = kind.add(1);
            while *x == *kind {
                x = x.add(1);
            }
        }

        match *kind as u8 {
            b'-' => {
                start = lnum - (bigness as linenr_T) * (x.offset_from(kind) as linenr_T) + 1;
                end = start + (bigness as linenr_T) - 1;
                curs = end;
            }

            b'=' => {
                start = lnum - ((bigness as linenr_T) + 1) / 2 + 1;
                end = lnum + ((bigness as linenr_T) + 1) / 2 - 1;
                curs = lnum;
                minus = 1;
            }

            b'^' => {
                start = lnum - (bigness as linenr_T) * 2;
                end = lnum - (bigness as linenr_T);
                curs = lnum - (bigness as linenr_T);
            }

            b'.' => {
                start = lnum - ((bigness as linenr_T) + 1) / 2 + 1;
                end = lnum + ((bigness as linenr_T) + 1) / 2 - 1;
                curs = end;
            }

            _ => {
                start = lnum;
                if *kind == b'+' as c_char {
                    start += (bigness as linenr_T) * (x.offset_from(kind) as linenr_T - 1) + 1;
                } else if (*eap).addr_count == 0 {
                    start += 1;
                }
                end = start + (bigness as linenr_T) - 1;
                curs = end;
            }
        }

        start = start.max(1);
        end = end.min((*curbuf).b_ml.ml_line_count);
        curs = curs.max(1).min((*curbuf).b_ml.ml_line_count);

        for i in start..=end {
            if minus != 0 && i == lnum {
                msg_putchar(b'\n' as c_int);
                for _ in 1..Columns {
                    msg_putchar(b'-' as c_int);
                }
            }

            print_line(
                i,
                ((*eap).flags & EXFLAG_NR) != 0,
                ((*eap).flags & EXFLAG_LIST) != 0,
                i == start,
            );

            if minus != 0 && i == lnum {
                msg_putchar(b'\n' as c_int);
                for _ in 1..Columns {
                    msg_putchar(b'-' as c_int);
                }
            }
        }

        if (*curwin).w_cursor.lnum != curs {
            (*curwin).w_cursor.lnum = curs;
            (*curwin).w_cursor.col = 0;
        }
    }
}

/// @return true if the secure flag is set and also give an error message.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_secure() -> bool {
    unsafe {
        if secure != 0 {
            secure = 2;
            emsg(gettext(e_curdir.as_ptr()));
            return true;
        }

        if sandbox != 0 {
            emsg(gettext(e_sandbox.as_ptr()));
            return true;
        }
        false
    }
}

/// Get old substitute replacement string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn sub_get_replacement(ret_sub: *mut SubReplacementString) {
    unsafe {
        *ret_sub = old_sub;
    }
}

/// Set substitute string and timestamp.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn sub_set_replacement(sub: SubReplacementString) {
    unsafe {
        xfree(old_sub.sub.cast::<c_void>());
        if sub.additional_data != old_sub.additional_data {
            xfree(old_sub.additional_data);
        }
        old_sub = sub;
    }
}

/// Recognize `":%s/\n//"` and turn it into a join command.
unsafe fn sub_joining_lines(
    eap: *mut exarg_T,
    pat: NvimString,
    sub: *const c_char,
    cmd: *const c_char,
    save: bool,
    keeppatterns: bool,
) -> bool {
    unsafe {
        if !pat.data.is_null()
            && strcmp(pat.data, c"\\n".as_ptr()) == 0
            && *sub == NUL
            && (*cmd == NUL
                || (*cmd.add(1) == NUL
                    && (*cmd == b'g' as c_char
                        || *cmd == b'l' as c_char
                        || *cmd == b'p' as c_char
                        || *cmd == b'#' as c_char)))
        {
            if (*eap).skip != 0 {
                return true;
            }
            (*curwin).w_cursor.lnum = (*eap).line1;
            if *cmd == b'l' as c_char {
                (*eap).flags = EXFLAG_LIST;
            } else if *cmd == b'#' as c_char {
                (*eap).flags = EXFLAG_NR;
            } else if *cmd == b'p' as c_char {
                (*eap).flags = EXFLAG_PRINT;
            }

            let joined_lines_count = (*eap).line2 - (*eap).line1
                + 1
                + if (*eap).line2 < (*curbuf).b_ml.ml_line_count {
                    1
                } else {
                    0
                };
            if joined_lines_count > 1 {
                do_join(joined_lines_count as usize, false, true, false, true);
                sub_nsubs = joined_lines_count - 1;
                sub_nlines = 1;
                do_sub_msg(false);
                ex_may_print(eap);
            }

            if save {
                if !keeppatterns {
                    save_re_pat(RE_SUBST, pat.data, pat.size, p_magic);
                }
                add_to_history(HIST_SEARCH, pat.data, pat.size, true, NUL as c_int);
            }

            return true;
        }

        false
    }
}

/// Allocate memory to store the replacement text for `:substitute`.
unsafe fn sub_grow_buf(
    new_start: *mut NvimString,
    new_start_size: *mut usize,
    mut needed_size: usize,
) {
    unsafe {
        if (*new_start).data.is_null() {
            *new_start_size = needed_size + 50;
            (*new_start).data = xcalloc(1, *new_start_size).cast::<c_char>();
            *(*new_start).data = NUL;
            (*new_start).size = 0;
        } else {
            needed_size += (*new_start).size;
            if needed_size > *new_start_size {
                let prev_new_start_size = *new_start_size;
                *new_start_size = needed_size + 50;
                let added_len = *new_start_size - prev_new_start_size;
                (*new_start).data =
                    xrealloc((*new_start).data.cast::<c_void>(), *new_start_size).cast::<c_char>();
                ptr::write_bytes(
                    (*new_start).data.add(prev_new_start_size).cast::<u8>(),
                    0,
                    added_len,
                );
            }
        }
    }
}

/// Parse `cmd` string for `:substitute`'s `{flags}` and update `subflags_arg` accordingly.
unsafe fn sub_parse_flags(
    mut cmd: *mut c_char,
    subflags_arg: *mut subflags_T,
    which_pat: *mut c_int,
) -> *mut c_char {
    unsafe {
        if *cmd == b'&' as c_char {
            cmd = cmd.add(1);
        } else {
            (*subflags_arg).do_all = p_gd != 0;
            (*subflags_arg).do_ask = false;
            (*subflags_arg).do_error = true;
            (*subflags_arg).do_print = false;
            (*subflags_arg).do_list = false;
            (*subflags_arg).do_count = false;
            (*subflags_arg).do_number = false;
            (*subflags_arg).do_ic = SubIgnoreType::kSubHonorOptions;
        }
        while *cmd != NUL {
            if *cmd == b'g' as c_char {
                (*subflags_arg).do_all = !(*subflags_arg).do_all;
            } else if *cmd == b'c' as c_char {
                (*subflags_arg).do_ask = !(*subflags_arg).do_ask;
            } else if *cmd == b'n' as c_char {
                (*subflags_arg).do_count = true;
            } else if *cmd == b'e' as c_char {
                (*subflags_arg).do_error = !(*subflags_arg).do_error;
            } else if *cmd == b'r' as c_char {
                *which_pat = RE_LAST;
            } else if *cmd == b'p' as c_char {
                (*subflags_arg).do_print = true;
            } else if *cmd == b'#' as c_char {
                (*subflags_arg).do_print = true;
                (*subflags_arg).do_number = true;
            } else if *cmd == b'l' as c_char {
                (*subflags_arg).do_print = true;
                (*subflags_arg).do_list = true;
            } else if *cmd == b'i' as c_char {
                (*subflags_arg).do_ic = SubIgnoreType::kSubIgnoreCase;
            } else if *cmd == b'I' as c_char {
                (*subflags_arg).do_ic = SubIgnoreType::kSubMatchCase;
            } else {
                break;
            }
            cmd = cmd.add(1);
        }
        if (*subflags_arg).do_count {
            (*subflags_arg).do_ask = false;
        }

        cmd
    }
}

/// Skip over the `"sub"` part in `:s/pat/sub/` where `"delimiter"` is the separating
/// character.
unsafe fn skip_substitute(start: *mut c_char, delimiter: c_int) -> *mut c_char {
    unsafe {
        let mut p = start;
        while *p != NUL {
            if *p as c_int == delimiter {
                *p = NUL;
                p = p.add(1);
                break;
            }
            if *p == b'\\' as c_char && *p.add(1) != 0 {
                p = p.add(1);
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        p
    }
}

unsafe fn check_regexp_delim(c: c_int) -> c_int {
    unsafe {
        if isalpha(c) != 0 {
            emsg(gettext(
                c"E146: Regular expressions can't be delimited by letters".as_ptr(),
            ));
            return FAIL;
        }
        OK
    }
}

unsafe fn push_preview_lines(
    cmdpreview_ns: c_int,
    preview_lines: &mut PreviewLines,
    current_match: SubResult,
) {
    unsafe {
        if cmdpreview_ns > 0 {
            let match_lines = current_match.end.lnum - current_match.start.lnum + 1;
            if preview_lines.subresults.size > 0 {
                let last = (*preview_lines
                    .subresults
                    .items
                    .add(preview_lines.subresults.size - 1))
                .end
                .lnum;
                if last == current_match.start.lnum {
                    preview_lines.lines_needed += match_lines - 1;
                } else {
                    preview_lines.lines_needed += match_lines;
                }
            } else {
                preview_lines.lines_needed += match_lines;
            }
            preview_lines.subresults.push(current_match);
        }
    }
}

/// Perform a substitution from line `eap->line1` to line `eap->line2`.
unsafe fn do_sub(
    eap: *mut exarg_T,
    mut tm: proftime_T,
    cmdpreview_ns: c_int,
    cmdpreview_bufnr: handle_T,
) -> c_int {
    unsafe {
        let mut i: c_int;
        let mut regmatch: regmmatch_T = std::mem::zeroed();
        let mut sub: *mut c_char = ptr::null_mut();
        let mut pat = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        let delimiter: c_int;
        let mut has_second_delim = false;
        let mut sublen: usize;
        let mut got_quit = false;
        let mut got_match = false;
        let mut which_pat: c_int;
        let mut cmd = (*eap).arg;
        let mut first_line: linenr_T = 0;
        let mut last_line: linenr_T = 0;
        let old_line_count = (*curbuf).b_ml.ml_line_count;
        let mut sub_firstline: NvimString;
        let mut endcolumn = false;
        let keeppatterns = (cmdmod.cmod_flags & CMOD_KEEPPATTERNS) != 0;
        let mut preview_lines = PreviewLines {
            subresults: KVec::new(),
            lines_needed: 0,
        };
        let old_cursor = (*curwin).w_cursor;

        let mut did_save = false;

        if global_busy == 0 {
            sub_nsubs = 0;
            sub_nlines = 0;
        }
        let start_nsubs = sub_nsubs;

        if (*eap).cmdidx == cmdidx_T::CMD_tilde as c_int {
            which_pat = RE_LAST;
        } else {
            which_pat = RE_SUBST;
        }

        if *(*eap).cmd == b's' as c_char
            && *cmd != NUL
            && !ascii_iswhite(*cmd as c_int)
            && vim_strchr(c"0123456789cegriIp|\"".as_ptr(), (*cmd as u8) as c_int).is_null()
        {
            if check_regexp_delim((*cmd as u8) as c_int) == FAIL {
                return 0;
            }

            if *cmd == b'\\' as c_char {
                cmd = cmd.add(1);
                if vim_strchr(c"/?&".as_ptr(), (*cmd as u8) as c_int).is_null() {
                    emsg(gettext(e_backslash.as_ptr()));
                    return 0;
                }
                if *cmd != b'&' as c_char {
                    which_pat = RE_SEARCH;
                }
                pat = NvimString {
                    data: c"".as_ptr().cast_mut(),
                    size: 0,
                };
                delimiter = (*cmd as u8) as c_int;
                cmd = cmd.add(1);
                has_second_delim = true;
            } else {
                which_pat = RE_LAST;
                delimiter = (*cmd as u8) as c_int;
                cmd = cmd.add(1);
                pat.data = cmd;
                cmd = skip_regexp_ex(
                    cmd,
                    delimiter,
                    p_magic,
                    &raw mut (*eap).arg,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                pat.size = cmd.offset_from(pat.data) as usize;
                if *cmd as c_int == delimiter {
                    *cmd = NUL;
                    cmd = cmd.add(1);
                    has_second_delim = true;
                }
            }

            let p = cmd;
            cmd = skip_substitute(cmd, delimiter);
            sub = xstrdup(p);

            if (*eap).skip == 0 && !keeppatterns && cmdpreview_ns <= 0 {
                sub_set_replacement(SubReplacementString {
                    sub: xstrdup(sub),
                    timestamp: os_time(),
                    additional_data: ptr::null_mut(),
                });
            }
        } else if (*eap).skip == 0 {
            if old_sub.sub.is_null() {
                emsg(gettext(e_nopresub.as_ptr()));
                return 0;
            }
            pat = NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
            sub = xstrdup(old_sub.sub);
            endcolumn = (*curwin).w_curswant == MAXCOL;
        }

        if !sub.is_null() && sub_joining_lines(eap, pat, sub, cmd, cmdpreview_ns <= 0, keeppatterns)
        {
            xfree(sub.cast::<c_void>());
            return 0;
        }

        cmd = sub_parse_flags(cmd, &raw mut subflags, &raw mut which_pat);

        let save_do_all = subflags.do_all;
        let save_do_ask = subflags.do_ask;

        cmd = skipwhite(cmd);
        if ascii_isdigit(*cmd as c_int) {
            let count_arg = cmd;
            i = getdigits_int(&raw mut cmd, false, c_int::MAX);
            if i <= 0 && (*eap).skip == 0 && subflags.do_error {
                emsg(gettext(e_zerocount.as_ptr()));
                xfree(sub.cast::<c_void>());
                return 0;
            } else if i == c_int::MAX {
                semsg!(
                    gettext(e_val_too_large_len.as_ptr()),
                    cmd.offset_from(count_arg) as c_int,
                    count_arg,
                );
                xfree(sub.cast::<c_void>());
                return 0;
            }
            (*eap).line1 = (*eap).line2;
            (*eap).line2 += i - 1;
            (*eap).line2 = (*eap).line2.min((*curbuf).b_ml.ml_line_count);
        }

        cmd = skipwhite(cmd);
        if *cmd != NUL && *cmd != b'"' as c_char {
            (*eap).nextcmd = check_nextcmd(cmd);
            if (*eap).nextcmd.is_null() {
                semsg!(gettext(e_trailing_arg.as_ptr()), cmd);
                xfree(sub.cast::<c_void>());
                return 0;
            }
        }

        if (*eap).skip != 0 {
            xfree(sub.cast::<c_void>());
            return 0;
        }

        if !subflags.do_count && !modifiable(curbuf) {
            emsg(gettext(e_modifiable.as_ptr()));
            xfree(sub.cast::<c_void>());
            return 0;
        }

        let magic = if (*eap).cmdidx == cmdidx_T::CMD_smagic as c_int {
            true
        } else if (*eap).cmdidx == cmdidx_T::CMD_snomagic as c_int {
            false
        } else {
            p_magic != 0
        };
        if search_regcomp(
            pat.data,
            pat.size,
            ptr::null_mut(),
            RE_SUBST,
            which_pat,
            if cmdpreview_ns > 0 { 0 } else { SEARCH_HIS },
            magic,
            &raw mut regmatch,
        ) == FAIL
        {
            if subflags.do_error {
                emsg(gettext(e_invcmd.as_ptr()));
            }
            xfree(sub.cast::<c_void>());
            return 0;
        }

        if subflags.do_ic == SubIgnoreType::kSubIgnoreCase {
            regmatch.rmm_ic = 1;
        } else if subflags.do_ic == SubIgnoreType::kSubMatchCase {
            regmatch.rmm_ic = 0;
        }

        sub_firstline = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };

        debug_assert!(!sub.is_null());

        if *sub == b'\\' as c_char && *sub.add(1) == b'=' as c_char {
            let p = xstrdup(sub);
            xfree(sub.cast::<c_void>());
            sub = p;
        } else {
            let p = regtilde(sub, p_magic, cmdpreview_ns > 0);
            if p != sub {
                xfree(sub.cast::<c_void>());
                sub = p;
            }
        }

        let mut line2 = (*eap).line2;
        let mut timed_out: c_int = 0;

        let mut lnum = (*eap).line1;
        while lnum <= line2
            && !got_quit
            && timed_out == 0
            && !aborting()
            && (cmdpreview_ns <= 0
                || preview_lines.lines_needed <= p_cwh as linenr_T
                || lnum <= (*curwin).w_botline)
        {
            let mut nmatch = vim_regexec_multi(
                &raw mut regmatch,
                curwin,
                curbuf,
                lnum,
                0,
                &raw mut tm,
                &raw mut timed_out,
            );
            if nmatch != 0 {
                let mut copycol: colnr_T = 0;
                let mut matchcol: colnr_T = 0;
                let mut prev_matchcol: colnr_T = MAXCOL;
                let mut new_start = NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                };
                let mut new_start_size: usize = 0;
                let mut new_end: *mut c_char;
                let mut did_sub = false;
                let mut nmatch_tl: linenr_T = 0;
                let mut do_again: bool;
                let mut skip_match = false;
                let mut sub_firstlnum: linenr_T = lnum;
                let mut lnum_start: linenr_T;

                let mut line_matches: KVec<LineData> = KVec::new();

                if !got_match {
                    setpcmark();
                    got_match = true;
                }

                loop {
                    let mut current_match = SubResult {
                        start: lpos_T { lnum: 0, col: 0 },
                        end: lpos_T { lnum: 0, col: 0 },
                        pre_match: 0,
                    };

                    if regmatch.startpos[0].lnum > 0 {
                        current_match.pre_match = lnum;
                        lnum += regmatch.startpos[0].lnum;
                        sub_firstlnum += regmatch.startpos[0].lnum;
                        nmatch -= regmatch.startpos[0].lnum;
                        api_clear_string(&mut sub_firstline);
                    }

                    current_match.start.lnum = sub_firstlnum;

                    if lnum > (*curbuf).b_ml.ml_line_count {
                        break;
                    }
                    if sub_firstline.data.is_null() {
                        sub_firstline = cbuf_to_string(
                            ml_get(sub_firstlnum),
                            ml_get_len(sub_firstlnum) as usize,
                        );
                    }

                    (*curwin).w_cursor.lnum = lnum;
                    do_again = false;

                    'skip: {
                        if matchcol == prev_matchcol
                            && regmatch.endpos[0].lnum == 0
                            && matchcol == regmatch.endpos[0].col
                        {
                            if *sub_firstline.data.add(matchcol as usize) == NUL {
                                skip_match = true;
                            } else {
                                matchcol += utfc_ptr2len(sub_firstline.data.add(matchcol as usize));
                            }
                            current_match.start.col = matchcol;
                            current_match.end.lnum = sub_firstlnum;
                            current_match.end.col = matchcol;
                            break 'skip;
                        }

                        matchcol = regmatch.endpos[0].col;
                        prev_matchcol = matchcol;

                        if subflags.do_count {
                            if nmatch > 1 {
                                matchcol = sub_firstline.size as colnr_T;
                                nmatch = 1;
                                skip_match = true;
                            }
                            sub_nsubs += 1;
                            did_sub = true;
                            if !(*sub == b'\\' as c_char && *sub.add(1) == b'=' as c_char) {
                                break 'skip;
                            }
                        }

                        if subflags.do_ask && cmdpreview_ns <= 0 {
                            let mut typed: c_int = 0;
                            let save_State = State;
                            (*curwin).w_cursor.col = regmatch.startpos[0].col;

                            if (*curwin).w_onebuf_opt.wo_crb != 0 {
                                do_check_cursorbind();
                            }

                            if !vim_strchr(p_cpo, kCpoUndo).is_null() {
                                no_u_sync += 1;
                            }

                            while subflags.do_ask {
                                let mut orig_line = NvimString {
                                    data: ptr::null_mut(),
                                    size: 0,
                                };
                                let mut len_change: c_int = 0;
                                let save_p_lz = p_lz;
                                let save_p_fen = (*curwin).w_onebuf_opt.wo_fen;

                                (*curwin).w_onebuf_opt.wo_fen = 0;
                                let temp = RedrawingDisabled;
                                RedrawingDisabled = 0;

                                p_lz = 0;

                                if !new_start.data.is_null() {
                                    orig_line =
                                        cbuf_to_string(ml_get(lnum), ml_get_len(lnum) as usize);
                                    let new_line = NvimString {
                                        data: concat_str(
                                            new_start.data,
                                            sub_firstline.data.add(copycol as usize),
                                        ),
                                        size: new_start.size
                                            + (sub_firstline.size - copycol as usize),
                                    };

                                    len_change = new_line.size as c_int - orig_line.size as c_int;
                                    (*curwin).w_cursor.col += len_change;
                                    ml_replace(lnum, new_line.data, false);
                                }

                                Search.match_lines =
                                    regmatch.endpos[0].lnum - regmatch.startpos[0].lnum;
                                Search.match_endcol = regmatch.endpos[0].col + len_change;
                                if Search.match_lines == 0 && Search.match_endcol == 0 {
                                    Search.match_endcol = 1;
                                }
                                Search.hl_match = true;

                                update_topline(curwin);
                                validate_cursor(curwin);
                                redraw_later(curwin, UPD_SOME_VALID);
                                show_cursor_info_later(true);
                                update_screen();
                                redraw_later(curwin, UPD_SOME_VALID);

                                (*curwin).w_onebuf_opt.wo_fen = save_p_fen;

                                let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
                                let p_fmt = gettext(
                                    c"replace with %s? (y)es/(n)o/(a)ll/(q)uit/(l)ast/scroll up(^E)/down(^Y)"
                                        .as_ptr(),
                                );
                                snprintf(iobuff_ptr, IOSIZE, p_fmt, sub);
                                let p = xstrdup(iobuff_ptr);
                                typed = prompt_for_input(p, HLF_R, true, ptr::null_mut());
                                Search.hl_match = false;
                                xfree(p.cast::<c_void>());

                                msg_didout = false;
                                gotocmdline(true);
                                p_lz = save_p_lz;
                                RedrawingDisabled = temp;

                                if !orig_line.data.is_null() {
                                    ml_replace(lnum, orig_line.data, false);
                                }

                                need_wait_return = false;
                                if typed == b'q' as c_int || typed == ESC || typed == Ctrl_C {
                                    got_quit = true;
                                    break;
                                }
                                if typed == b'n' as c_int {
                                    break;
                                }
                                if typed == b'y' as c_int {
                                    break;
                                }
                                if typed == b'l' as c_int {
                                    subflags.do_all = false;
                                    line2 = lnum;
                                    break;
                                }
                                if typed == b'a' as c_char as c_int {
                                    subflags.do_ask = false;
                                    break;
                                }
                                if typed == Ctrl_E {
                                    scrollup_clamp();
                                } else if typed == Ctrl_Y {
                                    scrolldown_clamp();
                                }
                            }
                            State = save_State;
                            setmouse();
                            if !vim_strchr(p_cpo, kCpoUndo).is_null() {
                                no_u_sync -= 1;
                            }

                            if typed == b'n' as c_int {
                                if nmatch > 1 {
                                    matchcol = sub_firstline.size as colnr_T;
                                    skip_match = true;
                                }
                                break 'skip;
                            }
                            if got_quit {
                                break 'skip;
                            }
                        }

                        (*curwin).w_cursor.col = regmatch.startpos[0].col;

                        if nmatch > (*curbuf).b_ml.ml_line_count - sub_firstlnum + 1 {
                            nmatch = (*curbuf).b_ml.ml_line_count - sub_firstlnum + 1;
                            current_match.end.lnum = sub_firstlnum + nmatch;
                            skip_match = true;
                            if nmatch < 0 {
                                break 'skip;
                            }
                        }

                        if cmdpreview_ns > 0 && !has_second_delim {
                            current_match.start.col = regmatch.startpos[0].col;
                            if current_match.end.lnum == 0 {
                                current_match.end.lnum = sub_firstlnum + nmatch - 1;
                            }
                            current_match.end.col = regmatch.endpos[0].col;

                            if nmatch > 1 {
                                sub_firstlnum += nmatch - 1;
                                xfree(sub_firstline.data.cast::<c_void>());
                                sub_firstline = cbuf_to_string(
                                    ml_get(sub_firstlnum),
                                    ml_get_len(sub_firstlnum) as usize,
                                );
                                if sub_firstlnum <= line2 {
                                    do_again = true;
                                } else {
                                    subflags.do_all = false;
                                }
                            }
                            if skip_match {
                                xfree(sub_firstline.data.cast::<c_void>());
                                sub_firstline = cbuf_to_string(c"".as_ptr(), 0);
                                copycol = 0;
                            }
                            lnum += nmatch - 1;

                            break 'skip;
                        }

                        if cmdpreview_ns <= 0 || has_second_delim {
                            lnum_start = lnum;
                            let save_ma = (*curbuf).b_p_ma;
                            let save_sandbox = sandbox;
                            if subflags.do_count {
                                (*curbuf).b_p_ma = 0;
                                sandbox += 1;
                            }
                            let subflags_save = subflags;

                            textlock += 1;
                            sublen = vim_regsub_multi(
                                &raw mut regmatch,
                                sub_firstlnum - regmatch.startpos[0].lnum,
                                sub,
                                sub_firstline.data,
                                0,
                                REGSUB_BACKSLASH | if p_magic != 0 { REGSUB_MAGIC } else { 0 },
                            ) as usize;
                            textlock -= 1;

                            subflags = subflags_save;
                            if sublen == 0 || aborting() || subflags.do_count {
                                (*curbuf).b_p_ma = save_ma;
                                sandbox = save_sandbox;
                                break 'skip;
                            }

                            let tmp: NvimString;
                            if nmatch == 1 {
                                tmp = sub_firstline;
                            } else {
                                let lastlnum = sub_firstlnum + nmatch - 1;
                                tmp =
                                    cbuf_as_string(ml_get(lastlnum), ml_get_len(lastlnum) as usize);
                                nmatch_tl += nmatch - 1;
                            }
                            let copy_len = (regmatch.startpos[0].col - copycol) as usize;
                            sub_grow_buf(
                                &raw mut new_start,
                                &raw mut new_start_size,
                                copy_len
                                    + (tmp.size - regmatch.endpos[0].col as usize)
                                    + sublen
                                    + 1,
                            );

                            ptr::copy(
                                sub_firstline.data.add(copycol as usize),
                                new_start.data.add(new_start.size),
                                copy_len,
                            );
                            new_start.size += copy_len;

                            new_end = new_start.data.add(new_start.size);

                            if new_start_size - copy_len < sublen {
                                sublen = new_start_size - copy_len - 1;
                            }

                            let start_col = new_start.size as c_int;
                            current_match.start.col = start_col;

                            textlock += 1;
                            let mut n = vim_regsub_multi(
                                &raw mut regmatch,
                                sub_firstlnum - regmatch.startpos[0].lnum,
                                sub,
                                new_end,
                                sublen as colnr_T,
                                REGSUB_COPY
                                    | REGSUB_BACKSLASH
                                    | if p_magic != 0 { REGSUB_MAGIC } else { 0 },
                            ) as usize;
                            if n > 0 {
                                new_start.size += n - 1;
                            }
                            textlock -= 1;
                            sub_nsubs += 1;
                            did_sub = true;

                            (*curwin).w_cursor.col = 0;

                            copycol = regmatch.endpos[0].col;

                            if nmatch > 1 {
                                sub_firstlnum += nmatch - 1;
                                xfree(sub_firstline.data.cast::<c_void>());
                                sub_firstline = cbuf_to_string(
                                    ml_get(sub_firstlnum),
                                    ml_get_len(sub_firstlnum) as usize,
                                );
                                if sub_firstlnum <= line2 {
                                    do_again = true;
                                } else {
                                    subflags.do_all = false;
                                }
                            }
                            if skip_match {
                                xfree(sub_firstline.data.cast::<c_void>());
                                sub_firstline = cbuf_to_string(c"".as_ptr(), 0);
                                copycol = 0;
                            }

                            let mut replaced_bytes: bcount_t = 0;
                            let start = regmatch.startpos[0];
                            let end = regmatch.endpos[0];
                            for j in 0..(nmatch - 1) {
                                replaced_bytes += strlen(ml_get(lnum_start + j)) as bcount_t + 1;
                            }
                            replaced_bytes += (end.col - start.col) as bcount_t;

                            let lnum_before_newlines = lnum;

                            let mut p1 = new_end;
                            while *p1 != NUL {
                                if *p1 == b'\\' as c_char && *p1.add(1) != NUL {
                                    sublen -= 1;
                                    n = new_start.size - p1.offset_from(new_start.data) as usize;
                                    ptr::copy(p1.add(1), p1, n + 1);
                                    new_start.size -= 1;
                                    p1 = p1.add(1);
                                } else if *p1 == CAR as c_char {
                                    if u_inssub(lnum) == OK {
                                        let plen = (p1.offset_from(new_start.data) + 1) as colnr_T;
                                        *p1 = NUL;
                                        ml_append(lnum - 1, new_start.data, plen, false);
                                        mark_adjust(lnum + 1, MAXLNUM, 1, 0, kExtmarkNOOP);

                                        if subflags.do_ask {
                                            appended_lines(lnum - 1, 1);
                                        } else {
                                            if first_line == 0 {
                                                first_line = lnum;
                                            }
                                            last_line = lnum + 1;
                                        }
                                        sub_firstlnum += 1;
                                        lnum += 1;
                                        line2 += 1;
                                        (*curwin).w_cursor.lnum += 1;
                                        n = new_start.size - plen as usize;
                                        ptr::copy(p1.add(1), new_start.data, n + 1);
                                        new_start.size = n;
                                        p1 = new_start.data;
                                    } else {
                                        p1 = p1.add(1);
                                    }
                                } else {
                                    p1 = p1.add(utfc_ptr2len(p1) as usize);
                                }
                            }
                            let new_endcol = new_start.size as colnr_T;
                            current_match.end.col = new_endcol;
                            current_match.end.lnum = lnum;

                            let matchcols =
                                end.col - if end.lnum == start.lnum { start.col } else { 0 };
                            let subcols =
                                new_endcol - if lnum == lnum_start { start_col } else { 0 };
                            if !did_save {
                                u_save_cursor();
                                did_save = true;
                            }

                            line_matches.push(LineData {
                                start_col,
                                start,
                                end,
                                matchcols,
                                matchbytes: replaced_bytes,
                                subcols,
                                subbytes: (sublen as colnr_T - 1) as bcount_t,
                                lnum_before: lnum_before_newlines,
                                lnum_after: lnum,
                            });
                        }
                    }

                    // skip:
                    let lastone = skip_match
                        || got_int
                        || got_quit
                        || lnum > line2
                        || !(subflags.do_all || do_again)
                        || (*sub_firstline.data.add(matchcol as usize) == NUL
                            && nmatch <= 1
                            && re_multiline(regmatch.regprog) == 0);
                    nmatch = -1;

                    if lastone
                        || nmatch_tl > 0
                        || {
                            nmatch = vim_regexec_multi(
                                &raw mut regmatch,
                                curwin,
                                curbuf,
                                sub_firstlnum,
                                matchcol,
                                &raw mut tm,
                                &raw mut timed_out,
                            );
                            nmatch == 0
                        }
                        || regmatch.startpos[0].lnum > 0
                    {
                        if !new_start.data.is_null() {
                            strcpy(
                                new_start.data.add(new_start.size),
                                sub_firstline.data.add(copycol as usize),
                            );
                            new_start.size += sub_firstline.size - copycol as usize;
                            matchcol = sub_firstline.size as colnr_T - matchcol;
                            prev_matchcol = sub_firstline.size as colnr_T - prev_matchcol;

                            if u_savesub(lnum) != OK {
                                break;
                            }
                            ml_replace(lnum, new_start.data, true);

                            for match_idx in 0..line_matches.size {
                                let m = *line_matches.items.add(match_idx);
                                extmark_splice(
                                    curbuf,
                                    m.lnum_before - 1,
                                    m.start_col,
                                    m.end.lnum - m.start.lnum,
                                    m.matchcols,
                                    m.matchbytes,
                                    m.lnum_after - m.lnum_before,
                                    m.subcols,
                                    m.subbytes,
                                    kExtmarkUndo,
                                );
                            }

                            line_matches.size = 0;

                            if nmatch_tl > 0 {
                                lnum += 1;
                                if u_savedel(lnum, nmatch_tl) != OK {
                                    break;
                                }
                                for _ in 0..nmatch_tl {
                                    ml_delete(lnum);
                                }
                                mark_adjust(
                                    lnum,
                                    lnum + nmatch_tl - 1,
                                    MAXLNUM,
                                    -nmatch_tl,
                                    kExtmarkNOOP,
                                );
                                if subflags.do_ask {
                                    deleted_lines(lnum, nmatch_tl);
                                }
                                lnum -= 1;
                                line2 -= nmatch_tl;
                                nmatch_tl = 0;
                            }

                            if subflags.do_ask {
                                changed_bytes(lnum, 0);
                            } else {
                                if first_line == 0 {
                                    first_line = lnum;
                                }
                                last_line = lnum + 1;
                            }

                            sub_firstlnum = lnum;
                            xfree(sub_firstline.data.cast::<c_void>());
                            sub_firstline = new_start;
                            new_start = NvimString {
                                data: ptr::null_mut(),
                                size: 0,
                            };
                            matchcol = sub_firstline.size as colnr_T - matchcol;
                            prev_matchcol = sub_firstline.size as colnr_T - prev_matchcol;
                            copycol = 0;
                        }
                        if nmatch == -1 && !lastone {
                            nmatch = vim_regexec_multi(
                                &raw mut regmatch,
                                curwin,
                                curbuf,
                                sub_firstlnum,
                                matchcol,
                                &raw mut tm,
                                &raw mut timed_out,
                            );
                        }

                        if nmatch <= 0 {
                            if nmatch == -1 {
                                lnum -= regmatch.startpos[0].lnum;
                            }
                            push_preview_lines(cmdpreview_ns, &mut preview_lines, current_match);
                            break;
                        }
                    }
                    push_preview_lines(cmdpreview_ns, &mut preview_lines, current_match);

                    line_breakcheck();
                }

                if did_sub {
                    sub_nlines += 1;
                }
                xfree(new_start.data.cast::<c_void>());
                api_clear_string(&mut sub_firstline);
                line_matches.destroy();
            }

            line_breakcheck();

            if timed_out != 0 || profile_passed_limit(tm) {
                got_quit = true;
            }
            lnum += 1;
        }

        (*curbuf).deleted_bytes2 = 0;

        if first_line != 0 {
            i = (*curbuf).b_ml.ml_line_count - old_line_count;
            changed_lines(curbuf, first_line, 0, last_line - i, i, false);

            let num_added = (last_line - first_line) as i64;
            let num_removed = num_added - i as i64;
            buf_updates_send_changes(curbuf, first_line, num_added, num_removed);
        }

        xfree(sub_firstline.data.cast::<c_void>());

        if subflags.do_count {
            (*curwin).w_cursor = old_cursor;
        }

        if sub_nsubs > start_nsubs {
            if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                (*curbuf).b_op_start.lnum = (*eap).line1;
                (*curbuf).b_op_end.lnum = line2;
                (*curbuf).b_op_end.col = 0;
                (*curbuf).b_op_start.col = 0;
            }

            if global_busy == 0 {
                if !subflags.do_ask {
                    if endcolumn {
                        coladvance(curwin, MAXCOL);
                    } else {
                        beginline(BL_WHITE | BL_FIX);
                    }
                }
                if cmdpreview_ns <= 0
                    && !do_sub_msg(subflags.do_count)
                    && subflags.do_ask
                    && p_ch > 0
                {
                    msg(c"".as_ptr(), 0);
                }
            } else {
                global_need_beginline = true;
            }
            if subflags.do_print {
                print_line(
                    (*curwin).w_cursor.lnum,
                    subflags.do_number,
                    subflags.do_list,
                    true,
                );
            }
        } else if global_busy == 0 {
            if got_int {
                emsg(gettext(e_interr.as_ptr()));
            } else if got_match {
                if p_ch > 0 && !ui_has(kUIMessages) {
                    msg(c"".as_ptr(), 0);
                }
            } else if subflags.do_error {
                semsg!(gettext(e_patnotf2.as_ptr()), get_search_pat());
            }
        }

        if subflags.do_ask && hasAnyFolding(curwin) != 0 {
            changed_window_setting(curwin);
        }

        vim_regfree(regmatch.regprog);
        xfree(sub.cast::<c_void>());

        subflags.do_all = save_do_all;
        subflags.do_ask = save_do_ask;

        let mut retv: c_int = 0;

        if cmdpreview_ns > 0 && !aborting() {
            if got_quit || profile_passed_limit(tm) {
                set_option_direct(kOptInccommand, cstr_as_obj(c"".as_ptr()), 0, SID_NONE);
            } else if *p_icm != NUL && !pat.data.is_null() {
                if pre_hl_id == 0 {
                    pre_hl_id = syn_check_group(c"Substitute".as_ptr(), 10);
                }
                retv = show_sub(
                    eap,
                    old_cursor,
                    &raw mut preview_lines,
                    pre_hl_id,
                    cmdpreview_ns,
                    cmdpreview_bufnr,
                );
            }
        }

        preview_lines.subresults.destroy();
        retv
    }
}

/// Give message for number of substitutions.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_sub_msg(count_only: bool) -> bool {
    unsafe {
        if (((sub_nsubs as OptInt) > p_report && (KeyTyped || sub_nlines > 1 || p_report < 1))
            || count_only)
            && messaging()
        {
            let msg_buf_ptr = (&raw mut msg_buf).cast::<c_char>();
            if got_int {
                strcpy(msg_buf_ptr, gettext(c"(Interrupted) ".as_ptr()));
            } else {
                *msg_buf_ptr = NUL;
            }

            let msg_single = if count_only {
                ngettext(
                    c"%lld match on %lld line".as_ptr(),
                    c"%lld matches on %lld line".as_ptr(),
                    sub_nsubs as c_ulong,
                )
            } else {
                ngettext(
                    c"%lld substitution on %lld line".as_ptr(),
                    c"%lld substitutions on %lld line".as_ptr(),
                    sub_nsubs as c_ulong,
                )
            };
            let msg_plural = if count_only {
                ngettext(
                    c"%lld match on %lld lines".as_ptr(),
                    c"%lld matches on %lld lines".as_ptr(),
                    sub_nsubs as c_ulong,
                )
            } else {
                ngettext(
                    c"%lld substitution on %lld lines".as_ptr(),
                    c"%lld substitutions on %lld lines".as_ptr(),
                    sub_nsubs as c_ulong,
                )
            };
            vim_snprintf_add(
                msg_buf_ptr,
                MSG_BUF_LEN,
                ngettext(msg_single, msg_plural, sub_nlines as c_ulong),
                sub_nsubs as i64,
                sub_nlines as i64,
            );
            if msg(msg_buf_ptr, 0) {
                set_keep_msg(msg_buf_ptr, 0);
            }
            return true;
        }
        if got_int {
            emsg(gettext(e_interr.as_ptr()));
            return true;
        }
        false
    }
}

unsafe fn global_exe_one(cmd: *mut c_char, lnum: linenr_T) {
    unsafe {
        (*curwin).w_cursor.lnum = lnum;
        (*curwin).w_cursor.col = 0;
        if *cmd == NUL || *cmd == b'\n' as c_char {
            do_cmdline(
                c"p".as_ptr().cast_mut(),
                None,
                ptr::null_mut(),
                DOCMD_NOWAIT,
            );
        } else {
            do_cmdline(cmd, None, ptr::null_mut(), DOCMD_NOWAIT);
        }
    }
}

/// Execute a global command (`:g` / `:v`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_global(eap: *mut exarg_T) {
    unsafe {
        let mut lnum: linenr_T;
        let type_char: c_int;
        let mut cmd: *mut c_char;
        let delim: c_char;
        let pat: *mut c_char;
        let patlen: usize;
        let mut regmatch: regmmatch_T = std::mem::zeroed();

        if global_busy != 0 && ((*eap).line1 != 1 || (*eap).line2 != (*curbuf).b_ml.ml_line_count) {
            emsg(gettext(
                c"E147: Cannot do :global recursive with a range".as_ptr(),
            ));
            return;
        }

        if (*eap).forceit != 0 {
            type_char = b'v' as c_int;
        } else {
            type_char = (*(*eap).cmd as u8) as c_int;
        }
        cmd = (*eap).arg;
        let mut which_pat = RE_LAST;

        if *cmd == b'\\' as c_char {
            cmd = cmd.add(1);
            if vim_strchr(c"/?&".as_ptr(), (*cmd as u8) as c_int).is_null() {
                emsg(gettext(e_backslash.as_ptr()));
                return;
            }
            if *cmd == b'&' as c_char {
                which_pat = RE_SUBST;
            } else {
                which_pat = RE_SEARCH;
            }
            cmd = cmd.add(1);
            pat = c"".as_ptr().cast_mut();
            patlen = 0;
        } else if *cmd == NUL {
            emsg(gettext(
                c"E148: Regular expression missing from global".as_ptr(),
            ));
            return;
        } else if check_regexp_delim((*cmd as u8) as c_int) == FAIL {
            return;
        } else {
            delim = *cmd;
            cmd = cmd.add(1);
            pat = cmd;
            cmd = skip_regexp_ex(
                cmd,
                (delim as u8) as c_int,
                p_magic,
                &raw mut (*eap).arg,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if *cmd == delim {
                *cmd = NUL;
                cmd = cmd.add(1);
            }
            patlen = strlen(pat);
        }

        let mut used_pat: *mut c_char = ptr::null_mut();
        if search_regcomp(
            pat,
            patlen,
            &raw mut used_pat,
            RE_BOTH,
            which_pat,
            SEARCH_HIS,
            p_magic != 0,
            &raw mut regmatch,
        ) == FAIL
        {
            emsg(gettext(e_invcmd.as_ptr()));
            return;
        }

        if global_busy != 0 {
            lnum = (*curwin).w_cursor.lnum;
            let matched = vim_regexec_multi(
                &raw mut regmatch,
                curwin,
                curbuf,
                lnum,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if (type_char == b'g' as c_int && matched != 0)
                || (type_char == b'v' as c_int && matched == 0)
            {
                global_exe_one(cmd, lnum);
            }
        } else {
            let mut ndone: c_int = 0;
            lnum = (*eap).line1;
            while lnum <= (*eap).line2 && !got_int {
                let matched = vim_regexec_multi(
                    &raw mut regmatch,
                    curwin,
                    curbuf,
                    lnum,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                if regmatch.regprog.is_null() {
                    break;
                }
                if (type_char == b'g' as c_int && matched != 0)
                    || (type_char == b'v' as c_int && matched == 0)
                {
                    ml_setmarked(lnum);
                    ndone += 1;
                }
                line_breakcheck();
                lnum += 1;
            }

            if got_int {
                msg(gettext(e_interr.as_ptr()), 0);
            } else if ndone == 0 {
                if type_char == b'v' as c_int {
                    smsg!(
                        0,
                        gettext(c"Pattern found in every line: %s".as_ptr()),
                        used_pat,
                    );
                } else {
                    smsg!(0, gettext(c"Pattern not found: %s".as_ptr()), used_pat);
                }
            } else {
                global_exe(cmd);
            }
            ml_clearmarked();
        }
        vim_regfree(regmatch.regprog);
    }
}

/// Execute `cmd` on lines marked with `ml_setmarked()`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn global_exe(cmd: *mut c_char) {
    unsafe {
        let old_buf = curbuf;

        setpcmark();

        msg_didout = true;

        sub_nsubs = 0;
        sub_nlines = 0;
        global_need_msg_kind = true;
        global_need_beginline = false;
        global_busy = 1;
        let old_lcount = (*curbuf).b_ml.ml_line_count;

        while !got_int && global_busy == 1 {
            let lnum = ml_firstmarked();
            if lnum == 0 {
                break;
            }
            global_exe_one(cmd, lnum);
            os_breakcheck();
        }

        global_busy = 0;
        if global_need_beginline {
            beginline(BL_WHITE | BL_FIX);
        } else {
            check_cursor(curwin);
        }

        changed_line_abv_curs();

        if msg_col == 0 && msg_scrolled == 0 {
            msg_didout = false;
        }

        if !do_sub_msg(false) && curbuf == old_buf {
            msgmore((*curbuf).b_ml.ml_line_count - old_lcount);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_old_sub() {
    unsafe {
        sub_set_replacement(SubReplacementString {
            sub: ptr::null_mut(),
            timestamp: 0,
            additional_data: ptr::null_mut(),
        });
    }
}

/// Set up for a tagpreview.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prepare_tagpreview(undo_sync: bool, use_previewpopup: bool) -> bool {
    unsafe {
        if (*curwin).w_onebuf_opt.wo_pvw != 0 {
            return false;
        }

        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_onebuf_opt.wo_pvw != 0 {
                if use_previewpopup {
                    if (*wp).w_kind != kWinPreview {
                        wp = (*wp).w_next;
                        continue;
                    }
                } else if (*wp).w_floating {
                    wp = (*wp).w_next;
                    continue;
                }
                if use_previewpopup {
                    win_float_anchor_preview(wp);
                }
                win_enter(wp, undo_sync);
                return false;
            }
            wp = (*wp).w_next;
        }

        if use_previewpopup {
            let wp = win_float_special(false, true, kWinPreview);
            if wp.is_null() {
                return false;
            }
            win_enter(wp, undo_sync);
            return true;
        }
        if win_split(
            if g_do_tagpreview > 0 {
                g_do_tagpreview
            } else {
                0
            },
            0,
        ) == FAIL
        {
            return false;
        }

        (*curwin).w_onebuf_opt.wo_pvw = 1;
        (*curwin).w_onebuf_opt.wo_wfh = 1;
        (*curwin).w_onebuf_opt.wo_scb = 0;
        (*curwin).w_onebuf_opt.wo_crb = 0;
        (*curwin).w_onebuf_opt.wo_diff = 0;

        set_option_direct(kOptFoldcolumn, cstr_as_obj(c"0".as_ptr()), 0, SID_NONE);
        true
    }
}

/// Shows the effects of the `:substitute` command being typed (`'inccommand'`).
unsafe fn show_sub(
    eap: *mut exarg_T,
    old_cusr: pos_T,
    preview_lines: *mut PreviewLines,
    hl_id: c_int,
    cmdpreview_ns: c_int,
    cmdpreview_bufnr: handle_T,
) -> c_int {
    unsafe {
        let save_shm_p = xstrdup(p_shm);
        let lines_size = (*preview_lines).subresults.size;
        let lines_items = (*preview_lines).subresults.items;
        let orig_buf = curbuf;
        let mut cmdpreview_buf: *mut buf_T = ptr::null_mut();

        set_option_direct(kOptShortmess, cstr_as_obj(c"F".as_ptr()), 0, SID_NONE);

        for i in 0..lines_size {
            let curres = *lines_items.add(i);
            if curres.start.lnum >= old_cusr.lnum {
                (*curwin).w_cursor.lnum = curres.start.lnum;
                (*curwin).w_cursor.col = curres.start.col;
                break;
            }
        }

        update_topline(curwin);

        let mut col_width: c_int = 0;
        let preview = (*p_icm == b's' as c_char)
            && ((*eap).line1 != old_cusr.lnum || (*eap).line2 != old_cusr.lnum);

        if preview {
            cmdpreview_buf = buflist_findnr(cmdpreview_bufnr);
            debug_assert!(!cmdpreview_buf.is_null());

            if lines_size > 0 {
                let last_match = *lines_items.add(lines_size - 1);
                let highest_lnum = last_match.start.lnum.max(last_match.end.lnum);
                debug_assert!(highest_lnum > 0);
                col_width = (f64::from(highest_lnum).log10() as c_int) + 1 + 3;
            }
        }

        let mut str: *mut c_char = ptr::null_mut();
        let mut old_line_size: colnr_T = 0;
        let mut line_size: colnr_T = 0;
        let mut linenr_preview: linenr_T = 0;
        let mut linenr_origbuf: linenr_T = 0;
        let mut next_linenr: linenr_T;

        for matchidx in 0..lines_size {
            let m = *lines_items.add(matchidx);

            if !cmdpreview_buf.is_null() {
                let mut p_start = lpos_T {
                    lnum: 0,
                    col: m.start.col,
                };
                let mut p_end = lpos_T {
                    lnum: 0,
                    col: m.end.col,
                };

                buf_ensure_loaded(cmdpreview_buf);

                if m.pre_match == 0 {
                    next_linenr = m.start.lnum;
                } else {
                    next_linenr = m.pre_match;
                }
                if next_linenr == linenr_origbuf {
                    next_linenr += 1;
                    p_start.lnum = linenr_preview;
                    p_end.lnum = linenr_preview;
                }

                while next_linenr <= m.end.lnum {
                    if next_linenr == m.start.lnum {
                        p_start.lnum = linenr_preview + 1;
                    }
                    if next_linenr == m.end.lnum {
                        p_end.lnum = linenr_preview + 1;
                    }
                    let line: *const c_char;
                    if next_linenr == (*orig_buf).b_ml.ml_line_count + 1 {
                        line = c"".as_ptr();
                    } else {
                        line = ml_get_buf(orig_buf, next_linenr);
                        line_size = ml_get_buf_len(orig_buf, next_linenr) + col_width + 1;

                        if line_size > old_line_size {
                            str =
                                xrealloc(str.cast::<c_void>(), line_size as usize).cast::<c_char>();
                            old_line_size = line_size;
                        }
                    }
                    snprintf(
                        str,
                        line_size as usize,
                        c"|%*d| %s".as_ptr(),
                        col_width - 3,
                        next_linenr,
                        line,
                    );
                    if linenr_preview == 0 {
                        ml_replace_buf(cmdpreview_buf, 1, str, true, false);
                    } else {
                        ml_append_buf(cmdpreview_buf, linenr_preview, str, line_size, false);
                    }
                    linenr_preview += 1;
                    next_linenr += 1;
                }
                linenr_origbuf = m.end.lnum;

                bufhl_add_hl_pos_offset(
                    cmdpreview_buf,
                    cmdpreview_ns,
                    hl_id,
                    p_start,
                    p_end,
                    col_width,
                );
            }
            bufhl_add_hl_pos_offset(orig_buf, cmdpreview_ns, hl_id, m.start, m.end, 0);
        }

        xfree(str.cast::<c_void>());

        set_option_direct(kOptShortmess, cstr_as_obj(save_shm_p), 0, SID_NONE);
        xfree(save_shm_p.cast::<c_void>());

        if preview { 2 } else { 1 }
    }
}

/// `:substitute` command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_substitute(eap: *mut exarg_T) {
    unsafe {
        do_sub(eap, profile_zero(), 0, 0);
    }
}

/// `:substitute` command preview callback.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_substitute_preview(
    eap: *mut exarg_T,
    cmdpreview_ns: c_int,
    cmdpreview_bufnr: handle_T,
) -> c_int {
    unsafe {
        if *(*eap).arg != NUL && !ascii_isalnum(*(*eap).arg as c_int) {
            let save_eap = (*eap).arg;
            let retv = do_sub(
                eap,
                profile_setlimit(p_rdt),
                cmdpreview_ns,
                cmdpreview_bufnr,
            );
            (*eap).arg = save_eap;
            return retv;
        }

        0
    }
}

/// Skip over the pattern argument of `":vimgrep /pat/[g][j]"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn skip_vimgrep_pat(
    mut p: *mut c_char,
    s: *mut *mut c_char,
    flags: *mut c_int,
) -> *mut c_char {
    unsafe {
        if vim_isIDc((*p as u8) as c_int) {
            if !s.is_null() {
                *s = p;
            }
            p = skiptowhite(p);
            if !s.is_null() && *p != NUL {
                *p = NUL;
                p = p.add(1);
            }
        } else {
            if !s.is_null() {
                *s = p.add(1);
            }
            let c = (*p as u8) as c_int;
            p = skip_regexp(p.add(1), c, 1);
            if (*p as u8) as c_int != c {
                return ptr::null_mut();
            }

            if !s.is_null() {
                *p = NUL;
            }
            p = p.add(1);

            while *p == b'g' as c_char || *p == b'j' as c_char || *p == b'f' as c_char {
                if !flags.is_null() {
                    if *p == b'g' as c_char {
                        *flags |= VGR_GLOBAL;
                    } else if *p == b'j' as c_char {
                        *flags |= VGR_NOJUMP;
                    } else {
                        *flags |= VGR_FUZZY;
                    }
                }
                p = p.add(1);
            }
        }
        p
    }
}

/// `:oldfiles` (sync) and `:browse oldfiles` (async).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_oldfiles(eap: *mut exarg_T) {
    unsafe {
        nlua_call_excmd(
            c"vim._core.ex_cmd".as_ptr(),
            c"ex_oldfiles".as_ptr(),
            eap,
            &raw const cmdmod,
            ptr::null_mut(),
        );
    }
}
