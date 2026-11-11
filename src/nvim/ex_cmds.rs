#![forbid(unsafe_code)]

//! Functions for command-line Ex commands (`src/nvim/ex_cmds.c`).

use crate::api::private::helpers::{cstr_as_obj, cstr_as_string, cstrn_as_string};
use crate::api::private::validate::NvimString;
use crate::arglist::{
    CCGD_AW, CCGD_EXCMD, CCGD_FORCEIT, CCGD_MULTWIN, ML_EMPTY, check_arg_idx, do_argfile,
};
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_VERBOSE, EVENT_BUFADD, EVENT_BUFENTER, EVENT_BUFFILEPOST, EVENT_BUFFILEPRE,
    EVENT_BUFLEAVE, EVENT_BUFWINENTER, EVENT_SHELLCMDPOST, EVENT_SHELLFILTERPOST, apply_autocmds,
    apply_autocmds_retval, augroup_exists, do_doautocmd,
};
use crate::buffer::{
    BCO_ENTER, BF_CHECK_RO, BF_NEVERLOADED, BF_NEW, BF_NOTEDITED, BF_READERR, BFA_KEEP_UNDO,
    BLN_CURBUF, BLN_LISTED, BLN_NOCURWIN, Bufref, DOBUF_UNLOAD, FileBuf, GetFileResult,
    K_CD_CAUSE_BUFFER, K_SHM_FILEINFO, K_UIMESSAGES, KEYMAP_INIT, MAXLNUM, SEA_DIALOG, SEA_QUIT,
    bt_dontwrite, bt_nofilename, buf_clear_file, buf_ensure_loaded, buf_freeall, buf_hide,
    buf_name_changed, buflist_altfpos, buflist_findfmark, buflist_findname, buflist_findnr,
    buflist_new, bufref_valid, close_buffer, do_autochdir, do_modelines, fileinfo, fname_expand,
    get_winopts, handle_swap_exists, maketitle, modifiable, no_write_message, no_write_message_buf,
    open_buffer, otherfile, set_buflisted, set_bufref, setaltfname, setfname,
};
use crate::buffer_updates::buf_updates_send_changes;
use crate::bufwrite::buf_write;
use crate::change::{
    ML_DEL_MESSAGE, appended_lines, appended_lines_mark, changed_bytes, changed_lines, del_lines,
    deleted_lines, deleted_lines_mark,
};
use crate::channel::channel_job_running;
use crate::charset::{
    EOL_MAC, NUL, STR2NR_BIN, STR2NR_FORCE, STR2NR_HEX, STR2NR_OCT, ascii_isalnum, ascii_isalpha,
    ascii_isdigit, ascii_iswhite, getdigits_int, transchar, transchar_nonprint, vim_is_idc,
    vim_isprintc,
};
use crate::cmdexpand::{Cmdidx, RE_LAST};
use crate::cmdhist::{HIST_SEARCH, add_to_history};
use crate::cursor::{
    Colnr, CursorPos, Linenr, Lpos, check_cursor, check_cursor_col, check_cursor_lnum, check_pos,
    coladvance, equalpos, get_cursor_line_slice, get_cursor_pos_ptr,
};
use crate::decoration::bufhl_add_hl_pos_offset;
use crate::dialog::{VIM_YES, prompt_for_input, vim_dialog_yesno};
use crate::diff::{K_SHELL_OPT_DO_OUT, K_SHELL_OPT_FILTER, diff_buf_add, diff_invalidate};
use crate::digraph::{get_digraph_for_char, keymap_init};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, number_width, redraw_curbuf_later, redraw_later,
    show_cursor_info_later, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{TvFloat, Varnumber};
use crate::eval::vars::{VV_SWAPCOMMAND, get_vim_var_str, set_vim_var_bytes, set_vim_var_string};
use crate::ex_cmds2::{
    autowrite, autowrite_all, buf_write_all, check_changed, check_fname, dialog_changed,
};
use crate::ex_docmd::{
    CMOD_CONFIRM, CMOD_KEEPALT, CMOD_KEEPMARKS, CMOD_KEEPPATTERNS, CMOD_LOCKMARKS, Exarg,
    before_quit_all, check_nextcmd, do_cmdline, ex_may_print, not_exiting,
};
use crate::ex_eval::{aborting, should_abort};
use crate::ex_getln::{curbuf_locked, gotocmdline, text_locked};
use crate::extmark::{Bcount, K_EXTMARK_NOOP, K_EXTMARK_UNDO, extmark_move_region, extmark_splice};
use crate::fileio::{
    READ_FILTER, READ_KEEP_UNDO, READ_NOWINENTER, buf_check_timestamp, readfile, set_file_options,
    set_forced_fenc, vim_tempname, write_lnum_adjust,
};
use crate::fold::{fold_move_range, fold_update, fold_update_all, has_any_folding};
use crate::globals::*;
use crate::help::prepare_help_buffer;
use crate::highlight_group::syn_check_group;
use crate::indent::{get_indent, get_indent_lnum, set_indent};
use crate::input::{redo_append_lit, redo_append_str};
use crate::insert::beginline;
use crate::keycodes::{CAR, ESC, NL, TAB};
use crate::lua::executor::nlua_call_excmd;
use crate::main_init::getout;
use crate::mark::{mark_adjust, mark_adjust_nofold, set_last_cursor, setpcmark};
use crate::regexp::{
    MAXCOL, RE_MAGIC, REGSUB_BACKSLASH, REGSUB_COPY, REGSUB_MAGIC, Regmatch, Regmmatch,
    re_multiline, regtilde, skip_regexp, skip_regexp_err, skip_regexp_ex, vim_regcomp, vim_regexec,
    vim_regexec_multi, vim_regsub_multi,
};
use crate::runtime::SID_NONE;
use crate::window::{
    WinInfoInfo, check_can_set_curbuf_forceit, check_lnums, curwin_init, update_cwd, win_enter,
    win_split, win_valid,
};

use crate::mbyte::{
    utf_char2bytes, utf_iscomposing_first, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::memline::{
    makeswapname, ml_append, ml_append_buf, ml_clearmarked, ml_delete, ml_delete_flags,
    ml_find_line_or_offset, ml_firstmarked, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len,
    ml_replace, ml_replace_buf, ml_setmarked,
};
use crate::memory::bytes_to_cstring;
use crate::message::{
    emsg, message_filtered, messaging, msg, msg_check_for_delay, msg_clr_eos, msg_end,
    msg_ext_no_fast, msg_ext_set_kind, msg_multiline, msg_outtrans, msg_prt_line, msg_putchar,
    msg_puts, msg_puts_hl, msg_sb_eol, msg_start, msgmore, semsg, set_keep_msg, smsg, wait_return,
};
use crate::mouse::setmouse;
use crate::r#move::{
    changed_line_abv_curs, changed_window_setting, do_check_cursorbind, invalidate_botline_win,
    scrolldown_clamp, scrollup_clamp, update_topline, validate_cursor,
};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::normal::reset_visual;
use crate::ops::do_join;
use crate::option::{
    OptInt, buf_copy_options, copy_option_part, get_fileformat, set_option_direct, shortmess,
};
use crate::option_vars::*;
use crate::optionstr::OptIndex;
use crate::os::fs::{
    CASE_INSENSITIVE_FILENAME, MAXPATHL, os_file_is_writable, os_file_mkdir, os_isdir,
    os_path_exists, os_remove,
};
use crate::os::input::{fast_breakcheck, line_breakcheck, os_breakcheck};
use crate::os::shell::call_shell;
use crate::os::time::{os_time, time_t};
use crate::path::{fix_fname, invocation_path_tail, path_fix_case};
use crate::plines::{linetabsize_col, plines_m_win_fill};
use crate::profile::{Proftime, profile_passed_limit, profile_setlimit, profile_zero};
use crate::search::{get_search_pat, last_search_pat, save_re_pat, search_regcomp};
use crate::shada::SubReplacementString;
use crate::spell::parse_spelllang;
use crate::state::{MODE_CMDLINE, MODE_INSERT, MODE_LANGMAP};
use crate::strings::{
    concat_str, format_to_buf, format_to_buf_add, format_to_buf_safelen, gettext, ngettext,
    strcoll, vim_strsave_escaped,
};
use crate::terminal::{terminal_check_size, terminal_running};
use crate::ui::{ui_cursor_goto, ui_cursor_shape, ui_has};
use crate::undo::{
    u_inssub, u_save, u_save_cursor, u_savecommon, u_savedel, u_savesub, u_sync, u_unchanged,
};
use crate::winfloat::{win_float_anchor_preview, win_float_special, win_float_update_preview};
use std::ffi::CStr;
use std::ptr;

#[cfg(not(unix))]
use crate::charset::rem_backslash;
#[cfg(unix)]
use crate::os::fs::{NODE_OTHER, os_nodetype};

use crate::api::vim::HLF_N;
use crate::eval::funcs::{RE_SEARCH, VIM_QUESTION};

pub(crate) const CTRL_C_CONST: i32 = 3;
pub(crate) const CTRL_E: i32 = 5;
pub(crate) const CTRL_Y: i32 = 25;

pub(crate) const DIALOG_MSG_SIZE: usize = 1000;

pub(crate) const BL_WHITE: i32 = 1;
pub(crate) const BL_SOL: i32 = 2;
pub(crate) const BL_FIX: i32 = 4;

pub(crate) const RE_SUBST: i32 = 1;
pub(crate) const RE_BOTH: i32 = 2;
pub(crate) const SEARCH_HIS: i32 = 0x20;

pub(crate) const EXFLAG_LIST: i32 = 0x01;
pub(crate) const EXFLAG_NR: i32 = 0x02;
pub(crate) const EXFLAG_PRINT: i32 = 0x04;

pub(crate) const K_SHELL_OPT_READ: i32 = 16;
pub(crate) const K_SHELL_OPT_WRITE: i32 = 32;

pub(crate) const HLF_R: i32 = 18;

pub(crate) const K_SHM_OVERALL: i32 = b'O' as i32;

const K_CPO_ALTWRITE: i32 = b'A' as i32;
const K_CPO_OVERNEW: i32 = b'O' as i32;
const K_CPO_REMMARK: i32 = b'R' as i32;
pub(crate) const K_CPO_UNDO: i32 = b'u' as i32;

pub(crate) const OPT_WINONLY: i32 = 0x08;
pub(crate) const B_IMODE_LMAP: OptInt = 1;
pub(crate) const K_WIN_PREVIEW: i32 = 2;

pub(crate) const K_OPT_FOLDCOLUMN: OptIndex = 102;
pub(crate) const K_OPT_INCCOMMAND: OptIndex = 145;
pub(crate) const K_OPT_SHORTMESS: OptIndex = 271;

pub const ECMD_HIDE: i32 = 0x01;
pub const ECMD_SET_HELP: i32 = 0x02;
pub const ECMD_OLDBUF: i32 = 0x04;
pub const ECMD_FORCEIT: i32 = 0x08;
pub const ECMD_ADDBUF: i32 = 0x10;
pub const ECMD_ALTBUF: i32 = 0x20;
pub const ECMD_NOWINENTER: i32 = 0x40;

pub const ECMD_LASTL: Linenr = 0;
pub const ECMD_LAST: Linenr = -1;
pub const ECMD_ONE: Linenr = 1;

pub const VGR_GLOBAL: i32 = 1;
pub const VGR_NOJUMP: i32 = 2;
pub const VGR_FUZZY: i32 = 4;

const E_NON_NUMERIC_ARGUMENT_TO_Z: &CStr = c"E144: Non-numeric argument to :z";

/// Case matching style to use for `:substitute`.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SubIgnoreType {
    SubHonorOptions = 0,
    SubIgnoreCase = 1,
    SubMatchCase = 2,
}

/// Flags kept between calls to `:substitute`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Subflags {
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
    start: Lpos,
    end: Lpos,
    pre_match: Linenr,
}

/// Collected results of a substitution for showing them in the preview window.
struct PreviewLines {
    subresults: Vec<SubResult>,
    lines_needed: Linenr,
}

#[derive(Clone, Copy)]
enum SortKey {
    Line {
        start_col_nr: Varnumber,
        end_col_nr: Varnumber,
    },
    Num {
        value: Varnumber,
        is_number: bool,
    },
    Float(TvFloat),
}

#[derive(Clone, Copy)]
struct Sorti {
    lnum: Linenr,
    st_u: SortKey,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LineData {
    start_col: i32,
    start: Lpos,
    end: Lpos,
    matchcols: i32,
    matchbytes: Bcount,
    subcols: i32,
    subbytes: Bcount,
    lnum_before: Linenr,
    lnum_after: Linenr,
}

struct ExCmdsState {
    sort_lc: std::cell::Cell<bool>,
    sort_ic: std::cell::Cell<bool>,
    sort_nr: std::cell::Cell<i32>,
    sort_rx: std::cell::Cell<bool>,
    sort_flt: std::cell::Cell<bool>,
    sort_abort: std::cell::Cell<bool>,
    global_need_msg_kind: std::cell::Cell<bool>,
    append_indent: std::cell::Cell<i32>,
    global_need_beginline: std::cell::Cell<bool>,
    subflags: std::cell::Cell<Subflags>,
    pre_hl_id: std::cell::Cell<i32>,
}

impl ExCmdsState {
    const fn new() -> Self {
        Self {
            sort_lc: std::cell::Cell::new(false),
            sort_ic: std::cell::Cell::new(false),
            sort_nr: std::cell::Cell::new(0),
            sort_rx: std::cell::Cell::new(false),
            sort_flt: std::cell::Cell::new(false),
            sort_abort: std::cell::Cell::new(false),
            global_need_msg_kind: std::cell::Cell::new(false),
            append_indent: std::cell::Cell::new(0),
            global_need_beginline: std::cell::Cell::new(false),
            subflags: std::cell::Cell::new(Subflags {
                do_all: false,
                do_ask: false,
                do_count: false,
                do_error: true,
                do_print: false,
                do_list: false,
                do_number: false,
                do_ic: SubIgnoreType::SubHonorOptions,
            }),
            pre_hl_id: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static EXCMDS_STATE: ExCmdsState = const { ExCmdsState::new() };
    static EXCMDS_PREVCMD: std::cell::RefCell<Option<std::ffi::CString>> =
        const { std::cell::RefCell::new(None) };
    static EXCMDS_OLD_SUB: std::cell::RefCell<SubReplacementString> =
        const {
            std::cell::RefCell::new(SubReplacementString {
                sub: None,
                timestamp: 0,
                additional_data: None,
            })
        };
}

use crate::buffer::bt_dontwrite_msg;
use crate::undo::{buf_is_changed_fn, curbuf_is_changed_fn};

/// `":ascii"` and `"ga"` implementation.
pub fn do_ascii(_eap: &mut Exarg) {
    let data = get_cursor_pos_ptr();
    let data_bytes = data.to_bytes();
    let len = utfc_ptr2len(data_bytes) as usize;

    if len == 0 {
        msg(c"NUL", 0);
        return;
    }

    let mut need_clear = true;
    msg_sb_eol();
    msg_start();

    let mut c = utf_ptr2char(data_bytes);
    let mut off: usize = 0;

    if c < 0x80 {
        if c == NL {
            c = NUL as i32;
        }
        let cval = if c == CAR && get_fileformat(curbuf_mut()) == EOL_MAC {
            NL
        } else {
            c
        };
        let mut buf1: [u8; 20] = [0; 20];
        if vim_isprintc(c) && (c < b' ' as i32 || c > b'~' as i32) {
            let mut buf3: [u8; 7] = [0; 7];
            transchar_nonprint(Some(crate::globals::curbuf_mut()), &mut buf3, c);
            format_to_buf!(&mut buf1, c"  <%s>", &buf3);
        } else {
            buf1[0] = 0;
        }
        let buf2: [u8; 20] = [0; 20];

        if let Some(dig) = get_digraph_for_char(cval) {
            format_to_buf!(
                &mut g_st().iobuff,
                gettext(c"<%s>%s%s  %d,  Hex %02x,  Oct %03o, Digr %s"),
                transchar(c),
                &buf1,
                &buf2,
                cval,
                cval,
                cval,
                &dig,
            );
        } else {
            format_to_buf!(
                &mut g_st().iobuff,
                gettext(c"<%s>%s%s  %d,  Hex %02x,  Octal %03o"),
                transchar(c),
                &buf1,
                &buf2,
                cval,
                cval,
                cval,
            );
        }

        msg_multiline(
            cstr_as_string(Some(g_st().iobuff_cstr())),
            0,
            true,
            false,
            &mut need_clear,
        );

        off += utf_ptr2len(data_bytes) as usize;
    }

    while off < len {
        c = utf_ptr2char(&data_bytes[off..]);

        let mut iobuff_len: usize = 0;
        if off > 0 {
            g_st().iobuff_bytes_mut()[iobuff_len] = b' ';
            iobuff_len += 1;
        }
        g_st().iobuff_bytes_mut()[iobuff_len] = b'<';
        iobuff_len += 1;
        if utf_iscomposing_first(c) {
            g_st().iobuff_bytes_mut()[iobuff_len] = b' ';
            iobuff_len += 1;
        }
        iobuff_len += utf_char2bytes(
            c,
            &mut g_st().iobuff_bytes_mut()[iobuff_len..iobuff_len + 6],
        ) as usize;

        if let Some(dig) = get_digraph_for_char(c) {
            let fmt = if c < 0x10000 {
                gettext(c"> %d, Hex %04x, Oct %o, Digr %s")
            } else {
                gettext(c"> %d, Hex %08x, Oct %o, Digr %s")
            };
            format_to_buf!(&mut g_st().iobuff[iobuff_len..], fmt, c, c, c, &dig,);
        } else {
            let fmt = if c < 0x10000 {
                gettext(c"> %d, Hex %04x, Octal %o")
            } else {
                gettext(c"> %d, Hex %08x, Octal %o")
            };
            format_to_buf!(&mut g_st().iobuff[iobuff_len..], fmt, c, c, c);
        }

        msg_multiline(
            cstr_as_string(Some(g_st().iobuff_cstr())),
            0,
            true,
            false,
            &mut need_clear,
        );

        off += utf_ptr2len(&data_bytes[off..]) as usize;
    }

    if need_clear {
        msg_clr_eos();
    }
    msg_end();
}

/// `":left"`, `":center"` and `":right"`: align text.
pub fn ex_align(eap: &mut Exarg) {
    let mut indent: i32 = 0;
    let mut new_indent: i32;

    if curwin_mut().w_onebuf_opt.wo_rl {
        if eap.cmdidx == Cmdidx::CmdRight {
            eap.cmdidx = Cmdidx::CmdLeft;
        } else if eap.cmdidx == Cmdidx::CmdLeft {
            eap.cmdidx = Cmdidx::CmdRight;
        }
    }

    let mut width = crate::charset::parse_i64_prefix(eap.arg.as_bytes()) as i32;
    let save_curpos = curwin_mut().w_cursor;
    if eap.cmdidx == Cmdidx::CmdLeft {
        if width >= 0 {
            indent = width;
        }
    } else {
        if width <= 0 {
            width = curbuf_mut().b_p_tw as i32;
        }
        if width == 0 && curbuf_mut().b_p_wm > 0 {
            width = curwin_mut().w_view_width - curbuf_mut().b_p_wm as i32;
        }
        if width <= 0 {
            width = 80;
        }
    }

    if !u_save(eap.line1 - 1, eap.line2 + 1) {
        return;
    }

    curwin_mut().w_cursor.lnum = eap.line1;
    while curwin_mut().w_cursor.lnum <= eap.line2 {
        if eap.cmdidx == Cmdidx::CmdLeft {
            new_indent = indent;
        } else {
            let mut has_tab: i32 = 0;
            let len =
                linelen((eap.cmdidx == Cmdidx::CmdRight).then_some(&mut has_tab)) - get_indent();

            if len <= 0 {
                curwin_mut().w_cursor.lnum += 1;
                continue;
            }

            if eap.cmdidx == Cmdidx::CmdCenter {
                new_indent = (width - len) / 2;
            } else {
                new_indent = width - len;

                if has_tab != 0 {
                    while new_indent > 0 {
                        set_indent(new_indent, 0);
                        if linelen(None) <= width {
                            loop {
                                new_indent += 1;
                                set_indent(new_indent, 0);
                                if linelen(None) > width {
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
        curwin_mut().w_cursor.lnum += 1;
    }
    changed_lines(curbuf_mut(), eap.line1, 0, eap.line2 + 1, 0, true);
    curwin_mut().w_cursor = save_curpos;
    beginline(BL_WHITE | BL_FIX);
}

/// @return the length of the current line, excluding trailing white space.
fn linelen(has_tab: Option<&mut i32>) -> i32 {
    let line_bytes = get_cursor_line_slice();
    if line_bytes.is_empty() {
        return 0;
    }
    let first_off = crate::charset::skipwhite_len(line_bytes);
    let mut last_off = line_bytes.len();
    while last_off > first_off && ascii_iswhite(i32::from(line_bytes[last_off - 1])) {
        last_off -= 1;
    }
    let trimmed = crate::memory::bytes_to_cstring(&line_bytes[..last_off]);
    let len = linetabsize_col(0, &trimmed);
    if let Some(ht) = has_tab {
        *ht = i32::from(line_bytes[first_off..last_off].contains(&(TAB as u8)));
    }

    len
}

fn string_compare(s1: &CStr, s2: &CStr) -> i32 {
    if EXCMDS_STATE.with(|s| s.sort_lc.get()) {
        return strcoll(s1, s2);
    }
    if EXCMDS_STATE.with(|s| s.sort_ic.get()) {
        s1.to_bytes()
            .iter()
            .map(u8::to_ascii_lowercase)
            .cmp(s2.to_bytes().iter().map(u8::to_ascii_lowercase)) as i32
    } else {
        s1.to_bytes().cmp(s2.to_bytes()) as i32
    }
}

fn sort_compare(l1: &Sorti, l2: &Sorti, sortbuf1: &mut [u8], sortbuf2: &mut [u8]) -> i32 {
    let l1 = *l1;
    let l2 = *l2;
    let result: i32;

    if EXCMDS_STATE.with(|s| s.sort_abort.get()) {
        return 0;
    }
    fast_breakcheck();
    if g_st().got_int {
        EXCMDS_STATE.with(|s| s.sort_abort.set(true));
    }

    match (l1.st_u, l2.st_u) {
        (
            SortKey::Num {
                value: v1,
                is_number: is_num1,
            },
            SortKey::Num {
                value: v2,
                is_number: is_num2,
            },
        ) => {
            if is_num1 != is_num2 {
                result = if is_num1 && !is_num2 { 1 } else { -1 };
            } else if v1 == v2 {
                result = 0;
            } else if v1 > v2 {
                result = 1;
            } else {
                result = -1;
            }
        }
        (SortKey::Float(f1), SortKey::Float(f2)) => {
            if f1 == f2 {
                result = 0;
            } else if f1 > f2 {
                result = 1;
            } else {
                result = -1;
            }
        }
        (
            SortKey::Line {
                start_col_nr: s1,
                end_col_nr: e1,
            },
            SortKey::Line {
                start_col_nr: s2,
                end_col_nr: e2,
            },
        ) => {
            let len1 = (e1 - s1) as usize;
            let line1 = &crate::memline::ml_get_slice(l1.lnum)[s1 as usize..s1 as usize + len1];
            sortbuf1[..len1].copy_from_slice(line1);
            sortbuf1[len1] = 0;

            let len2 = (e2 - s2) as usize;
            let line2 = &crate::memline::ml_get_slice(l2.lnum)[s2 as usize..s2 as usize + len2];
            sortbuf2[..len2].copy_from_slice(line2);
            sortbuf2[len2] = 0;

            result = string_compare(
                crate::memory::bytes_as_cstr(sortbuf1),
                crate::memory::bytes_as_cstr(sortbuf2),
            );
        }
        _ => unreachable!(),
    }

    if result == 0 {
        return l1.lnum - l2.lnum;
    }
    result
}

/// `":sort"`.
pub fn ex_sort(eap: &mut Exarg) {
    let mut regmatch: Regmatch = Regmatch::new();
    let mut maxlen: i32 = 0;
    let mut count = (eap.line2 - eap.line1) as usize + 1;
    let mut i: usize = 0;
    let mut unique = false;
    let mut sort_what: i32 = 0;

    if count <= 1 {
        return;
    }

    if !u_save(eap.line1 - 1, eap.line2 + 1) {
        return;
    }
    regmatch.free_prog();
    let mut nrs_vec: Vec<Sorti> = Vec::with_capacity(count);

    EXCMDS_STATE.with(|s| s.sort_abort.set(false));
    EXCMDS_STATE.with(|s| s.sort_ic.set(false));
    EXCMDS_STATE.with(|s| s.sort_lc.set(false));
    EXCMDS_STATE.with(|s| s.sort_rx.set(false));
    EXCMDS_STATE.with(|s| s.sort_nr.set(0));
    EXCMDS_STATE.with(|s| s.sort_flt.set(false));
    let mut format_found: usize = 0;
    let mut change_occurred = false;

    'sortend: {
        let mut p_idx = 0usize;
        loop {
            let pb = eap.arg_cstr_buf_mut().get(p_idx).copied().unwrap_or(0);
            if pb == 0 {
                break;
            }
            if ascii_iswhite(pb) {
                // Skip
            } else if pb == b'i' {
                EXCMDS_STATE.with(|s| s.sort_ic.set(true));
            } else if pb == b'l' {
                EXCMDS_STATE.with(|s| s.sort_lc.set(true));
            } else if pb == b'r' {
                EXCMDS_STATE.with(|s| s.sort_rx.set(true));
            } else if pb == b'n' {
                EXCMDS_STATE.with(|s| s.sort_nr.set(1));
                format_found += 1;
            } else if pb == b'f' {
                EXCMDS_STATE.with(|s| s.sort_flt.set(true));
                format_found += 1;
            } else if pb == b'b' {
                sort_what = STR2NR_BIN + STR2NR_FORCE;
                format_found += 1;
            } else if pb == b'o' {
                sort_what = STR2NR_OCT + STR2NR_FORCE;
                format_found += 1;
            } else if pb == b'x' {
                sort_what = STR2NR_HEX + STR2NR_FORCE;
                format_found += 1;
            } else if pb == b'u' {
                unique = true;
            } else if pb == b'"' {
                break;
            } else if let Some(nc) = check_nextcmd(crate::memory::bytes_as_cstr(
                &eap.arg_cstr_buf_mut()[p_idx..],
            )) {
                eap.nextcmd = crate::strings::OwnedSlice::from_cstr(nc);
                break;
            } else if !ascii_isalpha(pb) && regmatch.regprog.is_null() {
                let s_off = {
                    let p_after =
                        crate::memory::bytes_as_cstr(&eap.arg_cstr_buf_mut()[p_idx + 1..]);
                    let Some(s_cstr) = skip_regexp_err(p_after, pb as i32, 1) else {
                        break 'sortend;
                    };
                    p_idx + 1 + ((s_cstr.as_ptr() as usize) - (p_after.as_ptr() as usize))
                };

                eap.arg_cstr_buf_mut()[s_off] = 0;

                if s_off == p_idx + 1 {
                    let Some(last_pat) = last_search_pat() else {
                        emsg(gettext(E_NOPREVRE));
                        break 'sortend;
                    };
                    regmatch.set_prog(vim_regcomp(&last_pat, RE_MAGIC));
                } else {
                    let pat_cstr =
                        crate::memory::bytes_as_cstr(&eap.arg_cstr_buf_mut()[p_idx + 1..]);
                    regmatch.set_prog(vim_regcomp(pat_cstr, RE_MAGIC));
                }
                if regmatch.regprog.is_null() {
                    break 'sortend;
                }
                p_idx = s_off;
                regmatch.rm_ic = opt_vars().p_ic;
            } else {
                let err_cstr = crate::memory::bytes_as_cstr(&eap.arg_cstr_buf_mut()[p_idx..]);
                semsg!(gettext(E_INVARG2), err_cstr);
                break 'sortend;
            }
            p_idx += 1;
        }

        if format_found > 1 {
            emsg(gettext(E_INVARG));
            break 'sortend;
        }

        EXCMDS_STATE.with(|s| s.sort_nr.set(s.sort_nr.get() | sort_what));

        for lnum in eap.line1..=eap.line2 {
            let s_cstr = ml_get(lnum);
            let len = ml_get_len(lnum);
            maxlen = maxlen.max(len);

            let mut start_col: Colnr = 0;
            let mut end_col: Colnr = len;
            if !regmatch.regprog.is_null() && vim_regexec(&mut regmatch, s_cstr, 0) {
                if EXCMDS_STATE.with(|s| s.sort_rx.get()) {
                    start_col = regmatch.start_offset(0, s_cstr) as Colnr;
                    end_col = regmatch.end_offset(0, s_cstr) as Colnr;
                } else {
                    start_col = regmatch.end_offset(0, s_cstr) as Colnr;
                }
            } else if !regmatch.regprog.is_null() {
                end_col = 0;
            }

            let st_u = if EXCMDS_STATE.with(|s| s.sort_nr.get()) != 0
                || EXCMDS_STATE.with(|s| s.sort_flt.get())
            {
                let line_slice = crate::memline::ml_get_slice(lnum);
                let sub_slice = &line_slice[(start_col.max(0) as usize).min(line_slice.len())
                    ..(end_col.max(0) as usize).min(line_slice.len())];
                if EXCMDS_STATE.with(|s| s.sort_nr.get()) != 0 {
                    let rem = if (sort_what & STR2NR_HEX) != 0 {
                        crate::charset::skiptohex_slice(sub_slice)
                    } else if (sort_what & STR2NR_BIN) != 0 {
                        crate::charset::skiptobin_slice(sub_slice)
                    } else {
                        crate::charset::skiptodigit_slice(sub_slice)
                    };
                    let off = sub_slice.len() - rem.len();
                    let rem = if off > 0 && sub_slice[off - 1] == b'-' {
                        &sub_slice[off - 1..]
                    } else {
                        rem
                    };
                    if rem.is_empty() {
                        SortKey::Num {
                            is_number: false,
                            value: 0,
                        }
                    } else {
                        let mut val: Varnumber = 0;
                        crate::charset::vim_slice2nr(
                            rem,
                            None,
                            None,
                            sort_what,
                            Some(&mut val),
                            None,
                            false,
                            None,
                        );
                        SortKey::Num {
                            is_number: true,
                            value: val,
                        }
                    }
                } else {
                    let mut rem = crate::charset::skipwhite_slice(sub_slice);
                    if let Some(after_plus) = rem.strip_prefix(b"+") {
                        rem = crate::charset::skipwhite_slice(after_plus);
                    }

                    if rem.is_empty() {
                        SortKey::Float(-f64::MAX)
                    } else {
                        let (val, _) = crate::strings::strtod_slice(rem);
                        SortKey::Float(val)
                    }
                }
            } else {
                SortKey::Line {
                    start_col_nr: start_col as Varnumber,
                    end_col_nr: end_col as Varnumber,
                }
            };

            nrs_vec.push(Sorti { lnum, st_u });

            if !regmatch.regprog.is_null() {
                fast_breakcheck();
            }
            if g_st().got_int {
                break 'sortend;
            }
        }

        let mut sortbuf1_vec = vec![0u8; maxlen as usize + 1];
        let mut sortbuf2_vec = vec![0u8; maxlen as usize + 1];

        nrs_vec.sort_by(|a, b| sort_compare(a, b, &mut sortbuf1_vec, &mut sortbuf2_vec).cmp(&0));

        if EXCMDS_STATE.with(|s| s.sort_abort.get()) {
            break 'sortend;
        }

        let mut old_count: Bcount = 0;
        let mut new_count: Bcount = 0;

        let mut lnum = eap.line2;
        while i < count {
            let get_lnum = nrs_vec[if eap.forceit { count - i - 1 } else { i }].lnum;

            if get_lnum + (count as Linenr - 1) != lnum {
                change_occurred = true;
            }

            let s_cstr = ml_get(get_lnum);
            let bytelen: Colnr = ml_get_len(get_lnum) + 1;
            old_count += bytelen as Bcount;
            if !unique
                || i == 0
                || string_compare(s_cstr, crate::memory::bytes_as_cstr(&sortbuf1_vec)) != 0
            {
                let s_bytes = s_cstr.to_bytes_with_nul();
                sortbuf1_vec[..s_bytes.len()].copy_from_slice(s_bytes);
                if !ml_append(
                    lnum,
                    crate::memory::bytes_until_nul(&sortbuf1_vec),
                    0,
                    false,
                ) {
                    break;
                }
                lnum += 1;
                new_count += bytelen as Bcount;
            }
            fast_breakcheck();
            if g_st().got_int {
                break 'sortend;
            }
            i += 1;
        }

        if i == count {
            for _ in 0..count {
                ml_delete(eap.line1);
            }
        } else {
            count = 0;
        }

        let deleted = count as Linenr - (lnum - eap.line2);
        if deleted > 0 {
            mark_adjust(
                eap.line2 - deleted,
                eap.line2,
                MAXLNUM,
                -deleted,
                K_EXTMARK_NOOP,
            );
            msgmore(-deleted);
        } else if deleted < 0 {
            mark_adjust(eap.line2, MAXLNUM, -deleted, 0, K_EXTMARK_NOOP);
        }

        if change_occurred || deleted != 0 {
            extmark_splice(
                curbuf_mut(),
                eap.line1 - 1,
                0,
                count as i32,
                0,
                old_count,
                lnum - eap.line2,
                0,
                new_count,
                K_EXTMARK_UNDO,
            );
            changed_lines(curbuf_mut(), eap.line1, 0, eap.line2 + 1, -deleted, true);
        }

        curwin_mut().w_cursor.lnum = eap.line1;
        beginline(BL_WHITE | BL_FIX);
    }

    // sortend:
    regmatch.free_prog();
    if g_st().got_int {
        emsg(gettext(E_INTERR));
    }
}

/// `":uniq"`.
pub fn ex_uniq(eap: &mut Exarg) {
    let mut regmatch: Regmatch = Regmatch::new();
    let mut maxlen: i32 = 0;
    let mut count = eap.line2 - eap.line1 + 1;
    let mut keep_only_unique = false;
    let keep_only_not_unique = eap.forceit;
    let mut deleted: Linenr = 0;

    if count <= 1 {
        return;
    }

    if !u_save(eap.line1 - 1, eap.line2 + 1) {
        return;
    }
    regmatch.free_prog();

    EXCMDS_STATE.with(|s| s.sort_abort.set(false));
    EXCMDS_STATE.with(|s| s.sort_ic.set(false));
    EXCMDS_STATE.with(|s| s.sort_lc.set(false));
    EXCMDS_STATE.with(|s| s.sort_rx.set(false));
    EXCMDS_STATE.with(|s| s.sort_nr.set(0));
    EXCMDS_STATE.with(|s| s.sort_flt.set(false));
    let mut change_occurred = false;

    'uniqend: {
        let mut p_idx = 0usize;
        loop {
            let b0 = eap.arg_cstr_buf_mut().get(p_idx).copied().unwrap_or(0);
            if b0 == 0 {
                break;
            }
            if ascii_iswhite(b0 as i32) {
                // Skip
            } else if b0 == b'i' {
                EXCMDS_STATE.with(|s| s.sort_ic.set(true));
            } else if b0 == b'l' {
                EXCMDS_STATE.with(|s| s.sort_lc.set(true));
            } else if b0 == b'r' {
                EXCMDS_STATE.with(|s| s.sort_rx.set(true));
            } else if b0 == b'u' {
                if !keep_only_not_unique {
                    keep_only_unique = true;
                }
            } else if b0 == b'"' {
                break;
            } else if eap.nextcmd.is_null()
                && let Some(nc) = check_nextcmd(crate::memory::bytes_as_cstr(
                    &eap.arg_cstr_buf_mut()[p_idx..],
                ))
            {
                eap.nextcmd = crate::strings::OwnedSlice::from_cstr(nc);
                break;
            } else if !b0.is_ascii_alphabetic() && regmatch.regprog.is_null() {
                let s_off = {
                    let p_after =
                        crate::memory::bytes_as_cstr(&eap.arg_cstr_buf_mut()[p_idx + 1..]);
                    let Some(s_cstr) = skip_regexp_err(p_after, b0 as i32, 1) else {
                        break 'uniqend;
                    };
                    p_idx + 1 + ((s_cstr.as_ptr() as usize) - (p_after.as_ptr() as usize))
                };

                eap.arg_cstr_buf_mut()[s_off] = 0;

                if s_off == p_idx + 1 {
                    let Some(last_pat) = last_search_pat() else {
                        emsg(gettext(E_NOPREVRE));
                        break 'uniqend;
                    };
                    regmatch.set_prog(vim_regcomp(&last_pat, RE_MAGIC));
                } else {
                    let pat_cstr =
                        crate::memory::bytes_as_cstr(&eap.arg_cstr_buf_mut()[p_idx + 1..]);
                    regmatch.set_prog(vim_regcomp(pat_cstr, RE_MAGIC));
                }
                if regmatch.regprog.is_null() {
                    break 'uniqend;
                }
                p_idx = s_off;
                regmatch.rm_ic = opt_vars().p_ic;
            } else {
                let err_cstr = crate::memory::bytes_as_cstr(&eap.arg_cstr_buf_mut()[p_idx..]);
                semsg!(gettext(E_INVARG2), err_cstr);
                break 'uniqend;
            }
            p_idx += 1;
        }

        for lnum in eap.line1..=eap.line2 {
            let len = ml_get_len(lnum);
            if maxlen < len {
                maxlen = len;
            }

            if g_st().got_int {
                break 'uniqend;
            }
        }

        let mut sortbuf1_vec = vec![0u8; maxlen as usize + 1];
        let mut sortbuf2_vec = vec![0u8; maxlen as usize + 1];

        let mut match_continue = false;
        let mut next_is_unmatch = false;
        let mut done_lnum = eap.line1 - 1;
        let mut i: Linenr = 0;
        while i < count {
            let get_lnum = eap.line1 + i;

            let s_cstr = ml_get(get_lnum);
            let s_bytes = s_cstr.to_bytes();
            let len = ml_get_len(get_lnum);

            let mut start_col: Colnr = 0;
            let mut end_col: Colnr = len;
            if !regmatch.regprog.is_null() && vim_regexec(&mut regmatch, s_cstr, 0) {
                if EXCMDS_STATE.with(|s| s.sort_rx.get()) {
                    start_col = regmatch.start_offset(0, s_cstr) as Colnr;
                    end_col = regmatch.end_offset(0, s_cstr) as Colnr;
                } else {
                    start_col = regmatch.end_offset(0, s_cstr) as Colnr;
                }
            } else if !regmatch.regprog.is_null() {
                end_col = 0;
            }
            let sc_u = (start_col.max(0) as usize).min(s_bytes.len());
            let ec_u = (end_col.max(start_col) as usize).min(s_bytes.len());
            let sub_len = ec_u - sc_u;
            sortbuf2_vec[..sub_len].copy_from_slice(&s_bytes[sc_u..ec_u]);
            sortbuf2_vec[sub_len] = 0;

            let s_sub_cstr = crate::memory::bytes_as_cstr(&sortbuf2_vec[..=sub_len]);
            let mut is_match = if i > 0 {
                string_compare(s_sub_cstr, crate::memory::bytes_as_cstr(&sortbuf1_vec)) == 0
            } else {
                false
            };
            let mut delete_lnum: Linenr = 0;
            if next_is_unmatch {
                is_match = false;
                next_is_unmatch = false;
            }

            if !keep_only_unique && !keep_only_not_unique {
                if is_match {
                    delete_lnum = get_lnum;
                } else {
                    let s_bytes = s_sub_cstr.to_bytes_with_nul();
                    sortbuf1_vec[..s_bytes.len()].copy_from_slice(s_bytes);
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
                    let s_bytes = s_sub_cstr.to_bytes_with_nul();
                    sortbuf1_vec[..s_bytes.len()].copy_from_slice(s_bytes);
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
                let s_bytes = s_sub_cstr.to_bytes_with_nul();
                sortbuf1_vec[..s_bytes.len()].copy_from_slice(s_bytes);
            }

            if delete_lnum > 0 {
                ml_delete(delete_lnum);
                i -= get_lnum - delete_lnum + 1;
                count -= 1;
                deleted += 1;
                change_occurred = true;
            }

            fast_breakcheck();
            if g_st().got_int {
                break 'uniqend;
            }
            i += 1;
        }

        mark_adjust(
            eap.line2 - deleted,
            eap.line2,
            MAXLNUM,
            -deleted,
            if change_occurred {
                K_EXTMARK_UNDO
            } else {
                K_EXTMARK_NOOP
            },
        );
        msgmore(-deleted);

        if change_occurred {
            changed_lines(curbuf_mut(), eap.line1, 0, eap.line2 + 1, -deleted, true);
        }

        curwin_mut().w_cursor.lnum = eap.line1;
        beginline(BL_WHITE | BL_FIX);
    }

    // uniqend:
    regmatch.free_prog();
    if g_st().got_int {
        emsg(gettext(E_INTERR));
    }
}

/// `:move` command - move lines `line1`-`line2` to line `dest`.
pub fn do_move(line1: Linenr, line2: Linenr, mut dest: Linenr) -> bool {
    if dest >= line1 && dest < line2 {
        emsg(gettext(c"E134: Cannot move a range of lines into itself"));
        return false;
    }

    if dest == line1 - 1 || dest == line2 {
        curwin_mut().w_cursor.lnum = if dest >= line1 {
            dest
        } else {
            dest + (line2 - line1) + 1
        };
        return true;
    }

    let start_byte = ml_find_line_or_offset(curbuf_mut(), line1, None, true) as Bcount;
    let end_byte = ml_find_line_or_offset(curbuf_mut(), line2 + 1, None, true) as Bcount;
    let extent_byte = end_byte - start_byte;
    let dest_byte = ml_find_line_or_offset(curbuf_mut(), dest + 1, None, true) as Bcount;

    let num_lines = line2 - line1 + 1;

    if !u_save(dest, dest + 1) {
        return false;
    }

    let mut extra: Linenr = 0;
    for l in line1..=line2 {
        let str_cstr = bytes_to_cstring(crate::memline::ml_get_slice(l + extra));
        ml_append(dest + l - line1, str_cstr.to_bytes(), 0, false);
        if dest < line1 {
            extra += 1;
        }
    }

    let mut last_line = curbuf_mut().b_ml.ml_line_count;
    mark_adjust_nofold(line1, line2, last_line - line2, 0, K_EXTMARK_NOOP);

    g_st().disable_fold_update += 1;
    changed_lines(
        curbuf_mut(),
        last_line - num_lines + 1,
        0,
        last_line + 1,
        num_lines,
        false,
    );
    g_st().disable_fold_update -= 1;

    let mut line_off: i32 = 0;
    let mut byte_off: Bcount = 0;
    if dest >= line2 {
        mark_adjust_nofold(line2 + 1, dest, -num_lines, 0, K_EXTMARK_NOOP);
        for tab in for_all_tabpages_ptr() {
            for win in for_all_windows_in_tab_mut(&*tab) {
                if win.w_buffer == g_st().curbuf {
                    fold_move_range(&mut win.w_folds, line1, line2, dest);
                }
            }
        }
        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start.lnum = dest - num_lines + 1;
            curbuf_mut().b_op_end.lnum = dest;
        }
        line_off = -num_lines;
        byte_off = -extent_byte;
    } else {
        mark_adjust_nofold(dest + 1, line1 - 1, num_lines, 0, K_EXTMARK_NOOP);
        for tab in for_all_tabpages_ptr() {
            for win in for_all_windows_in_tab_mut(&*tab) {
                if win.w_buffer == g_st().curbuf {
                    fold_move_range(&mut win.w_folds, dest + 1, line1 - 1, line2);
                }
            }
        }
        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start.lnum = dest + 1;
            curbuf_mut().b_op_end.lnum = dest + num_lines;
        }
    }
    if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
        curbuf_mut().b_op_end.col = 0;
        curbuf_mut().b_op_start.col = 0;
    }
    mark_adjust_nofold(
        last_line - num_lines + 1,
        last_line,
        -(last_line - dest - extra),
        0,
        K_EXTMARK_NOOP,
    );

    g_st().disable_fold_update += 1;
    changed_lines(
        curbuf_mut(),
        last_line - num_lines + 1,
        0,
        last_line + 1,
        -extra,
        false,
    );
    g_st().disable_fold_update -= 1;

    buf_updates_send_changes(curbuf_mut(), dest + 1, num_lines as i64, 0);

    if !u_save(line1 + extra - 1, line2 + extra + 1) {
        return false;
    }

    for _ in line1..=line2 {
        ml_delete_flags(line1 + extra, ML_DEL_MESSAGE);
    }
    if g_st().global_busy == 0 && (num_lines as OptInt) > opt_vars().p_report {
        smsg!(
            0,
            ngettext(c"%lld line moved", c"%lld lines moved", num_lines as u64,),
            num_lines as i64,
        );
    }

    extmark_move_region(
        curbuf_mut(),
        line1 - 1,
        0,
        start_byte,
        line2 - line1 + 1,
        0,
        extent_byte,
        dest + line_off,
        0,
        dest_byte + byte_off,
        K_EXTMARK_UNDO,
    );

    if dest >= line1 {
        curwin_mut().w_cursor.lnum = dest;
    } else {
        curwin_mut().w_cursor.lnum = dest + (line2 - line1) + 1;
    }

    if line1 < dest {
        dest += num_lines + 1;
        last_line = curbuf_mut().b_ml.ml_line_count;
        dest = dest.min(last_line + 1);
        changed_lines(curbuf_mut(), line1, 0, dest, 0, false);
    } else {
        changed_lines(curbuf_mut(), dest + 1, 0, line1 + num_lines, 0, false);
    }

    buf_updates_send_changes(curbuf_mut(), line1 + extra, 0, num_lines as i64);

    true
}

/// `":copy"`.
pub fn ex_copy(mut line1: Linenr, mut line2: Linenr, n: Linenr) {
    let count = line2 - line1 + 1;
    if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
        curbuf_mut().b_op_start.lnum = n + 1;
        curbuf_mut().b_op_end.lnum = n + count;
        curbuf_mut().b_op_end.col = 0;
        curbuf_mut().b_op_start.col = 0;
    }

    if !u_save(n, n + 1) {
        return;
    }

    curwin_mut().w_cursor.lnum = n;
    while line1 <= line2 {
        let p = bytes_to_cstring(crate::memline::ml_get_slice(line1));
        ml_append(curwin_mut().w_cursor.lnum, p.to_bytes(), 0, false);

        if line1 == n {
            line1 = curwin_mut().w_cursor.lnum;
        }
        line1 += 1;
        if curwin_mut().w_cursor.lnum < line1 {
            line1 += 1;
        }
        if curwin_mut().w_cursor.lnum < line2 {
            line2 += 1;
        }
        curwin_mut().w_cursor.lnum += 1;
    }

    appended_lines_mark(n, count);
    if g_st().visual_global.active {
        check_pos(curbuf_mut(), &mut g_st().visual_global.start);
    }

    msgmore(count);
}

pub fn free_prev_shellcmd() {
    EXCMDS_PREVCMD.with(|p| *p.borrow_mut() = None);
}

/// Check that `prevcmd` is not NULL.
fn prevcmd_is_set() -> bool {
    if EXCMDS_PREVCMD.with(|p| p.borrow().is_none()) {
        emsg(gettext(E_NOPREV));
        return false;
    }
    true
}

/// Handle the `":!cmd"` command.
pub fn do_bang(addr_count: i32, eap: &mut Exarg, forceit: bool, do_in: bool, do_out: bool) {
    let arg_owned = eap.arg.to_cstring();
    let line1 = eap.line1;
    let line2 = eap.line2;
    let mut newcmd: Vec<u8> = Vec::new();
    let scroll_save = g_st().msg_scroll;

    if check_secure() {
        return;
    }

    if addr_count == 0 {
        g_st().msg_scroll = 0;
        autowrite_all();
        g_st().msg_scroll = scroll_save;
    }

    let mut ins_prevcmd = forceit;

    let skip_n = crate::charset::skipwhite_len(arg_owned.to_bytes());
    let mut trailarg_vec = arg_owned.to_bytes_with_nul()[skip_n..].to_vec();
    loop {
        let mut t: Vec<u8> = Vec::new();
        if !newcmd.is_empty() {
            t.extend_from_slice(crate::memory::bytes_until_nul(&newcmd));
        }
        if ins_prevcmd {
            if !prevcmd_is_set() {
                return;
            }
            EXCMDS_PREVCMD.with(|p| {
                t.extend_from_slice(p.borrow().as_ref().unwrap().to_bytes());
            });
        }
        let mut p_idx = t.len();
        t.extend_from_slice(crate::memory::bytes_as_cstr(&trailarg_vec).to_bytes_with_nul());
        newcmd = t;

        let mut next_trail_off: Option<usize> = None;
        while newcmd[p_idx] != 0 {
            if newcmd[p_idx] == b'!' {
                if p_idx > 0 && newcmd[p_idx - 1] == b'\\' {
                    let rest_len = crate::memory::bytes_until_nul(&newcmd[p_idx..]).len() + 1;
                    newcmd.copy_within(p_idx..(p_idx + rest_len), p_idx - 1);
                } else {
                    newcmd[p_idx] = 0;
                    next_trail_off = Some(p_idx + 1);
                    ins_prevcmd = true;
                    break;
                }
            }
            p_idx += 1;
        }
        let Some(off) = next_trail_off else {
            break;
        };
        let rem_cstr = crate::memory::cstring_until_nul(&newcmd[off..]);
        newcmd.truncate(off);
        trailarg_vec = rem_cstr.into_bytes_with_nul();
    }

    if !crate::memory::bytes_until_nul(&newcmd).is_empty() {
        EXCMDS_PREVCMD.with(|p| *p.borrow_mut() = Some(crate::memory::cstring_until_nul(&newcmd)));
    }

    'theend: {
        if g_st().bangredo {
            if !prevcmd_is_set() {
                break 'theend;
            }

            let cmd = EXCMDS_PREVCMD
                .with(|p| vim_strsave_escaped(p.borrow().as_ref().unwrap().as_bytes(), b"%#"));
            redo_append_lit(cmd.to_bytes());
            redo_append_str(b"\n");
            g_st().bangredo = false;
        }
        if !opt_vars().p_shq.is_empty() {
            let shq_owned = opt_vars().p_shq.to_owned();
            let shq_bytes = shq_owned.as_bytes();
            EXCMDS_PREVCMD.with(|p| {
                let borrow = p.borrow();
                let prev_bytes = borrow.as_ref().map_or(&[][..], |s| s.to_bytes());
                newcmd = Vec::with_capacity(prev_bytes.len() + 2 * shq_bytes.len() + 1);
                newcmd.extend_from_slice(shq_bytes);
                newcmd.extend_from_slice(prev_bytes);
                newcmd.extend_from_slice(shq_bytes);
                newcmd.push(0);
            });
        }
        if addr_count == 0 {
            msg_start();
            msg_ext_no_fast();
            msg_ext_set_kind(c"shell_cmd");
            msg_putchar(b':' as i32);
            msg_putchar(b'!' as i32);
            msg_outtrans(crate::memory::bytes_as_cstr(&newcmd), 0, false);
            msg_clr_eos();
            ui_cursor_goto(g_st().msg_row, g_st().msg_col);

            do_shell(Some(crate::memory::bytes_as_cstr(&newcmd)), 0);
        } else {
            do_filter(
                line1,
                line2,
                eap,
                crate::memory::bytes_as_cstr(&newcmd),
                do_in,
                do_out,
            );
            apply_autocmds(EVENT_SHELLFILTERPOST, None, None, false, curbuf_opt_mut());
        }
    }
}

/// `do_filter`: filter lines through a command given by the user.
fn do_filter(line1: Linenr, line2: Linenr, eap: &mut Exarg, cmd: &CStr, do_in: bool, do_out: bool) {
    let mut itmp_owned: Option<std::ffi::CString> = None;
    let mut otmp_owned: Option<std::ffi::CString> = None;
    let old_curbuf = g_st().curbuf;
    let mut shell_flags: i32 = 0;
    let orig_start = curbuf_mut().b_op_start;
    let orig_end = curbuf_mut().b_op_end;
    let stmp = opt_vars().p_stmp;

    if cmd.to_bytes().is_empty() {
        return;
    }

    let save_cmod_flags = g_st().cmdmod.cmod_flags;
    g_st().cmdmod.cmod_flags &= !CMOD_LOCKMARKS;

    let cursor_save = curwin_mut().w_cursor;
    let mut linecount = line2 - line1 + 1;
    curwin_mut().w_cursor.lnum = line1;
    curwin_mut().w_cursor.col = 0;
    changed_line_abv_curs();
    invalidate_botline_win(curwin_mut());

    if do_out {
        shell_flags |= K_SHELL_OPT_DO_OUT;
    }

    'filterend: {
        if !do_in && do_out && !stmp {
            shell_flags |= K_SHELL_OPT_READ;
            curwin_mut().w_cursor.lnum = line2;
        } else if do_in && !do_out && !stmp {
            shell_flags |= K_SHELL_OPT_WRITE;
            curbuf_mut().b_op_start.lnum = line1;
            curbuf_mut().b_op_end.lnum = line2;
        } else if do_in && do_out && !stmp {
            shell_flags |= K_SHELL_OPT_READ | K_SHELL_OPT_WRITE;
            curbuf_mut().b_op_start.lnum = line1;
            curbuf_mut().b_op_end.lnum = line2;
            curwin_mut().w_cursor.lnum = line2;
        } else {
            if do_in {
                itmp_owned = vim_tempname();
                if itmp_owned.is_none() {
                    emsg(gettext(E_NOTMP));
                    break 'filterend;
                }
            }
            if do_out {
                otmp_owned = vim_tempname();
                if otmp_owned.is_none() {
                    emsg(gettext(E_NOTMP));
                    break 'filterend;
                }
            }
        }

        let itmp_cstr = itmp_owned.as_deref();
        let otmp_cstr = otmp_owned.as_deref();

        g_st().no_wait_return += 1;
        if let Some(itmp_s) = itmp_cstr
            && !buf_write(
                curbuf_mut(),
                Some(itmp_s),
                None,
                line1,
                line2,
                Some(&*eap),
                false,
                false,
                false,
                true,
            )
        {
            if !ui_has(K_UIMESSAGES) {
                msg_putchar(b'\n' as i32);
            }
            g_st().no_wait_return -= 1;
            if !aborting() {
                semsg!(gettext(c"E482: Can't create file %s"), itmp_s);
            }
            break 'filterend;
        }
        if g_st().curbuf != old_curbuf {
            break 'filterend;
        }

        if !do_out && !ui_has(K_UIMESSAGES) {
            msg_putchar(b'\n' as i32);
        }

        let cmd_buf = make_filter_cmd(cmd, itmp_cstr, otmp_cstr, do_in);
        ui_cursor_goto(g_st().rows_global - 1, 0);

        'error: {
            if do_out {
                if !u_save(line2, line2 + 1) {
                    break 'error;
                }
                redraw_curbuf_later(UPD_VALID);
            }
            let mut read_linecount = curbuf_mut().b_ml.ml_line_count;

            call_shell(
                Some(cmd_buf.as_c_str()),
                K_SHELL_OPT_FILTER | shell_flags,
                None,
            );
            drop(cmd_buf);

            g_st().did_check_timestamps = false;
            g_st().need_check_timestamps = true;

            os_breakcheck();
            g_st().got_int = false;

            if do_out {
                if let Some(otmp_s) = otmp_cstr {
                    if readfile(
                        Some(otmp_s),
                        None,
                        line2,
                        0,
                        MAXLNUM,
                        Some(&*eap),
                        READ_FILTER,
                        false,
                    ) != Some(true)
                    {
                        if !aborting() {
                            msg_putchar(b'\n' as i32);
                            semsg!(gettext(E_CANT_READ_FILE_STR), otmp_s);
                        }
                        break 'error;
                    }
                    if g_st().curbuf != old_curbuf {
                        break 'filterend;
                    }
                }

                read_linecount = curbuf_mut().b_ml.ml_line_count - read_linecount;

                if (shell_flags & K_SHELL_OPT_READ) != 0 {
                    curbuf_mut().b_op_start.lnum = line2 + 1;
                    curbuf_mut().b_op_end.lnum = curwin_mut().w_cursor.lnum;
                    appended_lines_mark(line2, read_linecount);
                }

                if do_in {
                    if (g_st().cmdmod.cmod_flags & CMOD_KEEPMARKS) != 0
                        || !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_REMMARK)
                    {
                        if read_linecount >= linecount {
                            mark_adjust(line1, line2, linecount, 0, K_EXTMARK_NOOP);
                        } else {
                            mark_adjust(
                                line1,
                                line1 + read_linecount - 1,
                                linecount,
                                0,
                                K_EXTMARK_NOOP,
                            );
                            mark_adjust(line1 + read_linecount, line2, MAXLNUM, 0, K_EXTMARK_NOOP);
                        }
                    }

                    curwin_mut().w_cursor.lnum = line1;
                    del_lines(linecount, true);
                    if read_linecount == 0 {
                        let l = line1.min(curbuf_mut().b_ml.ml_line_count);
                        curbuf_mut().b_op_start.lnum = l;
                        curbuf_mut().b_op_end.lnum = l;
                        curbuf_mut().b_op_start.col = 0;
                        curbuf_mut().b_op_end.col = 0;
                    } else {
                        curbuf_mut().b_op_start.lnum -= linecount;
                        curbuf_mut().b_op_end.lnum -= linecount;
                    }
                    write_lnum_adjust(-linecount);
                    fold_update(
                        curwin_mut(),
                        curbuf_mut().b_op_start.lnum,
                        curbuf_mut().b_op_end.lnum,
                    );
                } else {
                    linecount = curbuf_mut().b_op_end.lnum - curbuf_mut().b_op_start.lnum + 1;
                    curwin_mut().w_cursor.lnum = curbuf_mut().b_op_end.lnum;
                }

                beginline(BL_WHITE | BL_FIX);
                g_st().no_wait_return -= 1;

                if (linecount as OptInt) > opt_vars().p_report {
                    if do_in {
                        format_to_buf!(
                            &mut g_st().msg_buf,
                            gettext(c"%lld lines filtered"),
                            linecount as i64,
                        );
                        let msg_cstr =
                            crate::memory::bytes_as_cstr(g_st().borrow_mut().msg_buf_bytes_mut());
                        if msg(msg_cstr, 0) && g_st().borrow_mut().msg_scroll == 0 {
                            set_keep_msg(Some(msg_cstr), 0);
                        }
                    } else {
                        msgmore(linecount);
                    }
                }
                break 'filterend;
            }
        }

        // error:
        curwin_mut().w_cursor = cursor_save;
        g_st().no_wait_return -= 1;
        if !ui_has(K_UIMESSAGES) {
            wait_return(0);
        }
    }

    // filterend:
    g_st().cmdmod.cmod_flags = save_cmod_flags;
    if g_st().curbuf != old_curbuf {
        g_st().no_wait_return -= 1;
        emsg(gettext(
            c"E135: *Filter* Autocommands must not change current buffer",
        ));
    } else if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
        curbuf_mut().b_op_start = orig_start;
        curbuf_mut().b_op_end = orig_end;
    }

    if let Some(ref itmp) = itmp_owned {
        os_remove(itmp);
    }
    if let Some(ref otmp) = otmp_owned {
        os_remove(otmp);
    }
}

/// Call a shell to execute a command.
pub fn do_shell(cmd: Option<&CStr>, flags: i32) {
    if check_secure() {
        msg_end();
        return;
    }

    msg_putchar(b'\r' as i32);
    msg_putchar(b'\n' as i32);

    if opt_vars().p_warn && !g_st().autocmd_busy && g_st().msg_silent == 0 {
        for buf in for_all_buffers_mut() {
            if buf_is_changed_fn(buf) {
                msg_puts(gettext(c"[No write since last change]\n"));
                break;
            }
        }
    }

    ui_cursor_goto(g_st().msg_row, g_st().msg_col);
    call_shell(cmd, flags, None);
    if g_st().msg_silent == 0 {
        g_st().msg_didout = true;
    }
    g_st().did_check_timestamps = false;
    g_st().need_check_timestamps = true;

    g_st().msg_row = g_st().rows_global - 1;
    g_st().msg_col = 0;

    apply_autocmds(EVENT_SHELLCMDPOST, None, None, false, curbuf_opt_mut());
}

#[cfg(not(unix))]
fn find_pipe(cmd: &[u8]) -> Option<usize> {
    let cmd = crate::memory::bytes_until_nul(cmd);
    let mut inquote = false;
    let mut idx = 0usize;
    while idx < cmd.len() {
        if !inquote && cmd[idx] == b'|' {
            return Some(idx);
        }
        if cmd[idx] == b'"' {
            inquote = !inquote;
        } else if rem_backslash(&cmd[idx..]) {
            idx += 1;
        }
        idx += 1;
    }
    None
}

/// Create a shell command from a command string, input redirection file and
/// output redirection file.
pub fn make_filter_cmd(
    cmd: &CStr,
    itmp: Option<&CStr>,
    otmp: Option<&CStr>,
    do_in: bool,
) -> std::ffi::CString {
    let p_sh = opt_vars().p_sh.to_owned();
    let sh_tail = invocation_path_tail(&p_sh, None).to_bytes();
    #[cfg(unix)]
    let is_fish_shell = sh_tail.starts_with(b"fish");
    #[cfg(not(unix))]
    let is_fish_shell = false;

    let is_pwsh = sh_tail.starts_with(b"pwsh") || sh_tail.starts_with(b"powershell");

    let mut len = cmd.to_bytes().len() + 1;

    len += if is_fish_shell {
        b"begin; ; end".len()
    } else if !is_pwsh {
        b"()".len()
    } else {
        0
    };

    if let Some(itmp_s) = itmp {
        len += if is_pwsh {
            itmp_s.to_bytes().len() + b"& { Get-Content  | &  }".len() + 6
        } else {
            itmp_s.to_bytes().len() + b" {  <  } ".len()
        };
    }

    if do_in && is_pwsh {
        len += b" $input | \0".len();
    }

    if let Some(otmp_s) = otmp {
        len += otmp_s.to_bytes().len() + opt_vars().p_srr.as_bytes().len() + 2;
    }

    let mut buf_vec = vec![0u8; len];

    let cmd_bytes = cmd.to_bytes();
    let itmp_bytes = itmp.map_or(&[][..], CStr::to_bytes);
    if is_pwsh {
        if itmp.is_some() {
            crate::memory::copy_bytes_to_c_buf(&mut buf_vec[..len - 1], b"& { Get-Content ");
            crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len - 1], itmp_bytes);
            crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len - 1], b" | & ");
            crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len - 1], cmd_bytes);
            crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len - 1], b" }");
        } else if do_in {
            crate::memory::copy_bytes_to_c_buf(&mut buf_vec[..len - 1], b" $input | ");
            crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len], cmd_bytes);
        } else {
            crate::memory::copy_bytes_to_c_buf(&mut buf_vec, cmd_bytes);
        }
    } else {
        #[cfg(unix)]
        {
            if itmp.is_some() || otmp.is_some() {
                let fmt = if is_fish_shell {
                    c"begin; %s; end"
                } else {
                    c"(%s)"
                };
                format_to_buf!(&mut buf_vec, fmt, cmd);
            } else {
                crate::memory::copy_bytes_to_c_buf(&mut buf_vec, cmd_bytes);
            }

            if itmp.is_some() {
                crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len - 1], b" < ");
                crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len - 1], itmp_bytes);
            }
        }
        #[cfg(not(unix))]
        {
            crate::memory::copy_bytes_to_c_buf(&mut buf_vec, cmd_bytes);
            if itmp.is_some() {
                if opt_vars().p_shq.is_empty()
                    && let Some(pipe_idx) = find_pipe(&buf_vec)
                {
                    buf_vec[pipe_idx] = 0;
                }
                crate::memory::append_bytes_to_c_buf(&mut buf_vec, b" < ");
                crate::memory::append_bytes_to_c_buf(&mut buf_vec, itmp_bytes);
                if opt_vars().p_shq.is_empty()
                    && let Some(pipe_idx) = find_pipe(cmd_bytes)
                {
                    crate::memory::append_bytes_to_c_buf(&mut buf_vec[..len - 1], b" ");
                    crate::memory::append_bytes_to_c_buf(
                        &mut buf_vec[..len - 1],
                        &cmd_bytes[pipe_idx..],
                    );
                }
            }
        }
    }
    if let Some(otmp_s) = otmp {
        append_redir(&mut buf_vec, opt_vars().p_srr.as_cstr(), otmp_s);
    }
    crate::memory::cstring_until_nul(&buf_vec)
}

/// Append output redirection for the given file to the given buffer.
pub fn append_redir(buf: &mut [u8], opt: &CStr, fname: &CStr) {
    let end_idx = crate::memory::bytes_until_nul(buf).len();
    let opt_bytes = opt.to_bytes();
    let mut idx = 0usize;
    let mut found_pct_s = false;
    while let Some(pos) = opt_bytes[idx..].iter().position(|&b| b == b'%') {
        let p_idx = idx + pos;
        let next_b = opt_bytes.get(p_idx + 1).copied().unwrap_or(0);
        if next_b == b's' {
            found_pct_s = true;
            break;
        } else if next_b == b'%' {
            idx = p_idx + 2;
        } else {
            idx = p_idx + 1;
        }
    }
    if found_pct_s {
        buf[end_idx] = b' ';
        let rem = &mut buf[end_idx + 1..];
        format_to_buf!(rem, opt, fname);
    } else {
        let rem = &mut buf[end_idx..];
        format_to_buf!(rem, c" %s %s", opt, fname);
    }
}

pub fn print_line_no_prefix(lnum: Linenr, use_number: bool, list: bool) {
    let mut numbuf: [u8; 30] = [0; 30];

    if curwin_mut().w_onebuf_opt.wo_nu || use_number {
        format_to_buf!(&mut numbuf, c"%*d ", number_width(curwin_mut()), lnum);
        msg_puts_hl(crate::memory::bytes_as_cstr(&numbuf), HLF_N + 1, false);
    }
    msg_prt_line(crate::memline::ml_get_slice(lnum), list);
}

/// Print a text line. Also in silent mode (`"ex -s"`).
pub fn print_line(lnum: Linenr, use_number: bool, list: bool, first: bool) {
    let save_silent = g_st().silent_mode;

    if message_filtered(ml_get(lnum)) {
        return;
    }

    g_st().silent_mode = false;
    g_st().info_message = true;
    if (g_st().global_busy == 0 || EXCMDS_STATE.with(|s| s.global_need_msg_kind.get())) && first {
        msg_start();
        msg_ext_set_kind(c"list_cmd");
        EXCMDS_STATE.with(|s| s.global_need_msg_kind.set(false));
    } else if !save_silent {
        msg_putchar(b'\n' as i32);
    }
    print_line_no_prefix(lnum, use_number, list);
    if save_silent {
        msg_putchar(b'\n' as i32);
        g_st().silent_mode = save_silent;
    }
    g_st().info_message = false;
}

pub fn rename_buffer(new_fname: Option<&CStr>) -> bool {
    let mut buf = g_st().curbuf;
    apply_autocmds(EVENT_BUFFILEPRE, None, None, false, curbuf_opt_mut());
    if g_st().curbuf != buf {
        return false;
    }
    if aborting() {
        return false;
    }
    let (fname_cstr, sfname_cstr, xfname_kind) = curbuf_mut().take_fnames();
    if !setfname(curbuf_mut(), new_fname, None, true) {
        curbuf_mut().restore_fnames(fname_cstr, sfname_cstr, xfname_kind);
        return false;
    }
    curbuf_mut().b_flags |= BF_NOTEDITED;
    let xfname = match xfname_kind {
        crate::buffer::BufFname::None => None,
        crate::buffer::BufFname::Short => sfname_cstr.as_deref(),
        crate::buffer::BufFname::Full => fname_cstr.as_deref(),
    };
    if xfname.is_some_and(|s| !s.to_bytes().is_empty()) {
        buf = crate::globals::BufPtr::from(buflist_new(
            fname_cstr.as_deref(),
            xfname,
            curwin_mut().w_cursor.lnum,
            0,
        ));
        if let Some(b) = buf.opt()
            && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0
        {
            curwin_mut().w_alt_fnum = b.b_fnum;
        }
    }
    drop(fname_cstr);
    drop(sfname_cstr);
    apply_autocmds(EVENT_BUFFILEPOST, None, None, false, curbuf_opt_mut());
    do_autochdir();
    true
}

/// `":file[!] [fname]"`.
pub fn ex_file(eap: &mut Exarg) {
    if eap.addr_count > 0 && (!eap.arg.is_empty() || eap.line2 > 0 || eap.addr_count > 1) {
        emsg(gettext(E_INVARG));
        return;
    }

    if !eap.arg.is_empty() || eap.addr_count == 1 {
        if !rename_buffer(eap.arg.as_opt_cstr()) {
            return;
        }
        g_st().redraw_tabline = true;
    }

    if eap.arg.is_empty() || !shortmess(K_SHM_FILEINFO) {
        fileinfo(0, 0, eap.forceit);
    }
}

/// `":update"`.
pub fn ex_update(eap: &mut Exarg) {
    if curbuf_is_changed_fn()
        || (!bt_nofilename(Some(crate::globals::curbuf_mut()))
            && curbuf_mut()
                .ffname_opt()
                .is_some_and(|f| !os_path_exists(f)))
    {
        do_write(eap);
    }
}

/// `":write"` and `":saveas"`.
pub fn ex_write(eap: &mut Exarg) {
    if eap.cmdidx == Cmdidx::CmdSaveas {
        eap.line1 = 1;
        eap.line2 = curbuf_mut().b_ml.ml_line_count;
    }

    if eap.usefilter {
        do_bang(1, eap, false, true, false);
    } else {
        do_write(eap);
    }
}

#[cfg(unix)]
fn check_writable(fname: &CStr) -> bool {
    if os_nodetype(fname) == NODE_OTHER {
        semsg!(
            gettext(c"E503: \"%s\" is not a file or writable device"),
            fname
        );
        return false;
    }
    true
}

fn handle_mkdir_p_arg(eap: &mut Exarg, fname: Option<&CStr>) -> bool {
    if eap.mkdir_p && os_file_mkdir(fname.unwrap_or(c""), 0o755) < 0 {
        return false;
    }
    true
}

/// Write current buffer to file `eap->arg`.
pub fn do_write(eap: &mut Exarg) -> bool {
    let mut fname: Option<&CStr> = None;
    let mut retval = false;
    let mut free_fname: Option<std::ffi::CString> = None;
    let mut alt_buf = crate::globals::BufPtr::NULL;

    if not_writing() {
        return false;
    }

    let arg_owned = eap.arg.to_cstring();
    let mut ffname: Option<&CStr> = Some(arg_owned.as_c_str());
    let other: bool;

    'theend: {
        if eap.arg.is_empty() {
            if eap.cmdidx == Cmdidx::CmdSaveas {
                emsg(gettext(E_ARGREQ));
                break 'theend;
            }
            other = false;
        } else {
            fname = ffname;
            free_fname = fix_fname(ffname);
            if let Some(ref ff) = free_fname {
                ffname = Some(ff.as_c_str());
            }
            other = otherfile(ffname);
        }

        if other {
            if crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_ALTWRITE)
                || eap.cmdidx == Cmdidx::CmdSaveas
            {
                alt_buf = crate::globals::BufPtr::from(setaltfname(ffname, fname, 1));
            } else {
                alt_buf = crate::globals::BufPtr::from(buflist_findname(ffname));
            }
            if let Some(ab) = alt_buf.opt()
                && ab.b_ml.ml_mfp.is_some()
            {
                emsg(gettext(E_BUFLOADED));
                break 'theend;
            }
        }

        if !other {
            if bt_dontwrite_msg(Some(crate::globals::curbuf_mut())) || !check_fname() {
                break 'theend;
            }
            #[cfg(unix)]
            if !check_writable(curbuf_mut().ffname_cstr()) {
                break 'theend;
            }
            if check_readonly(&mut eap.forceit, curbuf_mut()) {
                break 'theend;
            }
        }

        if !other {
            ffname = curbuf_mut().ffname_opt();
            fname = curbuf_mut().fname_opt();
            if (eap.line1 != 1 || eap.line2 != curbuf_mut().b_ml.ml_line_count)
                && !eap.forceit
                && !eap.append
                && !opt_vars().p_wa
            {
                if opt_vars().p_confirm || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                    if vim_dialog_yesno(VIM_QUESTION, None, gettext(c"Write partial file?"), 2)
                        != VIM_YES
                    {
                        break 'theend;
                    }
                    eap.forceit = true;
                } else {
                    emsg(gettext(c"E140: Use ! to write partial buffer"));
                    break 'theend;
                }
            }
        }

        if check_overwrite(eap, curbuf_mut(), fname, ffname, other) {
            if eap.cmdidx == Cmdidx::CmdSaveas && !alt_buf.is_null() {
                let was_curbuf = g_st().curbuf;

                apply_autocmds(EVENT_BUFFILEPRE, None, None, false, curbuf_opt_mut());
                apply_autocmds(EVENT_BUFFILEPRE, None, None, false, alt_buf.opt_mut());
                if g_st().curbuf != was_curbuf || aborting() {
                    retval = false;
                    break 'theend;
                }

                alt_buf.swap_fnames(curbuf_mut());

                buf_name_changed(curbuf_mut());
                apply_autocmds(EVENT_BUFFILEPOST, None, None, false, curbuf_opt_mut());
                apply_autocmds(EVENT_BUFFILEPOST, None, None, false, alt_buf.opt_mut());
                if !alt_buf.b_p_bl {
                    alt_buf.b_p_bl = true;

                    apply_autocmds(EVENT_BUFADD, None, None, false, alt_buf.opt_mut());
                }
                if g_st().curbuf != was_curbuf || aborting() {
                    retval = false;
                    break 'theend;
                }

                if curbuf_mut().b_p_ft.is_empty() {
                    if augroup_exists(Some(c"filetypedetect")) {
                        do_doautocmd(c"filetypedetect BufRead", true, None);
                    }
                    do_modelines(0);
                }

                fname = curbuf_mut().sfname_opt();
            }

            if !handle_mkdir_p_arg(eap, fname) {
                retval = false;
                break 'theend;
            }

            let name_was_missing = curbuf_mut().b_ffname.is_none();
            retval = buf_write(
                curbuf_mut(),
                ffname,
                fname,
                eap.line1,
                eap.line2,
                Some(&*eap),
                eap.append,
                eap.forceit,
                true,
                false,
            );

            if eap.cmdidx == Cmdidx::CmdSaveas && retval {
                curbuf_mut().b_p_ro = false;
                g_st().redraw_tabline = true;
            }

            if eap.cmdidx == Cmdidx::CmdSaveas || name_was_missing {
                do_autochdir();
            }
        }
    }

    drop(free_fname);
    retval
}

/// Check if it is allowed to overwrite a file.
pub fn check_overwrite(
    eap: &mut Exarg,
    buf: &mut FileBuf,
    fname_opt: Option<&CStr>,
    ffname_opt: Option<&CStr>,
    other: bool,
) -> bool {
    let fname_cstr = fname_opt.unwrap_or(c"");
    let ffname_cstr = ffname_opt.unwrap_or(c"");
    if (other
        || (!bt_nofilename(Some(&*buf))
            && ((buf.b_flags & BF_NOTEDITED) != 0
                || ((buf.b_flags & BF_NEW) != 0
                    && !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_OVERNEW))
                || (buf.b_flags & BF_READERR) != 0)))
        && !opt_vars().p_wa
        && os_path_exists(ffname_cstr)
    {
        if !eap.forceit && !eap.append {
            if os_isdir(ffname_cstr) {
                semsg!(gettext(E_ISADIR2), ffname_cstr);
                return false;
            }
            if opt_vars().p_confirm || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                let mut buff: [u8; DIALOG_MSG_SIZE] = [0; DIALOG_MSG_SIZE];
                format_to_buf!(
                    &mut buff,
                    gettext(c"Overwrite existing file \"%s\"?"),
                    fname_cstr,
                );
                if vim_dialog_yesno(VIM_QUESTION, None, crate::memory::bytes_as_cstr(&buff), 2)
                    != VIM_YES
                {
                    return false;
                }
                eap.forceit = true;
            } else {
                emsg(gettext(E_EXISTS));
                return false;
            }
        }

        if other && g_st().emsg_silent == 0 {
            let mut dir = [0u8; MAXPATHL];
            if opt_vars().p_dir.is_empty() {
                dir[0] = b'.';
                dir[1] = 0;
            } else {
                let p_dir = opt_vars().p_dir.to_owned();
                let mut p = p_dir.as_bytes();
                copy_option_part(&mut p, &mut dir, c",");
            }
            if let Some(swapname) = makeswapname(fname_opt, crate::memory::bytes_as_cstr(&dir))
                && os_path_exists(&swapname)
            {
                if opt_vars().p_confirm || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                    let mut buff: [u8; DIALOG_MSG_SIZE] = [0; DIALOG_MSG_SIZE];
                    format_to_buf!(
                        &mut buff,
                        gettext(c"Swap file \"%s\" exists, overwrite anyway?"),
                        &swapname,
                    );
                    if vim_dialog_yesno(VIM_QUESTION, None, crate::memory::bytes_as_cstr(&buff), 2)
                        != VIM_YES
                    {
                        return false;
                    }
                    eap.forceit = true;
                } else {
                    semsg!(
                        gettext(c"E768: Swap file exists: %s (:silent! overrides)"),
                        &swapname,
                    );
                    return false;
                }
            }
        }
    }
    true
}

/// Handle `":wnext"`, `":wNext"` and `":wprevious"` commands.
pub fn ex_wnext(eap: &mut Exarg) {
    let i = if eap.cmd.as_bytes().get(1) == Some(&b'n') {
        curwin_mut().w_arg_idx + eap.line2
    } else {
        curwin_mut().w_arg_idx - eap.line2
    };
    eap.line1 = 1;
    eap.line2 = curbuf_mut().b_ml.ml_line_count;
    if do_write(eap) {
        do_argfile(eap, i);
    }
}

/// `":wall"`, `":wqall"` and `":xall"`: Write all changed files (and exit).
pub fn do_wqall(eap: &mut Exarg) {
    let mut error: i32 = 0;
    let save_forceit = eap.forceit;
    let save_exiting = g_st().exiting;

    if eap.cmdidx == Cmdidx::CmdXall || eap.cmdidx == Cmdidx::CmdWqall {
        if !before_quit_all(eap) {
            return;
        }
        g_st().exiting = true;
    }

    let mut buf = g_st().firstbuf;
    while !buf.is_null() {
        let b = buf.borrow_mut();
        if g_st().exiting
            && !eap.forceit
            && !b.terminal.is_null()
            && channel_job_running(b.b_p_channel as u64)
        {
            no_write_message_buf(b);
            error += 1;
            buf = b.b_next;
            continue;
        } else if !buf_is_changed_fn(b) || bt_dontwrite(Some(&*b)) {
            buf = b.b_next;
            continue;
        }

        if not_writing() {
            error += 1;
            break;
        }
        if b.b_ffname.is_none() {
            semsg!(
                gettext(c"E141: No file name for buffer %lld"),
                b.b_fnum as i64,
            );
            error += 1;
        } else if check_readonly(&mut eap.forceit, b) || {
            let b_fname = b.fname_opt().map(CStr::to_owned);
            let b_ffname = b.ffname_opt().map(CStr::to_owned);
            !check_overwrite(eap, b, b_fname.as_deref(), b_ffname.as_deref(), false)
        } {
            error += 1;
        } else {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, buf.opt_mut());
            if !handle_mkdir_p_arg(eap, b.fname_opt()) || !buf_write_all(b, eap.forceit) {
                error += 1;
            }
            if !bufref_valid(Some(&bufref)) {
                buf = g_st().firstbuf;
                eap.forceit = save_forceit;
                if !buf.is_null() {
                    buf = buf.b_next;
                }
                continue;
            }
        }
        eap.forceit = save_forceit;
        buf = buf.b_next;
    }
    if g_st().exiting {
        if error == 0 {
            getout(0);
        }
        not_exiting(save_exiting);
    }
}

/// Check the `'write'` option.
fn not_writing() -> bool {
    if opt_vars().p_write {
        return false;
    }
    emsg(gettext(
        c"E142: File not written: Writing is disabled by 'write' option",
    ));
    true
}

/// Check if a buffer is read-only.
fn check_readonly(forceit: &mut bool, buf: &mut FileBuf) -> bool {
    if !*forceit
        && (buf.b_p_ro
            || (os_path_exists(buf.ffname_cstr()) && os_file_is_writable(buf.ffname_cstr()) == 0))
    {
        if (opt_vars().p_confirm || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
            && buf.fname_opt().is_some()
        {
            let mut buff: [u8; DIALOG_MSG_SIZE] = [0; DIALOG_MSG_SIZE];
            let fname = buf.fname_opt().unwrap_or_else(|| gettext(c"Untitled"));
            if buf.b_p_ro {
                format_to_buf!(
                    &mut buff,
                    gettext(c"'readonly' option is set for \"%s\".\nDo you wish to write anyway?"),
                    fname,
                );
            } else {
                format_to_buf!(
                    &mut buff,
                    gettext(
                        c"File permissions of \"%s\" are read-only.\nIt may still be possible to write it.\nDo you wish to try?",
                    ),
                    fname,
                );
            }

            if vim_dialog_yesno(VIM_QUESTION, None, crate::memory::bytes_as_cstr(&buff), 2)
                == VIM_YES
            {
                *forceit = true;
                return false;
            }
            return true;
        } else if buf.b_p_ro {
            emsg(gettext(E_READONLY));
        } else {
            semsg!(
                gettext(c"E505: \"%s\" is read-only (add ! to override)"),
                buf.fname_cstr(),
            );
        }
        return true;
    }

    false
}

/// Try to abandon the current file and edit a new or existing file.
pub fn getfile(
    fnum: i32,
    ffname_arg: Option<&CStr>,
    sfname_arg: Option<&CStr>,
    setpm: bool,
    lnum: Linenr,
    forceit: bool,
) -> GetFileResult {
    if !check_can_set_curbuf_forceit(forceit) {
        return GetFileResult::Error;
    }

    let other: bool;
    let retval: GetFileResult;
    let expanded: Option<(std::ffi::CString, std::ffi::CString)>;

    if text_locked() {
        return GetFileResult::Error;
    }
    if curbuf_locked() {
        return GetFileResult::Error;
    }

    if fnum == 0 {
        expanded = fname_expand(g_st().curbuf.opt(), ffname_arg, sfname_arg);
        other = otherfile(expanded.as_ref().map(|(ff, _)| ff.as_c_str()));
    } else {
        expanded = None;
        other = fnum != curbuf_mut().b_fnum;
    }

    if other {
        g_st().no_wait_return += 1;
    }
    'theend: {
        if other
            && !forceit
            && curbuf_mut().b_nwindows == 1
            && !buf_hide(curbuf_mut())
            && curbuf_is_changed_fn()
            && !autowrite(curbuf_mut(), forceit)
        {
            if opt_vars().p_confirm && opt_vars().p_write {
                dialog_changed(curbuf_mut(), false);
            }
            if curbuf_is_changed_fn() {
                g_st().no_wait_return -= 1;
                no_write_message();
                retval = GetFileResult::NotWritten;
                break 'theend;
            }
        }
        if other {
            g_st().no_wait_return -= 1;
        }
        if setpm {
            setpcmark();
        }
        if !other {
            if lnum != 0 {
                curwin_mut().w_cursor.lnum = lnum;
            }
            check_cursor_lnum(curwin_mut());
            beginline(BL_SOL | BL_FIX);
            retval = GetFileResult::SameFile;
        } else if do_ecmd(
            fnum,
            expanded.as_ref().map(|(ff, _)| ff.as_c_str()),
            expanded.as_ref().map(|(_, sf)| sf.as_c_str()),
            None,
            lnum,
            (if buf_hide(curbuf_mut()) { ECMD_HIDE } else { 0 })
                + (if forceit { ECMD_FORCEIT } else { 0 }),
            Some(curwin_mut()),
        ) {
            retval = GetFileResult::OpenOther;
        } else {
            retval = GetFileResult::Error;
        }
    }

    drop(expanded);
    retval
}

/// Set `v:swapcommand` for the `SwapExists` autocommands.
pub fn set_swapcommand(command: Option<&CStr>, newlnum: Linenr) -> bool {
    if (command.is_none() && newlnum <= 0) || !get_vim_var_str(VV_SWAPCOMMAND).is_empty() {
        return false;
    }
    let valsize = if let Some(cmd) = command {
        cmd.to_bytes().len() + 3
    } else {
        30
    };
    let mut data_vec = vec![0u8; valsize];
    let size = if let Some(cmd) = command {
        format_to_buf_safelen!(&mut data_vec, c":%s\r", cmd)
    } else {
        format_to_buf_safelen!(&mut data_vec, c"%lldG", newlnum as i64)
    };
    set_vim_var_bytes(VV_SWAPCOMMAND, &data_vec[..size]);
    true
}

/// Start editing a new file.
pub fn do_ecmd(
    fnum: i32,
    ffname_arg: Option<&CStr>,
    sfname_arg: Option<&CStr>,
    eap: Option<&Exarg>,
    mut newlnum: Linenr,
    flags: i32,
    oldwin_arg: Option<&mut WinInfoInfo>,
) -> bool {
    let mut oldwin = crate::globals::WinPtr::from(oldwin_arg);
    let mut ffname = ffname_arg;
    let mut sfname = sfname_arg;
    let sfname_fixed: Option<std::ffi::CString>;
    let other_file: bool;
    let oldbuf: bool;
    let mut auto_buf = false;
    let mut new_name: Option<std::ffi::CString> = None;
    let mut did_set_swapcommand = false;
    let mut buf: crate::globals::BufPtr;
    let mut bufref: Bufref = Bufref::new();
    let mut old_curbuf: Bufref = Bufref::new();
    let mut free_fname: Option<std::ffi::CString> = None;
    let mut retval = false;
    let mut topline: Linenr = 0;
    let mut newcol: i32 = -1;
    let mut solcol: i32 = -1;
    let command: Option<std::ffi::CString> = eap.as_ref().and_then(|e| e.do_ecmd_cmd.clone());
    let mut did_get_winopts = false;
    let mut readfile_flags: i32 = 0;
    let mut did_inc_redrawing_disabled = false;
    let use_win_so = curwin_mut().w_onebuf_opt.wo_so >= 0;

    set_bufref(&mut old_curbuf, g_st().curbuf.opt_mut());

    'theend: {
        let opt_is_empty =
            |opt: Option<&CStr>| -> bool { opt.is_none_or(|s| s.to_bytes().is_empty()) };
        if fnum != 0 {
            if fnum == curbuf_mut().b_fnum {
                return true;
            }
            other_file = true;
        } else {
            let sfname_was_none = sfname.is_none();
            if sfname.is_none() {
                sfname = ffname;
            }
            if CASE_INSENSITIVE_FILENAME && let Some(sfname_cstr) = sfname {
                let mut bytes = sfname_cstr.to_bytes_with_nul().to_vec();
                path_fix_case(&mut bytes);
                sfname_fixed = Some(crate::memory::cstring_until_nul(&bytes));
                sfname = sfname_fixed.as_deref();
                if sfname_was_none {
                    ffname = sfname;
                }
            }

            if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) != 0 && opt_is_empty(ffname) {
                break 'theend;
            }

            if ffname.is_none() {
                other_file = true;
            } else if opt_is_empty(ffname) && curbuf_mut().b_ffname.is_none() {
                other_file = false;
            } else {
                if opt_is_empty(ffname) {
                    ffname = curbuf_mut().ffname_opt();
                    sfname = curbuf_mut().fname_opt();
                }
                free_fname = fix_fname(ffname);
                if let Some(ref ff) = free_fname {
                    ffname = Some(ff.as_c_str());
                }
                other_file = otherfile(ffname);
            }
        }

        if !other_file && !curbuf_mut().terminal.is_null() {
            check_arg_idx(curwin_mut());
            maketitle();
            retval = true;
            break 'theend;
        }

        if ((!other_file && (flags & ECMD_OLDBUF) == 0)
            || (curbuf_mut().b_nwindows == 1
                && (flags & (ECMD_HIDE | ECMD_ADDBUF | ECMD_ALTBUF)) == 0))
            && check_changed(
                curbuf_mut(),
                (if opt_vars().p_awa { CCGD_AW } else { 0 })
                    | (if other_file { 0 } else { CCGD_MULTWIN })
                    | (if (flags & ECMD_FORCEIT) != 0 {
                        CCGD_FORCEIT
                    } else {
                        0
                    })
                    | (if eap.is_none() { 0 } else { CCGD_EXCMD }),
            )
        {
            if fnum == 0 && other_file && ffname.is_some() {
                setaltfname(ffname, sfname, if newlnum < 0 { 0 } else { newlnum });
            }
            break 'theend;
        }

        reset_visual();

        if !oldwin.is_null() && !win_valid(oldwin.opt()) {
            oldwin = crate::globals::WinPtr::NULL;
        }

        did_set_swapcommand = set_swapcommand(command.as_deref(), newlnum);

        if other_file {
            let prev_alt_fnum = curwin_mut().w_alt_fnum;

            if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) == 0 {
                if (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
                    curwin_mut().w_alt_fnum = curbuf_mut().b_fnum;
                }
                if let Some(ow) = oldwin.opt_mut() {
                    buflist_altfpos(ow);
                }
            }

            if fnum != 0 {
                buf = crate::globals::BufPtr::from(buflist_findnr(fnum));
            } else {
                if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) != 0 {
                    let mut tlnum: Linenr = 0;

                    if let Some(ref cmd) = command {
                        tlnum = crate::charset::parse_i64_prefix(cmd.as_bytes()) as Linenr;
                        if tlnum <= 0 {
                            tlnum = 1;
                        }
                    }
                    let newbuf = buflist_new(ffname, sfname, tlnum, BLN_LISTED | BLN_NOCURWIN);
                    if let Some(nb) = newbuf
                        && (flags & ECMD_ALTBUF) != 0
                    {
                        curwin_mut().w_alt_fnum = nb.b_fnum;
                    }
                    break 'theend;
                }
                buf = crate::globals::BufPtr::from(buflist_new(
                    ffname,
                    sfname,
                    0,
                    BLN_CURBUF
                        | if (flags & ECMD_SET_HELP) != 0 {
                            0
                        } else {
                            BLN_LISTED
                        },
                ));
                if !oldwin.is_null() {
                    oldwin = crate::globals::curwin_handle();
                }
                set_bufref(&mut old_curbuf, g_st().curbuf.opt_mut());
            }
            if buf.is_null() {
                break 'theend;
            }
            if buf.b_locked_split > 0 {
                emsg(gettext(E_CANNOT_SWITCH_TO_A_CLOSING_BUFFER));
                break 'theend;
            }
            if curwin_mut().w_alt_fnum == buf.b_fnum && prev_alt_fnum != 0 {
                curwin_mut().w_alt_fnum = prev_alt_fnum;
            }
            if buf.b_ml.ml_mfp.is_none() {
                oldbuf = false;
            } else {
                oldbuf = true;
                set_bufref(&mut bufref, buf.opt_mut());
                buf_check_timestamp(buf.borrow_mut());
                if !bufref_valid(Some(&bufref)) || old_curbuf.br_buf != g_st().curbuf {
                    break 'theend;
                }
                if aborting() {
                    break 'theend;
                }
            }

            if (oldbuf && newlnum == ECMD_LASTL) || newlnum == ECMD_LAST {
                let pos = buflist_findfmark(buf.borrow_mut()).mark;
                newlnum = pos.lnum;
                solcol = pos.col;
            }

            if buf != g_st().curbuf.as_ptr() {
                if let Some(f) = buf.fname_opt() {
                    new_name = Some(f.to_owned());
                }
                let save_au_new_curbuf = g_st().au_new_curbuf;
                set_bufref(&mut g_st().au_new_curbuf, buf.opt_mut());
                apply_autocmds(EVENT_BUFLEAVE, None, None, false, curbuf_opt_mut());

                if !bufref_valid(Some(&g_st().au_new_curbuf)) {
                    delbuf_msg(new_name.take());
                    g_st().au_new_curbuf = save_au_new_curbuf;
                    break 'theend;
                }
                if aborting() {
                    g_st().au_new_curbuf = save_au_new_curbuf;
                    break 'theend;
                }
                if buf == g_st().curbuf.as_ptr() {
                    auto_buf = true;
                } else {
                    let mut the_curwin = curwin_ptr();

                    the_curwin.w_locked += 1;
                    buf.b_locked += 1;

                    if old_curbuf.br_buf == g_st().curbuf {
                        buf_copy_options(buf.borrow_mut(), BCO_ENTER);
                    }

                    u_sync(false);
                    close_buffer(
                        curwin_ptr().opt_mut(),
                        g_st().curbuf.opt_mut(),
                        if (flags & ECMD_HIDE) != 0
                            || (!curbuf_mut().terminal.is_null()
                                && terminal_running(curbuf_mut().terminal.borrow()))
                        {
                            0
                        } else {
                            DOBUF_UNLOAD
                        },
                        false,
                        false,
                        !oldwin.is_null(),
                    );

                    if win_valid(the_curwin.opt()) {
                        {
                            the_curwin.w_locked -= 1;
                        }
                    }
                    buf.b_locked -= 1;

                    if aborting() && !curwin_mut().w_buffer.is_null() {
                        g_st().au_new_curbuf = save_au_new_curbuf;
                        break 'theend;
                    }
                    if !bufref_valid(Some(&g_st().au_new_curbuf)) {
                        if curwin_mut().w_buffer.is_null() {
                            buf = g_st().lastbuf;
                        } else {
                            delbuf_msg(new_name.take());
                            g_st().au_new_curbuf = save_au_new_curbuf;
                            break 'theend;
                        }
                    }
                    if buf == g_st().curbuf.as_ptr() {
                        auto_buf = true;
                    } else {
                        if !curwin_mut().w_buffer.is_null() {
                            curwin_mut().buf_mut().b_nwindows -= 1;
                        }

                        curwin_mut().w_buffer = buf;
                        g_st().curbuf = buf;
                        curbuf_mut().b_nwindows += 1;

                        if !oldbuf && let Some(eap) = eap {
                            set_file_options(true, Some(eap));
                            set_forced_fenc(eap);
                        }
                    }

                    get_winopts(curbuf_mut());
                    did_get_winopts = true;
                }
                new_name = None;
                g_st().au_new_curbuf = save_au_new_curbuf;
            }

            curwin_mut().w_pcmark.lnum = 1;
            curwin_mut().w_pcmark.col = 0;
        } else {
            if (flags & (ECMD_ADDBUF | ECMD_ALTBUF)) != 0 || !check_fname() {
                break 'theend;
            }
            oldbuf = (flags & ECMD_OLDBUF) != 0;
        }

        g_st().redrawing_disabled += 1;
        did_inc_redrawing_disabled = true;

        buf = g_st().curbuf;
        if (flags & ECMD_SET_HELP) != 0 || g_st().keep_help_flag {
            prepare_help_buffer();
        } else if !curbuf_mut().b_help {
            set_buflisted(1);
        }

        if buf != g_st().curbuf.as_ptr() {
            break 'theend;
        }
        if aborting() {
            break 'theend;
        }

        curbuf_mut().b_did_filetype = false;

        if !other_file && !oldbuf {
            set_last_cursor(curwin_mut());
            if newlnum == ECMD_LAST || newlnum == ECMD_LASTL {
                newlnum = curwin_mut().w_cursor.lnum;
                solcol = curwin_mut().w_cursor.col;
            }
            buf = g_st().curbuf;
            new_name = buf.fname_opt().map(CStr::to_owned);
            set_bufref(&mut bufref, buf.opt_mut());

            if (curbuf_mut().b_flags & BF_NEVERLOADED) == 0
                && (opt_vars().p_ur < 0
                    || (curbuf_mut().b_ml.ml_line_count as OptInt) <= opt_vars().p_ur)
            {
                u_sync(false);
                if !u_savecommon(
                    curbuf_mut(),
                    0,
                    curbuf_mut().b_ml.ml_line_count + 1,
                    0,
                    true,
                ) {
                    break 'theend;
                }
                u_unchanged(curbuf_mut());
                buf_freeall(curbuf_mut(), BFA_KEEP_UNDO);

                readfile_flags = READ_KEEP_UNDO;
            } else {
                buf_freeall(curbuf_mut(), 0);
            }
            if !bufref_valid(Some(&bufref)) {
                delbuf_msg(new_name.take());
                break 'theend;
            }
            new_name = None;

            if buf != g_st().curbuf.as_ptr() {
                break 'theend;
            }
            if aborting() {
                break 'theend;
            }
            buf_clear_file(curbuf_mut());
            curbuf_mut().b_op_start.lnum = 0;
            curbuf_mut().b_op_end.lnum = 0;
        }

        retval = true;

        if !other_file {
            curbuf_mut().b_flags &= !BF_NOTEDITED;
        }

        check_arg_idx(curwin_mut());

        if !auto_buf {
            curwin_init();

            for tp in for_all_tabpages_ptr() {
                for win in for_all_windows_in_tab_mut(&*tp) {
                    if win.w_buffer == g_st().curbuf {
                        fold_update_all(win);
                    }
                }
            }

            do_autochdir();

            let orig_pos = curwin_mut().w_cursor;
            topline = curwin_mut().w_topline;
            if !oldbuf {
                g_st().swap_exists_action = SEA_DIALOG;
                curbuf_mut().b_flags |= BF_CHECK_RO;

                if (flags & ECMD_NOWINENTER) != 0 {
                    readfile_flags |= READ_NOWINENTER;
                }
                if should_abort(open_buffer(false, eap, readfile_flags)) {
                    retval = false;
                }

                if g_st().swap_exists_action == SEA_QUIT {
                    retval = false;
                }
                handle_swap_exists(Some(&mut old_curbuf));
            } else {
                do_modelines(OPT_WINONLY);

                apply_autocmds_retval(
                    EVENT_BUFENTER,
                    None,
                    None,
                    false,
                    curbuf_opt_mut(),
                    &mut retval,
                );
                if (flags & ECMD_NOWINENTER) == 0 {
                    apply_autocmds_retval(
                        EVENT_BUFWINENTER,
                        None,
                        None,
                        false,
                        curbuf_opt_mut(),
                        &mut retval,
                    );
                }
            }
            check_arg_idx(curwin_mut());

            if !equalpos(curwin_mut().w_cursor, orig_pos) {
                let text_bytes = crate::cursor::get_cursor_line_slice();
                let first_nonwhite =
                    (text_bytes.len() - crate::charset::skipwhite_slice(text_bytes).len()) as i32;

                if curwin_mut().w_cursor.lnum != orig_pos.lnum
                    || curwin_mut().w_cursor.col != first_nonwhite
                {
                    newlnum = curwin_mut().w_cursor.lnum;
                    newcol = curwin_mut().w_cursor.col;
                }
            }
            if curwin_mut().w_topline == topline {
                topline = 0;
            }

            changed_line_abv_curs();

            maketitle();
            if retval {
                win_float_update_preview(curwin_mut());
            }
        }

        if curwin_mut().w_onebuf_opt.wo_diff {
            diff_buf_add(curbuf_mut());
            diff_invalidate(curbuf_mut());
        }

        if did_get_winopts
            && curwin_mut().w_onebuf_opt.wo_spell
            && !curwin_mut().w_s_ptr().b_p_spl.is_empty()
        {
            parse_spelllang(curwin_mut());
        }

        if command.is_none() {
            if newcol >= 0 {
                curwin_mut().w_cursor.lnum = newlnum;
                curwin_mut().w_cursor.col = newcol;
                check_cursor(curwin_mut());
            } else if newlnum > 0 {
                curwin_mut().w_cursor.lnum = newlnum;
                check_cursor_lnum(curwin_mut());
                if solcol >= 0 && !opt_vars().p_sol {
                    curwin_mut().w_cursor.col = solcol;
                    check_cursor_col(curwin_mut());
                    curwin_mut().w_cursor.coladd = 0;
                    curwin_mut().w_set_curswant = true;
                } else {
                    beginline(BL_SOL | BL_FIX);
                }
            } else {
                beginline(BL_WHITE | BL_FIX);
            }
        }

        check_lnums(false);

        if oldbuf && !auto_buf {
            let msg_scroll_save = g_st().msg_scroll;

            if shortmess(K_SHM_OVERALL)
                && g_st().msg_listdo_overwrite == 0
                && !g_st().exiting
                && opt_vars().p_verbose == 0
            {
                g_st().msg_scroll = 0;
            }
            if g_st().msg_scroll == 0 {
                msg_check_for_delay(false);
            }
            msg_start();
            g_st().msg_scroll = msg_scroll_save;
            g_st().msg_scrolled_ign = true;

            if !shortmess(K_SHM_FILEINFO) {
                fileinfo(0, 1, false);
            }

            g_st().msg_scrolled_ign = false;
        }

        curbuf_mut().b_last_used = os_time() as time_t;

        if let Some(ref cmd) = command {
            do_cmdline(
                Some(cmd.as_c_str()),
                crate::cmdexpand::LineGetter::None,
                DOCMD_VERBOSE,
            );
        }

        if (curbuf_mut().b_kmap_state & KEYMAP_INIT) != 0 {
            keymap_init();
        }

        g_st().redrawing_disabled -= 1;
        did_inc_redrawing_disabled = false;
        if !g_st().skip_redraw {
            let n = if use_win_so {
                curwin_mut().w_onebuf_opt.wo_so
            } else {
                opt_vars().p_so
            };
            if topline == 0 && command.is_none() {
                if use_win_so {
                    curwin_mut().w_onebuf_opt.wo_so = 999;
                } else {
                    opt_vars().p_so = 999;
                }
            }
            update_topline(curwin_mut());
            curwin_mut().w_scbind_pos =
                plines_m_win_fill(crate::globals::curwin_mut(), 1, curwin_mut().w_topline);
            if use_win_so {
                curwin_mut().w_onebuf_opt.wo_so = n;
            } else {
                opt_vars().p_so = n;
            }
            redraw_curbuf_later(UPD_NOT_VALID);
        }

        update_cwd(K_CD_CAUSE_BUFFER);

        do_autochdir();
    }

    // theend:
    if bufref_valid(Some(&old_curbuf)) && !old_curbuf.br_buf.terminal.is_null() {
        terminal_check_size(old_curbuf.br_buf.terminal.borrow_mut());
    }
    if (!bufref_valid(Some(&old_curbuf)) || old_curbuf.br_buf != g_st().curbuf)
        && !curbuf_mut().terminal.is_null()
    {
        terminal_check_size(curbuf_mut().terminal_mut());
    }

    if did_inc_redrawing_disabled {
        g_st().redrawing_disabled -= 1;
    }
    if did_set_swapcommand {
        set_vim_var_string(VV_SWAPCOMMAND, None, -1);
    }
    drop(new_name);
    drop(free_fname);
    retval
}
fn delbuf_msg(name: Option<std::ffi::CString>) {
    semsg!(
        gettext(c"E143: Autocommands unexpectedly deleted new buffer %s"),
        name.as_deref().unwrap_or(c""),
    );
    g_st().au_new_curbuf.br_buf = crate::globals::BufPtr::NULL;
    g_st().au_new_curbuf.br_buf_free_count = 0;
}

/// Get the next line of text for `":append"`, `":insert"` and `":change"`.
fn get_append_line(eap: &mut Exarg, indent: i32) -> Option<Vec<u8>> {
    if eap.arg.first_byte() == b'|' {
        let theline = eap.arg.wrapping_add(1).to_cstring().into_bytes_with_nul();

        if let Some(b) = eap.arg_cstr_buf_mut().first_mut() {
            *b = NUL as u8;
        }

        Some(theline)
    } else if eap.ea_getline.is_none() {
        if eap.nextcmd.is_null() {
            return None;
        }
        let next_bytes = eap.nextcmd.as_bytes();
        let len = next_bytes
            .iter()
            .position(|&b| b == NL as u8)
            .unwrap_or(next_bytes.len());
        let mut theline = eap.nextcmd.slice(len).to_vec();
        theline.push(0);
        if len < next_bytes.len() {
            eap.nextcmd = eap.nextcmd.wrapping_add(len + 1);
        } else {
            eap.nextcmd = crate::strings::OwnedSlice::NULL;
        }
        Some(theline)
    } else {
        let getline_state = g_st().state_global;
        g_st().state_global = MODE_CMDLINE;
        let looplevel = eap.cstack().cs_looplevel;
        let raw_line =
            eap.ea_getline
                .call(if looplevel > 0 { -1 } else { NUL as i32 }, indent, true);
        g_st().state_global = getline_state;
        raw_line.map(std::ffi::CString::into_bytes_with_nul)
    }
}

/// `":insert"` and `":append"`, also used by `":change"`.
pub fn ex_append(eap: &mut Exarg) {
    let mut did_undo = false;
    let mut lnum = eap.line2;
    let mut indent: i32 = 0;
    let mut empty = (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0;

    if eap.skip {
        while let Some(theline) = get_append_line(eap, 0) {
            if theline[0] == b'.' && theline[1] == 0 {
                break;
            }
        }
        return;
    }

    if eap.forceit {
        curbuf_mut().b_p_ai = !curbuf_mut().b_p_ai;
    }

    if eap.cmdidx != Cmdidx::CmdChange && curbuf_mut().b_p_ai && lnum > 0 {
        EXCMDS_STATE.with(|s| s.append_indent.set(get_indent_lnum(lnum)));
    }

    if eap.cmdidx != Cmdidx::CmdAppend {
        lnum -= 1;
    }

    if empty && lnum == 1 {
        lnum = 0;
    }

    let save_state = g_st().state_global;
    g_st().state_global = MODE_INSERT;
    if curbuf_mut().b_p_iminsert == B_IMODE_LMAP {
        g_st().state_global |= MODE_LANGMAP;
    }

    loop {
        g_st().msg_scroll = 1;
        g_st().need_wait_return = false;
        if curbuf_mut().b_p_ai {
            if EXCMDS_STATE.with(|s| s.append_indent.get()) >= 0 {
                indent = EXCMDS_STATE.with(|s| s.append_indent.get());
                EXCMDS_STATE.with(|s| s.append_indent.set(-1));
            } else if lnum > 0 {
                indent = get_indent_lnum(lnum);
            }
        }
        let mut theline_vec = get_append_line(eap, indent);
        g_st().lines_left = g_st().rows_global - 1;
        let Some(ref mut theline_v) = theline_vec else {
            break;
        };

        let mut vcol: i32 = 0;
        let mut p_idx = 0usize;
        while indent > vcol {
            if theline_v.get(p_idx) == Some(&b' ') {
                vcol += 1;
            } else if theline_v.get(p_idx) == Some(&(TAB as u8)) {
                vcol += 8 - vcol % 8;
            } else {
                break;
            }
            p_idx += 1;
        }
        let p_cstr = crate::memory::bytes_as_cstr(&theline_v[p_idx.min(theline_v.len())..]);
        if p_cstr.to_bytes() == b"."
            || (!did_undo && !u_save(lnum, lnum + 1 + if empty { 1 } else { 0 }))
        {
            break;
        }

        if p_cstr.to_bytes().is_empty() {
            theline_v[0] = 0;
        }

        did_undo = true;
        ml_append(lnum, crate::memory::bytes_until_nul(theline_v), 0, false);
        if empty {
            appended_lines(lnum, 1);
        } else {
            appended_lines_mark(lnum, 1);
        }

        lnum += 1;

        if empty {
            ml_delete(2);
            empty = false;
        }
    }
    g_st().state_global = save_state;
    ui_cursor_shape();

    if eap.forceit {
        curbuf_mut().b_p_ai = !curbuf_mut().b_p_ai;
    }

    if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
        curbuf_mut().b_op_start.lnum = if eap.line2 < curbuf_mut().b_ml.ml_line_count {
            eap.line2 + 1
        } else {
            curbuf_mut().b_ml.ml_line_count
        };
        if eap.cmdidx != Cmdidx::CmdAppend {
            curbuf_mut().b_op_start.lnum -= 1;
        }
        curbuf_mut().b_op_end.lnum = if eap.line2 < lnum {
            lnum
        } else {
            curbuf_mut().b_op_start.lnum
        };
        curbuf_mut().b_op_end.col = 0;
        curbuf_mut().b_op_start.col = 0;
    }
    curwin_mut().w_cursor.lnum = lnum;
    check_cursor_lnum(curwin_mut());
    beginline(BL_SOL | BL_FIX);

    g_st().need_wait_return = false;
    redraw_curbuf_later(UPD_NOT_VALID);
}

/// `":change"`.
pub fn ex_change(eap: &mut Exarg) {
    if eap.skip {
        ex_append(eap);
        return;
    }
    if eap.line2 >= eap.line1 && !u_save(eap.line1 - 1, eap.line2 + 1) {
        return;
    }

    if if eap.forceit {
        !curbuf_mut().b_p_ai
    } else {
        curbuf_mut().b_p_ai
    } {
        EXCMDS_STATE.with(|s| s.append_indent.set(get_indent_lnum(eap.line1)));
    }

    let mut lnum = eap.line2;
    while lnum >= eap.line1 {
        if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
            break;
        }
        ml_delete(eap.line1);
        lnum -= 1;
    }

    check_cursor_lnum(curwin_mut());
    deleted_lines_mark(eap.line1, eap.line2 - lnum);

    eap.line2 = eap.line1;
    ex_append(eap);
}

pub fn ex_z(eap: &mut Exarg) {
    let mut bigness: i64;
    let mut minus: i32 = 0;
    let mut start: Linenr;
    let mut end: Linenr;
    let mut curs: Linenr;
    let lnum = eap.line2;

    if eap.forceit {
        bigness = (g_st().rows_global - 1) as i64;
    } else if g_st().firstwin == g_st().lastwin {
        bigness = curwin_mut().w_onebuf_opt.wo_scr * 2;
    } else {
        bigness = (curwin_mut().w_view_height - 3) as i64;
    }
    bigness = bigness.max(1);

    let kind_bytes = eap.arg.as_bytes();
    let kind_b = kind_bytes.first().copied().unwrap_or(0);
    let mut rem = kind_bytes;
    if matches!(kind_b, b'-' | b'+' | b'=' | b'^' | b'.') {
        rem = &rem[1..];
    }
    while rem.first().is_some_and(|&b| matches!(b, b'-' | b'+')) {
        rem = &rem[1..];
    }

    if !rem.is_empty() {
        if !rem[0].is_ascii_digit() {
            emsg(gettext(E_NON_NUMERIC_ARGUMENT_TO_Z));
            return;
        }
        bigness = crate::charset::parse_i64_prefix(rem);

        if bigness > 2 * (curbuf_mut().b_ml.ml_line_count as i64) || bigness < 0 {
            bigness = 2 * (curbuf_mut().b_ml.ml_line_count as i64);
        }

        opt_vars().p_window = bigness as i32 as OptInt;
        if kind_b == b'=' {
            bigness += 2;
        }
    }

    let rep_count = if matches!(kind_b, b'-' | b'+') {
        1 + kind_bytes[1..].iter().take_while(|&&b| b == kind_b).count() as Linenr
    } else {
        0
    };

    match kind_b {
        b'-' => {
            start = lnum - (bigness as Linenr) * rep_count + 1;
            end = start + (bigness as Linenr) - 1;
            curs = end;
        }

        b'=' => {
            start = lnum - ((bigness as Linenr) + 1) / 2 + 1;
            end = lnum + ((bigness as Linenr) + 1) / 2 - 1;
            curs = lnum;
            minus = 1;
        }

        b'^' => {
            start = lnum - (bigness as Linenr) * 2;
            end = lnum - (bigness as Linenr);
            curs = lnum - (bigness as Linenr);
        }

        b'.' => {
            start = lnum - ((bigness as Linenr) + 1) / 2 + 1;
            end = lnum + ((bigness as Linenr) + 1) / 2 - 1;
            curs = end;
        }

        _ => {
            start = lnum;
            if kind_b == b'+' {
                start += (bigness as Linenr) * (rep_count - 1) + 1;
            } else if eap.addr_count == 0 {
                start += 1;
            }
            end = start + (bigness as Linenr) - 1;
            curs = end;
        }
    }

    start = start.max(1);
    end = end.min(curbuf_mut().b_ml.ml_line_count);
    curs = curs.max(1).min(curbuf_mut().b_ml.ml_line_count);

    for i in start..=end {
        if minus != 0 && i == lnum {
            msg_putchar(b'\n' as i32);
            for _ in 1..g_st().columns_global {
                msg_putchar(b'-' as i32);
            }
        }

        print_line(
            i,
            (eap.flags & EXFLAG_NR) != 0,
            (eap.flags & EXFLAG_LIST) != 0,
            i == start,
        );

        if minus != 0 && i == lnum {
            msg_putchar(b'\n' as i32);
            for _ in 1..g_st().columns_global {
                msg_putchar(b'-' as i32);
            }
        }
    }

    if curwin_mut().w_cursor.lnum != curs {
        curwin_mut().w_cursor.lnum = curs;
        curwin_mut().w_cursor.col = 0;
    }
}

/// @return true if the secure flag is set and also give an error message.
pub fn check_secure() -> bool {
    if g_st().secure != 0 {
        g_st().secure = 2;
        emsg(gettext(E_CURDIR));
        return true;
    }

    if g_st().sandbox != 0 {
        emsg(gettext(E_SANDBOX));
        return true;
    }
    false
}

/// Get old substitute replacement string.
pub fn sub_get_replacement(ret_sub: &mut SubReplacementString) {
    *ret_sub = EXCMDS_OLD_SUB.with(|s| s.borrow().clone());
}

/// Set substitute string and timestamp.
pub fn sub_set_replacement(sub: SubReplacementString) {
    EXCMDS_OLD_SUB.with(|s| *s.borrow_mut() = sub);
}

/// Recognize `":%s/\n//"` and turn it into a join command.
fn sub_joining_lines(
    eap: &mut Exarg,
    pat: NvimString,
    sub: &CStr,
    cmd: &CStr,
    save: bool,
    keeppatterns: bool,
) -> bool {
    let sub_bytes = sub.to_bytes();
    let cmd_bytes = cmd.to_bytes();
    if !pat.data.is_null()
        && pat.as_bytes() == b"\\n"
        && sub_bytes.is_empty()
        && (cmd_bytes.is_empty()
            || (cmd_bytes.len() == 1 && matches!(cmd_bytes[0], b'g' | b'l' | b'p' | b'#')))
    {
        if eap.skip {
            return true;
        }
        curwin_mut().w_cursor.lnum = eap.line1;
        match cmd_bytes.first() {
            Some(&b'l') => eap.flags = EXFLAG_LIST,
            Some(&b'#') => eap.flags = EXFLAG_NR,
            Some(&b'p') => eap.flags = EXFLAG_PRINT,
            _ => {}
        }

        let joined_lines_count =
            eap.line2 - eap.line1 + 1 + i32::from(eap.line2 < curbuf_mut().b_ml.ml_line_count);
        if joined_lines_count > 1 {
            do_join(joined_lines_count as usize, false, true, false, true);
            g_st().sub_nsubs = joined_lines_count - 1;
            g_st().sub_nlines = 1;
            do_sub_msg(false);
            ex_may_print(eap);
        }

        if save {
            if !keeppatterns {
                save_re_pat(
                    RE_SUBST,
                    pat.as_cstr(),
                    pat.size,
                    i32::from(opt_vars().p_magic),
                );
            }
            add_to_history(HIST_SEARCH, pat.as_bytes(), true, NUL as i32);
        }

        return true;
    }

    false
}

#[derive(Default)]
struct SubBuf {
    vec: Option<Vec<u8>>,
    size: usize,
}

impl SubBuf {
    const fn new() -> Self {
        Self { vec: None, size: 0 }
    }

    #[inline]
    fn is_null(&self) -> bool {
        self.vec.is_none()
    }

    #[inline]
    fn as_slice(&self) -> &[u8] {
        self.vec.as_deref().unwrap_or(&[])
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [u8] {
        self.vec.as_deref_mut().unwrap_or(&mut [])
    }

    #[inline]
    fn clear(&mut self) {
        self.vec = None;
        self.size = 0;
    }

    #[inline]
    fn from_cbuf(bytes: &[u8]) -> Self {
        let mut v = Vec::with_capacity(bytes.len() + 1);
        v.extend_from_slice(bytes);
        v.push(0);
        Self {
            vec: Some(v),
            size: bytes.len(),
        }
    }

    #[inline]
    fn as_nvim_string(&self) -> NvimString {
        self.vec.as_deref().map_or(NvimString::new(), |v| {
            NvimString::from_bytes_len(v, self.size)
        })
    }
}

/// Allocate memory to store the replacement text for `:substitute`.
fn sub_grow_buf(new_start: &mut SubBuf, new_start_size: &mut usize, mut needed_size: usize) {
    if let Some(ref mut v) = new_start.vec {
        needed_size += new_start.size;
        if needed_size > *new_start_size {
            *new_start_size = needed_size + 50;
            v.resize(*new_start_size, 0);
        }
    } else {
        *new_start_size = needed_size + 50;
        new_start.vec = Some(vec![0; *new_start_size]);
        new_start.size = 0;
    }
}

/// Parse `cmd` string for `:substitute`'s `{flags}` and update `subflags_arg` accordingly.
fn sub_parse_flags<'a>(
    cmd: &'a CStr,
    subflags_arg: &mut Subflags,
    which_pat: &mut i32,
) -> &'a CStr {
    let with_nul = cmd.to_bytes_with_nul();
    let bytes = cmd.to_bytes();
    let mut rem = bytes;
    if let Some(after_amp) = rem.strip_prefix(b"&") {
        rem = after_amp;
    } else {
        subflags_arg.do_all = opt_vars().p_gd;
        subflags_arg.do_ask = false;
        subflags_arg.do_error = true;
        subflags_arg.do_print = false;
        subflags_arg.do_list = false;
        subflags_arg.do_count = false;
        subflags_arg.do_number = false;
        subflags_arg.do_ic = SubIgnoreType::SubHonorOptions;
    }
    while let Some((&b, rest)) = rem.split_first() {
        match b {
            b'g' => subflags_arg.do_all = !subflags_arg.do_all,
            b'c' => subflags_arg.do_ask = !subflags_arg.do_ask,
            b'n' => subflags_arg.do_count = true,
            b'e' => subflags_arg.do_error = !subflags_arg.do_error,
            b'r' => *which_pat = RE_LAST,
            b'p' => subflags_arg.do_print = true,
            b'#' => {
                subflags_arg.do_print = true;
                subflags_arg.do_number = true;
            }
            b'l' => {
                subflags_arg.do_print = true;
                subflags_arg.do_list = true;
            }
            b'i' => subflags_arg.do_ic = SubIgnoreType::SubIgnoreCase,
            b'I' => subflags_arg.do_ic = SubIgnoreType::SubMatchCase,
            _ => break,
        }
        rem = rest;
    }
    if subflags_arg.do_count {
        subflags_arg.do_ask = false;
    }

    let off = bytes.len() - rem.len();
    CStr::from_bytes_until_nul(&with_nul[off..]).unwrap_or(c"")
}

/// Skip over the `"sub"` part in `:s/pat/sub/` where `"delimiter"` is the separating
/// character.
fn skip_substitute(start: &mut [u8], delimiter: i32) -> usize {
    let len = crate::memory::bytes_until_nul(start).len();
    let mut idx = 0usize;
    while idx < len {
        if i32::from(start[idx]) == delimiter {
            start[idx] = 0;
            idx += 1;
            break;
        }
        if start[idx] == b'\\' && idx + 1 < len {
            idx += 1;
        }
        idx += crate::mbyte::utfc_slice2len(&start[idx..len]).max(1);
    }
    idx
}

fn check_regexp_delim(c: i32) -> bool {
    if ascii_isalpha(c) {
        emsg(gettext(
            c"E146: Regular expressions can't be delimited by letters",
        ));
        return false;
    }
    true
}

fn push_preview_lines(
    cmdpreview_ns: i32,
    preview_lines: &mut PreviewLines,
    current_match: SubResult,
) {
    if cmdpreview_ns > 0 {
        let match_lines = current_match.end.lnum - current_match.start.lnum + 1;
        if let Some(last_res) = preview_lines.subresults.last() {
            if last_res.end.lnum == current_match.start.lnum {
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

/// Perform a substitution from line `eap->line1` to line `eap->line2`.
fn do_sub(eap: &mut Exarg, tm: Proftime, cmdpreview_ns: i32, cmdpreview_bufnr: ObjHandle) -> i32 {
    let mut i: i32;
    let mut regmatch: Regmmatch = Regmmatch::new();
    let mut sub = ptr::null_mut();
    let mut sub_owned: Option<std::ffi::CString> = None;
    let mut pat = NvimString::new();
    let delimiter: i32;
    let mut has_second_delim = false;
    let mut got_quit = false;
    let mut got_match = false;
    let mut which_pat: i32;
    let mut cmd = eap
        .arg_cstr_buf_mut()
        .as_mut_ptr()
        .cast::<std::ffi::c_char>();
    let mut first_line: Linenr = 0;
    let mut last_line: Linenr = 0;
    let old_line_count = curbuf_mut().b_ml.ml_line_count;
    let mut sub_firstline: SubBuf;
    let mut endcolumn = false;
    let keeppatterns = (g_st().cmdmod.cmod_flags & CMOD_KEEPPATTERNS) != 0;
    let mut preview_lines = PreviewLines {
        subresults: Vec::new(),
        lines_needed: 0,
    };
    let old_cursor = curwin_mut().w_cursor;

    let mut did_save = false;

    if g_st().global_busy == 0 {
        g_st().sub_nsubs = 0;
        g_st().sub_nlines = 0;
    }
    let start_nsubs = g_st().sub_nsubs;

    if eap.cmdidx == Cmdidx::CmdTilde {
        which_pat = RE_LAST;
    } else {
        which_pat = RE_SUBST;
    }

    let cmd_base = eap.arg_cstr_buf_mut().as_ptr() as usize;
    macro_rules! ptr_cstr {
        ($p:expr, $eap:expr) => {{
            let p: *const std::ffi::c_char = $p;
            let eap_ref: &Exarg = $eap;
            if p.is_null() {
                c""
            } else {
                let off = (p as usize).saturating_sub(cmd_base);
                let slice = eap_ref.arg.cstr_buf();
                crate::memory::bytes_as_cstr(&slice[off.min(slice.len())..])
            }
        }};
    }
    let ptr_first_byte = |p: *const std::ffi::c_char, eap: &Exarg| -> u8 {
        ptr_cstr!(p, eap).to_bytes().first().copied().unwrap_or(0)
    };

    if eap.cmd.first_byte() == b's'
        && !ptr_cstr!(cmd, eap).to_bytes().is_empty()
        && !ascii_iswhite(ptr_first_byte(cmd, eap))
        && !crate::strings::vim_haschr(c"0123456789cegriIp|\"", i32::from(ptr_first_byte(cmd, eap)))
    {
        if !check_regexp_delim(i32::from(ptr_first_byte(cmd, eap))) {
            return 0;
        }

        if ptr_first_byte(cmd, eap) == b'\\' {
            cmd = cmd.wrapping_add(1);
            if !crate::strings::vim_haschr(c"/?&", i32::from(ptr_first_byte(cmd, eap))) {
                emsg(gettext(E_BACKSLASH));
                return 0;
            }
            if ptr_first_byte(cmd, eap) != b'&' {
                which_pat = RE_SEARCH;
            }
            pat = NvimString::empty_cstr();
            delimiter = i32::from(ptr_first_byte(cmd, eap));
            cmd = cmd.wrapping_add(1);
            has_second_delim = true;
        } else {
            which_pat = RE_LAST;
            delimiter = i32::from(ptr_first_byte(cmd, eap));
            cmd = cmd.wrapping_add(1);
            let cmd_off = (cmd as usize).saturating_sub(cmd_base);
            let cmd_len = ptr_cstr!(cmd, eap).to_bytes().len();
            let off = skip_regexp_ex(
                &mut eap.arg_cstr_buf_mut()[cmd_off..cmd_off + cmd_len + 1],
                delimiter,
                i32::from(opt_vars().p_magic),
                None,
                None,
                None,
            );
            cmd = cmd.wrapping_add(off);
            if i32::from(ptr_first_byte(cmd, eap)) == delimiter {
                let delim_off = (cmd as usize).saturating_sub(cmd_base);
                eap.arg_cstr_buf_mut()[delim_off] = 0;

                cmd = cmd.wrapping_add(1);
                has_second_delim = true;
            }
            pat =
                NvimString::from_bytes_len(&eap.arg.as_cstr().to_bytes_with_nul()[cmd_off..], off);
        }

        let p = cmd;
        let sub_off_start = (cmd as usize).saturating_sub(cmd_base);
        let sub_len = ptr_cstr!(cmd, eap).to_bytes().len();
        let sub_off = skip_substitute(
            &mut eap.arg_cstr_buf_mut()[sub_off_start..sub_off_start + sub_len + 1],
            delimiter,
        );
        cmd = cmd.wrapping_add(sub_off);
        let sub_cstr = ptr_cstr!(p, eap).to_owned();
        sub = sub_cstr.as_ptr().cast_mut();
        sub_owned = Some(sub_cstr);

        if !eap.skip && !keeppatterns && cmdpreview_ns <= 0 {
            sub_set_replacement(SubReplacementString {
                sub: sub_owned.clone(),
                timestamp: os_time(),
                additional_data: None,
            });
        }
    } else if !eap.skip {
        let Some(sub_cstr) = EXCMDS_OLD_SUB.with(|s| s.borrow().sub.clone()) else {
            emsg(gettext(E_NOPRESUB));
            return 0;
        };
        pat = NvimString::new();
        sub = sub_cstr.as_ptr().cast_mut();
        sub_owned = Some(sub_cstr);
        endcolumn = curwin_mut().w_curswant == MAXCOL;
    }

    if !sub.is_null() {
        let cmd_owned = ptr_cstr!(cmd, eap).to_owned();
        if sub_joining_lines(
            eap,
            pat.clone(),
            sub_owned.as_deref().unwrap_or(c""),
            &cmd_owned,
            cmdpreview_ns <= 0,
            keeppatterns,
        ) {
            return 0;
        }
    }

    let mut subflags = EXCMDS_STATE.with(|s| s.subflags.get());
    cmd = sub_parse_flags(ptr_cstr!(cmd, eap), &mut subflags, &mut which_pat)
        .as_ptr()
        .cast_mut();
    EXCMDS_STATE.with(|s| s.subflags.set(subflags));

    let save_do_all = EXCMDS_STATE.with(|s| s.subflags.get()).do_all;
    let save_do_ask = EXCMDS_STATE.with(|s| s.subflags.get()).do_ask;

    cmd = cmd.wrapping_add(crate::charset::skipwhite_len(
        ptr_cstr!(cmd, eap).to_bytes(),
    ));
    if ascii_isdigit(ptr_first_byte(cmd, eap)) {
        let count_arg = cmd;
        let cmd_bytes = ptr_cstr!(cmd, eap).to_bytes();
        let mut rem = cmd_bytes;
        i = getdigits_int(&mut rem, false, i32::MAX);
        cmd = cmd.wrapping_add(cmd_bytes.len() - rem.len());
        if i <= 0 && !eap.skip && EXCMDS_STATE.with(|s| s.subflags.get()).do_error {
            emsg(gettext(E_ZEROCOUNT));
            return 0;
        } else if i == i32::MAX {
            semsg!(
                gettext(E_VAL_TOO_LARGE_LEN),
                ((cmd as isize) - (count_arg as isize)) as i32,
                ptr_cstr!(count_arg, eap),
            );
            return 0;
        }
        eap.line1 = eap.line2;
        eap.line2 += i - 1;
        eap.line2 = eap.line2.min(curbuf_mut().b_ml.ml_line_count);
    }

    cmd = cmd.wrapping_add(crate::charset::skipwhite_len(
        ptr_cstr!(cmd, eap).to_bytes(),
    ));
    let b0 = ptr_first_byte(cmd, eap);
    if b0 != 0 && b0 != b'"' {
        eap.nextcmd = crate::strings::OwnedSlice::from_opt_cstr(check_nextcmd(ptr_cstr!(cmd, eap)));
        if eap.nextcmd.is_null() {
            semsg!(gettext(E_TRAILING_ARG), ptr_cstr!(cmd, eap));
            return 0;
        }
    }

    if eap.skip {
        return 0;
    }

    if !EXCMDS_STATE.with(|s| s.subflags.get()).do_count && !modifiable(curbuf_mut()) {
        emsg(gettext(E_MODIFIABLE));
        return 0;
    }

    let magic = if eap.cmdidx == Cmdidx::CmdSmagic {
        true
    } else if eap.cmdidx == Cmdidx::CmdSnomagic {
        false
    } else {
        opt_vars().p_magic
    };
    if !search_regcomp(
        pat.as_cstr(),
        pat.size,
        None,
        RE_SUBST,
        which_pat,
        if cmdpreview_ns > 0 { 0 } else { SEARCH_HIS },
        magic,
        &mut regmatch,
    ) {
        if EXCMDS_STATE.with(|s| s.subflags.get()).do_error {
            emsg(gettext(E_INVCMD));
        }
        return 0;
    }

    if EXCMDS_STATE.with(|s| s.subflags.get()).do_ic == SubIgnoreType::SubIgnoreCase {
        regmatch.rmm_ic = true;
    } else if EXCMDS_STATE.with(|s| s.subflags.get()).do_ic == SubIgnoreType::SubMatchCase {
        regmatch.rmm_ic = false;
    }

    sub_firstline = SubBuf::new();

    debug_assert!(!sub.is_null());

    if !sub_owned
        .as_deref()
        .unwrap_or(c"")
        .to_bytes()
        .starts_with(b"\\=")
        && let Some(ref mut cur_sub) = sub_owned
    {
        let new_sub = crate::memory::cstring_modify(cur_sub, |sub_slice| {
            regtilde(sub_slice, i32::from(opt_vars().p_magic), cmdpreview_ns > 0)
        });
        if let Some(new_sub) = new_sub {
            *cur_sub = new_sub;
        }
    }

    let mut line2 = eap.line2;
    let mut timed_out: i32 = 0;

    let mut lnum = eap.line1;
    while lnum <= line2
        && !got_quit
        && timed_out == 0
        && !aborting()
        && (cmdpreview_ns <= 0
            || preview_lines.lines_needed <= opt_vars().p_cwh as Linenr
            || lnum <= curwin_mut().w_botline)
    {
        let mut nmatch = vim_regexec_multi(
            &mut regmatch,
            Some(curwin_mut()),
            curbuf_mut(),
            lnum,
            0,
            Some(tm),
            Some(&mut timed_out),
        );
        if nmatch != 0 {
            let mut copycol: Colnr = 0;
            let mut matchcol: Colnr = 0;
            let mut prev_matchcol: Colnr = MAXCOL;
            let mut new_start = SubBuf::new();
            let mut new_start_size: usize = 0;
            let mut did_sub = false;
            let mut nmatch_tl: Linenr = 0;
            let mut do_again: bool;
            let mut skip_match = false;
            let mut sub_firstlnum: Linenr = lnum;
            let mut lnum_start: Linenr;

            let mut line_matches: Vec<LineData> = Vec::new();

            if !got_match {
                setpcmark();
                got_match = true;
            }

            loop {
                let mut current_match = SubResult {
                    start: Lpos { lnum: 0, col: 0 },
                    end: Lpos { lnum: 0, col: 0 },
                    pre_match: 0,
                };

                if regmatch.startpos[0].lnum > 0 {
                    current_match.pre_match = lnum;
                    lnum += regmatch.startpos[0].lnum;
                    sub_firstlnum += regmatch.startpos[0].lnum;
                    nmatch -= regmatch.startpos[0].lnum;
                    sub_firstline.clear();
                }

                current_match.start.lnum = sub_firstlnum;

                if lnum > curbuf_mut().b_ml.ml_line_count {
                    break;
                }
                if sub_firstline.is_null() {
                    let sub_slice = crate::memline::ml_get_slice(sub_firstlnum);
                    sub_firstline = SubBuf::from_cbuf(sub_slice);
                }

                curwin_mut().w_cursor.lnum = lnum;
                do_again = false;

                'skip: {
                    if matchcol == prev_matchcol
                        && regmatch.endpos[0].lnum == 0
                        && matchcol == regmatch.endpos[0].col
                    {
                        if sub_firstline.as_slice()[matchcol as usize] == 0 {
                            skip_match = true;
                        } else {
                            matchcol +=
                                utfc_ptr2len(&sub_firstline.as_slice()[matchcol as usize..]);
                        }
                        current_match.start.col = matchcol;
                        current_match.end.lnum = sub_firstlnum;
                        current_match.end.col = matchcol;
                        break 'skip;
                    }

                    matchcol = regmatch.endpos[0].col;
                    prev_matchcol = matchcol;

                    if EXCMDS_STATE.with(|s| s.subflags.get()).do_count {
                        if nmatch > 1 {
                            matchcol = sub_firstline.size as Colnr;
                            nmatch = 1;
                            skip_match = true;
                        }
                        g_st().sub_nsubs += 1;
                        did_sub = true;
                        if !sub_owned
                            .as_deref()
                            .unwrap_or(c"")
                            .to_bytes()
                            .starts_with(b"\\=")
                        {
                            break 'skip;
                        }
                    }

                    if EXCMDS_STATE.with(|s| s.subflags.get()).do_ask && cmdpreview_ns <= 0 {
                        let mut typed: i32 = 0;
                        let save_state = g_st().state_global;
                        curwin_mut().w_cursor.col = regmatch.startpos[0].col;

                        if curwin_mut().w_onebuf_opt.wo_crb {
                            do_check_cursorbind();
                        }

                        if crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_UNDO) {
                            g_st().no_u_sync += 1;
                        }

                        while EXCMDS_STATE.with(|s| s.subflags.get()).do_ask {
                            let mut orig_line = SubBuf::new();
                            let mut len_change: i32 = 0;
                            let save_p_lz = opt_vars().p_lz;
                            let save_p_fen = curwin_mut().w_onebuf_opt.wo_fen;

                            curwin_mut().w_onebuf_opt.wo_fen = false;
                            let temp = g_st().redrawing_disabled;
                            g_st().redrawing_disabled = 0;

                            opt_vars().p_lz = false;

                            if !new_start.is_null() {
                                let lnum_slice = crate::memline::ml_get_slice(lnum);
                                orig_line = SubBuf::from_cbuf(lnum_slice);
                                let sub_rem = crate::memory::bytes_until_nul(
                                    &sub_firstline.as_slice()[copycol as usize..],
                                );
                                let new_line = concat_str(
                                    crate::memory::bytes_until_nul(new_start.as_slice()),
                                    sub_rem,
                                );
                                let new_line_size =
                                    new_start.size + (sub_firstline.size - copycol as usize);

                                len_change = new_line_size as i32 - orig_line.size as i32;
                                curwin_mut().w_cursor.col += len_change;
                                crate::memline::ml_replace_len(
                                    lnum,
                                    &new_line.to_bytes()
                                        [..new_line_size.min(new_line.to_bytes().len())],
                                    new_line_size,
                                    true,
                                );
                            }

                            g_st().search_global.match_lines =
                                regmatch.endpos[0].lnum - regmatch.startpos[0].lnum;
                            g_st().search_global.match_endcol = regmatch.endpos[0].col + len_change;
                            if g_st().search_global.match_lines == 0
                                && g_st().search_global.match_endcol == 0
                            {
                                g_st().search_global.match_endcol = 1;
                            }
                            g_st().search_global.hl_match = true;

                            update_topline(curwin_mut());
                            validate_cursor(curwin_mut());
                            redraw_later(Some(crate::globals::curwin_mut()), UPD_SOME_VALID);
                            show_cursor_info_later(true);
                            update_screen();
                            redraw_later(Some(crate::globals::curwin_mut()), UPD_SOME_VALID);

                            curwin_mut().w_onebuf_opt.wo_fen = save_p_fen;

                            let p_fmt = gettext(
                                c"replace with %s? (y)es/(n)o/(a)ll/(q)uit/(l)ast/scroll up(^E)/down(^Y)",
                            );
                            format_to_buf!(
                                &mut g_st().iobuff,
                                p_fmt,
                                sub_owned.as_deref().unwrap_or(c""),
                            );
                            let p = g_st().iobuff_cstr().to_owned();
                            typed = prompt_for_input(Some(p.as_c_str()), HLF_R, true, None);
                            g_st().search_global.hl_match = false;
                            drop(p);

                            g_st().msg_didout = false;
                            gotocmdline(true);
                            opt_vars().p_lz = save_p_lz;
                            g_st().redrawing_disabled = temp;

                            if let Some(ref orig_vec) = orig_line.vec {
                                crate::memline::ml_replace_len(
                                    lnum,
                                    &orig_vec[..orig_line.size.min(orig_vec.len())],
                                    orig_line.size,
                                    true,
                                );
                            }

                            g_st().need_wait_return = false;
                            if typed == b'q' as i32 || typed == ESC || typed == CTRL_C_CONST {
                                got_quit = true;
                                break;
                            }
                            if typed == b'n' as i32 {
                                break;
                            }
                            if typed == b'y' as i32 {
                                break;
                            }
                            if typed == b'l' as i32 {
                                EXCMDS_STATE.with(|s| {
                                    let mut f = s.subflags.get();
                                    f.do_all = false;
                                    s.subflags.set(f);
                                });
                                line2 = lnum;
                                break;
                            }
                            if typed == b'a' as i32 {
                                EXCMDS_STATE.with(|s| {
                                    let mut f = s.subflags.get();
                                    f.do_ask = false;
                                    s.subflags.set(f);
                                });
                                break;
                            }
                            if typed == CTRL_E {
                                scrollup_clamp();
                            } else if typed == CTRL_Y {
                                scrolldown_clamp();
                            }
                        }
                        g_st().state_global = save_state;
                        setmouse();
                        if crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_UNDO) {
                            g_st().no_u_sync -= 1;
                        }

                        if typed == b'n' as i32 {
                            if nmatch > 1 {
                                matchcol = sub_firstline.size as Colnr;
                                skip_match = true;
                            }
                            break 'skip;
                        }
                        if got_quit {
                            break 'skip;
                        }
                    }

                    curwin_mut().w_cursor.col = regmatch.startpos[0].col;

                    if nmatch > curbuf_mut().b_ml.ml_line_count - sub_firstlnum + 1 {
                        nmatch = curbuf_mut().b_ml.ml_line_count - sub_firstlnum + 1;
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
                            let sub_slice = crate::memline::ml_get_slice(sub_firstlnum);
                            sub_firstline = SubBuf::from_cbuf(sub_slice);
                            if sub_firstlnum <= line2 {
                                do_again = true;
                            } else {
                                EXCMDS_STATE.with(|s| {
                                    let mut f = s.subflags.get();
                                    f.do_all = false;
                                    s.subflags.set(f);
                                });
                            }
                        }
                        if skip_match {
                            sub_firstline = SubBuf::from_cbuf(b"");
                            copycol = 0;
                        }
                        lnum += nmatch - 1;

                        break 'skip;
                    }

                    if cmdpreview_ns <= 0 || has_second_delim {
                        lnum_start = lnum;
                        let save_ma = curbuf_mut().b_p_ma;
                        let save_sandbox = g_st().sandbox;
                        if EXCMDS_STATE.with(|s| s.subflags.get()).do_count {
                            curbuf_mut().b_p_ma = false;
                            g_st().sandbox += 1;
                        }
                        let subflags_save = EXCMDS_STATE.with(|s| s.subflags.get());

                        g_st().textlock += 1;
                        let rel_lnum = sub_firstlnum - regmatch.startpos[0].lnum;
                        let sub_cstr = sub_owned.as_deref().unwrap_or(c"");
                        let sublen_i = vim_regsub_multi(
                            &mut regmatch,
                            rel_lnum,
                            sub_cstr,
                            &mut [],
                            REGSUB_BACKSLASH | if opt_vars().p_magic { REGSUB_MAGIC } else { 0 },
                        );
                        let mut sublen = sublen_i as usize;
                        g_st().textlock -= 1;

                        EXCMDS_STATE.with(|s| s.subflags.set(subflags_save));
                        if sublen == 0
                            || aborting()
                            || EXCMDS_STATE.with(|s| s.subflags.get()).do_count
                        {
                            curbuf_mut().b_p_ma = save_ma;
                            g_st().sandbox = save_sandbox;
                            break 'skip;
                        }

                        let tmp: NvimString;
                        if nmatch == 1 {
                            tmp = sub_firstline.as_nvim_string();
                        } else {
                            let lastlnum = sub_firstlnum + nmatch - 1;
                            let last_slice = crate::memline::ml_get_slice(lastlnum);
                            tmp = cstrn_as_string(last_slice);
                            nmatch_tl += nmatch - 1;
                        }
                        let copy_len = (regmatch.startpos[0].col - copycol) as usize;
                        sub_grow_buf(
                            &mut new_start,
                            &mut new_start_size,
                            copy_len + (tmp.size - regmatch.endpos[0].col as usize) + sublen + 1,
                        );

                        let cc = copycol as usize;
                        let ns_size = new_start.size;
                        new_start.as_mut_slice()[ns_size..ns_size + copy_len]
                            .copy_from_slice(&sub_firstline.as_slice()[cc..cc + copy_len]);
                        new_start.size += copy_len;

                        let new_end_idx = new_start.size;

                        if new_start_size - copy_len < sublen {
                            sublen = new_start_size - copy_len - 1;
                        }

                        let start_col = new_start.size as i32;
                        current_match.start.col = start_col;

                        g_st().textlock += 1;
                        let rel_lnum = sub_firstlnum - regmatch.startpos[0].lnum;
                        let mut n = vim_regsub_multi(
                            &mut regmatch,
                            rel_lnum,
                            sub_cstr,
                            &mut new_start.as_mut_slice()[new_end_idx..new_end_idx + sublen],
                            REGSUB_COPY
                                | REGSUB_BACKSLASH
                                | if opt_vars().p_magic { REGSUB_MAGIC } else { 0 },
                        ) as usize;
                        if n > 0 {
                            new_start.size += n - 1;
                        }
                        g_st().textlock -= 1;
                        g_st().sub_nsubs += 1;
                        did_sub = true;

                        curwin_mut().w_cursor.col = 0;

                        copycol = regmatch.endpos[0].col;

                        if nmatch > 1 {
                            sub_firstlnum += nmatch - 1;
                            let sub_slice = crate::memline::ml_get_slice(sub_firstlnum);
                            sub_firstline = SubBuf::from_cbuf(sub_slice);
                            if sub_firstlnum <= line2 {
                                do_again = true;
                            } else {
                                EXCMDS_STATE.with(|s| {
                                    let mut f = s.subflags.get();
                                    f.do_all = false;
                                    s.subflags.set(f);
                                });
                            }
                        }
                        if skip_match {
                            sub_firstline = SubBuf::from_cbuf(b"");
                            copycol = 0;
                        }

                        let mut replaced_bytes: Bcount = 0;
                        let start = regmatch.startpos[0];
                        let end = regmatch.endpos[0];
                        for j in 0..(nmatch - 1) {
                            replaced_bytes +=
                                crate::memline::ml_get_slice(lnum_start + j).len() as Bcount + 1;
                        }
                        replaced_bytes += (end.col - start.col) as Bcount;

                        let lnum_before_newlines = lnum;

                        let mut p1_idx = new_end_idx;
                        while new_start.as_slice()[p1_idx] != 0 {
                            let p1_b0 = new_start.as_slice()[p1_idx];
                            let p1_b1 = new_start.as_slice()[p1_idx + 1];
                            if p1_b0 == b'\\' && p1_b1 != 0 {
                                sublen -= 1;
                                n = new_start.size - p1_idx;
                                new_start
                                    .as_mut_slice()
                                    .copy_within(p1_idx + 1..p1_idx + 1 + n + 1, p1_idx);
                                new_start.size -= 1;
                                p1_idx += 1;
                            } else if p1_b0 == CAR as u8 {
                                if u_inssub(lnum) {
                                    let plen = (p1_idx + 1) as Colnr;

                                    new_start.as_mut_slice()[p1_idx] = 0;

                                    ml_append(
                                        lnum - 1,
                                        &new_start.as_slice()[..(plen as usize).saturating_sub(1)],
                                        plen,
                                        false,
                                    );
                                    mark_adjust(lnum + 1, MAXLNUM, 1, 0, K_EXTMARK_NOOP);

                                    if EXCMDS_STATE.with(|s| s.subflags.get()).do_ask {
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
                                    curwin_mut().w_cursor.lnum += 1;
                                    n = new_start.size - plen as usize;

                                    new_start
                                        .as_mut_slice()
                                        .copy_within(p1_idx + 1..p1_idx + 1 + n + 1, 0);
                                    new_start.size = n;
                                    p1_idx = 0;
                                } else {
                                    p1_idx += 1;
                                }
                            } else {
                                p1_idx += utfc_ptr2len(
                                    &new_start.as_slice()[p1_idx.min(new_start.as_slice().len())..],
                                ) as usize;
                            }
                        }
                        let new_endcol = new_start.size as Colnr;
                        current_match.end.col = new_endcol;
                        current_match.end.lnum = lnum;

                        let matchcols =
                            end.col - if end.lnum == start.lnum { start.col } else { 0 };
                        let subcols = new_endcol - if lnum == lnum_start { start_col } else { 0 };
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
                            subbytes: (sublen as Colnr - 1) as Bcount,
                            lnum_before: lnum_before_newlines,
                            lnum_after: lnum,
                        });
                    }
                }

                // skip:
                let lastone = skip_match
                    || g_st().got_int
                    || got_quit
                    || lnum > line2
                    || !(EXCMDS_STATE.with(|s| s.subflags.get()).do_all || do_again)
                    || (sub_firstline.as_slice()[matchcol as usize] == 0
                        && nmatch <= 1
                        && re_multiline(&regmatch.regprog) == 0);
                nmatch = -1;

                if lastone
                    || nmatch_tl > 0
                    || {
                        nmatch = vim_regexec_multi(
                            &mut regmatch,
                            Some(curwin_mut()),
                            curbuf_mut(),
                            sub_firstlnum,
                            matchcol,
                            Some(tm),
                            Some(&mut timed_out),
                        );
                        nmatch == 0
                    }
                    || regmatch.startpos[0].lnum > 0
                {
                    if !new_start.is_null() {
                        let tail = crate::memory::bytes_as_cstr(
                            &sub_firstline.as_slice()
                                [(copycol as usize).min(sub_firstline.as_slice().len())..],
                        )
                        .to_bytes_with_nul()
                        .to_vec();

                        let ns_size = new_start.size;
                        new_start.as_mut_slice()[ns_size..ns_size + tail.len()]
                            .copy_from_slice(&tail);

                        new_start.size += sub_firstline.size - copycol as usize;
                        matchcol = sub_firstline.size as Colnr - matchcol;
                        prev_matchcol = sub_firstline.size as Colnr - prev_matchcol;

                        if !u_savesub(lnum) {
                            break;
                        }
                        ml_replace(
                            lnum,
                            crate::memory::bytes_until_nul(new_start.as_slice()),
                            true,
                        );

                        for m in line_matches.drain(..) {
                            extmark_splice(
                                curbuf_mut(),
                                m.lnum_before - 1,
                                m.start_col,
                                m.end.lnum - m.start.lnum,
                                m.matchcols,
                                m.matchbytes,
                                m.lnum_after - m.lnum_before,
                                m.subcols,
                                m.subbytes,
                                K_EXTMARK_UNDO,
                            );
                        }

                        if nmatch_tl > 0 {
                            lnum += 1;
                            if !u_savedel(lnum, nmatch_tl) {
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
                                K_EXTMARK_NOOP,
                            );
                            if EXCMDS_STATE.with(|s| s.subflags.get()).do_ask {
                                deleted_lines(lnum, nmatch_tl);
                            }
                            lnum -= 1;
                            line2 -= nmatch_tl;
                            nmatch_tl = 0;
                        }

                        if EXCMDS_STATE.with(|s| s.subflags.get()).do_ask {
                            changed_bytes(lnum, 0);
                        } else {
                            if first_line == 0 {
                                first_line = lnum;
                            }
                            last_line = lnum + 1;
                        }

                        sub_firstlnum = lnum;
                        sub_firstline = std::mem::take(&mut new_start);
                        matchcol = sub_firstline.size as Colnr - matchcol;
                        prev_matchcol = sub_firstline.size as Colnr - prev_matchcol;
                        copycol = 0;
                    }
                    if nmatch == -1 && !lastone {
                        nmatch = vim_regexec_multi(
                            &mut regmatch,
                            Some(curwin_mut()),
                            curbuf_mut(),
                            sub_firstlnum,
                            matchcol,
                            Some(tm),
                            Some(&mut timed_out),
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
                g_st().sub_nlines += 1;
            }
            new_start.clear();
            sub_firstline.clear();
        }

        line_breakcheck();

        if timed_out != 0 || profile_passed_limit(tm) {
            got_quit = true;
        }
        lnum += 1;
    }

    curbuf_mut().deleted_bytes2 = 0;

    if first_line != 0 {
        i = curbuf_mut().b_ml.ml_line_count - old_line_count;
        changed_lines(curbuf_mut(), first_line, 0, last_line - i, i, false);

        let num_added = (last_line - first_line) as i64;
        let num_removed = num_added - i as i64;
        buf_updates_send_changes(curbuf_mut(), first_line, num_added, num_removed);
    }

    drop(sub_firstline);

    if EXCMDS_STATE.with(|s| s.subflags.get()).do_count {
        curwin_mut().w_cursor = old_cursor;
    }

    if g_st().sub_nsubs > start_nsubs {
        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start.lnum = eap.line1;
            curbuf_mut().b_op_end.lnum = line2;
            curbuf_mut().b_op_end.col = 0;
            curbuf_mut().b_op_start.col = 0;
        }

        if g_st().global_busy == 0 {
            if !EXCMDS_STATE.with(|s| s.subflags.get()).do_ask {
                if endcolumn {
                    coladvance(curwin_mut(), MAXCOL);
                } else {
                    beginline(BL_WHITE | BL_FIX);
                }
            }
            if cmdpreview_ns <= 0
                && !do_sub_msg(EXCMDS_STATE.with(|s| s.subflags.get()).do_count)
                && EXCMDS_STATE.with(|s| s.subflags.get()).do_ask
                && opt_vars().p_ch > 0
            {
                msg(c"", 0);
            }
        } else {
            EXCMDS_STATE.with(|s| s.global_need_beginline.set(true));
        }
        if EXCMDS_STATE.with(|s| s.subflags.get()).do_print {
            print_line(
                curwin_mut().w_cursor.lnum,
                EXCMDS_STATE.with(|s| s.subflags.get()).do_number,
                EXCMDS_STATE.with(|s| s.subflags.get()).do_list,
                true,
            );
        }
    } else if g_st().global_busy == 0 {
        if g_st().got_int {
            emsg(gettext(E_INTERR));
        } else if got_match {
            if opt_vars().p_ch > 0 && !ui_has(K_UIMESSAGES) {
                msg(c"", 0);
            }
        } else if EXCMDS_STATE.with(|s| s.subflags.get()).do_error {
            semsg!(gettext(E_PATNOTF2), get_search_pat());
        }
    }

    if EXCMDS_STATE.with(|s| s.subflags.get()).do_ask && has_any_folding(curwin_mut()) != 0 {
        changed_window_setting(curwin_mut());
    }

    regmatch.free_prog();
    drop(sub_owned);

    EXCMDS_STATE.with(|s| {
        let mut f = s.subflags.get();
        f.do_all = save_do_all;
        s.subflags.set(f);
    });
    EXCMDS_STATE.with(|s| {
        let mut f = s.subflags.get();
        f.do_ask = save_do_ask;
        s.subflags.set(f);
    });

    let mut retv: i32 = 0;

    if cmdpreview_ns > 0 && !aborting() {
        if got_quit || profile_passed_limit(tm) {
            set_option_direct(K_OPT_INCCOMMAND, cstr_as_obj(Some(c"")), 0, SID_NONE);
        } else if !opt_vars().p_icm.is_empty() && !pat.data.is_null() {
            if EXCMDS_STATE.with(|s| s.pre_hl_id.get()) == 0 {
                EXCMDS_STATE.with(|s| s.pre_hl_id.set(syn_check_group(b"Substitute")));
            }
            retv = show_sub(
                eap,
                old_cursor,
                &mut preview_lines,
                EXCMDS_STATE.with(|s| s.pre_hl_id.get()),
                cmdpreview_ns,
                cmdpreview_bufnr,
            );
        }
    }

    retv
}

/// Give message for number of substitutions.
pub fn do_sub_msg(count_only: bool) -> bool {
    if (((g_st().sub_nsubs as OptInt) > opt_vars().p_report
        && (g_st().key_typed || g_st().sub_nlines > 1 || opt_vars().p_report < 1))
        || count_only)
        && messaging()
    {
        if g_st().got_int {
            crate::memory::copy_bytes_to_c_buf(
                g_st().msg_buf_bytes_mut(),
                gettext(c"(Interrupted) ").to_bytes(),
            );
        } else {
            g_st().msg_buf[0] = 0;
        }

        let msg_single = if count_only {
            ngettext(
                c"%lld match on %lld line",
                c"%lld matches on %lld line",
                g_st().sub_nsubs as u64,
            )
        } else {
            ngettext(
                c"%lld substitution on %lld line",
                c"%lld substitutions on %lld line",
                g_st().sub_nsubs as u64,
            )
        };
        let msg_plural = if count_only {
            ngettext(
                c"%lld match on %lld lines",
                c"%lld matches on %lld lines",
                g_st().sub_nsubs as u64,
            )
        } else {
            ngettext(
                c"%lld substitution on %lld lines",
                c"%lld substitutions on %lld lines",
                g_st().sub_nsubs as u64,
            )
        };
        format_to_buf_add!(
            &mut g_st().msg_buf,
            ngettext(msg_single, msg_plural, g_st().sub_nlines as u64),
            g_st().sub_nsubs as i64,
            g_st().sub_nlines as i64,
        );
        let msg_cstr = crate::memory::bytes_as_cstr(g_st().borrow_mut().msg_buf_bytes_mut());
        if msg(msg_cstr, 0) {
            set_keep_msg(Some(msg_cstr), 0);
        }
        return true;
    }
    if g_st().got_int {
        emsg(gettext(E_INTERR));
        return true;
    }
    false
}

fn global_exe_one(cmd: &CStr, lnum: Linenr) {
    curwin_mut().w_cursor.lnum = lnum;
    curwin_mut().w_cursor.col = 0;
    let bytes = cmd.to_bytes();
    if bytes.is_empty() || bytes.first() == Some(&b'\n') {
        do_cmdline(Some(c"p"), crate::cmdexpand::LineGetter::None, DOCMD_NOWAIT);
    } else {
        do_cmdline(Some(cmd), crate::cmdexpand::LineGetter::None, DOCMD_NOWAIT);
    }
}

/// Execute a global command (`:g` / `:v`).
pub fn ex_global(eap: &mut Exarg) {
    let mut lnum: Linenr;
    let delim: u8;
    let patlen: usize;
    let mut regmatch: Regmmatch = Regmmatch::new();

    if g_st().global_busy != 0 && (eap.line1 != 1 || eap.line2 != curbuf_mut().b_ml.ml_line_count) {
        emsg(gettext(c"E147: Cannot do :global recursive with a range"));
        return;
    }

    let type_char: i32 = if eap.forceit {
        b'v' as i32
    } else {
        (eap.cmd.first_byte()) as i32
    };
    let mut cmd_idx = 0usize;
    let mut which_pat = RE_LAST;
    let pat_idx: Option<usize>;

    if eap.arg_cstr_buf_mut().first() == Some(&b'\\') {
        cmd_idx += 1;
        let b1 = eap.arg_cstr_buf_mut().get(cmd_idx).copied().unwrap_or(0);
        if !crate::strings::vim_haschr(c"/?&", i32::from(b1)) {
            emsg(gettext(E_BACKSLASH));
            return;
        }
        if b1 == b'&' {
            which_pat = RE_SUBST;
        } else {
            which_pat = RE_SEARCH;
        }
        cmd_idx += 1;
        pat_idx = None;
        patlen = 0;
    } else if eap.arg_cstr_buf_mut().first().copied().unwrap_or(0) == 0 {
        emsg(gettext(c"E148: Regular expression missing from global"));
        return;
    } else if !check_regexp_delim(i32::from(
        eap.arg_cstr_buf_mut().first().copied().unwrap_or(0),
    )) {
        return;
    } else {
        delim = eap.arg_cstr_buf_mut()[cmd_idx];
        cmd_idx += 1;
        pat_idx = Some(cmd_idx);
        let cmd_len = crate::memory::bytes_until_nul(&eap.arg_cstr_buf_mut()[cmd_idx..]).len();
        let off = skip_regexp_ex(
            &mut eap.arg_cstr_buf_mut()[cmd_idx..cmd_idx + cmd_len + 1],
            delim as i32,
            i32::from(opt_vars().p_magic),
            None,
            None,
            None,
        );
        cmd_idx += off;
        if eap.arg_cstr_buf_mut().get(cmd_idx).copied().unwrap_or(0) == delim {
            eap.arg_cstr_buf_mut()[cmd_idx] = 0;
            cmd_idx += 1;
        }
        patlen = crate::memory::bytes_until_nul(&eap.arg_cstr_buf_mut()[pat_idx.unwrap()..]).len();
    }

    let pat_cstr = match pat_idx {
        Some(idx) => crate::memory::bytes_as_cstr(&eap.arg_cstr_buf_mut()[idx..]),
        None => c"",
    };
    let mut used_pat = std::ffi::CString::default();
    if !search_regcomp(
        pat_cstr,
        patlen,
        Some(&mut used_pat),
        RE_BOTH,
        which_pat,
        SEARCH_HIS,
        opt_vars().p_magic,
        &mut regmatch,
    ) {
        emsg(gettext(E_INVCMD));
        return;
    }

    let cmd_owned = crate::memory::cstring_until_nul(&eap.arg_cstr_buf_mut()[cmd_idx..]);
    let cmd_cstr = cmd_owned.as_c_str();
    if g_st().global_busy != 0 {
        lnum = curwin_mut().w_cursor.lnum;
        let matched = vim_regexec_multi(
            &mut regmatch,
            Some(curwin_mut()),
            curbuf_mut(),
            lnum,
            0,
            None,
            None,
        );
        if (type_char == b'g' as i32 && matched != 0) || (type_char == b'v' as i32 && matched == 0)
        {
            global_exe_one(cmd_cstr, lnum);
        }
    } else {
        let mut ndone: i32 = 0;
        lnum = eap.line1;
        while lnum <= eap.line2 && !g_st().got_int {
            let matched = vim_regexec_multi(
                &mut regmatch,
                Some(curwin_mut()),
                curbuf_mut(),
                lnum,
                0,
                None,
                None,
            );
            if regmatch.regprog.is_null() {
                break;
            }
            if (type_char == b'g' as i32 && matched != 0)
                || (type_char == b'v' as i32 && matched == 0)
            {
                ml_setmarked(lnum);
                ndone += 1;
            }
            line_breakcheck();
            lnum += 1;
        }

        if g_st().got_int {
            msg(gettext(E_INTERR), 0);
        } else if ndone == 0 {
            if type_char == b'v' as i32 {
                smsg!(0, gettext(c"Pattern found in every line: %s"), &used_pat,);
            } else {
                smsg!(0, gettext(c"Pattern not found: %s"), &used_pat);
            }
        } else {
            global_exe(cmd_cstr);
        }
        ml_clearmarked();
    }
    regmatch.free_prog();
}

/// Execute `cmd` on lines marked with `ml_setmarked()`.
pub fn global_exe(cmd: &CStr) {
    let old_buf = g_st().curbuf;

    setpcmark();

    g_st().msg_didout = true;

    g_st().sub_nsubs = 0;
    g_st().sub_nlines = 0;
    EXCMDS_STATE.with(|s| s.global_need_msg_kind.set(true));
    EXCMDS_STATE.with(|s| s.global_need_beginline.set(false));
    g_st().global_busy = 1;
    let old_lcount = curbuf_mut().b_ml.ml_line_count;

    while !g_st().got_int && g_st().global_busy == 1 {
        let lnum = ml_firstmarked();
        if lnum == 0 {
            break;
        }
        global_exe_one(cmd, lnum);
        os_breakcheck();
    }

    g_st().global_busy = 0;
    if EXCMDS_STATE.with(|s| s.global_need_beginline.get()) {
        beginline(BL_WHITE | BL_FIX);
    } else {
        check_cursor(curwin_mut());
    }

    changed_line_abv_curs();

    if g_st().msg_col == 0 && g_st().msg_scrolled == 0 {
        g_st().msg_didout = false;
    }

    if !do_sub_msg(false) && g_st().curbuf == old_buf {
        msgmore(curbuf_mut().b_ml.ml_line_count - old_lcount);
    }
}

pub fn free_old_sub() {
    sub_set_replacement(SubReplacementString {
        sub: None,
        timestamp: 0,
        additional_data: None,
    });
}

/// Set up for a tagpreview.
pub fn prepare_tagpreview(undo_sync: bool, use_previewpopup: bool) -> bool {
    if curwin_mut().w_onebuf_opt.wo_pvw {
        return false;
    }

    for wp in for_all_windows_ptr() {
        if wp.w_onebuf_opt.wo_pvw {
            if use_previewpopup {
                if wp.w_kind != K_WIN_PREVIEW {
                    continue;
                }
            } else if wp.w_floating {
                continue;
            }
            if use_previewpopup {
                win_float_anchor_preview(&mut *wp);
            }
            win_enter(wp, undo_sync);
            return false;
        }
    }

    if use_previewpopup {
        let Some(wp) = win_float_special(false, true, K_WIN_PREVIEW) else {
            return false;
        };
        win_enter(wp, undo_sync);
        return true;
    }
    if !win_split(
        if g_st().g_do_tagpreview > 0 {
            g_st().g_do_tagpreview
        } else {
            0
        },
        0,
    ) {
        return false;
    }

    curwin_mut().w_onebuf_opt.wo_pvw = true;
    curwin_mut().w_onebuf_opt.wo_wfh = true;
    curwin_mut().w_onebuf_opt.wo_scb = false;
    curwin_mut().w_onebuf_opt.wo_crb = false;
    curwin_mut().w_onebuf_opt.wo_diff = false;

    set_option_direct(K_OPT_FOLDCOLUMN, cstr_as_obj(Some(c"0")), 0, SID_NONE);
    true
}

/// Shows the effects of the `:substitute` command being typed (`'inccommand'`).
fn show_sub(
    eap: &mut Exarg,
    old_cusr: CursorPos,
    preview_lines: &mut PreviewLines,
    hl_id: i32,
    cmdpreview_ns: i32,
    cmdpreview_bufnr: ObjHandle,
) -> i32 {
    let save_shm_p = opt_vars().p_shm.to_owned();
    let subresults = &preview_lines.subresults;
    let mut cmdpreview_buf: Option<&'static mut FileBuf> = None;

    set_option_direct(K_OPT_SHORTMESS, cstr_as_obj(Some(c"F")), 0, SID_NONE);

    for curres in subresults {
        if curres.start.lnum >= old_cusr.lnum {
            curwin_mut().w_cursor.lnum = curres.start.lnum;
            curwin_mut().w_cursor.col = curres.start.col;
            break;
        }
    }

    update_topline(curwin_mut());

    let mut col_width: i32 = 0;
    let preview = (opt_vars().p_icm.first_byte() == b's')
        && (eap.line1 != old_cusr.lnum || eap.line2 != old_cusr.lnum);

    if preview {
        cmdpreview_buf = crate::buffer::buflist_findnr_opt(cmdpreview_bufnr);
        debug_assert!(cmdpreview_buf.is_some());

        if let Some(last_match) = subresults.last() {
            let highest_lnum = last_match.start.lnum.max(last_match.end.lnum);
            debug_assert!(highest_lnum > 0);
            col_width = (f64::from(highest_lnum).log10() as i32) + 1 + 3;
        }
    }

    let mut str_buf: Vec<u8> = Vec::new();
    let mut line_size: Colnr = 0;
    let mut linenr_preview: Linenr = 0;
    let mut linenr_origbuf: Linenr = 0;
    let mut next_linenr: Linenr;

    for &m in subresults {
        if let Some(ref mut cp_buf) = cmdpreview_buf {
            let mut p_start = Lpos {
                lnum: 0,
                col: m.start.col,
            };
            let mut p_end = Lpos {
                lnum: 0,
                col: m.end.col,
            };

            buf_ensure_loaded(cp_buf);

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
                let line_cstr = if next_linenr == curbuf_mut().b_ml.ml_line_count + 1 {
                    c""
                } else {
                    line_size = ml_get_buf_len(curbuf_mut(), next_linenr) + col_width + 1;

                    if (line_size as usize) > str_buf.len() {
                        str_buf.resize(line_size as usize, 0);
                    }
                    ml_get_buf(curbuf_mut(), next_linenr)
                };
                format_to_buf!(
                    &mut str_buf[..line_size as usize],
                    c"|%*d| %s",
                    col_width - 3,
                    next_linenr,
                    line_cstr,
                );
                let str_slice = crate::memory::bytes_until_nul(&str_buf);
                if linenr_preview == 0 {
                    ml_replace_buf(cp_buf, 1, str_slice, true, false);
                } else {
                    ml_append_buf(
                        cp_buf,
                        linenr_preview,
                        &str_buf[..(line_size as usize).saturating_sub(1)],
                        line_size,
                        false,
                    );
                }
                linenr_preview += 1;
                next_linenr += 1;
            }
            linenr_origbuf = m.end.lnum;

            bufhl_add_hl_pos_offset(cp_buf, cmdpreview_ns, hl_id, p_start, p_end, col_width);
        }
        bufhl_add_hl_pos_offset(curbuf_mut(), cmdpreview_ns, hl_id, m.start, m.end, 0);
    }

    set_option_direct(
        K_OPT_SHORTMESS,
        cstr_as_obj(Some(save_shm_p.as_c_str())),
        0,
        SID_NONE,
    );

    if preview { 2 } else { 1 }
}

/// `:substitute` command.
pub fn ex_substitute(eap: &mut Exarg) {
    do_sub(eap, profile_zero(), 0, 0);
}

/// `:substitute` command preview callback.
pub fn ex_substitute_preview(
    eap: &mut Exarg,
    cmdpreview_ns: i32,
    cmdpreview_bufnr: ObjHandle,
) -> i32 {
    if !eap.arg.is_empty() && !ascii_isalnum(eap.arg.first_byte()) {
        let save_eap = eap.arg.clone();
        let retv = do_sub(
            eap,
            profile_setlimit(opt_vars().p_rdt),
            cmdpreview_ns,
            cmdpreview_bufnr,
        );
        eap.arg = save_eap;
        return retv;
    }

    0
}

/// Skip over the pattern argument of `":vimgrep /pat/[g][j]"`.
pub fn skip_vimgrep_pat<'a>(
    p_cstr: &'a CStr,
    mut s: Option<&mut Option<std::ffi::CString>>,
    mut flags: Option<&mut i32>,
) -> Option<&'a CStr> {
    let p_bytes = p_cstr.to_bytes();
    let first_b = p_bytes.first().copied().unwrap_or(0);
    let mut off: usize;
    if vim_is_idc(i32::from(first_b)) {
        off = p_bytes
            .iter()
            .position(|&b| ascii_iswhite(i32::from(b)))
            .unwrap_or(p_bytes.len());
        if let Some(ref mut s) = s {
            **s = Some(bytes_to_cstring(&p_bytes[..off]));
        }
        if s.is_some() && off < p_bytes.len() {
            off += 1;
        }
    } else if first_b == 0 {
        return None;
    } else {
        let c = i32::from(first_b);
        let after_delim = crate::memory::CStrSubslice::advance(p_cstr, 1);
        let end_cstr = skip_regexp(after_delim, c, 1);
        let pat_len = (end_cstr.as_ptr() as usize) - (after_delim.as_ptr() as usize);
        off = 1 + pat_len;
        if p_bytes.get(off).copied().map_or(0, i32::from) != c {
            return None;
        }

        if let Some(ref mut s) = s {
            **s = Some(bytes_to_cstring(&p_bytes[1..off]));
        }
        off += 1;

        let tail = &p_bytes[off..];
        let mut count = 0usize;
        for &b in tail {
            if !matches!(b, b'g' | b'j' | b'f') {
                break;
            }
            if let Some(ref mut flags) = flags {
                match b {
                    b'g' => **flags |= VGR_GLOBAL,
                    b'j' => **flags |= VGR_NOJUMP,
                    _ => **flags |= VGR_FUZZY,
                }
            }
            count += 1;
        }
        off += count;
    }
    Some(crate::memory::CStrSubslice::advance(p_cstr, off))
}

/// `:oldfiles` (sync) and `:browse oldfiles` (async).
pub fn ex_oldfiles(eap: &mut Exarg) {
    nlua_call_excmd(
        c"vim._core.ex_cmd",
        c"ex_oldfiles",
        eap,
        &g_st().cmdmod,
        None,
    );
}
